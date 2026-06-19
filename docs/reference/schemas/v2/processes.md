# `processes.json` schema (`v2`)

Schema: [`processes.json`](./processes.json)

`processes.json` is the per-VM process DAG contract from ADR 0004. It tells
the daemon which long-lived roles exist, how they depend on each other, and
which minijail profiles constrain them.

## Top-level fields

- `schemaVersion` — schema directory/version for this artifact.
- `vms` — one process graph per VM.

## Per-VM process graph

Each VM row carries the runner/process inventory needed by dry-run and future
apply paths:

- declared processes/roles (Cloud Hypervisor or QEMU media runners, virtiofsd
  shares, swtpm, sidecars)
- dependency edges / topological ordering
- shutdown ordering metadata
- profile references into `minijail-profile.json`
- role-local intent IDs and runner metadata used by the broker

## Contract notes

- `vm start --dry-run` / `vm stop --dry-run` snapshots are derived from this
  process DAG.
