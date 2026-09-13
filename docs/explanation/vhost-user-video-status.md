> **Historical or compatibility reference.** Current product behavior uses Zone-owned Resources, the Guest controller, d2bd, and typed Providers. Older VM, environment, and lifecycle names on this page are retained for migration or evidence only.

# vhost-user-video status

## Current status

The crosvm vhost-user-media backend is wired as a supported opt-in path for
graphics VMs that set:

```nix
d2b.vms.<vm>.graphics.videoSidecar = true;
```

The implementation is still intentionally narrow: H264 decode only and
daemon/broker supervision only. The video sidecar's declared Process row
selects one of two closed worker templates. `video-worker` binds only the
render node (`/dev/dri/renderD128`); the explicit
`graphics.videoNvidiaDecode = true` opt-in selects `video-worker-nvidia`,
whose posture adds only `/dev/nvidiactl`, `/dev/nvidia0`, and
`/dev/nvidia-uvm` alongside the render node inside the broker's private
masked `/dev`. There is no per-VM video systemd unit and no stock crosvm or
stock Cloud Hypervisor fallback.

## Historical blocker

An earlier assessment on 2026-06-03 found two blockers in the inline
`crosvmVideo` derivation:

1. `pkgs/vhost-user-video/` was copied into crosvm but not registered in
   crosvm's vhost-user backend module or `device` CLI, so
   `crosvm device video-decoder` was unavailable.
2. The injected backend reused crosvm media helper types that were private in
   the pinned crosvm revision.

The current derivation resolves those in `nixos-modules/processes-json.nix` by
registering the `video` module/subcommand and making the required crosvm media
helper types public within the patched build. Static validation now builds the
patched crosvm video binary and checks `device video-decoder --help` on the
exact store path referenced by the trusted process graph.

## Why the implementation stays narrow

The sidecar is a host GPU attack surface, so d2b keeps the exposed surface
closed:

- one AF_UNIX socket per VM;
- `--backend vaapi` only;
- no free-form crosvm video extra args;
- no TCP/vsock listener forms;
- the `video-worker` template binds `/dev/dri/renderD128` only;
- `graphics.videoNvidiaDecode = true` selects `video-worker-nvidia`, which
  adds only the three reviewed NVIDIA nodes above;
- empty capabilities and `w1-video` seccomp profile.

Any NVIDIA device-node access beyond the reviewed `videoNvidiaDecode` allowlist
or any hardware encode support requires a new review and is not part of this
path.
