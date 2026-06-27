#![allow(dead_code)]

//! Branch-local goal cycles: contract, durable state, evaluator integration, and keep/discard.
//!
//! Goals live under `.orchid/goals/<id>/` with `goal.toml`, cycle reports, and JSONL
//! traces. Cycles advance through ready, running, evaluate, and terminal decisions.

use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::core::{now_iso, parse_duration, OrchError, OrchResult};
use crate::gitstate;
use crate::paths::{
    atomic_write, atomic_write_json, goal_current_path, goal_dir, path_to_string, read_text,
    repo_path,
};
use crate::taskfile::{quote_toml_string, split_frontmatter};

const GOAL_TOML: &str = "goal.toml";
const STATE_JSON: &str = "state.json";
const RESULTS_JSONL: &str = "results.jsonl";
const MEASUREMENTS_JSONL: &str = "measurements.jsonl";

/// Sanitized identifier for a goal directory under `.orchid/goals/`.
#[derive(Debug, Clone, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct GoalId(String);

impl GoalId {
    pub(crate) fn explicit(value: &str) -> OrchResult<Self> {
        if !value.is_empty()
            && !matches!(value, "." | "..")
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        {
            return Ok(Self(value.to_string()));
        }
        Err(invalid_goal_id(value))
    }

    pub(crate) fn sanitize(value: &str) -> OrchResult<Self> {
        let raw = value.rsplit('/').next().unwrap_or(value);
        let mut out = String::new();
        let mut last_was_sep = true;
        for byte in raw.bytes() {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
                out.push(byte as char);
                last_was_sep = false;
            } else if !last_was_sep {
                out.push('-');
                last_was_sep = true;
            }
        }
        while out.ends_with(['.', '_', '-']) {
            out.pop();
        }
        Self::explicit(&out)
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for GoalId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
#[allow(dead_code)]
pub(crate) struct GoalRequest {
    pub(crate) id: Option<GoalId>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GoalInitRequest {
    pub(crate) goal_id: GoalId,
    pub(crate) goal: String,
    pub(crate) evaluator: String,
    pub(crate) primary_metric: String,
    pub(crate) direction: GoalDirection,
    pub(crate) minimum_delta: f64,
    pub(crate) max_iterations: u32,
    pub(crate) max_duration: TimeDelta,
    pub(crate) max_duration_raw: String,
    pub(crate) hypothesis: String,
    pub(crate) protected_surfaces: Vec<PathBuf>,
    pub(crate) scope: Vec<PathBuf>,
}

impl GoalInitRequest {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        goal_id: GoalId,
        goal: impl Into<String>,
        evaluator: impl Into<String>,
        primary_metric: impl Into<String>,
        direction: GoalDirection,
        minimum_delta: f64,
        max_iterations: u32,
        max_duration: impl Into<String>,
        hypothesis: impl Into<String>,
        protected_surfaces: Vec<PathBuf>,
        scope: Vec<PathBuf>,
    ) -> OrchResult<Self> {
        let max_duration_raw = max_duration.into();
        let max_duration = parse_duration(&max_duration_raw)?;
        if !minimum_delta.is_finite() {
            return Err(OrchError::new("min-delta must be a finite number")
                .detail("min_delta", minimum_delta.to_string()));
        }
        if max_iterations == 0 {
            return Err(OrchError::new("max-iterations must be at least 1")
                .detail("max_iterations", max_iterations.to_string()));
        }
        Ok(Self {
            goal_id,
            goal: goal.into(),
            evaluator: evaluator.into(),
            primary_metric: primary_metric.into(),
            direction,
            minimum_delta,
            max_iterations,
            max_duration,
            max_duration_raw,
            hypothesis: hypothesis.into(),
            protected_surfaces,
            scope,
        })
    }

    pub(crate) fn into_contract(self, started_at: String) -> GoalContract {
        GoalContract {
            goal_id: self.goal_id,
            goal: self.goal,
            evaluator: self.evaluator,
            primary_metric: self.primary_metric,
            direction: self.direction,
            minimum_delta: self.minimum_delta,
            max_iterations: self.max_iterations,
            max_duration: self.max_duration_raw,
            started_at,
            protected_surfaces: self.protected_surfaces,
            scope: self.scope,
        }
    }

    pub(crate) fn into_contract_and_state(self, started_at: String) -> (GoalContract, GoalState) {
        let hypothesis = self.hypothesis.clone();
        let contract = self.into_contract(started_at);
        let mut state = GoalState::initial(&contract);
        state.next_hypothesis = hypothesis;
        (contract, state)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Durable goal configuration persisted as `goal.toml` under `.orchid/goals/<id>/`.
pub(crate) struct GoalContract {
    pub(crate) goal_id: GoalId,
    pub(crate) goal: String,
    pub(crate) evaluator: String,
    pub(crate) primary_metric: String,
    pub(crate) direction: GoalDirection,
    pub(crate) minimum_delta: f64,
    pub(crate) max_iterations: u32,
    pub(crate) max_duration: String,
    pub(crate) started_at: String,
    pub(crate) protected_surfaces: Vec<PathBuf>,
    pub(crate) scope: Vec<PathBuf>,
}

impl GoalContract {
    pub(crate) fn write(&self, root: &Path) -> OrchResult<()> {
        let dir = safe_goal_dir(root, &self.goal_id)?;
        atomic_write(&dir.join(GOAL_TOML), &self.to_toml())?;
        atomic_write(&goal_current_path(root), &(self.goal_id.to_string() + "\n"))?;
        Ok(())
    }

    pub(crate) fn read(root: &Path, goal_id: &GoalId) -> OrchResult<Self> {
        let path = safe_goal_dir(root, goal_id)?.join(GOAL_TOML);
        toml::from_str(&read_text(&path)?).map_err(|err| {
            OrchError::new("invalid goal.toml")
                .detail("path", path_to_string(&path))
                .detail("message", err.to_string())
        })
    }

    fn to_toml(&self) -> String {
        let mut lines = vec![
            format!("goal_id = {}", quote_toml_string(self.goal_id.as_str())),
            format!("goal = {}", quote_toml_string(&self.goal)),
            format!("evaluator = {}", quote_toml_string(&self.evaluator)),
            String::new(),
            format!(
                "primary_metric = {}",
                quote_toml_string(&self.primary_metric)
            ),
            format!("direction = {}", quote_toml_string(self.direction.as_str())),
            format!("minimum_delta = {}", self.minimum_delta),
            String::new(),
            format!("max_iterations = {}", self.max_iterations),
            format!("max_duration = {}", quote_toml_string(&self.max_duration)),
            format!("started_at = {}", quote_toml_string(&self.started_at)),
            String::new(),
            format!(
                "protected_surfaces = {}",
                toml_string_array(&self.protected_surfaces)
            ),
            format!("scope = {}", toml_string_array(&self.scope)),
        ];
        lines.push(String::new());
        lines.join("\n")
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// Per-goal cycle pointer and budget counters persisted as `state.json`.
pub(crate) struct GoalState {
    pub(crate) status: GoalStatus,
    pub(crate) cycle: String,
    pub(crate) started_at: String,
    pub(crate) updated_at: String,
    pub(crate) baseline_commit: Option<String>,
    pub(crate) baseline_value: Option<f64>,
    pub(crate) best_commit: Option<String>,
    pub(crate) best_value: Option<f64>,
    pub(crate) iterations_completed: u32,
    pub(crate) budget_exhausted: bool,
    pub(crate) budget_exhausted_reason: Option<String>,
    pub(crate) next_hypothesis: String,
    pub(crate) last_decision: Option<EvaluatorRecommendation>,
    pub(crate) last_report: Option<String>,
}

impl GoalState {
    pub(crate) fn initial(contract: &GoalContract) -> Self {
        let now = now_iso();
        Self {
            status: GoalStatus::Baseline,
            cycle: first_cycle_id(),
            started_at: contract.started_at.clone(),
            updated_at: now,
            baseline_commit: None,
            baseline_value: None,
            best_commit: None,
            best_value: None,
            iterations_completed: 0,
            budget_exhausted: false,
            budget_exhausted_reason: None,
            next_hypothesis: String::new(),
            last_decision: None,
            last_report: None,
        }
    }

    pub(crate) fn write(&self, root: &Path, goal_id: &GoalId) -> OrchResult<()> {
        atomic_write_json(&safe_goal_dir(root, goal_id)?.join(STATE_JSON), self)
    }

    pub(crate) fn read(root: &Path, goal_id: &GoalId) -> OrchResult<Self> {
        let path = safe_goal_dir(root, goal_id)?.join(STATE_JSON);
        serde_json::from_str(&read_text(&path)?).map_err(|err| {
            OrchError::new("invalid goal state")
                .detail("path", path_to_string(&path))
                .detail("message", err.to_string())
        })
    }

    pub(crate) fn budget_decision(
        &self,
        contract: &GoalContract,
        now: DateTime<Utc>,
    ) -> OrchResult<BudgetDecision> {
        if self.iterations_completed >= contract.max_iterations {
            return Ok(BudgetDecision::exhausted("max_iterations"));
        }
        let started_at = DateTime::parse_from_rfc3339(&contract.started_at)
            .map_err(|err| {
                OrchError::new("invalid goal started_at")
                    .detail("started_at", contract.started_at.clone())
                    .detail("message", err.to_string())
            })?
            .with_timezone(&Utc);
        if now - started_at >= parse_duration(&contract.max_duration)? {
            return Ok(BudgetDecision::exhausted("max_duration"));
        }
        Ok(BudgetDecision::continue_goal())
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct BudgetDecision {
    pub(crate) exhausted: bool,
    pub(crate) reason: Option<String>,
}

impl BudgetDecision {
    fn continue_goal() -> Self {
        Self {
            exhausted: false,
            reason: None,
        }
    }

    fn exhausted(reason: &str) -> Self {
        Self {
            exhausted: true,
            reason: Some(reason.to_string()),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GoalStatus {
    Setup,
    Baseline,
    Ready,
    Running,
    Evaluate,
    Keep,
    Discard,
    Blocked,
    Done,
    Stopped,
}

impl GoalStatus {
    pub(crate) fn parse(value: &str) -> OrchResult<Self> {
        match value {
            "setup" => Ok(Self::Setup),
            "baseline" => Ok(Self::Baseline),
            "ready" => Ok(Self::Ready),
            "running" => Ok(Self::Running),
            "evaluate" => Ok(Self::Evaluate),
            "keep" => Ok(Self::Keep),
            "discard" => Ok(Self::Discard),
            "blocked" => Ok(Self::Blocked),
            "done" => Ok(Self::Done),
            "stopped" => Ok(Self::Stopped),
            _ => Err(OrchError::new("invalid goal status").detail("status", value)),
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum GoalDirection {
    LowerIsBetter,
    HigherIsBetter,
    Target,
}

impl GoalDirection {
    pub(crate) fn parse(value: &str) -> OrchResult<Self> {
        match value {
            "lower-is-better" => Ok(Self::LowerIsBetter),
            "higher-is-better" => Ok(Self::HigherIsBetter),
            "target" => Ok(Self::Target),
            _ => Err(OrchError::new("invalid goal direction").detail("direction", value)),
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::LowerIsBetter => "lower-is-better",
            Self::HigherIsBetter => "higher-is-better",
            Self::Target => "target",
        }
    }

    pub(crate) fn improvement(self, baseline: f64, candidate: f64) -> Option<f64> {
        match self {
            Self::HigherIsBetter => Some(candidate - baseline),
            Self::LowerIsBetter => Some(baseline - candidate),
            Self::Target => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum EvaluatorRecommendation {
    Keep,
    Discard,
    Blocked,
    Done,
}

impl EvaluatorRecommendation {
    pub(crate) fn parse(value: &str) -> OrchResult<Self> {
        match value {
            "keep" => Ok(Self::Keep),
            "discard" => Ok(Self::Discard),
            "blocked" => Ok(Self::Blocked),
            "done" => Ok(Self::Done),
            _ => {
                Err(OrchError::new("invalid evaluator recommendation")
                    .detail("recommendation", value))
            }
        }
    }
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct GoalReport {
    pub(crate) cycle: String,
    pub(crate) status: GoalReportStatus,
    pub(crate) next_hypothesis: String,
    pub(crate) body: String,
}

impl GoalReport {
    pub(crate) fn parse(text: &str, path: &Path) -> OrchResult<Self> {
        let (meta, body) = split_frontmatter(text, path)?;
        let cycle = required_string(&meta, "cycle")?;
        let status = GoalReportStatus::parse(&required_string(&meta, "status")?)?;
        let next_hypothesis = required_string(&meta, "next_hypothesis")?;
        Ok(Self {
            cycle,
            status,
            next_hypothesis,
            body,
        })
    }

    #[allow(dead_code)]
    pub(crate) fn read(path: &Path) -> OrchResult<Self> {
        Self::parse(&read_text(path)?, path)
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum GoalReportStatus {
    ReadyForEvaluation,
    Blocked,
}

impl GoalReportStatus {
    pub(crate) fn parse(value: &str) -> OrchResult<Self> {
        match value {
            "ready_for_evaluation" => Ok(Self::ReadyForEvaluation),
            "blocked" => Ok(Self::Blocked),
            _ => Err(OrchError::new("invalid goal report status").detail("status", value)),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EvaluatorResult {
    pub(crate) status: String,
    pub(crate) recommendation: EvaluatorRecommendation,
    pub(crate) metric: String,
    pub(crate) baseline: f64,
    pub(crate) candidate: f64,
    pub(crate) delta: f64,
    pub(crate) reason: String,
    pub(crate) raw: Map<String, Value>,
}

impl EvaluatorResult {
    pub(crate) fn parse_json(text: &str) -> OrchResult<Self> {
        let value: Value = serde_json::from_str(text).map_err(|err| {
            OrchError::new("invalid evaluator JSON").detail("message", err.to_string())
        })?;
        let Value::Object(raw) = value else {
            return Err(OrchError::new("invalid evaluator JSON").detail("message", "not an object"));
        };
        Ok(Self {
            status: required_string(&raw, "status")?,
            recommendation: EvaluatorRecommendation::parse(&required_string(
                &raw,
                "recommendation",
            )?)?,
            metric: required_string(&raw, "metric")?,
            baseline: required_f64(&raw, "baseline")?,
            candidate: required_f64(&raw, "candidate")?,
            delta: required_f64(&raw, "delta")?,
            reason: required_string(&raw, "reason")?,
            raw,
        })
    }

    pub(crate) fn append_measurement(
        &self,
        root: &Path,
        goal_id: &GoalId,
        cycle: &str,
    ) -> OrchResult<()> {
        let mut row = self.raw.clone();
        row.entry("cycle".to_string())
            .or_insert_with(|| Value::String(cycle.to_string()));
        append_jsonl(
            &safe_goal_dir(root, goal_id)?.join(MEASUREMENTS_JSONL),
            &row,
        )
    }
}

pub(crate) fn append_result(
    root: &Path,
    goal_id: &GoalId,
    result: &Map<String, Value>,
) -> OrchResult<()> {
    append_jsonl(&safe_goal_dir(root, goal_id)?.join(RESULTS_JSONL), result)
}

pub(crate) fn next_cycle_id(cycle: &str) -> OrchResult<String> {
    let raw = cycle
        .strip_prefix('C')
        .ok_or_else(|| OrchError::new("invalid goal cycle").detail("cycle", cycle))?;
    let number: u32 = raw
        .parse()
        .map_err(|_| OrchError::new("invalid goal cycle").detail("cycle", cycle))?;
    Ok(format!("C{:03}", number + 1))
}

pub(crate) fn first_cycle_id() -> String {
    "C001".to_string()
}

pub(crate) fn report_path(root: &Path, goal_id: &GoalId, cycle: &str) -> OrchResult<PathBuf> {
    repo_path(
        root,
        safe_goal_dir(root, goal_id)?
            .join("reports")
            .join(format!("{cycle}.md")),
        "goal_report_path",
    )
}

pub(crate) fn init_goal(root: &Path, request: GoalInitRequest) -> OrchResult<String> {
    let goal_id = request.goal_id.clone();
    let (contract, mut state) = request.into_contract_and_state(now_iso());
    match baseline_evaluator(root, &contract) {
        Ok((commit, value)) => {
            state.status = GoalStatus::Ready;
            state.baseline_commit = commit.clone();
            state.baseline_value = Some(value);
            state.best_commit = commit;
            state.best_value = Some(value);
        }
        Err(_) => {
            state.status = GoalStatus::Setup;
        }
    }
    state.updated_at = now_iso();
    contract.write(root)?;
    state.write(root, &goal_id)?;
    render_goal_prompt(root, &contract, &state)
}

pub(crate) fn current_goal(root: &Path) -> OrchResult<Option<(GoalContract, GoalState)>> {
    let current_path = goal_current_path(root);
    if !current_path.exists() {
        return Ok(None);
    }
    let raw = read_text(&current_path)?;
    let goal_id = GoalId::explicit(raw.trim())?;
    let contract = GoalContract::read(root, &goal_id)?;
    let state = GoalState::read(root, &goal_id)?;
    Ok(Some((contract, state)))
}

pub(crate) fn render_goal_prompt(
    root: &Path,
    contract: &GoalContract,
    state: &GoalState,
) -> OrchResult<String> {
    match state.status {
        GoalStatus::Setup | GoalStatus::Baseline => maybe_establish_baseline(root, contract, state),
        GoalStatus::Ready | GoalStatus::Running | GoalStatus::Evaluate => {
            render_or_evaluate_goal(root, contract, state)
        }
        GoalStatus::Keep | GoalStatus::Discard => Ok(render_goal_decision(contract, state)),
        GoalStatus::Blocked => Ok(render_goal_blocked(contract, state)),
        GoalStatus::Done | GoalStatus::Stopped => render_goal_finish(root, contract, state),
    }
}

pub(crate) fn render_goal_prompt_and_advance(
    root: &Path,
    contract: &GoalContract,
    state: &GoalState,
) -> OrchResult<String> {
    let prompt = render_goal_prompt(root, contract, state)?;
    if state.status == GoalStatus::Ready
        && !report_path(root, &contract.goal_id, &state.cycle)?.exists()
    {
        let mut next = state.clone();
        next.status = GoalStatus::Running;
        next.updated_at = now_iso();
        next.write(root, &contract.goal_id)?;
    }
    Ok(prompt)
}

pub(crate) fn render_no_goal_prompt() -> String {
    "# Goal Setup\n\nNo current goal is initialized.\n\nRun `orchid goal init` with `--goal`, `--metric`, `--direction`, `--min-delta`, `--hypothesis`, `--max-iterations`, and `--max-duration`.\n".to_string()
}

pub(crate) fn render_goal_status(
    root: &Path,
    contract: &GoalContract,
    state: &GoalState,
) -> OrchResult<String> {
    let counts = goal_result_counts(root, &contract.goal_id)?;
    Ok(format!(
        "# Goal Status\n\n- Goal: `{}`\n- Reason: `{}`\n- Best result: `{}` at `{}`\n- Budget: `{}/{}` iterations, exhausted: `{}`\n- Kept cycles: `{}`\n- Discarded cycles: `{}`\n- Branch state: current goal files are local under `.orchid/goals/{}`\n\nNo pull request was created.\n",
        contract.goal_id.as_str(),
        goal_status_label(state.status),
        optional_f64(state.best_value),
        optional_string(state.best_commit.as_deref()),
        state.iterations_completed,
        contract.max_iterations,
        state.budget_exhausted,
        counts.kept,
        counts.discarded,
        contract.goal_id.as_str(),
    ))
}

pub(crate) fn finish_goal(
    root: &Path,
    contract: &GoalContract,
    state: &GoalState,
) -> OrchResult<String> {
    let mut next = state.clone();
    if !matches!(next.status, GoalStatus::Done) {
        next.status = GoalStatus::Stopped;
    }
    next.updated_at = now_iso();
    next.write(root, &contract.goal_id)?;
    render_goal_finish(root, contract, &next)
}

pub(crate) fn render_goal_finish(
    root: &Path,
    contract: &GoalContract,
    state: &GoalState,
) -> OrchResult<String> {
    let counts = goal_result_counts(root, &contract.goal_id)?;
    Ok(format!(
        "# Goal Finish\n\n- Goal: `{}`\n- Reason: `{}`\n- Best result: `{}` at `{}`\n- Budget: `{}/{}` iterations, exhausted: `{}`\n- Kept cycles: `{}`\n- Discarded cycles: `{}`\n- Branch state: finish requested without PR creation\n\nNo pull request was created.\n",
        contract.goal_id.as_str(),
        state
            .budget_exhausted_reason
            .as_deref()
            .unwrap_or_else(|| goal_status_label(state.status)),
        optional_f64(state.best_value),
        optional_string(state.best_commit.as_deref()),
        state.iterations_completed,
        contract.max_iterations,
        state.budget_exhausted,
        counts.kept,
        counts.discarded,
    ))
}

fn baseline_evaluator(root: &Path, contract: &GoalContract) -> OrchResult<(Option<String>, f64)> {
    let head = gitstate::head_commit(root)?;
    let output = evaluator_command(root, contract)?
        .env("ORCHID_GOAL_CYCLE", first_cycle_id())
        .env("ORCHID_GOAL_BASELINE_COMMIT", head.as_deref().unwrap_or(""))
        .env("ORCHID_GOAL_BASELINE_VALUE", "")
        .output()
        .map_err(OrchError::from)?;
    if !output.status.success() {
        return Err(OrchError::new("goal evaluator failed")
            .detail("evaluator", contract.evaluator.clone())
            .detail(
                "stderr",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let result = EvaluatorResult::parse_json(&stdout)?;
    if result.metric != contract.primary_metric {
        return Err(OrchError::new("evaluator metric mismatch")
            .detail("expected", contract.primary_metric.clone())
            .detail("actual", result.metric));
    }
    Ok((head, result.baseline))
}

fn maybe_establish_baseline(
    root: &Path,
    contract: &GoalContract,
    state: &GoalState,
) -> OrchResult<String> {
    let Ok((commit, value)) = baseline_evaluator(root, contract) else {
        return Ok(render_goal_setup_for_contract(contract));
    };
    let mut next = state.clone();
    next.status = GoalStatus::Ready;
    next.baseline_commit = commit.clone();
    next.baseline_value = Some(value);
    next.best_commit = commit;
    next.best_value = Some(value);
    next.updated_at = now_iso();
    next.write(root, &contract.goal_id)?;
    render_goal_ready(root, contract, &next)
}

fn render_or_evaluate_goal(
    root: &Path,
    contract: &GoalContract,
    state: &GoalState,
) -> OrchResult<String> {
    let path = report_path(root, &contract.goal_id, &state.cycle)?;
    if !path.exists() {
        return if state.status == GoalStatus::Ready {
            render_goal_ready(root, contract, state)
        } else {
            render_goal_running(root, contract, state)
        };
    }
    let next = evaluate_cycle_report(root, contract, state, &path)?;
    render_goal_prompt(root, contract, &next)
}

fn evaluate_cycle_report(
    root: &Path,
    contract: &GoalContract,
    state: &GoalState,
    path: &Path,
) -> OrchResult<GoalState> {
    let report = GoalReport::read(path)?;
    if report.cycle != state.cycle {
        return Err(OrchError::new("goal report cycle mismatch")
            .detail("expected", state.cycle.clone())
            .detail("actual", report.cycle));
    }

    let mut next = state.clone();
    next.status = GoalStatus::Evaluate;
    next.last_report = Some(path_to_string(path));
    next.next_hypothesis = report.next_hypothesis.clone();
    next.updated_at = now_iso();

    if report.status == GoalReportStatus::Blocked {
        next.status = GoalStatus::Blocked;
        next.budget_exhausted_reason = Some("report blocked".to_string());
        let candidate_commit = gitstate::head_commit(root)?;
        append_cycle_result(
            root,
            contract,
            CycleResultTrace {
                cycle: &next.cycle,
                baseline_commit: next.baseline_commit.as_deref(),
                candidate_commit: candidate_commit.as_deref(),
                next_hypothesis: &next.next_hypothesis,
                decision: "blocked",
                evaluator: None,
                reason: Some("report blocked"),
            },
        )?;
        next.write(root, &contract.goal_id)?;
        return Ok(next);
    }

    let protected_changes =
        changed_protected_surfaces(root, contract, next.baseline_commit.as_deref())?;
    if !protected_changes.is_empty() {
        next.status = GoalStatus::Blocked;
        next.budget_exhausted_reason = Some(format!(
            "protected surface changed: {}",
            protected_changes.join(", ")
        ));
        let candidate_commit = gitstate::head_commit(root)?;
        append_cycle_result(
            root,
            contract,
            CycleResultTrace {
                cycle: &next.cycle,
                baseline_commit: next.baseline_commit.as_deref(),
                candidate_commit: candidate_commit.as_deref(),
                next_hypothesis: &next.next_hypothesis,
                decision: "blocked",
                evaluator: None,
                reason: next.budget_exhausted_reason.as_deref(),
            },
        )?;
        next.write(root, &contract.goal_id)?;
        return Ok(next);
    }

    let mut evaluator = run_evaluator(root, contract, state)?;
    if evaluator.metric != contract.primary_metric {
        return Err(OrchError::new("evaluator metric mismatch")
            .detail("expected", contract.primary_metric.clone())
            .detail("actual", evaluator.metric));
    }
    if evaluator.status != "pass" {
        evaluator.recommendation = EvaluatorRecommendation::Blocked;
        evaluator.reason = format!("evaluator status {}", evaluator.status);
    }
    evaluator.append_measurement(root, &contract.goal_id, &state.cycle)?;

    if evaluator.recommendation == EvaluatorRecommendation::Keep {
        if let Some(baseline_value) = state.baseline_value {
            if let Some(improvement) = contract
                .direction
                .improvement(baseline_value, evaluator.candidate)
            {
                if improvement < contract.minimum_delta {
                    evaluator.recommendation = EvaluatorRecommendation::Discard;
                    evaluator.reason = format!(
                        "improvement {improvement} below min_delta {}",
                        contract.minimum_delta
                    );
                }
            }
        }
    }

    next.last_decision = Some(evaluator.recommendation);
    let recommendation = evaluator.recommendation;
    next.status = match recommendation {
        EvaluatorRecommendation::Keep => GoalStatus::Keep,
        EvaluatorRecommendation::Discard => GoalStatus::Discard,
        EvaluatorRecommendation::Blocked => GoalStatus::Blocked,
        EvaluatorRecommendation::Done => GoalStatus::Done,
    };
    next.updated_at = now_iso();
    let result_cycle = next.cycle.clone();
    let result_baseline_commit = next.baseline_commit.clone();
    let result_next_hypothesis = next.next_hypothesis.clone();
    let result_reason = evaluator.reason.clone();

    if recommendation == EvaluatorRecommendation::Keep {
        let protected_changes =
            changed_protected_surfaces(root, contract, next.baseline_commit.as_deref())?;
        if !protected_changes.is_empty() {
            next.status = GoalStatus::Blocked;
            next.last_decision = Some(EvaluatorRecommendation::Blocked);
            next.budget_exhausted_reason = Some(format!(
                "protected surface changed: {}",
                protected_changes.join(", ")
            ));
            let candidate_commit = gitstate::head_commit(root)?;
            append_cycle_result(
                root,
                contract,
                CycleResultTrace {
                    cycle: &result_cycle,
                    baseline_commit: result_baseline_commit.as_deref(),
                    candidate_commit: candidate_commit.as_deref(),
                    next_hypothesis: &result_next_hypothesis,
                    decision: "blocked",
                    evaluator: None,
                    reason: next.budget_exhausted_reason.as_deref(),
                },
            )?;
            next.write(root, &contract.goal_id)?;
            return Ok(next);
        }
    }

    if recommendation == EvaluatorRecommendation::Keep {
        close_cycle(root, contract, &mut next, &evaluator)?;
        append_cycle_result(
            root,
            contract,
            CycleResultTrace {
                cycle: &result_cycle,
                baseline_commit: result_baseline_commit.as_deref(),
                candidate_commit: next.best_commit.as_deref(),
                next_hypothesis: &result_next_hypothesis,
                decision: recommendation_label(recommendation),
                evaluator: Some(&evaluator),
                reason: Some(&result_reason),
            },
        )?;
    } else {
        let candidate_commit = gitstate::head_commit(root)?;
        append_cycle_result(
            root,
            contract,
            CycleResultTrace {
                cycle: &result_cycle,
                baseline_commit: result_baseline_commit.as_deref(),
                candidate_commit: candidate_commit.as_deref(),
                next_hypothesis: &result_next_hypothesis,
                decision: recommendation_label(recommendation),
                evaluator: Some(&evaluator),
                reason: Some(&result_reason),
            },
        )?;
        close_cycle(root, contract, &mut next, &evaluator)?;
    }

    let budget = next.budget_decision(contract, crate::core::utc_now())?;
    if budget.exhausted && next.status != GoalStatus::Blocked {
        next.status = GoalStatus::Done;
        next.budget_exhausted = true;
        next.budget_exhausted_reason = budget.reason;
    } else if matches!(next.status, GoalStatus::Keep | GoalStatus::Discard) {
        next.status = GoalStatus::Ready;
    }
    next.write(root, &contract.goal_id)?;
    Ok(next)
}

fn run_evaluator(
    root: &Path,
    contract: &GoalContract,
    state: &GoalState,
) -> OrchResult<EvaluatorResult> {
    let output = evaluator_command(root, contract)?
        .env("ORCHID_GOAL_CYCLE", &state.cycle)
        .env(
            "ORCHID_GOAL_BASELINE_COMMIT",
            state.baseline_commit.as_deref().unwrap_or(""),
        )
        .env(
            "ORCHID_GOAL_BASELINE_VALUE",
            state
                .baseline_value
                .map(|value| value.to_string())
                .unwrap_or_default(),
        )
        .env("ORCHID_GOAL_DIRECTION", contract.direction.as_str())
        .env("ORCHID_GOAL_MIN_DELTA", contract.minimum_delta.to_string())
        .output()
        .map_err(OrchError::from)?;
    if !output.status.success() {
        return Err(OrchError::new("goal evaluator failed")
            .detail("evaluator", contract.evaluator.clone())
            .detail(
                "stderr",
                String::from_utf8_lossy(&output.stderr).trim().to_string(),
            ));
    }
    EvaluatorResult::parse_json(&String::from_utf8_lossy(&output.stdout))
}

fn evaluator_command(root: &Path, contract: &GoalContract) -> OrchResult<Command> {
    let goal_dir = safe_goal_dir(root, &contract.goal_id)?;
    let mut command = Command::new("sh");
    command
        .arg("-c")
        .arg(&contract.evaluator)
        .current_dir(root)
        .env("ORCHID_GOAL_ID", contract.goal_id.as_str())
        .env("ORCHID_GOAL_DIR", goal_dir);
    Ok(command)
}

fn changed_protected_surfaces(
    root: &Path,
    contract: &GoalContract,
    baseline: Option<&str>,
) -> OrchResult<Vec<String>> {
    let scopes: Vec<String> = contract
        .protected_surfaces
        .iter()
        .map(|path| path_to_string(path))
        .collect();
    gitstate::changed_protected_paths(root, &scopes, baseline)
}

fn close_cycle(
    root: &Path,
    contract: &GoalContract,
    state: &mut GoalState,
    evaluator: &EvaluatorResult,
) -> OrchResult<()> {
    match evaluator.recommendation {
        EvaluatorRecommendation::Keep => keep_cycle(root, contract, state, evaluator),
        EvaluatorRecommendation::Discard => discard_cycle(root, state, evaluator),
        EvaluatorRecommendation::Blocked => {
            state.status = GoalStatus::Blocked;
            state.budget_exhausted_reason = Some(evaluator.reason.clone());
            Ok(())
        }
        EvaluatorRecommendation::Done => {
            state.status = GoalStatus::Done;
            state.iterations_completed = state.iterations_completed.saturating_add(1);
            Ok(())
        }
    }
}

fn keep_cycle(
    root: &Path,
    contract: &GoalContract,
    state: &mut GoalState,
    evaluator: &EvaluatorResult,
) -> OrchResult<()> {
    let scope: Vec<String> = contract
        .scope
        .iter()
        .map(|path| path_to_string(path))
        .collect();
    let staged = gitstate::stage_goal_candidates(root, &scope)?;
    let commit = if staged.is_empty() {
        gitstate::head_commit(root)?
            .ok_or_else(|| OrchError::new("goal keep requires a git head commit"))?
    } else {
        gitstate::commit_goal_keep(root, contract.goal_id.as_str(), &state.cycle)?
    };
    state.baseline_commit = Some(commit.clone());
    state.baseline_value = Some(evaluator.candidate);
    state.best_commit = Some(commit);
    state.best_value = Some(evaluator.candidate);
    state.iterations_completed = state.iterations_completed.saturating_add(1);
    state.cycle = next_cycle_id(&state.cycle)?;
    Ok(())
}

fn discard_cycle(
    root: &Path,
    state: &mut GoalState,
    _evaluator: &EvaluatorResult,
) -> OrchResult<()> {
    let baseline = state
        .baseline_commit
        .clone()
        .ok_or_else(|| OrchError::new("goal baseline commit missing"))?;
    gitstate::reset_hard(root, &baseline)?;
    gitstate::clean_goal_candidates(root)?;
    state.iterations_completed = state.iterations_completed.saturating_add(1);
    state.cycle = next_cycle_id(&state.cycle)?;
    Ok(())
}

struct CycleResultTrace<'a> {
    cycle: &'a str,
    baseline_commit: Option<&'a str>,
    candidate_commit: Option<&'a str>,
    next_hypothesis: &'a str,
    decision: &'a str,
    evaluator: Option<&'a EvaluatorResult>,
    reason: Option<&'a str>,
}

fn append_cycle_result(
    root: &Path,
    contract: &GoalContract,
    trace: CycleResultTrace<'_>,
) -> OrchResult<()> {
    let mut row = Map::new();
    row.insert("cycle".to_string(), Value::String(trace.cycle.to_string()));
    row.insert(
        "decision".to_string(),
        Value::String(trace.decision.to_string()),
    );
    row.insert(
        "baseline_commit".to_string(),
        trace
            .baseline_commit
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
    row.insert(
        "candidate_commit".to_string(),
        trace
            .candidate_commit
            .map(|value| Value::String(value.to_string()))
            .unwrap_or(Value::Null),
    );
    row.insert(
        "next_hypothesis".to_string(),
        Value::String(trace.next_hypothesis.to_string()),
    );
    if let Some(reason) = trace.reason {
        row.insert("reason".to_string(), Value::String(reason.to_string()));
    }
    if let Some(evaluator) = trace.evaluator {
        row.insert(
            "metric".to_string(),
            Value::String(evaluator.metric.clone()),
        );
        row.insert("baseline".to_string(), json_number(evaluator.baseline)?);
        row.insert("candidate".to_string(), json_number(evaluator.candidate)?);
        row.insert("delta".to_string(), json_number(evaluator.delta)?);
    }
    append_result(root, &contract.goal_id, &row)
}

fn json_number(value: f64) -> OrchResult<Value> {
    serde_json::Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| OrchError::new("invalid JSON").detail("message", "non-finite number"))
}

fn recommendation_label(recommendation: EvaluatorRecommendation) -> &'static str {
    match recommendation {
        EvaluatorRecommendation::Keep => "keep",
        EvaluatorRecommendation::Discard => "discard",
        EvaluatorRecommendation::Blocked => "blocked",
        EvaluatorRecommendation::Done => "done",
    }
}

fn render_goal_setup_for_contract(contract: &GoalContract) -> String {
    format!(
        "# Goal Setup\n\n- Goal: `{}`\n- Evaluator: `{}`\n- Metric: `{}`\n\nMake `{}` run successfully from the repository root and print valid evaluator JSON with `status`, `recommendation`, `metric`, `baseline`, `candidate`, `delta`, and `reason` fields.\n\nAfter the evaluator works, run `orchid goal init` again with the same flags or run `orchid goal` to continue from the current state.\n",
        contract.goal_id.as_str(),
        contract.evaluator,
        contract.primary_metric,
        contract.evaluator,
    )
}

fn render_goal_ready(
    root: &Path,
    contract: &GoalContract,
    state: &GoalState,
) -> OrchResult<String> {
    let report_path = report_path(root, &contract.goal_id, &state.cycle)?;
    Ok(format!(
        "# Goal Ready\n\n- Goal: `{}`\n- Cycle: `{}`\n- Metric: `{}`\n- Direction: `{}`\n- Baseline: `{}` at `{}`\n- Budget: `{}/{}` iterations, max duration `{}`\n- Next hypothesis: {}\n- Recent results: `{}`\n- Report path: `{}`\n\nImplement one focused attempt, then write a Markdown report at the report path with TOML frontmatter containing `cycle`, `status`, and `next_hypothesis`.\n",
        contract.goal_id.as_str(),
        state.cycle,
        contract.primary_metric,
        contract.direction.as_str(),
        optional_f64(state.baseline_value),
        optional_string(state.baseline_commit.as_deref()),
        state.iterations_completed,
        contract.max_iterations,
        contract.max_duration,
        state.next_hypothesis,
        optional_string(state.last_report.as_deref()),
        path_to_string(&report_path),
    ))
}

fn render_goal_running(
    root: &Path,
    contract: &GoalContract,
    state: &GoalState,
) -> OrchResult<String> {
    let report_path = report_path(root, &contract.goal_id, &state.cycle)?;
    Ok(format!(
        "# Goal Running\n\n- Goal: `{}`\n- Cycle: `{}`\n- Expected report path: `{}`\n\nWrite the cycle report before asking Orchid to evaluate this attempt.\n",
        contract.goal_id.as_str(),
        state.cycle,
        path_to_string(&report_path),
    ))
}

fn render_goal_decision(contract: &GoalContract, state: &GoalState) -> String {
    format!(
        "# Goal Decision\n\n- Goal: `{}`\n- Cycle: `{}`\n- Decision: `{}`\n- Next hypothesis: {}\n\nOrchid recorded the decision and durable traces. Git keep/discard effects are pending.\n",
        contract.goal_id.as_str(),
        state.cycle,
        goal_status_label(state.status),
        state.next_hypothesis,
    )
}

fn render_goal_blocked(contract: &GoalContract, state: &GoalState) -> String {
    format!(
        "# Goal Blocked\n\n- Goal: `{}`\n- Cycle: `{}`\n- Reason: `{}`\n",
        contract.goal_id.as_str(),
        state.cycle,
        state
            .budget_exhausted_reason
            .as_deref()
            .unwrap_or("blocked"),
    )
}

#[derive(Default)]
struct GoalResultCounts {
    kept: usize,
    discarded: usize,
}

fn goal_result_counts(root: &Path, goal_id: &GoalId) -> OrchResult<GoalResultCounts> {
    let path = safe_goal_dir(root, goal_id)?.join(RESULTS_JSONL);
    if !path.exists() {
        return Ok(GoalResultCounts::default());
    }
    let mut counts = GoalResultCounts::default();
    for (index, line) in read_text(&path)?.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|err| {
            OrchError::new("invalid goal result trace")
                .detail("path", path_to_string(&path))
                .detail("line", (index + 1).to_string())
                .detail("message", err.to_string())
        })?;
        match value.get("decision").and_then(Value::as_str) {
            Some("keep") => counts.kept += 1,
            Some("discard") => counts.discarded += 1,
            _ => {}
        }
    }
    Ok(counts)
}

fn goal_status_label(status: GoalStatus) -> &'static str {
    match status {
        GoalStatus::Setup => "setup",
        GoalStatus::Baseline => "baseline",
        GoalStatus::Ready => "ready",
        GoalStatus::Running => "running",
        GoalStatus::Evaluate => "evaluate",
        GoalStatus::Keep => "keep",
        GoalStatus::Discard => "discard",
        GoalStatus::Blocked => "blocked",
        GoalStatus::Done => "done",
        GoalStatus::Stopped => "stopped",
    }
}

fn optional_f64(value: Option<f64>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "pending".to_string())
}

fn optional_string(value: Option<&str>) -> &str {
    value.unwrap_or("pending")
}

fn safe_goal_dir(root: &Path, goal_id: &GoalId) -> OrchResult<PathBuf> {
    let goals = repo_path(root, crate::paths::goals_dir(root), "goals_dir")?;
    let dir = repo_path(root, goal_dir(root, goal_id.as_str()), "goal_dir")?;
    if !dir.starts_with(&goals) || dir == goals {
        return Err(invalid_goal_id(goal_id.as_str()));
    }
    Ok(dir)
}

fn append_jsonl(path: &Path, data: &Map<String, Value>) -> OrchResult<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    serde_json::to_writer(&mut file, data)
        .map_err(|err| OrchError::new("invalid JSON").detail("message", err.to_string()))?;
    file.write_all(b"\n")?;
    file.sync_all()?;
    Ok(())
}

fn required_string(map: &Map<String, Value>, key: &str) -> OrchResult<String> {
    match map.get(key).and_then(Value::as_str) {
        Some(value) if !value.is_empty() => Ok(value.to_string()),
        _ => Err(OrchError::new("missing required goal field").detail("field", key)),
    }
}

fn required_f64(map: &Map<String, Value>, key: &str) -> OrchResult<f64> {
    match map.get(key).and_then(Value::as_f64) {
        Some(value) => Ok(value),
        None => Err(OrchError::new("missing required goal field").detail("field", key)),
    }
}

fn toml_string_array(paths: &[PathBuf]) -> String {
    let values: Vec<String> = paths
        .iter()
        .map(|path| quote_toml_string(&path_to_string(path)))
        .collect();
    format!("[{}]", values.join(", "))
}

fn invalid_goal_id(value: &str) -> OrchError {
    OrchError::with_code("invalid goal id", "invalid_goal_id").detail("goal_id", value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn explicit_goal_ids_accept_only_safe_ascii() {
        assert_eq!(
            GoalId::explicit("search.v2_ok-1").unwrap().as_str(),
            "search.v2_ok-1"
        );
        assert_eq!(GoalId::explicit("").unwrap_err().code, "invalid_goal_id");
        assert_eq!(GoalId::explicit("..").unwrap_err().code, "invalid_goal_id");
        assert_eq!(
            GoalId::explicit("../nope").unwrap_err().code,
            "invalid_goal_id"
        );
        assert_eq!(
            GoalId::explicit("ümlaut").unwrap_err().code,
            "invalid_goal_id"
        );
    }

    #[test]
    fn branch_goal_ids_sanitize_to_branch_leaf() {
        assert_eq!(
            GoalId::sanitize("loop/search-ranking-proof")
                .unwrap()
                .as_str(),
            "search-ranking-proof"
        );
        assert_eq!(
            GoalId::sanitize("feature/Search Ranking!")
                .unwrap()
                .as_str(),
            "Search-Ranking"
        );
        assert_eq!(GoalId::sanitize("///").unwrap_err().code, "invalid_goal_id");
    }

    #[test]
    fn parses_goal_direction_recommendation_and_status() {
        assert_eq!(
            GoalDirection::parse("lower-is-better").unwrap(),
            GoalDirection::LowerIsBetter
        );
        assert_eq!(
            EvaluatorRecommendation::parse("discard").unwrap(),
            EvaluatorRecommendation::Discard
        );
        assert_eq!(GoalStatus::parse("blocked").unwrap(), GoalStatus::Blocked);
        assert!(GoalDirection::parse("faster").is_err());
        assert!(EvaluatorRecommendation::parse("maybe").is_err());
        assert!(GoalStatus::parse("waiting").is_err());
    }

    #[test]
    fn parses_goal_report_frontmatter() {
        let text = r#"+++
cycle = "C001"
status = "ready_for_evaluation"
next_hypothesis = "precompute weights"
+++

## Summary
Done.
"#;
        let report = GoalReport::parse(text, Path::new("C001.md")).unwrap();
        assert_eq!(report.cycle, "C001");
        assert_eq!(report.status, GoalReportStatus::ReadyForEvaluation);
        assert_eq!(report.next_hypothesis, "precompute weights");
        assert!(report.body.contains("Done."));
    }

    #[test]
    fn rejects_invalid_goal_report_status() {
        let text = r#"+++
cycle = "C001"
status = "done"
next_hypothesis = "next"
+++
"#;
        assert!(GoalReport::parse(text, Path::new("C001.md")).is_err());
    }

    #[test]
    fn parses_evaluator_json_and_preserves_extra_fields() {
        let result = EvaluatorResult::parse_json(
            r#"{
              "status": "pass",
              "recommendation": "keep",
              "metric": "p95_ms",
              "baseline": 120.0,
              "candidate": 113.6,
              "delta": 6.4,
              "reason": "improved",
              "sample_count": 12
            }"#,
        )
        .unwrap();
        assert_eq!(result.recommendation, EvaluatorRecommendation::Keep);
        assert_eq!(result.metric, "p95_ms");
        assert_eq!(result.raw["sample_count"], 12);
        assert!(EvaluatorResult::parse_json(r#"{"recommendation":"keep"}"#).is_err());
    }

    #[test]
    fn budget_decision_checks_iteration_and_duration_limits() {
        let contract = contract();
        let mut state = GoalState::initial(&contract);
        let now = Utc.with_ymd_and_hms(2026, 6, 17, 12, 30, 0).unwrap();
        assert!(!state.budget_decision(&contract, now).unwrap().exhausted);

        state.iterations_completed = 10;
        let decision = state.budget_decision(&contract, now).unwrap();
        assert!(decision.exhausted);
        assert_eq!(decision.reason.as_deref(), Some("max_iterations"));

        state.iterations_completed = 9;
        let late = Utc.with_ymd_and_hms(2026, 6, 17, 23, 0, 1).unwrap();
        let decision = state.budget_decision(&contract, late).unwrap();
        assert!(decision.exhausted);
        assert_eq!(decision.reason.as_deref(), Some("max_duration"));
    }

    #[test]
    fn init_request_preserves_initial_hypothesis_in_state() {
        let (contract, state) = GoalInitRequest::new(
            GoalId::explicit("search-ranking-proof").unwrap(),
            "Reduce search ranking p95",
            "just goal-eval",
            "p95_ms",
            GoalDirection::LowerIsBetter,
            5.0,
            10,
            "10h",
            "cache normalized query features",
            vec![],
            vec![],
        )
        .unwrap()
        .into_contract_and_state("2026-06-17T12:00:00Z".to_string());

        assert_eq!(contract.goal_id.as_str(), "search-ranking-proof");
        assert_eq!(state.next_hypothesis, "cache normalized query features");
    }

    #[test]
    fn increments_cycle_ids() {
        assert_eq!(first_cycle_id(), "C001");
        assert_eq!(next_cycle_id("C001").unwrap(), "C002");
        assert_eq!(next_cycle_id("C099").unwrap(), "C100");
        assert!(next_cycle_id("1").is_err());
    }

    #[test]
    fn writes_goal_contract_state_pointer_and_jsonl_traces() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".orchid")).unwrap();
        let contract = contract();
        contract.write(tmp.path()).unwrap();
        let state = GoalState::initial(&contract);
        state.write(tmp.path(), &contract.goal_id).unwrap();

        let goal_root = tmp.path().join(".orchid/goals/search-ranking-proof");
        assert!(goal_root.join(GOAL_TOML).exists());
        assert!(goal_root.join(STATE_JSON).exists());
        assert_eq!(
            std::fs::read_to_string(tmp.path().join(".orchid/goal-current")).unwrap(),
            "search-ranking-proof\n"
        );
        assert_eq!(
            GoalContract::read(tmp.path(), &contract.goal_id)
                .unwrap()
                .primary_metric,
            "p95_ms"
        );
        assert_eq!(
            GoalState::read(tmp.path(), &contract.goal_id)
                .unwrap()
                .status,
            GoalStatus::Baseline
        );

        let evaluator = EvaluatorResult::parse_json(
            r#"{"status":"pass","recommendation":"keep","metric":"p95_ms","baseline":120.0,"candidate":110.0,"delta":10.0,"reason":"better"}"#,
        )
        .unwrap();
        evaluator
            .append_measurement(tmp.path(), &contract.goal_id, "C001")
            .unwrap();
        append_result(
            tmp.path(),
            &contract.goal_id,
            json!({"cycle": "C001", "decision": "keep"})
                .as_object()
                .unwrap(),
        )
        .unwrap();
        assert!(std::fs::read_to_string(goal_root.join(MEASUREMENTS_JSONL))
            .unwrap()
            .contains("\"candidate\":110.0"));
        assert!(std::fs::read_to_string(goal_root.join(RESULTS_JSONL))
            .unwrap()
            .contains("\"decision\":\"keep\""));
    }

    #[test]
    fn report_paths_stay_under_goal_directory() {
        let tmp = TempDir::new().unwrap();
        std::fs::create_dir(tmp.path().join(".orchid")).unwrap();
        let id = GoalId::explicit("safe").unwrap();
        let path = report_path(tmp.path(), &id, "C001").unwrap();
        assert!(path.ends_with(".orchid/goals/safe/reports/C001.md"));
    }

    fn contract() -> GoalContract {
        GoalInitRequest::new(
            GoalId::explicit("search-ranking-proof").unwrap(),
            "Reduce search ranking p95",
            "just goal-eval",
            "p95_ms",
            GoalDirection::LowerIsBetter,
            5.0,
            10,
            "10h",
            "cache normalized query features",
            vec![PathBuf::from("justfile")],
            vec![],
        )
        .unwrap()
        .into_contract("2026-06-17T12:00:00Z".to_string())
    }
}
