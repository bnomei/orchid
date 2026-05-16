use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, TimeDelta, Utc};
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};

use crate::core::{
    elapsed_seconds, insert, now_iso, parse_duration, parse_iso_datetime, utc_now, ErrorCode,
    OrchError, OrchResult, DEFAULT_STALE_AFTER,
};
use crate::model::{CompactLease, LeaseId, LeaseRecord};
use crate::paths::{
    atomic_write_json, leases_dir, locks_dir, orch_dir, packets_dir, path_to_string, relpath,
    repo_path, reports_dir, spec_research_root,
};
use crate::specs::safe_spec_id;

pub(crate) struct RuntimeLock {
    path: PathBuf,
    root: PathBuf,
}

impl Drop for RuntimeLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(parent) = self.path.parent() {
            let _ = fs::remove_dir(parent);
        }
        let _ = fs::remove_dir(orch_dir(&self.root));
    }
}

pub(crate) fn runtime_lock(root: &Path) -> OrchResult<RuntimeLock> {
    let lock_dir = locks_dir(root);
    fs::create_dir_all(&lock_dir)?;
    let path = lock_dir.join("state.lock");
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&path)
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                OrchError::coded("runtime lock busy", ErrorCode::RuntimeLockBusy)
                    .detail("lock", relpath(&path, root))
            } else {
                OrchError::from(err)
            }
        })?;
    writeln!(
        file,
        "{}",
        serde_json::json!({"pid": std::process::id(), "created_at": now_iso()})
    )?;
    Ok(RuntimeLock {
        path,
        root: root.to_path_buf(),
    })
}

pub(crate) fn active_leases(root: &Path) -> OrchResult<Vec<LeaseRecord>> {
    Ok(all_leases(root)?
        .into_iter()
        .filter(|lease| lease.status().is_active())
        .collect())
}

pub(crate) fn all_leases(root: &Path) -> OrchResult<Vec<LeaseRecord>> {
    let path = leases_dir(root);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let mut lease_paths: Vec<PathBuf> = fs::read_dir(path)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    lease_paths.sort();

    let mut leases = Vec::new();
    for lease_path in lease_paths {
        let Ok(raw) = fs::read_to_string(&lease_path) else {
            continue;
        };
        let Ok(Value::Object(mut data)) = serde_json::from_str::<Value>(&raw) else {
            continue;
        };
        data.entry("_path".to_string())
            .or_insert_with(|| Value::String(relpath(&lease_path, root)));
        leases.push(LeaseRecord::from_map(data));
    }
    Ok(leases)
}

pub(crate) fn load_lease(root: &Path, lease_id: &str) -> OrchResult<LeaseRecord> {
    let path = leases_dir(root).join(format!("{lease_id}.json"));
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
    data.entry("_path".to_string())
        .or_insert_with(|| Value::String(relpath(&path, root)));
    Ok(LeaseRecord::from_map(data))
}

pub(crate) fn save_lease(root: &Path, lease: &LeaseRecord) -> OrchResult<()> {
    let lease_id = lease
        .id()
        .ok_or_else(|| OrchError::new("lease missing lease_id"))?;
    atomic_write_json(
        &leases_dir(root).join(format!("{lease_id}.json")),
        lease.raw(),
    )
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
        lease_id: lease.id_value(),
        task: lease.task_value(),
        owner: lease.owner_value(),
        lease_mode: lease.mode(),
        scope: lease
            .get("scope")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
        heartbeat_at: lease.heartbeat_at(),
        age_seconds: elapsed_seconds(lease.get("started_at"), now),
        heartbeat_age_seconds: elapsed_seconds(lease.heartbeat_or_started(), now),
        stale: lease_stale(lease, now, stale_after),
    })
}

pub(crate) fn runtime_summary(root: &Path) -> OrchResult<Map<String, Value>> {
    let now = utc_now();
    let stale_after = parse_duration(DEFAULT_STALE_AFTER)?;
    let leases: Vec<Value> = active_leases(root)?
        .into_iter()
        .map(|lease| {
            compact_lease(&lease, Some(now), Some(stale_after))
                .map(|lease| Value::Object(lease.to_payload()))
        })
        .collect::<OrchResult<_>>()?;
    let mut map = Map::new();
    insert(&mut map, "active_count", leases.len() as i64);
    insert(&mut map, "active_leases", Value::Array(leases));
    Ok(map)
}

pub(crate) fn with_runtime(
    root: &Path,
    mut payload: Map<String, Value>,
) -> OrchResult<Map<String, Value>> {
    if !payload.contains_key("runtime") {
        payload.insert("runtime".to_string(), Value::Object(runtime_summary(root)?));
    }
    Ok(payload)
}

pub(crate) fn report_path_for_lease(root: &Path, lease: &LeaseRecord) -> OrchResult<PathBuf> {
    if let Some(report_path) = lease.report_path() {
        return repo_path(root, report_path, "report_path");
    }
    let lease_id = lease
        .id()
        .ok_or_else(|| OrchError::new("lease missing lease_id"))?;
    repo_path(
        root,
        reports_dir(root).join(format!("{lease_id}.md")),
        "report_path",
    )
}

fn remove_if_exists(path: &Path, root: &Path, deleted: &mut Vec<String>) -> OrchResult<()> {
    if path.exists() {
        fs::remove_file(path)?;
        deleted.push(relpath(path, root));
    }
    Ok(())
}

fn remove_tree_if_exists(path: &Path, root: &Path, deleted: &mut Vec<String>) -> OrchResult<()> {
    if path.exists() {
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
        deleted.push(relpath(path, root));
    }
    Ok(())
}

pub(crate) fn prune_empty_runtime_dirs(root: &Path) -> Vec<String> {
    let mut removed = Vec::new();
    for path in [
        reports_dir(root),
        packets_dir(root),
        leases_dir(root),
        locks_dir(root),
        spec_research_root(root),
        orch_dir(root),
    ] {
        if path.is_dir() && fs::remove_dir(&path).is_ok() {
            removed.push(relpath(&path, root));
        }
    }
    removed
}

pub(crate) fn close_lease_files(
    root: &Path,
    lease: &LeaseRecord,
) -> OrchResult<(Vec<String>, Vec<String>)> {
    let lease_id = lease
        .id()
        .ok_or_else(|| OrchError::new("lease missing lease_id"))?;
    let mut deleted = Vec::new();
    remove_if_exists(
        &leases_dir(root).join(format!("{lease_id}.json")),
        root,
        &mut deleted,
    )?;

    let packet_dir = packets_dir(root);
    if packet_dir.exists() {
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

    if lease.report_path().is_some() {
        remove_if_exists(&report_path_for_lease(root, lease)?, root, &mut deleted)?;
    }
    remove_if_exists(
        &reports_dir(root).join(format!("{lease_id}.md")),
        root,
        &mut deleted,
    )?;
    Ok((deleted, prune_empty_runtime_dirs(root)))
}

pub(crate) fn clean_spec_research(
    root: &Path,
    spec_id: &str,
) -> OrchResult<(Vec<String>, Vec<String>)> {
    let mut deleted = Vec::new();
    remove_tree_if_exists(&spec_research_dir(root, spec_id)?, root, &mut deleted)?;
    Ok((deleted, prune_empty_runtime_dirs(root)))
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
    LeaseId::from_raw(lease_id)
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
