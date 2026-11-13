//! reflink、文件/upper 复制与时间戳/xattr 保留。

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use crate::runtime::backend::BackendError;
use super::layer::*;
use super::util::*;

const FICLONE: libc::c_ulong = 0x4004_9409;

#[cfg(test)]
thread_local! {
    static CLONE_FAIL_AFTER: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
    static CLONE_ENTRIES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(crate) fn set_clone_failure_after(entries: Option<usize>) {
    CLONE_FAIL_AFTER.with(|value| value.set(entries));
    CLONE_ENTRIES.with(|value| value.set(0));
}

/// Tests inject clone failures through thread-local counters, so cloning must
/// stay on the caller's thread while an injection is armed.
pub(crate) fn clone_failure_requires_serial_execution() -> bool {
    #[cfg(test)]
    {
        CLONE_FAIL_AFTER.with(|value| value.get().is_some())
    }
    #[cfg(not(test))]
    {
        false
    }
}

pub(crate) fn staging_path(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "layer".to_string());
    target
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!(".tmp-{name}-{}", now_nanos()))
}

/// Atomically publish a fully-written staging tree.  A crash before this
/// rename leaves only a `.tmp-*` directory, which is never referenced by
/// metadata and is removed on the next backend start.
pub(crate) fn finalize_staging(staging: &Path, target: &Path) -> Result<(), BackendError> {
    if path_exists(target) {
        remove_directory_force(target)?;
    }
    fs::rename(staging, target).map_err(error)?;
    if let Some(parent) = target.parent()
        && let Ok(directory) = File::open(parent)
    {
        directory.sync_all().map_err(error)?;
    }
    Ok(())
}

pub(crate) fn cleanup_staging(root: &Path) -> Result<(), BackendError> {
    let Ok(entries) = fs::read_dir(root) else {
        return Ok(());
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with(".tmp-") {
            remove_directory_force(&entry.path())?;
        }
    }
    Ok(())
}

pub(crate) fn reflink_file(source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let source_file = File::open(source)?;
    let target_file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(target)?;
    let result = unsafe {
        libc::ioctl(
            target_file.as_raw_fd(),
            FICLONE,
            source_file.as_raw_fd(),
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// True when an upper tree has no entries.  `read_dir` only fetches the first
/// directory entry, so this is O(1) in the number of files.  The overlay
/// `work` directory is a sibling of `upper`, never a child.
pub(crate) fn upper_is_empty(path: &Path) -> bool {
    match fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(value) if value.kind() == std::io::ErrorKind::NotFound => true,
        Err(_) => false,
    }
}

/// Count upper entries recursively, stopping at `limit`.  Cost is
/// O(min(entries, limit)); directories count as entries.  Symlinks are not
/// followed.
pub(crate) fn upper_entry_count(path: &Path, limit: usize) -> usize {
    let mut count = 0;
    let mut pending = vec![path.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries {
            let Ok(entry) = entry else {
                continue;
            };
            count += 1;
            if count >= limit {
                return count;
            }
            if let Ok(file_type) = entry.file_type()
                && file_type.is_dir()
            {
                pending.push(entry.path());
            }
        }
    }
    count
}

/// Clone only one upper directory.  Preserves overlay markers, whiteouts,
/// modes, symlinks and mtimes.  Never uses hard links.
pub(crate) fn clone_upper(source: &Path, target: &Path, use_reflink: bool) -> Result<(), BackendError> {
    clone_upper_excluding(source,target,use_reflink,&std::collections::BTreeSet::new())
}

pub(crate) fn clone_upper_excluding(source: &Path, target: &Path, use_reflink: bool, excluded: &std::collections::BTreeSet<PathBuf>) -> Result<(), BackendError> {    if !path_exists(source) {
        return Ok(());
    }
    fs::create_dir_all(target).map_err(error)?;
    let mut entries: Vec<_> = fs::read_dir(source)
        .map_err(error)?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        #[cfg(test)]
        {
            let count = CLONE_ENTRIES.with(|value| {
                let next = value.get() + 1;
                value.set(next);
                next
            });
            if let Some(limit) = CLONE_FAIL_AFTER.with(|value| value.get())
                && count >= limit
            {
                return Err(BackendError::new("injected clone failure"));
            }
        }
        let from = entry.path();
        let to = target.join(entry.file_name());
        if excluded.contains(&from) { fs::create_dir_all(&to).map_err(error)?; continue; }
        let metadata = fs::symlink_metadata(&from).map_err(error)?;
        let file_type = metadata.file_type();
        if file_type.is_dir() {
            clone_upper_excluding(&from, &to, use_reflink, excluded)?;
            fs::set_permissions(&to, fs::Permissions::from_mode(metadata.mode() & 0o7777))
                .map_err(error)?;
            copy_overlay_xattrs(&from, &to)?;
            set_times(&to, &metadata)?;
        } else if file_type.is_symlink() {
            let link = fs::read_link(&from).map_err(error)?;
            symlink(link, &to).map_err(error)?;
            set_times(&to, &metadata)?;
        } else if file_type.is_char_device() && metadata.rdev() == 0 {
            let c_path = CString::new(to.as_os_str().as_bytes())
                .map_err(|_| BackendError::new("invalid whiteout path"))?;
            if unsafe {
                libc::mknod(
                    c_path.as_ptr(),
                    libc::S_IFCHR | (metadata.mode() & 0o7777),
                    libc::makedev(0, 0),
                )
            } != 0
            {
                return Err(error(std::io::Error::last_os_error()));
            }
            fs::set_permissions(&to, fs::Permissions::from_mode(metadata.mode() & 0o7777))
                .map_err(error)?;
            set_times(&to, &metadata)?;
        } else if file_type.is_file() {
            let cloned = use_reflink && reflink_file(&from, &to).is_ok();
            if !cloned {
                fs::copy(&from, &to).map_err(error)?;
            }
            fs::set_permissions(&to, fs::Permissions::from_mode(metadata.mode() & 0o7777))
                .map_err(error)?;
            copy_overlay_xattrs(&from, &to)?;
            set_times(&to, &metadata)?;
        } else {
            let c_path = CString::new(to.as_os_str().as_bytes())
                .map_err(|_| BackendError::new("invalid special file path"))?;
            if unsafe {
                libc::mknod(
                    c_path.as_ptr(),
                    metadata.mode(),
                    metadata.rdev(),
                )
            } != 0
            {
                return Err(error(std::io::Error::last_os_error()));
            }
            set_times(&to, &metadata)?;
        }
    }
    Ok(())
}

pub(crate) fn copy_overlay_xattrs(source: &Path, target: &Path) -> Result<(), BackendError> {
    for name in ["opaque", "redirect", "metacopy", "whiteout"] {
        for namespace in ["trusted.overlay.", "user.overlay."] {
            let full = format!("{namespace}{name}");
            if let Some(value) = get_xattr(source, &full) {
                let _ = set_xattr(target, &full, &value);
            }
        }
    }
    Ok(())
}

pub(crate) fn set_times(path: &Path, metadata: &fs::Metadata) -> Result<(), BackendError> {
    let path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| BackendError::new("invalid timestamp path"))?;
    let times = [
        libc::timespec {
            tv_sec: metadata.atime(),
            tv_nsec: metadata.atime_nsec(),
        },
        libc::timespec {
            tv_sec: metadata.mtime(),
            tv_nsec: metadata.mtime_nsec(),
        },
    ];
    let result = unsafe {
        libc::utimensat(
            libc::AT_FDCWD,
            path.as_ptr(),
            times.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        // Timestamp preservation is best-effort on filesystems that reject it;
        // semantic content and modes are already copied.
        Ok(())
    }
}

pub(crate) fn copy_file(source: &Path, target: &Path) -> Result<(), BackendError> {
    let mut input = File::open(source).map_err(|value| {
        BackendError::new(format!("open source {}: {value}", source.display()))
    })?;
    let mut output = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(target)
        .map_err(|value| {
            BackendError::new(format!("open target {}: {value}", target.display()))
        })?;
    std::io::copy(&mut input, &mut output).map_err(error)?;
    // Persistence is finalized by the caller with one directory sync after
    // the complete copy layer is atomically renamed into place.
    Ok(())
}
