### Added

- Added `d2b-provider-execution-policy`, the crate that owns the
  `ExecutionPolicy` resource type's identity: the `DriverDescriptor` the
  plane registers the type by, and the decode boundary a committed row is
  read through. The row is read as the canonical `ExecutionPolicySpec` in
  `d2b-contracts-resource`, whose wire mirror denies unknown fields, so a row
  that still carries a mount, a device-node bind, a host path, or any other
  independently granted access is refused at decode rather than decoded with
  that grant dropped. The type itself is not yet in the generated type
  authority or the standard ResourceType catalog: registration is the cutover
  unit's change, and until it lands an `ExecutionPolicy/<name>` reference
  cannot be constructed, which is the correct shape for a type no consumer may
  select yet.
- Added `d2b_provider_role::rbac::AcceptedRoleEvidence` and the graph-bound
  `contains_for_role`/`insert_allow_for_role` cache surface. A cached
  authorization decision is now bindable to the exact accepted `Role` row it
  was made from, under the contract's own framed digest, so editing one rule
  in that row invalidates the decision instead of leaving it served until its
  tick expires. An entry recorded without that evidence is never served
  through the graph-bound reader.

### Changed

- The `Operation` provider crate no longer carries its own copy of the
  operation contract. Every facet - payload and result schemas, the audit,
  authority, descriptor-carriage, and bounds facets, payload provenance, and
  the trusted `OperationImplementation` - is re-exported from
  `d2b-contracts-resource`, so a second schema that could drift from the one
  the graph publishes is gone. What stays local is the retired row a
  committed `Command` materializes, carrying `ownerRef` and an inherited wire
  discriminant; the canonical contract has no such row, and the cutover
  deletes this shape together with the foundation seed's materialization.
- The `SeccompProfile` provider crate's registration suite now pins the
  canonical contract: a committed row decodes as the contract's narrow
  syscall-filter spec, and a row carrying a namespace set, a cgroup set, a
  device-node bind, a mount, or a host path does not decode at all.
- The `Role` and `RoleBinding` provider crates' documentation now names the
  canonical contracts their rows are read as and states the admission rule a
  `RoleBinding` confers no authority of its own: a candidate binding is
  evaluated against the prior accepted graph, so it can never authorize its
  own creation.
