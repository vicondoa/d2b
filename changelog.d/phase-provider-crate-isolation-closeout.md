### Changed

- Every provider family's resource model now has one declaration file per
  crate: `resource-types.json` names the crate's resource types (their
  verbs, execution classes, reads, and provides), and the Roles,
  Principals, storage roots, seccomp classes, device classes, and
  capability grants the rows the crate declares expose. The children the
  drivers mint (with the provider that owns each child) live in the
  crate's `driver.rs` declarations instead of a daemon-side table. The
  daemon's hand-built per-family tables are gone: the
  type authority, the Nix type registry and inventories, the provider
  projections, the zone and zone-link shapes, the client-layer catalogs, and
  the host user allocation rows all derive from these
  declarations. Adding a row, renaming a vocabulary word, or retiring a
  type now moves every consumer in the same change instead of leaving the
  shared crates to restate it. The generated modules under
  `nixos-modules/generated/`, `nixos-modules/host-users.nix`, and the
  contracts type authority are committed output of the generators, with
  drift gates that fail when a generated view does not match what the
  declarations produce.
- `check-provider-crate-layout` now refuses any resource driver declared in
  a shared crate, any family-knowledge token in a shared crate that is not
  an explicit ratchet row, any tokenless structural signal in a shared crate
  (a per-family branch, a hand-written Nix literal, a golden wire string,
   a type-name match arm) that the ratchet does not already carry, any
   provider crate carrying another family's identity token, and any ratchet
  row whose site has moved. The two ratchets hold the shared-crate knowledge
  the tree still legitimately carries; each remaining row names its reason or
  the blocker that refuses its move, so a reader can see why a site stays
  without reading the policy crate. A new occurrence in a shared crate
  fails the widened scan; a stale row fails too, so the inventory only
  shrinks or restates reasons. The remaining shared-crate rows are the
  documented permanent carve-outs: the broker's provider-free pin, wire
  vocabulary crossing CLI/daemon/broker boundaries, trusted-bundle and
  manifest wire shapes, golden wire-vector test strings, and the daemon's
  plane wiring that spells the provider's own typed API.

- The daemon's shared state shrank to the structural state it owns. The
  per-provider Nix tables that were hand-maintained in shared Nix modules are
  now generated from the declarations, so the literals in the hand-written
  modules are committed views of generated facts rather than second sources.
- Provider crates may carry family-knowledge tokens only in their own
  crate: the crate-content proof refuses a provider crate spelling another
  family's identity, so the in-crate knowledge the gate admits is an
  explicit, shrinking exemption list with per-row reasons.
- Committed constants that the generators do not refresh were re-verified against
  the current tree at closeout: the host-contract golden digest
  (`tests/unit/nix/cases/host-contract-digest.nix`) is unchanged at
  `sha256:f6c138da465614a39091a6eabb41d8a3b86782a85ff51e14e46e1f10b60b6cc0`,
  and the policy documents' drift surfaces (the broker operations catalog,
   the principal allocation) are green, so no row moved without its
  consumers moving with it.

### For operators

No operator-visible behavior changes. The same zones, resources, roles,
principals, storage roots, and operation envelopes are admitted, served,
and retired by the same verbs in the same order. What changed is where the
model lives: one declaration per crate, one generator aggregate, committed
generated views with drift gates, and an exemption inventory whose every row
carries the reason or blocker for its stay.