use std::path::Path;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde_json::{Map, Value};

use crate::core::{emit, json_fail, OrchResult, DEFAULT_STALE_AFTER};
use crate::orchestration::{
    self, BlockRequest, CleanupRequest, CloseRequest, CompleteRequest, LeaseRequest, NextRequest,
    PacketRequest, PacketRoleKind, ReportCheckRequest,
};
use crate::paths::root_from_arg;

#[derive(Parser)]
#[command(name = "orchid", about = "Task-file orchestration helper")]
struct Cli {
    #[arg(long, help = "Repo root; defaults to cwd")]
    root: Option<String>,
    #[arg(long, help = "Pretty-print JSON")]
    pretty: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Ready(ReadyArgs),
    Status(StatusArgs),
    Lease(LeaseArgs),
    Running,
    Heartbeat {
        lease: String,
    },
    Stale {
        #[arg(long, default_value = "30m")]
        older_than: String,
    },
    Release {
        lease: String,
        #[arg(long, default_value = "")]
        reason: String,
    },
    Close(CloseArgs),
    Cleanup(CleanupArgs),
    Next(NextArgs),
    #[command(name = "research-path")]
    ResearchPath(ResearchPathArgs),
    #[command(name = "research-clean")]
    ResearchClean {
        spec: String,
    },
    Packet(PacketArgs),
    #[command(name = "report-check")]
    ReportCheck {
        report: String,
    },
    #[command(name = "git-status")]
    GitStatus,
    #[command(name = "git-touched")]
    GitTouched {
        #[arg(long)]
        lease: String,
    },
    #[command(name = "git-stage-plan")]
    GitStagePlan {
        #[arg(long)]
        lease: String,
    },
    Complete(CompleteArgs),
    Block(BlockArgs),
    Lint,
}

#[derive(Args)]
struct ReadyArgs {
    #[arg(long, action = clap::ArgAction::Append, help = "Restrict ready queue to this spec; repeatable")]
    spec: Vec<String>,
    #[arg(
        long,
        help = "Select only the first open active spec by numerical prefix"
    )]
    all_open: bool,
    #[arg(long)]
    explain: bool,
}

#[derive(Args)]
struct StatusArgs {
    #[arg(long, action = clap::ArgAction::Append, help = "Restrict status to this spec; repeatable")]
    spec: Vec<String>,
    #[arg(
        long,
        help = "Show status for the first open active spec by numerical prefix"
    )]
    all_open: bool,
}

#[derive(Args)]
#[command(group = clap::ArgGroup::new("mode").args(["serial", "allow_parallel"]).multiple(false))]
struct LeaseArgs {
    target: String,
    task_id: Option<String>,
    #[arg(long)]
    owner: String,
    #[arg(long)]
    lease_id: Option<String>,
    #[arg(long, help = "Reject this lease if any other active lease exists")]
    serial: bool,
    #[arg(
        long,
        help = "Intentionally allow a disjoint lease while other leases are active"
    )]
    allow_parallel: bool,
}

#[derive(Args)]
struct CloseArgs {
    #[arg(long)]
    lease: String,
    #[arg(long, help = "Close and delete files for an active lease")]
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
struct NextArgs {
    #[arg(long, action = clap::ArgAction::Append, help = "Restrict next action to this spec; repeatable")]
    spec: Vec<String>,
    #[arg(
        long,
        help = "Select only the first open active spec by numerical prefix"
    )]
    all_open: bool,
    #[arg(long, default_value = DEFAULT_STALE_AFTER)]
    older_than: String,
    #[arg(long)]
    explain: bool,
}

#[derive(Args)]
struct ResearchPathArgs {
    spec: String,
    #[arg(long, help = "Create .orchid/spec-research/<spec-id>")]
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
    #[arg(long)]
    lease: String,
    #[arg(long, value_enum, default_value = "worker")]
    role: PacketRole,
}

#[derive(Args)]
struct CompleteArgs {
    #[arg(long)]
    lease: String,
    #[arg(long)]
    verified_by: String,
    #[arg(long, default_value = "")]
    implemented_by: String,
    #[arg(long, default_value = "passed")]
    verification_status: String,
    #[arg(long, default_value = "")]
    report: String,
    #[arg(long, default_value = "")]
    commit: String,
    #[arg(long, default_value = "")]
    commit_review: String,
    #[arg(long)]
    clean_spec_research: bool,
}

#[derive(Args)]
struct BlockArgs {
    target: String,
    task_id: Option<String>,
    #[arg(long)]
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
            let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(true);
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
            explain: args.explain,
        },
    )
}

fn cmd_status(root: &Path, args: &StatusArgs) -> OrchResult<Map<String, Value>> {
    orchestration::status(
        root,
        &orchestration::StatusRequest {
            specs: args.spec.clone(),
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
            lease_id: args.lease_id.clone(),
            serial: args.serial,
            allow_parallel: args.allow_parallel,
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
            explain: args.explain,
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
