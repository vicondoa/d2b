# Realm identity lifecycle contract

**Diataxis category:** reference.

The realm identity lifecycle contract defines metadata for enrollment,
controller generations, key rotation, revocation, teardown, recovery, and
redacted identity audit events. Nix emits deterministic, non-secret identity
configuration rows in canonical realm-path order. The rows are configuration
inputs only; they do not load credentials or perform lifecycle operations.

The schema roots are generated into
[`schemas/v2/d2b-realm-core.json`](./schemas/v2/d2b-realm-core.json) and
summarized in the generated companion
[`schemas/v2/d2b-realm-core.md`](./schemas/v2/d2b-realm-core.md). Do not edit
the generated JSON by hand.

## Identity refs and fingerprints

Realm identity and controller-generation records use opaque references and
fingerprints instead of key material:

| Field family | Meaning | Secret material? |
| --- | --- | --- |
| `RealmIdentityRef` | Stable reference to a realm identity record in the owning implementation. | No. |
| `ControllerGenerationCredentialRef` | Stable reference to a controller-generation credential. | No. |
| `RealmIdentityFingerprint` | `sha256:<64 lowercase hex chars>` fingerprint of the realm identity key. | No. |
| `KeyFingerprint` | `sha256:<64 lowercase hex chars>` fingerprint of a controller or pinned key. | No. |
| `SignatureRef` | Detached signature reference or bounded signature fingerprint for route metadata. | No. |

These values are safe metadata for schemas, diagnostics, and audit
correlation. They must not be treated as private keys, public key bytes,
provider credentials, relay credentials, session secrets, signed credential
material, or endpoint authority.

`/etc/d2b/realm-identity.json` contains only enabled realms that declare at
least one such reference or fingerprint. Realm paths are encoded
most-specific-first. Child process launch records refer to this file but never
embed key material.

`RealmIdentityMetadata` records the realm, identity reference, identity
fingerprint, lifecycle status, creation time, and optional expiry. Status is a
low-cardinality enum: `active`, `rotating`, `superseded`, `revoked`, or
`recovery-only`.

## Controller generations

`ControllerGenerationMetadata` binds one controller generation to:

- the realm path;
- a `ControllerGenerationId`;
- the active `RealmIdentityMetadata`;
- a `ControllerGenerationCredentialRef`;
- a credential fingerprint;
- issue and validity timestamps;
- status and optional revocation id.

Generation metadata lets future route advertisements, resolver preflight, and
session admission reject stale or revoked controller state before accepting
operations. The current DTO does not start a controller, sign frames, rotate
credentials, or terminate sessions by itself.

## Parent trust anchors and child pins

Enrollment is parent/child trust metadata:

- `ParentTrustAnchor` records the parent realm identity reference and
  fingerprint installed into the child, plus the child controller generation
  that accepted the anchor.
- `ChildKeyPin` records the child identity reference and fingerprint accepted
  by the parent during enrollment.
- `KeyPin` is the generic edge pin shape for parent trust anchors, child
  identities, and controller-generation keys.

An `EnrollmentRecord` combines the parent realm, child realm, controller
generation, parent trust anchor, child key pin, bootstrap method token,
status/reason, timestamps, and optional correlation id. Enrollment statuses
include `pending`, `accepted`, `rejected`, `superseded`, `revoked`, and
`recovery-required`.

## Rotation

`KeyRotationPlan` is metadata for planned replacement of either a realm
identity or a controller-generation credential. The subject identifies the
current reference/fingerprint; replacement fields carry only new opaque refs
and fingerprints. Rotation reasons are stable labels such as `routine`,
`operator-requested`, `suspected-compromise`, `parent-requested`, `recovery`,
and `algorithm-migration`.

`KeyRotationEvent` records low-cardinality event/status transitions for audit
and diagnostics. It does not carry generated key bytes, signatures, provider
tokens, or process output.

## Revocation lists and teardown directives

`RevocationRecord` names the issuing realm and controller generation, the
revoked target, the low-cardinality reason, status, timestamps, and optional
correlation id. Targets may be realm keys, realm identities, controller
generations, controller credentials, enrollments, or policy grants.

`RevocationList` is a parent-pushed snapshot with one to 512 records,
propagation status for up to 64 realms, an optional superseded list id, and an
optional correlation id.

`SessionTeardownDirective` describes the sessions a future runtime must
terminate after revocation. It carries the revocation id, issuer realm,
affected realm, reason, optional affected workloads, timestamp, and optional
correlation id. It is intentionally descriptive: it does not implement
routing, relay transport, process cleanup, stream cancellation, or live
session enforcement in the current runtime.

## Recovery

`RecoveryProcedure` records recovery for a lost or compromised child
controller key. It names the parent and child realms, the affected generation,
reason/status, optional replacement generation metadata, bounded evidence
references, timestamps, and optional correlation id.

Evidence references are opaque bounded tokens. They are not log blobs,
provider payloads, credentials, signatures, paths, or secret material.
Recovery statuses are metadata labels: `requested`, `parent-approved`,
`isolating`, `reissued`, `completed`, and `rejected`.

## Future enforcement boundary

Identity lifecycle metadata is designed for future admission and routing code
to consume alongside:

- [Realm access resolver contract](./realm-access-resolver.md), which returns
  controller bindings and stale/missing controller diagnostics;
- [Realm tree routing contract](./realm-routing.md), which records strict
  parent/child route advertisements, namespace delegation, replay bounds,
  shortcut authorization, and route audit/telemetry metadata;
- [Realm controller configuration](./realm-controller-config.md), which
  reserves host-local daemon, broker, socket, state, audit, and allocator
  metadata;
- [Realm core model reference](./realm-core.md), which owns the generated
  schema roots and codec-neutral frame model.

Until runtime enforcement lands, these records do not:

- route `d2b` commands to per-realm daemons;
- authenticate relay sessions or provider controllers;
- evaluate realm policy grants;
- enforce revocation during live operation or stream admission;
- tear down current sessions or processes;
- replace existing `SO_PEERCRED` plus `d2b` group authorization on the global
  public socket.

Implementations that later consume these DTOs must fail closed on stale,
revoked, missing, or mismatched identity/controller metadata and keep audit
records bounded and redacted.

## Related references

- [Realm core model reference](./realm-core.md)
- [Generated realm-core schema companion](./schemas/v2/d2b-realm-core.md)
- [Realm access resolver contract](./realm-access-resolver.md)
- [Realm tree routing contract](./realm-routing.md)
- [Realm controller configuration](./realm-controller-config.md)
- [Realm policy](./realm-policy.md)
