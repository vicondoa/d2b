# Migrating from microvm.nix

> How-to: migrate an existing `microvm.nix` configuration to use `d2b`
> for richer host-side glue (networking, sidecars, CLI, per-VM /nix/store,
> SSH key management).
>
> Reading time: ~15 minutes.
> Difficulty: intermediate.

`d2b` does not replace [`microvm.nix`][upstream] — it sits on top of it.
The upstream module still owns the `microvm.vms.<vm>.config` evaluation,
the hypervisor runners, the volume/share machinery, and the
`microvm@<vm>.service` unit. `d2b` adds an opinionated layer above:
per-env networking (bridges + auto-declared NAT VM + dnsmasq + nftables),
per-VM `/nix/store` hardlink farms, SSH key lifecycle, a `d2b` CLI,
and component toggles (graphics, audio, TPM, USBIP).

This document is the recipe for porting a working `microvm.nix`
deployment over.

[upstream]: https://github.com/microvm-nix/microvm.nix

## Before you start

You should already have:

- A flake-based NixOS host with `microvm.nix` imported via
  `inputs.microvm.nixosModules.host`.
- At least one working `microvm.vms.<name>` declaration that you can
  `microvm -R <name>` or `systemctl start microvm@<name>` today.
- A handle on the upstream concepts: `interfaces`, `volumes`, `shares`,
  `hypervisor`, `microvm.stateDir`. If `microvm.interfaces` and "tap vs.
  user vs. macvtap" do not ring a bell, read the upstream README first.
- Root on the target host. Activation rewrites
  `microvm.stateDir`, drops new systemd units, and creates
  `/var/lib/d2b/`.

Pick a maintenance window. The first `nixos-rebuild switch` after
adoption will:

- Move state from your existing `microvm.stateDir` (commonly
  `/var/lib/microvms/`) to `/var/lib/d2b/vms/<vm>/`.
- Tear down whatever taps / bridges / dnsmasq instances you ran by hand
  and replace them with per-env bridges + an auto-declared net VM.
- Generate fresh per-VM Ed25519 SSH host keys under
  `/var/lib/d2b/keys/` — your existing in-VM `authorized_keys` for
  ssh-from-host will need to be updated (or just trust the framework's
  injection flow, below).

If any of that is a hard "no" today (e.g. you cannot accept losing
the existing taps), stop here and shrink the scope first.

## What changes

| Concern              | microvm.nix-only                                   | d2b                                                              |
| -------------------- | -------------------------------------------------- | -------------------------------------------------------------------- |
| VM declaration       | `microvm.vms.<vm>.config = { ... }`                | `d2b.vms.<vm>.config = { ... }` (env-aware, components-aware)    |
| Per-VM /nix/store    | shared host `/nix/store` via virtiofs              | per-VM hardlink farm under `/var/lib/d2b/vms/<vm>/store/`        |
| `microvm.stateDir`   | `/var/lib/microvms/` (upstream default)            | `/var/lib/d2b/vms/` (forced by `host.nix`)                       |
| Networking           | user-owned (`microvm.interfaces`, manual bridges)  | per-env net VM auto-declared as `sys-<env>-net` + bridges            |
| DHCP / DNS for VMs   | user-owned (`services.dnsmasq` on host, or static) | dnsmasq inside the per-env net VM, host-reservations from `index`    |
| Egress firewall      | host nftables                                      | nftables inside the net VM (`hostBlocklist` + RFC1918 DROP)          |
| VM-to-VM isolation   | shared bridge by default                           | one bridge per env, no inter-env forwarding                          |
| SSH into the VM      | bake keys into the guest's nixos config            | framework-managed per-VM Ed25519 key under `d2b.site.keysDir`    |
| Lifecycle commands   | `systemctl start microvm@<vm>` + `microvm -R`      | `d2b vm start\|vm stop\|switch\|build\|boot\|test\|status <vm>` (dispatches through `d2bd` → `d2b-priv-broker` per ADR 0015 v1.0 daemon-only) |
| Autostart            | `microvm.autostart` list                           | `d2b.vms.<vm>.autostart` per-VM bool                             |
| Graphics             | hand-rolled crosvm + virtio-gpu wiring             | `d2b.vms.<vm>.graphics.enable = true` (component toggle)         |
| TPM                  | hand-rolled `swtpm`, manual socket plumbing        | `d2b.vms.<vm>.tpm.enable = true`                                 |
| Audio                | hand-rolled vhost-user-sound                       | `d2b.vms.<vm>.audio.enable = true` + `d2b audio mic/speaker` |
| USBIP                | manual `usbipd` + `usbip attach` on the host       | `d2b usb attach <vm> <busid> --apply` (dispatches through d2bd → broker `SpawnRunner` per-env usbipd runner; legacy systemd `d2b-sys-<env>-usbipd-proxy.{service,socket}` was retired in v1.0 per ADR 0015) |
| Non-root start/stop  | sudo every time                                    | `d2b` group + SO_PEERCRED at `public.sock` accept time (v1.0 daemon-only per ADR 0015; the polkit per-VM allowlist was retired in v1.0) |
| Wrapping             | direct `microvm@<vm>.service`                      | wrapped by `d2bd` supervisor DAG + broker `SpawnRunner` per-runner pidfd ownership (the legacy `d2b@<vm>.service` wrapper was retired in v1.0) |

## Option mapping

For every common upstream pattern, the section below shows the
`d2b` equivalent. Examples are minimal — copy what you need
into your real config.

### Pattern: one VM, one tap, host runs dnsmasq

**microvm.nix:**

```nix
{
  microvm.vms.work = {
    config = { ... }: { networking.hostName = "work"; };
    interfaces = [{
      type = "tap";
      id = "vm-work";
      mac = "02:00:00:00:00:10";
    }];
    volumes = [{
      image = "/var/lib/microvms/work/root.img";
      mountPoint = "/";
      size = 4096;
    }];
  };
}
```

**d2b:**

```nix
{
  d2b.hostLanCidrs = [ "192.168.1.0/24" ];

  d2b.envs.work = {
    lanSubnet    = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };

  d2b.vms.work = {
    env    = "work";
    index  = 10;            # → 10.20.0.10
    ssh.user = "alice";
    config = { networking.hostName = "work"; };
  };
}
```

You did **not** write `interfaces`, the tap name, the MAC, or
`microvm.stateDir`. The framework derives all of them from
`(env, index)`. The auto-declared `sys-work-net` VM takes care of
DHCP + DNS + egress NAT on `br-work-lan`.

### Pattern: multiple VMs in one trust boundary

**microvm.nix:** you would declare one `microvm.vms.*` block per VM,
share a bridge by hand, and reserve MACs yourself.

**d2b:**

```nix
{
  d2b.envs.work = {
    lanSubnet    = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };

  d2b.vms.work-app = {
    env = "work"; index = 10; ssh.user = "alice";
    config = { networking.hostName = "work-app"; };
  };
  d2b.vms.work-db = {
    env = "work"; index = 11; ssh.user = "alice";
    config = { networking.hostName = "work-db"; };
  };
}
```

`index` is unique **per env**. Both VMs share `br-work-lan` and route
egress via `sys-work-net`, but they **cannot directly talk to each
other** — workload taps are marked `Isolated = true` in the LAN bridge
(see `nixos-modules/network.nix:376-386`), and the net VM does not
forward eth1→eth1 (`nixos-modules/net.nix:135-155`). If you need
explicit VM-to-VM traffic (e.g. service mesh inside an env), opt in
with the two-step unsafe acknowledgement:

```nix
d2b.site.allowUnsafeEastWest = true;
d2b.envs.work.lan.allowEastWest = true;
```

Leave both unset for the default isolated LAN.

### Pattern: multiple envs (work / personal)

```nix
{
  d2b.envs.work = {
    lanSubnet    = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };
  d2b.envs.personal = {
    lanSubnet    = "10.30.0.0/24";
    uplinkSubnet = "192.0.2.4/30";
  };

  d2b.vms.work-app = {
    env = "work"; index = 10; ssh.user = "alice";
    config = { networking.hostName = "work-app"; };
  };
  d2b.vms.personal-app = {
    env = "personal"; index = 10; ssh.user = "alice";
    config = { networking.hostName = "personal-app"; };
  };
}
```

Disjoint `lanSubnet` and `uplinkSubnet`. Reusing `index = 10` across
envs is fine — uniqueness is scoped per-env. There is no inter-env
route; the two LAN bridges are independent.

### Pattern: tunneled uplinks (per-env MTU + MSS clamp)

If an env rides a tunnel or overlay (WireGuard, Tailscale, VXLAN,
PPPoE, ...), set `mtu` to the effective path MTU and enable
`mssClamp` so forwarded TCP SYN packets advertise a safe MSS:

```nix
{
  d2b.envs.work = {
    lanSubnet = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
    mtu = 1280;
    mssClamp = true;
  };
}
```

`mtu` threads through the env's bridges, taps, and guest NICs.
`mssClamp = true` adds the net VM nftables rule
`tcp flags syn tcp option maxseg size set rt mtu`, which keeps
forwarded TCP flows aligned with the routed path MTU.

See `examples/multi-env/` for a fully-annotated version.

### Pattern: graphics-enabled VM

Anything you previously hand-wired for crosvm + virtio-gpu + Wayland
cross-domain goes away.

```nix
{
  d2b.site.waylandUser   = "alice";
  d2b.site.launcherUsers = [ "alice" ];

  d2b.envs.desktop = {
    lanSubnet    = "10.42.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };

  d2b.vms.workstation = {
    env = "desktop"; index = 10; ssh.user = "alice";
    graphics.enable = true;       # crosvm GPU sidecar + Wayland
    audio.enable    = true;       # vhost-user-sound → host PipeWire
    config = { networking.hostName = "workstation"; };
  };
}
```

`graphics.enable = true` implicitly pins `microvm.hypervisor =
"cloud-hypervisor"` — the only hypervisor wired for the GPU sidecar.
Do not also start it with `systemctl start microvm@workstation`: use
`d2b vm start workstation` from a Plasma/sway/Hyprland terminal so the
sidecar can reach `$WAYLAND_DISPLAY`. See `examples/graphics-workstation/`.

### Pattern: TPM-backed VM

**microvm.nix:** hand-roll `swtpm` socket, point the runner at it.

**d2b:**

```nix
d2b.vms.secure = {
  env = "work"; index = 12; ssh.user = "alice";
  tpm.enable = true;            # swtpm + CRB on /dev/tpm0 in guest
  config = { networking.hostName = "secure"; };
};
```

`tpm.enable = true` pins cloud-hypervisor and points TPM state at
`/var/lib/d2b/vms/secure/swtpm/`. Treat that directory as secret;
back up only to encrypted, access-controlled media.

### Pattern: YubiKey passthrough

```nix
d2b.site.yubikey.enable = true;       # host udev; usbip-host loads on per-VM opt-in

d2b.vms.work-app = {
  env = "work"; index = 10; ssh.user = "alice";
  usbip.yubikey = true;                   # guest vhci_hcd + usbip CLI
  config = { networking.hostName = "work-app"; };
};
```

Then `d2b usb attach work-app <busid> --apply` from the host
attaches a plugged-in YubiKey via the per-env usbipd broker-spawned
runner under `d2b.slice/sys-work/usbipd-proxy` (v1.0 per ADR
0015; the legacy `d2b usb work-app` bash orchestrator + the
per-env `d2b-sys-work-usbipd-proxy.service` systemd unit were
retired in v1.0). Ctrl-C detaches.

### Pattern: keeping legacy / hand-rolled networking

If you have one VM you cannot port yet (e.g. it needs a macvtap onto
a physical NIC), leave `env = null` and keep using
`microvm.interfaces` and `systemd.network` inside `config`:

```nix
d2b.vms.legacy = {
  env = null;                              # opt out of per-env wiring
  ssh.user = null;                         # d2b CLI won't ssh in
  config = { ... }: {
    microvm.interfaces = [{
      type = "macvtap";
      id   = "vm-legacy";
      mac  = "02:00:00:00:00:99";
      macvtap = { link = "eno1"; mode = "bridge"; };
    }];
    systemd.network.networks."10-eth" = { /* ... */ };
  };
};
```

The framework still gives you a per-VM `/nix/store`, the unit wrapper,
and the manifest entry — just no env-derived addressing.

### Pattern: per-VM overrides of upstream knobs

`d2b.vms.<vm>.config` is just a NixOS module merged into the guest.
It can carry `microvm.*` options directly:

```nix
d2b.vms.work-app = {
  env = "work"; index = 10; ssh.user = "alice";
  config = {
    networking.hostName = "work-app";
    microvm.mem  = 4096;
    microvm.vcpu = 4;
    microvm.volumes = [{
      image = "data.img";
      mountPoint = "/var/lib/app";
      size = 8192;
    }];
    # Hypervisor override. Without graphics/tpm/audio the framework
    # leaves microvm.hypervisor alone, so this is yours to set.
    microvm.hypervisor = "qemu";
  };
};
```

Do **not** declare `microvm.shares = [{ source = "/nix/store"; ... }]`
in here — the framework injects per-VM store + store-meta shares
with `lib.mkForce` and a duplicate will produce a confusing module
conflict.

## Step-by-step migration

The ordering below is deliberate: **all read-only and code-only steps
come before any on-disk state moves**, so that a failed eval or build
leaves your host in exactly the pre-migration state. State migration
(stopping VMs + moving directories) happens only after the new config
has built cleanly. See "Rollback" at the end of this section.

1. **Inventory** (read-only). List every VM you currently declare:

   ```bash
   nix eval .#nixosConfigurations.<host>.config.microvm.vms \
     --apply 'builtins.attrNames'
   ```

   Write the list down. Every name here becomes a `d2b.vms.<name>`.

2. **Group by trust / network boundary, pick CIDRs** (read-only / planning).

   A "trust boundary" is the coarsest answer to "which of these VMs
   are allowed to see each other on the LAN, and which must be
   quarantined?" Each group becomes one `d2b.envs.<env>`. Most
   consumers end up with 1–3 envs (e.g. `work`, `personal`, `lab`).

   For each env, choose:

   - `lanSubnet` — a `/24` for the workload bridge. Must not overlap
     `d2b.hostLanCidrs` or any other env's `lanSubnet`.
   - `uplinkSubnet` — a `/30` for the host↔net-VM point-to-point link.
     Pick from RFC 5737 (`192.0.2.0/24`, `198.51.100.0/24`,
     `203.0.113.0/24`) so the addresses visibly belong to d2b.

   Also set `d2b.hostLanCidrs` from `ip route` output (capture
   every CIDR the host sits on). The framework unions this into
   every env's `hostBlocklist`, so VMs cannot reach the host's
   neighbours.

   For each VM, decide whether it needs `graphics.enable`,
   `audio.enable`, `tpm.enable`, or `usbip.yubikey = true`. If
   anything turns on graphics or audio, plan to set
   `d2b.site.waylandUser` to the desktop session user.

3. **Add the flake input + import the module** (code only, no
   activation). In your top-level `flake.nix`:

   ```nix
   {
     inputs = {
       nixpkgs.url   = "github:NixOS/nixpkgs/nixos-unstable";
       microvm.url   = "github:microvm-nix/microvm.nix";
       microvm.inputs.nixpkgs.follows = "nixpkgs";

       # IMPORTANT: pin >= v0.1.6. Earlier v0.1.x releases shipped
       # known framework bugs surfaced by the first real consumer
       # migration (graphics-VM /dev/net/tun device-deny, swtpm
       # parent-dir ACL gap, route-preflight bootstrap deadlock,
       # nixos-rebuild restarting VMs mid-flight). v0.1.5 fixed the
       # code; v0.1.6 ships the matching docs catch-up so this
       # how-to and the rest of the reference docs describe the
       # behaviour you're actually running.
       d2b.url = "github:vicondoa/d2b/v0.1.6";
       d2b.inputs.nixpkgs.follows = "nixpkgs";
       d2b.inputs.microvm.follows = "microvm";
     };
   }
   ```

   The `follows` lines keep your nixpkgs and microvm pins
   single-sourced. In the same flake's
   `nixosConfigurations.<host>.modules`, add:

   ```nix
   inputs.d2b.nixosModules.default
   ```

   Do **not** also import the upstream `microvm.nixosModules.host`
   manually — `d2b`'s default module pulls it in.

4. **Replace `microvm.vms.*` with `d2b.vms.*`** (code only).
   Apply the mapping from the previous section. For each VM `<name>`:

   - Drop `interfaces` (env-managed) and the manual MAC.
   - Move whatever was in `microvm.vms.<name>.config` into
     `d2b.vms.<name>.config` verbatim.
   - Keep `microvm.mem`, `microvm.vcpu`, `microvm.volumes`,
     `microvm.hypervisor` inside that `config` block — they still
     resolve.
   - Set `env`, `index`, `ssh.user`.

5. **Build, do not activate.** This is the last reversible-by-edit
   step:

   ```bash
   nixos-rebuild build --flake .#<host>     # eval + build, NO activation
   ```

   Eval-time assertions catch most mistakes here:

   - CIDR overlap between envs or against `hostLanCidrs`.
   - `graphics.enable = true` with `waylandUser = null`.
   - `env` name longer than 8 chars (IFNAMSIZ limit on
     `br-<env>-lan`).
   - Duplicate `index` within an env.

   If this fails, no state has moved — fix your config and re-run.

6. **Stop running VMs.** Once the build is clean:

   ```bash
   for vm in <names>; do
     systemctl stop microvm@$vm
   done
   ```

7. **Move state directories** (the first non-reversible step):

   ```bash
   for vm in <names>; do
     mv /var/lib/microvms/$vm /var/lib/d2b/vms/$vm
   done
   ```

   Volumes referenced by absolute path inside `microvm.volumes` need
   their `image = ...` paths updated to the new location. Volumes
   referenced by bare filename (relative to `microvm.stateDir`) move
   transparently because the framework forces `microvm.stateDir =
   /var/lib/d2b/vms`.

8. **Activate.**

   ```bash
   nixos-rebuild switch --flake .#<host>     # commit, restart units
   ```

9. **Verify.**

   ```bash
   d2b list                       # what's declared + status
   d2b status <vm>                # per-VM health
   d2b vm start <vm> --apply      # bring up (graphics: needs Wayland)
   d2b switch <vm> --apply        # push a new closure live
   ```

   For headless VMs, `autostart = true` plus
   `d2b vm list` will show the broker-spawned runner state
   (`d2b vm start <vm>` registers the runner in the supervisor
   pidfd table). SSH into each migrated VM to confirm reachability.

### After every subsequent `nixos-rebuild switch` (v1.0)

In v1.0 (per ADR 0015) `d2bd` and `d2b-priv-broker` are the
only persistent system units the framework declares; rebuilds update
the systemd unit files and `/etc/d2b/{bundle,host,processes,
privileges}.json` but the broker's per-runner pidfd ownership
protects in-flight session state (interactive Wayland clients,
in-RAM Entra device-bound tokens, virtiofsd socket handshakes) —
the runners are not respawned. Use `d2b vm restart <vm> --apply`
to explicitly cycle a VM after a rebuild.

After `nixos-rebuild switch`, check whether any VM has pending
changes:

```bash
d2b list
```

A VM with a drift between its declared closure and its booted
closure is flagged in the STATUS column:

```
NAME             ENV    GRAPHICS TPM   USBIP   STATIC_IP       STATUS
work             work   true     true  true    10.20.0.10      running [pending restart]
```

Apply with:

```bash
d2b vm restart <vm> --apply
```

(Or `d2b switch <vm> --apply` if you want a per-VM closure rebuild +
live activation via SSH; restart cycles the existing closure
cleanly.)

`d2b status <vm>` prints both the `booted` and `current`
store paths plus the exact remediation command, so the user
doesn't have to memorize which command applies which kind of
change. For the full predicate semantics see
[`docs/reference/cli-contract.md` — Pending-restart signal](../reference/cli-contract.md#pending-restart-signal-v015).

### Rollback

- **Step 5 (build) fails:** no on-disk state has moved. Revert your
  config changes (`git checkout -- .` or undo the edits from steps
  3–4) and rebuild against the old config — your existing VMs are
  untouched.
- **Steps 6–7 succeed but step 8 (`switch`) fails:** the new closure
  is built but not active; state directories have been renamed. To
  roll back: revert the config, move state back
  (`mv /var/lib/d2b/vms/<vm> /var/lib/microvms/<vm>` for each
  VM), and `nixos-rebuild switch --flake .#<host>` against the old
  config. Start the VMs with `systemctl start microvm@<vm>` as
  before.
- **Step 9 verification fails on a specific VM** but activation
  succeeded: prefer fixing forward (the per-env net VM may take a
  few seconds to come up; check `d2b status sys-<env>-net` and
  the troubleshooting section). If a deeper rollback is needed,
  `nixos-rebuild switch --rollback` reverts to the previous
  generation, then move state back as above.

## What microvm.nix users gain

- **Per-env network isolation.** NAT-only egress, no inter-env
  routing, host-LAN drop rule applied by default.
- **Per-VM /nix/store.** Each guest sees only its own closure plus
  the microvm.nix runner — a closure-limited `/nix/store` view backed
  by a per-VM hardlink farm under `/var/lib/d2b/vms/<vm>/store/`.
  Zero byte duplication. `d2b switch <vm> --apply` updates it live
  without a VM reboot. Back up `/var/lib/d2b/` only to encrypted,
  access-controlled media.
- **Explicit lifecycle.** In v1.0 (per ADR 0015) `d2b vm start /
  stop / restart` dispatch through `d2bd` → `d2b-priv-broker`;
  the broker's `SpawnRunner` / `SignalRunner` ops + supervisor pidfd
  table are the lifecycle-of-record. Single commands, clear exit
  codes (`docs/reference/cli-contract.md`).
- **CLI ergonomics.** `d2b vm start / vm stop / status / list /
  audio / usb` — no more remembering tap names, MAC byte counts, or
  which env's usbipd is bound to which `192.0.2.X`.
- **SSH key management.** Per-VM Ed25519 keys generated at activation,
  ACL'd to the `d2b` group, injected into the guest
  at boot via `d2b-load-host-keys.service`. No flake-baked keys.
- **Permission boundary.** Members of `d2b` can drive
  `vm start` / `vm stop` / `vm restart` against `d2bd`'s public
  socket (mode 0660, group `d2b`); `SO_PEERCRED` at
  accept time is the authorisation surface. The legacy polkit per-VM
  allowlist was retired in v1.0 (ADR 0015).

## What microvm.nix users lose / what's d2b-only

- **Single-user assumption.** `d2b.site.waylandUser` is a single
  string — graphics + audio sidecars bind that user's
  `/run/user/<uid>/wayland-0` and `pipewire-0`. Multi-user desktops
  need additional work (out of scope for v0.1.0).
- **Wayland-only graphics.** The crosvm GPU sidecar speaks Wayland
  cross-domain. No X11 fallback.
- **x86_64-linux only for graphics + audio.** The crosvm + spectrum-ch +
  vhost-device-sound chain is gated to `x86_64-linux` via
  `meta.platforms`. Headless VMs evaluate on aarch64; graphics/audio
  VMs throw at eval time on non-x86_64.
- **Higher-level options shadow some upstream knobs.** Setting
  `graphics.enable = true` pins `microvm.hypervisor = cloud-hypervisor`
  (via `lib.mkDefault`); the same applies to `tpm.enable` and
  `audio.enable` (the vhost-user-sound device is wired only via
  cloud-hypervisor's `--device` plumbing — see
  `nixos-modules/components/audio/guest.nix:121-127`). You can still
  override per-VM via `d2b.vms.<vm>.config.microvm.hypervisor = ...`
  for headless VMs.
- **Framework-owned shares.** Do not add a `/nix/store` entry to
  `microvm.shares` in `d2b.vms.<vm>.config` — the framework
  injects it with `lib.mkForce`.
- **The daemon owns VM lifecycle.** In v1.0 (per ADR 0015) per-VM
  lifecycle moved fully to `d2bd` -> `d2b-priv-broker` via the
  supervisor DAG; the legacy systemd wrapper path was retired. Use
  `d2b vm start / vm stop / vm restart` for day-to-day lifecycle.

## Naming conventions you'll see post-migration

- `br-<env>-lan` — workload LAN bridge for env `<env>`.
- `br-<env>-up` — point-to-point host↔net-VM bridge.
- `sys-<env>-net` — auto-declared net VM (NAT + dnsmasq + nftables).
- `vm-<vm>-<env>` / `vm-<vm>-up` — taps on the bridges above.
- `d2b-sys-<env>-usbipd-proxy.service` — host-side USBIP proxy
  per env (retired as a host singleton and now a broker-spawned
  runner per ADR 0015; the unit name above is preserved as the
  cgroup leaf identifier).
- `d2bd.service` — daemon control plane (read-only RPCs + dispatch
  to broker; never root).
- `d2b-priv-broker.{service,socket}` — socket-activated privileged
  broker (single audited host-mutation surface; see
  [`docs/reference/privileges.md`](../reference/privileges.md)).
- broker-spawned VM runners — in v1.0 the broker `SpawnRunner` path is
  the lifecycle of record.
- `d2b` — host group whose members can drive `vm start
  / vm stop / vm restart` against `d2bd`'s public socket (mode
  0660, group `d2b`).

## Backup / state directories

- Upstream defaults: `microvm.stateDir = /var/lib/microvms/<vm>/`.
- After migration: `microvm.stateDir = /var/lib/d2b/vms/`
  (forced in `nixos-modules/host.nix`). Per-VM state lives under
  `/var/lib/d2b/vms/<vm>/`.
- Per-VM `/nix/store` mirror: `/var/lib/d2b/vms/<vm>/store/`
  (hardlinks; same FS as `/nix/store` is required).
- SSH keys: `/var/lib/d2b/keys/<vm>_ed25519` (private) +
  `.pub`. Mode `0700`, ACL'd to `d2b`.
- TPM state (if `tpm.enable = true`):
  `/var/lib/d2b/vms/<vm>/swtpm/`. Treat as secret; back up only to
  encrypted, access-controlled media.

Back up `/var/lib/d2b/` only to encrypted, access-controlled media
(TPM NVRAM and per-VM SSH keys live there). Restoring requires the
same `d2b.site.keysDir` / `stateDir` layout — those are
advisory-only in v0.1.0 and effectively hardcoded.

## Troubleshooting

**Eval fails with `d2b.envs.<env>: lanSubnet overlaps …`.**
You picked a LAN subnet that overlaps `d2b.hostLanCidrs` or
another env's `lanSubnet`. Pick a disjoint `/24`.

**Eval fails with `graphics.enable = true` but `waylandUser = null`.**
Set `d2b.site.waylandUser = "<your-user>"` and declare that user
in `users.users`. The user must have a running Wayland session at the
time `d2b vm start <vm>` runs.

**`d2b vm start <vm>` fails: `cannot find $WAYLAND_DISPLAY`.**
You ran it over SSH or as root. Graphics VMs require a terminal
inside the host's Wayland session. Headless VMs work over SSH and
as root.

**Stale tap interface from the pre-migration setup.**
`ip link delete vm-<oldname>` and rerun `nixos-rebuild switch`. The
framework only manages the taps it declares.

**`d2b switch <vm> --apply` errors with `cross-FS hardlink refused`.**
`/var/lib/d2b` and `/nix/store` are on different filesystems.
The per-VM store needs same-FS hardlinks; move
`/var/lib/d2b` to the same FS as `/nix/store` (typically by
remounting or relocating).

**`d2b vm start` is denied by the daemon socket.**
The invoking user is not in `d2b`. Add them to
`d2b.site.launcherUsers` (which only adjoins the group; you must
still declare the user) and re-log-in so the group membership
takes effect.

**SSH into the VM still uses your old key.**
The guest's `authorized_keys` is populated at boot by
`d2b-load-host-keys.service`. Restart the VM
(`d2b vm stop <vm> --apply && d2b vm start <vm> --apply`) or, inside the guest,
`systemctl restart d2b-load-host-keys.service`.

**`microvm.vms.<vm>` declared in two places.**
You left an old `microvm.vms.<name>` block alongside the new
`d2b.vms.<name>`. Remove the old one — the framework manages
the translation.

**Per-env net VM (`sys-<env>-net`) won't start.**
Check `d2b vm status sys-<env>-net` first. The most common cause is
that the env's `lanSubnet` is not a `/24` ending in `.0`, or
`uplinkSubnet` is not a `/30`. Eval should have caught this; if it
didn't, file an issue.

## See also

- [Design / threat model](../explanation/design.md)
- [Per-component reference](../reference/)
- [Manifest schema](../reference/manifest-schema.md)
- [CLI contract](../reference/cli-contract.md)
- [Examples](../../examples/)
- [CHANGELOG](../../CHANGELOG.md)
