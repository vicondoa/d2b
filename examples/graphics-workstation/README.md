# Example: graphics workstation VM

A full desktop-grade nixling consumer flake. One workload VM
(`corp-desktop`) with **graphics**, **audio**, and **YubiKey
USBIP** all enabled — i.e. every nixling component that touches a
host-side sidecar.

This is the answer to "I want a Wayland desktop inside a microVM
that can play sound and authenticate with my YubiKey." If you want
a smaller starting point with no sidecars, see
[`examples/minimal/`](../minimal) instead.

Placeholders used throughout: `alice` for the host's Wayland user,
`corp-desktop` for the VM, `desktop` for the env. Swap them for
whatever you actually run.

## Files

| File                | Purpose                                           |
|---------------------|---------------------------------------------------|
| `flake.nix`         | Inputs (`nixpkgs`, `nixling`) + one `nixosConfigurations.demo`. |
| `configuration.nix` | Host setup: SDDM + Plasma 6, PipeWire, `alice`, one env, one VM. |

## What gets enabled

| `nixling.vms.corp-desktop.<opt>` | What it pulls in                                      |
|----------------------------------|-------------------------------------------------------|
| `graphics.enable = true`         | crosvm GPU sidecar, Wayland cross-domain forwarding   |
| `audio.enable = true`            | vhost-user-sound sidecar → host PipeWire client       |
| `usbip.yubikey = true`           | Per-env USBIP proxy + in-VM `vhci_hcd` + `usbip` CLI  |

The matching site-level requirements are declared in
`configuration.nix`:

```nix
nixling.site = {
  waylandUser   = "alice";        # required for graphics/audio
  launcherUsers = [ "alice" ];    # polkit grant for `nixling up`
  yubikey.enable = true;          # host udev + usbip-host module
};
```

## The Wayland-forwarding model

Graphics VMs do **not** ship pixels back to the host as a video
stream. Instead, the guest's Wayland clients connect through a
crosvm-side GPU device that speaks the **virtio-gpu cross-domain
context** protocol to the host. The host runs a per-VM
`nixling-<vm>-gpu.service` sidecar (a hardened, isolated systemd
unit running as a dedicated user) that:

1. Speaks `crosvm device gpu` on the VM side over vhost-user-gpu.
2. Translates Wayland surface allocations from the guest into real
   surfaces on the host's compositor by opening
   `/run/user/<uid>/wayland-0` — the socket of the user named by
   `nixling.site.waylandUser`.
3. Lets the host compositor (Plasma here, but sway / Hyprland /
   GNOME work identically) own focus, decorations, multi-monitor
   placement, and HiDPI — the guest sees a single Wayland output
   pre-mapped per host monitor.

Practical implications:

- A graphics VM with `nixling.site.waylandUser = null` is a hard
  eval error — there's no host compositor to forward into.
- `autostart = true` on a graphics VM is rejected; the systemd
  unit cannot reach the user's Wayland session at boot. Always
  bring graphics VMs up interactively from a Plasma (or sway,
  etc.) terminal: `nixling up corp-desktop`.
- The sidecar is hardened with `MemoryDenyWriteExecute = false`
  (the crosvm GPU command-buffer JIT needs `PROT_WRITE|PROT_EXEC`
  — see Spec correction #19) and
  `RestrictAddressFamilies = [ AF_UNIX AF_NETLINK AF_VSOCK ]`
  (cloud-hypervisor uses vsock for `sd_notify` — see Spec
  correction #20).

## The audio model

Audio is **mediated**, not raw-passthrough. The framework's
per-VM `nixling-<vm>-snd.service` sidecar runs
`vhost-device-sound` (from rust-vmm) on the host side and exposes
a virtio-sound device into the guest. On the host side the sidecar
talks to PipeWire as a regular client, so:

- The VM appears in `plasma-pa` / `wpctl status` as a PipeWire
  client named `nixling-corp-desktop`. You can mute, route, and
  EQ it per-stream like any other app.
- Both mic capture and speaker playback are **off by default**.
  The per-VM state file at
  `/var/lib/nixling/vms/corp-desktop/state/audio-state.json`
  records `{ "mic": false, "speaker": false }` on first
  materialisation. The virtio-sound device is only present in the
  guest while at least one of the two is granted.
- Grant interactively:

  ```bash
  nixling audio mic     on  corp-desktop   # grant microphone
  nixling audio speaker on  corp-desktop   # grant playback
  nixling audio status      corp-desktop   # show current state
  nixling audio mic     off corp-desktop   # revoke microphone
  nixling audio off         corp-desktop   # revoke both
  ```

  Grants persist across reboots in the same JSON state file. The
  CLI applies them live without needing to bounce the VM.

- If you want a VM to come up with audio already granted (e.g. a
  dedicated video-call VM), flip the eval-time defaults in
  `configuration.nix`:

  ```nix
  nixling.vms.corp-desktop.audio = {
    enable                = true;
    allowMicByDefault     = true;
    allowSpeakerByDefault = true;
  };
  ```

  Those defaults are consulted **only** the first time the audio
  state file is materialised; subsequent `nixling audio …` edits
  override them and persist.

## The YubiKey USBIP attach flow

`usbip.yubikey = true` on the VM plus `nixling.site.yubikey.enable
= true` on the host wires up:

- **Host side** (from `nixling.site.yubikey.enable`):
  - udev rules for Yubico vendor ID `1050` so the hidraw / raw-USB
    nodes carry `GROUP="kvm" MODE="0660" uaccess`.
  - The `usbip-host` kernel module is loaded.
  - A per-env USBIP proxy: `nixling-sys-desktop-usbipd-proxy.service`
    bound to the host's uplink IP (here `192.0.2.1`, the host's
    side of the env's `/30`). Per-env loopback isolation — there's
    no host-wide singleton — see Spec correction #4 for the
    rationale.

- **Guest side** (from `usbip.yubikey = true`):
  - The `vhci_hcd` kernel module is loaded so the guest has a
    virtual USB host controller.
  - The `usbip` userspace CLI is installed.

To attach a plugged-in YubiKey to `corp-desktop`:

```bash
nixling usb corp-desktop
```

That command:

1. Acquires `/run/nixling/usbipd.lock` so only one VM at a time
   owns the device.
2. Detaches the YubiKey from any other env's proxy (the lock
   guarantees no cross-env race).
3. Binds the device to the destination env's usbipd proxy.
4. Inside the VM, runs `usbip attach -r <host-uplink-ip> -b <busid>`
   so the guest's `vhci_hcd` adopts the device.

Press `Ctrl-C` to detach cleanly; the lock is released and the
device falls back to the host's xhci driver. PIN, touch, and
challenge-response work transparently inside the VM after attach
— it is a real USB endpoint, not an emulated one.

## Why this example is `x86_64-linux`-only

The flake hard-pins `system = "x86_64-linux"` in
`nixosConfigurations.demo`. Reason: `graphics.enable = true` and
`audio.enable = true` transitively depend on three x86_64-only
packages:

- `pkgs/spectrum-ch` — patched cloud-hypervisor build.
- `pkgs/crosvm-patched` — the GPU sidecar binary.
- `pkgs/vhost-device-sound` — the audio sidecar binary.

`nixos-modules/host.nix`'s `checkVmPlatform` gate throws an
eval-time error with a clear message if any VM with
`graphics.enable` or `audio.enable` is evaluated against a
non-x86_64-linux host, so a misconfigured aarch64 cross-eval
fails fast instead of producing a confusing downstream
"package not available on platform" error inside the component
let-bindings.

Headless VMs (no graphics, no audio, no usbip-related host
sidecars beyond what aarch64 supports) are arch-agnostic and
evaluate cleanly on `aarch64-linux` — see
[`examples/minimal/`](../minimal) for that path.

## What materialises after `nixos-rebuild switch`

A single `nixos-rebuild switch --flake .#demo` (followed
implicitly by the host activation script) produces:

- **State directories:**
  - `/var/lib/nixling/keys/corp-desktop_ed25519{,.pub}` — the
    framework-managed Ed25519 SSH key (mode 0600 root:nixling-launcher).
  - `/var/lib/nixling/vms/corp-desktop/` — the per-VM workdir
    (microvm.nix-owned `var.img`, virtiofsd state, etc.).
  - `/var/lib/nixling/vms/corp-desktop/state/audio-state.json` —
    the live audio-grant state file (created on first
    materialisation; defaults to `{mic:false, speaker:false}`).
  - `/var/lib/nixling/vms/corp-desktop/store/` — the per-VM
    `/nix/store` hardlink farm so the guest sees only its own
    closure.
  - `/var/lib/nixling/vms/sys-desktop-net/` — the auto-declared
    headless net VM that NATs the env's LAN.

- **Bridges:**
  - `br-desktop-up`  — point-to-point /30 between host (`.1`)
    and the net VM (`.2`).
  - `br-desktop-lan` — /24 the net VM (`.1`) and workload VMs
    (`.10` and up) share. The host has **no** interface on this
    bridge — workload VMs cannot reach the host's neighbours.

- **Systemd units (sample, not exhaustive):**
  - `microvm@corp-desktop.service` (the VM itself, owned by
    microvm.nix).
  - `microvm@sys-desktop-net.service` (the auto-declared net
    VM, `autostart = true`).
  - `nixling-corp-desktop-gpu.service`    — GPU sidecar.
  - `nixling-corp-desktop-snd.service`    — audio sidecar.
  - `nixling-sys-desktop-usbipd-backend.service`
    + `nixling-sys-desktop-usbipd-proxy.service` — per-env USBIP
    backend (loopback) + uplink-IP-bound proxy.
  - `nixling-net-route-preflight.service` — fail-closed env-route
    sanity check ordered after `network-online.target`.

- **Polkit grants:** `alice` (member of `nixling-launcher`) can
  start / stop / restart any of the framework's own units without
  a password prompt.

- **CLI on `$PATH`:** `nixling` (a `writeShellApplication`-shaped
  Bash CLI for daily VM ops — see `nixling --help`).

## Bringing the VM up

Once the host activation completes:

```bash
nixling list                              # expect 'corp-desktop' + 'sys-desktop-net'
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# corp-desktop       desktop   true      false true    10.42.0.10      stopped
# sys-desktop-net    desktop   false     false false   192.0.2.2       systemd (net-vm)

nixling status                            # adds a "Bridge health" block
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# corp-desktop       desktop   true      false true    10.42.0.10      stopped
# sys-desktop-net    desktop   false     false false   192.0.2.2       systemd (net-vm)
#
# === Bridge health ===
# BRIDGE               STATE      ADMIN   EXPECTED     RESULT
# br-desktop-up        UP         up      UP           ok
# br-desktop-lan       NO-CARRIER up      NO-CARRIER   no-carrier (no workloads up)

# `corp-desktop` is GRAPHICS=true, so its STATUS will go to
# `interactive` (not `systemd`) after the next `nixling up
# corp-desktop` — graphics VMs cannot run as systemd units because
# there is no Wayland compositor in the system unit's PID 1.

nixling up corp-desktop                   # interactive boot from a Plasma terminal
ssh -i /var/lib/nixling/keys/corp-desktop_ed25519 alice@10.42.0.10 hostname
nixling audio mic on corp-desktop         # grant microphone
nixling audio speaker on corp-desktop     # grant speakers
nixling usb corp-desktop                  # attach YubiKey (Ctrl-C to detach)
nixling down corp-desktop                 # clean shutdown
```

## Verifying the example before deploying

The two evaluation checks the upstream test gate runs against this
example are:

```bash
cd examples/graphics-workstation
nix flake check --no-build --all-systems
nix eval .#nixosConfigurations.demo.config.system.build.toplevel.drvPath
```

Both should return a derivation path without errors. They do not
build anything — building `system.build.toplevel` pulls in the full
Plasma + PipeWire + nixling closure and takes minutes.

## Customising

- Want a second isolated env (e.g. one for work, one for
  personal)? Add a second `nixling.envs.<name>` block with
  non-overlapping subnets, and a second `nixling.vms.<name>` with
  `env = "<that-env>"`. The framework materialises bridges, the
  net VM, the USBIP proxy, and polkit grants per-env in lockstep.
- Want this VM to **not** forward Wayland (e.g. a headless
  background-service VM)? Drop `graphics.enable` and
  `audio.enable` — at that point you don't need
  `nixling.site.waylandUser` either. See `examples/minimal/`.
- Want a different Wayland compositor on the host? Swap
  `services.desktopManager.plasma6.enable = true` for your
  compositor of choice; the GPU sidecar talks to whatever owns
  the `wayland-0` socket of the user named by
  `nixling.site.waylandUser`.

## Common gotchas

- **`nixling up corp-desktop` must run from a Plasma/Wayland
  terminal on the host** — not over SSH, not as a systemd unit.
  The launcher reads the operator's Wayland environment to wire
  the crosvm GPU sidecar; over SSH there is no `wayland-0` socket
  to reach. (Headless VMs are unaffected.)
- **`nixling.site.waylandUser` must own an active session** when
  the VM boots, not just be declared. A fresh boot with no Plasma
  login leaves the GPU sidecar idle; `nixling up` will block on
  the Wayland socket.
- **YubiKey USBIP is exclusive across envs.** Only one env's
  USBIP backend can hold a `usbip bind` at a time — the CLI
  detaches other env backends before binding. If `nixling usb`
  hangs, check that no other env's backend has the device.
- **The fail-closed `nixling-net-route-preflight.service` runs
  before any nixling VM starts.** A stale `ip route` or a
  CIDR-overlap between this env and your host LAN refuses VM
  start with a precise error naming the env.

## See also

- [`examples/minimal`](../minimal/) — read-and-copy headless starter
- [`examples/multi-env`](../multi-env/) — two isolated envs (work + personal)
- [`examples/with-entra-id`](../with-entra-id/) — Entra-ID composition via the sibling flake
- [`templates/default`](../../templates/default/) — scaffold via `nix flake init`

> **Note on the in-tree path** — the version of `flake.nix` checked
> into this directory uses `nixling.url = "path:../..";` so the
> example can be evaluated against the in-tree framework without a
> network. When you copy this layout into your own repo, swap it
> for a real flake ref (`github:vicondoa/nixling/v0.1.0` or a
> pinned revision).
