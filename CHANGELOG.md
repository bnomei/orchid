# Changelog

All notable changes to this project will be documented in this file.

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
