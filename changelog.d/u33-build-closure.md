### Added

- The new graph's build and test composition is generated from the four
  authoritative per-crate provider declarations and written into a
  caller-owned isolated directory, so the complete production replacement is
  staged without changing what production generates today. The composition
  reads none of the authorities the new graph retires: the merged
  broker-operation rows document, the principal allocation, the handwritten
  privilege copy, or the in-generator privilege and family-scope tables. Each
  staged artifact records the committed production path it replaces and the
  declaration that produced it, so the cutover is a copy rather than a
  re-derivation.
- An isolated Nix test surface projects the canonical graph policy from a
  declaration projection and refuses the retired knobs - a privilege table, a
  role-scope map, a family-scope table, a broker wire-variant list, a
  principal allocation - rather than reading them as if they were absent. It
  imports no production option module and declares no NixOS option, so the new
  graph's Nix half is proven with no change to the public production options.

### Fixed

- `packages/d2bd/tests/zone_provider_acceptance.rs` is now its own Bazel test
  target with its own crate root. It was listed as a source of the
  `resource_operator_activation` target, which has a single crate root, so the
  file was never type-checked by the gate: a break in it reached `make check`
  green while `cargo test` failed. The one stale call site it carried is
  repaired, and the activation target's dependency list now names only what
  that file uses.
