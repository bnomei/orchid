---
name: orchid
description: Coordinates scoped agent execution through the orchid CLI. Use when a repository has specs/SPEC_ID/tasks/*.md files or needs a scoped bud delegation, and the user asks to implement, dispatch, validate, recover, complete, stage, commit/review, or clean up Orchid leases.
---

# Orchid

Use `orchid` as the control plane from the repository root. Orchid owns leases,
packets, reports, touched-file attribution, staging plans, and `.orchid`
runtime state. Follow JSON ACKs from the CLI instead of parsing generated
Markdown or Git porcelain.

## Mayor Loop

Think in this loop:

`Observe -> Select -> Lease -> Dispatch -> Ingest -> Verify -> Stage -> Commit/Review -> Close -> Continue/Stop`

Start by observing only the user-authorized scope:

```sh
orchid next --spec SPEC_ID --explain
orchid ready --spec SPEC_ID --explain
orchid status --spec SPEC_ID
```

Use `--all-open` only when the user explicitly authorizes work across open
specs. If the authorized queue drains, or `next` returns `blocked` or `done`,
stop and report. Do not broaden scope on your own.

Phase guide:

- `dispatch`: lease one ready task and create a worker packet.
- `wait`: poll `orchid status --spec SPEC_ID` with backoff; do not ask workers
  for routine progress.
- `validate`: check the report, touched files, and task-specific evidence.
- `stage`: ask Orchid for the staging plan, then stage only returned pathspecs.
- `cleanup`: close completed leases after durable commit/review state exists.
- `recover`: inspect stale leases, release or continue them, or report the blocker.

## Command Flow

Lease before dispatching:

```sh
orchid lease SPEC_ID TASK_ID --owner worker:AGENT_ID --agent-id AGENT_ID --serial
orchid packet --lease LEASE_ID --role worker
```

For a scoped ephemeral delegation without durable spec files, create a bud and
hand the returned packet path to the worker:

```sh
orchid bud --title "SHORT TITLE" --scope PATH --instructions PROMPT.md --agent-id AGENT_ID --serial
```

If the agent id becomes known after lease creation, attach it from the
coordinator and then continue with the lease id returned by status:

```sh
orchid lease-attach-agent --lease LEASE_ID --agent-id AGENT_ID
orchid status --agent-id AGENT_ID
```

Use `agent_id` only for discovery/recovery. Operational commands stay
lease-based.

Send spawned agents only the packet path and report path from Orchid's JSON
output. Tell them they are not alone in the worktree and may only work or
review. They must not stage files, commit, edit task state, close leases, or
take over final handoff.

Default to `--serial` in a shared worktree. Use `--allow-parallel` only after
checking active leases and confirming the scopes and likely behavior do not
overlap. File-scope disjointness alone is not enough if tasks share generated
state, global config, migrations, or test fixtures.

When a worker returns, ingest the report and verify the claim:

```sh
orchid report-check REPORT_PATH
orchid git-touched --lease LEASE_ID
orchid git-status
```

Treat worker reports as claims until validation passes. If touched files are
outside scope, classify before proceeding: fix an obviously missing narrow
scope only when authorized; otherwise pause and report. If validation fails,
fix inside scope or return the task to the worker before completion.

After green validation, complete the lease and ask for the staging plan:

```sh
orchid complete --lease LEASE_ID --verified-by VALIDATOR_OR_MAYOR
orchid git-stage-plan --lease LEASE_ID
```

The coordinator stages only returned pathspecs. Never use `git add .` in a
shared worktree. Follow repo conventions for validation, branch state, commit
messages, signing/signoff, and review. Make the intentional commit yourself,
then run an independent auto-review against the original request, the spec, and
the resulting diff before final handoff.

After commit/review state is durable, close runtime files:

```sh
orchid close --lease LEASE_ID
orchid cleanup --completed
```

## Guardrails

- Stay inside the scope the user authorized.
- Clarify scope, dispatch order, review expectations, or final handoff before
  leasing when they are unclear.
- Do not hand-edit `.orchid` runtime files or task state during routine work.
- Do not read generated packets during normal orchestration; packets are
  worker/validator input, and ACKs carry the coordinator details.
- Workers and validators may read packets, repo code, and reports, but they
  must not stage, commit, edit task state, close leases, or own final handoff.
- Account for active leases before dispatching, staging, or cleaning up.
- Inactive spec folders are not dispatchable: `DRAFT-*`, `TBD-*`, `MANUAL-*`,
  `DONE-*`, or exact `DRAFT`, `TBD`, `MANUAL`, `DONE`.
- Keep `.orchid` ephemeral. Close or clean completed runtime files once commit
  and review status are recorded.
- Ignore unrelated dirty worktree changes and never revert user work.

## Done

Stop when the authorized queue is done or blocked, all validated work has been
completed through Orchid, approved pathspecs are staged and committed by the
coordinator, independent review has run, and completed runtime files are closed
or cleaned.
