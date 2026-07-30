# Gate 0 re-evaluation: delivery-contract amendment

| Field | Value |
| --- | --- |
| Trigger | FR-056 - amending an Accepted specification-set member re-triggers Gate 0 across the manifest |
| Amended member | `docs/specs/ADR-046-validation-and-delivery.md` (Accepted) |
| Amended sections | §4, §12.1, §12.3 |
| Satisfies | FR-056 |
| Status | Mechanical half discharged; human-review half not applicable, see §4 |

## 1. What was amended

This change amends three sections of the delivery contract:

- **§4** - permits pipelined implementation start, and relocates the
  prior-wave-merged condition from wave entry to the successor's panel request
  and seal. The strict panel, seal, and merge ordering is preserved verbatim.
- **§12.1** - panel binding.
- **§12.3** - the once-per-wave binding ten-role panel and its attestation
  requirements.

The amendment is accompanied by FR-056 (this re-evaluation requirement),
FR-057 (the entry-evidence versus exit-evidence distinction), and FR-058 (the
bound on the W0 and W1 waiver's scope).

## 2. Mechanical half of Gate 0

Gate 0's mechanical half is the manifest consistency check: the three generated
manifests must be regenerated from the amended member, and the recorded content
digests plus the specification-to-work-item bijection must reconcile. The three
manifests are:

- `docs/specs/ADR-046-spec-set.json`
- `docs/specs/ADR-046-work-items.json`
- `docs/specs/ADR-046-implementation-graph.json`

`make test-drift` is the gate that enforces this. It runs `xtask spec-registry`
and `xtask implementation-graph`, regenerates all three manifests, and fails if
the committed outputs differ from the regenerated ones.

**Observed state at the time this record was written:** `make test-drift`
reports the spec-registry as consistent at 55 members and 545 work items, and
the implementation graph regenerates without a bijection error. It **fails** on
exactly one difference: the `sha256` digest recorded for
`docs/specs/ADR-046-validation-and-delivery.md` in
`docs/specs/ADR-046-spec-set.json` still names the pre-amendment content. No
other manifest entry, and no work-item or graph edge, differs.

That single digest is the expected, mechanical consequence of amending a member
without yet re-running the generator. Regenerating and committing the three
manifests discharges the mechanical half of Gate 0. That regeneration is the
one remaining mechanical action; it is not a semantic finding, and it is owned
by the change that carries the amendment itself.

This record does not claim `make test-drift` currently passes. It records the
exact failure observed, so that a later reader is not misled into believing the
digests already reconcile.

## 3. What the amendment does not change

- No specification member was added, removed, or moved between statuses.
- No work item was added, removed, retitled, or reassigned to a different wave,
  so the specification-to-work-item bijection is unaffected. The regenerated
  work-item and implementation-graph manifests are byte-identical to the
  committed ones.
- No decision in the decision register is contradicted or superseded.

## 4. Human-review half of Gate 0

Gate 0's other half is human review of the evidence a wave has already produced
against the amended contract: re-checking in-flight validation runs, panel
attestation records, and seals that were taken under the superseded text.

**That half is not applicable here, because there is no such evidence.** No wave
has sealed under this delivery contract. There is no in-flight validation
import, no panel request outstanding, no set of `panel-attest` records bound to
a candidate snapshot, and no seal that the amended §4, §12.1, or §12.3 could
retroactively invalidate. W2 has not requested a panel.

This is a statement that the input set is empty, not a statement that a review
was performed and found nothing. If a wave had sealed, this section would owe an
enumeration of its evidence and a per-item judgement on whether the amendment
disturbs it.

## 5. Standing consequence

Because the amendment lands before any wave seals, the amended §4, §12.1, and
§12.3 are the text every wave from W2 onward is delivered under. No wave record
exists that was produced under the superseded text, so the program carries no
mixed-contract evidence and no wave needs to be re-panelled.
