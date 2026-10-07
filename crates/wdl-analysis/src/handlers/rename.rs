//! Handlers for "rename" requests.
//!
//! This module implements the LSP `textDocument/rename` functionality for WDL
//! files.
//!
//! See: [LSP Specification](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_rename)

use std::collections::HashMap;

use anyhow::Result;
use anyhow::bail;
use lsp_types::TextEdit;
use lsp_types::Url;
use lsp_types::WorkspaceEdit;
use wdl_ast::AstNode;
use wdl_ast::AstToken;
use wdl_ast::lexer::v1::is_ident;

use crate::SourcePosition;
use crate::SourcePositionEncoding;
use crate::graph::DocumentGraph;
use crate::handlers;
use crate::handlers::DocumentReference;
use crate::queue::DocumentRename;

/// Renames a symbol at a given position in a document.
///
/// It first finds all references to the symbol at the given position,
/// including the definition itself. Then, it creates a `WorkspaceEdit`
/// to rename all occurrences.
///
/// The rename is rejected if the new name is not a valid WDL identifier.
pub fn rename(
    graph: &DocumentGraph,
    document_uri: &Url,
    position: SourcePosition,
    encoding: SourcePositionEncoding,
    new_name: String,
) -> Result<Option<WorkspaceEdit>> {
    if !is_ident(&new_name) {
        bail!("name `{new_name}` is not a valid WDL identifier");
    }

    let references = handlers::find_all_references(graph, document_uri, position, encoding, true)?;
    if references.is_empty() {
        return Ok(None);
    }

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for location in references {
        let text_edit = TextEdit {
            range: location.range,
            new_text: new_name.clone(),
        };
        changes.entry(location.uri).or_default().push(text_edit);
    }

    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }))
}

/// Calculates the relative path from `base_dir` to `target_path` for use in an
/// import statement.
fn relative_import_path(
    target_path: &std::path::Path,
    base_dir: &std::path::Path,
) -> Option<String> {
    pathdiff::diff_paths(target_path, base_dir)
        .map(|diff| diff.to_string_lossy().replace('\\', "/"))
}

/// Renames a set of documents and updates all references.
pub fn rename_documents(
    graph: &DocumentGraph,
    renames: Vec<DocumentRename>,
) -> Result<Option<WorkspaceEdit>> {
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for rename in renames {
        let Ok(old_path) = rename.document.to_file_path() else {
            continue;
        };

        let Some(old_stem) = old_path.file_stem() else {
            continue;
        };

        let Ok(new_path) = rename.new_uri.to_file_path() else {
            continue;
        };

        let Some(new_name) = new_path.file_name() else {
            continue;
        };

        let Some(new_stem) = new_path.file_stem() else {
            continue;
        };

        let new_name_str = new_name.to_string_lossy();
        let new_stem_str = new_stem.to_string_lossy();

        // We'll skip replacing the implicit namespace if it:
        //
        // 1. Hasn't changed
        // 2. Is no longer a valid identifier
        //
        // But in any case, we'll still update the import statement as needed
        let replace_namespace = new_stem != old_stem && new_stem.to_str().is_some_and(is_ident);

        for location in handlers::find_document_references(graph, &rename.document)? {
            let (location, new_name) = match location {
                DocumentReference::Path(location) => {
                    let mut rel_path = new_name_str.to_string();
                    if let Ok(dep_path) = location.uri.to_file_path()
                        && let Some(dep_dir) = dep_path.parent()
                        && let Some(rel) = relative_import_path(&new_path, dep_dir)
                    {
                        rel_path = rel;
                    }

                    (location, rel_path)
                }
                DocumentReference::Name(location) => {
                    if !replace_namespace {
                        continue;
                    }

                    (location, new_stem_str.to_string())
                }
            };

            changes.entry(location.uri).or_default().push(TextEdit {
                range: location.range,
                new_text: new_name.clone(),
            });
        }

        let Some(index) = graph.get_index(&rename.document) else {
            continue;
        };

        let node = graph.get(index);

        let Some(document) = node.document() else {
            continue;
        };
        let Some(lines) = node.parse_state().lines() else {
            continue;
        };
        let Some(new_dir) = new_path.parent() else {
            continue;
        };

        for import in document.root().children::<wdl_ast::v1::ImportStatement>() {
            let wdl_ast::v1::ImportSource::Uri(uri) = import.source() else {
                continue;
            };

            let Some(wdl_ast::v1::LiteralStringText::Token(text)) = uri.text() else {
                continue;
            };

            if let Ok(import_uri) = node.uri().join(text.text())
                && let Ok(import_path) = import_uri.to_file_path()
                && let Some(rel) = relative_import_path(&import_path, new_dir)
            {
                let loc = handlers::common::location_from_span(node.uri(), text.span(), lines)?;
                changes
                    .entry(rename.document.clone())
                    .or_default()
                    .push(TextEdit {
                        range: loc.range,
                        new_text: rel,
                    });
            }
        }
    }

    if changes.is_empty() {
        return Ok(None);
    }

    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }))
}
