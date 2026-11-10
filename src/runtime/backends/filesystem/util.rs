//! 路径、目录删除、z 作用域与错误等通用工具。

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Stdio};

use crate::protocol::contract::PathScope;
use crate::runtime::backend::BackendError;
use super::layer::View;

pub(super) fn error(value: impl std::fmt::Display) -> BackendError {
    BackendError::new(value.to_string())
}

pub(super) fn normalize_scopes(scope: &[PathScope]) -> Result<Vec<PathScope>, BackendError> {
    // Empty means "all mounts"; PathScope is only an optional filter now.
    let mut scopes = Vec::new();
    for value in scope {
        if !scopes.contains(value) {
            scopes.push(*value);
        }
    }
    Ok(scopes)
}

pub(super) fn validate_component(id: &str) -> Result<(), BackendError> {
    if id.is_empty()
        || id == "."
        || id == ".."
        || id.contains('/')
        || id.contains('\\')
        || id.contains('\0')
    {
        return Err(BackendError::new(format!(
            "invalid filesystem identifier {id:?}"
        )));
    }
    Ok(())
}

pub(super) fn path_exists(path: &Path) -> bool {
    fs::symlink_metadata(path).is_ok()
}

pub(super) fn reset_dir(path: &Path) -> Result<(), BackendError> {
    if path_exists(path) {
        remove_directory_force(path)?;
    }
    fs::create_dir_all(path).map_err(error)
}

/// OverlayFS work directories can contain a mode-000 `work` child.  Plain
/// `remove_dir_all` cannot descend into it, so make directories traversable
/// before unlinking their contents.
pub(super) fn remove_directory_force(path: &Path) -> Result<(), BackendError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(value) if value.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(value) => return Err(error(value)),
    };
    if metadata.is_dir() {
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        match fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    let entry = match entry {
                        Ok(entry) => entry,
                        Err(value) if value.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(value) => return Err(error(value)),
                    };
                    remove_directory_force(&entry.path())?;
                }
            }
            Err(value) if value.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(value) => return Err(error(value)),
        }
        match fs::remove_dir(path) {
            Ok(()) => Ok(()),
            Err(value) if value.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(value) => Err(error(value)),
        }?;
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(value) if value.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(value) => Err(error(value)),
        }?;
    }
    Ok(())
}

pub(super) fn ensure_overlay_disjoint(target: &Path, upper: &Path) -> Result<(), BackendError> {
    // An upper directory below its lower mount is supported (the state
    // directory is commonly below the workspace mount).  It is only unsafe
    // when it *is* the mount target, so that is the one case rejected.  The
    // state directory itself is masked with a tmpfs after the overlays.
    let target = fs::canonicalize(target).unwrap_or_else(|_| target.to_path_buf());
    let upper = fs::canonicalize(upper).unwrap_or_else(|_| upper.to_path_buf());
    if target == upper {
        return Err(BackendError::new(format!(
            "overlay upper {} must not be the mount target itself",
            upper.display()
        )));
    }
    Ok(())
}

pub(super) fn remove_path(btrfs: &Path, path: &Path, use_btrfs: bool) -> Result<(), BackendError> {
    remove_path_with_mode(btrfs, path, use_btrfs, use_btrfs)
}

/// Linux btrfs gives each subvolume its own st_dev, even on one filesystem.
/// OverlayFS renames copy-up temporaries from work into upper, so both trees
/// must have the same device (and hence the same btrfs subvolume).
pub(super) fn ensure_overlay_work(upper: &Path, work: &Path) -> Result<(), BackendError> {
    let upper_device = fs::metadata(upper).map_err(error)?.dev();
    let work_device = fs::metadata(work).map_err(error)?.dev();
    if upper_device != work_device {
        return Err(BackendError::new(format!(
            "overlay upper {} and work {} are on different filesystems or btrfs subvolumes; \
             copy-up rename would fail with EXDEV. Preserve this state directory and recreate \
             the environment with upper/work inside one volume subvolume; automatic migration \
             of the old upper-as-subvolume layout is not supported",
            upper.display(), work.display()
        )));
    }
    Ok(())
}

pub(super) fn remove_path_with_mode(
    btrfs: &Path,
    path: &Path,
    use_btrfs: bool,
    fast_delete: bool,
) -> Result<(), BackendError> {
    if !path_exists(path) {
        return Ok(());
    }
    let mut btrfs_failed = None;
    if use_btrfs && fast_delete {
        let output = Command::new(btrfs)
            .args([
                std::ffi::OsStr::new("subvolume"),
                std::ffi::OsStr::new("delete"),
                path.as_os_str(),
            ])
            .output()
            .map_err(error)?;
        if output.status.success() {
            return Ok(());
        }
        btrfs_failed = Some(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    if path_exists(path) && use_btrfs {
        let _ = Command::new(btrfs)
            .args([
                std::ffi::OsStr::new("property"),
                std::ffi::OsStr::new("set"),
                path.as_os_str(),
                std::ffi::OsStr::new("ro"),
                std::ffi::OsStr::new("false"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    if path_exists(path) {
        if use_btrfs {
            remove_btrfs_tree(btrfs, path)?;
        } else {
            remove_directory_force(path)?;
        }
    }
    if path_exists(path) {
        return Err(BackendError::new(format!(
            "could not remove filesystem path {}: {}",
            path.display(),
            btrfs_failed.unwrap_or_else(|| "path still exists".to_string())
        )));
    }
    Ok(())
}

/// Recursively delete a tree that may contain btrfs subvolumes.  Plain
/// `remove_dir_all` leaves nested subvolume roots behind, so every directory
/// gets a best-effort `btrfs subvolume delete` before `rmdir`.
pub(super) fn remove_btrfs_tree(btrfs: &Path, path: &Path) -> Result<(), BackendError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(value) if value.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(value) => return Err(error(value)),
    };
    if metadata.is_dir() {
        // Clear read-only before descending; otherwise removing children of a
        // read-only snapshot fails with EROFS.
        let _ = Command::new(btrfs)
            .args([
                std::ffi::OsStr::new("property"),
                std::ffi::OsStr::new("set"),
                path.as_os_str(),
                std::ffi::OsStr::new("ro"),
                std::ffi::OsStr::new("false"),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o700));
        match fs::read_dir(path) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.map_err(error)?;
                    remove_btrfs_tree(btrfs, &entry.path())?;
                }
            }
            Err(value) if value.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(value) => return Err(error(value)),
        }
        let _ = Command::new(btrfs)
            .args([
                std::ffi::OsStr::new("subvolume"),
                std::ffi::OsStr::new("delete"),
                path.as_os_str(),
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        match fs::remove_dir(path) {
            Ok(()) => Ok(()),
            Err(value) if value.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(value) => Err(error(value)),
        }
    } else {
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(value) if value.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(value) => Err(error(value)),
        }
    }
}

pub(crate) fn normalize_relative_path(value: &str) -> Result<String, BackendError> {
    let path = Path::new(value);
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::RootDir => {}
            Component::ParentDir | Component::Prefix(_) => {
                return Err(BackendError::new(format!(
                    "overlay redirect path is not relative: {value}"
                )));
            }
        }
    }
    Ok(normalized.to_string_lossy().into_owned())
}


pub(crate) fn remove_subtree(view: &mut View, path: &str) {
    let prefix = format!("{path}/");
    view.retain(|key, _| key != path && !key.starts_with(&prefix));
}


pub(crate) fn remove_children(view: &mut View, path: &str) {
    let prefix = format!("{path}/");
    view.retain(|key, _| !key.starts_with(&prefix));
}


pub(crate) fn safe_join(root: &Path, relative: &str) -> Result<PathBuf, BackendError> {
    let path = Path::new(relative);
    if path
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(BackendError::new(format!(
            "unsafe filesystem relative path {relative:?}"
        )));
    }
    Ok(root.join(path))
}


pub(crate) fn short_hash(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(value.as_bytes());
    format!("{:x}", digest)[..16].to_string()
}


pub(crate) fn now_nanos() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or(0)
}
