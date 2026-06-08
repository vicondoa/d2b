# 0002. Non-root nixlingd plus minimal privileged broker

- Status: Accepted
- Date: 2026-05-25
- Wave: W0b
- Plan slice: "`nixlingd` is not a broad root daemon. Long-lived VM/sidecar/helper payloads must execute as declared non-root role users under minijail."
- Companion ADRs: [ADR 0001](0001-systemd-free-vm-orchestration.md), [ADR 0003](0003-minijail-provisioning-and-sandbox-interface.md), [ADR 0005](0005-network-firewall-and-tap-model.md), ADR 0007

## Context

The v0.4.0 baseline runs per-VM lifecycle through NixOS systemd units,
root-owned activation, and polkit-mediated launcher authorization. That
model works on NixOS but couples host mutation, supervision, and
authorization to root systemd services rather than to a portable control
plane.

The portability plan splits the Rust control plane into a caller-owned
CLI, a dedicated non-root `nixlingd`, and a minimal
`nixling-priv-broker` that performs only non-delegable host mutations.
The daemon owns orchestration state and public API policy, while the
broker is intentionally not a root shell and must validate every request
against its own trusted bundle.

AGENTS.md identifies polkit and the `nixling-launcher` group as the
current privilege boundary, and it warns that mistakes in launcher policy
can either lock everyone out or authorize too much. W0b keeps the group
concept but moves authorization to filesystem Unix sockets, peer
credentials, and an explicit operation matrix.

The plan's broker contract requires AF_UNIX IPC, closed Rust operation
enums, fd-oriented resource transfer, safe path resolution, root-owned
audit logging, and a pause or kill-switch for post-compromise recovery.
Those requirements bound what the privileged process may do even if the
non-root daemon is compromised.

## Decision

1. `nixlingd` runs as a dedicated non-root system user and is itself confined by a minijail role profile.
2. `nixling-priv-broker` is the only long-lived root process, listens only on AF_UNIX `SOCK_SEQPACKET` at `/run/nixling/priv.sock`, is reachable only by the `nixlingd` uid, and exposes a closed Rust operation enum with no command strings.
3. The broker independently opens its trusted root-owned manifest bundle, verifies owner, mode, version, and hash, and re-derives paths, uids, gids, capabilities, and resource identifiers from its own copy.
4. Privileged resources including TAP fds, `/dev/kvm`, `/dev/vhost-*`, cgroup dirfds, and pre-bound Unix sockets are returned to the daemon with `SCM_RIGHTS` rather than as path strings.
5. Broker filesystem access uses `openat2`, `O_NOFOLLOW`, and `RESOLVE_BENEATH` style resolution, and ownership or mode changes use fd-based `fchown` and `fchmod` only.
6. Public authorization uses `nixling-launcher` for daily lifecycle operations and `nixling-admin` for destructive, host-prepare, manifest-activation, and key-rotation operations, with peer identity from `SO_PEERCRED` and supplementary groups from `getgrouplist()` or an equivalent system-account query.
7. The broker writes an allow-and-deny audit log that is root-owned and append-only where practical, and it exposes admin-only pause and resume or kill-switch RPCs for post-compromise recovery.
8. Cgroup v2 delegation is pinned to a broker-created `/sys/fs/cgroup/nixling.slice`. The broker prepares `cgroup.subtree_control` on every ancestor needed to delegate that subtree, validates that `nixlingd` receives only the non-root delegation it needs, and fails closed during host check when the delegation cannot be made safe. W3 host-check implementation is the gate that turns this ADR pin into enforced startup behavior.

## Consequences

1. Positive: A compromised daemon cannot ask the broker to execute arbitrary shell commands or trust daemon-supplied paths.
2. Positive: Privileged kernel objects become capabilities transported as fds, which reduces path races and long-lived device-node exposure.
3. Positive: The launcher/admin split preserves the current operator model while making destructive and secret-bearing operations default-deny.
4. Negative: Broker implementation and tests must cover manifest skew, path traversal, symlink races, fd passing, and denied enum variants from the first privileged wave.
5. Neutral: ADR 0003 defines how the daemon and broker are jailed, ADR 0005 enumerates the network operations the broker must support, and ADR 0008 owns the kernel floor for `openat2`, pidfd, cgroup, and networking assumptions.

## Alternatives considered

- Run `nixlingd` as root: rejected because it would concentrate orchestration, parsing, sockets, and host mutation in one broad root daemon.
- Keep using polkit for every lifecycle mutation: rejected because it remains systemd/NixOS-centric and does not cover the private broker contract.
- Let the daemon pass paths and command fragments to a helper: rejected because that recreates a root shell boundary and makes path validation unenforceable.
- Use abstract Unix sockets: rejected because filesystem sockets provide inspectable ownership, modes, and cleanup semantics under `/run/nixling`.

## Kernel resource pins

- The broker owns creation and preparation of `/sys/fs/cgroup/nixling.slice`.
- The broker enables the required controllers through ancestor
  `cgroup.subtree_control` files before handing the delegated subtree to
  `nixlingd`.
- The daemon and long-lived payloads do not create root-owned cgroup
  hierarchy outside the delegated nixling slice.
- Host check fails closed if `/sys/fs/cgroup/nixling.slice` cannot be
  created, if any ancestor `cgroup.subtree_control` write fails, or if
  the resulting delegation would require `nixlingd` to retain root.
- W3 host-check implementation is the required enforcement gate for
  this cgroup v2 delegation contract.

## References

- plan.md, "Rust control plane"
- plan.md, "Socket and authorization model"
- plan.md, "Privileged broker contract"
- plan.md, "Secrets, logs, and audit"
- AGENTS.md, "Polkit / launcher group"
- AGENTS.md, "Don'ts (security-relevant)"
- [SECURITY.md](../../SECURITY.md)
