//! Node/View、diff 与 merge 测试。

use super::super::*;
use super::support::*;

#[test]
fn overlay_diff_translates_whiteout_opaque_redirect_and_metacopy() {
    let root = temp();
    let base = root.path().join("base");
    let upper = root.path().join("upper");
    write(&base, "modified.txt", "base");
    write(&base, "deleted.txt", "gone");
    write(&base, "opaque/old.txt", "old");
    write(&base, "old/kept.txt", "same");
    write(&base, "meta.txt", "same");

    write(&upper, "modified.txt", "new");
    write(&upper, "deleted.txt", "ignored");
    set_test_xattr(&upper.join("deleted.txt"), "user.overlay.whiteout", b"y");
    write(&upper, "opaque/new.txt", "new");
    set_test_xattr(&upper.join("opaque"), "user.overlay.opaque", b"y");
    write(&upper, "new/kept.txt", "same");
    set_test_xattr(&upper.join("new"), "user.overlay.redirect", b"old");
    write(&upper, "meta.txt", "");
    set_test_xattr(&upper.join("meta.txt"), "user.overlay.metacopy", b"y");

    let mut delta = View::new();
    let redirects = apply_layer(&mut delta, &upper).unwrap();
    let changes = changes_from_delta(&delta, &redirects);

    assert!(
        changes
            .iter()
            .any(|change| change.path == "modified.txt" && change.kind == "change")
    );
    assert!(
        changes
            .iter()
            .any(|change| change.path == "deleted.txt" && change.kind == "delete")
    );
    assert!(
        changes
            .iter()
            .any(|change| change.path == "opaque/new.txt" && change.kind == "change")
    );
    assert!(changes.iter().any(|change| change.path == "new"
        && change.kind == "rename"
        && change.old_path.as_deref() == Some("old")));
    // The delta layer contains a tombstone, not just an absent path.
    assert!(delta.get("deleted.txt").is_some_and(Node::is_whiteout));
    // Content was never loaded into the view.
    assert!(
        delta
            .values()
            .all(|node| node.source.is_some() || node.is_whiteout())
    );
}

#[test]
fn merge_delta_reports_conflicts_and_accepts_independent_changes() {
    let base = Node {
        mode: libc::S_IFREG | 0o644,
        size: 4,
        mtime_sec: 1,
        mtime_nsec: 0,
        inode: 1,
        kind: NodeKind::File,
        source: Some(PathBuf::from("/base/a")),
        opaque: false,
        redirect: None,
        metacopy: false,
    };
    let base_maps = vec![View::from([
        ("a".into(), base.clone()),
        ("b".into(), base.clone()),
        ("c".into(), base.clone()),
    ])];
    let theirs = Node {
        mode: base.mode,
        size: 5,
        mtime_sec: 2,
        mtime_nsec: 0,
        inode: 2,
        kind: NodeKind::File,
        source: Some(PathBuf::from("/source/a")),
        opaque: false,
        redirect: None,
        metacopy: false,
    };
    let delta = View::from([
        ("a".into(), theirs.clone()),
        (
            "b".into(),
            Node {
                mode: base.mode,
                size: 0,
                mtime_sec: 0,
                mtime_nsec: 0,
                inode: 0,
                kind: NodeKind::Whiteout,
                source: None,
                opaque: false,
                redirect: None,
                metacopy: false,
            },
        ),
        ("d".into(), theirs.clone()),
    ]);
    let target = View::from([
        (
            "a".into(),
            Node {
                size: 6,
                mtime_sec: 3,
                ..theirs.clone()
            },
        ),
        (
            "b".into(),
            Node {
                size: 7,
                mtime_sec: 4,
                ..base.clone()
            },
        ),
        ("d".into(), theirs.clone()),
    ]);
    let target_maps = vec![target.clone()];
    let conflicts = merge_delta(&delta, &[], &base_maps, &target_maps, &target);
    assert!(
        conflicts
            .iter()
            .any(|value| value.path == "a" && value.kind == "content")
    );
    assert!(
        conflicts
            .iter()
            .any(|value| value.path == "b" && value.kind == "delete_modify")
    );
    assert!(conflicts.iter().all(|value| value.path != "d"));
}
