//! CLI argument parsing and dispatch into orchestration command handlers.
//!
//! Subcommands emit compact JSON ACKs by default; goal-related commands may render
//! Markdown prompts for agent-facing workflows.

use std::path::{Path, PathBuf};

use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use serde_json::{Map, Value};

use crate::core::{emit, emit_markdown, json_fail, OrchResult, DEFAULT_STALE_AFTER};
use crate::gitstate;
use crate::goal::{self, GoalDirection, GoalId, GoalInitRequest};
use crate::model::LEASE_SCHEMA_VERSION;
use crate::orchestration::{
    self, AttachAgentRequest, BlockRequest, BudRequest, CleanupRequest, CloseRequest,
    CompleteRequest, LeaseRequest, NextRequest, PacketRequest, PacketRoleKind, ReportCheckRequest,
};
use crate::paths::root_from_arg;

#[derive(Parser)]
#[command(
    name = "orchid",
    about = "Coordinate scoped agent work from repo-local specs and bud leases",
    long_about = "Orchid coordinates scoped agent work from repo-local specs and runtime-only bud leases. It leases Markdown task files or ephemeral bud instructions, creates fresh role packets, validates worker reports, checks Git scope, and emits JSON ACKs for orchestrators."
)]
struct Cli {
    #[arg(long, help = "Repository root; defaults to the current directory")]
    root: Option<String>,
    #[arg(long, global = true, help = "Pretty-print JSON output")]
    pretty: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    #[command(about = "Advertise machine-readable protocol capabilities")]
    Capabilities,
    #[command(about = "List ready task files")]
    Ready(ReadyArgs),
    #[command(about = "Summarize specs, task states, active leases, or one agent lease")]
    Status(StatusArgs),
    #[command(about = "Reserve a task for one scoped worker")]
    Lease(LeaseArgs),
    #[command(about = "Create a runtime-only bud lease and worker packet")]
    Bud(BudArgs),
    #[command(name = "lease-attach-agent", about = "Attach an agent id to a lease")]
    LeaseAttachAgent(AttachAgentArgs),
    #[command(about = "List active lease runtime files")]
    Running,
    #[command(about = "Refresh a lease heartbeat timestamp")]
    Heartbeat {
        #[arg(help = "Lease id to heartbeat, for example l_123")]
        lease: String,
    },
    #[command(about = "Find leases with stale heartbeats")]
    Stale {
        #[arg(
            long,
            default_value = "30m",
            help = "Minimum lease age, such as 10m, 2h, or 1d"
        )]
        older_than: String,
    },
    #[command(about = "Release a lease without completing its task")]
    Release {
        #[arg(help = "Lease id to release")]
        lease: String,
        #[arg(long, default_value = "", help = "Reason recorded in the release ACK")]
        reason: String,
    },
    #[command(about = "Close lease runtime files after handoff")]
    Close(CloseArgs),
    #[command(about = "Remove completed or released runtime artifacts")]
    Cleanup(CleanupArgs),
    #[command(about = "Decide the next orchestration action")]
    Next(NextArgs),
    #[command(
        name = "research-path",
        about = "Print or create a spec research workspace"
    )]
    ResearchPath(ResearchPathArgs),
    #[command(name = "research-clean", about = "Delete a spec research workspace")]
    ResearchClean {
        #[arg(help = "Spec id or specs/<spec-id> path")]
        spec: String,
    },
    #[command(about = "Generate a worker, validator, reviewer, or loop-runner packet")]
    Packet(PacketArgs),
    #[command(
        name = "report-check",
        about = "Validate a worker report before completion"
    )]
    ReportCheck {
        #[arg(help = "Report path, usually .orchid/reports/<lease>.md")]
        report: String,
    },
    #[command(name = "git-status", about = "Return compact Git status")]
    GitStatus,
    #[command(
        name = "git-touched",
        about = "Compare Git changes against a lease scope"
    )]
    GitTouched {
        #[arg(long, help = "Lease id to inspect")]
        lease: String,
    },
    #[command(name = "git-stage-plan", about = "Plan safe Git pathspecs for a lease")]
    GitStagePlan {
        #[arg(long, help = "Lease id to plan staging for")]
        lease: String,
    },
    #[command(about = "Record verified work as complete")]
    Complete(CompleteArgs),
    #[command(about = "Mark a task blocked with a reason")]
    Block(BlockArgs),
    #[command(about = "Validate spec and task-file structure")]
    Lint,
    #[command(about = "Run a branch-local goal improvement loop")]
    Goal(GoalArgs),
}

#[derive(Args)]
struct GoalArgs {
    #[command(subcommand)]
    command: Option<GoalCommand>,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)]
enum GoalCommand {
    #[command(about = "Initialize a branch-local goal contract")]
    Init(GoalInitArgs),
    #[command(about = "Show current goal status")]
    Status,
    #[command(about = "Finish the current goal without creating a pull request")]
    Finish,
}

#[derive(Args)]
struct GoalInitArgs {
    #[arg(long, help = "Stable goal id; defaults from the current branch")]
    id: Option<String>,
    #[arg(long, help = "Goal statement for the improvement loop")]
    goal: String,
    #[arg(long, help = "Evaluator command")]
    evaluator: Option<String>,
    #[arg(long, help = "Primary metric name")]
    metric: String,
    #[arg(long, value_enum, help = "Metric direction")]
    direction: GoalDirectionArg,
    #[arg(long, help = "Minimum metric delta required to keep a cycle")]
    min_delta: f64,
    #[arg(long, help = "Initial hypothesis to try")]
    hypothesis: String,
    #[arg(long, help = "Maximum improvement cycles")]
    max_iterations: u32,
    #[arg(long, help = "Maximum duration, such as 30m, 2h, or 1d")]
    max_duration: String,
    #[arg(long, action = clap::ArgAction::Append, help = "Protected path that blocks automatic decisions; repeatable")]
    protected_surface: Vec<PathBuf>,
    #[arg(long, action = clap::ArgAction::Append, help = "Goal work scope path; repeatable")]
    scope: Vec<PathBuf>,
}

#[derive(Copy, Clone, ValueEnum)]
enum GoalDirectionArg {
    #[value(name = "lower-is-better")]
    LowerIsBetter,
    #[value(name = "higher-is-better")]
    HigherIsBetter,
    Target,
}

impl From<GoalDirectionArg> for GoalDirection {
    fn from(value: GoalDirectionArg) -> Self {
        match value {
            GoalDirectionArg::LowerIsBetter => GoalDirection::LowerIsBetter,
            GoalDirectionArg::HigherIsBetter => GoalDirection::HigherIsBetter,
            GoalDirectionArg::Target => GoalDirection::Target,
        }
    }
}

#[derive(Args)]
#[command(group = clap::ArgGroup::new("detail").args(["explain", "brief"]).multiple(false))]
struct ReadyArgs {
    #[arg(long, action = clap::ArgAction::Append, help = "Limit ready queue to a spec id; repeatable")]
    spec: Vec<String>,
    #[arg(long, help = "Select the first open active spec by numerical prefix")]
    all_open: bool,
    #[arg(
        long,
        hide = true,
        help = "Include blocked tasks and selection details; default behavior"
    )]
    explain: bool,
    #[arg(long, help = "Omit blocked tasks and selection details")]
    brief: bool,
}

#[derive(Args)]
struct StatusArgs {
    #[arg(long, action = clap::ArgAction::Append, help = "Limit status to a spec id; repeatable")]
    spec: Vec<String>,
    #[arg(long, help = "Find the lease attached to a discovery-only agent id")]
    agent_id: Option<String>,
    #[arg(
        long,
        help = "Show status for the first open active spec by numerical prefix"
    )]
    all_open: bool,
}

#[derive(Args)]
#[command(group = clap::ArgGroup::new("mode").args(["serial", "allow_parallel"]).multiple(false))]
struct LeaseArgs {
    #[arg(
        help = "Task target: SPEC with TASK_ID, SPEC/TASK, specs/SPEC with TASK_ID, or specs/SPEC/tasks/TASK.md"
    )]
    target: String,
    #[arg(help = "Task id when TARGET is a spec id or specs/SPEC path")]
    task_id: Option<String>,
    #[arg(long, help = "Lease owner label, such as worker:agent_123")]
    owner: String,
    #[arg(long, help = "Discovery-only runtime agent id attached to this lease")]
    agent_id: Option<String>,
    #[arg(
        long,
        help = "Override worker reasoning effort: low, medium, high, or xhigh"
    )]
    worker_reasoning_effort: Option<String>,
    #[arg(
        long,
        default_value = "",
        help = "Override worker model for this lease"
    )]
    worker_model: String,
    #[arg(long, help = "Override generated lease id for tests or recovery")]
    lease_id: Option<String>,
    #[arg(long, help = "Require no other active leases")]
    serial: bool,
    #[arg(long, help = "Allow a disjoint lease while other leases are active")]
    allow_parallel: bool,
}

#[derive(Args)]
#[command(group = clap::ArgGroup::new("mode").args(["serial", "allow_parallel"]).multiple(false))]
struct BudArgs {
    #[arg(long, help = "Short title for the bud delegation")]
    title: String,
    #[arg(long, action = clap::ArgAction::Append, help = "Allowed write-scope path; repeatable; required")]
    scope: Vec<String>,
    #[arg(long, help = "Markdown instruction file to snapshot into .orchid/buds")]
    instructions: String,
    #[arg(
        long,
        help = "Lease owner label; defaults from --agent-id or worker:unassigned"
    )]
    owner: Option<String>,
    #[arg(long, help = "Discovery-only runtime agent id attached to this lease")]
    agent_id: Option<String>,
    #[arg(long, help = "Worker reasoning effort: low, medium, high, or xhigh")]
    worker_reasoning_effort: Option<String>,
    #[arg(long, default_value = "", help = "Worker model override for this bud")]
    worker_model: String,
    #[arg(long, help = "Override generated lease id for tests or recovery")]
    lease_id: Option<String>,
    #[arg(long, help = "Require no other active leases")]
    serial: bool,
    #[arg(long, help = "Allow a disjoint lease while other leases are active")]
    allow_parallel: bool,
}

#[derive(Args)]
struct AttachAgentArgs {
    #[arg(long, help = "Lease id to update")]
    lease: String,
    #[arg(long, help = "Discovery-only runtime agent id to attach")]
    agent_id: String,
}

#[derive(Args)]
struct CloseArgs {
    #[arg(long, help = "Lease id to close")]
    lease: String,
    #[arg(
        long,
        help = "Close and delete runtime files even if the lease is active"
    )]
    force: bool,
}

#[derive(Args)]
struct CleanupArgs {
    #[arg(
        long,
        help = "Delete completed/released lease files, packets, and reports"
    )]
    completed: bool,
}

#[derive(Args)]
#[command(group = clap::ArgGroup::new("detail").args(["explain", "brief"]).multiple(false))]
struct NextArgs {
    #[arg(long, action = clap::ArgAction::Append, help = "Limit next action to a spec id; repeatable")]
    spec: Vec<String>,
    #[arg(long, help = "Select the first open active spec by numerical prefix")]
    all_open: bool,
    #[arg(long, default_value = DEFAULT_STALE_AFTER, help = "Minimum lease age for recover/stale decisions")]
    older_than: String,
    #[arg(
        long,
        hide = true,
        help = "Include recommended action, queues, and blockers; default behavior"
    )]
    explain: bool,
    #[arg(long, help = "Omit secondary queues and blockers")]
    brief: bool,
}

#[derive(Args)]
struct ResearchPathArgs {
    #[arg(help = "Spec id or specs/<spec-id> path")]
    spec: String,
    #[arg(long, help = "Create the workspace if missing")]
    create: bool,
}

#[derive(Copy, Clone, ValueEnum)]
enum PacketRole {
    Worker,
    Validator,
    Reviewer,
    #[value(name = "loop-runner")]
    LoopRunner,
}

impl From<PacketRole> for PacketRoleKind {
    fn from(value: PacketRole) -> Self {
        match value {
            PacketRole::Worker => PacketRoleKind::Worker,
            PacketRole::Validator => PacketRoleKind::Validator,
            PacketRole::Reviewer => PacketRoleKind::Reviewer,
            PacketRole::LoopRunner => PacketRoleKind::LoopRunner,
        }
    }
}

#[derive(Args)]
struct PacketArgs {
    #[arg(long, help = "Lease id to build a role packet for")]
    lease: String,
    #[arg(
        long,
        value_enum,
        default_value = "worker",
        help = "Packet role to generate"
    )]
    role: PacketRole,
}

#[derive(Args)]
struct CompleteArgs {
    #[arg(long, help = "Lease id to complete")]
    lease: String,
    #[arg(long, help = "Validator or coordinator label that verified the work")]
    verified_by: String,
    #[arg(long, default_value = "", help = "Worker label to record on the task")]
    implemented_by: String,
    #[arg(long, default_value = "passed", help = "Verification result to record")]
    verification_status: String,
    #[arg(
        long,
        default_value = "",
        help = "Report path or summary reference to record"
    )]
    report: String,
    #[arg(
        long,
        default_value = "",
        help = "Commit hash produced by the coordinator"
    )]
    commit: String,
    #[arg(
        long,
        default_value = "",
        help = "Independent review reference for the commit"
    )]
    commit_review: String,
    #[arg(long, help = "Delete .orchid/spec-research/<spec-id> after completion")]
    clean_spec_research: bool,
}

#[derive(Args)]
struct BlockArgs {
    #[arg(
        help = "Task target: SPEC with TASK_ID, SPEC/TASK, specs/SPEC with TASK_ID, or specs/SPEC/tasks/TASK.md"
    )]
    target: String,
    #[arg(help = "Task id when TARGET is a spec id or specs/SPEC path")]
    task_id: Option<String>,
    #[arg(long, help = "Reason to write to task state")]
    reason: String,
}

/// Parse CLI arguments, resolve the repository root, run the requested subcommand, and
/// print JSON or Markdown output. Returns a process exit code: `0` on success, `1` on failure.
pub fn run() -> i32 {
    let cli = Cli::parse();
    let root = match root_from_arg(cli.root.as_deref()) {
        Ok(root) => root,
        Err(error) => {
            let mut payload = json_fail(&error.message, Some(&error.code));
            finalize_json_ack(&mut payload, &cli.command);
            emit(&payload, cli.pretty);
            return 1;
        }
    };

    let result: OrchResult<CommandOutput> = match run_command(&root, &cli.command) {
        Ok(output) => Ok(output),
        Err(error) => {
            let mut payload = json_fail(&error.message, Some(&error.code));
            payload.extend(error.details);
            Ok(CommandOutput::Json(payload))
        }
    };

    match result {
        Ok(output) => match output {
            CommandOutput::Json(mut payload) => {
                let ok = finalize_json_ack(&mut payload, &cli.command);
                emit(&payload, cli.pretty);
                if ok {
                    0
                } else {
                    1
                }
            }
            CommandOutput::Markdown(markdown) => {
                emit_markdown(&markdown);
                0
            }
        },
        Err(error) => {
            let mut payload = json_fail(&error.message, Some(&error.code));
            payload.extend(error.details);
            finalize_json_ack(&mut payload, &cli.command);
            emit(&payload, cli.pretty);
            1
        }
    }
}

enum CommandOutput {
    Json(Map<String, Value>),
    Markdown(String),
}

impl From<Map<String, Value>> for CommandOutput {
    fn from(value: Map<String, Value>) -> Self {
        Self::Json(value)
    }
}

fn run_command(root: &Path, command: &Command) -> OrchResult<CommandOutput> {
    match command {
        Command::Capabilities => Ok(cmd_capabilities().into()),
        Command::Ready(args) => cmd_ready(root, args).map(Into::into),
        Command::Status(args) => cmd_status(root, args).map(Into::into),
        Command::Lease(args) => cmd_lease(root, args).map(Into::into),
        Command::Bud(args) => cmd_bud(root, args).map(Into::into),
        Command::LeaseAttachAgent(args) => cmd_lease_attach_agent(root, args).map(Into::into),
        Command::Running => cmd_running(root).map(Into::into),
        Command::Heartbeat { lease } => cmd_heartbeat(root, lease).map(Into::into),
        Command::Stale { older_than } => cmd_stale(root, older_than).map(Into::into),
        Command::Release { lease, reason } => cmd_release(root, lease, reason).map(Into::into),
        Command::Close(args) => cmd_close(root, args).map(Into::into),
        Command::Cleanup(args) => cmd_cleanup(root, args).map(Into::into),
        Command::Next(args) => cmd_next(root, args).map(Into::into),
        Command::ResearchPath(args) => cmd_research_path(root, args).map(Into::into),
        Command::ResearchClean { spec } => cmd_research_clean(root, spec).map(Into::into),
        Command::Packet(args) => cmd_packet(root, args).map(Into::into),
        Command::ReportCheck { report } => cmd_report_check(root, report).map(Into::into),
        Command::GitStatus => cmd_git_status(root).map(Into::into),
        Command::GitTouched { lease } => cmd_git_touched(root, lease).map(Into::into),
        Command::GitStagePlan { lease } => cmd_git_stage_plan(root, lease).map(Into::into),
        Command::Complete(args) => cmd_complete(root, args).map(Into::into),
        Command::Block(args) => cmd_block(root, args).map(Into::into),
        Command::Lint => cmd_lint(root).map(Into::into),
        Command::Goal(args) => cmd_goal(root, args).map(CommandOutput::Markdown),
    }
}

impl Command {
    fn name(&self) -> &'static str {
        match self {
            Self::Capabilities => "capabilities",
            Self::Ready(_) => "ready",
            Self::Status(_) => "status",
            Self::Lease(_) => "lease",
            Self::Bud(_) => "bud",
            Self::LeaseAttachAgent(_) => "lease-attach-agent",
            Self::Running => "running",
            Self::Heartbeat { .. } => "heartbeat",
            Self::Stale { .. } => "stale",
            Self::Release { .. } => "release",
            Self::Close(_) => "close",
            Self::Cleanup(_) => "cleanup",
            Self::Next(_) => "next",
            Self::ResearchPath(_) => "research-path",
            Self::ResearchClean { .. } => "research-clean",
            Self::Packet(_) => "packet",
            Self::ReportCheck { .. } => "report-check",
            Self::GitStatus => "git-status",
            Self::GitTouched { .. } => "git-touched",
            Self::GitStagePlan { .. } => "git-stage-plan",
            Self::Complete(_) => "complete",
            Self::Block(_) => "block",
            Self::Lint => "lint",
            Self::Goal(_) => "goal",
        }
    }
}

const ACK_VERSION: i64 = 1;
const ACTION_VERSION: i64 = 1;

fn finalize_json_ack(payload: &mut Map<String, Value>, command: &Command) -> bool {
    let ok = payload
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or_else(|| !payload.contains_key("error"));
    payload.insert("ack_version".to_string(), Value::from(ACK_VERSION));
    payload.insert("ok".to_string(), Value::Bool(ok));
    payload.insert(
        "command".to_string(),
        Value::String(command.name().to_string()),
    );
    if matches!(command, Command::Next(_)) {
        finalize_next_actions(payload);
    }
    ok
}

fn finalize_next_actions(payload: &mut Map<String, Value>) {
    let commands = payload
        .get("commands")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| payload.get("cmds").and_then(Value::as_array).cloned())
        .or_else(|| {
            payload
                .get("cmd")
                .and_then(Value::as_array)
                .cloned()
                .map(|command| vec![Value::Array(command)])
        })
        .unwrap_or_default();
    let actions = commands
        .iter()
        .map(|argv| {
            let mut action = Map::new();
            action.insert("type".to_string(), Value::String("command".to_string()));
            action.insert("argv".to_string(), argv.clone());
            Value::Object(action)
        })
        .collect();
    payload.insert("action_version".to_string(), Value::from(ACTION_VERSION));
    payload.insert("commands".to_string(), Value::Array(commands));
    payload.insert("actions".to_string(), Value::Array(actions));
    if let Some(phase) = payload.get("phase").and_then(Value::as_str) {
        payload.insert(
            "recommended_action".to_string(),
            Value::String(phase.to_string()),
        );
    }
}

fn cmd_capabilities() -> Map<String, Value> {
    let mut protocols = Map::new();
    protocols.insert("ack".to_string(), Value::from(ACK_VERSION));
    protocols.insert("actions".to_string(), Value::from(ACTION_VERSION));
    protocols.insert(
        "lease_schema".to_string(),
        Value::from(LEASE_SCHEMA_VERSION),
    );
    let command_names: Vec<String> = Cli::command()
        .get_subcommands()
        .map(|command| command.get_name().to_string())
        .collect();
    let mut payload = Map::new();
    payload.insert("protocols".to_string(), Value::Object(protocols));
    payload.insert(
        "json_commands".to_string(),
        owned_string_values(
            command_names
                .iter()
                .filter(|name| name.as_str() != "goal")
                .cloned(),
        ),
    );
    payload.insert(
        "markdown_commands".to_string(),
        owned_string_values(
            command_names
                .into_iter()
                .filter(|name| name.as_str() == "goal"),
        ),
    );
    payload.insert(
        "features".to_string(),
        string_values(&[
            "typed_blocker_codes",
            "read_only_agent_status",
            "released_lease_attribution",
            "spec_scoped_next",
        ]),
    );
    payload
}

fn string_values(items: &[&str]) -> Value {
    Value::Array(
        items
            .iter()
            .map(|item| Value::String((*item).to_string()))
            .collect(),
    )
}

fn owned_string_values(items: impl IntoIterator<Item = String>) -> Value {
    Value::Array(items.into_iter().map(Value::String).collect())
}

fn cmd_goal(root: &Path, args: &GoalArgs) -> OrchResult<String> {
    match &args.command {
        Some(GoalCommand::Init(args)) => cmd_goal_init(root, args),
        Some(GoalCommand::Status) => cmd_goal_status(root),
        Some(GoalCommand::Finish) => cmd_goal_finish(root),
        None => cmd_goal_current(root),
    }
}

fn cmd_goal_init(root: &Path, args: &GoalInitArgs) -> OrchResult<String> {
    let goal_id = match args.id.as_deref() {
        Some(raw) => GoalId::explicit(raw)?,
        None => GoalId::sanitize(
            &gitstate::current_branch(root)?.unwrap_or_else(|| "goal".to_string()),
        )?,
    };
    let request = GoalInitRequest::new(
        goal_id,
        args.goal.clone(),
        args.evaluator
            .clone()
            .unwrap_or_else(|| "just goal-eval".to_string()),
        args.metric.clone(),
        args.direction.into(),
        args.min_delta,
        args.max_iterations,
        args.max_duration.clone(),
        args.hypothesis.clone(),
        args.protected_surface.clone(),
        args.scope.clone(),
    )?;
    goal::init_goal(root, request)
}

fn cmd_goal_current(root: &Path) -> OrchResult<String> {
    match goal::current_goal(root)? {
        Some((contract, state)) => goal::render_goal_prompt_and_advance(root, &contract, &state),
        None => Ok(goal::render_no_goal_prompt()),
    }
}

fn cmd_goal_status(root: &Path) -> OrchResult<String> {
    match goal::current_goal(root)? {
        Some((contract, state)) => goal::render_goal_status(root, &contract, &state),
        None => Ok("# Goal Status\n\nNo current goal is initialized.\n".to_string()),
    }
}

fn cmd_goal_finish(root: &Path) -> OrchResult<String> {
    match goal::current_goal(root)? {
        Some((contract, state)) => goal::finish_goal(root, &contract, &state),
        None => Ok("# Goal Finish\n\nNo current goal is initialized.\n".to_string()),
    }
}

fn cmd_ready(root: &Path, args: &ReadyArgs) -> OrchResult<Map<String, Value>> {
    orchestration::ready(
        root,
        &orchestration::ReadyRequest {
            specs: args.spec.clone(),
            all_open: args.all_open,
            explain: args.explain || !args.brief,
        },
    )
}

fn cmd_status(root: &Path, args: &StatusArgs) -> OrchResult<Map<String, Value>> {
    orchestration::status(
        root,
        &orchestration::StatusRequest {
            specs: args.spec.clone(),
            agent_id: args.agent_id.clone(),
            all_open: args.all_open,
        },
    )
}

fn cmd_lease(root: &Path, args: &LeaseArgs) -> OrchResult<Map<String, Value>> {
    orchestration::lease(
        root,
        &LeaseRequest {
            target: args.target.clone(),
            task_id: args.task_id.clone(),
            owner: args.owner.clone(),
            agent_id: args.agent_id.clone(),
            worker_reasoning_effort: args.worker_reasoning_effort.clone(),
            worker_model: args.worker_model.clone(),
            lease_id: args.lease_id.clone(),
            serial: args.serial,
            allow_parallel: args.allow_parallel,
        },
    )
}

fn cmd_bud(root: &Path, args: &BudArgs) -> OrchResult<Map<String, Value>> {
    orchestration::bud(
        root,
        &BudRequest {
            title: args.title.clone(),
            scope: args.scope.clone(),
            instructions: args.instructions.clone(),
            owner: args.owner.clone(),
            agent_id: args.agent_id.clone(),
            worker_reasoning_effort: args.worker_reasoning_effort.clone(),
            worker_model: args.worker_model.clone(),
            lease_id: args.lease_id.clone(),
            serial: args.serial,
            allow_parallel: args.allow_parallel,
        },
    )
}

fn cmd_lease_attach_agent(root: &Path, args: &AttachAgentArgs) -> OrchResult<Map<String, Value>> {
    orchestration::lease_attach_agent(
        root,
        &AttachAgentRequest {
            lease: args.lease.clone(),
            agent_id: args.agent_id.clone(),
        },
    )
}

fn cmd_running(root: &Path) -> OrchResult<Map<String, Value>> {
    orchestration::running(root)
}

fn cmd_heartbeat(root: &Path, lease_id: &str) -> OrchResult<Map<String, Value>> {
    orchestration::heartbeat(root, lease_id)
}

fn cmd_stale(root: &Path, older_than: &str) -> OrchResult<Map<String, Value>> {
    orchestration::stale(root, older_than)
}

fn cmd_release(root: &Path, lease_id: &str, reason: &str) -> OrchResult<Map<String, Value>> {
    orchestration::release(root, lease_id, reason)
}

fn cmd_close(root: &Path, args: &CloseArgs) -> OrchResult<Map<String, Value>> {
    orchestration::close(
        root,
        &CloseRequest {
            lease: args.lease.clone(),
            force: args.force,
        },
    )
}

fn cmd_cleanup(root: &Path, args: &CleanupArgs) -> OrchResult<Map<String, Value>> {
    orchestration::cleanup(
        root,
        &CleanupRequest {
            completed: args.completed,
        },
    )
}

fn cmd_research_path(root: &Path, args: &ResearchPathArgs) -> OrchResult<Map<String, Value>> {
    orchestration::research_path(
        root,
        &orchestration::ResearchPathRequest {
            spec: args.spec.clone(),
            create: args.create,
        },
    )
}

fn cmd_research_clean(root: &Path, spec: &str) -> OrchResult<Map<String, Value>> {
    orchestration::research_clean(root, spec)
}

fn cmd_next(root: &Path, args: &NextArgs) -> OrchResult<Map<String, Value>> {
    orchestration::next(
        root,
        &NextRequest {
            specs: args.spec.clone(),
            all_open: args.all_open,
            older_than: args.older_than.clone(),
            explain: args.explain || !args.brief,
        },
    )
}

fn cmd_packet(root: &Path, args: &PacketArgs) -> OrchResult<Map<String, Value>> {
    orchestration::packet(
        root,
        &PacketRequest {
            lease: args.lease.clone(),
            role: args.role.into(),
        },
    )
}

fn cmd_report_check(root: &Path, report: &str) -> OrchResult<Map<String, Value>> {
    orchestration::report_check(
        root,
        &ReportCheckRequest {
            report: report.to_string(),
        },
    )
}

fn cmd_git_status(root: &Path) -> OrchResult<Map<String, Value>> {
    orchestration::git_status(root)
}

fn cmd_git_touched(root: &Path, lease_id: &str) -> OrchResult<Map<String, Value>> {
    orchestration::git_touched(root, lease_id)
}

fn cmd_git_stage_plan(root: &Path, lease_id: &str) -> OrchResult<Map<String, Value>> {
    orchestration::git_stage_plan(root, lease_id)
}

fn cmd_complete(root: &Path, args: &CompleteArgs) -> OrchResult<Map<String, Value>> {
    orchestration::complete(
        root,
        &CompleteRequest {
            lease: args.lease.clone(),
            verified_by: args.verified_by.clone(),
            implemented_by: args.implemented_by.clone(),
            verification_status: args.verification_status.clone(),
            report: args.report.clone(),
            commit: args.commit.clone(),
            commit_review: args.commit_review.clone(),
            clean_spec_research: args.clean_spec_research,
        },
    )
}

fn cmd_block(root: &Path, args: &BlockArgs) -> OrchResult<Map<String, Value>> {
    orchestration::block(
        root,
        &BlockRequest {
            target: args.target.clone(),
            task_id: args.task_id.clone(),
            reason: args.reason.clone(),
        },
    )
}

fn cmd_lint(root: &Path) -> OrchResult<Map<String, Value>> {
    orchestration::lint(root)
}
