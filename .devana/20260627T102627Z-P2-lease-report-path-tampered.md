DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/runtime.rs:199 | lease-report-path-tampered

# Lease Report Path Field Is Not Pinned To Canonical Location

## Finding

`report_path_for_lease` returns whatever `report_path` string is stored in lease JSON when non-empty. It is not bound to the canonical `.orchid/reports/{lease_id}.md` path. Tampered or redirected `report_path` values pass `repo_path` and drive `report-check`, packet rendering, and `next` reports_ready.

## Violated Invariant Or Contract

Each lease's report should live at `.orchid/reports/{lease_id}.md` as set at lease creation (`orchestration.rs:325-328`). Validation should not follow arbitrary redirects in lease JSON.

## Oracle

Lease creation sets `report_path` to `reports_dir/{lease_id}.md`. `report_check` compares `expected_report` from lease JSON, not recomputed canonical path (`orchestration.rs:1249-1257`).

## Counterexample

1. Active lease `l_123` normally uses `.orchid/reports/l_123.md`.
2. Lease JSON is edited to `"report_path": "specs/example/tasks/T001.md"` (existing repo file).
3. `orchid report-check .orchid/reports/l_123.md` fails path mismatch, but `orchid report-check specs/example/tasks/T001.md` succeeds if frontmatter `lease_id` matches.
4. `close_lease_files` still deletes only `reports/l_123.md`, not the tampered path.

## Why It Might Matter

Misdirected report validation can accept content from unrelated repo files while coordinators believe the canonical report path was checked.

## Proof

Dataflow trace: tampered `report_path` in lease JSON → `report_path_for_lease` → `report_check` path equality on relative strings, not canonical `{lease_id}.md` invariant.

## Counterevidence Checked

`report_check` still requires matching `lease_id` in frontmatter and loads the lease record. Cross-lease swap is caught when paths differ from each lease's stored `report_path`. This report is intra-lease redirection within the repo.

## Suggested Next Step

Ignore stored `report_path` for validation and always resolve `.orchid/reports/{lease_id}.md`, or reject divergent values on load.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: fixed. `report_path_for_lease` (src/runtime.rs) now always resolves the canonical `.orchid/reports/{lease_id}.md` and no longer honors the `report_path` stored in lease JSON. Lease creation always writes that canonical value and `close_lease_files` already deletes exactly that path, so a tampered/redirected stored value can no longer drive report-check/packet rendering at an arbitrary in-repo file. Worktree report-check is unaffected (the external request path still resolves to the same canonical relpath). Regression test `report_check_ignores_tampered_lease_report_path` added (fails without the fix); full suite green.

DEVANA-KEY: src/runtime.rs:199 | lease-report-path-tampered
DEVANA-SUMMARY: fixed | P2 | high | report_path_for_lease trusts lease JSON instead of pinning .orchid/reports/{lease_id}.md, so report-check can validate redirected paths.