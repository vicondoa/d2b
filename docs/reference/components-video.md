# Reference: video sidecar component

## What this component does

The video sidecar exposes the historical H264 decode path for graphics VMs:

```text
guest ffmpeg h264_v4l2m2m
  -> /dev/video*
  -> guest virtio_media driver
  -> patched Cloud Hypervisor --vhost-user-media
  -> patched crosvm device video-decoder --backend vaapi
  -> host VA-API through the declared video device allowlist
```

This is a decode path, not a hardware encoder. The supported host device
contract is a closed allowlist: by default the video runner sees only
`/dev/dri/renderD128`. NVIDIA VA-API/NVDEC requires an explicit
`graphics.videoNvidiaDecode = true` opt-in, which adds only
`/dev/nvidiactl`, `/dev/nvidia0`, and `/dev/nvidia-uvm`; broad `/dev`
access is still masked.

## Enablement model

Video is an explicit graphics opt-in:

```nix
d2b.vms.<vm>.graphics.enable = true;
d2b.vms.<vm>.graphics.videoSidecar = true;
```

`graphics.videoSidecar = true` without `graphics.enable = true` fails eval.

On NVIDIA hosts using the proprietary VA-API/NVDEC backend, explicitly opt in
to the additional closed device set:

```nix
d2b.vms.<vm>.graphics.videoNvidiaDecode = true;
```

This requires `graphics.videoSidecar = true`.
Video remains default-off so headless and ordinary graphics VMs do not build
or start the media backend.

Firefox does not consume this `/dev/video*` V4L2 M2M path directly. Firefox's
Linux hardware decode path is VA-API; the experimental switch for that is
`d2b.vms.<vm>.graphics.virglVideo`, documented in
[`components-graphics.md`](./components-graphics.md). Keep the distinction
clear: `videoSidecar` is the daemon-spawned vhost-user media device, while
`virglVideo` advertises VA-API video through the GPU/virglrenderer path.
Video decode is distinct from display streaming and GPU acceleration; see
[display and virtual I/O capabilities](./display-io-capabilities.md).

## Host-side resources

When enabled, the daemon-owned process DAG adds a `video` node. There is no
per-VM systemd unit; `d2bd` asks `d2b-priv-broker` to spawn the runner
with `SpawnRunner { role: Video }` and tracks it by pidfd.

| Resource | Shape |
| --- | --- |
| Supervisor | `d2bd` DAG executor |
| Runner role | `RunnerRole::Video` |
| Minijail profile | `vm-<vm>-video` |
| Principal | `d2b-<vm>-video` |
| Runtime directory | `/run/d2b-video/<vm>/` |
| vhost-user socket | `/run/d2b-video/<vm>/video.sock` |
| Binary | patched crosvm video build from `nixos-modules/processes-json.nix` |
| argv | `crosvm device video-decoder --socket-path /run/d2b-video/<vm>/video.sock --backend vaapi` |
| Device allowlist | default: `/dev/dri/renderD128`; with `graphics.videoNvidiaDecode = true`: also `/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm` |

Cloud Hypervisor must be the vendored patched `pkgs/spectrum-ch` build and its
final argv must contain exactly one:

```text
--vhost-user-media socket=/run/d2b-video/<vm>/video.sock
```

`tests/video-contract-eval.sh` asserts this final evaluated shape, including
the VM-attribute-name socket identity when the guest `networking.hostName`
differs.

## Guest-side resources

The guest module adds:

- `microvm.cloud-hypervisor.extraArgs = [ "--vhost-user-media" "socket=/run/d2b-video/<vm>/video.sock" ]`
- `boot.extraModulePackages = [ virtio-media-driver ]`
- `boot.kernelModules = [ "virtio_media" ]`

`virtio_media` is a guest driver. Host-side preflights must not require
`virtio_media` in host `/proc/modules`.

## Runtime invariants

- Transport is AF_UNIX only. There is no TCP, vsock, or alternate listener
  form for the media path.
- The runtime directory ACL grants only the video sidecar UID and the
  cloud-hypervisor runner UID. Other same-VM sidecars do not inherit
  `video.sock` access.
- The video principal is dedicated. It is not the GPU principal, and
  activation/broker ACL refreshes must not grant it host Wayland, PipeWire,
  or Pulse session sockets.
- The broker masks `/dev` for the video runner and recreates only the
  declared `deviceBinds`; NVIDIA decode is not a broad `/dev` bind.
- The video node depends on the actual graphics predecessor:
  `gpu -> video` for normal graphics and `gpu-render-node -> video` for
  render-node-only graphics.
- Cloud Hypervisor starts only after the video socket reaches the daemon's
  non-destructive listening readiness predicate.
- The video argv contract is closed. Free-form crosvm extra args and backend
  overrides are not accepted.
- The broker verifies `SpawnRunner` VM, role id, and role selector match the
  trusted bundle intent before spawning.

## Wire-contract pins

`pkgs/spectrum-ch/cloud-hypervisor/0003-vhost-user-media-device.patch`
hard-codes the virtio-media wire shape. The Rust constants in
`d2b_host::video_argv` and
`tests/golden/runner-shape/video-argv-minimal.txt` mirror these values:

| Pin | Value |
| --- | --- |
| virtio id | `48` |
| queues | `2` |
| queue size | `256` |
| shared-memory region | `268435456` bytes |
| vring base | `0` |
| protocol flags | `BACKEND_REQ\|REPLY_ACK\|SHMEM_MAP_CROSVM` |
| allocator | `pci-mem64` |

## See also

- [`components-graphics.md`](./components-graphics.md)
- [`privileges.md`](./privileges.md)
- [`runner-shape-audit.md`](./runner-shape-audit.md)
