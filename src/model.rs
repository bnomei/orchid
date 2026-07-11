//! Domain identifiers, lease records, scope rules, and JSON payload shapes.
//!
//! Types here encode the contracts between specs, runtime leases, Git staging,
//! and coordinator-facing ACK fields.

use serde_json::{Map, Value};
use sha1::{Digest, Sha1};

use crate::core::{insert, string_list, value_to_string, ErrorCode, OrchError, OrchResult};

/// Single-segment spec directory name under `specs/` (no path separators).
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

/// Task identifier from frontmatter or the Markdown filename stem.
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

/// Runtime lease filename stem; ASCII alphanumeric and underscores only.
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

pub(crate) const LEASE_SCHEMA_VERSION: i64 = 1;
pub(crate) const LEASE_CONTEXT_SCHEMA_VERSION: i64 = 1;
pub(crate) const COMPLETION_INTENT_SCHEMA_VERSION: i64 = 1;

/// Content-addressed revision for lease context snapshots and completion intents.
pub(crate) fn lease_context_revision(kind: &str, fields: &[(&str, &str)]) -> String {
    let mut hasher = Sha1::new();
    hash_context_part(&mut hasher, "kind", kind);
    for (key, value) in fields {
        hash_context_part(&mut hasher, key, value);
    }
    format!("sha1:{}", hex_bytes(hasher.finalize().as_slice()))
}

/// Revision hash for one text blob such as a task body or bud instructions.
pub(crate) fn content_revision(kind: &str, content: &str) -> String {
    lease_context_revision(kind, &[("content", content)])
}

fn hash_context_part(hasher: &mut Sha1, key: &str, value: &str) {
    hasher.update((key.len() as u64).to_be_bytes());
    hasher.update(key.as_bytes());
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value.as_bytes());
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(HEX[(byte >> 4) as usize] as char);
        output.push(HEX[(byte & 0x0f) as usize] as char);
    }
    output
}

fn lease_context_snapshot_field_error(data: &Map<String, Value>) -> Option<String> {
    let snapshot = data.get("context_snapshot")?;
    let Some(snapshot) = snapshot.as_object() else {
        return Some("lease file has invalid context_snapshot field".to_string());
    };
    if snapshot.get("schema_version").and_then(Value::as_i64) != Some(LEASE_CONTEXT_SCHEMA_VERSION)
    {
        return Some("lease file has invalid context snapshot schema_version".to_string());
    }
    let Some(kind) = snapshot.get("kind").and_then(Value::as_str) else {
        return Some("lease file has invalid context snapshot kind".to_string());
    };
    let outer_kind = data.get("kind").and_then(Value::as_str).unwrap_or("task");
    if kind != outer_kind {
        return Some("lease file context snapshot kind does not match lease kind".to_string());
    }
    let Some(revision) = snapshot.get("revision").and_then(Value::as_str) else {
        return Some("lease file has invalid context snapshot revision".to_string());
    };
    let required = match kind {
        "task" => ["task_source", "requirements", "design"].as_slice(),
        "bud" => ["instructions"].as_slice(),
        _ => return Some("lease file has invalid context snapshot kind".to_string()),
    };
    if let Some(key) = required
        .iter()
        .find(|key| !snapshot.get(**key).is_some_and(Value::is_string))
    {
        return Some(format!(
            "lease file has invalid context snapshot {key} field"
        ));
    }
    let expected_revision = match kind {
        "task" => lease_context_revision(
            kind,
            &[
                ("task_source", snapshot["task_source"].as_str().unwrap()),
                ("requirements", snapshot["requirements"].as_str().unwrap()),
                ("design", snapshot["design"].as_str().unwrap()),
            ],
        ),
        "bud" => lease_context_revision(
            kind,
            &[("instructions", snapshot["instructions"].as_str().unwrap())],
        ),
        _ => unreachable!(),
    };
    (revision != expected_revision)
        .then(|| "lease file has invalid context snapshot revision".to_string())
}

fn completion_intent_field_error(data: &Map<String, Value>) -> Option<String> {
    let intent = data.get("completion_intent")?;
    let Some(intent) = intent.as_object() else {
        return Some("lease file has invalid completion_intent field".to_string());
    };
    if data.get("kind").and_then(Value::as_str) != Some("task") {
        return Some("only task leases may contain a completion intent".to_string());
    }
    if intent.get("schema_version").and_then(Value::as_i64)
        != Some(COMPLETION_INTENT_SCHEMA_VERSION)
    {
        return Some("lease file has invalid completion intent schema_version".to_string());
    }
    let state = intent.get("state").and_then(Value::as_str);
    match (state, data.get("status").and_then(Value::as_str)) {
        (Some("prepared"), Some("active")) | (Some("committed"), Some("completed")) => {}
        _ => return Some("lease file has invalid completion intent state".to_string()),
    }
    for key in [
        "task_path",
        "task_before_revision",
        "task_after_revision",
        "task_after",
        "completed_at",
        "implemented_by",
        "verified_by",
        "verification_status",
        "report",
        "commit",
        "commit_review",
    ] {
        if !intent.get(key).is_some_and(Value::is_string) {
            return Some(format!(
                "lease file has invalid completion intent {key} field"
            ));
        }
    }
    if intent
        .get("clean_spec_research")
        .and_then(Value::as_bool)
        .is_none()
    {
        return Some(
            "lease file has invalid completion intent clean_spec_research field".to_string(),
        );
    }
    if intent.get("task_path").and_then(Value::as_str)
        != data.get("task_path").and_then(Value::as_str)
    {
        return Some("lease file completion intent task_path does not match lease".to_string());
    }
    if intent.get("verification_status").and_then(Value::as_str) != Some("passed") {
        return Some("lease file has invalid completion intent verification_status".to_string());
    }
    let after = intent.get("task_after").and_then(Value::as_str).unwrap();
    if intent.get("task_after_revision").and_then(Value::as_str)
        != Some(content_revision("completion-task", after).as_str())
    {
        return Some("lease file has invalid completion intent task_after_revision".to_string());
    }
    None
}

/// Validate lease JSON schema and required fields; return a human-readable error.
pub(crate) fn lease_record_field_error(data: &Map<String, Value>) -> Option<String> {
    let schema_version = match data.get("schema_version") {
        None => None,
        Some(Value::Number(number)) if number.as_i64() == Some(LEASE_SCHEMA_VERSION) => {
            Some(LEASE_SCHEMA_VERSION)
        }
        Some(Value::Number(number)) => {
            return Some(format!(
                "lease file has unsupported schema_version {}",
                number
            ));
        }
        Some(_) => return Some("lease file has invalid schema_version field".to_string()),
    };
    match data.get("status") {
        None => return Some("lease file is missing required status field".to_string()),
        Some(Value::String(raw)) if matches!(raw.as_str(), "active" | "completed" | "released") => {
        }
        Some(Value::String(raw)) => {
            return Some(format!("lease file has invalid status value {raw}"));
        }
        Some(_) => return Some("lease file has invalid status field".to_string()),
    }
    if let Some(value) = data.get("kind") {
        match value.as_str() {
            Some("task" | "bud") => {}
            Some(raw) => return Some(format!("lease file has invalid kind value {raw}")),
            None => return Some("lease file has invalid kind field".to_string()),
        }
    }
    if let Some(value) = data.get("lease_mode") {
        match value.as_str() {
            Some("single" | "serial" | "parallel") => {}
            Some(raw) => return Some(format!("lease file has invalid lease_mode value {raw}")),
            None => return Some("lease file has invalid lease_mode field".to_string()),
        }
    }
    if let Some(error) = lease_context_snapshot_field_error(data) {
        return Some(error);
    }
    if let Some(error) = completion_intent_field_error(data) {
        return Some(error);
    }
    schema_version?;
    for key in [
        "lease_id",
        "kind",
        "status",
        "lease_mode",
        "owner",
        "task",
        "task_path",
        "started_at",
        "heartbeat_at",
        "base_head",
        "packet_path",
        "report_path",
        "worker_reasoning_effort",
    ] {
        if !data.get(key).is_some_and(Value::is_string) {
            return Some(format!("lease file has invalid or missing {key} field"));
        }
    }
    for key in ["scope", "baseline_changed"] {
        if !data.get(key).is_some_and(|value| {
            value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string))
        }) {
            return Some(format!("lease file has invalid or missing {key} field"));
        }
    }
    for key in ["completed_changed", "released_changed"] {
        if data.get(key).is_some_and(|value| {
            !value
                .as_array()
                .is_some_and(|items| items.iter().all(Value::is_string))
        }) {
            return Some(format!("lease file has invalid {key} field"));
        }
    }
    for key in [
        "completed_changed_unavailable",
        "released_changed_unavailable",
    ] {
        if data.get(key).is_some_and(|value| !value.is_boolean()) {
            return Some(format!("lease file has invalid {key} field"));
        }
    }
    if !data.get("baseline_status").is_some_and(|value| {
        value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_object))
    }) {
        return Some("lease file has invalid or missing baseline_status field".to_string());
    }
    for key in ["baseline_fingerprints", "spec_policy"] {
        if !data.get(key).is_some_and(Value::is_object) {
            return Some(format!("lease file has invalid or missing {key} field"));
        }
    }
    if data
        .get("released_fingerprints")
        .is_some_and(|value| !value.is_object())
    {
        return Some("lease file has invalid released_fingerprints field".to_string());
    }
    if let Some(value) = data.get("agent_id") {
        if !value.is_string() {
            return Some("lease file has invalid agent_id field".to_string());
        }
    }
    if let Some(value) = data.get("worker_model") {
        if !value.is_string() {
            return Some("lease file has invalid worker_model field".to_string());
        }
    }
    let effort = ReasoningEffort::from_value(data.get("worker_reasoning_effort"));
    if !effort.is_valid() {
        return Some("lease file has invalid worker_reasoning_effort value".to_string());
    }
    None
}

fn is_safe_lease_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

/// Write-scope entries used for lease exclusivity and Git path attribution.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Scope {
    entries: Vec<String>,
}

impl Scope {
    pub(crate) fn from_entries(entries: Vec<String>) -> Self {
        Self { entries }
    }

    pub(crate) fn contains_path(&self, path: &str) -> bool {
        let norm_path = normalize_path_for_scope(path);
        self.entries.iter().any(|scope| {
            let norm_scope = normalize_scope_entry(scope);
            norm_scope.is_empty()
                || norm_path == norm_scope
                || norm_path.starts_with(&(norm_scope.trim_end_matches('/').to_string() + "/"))
        })
    }

    pub(crate) fn overlaps(&self, other: &Scope) -> bool {
        self.entries.iter().any(|left| {
            let ln = normalize_scope_entry(left);
            other.entries.iter().any(|right| {
                let rn = normalize_scope_entry(right);
                ln.is_empty()
                    || rn.is_empty()
                    || ln == rn
                    || ln.starts_with(&(rn.trim_end_matches('/').to_string() + "/"))
                    || rn.starts_with(&(ln.trim_end_matches('/').to_string() + "/"))
            })
        })
    }
}

/// Normalize a scope path entry for prefix overlap and lint checks.
pub(crate) fn normalize_scope_entry(value: &str) -> String {
    normalize_path_for_scope(&value.replace('\\', "/"))
}

fn normalize_path_for_scope(value: &str) -> String {
    let mut cleaned = value.trim();
    while let Some(rest) = cleaned.strip_prefix("./") {
        cleaned = rest;
    }
    let cleaned = cleaned.trim_matches('/');
    if cleaned == "." {
        String::new()
    } else {
        cleaned.to_string()
    }
}

/// True when a scope entry normalizes to an empty path prefix (repo-wide wildcard).
pub(crate) fn scope_entry_is_blank(value: &str) -> bool {
    normalize_scope_entry(value).is_empty()
}

/// True when a scope entry contains a `..` segment and must be rejected at lint.
pub(crate) fn scope_entry_escapes_root(value: &str) -> bool {
    normalize_scope_entry(value)
        .split('/')
        .any(|segment| segment == "..")
}

/// Task lifecycle state stored in TOML frontmatter and enforced by lease/complete gates.
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

    pub(crate) fn is_dispatchable(&self) -> bool {
        matches!(
            self,
            TaskStatus::Todo | TaskStatus::PendingValidation | TaskStatus::PendingReview
        )
    }

    pub(crate) fn is_done(&self) -> bool {
        matches!(self, TaskStatus::Done)
    }

    pub(crate) fn is_completable(&self) -> bool {
        matches!(
            self,
            TaskStatus::Todo | TaskStatus::PendingValidation | TaskStatus::PendingReview
        )
    }
}

/// Worker verification policy from task frontmatter (`mayor`, `required`, `validator`).
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

/// Worker reasoning effort copied into lease and packet metadata.
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

/// Lease lifecycle state stored in `.orchid/leases/*.json`.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) enum LeaseStatus {
    Active,
    Completed,
    Released,
    Other(String),
}

impl LeaseStatus {
    pub(crate) fn from_value(value: Option<&Value>) -> Self {
        let Some(raw) = value.and_then(Value::as_str).filter(|raw| !raw.is_empty()) else {
            return Self::Other(String::new());
        };
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

    pub(crate) fn is_released(&self) -> bool {
        matches!(self, Self::Released)
    }
}

/// Distinguishes durable task leases from ephemeral bud delegations.
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

/// Exclusive, serial, or parallel reservation relative to other active leases.
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

/// Durable lease JSON used to attribute touched files and reserve write scope.
#[derive(Debug, Clone)]
pub(crate) struct LeaseRecord {
    data: Map<String, Value>,
}

/// Fields required to mint a new active task lease record.
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
    pub(crate) baseline_fingerprints: Value,
    pub(crate) baseline_status: Value,
    pub(crate) report_path: String,
    pub(crate) spec_policy: Value,
    pub(crate) worker_reasoning_effort: String,
    pub(crate) worker_model: Option<String>,
    pub(crate) context_snapshot: Value,
}

impl LeaseRecord {
    pub(crate) fn from_map(data: Map<String, Value>) -> Self {
        Self { data }
    }

    pub(crate) fn new_active(input: ActiveLeaseRecordInput) -> Self {
        let mut data = Map::new();
        insert(&mut data, "schema_version", LEASE_SCHEMA_VERSION);
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
        insert(
            &mut data,
            "baseline_fingerprints",
            input.baseline_fingerprints,
        );
        insert(&mut data, "baseline_status", input.baseline_status);
        insert(&mut data, "packet_path", "");
        insert(&mut data, "report_path", input.report_path);
        insert(&mut data, "spec_policy", input.spec_policy);
        insert(
            &mut data,
            "worker_reasoning_effort",
            input.worker_reasoning_effort,
        );
        if let Some(worker_model) = input.worker_model.filter(|value| !value.is_empty()) {
            insert(&mut data, "worker_model", worker_model);
        }
        insert(&mut data, "context_snapshot", input.context_snapshot);
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

    pub(crate) fn has_git_baseline(&self) -> bool {
        match self.git_baseline() {
            GitBaseline::Captured { head } => !head.is_empty(),
            GitBaseline::Unavailable => false,
        }
    }

    pub(crate) fn git_baseline(&self) -> GitBaseline<'_> {
        match self.get_str("base_head").filter(|value| !value.is_empty()) {
            Some(head) => GitBaseline::Captured { head },
            None => GitBaseline::Unavailable,
        }
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

    pub(crate) fn baseline_fingerprints(&self) -> Option<&Map<String, Value>> {
        self.get("baseline_fingerprints").and_then(Value::as_object)
    }

    pub(crate) fn completed_changed(&self) -> Option<Vec<String>> {
        self.get("completed_changed")
            .map(|value| string_list(Some(value)))
    }

    pub(crate) fn released_changed(&self) -> Option<Vec<String>> {
        self.get("released_changed")
            .map(|value| string_list(Some(value)))
    }

    pub(crate) fn released_fingerprints(&self) -> Option<&Map<String, Value>> {
        self.get("released_fingerprints").and_then(Value::as_object)
    }

    pub(crate) fn context_snapshot(&self) -> Option<&Map<String, Value>> {
        self.get("context_snapshot").and_then(Value::as_object)
    }

    pub(crate) fn context_revision(&self) -> Option<&str> {
        self.context_snapshot()?
            .get("revision")
            .and_then(Value::as_str)
    }

    pub(crate) fn context_text(&self, key: &str) -> Option<&str> {
        self.context_snapshot()?.get(key).and_then(Value::as_str)
    }

    pub(crate) fn completion_intent(&self) -> Option<&Map<String, Value>> {
        self.get("completion_intent").and_then(Value::as_object)
    }

    pub(crate) fn report_path(&self) -> Option<&str> {
        self.get_str("report_path").filter(|path| !path.is_empty())
    }

    pub(crate) fn spec_policy(&self) -> Option<&Map<String, Value>> {
        self.get("spec_policy").and_then(Value::as_object)
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

/// Whether a Git `base_head` was captured when the lease started.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum GitBaseline<'a> {
    Unavailable,
    Captured { head: &'a str },
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

/// Compact lease summary for status, stale, and next-action snapshots.
#[derive(Debug, Clone)]
pub(crate) struct CompactLease {
    pub(crate) id: Value,
    pub(crate) task: Value,
    pub(crate) owner: Value,
    pub(crate) kind: String,
    pub(crate) scope: Vec<String>,
    pub(crate) agent_id: Option<String>,
    pub(crate) worker_reasoning_effort: String,
    pub(crate) worker_model: Option<String>,
    pub(crate) mode: String,
    pub(crate) heartbeat_at: Option<String>,
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
        if !self.scope.is_empty() {
            insert(&mut map, "scope", string_values(self.scope.clone()));
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
        if let Some(heartbeat_at) = &self.heartbeat_at {
            insert(&mut map, "heartbeat_at", heartbeat_at.clone());
        }
        if self.stale {
            insert(&mut map, "stale", true);
        }
        map
    }
}

/// Git staging plan attributing in-scope lease-window edits and flagging unsafe cases.
#[derive(Debug, Clone)]
pub(crate) struct StagePlan {
    pub(crate) lease_id: String,
    pub(crate) task: String,
    pub(crate) git_available: bool,
    pub(crate) safe_to_stage: bool,
    pub(crate) pathspecs: Vec<String>,
    pub(crate) records: Vec<Value>,
    pub(crate) excluded: Map<String, Value>,
    pub(crate) excluded_records: Map<String, Value>,
}

impl StagePlan {
    pub(crate) fn to_payload(&self) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("lease_id".to_string(), Value::String(self.lease_id.clone()));
        map.insert("task".to_string(), Value::String(self.task.clone()));
        map.insert("git".to_string(), Value::Bool(self.git_available));
        map.insert("safe_to_stage".to_string(), Value::Bool(self.safe_to_stage));
        if !self.pathspecs.is_empty() {
            map.insert(
                "pathspecs".to_string(),
                Value::Array(self.pathspecs.iter().cloned().map(Value::String).collect()),
            );
        }
        if !self.records.is_empty() {
            map.insert("records".to_string(), Value::Array(self.records.clone()));
        }
        if !self.excluded.is_empty() {
            map.insert("excluded".to_string(), Value::Object(self.excluded.clone()));
        }
        if !self.excluded_records.is_empty() {
            map.insert(
                "excluded_records".to_string(),
                Value::Object(self.excluded_records.clone()),
            );
        }
        map
    }
}

/// Worker or validator report status parsed from Markdown frontmatter.
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

/// Role that produced a report under `.orchid/reports/`.
#[derive(Debug, Clone)]
pub(crate) enum ReportKind {
    Worker,
    Validator,
    Reviewer,
    LoopRunner,
    Unknown(String),
}

impl ReportKind {
    fn from_value(value: Option<&Value>) -> Self {
        match value {
            None => Self::Worker,
            Some(Value::String(raw)) => match raw.as_str() {
                "worker" => Self::Worker,
                "validator" => Self::Validator,
                "reviewer" => Self::Reviewer,
                "loop-runner" => Self::LoopRunner,
                _ => Self::Unknown(raw.clone()),
            },
            Some(_) => Self::Unknown("<non-string>".to_string()),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Worker => "worker",
            Self::Validator => "validator",
            Self::Reviewer => "reviewer",
            Self::LoopRunner => "loop-runner",
            Self::Unknown(raw) => raw,
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }

    pub(crate) fn is_validator(&self) -> bool {
        matches!(self, Self::Validator)
    }

    pub(crate) fn is_worker(&self) -> bool {
        matches!(self, Self::Worker)
    }
}

/// Validator verdict when report `kind` is `validator`.
#[derive(Debug, Clone)]
pub(crate) enum ValidatorVerdict {
    Passed,
    Failed,
    Blocked,
    Unknown(String),
}

impl ValidatorVerdict {
    fn from_value(value: Option<&Value>) -> Self {
        match value {
            Some(Value::String(raw)) => match raw.as_str() {
                "passed" => Self::Passed,
                "failed" => Self::Failed,
                "blocked" => Self::Blocked,
                _ => Self::Unknown(raw.clone()),
            },
            Some(_) => Self::Unknown("<non-string>".to_string()),
            None => Self::Unknown(String::new()),
        }
    }

    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Unknown(raw) => raw,
        }
    }

    pub(crate) fn is_valid(&self) -> bool {
        !matches!(self, Self::Unknown(_))
    }
}

/// Parsed report frontmatter with typed kind, status, and validator verdict.
#[derive(Debug, Clone)]
pub(crate) struct ReportFrontmatter {
    data: Map<String, Value>,
    kind: ReportKind,
    status: ReportStatus,
    validator_verdict: ValidatorVerdict,
}

impl ReportFrontmatter {
    pub(crate) fn from_map(data: Map<String, Value>) -> Self {
        let kind = ReportKind::from_value(data.get("kind"));
        let status = ReportStatus::parse(data.get("status").and_then(Value::as_str).unwrap_or(""));
        let validator_verdict = ValidatorVerdict::from_value(data.get("verdict"));
        Self {
            data,
            kind,
            status,
            validator_verdict,
        }
    }

    pub(crate) fn lease_id(&self) -> &str {
        self.data
            .get("lease_id")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub(crate) fn lease_started_at(&self) -> Option<&str> {
        self.data.get("lease_started_at").and_then(Value::as_str)
    }

    pub(crate) fn context_revision(&self) -> Option<&str> {
        self.data.get("context_revision").and_then(Value::as_str)
    }

    pub(crate) fn status(&self) -> &ReportStatus {
        &self.status
    }

    pub(crate) fn kind(&self) -> &ReportKind {
        &self.kind
    }

    pub(crate) fn validator_verdict(&self) -> &ValidatorVerdict {
        &self.validator_verdict
    }

    pub(crate) fn commands_run_count(&self) -> Option<usize> {
        self.data
            .get("commands_run")?
            .as_array()
            .and_then(|items| items.iter().all(Value::is_string).then_some(items.len()))
    }

    pub(crate) fn result_is_non_empty(&self) -> bool {
        self.data
            .get("result")
            .and_then(Value::as_str)
            .is_some_and(|result| !result.trim().is_empty())
    }
}

/// Spec execution policy copied into leases (manual gate, fanout, checkpoints).
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

    pub(crate) fn is_manual(&self) -> bool {
        self.data.get("execution_policy").and_then(Value::as_str) == Some("manual")
    }

    pub(crate) fn checkpoint_before_implementation(&self) -> bool {
        self.data.get("human_checkpoint").and_then(Value::as_str) == Some("before-implementation")
    }

    pub(crate) fn fanout_is_serial(&self) -> bool {
        self.data.get("fanout_policy").and_then(Value::as_str) == Some("serial")
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
    fn contains_path_does_not_rewrite_backslash_in_git_path() {
        let scope = Scope::from_entries(vec!["src".to_string()]);
        assert!(scope.contains_path("src/evil.rs"));
        assert!(!scope.contains_path("src\\evil.rs"));
        let win_scope = Scope::from_entries(vec!["src\\feature".to_string()]);
        assert!(win_scope.contains_path("src/feature/file.rs"));
    }

    #[test]
    fn root_scope_spellings_collapse_consistently() {
        for value in [".", "/", "./", "", "././", "./."] {
            assert_eq!(normalize_scope_entry(value), "", "value: {value:?}");
        }
    }

    #[test]
    fn scope_entry_is_blank_detects_wildcard_spellings() {
        for value in [".", "/", "./", "", "   ", "././"] {
            assert!(scope_entry_is_blank(value), "value: {value:?}");
        }
        assert!(!scope_entry_is_blank("src/feature"));
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
        assert!(!status.is_dispatchable());

        let mode = VerificationMode::parse(raw.as_str().unwrap());
        assert!(!mode.is_dispatchable());
    }

    #[test]
    fn task_status_dispatchability_matches_lease_entry_states() {
        for raw in ["todo", "pending_validation", "pending_review"] {
            let status = TaskStatus::from_value(Some(&Value::String(raw.to_string())));
            assert!(status.is_dispatchable(), "status should dispatch: {raw}");
        }

        for raw in ["blocked", "done", "custom"] {
            let status = TaskStatus::from_value(Some(&Value::String(raw.to_string())));
            assert!(
                !status.is_dispatchable(),
                "status should not dispatch: {raw}"
            );
        }
    }
}
