DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/gitstate.rs:138 | git-unavailable-safe-stage

# Git Unavailable Makes Stage Plan Default Safe

## Finding

When git is not available in the repo, `git_status_data` returns only `git: false`. `touched_for_lease` then produces empty change sets and never sets `safe_to_stage: false`. `stage_plan_for_lease` defaults `safe_to_stage` to `true` when the field is absent.

## Violated Invariant Or Contract

`git-touched` and `git-stage-plan` should not imply staging is safe when git attribution is unavailable. `git-status` exposes `git: false`; lease-scoped git commands should surface the same limitation.

## Oracle

`git_status_data` early return (`gitstate.rs:138-143`). `stage_plan_for_lease` uses `.unwrap_or(true)` for `safe_to_stage` (`gitstate.rs:672-675`). Normal git repos set `safe_to_stage: false` when out-of-scope or ambiguous paths exist (`644-646`).

## Counterexample

1. Run orchid in a directory without git (or where `git rev-parse` fails).
2. `orchid lease` captures empty `baseline_changed`.
3. Worker edits files under lease scope.
4. `orchid git-touched --lease l_x` returns in-scope paths with no `safe_to_stage: false`.
5. `orchid git-stage-plan --lease l_x` omits `safe_to_stage` (true by default) and returns pathspecs.

## Why It Might Matter

Coordinators that stage from `git-stage-plan` pathspecs without reading `git: false` from a separate command may stage changes without baseline attribution in non-git trees.

## Proof

Dataflow trace: `git_available` false → empty status → `touched_for_lease` skips blocked branches → `safe_to_stage` defaults true at stage plan sink.

## Counterevidence Checked

In normal git repos, scope and ambiguity checks work. Orchid does not execute `git add` itself; risk is on external coordinators. Still a contract mismatch between `git-status` signal and lease-scoped plan output.

## Suggested Next Step

Set `safe_to_stage: false` and include `git: false` in `git-touched` / `git-stage-plan` payloads when git is unavailable.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: reopened. The current code propagates `git: false` from `touched_for_lease`, but `stage_plan_for_lease` still computes `safe_to_stage` with `unwrap_or(true)` when the field is absent. A non-git root therefore still yields a stage plan whose safety flag defaults true instead of explicitly blocking attribution.
- 2026-06-27: fixed. `touched_for_lease` now emits `safe_to_stage: false` whenever git is unavailable, so `git-touched` and `git-stage-plan` both report the missing attribution boundary instead of allowing `StagePlan` to default safe. Regression test `stage_plan_marks_unsafe_when_git_unavailable` now asserts both `git: false` and `safe_to_stage: false`.

DEVANA-KEY: src/gitstate.rs:138 | git-unavailable-safe-stage
DEVANA-SUMMARY: fixed | P2 | high | When git is unavailable, git-stage-plan defaults safe_to_stage to true instead of blocking staging attribution.
