# Orchid

[![Crates.io Version](https://img.shields.io/crates/v/orchid-cli)](https://crates.io/crates/orchid-cli)
[![Crates.io Downloads](https://img.shields.io/crates/d/orchid-cli)](https://crates.io/crates/orchid-cli)
[![License](https://img.shields.io/crates/l/orchid-cli)](https://crates.io/crates/orchid-cli)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange)](https://www.rust-lang.org)

Orchid is a Rust CLI for coordinating scoped coding-agent work in a shared Git
repository. It turns Markdown task files into exclusive write-scope leases,
generates role-specific handoff packets, validates worker report contracts and
Git attribution, and tells a coordinator which phase comes next.

Durable work definitions live in `specs/`. Disposable leases, packets, reports,
research notes, and goal-loop state live under `.orchid/`. Most commands emit
JSON acknowledgements (ACKs), so agent harnesses can follow structured output
instead of scraping prose or Git porcelain.

Use Orchid when you want to:

- Find the next ready task in a Markdown spec.
- Lease one task or one runtime-only bud to an agent.
- Generate worker, validator, reviewer, or loop-runner packets.
- Validate worker reports and compare touched files against the lease scope.
- Stage only the paths that belong to a completed lease.
- Recover or release workers whose lease heartbeats became stale.
- Keep exploratory research separate from durable specifications.
- Run a branch-local metric improvement loop with `orchid goal`.

Orchid is intentionally local and inspectable: one CLI, Markdown plus TOML
planning files, JSON runtime records, no daemon, no database, and no built-in
pull request workflow. Your coordinator still owns agent creation, project
validation, spec/bud commits, reviews, and final handoff. Orchid creates commits
only when a goal-loop cycle is kept.

## Install

Use a release archive, the npm wrapper, or the Docker image when you only need
the binary. Use Cargo when Rust is already part of your toolchain.

### GitHub Releases

Download an archive from
[GitHub Releases](https://github.com/bnomei/orchid/releases), extract it, and
put the `orchid` binary on your `PATH`.

Release archives target x86-64 and arm64 musl Linux, x86-64 and Apple Silicon
macOS, and x86-64 Windows with MSVC.

### Crates.io

```sh
cargo install orchid-cli
```

### npm wrapper

Requires Node.js 18 or newer. The wrapper supports Linux and macOS on x64 or
arm64, plus Windows on x64. Linux and macOS also need `tar` to extract the
downloaded release archive.

```sh
npx @bnomei/orchid --version
```

The npm package is a thin wrapper. On first run it downloads the matching
GitHub Release binary, verifies its published `.sha256` checksum, caches it
locally, and forwards arguments to Orchid.

### Docker

The container image supports Linux x86-64 and arm64 and runs the published musl
release binary as a non-root user.

```sh
docker run --rm ghcr.io/bnomei/orchid:0.7.1 --version
```

Mount a repository for normal Orchid work. On Unix hosts, pass your user ID so
the container can write its `.orchid/` runtime directory in the bind mount:

```sh
docker run --rm --user "$(id -u):$(id -g)" \
  -v "$PWD:/workspace" -w /workspace \
  ghcr.io/bnomei/orchid:0.7.1 status
```

### From source

```sh
git clone https://github.com/bnomei/orchid.git
cd orchid
cargo build --release
```

Verify the install:

```sh
orchid --help
orchid --version
```

Expected output starts with:

```txt
Orchid coordinates scoped agent work from repo-local specs and runtime-only bud leases.
```

## Repository setup

Keep runtime files out of version control:

```gitignore
.orchid/
```

Commit durable planning files such as `specs/`, copied `skills/`, and any
project-specific validation scripts.

Orchid resolves the repository root in this order:

1. The path passed with `--root`.
2. The nearest ancestor containing `.orchid/`.
3. The Git worktree root.
4. The current directory.

This lets you run Orchid from a subdirectory after the repository has runtime
state. Pass `--root <PATH>` when a harness should not depend on discovery.

## Quickstart

This quickstart creates one dispatchable task, leases it, and generates a worker
packet. Run it from a Git repository where `orchid` is on `PATH`. The repository
must have at least one commit for full Git attribution.

1. Create a minimal task file:

   ```sh
   mkdir -p specs/example/tasks src/feature
   cat > specs/example/tasks/T001.md <<'EOF'
   +++
   id = "T001"
   title = "Implement first slice"
   status = "todo"
   scope = ["src/feature/"]
   depends = []
   covers = []
   verification_mode = "validator"
   verification_status = "pending"
   worker_reasoning_effort = "medium"
   worker_model = ""
   +++

   ## Context

   Implement the first slice.

   ## DoD

   Create or update code under `src/feature/`.

   ## Validation

   Run the focused project check for this slice.
   EOF
   ```

2. Check that Orchid can dispatch the task:

   ```sh
   orchid lint
   orchid ready --spec example
   ```

   Expected `ready` output includes:

   ```json
   {
     "ready": [
       {
         "task": "example/T001",
         "scope": ["src/feature/"],
         "verify": "validator",
         "worker_reasoning_effort": "medium"
       }
     ]
   }
   ```

3. Lease the task and create a worker packet:

   ```sh
   orchid lease example T001 --owner worker:agent_123 --agent-id agent_123 --serial
   orchid packet --lease <LEASE_ID> --role worker
   ```

   Use the `lease_id` returned by the first command. The packet command writes a
   Markdown packet under `.orchid/packets/` and returns its path as JSON.

4. After the worker writes the report path returned by `lease` or `packet`,
   validate and complete the lease:

   ```sh
   orchid report-check .orchid/reports/<LEASE_ID>.md
   orchid git-touched --lease <LEASE_ID>
   orchid complete --lease <LEASE_ID> --verified-by validator:agent_456
   orchid git-stage-plan --lease <LEASE_ID>
   ```

   Stage only the pathspecs returned by `git-stage-plan`, then create the
   repository's commit and review record.

5. After the handoff is durable, remove the lease runtime files:

   ```sh
   orchid close --lease <LEASE_ID>
   ```

## How Orchid works

The coordinator loop is:

```text
observe -> select -> lease -> dispatch -> report -> validate
        -> complete -> stage -> commit/review -> close
```

`orchid next` observes task state, leases, reports, Git state, and stale
heartbeats, then returns one phase plus the next command or commands. Its
decision order prevents new dispatch from skipping recovery, validation, or
safe staging work:

| Phase | Coordinator action |
| --- | --- |
| `recover` | Inspect a stale lease before dispatching more work. |
| `repair` | Regenerate the worker packet from the failed validator report. |
| `validate` | Check a worker report and compare touched paths with the lease. |
| `stage` | Request the safe pathspec plan for completed work. |
| `dispatch` | Lease the returned ready task; parallel dispatch includes `--allow-parallel`. |
| `wait` | Leave active work alone and observe again later. |
| `cleanup` | Close terminal lease artifacts after durable handoff. |
| `blocked` | Resolve the reported task or policy blocker. |
| `done` | Stop; no dispatchable work remains in the selected queue. |

The task file is the durable work record. A lease is a runtime snapshot of the
task scope, worker settings, spec policy, and Git baseline. Editing a task or
`spec.toml` after leasing does not retroactively weaken the active lease's
conflict and attribution guards.

Unreadable or identity-mismatched lease files fail closed for lease-sensitive
orchestration. Inspection ACKs surface recovery details instead of silently
ignoring the damaged record.

## Main workflows

Choose the workflow by the durability and decision model you need:

| Workflow | Use it for | Work definition | Git behavior |
| --- | --- | --- | --- |
| Spec task | Planned work with dependencies, validation, and review gates | `specs/<id>/tasks/*.md` | Coordinator stages and commits returned pathspecs. |
| Bud | One scoped delegation without a task file | Instruction snapshot under `.orchid/buds/` | Coordinator stages and commits returned pathspecs. |
| Goal loop | Repeated measurable experiments on a dedicated branch | Goal contract under `.orchid/goals/` | Orchid commits keeps and hard-resets discards. |

### Spec task orchestration

Use spec tasks for durable work that should be split, leased, validated, and
completed over time.

```sh
orchid next --spec example
orchid ready --spec example
orchid status --spec example
orchid lease example T001 --owner worker:agent_123 --agent-id agent_123 --serial
orchid packet --lease <LEASE_ID> --role worker
orchid report-check .orchid/reports/<LEASE_ID>.md
orchid git-touched --lease <LEASE_ID>
orchid complete --lease <LEASE_ID> --verified-by validator:agent_456
orchid git-stage-plan --lease <LEASE_ID>
```

Stage and commit only the returned pathspecs. After commit and review state is
durable, clean up the runtime artifacts:

```sh
orchid close --lease <LEASE_ID>
orchid cleanup --completed
```

`next` returns the recommended phase and one `argv` action. Use `--explain`
only when the mayor needs queue candidates or attribution diagnostics.

Use repeatable `--spec` flags to select explicit specs. Use `--all-open` to
select the first active spec with unfinished work by numerical prefix; it does
not select every open spec or broaden the user's authorized scope.

### Parallel work and recovery

Orchid defaults to one active lease unless the coordinator chooses a mode:

- `--serial` requires that no other lease is active.
- `--allow-parallel` permits another active lease only when effective write
  scopes do not overlap and the spec does not set `fanout_policy = "serial"`.

Scope disjointness only protects paths. The coordinator must still account for
shared services, generated state, migrations, fixtures, and other behavioral
conflicts before enabling parallel work.

Attach a runtime agent ID after dispatch when it was not known at lease time:

```sh
orchid lease-attach-agent --lease <LEASE_ID> --agent-id agent_123
orchid status --agent-id agent_123
```

Agent IDs are discovery metadata; operational commands remain lease-based.
Workers can refresh liveness with `orchid heartbeat <LEASE_ID>`. Coordinators
can inspect stale work and release an abandoned lease:

```sh
orchid stale --older-than 30m
orchid release <LEASE_ID> --reason "worker exited before handoff"
```

`orchid next --older-than <DURATION>` uses the same duration syntax (`10m`,
`2h`, or `1d`). A stale lease with a finished report is routed to validation;
a generated draft is still routed to recovery.

### Bud delegation

Use a bud for one scoped, runtime-only delegation when you do not want to create
durable `specs/` files.

```sh
orchid bud \
  --title "Diagnose research runner failure" \
  --scope src/research \
  --instructions /tmp/bud.md \
  --worker-reasoning-effort medium \
  --agent-id agent_123 \
  --serial
```

`bud` snapshots the instruction file under `.orchid/buds/`, creates a lease,
generates a worker packet and draft report stub, and returns both paths.

### Spec research workspaces

Use a research workspace for raw investigation that should inform a spec but
should not become durable requirements yet:

```sh
orchid research-path 001-example --create
orchid research-clean 001-example
```

The first command creates `.orchid/spec-research/<resolved-spec-id>/`. The
second deletes that workspace. `orchid complete --clean-spec-research` can
remove the matching workspace when the task completes.

### Goal loop

Use `orchid goal` for one branch-local improvement target. A trusted evaluator
reports measurements and a recommendation; Orchid applies metric, protected
surface, scope, and budget policy before it keeps, discards, blocks, or finishes
the cycle.

```sh
orchid goal init \
  --goal "Reduce search ranking p95 without changing correctness" \
  --metric p95_ms \
  --direction lower-is-better \
  --min-delta 5 \
  --hypothesis "cache normalized query features before ranking" \
  --max-iterations 10 \
  --max-duration 10h \
  --scope src/search \
  --protected-surface benches/ranking.rs
```

The default evaluator is `just goal-eval`. Override it with `--evaluator` when
your repository uses a different command. The evaluator must run from the repo
root and print one JSON object with `status`, `recommendation`, `metric`,
`baseline`, `candidate`, `delta`, and `reason`.

Orchid passes `ORCHID_GOAL_ID`, `ORCHID_GOAL_DIR`, `ORCHID_GOAL_CYCLE`,
`ORCHID_GOAL_BASELINE_COMMIT`, and `ORCHID_GOAL_BASELINE_VALUE` to the evaluator.
Candidate evaluations also receive `ORCHID_GOAL_DIRECTION` and
`ORCHID_GOAL_MIN_DELTA`.

At least one `--scope` is required. Repeat it to define every path Orchid may
stage for an automatic keep. Orchid rejects a keep when paths are already staged
outside that scope. Repeat `--protected-surface` for evaluator, fixture, policy,
or correctness-sensitive paths. Existing protected changes block evaluation;
changes introduced while the evaluator runs are checked again before a keep.

Evaluator recommendations are `keep`, `discard`, `blocked`, or `done`. A
non-`pass` evaluator status becomes `blocked`, and an improvement smaller than
`--min-delta` becomes `discard` for higher- or lower-is-better metrics.

Run the loop:

```sh
orchid goal
orchid goal status
orchid goal finish
```

`orchid goal` prints Markdown prompts instead of JSON. When a cycle is ready,
it asks for a report under `.orchid/goals/<goal-id>/reports/C###.md` with TOML
frontmatter:

```md
+++
cycle = "C001"
status = "ready_for_evaluation"
next_hypothesis = "next idea to try"
+++

## Summary

What changed and what evidence was collected.
```

For a keep, Orchid stages in-scope candidate changes and commits them as
`goal(<goal-id>): keep <cycle>`. A discard runs `git reset --hard` to the cycle
baseline and `git clean -fd -e .orchid/` from the repository root.

> **Warning:** Run goal loops only on a dedicated branch with no unrelated
> tracked edits or untracked non-ignored files. A discard can remove changes
> outside the declared goal scope. `.orchid/` is preserved.

## Spec layout

A spec is a directory under `specs/`. Task Markdown files are the dispatch unit.
Requirements, design, and policy files are optional context, but the bundled
skills create them for stable work.

```text
specs/001-example/
  requirements.md
  design.md
  spec.toml
  tasks/
    T001.md
    T002.md
```

Inactive spec directories are ignored by dispatch commands when their name is
`DRAFT`, `TBD`, `MANUAL`, `DONE`, starts with `DRAFT-`, `TBD-`, `MANUAL-`, or
`DONE-`, or starts with `_`.

### Task frontmatter

Task files use TOML frontmatter followed by Markdown context:

```md
+++
id = "T001"
title = "Implement first slice"
status = "todo"
scope = ["src/feature/"]
depends = []
covers = ["R001"]
verification_mode = "validator"
verification_status = "pending"
worker_reasoning_effort = "medium"
worker_model = ""
+++

## Context

Worker-facing context.

## DoD

Observable completion criteria.

## Validation

Focused command or check for this slice.
```

Important fields:

| Field | Contract |
| --- | --- |
| `id` | Defaults to the filename stem when omitted. |
| `status` | Must be `todo`, `pending_validation`, `pending_review`, `blocked`, or `done`. |
| `scope` | Required. Used for active lease conflicts, touched-file checks, and staging plans. |
| `depends` | Optional task references. A task is blocked until dependencies are `done`. |
| `verification_mode` | Must be `mayor`, `required`, or `validator`. |
| `worker_reasoning_effort` | `low`, `medium`, `high`, or `xhigh`. Missing legacy values default to `medium`. |
| `worker_model` | Optional model override. Leave blank unless a task requires one. |

`orchid complete` writes completion metadata back to the task frontmatter.
`orchid block` writes the blocked state and reason.

Dependencies can use a task ID from the same spec (`T001`), a cross-spec
reference (`002-other/T001` or `specs/002-other/T001`), or `-` as a placeholder.
Only a dependency with `status = "done"` is satisfied.

Task routing metadata does not spawn an agent. `worker_reasoning_effort` and the
opaque `worker_model` string flow through `ready`, `next`, and the lease so the
coordinator can choose the worker. They stay out of worker packets and reports;
explicit `lease` or `bud` flags override task metadata.

### Spec policy

`spec.toml` is optional. Orchid directly enforces these policy keys:

| Key | Effect |
| --- | --- |
| `execution_policy = "manual"` | Blocks dispatch for the spec. |
| `human_checkpoint = "before-implementation"` | Blocks dispatch until the policy changes. |
| `fanout_policy = "serial"` | Rejects `--allow-parallel` leases for the spec. |

Other policy keys are preserved in the lease record for coordinator inspection.

### Role report contracts

Generated packets create the canonical report stub with its role-specific
frontmatter template. Worker reports use `.orchid/reports/<LEASE_ID>.md`.
Validator, reviewer, and loop-runner reports use the same name with their role
suffix, for example `.orchid/reports/<LEASE_ID>-validator.md`.

Every generated template binds its report to the lease instance and frozen
context revision:

```md
+++
lease_id = "<LEASE_ID>"
lease_started_at = "<LEASE_START_TIME>"
context_revision = "<CONTEXT_REVISION>"
kind = "worker"
status = "ready_for_validation"
draft = true
commands_run = []
result = ""
+++

## Summary

What changed.

## Evidence

What proves it.

## Notes

Anything the coordinator must know.
```

Workers replace the stub with their evidence and set `draft = false` before the
report is ready. `report-check` rejects drafts, so they cannot accidentally
advance validation.

Valid statuses are `ready_for_validation`, `needs_fix`, `blocked`, and `done`.
Validator reports also set `verdict = "passed"`, `"failed"`, or `"blocked"`;
their status must respectively be `done`, `needs_fix`, or `blocked`.

Non-worker packets can include a canonical worker report with
`--source-report`. Orchid records that explicit source so later packet refreshes
keep the same handoff. `orchid report-check` validates the role, lease instance,
and canonical report path, and verifies a supplied context revision. It warns
when a legacy local report omits its lease-instance or context-revision binding.
It returns evidence diagnostics; it does not replace project-specific validation
or authorize task completion.

The lease freezes task context and policy at dispatch. Packet generation and
report validation use that revision, rather than silently treating later edits
to a task or spec as the worker's original instructions.

## Runtime layout

Orchid creates runtime files on demand:

```text
.orchid/
  locks/           # repository-wide mutation lock
  leases/          # JSON lease records and Git baselines
  packets/         # generated role-specific Markdown packets
  reports/         # worker report contract paths
  buds/            # snapshotted runtime-only instructions
  spec-research/   # disposable pre-spec investigation
  goals/           # goal contracts, cycle state, traces, and reports
  goal-current     # active goal ID pointer
```

Do not hand-edit these files during routine orchestration. Mutating commands use
a repository-local lock and confine managed paths to the repository. Orchid
explicitly rejects symlinked lease, packet, bud, and report directories.
Leases are the only coordination claim Orchid makes; it has no separate
resource-claim protocol.

## CLI reference

Most commands return compact JSON. Add the global `--pretty` flag before or
after the subcommand for human-readable JSON. `orchid goal`, `orchid goal init`,
`orchid goal status`, and `orchid goal finish` return Markdown prompts instead.

Successful commands exit with status `0`. Structured failures include an
`error` message and, where defined, a stable `code`, then exit with status `1`.

| Command | Purpose |
| --- | --- |
| `orchid ready` | List dispatchable task files. Requires `--spec` or `--all-open`. |
| `orchid next` | Decide the next orchestration phase. Requires `--spec` or `--all-open`. |
| `orchid status` | Summarize specs, task states, active leases, or one `--agent-id`. |
| `orchid doctor` | Read-only runtime health: lint summary, stale work, corrupt leases, and safe recovery commands. |
| `orchid inspect --lease <ID>` | Read-only detail for one active or terminal lease and its handoff artifacts. |
| `orchid lease` | Reserve one spec task for a scoped worker. |
| `orchid bud` | Create a runtime-only scoped lease and worker packet. |
| `orchid lease-attach-agent` | Attach a runtime agent id after lease creation. |
| `orchid running` | List active lease runtime files. |
| `orchid heartbeat` | Refresh a lease heartbeat timestamp. |
| `orchid stale` | Find leases older than a duration, defaulting to `30m`. |
| `orchid release` | Release a lease without completing its task. |
| `orchid packet` | Generate a `worker`, `validator`, `reviewer`, or `loop-runner` packet. |
| `orchid report-check` | Validate report frontmatter and lease/report alignment. |
| `orchid git-status` | Return compact Git status plus active lease ids. |
| `orchid git-touched` | Compare Git changes against a lease scope. |
| `orchid git-stage-plan` | Return safe pathspecs for a completed lease. |
| `orchid complete` | Mark verified work complete. |
| `orchid completion-recover --lease <ID>` | Finish a task completion that was interrupted after its durable intent was written. |
| `orchid close` | Delete one lease's runtime files after handoff. |
| `orchid cleanup --completed` | Delete completed or released runtime artifacts. |
| `orchid block` | Mark a task blocked with a reason. |
| `orchid lint` | Validate task-file structure. |
| `orchid research-path` | Print or create `.orchid/spec-research/<spec-id>`. |
| `orchid research-clean` | Delete a spec research workspace. |
| `orchid goal` | Advance the current metric loop and render its agent prompt. |
| `orchid goal init` | Create the goal contract, baseline, budgets, and first hypothesis. |
| `orchid goal status` | Read current goal state without advancing it. |
| `orchid goal finish` | Stop the current goal without creating a pull request. |

Run `orchid <COMMAND> --help` for the complete argument reference.

## Agent contract

Workers receive generated packets. Coordinators should follow JSON ACKs instead
of parsing those packets. Goal commands are the deliberate exception: their
Markdown is the agent-facing state-machine prompt.

Workers should:

- Edit only inside the lease scope.
- Run focused validation.
- Write the report path returned by Orchid.
- Avoid staging, committing, editing task state, completing leases, or closing
  runtime files.

Coordinators should:

- Choose worker effort and optional model from `next` or `ready` before
  spawning a worker.
- Treat worker reports as claims until validation passes.
- Run `report-check`, `git-touched`, and project-specific validation before
  `complete`.
- Resolve ordinary in-scope attribution ambiguity as mayor-owned work with
  `complete --accept-attribution <path> --reason "..."`; do not dispatch a
  reconciliation-only worker.
- Stage only the pathspecs from `git-stage-plan`; do not use `git add .` in a
  shared worktree.
- Close or clean completed runtime files after commit or review state is
  durable.

## Troubleshooting

### `goal scope must include at least one --scope path`

Add one or more repeatable scope flags to `orchid goal init`:

```sh
orchid goal init <OTHER_FLAGS> --scope src/search --scope benches/search.rs
```

### `next` returns `recover`

Inspect the returned stale lease IDs, then either resume the worker and refresh
its heartbeat or release the lease:

```sh
orchid stale --older-than 30m
orchid heartbeat <LEASE_ID>
# or
orchid release <LEASE_ID> --reason "abandoned worker"
```

### Runtime state needs attention

Start with a bounded read-only health check. It reports stale leases, corrupt
runtime files, and a `completion-recover` command when Orchid has a durable
completion intent it can safely attempt to reconcile:

```sh
orchid doctor --pretty
orchid inspect --lease <LEASE_ID> --pretty
```

`doctor` does not release work, rewrite packets, or claim resources. Use
`next`, `report-check`, and the returned recovery command to choose the next
authorized action.

### A lease cannot run in parallel

Run `orchid running --pretty` and compare the reported lease scopes with current
task scopes. Use `--serial` when work shares paths or behavior. `spec.toml` with
`fanout_policy = "serial"` always rejects `--allow-parallel`.

### Completion or staging is unsafe

Inspect both contracts before changing task state:

```sh
orchid report-check .orchid/reports/<LEASE_ID>.md
orchid git-touched --lease <LEASE_ID> --explain
```

`preexisting_dirty` alone is informational. The mayor may accept exact,
evidence-backed ambiguous paths already in frozen scope with
`complete --accept-attribution`. Orchid fingerprints that accepted content, so
any later edit is again unsafe to stage; out-of-scope, changed-after-release,
and missing-snapshot evidence remains a hard stop. Do not bypass the returned
staging plan with `git add .`.

## Included skills

This repository includes agent skills that wrap the CLI:

- [`skills/make-specs`](skills/make-specs/SKILL.md): creates `requirements.md`,
  `design.md`, optional `spec.toml`, and scoped task files.
- [`skills/orchid`](skills/orchid/SKILL.md): runs the orchestration loop,
  validation, staging, cleanup, bud work, and goal cycles.

Copy or install the skills into your agent setup, then adapt their validation,
review, and commit conventions to your repository.

For a longer prompt that asks another agent to research and customize these
skills, see
[`skills/skill-enrichment-prompt.md`](skills/skill-enrichment-prompt.md).

## Develop

Run the local checks used by CI:

```sh
cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
cargo package --locked
```

Source anchors for README claims:

- [`Cargo.toml`](Cargo.toml) defines the crate metadata and Rust baseline.
- [`src/cli.rs`](src/cli.rs) defines the CLI surface and command output mode.
- [`src/paths.rs`](src/paths.rs) defines repository discovery and confined
  runtime paths.
- [`src/orchestration.rs`](src/orchestration.rs) implements leases, packets,
  reports, Git checks, policy enforcement, linting, and cleanup.
- [`src/planner.rs`](src/planner.rs) defines the `next` phase decision order.
- [`src/gitstate.rs`](src/gitstate.rs) implements Git baselines, attribution,
  staging plans, and goal keep/discard operations.
- [`src/runtime.rs`](src/runtime.rs) defines locks, lease persistence, stale
  detection, and runtime cleanup.
- [`src/model.rs`](src/model.rs) defines task, lease, report, policy, and routing
  metadata contracts.
- [`src/specs.rs`](src/specs.rs) implements spec selection, inactive spec
  filtering, readiness, and dependencies.
- [`src/taskfile.rs`](src/taskfile.rs) implements task frontmatter parsing and
  round-trip updates.
- [`src/goal.rs`](src/goal.rs) implements the goal loop.
- [`tests/cli.rs`](tests/cli.rs) exercises the end-to-end CLI contracts.
- [`.github/workflows/release.yml`](.github/workflows/release.yml) defines the
  published archives, npm wrapper, and GHCR image targets.
- [`Dockerfile`](Dockerfile) builds the GHCR image from verified Linux release
  assets.
- [`npm/orchid`](npm/orchid) contains the npm launcher and its release-asset
  checksum verification.
