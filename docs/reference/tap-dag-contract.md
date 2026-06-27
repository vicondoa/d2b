# Tap DAG contract

> Status: canonical. Operator-facing contract for the per-VM tap
> interfaces the daemon brings up via the host-prep DAG. The
> implementation modules cited below are the source of truth; this
> doc exists to pin the operator-observable shape so drift between
> code and doc is detectable by
> [`tests/tap-dag-contract-doc-eval.sh`](../../tests/tap-dag-contract-doc-eval.sh).

The host-prep DAG ([host-prep-dag.md](./host-prep-dag.md))
contains a single tap-creation step per VM:
`<vm>:bring-up-tap-interface`. This document defines the
**naming**, **ordering**, **ownership**, **handoff mode**,
**reconciliation**, and **failure** semantics that step must
satisfy.

## Canonical naming convention

Per-VM tap interfaces are NOT named after the VM. The framework
uses a deterministic hash-derived shape so every name fits inside
the kernel `IFNAMSIZ - 1 = 15` byte limit regardless of how long
the VM name is.

Authoritative emitter: [`d2b_host::ifname::derive_from_env_vm`](../../packages/d2b-host/src/ifname.rs).

| Resource          | Pattern                                  | Example         |
| ----------------- | ---------------------------------------- | --------------- |
| Per-VM tap        | `<prefix>t<HASH8>`                       | `d2b-tWORK010`   |
| Per-env bridge    | `<prefix>b<HASH8>`                       | `d2b-bWORK000`   |

Where:

- `<prefix>` is the configurable d2b ifname prefix. Default
  `d2b-` (see `d2b_host::ifname::DEFAULT_PREFIX`). Prefix must
  satisfy `validate_prefix` (≤ 8 bytes, ends in `-`, alphabet
  `[A-Za-z0-9_-]`).
- The single role tag character is `t` for taps and `b` for
  bridges (`d2b_host::ifname::{TAP_TAG, BRIDGE_TAG}`).
- `<HASH8>` is the FNV-1a 64-bit hash of
  `env || 0x1f || vm.unwrap_or("") || 0x1e || role_tag`, base32-
  encoded (Crockford alphabet without `I`/`L`/`O`/`U`), truncated
  to `HASH_SUFFIX_LEN = 8` chars.
- With the default `d2b-` prefix the total length is 12 bytes
  (`d2b- + t + 8 hash chars`), well within IFNAMSIZ-1.

Reverse-lookup: [`d2b_host::ifname::looks_d2b_owned`]
recognises any name matching `<prefix>{b|t}<8 base32 chars>` and
is the basis for the daemon's "is this link mine?" check during
reconciliation. Foreign links are ignored fail-closed: the daemon
never touches a tap that does not pass `looks_d2b_owned` with
the configured prefix.

The mapping `(env, vm?, role) → derived_ifname` is emitted into
the trusted bundle as `host.json::ifNameMapping`
(`d2b_core::host::IfNameMapping`) and re-validated by the
broker via `d2b_host::ifname::detect_collisions`. Two distinct
keys hashing to the same derived ifname is a fail-closed emitter
error (`IfNameError::IfNameCollision`) — there is no operator
escape hatch.

## DAG position and ordering

`<vm>:bring-up-tap-interface` sits between two other host-prep
steps; its position in the per-VM DAG is fixed by
[`d2b_host::host_prep_dag::build_host_prep_dag`](../../packages/d2b-host/src/host_prep_dag.rs):

```text
apply-nftables-rules  --->  bring-up-tap-interface  --->  pre-open-vhost-net-fd
                                       |
                                       v
                              (per-VM process DAG;
                               SpawnRunner happens here)
```

**Upstream gates** (must succeed before tap creation):

1. `<vm>:apply-nftables-rules` — declared `depends_on`. The
   `inet d2b` table must hold the per-VM/per-busid carve-outs
   before the tap is attached to the bridge; this prevents a
   race where packets traverse the bridge before the firewall
   policy is in place.
2. `ApplyNmUnmanaged` (host-wide, not per-VM) — enforced inside
   the broker handler via [`TapCreateGate`](../../packages/d2b-priv-broker/src/ops/tap.rs).
   The broker REFUSES `CreateTapFd` / `CreatePersistentTap`
   unless the prior `ApplyNmUnmanaged` op recorded either
   `NmUnmanagedOutcome::Applied` or
   `NmUnmanagedOutcome::NotApplicableNmAbsentConfiguredCoexist`.
   Missing gate surfaces as broker error
   `"nm-unmanaged-pre-create-required"` which the daemon wraps
   in `HostPrepStepFailed { step_id: <vm>:bring-up-tap-interface, ... }`.

**Downstream consumers** (must wait for tap creation):

1. `<vm>:pre-open-vhost-net-fd` — declared `depends_on`. Vhost-net
   fd cannot be opened against a tap that does not yet exist.
2. Per-VM process DAG `SpawnRunner` (cloud-hypervisor exec). The
   process DAG cannot begin until the entire host-prep DAG has
   reported success — see
   [host-prep-dag.md §"Failure semantics"](./host-prep-dag.md#failure-semantics).

**Tap → bridge attachment.** As part of the broker's tap-create
ioctl sequence (`d2b_host::netlink::ipv6_off_sequence`), the
tap is bound to its env bridge with `bridge_slave` master + the
per-role port flag matrix from
[`d2b_host::bridge_port::BridgePortFlagSet::defaults_for`](../../packages/d2b-host/src/bridge_port.rs).
The post-create netlink readback verifies every flag; any drift
fails the step fail-closed and the tap is torn down before
returning.

**Teardown order (stop DAG).** The reverse ordering applies:
runner SIGTERM/SIGKILL via supervisor pidfd → broker
`ApplyNftables` to remove per-VM carve-outs → broker tap teardown
(release fd for `TapFd`, or `DeleteTap` for `PersistentTap`). A tap
is never removed while its CH is still live.

## Handoff modes

The bundle declares the host-wide handoff mode in
`host.json::chConfig.netHandoffMode` per the
[`d2b_core::host::ChNetHandoffMode`](../../packages/d2b-core/src/host.rs)
enum. Both modes use the same hash-derived ifname; only the fd
ownership transitions differ.

### `TapFd` (preferred)

Broker op: `BrokerRequest::CreateTapFd`.

1. Broker opens `/dev/net/tun` and runs `TUNSETIFF` against the
   derived ifname.
2. Broker applies the per-link IPv6-off sysctl sequence
   (`disable_ipv6 = 1`, `accept_ra = 0`, `autoconf = 0`,
   `addr_gen_mode = 1`) BEFORE attaching the tap to the bridge —
   so no IPv6 SLAAC/link-local packet ever leaves a d2b tap.
3. Broker sets MAC + MTU per the resolved bundle intent.
4. Broker attaches the tap to its env bridge with the
   per-role port flags (`SetBridgePortFlags`).
5. Broker returns the tap fd to the daemon via `SCM_RIGHTS`.
6. Daemon retains the fd until the per-VM process DAG dispatches
   `SpawnRunner`. The fd is passed to cloud-hypervisor as the
   `tap=fd:<N>` argv form.
7. After CH adopts the fd (CH `vm.create` returns success), the
   broker and daemon release their retained copies; the kernel
   reference count drops to one (CH), and tap teardown happens
   automatically when CH exits.

The runner (cloud-hypervisor) runs **without** `CAP_NET_ADMIN`
in this mode. The tap fd is the only handle CH ever sees; CH
cannot create, rename, or delete taps.

### `PersistentTap` (fallback)

Broker op: `BrokerRequest::CreatePersistentTap`. Used when the
packaged CH binary lacks the `tap-fd` capability (probed at
bundle build via `host.json::chConfig`).

1. Broker performs steps 1–4 of the `TapFd` flow above.
2. Broker calls `TUNSETPERSIST = 1` so the tap survives broker
   process exit.
3. Broker calls `TUNSETOWNER(uid)` + `TUNSETGROUP(gid)` with the
   **exact** uid/gid of the runner that `SpawnRunner` will
   exec — this is the per-VM runner uid (graphics VMs use
   `d2b-<vm>-gpu` uid + `kvm` group; headless workloads use
   the `microvm` per-VM uid; per the networking
   guidance, the broker MUST bind to the same uid/gid the
   spawn intent will use).
4. Broker returns the derived ifname (no fd) in
   `TapReadyResponse`. The runner opens `/dev/net/tun` itself
   and re-attaches by name; the TUNSETOWNER/TUNSETGROUP bindings
   gate the open.

The runner needs read/write access to `/dev/net/tun` (mediated
by device cgroup + the persistent-tap owner uid/gid). It still
has no `CAP_NET_ADMIN`; the tap was fully configured by the
broker.

## Tap-fd ownership transitions

For the `TapFd` mode, the per-VM fd lifecycle is:

| Stage                      | Fd holder(s)                          | Notes |
| -------------------------- | ------------------------------------- | ----- |
| 1. After `CreateTapFd`     | broker                                | broker still holds an `OwnedFd` after `TUNSETIFF` |
| 2. SCM_RIGHTS to daemon    | broker + daemon (kernel refcount 2)   | broker drops its copy after `sendmsg` returns; daemon retains across the rest of host-prep |
| 3. SCM_RIGHTS to CH        | daemon + CH (kernel refcount 2)       | daemon passes the fd in `SpawnRunner`'s SCM payload; CH adopts during `vm.create` |
| 4. After CH adoption       | CH only (kernel refcount 1)           | daemon closes its copy on `vm.create` ack; the broker has been closed since stage 2 |
| 5. CH exit                 | none                                  | kernel reaps the tap on last-fd close (transient tap) |

The unix-socket carrying the SCM_RIGHTS payload is the broker
private socket (`/run/d2b/priv.sock`) for broker→daemon, and
the daemon-to-runner control socket for daemon→CH. The socket
modes are documented in [privileges.md](./privileges.md); the
relevant invariant for this contract is that both sockets remain
confined to the daemon/broker control plane rather than the public
launcher surface.

For the `PersistentTap` mode there is no per-handoff fd
movement; the binding is uid/gid + persistence on the kernel
tap, and the runner opens `/dev/net/tun` itself.

## Reconciliation on daemon restart

On startup, before serving the first mutating verb, the daemon
walks the host's link table and reconciles per-VM taps against
the bundle:

1. Enumerate every link whose name matches
   `looks_d2b_owned(name, prefix)`.
2. For each, look up the corresponding bundle mapping by
   derived ifname (`host.json::ifNameMapping`).
3. If a tap exists but no bundle mapping does: it is an
   **orphan** tap. The daemon dispatches the broker's
   tap-teardown op for it. (Transient `TapFd` taps cannot
   survive broker death; persistent taps from a now-removed
   VM can.)
4. If a bundle mapping exists but the live tap does not: the
   daemon does NOT re-create eagerly. The tap is re-created on
   the next `vm start <vm>` via the normal host-prep DAG.
5. If a tap exists for a VM that is also alive (pidfd present
   in `/run/d2b/state/runner-pidfds/<vm>`): the daemon
   leaves it alone. Reconciliation never tears down a tap whose
   CH is still live.

Foreign links (no `d2b-` prefix, or `d2b-` prefix but failing
`looks_d2b_owned`) are ignored. The daemon NEVER deletes a
link it does not own.

## Failure semantics

Tap creation failure surfaces as the typed
[`HostPrepStepFailed`](../../packages/d2b-host/src/host_prep_dag.rs)
envelope. The envelope is produced by the daemon and shaped as:

```json
{
  "step_id": "<vm>:bring-up-tap-interface",
  "op_kind": "CreateTapFd",
  "broker_error": "<raw broker error string>"
}
```

The `op_kind` is the string returned by
`HostPrepStepKind::BringUpTapInterface.broker_op_name()` which
is always `"CreateTapFd"` regardless of whether the host's
declared mode is `TapFd` or `PersistentTap` — the broker
dispatches by request variant, not by step-kind label. The
broker error string carries the concrete cause:

| Cause                                                | `broker_error` substring |
| ---------------------------------------------------- | ------------------------ |
| Bundle did not declare a tap intent for this VM      | `tap intent not found for vm` |
| Prior `ApplyNmUnmanaged` did not record success      | `nm-unmanaged-pre-create-required` |
| `/dev/net/tun` open failed                           | `open /dev/net/tun:` |
| `TUNSETIFF` rejected the derived ifname              | `tun-setiff:` |
| Bridge attach / port-flag readback drift             | `bridge-port-readback:` |
| `TUNSETOWNER`/`TUNSETGROUP` failed (persistent only) | `tun-setowner:` / `tun-setgroup:` |

Per the host-prep DAG's fail-fast contract, a failed
`bring-up-tap-interface` aborts the rest of the per-VM host-prep
DAG; the per-VM process DAG is not invoked, and CH is not
spawned. Subsequent `vm start <vm>` attempts re-resolve the DAG
from scratch and re-dispatch.

The daemon's broker-error wrapping does NOT cache failure: there
is no "tap is poisoned" daemon state. If the operator fixes the
underlying cause (e.g. removes a foreign NetworkManager
ownership marker so `ApplyNmUnmanaged` can succeed), the next
`vm start` proceeds.

## Cross-references

- **Host-prep DAG**: [host-prep-dag.md](./host-prep-dag.md) —
  parent contract for the per-VM DAG this step lives in.
- **Bridge port flags**: [`packages/d2b-host/src/bridge_port.rs`](../../packages/d2b-host/src/bridge_port.rs).
- **Tap broker handlers**: [`packages/d2b-priv-broker/src/ops/tap.rs`](../../packages/d2b-priv-broker/src/ops/tap.rs).
- **Derived ifname emitter**: [`packages/d2b-host/src/ifname.rs`](../../packages/d2b-host/src/ifname.rs).
- **Host config DTO**: [`packages/d2b-core/src/host.rs`](../../packages/d2b-core/src/host.rs) — `ChNetHandoffMode`, `IfNameMapping`, `HostChConfig`.
- **Failure envelope**: [`packages/d2b-host/src/host_prep_dag.rs`](../../packages/d2b-host/src/host_prep_dag.rs) — `HostPrepStepFailed`.
- **Drift gate**: [`tests/tap-dag-contract-doc-eval.sh`](../../tests/tap-dag-contract-doc-eval.sh) — fails if any of the above implementation symbols diverge from this document.
- **Related references**:
  - [host-prep-dag.md](./host-prep-dag.md) — parent host-prep DAG scaffold.
  - [stop-dag-reconcile.md](./stop-dag-reconcile.md) — reverse-order teardown contract.
  - [per-vm-state-ownership.md](./per-vm-state-ownership.md) — per-VM uid/gid leaf ownership the persistent-tap mode binds to.
