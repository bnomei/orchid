DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
Location: src/planner.rs:225 | Slug: ready-next-wait

# `ready --spec` and `next --spec` disagree when scopes are disjoint

## Finding

`ready_tasks` blocks a task only when an active lease overlaps its scope or task path. `decide_next` enters the `wait` phase whenever any active lease exists globally, before considering spec-scoped ready work. For disjoint parallel scopes, `ready` can list a task while `next` returns `wait` with no dispatch command.

## Violated Invariant Or Contract

When `ready --spec` reports a task as ready with no scope conflict, `next --spec` should not defer solely because an unrelated active lease exists in a different scope.

## Oracle

Scope overlap check in `ready_tasks` (~403–414 in `src/specs.rs`). Planner `wait` gate: `if !active.is_empty()` (~225–233 in `src/planner.rs`). Dispatch is evaluated only after wait, stage, and cleanup branches.

## Counterexample

1. Active lease on `other/T001`, scope `src/other/`.
2. `example/T005` is `todo` with scope `src/feature/` (disjoint).
3. `orchid ready --spec example` → `T005` in `ready`.
4. `orchid next --spec example` → `phase: "wait"`, empty commands.
5. `orchid lease example T005` without `--allow-parallel` would fail, but `next` never surfaces dispatch intent.

## Why It Might Matter

Coordinators see contradictory ACKs from the two primary observe commands and cannot tell whether to spawn work or keep waiting.

## Proof

Cross-entry mismatch on identical repo state: `ready` includes task, `next` returns `wait`. Control-flow trace: global `active.is_empty()` check overrides scope-safe readiness.

## Counterevidence Checked

Default serial mode explains why `lease` requires `--allow-parallel` when other active leases exist. That does not explain why `ready` omits a scope-conflict reason while `next` waits globally. No test covers disjoint multi-spec parallelism.

## Suggested Next Step

Align `next` wait semantics with `ready_tasks` scope rules, or have `ready` mark tasks blocked when any global active lease exists under serial mode; document and test the chosen policy.

## Status Notes

- 2026-06-27: reopened. `decide_next` now echoes scope-disjoint ready tasks in the wait details, but it still returns `phase: wait` with empty commands whenever any active lease exists before dispatch is considered. The original `ready` versus `next` mismatch remains for disjoint parallel scopes.
- 2026-06-27: fixed. `decide_next` now dispatches ready work when active leases exist but none are serial, adding `--allow-parallel` to the lease command so the command matches the active-lease contract. Active serial leases still produce `wait`. Regression coverage added in `next_dispatches_scope_disjoint_ready_tasks_with_parallel_flag` and planner phase-priority tests.
- 2026-06-27: review fix. Ready-task metadata now carries serial fanout policy, and `next` waits instead of emitting an unrunnable `--allow-parallel` command when the selected ready task belongs to a serial-fanout spec. Regression test `next_waits_for_serial_fanout_ready_task_under_active_lease` covers the command contract.

DEVANA-KEY: src/planner.rs:225 | P2 | ready-next-wait
DEVANA-SUMMARY: fixed | P2 | high | ready lists scope-disjoint tasks but next returns wait whenever any unrelated active lease exists.
