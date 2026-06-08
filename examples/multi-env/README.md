# examples/multi-env — two isolated envs

Two `nixling.envs.<env>` instances side-by-side. Each env gets its
own bridges, its own auto-declared net VM, its own dnsmasq pool, its
own nftables ruleset, and its own usbipd-proxy backend port. VMs in
different envs cannot reach each other at the network layer; the
host can reach both.

This is the conceptually richest example: it shows **why** the env
abstraction exists and what materialises behind a two-line env
declaration.

## Why two envs

The security model is **separation of concerns by risk class**.
"Work" software (the corporate identity broker, browser sessions
signed in to corporate SaaS, conferencing clients with system-wide
permissions) lives in one env. "Personal" software (the dev VM,
the throwaway browser, an LLM client) lives in another. They do
not share a LAN, do not share a default route, and cannot
laterally reach each other if one is compromised.

If you only need one env, see [`../minimal/`](../minimal/) — multi-
env is for users whose threat model includes "I do not want my
personal dev VM to be able to ARP-scan the corporate VM, even on
the same physical host".

## Topology

```
   host (192.168.1.42 on its physical LAN)
   │
   ├─ br-work-up  (192.0.2.0/30) ───── sys-work-net VM
   │   │   .1 (host)                      .2 (uplink)
   │   │                                  │
   │   │                       br-work-lan (10.20.0.0/24)
   │   │                                  │  .1 (gateway, dnsmasq, NAT)
   │   │                                  │
   │   │                                  ├─ work-app (10.20.0.10)
   │   │                                  └─ … future work-* VMs
   │   │
   │   └─ nixling-sys-work-usbipd-proxy.service
   │      bound to 192.0.2.1:3240
   │      → backend 127.0.0.1:3242
   │
   └─ br-personal-up (192.0.2.4/30) ── sys-personal-net VM
       │   .5 (host)                       .6 (uplink)
       │                                   │
       │                       br-personal-lan (10.30.0.0/24)
       │                                   │  .1 (gateway, dnsmasq, NAT)
       │                                   │
       │                                   ├─ personal-app (10.30.0.10)
       │                                   └─ … future personal-* VMs
       │
       └─ nixling-sys-personal-usbipd-proxy.service
          bound to 192.0.2.5:3240
          → backend 127.0.0.1:3241
```

Three things worth noticing on the diagram:

1. **The host has NO interface on either `*-lan` bridge.** It only
   sits on the two `/30` uplinks. A workload VM that tries to
   reach the host can only do so through its env's net VM, which
   firewalls everything except the carved-out USBIP TCP/3240
   towards the host's uplink IP.
2. **The two LAN bridges are not bridged together anywhere.** The
   host's IPv4 forwarding table has no route from `10.20.0.0/24`
   into `10.30.0.0/24`. Each LAN is reachable only via its own
   net VM, and neither net VM is configured to forward into the
   other env's uplink.
3. **The host CAN reach both LANs.** `network.nix` installs a
   static route per env (`10.20.0.0/24 via 192.0.2.2`,
   `10.30.0.0/24 via 192.0.2.6`) so `ssh alice@10.20.0.10` and
   `ssh alice@10.30.0.10` both work from the host shell.

## What gets auto-declared

Two-line env declarations:

```nix
nixling.envs.work     = { lanSubnet = "10.20.0.0/24"; uplinkSubnet = "192.0.2.0/30"; };
nixling.envs.personal = { lanSubnet = "10.30.0.0/24"; uplinkSubnet = "192.0.2.4/30"; };
```

…produce, per env, with zero further config:

| Resource                           | `work`                                  | `personal`                              |
|------------------------------------|-----------------------------------------|-----------------------------------------|
| Uplink bridge                      | `br-work-up`                            | `br-personal-up`                        |
| LAN bridge                         | `br-work-lan`                           | `br-personal-lan`                       |
| Host uplink IP                     | `192.0.2.1`                             | `192.0.2.5`                             |
| Net VM (auto, `autostart = true`)  | `nixling.vms.sys-work-net`              | `nixling.vms.sys-personal-net`          |
| Net VM uplink IP                   | `192.0.2.2`                             | `192.0.2.6`                             |
| Net VM LAN IP (= gateway, dnsmasq) | `10.20.0.1`                             | `10.30.0.1`                             |
| DHCP overflow pool                 | `10.20.0.251–254`                       | `10.30.0.251–254`                       |
| usbipd proxy unit                  | `nixling-sys-work-usbipd-proxy`         | `nixling-sys-personal-usbipd-proxy`     |
| usbipd proxy bind                  | `192.0.2.1:3240`                        | `192.0.2.5:3240`                        |
| usbipd backend port (loopback)     | `3242`                                  | `3241`                                  |
| Host static route                  | `10.20.0.0/24 via 192.0.2.2`            | `10.30.0.0/24 via 192.0.2.6`            |
| Net VM state dir                   | `/var/lib/nixling/sys/work-net/`        | `/var/lib/nixling/sys/personal-net/`    |

The two `sys-*-net` VMs are real microVMs, just declared by the
framework instead of the user. They show up in `nixling list` like
any other VM and can be inspected with `nixling console sys-work-net`.
They are autostarted at host boot — see `nixling.vms.<name>.autostart`,
defaulted to `true` for net VMs by `network.nix`.

### Backend port allocation

Per-env USBIP backend ports are `3241 + alphabetical-index of env
name`. `network.nix` uses `lib.attrNames` over the enabled env set,
which returns names sorted lexicographically:

| env        | alphabetical index | backend port |
|------------|--------------------|--------------|
| `personal` | 0                  | `3241`       |
| `work`     | 1                  | `3242`       |

The sort-determinism matters: adding a new env shifts ports for
any env that sorts after it. The uplink-side proxy bind
(`<host-uplink-ip>:3240`) is stable regardless — guests address
`3240`, only the backend port behind the proxy moves. Pin it via
`extraNetConfig` if you need cross-env stability.

## Per-VM derivation rules

Workload VMs reference an env via `env` + `index`:

```nix
nixling.vms.work-app     = { env = "work";     index = 10; };  # → 10.20.0.10
nixling.vms.personal-app = { env = "personal"; index = 10; };  # → 10.30.0.10
```

From `(env, index)`, the framework deterministically derives:

- **IP**: `<lan-subnet-prefix>.<index>`.
- **MAC**: `02:<8-hex-chars-of-sha256(env + "-lan")>:<index-as-2-hex-digits>`
  — see `lib.nix`'s `mkMac`. Same env + index always yields the
  same MAC, so dnsmasq reservations are stable across rebuilds.
- **Tap name**: `<env>-l<index>` on `br-<env>-lan`. Capped to 15 chars
  (Linux interface name limit); env names are constrained to ≤ 8
  characters in `assertions.nix` to leave room.
- **dnsmasq host-reservation** in the net VM's config:
  `dhcp-host=<MAC>,<hostname>,<IP>,infinite`.
- **Per-VM firewall policy** in the net VM's nftables ruleset:
  - LAN ↔ LAN ACCEPT (intra-env east-west).
  - LAN → `192.0.2.1:3240` ACCEPT (USBIP carve-out to host uplink).
  - LAN → `nixling.hostLanCidrs` DROP (host's primary LAN blocked).
  - LAN → all other destinations: ACCEPT (masqueraded via the net
    VM's uplink → host → physical NIC).

Index uniqueness is scoped **per-env** — `work-app.index = 10` and
`personal-app.index = 10` is fine; the framework derives different
MACs and IPs because the env name is part of the MAC seed and
the LAN subnet differs.

## `nixling.hostLanCidrs`: block host neighbours

```nix
nixling.hostLanCidrs = [ "192.168.1.0/24" ];
```

Unioned into every env's `hostBlocklist`, so a workload VM in
*any* env cannot reach anything on the host's physical LAN — not
just the host's own IP. Set this to whatever `ip route` says is
your physical LAN.

The default `hostBlocklist` already covers RFC1918 broadly
(`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`),
but a host on a non-RFC1918 wire (public IPv4 office LAN) would
otherwise be exposed via the env's masquerade. Putting your real
LAN here is the defence-in-depth move.

## dnsmasq per-env reservations

Each env's net VM runs an isolated dnsmasq bound to the env's LAN
bridge. The reservations are derived at host eval time from the env's
workload VMs and baked into the net VM's `services.dnsmasq.settings`.

For `work-app` (env=work, index=10) the resulting net-VM dnsmasq
config gets, in effect:

```ini
dhcp-range=10.20.0.251,10.20.0.254,255.255.255.0,24h
dhcp-host=02:<sha256("work-lan")[0:8]>:0a,work-app,10.20.0.10,infinite
```

Workload VMs run pure-DHCP networkd (no static IP in the guest);
the reservation guarantees they always get the framework-derived IP.

## `extraNetConfig`: when and when not

`nixling.envs.<env>.extraNetConfig` is an arbitrary NixOS module
merged into the env's auto-declared net VM. It exists for things the
framework deliberately doesn't have first-class options for:

- Extra `services.dnsmasq.settings.address` entries (split DNS into
  the env, e.g. resolving `internal.corp.com` to a fixed LAN IP).
- Extra `networking.nftables.ruleset` chunks (a TLS-terminating
  proxy carve-out, a per-env outbound DENY list).
- Hostname / banner / extra SSH keys on the net VM itself.

**⚠️ Strong warning.** `extraNetConfig` is an UNSAFE escape hatch.
The framework's own NAT, dnsmasq, firewall, and route declarations
form one self-consistent set; arbitrary user modules merged into
the same net VM can:

- Open holes in the firewall (a blanket
  `networking.firewall.allowedTCPPorts = [ … ]` exposes the net VM
  on its uplink, which is reachable from the host).
- Conflict with the framework's nftables ruleset and lose rules at
  table-merge time.
- Break dnsmasq if user settings collide with framework-generated
  `dhcp-host` / `dhcp-range` lines.
- Subvert the env isolation invariant (e.g. by adding a second
  interface on the wrong bridge).

Treat any `extraNetConfig` block as part of your TCB. The empty
block in `configuration.nix` documents the option without
changing behaviour.

## USBIP per-env isolation

Each env's USBIP path is fully isolated:

1. **Uplink proxy** — `nixling-sys-<env>-usbipd-proxy.service` binds
   to the env's host uplink IP on TCP/3240. Workload VMs in `work`
   `usbip attach` to `192.0.2.1`; in `personal` they hit `192.0.2.5`.
   A VM addressing the wrong env's uplink IP is firewalled off by
   that env's net VM.
2. **Backend port** — the proxy forwards to a per-env loopback port
   (`3241 + alphabetical-index`). Each env runs its own
   `nixling-sys-<env>-usbipd-backend.service`, so the underlying
   usbipd processes are also separated. Attaching a YubiKey to the
   `work` env never exposes it on the `personal` env's path.
3. **Net-VM nftables** — the LAN-to-uplink-IP carve-out names the
   env's own uplink IP. A `work` VM cannot reach the `personal`
   env's uplink IP via the routing fabric, because the `work` net
   VM's default route goes via the host's `192.0.2.1` and the host
   has no route from `work-lan` to the `personal-up` bridge.

`nixling usb <vm>` reads the VM's env from the manifest and
addresses the correct uplink IP automatically.

## Try it

```bash
# Eval the example without building.
nix eval examples/multi-env#nixosConfigurations.demo.config.system.build.toplevel.drvPath

# Quick `nix flake check` (the example flake has its own checks set).
nix flake check examples/multi-env --no-build --all-systems
```

Both gates run in CI as part of the top-level `tests/static.sh`.

## After activation

A successful `nixos-rebuild switch` leaves you with the bridges,
the two net VMs already running (`autostart = true` by
construction in `network.nix`), and both workload VMs **down**.
Concretely:

```bash
nixling list
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# personal-app       personal  false     false false   10.30.0.10      stopped
# sys-personal-net   personal  false     false false   192.0.2.6       systemd (net-vm)
# sys-work-net       work      false     false false   192.0.2.2       systemd (net-vm)
# work-app           work      false     false false   10.20.0.10      stopped

nixling status
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# personal-app       personal  false     false false   10.30.0.10      stopped
# sys-personal-net   personal  false     false false   192.0.2.6       systemd (net-vm)
# sys-work-net       work      false     false false   192.0.2.2       systemd (net-vm)
# work-app           work      false     false false   10.20.0.10      stopped
#
# === Bridge health ===
# BRIDGE               STATE      ADMIN   EXPECTED     RESULT
# br-personal-up       UP         up      UP           ok
# br-personal-lan      NO-CARRIER up      NO-CARRIER   no-carrier (no workloads up)
# br-work-up           UP         up      UP           ok
# br-work-lan          NO-CARRIER up      NO-CARRIER   no-carrier (no workloads up)

# Net VMs (`sys-<env>-net`) show STATUS=`systemd (net-vm)` — they
# are framework-managed and `autostart = true` by construction in
# `nixos-modules/network.nix`. Workload VMs default to `stopped`
# until you `nixling up <vm>` (or set `autostart = true` per-VM).

nixling up work-app
nixling up personal-app
ssh -i /var/lib/nixling/keys/work-app_ed25519     alice@10.20.0.10 hostname
ssh -i /var/lib/nixling/keys/personal-app_ed25519 alice@10.30.0.10 hostname

# Prove the isolation invariant: work-app cannot reach personal-app.
ssh -i /var/lib/nixling/keys/work-app_ed25519 alice@10.20.0.10 \
  -- 'ping -c1 -W2 10.30.0.10'
# (expect timeout: net VMs do not bridge between envs)
```

## Troubleshooting

### `nixling@<vm>.service` won't start; `nixling-net-route-preflight.service` failed

The route preflight oneshot probes each env's workload IP and
refuses to let any nixling VM start if a host route resolves via
the wrong device (i.e. not the env's expected `br-<env>-up`
uplink bridge). The unit is `Type=oneshot`, ordered before every
`nixling@<vm>.service` via `requiredBy`/`before`, and on failure
its stderr names the offending env. Common causes:

- **Stale `ip route` entry from a previous config** that conflicts
  with the env's uplink. Inspect with
  `ip route show table all | grep -E '10\.(20|30)\.'`. Delete
  stragglers with `sudo ip route del <route>` and then
  `systemctl reset-failed nixling-net-route-preflight`.
- **Chosen env CIDR overlaps a route the host already owns**
  (Tailscale subnet, WireGuard, VPN-pushed route). Pick a disjoint
  CIDR or unset the conflicting route source.
- **Bridge `br-<env>-up` not present** — typically a botched
  rebuild. Re-run `sudo nixos-rebuild switch` and watch
  `systemd-networkd` logs for the bridge.

The preflight's exact error format is:

```
nixling-net-route-preflight: ERROR env '<env>' workload IP <x.y.z.10> resolves via:
  <ip-route-output>
  expected dev br-<env>-up; check for stale routes / CIDR overlaps.
```

## Common gotchas

- **CIDR overlap is fatal at eval time.** Both envs share the host
  but their `lanSubnet`s and `uplinkSubnet`s MUST be disjoint from
  each other AND from every entry in `nixling.hostLanCidrs`.
  `assertions.nix` enforces this with `cidrOverlaps`; the route
  preflight above is the runtime backstop.
- **Workload VMs in env A cannot reach VMs in env B by design.**
  Each per-env net VM masquerades only to its own uplink; nothing
  forwards traffic between `br-work-lan` and `br-personal-lan`.
  This is the entire point of multi-env.
- **The host CAN reach both LANs** via the static routes
  `network.nix` installs (`10.20.0.0/24 via 192.0.2.2` and
  `10.30.0.0/24 via 192.0.2.6`). If `nixling-net-route-preflight`
  trips, this is the chain that broke.
- **USBIP backend port assignment is alphabetical.** Adding a new
  env that sorts before `personal` shifts the backend ports
  underneath; the uplink-side `<host-uplink-ip>:3240` bind is
  stable so guests don't notice, but log analysis on the host
  needs to follow the rename.

## What this example does NOT cover

- **Graphics or audio.** Both VMs are headless. See
  [`../graphics-workstation/`](../graphics-workstation/).
- **Entra ID / Himmelblau.** See
  [`../with-entra-id/`](../with-entra-id/).
- **Persistent state.** Both VMs are pure NixOS evals — no
  `var.img`, no `microvm.volumes`, no TPM. Add those in your
  consumer config; nothing about per-env isolation changes.

## See also

- [`examples/minimal`](../minimal/) — read-and-copy headless starter
- [`examples/graphics-workstation`](../graphics-workstation/) — desktop VM with Wayland + audio + USBIP
- [`examples/with-entra-id`](../with-entra-id/) — Entra-ID composition via the sibling flake
- [`templates/default`](../../templates/default/) — scaffold via `nix flake init`

> **Note on the in-tree path** — the version of `flake.nix` checked
> into this directory uses `nixling.url = "path:../..";` so the
> example can be evaluated against the in-tree framework without a
> network. When you copy this layout into your own repo, swap it
> for a real flake ref (`github:vicondoa/nixling/v0.1.0` or a
> pinned revision).
