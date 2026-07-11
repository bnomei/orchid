//! `.orchid/` runtime persistence: leases, packets, reports, and the command lock.
//!
//! Mutating commands acquire a short-lived [`RuntimeLock`] backed by an operating-system
//! advisory lock. The open file descriptor owns the lock, so process exit releases it
//! without timestamp-based lock stealing.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};
use fs2::FileExt;
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};

use crate::core::{
    elapsed_seconds, now_iso, parse_duration, parse_iso_datetime, utc_now, ErrorCode, OrchError,
    OrchResult, DEFAULT_STALE_AFTER,
};
use crate::model::{
    lease_status_field_error, validate_lease_id, CompactLease, LeaseId, LeaseRecord,
    ReasoningEffort,
};
use crate::paths::{
    atomic_write_json, buds_dir, leases_dir, locks_dir, orch_dir, packets_dir, path_to_string,
    read_text, relpath, repo_path, reports_dir, spec_research_root,
};
use crate::specs::safe_spec_id;

pub(crate) struct RuntimeLock {
    file: File,
    metadata_path: PathBuf,
}

impl Drop for RuntimeLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.metadata_path);
        let _ = FileExt::unlock(&self.file);
    }
}

/// Acquire the repo-wide runtime lock for the lifetime of the returned file descriptor.
pub(crate) fn runtime_lock(root: &Path) -> OrchResult<RuntimeLock> {
    let lock_dir = repo_path(root, locks_dir(root), "lock_dir")?;
    fs::create_dir_all(&lock_dir)?;
    let lock_dir = repo_path(root, lock_dir, "lock_dir")?;
    let meta = fs::symlink_metadata(&lock_dir)?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(
            OrchError::coded("path outside repo", ErrorCode::PathOutsideRepo)
                .detail("lock_dir", path_to_string(&lock_dir)),
        );
    }
    let path = repo_path(root, lock_dir.join("state.lock"), "lock_path")?;
    if path
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return Err(
            OrchError::coded("path outside repo", ErrorCode::PathOutsideRepo)
                .detail("lock", relpath(&path, root)),
        );
    }
    let metadata_path = repo_path(root, lock_dir.join("state.json"), "lock_metadata_path")?;
    try_acquire_lock(&path, &metadata_path, root)
}

fn try_acquire_lock(path: &Path, metadata_path: &Path, root: &Path) -> OrchResult<RuntimeLock> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(OrchError::from)?;
    if let Err(error) = FileExt::try_lock_exclusive(&file) {
        let mut failure = OrchError::coded("runtime lock busy", ErrorCode::RuntimeLockBusy)
            .detail("lock", relpath(path, root))
            .detail("message", error.to_string());
        if let Ok(Value::Object(metadata)) = read_text(metadata_path)
            .and_then(|raw| serde_json::from_str::<Value>(&raw).map_err(OrchError::from))
        {
            if let Some(pid) = metadata.get("pid") {
                failure = failure.detail("owner_pid", pid.clone());
            }
            if let Some(created_at) = metadata.get("created_at") {
                failure = failure.detail("created_at", created_at.clone());
                if let Some(stamp) = parse_iso_datetime(Some(created_at)) {
                    failure =
                        failure.detail("age_seconds", (utc_now() - stamp).num_seconds().max(0));
                }
            }
        }
        return Err(failure);
    }
    let metadata = serde_json::json!({"pid": std::process::id(), "created_at": now_iso()});
    if let Err(error) = atomic_write_json(metadata_path, &metadata) {
        let _ = FileExt::unlock(&file);
        return Err(error);
    }
    Ok(RuntimeLock {
        file,
        metadata_path: metadata_path.to_path_buf(),
    })
}

pub(crate) fn active_leases(root: &Path) -> OrchResult<Vec<LeaseRecord>> {
    Ok(all_leases(root)?
        .into_iter()
        .filter(|lease| lease.status().is_active())
        .collect())
}

#[derive(Debug, Clone)]
pub(crate) struct CorruptLeaseFile {
    pub(crate) lease_id: String,
    pub(crate) path: String,
    pub(crate) code: String,
    pub(crate) error: String,
}

impl CorruptLeaseFile {
    fn new(
        lease_id: impl Into<String>,
        path: impl Into<String>,
        code: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        Self {
            lease_id: lease_id.into(),
            path: path.into(),
            code: code.into(),
            error: error.into(),
        }
    }

    pub(crate) fn is_invalid_lease_id(&self) -> bool {
        self.code == ErrorCode::InvalidLeaseId.as_str()
    }

    pub(crate) fn to_payload(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("lease_id".to_string(), Value::String(self.lease_id.clone()));
        map.insert("path".to_string(), Value::String(self.path.clone()));
        map.insert("code".to_string(), Value::String(self.code.clone()));
        map.insert("error".to_string(), Value::String(self.error.clone()));
        map
    }

    fn to_error(&self) -> OrchError {
        OrchError::coded("cannot parse lease file", ErrorCode::CorruptLeaseFile)
            .detail("lease_id", self.lease_id.clone())
            .detail("path", self.path.clone())
            .detail("error", self.error.clone())
    }
}

pub(crate) struct LeaseScan {
    pub(crate) leases: Vec<LeaseRecord>,
    pub(crate) corrupt_leases: Vec<CorruptLeaseFile>,
}

pub(crate) fn all_leases(root: &Path) -> OrchResult<Vec<LeaseRecord>> {
    let scan = scan_leases(root)?;
    if let Some(corrupt) = scan.corrupt_leases.first() {
        return Err(corrupt.to_error());
    }
    Ok(scan.leases)
}

pub(crate) fn active_leases_lenient(root: &Path) -> OrchResult<LeaseScan> {
    let mut scan = scan_leases(root)?;
    scan.leases.retain(|lease| lease.status().is_active());
    Ok(scan)
}

pub(crate) fn scan_leases(root: &Path) -> OrchResult<LeaseScan> {
    let path = leases_dir(root);
    if !path.exists() {
        return Ok(LeaseScan {
            leases: Vec::new(),
            corrupt_leases: Vec::new(),
        });
    }
    repo_path(root, &path, "leases_dir")?;
    let mut lease_paths: Vec<PathBuf> = fs::read_dir(path)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    lease_paths.sort();

    let mut leases = Vec::new();
    let mut corrupt_leases = Vec::new();
    for lease_path in lease_paths {
        let Some(expected_lease_id) = lease_path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        if validate_lease_id(expected_lease_id).is_err() {
            continue;
        }
        let lease_path = repo_path(root, &lease_path, "lease_path")?;
        let path = relpath(&lease_path, root);
        let raw = match fs::read_to_string(&lease_path) {
            Ok(raw) => raw,
            Err(err) => {
                corrupt_leases.push(CorruptLeaseFile::new(
                    expected_lease_id,
                    path,
                    ErrorCode::CorruptLeaseFile.as_str(),
                    err.to_string(),
                ));
                continue;
            }
        };
        let parsed = match serde_json::from_str::<Value>(&raw) {
            Ok(parsed) => parsed,
            Err(err) => {
                corrupt_leases.push(CorruptLeaseFile::new(
                    expected_lease_id,
                    path,
                    ErrorCode::CorruptLeaseFile.as_str(),
                    err.to_string(),
                ));
                continue;
            }
        };
        let Value::Object(mut data) = parsed else {
            corrupt_leases.push(CorruptLeaseFile::new(
                expected_lease_id,
                path,
                ErrorCode::CorruptLeaseFile.as_str(),
                "lease file is not a json object",
            ));
            continue;
        };
        if let Err(err) = bind_lease_record_id(&mut data, expected_lease_id) {
            corrupt_leases.push(CorruptLeaseFile::new(
                expected_lease_id,
                path,
                err.code,
                err.message,
            ));
            continue;
        }
        if let Some(error) = lease_status_field_error(&data) {
            corrupt_leases.push(CorruptLeaseFile::new(
                expected_lease_id,
                path,
                ErrorCode::CorruptLeaseFile.as_str(),
                error,
            ));
            continue;
        }
        data.entry("_path".to_string())
            .or_insert_with(|| Value::String(path));
        leases.push(LeaseRecord::from_map(data));
    }
    Ok(LeaseScan {
        leases,
        corrupt_leases,
    })
}

pub(crate) fn load_lease(root: &Path, lease_id: &str) -> OrchResult<LeaseRecord> {
    validate_lease_id(lease_id)?;
    let path = repo_path(
        root,
        leases_dir(root).join(format!("{lease_id}.json")),
        "lease_path",
    )?;
    if !path.exists() {
        return Err(
            OrchError::coded("lease not found", ErrorCode::LeaseNotFound)
                .detail("lease_id", lease_id),
        );
    }
    let Value::Object(mut data) = serde_json::from_str::<Value>(&fs::read_to_string(&path)?)?
    else {
        return Err(OrchError::new("invalid lease json").detail("lease_id", lease_id));
    };
    bind_lease_record_id(&mut data, lease_id)?;
    if let Some(error) = lease_status_field_error(&data) {
        return Err(CorruptLeaseFile::new(
            lease_id,
            relpath(&path, root),
            ErrorCode::CorruptLeaseFile.as_str(),
            error,
        )
        .to_error());
    }
    data.entry("_path".to_string())
        .or_insert_with(|| Value::String(relpath(&path, root)));
    Ok(LeaseRecord::from_map(data))
}

/// Filename is authoritative: fill missing JSON `lease_id`, reject mismatches as corrupt.
fn bind_lease_record_id(
    data: &mut serde_json::Map<String, Value>,
    expected: &str,
) -> OrchResult<()> {
    match data.get("lease_id").and_then(Value::as_str) {
        Some(actual) if actual == expected => Ok(()),
        Some(actual) => Err(
            OrchError::coded("invalid lease id", ErrorCode::InvalidLeaseId)
                .detail("lease_id", actual)
                .detail("expected_lease_id", expected),
        ),
        None => {
            data.insert("lease_id".to_string(), Value::String(expected.to_string()));
            Ok(())
        }
    }
}

pub(crate) fn save_lease(root: &Path, lease: &LeaseRecord) -> OrchResult<()> {
    let lease_id = lease
        .id()
        .ok_or_else(|| OrchError::new("lease missing lease_id"))?;
    validate_lease_id(lease_id)?;
    let path = repo_path(
        root,
        leases_dir(root).join(format!("{lease_id}.json")),
        "lease_path",
    )?;
    let mut data = lease.raw().clone();
    data.retain(|key, _| !key.starts_with('_'));
    atomic_write_json(&path, &data)
}

pub(crate) fn lease_stale(lease: &LeaseRecord, now: DateTime<Utc>, stale_after: TimeDelta) -> bool {
    let heartbeat = lease
        .heartbeat_or_started()
        .and_then(|value| parse_iso_datetime(Some(value)));
    heartbeat.is_none_or(|stamp| stamp < now - stale_after)
}

pub(crate) fn compact_lease(
    lease: &LeaseRecord,
    now: Option<DateTime<Utc>>,
    stale_after: Option<TimeDelta>,
) -> OrchResult<CompactLease> {
    let now = now.unwrap_or_else(utc_now);
    let stale_after = match stale_after {
        Some(value) => value,
        None => parse_duration(DEFAULT_STALE_AFTER)?,
    };
    Ok(CompactLease {
        id: lease.id_value(),
        task: lease.task_value(),
        owner: lease.owner_value(),
        kind: lease.kind().as_str().to_string(),
        agent_id: lease.agent_id().map(str::to_string),
        worker_reasoning_effort: lease
            .worker_reasoning_effort()
            .unwrap_or(ReasoningEffort::Medium.as_str())
            .to_string(),
        worker_model: lease.worker_model().map(str::to_string),
        mode: lease.mode(),
        age: elapsed_seconds(lease.heartbeat_or_started(), now),
        stale: lease_stale(lease, now, stale_after),
    })
}

pub(crate) fn report_path_for_lease(root: &Path, lease: &LeaseRecord) -> OrchResult<PathBuf> {
    let lease_id = lease
        .id()
        .ok_or_else(|| OrchError::new("lease missing lease_id"))?;
    validate_lease_id(lease_id)?;
    repo_path(
        root,
        reports_dir(root).join(format!("{lease_id}.md")),
        "report_path",
    )
}

fn remove_if_exists(path: &Path, root: &Path, deleted: &mut Vec<String>) -> OrchResult<()> {
    let path = repo_path(root, path, "delete_path")?;
    if path.exists() {
        fs::remove_file(&path)?;
        deleted.push(relpath(&path, root));
    }
    Ok(())
}

fn remove_tree_if_exists(path: &Path, root: &Path, deleted: &mut Vec<String>) -> OrchResult<()> {
    let path = repo_path(root, path, "delete_path")?;
    if path.exists() {
        if fs::symlink_metadata(&path)?.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
        deleted.push(relpath(&path, root));
    }
    Ok(())
}

pub(crate) fn prune_empty_runtime_dirs(root: &Path) -> OrchResult<Vec<String>> {
    let mut removed = Vec::new();
    for path in [
        reports_dir(root),
        packets_dir(root),
        buds_dir(root),
        leases_dir(root),
        locks_dir(root),
        spec_research_root(root),
        orch_dir(root),
    ] {
        let path = repo_path(root, path, "runtime_dir")?;
        if path.is_dir() && fs::remove_dir(&path).is_ok() {
            removed.push(relpath(&path, root));
        }
    }
    Ok(removed)
}

pub(crate) fn close_lease_files(
    root: &Path,
    lease: &LeaseRecord,
) -> OrchResult<(Vec<String>, Vec<String>)> {
    let lease_id = lease
        .id()
        .ok_or_else(|| OrchError::new("lease missing lease_id"))?;
    validate_lease_id(lease_id)?;
    let mut deleted = Vec::new();

    let packet_dir = packets_dir(root);
    if packet_dir.exists() {
        repo_path(root, &packet_dir, "packet_dir")?;
        let mut packets: Vec<PathBuf> = fs::read_dir(packet_dir)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .and_then(|s| s.to_str())
                    .is_some_and(|name| {
                        name.starts_with(&format!("{lease_id}-")) && name.ends_with(".md")
                    })
            })
            .collect();
        packets.sort();
        for packet in packets {
            remove_if_exists(&packet, root, &mut deleted)?;
        }
    }

    remove_if_exists(
        &reports_dir(root).join(format!("{lease_id}.md")),
        root,
        &mut deleted,
    )?;
    remove_if_exists(
        &buds_dir(root).join(format!("{lease_id}.md")),
        root,
        &mut deleted,
    )?;
    remove_if_exists(
        &leases_dir(root).join(format!("{lease_id}.json")),
        root,
        &mut deleted,
    )?;
    Ok((deleted, prune_empty_runtime_dirs(root)?))
}

pub(crate) fn clean_spec_research(
    root: &Path,
    spec_id: &str,
) -> OrchResult<(Vec<String>, Vec<String>)> {
    let mut deleted = Vec::new();
    remove_tree_if_exists(&spec_research_dir(root, spec_id)?, root, &mut deleted)?;
    Ok((deleted, prune_empty_runtime_dirs(root)?))
}

pub(crate) fn spec_research_dir(root: &Path, spec_id: &str) -> OrchResult<PathBuf> {
    Ok(spec_research_root(root).join(safe_spec_id(spec_id)?))
}

pub(crate) fn completed_runtime_leases(root: &Path) -> OrchResult<Vec<LeaseRecord>> {
    Ok(all_leases(root)?
        .into_iter()
        .filter(|lease| lease.status().is_completed())
        .collect())
}

pub(crate) fn cleanup_runtime_leases(root: &Path) -> OrchResult<Vec<LeaseRecord>> {
    Ok(all_leases(root)?
        .into_iter()
        .filter(|lease| lease.status().is_completed() || lease.status().is_released())
        .collect())
}

pub(crate) fn lease_id_for(task_path: &Path, owner: &str) -> LeaseId {
    let seed = format!("{}:{}:{}", path_to_string(task_path), owner, now_iso());
    let mut hasher = Sha1::new();
    hasher.update(seed.as_bytes());
    let digest = hasher.finalize();
    let mut lease_id = String::with_capacity(14);
    lease_id.push_str("l_");
    for byte in digest.as_slice().iter().take(6) {
        lease_id.push(hex_char(byte >> 4));
        lease_id.push(hex_char(byte & 0x0f));
    }
    LeaseId::from_generated(lease_id)
}

fn hex_char(value: u8) -> char {
    match value {
        0..=9 => (b'0' + value) as char,
        10..=15 => (b'a' + value - 10) as char,
        _ => unreachable!("hex nibble is always in range"),
    }
}

impl From<serde_json::Error> for OrchError {
    fn from(error: serde_json::Error) -> Self {
        OrchError::coded("invalid JSON", ErrorCode::InvalidJson)
            .detail("message", error.to_string())
    }
}
