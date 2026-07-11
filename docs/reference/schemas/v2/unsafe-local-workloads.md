# `unsafe-local-workloads.json` schema (`v2`)

Schema: [`unsafe-local-workloads.json`](./unsafe-local-workloads.json)

This private artifact contains normalized configured argv and persistent-shell
policy for explicitly enabled unsafe-local workloads, plus configured launcher
items for local VM workloads. `d2bd` resolves it from the hash-verified bundle;
public requests never supply argv.

## Contract notes

- `schemaVersion` is `v2`.
- Unsafe-local workloads are bounded to 256 entries; configured local-VM
  workloads are bounded to 256 entries. The combined private artifact is
  therefore bounded to 512 workload rows.
- Every unsafe-local identity names `runtimeKind` and `providerId` as
  `unsafe-local`.
- `localVmWorkloads` entries require `runtimeKind = "nixos"`.
  `legacyVmName` is optional; first-class workloads use their workload id as
  the backing VM name.
- Every emitted workload has between 1 and 64 private exec/shell items.
  Unsupported public-only item kinds are omitted. Exec argv is bounded,
  non-empty, and NUL-free; shell items require shell policy.
