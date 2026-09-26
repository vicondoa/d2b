### Changed

- The unpublished `StatusServicesOutputV3` type and its `from_v2` conversion
  shim are removed from the d2b-contracts-control crate. The CLI still emits
  the V2 status shape; the migration guide now describes the V3 shape as its
  rename map applied to V2 rather than naming a type, so tooling keeps parsing
  V2 until the emit-side flip.
- `LevelPercent` is no longer re-exported from the CLI-output module; the
  type remains available through `d2b_contracts`, the path the API docs pin.
- The helper wire types' `HelperSnapshot::validate` and
  `HelperLaunchRequest::validate_bounds` are now crate-internal; the helper
  validates workload identities through the existing
  `validate_unsafe_local_resource_identity` function, so the wire behavior
  and serde admission are unchanged.