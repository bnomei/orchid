# orchid

[![Crates.io Version](https://img.shields.io/crates/v/orchid-cli)](https://crates.io/crates/orchid-cli)
[![Crates.io Downloads](https://img.shields.io/crates/d/orchid-cli)](https://crates.io/crates/orchid-cli)
[![License](https://img.shields.io/crates/l/orchid-cli)](https://crates.io/crates/orchid-cli)
[![Rust 1.82+](https://img.shields.io/badge/rust-1.82%2B-orange)](https://www.rust-lang.org)

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
  complete, stage, commit/review, and clean up.

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

For a longer prompt that asks another agent to research and customize the
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

```sh
cargo install orchid-cli
```

## CLI Quickstart

The skills call the same CLI shown below. You can also drive Orchid manually:

```sh
orchid ready --spec example --explain
orchid lease example T001 --owner worker:agent_123 --serial
orchid packet --lease l_123 --role worker
```

## Workflow

Use Orchid when work can be split into small, scoped task files and handed to
one worker at a time.

1. Write a spec under `specs/<spec-id>/`.
2. Ask Orchid what phase the spec is in.
3. Lease exactly one ready task.
4. Generate a packet and hand that packet path to a worker.
5. Check the worker report and touched files.
6. Complete the lease after validation.
7. Stage only the paths Orchid says belong to the lease.
8. Close completed runtime files.

Inspect the queue and choose the next action:

```sh
orchid ready --spec example --explain
orchid next --spec example
orchid status --spec example
```

`next` reports a phase such as `dispatch`, `wait`, `validate`, `stage`,
`cleanup`, `recover`, `blocked`, or `done`. With `--explain`, it also includes
the relevant queue or runtime detail. Use `status` for a cheap count of task
state and active leases.

Lease one task before handing it to an agent, then generate the packet:

```sh
orchid lease example T001 --owner worker:agent_123
orchid packet --lease l_123 --role worker
```

The lease records owner, scope, Git baseline, report path, and heartbeat data
under `.orchid/`.

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
`required`, or `validator`.

## Agent Contract

The packet generated by `orchid packet` contains the lease details, task file,
requirements, design, and a report template. Workers should edit only inside
the task scope, run focused validation, and write the report path returned by
the lease or packet command. Workers and reviewers should not stage files,
commit, edit task state, close leases, or take over final handoff.

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
