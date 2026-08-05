# ADR 0053 panel prompt sources and construction contract

- Kind: decision-support specification for
  [ADR 0053](../0053-gascity-contributor-infrastructure.md). This is not
  shipped product documentation and it is not a Diataxis page. It exists so
  that decision D21 has one committed collection point for prompt guidance
  covering every panel seat and every rig stage, rather than twelve prompt
  files with twelve independent notions of what a good source is.
- Status: supports the 2026-08-04 amendment to ADR 0053. It records sources,
  licensing constraints, prompt requirements and local conventions; it does not
  itself decide roster composition, selection, the surface classifier, or the
  verdict schema. D21 decides those.
- Retrieval date for every external source below: **2026-08-04**. Sources move.
  Section 7 is the caveat register for the ones already observed moving.
- Scope: the twelve-role reviewer pool
  (`software`, `test`, `product`, `docs`, `security`, `observability`,
  `simplicity`, `reliability`, `agentic`, `nixos`, `networking`, `kernel`), the
  four `software` language profiles that D21 binds from the change surface, the
  Gas City `product` profile that D21 binds for reviews of ADR
  0053 itself, and the prompt-construction contract for every Gas City rig
  stage and seam the opted-in workflow runs.
- Non-scope: the operative seat prompts. Those live in
  `.github/agents/panel-*.agent.md` and are governed by
  [`docs/contributing/panel-review.md`](../../contributing/panel-review.md).
  This document is their input, not their replacement, and nothing dispatches
  it.

## 0. How to read this document

### 0.1 Normativity markers

Every source below carries one of three markers. The marker is load-bearing:
a seat may only state a finding as **blocking** on the authority of an `N`
source, a repository rule, or a defect it can demonstrate from the diff. An `A`
source supports an argument; it does not by itself make a finding blocking.

| Marker | Meaning |
| --- | --- |
| **N** | Normative. A standards body, a language or platform specification, an official reference manual, or a vendor statement about the vendor's own product behaviour. |
| **A** | Advisory. Widely used practice guidance that is nobody's standard: company engineering handbooks, personal books and essays, community style guides, community prompt collections. |
| **M** | Moving. The URL or its content has been observed to change, redirect, rate limit, or track an unstable channel. Cite the retrieval date; prefer a pinned or versioned spelling where one exists. |

A source can carry two markers. `N M` means the content is normative but the
address is unstable.

### 0.2 Source quality and licensing ladder

Normativity says whether a claim can block. It says nothing about whether the
text may be reused. Those are different questions and this document answers
both, because the most quotable reviewer prompts in circulation are the ones
with the worst provenance.

| Tier | Definition | Verified examples | Permitted use |
| --- | --- | --- | --- |
| **T1** first-party normative | Standards bodies, language and platform specifications, official reference manuals, a vendor statement about the vendor's own product | PEP 8, RFC 430, Cargo SemVer reference, OpenTelemetry semantic conventions, man7 and kernel.org pages, Gas City `docs/reference/specs/*`, GitHub Copilot documentation, `code.claude.com` documentation | May carry a blocking finding. Short normative sentences may be quoted with citation. |
| **T2** mature project practice | Engineering handbooks and mature-project conventions with a named owner and a change history | Google engineering practices, Google Shell Style Guide, Nixpkgs `pkgs/README.md` and `pkgs/by-name/README.md`, Rust API Guidelines | Supports an argument. Blocking only when a repository rule also applies. |
| **T3** permissively licensed community and upstream prompt assets | MIT, CC0 or Apache-licensed prompt, agent and skill collections whose licence is verified at a pinned commit | `github/awesome-copilot` (MIT), `github/spec-kit` (MIT), `obra/superpowers` as vendored (MIT), `EveryInc/compound-engineering-plugin` as vendored (MIT), `anthropics/claude-code-security-review` (MIT), `f/awesome-chatgpt-prompts` prompt data (CC0) | Extract **structures and checklist items**. Do not copy prose. Never blocking on their authority alone. |
| **T4** restricted or unverifiable | No-derivatives or non-commercial licences, sources with no stated licence, unattributable aggregations, extracted or leaked prompt texts | `hesreallyhim/awesome-claude-code` (CC BY-NC-ND 4.0 at the surveyed revision), `gastownhall/gascity-packs` at the pinned commit (no repository LICENSE), third-party compilations claiming to reproduce an unpublished vendor prompt | Read and cite as **behavioural evidence about the system under review**, and only where naming the source is itself the point. **No copying and no adaptation of text, organization, or expressive structure.** Never cite as authority. |

**T4 is behavioural evidence, not a template, and the distinction is the one
most likely to be blurred.** Reading `GCP gascity/assets/workflows/build-base/review.md`
to establish *that upstream gates this step on an artifact schema*, and citing
it for that, is evidence about a system this repository integrates with; that
is legitimate and section 5 depends on it. Retyping its headings in the same
order, mirroring its section decomposition, or lifting its checklist taxonomy
is adaptation of expressive structure, and the absence of a licence grant makes
that unavailable regardless of how much the words are changed. The
consequence for section 5 is concrete: a d2b stage prompt is derived from the
permissively licensed material below plus this repository's own requirements,
and the T4 anchor tells the author **which upstream step the prompt overrides
and what that step declares**, not how to lay the prompt out.

**Adaptation policy.** These are rules, not preferences.

1. **Extract structure, retype text - from T3 only.** Take the taxonomy, the
   checklist headings, the ordering, the "what I do not flag" device **from a
   permissively licensed source**. Do not paste more
   than a short quoted fragment from any T3 asset, and attribute the source
   repository and its licence when you do. This permission does not reach T4:
   from a T4 source neither the text nor the structure is available.
2. **Never adopt a premade prompt's numbers.** Confidence anchors, file-size
   thresholds, changed-line depth bands, duplication line counts, percentage
   exploitability bars and multi-band severity ladders are tuned to codebases
   that are not this one. Importing a number silently creates a threshold this
   repository never authorised. Section 5.5 lists the specific numbers observed
   and refused.
3. **Pin every external GitHub source to a commit blob URL.** ADR 0053's own
   convention is a pinned blob URL, never `tree/main` or `blob/master`.
4. **Never use a leaked, stolen, confidential, unattributed,
   reverse-engineered or licence-incompatible prompt**, and never build on one
   indirectly through a compilation that does. Where a T4 source's underlying
   idea is genuinely useful, find it in a T1 source or do without it.
5. **MIT obligations are real.** MIT requires the copyright and permission
   notice to travel with "substantial portions". A checklist-heading list is
   not a substantial portion; a whole agent file is. If a whole file is ever
   vendored, vendor it the way `gascity-packs` vendors its subtrees, with an
   `upstream.toml` recording source, commit, licence and vendored paths.
6. **State the provenance or drop the source.** A prompt fragment whose origin
   cannot be named is not usable here at any tier.

### 0.3 What a prompt built from this document must not become

A reading list is not a prompt. Every section below ends with prompt
requirements and anti-patterns because the failure this document exists to
prevent is a seat that recites its sources instead of reviewing the diff.
`docs/contributing/panel-review.md` records the observed version of that
failure: seats with no stated threshold treat anything they notice as blocking,
and because `signoff` is true if and only if `recommendations` is empty, each
such seat costs a full extra round across the entire roster.

The second failure this document exists to prevent is the opposite one, and it
is visible in the upstream assets surveyed here: a prompt that is all mechanics
and no threshold. The Gas City `plan-review.md` stage prompt is 408 bytes and
carries one sentence of review instruction; `prepare.md` spends most of its
length on bead-command quoting rules; Spec Kit's command templates devote
roughly half their bytes to extension-hook boilerplate. A seat prompt built in
that shape tells the reviewer how to file a verdict and nothing about what
earns one.

## 1. The shared contract every seat prompt carries

### 1.1 The finding bar

The repository already owns this. Every `.github/agents/panel-*.agent.md`
carries a byte-identical `## The bar for a finding` section, and
`scripts/copilot/check-bindings.mjs` enforces the byte identity. Three new
seats mean three new copies of the same bytes, not three new thresholds, and
the seat D21 removes takes its copy with it: ten committed prompt files minus
`panel-rust.agent.md` plus three is twelve, and the bidirectional check is what
proves that rather than this sentence.
Nothing in this document restates, paraphrases, or improves that block; a seat
prompt that does has broken the gate.

Two rules from that block matter enough to name here because every section
below assumes them:

- **Report the class, not the instance.** A finding that names one substituted
  position closes one position; a finding that names the class closes the
  class.
- **Prose asserting a property is not evidence of it.** Where the delta claims
  a property, check the property.

### 1.2 Ownership map, and the non-overlap rule

A roster of eight to twelve seats overlaps more than a fixed roster of ten.
The guard is an explicit ownership map, restated in every prompt. D21 decides
the boundaries; this is the operative restatement.

| Territory | Owner |
| --- | --- |
| Correctness of changed control flow; structure, readability and error handling; local coding, naming, file and directory conventions; measured performance in every language; and the per-language standards depth of the active profiles, including unsafe and FFI soundness, public API and SemVer classification, and workspace dependency direction | `software` |
| Coverage of new behaviour, invisible regression classes, gate placement, whether cited validation covers the change | `test` |
| Operator experience, naming surface, migration and deprecation, default-off shape, error actionability; scope and gap fidelity against decision and acceptance items; external contract fidelity across CLI, exit codes, wire and artifact schema and version discipline; cross-decision consistency and supersession | `product` |
| Diataxis placement, changelog, schema-to-prose drift, ADR index coverage, process-marker and dash rules; intra-document coherence, terminology drift, undefined forward references, ambiguity, cross-links | `docs` |
| Exploitability, attacker model, authorization and trust boundaries | `security` |
| Metric, span, log and audit shape; cardinality; retention; redaction; exporter correctness | `observability` |
| Minimal decision surface, reuse over reinvention, abstraction count, indirection, dependency adoption and removal, deletions; and the same questions asked of a record rather than of code | `simplicity` |
| Resource ownership and cleanup on error and crash paths; restart, adoption and idempotency; ordering and concurrency across components; partial failure and degraded state; on-disk state and schema migration | `reliability` |
| Agent profiles, instruction layering, prompt contracts, formula and pack mechanics, mechanical gates versus prompt-only assurances | `agentic` |
| Module and option system, activation ordering, merge semantics and priority, eval-time assertions, structural option surfaces per RFC 42, NixOS-specific correctness | `nixos` |
| Reachability delta, firewall posture, address and port allocation, MTU and MSS, routing, host network coexistence | `networking` |
| Syscall and kernel interface semantics, version floors, race classes, descriptor inheritance and lock semantics, signal semantics, mount semantics, filesystem error cases | `kernel` |

Four boundaries are stated again because they are the ones that will be got
wrong:

- **`software` versus `simplicity`.** `software` reviews the shape of the code
  that exists. `simplicity` asks whether it should exist. A finding that says
  "this function is hard to read" is `software`'s; a finding that says "this
  abstraction has one implementor and should be deleted" is `simplicity`'s.
- **`software` versus `reliability` versus `kernel` versus `test`.**
  `software` owns in-function correctness and error propagation. `reliability`
  owns the design property across components: who owns this resource, who
  releases it when this process dies here, what the on-disk state means
  afterwards. `kernel` owns whether the syscall was used correctly and what
  kernel version it needs, **including descriptor inheritance across `exec`,
  open file description versus POSIX record lock semantics, signal disposition
  and restart behaviour, mount semantics, and which errno a filesystem call can
  return**. `test` owns whether any of it is covered. The `kernel` and
  `reliability` line is drawn at the same file twice: whether `O_CLOEXEC` was
  set on the `open` is `kernel`'s, whether the descriptor is closed on the
  error branch three frames up is `reliability`'s; whether an OFD lock survives
  the close of an unrelated descriptor to the same file is `kernel`'s, whether
  the lock is released when the component crashes is `reliability`'s.
- **`software` versus `nixos`, the only remaining language overlap.**
  `software`'s Nix profile owns Nix as code: readability, naming, idiom,
  `with`-scope and `let` hygiene, dead bindings, and the boundary where
  formatting stops being a finding. `nixos` owns Nix as a module system:
  option declarations and types, `mkDefault` versus `mkForce`, merge
  semantics, eval-time assertions, structural option surfaces per RFC 42,
  activation ordering, and ADR 0015's three-root-unit
  rule. A finding about how the expression reads is `software`'s; a finding
  about what module evaluation will do with it is `nixos`'s. **RFC 42 sits
  wholly on the `nixos` side**, because a stringly `extraConfig` is a statement
  about what the module system can merge and type-check rather than about how
  the expression reads; an earlier revision of this document gave it to both
  seats, which is exactly the duplicate blocking finding the split exists to
  prevent. This is the one
  place in the pool where two seats read the same file on purpose, and it is
  the boundary most likely to produce a duplicate blocking finding, so both
  prompts carry it in these words. There is deliberately **no** equivalent
  overlap for Rust, Python or shell: those languages have one reviewer, and it
  is `software`.
- **`product` under the Gas City profile versus `agentic`.** Gas City-profiled
  `product` owns the **truth and normativity** of this record's claims about
  upstream software. `agentic` owns the **mechanics of what d2b itself
  authors**: `extends` rather than reimplementation, `check` as the only loop,
  bounded `max_attempts`, drain targeting, prompt contracts. Both cite the same
  pinned Gas City specifications, and the question each asks of them is
  different.

**A seat that notices something in another seat's territory reports it as an
observation in its summary, never as a `recommendation`.** There is no
non-blocking findings field in the record: `recommendations` is the blocking
channel and nothing else exists for a new observation. D21 declined to add one
and still does. `prior_resolutions` is not a counterexample: it carries a
two-member enum keyed by identifiers the controller issued, it can say nothing
the controller did not already ask about, and it is the release evidence for
findings this seat already made rather than a channel for new ones. The
observation belongs
in the seat's working notes under `.scratch/panel/<round>/`, which ADR 0053
already records as not read by the gate. A seat that wants another seat's
finding to block asks for that seat to be on the roster; it does not launder
the finding through its own verdict.

### 1.3 The verdict contract

Every prompt states the record contract in the same words, and states that the
producer writes exactly four things: `relevant`, `signoff`, `recommendations`,
and `prior_resolutions`. Roster membership, selection reason, matched rules,
surface class, round ordinal, seat profile, finding identifiers, and effective
relevance are
controller-derived and are not the seat's to assert. D21 owns the semantics;
the prompt restates only what the seat must do:

- Write `relevant: false` only when the change surface genuinely does not reach
  this seat's territory. It carries `signoff: true` and an empty
  `recommendations`, and it is a pass, not an abstention. Say in one line why,
  in the summary. Upstream Compound Engineering's selector requires the same
  thing of a skipped lane, a recorded reason rather than silence, and that is
  the one part of its design worth borrowing.
- `relevant: false` is not an exit. A seat that was relevant earlier in the
  same candidate stays on the roster whatever it writes later: the controller
  normalizes the later claim to effective relevance true, records both values,
  and keeps the seat held. Writing it is not an error and is not refused; it
  simply does not release the seat.
- `signoff` is true if and only if `recommendations` is empty. A seat that
  wants to raise something it is not willing to block on has the summary for
  it.
- `prior_resolutions` carries one entry per open prior finding identifier the
  dispatch payload gave this seat, and nothing else. See section 1.4.
- **The prompt does not tell the seat why it was selected**, and the seat must
  not infer it. D21 keeps `selection_reason` controller-side precisely so that
  a seat cannot read "you are here to satisfy a headcount" and act on it. Judge
  relevance from the change surface, not from the roster.

### 1.4 The prior-recommendation duty

D21 requires it and every prompt carries it: a seat whose earlier record on
this candidate lineage carried recommendations takes each prior recommendation
in turn and judges it **resolved or not resolved against the new delta**,
before issuing any verdict. A restated fix
and a dropped miss look identical without this, and both cost full rounds.

**The judgement is recorded twice, in two places that do different jobs.** The
prose judgement opens the summary, where a human reads it. The machine
judgement goes in `prior_resolutions`, one entry per open finding identifier,
`state` drawn from `resolved | not_resolved`, and it is what the gate reads.
Prose alone was the earlier design and it did not hold: a held seat could sign
off with a summary saying anything at all, and nothing downstream could tell
whether the finding had been addressed or forgotten.

The mechanics the prompt must state exactly:

- The dispatch payload gives the seat its **own** open finding identifiers, and
  only its own. The seat does not mint, rename or retire an identifier, and a
  record naming one it was not given is refused.
- Coverage is exact: every identifier given, once each, no others. An
  incomplete set is refused with the missing identifiers named, so a seat that
  answers three of four is told which one it skipped rather than silently
  releasing.
- `not_resolved` requires a recommendation in the same record carrying that
  identifier as its `supersedes` value. `resolved` forbids one. That is what
  keeps `signoff` true if and only if `recommendations` is empty: any
  `not_resolved` produces a recommendation and therefore a false sign-off.
- **A seat leaves the held set only on a true sign-off plus a complete
  all-`resolved` set.** A held seat with genuinely nothing further writes
  `relevant: false`, `signoff: true` and a complete all-`resolved` set, which
  is a specific claim about each finding rather than a silence that reads like
  one. Writing the `relevant: false` without the resolutions releases nothing.

Two corollaries:

- A seat newly added in a later round does not re-litigate a point an earlier
  round settled unless the new bytes reopened it. The delta ranges it is given
  are the scope. It has no open identifiers and carries an empty
  `prior_resolutions`.
- "Not resolved" is a finding and goes in `recommendations`, linked to the
  identifier it supersedes. "Resolved" is an entry plus a sentence in the
  summary. There is no third state, because the enum has two members.

### 1.5 Source hygiene

- Cite the `N` source when a finding rests on a rule; name the rule, the
  section or the identifier, not just the site.
- Deep links into other repositories are commit-pinned.
- A source marked `M` is cited with its retrieval date. If the page has moved,
  say so in the summary rather than citing an address that no longer resolves.
- Where a repository rule and an external source disagree, the repository rule
  wins, and `AGENTS.md` says so: existing code is canon.
- A convention finding cites **both** the quotable rule from a named standards
  source or repository file **and** the violating line in the delta. If either
  half is missing, the finding is dropped. This is the one rule worth importing
  wholesale from upstream Compound Engineering's project-standards reviewer.

## 2. Mandatory seats

These seven are on every panel. D21 forbids removing any of them.

Shorthand used in the source tables below, all verified at these commits on
2026-08-04:

- `AC` = `https://github.com/github/awesome-copilot/blob/dab758a392cd6b06e806c1aa0444e2bc463b32f9/`
- `SK` = `https://github.com/github/spec-kit/blob/d1e86f638277a99b82715c22c90558cd58d3cffd/`
- `GCP` = `https://github.com/gastownhall/gascity-packs/blob/0b9574272814ba175950731b73c2cc201804ee61/`
- `GC` = `https://github.com/gastownhall/gascity/blob/ad4d0ab4a9e14f57faed3eaa20a658ef743e1c09/`

Licence status of each, verified by reading the licence artifacts rather than
assumed: `github/awesome-copilot` MIT, Copyright GitHub, Inc.;
`github/spec-kit` MIT, Copyright GitHub, Inc.;
`anthropics/claude-code-security-review` MIT, Copyright 2025 Anthropic;
`anthropics/anthropic-cookbook` MIT, Copyright 2023 Anthropic;
`f/awesome-chatgpt-prompts` dual, MIT for code and **CC0 1.0 for prompt
content**; `hesreallyhim/awesome-claude-code` **CC BY-NC-ND 4.0**;
`gastownhall/gascity-packs` **has no repository LICENSE at the pinned commit**,
with MIT present only inside its two vendored subtrees,
`superpowers/vendor/superpowers/` from `obra/superpowers@6fd4507` (Copyright
2025 Jesse Vincent) and
`compound-engineering/vendor/compound-engineering-plugin/` from
`EveryInc/compound-engineering-plugin@b625049` (Copyright 2025 Every), each
recorded in an `upstream.toml`.

**One structural correction that governs every `awesome-copilot` citation
below.** At commit `dab758a3` that repository has **no `prompts/` directory and
no `chatmodes/` directory**. The historical `*.prompt.md` and `*.chatmode.md`
asset classes do not exist at this pin; assets live at `agents/*.agent.md` and
`skills/<name>/SKILL.md`. Any citation of a `prompts/` path at this pin would
404. Only assets **actually read in full** are cited below; a much larger
inventory exists at the pin and is deliberately not cited, because naming a
file by its filename is not evidence of its content, and this collection
contains files whose names badly mispredict what is in them.

### 2.1 `software`

**Purpose and scope.** The mandatory multi-language reviewer. It reviews every
candidate, in every language this repository actually contains: Rust, Python,
Bash and POSIX shell, and Nix. **Correctness first**, then structure,
readability and error handling, then local conventions, then measured
performance. D21 removed the separate `rust` seat, so this seat now also owns
the Rust depth that seat held: unsafe and FFI soundness, public API design,
Cargo SemVer classification, and workspace dependency direction. What still
belongs elsewhere: whether the code should exist at all is `simplicity`'s;
cross-component resource ownership is `reliability`'s; and the Nix **module
system** is `nixos`'s, while Nix **as code** is this seat's.

**The prompt is assembled from a shared part and a profile part, and the split
is what stops breadth from becoming shallowness.**

- The **shared part always runs**, whatever the delta contains. It is the
  correctness-first hunt, structure and error handling, local conventions, and
  measured performance, in that order. Nothing about it is conditional.
- The **profile part** is one section per language, each naming its own
  normative sources and its own duties. The profiles that run are exactly the
  ones D21's `software-*-profile` rules bound for this candidate. The seat does
  not choose them and cannot skip them; the dispatch record carries the set and
  the gate refuses a record produced under a different one.

**Profile activation, restated from D21 so a prompt author does not have to
cross-reference it.**

| Profile | Activates on |
| --- | --- |
| `rust` | `**/*.rs`, `**/Cargo.toml`, `Cargo.lock`, `rust-toolchain*.toml`, `**/BUILD.bazel`, `**/*.bzl`, `MODULE.bazel*` |
| `python` | `**/*.py`, `pyproject.toml`, `setup.py`, `setup.cfg`, `requirements*.txt` |
| `shell` | `**/*.sh`, `**/*.bash`, any extensionless path whose parent directory is named `bin` or `tools`, or an added line beginning `#!/bin/sh`, `#!/usr/bin/env sh`, `#!/bin/bash`, `#!/usr/bin/env bash`, `#!/bin/dash` or `# shellcheck shell=` |
| `nix` | `**/*.nix`, `flake.lock` |

Three consequences of that table matter to the prompt and are stated in it:

- **A mixed diff activates every applicable profile.** There is no primary
  language. A change touching `.rs`, `.sh` and `.nix` runs three profiles, and
  the defect a mixed diff most often hides is the one at the language boundary,
  which is why no election happens.
- **The Bazel files activate the Rust profile on purpose.** They are Starlark,
  not Rust, but ADR 0052 makes them the Rust build and test graph, and the
  workspace-dependency-direction duty in the Rust profile is exactly what reads
  them.
- **An empty profile set is not an abstention.** Measured 2026-08-04, `.mjs`
  under `scripts/copilot/` and `.github/skills/*/scripts/` is the one real
  source type in this tree with no profile, and `Makefile`, `.json` and
  `.toml` outside the listed manifests bind nothing either. On such a delta the
  seat runs the shared part in full and states that no profile applied. It does
  **not** write `relevant: false`, and D21's floor makes that mechanical: on a
  code-operative candidate `software` must be effectively relevant. A source
  type with no profile is a reason to review without a standards citation,
  never a reason to skip the review.

**Shared primary guidance**, cited by any profile or by none.

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| Google Engineering Practices, The Standard of Code Review | A | T2 | <https://google.github.io/eng-practices/review/reviewer/standard.html> |
| Google Engineering Practices, What to look for in a code review | A | T2 | <https://google.github.io/eng-practices/review/reviewer/looking-for.html> |
| Brendan Gregg, the USE method | A | T2 | <https://www.brendangregg.com/usemethod.html> |
| `AGENTS.md`, `docs/contributing/*` and the existing tree | N | T1 | repository-local, and binding over everything above and over every profile below |

**Premade prompt assets, read in full at the pin.**

| Asset | Licence | What transfers |
| --- | --- | --- |
| `GCP compound-engineering/agents/ce-correctness-reviewer/prompt.template.md` | MIT via vendored subtree | The core device: read code by **mentally executing it**, and five named hunt classes - off-by-one and boundary, absence and null propagation, race and time-of-check-to-time-of-use, invalid state transitions, broken error propagation. Also a "what you do not flag" list that explicitly cedes style, optimization, naming and speculative defensive checks. |
| `AC agents/gem-reviewer.agent.md` | MIT | Two rules worth taking: **review only changed lines plus their immediate context**, function scope and callers, rather than reading whole files for a small change; and reject vague acceptance criteria in favour of deterministic verification. |
| `GCP compound-engineering/agents/ce-project-standards-reviewer/prompt.template.md` | MIT via vendored subtree | Every convention finding must cite a quotable rule from a named standards file **and** the violating diff line, or be dropped. |

**Shared prompt requirements.**

- **State the review order and follow it.** First, mentally execute the changed
  control flow and hunt the five classes: boundary and off-by-one; absence and
  null propagation where the language has it, which in Rust means `Option`,
  `Result`, `unwrap`, `expect` and index panics rather than null; race and
  time-of-check-to-time-of-use; invalid state transitions; broken error
  propagation, including a swallowed error, a discarded result, and a
  predicate that returns the safe-looking value on internal failure. Second,
  structure, readability and error handling. Third, local coding, naming, file
  and directory conventions. Fourth, measured performance.
- **Correctness outranks every profile.** A profile supplies the standard that
  makes a finding citable; it does not reorder the hunt. A record whose
  findings are all convention-level or all profile-level while a logic defect
  sits in the delta has not done the work. Say so in the prompt, as a named
  failure rather than an aspiration. This is the failure that merging the Rust
  seat into this one makes newly possible, so it is stated first and stated
  twice.
- **Open the record by naming the bound profile set**, then review under each
  one. A seat that reviewed a pure-Nix diff and cites Rust guidance has not
  read the diff; a seat whose bound set includes `rust` and whose record says
  nothing under the Rust profile has not run it.
- Every blocking finding cites a file and a line range inside the delta.
- **Follow repository-local convention first**, per `AGENTS.md`: existing code
  is canon. Where no local rule exists, cite the external rule by name and
  section. Section 6 records what the local conventions actually are, which of
  them are gate-enforced and which are only observed, and the places where
  external guidance does not fit this tree.
- Performance findings are either an algorithmic or complexity claim
  demonstrable from the code, or backed by a measurement in the supplied
  validation evidence. Nothing speculative. Performance is wholly this seat's
  in every language; no other seat owns any part of it.
- State the Google standard rule in effect: approve when the change improves
  code health even if it is not perfect. Preference-only comments are
  observations, never `recommendations`.

**Shared anti-patterns and non-goals.**

- Blocking on formatting a formatter already owns: rustfmt, ruff or black,
  shfmt, nixfmt.
- Numeric style thresholds imported from a community asset. "Functions under
  20 to 30 lines", "nesting depth at most 3 to 4" and "duplication at more than
  3 matching lines" are all real numbers in surveyed T3 prompts and none of
  them is a rule here. If a threshold matters it belongs in a lint
  configuration, which is the mechanical-gate rule again.
- Re-running validations the panel was told not to re-run. Validation evidence
  is supplied.
- Reciting a profile's source list. The profiles below are a citation index for
  findings, not a checklist to walk end to end on every candidate.
- Abstraction-count and dependency-adoption arguments, which are
  `simplicity`'s; cross-component resource lifetime, which is `reliability`'s;
  module-system semantics, which are `nixos`'s.

#### 2.1a The `rust` profile

**What it owns.** Everything the removed `rust` seat owned, plus Rust-specific
performance, which was already this seat's: unsafe and FFI soundness, public
API design, the Cargo SemVer classification of a signature change, workspace
dependency direction, error-source chains and non-exhaustiveness, and Rust
naming and idiom.

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| The Rust Reference | N M | T1 | <https://doc.rust-lang.org/reference/> |
| The Rust Style Guide | N M | T1 | <https://doc.rust-lang.org/nightly/style-guide/> |
| Rust API Guidelines checklist | N | T2 | <https://rust-lang.github.io/api-guidelines/checklist.html> |
| Rust API Guidelines, naming | N | T2 | <https://rust-lang.github.io/api-guidelines/naming.html> |
| RFC 430, finalizing naming conventions | N | T1 | <https://github.com/rust-lang/rfcs/blob/master/text/0430-finalizing-naming-conventions.md> |
| Cargo Book, SemVer Compatibility | N | T1 | <https://doc.rust-lang.org/cargo/reference/semver.html> |
| The Rustonomicon | N | T1 | <https://doc.rust-lang.org/nomicon/> |
| Unsafe Code Guidelines reference | A M | T2 | <https://rust-lang.github.io/unsafe-code-guidelines/> |
| Clippy lint index | N M | T1 | <https://rust-lang.github.io/rust-clippy/master/index.html> |
| The Rust Performance Book | A | T2 | <https://nnethercote.github.io/perf-book/> |
| RustSec advisory database | N | T1 | <https://rustsec.org/> |
| Workspace lint tables in `packages/Cargo.toml` and each member manifest | N | T1 | repository-local, and binding over every row above |
| ADR 0009, Rust toolchain, MSRV, and supply-chain policy | N | T1 | repository-local, and binding |
| ADR 0052, Bazel as the Rust build and test scheduler | N | T1 | repository-local |

**The Unsafe Code Guidelines carry an explicit work-in-progress caveat and are
marked `A M` for that reason.** That document describes itself as an unfinished
effort to pin down what unsafe Rust may assume; it is not a specification and
Rust's operational semantics are not settled. A finding may use it to *frame* a
soundness argument and may not block on it alone. The Rustonomicon is the `N`
source for soundness reasoning, and the compiler's own behaviour plus a
demonstrable defect is stronger than either.

**The local unsafe perimeter is mechanical, and a prompt that does not know it
will file findings the build already prevents.** Measured 2026-08-04 from
committed code:

- `packages/Cargo.toml` is the root workspace manifest and declares
  `[workspace.lints.rust] unsafe_code = "forbid"` and
  `[workspace.lints.clippy] all = "warn"`.
- Inheritance is opt-in. 53 of the 57 crates under `packages/` carry
  `[lints] workspace = true` and are therefore under the forbid.
- Four are not. `d2b-priv-broker` and `d2b-guest-shell-runner` are named in the
  root manifest's `exclude` list; `d2b-guest-shell-runner` and
  `d2b-wlproxy-spike` each declare their own `[workspace]` and are separate
  workspaces; and `d2b-priv-broker` sets `[lints.rust] unsafe_code = "deny"`
  locally with a quarantined `src/sys.rs` carrying item-level
  `#[allow(unsafe_code)]`.
- **`d2b-host-activation-helper` is the interesting one**: it is a member of
  the root workspace, it declares no `[lints]` table at all, so it inherits
  nothing, and it contains `unsafe { libc::open }`, `unsafe { libc::fcntl }`
  and `unsafe { libc::close }` calls. A workspace member that silently opts out
  of the workspace lint posture by omission is exactly the class of defect this
  profile exists to catch, and it is recorded here as an observation about the
  tree, not as a change this record makes.
- A committed ratchet in
  `packages/d2b-contract-tests/tests/policy_efficiency_ratchet.rs` scans every
  tracked `packages/**/*.rs` and fails on a file-wide `#![allow(unsafe_code)]`,
  requiring a narrow item-level allowance instead. It carries its own negative
  fixture.

**Prompt requirements.** Tag each finding with a clippy lint identifier or an
API-guidelines item where one exists. Every `unsafe` block carries a stated
safety contract and a proof sketch, and the finding states which of the four
lint regimes above the crate is under, because a new `unsafe` block in a
`workspace = true` crate is a compile failure rather than a review finding
while the same block in `d2b-host-activation-helper` is neither. A **new**
crate without `[lints] workspace = true`, or an edit removing that line, is a
finding on its own. Public API changes are classified against the Cargo SemVer
table, and the operator-facing consequence of that classification is handed to
`product` rather than duplicated. Error types are reviewed for `source()`
chains and non-exhaustiveness. Workspace dependency direction is checked
against the crate graph the repository declares, in `packages/Cargo.toml` and
in the Bazel files ADR 0052 makes authoritative for the build. Naming findings
cite RFC 430 or the API guidelines naming chapter by item.

**Anti-patterns.** Rewriting idiomatic-but-unfamiliar code on taste; demanding
zero-`clone` purity with no measurement; **blocking on crate or feature
naming**, which the Rust API Guidelines themselves mark unsettled, in the
absence of a repository rule, per section 6.3; treating the Unsafe Code
Guidelines as normative; and filing a soundness finding against a crate the
workspace forbid already covers without saying why the forbid does not apply.

#### 2.1b The `python` profile

**What it owns.** Python style, naming, structure and typing, plus
distribution naming on the day this repository acquires a distribution.

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| PEP 8, Style Guide for Python Code | N | T1 | <https://peps.python.org/pep-0008/> |
| PEP 20, The Zen of Python | A | T1 | <https://peps.python.org/pep-0020/> |
| The Python Language Reference | N | T1 | <https://docs.python.org/3/reference/> |
| The Python Standard Library reference | N | T1 | <https://docs.python.org/3/library/> |
| `typing` module documentation, and PEP 484 for the underlying rules | N | T1 | <https://docs.python.org/3/library/typing.html> |
| PyPA specification, distribution name normalization | N | T1 | <https://packaging.python.org/en/latest/specifications/name-normalization/> |
| Section 6.4 of this document, the local `kebab-case.py` transition rule | N | T1 | repository-local, and binding over PEP 8 until the migration lands |

**PEP 20 is `A`, not `N`, and the distinction is load-bearing.** It is a
numbered PEP, which is why it is T1, and it is a list of aphorisms, which is
why it cannot make a finding blocking. "Explicit is better than implicit"
supports an argument about a specific line; it never carries one on its own.
A finding whose only authority is a Zen aphorism is dropped.

**Prompt requirements.** Map every Python style finding to a PEP 8 section by
name. Apply the transition rule in section 6.4 exactly: a **new** Python file
named `kebab-case.py` is a finding, an **existing** one is not, and neither is
a change to one; that rule is deleted the day the `snake_case.py` migration
lands. Type annotations are reviewed against the `typing` documentation where
they exist and are not demanded where the file does not use them, because
nothing in this tree requires them. Cite the standard library reference before
proposing a dependency; a hand-rolled routine that duplicates a
standard-library one is a `simplicity` observation and a `software` finding
only where the hand-rolled version is wrong. **PyPA name normalization applies
only when a distribution or package name is actually at stake**, and measured
2026-08-04 this tree has no `pyproject.toml`, `setup.py`, `setup.cfg` or
`requirements*.txt` at all, so that rule is dormant rather than live. **PEP 423
is Deferred and must never be cited as normative.**

**Anti-patterns.** Demanding type annotations on a script that has none;
blocking on formatting ruff or black would fix; renaming an existing
`kebab-case.py` file inside an unrelated candidate; citing a Zen aphorism as
authority.

#### 2.1c The `shell` profile

**What it owns.** Bash and POSIX shell correctness, quoting, error handling and
naming, in a tree that contains both dialects and one hard rule that is easy to
misread.

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| POSIX Shell Command Language, Open Group Base Specifications Issue 8 | N | T1 | <https://pubs.opengroup.org/onlinepubs/9799919799/utilities/V3_chap02.html> |
| ShellCheck wiki, per-code rationale | A | T2 | <https://www.shellcheck.net/wiki/SC2086> |
| Google Shell Style Guide, **Bash only**, and only as mature-project practice | A | T2 | <https://google.github.io/styleguide/shellguide.html> |
| Section 6.5 of this document, dialect classification and the local strict-mode, function and environment naming conventions | N | T1 | repository-local, and binding |

**Classify by shebang before citing anything.** The full rule and the measured
dialect census are in section 6.5; the profile restates the operative half. The
Google guide is a Bash guide and prescribes Bash-only constructs, so applying
it to a POSIX `sh` script produces a finding that would break the script. It is
`A` T2 and is citable only against a Bash target and only as mature-project
practice, never as a standard. Where the guide and ShellCheck agree, cite the
ShellCheck code, because a code is checkable and a guide section is not. Where
they disagree, the declared shebang and the ShellCheck code win.

**The profile is activated from the controller's interpreter fact, not from the
hunk.** D21 derives an interpreter fact for every extensionless code-operative
path in the change surface, read from the candidate snapshot rather than from
added lines, so this profile activates on **every** edit to an extensionless
shell script rather than only on the commit that introduced its shebang. Two
consequences for the prompt: the seat may be given a shell profile on a delta
whose hunks contain no shebang at all, which is correct and is not a signal
that the binding was wrong; and an `undecidable` first line binds the profile
deliberately, so a seat handed a file it cannot classify says so in the summary
rather than treating the binding as an error.

**Prompt requirements.** Determine the dialect from the shebang, or from a
`# shellcheck shell=` directive where the file is sourced rather than executed;
if neither is present, that absence is itself the finding. Map every finding to
a ShellCheck code where one exists. Check the repository's own strict-mode,
function-naming and environment-variable-naming conventions from section 6.5
before citing any external guide. **Never confuse the no-Bash CLI invariant
with a ban on shell.** ADR 0017 and `AGENTS.md` forbid a Bash **CLI fallback**;
the Rust `d2b` binary is the only operator surface and an abstract-syntax-tree
walker enforces that the Rust CLI does not invoke bash. That rule is about the
operator surface. Measured 2026-08-04 this repository commits 113
`#!/usr/bin/env bash` files and the entire Layer-1 gate is built from them, so
a seat that reads the invariant as a ban on shell in `tests/` or `scripts/` has
misapplied it, and a seat that lets a Bash bridge back into the CLI path has
missed the rule that matters.

**Anti-patterns.** Citing Bash-only constructs against a `#!/usr/bin/env sh`
target; blocking on formatting shfmt owns; treating the Google guide as
normative; filing a no-Bash-invariant finding against a contributor or test
script.

#### 2.1d The `nix` profile

**What it owns.** Nix as code: readability, naming, idiom, `with`-scope and
`let` hygiene, dead bindings, structural configuration, and file and option
naming. It does **not** own the module system; see the `software`-versus-
`nixos` boundary in section 1.2, which both prompts carry verbatim.

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| Nix Reference Manual | N M | T1 | <https://nix.dev/manual/nix/latest/> |
| NixOS Manual, Writing NixOS Modules, for the boundary rather than for the depth | N M | T1 | <https://nixos.org/manual/nixos/stable/#sec-writing-modules> |
| nix.dev best practices | N | T1 | <https://nix.dev/guides/best-practices> |
| Nixpkgs `CONTRIBUTING.md` and `pkgs/README.md`, conventions and file organisation | A | T2 | <https://github.com/NixOS/nixpkgs/blob/master/pkgs/README.md> |
| Nixpkgs `pkgs/by-name/README.md`, mechanical file and directory layout | A | T2 | <https://github.com/NixOS/nixpkgs/blob/master/pkgs/by-name/README.md> |
| RFC 166, Nix formatting, adopting nixfmt as the standard formatter | N | T1 | <https://github.com/NixOS/rfcs/blob/master/rfcs/0166-nix-formatting.md> |
| Section 6.6 of this document, and the observed `kebab-case.nix` layout of `nixos-modules/` | N | T1 | repository-local, and binding |

**RFC 42 is deliberately absent from this table.** Structural `settings`
versus stringly `extraConfig` is a statement about what the module system can
merge, type-check and report on, which is `nixos`'s territory under section
1.2. An earlier revision of this document listed it here **and** in section
3.3, instructing two seats to enforce the same rule; on a panel where one
non-empty record re-runs the whole roster, that is a manufactured duplicate
blocking finding. A stringly option noticed while reading Nix as code is an
observation for the summary, addressed to `nixos`, not a recommendation from
this profile.

**RFC 166 is the formatting boundary, and the boundary is a refusal.** It
adopted a standard Nix formatter through the RFC process, which settles
formatting as a mechanical concern. A formatting finding from this profile is
therefore never blocking; the remedy is to run the formatter. What the profile
does own is everything the formatter cannot see: a `with` over a large scope, a
`let` binding nobody uses, and a file whose name does not follow the
`kebab-case.nix` convention `nixos-modules/` observes universally.

**Prompt requirements.** Map every Nix finding to a nix.dev best practice, a
Nixpkgs convention, or the observed local layout, and say which. Where
a naming or layout question arises, prefer the argument-renaming discipline
Nixpkgs states over hiding a version constraint in an override, because a
hidden constraint is the merge-conflict failure mode in a place with no merge
tool. Option attribute paths in `camelCase` are long-standing convention and
**not** specification, so an option-naming finding is advisory unless a local
rule applies.

**Anti-patterns.** Reviewing module evaluation, option merge order and
priority, `mkDefault` versus `mkForce`, eval-time assertions, RFC 42 structural
option surfaces, activation ordering or unit proposals, all of
which are `nixos`'s and produce the duplicate blocking finding this split
exists to prevent; blocking on formatting nixfmt owns; and citing the Nixpkgs
manual's coding-conventions chapter, which is now a stub of "this section has
been moved" pointers and contains no conventions at all.

### 2.2 `test`

**Purpose and scope.** Whether the new behaviour is covered, what could regress
invisibly, whether a proposed test is worth its cost, and whether the cited
validation actually covers the change.

**Primary guidance.**

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| Software Engineering at Google, chapter 11, Testing Overview | A | T2 | <https://abseil.io/resources/swe-book/html/ch11.html> |
| The Rust Book, chapter 11, Writing Automated Tests | N | T1 | <https://doc.rust-lang.org/book/ch11-00-testing.html> |
| pytest documentation | N | T1 | <https://docs.pytest.org/en/stable/> |
| bats-core documentation | N | T1 | <https://bats-core.readthedocs.io/en/stable/> |
| Google Testing Blog, Flaky Tests at Google and How We Mitigate Them, 2016 | A | T2 | <https://testing.googleblog.com/2016/05/flaky-tests-at-google-and-how-we.html> |
| `tests/AGENTS.md` in this repository | N | T1 | repository-local, and binding over everything above |

**Premade prompt assets, read in full at the pin.**

| Asset | Licence | What transfers |
| --- | --- | --- |
| `GCP superpowers/vendor/superpowers/skills/verification-before-completion/SKILL.md` | MIT | The best "evidence before claims" artifact surveyed. A claim-to-evidence table with an explicit **not sufficient** column: "tests pass" requires command output with a failure count, not "the previous run passed"; "bug fixed" requires the original symptom retested; "regression test works" requires a demonstrated red-green cycle, not one green run; "agent completed" requires a version-control diff, not the agent's own report. |
| `GCP superpowers/vendor/superpowers/skills/test-driven-development/SKILL.md` | MIT | The verify-red discipline with three acceptance conditions - the test fails rather than errors, the failure message is the expected one, and it fails because the feature is missing rather than because of a typo - and a rationalization table that pre-refutes named excuses. The table device is the transferable part. |
| `GCP compound-engineering/agents/ce-testing-reviewer/prompt.template.md` | MIT via vendored subtree | Tests that prove the code works rather than tests that exist. False-confidence tests named as **worse than no test**: assert-no-throw, truthiness assertions, mocks verifying mocks. Brittle implementation-coupled tests, sad-path gaps, and an explicit refusal to flag coverage-percentage targets or test-style preferences. |
| `AC agents/qa-subagent.agent.md` | MIT | "Assume it is broken until proven otherwise"; six test-plan categories - happy path, boundary, negative, error handling, concurrency, security; five quality standards with "no sleep-based waits" and "no order-dependent execution" stated concretely. |
| `GCP pr-pipeline/formulas/mol-pr-review.formula.toml`, test themes | no repository licence; structure only | Diff-detectable isolation failures: environment leakage into later tests, hard-coded sleeps standing in for readiness probes, package-level state shared without capture and restore, cleanup that does not retry on transient `EBUSY`. |

**Prompt requirements.**

- For each new behaviour in the diff, name either the covering test path or the
  specific uncovered case. "Add more tests" is not a finding.
- Classify every proposed test as unit, integration, or system, with a stated
  cost and a determinism argument.
- Enumerate invisible regression classes explicitly: schema drift, ordering,
  restart and idempotency, cross-version compatibility.
- Place the test in the layer `tests/AGENTS.md` assigns. A proposal that lands
  a test in the wrong layer is wrong even if the assertion is right.
- **Audit the supplied evidence against the claim-to-evidence table; do not run
  anything.** The upstream verification skill requires the agent to run the
  command itself. Invert the actor: this seat checks that the evidence the
  integrator supplied meets the bar, and a regression test with no demonstrated
  red-green cycle in that evidence is a finding.
- Know the two coverage traps `AGENTS.md` records: Layer-1 Rust orchestration
  excludes the fixture-dependent contract crate and runs
  `test-fixture-contracts` separately, so a Rust shard is not evidence of
  fixture coverage; and an advisory job's result is never validation evidence.

**Anti-patterns and non-goals.**

- Coverage-percentage targets with no named uncovered behaviour.
- Tests that assert on implementation internals, or on log and format strings
  that are not contracts.
- Treating a green run as sufficient. This repository's canonical counter-
  example is an early observability panel that returned 0 of 8 sign-offs with
  11 HIGH findings that `tests/static.sh` caught none of.

### 2.3 `product`

**Purpose and scope.** Operator experience and contract shape: naming surface,
CLI contract and exit codes, default-off opt-in shape, migration and
deprecation policy, and whether an error message tells the operator what to do
next. Plus two territories D21 added: **scope and gap fidelity** against the
artifact's own decision and acceptance items, and **external contract
fidelity** across CLI, exit codes, wire and artifact schema, and version
discipline, including the operator upgrade path and cross-decision consistency.

**Primary guidance.**

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| Semantic Versioning 2.0.0 | N | T1 | <https://semver.org/spec/v2.0.0.html> |
| Command Line Interface Guidelines | A | T2 | <https://clig.dev/> |
| `docs/reference/error-codes.md` and the typed error contract | N | T1 | repository-local, generated |
| `docs/reference/cli-contract.md`, lifecycle FSM, signals, exit codes | N | T1 | repository-local |
| `docs/reference/compatibility.md` and `default-switch-and-deprecation.md` | N | T1 | repository-local |
| ADR index and the decision records it lists | N | T1 | repository-local |

**Premade prompt assets, read in full at the pin.**

| Asset | Licence | What transfers |
| --- | --- | --- |
| `SK templates/commands/clarify.md` | MIT | A ten-category coverage map with a Clear, Partial or Missing status per category: functional scope and behaviour; domain and data model including identity, uniqueness, lifecycle and state transitions, and scale assumptions; interaction and flow; non-functional quality attributes; integration and external dependencies; edge cases and failure handling; constraints and tradeoffs; terminology and consistency; completion signals; and placeholders. This is exactly the shape needed to judge whether a record under-specifies. Also the vague-adjective rule: "robust", "intuitive", "fast", "scalable" and "secure" are findings when unquantified. |
| `SK templates/commands/analyze.md` | MIT | A strictly read-only analysis contract with six detection passes - duplication, ambiguity, underspecification, constitution alignment, coverage gaps, inconsistency - a coverage summary table, and a determinism requirement that rerunning without changes produces consistent identifiers and counts. Also the three prohibitions: never modify files, never hallucinate a missing section, report zero issues gracefully. |
| `GCP compound-engineering/agents/ce-scope-guardian-reviewer/prompt.template.md` | MIT via vendored subtree | The symmetry that makes gap analysis a real lens: **scope exceeds goals** and **goals exceed scope** are both findings, and the first pass is "what already exists?". |

**Prompt requirements.**

- Tie every recommendation to a decision identifier or acceptance item in the
  artifact under review. A product finding that names no decision is a
  preference.
- **Enumerate coverage explicitly.** List every decision and acceptance item of
  the artifact under review and state, per item, covered or not covered by this
  delta. Then list anything in the delta that no item asked for. Both
  directions are findings; unrequested scope is not a bonus.
- **State compatibility for every changed contract.** Any change to a
  serialized type, a wire message, an artifact schema, a schema constant, a CLI
  flag or an exit code gets an explicit classification: breaking, additive, or
  version bump, with the version discipline named. Type-level Rust SemVer
  classification stays with `software`'s Rust profile; the operator-facing
  consequence is this seat's.
- Judge the default: a new surface that is on by default in a framework whose
  posture is fail-closed and opt-in needs an argument, not an omission.
- Judge the error: name the remediation the message gives, or the remediation
  it fails to give.
- Judge the migration and the upgrade path: what breaks for an existing
  operator, what they must do, and whether the record says so.
- Judge cross-decision consistency: does this contradict or silently supersede
  an accepted ADR, and if it supersedes one, does it say so in both records?

**Anti-patterns and non-goals.**

- Reopening decisions the record marks as non-goals without new evidence.
- Naming-taste findings issued as blocking.
- Reviewing implementation structure, which is `software` and `simplicity`.
- Adopting Spec Kit's quotas. Its "maximum 5 questions", "answer in at most 5
  words", "limit to 50 findings" and its four-band severity ladder are Spec Kit
  machinery, and d2b has one blocking channel.

#### 2.3a The Gas City `product` profile

D21 binds this profile whenever the candidate touches ADR 0053 or this
document. Under it, `product` is a Gas City expert: it judges whether the
record's claims about upstream behaviour are true against commit-pinned
normative sources, not against guides.

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| Gas City Pack Specification | N | T1 | `GC docs/reference/specs/pack-spec.md` |
| Formula Specification v2 | N | T1 | `GC docs/reference/specs/formula-spec-v2.md` |
| Reference index, declaring which pages are normative | N | T1 | `GC docs/reference/index.md` |
| Command Execution Trust Boundaries | N | T1 | `GC docs/reference/trust-boundaries.md` |
| Understanding Formulas | A | T2 | `GC docs/guides/understanding-formulas.md` |
| Configuring an Agent | A | T2 | `GC docs/guides/configuring-an-agent.md` |
| `build-base` formula, the stage and seam contract | N for what it declares | T4, behavioural evidence only | `GCP gascity/formulas/build-base.formula.toml` |
| `fix-loop-base` formula | N for what it declares | T4, behavioural evidence only | `GCP gascity/formulas/fix-loop-base.formula.toml` |
| Per-stage prompt assets | A | T4, behavioural evidence only | `GCP gascity/assets/workflows/build-base/<stage>.md` |
| GitHub Spec Kit planning templates | N | T1 | `SK templates/commands/plan.md` |

**Prompt requirements.**

- Cite a pinned Gas City spec or source URL for every claim about upstream
  behaviour that the finding depends on.
- Respect the normativity ladder Gas City's own reference index declares:
  `docs/reference/specs/*` is authoritative; generated reference pages are
  generated from code and are not hand-editable contracts; `docs/guides/*` are
  explanatory. A finding that contradicts a spec page loses.
- **A claim about what a stage does cites that stage's declared
  `description_file` or its formula row, never a guide.** The eleven
  `build-base` steps and the three `fix-loop-base` steps are enumerated in
  section 5.
- Check **parsed versus enforced**. ADR 0053 already relies on the distinction
  for import `version` and `requires_gc`. Verify enforcement in source at a
  pinned commit, not in prose. `build-base` declaring `internal = true`,
  `target_required = true` and `contract = "graph.v2"` is an enforcement claim
  to verify, not prose to quote.
- Distinguish claims measured at `main` from claims true of the deployed
  v1.4.0 binary. ADR 0053's P7 exists because those diverge by hundreds of
  commits, and a product finding that ignores which side of that gap a claim
  sits on is unusable.
- **Respect the licence boundary.** `gastownhall/gascity-packs` carries no
  repository LICENSE at the pinned commit. Its formulas and stage prompts may
  be read and cited as **behavioural evidence** of what upstream declares;
  neither their text nor their organisation nor their expressive structure may
  be reproduced or adapted in a d2b artifact. A finding that d2b copied one is
  this profile's to raise.

**Anti-patterns and non-goals.**

- Citing a Gas City guide as normative when a spec page covers the same point.
- Citing unpinned `tree/main` URLs for load-bearing claims.
- Reviewing d2b product surface under this profile. D1 already forbids Gas City
  from acquiring one; the profile reviews the record's upstream claims.
- Raising upstream divergence as a defect. d2b's deterministic trigger table
  deliberately diverges from Compound Engineering's model-judgment reviewer
  selector, which forbids keyword matching outright. That is a decided
  divergence with a stated rationale, not a finding.

### 2.4 `docs`

**Purpose and scope.** Diataxis placement, changelog fragments, drift between
prose and machine schema, ADR index coverage, process-marker and dash rules,
and whether binding documentation landed with the change. Plus the territory
D21 added: **intra-document coherence**.

**Primary guidance.**

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| Diataxis | A M | T2 | <https://diataxis.fr/> |
| Google developer documentation style guide | A | T2 | <https://developers.google.com/style> |
| Keep a Changelog 1.1.0 | A | T2 | <https://keepachangelog.com/en/1.1.0/> |
| Semantic Versioning 2.0.0 | N | T1 | <https://semver.org/spec/v2.0.0.html> |
| GitHub Docs, adding repository custom instructions, for `AGENTS.md` precedence | N M | T1 | <https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/add-custom-instructions/add-repository-instructions> |
| Conventional Commits 1.0.0 | A | T2 | <https://www.conventionalcommits.org/en/v1.0.0/> |
| `AGENTS.md` and `docs/contributing/*` in this repository | N | T1 | repository-local, and binding over everything above |

**Premade prompt assets, read in full at the pin.**

| Asset | Licence | What transfers |
| --- | --- | --- |
| `GCP compound-engineering/agents/ce-coherence-reviewer/prompt.template.md` | MIT via vendored subtree | The coherence lens, and it is the reason D21 gave this territory to `docs`: contradiction between sections, terminology drift, forward references to things never defined, broken internal references, and genuine ambiguity defined operationally as "statements two careful readers would interpret differently". Also ungrouped multi-concern requirement lists treated as a structural finding. |
| `SK templates/commands/clarify.md` | MIT | The write-back discipline: when a clarification invalidates an earlier ambiguous statement, **replace** that statement rather than adding a correction beside it, and leave no obsolete contradictory text. Directly applicable to a self-amending ADR. Also the diff-hygiene rule of changing only the markers whose state actually changed. |

**Prompt requirements.**

- Classify every added or changed document by Diataxis mode and reject
  mixed-mode pages.
- Check that documented schema and machine schema agree; markdown-to-JSON drift
  is a named repository concern.
- Require changelog fragments and `AGENTS.md` updates to land in the same
  change as load-bearing behaviour.
- Enforce the repository's two mechanical prose rules: ASCII hyphen only, and
  process markers out of shipped artifacts. Both have gates in
  `tests/tools/tier0-first-pass.sh`; a docs finding should name the gate.
- Require commit-pinned deep links into other repositories.
- **Read the document for coherence, not just for placement.** Name
  contradictions between sections, terminology that drifts between two names
  for one concept, forward references to a thing the document never defines,
  cross-links that do not resolve, and statements two careful readers would read
  differently. On a self-amending record, check that an amendment **replaced**
  the statement it invalidates rather than leaving both.
- Cross-decision consistency and supersession belong to `product`. This seat
  owns coherence **within** the artifact.

**Anti-patterns and non-goals.**

- Prose nitpicks raised as blocking `recommendations`.
- Requesting a new document where an existing page in the correct quadrant
  should be extended.
- Importing an upstream asset's autofix classification. Compound Engineering's
  coherence reviewer names patterns as safe to fix automatically; d2b has no
  autofix pipeline and this seat proposes, it does not edit.

### 2.5 `security`

**Purpose and scope.** Security engineering combined with an adversarial
penetration-tester mindset. This seat does not produce hardening checklists. It
states an attacker model and then tries to reach something with it, across
code, design, and architecture.

**Primary guidance.**

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| NIST SP 800-115, Technical Guide to Information Security Testing and Assessment | N | T1 | <https://csrc.nist.gov/pubs/sp/800/115/final> |
| NIST SP 800-218, Secure Software Development Framework v1.1, 2022 | N | T1 | <https://csrc.nist.gov/pubs/sp/800/218/final> |
| MITRE CWE and the CWE Top 25 | N | T1 | <https://cwe.mitre.org/top25/> |
| OWASP Application Security Verification Standard | A M | T2 | <https://owasp.org/www-project-application-security-verification-standard/> |
| OWASP Web Security Testing Guide | A M | T2 | <https://owasp.org/www-project-web-security-testing-guide/> |
| OWASP Threat Modeling process | A | T2 | <https://owasp.org/www-community/Threat_Modeling_Process> |
| OWASP Cheat Sheet Series | A | T2 | <https://cheatsheetseries.owasp.org/> |
| RustSec advisory database | N | T1 | <https://rustsec.org/> |
| Gas City trust boundaries, for system-specific posture | N | T1 | `GC docs/reference/trust-boundaries.md` |
| `SECURITY.md` and `docs/explanation/design.md` threat model | N | T1 | repository-local |

**Premade prompt assets, read in full at the pin.**

| Asset | Licence | What transfers |
| --- | --- | --- |
| `anthropics/claude-code-security-review`, `claudecode/prompts.py`, pinned at `0c6a49f1fa56a1d472575da86a94dbc1edb78eda` | MIT, Copyright 2025 Anthropic | The closest first-party analogue to d2b's finding bar that exists in public. Four transferable devices: **scope the review to the delta**, explicitly not a general code review and not a place to raise pre-existing concerns; a stated preference for **misses over noise**, with the bar "something a security engineer would confidently raise in a pull request review"; an explicit **out-of-territory exclusion list**, stated twice, which is first-party evidence that naming what a reviewer must not report is as load-bearing as naming what it must; and a three-phase method - research the repository's existing security patterns, compare the new code against **those** patterns rather than an external ideal, then trace data flow from input to sensitive operation. Every finding carries an exploitation scenario, not just a description. |
| `AC agents/se-security-reviewer.agent.md` | MIT | A "step 0" that builds a targeted review plan before any finding: classify the code type, classify the risk level, then select a small number of relevant check categories. This matches d2b's requirement that the seat state an adversary model first. |
| `GCP pr-pipeline/formulas/mol-pr-review.formula.toml`, fail-closed theme | no repository licence; structure only | A precise, diff-detectable articulation of "prefer a check that denies to a check that warns": a boolean predicate used as a safety gate that returns the safe-looking value on internal error, a discarded write result, and catch-and-ignore. |

**Prompt requirements.**

- State the adversary model for this review before any finding: assets, the
  trust boundaries this delta crosses, and the capability the attacker is
  assumed to have. In this repository the standing model includes a principal
  that already holds the `gascity` uid, because ADR 0053 D10 says agents share
  it.
- Every blocking finding carries a CWE identifier or an ASVS requirement
  identifier, plus a concrete exploitation path: preconditions, steps,
  observable impact. "Could be unsafe" is not a finding.
- Probe design and architecture, not only code: authorization boundaries,
  time-of-check-to-time-of-use, confused deputy, capability and syscall sets,
  secret and personal-data flow into telemetry, retention defaults, and the
  failure mode of each check under partial failure.
- Prefer a check that denies to a check that warns, and say which one the
  delta implements. A degrading check is itself the finding.
- Judge the delta against **this repository's** established security patterns
  first, per the middle phase of the Anthropic prompt and per `AGENTS.md`:
  existing code is canon.
- Dependency advisories cite an advisory identifier and the affected version
  range.
- State the residual risk being accepted when signing off.

**Anti-patterns and non-goals.**

- Generic top-ten recitations with no mapping to a line in the delta.
- Treating an absent hardening control as a finding when the threat model does
  not reach it.
- Proposing exploit code, or exploitation against any live third-party system.
  This seat reasons about exploitability from the artifacts supplied to it.
- Duplicating `observability`'s label and telemetry findings without adding an
  exploitation path.
- **Importing the Anthropic prompt's exclusion list.** It excludes denial of
  service, secrets stored on disk, rate limiting and resource exhaustion, and
  input validation on non-security-critical fields, because other processes
  cover those in the pipeline it was written for. In d2b, reachability-shaped
  denial of service belongs to `networking`, resource behaviour to
  `reliability` and `observability`, and on-disk secrets squarely to this seat.
  Copying that list would create real blind spots.
- Importing its numbers: the "more than 80 percent confident" bar, the
  confidence bands, and the three-level severity ladder are all out of scope
  for a record with one blocking channel.

### 2.6 `observability`

**Purpose and scope.** Metric label cardinality, span attribute hygiene, log
and audit shape, retention, redaction, and exporter correctness.

**Primary guidance.**

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| OpenTelemetry semantic conventions, Naming, marked Stable | N | T1 | <https://opentelemetry.io/docs/specs/semconv/general/naming/> |
| OpenTelemetry attribute registry | N | T1 | <https://opentelemetry.io/docs/specs/semconv/registry/attributes/> |
| Prometheus metric and label naming practices | A | T2 | <https://prometheus.io/docs/practices/naming/> |
| Prometheus instrumentation practices | A | T2 | <https://prometheus.io/docs/practices/instrumentation/> |
| Prometheus histograms and summaries | A | T2 | <https://prometheus.io/docs/practices/histograms/> |
| Google SRE Book, Monitoring Distributed Systems | A | T2 | <https://sre.google/sre-book/monitoring-distributed-systems/> |
| Google SRE Workbook, Alerting on SLOs | A | T2 | <https://sre.google/workbook/alerting-on-slos/> |
| ADR 0026 and `docs/reference/daemon-metrics.md` | N | T1 | repository-local, and binding |

**Premade prompt assets: none suitable, and this is a verified result.**
Enumerating `github/awesome-copilot` `agents/` and `skills/` at the pin, the
Compound Engineering agent set, the Superpowers skill set, and Spec Kit's
templates found **only vendor-specific** observability assets - Elasticsearch,
Dynatrace, New Relic, Application Insights, Arize, Phoenix - and **no
vendor-neutral review rubric for metric, span or log shape, cardinality or
retention** anywhere. Spec Kit's clarification taxonomy names observability as
a category and supplies no criteria for it. This seat is therefore authored
from the OpenTelemetry and Prometheus sources above plus this repository's own
metrics reference, and none of the vendor assets is adopted.

**Prompt requirements.**

- Require a cardinality budget statement for every new label or attribute: a
  bounded value set, or a named upper bound with the argument for why it is
  safe. ADR 0053 D17 already requires label values to come from closed
  enumerations, so an unbounded label is a rule violation, not an opinion.
- Require names to conform to OpenTelemetry naming rules and Prometheus naming
  and unit conventions, and to reuse a registry attribute where one exists.
- Require an explicit no-secrets check on span attributes and log lines: no
  credentials, command output, store paths, or raw identifiers.
- Require every alert-relevant signal to state its consumer: page, ticket, or
  dashboard only.

**Anti-patterns and non-goals.**

- Unbounded labels: identifiers, paths, user input, error strings.
- Quantiles computed over pre-aggregated summaries across instances.
- Metrics with no stated question they answer.
- Citing a vendor observability agent as if it were a neutral rubric.

### 2.7 `simplicity`

**Purpose and scope.** Favour the simplest maintainable implementation that
meets the stated requirements. Favour reuse of a mature, supported, performant
library or crate where one meets the requirements. Prevent reinvented wheels,
needless indirection, and needless wordiness. Reduce lines of code where the
reduction lowers risk and improves clarity.

This seat exists because none of the others own deletion. `software` reviews
what is written; `simplicity` asks whether it should have been written. D21
makes it mandatory, which means it must have something to say about a
documentation-only candidate, so it carries **two lenses**.

**The code lens** is the charter above. **The artifact lens** applies to an
ADR, a specification, a plan or a contributor document, and asks four
questions:

1. **Is the decision surface minimal?** Does the record decide more than it
   must to unblock the work, and does each decided property carry a stated
   reason it could not be deferred?
2. **Does it reinvent upstream behaviour?** Where the record specifies a
   mechanism that a named upstream already provides, does it say why the
   upstream one does not fit, or does it simply not mention it?
3. **Is a rejected alternative being reintroduced?** A record that re-adds a
   previously rejected option without new evidence is the prose equivalent of
   a reverted commit landing again.
4. **Is a contract stated once?** Duplicated prose, a rule stated both in prose
   and in a schema without one being derived from the other, and a paragraph
   that restates the paragraph above it are all the same defect: two places to
   change and one of them will be missed.

**Primary guidance.**

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| Russ Cox, Our Software Dependency Problem | A | T2 | <https://research.swtch.com/deps> |
| OpenSSF Concise Guide for Evaluating Open Source Software | A | T2 | <https://best.openssf.org/Concise-Guide-for-Evaluating-Open-Source-Software> |
| OpenSSF Concise Guide for Developing More Secure Software | A | T2 | <https://best.openssf.org/Concise-Guide-for-Developing-More-Secure-Software> |
| SLSA v1.1 levels | N | T1 | <https://slsa.dev/spec/v1.1/levels> |
| RustSec advisory database | N | T1 | <https://rustsec.org/> |
| PEP 20, The Zen of Python | N | T1 | <https://peps.python.org/pep-0020/> |
| Martin Fowler, Yagni | A | T2 | <https://martinfowler.com/bliki/Yagni.html> |
| ADR 0035, efficiency and simplification roadmap | N | T1 | repository-local |
| ADR 0009, Rust toolchain, MSRV, and supply-chain policy | N | T1 | repository-local, and binding on any dependency proposal |

**Premade prompt assets, read in full at the pin.**

| Asset | Licence | What transfers |
| --- | --- | --- |
| `GCP compound-engineering/agents/ce-maintainability-reviewer/prompt.template.md` | MIT via vendored subtree | "Delete complexity rather than rearrange it", complexity moved rather than removed, thin wrappers and identity abstractions, premature abstraction defined as an interface with one implementor, and the demand for a **concrete reframe** in the fix rather than "consider refactoring". |
| `AC agents/gem-code-simplifier.agent.md` | MIT | Four-axis analysis taxonomy - dead code, complexity, duplication, naming; an explicit **Chesterton's Fence** gate requiring history and tests to be consulted before any removal; an **impact triage** rule requiring the blast radius of exported symbols to be stated before proposing a change; a **do not refactor** list covering working code that will not change, critical code with no tests, and time pressure; and a public-contract guard treating exported functions, schemas, configuration keys and event names as contracts unless proven private. |

**Prompt requirements.**

- Every finding names the concrete deletion or collapse being asked for: what
  to delete, what to merge, what to move. "Consider refactoring" is not a
  finding.
- **State which lens produced the finding**, code or artifact. A prose finding
  under the code lens is a category error and will read as noise.
- Reuse versus rewrite: where the delta hand-rolls behaviour a mature library
  already provides, name the specific library and version and evaluate it
  against the OpenSSF criteria: maintenance activity, release cadence, advisory
  history, transitive footprint, licence.
- Symmetrical scrutiny in the other direction. Any dependency recommendation,
  whether adding one or keeping one, must carry evidence on six axes:
  **maintenance** status, **advisory** history, **licence** compatibility,
  **transitive** cost, **build and runtime weight**, and **performance**. A
  recommendation missing any axis is incomplete and is not blocking.
  ADR 0009 governs what may actually be added; this seat proposes, it does not
  waive.
- Apply the Chesterton's Fence and blast-radius gates before proposing any
  removal: consult history and tests, and state which symbols cross a file
  boundary.
- Lines-of-code reduction claims are justified by clarity or risk reduction,
  with the resulting code still readable and still covered.
- Carry severity anchors and a confidence floor, and suppress below the floor,
  so this seat cannot veto on taste. **The floor is repository-tuned and stated
  in the prompt**; it is not inherited from any upstream asset.
- Carry an explicit "what I do not flag" section in the prompt itself.

**Anti-patterns and non-goals.** These are rejections, not preferences. A
recommendation that does any of the following is itself the defect:

- **Code golf.** Density, clever one-liners, or removed clarity sold as fewer
  lines.
- **Lost validation, error handling, tests, or observability.** Deleting a
  bounds check, an error branch, a test, a log line, or a metric is not
  simplification. It is a behaviour change with a smaller diff.
- **Dependency sprawl.** Adding a crate or package to save a handful of lines,
  or adding several small dependencies where one already present would do.
- **Complexity laundering.** Moving complexity behind a dependency, a macro, a
  generic, or a configuration surface and calling the result simpler.
- **Unsupported or unmaintained dependencies.** A dependency with no recent
  releases, no advisory response history, or an incompatible licence fails on
  its own account whatever it saves.
- **Abstraction churn.** A refactor that redistributes the same concepts across
  more files.
- Style or naming taste issued as blocking.
- Complexity that mirrors genuine domain complexity, an abstraction with
  multiple real consumers, or a framework-mandated pattern. None of those are
  findings.
- **Line counters.** The upstream simplifier emits removed-line and
  changed-line counts. Adopting them pushes the seat toward code golf, which is
  the first rejection on this list.

Note on both upstream precedents: they are product code, they can change
without notice, and their concrete numbers are tuned for other codebases. The
maintainability reviewer's file-size thresholds and delegation-hop limit, and
the simplifier's duplication line count, are all refused. Adopt the structures:
severity anchors plus a confidence floor plus an explicit non-flag list, the
fence gate, the blast-radius rule, and the do-not-refactor list.

## 3. Optional seats

D21 selects these by trigger. At least three of the five are on every
code-operative panel and at least one on every documentation-only panel; all
five can be. The fill order when triggers do not reach the floor is
`[reliability, agentic, nixos, networking, kernel]`.

There is no language seat in this list other than `nixos`, and that is
deliberate: D21 removed the `rust` seat and moved its territory into
`software`'s Rust profile. `nixos` survives as a seat because the NixOS module
system is a different question from Nix code quality, not a deeper reading of
the same documents.

### 3.1 `reliability`

**Purpose and scope.** Resource ownership and cleanup on error and crash paths;
restart, adoption and idempotency; ordering and concurrency across components;
partial failure and degraded-state behaviour; and on-disk state and schema
migration. The question this seat asks that no other seat asks is: who owns
this resource, who releases it when this process dies here, and what does the
persisted state mean afterwards?

**The boundary, restated because it is the whole justification for the seat.**
`kernel` owns syscall and kernel-interface semantics and version floors, so
whether `pidfd_open` was called correctly is `kernel`'s and whether the
descriptor is closed on the error branch three frames up is this seat's.
**Descriptor inheritance and lock semantics moved to `kernel` with this
revision**: whether `O_CLOEXEC` was set, and whether an open file description
lock behaves the way the code assumes across a `fork` or a second `close`, are
questions about what the syscall does, and they are cited from the same man
pages `kernel` already owns. What stays here is the cross-component lifecycle
question those primitives serve: who holds this descriptor, who releases this
lock when the process dies here, and what the next start-up finds.
`software` owns in-function correctness and error propagation, so a swallowed
`Result` is `software`'s and an unreleased lock across a component boundary is
this seat's. `test` owns whether a restart path is covered, which presumes
somebody already decided what the restart path should be; deciding that is this
seat's. `product` owns the operator-facing migration and compatibility
experience; the on-disk state transition that migration performs is this
seat's.

**Primary guidance.**

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| Google SRE Book, Managing Critical State and Handling Overload | A | T2 | <https://sre.google/sre-book/managing-critical-state/> |
| Google SRE Workbook, Addressing Cascading Failures | A | T2 | <https://sre.google/sre-book/addressing-cascading-failures/> |
| `rename(2)`, atomic replacement semantics, for the durability ordering rather than the errno set | N | T1 | <https://man7.org/linux/man-pages/man2/rename.2.html> |
| `fsync(2)` and durability ordering | N | T1 | <https://man7.org/linux/man-pages/man2/fsync.2.html> |
| The Rustonomicon, on panics, unwinding and `Drop` | N | T1 | <https://doc.rust-lang.org/nomicon/exception-safety.html> |
| ADR 0034, storage lifecycle, restart and synchronization | N | T1 | repository-local, and binding |
| ADR 0011, cgroup v2 delegation and pidfd handoff | N | T1 | repository-local, and binding |
| ADR 0040, graceful VM shutdown | N | T1 | repository-local |
| ADR 0049, store-owned mutation seal | N | T1 | repository-local |
| `docs/explanation/daemon-lifecycle.md` | N | T1 | repository-local |

`open(2)` and `fcntl(2)` were listed here in an earlier revision and now sit
with `kernel` in section 3.5, which is where descriptor inheritance and OFD
lock semantics belong. This seat still reasons about the resources those calls
produce; it cites `kernel`'s finding rather than restating the man page.

**Premade prompt assets: none found, and the nearest neighbour is a
misleading match.** Compound Engineering carries a conditional `reliability`
lane, and its reviewer is scoped to single-boundary error handling rather than
to cross-component resource ownership; adopting it would recreate the overlap
with `software` that this seat exists to avoid. `GCP
pr-pipeline/formulas/mol-pr-review.formula.toml` carries no repository licence
and is therefore **behavioural evidence only**: it establishes that upstream's
review formula scores resource lifecycle and concurrency separately, which is
worth knowing about a system this repository integrates with, and its
categories, wording and organisation are not available to adapt. The prompt
requirements below are derived instead from ADR 0034, ADR 0011, ADR 0040 and
ADR 0049 and from `AGENTS.md`'s single-repair-owner rule, which is the correct
provenance for a seat whose whole subject is this repository's own decisions.

**Prompt requirements.**

- **Walk every acquisition in the delta to its release.** For each file
  descriptor, lock, lease, mount, child process, cgroup, temporary path,
  socket, thread and task the delta creates, name where it is released on the
  success path, on each error path, and on abnormal termination. An acquisition
  with no named release on some path is a finding, and the finding names the
  path.
- **State the crash-point question.** Pick the two or three points in the delta
  where a `SIGKILL` between two operations leaves observable state, and state
  what the next start-up sees and does. ADR 0034's rule is adopt before
  cleanup; a delta that cleans up first is a finding against a named ADR.
- **Judge idempotency explicitly.** Running the changed operation twice, and
  running it once after a partial previous run, either converges or is a
  finding. Say which.
- **Name the ordering and concurrency assumptions.** Which operations must be
  ordered, what enforces the order, and what happens when two instances race.
  Where a delta adds a spawn, a lock or shared mutable state, the assumption is
  stated or it is a finding.
- **Judge partial failure and degraded state.** What does the component report
  when half its work succeeded? ADR 0034 requires typed degraded-state
  reporting rather than silence or a broad repair sweep.
- **Judge on-disk state and schema migration.** A changed persisted shape, a
  changed schema constant, or a changed path layout gets a stated read path for
  the old shape: migrate, refuse with a typed error, or a decided clean break.
  A silent reinterpretation of old bytes is the finding this seat exists for.
  The operator-facing half of the same change belongs to `product`.
- Cite the ADR when one governs. Storage, locking, adoption and cleanup are
  ADR 0034's; pidfd and cgroup handoff are ADR 0011's; shutdown ordering is
  ADR 0040's.

**Anti-patterns and non-goals.**

- Restating a `software` finding about an in-function error branch without the
  cross-component ownership question that makes it this seat's.
- Demanding retries, timeouts or circuit breakers with no named failure mode
  they address. A resilience pattern proposed for its own sake is taste.
- Reviewing syscall correctness, descriptor inheritance, lock or signal
  semantics, or kernel version floors, which are `kernel`'s.
- Proposing a second repair owner. `AGENTS.md` requires every host-mutable path
  or lock surface to name a single repair owner and route repair through it;
  "add a cleanup pass" is usually a finding against that rule rather than a
  fix.

### 3.2 `agentic`

**Purpose and scope.** Expert in GitHub Copilot and in Gas City: custom agents,
instruction layering, prompt files, tool restriction, durable formula
orchestration, review loops, handoffs, and the substitution of mechanical gates
for prompt-only assurances. Concretely, the surfaces this seat owns in this
repository are `.github/agents/**`, `.github/skills/**`, `.github/prompts/**`,
`.github/instructions/**`, `.github/copilot-instructions.md`,
`scripts/copilot/**`, every `AGENTS.md`, and any Gas City formula, pack or
prompt template the repository authors.

**Primary guidance.**

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| GitHub Docs, About custom agents | N M | T1 | <https://docs.github.com/en/copilot/concepts/agents/cloud-agent/about-custom-agents> |
| GitHub Docs, Adding repository custom instructions | N M | T1 | <https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/add-custom-instructions/add-repository-instructions> |
| GitHub Docs, About customizing GitHub Copilot responses | N M | T1 | <https://docs.github.com/en/copilot/concepts/prompting/response-customization> |
| GitHub Docs, custom instructions support matrix | N M | T1 | <https://docs.github.com/en/copilot/reference/custom-instructions-support> |
| GitHub Docs, Best practices for using Copilot to work on tasks | A M | T1 | <https://docs.github.com/en/copilot/tutorials/cloud-agent/get-the-best-results> |
| VS Code, Use custom instructions | N M | T1 | <https://code.visualstudio.com/docs/agent-customization/custom-instructions> |
| VS Code, Use prompt files | N M | T1 | <https://code.visualstudio.com/docs/agent-customization/prompt-files> |
| Anthropic, Best practices for Claude Code | N M | T1 | <https://code.claude.com/docs/en/best-practices> |
| Anthropic, Create custom subagents | N M | T1 | <https://code.claude.com/docs/en/sub-agents> |
| Anthropic, Prompt engineering overview | N M | T1 | <https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/overview> |
| Anthropic, Prompting best practices | N M | T1 | <https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-prompting-best-practices> |
| Anthropic, prompt engineering interactive tutorial | N | T1 | <https://github.com/anthropics/prompt-eng-interactive-tutorial> |
| Anthropic Cookbook, metaprompt notebook | N | T1, MIT | <https://github.com/anthropics/claude-cookbooks/blob/main/misc/metaprompt.ipynb> |
| Anthropic, Writing effective tools for AI agents, 2025-09-11 | A M | T1 | <https://www.anthropic.com/engineering/writing-tools-for-agents> |
| `AGENTS.md` cross-tool convention | A | T2 | <https://agents.md/> |
| Gas City Formula Specification v2 | N | T1 | `GC docs/reference/specs/formula-spec-v2.md` |
| Gas City Pack Specification | N | T1 | `GC docs/reference/specs/pack-spec.md` |
| Gas City, Understanding Formulas | A | T2 | `GC docs/guides/understanding-formulas.md` |
| Gas City, Configuring an Agent | A | T2 | `GC docs/guides/configuring-an-agent.md` |
| `docs/contributing/copilot-agents.md` and `panel-review.md` | N | T1 | repository-local, and binding |

Three first-party statements worth carrying into the prompt because they change
what the seat may accept:

- Subagents run in their own context with their own system prompt and tool
  access, and tool restriction is named as a way to **enforce constraints**,
  not merely to save tokens. That makes an unrestricted `tools` list on a
  narrow agent a reviewable choice.
- Instruction inheritance is **agent-dependent**: Anthropic documents that its
  built-in Explore and Plan subagents skip the repository instruction file that
  every other subagent loads. That is first-party evidence for this seat's
  standing rule that a design may not rely on ordering or inheritance between
  instruction files.
- The verification framing: give the agent a check it can run, and have it show
  evidence rather than assert success, because without a runnable check
  "looks done" is the only available signal. That is the same argument as this
  seat's mechanical-gates rule, from the vendor.

**Premade prompt assets, read in full at the pin.**

| Asset | Licence | What transfers |
| --- | --- | --- |
| `GCP compound-engineering/agents/ce-agent-native-reviewer/prompt.template.md` | MIT via vendored subtree | An anti-pattern table with named failure modes - orphan feature, context starvation, sandbox isolation, silent action, capability hiding, workflow tool, decision input - and the design principles behind them: action parity, context parity, shared workspace, primitives over workflows, dynamic context injection. |
| `AC agents/agent-governance-reviewer.agent.md` | MIT | A six-item review checklist: policy checks on tool functions, input scanned before agent processing, no credentials in agent configuration, audit logging for tool calls and governance decisions, rate limits on tool calls, trust boundaries between agents. Plus two rules matching this repository's posture exactly: fail closed and deny on ambiguity, and prefer explicit allowlists to blocklists. |
| `AC skills/agent-governance/SKILL.md` | MIT | The policy composition rule that most restrictive wins, and the pipeline framing from request through intent classification and policy check to tool execution and audit log. |
| `GCP compound-engineering/agents/ce-code-review-selector/prompt.template.md` | MIT via vendored subtree | Read for **contrast**, not adoption. It selects conditional reviewers by model judgment and forbids keyword matching outright; d2b chose a deterministic versioned table for exactly the auditability that choice gives up. The one rule worth borrowing is that a skipped lane produces an explicit artifact with a reason rather than silence, which is the same shape as d2b's `relevant: false`. |
| `GCP compound-engineering/agents/ce-plan-review-synthesizer/` and `ce-code-review-synthesizer/prompt.template.md` | MIT via vendored subtree | Four verbs for any synthesis seam: deduplicate findings, distinguish required changes from residual risk, suppress non-actionable noise, produce the single signal the downstream check consumes. Both also carry an isolation clause forbidding the reviewer from invoking provider-native subagents, slash commands or the upstream plugin runtime, which is directly transferable to d2b seats. |

**Prompt requirements.**

- Validate agent-profile mechanics: correct directory, correct filename, valid
  YAML frontmatter, a real `description`, a stated rationale for the `tools`
  restriction, and a scope level that matches intent.
- Validate instruction layering: repository-wide versus path-specific versus
  the nearest `AGENTS.md`, no contradictory rules, and **no reliance on
  ordering or inheritance between instruction files**, because the vendor
  documentation guarantees neither and documents at least one agent that skips
  the repository file entirely.
- Validate Gas City correctness: `extends` rather than reimplementation,
  `check` as the only loop, bounded `max_attempts`, correct drain and convoy
  targeting, `prompt.template.md` naming, directory agents rather than inline
  agent tables in new packs, and no inlined secrets.
- Validate durability: does the change keep work recoverable across session
  crashes, and is the handoff shape explicit and matched to the fix loop?
- Treat prompt text as a contract surface: byte-identity requirements,
  versioning, and where drift would be detected. This repository already
  enforces byte-identical seat copies of the finding bar.
- **Replace prompt-only assurances with mechanical gates.** Where a change
  says "the agent is instructed not to", the finding is: what check makes that
  true? A vendor statement that the model may not follow instructions
  identically every time is on the record, so an instruction is not a control.
- Validate the licensing and provenance of any prompt material the change
  introduces, against section 0.2. A prompt fragment with no stated source is a
  finding.

**Anti-patterns and non-goals.**

- Accepting a prompt-only remedy where a check script or lint can enforce the
  rule.
- Relying on instruction ordering, or on deterministic instruction-following.
- Introducing an unbounded agent loop, or a loop outside `check`.
- Granting an agent profile every tool when the task needs a narrow set.
- Citing IDE-specific behaviour as if it applied to the cloud agent or the CLI
  without checking the support matrix.
- Raising d2b's deterministic roster selection as a divergence defect. It is a
  decided divergence from upstream with a stated rationale.
- Copying Gas City runtime vocabulary into a d2b prompt. Bead command text,
  metadata key names, schema identifiers, retry budgets and convoy vocabulary
  are upstream contracts and are meaningless here. Section 5.5 enumerates them.

### 3.3 `nixos`

**Purpose and scope.** Module wiring, option declarations, `mkDefault` and
`mkForce` correctness, eval-time assertions, activation ordering, merge
semantics and priority, structural option surfaces per RFC 42, and
NixOS-specific correctness. **Not** general Nix code quality: readability,
naming, idiom, `with`-scope and `let` hygiene and the formatter boundary are
`software`'s Nix profile, per section 2.1d and the boundary in section 1.2.
**RFC 42 belongs to this seat alone**; an earlier revision of this document
assigned it to both seats, which invites two blocking findings for one defect
on a panel where any non-empty record re-runs the entire roster.
This is the one seat pair in the pool that reads the same file on purpose, so
both prompts carry the split in the same words: how the expression reads is
`software`'s, what module evaluation does with it is this seat's.

**Primary guidance.**

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| NixOS Manual, Writing NixOS Modules | N M | T1 | <https://nixos.org/manual/nixos/stable/#sec-writing-modules> |
| nix.dev module system tutorial | N | T1 | <https://nix.dev/tutorials/module-system/> |
| RFC 42, structural `settings` instead of stringly `extraConfig` | N | T1 | <https://github.com/NixOS/rfcs/blob/master/rfcs/0042-config-option.md> |
| nix.dev best practices | N | T1 | <https://nix.dev/guides/best-practices> |
| Nixpkgs `CONTRIBUTING.md` and `pkgs/README.md`, conventions and file organisation | A | T2 | <https://github.com/NixOS/nixpkgs/blob/master/pkgs/README.md> |
| Nixpkgs `pkgs/by-name/README.md`, mechanical file and directory layout | A | T2 | <https://github.com/NixOS/nixpkgs/blob/master/pkgs/by-name/README.md> |
| Nixpkgs Manual | N M | T1 | <https://nixos.org/manual/nixpkgs/stable/> |
| Nix Reference Manual | N M | T1 | <https://nix.dev/manual/nix/latest/> |
| ADR 0015, daemon-only clean break | N | T1 | repository-local, and binding on any unit proposal |
| `nixos-modules/assertions.nix` and the eval-time assertion row in `docs/contributing/critical-subsystems.md` | N | T1 | repository-local, and binding |
| `AGENTS.md` critical-subsystem index, the net VM `lib.mkForce` row and the do-not-delete-an-assertion rule | N | T1 | repository-local, and binding |

**Premade prompt assets: none exist, and this is a verified enumeration
result.** Searching `github/awesome-copilot` at commit `dab758a3` across its
`agents/` directory of roughly 215 files and its `skills/` directory of roughly
380 directories found **no Nix or NixOS asset of any kind**; the nearest
neighbours are Arch, Debian, CentOS and Fedora administration helpers.
Compound Engineering's agent set contains no Nix reviewer. Superpowers has no
language-specific reviewer other than test-driven development. Spec Kit is
language-agnostic. This seat is therefore authored from the normative sources
above plus this repository's own `nixos-modules/` conventions, and nothing is
adapted from a premade prompt.

**Prompt requirements.** Option types and defaults declared with descriptions;
activation ordering checked; new configuration surfaces expressed as structural
`settings` per RFC 42; module evaluation free of impure or ambient state; and
any proposed unit checked against ADR 0015's three-root-unit rule before
anything else. Where a naming or layout question arises, prefer the
argument-renaming discipline Nixpkgs states over hiding a version constraint in
an override, because a hidden constraint is the merge-conflict failure mode in
a place with no merge tool.

**Eval-time assertions are a contract with consumers, and the prompt says so.**
`nixos-modules/assertions.nix` is a critical subsystem in this repository's own
index: the assertions there are what a consumer's evaluation hits before
anything is built, and they are the reason a misconfiguration fails at eval
rather than at runtime on a host. The seat is required to check, on any delta
that touches an option surface or an invariant:

- **Does a new consumer-visible invariant have an assertion?** A rule enforced
  only by prose, or only by a runtime check inside the daemon, is a rule a
  consumer discovers after deployment. State which assertion carries it.
- **Was an existing assertion weakened, narrowed or deleted?** `AGENTS.md` is
  explicit: do not paper over a failing assertion by deleting it; if the
  predicate is wrong, fix the predicate, and if the predicate is right but the
  message misleads, fix the message. A delta that removes or narrows a
  predicate without an argument that the predicate was wrong is a blocking
  finding, and the finding names the invariant that stopped being checked.
- **Is the failure message actionable?** An assertion that fires and names no
  corrective action costs a consumer the same debugging session an unasserted
  invariant would have.
- **`assertions` versus `warnings`.** A warning is not a control. Choosing a
  warning where the condition makes the resulting system wrong is a finding;
  the repository's posture is that a check either denies or is not a check.

**Merge priority is judged against ownership, and the repository rule is
narrower than the common advice.** The widely repeated version - use
`mkDefault` for everything a consumer configures and `mkForce` only to override
consumers - is **not** this repository's rule and the prompt must not apply it.
The actual rule:

- **Every priority choice is justified against ownership.** Ask who owns the
  value. If the framework supplies a starting point a consumer is expected to
  change, `mkDefault` is right, because it preserves overrideability. If the
  framework owns an invariant, a bare definition or `mkForce` may be right, and
  which one it is depends on what else defines the same option.
- **`mkForce` is permitted where a framework-owned invariant intentionally
  neutralizes or overrides a competing definition**, and the canonical critical
  example is in this tree: `nixos-modules/net.nix` uses `lib.mkForce` to
  neutralize `base.nix`'s `10-eth-dhcp` on the net VM's uplink, because the net
  VM must not dual-stack DHCP there. `AGENTS.md` lists removing that `mkForce`
  as a security-relevant don't and names
  `tests/unit/nix/cases/net-vm-network.nix` as the check. A `mkForce` of that
  shape is correct and a finding against it is wrong.
- **`mkForce` is never a way to make an unexplained merge conflict go away.**
  The distinguishing question is whether the delta can name the competing
  definition and say why this one wins. If it can, the priority is a decision.
  If it cannot, the priority is a silencer, and the finding is that the
  conflict was never diagnosed.
- **State the merge result, not just the priority.** For a changed list, set or
  attribute-set option, say what the merged value is across the modules that
  define it, and whether the change alters that result for a consumer who
  defines the same option. Priority regressions are silent by construction:
  nothing fails, the value is simply different.

**Anti-patterns and non-goals.** New `extraConfig`-style stringly options;
`mkForce` used to paper over a merge conflict the delta cannot name; **also**
the opposite error, filing a finding against the net VM's `10-eth-dhcp`
neutralizer or any other framework-owned invariant that is correctly forced;
demanding `mkDefault` as a blanket rule, which is not this repository's
posture; deleting or narrowing an assertion instead of fixing its predicate;
`with` over large scopes, which is `software`'s Nix profile and is named here
only so the boundary is visible from both sides; blocking on formatting nixfmt
owns; citing the Nixpkgs
manual's coding conventions chapter, which is now a stub of "this section has
been moved" pointers and contains no conventions at all.

### 3.4 `networking`

**Purpose and scope.** Bridge isolation, firewall posture, DHCP and DNS
behaviour, routing invariants, MTU and MSS handling, address hygiene, and host
network coexistence.

**Primary guidance.**

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| BCP 38 and RFC 2827, ingress filtering | N | T1 | <https://www.rfc-editor.org/info/bcp38> |
| RFC 6890, special-purpose address registries | N | T1 | <https://www.rfc-editor.org/info/rfc6890> |
| RFC 1918, address allocation for private internets | N | T1 | <https://www.rfc-editor.org/info/rfc1918> |
| RFC 5737, IPv4 address blocks reserved for documentation | N | T1 | <https://www.rfc-editor.org/info/rfc5737> |
| RFC 1191, path MTU discovery | N | T1 | <https://www.rfc-editor.org/info/rfc1191> |
| RFC 4821, packetization layer path MTU discovery | N | T1 | <https://www.rfc-editor.org/info/rfc4821> |
| nftables wiki | N | T1 | <https://wiki.nftables.org/> |
| `ip-route(8)` | N | T1 | <https://man7.org/linux/man-pages/man8/ip-route.8.html> |
| `vsock(7)` | N | T1 | <https://man7.org/linux/man-pages/man7/vsock.7.html> |
| `systemd.network(5)` | N M | T1 | <https://www.freedesktop.org/software/systemd/man/latest/systemd.network.html> |
| NIST SP 800-41 Rev. 1, Guidelines on Firewalls and Firewall Policy, 2009 | N | T1 | <https://csrc.nist.gov/pubs/sp/800/41/r1/final> |
| ADR 0005 and ADR 0013, the firewall and coexistence contract | N | T1 | repository-local, and binding |
| ADR 0012, the IPv6-off sysctl ordering and hash-derived interface names | N | T1 | repository-local, and binding |
| `docs/contributing/critical-subsystems.md`, the net VM networking and firewall row and the ownership-marker conventions | N | T1 | repository-local, and binding |
| `docs/reference/inet-d2b-chains.md` and `host-egress-policy.md` | N | T1 | repository-local |

**Premade prompt assets: none exist, and this is a verified enumeration
result.** No reachability-delta, firewall-posture or address and port
allocation review prompt exists in `github/awesome-copilot` at the pin, in
Compound Engineering's agent set, in Superpowers, or in Spec Kit. The
`awesome-copilot` collection has cloud-vendor networking agents, which are
about managed cloud topologies rather than about a reachability delta, and
Compound Engineering's nearest lanes explicitly cede network posture. The only
transferable community pattern is the diff-walk trigger "any new endpoint",
taken as a question rather than as text. This seat is authored from the
normative sources above and this repository's firewall contract.

**Prompt requirements.** State the reachability delta: who can reach what, from
where, before and after. Default-deny posture per environment. Address, context
identifier and port allocation cites a registry or the repository's documented
allocation scheme. DNS and DHCP regressions and bridge isolation invariants
named explicitly. Beyond those, nine repository-critical invariants, each of
which the seat checks by name because each is load-bearing here:

- **The net VM uplink DHCP neutralizer.** Any change under
  `nixos-modules/net.nix` is checked against the `lib.mkForce` neutralization
  of `base.nix`'s `10-eth-dhcp` that `AGENTS.md` names as a security-relevant
  don't, and against `tests/unit/nix/cases/net-vm-network.nix`. Losing it
  dual-stacks DHCP on the uplink and breaks NAT. This is a critical-subsystem
  row, not a preference.
- **MTU and MSS.** Per-environment MTU is part of the same critical-subsystem
  row. Where a path crosses a bridge or a tunnel, state the resulting MTU and
  whether MSS clamping is applied on the forward path. An unclamped MSS behind
  a reduced-MTU link is the classic failure that presents as "large transfers
  hang" and never as a connection error, so it is checked explicitly rather
  than noticed later.
- **Exact interface, state and rule order.** Networking correctness here is
  ordering-sensitive in three places at once: the order sysctls are applied
  relative to interface creation, the order a bridge port is enslaved relative
  to its flags, and the order rules land within a chain. ADR 0012 fixes a
  five-step IPv6-off ordering for exactly this reason. A delta that changes
  any of the three states the new order and why it is equivalent, or it is a
  finding.
- **CIDR overlap and prefix arithmetic.** Any new or changed subnet, prefix
  length or derived address is checked for overlap with every other declared
  environment and with the host's own ranges, and the arithmetic is shown.
  `nixos-modules/assertions.nix` already refuses overlapping CIDRs at eval
  time; a delta that widens a prefix without rechecking that assertion is a
  finding.
- **Address hygiene and generic identity.** Documentation, examples, tests and
  defaults use RFC 1918 ranges for private addressing and RFC 5737 ranges for
  documentation, and generic placeholder names. `AGENTS.md` forbids committing
  real hostnames, real user identifiers and real network ranges, and records
  that the tree has no such leaks today; a delta that introduces one is a
  blocking finding regardless of how harmless the specific value looks.
- **Ownership markers and byte preservation.** Every d2b host mutation is
  delimited so foreign configuration survives byte for byte: nftables rules and
  chains in the `inet d2b` table carry `comment "d2b managed: <ownership-id>"`
  and foreign tables are never flushed; `/etc/hosts` and
  `/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf` are delimited by
  `# d2b-managed begin` and `# d2b-managed end`. A delta that writes outside a
  marker, or that widens what a marker covers, is a finding.
- **A foreign marker is fail-closed, never a signal to overwrite.** Finding a
  foreign marker where d2b expects its own raises `path-safety-violation`,
  `nm-managed-foreign-conflict` or `foreign-nft-rule-preserved`. A delta that
  turns one of those into a warning, a retry or an overwrite is a blocking
  finding.
- **systemd-networkd is detection-only.** d2b never writes systemd-networkd
  configuration. A delta that adds a write path there is a finding against the
  coexistence contract even if it works.
- **nftables ownership comments and table scope.** New rules land in the named
  `inet d2b` table with the ownership comment, in the documented four-chain
  layout, and never in `raw`, `mangle` or `nat`. Cite
  `docs/reference/inet-d2b-chains.md` for the layout and ADR 0013 for the
  detector-to-policy matrix.

**Anti-patterns and non-goals.** Hardcoded addresses outside documented ranges;
temporarily-open rules with no expiry or owner; conflating a host-only
transport with an authenticated channel; treating denial of service as out of
scope because a security prompt from another project excluded it; proposing a
second repair owner for a host network path, which `AGENTS.md` forbids;
reviewing the syscall semantics of a socket call, which is `kernel`'s.

### 3.5 `kernel`

**Purpose and scope.** pidfd, cgroup v2, namespace, mount, signal, ioctl and
filesystem semantics, descriptor inheritance and file-locking semantics, kernel
version assumptions, and Linux API edge cases.

**Primary guidance.**

| Source | Class | Tier | URL |
| --- | --- | --- | --- |
| `pidfd_open(2)` | N | T1 | <https://man7.org/linux/man-pages/man2/pidfd_open.2.html> |
| Kernel documentation, Control Group v2 | N | T1 | <https://docs.kernel.org/admin-guide/cgroup-v2.html> |
| `namespaces(7)` | N | T1 | <https://man7.org/linux/man-pages/man7/namespaces.7.html> |
| `mount_namespaces(7)` | N | T1 | <https://man7.org/linux/man-pages/man7/mount_namespaces.7.html> |
| `mount(2)` | N | T1 | <https://man7.org/linux/man-pages/man2/mount.2.html> |
| `open(2)`, including `O_CLOEXEC` and descriptor inheritance | N | T1 | <https://man7.org/linux/man-pages/man2/open.2.html> |
| `fcntl(2)`, open file description locks and `FD_CLOEXEC` | N | T1 | <https://man7.org/linux/man-pages/man2/fcntl.2.html> |
| `openat2(2)`, resolve flags | N | T1 | <https://man7.org/linux/man-pages/man2/openat2.2.html> |
| `seccomp(2)` | N | T1 | <https://man7.org/linux/man-pages/man2/seccomp.2.html> |
| Kernel documentation, seccomp filter | N | T1 | <https://docs.kernel.org/userspace-api/seccomp_filter.html> |
| `signal(7)` | N | T1 | <https://man7.org/linux/man-pages/man7/signal.7.html> |
| `sigaction(2)` | N | T1 | <https://man7.org/linux/man-pages/man2/sigaction.2.html> |
| `signal-safety(7)`, async-signal-safe function list | N | T1 | <https://man7.org/linux/man-pages/man7/signal-safety.7.html> |
| `errno(3)`, for the filesystem and interruption error set | N | T1 | <https://man7.org/linux/man-pages/man3/errno.3.html> |
| `rename(2)`, for its error semantics; the durability ordering stays `reliability`'s | N | T1 | <https://man7.org/linux/man-pages/man2/rename.2.html> |
| Kernel ABI stability documentation | N | T1 | <https://docs.kernel.org/admin-guide/abi.html> |
| ADR 0008 and ADR 0011, the platform floor and the cgroup and pidfd contract | N | T1 | repository-local, and binding |
| ADR 0034's `O_CLOEXEC` and OFD-lock requirement, restated in `AGENTS.md` | N | T1 | repository-local, and binding |

**Premade prompt assets: none exist, and this is a verified enumeration
result.** No kernel, syscall, cgroup, namespace or seccomp review asset exists
in `github/awesome-copilot` at the pin, in Compound Engineering, in
Superpowers, or in Spec Kit. The closest match anywhere is an embedded C
authoring agent, which is authoring guidance for a different domain and citing
it here would trip this seat's own anti-pattern about citing the wrong kind of
source for kernel semantics. This seat is authored from man-pages and
kernel.org documentation, which is the correct outcome rather than a shortfall.

**Prompt requirements.** Every syscall or kernel-interface assumption cites a
man page or a kernel.org page and states the minimum kernel version, checked
against the repository's declared floor. Race classes named explicitly: pid
reuse, descriptor inheritance, mount propagation, and time-of-check to
time-of-use on path resolution. cgroup v1 versus v2 assumptions stated. Where
the delta resolves a path under a privileged identity, check the resolve-flag
set against the anchored-resolution rule this repository already applies, and
treat a dropped flag as a finding rather than a style difference.

Four territories moved into this seat with the 2026-08-04 revision, because
each is a question about what the syscall does rather than about who owns the
resource. `reliability` still owns the cross-component lifecycle those
primitives serve; the split is restated in section 1.2 and in 3.1.

- **Descriptor inheritance.** Every descriptor the delta creates is checked for
  `O_CLOEXEC` at creation, not for `FD_CLOEXEC` set afterwards: setting
  close-on-exec after creation is not atomic and a `fork` and `exec` in the
  window leaks the descriptor into a child. ADR 0034 requires `O_CLOEXEC` by
  name and `AGENTS.md` restates it. A delta that opens without it, or that
  relies on a later `fcntl`, is a finding, and the finding names the exec path
  that would inherit.
- **Locks.** Open file description locks and POSIX record locks are not
  interchangeable and the difference bites in one specific way: a POSIX record
  lock is dropped when **any** descriptor to that file is closed by the
  process, including one an unrelated code path opened, while an OFD lock is
  tied to the open file description. ADR 0034 requires OFD locks. A delta that
  reaches for `flock` or `F_SETLK` where an `F_OFD_SETLK` is required, or that
  assumes lock ownership survives a descriptor it does not control, is a
  finding.
- **Signals.** Signal disposition, the async-signal-safety of anything reached
  from a handler, `SA_RESTART` versus explicit `EINTR` retry, `SIGCHLD` and
  reaping, and `SIGPIPE` on a write to a closed peer. Two rules the prompt
  states outright: only async-signal-safe functions are callable from a
  handler, per `signal-safety(7)`, and a blocking call that can return `EINTR`
  either has `SA_RESTART` or has an explicit retry loop, never neither.
- **Filesystem and interruption error cases.** Every filesystem call in the
  delta is checked against the errors it can actually return, with four named
  because they are the ones handled wrongly most often here: **`EXDEV`**, which
  makes a `rename` across a filesystem boundary fail rather than fall back, and
  which matters because the hardlink farm requires `/var/lib/d2b` and
  `/nix/store` on the same filesystem; **`EINTR`**, per the signals rule above;
  **`EAGAIN`**, which is not an error on a non-blocking descriptor and is
  frequently treated as one; and **`ENOSPC`**, which must leave the on-disk
  state readable rather than half-written. A delta that maps any of the four
  into a generic error without saying so is a finding.

**Anti-patterns and non-goals.** Citing distribution blog posts or
administration guides for kernel semantics; asserting behaviour with no version
floor; reviewing userspace design as if it were kernel code; taking
cross-component resource lifetime, which is `reliability`'s, or the durability
ordering of a write, which is also `reliability`'s even where this seat owns
the errno set of the same call.

## 4. Honest coverage statement

The claim this document makes, precisely: **complete prompt-construction
coverage of all eleven Gas City `build-base` stages, all three `fix-loop-base`
steps, and the clarification seam; source-backed coverage of seven of the
twelve seats, six of them fully and `product` partly; and five seats plus one
language profile carried by normative specifications and local code because no
suitable premade review prompt exists for them.**

| Seat | Premade review prompt available? |
| --- | --- |
| `software` | Yes for the shared part: correctness structure from Compound Engineering, changed-lines discipline from awesome-copilot, convention-citation rule from Compound Engineering. **Its Rust profile is the exception** and is the source gap recorded in the profile table below. |
| `test` | Yes, and the strongest set of any seat: an evidence table, a red-green protocol, and a false-confidence taxonomy. |
| `product` | Partly. Spec Kit's clarification taxonomy and read-only analysis contract are strong; the authoring-aid product agents are not review rubrics and are not used. |
| `docs` | Yes, via the Compound Engineering coherence reviewer and Spec Kit's replace-do-not-append rule. |
| `security` | Yes, and it is first-party and MIT: the Anthropic security review prompt. |
| `observability` | **No.** Only vendor-specific assets exist. Authored from OpenTelemetry and Prometheus sources. |
| `simplicity` | Yes, two independent precedents, both with numbers that must be refused. |
| `reliability` | **No suitable one.** The nearest upstream lane is scoped to single-boundary error handling and would recreate the `software` overlap. Authored from man-pages, SRE material and local ADRs. |
| `agentic` | Yes, and unusually well covered. |
| `nixos` | **No.** Verified absent from all five surveyed collections. Authored from the NixOS module system reference, Nixpkgs conventions and local `nixos-modules/`. |
| `networking` | **No.** Verified absent from all five surveyed collections. Authored from RFCs, nftables, systemd and NIST sources plus the local firewall contract. |
| `kernel` | **No.** Verified absent from all five surveyed collections. Authored from man-pages and kernel.org. |

**Profile-level coverage inside `software`**, stated separately because D21
merged the Rust seat into this one and a merged seat can hide a gap that a
missing seat cannot:

| `software` profile | Premade review prompt available? |
| --- | --- |
| `rust` | **No, for the depth this profile carries.** What exists in `github/awesome-copilot` at the pin is model-context-protocol server **generation** guidance and a general repository-wide Rust instructions file that is advisory community content with no unsafe, FFI or SemVer depth. Compound Engineering has no Rust reviewer; its only language-specific reviewer is for Swift. Superpowers and Spec Kit are language-agnostic. Authored from the Rust Reference, the API Guidelines, the Cargo SemVer reference, the Rustonomicon and the measured local lint perimeter in 2.1a. |
| `python` | **No.** No Python review rubric was found in the five collections; what exists is authoring guidance. Authored from PEP 8, the language and library references, the typing documentation and section 6.4. |
| `shell` | **Partly**, and not as a prompt. ShellCheck's per-code wiki is the closest thing to a review rubric that exists for shell and is used as one, together with the Google guide as Bash-only mature-project practice. No agent asset transfers. |
| `nix` | **No.** Same verified absence as the `nixos` seat: no Nix asset of any kind in the surveyed collections. Authored from the Nix reference, nix.dev, Nixpkgs conventions and RFC 166; RFC 42 belongs to the `nixos` seat, not to this profile. |

**This is the honest form of what merging the Rust seat cost.** The depth gap
did not close; it moved from a seat with its own file, where an absent prompt
is conspicuous, into one profile of the largest prompt in the pool, where it is
not. Naming it in its own table is the whole mitigation, and it is a
documentation control rather than a mechanical one.

The five collections enumerated to reach those "no" results, at the pins in
section 2: `github/awesome-copilot` `agents/` and `skills/`, the Compound
Engineering agent set vendored into `gascity-packs`, the Superpowers skill set
vendored into `gascity-packs`, `github/spec-kit` `templates/` and
`templates/commands/`, and the Gas City `build-base` and `fix-loop-base` stage
assets. These are absences established by directory-level enumeration, not by a
search that came up empty.

## 5. Prompt construction across the Gas City rig stages and seams

ADR 0053 opts into a Gas City orchestration that extends `build-base` and
`fix-loop-base` rather than reimplementing them. This section is the
prompt-construction contract for every stage that orchestration actually runs,
so that a d2b-authored stage prompt is written against the upstream step it
overrides rather than against a remembered workflow.

### 5.1 The authoritative stage and seam list

Read at the pin from `GCP gascity/formulas/build-base.formula.toml`. These are
the declared steps in order, with their run target, artifact schema and whether
a `check` gates them.

| # | Step id | Run target | Artifact schema | Gated |
| --- | --- | --- | --- | --- |
| 1 | `prepare` | `gc.run-operator` | none | no |
| 2 | `requirements` | `gc.requirements-planner` | `gc.build.requirements.v1` | yes |
| 3 | `plan` | `gc.design-author` | `gc.build.plan.v1` | yes |
| 4 | `plan-review` | `gc.review-synthesizer` | none | **no** |
| 5 | `decompose` | `gc.task-decomposer` | `gc.build.decomposition.v1` | yes |
| 6 | `implement` | the implementation target | none | no |
| 6b | `implement-same-session` | the implementation target | none | no |
| 7 | `summarize-implementation` | `gc.run-operator` | `gc.build.implementation-summary.v1` | yes |
| 8 | `review` | `gc.implementation-reviewer` | `gc.build.review.v1` | yes |
| 9 | `finalize` | `gc.run-operator` | `gc.build.final-report.v1` | yes |
| 10 | `publish` | `gc.publisher` | none | no |

`fix-loop-base` adds three steps: `plan-fixes` to the review synthesizer,
`apply-fixes` to the implementation target, and `re-review` to the
implementation reviewer, the last of which is gated.

Three statements about this list are load-bearing and easy to get wrong:

- **There is no `clarify` stage in `build-base`.** Clarification is a seam, not
  a stage: it is a planning input this repository already owns through the
  `speckit-clarify` skill, and Spec Kit's `/clarify` is the model for it.
  Section 5.4 covers it as a seam. A prompt or record that names an upstream
  clarification stage is describing something the upstream contract does not
  have.
- **The fix loop is a seam, not a `build-base` stage.** It hangs off the
  `review_fix_formula` variable, whose default is `fix-loop-base`, bounded by
  an iteration variable. Its three steps are real and are covered below, but
  calling the fix loop an upstream build-base stage is wrong.
- **`prepare`, `plan-review` and `publish` have no artifact schema and no
  check.** Those are the three places where a prompt is the only control, which
  is precisely where the `agentic` seat's replace-prompt-with-gate rule bites
  hardest, and where a d2b override must supply its own stop condition.

**Licence posture for this whole section.** The stage prompt assets at
`GCP gascity/assets/workflows/build-base/<stage>.md` and
`GCP gascity/assets/workflows/fix-loop-base/<step>.md` sit in a repository with
**no LICENSE at the pinned commit**. They may be read and cited as
**behavioural evidence** of what upstream declares - which step exists, what it
dispatches, what artifact schema it produces, whether a check gates it - and
that is exactly how the table above uses them. Their **text, their
organisation and their expressive structure may not be copied or adapted**. An
earlier revision of this document said their structure could inform a d2b
prompt, which contradicted this document's own T4 rule in section 0.2; that
permission is withdrawn. A d2b stage prompt is derived from the permissively
licensed sources named per stage below - Superpowers and Compound Engineering
as vendored MIT with provenance in `upstream.toml`, Spec Kit MIT,
`awesome-copilot` MIT - from T1 normative documentation and standards, and from
this repository's own requirements. The upstream anchor tells the author which
step is being overridden; it does not supply the shape.

### 5.2 Stage prompt contracts

Each entry gives the upstream anchor, the best public prompt sources for that
stage, the behaviours a d2b override must require, and the upstream mechanics
and thresholds that must not be copied.

#### `prepare`, intake and context

- **Upstream anchor.** `build-base` step 1, `gc.run-operator`, asset
  `prepare.md`. No artifact schema, no check.
- **Best public sources.** Superpowers `using-git-worktrees` (MIT) for
  isolation detection before creation, including its false-positive guard that
  a submodule also looks like a separate work tree, and its "never fight the
  harness" rule that using a raw fallback where the platform has a native
  mechanism creates state the harness cannot see. Anthropic's Claude Code best
  practices for the context-budget framing: performance degrades as the context
  fills, so what is loaded is a decision.
- **Required behaviours.** Claim the work item before any work begins, so
  monitoring does not read a false stall while work is in flight. Capture the
  inputs and the report path. **State what was loaded and what was deliberately
  not loaded**, which is the only part of context management that survives into
  a reviewable artifact. Validate declared mode inputs against the methodology
  vocabulary before any stage runs, and treat an out-of-vocabulary value as a
  **blocked** outcome rather than a best-effort run - the same fail-closed
  posture D21 gives an unrecognised rule identifier. Detect existing isolation
  before creating any, and obtain consent before mutating a contributor's
  workspace.
- **Must not copy.** The bead command text and its metadata-flag guidance,
  which is roughly the majority of the upstream asset's length and is CLI
  documentation leaking into a prompt. The metadata key names. Superpowers'
  hard-coded worktree paths and platform-specific tool names.

#### `requirements`, specification

- **Upstream anchor.** `build-base` step 2, `gc.requirements-planner`, artifact
  schema `gc.build.requirements.v1`, gated by an artifact validity check.
- **Best public sources.** Spec Kit `SK templates/commands/specify.md` (MIT)
  for requirement shape. Superpowers `brainstorming` (MIT) for the hard gate
  that no implementation action happens until a design is presented and
  approved, "regardless of perceived simplicity", and for its written-spec
  self-review with four named checks: placeholder scan, internal consistency,
  scope check, and ambiguity check phrased operationally as "could any
  requirement be read two ways; if so pick one and make it explicit".
- **Required behaviours.** Every requirement is testable and carries a
  definition-of-done indicator. Unquantified adjectives are findings.
  **Never ask a question in headless mode**; record unresolved ambiguity inside
  the artifact instead. On a repair attempt, **repair the artifact in place
  rather than regenerating it**, which is the upstream rule and is the same
  discipline D21 asks of a seat amending its own prior record. Close the step
  only after the artifact path is recorded.
- **Must not copy.** The schema identifier and the validator script path. The
  retry budget. Superpowers' one-question-at-a-time interactive loop and its
  approval-after-each-section gate, both of which presume a human in the loop
  that a headless run does not have.

#### `plan`, planning

- **Upstream anchor.** `build-base` step 3, `gc.design-author`, schema
  `gc.build.plan.v1`, gated.
- **Best public sources.** Superpowers `writing-plans` (MIT), which is the
  strongest planning asset surveyed: write for an engineer with zero context
  and questionable taste; exact file paths always; complete code in every step
  that changes code; exact commands with expected output. Its **No
  Placeholders** list is a set of plan failures rather than suggestions: "TBD",
  "add appropriate error handling", "write tests for the above" with no test
  code, "similar to task N", steps that describe what without showing how, and
  references to types or functions no task defines. Spec Kit
  `SK templates/commands/plan.md` (MIT) for the research consolidation format
  of decision, rationale and alternatives considered.
- **Required behaviours.** Decompose the file structure **before** defining
  tasks, on the stated rationale that you reason best about code you can hold
  in context at once. Carry the placeholder ban list explicitly. State the
  ownership of each file so parallel scopes stay disjoint, which is d2b's own
  plan requirement and maps onto the upstream file-structure step.
- **Must not copy.** Upstream artifact filenames. The schema identifier.
  Superpowers' product-specific plan directory paths.

#### `plan-review`

- **Upstream anchor.** `build-base` step 4, `gc.review-synthesizer`, **no
  artifact schema and no check**. The only upstream stop condition is the
  model's own judgement.
- **Best public sources.** Superpowers `writing-plans` self-review and
  `executing-plans` step 1 (MIT), which requires concerns to be raised before
  starting rather than discovered during. Compound Engineering
  `ce-plan-review-synthesizer` (MIT via vendored subtree) for the four
  synthesis verbs: deduplicate, separate required changes from residual risk,
  suppress non-actionable noise, emit one signal.
- **Required behaviours.** Three mechanical passes, each with a stated pass
  criterion because this stage has no artifact check and **the prompt is the
  gate**: spec coverage, walking each requirement and naming the task that
  implements it or recording the gap; a placeholder scan against the ban list;
  and type and identifier consistency across tasks, where a function named one
  way in an early task and another way in a later one is a defect rather than a
  typo. A d2b override supplies the stop condition upstream does not.
- **Must not copy.** The ungated shape itself. An ungated review stage whose
  only stop condition is a model's judgement is the one upstream structure this
  repository should not reproduce.

#### `decompose`, the task graph

- **Upstream anchor.** `build-base` step 5, `gc.task-decomposer`, schema
  `gc.build.decomposition.v1`, gated.
- **Best public sources.** Superpowers `writing-plans` bite-sized granularity
  (MIT): each step is one action of a few minutes, and the canonical shape is
  write failing test, run to confirm it fails, minimal implementation, run to
  confirm pass, commit. Its file-structure rules: units with clear boundaries
  and one responsibility per file; files that change together live together;
  split by responsibility, not by technical layer; and in an existing codebase,
  follow established patterns rather than unilaterally restructuring. Spec Kit
  `SK templates/commands/tasks.md` (MIT) for phase ordering and the requirement
  that each phase declare independent test criteria.
- **Required behaviours.** Emit a graph whose nodes are file-disjoint where
  they are meant to run in parallel, which is d2b's wave rule. Each unit
  carries an independently checkable completion condition; a stopping condition
  a machine cannot evaluate is not a stopping condition. Where scopes cannot be
  made file-disjoint, precede them with a prep commit landing the shared
  contract, so each scope opens against a stable base.
- **Must not copy.** The schema identifier, the convoy vocabulary, and the
  drain policy names, all of which are Gas City runtime contracts.

#### `implement` and `implement-same-session`

- **Upstream anchor.** `build-base` steps 6 and 6b, both dispatching the
  implementation target, neither gated, with the drain policy selecting between
  separate sessions and one shared session and the shared variant carrying a
  skip-remaining failure policy.
- **Best public sources.** Superpowers `executing-plans`,
  `subagent-driven-development` and `test-driven-development` (MIT). Spec Kit
  `SK templates/commands/implement.md` (MIT) for a pre-implementation checklist
  gate that computes a pass or fail table and halts for confirmation on fail.
- **Required behaviours.** State the drain policy in effect and its failure
  semantics, because they differ: a shared-session drain that stops at the
  first failure leaves later items untouched, and a prompt that does not say so
  produces a partially applied plan that reads as complete. A fresh subagent
  per task where the policy is separate. Where tests are the mechanism, state
  the verify-red conditions: the test fails rather than errors, the failure
  message is the expected one, and it fails because the feature is missing.
- **Must not copy.** The convoy and member-access vocabulary. The
  language-specific test commands in the upstream skills. Superpowers'
  second-person register and its emotive framing, which conflict with this
  repository's imperative documentation voice.

#### `summarize-implementation`, the handoff

- **Upstream anchor.** `build-base` step 7, `gc.run-operator`, schema
  `gc.build.implementation-summary.v1`, gated.
- **Best public sources.** Superpowers `requesting-code-review` (MIT), which
  states the single most transferable handoff rule in the survey: the reviewer
  gets **precisely constructed context, never the session's history**, which
  keeps the reviewer on the work product rather than on the author's thought
  process. Its required inputs pin a base and a head commit rather than a
  branch name.
- **Required behaviours.** Pin base and head commit identifiers, never a branch
  name, which is exactly d2b's own snapshot discipline. Construct the reviewer
  bundle deliberately: the diff ranges, the validation evidence the integrator
  already ran, and the acceptance items, and nothing about how the work felt.
  **Coverage identifiers round-trip**: every item identifier listed upstream
  appears exactly once in the coverage trace and once in the human-readable
  coverage table with the same status, and the coverage status vocabulary is
  distinct from the verdict vocabulary so the two cannot be confused.
- **Must not copy.** The schema identifier, the metadata key names, and the
  upstream status tokens.

#### `review`

- **Upstream anchor.** `build-base` step 8, `gc.implementation-reviewer`,
  schema `gc.build.review.v1`, gated. This is the step ADR 0053's panel
  overrides.
- **Best public sources.** The Anthropic security review prompt (MIT) for
  delta-scoping and the misses-over-noise posture. Compound Engineering's
  reviewer set (MIT via vendored subtree) for per-lane hunt classes. Google
  engineering practices for the approval standard. The review categories a d2b
  override uses are **not** taken from upstream's scorecard, which sits in an
  unlicensed tree: they are the ownership map of section 1.2, which D21 decided
  and which is the only category list that can match this panel's seats. That
  upstream scores a review across named categories is a fact worth knowing
  about the system this repository integrates with; its category list, wording
  and ordering are not available to adapt, and an earlier revision of this
  document reproduced them here.
- **Required behaviours.** Review authority is an **explicit input, not a seat
  choice**, and d2b's panel is report-shaped: findings only, no code mutation,
  no fixes applied from this stage. Say so in every seat prompt, because
  roughly half of the surveyed community assets are implementers wearing
  reviewer names. Findings tie to concrete files, commands or artifact paths.
  Walk the diff once per category rather than reading whole files. Where a
  category has no findings, say so rather than omitting the heading, so an
  unexamined category is distinguishable from a clean one. **Gap analysis is a
  review lens inside this stage, not a separate lifecycle stage**, which is
  upstream's own framing and matches D21's decision to give it to `product`
  rather than to a new seat.
- **Must not copy.** Multi-band severity ladders and their block,
  request-changes and approve decision matrices: d2b has exactly one blocking
  channel and a four-band verdict has nowhere to land. Import the **mechanism**
  instead, a stated rule for which severities may enter `recommendations` at
  all, which the byte-identical finding bar already owns. Also: the
  report-file-writing instructions embedded in several community reviewer
  assets, the JSON envelopes with residual-risk and testing-gap side channels
  that d2b deliberately refused, and every confidence anchor and depth band.

#### `finalize`, fresh evidence

- **Upstream anchor.** `build-base` step 9, `gc.run-operator`, schema
  `gc.build.final-report.v1`, gated.
- **Best public sources.** Superpowers `verification-before-completion` (MIT):
  a five-step gate function - identify the command that proves the claim, run
  it fresh and complete, read the full output and exit code and failure count,
  verify the output confirms the claim, only then make the claim - with the
  rule that skipping a step is lying rather than verifying, and a claim to
  evidence table with an explicit not-sufficient column. Anthropic's Claude
  Code best practices for the same argument in vendor form: have the agent show
  evidence rather than assert success.
- **Required behaviours.** No completion claim without fresh verification
  evidence. Name the command, the output and the exit status. Red flags that
  are themselves findings: hedged completion language, and satisfaction
  expressed before the evidence. **The panel inverts the actor**: reviewers do
  not re-run validation, they audit the evidence the integrator supplied
  against the same table, and missing or insufficient evidence is a finding.
  This inversion is deliberate and must be stated in the prompt, because the
  upstream skill says the opposite and a seat that follows it will stampede the
  shared build cache.
- **Must not copy.** The schema identifier, the upstream report filenames, and
  the upstream language-specific verification commands.

#### `publish`, default-off publication

- **Upstream anchor.** `build-base` step 10, `gc.publisher`, no artifact
  schema, no check, with both the push and open-pull-request variables
  defaulting to false.
- **Best public sources.** Superpowers `finishing-a-development-branch` (MIT):
  verify tests before presenting options and stop if failing; detect the
  environment before offering actions; provenance-based cleanup, removing only
  what you created, because removing a harness-owned work tree creates phantom
  state; a strict ordering of merge, then remove work tree, then delete branch;
  and a typed confirmation for a destructive option.
- **Required behaviours.** **Publication is default-off upstream and a prompt
  that assumes a push is a contract violation.** Record an explicit no-op
  reason rather than silence when publication does not happen, which is
  upstream's own behaviour and generalises to d2b's rule that a `relevant:
  false` pass carries a stated reason. In d2b's overriding design, publication
  is required in v1 but the authority is not the prompt's: the publisher
  verifies the manifest independently, receives the body as bounded bytes
  rather than a substitutable path, and merge stays human-only. Say that in the
  prompt so no stage prompt believes it can merge.
- **Must not copy.** The upstream publish status and reason token spellings,
  and the upstream branch and worktree option menus, which are its user
  experience contract rather than a rule.

#### `plan-fixes` and `apply-fixes`, the fix seam

- **Upstream anchor.** `fix-loop-base` steps 1 and 2, dispatching the review
  synthesizer and the implementation target. **Both upstream prompt assets are
  placeholders**, a few hundred bytes each, whose content is that a concrete
  pack overrides them. They carry **no substantive review or fix guidance**,
  and this document says so rather than citing them as sources they are not.
- **Best public sources.** Superpowers `receiving-code-review` (MIT), which is
  where the real guidance for this seam lives: a six-step response pattern of
  read, understand, verify, evaluate, respond, implement one at a time; an
  implementation order of clarify first, then blocking issues, then simple
  fixes, then complex fixes, testing each individually; a **forbidden
  responses** list banning agreement filler and gratitude; a rule that an
  unclear item blocks **all** implementation rather than only itself, because
  partial understanding produces the wrong fix; a check for actual usage before
  implementing a reviewer's suggestion; and explicit permission and method to
  push back with technical reasoning and working tests.
- **Required behaviours.** Fix rounds address **only** the findings raised,
  which is this repository's existing rule: a genuine defect found while fixing
  something else is filed separately, because unrequested changes are new
  content and new content invalidates the round's evidence. The loop is bounded
  and the bound is stated. Fixes are applied by re-entering the development
  loop, not by extending the review stage. Every held reviewer's prior
  recommendation gets an explicit resolved or not-resolved judgement, per
  section 1.4.
- **Must not copy.** Any claim that the upstream placeholders contain
  guidance. The upstream iteration budget, which is a Gas City retry number and
  not d2b's round budget. The upstream metadata keys and failure-class tokens.

#### `re-review`

- **Upstream anchor.** `fix-loop-base` step 3, `gc.implementation-reviewer`,
  gated. Unlike its two siblings this asset has real content: an iteration
  bound and an artifact-repair rule.
- **Best public sources.** The same review-stage sources, plus the upstream
  repair-in-place rule shared across six `build-base` stages.
- **Required behaviours.** A later round reviews the delta since that reviewer
  last reviewed **plus** the full branch for context, which is d2b's existing
  two-range rule. Amend the prior record rather than regenerating it. Any
  content change invalidates every prior sign-off in the phase, while roster
  membership and seat identity persist: D21 reconciles those two rules and the
  prompt must not treat them as in tension.
- **Must not copy.** The iteration bound. The artifact schema identifier.

### 5.3 The clarification seam

Clarification is not an upstream `build-base` stage. It is a **planning seam
and a workflow input**: this repository already carries a `speckit-clarify`
skill, and Spec Kit's `SK templates/commands/clarify.md` (MIT) is the model.
Mapping it as a stage would misdescribe the upstream contract; mapping it as a
seam describes what actually happens, which is that clarification output feeds
`requirements` and `plan`.

- **Best transferable mechanics.** The ten-category ambiguity taxonomy with a
  Clear, Partial or Missing status per category, reproduced in section 2.3. A
  materiality filter: only ask what materially changes architecture, data
  modelling, task decomposition, test design, operational readiness or
  validation, and skip when the answer would change neither implementation nor
  validation strategy. A question-quality rule that transfers directly to
  finding quality: **never use a topic label, a section heading or a
  requirement identifier as the question itself**, and always give a
  one-sentence statement of what is at stake. A write-back discipline that
  replaces an invalidated statement rather than appending a correction beside
  it. And an honest exit: recording that no critical ambiguity was found is a
  valid outcome.
- **Reworded for a d2b panel record.** A `recommendation` is a self-contained
  imperative that a reader who does not know this workflow can act on, not a
  topic label and not an identifier. It states what is at stake in one
  sentence. This is the same rule the finding bar already implies, made
  operational.
- **Must not copy.** The question quota, the answer-length limit, the option
  count, the impact-times-uncertainty prioritisation heuristic, the extension
  hook machinery, the argument placeholder tokens, the constitution path, and
  the requirement identifier prefixes. Also the interactive one-question-at-a-
  time loop, which a headless panel cannot run.

### 5.4 Cross-cutting behaviours every stage prompt carries

Six rules appear in enough upstream stages to be treated as the shared contract
rather than as per-stage advice:

1. **Close only after the artifact exists at its declared path.** Six upstream
   stages end with that sentence. The d2b form is that a seat closes only after
   its verdict record exists where the request said it would.
2. **Repair in place on a retry, do not regenerate.** Same six stages. The d2b
   form is that a seat re-running in a later round amends its prior record
   rather than producing an unrelated new one.
3. **Never ask a question in headless mode.** Record unresolved ambiguity
   inside the artifact. d2b panel seats are non-interactive by construction, so
   every interactive construct in an adapted asset must be removed rather than
   softened.
4. **Fail closed on an out-of-vocabulary input.** Upstream validates declared
   mode inputs before any stage runs and treats an unknown value as blocked.
   D21 applies the identical rule to an unrecognised rule identifier, an
   unimplemented table version, an over-large change surface, and an
   undecidable surface class.
5. **Record a no-op with a reason.** Upstream's publish step records why it did
   nothing rather than being silent. d2b's `relevant: false` carries a one-line
   reason in the summary for the same purpose.
6. **Work only in the assigned lane.** Both Compound Engineering synthesis
   prompts end by forbidding the reviewer from invoking provider-native
   subagents, slash commands or the upstream plugin runtime. The d2b form is
   the ownership map plus the rule that a seat does not edit the diff.

### 5.5 The must-not-copy register

Collected in one place so that a prompt author can check a draft against it.
Nothing in this list may appear in a d2b seat or stage prompt.

**Gas City runtime contracts.** Bead command text and its metadata-flag
guidance; outcome, failure-class, attempt, attempt-log, blocked-reason,
convoy-id, work-dir and step-ref metadata keys; the six build artifact schema
identifiers; the validator script path; the convoy, drain, member-access,
single-lane and item-failure vocabulary; and the upstream artifact filenames.

**Gas City stage prompt text, organisation and expressive structure.** The
assets under `GCP gascity/assets/workflows/` and the formulas under
`GCP gascity/formulas/` and `GCP pr-pipeline/formulas/` carry no licence grant
at the pinned commit. Nothing from them is copied and nothing is adapted: not
the prose, not the section decomposition, not the heading sequence, not the
checklist taxonomy, and not a scorecard category list. They are read to
establish what upstream declares, and cited for that. Where a structure is
genuinely needed, it comes from a permissively licensed source or from this
repository's own requirements, both of which section 5.2 names per stage.

**Gas City and Compound Engineering numbers.** The stage retry budget, the two
bounded repair attempts, the fix-loop iteration default, the step timeout; the
confidence anchor ladder and its four labels; the file-size finding thresholds
and the delegation-hop limit; the adversarial depth bands and their changed-line
triggers; the scope complexity smells expressed as file and abstraction counts,
the priority-distribution percentage, and the unsourced cost-gap multiplier.

**Compound Engineering routing vocabulary.** The priority tier tokens, the
autofix classification, the owner and resolver tokens, the pre-existing and
soft-bucket routing concepts, and the JSON envelope's residual-risk and
testing-gap arrays, which are precisely the non-blocking side channels d2b
deliberately refused. Also its lane and persona names, including the
language-specific and framework-specific ones, which have no analogue here.

**Spec Kit machinery.** The extension-hook block, the argument and script
placeholder tokens, the frontmatter script and handoff keys, the state
directory paths, the constitution path, the command identifiers, the four-band
severity ladder, the finding-count cap, the question quota and answer-length
limit, and the requirement and finding identifier prefixes.

**awesome-copilot frontmatter and plumbing.** Model pins, tool lists, mode and
visibility keys, hardcoded report output paths, the runtime envelope and score
fields of the generated-agent family, and the duplication line-count threshold.

**Superpowers voice and paths.** The second-person register and emotive
framing, the product-specific document and worktree paths, the skill-reference
prefix, the platform-specific tool names, the exact option menus and typed
confirmation tokens, and the language-specific test commands.

**The Anthropic security prompt's exclusions, bands and taxonomy.** Its
exclusion list is correct for its own pipeline and wrong here, as section 2.5
explains. Its confidence bar, confidence bands, severity ladder, JSON field
names and web-application vulnerability category tokens are equally
non-transferable.

**Everything emoji and every non-ASCII dash.** Nearly every surveyed community
asset uses them; this repository's gate fails closed on the dash class and the
scan covers tracked and untracked files alike. Extract structure and retype the
text. One community asset's own rule is worth keeping even though its prose
violates it: strict ASCII-only output.

## 6. Local naming and layout conventions

The `software` seat is required to check identifier naming **and** file and
directory naming, and is required to follow repository-local convention first.
That instruction is only usable if the local conventions are written down, and
most of them are not: `docs/reference/naming-conventions.md` is a glossary of
**runtime** identifiers - service and user names, broker caller-role audit
labels, realm, workload, VM and environment identifiers, session names, network
device names - and covers **no** source identifier, source file name, or
directory name in any language. A seat that cites it for a source-naming
finding is citing a page that does not contain the rule.

This section fills that gap, and it distinguishes two things that are easy to
conflate.

- An **explicit gate** is a rule a committed check enforces. Violating it
  breaks the build, and a finding may cite the gate.
- **Observed canon** is a pattern the tree follows consistently with no check
  behind it. `AGENTS.md` makes it binding anyway, because existing code is
  canon, but a finding must cite it as observed practice and name what it
  observed, not claim a gate that does not exist.

### 6.1 Explicit gates that touch naming and layout

Measured 2026-08-04 from committed code:

| Gate | What it enforces | Where |
| --- | --- | --- |
| ASCII hyphen only | No non-ASCII dash codepoint anywhere in tracked or non-ignored untracked files, including file content of every type | `scan_dashes` in `tests/tools/tier0-first-pass.sh`, via `make check-tier0` |
| Process markers out of shipped artifacts | Wave, phase, revision, follow-up and finding tags absent from governed paths, against a frozen allowlist; the script is authoritative for governed paths and exceptions | `scan_process_markers` in the same script |
| ADR index coverage | Set equality between `docs/adr/0NNN-*.md` at depth one and the entries linked from `docs/adr/README.md`. Files under `docs/adr/specs/` are **not** indexed and must not be added to the index | `tests/unit/meta/adr-index-coverage.sh` |
| Panel seat file set | Bidirectional set equality between the roster constant and `.github/agents/panel-<role>.agent.md`, with the filename derived by kebab-casing the enum variant, plus byte-identical finding-bar blocks | `scripts/copilot/check-bindings.mjs` |
| Generated artifact drift | A fixed path list must match regenerated output | `tests/unit/gates/drift-check.sh` |

Everything else below is observed canon.

### 6.2 Observed canon in this tree

Measured 2026-08-04 by enumeration, not by sampling:

| Surface | Observed convention | Evidence |
| --- | --- | --- |
| Rust crate directories | `kebab-case`, almost all prefixed `d2b-`, with `d2bd` and `xtask` as the two deliberate exceptions | `packages/` |
| Rust module files | `snake_case.rs`, including nested module directories | `packages/*/src/**` |
| Nix module files | `kebab-case.nix`, grouped by prefix, with `default.nix` as the aggregator | `nixos-modules/` |
| Shell scripts | `kebab-case.sh`, plus two extensionless executables under a `tools` or `bin` directory | `tests/`, `scripts/`, `tests/tools/layer1-jobs`, `labs/window-chrome/bin/d2b-chrome-lab` |
| Node scripts | `kebab-case.mjs` | `scripts/copilot/`, `.github/skills/*/scripts/` |
| Python files | `kebab-case.py`, universally, twelve of twelve | see 6.4 |
| Documentation | `kebab-case.md` under a Diataxis directory; ADRs `NNNN-kebab-case.md` | `docs/` |
| Test layout | Layer and kind decide the directory, per the binding local manual | `tests/AGENTS.md` |

### 6.3 Rust

| Scope | Source | Class | Tier | Rule |
| --- | --- | --- | --- | --- |
| Identifier casing | RFC 430 | N | T1 | The casing standard itself |
| Identifier casing, tabulated | Rust API Guidelines, naming | N | T2 | Modules, functions, methods and locals `snake_case`; types, traits and enum variants `UpperCamelCase`; statics and constants `SCREAMING_SNAKE_CASE`; type parameters concise `UpperCamelCase`; lifetimes short lowercase |
| Acronyms | same | N | T2 | Acronyms and compound contractions count as one word: `Uuid`, not `UUID` |
| Word splitting | same | N | T2 | A word is never a single letter unless it is the last one |
| Conversions | same, item C-CONV | N | T2 | `as_` free, `to_` expensive, `into_` owned to owned |
| Getters | same, item C-GETTER | N | T2 | No `get_` prefix |
| Iterators | same, items C-ITER and C-ITER-TY | N | T2 | `iter`, `iter_mut`, `into_iter`, with `into_iter()` returning a type named `IntoIter` |
| Formatting | The Rust Style Guide | N M | T1 | Channel-tracking address; pin the edition when a finding depends on it |

**Two acknowledged unsettled areas, and neither may block without a local
rule.** The Rust API Guidelines mark **crate naming** unclear and give exactly
one firm rule, that a crate name should not carry a language suffix or prefix;
and they mark **feature naming** unclear in the same table while a separate
item gives a firm sub-rule against meaningless connective words, which is a
tension the document acknowledges about itself. Word order is explicitly
advisory, with consistency inside the crate mattering more than the particular
order. The `software` seat's Rust profile therefore may not block on crate
casing or feature naming on the authority of that document. This repository's
existing crate names are canon.

**The workspace lint perimeter is part of the Rust convention set, not separate
from it.** Section 2.1a records it in full, measured: a root
`unsafe_code = "forbid"` that 53 of 57 crates opt into by declaring
`[lints] workspace = true`, four crates outside it, and a committed ratchet
against file-wide `#![allow(unsafe_code)]`. A naming or style finding that
ignores which lint regime the crate is under will sometimes be a finding
against something the build already rejects.

### 6.4 Python, the present conflict and the settled direction

**The conflict, measured.** All twelve tracked Python files use
`kebab-case.py`:

```
labs/venus-vulkan-video/guest/firefox-decoder.py
labs/venus-vulkan-video/guest/firefox-gates.py
labs/venus-vulkan-video/guest/firefox-negative-control.py
labs/venus-vulkan-video/guest/firefox-youtube.py
labs/venus-vulkan-video/tests/capset-clear-check.py
labs/venus-vulkan-video/tests/gen-video-reject.py
nixos-modules/guest-control-token-materialize.py
tests/fixtures/gen-w3-cli-goldens.py
tests/tools/guest-workspace-drift.py
tests/tools/layer1-jobs.py
tests/tools/materialize-eval-fixture.py
tests/unit/meta/ci-runner-regression.py
```

Zero use `snake_case.py`. Every one is a standalone tool invoked by path, which
is why the convention survived: a hyphenated filename is not importable as a
module, and nothing here imports them. PEP 8 is unambiguous that modules take
short all-lowercase names in which underscores may be used, and a hyphen is not
a legal identifier character, so these files are unimportable by construction.
PEP 8 also carries its own escape hatch, that consistency with existing project
style wins over the guide, which is the same rule as `AGENTS.md`'s existing
code is canon. Both readings are defensible, which is exactly why this needs a
decision rather than a seat's judgement.

**The settled direction.** All Python files migrate to PEP 8 `snake_case.py`.

**What that means here, precisely:**

- **ADR 0053 renames no Python file.** The migration is a separate change, and
  a mechanical rename belongs in its own commit under this repository's
  one-logical-change rule.
- **Any Python filename finding raised against ADR 0053's own delta is out of
  scope for that candidate and must not block it.** The conflict is recorded;
  it is not this record's to fix.
- **Any new Python file introduced by ADR 0053 work is written
  `snake_case.py` from the start**, so the migration does not grow while it is
  pending.
- **The `software` seat prompt carries a transition rule, not an unconditional
  rule.** Until the migration lands, the prompt states: a **new** Python file
  named `kebab-case.py` is a finding; an **existing** `kebab-case.py` file is
  not, and neither is a change to one. After the migration lands, that
  transition rule is deleted and `snake_case.py` becomes unconditional. The
  panel implementation must either depend on the migration or carry this
  transition rule explicitly. It must not ship a rule that makes twelve
  committed files fail overnight, which is the failure mode that turns a seat
  into noise on its first run.
- **The transition rule is itself a liability.** A transition rule that nobody
  removes becomes permanent. Whoever lands the migration deletes the rule in
  the same change.

Neutral corroboration that `snake_case.py` is the ordinary practice, offered as
observation rather than as authority: both first-party Python codebases read
during source collection use snake_case module names throughout, and one
upstream repository that mixes conventions still uses snake_case for its own
tooling script. The normative source remains PEP 8.

Distribution and package naming, if it ever arises, is governed by the Python
Packaging Authority name-normalization specification, not by PEP 423, which is
**Deferred** and must not be cited as normative.

### 6.5 Shell

**Classify by shebang before citing anything.** This repository contains both
dialects, measured 2026-08-04 across 119 tracked `.sh` files and six tracked
extensionless files: 113 files declare `#!/usr/bin/env bash`, of
which 111 end in `.sh` and **two carry no extension at all**
(`tests/tools/layer1-jobs` and `labs/window-chrome/bin/d2b-chrome-lab`); 5
files declare `#!/usr/bin/env sh`; **2 declare `#!/bin/sh`, both extensionless
under `tests/tools/`** (`ci-shell` and `scrub-shell-environment`); and 3
sourced fragments declare a ShellCheck
`shell=bash` directive instead of a shebang. Seven POSIX `sh` targets, not
five, and the two the earlier count missed are exactly the extensionless ones,
which is the same trap twice. Those extensionless files are
the reason this rule is written as classify-by-shebang rather than
classify-by-suffix, and the reason D21's `shell` profile activates on
extensionless paths under a `bin` or `tools` directory. The Google Shell Style
Guide is a **Bash** guide and prescribes Bash-only constructs. Applying its
advice to a POSIX `sh` script produces a finding that would break the script.

The rule for the `software` seat's `shell` profile:

- Determine the dialect from the shebang, or from the ShellCheck directive
  where a file is sourced rather than executed. If neither is present, that is
  itself the finding.
- For a Bash target, the Google guide is citable as `A` T2, as mature-project
  practice and never as a standard, and ShellCheck codes are the mapping
  target.
- For a POSIX `sh` target, cite the Open Group Base Specifications and
  ShellCheck. Do **not** import Bash-only recommendations, and where the two
  sources disagree, prefer the ShellCheck code plus the declared shebang over
  the style guide.
- Where the guide and ShellCheck agree, cite the code, because a code is
  checkable and a guide section is not.

**Local shell conventions, measured 2026-08-04 by enumeration.** These are
observed canon with no gate behind them, and they outrank the Google guide
wherever they differ:

| Convention | Observed | Note for a finding |
| --- | --- | --- |
| Strict mode | `set -euo pipefail` is the dominant opening, 85 occurrences; `set -uo pipefail` appears 21 times, `set -e` 6, `set -eu` 5, `set -u` once | The `-e`-less variant is deliberate in scripts that accumulate failures and report them together rather than aborting on the first. A finding that demands `-e` on such a script has misread its shape; a finding that a **new** script omits strict mode entirely is sound |
| Function declaration | `name() {` universally, 522 occurrences; the `function` keyword appears **zero** times | The Google guide permits both. Local canon does not. A new `function foo()` is a convention finding citing this row |
| Function naming | `snake_case`, zero hyphenated function names | Cite this row, not the Google guide |
| Environment variables | `SCREAMING_SNAKE_CASE`, 107 of 108 exports, with `D2B_` and `D2BD_` prefixes on repository-owned ones | A new unprefixed global export is a finding; a local loop variable is not |

**The no-Bash rule is narrower than it looks, and confusing it is a real
error.** ADR 0017 and `AGENTS.md` forbid a Bash **CLI fallback**: the Rust
`d2b` binary is the only operator surface, the legacy opt-in environment knobs
are no-ops, and an abstract-syntax-tree walker plus a source policy enforce that
the Rust CLI does not invoke bash. That rule is about the operator surface. It
does **not** ban contributor and test Bash scripts, and could not, since 113 of
them are committed and the Layer-1 gate is built from them. A seat that reads
the no-Bash invariant as a ban on shell in `tests/` or `scripts/` has
misapplied it, and a seat that lets a Bash bridge back into the CLI path has
missed the rule that matters.

### 6.6 Nix and NixOS

| Scope | Source | Class | Tier | Rule |
| --- | --- | --- | --- | --- |
| Package file and directory layout | Nixpkgs `pkgs/by-name/README.md` | A | T2 | A mechanical sharded layout with two hard constraints and a machine validator, which is a good precedent for replacing a prompt-only assurance with a gate |
| Conventions, file naming and organisation | Nixpkgs `CONTRIBUTING.md` and `pkgs/README.md` | A | T2 | The current homes for syntax, coding conventions, package naming and versioning |
| Structural configuration | RFC 42 | N | T1 | Structural `settings` rather than stringly `extraConfig`. **Owned by `nixos`, not by `software`'s Nix profile**, because it is a statement about what the module system can merge and type-check |
| Merge priority | This repository's ownership rule, section 3.3 | N | T1 | Priority is justified against ownership. `mkForce` is permitted where a framework-owned invariant intentionally neutralizes a competing definition, the net VM `10-eth-dhcp` neutralizer being the canonical critical case, and never to silence an unexplained conflict; `mkDefault` preserves consumer overrideability where the framework supplies a default. **Owned by `nixos`** |
| Formatting | RFC 166, which adopted nixfmt as the standard formatter through the RFC process | N | T1 | Formatting belongs to the formatter, not to a seat. This is a boundary, and the boundary is a refusal: a formatting finding is never blocking, and the remedy is to run the formatter |
| Option naming | NixOS manual option declarations, plus convention | A | T2 | `camelCase` option attribute paths are long-standing convention, **not** specification. Flag as advisory |

Three cautions. The Nixpkgs manual's coding-conventions chapter is now a stub
of "this section has been moved" pointers and contains no conventions; citing
it is citing an empty page. Nixpkgs' argument-renaming discipline, preferring
to rename the argument over hiding a version constraint in an override on the
stated rationale that a hidden constraint is worse than an explicit conflict,
is a genuinely useful transfer, and it belongs to **both** the `nixos` seat and
`software`'s Nix profile depending on whether the question is what the module
system does or how the expression reads. And the split between those two is
section 1.2's, restated in 2.1d and 3.3, so a prompt author does not have to
infer it from a source table.

### 6.7 Directories, branches and commits

- **General directory naming has no standard, and local structure wins.** The
  three defensible anchors are the Nixpkgs sharded layout, the upstream
  planning rule that files which change together live together and are split by
  responsibility rather than by technical layer, and this repository's own tree,
  which `AGENTS.md` makes canon. A seat states which of the three it is
  applying and never issues a directory-naming finding on external authority
  alone. `AGENTS.md` already says where new behaviour goes: a focused file
  under `nixos-modules/` or its components directory, wired in from the
  aggregator, rather than fattening an existing file.
- **Branch naming has no standard.** `git check-ref-format` is the only
  normative constraint; everything beyond it is project convention.
- **Commit conventions are local and stricter than the community norm.**
  Conventional Commits is `A` T2 and advisory. This repository's binding rules
  are its own: short imperative area-prefixed subjects, why in the body, a
  trailing wave tag on feature branches, and an outright ban on AI, tool and
  model attribution in subjects, bodies, pull-request descriptions, changelog
  entries and shipped docs, including the co-authorship trailer. Every imported
  commit or pull-request prompt asset must have its attribution trailers
  stripped, and an attribution trailer is a rule violation with a named owner
  in `docs/contributing/changelog-and-commits.md`.

### 6.8 Conflicts between common remote guidance and this repository

Summarised so that a prompt author does not have to rediscover them. Local
always wins; what varies is how visible the conflict is.

| Remote guidance | Local rule | Resolution |
| --- | --- | --- |
| Commit assets that add attribution trailers | No AI, tool or model attribution anywhere, including the co-authorship trailer | Strip attribution from every imported asset; treat a trailer as a rule violation |
| Google Shell Style Guide's Bash-only constructs | Both dialects present; the no-Bash rule is about the CLI surface only | Classify by shebang, per 6.5 |
| Community assets' emoji, arrows and typographic dashes | ASCII hyphen only, gate-enforced across all file types | Extract structure, retype text, never paste |
| Multi-band severity ladders and verdict matrices | One blocking channel: `signoff` is true if and only if `recommendations` is empty | Import the threshold mechanism, not the bands |
| Reviewer assets that write report files to fixed paths | The record is four producer-written fields; scratch goes under the panel scratch directory and is not read by the gate | Strip all file-writing instructions |
| Verification skills that require the reviewer to run commands | Reviewers do not re-run validation; evidence is supplied | Invert the actor: audit the supplied evidence against the same table |
| Simplification assets that reward smaller diffs with line counters | Code golf and lost validation are rejections | Import the taxonomy, drop the counters |
| Assets that freely propose adding libraries | ADR 0009 governs the supply chain; the six-axis evidence rule applies | External guidance cannot waive ADR 0009 |
| Numeric style thresholds from community prompts | Formatting belongs to formatters; lints are the mechanical gate | A threshold that matters belongs in a lint configuration |
| Model-judgment reviewer selection | D21's versioned constant trigger table | A decided divergence with a stated rationale, not a finding |
| Assets that assume the reviewer may push fixes | The panel produces verdicts, not edits | State the no-edit boundary in every seat prompt |

## 7. Caveat register

Recorded so that a later reader can tell a moved address from a wrong one. All
observations are from 2026-08-04 unless stated.

- **GitHub Copilot documentation is actively restructuring.** Observed
  redirects and removals: the custom-instructions how-to moved under a
  `copilot-on-github/customize-copilot` prefix; `about-custom-agents` returns
  404 at its old location and resolves under a `cloud-agent` segment; and the
  best-results tutorial redirects from a `coding-agent` segment to a
  `cloud-agent` one. The two pages most load-bearing for the `agentic` seat
  were re-fetched on 2026-08-04 and resolved with the content described here.
- **VS Code moved its whole customization tree** from a Copilot-scoped path to
  an agent-customization path. The new path was re-fetched and resolves.
- **Anthropic moved its Claude Code guidance.** The engineering-blog
  best-practices post now redirects to `code.claude.com/docs/en/best-practices`
  and the content was rewritten rather than relocated; the prompt-engineering
  overview moved from a docs domain to a platform domain; and the agent-SDK
  post moved to a different company domain. The archived original of the
  April 2025 post is available at the Internet Archive and shows **no byline**
  in its extracted text, so authorship is not verifiable from it. Cite the post
  as Anthropic-authored.
- **`github/awesome-copilot` removed its `prompts/` and `chatmodes/`
  directories.** At the pinned commit only `agents/` and `skills/` exist. Any
  pre-existing citation of a `prompts/*.prompt.md` path at this pin is wrong.
  This is exactly the kind of movement the register exists for, and it is why
  the collection is marked `A M`.
- **OpenTelemetry** retired its attribute-naming path; the live page is the
  general naming page, re-fetched and marked Stable.
- **Diataxis** rate limited automated fetches during source collection. The
  framework and address are stable and this repository already organises around
  it; treat the fetch failure as a rate limit, not a dead link.
- **`https://adr.github.io/`** rendered as an effectively empty page. Do not
  cite it as a substantive reference.
- **The Nixpkgs coding-conventions manual chapter is a stub.** Every section
  is a "moved" pointer. Cite `CONTRIBUTING.md`, `pkgs/README.md` and
  `pkgs/by-name/README.md` instead.
- **Channel-tracking addresses.** The Rust Style Guide address is the nightly
  channel; the Clippy lint index tracks nightly; NixOS and Nixpkgs stable
  manuals move with each release; systemd `latest` man pages track upstream;
  and the Nix manual `latest` address redirects to a versioned URL. Pin the
  version in the seat prompt where a finding depends on it.
- **Community-versioned sources.** OWASP ASVS and WSTG change materially
  between major versions; pin the version in the prompt.
- **Dated sources.** The NIST firewall guideline is from 2009, the secure
  development framework from 2022, and the flaky-tests post from 2016. All
  remain citable; none should be cited as current practice without saying so.
- **Licensing.** `gastownhall/gascity-packs` has **no repository LICENSE at the
  pinned commit**; the GitHub licence endpoint returns 404 for it. MIT appears
  only inside its two vendored subtrees, each with an `upstream.toml` recording
  the upstream project, commit and licence. Its own formulas and stage prompts
  are therefore **behavioural-evidence-only** sources: readable and citable for
  what upstream declares, with no copying and no adaptation of their text,
  organisation or expressive structure. An earlier revision of this document
  described them as read-for-structure, which was inconsistent with the T4 rule
  in section 0.2 and is corrected.
  `hesreallyhim/awesome-claude-code` is **CC BY-NC-ND 4.0** at the surveyed
  revision: the no-derivatives term means adaptation is not permitted, so it is
  a discovery index only, and anything it lists must be checked against that
  project's own licence before use. `f/awesome-chatgpt-prompts` prompt content
  is **CC0**, which makes it the most permissive source surveyed and also the
  least useful: it is general-purpose persona prompts with no evidence bar and
  no finding threshold, and its value here is as a **negative example** of
  exactly the failure `docs/contributing/panel-review.md` records.
  `anthropics/skills` is mixed-licence within one repository and carries a
  demonstration-purposes disclaimer, so it is `A M` and is not cited as
  uniformly open source.
- **Boris Cherny: an explicit negative result, recorded so the search is not
  repeated.** Anthropic-owned webinar pages describe him as the inventor and
  founder of Claude Code, which establishes the **attribution and the role**
  and is citable for that. Those recordings are registration-gated, so their
  **contents are not quoted here**. A search of his GitHub account's repository
  list and a scoped code search found **no first-party public prompt, skill,
  agent or instruction artifact** authored by him; the only recently updated
  repository on that account is marked a fork, and a fork is not evidence of
  authorship. Every widely circulated "Claude Code prompt" or tip compilation
  attributed to him is third-party, self-described in at least one case as
  synthesised from dozens of sources, published in at least one case under a
  misspelled filename, and carries no verifiable licence. **None is cited, none
  is quoted, and none is adapted.** Where the underlying idea is genuinely
  useful, the same substance is available first-party: plan-then-code and
  subagent context isolation are both documented on `code.claude.com`. Nothing
  in this document rests on leaked or extracted prompt research, and no such
  research was performed.
- **Upstream product code.** The Compound Engineering and Gas City prompts are
  pinned blobs. They are product code, not specifications, and can change
  without notice.
- **Remote citations are by path and section, not by line range.** The GitHub
  file interface does not return line numbers, so remote citations here are
  path plus heading. Line-ranged remote citations would require a local
  checkout at the pinned commit.
- **Not measured.** Nobody has measured how often each optional-seat trigger
  would fire against this repository's history. The expectation that
  `reliability` fires on nearly every code change is inferred from
  the tree shape, not observed, and the same is true of the expectation that
  `quorum_fill` will seat `nixos` on Rust-only candidates now that the
  `rust-sources` rule is gone. Nor has the round-count cost of a ten-seat
  floor been measured; ADR 0053's Consequences section states the arithmetic
  and marks it as arithmetic.
- **Read versus enumerated.** Every asset cited in a source table above was
  read in full at its pinned commit. Larger inventories exist in
  `github/awesome-copilot` and in the Compound Engineering and Superpowers
  sets, and unread files are deliberately not cited: in this collection
  especially, a filename is a poor predictor of content, and at least one asset
  whose name promises review guidance contains a templated generator instead.

**There is deliberately no link-check gate.** A network-dependent blocking
check is a flaky blocking check, and the failure it would catch, a moved
documentation URL, is not a failure that should stop a merge. The mitigations
are the retrieval date, the `M` marker, and the `docs` seat, which reviews this
file whenever it changes.

## 8. Change discipline

- This document is decision support. Changing it does not change D21. Changing
  the pool, the selection rules, the surface classifier, the `software`
  language-profile activation rules, the verdict schema, or the mandatory set
  requires amending ADR 0053.
- A change here that alters what a seat blocks on is a change to the panel's
  behaviour and lands with the corresponding `.github/agents/panel-*.agent.md`
  edit, not before it and not after it.
- The shared finding-bar block is byte-identical across seat files by gate.
  Nothing in this document may be copied into it.
- Adding a source means adding its normativity marker, its tier, and its
  retrieval date. A source with any of the three missing is not usable in a
  blocking finding.
- Adding a source also means stating its licence and provenance where reuse of
  its text or structure is intended. A source whose provenance cannot be stated
  is not usable at all.
- This file lives under `docs/adr/specs/` and is deliberately **not** in the
  ADR index. The index gate matches ADR files at depth one only; adding this
  file to the index would break set equality.
