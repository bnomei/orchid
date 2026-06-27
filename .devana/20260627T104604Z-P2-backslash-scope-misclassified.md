DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | medium | security=no
DEVANA-KEY: src/model.rs:129 | backslash-scope-misclassified

# Backslash rewrite on git paths misclassifies an out-of-scope file as in-scope

## Finding

`normalize_scope_entry` unconditionally rewrites `\` to `/`
(`src/model.rs:129`). It is a scope-*string* normalizer (intended to accept
Windows-style `src\feature`), but `Scope::contains_path` applies it to the
git-reported *path* too (`src/model.rs:104`), via `path_in_scope` at
`src/gitstate.rs:591`. On Unix/macOS a backslash is a legitimate literal filename
byte, and `git status --porcelain=v2 -z` emits paths verbatim, so a top-level
file literally named `src\evil.rs` is rewritten to `src/evil.rs` and treated as
if it lived inside directory `src`.

## Violated Invariant Or Contract

A path's scope membership must reflect true filesystem containment. The
out-of-scope detection (`out_of_scope` set -> `safe_to_stage=false`,
`gitstate.rs:600-645`) must classify a file that is not inside the scope
directory as out-of-scope.

## Oracle

`touched_for_lease` / `stage_plan_for_lease` exist to flag and block
out-of-scope changes; a literal top-level file `src\evil.rs` is not inside the
`src/` directory and must be reported out-of-scope.

## Counterexample

Lease scope `["src"]`. Worker creates a repo-root file whose name literally
contains a backslash: `src\evil.rs` (one top-level path component). Git reports
`src\evil.rs` verbatim. `path_in_scope("src\evil.rs", ["src"])` ->
`contains_path` -> `normalize_scope_entry("src\evil.rs")` = `"src/evil.rs"`,
which `starts_with("src/")` -> treated in-scope. The file is added to
`stage_paths`, `out_of_scope` stays empty, `safe_to_stage` stays `true`, and the
stage plan emits `:(literal)src\evil.rs`, which matches the real top-level file.
The out-of-scope file is staged and never flagged.

## Why It Might Matter

The scope gate is the containment boundary for a lease's writes; misclassifying a
genuinely out-of-scope file defeats the out-of-scope detection that gate exists
to enforce, and the same root cause also skews `scopes_overlap` (lease-conflict
detection, model.rs:113) and `changed_protected_paths` (gitstate.rs:519).

## Proof

Dataflow: source = literal git path `src\evil.rs`; wrong transform =
`normalize_scope_entry`'s `\`->`/` applied to the git path (model.rs:129 via
model.rs:104); sink = scope gate at gitstate.rs:591 classifies in-scope ->
`stage` set -> stage-plan pathspec.

## Counterevidence Checked

Is the pathspec sink defended? Yes for *injection* — pathspecs are always
`:(literal)`-prefixed with a `--` separator and `Command::args` is not
shell-parsed — but that is irrelevant here: this is a *classification* error
upstream of the sink, where a real out-of-scope file is mislabeled in-scope and
then legitimately staged via its correct literal pathspec. No upstream filter
rejects backslash paths (`is_visible_path` only screens the `.orchid/` prefix,
gitstate.rs:383). Distinct from `scope-dot-inert`/`scope-parent-inert@model.rs:128`
(inert tokens that match nothing); here a token wrongly matches.

## Suggested Next Step

Normalize the scope *strings* once at parse time, but compare git-reported paths
without rewriting `\` (paths from `-z` porcelain are already canonical separators
on the running platform).

## Agent Handoff

After working this report, preserve the original finding body. Update line 2
`DEVANA-STATE:` and the final `DEVANA-SUMMARY:` prefix. Keep `DEVANA-KEY:` stable
unless the same finding moved.

## Status Notes

- 2026-06-27: open by Devana. Verified contains_path normalizes both scope and path through normalize_scope_entry, which rewrites backslash at model.rs:129.
- 2026-06-27: fixed. Split path normalization in two (src/model.rs): a new `normalize_path_for_scope` (no backslash rewrite) is used for the git-reported path in `Scope::contains_path`, while `normalize_scope_entry` keeps the `\`->`/` rewrite for user-authored scope ENTRIES (delegating to the path normalizer after the rewrite). A literal top-level `src\evil.rs` is no longer misclassified as inside `src/`, restoring out-of-scope staging/protected-path detection on Unix/macOS; Windows-style scope strings still match real paths. Regression test `contains_path_does_not_rewrite_backslash_in_git_path` added (verified failing when contains_path uses the old normalizer); full suite green.

DEVANA-KEY: src/model.rs:129 | backslash-scope-misclassified
DEVANA-SUMMARY: fixed | P2 | medium | normalize_scope_entry rewrites `\`->`/` on git-reported paths too, so a literal top-level file `src\evil.rs` is misclassified as inside scope `src`, defeating out-of-scope staging detection on Unix/macOS.
