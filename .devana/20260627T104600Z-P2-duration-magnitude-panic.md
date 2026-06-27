DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/core.rs:258 | duration-magnitude-panic

# parse_duration aborts the process on a large-magnitude duration

## Finding

`parse_duration` validates a duration string and is expected to return
`Err(InvalidDuration)` for anything it cannot accept. After the `len < 2`,
integer-parse, and unknown-unit guards, it builds a `chrono::TimeDelta` with the
*panicking* constructors `TimeDelta::seconds/minutes/hours/days(amount)`
(`src/core.rs:258-261`) using a fully caller-controlled `i64 amount`. These
constructors panic ("TimeDelta::days out of bounds") when the resulting value
exceeds chrono's `TimeDelta` range, so a syntactically valid but huge magnitude
crashes the process instead of returning a structured error.

## Violated Invariant Or Contract

`parse_duration -> OrchResult<TimeDelta>` must yield `InvalidDuration` for every
duration it cannot represent; an out-of-range magnitude is just another invalid
duration and must not panic the binary.

## Oracle

The function's own contract: three explicit `Err(InvalidDuration)` branches
(`core.rs:247-265`) and the tests at `core.rs:285-291` assert it returns a
stable error code for bad input. chrono 0.4 documents `TimeDelta::days/hours/
minutes/seconds` as panicking on out-of-bounds; the non-panicking `try_*`
variants are not used here.

## Counterexample

`orchid stale --older-than 99999999999999d` (or `next --older-than`, or
`goal init --max-duration 99999999999999d`). `amount = 99999999999999` parses as
`i64`; `TimeDelta::days` overflows chrono's seconds range (~9.2e15 s) and panics.
`9999999999999999s` is an equivalent minimal trigger.

## Why It Might Matter

A coordinator that forwards an oversized duration receives an unhandled panic /
non-zero abort with no JSON ACK, instead of the documented `invalid_duration`
failure. Any automation parsing the JSON contract breaks, and the panic message
goes to stderr instead of the structured error channel.

## Proof

Dataflow + dependency-source: `request.older_than` / `max_duration` ->
`parse_duration` (`orchestration.rs:517`, `orchestration.rs:725`, `goal.rs:111`,
`goal.rs:280`, `runtime.rs:180`) -> `amount: i64` (parse succeeds for ~1e14
values) -> `TimeDelta::days(amount)` -> chrono `expect()` panic. No
`checked_*`/`try_*`/saturating wrapper exists on lines 258-261.

## Counterevidence Checked

Could the magnitude be rejected earlier? Only `len < 2`, integer-parse failure,
and unknown unit are guarded; magnitude is never bounded. Could chrono saturate?
No — the bare `days/hours/minutes/seconds` constructors are
`expect(try_*(...), "...out of bounds")`. Distinct from `negative-duration@245`
(sign semantics) and `duration-utf8-panic@253` (non-ASCII split-at-byte panic);
this is a valid-ASCII, valid-unit, in-`i64` value that overflows `TimeDelta`.

## Suggested Next Step

Use `TimeDelta::try_days/try_hours/try_minutes/try_seconds` (or `checked_*`) and
map `None` to `Err(InvalidDuration)`.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2
`DEVANA-STATE:` and the final `DEVANA-SUMMARY:` prefix. Keep `DEVANA-KEY:` stable
unless the same finding moved.

## Status Notes

- 2026-06-27: open by Devana. Verified TimeDelta panicking constructors at core.rs:258-261 with i64 amount and no magnitude guard.
- 2026-06-27: fixed. `parse_duration` (src/core.rs) now builds the TimeDelta with chrono's non-panicking `try_seconds/try_minutes/try_hours/try_days` and maps `None` to `InvalidDuration`, so a large-magnitude (in-i64 but out-of-TimeDelta-range) value like `99999999999999d` returns the structured error instead of aborting the process. Regression tests added: unit `duration_parser_rejects_out_of_range_magnitude_without_panicking` and CLI `stale_rejects_out_of_range_duration_with_structured_error` (the CLI one fails — no JSON ACK — without the fix); full suite green.

DEVANA-KEY: src/core.rs:258 | duration-magnitude-panic
DEVANA-SUMMARY: fixed | P2 | high | parse_duration builds TimeDelta with chrono's panicking day/hour/minute/second constructors, so a large-magnitude duration like "99999999999999d" aborts the process instead of returning InvalidDuration.
