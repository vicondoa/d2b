# ADR 0046 Credential ResourceType

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resources-credential` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-credential-secret-service`, `d2b-provider-credential-entra`, `d2b-provider-credential-managed-identity`, Credential controller, Nix Credential compiler |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-primitive-resource-composition`, `ADR-046-componentsession-and-bus` |
| Supersedes | Current v3 `CredentialProvider` trait, `CredentialStatus`, `CredentialPlane` model in `d2b-realm-provider/src/credential.rs` and `provider.rs` |

## Purpose

This spec defines the `Credential` ResourceType, its opaque lease/typed
operation model, its placement and scope constraints, its zero-secret-bytes
invariant for all persistent and observable surfaces (resource store, status,
audit, OTEL, logs), its end-to-end sensitive output delivery contract via dedicated
Noise ComponentSession, the three initial Credential Provider dossiers
(`credential-secret-service`, `credential-entra`,
`credential-managed-identity`), and every work item required to take these from
the v3 baseline to a running d2b 3.0 controller.

## Core design principle: persistent and observable surfaces are zero-secret

`spec`, `status`, the resource store, revision/WAL, bus routing DTOs, error
messages, audit records, OTEL spans, metrics, and all log lines **never
contain secret material** in any field or byte. This invariant is unconditional
for those surfaces:

- `spec` contains only non-secret declarative fields (providerRef, ownerRef,
  scope/audience selectors, rotation/expiry policy, and allowed operation
  classes).
- `status` contains only opaque lease identifiers, generation counters, expiry
  timestamps, and phase/condition/outcome values. It never contains token bytes,
  key material, passwords, connection strings, or byte buffers.
- The resource store row, redb WAL, and revision log never contain secret bytes.
- Resource API routing DTOs, d2b-bus request/response envelopes, and
  ResourceRef handles never carry secret bytes. The sole exception is the
  dedicated credential-delivery ComponentSession record described in
  §Credential-delivery endpoint contract: token bytes (`AcquireToken`,
  `RefreshToken`) and signature bytes (`SignChallenge`) are delivered
  exclusively through end-to-end Noise-encrypted records between the
  Credential Provider and the authorized consumer Provider process.
  d2b-bus authorizes the route and forwards the opaque Noise-protected
  records without terminating or decrypting them.
- Error/outcome messages are bounded, redacted, and must not echo request
  arguments.
- Audit records carry only the stable code plus the opaque lease handle
  digest. They never carry payload content, token prefix/suffix, or
  provider-internal diagnostics.
- OTEL spans and metrics never label secret bytes, provider-internal identity,
  or token scope values.

The injected credential client/port in the Credential Provider process acquires
the secret from the external service. Token delivery to the authorized consumer
Provider Process uses the end-to-end Noise channel defined in
§Credential-delivery endpoint contract; no intermediate process terminates
or stores that channel's content.

## Credential resource schema

### Spec

```yaml
apiVersion: resources.d2b.io/v3
type: Credential
metadata:
  name: work-entra
  zone: dev
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  ownerRef: null            # optional; see §Ownership
  finalizers: []
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00Z
  updatedAt: 2026-07-22T00:00:00Z
spec:
  providerRef: Provider/credential-entra
  scope:
    executionRef: Guest/work-vm       # optional Host or Guest placement restriction
    domainFilter: user                # optional: system | user
    userRef: User/alice               # optional; required when domainFilter=user
  audience: azure-resource-manager    # Provider-validated opaque audience token
  consumerRef: Provider/display-wayland  # optional; restricts which Provider may acquire
  allowedOperations:
    - acquire-token
    - refresh-token
    - revoke-token
  rotation:
    policy: proactive                 # on-expiry | proactive | on-demand
    proactiveWindowMs: 300000         # rotate when remaining lifetime < this value
    maxLeaseLifetimeMs: 3600000       # maximum lease lifetime cap
  expiry:
    hardDeadlineMs: 28800000          # hard maximum from issue time; 0 = provider default
  revocation:
    onOwnerDelete: immediate          # immediate | drain-leases
    onProviderGeneration: immediate   # immediate | drain-leases
status:
  observedGeneration: 1
  phase: Ready
  conditions:
    - type: CredentialReady
      status: "True"
      reason: lease-active
      message: ""
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:01Z
    - type: RotationDue
      status: "False"
      reason: within-window
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:01Z
  lastReconciledAt: 2026-07-22T00:00:01Z
  startedAt: 2026-07-22T00:00:01Z
  completedAt: null
  outcome: null
  credential:
    leaseHandle: <opaque bounded token; not a secret>
    leaseState: Active             # Active | Expired | Revoked | Unknown
    rotationGeneration: 1
    sourceVersion: <opaque bounded token; not a secret>
    expiresAtUnixMs: 1753228801000
    issuedAtUnixMs: 1753225201000
    lastRefreshedAt: 2026-07-22T00:00:01Z
    lastRotatedAt: null
    placementBinding: user-agent     # user-agent | host-system | guest-agent
```

### Spec field reference

| Field | Type | Required | Rules |
| --- | --- | --- | --- |
| `providerRef` | ResourceRef | Yes | Resolves `Provider/credential-*` in same Zone; must be Ready |
| `scope.executionRef` | ResourceRef | No | `Host/<name>` or `Guest/<name>`; restricts which execution context may acquire a lease; null = any execution context in Zone |
| `scope.domainFilter` | enum | No | `system` or `user`; restricts process domain allowed to acquire; null = any domain allowed by execution context |
| `scope.userRef` | ResourceRef | No | `User/<name>`; required when `scope.domainFilter=user`; restricts acquiring user identity |
| `audience` | string | Yes | Provider-validated non-secret audience token (e.g. Azure resource URI prefix, Secret Service collection alias, IMDS audience ID); max 256 chars; charset restricted to reject secret shapes (same rules as `OpaqueAzureRef`) |
| `consumerRef` | ResourceRef | No | `Provider/<name>`; restricts which Provider may invoke credential-bound methods; the consumer Provider's signed component descriptor and Role/RoleBinding selects the exact receiving component and Process within that Provider; null = any subject authorized by RBAC; must be in same Zone; no arbitrary component fallback |
| `allowedOperations` | enum[] | Yes | Bounded non-empty closed set of typed operation classes; see §Operation classes |
| `rotation.policy` | enum | Yes | `on-expiry` (rotate only after expiry), `proactive` (rotate before expiry window), or `on-demand` (never auto-rotate; consumer drives refresh explicitly) |
| `rotation.proactiveWindowMs` | u64 | No | Required when policy=proactive; rotate when remaining lifetime is below this threshold; max 1 800 000 ms (30 min); must be less than `rotation.maxLeaseLifetimeMs / 2` |
| `rotation.maxLeaseLifetimeMs` | u64 | No | Cap on Provider-granted lease lifetime; 0 = use Provider default cap; max `MAX_PROVIDER_LEASE_LIFETIME_MS` (7 days, per main contract constant) |
| `expiry.hardDeadlineMs` | u64 | No | Hard maximum total lifetime from issue; 0 = use Provider default; must not exceed `rotation.maxLeaseLifetimeMs` if both are non-zero |
| `revocation.onOwnerDelete` | enum | Yes | `immediate` (revoke all active leases before finalizer completes) or `drain-leases` (allow active leases to expire naturally) |
| `revocation.onProviderGeneration` | enum | Yes | `immediate` (revoke all active leases on Provider generation change) or `drain-leases` |

Provider-specific config fields are permitted in a bounded `providerSettings`
object validated against the signed Provider schema. No provider-specific field
may accept secret bytes.

### Status credential sub-object field reference

| Field | Rules |
| --- | --- |
| `leaseHandle` | Opaque, bounded (max 256 chars), non-secret handle assigned by the provider process; stable across refreshes within one rotation generation; suitable for d2b-bus method routing; never a token or partial token |
| `leaseState` | `Active`, `Expired`, `Revoked`, or `Unknown` |
| `rotationGeneration` | Monotonic counter incremented on rotation; bounded u64 |
| `sourceVersion` | Opaque, bounded, non-secret credential source version from the Provider port |
| `expiresAtUnixMs` | RFC 3339-equivalent Unix milliseconds of lease expiry; 0 when lease is absent |
| `issuedAtUnixMs` | Unix milliseconds of last successful lease acquisition |
| `lastRefreshedAt` | RFC 3339 UTC of last successful lease refresh |
| `lastRotatedAt` | RFC 3339 UTC of last successful rotation cycle |
| `placementBinding` | Closed stable code: `user-agent`, `host-system`, or `guest-agent` |

### Status conditions

| Condition type | Meaning |
| --- | --- |
| `CredentialReady` | `True` when leaseState=Active and lease is not within rotation window |
| `RotationDue` | `True` when policy=proactive and remaining lifetime < proactiveWindowMs |
| `ProviderUnavailable` | `True` when the credential provider process cannot be reached |
| `LeaseRevoked` | `True` when leaseState=Revoked and no replacement has been issued |

## Operation classes

`allowedOperations` uses a closed, non-secret typed enum. It never encodes
audience bytes, scopes, claims, or credential content:

| Class | Meaning |
| --- | --- |
| `acquire-token` | Consumer may request the provider process to use the credential to obtain a service token |
| `refresh-token` | Consumer may request the provider process to refresh an existing token |
| `revoke-token` | Consumer may request the provider process to revoke an existing token |
| `sign-challenge` | Consumer may request the provider process to sign a challenge without token export |
| `inspect-metadata` | Consumer may inspect opaque lease metadata without acquiring a token |

Operation-class validation:

- a `consumerRef` that is not authorized for a requested class fails closed;
- operation class sets are not extensible at runtime; new classes require a
  spec generation change and Provider schema update;
- the `inspect-metadata` class alone never grants token acquisition;
- each class maps to exactly one credential-bound service method on
  `d2b.credential.v3`.

## Placement model: User, Host, and Guest

A Credential resource belongs to one Zone. Its placement within that Zone is
governed by `scope.executionRef` and `scope.domainFilter`.

### User-domain Credential

```yaml
spec:
  providerRef: Provider/credential-secret-service
  scope:
    executionRef: Host/host-system
    domainFilter: user
    userRef: User/alice
```

The credential provider process runs in the user domain of `Host/host-system`
as `User/alice`. Only processes running as `User/alice` in the user domain of
that Host may invoke credential-bound methods. The Provider process is a
user-domain Process resource under the same Host.

### Host-system Credential

```yaml
spec:
  providerRef: Provider/credential-managed-identity
  scope:
    executionRef: Host/host-system
    domainFilter: system
```

The credential provider process runs as a system-domain Process under
`Host/host-system`. Only system-domain processes on that Host with an
authorized `consumerRef` may acquire a lease.

### Guest Credential

```yaml
spec:
  providerRef: Provider/credential-entra
  scope:
    executionRef: Guest/work-vm
    domainFilter: user
    userRef: User/alice
```

The Credential is served by a provider process running inside the Guest VM.
The provider process is a user-domain Process under `Guest/work-vm`. Cross-Zone
consumption is not permitted in the initial contract.

### Placement binding derivation

The controller derives `status.credential.placementBinding` from:

- `user-agent` when `scope.domainFilter=user` (regardless of Host or Guest);
- `host-system` when `scope.executionRef` resolves to a Host and `domainFilter=system`;
- `guest-agent` when `scope.executionRef` resolves to a Guest and
  `domainFilter=system`.

Scope validation at create/update:

- `scope.executionRef` must resolve to a Ready Host or Guest in the same Zone;
- `scope.userRef` must resolve to a Ready User in the same Zone;
- `userRef` is required when `domainFilter=user`;
- the resolved Provider must support the derived placement binding;
- the resolved consumerRef (if present) must be in the same Zone and be
  compatible with the placement.

## Ownership and finalizer model

### ownerRef

`metadata.ownerRef` is optional. Common ownership patterns:

- a Guest resource (e.g. `Guest/work-vm`) owns its Credentials; VM
  controller creates and manages them;
- a Provider resource (e.g. `Provider/display-wayland`) owns Credentials it
  declares in its root config/descriptor;
- a standalone Credential has no ownerRef and is managed directly.

Any committed child mutation of the owning resource triggers the owner's
controller reconcile loop. The Credential controller does not implicitly
become a child of its credential Provider.

### Finalizers

| Finalizer ID | Owner | Meaning |
| --- | --- | --- |
| `credential.d2b.io/provider-revoke` | Credential controller | Revoke all active leases before deletion proceeds; honors `revocation.onOwnerDelete` policy |
| `credential.d2b.io/consumer-drain` | consumerRef controller | Drain in-flight operations before Provider releases the lease handle |

Finalizer execution order: `consumer-drain` completes before
`provider-revoke`. A missing consumerRef removes the `consumer-drain` finalizer
automatically.

A terminal revocation (`leaseState=Revoked`) or a confirmed expiry
(`leaseState=Expired`) satisfies the `provider-revoke` finalizer immediately
without further network calls. An ambiguous or unknown state requires one
revoke attempt before the controller reports `blocked` with a `provider-unavailable`
condition and sets a bounded `requeue-at`.

## RBAC

The Credential ResourceType adds these standard resource verbs to the native
RBAC model:

| Verb | Who | Meaning |
| --- | --- | --- |
| `get` | Any authorized subject | Read Credential metadata/spec/status (no secret bytes present) |
| `list` | Any authorized subject | List Credentials matching filters |
| `watch` | Any authorized subject | Watch Credential events |
| `create` | Deployer/system-core configuration controller | Create a new Credential resource |
| `update-spec` | Deployer/system-core configuration controller | Replace Credential spec |
| `update-status` | Credential controller only | Update Credential status |
| `update-finalizers` | Credential controller, consumerRef controller | Add/remove owned finalizers |
| `delete` | Deployer/system-core configuration controller | Request Credential deletion |
| `use-credential` | Consumer subject authorized via consumerRef | Invoke credential-bound service methods on `d2b.credential.v3` |

`use-credential` is a runtime service verb evaluated by d2b-bus when a consumer
invokes a credential-bound method. It is checked against the same
Role/RoleBinding engine that governs resource verbs. The Role rule shape:

```yaml
rules:
  - resourceTypes: [Credential]
    verbs: [use-credential]
    resourceNames: [work-entra]
    zones: [dev]
    executionRefs: [Guest/work-vm]
    operationClasses: [acquire-token, refresh-token]
```

`operationClasses` is a Credential-specific rule extension narrowing which
operation classes are authorized in the bind. An empty list means no
operation-class restriction beyond the resource spec's own `allowedOperations`.
The effective operation set is the intersection of the resource `allowedOperations`
and the Role `operationClasses`.

Status ownership: only the Credential controller's exact registered process
generation may call `UpdateStatus` on a Credential resource.

## Credential-bound service methods

The `d2b.credential.v3` protobuf/ttrpc service is served by the credential
provider process and routed through d2b-bus over ComponentSession. It is not
served directly by the Zone runtime.

Route key example:

```text
(
  Zone=dev,
  service=d2b.credential.v3,
  method=AcquireToken,
  target=Credential/work-entra,
  schema_fingerprint=<digest>,
  provider_generation=<gen>,
  controller_generation=<gen>
)
```

### Service methods

| Method | Required operation class | Outer DTO fields | Token bytes |
| --- | --- | --- | --- |
| `Status` | none (inspect-metadata or RBAC get) | `CredentialStatusResponse`: leaseState, rotationGeneration, sourceVersion, expiresAtUnixMs, placementBinding | none |
| `AcquireToken` | `acquire-token` | `AcquireTokenResponse`: leaseHandle, sourceVersion, rotationGeneration, expiresAtUnixMs | raw token bytes in a dedicated sensitive ComponentSession record (see §Credential-delivery endpoint contract) |
| `RefreshToken` | `refresh-token` | `RefreshTokenResponse`: leaseHandle, sourceVersion, rotationGeneration, new expiresAtUnixMs | raw token bytes in a dedicated sensitive ComponentSession record |
| `RevokeToken` | `revoke-token` | `RevokeTokenResponse`: closed revocation result (Revoked or AlreadyRevoked), revokedAtUnixMs | none |
| `SignChallenge` | `sign-challenge` | `SignChallengeResponse`: outcome code | signature bytes in a dedicated sensitive ComponentSession record (same channel as token delivery) |
| `InspectMetadata` | `inspect-metadata` | `InspectMetadataResponse`: leaseState, rotationGeneration, sourceVersion, expiresAtUnixMs | none |

The `Status`, `RevokeToken`, and `InspectMetadata` response DTOs are non-secret:
they carry only opaque identifiers, outcome codes, and timestamps. `AcquireToken`,
`RefreshToken`, and `SignChallenge` additionally deliver secret bytes in a
dedicated sensitive ComponentSession record (see §Credential-delivery endpoint
contract). Every method:

- rejects a request where the authenticated subject is not the declared
  `consumerRef` (when consumerRef is set) and RBAC `use-credential` is denied;
- rejects an operation class not in `spec.allowedOperations`;
- returns a stable closed error code rather than provider-internal diagnostics;
- carries operation/idempotency/correlation IDs from d2b-bus context;
- enforces a per-call deadline propagated from the d2b-bus context.

Token bytes and signature output travel end-to-end in the Noise-encrypted
sensitive ComponentSession record between the Credential Provider and the
authorized consumer Provider Process; they never enter the outer DTO, the
resource spec/status, the store, or any audit/log surface. The provider process's
injected credential client/port acquires the token from the external service and
delivers it through this bounded record; there is no ambient SDK chain or fallback
path. The consumer Provider's signed component descriptor and Role/RoleBinding
determines the exact receiving Process; no arbitrary component is permitted.

## Credential-delivery endpoint contract

Token and signature delivery uses a dedicated end-to-end Noise ComponentSession
established directly between the Credential Provider process and the authorized
consumer Provider process. d2b-bus authorizes the route and then forwards opaque
Noise-protected records between the two endpoints; it never terminates, decrypts,
or stores the delivery channel's content. This channel is used for all three
sensitive-output methods: `AcquireToken`, `RefreshToken`, and `SignChallenge`.

### Session profile

The delivery channel MUST use `Noise_KK_25519_ChaChaPoly_SHA256`. The NN
profile is forbidden for sensitive output delivery; any anonymous-channel attempt
is rejected immediately. Both parties require enrolled static keys.

- **Credential Provider** key: registered at Provider installation; the bus
  holds only the public key.
- **Consumer Provider** key: extracted from the consumer Provider's signed
  component descriptor. The consuming component/Process identity is derived from
  the consumer Provider's Role/RoleBinding, not from an arbitrary component
  selection.

### Binding contract

Each delivery session binds:

| Field | Description |
| --- | --- |
| `credentialRef` | Canonical Credential ResourceRef: `Credential/<name>` |
| `credentialUID` | Credential resource UID (stable across spec updates) |
| `credentialGeneration` | Credential resource generation at time of delivery |
| `consumerProviderRef` | `Provider/<name>` matching `spec.consumerRef` |
| `consumerComponentGeneration` | Consumer Provider component generation (from signed descriptor) |
| `audience` | Closed `spec.audience` value (opaque; not echoed in logs or spans) |
| `operationClass` | Closed operation class (`acquire-token`, `refresh-token`, or `sign-challenge`) |
| `expiryUnixMs` | Absolute expiry of this delivery session; clipped to `spec.rotation.maxLeaseLifetimeMs` |
| `deadlineUnixMs` | Hard session close deadline; must be ≤ `expiryUnixMs` |
| `routeDigest` | Digest of the bus-authorized route parameters (Zone, consumer, provider) |
| `schemaVersion` | Fixed version of this binding contract |
| `maxTokenBytes` | Closed upper bound on sensitive output size (token or signature bytes) for this delivery session |
| `transcriptDigest` | Noise transcript digest after handshake completion, before any record |

The binding is constructed by d2b-bus during route authorization and conveyed to
both endpoints in the Noise handshake prologue. Both parties MUST verify the
full binding before accepting records.

### Security requirements for the delivery session

1. **Enrolled keys only**: Both static keys must be enrolled and verified at
   session initiation. Any NN/NX/N pattern or raw-token bootstrap attempt is
   rejected immediately; the session is closed and zeroized.

2. **Replay-safe sequence**: Each delivery session carries a monotonically
   increasing per-credential-UID sequence number. A replay of a prior session's
   ciphertext at the same or lower sequence number is rejected.

3. **Bounded output size**: The sensitive output record (token or signature bytes)
   MUST NOT exceed `maxTokenBytes` (delivery-session bound). Any record exceeding
   this size is rejected; the channel is closed and zeroized immediately.
   Fragmentation is not permitted unless the protocol explicitly bounds each
   fragment and the reassembled record does not exceed `maxTokenBytes`.

4. **Zeroizing buffers**: The delivery record's plaintext MUST be zeroed in memory
   immediately after the consumer extracts it. The Credential Provider MUST zero
   the plaintext source after encryption. All intermediate buffers involved in
   serialization and deserialization are zeroizing types.

5. **Redacted Debug**: All credential-bearing Rust types involved in delivery
   (request, response, record wrapper, buffer) MUST implement `Debug` via a
   redacted hand-written impl that emits only the type name and a placeholder
   value. Derived `Debug` is forbidden for these types.

6. **No automatic success-shaped replay**: After any ambiguous delivery outcome
   (timeout, partial write, disconnection before confirmation), the Credential
   Provider MUST NOT automatically retry with the same record. The consumer
   must re-initiate via `AcquireToken`, `RefreshToken`, or `SignChallenge`,
   which establishes a new delivery session with a fresh sequence number.

7. **Immediate close/zeroize**: After the delivery record is confirmed received
   (consumer ACKs the delivery session record), the Credential Provider closes
   the delivery channel and zeroizes all session key material. The consumer
   similarly closes and zeroizes after extraction. The channel is not reused
   across multiple deliveries.

### RBAC enforcement point

d2b-bus performs the following checks before authorizing the route:

- RBAC `use-credential` for the authenticated consumer Provider subject,
  Credential ResourceRef, and operation class;
- `spec.consumerRef` matches the consumer Provider identity from the signed
  component descriptor;
- `spec.allowedOperations` includes the operation class;
- The current lease state permits the operation class (Active/RotationDue for
  `refresh-token`; no existing active lease required for `acquire-token`
  subject to `maxLeases`);
- No Role/RoleBinding or Provider generation change has revoked the
  authorization since the last audit checkpoint.

After these checks pass, d2b-bus forwards opaque Noise-encrypted records
between the two endpoints until the delivery session closes. Bus never buffers
or stores the records.

## Noise session binding for credential-bound calls

Non-secret credential operations (`Status`, `RevokeToken`, `InspectMetadata`)
traverse the standard ComponentSession/d2b-bus stack defined in
`ADR-046-componentsession-and-bus`:

1. The consumer process authenticates to d2b-bus using a local
   `Noise_NN_25519_ChaChaPoly_SHA256` ComponentSession over a Unix socketpair
   or seqpacket transport. The transport provides `SO_PEERCRED` and process
   identity evidence mapping the consumer to an exact Zone subject.

2. d2b-bus checks native RBAC `use-credential` for the authenticated subject,
   target Credential ResourceRef, operation class, and current
   Role/RoleBinding/Provider revisions.

3. For non-secret methods, d2b-bus routes the call to the exact credential
   provider process using an enrolled `Noise_KK_25519_ChaChaPoly_SHA256`
   session. The credential provider's static public key is registered at
   Provider installation; the bus holds only the public key.

4. The credential provider process responds with the non-secret outer DTO.
   The `AuthenticatedSubjectContext` carried by the bus routing includes
   `credentialRef`, `operationClass`, `consumerRef`, and the session
   generation/transcript digest for audit.

For secret-returning methods (`AcquireToken`, `RefreshToken`, `SignChallenge`),
d2b-bus additionally initiates the credential-delivery endpoint contract
(§Credential-delivery endpoint contract): it authorizes the route, then
forwards opaque Noise-encrypted records between the Credential Provider and
the consumer Provider without terminating or decrypting the delivery channel.
The outer routing channel uses KK; the delivery channel is a separate nested
KK session whose plaintext is never accessible to the bus.

A credential-bound ComponentSession carries its authorization lease revision.
When the lease expires or is invalidated by a Role/RoleBinding or Provider
generation change, d2b-bus closes the stream and refuses new requests on the
existing session. The consumer must re-establish both the ComponentSession and
re-request the credential-bound operation.

## Rotation, expiry, and revocation state machine

```text
Absent (no active lease)
  |-- AcquireToken ---------> Active (leaseState=Active, CredentialReady=True)

Active
  |-- proactive window ------> RotationDue (CredentialReady=True, RotationDue=True)
  |-- RefreshToken ----------> Active (rotationGeneration unchanged, new expiresAtUnixMs)
  |-- [rotation] ------------> Active (rotationGeneration+1, new leaseHandle)
  |-- Provider gen change ---> [revocation.onProviderGeneration=immediate] -> Revoked
  |                            [revocation.onProviderGeneration=drain-leases] -> Expired on deadline
  |-- RevokeToken -----------> Revoked (leaseState=Revoked, CredentialReady=False)
  |-- expiry deadline -------> Expired (leaseState=Expired, CredentialReady=False)

RotationDue
  |-- rotation attempt ------> Active (new rotationGeneration)
  |-- rotation fails --------> RotationDue/Degraded (bounded retry)
  |-- final retry fails -----> Failed (CredentialReady=False, outcome=rotation-failed)

Expired
  |-- AcquireToken ----------> Active (new lease; rotationGeneration+1)

Revoked
  |-- AcquireToken ----------> Active if policy permits re-acquisition
  |-- resource delete -------> provider-revoke finalizer satisfied immediately
```

### Rotation algorithm

For `policy=proactive`:

1. Controller sets `RotationDue=True` condition when
   `now + proactiveWindowMs >= expiresAtUnixMs`.
2. Controller issues a new AcquireToken request to the provider process with
   `requested_expiry_unix_ms` = `now + maxLeaseLifetimeMs` (capped by provider
   maximum).
3. On grant: increment `rotationGeneration`, store new `leaseHandle` and
   `expiresAtUnixMs`, commit status, clear `RotationDue`.
4. Old lease is valid until the new one is active. A second AcquireToken with
   the same `idempotency_key` returns the same grant.
5. On failure: bounded retry under `requeue-at` disposition; degrade after
   final retry.

For `policy=on-demand`: controller does not auto-rotate. Consumer explicitly
calls `RefreshToken` or `RevokeToken`+`AcquireToken`. Controller only monitors
expiry for status conditions.

### Revocation on Provider generation change

When the Credential provider Process is replaced (new Provider generation):

- `immediate` policy: controller immediately calls `RevokeToken` against the
  old provider process before the generation changes. If the old process is
  unreachable, the controller marks `leaseState=Revoked` and writes a bounded
  audit record.
- `drain-leases` policy: controller does not actively revoke. Active leases
  expire by their natural deadline. Status remains `Active` until expiry.

## Async reconciliation

The Credential controller handles async reconciliation following the
`ADR-046-resource-reconciliation` loop model with these Credential-specific
behaviors:

### Controller descriptor

```yaml
providerId: Provider/credential-<impl>
controllerType: Credential
resourceTypes: [Credential]
watchSelectors:
  - resourceType: Credential
    providerRefFilter: Provider/credential-<impl>
  - resourceType: Provider
    nameFilter: credential-<impl>
dependencySelectors:
  - resourceType: Provider
    relationship: providerRef
  - resourceType: Host
    relationship: scope.executionRef
  - resourceType: Guest
    relationship: scope.executionRef
  - resourceType: User
    relationship: scope.userRef
ownerChildTriggers: [owned-resource-changed]
reconcileConcurrency: 8
maxPendingResources: 256
finalizers: [credential.d2b.io/provider-revoke]
observeInterval: 30s       # check expiry/rotation due; no external drift
```

### Reconcile lifecycle

**Create**: controller acquires a lease if `allowedOperations` includes
`acquire-token`; sets `leaseState=Active`, writes status, sets
`CredentialReady=True`.

**Spec update**: if `providerRef` changes, revoke old lease under old policy,
then acquire new lease under new provider. If scope/audience changes and the
provider requires re-acquisition, revoke old and acquire new.

**Dependency-ready**: when the credential Provider transitions to Ready after a
Pending state, the controller re-attempts any pending lease acquisition.

**Scheduled-observe**: every `observeInterval`, the controller calls
`InspectMetadata` on the provider to confirm `leaseState=Active` and check
whether rotation is due. This detects out-of-band revocations.

**Deletion-requested**: trigger `provider-revoke` finalizer. On completion,
clear finalizer and allow core to emit `phase=Deleted`.

**Provider-generation-changed**: apply `revocation.onProviderGeneration`
policy as described in §Rotation, expiry, and revocation.

**Unknown state**: when the credential provider process is unreachable or
returns an error that cannot be resolved without external action, set
`phase=Degraded`, `ProviderUnavailable=True`, and `CredentialReady=False`.
Bounded retry under `requeue-at`. After retry exhaustion, set `phase=Failed`.

### Idempotency

Every AcquireToken/RefreshToken/RevokeToken call carries a stable
`idempotency_key` derived from the Credential UID, current `rotationGeneration`,
and the operation class. A duplicate acquire with the same key returns the
existing grant without double-issuing.

## Errors

Stable Credential-specific error codes:

| Code | Meaning |
| --- | --- |
| `credential-not-found` | Credential resource does not exist in this Zone |
| `credential-provider-unavailable` | Credential provider process unreachable or not Ready |
| `credential-lease-expired` | Lease is past its expiry deadline |
| `credential-lease-revoked` | Lease was explicitly revoked |
| `credential-operation-denied` | Operation class not in `allowedOperations` or RBAC denied |
| `credential-consumer-mismatch` | Requesting subject does not match `consumerRef` |
| `credential-placement-mismatch` | Request execution context/domain does not match `scope` |
| `credential-rotation-failed` | Proactive rotation attempt failed after bounded retries |
| `credential-invariant-failure` | Provider returned a response failing invariant checks |
| `credential-schema-invalid` | Spec field fails validation at create/update |
| `credential-queue-pressure` | Provider lease table at capacity; retry after backpressure |

All error messages are bounded (max 240 UTF-8 chars), stripped of control
characters, and must not contain token bytes, URLs, UUIDs, provider diagnostics,
host paths, or connection string shapes.

## Audit

Audit records for Credential operations:

| Event | Fields retained |
| --- | --- |
| Credential resource create/update/delete | Zone, subject digest, ResourceRef, verb, revision result, authorization decision |
| `AcquireToken` | Zone, subject digest, credential ResourceRef, operation class, `rotationGeneration`, outcome code, idempotency key digest |
| `RefreshToken` | Zone, subject digest, credential ResourceRef, operation class, `rotationGeneration`, outcome code, idempotency key digest |
| `RevokeToken` | Zone, subject digest, credential ResourceRef, operation class, `rotationGeneration`, revocation result code |
| `SignChallenge` | Zone, subject digest, credential ResourceRef, operation class, outcome code (no signature bytes) |
| Rotation | Zone, credential ResourceRef, trigger reason, old `rotationGeneration`, new `rotationGeneration`, outcome code |
| Provider generation change revocation | Zone, credential ResourceRef, policy applied, outcome code |

Excluded from all audit records: token bytes, key material, passwords, bearer
strings, provider-internal diagnostics, host paths, connection strings,
audience literals, tenant/subscription/client IDs, endpoint URIs, and
Noise/session key material.

## OTEL and metrics

### Spans

Span names use the pattern `d2b.credential.<operation>`:

- `d2b.credential.acquire_token`
- `d2b.credential.refresh_token`
- `d2b.credential.revoke_token`
- `d2b.credential.sign_challenge`
- `d2b.credential.inspect_metadata`
- `d2b.credential.reconcile`
- `d2b.credential.rotation`

Required span attributes (closed set):

| Attribute | Value |
| --- | --- |
| `d2b.zone` | Zone name |
| `d2b.credential.name` | Credential resource name |
| `d2b.credential.provider` | Provider name (e.g. `credential-entra`) |
| `d2b.credential.operation_class` | Closed enum string |
| `d2b.credential.placement_binding` | `user-agent` / `host-system` / `guest-agent` |
| `d2b.credential.outcome` | Stable closed outcome code |
| `d2b.credential.rotation_generation` | Numeric rotation generation |

Forbidden from spans/attributes: token bytes, audience literals, provider
diagnostics, host paths, resource IDs, tenant/subscription IDs, endpoint URIs,
correlation IDs that embed secret shapes.

### Metrics

| Metric | Type | Labels |
| --- | --- | --- |
| `d2b_credential_operations_total` | Counter | `provider`, `operation_class`, `placement_binding`, `outcome` |
| `d2b_credential_lease_expiry_seconds` | Gauge | `provider`, `credential_name`, `placement_binding` |
| `d2b_credential_rotation_total` | Counter | `provider`, `policy`, `outcome` |
| `d2b_credential_provider_health` | Gauge (0/1) | `provider` |
| `d2b_credential_active_leases` | Gauge | `provider`, `placement_binding` |

Label cardinality is bounded. `credential_name` is used only in expiry gauges
where per-resource precision is required; it is omitted from high-cardinality
counters.

## Nix configuration

### Zone-level Credential declaration

```nix
# d2b.zones.<zone>.resources.<name> = { type = "..."; spec = { ... }; }
# metadata is optional: only ownerRef and presentation metadata
# (labels, annotations) are Nix-authorable. Option types, defaults,
# and docs come from the committed ResourceTypeSchema and Provider schema.
{
  d2b.zones.dev.resources = {

    work-entra = {
      type = "Credential";
      # optional: ownerRef and/or presentation labels/annotations only
      metadata = {
        labels = { "team" = "platform"; };
      };
      spec = {
        providerRef = "Provider/credential-entra";
        scope = {
          executionRef = "Guest/work-vm";
          domainFilter = "user";
          userRef = "User/alice";
        };
        audience = "azure-resource-manager";
        consumerRef = "Provider/display-wayland";
        allowedOperations = [ "acquire-token" "refresh-token" ];
        rotation = {
          policy = "proactive";
          proactiveWindowMs = 300000;
          maxLeaseLifetimeMs = 3600000;
        };
        revocation = {
          onOwnerDelete = "immediate";
          onProviderGeneration = "immediate";
        };
      };
    };

    local-keyring = {
      type = "Credential";
      spec = {
        providerRef = "Provider/credential-secret-service";
        scope = {
          executionRef = "Host/host-system";
          domainFilter = "user";
          userRef = "User/alice";
        };
        audience = "user-session";
        allowedOperations = [ "acquire-token" "revoke-token" "inspect-metadata" ];
        rotation = {
          policy = "on-demand";
        };
        revocation = {
          onOwnerDelete = "immediate";
          onProviderGeneration = "immediate";
        };
      };
    };

  };
}
```

The authoring shape is the same for every ResourceType:
- `d2b.zones.<zone>.resources.<name>` is the single generic resource attr path;
  there is no type-specific sub-namespace.
- `type` is the ResourceType string (`"Credential"`, `"Provider"`, `"Host"`, etc.).
- `metadata` is **optional**. When present it may contain `ownerRef` and/or
  presentation metadata (`labels`, `annotations`). No other metadata field is
  Nix-authorable.
- `spec` fields mirror the canonical ResourceTypeSchema `spec` object exactly —
  the same field names, the same nesting, the same value shapes. There is no
  second bespoke Nix vocabulary, no renamed subkey, and no Provider-specific
  re-nesting.
- `metadata.name` is derived from the attr key (`work-entra`).
- `metadata.zone` is derived from the Zone attr key (`dev`).
- `apiVersion` defaults to `resources.d2b.io/v3` and is not authored.
- `status` is omitted from Nix entirely; it is controller-managed and read-only.
- `metadata.uid`, `generation`, `revision`, `managedBy`, `configurationGeneration`,
  `createdAt`, `updatedAt` are filled by core; they are never authored in Nix.

Nix option types, defaults, and inline documentation for every `spec` field are
**generated from the same `ResourceTypeSchema` JSON** (`docs/reference/schemas/v3/credential.json`
and the signed Provider schema for `providerRef`-typed constraints) that drives
build-time validation. No bespoke Nix option module is maintained separately
from the schema.

Provider-specific config (e.g. `credential-entra` root config such as `tenantId`)
lives under the Provider resource's own `spec`, not on the Credential resource.
The Credential `spec` carries only non-secret selector/policy fields. Any string
field that resembles a secret fails the eval-time secret-shape assertion before
the NixOS build completes.

### Artifact catalog

Derivation-valued inputs (Provider binaries, NixOS system closures) are configured
exclusively in a separate named artifact catalog, never inside a ResourceSpec.

```nix
# d2b.artifacts.<id> = { package = <derivation>; type = "provider"|"nixos-system"|...; };
# IDs are plain bounded identifiers ([A-Za-z0-9_-], max 128 chars).
# Nix builds and includes the derivation, validates catalog type/ID/duplicates/trust,
# and emits a private integrity-pinned artifact catalog mapping each ID to
# type / digest / closure metadata. Store paths are private catalog implementation
# data; they never appear in any resource spec, status, audit record, or log.
{
  d2b.artifacts = {
    credential-secret-service-bin = {
      package = pkgs.d2b-provider-credential-secret-service;
      type    = "provider";
    };
    credential-entra-bin = {
      package = pkgs.d2b-provider-credential-entra;
      type    = "provider";
    };
    credential-managed-identity-bin = {
      package = pkgs.d2b-provider-credential-managed-identity;
      type    = "provider";
    };
  };
}
```

Provider resources reference their binary via `spec.artifactId`; Guest system
resources use `spec.systemArtifactId`. Both are plain bounded string IDs — not
`*Ref` fields, because `Artifact` is not a ResourceType and the value does not
serialize as `<ResourceType>/<name>`. A missing or wrong-type artifact ID fails
the NixOS build; the error identifies the invalid ID and its catalog entry.

Artifact catalog rules:
- Each ID is unique within the catalog; duplicate IDs fail the NixOS build.
- The `type` field must match the expected type for the consuming resource field
  (`"provider"` for `spec.artifactId` in a Provider resource; `"nixos-system"`
  for `spec.systemArtifactId` in a Guest resource).
- Catalog entries are validated and integrity-pinned at build time alongside the
  resource bundle; the emitted artifact catalog is co-located with the resource
  bundle at `/etc/d2b/zones/<zone>/artifact-catalog.json` (mode 0640,
  root-readable, `d2bd`-readable), never served over the public d2b-bus surface.
- `activation-nixos` verifies both the resource bundle digest and the artifact
  catalog digest before creating or updating any resource.
- Credential resource specs (`type = "Credential"`) do not have `artifactId` or
  `systemArtifactId` fields; those fields belong on Provider and Guest specs
  respectively.

### Eval-time and build-time validation

All Credential resource declarations are validated at Nix eval time (before the
build) and at build time (against committed generated schemas). Because Nix option
types, defaults, and constraint docs are generated from the ResourceTypeSchema,
many type/bounds errors are caught by the Nix option type system before
assertions run.

#### Eval-time assertions (Nix, `nixos-modules/assertions.nix` pattern)

Applied to every `d2b.zones.<zone>.resources.<name>` entry whose `type = "Credential"`:

- `spec.providerRef` resolves a declared `Provider/credential-*` entry in the
  same Zone; the Provider's `credentialDomains` must include the declared
  `spec.scope.domainFilter`.
- `spec.audience` passes the OpaqueAzureRef-equivalent charset and length validation
  (`^[A-Za-z0-9._:/@-]+$`, max 256 chars; rejects `=`, `+`, whitespace, `{}`).
- `spec.rotation.proactiveWindowMs` < `spec.rotation.maxLeaseLifetimeMs / 2`
  when both are non-zero.
- `spec.rotation.policy = "proactive"` requires `proactiveWindowMs > 0` and
  `maxLeaseLifetimeMs > 0`.
- `spec.consumerRef`, if set, resolves a declared `Provider/<name>` in the same Zone.
- `spec.scope.executionRef` resolves a declared `Guest/<name>` or `Host/<name>`
  in the same Zone.
- `spec.scope.userRef`, when set, resolves a declared `User/<name>` in the same
  Zone; required when `spec.scope.domainFilter = "user"`.
- `spec.allowedOperations` is a non-empty subset of the Provider's declared
  `supportedOperations`; values outside the Provider's set fail the assertion.
- Provider-specific constraints (placement, supported operations) are checked
  against the committed Provider schema (`docs/reference/schemas/v3/` or the
  generated schema embedded in the Provider package): `credential-secret-service`
  rejects `domainFilter = system` and `domainFilter = guest`; `credential-managed-identity`
  rejects `domainFilter = user`.
- No `spec` field accepts a credential-shaped value: `contains_sensitive_shape`
  guard (adapted from `d2b-realm-provider/src/error.rs`) runs at Nix eval time
  on all string fields; the eval fails with a descriptive assertion if any field
  matches a secret pattern.
- No Credential resource name (attr key) uses the `sys-` reserved prefix.
- Two Credential resources may not declare the same `(providerRef, scope.executionRef,
  scope.userRef, audience)` tuple in the same Zone (duplicate-binding conflict
  rule).
- `spec.providerRef` must resolve a Provider resource in the same Zone; that
  Provider resource's `spec.artifactId` must resolve an artifact catalog entry
  of `type = "provider"`. A missing catalog entry or wrong type fails the eval.

#### Build-time validation (via `xtask gen-schemas` and drift gate)

- Each Credential resource `spec` is validated against the committed
  `ResourceTypeSchema` JSON at `docs/reference/schemas/v3/credential.json`.
  Unknown fields, wrong types, and out-of-bounds values produce a build error.
  **This is the same schema that generated the Nix option types; both must agree.**
- The Provider-specific schema for `spec.providerRef` (e.g.
  `docs/reference/schemas/v3/provider-credential-entra-config.json`) is
  cross-checked: the `audience` charset rules and the `tenantId`
  format match the Provider's declared validation constraints.
- The drift gate (`make test-drift`) regenerates all schemas with
  `cargo xtask gen-schemas` and asserts `git diff --exit-code`. A committed
  schema change that does not match the crate-derived schema fails the gate.
  A Nix options module that diverges from the generated schema also fails the
  gate.
- Secrets remain as `Credential` refs: any provider-specific config field
  declared with `secretRef: true` in the Provider schema must reference
  `Credential/<name>`, not an inline string. The build-time validator confirms
  this constraint on every Provider-specific config block.
- **Artifact catalog validation**: each `d2b.artifacts.<id>` entry is validated
  for type (`"provider"`, `"nixos-system"`, etc.), charset of the ID
  (`[A-Za-z0-9_-]+`, max 128 chars), and uniqueness (duplicate IDs fail the
  build). Every `spec.artifactId` in a Provider resource must resolve an entry
  of `type = "provider"` in the catalog. Store paths and closure metadata in the
  emitted artifact catalog are private; they are not present in any resource
  spec, resource bundle, status, audit record, or log surface. The artifact
  catalog is emitted at `/etc/d2b/zones/<zone>/artifact-catalog.json` with its
  own SHA-256 digest header; `activation-nixos` verifies this digest alongside
  the resource bundle digest before any create/update.

### Canonical ResourceSpec JSON shape

Nix renders each Credential declaration as a canonical JSON object with
deterministic defaults applied. The `spec` attr maps directly and unchanged to
the `spec` object in the output. The output for the `work-entra` example above:

```json
{
  "apiVersion": "resources.d2b.io/v3",
  "type": "Credential",
  "metadata": {
    "name": "work-entra",
    "zone": "dev",
    "labels": {
      "team": "platform"
    },
    "annotations": {},
    "ownerRef": null
  },
  "spec": {
    "providerRef": "Provider/credential-entra",
    "scope": {
      "executionRef": "Guest/work-vm",
      "domainFilter": "user",
      "userRef": "User/alice"
    },
    "audience": "azure-resource-manager",
    "consumerRef": "Provider/display-wayland",
    "allowedOperations": ["acquire-token", "refresh-token"],
    "rotation": {
      "policy": "proactive",
      "proactiveWindowMs": 300000,
      "maxLeaseLifetimeMs": 3600000
    },
    "revocation": {
      "onOwnerDelete": "immediate",
      "onProviderGeneration": "immediate"
    }
  }
}
```

Rules for this output:

- `metadata.name` is the attr key (`work-entra`); `metadata.zone` is the Zone
  attr key (`dev`); `apiVersion` is the fixed default `resources.d2b.io/v3` and
  is a top-level field, not nested inside `metadata`.
  None of these are authored in Nix.
- `metadata` in the Nix-rendered output contains only: `name` (derived), `zone`
  (derived), and optionally `ownerRef` and presentation metadata (`labels`,
  `annotations`) if authored. `finalizers` is omitted from the Nix-rendered
  input; core manages finalizers and never accepts them from the Nix bundle.
- `metadata.uid`, `metadata.generation`, `metadata.revision`, `metadata.createdAt`,
  `metadata.updatedAt` are not present: these are assigned by the resource store
  on create/update and are not Nix-authored fields.
- `status` is not present: status is exclusively controller-managed.
- The `spec` object in JSON is identical in structure to the `spec` attr in Nix.
  No field is renamed, re-nested, or transformed by the Nix layer.
- All references use the canonical ResourceRef form (`<Type>/<name>`).
- `allowedOperations` is sorted alphabetically and deduplicated.
- String fields are Unicode-normalized (NFC) before serialization.
- Deterministic defaults applied at eval time:
  `revocation.onOwnerDelete` defaults to `"immediate"` when absent;
  `revocation.onProviderGeneration` defaults to `"immediate"` when absent.
- `metadata.labels` and `metadata.annotations` in the Nix authoring map
  directly to the corresponding output fields. They are optional presentation
  metadata. No management labels are generated by Nix.
- Core assigns `metadata.managedBy = "configuration"` and
  `metadata.configurationGeneration = <N>` to all configuration-managed
  resources at create/update time. These are first-class metadata fields, not
  labels, and are never authored in Nix.
- Build validation compares this canonical rendered JSON against the committed
  ResourceTypeSchema to confirm exact structural parity.

### Zone resource bundle and generation emission

Nix emits all Credential resource declarations for a Zone into the Zone resource
bundle file alongside other ResourceTypes. The bundle is produced by the
`activation-nixos` Provider's Nix configuration compiler:

```json
{
  "bundleVersion": 1,
  "zone": "dev",
  "nixConfigGeneration": 42,
  "emittedAt": "2026-07-22T00:00:00Z",
  "digestAlgorithm": "sha256",
  "digest": "<sha256-hex of canonical-json-body>",
  "resources": [
    { "type": "Credential", "name": "local-keyring", "spec": { ... } },
    { "type": "Credential", "name": "work-entra",    "spec": { ... } }
  ]
}
```

Rules:

- `resources` is sorted lexicographically by `(type, name)` before digest
  computation. The order is deterministic and reproducible.
- `digest` covers the UTF-8 encoding of the canonical JSON of the `resources`
  array (without the `digest` field itself). The digest is computed by Nix at
  eval time using a pure derivation and committed into the store; the
  `activation-nixos` Provider verifies it before applying.
- The bundle file is stored at
  `/etc/d2b/zones/<zone>/resource-bundle.json` by the NixOS activation script.
  It is root-readable, `d2bd`-readable, mode 0640.
- The bundle includes all Nix-managed ResourceTypes for the Zone, not only
  Credentials. Credential entries appear in the `resources` array sorted with
  all other types.
- `nixConfigGeneration` is the NixOS system configuration generation number
  (from `config.system.nixos.version` or the NixOS generation number).
- A NixOS build failure (schema validation, secret-shape detection, reference
  resolution) prevents the bundle from being emitted. The prior bundle remains
  on disk until replaced by a successful build.

### Resource managedBy classification

The `activation-nixos` Provider distinguishes three classes of Credential
resources within a Zone by `metadata.managedBy`:

| Class | Marker | Example |
| --- | --- | --- |
| **Configuration-managed** | `metadata.managedBy = "configuration"` | Any resource declared under `d2b.zones.<zone>.resources` in Nix |
| **Controller-created** | `metadata.managedBy = "controller"` | Ephemeral lease records, auto-generated proxy credentials created by a Provider controller during reconciliation |
| **API-created** | `metadata.managedBy = "api"` | Credentials created directly via the resource API by an operator or external system; persist until an explicit Delete; never generation-swept |

Rules:

- `managedBy` and `ownerRef` are orthogonal. Configuration-managed and
  API-created resources may carry an optional same-Zone `ownerRef` and
  participate in owner cascade under the standard ownerRef deletion propagation.
  The presence of an `ownerRef` does not change their `managedBy` classification.
- Only configuration-managed Credentials are subject to the generation cleanup
  contract described below.
- Controller-created Credential resources carry a controller-assigned `ownerRef`
  to their parent resource. Their lifecycle is governed by the owner controller's
  reconciliation loop and the standard ownerRef deletion propagation.
- API-created Credential resources persist until an explicit Delete is issued.
  They are never swept by a generation transition. They may carry an authorized
  same-Zone `ownerRef` and participate in owner cascade.
- The `activation-nixos` Provider MUST NOT delete controller-created or
  API-created resources merely because they are absent from the new Nix
  configuration set. Absence from the Nix bundle is not a delete signal for
  resources whose `metadata.managedBy` is `"controller"` or `"api"`.

### Generation transition and cleanup contract

When a new NixOS generation is activated, the `activation-nixos` Provider:

1. **Reads and integrity-verifies** the new resource bundle (SHA-256 digest
   check against the committed bundle digest).

2. **Computes the desired set** of configuration-managed Credential resources
   from the bundle (resources whose `metadata.managedBy = "configuration"`).

3. **Creates or updates** resources in the desired set:
   - A resource in the desired set that does not exist in the store is created
     with generation 1.
   - A resource in the desired set whose stored spec differs from the bundle
     spec receives a full spec replace (generation increments; revision
     increments).
   - A resource in the desired set whose stored spec is identical to the bundle
     spec receives no write (idempotent).

4. **Activates the new generation without blocking on cleanup**: The new
   bundle's activation completes and the `activation-nixos` Provider's own
   status transitions to `Ready` (or `Degraded/pending-cleanup` — see below)
   immediately after step 3. The activation does not wait for removed resources
   to finish deletion.

5. **Issues async Delete for removed resources**: Configuration-managed
   resources currently in the store (those with `metadata.managedBy =
   "configuration"`) that are **absent from the new desired set** receive an
   authorized `delete` (sets `deletionRequestedAt`). These deletes are issued
   asynchronously after the activation completes. The Credential controller
   processes them through its normal finalizer path: `provider-revoke` finalizer
   runs, the credential service calls `RevokeToken` on all active leases, then
   the resource transitions to `phase=Deleted`. Resources with
   `metadata.managedBy = "controller"` or `metadata.managedBy = "api"` are
   never touched by this path.

6. **Retains prior configuration bundles** up to the configured count (default:
   3; range: 1..16, set via Zone configuration). Prior bundles are stored at
   `/etc/d2b/zones/<zone>/resource-bundle.<N>.prev.json`. When the retained
   count would be exceeded, the oldest retained bundle is pruned. There is no
   time-based retention window. Rollback re-activates a retained prior bundle;
   if resources from that bundle were already deleted, they are re-created from
   scratch (prior secrets are not restored; fresh leases are acquired by the
   credential controller after re-creation).

#### Zone and Credential status during cleanup

While removed Credential resources are pending deletion:

- Each removed Credential resource transitions immediately to
  `phase=Degraded` with condition `Cleanup=True`:

  ```yaml
  type: Cleanup
  status: "True"
  reason: nix-generation-removed
  message: "credential removed from nix configuration; pending provider-revoke finalizer"
  observedGeneration: <last generation>
  lastTransitionAt: <deletionRequestedAt>
  ```

- The Zone's `activation-nixos` Provider status reflects a `PendingCleanup`
  condition while any removed configuration-managed Credential (or other
  configuration-managed resource) is still in the store:

  ```yaml
  type: PendingCleanup
  status: "True"
  reason: removed-resources-pending-deletion
  message: "N credential(s) pending deletion from prior nix generation"
  ```

- The Zone `activation-nixos` Provider transitions to `phase=Ready` (clearing
  `PendingCleanup`) only after all removed resources reach `phase=Deleted`.

- Stalled cleanup (e.g. finalizer blocked on a `ProviderUnavailable` condition)
  surfaces as `phase=Degraded` on the Credential resource and on the Provider.
  The Credential controller retries with backpressure and reports the stall via
  the bounded `ProviderUnavailable` condition on the resource.

#### Owner-controller child reconciliation

When a configuration-managed Credential resource is deleted (either by
generation cleanup or by explicit Nix removal), the Credential controller
reconciles and deletes its controller-created children:

- The Credential controller's `finalize` handler lists all `ownerRef =
  Credential/<name>` children in the Zone (e.g., ephemeral lease records).
- Children are deleted child-first in dependency order before the
  `provider-revoke` finalizer is released.
- The Credential controller does not broadly delete all resources that happen
  to share a related name or label; it deletes only the exact `ownerRef`-bound
  children it created.

### Status, errors, and audit for generation transitions and cleanup

#### Status fields specific to cleanup

The `activation-nixos` Provider adds these fields to its own status during a
generation transition:

```yaml
status:
  activationGeneration: 42          # the NixOS generation just activated
  pendingCleanupCount: 2            # number of configuration-managed resources awaiting deletion
  pendingCleanupResourceRefs:       # bounded list (max 64 entries; truncated with note)
    - "Credential/old-keyring"
    - "Credential/deprecated-entra"
  retainedConfigurationCount: 3     # current number of retained prior bundles
  retainedConfigurationMax: 3       # configured maximum (default 3, range 1..16)
```

#### Errors

| Code | Meaning |
| --- | --- |
| `nix-bundle-digest-mismatch` | Bundle SHA-256 does not match declared digest; activation aborted |
| `nix-bundle-schema-invalid` | One or more resource specs fail ResourceTypeSchema validation at activation time |
| `nix-bundle-ref-unresolved` | A resource spec carries a `ResourceRef` that does not resolve in the current Zone at activation time |
| `nix-cleanup-stalled` | A removed resource's finalizer has not completed within the stall threshold; the Zone remains `Degraded/pending-cleanup` |
| `nix-cleanup-revoke-failed` | `RevokeToken` returned a permanent error during cleanup finalizer; operator intervention required |
| `nix-rollback-recreation-failed` | A resource from a retained prior bundle could not be re-created during rollback (e.g. Provider unreachable); fresh lease acquisition will retry |

#### Audit

| Event | Fields retained |
| --- | --- |
| Bundle activated (new NixOS generation) | Zone, `activationGeneration`, `nixConfigGeneration`, bundle digest, resource create/update/skip counts, removed count |
| Configuration-managed resource removed (async delete issued) | Zone, Credential ResourceRef, `activationGeneration`, `deletionRequestedAt`, reason `nix-generation-removed` |
| Cleanup complete | Zone, Credential ResourceRef, final `phase=Deleted`, `activationGeneration`, `cleanupLatencyMs` |
| Cleanup stalled | Zone, Credential ResourceRef, stall duration, last error code |
| Rollback initiated | Zone, from/to `activationGeneration`, `retainedConfigurationCount` |

All audit records exclude: token bytes, key material, prior-generation spec
contents, provider diagnostics, and user-identifying path components.

### Eval/build/runtime tests for Nix configuration and cleanup

#### Eval/build tests (nix-unit, drift gate)

| Test | Validates |
| --- | --- |
| `nix-unit` golden JSON envelope for each Credential example | Spec JSON shape, metadata shape (no management labels in Nix output), field ordering, canonical defaults |
| `nix-unit` assertion-failure for secret-shaped `audience` | `contains_sensitive_shape` eval-time rejection |
| `nix-unit` assertion-failure for `providerRef` not resolving | Eval-time reference validation |
| `nix-unit` assertion-failure for `domainFilter=system` on `credential-secret-service` | Provider-specific placement constraint |
| `nix-unit` assertion-failure for `proactiveWindowMs > maxLeaseLifetimeMs / 2` | Rotation bounds |
| `nix-unit` duplicate-binding conflict assertion | Same `(providerRef, executionRef, userRef, audience)` in same Zone |
| `nix-unit` full bundle JSON with two Credentials: sorted by name, digest present | Bundle sort order and digest field |
| `nix-unit` assertion-failure for missing `d2b.artifacts` entry referenced by Provider `artifactId` | Artifact catalog ID resolution |
| `nix-unit` assertion-failure for `d2b.artifacts.<id>.type != "provider"` for Provider `artifactId` | Artifact catalog type enforcement |
| `nix-unit` assertion-failure for duplicate `d2b.artifacts` ID | Artifact catalog duplicate-ID rejection |
| `make test-drift` (`cargo xtask gen-schemas`) | Schema drift gate; must pass with zero diff |

#### Build tests

| Test | Validates |
| --- | --- |
| Provider-specific schema cross-check: `credential-entra` `audience` charset in bundle | Provider schema validation against generated schema file |
| Bundle digest round-trip: compute digest of sorted resources; verify matches bundle header | Integrity-pinned bundle correctness |
| Artifact catalog round-trip: emitted artifact catalog digest matches header; store paths absent from resource bundle and status | Artifact catalog integrity and store-path privacy |
| Nix store contains no credential-shaped bytes and no store paths in any resource bundle derivation output | Secret-shape and store-path detection covers generated bundle and options modules |

#### Runtime tests (integration, `tests/host-integration/`)

| Test | Validates |
| --- | --- |
| `credential-cleanup-basic`: NixOS generation N has `work-entra`; generation N+1 removes it; resource reaches `phase=Deleted` | Async Delete path end-to-end |
| `credential-cleanup-nonblocking`: generation N+1 activation completes (returns Ready status on new resources) before `work-entra` cleanup finalizer finishes | Activation does not block on cleanup |
| `credential-cleanup-pending-status`: during cleanup, removed resource shows `Cleanup=True`/`nix-generation-removed`; Zone Provider shows `PendingCleanup=True` | Status fields during cleanup |
| `credential-cleanup-stalled`: `credential-secret-service` unavailable during cleanup; removed resource shows `Degraded`/`nix-cleanup-stalled`; recovers after Provider returns | Stall detection and recovery |
| `credential-cleanup-controller-children-preserved`: a controller-created lease record with `ownerRef=Credential/work-entra` is deleted by the Credential controller's finalizer, not orphaned or double-deleted | Owner controller child cleanup |
| `credential-cleanup-no-dynamic-deletion`: a controller-created Credential (`metadata.managedBy = "controller"`) and an API-created Credential (`metadata.managedBy = "api"`) with same name pattern as a removed configuration-managed Credential are NOT deleted by activation-nixos cleanup | Configuration-managed vs controller/API isolation |
| `credential-retained-generation-count`: after cleanup completes, up to `retainedConfigurationMax` (default 3) prior bundles are retained; rollback re-creates removed resource from retained bundle; exceeding the count prunes the oldest retained bundle | Count-based retention and retained-generation rollback |
| `credential-bundle-digest-mismatch`: tampered bundle file causes `nix-bundle-digest-mismatch` and aborts activation | Integrity verification |

## Three-plane credential model and v3 reachability

The v3 baseline distinguishes three credential planes in
`d2b-realm-provider/src/credential.rs` (`b5ddbed6`):

| Plane | v3 type (current symbols) | Evidence class | v3 reachability | Target mapping note |
| --- | --- | --- | --- | --- |
| Azure control plane | `AzureControlPlaneRef` (`tenant_id: OpaqueAzureRef`, `subscription_id: OpaqueAzureRef`, `region: OpaqueAzureRef`) in `d2b-realm-provider/src/credential.rs` | `implemented-and-reachable` (tests pass, used in bundle resolver) | Opaque ref data only; no credential acquisition path | Current type retained directly; `OpaqueAzureRef` validation reused in `credential-entra` and `credential-managed-identity` config |
| Container managed identity | `ManagedIdentityRef` (`client_id: OpaqueAzureRef`) in `d2b-realm-provider/src/credential.rs`; `ProviderWorkloadIdentity::ManagedIdentity` in `d2b-realm-provider/src/types.rs:ProviderGuestdBootstrapContract.workload_identity` | `implemented-and-reachable` (tests pass, ACA config path; `ProviderWorkloadIdentity` used in live ACA bootstrap) | Opaque ref data only; IMDS acquisition is inside ACA container | Current types retained; `credential-managed-identity` Provider formalizes this as a Credential resource; `ProviderGuestdBootstrapContract.workload_identity` uses current `WorkloadId`-backed ACA workload concept (target: Guest-based execution context) |
| d2b-internal session | `SessionCredentialBinding` in `d2b-realm-provider/src/credential.rs`; fields import `d2b_realm_core::{RealmPath, WorkloadId, GatewayId, StreamId}` (current v3 names from `d2b-realm-core`): `realm: RealmPath` (current; target terminology: Zone), `workload: WorkloadId` (current; target terminology: Guest execution context for ACA sandbox), `gateway: GatewayId`, `display_stream: StreamId` | `implemented-and-reachable` (tests pass, gateway mux path) | Gateway-internal; never stored as a resource | **Not migrated.** Current names `RealmPath`/`WorkloadId` are v3 Realm-era symbols; their ADR 0046 successors are Zone and Guest. This binding is a per-session ephemeral gateway artifact, not a Credential ResourceType |

None of the three planes represents a general-purpose Credential ResourceType.
The ADR 0046 Credential ResourceType is a new construct (`ADR-only`) that
replaces the role of the old `CredentialProvider` trait
(`d2b-realm-provider/src/provider.rs:301`, current v3 symbol), which is `implemented-and-reachable`
with only `status()` and `enrollment_valid()`, for the three Credential
Provider families.

The `SessionCredentialBinding` plane is not migrated to a Credential resource:
it is a per-session ephemeral artifact owned exclusively by the gateway runtime
and has no independent lifecycle. It is excluded from the ADR 0046 Credential
ResourceType catalog. Its current v3 field `realm: RealmPath` uses the current
Realm terminology and does not introduce a Zone Credential resource ref.

The `no_secrets_or_credentials: bool` invariant in
`packages/d2b-core/src/realm_workloads_launcher.rs:LauncherMetadataInvariants`
(evidence class: `implemented-and-reachable`; current v3 symbol in the
`RealmWorkloadsLauncherV2Json` artifact — current Realm-era name for the Zone
launcher metadata artifact) provides direct v3 evidence for the zero-secret-bytes
design principle applied at the launcher boundary. The target Credential
ResourceType extends this invariant to all resource/store/status/audit/log
boundaries. Token delivery to the authorized consumer uses a dedicated
end-to-end Noise-encrypted ComponentSession (see §Credential-delivery endpoint
contract); neither the bus intermediary nor any persistent surface stores or
decrypts those records.

The `AzureControlPlaneRef` and `ManagedIdentityRef` planes provide the opaque
non-secret audience reference pattern used by the `credential-entra` and
`credential-managed-identity` Provider specs (§Credential Provider dossiers).

## Required crate layout for Credential Provider packages

Every `packages/d2b-provider-<base>-<implementation>/` crate for a Credential
Provider MUST contain all four of the following paths. Missing any path is a
workspace/package policy failure enforced by `make test-policy` (via
`packages/d2b-contract-tests`):

| Path | Contents |
| --- | --- |
| `src/` | Provider implementation, controller, service handler, binary entry points, and all colocated unit tests (`#[cfg(test)]` modules within `src/` files). One binary per role declared in the dossier's "Process components" table. |
| `tests/` | Hermetic Cargo integration tests (`#[test]` in `tests/*.rs`, no external process or container): ResourceType/controller lifecycle, provider conformance (`d2b-provider-toolkit::conformance::check_provider_conformance`), fault injection (locked/unavailable/interaction-required/generation-mismatch/oversize), canary enforcement (secret/endpoint/object-path canaries absent from all output), delivery-session binding, and placement rejection tests. Run with `cargo test -p d2b-provider-credential-<impl>`. |
| `integration/` | Heavier fixtures and scenarios invoked by existing test orchestration (`make test-integration`, `make test-host-integration`): container-backed Provider service, Host/Guest placement, cross-process d2b-bus routing, provider-system startup/restart/drain, and Nix-generation cleanup/rollback. Files are shell scripts, Nix expressions, or container specs consumed by `tests/integration/containers/` or `tests/host-integration/`; they are NOT run by `cargo test`. |
| `README.md` | All sections listed in §Provider README required sections below. |

### Provider README required sections

Every Provider `README.md` MUST contain these sections in order:

1. **Provider identity** — `providerRef`, managed ResourceType(s), provider
   generation/versioning policy, Zone placement constraints.
2. **Config schema** — `spec` fields (non-secret only), types, defaults,
   constraints, and a worked example using the `d2b.zones.<zone>.resources`
   Nix authoring shape.
3. **ResourceTypes managed** — for each managed ResourceType: lifecycle phases,
   status conditions owned, finalizers owned.
4. **Controllers, services, workers, and binaries** — one subsection per
   component from the dossier "Process components" table: binary name, role,
   domain (user/system), placement constraints.
5. **Placement** — supported `placementBinding` values and rejected values
   with their error codes.
6. **Dependencies and RBAC** — required Zone resources (`executionRef`,
   `consumerRef`, `userRef`), RBAC verbs consumed, consumer Provider
   requirements, cross-resource ordering.
7. **Security, state, and telemetry** — secret isolation model; what the
   Provider persists (opaque handles only; no token bytes); audit events
   emitted; OTEL spans/metrics emitted; canary enforcement.
8. **Build, test, and integration commands** — exact `cargo`/`make` invocations
   for `src/` unit tests, `tests/` integration tests, and `integration/`
   scenarios.
9. **Standalone-repo usage** *(mandatory before first release to a sibling
   flake)* — how to consume the crate outside the monorepo; flake input
   pattern; nixpkgs/toolkit input-follows boilerplate; compatibility
   constraints.

## Credential Provider dossiers

### Provider: `credential-secret-service`

| Field | Value |
| --- | --- |
| providerRef | `Provider/credential-secret-service` |
| Implements | `Credential` |
| Crate | `packages/d2b-provider-credential-secret-service/` |
| Required layout | `src/` (impl + unit tests), `tests/` (hermetic Cargo integration/conformance/fault), `integration/` (container/Host/Guest fixtures), `README.md` (all §Provider README required sections) |
| Main reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-secret-service/src/{lib.rs, tests.rs}` |

#### Description

User-session Secret Service credential provider. The provider process runs
exclusively in the user domain (`scope.domainFilter=user`) of a Host. It is
constructed only for the authenticated user-domain Process (ADR 0046 target:
`Process` ResourceType) authorized by `scope.userRef`, and communicates with an
injected `Oo7SecretServicePort` implementation. The `oo7` port owns all
interaction with the FreeDesktop.org Secret Service D-Bus API (GNOME Keyring,
KWallet, or equivalent) and retains all credential material. In the outer
`d2b.credential.v3` RPC path, only bounded opaque lease metadata leaves this
module (no token bytes in outer DTOs, status, audit, or logs). Token bytes are
released exclusively via the dedicated end-to-end `Noise_KK` delivery record
to the authorized consumer (see §Credential-delivery endpoint contract).

The main-branch reuse source encodes this placement restriction as
`SecretServiceOwner::Userd` (`packages/d2b-provider-credential-secret-service/src/lib.rs`,
evidence class for reuse: main `a1cc0b2d` only). The current v3 baseline crate
`packages/d2b-userd/src/lib.rs` is a **guest exec stub** (service mode exits 78:
`"service mode is not implemented in this build"`; contains only `UserAttachRequest`
and `UserExecSession` guest wire primitives; evidence class: `test-only-or-preview`).
It provides no credential functionality and is not the hosting process for this
Provider. The hosting user-domain Process is fully new work (ADR-only ResourceType).

There is no system-domain path, no host-daemon path, no keyring file path, and no
environment-fallback.

#### Provider resource shape for `credential-secret-service`

`spec.artifactId` is a sibling of `spec.config` on the Provider resource, not
inside `spec.config`. A complete Provider resource declaration for this Provider:

```nix
d2b.zones.dev.resources.credential-secret-service = {
  type = "Provider";
  spec = {
    artifactId = "credential-secret-service-bin";  # d2b.artifacts entry; type = "provider"
    config = {
      collectionAlias = "login";
      maxLeases       = 64;
      lockPolicy      = "fail-closed";
    };
  };
};
```

#### Root `spec.config` schema (bounded, non-secret)

```yaml
# Provider.spec.config — runtime config only; artifactId is Provider.spec.artifactId
collectionAlias: login          # provider-validated Secret Service collection alias
maxLeases: 64                   # maximum concurrent active leases; max 256
lockPolicy: fail-closed         # fail-closed | fail-degraded when keyring is locked
```

#### Supported placement bindings

- `user-agent` only. `host-system` and `guest-agent` are rejected at
  Provider-install validation.

#### Supported operation classes

- `acquire-token`, `refresh-token`, `revoke-token`, `inspect-metadata`.
- `sign-challenge` is not supported; schema rejects it.

#### Process components

| Component | Type | Domain | Binary |
| --- | --- | --- | --- |
| `secret-service-controller` | controller | user | `d2b-provider-credential-secret-service` |

One controller process per user identity per Zone. It is a user-domain Process
under the Host declared in `scope.executionRef`.

#### Credential state machine (provider-internal)

`SecretServiceState`: `Locked | Unlocked`.
`SecretServiceLeaseState`: `Active | Revoked | Expired`.

When state=Locked: AcquireToken returns `credential-provider-unavailable`
(mapped from `SecretServicePortError::Locked`). The status reflects
`ProviderUnavailable=True`, `leaseState=Unknown`.

#### Credential-bound method mapping

| `d2b.credential.v3` method | `Oo7SecretServicePort` call |
| --- | --- |
| `AcquireToken` | `issue_lease(&SecretServiceLeaseRequest)` → `SecretServiceLeaseGrant` |
| `RefreshToken` | `refresh_lease(&SecretServiceLeaseRef)` → `SecretServiceLeaseRenewal` |
| `RevokeToken` | `revoke_lease(&SecretServiceLeaseRef)` → `SecretServiceLeaseRevocation` |
| `InspectMetadata` | `inspect_lease(&SecretServiceLeaseRef)` → `SecretServiceLeaseInspection` |

No port method accepts or returns a password, secret value, token, endpoint,
path, file descriptor, or byte buffer. The `credential_canary` and
`object_path_canary` fields in the main test (`tests.rs`) demonstrate that
those values remain inside the fake port and never appear in provider output.

#### Status derivation

`CredentialReady=True` when `SecretServiceState=Unlocked` and
`leaseState=Active`. `ProviderUnavailable=True` when `SecretServicePortError::Unavailable`
or `SecretServicePortError::Locked` is returned consistently.

#### v3 current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `d2b-realm-provider/src/provider.rs:CredentialProvider` (status-only trait), `credential.rs` (plane types) |
| Evidence class | Minimal trait is `implemented-and-reachable`; full lease model is `ADR-only` in v3 |
| Main reuse source | main `a1cc0b2d` `d2b-provider-credential-secret-service/src/lib.rs`: `Oo7SecretServicePort`, `SecretServiceLeaseRequest/Ref/Grant/Inspection/Renewal/Revocation`, `SecretServiceCredentialProvider`, `SecretServiceCredentialProviderFactory`; `tests.rs`: `FakeOo7Port`, `credential_canary` enforcement, lease lifecycle and locked-state tests |
| Reuse action | copy and adapt (reversion v3 types; replace v2 `CredentialProvider` trait with v3 `d2b.credential.v3` service; replace v2 provider registry with Provider resource) |
| Required delta | v3 contract names/versions, Provider resource/controller descriptor, d2b-bus routing, Zone/Resource placement/scope, async reconciliation integration |
| Excluded main assumptions | v2 EndpointRole/Realm/userd process model; v2 ProviderFactory/ProviderRegistryBuilder; v2 component-session auth and prologue |
| Destination | `packages/d2b-provider-credential-secret-service/` (reused crate name) |

---

### Provider: `credential-entra`

| Field | Value |
| --- | --- |
| providerRef | `Provider/credential-entra` |
| Implements | `Credential` |
| Crate | `packages/d2b-provider-credential-entra/` |
| Required layout | `src/` (impl + unit tests), `tests/` (hermetic Cargo integration/conformance/fault), `integration/` (container/Host/Guest fixtures), `README.md` (all §Provider README required sections) |
| Main reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-entra/src/{lib.rs, tests.rs}` |

#### Description

Entra credential provider for an exact co-located cloud consumer. The injected
`EntraCredentialClient` retains all token material and is implemented in the
same provider-agent process as its SDK consumer (e.g., a cloud-connected guest
provider agent). In the outer `d2b.credential.v3` RPC path, only bounded opaque
lease metadata leaves this module (no token or signature bytes in outer DTOs,
status, audit, or logs). Token and signature bytes are released exclusively via
the dedicated end-to-end `Noise_KK` delivery record to the authorized consumer
(see §Credential-delivery endpoint contract). There
is no key-chain, environment, developer-tool, or alternate credential-chain path.

`EntraCredentialOwner` is `ExactConsumer`: the provider is constructed for a
single exact co-located consumer identified by `consumerRef` in the Credential
spec.

#### Provider resource shape for `credential-entra`

`spec.artifactId` is a sibling of `spec.config`, not inside `spec.config`:

```nix
d2b.zones.dev.resources.credential-entra = {
  type = "Provider";
  spec = {
    artifactId = "credential-entra-bin";  # d2b.artifacts entry; type = "provider"
    config = {
      tenantId           = "2f8e1c3a-1234-5678-9abc-def012345678";
      authorityUrl       = "login.microsoftonline.com";
      maxLeases          = 64;
      interactionPolicy  = "fail-closed";
    };
  };
};
```

#### Root `spec.config` schema (bounded, non-secret)

```yaml
# Provider.spec.config — runtime config only; artifactId is Provider.spec.artifactId
tenantId: "2f8e1c3a-1234-5678-9abc-def012345678"  # opaque Azure tenant GUID; not a ResourceRef
authorityUrl: "login.microsoftonline.com"          # bounded hostname; no secret shape
maxLeases: 64
interactionPolicy: fail-closed    # fail-closed | interaction-required
```

`tenantId` uses the `OpaqueAzureRef` charset restriction
(`^[A-Za-z0-9._-]+$`) from `d2b-realm-provider/src/credential.rs:OpaqueAzureRef`
(current v3 source name) to structurally reject secret-shaped values at parse time.
The field name is `tenantId`, not `tenantIdRef`; it is an opaque inline identifier,
not a `<ResourceType>/<name>` ResourceRef.

#### Supported placement bindings

- `user-agent` (user-domain on Host or Guest);
- `guest-agent` (system-domain on Guest).
- `host-system` is rejected; Entra credentials require a configured
  consumer-agent co-location.

#### Supported operation classes

All five classes: `acquire-token`, `refresh-token`, `revoke-token`,
`sign-challenge`, `inspect-metadata`.

#### Process components

| Component | Type | Domain | Binary |
| --- | --- | --- | --- |
| `entra-controller` | controller | user or system per spec | `d2b-provider-credential-entra` |

#### Credential-bound method mapping

| `d2b.credential.v3` method | `EntraCredentialClient` call |
| --- | --- |
| `AcquireToken` | `issue_lease(&EntraLeaseRequest)` → `EntraLeaseGrant` |
| `RefreshToken` | `refresh_lease(&EntraLeaseRef)` → `EntraLeaseRenewal` |
| `RevokeToken` | `revoke_lease(&EntraLeaseRef)` → `EntraLeaseRevocation` |
| `InspectMetadata` | `inspect_lease(&EntraLeaseRef)` → `EntraLeaseInspection` |

`EntraClientError::InteractionRequired` maps to
`credential-provider-unavailable` (not `credential-operation-denied`): it
signals a transient state, not a policy denial.

`EntraClientState`: `Ready | InteractionRequired`.

#### Three-plane relationship

The `AzureControlPlaneRef` type in v3
(`d2b-realm-provider/src/credential.rs:AzureControlPlaneRef`) provides the
`tenant_id`, `subscription_id`, and `region` non-secret references used in the
`credential-entra` root config. The `OpaqueAzureRef` parse/validation logic is
reused directly. The config schema for this Provider uses the same charset
restriction without carrying a secret.

#### v3 current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `d2b-realm-provider/src/credential.rs:AzureControlPlaneRef`, `OpaqueAzureRef` (implemented-and-reachable); `provider.rs:CredentialProvider` (minimal, implemented-and-reachable) |
| Evidence class | Opaque ref model is reachable; full Entra lease provider is `ADR-only` in v3 |
| Main reuse source | main `a1cc0b2d` `d2b-provider-credential-entra/src/lib.rs`: `EntraCredentialClient`, `EntraLeaseRequest/Ref/Grant/Inspection/Renewal/Revocation`, `EntraCredentialProvider`, `EntraCredentialProviderFactory`; `tests.rs`: `FakeEntraClient`, `credential_canary`/`endpoint_canary` enforcement, `interaction-required` and colocated-consumer tests |
| Reuse action | copy and adapt |
| Required delta | v3 contract versions, Provider resource/descriptor, d2b-bus routing, v3 placement/scope, `OpaqueAzureRef` reuse for config validation |
| Excluded main assumptions | v2 AgentPlacementBinding, v2 EndpointRole/Realm, v2 ProviderFactory |
| Destination | `packages/d2b-provider-credential-entra/` (reused crate name) |

---

### Provider: `credential-managed-identity`

| Field | Value |
| --- | --- |
| providerRef | `Provider/credential-managed-identity` |
| Implements | `Credential` |
| Crate | `packages/d2b-provider-credential-managed-identity/` |
| Required layout | `src/` (impl + unit tests), `tests/` (hermetic Cargo integration/conformance/fault), `integration/` (container/Host/Guest fixtures), `README.md` (all §Provider README required sections) |
| Main reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-managed-identity/src/{lib.rs, tests.rs}` |

#### Description

Host or Guest managed-identity credential provider for an exact co-located SDK
consumer. There is no environment credential chain, no developer-tool fallback,
and no keyring path. The injected `ManagedIdentityCredentialClient` holds all
token material. `ManagedIdentityCredentialOwner` is `ExactSdkConsumer`.

Managed identity runs on Hosts or Guests where an IMDS-compatible endpoint is
available (e.g. Azure VM instance metadata, ACA sidecar IMDS). The provider
config carries only the opaque `ManagedIdentityRef.client_id` (an
`OpaqueAzureRef`, current v3 source type names) identifying the user-assigned managed identity.

#### Provider resource shape for `credential-managed-identity`

`spec.artifactId` is a sibling of `spec.config`, not inside `spec.config`:

```nix
d2b.zones.dev.resources.credential-managed-identity = {
  type = "Provider";
  spec = {
    artifactId = "credential-managed-identity-bin";  # d2b.artifacts entry; type = "provider"
    config = {
      clientId          = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
      imdsEndpointAlias = "azure-imds";
      maxLeases         = 64;
    };
  };
};
```

#### Root `spec.config` schema (bounded, non-secret)

```yaml
# Provider.spec.config — runtime config only; artifactId is Provider.spec.artifactId
clientId: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"   # opaque Azure MI client GUID; not a ResourceRef
imdsEndpointAlias: azure-imds                       # provider-validated closed alias
maxLeases: 64
```

`clientId` is an opaque inline identifier validated against the `OpaqueAzureRef`
charset (`^[A-Za-z0-9._-]+$`). The field name is `clientId`, not `clientIdRef`.

#### Supported placement bindings

- `host-system` and `guest-agent`. The IMDS endpoint is machine-local.
- `user-agent` is rejected: managed identity is not a user-session credential.

#### Supported operation classes

`acquire-token`, `refresh-token`, `revoke-token`, `inspect-metadata`.
`sign-challenge` is not supported.

#### Process components

| Component | Type | Domain | Binary |
| --- | --- | --- | --- |
| `managed-identity-controller` | controller | system | `d2b-provider-credential-managed-identity` |

#### Credential-bound method mapping

| `d2b.credential.v3` method | `ManagedIdentityCredentialClient` call |
| --- | --- |
| `AcquireToken` | `issue_lease(&ManagedIdentityLeaseRequest)` → `ManagedIdentityLeaseGrant` |
| `RefreshToken` | `refresh_lease(&ManagedIdentityLeaseRef)` → `ManagedIdentityLeaseRenewal` |
| `RevokeToken` | `revoke_lease(&ManagedIdentityLeaseRef)` → `ManagedIdentityLeaseRevocation` |
| `InspectMetadata` | `inspect_lease(&ManagedIdentityLeaseRef)` → `ManagedIdentityLeaseInspection` |

`ManagedIdentityClientState`: `Ready | Unavailable`.

#### Three-plane relationship

The `ManagedIdentityRef` type in v3 (`d2b-realm-provider/src/credential.rs:ManagedIdentityRef`)
provides the `client_id` `OpaqueAzureRef` used directly in this Provider's root
config. The ACA path in v3 (`d2bd/src/lib.rs:managed_identity_client_id`,
`d2b-provider-aca/src/lib.rs`) stores the managed identity client ID as an
unwrapped `Option<String>` today (`implemented-and-reachable`). The v3 Provider
config formalizes this as an `OpaqueAzureRef` with the same charset restriction.

#### v3 current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `d2b-realm-provider/src/credential.rs:ManagedIdentityRef` (implemented-and-reachable); `d2bd/src/lib.rs:managed_identity_client_id` (reachable ACA config field); `d2b-provider-aca/src/lib.rs:managed_identity_client_id` (reachable) |
| Evidence class | Opaque ref data model is reachable; full lease provider is `ADR-only` in v3 |
| Main reuse source | main `a1cc0b2d` `d2b-provider-credential-managed-identity/src/lib.rs`: `ManagedIdentityCredentialClient`, `ManagedIdentityLeaseRequest/Ref/Grant/Inspection/Renewal/Revocation`, `ManagedIdentityCredentialProvider`; `tests.rs`: `FakeClient`, canary enforcement, colocated-consumer and unavailable tests |
| Reuse action | copy and adapt |
| Required delta | v3 contract versions, Provider resource/descriptor, placement restriction (system-domain only), d2b-bus routing, `OpaqueAzureRef` config validation |
| Excluded main assumptions | v2 AgentPlacementBinding, v2 EndpointRole/Realm, v2 ProviderFactory |
| Destination | `packages/d2b-provider-credential-managed-identity/` (reused crate name) |

## Current-code fit summary

| Item | Treatment |
| --- | --- |
| Current anchor | `packages/d2b-realm-provider/src/credential.rs`, `provider.rs`; `packages/d2b-core/src/privileges.rs`, `static_invariants.rs`; `packages/d2b-core/src/realm_workloads_launcher.rs:LauncherMetadataInvariants.no_secrets_or_credentials`; `nixos-modules/assertions.nix` |
| Evidence class | `CredentialProvider` trait with status/enrollment is `implemented-and-reachable`; three-plane opaque refs are `implemented-and-reachable`; `ProviderWorkloadIdentity::ManagedIdentity` in `ProviderGuestdBootstrapContract` is `implemented-and-reachable`; `LauncherMetadataInvariants.no_secrets_or_credentials` is `implemented-and-reachable`; v3 `d2b-userd` (`packages/d2b-userd/`) is a guest exec stub only (`test-only-or-preview`; no credential functionality); Credential ResourceType, lease model, operation classes, typed service methods, controller, reconciliation, and async loop are `ADR-only` |
| Behavior retained | Zero secret bytes invariant (structurally enforced); `OpaqueAzureRef` charset/length validation; typed capability denial error shape; bounded/redacted error messages; injected-client pattern keeps secret material in the client process |
| Required delta | Credential ResourceType schema; opaque lease/typed operation service; three Credential Provider crates/controllers; d2b-bus routing; async reconciliation integration; Nix resource declaration/assertion; audit/OTEL |
| Reuse path | Copy/adapt three Credential Provider implementations and their test suites from main as named in each dossier; reuse `OpaqueAzureRef` directly from v3 `d2b-realm-provider/src/credential.rs`; adapt `SecretAccess`/privilege audit shape from `d2b-core/src/privileges.rs` |
| Replacement/deletion | Old `CredentialProvider` trait removed only after all three v3 Providers have tested replacement controllers; old three-plane types remain in v3 baseline and are not removed until Credential resource integration is live |
| Feasibility proof | Main proves three Provider implementations; each has fake-client test suites covering acquire/refresh/revoke/inspect, idempotency, locked/unavailable/interaction-required, canary enforcement, generation validation, and lease cardinality limits |
| Future owner | Work items below |

## Implementation work items

### ADR046-credential-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-credential-001` |
| Dependency/owner | `ADR046-object-001` (resource envelope); `ADR046-identities-001` (types); W0 shared contract root; `d2b-contracts` |
| Current source | `packages/d2b-realm-provider/src/credential.rs`: `OpaqueAzureRef`, parse/deserialize/charset validation, tests |
| Reuse source | main `a1cc0b2d`: `d2b-provider-credential-secret-service/src/lib.rs` types: `SecretServiceLeaseRequest`, `SecretServiceLeaseRef`, `SecretServiceLeaseGrant`, `SecretServiceLeaseInspection`, `SecretServiceLeaseRenewal`, `SecretServiceLeaseRevocation`, `SecretServiceLeaseState`; parallel entra/managed-identity types; `CredentialLease`, `CredentialLeaseRequest`, `CredentialLeaseState` from `d2b-contracts/src/v2_provider.rs` |
| Reuse action | extract and adapt |
| Destination | `packages/d2b-contracts/src/v3/credential.rs` |
| Detailed design | Define `CredentialSpec`, `CredentialStatus`, `CredentialLeaseHandle` (opaque bounded newtype), `CredentialRotationPolicy`, `CredentialRevocationPolicy`, `CredentialScope`, `OperationClass` enum, `CredentialLeaseState`, `PlacementBinding`, `CredentialConditionType`, and all serde/validation/redaction helpers; reuse `OpaqueAzureRef` from v3 baseline directly; enforce zero-secret-bytes charset validation on all string fields at construction |
| Integration | Credential controller, Provider dossier schemas, Nix compiler, and resource store all consume one canonical contract |
| Data migration | Full d2b 3.0 reset; no v2 credential import |
| Validation | Schema golden vectors; charset/length tests; serde unknown-field rejection; `OpaqueAzureRef` round-trip and secret-shape rejection parity; `leaseHandle` and `sourceVersion` opaque newtype tests; status redaction tests |
| Removal proof | Old `CredentialProvider` trait and `CredentialStatus` enum removed only after all v3 Credential Provider controllers consume this contract |

### ADR046-credential-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-credential-002` |
| Dependency/owner | `ADR046-credential-001`; `ADR046-api-001` (resource API); `ADR046-bus-001` (d2b-bus); Credential service owner |
| Current source | `packages/d2b-realm-provider/src/provider.rs:CredentialProvider` (status-only trait); `d2b-contracts/proto/v2/provider_credential.proto` (main: Health, Capabilities, Status, AcquireLease, RefreshLease, RevokeLease) |
| Reuse source | main `a1cc0b2d`: `packages/d2b-contracts/proto/v2/provider_credential.proto` method names |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/proto/v3/credential.proto`; `packages/d2b-credential-service/src/{service.rs, client.rs, server.rs}` |
| Detailed design | Define `d2b.credential.v3` protobuf service with methods: `Status`, `AcquireToken`, `RefreshToken`, `RevokeToken`, `SignChallenge`, `InspectMetadata`; each request carries `credential_ref`, `operation_class`, `operation_id`, `idempotency_key`, `requested_expiry_unix_ms`, `deadline_unix_ms`; `Status`, `RevokeToken`, and `InspectMetadata` responses carry only non-secret metadata (leaseHandle digest, rotationGeneration, sourceVersion, expiresAtUnixMs, state, outcome code); `AcquireToken`, `RefreshToken`, and `SignChallenge` responses additionally include a `delivery_session_params` field carrying the binding contract fields required to establish the end-to-end credential-delivery ComponentSession (see §Credential-delivery endpoint contract); the token bytes themselves travel in the separate Noise-encrypted delivery session, never in the outer DTO; strict unknown-field rejection; bounded message sizes; all record wrappers for delivery sessions must be zeroizing types |
| Integration | d2b-bus routes `d2b.credential.v3` service to the exact credential provider Process identified by `Credential.spec.providerRef`; RBAC checks `use-credential` verb before dispatch; for `AcquireToken`/`RefreshToken`/`SignChallenge`, bus additionally authorizes the credential-delivery endpoint route and forwards opaque Noise-encrypted delivery records without terminating or buffering them; bus never stores or inspects delivery record plaintext |
| Data migration | None |
| Validation | Protocol golden vectors for each method; malformed/oversize rejection; `leaseHandle` opacity tests (secret-canary must not appear in outer DTO or delivery routing metadata); locked/unavailable/denied/expired state tests; delivery session binding contract round-trip; zeroizing record type unit tests; delivery channel never materialized in non-delivery method tests |
| Removal proof | Old v2 `CredentialProviderService` proto removed only after all v3 callers migrate |

### ADR046-credential-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-credential-003` |
| Dependency/owner | `ADR046-credential-001`, `ADR046-credential-002`; `ADR046-reconcile-001`; credential-secret-service owner |
| Current source | `packages/d2b-realm-provider/src/provider.rs:CredentialProvider` (minimal v3 baseline) |
| Reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-secret-service/src/lib.rs` (full implementation); `src/tests.rs` (full test suite including `FakeOo7Port`, lease lifecycle, locked state, canary enforcement, cardinality limits) |
| Reuse action | copy and adapt |
| Destination | `packages/d2b-provider-credential-secret-service/src/{lib.rs, controller.rs, service.rs, main.rs}` (implementation + binary); `packages/d2b-provider-credential-secret-service/tests/{lifecycle.rs, conformance.rs, faults.rs, canary.rs, delivery.rs, placement.rs}` (hermetic Cargo integration); `packages/d2b-provider-credential-secret-service/integration/{container-service.sh, host-placement.nix, cleanup-rollback.sh}` (orchestration fixtures); `packages/d2b-provider-credential-secret-service/README.md` (all §Provider README required sections) |
| Detailed design | Adapt `SecretServiceCredentialProvider` and `SecretServiceCredentialProviderFactory` to use v3 `d2b.credential.v3` service interface; replace v2 `CredentialProvider` trait impl with v3 controller/service handler; adapt `Oo7SecretServicePort` trait (retain all methods unchanged); ensure `SecretServiceOwner::Userd` placement restriction rejects system-domain and guest-agent construction; validate `collectionAlias` against provider-internal charset (not `OpaqueAzureRef`; collection aliases may include spaces); integrate with Provider resource descriptor and controller toolkit; test `credential_canary` never appears in any service response |
| Integration | Target: user-domain Process (ADR-only `Process` ResourceType) under Host (ADR-only `Host` ResourceType); d2b-bus (ADR-only) routes `d2b.credential.v3` calls to this process; Credential controller reconciles status. Current v3 has no user-credential host process: v3 `d2b-userd` (`packages/d2b-userd/src/lib.rs`) is a guest exec stub only (exits 78 in service mode; `UserAttachRequest`/`UserExecSession` guest wire primitives only; evidence class: `test-only-or-preview`); no credential or keyring functionality. This integration path is fully new (ADR-only) work. |
| Data migration | Full reset; no migration from old `CredentialProvider` trait |
| Validation | **`src/` unit** (`#[cfg(test)]` in `src/`): `Oo7SecretServicePort` trait API surface, `SecretServiceOwner` placement guard, `collectionAlias` charset, `lockPolicy` state transitions. **`tests/` Cargo integration** (`cargo test -p d2b-provider-credential-secret-service`): copied test suite from main with v3 type substitutions; add `lifecycle.rs` (acquire/refresh/revoke/inspect end-to-end with `FakeOo7Port`); `conformance.rs` (all 11 `check_provider_conformance` arms pass); `faults.rs` (locked state → `credential-provider-unavailable`, unavailable, cardinality limit); `canary.rs` (`credential_canary` and `object_path_canary` absent from every response, status field, and delivery record); `delivery.rs` (delivery-session binding contract, zeroizing buffer, replay-safe sequence); `placement.rs` (system-domain and guest-agent construction rejected). **`integration/` fixtures**: `container-service.sh` (container-backed Provider service start/stop/drain); `host-placement.nix` (user-domain Host/Process placement in runNixOSTest); `cleanup-rollback.sh` (Nix-generation removal triggers async Delete and Provider-revoke finalizer). |
| Removal proof | Old `d2b-realm-provider:CredentialProvider` trait removed only after this controller and the other two Credential controllers reach full reconcile parity |

### ADR046-credential-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-credential-004` |
| Dependency/owner | `ADR046-credential-001`, `ADR046-credential-002`; `ADR046-reconcile-001`; credential-entra owner |
| Current source | `packages/d2b-realm-provider/src/credential.rs:AzureControlPlaneRef`, `OpaqueAzureRef` (v3 baseline, reachable) |
| Reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-entra/src/lib.rs` (full implementation); `src/tests.rs` (full test suite including `FakeEntraClient`, `credential_canary`/`endpoint_canary`, interaction-required, colocated-consumer, generation-mismatch tests) |
| Reuse action | copy and adapt |
| Destination | `packages/d2b-provider-credential-entra/src/{lib.rs, controller.rs, service.rs, main.rs}`; `packages/d2b-provider-credential-entra/tests/{lifecycle.rs, conformance.rs, faults.rs, canary.rs, delivery.rs, placement.rs}`; `packages/d2b-provider-credential-entra/integration/{container-service.sh, guest-placement.nix, cleanup-rollback.sh}`; `packages/d2b-provider-credential-entra/README.md` (all §Provider README required sections) |
| Detailed design | Adapt `EntraCredentialProvider` and `EntraCredentialProviderFactory` to v3 service; replace v2 `AgentPlacementBinding` with v3 `PlacementBinding` enum (user-agent, guest-agent only; reject host-system); validate `tenantId` config field using `OpaqueAzureRef::parse` from v3 baseline `d2b-realm-provider/src/credential.rs` (note: current v3 source field is named via `AzureControlPlaneRef`; target field name is `tenantId`); retain `EntraCredentialClient` trait unchanged; map `EntraClientError::InteractionRequired` to `credential-provider-unavailable` (not denied); enforce `EntraCredentialOwner::ExactConsumer` so only the declared `consumerRef` may acquire |
| Integration | User-domain or system-domain Process under Guest; d2b-bus routing; Credential controller |
| Data migration | Full reset |
| Validation | **`src/` unit**: `EntraCredentialClient` trait API, `OpaqueAzureRef::parse` on `tenantId`, `EntraCredentialOwner::ExactConsumer` guard, `EntraClientState` transitions. **`tests/` Cargo integration**: `lifecycle.rs` (acquire/refresh/revoke/inspect with `FakeEntraClient`); `conformance.rs` (all conformance arms); `faults.rs` (interaction-required → unavailable, generation-mismatch, colocated-consumer rejection); `canary.rs` (`credential_canary` and `endpoint_canary` absent from every response and delivery record); `delivery.rs` (delivery-session binding, zeroizing, replay-safe); `placement.rs` (host-system placement rejected). **`integration/` fixtures**: `container-service.sh`; `guest-placement.nix` (user-domain and system-domain Process on Guest in runNixOSTest); `cleanup-rollback.sh`. |
| Removal proof | Same as ADR046-credential-003 |

### ADR046-credential-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-credential-005` |
| Dependency/owner | `ADR046-credential-001`, `ADR046-credential-002`; `ADR046-reconcile-001`; credential-managed-identity owner |
| Current source | `packages/d2b-realm-provider/src/credential.rs:ManagedIdentityRef` (v3 baseline, reachable); `d2bd/src/lib.rs:managed_identity_client_id` (reachable ACA config); `d2b-provider-aca/src/lib.rs:managed_identity_client_id` (reachable) |
| Reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-managed-identity/src/lib.rs` (full implementation); `src/tests.rs` (full test suite) |
| Reuse action | copy and adapt |
| Destination | `packages/d2b-provider-credential-managed-identity/src/{lib.rs, controller.rs, service.rs, main.rs}`; `packages/d2b-provider-credential-managed-identity/tests/{lifecycle.rs, conformance.rs, faults.rs, canary.rs, delivery.rs, placement.rs}`; `packages/d2b-provider-credential-managed-identity/integration/{container-service.sh, host-guest-placement.nix, aca-credential-ref.sh, cleanup-rollback.sh}`; `packages/d2b-provider-credential-managed-identity/README.md` (all §Provider README required sections) |
| Detailed design | Adapt `ManagedIdentityCredentialProvider` to v3 service; enforce `ManagedIdentityCredentialOwner::ExactSdkConsumer`; reject user-agent placement (IMDS is machine-local, not user-session); validate `clientId` config using `OpaqueAzureRef::parse` directly from v3 baseline (note: current v3 source field is named via `ManagedIdentityRef.client_id`; target field name is `clientId`); retain `ManagedIdentityCredentialClient` trait unchanged; map `ManagedIdentityClientState::Unavailable` to `credential-provider-unavailable`; `sign-challenge` operation class returns schema-invalid immediately |
| Integration | System-domain Process under Host or Guest; d2b-bus routing; Credential controller; ACA runtime-azure-container-apps Provider may hold a reference to this Credential resource |
| Data migration | Full reset; `d2b-provider-aca` managed_identity_client_id config field migrated to a Credential resource reference in the v3 ACA Provider config |
| Validation | **`src/` unit**: `ManagedIdentityCredentialClient` trait, `OpaqueAzureRef::parse` on `clientId`, `ManagedIdentityCredentialOwner::ExactSdkConsumer` guard, `imdsEndpointAlias` validation, `sign-challenge` schema-invalid fast path. **`tests/` Cargo integration**: `lifecycle.rs` (acquire/refresh/revoke/inspect with `FakeClient`); `conformance.rs`; `faults.rs` (unavailable state, colocated-consumer rejection); `canary.rs` (canary absent from all responses and delivery records); `delivery.rs`; `placement.rs` (user-agent placement rejected). **`integration/` fixtures**: `container-service.sh`; `host-guest-placement.nix` (system-domain Host and Guest placement in runNixOSTest); `aca-credential-ref.sh` (ACA Provider config uses `credentialRef`; raw `managed_identity_client_id` absent); `cleanup-rollback.sh`. |
| Removal proof | `d2b-provider-aca:managed_identity_client_id` raw field removed only after `credential-managed-identity` Provider controller is integrated and the ACA Provider config uses `credentialRef` |

### ADR046-credential-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-credential-006` |
| Dependency/owner | `ADR046-credential-001`, `ADR046-credential-002`; `ADR046-reconcile-001`, `ADR046-reconcile-002`; Credential controller owner |
| Current source | No direct v3 current source; controller is `ADR-only` |
| Reuse source | main `a1cc0b2d`: `packages/d2b-provider-toolkit/src/conformance.rs` (provider conformance pattern); `packages/d2b-provider-toolkit/src/adapter.rs` (controller toolkit pattern) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-credential-<impl>/src/controller.rs`; `packages/d2b-contracts/src/v3/credential_controller.rs` |
| Detailed design | Implement Credential controller handler conforming to `ADR-046-resource-reconciliation` async loop; implement `reconcile`, `observe`, `finalize`, `drain`, and `health` handlers; implement rotation state machine (proactive/on-expiry/on-demand policies); implement `provider-revoke` finalizer execution with `revocation.onOwnerDelete` policy; implement provider-generation-change detection and revocation; implement `CredentialReady`, `RotationDue`, `ProviderUnavailable`, `LeaseRevoked` condition logic; implement bounded idempotency key derivation (Credential UID + rotationGeneration + operation class, no secret material); implement `observeInterval=30s` health check calling `InspectMetadata`; bounded retry/backpressure with typed `credential-rotation-failed` outcome; enforce `MAX_LOCAL_LEASES=256` per controller provider instance |
| Integration | Provider controller Process → d2b-bus → `d2b.credential.v3` service in provider process → injected client/port; status updates through resource API; watch subscription on Credential, Provider, Host/Guest dependency types |
| Data migration | None; v3 reset |
| Validation | Controller state-machine golden vectors; rotation-policy matrix (proactive/on-demand/on-expiry × success/locked/unavailable/expired); finalizer execution tests; provider-generation-change revocation tests; idempotency key derivation tests; observe-interval drift detection test; canary tests confirm zero secret bytes in all controller-written status fields |
| Removal proof | Not applicable (new controller) |

### ADR046-credential-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-credential-007` |
| Dependency/owner | `ADR046-credential-001`; `ADR046-identities-002` (Nix resource compiler); `ADR046-api-001` (resource API, for create/update/delete); `ADR046-reconcile-001` (activation-nixos controller); Nix integrator |
| Current source | `nixos-modules/assertions.nix` (secret-shape assertions, `implemented-and-reachable`). Note: `nixos-modules/options-realms.nix` line 318 contains `d2b.realms.<realm>.relay.credentialRef` — this uses the current v3 **Realm** terminology (current symbol for Zone) and is a relay credential **state-directory path** reference for gateway relay provisioning, not a Zone Credential resource declaration. It is not a source for this work item. No current v3 source exists for Zone Credential resource Nix declarations (ADR-only). |
| Reuse action | adapt |
| Destination | `nixos-modules/options-resources.nix` (generic schema-derived resource options; not type-specific), `nixos-modules/activation-nixos-cleanup.nix` |
| Detailed design | **(1) Schema-derived options and eval-time validation**: implement `d2b.zones.<zone>.resources.<name> = { type = "..."; spec = { ... }; }` as a generic attrset option; an optional `metadata` sub-attr may contain `ownerRef` and/or presentation `labels`/`annotations`. Nix option types, defaults, and inline docs for `spec` fields are generated from the committed `ResourceTypeSchema` JSON (`docs/reference/schemas/v3/credential.json`) and the signed Provider schema — no bespoke options module is maintained separately. `metadata.name` derives from the attr key; `metadata.zone` from the Zone attr key; `apiVersion` defaults to `resources.d2b.io/v3`; `status`/`uid`/`generation`/`revision`/timestamps/`managedBy`/`configurationGeneration` are not authored. Core assigns `metadata.managedBy = "configuration"` and `metadata.configurationGeneration = <N>` to all configuration-managed resources at create/update time; these are never authored in Nix. Eval-time assertions (applied to all entries with `type = "Credential"`): `spec.providerRef` resolves a Provider in the same Zone whose `credentialDomains` includes `spec.scope.domainFilter` and whose `spec.artifactId` (a sibling of `spec.config` on the Provider resource, not inside `spec.config`) resolves an artifact catalog entry of `type = "provider"`; `spec.audience` charset (`^[A-Za-z0-9._:/@-]+$`, max 256); `spec.rotation.proactiveWindowMs < maxLeaseLifetimeMs / 2`; `spec.consumerRef`/`scope.executionRef`/`scope.userRef` resolve declared Zone resources; duplicate `(providerRef, executionRef, userRef, audience)` tuple rejected; `contains_sensitive_shape` on all string fields; Provider-specific placement constraints; `allowedOperations` ⊆ `providerRef.supportedOperations`. **(2) Canonical JSON and bundle emission**: render `spec` attr directly to `spec` object in canonical JSON (no field renames/re-nesting); `metadata` in output contains only derived `name`/`zone` and optionally Nix-authored `ownerRef`/`labels`/`annotations`; `apiVersion` is top-level, not inside `metadata`; `finalizers` is omitted from the Nix-rendered input (core manages finalizers, never accepts them from the bundle); no management labels are emitted by Nix; sort bundle by `(type, name)`; write to `/etc/d2b/zones/<zone>/resource-bundle.json` with digest. **(2b) Artifact catalog emission**: derivation-valued inputs (`d2b.artifacts.<id>`) are compiled separately into an integrity-pinned artifact catalog (`/etc/d2b/zones/<zone>/artifact-catalog.json`) with its own digest header; each entry records `id`, `type`, `sha256`, and bounded closure metadata; store paths are private catalog implementation data absent from the resource bundle, status, audit, and logs; `activation-nixos` verifies both digests before any create/update; missing or wrong-type `artifactId` references fail the NixOS build. **(3) Build-time schema validation**: validate rendered JSON against `docs/reference/schemas/v3/credential.json` and Provider-specific schema; enforce `secretRef` fields use `Credential/<name>` refs; enforce no store paths in any resource bundle or status output; drift gate (`make test-drift`) regenerates schemas with `cargo xtask gen-schemas` and asserts `git diff --exit-code`; Nix options module drift checked in the same gate. **(4) Generation transition and cleanup contract**: activation-nixos controller verifies SHA-256 digest of both resource bundle and artifact catalog, creates/updates desired-set resources, activates without blocking on cleanup, issues async Delete for absent configuration-managed resources (those with `metadata.managedBy = "configuration"`), sets Degraded/Cleanup=True on removed resources; retains up to `retainedConfigurationMax` (default 3, range 1..16) prior bundles; oldest prune when count exceeded; no time-based rollback window. **(5) Configuration-managed vs controller/API isolation**: `managedBy` and `ownerRef` are orthogonal; configuration-managed and API-created resources may each carry an optional same-Zone `ownerRef` and participate in owner cascade. Cleanup checks `metadata.managedBy = "configuration"` before issuing Delete; resources with `metadata.managedBy = "controller"` or `metadata.managedBy = "api"` are never deleted by this path; API-created resources persist until explicit Delete and are never generation-swept. |
| Integration | `activation-nixos` Provider creates/updates Credential resources from emitted envelopes; Credential controller `provider-revoke` finalizer handles cleanup Deletes; owner controller reconciles children of deleted configuration-managed Credentials |
| Data migration | None; v3 reset |
| Validation | **(eval/build)**: nix-unit golden JSON envelope for each example (spec shape, no management labels in Nix output, sort, digest); assertion-failure tests for secret-shaped audience, mismatched providerRef/domainFilter, proactiveWindow > half maxLifetime, duplicate binding tuple, unresolved refs; artifact catalog: assertion-failure for missing `artifactId`, wrong-type `artifactId`, duplicate catalog ID; bundle + artifact catalog digest round-trip; artifact catalog store-path absence from resource bundle and status; Provider-specific schema cross-check; `make test-drift` schema drift gate. **(runtime integration in `tests/host-integration/`)**: `credential-cleanup-basic` (removed resource reaches Deleted); `credential-cleanup-nonblocking` (activation Ready before cleanup finalizer finishes); `credential-cleanup-pending-status` (Cleanup=True on removed resource, PendingCleanup=True on Provider); `credential-cleanup-stalled` (Degraded stall detection and recovery); `credential-cleanup-controller-children-preserved` (ownerRef children cleaned by Credential controller); `credential-cleanup-no-dynamic-deletion` (controller-created Credential with `managedBy = "controller"` not deleted); `credential-retained-generation-count` (up to retainedConfigurationMax bundles retained; rollback re-creates from retained bundle; oldest pruned when count exceeded); `credential-bundle-digest-mismatch` (tampered bundle aborts activation). |
| Removal proof | Not applicable (new module) |

### ADR046-credential-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-credential-008` |
| Dependency/owner | `ADR046-credential-001`, `ADR046-credential-006`; audit/OTEL integrator |
| Current source | `packages/d2b-core/src/privileges.rs:SecretAccess` (implemented-and-reachable); `d2b-realm-provider/src/error.rs:ProviderDiagnostic`/`contains_sensitive_shape` (implemented-and-reachable); `packages/d2b-contract-tests/tests/policy_observability.rs` (reachable audit policy tests) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-credential-<impl>/src/audit.rs`, `telemetry.rs`; `packages/d2b-contract-tests/tests/credential_audit.rs` |
| Detailed design | Implement audit record emission for all credential service methods and controller events using the field set defined in §Audit; implement OTEL span/metric emission using the closed label set in §OTEL and metrics; implement `contains_sensitive_shape` check in all string fields of audit records and metric label values (adapted from `d2b-realm-provider/src/error.rs:contains_sensitive_shape`); add canary-enforcement tests that verify `"secret-canary"`, `"entra-token-canary"`, and `"managed-identity-canary"` values never appear in any audit record, metric label, span attribute, log line, or status field across all Provider test suites |
| Integration | Credential controller and service handlers emit audit records and telemetry through Zone audit/OTEL paths |
| Data migration | None |
| Validation | Canary tests across all three Provider crates; audit record field-presence tests; metric label cardinality tests; span attribute absence tests for forbidden fields |
| Removal proof | Not applicable |
