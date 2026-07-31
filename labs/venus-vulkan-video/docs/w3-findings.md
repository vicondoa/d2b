# W3 - hardening the decode path

**Status: the validation surface is landed and the decode still works.** Four
work items remain open and are named at the bottom.

## The regression gate, which is the point

Every hardening change had to keep this true, and it does:

| | Before hardening | After |
|---|---|---|
| guest `ffmpeg -hwaccel vulkan` | `pix_fmt: vulkan`, 90 frames, 0 errors | **unchanged** |
| Firefox on the local corpus | +512 decode commands, +3 sessions | **unchanged** |

That gate did not exist before the spike. W2 closed with 14 hermetic gates and
nothing runtime-verified; every W3 change is now checked against a decode that
actually runs.

## What landed

### Object model - the spike's model was wrong

Sessions and parameters objects are **siblings owned by the device**, not
parent and children. The spike cascaded on destroy, following the
`vkr_descriptor_pool` precedent.

That cascade was harmful rather than merely redundant: it freed the renderer's
record **without** calling `vkDestroyVideoSessionParametersKHR`, leaking the
host object, and a conformant guest destroying its parameters afterwards found
a record already gone.

Settled against Mesa's own common implementation rather than by argument:
`struct vk_video_session_parameters` (`src/vulkan/runtime/vk_video.h:127`)
holds no back-pointer to a session, and its destroy unlinks from no parent
list. A reference implementation would have to track the relationship in order
to free them, if the relationship were owning.

### Coding scope

Decoding outside a video coding scope is **undefined behaviour** per spec, not
a guaranteed error return, so on a proprietary driver it can mean device-lost
or silent corruption. The renderer is the enforcement point.

Default is *outside*, so a path that forgets to maintain the flag fails closed.
Nested `Begin` is rejected. The scope is a boolean rather than a depth counter,
because the spec has no nesting to model and treating a second `Begin` as
"depth 2" would invent semantics the driver does not implement.

Checked at `vkEndCommandBuffer`, and cleared on both `vkResetCommandBuffer` and
re-`Begin`. Tracking entry and exit without checking termination leaves the
tracking decorative; not clearing on reset would let a stale scope carry into a
recording that never opened one.

The session is threaded through the scope as an **object id, not a pointer**.
Nothing stops the guest destroying the session while a command buffer still
references it, and the decode path would read freed memory to find the DPB
limits. An id costs one lookup and makes a destroyed session a clean rejection.

### DPB integrity

`referenceSlotCount` is bounded by `maxDpbSlots`, because bounding the indices
says nothing about how many there are. Every reference slot is range- and
membership-checked, **and so is `pSetupReferenceSlot` when its index is
non-negative** - a setup slot with a real index is a write target, and the
spike exempted it entirely, which let decoded output land on an image the
session never bound.

A negative index means the resource is *ignored* and may be uninitialised, so
validation and handle replacement both skip it rather than dereferencing
whatever is there and rejecting ordinary B-frame content.

### Bounds, caps and sequencing

- Bitstream range resolves `VK_WHOLE_SIZE` before comparing, and compares by
  subtraction. `offset + range` wraps in 64-bit arithmetic and the naive check
  passes for exactly the input it exists to reject.
- Live sessions capped per context, checked before allocating or forwarding.
- `updateSequenceCount` must be **strictly greater**, not exactly one more,
  which would reject a conformant client whose counter advances by more. It
  advances only on host success.
- Format and usage allowlists on the video format query, enforced on the
  inbound usage and the outbound reply.

## Gates

Both W2 "advertise nothing" gates were **inverted, not deleted**. Deleting them
would have removed the encode assertion at precisely the moment decode went
live and encode became the only thing standing between the guest and an
unimplemented surface.

| Harness | Checks | Mutations observed firing |
|---|---:|---:|
| `video-validate-controls.c` (new) | 41 | 9 |
| `scrub-controls.c` (inverted) | 59 | 4 |
| `video-exposure-gate.sh` (inverted) | - | 4 |
| `video-capset-gate.sh` (narrowed) | - | 2 |
| `uncovered-dispatch-gate.sh` (taught) | - | 2 |

All 14 protocol gates green.

## Four things this wave got wrong, and how they were caught

Each was caught by mutation testing, and each would have shipped as a green
signal that measured nothing.

### 1. I built an inert gate

Teaching `uncovered-dispatch-gate.sh` to recognise allowlist guards made it
green. Then the mutation: deleting the guard entirely, leaving zero occurrences
in the source. **The gate still passed, reporting 0 unguarded.**

Adding the header to the searched text meant the regex matched the function
*definitions* rather than a *call* at the dispatch site. That is
credit-by-mention, the shape behind this program's earlier false passes,
reproduced while fixing it.

Reverted, then fixed properly by binding the match to the **member name** - a
definition cannot satisfy that, because its parameter is named `usage`, not
`imageUsage`.

### 2. A mutation harness that reported false negatives

Three validator mutations came back INERT. They had applied; the resulting
out-of-bounds read aborted the binary under ASan *before* it printed its
summary line, so grepping for a failure count found nothing.

That is the W2 lesson about malformed mutations impersonating broken gates,
reproduced in the tooling built to avoid it. Those three turned out to be the
strongest of the six: removing the bound does not merely fail an assertion, it
walks off the end of the array.

### 3. A dead hand-written mask

`vkr_video_scrub.h` carried a hand-written `VKR_VIDEO_SUPPORTED_QUEUE_BITS` and
subtracted it from the generated mask. The generator already derives its masks
from the supported set, so there was nothing to subtract - and it was a
**second hand-written set deciding a question the generator answers**, which is
the duplication the panel flagged, reproduced in the scrub.

Found by a mutation that should have fired and did not: forcing the supported
set to zero changed nothing, because the subtraction had no effect either way.

### 4. A filter with no control at all

The queue codec-operation mask had no assertion anywhere. Replacing it with
`~0u` changed nothing observable. An unasserted filter is indistinguishable
from an absent one.

It matters because the decode queue bit and the codec list are separate
obligations: a queue advertising decode with an empty list decodes nothing, and
an application correctly declines it - precisely the silent failure measured
before the queue fix.

## Open

1. **Nested table derivation** (§4.5). `NESTED_ARRAYS` / `NESTED_REFS` are still
   hand-declared, and the generator still fails *open* on deferred pointer
   members - it warns and emits nothing for five of them.
2. **StdVideo content bounds** (§4.7). W1 deep-copies every field so no guest
   pointer crosses the wire, but the *contents* are unchecked:
   `seq_parameter_set_id` and `pic_parameter_set_id` are guest-controlled
   indices the host driver uses.
3. **Venus cache audit** (§4.10). The queue-family cache silently dropped a
   chained struct; the finding is that the *pattern* exists, not that this was
   the only instance.
4. **`CtxDetachResource: ErrRutabaga(InvalidContextId)`** (§4.9). Unexplained,
   predates the spike, does not prevent decode. Bounded commitment: explain it
   or document why it is benign.

Also outstanding: the enforcement gate's model is now partially stale - it asks
"is there a reject predicate", which is the wrong question for a supported
value. See `tests/README.md`.
