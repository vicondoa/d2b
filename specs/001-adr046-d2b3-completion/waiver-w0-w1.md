# Waiver: W0 and W1 delivered without sealed wave records

| Field | Value |
| --- | --- |
| Scope | Waves W0 and W1 of the ADR-046 d2b 3.0 delivery program |
| Satisfies | FR-034 |
| Bounded by | FR-035, FR-036 |
| Status | Active, one-time, non-precedential |
| Produced | Before W2 entry is declared met |

## 1. What this waiver records

Waves W0 and W1 were delivered **without** the delivery contract's sealed wave
records. This document is the explicit written waiver that FR-034 requires, and
it exists so that the gap is recorded rather than silently absorbed.

## 2. Missing artifacts, named precisely

For **each** of W0 and W1, the following delivery-contract artifacts were never
produced and do not exist:

| Wave | Missing artifact | Count |
| --- | --- | --- |
| W0 | Panel receipts - one attested record per roster role, bound to the wave's candidate snapshot | 10, all absent |
| W0 | Wave seal - the sealed wave record binding candidate id, content id, snapshot digest, and the unanimous panel result | 1, absent |
| W1 | Panel receipts - one attested record per roster role, bound to the wave's candidate snapshot | 10, all absent |
| W1 | Wave seal - the sealed wave record binding candidate id, content id, snapshot digest, and the unanimous panel result | 1, absent |

In total: twenty panel receipts and two seals are missing. No partial or
substitute panel record exists for either wave.

## 3. Evidence actually relied upon instead

In place of the sealed records, delivery of W0 and W1 rests on:

1. **All 14 assigned work items are recorded as `Merged`** in the generated
   work-item manifest. This is the complete set of W0 and W1 assignments - 8
   items in W0 and 6 items in W1 - and no assigned item is in any other state.

   | Wave | Assigned work items | Count | State |
   | --- | --- | --- | --- |
   | W0 | `ADR046-api-001`, `ADR046-api-002`, `ADR046-decisions-001`, `ADR046-feasibility-001`, `ADR046-identities-001`, `ADR046-identities-002`, `ADR046-object-001`, `ADR046-store-001` | 8 | all `Merged` |
   | W1 | `ADR046-bus-001`, `ADR046-object-002`, `ADR046-reconcile-001`, `ADR046-reconcile-002`, `ADR046-session-001`, `ADR046-session-002` | 6 | all `Merged` |

   Across the entire manifest of 545 ADR-046 work items, exactly 14 carry
   `implementationState = "Merged"`, and those 14 are precisely the W0 and W1
   assignments above. Every other item is `Planned`.

2. **The work landed through reviewed pull requests**, not by direct push or
   local merge. The integration lineage records:

   - `45aa20e5` - merge of pull request #336 (`adr046-w0prep-integrate`)
   - `aa1b188a` - merge of pull request #337 (`adr046-w0-integrate`)
   - `16218857` - pull request #338, the W1 store-backend, session, bus, and
     reconciliation landing
   - `c2b7c871` - pull request #342, the delivery plan and constitution update

   Both waves additionally carry recorded review follow-up rounds whose commits
   close named findings, so review did produce and land corrections rather than
   rubber-stamping the branches.

This evidence is **weaker** than a seal. It attests that the work merged under
human review; it does not attest a unanimous ten-role panel bound to an
immutable candidate snapshot, and it produces no content-addressed record that
a later auditor can re-verify against the exact delivered tree.

### Note on the claimed count

The specification prose states "all 14 assigned work items". The generated
manifests confirm that number exactly: 8 items in W0 plus 6 items in W1 equals
14, and all 14 are `Merged`. No discrepancy was found, so no FR-046 amendment
arises from this waiver.

## 4. Timing relative to W2 entry

This waiver is produced **before** W2 entry is declared met. Under FR-036 the
absence of W0 and W1 seals does not block W2 entry; this document is the
artifact that makes that non-blocking condition an audited decision rather than
an omission.

## 5. One-time exception, not a precedent

This waiver is a **one-time documented exception**. Under FR-035:

- Sealed delivery begins at W2.
- Every wave from W2 through W8 MUST produce a complete seal.
- This waiver **MUST NOT** be extended to any other wave.
- This waiver **MUST NOT** be reused, in whole or in part, for any other wave.
- This waiver **MUST NOT** be cited as precedent, analogy, or mitigating
  context for any wave from W2 onward.

Any future request to deliver a wave without a seal is a new specification
amendment under FR-046, not an application of this document.

## 6. Relationship to the tracked constitution deviation

This waiver is the **sole mitigation** recorded for the tracked constitution
Principle VI deviation covering W0 and W1. There is no second compensating
control, no retroactive panel, and no plan to reconstruct the missing receipts
against the delivered snapshots.

Consequently: **without this document the deviation is undocumented in
practice.** Removing, weakening, or superseding this waiver without replacing
its record leaves the program with an unrecorded departure from Principle VI
for the first two waves of the delivery.
