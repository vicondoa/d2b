# DMA-BUF export of decode output: what is actually missing

Status: measured. Both columns come from the same probe source
(`tests/probe-dmabuf-export.c`), built against each stack's own loader and
run on the same machine and driver revision.

## The measurement

Format is NV12 (`VK_FORMAT_G8_B8R8_2PLANE_420_UNORM`). Decode usage is
`VIDEO_DECODE_DST | SAMPLED` with an H.264 High progressive profile in the
`pNext` chain. External handle type is `DMA_BUF`.

| Case | Host NVIDIA | Guest Venus |
|---|---|---|
| A. optimal tiling + decode usage + profile, no external | `VK_SUCCESS` | `VK_SUCCESS` |
| B. optimal tiling + decode usage + profile + DMA_BUF | `VK_SUCCESS`, exportable | `VK_ERROR_FORMAT_NOT_SUPPORTED` |
| C. modifier tiling + decode usage + profile + DMA_BUF | `VK_ERROR_FORMAT_NOT_SUPPORTED` | `VK_ERROR_FORMAT_NOT_SUPPORTED` |
| D. modifier tiling + sampled only + DMA_BUF | `VK_SUCCESS`, exportable | `VK_SUCCESS`, exportable |

Both stacks report exactly one DRM format modifier for NV12: `LINEAR`
(`0x0`), with `VIDEO_DECODE_OUTPUT` **absent** from its tiling features.

## What each row establishes

**A** is the sanity check. A decode-output image is creatable on both. Nothing
about video decode itself is missing.

**D** is the isolation control. Modifier-tiled DMA-BUF export works on both
stacks when the usage is ordinary sampling. So neither "modifier tiling" nor
"DMA-BUF export" is broken in general, and Venus forwards that pairing
correctly. Only the combination with video usage fails.

**B is the Venus gap, and it is ours.** The host answers this query and
reports the memory exportable; Venus refuses it. Same format, same usage, same
handle type, same profile. Venus is not forwarding, or not supporting, the
external-memory image-format query when video decode usage bits are present.
This is a defect in the fork and it is fixable here.

**C is not ours.** It fails identically on the host, so Venus is faithfully
reflecting the driver. Together with the modifier list -- one modifier, LINEAR,
without `VIDEO_DECODE_OUTPUT` -- this says the NVIDIA driver does not support
modifier-tiled video decode output on this hardware at all.

## Why fixing B alone probably does not finish the job

ffmpeg only asks for exportable memory under a condition it evaluates before
any of the above (`hwcontext_vulkan.c`, `vulkan_pool_alloc`):

```c
if (p->vkctx.extensions & FF_VK_EXT_EXTERNAL_DMABUF_MEMORY &&
    hwctx->tiling == VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT)
    try_export_flags(..., VK_EXTERNAL_MEMORY_HANDLE_TYPE_DMA_BUF_BIT_EXT);
```

The export request is gated on **modifier tiling**, which is row C -- the row
the host itself rejects. So closing B makes Venus agree with the host, but
ffmpeg still would not request exportable memory for an optimal-tiled decode
frame, and `vulkan_map_to_drm` would still be handed non-exportable memory.

Closing B is therefore necessary for correctness and for parity with the host,
but on its own it is not predicted to make Firefox's direct-export path work.
That prediction should be tested rather than assumed, since it rests on
ffmpeg's tiling choice for decode frames, which has not been measured directly.

## The consequence for the two available paths

Firefox has exactly two ways to get a decoded Vulkan frame onto the screen, and
this lab has now hit a wall in each:

- **Direct export** (`direct-export.enabled = true`): blocked as described
  above, and currently crashes the RDD process in `vn_GetMemoryFdKHR` because
  the memory was never allocated exportable.
- **GPU copy** (`direct-export.enabled = false`): produced the green frame. The
  cause was recorded separately: `CopyYUVDataImpl` performs a GL
  `BlitTextureToTexture` that fails on virgl with `GL_INVALID_OPERATION` and
  wedges the rendering context.

The copy path's blocker is in **virglrenderer**, which this lab also forks, and
it does not require DMA-BUF export of decode output at all. On the present
evidence it is the shorter route to a working prototype, and it does not depend
on a driver capability the host has been measured not to have.

## Do not conclude from this that Venus video decode is broken

Decode works. That was measured separately: guest ffmpeg through Venus drove
the host NVDEC engine at 98% against 99% host-native. What is missing is the
handoff of the decoded frame to a consumer, not the decode.

## Update: direct export is structurally unavailable to unmodified Firefox here

Two further measurements close this off.

**Firefox ties direct export to modifier tiling.** It only switches the frames
pool to `VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT` when the modifier list is not
linear-only, and it only calls `av_hwframe_map` when the pool actually carries
that tiling:

```c
if (VulkanDirectDecodeExportEnabled() && !drmModsAreLinearOrEmpty) { ... }
if (hf->tiling == VK_IMAGE_TILING_DRM_FORMAT_MODIFIER_EXT && vkf && ...)
```

So the only export shape Firefox can ask for is the modifier-tiled one -- row C,
which the host NVIDIA driver itself refuses. Row B (optimal tiling with DMA_BUF)
is supported by the host and is a real Venus gap worth closing for parity, but
closing it cannot help this path, because Firefox will not request an
optimal-tiled export. Reaching row B would require changing Firefox, which this
lab does not do.

**ffmpeg's export error path double-frees.** After making Venus fail the export
cleanly instead of dereferencing NULL, the crash moved to
`av_frame_unref -> ff_hwframe_unmap -> free`. In `vulkan_map_to_drm`,
`ff_hwframe_map_create` transfers ownership of `drm_desc` to the mapped frame
with `vulkan_unmap_to_drm` as its destructor, but every subsequent failure still
jumps to `end: av_free(drm_desc)`. Any failure after the map is created is
therefore a double free. It is not reachable on a native driver, where
`GetMemoryFdKHR` succeeds.

That is three error paths in a row -- Venus's, ffmpeg's, and Firefox's fallback
-- that had never executed, because on the hardware everyone develops against
the happy path always succeeds.

### What this means for the prototype

Direct export is closed on this hardware without patching Firefox. The
remaining route is the GPU-copy path, which:

- already runs hardware decode (measured earlier: decode commands and sessions
  were flowing while the output was green),
- needs no DMA-BUF export of decode output at all,
- and is blocked by a single defect in virglrenderer, which this lab forks:
  `CopyYUVDataImpl` performs a GL `BlitTextureToTexture` that fails with
  `GL_INVALID_OPERATION` and wedges the rendering context.

Selecting it requires only turning `direct-export.enabled` off, which makes
Firefox skip the modifier-tiling block and never call `av_hwframe_map`. The
Venus null-deref fix stays regardless: it is correct on its own terms, and it
is what turned an unattributable software-decode fallback into a legible error.
