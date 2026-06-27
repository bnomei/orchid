DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | medium | security=no
DEVANA-KEY: src/runtime.rs:47 | stale-runtime-lock-deadlock

# A crashed process leaves an unrecoverable runtime lock

## Finding

`runtime_lock` acquires `.orchid/locks/state.lock` with
`OpenOptions::create_new(true)` (`src/runtime.rs:47`) and writes a payload of
`{"pid", "created_at"}` (`runtime.rs:59-63`). The lock is removed only in
`RuntimeLock::drop` (`runtime.rs:25-32`). No code anywhere reads `pid` or
`created_at`, probes liveness, checks age, or force-clears the lock. If a process
holding the lock is terminated without running `Drop` (SIGKILL, OOM kill, power
loss), the lock file persists and every subsequent command fails with
`RuntimeLockBusy` forever.

## Violated Invariant Or Contract

A mutual-exclusion lock must be recoverable: a lock left by a dead owner must not
block all future commands indefinitely. The embedded `pid`/`created_at` payload
is precisely the data needed for liveness/age-based recovery, signaling that
recoverability was the intended contract.

## Oracle

The recorded `pid`/`created_at` (runtime.rs:62) exist to enable a staleness/owner
check. `rg "created_at|RuntimeLockBusy"` shows the file is created and the busy
error is mapped, but the payload is never read back — recovery was designed but
not implemented.

## Counterexample

1. `orchid lease ...` (or any mutating command) calls `runtime_lock`, creating
   `.orchid/locks/state.lock`.
2. The process is SIGKILLed / OOM-killed / loses power before `Drop` runs (Drop
   runs only on normal return or unwind, not on SIGKILL).
3. `state.lock` remains on disk.
4. Every later command's `runtime_lock` hits `ErrorKind::AlreadyExists` ->
   `RuntimeLockBusy`. The orchestrator is wedged until a human manually removes
   the file.

## Why It Might Matter

A single abnormal termination takes the whole orchestration offline with no
self-recovery and no operator-facing remediation command; the only fix is manual
`rm .orchid/locks/state.lock`, which an automated coordinator cannot do safely on
its own.

## Proof

Control-flow / event-order: `create_new(true)` is purely advisory-on-existence;
the sole removal path is `Drop` (runtime.rs:27), which does not run on SIGKILL.
No reader of `pid`/`created_at` exists in the tree, so no staleness reaper or
force path can reclaim the lock.

## Counterevidence Checked

"Commands are short-lived, so the crash window is tiny." True, but the
consequence is a hard, manual-only lockout, and the recorded pid/created_at prove
recoverability was the intended contract. Distinct from
`runtime-lock-rm-orchid@runtime.rs:31` (Drop over-removing `.orchid`); this is the
missing acquire-side staleness/liveness check. Concurrency itself is handled
correctly (one winner via `create_new`); the defect is non-recovery after a dead
owner.

## Suggested Next Step

On `AlreadyExists`, read the payload and reclaim the lock when the recorded pid
is not alive or `created_at` is older than a bound; or add an explicit
`--force` lock-clear path.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2
`DEVANA-STATE:` and the final `DEVANA-SUMMARY:` prefix. Keep `DEVANA-KEY:` stable
unless the same finding moved.

## Status Notes

- 2026-06-27: open by Devana. Verified pid/created_at written at runtime.rs:62 and never read; removal only via Drop.
- 2026-06-27: fixed. `runtime_lock` (src/runtime.rs) now, on `AlreadyExists`, reads the lock's age (recorded `created_at`, falling back to file mtime when the payload is missing/corrupt) and reclaims it when older than `LOCK_STALE_AFTER_SECONDS` (300s — far longer than any legitimate sub-second hold). Reclaim is race-free via an atomic per-process `rename` claim (only one racer wins; losers fall back to a normal busy result). A SIGKILL/OOM/power-loss between acquire and Drop no longer wedges every later command, while a fresh lock is still respected. The mtime fallback keeps a mid-creation lock young (never wrongly reclaimed). Regression test `runtime_lock_reclaims_stale_lock_but_respects_fresh_one` added (fails without the fix); full suite green.

DEVANA-KEY: src/runtime.rs:47 | stale-runtime-lock-deadlock
DEVANA-SUMMARY: fixed | P2 | medium | runtime_lock writes pid/created_at but never reads them, so a SIGKILL/OOM/power-loss between acquire and Drop leaves state.lock on disk and every later command fails RuntimeLockBusy with no recovery path.
