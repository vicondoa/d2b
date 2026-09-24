# U69 d2b-provider-telemetry-binding

Lean already. Ship.

Audited the whole crate: `src/driver.rs` (1,242 lines), `src/lib.rs` exports, `tests/registration.rs`, `tests/registration.rs` BUILD wiring, and the descriptor/verbs/reads/creations/execution domain constants region (driver.rs:499-671). No new dead machinery. Checks run:

- **No zero-caller public surface beyond what U1 already records.** Every pub item in lib.rs/driver.rs either serves the registration boundary or has a production caller:
  - `telemetry_binding_descriptor()` consumed by the plane at `packages/d2bd/src/resource_plane_v3.rs:2793` (workspace-wide caller search confirms the only live production consumer; no other crate imports it).
  - `TELEMETRY_BINDING_TYPE`, `TELEMETRY_BINDING_TYPE`-scoped verbs, `TELEMETRY_BINDING_COLLECTOR_CREATION`, `TELEMETRY_BINDING_ENDPOINT_CREATION`, `telemetry_binding_spec_decoder`, `telemetry_binding_descriptor`, and the driver descriptor id are the registration contract (registration.rs asserts the type, decoder, factory, and the collector/endpoint pair).
  - `TELEMETRY_BINDING_TYPE` and `TELEMETRY_PROVIDER_REF` are read by the plane's provider directory/descriptor (external callers list: d2bd only).
- **No hand-rolled stdlib/native helpers.** No UUID/hex/UUIDv4 renderer, no base64 codec, no JSON-Schema validator, no UUID renderer in this crate (the U1 cross-cutting #S1/#S2/#P7 findings name only three in-scope call sites - none in this crate; TELEMETRY_BINDING_VERBS was reported against the family, not against this crate's lane).
- **No test-only live code in production paths** beyond the driver's own `cfg(test)` helpers; the driver tests use a recording manager fake, not production surface.
- **`src/generated/`** absent in this crate. d2bd-runtime test-support dep is a real consumer (BUILD.bazel); no test-support surface is dead.
- **Consistency note (family):** `TELEMETRY_BINDING_COLLECTOR_CREATION.order < TELEMETRY_BINDING_ENDPOINT_CREATION.order` is enforced by a compile-time const assert (driver.rs tests) and the descriptor's `creations` field carries both; the §Context table for U69-#S2 already reports the un-migrated 9-verb list - no new evidence here.

## Consistency notes

None. This is a provider-lane leaf row; no duplicate type definitions, no wire-shape skew, no hand-rolled copies of shared constructors beyond what the U1 refusal ledger already refuses (TELEMETRY_BINDING_VERBS family list, refused as cross-cutting per U1-(a) call, and the OTel copy).

## Reopened refusals

None. U69 has no prior refused findings; the two prior rows (#PR10 partial, #S2 applied) are honored as recorded (nothing to re-audit). The crate's only reported item - the cross-cutting 9-verb family list - is explicitly refused under U1 (#S2) and carries no new evidence here.

## Checked

Read `src/driver.rs` (production + tests, all 1,242 lines), `src/lib.rs`, `tests/registration.rs`, `BUILD.bazel`, `Cargo.toml`, crate README. Grep-based caller verification across the workspace (packages, d2bd, BUILD files) for every pub export. Identified zero new dead code; the crate's only filed finding (the telemetry-binding 9-verb family list in the converted-type verbs) was already reported as cross-cutting and refused under U1#S2, so no new ledget entry required. No findings.
