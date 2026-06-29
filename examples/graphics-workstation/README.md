# Example: graphics workstation VM

A full desktop-grade d2b consumer flake. One workload VM
(`corp-desktop`) with **graphics**, **audio**, and **YubiKey
USBIP** all enabled — i.e. every d2b component that touches a
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
| `flake.nix`         | Inputs (`nixpkgs`, `d2b`) + one `nixosConfigurations.demo`. |
| `configuration.nix` | Host setup: SDDM + Plasma 6, PipeWire, `alice`, one env, one VM. |

## What gets enabled

| `d2b.vms.corp-desktop.<opt>` | What it pulls in                                      |
|----------------------------------|-------------------------------------------------------|
| `graphics.enable = true`         | crosvm GPU sidecar                                    |
| `graphics.crossDomainTrusted = true` | Virtio-gpu cross-domain forwarding through the host-side filter proxy |
| `audio.enable = true`            | vhost-user-sound sidecar → host PipeWire client       |
| `usbip.yubikey = true`           | Per-env USBIP proxy + in-VM `vhci_hcd` + `usbip` CLI  |

The matching site-level requirements are declared in
`configuration.nix`:

```nix
d2b.site = {
  waylandUser   = "alice";        # required for graphics/audio
  launcherUsers = [ "alice" ];    # adds alice to the d2b lifecycle group
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
   `d2b-<vm>-gpu`) — receives the guest Wayland traffic and
   forwards it to the host-side filter socket at
   `/run/d2b-wlproxy/<vm>/wayland-0`. The GPU sidecar does NOT
   connect directly to the host compositor socket.
4. **`d2b-wayland-proxy`** (running as `d2b-<vm>-wlproxy`,
   listening on `/run/d2b-wlproxy/<vm>/wayland-0`) — mediates
   between the GPU sidecar and the real host compositor. It hides
   high-risk Wayland globals (screen capture, virtual input, etc.),
   rewrites guest app IDs to `d2b.<vm>.<original-app-id>` so the
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

- A graphics VM with `d2b.site.waylandUser = null` is a hard
  eval error — there's no host compositor to forward into.
- `autostart = true` on a graphics VM is rejected; the daemon cannot
  reach the user's Wayland session at boot. Always bring graphics VMs
  up interactively from a compositor terminal:
  `d2b vm start corp-desktop --apply`.
- Guest app IDs are prefixed with `d2b.corp-desktop.` by the
  filter proxy. If you use niri, you can opt into a generated
  window-rule include file via
  `d2b.site.ui.compositors.niri.enable = true`
  — see [`docs/how-to/niri-vm-borders.md`](../../docs/how-to/niri-vm-borders.md).
- This example explicitly sets `graphics.crossDomainTrusted = true` so it
  exercises the filtered cross-domain path. Do not use that setting for
  VMs that run privileged Docker/container workloads.

## The audio model

Audio is **mediated**, not raw-passthrough. The framework's
per-VM `d2b-<vm>-snd.service` sidecar runs
`vhost-device-sound` (from rust-vmm) on the host side and exposes
a virtio-sound device into the guest. On the host side the sidecar
talks to PipeWire as a regular client, so:

- The VM appears in `plasma-pa` / `wpctl status` as a PipeWire
  client named `d2b-corp-desktop`. You can mute, route, and
  EQ it per-stream like any other app.
- Both mic capture and speaker playback are **off by default**.
  The per-VM state file at
  `/var/lib/d2b/vms/corp-desktop/state/audio-state.json`
  records `{ "mic": false, "speaker": false }` on first
  materialisation. The virtio-sound device is only present in the
  guest while at least one of the two is granted.
- Grant interactively:

  ```bash
  d2b audio mic     on  corp-desktop   # grant microphone
  d2b audio speaker on  corp-desktop   # grant playback
  d2b audio status      corp-desktop   # show current state
  d2b audio mic     off corp-desktop   # revoke microphone
  d2b audio off         corp-desktop   # revoke both
  ```

  Grants persist across reboots in the same JSON state file. The
  CLI applies them live without needing to bounce the VM.

- If you want a VM to come up with audio already granted (e.g. a
  dedicated video-call VM), flip the eval-time defaults in
  `configuration.nix`:

  ```nix
  d2b.vms.corp-desktop.audio = {
    enable                = true;
    allowMicByDefault     = true;
    allowSpeakerByDefault = true;
  };
  ```

  Those defaults are consulted **only** the first time the audio
  state file is materialised; subsequent `d2b audio …` edits
  override them and persist.

## The YubiKey USBIP attach flow

`usbip.yubikey = true` on the VM plus `d2b.site.yubikey.enable
= true` on the host wires up:

- **Host side** (from `d2b.site.yubikey.enable`):
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
d2b usb probe
```

Then attach the declared device through the daemon:

```bash
d2b vm start corp-desktop --apply
d2b usb attach corp-desktop 1-2 --apply
```

Replace `1-2` with the busid you declared for your host. To release the claim,
run `d2b usb detach corp-desktop 1-2 --apply`. For degraded probe rows or
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
  - `/var/lib/d2b/keys/corp-desktop_ed25519{,.pub}` — the
    framework-managed Ed25519 SSH key (mode 0600 root:d2b).
  - `/var/lib/d2b/vms/corp-desktop/` — the per-VM workdir
    (microvm.nix-owned `var.img`, virtiofsd state, etc.).
  - `/var/lib/d2b/vms/corp-desktop/state/audio-state.json` —
    the live audio-grant state file (created on first
    materialisation; defaults to `{mic:false, speaker:false}`).
  - `/var/lib/d2b/vms/corp-desktop/store/` — the per-VM
    `/nix/store` hardlink farm so the guest sees only its own
    closure.
  - `/var/lib/d2b/vms/sys-desktop-net/` — the auto-declared
    headless net VM that NATs the env's LAN.

- **Bridges:**
  - `br-desktop-up`  — point-to-point /30 between host (`.1`)
    and the net VM (`.2`).
  - `br-desktop-lan` — /24 the net VM (`.1`) and workload VMs
    (`.10` and up) share. The host has **no** interface on this
    bridge — workload VMs cannot reach the host's neighbours.

- **Root-visible services:** `d2bd.service`,
  `d2b-priv-broker.socket`, and `d2b-priv-broker.service`.
  Per-VM runners (cloud-hypervisor, virtiofsd, swtpm, GPU/audio/USBIP
  sidecars) are broker-spawned and supervised by `d2bd`, not by
  per-VM systemd templates.

- **Lifecycle access:** `alice` is a member of the `d2b` group via
  `launcherUsers`, so the daemon authorizes lifecycle requests through
  its public socket peer credentials.

- **CLI on `$PATH`:** `d2b`, the Rust CLI for daily VM operations
  (see `d2b --help`).

## Bringing the VM up

Once the host activation completes:

```bash
d2b list                              # expect 'corp-desktop' + 'sys-desktop-net'
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# corp-desktop       desktop   true      false true    10.42.0.10      stopped
# sys-desktop-net    desktop   false     false false   192.0.2.2       running (net-vm)

d2b status                            # adds a "Bridge health" block
# NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS
# corp-desktop       desktop   true      false true    10.42.0.10      stopped
# sys-desktop-net    desktop   false     false false   192.0.2.2       running (net-vm)
#
# === Bridge health ===
# BRIDGE               STATE      ADMIN   EXPECTED     RESULT
# br-desktop-up        UP         up      UP           ok
# br-desktop-lan       NO-CARRIER up      NO-CARRIER   no-carrier (no workloads up)

# `corp-desktop` is GRAPHICS=true, so start it from an active
# Plasma/Wayland session. After the daemon starts its runners, STATUS
# transitions to `running`.

d2b vm start corp-desktop --apply      # interactive boot from a Plasma terminal
ssh -i /var/lib/d2b/keys/corp-desktop_ed25519 alice@10.42.0.10 hostname
d2b audio mic on corp-desktop         # grant microphone
d2b audio speaker on corp-desktop     # grant speakers
d2b usb attach corp-desktop 1-2 --apply # attach declared YubiKey busid
d2b vm stop corp-desktop --apply       # clean shutdown
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
Plasma + PipeWire + d2b closure and takes minutes.

## Customising

- Want a second isolated env (e.g. one for work, one for
  personal)? Add a second `d2b.envs.<name>` block with
  non-overlapping subnets, and a second `d2b.vms.<name>` with
  `env = "<that-env>"`. The framework materialises bridges, the
  net VM, the USBIP proxy, and lifecycle group access in lockstep.
- Want this VM to **not** forward Wayland (e.g. a headless
  background-service VM)? Drop `graphics.enable` and
  `audio.enable` — at that point you don't need
  `d2b.site.waylandUser` either. See `examples/minimal/`.
- Want a different Wayland compositor on the host? Swap
  `services.desktopManager.plasma6.enable = true` for your
  compositor of choice; the host-side Wayland proxy connects to
  whatever socket the user named by `d2b.site.waylandUser` owns.

## Common gotchas

- **`d2b vm start corp-desktop --apply` must run from a Plasma/Wayland
  terminal on the host** — not over SSH.
  The launcher reads the operator's Wayland environment to wire
  the crosvm GPU sidecar; over SSH there is no `wayland-0` socket
  to reach. (Headless VMs are unaffected.)
- **`d2b.site.waylandUser` must own an active session** when
  the VM boots, not just be declared. A fresh boot with no Plasma
  login leaves the GPU sidecar idle; `d2b vm start` will block on
  the Wayland socket.
- **YubiKey USBIP is exclusive per declared busid.** A durable broker claim
  names the owning VM. If `d2b usb probe` reports a degraded or
  other-owner row, follow the command it prints or the USBIP troubleshooting
  runbook rather than editing locks or sysfs driver links directly.
- **Host route/CIDR diagnostics are fail-closed.** A stale `ip route`
  or a CIDR overlap between this env and your host LAN refuses the
  affected lifecycle operation with a precise error naming the env.

## After subsequent rebuilds

`nixos-rebuild switch` updates the declared d2b bundle and may
restart `d2bd`, but daemon restarts are continuation events:
running VM runners are re-adopted rather than cycled. After rebuilding,
`d2b list` flags any VM whose declared closure has drifted from the
running one as `[pending restart]`; apply with `d2b vm restart
<vm> --apply`. See
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
> into this directory uses `d2b.url = "path:../..";` so the
> example can be evaluated against the in-tree framework without a
> network. When you copy this layout into your own repo, swap it
> for a real flake ref (`github:vicondoa/d2b/v0.1.0` or a
> pinned revision).
