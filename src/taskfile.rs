use std::path::{Path, PathBuf};

use serde_json::{Map, Number, Value};

use crate::core::{string_list, value_to_string, OrchError, OrchResult};
use crate::model::{TaskId, TaskStatus};
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

#[derive(Debug, Clone)]
pub(crate) struct TaskFrontmatter {
    raw: Map<String, Value>,
}

impl TaskFrontmatter {
    pub(crate) fn from_map(raw: Map<String, Value>) -> Self {
        Self { raw }
    }

    pub(crate) fn raw_mut(&mut self) -> &mut Map<String, Value> {
        &mut self.raw
    }

    pub(crate) fn into_map(self) -> Map<String, Value> {
        self.raw
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
}

#[derive(Debug, Clone)]
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
}

pub(crate) fn split_frontmatter(
    text: &str,
    path: &Path,
) -> OrchResult<(Map<String, Value>, String)> {
    if !text.starts_with("+++\n") {
        return Err(OrchError::new("missing TOML frontmatter").detail("path", path_to_string(path)));
    }
    let start = FRONTMATTER.len() + 1;
    let marker = "\n+++\n";
    let Some(end) = text[start..].find(marker).map(|idx| idx + start) else {
        return Err(
            OrchError::new("unterminated TOML frontmatter").detail("path", path_to_string(path))
        );
    };
    let raw = &text[start..end];
    let body = text[end + marker.len()..].to_string();
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

pub(crate) fn load_task(path: impl AsRef<Path>, root: &Path) -> OrchResult<Task> {
    let path = repo_path(root, path.as_ref(), "task_path")?;
    let (mut meta, body) = split_frontmatter(&read_text(&path)?, &path)?;
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
        frontmatter: TaskFrontmatter::from_map(meta),
        body,
    })
}

pub(crate) fn quote_toml_string(value: &str) -> String {
    serde_json::to_string(value).expect("string encoding")
}

fn dump_toml_value(value: &Value) -> OrchResult<String> {
    match value {
        Value::String(raw) => Ok(quote_toml_string(raw)),
        Value::Bool(raw) => Ok(if *raw { "true" } else { "false" }.to_string()),
        Value::Number(raw) if raw.is_i64() || raw.is_u64() => Ok(raw.to_string()),
        Value::Array(items) => {
            if items.is_empty() {
                return Ok("[]".to_string());
            }

            let mut dumped = Vec::with_capacity(items.len());
            for item in items {
                let stringified = value_to_string(item).unwrap_or_default();
                dumped.push(format!("    {},", quote_toml_string(&stringified)));
            }
            Ok(format!("[\n{}\n]", dumped.join("\n")))
        }
        Value::Null => Ok(quote_toml_string("")),
        other => Err(OrchError::new("unsupported frontmatter value type")
            .detail("type", value_type_name(other))),
    }
}

pub(crate) fn dump_frontmatter(meta: &Map<String, Value>) -> OrchResult<String> {
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
        lines.push(format!("{key} = {}", dump_toml_value(value)?));
    }
    lines.push(FRONTMATTER.to_string());
    Ok(lines.join("\n") + "\n")
}

pub(crate) fn write_task(task: &Task, meta: &Map<String, Value>) -> OrchResult<()> {
    atomic_write(&task.path, &(dump_frontmatter(meta)? + &task.body))
}

pub(crate) fn write_task_frontmatter(task: &Task, frontmatter: TaskFrontmatter) -> OrchResult<()> {
    write_task(task, &frontmatter.into_map())
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

fn value_type_name(value: &Value) -> &'static str {
    match value {
        Value::Null => "NoneType",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "str",
        Value::Array(_) => "list",
        Value::Object(_) => "dict",
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
}
