### Added

- The `Device` driver's reconcile now produces the `DeviceBinding` rows its
  source owns (U16, KTD2/KTD3). The pass runs inside the verb that already
  reconciles the parent row, resolves the realizing component through the
  family's own `component_for_provider` from the row's own `providerRef`,
  admits the declared relationships through the existing
  `admit_device_requests`, derives one row per admitted relationship through
  the existing `canonical_binding_rows`, and commits each through the
  manager-routed `SharedProviderChildSurface`, so the row is durable before
  the binding's actor exists. The pass is idempotent (row names derive from
  the KTD3 key, so an unchanged parent re-ensures the same rows and the
  manager answers `Unchanged`) and ownership-bounded on reset: the rows this
  source owns that the pass no longer derives are retired, and the
  Zone-declared worker rows and effect-created rows the shared child diff
  declines to touch are left alone.
- `DeviceBindingEffects::declared_bindings` (driver port): the declared
  consumer requests cross with the admission evidence the source's admission
  is fenced against. Neither fact is minted in this family - the
  `BindingAuthorization` is the graph authority's verdict on the
  relationship's own mutation, and the freshness fence's desired revision and
  digest are the authority journal's - so the default answer declares
  nothing. A pass without that evidence commits nothing new, retires nothing
  on its account, and reports which fact was missing
  (`BindingProductionRefusal::AuthorizationEvidenceAbsent`,
  `::FreshnessFenceAbsent`, `::SourceSpecUndecodable`) in the driver's log.
- `BindingProductionRefusal` reports `AdmissionRefused` with the shared
  contract's own closed stage and reason when a declared request is refused,
  so a batch refusal is named rather than silently producing an empty
  desired set.
- `FixedInventory` (test support) serves one explicitly built inventory, so a
  test can observe a capability the host no longer backs without restating
  the host device-node matrix.

### Changed

- A device whose trusted inventory no longer backs the capability a committed
  row names, or whose declared Provider vocabulary no longer carries that
  name, now has that binding row retired by the source that owns it. A
  capability that is still backed is kept, because a pass that could not
  evaluate the admission is not a withdrawal.
- The `DeviceBinding` serving driver and the producing pass read the parent
  row's typed spec through one shared decode, so the two halves cannot
  disagree about what the row declares.
