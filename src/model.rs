use serde_json::{Map, Value};

use crate::core::{insert, string_list, value_to_string, ErrorCode, OrchError, OrchResult};

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct SpecId(String);

impl SpecId {
    pub(crate) fn parse(value: &str) -> OrchResult<Self> {
        let mut raw = value.trim().replace('\\', "/");
        if let Some(rest) = raw.strip_prefix("specs/") {
            raw = rest.to_string();
        }
        let valid = !raw.is_empty()
            && raw != "."
            && raw != ".."
            && !raw.contains('/')
            && raw
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_alphanumeric())
            && raw
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'));
        if !valid {
            return Err(
                OrchError::coded("invalid spec id", ErrorCode::InvalidSpecId).detail("spec", value),
            );
        }
        Ok(Self(raw))
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct TaskId(String);

impl TaskId {
    pub(crate) fn from_raw(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub(crate) struct LeaseId(String);

impl LeaseId {
    pub(crate) fn parse(value: impl Into<String>) -> OrchResult<Self> {
        let value = value.into();
        if !is_safe_lease_id(&value) {
            return Err(
                OrchError::coded("invalid lease id", ErrorCode::InvalidLeaseId)
                    .detail("lease_id", value),
            );
        }
        Ok(Self(value))
    }

    pub(crate) fn from_generated(value: impl Into<String>) -> Self {
        let value = value.into();
        debug_assert!(is_safe_lease_id(&value));
        Self(value)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

pub(crate) fn validate_lease_id(value: &str) -> OrchResult<()> {
    LeaseId::parse(value).map(|_| ())
}

fn is_safe_lease_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Scope {
    entries: Vec<String>,
}

impl Scope {
    pub(crate) fn from_entries(entries: Vec<String>) -> Self {
        Self { entries }
    }

    pub(crate) fn contains_path(&self, path: &str) -> bool {
        let norm_path = normalize_scope_entry(path);
        self.entries.iter().any(|scope| {
            let norm_scope = normalize_scope_entry(scope);
            !norm_scope.is_empty()
                && (norm_path == norm_scope
                    || norm_path.starts_with(&(norm_scope.trim_end_matches('/').to_string() + "/")))
        })
    }

    pub(crate) fn overlaps(&self, other: &Scope) -> bool {
        self.entries.iter().any(|left| {
            let ln = normalize_scope_entry(left);
            other.entries.iter().any(|right| {
                let rn = normalize_scope_entry(right);
                !ln.is_empty()
                    && !rn.is_empty()
                    && (ln == rn
                        || ln.starts_with(&(rn.trim_end_matches('/').to_string() + "/"))
                        || rn.starts_with(&(ln.trim_end_matches('/').to_string() + "/")))
            })
        })
    }
}

pub(crate) fn normalize_scope_entry(value: &str) -> String {
    let mut cleaned = value.trim().replace('\\', "/");
    while let Some(rest) = cleaned.strip_prefix("./") {
        cleaned = rest.to_string();
    }
    cleaned.trim_matches('/').to_string()
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum TaskStatus {
    Blocked,
    Done,
    PendingReview,
    PendingValidation,
    Todo,
    Unknown(String),
}

impl TaskStatus {
    pub(crate) fn from_value(value: Option<&Value>) -> Self {
        let raw = value
            .and_then(value_to_string)
            .unwrap_or_else(|| "todo".to_string());
        match raw.as_str() {
            "blocked" => Self::Blocked,
            "done" => Self::Done,
            "pending_review" => Self::PendingReview,
            "pending_validation" => Self::PendingValidation,
            "todo" => Self::Todo,
            _ => Self::Unknown(raw),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            TaskStatus::Blocked => "blocked",
            TaskStatus::Done => "done",
            TaskStatus::PendingReview => "pending_review",
            TaskStatus::PendingValidation => "pending_validation",
            TaskStatus::Todo => "todo",
            TaskStatus::Unknown(raw) => raw,
        }
    }

    pub(crate) fn is_todo(&self) -> bool {
        matches!(self, TaskStatus::Todo)
    }

    pub(crate) fn is_done(&self) -> bool {
        matches!(self, TaskStatus::Done)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum VerificationMode {
    Mayor,
    Required,
    Validator,
    Unknown(String),
}

impl VerificationMode {
    pub(crate) fn parse(raw: &str) -> Self {
        match raw {
            "mayor" => Self::Mayor,
            "required" => Self::Required,
            "validator" => Self::Validator,
            _ => Self::Unknown(raw.to_string()),
        }
    }

    pub(crate) fn is_dispatchable(&self) -> bool {
        matches!(
            self,
            VerificationMode::Mayor | VerificationMode::Required | VerificationMode::Validator
        )
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum ReasoningEffort {
    Low,
    Medium,
    High,
    XHigh,
    Unknown(String),
}

impl ReasoningEffort {
    pub(crate) fn parse(raw: &str) -> Self {
        match raw {
            "low" => Self::Low,
            "medium" => Self::Medium,
            "high" => Self::High,
            "xhigh" => Self::XHigh,
            _ => Self::Unknown(raw.to_string()),
        }
    }

    pub(crate) fn from_value(value: Option<&Value>) -> Self {
        match value {
            None => Self::Medium,
            Some(Value::String(raw)) if raw.is_empty() => Self::Unknown(raw.clone()),
            Some(Value::String(raw)) => Self::parse(raw),
            Some(other) => Self::Unknown(other.to_string()),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Unknown(raw) => raw,
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum LeaseStatus {
    Active,
    Completed,
    Released,
    Other(String),
}

impl LeaseStatus {
    pub(crate) fn from_value(value: Option<&Value>) -> Self {
        let raw = value.and_then(Value::as_str).unwrap_or("active");
        match raw {
            "active" => Self::Active,
            "completed" => Self::Completed,
            "released" => Self::Released,
            _ => Self::Other(raw.to_string()),
        }
    }

    pub(crate) fn is_active(&self) -> bool {
        matches!(self, Self::Active)
    }

    pub(crate) fn is_completed(&self) -> bool {
        matches!(self, Self::Completed)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum LeaseKind {
    Task,
    Bud,
    Other(String),
}

impl LeaseKind {
    pub(crate) fn from_value(value: Option<&Value>) -> Self {
        let raw = value.and_then(Value::as_str).unwrap_or("task");
        match raw {
            "task" => Self::Task,
            "bud" => Self::Bud,
            _ => Self::Other(raw.to_string()),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            LeaseKind::Task => "task",
            LeaseKind::Bud => "bud",
            LeaseKind::Other(raw) => raw,
        }
    }

    pub(crate) fn is_bud(&self) -> bool {
        matches!(self, Self::Bud)
    }
}

#[derive(Debug, Copy, Clone, Eq, PartialEq)]
pub(crate) enum LeaseMode {
    Parallel,
    Serial,
    Single,
}

impl LeaseMode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            LeaseMode::Parallel => "parallel",
            LeaseMode::Serial => "serial",
            LeaseMode::Single => "single",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LeaseRecord {
    data: Map<String, Value>,
}

pub(crate) struct ActiveLeaseRecordInput {
    pub(crate) lease_id: LeaseId,
    pub(crate) lease_mode: LeaseMode,
    pub(crate) owner: String,
    pub(crate) agent_id: Option<String>,
    pub(crate) task: String,
    pub(crate) task_path: String,
    pub(crate) scope: Vec<String>,
    pub(crate) started_at: String,
    pub(crate) base_head: String,
    pub(crate) baseline_changed: Value,
    pub(crate) report_path: String,
    pub(crate) worker_reasoning_effort: String,
    pub(crate) worker_model: Option<String>,
}

impl LeaseRecord {
    pub(crate) fn from_map(data: Map<String, Value>) -> Self {
        Self { data }
    }

    pub(crate) fn new_active(input: ActiveLeaseRecordInput) -> Self {
        let mut data = Map::new();
        insert(&mut data, "lease_id", input.lease_id.into_string());
        insert(&mut data, "kind", LeaseKind::Task.as_str());
        insert(&mut data, "status", LeaseStatus::Active.as_str());
        insert(&mut data, "lease_mode", input.lease_mode.as_str());
        insert(&mut data, "owner", input.owner);
        if let Some(agent_id) = input.agent_id.filter(|value| !value.is_empty()) {
            insert(&mut data, "agent_id", agent_id);
        }
        insert(&mut data, "task", input.task);
        insert(&mut data, "task_path", input.task_path);
        insert(&mut data, "scope", string_values(input.scope));
        insert(&mut data, "started_at", input.started_at.clone());
        insert(&mut data, "heartbeat_at", input.started_at);
        insert(&mut data, "base_head", input.base_head);
        insert(&mut data, "baseline_changed", input.baseline_changed);
        insert(&mut data, "packet_path", "");
        insert(&mut data, "report_path", input.report_path);
        insert(
            &mut data,
            "worker_reasoning_effort",
            input.worker_reasoning_effort,
        );
        if let Some(worker_model) = input.worker_model.filter(|value| !value.is_empty()) {
            insert(&mut data, "worker_model", worker_model);
        }
        Self { data }
    }

    pub(crate) fn get(&self, key: &str) -> Option<&Value> {
        self.data.get(key)
    }

    pub(crate) fn get_str(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Value::as_str)
    }

    pub(crate) fn set(&mut self, key: &str, value: impl Into<Value>) {
        insert(&mut self.data, key, value);
    }

    pub(crate) fn id(&self) -> Option<&str> {
        self.get_str("lease_id")
    }

    pub(crate) fn id_value(&self) -> Value {
        self.get("lease_id").cloned().unwrap_or(Value::Null)
    }

    pub(crate) fn task_value(&self) -> Value {
        self.get("task").cloned().unwrap_or(Value::Null)
    }

    pub(crate) fn status(&self) -> LeaseStatus {
        LeaseStatus::from_value(self.get("status"))
    }

    pub(crate) fn kind(&self) -> LeaseKind {
        LeaseKind::from_value(self.get("kind"))
    }

    pub(crate) fn is_bud(&self) -> bool {
        self.kind().is_bud()
    }

    pub(crate) fn mode(&self) -> String {
        self.get_str("lease_mode").unwrap_or("").to_string()
    }

    pub(crate) fn owner_value(&self) -> Value {
        self.get("owner").cloned().unwrap_or(Value::Null)
    }

    pub(crate) fn task_path(&self) -> &str {
        self.get_str("task_path").unwrap_or("")
    }

    pub(crate) fn agent_id(&self) -> Option<&str> {
        self.get_str("agent_id").filter(|value| !value.is_empty())
    }

    pub(crate) fn title(&self) -> Option<&str> {
        self.get_str("title").filter(|value| !value.is_empty())
    }

    pub(crate) fn instructions_path(&self) -> Option<&str> {
        self.get_str("instructions_path")
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn packet_path(&self) -> Option<&str> {
        self.get_str("packet_path")
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn worker_packet_path(&self) -> Option<&str> {
        self.get_str("worker_packet_path")
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn scope(&self) -> Vec<String> {
        string_list(self.get("scope"))
    }

    pub(crate) fn baseline_changed(&self) -> Vec<String> {
        string_list(self.get("baseline_changed"))
    }

    pub(crate) fn report_path(&self) -> Option<&str> {
        self.get_str("report_path").filter(|path| !path.is_empty())
    }

    pub(crate) fn worker_reasoning_effort(&self) -> Option<&str> {
        self.get_str("worker_reasoning_effort")
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn worker_model(&self) -> Option<&str> {
        self.get_str("worker_model")
            .filter(|value| !value.is_empty())
    }

    pub(crate) fn heartbeat_or_started(&self) -> Option<&Value> {
        self.get("heartbeat_at").or_else(|| self.get("started_at"))
    }

    pub(crate) fn raw(&self) -> &Map<String, Value> {
        &self.data
    }
}

impl LeaseStatus {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            LeaseStatus::Active => "active",
            LeaseStatus::Completed => "completed",
            LeaseStatus::Released => "released",
            LeaseStatus::Other(raw) => raw,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CompactLease {
    pub(crate) id: Value,
    pub(crate) task: Value,
    pub(crate) owner: Value,
    pub(crate) kind: String,
    pub(crate) agent_id: Option<String>,
    pub(crate) worker_reasoning_effort: String,
    pub(crate) worker_model: Option<String>,
    pub(crate) mode: String,
    pub(crate) age: i64,
    pub(crate) stale: bool,
}

impl CompactLease {
    pub(crate) fn to_payload(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("id".to_string(), self.id.clone());
        map.insert("task".to_string(), self.task.clone());
        map.insert("owner".to_string(), self.owner.clone());
        if self.kind != "task" {
            insert(&mut map, "kind", self.kind.clone());
        }
        if let Some(agent_id) = &self.agent_id {
            insert(&mut map, "agent_id", agent_id.clone());
        }
        insert(
            &mut map,
            "worker_reasoning_effort",
            self.worker_reasoning_effort.clone(),
        );
        if let Some(worker_model) = &self.worker_model {
            insert(&mut map, "worker_model", worker_model.clone());
        }
        insert(&mut map, "age", self.age);
        if self.mode != "single" {
            insert(&mut map, "mode", self.mode.clone());
        }
        if self.stale {
            insert(&mut map, "stale", true);
        }
        map
    }
}

#[derive(Debug, Clone)]
pub(crate) struct StagePlan {
    pub(crate) lease_id: String,
    pub(crate) task: String,
    pub(crate) safe_to_stage: bool,
    pub(crate) pathspecs: Vec<String>,
    pub(crate) excluded: Map<String, Value>,
}

impl StagePlan {
    pub(crate) fn to_payload(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("lease_id".to_string(), Value::String(self.lease_id.clone()));
        map.insert("task".to_string(), Value::String(self.task.clone()));
        if !self.safe_to_stage {
            map.insert("safe_to_stage".to_string(), Value::Bool(false));
        }
        if !self.pathspecs.is_empty() {
            map.insert(
                "pathspecs".to_string(),
                Value::Array(self.pathspecs.iter().cloned().map(Value::String).collect()),
            );
        }
        if !self.excluded.is_empty() {
            map.insert("excluded".to_string(), Value::Object(self.excluded.clone()));
        }
        map
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum ReportStatus {
    Blocked,
    Done,
    NeedsFix,
    ReadyForValidation,
    Unknown(String),
}

impl ReportStatus {
    pub(crate) fn parse(raw: &str) -> Self {
        match raw {
            "blocked" => Self::Blocked,
            "done" => Self::Done,
            "needs_fix" => Self::NeedsFix,
            "ready_for_validation" => Self::ReadyForValidation,
            _ => Self::Unknown(raw.to_string()),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Blocked => "blocked",
            Self::Done => "done",
            Self::NeedsFix => "needs_fix",
            Self::ReadyForValidation => "ready_for_validation",
            Self::Unknown(raw) => raw,
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }

    pub(crate) fn next_action(&self) -> &str {
        match self {
            Self::ReadyForValidation => "validation",
            _ => self.as_str(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ReportFrontmatter {
    data: Map<String, Value>,
    status: ReportStatus,
}

impl ReportFrontmatter {
    pub(crate) fn from_map(data: Map<String, Value>) -> Self {
        let status = ReportStatus::parse(data.get("status").and_then(Value::as_str).unwrap_or(""));
        Self { data, status }
    }

    pub(crate) fn lease_id(&self) -> &str {
        self.data
            .get("lease_id")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub(crate) fn status(&self) -> &ReportStatus {
        &self.status
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SpecPolicy {
    data: Map<String, Value>,
}

impl SpecPolicy {
    pub(crate) fn from_map(data: Map<String, Value>) -> Self {
        Self { data }
    }

    pub(crate) fn empty() -> Self {
        Self { data: Map::new() }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    pub(crate) fn is_manual(&self) -> bool {
        self.data.get("execution_policy").and_then(Value::as_str) == Some("manual")
    }

    pub(crate) fn checkpoint_before_implementation(&self) -> bool {
        self.data.get("human_checkpoint").and_then(Value::as_str) == Some("before-implementation")
    }

    pub(crate) fn into_map(self) -> Map<String, Value> {
        self.data
    }
}

fn string_values(items: Vec<String>) -> Value {
    Value::Array(items.into_iter().map(Value::String).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_normalization_and_overlap_are_path_aware() {
        assert_eq!(normalize_scope_entry("./src\\feature/"), "src/feature");
        let parent = Scope::from_entries(vec!["./src/feature/".to_string()]);
        let child = Scope::from_entries(vec!["src/feature/file.rs".to_string()]);
        let sibling = Scope::from_entries(vec!["src/feature-other".to_string()]);

        assert!(parent.contains_path("src/feature/file.rs"));
        assert!(parent.overlaps(&child));
        assert!(!parent.overlaps(&sibling));
    }

    #[test]
    fn invalid_spec_ids_keep_stable_error_code() {
        let err = SpecId::parse("../outside").unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidSpecId.as_str());
        assert_eq!(err.details["spec"], "../outside");
    }

    #[test]
    fn unknown_states_are_preserved_for_linting() {
        let raw = Value::String("strange".to_string());
        let status = TaskStatus::from_value(Some(&raw));
        assert_eq!(status.as_str(), "strange");
        assert!(!status.is_todo());

        let mode = VerificationMode::parse(raw.as_str().unwrap());
        assert!(!mode.is_dispatchable());
    }
}
