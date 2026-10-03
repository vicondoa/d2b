### Added

- Volume source-side admission for the canonical `VolumeBindingRequest`
  relationship (`d2b-provider-volume-local`). One path now admits every
  consumer kind a Volume binding delivers to - `Process`,
  `EphemeralProcess`, `Host`, and `Guest` - and both presentations
  (filesystem destination and consumer device slot), reading per-kind
  eligibility from `BindingKind::Volume` rather than matching a resource
  type at each call site. `admit_consumer_request` /
  `admit_consumer_requests` build the source's own `SourceAdmission` and
  hand the decision to `admit_binding_request`, so the authorization
  grant, the declared realization support, and the freshness fence are all
  enforced rather than assumed.
- Source-owned writer arbitration and destination arbitration
  (`admit_consumer_request`). A second relationship cannot take a writer
  another live relationship holds - across consumers and across
  realization backends, since the writer belongs to the source and not to
  a view - and one consumer cannot claim one destination twice, while the
  same destination on a different consumer stays admitted. Re-admitting a
  relationship already in hand is idempotent rather than a conflict with
  itself. The durable claim itself stays with the single reservation owner
  that already arbitrates it; the source decides and keeps no second
  record.
- A declared view subdirectory is what a relationship presents
  (`AdmittedVolumeBinding::view_subdirectory`). It is the view's declared
  `ViewSpec` path, never the Volume root substituted for a subtree the view
  did not declare, and a request naming no view is refused rather than
  defaulted to the root.
- Parent support and child-default normalization targeting the correct
  consumer (`normalize_consumer_request`). A `ChildSupportCeiling` bounds
  admission and creates nothing; a `ChildRequestDefaults` fills an unset
  field of exactly the child it names and refuses every other consumer; a
  child's own declaration is never overridden.
- Release and source deletion are separate decisions
  (`decide_source_release`, `SourceReleaseObservation`,
  `SourceReleaseDecision`). Releasing the last consumer retains the shared
  source; only an explicit deletion request for a source this Provider owns
  reaches cleanup, and an externally owned source is never this Provider's
  decision to delete.
- The committed `VolumeBinding` row is the consumer's canonical request.
  `canonical_binding_row` / `binding_row_name` mint one deterministic row
  per admitted relationship, named from the KTD3 key rather than a
  declaration position, and `d2b_provider_volume::canonical_binding_children`
  turns an admitted set into child ensures whose desired bytes are the
  request itself.
- `parsed_consumer_request` reads a stored `VolumeBinding` row back as the
  canonical request, so a reader and the graph cannot disagree about which
  relationship a row declares.

### Fixed

- `VolumePresentation` serialized its block variant as `device_slot` while
  its wire decoder reads `deviceSlot`, so a committed canonical request
  carrying a block presentation could not be read back and its row declared
  no relationship at all. The container now renames its variant fields and
  repeats the casing on the `schemars` attribute, which schemars 0.8 does
  not read from the container.

### Changed

- Source-side refusals on the new path are `BindingRefusal` - a stage and a
  reason from the shared contract - instead of this crate's own error
  vocabulary. The old `VolumeLocalError` attachment path is unchanged and
  still serves the unchanged production entry point.
