use serde_json::{Map, Value};

use crate::model::{CompactLease, StagePlan};

#[derive(Debug, Clone)]
pub(crate) struct ReportReady {
    pub(crate) lease_id: String,
    pub(crate) task: String,
    pub(crate) report: String,
}

impl ReportReady {
    fn to_payload(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("lease_id".to_string(), Value::String(self.lease_id.clone()));
        map.insert("task".to_string(), Value::String(self.task.clone()));
        map.insert("report".to_string(), Value::String(self.report.clone()));
        map
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CleanupCandidate {
    pub(crate) lease_id: String,
    pub(crate) task: String,
}

impl CleanupCandidate {
    fn to_payload(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("lease_id".to_string(), Value::String(self.lease_id.clone()));
        map.insert("task".to_string(), Value::String(self.task.clone()));
        map
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ReadyTask {
    pub(crate) id: String,
    pub(crate) spec: String,
    pub(crate) task: String,
    pub(crate) scope: Vec<String>,
    pub(crate) verify: String,
}

impl ReadyTask {
    fn to_payload(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("task".to_string(), Value::String(self.task.clone()));
        map.insert(
            "scope".to_string(),
            Value::Array(self.scope.iter().cloned().map(Value::String).collect()),
        );
        map.insert("verify".to_string(), Value::String(self.verify.clone()));
        map
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BlockedTask {
    pub(crate) task: String,
    pub(crate) reason: String,
}

impl BlockedTask {
    pub(crate) fn to_payload(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("task".to_string(), Value::String(self.task.clone()));
        map.insert("reason".to_string(), Value::String(self.reason.clone()));
        map
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum Phase {
    Blocked,
    Cleanup,
    Dispatch,
    Done,
    Recover,
    Stage,
    Validate,
    Wait,
}

impl Phase {
    fn as_str(self) -> &'static str {
        match self {
            Phase::Blocked => "blocked",
            Phase::Cleanup => "cleanup",
            Phase::Dispatch => "dispatch",
            Phase::Done => "done",
            Phase::Recover => "recover",
            Phase::Stage => "stage",
            Phase::Validate => "validate",
            Phase::Wait => "wait",
        }
    }
}

pub(crate) struct NextInput {
    pub(crate) stale: Vec<CompactLease>,
    pub(crate) reports_ready: Vec<ReportReady>,
    pub(crate) active: Vec<CompactLease>,
    pub(crate) stage: Vec<StagePlan>,
    pub(crate) cleanup: Vec<CleanupCandidate>,
    pub(crate) ready: Vec<ReadyTask>,
    pub(crate) blocked: Vec<BlockedTask>,
    pub(crate) counts: Map<String, Value>,
    pub(crate) older_than: String,
    pub(crate) explain: bool,
}

pub(crate) struct NextDecision {
    pub(crate) phase: Phase,
    pub(crate) commands: Vec<Vec<String>>,
    pub(crate) details: Map<String, Value>,
}

impl NextDecision {
    pub(crate) fn to_payload(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert(
            "phase".to_string(),
            Value::String(self.phase.as_str().to_string()),
        );
        if self.commands.len() == 1 {
            map.insert(
                "cmd".to_string(),
                Value::Array(
                    self.commands[0]
                        .iter()
                        .cloned()
                        .map(Value::String)
                        .collect(),
                ),
            );
        } else if !self.commands.is_empty() {
            map.insert(
                "cmds".to_string(),
                Value::Array(
                    self.commands
                        .iter()
                        .map(|command| {
                            Value::Array(command.iter().cloned().map(Value::String).collect())
                        })
                        .collect(),
                ),
            );
        }
        map.extend(self.details.clone());
        map
    }
}

pub(crate) fn decide_next(input: NextInput) -> NextDecision {
    let NextInput {
        stale,
        reports_ready,
        active,
        stage,
        cleanup,
        ready,
        blocked,
        counts,
        older_than,
        explain,
    } = input;
    let has_blocked = !blocked.is_empty();
    let visible_blocked = if explain { blocked } else { Vec::new() };
    if !stale.is_empty() {
        let mut details = Map::new();
        details.insert("stale".to_string(), compact_array(stale));
        insert_non_empty(&mut details, "blocked", blocked_array(visible_blocked));
        return NextDecision {
            phase: Phase::Recover,
            commands: vec![vec![
                "stale".to_string(),
                "--older-than".to_string(),
                older_than,
            ]],
            details,
        };
    }
    if !reports_ready.is_empty() {
        let first = &reports_ready[0];
        let report = first.report.clone();
        let lease_id = first.lease_id.clone();
        let mut details = Map::new();
        details.insert("reports_ready".to_string(), reports_array(reports_ready));
        insert_non_empty(&mut details, "blocked", blocked_array(visible_blocked));
        return NextDecision {
            phase: Phase::Validate,
            commands: vec![
                vec!["report-check".to_string(), report],
                vec!["git-touched".to_string(), "--lease".to_string(), lease_id],
            ],
            details,
        };
    }
    if !active.is_empty() {
        let mut details = Map::new();
        details.insert("active".to_string(), compact_array(active));
        insert_non_empty(&mut details, "blocked", blocked_array(visible_blocked));
        return NextDecision {
            phase: Phase::Wait,
            commands: Vec::new(),
            details,
        };
    }

    let stage_candidates: Vec<StagePlan> = stage
        .into_iter()
        .filter(|item| !item.pathspecs.is_empty() || !item.safe_to_stage)
        .collect();
    if !stage_candidates.is_empty() {
        let first = &stage_candidates[0];
        let mut details = Map::new();
        details.insert(
            "stage".to_string(),
            Value::Array(
                stage_candidates
                    .iter()
                    .map(|item| Value::Object(item.to_payload()))
                    .collect(),
            ),
        );
        insert_non_empty(&mut details, "blocked", blocked_array(visible_blocked));
        return NextDecision {
            phase: Phase::Stage,
            commands: vec![vec![
                "git-stage-plan".to_string(),
                "--lease".to_string(),
                first.lease_id.clone(),
            ]],
            details,
        };
    }
    if !cleanup.is_empty() {
        let lease_id = cleanup[0].lease_id.clone();
        let mut details = Map::new();
        details.insert("cleanup".to_string(), cleanup_array(cleanup));
        insert_non_empty(&mut details, "blocked", blocked_array(visible_blocked));
        return NextDecision {
            phase: Phase::Cleanup,
            commands: vec![vec!["close".to_string(), "--lease".to_string(), lease_id]],
            details,
        };
    }
    if !ready.is_empty() {
        let first = &ready[0];
        let spec = first.spec.clone();
        let id = first.id.clone();
        let mut details = Map::new();
        details.insert("ready".to_string(), ready_array(ready));
        insert_non_empty(&mut details, "blocked", blocked_array(visible_blocked));
        return NextDecision {
            phase: Phase::Dispatch,
            commands: vec![vec![
                "lease".to_string(),
                spec,
                id,
                "--owner".to_string(),
                "worker:<agent-id>".to_string(),
            ]],
            details,
        };
    }

    let total: i64 = counts.values().filter_map(Value::as_i64).sum();
    let done = counts.get("done").and_then(Value::as_i64).unwrap_or(0);
    let phase = if total != 0 && done == total {
        Phase::Done
    } else if has_blocked {
        Phase::Blocked
    } else {
        Phase::Done
    };
    let mut details = Map::new();
    details.insert("counts".to_string(), Value::Object(counts));
    insert_non_empty(&mut details, "blocked", blocked_array(visible_blocked));
    NextDecision {
        phase,
        commands: Vec::new(),
        details,
    }
}

fn compact_array(items: Vec<CompactLease>) -> Value {
    Value::Array(
        items
            .into_iter()
            .map(|item| Value::Object(item.to_payload()))
            .collect(),
    )
}

fn reports_array(items: Vec<ReportReady>) -> Value {
    Value::Array(
        items
            .into_iter()
            .map(|item| Value::Object(item.to_payload()))
            .collect(),
    )
}

fn cleanup_array(items: Vec<CleanupCandidate>) -> Value {
    Value::Array(
        items
            .into_iter()
            .map(|item| Value::Object(item.to_payload()))
            .collect(),
    )
}

fn ready_array(items: Vec<ReadyTask>) -> Value {
    Value::Array(
        items
            .into_iter()
            .map(|item| Value::Object(item.to_payload()))
            .collect(),
    )
}

fn blocked_array(items: Vec<BlockedTask>) -> Value {
    Value::Array(
        items
            .into_iter()
            .map(|item| Value::Object(item.to_payload()))
            .collect(),
    )
}

fn insert_non_empty(map: &mut Map<String, Value>, key: &str, value: Value) {
    if value.as_array().is_some_and(Vec::is_empty) {
        return;
    }
    map.insert(key.to_string(), value);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> NextInput {
        NextInput {
            stale: Vec::new(),
            reports_ready: Vec::new(),
            active: Vec::new(),
            stage: Vec::new(),
            cleanup: Vec::new(),
            ready: Vec::new(),
            blocked: Vec::new(),
            counts: Map::new(),
            older_than: "30m".to_string(),
            explain: false,
        }
    }

    fn compact_lease(lease_id: &str) -> CompactLease {
        CompactLease {
            id: Value::String(lease_id.to_string()),
            task: Value::String("example/T001".to_string()),
            owner: Value::String("worker:agent_123".to_string()),
            mode: "single".to_string(),
            age: 0,
            stale: false,
        }
    }

    fn report_ready() -> ReportReady {
        ReportReady {
            lease_id: "l_report".to_string(),
            task: "example/T001".to_string(),
            report: "reports/example/T001.md".to_string(),
        }
    }

    fn cleanup_candidate() -> CleanupCandidate {
        CleanupCandidate {
            lease_id: "l_cleanup".to_string(),
            task: "example/T001".to_string(),
        }
    }

    fn ready_task() -> ReadyTask {
        ReadyTask {
            id: "T001".to_string(),
            spec: "example".to_string(),
            task: "example/T001".to_string(),
            scope: vec!["src/feature/".to_string()],
            verify: "validator".to_string(),
        }
    }

    fn blocked_task() -> BlockedTask {
        BlockedTask {
            task: "example/T999".to_string(),
            reason: "status:blocked".to_string(),
        }
    }

    fn stage_plan(safe_to_stage: bool, pathspecs: Vec<&str>) -> StagePlan {
        StagePlan {
            lease_id: "l_stage".to_string(),
            task: "example/T001".to_string(),
            safe_to_stage,
            pathspecs: pathspecs.into_iter().map(str::to_string).collect(),
            excluded: Map::new(),
        }
    }

    fn done_counts() -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("done".to_string(), Value::Number(2.into()));
        map
    }

    #[test]
    fn next_phase_priority_is_table_driven() {
        type Arrange = Box<dyn Fn(&mut NextInput)>;
        let cases: Vec<(&str, Phase, Arrange)> = vec![
            (
                "stale/recover",
                Phase::Recover,
                Box::new(|input| {
                    input.stale = vec![compact_lease("l_stale")];
                    input.reports_ready = vec![report_ready()];
                    input.active = vec![compact_lease("l_active")];
                    input.stage = vec![stage_plan(true, vec!["src/planner.rs"])];
                    input.cleanup = vec![cleanup_candidate()];
                    input.ready = vec![ready_task()];
                    input.blocked = vec![blocked_task()];
                    input.counts = done_counts();
                }),
            ),
            (
                "reports_ready/validate",
                Phase::Validate,
                Box::new(|input| {
                    input.reports_ready = vec![report_ready()];
                    input.active = vec![compact_lease("l_active")];
                    input.stage = vec![stage_plan(true, vec!["src/planner.rs"])];
                    input.cleanup = vec![cleanup_candidate()];
                    input.ready = vec![ready_task()];
                    input.blocked = vec![blocked_task()];
                    input.counts = done_counts();
                }),
            ),
            (
                "active/wait",
                Phase::Wait,
                Box::new(|input| {
                    input.active = vec![compact_lease("l_active")];
                    input.stage = vec![stage_plan(true, vec!["src/planner.rs"])];
                    input.cleanup = vec![cleanup_candidate()];
                    input.ready = vec![ready_task()];
                    input.blocked = vec![blocked_task()];
                    input.counts = done_counts();
                }),
            ),
            (
                "unsafe stage/stage",
                Phase::Stage,
                Box::new(|input| {
                    input.stage = vec![stage_plan(false, Vec::new())];
                    input.cleanup = vec![cleanup_candidate()];
                    input.ready = vec![ready_task()];
                    input.blocked = vec![blocked_task()];
                    input.counts = done_counts();
                }),
            ),
            (
                "nonempty stage/stage",
                Phase::Stage,
                Box::new(|input| {
                    input.stage = vec![stage_plan(true, vec!["src/planner.rs"])];
                    input.cleanup = vec![cleanup_candidate()];
                    input.ready = vec![ready_task()];
                    input.blocked = vec![blocked_task()];
                    input.counts = done_counts();
                }),
            ),
            (
                "cleanup/cleanup",
                Phase::Cleanup,
                Box::new(|input| {
                    input.cleanup = vec![cleanup_candidate()];
                    input.ready = vec![ready_task()];
                    input.blocked = vec![blocked_task()];
                    input.counts = done_counts();
                }),
            ),
            (
                "ready/dispatch",
                Phase::Dispatch,
                Box::new(|input| {
                    input.ready = vec![ready_task()];
                    input.blocked = vec![blocked_task()];
                    input.counts = done_counts();
                }),
            ),
            (
                "blocked-only/blocked",
                Phase::Blocked,
                Box::new(|input| {
                    input.blocked = vec![blocked_task()];
                }),
            ),
            (
                "all-done/done",
                Phase::Done,
                Box::new(|input| {
                    input.counts = done_counts();
                }),
            ),
            ("empty scope/done", Phase::Done, Box::new(|_input| {})),
        ];

        for (name, expected_phase, arrange) in cases {
            let mut input = input();
            arrange(&mut input);

            let decision = decide_next(input);

            assert_eq!(decision.phase, expected_phase, "{name}");
        }
    }
}
