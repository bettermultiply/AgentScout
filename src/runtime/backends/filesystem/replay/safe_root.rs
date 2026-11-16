//! openat2 根目录 fd 操作（防绝对符号链接逃逸）。

use std::ffi::CString;
use std::fs::File;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;

use super::super::util::*;
use crate::runtime::backend::BackendError;

const RESOLVE_NO_MAGICLINKS: u64 = 0x02;
const RESOLVE_IN_ROOT: u64 = 0x10;

#[repr(C)]
pub(crate) struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}

/// Directory-fd-rooted operations for replaying into a live target mount.
/// All paths are resolved with `openat2(RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS)`
/// and final components use `O_NOFOLLOW`; absolute symlinks cannot escape to
/// the caller's root.
pub(crate) struct SafeRoot {
    fd: OwnedFd,
}

impl SafeRoot {
    pub(crate) fn open(pid: u32) -> Result<Self, BackendError> {
        let path = CString::new(format!("/proc/{pid}/root"))
            .map_err(|_| BackendError::new("invalid target root path"))?;
        let fd = unsafe {
            libc::open(
                path.as_ptr(),
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(Self {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    pub(crate) fn openat2(
        &self,
        path: &str,
        flags: libc::c_int,
        mode: u32,
        resolve: u64,
    ) -> Result<OwnedFd, BackendError> {
        let path =
            CString::new(path).map_err(|_| BackendError::new("invalid target-relative path"))?;
        let how = OpenHow {
            flags: flags as u64,
            mode: mode as u64,
            resolve,
        };
        let result = unsafe {
            libc::syscall(
                libc::SYS_openat2,
                self.fd.as_raw_fd() as libc::c_long,
                path.as_ptr() as libc::c_long,
                &how as *const OpenHow as libc::c_long,
                std::mem::size_of::<OpenHow>() as libc::c_long,
            )
        };
        if result < 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(unsafe { OwnedFd::from_raw_fd(result as libc::c_int) })
    }

    pub(crate) fn open_dir(&self, path: &str, create: bool) -> Result<OwnedFd, BackendError> {
        if path.is_empty() || path == "." {
            let fd = unsafe { libc::dup(self.fd.as_raw_fd()) };
            if fd < 0 {
                return Err(error(std::io::Error::last_os_error()));
            }
            return Ok(unsafe { OwnedFd::from_raw_fd(fd) });
        }
        let mut current: Option<OwnedFd> = None;
        let mut prefix = String::new();
        for component in path.split('/') {
            if component.is_empty() || component == "." {
                continue;
            }
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            match self.openat2(
                &prefix,
                libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                0,
                RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS,
            ) {
                Ok(fd) => current = Some(fd),
                Err(value) if create && value.message.contains("No such file") => {
                    let parent = match &current {
                        Some(parent) => unsafe {
                            OwnedFd::from_raw_fd(libc::dup(parent.as_raw_fd()))
                        },
                        None => unsafe { OwnedFd::from_raw_fd(libc::dup(self.fd.as_raw_fd())) },
                    };
                    let name = CString::new(component)
                        .map_err(|_| BackendError::new("invalid target directory name"))?;
                    if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } != 0 {
                        return Err(error(std::io::Error::last_os_error()));
                    }
                    current = Some(self.openat2(
                        &prefix,
                        libc::O_PATH | libc::O_DIRECTORY | libc::O_CLOEXEC,
                        0,
                        RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS,
                    )?);
                }
                Err(value) => return Err(value),
            }
        }
        current.ok_or_else(|| BackendError::new("target path has no components"))
    }

    pub(crate) fn parent(
        &self,
        path: &str,
        create: bool,
    ) -> Result<(OwnedFd, String), BackendError> {
        let path = normalize_relative_path(path)?;
        let (parent, name) = path
            .rsplit_once('/')
            .map(|(parent, name)| (parent.to_string(), name.to_string()))
            .unwrap_or_else(|| (String::new(), path));
        if name.is_empty() {
            return Err(BackendError::new("target path has no file name"));
        }
        Ok((self.open_dir(&parent, create)?, name))
    }

    pub(crate) fn remove(&self, path: &str) -> Result<(), BackendError> {
        let path = normalize_relative_path(path)?;
        if path.is_empty() {
            return Err(BackendError::new("refusing to remove target root"));
        }
        let (parent, name) = self.parent(&path, false)?;
        let c_name = CString::new(name.clone())
            .map_err(|_| BackendError::new("invalid target file name"))?;
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                c_name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result != 0 {
            let value = std::io::Error::last_os_error();
            if value.kind() == std::io::ErrorKind::NotFound {
                return Ok(());
            }
            return Err(error(value));
        }
        if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
            if let Ok(child) = self.openat2(
                &path,
                libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0,
                RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS,
            ) {
                remove_dir_fd(&child)?;
            }
            if unsafe { libc::unlinkat(parent.as_raw_fd(), c_name.as_ptr(), libc::AT_REMOVEDIR) }
                != 0
            {
                return Err(error(std::io::Error::last_os_error()));
            }
        } else if unsafe { libc::unlinkat(parent.as_raw_fd(), c_name.as_ptr(), 0) } != 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(())
    }

    pub(crate) fn exists(&self, path: &str) -> Result<bool, BackendError> {
        let path = normalize_relative_path(path)?;
        let (parent, name) = match self.parent(&path, false) {
            Ok(value) => value,
            Err(_) => return Ok(false),
        };
        let c_name = CString::new(name).map_err(|_| BackendError::new("invalid target name"))?;
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let result = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                c_name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if result == 0 {
            Ok(true)
        } else if std::io::Error::last_os_error().kind() == std::io::ErrorKind::NotFound {
            Ok(false)
        } else {
            Err(error(std::io::Error::last_os_error()))
        }
    }

    pub(crate) fn mkdir(&self, path: &str, mode: u32) -> Result<(), BackendError> {
        let path = normalize_relative_path(path)?;
        let (parent, name) = self.parent(&path, true)?;
        let c_name = CString::new(name.clone())
            .map_err(|_| BackendError::new("invalid target directory name"))?;
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let exists = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                c_name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0;
        if exists {
            if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
                return Ok(());
            }
            drop(parent);
            self.remove(&path)?;
            let (parent, name) = self.parent(&path, true)?;
            let c_name = CString::new(name)
                .map_err(|_| BackendError::new("invalid target directory name"))?;
            if unsafe {
                libc::mkdirat(
                    parent.as_raw_fd(),
                    c_name.as_ptr(),
                    (mode & 0o7777) as libc::mode_t,
                )
            } != 0
            {
                return Err(error(std::io::Error::last_os_error()));
            }
            return Ok(());
        }
        if unsafe {
            libc::mkdirat(
                parent.as_raw_fd(),
                c_name.as_ptr(),
                (mode & 0o7777) as libc::mode_t,
            )
        } != 0
        {
            return Err(error(std::io::Error::last_os_error()));
        }
        let _ = mode;
        Ok(())
    }

    pub(crate) fn write_file(
        &self,
        path: &str,
        source: &Path,
        mode: u32,
    ) -> Result<(), BackendError> {
        let path = normalize_relative_path(path)?;
        let (parent, name) = self.parent(&path, true)?;
        let c_name = CString::new(name.clone())
            .map_err(|_| BackendError::new("invalid target file name"))?;
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        let exists = unsafe {
            libc::fstatat(
                parent.as_raw_fd(),
                c_name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } == 0;
        if exists && (stat.st_mode & libc::S_IFMT) != libc::S_IFREG {
            drop(parent);
            self.remove(&path)?;
            return self.write_file(&path, source, mode);
        }
        drop(parent);
        let output = self.openat2(
            &path,
            libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            mode & 0o7777,
            RESOLVE_IN_ROOT | RESOLVE_NO_MAGICLINKS,
        )?;
        let mut input = File::open(source).map_err(error)?;
        let mut output = File::from(output);
        std::io::copy(&mut input, &mut output).map_err(error)?;
        let _ = unsafe { libc::fchmod(output.as_raw_fd(), mode & 0o7777) };
        Ok(())
    }

    pub(crate) fn symlink(&self, path: &str, target: &Path) -> Result<(), BackendError> {
        let path = normalize_relative_path(path)?;
        let (parent, name) = self.parent(&path, true)?;
        let c_name = CString::new(name).map_err(|_| BackendError::new("invalid symlink name"))?;
        let c_target = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| BackendError::new("invalid symlink target"))?;
        if unsafe { libc::symlinkat(c_target.as_ptr(), parent.as_raw_fd(), c_name.as_ptr()) } != 0 {
            return Err(error(std::io::Error::last_os_error()));
        }
        Ok(())
    }
}

pub(crate) fn remove_dir_fd(fd: &OwnedFd) -> Result<(), BackendError> {
    let dup = unsafe { libc::dup(fd.as_raw_fd()) };
    if dup < 0 {
        return Err(error(std::io::Error::last_os_error()));
    }
    let directory = unsafe { libc::fdopendir(dup) };
    if directory.is_null() {
        let value = std::io::Error::last_os_error();
        unsafe { libc::close(dup) };
        return Err(error(value));
    }
    loop {
        let entry = unsafe { libc::readdir(directory) };
        if entry.is_null() {
            break;
        }
        let name = unsafe { std::ffi::CStr::from_ptr((*entry).d_name.as_ptr()) };
        let bytes = name.to_bytes();
        if bytes == b"." || bytes == b".." {
            continue;
        }
        let mut stat: libc::stat = unsafe { std::mem::zeroed() };
        if unsafe {
            libc::fstatat(
                fd.as_raw_fd(),
                name.as_ptr(),
                &mut stat,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        } != 0
        {
            unsafe { libc::closedir(directory) };
            return Err(error(std::io::Error::last_os_error()));
        }
        if (stat.st_mode & libc::S_IFMT) == libc::S_IFDIR {
            let child = unsafe {
                libc::openat(
                    fd.as_raw_fd(),
                    name.as_ptr(),
                    libc::O_PATH | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if child < 0 {
                unsafe { libc::closedir(directory) };
                return Err(error(std::io::Error::last_os_error()));
            }
            let child = unsafe { OwnedFd::from_raw_fd(child) };
            if let Err(value) = remove_dir_fd(&child) {
                unsafe { libc::closedir(directory) };
                return Err(value);
            }
            if unsafe { libc::unlinkat(fd.as_raw_fd(), name.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
                unsafe { libc::closedir(directory) };
                return Err(error(std::io::Error::last_os_error()));
            }
        } else if unsafe { libc::unlinkat(fd.as_raw_fd(), name.as_ptr(), 0) } != 0 {
            unsafe { libc::closedir(directory) };
            return Err(error(std::io::Error::last_os_error()));
        }
    }
    unsafe { libc::closedir(directory) };
    Ok(())
}
