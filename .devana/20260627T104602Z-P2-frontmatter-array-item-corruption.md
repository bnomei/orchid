DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/taskfile.rs:293 | frontmatter-array-item-corruption

# Non-scalar / typed array frontmatter items are silently corrupted on rewrite

## Finding

`dump_toml_value`'s Array arm stringifies every element with
`value_to_string(item)` (`src/taskfile.rs:293`) and wraps the result in
`quote_toml_string`. `value_to_string` (`src/core.rs:202-209`) has an
`other => Some(other.to_string())` fallthrough, so a non-scalar item (an object,
a nested array) is rendered as compact JSON and a numeric item is rendered as a
quoted string. Unlike the already-filed float / top-level-table cases, this does
**not** error — it silently rewrites the value with a different type/structure.

## Violated Invariant Or Contract

A frontmatter load -> dump round-trip must preserve each value's structure and
type, or fail loudly. Array elements must not be silently coerced to quoted
strings.

## Oracle

`split_frontmatter` accepts any valid TOML, and `lint`
(`orchestration.rs:1314-1342`) never constrains array element types, so such a
value loads and passes lint. The dump path is the only contract for writing the
file back, and `dump_toml_value` already chooses to *error* on unsupported scalar
shapes (float, table) — the Array arm violating that by silently coercing is the
mismatch.

## Counterexample

A task whose frontmatter contains a TOML array of inline tables, e.g.
`covers = [{ id = "T1" }]`, loads as `Array[Object]`. The next `complete`
(`orchestration.rs:669`) or `block` re-dumps the whole map via
`write_task_frontmatter`, rewriting `covers` as `covers = ["{\"id\":\"T1\"}"]` —
the table becomes a quoted JSON blob. Secondary: a typed int array
`extra = [1, 2]` is rewritten as `extra = ["1", "2"]` (int -> string).

## Why It Might Matter

Any status-mutating command silently destroys structured or typed array metadata
a user or tool stored in task frontmatter; the corruption is invisible (no error,
no warning) and irreversible once written.

## Proof

Dataflow trace: `covers` -> `load_task` -> `toml_to_json` (Array of Object,
taskfile.rs:372-379) -> `write_task_frontmatter` ->
`dump_frontmatter_with_array_styles` -> `dump_toml_value` Array arm
(taskfile.rs:291-295) -> `value_to_string(Object)` returns a JSON string
(core.rs:208) -> `quote_toml_string`. Output type differs from input; no error
path.

## Counterevidence Checked

"These fields are never arrays-of-tables in practice." There is no schema/type
validation at load or in lint constraining array element types, so a
hand-authored or tool-written task file can carry them, and the corruption fires
unconditionally on the next write. Scalars handled correctly: bool, empty array,
and i64/u64 have dedicated dump arms (taskfile.rs:281-289) and round-trip fine.
Distinct from `frontmatter-float-undumpable@282` and
`frontmatter-nested-undumpable@373` (both hard errors); this is silent data loss.

## Suggested Next Step

In the Array arm, dump each item through `dump_toml_value` recursively (so
non-scalar/typed items either serialize correctly or hit the existing
unsupported-type error) instead of stringifying via `value_to_string`.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2
`DEVANA-STATE:` and the final `DEVANA-SUMMARY:` prefix. Keep `DEVANA-KEY:` stable
unless the same finding moved.

## Status Notes

- 2026-06-27: open by Devana. Verified Array arm calls value_to_string (taskfile.rs:293) and core.rs:208 fallthrough returns JSON string for objects.
- 2026-06-27: fixed. `dump_toml_value`'s Array arm now dumps each element recursively via `dump_toml_value(item, Some(Inline))` instead of `value_to_string` + `quote_toml_string`. Arrays of inline tables and typed-number arrays now round-trip with their structure/type preserved (or hit the existing unsupported-type error) rather than being silently rewritten as quoted strings by complete/block. Regression test `task_frontmatter_round_trips_typed_and_structured_array_items` added (verified failing under the old logic, passing under the fix); full suite green.

DEVANA-KEY: src/taskfile.rs:293 | frontmatter-array-item-corruption
DEVANA-SUMMARY: fixed | P2 | high | dump_toml_value stringifies array items via value_to_string, whose object/number fallthrough silently rewrites arrays of tables as quoted JSON and int arrays as string arrays, so complete/block corrupt structured frontmatter without error.
