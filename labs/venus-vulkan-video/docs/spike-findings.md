# Feasibility spike - Venus can carry Vulkan Video

**Result: it works.** Stock FFmpeg in the guest decodes H.264 on the host
NVIDIA T1000 through Venus/virtio-gpu, with frames staying in Vulkan images.

```
[Vulkan] Using queue family 3 (queues: 1) for decode
[h264]   Decode modes: reuse_dst_dpb
[h264]   Chosen frame pixfmt: nv12 (Vulkan ID: 1000156003)
[h264]   Vulkan decoder initialization successful
         pix_fmt: vulkan
         90 frames decoded; 0 decode errors
```

Artifacts: `evidence/spike-decode.log`, `evidence/spike-decode-negative.log`,
`evidence/spike-guest-vulkaninfo.txt`.

Negative control: the same clip without `-hwaccel vulkan` decodes to
`yuv420p` with zero Vulkan mentions. `/dev/video*` does not exist in the
guest, so nothing here is V4L2.

## What this retires

The plan rated three risks as the ones that could kill the prototype, and none
was testable without executing a decode. All three are now answered:

| Risk | Answer |
|---|---|
| DPB image sharing across virtio-gpu | Works. NVIDIA reports `reuse_dst_dpb` (`DPB_AND_OUTPUT_COINCIDE`), and the DPB images allocate and bind through the ordinary Venus image path. |
| Video session memory binding through Venus's memory path | Works unmodified. `vkGetVideoSessionMemoryRequirementsKHR` + `vkBindVideoSessionMemoryKHR` needed no special handling in `vn_device_memory`. |
| Decode queue family mapping | Works. The host's dedicated decode family surfaces as guest family 3, `transfer decode sparse`, and FFmpeg selects it. |

Combined with Phase 0's Firefox result - hardware WebRender **and**
`HARDWARE_VIDEO_DECODING_VULKAN` both available in the cage session - the
architecture is sound end to end. What remains is hardening and evidence, not
discovery.

## The four defects the spike found

Each was invisible to static analysis and each would have been written into the
hardened implementation as a permanent bug.

### 1. Mesa needs all thirteen entrypoints, and says nothing if they are missing

Mesa's entrypoint generator emits **weak** references
(`vk_entrypoints_gen.py:51-55`). A missing `vn_*` implementation is not a build
error; it becomes a `NULL` dispatch slot. The extension can therefore be
advertised, `vkGetDeviceProcAddr` returns non-NULL because the extension is
enabled, and the call dereferences NULL.

The Mesa fork previously carried *only* vendored protocol headers, so the
obvious reading - "add three lines to the passthrough table" - produces exactly
that failure.

### 2. The renderer was rejecting the images a decoder must allocate

Measured failure, before the fix:

```
[Vulkan] Image creation failure: VK_ERROR_FEATURE_NOT_PRESENT
[h264]   Failed setup for format vulkan: hwaccel initialisation returned error.
```

Capability queries had already succeeded and returned real host numbers, so
everything up to allocation looked healthy. W2's rejection surface was refusing
`vkCreateImage` for the DPB image, because it carries
`VK_IMAGE_USAGE_VIDEO_DECODE_DPB_BIT_KHR` and a `VkVideoProfileListInfoKHR`
pNext.

This is the reject-to-validate transition the W3 plan is about, arriving as a
concrete failure rather than a design discussion. **Notably, W2 advertising
nothing is what made it invisible**: no guest could reach the rejection while
the capset was masked, so the fact that the rejection covered mandatory decode
allocations could not surface until the surface went live.

### 3. Venus caches queue family properties, so the video pNext was never filled

`vn_GetPhysicalDeviceQueueFamilyProperties2` serves from a cache built at
physical-device init and special-cases exactly one chained struct,
`VkQueueFamilyGlobalPriorityProperties`. Anything else an application chains is
silently never written.

The intermediate state was the instructive one: the guest saw all three
extensions and the decode queue bit, but an empty codec list. An application
correctly refuses a video queue that decodes nothing - and FFmpeg's way of
refusing is a silent fall back to software.

### 4. The queue scrub was incoherent once decode was implemented

W2 stripped both video queue bits and zeroed `videoCodecOperations`, which was
right while the renderer implemented neither direction. Left in place after
decode landed, it advertised the extension on the device while the queue that
would carry it reported no video capability.

Measured: three extensions visible, zero `QUEUE_VIDEO_DECODE_BIT_KHR`.

## The one piece worth keeping

Everything else on the spike branches is throwaway. This is not.

The rejection table is generated, and the fix for defect 2 belongs in the
generator, not the header. `gen-video-reject.py` now takes a single
hand-written input:

```python
SUPPORTED_VIDEO_EXTENSIONS = frozenset({
    'VK_KHR_video_queue',
    'VK_KHR_video_decode_queue',
    'VK_KHR_video_decode_h264',
})
```

Every value, struct, pNext arm and scrub mask is derived by intersecting
vk.xml against it. A value contributed only by an extension not named there is
rejected automatically, including values added by a future vk.xml revision.

**The direction is the whole point.** W2's defect - one root cause found twelve
times across 23 panel rounds - was always a hand-written set deciding whether a
guard applies. Hand-listing what to *reject* reproduces it exactly, because such
a list is complete only until the registry changes. Naming what is *supported*
makes the default deny. This is W3 §4.6's requirement, landed early because the
spike needed it and hand-editing the generated header would have thrown the work
away.

Result: **0 decode values rejected, 28 encode values still rejected.**

Three subtleties the naive version got wrong, all worth preserving:

- A value reachable from any unsupported video extension stays rejected even if
  a supported extension also declares it. Supported extensions are skipped
  during collection rather than subtracted afterwards, because subtracting
  would clear a value on the strength of one declaration while another still
  reaches it.
- Predicates whose value set becomes empty are still emitted. Call sites
  reference them by name, so dropping one breaks the build - and silently
  unwiring a guard is worse than an always-false one, because the call site is
  where a later unsupported value would have to be caught.
- Support propagates across the `FlagBits` -> `FlagBits2` widening by **bit
  position**. The 64-bit variants are declared by `VK_KHR_maintenance5`, not by
  any video extension, so the decode bits stayed rejected in their widened form
  only. Position is the honest key; matching on spelling is the heuristic this
  generator exists to avoid.

## What the spike deliberately does not have

Stated plainly so none of it is mistaken for done. The spike trusts the guest
far more than the hardened implementation will:

- no profile validation - any profile is forwarded to the host driver;
- no DPB reference-slot membership or range checks;
- no coding-scope tracking, so a decode outside `Begin`/`End` reaches the
  driver, where the spec says the behaviour is undefined rather than an error;
- no `updateSequenceCount` enforcement;
- no aggregate resource caps, so a guest can loop session creation;
- no generation-tagged handles and no two-pass handle replacement, so a
  partially-failed replacement can forward a mix of guest and host handles;
- no negative controls for any of the above.

Handle lookups *are* checked, because a NULL dereference is not a useful
experiment.

## Carried into W3 and W4

The hardened implementation now has a known-good shape to be written against
rather than an imagined one:

1. **Mesa must implement every entrypoint of any extension it advertises.** The
   weak-reference behaviour makes this silent, so it wants an explicit check
   rather than reviewer attention.
2. **Anything Venus caches must cache what applications chain.** The queue
   family cache is one instance; W4 should look for others rather than assume
   this was the only one.
3. **The supported-set generator is the W3 §4.6 deliverable** and is already
   landed. W3 inherits it rather than designing it.
4. **`HARDWARE_VIDEO_DECODING` remains blocklisted in Firefox** with
   `FEATURE_FAILURE_VIDEO_DECODING_TEST_FAILED` - the VA-API probe failing, as
   expected in a guest with no VA-API driver. Whether that suppresses the
   Vulkan decoder in Firefox specifically is still unanswered; FFmpeg does not
   consult it. W6 settles it.
5. The `CtxDetachResource: ErrRutabaga(InvalidContextId)` messages in the crosvm
   log predate the spike and did not prevent decode. Unexplained, not
   load-bearing, and worth a look during W8 rather than now.
