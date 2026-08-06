# Gate 0 re-evaluation: SPIKE-01 RSS rerun amendment

| Field | Value |
| --- | --- |
| Trigger | FR-056 - amending an Accepted specification-set member re-triggers Gate 0 across the manifest |
| Amendment request | [`amendment-spike-01-rerun.md`](./amendment-spike-01-rerun.md) |
| Amended members | `ADR-046-validation-and-delivery`, `ADR-046-feasibility-and-spikes`, `ADR-046-resource-store-redb`, `ADR-046-decision-register` (all Accepted) |
| Grounding artifact | `proofs/redb-resource-store-spike/RESULTS-rerun-2026-08-02.md` |
| Satisfies | FR-056 |
| Status | Mechanical half discharged; human-review half **has a non-empty input set** and is recorded in section 4 |

This is the second Gate 0 re-evaluation of the program. The first,
[`gate0-reevaluation.md`](./gate0-reevaluation.md), covered the
delivery-contract amendment and recorded an empty human-review input set.
That is not the case here, and section 4 is the part of this document that
matters.

## 1. What was amended

The disposable resource-store proof's whole-process RSS gate was re-measured
under the public heavy gate on 2026-08-02. The result of record is
`proofs/redb-resource-store-spike/RESULTS-rerun-2026-08-02.md`:

| Item | Value |
| --- | --- |
| Hard fixture | `rss-fixture --resources 10000 --watches 100` |
| Raw runs | 18,428 / 18,396 / 18,552 KiB |
| Median | 18,428 KiB |
| Threshold | 24,576 KiB |
| Headroom | 6,148 KiB |
| Baseline subtraction | None |
| Verdict | MEASURED-PASS |

Four Accepted members now state that result instead of the superseded failure:

- `ADR-046-validation-and-delivery` - the §3.2 `ADR046-W1` wave row, the
  `ADR046-feasibility-001` and `ADR046-store-004` determination rows, and the
  §10.4 narrative.
- `ADR-046-feasibility-and-spikes` - the redb rationale paragraph, the
  evidence-classification matrix row, the SPIKE-01 and SPIKE-02 status rows,
  and the `ADR046-feasibility-001` evidence row.
- `ADR-046-resource-store-redb` - the staged aggregate-RSS paragraph, the
  current-code-fit feasibility-proof row, the feasibility-gate narrative, and
  the `ADR046-store-002` / `-004` / `-005` evidence and validation rows.
- `ADR-046-decision-register` - D128.

`proofs/redb-resource-store-spike/RESULTS.md` and
`proofs/redb-resource-store-spike/RESULTS-corrections.md` are untouched. The
first remains the historical failed record; the second remains a
non-authoritative prototype and is now mechanically excluded from authority
(section 5).

## 2. Mechanical half of Gate 0

The three generated manifests were regenerated with the repository generators
(`xtask spec-registry`, `xtask implementation-graph`) rather than hand-edited.
`spec-registry` reported the unchanged census: **55 members, 545 work items**.

The complete mechanical consequence of the amendment is:

| Manifest | Change |
| --- | --- |
| `ADR-046-spec-set.json` | Exactly 4 `sha256` digests, one per amended member. Nothing else. |
| `ADR-046-work-items.json` | 4 `evidence` and 3 `validation` string fields (`ADR046-feasibility-001`, `ADR046-store-002`, `ADR046-store-004`, `ADR046-store-005`). |
| `ADR-046-implementation-graph.json` | 3 `validation` string fields (`ADR046-store-002`, `-004`, `-005`). |
| `ADR-046-implementation-graph.md` | **Byte-identical.** |

Recorded digest transitions:

| Member | Before | After |
| --- | --- | --- |
| `ADR-046-decision-register` | `9d2fd322b3a557116458350d6d337d50970ae97dda2d6439acfc05e4e9ccc18c` | `aa8cfe6fd18a4da86b0dbd02d8a3ed8b7afb93a4db66ac5d2b4ca4ec60e588a2` |
| `ADR-046-feasibility-and-spikes` | `76c514283128f42925a6bd6d0105aa0b6f7ce11440b19227292c80418ae8cf72` | `d0634339b6a1712769cae183d04a17dc710530b98ec7da7f2df71538b8f8c720` |
| `ADR-046-resource-store-redb` | `6d4e50a8106d59c716533364e89373a0d2a0698f4e0f8f75d0e3ccfb81c57e5a` | `d0c618d34b47b2203bdbf1b394c6379b4a91495191fb835ef285ea34139a095f` |
| `ADR-046-validation-and-delivery` | `d361e2b39e4949b5f5c6d40b1a648fa8d233f03405337deca2432ef88d196aad` | `0b6c557ab3dfddc1b6817abc914a4a12faa6624b8beb83f9d418acee8f19d733` |

That the implementation graph is byte-identical is the load-bearing
observation, not a footnote: it means **no work item was added, removed,
renamed, reassigned to another wave, or moved between implementation states,
and no dependency edge changed.** The specification-to-work-item bijection is
untouched, so the amendment cannot have disturbed any wave's scope. The
mechanical half of Gate 0 is discharged by `make test-drift`.

## 3. What the amendment deliberately does not change

- No member changed status. All four remain `Accepted`.
- `ADR046-store-004`, `ADR046-store-002`, `ADR046-store-005`, and
  `ADR046-reconcile-003` remain `Planned` in `ADR046-W5`. A passing
  disposable-proof measurement does not advance a production work item.
- The 24,576 KiB gate, the no-baseline-subtraction rule, and the four named
  design corrections (range-seek replay, streaming decode, shared immutable
  fan-out, global watch-admission budget) are all unchanged.
- `CHANGELOG.md` keeps the released failure entry verbatim. Correcting the
  shipped release notes is the release-note owner's call, not this amendment's
  (section 6 states the mechanical cost of that decision).

## 4. Human-review half of Gate 0

FR-056 requires re-opening the amended specifications' validation and panel
evidence, and requires that a wave holding evidence gathered before the
amendment regather it rather than carry it forward. Unlike the first
re-evaluation, the input set here is **not empty**.

### 4.1 Enumerated input set

| Wave | Delivery state | Disposition |
| --- | --- | --- |
| `ADR046-W0`, `ADR046-W1` | Delivered under the written waiver ([`waiver-w0-w1.md`](./waiver-w0-w1.md)) | Unaffected. Their 14 work items are recorded `Merged`; the amendment changes no work item's state. `ADR046-feasibility-001` stays `Merged` on the strength of the proof crate existing, which the amendment does not touch. |
| `ADR046-W2`, `ADR046-W3`, `ADR046-W4` | Sealed and merged | Unaffected. Each sealed against a snapshot in which the RSS row read MEASURED-FAIL, which was the true state at that time and is preserved verbatim in `RESULTS.md`. None of the three owns a work item whose evidence or validation string changed. |
| `ADR046-W5` | **Panel request outstanding, no seal** | **Invalidated. Must regather.** See 4.2. |
| `ADR046-W6`-`ADR046-W8` | Not started | Nothing to regather. |

### 4.2 The W5 finding

The delivery state holds one `adr046w5` candidate,
`d20267eec23f90b9cd6931e4bd322b66e259533849c8170617fbd002381493a4`, bound to
snapshot `7a04d9b86df6c8b8704b4bd79ddc25603fedae47d1a521f0b6fa420451816c3a`
over head `19b77dad63060bcadd41f1ef800978d2c53cc030` (pull request 368). It
carries a ten-role `panel-request.json` and an imported local-host evidence
set, and it carries **no attestation records and no seal**.

Two of its imported evidence records are `redb-rss-spike-observation` and
`redb-full-scale-proof`, both imported before this amendment existed. The
first is precisely the class of evidence FR-056 names: an RSS observation
gathered against the superseded conclusion.

The disposition is therefore:

1. That candidate's panel request is superseded. The amendment is content
   change, and content change invalidates every prior sign-off in the phase.
   Since zero attestations exist, nothing is being withdrawn - but a panel
   dispatched against that snapshot must not be counted.
2. `ADR046-W5` must take a **new** candidate snapshot after this amendment
   merges, re-import its validation evidence against that snapshot, and issue
   a fresh ten-role panel request. `redb-rss-spike-observation` in particular
   must be re-imported so it points at the rerun artifact rather than at the
   superseded conclusion.
3. `ADR046-W5` must not seal until that has happened. This document is the
   Gate 0 pass that unblocks the seal; it is not a substitute for the seal's
   own conditions.

This is a real cost, and it is the cost FR-056 exists to make visible rather
than to avoid.

### 4.3 What the passing rerun does not license

Stated here because it is the misreading this amendment most invites: the
rerun is a measurement of a **disposable proof crate**, not of
`packages/d2b-resource-store-redb`. It does not make the production backend,
the watch dispatcher, or the real-backend reaction benchmark reachable,
accepted, or merged. Those three still owe:

- production whole-process RSS evidence measured on the production engine
  against the unchanged 24,576 KiB gate with no baseline subtraction;
- the conformance, security, durability, watch-budget, and backup/migration
  evidence their own work-item validation rows name;
- `ADR046-reconcile-003`'s reaction benchmark against the accepted backend.

The amended member specs now say this in every place they previously named the
failure, and the policy lint pins the sentence that says it.

## 5. Prototype exclusion

`proofs/redb-resource-store-spike/RESULTS-corrections.md` carries a complete,
plausible, MEASURED-PASS threshold table whose RSS figures differ from the
authoritative rerun by a digit transposition: `18,468 KiB` / `6,108 KiB`
against the real `18,428 KiB` / `6,148 KiB`. It is exactly the artifact a
hurried reader cites.

Excluding it by convention was judged insufficient. It is now excluded
mechanically by `the_corrections_prototype_is_never_an_authoritative_result`
in `packages/d2b-contract-tests/tests/policy_adr046_spec_literals.rs`, which
asserts that the prototype is not a registered result source, that it does not
satisfy the canonical measurement parser, that it does not carry the
authoritative fingerprint, and that neither of its transposed figures appears
anywhere under `docs/**` or in `CHANGELOG.md`.

## 6. Standing consequence

The RSS measurement literals are now pinned in two directions at once, and a
future author needs to know both:

- The **current** figures (`18,428 KiB`, `6,148 KiB`, `24,576 KiB`) are
  inventoried at exact global counts across `docs/**` and `CHANGELOG.md`.
- The **superseded** figures (`25,216 KiB`, `24.625 MiB`, `640 KiB`,
  `2.6% above 24,576 KiB`) are inventoried at exactly one copy each: the
  released changelog entry. Reintroducing the failed figure into any
  specification fails the lint, and so does deleting the retained history.

The concrete consequence: **a changelog fragment that quotes any of those
literals will fail `test-policy` when it is folded into `CHANGELOG.md`**,
because the fold moves text into an inventoried document. The failure names
the file and the expected count, so the fix is to bump the pinned count in the
same change that adds the release note. The fragment landed with this
amendment deliberately carries no numeric literal for that reason.
