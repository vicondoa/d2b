### Added

- The `DeviceBinding` serving driver now DECIDES presence instead of assuming
  it. A reconcile pass reads the trusted inventory through the family's declared
  inventory facet, re-admits the committed relationship through the family's
  own `admit_device_request`, and hands the relationship and the freshly
  observed inventory to the family's own `decide_presence`; what that decision
  returns is what the driver publishes. Retained reports `Admitted` with the
  opaque `DeviceAuthorityKey` the trusted inventory resolved, a relationship
  whose effect cannot be proven reports the new
  `UnattachedReason::PresenceUnproven`, and a capability the host no longer
  backs still reports `UnattachedReason::CapabilityNotBacked`.
- `DeviceBindingDriverStatus::Replaced`: a device replaced behind the same
  capability name resolves to a different physical authority than the pass that
  last delivered the row. The replacement is reported by name and re-checked
  rather than carried forward, so a consumer is never handed a grant for
  hardware it was not admitted against.
- `DeviceBindingAuthoritySource` (declared facet) and `DeviceBindingEvidence`:
  the graph-authority evidence one committed row's presence is decided from -
  the canonical request the authority admitted, the `BindingAuthorization`
  that admitted it, the dependency fence it was fenced against, and the
  `BindingLifecycleState` observed for the relationship now. `admit_device_request`
  refuses without the first three and `decide_presence` reads the fourth, so
  the serving half re-admits rather than admitting itself. Evidence about a
  different source, consumer, slot, capability, or claim is refused for the
  row it is served against.
- `RecordedAuthority` and `device_facets` (test support): a recording authority
  facet over one explicitly observed lifecycle, and a facet-set builder over a
  test's own inventory and authority observations.

### Changed

- The `DeviceBinding` driver resolves the parent `Device` row's and the
  consumer row's store-assigned identities alongside the committed spec, so a
  serving pass derives the same KTD3 key the source admitted and a device or
  consumer replaced under the same name yields a different relationship rather
  than the old one continued.
- An inventory a serving pass cannot read, an authority journal that cannot
  answer, and a lifecycle that proves no effective result are all reported as
  degraded rather than revoked: uncertainty is never read as either granted use
  or a completed release.
- `DeviceBindingEffects::binding_evidence` defaults to reporting the evidence
  as unavailable. A seam with no authority journal behind it holds that refusal,
  so every relationship reports degraded instead of the family granting itself
  device authority.