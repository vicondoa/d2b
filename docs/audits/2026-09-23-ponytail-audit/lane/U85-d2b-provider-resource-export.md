# U85 d2b-provider-resource-export

**Blast radius:** `leaf`.

- `stdlib` None — the crate is a 38-line declaration-only metadata type crate at the post-fold floor: `driver.rs` (20 LOC) is the per-crate declaration driver kept by crate-layout policy, `lib.rs` (18 LOC) holds the crate doc + the `resource_export_descriptor` declaration builder. Zero-caller scan of the whole 38-LOC crate found no dead render sitesaire, no hand-rolled dashed-hex uids (no `{:02x}`-dash renderer present — the crate relies on the shared `ResourceUid::from_bytes` at d2b-contracts/src/identity.rs:580), and no pub surface beyond the shared-metadata fold already recorded (ledger #A1/#P1) plus the policy-required registration test. [packages/d2b-provider-resource-export/] (leaf)

**net:** -0 lines, -0 deps.

## Consistency notes
(none — resource-export is a metadata type crate, not a contracts/types crate; it carries no duplicate type definition, and its single declared descriptor is the shared `metadata_descriptor` builder.)

## Reopened refusals
- none (prior finding #P9 [refused] integration/*.rs + README scaffolds — crate-layout policy; stays refused, no new evidence)
