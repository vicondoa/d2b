# `unsafe-local-workloads.json` schema (`v2`)

Schema: [`unsafe-local-workloads.json`](./unsafe-local-workloads.json)

This private artifact contains normalized configured argv and persistent-shell
policy for explicitly enabled unsafe-local workloads, plus configured launcher
items for local VM workloads. `d2bd` resolves it from the hash-verified bundle;
public requests never supply argv.

## Contract notes

- `schemaVersion` is `v2`.
- Workload and launcher-item counts are bounded.
- Every identity names `runtimeKind` and `providerId` as `unsafe-local`.
- `localVmWorkloads` entries require `runtimeKind = "nixos"`.
  `legacyVmName` is optional; first-class workloads use their workload id as
  the backing VM name.
- Exec argv is bounded, non-empty, and NUL-free; shell items require shell
  policy.
