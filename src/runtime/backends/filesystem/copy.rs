//! reflink、文件/upper 复制与时间戳/xattr 保留。

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};
use std::path::Path;

use crate::runtime::backend::BackendError;
use super::layer::*;
use super::util::*;

const FICLONE: libc::c_ulong = 0x4004_9409;

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

/// Clone only one upper directory.  Preserves overlay markers, whiteouts,
/// modes, symlinks and mtimes.  Never uses hard links.
pub(crate) fn clone_upper(source: &Path, target: &Path, use_reflink: bool) -> Result<(), BackendError> {
    if !path_exists(source) {
        return Ok(());
    }
    fs::create_dir_all(target).map_err(error)?;
    let mut entries: Vec<_> = fs::read_dir(source)
        .map_err(error)?
        .filter_map(Result::ok)
        .collect();
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let from = entry.path();
        let to = target.join(entry.file_name());
        let metadata = fs::symlink_metadata(&from).map_err(error)?;
        let file_type = metadata.file_type();
        if file_type.is_dir() {
            clone_upper(&from, &to, use_reflink)?;
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
    output.sync_all().map_err(error)?;
    Ok(())
}

