# Support matrix

**Diataxis category:** reference.

The current supported product is the local Zone resource plane on a NixOS
host. Host acceptance covers the operator's `/etc/nixos` configuration, d2bd
startup, and Cloud Hypervisor Guest boot.

| Surface | Current status |
| --- | --- |
| NixOS x86_64 host | Primary target; U20 owns real-host acceptance. |
| Cloud Hypervisor Guest | Current local runtime; U20 owns KVM/boot acceptance. |
| QEMU media Provider | Optional local contract; no U20 host acceptance claim. |
| Device, audio, display, shell, storage, and network Providers | Zone-scoped Layer-1 contracts with owner-local tests. |
| Azure Container Apps sandbox | Deferred; test only after U20. |
| Other distributions or hosts | Not part of the current acceptance target. |

## Clean break

This release line is a clean break from v1/v2 configuration and lifecycle.
There is no supported state conversion, host-path adoption, data-retention
promise, or rollback-preservation requirement for older deployments.

## Evidence

Layer-1 evidence uses the public Bazel/Make gates:

```bash
make test-unit
make test-nix-unit
make test-fixture-contracts
make test-rust-supply-chain
make test-policy
make test-drift
make test-flake
make check
```

These gates do not substitute for U20's real-host switch, daemon startup, or
Cloud Hypervisor boot. An unavailable host prerequisite blocks that acceptance
lane rather than becoming a pass.

See [the compatibility policy](./compatibility.md),
[the daemon lifecycle](../explanation/daemon-lifecycle.md), and
[the critical subsystem guidance](../contributing/critical-subsystems.md).
