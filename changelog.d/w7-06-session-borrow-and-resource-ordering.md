### Changed

- `HandshakeOffer` now converts from an endpoint policy by borrow only:
  the by-value `From<EndpointPolicy>` implementation is gone, so every
  caller passes a reference and no admission path clones a policy purely
  to build a comparison offer (`component_session.rs`).
- Guest target assignment and instance listings sort resource identities
  with an owned cached key over the zone, type name, and name fields,
  replacing a comparator that could not be expressed as a key
  (`target.rs`, `guest_target.rs`).
