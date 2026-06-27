DEVANA-FINDING: v1
DEVANA-STATE: fixed | P2 | high | security=no
DEVANA-KEY: src/runtime.rs:129 | lease-path-metadata-leak

# Load Lease Injects Ephemeral Path Into Persisted JSON

## Finding

`load_lease` and `all_leases` inject a `_path` field into the in-memory lease map for runtime bookkeeping. Any subsequent `save_lease` writes the full `lease.raw()` map, persisting `_path` into `.orchid/leases/<id>.json`.

## Violated Invariant Or Contract

Lease JSON on disk should contain only domain fields. Internal load-time metadata must not leak into durable storage.

## Oracle

`new_active` never sets `_path` (`model.rs:353-382`). `save_lease` serializes `lease.raw()` wholesale (`runtime.rs:162`).

## Counterexample

1. Fresh lease JSON has no `_path` key.
2. `orchid heartbeat l_123` loads lease, mutates `heartbeat_at`, saves.
3. On-disk JSON now includes `"_path": ".orchid/leases/l_123.json"`.
4. Same leak on `release`, `packet`, `complete`, `lease-attach-agent` (any load→mutate→save path).

## Why It Might Matter

Pollutes lease artifacts with implementation details, increases diff noise, and creates a latent field that external tools might misinterpret as contract.

## Proof

Dataflow trace: `load_lease` inserts `_path` → mutator calls `save_lease` → `atomic_write_json(lease.raw())` persists `_path`.

## Counterevidence Checked

Brand-new `lease()` is safe because it never loads before first save. `bind_lease_record_id` patches missing `lease_id` but does not strip `_path`.

## Suggested Next Step

Strip `_path` (and other internal keys) before `save_lease`, or store `_path` outside the serialized map.

## Agent Handoff

After working this report, preserve the original finding body. Update line 2 `DEVANA-STATE: ...` and the final `DEVANA-SUMMARY:` status/priority/confidence prefix. Use one of: `open`, `fixed`, `invalid`, `stale`, `duplicate`, `wontfix`. Keep `DEVANA-KEY:` stable unless the same finding moved. Add dated notes below with evidence checked.

## Status Notes

- 2026-06-27: open by Devana. Initial report written from static source inspection.
- 2026-06-27: fixed. `save_lease` (src/runtime.rs) now clones the lease map and drops any `_`-prefixed key before `atomic_write_json`, so the in-memory `_path` injected by `load_lease`/`all_leases` (confirmed never read back anywhere) no longer leaks into durable lease JSON; the strip is generic for any future internal bookkeeping key. Regression test `save_lease_does_not_persist_internal_path_metadata` added (load→heartbeat→save then assert no `_path`; fails without the fix); full suite green.

DEVANA-KEY: src/runtime.rs:129 | lease-path-metadata-leak
DEVANA-SUMMARY: fixed | P2 | high | load_lease injects _path into the lease map and save_lease persists it, leaking runtime metadata into lease JSON.