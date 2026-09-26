### Fixed

- The committed cloud hypervisor Provider artifact ships a signature that
  verifies again. `packages/d2b-provider-guest-cloud-hypervisor/provider-manifest.json.sig`
  was signed over an earlier manifest, so the commit that moved
  `root-config.schema.json` and updated the manifest `configDigest` without
  re-signing left the compiler refusing the artifact with
  `provider-signature-verification-failed`. The publisher keypair is rotated:
  `publisher-public-key.pem` now carries the new SPKI public key and
  `provider-manifest.json.sig` is a fresh raw 64-byte Ed25519 signature over
  the current manifest bytes. The manifest itself is byte-for-byte unchanged.
  The previous private key is not recoverable, so the publisher key changes
  rather than being re-derived; the operator holds the new private half
  outside the repository. The new public key is the committed
  `packages/d2b-provider-guest-cloud-hypervisor/publisher-public-key.pem`,
  whose `sha256` over the DER `SubjectPublicKeyInfo` is
  `d798fa9f94f64015d5711c0484b5b30cef1b6c9bb918cb46bf43338e0d817b57`.
  Consumers that pinned the old publisher key must trust the new public key
  before building a Zone that installs this artifact.

### Added

- A hermetic test in `packages/d2b-resource-compiler` loads the committed
  manifest, signature sidecar, and publisher key and asserts the signature
  verifies over the manifest, with a probe for each half moving alone. It runs
  in the Layer-1 `make check` aggregate, so a future manifest or schema change
  that forgets to re-sign fails the gate instead of shipping. The Nix build
  only asserted that the sidecar was 64 bytes long, so nothing caught the
  staleness before.
