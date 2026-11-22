use super::super::SpeculativeTree;
use serde_json::json;
use tempfile::tempdir;

#[test]
fn ended_reopen_preserves_saved_workspace_path() {
    let run = tempdir().unwrap();
    let workspace = tempdir().unwrap();
    let tree = SpeculativeTree::reopen_ended(
        run.path(),
        "run-test",
        3,
        &json!({"workspace_path": workspace.path()}),
    )
    .unwrap();

    assert_eq!(tree.workspace(), workspace.path());
}
