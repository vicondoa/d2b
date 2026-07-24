# ADR 0046 Provider dossier: `credential-secret-service`

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-credential-secret-service` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `packages/d2b-provider-credential-secret-service/`, Credential controller (user-domain), Nix Credential compiler |
| Depends on | `ADR-046-resources-credential`, `ADR-046-provider-model-and-packaging`, `ADR-046-componentsession-and-bus`, `ADR-046-resource-reconciliation`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support`, `ADR-046-resource-api-and-authorization`, `ADR-046-components-processes-and-sandbox` |
| Supersedes | v2 `SecretServiceCredentialProvider` / `SecretServiceCredentialProviderFactory` in `d2b-realm-provider`; v2 `CredentialProvider` trait |
| Work items | ADR046-cred-ss-003 (primary), ADR046-cred-ss-001, ADR046-cred-ss-002, ADR046-cred-ss-004, ADR046-cred-ss-005, ADR046-cred-ss-006 |

---

## 1. Provider identity

| Field | Value |
| --- | --- |
| `providerRef` | `Provider/credential-secret-service` |
| Implements | `Credential` ResourceType |
| Placement bindings | `user-agent` only |
| System/user domain | `user` only |
| Crate | `packages/d2b-provider-credential-secret-service/` |
| Binary | `d2b-provider-credential-secret-service` |
| Component type | `controller` |
| Cardinality | One controller process per `(Zone, User, executionRef)` triple |
| Provider generation | Monotonic; incremented on package version, config, or descriptor change |
| Compatibility | `credential` Provider API major exact; minor additive only |

**Zone placement constraint**: `scope.executionRef` resolves a `Host/<name>` or
`Guest/<name>` in the same Zone. Both Host and Guest execution contexts are
supported when `scope.domainFilter=user`.

**Domain restriction**: this Provider supports `domainFilter=user` only. Any
Credential or Provider resource with `domainFilter=system` referencing this
Provider fails at Nix eval time, Provider install validation, and controller
construction. There is no system-domain path, no host-daemon path, no keyring
file path, and no environment-fallback.

---

## 2. Crate layout

Every `packages/d2b-provider-credential-secret-service/` path is enforced by
the workspace policy gate (`make test-policy`). Missing any path is a policy
failure.

```text
packages/d2b-provider-credential-secret-service/
  src/
    lib.rs         — port trait, lease DTOs, owner placement guard, and unit tests
    controller.rs  — Credential controller: reconcile/observe/finalize/drain/health handlers
    service.rs     — d2b.credential.v3 service handler bound to controller state
    main.rs        — binary entry point; constructs process context and drives controller
  tests/
    lifecycle.rs   — acquire/refresh/revoke/inspect end-to-end with FakeOo7Port
    conformance.rs — all check_provider_conformance arms pass
    faults.rs      — locked state → credential-provider-unavailable; unavailable; cardinality limit
    canary.rs      — credential_canary and object_path_canary absent from every response, status field, and delivery record
    delivery.rs    — delivery-session binding contract; zeroizing buffer; replay-safe sequence
    placement.rs   — user-agent on Host and Guest accepted; system-domain and guest-agent (system-domain on Guest) rejected
  integration/
    container-service.sh    — container-backed Provider service start/stop/drain
    host-placement.nix      — user-domain Host/Process placement (executionRef=Host) in runNixOSTest
    guest-placement.nix     — user-domain Guest/Process placement (executionRef=Guest) in runNixOSTest
    cleanup-rollback.sh     — Nix-generation removal triggers async Delete and provider-revoke finalizer
    README.md               — integration fixture descriptions and invocation instructions (optional; root README.md is the mandated policy gate)
  README.md                 — all §Provider README required sections (see §17)
  Cargo.toml
```

---

## 3. Provider resource Nix shape

`spec.artifactId` is a sibling of `spec.config` on the Provider resource.

```nix
d2b.zones.dev.resources.credential-secret-service = {
  type = "Provider";
  spec = {
    artifactId = "credential-secret-service-bin";   # d2b.artifacts entry; type = "provider"
    config = {
      collectionAlias = "login";   # provider-validated Secret Service collection alias
      maxLeases       = 64;        # max concurrent active leases; range 1..256
      lockPolicy      = "fail-closed";  # fail-closed | fail-degraded
    };
  };
};

d2b.artifacts.credential-secret-service-bin = {
  package = pkgs.d2b-provider-credential-secret-service;
  type    = "provider";
};
```

`artifactId` is a plain bounded identifier matching `^[a-z][a-z0-9-]*$`,
not a `*Ref` field. `Artifact` is not a ResourceType.

---

## 4. Root `spec.config` schema

Runtime configuration only. `artifactId` is a sibling on the Provider spec, not
inside `config`. No `config` field accepts a secret-shaped value.

| Field | Type | Default | Rules |
| --- | --- | --- | --- |
| `collectionAlias` | string | `"login"` | Provider-validated Secret Service collection alias; charset: printable ASCII except `"`, `\`, `\n`, `\r`, `\0`; max 128 chars; not an `OpaqueAzureRef` (collection aliases may include spaces); must not be empty |
| `maxLeases` | u32 | `64` | Maximum concurrent active leases in the local lease table; range 1..=256; matches `MAX_LOCAL_LEASES` in the crate |
| `lockPolicy` | enum | `"fail-closed"` | `fail-closed` = return `credential-provider-unavailable` when keyring is locked; `fail-degraded` = set `ProviderUnavailable=True` condition and continue with degraded status |

Provider-specific constraint: `credential-secret-service` does not have a
`tenantId`, `authorityUrl`, `clientId`, or `imdsEndpointAlias` field. Those
fields belong on other credential Providers and are rejected here.

Nix eval-time secret-shape assertion (`contains_sensitive_shape`) runs on all
string fields of this `config` block before the NixOS build completes.

---

## 5. Credential resource spec (as consumed by this Provider)

The `Credential` ResourceType spec is defined canonically in
`ADR-046-resources-credential`.

Normative D089 spec layering: Credential base fields are ResourceType base
`spec.*` fields, including `spec.providerRef`, `audience`, `scope`,
`allowedOperations`, `rotation`, and `revocation`. This Provider's desired-only
extension is the canonical `spec.provider = { schemaId:
"credential-secret-service.d2bus.org/Credential/spec", schemaVersion, settings }`
envelope; it is manifest-registered/signed, strict deny-unknown, bounded, versioned
and digested, validated against `spec.providerRef` at Nix build and API
admission, implementation-only, and may not shadow base fields. Shared fields
are promoted to the Credential base. The Provider implements the exact base
Credential spec/status version/fingerprint, accepts the canonical minimal valid
base Spec, and rejects unsupported optional base capabilities only through its
signed capability matrix and provider-neutral `unsupported-capability`.
`spec.provider` aligns with `status.provider`; generic CLI/controllers operate on
the base spec and base status only. No secret
bytes or credential material are allowed in any spec layer, including
`spec.provider.settings`; credential bytes are delivered only over Noise_KK
sessions.

Fields specific to this Provider:

### 5.1 Full Nix example

```nix
d2b.zones.dev.resources.local-keyring = {
  type = "Credential";
  metadata = {
    labels = { "purpose" = "user-session"; };
  };
  spec = {
    providerRef = "Provider/credential-secret-service";
    scope = {
      executionRef = "Host/host-system";
      domainFilter = "user";
      userRef      = "User/alice";
    };
    audience            = "user-session";
    consumerRef         = "Provider/shell-terminal";   # optional
    allowedOperations   = [ "acquire-token" "refresh-token" "revoke-token" "inspect-metadata" ];
    rotation = {
      policy              = "on-demand";
    };
    revocation = {
      onOwnerDelete          = "immediate";
      onProviderGeneration   = "immediate";
    };
  };
};
```

### 5.2 Spec field reference (Provider-specific constraints)

| Field | Provider constraint |
| --- | --- |
| `providerRef` | Must be `Provider/credential-secret-service` for this Provider |
| `scope.domainFilter` | Must be `"user"`; `"system"` and `"guest"` rejected at eval time and Provider construction |
| `scope.userRef` | Required (domainFilter=user rule) |
| `scope.executionRef` | Must resolve a `Host/<name>` or `Guest/<name>` in the same Zone; both supported with `domainFilter=user` |
| `audience` | Provider-validated non-secret opaque value; charset `^[A-Za-z0-9._:/@-]+$`, max 256 chars |
| `allowedOperations` | Subset of `{ acquire-token, refresh-token, revoke-token, inspect-metadata }`; `sign-challenge` is not supported and schema-rejects it at eval and Provider install |
| `consumerRef` | Optional; restricts the acquiring Provider; no arbitrary component fallback |

### 5.3 Canonical rendered ResourceSpec JSON

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "Credential",
  "metadata": {
    "name": "local-keyring",
    "zone": "dev",
    "labels": { "purpose": "user-session" },
    "annotations": {},
    "ownerRef": null
  },
  "spec": {
    "providerRef": "Provider/credential-secret-service",
    "scope": {
      "executionRef": "Host/host-system",
      "domainFilter": "user",
      "userRef": "User/alice"
    },
    "audience": "user-session",
    "consumerRef": "Provider/shell-terminal",
    "allowedOperations": ["acquire-token", "inspect-metadata", "refresh-token", "revoke-token"],
    "rotation": {
      "policy": "on-demand"
    },
    "revocation": {
      "onOwnerDelete": "immediate",
      "onProviderGeneration": "immediate"
    }
  }
}
```

`allowedOperations` is sorted alphabetically and deduplicated in the emitted
JSON.

---

## 6. Credential controller

### 6.1 Controller descriptor

```yaml
providerId:          Provider/credential-secret-service
controllerType:      Credential
resourceTypes:       [Credential]
watchSelectors:
  - resourceType: Credential
    providerRefFilter: Provider/credential-secret-service
  - resourceType: Provider
    nameFilter: credential-secret-service
dependencySelectors:
  - resourceType: Provider
    relationship: providerRef
  - resourceType: Host
    relationship: scope.executionRef
  - resourceType: Guest
    relationship: scope.executionRef
  - resourceType: User
    relationship: scope.userRef
ownerChildTriggers:  [owned-resource-changed]
reconcileConcurrency: 8
maxPendingResources: 256
finalizers:
  - credential.d2bus.org/provider-revoke
observeInterval:     30s
```

### 6.2 Controller handler methods

All handlers are async. They return a typed `ReconcileResult` carrying the
next disposition: `Ok(Ready)`, `Degrade(reason, requeue_at)`, `Fail(outcome)`,
`Finalizing`, or `Blocked(reason, requeue_at)`.

#### Currency and upgrade (D091)

The controller implements `assess_update`, `plan_upgrade`, and
`execute_upgrade`. A Provider generation or signed artifact generation/digest
change updates universal `status.update` with `state: UpdateAvailable` or
`state: UpgradeRequired`, `reasons` including `ProviderGenerationChanged` or
`ArtifactChanged`, observed/target generation or digest IDs,
`disruption: Reload` or `disruption: Restart` for the credential component
realization, `preserveState: true`, bounded `owned`/`dependencies`, and
`lastAssessedAt`. Disruptive changes MUST return `UpgradeRequired` rather than
applying in place; non-disruptive changes reconcile normally. Credential
rotation is not an upgrade and remains the §6.5 flow. `status.update` MUST NOT
contain secret bytes, tokens, or lease material; only bounded non-secret
generation/digest IDs and lease metadata already permitted by the Credential
base may appear. Token delivery remains solely over `Noise_KK`.

#### Expedited reconcile on mutation (D090)

For `Create`, `UpdateSpec`, and `Delete` with `waitForReconcile`, the controller
MUST perform no Secret Service effect, finalizer change, or status mutation
until core supplies `CommittedRevisionProof {resourceUid,generation,revision,operationId}`.
Abort before that proof has no effect. After durable commit, the commit is never
rolled back if the reconcile pass times out. The response returns the committed
object, post-pass projected layered status, disposition
(`Converged|Progressing|Blocked|UpgradeRequired|Failed`), and
`statusPersistence: pending|committed`. Effect idempotency keys derive from
`(UID,generation,revision,operationId)` and use the same per-resource
single-flight priority lane.

#### `reconcile(resource, context)`

Called when a Credential resource assigned to this Provider has been created,
updated, or depends-on-trigger fires. Steps:

1. Verify the resource's `spec.providerRef` matches this Provider instance
   generation; reject if generation mismatch.
2. Validate `scope.domainFilter = "user"` and `scope.executionRef` resolves a
   ready `Host/<name>` or `Guest/<name>` in the same Zone. If not, set
   `phase=Degraded`, `ProviderUnavailable=True`, return `Degrade`.
3. If `spec.allowedOperations` includes `acquire-token` and `leaseState ∉ {Active}`:
   a. Derive idempotency key:
      `sha256(credential_uid || rotation_generation.to_le_bytes() || b"acquire")`,
      encoded as hex; max 64 chars; no secret material.
   b. Call `Oo7SecretServicePort::issue_lease(&SecretServiceLeaseRequest)`.
   c. On grant: write status (`leaseState=Active`, `leaseHandle`, `sourceVersion`,
      `expiresAtUnixMs`, `rotationGeneration`, `issuedAtUnixMs`, `CredentialReady=True`).
   d. On `SecretServicePortError::Locked`: apply `lockPolicy`; set
      `ProviderUnavailable=True`, `leaseState=Unknown`. Return
      `Degrade("keyring-locked", requeue_at)`.
   e. On `SecretServicePortError::Unavailable`: set `ProviderUnavailable=True`.
      Return `Degrade("provider-unavailable", requeue_at)`.
   f. On `SecretServicePortError::CompletionUnknown`: set
      `ProviderUnavailable=True`. Do NOT auto-retry with the same record.
      Return `Degrade("completion-unknown", requeue_at)`.
4. If `rotation.policy = "proactive"` and `RotationDue=True` condition is set:
   execute the rotation algorithm (§6.5).
5. Commit status mutation batch with optimistic revision precondition.

#### `observe(resource, context)`

Called every `observeInterval` (30 s) to detect out-of-band lease changes. Steps:

1. Call `Oo7SecretServicePort::inspect_lease(&SecretServiceLeaseRef)`.
2. On success: update `leaseState`, `expiresAtUnixMs`, `lastRefreshedAt`; if
   `leaseState` changed from `Active` to `Revoked/Expired`, clear
   `CredentialReady`, set `LeaseRevoked` or check expiry.
3. If `policy=proactive` and remaining lifetime < `proactiveWindowMs`: set
   `RotationDue=True`, requeue for rotation.
4. On `SecretServicePortError::Locked`: update status per `lockPolicy`; requeue.
5. Commit status batch.

#### `finalize(resource, context)`

Called when `deletionRequestedAt` is set and `credential.d2bus.org/provider-revoke`
finalizer is present. Canonical sequence: revoke/drain → delete scoped Process
(if applicable) → clear `provider-revoke` → (core) event-only `Deleted` revision
+ row/index removal → (audit subsystem) closure audit with dedup.

1. If `spec.revocation.onOwnerDelete = "immediate"`:
   a. If `leaseState=Active` or `leaseState=Unknown`: call
      `Oo7SecretServicePort::revoke_lease(&SecretServiceLeaseRef)`.
   b. On `Revoked` or `AlreadyRevoked`: set `leaseState=Revoked`,
      `LeaseRevoked=True`, commit status.
   c. On `SecretServicePortError::Unavailable` or `Locked`: set
      `ProviderUnavailable=True`; return `Blocked("provider-unavailable",
      requeue_at)`.
2. If `spec.revocation.onOwnerDelete = "drain-leases"`:
   - If `leaseState=Expired`: proceed.
   - If `leaseState=Active`: set `LeaseRevoked=False` condition; requeue until
     `expiresAtUnixMs` passes.
3. If `leaseState=Revoked` or `leaseState=Expired`: proceed without further
   network calls (terminal state satisfies the finalizer).
4. Emit bounded revoke outcome audit record (Zone, `resource_name_digest`,
   operation class, rotationGeneration, revocation outcome code; no token
   bytes, paths, or provider diagnostics). The controller MUST NOT emit a
   resource-deleted closure audit; that is appended by the audit subsystem after
   the core event-only `Deleted` revision with dedup/exactly-once recovery.
5. Issue a controller-initiated Delete for the scoped `Process` resource owned
   by `Provider/credential-secret-service` for this `(Zone, User, executionRef)`
   triple (if one was created). This is a regular resource delete via the
   controller's write authority; the Process controller completes it through its
   own lifecycle before the Credential row is removed.
6. Clear the `credential.d2bus.org/provider-revoke` finalizer. After this, core
   writes the event-only `Deleted` revision and removes the Credential row and
   indexes atomically.

#### `drain(resource, context)`

Called before a Provider generation change. Steps:

1. Apply `spec.revocation.onProviderGeneration` policy.
2. `immediate`: call `revoke_lease`; on success, set `leaseState=Revoked`.
3. `drain-leases`: do nothing; leases expire by natural deadline.
4. Emit bounded audit record.

#### `health() -> ControllerHealth`

Returns `{ state: Ready | Degraded, provider_process_reachable: bool,
active_leases: u32, locked_count: u32 }`. Does not expose collection object
paths, user-identifying data, or token bytes. Core reads this health response
to aggregate Provider-level status; the controller writes only its scoped
Credential and Process health, never Provider-level status directly.

### 6.3 Service methods (`d2b.credential.v3`)

The controller process serves the `d2b.credential.v3` protobuf/ttrpc service
routed through d2b-bus. All methods are async.

| Method | Port call | Outer DTO | Sensitive output |
| --- | --- | --- | --- |
| `Status` | `inspect_lease` + `state` | `CredentialStatusResponse` (leaseState, rotationGeneration, sourceVersion, expiresAtUnixMs, placementBinding) | none |
| `AcquireToken` | `issue_lease` | `AcquireTokenResponse` (leaseHandle, sourceVersion, rotationGeneration, expiresAtUnixMs) | raw token bytes in dedicated Noise_KK delivery record |
| `RefreshToken` | `refresh_lease` | `RefreshTokenResponse` (leaseHandle, sourceVersion, rotationGeneration, new expiresAtUnixMs) | raw token bytes in dedicated Noise_KK delivery record |
| `RevokeToken` | `revoke_lease` | `RevokeTokenResponse` (Revoked or AlreadyRevoked, revokedAtUnixMs) | none |
| `InspectMetadata` | `inspect_lease` | `InspectMetadataResponse` (leaseState, rotationGeneration, sourceVersion, expiresAtUnixMs) | none |

`sign-challenge` is not implemented. Any request with `operationClass =
sign-challenge` returns `credential-schema-invalid` immediately without
consulting the port.

Every method:
- rejects a request where the authenticated subject is not the declared
  `consumerRef` (when set) and RBAC `use-credential` is denied;
- rejects an operation class not in `spec.allowedOperations`;
- returns a stable closed error code, never provider-internal diagnostics;
- carries operation/idempotency/correlation IDs from d2b-bus context;
- enforces a per-call deadline propagated from the d2b-bus context.

### 6.4 `Oo7SecretServicePort` trait (injected interface)

```rust
#[async_trait]
pub trait Oo7SecretServicePort: Send + Sync {
    async fn state(&self) -> Result<SecretServiceState, SecretServicePortError>;

    async fn issue_lease(
        &self,
        request: &SecretServiceLeaseRequest,
    ) -> Result<SecretServiceLeaseGrant, SecretServicePortError>;

    async fn inspect_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseInspection, SecretServicePortError>;

    async fn refresh_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseRenewal, SecretServicePortError>;

    async fn revoke_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> Result<SecretServiceLeaseRevocation, SecretServicePortError>;
}
```

None of these methods accepts or returns a password, secret value, token,
endpoint, path, file descriptor, or byte buffer. The port owns all interaction
with the FreeDesktop.org Secret Service D-Bus API (GNOME Keyring, KWallet, or
equivalent) and retains all credential material. Only bounded opaque lease
metadata crosses the `Oo7SecretServicePort` boundary.

The production implementation of this trait is the `oo7`-backed D-Bus adapter
in `src/lib.rs`. It operates exclusively over the inherited portal FD delivered
in the LaunchTicket; it must not call `Session::new()`, connect to any ambient
D-Bus address or socket path, or read `DBUS_SESSION_BUS_ADDRESS`. Tests use
`FakeOo7Port` (§10.2).

### 6.5 Rotation algorithm

For `rotation.policy = "proactive"`:

1. Controller's `observe` handler sets `RotationDue=True` when
   `now_unix_ms + proactiveWindowMs >= expiresAtUnixMs`.
2. `reconcile` detects `RotationDue=True` and calls `issue_lease` with a new
   idempotency key:
   `sha256(credential_uid || (rotation_generation + 1).to_le_bytes() || b"acquire")`.
3. On grant: increment `rotationGeneration`; store new `leaseHandle` and
   `expiresAtUnixMs`; clear `RotationDue`; commit status; emit rotation audit
   record (Zone, `resource_name_digest`, trigger reason, old
   rotationGeneration, new rotationGeneration, outcome code).
4. Old lease remains valid until the new lease is confirmed active.
5. A duplicate `issue_lease` with the same idempotency key returns the same
   grant without double-issuing.
6. On failure: bounded retry under `requeue-at` disposition; after retry
   exhaustion, `phase=Failed`, `outcome=rotation-failed`.

For `rotation.policy = "on-demand"`: controller does not auto-rotate. Consumer
explicitly calls `RefreshToken` or `RevokeToken`+`AcquireToken`. Controller only
monitors expiry for status conditions.

For `rotation.policy = "on-expiry"`: controller calls `issue_lease` only after
`leaseState` transitions to `Expired`.

### 6.6 Provider-internal state machine

```
SecretServiceState:      Locked | Unlocked
SecretServiceLeaseState: Active | Revoked | Expired
```

The state machine is process-local and reconstructed from the owning
`Credential.status`, core Operation ledger entries, and live Secret Service
observation after restart. Under D087 the Provider declares **no Provider state
Volume**: bounded non-secret operational state belongs in status by default, and
this Provider does not satisfy the storage-need test. ProviderStateSet remains
only the optional query-time grouping of declared state Volumes and is empty for
`credential-secret-service` (D086, superseded by D087).

When `SecretServiceState = Locked`:
- `AcquireToken` returns `credential-provider-unavailable`
  (from `SecretServicePortError::Locked`).
- Status reflects `ProviderUnavailable=True`, `leaseState=Unknown`.
- Controller applies `lockPolicy`:
  - `fail-closed`: return error immediately on every call while locked.
  - `fail-degraded`: set `phase=Degraded` + `ProviderUnavailable=True`; requeue.
- The port is polled at `observeInterval` to detect unlock.

When `SecretServiceState = Unlocked`:
- All service methods proceed normally.
- `CredentialReady=True` when `leaseState=Active`.

---

## 7. User-agent placement

### 7.1 Process component

| Component | Type | Domain | Binary | Cardinality |
| --- | --- | --- | --- | --- |
| `secret-service-controller` | controller | user | `d2b-provider-credential-secret-service` | One per `(Zone, User, executionRef)` triple |

The controller process is a user-domain `Process` resource under the Host or
Guest declared in `scope.executionRef`. It is launched and supervised by
`Provider/system-systemd`.

### 7.2 `SecretServiceOwner`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceOwner {
    /// The exact user's d2b user-domain process.
    Userd,
}
```

The owner is `SecretServiceOwner::Userd`. The controller is constructed only
for a user-domain Process authorized by the matching `scope.userRef`. Construction
for any other owner type fails with `SecretServiceProviderError::InvalidConsumer`.

`SecretServiceProviderError::NotColocated` is returned when the consumer
Provider and Secret Service provider are not co-located in the same user session.

The consumer Provider identity (from `spec.consumerRef`, checked by d2b-bus RBAC
against the authenticated ComponentSession subject) and the user identity (from
`spec.scope.userRef`, checked against the authenticated user-domain context of
the requesting Process) are enforced independently by d2b-bus; neither derives
from the other.

### 7.3 Canonical controller Process resource template

One Process resource is created per `(Zone, User, executionRef)` triple.
`metadata.ownerRef` is `Provider/credential-secret-service`; the controller
creates and manages it during provider reconciliation and does not expose it in
the public Nix authoring surface.

`spec.template` is a plain string naming the component descriptor. The component
descriptor for `secret-service-controller` declares the required authenticated
`dbus-session` FD attachment; the Process controller validates this attachment
and threads it into the LaunchTicket's inherited FD table before the process is
spawned. `spec` has no generic `attachments` array. Stable service identities
are separate owned `Endpoint` resources, not inline Process fields.

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  # Generated name; pattern: credential-ss-<credential-uid-prefix>; no secret component.
  name: credential-ss-<generated>
  zone: <zone>
  ownerRef: Provider/credential-secret-service
spec:
  providerRef: Provider/system-systemd
  executionRef: <Host/<name> | Guest/<name>>  # mirrors Credential spec.scope.executionRef
  domain: user
  userRef: <User/<name>>                      # mirrors Credential spec.scope.userRef
  processClass: controller
  template: secret-service-controller         # component descriptor ID; plain string
  sandbox:
    namespaceClasses: [mount, pid, ipc]       # Provider/system-systemd rejects user namespace class;
                                              # same-UID execution is guaranteed by spec.userRef
    capabilityClasses: []                     # zero capability classes; no host caps
    seccompClass: strict                      # closed strict seccomp class
    noNewPrivileges: true
    startRoot: false                          # does not require elevated start
    environmentClass: minimal                 # no inherited environment variables
    readOnlyRoot: true                        # read-only root filesystem
  budget:
    cpu:
      request: "10m"                          # 10 millicores baseline
      limit: "500m"                           # 0.5 core ceiling
    memory:
      request: "16Mi"                         # 16 MiB baseline
      limit: "64Mi"                           # 64 MiB hard ceiling
    pids:
      limit: 32                               # max 32 PIDs in the cgroup
    fds:
      limit: 64                               # max 64 open file descriptors
  networkUsage: null                          # no ambient network; all comms via d2b-bus portal
  readiness:
    initialDelay: 2s
    timeout: 30s
    failureThreshold: 5
    successThreshold: 1
    class: provider-defined                   # provider verifies d2b-bus registration
```

The controller's stable credential service identity is a separate owned
`Endpoint` resource:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: credential-ss-<generated>-credential-service
  zone: <zone>
  ownerRef: Provider/credential-secret-service
spec:
  providerRef: Provider/credential-secret-service
  producerRef: Process/credential-ss-<generated>
  endpointClass: service
  transport: unix
  purpose: credential-secret-service.d2bus.org/credential-service
  serviceFingerprint: credential.d2bus.org/CredentialService.v3
  locality: host-local
  visibility: zone
  attachmentPolicy: component-session
  consumerPolicy: same-zone-authorized
  lifecyclePolicy: recycle-with-producer
status:
  readiness: Ready
  observedProducerGeneration: 1
  observedResourceGeneration: 1
  endpointGeneration: 1
  connectionAvailability: available
  leaseAvailability: lease-required
```

## Endpoint resources (D092)

`Provider/credential-secret-service` conforms to the standard `Endpoint` base
schema. The stable `d2b.credential.v3`-class ComponentSession service identity is
an owned `Endpoint` resource with `producerRef`; consumers use
`Endpoint/<name>`. Endpoint spec/status never carries Secret Service object
paths, D-Bus addresses, fd numbers, lease handles with authority, token bytes,
passwords, credential values, or other secrets. Resolution occurs only through
an authorized EffectPort/LaunchTicket; unauthorized resolution returns
`endpoint-resolve-denied`. Producer restart bumps
`Endpoint.status.endpointGeneration`, causing consumers to observe
`dependency-changed` and reacquire through a fresh authorized ticket.

## Retained opaque handles (D092 promotion test)

- pidfds for the user-domain controller Process are supervision handles, not
  stable endpoint identities.
- The pre-opened D-Bus fd index is a LaunchTicket-local attachment slot and
  remains opaque.
- `OwnedTransport`, ComponentSession IDs, named streams, and the sensitive
  Noise_KK delivery session handle are in-memory per-session capabilities.
- `leaseHandle` and `operationId` values remain bounded, non-secret,
  non-authorizing status/idempotency handles and are revalidated before use.

### 7.4 Injected port: pre-opened D-Bus FD

The controller is constructed with an injected `Arc<dyn Oo7SecretServicePort>`.
The production implementation wraps the pre-opened Secret Service connection
port delivered by the fixed user supervisor as the user portal FD in the
LaunchTicket's inherited FD table. The component descriptor for
`secret-service-controller` declares a required authenticated `dbus-session`
FD attachment; the Process controller validates this attachment and threads it
into the LaunchTicket before the process is spawned. There is no
`$DBUS_SESSION_BUS_ADDRESS` lookup, no ambient socket path discovery, no
Volume-mounted socket path, no environment credential chain, no keyring file
path, and no host-daemon fallback. The user portal FD is the only path from
the controller to the keyring; absent it, the controller fails closed.

None of the `Oo7SecretServicePort` methods accepts or returns a password, secret
value, token, endpoint, path, or byte buffer; only opaque non-secret identifiers
and bounded metadata cross the port boundary.

---

## 8. Noise KK delivery: end-to-end token delivery

Token bytes (`AcquireToken`, `RefreshToken`) are never present in the outer
`d2b.credential.v3` response DTO, resource status, audit records, OTEL spans,
or any log line.

### 8.1 Session profile

The delivery channel MUST use `Noise_KK_25519_ChaChaPoly_SHA256`. `Noise_NN`
is forbidden for sensitive output delivery.

- **Credential Provider key**: registered at Provider installation; d2b-bus
  holds only the public key.
- **Consumer Provider key**: extracted from the consumer Provider's signed
  component descriptor; the consuming component and Process identity are derived
  from the consumer Provider's Role/RoleBinding.

### 8.2 Delivery session binding

Each delivery session binds these fields (from
`ADR-046-componentsession-and-bus` §Credential-delivery endpoint contract):

| Field | Source |
| --- | --- |
| `credentialRef` | `Credential/<name>` |
| `credentialUID` | Credential resource UID |
| `credentialGeneration` | Credential generation at delivery time |
| `consumerProviderRef` | `Provider/<name>` matching `spec.consumerRef` |
| `consumerComponentGeneration` | Consumer Provider component generation |
| `audience` | `spec.audience` value (opaque; not echoed in logs or spans) |
| `operationClass` | `acquire-token` or `refresh-token` |
| `expiryUnixMs` | Clipped to `spec.rotation.maxLeaseLifetimeMs` |
| `deadlineUnixMs` | Hard session close deadline (≤ `expiryUnixMs`) |
| `routeDigest` | Digest of bus-authorized route parameters |
| `schemaVersion` | Fixed version of the binding contract |
| `maxTokenBytes` | Closed upper bound on sensitive output size |
| `transcriptDigest` | Noise transcript digest after handshake |

Both parties MUST verify the full binding before accepting records. The binding
is conveyed in the Noise handshake prologue.

### 8.3 Security requirements for the delivery session

1. **Enrolled keys only**: both static keys must be enrolled at session
   initiation. Any NN/NX/N pattern is rejected immediately; session is closed
   and zeroized.
2. **Replay-safe sequence**: monotonically increasing per-credential-UID sequence
   number. A replay at the same or lower sequence number is rejected.
3. **Bounded output**: sensitive output record MUST NOT exceed `maxTokenBytes`.
   Oversize records reject immediately; channel closed and zeroized.
4. **Zeroizing buffers**: plaintext is zeroed in memory immediately after the
   consumer extracts it. The provider zeroizes after encryption. All intermediate
   buffers are zeroizing types.
5. **Redacted Debug**: all credential-bearing Rust types involved in delivery
   (request, response, record wrapper, buffer) MUST implement `Debug` via a
   hand-written impl emitting only the type name and a placeholder. Derived
   `Debug` is forbidden for these types.
6. **No automatic success-shaped replay**: after any ambiguous delivery outcome
   (timeout, partial write, disconnection), the provider MUST NOT auto-retry
   with the same record. The consumer must re-initiate via `AcquireToken` or
   `RefreshToken`.
7. **Immediate close/zeroize**: after the delivery record is confirmed received,
   the provider closes and zeroizes the delivery channel. The channel is not
   reused across multiple deliveries.

### 8.4 RBAC enforcement point

d2b-bus performs the following checks before authorizing the delivery route:

- RBAC `use-credential` for the authenticated consumer Provider subject,
  Credential ResourceRef, and operation class;
- `spec.consumerRef` matches the consumer Provider identity from the signed
  component descriptor;
- `spec.allowedOperations` includes the operation class;
- current lease state permits the operation class;
- no Role/RoleBinding or Provider generation change has revoked authorization
  since the last audit checkpoint.

After these checks pass, d2b-bus forwards opaque Noise-encrypted records between
the two endpoints. The bus never buffers or stores the records.

---

## 9. Lease lifecycle

### 9.1 State machine

The latest bounded lease checkpoint is status/Operation-ledger state, not a
Provider state Volume payload. Per D088, Credential lease metadata common to all
implementations (`leaseState`, opaque non-authorizing `leaseHandle`, `audience`,
expiry/issue timestamps, and phase) lives in `status.resource`; Secret
Service-specific observations (`rotationGeneration`, `sourceVersion`, retry
counters, and closed-enum outcome) live in `status.provider.details`. Any
`leaseHandle` written to status is opaque, non-authorizing, bounded, safe for
authorized status readers, and independently revalidated against the Secret
Service port before use. Secret Service object paths and credential bytes never
enter status, audit, telemetry, or storage.

```
Absent
  └── AcquireToken ──────────► Active (CredentialReady=True)

Active
  ├── [proactive window] ────► RotationDue (CredentialReady=True, RotationDue=True)
  ├── RefreshToken ──────────► Active (rotationGeneration unchanged, new expiresAtUnixMs)
  ├── [rotation] ────────────► Active (rotationGeneration+1, new leaseHandle)
  ├── RevokeToken ───────────► Revoked (CredentialReady=False, LeaseRevoked=True)
  ├── expiry deadline ───────► Expired (CredentialReady=False)
  └── Provider gen change ───► [onProviderGeneration=immediate] → Revoked
                                [onProviderGeneration=drain-leases] → Expired on deadline

RotationDue
  ├── rotation success ───────► Active (rotationGeneration+1)
  ├── rotation failure ───────► RotationDue/Degraded (bounded retry)
  └── retry exhaustion ───────► Failed (outcome=rotation-failed)

Expired
  └── AcquireToken ──────────► Active (new lease; rotationGeneration+1)

Revoked
  ├── AcquireToken ──────────► Active (if policy permits re-acquisition)
  └── resource delete ───────► provider-revoke finalizer satisfied immediately
```

### 9.2 Idempotency

Every `issue_lease` / `refresh_lease` / `revoke_lease` call carries a stable
`idempotency_key` derived from:

```
idempotency_key = sha256(credential_uid || rotation_generation.to_le_bytes() || operation_tag)
```

Where `operation_tag` is one of: `b"acquire"`, `b"refresh"`, `b"revoke"`. The
key is hex-encoded; max 64 chars; contains no secret material. A duplicate call
with the same key returns the existing result without double-issuing.

### 9.3 Lease cardinality

`MAX_LOCAL_LEASES = 256`. The controller maintains a bounded in-memory lease
table. When the table is full, `issue_lease` returns
`credential-queue-pressure`. The `maxLeases` Provider config value (range 1..256)
further restricts the effective limit per Provider instance.

### 9.4 Finalizers

| Finalizer ID | Owner | Meaning |
| --- | --- | --- |
| `credential.d2bus.org/provider-revoke` | Credential controller | Revoke all active leases before deletion; honors `revocation.onOwnerDelete` policy |
| `credential.d2bus.org/consumer-drain` | `consumerRef` controller | Drain in-flight operations before Provider releases lease handle |

Execution order: `consumer-drain` completes before `provider-revoke`. A missing
`consumerRef` removes `consumer-drain` automatically. A terminal `leaseState`
(Revoked or Expired) satisfies `provider-revoke` immediately without further
network calls.

---

## 10. RBAC and security

### 10.1 Resource verbs

| Verb | Authorized subjects |
| --- | --- |
| `get` | Any authorized subject |
| `list` | Any authorized subject |
| `watch` | Any authorized subject |
| `create` | Deployer/system-core configuration controller |
| `update-spec` | Deployer/system-core configuration controller |
| `update-status` | Credential controller (exact registered process generation) only |
| `update-finalizers` | Credential controller; `consumerRef` controller |
| `delete` | Deployer/system-core configuration controller |
| `use-credential` | Consumer subject authorized via `consumerRef` and Role/RoleBinding |

`use-credential` Role rule shape:

```yaml
rules:
  - resourceTypes: [Credential]
    verbs: [use-credential]
    resourceNames: [local-keyring]
    zones: [dev]
    executionRefs: [Host/host-system]
    operationClasses: [acquire-token, refresh-token]
```

The effective operation set is the intersection of `spec.allowedOperations` and
the Role `operationClasses`.

### 10.2 Secret isolation invariants

1. **Zero-secret-bytes invariant**: `spec`, `status`, the resource store row,
   redb WAL, revision log, bus routing DTOs, error messages, audit records,
   OTEL spans, metrics, and all log lines never contain secret bytes in any
   field.
2. **Port boundary**: the `Oo7SecretServicePort` trait is the sole boundary
   through which the controller interacts with the keyring. No method accepts or
   returns a password, secret value, token, endpoint, path, file descriptor, or
   byte buffer.
3. **Delivery channel**: token bytes travel exclusively in the end-to-end
   Noise_KK delivery record between the Credential Provider process and the
   authorized consumer Provider process. d2b-bus never terminates or stores
   delivery record content.
4. **Redacted Debug**: `SecretServiceLeaseRequest` and `SecretServiceLeaseRef`
   implement hand-written `Debug` that omits `operation`, `credential_provider_id`,
   and `consumer_provider_id`; they emit only generation, placement binding,
   operation count, and expiry fields.
5. **`object_path_canary` enforcement**: the `object_path_canary` value held
   by `FakeOo7Port` in tests MUST NOT appear in any service response, status
   field, audit record, log line, OTEL span attribute, or delivery record. This
   is enforced by `tests/canary.rs`.
6. **`credential_canary` enforcement**: the `credential_canary` value held by
   `FakeOo7Port` MUST NOT appear in any service response, status field, audit
   record, log line, OTEL span, metric label, or delivery record.
7. **No ambient portal fallback**: the Secret Service connection port reaches the
   controller exclusively through the fixed user supervisor/user portal: the
   framework delivers it as a pre-opened FD in the LaunchTicket's inherited FD
   table. The crate adapter operates only over this inherited portal FD; it must
   not discover D-Bus addresses, connect to ambient sockets, read
   `DBUS_SESSION_BUS_ADDRESS`, or open any socket path to reach the keyring.
   Absent the portal FD, the controller fails closed.
8. **Status-first operational state (D087)**: bounded non-secret controller
   and lease observation lives in the owning `Credential.status` subresource
   and the core Operation ledger by default. In-flight idempotency/retry is an
   Operation-ledger concern; the latest bounded lease result/checkpoint lives in
   status. `credential-secret-service` declares **no Provider state Volume**
   because it has no payload that passes the storage-need test: no secret
   private recovery data may enter storage, there is no large/binary/file
   content, no private data unsafe for authorized status readers is persisted,
   and bounded recovery state is suitable for revisioned status plus external
   revalidation. ProviderStateSet is optional query-time grouping and is empty
   for this Provider (D086, superseded by D087); there is no bootstrap state
   Volume mechanism or exception. Status is revisioned,
   optimistic-status-writer controlled, RBAC-readable, redacted,
   observation-only, written only on material change, and bounded to the core
   status limits (total ≤ 64 KiB, provider-specific detail ≤ 32 KiB, with
   `status-oversize` rejection). It must not contain secrets, tokens, keys, PSKs,
   authority-conferring credential handles, Secret Service object paths,
   private path/argv/env/PID/unit data, raw provider error bodies, large blobs,
   or churn-heavy content. Any opaque handle in status is non-secret,
   non-authorizing, bounded, safe for authorized status readers, and
   independently revalidated before use. Raw credential delivery remains
   transient process memory over dedicated Noise_KK sensitive sessions only.

### 10.3 Canary tests (`tests/canary.rs`)

Every test in `canary.rs` constructs a `FakeOo7Port` with a non-empty
`credential_canary` and a non-empty `object_path_canary`, runs a complete
operation (acquire, refresh, revoke, inspect), and asserts that neither canary
string appears anywhere in:

- the outer `d2b.credential.v3` response DTO;
- any `status` field serialized to JSON;
- any audit record serialized to JSON;
- any log line captured by the test subscriber;
- any OTEL span attribute captured by the test subscriber;
- the delivery session binding parameters.

A failing canary test is a hard error that blocks the PR.

---

## 11. Status, errors, audit, and OTEL

Per D088, ResourceType-common Credential observation lives in `status.resource`:
the non-secret lease metadata base that is identical across Credential
implementations. Secret Service-specific lease observations live only in
`status.provider` with `providerRef`, qualified `schemaId`
`credential-secret-service.d2bus.org/Credential/status`, `schemaVersion`,
`observedProviderGeneration`, and strict bounded redacted `details`
(≤32 KiB, unknown-field-denied). The controller writes all present layers
atomically in one status mutation; shared
fields are never duplicated into `status.provider`, and the extension schema is
registered and signed in the Provider manifest. No secret bytes appear in any
status layer.

### 11.1 Status conditions

| Condition type | Meaning |
| --- | --- |
| `CredentialReady` | `True` when `leaseState=Active` and not within rotation window |
| `RotationDue` | `True` when `policy=proactive` and remaining lifetime < `proactiveWindowMs` |
| `ProviderUnavailable` | `True` when port returns `Locked` or `Unavailable` consistently |
| `LeaseRevoked` | `True` when `leaseState=Revoked` and no replacement issued |

### 11.2 Stable error codes

| Code | Meaning |
| --- | --- |
| `credential-not-found` | Credential resource does not exist in this Zone |
| `credential-provider-unavailable` | Port returned `Locked` or `Unavailable` |
| `credential-lease-expired` | Lease is past expiry deadline |
| `credential-lease-revoked` | Lease was explicitly revoked |
| `credential-operation-denied` | Operation class not in `allowedOperations` or RBAC denied |
| `credential-consumer-mismatch` | Requesting subject does not match `consumerRef` |
| `credential-placement-mismatch` | Scope domain is not `user` or Host constraint violated |
| `credential-rotation-failed` | Proactive rotation failed after bounded retries |
| `credential-invariant-failure` | Port returned a response failing invariant checks |
| `credential-schema-invalid` | `sign-challenge` requested (unsupported); or spec fails validation |
| `credential-queue-pressure` | Lease table at capacity (`maxLeases`) |

All error messages are bounded (max 240 UTF-8 chars), stripped of control
characters, and must not contain token bytes, URLs, UUIDs, provider diagnostics,
host paths, or connection string shapes.

### 11.3 Audit records

| Event | Retained fields |
| --- | --- |
| Credential resource create/update/delete | Zone, subject digest, `resource_name_digest`, verb, revision result, authorization decision |
| `AcquireToken` | Zone, subject digest, `resource_name_digest`, operation class, `rotationGeneration`, outcome code, idempotency key digest |
| `RefreshToken` | Zone, subject digest, `resource_name_digest`, operation class, `rotationGeneration`, outcome code, idempotency key digest |
| `RevokeToken` | Zone, subject digest, `resource_name_digest`, operation class, `rotationGeneration`, revocation result code |
| Rotation | Zone, `resource_name_digest`, trigger reason, old `rotationGeneration`, new `rotationGeneration`, outcome code |
| Provider generation change revocation | Zone, `resource_name_digest`, policy applied, outcome code |
| Finalize (`provider-revoke`) | Zone, `resource_name_digest`, revocation outcome, `revokedAtUnixMs` |
| Bundle activated | Zone, `activationGeneration`, digest, create/update/skip/removed counts |
| Cleanup complete *(audit subsystem only; appended post-core-deletion; not emitted by controller)* | Zone, `resource_name_digest`, event-only Deleted revision committed and row/indexes removed atomically, `activationGeneration`, `cleanupLatencyMs` |

`resource_name_digest` is SHA-256 of the Credential resource name, never the
raw name. It is admitted only to the authorization-controlled bounded Zone
audit stream and, for caller-initiated operations, after the authorization
decision. Raw Credential name, ResourceRef, and UID are excluded. The digest
is never copied to telemetry, logs, collector diagnostics, or support
summaries.

Excluded from all audit records: token bytes, key material, passwords, bearer
strings, provider-internal diagnostics, host paths, connection strings, audience
literals, user-identifying path components, Noise/session key material, and
Secret Service object paths.

### 11.4 OTEL spans

Span names follow `d2b.credential.<operation>`:

- `d2b.credential.acquire_token`
- `d2b.credential.refresh_token`
- `d2b.credential.revoke_token`
- `d2b.credential.inspect_metadata`
- `d2b.credential.reconcile`
- `d2b.credential.rotation`

Required span attributes (closed set):

| Attribute | Value |
| --- | --- |
| `d2b.credential.provider` | `credential-secret-service` |
| `d2b.credential.operation_class` | Closed enum string |
| `d2b.credential.placement_binding` | `user-agent` |
| `d2b.credential.outcome` | Stable closed outcome code |
| `d2b.credential.rotation_generation` | Numeric rotation generation |

Credential telemetry uses only applicable generic OTEL Resource attributes from
the collector's closed allowlist:

| Resource attribute | Value |
| --- | --- |
| `d2b.zone` | Zone name, re-stamped at trusted ingress |
| `d2b.provider` | `credential-secret-service` |
| `d2b.component` | Signed controller/service component ID |
| `service.name` | Fixed controller/service name |
| `service.namespace` | Fixed service namespace |
| `service.version` | Build version |

No OTEL Resource attribute or span attribute carries a Credential resource
name, ResourceRef, UID, digest (including `resource_name_digest`), or derived
identity token. Also forbidden: token bytes, audience literals, provider
diagnostics, host paths, Secret Service object paths, collection names,
resource IDs, and correlation IDs embedding secret shapes.

### 11.5 Metrics

| Metric | Type | Labels |
| --- | --- | --- |
| `d2b_credential_operations_total` | Counter | `provider=credential-secret-service`, `operation_class`, `placement_binding=user-agent`, `outcome` |
| `d2b_credential_lease_expiry_seconds` | Gauge | `provider=credential-secret-service`, `placement_binding=user-agent` |
| `d2b_credential_rotation_total` | Counter | `provider=credential-secret-service`, `policy`, `outcome` |
| `d2b_credential_provider_health` | Gauge (0/1) | `provider=credential-secret-service` |
| `d2b_credential_active_leases` | Gauge | `provider=credential-secret-service`, `placement_binding=user-agent` |

The expiry gauge reports the minimum seconds remaining across active
user-agent leases (0 when none). Label cardinality is bounded and semantic;
metric labels carry no Credential resource name, ResourceRef, UID, digest, or
derived identity token. Credential identity is available only as
`resource_name_digest` in authorized bounded audit records, never telemetry.
Generic allowlisted OTEL Resource attributes such as `d2b.zone`,
`d2b.provider`, and `d2b.component` remain available and are not copied into
metric labels or span attributes. Secret-shape assertions run on all label
values.

---

## 12. Nix artifact and build

### 12.1 Artifact catalog entry

```nix
d2b.artifacts.credential-secret-service-bin = {
  package = pkgs.d2b-provider-credential-secret-service;
  type    = "provider";
};
```

The ID `credential-secret-service-bin` matches `^[a-z][a-z0-9-]*$`. It is not
a `*Ref`. The catalog entry:

- is integrity-pinned at build time alongside the resource bundle;
- is emitted to the global private catalog `/etc/d2b/artifact-catalog.json`
  (owner `root:d2bd`, mode 0640), shared across all Zones;
- carries `id`, `type`, `sha256`, `storePath` (private; implementation data
  only), and bounded closure metadata;
- `storePath` is private implementation data used by `activation-nixos` for
  package staging; it never appears in resource spec, status, audit records,
  or any log line;
- has its own SHA-256 digest header verified by `activation-nixos` before any
  create/update.

### 12.2 Eval-time assertions (Nix, `nixos-modules/assertions.nix` pattern)

Applied to every `d2b.zones.<zone>.resources.<name>` entry with
`type = "Credential"` and `spec.providerRef = "Provider/credential-secret-service"`:

- `spec.scope.domainFilter` must be `"user"`; `"system"` and `"guest"` fail
  the assertion.
- `spec.scope.executionRef` must resolve a declared `Host/<name>` or
  `Guest/<name>` in the same Zone.
- `spec.scope.userRef` must be set and resolve a declared `User/<name>` in the
  same Zone.
- `spec.allowedOperations` must be a non-empty subset of
  `{ acquire-token, refresh-token, revoke-token, inspect-metadata }`;
  `sign-challenge` fails the assertion.
- `spec.audience` must pass `^[A-Za-z0-9._:/@-]+$` charset and max 256 chars.
- `spec.consumerRef`, if set, must resolve a declared `Provider/<name>` in the
  same Zone.
- `contains_sensitive_shape` runs on all string fields; secrets in any field
  fail the eval.
- Duplicate `(providerRef, executionRef, userRef, audience)` tuple in the same
  Zone is rejected.
- `spec.providerRef` must resolve a Provider resource whose `spec.artifactId`
  resolves an artifact catalog entry of `type = "provider"`.

### 12.3 Build-time validation

- Each Credential spec is validated against
  `docs/reference/schemas/v3/credential.json`.
- The Provider-specific schema cross-check confirms `audience` charset and
  `collectionAlias` format match the Provider's declared constraints.
- Drift gate (`make test-drift`): `cargo xtask gen-schemas` + `git diff
  --exit-code`. A committed schema change not matching the crate-derived schema
  fails the gate.
- Nix options module drift is checked in the same gate.
- Bundle and artifact catalog: the bundle digest round-trip test verifies the
  sorted-resources digest matches the bundle header. The global artifact catalog
  (`/etc/d2b/artifact-catalog.json`) round-trip test verifies the catalog digest
  header and asserts that `storePath` values are absent from the resource bundle,
  resource status, and log outputs.
- Process spec golden: the `controller_process_spec_golden` unit test in
  `src/controller.rs` serializes the Process resource generated by `reconcile()`
  and asserts exact field shapes: `template = "secret-service-controller"`;
  `sandbox.namespaceClasses = [mount, pid, ipc]` (no `user` class;
  `Provider/system-systemd` rejects it; same-UID execution via `spec.userRef`),
  `sandbox.capabilityClasses = []`, `sandbox.seccompClass = "default-strict"`,
  `sandbox.noNewPrivileges = true`, `sandbox.startRoot = false`,
  `sandbox.readOnlyRoot = true`; `budget.memory.limit = "64Mi"`,
  `budget.pids.limit = 32`, `budget.fds.limit = 64`; `networkUsage = null`;
  owned `Endpoint` resource for the credential service; `readiness.class = "provider-defined"`. Any drift from this
  shape fails the test and blocks the PR.

### 12.4 Generation transition and cleanup contract

When a Nix configuration generation removes a Credential resource that
referenced `Provider/credential-secret-service`:

1. `activation-nixos` verifies the new resource bundle and artifact catalog
   SHA-256 digests.
2. It creates/updates the new desired set without blocking on cleanup.
3. It issues async Delete for the removed resource
   (`metadata.managedBy` is set exclusively by core, never by ownerRef, labels,
   or bundle-authored fields; `deletionRequestedAt` set).
4. The Credential controller runs the `provider-revoke` finalizer (see §6.2):
   `RevokeToken` is called; the controller emits a revoke outcome audit but MUST
   NOT emit the resource-deleted closure audit. The scoped `Process` resource
   (if one was created for this Credential) is deleted by the controller before
   clearing the finalizer. The `provider-revoke` finalizer is then cleared. The
   core store transaction writes the event-only `Deleted` revision and removes
   the resource row and indexes atomically. The audit subsystem appends the
   closure audit after that committed revision with dedup/exactly-once recovery.
   No `phase=Deleted` row persists. Finalizers are never force-cleared.
5. Resources with `metadata.managedBy = "controller"` or `"api"` are never
   touched by this path.
6. Prior bundles are retained up to `retainedGenerations` (default 3,
   range 1..16). Rollback re-creates removed resources from a retained bundle;
   fresh leases are acquired after re-creation (prior secrets are not restored).

Removed resource status during cleanup:

```yaml
phase: Degraded
conditions:
  - type: Cleanup
    status: "True"
    reason: nix-generation-removed
    message: "credential removed from nix configuration; pending provider-revoke finalizer"
```

---

## 13. Async reconcile loop

All controller handlers are async. The reconcile loop follows
`ADR-046-resource-reconciliation`:

- A dedicated watch task reads the Credential, Provider, Host, Guest, and User
  watch streams concurrently.
- Per-resource reconcile tasks run independently; independent resources
  reconcile in parallel within `reconcileConcurrency = 8`.
- A long-running `observe` or `finalize` task does not block the next ready
  reconcile task.
- There is no fixed poll or debounce delay. Core delivers bounded hints
  immediately after durable commit.
- Retry backoff is exponential with jitter; max 5 retries before `Fail`.
- `observeInterval = 30s` is a lightweight `InspectMetadata` poll; it does not
  re-acquire leases or mutate the resource store on success.
- Idempotency keys prevent double-issuing on concurrent or redundant reconcile
  triggers.
- The reconcile loop cancels outstanding tasks cleanly on controller drain.

---

## 14. Current-code fit and reuse

| Item | Treatment |
| --- | --- |
| Current anchor (v3 baseline `b5ddbed6`) | `d2b-realm-provider/src/provider.rs:CredentialProvider` (status-only trait, `implemented-and-reachable`); `credential.rs` (three-plane opaque refs, `implemented-and-reachable`); `packages/d2b-core/src/realm_workloads_launcher.rs:LauncherMetadataInvariants.no_secrets_or_credentials` (`implemented-and-reachable`) |
| Evidence class | Full lease model, controller, controller descriptor, d2b-bus routing, and async reconciliation are `ADR-only`; `Oo7SecretServicePort` trait, lease DTOs, and `FakeOo7Port` test suite are adapted from main |
| Main reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-secret-service/src/lib.rs` (`Oo7SecretServicePort`, `SecretServiceLeaseRequest/Ref/Grant/Inspection/Renewal/Revocation`, `SecretServiceState/LeaseState`, `SecretServiceProviderError`, `SecretServiceOwner`, `SecretServiceCredentialProvider`, `SecretServiceCredentialProviderFactory`); `src/tests.rs` (`FakeOo7Port`, `credential_canary`, `object_path_canary`, lease lifecycle, locked-state tests, cardinality limits) |
| Reuse action | copy and adapt (revert v2 types; replace v2 `CredentialProvider` trait with v3 `d2b.credential.v3` service; replace v2 ProviderFactory/registry with Provider resource/descriptor) |
| Behavior retained | Zero-secret-bytes invariant structurally enforced at port boundary; `SecretServiceOwner::Userd` placement restriction; bounded opaque lease metadata only crosses the port boundary; `credential_canary`/`object_path_canary` enforcement; hand-written `Debug` on request/ref types; injected-port pattern; `MAX_LOCAL_LEASES` cardinality cap |
| Required delta | v3 contract names/versions; Provider resource and signed controller descriptor; d2b-bus routing; Zone/Resource placement/scope; async reconcile loop and handler methods; `Noise_KK` delivery session; OTEL/audit emission; Nix resource compiler integration; workspace layout (`src/`, `tests/`, `integration/`, `README.md`); D087 status-first state model: `credential-secret-service` declares no Provider state Volume, ProviderStateSet is optional/query-time and empty, and bounded non-secret lease/acquisition/retry observation lives in `Credential.status` plus the core Operation ledger; no bootstrap state Volume mechanism (D086, superseded by D087); any status handle is non-secret, non-authorizing, bounded, safe for authorized status readers, and independently revalidated; `networkUsage: null` in Process spec; finalize sequence separates revoke/drain + Process deletion from closure audit (audit subsystem only); core aggregates Provider status (controller writes only scoped Credential/Process health) |
| Excluded main assumptions | v2 `EndpointRole`/`Realm`/`userd` process model; v2 `ProviderFactory`/`ProviderRegistryBuilder`; v2 component-session auth and prologue; v2 `AgentPlacementBinding`; v2 `CredentialLease`/`CredentialLeaseState` from `d2b-contracts::v2_provider`; v2 `CredentialPlacementBinding::UserAgent` struct (replaced by v3 `PlacementBinding::UserAgent` enum variant) |
| Replacement/deletion | Old `CredentialProvider` trait (`d2b-realm-provider/src/provider.rs`) and `CredentialStatus` enum removed only after all three v3 Credential Provider controllers reach full reconcile parity |
| Feasibility proof | Main `a1cc0b2d` proves: `Oo7SecretServicePort` trait API; `FakeOo7Port` with canary enforcement; acquire/refresh/revoke/inspect lifecycle; locked-state → unavailable mapping; cardinality limits; check_provider_conformance pattern |

---

## 15. Implementation work items

### ADR046-cred-ss-003 (primary)

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cred-ss-003` |
| Dependency/owner | `ADR046-cred-ss-001` (contract types); `ADR046-cred-ss-002` (service proto); `ADR046-reconcile-001`; credential-secret-service owner |
| Current source | `packages/d2b-realm-provider/src/provider.rs:CredentialProvider` (minimal v3 baseline) |
| Reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-secret-service/src/lib.rs` (full implementation); `src/tests.rs` (full test suite including `FakeOo7Port`, lease lifecycle, locked state, canary enforcement, cardinality limits) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-credential-secret-service/src/{lib.rs, controller.rs, service.rs, main.rs}`; `packages/d2b-provider-credential-secret-service/tests/{lifecycle.rs, conformance.rs, faults.rs, canary.rs, delivery.rs, placement.rs}`; `packages/d2b-provider-credential-secret-service/integration/{container-service.sh, host-placement.nix, guest-placement.nix, cleanup-rollback.sh}`; `packages/d2b-provider-credential-secret-service/README.md` |
| Detailed design | Adapt `SecretServiceCredentialProvider` and `SecretServiceCredentialProviderFactory` to v3 `d2b.credential.v3` service; replace v2 `CredentialProvider` trait with v3 controller/service handler; retain `Oo7SecretServicePort` trait methods unchanged; ensure `SecretServiceOwner::Userd` placement guard rejects system-domain and guest-agent construction; validate `collectionAlias` against provider-internal charset (not `OpaqueAzureRef`; collection aliases may include spaces); integrate with Provider resource descriptor and controller toolkit; test that `credential_canary` never appears in any service response; create a Process resource per `(Zone, User, executionRef)` triple with `template = "secret-service-controller"` (plain string), canonical `sandbox` fields (`namespaceClasses`, `capabilityClasses`, `seccompClass`, `noNewPrivileges`, `startRoot`, `environmentClass`, `readOnlyRoot`), `budget` with nested `cpu`/`memory`/`pids`/`fds` sub-fields, `networkUsage: null`, no inline endpoint fields, an owned credential-service `Endpoint` resource, and `readiness.class = "provider-defined"`; component descriptor declares the required authenticated `dbus-session` FD attachment carried privately by the LaunchTicket; D087 status-first state model: no Provider state Volume is declared, ProviderStateSet is optional/query-time and empty, no Volume mount or layout principal is required, and the storage-need test is not met; bounded non-secret lease/acquisition/retry observation lives in `Credential.status` plus the core Operation ledger; any opaque status handle is non-secret, non-authorizing, bounded, safe for authorized status readers, and independently revalidated; no token/object-path/lease bytes persist anywhere; finalize() emits revoke outcome audit but MUST NOT emit the resource-deleted closure audit (audit subsystem only); controller writes only scoped Credential/Process health (core aggregates Provider status) Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | Target: user-domain `Process` resource under `Host` or `Guest` (ADR-only ResourceType); d2b-bus routes `d2b.credential.v3` calls to this process; Credential controller reconciles status. Current v3 has no user-credential host process: v3 `d2b-userd` is a guest exec stub (exits 78 in service mode; no credential functionality; `test-only-or-preview`). This integration path is fully new (ADR-only) work. |
| Data migration | Full reset; no migration from old `CredentialProvider` trait |
| Validation | See §16 |
| Removal proof | Old `d2b-realm-provider:CredentialProvider` trait removed only after all three v3 Credential Provider controllers reach full reconcile parity |

### ADR046-cred-ss-001 (dependency: contract types)

| Field | Value |
| --- | --- |
| Dependency/owner | Dependency for ADR046-cred-ss-003; owner: `packages/d2b-contracts` Credential ResourceType contract |
| Current source | `packages/d2b-realm-provider/src/provider.rs:CredentialProvider`; `packages/d2b-realm-provider/src/credential.rs` opaque credential refs and `OpaqueAzureRef` helpers |
| Reuse action | adapt |
| Destination | packages/d2b-contracts/src/v3/credential.rs |
| Detailed design | Contract types: define `CredentialSpec`, `CredentialStatus`, `CredentialLeaseHandle`, `OperationClass`, `PlacementBinding`, `CredentialConditionType`, and serde/validation/redaction helpers. Reuse `OpaqueAzureRef` from the v3 baseline. Full detail remains in `ADR-046-resources-credential` §Implementation work items. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt current credential/status concepts into v3 ResourceType DTOs; reuse `OpaqueAzureRef` directly where applicable. |
| Integration | Nix compiler emits these DTOs; ResourceAPI stores them; credential-secret-service controller/service consumes them; CLI and conformance tests validate base Credential spec/status behavior. |
| Data migration | Full d2b 3.0 reset; no v2 CredentialProvider status/config import |
| Validation | Credential ResourceType schema/serde/redaction tests from `ADR-046-resources-credential`; credential-secret-service conformance consumes the shared types |
| Removal proof | Old `d2b-realm-provider:CredentialProvider` trait and `CredentialStatus` enum are removed only after all three v3 Credential Provider controllers reach full reconcile parity |

Defines `CredentialSpec`, `CredentialStatus`, `CredentialLeaseHandle`,
`OperationClass`, `PlacementBinding`, `CredentialConditionType`, and all
serde/validation/redaction helpers in
`packages/d2b-contracts/src/v3/credential.rs`. Reuses `OpaqueAzureRef`
directly from v3 `d2b-realm-provider/src/credential.rs`. Full detail in
`ADR-046-resources-credential` §Implementation work items.

### ADR046-cred-ss-002 (dependency: service proto)

| Field | Value |
| --- | --- |
| Dependency/owner | Dependency for ADR046-cred-ss-003; owner: credential service contract/codegen |
| Current source | None — net-new v3 `d2b.credential.v3` service; no pre-ADR45 baseline service proto equivalent |
| Reuse action | create |
| Destination | packages/d2b-contracts/proto/v3/credential.proto; packages/d2b-credential-service/ |
| Detailed design | Service proto: define the `d2b.credential.v3` protobuf/ttrpc service and generate typed client/server code. Full detail remains in `ADR-046-resources-credential` §Implementation work items. Primary reuse disposition: `create`. Preserved source-plan detail: net-new service contract replacing the v2 in-process `CredentialProvider` trait. |
| Integration | d2b-bus routes Credential service calls to credential-secret-service Process instances; generated client/server types bind the controller/service implementation to ComponentSession delivery. |
| Data migration | Full d2b 3.0 reset; no v2 service state import |
| Validation | Generated-code compile tests and credential service contract tests from `ADR-046-resources-credential`; credential-secret-service lifecycle/delivery tests consume the generated service |
| Removal proof | V2 `CredentialProvider` trait calls are superseded by `d2b.credential.v3` only after all credential providers reach parity |

Defines `d2b.credential.v3` protobuf/ttrpc service in
`packages/d2b-contracts/proto/v3/credential.proto` and generates typed
client/server in `packages/d2b-credential-service/`. Full detail in
`ADR-046-resources-credential` §Implementation work items.

### ADR046-cred-ss-004 (dependency: controller toolkit)

| Field | Value |
| --- | --- |
| Dependency/owner | Dependency for ADR046-cred-ss-003; owner: common Credential controller/reconciliation toolkit |
| Current source | ADR-only controller pattern from `ADR-046-resource-reconciliation`; no concrete secret-service baseline controller to import |
| Reuse action | create |
| Destination | packages/d2b-provider-credential-<impl>/src/controller.rs |
| Detailed design | Controller toolkit: implement the common Credential controller handler conforming to the `ADR-046-resource-reconciliation` async loop. Secret-service-specific controller code plugs into this handler while keeping provider bytes out of status/store/audit. Primary reuse disposition: `create`. Preserved source-plan detail: net-new shared controller handler pattern specialized by each Credential Provider. |
| Integration | Resource watches and Operation ledger drive the controller loop; credential-secret-service handler uses the toolkit to reconcile Credential status, finalizers, Process health, and service lifecycle. |
| Data migration | None — controller toolkit code only; no runtime state migration |
| Validation | Shared reconciliation tests from `ADR-046-resources-credential`; credential-secret-service lifecycle/fault tests verify the handler integration |
| Removal proof | None — shared toolkit is additive; v2 trait removal is tracked by ADR046-cred-ss-003/001 parity |

Implements the common Credential controller handler conforming to the
`ADR-046-resource-reconciliation` async loop in
`packages/d2b-provider-credential-<impl>/src/controller.rs`. Full detail in
`ADR-046-resources-credential` §Implementation work items.

### ADR046-cred-ss-005 (dependency: Nix compiler)

| Field | Value |
| --- | --- |
| Dependency/owner | Dependency for ADR046-cred-ss-003; owner: Nix resource compiler and activation cleanup |
| Current source | None — net-new v3 `d2b.zones.<zone>.resources.<name>` Credential/Provider authoring surface; no pre-ADR45 baseline equivalent |
| Reuse action | create |
| Destination | nixos-modules/options-resources.nix; nixos-modules/activation-nixos-cleanup.nix |
| Detailed design | Nix compiler: implement `d2b.zones.<zone>.resources.<name>` authoring, eval-time assertions, canonical JSON emission, artifact catalog, bundle digest, and generation cleanup contract. Full detail remains in `ADR-046-resources-credential` §Implementation work items. Primary reuse disposition: `create`. Preserved source-plan detail: net-new Nix resource emission and cleanup contract. |
| Integration | Nix emits Provider/Credential resource JSON and artifact catalog entries; ResourceAPI admission and credential-secret-service controller consume the rendered resources; activation cleanup issues async Delete/finalizer flow on generation removal. |
| Data migration | Full d2b 3.0 reset; no old credential config is imported into v3 resources |
| Validation | Nix eval/assertion/golden tests from `ADR-046-resources-credential`; credential-secret-service cleanup rollback integration fixture |
| Removal proof | None — new v3 Nix resource surface; old trait removal waits for controller parity |

Implements `d2b.zones.<zone>.resources.<name>` Nix authoring, eval-time
assertions, canonical JSON emission, artifact catalog, bundle digest, and
generation cleanup contract in `nixos-modules/options-resources.nix` and
`nixos-modules/activation-nixos-cleanup.nix`. Full detail in
`ADR-046-resources-credential` §Implementation work items.

### ADR046-cred-ss-006 (dependency: audit/OTEL)

| Field | Value |
| --- | --- |
| Dependency/owner | Dependency for ADR046-cred-ss-003; owner: credential-secret-service audit and telemetry implementation |
| Current source | `packages/d2b-core/src/realm_workloads_launcher.rs:LauncherMetadataInvariants.no_secrets_or_credentials`; secret-service main reuse canary tests listed in §14 |
| Reuse action | adapt |
| Destination | packages/d2b-provider-credential-secret-service/src/{audit.rs,telemetry.rs} |
| Detailed design | Audit/OTEL: emit authorized bounded audit records with Credential identity represented only by `resource_name_digest`, and emit OTEL spans/metrics for all credential service methods and controller events with canary enforcement, expiry aggregated across user-agent leases, no Credential resource name, ResourceRef, UID, digest, derived identity token, Zone/Credential/resource-name-derived metric label, or non-allowlisted OTEL Resource attribute; retain applicable generic collector-allowlisted Resource attributes (`d2b.zone`, `d2b.provider`, `d2b.component`, and service fields); no token/object-path/lease bytes in status, delivery outer headers, audit, metrics, spans, or logs. Full detail remains in `ADR-046-resources-credential` §Implementation work items. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt zero-secret invariant and canary test pattern to credential-secret-service audit/OTEL surfaces. |
| Integration | Controller and service methods call audit/telemetry helpers; audit subsystem and OTEL exporters consume bounded event/span/metric records; canary tests verify every public observable surface stays secret-free. |
| Data migration | None — audit/telemetry only; no runtime state migration |
| Validation | Credential audit/OTEL tests from `ADR-046-resources-credential` require `resource_name_digest` in authorized audit records and reject raw Credential name/ResourceRef/UID; `tests/canary.rs` structurally asserts exact absence of `vm`, `zone`, `zone_id`, `zone_uid`, `credential_name`, `credential_ref`, `credential_uid`, `credential_digest`, `resource_name_digest`, and every resource-name-derived metric key; Credential name/ref/UID/digest canaries are absent from all OTEL Resource attributes, span attributes, and metric labels; Zone-name canaries are absent from spans and labels while generic collector-allowlisted Resource attributes remain; complete secret-service metric/span frames pass the shared collector ingress validator, while adding `d2b.credential.name` or any Credential identity key/value rejects the whole frame; `tests/delivery.rs` covers credential-secret-service delivery |
| Removal proof | None — audit/telemetry helpers are new; no prior owner to remove |

Implements audit record and OTEL span/metric emission for all credential
service methods and controller events in
`packages/d2b-provider-credential-secret-service/src/{audit.rs, telemetry.rs}`.
Full detail in `ADR-046-resources-credential` §Implementation work items.

---

## 16. Tests

### 16.1 `src/` unit tests (`#[cfg(test)]` in `src/lib.rs`, `src/controller.rs`)

| Test | Validates |
| --- | --- |
| `Oo7SecretServicePort` trait API surface | All five port methods are async and take/return only non-secret types |
| `SecretServiceOwner` placement guard | Construction with non-Userd owner returns `SecretServiceProviderError::InvalidConsumer` |
| `collectionAlias` charset | Accepts valid aliases (with spaces); rejects empty string and control chars |
| `lockPolicy` state transitions | `fail-closed` returns error; `fail-degraded` sets Degraded status |
| `SecretServiceLeaseRequest` Debug | Emits only generation/placement/operation_count/expiry; no provider_id or operation field |
| `SecretServiceLeaseRef` Debug | Same redacted shape as request |
| `controller_process_spec_golden` | `reconcile()` generates a Process resource with `template = "secret-service-controller"`, `sandbox.namespaceClasses = [mount, pid, ipc]` (no `user` class; `Provider/system-systemd` rejects it; same-UID execution via `spec.userRef`), `sandbox.capabilityClasses = []`, `sandbox.seccompClass = "default-strict"`, `sandbox.noNewPrivileges = true`, `sandbox.startRoot = false`, `sandbox.readOnlyRoot = true`, `budget.memory.limit = "64Mi"`, `budget.pids.limit = 32`, `budget.fds.limit = 64`, `networkUsage = null`, no inline endpoint fields, an owned credential-service `Endpoint` resource, `readiness.class = "provider-defined"` |
| `finalize_does_not_emit_closure_audit` | `finalize()` emits a revoke outcome audit record but does not produce a `resource-deleted` closure event; verified by asserting no `Cleanup complete` record is captured by the test audit subscriber (closure record is audit-subsystem-only) |
| `status_first_state_golden` | Provider descriptor declares `resourceTypes: [Credential]` only and no Provider state Volume; ProviderStateSet query returns an empty grouping; Process specs contain no state mount; bounded non-secret lease/acquisition/retry fields are written only to `Credential.status` and the core Operation ledger; status rejects oversize/provider-detail overrun and excludes token bytes, keys, PSKs, Secret Service object paths, private paths, raw provider error bodies, and authority-conferring handles |

### 16.2 `tests/` Cargo integration tests (`cargo test -p d2b-provider-credential-secret-service`)

#### `tests/lifecycle.rs`

| Test | Validates |
| --- | --- |
| `acquire_token_unlocked` | `FakeOo7Port` unlocked; `AcquireToken` returns leaseHandle, sourceVersion, rotationGeneration=1, expiresAtUnixMs; `leaseState=Active`; `CredentialReady=True` |
| `refresh_token_extends_expiry` | After acquire; `RefreshToken` returns new expiresAtUnixMs; rotationGeneration unchanged; `lastRefreshedAt` updated |
| `revoke_token_idempotent` | `RevokeToken` returns `Revoked`; second call returns `AlreadyRevoked`; `leaseState=Revoked`; `CredentialReady=False` |
| `inspect_metadata_reflects_state` | `InspectMetadata` returns current leaseState, sourceVersion, rotationGeneration, expiresAtUnixMs |
| `proactive_rotation_success` | `rotation.policy=proactive`; `observe` sets `RotationDue=True` at window; `reconcile` issues new lease; `rotationGeneration` increments; old lease valid until new confirmed |
| `on_demand_no_auto_rotation` | `rotation.policy=on-demand`; controller does not auto-rotate; `RotationDue` never set |
| `on_expiry_rotation` | `rotation.policy=on-expiry`; controller acquires new lease after `leaseState=Expired`; `rotationGeneration` increments |
| `idempotency_key_no_double_issue` | Duplicate `issue_lease` with same idempotency key returns same grant; `issue_calls` count does not increment on duplicate |
| `revocation_on_provider_generation_immediate` | `revocation.onProviderGeneration=immediate`; `drain` handler calls `revoke_lease`; `leaseState=Revoked` |
| `revocation_on_provider_generation_drain` | `revocation.onProviderGeneration=drain-leases`; drain handler does not call `revoke_lease`; lease expires by deadline |
| `process_resource_created_with_correct_spec` | After first `reconcile()`, the owned Process resource exists with `template = "secret-service-controller"`, `sandbox.namespaceClasses = [mount, pid, ipc]` (no `user` class), `sandbox.capabilityClasses = []`, `sandbox.seccompClass = "default-strict"`, `sandbox.startRoot = false`, `sandbox.readOnlyRoot = true`, `budget.cpu` and `budget.memory` nested sub-fields present, `budget.pids.limit = 32`, `budget.fds.limit = 64`, `networkUsage = null`, no inline endpoint fields, an owned credential-service `Endpoint` resource, and `readiness.class = "provider-defined"` |

#### `tests/conformance.rs`

All `check_provider_conformance` arms pass for `d2b-provider-credential-secret-service`.

#### `tests/faults.rs`

| Test | Validates |
| --- | --- |
| `locked_state_fail_closed` | `FakeOo7Port.state = Locked`; `lockPolicy=fail-closed`; `AcquireToken` returns `credential-provider-unavailable`; status `ProviderUnavailable=True`, `leaseState=Unknown` |
| `locked_state_fail_degraded` | `lockPolicy=fail-degraded`; `phase=Degraded`; `ProviderUnavailable=True`; does not return error to caller |
| `unavailable_port` | `FakeOo7Port::issue_lease` returns `SecretServicePortError::Unavailable`; `credential-provider-unavailable` returned; bounded retry |
| `completion_unknown_no_auto_retry` | `FakeOo7Port::issue_lease` returns `CompletionUnknown`; provider does NOT retry with same record; consumer must re-initiate |
| `cardinality_limit` | `maxLeases=2`; third `AcquireToken` returns `credential-queue-pressure` |
| `lease_expired_reacquire` | `leaseState=Expired`; `AcquireToken` opens new lease; `rotationGeneration` increments |
| `rotation_failure_exhaustion` | Proactive rotation fails `maxRetries` times; `phase=Failed`, `outcome=rotation-failed` |
| `sign_challenge_schema_invalid` | `sign-challenge` operation class returns `credential-schema-invalid` immediately |

#### `tests/canary.rs`

| Test | Validates |
| --- | --- |
| `canary_absent_acquire_response` | `credential_canary` absent from `AcquireTokenResponse` outer DTO |
| `canary_absent_refresh_response` | `credential_canary` absent from `RefreshTokenResponse` outer DTO |
| `canary_absent_revoke_response` | `credential_canary` absent from `RevokeTokenResponse` |
| `canary_absent_inspect_response` | `credential_canary` absent from `InspectMetadataResponse` |
| `canary_absent_status_json` | `credential_canary` absent from status JSON serialization |
| `object_path_absent_all_responses` | `object_path_canary` absent from all response DTOs |
| `canary_absent_audit_records` | `credential_canary` and `object_path_canary` absent from all audit record JSON |
| `canary_absent_telemetry_attributes` | Neither secret canary nor any Credential name/ref/UID/digest canary is present in OTEL Resource or span attributes; generic collector-allowlisted Resource attributes remain |
| `metric_identity_labels_absent` | No descriptor key is `vm`, `zone`, `zone_id`, `zone_uid`, `credential_name`, `credential_ref`, `credential_uid`, `credential_digest`, `resource_name_digest`, or resource-name-derived; Credential name/ref/UID/digest and Zone-name canaries are absent from label values; Zone-name canaries are also absent from span attributes |
| `collector_allowlist_frame_accepted` | Complete metric/span frames with only generic allowlisted Resource attributes are accepted; injecting `d2b.credential.name` or any Credential name/ref/UID/digest key or value rejects the whole frame |
| `canary_absent_delivery_binding` | Neither canary present in delivery session binding parameters |

#### `tests/delivery.rs`

| Test | Validates |
| --- | --- |
| `delivery_session_binding_fields` | All binding contract fields (§8.2) present and correct for a successful `AcquireToken` |
| `delivery_zeroizing_buffer` | Sensitive output buffer is a zeroizing type; plaintext zeroed after extraction |
| `delivery_replay_safe_sequence` | A replayed delivery record at the same sequence number is rejected |
| `delivery_enrolled_keys_only` | NN-profile delivery attempt rejected immediately |
| `delivery_max_token_bytes_enforced` | Record exceeding `maxTokenBytes` causes immediate rejection and channel close |
| `delivery_single_use_channel` | Delivery channel closed and zeroized after confirmation; not reused |
| `delivery_no_auto_retry_on_ambiguous` | `CompletionUnknown` during delivery does not trigger automatic retry |

#### `tests/placement.rs`

| Test | Validates |
| --- | --- |
| `system_domain_rejected` | Construction with `domainFilter=system` returns `SecretServiceProviderError::InvalidConsumer` |
| `guest_agent_binding_rejected` | Construction with `placementBinding=guest-agent` (system-domain on Guest) returns `SecretServiceProviderError::InvalidConsumer` |
| `host_system_binding_rejected` | Construction with `placementBinding=host-system` returns `SecretServiceProviderError::InvalidConsumer` |
| `user_agent_on_host_accepted` | Construction with `domainFilter=user`, `executionRef=Host/<name>`, and `placementBinding=user-agent` succeeds |
| `user_agent_on_guest_accepted` | Construction with `domainFilter=user`, `executionRef=Guest/<name>`, and `placementBinding=user-agent` succeeds |
| `not_colocated_rejected` | Consumer and provider on different users returns `SecretServiceProviderError::NotColocated` |

### 16.3 `integration/` fixtures (invoked by `make test-integration` / `make test-host-integration`)

Files in `integration/` are shell scripts, Nix expressions, or container specs.
They are NOT run by `cargo test`.

#### `integration/container-service.sh`

Starts a container-backed `credential-secret-service` Provider process with a
`FakeOo7Port`-equivalent canary D-Bus stub, exercises the full d2b-bus routing
path for `AcquireToken`/`RefreshToken`/`RevokeToken`/`InspectMetadata`, and
verifies that canary values do not appear in any captured bus message or log.

#### `integration/host-placement.nix`

`runNixOSTest` scenario for Host execution context:
- Declares `Provider/credential-secret-service`, `Host/host-system`,
  `User/alice`, and `Credential/local-keyring` with
  `scope.executionRef=Host/host-system` in the Zone resource bundle.
- Verifies the controller process is launched as `User/alice` in the user
  domain of `Host/host-system`.
- Verifies `leaseState=Active` and `CredentialReady=True` after acquisition.
- Verifies that `d2b zone inspect dev Credential/local-keyring` shows no secret
  bytes in status output.

#### `integration/guest-placement.nix`

`runNixOSTest` scenario for Guest execution context:
- Declares `Provider/credential-secret-service`, `Guest/work-vm`,
  `User/alice`, and `Credential/guest-keyring` with
  `scope.executionRef=Guest/work-vm` in the Zone resource bundle.
- Verifies the controller process is launched as `User/alice` in the user
  domain of `Guest/work-vm`.
- Verifies `leaseState=Active` and `CredentialReady=True` after acquisition.
- Verifies that `d2b zone inspect dev Credential/guest-keyring` shows no secret
  bytes in status output.

#### `integration/cleanup-rollback.sh`

1. NixOS generation N declares `Credential/local-keyring`.
2. Generation N+1 removes it.
3. Verifies the resource row is atomically removed from the store after an
   event-only `Deleted` revision is written (no persisted `phase=Deleted` row
   remains in the store).
4. Verifies activation of generation N+1 does not block on cleanup finalizer.
5. Verifies rollback to generation N re-creates `local-keyring` from the
   retained bundle; a fresh lease is acquired.

#### `integration/README.md`

An optional `integration/README.md` may document fixture descriptions and
invocation instructions (`make test-integration`, `make test-host-integration`),
canary enforcement, and fake Secret Service usage. It is not separately mandated
by the workspace policy gate. The policy gate enforces only the four root items:
`src/`, `tests/`, `integration/`, and `README.md` at the crate root.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-credential-secret-service --lib --tests`
completes in ≤2 s warm-cache execution time (compilation excluded). They use a
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only — no
process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build,
KVM, USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny
temp fixtures. Any scenario needing those lives only in `integration/`, which
keeps a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test budget.

---

## 17. `README.md` required sections

The `packages/d2b-provider-credential-secret-service/README.md` MUST contain
these sections in order:

### Section 1: Provider identity

`providerRef = Provider/credential-secret-service`; implements `Credential`;
`user-agent` only; one controller binary; cardinality; Zone placement
constraints; versioning policy.

### Section 2: Config schema

`spec.config` fields (`collectionAlias`, `maxLeases`, `lockPolicy`), types,
defaults, constraints, and the worked Nix example from §3 above.

### Section 3: ResourceTypes managed

`Credential` lifecycle phases; status conditions owned (`CredentialReady`,
`RotationDue`, `ProviderUnavailable`, `LeaseRevoked`); finalizers owned
(`credential.d2bus.org/provider-revoke`).

### Section 4: Controllers, services, workers, and binaries

`secret-service-controller`: binary `d2b-provider-credential-secret-service`,
controller type, user domain, Host or Guest placement, cardinality
(`(Zone, User, executionRef)` triple).

### Section 5: Placement

Supported: `user-agent` on Host or Guest (when `domainFilter=user`). Rejected:
`host-system` and `guest-agent` (system-domain bindings). Error codes for each
rejected binding.

### Section 6: Dependencies and RBAC

Required Zone resources: `Host/<name>` or `Guest/<name>` (executionRef),
`User/<name>` (userRef), optional `Provider/<name>` (consumerRef); RBAC verbs
consumed; consumer Provider requirements; cross-resource ordering (Host or Guest
and User must be Ready before controller acquires first lease).

### Section 7: Security, state, and telemetry

Secret isolation model; D087 status-first state model; no Provider state Volume
for this credential Provider; ProviderStateSet is optional/query-time and empty;
bounded non-secret status fields (opaque non-authorizing lease handle, source
version, rotation generation, expiry timestamps, retry/outcome enums) plus core
Operation ledger state; no token bytes, Secret Service object paths, or private
runtime details in status, audit, telemetry, or storage; audit events; OTEL spans
and metrics; canary enforcement (`credential_canary`, `object_path_canary`).

### Section 8: Build, test, and integration commands

```bash
# Unit tests (src/ inline)
cargo test -p d2b-provider-credential-secret-service

# Hermetic Cargo integration tests (tests/)
cargo test -p d2b-provider-credential-secret-service --tests

# Container integration fixtures
make test-integration   # see integration/README.md

# NixOS runNixOSTest placement and cleanup fixtures
make test-host-integration   # see integration/README.md
```

### Section 9: Standalone-repo usage *(required before first release)*

How to consume outside the monorepo; flake input pattern; `inputs.d2b.inputs.nixpkgs.follows`
boilerplate; compatibility constraints; minimum toolkit version.

---

## 18. Removal contract

The following current symbols are removed only after the stated live successor
conditions are met:

| Symbol to remove | Location | Successor condition |
| --- | --- | --- |
| `CredentialProvider` trait | `d2b-realm-provider/src/provider.rs` | All three v3 Credential Provider controllers (`credential-secret-service`, `credential-entra`, `credential-managed-identity`) have tested replacement controllers consuming `d2b.credential.v3` service |
| `CredentialStatus` enum | `d2b-realm-provider/src/provider.rs` | Same as above |
| v2 `CredentialLease` / `CredentialLeaseState` | `d2b-contracts/src/v2_provider.rs` | All v3 callers migrate to `d2b-contracts/src/v3/credential.rs` |
| `d2b-provider-aca:managed_identity_client_id` raw field | `d2b-provider-aca/src/lib.rs` | `credential-managed-identity` Provider controller integrated; ACA Provider config uses `credentialRef` |
| v2 `CredentialProviderService` proto | `d2b-contracts/proto/v2/provider_credential.proto` | All v3 callers migrate to `d2b.credential.v3` |

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.
