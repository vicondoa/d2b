# `realm-identity.json` schema (`v2`)

Schema: [`realm-identity.json`](./realm-identity.json)

`realm-identity.json` is private metadata-only realm identity configuration.
It records refs and fingerprints derived from `d2b.realms.<realm>.keys` so
the daemon and broker can strictly validate declared identity metadata without
loading secrets or changing runtime trust behavior.

## Top-level fields

- `schemaVersion` — schema version for this artifact.
- `runtimeState` — closed runtime state enum. The value in this scope remains
  `metadata-only`.
- `realms` — enabled realm rows that declare at least one identity/key ref or
  fingerprint.
- `invariants` — booleans asserting the artifact is metadata-only, contains no
  secret material, and preserves current runtime behavior.

## Realm fields

- `realm` — most-specific-first realm path labels.
- `realmIdentityRef` and `realmIdentityFingerprint` — optional opaque locator
  and SHA-256 fingerprint for the realm identity key.
- `controllerCredentialRef` and `controllerCredentialFingerprint` — optional
  opaque locator and SHA-256 fingerprint for the controller-generation
  credential.
- `trustBundleRef`, `enrollmentRef`, and `rotationPolicyRef` — optional
  bounded non-secret metadata refs.

## Contract notes

- No private keys, public key bytes, signatures, provider tokens, relay
  credentials, or session secrets are represented.
- Loading this artifact is validation/logging only; live trust sessions,
  authentication, routing, and enforcement remain unchanged.
