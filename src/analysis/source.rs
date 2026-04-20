//! Sources for a WDL documents used in analysis.

use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use anyhow::Result;
use anyhow::anyhow;
use anyhow::bail;
use path_clean::PathClean;
use tempfile::NamedTempFile;
use url::Url;
use wdl::analysis::Analyzer;

/// Remote URL schemes that are parsed as `Source::Url`.
const REMOTE_URL_SCHEMES: &[&str] = &["https://", "http://"];

/// File URL schemes that are parsed as `Source::File`.
const FILE_URL_SCHEMES: &[&str] = &["file://"];

/// Helper to check if a given string starts with the given prefix, ignoring
/// ASCII case.
fn starts_with_ignore_ascii_case(s: &str, prefix: &str) -> bool {
    s.get(0..prefix.len())
        .map(|s| s.eq_ignore_ascii_case(prefix))
        .unwrap_or(false)
}

/// Determines if the given string is a remote URL.
fn is_remote_url(s: &str) -> bool {
    REMOTE_URL_SCHEMES
        .iter()
        .any(|scheme| starts_with_ignore_ascii_case(s, scheme))
}

/// Determines if the given string is a `file://` URL.
fn is_file_url(s: &str) -> bool {
    FILE_URL_SCHEMES
        .iter()
        .any(|scheme| starts_with_ignore_ascii_case(s, scheme))
}

/// Determines if the given string is prefixed with a supported URL scheme for
/// source files.
pub(crate) fn is_supported_source_url(s: &str) -> bool {
    is_remote_url(s) || is_file_url(s)
}

/// An input provided over stdin.
#[derive(Clone, Debug)]
pub struct StdinSource {
    temp_file: Arc<NamedTempFile>,
    url: Url,
}

impl StdinSource {
    /// Create a new `StdinSource`.
    fn new() -> Result<Self> {
        let mut temp_file = NamedTempFile::new()?;
        let Ok(url) = Url::from_file_path(temp_file.path()) else {
            bail!("failed to convert path to URL");
        };

        std::io::copy(&mut std::io::stdin(), &mut temp_file)?;

        Ok(Self {
            temp_file: Arc::new(temp_file),
            url,
        })
    }

    /// Get the backing temp file path.
    pub fn path(&self) -> &Path {
        self.temp_file.path()
    }

    /// Get the backing temp file path as a URL.
    pub fn url(&self) -> &Url {
        &self.url
    }
}

/// A directory input.
#[derive(Clone, Debug)]
pub struct DirectorySource {
    /// The directory path.
    path: PathBuf,

    /// The directory path as a URL.
    url: Url,
}

impl DirectorySource {
    /// Get the directory path as a URL.
    pub fn url(&self) -> &Url {
        &self.url
    }

    /// Get the directory path.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl TryFrom<PathBuf> for DirectorySource {
    type Error = anyhow::Error;

    fn try_from(path: PathBuf) -> Result<Self, Self::Error> {
        let Ok(url) = Url::from_file_path(&path) else {
            bail!("failed to parse URL");
        };
        Ok(DirectorySource { path, url })
    }
}

impl AsRef<Path> for DirectorySource {
    fn as_ref(&self) -> &Path {
        &self.path
    }
}

/// A source for an analysis.
#[derive(Clone, Debug)]
pub enum Source {
    /// The source is stdin.
    Stdin(StdinSource),

    /// The source is a local file.
    File(Url),

    /// The source is a remote URL.
    Url(Url),

    /// The source is a local directory.
    Directory(DirectorySource),
}

impl Source {
    /// Get the current directory as a `Source`.
    pub fn current_dir() -> Self {
        Source::Directory(
            DirectorySource::try_from(
                std::env::current_dir()
                    .unwrap_or_else(|_| PathBuf::from(std::path::Component::CurDir.as_os_str())),
            )
            .expect("directory path should convert to URL"),
        )
    }

    /// Attempts to reference the source as a URL.
    pub fn as_url(&self) -> &Url {
        match self {
            Source::File(url) | Source::Url(url) => url,
            Source::Directory(dir) => dir.url(),
            Source::Stdin(source) => &source.url,
        }
    }

    /// Registers the source within an [`Analyzer`].
    ///
    /// Returns a [`NamedTempFile`] if the source is [`Source::Stdin`].
    pub async fn register<T: Send + Clone + 'static>(
        self,
        analyzer: &Analyzer<T>,
    ) -> Result<Option<StdinSource>> {
        match self {
            Source::File(url) | Source::Url(url) => analyzer.add_document(url).await.map(|_| None),
            Source::Directory(source) => analyzer.add_directory(&source.path).await.map(|_| None),
            Source::Stdin(source) => {
                analyzer.add_document(source.url.clone()).await?;
                Ok(Some(source))
            }
        }
    }
}

impl std::fmt::Display for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Source::File(url) | Source::Url(url) => write!(f, "{url}"),
            Source::Directory(source) => write!(f, "{path}", path = source.path.display()),
            Source::Stdin(_) => write!(f, "stdin"),
        }
    }
}

impl std::str::FromStr for Source {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "-" {
            return StdinSource::new().map(Source::Stdin);
        }

        if is_remote_url(s) {
            return Ok(Self::Url(
                s.parse().with_context(|| format!("invalid URL `{s}`"))?,
            ));
        }

        if is_file_url(s) {
            return Ok(Self::File(
                s.parse().with_context(|| format!("invalid URL `{s}`"))?,
            ));
        }

        let path = Path::new(s);

        let path = std::path::absolute(path)
            .map_err(|_| anyhow!("failed to convert `{path}` to a URI", path = path.display()))
            .map(|path| path.clean())?;

        if !path.exists() {
            bail!("source file `{s}` does not exist");
        }

        if path.is_dir() {
            return DirectorySource::try_from(path).map(Source::Directory);
        } else if path.is_file()
            && let Ok(url) = Url::from_file_path(&path)
        {
            return Ok(Source::File(url));
        }

        bail!("failed to convert `{s}` to a URI")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let path = std::path::absolute(file.path()).unwrap();

        let source = path.to_str().unwrap().parse::<Source>().unwrap();
        assert!(matches!(source, Source::File(_)));
        let url = source.as_url();
        assert_eq!(url.scheme(), "file");
        assert_eq!(url.to_file_path().unwrap(), path);
    }

    #[test]
    fn directory() {
        let dir = tempfile::TempDir::new().unwrap();
        let name = dir.path().as_os_str().to_str().unwrap();

        assert!(matches!(name.parse().unwrap(),
            Source::Directory(source)
            if source.path.as_os_str().to_str().unwrap() == name));
    }

    #[test]
    fn url() {
        const EXAMPLE: &str = "https://example.com/";
        assert!(matches!(EXAMPLE.parse().unwrap(),
            Source::Url(url)
            if url.as_str()
                == EXAMPLE
        ));
    }

    #[test]
    fn missing_file() {
        let err = "a-random-file-that-doesnt-exist.txt"
            .parse::<Source>()
            .unwrap_err();

        assert_eq!(
            err.to_string(),
            "source file `a-random-file-that-doesnt-exist.txt` does not exist"
        );
    }

    #[test]
    fn invalid_source() {
        let err = "".parse::<Source>().unwrap_err();

        assert_eq!(err.to_string(), "failed to convert `` to a URI");
    }
}
