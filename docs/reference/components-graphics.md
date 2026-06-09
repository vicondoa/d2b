# `nixling.vms.<vm>.graphics.*`

> Reference for the `graphics` component module.
> Source: [`nixos-modules/components/graphics.nix`](../../nixos-modules/components/graphics.nix)
> Host-side wiring: [`nixos-modules/processes-json.nix`](../../nixos-modules/processes-json.nix), [`nixos-modules/minijail-profiles.nix`](../../nixos-modules/minijail-profiles.nix), [`nixos-modules/host-users.nix`](../../nixos-modules/host-users.nix)

## What this component does

Exposes a virtio-gpu device to the guest and forwards Wayland clients
running inside the VM to the host compositor over the virtio-gpu
cross-domain channel. The guest sees a normal Wayland session
(`WAYLAND_DISPLAY=wayland-1`, `GDK_BACKEND=wayland`, etc.).

When `graphics.crossDomainTrusted = true` and
`graphics.waylandFilter.enable = true`, the guest-side
`wl-cross-domain-proxy` bridges the virtio-gpu cross-domain transport to
the guest socket, while the host-side `nixling-wayland-filter` runs as a
broker-spawned `wayland-proxy` role and mediates access to the real host
compositor. `nixlingd` supervises the daemon-owned process DAG and asks
`nixling-priv-broker` to spawn the wayland proxy, GPU sidecar
(`crosvm device gpu`), and cloud-hypervisor runner as pidfd-tracked
runners. The GPU sidecar runs as the dedicated per-VM
`nixling-<vm>-gpu` system user, not as the operator's Wayland user.

## Options (host-side)

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.vms.<vm>.graphics.enable` | bool | `false` | Enable virtio-gpu + Wayland cross-domain forward. Implies `hypervisor = cloud-hypervisor`. |
| `nixling.vms.<vm>.graphics.crossDomainTrusted` | bool | `false` | Allow the `cross-domain` context type in the crosvm GPU sidecar. Set true only for VMs whose primary purpose is Wayland forwarding (e.g. a FreeRDP launchpad). Must be false for VMs running Docker — a privileged-container escape could attack the host compositor via cross-domain. |
| `nixling.vms.<vm>.graphics.waylandFilter.enable` | bool | `true` | When cross-domain forwarding is trusted, insert the host-jailed `nixling-wayland-filter` between crosvm and the real host compositor. Disable only to use the legacy direct compositor socket path. |
| `nixling.vms.<vm>.graphics.waylandFilter.denyGlobals` | list of str | `[]` | Additional Wayland globals to hide from the guest. |
| `nixling.vms.<vm>.graphics.waylandFilter.allowGlobals` | list of str | `[]` | Globals to allow even if denied by the secure defaults. The proxy emits runtime advisory diagnostics for boundary-narrowing overrides. |
| `nixling.vms.<vm>.graphics.waylandFilter.maxVersions` | attrs of positive int | `{}` | Per-interface advertised version caps passed as `--max-version INTERFACE=VERSION`. |
| `nixling.vms.<vm>.graphics.virglVideo` | bool | `false` | Experimental Firefox/VA-API path: enables `VIRGL_RENDERER_USE_VIDEO` through crosvm/rutabaga. Default off because prior testing deadlocked the GPU command loop when video caps were advertised. |

Site-level dependency:

| Option | Type | Required when | Description |
|---|---|---|---|
| `nixling.site.waylandUser` | nullable str | any VM has `graphics.enable = true` | Username of the host's primary Wayland session. The GPU sidecar or the host-side Wayland filter needs this user's `/run/user/<uid>/<waylandDisplay>` socket. Eval fails with a clear message if unset. |

## Options (guest-side propagation)

`host.nix` propagates the host-side trust flag into the guest config
under `mkIf vm'.graphics.enable`:

```nix
(lib.mkIf vm'.graphics.enable {
  nixling.graphics.crossDomainTrusted = vm'.graphics.crossDomainTrusted;
  nixling.graphics.virglVideo = vm'.graphics.virglVideo;
})
```

The matching guest-visible option lives in the imported
`components/graphics.nix`:

| Option | Type | Default | Description |
|---|---|---|---|
| `nixling.graphics.crossDomainTrusted` | bool | `false` | Resolved guest-side mirror of the per-VM flag. When false, a shell shim wraps `crosvm` and strips `cross-domain` from the `--params` JSON before invoking the real binary. |
| `nixling.graphics.virglVideo` | bool | `false` | Resolved guest-side mirror of the per-VM flag. When true, the patched crosvm/rutabaga build passes `VIRGL_RENDERER_USE_VIDEO` to virglrenderer. |

## Host-side resources created

- **`nixling-<vm>-gpu` system user + group** (declared in
  [`host-users.nix`](../../nixos-modules/host-users.nix)). It is a
  per-VM runner principal and is separate from the host Wayland user.
- **Daemon process nodes** in `processes.json`: `wayland-proxy` when the
  filter is enabled for a cross-domain VM, `gpu` (or `gpu-render-node`),
  and `cloud-hypervisor-runner`. `nixlingd` supervises them through the
  broker `SpawnRunner` / pidfd path; no per-VM graphics systemd service
  is emitted.
- **`/run/nixling-wlproxy/<vm>/wayland-0`** for the filtered compositor
  socket that crosvm connects to when the host filter is active.
  `/run/nixling-gpu/<vm>/` remains the GPU role-local runtime directory.
- **Device allowlist** from the minijail profile. Normal GPU runners
  use the closed device set needed by cloud-hypervisor/crosvm; the
  render-node-only profile uses broker-prepared fd passing instead of
  broad host device access.
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
- **`wl-cross-domain-proxy`** packaged under `pkgs/` for the guest-side
  virtio-gpu cross-domain bridge.

## Guest-side resources created

- `hardware.graphics.enable = true`.
- `microvm.graphics.enable = true`; `microvm.kernelParams += [ "nofb" "video=off" ]`
  so fbcon does not bind to virtio-gpu and never issues `SET_SCANOUT`
  (suppresses the chromeless host-side "crosvm" scanout window for
  non-cross-domain VMs).
- `microvm.graphics.crosvmPackage` = either `crosvmPatched`
  (cross-domain trusted) or a shell shim around `crosvmPatched` that
  strips `cross-domain` from `--params`.
- `systemd.user.services.wayland-proxy` — when
  `crossDomainTrusted = true`, runs `wl-cross-domain-proxy` for the
  guest-side virtio-gpu cross-domain bridge.
- `environment.sessionVariables` pinning `WAYLAND_DISPLAY`,
  `QT_QPA_PLATFORM`, `GDK_BACKEND`, `XDG_SESSION_TYPE`,
  `SDL_VIDEODRIVER`, `CLUTTER_BACKEND`, `MOZ_ENABLE_WAYLAND`, plus
  Mesa probing knobs (`VK_DRIVER_FILES` pinned to virtio_icd + lvp,
  `MESA_LOADER_DRIVER_OVERRIDE=virtio_gpu`, `LIBGL_KOPPER_DISABLE`,
  `EGL_LOG_LEVEL=fatal`).

## Runtime invariants

- The CH + crosvm-gpu processes show up as `nixling-<vm>-gpu` in
  `ps -ef`; never as the operator's Wayland user.
- With the host filter active, the GPU runner connects to
  `/run/nixling-wlproxy/<vm>/wayland-0` and does not hold the real host
  compositor socket. The `wayland-proxy` role is the VM-specific process
  with access to the real compositor socket.
- The guest cannot reach the host compositor outside of virtio-gpu
  cross-domain (no Wayland socket bind-mount into the guest).
- With `crossDomainTrusted = false`, every `--params` payload reaching
  `crosvm device gpu` has `cross-domain` stripped — verifiable via
  `ps -fC crosvm` on the host.
- With `graphics.virglVideo = true`, the GPU process node carries the
  non-blocking status marker
  `component-specific:graphics.virglVideo=true`, and the patched
  crosvm/rutabaga build passes `VIRGL_RENDERER_USE_VIDEO` to
  virglrenderer. This is experimental and remains default-off.
- `graphics.enable = true` is x86_64-linux only. `checkVmPlatform`
  in `host.nix` throws an eval-time error naming the VM if the host
  is `aarch64-linux`.

## Lifecycle

Graphics lifecycle is daemon-supervised. `nixling vm start <vm>` sends
the request to `nixlingd`; the daemon evaluates the per-VM DAG and uses
the broker to spawn `gpu` / `gpu-render-node`, optional sidecars, and
`cloud-hypervisor-runner` in dependency order. Runners are tracked by
pidfd and are stopped/restarted through the same daemon/broker path.

Implications:

- **`nixos-rebuild switch` does NOT restart the running VM.**
  `nixlingd.service` itself carries `restartIfChanged = false`.
  After a rebuild, `nixling list`
  flags the VM with `[pending restart]` if its `current` closure
  has drifted from `booted`. Apply with `nixling vm restart <vm>`.

- **`booted` symlink is owned by the daemon start path.** The daemon
  updates per-VM `booted`/`current` state so pending-restart detection
  works for graphics and headless VMs without per-VM systemd units.

- **`nixling status <vm>` reports `pending-restart: yes/no`** with
  both store paths and the exact remediation command.

See [`docs/explanation/design.md`](../explanation/design.md#per-vm-sidecars)
for the full lifecycle rationale.

## Hardening notes

The GPU runner authority comes from the emitted minijail profile and
the broker `SpawnRunner` plan, not a per-VM service template:

- zero host capabilities unless a role-specific profile explicitly
  grants them;
- broker-controlled argv and environment;
- role-local writable paths under `/var/lib/nixling/vms/<vm>` and
  `/run/nixling-gpu/<vm>`;
- closed or fd-passed device access depending on the GPU profile;
- pidfd registration and broker audit for every spawned runner.

The spectrum-ch CH build itself carries upstream spectrum-os
sandboxing; the crosvm device gpu seccomp `.bpf` files are present
in the closure but not yet loaded at runtime (the `crosvm device gpu`
subcommand exposes no `--seccomp-policy-dir` flag in the pinned
nixpkgs rev — defence-in-depth payload waiting on an upstream knob).

## Common gotchas / failure modes

- **Black screen / no guest window.** The host `wayland-0` socket
  must be reachable as the user named by `nixling.site.waylandUser`.
  `nixling vm start <vm>` must be invoked from a Plasma session terminal —
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
- **Sidecar permission denied on the Wayland socket.** The host
  Wayland socket must exist for `nixling.site.waylandUser` before the VM
  starts. If the socket is absent, the host-side filter or the direct GPU
  fallback cannot connect to the compositor.
- **Cross-domain forwarding silently disabled.** With
  `crossDomainTrusted = false` (the default) GUI apps still work via
  virgl2 + standard virtio-gpu, but advanced cross-domain features
  (Wayland-forwarding launchpad use case) won't.

## See also

- [Design / threat model](../explanation/design.md)
- [Manifest schema](./manifest-schema.md) — graphics state is surfaced
  through the evaluated bundle and daemon status, not a per-VM
  systemd unit.
- [CLI contract](./cli-contract.md) — `nixling vm start <vm>` /
  `nixling vm stop <vm>` lifecycle.
- [`examples/graphics-workstation`](../../examples/graphics-workstation/) —
  end-to-end example with graphics + audio + USBIP YubiKey.
- [`examples/with-entra-id`](../../examples/with-entra-id/) — graphics
  VM composed with the sibling `nixos-entra-id` flake.
