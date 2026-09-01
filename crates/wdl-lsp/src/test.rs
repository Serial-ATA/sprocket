//! Utilities for working with `sprocket dev test` test definitions.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ffi::OsStr;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use anyhow::bail;
use line_index::LineIndex;
use sprocket_test_types::DocumentTests;
use tokio::sync::Mutex;
use url::Url;
use uuid::Uuid;
use wdl_analysis::Diagnostics;
use wdl_analysis::IncrementalChange;

/// The result of a [`SprocketTestYaml`] analysis.
#[derive(Debug)]
pub struct AnalysisResult {
    /// The unique ID of this analysis.
    pub id: Arc<str>,
    /// The analyzed document.
    pub document: Arc<SprocketTestYaml>,
    /// The diagnostics from parsing and analysis.
    pub diagnostics: Diagnostics,
}

/// The parse status of a `sprocket dev test` YAML file.
#[derive(Clone, Debug)]
enum DocumentState {
    /// The document was successfully parsed.
    Parsed((DocumentTests, Diagnostics)),
    /// The document failed to parse.
    Failed(Diagnostics),
}

/// A `sprocket dev test` YAML file.
#[derive(Clone, Debug)]
pub struct SprocketTestYaml {
    /// The line index of the document.
    pub lines: LineIndex,
    /// The current source of the file.
    pub source: String,
    /// The path to the file on disk.
    pub path: PathBuf,
    /// The parsed document, if any.
    document: Option<DocumentState>,
}

impl SprocketTestYaml {
    /// Get the tests from the document, if it was parsed.
    pub fn tests(&self) -> Option<&DocumentTests> {
        match self.document.as_ref() {
            Some(DocumentState::Parsed((tests, _))) => Some(tests),
            _ => None,
        }
    }
}

/// A cache of all known `sprocket dev test` YAML files.
#[derive(Debug, Default)]
pub struct SprocketTestCache {
    /// The documents in the cache.
    documents: Mutex<HashMap<Url, Arc<SprocketTestYaml>>>,
}

impl SprocketTestCache {
    /// Add a Sprocket test YAML file to the cache.
    pub async fn open(&self, uri: Url, content: String) -> Result<Arc<SprocketTestYaml>> {
        let Ok(path) = uri.to_file_path() else {
            // `Analyzer` only supports `file://` URIs anyway.
            bail!("unsupported uri: {uri}");
        };

        Ok(self
            .documents
            .lock()
            .await
            .entry(uri)
            .or_insert_with(|| {
                Arc::new(SprocketTestYaml {
                    lines: LineIndex::new(&content),
                    source: content,
                    path,
                    document: None,
                })
            })
            .clone())
    }

    /// Drop a [`SprocketTestYaml`] from the cache.
    pub async fn close(&self, uri: &Url) {
        self.documents.lock().await.remove(uri);
    }

    /// Apply a change to a [`SprocketTestYaml`].
    pub async fn change(&self, uri: Url, change: IncrementalChange) -> Result<(), anyhow::Error> {
        let mut docs = self.documents.lock().await;
        let Entry::Occupied(mut entry) = docs.entry(uri) else {
            return Ok(());
        };

        let test_yaml = Arc::make_mut(entry.get_mut());
        let (new_source, new_lines) = if change.start.is_some() {
            change.apply()?
        } else {
            let mut source = test_yaml.source.clone();
            let mut lines = test_yaml.lines.clone();
            change.apply_to(&mut source, &mut lines)?;
            (source, lines)
        };

        test_yaml.source = new_source;
        test_yaml.lines = new_lines;
        test_yaml.document = None;
        Ok(())
    }

    /// Get a [`SprocketTestYaml`] by its URI.
    pub async fn get(&self, uri: &Url) -> Option<Arc<SprocketTestYaml>> {
        let docs = self.documents.lock().await;
        docs.get(uri).cloned()
    }

    /// Returns true if the URI exists in the server's test YAML cache.
    pub async fn contains(&self, uri: &Url) -> bool {
        self.documents.lock().await.contains_key(uri)
    }

    /// Get a [`SprocketTestYaml`] by its URI, ensuring it is parsed beforehand.
    pub async fn ensure_parsed(
        &self,
        uri: Url,
    ) -> Result<Option<Arc<SprocketTestYaml>>, Diagnostics> {
        let mut docs = self.documents.lock().await;
        let Entry::Occupied(mut entry) = docs.entry(uri) else {
            return Ok(None);
        };

        let test_yaml = Arc::make_mut(entry.get_mut());

        // Parse if the document state doesn't exist yet
        if test_yaml.document.is_none() {
            let new_state = match DocumentTests::parse(&test_yaml.source) {
                Ok(result) => DocumentState::Parsed(result),
                Err(err) => DocumentState::Failed(err),
            };

            test_yaml.document = Some(new_state);
        }

        match &test_yaml.document.as_ref().unwrap() {
            DocumentState::Parsed(_) => Ok(Some(Arc::clone(entry.get()))),
            DocumentState::Failed(diagnostics) => Err(diagnostics.clone()),
        }
    }

    /// Evaluates the document's validation state and returns all associated
    /// diagnostics.
    pub async fn analyze_document(
        &self,
        uri: Url,
        associated_wdl: &wdl_analysis::Document,
    ) -> Result<Option<AnalysisResult>> {
        let mut docs = self.documents.lock().await;
        let Entry::Occupied(mut entry) = docs.entry(uri) else {
            return Ok(None);
        };

        let test_yaml = Arc::make_mut(entry.get_mut());

        // Ensure it has been parsed before attempting validation
        if test_yaml.document.is_none() {
            let new_state = match DocumentTests::parse(&test_yaml.source) {
                Ok(result) => DocumentState::Parsed(result),
                Err(err) => DocumentState::Failed(err),
            };
            test_yaml.document = Some(new_state);
        }

        let doc = test_yaml.document.as_mut().unwrap();

        // Evaluate current state and transition if necessary
        let diagnostics = match &doc {
            DocumentState::Failed(diagnostics) => diagnostics.clone(),
            DocumentState::Parsed((tests, parse_diagnostics)) => {
                let mut diagnostics = parse_diagnostics.clone();
                if let Err(e) = tests.validate(associated_wdl) {
                    diagnostics.extend(e);
                }

                diagnostics
            }
        };

        Ok(Some(AnalysisResult {
            id: Uuid::new_v4().to_string().into(),
            document: entry.get().clone(),
            diagnostics,
        }))
    }
}

/// Check if a directory is a valid Sprocket test directory.
///
/// A Sprocket test directory is valid if:
/// 1. Its name is `test`.
/// 2. Its parent contains at least one `.wdl` file.
fn is_sprocket_test_dir(path: &std::path::Path) -> bool {
    if !path.is_dir() {
        return false;
    }
    if path.file_name().and_then(|s| s.to_str()) != Some("test") {
        return false;
    }
    let Some(parent) = path.parent() else {
        return false;
    };
    if let Ok(entries) = std::fs::read_dir(parent) {
        for entry in entries.flatten() {
            if let Ok(file_type) = entry.file_type()
                && file_type.is_file()
                && entry.path().extension().and_then(|s| s.to_str()) == Some("wdl")
            {
                return true;
            }
        }
    }
    false
}

/// Check if a file is a valid Sprocket test definition file.
///
/// A Sprocket test definition file is valid if:
/// 1. Its extension is `yaml` or `yml`.
/// 2. Either its parent is a valid Sprocket test directory, OR there is an
///    accompanying `.wdl` file of the same name in the same directory.
pub fn is_sprocket_test_file(uri: &Url) -> bool {
    let Some(path) = uri.to_file_path().ok() else {
        return false;
    };

    let Some(ext) = path.extension().and_then(OsStr::to_str) else {
        return false;
    };
    if ext != "yaml" && ext != "yml" {
        return false;
    }

    let Some(parent) = path.parent() else {
        return false;
    };

    if is_sprocket_test_dir(parent) {
        return true;
    }

    let Some(base_name) = path.file_name() else {
        return false;
    };
    let wdl_sibling = parent.join(base_name).with_extension("wdl");
    wdl_sibling.is_file()
}
