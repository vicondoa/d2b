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
| `graphics.enable = true`         | crosvm GPU sidecar                                    |
| `graphics.crossDomainTrusted = true` | Virtio-gpu cross-domain forwarding through the host-side filter proxy |
| `audio.enable = true`            | vhost-user-sound sidecar → host PipeWire client       |
| `usbip.yubikey = true`           | Per-env USBIP proxy + in-VM `vhci_hcd` + `usbip` CLI  |

The matching site-level requirements are declared in
`configuration.nix`:

```nix
nixling.site = {
  waylandUser   = "alice";        # required for graphics/audio
  launcherUsers = [ "alice" ];    # polkit grant for `nixling vm start`
  yubikey.enable = true;          # host udev; usbip-host loads on per-VM opt-in
};
```

## The Wayland-forwarding model

Graphics VMs do **not** ship pixels back to the host as a video
stream. Instead, the guest's Wayland clients connect through a
chain that spans the VM boundary:

1. **Guest** — the guest runs `wl-cross-domain-proxy` (a lightweight
   virtio-gpu cross-domain bridge). Guest apps see a normal Wayland
   socket (`WAYLAND_DISPLAY=wayland-1`).
2. **virtio-gpu cross-domain channel** — the cross-domain transport
   carries Wayland protocol messages and surface allocations from the
   guest through the KVM boundary to the crosvm GPU sidecar on the
   host.
3. **crosvm GPU sidecar** (`crosvm device gpu`, running as
   `nixling-<vm>-gpu`) — receives the guest Wayland traffic and
   forwards it to the host-side filter socket at
   `/run/nixling-wlproxy/<vm>/wayland-0`. The GPU sidecar does NOT
   connect directly to the host compositor socket.
4. **`nixling-wayland-filter`** (running as `nixling-<vm>-wlproxy`,
   listening on `/run/nixling-wlproxy/<vm>/wayland-0`) — mediates
   between the GPU sidecar and the real host compositor. It hides
   high-risk Wayland globals (screen capture, virtual input, etc.),
   rewrites guest app IDs to `nixling.<vm>.<original-app-id>` so the
   host compositor can identify VM windows, and prefixes window titles
   with `[<vm>] ` for non-niri compositors. The `wlproxy` role is the
   **only** VM-specific process that holds the real host compositor socket.
5. **Host compositor** — receives forwarded surfaces and owns focus,
   decorations, multi-monitor placement, and HiDPI.

This design means the GPU sidecar is never directly trusted with the
host compositor socket. A compromised GPU command buffer can at most
reach the filter socket; the filter proxy enforces the host-compositor
trust boundary.

Practical implications:

- A graphics VM with `nixling.site.waylandUser = null` is a hard
  eval error — there's no host compositor to forward into.
- `autostart = true` on a graphics VM is rejected; the daemon cannot
  reach the user's Wayland session at boot. Always bring graphics VMs
  up interactively from a compositor terminal:
  `nixling vm start corp-desktop --apply`.
- Guest app IDs are prefixed with `nixling.corp-desktop.` by the
  filter proxy. If you use niri, you can opt into a generated
  window-rule include file via
  `nixling.site.ui.compositors.niri.enable = true`
  — see [`docs/how-to/niri-vm-borders.md`](../../docs/how-to/niri-vm-borders.md).
- This example explicitly sets `graphics.crossDomainTrusted = true` so it
  exercises the filtered cross-domain path. Do not use that setting for
  VMs that run privileged Docker/container workloads.

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
  - The `usbip-host` kernel module is loaded once an enabled VM opts
    into `usbip.yubikey = true`.
  - A broker-spawned per-env USBIP backend and proxy under the
    daemon-owned lifecycle DAG. The proxy binds the host's uplink IP
    (here `192.0.2.1`, the host side of the env's `/30`); there is no
    host-wide USBIP singleton.

- **Guest side** (from `usbip.yubikey = true`):
  - The `vhci_hcd` kernel module is loaded so the guest has a
    virtual USB host controller.
  - The `usbip` userspace CLI is installed for authenticated guestd
    import/detach operations.

Declare the approved busid in the copied example and use the read-only probe to
confirm host observation:

```bash
nixling usb probe
```

Then attach the declared device through the daemon:

```bash
nixling vm start corp-desktop --apply
nixling usb attach corp-desktop 1-2 --apply
```

Replace `1-2` with the busid you declared for your host. To release the claim,
run `nixling usb detach corp-desktop 1-2 --apply`. For degraded probe rows or
restart recovery, use the targeted runbook:
[`docs/how-to/troubleshoot-usbip.md`](../../docs/how-to/troubleshoot-usbip.md).

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
    framework-managed Ed25519 SSH key (mode 0600 root:nixling).
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
  - broker-spawned `usbipd-backend` / `usbipd-proxy` runners under
    `nixling.slice/sys-desktop/` — per-env USBIP backend (loopback)
    + uplink-IP-bound proxy.
  - `nixling-net-route-preflight.service` — fail-closed env-route
    sanity check ordered after `network-online.target`.

- **Polkit grants:** `alice` (member of `nixling`) can
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
# `interactive` (not `systemd`) after the next `nixling vm start
# corp-desktop` — graphics VMs cannot run as systemd units because
# there is no Wayland compositor in the system unit's PID 1.

nixling vm start corp-desktop --apply      # interactive boot from a Plasma terminal
ssh -i /var/lib/nixling/keys/corp-desktop_ed25519 alice@10.42.0.10 hostname
nixling audio mic on corp-desktop         # grant microphone
nixling audio speaker on corp-desktop     # grant speakers
nixling usb attach corp-desktop 1-2 --apply # attach declared YubiKey busid
nixling vm stop corp-desktop --apply       # clean shutdown
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
  compositor of choice; the host-side Wayland filter proxy connects to
  whatever socket the user named by `nixling.site.waylandUser` owns.

## Common gotchas

- **`nixling vm start corp-desktop --apply` must run from a Plasma/Wayland
  terminal on the host** — not over SSH, not as a systemd unit.
  The launcher reads the operator's Wayland environment to wire
  the crosvm GPU sidecar; over SSH there is no `wayland-0` socket
  to reach. (Headless VMs are unaffected.)
- **`nixling.site.waylandUser` must own an active session** when
  the VM boots, not just be declared. A fresh boot with no Plasma
  login leaves the GPU sidecar idle; `nixling vm start` will block on
  the Wayland socket.
- **YubiKey USBIP is exclusive per declared busid.** A durable broker claim
  names the owning VM. If `nixling usb probe` reports a degraded or
  other-owner row, follow the command it prints or the USBIP troubleshooting
  runbook rather than editing locks or sysfs driver links directly.
- **The fail-closed `nixling-net-route-preflight.service` runs
  before any nixling VM starts.** A stale `ip route` or a
  CIDR-overlap between this env and your host LAN refuses VM
  start with a precise error naming the env.

## After subsequent rebuilds

Every per-VM lifecycle service in the framework carries
`restartIfChanged = false`, so a `nixos-rebuild switch` updates
unit files but does NOT cycle running VMs. After rebuilding,
`nixling list` flags any VM whose declared closure has drifted
from the running one as `[pending restart]`; apply with
`nixling vm restart <vm> --apply`. See
[`templates/default/README.md` — After every subsequent rebuild](../../templates/default/README.md#after-every-subsequent-rebuild)
for the recommended workflow and
[`docs/reference/cli-contract.md`](../../docs/reference/cli-contract.md#pending-restart-signal-v015)
for the exact predicate.

## See also

- [`examples/minimal`](../minimal/) — read-and-copy headless starter
- [`examples/multi-env`](../multi-env/) — two isolated envs (work + personal)
- [`examples/with-entra-id`](../with-entra-id/) — Entra-ID composition via the sibling flake
- [`templates/default`](../../templates/default/) — scaffold via `nix flake init`
- [`docs/how-to/troubleshoot-usbip.md`](../../docs/how-to/troubleshoot-usbip.md) — USBIP/YubiKey recovery runbook

> **Note on the in-tree path** — the version of `flake.nix` checked
> into this directory uses `nixling.url = "path:../..";` so the
> example can be evaluated against the in-tree framework without a
> network. When you copy this layout into your own repo, swap it
> for a real flake ref (`github:vicondoa/nixling/v0.1.0` or a
> pinned revision).
