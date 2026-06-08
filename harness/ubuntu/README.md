# Ubuntu harness skeleton (W0b s4)

## What this is

This directory is the W0b non-NixOS Ubuntu harness skeleton for the nixling portability plan. It is hermetic: Nix materializes the layout and smoke-tests the stub JSON, but does not build or boot a live Ubuntu VM image.

W3 lifts this into a real `nixling host check` / host-prepare runner, W4 uses it for the Tier-1 alpha walkthrough, and W9 folds it into onboarding/install flow.

## Inventory

- `host-check-stub.sh` — portable Bash, read-only stub that prints the future host-check JSON shape and always exits 0.
- `expected-host-check.json` — canonical green Ubuntu 24.04 snapshot for human comparison.
- `run-host-check-on-current-host.sh` — developer helper that runs the stub and diffs current output against the snapshot while normalizing expected host-specific details.
- `default.nix` — derivation that copies this harness into `$out/harness/ubuntu` and verifies the stub emits parseable JSON.

## Run locally on Ubuntu 24.04

```bash
bash harness/ubuntu/host-check-stub.sh | jq .
bash harness/ubuntu/run-host-check-on-current-host.sh
```

The stub checks only kernel version, cgroup v2 unified mode, nftables, Nix, KVM, and minijail presence. All other fields are marked `todo-wave-w3`.

## Extending in W3

W3 should replace the stub with the real host-check implementation, add host-prepare remediation, and extend the matrix according to ADR 0008 (supported platforms) once that ADR is integrated. Keep the JSON keys stable so W4/W9 documentation and onboarding can consume the same report shape.

## Explicitly not yet covered

W0b does not provide a live Ubuntu VM image build, KVM/root-access integration testing, host mutation, or cargo-deny/audit Rust toolchain bring-up on the target host. Those land across W3-W9.
