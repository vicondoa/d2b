### Added

- Proposed a committed generated splice witness that preserves the privileged
  broker's standalone Cargo workspace and authoritative lock while modelling
  its exact realized path closure.
- Assigned the broker witness solely to `cargo xtask gen-bazel` and made
  `gen-bazel --check` strictly read-only. Exact authority, witness,
  declaration-ledger, lock, repository, target, and spoke comparisons retain
  independent fail-closed mutations. Target and source expectations come
  independently from Cargo manifests and locked metadata, and check mode
  preserves both repository and controlled external state.
- Split `d2b-core`, `d2b-contracts`, and `d2b-host` where their complete
  configured dependency graphs differ between broker production and tests,
  while retaining one variant for complete contexts that are equal. Default,
  layer1-bootstrap, and fake-backends broker carriers keep independent target,
  feature, edge, and case censuses.
- Corrected the inventory to four authoritative hub/workspace locks for
  `main`, `broker`, `guest`, and `walker`, with `packages/Cargo.guest.lock`
  separate and supply-chain carriers intentionally limited to three locks.
  Broker repin is not authorized or implemented by this record. An already
  built xtask process returns a no-child, no-write
  `broker-repin-architecture-pending` result pending a separate accepted
  decision and renewed plan review; Cargo bootstrap output and state are
  outside that exact-result contract. Main, guest, and walker repin behavior
  is unchanged.
