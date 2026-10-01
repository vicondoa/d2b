---
type: changed
area: contracts,providers,daemon
---

### Changed

- **Removed the standalone `Command` resource authority.** A declared launch
  shape is no longer a first-class resource type. `d2b-provider-command` is
  deleted along with its workspace membership, generated declarations, frozen
  type-authority entries, and generated schema. An `Operation` row's executable
  declaration is provider-owned contract data resolved through
  `OperationImplementation`, which names a declared `Provider` method or a
  trusted executable template; the graph no longer materializes an operation
  row from a command row.
- **Reduced the `Role` contract to the authorization-only shape.** `RoleSpec`,
  `RolePosture`, `RoleNamespaces`, `RoleMount`, `RoleMountPath`, and the
  role-local `PrincipalRef` are removed. `AuthorizedRole` now carries the
  whole contract: bounded rules plus the `Operation` rows the holder may
  create. The wire mirror denies `posture`, `commandRefs`, and `mounts`, so a
  row that still carries one is rejected rather than decoded with its
  authority quietly dropped.
- **Removed the pre-cutover materialized-operation row.** `OperationSpec` and
  `MaterializedOperationError` in `d2b-provider-operation` are deleted; that
  module is now a pure re-export of the canonical `CallableOperation` facets,
  so a drift between the provider crate and the contract the graph publishes
  is not expressible.