//! 后台删除工作者与 trash 回收测试。

use super::super::*;
use super::support::*;

#[test]
fn startup_reclaims_trash_left_by_a_previous_run() {
    let root = temp();
    let trash = root.path().join("state/trash");
    let leftover = trash.join("999-1-crashed-environment");
    fs::create_dir_all(leftover.join("upper")).unwrap();
    fs::write(leftover.join("upper/file.txt"), b"leftover").unwrap();

    let backend = new_overlay_backend(root.path());
    wait_until(|| trash_is_empty(&trash));
    assert!(
        trash_is_empty(&trash),
        "startup did not reclaim trash left by a previous run"
    );
    assert!(
        !leftover.exists(),
        "startup left a retired environment in trash"
    );
    drop(backend);
}

#[test]
fn idle_worker_wakes_for_a_later_discard() {
    let root = temp();
    let trash = root.path().join("state/trash");
    let backend = new_overlay_backend(root.path());

    let first = root.path().join("first-retired");
    fs::create_dir_all(first.join("nested")).unwrap();
    fs::write(first.join("nested/file.txt"), b"first").unwrap();
    backend.retire_path(&first).unwrap();
    wait_until(|| trash_is_empty(&trash));
    assert!(trash_is_empty(&trash), "first discard was not reclaimed");

    // Give the worker time to return to its idle wait before signalling again.
    thread::sleep(std::time::Duration::from_millis(100));

    let second = root.path().join("second-retired");
    fs::create_dir_all(second.join("nested")).unwrap();
    fs::write(second.join("nested/file.txt"), b"second").unwrap();
    backend.retire_path(&second).unwrap();
    wait_until(|| trash_is_empty(&trash));
    assert!(
        trash_is_empty(&trash),
        "the idle worker did not wake for a later discard"
    );
    drop(backend);
}

#[test]
fn repeated_discards_leave_trash_empty() {
    let root = temp();
    let trash = root.path().join("state/trash");
    let backend = new_overlay_backend(root.path());

    let mut retired = Vec::new();
    for index in 0..8 {
        let path = root.path().join(format!("retired-{index}"));
        fs::create_dir_all(path.join("nested")).unwrap();
        fs::write(path.join("nested/file.txt"), format!("entry-{index}")).unwrap();
        backend.retire_path(&path).unwrap();
        retired.push(path);
    }

    wait_until(|| trash_is_empty(&trash));
    assert!(
        trash_is_empty(&trash),
        "coalesced discard signals left entries in trash"
    );
    for path in retired {
        assert!(!path.exists(), "retired path {} still exists", path.display());
    }
    drop(backend);
}
