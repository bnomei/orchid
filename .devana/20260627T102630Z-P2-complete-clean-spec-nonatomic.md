DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/orchestration.rs:676 | complete-clean-spec-nonatomic

# Complete Clean Spec Research Runs After Durable Completion

## Finding

`complete --clean-spec-research` writes task `done` and lease `completed` before calling `clean_spec_research`. If research cleanup fails, the command returns an error but completion is already committed and spec-research artifacts remain.

## Violated Invariant Or Contract

Optional cleanup should not leave the primary operation succeeded on disk while returning failure for the bundled cleanup step, unless documented as best-effort.

## Oracle

`complete` ordering at `orchestration.rs:669-684`: frontmatter and lease save precede `clean_spec_research`. README lists `research-clean` as a separate recovery command.

## Counterexample

1. `orchid complete --lease l_123 --clean-spec-research` on a task with `.orchid/spec-research/example/` present.
2. `write_task_frontmatter` and `save_lease` succeed → task `done`, lease `completed`.
3. `clean_spec_research` fails (permissions, I/O).
4. CLI returns `Err`; task remains done; research workspace still on disk.

## Why It Might Matter

Coordinators see a failed complete while the task is already marked done, requiring manual `research-clean` and creating ambiguous handoff state.

## Proof

Control-flow trace: durable writes → conditional cleanup with `?` propagation and no rollback of task/lease state.

## Counterevidence Checked

Without `--clean-spec-research`, path is consistent. Distinct from `complete-task-before-lease` (ordering between task and lease fields).

## Suggested Next Step

Run research cleanup before marking complete, or return success with a partial cleanup detail when the core completion succeeded.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: fixed. `complete` (src/orchestration.rs) no longer propagates a `--clean-spec-research` failure with `?` after completion is already durable. The bundled cleanup is now best-effort (research-clean is also a standalone recovery command): on failure it records a `spec_research_clean_error` {message, code} detail in the payload and still returns Ok, so a completed task is never reported as a failed command. Chose best-effort over "cleanup-before-complete" to avoid the inverse hazard of deleting research when completion later fails. Regression test `complete_clean_spec_research_failure_keeps_completion` added (forces an undeletable research dir; fails without the fix); full suite green.

DEVANA-KEY: src/orchestration.rs:676 | complete-clean-spec-nonatomic
DEVANA-SUMMARY: fixed | P2 | high | complete --clean-spec-research commits task/lease completion before research cleanup, so cleanup failure leaves a done task and leftover research files.