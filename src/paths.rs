//! Repository root discovery, `.orchid/` layout helpers, and confined filesystem I/O.
//!
//! All repo-bound paths are normalized and checked to stay under the discovered root;
//! symlinked runtime directories are rejected before mutating commands touch them.

use std::env;
use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::Serialize;

use crate::core::{ErrorCode, OrchError, OrchResult};

pub(crate) fn root_from_arg(value: Option<&str>) -> OrchResult<PathBuf> {
    if let Some(value) = value {
        return abs_clean(expand_home(value));
    }
    let path = abs_clean(env::current_dir()?)?;
    Ok(discover_git_root(&path)
        .or_else(|| discover_orchid_root(&path))
        .unwrap_or(path))
}

fn expand_home(value: &str) -> PathBuf {
    if value == "~" {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home);
        }
    }
    if let Some(rest) = value.strip_prefix("~/") {
        if let Some(home) = env::var_os("HOME") {
            return PathBuf::from(home).join(rest);
        }
    }
    PathBuf::from(value)
}

fn abs_clean(path: PathBuf) -> OrchResult<PathBuf> {
    let absolute = if path.is_absolute() {
        path
    } else {
        env::current_dir()?.join(path)
    };
    Ok(clean_path(&absolute))
}

fn clean_path(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Walk ancestors from `path` until a `.orchid` marker directory is found.
pub(crate) fn discover_orchid_root(path: &Path) -> Option<PathBuf> {
    let start = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    start
        .ancestors()
        .find(|ancestor| {
            fs::symlink_metadata(ancestor.join(".orchid"))
                .map(|meta| meta.is_dir())
                .unwrap_or(false)
        })
        .map(Path::to_path_buf)
}

fn discover_git_root(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["-C", path.to_str()?, "rev-parse", "--show-toplevel"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8(output.stdout).ok()?;
    let root = root.trim();
    if root.is_empty() {
        return None;
    }
    abs_clean(PathBuf::from(root)).ok()
}

pub(crate) fn orch_dir(root: &Path) -> PathBuf {
    root.join(".orchid")
}

pub(crate) fn locks_dir(root: &Path) -> PathBuf {
    orch_dir(root).join("locks")
}

pub(crate) fn leases_dir(root: &Path) -> PathBuf {
    orch_dir(root).join("leases")
}

pub(crate) fn packets_dir(root: &Path) -> PathBuf {
    orch_dir(root).join("packets")
}

pub(crate) fn buds_dir(root: &Path) -> PathBuf {
    orch_dir(root).join("buds")
}

pub(crate) fn reports_dir(root: &Path) -> PathBuf {
    orch_dir(root).join("reports")
}

#[allow(dead_code)]
pub(crate) fn goal_current_path(root: &Path) -> PathBuf {
    orch_dir(root).join("goal-current")
}

#[allow(dead_code)]
pub(crate) fn goals_dir(root: &Path) -> PathBuf {
    orch_dir(root).join("goals")
}

#[allow(dead_code)]
pub(crate) fn goal_dir(root: &Path, goal_id: &str) -> PathBuf {
    goals_dir(root).join(goal_id)
}

pub(crate) fn spec_research_root(root: &Path) -> PathBuf {
    orch_dir(root).join("spec-research")
}

pub(crate) fn ensure_runtime_dirs(root: &Path) -> OrchResult<()> {
    for (label, path) in [
        ("leases_dir", leases_dir(root)),
        ("packets_dir", packets_dir(root)),
        ("buds_dir", buds_dir(root)),
        ("reports_dir", reports_dir(root)),
    ] {
        ensure_under_root(path.clone(), root, label)?;
        fs::create_dir_all(&path)?;
        ensure_runtime_dir(root, &path, label)?;
    }
    Ok(())
}

fn ensure_runtime_dir(root: &Path, path: &Path, label: &str) -> OrchResult<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(
            OrchError::coded("path outside repo", ErrorCode::PathOutsideRepo)
                .detail(label, path_to_string(path)),
        );
    }
    ensure_under_root(path.to_path_buf(), root, label)?;
    Ok(())
}

pub(crate) fn relpath(path: &Path, root: &Path) -> String {
    let clean = clean_path(path);
    let root = clean_path(root);
    clean
        .strip_prefix(root)
        .map(path_to_string)
        .unwrap_or_else(|_| path_to_string(&clean))
}

/// Resolve `path` under `root`, rejecting escapes through `..` or symlinks.
pub(crate) fn ensure_under_root(path: PathBuf, root: &Path, label: &str) -> OrchResult<PathBuf> {
    let path = abs_clean(path)?;
    let root = abs_clean(root.to_path_buf())?;
    if !path.starts_with(&root) {
        return Err(
            OrchError::coded("path outside repo", ErrorCode::PathOutsideRepo)
                .detail(label, path_to_string(&path)),
        );
    }
    ensure_canonical_under_root(&path, &root, label)?;
    Ok(path)
}

fn ensure_canonical_under_root(path: &Path, root: &Path, label: &str) -> OrchResult<()> {
    let root = fs::canonicalize(root)?;
    let anchor = if fs::symlink_metadata(path).is_ok() {
        fs::canonicalize(path)?
    } else {
        canonical_existing_ancestor(path)?
    };
    if !anchor.starts_with(&root) {
        return Err(
            OrchError::coded("path outside repo", ErrorCode::PathOutsideRepo)
                .detail(label, path_to_string(path)),
        );
    }
    Ok(())
}

fn canonical_existing_ancestor(path: &Path) -> OrchResult<PathBuf> {
    for ancestor in path.ancestors().skip(1) {
        if fs::symlink_metadata(ancestor).is_ok() {
            return Ok(fs::canonicalize(ancestor)?);
        }
    }
    Err(OrchError::new("I/O error").detail("message", "path has no existing ancestor"))
}

pub(crate) fn repo_path(root: &Path, value: impl AsRef<Path>, label: &str) -> OrchResult<PathBuf> {
    let value = value.as_ref();
    let path = if value.is_absolute() {
        value.to_path_buf()
    } else {
        root.join(value)
    };
    ensure_under_root(path, root, label)
}

pub(crate) fn read_text(path: &Path) -> OrchResult<String> {
    Ok(fs::read_to_string(path)?)
}

/// Write `data` atomically via a pid-scoped temp file and rename.
pub(crate) fn atomic_write(path: &Path, data: &str) -> OrchResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut last_error = None;
    for attempt in 0..100 {
        let tmp = tmp_path(path, attempt);
        match fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(data.as_bytes()) {
                    let _ = fs::remove_file(&tmp);
                    return Err(error.into());
                }
                if let Err(error) = file.sync_all() {
                    let _ = fs::remove_file(&tmp);
                    return Err(error.into());
                }
                drop(file);
                if let Err(error) = replace_file(&tmp, path) {
                    let _ = fs::remove_file(&tmp);
                    return Err(error);
                }
                return Ok(());
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                last_error = Some(error);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(last_error
        .unwrap_or_else(|| {
            std::io::Error::new(ErrorKind::AlreadyExists, "temporary file already exists")
        })
        .into())
}

fn tmp_path(path: &Path, attempt: u32) -> PathBuf {
    path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("tmp"),
        std::process::id(),
        attempt
    ))
}

fn replace_file(tmp: &Path, path: &Path) -> OrchResult<()> {
    replace_file_with(
        tmp,
        path,
        |from, to| fs::rename(from, to),
        |target| fs::remove_file(target),
    )
}

fn replace_file_with(
    tmp: &Path,
    path: &Path,
    mut rename: impl FnMut(&Path, &Path) -> std::io::Result<()>,
    mut remove_file: impl FnMut(&Path) -> std::io::Result<()>,
) -> OrchResult<()> {
    match rename(tmp, path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::AlreadyExists => {
            replace_file_with_backup(tmp, path, &mut rename, &mut remove_file)
        }
        Err(error) => Err(error.into()),
    }
}

fn replace_file_with_backup(
    tmp: &Path,
    path: &Path,
    rename: &mut impl FnMut(&Path, &Path) -> std::io::Result<()>,
    remove_file: &mut impl FnMut(&Path) -> std::io::Result<()>,
) -> OrchResult<()> {
    let mut last_error = None;
    for attempt in 0..100 {
        let backup = backup_path(path, attempt);
        if fs::symlink_metadata(&backup).is_ok() {
            last_error = Some(std::io::Error::new(
                ErrorKind::AlreadyExists,
                "backup file already exists",
            ));
            continue;
        }
        match rename(path, &backup) {
            Ok(()) => match rename(tmp, path) {
                Ok(()) => {
                    remove_file(&backup)?;
                    return Ok(());
                }
                Err(install_error) => match rename(&backup, path) {
                    Ok(()) => return Err(install_error.into()),
                    Err(restore_error) => {
                        return Err(OrchError::new("I/O error")
                            .detail(
                                "message",
                                format!(
                                    "{install_error}; failed to restore backup: {restore_error}"
                                ),
                            )
                            .detail("backup", path_to_string(&backup)));
                    }
                },
            },
            Err(error) if error.kind() == ErrorKind::AlreadyExists => {
                last_error = Some(error);
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(last_error
        .unwrap_or_else(|| std::io::Error::new(ErrorKind::AlreadyExists, "backup file exists"))
        .into())
}

fn backup_path(path: &Path, attempt: u32) -> PathBuf {
    path.with_file_name(format!(
        ".{}.{}.{}.bak",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("tmp"),
        std::process::id(),
        attempt
    ))
}

pub(crate) fn atomic_write_json<T: Serialize>(path: &Path, data: &T) -> OrchResult<()> {
    let mut text = serde_json::to_string_pretty(data).expect("json encoding");
    text.push('\n');
    atomic_write(path, &text)
}

pub(crate) fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn discover_orchid_root_requires_marker_directory() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let root = tmp.path().join("repo");
        let child = root.join("nested");
        fs::create_dir_all(&child).expect("child dir");
        fs::write(root.join(".orchid"), "not a directory").expect("marker file");

        assert_eq!(discover_orchid_root(&child), None);

        fs::remove_file(root.join(".orchid")).expect("remove marker file");
        fs::create_dir(root.join(".orchid")).expect("marker dir");

        assert_eq!(discover_orchid_root(&child), Some(root));
    }

    #[test]
    fn replace_file_already_exists_fallback_restores_destination_after_install_failure() {
        let tmp_dir = tempfile::TempDir::new().expect("tempdir");
        let path = tmp_dir.path().join("state.json");
        let tmp = tmp_dir
            .path()
            .join(format!(".state.json.{}.0.tmp", std::process::id()));
        fs::write(&path, "original\n").expect("write destination");
        fs::write(&tmp, "replacement\n").expect("write temp");

        let rename_count = Cell::new(0);
        let result = replace_file_with(
            &tmp,
            &path,
            |from, to| {
                rename_count.set(rename_count.get() + 1);
                match rename_count.get() {
                    1 => Err(std::io::Error::new(
                        ErrorKind::AlreadyExists,
                        "simulated replace collision",
                    )),
                    3 => Err(std::io::Error::new(
                        ErrorKind::PermissionDenied,
                        "simulated reinstall failure",
                    )),
                    _ => fs::rename(from, to),
                }
            },
            |target| fs::remove_file(target),
        );

        let error = result.expect_err("install failure should be returned");
        assert_eq!(error.code, "i_o_error");
        assert_eq!(
            fs::read_to_string(&path).expect("destination"),
            "original\n"
        );
        assert_eq!(
            fs::read_to_string(&tmp).expect("temp remains"),
            "replacement\n"
        );
        assert_eq!(rename_count.get(), 4);
    }
}
