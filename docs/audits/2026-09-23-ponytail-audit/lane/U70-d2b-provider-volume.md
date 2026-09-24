# U70 d2b-provider-volume

Lean already. Ship.

Checked: the whole of `src/` (driver.rs 1404 lines, effects_service.rs 346, facets.rs 74,
lib.rs 48, test_support.rs 120) plus `tests/registration.rs` and `integration/volume.rs`,
and worker-side caller verification for every public export against the daemon-plane
composition roots:

- `VOLUME_EFFECTS_SERVICE` / `VOLUME_EFFECTS_SERVICE.id` -> resolved at startup by name and
  erased at the composition boundary (`packages/d2bd/src/provider_lifecycle.rs:103`,
  `packages/d2bd/src/shared_provider_effects.rs:3449`, and resource_plane_v3.rs:2280 +
  the `the_volume_effects_service_answers_has_layout_through_the_binding` plane test at
  4452). The `has-layout` service is a declared zone-plane declaration, hosted per zone by
  the daemon - the same recover probe the driver reads (U7). Not dead.
- `VOLUME_CREATIONS` / `volume_descriptor` / `VolumeDriverArgs` / spec decoder / factory ->
  registered types, decoder hooks, and the `VolumeBinding` child creation all consumed at
  the plane registration (`packages/d2bd/src/resource_plane_v3.rs:721,2280,2983,4460` and
  this crate's own tests/registration.rs). Deterministic child derivation + custody of the
  one derived binding are exercised live in driver tests (F1: ensure-before-spawn order).
- Every verb/const surface (`VOLUME_TYPE_NAME`, `VOLUME_CREATIONS`, `VOLUME_READS`,
  `VOLUME_EXECUTION_DOMAINS`, `VOLUME_RESYNC`, `VOLUME_CREATIONS[0].provider_ref`) is
  declaration surface the kind serves by descriptor - required declaration, not speculative.
- No hand-rolled UUID rendering remains in this crate: uid work rides
  `ResourceUid::from_bytes` / `ResourceRef::new` / `volume_ref` (issue #508 three call
  sites already migrated per #P7). The `resource_uid` test-support seam and scripted
  runtime doubles are cfg(test)/test-support-gated seams with live consumers; nothing
  public is write-only or caller-less.
- Both `has-layout` wire-answer literals (`{\"hasLayout\":true|false}`) and both
  `ensure-layout` idempotent layout effect arms are asserted byte-for-byte in tests - the
  two canonical payloads are pinned, and the recover/adoption pass they serve is
  revalidated (R10/R11).

Net: 0 lines, 0 deps.

## Consistency notes

None: no type-layer duplicate or wire-shape skew introduced here. The Volume family was
already the convergence target of the shared-driver and shared-effects-service work (U7,
U70), so this crate's driver.rs and effects service are single implementations - no
external port, no daemon state type, no second copy to reconcile against.

## Checked

Read `driver.rs` (declaration, factory, validate/recover/reconcile/finalize/delete verbs,
desired-child derivation with providerRef push and deterministic child keying, and the full
driver test suite incl. degraded-layout retry cadence and adoption-after-restart), the
effects effects_service surface (`VOLUME_EFFECTS_SERVICE` service declaration, `has-layout`
payload contract with its own refusal codes), the facets/`VolumeRuntime` trait port, lib.rs
re-export block, test_support.rs scripted and recording doubles, and tests/registration.rs.
Ran workspace-wide caller searches on `VOLUME_*` exports through `packages/` and `d2bd/`
and verified each lands at a composition-root registration or a plane test that asserts the
declaration (U7/R2/R3). No dead code, no hand-rolled stdlib equivalent, no unconsumed
flexibility found. Prior-ledger items #P7/#S1/#S2 already applied or refused with no
changed circumstance to reopen.
