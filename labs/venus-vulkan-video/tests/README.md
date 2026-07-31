# The gate suite

`nix run '<lab-flake>#protocol-checks'` runs nine gates. All are hermetic from
the lock and all have been mutation-tested: an injected defect must make the
gate fail, and reverting must make it pass.

**A gate that has never failed has not been shown to work.** This lab produced
five documented false passes before that rule was adopted:

- `pipefail` + `grep -q` inverted an ABI check, which printed *"22 layouts
  verified purely additive"* having verified nothing - twice, the same pattern
- `strings` on a shared library reported extensions PRESENT that were unsupported
- `ffmpeg -hwaccel vulkan` exits 0 while silently falling back to software
- `git push` reported success while pushing an unchanged branch

## The gates

| gate | proves | mutation observed to fail it |
|---|---|---|
| `pins-check.sh` | `PINS.md` matches `flake.lock` | any pin bump without a manifest update |
| `video-exposure-gate.sh` | no video extension enabled in `vkr_extension_table`; no video dispatch entry | `.KHR_video_queue = true`; **`= 1`**; **`= (bool)1`**; wire a dispatch entry; split the assignment across lines |
| `video-capset-gate.sh` | the Venus capset carries no video bit | **six**: truncate the clear loop; delete the clear; drop a number from the array; read a different source array; **mask a different target array**; **early `break` in the loop body**. Removing the capset `memcpy` fails loudly (`NO-CAPSET-TARGET`), not silently |
| `video-value-surface.sh` | which video values exist (**103**) | drop a value; add an unclassifiable one |
| `video-site-manifest.sh` | which struct member carries video, and which way (**189**) | drop a site; flip a direction |
| `video-enforcement-gate.sh` | how many carrying sites are actually enforced (**95 enforced, 93 gated, 1 unenforced** of 189) | stale pin; delete a scrub and watch 95 -> 94 |
| `uncovered-dispatch-gate.sh` | surfaces the manifest cannot see, e.g. MESA vendor commands (**2** pairs, 0 unguarded) | unguard an occurrence; add an unmodelled carrier |
| `reply-hygiene-gate.sh` | a rejection returns a well-formed non-disclosing reply (**0** unwritten) | replace a zeroing `memset` with a bare read; leave one encoded leaf unwritten |
| `video-pnext-surface.sh` | which non-video commands decode video pNext (**5**) | drop an entry point |
| `video-array-cap-audit.sh` | every video array capped **before** allocation (**10**) | **four**: inert guard body on the `array_size` path (2 flagged); inert body on the `iter_count` path (8 flagged); `set_fatal` without `return`; cap removed entirely |
| `generator-drift-check.sh` | the vendored header byte-matches a fresh generator run | hand-edit the generated header |
| `header-sync-check.sh` | forks' vendored headers match the pinned generator | inject one line into a vendored header |
| `abi-gate.sh` | 345 command ids byte-identical; layouts purely additive | add an initializer to a pre-existing function |

> **"Observed to fail" means exactly that.** An earlier version of this table
> listed a mutation for every gate, and a reviewer reasonably read that as
> evidence they had all been mutation-tested. Two of them had not, and both were
> later found false-passing -- the capset gate while claiming
> `PASS -- every video number is masked out of the capset` with two of three
> extensions still advertised, and the exposure gate while `.KHR_video_queue = 1`
> enabled video. A mutation written down is a plan; a mutation run and seen to
> fail is evidence. Only the latter belongs in this column.

Plus two executable suites under ASan/UBSan `halt_on_error=1`:

- `roundtrip` - 58 checks on the H.264 wire format.
- `scrub-controls` - **46** checks on the scrub and rejection logic, decode and
  encode, every one of them paired with a control.

### Why the controls are the point

`tests/scrub/scrub-controls.c` exists because the obvious test is vacuous.
Asserting "the guest sees no video bit" passes whether or not scrubbing
exists, since zero is also what an unset host bit looks like. So each outbound
case asserts the fixture **carries** the bit, scrubs, then asserts it is gone.

Rejection cases carry the mirror control: a video value must be rejected *and*
an ordinary value accepted. A reject function returning `true` unconditionally
would satisfy every positive case while breaking all rendering.

Three mutations show the controls bite rather than merely pass:

| mutation | caught by |
| --- | --- |
| queue-flags scrub made a no-op | the positive control |
| layout reject always returns true | four negative controls |
| bitmask test replaced with equality | four mixed-bit controls |

That third one is the bug the mask test actually guards: a guest sets the video
bit **alongside** legitimate ones, so equality sails straight past it.

The suite is hermetic against the pinned fork and needs no KVM, so it is
unaffected by the blocked lab VM boot.

## Why they are derived, not listed

Every one of these surfaces was hand-enumerated first, and **every hand
enumeration was incomplete.** Three separate times:

| surface | hand answer | derived answer |
|---|---|---|
| array caps | 4 | 10 |
| pNext entry points | 2 | 5 |
| video values | 59 | 103 |

The derivations were then wrong too, and the failures rhyme. Each was a
**name-shaped assumption that quietly matched the wrong thing**:

- `VK_STRUCTURE_TYPE_VIDEO` does not match
  `VK_STRUCTURE_TYPE_QUEUE_FAMILY_VIDEO_PROPERTIES_KHR`
- `vn_decode_VkVideo` does not match `VkPhysicalDeviceVideoFormatInfoKHR`
- `vn_encode_<T>` matches the generic helper **definition**, not a call site
- `\bqueueFlags\b` matches `/* skip val->queueFlags */`, which means the
  opposite of what the match implies
- a non-greedy struct body ran past the next struct's opening tag, so
  `finditer` silently swallowed 214 sites

**Deriving from a name is hand-enumeration wearing a script.** Only the vk.xml
type graph and the generated function bodies are mechanical.

## Fail-closed is the only part that self-caught

Every gate refuses to emit a golden it cannot justify. That branch found two
things no reviewer named:

- `VK_QUERY_RESULT_WITH_STATUS_BIT_KHR`, reachable on the already-dispatched
  `vkGetQueryPoolResults`
- six encode values dragged in by dependency-following

Everything else was found by a human reviewer. Keep the fail-closed branches.

## Three lessons worth carrying

**A count going up is not evidence.** The site manifest reported 329 sites and
looked plausible; it was missing 214, including a row a reviewer had named.
Only grepping for that specific expected row caught it.

**Zero is not evidence of scrubbing.** A test asserting the guest sees no video
bits passes whether or not scrubbing exists, because zero is also what an unset
host bit looks like. Every outbound test needs a positive control: assert the
pre-scrub value *carries* the bit first.

**A gate can read its own documentation.** The enforcement gate credited a
scrub as present because the scrub header's *comment* named the type and
member it handled. Worse, a comment parses as no function at all, so the
crediting path took the "inline scrub, nothing to wire" branch and skipped
every wiring check - three successive wiring fixes therefore changed nothing,
because the check they hardened was never reached.

Two parsing quirks were involved, and **both had already been found and
written down during the manifest derivation, then not carried across**:
comments must be stripped before analysis, and a declaration may put its
return type on its own line. A lesson recorded in one gate is not a lesson
applied in the next.

The rule this produces: **credit wiring, not existence.** A helper only counts
when it is transitively reachable from a `vkr_dispatch_*` entry point. "Called
somewhere" is not enough - a scrub helper called only by its sibling wrapper
inside the same header satisfies it while the real dispatch path calls neither.

And a note on mutation testing: the first mutation of this gate moved nothing,
which looked like the gate was still broken. It was not. The mutation cut one
of *two* call sites and the helper stayed reachable through the other. **A
mutation that fails to move the number is a claim about the mutation before it
is a claim about the gate.**

## Three more lessons, from the round-1 panel

All seven reviewers rejected the wave and six independently found the same
thing, which is itself the lesson: **a gate you wrote cannot referee your own
coverage claim.**

**One guarded path is not a guarded row.** The gate credited a manifest row as
soon as any one of its command paths was guarded. 48 of 189 rows name more than
one path, so this was the common case. Six commands were forwarding video values
to the host completely unguarded while their rows read as enforced. Credit is
now per command path, and the mutation that proves it removes ONE path's guard
while leaving its sibling intact.

**Where a chunk starts matters as much as where it ends.** Splitting source on a
closing-brace line looks safe until you notice a chunk begins after the
*previous* function's brace - and headers end structs with `};`, which is not a
boundary. Chunks then started inside unrelated declarations, signature parsing
returned a name from some other struct, and real helpers never entered the call
graph. This is what made the gate appear nondeterministic: file ordering moved
the boundaries, so the same code gave different answers on the same data.
Functions are now extracted by brace counting.

**A guard can be the vulnerability.** The external-buffer rejection zeroed the
whole reply struct, including `sType` - which the generated reply encoder
asserts on. A guest could trip a host assertion simply by asking for
external-buffer properties with a video usage bit. The code written to close a
leak had opened a denial of service. Neither the enforcement gate nor the
control harness could catch it: both reason about whether a value is *rejected*,
never about whether the rejection leaves a well-formed reply.

### And a limit worth naming

**The enforcement gate proves reachability, not data flow.** It credits a
validator reachable from a command's dispatch entry - not one applied to the
field the manifest names. That is precisely how a missing `pDependencies[].pNext`
walk stayed credited. A green gate is necessary, not sufficient; every new wiring
site should name the field it guards in review, because the gate will not
distinguish `pAttachments[i].pNext` from `pDependencies[i].pNext`.

## The round-2 lesson: I repeated the mistake while fixing it

Round 1 found that the checks covered decode but not encode. I fixed it by
**hand-editing the scrub masks** - and missed the encode quantization-map
values doing so. Round 2 caught that.

This lab's whole method is "derive, don't enumerate", and under time pressure I
reached for a hand edit anyway, on the exact surface where hand enumeration had
already failed six times. The masks are generated now:
`VKR_VIDEO_QUEUE_BITS`, both format-feature masks and the outbound layout
predicate come out of `gen-video-reject.py` from vk.xml video-extension data,
so `generator-drift-check.sh` covers them.

The controls had the matching hole: they tested decode values only, so a mask
regressing to a decode-only subset would have passed. Nine encode checks were
added, and regressing the masks fails exactly four of them.

**A fix applied by hand to a surface that is supposed to be derived is a
regression, even when it is correct.** It was correct for queue flags and
format features and wrong for quantization maps, and nothing but a reviewer
would have told the difference.

## The one root cause, found six times

Six panel rounds produced six findings that all look different and are the same
thing: **a set I wrote by hand decided whether a guard applied.**

| round | the hand-written set | what it missed |
| --- | --- | --- |
| 1 | which command path to guard | five sibling paths on the same row |
| 2 | which scrub mask bits are video | the encode quantization-map bits |
| 3 | which struct members to walk | arrays, then pointers, inside chained structs |
| 3 | which commands the manifest covers | everything vk.xml does not describe |
| 4 | which discovered surfaces need a guard | both of them |
| 5 | which descriptor tags carry an image | two QCOM arms, plus one wrongly included |
| 6 | which **types** are video carriers at all | video extends 25 types; the set covered 9 |

Round 5 is the cleanest illustration: the hand list was wrong in *both*
directions at once. It named `COMBINED_IMAGE_SAMPLER`, which is not a `pImage`
arm, so reading `data.pImage` for it would have reinterpreted a different union
member; and it omitted the two QCOM arms, which are.

Round 6 is the most instructive of the seven. The carrier set was seeded from
the **manifest** - so a type video extends that no manifest row happened to
name got no validator emitted at all. `VK_KHR_video_maintenance1` puts a video
bit in the ordinary create flags of images and buffers, neither type appeared in
any row, and neither was checked anywhere.

**The manifest says which sites carry a value. It does not get to decide which
types exist.** Confusing those two is how a derivation ends up with a
hand-drawn boundary after all: everything inside it was derived correctly, and
nothing asked whether the boundary was right.

Every one of these is now derived - from vk.xml, or from the generated decoder -
and covered by `generator-drift-check.sh`. The rule the lab already had was
"derive, don't enumerate". What six rounds added is **where** to look for
violations of it: not at the values, but at every place a set decides whether a
check runs at all.

## Rounds 3-4: the value is always one level further out

Each panel round found the same class of defect one level further from where
the previous fix had looked.

| round | where the value was |
| --- | --- |
| 1 | on a sibling command path the row also named |
| 2 | in an array inside a chained struct |
| 3 | behind a pointer inside a chained struct |
| 3 | on a command vk.xml does not describe, so no row existed at all |
| 4 | on a surface a gate had discovered but nothing enforced |

That last one is the sharpest. `uncovered-dispatch-gate.sh` was written to close
the blind spot round 3 exposed - and it counted discovered surfaces without
checking whether they were guarded. **A surface a gate has discovered but
nothing guards is strictly worse than one it has not discovered**, because the
pin now asserts it is accounted for. It is the same existence-versus-enforcement
distinction the enforcement gate had already been forced to learn, and I did
not carry it into the gate I wrote immediately afterwards.

### The gate's first act was to fail on my own tooling

On its first full run the guard check reported both MESA surfaces UNGUARDED.
The guards were fine. `PINS.md` and `flake.lock` pointed at a revision
predating the guard commit, because the rev had been read *before* relocking.
**The suite had been validating a stale tree.**

A gate that only ever confirms what you believe is not earning its runtime. This
one's first output was that what the suite validated was not what had been
written.

## The second obligation, which no gate here checks

Three separate rounds found defects not in *whether* a value was rejected but in
*what the rejection returned*:

- zeroing a reply struct also zeroed its `sType`, which the generated encoder
  asserts on - the guard became a guest-triggerable host assert;
- writing `args->pPropertyCount = 0` nulled the reply pointer instead of the
  count, so the reply omitted the field rather than reporting none;
- returning before the host fill left output payloads as whatever was in reply
  storage, and the encoder serialised it - a rejection that leaked;
- a capacity query still reported the unfiltered count, telling the guest how
  many video layouts the host supports.

**Rejecting a value and returning a well-formed, non-disclosing reply are
separate obligations.** Every gate in this lab checks the first. None checks the
second. The enforcement gate would stay green with all four of those defects
present, and did.

That last one is worth dwelling on. The comment in the scrub recorded the
reasoning that produced it - over-reporting capacity is safe, under-reporting is
not - which is correct about *correctness* and silent about *disclosure*. A
justification that is true on the axis you were thinking about will not warn you
about the axis you were not.

## The gate inherited the defect it was built to catch

`reply-hygiene-gate.sh` was written to close a class the other gates could not
see. It then had that same class found inside it, twice:

| what the gate hand-chose | what it missed |
| --- | --- |
| outputs are `vn_encode_simple_pointer` | blob arrays -- `vkGetQueryPoolResults.pData` |
| rejects are `if` conditions calling `vkr_video_*` | helper predicates -- both render-pass creates |

Both are the wave's one shape: a set written by hand deciding whether a check
runs. Writing a gate to catch that shape does not exempt the gate from it.

Each fix also paid for itself immediately. Deriving blob-array outputs surfaced
`vkWriteResourceDescriptorMESA`, and neither reviewer had named it.

The rule that follows: **when a gate encodes "what counts as X", that encoding
is itself a hand-written set and needs the same scrutiny as the code.** The
useful question is not "does the gate pass" but "what did the gate decide not
to look at".

## Current state

`video-enforcement-gate.sh` reports **95 of 189 enforced, 93 gated, 1
unenforced**, pinned at 1.

The three buckets are different claims and are reported separately on purpose:

- **enforced** - a scrub or rejection naming the type and member is
  transitively reachable from a `vkr_dispatch_*` entry point.
- **gated** - every command that could carry the value has no dispatch entry,
  so it cannot be invoked. This credit is *contingent*: the moment one of those
  commands gains a dispatch entry, the sites stop being closed. The gate prints
  that dependency rather than leaving it implicit.
- **unenforced** - genuinely open. One site, `VkBindMemoryStatus.pResult`,
  with a written rationale in `docs/open-direction-derivation.md`.

Gating is derived, not guessed. It originally asked whether a command path had
"Video" in its name; it now asks whether the command has a `vkr_dispatch_`
entry at all. That is a fact the source answers, and it correctly gated three
commands that are undispatched and have nothing to do with video.

The enforcement code is **generated** from the same manifest that defines the
obligation (`tests/gen-video-reject.py`), including the pNext walker, the
presence rejections for video-typed chained structs, and the nested-array
walks. Hand-listing this surface was wrong every time it was tried.

- **A member name is not an identity.** Two structs can own members of the same
  name, and one's guard will silently cover for the other. Key every obligation
  by `(owning struct, member)`. This defect was found in the coverage gate,
  fixed there, and then reproduced verbatim in the next gate written -- because
  the lesson had been recorded as a fact about one gate instead of a rule about
  identity. When a finding is fixed, ask what class it belongs to, then re-audit
  every gate against the class.
- **When strictness does not fire the mutation, the discriminator may be at the
  wrong level.** Three consecutive attempts narrowed *which functions count* for
  a check whose defect was *which struct it was credited to*. Narrowing a
  correct-but-orthogonal axis can never close the gap; it just moves the
  baseline around.
- **One row per obligation.** When a single manifest/credit row stands for
  several independent obligations, satisfying any one of them credits them all.
  Seen twice: a populated-list scrub sharing a row with a capacity rewrite, and
  a top-level output member standing in for every leaf its encoder serialises.
  Split the row, or add an explicit obligation-kind dimension.
- **A mutation that does not move the number is a claim about the mutation
  first.** Twice this was true and following it found the real defect; three
  other times it hid one. Always resolve which it is before editing the gate.
- **Mutation-test the gate that guards the headline claim first.** The capset
  gate false-passed a mutation that left two of three video extensions
  advertised to the guest, and it had never been observed failing. Prefer
  auditing gates in order of how load-bearing their claim is, not in the order
  the panel happens to raise them.
- **Enumerate what is permitted, not what is forbidden.** Every one of this
  wave's repeated defects was a forbidden-list or an accepted-spelling list that
  the author's imagination bounded. `= true` was matched as "enabled" so `= 1`
  read as absence. Invert the check: match any assignment and allow only the
  explicitly disabling values, so unanticipated spellings fail closed.
- **When a check binds N facts, mutate each of the N independently.** A capset
  check documented as "binds three facts" bound two; the third was asserted in
  the comment and absent from the regex. Source-side and target-side are
  separate facts: verifying the loop reads the right array says nothing about
  where it writes. Count the claims, then write one mutation per claim.
- **Assert the mutation applied before trusting the verdict.** A helper that
  silently failed to forward its arguments made three mutation runs test the
  pristine tree and report PASS. Diff the mutated file against the baseline
  first; a mutation harness that cannot fail loudly is worse than none.


## W3: the enforcement gate's model is now partially stale

The pin moved from `--expect-unenforced 1` to `--expect-unenforced 92`. That is
a large jump and it is not a regression, but it does mean the gate is measuring
something less useful than it used to, so the reason is recorded here rather
than left in a commit message.

**What changed.** W2's boundary was "video is entirely absent", so every video
value arriving anywhere was a rejection, and the gate asked a single well-posed
question: *is there a reject predicate reachable from every command path that
can carry this value?* 189 rows, 95 enforced, 93 gated by NULL dispatch, 1
deferred.

W3 supports H.264 decode. The generator now derives the reject surface by
intersecting vk.xml against a supported set, so decode values are no longer
rejected anywhere -- correctly, because rejecting them would advertise an
extension whose values are refused on arrival.

**What the 92 are.** Almost entirely members of video-typed structs --
`VkVideoBeginCodingInfoKHR.videoSession`, `VkBindVideoSessionMemoryInfoKHR.*`,
`VkVideoCapabilitiesKHR.flags` and so on -- which are now *inputs to a
supported feature* rather than values that must never appear. Plus
`VkBindMemoryStatus.pResult`, which was the pre-existing deferred row and is
unchanged.

**Why the number is not the point.** For a supported value, "is there a reject
predicate" is the wrong question. The right one is "is it validated for the site
it arrived at", and that is what the W3 validators do -- DPB slot range and
membership, coding scope, bitstream bounds, sequence ordering -- none of which
this gate can see, because it looks for rejections.

So the pin's meaning has narrowed: it still detects a decode value regaining a
reject predicate, which would break decode, and it still detects the encode
surface losing one. It no longer measures how much of the video surface is
checked.

**What covers the gap.** `tests/scrub/video-validate-controls.c` -- 30 checks,
six mutations observed firing, run under ASan and UBSan. That harness asks the
right question for supported values, and it is a negative-control harness
precisely because a validator that accepts everything is invisible to a
positive one.

Rebuilding the enforcement gate around validator coverage rather than rejection
coverage is real work and is not done. Until it is, **the enforcement number
should not be read as a coverage measurement for the decode path.**
