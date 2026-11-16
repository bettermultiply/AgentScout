//! 将 delta 重放到目标 upper 或 live mount，并回滚已创建项。

use std::collections::BTreeSet;
use std::ffi::CString;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};

use super::super::copy::{clone_upper, copy_file, copy_overlay_xattrs, reflink_file, set_times};
use super::super::layer::*;
use super::super::merge::source_content_source;
use super::super::util::*;
use super::safe_root::SafeRoot;
use crate::runtime::backend::BackendError;

const COPY_WARNING_BYTES: u64 = 64 * 1024 * 1024;

struct JournalEntry {
    upper: PathBuf,
    relative: String,
    backup: Option<PathBuf>,
}

/// Operation journal for accept replay.  It records the target-upper state
/// before each overwrite/delete and can reverse every recorded operation.
pub(crate) struct ReplayJournal {
    root: PathBuf,
    entries: Vec<JournalEntry>,
    seen: BTreeSet<(PathBuf, String)>,
    copy_warnings: usize,
    fail_rollback: bool,
}

impl ReplayJournal {
    #[cfg(test)]
    pub(crate) fn new(root: PathBuf) -> Result<Self, BackendError> {
        Self::new_with_rollback_failure(root, false)
    }

    pub(crate) fn new_with_rollback_failure(
        root: PathBuf,
        fail_rollback: bool,
    ) -> Result<Self, BackendError> {
        reset_dir(&root)?;
        fs::create_dir_all(root.join("backups")).map_err(error)?;
        Ok(Self {
            root,
            entries: Vec::new(),
            seen: BTreeSet::new(),
            copy_warnings: 0,
            fail_rollback,
        })
    }

    fn record(&mut self, upper: &Path, relative: &str) -> Result<(), BackendError> {
        let key = (upper.to_path_buf(), relative.to_string());
        if !self.seen.insert(key) {
            return Ok(());
        }
        let target = safe_join(upper, relative)?;
        let backup = if path_exists(&target) {
            let backup = self
                .root
                .join("backups")
                .join(self.entries.len().to_string());
            backup_node(&target, &backup, &mut self.copy_warnings)?;
            Some(backup)
        } else {
            None
        };
        self.entries.push(JournalEntry {
            upper: upper.to_path_buf(),
            relative: relative.to_string(),
            backup,
        });
        Ok(())
    }

    pub(crate) fn rollback(mut self) -> (Vec<String>, usize) {
        let mut failures = Vec::new();
        if self.fail_rollback {
            for entry in self.entries.drain(..).rev() {
                failures.push(format!("{}: injected rollback failure", entry.relative));
            }
            return (failures, self.copy_warnings);
        }
        for entry in self.entries.drain(..).rev() {
            let target = safe_join(&entry.upper, &entry.relative)
                .unwrap_or_else(|_| entry.upper.join(entry.relative.trim_start_matches('/')));
            if let Err(error) = remove_visible(&target) {
                failures.push(format!("{}: {error}", entry.relative));
                continue;
            }
            if let Some(backup) = entry.backup
                && let Err(error) = backup_node(&backup, &target, &mut 0)
            {
                failures.push(format!("{}: {error}", entry.relative));
            }
        }
        if failures.is_empty() {
            let _ = remove_directory_force(&self.root);
        }
        (failures, self.copy_warnings)
    }

    pub(crate) fn sync(&self) -> Result<(), BackendError> {
        if let Ok(directory) = fs::File::open(self.root.join("backups")) {
            directory.sync_all().map_err(error)?;
        }
        if let Ok(directory) = fs::File::open(&self.root) {
            directory.sync_all().map_err(error)?;
        }
        Ok(())
    }

    pub(crate) fn cleanup(self) {
        let _ = remove_directory_force(&self.root);
    }
}

fn backup_node(
    source: &Path,
    target: &Path,
    copy_warnings: &mut usize,
) -> Result<(), BackendError> {
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(error)?;
    }
    let metadata = fs::symlink_metadata(source).map_err(error)?;
    let file_type = metadata.file_type();
    if file_type.is_dir() {
        clone_upper(source, target, true)
    } else if file_type.is_symlink() {
        let link = fs::read_link(source).map_err(error)?;
        symlink(link, target).map_err(error)?;
        set_times(target, &metadata)
    } else if file_type.is_char_device() && metadata.rdev() == 0 {
        let c_path = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| BackendError::new("invalid whiteout path"))?;
        if unsafe {
            libc::mknod(
                c_path.as_ptr(),
                libc::S_IFCHR | (metadata.mode() & 0o7777),
                libc::makedev(0, 0),
            )
        } != 0
        {
            return Err(error(io::Error::last_os_error()));
        }
        fs::set_permissions(target, fs::Permissions::from_mode(metadata.mode() & 0o7777))
            .map_err(error)
    } else if file_type.is_file() {
        let cloned = reflink_file(source, target).is_ok();
        if !cloned {
            if metadata.size() > COPY_WARNING_BYTES {
                *copy_warnings += 1;
                eprintln!(
                    "filesystem: accept rollback backup copied {} bytes without reflink: {}",
                    metadata.size(),
                    source.display()
                );
            }
            copy_file(source, target)?;
        }
        fs::set_permissions(target, fs::Permissions::from_mode(metadata.mode() & 0o7777))
            .map_err(error)?;
        copy_overlay_xattrs(source, target)?;
        set_times(target, &metadata)
    } else {
        let c_path = CString::new(target.as_os_str().as_bytes())
            .map_err(|_| BackendError::new("invalid special file path"))?;
        if unsafe { libc::mknod(c_path.as_ptr(), metadata.mode(), metadata.rdev()) } != 0 {
            return Err(error(io::Error::last_os_error()));
        }
        set_times(target, &metadata)
    }
}

pub(crate) fn remove_visible(path: &Path) -> Result<(), BackendError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => remove_directory_force(path),
        Ok(_) => {
            fs::remove_file(path).map_err(error)?;
            Ok(())
        }
        Err(value) if value.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(value) => Err(error(value)),
    }
}

pub(crate) enum ReplayRoot<'a> {
    Mount { root: &'a SafeRoot, prefix: String },
    Upper(&'a Path),
}

pub(crate) fn replay_delta(
    root: &ReplayRoot<'_>,
    delta: &View,
    redirects: &[(String, String)],
    source_maps: &[View],
    journal: &mut ReplayJournal,
    target_upper: &Path,
    fail_after: Option<usize>,
) -> Result<(), BackendError> {
    let mut applied = 0usize;
    let mut apply = |path: &str, node: &Node| -> Result<(), BackendError> {
        if fail_after == Some(applied) {
            return Err(BackendError::new(format!(
                "injected replay failure before operation {applied} at {path}"
            )));
        }
        journal.record(target_upper, path)?;
        let result = replay_node(root, path, node, source_maps);
        applied += 1;
        match result {
            Ok(_) => Ok(()),
            Err(value) => Err(BackendError::new(format!(
                "replay failed at {path}: {value}"
            ))),
        }
    };
    let run = (|| -> Result<(), BackendError> {
        let mut paths: Vec<(&String, &Node)> = delta.iter().collect();
        paths.sort_by_key(|(path, _)| path.len());
        for (path, node) in paths {
            apply(path, node)?;
        }
        for (old, new) in redirects {
            apply(
                old,
                &Node {
                    mode: libc::S_IFCHR | 0o600,
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
            )?;
            let mut renamed: BTreeSet<String> = BTreeSet::new();
            let old_prefix = format!("{old}/");
            let new_prefix = format!("{new}/");
            for map in source_maps {
                for key in map.keys() {
                    if key == old {
                        renamed.insert(new.clone());
                    } else if key.starts_with(&old_prefix) {
                        renamed.insert(format!("{new_prefix}{}", &key[old_prefix.len()..]));
                    } else if key == new || key.starts_with(&new_prefix) {
                        renamed.insert(key.clone());
                    }
                }
            }
            let mut renamed: Vec<String> = renamed.into_iter().collect();
            renamed.sort_by_key(|path| path.len());
            for path in renamed {
                if let Some(node) = resolve_node(source_maps, &path) {
                    apply(&path, &node)?;
                }
            }
        }
        Ok(())
    })();
    run
}

pub(crate) fn replay_node(
    root: &ReplayRoot<'_>,
    path: &str,
    node: &Node,
    source_maps: &[View],
) -> Result<(), BackendError> {
    match root {
        ReplayRoot::Upper(upper) => {
            let target = safe_join(upper, path)?;
            let existed = path_exists(&target);
            if node.opaque {
                remove_directory_force(&target)?;
                return Ok(());
            }
            stage_node(upper, path, node, source_maps)?;
            match &node.kind {
                NodeKind::Whiteout | NodeKind::Other(_) => Ok(()),
                NodeKind::Dir if !existed => Ok(()),
                NodeKind::File if !existed => Ok(()),
                NodeKind::Symlink(_) if !existed => Ok(()),
                _ => Ok(()),
            }
        }
        ReplayRoot::Mount { root, prefix } => {
            let target = format!("{prefix}/{path}");
            let existed = root.exists(&target)?;
            match &node.kind {
                NodeKind::Whiteout => {
                    root.remove(&target)?;
                    Ok(())
                }
                NodeKind::Dir => {
                    if node.opaque {
                        root.remove(&target)?;
                        root.mkdir(&target, node.mode)?;
                        Ok(())
                    } else {
                        root.mkdir(&target, node.mode)?;
                        if existed { Ok(()) } else { Ok(()) }
                    }
                }
                NodeKind::File => {
                    let source = if node.metacopy {
                        source_content_source(source_maps, path)
                    } else {
                        node.source.clone()
                    }
                    .ok_or_else(|| BackendError::new(format!("no content source for {path}")))?;
                    root.write_file(&target, &source, node.mode)?;
                    if existed { Ok(()) } else { Ok(()) }
                }
                NodeKind::Symlink(link) => {
                    if existed {
                        root.remove(&target)?;
                    }
                    root.symlink(&target, link)?;
                    if existed { Ok(()) } else { Ok(()) }
                }
                NodeKind::Other(kind) => Err(BackendError::new(format!(
                    "cannot replay special file {path} (mode 0o{kind:o})"
                ))),
            }
        }
    }
}

pub(crate) fn stage_node(
    root: &Path,
    path: &str,
    node: &Node,
    source_maps: &[View],
) -> Result<(), BackendError> {
    let target = safe_join(root, path)?;
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(error)?;
    }
    match &node.kind {
        NodeKind::Whiteout => create_whiteout(root, path),
        NodeKind::Dir => {
            if let Ok(metadata) = fs::symlink_metadata(&target)
                && !metadata.is_dir()
            {
                remove_visible(&target)?;
            }
            fs::create_dir_all(&target).map_err(error)?;
            if node.opaque {
                set_xattr(&target, "user.overlay.opaque", b"y")?;
            }
            Ok(())
        }
        NodeKind::File => {
            let source = if node.metacopy {
                source_content_source(source_maps, path)
            } else {
                node.source.clone()
            }
            .ok_or_else(|| BackendError::new(format!("no content source for {path}")))?;
            if let Ok(metadata) = fs::symlink_metadata(&target)
                && !metadata.file_type().is_file()
            {
                remove_visible(&target)?;
            }
            copy_file(&source, &target)?;
            fs::set_permissions(&target, fs::Permissions::from_mode(node.mode & 0o7777))
                .map_err(error)?;
            Ok(())
        }
        NodeKind::Symlink(link) => {
            let _ = fs::remove_file(&target);
            symlink(link, &target).map_err(error)?;
            Ok(())
        }
        NodeKind::Other(kind) => Err(BackendError::new(format!(
            "cannot stage special file {path} (mode 0o{kind:o})"
        ))),
    }
}
