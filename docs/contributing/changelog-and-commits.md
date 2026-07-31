# Changelog, versioning, and commit conventions

Every PR that changes code ships release notes. This file carries the detail:
the fragment workflow for concurrent branches, the auto-release path, the
changelog lifecycle at a version cut, the process-marker ban and its ratchet,
and the full commit trailing-tag grammar.

The binding rules are in [`../../AGENTS.md`](../../AGENTS.md) under "Changelog
and commits".

## Changelog & Releases

Every PR that changes code **must** ship release notes. The CI gate
enforces this and accepts either form: an entry in `CHANGELOG.md`, or a
changelog fragment under `changelog.d/`.

## Format

[Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Add entries under
`## [Unreleased]`. When ready to release, rename the section to
`## [X.Y.Z] - YYYY-MM-DD`.

## Fragments (`changelog.d/`)

When more than one branch is in flight, do **not** edit `CHANGELOG.md` -
every branch appending to the same `## [Unreleased]` block is a guaranteed
merge conflict. Write one `changelog.d/<branch-name>.md` fragment instead,
holding the same `### <Section>` headings and entries you would have added
to the block. Two branches never write the same file.

The integrator folds the fragments at merge time with
`make changelog-fold` (`cargo run --manifest-path packages/Cargo.toml -p
xtask -- changelog-fold`): entries collate by
section into `## [Unreleased]` in Keep a Changelog order, released
versions are untouched, and the consumed fragments are deleted. A
fragment with an unknown heading, a repeated heading, an empty section, or
content outside a section fails the fold rather than losing the entry. See
[`changelog.d/README.md`](../../changelog.d/README.md).

## Auto-release

Merging to `v3` with a new version header in `CHANGELOG.md` triggers:
1. Auto-creation of git tag `vX.Y.Z`
2. Build of all host binaries (`d2bd`, `d2b`, `d2b-priv-broker`,
   `d2b-wayland-proxy`, `d2b-activation-helper`)
3. GitHub Release with changelog notes + binary tarballs + `SHA256SUMS`

`v3` is the clean-break integration lineage and never merges to `main`, so
the release path cuts from `v3`, not `main` (see
[`docs/specs/ADR-046-validation-and-delivery.md`](../specs/ADR-046-validation-and-delivery.md)
"Only after all six hold").

Consumers can fetch pre-built binaries from the release instead of
building from source.

## Versioning

Follow semver. The version in `CHANGELOG.md` is the single source of truth.

## Commit-tag mapping

The tag examples in [Commit conventions](#commit-conventions) use this
mapping, and every commit that comes out of a panel-fix round MUST
carry the relevant tag:

- `Wn` = wave / phase number from the plan's parallelization graph
- `Wnfu` = first follow-up round on wave `n` after the first panel
  findings land
- `Wnfu<M>` = follow-up round `M` on wave `n` when a specific
  follow-up round must be named (for example `W5fu1`)
- `CN`, `HN`, `MN`, `LN` = finding ordinal `N`, prefixed by the
  severity letter from the JSON output (`critical` → `C`, `high` →
  `H`, `medium` → `M`, `low` → `L`)

Example: `( W1fu1 H3 )` means "wave 1, follow-up round 1,
addresses finding ranked HIGH-3."

Inline references to a specific commit in prose elsewhere may
use the compact form `(W2fu4 H10)` for readability - that's
shorthand for citing a commit, not the literal trailing tag
that the commit subject must end with. The trailing-tag form
in the commit subject itself always uses the spaced canonical
form (e.g. `... ( W2fu4 H10 )`).

## Versioning & changelog

The project follows [Semantic Versioning](https://semver.org/) and
[Keep a Changelog](https://keepachangelog.com/). The CHANGELOG is
organised **by version**, never by development phase.

## Changelog lifecycle

- **While a version is in development**, entries accumulate under the
  top `## [Unreleased]` block. It remains consumer-facing and follows
  the same process-marker ban as released sections; wave, phase,
  follow-up, round, panel, and finding bookkeeping stays in plans,
  commits, and PR descriptions.
- **When a version is cut**, the `[Unreleased]` block is renamed to
  `## [X.Y.Z] - YYYY-MM-DD` and its contents are **summarised by
  version**:
  - Collapse any per-wave/per-phase substructure into the standard
    Keep-a-Changelog groups (`Added`, `Changed`, `Fixed`,
    `Deprecated`, `Removed`, `Security`). There are no
    `### Added (W6)`-style subsection headers in a released section.
  - Strip every internal process marker - wave/phase/revision/
    follow-up/panel/round/finding tags such as `W3`, `W4-fu`,
    `( W1fu3 H20 )`, `P6`, `D5/P2.3` - from the released prose.
  - Each released section reads as a coherent, consumer-facing
    summary of what changed, not as a log of how the work was
    organised internally.
- A fresh empty `## [Unreleased]` block is left at the top after a
  cut. `manifestVersion` / `bundleVersion` bumps and breaking
  changes always get an explicit released entry.

## Process markers stay out of shipped artifacts

Internal development bookkeeping - wave tags (`W3`, `W4-fu`,
`W2-followup`), phase tags (`P0`-`P7`, `v1.1-P4`, `ph6-…`),
decision codes (`D5/P2.3`), follow-up/round/finding refs
(`fu3`, `H20`, `(rust-1)`) - is for organising work, not for
shipping. Do **not** introduce these markers into:

- source comments in `nixos-modules/`, `pkgs/`, `packages/`, or `proofs/`;
- shipped docs prose under `docs/{reference,how-to,explanation}/`,
  `proofs/**/*.md`, `README.md`, `SECURITY.md`, or example READMEs;
- any user-facing CLI surface (`clap` `about`/`help`/`long_help`
  text, error/observed-state messages, JSON envelope fields);
- CI workflow names, job names, step names, and test output that a
  contributor sees in GitHub Actions logs. CI labels should describe
  the behavior being validated (for example, "ADR index coverage
  guard" or "host validate dry-run"), not historical phase/process
  codes;
- every CHANGELOG section, including `[Unreleased]`.

These markers are still expected and welcome in the contexts where
they are load-bearing:

- planning artifacts (a session `plan.md`, the wave/parallelization
  graph);
- this file and the other process docs (Panel review, Commit
  conventions, `## Daemon-only end-state (P6 onward)`) that
  *document* the methodology;
- `docs/adr/**` - ADRs are dated historical records and may name the
  wave/phase that produced a decision;
- commit messages and PR descriptions on in-development feature
  branches (see Commit conventions).

The ban is mechanically enforced by `scan_process_markers` in
`tests/tools/tier0-first-pass.sh`, which runs as part of
`make check-tier0`. That script is authoritative for the governed
paths, marker patterns, narrow functional exceptions, exact diagnostics,
and use of the active exemption set. The pin's typed schema and frozen
universe are independently checked by
`packages/xtask/src/process_marker_pin.rs`; consult both implementations
when changing the ratchet.

Existing violations are recorded in
`tests/golden/pinned/process-marker-legacy-paths.json`. Its
`activePaths` array is the current exemption set and `retiredPaths`
records cleaned paths. Both arrays must be sorted and disjoint, every
entry must be a normalized relative path, and their combined path
universe must match the fixed SHA-256 digest embedded in both checkers.
The digest freezes the combined universe; there is no editable count
budget and no permitted swap that adds a different path.

An active path is exempt only while the scanner still finds a violation
there. Cleaning that path makes the gate fail with a `STALE:` line; move
the path from `activePaths` to `retiredPaths` in the same change, preserving
the frozen universe. A retired path is not exempt, so a marker there is
reported as a new violation. Handle the contributor-facing failure modes
as follows:

- For a new violation outside the allow-list, remove or reword the
  marker. If it is a genuine functional identifier, add a narrowly
  scoped scanner exception with policy review rather than growing
  legacy debt.
- For a stale active entry, move it to `retiredPaths`; do not delete it
  from the frozen universe.
- For a pin validation failure, restore sorted, unique, normalized arrays
  whose disjoint union matches the embedded digest. Do not add, delete, or
  replace a frozen path.

The exact scanner failure text may evolve;
`tests/tools/tier0-first-pass.sh` remains the authority for it, while
`packages/xtask/src/process_marker_pin.rs` is authoritative for typed pin
validation.

There are two deliberate functional exceptions. The consumer-facing
`d2b.defaultSwitchReadiness.<wave>` option namespace (keys
`w4Fu`…`p7`), its `readinessWaveSpecs` schema, and the
`/var/lib/d2b/validated/<wave>.json` evidence contract use
`wave`/phase tokens as **functional identifiers**. Those are part of
the public option/schema surface and are not bookkeeping; leave them.

`packages/xtask/src/delivery/` also has a narrow exception for the
delivery tool's closed `W0` through `W8` namespace. These exact tokens
identify CLI values and state-path segments rather than development
bookkeeping. The exception applies only inside that delivery
implementation; suffixed bookkeeping forms remain violations.

## Commit conventions

> The trailing wave-tag scheme below applies to in-development
> commits on feature branches / worktrees, where wave/phase tags are
> load-bearing planning context. It does not license process markers
> in shipped code, docs, or any CHANGELOG section - see
> [Versioning & changelog](#versioning--changelog).

- **Subject.** Short, imperative, prefixed with the touched
  area: `net: fix 10-eth-dhcp neutralization`,
  `manifest: bump manifestVersion to 2`,
  `cli: tighten exit-code table`.
- **Body.** Wrap at ~72 cols. Explain *why*, not what - the diff
  shows the what.
- **Traceability - canonical tag form (forward, W2fu4+).**
  Every commit subject MUST end with a trailing parenthesized
  tag in one of these exact forms:

  - `( W<N> )` - wave-N implementer work (no finding ref)
  - `( W<N>fu<M> )` - wave-N follow-up round M integrator
    merge (no finding ref); merge-shape suffixes like
    `octopus` are NOT permitted in the tag
  - `( W<N>fu<M> <S><N> )` - single finding fixed in
    follow-up round M. The finding-tag is `<S><N>` where
    `<S>` is the severity letter from the reviewer JSON
    (`C` = critical, `H` = high, `M` = medium, `L` = low)
    and `<N>` is the ordinal within that severity. Example:
    `( W2fu1 H3 )` = wave 2, follow-up 1, HIGH-3.
  - `( W<N>fu<M> <S1><N1> <S2><N2> ... )` - multi-finding
    follow-up commit when two or more findings genuinely express
    one coherent change and scattering them would not add
    review value. The trailing tag enumerates every finding
    closed by the commit, separated by single spaces. The commit
    body MUST explicitly call out the multi-finding scope (which
    findings are closed and why batching them in one commit
    aids review). Example: W3fu3 `( W3fu3 H4 H5 H6 )` aligned
    three docs (`privileges.md`, `AGENTS.md`,
    plan.md "Spec corrections") to point at `schemas/v2/` as
    the current bundle baseline in a single coherent commit.
    Reach for the single-finding form by default; reach for
    multi-finding only when the alternative is three or more
    trivially-small commits that all express the same
    statement.
  - `( W<N> <S><N> )` - single finding fixed inside the
    wave itself (rare; usually findings come during follow-ups)
  - `( W<N>a-<H> )` or `( W<N>a H<H> )` - post-wave **opening
    phase** that closes specific Spec-corrections deferrals or
    ships infrastructure work. Used when the work is genuinely
    pre-wave-N+1 prep rather than an in-wave follow-up. Examples:
    `( W3a-1 )` for the W3a-1 testing-infra batched harness,
    `( W4a H1 )` for the W4a-H1 audit retention commit. The
    spelling with the space (`W4a H1`) is what the W4a
    landings used and is the canonical form going forward; the
    dash-form (`W3a-1`) is permitted as a historical exception
    for the W3a commits that already shipped. Multi-finding
    follow-ups within an opening phase use the same
    `( W<N>afu<M> <S1><N1> <S2><N2> ... )` shape as a normal
    wave round (e.g. `( W4afu1 H1 H2 )` for a W4a follow-up
    closing R1 findings).

  Docs-only commits that don't close a specific finding (e.g.
  CHANGELOG.md grouping, AGENTS.md operating-manual updates after
  a wave closes) MAY omit the trailing tag when the subject
  itself is unambiguous about the scope (e.g. `CHANGELOG: W3fu4
  H1 H2 H3 H4 H5 grouped entry (R4 closure)`). Reach for the
  tag form whenever doing so would aid traceability; treat omitting
  it as the exception, not the default.

  No leading-tag form. No partition/topic words inside the
  parenthesized tag - those go in prose. Every commit
  produced in a panel-fix round MUST carry the relevant
  tag; see [Panel review](#panel-review) for the mapping
  and phase-gate policy.

  Historical exception: pre-W2fu4 commits in W0/W1/W2 carry
  some leading-tag variants (`(W2 s3) ...`) and some merge
  subjects with topic words (`(W2fu1 ipc)`, `(W2fu2 octopus)`).
  These remain in history for reference; future waves use the
  canonical form above. See the
  `docs: codify trailing-tag canonical form` commit
  (W2fu4 H10) for the full retrospective.

- **Signing.** Sign-offs / GPG signing are not used.
- **Typography.** Only the ASCII hyphen `-` may spell a dash in the
  subject or the body. See the Don'ts entry for the repository-wide rule
  and the banned codepoint list.
- **AI/tool attribution.** Do not tag or list the AI agent, assistant,
  or model used in commit subjects, commit bodies, PR descriptions,
  changelog entries, or shipped docs. Do not add `Co-authored-by`
  trailers for AI tools unless the human explicitly requests one for
  that change.
- **Atomicity.** One logical change per commit. Mechanical
  reformat or rename passes go in their own commit so the
  human-reviewable diff stays small.

