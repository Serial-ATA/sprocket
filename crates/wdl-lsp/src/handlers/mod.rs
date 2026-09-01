//! Language server protocol handlers.

pub mod code_lens;
pub mod diagnostic;

/// Get the WDL file path associated with a test definition YAML file.
///
/// A test YAML file *must* have an associated WDL file, otherwise we don't
/// consider it valid.
///
/// See [`is_sprocket_test_file()`](crate::test::is_sprocket_test_file)
fn associated_wdl_file_path(path: &std::path::Path) -> Option<std::path::PathBuf> {
    let base_name = path.file_name()?;
    let expected_wdl = std::path::Path::new(base_name).with_extension("wdl");
    let parent = path.parent()?;

    let in_test_dir =
        parent.is_dir() && parent.file_name().and_then(|s| s.to_str()) == Some("test");
    let wdl_dir = if in_test_dir {
        parent.parent()?
    } else {
        parent
    };

    let associated_wdl_path = wdl_dir.join(expected_wdl);
    if !associated_wdl_path.exists() {
        return None;
    }

    Some(associated_wdl_path)
}
