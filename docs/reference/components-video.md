# Reference: video sidecar component

## What this component does

The video sidecar adds a per-VM `vhost-device-video` process that exposes a
virtio-video / vhost-user-media socket to the guest. In nixling it is a
graphics-adjacent sidecar: it is started only for graphics VMs and shares the
same host ownership model as the GPU sidecar.

The host-side process runs as `nixling-<vm>-gpu`, not as a separate
`nixling-<vm>-video` user.

## Enablement model

There is no separate `nixling.vms.<vm>.video.enable` toggle today.

- `nixos-modules/default.nix` always imports `components/video/{host,guest}.nix`.
- The host unit and guest wiring are activated only when
  `nixling.vms.<vm>.graphics.enable = true`.

That is why the video sidecar uses the same per-VM graphics boundary and shares
`nixling.vms.<vm>.graphics.waylandUser` as its trust anchor.

## Host-side resources created

When graphics is enabled for a VM, the v1.0 daemon-only DAG provisions
(via broker `SpawnRunner` per ADR 0015):

| Resource | Shape |
| --- | --- |
| Supervisor | `nixlingd` DAG executor (pidfd-owned) |
| runtime directory | `/run/nixling-video/<vm>/` |
| vhost-user socket | `/run/nixling-video/<vm>/video.sock` |
| system user | `nixling-<vm>-gpu` |

The runner is gated on the matching Wayland socket under
`/run/nixling-wl/<vm>/wayland.sock` and then launches:

```text
vhost-device-video \
  --socket-path /run/nixling-video/<vm>/video.sock \
  --wayland-sock /run/nixling-wl/<vm>/wayland.sock
```

## Guest-side resources created

Inside the guest, the component adds:

- `microvm.devices = [ { bus = "pci"; path = "/run/nixling-video/<vm>/video.sock"; } ]`
- `boot.kernelModules = [ "virtio_video" ]`
- `environment.systemPackages = [ v4l-utils ]`

That gives the guest a virtio-video device and a minimal userspace toolset to
inspect it.

## Runtime invariants

- The video sidecar exists only for graphics VMs.
- The service carries `restartIfChanged = false`, matching nixling's per-VM
  lifecycle invariant.
- Startup is gated on the Wayland relay socket already existing.
- The guest device path is always the host-created UNIX socket above; no TCP or
  vsock transport is involved.

## Hardening notes

The service is intentionally narrow:

- `RestrictAddressFamilies = [ "AF_UNIX" ]`
- empty capability bounding set + ambient capabilities
- `NoNewPrivileges = true`
- `PrivateTmp = true`
- `ProtectSystem = "strict"`
- `ProtectHome = true`
- `ProtectKernelTunables = true`
- `ProtectControlGroups = true`
- `LockPersonality = true`
- `MemoryDenyWriteExecute = true`
- `RestrictRealtime = true`
- `SystemCallArchitectures = "native"`
- syscall filter only opens the small allowlist needed for the media sidecar

`tests/video-sidecar-hardening-eval.sh` asserts those hardening defaults at
eval time.

## Daemon-spawned shape (P1 end-state)

Starting with P1 the per-VM `nixling-<vm>-video.service` systemd template
is retired; `nixlingd` spawns the role directly via the privileged
broker's `SpawnRunner` path. The on-host shape moves from:

| Surface | pre-P1 (systemd template) | P1 daemon-spawned |
| --- | --- | --- |
| Process supervisor | `systemd` per-VM unit | `nixlingd` DAG executor |
| Profile binding | unit's `serviceConfig` + tmpfiles | broker minijail profile `vm-<vm>-video` |
| Cgroup leaf | `system.slice/nixling-<vm>-video.service` | `nixling.slice/<vm>/video` |
| Capability set | unit's `CapabilityBoundingSet = ""` | `caps = [ ]` in `vm-<vm>-video` (kernel-r2-4) |
| Render node bind | `DeviceAllow` on the unit | `readOnlyPaths += [ "/dev/dri/renderD128" ]` |
| Socket dir bind | `RuntimeDirectory = nixling-video/<vm>` | `writablePaths += [ videoRuntimeDirOf <vm> ]` |
| Seccomp ref | inherited from `SystemCallFilter` | `seccompPolicyRef = "w1-video"` |
| Validator | `tests/video-sidecar-hardening-eval.sh` (eval) | `tests/minijail-validator-video.sh` (positive + SIGSYS negative) |

The argv generator
[`nixling_host::video_argv`](https://github.com/vicondoa/nixling/blob/main/packages/nixling-host/src/video_argv.rs)
is the canonical source of the per-VM crosvm vhost-user-media invocation
the daemon hands to the broker. The byte-parity golden lives at
[`tests/golden/runner-shape/video-argv-minimal.txt`](../../tests/golden/runner-shape/video-argv-minimal.txt)
and `tests/video-argv-shape.sh` byte-compares it against the snapshot
test on every PR.

## kernel-8 wire-contract pins

`pkgs/spectrum-ch/cloud-hypervisor/0003-vhost-user-media-device.patch`
hard-codes the virtio-media wire shape that the video sidecar speaks to
the guest through cloud-hypervisor. These constants are **not** argv
flags — they live in the CH patch and the crosvm vhost-user-media
backend. The P1 golden mirrors them so any drift surfaces as a single
byte diff in CI:

| Pin | Value | Source line in `0003-vhost-user-media-device.patch` |
| --- | --- | --- |
| `virtio_id` | `48` | `const VIRTIO_ID_MEDIA: u32 = 48` |
| `num_queues` | `2` | `const NUM_QUEUES: u16 = QUEUE_SIZES.len() as _` |
| `queue_size` | `256` (both queues identical) | `const QUEUE_SIZES: &[u16] = &[256, 256]` |
| `shm_region_bytes` | `256 * 1024 * 1024` (= 268 435 456) | `VhostSharedMemoryRegion { length: 256 * 1024 * 1024, .. }` |
| `vring_base` | `0` (forced for every queue at activate) | `self.vu_common.vring_bases = Some(vec![0; queues.len()])` |
| `protocol_flags` | `SHMEM_MAP_CROSVM \| BACKEND_REQ \| REPLY_ACK` | `acked_protocol_features` mask |
| `mmio_allocator` | `pci-mem64` | `self.pci_segments[..].mem64_allocator.allocate(..)` |

Mirrored verbatim by the Rust constants
[`VIRTIO_ID_MEDIA`](https://github.com/vicondoa/nixling/blob/main/packages/nixling-host/src/video_argv.rs),
`VHOST_USER_MEDIA_NUM_QUEUES`, `VHOST_USER_MEDIA_QUEUE_SIZE`,
`VHOST_USER_MEDIA_SHM_REGION_BYTES`, `VHOST_USER_MEDIA_VRING_BASE`,
`VHOST_USER_MEDIA_PROTOCOL_FLAGS`, and
`VHOST_USER_MEDIA_MMIO_ALLOCATOR`. Changing any of the CH patch values
without updating the constant + the golden is a fail-closed regression.

## Guest kernel module (P2 follow-up)

The guest needs the `virtio_media` driver loaded. Per the P2 plan
(`kernel-r2-7` corrected) this is wired through
`boot.extraModulePackages` for every video-enabled VM. **Not yet
shipped** as of P1 — flagged here so operators don't enable the host
sidecar against a guest that can't bind it. Track via
`p2-kernel-modules` in the wave plan.

## Common gotchas

### The unit never starts

Check that the VM is actually a graphics VM. The video unit is filtered out for
headless VMs.

### The unit waits forever on `wayland.sock`

The video sidecar depends on the GPU/Wayland path creating
`/run/nixling-wl/<vm>/wayland.sock` first. Fix the graphics side first.

### You expected a separate `video.enable` option

There is none today. Treat video as part of the graphics stack.

## See also

- [`components-graphics.md`](./components-graphics.md)
- [`components-audio.md`](./components-audio.md)
- [`store-virtiofs.md`](./store-virtiofs.md)
