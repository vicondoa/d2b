# Privilege and broker contract

**Diataxis category:** reference.

d2bd is unprivileged. All host mutation is dispatched to the privileged
`d2b-broker` through a closed, typed operation set. The broker resolves
private paths and identities from the trusted Zone bundle; callers never send
raw host paths, argv, credentials, PIDs, cgroup paths, or device paths.

## Root-visible units

```text
d2bd.service
d2b-broker.socket
d2b-broker.service
```

The broker socket is systemd-owned and socket-activated. The broker adopts
the inherited fd and does not self-bind, self-chown, or self-fchmod it.

## Operation families

| Family | Examples | Owner |
| --- | --- | --- |
| Runner launch | `SpawnRunner`, `OpenPidfd` | broker |
| Cgroup | delegated leaf open, placement, leaf kill | broker |
| Network | TAP/bridge, sysctl, nftables, NetworkManager ownership | Network Provider + broker |
| Storage | store-view sync, gc roots, anchored locks, fd transfer | Volume Provider + broker |
| Device | TPM, USBIP, GPU, audio, security-key access | Device Provider + broker |
| Activation | Guest closure publication and generation commit | activation Provider + broker |
| Identity | key rotation, host-key trust, bounded audit export | identity Provider + broker |

Every operation carries an authenticated Zone/Resource scope, Provider and
controller generation, revision or operation identity, and an explicit
allow/deny disposition. Unknown operations and missing scope are denied.

## Runner security

The broker verifies the signed Provider/template and artifact commitment,
places the runner in a delegated leaf scoped to its Zone, Resource, and role,
and hands a pidfd to d2bd over `SCM_RIGHTS`. The broker remains the sole
parent and reaper. Raw PID comparisons are limited to restart adoption before
the pidfd is reopened.

## Ownership and cleanup

Host-mutable paths use one named repair owner. The broker preserves foreign
nftables, NetworkManager, systemd-networkd, cgroup, TPM, socket, and device
state. It refuses parent-cgroup kills, recursive store ACL changes, broad
runtime sweeps, and ownership-marker replacement.

## Audit

Each decision is recorded in the broker's append-only audit stream with
bounded Zone, Resource, Provider, operation, outcome, and redacted error
metadata. Secrets, raw paths, argv, terminal bytes, and private handles are
never audit labels or public response fields.

## Verification

```bash
d2b host check --json
d2b host doctor --read-only
d2b audit --json
make test-policy
make test-unit
```

See [the daemon lifecycle](../explanation/daemon-lifecycle.md),
[critical subsystem invariants](../contributing/critical-subsystems.md), and
[ADR 0015](../adr/0015-daemon-only-clean-break.md).
