use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::core::{ErrorCode, OrchError, OrchResult};

pub(crate) fn root_from_arg(value: Option<&str>) -> OrchResult<PathBuf> {
    let path = if let Some(value) = value {
        expand_home(value)
    } else {
        env::current_dir()?
    };
    abs_clean(path)
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

pub(crate) fn reports_dir(root: &Path) -> PathBuf {
    orch_dir(root).join("reports")
}

pub(crate) fn spec_research_root(root: &Path) -> PathBuf {
    orch_dir(root).join("spec-research")
}

pub(crate) fn ensure_runtime_dirs(root: &Path) -> OrchResult<()> {
    for path in [leases_dir(root), packets_dir(root), reports_dir(root)] {
        fs::create_dir_all(path)?;
    }
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

pub(crate) fn ensure_under_root(path: PathBuf, root: &Path, label: &str) -> OrchResult<PathBuf> {
    let path = abs_clean(path)?;
    let root = abs_clean(root.to_path_buf())?;
    if !path.starts_with(&root) {
        return Err(
            OrchError::coded("path outside repo", ErrorCode::PathOutsideRepo)
                .detail(label, path_to_string(&path)),
        );
    }
    Ok(path)
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

pub(crate) fn atomic_write(path: &Path, data: &str) -> OrchResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_file_name(format!(
        ".{}.{}.tmp",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("tmp"),
        std::process::id()
    ));
    fs::write(&tmp, data)?;
    fs::rename(tmp, path)?;
    Ok(())
}

pub(crate) fn atomic_write_json<T: Serialize>(path: &Path, data: &T) -> OrchResult<()> {
    let mut text = serde_json::to_string_pretty(data).expect("json encoding");
    text.push('\n');
    atomic_write(path, &text)
}

pub(crate) fn path_to_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
