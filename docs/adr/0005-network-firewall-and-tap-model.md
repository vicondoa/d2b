# 0005. Network, firewall, and TAP model

- Status: Accepted
- Date: 2026-05-25
- Wave: W0b
- Plan slice: "`host.json` encodes exact network intent: stable interface names as validated `IfName` newtypes: <=15 bytes (`IFNAMSIZ-1`), ASCII `[A-Za-z0-9_-]+`, d2b-owned prefix, no truncation."
- Companion ADRs: [ADR 0002](0002-non-root-daemon-and-privileged-broker.md), ADR 0006, ADR 0008

## Context

The v0.4.0 baseline creates per-environment network topology through
NixOS: a `br-<env>-up` uplink bridge for the host-to-net-VM /30, a
`br-<env>-lan` bridge for the net-VM-to-workload /24, an auto-declared
`sys-<env>-net` VM, dnsmasq, nftables inside the net VM, networkd, and
iptables carve-outs for USBIP. AGENTS.md marks net VM networking and
firewall behavior as a critical subsystem that must be validated with
the existing network tests before changes land.

The v0.4.0 baseline also introduced env-level network options for MTU,
MSS clamping, and east-west LAN isolation. The portability plan requires
those semantics to survive in `host.json`, including workload isolation
by default and an explicit two-key opt-in through both
`lan.allowEastWest` and `site.allowUnsafeEastWest` before same-LAN
workload traffic is unisolated.

Moving host preparation into `d2bd` and the broker changes who owns
bridges, TAPs, sysctls, firewall rules, `/etc/hosts`, NetworkManager
unmanaged state, USBIP rules, and route preflight. The new model must be
idempotent, marker-scoped, fail-closed on drift, and careful not to flush
or silently override foreign firewall managers.

The TAP and firewall decisions also bind the privilege boundary from ADR
0002. Long-lived hypervisors must not keep `CAP_NET_ADMIN`, so privileged
network construction has to happen in the broker and be handed off as
fds or tightly bounded persistent TAP ownership when fd-based Cloud
Hypervisor networking is unavailable.

## Decision

1. W1 `host.json` encodes typed `IfName` newtypes that are at most 15 bytes, match ASCII `[A-Za-z0-9_-]+`, use hash-based d2b-owned names to avoid IFNAMSIZ truncation, and carry stp-off, multicast-snooping-off, IPv6-disabled, configure-without-carrier, per-env MTU, and per-env MSS clamp intent.
2. Bridge-port flag defaults match the plan table: net VM LAN ports are unisolated, workload LAN ports are isolated unless both `lan.allowEastWest = true` and `site.allowUnsafeEastWest = true`, uplink point-to-point ports are unisolated, and neighbor suppression is off for every role.
3. The preferred TAP handoff is for the broker to open TAP and vhost fds and pass them by `SCM_RIGHTS`, with persistent `TUNSETOWNER` and `TUNSETGROUP` only as a fallback when Cloud Hypervisor cannot consume `--net fd=<n>`.
4. Long-lived hypervisor and runner profiles must not retain `CAP_NET_ADMIN`.
5. Host nftables ownership is limited to one named table `inet d2b` and chains within it, with hook priorities chosen so explicit d2b drops and allows execute before foreign filter chains for ACCEPT carve-outs and after foreign chains for default-drop behavior.
6. Detection of firewalld, ufw, Docker, libvirt, or iptables-nft selects a typed coexistence policy that refuses by default unless a documented carve-out applies, and `d2bd` periodically re-hashes the `inet d2b` table and reruns route preflight before VM start.
7. NetworkManager unmanaged configuration is materialized for every d2b bridge and TAP.
8. `/etc/hosts` management is limited to a begin/end sentinel-marked d2b block.
9. Route preflight runs before every VM start and fails closed on missing, conflicting, or drifted host routes.
10. USBIP is daemon-owned, guarded by a global and busid-exclusive lock, limited to one env backend or proxy per bound device, mirrored with source-based nft rules equivalent to current iptables behavior, and cleaned up on detach, failure, or daemon crash recovery.
11. IPv6 disablement is concrete because the broker writes per-link `disable_ipv6=1`, `accept_ra=0`, `autoconf=0`, and deterministic `addr_gen_mode` sysctls immediately after `RTM_NEWLINK` and before link-up.
12. `br_netfilter` is fail-closed: when the module is loaded, d2b refuses VM start unless `net.bridge.bridge-nf-call-iptables=0`, `net.bridge.bridge-nf-call-ip6tables=0`, and `net.bridge.bridge-nf-call-arptables=0`. Broker host prepare writes those sysctls, and host check fails closed when it cannot write or verify them.

## Consequences

1. Positive: Interface naming, bridge flags, MTU, MSS, IPv6, and east-west policy become typed manifest data instead of ad hoc host side effects.
2. Positive: The broker can create network resources without granting long-lived hypervisors `CAP_NET_ADMIN`.
3. Positive: Marker-scoped nftables, NetworkManager, hosts-file, USBIP, and route-preflight rules make host reconcile idempotent and auditable.
4. Negative: Refuse-by-default coexistence with foreign firewall managers will block some existing hosts until documented carve-outs and diagnostics are implemented.
5. Neutral: ADR 0002 defines the broker operation surface for these privileged actions, ADR 0006 owns the bundle artifacts that encode inputs, and ADR 0008 owns the supported kernel and platform matrix.

## Alternatives considered

- Preserve v0.4.0 bridge names verbatim: rejected because long env and VM names can exceed IFNAMSIZ and be truncated ambiguously.
- Keep `CAP_NET_ADMIN` in the Cloud Hypervisor runner: rejected because network setup is a broker responsibility and steady-state runners should not retain host-network mutation power.
- Flush and rewrite host firewall state: rejected because d2b must own only marked resources and must coexist safely with host policy.
- Leave IPv6 disablement advisory: rejected because generated link-local or RA-derived addresses would violate the plan's concrete IPv6-off posture.

## br_netfilter policy

If `br_netfilter` is loaded, d2b refuses VM start unless all of the
following sysctls are exactly `0`:

- `net.bridge.bridge-nf-call-iptables=0`
- `net.bridge.bridge-nf-call-ip6tables=0`
- `net.bridge.bridge-nf-call-arptables=0`

The broker writes those sysctls during host prepare. Host check fails
closed when it cannot write them, cannot read them back, or reads any
non-zero value while `br_netfilter` is loaded. This prevents bridge
traffic from unexpectedly traversing host iptables/ip6tables/arptables
paths outside the marker-scoped nftables model.

> **v1.1 reconciliation note** (resolves R10 networking-r10-2):
> [ADR 0012](0012-w3-ipv6-off-sysctl-set-and-hash-ifname.md)
> § "Ordered sysctl set" enumerates the host-prepare sysctl write
> ordering for IPv6 disablement and lists ONLY
> `bridge-nf-call-iptables=0` and `bridge-nf-call-ip6tables=0`
> in its ordered set (arptables is omitted there because it is not
> in the IPv6-disablement codepath). The three-sysctl read+verify
> set above remains authoritative for the `br_netfilter` host-check
> contract (all three must be `0`); ADR 0012 is concerned with the
> WRITE ordering for the IPv6 path and the two iptables/ip6tables
> sysctls happen to also be in that set, but the arptables sysctl
> is verified-only (not actively written by the IPv6-disablement
> path because arptables traffic does not affect IPv6 link-up).
> v1.1 implementations MUST write+verify all three per this ADR;
> ADR 0012's omission of arptables from its ordered set is NOT
> permission to skip the arptables verify-on-host-check (host
> check fails closed if any of the three reads non-zero while
> `br_netfilter` is loaded).

## References

- plan.md, "Baseline: d2b v0.4.0"
- plan.md, "Kernel resource model"
- plan.md, "Networking model"
- plan.md, "Privileged broker contract"
- plan.md, "Required test families"
- AGENTS.md, "Critical subsystems — handle with care"
- AGENTS.md, "Don'ts (security-relevant)"

## inet d2b table hook priorities and chain layout

W1 `host.json` carries an `nftables` block for the marker-owned
`inet d2b` table. The table is the only host nftables table that
d2b creates or reconciles. D2b never flushes foreign tables or
chains.

The concrete chain layout is:

> **v1.1 reconciliation note** (resolves R10 networking-r10-1 +
> R11 networking-r11-1): [ADR 0013](0013-w3-firewall-coexistence-policy.md)
> § "Chain layout (exactly four chains)" SUPERSEDES the six-chain
> `d2b_*`-named `-300/+300` layout below for v1.1+ implementations.
> ADR 0013 declares exactly four chains — **`prerouting`,
> `forward`, `output`, `input`** (no `d2b_*` prefix; the chain
> NAMES inside the `inet d2b` table use the hook names
> directly per ADR 0013's table) — at priorities `-150` /
> `-5` documented in
> [`docs/reference/inet-d2b-chains.md`](../reference/inet-d2b-chains.md);
> the six-chain split-by-policy layout in the table below uses
> the legacy `d2b_*` naming and `-300/+300` priorities — these
> are the v1.0 historical decision retained for narrative
> continuity. v1.1 implementation work (broker nftables emit +
> drift detector) MUST follow ADR 0013 + the reference doc, NOT
> this table. ADR 0013 formally extends ADR 0005 (per ADR 0013
> § References) and is the canonical source. The carve-out
> entries documented elsewhere in this ADR or in
> [`components-usbip.md`](../reference/components-usbip.md) using
> `d2b_forward` should be read as the equivalent `forward` chain
> per ADR 0013.

| Chain | Type | Hook | Priority | Policy | Role |
| --- | --- | --- | --- | --- | --- |
| `d2b_ingress_accept` | filter | `input` | `-300` | `accept` | Early ACCEPT carve-outs for daemon-owned host endpoints and marked management traffic. |
| `d2b_forward_accept` | filter | `forward` | `-300` | `accept` | Early ACCEPT carve-outs for net-VM forwarding, DHCP/DNS, USBIP, and explicitly allowed env traffic. |
| `d2b_egress_accept` | filter | `output` | `-300` | `accept` | Early ACCEPT carve-outs for host-originated d2b control-plane traffic. |
| `d2b_ingress_drop` | filter | `input` | `300` | `accept` | Late drops for marked d2b input traffic that did not match an allow rule. |
| `d2b_forward_drop` | filter | `forward` | `300` | `accept` | Late default-drop for cross-env, `net_vm_forward_blocklist`, and non-carved forwarding. |
| `d2b_egress_drop` | filter | `output` | `300` | `accept` | Late drops for marked d2b egress that must not escape policy. |

The priorities intentionally bracket the standard nftables filter
priority `0`: `-300` runs before foreign filter chains so required
ACCEPT carve-outs cannot be shadowed by ordinary host policy, while
`300` runs after foreign filter chains so d2b default-drop behavior
does not mask an earlier foreign drop. Foreign firewall managers still
select the coexistence policy (`coexist`, `refuse`, or
`require-unmanaged`) recorded in `host.json`; these priorities do not
authorize unsafe coexistence by themselves.

Rule ownership is marker-scoped within these chains. Host reconcile
rehashes the complete `inet d2b` table before VM start and fails
closed on drift, but it preserves all non-d2b tables and chains.
