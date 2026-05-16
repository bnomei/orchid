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
        map.insert("branch".to_string(), Value::String(String::new()));
        map.insert("head".to_string(), Value::String(String::new()));
        map.insert("clean".to_string(), Value::Bool(true));
        map.insert(
            "changed".to_string(),
            changed_object(Vec::new(), Vec::new(), Vec::new()),
        );
        map.insert("all_changed".to_string(), Value::Array(Vec::new()));
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

    let mut map = Map::new();
    map.insert("git".to_string(), Value::Bool(true));
    map.insert("branch".to_string(), Value::String(branch));
    map.insert("head".to_string(), Value::String(head));
    map.insert("clean".to_string(), Value::Bool(all_changed.is_empty()));
    map.insert(
        "changed".to_string(),
        changed_object(modified, staged, untracked),
    );
    map.insert("all_changed".to_string(), string_array(all_changed));
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
    changed.insert("modified".to_string(), string_array(modified));
    changed.insert("staged".to_string(), string_array(staged));
    changed.insert("untracked".to_string(), string_array(untracked));
    Value::Object(changed)
}

fn string_array(items: Vec<String>) -> Value {
    Value::Array(items.into_iter().map(Value::String).collect())
}

pub(crate) fn touched_for_lease(
    root: &Path,
    lease: &LeaseRecord,
) -> OrchResult<Map<String, Value>> {
    let status = git_status_data(root)?;
    let baseline: BTreeSet<String> = lease.baseline_changed().into_iter().collect();
    let current: BTreeSet<String> = string_list(status.get("all_changed")).into_iter().collect();
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

    let mut map = Map::new();
    map.insert("lease_id".to_string(), lease.id_value());
    map.insert("task".to_string(), lease.task_value());
    map.insert("scope".to_string(), string_array(scope));
    map.insert("touched_in_scope".to_string(), string_array(in_scope));
    map.insert("control_plane".to_string(), string_array(control_plane));
    map.insert(
        "out_of_scope".to_string(),
        string_array(out_of_scope.clone()),
    );
    map.insert(
        "preexisting_dirty".to_string(),
        string_array(preexisting_dirty),
    );
    map.insert("ambiguous".to_string(), string_array(ambiguous.clone()));
    map.insert(
        "safe_to_stage".to_string(),
        Value::Bool(out_of_scope.is_empty() && ambiguous.is_empty()),
    );
    map.insert("git".to_string(), Value::Object(status));
    Ok(map)
}

pub(crate) fn stage_plan_for_lease(root: &Path, lease: &LeaseRecord) -> OrchResult<StagePlan> {
    let data = touched_for_lease(root, lease)?;
    let mut pathspecs: BTreeSet<String> = string_list(data.get("touched_in_scope"))
        .into_iter()
        .collect();
    pathspecs.extend(string_list(data.get("control_plane")));

    let mut excluded = Map::new();
    excluded.insert(
        "out_of_scope".to_string(),
        data.get("out_of_scope")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    );
    excluded.insert(
        "preexisting_dirty".to_string(),
        data.get("preexisting_dirty")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    );
    excluded.insert(
        "ambiguous".to_string(),
        data.get("ambiguous")
            .cloned()
            .unwrap_or_else(|| Value::Array(Vec::new())),
    );

    Ok(StagePlan {
        lease_id: lease.id().unwrap_or("").to_string(),
        task: lease.get_str("task").unwrap_or("").to_string(),
        safe_to_stage: data
            .get("safe_to_stage")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        pathspecs: pathspecs.into_iter().collect(),
        excluded,
    })
}
