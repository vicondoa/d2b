# W3 - virglrenderer: harden the video decode path

Revision 4. Rev 1: **0/7, 25 findings**. Rev 2 answered them but was never
reviewed and predated any execution. Rev 3 rebased rev 2 onto measured fact and
returned **0/7 with 21 findings**. Rev 4 answers those; the full disposition,
including two disputed findings, is in `docs/w3-panel-round1.md` and reviewers
should read it alongside this plan.

Base: lab `efa4b2df`, virglrenderer `114e0304` (spike), Mesa `0f06521a` (spike).

## 0. What changed since revision 2, and why it matters

Rev 2 was written while nothing in the plan had ever run. A feasibility spike
has since executed the whole path, and four of its findings contradict or
substantially refine what rev 2 assumed. **Reviewers should read
`docs/spike-findings.md` before this document.**

The headline: **it works.** Stock FFmpeg in the guest decodes H.264 on the host
T1000 through Venus - `pix_fmt: vulkan`, 90 frames, 0 decode errors, negative
control clean. So W3 is no longer "make video work carefully"; it is "harden a
path that is known to work, without breaking it."

That is a materially better position than rev 2 planned for, and it changes the
risk profile in one specific way worth stating: **every hardening change from
here can be regression-tested against a working decode.** Rev 2's §6 asked for
negative controls that were hermetic-only. They are now runtime-checkable.

### Rev 2 claims the spike corrected

| Rev 2 said | Measured |
|---|---|
| §8 runtime execution is a hard exit criterion, currently unmet | **Met.** Full create/bind/begin/decode/end/destroy round trip returning VK_SUCCESS, plus real frame output. |
| §2 "out of scope: Mesa" - W3 is renderer-only | Renderer-only is **not sufficient**. Mesa's entrypoint generator emits weak references, so advertising an extension without implementing all 13 entrypoints yields NULL dispatch slots, not a build error. The spike had to land Mesa work to reach a decode at all. W3 and W4 are no longer cleanly separable; see §12. |
| §4.6 generated default-deny validators are W3 work | **Already landed** during the spike, because hand-editing the generated header would have discarded it. W3 inherits and extends it. |
| §4.2 "reject any query whose profile is not H.264 decode" | Still right, but the spike shows capability queries succeed and return real host numbers *before* anything else fails. Profile filtering is therefore not on the critical path to a working decode, and must be added without breaking the working one. |
| §3 the enforcement gate's `gated` bucket goes to zero for decode | Still the goal, but the spike changed the denominator: `SUPPORTED_VIDEO_EXTENSIONS` removed decode values from the reject surface entirely, so many rows no longer describe a rejection at all. The pin needs re-deriving, not just re-splitting. |

### What the spike found that rev 2 did not anticipate

Two of these are defects rev 2's work items would not have caught.

1. **Venus caches queue family properties** and special-cases exactly one
   chained struct. Anything else an application chains is silently never
   filled. The failure shape is instructive and generalises: the guest saw the
   decode queue bit with an empty codec list, and an application correctly
   refuses a video queue that decodes nothing. **W3/W4 must ask what else Venus
   caches**, rather than treat this as a one-off.
2. **W2's rejection surface refused the DPB image every decoder must
   allocate**, and this was structurally invisible: no guest could reach the
   rejection while the capset was masked. That is a general property of
   "advertise nothing" hardening - it cannot be validated against real use - and
   it is the strongest argument in the record for why W3's validators need
   runtime negative controls rather than hermetic ones alone.

## 1. What W3 is

W2's safety rested on one fact: video commands had no dispatch entry. That is
now false. The spike wired all thirteen, advertised three extensions, flipped
the capset, and turned the reject surface into a supported-set intersection.

**W3 is the wave that makes that defensible.** The spike trusts the guest; W3
must not.

| | W2 | spike | W3 |
|---|---|---|---|
| a video value arrives | reject | forward | **validate**, then forward |
| a video capability is queried | scrub | passthrough | passthrough for the supported profile, typed rejection otherwise |
| decode outside coding scope | unreachable | reaches the driver (UB per spec) | rejected in the renderer |
| a reference slot | unreachable | forwarded unchecked | membership + range checked |
| session count | unreachable | unbounded | capped per context |
| handle replacement failure | unreachable | can forward mixed handles | two-pass, nothing forwarded on failure |

## 2. Scope boundary

**In scope**: validation of everything the spike forwards unchecked; object
lifetime and pin classes; coding-scope tracking; aggregate resource bounds; the
gate transition; runtime negative controls.

**Out of scope**: decoder logic, V4L2, NVDEC/NVENC, bitstream parsing. The
renderer forwards; the host driver decodes. Encode stays rejected and scrubbed
- the spike preserved this and the generator reports 28 encode values still
rejected.

**No longer out of scope**: Mesa. See §12.

## 3. Gate transition

W2's `video-exposure-gate.sh` and `video-capset-gate.sh` assert the renderer
advertises nothing. Both are now false against the tree, so both must invert in
the same commit that makes them true - but rev 2's "invert, do not delete" still
holds, and for a sharper reason than rev 2 gave: the encode assertions in both
gates are the only mechanical statement that encode stayed closed while decode
opened. Deleting them would remove the check exactly when it starts mattering.

- `video-exposure-gate.sh` inverts: enabled video extensions are **exactly** the
  decode allowlist, each has a dispatch entry, and every encode extension is
  still absent and still undispatched. The encode assertion survives verbatim.
- `video-capset-gate.sh` inverts the same way. Note the spike replaced the
  hardcoded clear-list with a derivation over `_vn_info_extensions`, so the gate
  must now assert the derivation's *outcome*, not the presence of a literal list.
- **The enforcement pin must be re-derived, not re-split.** Rev 2 proposed
  `--expect-gated-decode 0` / `--expect-gated-encode N`. That no longer
  describes the tree: `SUPPORTED_VIDEO_EXTENSIONS` removed decode values from
  the reject surface, so decode rows are not "gated", they are *absent*. Re-run
  the manifest derivation and pin what it actually produces, with the delta from
  189 explained row by row.

## 4. Work items

### 4.1 Object model and lifecycle

**The spike's object graph is wrong and must be reverted.** It models sessions
as owning their parameters objects via `base.track_head`, with
`vkDestroyVideoSessionKHR` cascading - the `vkr_descriptor_pool` precedent.
`gpu` filed this as critical in round 2 and is right; `c`'s round-1 finding
that a session implicitly destroys its parameters is the opposite claim and is
withdrawn against the evidence below.

Video session parameters are **not** implicitly destroyed with their session.
Mesa's own common implementation settles it: `struct
vk_video_session_parameters` (`src/vulkan/runtime/vk_video.h:127`) holds no
back-pointer to a session, and `vk_video_session_parameters_destroy` unlinks
from no parent list. If the relationship were owning, the reference
implementation would have to track it in order to free them, and it does not.
Parameters and sessions are siblings owned by the device.

The cascade is not merely unnecessary, it is harmful in two ways. It frees the
renderer's record **without** calling `vkDestroyVideoSessionParametersKHR`, so
the host object leaks; and a conformant guest that destroys its parameters
after the session then looks up a record that is already gone. Remove the
cascade and the `parameters` list, and track parameters as ordinary
device-owned objects.

**Register the objects in `vkr_device_object.json`.** The spike hand-wired
create/destroy. `mesa-venus` is right that this bypasses the generator that
supplies dispatch registration, object tracking and base cleanup, so
device-teardown reaping is not inherited and the fork diverges from the
convention any upstream submission would be judged against. Video sessions are not `simple-object`, but the reason is **bound
memory only** - they have no Vulkan child objects, since parameters are
siblings (see above). Rev 4 said "children and bound memory", which contradicted
its own withdrawal of the cascade and would have led a generator variant to emit
tracking and reaping for children that do not exist (`c`, round 2). The variant
needs to model the bind-then-use lifecycle and nothing else. Decide between a
generator variant and a recorded justification for the hand-written form; do not
leave it hand-wired by default.

Still missing:

- **Bound memory tracking.** The spike does not record what was bound, so
  liveness is not checkable at destroy.
- **`updateSequenceCount`.** Guest value must be **strictly greater than** the
  stored value before forwarding; store the new value on host success, leave it
  unchanged on host failure, so renderer and host cannot desync.

  Rev 3 required exactly `stored + 1`, and I disputed `security`'s round-1
  finding against it (D2). `gpu` raised the same finding independently in round
  2, citing the VUID. Two independent reviewers converging outweighs my
  unverified reading, so **D2 is resolved against me**: exact-increment would
  reject a conformant client whose counter advances by more than one.
- **Aggregate resource bounds.** Per-array caps bound one command; they do not
  bound session count, bound memory, or DPB images, and the spike lets a guest
  loop `vkCreateVideoSessionKHR`. Enforce before forwarding: a per-context live
  session cap, and per-session parameter-set bounds.

  **The parameter-set bound is CAPACITY, not a lifetime total.** Rev 3 said
  "accumulated SPS <= 32 and PPS <= 256", which `gpu` and `security`
  independently identified as wrong. Those numbers are the size of the H.264 id
  space (5-bit `seq_parameter_set_id`, 8-bit `pic_parameter_set_id`), so they
  bound how many can be *live at once*. A long-running stream legitimately
  rotates parameter sets through the same ids via
  `vkUpdateVideoSessionParametersKHR`, so a monotonic lifetime counter would
  reject conformant streams after enough rotations -- a bug that only appears
  in long playback, which is exactly the case a short test clip misses. Bound
  the session's declared `maxStdSPSCount` / `maxStdPPSCount` capacity and the
  live set, never a running total.

  The 1 GiB ring pool is **not** a backstop: it bounds
  `vn_cs_decoder_alloc_temp_array` during parsing, not `VkDeviceMemory` bound to
  sessions.
- **Record pins vs submission pins**, with distinct lifetimes:

  | Pin class | Taken when | Released when |
  |---|---|---|
  | record | an object is referenced by a *recorded* command | `vkResetCommandBuffer`, re-`Begin`, `vkFreeCommandBuffers`, `vkResetCommandPool`, `vkDestroyCommandPool`, device/context teardown |
  | submission | the command buffer is submitted | the retirement fence for that submission signals |

  Deferring destruction to fence retirement alone is wrong twice over: a
  recorded-but-never-submitted buffer never retires, so pins leak forever; and
  an executable buffer can be resubmitted after a fence retires, so record pins
  must survive first submission.

- **Cross-context handle rejection.** Object-table lookup is context-scoped, so
  a handle from context A submitted on context B should fail at lookup. W3's
  first task is to **verify this in the existing table implementation and record
  the result**, not to assert it.

### 4.2 Capability queries

The spike passes host capabilities through unmodified, and the measured
capability block is exactly the correct answer: max level 52, 4096x4096, 17
references, `reuse_dst_dpb`. Rev 2's argument against field-level filtering is
confirmed by measurement - under-reporting any of these makes FFmpeg select a
lower tier or refuse streams the hardware handles.

So: **profile-level filtering only.** Reject a query whose profile is not H.264
decode with a proper `VK_ERROR_VIDEO_PROFILE_OPERATION_NOT_SUPPORTED_KHR`
rather than a generic failure. For a valid H.264 decode query, pass the host's
numbers through untouched.

For `vkGetPhysicalDeviceVideoFormatPropertiesKHR`, a **pinned format allowlist**
containing at minimum `VK_FORMAT_G8_B8R8_2PLANE_420_UNORM` - measured as the
one format FFmpeg chose (`nv12`, Vulkan ID 1000156003).

**The allowlist is enforced at `vkCreateImage`, not only at the query**
(`gpu`). Rev 3 filtered what the query reports and left creation unchecked, so
a guest that ignores the query could create a decode DST or DPB image in an
arbitrary format and have it forwarded. Filtering a query the guest is free not
to read is advice, not enforcement: the check belongs where the resource is
actually created.

### 4.3 Command-buffer decode path

**Coding-scope tracking.** Decoding outside a video coding scope is *undefined
behaviour* per spec, not a guaranteed error return; on a proprietary driver
that can mean device-lost or silent corruption. The spike forwards these
unchecked. Per-command-buffer scope state, default *outside*; `Begin` sets,
`End` clears; `vkCmdDecodeVideoKHR` and `vkCmdControlVideoCodingKHR` rejected
unless inside; nested `Begin` rejected.

**And the scope must be closed at `vkEndCommandBuffer`** (`security`). Rev 3
tracked entry and exit but never asserted termination, so a guest could record
`Begin` with no matching `End` and submit a buffer that leaves the host driver
inside an unterminated coding scope. Tracking the state without checking it at
the one point the recording becomes final is the gap that makes the rest of the
tracking decorative.

**DPB integrity**, four invariants:

1. **Membership.** Every `pReferenceSlots[i].pPictureResource->imageViewBinding`
   is in the session's bound DPB set. **Also `pSetupReferenceSlot`** whenever
   its `slotIndex != -1` -- rev 3 exempted the setup slot entirely, which
   `security` correctly flagged as a hole: a setup slot with a real index and an
   unverified image is a write target, and exempting it lets decoded output land
   on an image the session never bound.
2. **Range.** `slotIndex` is within the session's `maxDpbSlots`, for the setup
   slot as well as `pReferenceSlots[]`. Additionally `referenceSlotCount` is
   itself bounded by `maxDpbSlots`; rev 3 bounded the indices but not the count.
3. **Spatial bounds** (`gpu`, critical -- rev 3 had none). The bitstream range
   must lie within the bound buffer, and `codedOffset + codedExtent` within the
   dimensions of every picture resource it addresses.

   **Compute the bitstream bound without overflowing** (`c`, round 2).
   `srcBufferRange` may be `VK_WHOLE_SIZE` (`~0ULL`), so the obvious
   `srcBufferOffset + srcBufferRange <= size` wraps in 64-bit arithmetic and
   the check passes for exactly the input it exists to reject. Resolve
   `VK_WHOLE_SIZE` against the buffer size first, then compare as
   `offset <= size && range <= size - offset` - subtraction on values already
   known in range, which cannot wrap. The same shape applies to the extent
   check.

   **Bounding the range does not bound offsets INTO the range** (`security`,
   round 2). `VkVideoDecodeH264PictureInfoKHR::pSliceOffsets` is an array of
   guest-chosen byte offsets into the bitstream, and the generator sees its
   elements as ordinary `uint32_t` scalars - nothing marks them as offsets, so
   nothing checks them. W1's `ARRAY_COUNT_LIMITS` caps `pSliceOffsets` at 65536,
   which bounds how many there are and says nothing about their values. A guest
   can therefore pass a conformant `srcBufferRange` and a slice offset beyond
   it, and the host NVDEC engine starts reading outside the range this invariant
   just validated. Require every element to be strictly less than the resolved
   `srcBufferRange`, and `sliceCount` to be bounded by it. Generated code cannot infer these because handle sizes are
   opaque to it, so this is an explicit renderer-side check against the tracked
   object. Forwarding an oversized range lets a guest drive the host NVDEC
   engine out of bounds, whose failure mode is a host GPU fault rather than an
   error return.
4. **Aliasing.** `dstPictureResource` does not alias an active reference slot
   unless the profile reports
   `VK_VIDEO_DECODE_CAPABILITY_DPB_AND_OUTPUT_COINCIDE_BIT_KHR`.

**Invariant 4 needs care in both directions.** The measured host reports
`reuse_dst_dpb`, i.e. coincide *is* set, so on this hardware aliasing is legal
and a check that assumed the common case would reject every decode on the only
GPU the lab has. Rev 3 asked (Q2) whether to implement the non-coincide branch
at all given it cannot be exercised here. **It is implemented**, because `gpu`
and `security` independently supplied the same way to exercise it: mask
`reuse_dst_dpb` out of the capability reply to force the guest down the
non-coincide path. That turns an untestable branch into a testable one, which
was the entire objection.

**Negative slot indices.** `pSetupReferenceSlot` may be NULL, and
`slotIndex == -1` legitimately means "do not retain in the DPB" for a
non-reference picture, which is ordinary in High-profile B-frames. NULL and -1
are therefore accepted without a binding check -- but, per invariant 1, a setup
slot with any other index is checked exactly like a reference slot.

**No image-layout validation.** Layout correctness is the host driver's job.
Venus forwards command buffers as opaque recordings and cannot track barriers
without a state machine that would be both wrong and expensive. Stated
explicitly so nobody implements one.

**Handle replacement unwind.** Replacement is destructive, so a failure partway
leaves a struct holding a mix of host and guest handles, and forwarding that is
UB. Two-pass: validate every lookup, then replace. On failure nothing is
forwarded and the reply preserves `sType`/`pNext` verbatim - the W2 finding
where a zeroed `sType` turned a rejection into a guest-triggerable host assert.
Whether the dispatch lock is held across both passes **must be established and
recorded**, since otherwise two-pass has a TOCTOU window.

`pReferenceSlots[].pPictureResource->imageViewBinding` is a handle reached
through an array of pointer chains - a replacement obligation, not merely a
validation one. Confirm the generator walks it; fix the generator if not.

### 4.4 Queue-family invariant

The spike proved the mapping works (host decode family surfaces as guest family
3 and FFmpeg selects it). W3 must ensure it cannot silently stop working: never
map a guest video queue onto a host queue the host does not report as
video-capable, and keep `VkQueueFamilyVideoPropertiesKHR.videoCodecOperations`
truthful.

### 4.5 Derive the nested tables

`NESTED_ARRAYS` and `NESTED_REFS` remain hand-declared. Derive them:
`structextends` identifies chaining, `len` identifies counted pointers, and the
generator already parses both. Walk every struct reachable from a video command
arg through pointer members and `structextends`; a pointer with `len` emits an
array walk, a pointer without `len` to a struct carrying video values or handles
emits a dereference.

Diff the derived table against the hand-written one **for the non-video types**.
Any delta is a bug in one side or the other, and finding out which costs one
diff. The gate then asserts the derived set is a superset of the hand-written
one and fails on any vk.xml-reachable pointer left unwalked.

Two defects `c` found in the existing generator, both of which the derivation
must not inherit:

- **The array walk does not recurse into elements' pNext chains.** The
  generated loop validates scalar members of `pAttachmentImageInfos[i]` but
  never calls the pNext walker on the element, so a chained struct one level
  inside an array element is unreachable by the rejection surface. Since the
  whole point of the walker is that video values arrive from the side, an
  element-level chain is not an exotic case.
- **Deferred pointer members fail OPEN.** The generator currently identifies
  members needing an element walk, prints them to stderr, and emits nothing --
  five members today, including two `pCopySrcLayouts`/`pCopyDstLayouts` pairs.
  A warning is not a boundary. Either emit the walk or **fail the build**; a
  default-deny surface that silently omits a member it knows about is
  default-deny in name only.

### 4.6 Generated default-deny validators - LANDED

Already in the tree. `SUPPORTED_VIDEO_EXTENSIONS` is the single hand-written
input; every value, struct, pNext arm and scrub mask derives from intersecting
vk.xml against it. Measured outcome: 0 decode values rejected, 28 encode values
rejected.

W3 extends it with the three subtleties the spike had to get right, all of
which are already implemented and should be **reviewed rather than designed**:
laundering prevention, empty-set predicates still emitted, and `FlagBits2`
propagation by bit position.

Remaining W3 work here:

- Emit **positive validators** for supported values, not just absence of
  rejection. Today a supported value is simply not rejected; nothing checks it
  is *correct* for the site it arrived at.
- **Emit the supported set for C to consume** (`mesa-venus`). The spike states
  it twice -- `SUPPORTED_VIDEO_EXTENSIONS` in the generator, and a separate
  `vkr_supported_video_exts` literal in `vkr_renderer.c` driving the capset
  derivation. Two hand-maintained copies of the same list is precisely the
  shape W2 spent 23 rounds proving this codebase gets wrong, and here the two
  copies control *advertisement* and *acceptance* respectively: drift between
  them means advertising an extension whose values are rejected, or the
  reverse. The generator must emit the list and the C must consume it, so
  there is one source.

### 4.7 StdVideo content bounds (new)

W1 deep-copies every StdVideo field, so no guest pointer crosses the wire (see
the D1 rebuttal in `docs/w3-panel-round1.md`). But deep-copying a value is not
validating it, and `security` is right that the *contents* were unchecked.

Bound the guest-controlled indices against their spec ranges before forwarding:
`seq_parameter_set_id` 0-31 (5-bit), `pic_parameter_set_id` 0-255 (8-bit), and
the DPB-adjacent counts the host driver will use to index its own arrays.
Derive the bounds from the private XML where it states them rather than
restating the numbers, so the field description stays the single source.

### 4.8 Guest-side parameters lifetime - withdrawn

Rev 4 carried this from `c`'s round-1 finding that destroying a video session
implicitly destroys its parameters objects, so the guest leaks its
`vn_video_session_parameters` allocations.

**The premise is false** (see §4.1). Parameters are not implicitly destroyed,
so a conformant guest destroys each one itself and nothing leaks. Adding
tracking to free them at session destroy would free objects the host still
holds - the same defect as the renderer cascade, on the other side of the wire.

Item withdrawn. `c` is asked to judge the §4.1 evidence on the merits;
withdrawal of the original finding is permitted but not required.

### 4.9 Explain the teardown error (new, bounded)

`CtxDetachResource: ErrRutabaga(InvalidContextId)` appears in the crosvm log,
predates the spike, and does not prevent decode. Rev 3 proposed deferring it to
W8; `test`, `virtualization` and `security` all objected on the same ground,
and it is a good one: W3 is the wave that defines object lifetimes, pins and
destroy cascades, so an unexplained error on the resource-teardown boundary
could mask exactly the reference-count leak W3 claims to prevent.

**Bounded commitment: explain it, or document why it is benign.** Fixing it is
not a W3 deliverable -- it lives in crosvm/rutabaga, outside the three forks --
but closing W3 while an unexplained teardown error sits on the same boundary as
W3's central claim is not acceptable either.

### 4.10 Venus cache audit (new, mandatory)

The spike found `vn_GetPhysicalDeviceQueueFamilyProperties2` serving from a
cache that special-cases exactly one chained struct, so
`VkQueueFamilyVideoPropertiesKHR` was silently never filled. Rev 3 asked (Q3)
whether auditing Venus's other caches was W3, W4 or separate.

**It is W3.** The deciding argument is `security`'s: a cache that drops a
chained struct is a state desynchronisation between what the guest asked for
and what the host answered, and W3's validators reason about that state. A
guest able to induce a drop can make a validator decide on stale data.

Audit every Venus path that answers from a cache rather than forwarding, and
for each one record whether it can drop an application-chained struct. The
queue-family cache is one instance; the finding is that the *pattern* exists,
not that this was the only occurrence.

## 5. Ordering

```
7 step 0: off the spike branches, relock, refresh PINS
   │
   ├─→ 4.5 derive nested tables ─→ 4.1 objects ──┬─→ 4.3 command path ──┐
   │                                             │   (+ 4.7 StdVideo)   │
   ├─→ 4.6 positive validators ──→ 4.2 caps ─────┴─→ 4.4 queue family ──┼─→ 3. gate transition
   │
   ├─→ 4.8 guest params lifetime      (independent)
   ├─→ 4.9 explain teardown error     (independent, bounded)
   └─→ 4.10 Venus cache audit         (independent; feeds 4.2/4.4 if it finds one)
```

Step 0 comes first because every later build resolves from the lock, so
hardening committed against the spike branches would be validated against the
wrong tree - the W1 lesson.

Gate transition last, so gates are inverted only once what they assert is true.

4.8, 4.9 and 4.10 are independent and can run in parallel with the main chain.
4.10 is drawn feeding 4.2 and 4.4 because if the audit finds a second dropped
chained struct, that is where it would land.

## 6. Verification

The spike changes what is possible here, and the plan should use it.

- Every new or inverted gate **observed to fail** a mutation before being
  pinned, and the mutation **asserted to have actually applied** - W2 had four
  malformed mutations, two of which impersonated a pass.
- **Negative controls for every validator, run at runtime.** This is the
  asymmetry that matters: a no-op *rejecter* is caught by a positive control,
  but a no-op *validator* - one accepting everything - is caught only by a
  negative control. Rev 2 could only offer hermetic ones. Now every validator
  ships with a structurally-invalid input it must reject, and commenting out the
  check must make that control fail. Minimum for §4.3: a cross-context handle, a
  stale handle, a reference slot outside the session's DPB, a decode outside
  coding scope, an unterminated coding scope at `vkEndCommandBuffer`, and an
  out-of-range `srcBufferRange`.

  Round 2 added three checks to §4.3, and `test` is right that adding a check
  without adding its control is exactly what this section exists to prevent.
  Each needs its own control because each fails independently of the others:

  - **A `srcBufferRange` chosen to wrap in 64-bit arithmetic**, not merely one
    that is too large. A generic out-of-range input still passes if the
    overflow-safe formulation regresses to `offset + range <= size`, so only an
    input designed to wrap can detect that regression.
  - **An out-of-bounds `pSliceOffsets` element** with a conformant
    `srcBufferRange`. This is a distinct vector; a control that only oversizes
    the range never reaches the slice-offset check.
  - **An out-of-DPB image in `pSetupReferenceSlot` with `slotIndex >= 0`.** Rev
    3 exempted the setup slot entirely, so a regression re-exempting it would be
    invisible to a control that only ever puts the invalid handle in
    `pReferenceSlots[]`.
- **Negative controls for the §4.1 aggregate bounds too** (`test`). Rev 3
  required them for §4.3 and omitted §4.1, which leaves the session cap, the
  parameter-set capacity bound and the sequence-count check as limits never
  observed rejecting anything. A limit never seen to fire has not been shown to
  work - the same standard W2 established when it refused to pin a gate never
  observed failing.
- **A control for the two-pass handle-replacement unwind** (`test`). The unwind
  is the least-exercised path in §4.3 and the one whose failure forwards a
  struct holding a mix of guest and host handles. Feed a partially invalid
  handle array, force the unwind, and assert nothing was forwarded and the reply
  preserved `sType`/`pNext` verbatim.
- **A working-decode regression test is the primary gate.** Every hardening
  change must keep `pix_fmt: vulkan` with 0 decode errors. This is the single
  most valuable check W3 has and it did not exist before the spike.
- **The enforcement gate must credit validator correctness, not existence.**
  "Enforced" cannot mean "a function is wired", or a validator returning true
  unconditionally scores as enforced.

## 7. Generator and vendoring discipline

**Step 0, before any hardening: get off the spike branches.** The lab flake is
currently locked to the throwaway `spike-video` branches of the virglrenderer
and Mesa forks (`nixos`). W3 lands on `vulkan-video`, so the first commit
retargets both inputs, relocks, and refreshes `PINS.md` - which is stale
against the current lock and will fail `pins-check.sh` until regenerated.
Record the Vulkan-headers version there too; it is missing today, and the lab
builds both patched packages against it.

Also confirm the whole-tree `policy_cli_consumers.rs` scan stays green with the
lab's new apps and scripts present, since it is the one root gate that sees
`labs/`.

`vkr_video_reject.h` -> `vkr_video_validate.h` must be **one atomic commit**
covering the generator, the vendored header, `generator-drift-check.sh`'s
hardcoded `vendored=` path, and the `protocol-checks` staging line.
`header-sync-check.sh` does **not** cover this header - it covers
`vn_protocol.py` output - so `generator-drift-check.sh` is the sole guard and
must not have a gap.

Lock-update ordering, because the generator lives in the lab and its vendored
output in a fork: push venus-protocol -> `nix flake update venus-protocol-src`
-> regenerate and commit the vendored header in virglrenderer -> push ->
`nix flake update virglrenderer-src` -> commit the lock -> run
`protocol-checks`. The middle steps are the window where drift is invisible to
`nix build`, which resolves from the lock rather than the working tree. The
spike followed this and it worked; it is written down so it keeps working.

## 8. Runtime verification - met, and now a regression gate

Rev 2 made runtime execution a hard exit criterion because four reviewers said a
never-executed W3 must not close. **That criterion is met.** The measured round
trip is `vkCreateVideoSessionKHR` -> `vkGetVideoSessionMemoryRequirementsKHR` ->
`vkBindVideoSessionMemoryKHR` -> `vkCreateVideoSessionParametersKHR` ->
`vkCmdBeginVideoCodingKHR` -> `vkCmdDecodeVideoKHR` -> `vkCmdEndVideoCodingKHR`
-> destroy, with real frame output and a clean negative control.

The criterion therefore changes shape: it stops being an exit gate and becomes
a **regression gate** that every W3 commit must hold.

## 9. Withdrawn: the guest-observability dispute

Rev 2 §9 disputed a `mesa-venus` HIGH claiming W3 becomes observable from an
unmodified guest Mesa once the capset flips. Rev 2 argued
`vn_physical_device_get_passthrough_extensions()` reads a hardcoded static
allowlist, not the capset, so an unmodified Mesa cannot expose video regardless.

**That analysis was correct and the dispute is moot.** The spike confirms it
directly: advertising in the renderer changed nothing in the guest until Mesa's
passthrough table was edited *and* all thirteen entrypoints were implemented.
Rev 2 was right about the mechanism and, if anything, understated it - the
weak-reference behaviour means even editing the table is insufficient.

Both qualifications rev 2 accepted still stand and are now load-bearing rather
than precautionary: the margin protects against *unmodified upstream Mesa*, not
against a hostile or custom Venus client, which can send any command id. §4.3
and §4.6 are written to that standard.

## 10. Compatibility matrix

`VN_WIRE_FORMAT_VERSION` stays 1; video is gated purely by the capset, which is
safe only because the ABI is append-only (abi-gate: 345 ids byte-identical).

| guest | renderer | outcome |
|---|---|---|
| old (no video) | old (no video) | unchanged |
| old (no video) | **new** | the guest's passthrough table has no video entries, so exposure fails and video is never used. Capset bits are ignored by a guest with no code referencing those numbers. **Confirmed by the spike**, which observed exactly this state before Mesa changed. |
| **new** | old (no video) | renderer advertises nothing, guest degrades to "video unavailable". W4 must handle a missing capset bit without mis-reporting; a W4 gate asserts it. |
| **new** | **new** | the intended path, measured working. |

## 11. Questions resolved in round 1

Kept for the record; the reasoning is in `docs/w3-panel-round1.md`.

- **Q1 - Firefox's `HARDWARE_VIDEO_DECODING` blocklist.** Unanimous: a W6
  concern. FFmpeg does not consult it, so nothing in W3 depends on it.
- **Q2 - the non-coincide DPB branch.** Implement it, and test it by masking
  `reuse_dst_dpb` out of the capability reply. Two reviewers proposed the same
  method independently, which answers the objection that the branch was
  untestable on this hardware.
- **Q3 - auditing Venus's caches.** W3, not W4. A dropped chained struct is a
  state desync that W3's own validators reason over. Now §4.10.
- **Q4 - `CtxDetachResource`.** Investigate in W3 with a bounded outcome:
  explain it, or document why it is benign. Now §4.9.

## 12. Scope correction: W3 and W4 are no longer separable

Rev 2 scoped W3 as renderer-only with Mesa deferred to W4. The spike shows that
does not hold: a renderer-only W3 cannot be runtime-verified, because nothing in
the guest can reach the code without Mesa's entrypoints and passthrough table.
Since §8 makes runtime verification a regression gate on every commit, W3
necessarily carries the Mesa changes it needs to execute.

**Settled:** W3 covers renderer hardening **plus the Mesa work required to
execute and validate it**, named explicitly rather than left as "the minimum"
(`mesa-venus`, rounds 1 and 2):

- the 13 `vn_*` video entrypoints and the two object wrappers;
- the three passthrough-table entries;
- the **format-feature un-strip** - restoring
  `VK_FORMAT_FEATURE_VIDEO_DECODE_{OUTPUT,DPB}_BIT_KHR` on
  `VK_FORMAT_G8_B8R8_2PLANE_420_UNORM` only, leaving P010/P012 and every encode
  bit stripped;
- the **queue-family video properties cache**, so
  `VkQueueFamilyVideoPropertiesKHR` is actually filled;
- the **cache audit** (§4.10).

All but the audit already landed in the spike and are being re-landed on
`vulkan-video` rather than rewritten.

W4 keeps Mesa *hardening*: fail closed when the renderer or host lacks support,
and the compatibility cross-product from the `base/<rev>` tags. The cache audit
moved to W3 by the Q3 resolution and is **not** W4 work; rev 4 left it listed in
both places, which `mesa-venus` flagged as a direct contradiction.

This remains the one structural change to the wave graph. It was unanimously
accepted in round 1.
