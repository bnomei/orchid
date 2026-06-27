DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/model.rs:128 | scope-parent-inert

# Parent-Directory Scope Entry Is Non-Empty But Inert

## Finding

Scope entry `".."` survives `normalize_scope_entry` as a non-empty string (unlike `"."` which trims to empty). It passes lint's non-empty scope check but matches no normal repo paths and does not overlap with typical scopes, creating a phantom scope that bypasses conflict detection.

## Violated Invariant Or Contract

Non-empty scope entries should constrain writes and participate in overlap checks. README defines scope as the safety boundary for parallel work.

## Oracle

`scope-dot-inert` documents the `"."` case. `normalize_scope_entry` strips `./` prefixes and slashes but does not collapse `..` (`model.rs:128-134`). Lint only rejects `scope().is_empty()` (`orchestration.rs:1325-1327`).

## Counterexample

1. Task frontmatter `scope = [".."]` passes `orchid lint`.
2. `orchid lease` stores `scope: [".."]` on the lease record.
3. Worker edits `src/feature.rs`; `git-touched` marks changes out-of-scope (path does not match `".."`).
4. Second task with `scope = ["src/feature"]` leases successfully — `scopes_overlap([".."], ["src/feature"])` is false.
5. Parallel workers run on the same tree with no scope conflict error.

## Why It Might Matter

A single-character scope typo creates a lease that appears scoped but provides neither staging attribution nor exclusivity.

## Proof

Counterexample value `scope = [".."]` with concrete mismatch in `contains_path` and `scopes_overlap` string-prefix logic.

## Counterevidence Checked

Distinct token from `"."` in `scope-dot-inert`. `SpecId` and `GoalId` reject `".."` at other boundaries; task/bud scope has no analogous guard.

## Suggested Next Step

Reject `..` segments in scope at lint and bud creation, matching `normalize_scope_entry` treatment of `.`.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: fixed. New `model::scope_entry_escapes_root` flags any scope entry containing a `..` path segment (after normalization). Task lint now emits a `scope escapes repo root` error for such entries, and `bud` rejects them with the new `invalid_scope` ErrorCode, so a `..` (or `src/../x`) scope can no longer pass the non-empty check while providing neither staging attribution nor exclusivity. Regression tests `lint_rejects_parent_traversal_scope` and a `..` case in `bud_enforces_scope_and_parallel_guards` added (fail without the fix); full suite green.

DEVANA-KEY: src/model.rs:128 | scope-parent-inert
DEVANA-SUMMARY: fixed | P2 | high | scope entry ".." normalizes to a non-empty inert token that matches no paths and bypasses overlap checks, sibling to the "." case.