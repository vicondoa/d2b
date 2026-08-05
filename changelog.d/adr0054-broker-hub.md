### Added

- Proposed a committed generated splice witness that preserves the privileged
  broker's standalone Cargo workspace and authoritative lock while modelling
  its exact realized path closure.
- Assigned the broker witness solely to `cargo xtask gen-bazel` and made
  `gen-bazel --check` strictly read-only. Exact authority, witness,
  declaration-ledger, lock, repository, target, and spoke comparisons retain
  independent fail-closed mutations.
- Corrected the inventory to four authoritative hub/workspace locks for
  `main`, `broker`, `guest`, and `walker`, with `packages/Cargo.guest.lock`
  separate and supply-chain carriers intentionally limited to three locks.
  Broker repin is not authorized or implemented by this record and remains
  blocked by `broker-repin-architecture-pending` pending a separate accepted
  decision. Main, guest, and walker repin behavior is unchanged.
