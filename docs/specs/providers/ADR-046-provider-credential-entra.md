# Provider dossier: `credential-entra`

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-credential-entra` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 2 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `packages/d2b-provider-credential-entra/` |
| Depends on | `ADR-046-resources-credential`, `ADR-046-provider-model-and-packaging`, `ADR-046-componentsession-and-bus`, `ADR-046-resource-reconciliation`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support`, `ADR-046-resources-host-guest-process-user` |
| Decisions applied | D087 status-first state, D088/D089 status/spec layering, D090 expedited reconcile, D091 currency/upgrade, D092 Endpoint ResourceType, **D093 Entrablau identity Guest credential flow** |
| Supersedes | No Host login/token chains; no direct `EntraCredentialClient` production egress; no `DefaultAzureCredential`, environment, DBus, browser, or path discovery |

---

## 1. Provider identity and D093 outcome

| Field | Value |
| --- | --- |
| `providerRef` | `Provider/credential-entra` |
| Implements | `Credential` ResourceType |
| Provider schema ID | `credential-entra.d2bus.org/Credential/spec` |
| Status schema ID | `credential-entra.d2bus.org/Credential/status` |
| Login Endpoint purpose | `credential-entra.d2bus.org/entra-login-token` |
| Production token client | Entrablau identity Guest `Endpoint/<name>` implementing `credential-entra.d2bus.org/EntrablauLoginTokenService/v1` |
| Secret custody | Entrablau-enabled identity `Guest/<name>` only |
| Controller/agent custody | Secret-free; no token, refresh token, cookie, device-code authority, MSAL cache, browser state, or machine credential |

D093 grounds every `Provider/credential-entra` login and token acquisition in an
Entrablau-enabled identity Guest. The credential-entra controller and any helper
agent are orchestration components only: they validate ResourceRefs, authorize
ComponentSession routes, project bounded status, and coordinate lifecycle. They
never perform a Host login, never hold refresh tokens, never run a browser flow,
and never call Microsoft Entra directly.

**Non-exportable (D096).** Entra `Credential` resources are non-exportable: the
identity stays a same-Zone identity Guest and there is no `ResourceExport` for
Entra credentials/tokens unless a future, explicitly reviewed cross-Zone export
capability is added. Credential/token bytes never cross a Zone.

Each Entra `Credential` binds:

- an exact same-Zone `identityGuestRef: Guest/<name>`; and
- a stable `loginEndpointRef: Endpoint/<name>` whose producer is the Entrablau
  login/token service running inside that identity Guest.

The `scope.executionRef`/`consumerRef` pair names the exact allowed consumer. The
consumer may be the identity Guest itself or a different same-Zone Guest. If the
consumer is a different Guest, access-token bytes move only in end-to-end
`Noise_KK_25519_ChaChaPoly_SHA256` records from the Entrablau service to the
exact authenticated consumer process. The Host, d2b-bus, controllers, and all
intermediate transport see ciphertext only. Cross-Zone `identityGuestRef`,
`loginEndpointRef`, `scope.executionRef`, or consumer ResourceRefs are rejected.

---

## 2. Secret-free controller and placement model

`credential-entra` has a zero-secret control plane:

| Component | Placement | Secret custody | Responsibility |
| --- | --- | --- | --- |
| `entra-controller` | Zone controller placement selected by ProviderDeployment | None | Watch `Credential` resources, validate Entrablau bindings, manage status/finalizers, request expedited reconciles, and register capability/status schemas. |
| `entra-agent` (optional helper) | Inside `identityGuestRef` or the exact allowed consumer Guest | None | Open authenticated ComponentSessions to `loginEndpointRef`, enforce `consumerRef`/scope/RBAC, and report bounded observations. It does not decrypt access-token records. |
| Entrablau login/token service | Inside `identityGuestRef` | Owns all Entra secrets and login state | Performs interactive login, refresh-token handling, token cache, TPM-bound machine state, Entra network/TLS, and end-to-end access-token delivery. |

A `Credential` using `Provider/credential-entra` is never scoped to `Host/*`.
`scope.executionRef` and `identityGuestRef` must resolve to `Guest/<name>` in the
same Zone. Host placement is rejected because it would recreate Host credential
custody. User-domain or system-domain consumer processes are allowed only inside
Guests that the credential spec and RBAC authorize.

The controller/agent Process templates declare no Provider state Volume, no
secret mount, no browser path, no DBus path, no ambient network claim for Entra,
and no environment-derived credential input. Any helper agent uses the standard
bus/Endpoint launch-ticket path and receives only non-secret ResourceRefs,
generations, deadlines, and route digests.

---

## 3. Provider and Credential spec schemas

### 3.1 Provider `spec.config` (bounded, non-secret)

```yaml
spec:
  artifactId: credential-entra-provider
  config:
    maxLeases: 64
    interactionPolicy: interaction-required
```

| Field | Type | Required | Validation rule | Default |
| --- | --- | --- | --- | --- |
| `maxLeases` | u32 | No | 1..256; caps concurrent access-token lease requests brokered for this Provider | `64` |
| `interactionPolicy` | enum | No | `interaction-required` or `fail-closed`; controls whether `BeginLogin` may be started when no authenticated Entrablau session exists | `interaction-required` |

Provider config contains no tenant secret, authority URL, token endpoint, client
secret, certificate path, store path, browser path, DBus address, or environment
variable name. Tenant and authority policy that is necessary for Entra login is
owned by the Entrablau Guest service configuration supplied by the sibling
package, not by d2b core or the credential-entra controller.

### 3.2 Credential base fields used by D093

`identityGuestRef` and `loginEndpointRef` are Credential base fields supplied by
`ADR-046-resources-credential`. This Provider requires both.

```yaml
apiVersion: resources.d2bus.org/v3
type: Credential
metadata:
  name: work-entra
  zone: work
spec:
  providerRef: Provider/credential-entra
  identityGuestRef: Guest/work-identity
  loginEndpointRef: Endpoint/work-identity-entra-login-token
  scope:
    executionRef: Guest/aca-gateway
    domainFilter: system
    userRef: null
  consumerRef: Provider/runtime-azure-container-apps
  audience: azure-resource-manager
  allowedOperations: [acquire-token, refresh-token]
  rotation:
    policy: proactive
    proactiveWindowMs: 300000
    maxLeaseLifetimeMs: 3600000
  revocation:
    onOwnerDelete: immediate
    onProviderGeneration: immediate
  provider:
    schemaId: credential-entra.d2bus.org/Credential/spec
    schemaVersion: 1.0.0
    settings:
      tokenPurpose: azure-resource-manager
      loginMode: entrablau-guest
```

Rules:

1. `identityGuestRef` resolves to a `Guest/<name>` in the same Zone.
2. `loginEndpointRef` resolves to an `Endpoint/<name>` in the same Zone whose
   `purpose = credential-entra.d2bus.org/entra-login-token`, whose producer runs
   inside `identityGuestRef`, and whose `endpointGeneration` is current.
3. `scope.executionRef` resolves to the same Guest as `identityGuestRef` or to a
   different same-Zone Guest explicitly permitted by `consumerRef`, scope, and
   RBAC.
4. `consumerRef` is required and must name the exact Provider/component allowed
   to receive access-token plaintext.
5. `allowedOperations` is a non-empty subset of `acquire-token`, `refresh-token`,
   `revoke-token`, and `inspect-metadata`; `sign-challenge` is not provided by
   this Entrablau token service unless a future schema version adds it.
6. Provider-specific settings are bounded desired metadata only. They do not
   carry access tokens, refresh tokens, cookies, login URLs, device codes,
   store paths, service principal secrets, or tenant-private diagnostics.

---

## 4. Entrablau identity Guest dependency and Endpoint contract

### 4.1 External sibling classification

The identity Guest's NixOS system is consumer-composed with the sibling
`vicondoa/entrablau.nix` module. d2b core never imports that flake and never
vendors the Entrablau implementation.

```nix
inputs.entrablau.url = "github:vicondoa/entrablau.nix";
inputs.entrablau.inputs.nixpkgs.follows = "nixpkgs";

# Inside the NixOS system closure for Guest/work-identity only.
d2b.zones.work.resources.work-identity.spec.config.imports = [
  inputs.entrablau.nixosModules.default
];
```

This is an ADR-only / external-sibling integration target for ADR 0046. The
sibling/Guest package is expected to supply the manifest-declared Process and
Endpoint contract below. This dossier specifies the required contract; it does
not claim that current d2b core implements Entrablau.

The sibling package owns all guest-local secret and private state: Himmelblau /
Entrablau enrollment, TPM binding, machine credential, refresh-token state, token
cache, interactive login implementation, browser/device/desktop integration,
network/TLS policy for Microsoft Entra, and any large/private diagnostics. That
state is inside the identity Guest and outside d2b resource status. It is not a
d2b Provider state Volume and is never mounted into the Host.

### 4.2 Dependency alias

`Provider/credential-entra` declares one production dependency alias:

| Alias | Resolves to | Required | Notes |
| --- | --- | --- | --- |
| `entra-login-token` | `Credential.spec.loginEndpointRef` | Yes | `Endpoint/<name>` with purpose `credential-entra.d2bus.org/entra-login-token`, producer inside `identityGuestRef`, same Zone, ComponentSession attachment. |

The alias is resolved by ResourceRef and Endpoint generation, not by a Unix path,
store path, DBus name, process ID, environment variable, or hostname.

### 4.3 Endpoint resource schema

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: work-identity-entra-login-token
  zone: work
  ownerRef: Guest/work-identity
spec:
  providerRef: Provider/credential-entra
  producerRef: Process/work-identity-entrablau-login-token
  endpointClass: service
  transport: unix
  purpose: credential-entra.d2bus.org/entra-login-token
  serviceFingerprint: credential-entra.d2bus.org/EntrablauLoginTokenService/v1
  locality: guest-local
  visibility: provider
  attachmentPolicy: component-session
  consumerPolicy:
    allowedSubjects:
      - Provider/credential-entra
      - Provider/runtime-azure-container-apps
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
status:
  readiness: Ready
  endpointGeneration: 7
  observedProducerGeneration: 3
  connectionAvailability: available
```

Endpoint spec/status never contains raw Unix paths, fd numbers, browser URLs,
login URLs, device codes, cookies, tenant-private endpoints, token cache paths,
TPM paths, credential file paths, access tokens, refresh tokens, or user PII.
Endpoint resolution returns an authorized launch ticket or fails closed with
`endpoint-resolve-denied`.

The example deliberately uses the canonical `provider` visibility value.
`owner` would exclude the exact external consumer Provider, while `zone` would
make every same-Zone subject a visibility candidate. `consumerPolicy` then
narrows resolution to the orchestration Provider and the Credential's exact
consumer Provider; ordinary Role/RoleBinding authorization must also allow
`resolve`. Admission accepts only `owner|provider|zone`, denies unknown
visibility aliases, and requires every present `consumerPolicy` dimension to
match.

### 4.4 Typed service methods

Service fingerprint: `credential-entra.d2bus.org/EntrablauLoginTokenService/v1`.
All methods are ComponentSession methods. The interactive byte/UI stream is a
named stream on the authenticated session, not status text.

| Method | Required Role permission | Direction | Secret output? | Purpose |
| --- | --- | --- | --- | --- |
| `BeginLogin` | `use-credential/acquire-token` | request/reply + named stream | No status secret; UI bytes stay on stream | Start or resume an Entrablau interactive login session inside `identityGuestRef`. Opens optional named stream `credential-entra.d2bus.org/login-ui`. |
| `ObserveLogin` | `use-credential/observe` | request/reply | No | Return bounded non-secret observation for `interactionState`, session generation, deadline, and challenge metadata. |
| `CancelLogin` | `use-credential/revoke` | request/reply | No | Cancel a pending login session by generation; idempotent. |
| `AcquireAccessTokenLease` | `use-credential/acquire-token` | request/reply + sensitive KK record | Yes, only in KK record to consumer | Issue an on-demand access-token lease for `audience`/operation; outer reply carries metadata only. |
| `RevokeAccessTokenLease` | `use-credential/revoke` | request/reply | No | Revoke a non-secret lease handle or mark it unusable. |
| `InspectAccessTokenLease` | `use-credential/observe` | request/reply | No | Return bounded non-secret lease state. |

`BeginLogin`, `ObserveLogin`, and `CancelLogin` are the Credential base
interactive-login operations. The service may conduct browser, device, desktop,
or brokered login UI inside the identity Guest, but no device code conferring
authority, login URL, cookie, token, account UPN, display name, tenant-private
identifier, or user PII is copied into status, audit, OTEL, or outer RPC DTOs.
`challengeMetadata` is bounded and non-secret: examples are a closed
`challengeKind`, an expiry timestamp, a user-action class, and optional
operator-facing text that has passed redaction. It is never sufficient to
complete login outside the Guest.

---

## 5. Interactive login flow

### 5.1 State model

Credential base status owns the common interaction fields:

| Field | Allowed values/content |
| --- | --- |
| `status.resource.interactionState` | `NotRequired`, `Required`, `Starting`, `AwaitingUser`, `Authenticated`, `Failed`, `Unknown` |
| `status.resource.loginSessionGeneration` | Monotonic u64 issued by the Entrablau service; non-secret |
| `status.resource.loginDeadline` | RFC3339 or null; controller never waits for a human past this deadline |
| `status.resource.challengeMetadata` | Bounded non-secret object; no authority-conferring bytes or PII |

Entra-specific status under `credential-entra.d2bus.org/Credential/status`
contains only provider observations such as endpoint generation, last observed
login generation, lease state, bounded retry counters, and closed error codes.
It never duplicates base status fields.

### 5.2 `BeginLogin`

1. The caller must be authorized for the `Credential/<name>` by `consumerRef`,
   scope, the exact `use-credential/acquire-token` Role verb/subresource pair,
   and same-Zone ResourceRefs.
2. The credential-entra helper opens an authenticated ComponentSession to
   `loginEndpointRef` inside `identityGuestRef`.
3. `BeginLogin` binds the Credential UID/generation, `identityGuestRef`,
   `loginEndpointRef`, Endpoint generation, `consumerRef`, operation ID,
   requested deadline, and schema fingerprints into the session prologue.
4. Entrablau starts or resumes login inside the Guest and, if UI is required,
   attaches named stream `credential-entra.d2bus.org/login-ui` directly between
   the caller and the Entrablau service. The stream bytes are not status,
   audit, logs, metrics, or d2b-bus plaintext inspection data.
5. The controller projects `interactionState: Starting` or `AwaitingUser` and the
   bounded deadline/challenge metadata. It does not block until the user
   completes login.

### 5.3 `ObserveLogin` and `CancelLogin`

`ObserveLogin` returns the latest non-secret observation for the current
`loginSessionGeneration`. It may transition status to `Authenticated`, `Failed`,
`Required`, or `Unknown`. `CancelLogin` is idempotent and generation-bound; a
stale cancel cannot affect a newer login session.

### 5.4 D090 expedited reconcile behavior

For `Create`, `UpdateSpec`, and `Delete` with `waitForReconcile`, the controller
performs no external effect, status mutation, finalizer change, or Endpoint
session until Core supplies `CommittedRevisionProof`. After the durable commit,
the expedited pass may return a projected status with
`interactionState: Required`, `Starting`, or `AwaitingUser`. It never waits for
human login past `loginDeadline`; if the deadline would be exceeded, the response
is `Blocked` or `Progressing` with the projected interaction state and
`statusPersistence: pending|committed`.

---

## 6. On-demand access-token lease delivery

The Credential service exposes access tokens only as short-lived leases requested
on demand. `credential-entra` itself never stores or decrypts token bytes.

```text
Entrablau service (Guest/work-identity)
  owns refresh token + token cache + Entra TLS
        │
        │  Noise_KK_25519_ChaChaPoly_SHA256 sensitive records
        │  prologue binds Credential UID/gen, Endpoint gen,
        │  identityGuestRef, consumerRef, audience, operation, deadline,
        │  schema fingerprint, RBAC revision, and maxTokenBytes
        ▼
Exact authorized consumer process (same-Zone Guest)
  uses access token in memory for the requested operation only
```

Security requirements reuse the D055/D056 end-to-end KK delivery contract:

1. Static keys are enrolled before delivery. NN/NX/unauthenticated profiles are
   rejected.
2. The raw access token reaches only the exact authenticated consumer process
   permitted by `Credential.spec.consumerRef`, `scope`, and RBAC.
3. The Host, bus, controller, helper agent, relay, and any intermediate Provider
   see only encrypted records and bounded metadata.
4. Refresh tokens, token caches, machine credentials, cookies, account session
   state, and private login state never leave `identityGuestRef`.
5. Outer RPC replies carry only non-secret lease metadata: lease handle digest,
   expiry, issued-at time, generation, interaction state, and stable outcome.
6. Ambiguous delivery is not success and is not automatically replayed. The
   consumer must retry with a new operation/session.
7. Token buffers are zeroizing in the Entrablau service and consumer. No token,
   prefix/suffix, bearer string, JWT, cookie, refresh token, or device code is
   logged, audited, traced, labeled, stored in status, or written to a Volume.

`AcquireAccessTokenLease` may return `interactionState: Required` rather than a
token if no authenticated Entrablau session exists or if the identity Guest says
interactive login is required. That is a recoverable interaction condition, not a
policy denial.

---

## 7. Production token API client

The token API client remains injected and typed for testability, but the only
production implementation is the Entrablau identity Guest Endpoint described in
§4.

```rust
trait EntraTokenLeaseClient: Send + Sync {
    async fn begin_login(&self, request: BeginLoginRequest) -> Result<LoginObservation, CredentialError>;
    async fn observe_login(&self, request: ObserveLoginRequest) -> Result<LoginObservation, CredentialError>;
    async fn cancel_login(&self, request: CancelLoginRequest) -> Result<LoginObservation, CredentialError>;
    async fn acquire_access_token_lease(&self, request: TokenLeaseRequest) -> Result<TokenLeaseMetadata, CredentialError>;
    async fn revoke_access_token_lease(&self, request: RevokeLeaseRequest) -> Result<RevokeLeaseResult, CredentialError>;
    async fn inspect_access_token_lease(&self, request: InspectLeaseRequest) -> Result<TokenLeaseMetadata, CredentialError>;
}
```

The fake implementation used by hermetic tests may simulate login states and
access-token delivery, but production construction resolves
`Credential.spec.loginEndpointRef` and opens a ComponentSession to that Endpoint.
The credential-entra controller and helper agent perform no direct Entra network
egress; Entrablau owns Microsoft Entra network/TLS flows inside the identity
Guest.

There is no `DefaultAzureCredential`, no Azure SDK default chain, no environment
variable credential source, no DBus credential source, no filesystem token cache
path, no developer-tool credential path, no Host login fallback, no browser
fallback on the Host, and no env var fallback. Removing or disabling the
Entrablau Endpoint is a hard `interactionState: Unknown` or
`ProviderUnavailable` condition, not a fallback trigger.

---

## 8. Status-first state and zero-secret invariant

`Provider/credential-entra` declares no Provider state Volume. The ProviderStateSet
is empty. Entrablau's private state is guest-local state owned by the sibling
package inside the identity Guest, not a d2b Provider Volume.

Credential status may contain only bounded non-secret observations:

| Surface | Allowed | Forbidden |
| --- | --- | --- |
| `Credential.status.resource` | interaction state, login session generation, login deadline, bounded challenge metadata, lease state, expiry/issued timestamps | Credential name/ResourceRef/UID/identity digest; tokens, refresh tokens, device codes, login URLs, cookies, account names, tenant-private IDs, PII |
| `Credential.status.provider.details` | Endpoint generation, identity Guest generation digest, retry counters, closed error/outcome enums, non-authorizing lease handle digests | Credential name/ResourceRef/UID/identity digest; token bytes, Entra response bodies, MSAL cache entries, filesystem paths, DBus names, browser profile paths |
| Authorized bounded Zone audit | operation class, RBAC verb/subresource, result code, generation, and base `resource_name_digest` after authorization | raw Credential name/ResourceRef/UID; token/refresh/cookie/device-code bytes, login URL, user PII, cloud response bodies, host paths |
| OTEL Resource attributes | only applicable generic collector-allowlisted attributes: `d2b.zone`, `d2b.provider`, `d2b.component`, `service.name`, `service.namespace`, `service.version` | Credential name/ResourceRef/UID/identity digest and every Credential-derived attribute |
| Span attributes, metric labels, logs/Debug, status, and errors | closed provider/operation/outcome values that do not identify a Credential | Credential name/ResourceRef/UID/identity digest; route/session-binding digests derived from Credential identity; all secret/private values forbidden above |

The zero-secret invariant applies across spec, status, resource store, audit,
OTEL, logs, debug output, process descriptors, launch tickets, Endpoint spec/status,
and outer RPC DTOs. Secret bytes are present only inside the Entrablau Guest and,
for access tokens, inside the end-to-end KK record and exact consumer process.
Credential identity is a separate observability restriction: it remains in the
resource envelope and authenticated internal routing/session bindings, but
among observable outputs only the authorized bounded audit record may contain
it, and that record uses `resource_name_digest` rather than raw identity.

---

## 9. Nix composition

A consumer composes the identity Guest and then declares Credentials against the
Endpoint exported by that Guest. d2b core does not import the sibling flake.

```nix
{
  inputs.entrablau.url = "github:vicondoa/entrablau.nix";
  inputs.entrablau.inputs.nixpkgs.follows = "nixpkgs";

  # Identity Guest NixOS system. The sibling module declares the Entrablau
  # login/token Process and the Endpoint purpose
  # credential-entra.d2bus.org/entra-login-token.
  d2b.zones.work.resources.work-identity = {
    type = "Guest";
    spec = {
      providerRef = "Provider/runtime-cloud-hypervisor";
      defaultDomain = "system";
      allowedDomains = [ "system" "user" ];
      systemArtifactId = "work-identity-system";
      config.imports = [ inputs.entrablau.nixosModules.default ];
    };
  };

  d2b.zones.work.resources.work-identity-entra-login-token = {
    type = "Endpoint";
    spec = {
      providerRef = "Provider/credential-entra";
      producerRef = "Process/work-identity-entrablau-login-token";
      endpointClass = "service";
      transport = "unix";
      purpose = "credential-entra.d2bus.org/entra-login-token";
      serviceFingerprint = "credential-entra.d2bus.org/EntrablauLoginTokenService/v1";
      locality = "guest-local";
      visibility = "provider";
      attachmentPolicy = "component-session";
      consumerPolicy = {
        allowedSubjects = [
          "Provider/credential-entra"
          "Provider/runtime-azure-container-apps"
        ];
        allowedOperations = [ "resolve" ];
      };
      lifecyclePolicy = "recycle-with-producer";
    };
  };

  d2b.zones.work.resources.work-entra = {
    type = "Credential";
    spec = {
      providerRef = "Provider/credential-entra";
      identityGuestRef = "Guest/work-identity";
      loginEndpointRef = "Endpoint/work-identity-entra-login-token";
      scope.executionRef = "Guest/aca-gateway";
      scope.domainFilter = "system";
      consumerRef = "Provider/runtime-azure-container-apps";
      audience = "azure-resource-manager";
      allowedOperations = [ "acquire-token" "refresh-token" ];
      rotation.policy = "proactive";
      rotation.proactiveWindowMs = 300000;
      rotation.maxLeaseLifetimeMs = 3600000;
      revocation.onOwnerDelete = "immediate";
      revocation.onProviderGeneration = "immediate";
      provider = {
        schemaId = "credential-entra.d2bus.org/Credential/spec";
        schemaVersion = "1.0.0";
        settings = {
          tokenPurpose = "azure-resource-manager";
          loginMode = "entrablau-guest";
        };
      };
    };
  };
}
```

Eval-time validation rejects missing `identityGuestRef`/`loginEndpointRef`, any
cross-Zone reference, any Endpoint whose producer is not inside the identity
Guest, Host-scoped credential placement, an absent `consumerRef`, mismatched
consumer execution scope, any Endpoint visibility outside
`owner|provider|zone`, a `provider`-visible login Endpoint whose
`consumerPolicy.allowedSubjects` omits either `Provider/credential-entra` or the
Credential's exact `consumerRef`, and any string that matches known
secret/token shapes.

---

## 10. Reset, upgrade, and deletion

Credential reset and deletion affect only d2b credential metadata and active
lease/session observations unless the operator explicitly requests identity Guest
state destruction through the Entrablau sibling's own interface.

| Operation | Identity Guest TPM/login state |
| --- | --- |
| Credential spec update | Preserved unless the identity Guest or Endpoint binding changes; old sessions are generation-invalidated. |
| Provider generation update | Active access-token leases are revoked or allowed to drain per `revocation.onProviderGeneration`; Entrablau refresh state is preserved. |
| Credential delete | Active leases are revoked; status/finalizer cleanup occurs; Entrablau enrollment and TPM state are preserved. |
| Explicit identity reset | Must be a separate, auditable operation owned by the Entrablau Guest/sibling and must declare whether TPM/login state is preserved or destroyed. |

A reset must never silently wipe or recreate the identity Guest TPM/login state.
Destroying that state can force Entra re-enrollment and therefore requires an
explicit operator action and audit record. Preserving state is the default for
credential-entra lifecycle changes.

---

## 11. RBAC and errors

### RBAC verbs

| Verb | Who | Purpose |
| --- | --- | --- |
| `get`, `list`, `watch` | Authorized readers | Inspect non-secret spec/status. |
| `create`, `update-spec`, `delete` plus `admin-credential/admin` | Authorized deployer/configuration controller | Administer Credential resource lifecycle; neither permission implies the other. |
| `update-status` | `Provider/credential-entra` controller only | Write projected interaction/lease observations. |
| `update-finalizers` | `Provider/credential-entra` controller | Revoke/finalize Credential resources. |
| `use-credential/acquire-token` | Exact `consumerRef` plus RBAC | Begin login and acquire/refresh access-token leases permitted by `spec.allowedOperations`. |
| `use-credential/observe` | Exact `consumerRef` plus RBAC | Observe login or inspect bounded lease/status metadata. |
| `use-credential/revoke` | Exact `consumerRef` plus RBAC | Cancel login or revoke an access-token lease. |
| `admin-credential/admin` | Authorized operator/configuration controller | Gate explicit Credential lifecycle and identity-reset administration. |

The value after `/` is an exact entry in the existing Role rule
`subresources` field. The dossier defines no `operationClasses` Role field and
no method-name verbs such as `begin-login`. A consumer Role is shaped as:

```yaml
rules:
  - resourceTypes: [Credential]
    verbs: [use-credential]
    subresources: [acquire-token, observe, revoke]
    resourceNames: [work-entra]
    zones: [work]
    executionRefs: [Guest/aca-gateway]
    sessionVerbs: []
```

Service admission authenticates the exact consumer, checks the mapped
verb/subresource before Endpoint dispatch, then independently checks the exact
method operation class against `Credential.spec.allowedOperations`. Empty,
wildcard, unknown, or verb/subresource-mismatched Credential grants fail closed.
Administrative create/update/delete additionally require an
`admin-credential` rule with `subresources: [admin]` and the corresponding
ordinary mutation verb.

### Stable error codes

| Code | Condition |
| --- | --- |
| `credential-interaction-required` | Entrablau reports login is required before token acquisition. |
| `credential-login-starting` | `BeginLogin` accepted; user interaction stream starting. |
| `credential-login-cancelled` | Generation-bound login was cancelled. |
| `credential-login-timeout` | Login did not complete before `loginDeadline`. |
| `credential-endpoint-unavailable` | `loginEndpointRef` is absent, not Ready, or generation mismatched. |
| `credential-endpoint-generation-mismatch` | Caller observed stale Endpoint generation. |
| `credential-consumer-mismatch` | Authenticated caller does not match `consumerRef`/scope/RBAC. |
| `credential-placement-mismatch` | Host placement, cross-Zone reference, or non-Guest identity binding attempted. |
| `credential-provider-unavailable` | Entrablau service or identity Guest unavailable. |
| `credential-operation-denied` | RBAC or Entrablau policy denied the requested operation. |
| `credential-redaction-violation` | A candidate status/audit/log field contains secret-shaped content. |

All error messages are bounded stable text. They do not include login URLs,
device codes, user identifiers, tenant-private identifiers, HTTP bodies, paths,
DBus names, tokens, refresh tokens, cookies, a Credential name/ResourceRef/UID,
or any digest derived from Credential identity.

---

## 12. Work item

### ADR046-cred-entra-001

| Field | Value |
| --- | --- |
| Dependency/owner | `ADR046-credential-001` (Credential base fields and status), `ADR046-credential-002` (`d2b.credential.v3` service), D092 Endpoint, D093 Entrablau identity Guest decision, credential-entra owner |
| Current source | `d2b-realm-provider/src/credential.rs:OpaqueAzureRef` remains a bounded opaque identifier reuse source only; no Host login/token implementation is retained |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-credential-entra/src/{lib.rs,controller.rs,service.rs,controller_main.rs,agent_main.rs,audit.rs,telemetry.rs}` and corresponding tests/integration docs |
| Detailed design | Implement secret-free controller/helper; require Credential base `identityGuestRef` and `loginEndpointRef`; resolve dependency alias `entra-login-token`; validate same-Zone Guest placement and exact `consumerRef`; implement typed `EntraTokenLeaseClient` whose production implementation is the Entrablau Endpoint; implement `BeginLogin`/`ObserveLogin`/`CancelLogin` projection to Credential base interaction status; admit service calls only through canonical `use-credential` Role rules with exact `acquire-token`/`observe`/`revoke` subresources and administrative lifecycle only through `admin-credential/admin` plus the ordinary mutation verb, with no Credential-specific Role fields; emit the login Endpoint with canonical `visibility = provider` and an exact `consumerPolicy` for the orchestration Provider and configured consumer; route access-token leases end-to-end from Entrablau service to exact consumer over Noise_KK; keep all refresh/login/TPM state inside the Entrablau Guest; declare no Provider state Volume; reject Host placement and all ambient fallback chains; enforce Credential name/ResourceRef/UID/digest as audit-only observable identity, with only `resource_name_digest` retained in authorized bounded audit and no Credential identity in status/errors/logs/OTEL Resource or span attributes/metric labels. Primary reuse disposition: `adapt`. Preserved source-plan detail: Adapt tests/types where non-secret; replace production token acquisition with Entrablau Endpoint client. |
| Integration | Consumer composes `inputs.entrablau.nixosModules.default` into the identity Guest; sibling package declares login/token Process and Endpoint; d2b Credential resource binds `identityGuestRef`, `loginEndpointRef`, and `consumerRef` |
| Data migration | Full v3 reset of d2b Credential metadata; Entrablau Guest state is preserved unless explicitly destroyed by the sibling-owned reset flow |
| Validation | See §13 |
| Removal proof | Old abstract Host credential paths and any direct Entra effect client are deleted only after the Entrablau Endpoint-backed provider passes the test matrix |

---

## 13. Tests

### Unit and hermetic integration tests

| Test | Purpose |
| --- | --- |
| `test_fake_guest_login_success` | Fake Entrablau Guest reports `Required → Starting → AwaitingUser → Authenticated`; status carries only interaction fields and bounded challenge metadata. |
| `test_fake_guest_login_required` | Token acquisition without an authenticated Entrablau session returns `interactionState: Required`, no token, and no denial. |
| `test_fake_guest_login_cancel` | `CancelLogin` is generation-bound and idempotent; stale cancel cannot cancel a newer session. |
| `test_fake_guest_login_timeout` | `loginDeadline` expiration returns `credential-login-timeout`; expedited reconcile does not wait past deadline. |
| `test_fake_guest_login_restart_reobserve` | Controller/helper restart reconstructs status by observing Entrablau Endpoint; status is observation, not authority. |
| `test_endpoint_unavailable` | Missing/not Ready `loginEndpointRef` yields `credential-endpoint-unavailable` and no fallback. |
| `test_endpoint_generation_mismatch` | Stale Endpoint generation invalidates cached sessions and requires reacquire. |
| `test_endpoint_visibility_consumer_policy` | Login Endpoint accepts only `owner`, `provider`, or `zone`, uses `provider`, and requires exact `consumerPolicy.allowedSubjects` for `Provider/credential-entra` plus the configured `consumerRef`; `zone`, omitted consumer, aliases, and mismatches fail the dossier conformance case. |
| `test_host_placement_rejected` | Any `Host/*` `identityGuestRef`, `scope.executionRef`, or Host-system placement fails closed. |
| `test_same_zone_accepted_cross_zone_rejected` | Same-Zone identity/consumer Guest works; any cross-Zone ResourceRef is rejected at eval/admission. |
| `test_exact_consumer_rbac` | Only the authenticated `consumerRef` process can receive token delivery; other callers fail before Endpoint dispatch. |
| `test_credential_role_subresource_matrix` | `use-credential` admits only exact `acquire-token`/`observe`/`revoke` mappings; `admin-credential/admin` plus the ordinary mutation verb gates lifecycle; empty/wildcard/unknown/mismatched subresources and method-name verbs fail before Endpoint dispatch. |
| `test_token_refresh_redaction` | Access token, refresh token, cookies, device codes, login URLs, and MSAL cache canaries are absent from spec/status/audit/OTEL/logs/Debug. |
| `test_credential_identity_audit_only` | Credential name, canonical ResourceRef, UID, and derived-digest canaries are absent from status, errors, logs/Debug, every OTEL Resource/span attribute, and every metric label; the authorized bounded audit record retains only `resource_name_digest`, and an unauthorized request cannot elicit identity-bearing audit. Generic allowlisted OTEL Resource attributes remain present. |
| `test_e2e_record_only_delivery` | Raw access token appears only in the Entrablau→consumer Noise_KK record; bus/controller/helper see ciphertext only. |
| `test_no_ambient_fallbacks` | Production code path never constructs SDK default chains, environment credential sources, DBus/browser/path fallbacks, Host login flows, or direct Entra egress. |
| `test_nix_composition_identity_guest` | Consumer-composed `inputs.entrablau.nixosModules.default` in the identity Guest declares the expected Process and Endpoint schema without d2b core importing the sibling flake. |
| `test_tpm_preserve_on_credential_reset` | Credential reset/delete preserves Entrablau TPM/login state by default. |
| `test_tpm_destroy_requires_explicit_reset` | Destroying identity Guest TPM/login state requires explicit sibling-owned reset intent and emits a distinct audit record. |
| `test_status_size_and_challenge_bounds` | `challengeMetadata` and provider details obey D087/D088 size bounds and reject secret-shaped strings. |
| `test_d090_interaction_required_projection` | `waitForReconcile` after create/update returns projected `interactionState: Required` or `AwaitingUser` without waiting for human login. |

### Nix/eval assertions

- `identityGuestRef` and `loginEndpointRef` are required for every
  `Provider/credential-entra` Credential.
- `loginEndpointRef` must have purpose
  `credential-entra.d2bus.org/entra-login-token` and producer inside
  `identityGuestRef`.
- `consumerRef` is required and must match the authorized consumer Provider.
- The login Endpoint uses canonical `visibility = "provider"` and an exact
  `consumerPolicy.allowedSubjects` containing `Provider/credential-entra` and
  the configured `consumerRef`.
- Host placement and cross-Zone ResourceRefs are rejected.
- No Provider state Volume is emitted for `credential-entra`; Entrablau private
  state remains guest-local under the sibling package.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has p95 ≤50 ms with no wall-clock
sleep, and `cargo test -p d2b-provider-credential-entra --lib --tests`
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

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass —
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.

---

## 14. Current-code fit

| Item | Value |
| --- | --- |
| Evidence class | Full Entrablau-backed login/token acquisition is ADR-only in this branch; sibling integration is external. |
| Retained concepts | Bounded opaque identifiers, `ExactConsumer` authorization, redacted Debug/canary discipline, D055/D056 Noise_KK token delivery. |
| Replaced concepts | No abstract Host-login path, no direct `EntraCredentialClient` production Entra egress, no effect-port proxy through consumer runtime, no Host/user-agent token custody, no ambient fallback chains, and no Provider state Volume for Entra credentials. |
| Zero-secret invariant | Preserved and strengthened: controller/helper are secret-free; refresh/private login state lives only in the Entrablau identity Guest; access tokens go only to exact consumers over end-to-end KK records; Credential identity is observable only as the authorized bounded audit digest and is absent from status/errors/logs and every OTEL/metric identity surface. |
