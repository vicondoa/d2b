### Fixed

- Mirrored `d2b-resource-compiler` into the copied Guest workspace
  (`flake.nix`, `tests/fixtures/guest-rust-workspace/Cargo.toml`, and
  `packages/Cargo.guest.lock`) after the daemon took it on as a dev
  dependency; the realized supply-chain lane resolves that workspace and
  failed on the absent manifest, so the crate directory, its membership, and
  its lock entry are restored.
- Regenerated the package policy inputs so the `guest-static` contexts carry
  the refreshed `packages/Cargo.guest.lock` digest.