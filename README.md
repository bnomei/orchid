# orchid

[![Crates.io Version](https://img.shields.io/crates/v/orchid-cli)](https://crates.io/crates/orchid-cli)
[![Crates.io Downloads](https://img.shields.io/crates/d/orchid-cli)](https://crates.io/crates/orchid-cli)
[![License](https://img.shields.io/crates/l/orchid-cli)](https://crates.io/crates/orchid-cli)
[![Rust 1.85+](https://img.shields.io/badge/rust-1.85%2B-orange)](https://www.rust-lang.org)

Task-file orchestration helper for coordinating scoped agent work in a shared
Git worktree.

Orchid turns repo-local Markdown task files into a predictable agent handoff:
find ready work, lease one scoped task, generate a packet, verify the report,
and stage only the files that belong to that lease. It is useful when you want
agents to collaborate on bounded implementation slices without a separate
service, database, or custom runtime.

Orchid is:

- File-first: specs live in `specs/`, task state lives in TOML frontmatter, and
  runtime files stay under `.orchid/`.
- Scope-aware: leases record write scope and block overlapping active work.
- Agent-friendly: every command prints JSON so coordinators can follow ACKs
  instead of scraping prose.
- Git-conscious: leases capture a baseline, attribute touched files, and emit a
  staging plan.

## Why It Exists

Long-running agents need state outside the chat window. Orchid keeps that state
in Git-friendly files: specs, task packets, reports, touched-file evidence, and
completion metadata. A fresh agent session can resume from those artifacts
instead of replaying the conversation.

That also makes spawned subagents more effective. Each worker can start with a
fresh, isolated context containing only the lease, task, requirements, design,
and report template it needs. Instead of dragging the full coordinator thread
into every implementation attempt, Orchid keeps workers in a focused slice of
the problem where they can reason and edit with less distraction.

It is not formal spec-driven design. It is a small harness loop: capture the
target, split it into scoped slices, lease one slice, verify it, write state
back, and repeat.

Compared with systems like [Beads](https://gastownhall.github.io/beads/),
[Gas Town](https://docs.gastownhall.ai/), or
[ultimate-pi](https://pi.dev/packages/ultimate-pi), Orchid sits at the lighter
end: one Rust CLI, Markdown/TOML specs, compact JSON ACKs, no service, no
database, no daemon. Use those larger systems when you need a full issue graph,
dashboards, routing, or cross-repo coordination. Use Orchid when you want a
narrow, Git-ready execution layer that your own skills can adapt.

## Skills

Most users should interact with Orchid through the included skills:

- `skills/make-specs`: turns a request into `requirements.md`, `design.md`,
  `spec.toml`, and scoped `tasks/T###.md` files.
- `skills/orchid`: runs the Orchid loop: observe, lease, dispatch, validate,
  complete, stage, commit/review, clean up, and branch-local goal cycles.

Install or copy them into your skill setup, then adjust them to match
your own agent preferences, validation rules, and repository conventions. Once
you use the skills, the spec creation and orchestration processes take over the
stateful parts: they create and revise spec files, lease tasks, generate worker
packets, check reports, attribute touched files, update task frontmatter, and
clean `.orchid/` runtime files.

Example prompt for creating a spec:

```text
Use $make-specs for what we just discussed.
```

Example prompt for executing a spec:

```text
Implement the open spec 012 with $orchid.
```

> [!TIP]
> For a longer prompt that asks another agent to research and customize the
included skill stubs, see
[skills/skill-enrichment-prompt.md](skills/skill-enrichment-prompt.md).

## Git Ignore

Orchid runtime files are disposable and should not be committed. Add this to
your repository `.gitignore`:

```gitignore
.orchid/
```

Keep `specs/` task files and any copied `skills/` files under version control.

## Installation

### GitHub Releases

Download a prebuilt archive from the
[GitHub Releases](https://github.com/bnomei/orchid/releases) page, extract it,
and place `orchid` on your `PATH`.

### Crates.io

```sh
cargo install orchid-cli
```

### From Source

```sh
git clone https://github.com/bnomei/orchid.git
cd orchid
cargo build --release
```

## CLI Quickstart

The skills call the same CLI shown below. You can also drive Orchid manually:

```sh
orchid ready --spec example
orchid next --spec example
orchid lease example T001 --owner worker:agent_123 --agent-id agent_123 --serial
orchid packet --lease l_123 --role worker
```

## Workflow

Use Orchid when work can be split into small, scoped task files and handed to
one worker at a time.

1. Write a spec under `specs/<spec-id>/`.
2. Ask Orchid what phase the spec is in.
3. Inspect the next ready task's worker metadata from Orchid's JSON ACK.
4. Spawn the worker with that `worker_reasoning_effort` and optional model.
5. Lease exactly one ready task to that worker and generate the packet.
6. Validate the lease or packet ACK repeats the expected worker metadata.
7. Check the worker report and touched files.
8. Complete the lease after validation.
9. Stage only the paths Orchid says belong to the lease.
10. Close completed runtime files.

Inspect the queue and choose the next action:

```sh
orchid ready --spec example
orchid next --spec example
orchid status --spec example
```

`next` reports a phase such as `dispatch`, `wait`, `validate`, `stage`,
`cleanup`, `recover`, `blocked`, or `done`. By default, `ready` and `next`
include dispatch detail, blocked tasks, and the candidate task's
`worker_reasoning_effort` plus non-empty `worker_model`. This is the decision
point for spawning: pick the worker effort from `next` or `ready` before
creating the agent. Use `--brief` only when a caller needs the compact legacy
payload. Use `status` for a cheap count of task state and active leases.

After spawning the worker with the effort/model from `next` or `ready`, lease
the task to that agent and generate the packet:

```sh
orchid lease example T001 --owner worker:agent_123 --agent-id agent_123 --serial
orchid packet --lease l_123 --role worker
```

The lease records owner, scope, worker execution metadata, Git baseline, report
path, and heartbeat data under `.orchid/`. The lease and packet ACKs repeat
`worker_reasoning_effort` and non-empty `worker_model` so the coordinator can
validate that the spawned worker matches the task. If the mayor raises effort
for risk, do it before spawning; do not lower explicit task or user-requested
effort.

For scoped ephemeral work that should keep Orchid's lease, scope, packet, Git,
and report rails without creating durable spec files, create a bud:

```sh
orchid bud \
  --title "Diagnose research runner failure" \
  --scope src/research \
  --scope src/config.rs \
  --instructions /tmp/bud.md \
  --worker-reasoning-effort medium \
  --agent-id agent_123
```

`bud` writes only runtime files under `.orchid/`: a lease, instruction snapshot,
and worker packet. It returns JSON with the `lease_id`, `packet`, `report`,
`worker_reasoning_effort`, and non-empty `worker_model`. It does not create
`specs/` files and does not pre-create the report file. For buds, choose the
worker effort before spawning and pass the same value to `bud`; its ACK echoes
that metadata for validation. The worker reads the packet and writes the report.
The coordinator still owns `report-check`, `git-touched`, validation,
`complete`, and `close`.

If the orchestrator only learns the runtime agent id after creating a lease,
attach it later:

```sh
orchid lease-attach-agent --lease l_123 --agent-id agent_123
orchid status --agent-id agent_123
```

`agent_id` is only for discovery and recovery. Use the returned `lease_id` for
all operational commands.

Track work that is still in flight:

```sh
orchid running
orchid heartbeat l_123
orchid stale --older-than 30m
orchid release l_123 --reason "worker stopped"
```

The worker writes its report to the path returned by `lease` or shown in the
packet. A reviewer can then check the report, inspect touched files, and inspect
the repo baseline:

```sh
orchid report-check .orchid/reports/l_123.md
orchid git-touched --lease l_123
orchid git-status
```

After validation, mark the task complete and ask Orchid for the staging plan:

```sh
orchid complete --lease l_123 --verified-by validator:agent_456
orchid git-stage-plan --lease l_123
```

The coordinator stages only those pathspecs, follows the repo's commit and
signing conventions, runs any independent auto-review, and then closes the
lease artifacts:

```sh
orchid close --lease l_123
orchid cleanup --completed
```

For spec preparation and recovery, use the supporting commands:

```sh
orchid lint
orchid block example T002 --reason "needs product decision"
orchid research-path example --create
orchid research-clean example
```

## Goal Loop

`orchid goal` runs a branch-local improvement loop for one measurable target.
It is Markdown-first for agents and stores durable state under
`.orchid/goals/<goal-id>/`.

Start a goal with the metric, direction, budget, and first hypothesis:

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

By default the goal id is derived from the current branch and the evaluator is
`just goal-eval`. The evaluator must run from the repository root and print one
JSON object with `status`, `recommendation`, `metric`, `baseline`, `candidate`,
`delta`, and `reason`. Use repeatable `--protected-surface <path>` flags for
evaluator or fixture files that should block automatic keep/discard if changed,
and repeatable `--scope <path>` flags for advisory work surfaces.

Run the loop with:

```sh
orchid goal
orchid goal status
orchid goal finish
```

When a cycle is ready, Orchid prints the report path under
`.orchid/goals/<goal-id>/reports/C###.md`. The agent writes that Markdown report
with TOML frontmatter containing `cycle`, `status`, and `next_hypothesis`.
Orchid then runs the evaluator, appends `measurements.jsonl` and `results.jsonl`,
commits kept candidate changes as `goal(<goal-id>): keep <cycle>`, or discards a
failed attempt with `git reset --hard <baseline>` and non-`-x` clean behavior
while preserving `.orchid/`. v0 does not create pull requests.

## Spec Layout

An active spec is a directory under `specs/`:

```text
specs/001-example/
  requirements.md
  design.md
  spec.toml
  tasks/
    T001.md
    T002.md
```

Inactive specs are ignored by dispatch commands when their directory is named
`DRAFT`, `TBD`, `MANUAL`, `DONE`, or starts with `DRAFT-`, `TBD-`, `MANUAL-`,
or `DONE-`.

Task files use TOML frontmatter plus Markdown body:

```md
+++
id = "T001"
title = "Implement the first slice"
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

Describe the local code and decisions the worker needs.

## DoD

List the observable completion criteria.

## Validation

Name the focused command or check for this slice.
```

`scope` is the safety boundary Orchid uses for overlap checks, touched-file
attribution, and staging plans. `verification_mode` must be `mayor`,
`required`, or `validator`. `worker_reasoning_effort` must be `low`, `medium`,
`high`, or `xhigh`; omitted legacy values default to `medium`. Omit or leave
`worker_model` blank unless a task needs a specific model override; only
non-empty values are snapshotted into leases and emitted in ACKs.

## Agent Contract

The packet generated by `orchid packet` contains the lease details, worker
execution metadata, task file, requirements, design, and a report template.
Workers should edit only inside the task scope, run focused validation, and
write the report path returned by the lease or packet command. Workers and
reviewers should not stage files, commit, edit task state, close leases, or
take over final handoff.

Coordinators should follow Orchid's JSON ACKs rather than reading task files or
generated packets during orchestration. ACKs surface `worker_reasoning_effort`
and non-empty `worker_model` so the coordinator can choose the worker spawn
settings directly from trusted lease metadata.

Reports use TOML frontmatter:

```md
+++
lease_id = "l_123"
status = "ready_for_validation"
commands_run = ["cargo test"]
result = "passed"
+++

## Summary

## Evidence

## Notes
```

Treat reports as claims. Use `orchid report-check`, `orchid git-touched`, and
your own validation before running `orchid complete`.

## Staging And Cleanup

Before staging or committing, ask Orchid for a pathspec plan:

```sh
orchid git-stage-plan --lease l_123
```

Stage only the returned `pathspecs`. Do not use `git add .` in a shared
worktree. Once the task state is durable and commit/review status is recorded,
close the lease:

```sh
orchid close --lease l_123
orchid cleanup --completed
```
