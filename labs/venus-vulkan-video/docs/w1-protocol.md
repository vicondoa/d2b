# W1 - venus-protocol H.264 wire format

Status: **in progress.** The extension list, command-id allocation and the
append-only ABI gate are done and verified. StdVideo serialization is the
remaining work.

Fork: `vicondoa/venus-protocol-vulkan-video`, branch `vulkan-video`,
base `base/70991d4`.

## Append-only ABI gate - ✅ PASS

```bash
export VENUS_LAB_PYTHON=<python-with-mako>
bash tests/abi-gate.sh --snapshot ~/.local/state/venus-lab/evidence/cmdids-golden.txt  # from base/, BEFORE editing
bash tests/abi-gate.sh --check   ~/.local/state/venus-lab/evidence/cmdids-golden.txt
```

```
abi-gate: golden=345 current=358
abi-gate: PASS -- 345 ids preserved byte-identical
abi-gate: 13 additions, ids 346..358
```

| id | command |
|---|---|
| 346 | `vkGetPhysicalDeviceVideoCapabilitiesKHR` |
| 347 | `vkGetPhysicalDeviceVideoFormatPropertiesKHR` |
| 348 | `vkCreateVideoSessionKHR` |
| 349 | `vkDestroyVideoSessionKHR` |
| 350 | `vkGetVideoSessionMemoryRequirementsKHR` |
| 351 | `vkBindVideoSessionMemoryKHR` |
| 352 | `vkCreateVideoSessionParametersKHR` |
| 353 | `vkUpdateVideoSessionParametersKHR` |
| 354 | `vkDestroyVideoSessionParametersKHR` |
| 355 | `vkCmdBeginVideoCodingKHR` |
| 356 | `vkCmdEndVideoCodingKHR` |
| 357 | `vkCmdControlVideoCodingKHR` |
| 358 | `vkCmdDecodeVideoKHR` |

Two things worth recording about *why* this passes:

1. Upstream's `utils/print_vk_command_types.py` is **append-only by
   construction** - it reuses ids already present in the XML and only allocates
   new ones for genuinely new commands. The panel's renumbering concern was
   real in principle but is structurally prevented by the tool.
2. The generator **reproduces the committed XML exactly** at `base/70991d4`
   (345 ids, byte-identical), so the golden snapshot is trustworthy rather than
   an artifact of a differing environment.

The gate stays in place regardless, because the values are hand-assigned in
XML and a typo or a reused value would be just as damaging as a renumbering.

## What the generator already produces - more than expected

Adding the three extensions and regenerating **succeeds with no errors** (38
files). It emits:

- all **13** video command encoders (`vn_encode_*` / `vn_submit_*`)
- the `VkVideoSessionKHR` and `VkVideoSessionParametersKHR` handles
- the Vulkan-side video structs: `VkVideoSessionCreateInfoKHR`,
  `VkVideoDecodeInfoKHR`, `VkVideoBeginCodingInfoKHR`,
  `VkVideoCodingControlInfoKHR`, `VkVideoEndCodingInfoKHR`,
  `VkVideoReferenceSlotInfoKHR`, `VkVideoPictureResourceInfoKHR`,
  `VkVideoCapabilitiesKHR`, `VkVideoFormatPropertiesKHR`,
  `VkVideoSessionMemoryRequirementsKHR`,
  `VkVideoSessionParametersCreateInfoKHR`/`UpdateInfoKHR`
- the **profile-list types** the plan called out as needed on the image, buffer
  and format-query paths: `VkVideoProfileInfoKHR`, `VkVideoProfileListInfoKHR`,
  `VkVideoDecodeUsageInfoKHR`

## The remaining gap - and it is silent

**Zero `StdVideo` references appear anywhere in the generated output**, and
these four structs are absent entirely:

| struct | why it matters |
|---|---|
| `VkVideoDecodeH264PictureInfoKHR` | carries `pStdPictureInfo` - the per-picture codec state |
| `VkVideoDecodeH264ProfileInfoKHR` | selects the H.264 profile for session creation |
| `VkVideoDecodeH264DpbSlotInfoKHR` | per-DPB-slot reference metadata |
| `VkVideoDecodeH264SessionParametersAddInfoKHR` | the SPS/PPS arrays |

### Mechanism

`Gen.is_serializable()` rejects types it has no schema for. `get_chain()` splits
a struct's `pNext` into serializable and skipped, and `_init_supported_types()`
**rewrites `ty.p_next` to the serializable subset**. The H.264 structs point at
`StdVideo*` types, which are declared in `vk_video/vulkan_video_codec_h264std*.h`
rather than in `vk.xml`, so they are dropped.

**The failure mode is silence.** Generation succeeds, the code compiles, and all
13 commands look complete - but the codec payload would never cross the wire.
A guest would submit a decode whose picture info simply is not there. This is
precisely the "silently wrong" hazard raised in plan review, now confirmed with
its exact mechanism rather than predicted.

### Why the headers being vendored still helps

The `StdVideo*` **type definitions** are already vendored on both sides
(`src/venus/venus-protocol/vk_video/` in virglrenderer, `include/vk_video/` in
Mesa). So the remaining work is serialization only - the types themselves do not
need to be introduced, described in XML, or kept in sync by hand.

### What the serialization must not do

`StdVideoH264SpsFlags`, `StdVideoDecodeH264PictureInfoFlags` and friends are
**bitfield** structs. Their layout is not guaranteed across compilers, ABIs or
header versions, so a raw `sizeof`/`memcpy` wire encoding would be silently
wrong in exactly the way that is hardest to debug. Encoding must be
**field-level**, per the schema requirement carried from plan review.

## Design decision: how StdVideo gets serialized

Two facts constrain this, and together they pick the design:

1. **`vk.xml` already describes the Vulkan wrapper structs.** For example
   `VkVideoDecodeH264PictureInfoKHR` is fully specified, including
   `pSliceOffsets` with `len="sliceCount"`. Only the `StdVideo*` types are
   opaque, declared as bare names:
   ```xml
   <type requires="vk_video/vulkan_video_codec_h264std_decode.h"
         name="StdVideoDecodeH264PictureInfo"/>
   ```
2. **venus-protocol already has a private-XML mechanism.**
   `VN_PROTOCOL_PRIVATE_XMLS` loads `VK_MESA_venus_protocol.xml` and
   `VK_EXT_command_serialization.xml`, which use the ordinary registry
   `<types>` schema.

**Decision: describe the `StdVideo` H.264 types in a new private XML** rather
than hand-writing serializers or teaching the generator to parse C headers.
The existing generator then emits field-level encode/decode/size functions for
them automatically. This satisfies the "field-level schema, no raw struct
copies" requirement *by construction* - there is no hand-written serialization
to review, and no path by which a raw `memcpy` could creep in.

### The bitfield problem, and why flags are packed explicitly

`StdVideoDecodeH264PictureInfoFlags` and friends are bitfields:

```c
typedef struct StdVideoDecodeH264PictureInfoFlags {
    uint32_t    field_pic_flag : 1;
    uint32_t    is_intra : 1;
    uint32_t    IdrPicFlag : 1;
    uint32_t    bottom_field_flag : 1;
    uint32_t    is_reference : 1;
    uint32_t    complementary_field_pair : 1;
} StdVideoDecodeH264PictureInfoFlags;
```

Declaring each bit as a plain XML member does **not** work. The generator's
scalar helpers take pointers:

```c
static inline void vn_encode_uint32_t(struct vn_cs_encoder *enc, const uint32_t *val);
```

so it would emit `vn_encode_uint32_t(enc, &val->field_pic_flag)` - and **taking
the address of a bitfield is illegal C**. It would not compile.

Describing the flags struct as a single opaque `uint32_t` and copying it would
compile, but reintroduces exactly the ABI hazard raised in plan review: bitfield
allocation order and padding are implementation-defined, so guest and host could
disagree silently.

**Therefore each `*Flags` struct is carried on the wire as one `uint32_t`, packed
and unpacked by explicit shifts over named fields:**

```c
static inline uint32_t
vn_pack_StdVideoDecodeH264PictureInfoFlags(const StdVideoDecodeH264PictureInfoFlags *f)
{
    return ((uint32_t)f->field_pic_flag           << 0) |
           ((uint32_t)f->is_intra                 << 1) |
           ((uint32_t)f->IdrPicFlag               << 2) |
           ((uint32_t)f->bottom_field_flag        << 3) |
           ((uint32_t)f->is_reference             << 4) |
           ((uint32_t)f->complementary_field_pair << 5);
}
```

Each flag is read and written **by name**, so the compiler handles its own
bitfield layout on each side independently and the wire encoding is fixed by the
shift constants rather than by any ABI. This is field-level in the sense that
matters - no struct layout is ever assumed - while remaining legal C.

The shift assignments are part of the wire contract and are therefore covered by
the same append-only discipline as command ids: reordering them would silently
corrupt decode parameters.

## Status: StdVideo serialization COMPLETE

All four H.264 wrapper structs now serialize, and the generated output compiles
and links in **both** the driver and renderer variants -- the dual-build
checkpoint required at plan review.

| struct | status |
|---|---|
| `VkVideoDecodeH264PictureInfoKHR` | generated |
| `VkVideoDecodeH264ProfileInfoKHR` | generated |
| `VkVideoDecodeH264DpbSlotInfoKHR` | generated |
| `VkVideoDecodeH264SessionParametersAddInfoKHR` | generated |

18 `StdVideo` types have field-level serializers, including the SPS, PPS, VUI,
HRD and scaling-list types.

### Four things could not be expressed in XML

1. **Bitfield flag structs** -- carried as one `uint32_t` packed by explicit
   shifts over named fields. Generated from X-macro bit lists so pack and
   unpack cannot drift apart. Undefined bits are **rejected**, not masked.
2. **StdVideo enums** -- declared but never defined by the generator, and their
   underlying type is implementation-defined, so they go through an `int32_t`
   temporary exactly as the existing `size_t` helper does. Decode rejects values
   that do not survive the round trip.
3. **`StdVideoH264ScalingLists`** -- fixed-size 2D arrays, which the generator
   emits as `..., 6][16)`. Hand-serialized; both extents are compile-time
   constants so the size cannot be influenced by the guest.
4. **`int8_t` / `int16_t`** -- simply absent from `PRIMITIVE_TYPES`. Vulkan never
   uses them; the H.264 picture parameter set does, and their absence silently
   rejected the entire struct.

### Verified

```
abi-gate: PASS -- 345 ids preserved byte-identical, video at 346..358
ninja: driver + renderer compile and link
flag tests: all 5 flag types exhaustive round-trip; undefined bits rejected
```

## Remaining W1

1. ~~Bounds validation and allocation caps~~ - done; explicit caps landed and are
   exercised by T5/T6 of the round-trip harness.

2. `pNext` rejection tests - deferred to W2. The renderer-side dispatch they
   would test does not exist until `vkr_video.c` does.
3. ~~Fuzzable entry points over the new decoders~~ - done; T5 sweeps every
   4-byte word of every payload under ASan/UBSan.
4. ~~Wire the regenerated headers into the virglrenderer and Mesa forks~~ - done;
   both build.

## Bounds, overflow and allocation caps - inherited, and verified by reading

The plan required bounds validation, allocation caps and overflow checks on
guest-controlled counts. **These are satisfied by construction**, because the
payload structs go through the generator rather than hand-written serialization.

The generated decode for the guest-controlled counts looks like:

```c
vn_decode_uint32_t(dec, &val->stdSPSCount);
if (vn_peek_array_size(dec)) {
    const uint32_t iter_count = vn_decode_array_size(dec, val->stdSPSCount);
    val->pStdSPSs = vn_cs_decoder_alloc_temp_array(dec, sizeof(*val->pStdSPSs), iter_count);
    if (!val->pStdSPSs) return;
    ...
```

Three protections, all pre-existing:

1. **Count/size agreement.** `vn_decode_array_size(dec, expected)` compares the
   encoded array size against the declared count and calls
   `vn_cs_decoder_set_fatal(dec)` on any mismatch. A guest cannot declare
   `stdSPSCount = 5` and then supply a thousand elements.
2. **Integer overflow.** virglrenderer's `vkr_cs_decoder_alloc_temp_array` uses
   `__builtin_mul_overflow(size, count, &alloc_size)` and fails closed -
   logging, marking the stream fatal, and returning NULL.
3. **Allocation failure.** Every generated call site checks the returned pointer
   and returns immediately.

This covers `stdSPSCount`, `stdPPSCount`, `sliceCount` and
`pOffsetForRefFrame` - every guest-controlled length in the H.264 surface.

This is the strongest argument for describing the types in XML rather than
hand-writing serializers: the security properties are inherited automatically
and cannot be forgotten in one code path.

The fixed-extent members carry no risk at all: `PicOrderCnt[2]`, the scaling
lists and the HRD arrays all have compile-time constant extents taken from the
`STD_VIDEO_H264_*` macros.

### Now executable: round-trip, truncation and corruption

The earlier revision of this section said executable tests needed the real
`vkr_cs` from virglrenderer and scoped them to W2. That deferral is
**withdrawn**: the real primitives are five lines each and are now transcribed
into `tests/roundtrip/{vn_cs,vkr_cs}.h` from Mesa's `vn_cs.h` and
virglrenderer's `vkr_cs.h`, so the code under test is the generated serializer
rather than a mock of it.

Three deliberate deviations make the harness **stricter** than production:

| deviation | why |
|---|---|
| encode buffer is exact-sized and heap-allocated | ASan turns a one-byte overrun into a hard error; the real ring buffer would absorb it silently |
| inter-field padding is poisoned with `0xa5` | a decoder that reads padding sees poison, not a plausible zero that might match by luck |
| temp allocations are individual `malloc`s, not a bump pool | every decoded array gets its own redzone, so an off-by-one lands in a redzone rather than in the next object |

Six properties, `tests/roundtrip/main.c`:

| id | property | why it is the right check |
|---|---|---|
| **T1** | `encode(decode(b)) == b` | complete over the wire format *by construction* - any dropped, reordered, truncated or mis-packed field changes the bytes. There is no hand-written field-by-field comparator to drift out of date as the schema grows. |
| **T2** | `vn_sizeof_X()` == bytes written | a `sizeof` that under-predicts is a renderer heap overflow. This is the classic Venus bug class and it is invisible without an exact-sized buffer. |
| **T3** | every non-padding byte of SPS/PPS influences the wire | a field silently dropped from the schema becomes a non-influential offset and fails against a golden set. |
| **T4** | every truncated prefix is rejected | ASan proves the decoder did not read past the truncation point, which "returned an error" alone does not. |
| **T5** | every 4-byte word replaced with extremes and seeded PRNG values | ~17k decodes. Hits every guest-controlled count, flags word and enum **without needing to know where any of them are**. |
| **T6** | large count under a small budget fails closed | proves the cap is enforced before the allocation, not after. |
| **T7** | both cap emission paths - the `iter_count` loop guard and the `array_size` scalar guard - accepted at the cap and rejected one past it, with the temp pool measured | asserting "decode failed" cannot distinguish a cap that fires *before* the allocation from one that fires after. Measuring the pool can. Exhaustive coverage of the cap *set* is the audit gate's job; T7 proves the two emission shapes behave at runtime. |

Result: **58 checks, 0 failures**, under
`ASAN_OPTIONS=halt_on_error=1 UBSAN_OPTIONS=halt_on_error=1`.

```
VkVideoDecodeH264ProfileInfoKHR                    5 words x 8 patches,   18 accepted
VkVideoDecodeH264SessionParametersAddInfoKHR    1060 words x 8 patches, 8142 accepted
VkVideoDecodeH264SessionParametersCreateInfoKHR 1067 words x 8 patches, 8176 accepted
VkVideoDecodeH264PictureInfoKHR                   24 words x 8 patches,  124 accepted
VkVideoDecodeH264DpbSlotInfoKHR                   12 words x 8 patches,   51 accepted
```

T5 asserts a *defined outcome*, not failure - a corrupted word may describe a
different but still valid payload, and demanding rejection would be wrong. The
real assertion is carried by the sanitizers: no out-of-bounds read, no
undefined behaviour, no allocation past the budget.

#### The harness was mutation-tested before being trusted

A test suite that has never failed has not been shown to work. Two deliberate
defects were injected:

| mutation | caught by |
|---|---|
| unpack shift skewed by one, so encode and decode disagree | T1 (2 failures) |
| `frame_crop_top_offset` deleted from the private XML | T3 - exactly 4 new dead offsets, matching the 4-byte field |
| the `pProfiles` cap deleted from the generator | T7 - "temp pool grew to 544 bytes ... the cap fired after the allocation, not before it" |

Both are green again with the mutations reverted.

The second mutation exposed a **real bug**, now fixed: the private XML was not
listed in meson's `vn_xml_files`, so editing the wire schema did not trigger
regeneration. The build kept compiling stale serializers and no test could have
noticed - precisely the silent-failure class the private-XML approach exists to
prevent. It is recorded here rather than folded quietly into the harness commit
because the *first* thing the new tests did was find something.

| check | status |
|---|---|
| dual-variant compile + link | ✅ driver and renderer |
| append-only command-id ABI gate | ✅ 345 preserved byte-identical, video at 346-358 |
| append-only serialization **layout** gate | ✅ 22 changed functions, all verified purely additive |
| flag pack/unpack, all 5 types exhaustive | ✅ incl. rejection of undefined bits |
| round-trip idempotence, 5 payloads | ✅ T1 |
| `vn_sizeof` vs bytes written | ✅ T2 |
| field-influence coverage | ✅ T3 |
| truncation rejection under ASan | ✅ T4 |
| exhaustive single-word corruption | ✅ T5, ~17k decodes |
| allocation cap fails closed | ✅ T6 |
| array caps fire before allocation | ✅ T7, both emission paths |
| every video-reachable array is capped | ✅ cap-audit gate, 10 arrays |
| `pNext` rejection tests | deferred to W2 - the renderer-side `pNext` dispatch this would test does not exist until `vkr_video.c` does |

## Related but separate: the NVK Vulkan Video work


There is a checkout at `~/projects/mesa` on branch `nvk-vulkan-video` (from
`gitlab.freedesktop.org/dwlsalmeida/mesa`) implementing Vulkan Video in **NVK**,
Mesa's open-source NVIDIA driver. It is **not** part of this prototype and does
not overlap with it:

| | NVK branch | this lab |
|---|---|---|
| files touched | `src/nouveau/**` only | `src/virtio/vulkan/**` (Venus) |
| Venus driver files touched | **zero** | all of them |
| layer | native host Vulkan driver | guest→host virtio-gpu transport |
| wired into `/etc/nixos` | no | no (lab is standalone) |

The only `src/virtio/` paths in its history are incidental CI config files, not
driver code.

The two are **complementary rather than competing**: NVK is a host driver, and
Venus forwards to whatever host driver is present, so NVK-with-video could in
principle sit underneath Venus as an alternative to the proprietary driver.

Two reasons to keep it in mind anyway:

1. **Reference implementation.** `nvk_video_session.c` and `nvk_cmd_video.c`
   implement `VkVideoSessionKHR` lifecycle, DPB slot handling and H.264 decode
   command recording - the same semantics `vkr_video.c` needs. Worth reading
   during the renderer work, particularly the layered-DPB and
   `pic_idx`/`dpb_idx` ordering commits.
2. **Alternative host driver - not needed.** W0 already proved the
   **proprietary** driver decodes H.264 through Vulkan Video on this T1000. If
   the proprietary driver were ever dropped, NVK would be the fallback, but
   swapping it in mid-prototype would add a second unproven variable underneath
   a Venus layer that is itself still being debugged. Its video path also goes
   through nouveau's NVDEC, which is far less mature than the proprietary
   driver's.

### Advertising nothing is not the same as being unreachable

The first cap pass covered the H.264 codec payload and stopped, reasoning that
W1 advertises no video so nothing else is reachable. That reasoning was wrong,
and the W1 panel caught it.

`VkVideoProfileListInfoKHR` chains onto `VkImageCreateInfo` and
`VkBufferCreateInfo`, which **existing** commands already decode. The
renderer's `pNext` dispatch has no extension guard on the *decode* side - the
guard is on the encode side - so from the moment the video extensions enter the
protocol, a hostile guest can place that sType in an ordinary image or buffer
creation and have `pProfiles` decoded with a guest-chosen count. Advertising
nothing only means a *well-behaved* guest would not go there.

Rather than fix the one reported array, the generated renderer was audited for
every array reachable from a video struct or a video command. Three were
uncapped:

| array | cap | grounding |
|---|---|---|
| `pProfiles` | 16 | a profile list names the codecs a resource must be usable with; real lists hold one or two |
| `pReferenceSlots` | 64 | DPB slots. H.264 caps `max_num_ref_frames` at 16; with field pairs, real `maxDpbSlots` is about 17 |
| `pBindSessionMemoryInfos` | 64 | one per memory requirement the driver reports - a single-digit count in practice |

The generalisable lesson: **reachability, not advertisement, defines the attack
surface.** A `pNext` case becomes live as soon as the renderer can decode the
sType, regardless of whether any capset bit invites the guest to send it.

### The cap audit is a gate, because doing it by eye failed twice

The first pass covered the H.264 payload. The second added `pProfiles`,
`pReferenceSlots` and `pBindSessionMemoryInfos`. Both passes missed
`pVideoFormatProperties` and `pMemoryRequirements`, and three reviewers found
them independently.

The reason they were missed is worth stating, because it generalises: they are
**output** arrays. The guest supplies the capacity it wants filled and the data
flows back the other way, so they do not look like attack surface - but the
count is still guest-chosen, and the renderer still allocates it before writing
anything.

`tests/video-array-cap-audit.sh` now enumerates every
`vn_cs_decoder_alloc_temp_array` reachable from a video command or video struct
in the *generated* renderer and fails if any lacks a cap in the lines above it.
Ten arrays today, all capped. It checks the ordering, not just the existence of
a limit - a cap that fired after the allocation would still fail.

| array | cap | grounding |
|---|---|---|
| `pStdSPSs` | 32 | `seq_parameter_set_id` is 0..31 |
| `pStdPPSs` | 256 | `pic_parameter_set_id` is 0..255 |
| `pOffsetForRefFrame` | 255 | length is a `uint8_t` field |
| `pSliceOffsets` | 65536 | far above any real picture; bounds the allocation to 256 KiB |
| `pProfiles` | 16 | real profile lists hold one or two |
| `pReferenceSlots` | 64 | H.264 caps `max_num_ref_frames` at 16; real `maxDpbSlots` ≈ 17 |
| `pBindSessionMemoryInfos` | 64 | one per driver-reported memory requirement |
| `pVideoFormatProperties` | 256 | a handful of formats per profile in practice |
| `pMemoryRequirements` | 64 | single digits per session |

Caps are emitted only where storage is actually allocated. The other path is
the *driver* decoding a reply from the renderer into a buffer the guest itself
supplied: no allocation to bound, and the count comes from the host rather than
the guest. Emitting a cap there was both pointless and a compile error, since
the reply helpers return `VkResult` and the cap emits a bare `return`.

That last part only surfaced because `protocol-checks` **builds** the fork
rather than only regenerating it. The cap audit passed with all ten arrays
capped while the driver would not compile.
