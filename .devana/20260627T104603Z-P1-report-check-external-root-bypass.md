DEVANA-FINDING: v1
DEVANA-STATE: fixed | P1 | high | security=yes
DEVANA-KEY: src/orchestration.rs:1190 | report-check-external-root-bypass

# report-check reads and trusts a report file outside the configured root

## Finding

`report_path_from_request` (`src/orchestration.rs:1149`) first calls
`repo_path(root, value, "report_path")`, which correctly returns
`PathOutsideRepo` for paths outside the active root. On that error it falls back
to `external_orchid_report_path` (`orchestration.rs:1190`), which canonicalizes
the path and accepts **any** file whose canonical path matches
`<anything>/.orchid/reports/<single-name>` — the configured `root` is never
passed in. `report_check` then reads that external file (`orchestration.rs:1237`)
before any lease check, and computes its relpath against the *external* `.orchid`
parent, not the configured root.

## Violated Invariant Or Contract

Runtime files are confined under `<configured root>/.orchid`. `report_check` must
only read/validate reports under the active root; the `repo_path` rejection at
line 1150 is exactly that boundary decision, which the fallback re-admits.

## Oracle

Every other runtime sink routes through `ensure_under_root` / `repo_path` and
rejects `PathOutsideRepo`. `repo_path` already classified these paths as outside
the repo (the fallback is reached only on that error, line 1156), so the project's
own containment rule is the oracle.

## Counterexample

A worker with normal filesystem access creates
`/tmp/x/.orchid/reports/<lease_id>.md` containing valid frontmatter with
`lease_id = "<lease_id>"` of a real active lease whose stored report path is the
default `.orchid/reports/<lease_id>.md`. Running
`orchid report-check --report /tmp/x/.orchid/reports/<lease_id>.md` against a
project rooted elsewhere: `repo_path` -> `PathOutsideRepo`; the fallback
canonicalizes and accepts the external path, yielding
`rel = ".orchid/reports/<lease_id>.md"` (relative to `/tmp/x`). The
`expected_report` for the lease is the same default string, so the equality check
at `orchestration.rs:1251` passes, and the gate returns the status read from the
attacker-controlled external file.

## Why It Might Matter

The coordinator (a) reads a file outside the configured root and (b) is told the
lease's report is present and valid based on content that never lived under the
root — a confused-deputy spoof of the report-check gate, which is the
pre-completion verification step.

## Proof

Actor-to-resource trace: argv/worker-planted external file -> `report_check`
(1235) -> `report_path_from_request` fallback (1156-1162) ->
`external_orchid_report_path` (root-independent, 1190) -> `read_text` of
out-of-root file (1237) -> equality check passes for default report paths
(1251) -> gate returns valid.

## Counterevidence Checked

Is the fallback an intended worktree feature? It places no constraint tying the
external `.orchid` to the configured root, the same repo, or any worktree
relationship (`external_orchid_report_path` takes only `path`), and the read
happens before lease/identity validation. Even granting worktree intent, the
implementation crosses the read-outside-root boundary far more broadly than that
intent requires. Distinct from `lease-report-path-tampered@runtime.rs:199` (which
is about `report_path_for_lease` trusting lease JSON for the *expected* path);
this is the *request* path escaping root via the fallback, and persists even if
the expected path is pinned, because the external rel still equals the default.

## Suggested Next Step

Pass `root` into `external_orchid_report_path` / `orchid_report_relpath` and
require the resolved `.orchid` parent to equal the configured root (or drop the
fallback and require reports under root).

## Agent Handoff

After working this report, preserve the original finding body. Update line 2
`DEVANA-STATE:` and the final `DEVANA-SUMMARY:` prefix. Keep `DEVANA-KEY:` stable
unless the same finding moved.

## Status Notes

- 2026-06-27: open by Devana. Verified fallback at orchestration.rs:1156-1162 and external_orchid_report_path/orchid_report_relpath never receive or check the configured root.
- 2026-06-27: fixed. The external report-path fallback now requires the resolved out-of-root `.orchid/reports` parent to be a checkout of the *same* git repository as the configured root (shared `--git-common-dir`, new `gitstate::git_common_dir`), rather than admitting any filesystem path shaped `<x>/.orchid/reports/<name>`. This preserves the intended linked-worktree support (commit 0175ba8) while rejecting attacker-planted external reports. Regression test `report_check_rejects_external_orchid_reports_dir_in_unrelated_repo` added (fails without the fix); the existing worktree test was upgraded to a real `git worktree`. Full suite green.

DEVANA-KEY: src/orchestration.rs:1190 | report-check-external-root-bypass
DEVANA-SUMMARY: fixed | P1 | high | report-check's PathOutsideRepo fallback accepts any file shaped <any>/.orchid/reports/<name>, so a planted external file is read outside the configured root and spoofs the pre-completion report-check gate for default report paths.
