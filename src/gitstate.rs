//! Git porcelain integration for lease attribution, touched-file checks, and stage plans.
//!
//! Invokes the `git` CLI from the repo root; returns empty results when Git is
//! unavailable so callers can surface `git: false` instead of aborting.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Map, Value};

use crate::core::{string_list, OrchError, OrchResult};
use crate::model::{LeaseRecord, StagePlan};
use crate::specs::path_in_scope;

#[derive(Debug, Clone)]
struct GitStatusRecord {
    path: String,
    orig_path: Option<String>,
    kind: String,
    index: String,
    worktree: String,
    staged: bool,
    unstaged: bool,
    untracked: bool,
    ignored: bool,
    conflict: bool,
    score: Option<String>,
}

impl GitStatusRecord {
    fn paths(&self) -> Vec<String> {
        let mut paths = Vec::new();
        if let Some(orig_path) = &self.orig_path {
            paths.push(orig_path.clone());
        }
        if !paths.contains(&self.path) {
            paths.push(self.path.clone());
        }
        paths
    }

    fn visible_paths(&self) -> Vec<String> {
        self.paths()
            .into_iter()
            .filter(|path| is_visible_path(path))
            .collect()
    }

    fn primary_visible_path(&self) -> Option<&str> {
        is_visible_path(&self.path).then_some(self.path.as_str())
    }

    fn to_value(&self) -> Value {
        let mut map = Map::new();
        map.insert("path".to_string(), Value::String(self.path.clone()));
        if let Some(orig_path) = &self.orig_path {
            map.insert("orig_path".to_string(), Value::String(orig_path.clone()));
        }
        map.insert("kind".to_string(), Value::String(self.kind.clone()));
        map.insert("index".to_string(), Value::String(self.index.clone()));
        map.insert("worktree".to_string(), Value::String(self.worktree.clone()));
        map.insert("staged".to_string(), Value::Bool(self.staged));
        map.insert("unstaged".to_string(), Value::Bool(self.unstaged));
        if self.untracked {
            map.insert("untracked".to_string(), Value::Bool(true));
        }
        if self.ignored {
            map.insert("ignored".to_string(), Value::Bool(true));
        }
        if self.conflict {
            map.insert("conflict".to_string(), Value::Bool(true));
        }
        if let Some(score) = &self.score {
            map.insert("score".to_string(), Value::String(score.clone()));
        }
        map.insert("paths".to_string(), string_array(self.paths()));
        Value::Object(map)
    }
}

fn git(root: &Path, args: &[&str], check: bool) -> OrchResult<Vec<u8>> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(OrchError::from)?;
    if check && !output.status.success() {
        return Err(OrchError::new("git command failed")
            .detail(
                "args",
                Value::Array(
                    args.iter()
                        .map(|arg| Value::String((*arg).to_string()))
                        .collect(),
                ),
            )
            .detail(
                "stderr",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
    }
    Ok(output.stdout)
}

fn git_owned(root: &Path, args: &[String], check: bool) -> OrchResult<Vec<u8>> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    git(root, &refs, check)
}

fn git_text(root: &Path, args: &[&str], check: bool) -> OrchResult<String> {
    Ok(String::from_utf8_lossy(&git(root, args, check)?)
        .trim()
        .to_string())
}

pub(crate) fn current_branch(root: &Path) -> OrchResult<Option<String>> {
    if !git_available(root) {
        return Ok(None);
    }
    let branch = git_text(root, &["rev-parse", "--abbrev-ref", "HEAD"], true)?;
    Ok((!branch.is_empty() && branch != "HEAD").then_some(branch))
}

pub(crate) fn head_commit(root: &Path) -> OrchResult<Option<String>> {
    if !git_available(root) {
        return Ok(None);
    }
    Ok(Some(git_text(root, &["rev-parse", "HEAD"], true)?))
}

fn git_available(root: &Path) -> bool {
    git(root, &["rev-parse", "--show-toplevel"], true).is_ok()
}

pub(crate) fn git_common_dir(root: &Path) -> Option<PathBuf> {
    let raw = git_text(root, &["rev-parse", "--git-common-dir"], true).ok()?;
    if raw.is_empty() {
        return None;
    }
    let candidate = Path::new(&raw);
    let absolute = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    std::fs::canonicalize(&absolute).ok()
}

fn split_z(data: &[u8]) -> Vec<String> {
    data.split(|byte| *byte == 0)
        .filter(|item| !item.is_empty())
        .map(|item| String::from_utf8_lossy(item).to_string())
        .collect()
}

pub(crate) fn git_status_data(root: &Path) -> OrchResult<Map<String, Value>> {
    if !git_available(root) {
        let mut map = Map::new();
        map.insert("git".to_string(), Value::Bool(false));
        return Ok(map);
    }

    let branch = git_text(root, &["rev-parse", "--abbrev-ref", "HEAD"], true)?;
    let head = git_text(root, &["rev-parse", "HEAD"], true)?;
    let records = git_status_records(root)?;
    let changed = changed_object_from_records(&records);

    let mut map = Map::new();
    map.insert("git".to_string(), Value::Bool(true));
    map.insert("branch".to_string(), Value::String(branch));
    map.insert("head".to_string(), Value::String(head));
    map.insert("clean".to_string(), Value::Bool(records.is_empty()));
    insert_records_if_non_empty(&mut map, "records", &records);
    if changed.as_object().is_some_and(|items| !items.is_empty()) {
        map.insert("changed".to_string(), changed);
    }
    Ok(map)
}

fn git_status_records(root: &Path) -> OrchResult<Vec<GitStatusRecord>> {
    let output = git(
        root,
        &[
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
            "--renames",
        ],
        true,
    )?;
    Ok(parse_porcelain_v2_z(&output)
        .into_iter()
        .filter(|record| !record.visible_paths().is_empty())
        .collect())
}

fn changed_object_from_records(records: &[GitStatusRecord]) -> Value {
    let mut modified = BTreeSet::new();
    let mut staged = BTreeSet::new();
    let mut untracked = BTreeSet::new();

    for record in records {
        let Some(path) = record.primary_visible_path() else {
            continue;
        };
        if record.untracked {
            untracked.insert(path.to_string());
        } else {
            if record.unstaged {
                modified.insert(path.to_string());
            }
            if record.staged {
                staged.insert(path.to_string());
            }
        }
    }

    let mut changed = Map::new();
    insert_array_if_non_empty(&mut changed, "modified", modified.into_iter().collect());
    insert_array_if_non_empty(&mut changed, "staged", staged.into_iter().collect());
    insert_array_if_non_empty(&mut changed, "untracked", untracked.into_iter().collect());
    Value::Object(changed)
}

fn parse_porcelain_v2_z(data: &[u8]) -> Vec<GitStatusRecord> {
    let chunks = split_z(data);
    let mut records = Vec::new();
    let mut index = 0;

    while index < chunks.len() {
        let Some(entry) = porcelain_record_text(&chunks[index]) else {
            index += 1;
            continue;
        };

        match entry.as_bytes().first().copied() {
            Some(b'1') => {
                if let Some(record) = parse_ordinary_record(entry) {
                    records.push(record);
                }
            }
            Some(b'2') => {
                let orig_path = chunks.get(index + 1).cloned().unwrap_or_default();
                if let Some(record) = parse_rename_record(entry, orig_path) {
                    records.push(record);
                    index += 1;
                }
            }
            Some(b'u') => {
                if let Some(record) = parse_unmerged_record(entry) {
                    records.push(record);
                }
            }
            Some(b'?') => {
                if let Some(record) = parse_simple_record(entry, "untracked", true) {
                    records.push(record);
                }
            }
            Some(b'!') => {
                if let Some(record) = parse_simple_record(entry, "ignored", false) {
                    records.push(record);
                }
            }
            Some(b'#') | None | Some(_) => {}
        }

        index += 1;
    }

    records.sort_by_key(|record| record.paths());
    records
}

fn porcelain_record_text(chunk: &str) -> Option<&str> {
    if is_porcelain_record_start(chunk) {
        return Some(chunk);
    }

    chunk
        .split('\n')
        .map(|line| line.trim_start_matches('\r'))
        .find(|line| is_porcelain_record_start(line))
}

fn is_porcelain_record_start(value: &str) -> bool {
    matches!(
        value.as_bytes(),
        [b'1' | b'2' | b'u' | b'?' | b'!' | b'#', b' ', ..]
    )
}

fn parse_ordinary_record(entry: &str) -> Option<GitStatusRecord> {
    let parts: Vec<&str> = entry.splitn(9, ' ').collect();
    let xy = *parts.get(1)?;
    let path = (*parts.get(8)?).to_string();
    let (index, worktree) = xy_codes(xy);
    Some(GitStatusRecord {
        kind: ordinary_kind(index, worktree).to_string(),
        path,
        orig_path: None,
        index: index.to_string(),
        worktree: worktree.to_string(),
        staged: is_staged_code(index),
        unstaged: is_unstaged_code(worktree),
        untracked: false,
        ignored: false,
        conflict: false,
        score: None,
    })
}

fn parse_rename_record(entry: &str, orig_path: String) -> Option<GitStatusRecord> {
    let parts: Vec<&str> = entry.splitn(10, ' ').collect();
    let xy = *parts.get(1)?;
    let score = (*parts.get(8)?).to_string();
    let path = (*parts.get(9)?).to_string();
    let (index, worktree) = xy_codes(xy);
    Some(GitStatusRecord {
        kind: match score.as_bytes().first().copied() {
            Some(b'C') => "copied",
            _ => "renamed",
        }
        .to_string(),
        path,
        orig_path: (!orig_path.is_empty()).then_some(orig_path),
        index: index.to_string(),
        worktree: worktree.to_string(),
        staged: is_staged_code(index),
        unstaged: is_unstaged_code(worktree),
        untracked: false,
        ignored: false,
        conflict: false,
        score: Some(score),
    })
}

fn parse_unmerged_record(entry: &str) -> Option<GitStatusRecord> {
    let parts: Vec<&str> = entry.splitn(11, ' ').collect();
    let xy = *parts.get(1)?;
    let path = (*parts.get(10)?).to_string();
    let (index, worktree) = xy_codes(xy);
    Some(GitStatusRecord {
        kind: "unmerged".to_string(),
        path,
        orig_path: None,
        index: index.to_string(),
        worktree: worktree.to_string(),
        staged: true,
        unstaged: true,
        untracked: false,
        ignored: false,
        conflict: true,
        score: None,
    })
}

fn parse_simple_record(entry: &str, kind: &str, untracked: bool) -> Option<GitStatusRecord> {
    let path = entry.get(2..)?.to_string();
    Some(GitStatusRecord {
        path,
        orig_path: None,
        kind: kind.to_string(),
        index: ".".to_string(),
        worktree: if untracked { "?" } else { "!" }.to_string(),
        staged: false,
        unstaged: untracked,
        untracked,
        ignored: !untracked,
        conflict: false,
        score: None,
    })
}

fn xy_codes(xy: &str) -> (char, char) {
    let mut chars = xy.chars();
    (chars.next().unwrap_or('.'), chars.next().unwrap_or('.'))
}

fn ordinary_kind(index: char, worktree: char) -> &'static str {
    if index == 'D' || worktree == 'D' {
        "deleted"
    } else if index == 'A' || worktree == 'A' {
        "added"
    } else if index == 'T' || worktree == 'T' {
        "typechange"
    } else {
        "modified"
    }
}

fn is_staged_code(value: char) -> bool {
    !matches!(value, '.' | '?' | '!')
}

fn is_unstaged_code(value: char) -> bool {
    !matches!(value, '.' | '!')
}

fn is_visible_path(path: &str) -> bool {
    !path.starts_with(".orchid/")
}

fn string_array(items: Vec<String>) -> Value {
    Value::Array(items.into_iter().map(Value::String).collect())
}

fn insert_array_if_non_empty(map: &mut Map<String, Value>, key: &str, items: Vec<String>) {
    if !items.is_empty() {
        map.insert(key.to_string(), string_array(items));
    }
}

fn records_array(records: &[GitStatusRecord]) -> Value {
    Value::Array(records.iter().map(GitStatusRecord::to_value).collect())
}

fn insert_records_if_non_empty(
    map: &mut Map<String, Value>,
    key: &str,
    records: &[GitStatusRecord],
) {
    if !records.is_empty() {
        map.insert(key.to_string(), records_array(records));
    }
}

fn insert_records_value_if_non_empty(
    map: &mut Map<String, Value>,
    key: &str,
    records: &[GitStatusRecord],
) {
    if !records.is_empty() {
        map.insert(key.to_string(), records_array(records));
    }
}

pub(crate) fn changed_paths_value(status: &Map<String, Value>) -> Value {
    string_array(changed_paths(status))
}

/// Store `completed_changed` only when git was available at complete time.
pub(crate) fn apply_completed_changed_snapshot(
    lease: &mut LeaseRecord,
    status: &Map<String, Value>,
) {
    if status.get("git").and_then(Value::as_bool).unwrap_or(false) {
        lease.set("completed_changed", changed_paths_value(status));
    } else {
        lease.set("completed_changed_unavailable", true);
    }
}

pub(crate) fn status_records_value(status: &Map<String, Value>) -> Value {
    status
        .get("records")
        .cloned()
        .unwrap_or_else(|| Value::Array(Vec::new()))
}

pub(crate) fn changed_paths(status: &Map<String, Value>) -> Vec<String> {
    let mut paths = BTreeSet::new();
    if let Some(records) = status.get("records").and_then(Value::as_array) {
        for record in records {
            for path in string_list(record.get("paths")) {
                paths.insert(path);
            }
        }
        return paths.into_iter().collect();
    }

    if let Some(changed) = status.get("changed").and_then(Value::as_object) {
        for key in ["modified", "staged", "untracked"] {
            for path in string_list(changed.get(key)) {
                paths.insert(path);
            }
        }
    }
    paths.into_iter().collect()
}

fn git_status_records_from_status(status: &Map<String, Value>) -> Vec<GitStatusRecord> {
    status
        .get("records")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(git_status_record_from_value)
        .collect()
}

fn git_status_record_from_value(value: &Value) -> Option<GitStatusRecord> {
    let object = value.as_object()?;
    let path = object.get("path")?.as_str()?.to_string();
    Some(GitStatusRecord {
        path,
        orig_path: object
            .get("orig_path")
            .and_then(Value::as_str)
            .map(str::to_string),
        kind: object.get("kind")?.as_str()?.to_string(),
        index: object
            .get("index")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .to_string(),
        worktree: object
            .get("worktree")
            .and_then(Value::as_str)
            .unwrap_or(".")
            .to_string(),
        staged: object
            .get("staged")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        unstaged: object
            .get("unstaged")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        untracked: object
            .get("untracked")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        ignored: object
            .get("ignored")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        conflict: object
            .get("conflict")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        score: object
            .get("score")
            .and_then(Value::as_str)
            .map(str::to_string),
    })
}

pub(crate) fn visible_changed_paths(root: &Path) -> OrchResult<Vec<String>> {
    Ok(changed_paths(&git_status_data(root)?))
}

pub(crate) fn changed_paths_since(root: &Path, baseline: &str) -> OrchResult<Vec<String>> {
    if !git_available(root) {
        return Ok(Vec::new());
    }
    let out = git(
        root,
        &[
            "diff",
            "--no-renames",
            "--name-only",
            "-z",
            baseline,
            "HEAD",
        ],
        true,
    )?;
    Ok(split_z(&out))
}

pub(crate) fn changed_protected_paths(
    root: &Path,
    protected_surfaces: &[String],
    baseline: Option<&str>,
) -> OrchResult<Vec<String>> {
    let mut changed: Vec<String> = visible_changed_paths(root)?
        .into_iter()
        .filter(|path| path_in_scope(path, protected_surfaces))
        .collect();
    if let Some(baseline) = baseline {
        changed.extend(
            changed_paths_since(root, baseline)?
                .into_iter()
                .filter(|path| path_in_scope(path, protected_surfaces)),
        );
    }
    changed.sort();
    changed.dedup();
    Ok(changed)
}

pub(crate) fn stage_goal_candidates(root: &Path, scope: &[String]) -> OrchResult<Vec<String>> {
    let mut candidates = visible_changed_paths(root)?;
    if !scope.is_empty() {
        candidates.retain(|path| path_in_scope(path, scope));
    }
    if candidates.is_empty() {
        return Ok(candidates);
    }
    let mut args = vec!["add".to_string(), "--".to_string()];
    args.extend(candidates.iter().map(|path| format!(":(literal){path}")));
    git_owned(root, &args, true)?;
    Ok(candidates)
}

pub(crate) fn staged_paths_outside_scope(root: &Path, scope: &[String]) -> OrchResult<Vec<String>> {
    if scope.is_empty() || !git_available(root) {
        return Ok(Vec::new());
    }
    let mut paths = BTreeSet::new();
    for record in git_status_records(root)? {
        if !record.staged {
            continue;
        }
        for path in record.visible_paths() {
            if !path_in_scope(&path, scope) {
                paths.insert(path);
            }
        }
    }
    Ok(paths.into_iter().collect())
}

pub(crate) fn commit_goal_keep(root: &Path, goal_id: &str, cycle: &str) -> OrchResult<String> {
    git(
        root,
        &["commit", "-m", &format!("goal({goal_id}): keep {cycle}")],
        true,
    )?;
    git_text(root, &["rev-parse", "HEAD"], true)
}

pub(crate) fn reset_hard(root: &Path, commit: &str) -> OrchResult<()> {
    let commit = validate_full_commit_oid(root, commit)?;
    git(root, &["reset", "--hard", commit], true)?;
    Ok(())
}

fn validate_full_commit_oid<'a>(root: &Path, commit: &'a str) -> OrchResult<&'a str> {
    if !is_full_hex_object_id(commit) {
        return Err(invalid_goal_baseline_commit(commit));
    }

    let commit_object = format!("{commit}^{{commit}}");
    git(root, &["cat-file", "-e", &commit_object], true)
        .map_err(|err| invalid_goal_baseline_commit(commit).detail("stderr", err.details))?;
    Ok(commit)
}

fn is_full_hex_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn invalid_goal_baseline_commit(commit: &str) -> OrchError {
    OrchError::with_code(
        "invalid goal baseline commit",
        "invalid_goal_baseline_commit",
    )
    .detail("baseline_commit", commit.to_string())
}

pub(crate) fn clean_goal_candidates(root: &Path) -> OrchResult<()> {
    git(root, &["clean", "-fd", "-e", ".orchid/"], true)?;
    Ok(())
}

/// List repo-relative paths changed during an active or completed lease scope.
pub(crate) fn touched_for_lease(
    root: &Path,
    lease: &LeaseRecord,
) -> OrchResult<Map<String, Value>> {
    let status = git_status_data(root)?;
    let git_available = status.get("git").and_then(Value::as_bool).unwrap_or(false);
    let baseline: BTreeSet<String> = lease.baseline_changed().into_iter().collect();
    let completed_window: Option<BTreeSet<String>> = lease
        .completed_changed()
        .map(|paths| paths.into_iter().collect());
    let completed_snapshot_missing = lease.status().is_completed() && completed_window.is_none();
    let records = git_status_records_from_status(&status);
    let scope = lease.scope();
    let task_path = lease.task_path();

    let mut stage_paths = BTreeSet::new();
    let mut out_of_scope = BTreeSet::new();
    let mut ambiguous = BTreeSet::new();
    let mut missing_completion_snapshot = BTreeSet::new();
    let mut changed_records = Vec::new();
    let mut stage_records = Vec::new();
    let mut out_of_scope_records = Vec::new();
    let mut ambiguous_records = Vec::new();
    let mut missing_completion_snapshot_records = Vec::new();

    for record in records {
        let paths = record.visible_paths();
        if paths.is_empty() {
            continue;
        }

        if paths.iter().any(|path| baseline.contains(path)) {
            for path in paths {
                ambiguous.insert(path);
            }
            ambiguous_records.push(record);
            continue;
        }

        if completed_snapshot_missing {
            for path in paths {
                missing_completion_snapshot.insert(path);
            }
            missing_completion_snapshot_records.push(record);
            continue;
        }

        if let Some(window) = &completed_window {
            if !paths.iter().any(|path| window.contains(path)) {
                continue;
            }
        }

        changed_records.push(record.clone());
        let disallowed: Vec<String> = paths
            .iter()
            .filter(|path| !path_in_scope(path, &scope) && path.as_str() != task_path)
            .cloned()
            .collect();

        if disallowed.is_empty() {
            for path in paths {
                stage_paths.insert(path);
            }
            stage_records.push(record);
        } else {
            for path in disallowed {
                out_of_scope.insert(path);
            }
            out_of_scope_records.push(record);
        }
    }
    let mut preexisting_dirty: Vec<String> = baseline.into_iter().collect();
    preexisting_dirty.sort();

    let mut map = Map::new();
    map.insert("lease_id".to_string(), lease.id_value());
    map.insert("task".to_string(), lease.task_value());
    insert_records_if_non_empty(&mut map, "records", &changed_records);
    insert_array_if_non_empty(&mut map, "stage", stage_paths.into_iter().collect());
    insert_records_if_non_empty(&mut map, "stage_records", &stage_records);
    let mut blocked_by = Map::new();
    insert_array_if_non_empty(
        &mut blocked_by,
        "out_of_scope",
        out_of_scope.iter().cloned().collect(),
    );
    insert_array_if_non_empty(
        &mut blocked_by,
        "ambiguous",
        ambiguous.iter().cloned().collect(),
    );
    insert_array_if_non_empty(
        &mut blocked_by,
        "completion_snapshot_missing",
        missing_completion_snapshot.iter().cloned().collect(),
    );
    if !blocked_by.is_empty() {
        map.insert("blocked_by".to_string(), Value::Object(blocked_by));
    }
    let mut blocked_by_records = Map::new();
    insert_records_value_if_non_empty(
        &mut blocked_by_records,
        "out_of_scope",
        &out_of_scope_records,
    );
    insert_records_value_if_non_empty(&mut blocked_by_records, "ambiguous", &ambiguous_records);
    insert_records_value_if_non_empty(
        &mut blocked_by_records,
        "completion_snapshot_missing",
        &missing_completion_snapshot_records,
    );
    if !blocked_by_records.is_empty() {
        map.insert(
            "blocked_by_records".to_string(),
            Value::Object(blocked_by_records),
        );
    }
    insert_array_if_non_empty(&mut map, "preexisting_dirty", preexisting_dirty);
    if completed_snapshot_missing {
        map.insert("completion_snapshot_missing".to_string(), Value::Bool(true));
    }
    if !out_of_scope.is_empty() || !ambiguous.is_empty() || !missing_completion_snapshot.is_empty()
    {
        map.insert("safe_to_stage".to_string(), Value::Bool(false));
    }
    if !git_available {
        map.insert("git".to_string(), Value::Bool(false));
        map.insert("safe_to_stage".to_string(), Value::Bool(false));
    }
    Ok(map)
}

/// Build a [`StagePlan`] attributing in-scope edits inside the lease window.
pub(crate) fn stage_plan_for_lease(root: &Path, lease: &LeaseRecord) -> OrchResult<StagePlan> {
    let data = touched_for_lease(root, lease)?;
    let pathspecs: BTreeSet<String> = string_list(data.get("stage"))
        .into_iter()
        .map(|path| format!(":(literal){path}"))
        .collect();

    let mut excluded = Map::new();
    if let Some(blocked_by) = data.get("blocked_by").and_then(Value::as_object) {
        excluded.extend(blocked_by.clone());
    }
    if let Some(preexisting_dirty) = data.get("preexisting_dirty") {
        excluded.insert("preexisting_dirty".to_string(), preexisting_dirty.clone());
    }
    let mut excluded_records = Map::new();
    if let Some(blocked_by_records) = data.get("blocked_by_records").and_then(Value::as_object) {
        excluded_records.extend(blocked_by_records.clone());
    }

    Ok(StagePlan {
        lease_id: lease.id().unwrap_or("").to_string(),
        task: lease.get_str("task").unwrap_or("").to_string(),
        git_available: data.get("git").and_then(Value::as_bool).unwrap_or(true),
        safe_to_stage: data
            .get("safe_to_stage")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        pathspecs: pathspecs.into_iter().collect(),
        records: data
            .get("stage_records")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        excluded,
        excluded_records,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record_values(input: &[u8]) -> Vec<Value> {
        parse_porcelain_v2_z(input)
            .iter()
            .map(GitStatusRecord::to_value)
            .collect()
    }

    #[test]
    fn parses_porcelain_v2_ordinary_rename_and_untracked_records() {
        let data = b"1 .M N... 100644 100644 100644 aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb src/edit.rs\0\
2 R. N... 100644 100644 100644 cccccccccccccccccccccccccccccccccccccccc cccccccccccccccccccccccccccccccccccccccc R100 src/new.rs\0src/old.rs\0\
? src/untracked.rs\0";

        let records = record_values(data);

        assert_eq!(records.len(), 3);
        assert_eq!(records[0]["path"], "src/edit.rs");
        assert_eq!(records[0]["kind"], "modified");
        assert_eq!(records[0]["worktree"], "M");
        assert_eq!(records[0]["unstaged"], true);
        assert_eq!(records[1]["path"], "src/new.rs");
        assert_eq!(records[1]["orig_path"], "src/old.rs");
        assert_eq!(records[1]["kind"], "renamed");
        assert_eq!(
            records[1]["paths"],
            serde_json::json!(["src/old.rs", "src/new.rs"])
        );
        assert_eq!(records[2]["path"], "src/untracked.rs");
        assert_eq!(records[2]["kind"], "untracked");
        assert_eq!(records[2]["untracked"], true);
    }

    #[test]
    fn parser_tolerates_rtk_clean_and_merged_diagnostic_output() {
        assert!(parse_porcelain_v2_z(b"ok").is_empty());

        let data = b"[rtk] /!\\ No hook installed\n1 D. N... 100644 000000 000000 dddddddddddddddddddddddddddddddddddddddd dddddddddddddddddddddddddddddddddddddddd src/deleted.rs\0";
        let records = record_values(data);

        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["path"], "src/deleted.rs");
        assert_eq!(records[0]["kind"], "deleted");
        assert_eq!(records[0]["index"], "D");
        assert_eq!(records[0]["staged"], true);
    }
}
