# Changelog

All notable changes to this project will be documented in this file.

## [0.7.0] - 2026-07-12

### Added

- Added audited mayor acceptance for exact, ambiguous paths already inside a
  frozen lease scope: `complete --accept-attribution <path> --reason <reason>`.
- Packet generation now creates the canonical role report stub when it is absent.
- `next` now routes failed validator evidence directly into a worker repair
  packet with that report as its source.
- Added `orchid --version` for release and installation checks.
- Added verified npm wrapper and GHCR Docker image installation paths for the
  `0.7.0` release assets.

### Changed

- Simplified normal JSON output to compact phase-local facts; use `--explain`
  for record-level diagnostics.
- Moved ordinary attribution reconciliation into the mayor workflow rather than
  a manifest, recovery lease, or reconciliation worker.
- Kept worker packets focused on the task, scope, evidence, and report contract;
  worker routing metadata and policy remain coordinator-side.

### Fixed

- Pre-existing unrelated dirty paths no longer block an otherwise safe lease close.
- Draft reports cannot advance validation, accepted attribution is tied to the
  reviewed content, and retry packet refreshes preserve fresh worker evidence.

## [0.6.0] - 2026-07-11

### Added

- Added versioned lease records and coherent runtime snapshots, giving
  coordinators a stable view of leased task scope, policy, worker settings, and
  Git attribution.
- Added ACK v1 actions and capability metadata so coordinators can follow
  structured next steps without parsing worker packets.
- Added snapshot bindings and role-aware evidence contracts for packets and
  reports, making handoffs traceable to the specific lease context.
- Added `orchid completion-recover --lease <ID>` to safely resume an
  interrupted task completion after its durable intent has been written.
- Added read-only `orchid doctor` runtime health checks and
  `orchid inspect --lease <ID>` lease and handoff inspection.

### Changed

- Refreshed the README and bundled Orchid skills for the current orchestration
  workflow, and updated Cargo dependencies.

### Fixed

- Various bug fixes and reliability improvements.

## [0.5.0] - 2026-06-29

### Added

- Added broader regression coverage for lease lifecycle transitions, goal
  cycles, runtime locks, spec research cleanup, stage planning, scope
  validation, and planner dispatch.

### Changed

- Refined `next`, `ready`, lease, and research command behavior for serial
  fanout, scope-disjoint parallel work, stale leases with reports, exact numeric
  spec selectors, and selected-spec cleanup/stage routing.
- Updated dependency versions in the lockfile.

### Fixed

- Hardened scope and path validation for leases, task IDs, bud instruction
  files, report paths, `.orchid` root discovery, and Git stage planning.
- Fixed goal-cycle decisions around evaluator status, min-delta keep gates,
  protected-surface rechecks, clean-tree retries, baseline capture, and
  keep/discard lifecycle transitions.
- Fixed lease lifecycle edge cases including terminal release/heartbeat,
  completed-task close guards, active-agent reuse, stale runtime lock recovery,
  packet refresh on attach, and task completion rollback on failure.
- Preserved TOML frontmatter arrays, floats, nested tables, and Windows test
  compatibility across the new hardening paths.

## [0.4.0] - 2026-06-17

### Added

- Added goal-loop commands and state for `goal init`, bare `goal`, `goal status`,
  and `goal finish`, including evaluator contracts, baseline capture, keep,
  discard, done, blocked, and budget decisions.
- Added goal evaluator environment variables and durable measurement/result
  traces so goal cycles can be replayed and audited.
- Added first-class worker execution metadata with `worker_reasoning_effort` and
  optional `worker_model` task fields, lease snapshots, packet trusted lines,
  and JSON ACKs for coordinator spawn decisions.
- Added `--worker-reasoning-effort` and `--worker-model` overrides for `lease`
  and `bud`.
- Added `--brief` for compact `ready` and `next` output.
- Added cargo package verification and source-hygiene CI checks for the
  crates.io package contents.
- Added porcelain v2 Git status records to `git-status`, `git-touched`, and
  `git-stage-plan`, including status kinds for renames, deletes, mixed
  staged/unstaged changes, and untracked files.

### Changed

- Made detailed `ready` and `next` ACKs the default, so coordinators see worker
  effort/model before spawning subagents.
- Updated bundled `make-specs` and `orchid` skills to author, validate, and use
  worker execution metadata from ACKs.
- Updated bundled skills to route measurable hypothesis work through native
  `orchid goal` cycles instead of spec or legacy loop scaffolding.
- Switched lease touched/stage planning to structured porcelain v2 status data,
  forcing untracked-file and rename reporting for config-independent staging
  decisions.

### Fixed

- Rejected invalid worker reasoning effort and model values during lint,
  readiness, leasing, and bud creation.
- Blocked automatic staging for cross-scope renames by considering both the old
  and new paths in a porcelain status record.
- Accepted CRLF task frontmatter and normalized goal report paths so the full
  Windows test matrix passes.
- Tolerated RTK-compatible clean status output and merged RTK diagnostic chunks
  when parsing porcelain status streams.

## [0.3.3] - 2026-06-07

### Added

- Resolved numeric spec selectors like `--spec 003` to the unique active `003-*` spec directory, including lease targets such as `orchid lease 003 T001`.

### Fixed

- Accepted global `--pretty` after subcommands, so both `orchid --pretty lint` and `orchid lint --pretty` pretty-print JSON output.

## [0.3.2] - 2026-06-07

### Fixed

- Inferred the Orchid runtime root from the current directory by walking up to the nearest ancestor `.orchid`, so commands run from nested package directories still use the repository runtime state.
- Preserved explicit `--root` arguments without upward runtime discovery, preventing nested projects from accidentally operating on a parent Orchid runtime.
- Allowed `report-check` to validate reports addressed through external `.orchid/reports/<lease>.md` paths, including sibling worktree-relative report paths.

## [0.3.1] - 2026-06-01

### Security

- Hardened lease-id handling so runtime commands reject unsafe IDs before touching lease, packet, report, or lock paths.
- Rejected symlink escapes across repo, runtime, lease, spec, spec-research, sidecar, and atomic-write paths.
- Bound report validation to the lease-derived report path to prevent cross-lease report spoofing.
- Literalized Git stage-plan pathspecs so magic-looking filenames are not interpreted as Git pathspec operators.
- Isolated untrusted task, spec, and bud packet Markdown behind dynamically sized fences before trusted lifecycle instructions.

## [0.2.0] - 2026-05-25

### Added

- Added `orchid bud`, a runtime-only one-shot delegation flow that creates a scoped lease, instruction snapshot, worker packet, report path, and Git baseline without creating durable spec files.
- Added optional lease `agent_id` metadata, `lease-attach-agent`, and `status --agent-id` for discovery while keeping lifecycle commands lease-based.

## [0.1.6] - 2026-05-17

### Changed

- Improved `orchid --help` and subcommand help text for orchestration commands, lease targets, packet roles, and completion metadata.

## [0.1.5] - 2026-05-17

### Changed

- Raised the Rust baseline to 1.85 to keep current `sha1` and `toml` releases.

### Fixed

- Replaced task/runtime files safely on Windows when writing atomically.

## [0.1.4] - 2026-05-17

### Added

- Public release.
