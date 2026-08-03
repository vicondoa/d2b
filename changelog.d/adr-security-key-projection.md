### Added

- Added ADR 0051, a documentation-only decision record fixing the semantic
  backing contract for the provider-neutral `SecurityKeyService` and
  `SecurityKeyBinding` pair. The security-key family's closed
  `allowedBackingRefTypes` set is empty, because its provider-neutral base
  names no backing resource and the physical device belongs to the
  implementation extension; an empty allowlist denies every backing reference
  and is never read as unconstrained. The record replaces the backing
  declaration with a two-state value, requires export and backing admission to
  read the stored resource so a resource held on lease from another Zone can
  neither be re-exported nor become the backing of a new local authority
  claim, promotes the projection-protocol version to a declared descriptor
  field so a Provider artifact built for a different version is reported as
  version skew with an install-a-matching-artifact remedy rather than as a
  fingerprint mismatch, and requires every declared factory field to be
  published in the generated projection schemas. No crates, services,
  controllers, or Providers are created, and no specification or reference
  document changes.
