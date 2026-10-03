### Added

- The Volume driver's reconcile now produces the canonical `VolumeBinding`
  rows (U14, KTD2/KTD3). The pass that already owns the parent's reconcile
  asks the family's effect seam to admit the source's relationships,
  derives one row per admitted relationship through the existing
  `canonical_binding_children`, and commits each through the manager-routed
  `SharedProviderChildSurface`, so the row is durable before the binding's
  actor exists. The pass is idempotent (row names derive from the KTD3
  key, so an unchanged parent re-ensures the same rows and the manager
  answers `Unchanged`) and ownership-bounded on reset: a relationship the
  admitted set no longer names is retired with its owner. A relationship
  whose key does not name this row as its source is refused rather than
  committed under it, and the attachment-shaped diff leaves canonical rows
  it does not own alone - the two derivations share one resource type, and
  the row's own bytes say which derivation owns it.
- `VolumeRuntime::admit_bindings` (facet) and `VolumeDriverEffects::admit_bindings`
  (driver port): the admission evidence crosses as an admitted set, and its
  absence crosses as a named refusal (`BindingEvidenceAbsent`, listing
  `BindingAdmissionEvidence::Authorization` and `::FreshnessFence`). Neither
  the authorization nor the freshness fence is minted anywhere in this
  family: they live in the daemon's authority path, and a seam that was not
  given them refuses rather than substituting a grant. With no evidence the
  pass commits nothing and retires nothing, and says so in the driver's
  status and the log.

### Changed

- `VolumeDriverStatus::ServingChildren` carries the attachment-shaped
  `desired`/`converged` pair plus the canonical path's outcome
  (`CanonicalBindingState`), so a reader can tell a pass that committed
  relationships from one that committed none.
- `RecordingRuntime` (test support) scripts the canonical admission: a test
  either hands the seam an admitted set or withholds the evidence, and the
  driver commits accordingly.
