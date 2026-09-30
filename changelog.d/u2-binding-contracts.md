### Added

- Typed binding contracts for all five primitive relationship families:
  `d2b-contracts-resource` gains `VolumeBindingRequest`, `DeviceBindingRequest`,
  `NetworkBindingRequest`, `EndpointBindingRequest`, and
  `CredentialBindingRequest`. Each is a closed desired schema naming an exact
  source reference, an exact consumer, a stable consumer slot, the
  kind-specific rights, and the consumer-side presentation. None of them can
  carry a raw host source path, a numerical host principal, secret material, or
  a free-form command line, and every wire mirror denies unknown fields.
- Shared binding machinery in `v3::binding`: the `BindingKind` and
  `BindingConsumerKind` vocabularies, the stable `BindingSlot`, the
  `RequestedRights` vocabulary with its per-family eligibility, the observed
  `BindingLifecycleState`, the KTD3 `BindingKey`, the consumer `BindingSlotIndex`
  with coalesce/conflict/replace semantics, and the `admit_binding_request`
  evaluator.
- Admitted binding evidence (`BindingAdmission`, `BindingEvidence`,
  `SourceReservation`, `BindingObservation`) is minted only by the evaluator and
  only from an authorization grant, the source provider's own scoped decision,
  the selected realization's declared support, and the exact dependency
  revisions it is fenced against. A desired request alone cannot reach it, and
  an ownership, view, consumer, provider-assignment, or policy change
  invalidates earlier authority even when the spec generation is unchanged.
- Execution-parent input classification for `Host` and `Guest`:
  `ExecutionParentInput` separates a child target-support ceiling, a parent's
  own consumption, and defaults for one named child's request, so the three
  meanings of the old overloaded attachment list no longer collapse into one.