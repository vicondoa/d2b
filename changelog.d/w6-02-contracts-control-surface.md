### Changed

- The unpublished `StatusServicesOutputV3` type and its `from_v2` conversion
  shim are removed from the d2b-contracts-control crate. The CLI still emits
  the V2 status shape; the migration guide now promises the V3 type only at
  the emit-side flip, so tooling keeps parsing V2 until then.
- `LevelPercent` is no longer re-exported from the CLI-output module; the
  type remains available through `d2b_contracts`, the path the API docs pin.
- The helper wire types' `HelperSnapshot::validate` and
  `HelperLaunchRequest::validate_bounds` are now crate-internal; the helper
  validates workload identities through the existing
  `validate_unsafe_local_resource_identity` function, so the wire behavior
  and serde admission are unchanged.