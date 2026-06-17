use std::path::Path;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Map, Value};

use crate::core::{emit, json_fail, OrchResult, DEFAULT_STALE_AFTER};
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

pub fn run() -> i32 {
    let cli = Cli::parse();
    let root = match root_from_arg(cli.root.as_deref()) {
        Ok(root) => root,
        Err(error) => {
            let payload = json_fail(&error.message, Some(&error.code));
            emit(&payload, cli.pretty);
            return 1;
        }
    };

    let result: OrchResult<Map<String, Value>> = match run_command(&root, &cli.command) {
        Ok(payload) => Ok(payload),
        Err(error) => {
            let mut payload = json_fail(&error.message, Some(&error.code));
            payload.extend(error.details);
            Ok(payload)
        }
    };

    match result {
        Ok(payload) => {
            let ok = payload
                .get("ok")
                .and_then(Value::as_bool)
                .unwrap_or_else(|| !payload.contains_key("error"));
            emit(&payload, cli.pretty);
            if ok {
                0
            } else {
                1
            }
        }
        Err(error) => {
            let mut payload = json_fail(&error.message, Some(&error.code));
            payload.extend(error.details);
            emit(&payload, cli.pretty);
            1
        }
    }
}

fn run_command(root: &Path, command: &Command) -> OrchResult<Map<String, Value>> {
    match command {
        Command::Ready(args) => cmd_ready(root, args),
        Command::Status(args) => cmd_status(root, args),
        Command::Lease(args) => cmd_lease(root, args),
        Command::Bud(args) => cmd_bud(root, args),
        Command::LeaseAttachAgent(args) => cmd_lease_attach_agent(root, args),
        Command::Running => cmd_running(root),
        Command::Heartbeat { lease } => cmd_heartbeat(root, lease),
        Command::Stale { older_than } => cmd_stale(root, older_than),
        Command::Release { lease, reason } => cmd_release(root, lease, reason),
        Command::Close(args) => cmd_close(root, args),
        Command::Cleanup(args) => cmd_cleanup(root, args),
        Command::Next(args) => cmd_next(root, args),
        Command::ResearchPath(args) => cmd_research_path(root, args),
        Command::ResearchClean { spec } => cmd_research_clean(root, spec),
        Command::Packet(args) => cmd_packet(root, args),
        Command::ReportCheck { report } => cmd_report_check(root, report),
        Command::GitStatus => cmd_git_status(root),
        Command::GitTouched { lease } => cmd_git_touched(root, lease),
        Command::GitStagePlan { lease } => cmd_git_stage_plan(root, lease),
        Command::Complete(args) => cmd_complete(root, args),
        Command::Block(args) => cmd_block(root, args),
        Command::Lint => cmd_lint(root),
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
