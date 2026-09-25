# d2b-provider-operation - unit-test audit
tests: 6 · src files: 3
net: -0 tests, -0 lines

## Findings (biggest net first)
Nothing to cut. Ship. All 6 unit tests are inside src/operation.rs's contract-validation test surface and bind the machine-readable Operation wire contract; each pins a distinct invariant or error path, and none is duplicated. (The crate's only integration test tests/registration.rs pins the driver-descriptor registration - a separate surface.)
- gap: over-limit audit facets rejected (`OperationAudit::new`, src/operation.rs:148) - TooManyAuditFields is a real bound with no test; the audit-facet field cap (MAX_OPERATION_AUDIT_FIELDS / MAX_OPERATION_REDACTION_KEYS) is otherwise unenforced-by-test.
- gap: over-limit fd contract lists rejected (`OperationFds::new`, src/operation.rs:356) - TooManyFds bound untested despite MAX_OPERATION_FDS = 16 being a load contract.
- gap: ownerRef naming a non-Command resource rejected (`OperationSpec::new`, src/operation.rs:490) - InvalidOwnerRef path untested; only the Command/ valid name is exercised.

## Keep
- `a_materialized_operation_reuses_the_command_payload_contract` (src/operation.rs:732) - pins materialized-op positive path: ownerRef (Command type) accepted and stored, no wireTag, audit join accepted.
- `a_materialized_operation_never_carries_an_inherited_wire_tag` (src/operation.rs:742) - pins InheritedWireTagOnMaterialized: ownerRef + wireTag rejected.
- `secret_bearing_payloads_require_a_secret_access_ceiling` (src/operation.rs:761) - pins SecretAccessBelowPayload: write-only payload field with SecretAccess::None rejected.
- `audit_join_refuses_undeclared_or_secret_fields` (src/operation.rs:769) - pins InvalidAuditJoin for both an undeclared and a write-only join field.
- `zero_or_over_ceiling_bounds_are_refused` (src/operation.rs:791) - pins InvalidBounds for zero and over-ceiling payload/batch/stream limits.
- `the_wire_shape_round_trips_and_is_closed` (src/operation.rs:799) - pins canonical serde roundtrip equality, kebab-case enum rendering, and wireTag absence on the wire.

cross-check: d2bd/src/foundation_seed.rs (materialized operation row deserialization asserting owner_ref) overlaps this crate's keep-1 roundtrip ownership - see C2.
cross-check: xtask/src/zone_schema.rs golden (Operation resource schema) overlaps this crate's keep-6 wire-shape pin - see C2.
