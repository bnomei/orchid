DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/orchestration.rs:235 | status-all-open-narrower

# Status All-Open Counts Fewer Tasks Than Bare Status

## Finding

`orchid status` without flags loads tasks from all active spec directories. `orchid status --all-open` uses `select_tasks`, which narrows to only the first open spec's tasks. The same flag name produces a smaller task set than omitting it.

## Violated Invariant Or Contract

`--all-open` on `ready` and `next` means "pick the first open spec and dispatch within it." `status --all-open` should not silently mean "count fewer tasks than bare status" without naming the selected spec.

## Oracle

`orchestration.rs:235-238` vs `237-238` bare branch. `ready` includes `skipped_inactive_specs` and selected spec context; `status` omits both.

## Counterexample

1. Repo has open specs `001-a` (3 todo tasks) and `002-b` (5 todo tasks); `001-a` sorts first.
2. `orchid status` → `tasks: 8` (all active specs).
3. `orchid status --all-open` → `tasks: 3` (only `001-a`).
4. Automation comparing the two commands infers incorrect global queue depth.

## Why It Might Matter

Monitoring and coordinator scripts using `--all-open` under-report total work in the repo relative to bare `status`, with no selected-spec field to explain the discrepancy.

## Proof

Cross-entry mismatch: same CLI flag on `status` narrows scope vs bare invocation; asymmetric with user expectation of "all open" breadth.

## Counterevidence Checked

`ready --all-open` intentionally narrows dispatch scope the same way; the bug is the counterintuitive `status` semantics and missing selected-spec echo, not the shared `select_tasks` helper itself.

## Suggested Next Step

Echo `spec` in `status --all-open` payload, or document/count all open specs while still highlighting the dispatch target.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: fixed. `status` (src/orchestration.rs) now captures the selected specs from `select_tasks` and echoes a `specs` field whenever the task count was narrowed (`--spec`/`--all-open`), plus `skipped_inactive_specs` for `--all-open` (mirroring `ready`). The narrowing of `status --all-open` relative to bare `status` is no longer silent — automation can read which spec was counted. Regression test `status_all_open_echoes_selected_and_skipped_specs` added (fails without the fix); full suite green.

DEVANA-KEY: src/orchestration.rs:235 | status-all-open-narrower
DEVANA-SUMMARY: fixed | P2 | high | status --all-open counts only the first open spec while bare status counts all active specs, with no selected-spec field in the ACK.