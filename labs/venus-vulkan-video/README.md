# Venus Vulkan Video lab

An isolated prototype: make **stock, unmodified upstream Firefox** inside a guest
VM decode H.264 on the host NVIDIA GPU via `VK_KHR_video_decode_h264` forwarded
through Venus/virtio-gpu - while Firefox itself is GPU-rendered through the same
virtio-gpu device.

> **Experimental. Not production.** See [`AGENTS.md`](./AGENTS.md) for the
> binding isolation contract before changing anything here.

## Status: working

Unmodified Firefox decodes H.264 on the host GPU and the picture is correct.
Measured on a clean boot:

```
plane image failures : 0
layer-validation err : 0
BLIT failures        : 0
CmdSubmit3d refusals : 0
Illegal cmd buffer   : 0
guest coredumps      : 0
renderer decode      : decode_cmds=1024 sessions=1
host NVDEC engine    : nonzero in 28 of 35 samples, mean 2.83%, max 6%
```

Firefox carries **no patches**. Every change is in guest Mesa, host
virglrenderer, or lab configuration. `SOLUTION.md` is the full account of what
was wrong and why each change is needed; the short version is below.

### Reading the NVDEC number

About 3% is what 720p30 costs in real time on a T1000, not a throughput figure.
The number that carries the claim is the contrast: a software-decoding Firefox
reads a flat **0** on every sample, and the same clip decoded by guest `ffmpeg`
as fast as it can reads **98%** against 99% host-native. Real-time playback sits
where it should between those.

### What had to be true

A decoded NV12 frame is one `VkImage` with one `VkDeviceMemory`, exported as one
dmabuf with luma at offset 0 and chroma at offset 983040. Firefox imports the two
planes as separate EGL images, which is the portable thing that works on every
native driver. Four separate defects sat between that and a correct frame:

| # | Defect | Where |
|---|---|---|
| 1 | Both plane fds resolve to one GEM handle, so the second import looked like plane 0 of a fresh buffer | guest Mesa winsys |
| 2 | `*blob_mem` was dropped on the cache-hit path, and the caller decides from it whether to describe the resource at all | guest Mesa winsys |
| 3 | A description was only sent while the resource was untyped, so a wider one covering a new plane never left the guest | guest Mesa winsys |
| 4 | The host built no image for the extra plane, and when it did, used a fourcc this driver refuses (`GR88`, not `RG88`) | virglrenderer |

Each one alone is enough to produce a wrong picture, which is why partial fixes
kept relocating the symptom rather than removing it.

## Why

Today, hardware video decode for Firefox in a d2b graphics VM needs a **forked
Firefox** that patches `FFmpegVideoDecoder.cpp` to force the V4L2 decoder,
bypass the hardware-WebRender gate, and hard-enable H.264. Decode runs over
`virtio-media` → host VA-API, which is a *separate path* from rendering. The fork
must be rebased on every Firefox release.

This lab moves decode into the virtio-gpu/Venus path so decoded frames are
first-class Vulkan images on the same GPU as rendering, and the Firefox fork can
be deleted.

## The core insight

Firefox 153 already ships a complete Vulkan Video decoder, and
`MOZ_ENABLE_VULKAN_VIDEO` is compiled in **unconditionally for every GTK build**:

```python
# toolkit/moz.configure
set_config("MOZ_ENABLE_VULKAN_VIDEO", True, when=toolkit_gtk)
```

`SelectVulkanDecoderPhysicalDevice()` hard-gates only on `VK_KHR_video_queue` +
`VK_KHR_video_decode_queue` being present. Venus advertises neither, so Firefox
silently falls back to VA-API and then software.

**So the browser needs no changes at all.** Every bit of the work is below it:

```
stock Firefox → libavcodec.so.62 (--enable-vulkan)
  → AV_HWDEVICE_TYPE_VULKAN → VK_KHR_video_decode_h264
  → Mesa Venus (guest)            ← we add extension exposure
  → virtio-gpu → crosvm/rutabaga  ← unchanged, forwards bytes uninspected
  → virglrenderer Venus renderer  ← we add vkr_video.c
  → host NVIDIA Vulkan ICD → T1000 NVDEC4
```

## Current upstream state

Nothing exists. This is greenfield across three projects, verified by direct
source inspection:

| Layer | State |
|---|---|
| venus-protocol | zero video extensions in `VK_XML_EXTENSION_LIST`; no wire commands |
| virglrenderer | no `vkr_video.c`; `vkr_extension_table` strips video before the guest sees it |
| Mesa Venus | no video passthrough, **and** actively strips video format-feature bits from NV12 (MR !35842) |
| upstream MRs in flight | **none**, in any of the three |
| crosvm / rutabaga | transparent - **no changes needed** |

## Codec scope: H.264 only, permanently

| Codec | Venus | NVIDIA driver | T1000 (Turing TU117, NVDEC4) |
|---|---|---|---|
| **H.264** | we add it | ✅ shipped | ✅ |
| H.265 | - | ✅ shipped | ✅ (not needed) |
| VP9 | - | ❌ **never shipped** | ✅ |
| AV1 | - | ✅ shipped | ❌ **no hardware** |

`VK_KHR_video_decode_vp9` was ratified in 2025 but **no NVIDIA driver implements
it** (vulkan.gpuinfo.org: zero NVIDIA reports on any platform), and Turing has no
AV1 engine. YouTube is therefore pinned to H.264 by preference - a
configuration-only measure, so Firefox stays unmodified.

## Forks

| Upstream | Fork | Base |
|---|---|---|
| `virgl/venus-protocol` | [`vicondoa/venus-protocol-vulkan-video`](https://github.com/vicondoa/venus-protocol-vulkan-video) | `base/70991d4` |
| `virgl/virglrenderer` | [`vicondoa/virglrenderer-venus-vulkan-video`](https://github.com/vicondoa/virglrenderer-venus-vulkan-video) | `base/9ae1fb1c` |
| `mesa/mesa` | [`vicondoa/mesa-venus-vulkan-video`](https://github.com/vicondoa/mesa-venus-vulkan-video) | see [`PINS.md`](./PINS.md) |

Work happens on each fork's `vulkan-video` branch; `upstream` remains a remote so
changes stay rebaseable and upstreamable.

## Requirements

- NixOS host with an NVIDIA GPU (developed against a **T1000**, driver 595.71.05)
- A running Wayland session
- `/dev/dri/renderD128`, `/dev/nvidia*`, `/dev/udmabuf` readable by your user
- KVM access - see below

## Running it

Everything runs as your own user from `nix build` outputs. **No
`nixos-rebuild switch`, no `/etc/nixos` change, no systemd unit.**

```bash
# Resolve the lab through git+file, never a bare path: a `path:`/`.#` ref makes
# Nix copy the entire working tree into the store. See AGENTS.md rule 4.
LAB_FLAKE="git+file://$(git rev-parse --show-toplevel)?dir=labs/venus-vulkan-video"

nix run "$LAB_FLAKE#lab-vm"
```

That single command supplies every dependency and the built image, and it
**owns the `/dev/kvm` grant lifecycle**: it requests the grant if needed (one
`sudo setfacl`, non-persistent) and revokes *any* per-user ACL entry on exit -
including one left behind by an earlier crash. You do not need to grant it
yourself.

The helper is available for inspection and manual recovery:

```bash
bash host/grant-kvm.sh --status     # can this user open /dev/kvm?
bash host/grant-kvm.sh --has-acl    # is an ACL entry present? (no sudo)
bash host/grant-kvm.sh --revoke     # manual recovery after a hard crash
```

> **Granting KVM manually before launching is not recommended.** The launcher
> will still revoke it on exit, but while it is in place *any* process running
> as your user can open `/dev/kvm`. See AE-1 in the plan: this is an accepted,
> unresolved exception forced by the no-host-switch constraint, and a SIGKILL
> or hard crash can still bypass the trap. Run `--revoke` if in doubt.

The launcher starts, in order: `passt` (unprivileged NAT, no TAP), a per-run
nested `cage` compositor, the bubblewrapped crosvm GPU sidecar linked against
the **lab** virglrenderer, then Cloud Hypervisor. It tears all of them down by
process group on exit.

> **Contention warning.** The lab shares `/dev/kvm`, the render node,
> `/dev/udmabuf`, RAM and the GPU with any running d2b VMs. Stop them before
> taking measurements; the launcher warns when it detects them.

## Layout

```
flake.nix / flake.lock   self-contained; own nixpkgs pin (needs Firefox 153)
PINS.md                  exact revisions of everything (regenerate when the lock moves)
pkgs/                    the three forks + crosvm override + CH + stock Firefox
guest/                   guest NixOS config and disk image
host/                    grant-kvm.sh, run-lab-vm.sh
tests/                   capability probes, evidence and negative-control harnesses
docs/                    design, baseline, findings, upstreaming
```

Mutable state (fork clones, build output, disk overlays, evidence) lives
**outside the repo** at `${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/`, and
runtime state under `$XDG_RUNTIME_DIR/venus-lab/<runid>/`. This is deliberate -
see rule 4 in [`AGENTS.md`](./AGENTS.md).

## Evidence, not vibes

Firefox's fallback is **silent**, so "the video played" proves nothing. Every
claim needs an artifact and a negative control - a rerun with the feature
disabled where the decode commands and GPU decode-engine activity drop to zero.
Notably, `vkCmdDecodeVideoKHR` firing *plus* hardware WebRender is still not
sufficient to prove frames stayed GPU-backed; frames can round-trip through the
CPU before compositing. The full contract is in the plan's Evidence section.
