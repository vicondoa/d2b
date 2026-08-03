# Contract: Desktop companion surfaces

**Requirement**: FR-039, FR-040, FR-045, FR-061, FR-062, SC-024 | **Waves**: W5 publish, W8 verify

## Why this file exists

FR-039 makes an unadapted companion a **release blocker**. A gate that names "every companion"
is unenforceable without a written, versioned set. No such inventory existed in the repository
when this file was written: two of the most contract-coupled companions were absent from the
AGENTS.md sibling-flake section entirely, and README named them only in passing prose.

Publishing that inventory was therefore itself a deliverable, not a lookup. It has since
landed as `docs/reference/companion-contracts.md`, which is the shipped, consumer-facing set;
this file is the program-local record of the obligations behind it and of the FR-039/FR-045
resolution that governs when each stage may proceed.

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

CHK033 - requirements for *partial or stalled* companion adaptation as a scenario class,
distinct from the binary block - remains open and unassigned. FR-061 names the two lawful
outcomes when adaptation stalls; it does not decide whether a release may ship with one
companion degraded rather than absent, which is a product decision about release scope and
not a resolution of the FR-039/FR-045 conflict.

## Acceptance

Every companion in the inventory has a compatible version exercised against the release
candidate on the daily-driver host before 3.0 is tagged (SC-024). Publication of this
inventory and of the replacement contracts is not part of that acceptance; it is the
precondition that makes adaptation possible, and FR-061 forbids reading it as evidence of
compatibility.
