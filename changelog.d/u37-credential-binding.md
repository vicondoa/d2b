### Added

- A `CredentialBinding` is now realized as typed secret delivery under the
  common binding lifecycle. The source controller reads the `Credential` row's
  own committed policy - the audience the row is for, the operation classes it
  grants, the delivery Provider, and the lease-lifetime ceiling - and binds the
  consumer's request against it, so a request can no longer widen the row's
  authority by asking for more, and a spec change produces a different policy
  instead of leaving the old one in force.
- Delivery is fenced rather than merely checked. A minted delivery session
  binds the Credential generation, the consumer Provider and its component
  generation, the Provider generation, the credential rotation generation, the
  admitted dependency revisions, the audience, the method-derived operation
  class, the hard deadline, the absolute expiry, and a monotonic replay
  sequence. A changed audience, operation, component generation, or committed
  revision therefore invalidates the prior authority instead of letting a
  stale session keep working, and the service boundary refuses a session that
  is not the one the relationship admitted.
- A helper leg is an attenuation rather than a second authority. It names the
  session it rides and can never outlive the relationship, so a leg issued
  against an earlier session is stale and an expired delivery cannot be
  renewed through it.
- Revocation stays protocol-specific and is observed conservatively. A revoke
  the provider could not confirm - and a confirmed revoke with no dead-lease
  proof - leaves the relationship outstanding, so the generic lifecycle can
  never report it as released while cleanup stays withheld.

### Changed

- No credential material can reach a graph spec, a generic binding status, an
  audit record, or a publication snapshot. The delivery authority is not
  serializable and prints redacted; the status renders a closed, non-secret
  field set of the observed state, the two lifetime bounds, and the replay
  counter; and the graph spec is the consumer's own request, which carries only
  the audience, the operation classes, the lifetime bounds, the consumer, and
  the slot.
