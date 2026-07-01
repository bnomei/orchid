# orchid

[![Crates.io Version](https://img.shields.io/crates/v/orchid-cli)](https://crates.io/crates/orchid-cli)
[![Crates.io Downloads](https://img.shields.io/crates/d/orchid-cli)](https://crates.io/crates/orchid-cli)
[![License](https://img.shields.io/crates/l/orchid-cli)](https://crates.io/crates/orchid-cli)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange)](https://www.rust-lang.org)

Orchid is a Rust CLI for coordinating scoped agent work from repo-local task
files. It keeps durable work definitions in `specs/`, writes disposable runtime
state under `.orchid/`, and emits JSON ACKs that a coordinator can follow
without scraping prose.

Use Orchid when you want to:

- Find the next ready task in a Markdown spec.
- Lease one task or one runtime-only bud to an agent.
- Generate worker, validator, reviewer, or loop-runner packets.
- Validate worker reports and compare touched files against the lease scope.
- Stage only the paths that belong to a completed lease.
- Run a branch-local metric improvement loop with `orchid goal`.

Orchid is intentionally small: one CLI, Markdown plus TOML files, JSON ACKs, no
daemon, no database, and no built-in pull request workflow.

## Install

Use a release archive when you only need the binary, or use Cargo when Rust is
already part of your toolchain.

### GitHub Releases

Download an archive from
[GitHub Releases](https://github.com/bnomei/orchid/releases), extract it, and
put the `orchid` binary on your `PATH`.

### Crates.io

```sh
cargo install orchid-cli
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

## Quickstart

This quickstart creates one dispatchable task, leases it, and generates a worker
packet. Run it from a Git repository where `orchid` is on `PATH`.

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
   orchid close --lease <LEASE_ID>
   ```

   Stage only the pathspecs returned by `git-stage-plan`.

## Main workflows

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
orchid close --lease <LEASE_ID>
orchid cleanup --completed
```

`next` returns the recommended phase: `dispatch`, `wait`, `validate`, `stage`,
`cleanup`, `recover`, `blocked`, or `done`. Detailed `next` and `ready` output
is the default. Use `--brief` only for compact polling.

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
generates a worker packet, and returns the packet and report paths. It does not
pre-create the report file.

### Goal loop

Use `orchid goal` for one branch-local improvement target where an evaluator
decides whether each attempt is kept, discarded, blocked, or finished.

```sh
orchid goal init \
  --goal "Reduce search ranking p95 without changing correctness" \
  --metric p95_ms \
  --direction lower-is-better \
  --min-delta 5 \
  --hypothesis "cache normalized query features before ranking" \
  --max-iterations 10 \
  --max-duration 10h
```

The default evaluator is `just goal-eval`. Override it with `--evaluator` when
your repository uses a different command. The evaluator must run from the repo
root and print one JSON object with `status`, `recommendation`, `metric`,
`baseline`, `candidate`, `delta`, and `reason`.

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

Goal keeps commit candidate changes as `goal(<goal-id>): keep <cycle>`. Goal
discards restore the baseline with Git while preserving `.orchid/`, so run goal
loops only on branches where candidate changes may be automatically kept or
discarded.

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

### Spec policy

`spec.toml` is optional. Orchid directly enforces these policy keys:

| Key | Effect |
| --- | --- |
| `execution_policy = "manual"` | Blocks dispatch for the spec. |
| `human_checkpoint = "before-implementation"` | Blocks dispatch until the policy changes. |
| `fanout_policy = "serial"` | Rejects `--allow-parallel` leases for the spec. |

Other policy keys are preserved and copied into generated packets for agents to
read.

## Command reference

Most commands return JSON. `orchid goal`, `orchid goal init`,
`orchid goal status`, and `orchid goal finish` return Markdown.

| Command | Purpose |
| --- | --- |
| `orchid ready` | List dispatchable task files. Requires `--spec` or `--all-open`. |
| `orchid next` | Decide the next orchestration phase. Requires `--spec` or `--all-open`. |
| `orchid status` | Summarize specs, task states, active leases, or one `--agent-id`. |
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
| `orchid close` | Delete one lease's runtime files after handoff. |
| `orchid cleanup --completed` | Delete completed or released runtime artifacts. |
| `orchid block` | Mark a task blocked with a reason. |
| `orchid lint` | Validate task-file structure. |
| `orchid research-path` | Print or create `.orchid/spec-research/<spec-id>`. |
| `orchid research-clean` | Delete a spec research workspace. |
| `orchid goal` | Run or inspect a branch-local metric loop. |

Use `--pretty` with any JSON command when humans need formatted output.

## Agent contract

Workers receive generated packets. Coordinators should follow JSON ACKs from
`ready`, `next`, `lease`, `bud`, `packet`, `report-check`, `git-touched`, and
`git-stage-plan`.

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
- Stage only the pathspecs from `git-stage-plan`; do not use `git add .` in a
  shared worktree.
- Close or clean completed runtime files after commit or review state is
  durable.

## Included skills

This repository includes agent skill stubs that wrap the CLI:

- [`skills/make-specs`](skills/make-specs/SKILL.md): creates `requirements.md`,
  `design.md`, optional `spec.toml`, and scoped task files.
- [`skills/orchid`](skills/orchid/SKILL.md): runs the orchestration loop,
  validation, staging, cleanup, bud work, and goal cycles.

Copy or install the skills into your agent setup, then adapt their validation,
review, and commit conventions to your repository.

For a longer prompt that asks another agent to research and customize the skill
stubs, see
[`skills/skill-enrichment-prompt.md`](skills/skill-enrichment-prompt.md).

## Develop

Run the test suite:

```sh
cargo test
```

Run the source hygiene check:

```sh
scripts/check-source-hygiene.sh
```

Source anchors for README claims:

- [`Cargo.toml`](Cargo.toml) defines the crate metadata and Rust baseline.
- [`src/cli.rs`](src/cli.rs) defines the CLI surface and command output mode.
- [`src/orchestration.rs`](src/orchestration.rs) implements leases, packets,
  reports, Git checks, policy enforcement, linting, and cleanup.
- [`src/specs.rs`](src/specs.rs) implements spec selection, inactive spec
  filtering, readiness, and dependencies.
- [`src/taskfile.rs`](src/taskfile.rs) implements task frontmatter parsing and
  round-trip updates.
- [`src/goal.rs`](src/goal.rs) implements the goal loop.
- [`tests/cli.rs`](tests/cli.rs) exercises the end-to-end CLI contracts.
