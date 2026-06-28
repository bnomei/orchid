//! Markdown task files with TOML frontmatter: load, mutate, and round-trip write-back.
//!
//! Preserves field order, inline array style, and typed values so `complete` and
//! `block` can update task state without corrupting hand-authored frontmatter.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Map, Number, Value};

use crate::core::{string_list, value_to_string, OrchError, OrchResult};
use crate::model::{ReasoningEffort, TaskId, TaskStatus};
use crate::paths::{atomic_write, path_to_string, read_text, repo_path};

const FRONTMATTER: &str = "+++";
const TASK_FIELD_ORDER: &[&str] = &[
    "id",
    "title",
    "status",
    "scope",
    "depends",
    "covers",
    "verification_mode",
    "verification_status",
    "worker_reasoning_effort",
    "worker_model",
    "test_strategy",
    "slice",
    "reuse_targets",
    "read_allowlist",
    "bundle_with",
    "commit",
    "commit_review",
    "implemented_by",
    "verified_by",
    "completed_at",
    "blocked_at",
    "blocked_reason",
    "last_lease_id",
    "report",
];

/// Parsed TOML frontmatter for a task file, including per-field array style metadata.
#[derive(Debug, Clone)]
pub(crate) struct TaskFrontmatter {
    raw: Map<String, Value>,
    array_styles: BTreeMap<String, ArrayStyle>,
}

impl TaskFrontmatter {
    fn from_map_with_array_styles(
        raw: Map<String, Value>,
        array_styles: BTreeMap<String, ArrayStyle>,
    ) -> Self {
        Self { raw, array_styles }
    }

    pub(crate) fn raw_mut(&mut self) -> &mut Map<String, Value> {
        &mut self.raw
    }

    pub(crate) fn id(&self, fallback: &Path) -> String {
        self.id_model(fallback).into_string()
    }

    pub(crate) fn id_model(&self, fallback: &Path) -> TaskId {
        TaskId::from_raw(
            self.raw
                .get("id")
                .and_then(value_to_string)
                .unwrap_or_else(|| {
                    fallback
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("")
                        .to_string()
                }),
        )
    }

    pub(crate) fn status(&self) -> String {
        self.status_model().as_str().to_string()
    }

    pub(crate) fn status_model(&self) -> TaskStatus {
        TaskStatus::from_value(self.raw.get("status"))
    }

    pub(crate) fn scope(&self) -> Vec<String> {
        string_list(self.raw.get("scope"))
    }

    pub(crate) fn depends(&self) -> Vec<String> {
        string_list(self.raw.get("depends"))
    }

    pub(crate) fn verification_mode(&self) -> &str {
        self.raw
            .get("verification_mode")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub(crate) fn worker_reasoning_effort_model(&self) -> ReasoningEffort {
        ReasoningEffort::from_value(self.raw.get("worker_reasoning_effort"))
    }

    pub(crate) fn worker_reasoning_effort(&self) -> String {
        self.worker_reasoning_effort_model().as_str().to_string()
    }

    pub(crate) fn worker_model(&self) -> &str {
        self.raw
            .get("worker_model")
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub(crate) fn worker_model_is_valid(&self) -> bool {
        self.raw
            .get("worker_model")
            .is_none_or(|value| value.is_string())
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
enum ArrayStyle {
    Inline,
    Multiline,
}

#[derive(Debug, Clone)]
/// Loaded task file with parsed frontmatter and on-disk path under `specs/<id>/tasks/`.
pub(crate) struct Task {
    pub(crate) path: PathBuf,
    pub(crate) spec_id: String,
    frontmatter: TaskFrontmatter,
    pub(crate) body: String,
}

impl Task {
    pub(crate) fn frontmatter(&self) -> &TaskFrontmatter {
        &self.frontmatter
    }

    pub(crate) fn id(&self) -> String {
        self.frontmatter.id(&self.path)
    }

    pub(crate) fn status(&self) -> String {
        self.frontmatter.status()
    }

    pub(crate) fn status_model(&self) -> TaskStatus {
        self.frontmatter.status_model()
    }

    pub(crate) fn scope(&self) -> Vec<String> {
        self.frontmatter.scope()
    }

    pub(crate) fn depends(&self) -> Vec<String> {
        self.frontmatter.depends()
    }

    pub(crate) fn verification_mode(&self) -> &str {
        self.frontmatter.verification_mode()
    }

    pub(crate) fn worker_reasoning_effort_model(&self) -> ReasoningEffort {
        self.frontmatter.worker_reasoning_effort_model()
    }

    pub(crate) fn worker_reasoning_effort(&self) -> String {
        self.frontmatter.worker_reasoning_effort()
    }

    pub(crate) fn worker_model(&self) -> &str {
        self.frontmatter.worker_model()
    }

    pub(crate) fn worker_model_is_valid(&self) -> bool {
        self.frontmatter.worker_model_is_valid()
    }
}

pub(crate) fn split_frontmatter(
    text: &str,
    path: &Path,
) -> OrchResult<(Map<String, Value>, String)> {
    let (raw, body) = raw_frontmatter(text, path)?;
    let parsed: toml::Value = toml::from_str(raw).map_err(|err| {
        OrchError::new("invalid TOML frontmatter")
            .detail("path", path_to_string(path))
            .detail("message", err.to_string())
    })?;
    let Value::Object(meta) = toml_to_json(parsed) else {
        return Err(OrchError::new("invalid TOML frontmatter").detail("path", path_to_string(path)));
    };
    Ok((meta, body))
}

fn raw_frontmatter<'a>(text: &'a str, path: &Path) -> OrchResult<(&'a str, String)> {
    let start = if text.starts_with("+++\r\n") {
        FRONTMATTER.len() + 2
    } else if text.starts_with("+++\n") {
        FRONTMATTER.len() + 1
    } else {
        return Err(OrchError::new("missing TOML frontmatter").detail("path", path_to_string(path)));
    };
    let Some((end, marker)) = closing_frontmatter_marker(text, start) else {
        return Err(
            OrchError::new("unterminated TOML frontmatter").detail("path", path_to_string(path))
        );
    };
    let raw = &text[start..end];
    let body = text[end + marker.len()..].to_string();
    Ok((raw, body))
}

fn closing_frontmatter_marker(text: &str, start: usize) -> Option<(usize, &'static str)> {
    ["\n+++\n", "\n+++\r\n", "\r\n+++\n", "\r\n+++\r\n"]
        .into_iter()
        .filter_map(|marker| text[start..].find(marker).map(|idx| (idx + start, marker)))
        .min_by_key(|(idx, _)| *idx)
}

pub(crate) fn load_task(path: impl AsRef<Path>, root: &Path) -> OrchResult<Task> {
    let path = repo_path(root, path.as_ref(), "task_path")?;
    let text = read_text(&path)?;
    let (raw, _) = raw_frontmatter(&text, &path)?;
    let array_styles = array_styles(raw);
    let (mut meta, body) = split_frontmatter(&text, &path)?;
    let spec_id = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.file_name())
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_string();
    if !meta.contains_key("id") {
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        meta.insert("id".to_string(), Value::String(id));
    }
    Ok(Task {
        path,
        spec_id,
        frontmatter: TaskFrontmatter::from_map_with_array_styles(meta, array_styles),
        body,
    })
}

fn array_styles(raw: &str) -> BTreeMap<String, ArrayStyle> {
    let mut styles = BTreeMap::new();
    for line in raw.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        let value = value.trim_start();
        if !value.starts_with('[') {
            continue;
        }
        let style = if value.contains(']') {
            ArrayStyle::Inline
        } else {
            ArrayStyle::Multiline
        };
        styles.insert(key.trim().to_string(), style);
    }
    styles
}

pub(crate) fn quote_toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("string encoding")
}

fn toml_key_repr(key: &str) -> String {
    if !key.is_empty()
        && key
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    {
        key.to_string()
    } else {
        quote_toml_string(key)
    }
}

fn dump_toml_value(value: &Value, array_style: Option<ArrayStyle>) -> OrchResult<String> {
    match value {
        Value::String(raw) => Ok(quote_toml_string(raw)),
        Value::Bool(raw) => Ok(if *raw { "true" } else { "false" }.to_string()),
        Value::Number(raw) => Ok(raw.to_string()),
        Value::Object(map) => {
            let mut parts = Vec::with_capacity(map.len());
            for (key, item) in map {
                parts.push(format!(
                    "{} = {}",
                    toml_key_repr(key),
                    dump_toml_value(item, Some(ArrayStyle::Inline))?
                ));
            }
            if parts.is_empty() {
                Ok("{}".to_string())
            } else {
                Ok(format!("{{ {} }}", parts.join(", ")))
            }
        }
        Value::Array(items) => {
            if items.is_empty() {
                if array_style == Some(ArrayStyle::Multiline) {
                    return Ok("[\n]".to_string());
                }
                return Ok("[]".to_string());
            }

            let mut dumped = Vec::with_capacity(items.len());
            for item in items {
                dumped.push(dump_toml_value(item, Some(ArrayStyle::Inline))?);
            }

            if array_style == Some(ArrayStyle::Inline) {
                return Ok(format!("[{}]", dumped.join(", ")));
            }

            let lines: Vec<String> = dumped
                .into_iter()
                .map(|item| format!("    {item},"))
                .collect();
            Ok(format!("[\n{}\n]", lines.join("\n")))
        }
        Value::Null => Ok(quote_toml_string("")),
    }
}

#[cfg(test)]
fn dump_frontmatter(meta: &Map<String, Value>) -> OrchResult<String> {
    dump_frontmatter_with_array_styles(meta, &BTreeMap::new())
}

fn dump_frontmatter_with_array_styles(
    meta: &Map<String, Value>,
    array_styles: &BTreeMap<String, ArrayStyle>,
) -> OrchResult<String> {
    let mut keys: Vec<String> = TASK_FIELD_ORDER
        .iter()
        .filter(|key| meta.contains_key(**key))
        .map(|key| key.to_string())
        .collect();
    let mut remaining: Vec<String> = meta
        .keys()
        .filter(|key| !TASK_FIELD_ORDER.contains(&key.as_str()))
        .cloned()
        .collect();
    remaining.sort();
    keys.extend(remaining);

    let mut lines = vec![FRONTMATTER.to_string()];
    for key in keys {
        let value = meta.get(&key).unwrap_or(&Value::Null);
        lines.push(format!(
            "{key} = {}",
            dump_toml_value(value, array_styles.get(&key).copied())?
        ));
    }
    lines.push(FRONTMATTER.to_string());
    Ok(lines.join("\n") + "\n")
}

pub(crate) fn write_task_frontmatter(task: &Task, frontmatter: TaskFrontmatter) -> OrchResult<()> {
    atomic_write(
        &task.path,
        &(dump_frontmatter_with_array_styles(&frontmatter.raw, &frontmatter.array_styles)?
            + &task.body),
    )
}

pub(crate) fn read_optional(path: &Path) -> OrchResult<String> {
    if path.exists() {
        read_text(path)
    } else {
        Ok(String::new())
    }
}

fn toml_to_json(value: toml::Value) -> Value {
    match value {
        toml::Value::String(raw) => Value::String(raw),
        toml::Value::Integer(raw) => Value::Number(Number::from(raw)),
        toml::Value::Float(raw) => Number::from_f64(raw)
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
    use serde_json::json;

    #[test]
    fn task_frontmatter_round_trip_preserves_ordered_fields_and_body() {
        let mut meta = Map::new();
        meta.insert("id".to_string(), json!("T900"));
        meta.insert("title".to_string(), json!("Round trip"));
        meta.insert("status".to_string(), json!("todo"));
        meta.insert("scope".to_string(), json!(["src/orchid"]));
        meta.insert("depends".to_string(), json!([]));
        meta.insert("verification_mode".to_string(), json!("mayor"));

        let dumped = dump_frontmatter(&meta).unwrap();
        assert!(dumped.contains("scope = [\n    \"src/orchid\",\n]"));
        assert!(dumped.contains("depends = []"));
        let text = dumped + "\n## Context\n";
        let (parsed, body) = split_frontmatter(&text, Path::new("task.md")).unwrap();

        assert_eq!(parsed["id"], "T900");
        assert_eq!(parsed["scope"], json!(["src/orchid"]));
        assert_eq!(body, "\n## Context\n");
    }

    #[test]
    fn task_frontmatter_round_trips_float_and_nested_table_fields() {
        let mut meta = Map::new();
        meta.insert("id".to_string(), json!("T900"));
        meta.insert("status".to_string(), json!("todo"));
        meta.insert("priority".to_string(), json!(0.5));
        meta.insert(
            "meta".to_string(),
            json!({ "owner": "x", "owner.name": "alice", "team name": "search" }),
        );

        let dumped = dump_frontmatter(&meta).unwrap();
        assert!(dumped.contains("\"owner.name\" = \"alice\""));
        assert!(dumped.contains("\"team name\" = \"search\""));
        let text = dumped + "\n## Context\n";
        let (parsed, _) = split_frontmatter(&text, Path::new("task.md")).unwrap();

        assert_eq!(parsed["priority"], json!(0.5));
        assert_eq!(parsed["meta"]["owner"], "x");
        assert_eq!(parsed["meta"]["owner.name"], "alice");
        assert_eq!(parsed["meta"]["team name"], "search");
    }

    #[test]
    fn task_frontmatter_round_trips_typed_and_structured_array_items() {
        let mut meta = Map::new();
        meta.insert("id".to_string(), json!("T900"));
        meta.insert("status".to_string(), json!("todo"));
        meta.insert("covers".to_string(), json!([{ "id": "T1" }]));
        meta.insert("extra".to_string(), json!([1, 2]));

        let dumped = dump_frontmatter(&meta).unwrap();
        let text = dumped + "\n## Context\n";
        let (parsed, _) = split_frontmatter(&text, Path::new("task.md")).unwrap();

        assert_eq!(parsed["covers"], json!([{ "id": "T1" }]));
        assert_eq!(parsed["extra"], json!([1, 2]));
    }

    #[test]
    fn task_frontmatter_accepts_crlf_line_endings() {
        let text = "+++\r\nid = \"T900\"\r\nscope = [\"src/orchid\"]\r\n+++\r\n\r\n## Context\r\n";
        let (parsed, body) = split_frontmatter(text, Path::new("task.md")).unwrap();

        assert_eq!(parsed["id"], "T900");
        assert_eq!(parsed["scope"], json!(["src/orchid"]));
        assert_eq!(body, "\r\n## Context\r\n");
    }

    #[test]
    fn task_frontmatter_preserves_existing_inline_array_style() {
        let raw = "scope = [\"src/orchid\"]\ndepends = [\n    \"T001\",\n]\ncovers = [\n]\n";
        let styles = array_styles(raw);
        let mut meta = Map::new();
        meta.insert("id".to_string(), json!("T900"));
        meta.insert("scope".to_string(), json!(["src/orchid"]));
        meta.insert("depends".to_string(), json!(["T001"]));
        meta.insert("covers".to_string(), json!([]));

        let dumped = dump_frontmatter_with_array_styles(&meta, &styles).unwrap();

        assert!(dumped.contains("scope = [\"src/orchid\"]"));
        assert!(dumped.contains("depends = [\n    \"T001\",\n]"));
        assert!(dumped.contains("covers = [\n]"));
    }
}
