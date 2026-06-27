DEVANA-FINDING: v1
DEVANA-STATE: fixed | P3 | medium | security=no
DEVANA-KEY: src/gitstate.rs:526 | goal-scope-unenforced

# goal --scope is recorded but never enforced; keep commits every changed file

## Finding

`goal init --scope` (`cli.rs:155-156`) declares the surfaces a goal cycle may
touch and is persisted to `goal.toml` (`src/goal.rs:206`). But `contract.scope`
is only ever written — no evaluation path reads it. The keep path calls
`stage_goal_candidates` (`src/gitstate.rs:526`), which stages *every* visible
changed path (filtered only by the `.orchid/` prefix), with no scope argument and
no scope filter, then `commit_goal_keep` commits all of it as the new baseline.

## Violated Invariant Or Contract

A declared `--scope` should bound what a kept cycle commits; changes outside the
declared scope must not be folded into the goal baseline.

## Oracle

The `--scope` flag and the `scope = [...]` field in `goal.toml` imply
enforcement. The lease flow honors scope (`stage_plan_for_lease` /
`path_in_scope`, gitstate.rs:591); the goal keep flow does not.

## Counterexample

`goal init --scope src/ranker.rs ...`. During a cycle the agent edits
`src/ranker.rs` (intended) and incidentally `README.md`. The evaluator
recommends keep. `stage_goal_candidates` stages both files; `commit_goal_keep`
commits both. `README.md` is now part of the goal baseline and silently
attributed to the cycle.

## Why It Might Matter

Out-of-scope edits are committed into the goal's branch-local baseline without
warning, so the goal loop can quietly absorb unrelated working-tree changes,
defeating the purpose of declaring a scope and muddying cycle attribution.

## Proof

Contract-vs-runtime: `rg "\.scope" src/goal.rs` shows `scope` flows only into
`to_toml` (goal.rs:206); `stage_goal_candidates` (gitstate.rs:526) takes only
`root` and filters solely on the `.orchid/` prefix, so the declared scope has no
consumer in the keep/stage path.

## Counterevidence Checked

Scope may be intended as advisory in goal mode, with `protected_surfaces` as the
only hard guard (`changed_protected_surfaces`, goal.rs:868, does block on
protected paths). That is the strongest argument against this being a defect —
hence P3. Reported because the surfaced `--scope` flag and `goal.toml` field
imply an enforcement that never occurs, matching the repo's existing
"declared-but-unenforced" findings (`min-delta-unenforced`,
`fanout-policy-unenforced`).

## Suggested Next Step

Either enforce `contract.scope` in `stage_goal_candidates` (skip / flag
out-of-scope changes on keep), or document scope as advisory and stop accepting
it as a contract field.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2
`DEVANA-STATE:` and the final `DEVANA-SUMMARY:` prefix. Keep `DEVANA-KEY:` stable
unless the same finding moved.

## Status Notes

- 2026-06-27: open by Devana. Verified scope only written at goal.rs:206; stage_goal_candidates (gitstate.rs:526) takes no scope and filters only `.orchid/`.
- 2026-06-27: fixed (enforce option). `stage_goal_candidates` now takes the declared scope and, when non-empty, retains only changed paths inside it (via `path_in_scope`) before staging; `keep_cycle` passes `contract.scope`. Out-of-scope working-tree edits are no longer folded into the goal baseline on keep — they stay visible and uncommitted, matching the lease flow's scope handling. With no scope declared, behavior is unchanged (stage all). Regression test `goal_keep_commits_only_in_scope_changes` added (fails without the fix); full suite green.

DEVANA-KEY: src/gitstate.rs:526 | goal-scope-unenforced
DEVANA-SUMMARY: fixed | P3 | medium | goal --scope is persisted to goal.toml but read by no evaluation path, so keep stages and commits every changed file (only `.orchid/` excluded), folding out-of-scope edits into the goal baseline.
