DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/orchestration.rs:1364 | status-agent-terminal-lease

# Status By Agent Id Returns Terminal Leases

## Finding

`status --agent-id` calls `status_for_agent`, which scans all lease files matching `agent_id` with no filter for active status. A released or completed lease still attached to an agent id returns `ok` with packet and report paths as if it were current work.

## Violated Invariant Or Contract

README says `agent_id` is for discovery and recovery; operational commands should use `lease_id`. Discovery should prefer active leases or clearly distinguish terminal state from in-flight work.

## Oracle

`orchestration.rs:1364-1410` filters only by `lease.agent_id()`, not `lease.status().is_active()`. README lines 206-207 on agent_id discovery semantics.

## Counterexample

1. Worker completes `l_old`; lease JSON has `status: completed`, `agent_id: agent_123`.
2. Coordinator runs `orchid cleanup --completed` later, but lease file still exists briefly.
3. `orchid status --agent-id agent_123` returns `ok` with `status: completed`, `packet`, and `report` paths.
4. Coordinator spawns recovery worker against a finished lease.

## Why It Might Matter

Agent discovery ACKs can mislead automation into treating terminal leases as active assignments, especially before cleanup removes completed files.

## Proof

Control-flow trace: `all_leases` → filter `agent_id` match → return first match with no `is_active()` guard.

## Counterevidence Checked

`AgentLeaseAmbiguous` errors when multiple matches exist, but does not prefer active over terminal. `ensure_agent_id_available` blocks reuse while any lease file exists (separate `agent-id-reuse` finding).

## Suggested Next Step

Filter to active leases first; if only terminal matches exist, return a distinct error or `status: none_active`.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: fixed. `status_for_agent` now filters agent_id matches to active leases first: terminal-only matches return `agent_lease_not_found` (with `terminal_leases` detail) instead of presenting a completed/released lease as current work, and ambiguity is judged over active matches only (so a reusable agent_id with one lingering terminal lease + one new active lease resolves to the active one). Regression test `status_agent_id_ignores_terminal_lease` added (fails without the fix); full suite green.

DEVANA-KEY: src/orchestration.rs:1364 | status-agent-terminal-lease
DEVANA-SUMMARY: fixed | P2 | high | status --agent-id returns completed or released leases as the current assignment with no active-only filter.