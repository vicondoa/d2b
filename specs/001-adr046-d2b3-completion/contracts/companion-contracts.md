# Contract: Desktop companion surfaces

**Requirement**: FR-039, FR-040, FR-041, FR-042, FR-045, FR-061, FR-062, FR-063, FR-064, FR-065, SC-024 | **Waves**: W5 publish, W8 verify

## Why this file exists

FR-039 makes an unadapted companion a **release blocker**. A gate that names "every companion"
is unenforceable without a written, versioned set. No such inventory existed in the repository
when this file was written.

This file originally said the companions were "absent from the AGENTS.md sibling-flake section"
and named "only in passing prose" in README. Checked against the tree, the first half is wrong
in the direction that understates the problem: **`AGENTS.md` has no sibling-flake section and
names no companion at all.** The README half is right and is quantified under "Who is in the
set" below. Existing artifacts are canon, so the claim is corrected here rather than repeated.

Publishing that inventory was therefore itself a deliverable, not a lookup. It has since
landed as `docs/reference/companion-contracts.md`, which is the shipped, consumer-facing set;
this file is the program-local record of the obligations behind it, of the FR-039/FR-045
resolution that governs when each stage may proceed, and of the membership and pass tests that
decide who is in the set and what counts as verified.

## The set

| Companion | Surface consumed | Adaptation risk |
| --- | --- | --- |
| `d2b-toolkit` | Client DTOs, public-socket framing, Wayland color parsing, Waybar helpers | **Highest** - shared substrate the others build on; must adapt first |
| `d2b-wlterm` | Public socket `ShellOp` family; capability discovery via `runtime.operationCapabilities.guest.shell`; launcher metadata; `d2b-wayland-proxy` package | High |
| `d2b-wlcontrol` | Public socket; `/etc/d2b/ui-colors.{json,css}`; `d2b audio status --json`; security-key state DTOs; graceful-stop semantics | High - deepest surface |
| `d2b-clip-picker` | Versioned newline-delimited JSON picker protocol over an inherited socketpair fd; realm target naming; accent colors | Medium |
| `weezterm` | None - supplies the terminal binary the launcher invokes | Low |

`entrablau.nix` is an identity sibling composed per-Guest, not a desktop companion.
`wl-proxy` is an upstream crate dependency, not a companion.

## Obligations

| # | Obligation | Wave | Status |
| --- | --- | --- | --- |
| CO-1 | Publish the companion inventory as a versioned reference document naming each companion, its exact consumed surface, and its verification status | W5 | **Done** - `docs/reference/companion-contracts.md`, revision 1, landed at `b72b205f` |
| CO-2 | Publish replacement contracts early enough for companions to adapt, given no preview release may be published (FR-045) | W5 | **Done** - `docs/reference/zone-cli-contract.md`, revision 1, landed at `b72b205f` |
| CO-3 | Verify each companion by exercising it against the release candidate on a live host - not by version number or source inspection | W8 | Open |
| CO-4 | Hold the release while any companion lacks a verified compatible version | W8 | Open |
| CO-5 | Every "surface consumed" cell in the inventory resolves to a committed reference document, schema, or typed definition at a public ref | W5 exit | Open - the W5 exit condition for CO-2 |
| CO-6 | Carry the companion-adaptation assumption as an unvalidated risk with a mitigation and a detection point, and never restate it as fact | standing | **Done** - FR-062 |
| CO-7 | Classify every named surface at W8 as Conformant, Blocked, or Retired, defaulting to Blocked when the outcome cannot be classified | W8 | Open - the pass condition CO-3 is measured against |
| CO-8 | Record any Retired surface on the FR-042 retirement list before the tag, with justification, owner, restoring condition, and a release-note line, and never as a relabelled failure | W8 | Open |
| CO-9 | Re-derive the inventory at W8 under FR-064's two limbs, carrying repository, pinned commit, maintainer, discovery source, and consumed surfaces per row, with a recorded negative determination for any removal | W8 | Open |
| CO-10 | Hold every companion verification to all seven FR-065 conditions, and void every verification if the release-candidate snapshot moves | W8 | Open |

## The resolution, and what it binds

FR-039 blocks the release on external repositories while FR-045 forbids publishing a
preview they could build against. **FR-061 resolves this as a requirement, not as a
plan-level mitigation**, and the resolution is a boundary rather than a compromise:

- a **contract** is committed text, a committed schema, or a committed typed definition at a
  public git ref - publishing one is not a release, and FR-045 does not reach it;
- an **artifact** is anything a consumer's build could select or fetch as a version - a tag,
  a release, a binary, a substituter output, a version-pinned flake output - and FR-045
  forbids every one of them for the whole program.

Both original constraints survive intact. Nothing in FR-039 is loosened and nothing in
FR-045 is carved out; what changes is that the two are no longer read as competing, because
they govern different objects.

### Sequencing, and where each stage refuses

| Stage | Wave | Refuses when |
| --- | --- | --- |
| Publish inventory and contracts | W5 | A "surface consumed" cell names a surface with no committed contract at a public ref (CO-5) |
| Companions adapt | W5 to W8, external | Nothing here refuses. This program does not control a sibling repository's schedule, which is precisely why FR-062 exists |
| Verify on a live host against the candidate | W8 | Any inventory row lacks a verification record naming the candidate, the companion revision, the surfaces exercised, and the result |

The order is load bearing. Publication must precede adaptation because a maintainer cannot
implement against an unpublished shape, and verification must follow adaptation because
FR-040 requires a live exercise rather than an inspection. Running verification early against
an unadapted companion produces a failure that means nothing; skipping it produces a release
that means nothing.

### What is deliberately not claimed

**No external repository has been verified.** Publishing CO-1 and CO-2 established that the
contracts exist and are reachable. It established nothing about whether any companion has
read them, can implement them, or intends to. Every row in the published inventory reads
"Pending live-host verification" for that reason, and the shipped page says in its own words
that publication is not a compatibility sign-off.

**The adaptation assumption is unvalidated and is recorded as such.** FR-062 carries it as a
risk with a mitigation - contracts point at generated schemas and typed definitions rather
than paraphrasing them, so a maintainer implements against the same bytes the implementation
validates - and a detection point, the first live-host verification in W8. That detection
point is late, and saying so is part of the record.

### If adaptation stalls

Exactly two outcomes are lawful: **hold the release**, or **amend FR-045** through the
specification-amendment path with its own evidence. There is no third option. In particular,
publishing an unannounced preview is not a pragmatic exception to FR-045, it is a violation
of it, and treating contract publication as though it discharged verification is not a
shortcut through CO-3, it is a skipped gate.

The no-preview constraint is therefore preserved here rather than amended, because nothing
found while closing this item is evidence that it must be relaxed. What would constitute such
evidence is stated so a future reader can recognise it: a specific companion, a specific
surface, and a specific reason the published contract is insufficient to implement against.
Absent that, the constraint stands.

### What this does not close

Nothing in the companion family remains open. CHK018 is closed by FR-064 and CHK022 by
FR-065; see "Who is in the set" and "What counts as a pass" below.

## Who is in the set: the membership test

FR-039 makes membership release-blocking, so a set that can be argued is a set that can be
argued down. FR-064 fixes it with two limbs, both required.

### Why prose cannot settle it, measured

This file previously said the inventory was hard to derive because companions were absent from
the AGENTS.md sibling-flake section and named only in passing in README. Checked against the
tree, the situation is worse than that and worth stating exactly, because it is the evidence
for the mechanical test:

- **`AGENTS.md` names no companion at all.** There is no sibling-flake section.
- **`README.md` names them once**, at line 38, inside a sentence about colour output: "so niri,
  Waybar, wlcontrol, wlterm, clip-picker, and Wayland rails use the same
  host/realm/workload/state colors."

That single line gets three of the five published members under non-canonical short names
(`wlcontrol`, `wlterm`, `clip-picker` rather than `d2b-wlcontrol`, `d2b-wlterm`,
`d2b-clip-picker`), adds two upstream projects that are **not** members (`niri`, `Waybar`), and
omits `d2b-toolkit` and `weezterm` entirely. Prose may raise a candidate. It cannot settle one.

### Limb 1: discovery sources, closed

1. **The validation host's own flake inputs.** d2b targets a single trusted host with one
   operator, so the set adopting 3.0 can break is what that host actually runs. This is the
   primary source because it is enumerable and cannot be argued.
2. **The currently published inventory**, so the set never shrinks silently.
3. **Any repository a d2b reference doc, example, template, or how-to names** as consuming a
   d2b surface.

### Limb 2: consumed public operator surface, closed

The public daemon socket wire; the `d2b` CLI contract including `--json` and exit codes, and
its v3 replacement; the public `vms.json` manifest; `/etc/d2b/ui-colors.json` and
`/etc/d2b/ui-colors.css`; the clipboard picker protocol over the inherited `socketpair()`; the
public launcher metadata served through the authorized public daemon API; and the flake's
public outputs (`nixosModules`, `packages.<system>`, `templates`, `overlays`).

**Reading a private artifact is not membership, it is a defect.** `manifest-bundle.md` fixes
the boundary and every private bundle artifact installs `root:d2bd` `0640`. A candidate found
reading one is reported, not admitted - admitting it would record an unauthorised read as a
supported contract, which is the opposite of what the inventory is for.

### Evidence, additions, removals, and uncertainty

Each row carries the repository, the exact revision pinned on the validation host **as a
commit** rather than a tag or version string, the maintainer of record, the discovery source,
and the limb-2 surfaces it consumes. A row without a pinned revision is not a row: "which
version blocks" would be undecidable.

An addition needs only both limbs. A **removal needs a negative determination** - recorded
evidence that the candidate consumes no limb-2 surface, at a named revision, on a named date.
Removal by assertion or by absence from prose is not permitted.

**An uncertain candidate is in the set and blocks**, until a negative determination is
recorded. Wrongly including costs one determination; wrongly excluding ships a broken desktop.

## What counts as a pass

FR-065 fixes the pass condition. All seven conditions must hold; there is no aggregate reading
and no majority.

| # | Condition |
| --- | --- |
| 1 | Exercised on the daily-driver **live host** - not a VM, container, or CI runner |
| 2 | Against the **exact release-candidate snapshot** that will be tagged, named by commit |
| 3 | Companion at a **pinned revision**, named by commit |
| 4 | **Every** surface in the row exercised, not a sample |
| 5 | Every surface classified **Conformant or Retired** under FR-063 |
| 6 | **Zero** Blocked, including zero unclassifiable |
| 7 | Evidence recorded in FR-063's shape |

**Not a pass**, restated because each has been offered as one somewhere: source inspection; a
matching version number or tag; a green docs check; the publication of these contracts; a
successful build; a green CI run in the companion's own repository; an exercise against a
non-candidate build; an exercise off the live host; a partial exercise of the row.

**A moved candidate voids every verification against the old one.** If the release-candidate
snapshot changes, all recorded companion verifications are void and re-run. This mirrors the
rule that any content change invalidates prior panel sign-off, and it is what makes "verified
against the release candidate" measurable: without it, "the candidate" is whichever build was
convenient when someone looked.

### Still true: nothing external has been verified

FR-064 says how to decide membership and FR-065 says what passing means. Neither has been
applied to any repository. No candidate has been discovered from host flake inputs, no revision
has been pinned, no negative determination has been recorded, and no companion has been
exercised. All five published rows remain **Pending live-host verification**, and the five-row
inventory itself remains an unverified starting set that FR-064 will confirm or change at W8.

## Partial adaptation: the classification that decides the release

FR-063 answers the question this file previously left open, and the answer is **no**: a
required companion that is degraded holds the release exactly as an absent one does. What
changed is not the strictness but the boundary, because two different things were being
called degradation.

### Conformance is not degradation

`runtime.operationCapabilities` is a committed manifest field, emitted by
`nixos-modules/lib.nix` and pinned in `docs/reference/manifest-schema.json`.
`docs/reference/zone-cli-contract.md` already binds the shell client to "check
`runtime.operationCapabilities.guest.shell` before offering a shell action", and requires
`PoolUnavailable` and `FeatureDisabled` to render as distinct states.

A companion that reads that key, finds it false, and declines the action is **doing what the
contract instructs**. Calling that a defect would hold the release on a companion for obeying
d2b, and would make the capability surface it obeys pointless. Capability discovery is the
sanctioned way an operator's desktop shrinks: it is configuration-driven, operator-visible,
and typed.

Degradation is the other case, and it is the one SC-024 names: the surface is available, and
the companion cannot use it. That is "an operator's desktop degraded by adopting 3.0", and it
blocks.

### The three outcomes

| Outcome | Condition | Release |
| --- | --- | --- |
| **Conformant** | Every surface in the row works, or is unavailable through a published capability key or a named typed refusal state, and the companion refuses with an actionable message and takes no fallback | Ships |
| **Blocked** | Absent, crashes, hangs, silently wrong, falls back to another transport or privilege path or a legacy shape, refuses unactionably, needs an undocumented workaround, or cannot be classified | Held |
| **Retired** | A Blocked surface converted to an explicit FR-042 capability retirement, decided before the tag | Ships, named in the release notes |

**No partial credit.** A row with one Blocked surface is Blocked. **Unclassified is Blocked**,
because an inconclusive exercise and a broken one look the same from the gate.

**No fallback, ever.** This mirrors what the shipped contract already says of the surfaces
themselves: "there is no SSH, host-shell, per-VM service, or broker-operation fallback for a
refused shell request", and unknown fields "are refusals, not an invitation to guess a legacy
shape". A companion that reaches around a refusal is Blocked, not degraded.

### The actionable-refusal bar

A conformant refusal names the capability key or refusal state that is false, **and** at least
one concrete operator action - an option to set, a command to run, or an artifact to inspect
(FR-017).

Not actionable, and therefore Blocked:

- a bare "not supported" or "unavailable";
- a generic retry prompt;
- a message that names only the companion and not the capability;
- a silently disabled or greyed control with no explanation.

The last is the one a live-host exercise is most likely to wave through, because a greyed
button looks deliberate. It is not: the operator cannot tell a configuration choice from a
broken integration, which is precisely the state this bar exists to reject.

### The safety carve-out, and why it is not a special case

No separate rule is needed for security-relevant surfaces, and it is worth saying why. A
missing security-key state indicator in `d2b-wlcontrol`, or a missing `unsafe-local`
no-isolation posture, reads to an operator as "no ceremony in progress" and "isolated". A
capability-conditional refusal is visible and says so; a silent absence is not, and lands in
the unactionable class above, which is Blocked. The general rule already produces the strict
answer, so adding a security exception would only invite argument about its edges.

### Release-gate evidence, exactly

The shipped inventory already requires four items per row, and its fourth is "the result,
including **any capability refusal or degraded behavior**". That page anticipated this
classification, so **no shipped document changes**; what follows says how the fourth item is
populated.

Per row, the release record carries:

1. the exact release candidate exercised;
2. the companion revision and host integration used;
3. a live-host exercise of every surface named in the row; and
4. per surface, the outcome and its classification, plus:
   - for a capability-conditional refusal: the capability key or refusal state, its observed
     value, and the refusal text as displayed;
   - for a Retired surface: the retirement-list entry, its justification, its named owner, the
     condition that would restore the surface, and the release-note line;
   - for a Blocked surface: the observed behaviour, which holds the release.

Source inspection, a package version, a green docs check, and the fact that the contracts were
published at W5 remain excluded as substitutes (FR-040, FR-061).

### Migration and deprecation behaviour

A retirement is an enumerated fact, not a timeline. FR-045 leaves exactly one release, and
this repository deliberately retired its staged warning, fail-loud, and removal calendar at
the clean break - `docs/reference/default-switch-and-deprecation.md` is now a historical
landing page for that reason. Inventing a multi-release deprecation ladder for companions
would contradict a posture the repository already decided.

So a retirement carries a justification, an owner, the restoring condition, and a release-note
line, and nothing else. The published inventory row must not read as verified while a surface
is retired; it reads the retirement, so the gap stays visible rather than aging into silence.

Retirement is unavailable where FR-041 applies: if the capability's migration disposition
promised a successor, the successor must be obtainable and no retirement substitutes for it.
And a retirement is decided **before** the tag - relabelling a failed exercise afterwards is
the one move this whole classification exists to prevent.

## Acceptance

Every companion in the inventory has a compatible version exercised against the release
candidate on the daily-driver host before 3.0 is tagged (SC-024), where the inventory is the
one FR-064 derives, "compatible" means every named surface is Conformant or Retired under
FR-063, and "verified" means all seven FR-065 conditions hold. Publication of this inventory
and of the replacement contracts is not part of that acceptance; it is the precondition that
makes adaptation possible, and FR-061 forbids reading it as evidence of compatibility. A
surface that is Blocked, including one whose exercise could not be classified, holds the
release.
