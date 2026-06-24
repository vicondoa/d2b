# Explanation: how `nixling host prepare` works

> Diataxis: explanation. Conceptual model for the host-prepare
> contract. If you are looking for the operator walkthrough, read
> [`docs/how-to/host-prepare.md`](../how-to/host-prepare.md); if you
> are looking for the catalog of privileged operations, read
> [`docs/reference/privileges.md`](../reference/privileges.md).

> The conceptual model below describes the current host-prepare
> design. `host check` and `host prepare --dry-run` /
> `host destroy --dry-run` exercise the broker's read-only audit
> path and are wired live. The mutating `host prepare --apply` /
> `host destroy --apply` verbs are **not yet wired**: the daemon-side
> typed-intent dispatch and bundle resolver that back them are still
> pending, so both return the typed `daemon-down` envelope (exit 1)
> today — use `--dry-run` for now. When the daemon-side dispatch
> ships, the `--apply` verbs will dispatch through the broker reconcile
> ops (`ApplyNftables`, `ApplyRoute`, `ApplySysctl`,
> `UpdateHostsFile`, `ApplyNmUnmanaged`), with broker failures
> surfacing a typed `broker-error` envelope (exit 78). On a Tier 0
> NixOS-legacy host — one with no loadable daemon-owned nixling
> bundle — `host prepare --apply` is refused with
> `tier-0-legacy-uses-nixos-module` (exit 78). The per-VM
> `nixling.vms.<vm>.supervisor` option was removed in v1.1 (ADR 0015);
> every enabled VM is daemon-supervised. See
> [`docs/reference/compatibility.md`](../reference/compatibility.md)
> and ADR 0015.

This page exists so an operator who has never run `nixling host
prepare` can answer four questions before touching the command:

1. What is the broker allowed to mutate on my host?
2. What does it never mutate?
3. What is the safety boundary between `host check`, `host prepare
   --dry-run`, `host prepare --apply`, `host destroy --dry-run`, and
   `host destroy --apply`?
4. If a `host prepare --apply` half-finishes, how do I recover?

## The broker contract

`nixlingd` runs as an unprivileged system user. It performs no
privileged mutation itself; instead, every mutating step is forwarded
to `nixling-priv-broker` over a `0600` private Unix socket
(`priv.sock`). The broker:

- only ever speaks a **closed enum** of typed operations (see
  [`docs/reference/privileges.md`](../reference/privileges.md));
- re-derives every operating path from the trusted bundle, never from
  caller input;
- writes one append-only audit record per decision to
  `/var/lib/nixling/audit/broker-<utc-date>.jsonl` via a pre-opened
  `O_APPEND` fd that is `root:nixlingd 0640`;
- fails closed and audits `decision=denied-unknown` for any operation
  whose subject/scope is absent from the trusted bundle
  (`defaultForUnknown: deny`);
- never holds long-running state — every broker invocation is a
  short-lived per-operation process.

This is the trust boundary. Compromise of `nixlingd` cannot escalate
to arbitrary host mutation beyond the declared broker enum variants.
See [`SECURITY.md`](../../SECURITY.md) for the corresponding
threat-model statement.

## cgroup delegation

The broker creates `/sys/fs/cgroup/nixling.slice` and delegates the
subtree to `nixlingd`. The full 8-step v2 delegation algorithm
(controllers preflight, cpuset propagation, ordered
`cgroup.subtree_control` writes, leaf-only `cgroup.kill`) lives in
[ADR 0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md)
and the reference at
[`docs/reference/cgroup-delegation.md`](../reference/cgroup-delegation.md).

Important conceptual rules:

- `nixling.slice` and intermediate VM cgroup directories are kept
  **process-free**. Only leaf role cgroups hold processes.
- `cgroup.kill` on an ancestor of a daemon-owned leaf is **refused**
  (`cgroup-kill-on-ancestor-refused`). Teardown only touches the leaf
  the daemon declared.
- The broker chowns only the delegated subtree. Host cgroup root is
  never chowned.

## pidfd handoff

The broker forks role payloads with `clone3(CLONE_PIDFD)` (preferred)
or `fork + pidfd_open` (fallback) and ships the resulting CLOEXEC
pidfd to `nixlingd` over the private socket via `SCM_RIGHTS`. The
daemon's supervisor takes ownership in an explicit per-VM/per-role fd
table and uses `pidfd_send_signal` for control (and for VM-scoped
sidecars whose parent is the daemon, `waitid(P_PIDFD)` for reap) —
never raw PID kill/wait.

**SpawnRunner-child supervision.** The broker is the parent of every
`SpawnRunner` child and reaps via `waitid(P_PIDFD)` on its own
pidfd-table entry; the daemon is an observer (it receives a
duplicated pidfd via `SCM_RIGHTS` for BootedNotify identity
verification and lifecycle signalling but does not reap). Per
[ADR 0018](../adr/0018-microvm-nix-removal.md) § "set-booted
race-free serialization" / "broker-as-parent reaping model",
neither the broker nor `nixlingd` claims `PR_SET_CHILD_SUBREAPER`
for the SpawnRunner-child population — making either side a
subreaper would silently re-parent unrelated host processes into
the daemon/broker, breaking the audit/lifecycle model.

## What the host verbs may mutate

The host CLI splits read-only and mutating behaviour across distinct
verbs (canonical contract in
[`docs/how-to/host-prepare.md`](../how-to/host-prepare.md)):

- `nixling host check` — no mutation. Opens read-only file descriptors,
  reads `cgroup.controllers`, walks `/proc/modules`, reads
  `/proc/sys/kernel/modules_disabled`, reads `/sys/class/net/*/`, reads
  `/etc/hosts`, reads existing NetworkManager unmanaged config, hashes
  the current nftables ruleset, and produces a diff. It never opens an
  `O_WRONLY` fd outside its own scratch directory.
- `nixling host prepare --dry-run` — no mutation. Emits the reconcile
  diff the `--apply` form would execute. Mandatory `--dry-run` flag.
- `nixling host prepare --apply` — not yet wired; returns the typed
  `daemon-down` envelope (exit 1) today because the daemon-side
  typed-intent dispatch and bundle resolver are still pending. Once
  wired it mutates exactly the **nixling-owned**
  state. The owned set is identified by ownership markers — see
  ADRs 0011/0012/0013 — typically:

  - a `# nixling-managed begin` / `# nixling-managed end` block in
    `/etc/hosts`;
  - a `# nixling-managed begin` / `# nixling-managed end` block in
    `/etc/NetworkManager/conf.d/00-nixling-unmanaged.conf`;
  - the `inet nixling` nftables table (and only that table — peer
    tables, even foreign ones in the `inet` family, are never flushed);
  - the `nixling.slice` cgroup subtree;
  - bridges / TAPs whose name starts with the broker-derived
    `nl-`/`nlv-` prefix from the IfName hash scheme (ADR 0012).

  Mandatory `--apply` flag. Foreign-owned state in any of those
  domains is left alone; discovering a foreign ownership marker where
  nixling expects its own is fail-closed (`path-safety-violation`,
  `nm-managed-foreign-conflict`, `foreign-nft-rule-preserved`).
- `nixling host destroy --dry-run` — no mutation. Reports the
  nixling-owned set that `--apply` would withdraw.
- `nixling host destroy --apply` — not yet wired; returns
  `daemon-down` (exit 1) today. Once wired it withdraws the
  nixling-owned state
  in reverse dependency order. Mandatory `--apply` flag. Refuses if
  any matching VM is still running (`vm-still-running-refused`). Never
  touches foreign ownership markers.
- `nixling host doctor --read-only` — no mutation. Surfaces
  load-bearing findings without acting. Mandatory `--read-only`
  flag.
- `nixling host install --dry-run` — no mutation. Prints the
  synthesized 5-step installer preview.
- `nixling host install --apply` — live daemon → broker
  `RunHostInstall` path. Broker failures surface exit 78
  (`broker-error`) instead of falling back to bash.

## NetworkManager / systemd-networkd coexistence

Nixling cannot guarantee VM-network correctness without **exclusive
ownership** of every interface it creates. A foreign manager that
toggles MTU, IPv6 settings, RA, autoconf, or IP assignments mid-startup
can silently break a VM whose declared network depends on the
host-prepare IPv6-off ordering (ADR 0012). The broker therefore
treats NM/networkd coexistence as a fail-closed predicate.

**When the NM unmanaged config is written.** Per the 5-step IPv6-off
ordering (ADR 0012), the broker writes
the unmanaged drop-in
(`/etc/NetworkManager/conf.d/00-nixling-unmanaged.conf`, marker block
`# nixling-managed begin` / `# nixling-managed end`) **pre-create** —
before any `RTM_NEWLINK` for the nixling bridge or TAP. The broker
then triggers `nmcli general reload conf` (NM ≥ 1.20) or
`systemctl reload NetworkManager.service` (older NM). `nmcli
connection reload` is explicitly **not** used because it only reloads
connection profiles and ignores the `conf.d/*.conf` device-management
snippets where the unmanaged scope lives. Only once the reload returns
and the broker confirms via `nmcli -t -f DEVICE,STATE device status`
that the nixling ifname is `unmanaged` does it proceed to link create.

**How systemd-networkd hosts are handled.** systemd-networkd is
**detection-only** — the broker never writes a `*.network` or
`*.link` file. The host-prepare path probes for an active
systemd-networkd that is managing the nixling ifname prefix
(`nl-`/`nlv-`) by reading `/run/systemd/network/*.link` and the
`networkctl status` JSON output. If the prefix is being actively
managed, the broker refuses to create the link unless a
configured-unmanaged file (typically
`/etc/systemd/network/00-nixling-unmanaged.network` shipped by the
operator's NixOS module or distro packaging) is present with the
matching prefix in its `[Match] Name=` block. Without that explicit
acknowledgement the operator's networkd installation would race the
broker for ownership, so the broker refuses with
`nm-managed-foreign-conflict` (the same error code as the NM path).

**Why coexistence fails closed.** A foreign manager can:

- re-enable `accept_ra=1` on a nixling bridge mid-boot, which would
  inject SLAAC addresses against the per-link `disable_ipv6=1`
  invariant;
- assign an IPv4 address to a TAP the broker just created, which
  would compete with the in-VM DHCP client and silently break the
  declared per-env LAN;
- override MTU, breaking the per-env MTU/MSS clamp;
- re-toggle `bridge-nf-call-iptables`, allowing iptables/ip6tables
  to route around the `inet nixling` policy.

Nixling cannot detect every drift mid-startup, so it requires
exclusive ownership up front and fails closed on any sign of
contention.

**What happens if NM is absent.** Clean host — neither
`/run/NetworkManager/` nor `/var/lib/NetworkManager/` exists, and
`systemctl is-active NetworkManager.service` returns failure. The
broker records the detection result as `manager_detected: none` in
the `ApplyNmUnmanaged` audit record, writes nothing under
`/etc/NetworkManager/`, and proceeds with link create + IPv6-off
ordering. This is the typical daemon-mode case on hosts that do not
run NetworkManager.

Cross-references:

- [ADR 0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md)
  — IPv6-off sysctl set, hash-derived IfName, bridge-port defaults;
- [`docs/how-to/host-prepare.d/network.md`](../how-to/host-prepare.d/network.md) — operator walkthrough of the 5-step ordering.

## Tier behavior

Tier behavior is fully described in
[`docs/reference/support-matrix.md`](../reference/support-matrix.md).
Conceptually:

- **NixOS hosts**: `host prepare --apply` is refused. The NixOS
  module owns the prepare contract, while the daemon still owns
  runtime supervision.
- **Ubuntu 24.04 hosts**: `host check` and `host prepare --dry-run`
  enumerate the full reconcile plan (including the five-step
  IPv6-off ordering with NetworkManager 1.46 reload, ADR 0012);
  `--apply` is not yet wired and returns `daemon-down` (exit 1)
  today. When the daemon-side dispatch ships it will dispatch through
  the broker reconcile ops (`ApplyNftables`, `ApplyRoute`,
  `ApplySysctl`, `UpdateHostsFile`, `ApplyNmUnmanaged`) per ADR 0015.
- **Best-effort hosts**: `host check` and `--dry-run` still apply
  every fail-closed check; `--apply` carries the same pending
  disposition (`daemon-down`, exit 1, today) and, once wired, the
  same exit-78 typed-envelope behaviour on failure. The audit log is
  the system of record across the support matrix.

## Mixed legacy/daemon operation

A host may still contain legacy-systemd state or daemon-managed
state, but the broker only runs in the daemon-managed path.
Single-writer conflicts between the legacy systemd path and a
daemon-backed VM surface as `single-writer-conflict` and are
fail-closed.

## Recovery runbook

The mutating `host prepare --apply` / `host destroy --apply` verbs
are **not yet wired** — they return the typed `daemon-down` envelope
(exit 1) today, so this runbook describes the recovery flow that
applies once the daemon-side dispatch ships. If `host prepare --apply`
fails partway through, the operator runbook is:

1. **Pause the broker**: an admin uid runs
   `nixling admin broker --pause`. The broker stops accepting new
   operations; in-flight operations finish or time out.
2. **Inspect the audit log**:
   `nixling audit tail /var/lib/nixling/audit/broker-<utc>.jsonl`
   (admin-only; `authz-audit-requires-admin` otherwise). Every
   decision since the last successful `host check` is recorded with
   `decision`, `operation_fields`, and an `error_kind` if applicable.
3. **Re-run `host check`** to compute the diff between the trusted
   bundle and the half-applied host state. Because every prepare step
   is idempotent and ownership-marker-keyed, the diff faithfully
   represents what is left to do (or what is foreign-owned and will
   not be touched).
4. **Apply the residual diff** by re-running `host prepare --apply`,
   or **roll back** the nixling-owned state with `host destroy --apply`
   followed by an admin-approved fresh `host prepare --apply`.
5. **Rotate** any role-scoped secrets that the audit log surfaces as
   touched (the current broker enum has no secret-bearing variants;
   any future ones will be flagged `secret: yes` in
   [`docs/reference/privileges.md`](../reference/privileges.md)).
6. **Resume the broker** with `nixling admin broker --resume`.

For the security-policy framing — how this runbook integrates with
GitHub Security Advisory disclosure — read [`SECURITY.md`](../../SECURITY.md).

## Net-route preflight & network reconcile

> There is no `nixling-net-route-preflight.service` host singleton.
> The daemon owns the equivalent self-check directly inside
> `nixlingd`'s startup path; see
> [ADR 0015](../adr/0015-daemon-only-clean-break.md).

On every startup, `nixlingd` probes each env's LAN bridge under
`/sys/class/net/<bridge>/operstate` (existence + `operstate != down`).
The startup result is diagnostic and history-only: cold boots can
legitimately begin with the env bridges absent because the autostarted
net VMs own the host-prep DAG that creates and raises them. A failed
startup preflight therefore does **not** pre-skip the env's net VM or
workloads. If the net VM itself fails during autostart, the normal
autostart dependency gate marks that env's workloads degraded (not
failed), while read-only verbs continue to work.

The daemon persists a small jsonl history at
`<daemon-state-dir>/net-route-preflight-history.jsonl` (atomic rename;
retention `32` records) for diagnostics and manual recovery evidence.
The explicit recovery verb remains useful when bridge/route state is
known-bad outside the normal net-VM autostart path.

### Recovery: `nixling host reconcile --network --apply`

This is the focused mutating recovery verb (admin-only). It
re-runs the broker-side network slice of `host prepare`
(`ApplyNftables(host)` + per-env `ApplyRoute` + per-env
`ApplySysctl`) without starting any VM, and on success resets
the persistent consecutive-failure counter. It does NOT touch
`/etc/hosts` or the NetworkManager unmanaged file — those
remain scoped to a full `host prepare`.

```console
# Plan the reconcile (no mutation):
$ nixling host reconcile --network --dry-run

# Apply (admin):
$ nixling host reconcile --network --apply
```

The verb honours the standard `--dry-run` / `--apply` mandatory
pair and emits the typed error `--apply-or-dry-run-required`
(exit 78) when neither is set. The `net-route-preflight-degraded`
typed envelope (exit 66) uses this section as its remediation target
when a caller surfaces explicit network-preflight degradation.

## Cross-references

- [`docs/how-to/host-prepare.md`](../how-to/host-prepare.md) — operator how-to.
- [`docs/reference/privileges.md`](../reference/privileges.md) — broker enum operation matrix.
- [`docs/reference/cgroup-delegation.md`](../reference/cgroup-delegation.md) — cgroup algorithm.
- [`docs/reference/inet-nixling-chains.md`](../reference/inet-nixling-chains.md) — nftables chain layout.
- [`docs/reference/support-matrix.md`](../reference/support-matrix.md) — tier matrix.
- [`SECURITY.md`](../../SECURITY.md) — trust-boundary threat-model delta.
- ADRs [0011](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md),
  [0012](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md),
  [0013](../adr/0013-w3-firewall-coexistence-policy.md),
  [0014](../adr/0014-w3-modules-devices-runner-shape.md).
