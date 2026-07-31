# The green frame: root cause, fix, and what is still broken

## Fixed: the green frame

**Root cause, end to end.** Mesa's Venus deliberately zeroes
`VkPhysicalDeviceDrmPropertiesEXT` when the host GPU is NVIDIA
(`vn_wsi.c`, `is_nvidia && !is_gamescope`). It is a real workaround for a real
WSI problem - virgl exposes no explicit modifiers, wlroots compositors
advertise LINEAR, and NVIDIA cannot do LINEAR colour attachments - so the spoof
makes the WSI same-GPU check fail and forces the prime-blit path.

But the zeroed node is visible to **every** consumer of
`VK_EXT_physical_device_drm`, and Firefox's Vulkan video decoder uses exactly
that node for a completely different decision:

```
Venus reports render node 0,0; the guest node is 226,128
  -> Firefox logs "matches renderer: false"
  -> skips its own NVIDIA workaround that substitutes
     DRM_FORMAT_MOD_NVIDIA_BLOCK_LINEAR_2D for LINEAR
  -> LINEAR is the only modifier
  -> drmModsAreLinearOrEmpty == true, so direct export is impossible
     REGARDLESS of the pref
  -> copy path forced
  -> CopyYUVDataImpl calls GL BlitTextureToTexture between two DMA-BUF textures
  -> fails on virgl with GL_INVALID_OPERATION
  -> virglrenderer marks the context "Illegal command buffer"
  -> every later submission to it is refused
  -> video plays ~0.5s, then a green frame forever
```

One GL blit failed and produced 11,270 downstream refusals.

**Fix, without patching Firefox.** Two coupled changes:

1. `VN_DEBUG=no_nvidia_drm_spoof` - a new opt-out in our Mesa fork that reports
   the guest's real virtio-gpu DRM node. Safe here specifically because this
   guest composites through GL, so the WSI path the spoof protects is not in
   use.
2. `direct-export.enabled = true` - takes `MoveYUVDataImpl`, which performs no
   blit at all.

Neither works alone: direct export needs a non-LINEAR modifier, and Firefox
only selects one when it believes the decoder and compositor are the same GPU.

**Verified:**

| Signal | Before | After |
|---|---:|---:|
| `matches renderer` | false | **true** |
| Venus DRM render node | 0,0 | **226,128** |
| `failed to dispatch BLIT` | 1 | **0** |
| `CmdSubmit3d` refusals | 11,270 | **0** |
| Picture | flat dark green | **correct test pattern** |

Screenshot: `evidence/firefox-fixed.png`.

## Still broken: Firefox is not using the hardware decoder

The picture is correct because **software decode works**, not because the fix
completed the job.

| Workload | Host NVDEC utilisation |
|---|---|
| host-native `ffmpeg -hwaccel vulkan` | **99%** |
| guest `ffmpeg -hwaccel vulkan` via Venus | **98%** |
| **Firefox playing the same clip** | **0%, all 36 samples** |

Firefox's own log confirms it: `Create a video data from a shmem image`, planar
YCbCr - the software path - and only 3 `vkCmdDecodeVideoKHR` commands reached
the renderer for 600 frames.

**So Venus Vulkan Video genuinely drives the host NVDEC engine.** That is now
proven, not inferred: the guest ffmpeg number is within a point of the
host-native number. The remaining gap is specific to Firefox.

### The lead

Firefox runs **two** RDD processes, and only the first touches Vulkan:

```
[RDD 810 ]: Initialising Vulkan FFmpeg decoder
[RDD 810 ]: Selected Vulkan device ... matches renderer: true
[RDD 1787]: PDMInitializer, Init PDMs in RDD process
[RDD 1787]: PDM order: 0: FFmpeg(OS library)  1: FFmpeg(FFVPX)  2: Agnostic
[RDD 1787]: Using preferred SOFTWARE codec h264
```

RDD 810 initialises Vulkan successfully and then plays no further part. RDD
1787 - a different process - initialises the decoder modules from scratch and
selects software. Whether 810 crashed, was replaced, or the Vulkan state simply
does not survive into 1787 is the next thing to establish.

Ruled out so far: the guest carries two libavcodec majors (61 from ffmpeg
7.1.5, 62 from the pinned 8.1.2) and Firefox probes both, but **both contain
the Vulkan H.264 decode symbols**, so the version split is not the cause.

## A measurement that was nearly wrong

The first NVDEC sample of the guest read 0% and looked like proof that Venus
never reaches the hardware. It was an artifact: the command generated a 60s
clip first, and decode finished inside the window I spent waiting.

The control that caught it was running the same probe against **host-native**
Vulkan Video, which showed 99% - proving the instrument reports Vulkan Video at
all. Re-running with sampling started *before* decode showed 98% from the
guest.

Worth recording because the false version was the more dramatic finding, and it
would have been reported as a fundamental architectural blocker.

## Update: hardware decode reached, and the blit failure is now named

Two results supersede parts of the account above.

### Firefox now decodes on the host NVDEC engine

Measured with the sampler started before playback:

| Workload | Host NVDEC |
|---|---:|
| Firefox, before | 0% across every sample |
| **Firefox, now** | **nonzero in 27 of 35 samples, mean 3.3%, max 5%** |

with `decode_cmds=1024 sessions=1` in the renderer. 3.3% is the expected
order for 720p30 in real time on a T1000, and the contrast with the earlier
flat zeros confirms the instrument discriminates rather than merely reading
low.

So the decode half of the prototype works: unmodified Firefox, H.264, decoded
by the host GPU through Venus. What remains is presentation.

### The blit failure, with parameters

The earlier account said only that a GL blit failed with `GL_INVALID_OPERATION`.
`VREND_DEBUG=blit` could never have said more, because `VREND_DEBUG_ENABLED` is
`false` whenever `NDEBUG` is defined, which is every build this lab runs. With
unconditional diagnostics added to the fork:

```
BLIT failed with GL error 0x502 via glCopyImageSubData:
  src fmt PIPE_FORMAT_R8_UNORM   (view PIPE_FORMAT_R8G8_UNORM) egl:1 box 640x360
  dst fmt PIPE_FORMAT_R8G8_UNORM (view PIPE_FORMAT_R8G8_UNORM) egl:0 box 640x360
```

640x360 is the chroma plane of a 1280x720 NV12 frame, so this is Firefox's
per-plane copy of the UV plane. The destination is `R8G8_UNORM`, which is
correct for NV12 chroma. The source resource is `R8_UNORM`, which is not.

`glCopyImageSubData` requires matching texel block size and ignores texture
views: `R8_UNORM` is the 8-bit class and `R8G8_UNORM` the 16-bit class, so the
call is illegal and raises `GL_INVALID_OPERATION`. The fast path was selected
because `vrend_renderer_blit` tests the *view* formats

```c
format_is_copy_compatible(info->src.format, info->dst.format, comp_flags)
```

which are both `R8G8_UNORM` and therefore trivially equal, while
`vrend_renderer_copy_region` tests the *resource* formats a few thousand lines
earlier. Note also that falling through to `vrend_renderer_blit_int` would not
help by itself: a texture view cannot reinterpret `R8` as `R8G8` either, for the
same class reason.

`format_is_copy_compatible` already carries a carve-out for exactly this
disease -- an EGL-imported resource whose real internal format differs from the
format the caller blits with (`B8G8R8X8_UNORM` imported as `GL_RGB8`). This is
the same bug with a different format pair.

### The open question

Why is the source chroma resource `R8_UNORM` at all? The destination is
`R8G8_UNORM`, the dimensions are the chroma plane's, and the source is
EGL-image backed (`egl:1`), so the suspicion is that the NV12 second plane is
imported per-plane as a single-channel `R8` rather than as `R8G8`. That is a
resource-import question, not a blit question, and it is where the next work
goes. Guarding the fast path on resource formats would convert the crash into a
different failure, not into a correct frame, so it is not on its own a fix.

## Correction: the RG88 mapping was a real defect but not this bug

`PIPE_FORMAT_RG88_UNORM` is absent from virgl's `pipe_to_virgl_format` table, so
it resolves to `VIRGL_FORMAT_NONE` and the omission is silent in any build where
`debug_printf` goes nowhere. That is a genuine defect and the mapping was added.

It did not fix this. After the change the diagnostic is byte-for-byte identical:

```
src fmt PIPE_FORMAT_R8_UNORM (view PIPE_FORMAT_R8G8_UNORM) egl:1 box 640x360
dst fmt PIPE_FORMAT_R8G8_UNORM (view PIPE_FORMAT_R8G8_UNORM) egl:0 box 640x360
```

The causal link from "GR88 has no virgl mapping" to "the chroma resource is R8"
was inferred rather than measured, and the inference was wrong. Two things did
change: the failure count went from 1 to 585, roughly one per frame at 30fps,
which means the blit now fails per frame instead of failing once and poisoning
the context; and hardware decode continued throughout.

## Prior art: the same convention mismatch, one layer over

`vicondoa/virgl-vaapi-compat` records an earlier investigation into the VA-API
path. There, virgl exported decoded 4:2:0 surfaces as `DRM_PRIME_2`
`VA_FOURCC_I420` with three separate single-channel planes, while the client
expected `VA_FOURCC_YV12` ordering and rejected the descriptor outright. The fix
was a libva shim rewriting only the descriptor metadata, chosen specifically so
the client stayed unpatched.

That shim does not apply here: this path is Vulkan Video through Venus, libva is
not loaded, and there is no `vaExportSurfaceHandle` to hook. What carries over is
the shape of the bug. virgl has already been observed describing 4:2:0 chroma
with a different plane convention than Firefox expects, and `R8` is what a
single-channel I420-style U or V plane looks like, against the two-byte
interleaved `R8G8` an NV12 chroma plane requires.

The per-frame failure count argues against a straight three-plane I420 source,
which should fail twice per frame rather than once. So the working hypothesis is
one chroma plane declared single-channel, not a full I420/NV12 split. That is a
distinction to measure.

### Next step

Instrument where the plane descriptor is constructed and where the EGL image is
imported, logging the plane count, the DRM fourcc per plane, and the format each
resolves to. The blit is the symptom; the plane metadata is where the earlier
VA-API investigation found its answer, and reasoning about the blit has now
produced one wrong conclusion already.

## Resolved: the flattening is in guest Mesa, measured from both ends

Both ends of the path were finally instrumented, and they disagree.

Firefox, from its own `Dmabuf` log module (which the guest was already
configured to capture, and which went unread for most of this investigation):

```
plane 0 size 1280 x 720 format 20203852   = 'R8  '
plane 1 size  640 x 360 format 38385247   = 'GR88'
DMABufSurfaceYUV::UpdateYUVData() copy 1
DMABufSurfaceYUV::ImportPRIMESurfaceDescriptor() FOURCC 3231564e = 'NV12'
DMABufSurfaceYUV::CreateYUVPlaneGBM() size 640 x 360 plane 1
```

virglrenderer, from the import and blit traces:

```
dmabuf import: res_id=412 virgl_fmt=PIPE_FORMAT_R8_UNORM -> drm_fourcc=R8 1280x720
BLIT FAILED src h:412 res 1280x720 R8 (view R8G8) GL_RED -> dst res 640x360 R8G8 GL_RG8
```

Firefox asks for a `GR88` plane at 640x360. virglrenderer is handed `R8` at
1280x720 and never sees a `GR88` request. The only layer between them is guest
Mesa, so that is where the plane description is flattened.

### What this rules out

- **Firefox.** It describes both planes correctly and allocates a matching
  destination. There is nothing to hack here even if hacking it were allowed.
- **virglrenderer under-advertising.** `VIRGL_FORMAT_R8G8_UNORM` is present in
  `rg_base_formats` as `GL_RG8` with `view_class_16`, so Mesa is not being
  pushed into a fallback by the host refusing the format.
- **The two format-mapping gaps found earlier.** Adding
  `PIPE_FORMAT_RG88_UNORM` to virgl's conversion table and `GBM_FORMAT_GR88` to
  the GBM table were both real defects and both are kept, but neither is on this
  path: nothing ever reaches them, because the plane is already flattened to
  `R8` before either would be consulted.

### Note on where the fix goes

A virglrenderer-side compat mapping remains possible in principle -- the chroma
bytes are all present, since a 640x360 `R8G8` region is exactly a 1280x360 `R8`
region -- but it would be compensating downstream for a description that is
wrong upstream, and it cannot use `glCopyImageSubData` because that ignores
views and `R8` to `R8G8` crosses texel-size classes. Correcting the description
in Mesa is the smaller and more honest change.

### Method note

Four successive models of this copy were wrong, and each wrong model came from
inferring the path from source reading rather than measuring it. The answer came
from logging both ends and comparing, and part of the evidence had been sitting
in the guest's own log file, uncollected, the entire time.

## The guest asks correctly, and EGL accepts: the flattening is inside Mesa

Firefox's own EGL import attributes, from the guest `Dmabuf` log:

```
Plane 0: fd=108 pitch=1280 modifier=INVALID format=0x20203852 (R8)   size=1280x720
  Plane 0: zero-copy EGLImageTargetTexture2D succeeded
Plane 1: fd=129 pitch=1280 modifier=INVALID format=0x38385247 (GR88) size=640x360
  Plane 1: zero-copy EGLImageTargetTexture2D succeeded
```

Every field is right. `GR88` is the correct fourcc for interleaved NV12 chroma,
640x360 is the correct chroma geometry, and pitch 1280 is correct for 640 texels
at two bytes each. The two planes are separate buffers (fd 108 and fd 129), and
both imports report success.

virglrenderer, for the same resources, records `R8 1280x720`.

So the request is well formed, EGL accepts it, and what arrives at the host is a
single-channel texture with luma geometry. Nothing between those two points
except guest Mesa's EGL/gallium layer. Because the request is valid and
accepted rather than rejected, this is a defect rather than a configuration
gap: a driver that could not honour `GR88` should fail the import, not silently
substitute a different format and size.

This also finally explains why the blit sees what it sees. The failing blit's
source handles are the same resource ids the import trace recorded as
`R8 1280x720`, so the chroma copy is reading a luma-shaped single-channel
texture, which is exactly the `R8` to `R8G8` mismatch `glCopyImageSubData`
rejects.

### Search for prior art

Public searching produced no report of this specific failure. The upstream
virglrenderer and Mesa trackers sit behind anti-scraping and could not be
queried. What the search did confirm is that the path itself is standard and
long-established: the `R8`, `RG88` and `GR88` DRM fourccs were added precisely
so NV12 can be imported as "the Y plane as an R8 EGLImage and the UV plane as
either an RG88 or GR88 EGLImage", and Mesa's own developer list states that
"drivers usually handle NV12 as 2 separate textures R8 and R8G8". Firefox is
doing the normal thing; the virgl path is not honouring it.

## Why Mesa does this, and a workaround needing no code change

Mesa is not misbehaving, and the `GR88` question is a red herring.

`virgl_resource_from_handle()` imports an **existing** dmabuf. It does not
create a host resource; it looks up the one already allocated and copies the
caller's template. So the guest-side `GR88 640x360` view is a reinterpretation
of a buffer whose host-side resource keeps whatever format it was originally
created with. Both ends are behaving correctly and simply describe the same
memory differently. That difference only becomes fatal because the frame is
being pushed through a per-plane GPU copy, and `glCopyImageSubData` compares the
underlying textures rather than the views.

The real question is why the copy path is running at all. It is a configuration
chain, and every link is observable:

```c
// gfxPlatformGtk::InitPlatformHardwareVideoConfig
FeatureState& featureDec = gfxConfig::GetFeature(Feature::HARDWARE_VIDEO_DECODING);
if (!featureDec.IsEnabled()) { return; }        // early return
...
if (featureZeroCopy.IsEnabled()) gfxVars::SetHwDecodedVideoZeroCopy(true);
```

```c
// VideoFramePool::ShouldCopySurface
if (!gfx::gfxVars::HwDecodedVideoZeroCopy()) { return true; }   // always copy
return freeRatio < SURFACE_COPY_THRESHOLD;                      // 1.0/4.0
```

`HARDWARE_VIDEO_DECODING` is `unavailable` on this guest, runtime force-disabled
by the VA-API probe that cannot succeed here. That makes
`InitPlatformHardwareVideoConfig()` return before zero-copy is ever configured,
so `HwDecodedVideoZeroCopy()` stays false, so `ShouldCopySurface()`
unconditionally returns true, so every frame takes `CopyYUVDataImpl` and its
per-plane blit. The recorded pool state is `free ratio 1`, and `1.0 < 0.25` is
false, so with zero-copy enabled that copy would not happen at all.

### The workaround

Make the gfxInfo hardware-video-decoding probe succeed, so the feature is
enabled and zero-copy is configured. Either backend satisfies it:

- a working VA-API in the guest, which the virtio-gpu VA driver already
  provides on the sibling `graphics.virglVideo` path, or
- V4L2 through `virtio_media`.

Both are guest packaging and configuration. Neither is a source change to
Firefox, Mesa or virglrenderer.

Decode selection is unaffected: `InitHWDecoderIfAllowed()` tries
`InitVulkanDecoder()` before `InitVAAPIDecoder()`, so making VA-API merely
*present* does not displace the Vulkan path being tested here. Once the feature
is enabled, `media.ffmpeg.vaapi.force-surface-zero-copy = 1` can pin zero-copy
on rather than leaving it to the blocklist.

This is untested and is written down as a candidate, not a result. The
zero-copy path may meet the modifier and export constraints already documented
above, and if it does the answer will be a different one. But it costs a
configuration change to find out, where every alternative costs a patch.

## Root-cause candidate: the chroma plane is never typed

`virgl_resource_from_handle()` in the guest is the only place an imported
dmabuf gets its type communicated to the host:

```c
/* assign blob resource a type in case it was created untyped */
if (res->blob_mem && plane == 0 &&
    (host_feature_check_version >= 18 || VIRGL_CAP_V2_UNTYPED_RESOURCE)) {
   vs->vws->resource_set_type(vs->vws, res->hw_res,
                              pipe_to_virgl_format(res->b.format),
                              ..., res->b.width0, res->b.height0, ...);
}
```

An imported dmabuf arrives untyped -- `virgl_drm_winsys_resource_create_handle`
sets `res->maybe_untyped = info_arg.blob_mem ? true : false` -- so without this
call the host resource keeps whatever type it already had. The call is gated on
`res->blob_mem` and on `plane == 0`, and it is the guest, not the host, that
decides the format and geometry the host will record.

This fits the measurements. Exactly eight typed imports were traced for eight
surfaces, not sixteen for sixteen planes, and every one carried plane 0's
`R8 1280x720`. No `GR88` request ever reached the host, which is why adding
`GBM_FORMAT_GR88` to virglrenderer's table changed nothing: the code that would
have consulted it never ran.

It also explains why the failure follows the surface rather than the consumer.
The copy path and the compositor both fail on the same badly typed resource;
switching between them with the zero-copy pref moved the `GL_INVALID_OPERATION`
from `MediaPD~oder` context 10 to `Renderer` context 8 without fixing anything.

### What is not yet established

Which of the two gates fails for the chroma plane. Firefox imports the two
planes as separate single-plane images with separate fds (108 and 129), so
`plane` should be 0 for both, which points at `res->blob_mem` -- but that is
inference, and inference at this exact spot has produced two wrong fixes
already.

The decisive measurement is a log in `virgl_resource_from_handle` recording
`res->b.format`, `width0`, `height0`, `blob_mem`, `plane`, and whether the
`resource_set_type` block is entered, for both planes. That distinguishes
"never typed because not a blob" from "typed with the wrong values", which need
different fixes, and it is one guest build rather than another guess.

## Upstream status, and the mechanism that fits the count

**There is no newer Mesa to pick up.** The fork is 26.1.5, and the only
upstream commit touching `src/gallium/drivers/virgl/` that is not already in
our tree is unrelated (`kraid: Implement Foldable`). The relevant history --
`virgl: add support for VIRGL_CAP_V2_UNTYPED_RESOURCE`, `virgl: Allow importing
resources without known templ`, `virgl: Support new resource-layout command` --
is all present. Public searching found no report of this failure mode.

The upstream design intent is documented in the virglrenderer commit that added
untyped resources:

> An untyped resource is a virgl_resource without pipe_resource while
> vrend_context works with pipe_resources exclusively. When an untyped resource
> is attached, we defer the insertion into res_hash until
> VIRGL_CCMD_PIPE_RESOURCE_SET_TYPE is submitted.

That is load bearing for reading our data: an untyped resource is not findable
at all until it is typed. Our failing blit resolves `src h:412` without
complaint, so these resources are not untyped. They are typed, and typed wrong.

### The mechanism that explains the count

`virgl_drm_winsys_resource_create_handle` caches by GEM handle:

```c
res = util_hash_table_get(qdws->bo_handles, (void*)(uintptr_t)handle);
if (res) { ...; goto done; }
```

If `PRIME_FD_TO_HANDLE` resolves both plane fds to the same GEM handle -- which
happens when both planes are regions of one buffer -- the second import returns
the cached resource rather than creating one. That resource was already typed
by the first import, and `virgl_resource_from_handle` types from plane 0:
`R8`, `1280x720`. The chroma plane never gets typed as `GR88 640x360` because,
as far as virgl is concerned, there is only one resource and it already has a
type.

This predicts the measured count exactly: eight typed imports for eight
surfaces, not sixteen for sixteen planes. It also explains why Firefox's own
log looks correct -- it really does request `GR88 640x360` per plane, and EGL
really does accept it -- while the host only ever sees `R8 1280x720`. The guest
view is per plane; the underlying virgl resource is shared and single-typed.

### Confidence and the confirming measurement

This is the first mechanism that accounts for the plane count, the wrong
format, the wrong geometry, Firefox's correct-looking log, and the absence of
any `GR88` at the host, without needing any of them to be mistaken. It is not
yet proven.

The confirming measurement is small: log the GEM handle returned by
`PRIME_FD_TO_HANDLE` for each plane import, and whether the `bo_handles` lookup
hit. Identical handles with a cache hit on plane 1 confirms it. Distinct
handles refutes it and sends the search back to why a separate chroma resource
would be typed with luma values.

## ROOT CAUSE, confirmed by instrumented measurement

Both NV12 planes live in **one buffer**, and the chroma plane's import hits the
winsys BO cache, which loses the flag that decides whether the resource gets
typed. Measured in the guest, 1216 times each -- once per frame:

```
fd -> gem_handle=N cache=miss plane=0 stride=1280 offset=0      fmt=81
  from_handle: pipe_fmt=81 1280x720 plane=0 blob_mem=2 cap_ok=1 -> WILL TYPE (virgl_fmt=64)

fd -> gem_handle=N cache=HIT  plane=0 stride=1280 offset=983040 fmt=82
  from_handle: pipe_fmt=82  640x360 plane=0 blob_mem=0 cap_ok=1 -> NOT TYPED (virgl_fmt=65)
```

Reading it line by line:

- Both planes resolve to the **same GEM handle**. They are one allocation, with
  luma at offset 0 and chroma at offset 983040 (1280x768, so the luma plane is
  padded to a 768-row alignment).
- The luma import is a cache **miss**, carries `blob_mem=2`, and types the host
  resource as `R8 1280x720`.
- The chroma import is a cache **HIT**, and arrives at the typing gate with
  `blob_mem=0`, so `res->blob_mem && plane == 0` fails and the resource is
  **never typed**. The host keeps luma's type.
- `pipe_fmt=82` is `PIPE_FORMAT_RG88_UNORM` and it maps to `virgl_fmt=65`,
  `VIRGL_FORMAT_R8G8_UNORM`. The guest resolves the chroma format perfectly
  correctly. It simply never sends it.

### The proximate defect

`virgl_drm_winsys_resource_create_handle()` does not propagate `*blob_mem` on
the cache-hit path:

```c
if (res) {
   int32_t ref = p_atomic_inc_return(&res->reference.count);
   if (ref == 1) res->needed_references++;
   goto done;                       /* *blob_mem never assigned */
}
...
res->blob_mem = info_arg.blob_mem;
*blob_mem = info_arg.blob_mem;      /* only on the non-cached path */
```

The caller's `res->blob_mem` therefore stays 0 from its `CALLOC`, and every
import that shares a BO with an earlier one silently declines to type itself.

### Why this settles the earlier confusion

It explains, without needing any observation to be wrong:

- Eight typed imports for eight surfaces rather than sixteen for sixteen planes.
- The host only ever seeing `R8 1280x720`: luma's type, applied to a resource
  the chroma plane also uses.
- Firefox's log being entirely correct. It really does request `GR88 640x360`
  per plane and EGL really does accept it; the loss is below EGL.
- The two format-mapping fixes changing nothing. `virgl_fmt=65` proves the
  guest mapping works. The code that would have used it never runs, and the
  host-side `GBM_FORMAT_GR88` entry is equally unreachable. Both are still
  correct and are kept.
- The failure following the surface rather than the consumer. One badly typed
  resource breaks the copy path and the compositor alike, which is why the
  zero-copy pref moved `GL_INVALID_OPERATION` from context 10 to context 8.

### What a fix has to reckon with

Propagating `*blob_mem` on the cache-hit path is the obvious repair and is
almost certainly correct on its own terms, but it is not obviously sufficient
here. Both planes share one `hw_res`, and `resource_set_type` types the
underlying host resource, so typing the chroma plane would overwrite luma's
type rather than coexist with it. A single virgl resource carries a single
type, while this buffer needs two plane views.

The model virgl is built for is the multi-plane import: one image, format
`NV12`, walked by the `res->b.next` chain in `virgl_resource_from_handle`,
typed once with per-plane strides and offsets. Firefox instead performs two
independent single-plane imports of the same buffer, which that design does not
express. Making the BO cache key on more than the GEM handle, so each plane
view gets its own resource, is the other candidate and is the larger change.

Both are guest Mesa. Neither is Firefox, and neither is virglrenderer.

## The obvious fix is provably inert, and why this is architectural

Propagating `*blob_mem` on the cache-hit path is the natural repair, and it does
not work. `vrend_renderer_pipe_resource_set_type` treats a second typing of the
same resource as a silent success:

```c
/* either a bad res_id or the resource is already typed */
if (!res) {
   if (vrend_renderer_ctx_res_lookup(ctx, res_id))
      return 0;
   ...
}
```

So the chroma plane's `SET_TYPE` would be accepted and discarded. Luma would
not break, and chroma would not be fixed.

The reason is structural rather than a bug that can be patched in one place.
In virtio-gpu, one dmabuf is one host resource: the guest reaches it by
`PRIME_FD_TO_HANDLE` followed by `RESOURCE_INFO`, and both plane fds of a
shared NV12 buffer therefore resolve to the same `res_handle`. A resource
carries exactly one type. Firefox's per-plane import needs two differently
typed views -- `R8 1280x720` at offset 0 and `R8G8 640x360` at offset 983040 --
of that one resource, and the model cannot express it.

This is also why the failure is invisible on native drivers. There, each
per-plane `EGLImage` becomes its own GL texture straight from the dmabuf at its
own offset, with no shared resource-type indirection in between. virgl's
indirection is what cannot represent it.

### The three ways out, and what each costs

1. **Type the resource as multi-plane `NV12` at first import.** This is the
   model virgl is built for: one resource, `plane_count = 2`, per-plane strides
   and offsets, walked through `res->b.next`. The obstacle is knowledge -- at
   luma-import time the guest sees only `R8`, offset 0, and has not yet been
   told a chroma plane exists. `RESOURCE_INFO` returns `res_handle`, `blob_mem`
   and `size`, not a format.
2. **Give virglrenderer per-plane views of a typed resource.** Correct, and a
   protocol plus renderer change rather than a local patch.
3. **Have the client import NV12 as one multi-plane image.** Firefox is not
   patchable here by the lab's own rule, and this is the client's portable path
   on every other driver.

None is a small change, and the small change that looks right is inert. Per the
lab's decision rule, the blocker is documented rather than papered over.

## The buffer sharing is inherent, confirmed three ways

Three independent routes to "stop the planes sharing one buffer" are all closed:

1. **Two images instead of one.** ffmpeg will do this -- the NV12 entry carries
   a `{R8_UNORM, R8G8_UNORM}` fallback selected when `basics_primary` fails or
   `disable_multiplane` is set, and `basics_primary` comes from the driver's
   own `optimalTilingFeatures`, which Venus controls. But a Vulkan decode
   destination must be a single multi-planar image. `vkCmdDecodeVideoKHR` takes
   one `dstPictureResource`, so a two-image output is not a legal decode
   target. Choosing it would break decode, which currently works.
2. **Disjoint planes.** `VK_IMAGE_CREATE_DISJOINT_BIT` gives each plane its own
   memory and so its own dmabuf. `DISJOINT` does not appear anywhere in
   ffmpeg's `hwcontext_vulkan.c`; ffmpeg has no support for it.
3. **Patch the client.** Out of scope by the lab's own rule, and Firefox is
   doing the portable thing that works on every native driver.

So the decoded frame is one `VkImage` with one `VkDeviceMemory`, exported as one
dmabuf with the planes at offsets. That is not a defect anywhere; it is how
Vulkan video decode output is shaped.

Against that, virtio-gpu maps one dmabuf to one host resource and a resource
carries one type. Firefox needs two typed views of it. The mismatch is
structural on both sides at once, which is why every local repair attempted so
far has been either inert or a relocation of the symptom.

### The only remaining design, and its cost

virgl already has the vocabulary: `whandle->plane`, and a `SET_TYPE` carrying
`plane_count` with per-plane strides and offsets. What is missing is that
Firefox's two independent single-plane imports both arrive as plane 0 of
separate `pipe_resource`s that share one `hw_res`, so nothing ever describes
the buffer as one two-plane resource, and nothing can reference "plane 1 of
resource X" as a sampler or blit source.

Closing that means, at minimum: the guest recognising the cache-hit-with-offset
case and describing the buffer as a multi-plane resource, and virglrenderer
growing per-plane GL textures for a multi-plane typed resource, where
`vrend_resource` holds a single `gl_id` today. That is a protocol and renderer
change across both forks, not a patch.

It is tractable, and both components are ours. It is not a small change, and
saying otherwise after four wrong causal claims in this investigation would not
be credible.

## WORKING: unmodified Firefox, hardware decode, correct picture

Verified on a clean boot:

```
plane image failures : 0
layer-validation err : 0
BLIT failures        : 0
CmdSubmit3d refusals : 0
Illegal cmd buffer   : 0
renderer decode      : decode_cmds=1024 sessions=1
host NVDEC engine    : nonzero 28/35 samples, mean 2.83%, max 6%
guest coredumps      : 0
```

Screenshot shows the H.264 test pattern in full colour: no green cast, no black
frame, no wedged context. Firefox is unmodified; every change is in guest Mesa,
host virglrenderer, or configuration.

### The chain, and the four things that had to be true

A decoded NV12 frame is one `VkImage` with one `VkDeviceMemory`, exported as one
dmabuf with luma at offset 0 and chroma at offset 983040. Firefox imports the
planes as two separate EGL images. Everything below follows from that.

1. **The plane index has to survive the import.** Both plane fds resolve to the
   same GEM handle, so the second import hits the winsys BO cache and looked
   like plane 0 of a fresh buffer. Offsets are now recorded in first-seen order
   and the index of an offset is the plane index.
2. **The second import has to be describable at all.** The winsys dropped
   `*blob_mem` on the cache-hit path, and the caller decides from it whether to
   describe the resource, so every import sharing a buffer silently declined to.
3. **A wider description has to reach the host.** The winsys sent a description
   only while the resource was untyped. A description covering more planes than
   any before it is not a retype; it names part of the buffer nothing has
   described yet.
4. **The host has to build that plane, in a format it accepts.** Two spellings
   of the two-channel plane exist, and this host's EGL advertises `RG88` and not
   `GR88` -- importing as `GR88` failed outright. Both are tried. The plane view
   also had to be resolved before the texture-view branch, which read the plane
   index as a layer range and rejected it.

### Also required, and why

- `VN_DEBUG=no_nvidia_drm_spoof`. Removing it was tested and brought back 1820
  `CmdSubmit3d` refusals, so it is load bearing, not leftover.
- ffmpeg 8 on the library path, for the `semaphoreCount` fix.
- The Venus `vn_GetMemoryFdKHR` null guard.
- `direct-export.enabled = false` and the zero-copy route. The GPU-copy path
  blits from the resource's own texture rather than through a sampler view, so
  it does not benefit from per-plane images; the sampled path does.

### Measurement note

NVDEC reads ~3% because this is 720p30 in real time, not a throughput run. The
signal that matters is the contrast with a software-decoding Firefox, which
reads a flat 0 on every sample.
