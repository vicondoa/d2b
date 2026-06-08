# 0011 — W3 IPv6-off sysctl set, hash-derived IfName scheme, and bridge-port flag defaults

- Status: Accepted
- Date: 2026-04-29
- Wave: W3 (`s2` scope: bridge / TAP / NetworkManager / IPv6 / IfName)

## Context

W3 introduces idempotent host reconcile for bridges, TAPs, NetworkManager
unmanaged drop-ins, `/etc/hosts` ownership, route preflight, and
IPv6 disablement on every nixling-owned link. Three load-bearing
choices in this scope deserve an ADR because they freeze a contract
the integrator and consumer ABI depend on:

1. **The exact IPv6-off sysctl set per link** — what we write, in
   what order, and what we read back.
2. **The hash-derived `IfName` scheme** — how `(env, vm, role)`
   becomes an IFNAMSIZ-compliant Linux interface name with a
   deterministic emitter-time collision detector.
3. **Per-role bridge-port flag defaults** — every flag, every role,
   with a double opt-in for east-west bridges.

## Decision

### 1. IPv6-off sysctl set + ordering

For every nixling-created bridge or TAP, the broker performs the
5-step ordered sequence in `nixling_host::netlink::ipv6_off_sequence`:

1. Pre-create: install NM unmanaged drop-in + run `nmcli general
   reload conf` (NM >= 1.20) or `systemctl reload
   NetworkManager.service` (older NM). `nmcli connection reload` is
   **not sufficient** — it only reloads connection profiles, not
   the `conf.d/*.conf` device-management rules.
2. Create the link with `IFF_UP` cleared.
3. While the link is down, write the per-link sysctls:
   - `net.ipv6.conf.<if>.disable_ipv6 = 1`
   - `net.ipv6.conf.<if>.accept_ra = 0`
   - `net.ipv6.conf.<if>.autoconf = 0`
   - `net.ipv6.conf.<if>.addr_gen_mode = 1`
   - `net.ipv4.conf.<if>.arp_ignore = 1`
   - When `br_netfilter` is loaded:
     `net.bridge.bridge-nf-call-iptables = 0`,
     `net.bridge.bridge-nf-call-ip6tables = 0`.
4. Bring the link up.
5. Re-read every sysctl. Any drift is the `ipv6-sysctl-drift`
   canary; fail closed and audit.

The same readback runs pre-VM-start so foreign actors that flip a
sysctl after `host prepare --apply` cannot bring up VMs against
unintended IPv6 state.

### 2. Hash-derived IfName scheme

`<prefix><role-tag><HASH_SUFFIX>` where:

- `prefix` defaults to `nl-` (configurable per site, `<=8` bytes,
  `[A-Za-z0-9_-]+`, must end with `-`);
- `role-tag` is one ASCII char: `b` for bridge, `t` for TAP;
- `HASH_SUFFIX` is the 8-char Crockford base32 encoding of a
  64-bit FNV-1a hash over `env | 0x1F | vm? | 0x1E | role-tag`.

The default-prefix shape `nl-b<8>` is 12 bytes, well within
IFNAMSIZ-1 (15 bytes).

Two emitter-time guards:

- `IfNameError::IfNameTooLong` — refuses any candidate `>= 16`
  bytes (defence-in-depth even though the encoding can't produce
  one with the default prefix);
- `IfNameError::IfNameCollision(detail)` — `detect_collisions`
  scans every `IfNameMapping` for duplicate bridge or TAP names
  and returns the two colliding parties, with `env`, `vm`, and
  `role` recorded for the audit record.

The broker re-runs `detect_collisions` against its trusted bundle
copy before any host mutation. Both sides emit the same
`ifname-collision` error tag.

### 3. Per-role bridge-port flag defaults

Every flag, every role per
`nixling_host::bridge_port::BridgePortFlagSet::defaults_for`:

| Role                       | isolated | hairpin | learning | unicast_flood | multicast_flood | neigh_suppress | bpdu_guard | root_block | fast_leave |
| -------------------------- | -------- | ------- | -------- | ------------- | --------------- | -------------- | ---------- | ---------- | ---------- |
| `net-vm-lan`               | false    | false   | true     | true          | true            | false          | true       | true       | false      |
| `workload-lan-isolated`    | **true** | false   | true     | false         | false           | true           | true       | true       | true       |
| `workload-lan-east-west`   | false    | false   | true     | true          | true            | false          | false      | false      | false      |
| `uplink-p2p`               | true     | false   | false    | false         | false           | true           | true       | true       | true       |

`workload-lan-east-west` requires a **double opt-in** before
`SetBridgePortFlags` will accept it:

- `nixling.envs.<env>.lan.allowEastWest = true`;
- `nixling.site.allowUnsafeEastWest = true`.

`validate_role_against_policy` returns
`BridgePortPolicyError::EastWestRequiresEnvOptIn` /
`EastWestRequiresSiteOptIn` if either is missing. Other roles are
unconditionally accepted.

After every `SetBridgePortFlags`, the broker runs
`readback_bridge_port_flags` which calls
`bridge_port::validate_readback` — every flag must match the
per-role default. Drift is the `bridge-port-flag-drift` canary;
fail closed.

## Alternatives rejected

- **`net.ipv6.conf.all.disable_ipv6=1` globally instead of per-link.**
  Foreign tooling that expects host-level IPv6 (mDNS responders,
  Docker bridges, libvirt) breaks; per-link is the only safe
  reconcile.
- **SHA-256 or BLAKE3 for the IfName hash.** Brings in a heavy
  hashing crate for ~40 bits of namespace. FNV-1a is sufficient at
  this scale (~10^12 entries with negligible collision probability)
  and ships with zero deps.
- **Base32 RFC4648 instead of Crockford.** RFC4648 contains
  ambiguous glyphs (`I`/`l`/`0`/`O`). Crockford's alphabet was
  designed for human transcription which matches the operator UX
  (audit records, CLI status output).
- **Pretty `(env)-(vm)-(role)` names.** Tempting for debuggability
  but blows IFNAMSIZ on realistic VM names. The hash scheme stays
  in 12 bytes regardless of input length; the user-visible
  `IfNameMapping` rows in `host.json` expose the human-readable
  side, so debuggability does not regress.
- **Eager bridge-port-flag write with no readback.** Foreign
  bridge-port toggles (firewalld, libvirt, Cilium) can flip them
  silently. Readback gate is mandatory and the only contract the
  W3 pre-VM-start hook can rely on.
- **`nmcli connection reload`.** This reloads connection profiles
  only and does not pick up `conf.d/*.conf` device-management
  rules. Documented as a footgun upstream; W3 must use
  `nmcli general reload conf`.

## Security implications

- IPv6 disablement is the bulwark against link-local SLAAC leaking
  workload identity to the host LAN. Per-link rather than global
  preserves foreign IPv6 connectivity. The readback gate is the
  only thing that makes this contract enforceable against
  concurrent foreign mutation.
- `IfNameCollision` carries collision parties verbatim in the
  audit record. The risk surface is operator-visible only (no
  remote untrusted input feeds these names); collision reports
  are useful for incident triage.
- East-west bridge double opt-in puts the explicit unsafe toggle
  in the site config, not the env config. Compromising one env
  bundle cannot enable east-west alone.

## Test coverage

L1c canaries (every row in plan.md §"W3 pre-merge canary matrix"
that s2 owns):

- `ifname-too-long`, `ifname-collision`
  (`tests/ifname-collision.sh`);
- `ipv6-sysctl-drift`, `bridge-port-flag-drift`
  (`tests/host-prepare-network.sh`, `tests/ipv6-off-readback.sh`);
- `nm-managed-foreign-conflict`, `nm-reload-required`,
  `route-preflight-no-default-route`,
  `route-preflight-foreign-default-route`, `dnsmasq-not-bound`,
  `host-lan-cidr-ambiguous`, `ch-net-handoff-not-supported`
  (`tests/host-prepare-network.sh`);
- `path-safety-violation` on `UpdateHostsFile`,
  `ApplyNmUnmanaged`, `PrepareStateDir`, `PrepareRuntimeDir`
  (`tests/path-safety-violation-fs.sh`).

L1b unit tests cover every flag/role combination in
`bridge_port::tests` and every IPv6-off sysctl readback path in
`netlink::tests`.

## Consequences

- Operators see deterministic, short, hash-derived names — the
  pretty `(env)-(vm)` form lives in `host.json` and CLI status
  only.
- Foreign IPv6 connectivity on non-nixling links is preserved.
- The `bridge-port-flag-drift` and `ipv6-sysctl-drift` canaries
  catch foreign actors that flip per-link state after host prepare.
- The east-west double opt-in adds friction for operators who
  legitimately need it; the friction is intentional.

## References

- plan.md §"W3 IPv6-off ordering with NetworkManager / systemd-networkd"
- plan.md §"W3 IfName hash collision + mapping exposure"
- plan.md §"W3 bridge-port flag readback (every flag, every role)"
- plan.md §"W3 NetworkManager reload"
- plan.md §"W3 pre-merge canary matrix"
- ADR 0005 (network/firewall/TAP model) — the W2 baseline this
  ADR extends.
- ADR 0010 (wire protocol + typed errors) — wire surface for
  `IfNameCollision`, `BridgePortFlagDrift`, `SysctlReadback`.
