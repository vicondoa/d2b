# W0 baseline - measured facts

Everything here was **measured on the target host**, not assumed. Commands are
reproducible; raw reports land in
`${XDG_STATE_HOME:-$HOME/.local/state}/venus-lab/evidence/`.

**Captured:** 2026-07-28 · NVIDIA T1000, driver 595.71.05, NixOS, compositor `niri`

## 1. Host Vulkan Video capability - ✅ PASS

```bash
bash tests/host-caps.sh --host
```

| Capability | NVIDIA T1000 | llvmpipe |
|---|---|---|
| `VK_KHR_video_queue` | ✅ (rev 8) | ❌ |
| `VK_KHR_video_decode_queue` | ✅ | ❌ |
| `VK_KHR_video_decode_h264` | ✅ | ❌ |
| `QUEUE_VIDEO_DECODE_BIT_KHR` | ✅ | - |
| `videoCodecOperations` H264 | ✅ | - |

Device block: `deviceName = NVIDIA T1000`, `driverName = NVIDIA`,
`driverInfo = 595.71.05`.

**This is the go/no-go gate for the whole prototype, and it passes.** The host
driver already does everything we intend to forward; the missing pieces are
entirely in the Venus stack.

> **Evidence-quality note.** The probe deliberately attributes capabilities to a
> **specific device block**, because this host also exposes `llvmpipe`. An
> earlier whole-file `grep` version of this check reported PASS for reasons that
> would also have held if the NVIDIA ICD had vanished entirely - precisely the
> false-pass mode the panel warned about.

## 2. Capability inside the crosvm sandbox - ✅ PASS (after two fixes)

```bash
bash tests/host-caps.sh --in-sandbox
```

Same four capabilities present on the NVIDIA device from **inside** the
bubblewrap namespace the GPU sidecar will use.

This did not work on the first attempt, and the failures are the point of
running it:

| Symptom | Cause | Fix |
|---|---|---|
| `bwrap: execvp …: No such file or directory` | `#!/usr/bin/env bash` shebang - `/usr/bin` is not bound | invoke bash by absolute path |
| `readlink: command not found` | `/run/current-system/sw` not bound, so coreutils unreachable | bind it (symlink farm into the already-bound `/nix/store`; grants no new access) |
| **`vkCreateInstance` SEGFAULT** | **`/sys` not bound** - NVIDIA userspace enumerates the GPU through sysfs | **bind `/sys` read-only** |

`/sys`, `/dev/nvidia-modeset` and `/dev/nvidia-uvm-tools` were **absent from the
originally planned bind set**. Without them the sidecar would have reported no
NVIDIA Vulkan Video at all, while a host-shell probe kept passing - the exact
silent divergence the panel required this check to catch.

## 3. Protocol baseline (venus-protocol `base/70991d4`)

| Fact | Value |
|---|---|
| `VN_WIRE_FORMAT_VERSION` | `1` (line 28 of `vn_protocol.py`) |
| Video extensions in `VK_XML_EXTENSION_LIST` | **0** |
| `VK_COMMAND_TYPE_*` values assigned | **345** |
| Highest assigned value | **345** |
| Video command types | **0** |
| First free value | **346** |
| `xmls/` | `vk.xml`, `VK_EXT_command_serialization.xml`, `VK_MESA_venus_protocol.xml` |

**Correction to the plan's risk model.** The panel raised a HIGH finding that
adding entries to `VK_XML_EXTENSION_LIST` might renumber existing
`VK_COMMAND_TYPE_*` values and silently break old guests. Inspection shows IDs
are **explicitly assigned in the XML**:

```xml
<enum value="50"    name="VK_COMMAND_TYPE_vkCreateBuffer_EXT"/>
```

They are *not* derived from position in `VK_XML_EXTENSION_LIST`. The renumbering
hazard is therefore structurally much smaller than feared - but the append-only
gate still earns its place, because the values are **hand-assigned** and a typo
or reused value would be just as damaging. The 13 H.264 commands take **346-358**.

## 4. Mesa baseline (`base/5b7bcac9bab`, upstream 2026-07-29)

| Claim | Verified |
|---|---|
| No video extension in `vn_physical_device_get_passthrough_extensions()` | ✅ 0 matches |
| MR !35842 format scrubbing present | ✅ `allowed_ycbcr_feats` at `vn_physical_device.c:2118` |
| Allowlist contains no `VIDEO` feature bits | ✅ 0 matches - decode bits stripped from NV12 |

Mesa is not merely missing video support; it is **actively hardened against it**.

## 5. virglrenderer baseline (`base/9ae1fb1c`, upstream 2026-07-20)

No `vkr_video.c`. `vkr_extension_table` has no video entry, so
`vkr_extension_get_spec_version()` returns 0 and video extensions are stripped
before ever reaching the guest.

## 5a. Generated-protocol distribution - ✅ substantially de-risks the top plan risk

The plan's highest-severity technical risk was that `StdVideo*` structs live
outside `vk.xml` and the generator has no schema for them. Inspection shows the
situation is much better than assumed: **the StdVideo headers are already
vendored on both sides.**

| Consumer | Vendored generated headers | StdVideo headers |
|---|---|---|
| virglrenderer | `src/venus/venus-protocol/vn_protocol_renderer_*.h` (46 files) | ✅ `src/venus/venus-protocol/vk_video/` - includes `vulkan_video_codec_h264std{,_decode}.h` |
| Mesa | `src/virtio/venus-protocol/vn_protocol_driver_*.h` (38 files) | ✅ `include/vk_video/` |

`vn_protocol.py` emits **two variants** from one source via `--outdir`:
`vn_protocol_renderer_*` for virglrenderer and `vn_protocol_driver_*` for Mesa.
Both are **committed** generated files upstream, which settles the c-reviewer's
"commit vs regenerate" question: follow upstream and commit them.

So the missing piece is the **serialization code** for these types, not the type
definitions - the H.264 `StdVideo*` structs are already present, in-tree, on both
sides. Example, already available to both consumers:

```c
typedef struct StdVideoDecodeH264PictureInfo {
    StdVideoDecodeH264PictureInfoFlags    flags;
    uint8_t                               seq_parameter_set_id;
    ...
```

This does not remove the ABI hazard the c reviewer raised (these structs contain
bitfields and remain fragile to hand-rolled layouts), so the field-level schema
requirement in W1 stands. But it removes an entire class of feared work.

## 5b. crosvm ↔ virglrenderer binding - ✅ PROVEN

```bash
nix run "$LAB_FLAKE#prove-crosvm-binding"
```

| Check | Result |
|---|---|
| `--gpu-device-node` present in `crosvm device gpu --help` | ✅ PASS - the patch applied |
| crosvm closure references the **lab** virglrenderer | ✅ PASS - direct store reference |

Built artifacts:

- `virglrenderer-venus-vulkan-video-lab-9ae1fb1`
- `crosvm-venus-vulkan-video-0-unstable-2026-07-15`

This is the check that catches the `symlinkJoin` trap, where crosvm keeps
resolving nixpkgs' stock virglrenderer through its RPATH while everything
appears to work. `crosvm.override { virglrenderer = labVirglrenderer; }` is a
real relink and is confirmed as such.

## 5c. Guest Mesa ICD - ✅ PROVEN, and video baseline confirmed

```bash
nix run "$LAB_FLAKE#prove-guest-icd"
```

| Check | Result |
|---|---|
| Lab Mesa emits a Venus (`virtio`) ICD | ✅ `virtio_icd.x86_64.json`, api 1.4.354 |
| ICD `library_path` resolves **inside** the lab Mesa | ✅ `…-mesa-venus-vulkan-video-lab-bcf312f/lib/libvulkan_virtio.so` |
| `KHR_video_queue` in passthrough table | absent - correct W0 baseline |
| `KHR_video_decode_queue` in passthrough table | absent - correct W0 baseline |
| `KHR_video_decode_h264` in passthrough table | absent - correct W0 baseline |
| NV12 video format-feature bits | **STRIPPED** (MR !35842) - correct W0 baseline |

> **A false pass caught in the harness itself.** The first version of check 3
> ran `strings` over `libvulkan_virtio.so` and reported all three extensions
> **PRESENT**. That was wrong: every Vulkan extension *name* appears in Mesa's
> common extension-name table regardless of driver support. The real signal is
> Venus's static passthrough allowlist in
> `src/virtio/vulkan/vn_physical_device.c`, so the check now inspects the
> source that is actually built. Same class of error the panel flagged for
> Firefox evidence - worth repeating that a check which cannot fail proves
> nothing.

This check is also the **W4 flip-detector**: when guest exposure lands, these
four lines invert.

### Mesa fork rebased onto upstream 26.1

The fork's `vulkan-video` branch was moved from `main` to upstream's **26.1**
branch (`base/bcf312ff5c0`, 26.1.5). Building Mesa `main` against nixpkgs'
derivation fails with `ERROR: Unknown option: "clang-libdir"`, because that
option is introduced by nixpkgs' own `opencl.patch`, which no longer applies to
main's tree. 26.1 is the better target regardless: it matches the nixpkgs
derivation version exactly, the passthrough table is identical between main and
26.1, MR !35842 is present in 26.1.x, and d2b's deployed guests already run
26.1.x.

Corollary worth remembering: do **not** clear `patches` when overriding a
nixpkgs derivation's `src` - dropping the patch list silently removes build
options the derivation still passes.

## 5d. Guest baseline from inside the booted VM - ✅ CAPTURED

The VM boots to a graphical target with no host configuration change, and the
guest emits its capability report to the serial console at boot.

```
icd_json = …-mesa-venus-vulkan-video-lab-bcf312f/share/vulkan/icd.d/virtio_icd.x86_64.json
icd_lib  = …-mesa-venus-vulkan-video-lab-bcf312f/lib/libvulkan_virtio.so

deviceName = Virtio-GPU Venus (NVIDIA T1000)
driverName = venus
apiVersion = 1.4.329

VK_KHR_video_queue        = absent
VK_KHR_video_decode_queue = absent
VK_KHR_video_decode_h264  = absent
video_decode_queue_bit    = PRESENT
dev_video_nodes           = (none)
```

Four things are established at once:

1. **The lab Mesa is genuinely the ICD in use** - proven from inside the guest,
   not inferred from the image closure.
2. **The whole stack already works for ordinary Vulkan.** The guest sees
   `Virtio-GPU Venus (NVIDIA T1000)`: guest Vulkan → Venus → virtio-gpu →
   crosvm → lab virglrenderer → host NVIDIA driver. Only video is missing.
3. **The gap is exactly where predicted.** All three video extensions are
   absent, while `QUEUE_VIDEO_DECODE_BIT_KHR` *is* present - precisely matching
   the research finding that Venus does not filter queue flags but never
   advertises the extensions, so the bit is unusable. This is the single fact
   the whole prototype exists to change.
4. **No `/dev/video*` exists in the guest.** virtio-media is not in this path at
   all, so any decode observed later cannot be V4L2 in disguise.

## 5e. Guest decode baseline - ✅ CAPTURED, and a false pass caught

```
ffmpeg_hwaccels        = vdpau cuda vaapi drm opencl vulkan amf
ffmpeg_vulkan_exit     = 0
ffmpeg_vulkan_init_lines     = 0
ffmpeg_vulkan_fallback_lines = 1
ffmpeg_vulkan_decode   = DID_NOT_USE_VULKAN
dev_video_opened       = (none)
```

ffmpeg verbose confirms the mechanism:

```
Selecting decoder 'h264' because of requested hwaccel method vulkan
[Vulkan] Supported layers: VK_LAYER_MESA_device_select …
```

…and then no hwaccel initialisation, one fallback line, software decode.

> **The exit code is a false pass, measured.** The first version of this check
> ran `ffmpeg -hwaccel vulkan … -f null -` and reported `SUCCESS` purely on the
> zero exit status. ffmpeg exits **0** while silently falling back to software
> when hwaccel init fails. The check now attributes the outcome from the verbose
> log instead, and correctly reports `DID_NOT_USE_VULKAN`. This is the third
> false-pass found in this lab's own harness, after the `strings` extension
> check and the `pipefail`/`grep -q` inversion.

**This is the strongest form of the W0 Firefox baseline.** Firefox's Vulkan
Video path runs through the *system* `libavcodec`. If ffmpeg's own Vulkan
hwaccel cannot initialise on this guest, Firefox's cannot either - it is the
same library, the same Vulkan device, and the same missing extensions. The
decode baseline therefore establishes the Firefox baseline by construction,
without depending on browser logging.

`ffmpeg -hwaccels` listing `vulkan` is worth noting as its own trap: the build
supports Vulkan, so a capability check that stops at "is vulkan in the list"
would also report a false pass.

### MOZ_LOG capture: fixed, with a caveat for W6

The first attempt produced an empty log. Two causes, both now fixed:

1. `--headless --screenshot` exits almost immediately, often before the media
   stack has selected a decoder at all. Firefox now runs the page for 25s.
2. Firefox writes **one MOZ_LOG file per process**, and media decoding happens
   in a child. Picking the first match found an empty child log and reported
   "0 lines" while the real content sat in a sibling file. All files are now
   concatenated.

Capture now yields **3 log files, 898 lines** (was 0), with `gfxVars` lines
present.

**Caveat carried into W6:** decoder-selection and WebRender counters are still
zero, because **headless Firefox does not exercise the WebRender/compositor
path**. That is itself the finding: headless is the wrong harness for the
rendering gate. W6 must drive the real `cage` session, not `--headless`, since
Firefox refuses hardware decode unless it is already GPU-rendering.

## 5f. Host-native decode - ✅ **THE CONTROLLED EXPERIMENT**

```bash
bash tests/host-decode.sh
```

```
exit_status              = 0      <-- NOT sufficient evidence on its own
vulkan_init_lines        = 1
vulkan_fallback_lines    = 0
decoder_output_pix_fmt   = vulkan
RESULT: host DID decode H.264 through Vulkan Video (pix_fmt=vulkan)
```

90 frames decoded at 21.5x realtime. The decisive line is
`Reinit context to 1280x720, pix_fmt: vulkan` - the decoder's **output pixel
format is `vulkan`**, so frames are genuinely Vulkan-backed rather than decoded
in software and merely requested via `-hwaccel`.

**Host and guest run the identical ffmpeg, the identical clip, and the identical
command line. The only difference is Venus:**

| | host | guest |
|---|---|---|
| `exit_status` | 0 | 0 |
| `vulkan_init_lines` | 1 | 0 |
| `vulkan_fallback_lines` | 0 | 1 |
| `decoder_output_pix_fmt` | **`vulkan`** | **`yuv420p`** |
| verdict | **USED VULKAN** | **DID NOT USE VULKAN** |

That table is the whole prototype in miniature. It isolates the gap to the Venus
stack with a controlled experiment rather than an argument, and it removes the
ambiguity that would otherwise plague W5: if guest decode fails after the
protocol work, it cannot be blamed on a broken host path, because the host path
is measured working here.

**Both sides now run the identical pinned ffmpeg** (`8.1.2`, `/nix/store/wfdhrw2jxj59gllsf1kh70g7a1n6zyvs-ffmpeg-8.1.2-bin`), enforced by
the `host-decode` flake app: the script refuses to run against an ambient or
registry-resolved ffmpeg, because comparing two different builds would
invalidate the control.

`pix_fmt` was promoted to the primary attribution signal after this run, because
it is far more robust than matching log phrasing, which varies across ffmpeg
versions. Note again that `exit_status = 0` on **both** sides - the exit code
carries no information at all.

## 6. Host device access (user `paydro`)

| Device | Access |
|---|---|
| `/dev/dri/renderD128` | ✅ rw (0666) |
| `/dev/nvidia0`, `/dev/nvidiactl`, `/dev/nvidia-modeset`, `/dev/nvidia-uvm`, `/dev/nvidia-uvm-tools` | ✅ rw (0666) |
| `/dev/udmabuf` | ✅ rw via ACL `user:paydro:rw` |
| **`/dev/kvm`** | ❌ **denied** - 0660 `root:kvm`; ACL lists only `d2b-*` role users |

`/dev/kvm` is the only gap. See AE-1 in the plan and `host/grant-kvm.sh`.

## 7. Firefox gate audit

`MOZ_ENABLE_VULKAN_VIDEO` was the plan's highest-severity open risk. It is
**cleared**: `toolkit/moz.configure` sets it unconditionally for GTK builds, and
nixpkgs builds Firefox with `cairo-gtk3-wayland`.

| Gate | Status |
|---|---|
| `MOZ_ENABLE_VULKAN_VIDEO` | ✅ unconditional on GTK - no build-flag change needed |
| `LIBAVCODEC_VERSION_MAJOR >= 60` | ✅ 62 (ffmpeg 8.1.2) |
| ffmpeg `--enable-vulkan` | ✅ nixpkgs default on Linux |
| Hardware WebRender | ⏳ to verify in-guest |
| `gfxVars::UseH264HwDecode()` | ⏳ to verify in-guest |

## Status

| W0 item | Status |
|---|---|
| Fork repos created and seeded with `base/<rev>` tags | ✅ all three |
| Lab scaffolding + isolation contract | ✅ |
| `PINS.md` evidence manifest | ✅ |
| Host capability report | ✅ **PASS** |
| In-sandbox capability report | ✅ **PASS** |
| Upstream baseline claims verified against real source | ✅ |
| Lab flake with host/guest package split, locked | ✅ |
| `labVirglrenderer` builds from fork | ✅ |
| `labCrosvm` builds, relinked | ✅ |
| crosvm ↔ lab virglrenderer binding **proven** | ✅ **PASS** |
| State-outside-repo isolation holds | ✅ 92K in-repo, 1.2G outside |
| `labMesa` builds from fork (26.1.5) | ✅ |
| Guest Mesa ICD binding **proven** | ✅ **PASS** |
| Guest video baseline confirmed (absent + NV12 stripped) | ✅ |
| Stock Firefox 153 packaged, prefs-only, no source patch | ✅ |
| Guest image builds; closure contains lab Mesa | ✅ |
| `run-lab-vm.sh` written + shellchecked | ✅ |
| Root gates green with `labs/` present | ✅ `test-lint`, `policy_cli_consumers`, `policy_source` |
| Launcher executed end-to-end (VM boots to graphical target) | ✅ **PASS** |
| Guest `vulkaninfo` baseline from inside the booted VM | ✅ **CAPTURED** |
| Guest ICD proven to be lab Mesa, from inside the guest | ✅ **PASS** |
| Guest sees Venus device backed by host T1000 | ✅ |
| No `/dev/video*` in guest (virtio-media absent) | ✅ |
| Host-native Vulkan decode (`pix_fmt=vulkan`) | ✅ **PASS** |
| Guest decode baseline (`pix_fmt=yuv420p`, no Vulkan) | ✅ **CAPTURED** |
| /etc/d2b masked from sidecar (negative proof) | ✅ **PASS** |
| KVM revoke covers pre-existing grants | ✅ **PASS** |
| Firefox MOZ_LOG capture (3 files, 898 lines) | ✅ **FIXED** |
| Firefox WebRender gate evidence | ⚠️ headless does not exercise it; W6 must use the cage session |

**Decision-rule checkpoint: no blocker found.** The host driver exposes
everything required, and the gap is exactly where the research predicted - the
Venus stack.

## Artifacts

| Artifact | Store path |
|---|---|
| lab virglrenderer | `virglrenderer-venus-vulkan-video-lab-9ae1fb1` |
| lab crosvm | `crosvm-venus-vulkan-video-0-unstable-2026-07-15` |
| lab Mesa (guest Venus ICD) | `mesa-venus-vulkan-video-lab-bcf312f` |
| guest disk image | `nixos-disk-image` (16 GiB raw) |

## Reproducing

```bash
# Resolve the lab through git+file, never a bare path. A `path:`/`.#` ref makes
# Nix copy the ENTIRE working tree into the store (AGENTS.md rule 4).
LAB_FLAKE="git+file://$(git rev-parse --show-toplevel)?dir=labs/venus-vulkan-video"

# Capability probes (no VM required)
bash tests/host-caps.sh --host
bash tests/host-caps.sh --in-sandbox

# Host-native decode control (pinned ffmpeg; the guest comparison depends on it)
nix run "$LAB_FLAKE#host-decode"

# Binding proofs
nix run "$LAB_FLAKE#prove-crosvm-binding"
nix run "$LAB_FLAKE#prove-guest-icd"

# Build artifacts
nix build "$LAB_FLAKE#labVirglrenderer" "$LAB_FLAKE#labCrosvm" \
          "$LAB_FLAKE#labMesa" "$LAB_FLAKE#guestImage"

# Boot the VM (owns the /dev/kvm grant lifecycle; tears everything down on exit)
nix run "$LAB_FLAKE#lab-vm"
```

## Correction: the queue bit was a boundary leak, not just a capability gap

The W0 evidence above records, in the guest, with all three video extensions
absent:

```
video_decode_queue_bit    = PRESENT
```

It is written up as the reason FFmpeg falls back: the queue exists, the
extensions do not. That reading is correct and incomplete.

`VK_QUEUE_VIDEO_DECODE_BIT_KHR` lives in the **base**
`VkQueueFamilyProperties.queueFlags`, not in `VkQueueFamilyVideoPropertiesKHR`.
Both `vkGetPhysicalDeviceQueueFamilyProperties` and `...Properties2` are
dispatched by the renderer today and forward the host's flags unmodified. So
this line is also a record of **host video capability crossing to the guest
through a path that predates any video work in this lab.**

It was identified during W2 planning (door 7 of eight). The measurement was
right the first time; the interpretation stopped at "why doesn't FFmpeg work"
and did not ask "what is the guest being told that it should not be".

W2 covers it under the derived value surface: `queue-flag` is one of two
`outbound` buckets, alongside `format-feature`, and outbound values must be
scrubbed from guest-visible replies rather than forwarded.
