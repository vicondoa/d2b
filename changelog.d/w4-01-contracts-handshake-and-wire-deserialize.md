### Fixed

- Handshake offer derivation now converts from a borrowed endpoint policy at all seven call sites instead of cloning the policy first; the by-value conversion remains for owned policies.
- Export-target validation now rebuilds the target `ResourceRef` through a single `From<&ResourceEnvelope>` conversion instead of cloning the resource type and name at the comparison site.
- Consolidated the crate's hand-written Wire-struct `Deserialize` impls behind a shared `wire_deserialize!` macro; wire shapes, defaults, and constructor validation gates are unchanged.