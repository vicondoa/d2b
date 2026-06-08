# Host-prep DAG (P2)

> Status: **P2 ph2-dag-host-prep**. The Rust scaffold + typed broker
> request variants land in this commit; the live broker handlers for
> `SeedDnsmasqLease`, `BindMountFromHardlinkFarm`,
> `OwnershipMatrixCheck`, and `SshHostKeyPreflight` follow in later
> P2/P3 commits.

The host-prep DAG is the daemon-side replacement for the per-VM
systemd templates that the framework used to ship to do host
preparation immediately before starting a VM:

| Retired systemd template                                     | Replaced by host-prep DAG step                                              |
| ------------------------------------------------------------ | --------------------------------------------------------------------------- |
| `microvm-tap-interfaces@<vm>.service` (tap fd creation)      | `BringUpTapInterface` (broker `CreateTapFd` / `CreatePersistentTap`)        |
| `microvm-tap-interfaces@<vm>.service` (vhost-net fd open)    | `PreOpenVhostNetFd` (broker `OpenVhostNet`)                                 |
| `microvm-setup@<vm>.service` (dnsmasq lease seed)            | `SeedDnsmasqLease` (broker `SeedDnsmasqLease`, P2 stub)                     |
| `microvm-setup@<vm>.service` (store-view bind)               | `BindMountFromHardlinkFarm` (broker `BindMountFromHardlinkFarm`, P2 stub)   |
| ad-hoc nft drop-in installs                                  | `ApplyNftablesRules` (broker `ApplyNftables`, W3 live)                      |
| per-VM ownership matrix preflights (today: scattered)        | `OwnershipMatrixCheck` (P2 sibling agent)                                   |
| ad-hoc ssh-host-key drift checks (today: missing)            | `SshHostKeyPreflight` (P2 sibling agent)                                    |

Together these are the **only** per-VM host-prep work the daemon
performs before invoking the per-VM process DAG
(`nixlingd::supervisor::dag`). Everything else (swtpm flush,
virtiofsd spawn, cloud-hypervisor spawn, …) belongs to the process
DAG.

## Canonical step set

The complete step set for a workload VM (10 steps after P2fu1
kernel-r1-1 — see "Canonical ordering" below for the dependency
edge diagram):

1. `<vm>:ssh-host-key-preflight` — preflight, no deps
2. `<vm>:ownership-matrix-check` — preflight, no deps
3. `<vm>:apply-nm-unmanaged` — preflight, no deps; P2fu1 step that
   marks the per-VM tap-parent bridge as unmanaged in NetworkManager
   BEFORE tap creation so NM doesn't race the broker `TUNSETIFF` +
   `dev set master`
4. `<vm>:apply-nftables-rules` — deps: `ssh-host-key-preflight`,
   `ownership-matrix-check`, `apply-nm-unmanaged`
5. `<vm>:bring-up-tap-interface` — deps: `apply-nftables-rules`
6. `<vm>:apply-sysctl` — P2fu1 step running AFTER tap creation so
   `/proc/sys/net/ipv4/conf/<ifname>/` entries exist; deps:
   `bring-up-tap-interface`
7. `<vm>:set-bridge-port-flags` — P2fu1 step pinning `learning off`,
   `flood off`, `mcast_to_unicast off`; deps: `apply-sysctl`
8. `<vm>:pre-open-vhost-net-fd` — deps: `set-bridge-port-flags`
   (waits for the bridge-port flags to be pinned before vhost-net
   adopts the path)
9. `<vm>:bind-mount-from-hardlink-farm` — deps: `ownership-matrix-check`

Net VMs add one step:

10. `<vm>:seed-dnsmasq-lease` — deps: `apply-nftables-rules`

## Dependency graph

```text
ssh-host-key-preflight   ownership-matrix-check   apply-nm-unmanaged
         \                       /     \                  /
          \                     /       \                /
           +---> apply-nftables-rules <-+----------------+
                       |  \
                       |   +---> bind-mount-from-hardlink-farm  (from ownership-matrix-check)
                       |
                       +---> bring-up-tap-interface
                                    |
                                    +---> apply-sysctl
                                                |
                                                +---> set-bridge-port-flags
                                                              |
                                                              +---> pre-open-vhost-net-fd
                       |
                       +---> seed-dnsmasq-lease            (net VMs only)
```

The DAG is statically derived from each VM's properties in the
trusted bundle by `nixling_host::host_prep_dag::build_host_prep_dag`.
There is no operator-tunable knob — the step set is a deterministic
function of the bundle.

## Broker-op contract

Each step dispatches **exactly one** typed broker op. The daemon
never names raw paths/uids/argv on the wire; every step carries an
opaque `BundleStepRef` (per W3fu1 H1 security-1) that the broker
resolves against its own copy of the bundle.

| Step                          | Broker op (`nixling_ipc::broker_wire::BrokerRequest::…`) | Implementation status                                                                  |
| ----------------------------- | -------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `ApplyNmUnmanaged`            | `ApplyNmUnmanaged`                                       | Live (W3); P2fu1 added as DAG step BEFORE tap creation so NetworkManager doesn't race the broker's `TUNSETIFF` + `dev set master` and pull the link down. Replaces the `00-nixling-unmanaged.conf` materializer leaf of `microvm-setup@<vm>.service`. |
| `BringUpTapInterface`         | `CreateTapFd` (or `CreatePersistentTap`)                 | Live (W3 s2); P2 vm_start wiring deferred to per-VM intent rows                        |
| `ApplySysctl`                 | `ApplySysctl`                                            | Live (W3); P2fu1 added as DAG step AFTER tap creation so per-tap `/proc/sys/net/ipv4/conf/<ifname>/` entries exist. Replaces the sysctl-apply leaf of `microvm-setup@<vm>.service`. |
| `SetBridgePortFlags`          | `SetBridgePortFlags`                                     | Live (W3); P2fu1 added as DAG step AFTER `ApplySysctl` so `learning off`, `flood off`, `mcast_to_unicast off` reflect the final per-tap config. Replaces the `bridge link set` leaf of `microvm-tap-interfaces@<vm>.service`. |
| `PreOpenVhostNetFd`           | `OpenVhostNet`                                           | Live (W3 s4); P2 vm_start wiring deferred to per-VM intent rows                        |
| `SeedDnsmasqLease`            | `SeedDnsmasqLease`                                       | Typed scaffold (returns `Unimplemented { target_wave: "P2" }`); live handler P2/P3     |
| `BindMountFromHardlinkFarm`   | `BindMountFromHardlinkFarm`                              | Typed scaffold (returns `Unimplemented { target_wave: "P2" }`); live handler P2/P3     |
| `ApplyNftablesRules`          | `ApplyNftables`                                          | Live (W3); resolves via `BundleResolver::find_nft_intent`                              |
| `OwnershipMatrixCheck`        | `OwnershipMatrixCheck`                                   | Live (P2 ph2-p2-ownership-matrix); typed daemon-side enforcer in `nixlingd::ownership_preflight` |
| `SshHostKeyPreflight`         | `SshHostKeyPreflight`                                    | Live (P2 ph2-p2-ssh-host-key-preflight); typed daemon-side check in `nixlingd::ssh_host_key_preflight` |

**Canonical ordering (P2fu1 kernel-r1-1)**:

```text
SshHostKeyPreflight \
OwnershipMatrixCheck \         (all three are siblings — no upstream deps)
ApplyNmUnmanaged   /
       │
       ▼
ApplyNftablesRules           (depends on all three preflights)
       │
       ▼
BringUpTapInterface          (depends on ApplyNftablesRules)
       │
       ▼
ApplySysctl                  (depends on tap so /proc/sys/.../<ifname>/ exists)
       │
       ▼
SetBridgePortFlags           (depends on ApplySysctl)
       │
       ▼
PreOpenVhostNetFd            (depends on SetBridgePortFlags — bridge flags pinned first)

[net VMs only] SeedDnsmasqLease    (depends on ApplyNftablesRules)
BindMountFromHardlinkFarm           (depends on OwnershipMatrixCheck)
```

This sequence is the daemon-side equivalent of the systemd dependency
graph `microvm-setup@<vm>.service` + `microvm-tap-interfaces@<vm>.service`
emitted today. The per-VM systemd templates are scheduled for removal
in the P6 deletion sweep.

## Failure semantics

Step failure is **fail-fast**. The first step whose broker op fails
aborts the host-prep DAG — subsequent steps are not dispatched, and
the per-VM process DAG (`supervisor::dag`) is not invoked. The
operator sees the typed envelope

```json
{
  "code": "BrokerOperationFailed",
  "summary": "host-prep step <vm>:<kind> (<BrokerOp>) failed: <broker error>",
  "remediation": "…"
}
```

derived from `HostPrepStepFailed { step_id, op_kind, broker_error }`
in `nixling_host::host_prep_dag`.

A failed host-prep DAG does **not** poison the daemon: subsequent
`vm start` attempts re-resolve the DAG from scratch and re-dispatch
the failed step. There is no daemon-side caching of step success.

## Execution gate

Today the daemon **logs** the planned host-prep DAG on every VM
start (so the gate set `tests/host-prep-dag-eval.sh` and operators
can audit the planned step set) but only dispatches the broker ops
when `NIXLING_HOST_PREP_DAG_EXECUTE=1` is set in the daemon's
environment. This gate is removed once the P2/P3 broker handlers
listed above stop returning `Unimplemented`.

## Cross-references

- **Operating manual**: `AGENTS.md` §"Critical subsystems — handle
  with care" row "Control plane (W2+)" — this DAG lives in the
  control-plane scope and the operator-facing failure envelope is
  emitted by `nixlingd::dispatch_broker_vm_start`.
- **Plan**: `~/.copilot/session-state/<id>/plan.md` §"Phase 2:
  daemon-side host-prep replaces per-VM systemd templates" and
  task `ph2-dag-host-prep`.
- **Module docs**: `packages/nixling-host/src/host_prep_dag.rs` —
  authoritative step kind definitions and topo-sort algorithm.
- **Sibling P2 agents**:
  - `ph2-p2-ownership-matrix` (owns the `OwnershipMatrixCheck`
    broker handler + `nixling_host::ownership_matrix`).
  - `ph2-p2-ssh-host-key-preflight` (owns the `SshHostKeyPreflight`
    broker handler).
