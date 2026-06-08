# Design overview

Threat model and design rationale for [`vicondoa/nixling`][nixling].
This document sits in the *explanation* quadrant of the [Diataxis]
structure: it answers "why is nixling shaped this way?" rather than
"how do I configure it?". Companion documents — the manifest schema
([`../reference/manifest-schema.md`](../reference/manifest-schema.md)),
the CLI contract ([`../reference/cli-contract.md`](../reference/cli-contract.md)),
and the [`CHANGELOG.md`](../../CHANGELOG.md) — describe the *what*.

The doc tracks the implementation as it exists today (pre-v0.1.0).
Where a defense is incomplete, or where a design tradeoff has known
holes, that is called out explicitly. Concrete file paths under
`nixos-modules/` are cited so the reader can verify a claim against
the code.

[nixling]: https://github.com/vicondoa/nixling
[Diataxis]: https://diataxis.fr/

## Contents

- [1. The problem nixling solves](#1-the-problem-nixling-solves)
- [2. Threat model](#2-threat-model)
- [3. Architecture](#3-architecture)
- [4. Defenses in depth](#4-defenses-in-depth)
- [5. Limitations and known gaps](#5-limitations-and-known-gaps)
- [6. Why not X — design rationale FAQ](#6-why-not-x--design-rationale-faq)
- [7. References](#7-references)

## 1. The problem nixling solves

A single-user NixOS desktop wants more than one workspace on the
same physical machine — typically "work", "personal", and "risky
dev / browsing" — each with its own credentials, network identity,
USB device attachments, and disk state, and none of which should
be able to observe or interfere with another. The Wayland session
on the host is the one trusted surface the human actually
interacts with; everything else should be containable.

[microvm.nix] is the obvious building block: KVM-based isolation,
declarative per-VM NixOS config, no Xen-grade complexity. But
microvm.nix is deliberately a primitive. It does not opine on
networks (the consumer wires bridges by hand), it does not manage
SSH keys for the operator, it does not provide a single CLI that
behaves like a unit-of-work boundary, and its sidecar processes
(crosvm GPU forward, swtpm, vhost-device-sound, virtiofsd) run as
whatever the consumer's NixOS config sets — typically a shared
user with broader permissions than necessary.

Nixling is the per-host glue that turns microvm.nix into an
**opinionated workspace framework**: declare an env, declare some
workloads in it, get a fully isolated network + key management +
hardened sidecars + a `nixling` CLI for daily ops. The framework
does not fork microvm.nix; it composes on top of it by translating
`nixling.vms.<vm>` into `microvm.vms.<vm>` in
[`nixos-modules/host.nix:153-221`](../../nixos-modules/host.nix).

[microvm.nix]: https://github.com/microvm-nix/microvm.nix

## 2. Threat model

### Trust boundaries

```
          ┌──────────────────────────────────────────────────────┐
          │                       HOST                           │
          │                                                      │
          │   ┌──── Wayland user (trusted UI principal) ─────┐   │
          │   │  compositor + nixling CLI invocations         │   │
          │   └──────────────────┬────────────────────────────┘   │
          │                      │ polkit allowlist (start/stop)  │
          │                      │ + ssh via keysDir              │
          │                      ▼                                │
          │   ┌──── nixling sidecars (semi-trusted) ─────────┐   │
          │   │   nixling-<vm>-gpu      (per-VM uid)         │   │
          │   │   nixling-<vm>-snd      (per-VM uid)         │   │
          │   │   nixling-<vm>-swtpm    (per-VM uid)         │   │
          │   │   microvm-virtiofsd@<vm>(per-VM uid)         │   │
          │   │   nixling-sys-<env>-usbipd-{backend,proxy}   │   │
          │   └──────────────────┬────────────────────────────┘   │
          │                      │ vsock / virtio-* / ACL'd       │
          │                      │ sockets (wayland-0, pipewire-0)│
          ╞══════════════════════╪═══════════════════════════════ ╡
          │                      │ KVM boundary                   │
          │   ┌──────────────────▼────────────────────────────┐   │
          │   │             GUEST (untrusted)                  │   │
          │   │  workload userspace, in-VM kernel, browser…    │   │
          │   └────────────────────────────────────────────────┘   │
          │                                                      │
          └──────────────────────────────────────────────────────┘

                    ─── per-env LAN ───────  (point-to-point only)
   workload VM tap ─┤
                    └── br-<env>-lan ── sys-<env>-net ── br-<env>-up ── host
                       (isolated)                                       │
                                                                        │
                                                                  primary LAN
                                                                  (host's
                                                                  hostLanCidrs)
```

Five distinct boundaries:

1. **Host kernel ↔ guest userspace + guest kernel.** Enforced by
   KVM and the cloud-hypervisor / QEMU runtime supplied by
   microvm.nix. This is the strongest boundary nixling has and
   the one a workload-level compromise has to defeat first.
2. **Host kernel ↔ sidecar processes.** Sidecars run as
   dedicated, per-VM system users (no `DynamicUser`, no shared
   service account), with systemd-level hardening on top. See
   [§4](#4-defenses-in-depth) for the exact unit options.
3. **Sidecar ↔ guest userspace.** A sidecar talks to its guest
   only through its own purpose-built transport (a wayland
   socket, a pipewire socket, a TPM control socket, a vsock,
   the virtio-fs share). The guest cannot reach back through
   any other path; bind-mounts in the sidecar's mount namespace
   are exactly the file paths it needs.
4. **VM ↔ VM (intra-host).** Two workload VMs in the same env
   cannot exchange frames directly. The LAN bridge sets
   `Isolated = true` on every workload tap
   ([`nixos-modules/network.nix:376-386`](../../nixos-modules/network.nix));
   the only un-isolated port is `<env>-l1`, which belongs to the
   env's net VM. Across envs there is no shared bridge at all.
5. **Net VM ↔ outside world.** Each env's net VM is the sole
   egress point for the workloads in that env. The net VM runs
   nftables with a default-deny forward chain, a documented
   carve-out for USBIP to the host's uplink IP, and a
   `hostBlocklist` DROP rule
   ([`nixos-modules/net.nix:140-156`](../../nixos-modules/net.nix))
   that includes the host's primary LAN CIDRs.

### Threats addressed

**Compromised guest userspace.** A browser RCE, a malicious
container, an exploited package manager — anything that gets
code execution inside the guest — is bounded by the KVM
boundary. Within its env, the workload cannot reach peer
workloads (bridge isolation), cannot reach the host's primary
LAN (hostBlocklist), and cannot reach a different env at all
(distinct bridges + distinct net VMs). It *can* reach the
public internet, NAT'd through its env's net VM; that is
intentional — a workspace VM without internet is rarely useful.

**Compromised sidecar.** The GPU sidecar in particular runs
cloud-hypervisor and crosvm-device-gpu, both of which are
non-trivial native code with prior CVE history. Each sidecar
runs under a dedicated per-VM user (`nixling-<vm>-gpu`,
`nixling-<vm>-snd`, `nixling-<vm>-swtpm`; declared in
[`nixos-modules/host-users.nix`](../../nixos-modules/host-users.nix))
with `NoNewPrivileges`, `ProtectSystem=strict`, narrow
`ReadWritePaths`, `RestrictAddressFamilies` cut to the minimum
each backend requires, `DevicePolicy=closed`, and an explicit
`DeviceAllow` list. A compromise of `nixling-<vm>-gpu` can
touch `/dev/kvm`, `/dev/dri/renderD128`, the per-VM state dir
under `/var/lib/nixling/vms/<vm>/`, and the bind-mounted
wayland socket — and nothing else.

**Guest kernel exploit.** A privilege escalation from guest
userspace to the in-VM kernel does not cross the KVM boundary;
the host kernel is unaffected, and the guest's `/nix/store`
view is still restricted to the VM's own closure (see *Per-VM
nix store* below).

**Cross-VM lateral movement.** A workload in env A cannot reach
a workload in env B. There is no shared bridge — `br-A-lan` and
`br-B-lan` are distinct interfaces, each net VM is a separate
sandbox, and CIDR overlap is rejected at eval time
([`nixos-modules/network.nix:220-275`](../../nixos-modules/network.nix)
uses pure-Nix prefix arithmetic to detect e.g. `10.0.0.0/16 ⊃
10.0.1.0/24`). The per-VM `/nix/store` farm means even if a
workload chains a hypothetical microvm.nix host-side bug, it
cannot enumerate store paths it never had a closure entry for.

**Network sniffing on the shared LAN.** The host's primary LAN
(the wire the host's `eno*` interface sits on) is declared via
`nixling.hostLanCidrs`. Every env's net VM merges that list
into its `hostBlocklist`, so a workload cannot reach the host's
neighbours (NAS, printer, other workstations) even if the env's
default-deny chain were bypassed.

**DHCP preemption on net VMs (W5 H1).** A net VM has two NICs
matched by MAC. The guest baseline in
[`nixos-modules/base.nix:47-54`](../../nixos-modules/base.nix)
defines a catch-all `10-eth-dhcp` systemd-networkd network for
*workload* VMs; on a net VM that catch-all would sort
lex-first against the per-MAC `10-uplink` / `10-lan`
definitions, DHCP both NICs, and preempt the static config.
[`nixos-modules/net.nix:55-57`](../../nixos-modules/net.nix)
neutralises this by `lib.mkForce`-ing the catch-all's match to
a sentinel MAC (`00:00:00:00:00:00`) that no interface will
ever expose. Workload VMs continue to inherit the base.nix
catch-all unchanged.

**Untrusted disk state across reboot.** Per-VM TPM state lives
under `/var/lib/nixling/vms/<vm>/swtpm/` and is owned by
`nixling-<vm>-swtpm:nixling-<vm>-swtpm` mode 0700. No other VM's
swtpm process, no other VM's GPU sidecar, and no `kvm`-group
process can read it. The control socket is mode 0600 with an
ACL granting `nixling-<vm>-gpu` rw at `ExecStartPost` time
([`nixos-modules/host-sidecars.nix:79-83`](../../nixos-modules/host-sidecars.nix)).

### Threats *not* addressed

Nixling is deliberately not a defense against any of the following.
Pretending otherwise would be dishonest.

- **Physical attacker with host access.** Disk encryption, TPM
  unlock, secure boot, evil-maid attacks — all out of scope.
  Treat nixling's threat model as "host is up, host is trusted,
  attacker is on the wire or inside a guest."
- **Compromised host kernel.** Nixling is a host-trusted
  framework. If the host kernel falls, every VM falls with it.
- **Side channels (cache timing, branch predictor, Rowhammer,
  PCIe DMA peers).** Out of scope. KVM mitigations apply to
  whatever extent the upstream kernel + cloud-hypervisor +
  microcode provide; nixling adds nothing.
- **Supply chain attacks against nixpkgs, microvm.nix, or any
  upstream input.** Deferred to the consumer's own pin / audit
  discipline. The flake lock is the operator's responsibility.
- **TPM hardware backdoor or firmware attack.** Out of scope —
  the swtpm emulator we ship for VMs is software, but the host
  TPM (if used at all) is not nixling's concern.
- **Multi-user trust separation on the host.** Nixling assumes a
  single-human, single-Wayland-session host. The
  `nixling-launcher` group exists to make `nixling up <vm>`
  password-free for the human's account, not to model trust
  between two operators. SSH private keys at
  `/var/lib/nixling/keys/<vm>_ed25519` are readable by every
  member of `nixling-launcher`. A second untrusted human on the
  same machine breaks the threat model.
- **A malicious local launcher user.** A member of
  `nixling-launcher` can start, stop, and restart every nixling
  VM (within the unit allowlist), and read every per-VM SSH
  private key. The polkit grant ([§4](#4-defenses-in-depth))
  narrows the verb set and the unit set, but does not narrow
  *which* launcher user can drive *which* VM. By design.

## 3. Architecture

Nixling is a set of NixOS modules under `nixos-modules/`, aggregated
through `nixos-modules/default.nix`. The consumer imports
`nixos-modules/default.nix` from a top-level flake and populates
`nixling.site.*`, `nixling.envs.<env>.*`, and `nixling.vms.<vm>.*`.
Everything else is derived.

### `nixling@<vm>.service` — the per-VM wrapper

microvm.nix declares one template, `microvm@.service`. Nixling
wraps that with its own template, `nixling@.service`, declared in
[`nixos-modules/host-wrapper.nix`](../../nixos-modules/host-wrapper.nix).
The wrapper:

- `BindsTo + After microvm@%i`: if microvm.nix stops the
  underlying VM, the wrapper follows.
- Explicit `ExecStart`/`ExecStop` that calls
  `systemctl start|stop microvm@%i.service` — so
  `systemctl start nixling@<vm>` and `systemctl stop nixling@<vm>`
  symmetrically drive the underlying unit. `BindsTo` alone only
  propagates the bound→wrapper direction.
- `PropagatesStopTo` (systemd ≥249) belts-and-braces the stop
  direction.
- `Restart=` is intentionally omitted; microvm.nix owns restart
  policy on the underlying template.

Nixling pins `microvm.autostart = [ ]` so the upstream template
never carries its own `WantedBy=multi-user.target`. The wrapper is
the single source of truth for boot-time autostart, set per-VM via
`nixling.vms.<vm>.autostart`. This eliminates a class of double-
start bugs that would otherwise appear when both templates have
`wantedBy` attached.

### microvm.nix integration

Nixling does not fork microvm.nix. `nixos-modules/host.nix:153-221`
walks `config.nixling.vms`, validates the platform gate
(graphics/audio components are `x86_64-linux`-only via
`meta.platforms`), derives per-VM network metadata from the env
(MAC, IP, tap name, vsock CID), and emits a matching
`microvm.vms.<vm>` entry layered with `./base.nix` plus the
appropriate component modules. Net VMs are auto-declared the same
way ([`nixos-modules/network.nix:659-678`](../../nixos-modules/network.nix)),
just from `nixling.envs.<env>` metadata instead of an operator-
supplied module.

The `microvm.stateDir` override at
[`nixos-modules/host.nix:137`](../../nixos-modules/host.nix) puts
every nixling-managed file under `/var/lib/nixling/` instead of the
upstream default `/var/lib/microvms/`. This keeps one tree the
audit and backup scripts can reason about.

### Per-VM sidecars

For each declared VM, a subset of these sidecars exists, gated by
the per-VM component toggles:

- `microvm-virtiofsd@<vm>.service` — supplied by microvm.nix, runs
  as the microvm user, mediates the per-VM `/nix/store` share +
  any `microvm.shares` the consumer adds.
- `nixling-<vm>-store-sync.service` — populates the per-VM
  `/var/lib/nixling/vms/<vm>/store/` hardlink farm. Materialised
  for every enabled VM; see [`nixos-modules/store.nix`](../../nixos-modules/store.nix).
- `nixling-<vm>-gpu.service` — present when
  `nixling.vms.<vm>.graphics.enable = true`. Runs the
  whole `microvm-run` (cloud-hypervisor + crosvm GPU sidecar) as
  the dedicated `nixling-<vm>-gpu` user
  ([`nixos-modules/host-sidecars.nix:100-182`](../../nixos-modules/host-sidecars.nix)).
- `nixling-<vm>-snd.service` — present when
  `nixling.vms.<vm>.audio.enable = true`. Runs
  vhost-device-sound as `nixling-<vm>-snd`, exposes its
  socket at `/run/nixling/vms/<vm>/snd.sock`
  ([`nixos-modules/components/audio/host.nix`](../../nixos-modules/components/audio/host.nix)).
- `nixling-<vm>-swtpm.service` — present when
  `nixling.vms.<vm>.tpm.enable = true`. Per-VM software TPM
  emulator, state under `/var/lib/nixling/vms/<vm>/swtpm/`
  ([`nixos-modules/host-sidecars.nix:46-99`](../../nixos-modules/host-sidecars.nix)).

Per-env sidecars (one set per declared env, not per VM):

- `nixling-sys-<env>-usbipd-backend.service` — `usbipd` bound to
  `127.0.0.1:<envPort>`. `envPort` is `3241 + alphabetical index
  of env name`, computed deterministically at eval time.
- `nixling-sys-<env>-usbipd-proxy.{socket,service}` —
  socket-activated proxy that binds the env's
  `hostUplinkIp:3240` and forwards to the env's backend port via
  `systemd-socket-proxyd`.
- `nixling-net-route-preflight.service` — singleton, runs once
  per boot, fail-closed; documented under [§4](#4-defenses-in-depth).

### Per-env net VMs

Each `nixling.envs.<env>` causes
[`nixos-modules/network.nix`](../../nixos-modules/network.nix) to
materialise:

- Two host-side bridges (`br-<env>-up` /30 point-to-point host↔net,
  `br-<env>-lan` /24 net↔workloads — host has NO IP on the LAN
  bridge by design).
- A headless net VM `sys-<env>-net`, declared as a regular
  `nixling.vms.<netName>` and therefore subject to the same
  wrapper / store / sidecar machinery as any other VM. The VM's
  guest config comes from [`nixos-modules/net.nix`](../../nixos-modules/net.nix):
  nftables firewall, MASQUERADE on eth0, dnsmasq with DHCP
  host-reservations for every workload in the env, dropped IPv6.
- Per-tap networkd rules that route taps named `<env>-u*` to the
  uplink bridge (priority 30), `<env>-l1` to the LAN bridge un-
  isolated (priority 25), and `<env>-l*` for workloads to the
  LAN bridge with `Isolated = true` (priority 30).

The net VM's lifecycle is no more privileged than a workload's —
it is a regular nixling VM that happens to autostart, sit on both
bridges, and run NAT.

### Per-VM `/nix/store` hardlink farm

By default microvm.nix shares the host's entire `/nix/store` ro
into each guest, which leaks every package on the host (and every
other VM's closure) into each VM. Nixling replaces this with a
per-VM farm at `/var/lib/nixling/vms/<vm>/store/` containing only
the paths in that VM's `system.build.toplevel` closure. The farm
is built out of hardlinks into the host's real `/nix/store`, so
the disk overhead is directory entries only.

The hardlink trick is non-trivial: on NixOS `/nix/store` is bind-
mounted on top of itself, and Linux's `do_linkat` rejects cross-
mount hardlinks unconditionally even when the underlying device is
the same. The sync helper sidesteps this by running inside a
private mount namespace where `/nix/store` is lazily unmounted,
turning it into a plain directory under the root mount and
making the hardlink succeed
([`nixos-modules/store.nix`](../../nixos-modules/store.nix)).

The farm exposes itself to the guest via two virtio-fs shares:
the read-only closure as `/nix/.ro-store`, and a per-generation
metadata directory (`current → generations/N`, plus
`store-paths` and `db.dump`) as
`/run/nixling-store-meta`. The guest's
`nixling-load-store-db.service` (in
[`nixos-modules/base.nix:91-125`](../../nixos-modules/base.nix))
loads `db.dump` on every boot and on every `nixling switch`,
making `nix-store --query --valid` and `nix-shell` work without
seeing host paths.

### CLI

The `nixling` shell script (generated by
[`nixos-modules/cli.nix`](../../nixos-modules/cli.nix), see also
the behavioural contract at
[`../reference/cli-contract.md`](../reference/cli-contract.md))
is the daily-driver interface. Subcommands: `list`, `status`,
`up`, `down`, `console`, `switch`, `build`, `boot`, `test`,
`rollback`, `generations`, `gc`, `audio`, `usb`, `keys`, `trust`,
`rotate-known-host`, `audit`. The CLI is bash today and a Rust
port is planned for v0.2.0+; the
[manifest schema](../reference/manifest-schema.md) was designed
so that port can drop in without re-shaping the framework.

### Nixling-managed SSH keys

Pre-Phase-2b, the consumer was expected to declare per-VM SSH
keys themselves. Today
[`nixos-modules/host-keys.nix`](../../nixos-modules/host-keys.nix)
owns the whole lifecycle:

- At host activation, a single shell block (under `flock` on
  `<keysDir>/.lock`) walks every enabled VM, generates an
  Ed25519 keypair at `<keysDir>/<vm>_ed25519{,.pub}` if missing,
  repairs modes (0640 priv, 0644 pub) and ACL-grants the
  `nixling-launcher` group `r` on the private key.
- The same activation script stages the per-VM pubkey + the
  resolved `userAuthorizedKeys` content into
  `/var/lib/nixling/vms/<vm>/host-keys/`, which `host.nix`
  mounts into the guest via virtio-fs as
  `/run/nixling-host-keys/`. The guest baseline runs
  `nixling-load-host-keys.service` at boot, reads that share,
  dedupes, and writes the merged content into the SSH user's
  `~/.ssh/authorized_keys` ([`nixos-modules/base.nix:138-188`](../../nixos-modules/base.nix)).

The private key is never baked into the flake closure. It is
generated on the host the first time the consumer rebuilds, and
sticks around across rebuilds.

### State directory layout

Everything nixling owns lives under `/var/lib/nixling/`:

```
/var/lib/nixling/
├── vms/
│   └── <vm>/
│       ├── swtpm/              TPM2 state (NEVER wipe — IdP-bound creds)
│       ├── var.img             guest /var disk image (microvm.nix-owned)
│       ├── store/              per-VM /nix/store hardlink farm
│       ├── store-meta/         per-VM generation metadata
│       ├── host-keys/          host.pub + user-authorized-keys (virtiofs'd)
│       └── state/              per-VM mutable state (e.g. audio-state.json)
├── keys/
│   ├── .lock
│   ├── <vm>_ed25519            framework-managed SSH private key
│   └── <vm>_ed25519.pub
├── known_hosts.nixling         CLI-owned known_hosts file
└── known_hosts.nixling.lock
```

Net VMs land at `/var/lib/nixling/vms/sys-<env>-net/` for now;
splitting them off into a sibling `sys/<env>-net/` tree would
require either patching microvm.nix to expose a per-VM stateDir
override or filesystem-level bind-mounts. Tracked but not blocking.

### Naming conventions

Pulled out for ease of reference; the regexes and reserved names
are enforced by [`nixos-modules/assertions.nix`](../../nixos-modules/assertions.nix).

| Identifier                                | Constraint                              | Owner                       |
|-------------------------------------------|-----------------------------------------|-----------------------------|
| VM name (`nixling.vms.<vm>`)              | `^[a-z][a-z0-9-]*$`, ≤ ... no `sys-` prefix, not `launcher` | assertions.nix |
| Env name (`nixling.envs.<env>`)           | `^[a-z][a-z0-9-]*$`, length ≤ 8 (IFNAMSIZ-1=15 minus `br--lan` = 7) | network.nix |
| `nixling@<vm>.service`                    | user-facing per-VM unit                 | host-wrapper.nix            |
| `microvm@<vm>.service`                    | upstream microvm.nix unit               | microvm.nix                 |
| `nixling-<vm>-{gpu,snd,swtpm,store-sync}.service` | per-VM sidecars                 | host-sidecars.nix, audio/host.nix, store.nix |
| `nixling-sys-<env>-usbipd-{backend,proxy}.service` | per-env USBIP plumbing           | network.nix                 |
| `nixling-sys-<env>-net`                   | reserved auto-system VM name            | network.nix                 |
| `nixling-launcher` group                  | polkit principal for unit start/stop    | host-users.nix              |
| `nixling-<vm>-{gpu,snd,swtpm}` users      | dedicated per-VM sidecar uids           | host-users.nix              |

### Composition with framework-agnostic flakes (`nixos-entra-id`)

Nixling deliberately does **not** ship per-domain modules (Entra ID
device-join, corporate VPN clients, vendor identity glue). Those live
in sibling flakes that are framework-agnostic — they can in principle
be imported into any NixOS configuration, microVM or bare metal.
[`vicondoa/nixos-entra-id`](https://github.com/vicondoa/nixos-entra-id)
is the canonical example.

The split:

- **Nixling owns:** VM / env / sidecar lifecycle, network isolation
  (per-env bridges + net VM + NAT), per-VM `/nix/store` hardlink
  farm, the `nixling` CLI, and the host-side polkit + key
  management. Anything that only makes sense on a microVM host.
- **Domain flakes own:** identity (Himmelblau / Entra), corporate
  trust roots, vendor-specific guest kernel modules, anything that
  is a property of the *guest workload* rather than the *host
  framework*.
- **The seam** is one line in the consumer's flake:

  ```nix
  nixling.vms.work-vm.config.imports = [
    inputs.nixos-entra-id.nixosModules.default
  ];
  ```

  Nixling does not depend on `nixos-entra-id`, and `nixos-entra-id`
  does not depend on nixling — they meet only in the consumer
  flake's `config.imports`.

Why: nixling stays minimal and framework-agnostic. Domain flakes
stay reusable outside the microVM context. Neither tree has to
track the other's release cadence or test matrix.

See [`examples/with-entra-id/`](../../examples/with-entra-id/) for
the full composition pattern (one work VM with `tpm.enable = true`
+ the Entra module imported into its guest config).

## 4. Defenses in depth

For each defense below, the threat it addresses is named explicitly.
The list is not exhaustive — it covers the load-bearing controls.

### Per-VM dedicated system users

**Threat:** a compromised sidecar reads or modifies another VM's
state (TPM blob, GPU buffers, audio state file).

**Control:** [`nixos-modules/host-users.nix`](../../nixos-modules/host-users.nix)
declares one user per sidecar per VM (`nixling-<vm>-gpu`,
`nixling-<vm>-snd`, `nixling-<vm>-swtpm`), all `isSystemUser =
true`, no `DynamicUser`. Each unit's `User=` /`Group=` pin to the
matching pair. Cross-VM file access fails on Unix DAC alone
(0700 on the per-VM state dir, 0600 on the swtpm control socket)
before any further sandboxing is needed.

### systemd hardening

**Threat:** a sidecar compromise escapes into the host filesystem
or kernel.

**Control:** unit options layered on each sidecar:

| Sidecar         | NNP | ProtectSystem | RAFmilies                                | MDWE  | Notes                                |
|-----------------|-----|---------------|-------------------------------------------|-------|--------------------------------------|
| `-swtpm`        | yes | strict        | `AF_UNIX`                                  | yes   | TPM is purely local-Unix sockets     |
| `-gpu`          | yes | strict        | `AF_UNIX AF_NETLINK AF_VSOCK`              | **no** | crosvm JITs GPU command buffers      |
| `-snd`          | yes | strict        | as needed for PipeWire                     | yes   | see `components/audio/host.nix`      |
| usbipd backend  | yes | (relaxed)     | `AF_INET AF_INET6 AF_UNIX AF_NETLINK`      | n/a   | needs `/sys/bus/usb` enumeration     |
| usbipd proxy    | yes | strict        | none-via-CapBoundingSet=""                 | yes   | `systemd-socket-proxyd`              |

`MemoryDenyWriteExecute` is intentionally omitted on the GPU
sidecar at
[`nixos-modules/host-sidecars.nix:152-153`](../../nixos-modules/host-sidecars.nix):
crosvm's `device gpu` JITs GPU command-buffer code and breaks
under `MDWE`. That gap is an honest cost of running the patched
crosvm GPU forwarder; the surrounding `ProtectSystem=strict`,
`DevicePolicy=closed + DeviceAllow=[…]`, narrow `ReadWritePaths`,
and dedicated UID are the compensating controls.

### Polkit allowlist (exact-unit, narrow-verb)

**Threat:** an over-broad polkit rule lets a launcher user touch
units the framework doesn't own (the consumer's other microvm.nix
VMs, system-wide services, the polkit daemon itself).

**Control:** [`nixos-modules/host-polkit.nix`](../../nixos-modules/host-polkit.nix)
generates a JS array literal containing exactly the units this
framework materialises — `nixling@<vm>.service`,
`nixling-<vm>-store-sync.service`, plus the optional
`-gpu`/`-snd`/`-swtpm` triplets and the per-env usbipd units.
The rule checks `action.id`, then `verb ∈ {start, stop, restart}`,
then a literal-equality lookup in the array. `reload`,
`try-restart`, `enable`, `mask`, etc. still require a password.
The W2-era wildcard (`microvm@*`, `nixling-*`) is gone; an
upstream microvm.nix VM declared outside `nixling.vms` is not in
the allowlist.

A second rule lets `nixling-<vm>-gpu` start/stop/restart **only**
the matching `nixling-<vm>-snd.service`, for the case where
cloud-hypervisor is launched directly by `microvm-run` and needs
to ensure its audio sidecar is up.

### CIDR overlap validation (eval-time, fail-closed)

**Threat:** two envs with overlapping LAN subnets, an env LAN that
collides with the host's primary LAN, or a misconfigured
`uplinkSubnet` produce silent routing-table conflicts that
re-route traffic the operator believed isolated.

**Control:** [`nixos-modules/network.nix:213-275`](../../nixos-modules/network.nix)
runs pure-Nix IPv4 prefix arithmetic (via `lib.nix`'s
`cidrOverlaps`) over every pair of `{env, kind, cidr}` tuples,
including the host's `nixling.hostLanCidrs`. Any overlap aborts
evaluation with a message naming both sides. Exact-string-equality
was the previous check and missed real overlaps like `10.0.0.0/16
⊃ 10.0.1.0/24`.

### Route preflight, fail-closed

**Threat:** a stale or operator-added static route on the host
sends an env's LAN traffic via the wrong interface — typically
because an env's CIDR was changed and the old route was never
withdrawn — and the workload's traffic ends up egressing the
host's primary LAN instead of the env's net VM.

**Control:** `nixling-net-route-preflight.service`
([`nixos-modules/network.nix:543-586`](../../nixos-modules/network.nix))
runs at boot and probes `ip route get <env-lan>.10` for every env.
If the resolved next-hop is not the env's `uplinkBridge`, the
unit exits 1. Every nixling-managed VM unit (`nixling@<vm>`)
declares `Requires=` on this preflight, so a failed preflight
refuses to start any VM. `RemainAfterExit=true` keeps the unit
"active" between probes so the dependency is well-defined.

### Per-env USBIP, no host-wide singleton

**Threat:** a single host-wide `nixling-sys-usbipd.service`
binding to `127.0.0.1:3241` would be one misconfigured firewall
rule away from leaking USB device export to *every* env.

**Control:** [`nixos-modules/network.nix:484-587`](../../nixos-modules/network.nix)
declares one backend + one proxy per env, on distinct loopback
ports (`3241 + alphabetical-index-of-env`). The proxy socket
binds the env's `hostUplinkIp:3240` only. Three iptables rules
per env enforce this at the firewall layer too: an ACCEPT for
the env's own uplinkSubnet → 3240, a DROP for any other source
→ 3240 on the same bridge, and a DROP for non-loopback traffic
to the backend port. Rules are inserted at `nixos-fw 1` so they
fire before any NixOS-generated accept. The CLI's
`do_usb` / `do_up` paths add a fourth layer of defense by
stopping every non-target env's backend + socket before binding
a device into the kernel's usbip-host namespace (single-bind
invariant; documented in cli.nix's exclusive-export block).

### MAC sentinel for net-VM DHCP catch-all

**Threat:** the base.nix catch-all `10-eth-dhcp` network
(`matchConfig.Type = "ether"`) DHCPs both NICs on a net VM,
preempting the static `10-uplink` / `10-lan` definitions, and the
env's whole addressing plan dies silently.

**Control:** [`nixos-modules/net.nix:55-57`](../../nixos-modules/net.nix)
uses `lib.mkForce` to replace the catch-all's `matchConfig`
with a sentinel MAC (`00:00:00:00:00:00`) that no interface ever
exposes. systemd-networkd writes a harmless `.network` file that
matches nothing, the static configs win on priority, and workload
VMs (which still want the catch-all) are unaffected. See
[§6](#why-mac-sentinel-instead-of-mkforce-removal) for why this
shape was preferred over the obvious alternatives.

### SSH key generation by the framework, not the flake

**Threat:** baking per-VM SSH private keys into the flake closure
puts them in the world-readable `/nix/store` and ties their
rotation to a rebuild.

**Control:** keys are generated on the host at activation time by
[`nixos-modules/host-keys.nix`](../../nixos-modules/host-keys.nix),
written to `/var/lib/nixling/keys/<vm>_ed25519` (mode 0640, root-
owned, ACL'd `r` for `nixling-launcher`), and never enter the
store. The CLI consumes them through `keysDir`; the guest gets
only the corresponding pubkey via a virtio-fs share. Rotation is
a single `nixling keys rotate <vm>` invocation; no rebuild
required for that path.

The trade is honest: the private key is readable by every member
of `nixling-launcher`. That is intentional within the single-user
threat model — the launcher group is the human and the human's
own service principals. It is not a defense against a second
human on the same machine.

## 5. Limitations and known gaps

Tracked openly in `CHANGELOG.md`. Summarised here so the threat
model is honest about its incomplete edges:

- **USBIP per-env units materialise even when no VM in the env
  opts in.** Each declared env produces
  `nixling-sys-<env>-usbipd-{backend,proxy}.service` regardless
  of whether any workload sets `usbip.yubikey = true`. The units
  sit idle until something binds, but they are still installed.
  The relevant conditional belongs around
  [`nixos-modules/network.nix:484-650`](../../nixos-modules/network.nix);
  deferred to v0.2.0.
- **VM-to-VM east-west traffic within the same env is not
  supported.** Workload taps on the per-env LAN bridge are
  declared with `Isolated = true`, so two workload VMs sharing an
  env can each reach the net VM (and via NAT, the upstream LAN)
  but cannot directly reach each other. A future opt-out
  (`nixling.envs.<env>.intraLanIsolation = false`) is on the
  v0.2.0 wishlist; until then, treat each workload VM as a
  point-to-point endpoint of its env's gateway.
- **No static lint for the `mkOption { default = …; readOnly =
  true; }` + matching `config.<…>` assignment trio.** A
  reviewer-found bug in W5 (the `nixling.manifest` "set multiple
  times" defect when graphics VMs were synthesised) was caught
  by humans, not tooling. Phase 7a-followup will add a grep-level
  lint. Note that `store.nix` legitimately carries
  `readOnly + default` on options that have NO matching
  `config.<…>` assignment, so a two-of-three match is fine —
  only the full three is a bug.
- **`pkgs/spectrum-ch/default.nix` deliberately omits
  `meta.platforms`.** The other patched packages
  (`crosvm-patched`, `crosvm-seccomp`, `vhost-device-sound`) pin
  to `x86_64-linux`, but spectrum-ch intentionally does not.
  See the in-file comment. The platform gate is enforced at the
  `microvm.vms` translation point in `host.nix` instead.
- **`nixling.site.stateDir` and `keysDir` are advisory in
  v0.1.0.** Several modules still hardcode `/var/lib/nixling`
  and `/var/lib/nixling/keys`. Overriding the options today
  will leave stale entries on disk. Full threading is tracked
  for v0.2.0.
- **Audio mic/speaker enforcement is via PipeWire stream rules, not
  the kernel.** The framework injects per-direction PipeWire
  `client.conf` `stream.rules` keyed on the sidecar-advertised
  `nixling.mic` / `nixling.speaker` flags (see
  [`nixos-modules/components/audio/host.nix:432-469`](../../nixos-modules/components/audio/host.nix)),
  so a guest cannot reach the host's microphone or speakers when its
  side is set to `off`. The remaining caveat is that this enforcement
  lives in the host user's PipeWire session, not in the kernel — a
  privileged adversary on the host's session bus could in principle
  inspect stream presence (not content) via PipeWire introspection.
  Considered acceptable: the host's session bus is already in the
  trusted boundary, and `nixling audio` is the explicit toggle.

None of these gaps undermine the load-bearing isolation
boundaries; they are sharp edges around configurability and
audit ergonomics.

## 6. Why not X — design rationale FAQ

These are the questions that came up most often during the
refactor that brought nixling out of a personal NixOS host into
a standalone flake. The short answers are here; longer rationale
lives in the cited code.

### Why not just use microvm.nix directly?

Because microvm.nix is a primitive, and most of the bugs and
attacks that matter at the desktop-workspace layer are above
its level. microvm.nix gives you a VM; nixling gives you the
env model, the per-VM isolation glue, the polkit-and-keys story,
the audit conventions, and a single CLI that operates on those
abstractions. A `nixling.vms.<vm>` declaration is ~10 lines.
Doing the same thing by hand with microvm.nix is ~150 lines of
bridge plumbing, networkd rules, swtpm setup, sidecar
hardening, and key-management activation scripts — and every
one of those is an opportunity for a config drift across VMs.

### Why not multi-user / multi-tenant?

The trust-boundary work to make `nixling-launcher` a real
multi-principal grant — narrowing *which* user can drive
*which* VM, splitting `keysDir` access per principal, modelling
cross-user audit — multiplies the option surface and breaks
several of the simplifying assumptions the CLI makes today
(global flock files, shared `known_hosts`, single Wayland
user). Nixling targets the single-user desktop. Multi-tenant
desktop VM hosts are a different product, and Qubes is a much
better answer to that question than nixling could ever be.

### Why Wayland-only?

X11 has no display-server-level isolation. Two X clients on the
same display see each other's keystrokes by default, can read
each other's window contents trivially, and have no per-app
socket boundary. Wayland's per-app socket model maps cleanly to
per-VM forwarding: one wayland-0 per guest, mediated by a
patched crosvm GPU sidecar, ACL'd to the per-VM sidecar user.
The framework also does not want to maintain an X11 fallback
in parallel — the threat-modelling on it would be
substantially weaker than the Wayland path, and shipping a
weaker default just to support X is a bad trade.

### Why not OCI / containers?

Insufficient kernel-level isolation for the "risky-dev"
workspace use case. A browser sandbox escape inside a container
is still on the host kernel; a comparable escape in a VM is
bounded by KVM. For workloads where that boundary is the whole
point, container-grade isolation is the wrong tool. Lighter
workloads (CI runners that don't touch the desktop, daemons
that fit in a NixOS container) are fine in containers and not
in scope for nixling.

### Why per-VM `/nix/store` instead of the shared default?

Three reasons:

1. **Closure separation.** A workload VM literally cannot see
   store paths that aren't in its closure. This is audit-by-
   construction: you don't have to ask "could this VM read that
   package?", because the answer is no, the path is not in its
   farm.
2. **Smaller attack surface inside the guest.** Each VM's
   `nix-store --query --valid` and `nix-shell` only know about
   what the framework loaded into its DB at boot. There is no
   shared host store for a compromised guest to enumerate.
3. **Auditable bytes, no extra disk.** Hardlinks share inodes
   with the host's real `/nix/store`. The farm is a directory
   of names, not a copy of files; host garbage collection only
   trims what isn't pinned by any VM's generation.

The cross-mount hardlink trick (described in §3) is the cost
of admission. It is bounded — a single sync helper runs in a
private mount namespace — and the payoff is that the
isolation property is structural rather than policy-based.

### Why not Spectrum or Qubes?

Different design points. [Spectrum] is the project we lift
the patched cloud-hypervisor from (`pkgs/spectrum-ch`), and we
owe them the cross-domain Wayland forwarding work. But Spectrum
is its own OS, with its own assumptions; nixling is a flake
that drops into a NixOS configuration the consumer already
runs. [Qubes OS][qubes] is Fedora-based, uses Xen rather than
KVM, and has a strict GUI domain / network domain / template
domain trust model that does not map onto a single-user
NixOS host without rewriting the userland from scratch. The
two ecosystems target different platforms and different
threat models. Nixling's pitch is "you already run NixOS,
here's the workspace framework that fits there"; Qubes is
"start over on a different OS designed for this from the
ground up".

[Spectrum]: https://spectrum-os.org/
[qubes]: https://www.qubes-os.org/

### Why MAC sentinel instead of `mkForce`-removal?

Two cleaner-looking alternatives both have real problems.

The first is `networkConfig.DHCP = lib.mkForce "no"` on the
catch-all. That works, but the network is then still
materialised — systemd-networkd writes a `.network` file, the
name still sorts lex-first, and a future workload-VM
extension that wants the catch-all back has to undo a
`mkForce` instead of just setting it. Per-attribute overrides
are also more fragile to read at review time than a single
"this whole entry is neutralised" overlay.

The second is removing the catch-all entirely via
`systemd.network.networks."10-eth-dhcp" = lib.mkForce { }` or
`lib.mkOverride 30 null`. Removing a whole attribute is fiddly
in the nixpkgs module system and tends to lose attribute
provenance — future readers see a hole where there used to be
a config and don't know it came from base.nix.

The MAC sentinel — `lib.mkForce { matchConfig.MACAddress =
"00:00:00:00:00:00"; }` — keeps the entry materialised (so
future overlays compose), leaves the original intent visible
(the file is still called `10-eth-dhcp`, still imported by the
same base), and produces an unambiguous "this matches nothing"
signal at the systemd-networkd level. It is the minimum
mechanical change that fixes the lex-sort preemption
([`nixos-modules/net.nix:47-57`](../../nixos-modules/net.nix)).

## 7. References

Inside this repo:

- [`docs/reference/manifest-schema.md`](../reference/manifest-schema.md) —
  prose walkthrough of the per-VM JSON manifest at
  `/run/current-system/sw/share/nixling/vms.json`, plus the v1
  compatibility policy.
- [`docs/reference/manifest-schema.json`](../reference/manifest-schema.json) —
  the canonical JSON Schema Draft 2020-12 for the manifest.
- [`docs/reference/cli-contract.md`](../reference/cli-contract.md) —
  the behavioural contract for any `nixling` CLI implementation
  (subcommand inventory, lifecycle FSM, exit codes, signal
  semantics, JSON vs human output).
- [`CHANGELOG.md`](../../CHANGELOG.md) — version history,
  breaking changes, and the *Known gaps* section that feeds
  [§5](#5-limitations-and-known-gaps).
- [`examples/`](../../examples/) — runnable starters that
  demonstrate the design in practice (`minimal`,
  `graphics-workstation`, `multi-env`, `with-entra-id`).
- [`templates/default/`](../../templates/default/) —
  `nix flake init` scaffold with sentinel TODOs.

Upstream:

- [microvm.nix][microvm.nix] — the KVM-based microVM framework
  nixling composes on top of.
- [Spectrum OS][Spectrum] — origin of the patched cloud-
  hypervisor with virtio-gpu cross-domain support
  (`pkgs/spectrum-ch`).
- [systemd hardening reference][systemd-hardening] — the canon
  for the unit options used throughout
  `nixos-modules/host-sidecars.nix` and
  `nixos-modules/components/*.nix`.
- [Qubes OS][qubes] — different design point, referenced for
  contrast in [§6](#why-not-spectrum-or-qubes).

[systemd-hardening]: https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html
