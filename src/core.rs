use chrono::{DateTime, Local, SecondsFormat, TimeDelta, Utc};
use serde_json::{Map, Value};

pub(crate) const DEFAULT_STALE_AFTER: &str = "30m";

#[derive(Debug, Copy, Clone)]
pub(crate) enum ErrorCode {
    ActiveLeaseCloseRequiresForce,
    CleanupModeRequired,
    HumanCheckpoint,
    InactiveSpec,
    InvalidDuration,
    InvalidJson,
    InvalidSpecId,
    LeaseNotFound,
    ParallelNotConfirmed,
    PathOutsideRepo,
    InvalidReportStatus,
    ReportMissingLeaseId,
    RuntimeLockBusy,
    ScopeConflict,
    ScopeRequired,
    ScopeSelectorConflict,
    SerialBlocked,
    SpecManual,
    SpecNotFound,
    TaskAlreadyLeased,
    TaskIdRequired,
    TaskNotTodo,
}

impl ErrorCode {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            ErrorCode::ActiveLeaseCloseRequiresForce => "active_lease_close_requires_force",
            ErrorCode::CleanupModeRequired => "cleanup_mode_required",
            ErrorCode::HumanCheckpoint => "human_checkpoint",
            ErrorCode::InactiveSpec => "inactive_spec",
            ErrorCode::InvalidDuration => "invalid_duration",
            ErrorCode::InvalidJson => "invalid_json",
            ErrorCode::InvalidSpecId => "invalid_spec_id",
            ErrorCode::LeaseNotFound => "lease_not_found",
            ErrorCode::ParallelNotConfirmed => "parallel_not_confirmed",
            ErrorCode::PathOutsideRepo => "path_outside_repo",
            ErrorCode::InvalidReportStatus => "invalid_report_status",
            ErrorCode::ReportMissingLeaseId => "report_missing_lease_id",
            ErrorCode::RuntimeLockBusy => "runtime_lock_busy",
            ErrorCode::ScopeConflict => "scope_conflict",
            ErrorCode::ScopeRequired => "scope_required",
            ErrorCode::ScopeSelectorConflict => "scope_selector_conflict",
            ErrorCode::SerialBlocked => "serial_blocked",
            ErrorCode::SpecManual => "spec_manual",
            ErrorCode::SpecNotFound => "spec_not_found",
            ErrorCode::TaskAlreadyLeased => "task_already_leased",
            ErrorCode::TaskIdRequired => "task_id_required",
            ErrorCode::TaskNotTodo => "task_not_todo",
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OrchError {
    pub(crate) message: String,
    pub(crate) code: String,
    pub(crate) details: Map<String, Value>,
}

impl OrchError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            code: error_code(&message),
            message,
            details: Map::new(),
        }
    }

    pub(crate) fn with_code(message: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: code.into(),
            details: Map::new(),
        }
    }

    pub(crate) fn coded(message: impl Into<String>, code: ErrorCode) -> Self {
        Self::with_code(message, code.as_str())
    }

    pub(crate) fn detail(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.details.insert(key.into(), value.into());
        self
    }
}

impl From<std::io::Error> for OrchError {
    fn from(error: std::io::Error) -> Self {
        OrchError::new("I/O error").detail("message", error.to_string())
    }
}

pub(crate) type OrchResult<T> = Result<T, OrchError>;

pub(crate) fn error_code(message: &str) -> String {
    let mut out = String::new();
    let mut last_was_sep = true;
    for ch in message.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            out.push(ch);
            last_was_sep = false;
        } else if !last_was_sep {
            out.push('_');
            last_was_sep = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "orch_error".to_string()
    } else {
        out
    }
}

pub(crate) fn now_iso() -> String {
    Local::now().to_rfc3339_opts(SecondsFormat::Secs, false)
}

pub(crate) fn utc_now() -> DateTime<Utc> {
    Utc::now()
}

pub(crate) fn json_ok() -> Map<String, Value> {
    Map::new()
}

pub(crate) fn json_fail(error: &str, code: Option<&str>) -> Map<String, Value> {
    let mut map = Map::new();
    map.insert("error".to_string(), Value::String(error.to_string()));
    map.insert(
        "code".to_string(),
        Value::String(
            code.map(str::to_string)
                .unwrap_or_else(|| error_code(error)),
        ),
    );
    map
}

pub(crate) fn emit(payload: &Map<String, Value>, pretty: bool) {
    let value = Value::Object(payload.clone());
    if pretty {
        println!(
            "{}",
            serde_json::to_string_pretty(&value).expect("json encoding")
        );
    } else {
        println!("{}", serde_json::to_string(&value).expect("json encoding"));
    }
}

pub(crate) fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(raw)) if raw.is_empty() => Vec::new(),
        Some(Value::String(raw)) => vec![raw.clone()],
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(value_to_string)
            .filter(|item| !item.is_empty())
            .collect(),
        Some(other) => value_to_string(other).into_iter().collect(),
    }
}

pub(crate) fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(raw) => Some(raw.clone()),
        Value::Bool(raw) => Some(raw.to_string()),
        Value::Number(raw) => Some(raw.to_string()),
        other => Some(other.to_string()),
    }
}

pub(crate) fn parse_iso_datetime(value: Option<&Value>) -> Option<DateTime<Utc>> {
    let raw = match value {
        Some(Value::String(raw)) => raw.as_str(),
        Some(other) => {
            return DateTime::parse_from_rfc3339(&other.to_string())
                .ok()
                .map(|d| d.with_timezone(&Utc))
        }
        None => return None,
    };
    if raw.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|stamp| stamp.with_timezone(&Utc))
}

pub(crate) fn parse_iso_datetime_str(raw: &str) -> Option<DateTime<Utc>> {
    if raw.is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|stamp| stamp.with_timezone(&Utc))
}

pub(crate) fn elapsed_seconds(value: Option<&Value>, now: DateTime<Utc>) -> i64 {
    parse_iso_datetime(value)
        .map(|stamp| (now - stamp).num_seconds().max(0))
        .unwrap_or(0)
}

pub(crate) fn parse_duration(value: &str) -> OrchResult<TimeDelta> {
    let raw = value.trim();
    if raw.len() < 2 {
        return Err(
            OrchError::coded("invalid duration", ErrorCode::InvalidDuration)
                .detail("duration", value),
        );
    }
    let (amount, unit) = raw.split_at(raw.len() - 1);
    let amount: i64 = amount.parse().map_err(|_| {
        OrchError::coded("invalid duration", ErrorCode::InvalidDuration).detail("duration", value)
    })?;
    match unit {
        "s" => Ok(TimeDelta::seconds(amount)),
        "m" => Ok(TimeDelta::minutes(amount)),
        "h" => Ok(TimeDelta::hours(amount)),
        "d" => Ok(TimeDelta::days(amount)),
        _ => Err(
            OrchError::coded("invalid duration", ErrorCode::InvalidDuration)
                .detail("duration", value),
        ),
    }
}

pub(crate) fn insert(map: &mut Map<String, Value>, key: &str, value: impl Into<Value>) {
    map.insert(key.to_string(), value.into());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_parser_accepts_supported_units() {
        assert_eq!(parse_duration("30s").unwrap().num_seconds(), 30);
        assert_eq!(parse_duration("15m").unwrap().num_minutes(), 15);
        assert_eq!(parse_duration("2h").unwrap().num_hours(), 2);
        assert_eq!(parse_duration("3d").unwrap().num_days(), 3);
    }

    #[test]
    fn duration_parser_uses_stable_error_code() {
        let err = parse_duration("soon").unwrap_err();
        assert_eq!(err.message, "invalid duration");
        assert_eq!(err.code, ErrorCode::InvalidDuration.as_str());
        assert_eq!(err.details["duration"], "soon");
    }
}
