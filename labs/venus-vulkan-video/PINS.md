# PINS - exact revisions and measured host facts

Everything this lab depends on, pinned. Regenerate whenever `flake.lock` moves or
the host is upgraded; a stale entry here silently invalidates every capability
report and benchmark in `docs/`.

**Captured:** 2026-07-28 · **Host:** NixOS, compositor `niri`

## Forks

| Upstream | Fork | Base revision | Upstream date |
|---|---|---|---|
| `gitlab.freedesktop.org/virgl/venus-protocol` | `vicondoa/venus-protocol-vulkan-video` | `f81cb96` (W1; base `70991d4`) | 2026-07-29 |
| `gitlab.freedesktop.org/virgl/virglrenderer` | `vicondoa/virglrenderer-venus-vulkan-video` | `335be0b7` (W2; base `9ae1fb1c`) | 2026-07-29 |
| `gitlab.freedesktop.org/mesa/mesa` | `vicondoa/mesa-venus-vulkan-video` | `848ed88cbbf` (W1; base `bcf312ff5c0` = 26.1.5 - **not lock-pinned**, see below) | 2026-07-29 |

Each fork carries a `base/<rev>` tag at the exact upstream commit it was seeded
from, and does its work on a `vulkan-video` branch. The `base/` tags are what the
W1 append-only ABI gate and the W4 compatibility cross-product diff against, so
they must never be moved or deleted.

## Host hardware and driver

| Item | Value |
|---|---|
| GPU | NVIDIA **T1000** (Turing **TU117**, NVDEC generation 4) |
| Driver | **595.71.05** |
| NVIDIA Vulkan ICD | `/nix/store/rfacrwa133a0xibh0qig0lrva3n51bhz-nvidia-x11-595.71.05/lib/libGLX_nvidia.so.0` |
| ICD reported API version | **1.4.329** |
| `/run/opengl-driver` resolves to | `/nix/store/b42hqd87av5ywl57z7ih6rb1mhpizapp-graphics-drivers` |

The ICD's `library_path` points **into `/nix/store`**, which is why both
`/nix/store` and `/run/opengl-driver` must be bound into the crosvm sandbox - see
`AGENTS.md` rule 6.

### Vulkan headers

| Component | Version | Why it is pinned here |
|---|---|---|
| `vulkan-headers` | `vulkan-headers-1.4.350.0` | The single version both patched packages build against. W1 required one pinned Vulkan-Headers/`vk_video` version across both forks, because the generated protocol has to compile in Mesa and virglrenderer, which have different Meson wiring and include paths. It was absent from this manifest, so nothing tied the measurements to the header set that produced them. |

### NVDEC4 codec capability

| Codec | Hardware | NVIDIA Vulkan Video extension |
|---|---|---|
| H.264 | ✅ | ✅ `VK_KHR_video_decode_h264` (revision 9) |
| H.265 | ✅ | ✅ `VK_KHR_video_decode_h265` (revision 8) |
| VP9 | ✅ | ✅ `VK_KHR_video_decode_vp9` (revision 1) |
| AV1 | ❌ (needs NVDEC5 / Ampere) | ✅ advertised, but unusable here |

**Correction.** Earlier revisions of this file and of the plan asserted that no
NVIDIA driver ships `VK_KHR_video_decode_vp9`, citing zero device reports on
`vulkan.gpuinfo.org`. The lab's own measured host report contradicts that:

| Measurement | Location |
|---|---|
| `VK_KHR_video_decode_vp9 : extension revision 1` | `evidence/host-caps-host.txt:1393` |
| `VIDEO_CODEC_OPERATION_DECODE_VP9_BIT_KHR` in the decode queue's codec ops | `evidence/host-caps-host.txt:1535` |
| `VkPhysicalDeviceVideoDecodeVP9FeaturesKHR.videoDecodeVP9 = true` | `evidence/host-caps-host.txt:2120-2122` |
| VP9 Profile 0 / Profile 2 (8/10/12-bit) decode profiles | `evidence/host-caps-host.txt:2288-2290` |

Driver 595.71.05 ships VP9 decode on this device. The measurement is canon; the
prose was wrong.

This does **not** change the codec plan. H.264 remains the only target, because
it is what the wire format already carries and adding a second codec would widen
the surface before the first one executes. But VP9 is a *deferred* option rather
than a permanently blocked one, and the "pin YouTube to H.264 permanently"
rationale rests on scope, not on host capability. AV1 remains genuinely blocked:
Turing TU117/NVDEC4 has no AV1 decode engine, regardless of what the driver
advertises.

## Host device access (measured as user `paydro`)

| Device | Access | Notes |
|---|---|---|
| `/dev/dri/renderD128` | ✅ rw | mode 0666 |
| `/dev/nvidia0`, `/dev/nvidiactl`, `/dev/nvidia-uvm` | ✅ rw | mode 0666 |
| `/dev/udmabuf` | ✅ rw | POSIX ACL `user:paydro:rw`; backs `external-blob` |
| **`/dev/kvm`** | ❌ **denied** | 0660 `root:kvm`; ACL lists only `d2b-*` role users. See AE-1 |

## Toolchain

### Lab nixpkgs (needs Firefox 153)

| Item | Value |
|---|---|
| nixpkgs-unstable rev | `38a4887411571457d700c51c64a6e49ead2ed5ab` |
| `firefox-unwrapped` | **153.0** |
| `ffmpeg` | **8.1.2** |
| `libavcodec` soname | `libavcodec.so.62` (major 62 ≥ 60 required by Firefox) |
| ffmpeg `withVulkan` | **true** (nixpkgs default on Linux) |

### d2b's pinned nixpkgs (the baseline being replaced)

| Item | Value |
|---|---|
| virglrenderer | **1.3.0**, mesonFlags `-Dvideo=true -Dvenus=true` |
| Mesa | **26.1.1** |
| ffmpeg | 8.1 |
| firefox-unwrapped | 151.0.1 |
| vulkan-headers / loader / tools | 1.4.341.0 |
| cloud-hypervisor | 52.0 |
| passt | `2025_09_19` |

Note the lab intentionally uses a **newer nixpkgs** than d2b's pin. That drift is
the point of the isolation, and is documented rather than reconciled.

## Protocol facts (verified at `base/70991d4`)

| Fact | Value |
|---|---|
| `VN_WIRE_FORMAT_VERSION` | **1** - must **not** change (Venus requires exact guest/renderer equality) |
| `VK_COMMAND_TYPE_*` values assigned | **345** |
| Highest assigned value | **345** |
| Video command types present | **0** |
| First free value for video commands | **346** |
| `xmls/` contents | `vk.xml`, `VK_EXT_command_serialization.xml`, `VK_MESA_venus_protocol.xml` - no separate `video.xml` |

Command IDs are **explicitly assigned** in `VK_EXT_command_serialization.xml`
(e.g. `<enum value="50" name="VK_COMMAND_TYPE_vkCreateBuffer_EXT"/>`), not
derived from position in `VK_XML_EXTENSION_LIST`. The 13 H.264 video commands
therefore append at **346-358**, and no pre-existing value may change.

## Firefox gates (verified)

| Gate | Status |
|---|---|
| `MOZ_ENABLE_VULKAN_VIDEO` | ✅ `set_config(..., when=toolkit_gtk)` in `toolkit/moz.configure` - unconditional for GTK builds |
| nixpkgs Firefox toolkit | ✅ `cairo-gtk3-wayland` (`build-mozilla-mach/default.nix:299-303,482`) → `toolkit_gtk` true |
| `LIBAVCODEC_VERSION_MAJOR >= 60` | ✅ 62 |
| ffmpeg `--enable-vulkan` | ✅ |
| `media.hardware-video-decoding-vulkan.enabled` | set by policy at runtime |
| Hardware WebRender | must be proven in-guest |
| `gfxVars::UseH264HwDecode()` | must be verified in-guest |

For contrast, `MOZ_ENABLE_V4L2` is gated to `target.cpu in ("arm","aarch64","riscv64")` -
which is precisely the line the existing d2b Firefox fork patches to add
`x86_64`, and which this lab makes unnecessary.

## Which base revisions are lock-verified

`pins-check.sh` compares this file against `flake.lock`, so a row is only
*verified* if the revision is a locked input. Two of the three are:

| fork | base row | lock-pinned? |
|---|---|---|
| venus-protocol | `70991d4` | yes - `venus-protocol-base` |
| virglrenderer | `9ae1fb1c` | yes - `virglrenderer-base` |
| mesa | `bcf312ff5c0` | **no** |

The mesa fork was rebased onto 26.1 to match the nixpkgs derivation, so its
original `base/` tag is no longer an ancestor of the `vulkan-video` branch and
cannot be fetched from the remote. There is nothing to pin.

That row is documentation, not a verified pin, and is marked as such rather
than left looking like the other two. The distinction matters because the base
revisions are what the append-only ABI gate and the W4 compatibility diff
compare against: a base that cannot be fetched cannot be diffed.
