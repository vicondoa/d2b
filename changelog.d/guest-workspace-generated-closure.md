### Fixed

- The Guest static workspace now carries what the daemon actually compiles.
  Production includes `generated/new-graph/` directly rather than per-crate
  copies, and the Guest build assembled its source from `packages/` only, so
  the daemon and its contracts failed to compile with
  `could not read generated/new-graph/v3_converted_resource_types.rs`.
  `mkGuestRustPackagesSrc` stages the generated closure alongside the crates.
- The Guest workspace resolves template worker principals from the account
  database, so its reduced `d2b-core` manifest now carries `nix`. The
  Guest provisions its own swtpm and GPU worker accounts, and without the
  dependency every template-bound row in the Guest was refused as
  unprovisioned. `packages/Cargo.guest.lock` records the edge.