//! Spec discovery, task indexing, dependency resolution, and ready-task selection.
//!
//! Loads `specs/<id>/` trees, filters inactive specs, and decides which tasks are
//! dispatchable given scope, fanout policy, and cross-spec dependencies.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::core::{ErrorCode, OrchError, OrchResult};
use crate::model::{
    scope_entry_escapes_root, scope_entry_is_blank, LeaseRecord, Scope, SpecId, SpecPolicy,
    VerificationMode,
};
use crate::paths::{read_text, relpath, repo_path};
use crate::taskfile::{load_task, Task};

pub(crate) const STATUSES: &[&str] = &[
    "blocked",
    "done",
    "pending_review",
    "pending_validation",
    "todo",
];

pub(crate) type ReadyTasksResult = (Vec<Task>, Vec<Map<String, Value>>, Vec<String>);

const INACTIVE_SPEC_PREFIXES: &[&str] = &["DRAFT-", "TBD-", "MANUAL-", "DONE-"];
const INACTIVE_SPEC_NAMES: &[&str] = &["DRAFT", "TBD", "MANUAL", "DONE"];

pub(crate) fn path_in_scope(path: &str, scopes: &[String]) -> bool {
    Scope::from_entries(scopes.to_vec()).contains_path(path)
}

pub(crate) fn scopes_overlap(a: &[String], b: &[String]) -> bool {
    Scope::from_entries(a.to_vec()).overlaps(&Scope::from_entries(b.to_vec()))
}

pub(crate) fn spec_is_inactive(name: &str) -> bool {
    name.starts_with('_')
        || INACTIVE_SPEC_NAMES.contains(&name)
        || INACTIVE_SPEC_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
}

pub(crate) fn safe_spec_id(value: &str) -> OrchResult<String> {
    SpecId::parse(value).map(SpecId::into_string)
}

fn numerical_spec_key(name: &str) -> (u8, u64, String) {
    let digits: String = name.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if digits.is_empty() {
        (1, 0, name.to_string())
    } else {
        (0, digits.parse().unwrap_or(0), name.to_string())
    }
}

fn numeric_spec_selector_matches(name: &str, selector: &str) -> bool {
    name.strip_prefix(selector)
        .is_some_and(|rest| rest.starts_with('-'))
}

fn active_spec_names(root: &Path, specs: &Path) -> OrchResult<Vec<String>> {
    if !specs.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = fs::read_dir(specs)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|path| repo_path(root, path, "spec_dir"))
        .collect::<OrchResult<Vec<_>>>()?
        .into_iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .filter(|name| !spec_is_inactive(name))
        .collect();
    names.sort_by_key(|name| numerical_spec_key(name));
    Ok(names)
}

fn resolve_spec_selector(root: &Path, specs: &Path, selector: &str) -> OrchResult<String> {
    let name = safe_spec_id(selector)?;
    if spec_is_inactive(&name) {
        return Err(
            OrchError::coded("inactive spec is not dispatchable", ErrorCode::InactiveSpec)
                .detail("spec", name),
        );
    }

    let is_numeric_selector = !name.is_empty() && name.chars().all(|ch| ch.is_ascii_digit());
    if !is_numeric_selector {
        let exact_dir = repo_path(root, specs.join(&name), "spec_dir")?;
        if exact_dir.is_dir() {
            return Ok(name);
        }
        return Err(
            OrchError::coded("spec not found", ErrorCode::SpecNotFound).detail("spec", name)
        );
    }

    let matches: Vec<String> = active_spec_names(root, specs)?
        .into_iter()
        .filter(|candidate| numeric_spec_selector_matches(candidate, &name))
        .collect();
    let exact_dir = repo_path(root, specs.join(&name), "spec_dir")?;
    if exact_dir.is_dir() {
        return Ok(name);
    }
    match matches.as_slice() {
        [resolved] => Ok(resolved.clone()),
        [] => Err(OrchError::coded("spec not found", ErrorCode::SpecNotFound).detail("spec", name)),
        _ => Err(
            OrchError::coded("spec selector ambiguous", ErrorCode::SpecSelectorAmbiguous)
                .detail("spec", name)
                .detail(
                    "matches",
                    Value::Array(matches.into_iter().map(Value::String).collect()),
                ),
        ),
    }
}

pub(crate) fn resolve_spec(root: &Path, selector: &str) -> OrchResult<String> {
    let specs = repo_path(root, root.join("specs"), "specs_dir")?;
    resolve_spec_selector(root, &specs, selector)
}

fn resolve_spec_selectors(root: &Path, spec_names: &[String]) -> OrchResult<Vec<String>> {
    let specs = repo_path(root, root.join("specs"), "specs_dir")?;
    let mut resolved = Vec::new();
    for spec_name in spec_names {
        resolved.push(resolve_spec_selector(root, &specs, spec_name)?);
    }
    Ok(resolved)
}

pub(crate) fn active_spec_dirs(
    root: &Path,
    spec_names: Option<&[String]>,
) -> OrchResult<Vec<PathBuf>> {
    let specs = repo_path(root, root.join("specs"), "specs_dir")?;
    if !specs.exists() {
        return Ok(Vec::new());
    }
    if let Some(spec_names) = spec_names {
        let mut dirs = Vec::new();
        for name in resolve_spec_selectors(root, spec_names)? {
            let spec_dir = repo_path(root, specs.join(&name), "spec_dir")?;
            dirs.push(spec_dir);
        }
        return Ok(dirs);
    }

    let mut dirs: Vec<PathBuf> = fs::read_dir(specs)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|path| repo_path(root, path, "spec_dir"))
        .collect::<OrchResult<Vec<_>>>()?
        .into_iter()
        .filter(|path| {
            path.file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|name| !spec_is_inactive(name))
        })
        .collect();
    dirs.sort_by_key(|path| {
        let name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        numerical_spec_key(name)
    });
    Ok(dirs)
}

pub(crate) fn task_paths(root: &Path, spec_names: Option<&[String]>) -> OrchResult<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for spec_dir in active_spec_dirs(root, spec_names)? {
        let task_dir = repo_path(root, spec_dir.join("tasks"), "task_dir")?;
        if task_dir.exists() {
            let mut task_files: Vec<PathBuf> = fs::read_dir(task_dir)?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().and_then(|s| s.to_str()) == Some("md"))
                .map(|path| repo_path(root, path, "task_path"))
                .collect::<OrchResult<Vec<_>>>()?
                .into_iter()
                .collect();
            task_files.sort();
            paths.extend(task_files);
        }
    }
    Ok(paths)
}

pub(crate) fn load_tasks(root: &Path, spec_names: Option<&[String]>) -> OrchResult<Vec<Task>> {
    let mut tasks = Vec::new();
    for path in task_paths(root, spec_names)? {
        tasks.push(load_task(path, root)?);
    }
    Ok(tasks)
}

fn first_open_spec(tasks: &[Task]) -> Option<String> {
    let mut by_spec: BTreeMap<String, Vec<&Task>> = BTreeMap::new();
    for task in tasks {
        by_spec.entry(task.spec_id.clone()).or_default().push(task);
    }
    let mut specs: Vec<String> = by_spec.keys().cloned().collect();
    specs.sort_by_key(|spec| numerical_spec_key(spec));
    specs.into_iter().find(|spec| {
        by_spec
            .get(spec)
            .is_some_and(|items| items.iter().any(|task| !task.status_model().is_done()))
    })
}

pub(crate) fn select_tasks(
    root: &Path,
    spec_names: Option<&[String]>,
    all_open: bool,
) -> OrchResult<(Vec<Task>, Vec<String>)> {
    if spec_names.is_some_and(|names| !names.is_empty()) && all_open {
        return Err(OrchError::coded(
            "use either --spec or --all-open, not both",
            ErrorCode::ScopeSelectorConflict,
        ));
    }
    if let Some(spec_names) = spec_names {
        if !spec_names.is_empty() {
            let selected = resolve_spec_selectors(root, spec_names)?;
            let tasks = load_tasks(root, Some(&selected))?;
            return Ok((tasks, selected));
        }
    }
    if all_open {
        let tasks = load_tasks(root, None)?;
        let Some(selected) = first_open_spec(&tasks) else {
            return Err(OrchError::coded(
                "no open spec found",
                ErrorCode::NoOpenSpec,
            ));
        };
        let selected_tasks = tasks
            .into_iter()
            .filter(|task| task.spec_id == selected)
            .collect();
        return Ok((selected_tasks, vec![selected]));
    }
    Err(OrchError::coded(
        "ready requires --spec or --all-open",
        ErrorCode::ScopeRequired,
    ))
}

pub(crate) fn ensure_spec_dispatchable(root: &Path, spec_id: &str) -> OrchResult<()> {
    if spec_is_inactive(spec_id) {
        return Err(
            OrchError::coded("inactive spec is not dispatchable", ErrorCode::InactiveSpec)
                .detail("spec", spec_id),
        );
    }
    let policy = load_spec_policy(root, spec_id)?;
    if policy.is_manual() {
        return Err(OrchError::coded("spec manual", ErrorCode::SpecManual).detail("spec", spec_id));
    }
    if policy.checkpoint_before_implementation() {
        return Err(OrchError::coded(
            "human checkpoint blocks dispatch",
            ErrorCode::HumanCheckpoint,
        )
        .detail("spec", spec_id)
        .detail("checkpoint", "before-implementation"));
    }
    Ok(())
}

pub(crate) fn inactive_spec_names(root: &Path) -> OrchResult<Vec<String>> {
    let specs = repo_path(root, root.join("specs"), "specs_dir")?;
    if !specs.exists() {
        return Ok(Vec::new());
    }
    let mut names: Vec<String> = fs::read_dir(specs)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .map(|path| repo_path(root, path, "spec_dir"))
        .collect::<OrchResult<Vec<_>>>()?
        .into_iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|s| s.to_str())
                .map(str::to_string)
        })
        .filter(|name| spec_is_inactive(name))
        .collect();
    names.sort();
    Ok(names)
}

pub(crate) fn load_spec_policy(root: &Path, spec_id: &str) -> OrchResult<SpecPolicy> {
    let path = repo_path(
        root,
        root.join("specs").join(spec_id).join("spec.toml"),
        "spec_policy_path",
    )?;
    if !path.exists() {
        return Ok(SpecPolicy::empty());
    }
    let raw = read_text(&path)?;
    let parsed: toml::Value = toml::from_str(&raw).map_err(|err| {
        OrchError::new("invalid spec.toml")
            .detail("path", relpath(&path, root))
            .detail("message", err.to_string())
    })?;
    Ok(SpecPolicy::from_map(toml_to_json_object(parsed)))
}

pub(crate) fn task_key(task: &Task) -> String {
    format!("{}/{}", task.spec_id, task.id())
}

fn safe_task_id(task_id: &str) -> OrchResult<&str> {
    let valid = !task_id.is_empty()
        && task_id != "."
        && task_id != ".."
        && !task_id.contains('/')
        && !task_id.contains('\\')
        && !task_id.contains("..");
    if !valid {
        return Err(
            OrchError::coded("invalid task id", ErrorCode::InvalidTaskId)
                .detail("task_id", task_id),
        );
    }
    Ok(task_id)
}

pub(crate) fn resolve_task(root: &Path, target: &str, task_id: Option<&str>) -> OrchResult<Task> {
    let target_path = Path::new(target);
    if task_id.is_none() && target_path.extension().and_then(|s| s.to_str()) == Some("md") {
        return load_task(target_path, root);
    }
    if task_id.is_none() && target.contains('/') {
        let mut parts = target.splitn(2, '/');
        let spec = parts.next().unwrap_or("").to_string();
        let task = safe_task_id(parts.next().unwrap_or(""))?;
        let resolved = resolve_spec_selectors(root, &[spec])?;
        return load_task(
            root.join("specs")
                .join(&resolved[0])
                .join("tasks")
                .join(format!("{task}.md")),
            root,
        );
    }
    let Some(task_id) = task_id else {
        return Err(OrchError::coded(
            "task id is required unless target is a task path or spec/task",
            ErrorCode::TaskIdRequired,
        ));
    };
    let task_id = safe_task_id(task_id)?;
    let resolved = resolve_spec_selectors(root, &[target.to_string()])?;
    load_task(
        root.join("specs")
            .join(&resolved[0])
            .join("tasks")
            .join(format!("{task_id}.md")),
        root,
    )
}

pub(crate) fn task_by_ref<'a>(
    tasks: &'a [Task],
    spec_id: &str,
    reference: &str,
) -> Option<&'a Task> {
    if reference.is_empty() || reference == "-" {
        return None;
    }
    let reference = reference.strip_prefix("specs/").unwrap_or(reference);
    let (spec, task_id) = if let Some((spec, task_id)) = reference.split_once('/') {
        (spec, task_id)
    } else {
        (spec_id, reference)
    };
    tasks
        .iter()
        .find(|task| task.spec_id == spec && task.id() == task_id)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DependencyBlock {
    Missing(String),
    Unmet(String),
}

impl DependencyBlock {
    pub(crate) fn reason(&self) -> String {
        match self {
            DependencyBlock::Missing(dep) => format!("missing dependency:{dep}"),
            DependencyBlock::Unmet(dep) => format!("unmet dependency:{dep}"),
        }
    }

    pub(crate) fn error_code(&self) -> ErrorCode {
        match self {
            DependencyBlock::Missing(_) => ErrorCode::MissingDependency,
            DependencyBlock::Unmet(_) => ErrorCode::UnmetDependency,
        }
    }

    pub(crate) fn reference(&self) -> &str {
        match self {
            DependencyBlock::Missing(dep) | DependencyBlock::Unmet(dep) => dep,
        }
    }
}

pub(crate) fn dependency_block(task: &Task, all_tasks: &[Task]) -> Option<DependencyBlock> {
    for dep in task.depends() {
        if dep == "-" {
            continue;
        }
        match task_by_ref(all_tasks, &task.spec_id, &dep) {
            None => return Some(DependencyBlock::Missing(dep)),
            Some(dep_task) if dep_task.status() != "done" => {
                return Some(DependencyBlock::Unmet(dep));
            }
            Some(_) => {}
        }
    }
    None
}

/// Return dispatchable tasks plus blocked entries and lint messages for `ready`.
pub(crate) fn ready_tasks(
    root: &Path,
    spec_names: Option<&[String]>,
    all_open: bool,
    active: Option<&[LeaseRecord]>,
) -> OrchResult<ReadyTasksResult> {
    let (tasks, selected_specs) = select_tasks(root, spec_names, all_open)?;
    let all_tasks = load_tasks(root, None)?;
    let active = active.unwrap_or(&[]);
    let mut ready = Vec::new();
    let mut blocked = Vec::new();

    for task in &tasks {
        let policy = load_spec_policy(root, &task.spec_id)?;
        let mut reason: Option<String> = None;
        if policy.is_manual() {
            reason = Some("spec manual".to_string());
        } else if policy.checkpoint_before_implementation() {
            reason = Some("human checkpoint:before-implementation".to_string());
        } else if !task.status_model().is_todo() {
            reason = Some(format!("status:{}", task.status()));
        } else if !VerificationMode::parse(task.verification_mode()).is_dispatchable() {
            reason = Some("invalid verification_mode".to_string());
        } else if let Some(bad_scope) = task
            .scope()
            .iter()
            .find(|entry| scope_entry_is_blank(entry))
        {
            reason = Some(format!("blank scope entry:{bad_scope}"));
        } else if task
            .scope()
            .iter()
            .any(|entry| scope_entry_escapes_root(entry))
        {
            reason = Some("scope escapes repo root".to_string());
        } else if !task.worker_reasoning_effort_model().is_valid() {
            reason = Some("invalid worker_reasoning_effort".to_string());
        } else if !task.worker_model_is_valid() {
            reason = Some("invalid worker_model".to_string());
        } else if let Some(block) = dependency_block(task, &all_tasks) {
            reason = Some(block.reason());
        }

        if reason.is_none() {
            for lease in active {
                if lease.task_path() == relpath(&task.path, root) {
                    reason = Some("already leased".to_string());
                    break;
                }
                if scopes_overlap(&task.scope(), &lease.scope()) {
                    let lease_id = lease.id().unwrap_or("");
                    reason = Some(format!("scope conflict:{lease_id}"));
                    break;
                }
            }
        }

        if let Some(reason) = reason {
            let mut item = Map::new();
            item.insert("task".to_string(), Value::String(task_key(task)));
            item.insert("reason".to_string(), Value::String(reason));
            blocked.push(item);
        } else {
            ready.push(task.clone());
        }
    }
    Ok((ready, blocked, selected_specs))
}

pub(crate) fn selected_task_counts(tasks: &[Task]) -> Map<String, Value> {
    let mut counts: BTreeMap<String, i64> = STATUSES
        .iter()
        .map(|status| (status.to_string(), 0))
        .collect();
    for task in tasks {
        *counts.entry(task.status()).or_default() += 1;
    }
    counts
        .into_iter()
        .map(|(key, value)| (key, Value::Number(value.into())))
        .collect()
}

pub(crate) fn status_set() -> BTreeSet<&'static str> {
    STATUSES.iter().copied().collect()
}

fn toml_to_json_object(value: toml::Value) -> Map<String, Value> {
    match toml_to_json(value) {
        Value::Object(map) => map,
        _ => Map::new(),
    }
}

fn toml_to_json(value: toml::Value) -> Value {
    match value {
        toml::Value::String(raw) => Value::String(raw),
        toml::Value::Integer(raw) => Value::Number(raw.into()),
        toml::Value::Float(raw) => serde_json::Number::from_f64(raw)
            .map(Value::Number)
            .unwrap_or(Value::Null),
        toml::Value::Boolean(raw) => Value::Bool(raw),
        toml::Value::Datetime(raw) => Value::String(raw.to_string()),
        toml::Value::Array(items) => Value::Array(items.into_iter().map(toml_to_json).collect()),
        toml::Value::Table(table) => {
            let mut map = Map::new();
            for (key, value) in table {
                map.insert(key, toml_to_json(value));
            }
            Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_spec_id_is_rejected_before_filesystem_use() {
        let err = safe_spec_id("specs/../outside").unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidSpecId.as_str());
        assert_eq!(err.details["spec"], "specs/../outside");
    }

    #[test]
    fn scope_helpers_match_directory_boundaries() {
        assert!(path_in_scope(
            "src/feature/file.rs",
            &["src/feature".to_string()]
        ));
        assert!(!path_in_scope(
            "src/feature_extra/file.rs",
            &["src/feature".to_string()]
        ));
        assert!(path_in_scope("src/main.rs", &[".".to_string()]));
        assert!(path_in_scope("src/main.rs", &["/".to_string()]));
        assert!(scopes_overlap(
            &["src/feature".to_string()],
            &["src/feature/file.rs".to_string()]
        ));
        assert!(scopes_overlap(
            &[".".to_string()],
            &["src/feature/file.rs".to_string()]
        ));
        assert!(!scopes_overlap(
            &["src/feature".to_string()],
            &["src/feature-extra".to_string()]
        ));
    }
}
