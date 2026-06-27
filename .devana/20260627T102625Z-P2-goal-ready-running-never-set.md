DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/goal.rs:683 | goal-ready-running-never-set

# Goal Running State Is Documented But Never Written

## Finding

`docs/goal.md` documents the transition `ready → running → evaluate`, but production code never assigns `GoalStatus::Running` to `state.json`. When no cycle report exists and status is `Ready`, orchid re-renders the ready prompt instead of entering running.

## Violated Invariant Or Contract

Documented goal state machine includes a `running` phase between ready and evaluation. Runtime should either write `running` when work starts or document that the state is unused.

## Oracle

`docs/goal.md:641-644` state transitions. `render_or_evaluate_goal` at `goal.rs:681-687` branches on `GoalStatus::Ready` vs other statuses when report is missing.

## Counterexample

1. `orchid goal init` succeeds; `state.json` has `status: ready`, `cycle: C001`.
2. Agent begins work; no `reports/C001.md` yet.
3. `orchid goal` renders **Goal Ready** instructions again; `state.json` still shows `status: ready`.
4. `GoalStatus::Running` is only reachable via manual `state.json` edit (tests hand-edit this value).

## Why It Might Matter

Coordinators and agents cannot distinguish "cycle not started" from "cycle in progress" using durable goal state, and documented transitions do not match runtime telemetry.

## Proof

Contract mismatch: docs specify `ready → running`; code path `ready + no report → render_goal_ready` never sets `running`.

## Counterevidence Checked

`render_goal_running` exists but is dead code in normal flow. Post-cycle handlers reset to `Ready`, not `Running`. Not a crash; state-machine documentation drift.

## Suggested Next Step

Set `status: running` when rendering the first ready prompt for a cycle, or remove `running` from the documented machine.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: fixed. The bare `orchid goal` command now routes through `render_goal_prompt_and_advance` (src/goal.rs / cli.rs): on the first standalone request for a `ready` cycle with no report yet, it returns the ready kickoff prompt but advances durable `state.json` to `running`, so coordinators can distinguish "not started" from "in progress" (docs/goal.md ready→running→evaluate). The transition is driven from the user-facing command rather than the shared renderer, so `goal init` and post-decision cycle advances leave the new cycle in `ready` until an agent picks it up. Regression test `bare_goal_advances_ready_cycle_to_running` added (fails without the fix); full suite green.

DEVANA-KEY: src/goal.rs:683 | goal-ready-running-never-set
DEVANA-SUMMARY: fixed | P2 | high | docs/goal.md documents ready→running→evaluate but orchid never writes GoalStatus::Running in normal flow.