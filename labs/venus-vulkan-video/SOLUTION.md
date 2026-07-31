# How stock Firefox ends up decoding on the host GPU

This is the full account of the problem and the fix. It assumes you have read
[`README.md`](./README.md) for what the lab is trying to do.

Firefox is **unmodified**. Every change described here is in guest Mesa, host
virglrenderer, or lab configuration.

---

## 1. The shape of the system

```
stock Firefox (guest)
  └─ libavcodec.so.62  (ffmpeg 8, --enable-vulkan)
       └─ VK_KHR_video_decode_h264
            └─ Mesa Venus ICD (guest)          ← forwards Vulkan over virtio-gpu
                 └─ virtio-gpu / crosvm        ← transparent, forwards bytes
                      └─ virglrenderer (host)  ← Venus renderer + virgl GL renderer
                           └─ NVIDIA Vulkan ICD → T1000 NVDEC
```

Two distinct things travel this path and it matters throughout:

- **Decode** is Vulkan, through Venus. It worked early and was never the hard
  part.
- **Presentation** is GL, through virgl. The decoded frame is exported as a
  dmabuf and re-imported as GL textures for the compositor. Everything that was
  broken lived here.

Confusing the two cost a lot of time. "Decode works" was true for a long while
before anything appeared on screen correctly.

---

## 2. What a decoded frame actually is

`vkCmdDecodeVideoKHR` writes to a single multi-planar `VkImage`. For NV12 that
image is one `VkDeviceMemory` allocation containing both planes:

```
offset 0        luma    1280x720, one byte per texel,  stride 1280
offset 983040   chroma   640x360, two bytes per texel, stride 1280
```

983040 is 1280x768: the luma plane is padded to a 768-row alignment before
chroma begins. Note that chroma starts **beyond** the end of the luma plane's
own extent (1280x720 = 921600 bytes). That detail matters later.

This single-allocation shape is not negotiable:

- **Two images instead of one.** ffmpeg will do this - its NV12 entry carries a
  `{R8_UNORM, R8G8_UNORM}` fallback - but a Vulkan decode destination must be a
  single multi-planar image, because `vkCmdDecodeVideoKHR` takes one
  `dstPictureResource`. Choosing it breaks decode.
- **Disjoint planes.** `VK_IMAGE_CREATE_DISJOINT_BIT` would give each plane its
  own memory. `DISJOINT` appears nowhere in ffmpeg's `hwcontext_vulkan.c`.
- **Changing the client.** Off limits, and Firefox is doing the portable thing.

So one buffer, two planes at offsets, is a given. Everything else has to cope.

---

## 3. How Firefox consumes it

Firefox exports that frame and imports the planes **separately**, one EGL image
each. From its own `Dmabuf` log:

```
Plane 0: fd=108 pitch=1280 modifier=... format=0x20203852 (R8)   size=1280x720
  Plane 0: zero-copy EGLImageTargetTexture2D succeeded
Plane 1: fd=129 pitch=1280 modifier=... format=0x38385247 (GR88) size=640x360
  Plane 1: zero-copy EGLImageTargetTexture2D succeeded
```

Every field is correct. `GR88` is the right fourcc for interleaved NV12 chroma,
640x360 is the right geometry, and pitch 1280 is right for 640 texels at two
bytes each. Both imports report success.

This is also the standard approach, not something exotic: the `R8`, `RG88` and
`GR88` DRM fourccs exist precisely so NV12 can be imported as "the Y plane as an
R8 EGLImage and the UV plane as either an RG88 or GR88 EGLImage", and Mesa's own
developer list describes NV12 as normally handled as two textures.

The two plane fds refer to **regions of one buffer**, so in the guest they
resolve to the same GEM handle.

---

## 4. The four defects

### 4.1 The plane index did not survive the import

`virgl_drm_winsys_resource_create_handle()` caches imported buffer objects by
GEM handle:

```c
r = drmPrimeFDToHandle(qdws->fd, whandle->handle, &handle);
res = util_hash_table_get(qdws->bo_handles, (void *)(uintptr_t)handle);
if (res) { ...; goto done; }        /* cache hit: the same virgl_hw_res */
```

Both planes hit the same entry, and nothing recorded which plane each import
was. Every plane therefore presented as plane 0 of a fresh buffer.

That index is load bearing further down: Mesa's sampler-view encoder writes
`res->metadata.plane` into the field virglrenderer reads to select a per-plane
image.

```c
/* virgl_encode.c */
if (res->metadata.plane) {
   assert(state->u.tex.first_layer == 0 && state->u.tex.last_layer == 0);
   virgl_encoder_write_dword(ctx->cbuf, res->metadata.plane);
} else {
   virgl_encoder_write_dword(ctx->cbuf, state->u.tex.first_layer | ...);
}
```

**Fix.** Record import offsets per buffer object in first-seen order. The index
of an offset is the plane index. Plane 0 is at offset 0 by construction.

### 4.2 The second import could not be described at all

The same function assigns the caller's `*blob_mem` only on the path that queries
`RESOURCE_INFO`:

```c
if (res) { ...; goto done; }              /* *blob_mem never assigned */
...
res->blob_mem = info_arg.blob_mem;
*blob_mem = info_arg.blob_mem;            /* only on the non-cached path */
```

The caller decides from `blob_mem` whether to describe the resource to the host
at all:

```c
/* virgl_resource.c */
if (res->blob_mem && plane == 0 && (...)) {
   vs->vws->resource_set_type(...);
}
```

On a cache hit the caller's value stayed 0 from its `CALLOC`, so **every import
sharing a buffer object with an earlier one silently declined to describe
itself**. Measured directly: the chroma import arrived with `blob_mem=0` where
the luma import had `blob_mem=2`.

**Fix.** Report the cached resource's own value.

### 4.3 A wider description never left the guest

With 4.1 and 4.2 fixed the driver can describe the buffer as the planar whole
when a further plane arrives. The winsys then dropped it:

```c
if (!res->maybe_untyped) {          /* already described once */
   mtx_unlock(&qdws->bo_handles_mutex);
   return;                          /* dropped before it is sent */
}
```

That is right for a *retype* and wrong for a *plane*. A description covering
more planes than any before it is not a correction of the earlier one; it names
part of the same buffer that nothing has described yet.

**Fix.** Track the widest plane count already described and let a wider one
through.

Note this is not a retype on the host either, and does not need to be:
`vrend_renderer_pipe_resource_set_type()` treats a second description of an
already-typed resource as a silent success, so the first plane's type and
texture are untouched.

### 4.4 The host built no image for the extra plane, then built it wrong

virglrenderer already has per-plane images and already selects them by index:

```c
/* vrend_create_sampler_view */
} else if (needs_view && view->u.buf.first_element < ARRAY_SIZE(res->aux_plane_egl_image) &&
           res->aux_plane_egl_image[view->u.buf.first_element]) {
   void *image = res->aux_plane_egl_image[view->u.buf.first_element];
   glGenTextures(1, &view->gl_id);
   glEGLImageTargetTexture2DOES(view->target, (GLeglImageOES) image);
```

Both halves of the mechanism existed. Nothing connected them, and two further
problems sat in the way.

**The images were never created for this case.** Upstream builds them only from
a `gbm_bo`, and only for formats that cannot take a texture view. Neither holds
here: crosvm initialises virglrenderer with surfaceless EGL and no GBM device,
so `egl->gbm` is NULL, and the resource is typed `R8`, which can take a view.

**Fix.** When a description arrives for a resource that is already typed, build
an image for each plane beyond the first, directly from the dmabuf, using the
per-plane stride and offset the guest sent. Index 0 is deliberately left NULL so
the first plane keeps using the ordinary path.

**The fourcc has to be one the driver accepts.** The two-channel 8-bit plane has
two DRM spellings that differ only in which byte is named first, and a driver
may take one and refuse the other. Querying this host's EGL:

```
R8=1  GR88=0  RG88=1  NV12=1
```

It advertises `RG88` and refuses `GR88` - so importing the chroma plane as
`GR88`, the semantically correct spelling, failed outright, both with the
buffer's modifier and with an inferred layout. Firefox carries the same
substitution for the same reason:

```cpp
uint32_t wasGR = (mDrmFormats[aPlane] == DRM_FORMAT_GR88 || ...);
if (modifierCount <= 0 && (wasGR || wasRG))
   swappedFormat = wasGR ? DRM_FORMAT_RG88 : DRM_FORMAT_GR88;
```

**Fix.** Try both spellings rather than assuming either.

**The plane view has to be resolved first.** The guest encodes the plane index
in the field that otherwise reads as `first_layer`, so a plane view arrives
looking like a request for layer N of a single-layer texture. The texture-view
branch validated that as a layer range and rejected it:

```
vrend_create_sampler_view: Invalid number of layers (N) or zero levels requested
```

which poisoned the context before the plane could be resolved.

**Fix.** Test for an auxiliary plane image before the texture-view branch. Safe
because an auxiliary image exists at an index only for a resource whose planes
were imported separately.

---

## 5. Supporting changes, and why each is still needed

These are not part of the plane story but the result does not stand without
them.

| Change | Why |
|---|---|
| `VN_DEBUG=no_nvidia_drm_spoof` | Venus zeroes `VkPhysicalDeviceDrmPropertiesEXT` on NVIDIA hosts as a WSI workaround. Firefox reads that same node for an unrelated decision. **Tested by removal:** taking it out returned 1820 `CmdSubmit3d` refusals, so it is load bearing, not leftover. |
| ffmpeg 8 on `LD_LIBRARY_PATH` | ffmpeg 7's `vulkan_map_to_drm` waits on `semaphoreCount = planes` while `f->sem[]` is sized by image count, so an NV12 frame reads `sem[1] == VK_NULL_HANDLE`. Firefox prefers `libavcodec.so.62` already; the nixpkgs wrapper hardcodes `ffmpeg_7`, so `.62` was simply not on the path. |
| Venus `vn_GetMemoryFdKHR` null guard | Exporting memory allocated without an export handle type dereferenced NULL in the guest ICD. The assert compiles out in release. Returning an error lets the caller recover instead of losing the process. |
| `PIPE_FORMAT_RG88_UNORM` in virgl's table | `DRM_FORMAT_GR88` maps to it, and it was absent, so it resolved to `VIRGL_FORMAT_NONE`. Now genuinely on the path: the trace shows `virgl_fmt=65`. |
| `GBM_FORMAT_GR88` in virglrenderer's table | The GBM/virgl conversion table had no entry for a two-channel 8-bit plane. |
| `direct-export.enabled = false` | Firefox only ever requests the modifier-tiled export shape, which the host NVIDIA driver itself refuses. See §7. |

---

## 6. Why the copy path is not the one in use

Firefox has two routes from a decoded frame to the screen.

- **GPU copy** blits plane by plane from the imported surface into its own
  textures. That blit goes through the resource's own texture rather than a
  sampler view, so it does not benefit from per-plane images.
- **Zero copy** hands the imported surface to the compositor, which samples it -
  through sampler views, which do select per-plane images.

The copy path is therefore still wrong even with everything above, and the lab
selects zero copy. That selection is itself gated: `HW_DECODED_VIDEO_ZERO_COPY`
is configured only after `gfxPlatformGtk::InitPlatformHardwareVideoConfig()`
passes an early return that requires `HARDWARE_VIDEO_DECODING` to be enabled,
and that feature is runtime force-disabled here by a VA-API probe that cannot
succeed - the virtio-gpu VA driver loads and initialises but advertises no H.264
profiles.

`gfx.blacklist.hardwarevideodecoding` is set to `1` (`FEATURE_STATUS_OK`) to
skip that probe. This is honestly a lie told to the browser about a capability
the guest does not have, and it is written down as one in the flake. It does not
hand decoding to VA-API: `InitHWDecoderIfAllowed()` tries `InitVulkanDecoder()`
before `InitVAAPIDecoder()`, so Vulkan still decodes and VA-API is never used
for a frame.

Making that capability genuinely true - by finishing virgl's VA-API video path,
or by V4L2 through `virtio_media` - would remove the need for the pref. Fixing
the copy path to use per-plane images would remove the need for zero copy. Both
are open.

---

## 7. Things that look like fixes and are not

Recorded because each cost real time and would otherwise be retried.

- **Propagating `blob_mem` alone.** Necessary, not sufficient. Without 4.3 the
  wider description is dropped in the winsys, and without 4.4 the host has
  nothing to build.
- **Adding format-table entries alone.** Both entries are real gaps and are
  kept, but nothing reached them while the plane was never described.
- **Direct export.** Firefox only requests the modifier-tiled export shape, and
  that exact query - NV12, `VIDEO_DECODE_DST`, `DMA_BUF` handle type - is
  refused by the **host NVIDIA driver itself**, measured with the same probe on
  both stacks. NV12 offers one modifier there, `LINEAR`, without
  `VIDEO_DECODE_OUTPUT`.
- **Turning zero copy off to avoid the failing blit.** Moves
  `GL_INVALID_OPERATION` from the decoder context to the renderer context. Both
  consumers fail on the same badly described surface.
- **Resolving the plane through GBM.** `virgl_egl_aux_plane_image_from_gbm_bo()`
  is the upstream route and cannot serve here, because crosvm gives
  virglrenderer surfaceless EGL with no GBM device.

---

## 8. Reproducing and verifying

```bash
cd /path/to/repo
export VENUS_LAB_FLAKE="$PWD/labs/venus-vulkan-video"
bash labs/venus-vulkan-video/host/lab-ctl.sh start

# wait for the guest, then play a clip and capture the frame
nix run "$VENUS_LAB_FLAKE#lab-ssh" -- 'python3 /tmp/playlive.py 20; grim /tmp/shot.png'

# host decode engine, sampled from the host while it plays
nvidia-smi dmon -s u -d 1 -c 35

bash labs/venus-vulkan-video/host/lab-ctl.sh stop
bash labs/venus-vulkan-video/host/lab-ctl.sh reap
```

**Always `stop` and then `reap`.** A dead launcher with live children looks
exactly like "not running" and leaves CPU-burning orphans. `reap` matches on the
lab binaries and the run directory, so it cannot touch unrelated VMs on the same
host.

What a good run looks like, from the renderer log:

```
failed plane                     0
Invalid number of layers         0
BLIT FAILED                      0
error processing gpu command CmdSubmit3d   0
Illegal command buffer           0
decode_cmds=1024 sessions=1
```

and `nvidia-smi dmon` showing a nonzero decoder column across most samples.

Useful opt-in traces, all off by default because they fire per frame:

| Variable | Where | Shows |
|---|---|---|
| `VIRGL_TRACE_IMPORT` | guest | every dmabuf import: GEM handle, cache hit or miss, plane index, `blob_mem`, whether it will be described |
| `VIRGL_TRACE_DMABUF_IMPORT` | host | the fourcc and geometry the host receives |
| `VIRGL_TRACE_BLIT` | host | every blit, not only failures, with resource identity and both formats |

`VREND_DEBUG` is **not** usable for any of this: `VREND_DEBUG_ENABLED` is false
whenever `NDEBUG` is defined, which is every build this lab runs, so
`VREND_DEBUG=blit` prints nothing at all. That silence cost several cycles and
is why the traces above exist as unconditional or environment-gated code.

---

## 9. Method notes

Two habits did most of the work, and both were learned the hard way here.

**Measure both ends and compare.** Firefox's own `Dmabuf` log said `GR88
640x360`; the host trace said `R8 1280x720`. Neither alone was suspicious.
Together they named the layer at fault immediately. Part of that evidence sat
uncollected in the guest's log file for most of the investigation.

**A control is worth more than a positive.** Running the same probe binary
against the host stack is what proved `GR88` was refused there. Running the same
decode host-native is what proved an early guest NVDEC reading of 0% was a
sampling artefact rather than a fundamental blocker.

Four successive causal claims in this investigation were wrong, and every one
came from reading source and reasoning forward rather than instrumenting and
reading back. The fix arrived within an hour of the first trace that logged both
sides of the same operation.
