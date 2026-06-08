# How to: prepare a nixling host

> Diataxis: how-to. Operator-facing walkthrough for `nixling host
> prepare`. This document is assembled from the per-scope fragments
> under `docs/how-to/host-prepare.d/*.md`; keep the assembled view
> and fragments in sync when editing.

> `nixling host check`, `nixling host prepare --dry-run`,
> `nixling host destroy --dry-run`, and
> `nixling host doctor --read-only` exercise the broker's read-only
> audit path. The mutating `--apply` verbs (`host prepare --apply`,
> `host destroy --apply`) dispatch through the broker reconcile ops
> (`ApplyNftables`, `ApplyRoute`, `ApplySysctl`, `UpdateHostsFile`,
> `ApplyNmUnmanaged`) on supported non-NixOS hosts. On hosts where
> the NixOS module still owns prepare, the verb returns the typed
> `tier-0-legacy-uses-nixos-module` envelope (exit 78). Broker
> failures surface as `broker-error` (exit 78); daemon-unreachable
> surfaces `daemon-down` (exit 1). See
> [`docs/reference/compatibility.md`](../reference/compatibility.md)
> and [ADR 0015](../adr/0015-daemon-only-clean-break.md).

`nixling host prepare` is the single operator command that takes a
nixling host from "I just rendered the bundle" to "every declared VM
can boot". It runs as an unprivileged user against the daemon socket,
which forwards mutating steps to the privileged broker; only the
broker holds capabilities, and every mutation goes through a typed,
closed-enum operation with an append-only audit record.

The host CLI is split across seven verbs; the canonical contract is:

| Verb | Mutates host | Required flag |
| --- | --- | --- |
| `nixling host check` | no | n/a — read-only inventory + diff |
| `nixling host prepare --dry-run` | no | `--dry-run` mandatory; reports only |
| `nixling host prepare --apply` | yes (broker reconcile ops per ADR 0015) | `--apply` mandatory |
| `nixling host destroy --dry-run` | no | `--dry-run` mandatory; reports only |
| `nixling host destroy --apply` | yes (broker reconcile ops per ADR 0015) | `--apply` mandatory |
| `nixling host doctor --read-only` | no | `--read-only` mandatory |
| `nixling host install --dry-run` | no | `--dry-run` mandatory; reports the synthesized 5-step install plan |
| `nixling host install --apply` | yes (daemon → broker) | `--apply` mandatory; optional `--enable` + `--start`/`--no-start`; broker failures exit 78 |

The `--dry-run` and `--apply` forms are intentionally mutually
exclusive: there is no flag-less `nixling host prepare`. Operators who
want the read-only inventory run `nixling host check`; operators who
want the apply-plan-without-mutation run `nixling host prepare
--dry-run`. `host destroy --apply` only withdraws nixling-owned state
and refuses foreign ownership markers.

The four reconcile domains — cgroup delegation, network (bridge /
TAP / NM / IPv6 / hosts), firewall (`inet nixling` nftables
coexistence + USBIP rule skeleton), and modules + device nodes —
are each documented in the sections below, which are assembled from
smaller fragment files.

## Conceptual model + recovery

For the architectural rationale, ownership-marker model, NM/networkd
coexistence theory, dry-run/apply/destroy boundaries, and the
post-compromise recovery runbook, read
[`docs/explanation/host-prepare.md`](../explanation/host-prepare.md)
first. The fragments below assume that conceptual baseline.

For per-distro tier behavior (Tier 0 NixOS, Tier 1 alpha Ubuntu 24.04,
Tier 1-later Fedora/Arch, Tier 2 best-effort), read
[`docs/reference/support-matrix.md`](../reference/support-matrix.md).
The privileged operations the broker may run on your behalf are
catalogued in
[`docs/reference/privileges.md`](../reference/privileges.md).

---

## Section 1 — cgroup v2 delegation

# Host prepare: cgroup v2 delegation

> Host-prepare fragment. The full
> `docs/how-to/host-prepare.md` page is assembled from the fragments
> under `docs/how-to/host-prepare.d/*.md`; this file is the cgroup
> section.

`nixling` runs every VM payload inside a per-VM/per-role cgroup leaf
beneath `/sys/fs/cgroup/nixling.slice`. The slice is created by the
small `nixling-priv-broker` and then delegated to the non-root
`nixlingd` daemon so the daemon never needs `CAP_SYS_ADMIN` on the
host cgroup tree at runtime.

This page covers operator-visible behavior. The full algorithm,
ownership model, and audit record shape are in
[`docs/reference/cgroup-delegation.md`](../reference/cgroup-delegation.md).

## How to verify cgroup delegation prerequisites

Before running `nixling host prepare --apply`, confirm the host meets
the prerequisites:

```bash
# 1. Unified cgroup v2 hierarchy:
[ -f /sys/fs/cgroup/cgroup.controllers ] \
  && echo "ok: unified cgroup v2" \
  || echo "fail: legacy/hybrid cgroup layout"

# 2. Required controllers advertised on the root:
grep -wE 'cpu|memory|io|pids|cpuset' /sys/fs/cgroup/cgroup.controllers

# 3. nixlingd is non-root (delegation refuses uid 0):
id nixlingd
```

A host that boots with `systemd.unified_cgroup_hierarchy=0` or with the
legacy `cgroup` v1 mount option WILL fail `host check` with
`cgroup-v2-unified-not-present` (exit code 1).

## What `host check` reports for cgroup

`nixling host check` evaluates the following invariants in order; the
first failure is what the operator sees:

| Reported code | Meaning | Remediation |
| --- | --- | --- |
| `cgroup-v2-unified-not-present` | `/sys/fs/cgroup/cgroup.controllers` missing or unreadable. | Re-boot with the unified cgroup v2 hierarchy. NixOS: `boot.kernelParams = [ "systemd.unified_cgroup_hierarchy=1" ];`. |
| `cgroup-controllers-missing` | One of `cpu`, `memory`, `io`, `pids`, `cpuset` is absent from `cgroup.controllers`. | Confirm `systemd-cgls --all` works on the host; ensure the kernel exposes the missing controller. |
| `cgroup-delegation-refused` | Phase B (post-delegation) runtime mutation was attempted while the broker is still uid 0 — i.e., the broker failed to drop to `nixlingd` uid before the steady-state cgroup code path. Phase A privileged setup legitimately runs as root per ADR 0011. | Re-check the `nixlingd` user/group bootstrap and re-run `host prepare --apply`; verify the broker's drop-priv between Phase A and Phase B is wired correctly. |
| `cgroup-kill-on-ancestor-refused` | A broker-mediated `CgroupKill` op was requested on `nixling.slice` or an intermediate VM/host cgroup (i.e., `path_class: slice` or `vm-interior`). | This is a guard — the daemon re-requests `CgroupKill` against the specific leaf path instead. No operator action. |

Every check writes a record to the broker audit log at
`/var/lib/nixling/audit/broker-<utc-date>.jsonl` (root:nixlingd 0640),
keyed by `operation: "DelegateCgroupV2"` or `operation: "OpenCgroupDir"`.

## What `host prepare --apply` does for cgroup

For a successful apply, the broker performs the 8-step delegation
sequence documented in [`cgroup-delegation.md`][ref]:

1. probe the unified hierarchy;
2. assert `{cpu, memory, io, pids, cpuset}` are advertised;
3. ensure `cpuset.cpus`/`cpuset.mems` inherit from `.effective` on
   every ancestor before `+cpuset` is enabled;
4. enable `+cpu, +memory, +io, +pids, +cpuset` on `cgroup.subtree_control`
   in that strict order, verifying each enable by re-reading;
5. create `/sys/fs/cgroup/nixling.slice`;
6. keep `cpuset.cpus.partition` as `member` on `nixling.slice`
   and every nixling-created descendant (per-VM intermediate /
   per-role / host-scoped leaves); nixling does NOT read or
   write ancestor `cpuset.cpus.partition` (the cgroup v2 root
   is typically a partition root and that state is the host's
   concern, not nixling's);
7. fd-relative `fchown` the delegated subtree to `nixlingd:nixlingd`;
8. refuse Phase B (post-delegation) runtime mutation if the broker
   is still running as uid 0; Phase A privileged setup
   legitimately runs as root per ADR 0011 Decision item 2.

After the apply, `nixling.slice` is owned by `nixlingd` and the
delegated subtree carries every required controller in
`cgroup.subtree_control`. Threaded cgroups are forbidden.

`cgroup.kill` is permitted only via **broker-mediated** `CgroupKill`
on per-VM role leaves or host-scoped leaves during declared
teardown (v1.1+ — the broker is the sole writer per
[`docs/reference/cgroup-delegation.md`](../reference/cgroup-delegation.md)
"Broker ops on the cgroup tree"; daemon NEVER writes `cgroup.kill`
directly). Asking the broker to kill an ancestor returns
`cgroup-kill-on-ancestor-refused`.

[ref]: ../reference/cgroup-delegation.md

## Troubleshooting

### "cgroup-v2-unified-not-present"

The host is on a legacy or hybrid cgroup layout.

- **NixOS**: set `boot.kernelParams = [ "systemd.unified_cgroup_hierarchy=1" ];`
  and reboot. Most NixOS systems already run unified cgroup v2 by
  default; this only applies to hosts that explicitly opted out.
- **Ubuntu 24.04**: unified cgroup v2 is the default. If the probe
  fails, check `mount | grep cgroup` — the only mount under
  `/sys/fs/cgroup` should be `cgroup2`.

### "cgroup-controllers-missing"

The kernel is older than 6.6 or has one of the required controllers
disabled. Confirm `CONFIG_CPUSETS=y`, `CONFIG_MEMCG=y`,
`CONFIG_BLK_CGROUP=y`, `CONFIG_CGROUP_PIDS=y`, `CONFIG_CGROUP_SCHED=y`.

### "cgroup-delegation-refused" (uid 0)

The broker is supposed to enter the cgroup work path as the dropped
`nixlingd` user. If it reaches that path while still running as root,
something is wrong with the broker bootstrap. Re-check
`docs/explanation/host-prepare.md` § recovery.

### `kernel.modules_disabled=1`

Cgroup delegation does NOT load any kernel modules. This sysctl
does not block delegation. If you see `host-modules-locked` from
`host check`, that is a separate device-related preflight (scope s4),
not cgroup-related.

---

## Section 2 — network reconcile

# How to: prepare the host network

> Diataxis category: guide. Operator-facing walkthrough for the bridge /
> TAP / NetworkManager / IPv6 / `/etc/hosts` reconcile. The full
> `docs/how-to/host-prepare.md` document is assembled from fragments
> under `docs/how-to/host-prepare.d/`.

## Scope

This fragment covers the network reconcile deliverables:

- bridge + TAP lifecycle (`CreateTapFd`, `CreatePersistentTap`,
  `SetBridgePortFlags`);
- the 5-step **IPv6-off** ordering with NetworkManager / systemd-networkd
  (per-link sysctls plus `bridge-nf-call-*` when `br_netfilter` is
  loaded);
- NetworkManager unmanaged config + the **correct reload command**
  (`nmcli general reload conf`, NOT `nmcli connection reload`);
- the route preflight predicate set (default route, IPv6-absence,
  `/etc/hosts` managed-block, dnsmasq-bound, host LAN CIDR derivation);
- the Cloud Hypervisor net-handoff probe (`tap-fd` preferred,
  `persistent-tap` fallback, `ch-net-handoff-not-supported`
  fail-closed when neither mode works).

## Dry-run / apply / destroy walkthrough

```bash
# Read-only inventory: lists derived ifnames, declared bridges,
# detected NM version + state, host LAN CIDRs, route preflight result.
nixling host check --json

# Plan-only: emits the reconcile diff without mutating host state.
# Fully wired today.
sudo nixling host prepare --dry-run

# In v1.0 (per ADR 0015) `--apply` is wired live: it dispatches
# through the broker reconcile ops (ApplyNftables, ApplyRoute,
# ApplySysctl, UpdateHostsFile, ApplyNmUnmanaged), takes the per-VM
# lock, applies the diff, and runs the IPv6-off readback gate,
# failing closed on drift. Broker failures surface as the typed
# `broker-error` envelope (exit 78); daemon-unreachable surfaces
# `daemon-down` (exit 1).
sudo nixling host prepare --apply

# Same disposition for destroy: --apply is live in v1.0 and reverses
# the host-prepare mutations only (bridges, TAPs, NM drop-in, /etc/hosts
# managed block, IPv6 sysctls). Foreign state is never touched.
sudo nixling host destroy --apply
```

In v1.0 (per ADR 0015), the `--apply` invocations dispatch through
the broker reconcile ops (`ApplyNftables`, `ApplyRoute`,
`ApplySysctl`, `UpdateHostsFile`, `ApplyNmUnmanaged`) on every
non-Tier-0 host; broker failures surface as the typed `broker-error`
envelope (exit 78); daemon-unreachable surfaces `daemon-down`
(exit 1). The `host check` and `--dry-run` reads exercise the
broker's read-only audit path.

`host prepare --apply` is refused on Tier 0 NixOS-legacy hosts unless
at least one VM in the bundle declares
`nixling.vms.<vm>.supervisor = "nixlingd"`.

## Ownership markers (foreign-rule preservation guarantees)

The broker writes inside marker blocks that downstream consumers can
grep for and refuse to modify:

| File                                                | Begin marker                    | End marker                    |
| --------------------------------------------------- | ------------------------------- | ----------------------------- |
| `/etc/hosts`                                        | `# nixling-managed begin`       | `# nixling-managed end`       |
| `/etc/NetworkManager/conf.d/00-nixling-unmanaged.conf` | `# nixling-managed begin`    | `# nixling-managed end`       |
| `/proc/sys/net/ipv6/conf/<nixling-ifname>/*`        | per-link only (no marker file)  | n/a                           |
| `/proc/sys/net/ipv4/conf/<nixling-ifname>/*`        | per-link only                   | n/a                           |
| `/proc/sys/net/bridge/bridge-nf-call-*`             | global; only written when `br_netfilter` is loaded | n/a |

Foreign lines outside the marker block are preserved byte-for-byte.
The `tests/path-safety-violation-fs.sh` canary exercises symlink swap,
hardlink, rename-race, and world-writable-parent on every marked file.

## IPv6-off 5-step ordering (per link)

Each nixling-owned bridge or TAP follows the same sequence. Any drift
between the step-3 write and the step-5 readback is the
`ipv6-sysctl-drift` canary and fails closed.

1. **Pre-create**: install the NetworkManager `unmanaged` drop-in (or
   refuse on systemd-networkd hosts without a configured-unmanaged
   file). Trigger `nmcli general reload conf` (NM >= 1.20) — fall back
   to `systemctl reload NetworkManager.service` on older NM. **Do not
   use `nmcli connection reload`**: it only reloads connection
   profiles, not the `conf.d/*.conf` device-management snippets.
2. **Create link** with `IFF_UP` cleared (link-down).
3. **Write per-link sysctls** while the link is down:
   - `net/ipv6/conf/<ifname>/disable_ipv6=1`
   - `net/ipv6/conf/<ifname>/accept_ra=0`
   - `net/ipv6/conf/<ifname>/autoconf=0`
   - `net/ipv6/conf/<ifname>/addr_gen_mode=1`
   - `net/ipv4/conf/<ifname>/arp_ignore=1`
   - `net.bridge.bridge-nf-call-iptables=0`, `…-ip6tables=0` if
     `br_netfilter` is loaded.
4. **Bring link up** (`IFF_UP`).
5. **Readback gate** re-reads every sysctl above and fails closed
   on any drift. The same gate runs again pre-VM-start so foreign
   actors who flip a sysctl after host prepare cannot bring up VMs
   against unintended IPv6 state.

## Per-distro troubleshooting anchors

### Ubuntu 24.04 (Tier 1 alpha)

- NM 1.46. `nmcli general reload conf` is the correct command.
- `/proc/modules` typically contains `br_netfilter`; the bridge-nf
  sysctls are written.
- If `nmcli -t -f DEVICE,STATE device status` reports the nixling
  ifname as `connected` after the reload, the failure mode is
  `nm-managed-foreign-conflict`. Audit log lists the foreign profile
  ID; remove or rename it and re-run `host prepare --apply`.

### Fedora 40+ (Tier 1-later)

- NM 1.48. Same reload command as Ubuntu.
- `firewalld` is active by default. Host prepare detects `firewalld`
  and refuses to apply the `inet nixling` table unless an explicit
  coexistence policy is declared in the bundle (`refuse` is the
  default).

### Arch (Tier 2)

- NM versions vary. `host check` records the version under
  `host.networkManagerVersion`; the broker selects
  `general reload conf` vs `systemctl reload` based on it.

### NixOS (Tier 0)

- `host prepare --apply` is refused on the legacy path. Tier-0
  consumers use the NixOS module: every nixling-owned bridge, TAP,
  sysctl, NM unmanaged entry, and `/etc/hosts` block is materialised
  declaratively via `nixos-modules/`. The `host doctor --read-only`
  command still runs and reports drift between the module-emitted
  state and the live host.

## Cloud Hypervisor net handoff mode

`host check` probes the packaged CH binary and records the selected
mode in `host.json` under `host.ch.netHandoffMode`:

- `tap-fd` (preferred): the broker opens TAP + `/dev/vhost-net` and
  passes the fds via `SCM_RIGHTS`. The runner runs without
  `CAP_NET_ADMIN`.
- `persistent-tap` (fallback): the broker creates the TAP with
  `TUNSETPERSIST` + `TUNSETOWNER`/`TUNSETGROUP` set to the runner
  uid/gid. The runner opens the device node read-only.
- `ch-net-handoff-not-supported`: neither mode satisfies the
  declared VM network resources without `CAP_NET_ADMIN`. **Host
  prepare fails closed**. Remediation is recorded under
  `docs/reference/support-matrix.d/s4-tier-modules.md`. The recorded
  mode is consumed by runner planning; L2 confirmation tests cover
  both modes and the failure case.

## Host LAN CIDR derivation

`nixling host check` reports the detected host LAN CIDRs and any
`ambiguous-host-lan` finding (point-to-point / VPN-like links). The
derivation rule:

- skip nixling-owned links (by prefix);
- skip loopback (`lo`);
- skip Docker/libvirt-known prefixes (`docker*`, `virbr*`, `lxcbr*`);
- skip DOWN-state links;
- collect remaining IPv4 `RT_TABLE_MAIN scope LINK` destinations;
- flag VPN-like routes (point-to-point, no broadcast) as ambiguous —
  do not include automatically. Operator overrides via
  `nixling.site.hostLanCidrs`.

## Failure modes operators will see

| Audit `error_kind`                  | Meaning                                                  |
| ----------------------------------- | -------------------------------------------------------- |
| `ifname-too-long`                   | Derived ifname exceeded IFNAMSIZ-1 (15 bytes).           |
| `ifname-collision`                  | Two `(env, vm, role)` keys derived the same ifname.      |
| `ipv6-sysctl-drift`                 | Per-link IPv6 sysctl readback diverged from step-3 write.|
| `bridge-port-flag-drift`            | Post-`SetBridgePortFlags` readback diverged.             |
| `nm-managed-foreign-conflict`       | NM still claims a nixling-declared ifname.               |
| `nm-reload-required`                | NM reload command failed; broker rolled back.            |
| `route-preflight-no-default-route`  | Declared uplink has no matching default route.           |
| `route-preflight-foreign-default-route` | Default route exists but `via` differs from `host.json`. |
| `dnsmasq-not-bound`                 | Declared DNS daemon not bound on the LAN ifname/port.    |
| `host-lan-cidr-ambiguous`           | VPN-like link detected; needs `site.hostLanCidrs`.       |
| `ch-net-handoff-not-supported`      | CH binary supports neither `tap-fd` nor `persistent-tap`.|
| `path-safety-violation`             | Symlink/hardlink/rename-race on hosts/NM/state/runtime.  |

See also: `docs/adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md`
for the rationale + rejected alternatives.

---

## Section 3 — firewall coexistence

# Host firewall coexistence

This fragment is included in `docs/how-to/host-prepare.md`.

This document is the operator how-to for the `inet nixling` named
table that the privileged broker reconciles during `nixling host
prepare --apply` (and re-checks before every VM start). The
authoritative chain layout reference lives at
[`../reference/inet-nixling-chains.md`](../reference/inet-nixling-chains.md);
the architectural rationale is in
[ADR 0013](../adr/0013-w3-firewall-coexistence-policy.md).

## What nixling installs

Exactly one named table, `inet nixling`, with four chains:

| Chain        | Hook         | Priority | Policy   |
| ------------ | ------------ | -------- | -------- |
| `prerouting` | `prerouting` | `-150`   | `accept` |
| `forward`    | `forward`    | `-5`     | `drop`   |
| `output`     | `output`     | `-5`     | `accept` |
| `input`      | `input`     |  `-5`    | `accept` |

Every rule and chain carries `comment "nixling managed: <id>"`. Nixling
NEVER allocates `raw`, `mangle`, or `nat` hooks under `inet nixling`,
and NEVER runs `nft flush ruleset`.

## What nixling does NOT touch

- Foreign tables, chains, sets, maps. The reconcile path emits a
  declarative batch for `inet nixling` only; everything else stays
  byte-for-byte intact.
- Your `iptables-save` output. If the host runs the `iptables-nft`
  compatibility shim, nixling detects it and chooses `coexist` only
  when its hook priority demonstrably wins.

## Per-distro guidance

### Fedora / RHEL / CentOS Stream (firewalld)

Default policy: **refuse**. firewalld owns the nft `filter` family
under its own zone-based abstractions; coexistence at the unprivileged
`inet nixling` priority does not survive `firewall-cmd --reload`.

To use nixling on a firewalld host, either:

1. Stop firewalld (`systemctl disable --now firewalld`) and re-run
   `nixling host prepare --apply`; or
2. Replace firewalld with a firewall setup where nixling owns
   `inet nixling`; otherwise nixling fails closed.

### Ubuntu (ufw)

Default policy: **refuse**. ufw is implemented on top of the
`iptables-nft` shim and writes its own chains at a priority that
shadows `inet nixling`'s `forward` chain.

To use nixling on a ufw host:

1. `ufw disable` and re-run `nixling host prepare --apply`; or
2. Replace ufw with a firewall setup where nixling owns `inet
   nixling`; otherwise the host check refuses.

### Mixed Docker / libvirt setups

Default policy: **require-unmanaged**. Both Docker and libvirt write
their own `filter`/`nat` chains. Nixling will install `inet nixling`
alongside them but requires an explicit
`/etc/nixling/firewall.coexist-with-{docker,libvirt}.toml` marker so
the operator has acknowledged the forward-path arbitration that
follows. The host check enforces that marker, and the forward path is
verified
on every VM start via the post-apply `nft list table inet nixling -j`
re-hash; drift fails closed with `inet-nixling-drift`.

### iptables-nft compatibility shim

Default policy: **coexist**. Only safe when `iptables --version`
reports `(nf_tables)` AND no other manager is active. The pre-VM-start
hook re-reads `inet nixling`'s post-apply hash and refuses to start
VMs if a foreign rule has been inserted at a priority that would
shadow the nixling decision.

### NixOS (no manager)

Default policy: **coexist**. Nixling owns `inet nixling`; the rest of
the ruleset is whatever your `networking.firewall` / `networking.nftables`
declared.

## Drift detection

Every VM start re-hashes `nft list table inet nixling -j` (with
volatile `handle`/`index` fields stripped) and compares against the
digest stored in the bundle's `host.json`. Mismatches fail closed with
`inet-nixling-drift`; remediation is to re-run
`nixling host prepare --apply`.

## USBIP firewall carve-out

When a VM is configured for USBIP passthrough,
`UsbipBindFirewallRule` adds a per-busid source-based carve-out to
`inet nixling`'s `forward` chain BEFORE the generic allow/drop.
This is **firewall-only**; the USBIP attach/detach flow is handled
separately from this firewall carve-out.

## Troubleshooting

- **`firewall-coexistence-mismatch`**: the detected manager does not
  match the bundle's declared policy. Either change the bundle (allowed
  override per the matrix above) or stop/disable the offending manager
  and re-run `nixling host prepare --apply`.
- **`nft-foreign-rule-shadows-nixling`**: a foreign hook at a priority
  ≤ `-5` is active. Inspect with `nft list ruleset` and identify the
  source.
- **`inet-nixling-drift`**: the live table no longer matches the
  bundle digest. Re-apply with `nixling host prepare --apply`; if it
  recurs immediately, a periodic process is rewriting the ruleset
  (`firewalld --reload`, `ufw reload`, custom cron, …).

---

## Section 4 — kernel modules + device nodes

# Modules and devices

Operator how-to fragment for the kernel-module and device-node
requirements introduced by host prepare. The integrator assembles this fragment
into [`docs/how-to/host-prepare.md`](./host-prepare.md).

## Kernel modules

Host prepare runs a four-step probe before any `ModprobeIfAllowed` broker call:

1. `/proc/sys/kernel/modules_disabled` — if the file reads `1`, every
   `required` module that is neither built-in nor loaded surfaces as
   `host-modules-locked`. There is no remediation other than rebooting
   with `modules_disabled=0` or shipping the module built-in.
2. `/proc/modules` plus `/sys/module/<name>/` — loaded-module
   detection. Modules listed here are accepted without any further
   action.
3. `/lib/modules/$(uname -r)/modules.builtin` (preferred) or
   `modules.builtin.bin` — built-in detection. Built-in modules
   satisfy the requirement without needing `modprobe`.
4. `/boot/config-$(uname -r)` or `/proc/config.gz` — `CONFIG_*` checks
   used only as **secondary evidence**. The probe never refuses solely
   on the basis of a missing `CONFIG_*` line.

The broker accepts a `ModprobeIfAllowed` request only when the module
name appears in the trusted bundle's `kernelModules` matrix with
`loadAllowed: true`. Every decision (allow + deny) is audited with the
`module_name`, `matrix_entry_id`, and the `modules_disabled` sysctl
value captured at decision time.

### `br_netfilter` posture

If step 2 detects `br_netfilter` as loaded, the probe recommends
pinning:

- `net.bridge.bridge-nf-call-iptables=0`
- `net.bridge.bridge-nf-call-ip6tables=0`

so iptables / ip6tables cannot route around the `inet nixling`
policy. An ADR opt-in is required to suppress this recommendation.

### Distro troubleshooting

- **Ubuntu 24.04 (Tier 1).** Required modules (`kvm_intel`/`kvm_amd`,
  `tun`, `vhost_net`, `fuse`) ship as loadable. `modprobe.d`
  blacklists for any of these surface as `host-modules-locked`.
- **Fedora 40+ (Tier 1 later).** Same module set; `vhost_net` may need
  an explicit `modprobe vhost_net` on first boot.
- **Arch (Tier 2).** Kernel built with `MODULES_DISABLED=y` requires a
  rebuild before VM startup is accepted.
- **NixOS (Tier 0 legacy).** The framework's NixOS module is the
  primary path; `nixling host prepare --apply` is refused with
  `tier-0-legacy-uses-nixos-module`.

## Device nodes

The matrix validated in read-only mode:

| Class           | Default path          | Required mode | Required group | Notes |
| --------------- | --------------------- | ------------- | -------------- | ----- |
| `kvm`           | `/dev/kvm`            | `0660`        | `kvm`          | KVM acceleration. |
| `net-tun`       | `/dev/net/tun`        | `0660`        | `kvm`          | TAP / TUN. |
| `vhost-net`     | `/dev/vhost-net`      | `0660`        | `kvm`          | Vhost-net offload. |
| `fuse`          | `/dev/fuse`           | `0660`        | `fuse`         | virtiofsd. |
| `dri`           | `/dev/dri`            | `0660`        | `video`        | Optional GPU passthrough. |
| `nvidia-*`      | `/dev/nvidia*`        | `0660`        | `video`        | Optional NVIDIA. |
| `pipewire`      | `/run/user/pipewire-0`| socket        | n/a            | Optional audio sidecar. |
| `usbip-host`    | `/dev/usbip-host`     | `0660`        | `usbip`        | Optional USBIP. |
| `tpm`           | `/dev/tpm0`           | `0660`        | `tss`          | Optional TPM passthrough. |
| `vfio`          | `/dev/vfio/vfio`      | `0660`        | `vfio`         | Optional VFIO. |

Stricter modes are accepted; **looser** modes (anything with extra
world bits) fail closed as `loose-mode`. Group ownership is checked by
name; mismatch surfaces as `wrong-group`. The host check **never
mutates** ACLs; remediation is via the trusted bundle / NixOS module.

### Preflight boundary

This check is read-only preflight only. The per-VM `/nix/store`
hardlink farm, the mount namespace, and the virtiofsd setup all
belong to runtime startup. Host prepare surfaces blocking findings
under `host doctor --read-only` and **refuses** to mutate store state.

## Runner-shape preflight

`nixling host check` consumes `host.json`, `processes.json`, and
`closures/<vm>.json` runner-parity snapshots, then validates them
without launching Cloud Hypervisor:

- packaged CH capabilities match `host.json`'s declared row;
- every enabled VM has a `declaredRunner` argv hash present;
- CH API socket paths declare `mode = 0660` and a non-empty owner;
- vsock transports are Unix-socket-backed (`transport = "unix"`);
- virtiofsd / swtpm sidecar `dagNodeId`s appear in the
  `processes.json` DAG.

The same module probes the CH binary for net-handoff support. The
preferred mode is `tap-fd` (broker opens TAP + `/dev/vhost-net` and
passes fds via `SCM_RIGHTS`; runner has **no** `CAP_NET_ADMIN`). The
fallback is `persistent-tap` (broker creates a persistent TAP with
`TUNSETOWNER`/`TUNSETGROUP`). If neither mode satisfies the declared
VM network resources without `CAP_NET_ADMIN`, the host check fails
closed with `ch-net-handoff-not-supported`.

## ioctl allowlist

The broker derives a per-role ioctl allowlist from typed
[`DeviceClass`](../reference/manifest-bundle.md) entries; no
catch-all `ioctl: 1` exists. The 5-class negative-allowlist matrix
(`TAP/TUN`, cgroup chown, sysctl write, nft batch apply,
device-open) is exercised by `tests/ioctl-negative.sh` against fake
backends.

---

## Cross-references

- ADR 0011 — [cgroup v2 delegation and pidfd handoff](../adr/0011-cgroup-v2-delegation-and-pidfd-handoff.md)
- ADR 0012 — [IPv6-off sysctl set, hash-derived IfName, bridge-port defaults](../adr/0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md)
- ADR 0013 — [firewall coexistence policy matrix + `inet nixling` chain layout](../adr/0013-w3-firewall-coexistence-policy.md)
- ADR 0014 — [`kernel.modules_disabled=1` behavior, module probe order, CH net handoff selection, and runner-shape preflight](../adr/0014-w3-modules-devices-runner-shape.md)
- Reference: [`docs/reference/cgroup-delegation.md`](../reference/cgroup-delegation.md)
- Reference: [`docs/reference/inet-nixling-chains.md`](../reference/inet-nixling-chains.md)
- Reference: [`docs/reference/support-matrix.md`](../reference/support-matrix.md)
- Reference: [`docs/reference/privileges.md`](../reference/privileges.md)
- Explanation: [`docs/explanation/host-prepare.md`](../explanation/host-prepare.md)
- Security boundary deltas: [`SECURITY.md`](../../SECURITY.md) §
  host-prepare trust-boundary delta.
