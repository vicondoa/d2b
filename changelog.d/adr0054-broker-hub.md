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
- Split only `d2b-core` and `d2b-host` into broker production and broker-test
  library variants, keeping test-only features out of production and retaining
  one broker variant for shared crates whose measured contexts are equal.
- Corrected the inventory to four authoritative hub/workspace locks for
  `main`, `broker`, `guest`, and `walker`, with `packages/Cargo.guest.lock`
  separate and supply-chain carriers intentionally limited to three locks.
  Broker repin is not authorized or implemented by this record and remains
  a no-child, no-write `broker-repin-architecture-pending` refusal pending a
  separate accepted decision and renewed plan review. Main, guest, and walker
  repin behavior is unchanged.
