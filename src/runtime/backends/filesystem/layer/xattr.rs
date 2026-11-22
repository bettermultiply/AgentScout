
use std::ffi::CString;
use std::fs::{self, File};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::super::util::*;
use crate::runtime::backend::BackendError;

pub(crate) fn create_whiteout(root: &Path, path: &str) -> Result<(), BackendError> {
    let target = safe_join(root, path)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(error)?;
    }
    let _ = fs::remove_file(&target);
    let _ = fs::remove_dir(&target);
    let c_path = CString::new(target.as_os_str().as_bytes())
        .map_err(|_| BackendError::new(format!("invalid whiteout path {path}")))?;
    let result =
        unsafe { libc::mknod(c_path.as_ptr(), libc::S_IFCHR | 0o600, libc::makedev(0, 0)) };
    if result != 0 {
        // Fall back to an xattr whiteout where the kernel honours userxattr.
        File::create(&target).map_err(error)?;
        set_xattr(&target, "user.overlay.whiteout", b"y")?;
    }
    Ok(())
}

pub(crate) fn set_xattr(path: &Path, name: &str, value: &[u8]) -> Result<(), BackendError> {
    let c_path = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| BackendError::new(format!("invalid xattr path {}", path.display())))?;
    let c_name = CString::new(name).map_err(|_| BackendError::new("invalid xattr name"))?;
    let result = unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(error(std::io::Error::last_os_error()))
    }
}

pub(crate) fn overlay_xattr(path: &Path, name: &str) -> Option<Vec<u8>> {
    for namespace in ["trusted.overlay.", "user.overlay."] {
        let full = format!("{namespace}{name}");
        if let Some(value) = get_xattr(path, &full) {
            return Some(value);
        }
    }
    None
}

pub(crate) fn has_overlay_xattr(path: &Path, name: &str) -> bool {
    overlay_xattr(path, name).is_some()
}

pub(crate) fn get_xattr(path: &Path, name: &str) -> Option<Vec<u8>> {
    let path = CString::new(path.as_os_str().as_bytes()).ok()?;
    let name = CString::new(name).ok()?;
    let size = unsafe { libc::getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0) };
    if size < 0 {
        return None;
    }
    if size == 0 {
        return Some(Vec::new());
    }
    let mut buffer = vec![0u8; size as usize];
    let read = unsafe {
        libc::getxattr(
            path.as_ptr(),
            name.as_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
        )
    };
    if read < 0 {
        return None;
    }
    buffer.truncate(read as usize);
    Some(buffer)
}
