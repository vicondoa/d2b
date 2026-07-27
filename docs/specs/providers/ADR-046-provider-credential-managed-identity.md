# ADR 0046 Provider dossier: `credential-managed-identity`

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-credential-managed-identity` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-credential-managed-identity` crate, Credential controller, Nix Credential compiler |
| Depends on | `ADR-046-resources-credential`, `ADR-046-componentsession-and-bus`, `ADR-046-provider-model-and-packaging`, `ADR-046-resource-reconciliation`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support` |
| Supersedes | Current v3 `ManagedIdentityRef`, `managed_identity_client_id` ACA config field, `CredentialProvider` trait (status/enrollment-only) in `d2b-realm-provider/src/credential.rs` and `provider.rs` |

## Purpose

This dossier specifies the `credential-managed-identity` Provider for d2b
3.0. It covers every aspect required to implement, test, integrate, and
operate the Provider from a fresh baseline:

- `Provider.spec.config` schema with all field types, bounds, and invariants;
- two-process topology: one Zone-wide secret-free `managed-identity-controller`
  plus one `managed-identity-agent` service Process per Credential/SDK-consumer
  binding, co-located at the declared `executionRef`;
- SDK-consumer co-location contract for Azure VM and ACA Guest contexts;
- the injected `ManagedIdentityCredentialClient` held exclusively by the agent
  process - no ambient IMDS chain, no environment-variable fallback, no
  developer-credential path;
- all `d2b.credential.v3` service methods with exact controller/agent
  dispatch split, their lease types, and the state machine;
- raw sensitive-output delivery exclusively over a dedicated
  `Noise_KK_25519_ChaChaPoly_SHA256` end-to-end ComponentSession terminated
  by the agent process;
- canonical Process resource templates for both roles;
- `ExactSdkConsumer` validation on the authenticated bus subject independently
  of scope-field declarations;
- placement, RBAC, zero-secret-bytes invariant and redaction;
- status conditions, error codes, audit with Deleted-revision closure, and OTEL;
- Nix authoring shape, eval-time/build-time assertions, and cleanup contract;
- async reconcile loop including agent spawn/teardown lifecycle;
- exact v3 source reuse and main-branch copy targets;
- complete work items;
- `src/`/`tests/`/`integration/`/`README.md` test matrix;
- removal preconditions.

---

## Core design principle: all persistent and observable surfaces are zero-secret

Normative D089 spec layering: Credential base fields are ResourceType base
`spec.*` fields, including `spec.providerRef`, `audience`, `scope`,
`allowedOperations`, `rotation`, and `revocation`. This Provider's desired-only
extension is the canonical `spec.provider = { schemaId:
"credential-managed-identity.d2bus.org/Credential/spec", schemaVersion, settings }`
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

Every surface that may be observed by more than the authorized consumer
Provider process - resource spec, resource status, the redb store, revision
log, d2b-bus routing DTOs, audit records, OTEL spans, metrics, and all log
lines - **never contains secret material** in any field or byte.

Specifically:

- `spec` carries only opaque non-secret identifiers and policy fields
  (`clientId`, `imdsEndpointAlias`, `maxLeases`). No token, key, bearer
  string, connection string, or endpoint URL.
- `status` carries only opaque lease handles, timestamps, outcome codes, and
  phase/condition values. No token bytes, no IMDS response content.
- The resource store row, redb WAL, and revision log never contain secret bytes.
- Resource API routing DTOs and d2b-bus envelopes never carry secret bytes.
  The sole exception is the dedicated credential-delivery ComponentSession
  described in §Credential-delivery endpoint contract: raw token bytes travel
  exclusively inside end-to-end `Noise_KK`-encrypted records between the
  **`managed-identity-agent` process** and the authorized consumer Provider
  process. The `managed-identity-controller` process never touches token bytes,
  holds no `ManagedIdentityCredentialClient`, and never opens or terminates a
  credential-delivery channel. Bus intermediaries authorize and forward opaque
  protected records without decrypting them.
- Error and outcome messages are bounded (max 240 UTF-8 bytes), stripped of
  control characters, and must not echo token bytes, IMDS URLs, UUIDs,
  subscription IDs, or Azure connection-string shapes.
- Audit records carry only stable codes and opaque digests.
- OTEL spans and metrics never label secret bytes, IMDS URL fragments,
  tenant/subscription IDs, or Azure resource URI paths.
- The `contains_sensitive_shape` guard (adapted from
  `d2b-realm-provider/src/error.rs:contains_sensitive_shape`, evidence class
  `implemented-and-reachable` at baseline) is applied at construction time to
  every string field in config, audit, and telemetry output.

---

## Provider resource shape

### Nix declaration

```nix
# d2b.zones.<zone>.resources.<name> = { type = "Provider"; spec = { ... }; }
d2b.zones.dev.resources.credential-managed-identity = {
  type = "Provider";
  spec = {
    artifactId = "credential-managed-identity-bin"; # d2b.artifacts entry; type = "provider"
    config = {
      clientId                = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
      imdsEndpointAlias       = "azure-imds";
      maxLeases               = 64;
      controllerExecutionRef  = "Host/azure-vm-host";
    };
  };
};
```

`spec.artifactId` is a sibling of `spec.config` on the Provider resource, not
nested inside `spec.config`. `spec.config` is the bounded non-secret runtime
configuration block; `spec.artifactId` is the artifact catalog ID whose store
path and digest are private catalog implementation data never present in the
resource spec, status, audit, or logs.

### Canonical ResourceSpec JSON

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "Provider",
  "metadata": {
    "name": "credential-managed-identity",
    "zone": "dev"
  },
  "spec": {
    "artifactId": "credential-managed-identity-bin",
    "config": {
      "clientId": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
      "imdsEndpointAlias": "azure-imds",
      "maxLeases": 64,
      "controllerExecutionRef": "Host/azure-vm-host"
    }
  }
}
```

`metadata.uid`, `metadata.generation`, `metadata.revision`, `metadata.createdAt`,
`metadata.updatedAt`, `metadata.managedBy`, and `metadata.configurationGeneration`
are filled by core at create/update time and are never authored in Nix. `status`
is controller-managed and is not present in the Nix-rendered bundle output.

---

## Root `spec.config` schema

### Field reference

| Field | Type | Required | Bounds | Rules |
| --- | --- | --- | --- | --- |
| `clientId` | string | Yes | 1-128 chars; charset `^[A-Za-z0-9._-]+$` | Opaque Azure Managed Identity client GUID. Validated via `OpaqueAzureRef::parse` (current v3 baseline `d2b-realm-provider/src/credential.rs:OpaqueAzureRef`). The field name is `clientId`, not `clientIdRef`; it is an inline opaque identifier, not a `<ResourceType>/<name>` ResourceRef. A secret-shaped value (containing `=`, `+`, `/`, whitespace, or connection-string patterns) fails validation fail-closed. |
| `imdsEndpointAlias` | enum | Yes | closed set | Provider-validated closed alias string resolving to a known IMDS endpoint category. Never an endpoint URL, path, IP address, or hostname. Accepted values and their meanings are defined in §Closed alias set for `imdsEndpointAlias`. |
| `maxLeases` | u32 | No | 1-256; default 64 | Maximum concurrent active leases this Provider instance may hold. Requests beyond the ceiling return `credential-queue-pressure`. |
| `controllerExecutionRef` | ResourceRef | Yes | `Host/<name>` or `Guest/<name>` in same Zone | Execution target for the `managed-identity-controller` Process. Must resolve to a declared system-domain Host or Guest in the same Zone. The controller is secret-free regardless of placement: it holds no `ManagedIdentityCredentialClient`, makes no IMDS calls, and carries no token bytes. |

### `clientId` validation

`clientId` is validated using `OpaqueAzureRef::parse` from
`d2b-realm-provider/src/credential.rs` (evidence class:
`implemented-and-reachable`). The charset `^[A-Za-z0-9._-]+$` structurally
rejects secret-shaped values at parse time - the characters `=`, `/`, `+`, and
whitespace that bearer tokens and connection strings carry are all outside the
safe identifier set (fail-closed). A GUID-shaped value
(`aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee`) passes. A SAS token, connection
string, or bearer string fails. Maximum length is 128 chars
(`MAX_AZURE_REF_LEN`). The `clientId` field name matches the target config
schema; the current v3 field `ManagedIdentityRef.client_id` (snake_case) is the
source symbol and maps to camelCase `clientId` in the v3 Provider config.

### Closed alias set for `imdsEndpointAlias`

The Provider resolves an alias to its implementation-internal endpoint
configuration. The alias string is a short alphanumeric token; it is not a
URL, path, IP address, or fully-qualified hostname. The closed set is:

| Alias | Resolved category | Notes |
| --- | --- | --- |
| `azure-imds` | Standard Azure Instance Metadata Service (IMDS) | For Azure VM Hosts or Guests where the standard IMDS endpoint is reachable |
| `azure-imds-aca` | Azure Container Apps sidecar IMDS | For ACA-backed Guest contexts where the ACA sidecar IMDS endpoint is provided to the container |

Aliases outside this set fail the Provider install validation with a
schema-invalid error. The resolved endpoint is an internal implementation detail
of the Provider's `ManagedIdentityCredentialClient` implementation; it never
appears in resource spec, status, audit, metrics, log lines, or OTEL spans.
Adding a new alias requires a Provider version bump and schema change, not a
runtime string.

### No endpoint URL or path in config

`spec.config` must not and cannot contain a raw IMDS endpoint URL, path segment,
IP address, or hostname. The closed `imdsEndpointAlias` field is the only
mechanism for selecting the IMDS category. This prevents:

- secret-shaped strings masquerading as endpoint values;
- operator-supplied SSRF-risk custom endpoints;
- drift between what audit/telemetry can safely echo and what the field contains.

The eval-time `contains_sensitive_shape` assertion and the `OpaqueAzureRef`
charset check on `clientId` together enforce this at NixOS build time. The
Provider schema rejects unknown config fields at runtime.

---

## ProviderStateSet and Provider status

### ProviderStateSet

**ProviderStateSet** is the optional query-time grouping of a Provider's
declared state Volumes. Under D087, bounded non-secret controller and credential
operational state belongs in the owning resource's `status` subresource and the
core Operation ledger by default. `credential-managed-identity` declares **no
Provider state Volume**: managed-identity token bytes are secret and never
eligible for persistence, while the bounded non-secret lease observation is
revision-suitable status state and does not pass the storage-need test.

Therefore the ProviderStateSet for `Provider/credential-managed-identity` is
empty. Core ProviderDeployment creates no controller or agent state storage for
this Provider, component Processes receive no state mount, no layout principal
or `Provider/volume-local` state reconciliation is involved, and there is no
bootstrap state Volume mechanism or exception (D086, superseded by D087).

### Status-first operational state

Per D088, ResourceType-common Credential observation lives in `status.resource`:
the non-secret lease metadata base that is identical across Credential
implementations. Managed-identity-specific lease observations live only in
`status.provider` with `providerRef`, qualified `schemaId`
`credential-managed-identity.d2bus.org/Credential/status`, `schemaVersion`,
`observedProviderGeneration`, and strict bounded redacted `details`
(≤32 KiB, unknown-field-denied). The controller writes all present layers
atomically in one status mutation; shared
fields are never duplicated into `status.provider`, and the extension schema is
registered and signed in the Provider manifest.

The owning `Credential/<name>.status` carries only bounded, redacted,
RBAC-readable observation. `status.resource` carries lease phase, credential
readiness, expiry timestamps, audience, and opaque lease IDs that are
non-secret, non-authorizing, bounded, safe for authorized status readers, and
independently revalidated before use. `status.provider.details` carries
managed-identity-specific rotation generation, placement binding, bounded retry
counters, closed-enum error detail, and source observations. The core Operation
ledger carries in-flight idempotency/retry; status carries the latest bounded
lease result/checkpoint.

Status is revisioned, optimistic-status-writer controlled, observation-only,
reverified against external IMDS/effect-port reality after restart, written only
on material change, and bounded to the D087 status limits (total ≤ 64 KiB,
provider-specific detail ≤ 32 KiB, with `status-oversize` rejection). It must
not contain managed-identity token bytes, keys, PSKs, authority-conferring
credential handles, private path/argv/env/PID/unit data, raw IMDS/cloud error
bodies, large blobs, endpoint private detail, or unbounded/churn-heavy content.
Token bytes are held only transiently in the agent process and delivered solely
over dedicated Noise_KK sensitive sessions.

### Provider status aggregation

Core aggregates the `Provider/credential-managed-identity` status from the
controller Process health, agent Process health, and Zone-level resource
conditions. The credential controller writes status only to:

- `Credential/<name>.status` - lease state, rotation generation, placement
  binding, and condition flags received from the agent via status-update RPC;
- `Process/mi-agent-<name>.status` - managed by the Process Provider
  (`Provider/system-minijail`); the credential controller reads but does not
  write this status.

The credential controller never writes directly to `Provider/credential-managed-identity.status`.
Core owns Provider status aggregation; the controller contributes only by
keeping its own Process healthy and by updating the Credential resources it
manages.

---

## SDK-consumer co-location model

### `ManagedIdentityCredentialOwner::ExactSdkConsumer`

This Provider is constructed for one exact co-located SDK consumer identified
by `spec.consumerRef`. The `ManagedIdentityCredentialOwner` variant is
`ExactSdkConsumer`: only the declared `consumerRef` Provider's authorized
component may invoke `d2b.credential.v3` methods on a Credential resource
backed by this Provider. There is no ambient fallback, no global consumer, and
no unauthenticated consumer path.

### Azure VM context (Host)

When placed as `host-system` (system-domain Process under a `Host` resource),
the Provider process runs as a system-domain service on the Azure VM host.
The injected `ManagedIdentityCredentialClient` communicates with the
`azure-imds` IMDS endpoint available on the host. The co-located SDK consumer
is the system-domain Process on the same Host whose `consumerRef` matches this
Provider.

Example Credential resource for an Azure VM host:

```yaml
spec:
  providerRef: Provider/credential-managed-identity
  scope:
    executionRef: Host/azure-vm-host
    domainFilter: system
  audience: https://management.azure.com/
  consumerRef: Provider/runtime-azure-virtual-machine
  allowedOperations:
    - acquire-token
    - refresh-token
    - inspect-metadata
  rotation:
    policy: proactive
    proactiveWindowMs: 300000
    maxLeaseLifetimeMs: 3600000
  revocation:
    onOwnerDelete: immediate
    onProviderGeneration: immediate
```

### Azure Container Apps context (Guest)

When placed as `guest-agent` (system-domain Process under a `Guest` resource
backed by `Provider/runtime-azure-container-apps`), the Provider process runs
inside the container alongside the co-located SDK consumer. The injected client
uses the `azure-imds-aca` alias to reach the ACA sidecar IMDS endpoint provided
to the container. This is the formalized replacement for the current v3
`d2b-provider-aca/src/lib.rs:managed_identity_client_id` raw `Option<String>`
config field.

Example Credential resource for an ACA Guest:

```yaml
spec:
  providerRef: Provider/credential-managed-identity
  scope:
    executionRef: Guest/aca-sandbox
    domainFilter: system
  audience: https://relay.servicebus.windows.net/
  consumerRef: Provider/runtime-azure-container-apps
  allowedOperations:
    - acquire-token
    - refresh-token
    - revoke-token
    - inspect-metadata
  rotation:
    policy: proactive
    proactiveWindowMs: 300000
    maxLeaseLifetimeMs: 3600000
  revocation:
    onOwnerDelete: immediate
    onProviderGeneration: immediate
```

### No ambient IMDS chain

There is no environment credential chain, no `AZURE_CLIENT_ID` environment
variable fallback, no `DefaultAzureCredential` chain, no developer-tool
credential, and no keyring path. The `ManagedIdentityCredentialClient` is
injected during Provider construction with an explicit `imdsEndpointAlias`
binding. The production `ManagedIdentityCredentialProvider::new` constructor
accepts only an injected `impl ManagedIdentityCredentialClient`; it has no
ambient-discovery code path. A test may inject a `FakeClient`; production
injects the real client bound to the alias-resolved endpoint configuration.

The current v3 ACA path (`d2b-provider-aca/src/lib.rs`) uses an
`Option<String>` raw `managed_identity_client_id` field that is passed through
to the Azure SDK's credential chain. The v3 Provider formalizes this as an
`OpaqueAzureRef`-typed `clientId` config field and replaces the ambient-chain
path with an injected client bound to the closed alias.

---

## Supported placement bindings

| Placement binding | Accepted | Notes |
| --- | --- | --- |
| `host-system` | Yes | System-domain Process under a `Host` resource with IMDS-compatible endpoint |
| `guest-agent` | Yes | System-domain Process under a `Guest` resource with IMDS-compatible sidecar endpoint |
| `user-agent` | **Rejected** | Managed identity is a machine-level credential, not a user-session credential. Provider install validation and eval-time assertion both reject `domainFilter=user`. |

The placement binding is derived by the Credential controller from the Credential
resource's `scope` fields and stored in
`status.provider.details.placementBinding`.
The Provider schema declares `supportedPlacementBindings: [host-system,
guest-agent]` and `rejectedPlacementBindings: [user-agent]`. Any Credential
resource whose `scope.domainFilter = "user"` fails the NixOS eval-time assertion
before the bundle is emitted.

---

## Supported operation classes

| Operation class | Supported | Notes |
| --- | --- | --- |
| `acquire-token` | Yes | Issue a new token from IMDS for the configured `audience` |
| `refresh-token` | Yes | Refresh an existing active lease |
| `revoke-token` | Yes | Mark the lease revoked; no IMDS call required (managed identity tokens are not revocable at the IMDS level; local lease state is marked Revoked) |
| `inspect-metadata` | Yes | Return opaque lease metadata without token delivery |
| `sign-challenge` | **Not supported** | IMDS does not expose a signing primitive; `sign-challenge` is not in the Provider's declared `supportedOperations`; the schema immediately returns `credential-schema-invalid` |

`allowedOperations` in a Credential resource must be a non-empty subset of
`[acquire-token, refresh-token, revoke-token, inspect-metadata]`. Any element
outside this set, including `sign-challenge`, fails the NixOS eval-time assertion
and the runtime schema check.

---

## Credential-bound service methods

The `d2b.credential.v3` protobuf/ttrpc service methods are dispatched across
two processes depending on whether IMDS access is required:

- **`managed-identity-controller`** optionally serves `InspectMetadata` from
  stored resource-store state only. It holds no
  `ManagedIdentityCredentialClient`, makes no IMDS calls, and never terminates
  a credential-delivery session.
- **`managed-identity-agent`** (one per Credential/consumerRef binding) serves
  `AcquireToken`, `RefreshToken`, `RevokeToken`, and the live `InspectMetadata`
  path. The agent holds the injected `ManagedIdentityCredentialClient`,
  accesses the IMDS alias, and terminates the E2E `Noise_KK` delivery session.
  The agent is spawned and owned by the controller; its process is co-located
  at `Credential.spec.scope.executionRef`.

d2b-bus derives the exact operation class from the method and routes it to the
correct endpoint: stored `InspectMetadata` goes to the controller;
`AcquireToken`, `RefreshToken`, `RevokeToken`, and live `InspectMetadata` go to
the agent. No request carries a caller-selected operation-class field.

### Method table

| Method | Served by | Exact operation/Role subresource | Required Role permission | Outer DTO fields | Sensitive output |
| --- | --- | --- | --- | --- | --- |
| `AcquireToken` | **agent** | `acquire-token` | `use-credential/acquire-token` | `AcquireTokenResponse`: `leaseHandle`, `sourceVersion`, `rotationGeneration`, `expiresAtUnixMs` | Raw token bytes in dedicated `Noise_KK` delivery session (see §Credential-delivery endpoint contract) |
| `RefreshToken` | **agent** | `refresh-token` | `use-credential/refresh-token` | `RefreshTokenResponse`: `leaseHandle`, `sourceVersion`, `rotationGeneration`, new `expiresAtUnixMs` | Raw token bytes in dedicated `Noise_KK` delivery session |
| `RevokeToken` | **agent** | `revoke-token` | `use-credential/revoke-token` | `RevokeTokenResponse`: closed revocation result (`Revoked` or `AlreadyRevoked`), `revokedAtUnixMs` | None |
| `SignChallenge` | admission only | `sign-challenge` | `use-credential/sign-challenge` | `credential-schema-invalid` because this Provider does not declare the operation | None |
| `InspectMetadata` | **agent** (live) / **controller** (stored) | `inspect-metadata` | `use-credential/inspect-metadata` | `InspectMetadataResponse`: `leaseState`, `rotationGeneration`, `sourceVersion`, `expiresAtUnixMs` | None |

`SignChallenge` is not implemented. A `SignChallenge` call returns
`credential-schema-invalid` immediately, before any IMDS interaction, and
closes the request.

Every method:

- rejects an unauthenticated caller or a caller whose **authenticated bus
  subject** (established by the ComponentSession from SO_PEERCRED or the
  enrolled Noise_KK static key - see §ExactSdkConsumer authentication below)
  does not match `spec.consumerRef` when set, with `credential-consumer-mismatch`;
- rejects before Provider dispatch unless the method's one exact operation is
  present in both `spec.allowedOperations` and the Role `subresources` under
  `use-credential`, with `credential-operation-denied`;
- returns a stable closed error code rather than IMDS response content or
  provider-internal diagnostics;
- carries operation/idempotency/correlation IDs from the d2b-bus context;
- enforces a per-call deadline propagated from the d2b-bus context.

### `ManagedIdentityCredentialClient` trait

The injected trait is held **exclusively by the agent process** and is the sole
interface to the IMDS endpoint. It accepts and returns typed lease
request/grant/renewal/revocation/inspection values only. No trait method
parameter accepts a raw token string, endpoint URL, bearer header, environment
variable name, or byte buffer. No trait method return value carries a raw token
string to the outer calling context; token bytes flow only through the delivery
session after `AcquireToken` and `RefreshToken`.

```text
ManagedIdentityCredentialClient:
  issue_lease(&ManagedIdentityLeaseRequest) -> Result<ManagedIdentityLeaseGrant, ManagedIdentityClientError>
  refresh_lease(&ManagedIdentityLeaseRef) -> Result<ManagedIdentityLeaseRenewal, ManagedIdentityClientError>
  revoke_lease(&ManagedIdentityLeaseRef) -> Result<ManagedIdentityLeaseRevocation, ManagedIdentityClientError>
  inspect_lease(&ManagedIdentityLeaseRef) -> Result<ManagedIdentityLeaseInspection, ManagedIdentityClientError>
```

`ManagedIdentityClientState`: `Ready | Unavailable`.

`ManagedIdentityClientError::Unavailable` maps to
`credential-provider-unavailable`. There is no `InteractionRequired` state
for managed identity; IMDS is either reachable or not.

### `ManagedIdentityLeaseRequest`

Fields (all non-secret):

| Field | Type | Notes |
| --- | --- | --- |
| `audience_opaque_digest` | [u8; 32] | SHA-256 digest of the opaque `spec.audience` string; the raw audience string is not passed to the trait method or recorded in any audit/log surface |
| `requested_expiry_unix_ms` | u64 | Desired expiry; capped by `maxLeaseLifetimeMs` and provider maximum |
| `idempotency_key` | [u8; 32] | Derived from Credential UID + `rotationGeneration` + operation class; no secret material |

### `ManagedIdentityLeaseGrant`

Fields (all non-secret except the token bytes held internally):

| Field | Type | Notes |
| --- | --- | --- |
| `lease_handle` | ManagedIdentityLeaseHandle | Opaque newtype wrapping a bounded string (max 256 chars); not a token; stable across refreshes within one rotation generation |
| `source_version` | String | Opaque, bounded non-secret version string from the IMDS response (e.g. a generation/revision field); max 64 chars |
| `expires_at_unix_ms` | u64 | Token expiry from IMDS response |
| `issued_at_unix_ms` | u64 | Token issue time from IMDS response |

The raw token bytes are held by the `ManagedIdentityCredentialClient`
implementation internally; they are never present in the `ManagedIdentityLeaseGrant`
struct fields returned to the calling Provider service handler. Token bytes are
delivered exclusively through the `Noise_KK` end-to-end delivery session
established during `AcquireToken` and `RefreshToken` handling.

### `ManagedIdentityLeaseHandle`

A bounded newtype wrapping a short opaque string identifier. It must not
resemble, contain, or be derivable from a token, bearer string, IMDS response
fragment, GUID, or connection string. It is safe to include in audit records
and metric labels as an opaque handle. Maximum length: 256 chars.
`ManagedIdentityLeaseHandle` implements a hand-written `Debug` that emits only
`ManagedIdentityLeaseHandle(REDACTED)` - no auto-derived `Debug`.

### ExactSdkConsumer authentication

`ExactSdkConsumer` validation is performed by the **agent process** on the
`AuthenticatedSubjectContext` established by the incoming ComponentSession.
This validation is **independent of scope-field declarations** and works as
follows:

1. The agent's ComponentSession is established over a d2b-bus enrolled channel
   using either SO_PEERCRED (for system-domain callers on the same host) or the
   consumer Provider's enrolled Noise_KK static key (for enrolled component
   processes).
2. d2b-bus maps the authenticated credential to a Zone subject identifier via
   the Zone identity registry, yielding a `Provider/<consumer-name>` identity
   or a `Principal/<id>` for direct SO_PEERCRED callers.
3. d2b-bus provides the resulting `AuthenticatedSubjectContext` to the agent
   alongside the forwarded request envelope; the agent does not re-derive the
   identity from the request payload.
4. The agent checks: `AuthenticatedSubjectContext.provider_ref == spec.consumerRef`
   when `spec.consumerRef` is set. If the authenticated subject does not match,
   the method returns `credential-consumer-mismatch`. If `spec.consumerRef` is
   null (any authorized Provider), authentication still proceeds through the
   enrolled key - there is no unauthenticated path.
5. Additionally, the consumer's signed component descriptor (available through
   the enrolled key) must declare `credentialProviderRef: Provider/credential-managed-identity`
   for the RBAC `use-credential` check to pass.

This model means that even if a caller forges a `scope.consumerRef`-shaped
field in the request, the check fails: the validation is on the runtime
authenticated bus identity, not on any caller-supplied field.

`user-agent` placement is hard-rejected at Placement Validation time (see
§Placement validation); the agent process never receives a request from a
`user-agent` domain caller.

---

## Credential-delivery endpoint contract

Token bytes for `AcquireToken` and `RefreshToken` are delivered exclusively
through a dedicated end-to-end `Noise_KK_25519_ChaChaPoly_SHA256`
ComponentSession. The `Noise_NN` profile is forbidden for sensitive output
delivery; any anonymous-channel attempt is rejected immediately and the session
is zeroized.

### Session profile

Both parties must present enrolled static keys:

- **Credential Provider key**: registered at Provider installation; the bus
  holds only the public key.
- **Consumer Provider key**: extracted from the consumer Provider's signed
  component descriptor. The consuming component/Process identity is derived
  from the consumer Provider's Role/RoleBinding; no arbitrary component is
  permitted.

### Binding contract

Each delivery session binds (in the Noise prologue verified by both parties):

| Field | Description |
| --- | --- |
| `credentialRef` | `Credential/<name>` |
| `credentialUID` | Credential resource UID (stable across spec updates) |
| `credentialGeneration` | Credential resource generation at delivery time |
| `consumerProviderRef` | `Provider/<name>` matching `spec.consumerRef` |
| `consumerComponentGeneration` | Consumer Provider component generation from signed descriptor |
| `audience_digest` | SHA-256 of `spec.audience` - the raw audience string never enters the prologue or any log/audit surface |
| `operationClass` | Closed enum: `acquire-token` or `refresh-token` |
| `expiryUnixMs` | Absolute delivery-session expiry; clipped to `spec.rotation.maxLeaseLifetimeMs` |
| `deadlineUnixMs` | Hard session close deadline; must be ≤ `expiryUnixMs` |
| `routeDigest` | Digest of bus-authorized route parameters (Zone, consumer, provider) |
| `schemaVersion` | Fixed version of this binding contract |
| `maxTokenBytes` | Closed upper bound on sensitive record size for this session |
| `transcriptDigest` | Noise transcript digest after handshake, before first record |

### Security requirements

1. **Enrolled keys only**: both static keys enrolled and verified. Any NN/NX/N
   attempt is rejected immediately; session closed and zeroized.
2. **Replay-safe sequence**: each delivery session carries a monotonically
   increasing per-Credential-UID sequence number. A replay of a prior session's
   ciphertext at the same or lower sequence number is rejected.
3. **Bounded output size**: the sensitive record must not exceed `maxTokenBytes`.
   Any oversize record is rejected; channel closed and zeroized immediately. No
   fragmentation unless each fragment is independently bounded and the
   reassembled total is within `maxTokenBytes`.
4. **Zeroizing buffers**: the delivery record's plaintext is zeroed immediately
   after the consumer extracts it. The Provider zeros the plaintext source after
   encryption. All intermediate buffers are zeroizing types.
5. **Redacted Debug**: all credential-bearing Rust types involved in delivery
   implement `Debug` via a hand-written redacted impl. Derived `Debug` is
   forbidden for these types.
6. **No automatic success-shaped replay**: after any ambiguous delivery outcome,
   the Provider must not automatically retry with the same record. The consumer
   must re-initiate, establishing a new delivery session with a fresh sequence
   number.
7. **Immediate close/zeroize**: after confirmation, the Provider closes the
   delivery channel and zeroizes all session key material. The consumer
   similarly closes and zeroizes after extraction. The channel is not reused
   across multiple deliveries.

### RBAC enforcement at bus

d2b-bus performs the following checks before authorizing the delivery route:

- RBAC `use-credential` for the authenticated consumer Provider subject,
  Credential ResourceRef, and operation class;
- `spec.consumerRef` matches the consumer Provider identity;
- `spec.allowedOperations` includes the operation class;
- current lease state permits the operation class;
- no Role/RoleBinding or Provider generation change has revoked authorization
  since the last audit checkpoint.

After these checks pass, bus forwards opaque `Noise_KK`-encrypted records
between the two endpoints until the delivery session closes. Bus never buffers
or stores the records.

---

## Process components

| Component | Type | Domain | Binary | Cardinality |
| --- | --- | --- | --- | --- |
| `managed-identity-controller` | controller | system | `d2b-managed-identity-controller` | One per Zone; manages all `credential-managed-identity`-backed Credential resources; **no IMDS access; no token bytes** |
| `managed-identity-agent` | service | system | `d2b-managed-identity-agent` | One per Credential/consumerRef binding; co-located at `Credential.spec.scope.executionRef`; holds injected client; terminates E2E KK delivery |

The system-domain constraint is unconditional for both processes.
`user-agent` domain is rejected at Provider install validation and again at
Credential controller `reconcile` time.

### Controller process

The `managed-identity-controller` process:

- is owned by `Provider/credential-managed-identity` (`ownerRef`);
- holds **no** `ManagedIdentityCredentialClient`;
- makes **no** IMDS calls;
- never opens, terminates, or forwards a credential-delivery `Noise_KK` session;
- watches and reconciles all `Credential` resources backed by this Provider;
- spawns one `managed-identity-agent` Process resource per Credential binding
  when the Credential spec is admitted and its dependencies are ready (not
  after Credential `phase=Ready`, which depends on agent readiness); tears it
  down on Credential deletion;
- may serve `InspectMetadata` from resource-store state over
  `d2b.credential.v3`;
- monitors agent Process health via `ownerChildTrigger` watch; sets
  `ProviderUnavailable=True` on sustained agent Process failure.

The controller descriptor declares:

```yaml
providerId: Provider/credential-managed-identity
controllerType: Credential
resourceTypes: [Credential]
watchSelectors:
  - resourceType: Credential
    providerRefFilter: Provider/credential-managed-identity
  - resourceType: Provider
    nameFilter: credential-managed-identity
  - resourceType: Process
    ownerRefFilter: Provider/credential-managed-identity
dependencySelectors:
  - resourceType: Provider
    relationship: providerRef
  - resourceType: Host
    relationship: scope.executionRef
  - resourceType: Guest
    relationship: scope.executionRef
  - resourceType: Process
    relationship: ownerRef   # agent processes owned by Credential resources
ownerChildTriggers: [owned-resource-changed, agent-process-health-changed]
reconcileConcurrency: 8
maxPendingResources: 256
finalizers: [credential.d2bus.org/provider-revoke]
observeInterval: 30s
```

#### Currency and upgrade (D091)

The `managed-identity-controller` implements `assess_update`, `plan_upgrade`,
and `execute_upgrade`. A Provider generation or signed artifact generation/digest
change updates universal `status.update` with `state: UpdateAvailable` or
`state: UpgradeRequired`, `reasons` including `ProviderGenerationChanged` or
`ArtifactChanged`, observed/target generation or digest IDs,
`disruption: Reload` or `disruption: Restart` for the credential component
realization, `preserveState: true`, bounded `owned`/`dependencies`, and
`lastAssessedAt`. Disruptive changes MUST return `UpgradeRequired` rather than
applying in place; non-disruptive changes reconcile normally. Credential
rotation is not an upgrade and remains the lease lifecycle flow. `status.update`
MUST NOT contain secret bytes, tokens, or lease material; only bounded
non-secret generation/digest IDs and lease metadata already permitted by the
Credential base may appear. Token delivery remains solely over `Noise_KK`.

#### Expedited reconcile on mutation (D090)

For `Create`, `UpdateSpec`, and `Delete` with `waitForReconcile`, the
`managed-identity-controller` MUST perform no IMDS/effect-port call, Process
create/delete, finalizer change, or status mutation until core supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. Abort
before that proof has no effect. After durable commit, the commit is never
rolled back if the reconcile pass times out. The response returns the committed
object, post-pass projected layered status, disposition
(`Converged|Progressing|Blocked|UpgradeRequired|Failed`), and
`statusPersistence: pending|committed`. Effect idempotency keys derive from
`(UID,generation,revision,operationId)` and use the same per-resource
single-flight priority lane.

### Agent process

The `managed-identity-agent` process:

- is owned by the Credential resource it serves (`ownerRef: Credential/<name>`);
- is spawned and destroyed by the controller (never configuration-managed);
- is co-located at `Credential.spec.scope.executionRef` (Host system domain
  or Guest agent domain);
- receives exactly one `ManagedIdentityCredentialClient` implementation via an
  **effect port** supplied by the co-located runtime Provider at agent start-up;
  the client is bound to the IMDS alias resolved from the controller's
  LaunchTicket projection (see §Process resource templates); the agent never
  opens any direct network connection to an IMDS endpoint;
- serves `AcquireToken`, `RefreshToken`, `RevokeToken`, and the live
  `InspectMetadata` path over `d2b.credential.v3` to the exact authorized
  consumer process;
- terminates the E2E `Noise_KK_25519_ChaChaPoly_SHA256` delivery session for
  token bytes;
- reports lease state changes to the controller via controller-internal
  status-update RPC (not via `d2b.credential.v3`);
- holds no other Credentials, no Zone admin token, and no secrets beyond the
  IMDS-issued lease bytes held internally by the client implementation.

---

## Process resource templates

These templates show the canonical shape of the Process resources for both
roles. All fields shown are required unless marked optional; fields excluded
by the instruction set (principalRef, profileRef, endpoint `kind`, Process
`config`, `telemetry.componentRef`, `readiness.probe`/`timeoutMs`,
`network.allowedEffects`) are absent. `budget` uses canonical nested
cpu/memory/pids/fds fields - there is no top-level `class` key on
BudgetSpec. `telemetry` uses canonical metricsEnabled/tracingEnabled/logLevel/
sensitiveLabels fields - there is no `class` key on TelemetrySpec.
Controller-created agent resources must not appear in Zone bundle Nix
configuration - they are managed exclusively by the controller.

### Controller Process template

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: managed-identity-controller
  zone: <zone-id>
  ownerRef: Provider/credential-managed-identity
spec:
  processClass: controller
  template: managed-identity-controller
  providerRef: Provider/system-minijail
  executionRef: <Provider.spec.config.controllerExecutionRef>   # Host/<h> or Guest/<g>
  domain: system
  sandbox:
    namespaceClasses: [mount, pid, ipc]   # inherits execution target network namespace
    capabilityClasses: []
    seccompClass: strict
    startRoot: false
    noNewPrivileges: true
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    cpu:
      request: "100m"
      limit: "500m"
    memory:
      request: "32Mi"
      limit: "128Mi"
    pids:
      limit: 64
    fds:
      limit: 256
  readiness:
    class: provider-defined
    initialDelay: "1s"
    timeout: "10s"
    failureThreshold: 3
    successThreshold: 1
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: false
```

The controller sandbox declares `[mount, pid, ipc]` and inherits the execution
target's network namespace; all communication goes through the authorized
d2b-bus ComponentSession supplied at launch. No `networkUsage` section is needed.
No Process `config` section is needed;
the controller derives its configuration from the Provider resource spec at
registration time. `executionRef` is bound from `Provider.spec.config.controllerExecutionRef`
at spawn time; the controller remains secret-free regardless of placement.

### Agent Process template

The controller creates one agent Process resource for each Credential whose spec
is **admitted and whose dependencies are ready** (executionRef resolves to a live
Host or Guest, Provider/credential-managed-identity is Ready). The agent Process
is owned by the Credential resource (`ownerRef: Credential/<credential-name>`):
when the Credential is deleted the controller removes the agent Process before
releasing the `provider-revoke` finalizer.

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: mi-agent-<credential-name>         # controller-derived, non-configurable
  zone: <zone-id>
  ownerRef: Credential/<credential-name>   # deleted when Credential is deleted
spec:
  processClass: service
  template: managed-identity-agent
  providerRef: Provider/system-minijail
  executionRef: <Credential.spec.scope.executionRef>   # Host/<h> or Guest/<g>
  domain: system
  sandbox:
    namespaceClasses: [mount, pid, ipc]   # inherits execution target network namespace
    capabilityClasses: []
    seccompClass: strict
    startRoot: false
    noNewPrivileges: true
    environmentClass: minimal
    readOnlyRoot: true
  networkUsage:
    networkRef: null
    ports: []
    allowEgress: false
  budget:
    cpu:
      request: "50m"
      limit: "250m"
    memory:
      request: "16Mi"
      limit: "64Mi"
    pids:
      limit: 32
    fds:
      limit: 128
  readiness:
    class: provider-defined
    initialDelay: "1s"
    timeout: "5s"
    failureThreshold: 3
    successThreshold: 1
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: false
```

The agent Process produces its stable credential service identity as a separate
owned `Endpoint` resource. The sensitive KK token-delivery session is not an
Endpoint.

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: mi-agent-<credential-name>-credential-service
  zone: <zone-id>
  ownerRef: Credential/<credential-name>
spec:
  providerRef: Provider/credential-managed-identity
  producerRef: Process/mi-agent-<credential-name>
  endpointClass: service
  transport: unix
  purpose: credential-managed-identity.d2bus.org/credential-service
  serviceFingerprint: credential.d2bus.org/CredentialService.v3
  locality: host-local
  visibility: zone
  attachmentPolicy: component-session
  consumerPolicy:
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
status:
  resource:
    readiness: Ready
    observedProducerGeneration: 1
    observedResourceGeneration: 1
    endpointGeneration: 1
    connectionAvailability: available
    leaseAvailability: lease-required
  update:
    state: Current
    reasons: []
    observedGeneration: 1
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

## Endpoint resources (D092)

`Provider/credential-managed-identity` conforms to the standard `Endpoint` base
schema. The stable `d2b.credential.v3`-class ComponentSession service identity is
an owned `Endpoint` resource with `producerRef`; consumers use
`Endpoint/<name>`. Endpoint spec/status never carries IMDS URLs, raw endpoint
locators, fd numbers, token bytes, subscription IDs, credential handles with
authority, or secrets. Resolution occurs only through an authorized
EffectPort/LaunchTicket; unauthorized resolution returns `endpoint-resolve-denied`.
Producer restart bumps `Endpoint.status.endpointGeneration`, causing consumers to
observe `dependency-changed` and reacquire through a fresh authorized ticket.

## Retained opaque handles (D092 promotion test)

- pidfds for controller/agent Processes are supervision handles, not resources.
- LaunchTicket fd indexes for the injected IMDS/effect-port client and d2b-bus
  connection remain per-launch opaque attachment slots.
- `leaseHandle`, IMDS client aliases, `operationId`, and idempotency keys are
  bounded non-secret handles revalidated before use.
- `OwnedTransport`, ComponentSession IDs, and the sensitive Noise_KK token-delivery
  session handle are high-churn in-memory capabilities and remain internal.

#### IMDS access and LaunchTicket projection

The agent Process spec declares `networkUsage.allowEgress: false`. The agent never
opens any direct network connection. IMDS access is provided exclusively through
the injected `ManagedIdentityCredentialClient` implementation, supplied via an
**effect port** by the co-located runtime Provider at agent start-up.

The `imdsEndpointAlias` (closed enum `azure-imds | azure-imds-aca`) from
`Provider.spec.config` is projected into a **LaunchTicket** - a signed,
bounded, non-stored launch-time configuration blob - that the controller attaches
to the agent Process resource at spawn time. The runtime Provider reads the
LaunchTicket, resolves the alias to the correct IMDS client configuration, and
constructs the `ManagedIdentityCredentialClient` implementation bound to that
endpoint. The resolved endpoint URL never appears in:

- the Process resource spec or store entry;
- any resource status, audit record, metric label, OTEL span attribute, or log;
- the agent's environment variables, filesystem, or configuration namespace.

The `credentialRef` and other non-secret configuration needed by the agent at
start-up are also projected via the LaunchTicket, not stored in the Process spec
`config` section.

---

## Placement validation

### Eval-time (Nix assertions)

Applied to every `d2b.zones.<zone>.resources.<name>` entry whose
`type = "Credential"` and `spec.providerRef = "Provider/credential-managed-identity"`:

- `spec.scope.domainFilter` must not be `"user"`. Assertion failure with message:
  `"credential-managed-identity does not support user-domain placement; use credential-secret-service or credential-entra for user-session credentials"`.
- `spec.scope.executionRef` must resolve to a declared `Host/<name>` or
  `Guest/<name>` in the same Zone.
- `spec.allowedOperations` must be a non-empty subset of
  `[acquire-token, refresh-token, revoke-token, inspect-metadata]`. Any element
  outside this set (including `sign-challenge`) fails with: `"credential-managed-identity
  does not support operation class <class>"`.
- `spec.consumerRef`, if set, must resolve a declared `Provider/<name>` in
  the same Zone.
- `spec.audience` passes `OpaqueAzureRef`-equivalent charset validation
  (`^[A-Za-z0-9._:/@-]+$`, max 256 chars). Any field failing this check
  triggers the `contains_sensitive_shape` assertion.

### Runtime (controller and agent validation)

**Controller** (`reconcile`):

- Checks `spec.scope.domainFilter != "user"`. If the constraint is violated
  (should not occur after eval-time gate), the controller sets `phase=Failed`,
  `outcome=credential-placement-mismatch`, and does not spawn an agent.
- Validates that `spec.scope.executionRef` resolves to a live `Host` or `Guest`
  resource; if the target is not ready, the controller sets `phase=Pending`.
- Spawns an agent Process when the Credential spec is **admitted** (domain and
  allowedOperations checks pass) and all dependencies are **ready** (executionRef
  live, Provider Ready). The controller does not wait for the Credential to reach
  `phase=Ready` before spawning - doing so would be circular, because the
  Credential only reaches `phase=Ready` after the agent is ready.

**Agent** (per-method dispatch):

- `ManagedIdentityCredentialOwner::ExactSdkConsumer` is enforced against the
  **`AuthenticatedSubjectContext`** provided by the agent's ComponentSession.
  The check compares the runtime authenticated bus identity (established from
  SO_PEERCRED or the consumer's enrolled Noise_KK static key, independently of
  any caller-supplied field) against `spec.consumerRef`. If the authenticated
  subject does not match, the method returns `credential-consumer-mismatch`
  before any IMDS interaction. If `spec.consumerRef` is null, authentication
  still proceeds through the enrolled channel - there is no unauthenticated
  path.
- `sign-challenge` operation class returns `credential-schema-invalid`
  immediately before any IMDS interaction.
- `user-agent` domain is hard-rejected: the agent process is never started with
  `executionRef` resolving to a user-domain endpoint.

---

## RBAC

The `credential-managed-identity` Provider participates in the standard
Credential RBAC model defined in `ADR-046-resources-credential`.

### Required verbs

| Verb | Who | Meaning |
| --- | --- | --- |
| `get` | Any authorized subject | Read Credential metadata/spec/status; no secret bytes present |
| `list` | Any authorized subject | List managed-identity Credentials |
| `watch` | Any authorized subject; `managed-identity-controller` | Watch Credential events (controller watches all; consumers watch own) |
| `create` plus `admin-credential/create` | `activation-nixos` controller | Create Credential resource from bundle; neither permission implies the other |
| `update-spec` plus `admin-credential/update-spec` | `activation-nixos` controller | Replace Credential spec; neither permission implies the other |
| `update-status` | `managed-identity-controller` only | Update Credential status subresource |
| `update-finalizers` | `managed-identity-controller`, `consumerRef` controller | Add/remove owned finalizers |
| `delete` plus `admin-credential/delete` | `activation-nixos` controller | Request Credential deletion; neither permission implies the other |
| `use-credential` | Consumer subject authorized via `consumerRef`; dispatched by agent | Invoke one admitted `d2b.credential.v3` method under its exact allowed-operation subresource |

Agent Process creation/deletion uses ordinary `Process` `create`/`delete`
authority plus structural ownership checks. Reading the bound Credential uses
ordinary `Credential` `get`; `spawn-agent` and `get-credential` are not verbs.

### `use-credential` Role rule shape

```yaml
rules:
  - resourceTypes: [Credential]
    verbs: [use-credential]
    subresources: [acquire-token, refresh-token]
    resourceNames: [aca-mi-relay]
    zones: [dev]
    executionRefs: [Guest/aca-sandbox]
    sessionVerbs: []
```

The effective operation set is the intersection of the Credential resource's
`spec.allowedOperations` and the Role rule's exact `subresources`, further
narrowed by `consumerRef`, scope, and structural Provider/component checks.
Empty, wildcard, unknown, and mismatched subresources deny; there is no
alternate Credential-operation Role field or shorthand operation alias.

### Consumer descriptor requirement

The consuming component's signed component descriptor must:

- declare `credentialProviderRef: Provider/credential-managed-identity`;
- carry an enrolled static public key for the `Noise_KK` delivery channel;
- list the exact Credential ResourceRef and operation classes it will invoke.

The bus validates the descriptor fingerprint before routing the delivery session.
No arbitrary component within the consumer Provider may receive raw token bytes.

---

## Security invariants

### OpaqueAzureRef at construction time

`clientId` is validated by `OpaqueAzureRef::parse` at Provider config parse time.
A malformed or secret-shaped value causes Provider install validation to fail with
`credential-schema-invalid` before the Provider resource reaches `Ready` status.
The Provider process never starts with an invalid `clientId`.

### No ambient credential discovery

The `ManagedIdentityCredentialProvider::new` constructor signature - used by
the **`managed-identity-agent` process** at start-up - accepts only:

- a validated `ManagedIdentityClientConfig` (containing the resolved alias binding);
- an injected `impl ManagedIdentityCredentialClient + Send + Sync + 'static`.

It has no code path that calls `azure_identity::DefaultAzureCredential::new()`,
reads `AZURE_CLIENT_ID` or any other environment variable, consults a
filesystem path, or calls a credential chain builder. This invariant is
enforced by the absence of these imports in `src/agent.rs` and confirmed by the
`canary.rs` test suite.

The `managed-identity-controller` process does not import or call
`ManagedIdentityCredentialProvider::new`; it has no IMDS client and no
credential-discovery code path whatsoever.

### Injection contract for production

The co-located runtime Provider constructs the `ManagedIdentityCredentialClient`
implementation from the **LaunchTicket projection** attached by the controller at
agent spawn time. The projection contains:

- the `imdsEndpointAlias` (closed enum, no URL string) from `Provider.spec.config`;
- the `clientId` (already validated `OpaqueAzureRef`) from `Provider.spec.config`.

The runtime Provider supplies the resulting client implementation to the agent
via an effect port. The agent's `agent/main.rs` entry point receives the injected
client; it never reads `imdsEndpointAlias` or constructs a client from environment
variables or filesystem config. The resolved endpoint URL is a private
implementation detail of the production client and is never serialized, logged,
or placed in any provider output surface.

### Zero-secret status

The Credential **controller** writes only the following status fields (received
from the agent via status-update RPC):

Common `status.resource` lease metadata:

| Field | Value written | Secret? |
| --- | --- | --- |
| `leaseHandle` | `ManagedIdentityLeaseHandle` opaque newtype value | No |
| `leaseState` | Closed enum: `Active`, `Expired`, `Revoked`, `Unknown` | No |
| `audience` | Opaque bounded audience alias or digest; never raw Azure resource ID | No |
| `expiresAtUnixMs` | Unix milliseconds | No |
| `issuedAtUnixMs` | Unix milliseconds | No |

Managed-identity-specific `status.provider.details`:

| Field | Value written | Secret? |
| --- | --- | --- |
| `rotationGeneration` | Monotonic counter | No |
| `sourceVersion` | Opaque bounded string from grant | No |
| `lastRefreshedAt` | RFC 3339 UTC timestamp | No |
| `lastRotatedAt` | RFC 3339 UTC timestamp or null | No |
| `placementBinding` | `host-system` or `guest-agent` | No |

No token bytes, IMDS response fragments, base64-encoded material, or Azure
resource IDs appear in any status field. The `credential_canary` value used in
the test suite is explicitly confirmed absent from every status field written by
the controller in `canary.rs`.

---

## Redaction

All Rust types in the Provider that may transitively hold token bytes or IMDS
response content must implement `Debug` via a hand-written redacted impl:

```rust
impl fmt::Debug for ManagedIdentityLeaseGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ManagedIdentityLeaseGrant")
            .field("lease_handle", &self.lease_handle)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .field("source_version", &"REDACTED")
            .finish()
    }
}
```

Types that are unconditionally non-secret (e.g., `ManagedIdentityLeaseHandle`,
`ManagedIdentityLeaseState`) may use auto-derived `Debug`. The delivery record
wrapper and any intermediate buffer type used during token extraction must use
a hand-written `Debug` that emits only the type name and `REDACTED`.

`Display` impls must not expose token bytes. `Error::source()` chains must not
expose provider-internal diagnostic strings that might contain IMDS endpoint
details, subscription IDs, or resource path fragments.

---

## Lease model

### `ManagedIdentityLeaseState`

The non-secret lease state below is observed through `Credential.status` and the
core Operation ledger. `status.resource` may include expiry, audience, phase,
and opaque lease IDs that are non-secret and non-authorizing; managed-identity
rotationGeneration and bounded retry/outcome detail live in
`status.provider.details`. Token bytes are never status fields and never persist
to any Provider state Volume.

```
Absent (no active lease)
  |-- AcquireToken ---------> Active

Active
  |-- proactive window ------> RotationDue (CredentialReady=True, RotationDue=True)
  |-- RefreshToken ----------> Active (new expiresAtUnixMs, rotationGeneration unchanged)
  |-- [rotation] ------------> Active (rotationGeneration+1, new leaseHandle)
  |-- Provider gen change ---> [immediate] -> Revoked | [drain-leases] -> Expired on deadline
  |-- RevokeToken -----------> Revoked
  |-- expiry deadline -------> Expired

RotationDue
  |-- rotation attempt ------> Active (new rotationGeneration)
  |-- rotation fails --------> RotationDue/Degraded (bounded retry)
  |-- final retry fails -----> Failed (outcome=rotation-failed)

Expired
  |-- AcquireToken ----------> Active (new lease; rotationGeneration+1)

Revoked
  |-- AcquireToken ----------> Active if policy permits re-acquisition
  |-- resource delete -------> provider-revoke finalizer satisfied immediately
```

### Idempotency key derivation

Every `AcquireToken`/`RefreshToken`/`RevokeToken` call carries a stable
`idempotency_key` of type `[u8; 32]` derived as:

```text
SHA-256(Credential.UID || ':' || rotationGeneration.to_le_bytes() || ':' || operation_class_byte)
```

No secret material is present in any component of this derivation. A duplicate
acquire with the same key returns the existing grant without double-issuing an
IMDS token request.

### Lease cardinality limit

The controller enforces a maximum of `min(maxLeases, 256)` concurrent active
leases per Provider instance. New requests beyond this ceiling return
`credential-queue-pressure`. The count is maintained in the resource store by
the controller and checked before spawning a new agent Process.

---

## Status derivation

| Condition | Set `True` when | Set `False` when |
| --- | --- | --- |
| `CredentialReady` | `leaseState=Active` and not within rotation window | `leaseState` ≠ Active, or `RotationDue=True` and rotation failed |
| `RotationDue` | `policy=proactive` and `now + proactiveWindowMs >= expiresAtUnixMs` | Rotation completed successfully |
| `ProviderUnavailable` | Agent `ManagedIdentityClientState=Unavailable` on three consecutive health-check calls to controller, OR agent Process `phase=Failed` | Agent returns successful health-check response |
| `LeaseRevoked` | `leaseState=Revoked` and no replacement has been issued | New active lease acquired |

Phase transitions:

| Phase | Condition |
| --- | --- |
| `Pending` | Controller registered but agent Process not yet spawned or not yet `Ready` |
| `Ready` | Agent `phase=Ready`, `leaseState=Active`, `CredentialReady=True` |
| `Degraded` | Agent reachable but returning errors (rotation failed, intermittent IMDS unavailability), or agent Process failing with respawn scheduled |
| `Failed` | Final respawn/retry exhausted; `outcome` code set |
| `Deleted` | `provider-revoke` finalizer cleared; Core has atomically written event-only Deleted revision and removed resource row/index; audit subsystem has appended deletion record |

---

## Errors

Stable closed error codes for this Provider:

| Code | Condition |
| --- | --- |
| `credential-not-found` | Credential resource does not exist in this Zone |
| `credential-provider-unavailable` | IMDS endpoint unreachable or returning non-2xx; `ManagedIdentityClientState=Unavailable` |
| `credential-lease-expired` | Lease is past its expiry deadline |
| `credential-lease-revoked` | Lease was explicitly revoked |
| `credential-operation-denied` | Operation class not in `spec.allowedOperations` or RBAC denied |
| `credential-consumer-mismatch` | Authenticated consumer subject does not match `spec.consumerRef`; `ManagedIdentityCredentialOwner::ExactSdkConsumer` guard |
| `credential-placement-mismatch` | `scope.domainFilter=user` rejected at runtime; execution context does not match scope |
| `credential-rotation-failed` | Proactive rotation attempt failed after bounded retries |
| `credential-invariant-failure` | IMDS response failed internal invariant checks (e.g. expiry in the past, malformed lease handle) |
| `credential-schema-invalid` | `sign-challenge` operation class requested; or `spec.config` field fails validation |
| `credential-queue-pressure` | Active lease count at `maxLeases` ceiling; retry after backpressure |

All error messages:
- maximum 240 UTF-8 bytes;
- stripped of control characters;
- must not contain token bytes, IMDS URLs, IMDS response fragments, UUIDs,
  subscription IDs, resource paths, Azure endpoint hostnames, or
  connection-string patterns;
- must not echo request arguments (no audience literal, no requested
  expiry value, no lease handle value).

---

## Audit

### Events and field set

| Event | Fields retained |
| --- | --- |
| Credential resource create/update/delete | Zone, subject digest, `resource_name_digest`, verb, revision result, authorization decision |
| `AcquireToken` | Zone, subject digest, `resource_name_digest`, `operation_class="acquire-token"`, `rotationGeneration`, outcome code, idempotency key digest |
| `RefreshToken` | Zone, subject digest, `resource_name_digest`, `operation_class="refresh-token"`, `rotationGeneration`, outcome code, idempotency key digest |
| `RevokeToken` | Zone, subject digest, `resource_name_digest`, `operation_class="revoke-token"`, `rotationGeneration`, revocation result code |
| `InspectMetadata` | Zone, subject digest, `resource_name_digest`, `operation_class="inspect-metadata"`, outcome code |
| Proactive rotation | Zone, `resource_name_digest`, trigger `proactive-window`, old `rotationGeneration`, new `rotationGeneration`, outcome code |
| Provider generation change revocation | Zone, `resource_name_digest`, policy applied (`immediate` or `drain-leases`), outcome code |
| IMDS unavailability onset | Zone, `resource_name_digest`, consecutive-failure count, outcome code `credential-provider-unavailable` |
| Agent spawn | Zone, `resource_name_digest`, agent Process name, `executionRef`, outcome code |
| Agent Process failure | Zone, `resource_name_digest`, agent Process name, failure reason (closed code), failure count |
| **Deleted-phase closure** | Zone, `resource_name_digest`, `phase=Deleted`, `finalizer=credential.d2bus.org/provider-revoke`, `cleanupLatencyMs`, final `rotationGeneration`, outcome `resource-deleted` |

`resource_name_digest` is SHA-256 of the Credential resource name, never the
raw name. It is admitted only to the authorization-controlled bounded Zone
audit stream and, for caller-initiated operations, after the authorization
decision. Raw Credential name, ResourceRef, and UID are excluded. The digest
is never copied to telemetry, logs, collector diagnostics, or support
summaries.

The **Deleted-phase closure** audit record is written by the **audit subsystem**,
not by the controller. The controller's only deletion-time action is to clear
the `provider-revoke` finalizer. The controller never commits any store revision
for Credential deletion and never gates finalizer release on audit completion.
After the finalizer is cleared:

1. **Core store write**: Core atomically writes an event-only Deleted revision
   and removes the Credential row and index entries from the resource store.
2. **Audit subsystem**: The audit subsystem reads the committed Deleted revision
   and appends the closure record with exactly-once delivery. A stable dedup key
   bound to the committed revision prevents duplicate records on restart or
   audit-subsystem recovery. The controller never re-emits or re-enacts this
   step.

### Excluded from all audit records

Token bytes, raw IMDS response content, IMDS endpoint URL fragments, base64
material, bearer strings, `clientId` value, `imdsEndpointAlias` string, audience
literal value, tenant/subscription IDs, Azure resource URIs, `leaseHandle`
plaintext value (only the opaque handle digest is permitted), provider-internal
diagnostic strings, host filesystem paths, connection string shapes, and Noise/
session key material.

---

## OTEL and metrics

### Span names

```
d2b.credential.acquire_token
d2b.credential.refresh_token
d2b.credential.revoke_token
d2b.credential.inspect_metadata
d2b.credential.reconcile
d2b.credential.rotation
d2b.credential.provider_health_check
```

### Required span attributes (closed set)

| Attribute | Value |
| --- | --- |
| `d2b.credential.provider` | `credential-managed-identity` (literal) |
| `d2b.credential.operation_class` | Closed enum string |
| `d2b.credential.placement_binding` | `host-system` or `guest-agent` |
| `d2b.credential.outcome` | Stable closed outcome code |
| `d2b.credential.rotation_generation` | Numeric rotation generation |

Credential telemetry uses only applicable generic OTEL Resource attributes from
the collector's closed allowlist:

| Resource attribute | Value |
| --- | --- |
| `d2b.zone` | Zone name, re-stamped at trusted ingress |
| `d2b.provider` | `credential-managed-identity` |
| `d2b.component` | Signed controller/agent component ID |
| `service.name` | Fixed controller or agent service name |
| `service.namespace` | Fixed service namespace |
| `service.version` | Build version |

No OTEL Resource attribute or span attribute carries a Credential resource
name, ResourceRef, UID, digest (including `resource_name_digest`), or derived
identity token.

### Forbidden from spans and resource attributes

Token bytes, audience literal, IMDS URL fragments, `clientId` value,
`imdsEndpointAlias` value, provider-internal diagnostics, host filesystem paths,
resource IDs, tenant/subscription IDs, Azure endpoint URIs, correlation IDs that
embed secret shapes.

### Metrics

| Metric | Type | Labels |
| --- | --- | --- |
| `d2b_credential_operations_total` | Counter | `provider="credential-managed-identity"`, `operation_class`, `placement_binding`, `outcome` |
| `d2b_credential_lease_expiry_seconds` | Gauge | `provider`, `placement_binding` |
| `d2b_credential_rotation_total` | Counter | `provider`, `policy`, `outcome` |
| `d2b_credential_provider_health` | Gauge (0/1) | `provider`, `placement_binding` |
| `d2b_credential_active_leases` | Gauge | `provider`, `placement_binding` |
| `d2b_credential_imds_calls_total` | Counter | `provider`, `alias`, `outcome` |

`alias` in `d2b_credential_imds_calls_total` uses only the closed alias string
(`azure-imds`, `azure-imds-aca`); no resolved endpoint URL appears in this label.
Label cardinality is bounded and semantic. The expiry gauge reports the
minimum seconds remaining across active leases in each provider/placement
aggregate (0 when none). Metric labels carry no Credential resource name,
ResourceRef, UID, digest, or derived identity token. Credential identity is
available only as `resource_name_digest` in authorized bounded audit records,
never telemetry. Generic allowlisted OTEL Resource attributes such as
`d2b.zone`, `d2b.provider`, and `d2b.component` remain available and are not
copied into metric labels or span attributes.

---

## Nix configuration

### Zone-level Credential declaration examples

```nix
# Azure VM host placement (host-system)
{
  d2b.zones.prod.resources = {
    credential-managed-identity = {
      type = "Provider";
      spec = {
        artifactId = "credential-managed-identity-bin";
        config = {
          clientId          = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
          imdsEndpointAlias = "azure-imds";
          maxLeases         = 64;
        };
      };
    };

    azure-vm-mi = {
      type = "Credential";
      spec = {
        providerRef    = "Provider/credential-managed-identity";
        scope = {
          executionRef  = "Host/azure-vm-host";
          domainFilter  = "system";
        };
        audience       = "https://management.azure.com/";
        consumerRef    = "Provider/runtime-azure-virtual-machine";
        allowedOperations = [ "acquire-token" "refresh-token" "inspect-metadata" ];
        rotation = {
          policy              = "proactive";
          proactiveWindowMs   = 300000;
          maxLeaseLifetimeMs  = 3600000;
        };
        revocation = {
          onOwnerDelete        = "immediate";
          onProviderGeneration = "immediate";
        };
      };
    };
  };
}
```

```nix
# ACA Guest placement (guest-agent)
{
  d2b.zones.prod.resources = {
    credential-managed-identity-aca = {
      type = "Provider";
      spec = {
        artifactId = "credential-managed-identity-bin";
        config = {
          clientId          = "bbbbbbbb-cccc-dddd-eeee-ffffffffffff";
          imdsEndpointAlias = "azure-imds-aca";
          maxLeases         = 32;
        };
      };
    };

    aca-relay-mi = {
      type = "Credential";
      spec = {
        providerRef = "Provider/credential-managed-identity-aca";
        scope = {
          executionRef  = "Guest/aca-sandbox";
          domainFilter  = "system";
        };
        audience    = "https://relay.servicebus.windows.net/";
        consumerRef = "Provider/runtime-azure-container-apps";
        allowedOperations = [ "acquire-token" "refresh-token" "revoke-token" "inspect-metadata" ];
        rotation = {
          policy             = "proactive";
          proactiveWindowMs  = 300000;
          maxLeaseLifetimeMs = 3600000;
        };
        revocation = {
          onOwnerDelete        = "immediate";
          onProviderGeneration = "immediate";
        };
      };
    };
  };
}
```

### Agent Process resources are controller-managed

The `managed-identity-agent` Process resources that the controller creates for
each Credential binding are **not present in the Nix-rendered Zone bundle** and
must never be authored in Nix configuration. They are created and managed
exclusively by the `managed-identity-controller` as owned children of each
Credential resource. Their `managedBy` field is set to the controller's process
identity; they do not appear in Nix `d2b.zones.<zone>.resources` configuration.

If a Nix configuration entry with `type = "Process"` and a name matching the
controller-generated `mi-agent-<credential-name>` pattern is encountered, the
eval-time assertion raises an error with the message:
`"mi-agent-* Process resources are managed by managed-identity-controller; remove from Nix configuration"`.

### Artifact catalog entry

```nix
# Controller and agent are separate binaries in the same package:
d2b.artifacts.credential-managed-identity-bin = {
  package = pkgs.d2b-provider-credential-managed-identity;
  type    = "provider";
  # Exposes both binaries:
  #   d2b-managed-identity-controller
  #   d2b-managed-identity-agent
};
```

The `type = "provider"` field is required. The artifact ID must match
`^[a-z][a-z0-9-]*$` (lowercase, starts with a letter, hyphens allowed, max 128
chars). Store paths and closure metadata
in the emitted artifact catalog are private; they never appear in any resource
spec, status, audit record, or log line.

### Eval-time assertions

Applied to every entry with `type = "Credential"` and
`spec.providerRef = "Provider/credential-managed-identity"`:

1. `spec.scope.domainFilter` must not be `"user"` - assertion failure with
   descriptive message.
2. `spec.scope.executionRef` resolves a declared `Host/<name>` or `Guest/<name>`
   in the same Zone.
3. `spec.allowedOperations` is a non-empty subset of
   `[acquire-token, refresh-token, revoke-token, inspect-metadata]`; `sign-challenge`
   triggers an assertion failure.
4. `spec.audience` passes `^[A-Za-z0-9._:/@-]+$` charset; max 256 chars;
   `contains_sensitive_shape` guard.
5. `spec.consumerRef`, if set, resolves a declared `Provider/<name>` in the same Zone.
6. `spec.rotation.proactiveWindowMs < spec.rotation.maxLeaseLifetimeMs / 2` when both
   are non-zero.
7. `spec.rotation.policy = "proactive"` requires `proactiveWindowMs > 0` and
   `maxLeaseLifetimeMs > 0`.
8. No two Credential resources in the same Zone declare the same
   `(providerRef, scope.executionRef, audience)` tuple (duplicate-binding conflict).
9. The Provider resource's `spec.artifactId` resolves an artifact catalog entry of
   `type = "provider"`; missing or wrong-type entry fails the build.

Applied to every entry with `type = "Provider"` and
`spec.config.imdsEndpointAlias` present:

10. `spec.config.imdsEndpointAlias` must be one of the closed alias values
    (`azure-imds`, `azure-imds-aca`); an unknown value fails the eval.
11. `spec.config.clientId` passes `OpaqueAzureRef` charset validation; a
    secret-shaped value fails `contains_sensitive_shape`.
12. `spec.config.maxLeases` must be 1-256 inclusive.

### Build-time validation

- Each Credential resource `spec` validated against
  `docs/reference/schemas/v3/credential.json`.
- Provider-specific schema `docs/reference/schemas/v3/provider-credential-managed-identity-config.json`
  cross-checked: `clientId` charset, `imdsEndpointAlias` closed enum.
- `make test-drift` regenerates schemas with `cargo xtask gen-schemas` and
  asserts `git diff --exit-code`. A schema change diverging from crate-derived
  output fails the gate.
- Bundle digest round-trip: sorted resources, digest matches header.
- Artifact catalog round-trip: digest matches header; store paths absent from
  resource bundle and status.
- `contains_sensitive_shape` detection covers all emitted bundle derivation outputs.

### Generation cleanup contract

When a Credential resource backed by `credential-managed-identity` is removed
from the Nix configuration:

1. `activation-nixos` issues an async Delete (sets `deletionRequestedAt`).
2. The Credential resource transitions to `phase=Degraded` with `Cleanup=True` /
   `reason=nix-generation-removed`.
3. The `managed-identity-controller` processes the `provider-revoke` finalizer:
   a. Sends `RevokeToken` to the owned agent Process (or marks all active leases
      revoked if the agent is already gone) under `revocation.onOwnerDelete` policy.
   b. Deletes the `mi-agent-<credential-name>` Process resource (awaits the
      agent's graceful shutdown before removing the finalizer).
4. Once the agent Process deletion watch event is received and lease revocation
   is confirmed, the controller clears the `provider-revoke` finalizer. The
   controller performs no store write and no audit emission at this step.
5. After the finalizer is cleared, Core atomically writes the event-only Deleted
   revision and removes the Credential row and index entries from the store. The
   audit subsystem then appends the Deleted-phase closure record from the
   committed revision; exactly-once delivery is handled by the audit subsystem's
   dedup key, with no controller involvement.
6. Stalled cleanup surfaces as `Degraded` / `nix-cleanup-stalled` with bounded
   retry.
7. `activation-nixos` activation completes without blocking on cleanup.
8. Up to `retainedGenerations` (default 3, range 1-16) prior bundles
   retained; rollback re-creates Credential resources from retained bundle
   (fresh leases acquired; prior IMDS tokens are not restored).
9. Controller-created and API-created Credential resources are never deleted by
   the generation cleanup path.

---

## Async reconcile

### Controller descriptor (wire form)

```yaml
providerId: Provider/credential-managed-identity
controllerType: Credential
resourceTypes: [Credential]
watchSelectors:
  - resourceType: Credential
    providerRefFilter: Provider/credential-managed-identity
  - resourceType: Provider
    nameFilter: credential-managed-identity
  - resourceType: Process
    ownerRefFilter: Provider/credential-managed-identity
dependencySelectors:
  - resourceType: Provider
    relationship: providerRef
  - resourceType: Host
    relationship: scope.executionRef
  - resourceType: Guest
    relationship: scope.executionRef
  - resourceType: Process
    relationship: ownerRef
ownerChildTriggers: [owned-resource-changed, agent-process-health-changed]
reconcileConcurrency: 8
maxPendingResources: 256
finalizers: [credential.d2bus.org/provider-revoke]
observeInterval: 30s
```

### Reconcile lifecycle

**Create**:

1. Validate `spec.scope.domainFilter != "user"`.
2. Validate `spec.allowedOperations` ⊆ supported set.
3. Validate `spec.scope.executionRef` resolves a live `Host` or `Guest`.
4. **Admit**: if checks 1-3 pass and the Provider is Ready, create and apply the
   `mi-agent-<credential-name>` Process resource (using the agent Process template
   in §Process resource templates), owned by this Credential.  The controller does
   not wait for the Credential to reach `phase=Ready` before spawning the agent;
   the Credential reaches `phase=Ready` only after the agent is itself Ready.
5. Wait for the agent Process to reach `phase=Ready`.
6. Agent reports first successful lease acquisition; controller sets
   `CredentialReady=True`, transitions Credential `phase=Active`.

**Agent spawn/teardown lifecycle**:

- **Spawn**: When the Credential spec is **admitted** (domain/allowedOperations
  checks pass) and dependencies are **ready** (executionRef live, Provider Ready),
  the controller creates the agent Process resource and attaches a LaunchTicket
  projecting the `imdsEndpointAlias` and `credentialRef`. The agent Process is
  owned by the Credential (`ownerRef: Credential/<name>`); its lifecycle is bound
  to the Credential's lifecycle. The Credential does not reach `phase=Ready`
  until the agent is Ready; the spawn is triggered by admission+dependency-ready,
  not by `phase=Ready`.
- **Monitor**: Controller watches agent Process health via `ownerChildTrigger`.
  On agent Process `phase=Failed` or `phase=Degraded`: controller sets
  `ProviderUnavailable=True` on the Credential resource and schedules respawn
  after a bounded backoff.
- **Respawn**: Controller deletes the failed agent Process resource and creates
  a new one. Consecutive respawn failures (three or more) set `phase=Failed` on
  the Credential resource with `outcome=agent-process-failed`.
- **Teardown on deletion**: On Credential deletion, the controller drains
  in-flight requests, revokes active leases, then issues a delete on the agent
  Process resource. The controller observes the agent Process deletion watch
  event. The agent Process has no persisted `phase=Deleted` row; the watch
  event fires when the resource is removed from the store. Once the watch event
  is received, the controller clears the `provider-revoke` finalizer. Core then
  atomically writes the event-only Deleted revision for the Credential and
  removes its row/index; the audit subsystem appends the closure record with
  exactly-once dedup. The controller never commits a store deletion or emits
  the Deleted-phase closure audit record.

**Spec update**:

- If `providerRef` changes: revoke old lease (via agent RPC) under
  `revocation.onProviderGeneration` policy; tear down old agent; create new
  agent with updated config.
- If `audience`, `scope`, or `rotation` changes and re-acquisition is required:
  revoke old lease and spawn new agent.
- If only `allowedOperations` or metadata changes: update status without
  agent respawn.

**Dependency-ready**:

- When `Provider/credential-managed-identity` transitions from `Pending` to
  `Ready`, re-attempt agent spawn for all Credentials in `phase=Pending` or
  `phase=Degraded` with `ProviderUnavailable=True`.

**Scheduled-observe** (every 30 s):

- Send `InspectMetadata` health-check RPC to each active agent Process.
- Agent calls `ManagedIdentityCredentialClient::inspect_lease` and returns
  opaque health state to controller (no token bytes in response).
- If `leaseState=Active` and `policy=proactive` and
  `now + proactiveWindowMs >= expiresAtUnixMs`: set `RotationDue=True`.
- If agent returns `Unavailable`: increment consecutive-failure count; after
  three failures, set `ProviderUnavailable=True`.
- Detects out-of-band revocations (IMDS infrastructure rotation).

**Proactive rotation** (triggered by `RotationDue=True`):

1. Send rotation request to agent; agent calls `issue_lease` with new
   idempotency key (new `rotationGeneration`).
2. On grant: increment `rotationGeneration`, store new `leaseHandle` and
   `expiresAtUnixMs`; commit status; clear `RotationDue`.
3. Old lease valid until new one is active.
4. On failure: bounded retry under `requeue-at`; degrade after final retry with
   `outcome=rotation-failed`.

**Deletion requested** (`deletionRequestedAt` set):

1. Send graceful-stop signal to agent Process; wait for agent to drain
   in-flight requests (bounded deadline).
2. Agent executes revocation: calls `RevokeToken` on all active leases per
   `revocation.onOwnerDelete` policy; marks `leaseState=Revoked` and reports
   to controller.
3. Controller issues delete on the agent Process resource and observes its
   deletion watch event. The agent Process has no persisted `phase=Deleted`
   row; the watch event fires when the resource is removed from the store.
4. After the watch event is received and revocation is confirmed (`Revoked` or
   `AlreadyRevoked`): controller clears the `provider-revoke` finalizer.
   Core then atomically writes the event-only Deleted revision for the Credential
   and removes its row/index. The audit subsystem appends the Deleted-phase
   closure record from the committed revision with exactly-once dedup. The
   controller never commits the store deletion or emits this audit record.
5. On agent `Unavailable` during teardown: mark `leaseState=Revoked` in status
   (operator must verify externally); proceed to step 4 with
   `outcome=forced-revocation`.

**Provider generation change**:

- `revocation.onProviderGeneration=immediate`: send `RevokeToken` to agent
  before generation changes. If agent unreachable: mark `leaseState=Revoked`
  and emit audit record.
- `revocation.onProviderGeneration=drain-leases`: do not actively revoke; active
  leases expire by natural deadline; status remains `Active` until expiry.

**Unknown/ambiguous state**:

- Agent Process unreachable or IMDS returning non-recoverable error: set
  `phase=Degraded`, `ProviderUnavailable=True`, `CredentialReady=False`;
  bounded retry with respawn. After retry exhaustion: set `phase=Failed`.

### Concurrency and ordering

- `reconcileConcurrency: 8` - up to 8 Credential resources reconciled in
  parallel.
- Independent resources (distinct executionRefs) proceed concurrently.
- Per-resource reconcile tasks serialize: at most one in-flight reconcile per
  Credential UID.
- The watch task keeps reading continuously while reconcile tasks run; new
  high-water hints are coalesced.
- Long-running IMDS calls (inside the agent) do not block controller
  reconciliation of unrelated resources.

---

## Source reuse

### v3 baseline (current code)

| Symbol | File | Evidence class | Treatment |
| --- | --- | --- | --- |
| `OpaqueAzureRef` + `AzureRefError` + tests | `packages/d2b-realm-provider/src/credential.rs` | `implemented-and-reachable` | Reused directly; `clientId` field validation delegates to `OpaqueAzureRef::parse` |
| `ManagedIdentityRef` | `packages/d2b-realm-provider/src/credential.rs` | `implemented-and-reachable` | `client_id: OpaqueAzureRef` maps to `clientId` config field (snake_case → camelCase); retained as reuse anchor |
| `CredentialProvider` trait (status/enrollment-only) | `packages/d2b-realm-provider/src/provider.rs` | `implemented-and-reachable` | Superseded by `d2b.credential.v3` service interface; removed only after v3 controller has full parity |
| `ProviderWorkloadIdentity::ManagedIdentity` | `packages/d2b-realm-provider/src/types.rs` | `implemented-and-reachable` (live ACA bootstrap path) | ACA bootstrap migration: v3 ACA Provider config uses `credentialRef` after `credential-managed-identity` controller is live |
| `managed_identity_client_id: Option<String>` | `packages/d2b-provider-aca/src/lib.rs` (line 112) | `implemented-and-reachable` | Superseded by Credential resource + `credentialRef` in ACA Provider config; removed by work item `ADR046-cred-mi-001` removal precondition |
| `managed_identity_client_id` | `packages/d2bd/src/lib.rs` (line 3960, 4173) | `implemented-and-reachable` | Superseded; see above |
| `contains_sensitive_shape` | `packages/d2b-realm-provider/src/error.rs` | `implemented-and-reachable` | Adapted for all string-field guards in Provider config, audit, and telemetry output |
| `no_secrets_or_credentials: bool` | `packages/d2b-core/src/realm_workloads_launcher.rs:LauncherMetadataInvariants` | `implemented-and-reachable` | Provides v3 evidence for the zero-secret-bytes design principle; Credential ResourceType extends this invariant to all resource/store/status/audit/log boundaries |

### Main-branch reuse (commit `a1cc0b2da4a08ca3240a770a972fe4da6f912bef`)

| Main source | Selected behavior | Excluded ADR 0045 assumptions |
| --- | --- | --- |
| `packages/d2b-provider-credential-managed-identity/src/lib.rs` | `ManagedIdentityCredentialProvider`, `ManagedIdentityCredentialClient` trait, `ManagedIdentityLeaseRequest/Ref/Grant/Inspection/Renewal/Revocation`, `ManagedIdentityCredentialOwner::ExactSdkConsumer` | v2 `AgentPlacementBinding`, v2 `EndpointRole`, v2 `ProviderFactory`, v2 `ProviderRegistryBuilder`, v2 component-session auth/prologue |
| `packages/d2b-provider-credential-managed-identity/src/tests.rs` | `FakeClient`, `credential_canary` enforcement, `colocated-consumer` tests, unavailable-state tests, idempotency tests | v2 realm/userd process model, v2 session/bootstrap assumptions |

Reuse action: **copy and adapt**. v3 contract names, module paths, and type
versions are substituted. The `d2b.credential.v3` service interface replaces
the v2 `CredentialProvider` trait. The v3 Provider resource/descriptor/
controller loop (with agent spawn/teardown) replaces the v2 provider registry.
The `Noise_KK` delivery channel terminated by the **agent process** replaces
any v2 token delivery path. Surrounding ADR 0045 registry, realm model,
endpoint-role, and provider-factory code is explicitly excluded.

---

## Work items

### ADR046-mi-topology-001 (agent process split)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-credential-001 and ADR046-credential-002; owner: credential-managed-identity controller/agent topology. This topology contract is consumed by the later managed-identity controller work. |
| Current source | `packages/d2b-realm-provider/src/provider.rs:CredentialProvider` status/enrollment-only trait; main `a1cc0b2d` managed-identity Provider implementation and tests listed in §Source reuse |
| Reuse action | adapt |
| Destination | packages/d2b-provider-credential-managed-identity/src/{controller.rs,agent.rs}; packages/d2b-provider-credential-managed-identity/{controller/main.rs,agent/main.rs}; packages/d2b-provider-credential-managed-identity/tests/topology.rs |
| Detailed design | Implement the controller/agent process split: separate `d2b-managed-identity-controller` binary with no IMDS client and no KK delivery, and `d2b-managed-identity-agent` binary with injected IMDS client via effect port and KK delivery. Controller manages Credential resources, spawns/monitors agent Processes, uses canonical Process templates, attaches LaunchTickets projecting `imdsEndpointAlias` and `credentialRef`, monitors agent Process health with bounded backoff, performs Deleted-phase cleanup without emitting Deleted closure audit, and applies D087 status-first state with no Provider state Volume. Agent validates `ExactSdkConsumer` via `AuthenticatedSubjectContext`, serves token-delivery methods, terminates Noise_KK delivery sessions, reports lease state, declares no direct IMDS egress, and keeps token bytes transient. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt the main managed-identity Provider; replace v2 provider registry/session assumptions with v3 controller/agent Process topology and `d2b.credential.v3` service. |
| Integration | ProviderDeployment starts the controller Process; controller reconciles Credential resources and creates agent Process/Endpoint resources at the declared executionRef; d2b-bus routes `d2b.credential.v3` calls to the agent; co-located runtime Provider injects the IMDS client through the LaunchTicket/effect port; core aggregates Provider status and audit subsystem appends deletion records. |
| Data migration | Full d2b 3.0 reset; no v2 managed-identity process/session state import |
| Validation | `tests/topology.rs`; `integration/host-guest-placement.nix`; `make test-rust`; `make test-integration`; `make test-host-integration` |
| Removal proof | V2 single-process/trait topology is superseded once controller and agent Process split is integrated and all token delivery terminates in the agent |

### ADR046-cred-mi-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-cred-mi-001` |
| Dependency/owner | `ADR046-credential-001` (contracts), `ADR046-credential-002` (service); `ADR046-reconcile-001`; `ADR046-mi-topology-001`; `credential-managed-identity` crate owner |
| Current source | `packages/d2b-realm-provider/src/credential.rs:ManagedIdentityRef` (reachable); `packages/d2b-provider-aca/src/lib.rs:managed_identity_client_id` line 112 (reachable ACA config); `packages/d2bd/src/lib.rs:managed_identity_client_id` lines 3960, 4173 (reachable) |
| Reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-managed-identity/src/lib.rs` (full implementation); `src/tests.rs` (full test suite) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-credential-managed-identity/src/{lib.rs, controller.rs, agent.rs, service.rs, audit.rs, telemetry.rs}`; `packages/d2b-provider-credential-managed-identity/{controller/main.rs, agent/main.rs}`; `packages/d2b-provider-credential-managed-identity/tests/{lifecycle.rs, conformance.rs, faults.rs, canary.rs, delivery.rs, placement.rs, topology.rs}`; `packages/d2b-provider-credential-managed-identity/integration/{container-service.sh, host-guest-placement.nix, aca-credential-ref.sh, cleanup-rollback.sh}`; `packages/d2b-provider-credential-managed-identity/README.md` |
| Detailed design | (1) Adapt `ManagedIdentityCredentialProvider` to `d2b.credential.v3` service interface; split controller and agent roles (see `ADR046-mi-topology-001`). (2) Enforce `ManagedIdentityCredentialOwner::ExactSdkConsumer` in agent via `AuthenticatedSubjectContext` from ComponentSession, independently of scope fields. (3) Reject `user-agent` placement: `scope.domainFilter=user` returns `credential-placement-mismatch` before agent spawn. (4) Validate `clientId` using `OpaqueAzureRef::parse` from v3 baseline; artifact IDs match `^[a-z][a-z0-9-]*$`. (5) Validate `imdsEndpointAlias` against closed enum `{azure-imds, azure-imds-aca}`; project into LaunchTicket at spawn time (never into Process spec config or env); co-located runtime Provider constructs `ManagedIdentityCredentialClient` from LaunchTicket projection and supplies via effect port; resolved URL never in any output surface. (6) Agent Process declares `networkUsage.allowEgress=false`; uses canonical Process template shape (see `ADR046-mi-topology-001` design item 6). (7) Reject `sign-challenge` with `credential-schema-invalid` immediately. (8) Map `ManagedIdentityClientState::Unavailable` to `credential-provider-unavailable`; no `InteractionRequired` state. (9) Implement `ManagedIdentityLeaseHandle` as opaque bounded newtype with redacted `Debug`. (10) All token bytes held by injected client in agent; delivered only via agent-terminated `Noise_KK` delivery session. (11) Integrate with Provider resource descriptor and controller toolkit. (12) Confirm `credential_canary` never appears in any service response, status field, delivery record outer header, or audit record. (13) Apply D087 status-first state: declare no Provider state Volume, keep ProviderStateSet empty, and write only bounded non-secret lease observation to `Credential.status` plus the Operation ledger. Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | Agent `Process` resource under `Host` or `Guest` executionRef; controller `Process` resource at Zone system host; d2b-bus routes `d2b.credential.v3` token-delivery calls to agent; Credential controller reconciles status; ACA `Provider/runtime-azure-container-apps` holds `credentialRef` pointing to a `credential-managed-identity`-backed Credential resource |
| Data migration | Full v3 reset. `d2b-provider-aca:managed_identity_client_id` raw field migrated to a Credential resource reference in the v3 ACA Provider config; see removal precondition below. |
| Validation | See §Tests. Run `cargo test -p d2b-provider-credential-managed-identity --lib --test lifecycle --test conformance --test faults --test canary --test delivery --test placement --test topology`; run `integration/{container-service.sh,aca-credential-ref.sh,cleanup-rollback.sh}` through `make test-integration` and `integration/host-guest-placement.nix` through `make test-host-integration`. |
| Removal proof | `d2b-provider-aca:managed_identity_client_id` raw field removed only after the `credential-managed-identity` controller and agent are integrated and the ACA Provider config uses `credentialRef` exclusively; `ProviderWorkloadIdentity::ManagedIdentity` bootstrap path superseded only after the ACA Provider controller uses the Credential resource for token acquisition |

### ADR046-cred-mi-002 (shared with other Credential Providers)

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-credential-001, ADR046-credential-002; ADR046-reconcile-001, ADR046-reconcile-002; ADR046-mi-topology-001; owner: Credential controller toolkit and managed-identity controller |
| Current source | `packages/d2b-realm-provider/src/provider.rs:CredentialProvider` status/enrollment-only trait; main `a1cc0b2d` managed-identity controller/test behavior listed in §Source reuse |
| Reuse action | adapt |
| Destination | packages/d2b-provider-credential-managed-identity/src/controller.rs; packages/d2b-contracts/src/v3/credential_controller.rs |
| Detailed design | Managed-identity-specific controller design: implement async reconcile and agent spawn/teardown from §Async reconcile; enforce system-only domain; spawn agent on Credential admission plus dependency-ready, not on `phase=Ready`; implement `observeInterval=30s` health-check RPC to the agent, which calls `InspectMetadata` on the injected client; controller never calls IMDS; derive idempotency key as `SHA-256(UID \|\| ":" \|\| rotationGeneration.to_le_bytes() \|\| ":" \|\| operation_class_byte)`; enforce `MAX_LOCAL_LEASES=256` in the resource store; implement Deleted-phase closure by clearing `provider-revoke` only after agent Process deletion and revocation confirmation while core/audit own Deleted revision and deletion record. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt shared Credential controller lifecycle to managed-identity controller/agent spawn and teardown. |
| Integration | Shared Credential controller contract produces reconcile events; managed-identity controller consumes them, manages agent Process resources, and writes Credential/agent status; generated controller contracts are consumed by all Credential Providers. |
| Data migration | None - controller lifecycle code only; no runtime state import |
| Validation | Managed-identity reconcile/controller tests; shared Credential reconciliation tests; topology tests validating agent spawn/teardown and Deleted-phase cleanup |
| Removal proof | V2 `CredentialProvider` status/enrollment trait lifecycle is superseded after shared controller reconcile and managed-identity agent lifecycle pass parity |

This work item is shared with `credential-secret-service` and `credential-entra`;
the table above is the `managed-identity-controller` entry point for this work
item.

### ADR046-cred-mi-003 (shared Nix + cleanup)

| Field | Value |
| --- | --- |
| Dependency/owner | Depends on ADR046-credential-001, ADR046-credential-002, and ADR046-cred-mi-001; owner: Nix resource compiler and activation cleanup |
| Current source | `packages/d2b-provider-aca/src/lib.rs:managed_identity_client_id` and `packages/d2bd/src/lib.rs:managed_identity_client_id` are the current raw ACA managed-identity config surfaces; v3 Provider/Credential resource authoring is net-new |
| Reuse action | replace |
| Destination | nixos-modules/options-resources.nix; nixos-modules/activation-nixos-cleanup.nix; integration/aca-credential-ref.sh |
| Detailed design | Shared Nix and cleanup: implement Nix eval-time assertions 1-12 from §Eval-time assertions, closed enum schema for `imdsEndpointAlias`, `clientId` validation via `OpaqueAzureRef` charset, generation cleanup contract, and artifact catalog validation for `credential-managed-identity-bin`. Integration fixture asserts that the migrated ACA Provider config carries `credentialRef: "Credential/aca-relay-mi"` and the raw `managed_identity_client_id` string field is absent from rendered Provider config JSON and ACA runtime bundle. Primary reuse disposition: `replace`. Preserved source-plan detail: replace raw ACA managed identity client-id fields with Credential resource references and v3 Provider/Credential Nix emission. |
| Integration | Nix compiler emits Provider/Credential/ACA resource config; ResourceAPI admission validates it; activation cleanup deletes old generation resources through finalizers; ACA Provider consumes `credentialRef` to obtain tokens from the managed-identity Credential Provider. |
| Data migration | Full d2b 3.0 reset; raw ACA `managed_identity_client_id` config is replaced by a newly authored Credential resource reference rather than imported in place |
| Validation | Nix eval assertion tests; artifact catalog validation tests; `integration/aca-credential-ref.sh` |
| Removal proof | `managed_identity_client_id` raw fields in ACA config and daemon plumbing are removed only after ACA Provider config uses `credentialRef` exclusively |

### ADR046-cred-mi-004 (shared audit/OTEL)

| Field | Value |
| --- | --- |
| Dependency/owner | Depends on ADR046-cred-mi-001 and ADR046-mi-topology-001; owner: credential-managed-identity audit/telemetry implementation |
| Current source | `packages/d2b-realm-provider/src/error.rs:contains_sensitive_shape`; `packages/d2b-core/src/realm_workloads_launcher.rs:LauncherMetadataInvariants.no_secrets_or_credentials`; main `a1cc0b2d` managed-identity canary tests listed in §Source reuse |
| Reuse action | adapt |
| Destination | packages/d2b-provider-credential-managed-identity/src/{audit.rs,telemetry.rs}; packages/d2b-contract-tests/tests/credential_audit.rs |
| Detailed design | Shared audit/OTEL: emit audit records for all methods and controller events per §Audit, with Credential identity represented only by the authorized bounded `resource_name_digest`; emit OTEL spans and metrics per §OTEL and metrics with no Credential resource name, ResourceRef, UID, digest, derived identity token, Zone/Credential/resource-name-derived label, or non-allowlisted OTEL Resource attribute; retain applicable generic collector-allowlisted Resource attributes (`d2b.zone`, `d2b.provider`, `d2b.component`, and service fields); report expiry as the minimum for each provider/placement aggregate; add `d2b_credential_imds_calls_total` counter with bounded `alias` label; enforce `contains_sensitive_shape` on all string fields in audit records and metric labels; add canary tests for `managed-identity-canary`, `credential_canary`, `imds-endpoint-canary`, Credential name/ref/UID/digest, and Zone name in `canary.rs`. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt existing sensitive-shape guard and canary pattern to v3 audit, OTEL, and metric surfaces. |
| Integration | Controller and agent service methods call audit/telemetry helpers; audit subsystem and OTEL exporters consume bounded redacted records; contract tests validate credential audit shape across providers. |
| Data migration | None - audit/telemetry only; no runtime state import |
| Validation | `packages/d2b-contract-tests/tests/credential_audit.rs` requires `resource_name_digest` in authorized audit records and rejects raw Credential name/ResourceRef/UID; managed-identity `canary.rs` and audit/OTEL unit tests structurally assert exact absence of `vm`, `zone`, `zone_id`, `zone_uid`, `credential_name`, `credential_ref`, `credential_uid`, `credential_digest`, `resource_name_digest`, and every resource-name-derived label key; reject Credential name/ref/UID/digest canaries from all OTEL Resource attributes, span attributes, and metric labels; preserve generic collector-allowlisted Resource attributes including `d2b.zone`, `d2b.provider`, `d2b.component`, and service fields; reject Zone-name span/label canaries and sensitive shapes; pass complete managed-identity metric/span frames through the shared collector ingress validator and prove that adding `d2b.credential.name` or any Credential identity key/value rejects the whole frame |
| Removal proof | None - audit/telemetry helpers are additive; no prior owner to remove |

---

## Tests

### Required crate layout

```
packages/d2b-provider-credential-managed-identity/
  src/
    lib.rs           (ManagedIdentityCredentialProvider, client trait, lease types, redaction, unit tests)
    controller.rs    (async reconcile loop, agent spawn/teardown, status machine, observe, rotation/revocation)
    agent.rs         (agent service handler: AcquireToken/RefreshToken/RevokeToken/InspectMetadata; no ambient IMDS chain)
    service.rs       (d2b.credential.v3 service routing; delivery session integration)
    audit.rs         (audit record emission, Deleted-phase closure, contains_sensitive_shape guards)
    telemetry.rs     (OTEL span/metric emission, closed label set)
  controller/
    main.rs          (controller binary entry point: config parse, no IMDS client construction)
  agent/
    main.rs          (agent binary entry point: config parse, injected IMDS client construction, no ambient chain)
  tests/
    lifecycle.rs
    conformance.rs
    faults.rs
    canary.rs
    delivery.rs
    placement.rs
    topology.rs
  integration/
    container-service.sh
    host-guest-placement.nix
    aca-credential-ref.sh
    cleanup-rollback.sh
  README.md
```

Missing any of `src/`, `tests/`, `integration/`, or `README.md` is a workspace
policy failure enforced by `make test-policy` (via `packages/d2b-contract-tests`).

### `src/` unit tests (`#[cfg(test)]` in `src/` files)

| Test | Validates |
| --- | --- |
| `OpaqueAzureRef::parse` on `clientId`: valid GUIDs accepted; secret-shaped values rejected | `clientId` fail-closed parse |
| `imdsEndpointAlias` closed enum: `azure-imds` and `azure-imds-aca` accepted; unknown value rejected | Alias validation |
| `ManagedIdentityCredentialOwner::ExactSdkConsumer` guard (via `AuthenticatedSubjectContext`): wrong consumer subject returns `credential-consumer-mismatch`; check is on authenticated bus identity, not caller-supplied field | Owner enforcement via bus identity |
| `ManagedIdentityClientState` transitions: `Ready → Unavailable` and back | State machine transitions |
| `sign-challenge` returns `credential-schema-invalid` immediately without IMDS call | Operation class rejection |
| `ManagedIdentityLeaseHandle` debug output emits only `REDACTED` | Redacted Debug |
| `user-agent` placement construction fails with `credential-placement-mismatch` | Placement rejection |
| `contains_sensitive_shape` on all string fields in config, audit output, metric labels | Secret-shape guard |
| `maxLeases=0` and `maxLeases=257` rejected; `1` and `256` accepted | Bounds validation |

### `tests/` Cargo integration tests (`cargo test -p d2b-provider-credential-managed-identity`)

#### `lifecycle.rs`

| Test | Validates |
| --- | --- |
| Acquire → Active: `FakeClient::issue_lease` called with correct idempotency key; status `leaseState=Active`, `CredentialReady=True` | End-to-end acquire path (via agent) |
| Refresh: `FakeClient::refresh_lease` called (agent); status `expiresAtUnixMs` updated; `rotationGeneration` unchanged | Refresh path |
| Revoke: `FakeClient::revoke_lease` called (agent); status `leaseState=Revoked`, `CredentialReady=False` | Revoke path |
| InspectMetadata: `FakeClient::inspect_lease` called (agent); opaque metadata returned; no token bytes in response DTO | Inspect path |
| Proactive rotation: `RotationDue=True` triggers agent `issue_lease` with incremented `rotationGeneration` | Rotation trigger |
| Idempotent acquire: second call with same idempotency key returns same grant; `FakeClient::issue_lease` called exactly once | Idempotency |
| Re-acquire after expiry: `leaseState=Expired` → `AcquireToken` → `leaseState=Active`, `rotationGeneration` incremented | Expired re-acquire |
| Provider generation change (immediate): agent `RevokeToken` called on old lease before generation change; agent respawned with new config | Generation revocation |
| Provider generation change (drain-leases): no `RevokeToken` call; lease remains Active until natural expiry | Drain-leases policy |

#### `conformance.rs`

Calls `d2b_provider_toolkit::conformance::check_provider_conformance` with all
11 standard arms. Every arm must pass. Covers: descriptor validation, schema
fingerprint, placement binding declarations, operation class declarations,
`sign-challenge` absent from supported operations, `user-agent` absent from
supported placement bindings, status subresource auth, finalizer ownership,
audit record field set conformance, delivery session binding contract, RBAC
`use-credential` verb.

#### `faults.rs`

| Test | Validates |
| --- | --- |
| `FakeClient` returns `Unavailable` on `issue_lease`: status `ProviderUnavailable=True`, `phase=Degraded`, `CredentialReady=False` | Unavailability |
| Three consecutive `InspectMetadata` `Unavailable` responses: `ProviderUnavailable=True` set | Consecutive-failure threshold |
| `ProviderUnavailable` recovery: `FakeClient` returns success on next call; status `ProviderUnavailable=False` | Recovery path |
| `maxLeases` ceiling: `maxLeases+1`-th acquire returns `credential-queue-pressure`; count decrements on revoke | Cardinality limit |
| Generation mismatch on delivery session: rejected with closed channel | Generation mismatch |
| Provider process unreachable during `provider-revoke` finalizer: `leaseState=Revoked` recorded in status; audit record emitted; finalizer proceeds | Finalizer with unreachable provider |

#### `canary.rs`

| Test | Validates |
| --- | --- |
| `"managed-identity-canary"` absent from every `FakeClient` response DTO field, status field, audit record field, metric label value, OTEL Resource attribute, OTEL span attribute, and delivery record outer header | Zero-secret-bytes invariant |
| `"credential_canary"` absent from same surfaces | Cross-provider canary consistency |
| `"imds-endpoint-canary"` (a value that looks like an IMDS endpoint URL) absent from all output surfaces | Endpoint URL exclusion |
| `clientId` value absent from audit records, metric labels, OTEL spans, error messages, and log lines | Config field exclusion |
| `imdsEndpointAlias` value absent from OTEL span attributes, audit records, and error messages | Alias exclusion |
| IMDS response-shaped string absent from status, audit, OTEL, and logs | IMDS response content exclusion |
| Metric descriptors contain no `vm`, `zone`, `zone_id`, `zone_uid`, `credential_name`, `credential_ref`, `credential_uid`, `credential_digest`, `resource_name_digest`, or resource-name-derived key; Credential name/ref/UID/digest canaries are absent from OTEL Resource attributes, span attributes, and metric labels; Zone-name canaries are absent from spans and labels while generic collector-allowlisted Resource attributes remain | Structural telemetry identity exclusion |
| Complete metric/span frames with only generic allowlisted Resource attributes are accepted by the shared collector ingress validator; injecting `d2b.credential.name` or any Credential name/ref/UID/digest key or value rejects the whole frame | Closed collector allowlist gate |

#### `delivery.rs`

| Test | Validates |
| --- | --- |
| `AcquireToken` delivery session: binding contract fields all present; outer DTO carries no token bytes; fake delivery record zeroized after extraction | E2E delivery path |
| `RefreshToken` delivery session: new binding, same `credentialRef`; replay-safe sequence incremented | Refresh delivery |
| Oversize delivery record rejected; channel closed and zeroized | Size bound enforcement |
| Replay of prior session's ciphertext at same sequence number rejected | Replay-safe sequence |
| NN-profile delivery attempt rejected immediately | Profile enforcement |
| Delivery session key material zeroized after close; post-close send returns error | Session zeroize |

#### `placement.rs`

| Test | Validates |
| --- | --- |
| `scope.domainFilter=user` at construction: returns `credential-placement-mismatch` before any IMDS call or agent spawn | User-agent rejection |
| `host-system` with `azure-imds` alias: agent receives injected `ManagedIdentityCredentialClient` via effect port (LaunchTicket projection); no ambient discovery; `azure_identity::DefaultAzureCredential` absent from agent binary import graph | Host-system placement / ambient-free |
| `guest-agent` with `azure-imds-aca` alias: agent receives correct injected client via effect port | Guest-agent placement |
| `ExactSdkConsumer` with non-matching consumer `AuthenticatedSubjectContext` (forged field in request): `credential-consumer-mismatch` returned by agent; controller does not receive the mismatch | Consumer mismatch via bus identity |
| `ExactSdkConsumer` with matching `AuthenticatedSubjectContext` and null `spec.consumerRef`: authenticated caller accepted | Null consumerRef path |

#### `topology.rs`

| Test | Validates |
| --- | --- |
| Credential spec admitted + dependencies ready: controller creates `mi-agent-<name>` Process resource owned by Credential; agent Process transitions `phase=Ready`; controller sets Credential `CredentialReady=True` | Agent spawn on admission+dependency-ready (not on phase=Ready) |
| Credential not yet admitted (executionRef unresolved): controller does not spawn agent; Credential stays `phase=Pending` | No premature spawn |
| Credential `AcquireToken` request routed to agent, not controller; controller has no `FakeClient`; controller's `FakeClient` call count remains zero | Method dispatch to agent |
| Controller receives agent status-update RPC after `FakeClient::issue_lease` succeeds; controller commits status to resource store | Agent-to-controller status reporting |
| Agent Process `phase=Failed`: controller sets `ProviderUnavailable=True` on Credential; schedules respawn after backoff | Agent failure → controller response |
| Agent respawn: controller creates new `mi-agent-<name>` Process resource; new agent transitions `Ready`; controller clears `ProviderUnavailable` | Agent restart recovery |
| Three consecutive agent spawn failures: controller sets Credential `phase=Failed`, `outcome=agent-process-failed` | Respawn exhaustion |
| Credential `Delete`: controller sends graceful-stop to agent; agent drains in-flight requests and revokes leases; controller issues delete on agent Process resource; controller observes agent Process deletion watch event (no persisted `phase=Deleted` row for agent Process); controller clears `provider-revoke` finalizer; Core atomically writes event-only Deleted revision and removes Credential row/index; audit subsystem appends closure record with exactly-once dedup - finalizer cleared before Core deletion, Core deletion before audit, all verified in order | Graceful teardown; finalizer-then-Core-deletion-then-audit ordering |
| Audit exactly-once: simulate Core Deleted revision committed with no corresponding audit record; audit subsystem on recovery appends exactly once using dedup key bound to committed revision; controller does not re-emit; no duplicate in audit log | Audit subsystem exactly-once / no controller re-emit |
| Agent Process `ownerRef` matches Credential UID: controller watch filter correctly associates agent Process events with the owning Credential | ownerRef watch correctness |
| Controller `InspectMetadata` path: returns stored `leaseState` without calling `FakeClient`; agent `FakeClient` call count unchanged | Controller-side metadata inspection |
| Agent Process spec has `networkUsage.allowEgress=false`; agent receives `ManagedIdentityCredentialClient` via effect port (no direct network calls in agent binary) | Effect-port injection; no ambient egress |
| Agent Process template: all required canonical fields present (processClass, template, namespaceClasses `[mount,pid,ipc]`, capabilityClasses `[]`, seccompClass `strict`, startRoot `false`, noNewPrivileges `true`, environmentClass `minimal`, readOnlyRoot `true`, budget nested cpu.request/cpu.limit/memory.request/memory.limit/pids.limit/fds.limit, no inline endpoint fields, owned credential-service Endpoint resource, readiness class `provider-defined`/initialDelay/timeout/failureThreshold/successThreshold, telemetry metricsEnabled/tracingEnabled/logLevel/sensitiveLabels); all excluded fields absent (principalRef, profileRef, endpoint.kind, Process config, telemetry.componentRef, readiness.probe/timeoutMs, network.allowedEffects, budget.class, telemetry.class); agent `networkUsage: {networkRef:null,ports:[],allowEgress:false}` | Canonical template shape |
| Controller binary (`controller/main.rs`) constructed with no `ManagedIdentityCredentialClient` import; `FakeClient` inaccessible from controller entry point | Controller secret-free invariant |

### `integration/` fixtures

These are invoked by existing repository test orchestration (`make test-integration`,
`make test-host-integration`). They are shell scripts, Nix expressions, or
container specs; they are **not** run by `cargo test`.

#### `container-service.sh`

Container-backed Provider service lifecycle: start controller and agent
services with FakeClient-backed IMDS; confirm `d2b.credential.v3` methods
reachable; stop agent Process; confirm `ProviderUnavailable=True` within
`observeInterval`; controller respawns agent; confirm recovery.

#### `host-guest-placement.nix`

`runNixOSTest` fixture verifying:

- `host-system` placement: `managed-identity-controller` Process launched as
  Zone system host; `mi-agent-<name>` Process spawned by controller co-located
  under `Host/azure-vm-host`; `AcquireToken` via d2b-bus succeeds (routed to
  agent); status `placementBinding=host-system`. Controller never calls IMDS
  (verified via FakeClient call-count assertions on the controller side).
- `guest-agent` placement: controller spawns `mi-agent-<name>` Process
  co-located under `Guest/aca-sandbox`; `AcquireToken` via d2b-bus succeeds;
  status `placementBinding=guest-agent`.
- `user-agent` attempt: Credential resource with `domainFilter=user` fails
  NixOS eval-time assertion before build completes.

#### `aca-credential-ref.sh`

Verifies the ACA Provider migration path:

- Renders an ACA Provider config with `credentialRef: "Credential/aca-relay-mi"`.
- Confirms the rendered Provider config JSON does not contain the string
  `managed_identity_client_id` or any raw client-ID value.
- Confirms the `aca-relay-mi` Credential resource appears in the Zone resource
  bundle with correct `providerRef` and `consumerRef`.
- Confirms `d2b-provider-aca` receives only the opaque `Credential/<name>` ref
  at runtime; the raw `managed_identity_client_id` field is absent from the
  ACA runtime's bundle artifact.

#### `cleanup-rollback.sh`

Nix-generation lifecycle:

- Generation N declares `aca-relay-mi` Credential resource.
- Generation N+1 removes `aca-relay-mi`.
- Confirms `activation-nixos` completes (new resources Ready) before `aca-relay-mi`
  cleanup finalizer finishes.
- Confirms `aca-relay-mi` transitions `phase=Degraded` with `Cleanup=True` /
  `reason=nix-generation-removed` during cleanup.
- Confirms that after the `provider-revoke` finalizer completes, a watch event
  carrying the event-only Deleted revision is observed for `aca-relay-mi`, and
  that a subsequent resource lookup returns not-found - no persisted Deleted row
  remains in the store.
- Rollback from generation N+1 to N: `aca-relay-mi` re-created from retained
  bundle; fresh lease acquired; prior IMDS token not restored.

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has an advisory wall-clock p95
diagnostic threshold of <=50 ms; gate enforcement is aggregate per-crate
process CPU only. There is no wall-clock
sleep, and `cargo test -p d2b-provider-credential-managed-identity --lib --tests`
completes in ≤3 s warm-cache execution time (compilation excluded). They use a
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only - no
process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build,
KVM, USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny
temp fixtures. Any scenario needing those lives only in `integration/`, which
keeps a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test advisory threshold.

---

## Removal preconditions

| Item to remove | Removal precondition |
| --- | --- |
| `d2b-provider-aca/src/lib.rs:managed_identity_client_id: Option<String>` (line 112) and all references | `credential-managed-identity` Provider controller integrated and live; ACA Provider config uses `credentialRef: "Credential/<name>"` exclusively; `integration/aca-credential-ref.sh` passes |
| `d2bd/src/lib.rs:managed_identity_client_id` (lines 3960, 4173) and all references | Same as above |
| `d2b-realm-provider/src/types.rs:ProviderWorkloadIdentity::ManagedIdentity` bootstrap path | ACA Provider controller uses Credential resource for token acquisition; no remaining callsite |
| `d2b-realm-provider/src/provider.rs:CredentialProvider` trait (status/enrollment-only) | All three v3 Credential controllers (`secret-service`, `entra`, `managed-identity`) have full reconcile parity and their `tests/conformance.rs` suites pass |
| Old v2 `d2b-contracts/proto/v2/provider_credential.proto` | All v3 callers migrated to `d2b-contracts/proto/v3/credential.proto` |

No removal is performed until the live successor passes all required validation.
The current v3 `ManagedIdentityRef` type itself is **not** removed; it is
retained as the `OpaqueAzureRef` reuse anchor and the `clientId` field source.

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass -
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.

---

## Current-code fit summary

| Item | Treatment |
| --- | --- |
| Current anchor | `packages/d2b-realm-provider/src/credential.rs:ManagedIdentityRef` (implemented-and-reachable); `d2b-provider-aca/src/lib.rs:managed_identity_client_id` (implemented-and-reachable); `d2bd/src/lib.rs:managed_identity_client_id` (implemented-and-reachable); `CredentialProvider` trait (minimal, implemented-and-reachable); `OpaqueAzureRef` + tests (implemented-and-reachable); `contains_sensitive_shape` (implemented-and-reachable) |
| Evidence class | Opaque ref data model and ACA config path are reachable; full lease provider, `d2b.credential.v3` service, controller, and delivery channel are ADR-only |
| Behavior retained | `OpaqueAzureRef` charset/length validation fail-closed on `clientId`; injected-client pattern keeps token material in the client process; zero-secret-bytes invariant at config/audit/log boundary; `ExactSdkConsumer` ownership model; `contains_sensitive_shape` guard |
| Required delta | Full `ManagedIdentityCredentialProvider` implementation; split `managed-identity-controller` (no IMDS) + `managed-identity-agent` (IMDS client, KK delivery); `d2b.credential.v3` service dispatch split by role; agent spawn/teardown lifecycle in controller reconcile; `Noise_KK` delivery channel terminated by agent; Provider resource/descriptor; agent Process template; `imdsEndpointAlias` closed-alias config; `ExactSdkConsumer` via `AuthenticatedSubjectContext`; D087 status-first state model with no Provider state Volume, empty ProviderStateSet, status/Operation-ledger lease observation, and transient-only token bytes; Nix eval assertions; controller finalizer-release-only deletion path; Core-written event-only Deleted revision; audit subsystem deletion record with exactly-once dedup; audit/OTEL; `aca-credential-ref` migration |
| Feasibility proof | Main `a1cc0b2d` proves the `ManagedIdentityCredentialProvider` implementation; its `FakeClient` test suite covers acquire/refresh/revoke/inspect, idempotency, unavailable state, canary enforcement, `ExactSdkConsumer` enforcement, and lease cardinality |
| Future owner | `ADR046-mi-topology-001` (process split), `ADR046-cred-mi-001` (primary), `ADR046-cred-mi-002`, `ADR046-cred-mi-003`, `ADR046-cred-mi-004` |

---

## Provider README required sections

The `packages/d2b-provider-credential-managed-identity/README.md` MUST contain
these sections in order:

1. **Provider identity** - `Provider/credential-managed-identity`, managed
   ResourceType (`Credential`), provider generation/versioning policy,
   Zone placement constraints (`host-system` and `guest-agent` only).
2. **Config schema** - `spec.config` fields (`clientId`, `imdsEndpointAlias`,
   `maxLeases`), types, bounds, constraints, and worked examples for both
   Azure VM and ACA placement using the `d2b.zones.<zone>.resources` Nix
   authoring shape.
3. **ResourceTypes managed** - `Credential` only: lifecycle phases, status
   conditions owned (`CredentialReady`, `RotationDue`, `ProviderUnavailable`,
   `LeaseRevoked`), finalizers owned (`credential.d2bus.org/provider-revoke`).
   `Volume` is **not** listed here; this Provider declares no Provider state
   Volume under D087 because no managed-identity payload passes the
   storage-need test.
4. **Controllers, services, workers, and binaries** - two binaries:
   `d2b-managed-identity-controller` (controller; one per Zone; system domain;
   no IMDS access; spawns/supervises agent Processes) and
   `d2b-managed-identity-agent` (service; one per Credential binding;
   co-located at `executionRef`; holds IMDS client; terminates KK delivery;
   owned by Credential resource; controller-managed, not Nix-configured).
5. **Placement** - `host-system` and `guest-agent` supported;
   `user-agent` rejected with `credential-placement-mismatch`; `sign-challenge`
   rejected with `credential-schema-invalid`; agent Process resource
   co-located at `scope.executionRef`.
6. **Dependencies and RBAC** - required Zone resources (`executionRef Host|Guest`,
   `consumerRef Provider`), `use-credential` verb, `ExactSdkConsumer` enforcement
   via `AuthenticatedSubjectContext`, `Noise_KK` enrolled key requirement.
7. **Security, state, and telemetry** - no ambient IMDS chain in agent; no env
   fallback; controller holds no IMDS client; zero-secret-bytes invariant;
   opaque lease handles only; `credential_canary` and `imds-endpoint-canary`
   enforcement; Deleted-phase closure audit record; OTEL spans/metrics.
8. **Build, test, and integration commands** - exact `cargo`/`make` invocations:
   `cargo test -p d2b-provider-credential-managed-identity`,
   `make test-integration`, `make test-host-integration`.
9. **Standalone-repo usage** *(mandatory before first release to a sibling flake)* -
   flake input pattern; `nixpkgs`/toolkit input-follows boilerplate; compatibility
   constraints; `d2b.artifacts` catalog entry pattern.
