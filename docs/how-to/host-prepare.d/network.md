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
sudo nixling host prepare --dry-run

# Apply: takes the per-VM lock, applies the diff, and runs the
# IPv6-off readback gate. Fails closed on drift.
sudo nixling host prepare --apply

# Reverses the host-prepare mutations only (bridges, TAPs, NM drop-in,
# /etc/hosts managed block, IPv6 sysctls). Foreign state untouched.
sudo nixling host destroy --apply
```

`host prepare --apply` is refused on a Tier 0 NixOS-legacy host —
one where nixling resolves no daemon-owned bundle to reconcile. The
per-VM `nixling.vms.<vm>.supervisor` option was removed in v1.1 (per
ADR 0015); every enabled VM is now daemon-supervised, so a normal v1.1
host resolves to the daemon path.

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
