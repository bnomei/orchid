DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/model.rs:128 | scope-dot-inert

# A "." scope entry is silently inert: stages nothing and conflicts with nothing

## Finding

`normalize_scope_entry` collapses the whole-repo spellings `""`, `"/"`, and `"./"` to the empty string, and `Scope::contains_path` / `Scope::overlaps` guard on `!norm.is_empty()` so a meaningless entry never matches. But the single most common whole-repo spelling, `"."`, is NOT collapsed: it survives as the non-empty token `"."`, which then matches neither the equality branch nor the `"./"`-prefix branch. A task or lease whose `scope` is `["."]` therefore (a) treats every changed file as out-of-scope so nothing can be staged, and (b) overlaps with no other scope so it disables scope-conflict blocking entirely.

## Violated Invariant Or Contract

A scope entry that denotes the repository root / current directory must either be rejected or behave as a root that contains every path. The normalizer's deliberate collapse of `""`, `"/"`, `"./"` (model.rs:130-133) encodes the intent that root-like spellings are handled; `"."` is the one spelling that slips through into a non-empty-but-matches-nothing state.

## Oracle

Neighboring implementation in the same function. `normalize_scope_entry("/")` → `""`, `normalize_scope_entry("./")` → `""` (the `strip_prefix("./")` loop), `normalize_scope_entry("")` → `""`. But `normalize_scope_entry(".")`: trim → `"."`; `\`→`/` → `"."`; `strip_prefix("./")` does not fire (string is `"."`, not `"./"`); `trim_matches('/')` → `"."`. So `"."` is the only root-like spelling left non-empty.

## Counterexample

Task frontmatter `scope = ["."]` (author means "this task may touch the whole repo"):

- `path_in_scope("src/main.rs", &["."])` → `false`. In `touched_for_lease` (src/gitstate.rs:589-605) every visible changed path fails `path_in_scope`, lands in `out_of_scope`, and `safe_to_stage` becomes `false` — the stage plan is empty even though the task claimed whole-repo scope.
- `scopes_overlap(&["."], &["src/x"])` → `false` (needs `"." == "src/x"` or one a `"./"`-prefixed child of the other). In `ready_tasks` (src/specs.rs:409) a `["."]` lease conflicts with nothing, so unrelated tasks dispatch in parallel against a lease that intended to cover everything.

## Why It Might Matter

The second direction is the dangerous one: scope-conflict blocking is Orchid's core safety rail against overlapping parallel writers (README: "leases record write scope and block overlapping active work"). A `"."` lease silently bypasses that rail, allowing parallel agents to edit the same files the `"."` lease is working in. The first direction silently breaks staging for that task.

## Proof

Pure-function evaluation of `normalize_scope_entry` + `Scope::contains_path`/`Scope::overlaps` on the literal `"."`, contrasted with the empty-collapse the same function applies to `"/"`, `"./"`, `""`. Cross-entry mismatch: sibling root spellings collapse to empty (inert-and-guarded) while `"."` survives (non-empty-but-matches-nothing), so the `!is_empty()` guard is defeated and both `contains_path` and `overlaps` return wrong results.

## Counterevidence Checked

One could argue `"."` is simply unsupported input. Rebuttal: (1) the normalizer deliberately accepts and collapses the equivalent forms `"/"`, `"./"`, `""`, so handling root-like spellings is in-contract; (2) `scope` is free-text user frontmatter (`taskfile.rs:83` → `string_list`), so `["."]` is reachable from real input, and `"."` is the canonical git/shell shorthand for the whole tree; (3) the failure is silent — no rejection, no diagnostic — producing wrong staging and wrong conflict results. Changed paths from `git status` never normalize to `"."`, so the `norm_path == "."` branch can never rescue a real path.

## Suggested Next Step

In `normalize_scope_entry`, collapse `"."` to `""` alongside `"/"`/`"./"` (or strip a bare leading `.`), OR reject `"."`/root scopes at task-frontmatter validation. Add a unit test asserting `normalize_scope_entry(".") == ""` and that a `["."]` scope overlaps `["src/x"]`.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` prefix. Keep `DEVANA-KEY:` stable unless the same finding moved.

## Status Notes

- 2026-06-25: open by Devana. Static source inspection; pure-function proof, no build/run.
- 2026-06-27: reopened. `normalize_scope_entry(".")` now collapses to an empty normalized entry, but `Scope::contains_path` and `Scope::overlaps` still ignore empty normalized entries while task lint only checks that the raw `scope` array is non-empty. A task with `scope = ["."]` therefore still passes the raw non-empty check but matches no paths and overlaps no normal scope.
- 2026-06-27: fixed. Empty normalized scope entries now mean whole-repo scope in `Scope::contains_path` and `Scope::overlaps`, so `scope = ["."]` (and sibling root-like spellings) contains normal repo paths and conflicts with narrower scopes. Unit coverage added to `scope_helpers_match_directory_boundaries`.

DEVANA-KEY: src/model.rs:128 | scope-dot-inert
DEVANA-SUMMARY: fixed | P2 | high | A "." scope entry normalizes to a non-empty token that matches no path, so it stages nothing and bypasses scope-conflict blocking entirely while sibling spellings "/", "./", "" collapse safely.
