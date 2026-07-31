# OPEN: the direction column is not trustworthy for bitmask values

`tests/video-value-surface.sh` reports a `direction` for every value in the
video surface. **The value set is sound. The direction column is not, for
bitmask values.** This is an open finding, not a fixed one.

## What direction is for

Each direction implies a different obligation on W2's implementation:

| direction | obligation |
|---|---|
| inbound | reject the value before the host call |
| outbound | scrub the value from guest-visible replies |
| both | do both |

So a wrong direction silently drops a requirement. It is not a cosmetic field.

## Four models tried, and why each failed

| model | failed because |
|---|---|
| **1. value name bucket** | gave one direction per value; 50 of 103 travel both ways *(c, critical)* |
| **2. `vn_decode_<T>` / `vn_encode_<T>` exists** | matches the generic helper **definitions** the generator emits for every serializable type. `VkQueryType` has three decode call sites and one `vn_encode_VkQueryType` - its own definition - so it was marked `both` *(c, critical)* |
| **3. call sites only** (`vn_encode_<T>(enc,`) | correct for named types, but finds **nothing** for bitmask values, because they are never serialized through a helper named after their `FlagBits` type |
| **4. member-level site matrix** | right model, incomplete implementation - see below |

## Why bitmask values defeat type-level derivation

`VK_QUEUE_VIDEO_DECODE_BIT_KHR` is a `VkQueueFlagBits` value. It rides in a
`VkQueueFlags` member, which the generator serializes as a raw `VkFlags`:

```
vn_protocol_renderer_device.h:315:    vn_encode_VkFlags(enc, &val->queueFlags);
```

Nothing in that line names `VkQueueFlagBits`, `VkQueueFlags`, or video. No
amount of type-name matching can see it. The same is true of every
`usage-bit`, `sync-bit`, `format-feature` and `image-layout` value - most of
the inbound surface.

## The correct model, and where the implementation stopped

`c` specified it precisely: derive `(command, struct, member, value,
direction)` and golden **that**, then drive per-site tests from it.

The pieces that work:

- `FlagBits` → `Flags` typedef resolution from `vk.xml` (attribute order
  varies; match `requires=` or `bitvalues=` in either position).
- struct → member-type table from `vk.xml`.

The piece that does not: locating each struct's generated serializer body.
The lookup assumed a `vn_decode_<Struct>_self_temp` / `vn_encode_<Struct>_self`
naming convention. `VkQueueFamilyProperties` has no `_self` variant - it is
serialized inline - so every site lookup returned empty and the gate
fail-closed on 41 values rather than producing a wrong answer.

**Fail-closed did its job here.** The gate refused to emit a golden it could
not justify, instead of shipping 41 confidently-wrong directions.

## Current state

Reverted to the last green derivation, which is **model 2** - sound value set,
direction over-reporting `both` for types whose encode helper is only a
definition. Over-reporting `both` is the safe direction of error: it demands
scrubbing where none is needed, rather than omitting it where it is.

That is a defensible interim position, not a correct one.

## What must happen before W2 closes

1. Enumerate every struct serializer name shape the generator actually emits
   (`_self`, `_self_temp`, `_self_partial`, inline) rather than assuming.
2. Build the `(struct, member, value, direction)` matrix and commit it as the
   golden, replacing the value-level direction column.
3. Generate the enforcement tests from that matrix - `test`'s round-1 finding:
   the golden currently proves classification, not enforcement.
4. Give every scrub test a **positive control**: assert the pre-scrub value
   carries the video bit, then assert the reply clears it. A scrub test that
   only checks for zero passes when the host never set the bit.

Until then, no claim should be made that outbound scrubbing is completely
specified.

---

# Update: the site manifest is built

`tests/video-site-manifest.sh` now derives the `(struct, member, carrier type,
direction)` matrix that `gpu` and `c` independently specified. **543 sites -
317 inbound, 226 both.** Committed golden, wired into `protocol-checks`,
mutation-verified.

Items 1 and 2 of "what must happen before W2 closes" are done:

- [x] Enumerate the serializer name shapes the generator actually emits. There
      are nine; the **bare** form is the one `VkQueueFamilyProperties` uses, and
      assuming `_self` cost an entire earlier attempt.
- [x] Build and commit the site matrix.
- [ ] Generate the enforcement tests from it - `test`'s finding stands: the
      golden proves classification, not enforcement.
- [ ] Give every scrub test a **positive control**.

## The value-level direction column is now redundant, and still wrong

The site manifest supersedes it. `video-value-surface.sh` still reports a
per-value direction derived at type level, which over-reports `both` for types
whose encode helper is only a definition. That is the safe direction of error,
and the manifest is now the authority, but the redundant column should be
dropped rather than left to be mistaken for evidence.

## Three bugs found building it, all the same family

Each was a pattern that looked right and quietly matched the wrong thing:

1. Assumed serializer names end in `_self`. Nine shapes exist.
2. Struct-body regex ended at the first `</type>` - a member's own inline type
   tag - truncating every struct body to nothing. Zero sites derived.
3. With that fixed, a non-greedy body could run past the next struct's opening
   tag. `finditer` resumes *after* the match, so one over-long match silently
   swallowed every struct inside it. `VkBufferCreateInfo` vanished entirely,
   `sType` row included, while `VkImageCreateInfo` survived. Bounding the body
   against the next struct opening recovered **214** sites, 329 → 543.

**Bug 3 is the one to remember.** The gate did not fail - it under-reported,
and the missing row was exactly the site a reviewer had named. It was caught
only by grepping for that specific row rather than trusting the total.

A count going up is not evidence of correctness. A count that looks plausible
is not evidence either. The only thing that caught this was checking for a
named, expected entry.

---

# Update 2: value-level direction removed; the manifest is the authority

`test` found a fourth counterexample to value-level direction, and it inverted
the meaning of what it matched. The generator emits, for members it
deliberately does **not** serialize:

```
/* skip val->queueFlags */
```

A word-boundary match on the member name reads that skip marker as proof the
member *is* carried. `VkQueueFamilyProperties::queueFlags` is the clean case:
its only appearance in any decode body is inside that comment.

Comments are now stripped before indexing. **13 rows moved `both` → `outbound`.**
That is the direction that matters: those are members the host fills and the
guest reads, needing scrubbing and never inbound rejection.

`test`'s second counterexample also resolves: `VkBufferUsageFlags2CreateInfo::usage`
is inbound only.

## The scrub obligations, now precise

| site | why it matters |
|---|---|
| `VkQueueFamilyProperties.queueFlags` | door 7 (`virt`) |
| `VkQueueFamilyVideoPropertiesKHR.videoCodecOperations` | door 3 (`gpu`) |
| `VkFormatProperties{,3}.{linear,optimal,buffer}Features` | format-feature leak (`test`) |
| `VkVideoCapabilitiesKHR.flags` | |
| `VkVideoDecodeCapabilitiesKHR.flags` | |
| `VkVideoFormatPropertiesKHR.imageUsageFlags` | |
| `VkDrmFormatModifierProperties{,2}EXT.drmFormatModifierTilingFeatures` | |

**13 sites, derived - matching every leak reviewers found by hand, plus six
they did not name.**

## Value-level direction is gone

Three attempts failed three different ways, and the common error was the
premise: **direction is a property of a carrying site, not of a value.**
`VK_QUEUE_VIDEO_DECODE_BIT_KHR` is outbound in `queueFlags` and would be
inbound anywhere a guest could set it. "Which way does this value travel" has
no single answer.

`video-value-surface.sh` now answers only *which values exist, in which
family*. `video-site-manifest.sh` answers *where and which way*. A second,
weaker answer to a question another gate answers correctly is how a wrong
answer gets quoted as evidence.

## Remaining before W2 closes

- [x] Enumerate real serializer name shapes (nine)
- [x] Build and commit the site matrix (543 sites)
- [x] Remove the superseded value-level direction column
- [ ] **Generate enforcement tests from the manifest** - the golden proves
      classification, not enforcement
- [ ] **Positive control on every scrub test** - asserting the guest sees zero
      passes whether or not the scrub exists, because zero is also an unset
      host bit

The last two are `test`'s, and both remain open.

---

# Update 3: enforcement gated; one item left

`tests/video-enforcement-gate.sh` reads the manifest and reports, per site,
whether the renderer enforces what the site implies. **0 of 97.**

That number is the deliverable. `test`'s point was that the manifest proves
classification, not enforcement - a golden listing every site and an
implementation handling none are both consistent with a green manifest gate.
The enforcement gate is what makes the difference visible, and it has been seen
reporting the bad answer, which is the only way to trust it reporting the good
one later.

`--expect-unenforced 97` pins the count. Adding enforcement fails the gate
until the pin drops in the same commit, so progress is visible and a silent
regression back to zero is impossible.

## The manifest is 97, not 543

`VkStructureType` is a carrier because the video extensions add sTypes to it,
so every struct in the registry matched - 446 of 543 rows were
`VkApplicationInfo.sType` and its like. An sType matters only as a pNext case
label, and `video-pnext-surface.sh` already answers that precisely with 5
entry points. Dropped. All five reviewer-named sites survive.

| direction | sites | obligation |
|---|---|---|
| inbound | 80 | reject before the host call |
| outbound | 13 | scrub from guest-visible replies |
| both | 4 | both |

## Remaining before W2 closes

- [x] Enumerate real serializer name shapes (nine)
- [x] Build and commit the site matrix
- [x] Remove the superseded value-level direction column
- [x] Gate enforcement coverage against the manifest
- [ ] **Positive control on every scrub test**

The last item cannot be built yet: there are no scrub tests, because there is
no scrubbing code. It is a requirement on W2's implementation, recorded here so
it lands with the tests rather than after them.

**The requirement:** a scrub test that asserts the guest sees zero passes
whether or not the scrub exists, because zero is also what an unset host bit
looks like. Each of the 13 outbound sites needs its test to first assert the
pre-scrub value **carries** the video bit, then assert the guest-visible reply
clears it. Without that first assertion the test is not evidence.

This is the same shape as the five false passes recorded in W0/W1 - a check
that reports success without checking.

## The one deferred carrying site

`VkBindMemoryStatus.pResult` is pinned as the single unenforced site, and the
deferral is deliberate rather than an oversight.

It is unlike every other site in the manifest. `pResult` is a `VkResult *`
that the **host writes through** during `vkBindBufferMemory2` /
`vkBindImageMemory2`, so the value does not exist at the moment the guest's
request is validated. Closing it means scrubbing *after* the host call
returns, mapping a video-specific status such as
`VK_ERROR_VIDEO_PROFILE_OPERATION_NOT_SUPPORTED_KHR` onto a generic one - a
post-call rewrite path that no other site in W2 needs.

The generator already refuses to emit a scalar check for it: it is a pointer,
and a scalar check would test the address rather than the value behind it.
That refusal is why it shows up as unenforced instead of silently passing,
which is the correct failure mode.

It is pinned at 1 so it stays visible and cannot quietly become 2.

## Resolved: the path-aware gate now agrees with itself

`test`'s C1 finding was correct. The enforcement gate credited a manifest row as
soon as ONE of its command paths was guarded, while 48 of the 189 rows name more
than one. Six commands were forwarding video-carrying values to the host
completely unguarded -- `vkCmdSetEvent2`, `vkCmdWaitEvents2`,
`vkGetDeviceBufferMemoryRequirements`, `vkGetDeviceImageMemoryRequirements`,
`vkGetDeviceImageSparseMemoryRequirements` and
`vkGetDeviceImageSubresourceLayout` -- each verified at zero video guards before
and guarded after.

Making the gate path-aware exposed a second, deeper bug that had been corrupting
every count it ever produced.

**Chunking on `\n}\n` was wrong.** A chunk begins after the *previous*
function's closing brace, so it can start deep inside another file's
declarations: headers end structs with `};`, which is not a boundary. The first
`{` in such a chunk then belongs to an unrelated struct, so the signature parse
returned a name from that struct and the real helper never entered the call
graph at all. Its callers then read as unenforced.

That is precisely why the gate disagreed with an isolated reproduction of its
own logic. Both ran the same code on the same manifest, but the file ordering
differed, which moved the chunk boundaries -- so the same code gave different
answers on the same data. The symptom looked like nondeterminism; the cause was
a parser that depended on where an unrelated file happened to end.

Function definitions are now extracted by **brace counting**, and the name is
recorded during extraction rather than re-parsed from the chunk head. Every
attempt to re-parse that head was wrong in a different way: single-line-only
matching missed helpers whose return type sits on its own line, and once chunks
could start mid-file the head belonged to another declaration entirely.

Corrected coverage: **89 enforced, 93 gated, 7 unenforced.**

Mutation evidence, including the one `test` asked for:

| mutation | enforced |
|---|---|
| baseline | 89 |
| remove ONLY `vkCmdSetEvent2`'s guard, sibling still guarded | **75** |
| restored | 89 |

That middle row is the whole point: under the old any-path rule those rows stayed
credited while a live path forwarded the value untouched.


## Known limit: the gate proves reachability, not data flow

`gpu` found a path the corrected gate still credits wrongly.

`VkMemoryBarrier2` lists `vkCreateRenderPass2` among its command paths, because
it chains onto `VkSubpassDependency2`. The render-pass-2 walk never touched
`pDependencies`, so the value was not rejected - yet the gate counted the row as
enforced.

The reason is structural. The gate credits a validator that is **reachable**
from the command's dispatch entry, and `vkr_video_reject_pnext` *was* reachable
there through the attachment walk. Reachability is not data flow: it proves a
validator *can* run on that command, not that it is *applied to the field the
manifest names*.

This is the gate's honest ceiling. Closing it would need the call graph to carry
which argument each validator is applied to, which is dataflow analysis rather
than reachability. Until then:

- **A green enforcement gate is necessary, not sufficient.** It catches an
  entirely unwired path; it cannot catch a validator wired to the wrong field.
- Every new wiring site should name the field it guards in review, because the
  gate will not distinguish `pAttachments[i].pNext` from `pDependencies[i].pNext`.

The path itself is now wired, along with input-attachment reference pNext.

## Closed: nested output pNext payloads are now gated

`gpu` and `c` both found that a rejected `vkGetPhysicalDeviceImageFormatProperties2`
serialised its output pNext chain with payloads the reject path never wrote.
The fix is a generated `vkr_video_zero_image_format_pnext()`, derived from the
reply encoder's own pnext switch.

Gating the class took four attempts, and the first three all reported zero:

1. keyed on the reply body - a struct's chain is encoded one level down inside
   `vn_encode_<Type>()`, so the pattern could never match;
2. same, after stripping comments - still keyed on the wrong place;
3. the comment `"sType and pNext survive"` in the reject block matched a
   substring test for `pnext`, the fourth comments-as-code inversion in this lab.

The fourth attempt resolves each output member to its TYPE and asks whether
`vn_encode_<Type>_pnext` exists - the resolution step the first three skipped.

It then reported `vkGetPhysicalDeviceExternalBufferProperties`, and that was a
**false positive worth recording**: its `_pnext` encoder body is literally
`/* no known/supported struct */` and always writes NULL, so nothing can escape
through it. Had I trusted the gate instead of reading the encoder, I would have
guarded a chain that cannot carry anything. The gate now requires the chain to
actually serialise something.

Mutation-verified: removing the zeroing call reports the chain unwritten and
fails; restoring returns zero.

**The interim state is the point.** Between finding the defect and gating it,
the check was REVERTED rather than pinned at a green it had never been seen to
earn. A gate never observed failing asserts coverage it has not demonstrated,
and this lab has documented that failure mode four times. Carrying an honest
gap for a round was cheaper than carrying a dishonest number.


## I called the ceiling structural, and I was wrong

`nixos` found that `carries()` returned only the FIRST carrier in a nested
struct, so `VkPhysicalDeviceImageFormatInfo2` holding both `VkImageUsageFlags`
and `VkImageCreateFlags` could be credited after guarding only usage.

Fixing `carries()` did not make their mutation fire. I traced why: removing the
create-flags check from that validator left `vkr_video_value_VkImageCreateFlags`
still *reachable* from the dispatch, because the pNext walker reaches
`vkr_video_reject_VkFramebufferAttachmentImageInfo`, which also calls it.

**I concluded the remaining gap was structural** -- that closing it needed
dataflow analysis the gate could not do, and that the honest answer was to
record the limit and stop. That conclusion was wrong, and it was wrong in the
most tempting direction: it turned an unsolved problem into a principled-sounding
boundary.

The tractable formulation was one step away. A carrier is not just "a carrier
reachable from this command" -- it is **a carrier found inside a specific
struct**. Bind the two, and the guard requirement becomes: that struct's own
validator must check that carrier, and must be reachable from the dispatch.
Reachability by any other route no longer counts, because the other route
belongs to a different struct.

Mutation-verified: removing only the create-flags check from
`vkr_video_reject_VkPhysicalDeviceImageFormatInfo2`, leaving usage guarded, now
reports
`vkGetPhysicalDeviceImageFormatProperties2 carries VkPhysicalDeviceImageFormatInfo2.VkImageCreateFlags [UNGUARDED]`
and fails.

One exception is deliberate: an inline `vkr_video_value_*` call in the dispatch
body counts, because it is applied to that command's own args and is therefore
as specific as a validator. The MESA host-copy structs have no generated
validator at all -- vk.xml does not describe them -- and are guarded exactly
this way.

**The lesson is about the conclusion, not the code.** "This needs dataflow
analysis" was a true statement about the *reachability* formulation and a false
statement about the problem. Declaring a limit structural is a claim that
deserves at least one serious attempt to falsify it, because it is the most
comfortable place a piece of work can stop.


## Closed: occurrence-path binding (gpu M1)

`gpu` raised this on round 16 and again on round 17, correctly marking it
previously-raised. I acted on `nixos`'s finding the first time - the same
"newest first" failure I had already named once.

The defect: obligations were bound to `(owner struct, member, carrier)`, but a
command can reach one struct by several paths. `vkCmdBeginRendering` reaches
`VkRenderingAttachmentInfo` through `pColorAttachments[]`, `pDepthAttachment`
and `pStencilAttachment`. Deleting only the `pStencilAttachment` validation
left both gates green, because the same member checks stayed reachable through
colour and depth.

**The first implementation was reverted, and that was the right call.**
Discovery is easy; recognising a guard is not. Requiring the generated
validator by name flagged seven plainly-guarded occurrences. Accepting any
video-aware wrapper reduced that to three. The next loosening would have
accepted almost any nearby video call - the vacuity this gate exists to
prevent. Shipping it pinned at three known false positives would have been
worse, because a pin asserts the number is meaningful.

**What made it tractable was naming the shapes instead of loosening.** Three
guarding patterns exist in this renderer and all three are legitimate:

1. a generated validator applied to the occurrence;
2. a hand-written wrapper applied to the occurrence;
3. a loop binding the occurrence to a local, whose body then checks that
   local's members - the call never names the occurrence at all.

Matching each explicitly, rather than widening one pattern until the count hit
zero, gives a gate that is both green and meaningful. The difference matters:
loosening reaches zero by accepting more things as guards, and naming shapes
reaches zero by understanding what a guard looks like.

One further correction was needed. Recording an occurrence for every nested
struct produced 179 findings, almost all on structs like `VkExtent2D` that
cannot carry a video value at all. An occurrence obligation exists only where
the struct actually carries one.

Mutation-verified: deleting only the `pStencilAttachment` validation now
reports
`vkCmdBeginRendering carries VkRenderingAttachmentInfo.pStencilAttachment [UNGUARDED]`
and fails.

## Open: populated-list scrub vs capacity rewrite (security M1)

`security` found the sixth level, and it is in the **enforcement gate** - which
I had not re-examined while tightening the coverage gate five times. I had
flagged that gate as the thing I most wanted checked; the concern was correct.

`VkPhysicalDeviceHostImageCopyProperties.pCopySrcLayouts` is credited as
enforced, but deleting the populated-list scrub leaves the gate at 95 while
populated replies leak video layouts. Verified.

The credit survives because `vkr_video_fix_layout_capacity()` calls
`vkr_video_scrub_image_layout_list(probe.pCopySrcLayouts, ...)` - a genuine
video call naming that member. But it scrubs a **scratch probe** used to
re-derive a count when the guest passed NULL. It never touches the guest's
populated list.

Tightening the gate to require the member inside a video call's *arguments*
(rather than anywhere in the body) was a strict improvement and is kept - it
removed no true credits. **It does not close this finding**, because both the
real scrub and the capacity probe pass an expression naming the member. The
difference is *which object* the expression refers to: `p->pCopySrcLayouts` on
the guest's reply versus `probe.pCopySrcLayouts` on a local scratch array.

`security`'s framing is the right one and is more than a gate fix: **populated-list
scrubbing and capacity-count rewriting are two different obligations and should
be two manifest rows**, each with its own guard requirement. Crediting one with
the other is a modelling error, not a matching error, which is why no amount of
pattern tightening resolves it.

Recorded as open rather than closed with a textual discriminator on `probe.`
versus `p->`. That would pass the mutation and would be exactly the kind of
loosely-motivated pattern this wave has repeatedly shown to be wrong.

**Renderer status:** the populated-list scrub is present and correct today, so
this is a gate false-pass rather than a live leak. The risk is a future edit
removing it unnoticed - the same status the occurrence-path finding had before
it was closed.

### Second attempt also failed; the finding stays open

I tried to close `security` M1 a second way: require an **outbound** obligation
to be discharged by the outbound scrub walk, on the reasoning that
`vkr_video_fix_layout_capacity` is not reachable from
`vkr_video_scrub_physical_device_properties2` and so would stop standing in.
The discriminator is sound - I verified the capacity helper is not reachable
from the scrub walk - but my implementation was inert: baseline stayed at 95
and the mutation still did not fire. Reverted rather than left as dead code.

Two attempts, neither closing it. That strengthens rather than weakens
`security`'s and `c`'s framing: this is **not** a matching problem with a
cleverer pattern waiting to be found. One manifest row is standing for two
obligations, and the fix is to model them as two - separate rows or an explicit
obligation-kind dimension - with separate mutation-backed enforcement for each.

Recording two failed attempts is more useful than recording none: it is
evidence about *where* the fix has to live, and it is the reason I am not going
to try a third variation of the same idea.

## security M1 - CLOSED, on the fourth attempt, at a different level

The finding: `VkPhysicalDeviceHostImageCopyProperties.pCopySrcLayouts` was
credited enforced, but deleting its populated-list scrub left the gate at 95.
The row was credited by `vkr_video_fix_layout_capacity()`, which calls a genuine
scrub naming the member -- on a scratch probe, never the guest's list.

Attempts 1-3 all tried to make the *match* stricter: exclude the capacity
helper, require the call to be in a scrub function, require the member to be an
argument. Each held the baseline at 95 and none fired the mutation. I recorded
after attempt 2 that the discriminator was sound and my implementations buggy.
That was half right, and the wrong half cost a third attempt.

The actual cause: `VkPhysicalDeviceVulkan14Properties` **also** has a member
named `pCopySrcLayouts`, and it is scrubbed in a different arm of the same walk.
Removing one struct's scrub left the other satisfying a member-name-only check.
No amount of narrowing *which functions count* could fix that, because the
credit was coming from the right kind of function -- just for the wrong struct.

The fix binds the obligation to `(site, member)` and searches only the walk arm
handling that site. Mutation now fires: 95 -> 94, baseline restored to 95.

This is the **seventh** appearance of the wave's one root cause, and the second
time it appeared as cross-struct member-name collision specifically -- `virt`
found exactly this in the coverage gate at level 5. I fixed it there and did not
carry it to a check I wrote afterwards. The lesson from level 5 was recorded as a
fact about that gate rather than as a rule about identity, so the next gate
inherited the defect. The generalisation that survives: **a member name is not an
identity.** Every obligation, in every gate, must be keyed by the struct that
owns the member.

## Eighth appearance, found by applying the rule instead of waiting for the panel

Immediately after closing security M1 I re-audited every gate against the class
it belonged to -- "a member name is not an identity" -- rather than treating the
fix as done. Reply-hygiene, the named unexamined candidate, failed twice over.

**First defect: credit by mention.** The gate asked whether the output member's
name appeared anywhere in the reject block. Replacing the zeroing `memset` with
a bare read of the same member restored a real disclosure and left the gate at
0. The gate's own comment, two lines below the defect, already said *"naming the
member is not enough"* -- written for the pNext case and never applied to the
member case directly above it. Fixed by requiring a write: memset/memcpy over
the member, assignment to it or a field beneath it, or handing it to a
zero/scrub helper.

**Second defect: wrong granularity.** Requiring a write still did not fire the
mutation. That was a claim about the mutation first, and following it produced
the real finding: the gate models one obligation per top-level output member,
but the reply encoder serialises several leaves beneath it. Deleting the memset
over `->imageFormatProperties` left the sibling `pNext` zeroer naming the parent
member, which satisfied the single row for all of its leaves.

Obligations are now per encoded leaf, resolved from the member's type encoder,
with a whole-struct memset still crediting every leaf at once. Mutation A fires
with a precise message; baseline holds at 0; full suite exit 0.

This is the **same shape as security M1** -- several obligations collapsed into
one row -- appearing in a different gate within an hour of the first. Two gates
have now had it. The rule stands: when a finding is fixed, name its class and
re-audit every gate against the class, because the class is what recurs.

## Ninth appearance -- on the gate guarding the wave's central claim

Continuing the class sweep, I mutation-tested the capset gate, which I could not
recall ever having observed fail. W2's headline safety claim is "advertises
nothing", and the capset is the channel that claim is about: it reaches the
guest before any command is dispatched.

Truncating the clear loop to `i < 1` leaves ext 25 (`video_decode_queue`) and 41
(`video_decode_h264`) advertised to the guest. The gate printed
**`PASS -- every video number is masked out of the capset`** and listed all
three as "cleared".

Two independent defects, both already named elsewhere in this wave:

- **Credit by mention.** The cleared set was read out of the *comment-annotated
  array literal*, so a number was credited for being written down, not for being
  cleared. The gate's own header warns that an annotated list of numbers is not
  a mask -- and it was reading exactly that list as its evidence.
- **Split obligation.** Array membership and "a clear exists somewhere in
  src/venus" were checked independently and neither was tied to the other, so
  satisfying both separately never established that the clear consumed that
  array.

The check now binds three facts and refuses to infer any from the others: the
number is in an array literal; a loop iterates *that* array over its full
`ARRAY_SIZE`; and that loop's body masks the indexed element off the capset.
Comments are stripped before analysis. Four mutations verified: truncated loop,
deleted clear, number dropped from the array (fails precisely, naming only 41),
and the loop reading a different array. Baseline passes; full suite exit 0.

Packaging note: the first wiring resolved the helper `$0`-relatively, which
under Nix became `/nix/store/capset-clear-check.py`. It **failed closed** rather
than passing vacuously, which is the behaviour the lab wants from a missing
dependency, and it is now passed explicitly via `CAPSET_CLEAR_CHECK`.

Nine appearances. The sweep is finding these faster than the panel is, which is
the argument for auditing by class rather than by finding.

## Tenth appearance -- accepted spellings

Continuing the sweep to the exposure gate, which enforces the other half of
"advertises nothing": no `VK_KHR_video_*` enabled in `vkr_extension_table`, and
no video command with a non-NULL dispatch entry.

Adding `.KHR_video_queue = true` correctly failed the gate. Adding
`.KHR_video_queue = 1` did not. Against a bool field that is legal C, it enables
the extension, and the gate printed
`PASS -- renderer advertises no video support`.

The cause is the same root cause in yet another dress: the gate matched a
**hand-written set of accepted spellings** (`= true`) and treated everything
outside that set as absence. The check is now inverted -- it matches *any*
assignment to a video field and allows only the explicitly disabling values
`false` and `0`, so every other spelling fails closed. Verified: `= true`,
`= 1`, and `= (bool)1` all fail; `= false` passes; baseline passes; suite exit 0.

Ten appearances. Every one is "a hand-written set decides whether a guard
applies", and the durable countermeasure has been the same each time: **enumerate
what is permitted, not what is forbidden.** A forbidden-list is only as good as
the author's imagination; a permitted-list fails closed on everything the author
did not think of.

## Eleventh appearance -- inside the repair for the ninth

The security reviewer's round-22 finding, against the capset check I had written
one round earlier to close the ninth appearance:

> `capset-clear-check.py` claims to bind three facts but actually binds two. It
> verifies the loop reads the correct SOURCE array and that *some* array is
> masked, but never that the mask target is the array that becomes the capset.

Correct. My four mutations covered the source side -- reading from a different
array -- and never the target side. Replacing `ext_mask[n / 32] &= ~(...)` with
an unrelated array passed the gate while the capset kept the video bits.

The target array is now **discovered, not assumed**: whatever is `memcpy`'d into
`vk_extension_mask*` is the capset array, and the mask must target that. If no
such memcpy is found the check exits non-zero rather than guessing.

Also closed in the same commit: the test reviewer's early-exit vector, which
they raised but explicitly declined to file. A `break`/`continue`/`return` in
the loop body makes the `ARRAY_SIZE` bound a lie -- the loop is written to cover
the array but stops short. Closed rather than argued about, because "not present
in the source today" is what was said about several defects that later were.

Six mutations now verified against this one check: truncated loop, deleted
clear, number dropped from array, wrong source array, wrong mask target, early
break. Baseline passes; suite exit 0.

The lesson worth keeping is not about capsets. **When a check binds N facts,
enumerate the N and mutate each one independently.** I wrote "binds three facts"
in the commit message and believed it; two of the three were bound and the third
was asserted in prose. A reviewer caught the gap between what the comment
claimed and what the regex did -- which is the same comments-are-not-evidence
failure this lab has now seen at both the code and the gate level.

## Twelfth appearance -- the cap that caps nothing

Continuing the sweep to `video-array-cap-audit.sh`, which carries the wave's
only DoS claim: every video-reachable guest-controlled array is capped *before*
allocation.

The gate matched `if (array_size > N) {` and nothing else. Replacing the guard
body with a comment -- keeping the comparison, removing the rejection -- left
every array unbounded while the gate reported
`PASS -- 10 video-reachable arrays, all capped before allocation`. A guest-chosen
count would reach `vn_cs_decoder_alloc_temp_array()` with nothing stopping it.

Same split obligation as the capset and reply-hygiene gates: the *shape* of a
guard was credited without its *effect*. The check now locates the guard, walks
its braces, and requires the body to both call `vn_cs_decoder_set_fatal` and
`return`. The accepted-variable-name list (`iter_count|array_size`) is gone as
well -- any comparison against a constant counts, provided it rejects, so a
renamed loop variable cannot silently drop coverage.

The generator has **two** cap emission sites, and mutating one flagged only 2 of
10 arrays; mutating the other flagged the remaining 8. Both are now bound.

Verified: inert body on the `array_size` path (2 flagged), inert body on the
`iter_count` path (8 flagged), `set_fatal` without `return`, cap removed
entirely. Baseline 10; suite exit 0.

### A harness failure worth recording

The first mutation run reported PASS for all three mutations. The mutations had
never applied -- a shell helper failed to forward its arguments, so every run
tested the pristine tree. This is the fourth time in this wave a malformed
mutation has impersonated a result, and the second time it impersonated a
*passing* one, which is the dangerous direction. The rule already written down
covers it: **a mutation that does not move the number is a claim about the
mutation first.** I now assert the mutated file actually differs from the
baseline before trusting any verdict.

## W2 closed -- 7/7 at c0c09b25, round 23

Twenty-three rounds. Every round produced a finding; the last one did not.

The wave's single root cause -- **a hand-written set deciding whether a guard
applies** -- was found twelve times, in twelve different dresses. The panel found
some; the class sweep found more, and found them faster, because auditing by
class asks "where else does this shape live?" instead of waiting to be shown the
next instance.

What the last three rounds cost, and why they were worth it: round 21 signed off
7/7 on a tree whose capset gate was false-passing, and the sign-off was honest --
the reviewers had no way to know, because the README told them the gate had been
mutation-tested when it had not. That single inaccurate line in a test document
bought a unanimous but worthless sign-off. Round 22 caught a real blocking defect
in the fix for round 21's. Round 23 found nothing, which is the first time that
has happened.

Two things I would do differently from the start:

1. **Mutation-test every gate the day it is written**, in load-bearing order.
   Two gates were pinned without ever being observed to fail, and both were later
   found false-passing. The principle was written down early and applied late.
2. **Record only what was run.** The gate table listed planned mutations beside
   observed ones in the same column, and a reviewer reasonably read the column as
   evidence. Evidence and intent must not share a column.

Nothing in W2 is runtime-verified: `/dev/kvm` is unavailable to this account, so
every claim here is static or hermetic. Multiple reviewers accepted this
explicitly as a reservation rather than a closure blocker, and it is the first
thing W3 should retire.


## W3: uncovered-dispatch and the allowlist guard shape

**Status: CLOSED, after one attempted fix was reverted for being inert.**

`vkGetPhysicalDeviceVideoFormatPropertiesKHR` is guarded. The dispatch entry
rejects a disallowed `imageUsage` before the host call and filters the reply to
a pinned format allowlist, and both guards have mutation-proven negative
controls in `tests/scrub/video-validate-controls.c`.

`uncovered-dispatch-gate.sh` still reports all three of its surfaces as
`[UNGUARDED]`, because it looks for the `vkr_video_value_*` / `vkr_video_reject_*`
families and the W3 guards are allowlists - `vkr_video_format_is_allowed`,
`vkr_video_decode_usage_is_allowed`. A guard can reject a disallowed value or
accept only allowed ones; both close the surface, and after W3 the second shape
is the common one because decode values are supported rather than forbidden.

So the gate's notion of what a guard looks like is hand-written, which is this
wave's recurring root cause appearing inside the gate built to catch it.

### The attempted fix, and why it was reverted

Added `vkr_video_validate.h` to the gate's source text and matched the two
allowlist names inside the dispatch entry. The gate went green.

Then the mutation: removing the guard from the dispatch entry entirely, leaving
zero occurrences in `vkr_video.c`. **The gate still passed, reporting 0
unguarded.**

Adding the header to the searched text meant the regex matched the function
*definitions* in the header rather than a *call* at the dispatch site. Every
surface was credited for the guard existing somewhere, which is the
credit-by-mention shape that produced the reply-hygiene and enforcement false
passes earlier in this program - reproduced here while fixing it, and caught
only because the fix was mutation-tested before being pinned.

Reverted under the established rule: **a gate never observed failing must not be
pinned.** An inert gate is worse than a failing one, because the failing one is
telling the truth.

### What a correct fix needs

Match the guard as a **call reachable from the dispatch entry**, using the
existing call-graph machinery, rather than as text present in a searched blob -
and keep the definition-bearing header out of whatever corpus the match runs
against. The current `--expect-uncovered 2` pin plus the 3 reported unguarded
surfaces is the honest state until then.

The three surfaces are guarded in fact. The gate cannot currently see it, and
saying so is more useful than a green gate that has been shown not to fire.

### The fix that worked, and exactly what it covers

Two changes, because the first attempt had conflated two obligations:

1. **An outbound scrub was genuinely missing.** The format allowlist decides
   which format ROWS survive; it says nothing about the flag members on a
   surviving row, and the host may report a format that is both decode- and
   encode-capable with the encode bits set. `imageUsageFlags` and
   `imageCreateFlags` are now masked separately. Treating the row filter as
   covering both was the split-obligation mistake that produced earlier false
   passes here.

2. **The gate match is bound to the MEMBER NAME**, not the function name. That
   is what makes it immune to the failure the first attempt had: a definition
   cannot satisfy it, because the parameter is named `usage`/`props` rather
   than `imageUsage`/`imageUsageFlags`.

Mutations run before pinning:

| Mutation | Result |
|---|---|
| inbound `imageUsage` guard deleted | **FIRED** |
| outbound flag scrub deleted | **FIRED** |
| format row filter deleted | INERT -- and correctly so, see below |

The third is out of this gate's scope rather than a hole in it.
`VkVideoFormatPropertiesKHR.format` carries `VkFormat` with kind `scalar`, so
it is not a video-value carrier and neither shell gate asserts on it. **Its
coverage is `test_format_allowlist` in `tests/scrub/video-validate-controls.c`,
whose own mutation -- widening the allowlist to accept anything -- fires.**

Stating that precisely matters: "all mutations fired" would have been false,
and "the gate covers the format filter" would have been false too. What is
true is that each of the three guards has coverage, in two different harnesses.
