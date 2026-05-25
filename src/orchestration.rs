use std::fs;
use std::path::Path;

use chrono::{TimeZone, Utc};
use serde_json::{Map, Value};

use crate::core::{
    insert, json_ok, now_iso, parse_duration, parse_iso_datetime_str, value_to_string, ErrorCode,
    OrchError, OrchResult,
};
use crate::gitstate::{
    changed_paths_value, git_status_data, stage_plan_for_lease, touched_for_lease,
};
use crate::model::{ActiveLeaseRecordInput, LeaseId, LeaseMode, LeaseRecord, ReportFrontmatter};
use crate::paths::{
    atomic_write, buds_dir, ensure_runtime_dirs, leases_dir, packets_dir, relpath, repo_path,
    reports_dir,
};
use crate::planner::{
    decide_next, BlockedTask, CleanupCandidate, NextInput, ReadyTask, ReportReady,
};
use crate::runtime::{
    active_leases, all_leases, clean_spec_research, close_lease_files, compact_lease,
    completed_runtime_leases, lease_id_for, lease_stale, load_lease, prune_empty_runtime_dirs,
    report_path_for_lease, runtime_lock, save_lease, spec_research_dir,
};
use crate::specs::{
    ensure_spec_dispatchable, inactive_spec_names, load_spec_policy, load_tasks, ready_tasks,
    resolve_task, scopes_overlap, select_tasks, selected_task_counts, status_set, task_by_ref,
    task_key,
};
use crate::taskfile::{
    load_task, quote_toml_string, read_optional, split_frontmatter, write_task_frontmatter,
};

pub(crate) struct LeaseRequest {
    pub(crate) target: String,
    pub(crate) task_id: Option<String>,
    pub(crate) owner: String,
    pub(crate) agent_id: Option<String>,
    pub(crate) lease_id: Option<String>,
    pub(crate) serial: bool,
    pub(crate) allow_parallel: bool,
}

pub(crate) struct BudRequest {
    pub(crate) title: String,
    pub(crate) scope: Vec<String>,
    pub(crate) instructions: String,
    pub(crate) owner: Option<String>,
    pub(crate) agent_id: Option<String>,
    pub(crate) lease_id: Option<String>,
    pub(crate) serial: bool,
    pub(crate) allow_parallel: bool,
}

pub(crate) struct AttachAgentRequest {
    pub(crate) lease: String,
    pub(crate) agent_id: String,
}

pub(crate) struct NextRequest {
    pub(crate) specs: Vec<String>,
    pub(crate) all_open: bool,
    pub(crate) older_than: String,
    pub(crate) explain: bool,
}

pub(crate) struct ReadyRequest {
    pub(crate) specs: Vec<String>,
    pub(crate) all_open: bool,
    pub(crate) explain: bool,
}

pub(crate) struct StatusRequest {
    pub(crate) specs: Vec<String>,
    pub(crate) agent_id: Option<String>,
    pub(crate) all_open: bool,
}

pub(crate) struct ResearchPathRequest {
    pub(crate) spec: String,
    pub(crate) create: bool,
}

pub(crate) struct CompleteRequest {
    pub(crate) lease: String,
    pub(crate) verified_by: String,
    pub(crate) implemented_by: String,
    pub(crate) verification_status: String,
    pub(crate) report: String,
    pub(crate) commit: String,
    pub(crate) commit_review: String,
    pub(crate) clean_spec_research: bool,
}

pub(crate) struct BlockRequest {
    pub(crate) target: String,
    pub(crate) task_id: Option<String>,
    pub(crate) reason: String,
}

pub(crate) struct CloseRequest {
    pub(crate) lease: String,
    pub(crate) force: bool,
}

pub(crate) struct CleanupRequest {
    pub(crate) completed: bool,
}

#[derive(Copy, Clone, Eq, PartialEq)]
pub(crate) enum PacketRoleKind {
    Worker,
    Validator,
    Reviewer,
    LoopRunner,
}

impl PacketRoleKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Worker => "worker",
            Self::Validator => "validator",
            Self::Reviewer => "reviewer",
            Self::LoopRunner => "loop-runner",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Worker => "Worker",
            Self::Validator => "Validator",
            Self::Reviewer => "Reviewer",
            Self::LoopRunner => "Loop-Runner",
        }
    }

    fn note(self) -> &'static str {
        match self {
            Self::Worker => {
                "Implement the leased task inside scope, run focused validation, and write the report."
            }
            Self::Validator => {
                "Verify the worker claim and evidence. Do not implement fixes unless explicitly asked."
            }
            Self::Reviewer => {
                "Review the committed or staged task changes and report actionable findings."
            }
            Self::LoopRunner => "Run exactly the leased loop cycle and return bounded evidence.",
        }
    }
}

pub(crate) struct PacketRequest {
    pub(crate) lease: String,
    pub(crate) role: PacketRoleKind,
}

pub(crate) struct ReportCheckRequest {
    pub(crate) report: String,
}

pub(crate) fn ready(root: &Path, request: &ReadyRequest) -> OrchResult<Map<String, Value>> {
    let active = active_leases(root)?;
    let (ready, blocked, _) = ready_tasks(
        root,
        specs_arg(&request.specs),
        request.all_open,
        Some(&active),
    )?;
    let mut payload = json_ok();
    let skipped = if request.all_open {
        inactive_spec_names(root)?
    } else {
        Vec::new()
    };
    insert_non_empty(
        &mut payload,
        "skipped_inactive_specs",
        string_values(skipped),
    );
    insert(
        &mut payload,
        "ready",
        Value::Array(
            ready
                .iter()
                .map(|task| {
                    let mut item = Map::new();
                    insert(&mut item, "task", task_key(task));
                    insert(&mut item, "scope", string_values(task.scope()));
                    insert(&mut item, "verify", task.verification_mode());
                    Value::Object(item)
                })
                .collect(),
        ),
    );
    if request.explain {
        insert_non_empty(&mut payload, "blocked", objects_array(blocked));
    }
    Ok(payload)
}

pub(crate) fn status(root: &Path, request: &StatusRequest) -> OrchResult<Map<String, Value>> {
    if let Some(agent_id) = request
        .agent_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        if !request.specs.is_empty() || request.all_open {
            return Err(OrchError::coded(
                "agent status cannot combine with spec selectors",
                ErrorCode::ScopeSelectorConflict,
            )
            .detail("agent_id", agent_id));
        }
        return status_for_agent(root, agent_id);
    }

    let (tasks, _) = if specs_arg(&request.specs).is_some() || request.all_open {
        select_tasks(root, specs_arg(&request.specs), request.all_open)?
    } else {
        (load_tasks(root, None)?, Vec::new())
    };
    let counts = nonzero_counts(selected_task_counts(&tasks));
    let active = active_leases(root)?.len();
    let mut payload = json_ok();
    insert(&mut payload, "tasks", tasks.len() as i64);
    insert(&mut payload, "counts", Value::Object(counts));
    if active != 0 {
        insert(&mut payload, "active", active as i64);
    }
    Ok(payload)
}

pub(crate) fn lease(root: &Path, request: &LeaseRequest) -> OrchResult<Map<String, Value>> {
    let _lock = runtime_lock(root)?;
    ensure_runtime_dirs(root)?;
    let task = resolve_task(root, &request.target, request.task_id.as_deref())?;
    ensure_spec_dispatchable(root, &task.spec_id)?;
    if !task.status_model().is_todo() {
        return Err(OrchError::coded("task is not todo", ErrorCode::TaskNotTodo)
            .detail("task", task_key(&task))
            .detail("status", task.status()));
    }
    let active = active_leases(root)?;
    for lease in &active {
        if lease.task_path() == relpath(&task.path, root) {
            return Err(
                OrchError::coded("task already leased", ErrorCode::TaskAlreadyLeased)
                    .detail("lease_id", lease.id_value()),
            );
        }
        if scopes_overlap(&task.scope(), &lease.scope()) {
            return Err(OrchError::coded("scope conflict", ErrorCode::ScopeConflict)
                .detail("lease_id", lease.id_value())
                .detail("scope", string_values(lease.scope())));
        }
    }
    if request.serial && !active.is_empty() {
        return Err(OrchError::coded(
            "serial lease blocked by active leases",
            ErrorCode::SerialBlocked,
        )
        .detail("active_leases", compact_leases(active)?));
    }
    if !active.is_empty() && !request.allow_parallel {
        return Err(OrchError::coded(
            "active leases require --allow-parallel",
            ErrorCode::ParallelNotConfirmed,
        )
        .detail("active_leases", compact_leases(active)?));
    }

    let lease_id = request
        .lease_id
        .clone()
        .map(LeaseId::from_raw)
        .unwrap_or_else(|| lease_id_for(&task.path, &request.owner));
    ensure_lease_id_available(root, lease_id.as_str())?;
    ensure_agent_id_available(root, request.agent_id.as_deref(), None)?;
    let git_state = git_status_data(root)?;
    let lease_mode = if request.allow_parallel {
        LeaseMode::Parallel
    } else if request.serial {
        LeaseMode::Serial
    } else {
        LeaseMode::Single
    };
    let started_at = now_iso();
    let lease = LeaseRecord::new_active(ActiveLeaseRecordInput {
        lease_id: lease_id.clone(),
        lease_mode,
        owner: request.owner.clone(),
        agent_id: request.agent_id.clone(),
        task: task_key(&task),
        task_path: relpath(&task.path, root),
        scope: task.scope(),
        started_at,
        base_head: git_state
            .get("head")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        baseline_changed: changed_paths_value(&git_state),
        report_path: relpath(
            &reports_dir(root).join(format!("{}.md", lease_id.as_str())),
            root,
        ),
    });
    save_lease(root, &lease)?;

    let mut payload = json_ok();
    insert(&mut payload, "lease_id", lease_id.into_string());
    insert(&mut payload, "lease_mode", lease_mode.as_str());
    insert(&mut payload, "task", task_key(&task));
    insert(&mut payload, "task_path", relpath(&task.path, root));
    insert(
        &mut payload,
        "report",
        lease.get("report_path").cloned().unwrap_or(Value::Null),
    );
    if let Some(agent_id) = request
        .agent_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        insert(&mut payload, "agent_id", agent_id);
    }
    Ok(payload)
}

pub(crate) fn bud(root: &Path, request: &BudRequest) -> OrchResult<Map<String, Value>> {
    let _lock = runtime_lock(root)?;
    let scope: Vec<String> = request
        .scope
        .iter()
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
        .collect();
    if scope.is_empty() {
        return Err(OrchError::coded(
            "bud requires --scope",
            ErrorCode::ScopeRequired,
        ));
    }
    let instructions = fs::read_to_string(&request.instructions)?;
    ensure_runtime_dirs(root)?;

    let active = active_leases(root)?;
    for lease in &active {
        if scopes_overlap(&scope, &lease.scope()) {
            return Err(OrchError::coded("scope conflict", ErrorCode::ScopeConflict)
                .detail("lease_id", lease.id_value())
                .detail("scope", string_values(lease.scope())));
        }
    }
    if request.serial && !active.is_empty() {
        return Err(OrchError::coded(
            "serial lease blocked by active leases",
            ErrorCode::SerialBlocked,
        )
        .detail("active_leases", compact_leases(active)?));
    }
    if !active.is_empty() && !request.allow_parallel {
        return Err(OrchError::coded(
            "active leases require --allow-parallel",
            ErrorCode::ParallelNotConfirmed,
        )
        .detail("active_leases", compact_leases(active)?));
    }

    let owner = request
        .owner
        .as_deref()
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            request
                .agent_id
                .as_deref()
                .filter(|value| !value.is_empty())
                .map(|agent_id| format!("worker:{agent_id}"))
                .unwrap_or_else(|| "worker:unassigned".to_string())
        });
    ensure_agent_id_available(root, request.agent_id.as_deref(), None)?;
    let lease_id = if let Some(lease_id) = request.lease_id.clone().map(LeaseId::from_raw) {
        ensure_lease_id_available(root, lease_id.as_str())?;
        lease_id
    } else {
        unique_bud_lease_id(root, &request.title, &owner)?
    };
    let lease_id_text = lease_id.as_str().to_string();
    let git_state = git_status_data(root)?;
    let lease_mode = if request.allow_parallel {
        LeaseMode::Parallel
    } else if request.serial {
        LeaseMode::Serial
    } else {
        LeaseMode::Single
    };
    let started_at = now_iso();
    let instructions_path = buds_dir(root).join(format!("{lease_id_text}.md"));
    atomic_write(&instructions_path, &instructions)?;
    let mut lease = LeaseRecord::new_active(ActiveLeaseRecordInput {
        lease_id,
        lease_mode,
        owner: owner.clone(),
        agent_id: request.agent_id.clone(),
        task: format!("bud:{lease_id_text}"),
        task_path: String::new(),
        scope: scope.clone(),
        started_at,
        base_head: git_state
            .get("head")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        baseline_changed: changed_paths_value(&git_state),
        report_path: relpath(&reports_dir(root).join(format!("{lease_id_text}.md")), root),
    });
    lease.set("kind", "bud");
    lease.set("title", request.title.clone());
    lease.set("instructions_path", relpath(&instructions_path, root));
    let packet = render_packet_for_lease(root, &mut lease, &lease_id_text, PacketRoleKind::Worker)?;
    save_lease(root, &lease)?;

    let mut payload = json_ok();
    insert(&mut payload, "lease_id", lease_id_text);
    insert(&mut payload, "kind", "bud");
    insert(&mut payload, "lease_mode", lease_mode.as_str());
    insert(&mut payload, "task", lease.task_value());
    insert(&mut payload, "title", request.title.clone());
    insert(&mut payload, "owner", owner);
    if let Some(agent_id) = request
        .agent_id
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        insert(&mut payload, "agent_id", agent_id);
    }
    insert(&mut payload, "scope", string_values(scope));
    insert(&mut payload, "packet", packet);
    insert(
        &mut payload,
        "report",
        lease.get("report_path").cloned().unwrap_or(Value::Null),
    );
    insert(&mut payload, "status", "active");
    Ok(payload)
}

pub(crate) fn lease_attach_agent(
    root: &Path,
    request: &AttachAgentRequest,
) -> OrchResult<Map<String, Value>> {
    let _lock = runtime_lock(root)?;
    let mut lease = load_lease(root, &request.lease)?;
    ensure_agent_id_available(root, Some(&request.agent_id), Some(&request.lease))?;
    lease.set("agent_id", request.agent_id.clone());
    if lease.get_str("owner") == Some("worker:unassigned") {
        lease.set("owner", format!("worker:{}", request.agent_id));
    }
    save_lease(root, &lease)?;
    let mut payload = json_ok();
    insert(&mut payload, "lease_id", request.lease.clone());
    insert(&mut payload, "agent_id", request.agent_id.clone());
    insert(&mut payload, "kind", lease.kind().as_str());
    insert(&mut payload, "status", lease.status().as_str());
    Ok(payload)
}

pub(crate) fn next(root: &Path, request: &NextRequest) -> OrchResult<Map<String, Value>> {
    let stale_after = parse_duration(&request.older_than)?;
    let now = Utc::now();
    let (tasks, _) = select_tasks(root, specs_arg(&request.specs), request.all_open)?;
    let active = active_leases(root)?;
    let (ready, blocked, _) = ready_tasks(
        root,
        specs_arg(&request.specs),
        request.all_open,
        Some(&active),
    )?;
    let stale = active
        .iter()
        .filter(|lease| lease_stale(lease, now, stale_after))
        .map(|lease| compact_lease(lease, Some(now), Some(stale_after)))
        .collect::<OrchResult<Vec<_>>>()?;
    let mut reports_ready = Vec::new();
    for lease in &active {
        let report = report_path_for_lease(root, lease)?;
        if report.exists() {
            reports_ready.push(ReportReady {
                lease_id: lease.id().unwrap_or("").to_string(),
                task: lease.get_str("task").unwrap_or("").to_string(),
                report: relpath(&report, root),
            });
        }
    }
    let completed = completed_runtime_leases(root)?;
    let stage = completed
        .iter()
        .map(|lease| stage_plan_for_lease(root, lease))
        .collect::<OrchResult<Vec<_>>>()?;
    let cleanup = completed
        .iter()
        .map(|lease| CleanupCandidate {
            lease_id: lease.id().unwrap_or("").to_string(),
            task: lease.get_str("task").unwrap_or("").to_string(),
        })
        .collect();
    let ready_payload = ready
        .iter()
        .map(|task| ReadyTask {
            id: task.id(),
            spec: task.spec_id.clone(),
            task: task_key(task),
            scope: task.scope(),
            verify: task.verification_mode().to_string(),
        })
        .collect();
    let blocked = blocked
        .into_iter()
        .map(|item| BlockedTask {
            task: item
                .get("task")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            reason: item
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        })
        .collect();
    Ok(decide_next(NextInput {
        stale,
        reports_ready,
        active: active
            .iter()
            .map(|lease| compact_lease(lease, Some(now), Some(stale_after)))
            .collect::<OrchResult<Vec<_>>>()?,
        stage,
        cleanup,
        ready: ready_payload,
        blocked,
        counts: selected_task_counts(&tasks),
        older_than: request.older_than.clone(),
        explain: request.explain,
    })
    .to_payload())
}

pub(crate) fn complete(root: &Path, request: &CompleteRequest) -> OrchResult<Map<String, Value>> {
    let _lock = runtime_lock(root)?;
    let mut lease = load_lease(root, &request.lease)?;
    if lease.is_bud() {
        let completed_at = now_iso();
        let implemented_by = if request.implemented_by.is_empty() {
            lease.get_str("owner").unwrap_or("").to_string()
        } else {
            request.implemented_by.clone()
        };
        lease.set("status", "completed");
        lease.set("completed_at", completed_at);
        lease.set("implemented_by", implemented_by);
        lease.set("verified_by", request.verified_by.clone());
        lease.set("verification_status", request.verification_status.clone());
        if !request.report.is_empty() {
            lease.set("report", request.report.clone());
        }
        if !request.commit.is_empty() {
            lease.set("commit", request.commit.clone());
        }
        if !request.commit_review.is_empty() {
            lease.set("commit_review", request.commit_review.clone());
        }
        save_lease(root, &lease)?;
        let mut payload = json_ok();
        insert(&mut payload, "lease_id", request.lease.clone());
        insert(&mut payload, "kind", "bud");
        insert(&mut payload, "task", lease.task_value());
        return Ok(payload);
    }

    let task_path = lease.task_path();
    let task = load_task(repo_path(root, task_path, "task_path")?, root)?;
    let mut frontmatter = task.frontmatter().clone();
    let meta = frontmatter.raw_mut();
    insert(meta, "status", "done");
    insert(
        meta,
        "verification_status",
        request.verification_status.clone(),
    );
    insert(meta, "completed_at", now_iso());
    let implemented_by = if request.implemented_by.is_empty() {
        lease.get_str("owner").unwrap_or("").to_string()
    } else {
        request.implemented_by.clone()
    };
    insert(meta, "implemented_by", implemented_by);
    insert(meta, "verified_by", request.verified_by.clone());
    insert(meta, "last_lease_id", request.lease.clone());
    if request.report.is_empty() {
        meta.remove("report");
    } else {
        insert(meta, "report", request.report.clone());
    }
    if !request.commit.is_empty() {
        insert(meta, "commit", request.commit.clone());
    }
    if !request.commit_review.is_empty() {
        insert(meta, "commit_review", request.commit_review.clone());
    }
    let completed_at = meta.get("completed_at").cloned().unwrap_or(Value::Null);
    write_task_frontmatter(&task, frontmatter)?;
    lease.set("status", "completed");
    lease.set("completed_at", completed_at);
    save_lease(root, &lease)?;
    let mut payload = json_ok();
    insert(&mut payload, "lease_id", request.lease.clone());
    insert(&mut payload, "task", task_key(&task));
    if request.clean_spec_research {
        let (deleted, pruned) = clean_spec_research(root, &task.spec_id)?;
        insert_non_empty(
            &mut payload,
            "spec_research_deleted",
            string_values(deleted),
        );
        insert_non_empty(&mut payload, "pruned", string_values(pruned));
    }
    Ok(payload)
}

pub(crate) fn block(root: &Path, request: &BlockRequest) -> OrchResult<Map<String, Value>> {
    let _lock = runtime_lock(root)?;
    let task = resolve_task(root, &request.target, request.task_id.as_deref())?;
    let mut frontmatter = task.frontmatter().clone();
    let meta = frontmatter.raw_mut();
    insert(meta, "status", "blocked");
    insert(meta, "blocked_at", now_iso());
    insert(meta, "blocked_reason", request.reason.clone());
    write_task_frontmatter(&task, frontmatter)?;
    let mut payload = json_ok();
    insert(&mut payload, "task", task_key(&task));
    Ok(payload)
}

pub(crate) fn heartbeat(root: &Path, lease_id: &str) -> OrchResult<Map<String, Value>> {
    let _lock = runtime_lock(root)?;
    let mut lease = load_lease(root, lease_id)?;
    let heartbeat_at = now_iso();
    lease.set("heartbeat_at", heartbeat_at.clone());
    save_lease(root, &lease)?;
    let mut payload = json_ok();
    insert(&mut payload, "lease_id", lease_id);
    insert(&mut payload, "heartbeat_at", heartbeat_at);
    Ok(payload)
}

pub(crate) fn running(root: &Path) -> OrchResult<Map<String, Value>> {
    let mut payload = json_ok();
    insert(
        &mut payload,
        "leases",
        compact_leases(active_leases(root)?)?,
    );
    Ok(payload)
}

pub(crate) fn stale(root: &Path, older_than: &str) -> OrchResult<Map<String, Value>> {
    let cutoff = Utc::now() - parse_duration(older_than)?;
    let epoch = Utc.timestamp_opt(0, 0).single().unwrap();
    let mut stale = Vec::new();
    for lease in active_leases(root)? {
        let raw = lease
            .heartbeat_or_started()
            .and_then(value_to_string)
            .unwrap_or_default();
        let heartbeat = parse_iso_datetime_str(&raw).unwrap_or(epoch);
        if heartbeat < cutoff {
            let mut item = Map::new();
            item.insert("id".to_string(), lease.id_value());
            item.insert("task".to_string(), lease.task_value());
            insert(
                &mut item,
                "age",
                (Utc::now() - heartbeat).num_seconds().max(0),
            );
            stale.push(item);
        }
    }
    let mut payload = json_ok();
    insert(&mut payload, "stale", objects_array(stale));
    Ok(payload)
}

pub(crate) fn release(root: &Path, lease_id: &str, reason: &str) -> OrchResult<Map<String, Value>> {
    let _lock = runtime_lock(root)?;
    let mut lease = load_lease(root, lease_id)?;
    lease.set("status", "released");
    lease.set("released_at", now_iso());
    if !reason.is_empty() {
        lease.set("release_reason", reason);
    }
    save_lease(root, &lease)?;
    let mut payload = json_ok();
    insert(&mut payload, "lease_id", lease_id);
    Ok(payload)
}

pub(crate) fn research_path(
    root: &Path,
    request: &ResearchPathRequest,
) -> OrchResult<Map<String, Value>> {
    let spec_id = crate::specs::safe_spec_id(&request.spec)?;
    let path = spec_research_dir(root, &spec_id)?;
    let mut created = false;
    if request.create {
        fs::create_dir_all(&path)?;
        created = true;
    }
    let mut payload = json_ok();
    insert(&mut payload, "spec", spec_id);
    insert(&mut payload, "path", relpath(&path, root));
    if path.exists() {
        insert(&mut payload, "exists", true);
    }
    if created {
        insert(&mut payload, "created", true);
    }
    Ok(payload)
}

pub(crate) fn research_clean(root: &Path, spec: &str) -> OrchResult<Map<String, Value>> {
    let spec_id = crate::specs::safe_spec_id(spec)?;
    let _lock = runtime_lock(root)?;
    let (deleted, pruned) = clean_spec_research(root, &spec_id)?;
    let mut payload = json_ok();
    insert(&mut payload, "spec", spec_id);
    insert_non_empty(&mut payload, "deleted", string_values(deleted));
    insert_non_empty(&mut payload, "pruned", string_values(pruned));
    Ok(payload)
}

pub(crate) fn close(root: &Path, request: &CloseRequest) -> OrchResult<Map<String, Value>> {
    let _lock = runtime_lock(root)?;
    let lease = load_lease(root, &request.lease)?;
    if lease.status().is_active() && !request.force {
        return Err(OrchError::coded(
            "cannot close active lease without --force",
            ErrorCode::ActiveLeaseCloseRequiresForce,
        )
        .detail("lease_id", request.lease.clone()));
    }
    let (deleted, pruned) = close_lease_files(root, &lease)?;
    let mut payload = json_ok();
    insert(&mut payload, "lease_id", request.lease.clone());
    insert_non_empty(&mut payload, "deleted", string_values(deleted));
    insert_non_empty(&mut payload, "pruned", string_values(pruned));
    Ok(payload)
}

pub(crate) fn cleanup(root: &Path, request: &CleanupRequest) -> OrchResult<Map<String, Value>> {
    if !request.completed {
        return Err(OrchError::coded(
            "cleanup requires --completed",
            ErrorCode::CleanupModeRequired,
        ));
    }
    let _lock = runtime_lock(root)?;
    let mut closed = Vec::new();
    let mut deleted = Vec::new();
    for lease in all_leases(root)? {
        if lease.status().is_active() {
            continue;
        }
        let (lease_deleted, _) = close_lease_files(root, &lease)?;
        deleted.extend(lease_deleted);
        if let Some(lease_id) = lease.id() {
            closed.push(lease_id.to_string());
        }
    }
    deleted.sort();
    deleted.dedup();
    let mut payload = json_ok();
    insert_non_empty(&mut payload, "closed", string_values(closed));
    insert_non_empty(&mut payload, "deleted", string_values(deleted));
    insert_non_empty(
        &mut payload,
        "pruned",
        string_values(prune_empty_runtime_dirs(root)),
    );
    Ok(payload)
}

pub(crate) fn packet(root: &Path, request: &PacketRequest) -> OrchResult<Map<String, Value>> {
    let _lock = runtime_lock(root)?;
    ensure_runtime_dirs(root)?;
    let mut lease = load_lease(root, &request.lease)?;
    let packet = render_packet_for_lease(root, &mut lease, &request.lease, request.role)?;
    save_lease(root, &lease)?;
    let mut payload = json_ok();
    insert(&mut payload, "lease_id", request.lease.clone());
    insert(&mut payload, "role", request.role.as_str());
    insert(&mut payload, "packet", packet);
    Ok(payload)
}

fn render_packet_for_lease(
    root: &Path,
    lease: &mut LeaseRecord,
    lease_id: &str,
    role: PacketRoleKind,
) -> OrchResult<String> {
    let report_path = report_path_for_lease(root, lease)?;
    let packet_path = repo_path(
        root,
        packets_dir(root).join(format!("{}-{}.md", lease_id, role.as_str())),
        "packet_path",
    )?;
    let report_template = format!(
        "+++\nlease_id = {}\nstatus = {}\ncommands_run = []\nresult = \"\"\n+++\n\n## Summary\n\n## Evidence\n\n## Notes\n",
        quote_toml_string(lease_id),
        quote_toml_string("ready_for_validation")
    );
    let packet = if lease.is_bud() {
        render_bud_packet(root, lease, lease_id, role, &report_path, &report_template)?
    } else {
        render_task_packet(root, lease, lease_id, role, &report_path, &report_template)?
    };
    atomic_write(&packet_path, &packet)?;
    let packet_rel = relpath(&packet_path, root);
    lease.set("packet_path", packet_rel.clone());
    if role == PacketRoleKind::Worker {
        lease.set("worker_packet_path", packet_rel.clone());
    }
    Ok(packet_rel)
}

fn render_task_packet(
    root: &Path,
    lease: &LeaseRecord,
    lease_id: &str,
    role: PacketRoleKind,
    report_path: &Path,
    report_template: &str,
) -> OrchResult<String> {
    let task_path = repo_path(root, lease.task_path(), "task_path")?;
    let task = load_task(&task_path, root)?;
    let spec_dir = task_path.parent().and_then(|p| p.parent()).unwrap_or(root);
    let policy = load_spec_policy(root, &task.spec_id)?;
    let requirements = read_optional(&spec_dir.join("requirements.md"))?;
    let design = read_optional(&spec_dir.join("design.md"))?;
    let policy_text = if policy.is_empty() {
        "{}".to_string()
    } else {
        serde_json::to_string(&Value::Object(policy.into_map())).expect("json encoding")
    };
    let scope = lease.scope().join(", ");
    Ok([
        format!("# {} Packet - {}", role.title(), lease_id),
        String::new(),
        role.note().to_string(),
        String::new(),
        "## Lease".to_string(),
        String::new(),
        format!("- Lease: `{lease_id}`"),
        format!("- Task: `{}`", lease.get_str("task").unwrap_or("")),
        format!("- Task path: `{}`", lease.task_path()),
        format!("- Owner: `{}`", lease.get_str("owner").unwrap_or("")),
        lease
            .agent_id()
            .map(|agent_id| format!("- Agent id: `{agent_id}`"))
            .unwrap_or_default(),
        format!("- Scope: `{scope}`"),
        format!("- Report path: `{}`", relpath(report_path, root)),
        format!("- Spec policy: `{policy_text}`"),
        String::new(),
        "## Worker Report Contract".to_string(),
        String::new(),
        "Write a Markdown report with TOML frontmatter to the report path. Minimal template:"
            .to_string(),
        String::new(),
        "```md".to_string(),
        report_template.trim_end().to_string(),
        "```".to_string(),
        String::new(),
        "## Task".to_string(),
        String::new(),
        crate::paths::read_text(&task_path)?.trim_end().to_string(),
        String::new(),
        "## Requirements".to_string(),
        String::new(),
        if requirements.trim_end().is_empty() {
            "(none)".to_string()
        } else {
            requirements.trim_end().to_string()
        },
        String::new(),
        "## Design".to_string(),
        String::new(),
        if design.trim_end().is_empty() {
            "(none)".to_string()
        } else {
            design.trim_end().to_string()
        },
        String::new(),
    ]
    .join("\n"))
}

fn render_bud_packet(
    root: &Path,
    lease: &LeaseRecord,
    lease_id: &str,
    role: PacketRoleKind,
    report_path: &Path,
    report_template: &str,
) -> OrchResult<String> {
    let instructions_path = lease
        .instructions_path()
        .ok_or_else(|| OrchError::new("bud lease missing instructions_path"))?;
    let instructions =
        crate::paths::read_text(&repo_path(root, instructions_path, "instructions_path")?)?;
    let scope = lease.scope().join(", ");
    let agent_line = lease
        .agent_id()
        .map(|agent_id| format!("- Agent id: `{agent_id}`"))
        .unwrap_or_default();
    Ok([
        format!("# {} Packet - {}", role.title(), lease_id),
        String::new(),
        role.note().to_string(),
        String::new(),
        "## Lease".to_string(),
        String::new(),
        format!("- Lease: `{lease_id}`"),
        "- Kind: `bud`".to_string(),
        format!("- Task: `{}`", lease.get_str("task").unwrap_or("")),
        format!("- Title: `{}`", lease.title().unwrap_or("")),
        format!("- Owner: `{}`", lease.get_str("owner").unwrap_or("")),
        agent_line,
        format!("- Scope: `{scope}`"),
        format!("- Report path: `{}`", relpath(report_path, root)),
        String::new(),
        "## Worker Report Contract".to_string(),
        String::new(),
        "Write a Markdown report with TOML frontmatter to the report path. Minimal template:"
            .to_string(),
        String::new(),
        "```md".to_string(),
        report_template.trim_end().to_string(),
        "```".to_string(),
        String::new(),
        "## Bud Instructions".to_string(),
        String::new(),
        instructions.trim_end().to_string(),
        String::new(),
        "## Lifecycle Boundary".to_string(),
        String::new(),
        "Do not call Orchid lifecycle commands.".to_string(),
        "Read this packet, stay within scope, do the work, and write your report to the provided report path.".to_string(),
        "The orchestrator owns report-check, git-touched, validation, complete, and close.".to_string(),
        String::new(),
    ]
    .join("\n"))
}

pub(crate) fn report_check(
    root: &Path,
    request: &ReportCheckRequest,
) -> OrchResult<Map<String, Value>> {
    let report_path = repo_path(root, &request.report, "report_path")?;
    let (meta, _) = split_frontmatter(&crate::paths::read_text(&report_path)?, &report_path)?;
    let report = ReportFrontmatter::from_map(meta);
    let lease_id = report.lease_id();
    if lease_id.is_empty() {
        return Err(
            OrchError::coded("report missing lease_id", ErrorCode::ReportMissingLeaseId)
                .detail("report", relpath(&report_path, root)),
        );
    }
    let lease = load_lease(root, lease_id)?;
    if !report.status().is_valid() {
        return Err(
            OrchError::coded("invalid report status", ErrorCode::InvalidReportStatus)
                .detail("status", report.status().as_str()),
        );
    }
    let mut payload = json_ok();
    insert(&mut payload, "lease_id", lease_id);
    insert(&mut payload, "task", lease.task_value());
    insert(&mut payload, "report", relpath(&report_path, root));
    insert(&mut payload, "status", report.status().as_str());
    insert(&mut payload, "next", report.status().next_action());
    Ok(payload)
}

pub(crate) fn git_status(root: &Path) -> OrchResult<Map<String, Value>> {
    let mut payload = json_ok();
    payload.extend(git_status_data(root)?);
    let active_ids = active_leases(root)?
        .into_iter()
        .filter_map(|lease| lease.id().map(str::to_string))
        .collect();
    insert_non_empty(&mut payload, "active_leases", string_values(active_ids));
    Ok(payload)
}

pub(crate) fn git_touched(root: &Path, lease_id: &str) -> OrchResult<Map<String, Value>> {
    let lease = load_lease(root, lease_id)?;
    let data = touched_for_lease(root, &lease)?;
    let mut payload = json_ok();
    payload.extend(data);
    Ok(payload)
}

pub(crate) fn git_stage_plan(root: &Path, lease_id: &str) -> OrchResult<Map<String, Value>> {
    let lease = load_lease(root, lease_id)?;
    let mut payload = json_ok();
    payload.extend(stage_plan_for_lease(root, &lease)?.to_payload());
    Ok(payload)
}

pub(crate) fn lint(root: &Path) -> OrchResult<Map<String, Value>> {
    let tasks = load_tasks(root, None)?;
    let mut seen = std::collections::BTreeSet::new();
    let mut errors = Vec::new();
    let statuses = status_set();
    for task in &tasks {
        let key = task_key(task);
        if !seen.insert(key.clone()) {
            errors.push(error_item(&key, "duplicate task id"));
        }
        if !statuses.contains(task.status().as_str()) {
            errors.push(error_item(
                &key,
                &format!("invalid status:{}", task.status()),
            ));
        }
        if task.scope().is_empty() {
            errors.push(error_item(&key, "missing scope"));
        }
        if !crate::model::VerificationMode::parse(task.verification_mode()).is_dispatchable() {
            errors.push(error_item(&key, "invalid verification_mode"));
        }
        for dep in task.depends() {
            if dep != "-" && task_by_ref(&tasks, &task.spec_id, &dep).is_none() {
                errors.push(error_item(&key, &format!("missing dependency:{dep}")));
            }
        }
    }
    let mut payload = Map::new();
    let failed = !errors.is_empty();
    if failed {
        insert(&mut payload, "ok", false);
    }
    insert_non_empty(&mut payload, "errors", objects_array(errors));
    insert(&mut payload, "tasks", tasks.len() as i64);
    Ok(payload)
}

fn compact_leases(leases: Vec<LeaseRecord>) -> OrchResult<Value> {
    Ok(Value::Array(
        leases
            .iter()
            .map(|lease| {
                compact_lease(lease, None, None).map(|lease| Value::Object(lease.to_payload()))
            })
            .collect::<OrchResult<Vec<_>>>()?,
    ))
}

fn status_for_agent(root: &Path, agent_id: &str) -> OrchResult<Map<String, Value>> {
    let matches = all_leases(root)?
        .into_iter()
        .filter(|lease| lease.agent_id() == Some(agent_id))
        .collect::<Vec<_>>();
    if matches.is_empty() {
        return Err(
            OrchError::coded("agent lease not found", ErrorCode::AgentLeaseNotFound)
                .detail("agent_id", agent_id),
        );
    }
    if matches.len() > 1 {
        let lease_ids = matches
            .iter()
            .filter_map(|lease| lease.id().map(str::to_string))
            .collect::<Vec<_>>();
        return Err(
            OrchError::coded("agent lease ambiguous", ErrorCode::AgentLeaseAmbiguous)
                .detail("agent_id", agent_id)
                .detail("leases", string_values(lease_ids)),
        );
    }
    let lease = &matches[0];
    let mut payload = json_ok();
    insert(&mut payload, "agent_id", agent_id);
    insert(&mut payload, "lease_id", lease.id_value());
    insert(&mut payload, "kind", lease.kind().as_str());
    insert(&mut payload, "status", lease.status().as_str());
    insert(&mut payload, "task", lease.task_value());
    insert(&mut payload, "owner", lease.owner_value());
    if let Some(title) = lease.title() {
        insert(&mut payload, "title", title);
    }
    if let Some(packet_path) = lease.worker_packet_path().or_else(|| lease.packet_path()) {
        insert(&mut payload, "packet", packet_path);
    }
    if let Some(report_path) = lease.report_path() {
        insert(&mut payload, "report", report_path);
    }
    Ok(payload)
}

fn ensure_lease_id_available(root: &Path, lease_id: &str) -> OrchResult<()> {
    if leases_dir(root).join(format!("{lease_id}.json")).exists() {
        return Err(
            OrchError::coded("lease id already exists", ErrorCode::LeaseIdAlreadyExists)
                .detail("lease_id", lease_id),
        );
    }
    Ok(())
}

fn unique_bud_lease_id(root: &Path, title: &str, owner: &str) -> OrchResult<LeaseId> {
    for attempt in 0..1000 {
        let seed = if attempt == 0 {
            format!("bud:{title}")
        } else {
            format!("bud:{title}:{attempt}")
        };
        let candidate = lease_id_for(std::path::Path::new(&seed), owner);
        if !leases_dir(root)
            .join(format!("{}.json", candidate.as_str()))
            .exists()
        {
            return Ok(candidate);
        }
    }
    Err(OrchError::new("could not allocate bud lease id"))
}

fn ensure_agent_id_available(
    root: &Path,
    agent_id: Option<&str>,
    except_lease: Option<&str>,
) -> OrchResult<()> {
    let Some(agent_id) = agent_id.filter(|value| !value.is_empty()) else {
        return Ok(());
    };
    for lease in all_leases(root)? {
        if lease.id() == except_lease {
            continue;
        }
        if lease.agent_id() == Some(agent_id) {
            return Err(OrchError::coded(
                "agent id already attached",
                ErrorCode::AgentIdAlreadyAttached,
            )
            .detail("agent_id", agent_id)
            .detail("lease_id", lease.id_value()));
        }
    }
    Ok(())
}

fn insert_non_empty(map: &mut Map<String, Value>, key: &str, value: Value) {
    if value.as_array().is_some_and(Vec::is_empty) {
        return;
    }
    map.insert(key.to_string(), value);
}

fn nonzero_counts(counts: Map<String, Value>) -> Map<String, Value> {
    counts
        .into_iter()
        .filter(|(_, value)| value.as_i64().unwrap_or(0) != 0)
        .collect()
}

fn specs_arg(specs: &[String]) -> Option<&[String]> {
    if specs.is_empty() {
        None
    } else {
        Some(specs)
    }
}

fn string_values(items: Vec<String>) -> Value {
    Value::Array(items.into_iter().map(Value::String).collect())
}

fn objects_array(items: Vec<Map<String, Value>>) -> Value {
    Value::Array(items.into_iter().map(Value::Object).collect())
}

fn error_item(task: &str, error: &str) -> Map<String, Value> {
    let mut item = Map::new();
    insert(&mut item, "task", task);
    insert(&mut item, "error", error);
    item
}
