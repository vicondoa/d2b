# `nixling.vms.<vm>.graphics.*`

> Reference for the `graphics` component module.
> Source: [`nixos-modules/components/graphics.nix`](../../nixos-modules/components/graphics.nix)
> Host-side wiring: [`nixos-modules/host-sidecars.nix`](../../nixos-modules/host-sidecars.nix), [`nixos-modules/host-users.nix`](../../nixos-modules/host-users.nix)

## What this component does

Exposes a virtio-gpu device to the guest and forwards Wayland clients
running inside the VM to the host compositor over the virtio-gpu
cross-domain channel. The guest sees a normal Wayland session
(`WAYLAND_DISPLAY=wayland-1`, `GDK_BACKEND=wayland`, etc.); a
`wayland-proxy-virtwl` user service inside the guest relays surfaces
to the host's `wayland-0` socket via virtio-gpu. The hypervisor is
forced to `cloud-hypervisor` (vendored spectrum-os build) and
microvm.nix's runner spawns `crosvm device gpu` as the vhost-user-gpu
sidecar. The whole sidecar pipeline (CH + crosvm-gpu) runs on the
host as the dedicated per-VM `nixling-<vm>-gpu` system user, not as
the operator's Wayland user.

## Options (host-side)

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.vms.<vm>.graphics.enable` | bool | `false` | Enable virtio-gpu + Wayland cross-domain forward. Implies `hypervisor = cloud-hypervisor`. |
| `nixling.vms.<vm>.graphics.crossDomainTrusted` | bool | `false` | Allow the `cross-domain` context type in the crosvm GPU sidecar. Set true only for VMs whose primary purpose is Wayland forwarding (e.g. a FreeRDP launchpad). Must be false for VMs running Docker — a privileged-container escape could attack the host compositor via cross-domain. |

Site-level dependency:

| Option | Type | Required when | Description |
|---|---|---|---|
| `nixling.site.waylandUser` | nullable str | any VM has `graphics.enable = true` | Username of the host's primary Wayland session. The GPU sidecar binds this user's `/run/user/<uid>/wayland-0` into its private mount namespace. Eval fails with a clear message if unset. |

## Options (guest-side propagation)

`host.nix` propagates the host-side trust flag into the guest config
under `mkIf vm'.graphics.enable`:

```nix
(lib.mkIf vm'.graphics.enable {
  nixling.graphics.crossDomainTrusted = vm'.graphics.crossDomainTrusted;
})
```

The matching guest-visible option lives in the imported
`components/graphics.nix`:

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.graphics.crossDomainTrusted` | bool | `false` | Resolved guest-side mirror of the per-VM flag. When false, a shell shim wraps `crosvm` and strips `cross-domain` from the `--params` JSON before invoking the real binary. |

## Host-side resources created

- **`nixling-<vm>-gpu` system user + group** (declared in
  [`host-users.nix`](../../nixos-modules/host-users.nix)).
  `SupplementaryGroups = [ "kvm" ]`. Created once per graphics VM.
- **`nixling-<vm>-gpu.service`** (in
  [`host-sidecars.nix`](../../nixos-modules/host-sidecars.nix)) — runs
  the entire `microvm-run` pipeline (cloud-hypervisor + crosvm-gpu
  sidecar) as the `nixling-<vm>-gpu` user. `wants`/`after`
  `nixling-<vm>-swtpm.service` and `nixling-<vm>-snd.service` (each
  only present if the respective component is enabled).
  - `ExecStartPre`: `setfacl -m u:nixling-<vm>-gpu:rw /run/user/<wayland-uid>/wayland-0`.
  - `BindPaths`: `/run/user/<wayland-uid>/wayland-0:/run/nixling-gpu/<vm>/wayland-0` — only the socket is visible inside the sidecar's mount namespace, not the parent directory.
  - `ExecStopPost`: `setfacl -x u:nixling-<vm>-gpu /run/user/<wayland-uid>/wayland-0`.
  - `Restart = "no"` — graphics VMs are launched interactively from a Plasma terminal via `nixling up`; restart-on-failure would re-attempt a doomed start.
- **/dev/kvm + /dev/dri/renderD128 device allow** via
  `DevicePolicy = "closed"` + explicit `DeviceAllow` (no
  `PrivateDevices`).
- **fontconfig defaults** — `dejavu_fonts`, `liberation_ttf`,
  `noto-fonts` are added to `fonts.packages` so the guest's monospace
  alias resolves to DejaVu Sans Mono and `foot` doesn't warn.
- **Eval-time assertion** that `passthru.testedWithCrosvmRev` on the
  vendored spectrum-ch package matches `pkgs.crosvm.src.rev`. If
  nixpkgs bumps crosvm independently of the vendored CH, the system
  refuses to evaluate until the pair has been re-tested.
- **Vendored cloud-hypervisor** at `pkgs/spectrum-ch/` carrying the
  spectrum-os patch set. `microvm.cloud-hypervisor.package` is pinned
  to it.
- **Patched crosvm + seccomp policies** at `pkgs/patches/` and a
  `runCommand` that pre-compiles every `.policy` to `.bpf` from
  google/crosvm @ 299c1e7 (adds `MADV_GUARD_*` to the `madvise`
  allowlist). The `.bpf` files live alongside the crosvm binary under
  a `symlinkJoin`; the C parser fallback is never used.
- **Patched `wayland-proxy-virtwl`** (`patches/wayland-proxy-virtwl-multimon.patch`)
  that forwards every host `wl_output` global, not just the first.

## Guest-side resources created

- `hardware.graphics.enable = true`.
- `microvm.graphics.enable = true`; `microvm.kernelParams += [ "nofb" "video=off" ]`
  so fbcon does not bind to virtio-gpu and never issues `SET_SCANOUT`
  (suppresses the chromeless host-side "crosvm" scanout window for
  non-cross-domain VMs).
- `microvm.graphics.crosvmPackage` = either `crosvmPatched`
  (cross-domain trusted) or a shell shim around `crosvmPatched` that
  strips `cross-domain` from `--params`.
- `systemd.user.services.wayland-proxy` — runs
  `wayland-proxy-virtwl --virtio-gpu --tag=[<hostname>]\\ --x-display=0
  --xwayland-binary=<xwayland>`. `--tag` prefixes guest window titles
  with the VM name in square brackets for at-a-glance host-side
  identification.
- `environment.sessionVariables` pinning `WAYLAND_DISPLAY`,
  `DISPLAY`, `QT_QPA_PLATFORM`, `GDK_BACKEND`, `XDG_SESSION_TYPE`,
  `SDL_VIDEODRIVER`, `CLUTTER_BACKEND`, `MOZ_ENABLE_WAYLAND`, plus
  Mesa probing knobs (`VK_DRIVER_FILES` pinned to virtio_icd + lvp,
  `MESA_LOADER_DRIVER_OVERRIDE=virtio_gpu`, `LIBGL_KOPPER_DISABLE`,
  `EGL_LOG_LEVEL=fatal`).

## Runtime invariants

- The CH + crosvm-gpu processes show up as `nixling-<vm>-gpu` in
  `ps -ef`; never as the operator's Wayland user.
- Only `wayland-0` is reachable from inside the sidecar's mount
  namespace — the parent `/run/user/<uid>/` is invisible.
- The guest cannot reach the host compositor outside of virtio-gpu
  cross-domain (no Wayland socket bind-mount into the guest).
- With `crossDomainTrusted = false`, every `--params` payload reaching
  `crosvm device gpu` has `cross-domain` stripped — verifiable via
  `ps -fC crosvm` on the host.
- `graphics.enable = true` is x86_64-linux only. `checkVmPlatform`
  in `host.nix` throws an eval-time error naming the VM if the host
  is not `x86_64-linux`.

## Hardening notes

`nixling-<vm>-gpu.service` sandboxing (in
[`host-sidecars.nix`](../../nixos-modules/host-sidecars.nix)):

- `NoNewPrivileges`, `ProtectSystem=strict`, `ProtectHome`,
  `PrivateTmp`, `ProtectKernelTunables/Modules`,
  `ProtectControlGroups`, `ProtectClock`, `ProtectHostname`,
  `ProtectProc=invisible`, `LockPersonality`,
  `RestrictNamespaces`, `SystemCallArchitectures=native`.
- `SystemCallFilter = [ "@system-service" "~@privileged" "~@resources" ]`.
- `RestrictAddressFamilies = [ "AF_UNIX" "AF_NETLINK" "AF_VSOCK" ]` —
  `AF_VSOCK` is required because cloud-hypervisor uses vsock for
  `sd_notify`, `AF_NETLINK` for the tap helper.
- `DevicePolicy = "closed"` + explicit `DeviceAllow` for `/dev/kvm`
  and `/dev/dri/renderD128`. `PrivateDevices` is intentionally NOT
  set (it would override the explicit allow list).
- **`MemoryDenyWriteExecute` is intentionally OMITTED.** crosvm's
  GPU command-buffer translation needs `PROT_WRITE+PROT_EXEC` JIT
  pages; MDWE would SIGSEGV the sidecar on first frame. This is the
  documented exception versus the audio sidecar's template.
- `ReadWritePaths` exposes only `/var/lib/nixling/vms/<vm>` and
  `/run/nixling-gpu/<vm>`.

The spectrum-ch CH build itself carries upstream spectrum-os
sandboxing; the crosvm device gpu seccomp `.bpf` files are present
in the closure but not yet loaded at runtime (the `crosvm device gpu`
subcommand exposes no `--seccomp-policy-dir` flag in the pinned
nixpkgs rev — defence-in-depth payload waiting on an upstream knob).

## Common gotchas / failure modes

- **Black screen / no guest window.** The host `wayland-0` socket
  must be reachable as the user named by `nixling.site.waylandUser`.
  `nixling up <vm>` must be invoked from a Plasma session terminal —
  never as root, never over SSH (`autostart = true` is also wrong
  for graphics VMs and triggers an assertion in the audio module if
  audio is enabled).
- **Chromeless "crosvm" window appearing on the host.** crosvm's
  Wayland display backend unconditionally creates an `xdg_toplevel`
  for every scanout surface; `DisplayParameters.hidden` is honored
  only on Windows. The mitigation is the `nofb` kernel parameter
  (suppresses fbcon-driven `SET_SCANOUT`) plus a KWin window rule on
  the host that hides any window with `title=^crosvm$`. If a stray
  window appears, verify both are in place.
- **CH↔crosvm rev drift.** The assertion comparing
  `spectrumCH.passthru.testedWithCrosvmRev` to `pkgs.crosvm.src.rev`
  trips after a nixpkgs bump that touches `crosvm` without a matching
  spectrum-ch re-test. Read the vhost-user-gpu wire-protocol notes
  in the module header, re-test, then bump `testedWithCrosvmRev`.
- **Sidecar permission denied on the wayland socket.** The
  `ExecStartPre` ACL grant only works when the wayland session is
  already running as `nixling.site.waylandUser` at start time. If a
  graphics VM fails on `setfacl: ... wayland-0: No such file or
  directory`, the operator's session isn't live.
- **Cross-domain forwarding silently disabled.** With
  `crossDomainTrusted = false` (the default) GUI apps still work via
  virgl2 + standard virtio-gpu, but advanced cross-domain features
  (Wayland-forwarding launchpad use case) won't.

## See also

- [Design / threat model](../explanation/design.md)
- [Manifest schema](./manifest-schema.md) — the per-VM `nixling-<vm>-gpu`
  unit is exposed under the manifest's `units.gpu` field.
- [CLI contract](./cli-contract.md) — `nixling up <vm>` /
  `nixling down <vm>` lifecycle.
- [`examples/graphics-workstation`](../../examples/graphics-workstation/) —
  end-to-end example with graphics + audio + USBIP YubiKey.
- [`examples/with-entra-id`](../../examples/with-entra-id/) — graphics
  VM composed with the sibling `nixos-entra-id` flake.
