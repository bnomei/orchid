DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/runtime.rs:31 | runtime-lock-rm-orchid

# `RuntimeLock` drop removes empty `.orchid` marker directory

## Finding

When a `RuntimeLock` is released, its `Drop` implementation removes the lock file, prunes the empty `locks/` directory, then calls `fs::remove_dir` on the entire `.orchid` root. Lock-only commands such as `block`, `release`, and `heartbeat` acquire the lock without calling `ensure_runtime_dirs`, so they can leave `.orchid` containing only `locks/` and delete the whole marker on success.

## Violated Invariant Or Contract

`.orchid/` is documented as the durable runtime root for discovery and goal state. Lock cleanup should remove only the lock artifact, not the runtime marker directory when other workflows may rely on it.

## Oracle

`README.md` Git Ignore section treats `.orchid/` as the runtime home. `paths.rs:discover_orchid_root` walks ancestors looking for `.orchid`. `block` (`orchestration.rs:688-699`) takes `runtime_lock` but never calls `ensure_runtime_dirs`, unlike `lease`/`bud`/`packet`.

## Counterexample

1. Repository has no `.orchid/` (fresh clone or after aggressive cleanup).
2. Coordinator runs `orchid block example T002 --reason "waiting"`.
3. `runtime_lock` creates `.orchid/locks/state.lock` only.
4. `block` succeeds; lock drops.
5. `Drop` removes `state.lock`, removes empty `locks/`, then removes empty `.orchid/`.
6. A subsequent command without `--root` from a subdirectory may fail `discover_orchid_root` and default to CWD instead of the intended repo root.

## Why It Might Matter

Subtle root-discovery regressions after lightweight mutators; goal files under `.orchid/goals/` are safe when present because `remove_dir` fails on non-empty trees, but the marker disappears whenever `.orchid` is lock-only empty.

## Proof

Control-flow trace:

- `RuntimeLock::drop` (`runtime.rs:25-32`): `remove_file(lock)` → `remove_dir(locks/)` → `remove_dir(orch_dir(root))`.
- `block` path never populates `leases/`, `reports/`, or `goals/` before drop.

## Counterevidence Checked

`remove_dir(.orchid)` silently no-ops when subdirectories like `goals/` or `leases/` still contain files. `lease`/`bud`/`packet` call `ensure_runtime_dirs` and recreate runtime subtrees, so they usually leave non-empty `.orchid` behind.

Strongest false-positive reason: callers can pass explicit `--root`. Checked: default discovery path in `root_from_arg` depends on `.orchid` existing (`paths.rs:14-15`).

## Suggested Next Step

Stop removing `orch_dir` in `RuntimeLock::drop`, or only prune the lock subtree; add a test that `block` on a fresh repo leaves `.orchid/` present.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix.

## Status Notes

- 2026-06-25: open by Devana. Initial report from exhaustive `--all` hunt.
- 2026-06-27: reopened. The original `block` path now creates runtime directories, but the root cause remains: `RuntimeLock::drop` still removes `orch_dir(root)`. `research-clean` can delete the last `.orchid/spec-research/<spec>` subtree while holding the lock; pruning cannot remove `.orchid/locks` because `state.lock` is still present, then `Drop` removes the lock file, empty `locks/`, and the now-empty `.orchid` marker.
- 2026-06-27: fixed. `RuntimeLock::drop` now removes only the lock file and empty `locks/` directory, never the `.orchid` root marker. Regression coverage now expects successful close/research-clean cleanup paths to leave `.orchid` present for root discovery after the lock is released.

DEVANA-KEY: src/runtime.rs:31 | runtime-lock-rm-orchid
DEVANA-SUMMARY: fixed | P2 | high | RuntimeLock::drop deletes an empty .orchid directory after lock-only commands like block, breaking discover_orchid_root until the next lease recreates it.
