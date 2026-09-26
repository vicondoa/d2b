### Fixed

- Mirrored `d2b-resource-client` into the copied Guest workspace (`flake.nix`,
  `tests/fixtures/guest-rust-workspace/Cargo.toml`, and
  `packages/Cargo.guest.lock`) after
  `d2b-provider-guest-cloud-hypervisor` gained a dependency on it; the
  realized supply-chain lane resolves that workspace and failed on the absent
  manifest, so the crate directory, its membership, and its lock entry are
  restored.
- Regenerated the package policy inputs so the `guest-static` contexts carry
  the refreshed `packages/Cargo.guest.lock` digest.
