//! Integration tests for the `workspace/willRenameFiles` request.

use async_lsp::lsp_types::request::WillRenameFiles;
use async_lsp::lsp_types::*;
use pretty_assertions::assert_eq;

use crate::common::TestContext;
use crate::common::TestContextBuilder;

async fn setup() -> TestContext {
    let mut ctx = TestContextBuilder::new("will_rename_files").build();
    ctx.initialize().await;
    ctx
}

fn verify_edits(mut expected: Vec<TextEdit>, received: &[TextEdit]) {
    for item in received {
        let matched = expected.iter().position(|expected| expected == item);

        if let Some(index) = matched {
            expected.remove(index);
        } else {
            panic!("unexpected text edit returned: {item:?}");
        }
    }

    assert!(
        expected.is_empty(),
        "some expected items were not returned: {expected:?}"
    );
}

#[tokio::test]
#[test_log::test]
async fn should_handle_file_renames() {
    let ctx = setup().await;

    // `./dep.wdl` -> `./dep_new.wdl`
    let edit = ctx
        .request::<WillRenameFiles>(RenameFilesParams {
            files: vec![FileRename {
                old_uri: ctx.doc_uri("dep.wdl").to_string(),
                new_uri: ctx.doc_uri("dep_new.wdl").to_string(),
            }],
        })
        .await
        .unwrap()
        .expect("should produce edit");

    let expected_changes = vec![
        TextEdit {
            // First "dep.wdl" import
            range: Range {
                start: Position::new(2, 8),
                end: Position::new(2, 15),
            },
            new_text: "dep_new.wdl".to_string(),
        },
        TextEdit {
            // Second "dep.wdl" import
            range: Range {
                start: Position::new(3, 8),
                end: Position::new(3, 15),
            },
            new_text: "dep_new.wdl".to_string(),
        },
        TextEdit {
            // Implicit "dep" namespace
            range: Range {
                start: Position::new(6, 9),
                end: Position::new(6, 12),
            },
            new_text: "dep_new".to_string(),
        },
    ];

    let changes = edit.changes.expect("should produce changes");
    assert_eq!(changes.len(), 1);

    let changes = changes
        .get(&ctx.doc_uri("source.wdl"))
        .expect("should only produce changes for source.wdl");
    verify_edits(expected_changes, changes);
}

#[tokio::test]
#[test_log::test]
async fn should_handle_dependency_directory_changes() {
    // Moving a file with dependents should update all import statements and
    // implicit namespaces referencing it.

    let ctx = setup().await;

    // `./dep.wdl` -> `./some_dir/dep.wdl`
    let mut new_path = ctx.doc_path("dep.wdl");
    assert!(new_path.pop());
    new_path.push("some_dir");
    new_path.push("dep.wdl");

    let edit = ctx
        .request::<WillRenameFiles>(RenameFilesParams {
            files: vec![FileRename {
                old_uri: ctx.doc_uri("dep.wdl").to_string(),
                new_uri: Url::from_file_path(new_path).unwrap().to_string(),
            }],
        })
        .await
        .unwrap()
        .expect("should produce edit");

    let expected_changes = vec![
        TextEdit {
            // First "dep.wdl" import
            range: Range {
                start: Position::new(2, 8),
                end: Position::new(2, 15),
            },
            new_text: "some_dir/dep.wdl".to_string(),
        },
        TextEdit {
            // Second "dep.wdl" import
            range: Range {
                start: Position::new(3, 8),
                end: Position::new(3, 15),
            },
            new_text: "some_dir/dep.wdl".to_string(),
        },
    ];

    let changes = edit.changes.expect("should produce changes");
    assert_eq!(changes.len(), 1);

    let changes = changes
        .get(&ctx.doc_uri("source.wdl"))
        .expect("should only produce changes for source.wdl");
    verify_edits(expected_changes, changes);
}

#[tokio::test]
#[test_log::test]
async fn should_handle_directory_changes() {
    // Moving a file with dependencies needs to update the paths of dependencies
    // relative to its new location.
    //
    // In this case, `source.wdl` moves into another directory while depending
    // on the sibling `dep.wdl`. All `dep.wdl` imports need to be updated to
    // `../dep.wdl`.

    let ctx = setup().await;

    // `./source.wdl` -> `./some_dir/source.wdl`
    let mut new_path = ctx.doc_path("source.wdl");
    assert!(new_path.pop());
    new_path.push("some_dir");
    new_path.push("source.wdl");

    let edit = ctx
        .request::<WillRenameFiles>(RenameFilesParams {
            files: vec![FileRename {
                old_uri: ctx.doc_uri("source.wdl").to_string(),
                new_uri: Url::from_file_path(new_path).unwrap().to_string(),
            }],
        })
        .await
        .unwrap()
        .expect("should produce edit");

    let expected_changes = vec![
        TextEdit {
            // First "dep.wdl" import
            range: Range {
                start: Position::new(2, 8),
                end: Position::new(2, 15),
            },
            new_text: "../dep.wdl".to_string(),
        },
        TextEdit {
            // Second "dep.wdl" import
            range: Range {
                start: Position::new(3, 8),
                end: Position::new(3, 15),
            },
            new_text: "../dep.wdl".to_string(),
        },
    ];

    let changes = edit.changes.expect("should produce changes");
    assert_eq!(changes.len(), 1);

    let changes = changes
        .get(&ctx.doc_uri("source.wdl"))
        .expect("should only produce changes for source.wdl");
    verify_edits(expected_changes, changes);
}
