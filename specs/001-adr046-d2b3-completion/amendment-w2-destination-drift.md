# Amendment request: W2 destination drift for `ADR046-process-001`

| Field | Value |
| --- | --- |
| Scope | Wave assignment of `packages/d2b-process/` and `packages/d2b-provider-supervisor/`; widened in section 6 to the W3 and W5 rows of the same table |
| Raised under | FR-046 |
| Affected member spec | `ADR-046-validation-and-delivery`, section 3.2 |
| Affected manifests | `ADR-046-implementation-graph.json`, `ADR-046-work-items.json` |
| Status | Raised to the integrator; awaiting a separate specification amendment |
| Drift still present | Yes, as of this document |

## 1. The drift, with both sides quoted

### Prose side - `docs/specs/ADR-046-validation-and-delivery.md`, section 3.2

The wave destination table lists the two crates under **W2**:

> `ADR046-W2` | `ADR-046-primitive-resource-composition` / `ADR-046-zone-routing` |
> `packages/d2b-contracts/src/v3/{host,guest,execution_policy,process,volume,user,network,device,credential}.rs`;
> **`packages/d2b-process/`; `packages/d2b-provider-supervisor/`**; `packages/d2b-zone-routing/`

The same table also lists both crates again under **W4**:

> `ADR046-W4` | ... | **`packages/d2b-process/`, `d2b-provider-supervisor/` (process effect ports)**;
> `packages/d2b-core-controller/`; ...

### Manifest side - `docs/specs/ADR-046-implementation-graph.json`

The owning work item is assigned to **W4**:

```json
{
  "id": "ADR046-process-001",
  "wave": "W4",
  "destinations": [
    "`packages/d2b-process/src/`, `packages/d2b-provider-supervisor/src/`"
  ]
}
```

### Manifest side - `docs/specs/ADR-046-work-items.json`

The same item's destination agrees with the graph; the manifest carries no
independent wave field for it:

```json
{
  "workItemId": "ADR046-process-001",
  "destination": "`packages/d2b-process/src/`, `packages/d2b-provider-supervisor/src/`",
  "wave": null
}
```

The conflict is therefore narrow and specific: the prose schedules
`packages/d2b-process/` and `packages/d2b-provider-supervisor/` for **W2**,
while the implementation graph assigns their only owning work item,
`ADR046-process-001`, to **W4**.

## 2. Resolution under FR-046

FR-046 states that where the specification set's prose and the generated
implementation graph or work-item manifest disagree on wave assignment,
destination paths, or work-item identity, **the generated manifests are
authoritative**.

Accordingly:

- The implementation graph's **W4** assignment for `ADR046-process-001`
  **governs**.
- Implementers **follow the graph**. `packages/d2b-process/` and
  `packages/d2b-provider-supervisor/` are **not** W2 destinations. No W2 work
  item creates, owns, or is scoped to those crates.
- A W2 candidate snapshot that adds either crate is out of scope for W2 and
  should be rejected at review rather than reconciled against the prose.
- The section 3.2 W2 row is treated as stale prose for those two entries only.
  The rest of that row - the `d2b-contracts` v3 resource modules and
  `packages/d2b-zone-routing/` - is unaffected by this drift and remains W2.

## 3. Why the prose is not corrected here

Per FR-046, this drift **MUST NOT be silently corrected inside an
implementation wave**. Editing `ADR-046-validation-and-delivery.md` to remove
the two crates from its W2 row would amend a member specification, and amending
a member spec **re-opens that spec's validation and panel evidence**: the
snapshot the spec was validated and sealed against would no longer be the
snapshot in the tree, and its panel receipts would no longer bind the delivered
content.

The cost of that re-opening is not proportionate to a stale table cell, and
paying it mid-wave would put an implementation wave in the position of
invalidating governance evidence it does not own. The correct handling is to
record the drift, follow the authoritative manifest, and let a dedicated
amendment carry the prose change with its own validation and panel round.

## 4. Record and disposition

- The drift is **recorded here** and **raised to the integrator**.
- It is to be resolved by a **separate specification amendment** against
  `ADR-046-validation-and-delivery`, scheduled outside any implementation wave,
  which will re-run that spec's validation and panel evidence.
- Until that amendment lands, this document is the standing instruction: the
  graph's W4 assignment governs, and implementers follow the graph.

## 5. Status of the drift as described

The drift **still exists exactly as described**. Verification against the
current tree found:

- section 3.2 still lists `packages/d2b-process/` and
  `packages/d2b-provider-supervisor/` in its W2 destination cell;
- the implementation graph still assigns `ADR046-process-001` to `W4`;
- the work-item manifest's destination for that item still matches the graph.

One detail worth recording beyond the original statement: section 3.2 lists the
two crates under W4 **as well**, qualified as "process effect ports". The row is
therefore internally duplicative, not merely mismatched against the graph. The
amendment should resolve the duplication in the same change rather than only
deleting the W2 mention.

## 6. Scope widened: section 3.2 carries a family of these drifts

This document was opened for one row. The W3 wave found a second instance of
exactly the same shape in the same table, recorded as the "W3 destination set
disagrees with the section 3.2 wave table" bullet in section 5 of
[`implementation-debt.md`](./implementation-debt.md):

- section 3.2 gives W3's destinations as only `packages/d2b-provider/`,
  `packages/d2b-provider-toolkit/` and a
  `packages/d2b-provider-<base>-<implementation>/` skeleton generator, naming
  neither `packages/d2b-contracts/src/v3/provider.rs` (destination of
  `ADR046-provider-001`), nor `packages/d2b-contracts/src/v3/semantic_services/`
  (destination of `ADR046-provider-004`), nor
  `packages/d2b-provider-system-core/` (destination of `ADR046-provider-003`);
- the same table lists `packages/d2b-provider-system-{core,systemd,minijail}/`
  in its **W5** row, while `ADR046-provider-003` is a **W3** item in the
  implementation graph.

The resolution is identical to section 2: FR-046 makes the generated manifests
authoritative, so implementers follow the graph and the section 3.2 W3 and W5
rows are stale prose for those entries only.

**This amendment absorbs that drift**, rather than a second amendment being
opened for it. Both instances are the same table, the same member
specification, and the same single re-opening of that spec's validation and
panel evidence; splitting them would pay that cost twice for one edit. Any
further section 3.2 wave-table drift found by a later wave should be appended
here on the same reasoning.

Note the boundary: this batching covers drifts **in the section 3.2 wave
table**. It does not extend to frozen-contract amendments against other member
specifications, which carry their own evidence and are filed separately - see
[`amendment-frozen-cross-zone-contracts.md`](./amendment-frozen-cross-zone-contracts.md).
