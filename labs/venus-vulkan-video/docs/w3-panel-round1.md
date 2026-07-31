# W3 round-1 panel disposition (plan gate)

Round 1 on plan rev 3 returned **0/7 with 21 findings**. Reviewed tip:
lab `efa4b2df`, virglrenderer `114e0304`, Mesa `0f06521a`.

Rev 4 answers all of them. Two are disputed with evidence; the reviewers are
asked to judge those on the merits, and withdrawal is permitted but not
required.

## Accepted

| # | Reviewer | Sev | Finding | Answered in rev 4 |
|---|---|---|---|---|
| 1 | gpu | critical | No spatial bounds on `srcBufferOffset`/`srcBufferRange` vs buffer size, or `codedOffset`/`codedExtent` vs image view dimensions | §4.3 invariant 4 |
| 2 | security | critical | `pSetupReferenceSlot` bypasses range/membership when `slotIndex != -1`; `referenceSlotCount` unbounded | §4.3 invariants 1-2 restated |
| 3 | gpu + security | high/med | Accumulated (lifetime) SPS/PPS cap breaks streams that rotate parameter sets | §4.1 rewritten to capacity-based |
| 4 | security | high | Guest-controlled indexes inside StdVideo unvalidated (`seq_parameter_set_id` 0-31, `pic_parameter_set_id` 0-255) | §4.7 (new) |
| 5 | security | high | No coding-scope closure check at `vkEndCommandBuffer` | §4.3 scope tracking |
| 6 | c | high | Guest leaks `vn_video_session_parameters` on session destroy | §4.8 (new) |
| 7 | c | high | `NESTED_ARRAYS` walk does not recurse into array elements' pNext | §4.5 |
| 8 | c | medium | Generator warns and emits nothing for deferred pointer members - fails open | §4.5, now fails closed |
| 9 | gpu | medium | Format allowlist not enforced at `vkCreateImage` | §4.2 |
| 10 | test | high | No negative controls for the §4.1 aggregate bounds | §6 |
| 11 | test | high | No malformed-input control for the two-pass handle-replacement unwind | §6 |
| 12 | test + virt | high/med | `CtxDetachResource` teardown error must not be deferred | §11 Q4 resolved; §4.9 (new) |
| 13 | mesa-venus + virt | high | Mesa format un-strip / queue-family cache are W3 deliverables, and the cache audit is mandatory not optional | §12, §4.10 (new) |
| 14 | mesa-venus | high | Video objects should be registered in `vkr_device_object.json`, not hand-wired | §4.1 |
| 15 | mesa-venus | medium | Supported-extension set duplicated between the Python generator and the C capset derivation | §4.6 |
| 16 | nixos | high | Flake still pinned to throwaway `spike-video` branches | §7 step 0 |
| 17 | nixos | high | `PINS.md` stale, `pins-check.sh` will fail | §7 |
| 18 | nixos | medium | Vulkan headers pin missing from `PINS.md` | §7 |
| 19 | nixos | medium | Whole-tree `policy_cli_consumers.rs` may flag lab files | §7 |

## Disputed

### D1 - mesa-venus, critical: "StdVideo structs are forwarded as opaque blobs with raw guest pointers"

**Rebuttal, with evidence.** This is what W1 was built to prevent, and the
generated code does the opposite of what the finding describes.

The finding reasons that because the nested-table derivation keys on vk.xml's
`structextends` and `len`, and StdVideo types are not described that way in
vk.xml, they must be treated as opaque. The premise is right about vk.xml and
wrong about this tree: W1 added a **private XML**,
`venus-protocol/xmls/VK_VIDEO_std_h264.xml`, 12 structs and 84 members, whose
entire purpose is field-level description of these types. The plan's own
history records raw `sizeof`/`memcpy` wire layouts as **forbidden** because
StdVideo uses bitfields and is ABI-fragile.

Generated encoder, `build/vn_protocol_driver_transport.h:248-257`:

```c
if (val->pOffsetForRefFrame) {
    vn_encode_array_size(enc, val->num_ref_frames_in_pic_order_cnt_cycle);
    vn_encode_int32_t_array(enc, val->pOffsetForRefFrame, ...);
}
if (vn_encode_simple_pointer(enc, val->pScalingLists))
    vn_encode_StdVideoH264ScalingLists(enc, val->pScalingLists);
if (vn_encode_simple_pointer(enc, val->pSequenceParameterSetVui))
    vn_encode_StdVideoH264SequenceParameterSetVui(enc, val->pSequenceParameterSetVui);
```

Generated renderer decoder,
`virglrenderer/src/venus/venus-protocol/vn_protocol_renderer_transport.h:236-251`:

```c
val->pOffsetForRefFrame = vn_cs_decoder_alloc_temp_array(dec, ...);
val->pScalingLists      = vn_cs_decoder_alloc_temp(dec, sizeof(*val->pScalingLists));
val->pSequenceParameterSetVui = vn_cs_decoder_alloc_temp(dec, ...);
```

Every pointer member is deep-copied by value, and the renderer allocates its
own storage rather than dereferencing anything the guest supplied. No guest
pointer crosses the wire. W1's roundtrip harness has a dedicated check (T3)
asserting every non-padding SPS/PPS byte reaches the wire, precisely so a
dropped field cannot pass silently.

**No change made.** The reviewer is asked to judge this on the merits.

What the finding does correctly identify, and rev 4 adopts as finding 4 above,
is that deep-copying a value is not the same as validating it. The *contents*
of those StdVideo fields were unvalidated, and §4.7 now bounds them.

### D2 - security, medium: "`updateSequenceCount` must be strictly greater, not exactly +1"

**Rebuttal, with lower confidence.** My reading of
`VUID-VkVideoSessionParametersUpdateInfoKHR-updateSequenceCount-07215` is that
it requires the value to equal the current counter **plus one**, not merely to
increase. If that reading is right, exact-increment is the conformant check and
accepting a gap would let renderer and host desync - which is the failure the
check exists to prevent.

I have not been able to verify the VUID text from inside this tree, so I hold
this with less confidence than D1. **No change made pending the reviewer's
judgment.** If the reviewer can cite the VUID text and it says "greater than",
the check becomes `>` and rev 5 will carry it.

## Open questions resolved

- **Q1 (Firefox `HARDWARE_VIDEO_DECODING` blocklist).** Unanimous: purely a W6
  concern, nothing in W3 depends on it. Recorded as such.
- **Q2 (non-coincide DPB path).** Split 3-2 for implementing it (gpu,
  security, mesa-venus for; test, c against on "untestable branches rot"). The
  two *for* reviewers independently proposed the same concrete test - mask
  `reuse_dst_dpb` out of the capability reply to force the guest down the
  non-coincide path - which dissolves the objection, because the branch stops
  being untestable. **Resolved: implement, and test by capability masking.**
- **Q3 (Venus cache audit).** 3-1 for W3 (mesa-venus, virtualization, security
  for; test for W4). The deciding argument is security's: a cache that silently
  drops a chained struct lets a guest bypass W3's validators by
  desynchronising state, so it is a W3 correctness concern rather than W4
  tidying. **Resolved: mandatory W3 work item, §4.10.**
- **Q4 (`CtxDetachResource: ErrRutabaga(InvalidContextId)`).** Split 3-3
  (test, virtualization, security for W3; gpu, c, mesa-venus for W8). Resolved
  toward W3 on the strength of test's argument: W3 is the wave that defines
  object lifetimes, pins and destroy cascades, so an unexplained teardown error
  on the same boundary could mask exactly the reference-count leak W3 claims to
  prevent. **Resolved: bounded investigation in W3 (§4.9) - explain it or
  document why it is benign. Fixing it is not a W3 commitment.**

## Scope correction §12

Unanimously accepted, with mesa-venus and nixos adding that the Mesa work the
spike did (format-feature un-strip, queue-family video properties cache) must
be named as W3 deliverables rather than left implicit. Rev 4 names them.

---

# Round 2 (delta review)

Reviewed tip: plan rev 4. `gpu` returned three findings, all accepted, and two
of them overturn earlier conclusions.

## The parameters-lifetime reversal

`c` (round 1) said destroying a video session implicitly destroys its
parameters objects, so the guest leaks. `gpu` (round 2) said the opposite: they
are **not** implicitly destroyed, and cascading is a critical defect.

Resolved by evidence rather than by vote. Mesa's own common implementation is
decisive: `struct vk_video_session_parameters`
(`src/vulkan/runtime/vk_video.h:127`) holds no back-pointer to a session, and
`vk_video_session_parameters_destroy` unlinks from no parent list. If the
relationship were owning, the reference implementation would have to track it
in order to free them.

**`gpu` is right and `c`'s finding is withdrawn.** Consequences:

- The spike's renderer cascade is actively harmful, not merely unnecessary: it
  frees the renderer record *without* calling
  `vkDestroyVideoSessionParametersKHR`, leaking the host object, and a
  conformant guest destroying its parameters afterwards finds a record that is
  already gone. §4.1 now requires removing it.
- §4.8, added in rev 4 to answer `c`, rested on the false premise and is
  withdrawn.

This is the round's most valuable finding: the spike shipped it, round 1
confirmed the wrong direction, and only a second independent reader caught it.

## D2 resolved against me

I disputed `security`'s round-1 finding that `updateSequenceCount` must be
strictly greater rather than exactly `stored + 1`, holding it with stated low
confidence because I could not verify the VUID from inside the tree. `gpu`
raised the same finding independently in round 2 with the VUID reference.

Two independent reviewers converging outweighs an unverified reading.
**Accepted**: exact-increment would reject a conformant client whose counter
advances by more than one.

## Third finding

`gpu` high: handle replacement for `pSetupReferenceSlot->pPictureResource` must
be conditional on `slotIndex >= 0`. When the index is negative the spec says
the resource is ignored, so a conformant guest may leave it uninitialised, and
the plan's own "replacement obligation, not merely a validation one" note would
dereference it. Accepted - validation and replacement now skip an ignored setup
slot together.

## D1 still open

`mesa-venus`'s critical (StdVideo forwarded as opaque blobs with raw guest
pointers) remains disputed with the evidence in the D1 section above. No
reviewer has yet judged the rebuttal.
