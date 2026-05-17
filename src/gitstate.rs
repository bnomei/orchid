use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;

use serde_json::{Map, Value};

use crate::core::{string_list, OrchError, OrchResult};
use crate::model::{LeaseRecord, StagePlan};
use crate::specs::path_in_scope;

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

fn git_text(root: &Path, args: &[&str], check: bool) -> OrchResult<String> {
    Ok(String::from_utf8_lossy(&git(root, args, check)?)
        .trim()
        .to_string())
}

fn git_available(root: &Path) -> bool {
    git(root, &["rev-parse", "--show-toplevel"], true).is_ok()
}

fn split_z(data: &[u8]) -> Vec<String> {
    data.split(|byte| *byte == 0)
        .filter(|item| !item.is_empty())
        .map(|item| String::from_utf8_lossy(item).to_string())
        .collect()
}

fn git_name_set(root: &Path, args: &[&str]) -> OrchResult<Vec<String>> {
    let mut full_args = args.to_vec();
    full_args.push("-z");
    let mut names = split_z(&git(root, &full_args, true)?);
    names.sort();
    Ok(names)
}

pub(crate) fn git_status_data(root: &Path) -> OrchResult<Map<String, Value>> {
    if !git_available(root) {
        let mut map = Map::new();
        map.insert("git".to_string(), Value::Bool(false));
        return Ok(map);
    }

    let branch = git_text(root, &["rev-parse", "--abbrev-ref", "HEAD"], true)?;
    let head = git_text(root, &["rev-parse", "HEAD"], true)?;
    let unstaged: BTreeSet<String> = git_name_set(root, &["diff", "--name-only"])?
        .into_iter()
        .collect();
    let staged: BTreeSet<String> = git_name_set(root, &["diff", "--cached", "--name-only"])?
        .into_iter()
        .collect();
    let untracked: BTreeSet<String> =
        git_name_set(root, &["ls-files", "--others", "--exclude-standard"])?
            .into_iter()
            .collect();
    let all: BTreeSet<String> = unstaged
        .union(&staged)
        .cloned()
        .collect::<BTreeSet<_>>()
        .union(&untracked)
        .cloned()
        .collect();

    let modified = visible(unstaged);
    let staged = visible(staged);
    let untracked = visible(untracked);
    let all_changed = visible(all);
    let changed = changed_object(modified, staged, untracked);

    let mut map = Map::new();
    map.insert("git".to_string(), Value::Bool(true));
    map.insert("branch".to_string(), Value::String(branch));
    map.insert("head".to_string(), Value::String(head));
    map.insert("clean".to_string(), Value::Bool(all_changed.is_empty()));
    if changed.as_object().is_some_and(|items| !items.is_empty()) {
        map.insert("changed".to_string(), changed);
    }
    Ok(map)
}

fn visible(paths: BTreeSet<String>) -> Vec<String> {
    paths
        .into_iter()
        .filter(|path| !path.starts_with(".orchid/"))
        .collect()
}

fn changed_object(modified: Vec<String>, staged: Vec<String>, untracked: Vec<String>) -> Value {
    let mut changed = Map::new();
    insert_array_if_non_empty(&mut changed, "modified", modified);
    insert_array_if_non_empty(&mut changed, "staged", staged);
    insert_array_if_non_empty(&mut changed, "untracked", untracked);
    Value::Object(changed)
}

fn string_array(items: Vec<String>) -> Value {
    Value::Array(items.into_iter().map(Value::String).collect())
}

fn insert_array_if_non_empty(map: &mut Map<String, Value>, key: &str, items: Vec<String>) {
    if !items.is_empty() {
        map.insert(key.to_string(), string_array(items));
    }
}

pub(crate) fn changed_paths_value(status: &Map<String, Value>) -> Value {
    string_array(changed_paths(status))
}

fn changed_paths(status: &Map<String, Value>) -> Vec<String> {
    let mut paths = BTreeSet::new();
    if let Some(changed) = status.get("changed").and_then(Value::as_object) {
        for key in ["modified", "staged", "untracked"] {
            for path in string_list(changed.get(key)) {
                paths.insert(path);
            }
        }
    }
    paths.into_iter().collect()
}

pub(crate) fn touched_for_lease(
    root: &Path,
    lease: &LeaseRecord,
) -> OrchResult<Map<String, Value>> {
    let status = git_status_data(root)?;
    let baseline: BTreeSet<String> = lease.baseline_changed().into_iter().collect();
    let current: BTreeSet<String> = changed_paths(&status).into_iter().collect();
    let changed_since_lease: Vec<String> = current.difference(&baseline).cloned().collect();
    let ambiguous: Vec<String> = current.intersection(&baseline).cloned().collect();
    let scope = lease.scope();
    let task_path = lease.task_path();

    let mut in_scope = Vec::new();
    let mut out_of_scope = Vec::new();
    let mut control_plane = Vec::new();
    for path in changed_since_lease {
        if path_in_scope(&path, &scope) {
            in_scope.push(path);
        } else if path == task_path {
            control_plane.push(path);
        } else {
            out_of_scope.push(path);
        }
    }
    in_scope.sort();
    out_of_scope.sort();
    control_plane.sort();
    let mut preexisting_dirty: Vec<String> = baseline.into_iter().collect();
    preexisting_dirty.sort();
    let mut ambiguous = ambiguous;
    ambiguous.sort();
    let mut stage: Vec<String> = in_scope.into_iter().chain(control_plane).collect();
    stage.sort();
    stage.dedup();

    let mut map = Map::new();
    map.insert("lease_id".to_string(), lease.id_value());
    map.insert("task".to_string(), lease.task_value());
    insert_array_if_non_empty(&mut map, "stage", stage);
    let mut blocked_by = Map::new();
    insert_array_if_non_empty(&mut blocked_by, "out_of_scope", out_of_scope.clone());
    insert_array_if_non_empty(&mut blocked_by, "ambiguous", ambiguous.clone());
    if !blocked_by.is_empty() {
        map.insert("blocked_by".to_string(), Value::Object(blocked_by));
    }
    insert_array_if_non_empty(&mut map, "preexisting_dirty", preexisting_dirty);
    if !out_of_scope.is_empty() || !ambiguous.is_empty() {
        map.insert("safe_to_stage".to_string(), Value::Bool(false));
    }
    Ok(map)
}

pub(crate) fn stage_plan_for_lease(root: &Path, lease: &LeaseRecord) -> OrchResult<StagePlan> {
    let data = touched_for_lease(root, lease)?;
    let pathspecs: BTreeSet<String> = string_list(data.get("stage")).into_iter().collect();

    let mut excluded = Map::new();
    if let Some(blocked_by) = data.get("blocked_by").and_then(Value::as_object) {
        excluded.extend(blocked_by.clone());
    }
    if let Some(preexisting_dirty) = data.get("preexisting_dirty") {
        excluded.insert("preexisting_dirty".to_string(), preexisting_dirty.clone());
    }

    Ok(StagePlan {
        lease_id: lease.id().unwrap_or("").to_string(),
        task: lease.get_str("task").unwrap_or("").to_string(),
        safe_to_stage: data
            .get("safe_to_stage")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        pathspecs: pathspecs.into_iter().collect(),
        excluded,
    })
}
