# orchid

Task-file orchestration helper for coordinating scoped agent work.

`orchid` reads task specs from `specs/<spec-id>/tasks/*.md`, tracks runtime
state in `.orchid/`, and prints JSON ACKs for every command. If `--root` is not
provided, the current working directory is used.

## Usage

```sh
cargo install orchid-cli

orchid ready --spec example --explain
orchid lease example T001 --owner worker:agent_123 --serial
orchid packet --lease l_123 --role worker
```

## Flow

Start by asking for dispatchable work:

```sh
orchid ready --spec example --explain
orchid next --spec example
```

Lease one task before handing it to an agent. The lease records owner, scope,
git baseline, report path, and heartbeat data under `.orchid/`.

```sh
orchid lease example T001 --owner worker:agent_123
orchid packet --lease l_123 --role worker
```

The worker writes its report to the path returned by `lease` or shown in the
packet. A reviewer can then check the report and inspect touched files:

```sh
orchid report-check .orchid/reports/l_123.md
orchid git-touched --lease l_123
```

After validation, mark the task complete and clean up the lease artifacts:

```sh
orchid complete --lease l_123 --verified-by validator:agent_456
orchid close --lease l_123
```

Use `orchid running`, `orchid heartbeat <lease>`, `orchid stale`, and
`orchid release <lease>` to manage work that is still in flight.
