### Fixed

- Manager-backed rows render a complete, strictly decodable envelope again:
  spec-shaped rows now carry `metadata.managedBy` and
  `metadata.configurationGeneration`, a complete top-level status (contract
  shape with `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`,
  `outcome`, `update` and `resource`, `Pending` at the row's own generation
  when nothing was published), the contract's `Deleted` phase for a deleting
  row, and a payload digest resealed after any status or deletion stamp.
  Before the fix every manager row was refused by the strict envelope reader
  ("unpublished status: strict decode failed"), and deleting rows rendered a
  phase the contract does not contain.
- The Cloud Hypervisor setup-Volume read is manager-first: a row the manager
  holds resolves through the plane bridge, an absent row is retryable
  (`Pending`), and a manager/store read failure stays an error - never
  absence. The Guest's controller-finalizer custody gate reports the
  provider controller's finalizer present for a manager-served row, so the
  CH controller's child batch commits instead of the reconcile returning at
  the custody gate.
- The store-view farm path is traversable by the daemon: the anchored walk
  opens intermediate path components `O_PATH|O_DIRECTORY` (leaf read-only),
  and the broker postures the broker-created ancestor chain traversal-only
  for the daemon's group (search only, never read/write), so the generated
  `store-view-<guest>` Volume resolves instead of failing `EACCES`.
