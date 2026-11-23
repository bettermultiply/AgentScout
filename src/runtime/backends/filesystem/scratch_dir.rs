//! Temporary roots shared by filesystem tests and benchmark examples.

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::{
    ffi::OsStrExt,
    fs::{MetadataExt, PermissionsExt},
};
use std::path::{Path, PathBuf};
use std::process::Command;

pub(crate) struct ScratchDir {
    path: Option<PathBuf>,
    identity: (u64, u64),
    mount: u64,
    btrfs: bool,
}

impl ScratchDir {
    pub(crate) fn new(base: impl AsRef<Path>, prefix: &str) -> io::Result<Self> {
        let dir = tempfile::Builder::new()
            .prefix(prefix)
            .permissions(fs::Permissions::from_mode(0o700))
            .tempdir_in(fs::canonicalize(base)?)?;
        let meta = fs::symlink_metadata(dir.path())?;
        let mount = mount_id(dir.path())?;
        let path = c_path(dir.path())?;
        let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
        if unsafe { libc::statfs(path.as_ptr(), stat.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self {
            identity: (meta.dev(), meta.ino()),
            mount,
            btrfs: unsafe { stat.assume_init() }.f_type == libc::BTRFS_SUPER_MAGIC,
            path: Some(dir.keep()),
        })
    }

    pub(crate) fn path(&self) -> &Path {
        self.path.as_deref().expect("scratch directory is open")
    }

    pub(crate) fn close(mut self) -> io::Result<()> {
        self.cleanup()
    }

    fn cleanup(&mut self) -> io::Result<()> {
        let Some(path) = self.path.take() else {
            return Ok(());
        };
        let result = (|| {
            let meta = match fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            };
            if !meta.is_dir() || (meta.dev(), meta.ino()) != self.identity {
                return Err(io::Error::other("scratch root was replaced"));
            }
            remove_tree(&path, self.mount, self.btrfs)?;
            match fs::symlink_metadata(&path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error),
                Ok(_) => Err(io::Error::other("scratch root still exists")),
            }
        })();
        result.map_err(|error| io::Error::other(format!("cleanup {}: {error}", path.display())))
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        if let Err(error) = self.cleanup() {
            if std::thread::panicking() {
                eprintln!("{error}");
            } else {
                panic!("{error}");
            }
        }
    }
}

fn c_path(path: &Path) -> io::Result<CString> {
    CString::new(path.as_os_str().as_bytes()).map_err(io::Error::other)
}

pub(crate) fn mount_id(path: &Path) -> io::Result<u64> {
    let path = c_path(path)?;
    let mut stat = std::mem::MaybeUninit::<libc::statx>::uninit();
    if unsafe {
        libc::statx(
            libc::AT_FDCWD,
            path.as_ptr(),
            libc::AT_SYMLINK_NOFOLLOW,
            libc::STATX_MNT_ID,
            stat.as_mut_ptr(),
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    if stat.stx_mask & libc::STATX_MNT_ID == 0 {
        return Err(io::Error::other("kernel did not report the mount ID"));
    }
    Ok(stat.stx_mnt_id)
}

fn remove_tree(path: &Path, mount: u64, btrfs: bool) -> io::Result<()> {
    // The backend's detached trash worker may delete entries concurrently.
    match remove_entry(path, mount, btrfs) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
}

fn remove_entry(path: &Path, mount: u64, btrfs: bool) -> io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() {
        return fs::remove_file(path);
    }
    // st_dev can change across btrfs subvolumes even without a nested mount.
    if mount_id(path)? != mount {
        return Err(io::Error::other(format!(
            "refusing to cross mount at {}",
            path.display()
        )));
    }
    if btrfs && meta.ino() == 256 {
        // Empty subvolumes can be removed by their owner with rmdir. The btrfs
        // delete ioctl can require root even for subvolumes created by that owner.
        match fs::remove_dir(path) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(_) => {}
        }
        let output = Command::new("btrfs")
            .args(["property", "set", "-ts"])
            .arg(path)
            .args(["ro", "false"])
            .output()
            .map_err(|error| {
                io::Error::other(format!("run btrfs for {}: {error}", path.display()))
            })?;
        if !output.status.success() {
            if let Err(error) = fs::symlink_metadata(path)
                && error.kind() == io::ErrorKind::NotFound
            {
                return Ok(());
            }
            return Err(io::Error::other(format!(
                "make btrfs subvolume {} writable: {}",
                path.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
    }
    if meta.mode() & 0o700 != 0o700 {
        // OverlayFS may leave mode-000 work directories behind.
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    for entry in fs::read_dir(path)? {
        match entry {
            Ok(entry) => remove_tree(&entry.path(), mount, btrfs)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
    }
    fs::remove_dir(path)
}

#[cfg(test)]
#[test]
fn scratch_cleanup_removes_readonly_snapshots_during_unwind() {
    let root = ScratchDir::new("/var/tmp", "ascout-scratch-regression-").unwrap();
    if !root.btrfs {
        eprintln!("SKIP: /var/tmp is not btrfs");
        return;
    }
    if Command::new("btrfs").arg("version").output().is_err() {
        eprintln!("SKIP: btrfs is unavailable");
        return;
    }
    let run = ScratchDir::new(root.path(), "run-").unwrap();
    let path = run.path().to_owned();
    let original = path.join("original");
    let nested = original.join("nested");
    let snapshot = path.join("snapshot");
    let sentinel = root.path().join("keep");
    fs::write(&sentinel, "keep").unwrap();
    for dir in [&original, &nested] {
        assert!(
            Command::new("btrfs")
                .args(["subvolume", "create"])
                .arg(dir)
                .status()
                .unwrap()
                .success()
        );
        fs::write(dir.join("file"), "fixture").unwrap();
    }
    assert!(
        Command::new("btrfs")
            .args(["subvolume", "snapshot", "-r"])
            .args([&original, &snapshot])
            .status()
            .unwrap()
            .success()
    );
    std::os::unix::fs::symlink(root.path(), path.join("external")).unwrap();
    assert!(
        std::panic::catch_unwind(move || {
            let _run = run;
            panic!("fixture failure");
        })
        .is_err()
    );
    assert!(!path.exists());
    assert_eq!(fs::read_to_string(sentinel).unwrap(), "keep");
    root.close().unwrap();
}
