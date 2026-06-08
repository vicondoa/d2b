# Migrating from microvm.nix

> How-to: migrate an existing `microvm.nix` configuration to use `nixling`
> for richer host-side glue (networking, sidecars, CLI, per-VM /nix/store,
> SSH key management).
>
> Reading time: ~15 minutes.
> Difficulty: intermediate.

`nixling` does not replace [`microvm.nix`][upstream] — it sits on top of it.
The upstream module still owns the `microvm.vms.<vm>.config` evaluation,
the hypervisor runners, the volume/share machinery, and the
`microvm@<vm>.service` unit. `nixling` adds an opinionated layer above:
per-env networking (bridges + auto-declared NAT VM + dnsmasq + nftables),
per-VM `/nix/store` hardlink farms, SSH key lifecycle, a `nixling` CLI,
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
  `/var/lib/nixling/`.

Pick a maintenance window. The first `nixos-rebuild switch` after
adoption will:

- Move state from your existing `microvm.stateDir` (commonly
  `/var/lib/microvms/`) to `/var/lib/nixling/vms/<vm>/`.
- Tear down whatever taps / bridges / dnsmasq instances you ran by hand
  and replace them with per-env bridges + an auto-declared net VM.
- Generate fresh per-VM Ed25519 SSH host keys under
  `/var/lib/nixling/keys/` — your existing in-VM `authorized_keys` for
  ssh-from-host will need to be updated (or just trust the framework's
  injection flow, below).

If any of that is a hard "no" today (e.g. you cannot accept losing
the existing taps), stop here and shrink the scope first.

## What changes

| Concern              | microvm.nix-only                                   | nixling                                                              |
| -------------------- | -------------------------------------------------- | -------------------------------------------------------------------- |
| VM declaration       | `microvm.vms.<vm>.config = { ... }`                | `nixling.vms.<vm>.config = { ... }` (env-aware, components-aware)    |
| Per-VM /nix/store    | shared host `/nix/store` via virtiofs              | per-VM hardlink farm under `/var/lib/nixling/vms/<vm>/store/`        |
| `microvm.stateDir`   | `/var/lib/microvms/` (upstream default)            | `/var/lib/nixling/vms/` (forced by `host.nix`)                       |
| Networking           | user-owned (`microvm.interfaces`, manual bridges)  | per-env net VM auto-declared as `sys-<env>-net` + bridges            |
| DHCP / DNS for VMs   | user-owned (`services.dnsmasq` on host, or static) | dnsmasq inside the per-env net VM, host-reservations from `index`    |
| Egress firewall      | host nftables                                      | nftables inside the net VM (`hostBlocklist` + RFC1918 DROP)          |
| VM-to-VM isolation   | shared bridge by default                           | one bridge per env, no inter-env forwarding                          |
| SSH into the VM      | bake keys into the guest's nixos config            | framework-managed per-VM Ed25519 key under `nixling.site.keysDir`    |
| Lifecycle commands   | `systemctl start microvm@<vm>` + `microvm -R`      | `nixling up\|down\|switch\|build\|boot\|test\|status <vm>` + polkit  |
| Autostart            | `microvm.autostart` list                           | `nixling.vms.<vm>.autostart` per-VM bool                             |
| Graphics             | hand-rolled crosvm + virtio-gpu wiring             | `nixling.vms.<vm>.graphics.enable = true` (component toggle)         |
| TPM                  | hand-rolled `swtpm`, manual socket plumbing        | `nixling.vms.<vm>.tpm.enable = true`                                 |
| Audio                | hand-rolled vhost-user-sound                       | `nixling.vms.<vm>.audio.enable = true` + `nixling audio mic/speaker` |
| USBIP                | manual `usbipd` + `usbip attach` on the host       | per-env `nixling-sys-<env>-usbipd-proxy` + `nixling usb <vm>`        |
| Non-root start/stop  | sudo every time                                    | `nixling-launcher` group + polkit allowlist on framework units       |
| Wrapping             | direct `microvm@<vm>.service`                      | wrapped by `nixling@<vm>.service` (lifecycle), upstream still works  |

## Option mapping

For every common upstream pattern, the section below shows the
`nixling` equivalent. Examples are minimal — copy what you need
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

**nixling:**

```nix
{
  nixling.hostLanCidrs = [ "192.168.1.0/24" ];

  nixling.envs.work = {
    lanSubnet    = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };

  nixling.vms.work = {
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

**nixling:**

```nix
{
  nixling.envs.work = {
    lanSubnet    = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };

  nixling.vms.work-app = {
    env = "work"; index = 10; ssh.user = "alice";
    config = { networking.hostName = "work-app"; };
  };
  nixling.vms.work-db = {
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
explicit VM-to-VM traffic (e.g. service mesh inside an env), that is
not yet supported; track it as a future feature request.

### Pattern: multiple envs (work / personal)

```nix
{
  nixling.envs.work = {
    lanSubnet    = "10.20.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };
  nixling.envs.personal = {
    lanSubnet    = "10.30.0.0/24";
    uplinkSubnet = "192.0.2.4/30";
  };

  nixling.vms.work-app = {
    env = "work"; index = 10; ssh.user = "alice";
    config = { networking.hostName = "work-app"; };
  };
  nixling.vms.personal-app = {
    env = "personal"; index = 10; ssh.user = "alice";
    config = { networking.hostName = "personal-app"; };
  };
}
```

Disjoint `lanSubnet` and `uplinkSubnet`. Reusing `index = 10` across
envs is fine — uniqueness is scoped per-env. There is no inter-env
route; the two LAN bridges are independent.

See `examples/multi-env/` for a fully-annotated version.

### Pattern: graphics-enabled VM

Anything you previously hand-wired for crosvm + virtio-gpu + Wayland
cross-domain goes away.

```nix
{
  nixling.site.waylandUser   = "alice";
  nixling.site.launcherUsers = [ "alice" ];

  nixling.envs.desktop = {
    lanSubnet    = "10.42.0.0/24";
    uplinkSubnet = "192.0.2.0/30";
  };

  nixling.vms.workstation = {
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
`nixling up workstation` from a Plasma/sway/Hyprland terminal so the
sidecar can reach `$WAYLAND_DISPLAY`. See `examples/graphics-workstation/`.

### Pattern: TPM-backed VM

**microvm.nix:** hand-roll `swtpm` socket, point the runner at it.

**nixling:**

```nix
nixling.vms.secure = {
  env = "work"; index = 12; ssh.user = "alice";
  tpm.enable = true;            # swtpm + CRB on /dev/tpm0 in guest
  config = { networking.hostName = "secure"; };
};
```

`tpm.enable = true` pins cloud-hypervisor and points TPM state at
`/var/lib/nixling/vms/secure/swtpm/`. Treat that directory as secret;
back up only to encrypted, access-controlled media.

### Pattern: YubiKey passthrough

```nix
nixling.site.yubikey.enable = true;       # host udev + usbip-host

nixling.vms.work-app = {
  env = "work"; index = 10; ssh.user = "alice";
  usbip.yubikey = true;                   # guest vhci_hcd + usbip CLI
  config = { networking.hostName = "work-app"; };
};
```

Then `nixling usb work-app` from the host attaches a plugged-in
YubiKey via the per-env `nixling-sys-work-usbipd-proxy.service`.
Ctrl-C detaches.

### Pattern: keeping legacy / hand-rolled networking

If you have one VM you cannot port yet (e.g. it needs a macvtap onto
a physical NIC), leave `env = null` and keep using
`microvm.interfaces` and `systemd.network` inside `config`:

```nix
nixling.vms.legacy = {
  env = null;                              # opt out of per-env wiring
  ssh.user = null;                         # nixling CLI won't ssh in
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

`nixling.vms.<vm>.config` is just a NixOS module merged into the guest.
It can carry `microvm.*` options directly:

```nix
nixling.vms.work-app = {
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

   Write the list down. Every name here becomes a `nixling.vms.<name>`.

2. **Group by trust / network boundary, pick CIDRs** (read-only / planning).

   A "trust boundary" is the coarsest answer to "which of these VMs
   are allowed to see each other on the LAN, and which must be
   quarantined?" Each group becomes one `nixling.envs.<env>`. Most
   consumers end up with 1–3 envs (e.g. `work`, `personal`, `lab`).

   For each env, choose:

   - `lanSubnet` — a `/24` for the workload bridge. Must not overlap
     `nixling.hostLanCidrs` or any other env's `lanSubnet`.
   - `uplinkSubnet` — a `/30` for the host↔net-VM point-to-point link.
     Pick from RFC 5737 (`192.0.2.0/24`, `198.51.100.0/24`,
     `203.0.113.0/24`) so the addresses visibly belong to nixling.

   Also set `nixling.hostLanCidrs` from `ip route` output (capture
   every CIDR the host sits on). The framework unions this into
   every env's `hostBlocklist`, so VMs cannot reach the host's
   neighbours.

   For each VM, decide whether it needs `graphics.enable`,
   `audio.enable`, `tpm.enable`, or `usbip.yubikey = true`. If
   anything turns on graphics or audio, plan to set
   `nixling.site.waylandUser` to the desktop session user.

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
       nixling.url = "github:vicondoa/nixling/v0.1.6";
       nixling.inputs.nixpkgs.follows = "nixpkgs";
       nixling.inputs.microvm.follows = "microvm";
     };
   }
   ```

   The `follows` lines keep your nixpkgs and microvm pins
   single-sourced. In the same flake's
   `nixosConfigurations.<host>.modules`, add:

   ```nix
   inputs.nixling.nixosModules.default
   ```

   Do **not** also import the upstream `microvm.nixosModules.host`
   manually — `nixling`'s default module pulls it in.

4. **Replace `microvm.vms.*` with `nixling.vms.*`** (code only).
   Apply the mapping from the previous section. For each VM `<name>`:

   - Drop `interfaces` (env-managed) and the manual MAC.
   - Move whatever was in `microvm.vms.<name>.config` into
     `nixling.vms.<name>.config` verbatim.
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
     mv /var/lib/microvms/$vm /var/lib/nixling/vms/$vm
   done
   ```

   Volumes referenced by absolute path inside `microvm.volumes` need
   their `image = ...` paths updated to the new location. Volumes
   referenced by bare filename (relative to `microvm.stateDir`) move
   transparently because the framework forces `microvm.stateDir =
   /var/lib/nixling/vms`.

8. **Activate.**

   ```bash
   nixos-rebuild switch --flake .#<host>     # commit, restart units
   ```

9. **Verify.**

   ```bash
   nixling list                       # what's declared + status
   nixling status <vm>                # per-VM health
   nixling up    <vm>                 # bring up (graphics: needs Wayland)
   nixling switch <vm>                # push a new closure live
   ```

   For headless VMs, `autostart = true` plus
   `systemctl status microvm@<vm>` will show the same unit state
   `microvm.nix` always reported. SSH into each migrated VM to
   confirm reachability.

### After every subsequent `nixos-rebuild switch` (v0.1.5+)

Every per-VM lifecycle service in the framework carries
`restartIfChanged = false`. Rebuilds update the unit files in
`/etc/systemd/system/` but do NOT cycle the running VMs — this
protects in-flight session state (interactive Wayland clients,
in-RAM Entra device-bound tokens, virtiofsd socket handshakes).

After `nixos-rebuild switch`, check whether any VM has pending
changes:

```bash
nixling list
```

A VM with a drift between its declared closure and its booted
closure is flagged in the STATUS column:

```
NAME             ENV    GRAPHICS TPM   USBIP   STATIC_IP       STATUS
work             work   true     true  true    10.20.0.10      systemd [pending restart]
```

Apply with:

```bash
nixling restart <vm>
```

(Or `nixling switch <vm>` if you want a per-VM closure rebuild +
live activation via SSH; restart cycles the existing closure
cleanly.)

`nixling status <vm>` prints both the `booted` and `current`
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
  (`mv /var/lib/nixling/vms/<vm> /var/lib/microvms/<vm>` for each
  VM), and `nixos-rebuild switch --flake .#<host>` against the old
  config. Start the VMs with `systemctl start microvm@<vm>` as
  before.
- **Step 9 verification fails on a specific VM** but activation
  succeeded: prefer fixing forward (the per-env net VM may take a
  few seconds to come up; check `nixling status sys-<env>-net` and
  the troubleshooting section). If a deeper rollback is needed,
  `nixos-rebuild switch --rollback` reverts to the previous
  generation, then move state back as above.

## What microvm.nix users gain

- **Per-env network isolation.** NAT-only egress, no inter-env
  routing, host-LAN drop rule applied by default.
- **Per-VM /nix/store.** Each guest sees only its own closure plus
  the microvm.nix runner — a closure-limited `/nix/store` view backed
  by a per-VM hardlink farm under `/var/lib/nixling/vms/<vm>/store/`.
  Zero byte duplication. `nixling switch <vm>` updates it live without
  a VM reboot. Back up `/var/lib/nixling/` only to encrypted,
  access-controlled media.
- **Explicit lifecycle.** `nixling@<vm>.service` wraps
  `microvm@<vm>.service` so "start", "stop", "restart", and
  "rotate ssh key" all become single commands with clear exit codes
  (`docs/reference/cli-contract.md`).
- **CLI ergonomics.** `nixling up/down/status/list/audio/usb` — no
  more remembering tap names, MAC byte counts, or which env's
  usbipd is bound to which `192.0.2.X`.
- **SSH key management.** Per-VM Ed25519 keys generated at activation,
  ACL'd to the `nixling-launcher` group, injected into the guest
  at boot via `nixling-load-host-keys.service`. No flake-baked keys.
- **Polkit allowlist.** Members of `nixling-launcher` can drive
  `start`/`stop`/`restart` on framework units from a Plasma launcher
  without sudo prompts.

## What microvm.nix users lose / what's nixling-only

- **Single-user assumption.** `nixling.site.waylandUser` is a single
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
  override per-VM via `nixling.vms.<vm>.config.microvm.hypervisor = ...`
  for headless VMs.
- **Framework-owned shares.** Do not add a `/nix/store` entry to
  `microvm.shares` in `nixling.vms.<vm>.config` — the framework
  injects it with `lib.mkForce`.
- **`microvm@<vm>.service` is wrapped, not replaced.** Use
  `nixling@<vm>` for lifecycle (it understands the per-VM store
  sync, polkit, and audio state). Use `microvm@<vm>` if you need to
  reach upstream behaviour directly.

## Naming conventions you'll see post-migration

- `br-<env>-lan` — workload LAN bridge for env `<env>`.
- `br-<env>-up` — point-to-point host↔net-VM bridge.
- `sys-<env>-net` — auto-declared net VM (NAT + dnsmasq + nftables).
- `vm-<vm>-<env>` / `vm-<vm>-up` — taps on the bridges above.
- `nixling-sys-<env>-usbipd-proxy.service` — host-side USBIP proxy
  per env.
- `nixling@<vm>.service` — lifecycle wrapper.
- `microvm@<vm>.service` — upstream unit (still exists).
- `nixling-launcher` — host group whose members can drive framework
  units without sudo.

## Backup / state directories

- Upstream defaults: `microvm.stateDir = /var/lib/microvms/<vm>/`.
- After migration: `microvm.stateDir = /var/lib/nixling/vms/`
  (forced in `nixos-modules/host.nix`). Per-VM state lives under
  `/var/lib/nixling/vms/<vm>/`.
- Per-VM `/nix/store` mirror: `/var/lib/nixling/vms/<vm>/store/`
  (hardlinks; same FS as `/nix/store` is required).
- SSH keys: `/var/lib/nixling/keys/<vm>_ed25519` (private) +
  `.pub`. Mode `0700`, ACL'd to `nixling-launcher`.
- TPM state (if `tpm.enable = true`):
  `/var/lib/nixling/vms/<vm>/swtpm/`. Treat as secret; back up only to
  encrypted, access-controlled media.

Back up `/var/lib/nixling/` only to encrypted, access-controlled media
(TPM NVRAM and per-VM SSH keys live there). Restoring requires the
same `nixling.site.keysDir` / `stateDir` layout — those are
advisory-only in v0.1.0 and effectively hardcoded.

## Troubleshooting

**Eval fails with `nixling.envs.<env>: lanSubnet overlaps …`.**
You picked a LAN subnet that overlaps `nixling.hostLanCidrs` or
another env's `lanSubnet`. Pick a disjoint `/24`.

**Eval fails with `graphics.enable = true` but `waylandUser = null`.**
Set `nixling.site.waylandUser = "<your-user>"` and declare that user
in `users.users`. The user must have a running Wayland session at the
time `nixling up <vm>` runs.

**`nixling up <vm>` fails: `cannot find $WAYLAND_DISPLAY`.**
You ran it over SSH or as root. Graphics VMs require a terminal
inside the host's Wayland session. Headless VMs work over SSH and
as root.

**Stale tap interface from the pre-migration setup.**
`ip link delete vm-<oldname>` and rerun `nixos-rebuild switch`. The
framework only manages the taps it declares.

**`nixling switch <vm>` errors with `cross-FS hardlink refused`.**
`/var/lib/nixling` and `/nix/store` are on different filesystems.
The per-VM store needs same-FS hardlinks; move
`/var/lib/nixling` to the same FS as `/nix/store` (typically by
remounting or relocating).

**Polkit prompt still appears on `nixling up`.**
The invoking user is not in `nixling-launcher`. Add them to
`nixling.site.launcherUsers` (which only adjoins the group; you must
still declare the user) and re-log-in so the group membership
takes effect.

**SSH into the VM still uses your old key.**
The guest's `authorized_keys` is populated at boot by
`nixling-load-host-keys.service`. Restart the VM
(`nixling down <vm> && nixling up <vm>`) or, inside the guest,
`systemctl restart nixling-load-host-keys.service`.

**`microvm.vms.<vm>` declared in two places.**
You left an old `microvm.vms.<name>` block alongside the new
`nixling.vms.<name>`. Remove the old one — the framework manages
the translation.

**Per-env net VM (`sys-<env>-net`) won't start.**
`systemctl status microvm@sys-<env>-net` first. The most common
cause is that the env's `lanSubnet` is not a `/24` ending in `.0`,
or `uplinkSubnet` is not a `/30`. Eval should have caught this; if
it didn't, file an issue.

## See also

- [Design / threat model](../explanation/design.md)
- [Per-component reference](../reference/)
- [Manifest schema](../reference/manifest-schema.md)
- [CLI contract](../reference/cli-contract.md)
- [Examples](../../examples/)
- [CHANGELOG](../../CHANGELOG.md)
