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

## 6. The two presentation routes, and why zero copy is preferred

Firefox has two routes from a decoded frame to the screen.

- **Zero copy** hands the imported surface to the compositor, which samples it -
  through sampler views, which select per-plane images. **This is the preferred
  route and the one the lab selects**, because handing the surface over beats
  copying it: no per-frame plane copies at all.
- **GPU copy** blits plane by plane from the imported surface into Firefox's own
  textures. A blit does not go through a sampler view, so it needed the same
  per-plane resolution wiring separately. It now has it - see 6b - so the
  fallback is correct rather than broken, but it is still a copy, and still the
  fallback.

Zero copy is gated: `HW_DECODED_VIDEO_ZERO_COPY` is configured only after
`gfxPlatformGtk::InitPlatformHardwareVideoConfig()` passes an early return that
requires `HARDWARE_VIDEO_DECODING` to be enabled, and that feature is decided by
a VA-API probe.

That probe used to be unpassable, because the guest's `virtio_gpu` VA driver
loaded and initialised and then advertised no H.264 profiles. The lab therefore
set `gfx.blacklist.hardwarevideodecoding` to `1` (`FEATURE_STATUS_OK`) to skip
it - a lie told to the browser about a capability the guest did not have,
written down as one in the flake.

**That pref is gone, and the capability is real.** See section 6a.

`InitHWDecoderIfAllowed()` tries `InitVulkanDecoder()` before
`InitVAAPIDecoder()`, so VA-API is what makes the capability true and Vulkan
Video is still what decodes every frame.

Fixing the copy path to use per-plane images would remove the need for zero copy
in the first place. That one is still open, and the note in section 7 records a
measurement that narrows it.

---

## 6a. Making the capability honest

The guest advertising no H.264 profiles read like a missing host capability. It
was not. It was one flag.

Measured in order, each one cheap and each one narrowing the next:

| Question | Answer |
|---|---|
| Does the host do VA-API H.264 at all? | Yes - Main, High, ConstrainedBaseline, NVDEC direct backend |
| Does it still work inside the crosvm GPU sidecar's exact bwrap bind set? | Yes, identical |
| Is virglrenderer built with video? | Yes, `-Dvideo=true`, so `ENABLE_VIDEO` is defined |
| Does rutabaga supply the `get_drm_fd` callback video needs? | Yes |
| Does anything pass `VIRGL_RENDERER_USE_VIDEO`? | **No** |

That last row is the whole defect. `VIRGL_RENDERER_USE_VIDEO` is bit 11 of the
flags word `virgl_renderer_init()` takes. `rutabaga_gfx` generates the constant
into `src/generated/virgl_renderer_bindings.rs` and references it nowhere; its
`VirglRendererFlags` stops at bit 10 and exposes no `use_video()` builder, and
the inner `u32` is private, so crosvm cannot set the bit even deliberately.

The failure is silent because of where it lands. `virgl_video_init()` is what
assigns `va_dpy`; `virgl_video_fill_caps()` returns `-1` immediately on a NULL
`va_dpy`; so the virgl2 capset reaches the guest with `num_video_caps = 0` and
nothing anywhere reports an error. The guest driver then loads cleanly and
advertises nothing, which is indistinguishable from a host that cannot decode.

Turning it on exposed a second refusal underneath, and this one is a string
comparison:

```
INFO   VA-API version: 1.24
INFO   Driver version: VA-API NVDEC driver [direct backend]
ERROR  only supports mesa va drivers now
```

`virgl_video_init()` rejects every VA driver whose vendor string lacks
`Mesa Gallium`. libva had initialised and the NVDEC driver had loaded; it was
turned away on its name. The one host API this path needs is
`vaExportSurfaceHandle()` with `VA_SURFACE_ATTRIB_MEM_TYPE_DRM_PRIME_2`, which
is standard rather than Mesa-specific and which `nvidia-vaapi-driver`
implements already - it is how that driver hands frames to EGL consumers.
Upstream's own "now" reads as provisional.

Both are opt-in in the fork, and deliberately **two** knobs rather than one:
`VIRGL_FORCE_VIDEO` enables video, `VIRGL_VIDEO_ALLOW_ANY_VA_DRIVER` accepts a
non-Mesa driver. Enabling video and trusting this driver are different
decisions, and keeping them separable is what makes a later failure
attributable to one of them. Unset, both refuse exactly as upstream does.

`VIRGL_FORCE_VIDEO` tests its **value**, not its presence, unlike the trace
knobs in the same file. Those are diagnostics; this gates a capability that
needs a negative control, and a knob that cannot express "off" is precisely how
this program already produced one false pass on a locked Firefox pref.

### The result

| `VIRGL_FORCE_VIDEO` | host renderer | guest H.264 profiles |
|---|---|---|
| `0` | `Video not enabled` | **0** - driver loads, advertises nothing |
| `1` | `Video initialised on drm_fd 43` | **3** - ConstrainedBaseline, Main, High |

The off row reproduces the original symptom exactly, on demand. That is what
makes this causation rather than coincidence.

With the pref removed and the probe passing on what the driver reports:

```
renderer decode      : decode_cmds=2048 sessions=1
plane image failures : 0     BLIT failures      : 0
layer-validation err : 0     CmdSubmit3d        : 0
Illegal cmd buffer   : 0
frames               : 810 total, 6 dropped
host NVDEC           : nonzero in 35 of 35 samples
picture              : 12,619 distinct colours, mean RGB 152,158,158
```

`decode_cmds` is the load-bearing number: the negative control established that
it reads `0` when the Vulkan decoder is off, so a nonzero value means Vulkan
Video decoded, not VA-API.

The NVDEC percentage sampled higher than earlier runs in this lab. That is
**not** claimed as an improvement: this sampling window sat entirely inside
continuous looping playback, where earlier windows included startup, and the
earlier configuration has not been re-measured under this window. Unattributed.

### Correction: the guest's VA-API decode is not hardware backed

An earlier revision of this section said the probe now passes "on the merits"
and called the capability real without qualification. That was overstated, and
the measurement that should have accompanied the claim contradicts it.

What was actually established is that the guest **advertises** H.264 through
VA-API, and that the advertisement is what Firefox's probe reads. Whether that
advertisement is honest all the way to the decode engine is a separate question,
and it was not asked before the claim was made.

Asked afterwards, with the same probe run on both stacks:

| Decode | Frames | Speed | Host NVDEC |
|---|---:|---:|---:|
| Host-native VA-API | 72,180 | 46x | **94-98%** |
| Guest VA-API through virgl | 135,090 | 264x | **0%, every sample** |

The host row is the control, and it earns its keep twice: it proves the
instrument attributes NVDEC correctly, and it proves `nvidia-vaapi-driver`
drives the engine. So the guest row is not a measurement artefact.

The speed is the tell. The guest path ran **5.7x faster than the real hardware
decoder** while reporting 0 decode errors. Nothing beats the decode engine by
5.7x while using it, so whatever the guest is doing, it is not that.

Two readings that were wrong along the way, recorded because both are easy to
repeat:

- **`h264 (native)` in the stream mapping does not mean software.** With
  `-hwaccel`, "native" names the *decoder*; the hwaccel backend does the work.
  `pix_fmt: vaapi` is the field that actually answers the question.
- **`-hwaccel_output_format vaapi` exiting 0 does not prove hardware decode.**
  It proves VA-API surfaces were produced, not that a decode engine produced
  them.

### What this does and does not change

Unaffected, and still measured:

- Firefox carries no patches.
- Firefox decodes through **Vulkan Video**, not VA-API. `InitHWDecoderIfAllowed()`
  tries `InitVulkanDecoder()` before `InitVAAPIDecoder()`, `decode_cmds` is
  nonzero, and host NVDEC is nonzero during Firefox playback. That path is
  genuinely hardware backed.
- Removing `gfx.blacklist.hardwarevideodecoding` is still the right move and
  still works: Firefox now reaches its own conclusion from what the driver
  reports, instead of having the probe bypassed. Playback is correct with zero
  errors on every surface.

Changed:

- The justification is weaker than claimed. The pref removal replaced "assert a
  capability the guest does not have" with "rely on a capability the guest
  advertises but has not been shown to possess". That is an improvement, not the
  clean result the earlier wording described.
- The residual risk is narrow but real: if Firefox ever selected VA-API ahead of
  Vulkan, it would be selecting on an advertisement this lab has now measured
  against. It does not select it today.

### Where it terminates

That question is now answered, by counting the virgl decode path the way the
Venus one was already counted. Adding the counter took one small change; not
having it is why the earlier claim went unchallenged.

The guest is not the problem. Commands flow all the way across:

```
VIRGL-VIDEO-EVIDENCE decode_bitstream=2048 failed=0 last_err=0
```

The guest sends decode commands, virglrenderer receives them, and every
`vaCreateBuffer` and `vaRenderPicture` succeeds. Then:

```
ERROR  end picture failed, err = 0x17
```

2790 of them in one run, no other error code. `0x17` is
`VA_STATUS_ERROR_DECODING_ERROR`. The submission stage succeeds and
`vaEndPicture` - the call that actually commits the decode - is rejected by
`nvidia-vaapi-driver` for every frame.

That explains all three observations at once: NVDEC stays idle because no
decode is ever committed; the path runs 5.7x faster than real hardware because
nothing decodes; and the guest sees no error because the failure is entirely
host side and never travels back.

It also means the counter that read `failed=0` was measuring the wrong stage.
It counts `decode_bitstream`, which is `vaRenderPicture`; the failure is one
call later. A counter placed one stage short of the failure reports success
just as confidently as a correct one.

### The upstream check was load bearing after all

The `Mesa Gallium` refusal was overridden on the reading that it looked
conservative: the only host API this path needs to return a frame is
`vaExportSurfaceHandle()` with `DRM_PRIME_2`, which is standard and which
`nvidia-vaapi-driver` implements.

That reading was wrong, and this is what wrong looks like when it is measured
rather than argued. Export was never the hard part. **Consuming
virglrenderer's picture parameters and slice data is**, and this driver will
not. Upstream's "only supports mesa va drivers now" is a real constraint on
this driver, not an unreviewed leftover.

The override is kept, reclassified from unproven to known-failing, because the
failure is now precisely located and worth being able to reproduce. Its warning
names the call, the status code and the conclusion.

Keeping it separate from `VIRGL_FORCE_VIDEO` is what made this attributable at
all: video initialisation was correct and stayed correct, and the driver was
the thing that did not work. A single combined knob would have left both
suspects alive.

### Consequences for the pref

`gfx.blacklist.hardwarevideodecoding` stays removed, and the reasoning is now
fully in the open rather than resting on an assumption:

- Firefox decodes through Vulkan Video, which is measured as hardware backed.
  It never uses VA-API for a frame.
- Removing the pref stops bypassing the probe, which is strictly more
  transparent than asserting `FEATURE_STATUS_OK` over it.
- The advertisement Firefox reads is nonetheless **not** hardware backed on
  this host, and that is now a measured fact rather than an open question.

The residual risk is unchanged and narrow: a Firefox that preferred VA-API over
Vulkan would select on an advertisement whose decode fails. It does not, because
`InitHWDecoderIfAllowed()` tries `InitVulkanDecoder()` first.

Making the advertisement honest means making `vaEndPicture` succeed on a
non-Mesa driver. That is not configuration, and it is not a small fix.

### Why, exactly

The driver's own log names the failing call:

```
nvEndPicture cuvidDecodePicture failed: 1
```

`cuvidDecodePicture` is NVDEC's entry point and `1` is
`CUDA_ERROR_INVALID_VALUE`. Surfaces are created without complaint
(`nvCreateSurfaces2 Creating surface 1280x720, format 1`), so the rejection is
the picture itself.

virglrenderer's `h264_fill_slice_param()` sets two fields and leaves the rest
commented out:

```c
//vasp->slice_data_size;
//vasp->slice_data_offset;
//vasp->slice_data_flag;
//vasp->slice_data_bit_offset;
//vasp->first_mb_in_slice;
//vasp->slice_type;
ITEM_SET(vasp, desc, num_ref_idx_l0_active_minus1);
ITEM_SET(vasp, desc, num_ref_idx_l1_active_minus1);
```

That looks like laziness. It is not. **The data is not on the wire.** The
protocol's `virgl_h264_picture_desc` carries exactly one slice-related field:

```c
uint32_t slice_count;
```

A count. No per-slice size, offset, type, or first-macroblock. So
`h264_fill_slice_param()` cannot populate those fields, because the guest never
sent them.

### Why that is fine for Mesa and fatal for NVDEC

The virgl video protocol was designed against Mesa's VA drivers, which hand the
bitstream to hardware that parses slice headers itself. A driver that re-parses
does not need per-slice VA parameters, so the protocol never carried them.

NVDEC does not re-parse. `nvidia-vaapi-driver` builds `CUVIDPICPARAMS` from the
VA slice parameters, gets zeros where offsets and sizes belong, and
`cuvidDecodePicture` correctly rejects it.

So the constraint is **architectural, not a bug**. Upstream's
"only supports mesa va drivers now" is an accurate statement about what the
protocol can express. Lifting it requires extending the virgl video wire format
to carry per-slice parameters, across guest Mesa and virglrenderer, for every
codec - a protocol change, not a patch.

The lab's decision rule covers exactly this case: when forwarding is blocked by
a fundamental limitation, stop and document the exact blocker rather than
inventing a second architecture. This is that blocker, named to the field.

None of it is required here. This prototype decodes through Vulkan Video, which
is a different path with its own wire format, and which works.

---

## 6b. Fixing the GPU-copy fallback

The copy path produced a green picture: luma copied, chroma did not. That is the
original green-frame symptom, arriving through the other route.

Two separate defects, and the first fix hid the second.

### The plane was found; the copy was the problem

A blit command carries only resource handles, so when the planes of one buffer
share a resource nothing in the command names the plane. Resolving it needs the
same per-plane image the sampler views already use.

That resolution was written first and appeared not to work. Instrumenting each
of its five guards separately showed it working perfectly:

```
BLIT-PLANE calls=512 ok=256 same_fmt=256 no_egl=0 no_aux=0 ambiguous=0 bad_dims=0
```

512 blits splitting exactly 256 resolved and 256 `same_fmt` - the luma/chroma
pair, one of each per frame. **A bare `-1` return had said nothing about which
condition rejected, and five conditions have to hold together.** Counting them
separately turned "it does not work" into "it works, look elsewhere" in one run.

### `glCopyImageSubData` cannot consume an EGLImage texture

The failure was one step later, and the diagnostic named it:

```
BLIT FAILED (gl 0x502) via glCopyImageSubData:
  src fmt PIPE_FORMAT_R8_UNORM (view PIPE_FORMAT_R8G8_UNORM) gl_ifmt:0x1903
  -> dst fmt PIPE_FORMAT_R8G8_UNORM gl_ifmt:0x822b
```

`0x502` is `GL_INVALID_OPERATION`. `glCopyImageSubData` takes no formats: it
derives them from the texture objects and requires the two to share a texel size
class. Reading the format off the plane texture directly, while bound, gives the
answer:

```
BLIT-PLANE tex plane=1 ifmt=0x0 640x360
```

**The dimensions are right and the internal format is nothing at all.** A texture
bound from an EGLImage reports `GL_TEXTURE_INTERNAL_FORMAT` as `0` here, so
`glCopyImageSubData` cannot classify it and refuses.

Sampling carries no such requirement. That is precisely why these same per-plane
images already work for zero copy's sampler views, and it is the whole fix: the
plane blit belongs on the shader path, which samples.

`vrend_renderer_blit()` already excludes one case from the copy fast path for a
related reason - resources needing colorspace conversion "must have it applied
manually in a shader, i.e. require following the `vrend_renderer_blit_int()`
path". The plane blit joins it, one line above the existing guard.

The plane's own dimensions are set on the temporary resource, because the
blitter derives its source rectangle from them and the resource is typed by
luma.

### Result

| | before | after |
|---|---:|---:|
| `BLIT FAILED` | 1696 | **0** |
| Illegal command buffer | 0 | 0 |
| `CmdSubmit3d` refusals | 0 | 0 |
| Picture | flat green | **colour bars, mean RGB 155,155,158** |

### Zero copy is unchanged, and provably so

Zero copy remains the preferred and selected route
(`media.ffmpeg.vaapi.force-surface-zero-copy = 1`). Re-measured after the fix:
zero blit failures, zero plane-image failures, zero layer-validation errors,
zero illegal command buffers, zero refusals, `decode_cmds=512`, 511 frames with
6 dropped, and a correct picture at mean RGB 155,155,159.

The stronger evidence is that `BLIT-PLANE` printed **nothing at all** on that
run. The plane lookup is never reached, because zero copy issues no blits. The
change is inert on the preferred path by construction, not by luck.

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
- **Two wrong reasons for the copy path's failure**, both recorded here as
  settled before they were. The repair itself is real and is described in 6b;
  what belongs in this section is the pair of dead ends on the way to it.

  First: "the guest never imports the chroma plane separately when zero copy is
  off", measured as 6482 `plane=0` imports and zero `plane=1`. Re-measured while
  forcing the copy path with `media.ffmpeg.vaapi.force-surface-zero-copy = 0`,
  the same trace gives **6867 `plane=0` and 1372 `plane=1`**. Both runs reached
  the copy path, by different doors - the first by removing
  `gfx.blacklist.hardwarevideodecoding` so `HW_DECODED_VIDEO_ZERO_COPY` was
  never configured, the second by turning the surface pref off - and only the
  first suppresses the separate import. **"Force the copy path" is not one
  configuration**, and a measurement of it has to say which mechanism selected
  it.

  Second: "the plane lookup returns -1, so plane resolution does not work." It
  returned -1 for a reason nobody had asked for. Five guards have to hold
  together and a bare -1 names none of them; counting each separately showed the
  lookup resolving 256 of 512 blits correctly on the first instrumented run. The
  defect was one call later, in `glCopyImageSubData`.

  The shared shape is worth more than either: **a negative result from a
  predicate with several clauses is not evidence about any one of them.**
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
| `VIRGL_TRACE_BLIT_PLANE` | host | per-plane blit resolution: a separate count for each guard that can reject it, plus the plane texture's real internal format and size |

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
