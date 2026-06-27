DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/goal.rs:196 | nan-min-delta-corrupts-goal

# goal init --min-delta nan writes unparseable goal.toml and wedges the goal

# Finding

`goal init` accepts `--min-delta` as a bare `f64` (`src/cli.rs:146`) with no
value parser, and `GoalInitRequest::new` (`src/goal.rs:103-126`) stores it
without a finite/NaN check. `GoalContract::to_toml` serializes it with
`format!("minimum_delta = {}", self.minimum_delta)` (`src/goal.rs:196`). Rust's
`Display` for `f64::NAN` emits the literal `NaN`, which is **not** valid TOML
(the spec requires lowercase `nan`). Every later goal command re-reads the
contract via `toml::from_str`, which then rejects the file.

## Violated Invariant Or Contract

A value accepted at the CLI boundary and persisted must round-trip through its
own serializer/deserializer. `to_toml` must emit TOML that `GoalContract::read`
can parse back.

## Oracle

`goal init` returns success and writes `.orchid/goal/<id>/goal.toml`, but
`orchid goal`, `goal status`, and `goal finish` all call `current_goal` ->
`GoalContract::read` -> `toml::from_str` on that file. The round-trip is the
implicit contract; the init success ACK promises a usable goal.

## Counterexample

`orchid goal init --goal g --metric m --direction lower-is-better --min-delta nan
--hypothesis h --max-iterations 5 --max-duration 30m`

`f64::from_str("nan")` yields `f64::NAN` (clap uses `FromStr`). The contract is
written with the line `minimum_delta = NaN`. The next `orchid goal` /
`goal status` / `goal finish` fails to parse `.orchid/goal/<id>/goal.toml` and
the goal loop is permanently broken until the file is hand-edited.

## Why It Might Matter

A single mistyped flag durably corrupts persisted goal state on the very first
init; the improvement loop cannot start, resume, or even report status, and the
failure surfaces only on the *next* command, not at init time.

## Proof

Entrypoint -> sink -> reload: argv `--min-delta nan` -> `f64::NAN` (no validator,
cli.rs:146) -> `GoalInitRequest::new` stores verbatim (goal.rs:118) ->
`to_toml` writes `minimum_delta = NaN` (goal.rs:196) -> `GoalContract::read`
`toml::from_str` rejects `NaN` on the next command.

## Counterevidence Checked

Is NaN rejected by a value parser? `cli.rs:145-146` is a plain `min_delta: f64`,
no `value_parser`/range. Does `new()` validate? `goal.rs:97-126` validates only
`max_duration`; `minimum_delta` is stored as-is. Does toml accept `NaN`? The TOML
spec mandates lowercase special floats, and Rust's `Display` emits capitalized
`NaN`, so the round-trip is broken. `inf` writes as lowercase `inf` (valid TOML,
harmless since min_delta is unenforced); only `NaN` corrupts. Distinct from
`min-delta-unenforced@goal.rs:899` (that gate is never read); this is durable
state corruption at write time.

## Suggested Next Step

Reject non-finite `--min-delta` at the CLI boundary or in `GoalInitRequest::new`
(`if !minimum_delta.is_finite()` -> error), or serialize via a TOML-correct float
formatter.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2
`DEVANA-STATE:` and the final `DEVANA-SUMMARY:` prefix. Keep `DEVANA-KEY:` stable
unless the same finding moved.

## Status Notes

- 2026-06-27: open by Devana. Verified f64 has no clap validator and to_toml uses Display formatting at goal.rs:196.
- 2026-06-27: fixed. `GoalInitRequest::new` (src/goal.rs) now rejects a non-finite `minimum_delta` (`!is_finite()` → error) before any file is written, so `--min-delta nan` (which Display-serializes as the invalid-TOML literal `NaN` and wedges every later goal command) and infinity are caught at init with a structured error and no goal.toml left behind. Regression test `goal_init_rejects_non_finite_min_delta` added (covers `nan` and `inf`; fails without the fix). Note: `--min-delta -inf` is separately rejected by clap (leading `-` parsed as a flag). Full suite green.

DEVANA-KEY: src/goal.rs:196 | nan-min-delta-corrupts-goal
DEVANA-SUMMARY: fixed | P2 | high | --min-delta nan flows unvalidated into to_toml's `format!("minimum_delta = {}")`, writing the invalid-TOML literal `NaN`, so every later goal command fails to parse goal.toml and the goal is permanently wedged.
