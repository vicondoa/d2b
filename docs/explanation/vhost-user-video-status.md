# vhost-user-video status

## Current status

The crosvm vhost-user-media backend is wired as a supported opt-in path for
graphics VMs that set:

```nix
d2b.realms.<realm>.workloads.<workload> = {
  provider = "runtime";
  launcher.capabilities = [ "video" ];
};
```

The implementation is still intentionally narrow: H264 decode only and
daemon/broker supervision only. The default host-device allowlist is
render-node-only (`/dev/dri/renderD128`). NVIDIA VA-API/NVDEC decode requires
the explicit `graphics.videoNvidiaDecode = true` opt-in, which adds only
`/dev/nvidiactl`, `/dev/nvidia0`, and `/dev/nvidia-uvm` inside the broker's
private masked `/dev`. There is no per-VM video systemd unit and no stock
crosvm or stock Cloud Hypervisor fallback.

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
- `/dev/dri/renderD128` by default for the video runner;
- optional NVIDIA decode adds only the three reviewed NVIDIA nodes above;
- empty capabilities and `w1-video` seccomp profile.

Any NVIDIA device-node access beyond the reviewed `videoNvidiaDecode` allowlist
or any hardware encode support requires a new review and is not part of this
path.
