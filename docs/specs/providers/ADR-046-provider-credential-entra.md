# Provider dossier: `credential-entra`

| Field | Value |
| --- | --- |
| Dossier ID | `ADR-046-provider-credential-entra` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owner | `packages/d2b-provider-credential-entra/` |
| Depends on | `ADR-046-resources-credential`, `ADR-046-provider-model-and-packaging`, `ADR-046-componentsession-and-bus`, `ADR-046-resource-reconciliation`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support` |
| Supersedes | `d2b-realm-provider/src/credential.rs:AzureControlPlaneRef`/`OpaqueAzureRef`; `provider.rs:CredentialProvider` minimal trait |

---

## 1. Provider identity

| Field | Value |
| --- | --- |
| `providerRef` | `Provider/credential-entra` |
| Implements | `Credential` ResourceType |
| Crate | `packages/d2b-provider-credential-entra/` |
| Binaries | `d2b-provider-credential-entra-controller` (central, secret-free); `d2b-provider-credential-entra-agent` (per-Credential, co-located with consumer) |
| Required layout | `src/` (impl + colocated unit tests); `tests/` (hermetic Cargo integration, conformance, fault, canary, delivery, placement); `integration/` (container/Host/Guest fixtures); `README.md` (all §Provider README required sections) |
| Main reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-entra/src/{lib.rs, tests.rs}` |
| Provider generation policy | Monotonically increasing; generation increments on any spec-changing upgrade; consumers must re-acquire leases on provider generation change per `revocation.onProviderGeneration` policy |
| Zone placement constraints | Controller: one instance per Zone, system-domain on Zone host, no secret material. Agent: one per active Credential resource, placed at `scope.executionRef/domain/userRef`; placement binding `user-agent` or `guest-agent` only; `host-system` rejected |

---

## 2. Root `spec.config` schema

`spec.config` is a child of `Provider.spec`, not inside any nested config block.
`spec.artifactId` is a sibling of `spec.config` on the Provider resource.
No field in `spec.config` accepts secret bytes; all string fields pass the
`contains_sensitive_shape` guard at eval time before the NixOS build completes.

### Schema (bounded, non-secret)

```yaml
# Provider.spec.config — non-secret runtime config only
tenantId:                 "2f8e1c3a-1234-5678-9abc-def012345678"
authorityClass:           "public"
maxLeases:                64
interactionPolicy:        "fail-closed"
controllerExecutionRef:   "Host/host-system"
```

### Field reference

| Field | Type | Required | Validation rule | Default |
| --- | --- | --- | --- | --- |
| `tenantId` | string | Yes | `OpaqueAzureRef` charset: `^[A-Za-z0-9._-]+$`; max 256 chars; rejects `=`, `+`, whitespace, `{}`, URI-scheme prefixes | — |
| `authorityClass` | enum | Yes | Closed set: `public` \| `us-government` \| `china`; or an opaque effect-port alias declared by the injected consumer/runtime Provider; no endpoint URL, hostname, or secret byte; the injected `EntraCredentialClient` and core policy resolve actual endpoints/TLS from this class — no Provider DTO or config ever carries an endpoint | `"public"` |
| `maxLeases` | u32 | No | Range 1–256; upper bound is the hard `MAX_LOCAL_LEASES` constant enforced at runtime | `64` |
| `interactionPolicy` | enum | No | `fail-closed` (map `InteractionRequired` to `credential-provider-unavailable`; never prompt); `interaction-required` (reserved for future user-agent environments; currently treated identically to `fail-closed`) | `"fail-closed"` |
| `controllerExecutionRef` | ResourceRef | **Yes** | Must resolve to a `Host/<name>` in the same Zone; that Host must include `system` in `allowedDomains`; must not be a `Guest/<name>`; no secret shape; no fallback — operators must declare this field explicitly | — |

**`tenantId` is not a `ResourceRef`.** It is an opaque inline Azure tenant GUID
validated by the same `OpaqueAzureRef::parse` logic from v3 baseline
`d2b-realm-provider/src/credential.rs`. The field name `tenantId` explicitly
does not end in `Ref`; it must not be authored as `Provider/…` or any other
`<ResourceType>/<name>` pattern. The Nix eval-time assertion that all `*Ref`
fields follow `<ResourceType>/<name>` intentionally does not apply to `tenantId`.

**`authorityClass` is a closed enum, not an endpoint.** No Provider config,
sealed config projection, DTO, status, audit record, or OTEL span attribute
ever contains a raw hostname, URL, or TLS endpoint. The injected
`EntraCredentialClient` effect port and core policy resolve the actual authority
endpoints and TLS trust anchors from `authorityClass` internally. The
`public`, `us-government`, and `china` values map to the standard Microsoft
national cloud authority paths; effect-port aliases are opaque tokens registered
by the consumer/runtime Provider and resolved only inside the effect port
implementation.

**`controllerExecutionRef` is required.** There is no implicit Zone primary Host
fallback. `Zone.spec` is `{}` and does not declare a primary Host; the operator
must specify the Host explicitly.

### Nix authoring example

```nix
# d2b.artifacts entry (separate from resource spec)
d2b.artifacts.credential-entra-bin = {
  package = pkgs.d2b-provider-credential-entra;
  type    = "provider";
};

# Provider resource
d2b.zones.dev.resources.credential-entra = {
  type = "Provider";
  spec = {
    artifactId = "credential-entra-bin";   # references d2b.artifacts entry; type must be "provider"
    config = {
      tenantId                = "2f8e1c3a-1234-5678-9abc-def012345678";
      authorityClass          = "public";           # public | us-government | china
      maxLeases               = 64;
      interactionPolicy       = "fail-closed";
      controllerExecutionRef  = "Host/host-system"; # required; must be a Host/<name>
    };
  };
};

# Credential resource consuming this provider
d2b.zones.dev.resources.work-entra = {
  type = "Credential";
  metadata.labels."team" = "platform";
  spec = {
    providerRef    = "Provider/credential-entra";
    scope = {
      executionRef = "Guest/work-vm";
      domainFilter = "user";
      userRef      = "User/alice";
    };
    audience           = "azure-resource-manager";
    consumerRef        = "Provider/display-wayland";
    allowedOperations  = [ "acquire-token" "refresh-token" ];
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
```

---

## 3. ResourceTypes implemented and consumed

### Implements: `Credential`

The provider implements the full `Credential` ResourceType lifecycle for
Entra-bound credentials. Every `Credential` resource whose `spec.providerRef =
"Provider/credential-entra"` is owned by this controller.

#### Lifecycle phases

| Phase | Meaning |
| --- | --- |
| `Pending` | Controller is initializing or Provider process not yet Ready |
| `Ready` | `leaseState=Active`, `CredentialReady=True`, within-window |
| `Degraded` | Provider process unreachable or rotation failing; bounded retry in progress |
| `Failed` | Retry exhausted or unrecoverable error |
| `Terminating` | Deletion requested; `provider-revoke` finalizer running |
| `Deleted` | All finalizers removed; resource record gone from store |

#### Status conditions owned

| Condition type | Set by | Cleared by |
| --- | --- | --- |
| `CredentialReady` | `entra-controller` (via agent report) | `entra-controller` |
| `RotationDue` | `entra-controller` (via agent report) | `entra-controller` on successful rotation |
| `ProviderUnavailable` | `entra-controller` when agent reports `EntraClientState=InteractionRequired` or agent Process unreachable | `entra-controller` when agent recovers |
| `LeaseRevoked` | `entra-controller` when agent reports `leaseState=Revoked` | `entra-controller` on new lease acquisition |

#### Finalizers owned

| Finalizer ID | Owned by | Trigger |
| --- | --- | --- |
| `credential.d2b.io/provider-revoke` | `entra-controller` | `metadata.deletionRequestedAt != null` |

The `consumer-drain` finalizer is registered by the `consumerRef` Provider's
controller (e.g. `Provider/display-wayland`), not by `credential-entra`.

### Consumes: `Provider`, `Host`, `Guest`, `User`

The controller's watch selectors include:

- `Credential` (providerRefFilter: `Provider/credential-entra`)
- `Provider` (nameFilter: `credential-entra`) — own resource, for generation
  change detection
- `Host` and `Guest` — `scope.executionRef` dependency readiness
- `User` — `scope.userRef` dependency readiness
- `Process` (ownerRefFilter: controller-created agent Processes) — agent readiness
  and health

---

## 4. Controllers, services, workers, and binaries

`credential-entra` uses two separate Process components:

- **`entra-controller`** — one per Zone, system-domain on the Zone host, holds
  no credential material, no token bytes, and no `EntraCredentialClient`
  reference. Owns the reconcile loop: watches Credential resources, creates and
  deletes `entra-agent` Process resources, writes Credential status from agent
  reports, manages the `provider-revoke` finalizer.
- **`entra-agent`** — one per active Credential resource, placed exactly at
  `scope.executionRef / scope.domainFilter / scope.userRef`. This is the only
  process that constructs `EntraCredentialClient`, holds token material in
  memory, calls the Entra identity platform over HTTPS, serves `d2b.credential.v3`
  service methods to the authorized consumer, and establishes the end-to-end
  Noise KK delivery session. The agent Process is controller-created with
  `metadata.ownerRef = Credential/<name>`.

### Controller descriptor (`entra-controller`)

```yaml
providerId:             Provider/credential-entra
controllerType:         Credential
resourceTypes:          [Credential]
watchSelectors:
  - resourceType: Credential
    providerRefFilter: Provider/credential-entra
  - resourceType: Provider
    nameFilter: credential-entra
  - resourceType: Host
    relationship: scope.executionRef
  - resourceType: Guest
    relationship: scope.executionRef
  - resourceType: User
    relationship: scope.userRef
  - resourceType: Process
    ownerRefFilter: controller-created-agent
dependencySelectors:
  - resourceType: Provider
    relationship: providerRef
  - resourceType: Host
    relationship: scope.executionRef
  - resourceType: Guest
    relationship: scope.executionRef
  - resourceType: User
    relationship: scope.userRef
ownerChildTriggers:     [owned-resource-changed]
reconcileConcurrency:   8
maxPendingResources:    256
finalizers:             [credential.d2b.io/provider-revoke]
observeInterval:        30s
```

### Process components table

| Component ID | Type | Domain | Binary | Cardinality |
| --- | --- | --- | --- | --- |
| `entra-controller` | controller | system (Zone host) | `d2b-provider-credential-entra-controller` | One per Zone |
| `entra-agent` | service | user or system per `scope.domainFilter` | `d2b-provider-credential-entra-agent` | One per active Credential resource |

### Canonical Process template: `entra-controller`

The controller is a single Zone-wide system-domain Process. It holds no secret
material and makes no network calls to Entra. Core instantiates it once when
the Provider resource becomes Ready, placing it on the Host resolved from
`spec.config.controllerExecutionRef`.

```yaml
# Process resource template for entra-controller.
# Instantiated once per Zone by Provider/credential-entra on Provider Ready.
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: "<zone-id>-credential-entra-ctrl"   # derived from Zone ID at Provider install
  zone: "<zone-name>"
  ownerRef: Provider/credential-entra        # owning Provider; resolves template via component descriptor
spec:
  template: entra-controller-main            # plain ID mapped to executable by Provider descriptor
  processClass: controller
  providerRef: Provider/system-minijail
  executionRef: "<spec.config.controllerExecutionRef>"  # Host/<name>; resolved from Provider config
  domain: system
  userRef: null
  credentialRefs: []
  mounts:
    - volumeRef: "Volume/credential-entra--controller--ctrl-state--<host-short>"
      view: main
      mountPath: /state
      access: read-only
      required: true
  sandbox:
    pids:
      limit: 32
    fds:
      limit: 256
  networkUsage:
    allowEgress: false    # controller makes no outbound calls; no ambient network claim
  endpoints:
    - name: bus-registration
      transport: unix
      purpose: controller-registration
  readiness:
    class: provider-defined
```

### Canonical Process template: `entra-agent` (user-agent)

The controller creates one user-agent Process per Credential resource whose
`scope.domainFilter = "user"`. User-domain agents use `Provider/system-systemd`
because the system-systemd user supervisor handles authenticated transient user
scopes. The Process carries `metadata.ownerRef = Credential/<name>`; owner
cascade handles deletion when the Credential resource is deleted.

The agent itself declares `networkUsage.allowEgress = false`. MSAL calls to
the Entra identity platform are proxied through the injected async
`EntraCredentialClient` / effect port provided by the co-located consumer or
runtime Provider; the agent binary never opens an ambient HTTPS connection
directly.

```yaml
# User-agent Process resource template.
# Controller instantiates one per Credential with scope.domainFilter=user.
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: "<zone-id>-entra-agent-<credential-name>"   # controller-assigned; derived from Credential name
  zone: "<zone-name>"
  ownerRef: "Credential/<credential-name>"           # set by controller; owner cascade deletes on Credential removal
spec:
  template: entra-agent-main              # plain ID; resolved by Provider/credential-entra via ownerRef
  processClass: service
  providerRef: Provider/system-systemd    # user-domain agents run under system-systemd
  executionRef: "<Credential.spec.scope.executionRef>"  # Host/<name> or Guest/<name>
  domain: user
  userRef: "<Credential.spec.scope.userRef>"            # User/<name>
  credentialRefs: []                      # agent does not consume a Credential resource directly
  mounts:
    - volumeRef: "Volume/credential-entra--agent--agent-state--<credential-name-short>"
      view: main
      mountPath: /state
      access: read-only
      required: true
  sandbox:
    namespaceClasses: [mount, pid, ipc, uts, network]
    capabilityClasses: []
    seccompClass: strict
    startRoot: false
    noNewPrivileges: true
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    memory:
      limit: "128Mi"    # Noise session state + token material in process-local memory
    pids:
      limit: 32
    fds:
      limit: 64
  networkUsage:
    allowEgress: false  # no direct MSAL network claim; Entra calls via injected client/effect port
  endpoints:
    - name: credential-service
      transport: unix
      purpose: d2b.credential.v3
  readiness:
    class: provider-defined
```

### Canonical Process template: `entra-agent` (guest-agent, system-domain)

For Credentials with `scope.domainFilter = "system"` under a Guest, the
controller creates a system-domain agent using `Provider/system-minijail`,
which runs the agent directly in its declared cgroup leaf via broker
`clone3(CLONE_PIDFD | CLONE_INTO_CGROUP)`. All other sandbox, budget, and
network invariants are identical to the user-agent template.

```yaml
# Guest-agent system-domain Process resource template.
# Controller instantiates one per Credential with scope.domainFilter=system under a Guest.
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: "<zone-id>-entra-agent-<credential-name>"
  zone: "<zone-name>"
  ownerRef: "Credential/<credential-name>"
spec:
  template: entra-agent-main
  processClass: service
  providerRef: Provider/system-minijail   # system-domain guest agents use system-minijail
  executionRef: "<Credential.spec.scope.executionRef>"  # Guest/<name> only; host-system rejected
  domain: system
  userRef: null
  credentialRefs: []
  mounts:
    - volumeRef: "Volume/credential-entra--agent--agent-state--<credential-name-short>"
      view: main
      mountPath: /state
      access: read-only
      required: true
  sandbox:
      limit: 32
    fds:
      limit: 64
  networkUsage:
    allowEgress: false  # no direct MSAL network claim; Entra calls via injected client/effect port
  endpoints:
    - name: credential-service
      transport: unix
      purpose: d2b.credential.v3
  readiness:
    class: provider-defined
```

#### Binary: `d2b-provider-credential-entra-controller`

| Field | Value |
| --- | --- |
| Path in crate | `src/controller_main.rs` |
| Role | Central Zone-wide controller; owns reconcile loop; no token material |
| Domain | system only |
| Placement | System-domain Process under `Host/<spec.config.controllerExecutionRef>` |
| Startup | Reads sealed Provider config projection; verifies `tenantId` with `OpaqueAzureRef::parse`; validates `authorityClass` is a known enum value or registered effect-port alias; validates `controllerExecutionRef` resolves a ready system-domain Host; registers controller descriptor with d2b-bus; signals readiness. Does NOT construct `EntraCredentialClient` and never holds or logs an authority endpoint. |
| Shutdown | Drains in-flight reconcile work to bounded deadline; exits cleanly |

#### Binary: `d2b-provider-credential-entra-agent`

| Field | Value |
| --- | --- |
| Path in crate | `src/agent_main.rs` |
| Role | Per-Credential co-located service; constructs `EntraCredentialClient` from injected effect port; serves `d2b.credential.v3`; delivers tokens/signatures via Noise KK |
| Domain | user (system-systemd) or system-on-Guest (system-minijail) per `scope.domainFilter` |
| Placement | User-domain: `Host/<name>` or `Guest/<name>`. System-domain: `Guest/<name>` only; `Host/<name>` with `domain=system` is rejected |
| Startup | Receives sealed config projection (tenantId, authorityClass, Credential ref, consumerRef, effect-port FD index) from controller via LaunchTicket; opens the injected effect-port FD; constructs `EntraCredentialClient` using `authorityClass` to select authority endpoint/TLS internally; registers `d2b.credential.v3` service on d2b-bus bound to exact Credential ref; signals readiness. No ambient HTTPS socket opened; no raw authority endpoint URL is held, logged, or audited by the agent. |
| Shutdown | Drains in-flight RPC calls and active KK delivery sessions to bounded deadline; applies `revocation.onProviderGeneration` revocation if Provider generation is changing; zeroizes all key material and token buffers; exits cleanly |

#### pidfd, wait, and reap

Both the controller and agent Processes are launched, supervised, and reaped
via the ProviderSupervisor LaunchTicket mechanism defined in
`ADR-046-components-processes-and-sandbox`. Neither process is a PID1 unit or
owns a `.socket` or `.service` activation file. The Zone runtime holds the
mandatory pidfd for each and reaps on exit; neither binary forks, supervises
children, or calls `setsid`.

#### ProviderStateSet

A **ProviderStateSet** is a query-time logical grouping — the set of all Volume
resources in the Zone whose `metadata.ownerRef` resolves to
`Provider/credential-entra`. It is not a ResourceType or a stored artifact.

Every semantic component of `credential-entra` receives one private ordinary
Volume per declared state namespace, mandatory even while the payload schema
is empty. These are ordinary Volume resources following the exact Volume +
provider-state extension schema from `ADR-046-provider-state`. No Volume is
shared across components. Each component receives only its local view dirfd;
no cross-component dirfd is ever granted. Core ProviderDeployment creates
declared component state Volumes before starting component Processes and deletes
them after Processes have stopped. The `credential-entra` controller does not
create, delete, or watch its own state Volumes; `Provider/volume-local` is the
sole reconciler for these Volumes. Each component only mounts and consumes its
declared view.

**Controller Volume** — `credential-entra--controller--ctrl-state--<host-short>`,
one per Zone, created at Provider install:

```yaml
apiVersion: resources.d2b.io/v3
type: Volume
metadata:
  name: credential-entra--controller--ctrl-state--<host-short>
  zone: <zone>
  ownerRef: Provider/credential-entra
spec:
  kind: state
  providerRef: Provider/volume-local
  persistenceClass: persistent
  sensitivityClass: private
  stateSchema:
    schemaId: io.d2b.credential-entra/controller/ctrl-state
    schemaVersion: "1.0"
    schemaDigest: sha256:<hex>
    migrationPolicy: none
  quotaBytes: 65536
  maxBytes: 65536
  maxInodes: 256
  sourcePolicyId: credential-entra-state-v1
  sealingCredentialRef: null
  source:
    executionRef: Host/<controllerExecutionRef>
    settings: {}
  layout:
    - path: state
      type: directory
      ownerRef: User/credential-entra-system
      groupRef: User/credential-entra-system
      mode: "0700"
      sensitivity: private
      createPolicy: create-if-never-provisioned
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
      noFollow: true
  views:
    main:
      path: state
      rights: [read, traverse]
  identityMarker:
    class: broker-maintained
    markerRoot: provider-state-markers
  snapshotPolicy: null
  retentionPolicy: null
```

`User/credential-entra-system` is a Nix-preprovisioned principal. No
`ComponentPrincipal` ResourceRef is used.

**Agent Volumes** — controller, user-agent, and guest-agent each own a separate
full Volume; no two components share a Volume. Every agent Volume carries its
own `sourcePolicyId`, `quotaBytes`/`maxBytes`/`maxInodes` base quota, and
broker-maintained `identityMarker`.

**User-agent Volume** — `credential-entra--agent--agent-state--<credential-name-short>`:
`providerRef: Provider/volume-local`; `source.executionRef: <scope.executionRef>`
(a Host); `layout.ownerRef: User/<scope.userRef-short>` (Nix-preprovisioned);
`sensitivityClass: private`; no cross-uid Volume access.

**Guest-agent Volume** (system-domain, Guest) —
`credential-entra--agent--agent-state--<credential-name-short>`: one Volume with
`providerRef: Provider/volume-local`; includes an `attachments` entry with
`transport: virtiofs` and `executionRef: <scope.executionRef>` (Guest/<name>);
`Provider/volume-local` creates the virtiofs Export child resource as part of
the Volume lifecycle. There is no separate attachment Volume.

**Empty payload schema invariant.** Both controller and agent Volumes carry
`kind: state`, `persistenceClass: persistent`, and minimal nonzero base quota
fields — `quotaBytes: 65536`, `maxBytes: 65536`, `maxInodes: 256` — sufficient
for the identity marker file and directory inode. `quotaBytes: 0` is never
used; these Volumes are durable and participate fully in the upgrade, destroy,
and reset lifecycle. The stateSchema declares `migrationPolicy: none`; because
the payload schema is empty, no migration EphemeralProcess worker is ever
dispatched for these Volumes. The schema document (signed into the component
descriptor) contains no payload field declarations. Token bytes, lease bytes,
signature bytes, client secret material, authority endpoint state, hostnames,
and scope values never appear in any Volume field, stateSchema entry, audit
record, OTEL attribute, or log field — not even in redacted form. Volumes
survive component and Provider restart; restart does not destroy or recreate
the Volume.

---

## 5. Consumer co-location and `EntraCredentialOwner`

`credential-entra` enforces `EntraCredentialOwner::ExactConsumer`: **the
`entra-agent` process is constructed for exactly one co-located consumer**
identified by `spec.consumerRef` in the Credential resource. No other Provider,
component, or process may acquire a lease, even if RBAC otherwise permits.

**Co-location is achieved through the agent's Process placement**, not the
controller's placement. The `entra-agent` is launched at exactly
`scope.executionRef / scope.domainFilter / scope.userRef` — the same execution
context and domain as the `consumerRef` Provider process. The central
`entra-controller` runs Zone-wide on the Zone host and is never co-located with
the consumer. Cross-execution-context acquisition is rejected at d2b-bus RBAC
enforcement before any `entra-agent` service method is dispatched.

If `spec.consumerRef` is null, the Credential resource fails the Nix eval-time
assertion for `credential-entra` (entra credentials require a declared consumer;
open access is not supported). This is stricter than the base Credential spec
which allows null `consumerRef`.

Consumer co-location is validated at three points:

1. **Nix eval time**: `spec.consumerRef` is required and must resolve a declared
   `Provider/<name>` in the same Zone with a compatible placement binding.
2. **Controller at agent Process create time**: when creating the `entra-agent`
   Process resource, the controller verifies that `consumerRef` resolves a Ready
   Provider in the same Zone with a placement binding matching `scope.executionRef
   / scope.domainFilter / scope.userRef`.
3. **d2b-bus RBAC enforcement**: the authenticated consumer Provider subject is
   compared to `spec.consumerRef`; a mismatch returns `credential-consumer-mismatch`
   before any `EntraCredentialClient` method is invoked on the agent.

---

## 6. Injected `EntraCredentialClient` — no ambient credential chain

The `EntraCredentialClient` trait is the sole interface between the **`entra-agent`
process** and the external Entra identity platform. The central `entra-controller`
does **not** construct or hold an `EntraCredentialClient` reference; it contains
no token material and makes no calls to Entra.

**The agent does not hold a direct network capability.** Its Process template
declares `networkUsage.allowEgress = false`. Instead, the agent receives an
**injected async client / effect port** from the co-located consumer or runtime
Provider via the ProviderSupervisor LaunchTicket inherited FD table. This effect
port implements the `EntraCredentialClient` trait and proxies all calls to the
Entra identity platform through the consumer or runtime Provider's network
interface — the agent binary never opens an ambient HTTPS socket.

There is no ambient credential chain, no Azure SDK `DefaultAzureCredential` /
`ChainedTokenCredential` fallback, no environment variable credential source
(`AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_TENANT_ID`), no developer-tool
credential path (`az` CLI, VS Code, browser flow), and no managed-identity IMDS
endpoint within this provider. Every token acquisition path goes through the
injected client effect port.

### `EntraCredentialClient` trait (from main `a1cc0b2d`)

```rust
pub trait EntraCredentialClient: Send + Sync + 'static {
    async fn issue_lease(
        &self,
        request: &EntraLeaseRequest,
    ) -> Result<EntraLeaseGrant, EntraClientError>;

    async fn refresh_lease(
        &self,
        lease_ref: &EntraLeaseRef,
    ) -> Result<EntraLeaseRenewal, EntraClientError>;

    async fn revoke_lease(
        &self,
        lease_ref: &EntraLeaseRef,
    ) -> Result<EntraLeaseRevocation, EntraClientError>;

    async fn inspect_lease(
        &self,
        lease_ref: &EntraLeaseRef,
    ) -> Result<EntraLeaseInspection, EntraClientError>;
}
```

The trait is retained unchanged from main `a1cc0b2d`; no method signature
modification is required for v3.

### Error mapping

| `EntraClientError` variant | d2b stable error code | Rationale |
| --- | --- | --- |
| `InteractionRequired` | `credential-provider-unavailable` | Transient state requiring user interaction; not a policy denial |
| `Unauthorized` | `credential-operation-denied` | Policy or scope rejection from Entra |
| `ServiceUnavailable` | `credential-provider-unavailable` | Transient network or service failure |
| `InvalidRequest` | `credential-invariant-failure` | Request violated invariant; programming error |
| `LeaseLimitExceeded` | `credential-queue-pressure` | Backpressure; consumer should retry after delay |
| `LeaseNotFound` | `credential-not-found` | Lease handle no longer valid in provider state |

`EntraClientError::InteractionRequired` MUST map to `credential-provider-unavailable`,
not `credential-operation-denied`. This is a deliberate design choice: interaction
required is a transient unavailability, and the `EntraClientState` transitions
from `InteractionRequired` back to `Ready` without operator intervention once
the external condition resolves.

### `EntraClientState`

```
Ready | InteractionRequired
```

When `EntraClientState=InteractionRequired`:

- `AcquireToken` returns `credential-provider-unavailable`.
- `RefreshToken` returns `credential-provider-unavailable`.
- Controller sets `ProviderUnavailable=True`, `leaseState=Unknown`.
- Scheduled observe interval continues; state recovers automatically.

### Lease types (retained from main `a1cc0b2d`)

All lease types implement `Debug` via a hand-written redacted impl that emits
only the type name and a fixed placeholder. Derived `Debug` is forbidden.

| Type | Contents | Secret bytes |
| --- | --- | --- |
| `EntraLeaseRequest` | `audience: OpaqueAzureRef`; `idempotency_key: [u8; 32]`; `requested_expiry_unix_ms: u64`; `operation_id: OperationId` | No |
| `EntraLeaseRef` | Opaque bounded handle (non-secret newtype); `rotationGeneration: u64` | No |
| `EntraLeaseGrant` | `lease_ref: EntraLeaseRef`; `source_version: OpaqueSourceVersion`; `expires_at_unix_ms: u64`; `issued_at_unix_ms: u64`; `token_bytes: Zeroizing<Vec<u8>>` | Yes — `token_bytes` only |
| `EntraLeaseRenewal` | `lease_ref: EntraLeaseRef`; `new_expires_at_unix_ms: u64`; `token_bytes: Zeroizing<Vec<u8>>` | Yes — `token_bytes` only |
| `EntraLeaseRevocation` | `lease_ref: EntraLeaseRef`; `revoked_at_unix_ms: u64`; `result: RevocationResult` | No |
| `EntraLeaseInspection` | `lease_ref: EntraLeaseRef`; `lease_state: CredentialLeaseState`; `expires_at_unix_ms: u64`; `source_version: OpaqueSourceVersion` | No |

`token_bytes` fields are `Zeroizing<Vec<u8>>` and are dropped immediately after
the Noise KK delivery record is encrypted and confirmed sent. The plaintext
buffer is zeroed before the `Zeroizing` wrapper is dropped. Token bytes never
appear in outer RPC DTOs, status, resource store, audit records, OTEL spans, or
log lines.

---

## 7. Credential methods and lease lifecycle

### `d2b.credential.v3` method mapping

| d2b method | `EntraCredentialClient` call | Secret output? |
| --- | --- | --- |
| `AcquireToken` | `issue_lease(&EntraLeaseRequest)` → `EntraLeaseGrant` | Yes — token bytes via KK delivery session |
| `RefreshToken` | `refresh_lease(&EntraLeaseRef)` → `EntraLeaseRenewal` | Yes — refreshed token bytes via KK delivery session |
| `RevokeToken` | `revoke_lease(&EntraLeaseRef)` → `EntraLeaseRevocation` | No |
| `SignChallenge` | Constructs challenge-signing request; acquires token internally; signs using Entra | Yes — signature bytes via KK delivery session |
| `InspectMetadata` | `inspect_lease(&EntraLeaseRef)` → `EntraLeaseInspection` | No |
| `Status` | No external call; reads provider-internal state | No |

All non-secret outer DTOs carry only opaque lease identifiers, generation
counters, expiry timestamps, outcome codes, and closed enum values. No outer DTO
field ever contains a token prefix, token suffix, bearer string, key material,
or connection string. These methods are served by the **`entra-agent`** process;
the `entra-controller` never handles `d2b.credential.v3` service calls directly.

### Idempotency

Every `AcquireToken` and `RefreshToken` call carries an `idempotency_key`
derived from:

```text
HMAC-SHA256(
  key   = Credential.metadata.uid (bytes),
  input = rotationGeneration ∥ operation_class_byte
)
```

A duplicate `AcquireToken` with the same key and the same `rotationGeneration`
returns the existing `EntraLeaseGrant` without issuing a new token. The provider
tracks the idempotency key map in process-local memory; there is no persistent
idempotency store. After a provider restart, duplicate detection is not
guaranteed; the controller treats a post-restart duplicate as a fresh
acquisition.

### Lease cardinality

`maxLeases` (config field) caps the number of concurrent active
`EntraLeaseGrant` entries held by the provider process. Attempts to issue beyond
`maxLeases` return `credential-queue-pressure`. The hard constant
`MAX_LOCAL_LEASES = 256` is enforced at provider construction time; a config
value above 256 fails the Provider spec validation at install.

### State machine

```text
Absent
  |-- AcquireToken ---------> Active (CredentialReady=True)
  |-- RefreshToken ----------> credential-lease-expired (no active lease to refresh)

Active
  |-- proactive window ------> RotationDue (CredentialReady=True, RotationDue=True)
  |-- RefreshToken ----------> Active (same lease_ref, new expires_at)
  |-- [controller rotation] -> Active (new rotationGeneration, new lease_ref)
  |-- RevokeToken -----------> Revoked (leaseState=Revoked, CredentialReady=False)
  |-- expiry deadline -------> Expired (leaseState=Expired, CredentialReady=False)
  |-- InteractionRequired ---> Degraded(ProviderUnavailable=True); retry on recovery

RotationDue
  |-- rotation attempt ------> Active (rotationGeneration+1, new lease_ref)
  |-- rotation attempt fails -> RotationDue; bounded retry; degrade after final retry

Expired
  |-- AcquireToken ----------> Active (new lease; rotationGeneration+1)

Revoked
  |-- AcquireToken ----------> Active (if policy permits re-acquisition)
  |-- resource deletion -----> provider-revoke finalizer satisfied immediately

Degraded (InteractionRequired)
  |-- state recovers --------> previous state
  |-- retry exhausted -------> Failed
```

---

## 8. Raw token / signature only via end-to-end Noise KK delivery

Token bytes and signature bytes are delivered exclusively through a dedicated
end-to-end `Noise_KK_25519_ChaChaPoly_SHA256` ComponentSession established by
the **`entra-agent`** process. d2b-bus authorizes the route and forwards opaque
Noise-protected records without terminating, decrypting, or buffering the
delivery channel content. No intermediate process — including the
`entra-controller` — sees plaintext.

### Delivery session profile

- **Profile**: `Noise_KK_25519_ChaChaPoly_SHA256` (enrolled KK only; NN/NX/N
  patterns are rejected immediately and the session is closed and zeroized).
- **Credential Provider key**: registered at Provider installation; the bus
  holds only the public key.
- **Consumer Provider key**: extracted from the consumer Provider's signed
  component descriptor.

### Delivery session binding fields

Each delivery session binds:

| Field | Value |
| --- | --- |
| `credentialRef` | `Credential/<name>` |
| `credentialUID` | Credential resource UID (stable across spec updates) |
| `credentialGeneration` | Credential resource generation at delivery time |
| `consumerProviderRef` | `Provider/<name>` matching `spec.consumerRef` |
| `consumerComponentGeneration` | Consumer Provider component generation (from signed descriptor) |
| `audience` | `spec.audience` value (opaque; not echoed in logs or spans) |
| `operationClass` | Closed operation class (`acquire-token`, `refresh-token`, or `sign-challenge`) |
| `expiryUnixMs` | Absolute expiry; clipped to `spec.rotation.maxLeaseLifetimeMs` |
| `deadlineUnixMs` | Hard session close deadline ≤ `expiryUnixMs` |
| `routeDigest` | Digest of bus-authorized route parameters |
| `schemaVersion` | Fixed version of this binding contract |
| `maxTokenBytes` | Closed upper bound on token/signature bytes for this session |
| `transcriptDigest` | Noise transcript digest after handshake completion, before first record |

Both parties MUST verify the full binding before accepting records.

### Security requirements

1. **Enrolled keys only**: both static keys must be enrolled and verified at
   session initiation. Any anonymous-channel attempt is rejected immediately and
   the session is closed and zeroized.
2. **Replay-safe sequence**: each delivery session carries a monotonically
   increasing per-credential-UID sequence number. A replay at the same or lower
   sequence number is rejected.
3. **Bounded output size**: the sensitive output record MUST NOT exceed
   `maxTokenBytes`. Any oversized record is rejected; the channel is closed and
   zeroized immediately. Fragmentation is not permitted unless each fragment is
   explicitly bounded and the reassembled record does not exceed `maxTokenBytes`.
4. **Zeroizing buffers**: the delivery record's plaintext MUST be zeroed in
   memory immediately after the consumer extracts it. The provider MUST zero the
   plaintext source after encryption. All intermediate serialization/
   deserialization buffers are zeroizing types.
5. **Redacted Debug**: all credential-bearing Rust types (request, response,
   record wrapper, buffer) MUST implement `Debug` via a hand-written redacted
   impl emitting only the type name and a placeholder value. Derived `Debug` is
   forbidden for these types.
6. **No automatic replay on ambiguous outcome**: after any ambiguous delivery
   outcome (timeout, partial write, disconnect before confirmation), the provider
   MUST NOT automatically retry with the same record. The consumer must
   re-initiate via a new service method call, which establishes a new delivery
   session with a fresh sequence number.
7. **Immediate close and zeroize**: after the delivery record is confirmed
   received, the provider closes the delivery channel and zeroizes all session
   key material. The consumer similarly closes and zeroizes after extraction. The
   channel is not reused across multiple deliveries.

---

## 9. RBAC and security

### RBAC verbs

| Verb | Who | Enforced by |
| --- | --- | --- |
| `get` | Any authorized subject | d2b-bus resource API |
| `list` | Any authorized subject | d2b-bus resource API |
| `watch` | Any authorized subject | d2b-bus resource API |
| `create` | Deployer/system-core configuration controller | d2b-bus resource API |
| `update-spec` | Deployer/system-core configuration controller | d2b-bus resource API |
| `update-status` | `entra-controller` process only (exact registered generation) | d2b-bus resource API |
| `update-finalizers` | `entra-controller`; `consumerRef` controller | d2b-bus resource API |
| `delete` | Deployer/system-core configuration controller | d2b-bus resource API |
| `use-credential` | Consumer subject authorized by `consumerRef` and RBAC Role/RoleBinding | d2b-bus before service dispatch |

RBAC rule example for `use-credential`:

```yaml
rules:
  - resourceTypes: [Credential]
    verbs: [use-credential]
    resourceNames: [work-entra]
    zones: [dev]
    executionRefs: [Guest/work-vm]
    operationClasses: [acquire-token, refresh-token]
```

The effective operation set is the intersection of `spec.allowedOperations` and
the Role `operationClasses`. A consumer not matching `spec.consumerRef` receives
`credential-consumer-mismatch` before any `EntraCredentialClient` call.

### Zero-secret-bytes invariant

The zero-secret-bytes invariant is unconditional across all persistent and
observable surfaces for this Provider. Token bytes, key material, `tenantId`
literals, Azure scope values, bearer strings, and MSAL
cache entries:

- **do not appear** in `Credential.spec` or `Credential.status`;
- **do not appear** in the resource store row, redb WAL, or revision log;
- **do not appear** in d2b-bus routing DTOs or ResourceRef handles;
- **do not appear** in audit records;
- **do not appear** in OTEL span attributes, metric label values, or log lines;
- **do not appear** in outer `d2b.credential.v3` RPC response DTOs;
- **are delivered only** through the end-to-end Noise KK delivery session
  described in §8.

The `contains_sensitive_shape` guard (adapted from
`d2b-realm-provider/src/error.rs:contains_sensitive_shape`) is applied at:

- Nix eval time to all string fields in `Credential.spec` and
  `Provider.spec.config`;
- runtime to all error message strings before they leave the provider process;
- test time via the canary test suite (see §13).

### `host-system` placement rejection

The `credential-entra` Provider rejects `host-system` placement
(`scope.domainFilter=system` + `scope.executionRef=Host/<name>`) at Provider
install validation. Entra credentials require a configured consumer-agent
co-location (`user-agent` on Host or Guest, or `guest-agent` on Guest). A
system-domain host process without a co-located consumer agent cannot satisfy the
`EntraCredentialOwner::ExactConsumer` invariant.

### Process sandbox

Both the `entra-controller` and `entra-agent` Processes are compiled from the
canonical Process templates in §4 using semantic SandboxSpec fields. Both
templates share these sandbox invariants:

- `namespaceClasses: [mount, pid, ipc, uts, network]` — full isolation including
  private network namespace for both controller and agent;
- `capabilityClasses: []` — zero Linux capabilities granted;
- `seccompClass: strict` — minimal allow-list compiled by the selected Process
  Provider from the trusted bundle;
- `startRoot: false` — process does not start as in-namespace root;
- `noNewPrivileges: true` — `PR_SET_NO_NEW_PRIVS` before exec;
- `environmentClass: minimal` — only the fixed approved environment set;
- `readOnlyRoot: true` — rootfs mounted read-only.

The **controller** additionally:

- uses `Provider/system-minijail` — broker-compiled sandbox via `clone3(CLONE_PIDFD | CLONE_INTO_CGROUP)`;
- `networkUsage.allowEgress: false` — no outbound connections; communicates only via d2b-bus Unix socket.

The **agent** additionally:

- user-domain agent uses `Provider/system-systemd`; system-domain Guest agent uses `Provider/system-minijail`;
- `networkUsage.allowEgress: false` — no ambient MSAL network claim; all Entra calls go through
  the injected `EntraCredentialClient` effect port provided over the LaunchTicket inherited FD;
- no access to `/dev`, host Wayland sockets, PipeWire, or host-global broker FDs beyond
  the Provider supervisor ticket and the injected effect-port FD.

---

## 10. Status, errors, audit, and OTEL

### Status fields

The `Credential.status.credential` sub-object written by `entra-controller`:

| Field | Non-secret constraint |
| --- | --- |
| `leaseHandle` | Opaque bounded newtype (max 256 chars); never a token or partial token |
| `leaseState` | `Active \| Expired \| Revoked \| Unknown`; closed enum |
| `rotationGeneration` | Monotonic u64; bounded |
| `sourceVersion` | Opaque bounded newtype from `EntraLeaseInspection`; not a version string from any external system |
| `expiresAtUnixMs` | Unix milliseconds timestamp; 0 when absent |
| `issuedAtUnixMs` | Unix milliseconds timestamp of last successful acquisition |
| `lastRefreshedAt` | RFC 3339 UTC string |
| `lastRotatedAt` | RFC 3339 UTC string or null |
| `placementBinding` | `user-agent \| guest-agent`; closed enum |

No status field ever contains a tenant ID literal, audience literal, endpoint
URI, token prefix/suffix, or any byte from `EntraLeaseGrant.token_bytes`.

### Stable error codes

| Code | Condition |
| --- | --- |
| `credential-not-found` | Credential resource does not exist in this Zone |
| `credential-provider-unavailable` | `EntraClientState=InteractionRequired`; provider process unreachable; not Ready |
| `credential-lease-expired` | Lease is past its expiry deadline |
| `credential-lease-revoked` | Lease was explicitly revoked |
| `credential-operation-denied` | Operation class not in `allowedOperations`; RBAC denied; `Unauthorized` from Entra |
| `credential-consumer-mismatch` | Requesting subject does not match `spec.consumerRef` |
| `credential-placement-mismatch` | Request execution context/domain does not match `scope`; `host-system` attempted |
| `credential-rotation-failed` | Proactive rotation failed after bounded retries |
| `credential-invariant-failure` | Provider returned a response failing invariant checks |
| `credential-schema-invalid` | Spec field fails validation at create/update |
| `credential-queue-pressure` | `maxLeases` reached; retry after backpressure |

All error messages are bounded (max 240 UTF-8 chars), stripped of control
characters, and must not contain token bytes, URLs, UUIDs, provider diagnostics,
host paths, or connection string shapes. Error messages pass `contains_sensitive_shape`
before being returned.

### Audit events

| Event | Retained fields |
| --- | --- |
| Credential create/update/delete | Zone, subject digest (`sha256:<hex>`), ResourceRef, verb, revision result, authorization decision |
| `AcquireToken` | Zone, subject digest, `Credential/<name>`, operation class, `rotationGeneration`, outcome code, idempotency key digest |
| `RefreshToken` | Zone, subject digest, `Credential/<name>`, operation class, `rotationGeneration`, outcome code, idempotency key digest |
| `RevokeToken` | Zone, subject digest, `Credential/<name>`, operation class, `rotationGeneration`, revocation result code |
| `SignChallenge` | Zone, subject digest, `Credential/<name>`, operation class, outcome code (no signature bytes) |
| Rotation | Zone, `Credential/<name>`, trigger reason, old `rotationGeneration`, new `rotationGeneration`, outcome code |
| Provider generation change revocation | Zone, `Credential/<name>`, policy applied, outcome code |
| `InteractionRequired` state transition | Zone, `Credential/<name>`, direction (entered/recovered), outcome code |

Excluded from all audit records: token bytes, key material, passwords, bearer
strings, `tenantId` literals, `audience` literals, tenant/subscription/client
IDs, endpoint URIs, Noise/session key material, and provider-internal
diagnostics.

### OTEL spans

Span names follow `d2b.credential.<operation>`:

| Span name | Emitted on |
| --- | --- |
| `d2b.credential.acquire_token` | `AcquireToken` service call |
| `d2b.credential.refresh_token` | `RefreshToken` service call |
| `d2b.credential.revoke_token` | `RevokeToken` service call |
| `d2b.credential.sign_challenge` | `SignChallenge` service call |
| `d2b.credential.inspect_metadata` | `InspectMetadata` service call |
| `d2b.credential.reconcile` | Controller reconcile handler |
| `d2b.credential.rotation` | Rotation cycle |

Required span attributes (closed set):

| Attribute | Value |
| --- | --- |
| `d2b.zone` | Zone name |
| `d2b.credential.name` | Credential resource name |
| `d2b.credential.provider` | `credential-entra` |
| `d2b.credential.operation_class` | Closed enum string |
| `d2b.credential.placement_binding` | `user-agent` or `guest-agent` |
| `d2b.credential.outcome` | Stable closed outcome code |
| `d2b.credential.rotation_generation` | Numeric rotation generation |

Forbidden from spans and attributes: token bytes, `audience` literals, `tenantId`
literals, provider diagnostics, host paths, Azure resource IDs,
tenant/subscription IDs, endpoint URIs, correlation IDs that embed secret shapes,
and any value passing `contains_sensitive_shape`.

### OTEL metrics

| Metric | Type | Labels |
| --- | --- | --- |
| `d2b_credential_operations_total` | Counter | `provider=credential-entra`, `operation_class`, `placement_binding`, `outcome` |
| `d2b_credential_lease_expiry_seconds` | Gauge | `provider=credential-entra`, `credential_name`, `placement_binding` |
| `d2b_credential_rotation_total` | Counter | `provider=credential-entra`, `policy`, `outcome` |
| `d2b_credential_provider_health` | Gauge (0/1) | `provider=credential-entra` |
| `d2b_credential_active_leases` | Gauge | `provider=credential-entra`, `placement_binding` |

`credential_name` appears only in `d2b_credential_lease_expiry_seconds` where
per-resource precision is required. It is omitted from high-cardinality counters.
Label cardinality is bounded; no label ever encodes secret bytes or dynamic
identifiers beyond the resource name.

---

## 11. Nix configuration

### Eval-time assertions specific to `credential-entra`

In addition to the base Credential eval-time assertions defined in
`ADR-046-resources-credential §Eval-time assertions`, the following
`credential-entra`-specific assertions apply to every `Credential` resource
whose `spec.providerRef = "Provider/credential-entra"`:

1. **`consumerRef` is required**: `spec.consumerRef` must be non-null and must
   resolve a declared `Provider/<name>` in the same Zone. An absent
   `consumerRef` fails the Nix build with:
   ```
   error: credential-entra requires spec.consumerRef; open-access Credentials
   are not supported for this provider.
   ```

2. **`host-system` placement rejected**: if `spec.scope.executionRef` resolves
   to a `Host/<name>` and `spec.scope.domainFilter = "system"`, the eval fails
   with:
   ```
   error: credential-entra does not support host-system placement. Use
   domainFilter = "user" or place the Credential under a Guest.
   ```

3. **`audience` charset**: `spec.audience` must match `^[A-Za-z0-9._:/@-]+$`
   (max 256 chars). Values containing `=`, `+`, `{`, `}`, whitespace,
   URL-encoded percent sequences, or any byte that passes `contains_sensitive_shape`
   fail the eval.

4. **`tenantId` in Provider config**: `Provider.spec.config.tenantId` must
   match `^[A-Za-z0-9._-]+$` (max 256 chars). The field must not end in `Ref`,
   must not be a `<ResourceType>/<name>` pattern, and must not contain `://`,
   `/`, query-string characters, or any secret-shaped value.

5. **`authorityClass` is a closed enum**: `Provider.spec.config.authorityClass`
   must be one of `public`, `us-government`, or `china`, or an opaque effect-port
   alias declared by the consumer/runtime Provider. Reject any value containing
   `://`, `.`, port separators, path components, query strings, or any
   hostname-shaped bytes. Effect-port aliases are validated as opaque identifiers
   (`^[a-z][a-z0-9-]*$`); they are resolved only inside the effect port
   implementation and never expanded into an endpoint URL in any config, sealed
   projection, DTO, status field, or audit record.

6. **`allowedOperations` subset**: all five operation classes are supported;
   `spec.allowedOperations` must be a non-empty subset of
   `{acquire-token, refresh-token, revoke-token, sign-challenge, inspect-metadata}`.

7. **`scope.domainFilter` matches Provider capability**: `credential-entra`
   declares `credentialDomains = [user, system]` (system = guest-agent only).
   A `domainFilter = "system"` entry requires `scope.executionRef` to resolve a
   Guest (not a Host); this is enforced jointly by assertions 2 and 7.

8. **`controllerExecutionRef` in Provider config**: `Provider.spec.config.controllerExecutionRef`
   is **required**. It must match `^Host/[a-z][a-z0-9-]*$` and must resolve to a
   declared `Host/<name>` in the same Zone. It must not be a `Guest/<name>`
   reference. `Zone.spec` is `{}` — there is no Zone primary Host concept; Nix
   eval fails with a hard error if this field is absent:
   `error: credential-entra requires spec.config.controllerExecutionRef; specify
   the Host/<name> where the controller will run.`

### Generated schema cross-check

The build-time schema cross-check (`make test-drift`) validates that:

- `docs/reference/schemas/v3/provider-credential-entra-config.json` (generated
  by `cargo xtask gen-schemas`) matches the committed schema in the repository.
- The `audience` charset rules in the Credential ResourceTypeSchema
  (`docs/reference/schemas/v3/credential.json`) and the Provider-specific schema
  agree on the `^[A-Za-z0-9._:/@-]+$` constraint.
- The `tenantId`, `authorityClass`, and `controllerExecutionRef` schemas correctly
  declare `OpaqueAzureRef`, closed-enum, and ResourceRef constraints in the
  provider-specific JSON schema.
- No schema field marks a `secretRef: true` field that is not a
  `Credential/<name>` reference.

---

## 12. Async reconcile

The `entra-controller` implements the `ADR-046-resource-reconciliation` async
loop model. Its role in this split architecture is to manage the **lifecycle of
`entra-agent` Processes** — it is not responsible for token acquisition directly.
The `entra-agent` process independently handles `EntraCredentialClient`
acquisition, observation, and revocation. The controller coordinates the two via
Process resource CRUD and inter-process status reporting.

### Reconcile handlers

#### `reconcile` — triggered by: Create, Spec update, dependency Ready

1. Validate resolved Provider dependencies: `Provider/credential-entra` Ready,
   `scope.executionRef` Ready, `scope.userRef` Ready (if applicable).
2. Validate `spec.consumerRef` resolves a declared Ready Provider in the same Zone
   with a placement binding matching `scope.executionRef / scope.domainFilter /
   scope.userRef`.
3. If no `entra-agent` Process exists for this Credential: submit a
   `ProviderSupervisor::LaunchTicket` to create an `entra-agent` Process resource
   with `metadata.ownerRef = Credential/<name>`, placement derived from
   `scope.executionRef / scope.domainFilter / scope.userRef`, and a sealed config
   projection carrying `tenantId`, `authorityClass`, `consumerRef`, `maxLeases`,
   `interactionPolicy`, audience, and the `idempotency_key`.
4. Wait for agent Process Ready readiness probe. If agent Process transitions to
   `ProcessFailed` within `reconcileTimeout`: set `ProviderUnavailable=True`.
5. Once agent Process is Ready: set `CredentialReady=True` via agent status report
   (see `observe` below).
6. If `spec` changed (providerRef, scope, audience, consumerRef): signal the existing
   agent to revoke under the old spec, delete the old agent Process, and create a
   new one under the updated spec. The old agent Process deletion is owner-cascade
   via `metadata.ownerRef`.

#### `observe` — triggered by: `observeInterval=30s` timer

1. Read the `entra-agent` Process status from the resource store.
2. Propagate `leaseState`, `expiresAtUnixMs`, `sourceVersion`, and condition
   updates from the agent Process status into the Credential status.
3. Evaluate proactive rotation window:
   - If `now + proactiveWindowMs >= expiresAtUnixMs` and
     `rotation.policy = "proactive"`: set `RotationDue=True` condition.
4. If `RotationDue=True`: send a rotation signal to the `entra-agent` via its
   declared control endpoint; the agent drives the actual `rotate_lease` call.
5. If agent Process is absent (unexpected deletion or crash): set
   `ProviderUnavailable=True`; re-trigger `reconcile` to recreate the agent.
6. If agent reports `LeaseNotFound`: re-trigger `reconcile` for re-acquire.

#### `finalize` — triggered by: `metadata.deletionRequestedAt != null`

1. Apply `revocation.onOwnerDelete`:
   - `immediate`: signal the `entra-agent` to call `revoke_lease`; wait for
     agent to confirm `Revoked` state (agent updates Process status) before
     proceeding. If agent Process unreachable or gone: mark `leaseState=Revoked`;
     write bounded audit record.
   - `drain-leases`: do not signal revocation; allow natural expiry; delete
     agent Process immediately (owner cascade handles it if not already gone).
2. **Controller removes `credential.d2b.io/provider-revoke` finalizer** by
   calling `UpdateFinalizers`. When this is the last finalizer on the resource,
   core proceeds with deletion.
3. **Core (automatic after zero finalizers)**: performs one atomic store
   transaction writing the event-only `Deleted` revision and removing the
   row/indexes. No `Deleted` row persists; the row is absent after the
   transaction commits.
4. **Audit subsystem (post-commit)**: after the transaction commits, appends
   `ResourceMutation{event="deleted"}` using dedup/exactly-once recovery keyed
   on the revision. This append is not part of the store transaction.

#### Agent startup — triggered by: agent Process startup (agent side)

The `entra-agent` process independently, upon startup:

1. Reads its sealed config projection from the ProviderSupervisor inherited FD
   (tenantId, authorityClass, Credential ref, consumerRef, effect-port FD index).
2. Opens the injected effect-port FD and constructs an `EntraCredentialClient`
   implementation over it, using `authorityClass` to select authority
   endpoint/TLS internally. This effect port is provided by the co-located
   consumer or runtime Provider; no ambient HTTPS socket is opened and no raw
   authority endpoint URL is held in the agent's memory or logs.
3. Calls `issue_lease` via the injected client with the `idempotency_key`;
   stores `leaseHandle`, `expiresAtUnixMs`, `issuedAtUnixMs`, `rotationGeneration=1`.
4. Opens the `d2b.credential.v3` service listener; updates Process status to Ready.
5. Begins serving `AcquireLease`, `RenewLease`, `RevokeLease`, `InspectLease`
   method calls from the authorized `consumerRef` only.

On `EntraClientError::InteractionRequired`: agent sets Process status
`ProviderUnavailable=True`, `leaseState=Unknown`; controller observes and
requeuees at `observeInterval`.

#### Provider generation change

When the `Provider/credential-entra` generation changes:

- `immediate` policy: controller signals the agent to call `revoke_lease` against
  the old provider client before the generation transitions. If unreachable:
  marks `leaseState=Revoked`; writes bounded audit record.
- `drain-leases` policy: active leases expire naturally; status remains `Active`
  until expiry.

#### Retry and backpressure

| Outcome | Requeue behavior |
| --- | --- |
| `InteractionRequired` → `credential-provider-unavailable` | Requeue at `observeInterval` |
| `ServiceUnavailable` | Exponential backoff; max 10 retries; `phase=Degraded` on final retry |
| `LeaseLimitExceeded` → `credential-queue-pressure` | Requeue at 2×`observeInterval` |
| Rotation failure | Bounded retries; `credential-rotation-failed` outcome; `phase=Failed` on exhaustion |
| Idempotent re-acquire (same key) | No new token issued; existing grant returned |
| Agent Process creation failure | Requeue; controller logs `agent-launch-failed`; `ProviderUnavailable=True` |

Reconcile concurrency is 8; max pending resources per controller is 256.

---

## 13. Redaction

### Redacted types

All types that transitively contain token bytes, MSAL cache entries, session key
material, or signature bytes implement `Debug` via a hand-written redacted impl.
No derived `Debug` on these types. The impl emits:

```rust
impl fmt::Debug for EntraLeaseGrant {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EntraLeaseGrant").finish_non_exhaustive()
    }
}
```

The same pattern applies to: `EntraLeaseRequest`, `EntraLeaseRenewal`,
`DeliveryRecord`, `ZeroizingTokenBuffer`, and any future type that wraps token
bytes.

### `contains_sensitive_shape` guard

The function (adapted from `d2b-realm-provider/src/error.rs:contains_sensitive_shape`)
is applied to every outbound string field before it leaves the **agent** process:

- Error message strings returned to callers.
- Status field string values written to the resource store.
- Audit record field values.
- Metric label values.
- Span attribute values.

The guard rejects strings that match patterns for JWTs (`.` split, base64url
header), bearer tokens, hex-encoded key material (32+ contiguous hex chars),
UUIDs in bearer context, MSAL cache JSON shapes, `BEGIN …` PEM blocks, and
high-entropy strings (Shannon entropy above threshold in long fields).

---

## 14. Crate layout requirements

The workspace policy gate (`ADR046-provider-002`) enforces that the following
paths all exist in `packages/d2b-provider-credential-entra/`:

```text
packages/d2b-provider-credential-entra/
  src/
    lib.rs              # EntraCredentialClient trait, lease types, provider core
    controller.rs       # reconcile/observe/finalize handlers; agent Process management
    service.rs          # d2b.credential.v3 service dispatch (agent side)
    controller_main.rs  # controller binary entry point; sealed config read; registered descriptor
    agent_main.rs       # agent binary entry point; sealed config read; client construction
    audit.rs            # audit record emission
    telemetry.rs        # OTEL span/metric emission
  tests/
    lifecycle.rs    # acquire/refresh/revoke/inspect end-to-end with FakeEntraClient
    conformance.rs  # all provider conformance arms pass
    faults.rs       # interaction-required→unavailable; generation-mismatch; colocated-consumer rejection
    canary.rs       # credential_canary and endpoint_canary absent from all responses and delivery records
    delivery.rs     # delivery-session binding, zeroizing, replay-safe sequence
    placement.rs    # host-system placement rejected; user-agent and guest-agent accepted
  integration/
    container-service.sh    # container-backed Provider service start/stop/drain
    guest-placement.nix     # user-domain and system-domain agent Process on Guest (runNixOSTest)
    cleanup-rollback.sh     # Nix-generation removal triggers async Delete and provider-revoke finalizer
  README.md         # all §Provider README required sections (see §15 below)
```

The workspace policy rejects the crate if any of `src/`, `tests/`,
`integration/`, or `README.md` is absent.

---

## 15. Provider `README.md` required sections

The `packages/d2b-provider-credential-entra/README.md` MUST contain these
sections in order:

1. **Provider identity** — `Provider/credential-entra`; implements `Credential`;
   generation/versioning policy; Zone placement constraints.
2. **Config schema** — `spec.config` fields (tenantId, authorityClass, maxLeases,
   interactionPolicy), types, defaults, constraints, and worked Nix example.
3. **ResourceTypes managed** — `Credential`: lifecycle phases, owned status
   conditions, owned finalizers.
4. **Controllers, services, workers, and binaries** — `entra-controller`
   (Zone-wide, Zone-host system-domain, binary `d2b-provider-credential-entra-ctrl`,
   secret-free, manages Credential lifecycle and agent Processes); `entra-agent`
   (one per Credential, co-located at `scope.executionRef/domain/userRef`, binary
   `d2b-provider-credential-entra-agent`, constructs `EntraCredentialClient`,
   serves `d2b.credential.v3`).
5. **Placement** — `user-agent` and `guest-agent` accepted; `host-system`
   rejected with `credential-placement-mismatch`.
6. **Dependencies and RBAC** — required Zone resources (`executionRef`,
   `consumerRef` required, `userRef`); RBAC verbs consumed; `ExactConsumer`
   constraint; cross-resource ordering.
7. **Security, state, and telemetry** — secret isolation model (injected client;
   no ambient chain; Noise KK delivery only); what is persisted (opaque handles
   only; no token bytes); audit events; OTEL spans/metrics; canary enforcement.
8. **Build, test, and integration commands**:
   - `cargo test -p d2b-provider-credential-entra` (unit + Cargo integration)
   - `cargo test -p d2b-provider-credential-entra --test canary` (canary only)
   - `bash packages/d2b-provider-credential-entra/integration/container-service.sh`
   - `nix build .#checks.x86_64-linux.guest-placement-credential-entra`
   - `bash packages/d2b-provider-credential-entra/integration/cleanup-rollback.sh`
9. **Standalone-repo usage** — flake input pattern; nixpkgs/toolkit
   `inputs.follows` boilerplate; compatibility constraints.

---

## 16. v3 current-code fit

| Item | Value |
| --- | --- |
| Current anchor | `d2b-realm-provider/src/credential.rs:AzureControlPlaneRef`, `OpaqueAzureRef` (implemented-and-reachable); `provider.rs:CredentialProvider` minimal status-only trait (implemented-and-reachable) |
| Evidence class | Opaque ref model is reachable; full Entra lease provider is `ADR-only` in v3 |
| Main reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-entra/src/lib.rs` (full implementation: `EntraCredentialClient` trait, `EntraLeaseRequest/Ref/Grant/Inspection/Renewal/Revocation`, `EntraCredentialProvider`, `EntraCredentialProviderFactory`, `EntraCredentialOwner::ExactConsumer`, `EntraClientState`, `EntraClientError` mapping); `src/tests.rs` (full test suite: `FakeEntraClient`, `credential_canary`/`endpoint_canary` enforcement, interaction-required and colocated-consumer tests, generation-mismatch tests) |
| Reuse action | copy and adapt |
| Required delta | v3 contract versions; Provider resource/descriptor; d2b-bus routing; v3 `PlacementBinding` enum (user-agent, guest-agent; reject host-system); validate `tenantId` config using `OpaqueAzureRef::parse` (current v3 source field is `AzureControlPlaneRef.tenant_id`; target field name is `tenantId`; not a Ref); retain `EntraCredentialClient` trait unchanged; map `EntraClientError::InteractionRequired` to `credential-provider-unavailable`; enforce `EntraCredentialOwner::ExactConsumer`; replace v2 `AgentPlacementBinding` with v3 `PlacementBinding`; replace v2 ProviderFactory/EndpointRole/Realm with v3 Provider resource descriptor |
| Excluded main assumptions | v2 `AgentPlacementBinding`; v2 `EndpointRole`/`Realm`/`RealmPath`; v2 `ProviderFactory`/`ProviderRegistryBuilder`; v2 component-session auth and prologue; v2 `d2b-contracts/src/v2_provider.rs` types |
| Behavior retained | `EntraCredentialClient` trait (unchanged); zero-secret-bytes invariant; `OpaqueAzureRef` charset/validation; `ExactConsumer` ownership model; `FakeEntraClient` test infrastructure; `credential_canary`/`endpoint_canary` test enforcement |
| Replacement/deletion | Old `CredentialProvider` trait in `d2b-realm-provider/src/provider.rs` removed only after all three v3 Credential Provider controllers reach full reconcile parity and their integration tests pass |

---

## 17. Implementation work item

### ADR046-credential-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-credential-004` |
| Dependency/owner | `ADR046-credential-001` (v3 contract types); `ADR046-credential-002` (d2b.credential.v3 service proto); `ADR046-reconcile-001` (controller toolkit); credential-entra owner |
| Current source | `d2b-realm-provider/src/credential.rs:AzureControlPlaneRef`, `OpaqueAzureRef` (v3 baseline; implemented-and-reachable) |
| Reuse source | main `a1cc0b2d`: `packages/d2b-provider-credential-entra/src/lib.rs` (full implementation); `src/tests.rs` (full test suite including `FakeEntraClient`, `credential_canary`/`endpoint_canary`, interaction-required, colocated-consumer, generation-mismatch tests) |
| Reuse action | copy and adapt |
| Destination | `packages/d2b-provider-credential-entra/src/{lib.rs, controller.rs, service.rs, controller_main.rs, agent_main.rs, audit.rs, telemetry.rs}`; `packages/d2b-provider-credential-entra/tests/{lifecycle.rs, conformance.rs, faults.rs, canary.rs, delivery.rs, placement.rs}`; `packages/d2b-provider-credential-entra/integration/{container-service.sh, guest-placement.nix, cleanup-rollback.sh}`; `packages/d2b-provider-credential-entra/README.md` |
| Detailed design | (1) Copy `EntraCredentialClient` trait, `EntraLeaseRequest/Ref/Grant/Inspection/Renewal/Revocation`, `EntraCredentialProvider`, `EntraCredentialOwner::ExactConsumer`, `EntraClientState`, `EntraClientError` from main `a1cc0b2d` without modification to trait signatures. (2) Replace v2 `AgentPlacementBinding` with v3 `PlacementBinding` enum (`user-agent \| guest-agent`); reject `host-system` at construction with `credential-placement-mismatch`. (3) Validate `tenantId` config field at startup using `OpaqueAzureRef::parse` from v3 `d2b-realm-provider/src/credential.rs`; note that the current v3 source field is `AzureControlPlaneRef.tenant_id`; the target config field name is `tenantId`, which is an opaque inline identifier, not a `<ResourceType>/<name>` ResourceRef, and must not end in `Ref`. (4) Validate `authorityClass` config field is one of `{public, us-government, china}` or a registered effect-port alias; reject any string containing `://`, `.`, port separators, path components, query-string characters, or hostname-shaped bytes; effect-port aliases are validated as opaque identifiers matching `^[a-z][a-z0-9-]*$`. (5) Validate `controllerExecutionRef` is present and resolves a `Host/<name>` in the same Zone with `system` in `allowedDomains`; this field is **required** — there is no default Zone primary Host fallback; return a hard validation error if absent. (6) Retain `EntraCredentialClient` trait unchanged; adapt `EntraCredentialProvider` to v3 `d2b.credential.v3` service interface in `service.rs`. (7) Map `EntraClientError::InteractionRequired` to `credential-provider-unavailable` (not `credential-operation-denied`). (8) Enforce `EntraCredentialOwner::ExactConsumer`: `consumerRef` required in spec; reject any caller not matching `consumerRef` at d2b-bus before service dispatch. (9) Implement `entra-controller` in `controller.rs` and `controller_main.rs` per §12: receive ProviderSupervisor descriptor registration; watch Credential resources; on Create/Update create `entra-agent` Process via LaunchTicket with canonical Process template (template=entra-agent-main, correct providerRef per domainFilter, executionRef/domain/userRef from Credential scope, ownerRef=Credential/<name>, networkUsage.allowEgress=false, sealed config projection including tenantId, authorityClass, consumerRef, maxLeases, interactionPolicy, audience, idempotency_key, and effect-port FD index; no authorityUrl, no endpoint URL, no hostname); on agent Process failure set `ProviderUnavailable=True` and requeue; on Deletion: signal agent to revoke (per revocation policy), await agent confirmation, then call `UpdateFinalizers` to remove `credential.d2b.io/provider-revoke`; core automatically writes event-only Deleted revision and removes the row/indexes when no finalizers remain; audit subsystem appends deletion record post-commit with dedup/exactly-once recovery. (10) Implement `entra-agent` in `service.rs` and `agent_main.rs` per §12 agent startup: read sealed config from ProviderSupervisor inherited FD including effect-port FD index and authorityClass; open effect-port FD; construct `EntraCredentialClient` implementation over it using authorityClass to select authority endpoint/TLS internally (no ambient HTTPS socket opened; no hostname validation performed by the agent); `OpaqueAzureRef::parse(tenantId)`; call `issue_lease`; open `d2b.credential.v3` listener; write Process status Ready; serve credential methods for `consumerRef` only; establish Noise KK delivery channel for token/signature output. (11) Implement `audit.rs` and `telemetry.rs` per §10; apply `contains_sensitive_shape` guard in agent before all outbound string fields. |
| Integration | User-domain or system-domain Process under Guest (or user-domain under Host); `entra-controller` component registered with d2b-bus; Credential controller reconciles `Credential` resources with `providerRef=Provider/credential-entra`; `Provider/system-minijail` or `Provider/system-systemd` launches the process via ProviderSupervisor LaunchTicket |
| Data migration | Full v3 reset; no migration from old `CredentialProvider` trait |
| Validation | See §18 |
| Removal proof | Old `d2b-realm-provider:CredentialProvider` trait removed only after `credential-entra`, `credential-secret-service`, and `credential-managed-identity` controllers all reach full reconcile parity and their integration tests pass |

---

## 18. Tests

### `src/` unit tests (`#[cfg(test)]` in `src/` files)

| Test | Purpose |
| --- | --- |
| `test_controller_creates_agent_process` | Controller `reconcile` creates an `entra-agent` Process via `ProviderSupervisor::LaunchTicket` with correct `metadata.ownerRef`, placement, and sealed config fields (effect-port FD index included) |
| `test_agent_deleted_on_credential_delete` | Controller `finalize` sends revocation signal to agent; controller calls `UpdateFinalizers` to clear `credential.d2b.io/provider-revoke`; core performs event-only Deleted + row removal; audit appends post-commit |
| `test_controller_process_template_schema` | Controller Process template fields match canonical schema: `type: Process`, `namespaceClasses=[mount,pid,ipc,uts,network]`, `capabilityClasses=[]`, `seccompClass=strict`, `startRoot=false`, `noNewPrivileges=true`, `environmentClass=minimal`, `readOnlyRoot=true`, `networkUsage.allowEgress=false`, `mounts=[{view:main,mountPath:/state,access:read-only,required:true}]`, `endpoints=[{name:bus-registration,transport:unix,purpose:controller-registration}]`, `readiness.class=provider-defined` |
| `test_user_agent_process_template_schema` | User-agent Process template: `type: Process`, `providerRef=Provider/system-systemd`, `domain=user`, `networkUsage.allowEgress=false`, `mounts=[{view:main,mountPath:/state,access:read-only,required:true}]`, `endpoints=[{name:credential-service,transport:unix,purpose:d2b.credential.v3}]`, `budget.memory.limit="128Mi"`, `readiness.class=provider-defined`; no `binary`, `allowedSyscalls`, `maxRssBytes`, endpoint `kind`/`service` fields |
| `test_guest_agent_system_process_template_schema` | Guest-agent system-domain Process template: `type: Process`, `providerRef=Provider/system-minijail`, `domain=system`, `mounts=[{view:main,mountPath:/state,access:read-only,required:true}]`; all other fields identical to user-agent template |
| `test_entra_client_trait_surface` | Verify `EntraCredentialClient` trait is object-safe and all methods have correct async signatures |
| `test_opaque_azure_ref_parse_tenant_id` | `OpaqueAzureRef::parse` accepts valid GUIDs; rejects secret-shaped values, `://`, `/`, `+`, `=`, whitespace, `{}` |
| `test_authority_class_enum_validation` | Accepts `public`, `us-government`, `china`; accepts opaque effect-port alias matching `^[a-z][a-z0-9-]*$`; rejects any string containing `://`, `.`, port separators, path components, query-string characters, or hostname-shaped bytes; rejects unknown aliases |
| `test_provider_state_set_controller_volume_empty_payload` | Controller ProviderStateSet Volume is a durable `kind: state` Volume: `type: Volume`, `ownerRef: Provider/credential-entra`, `spec.kind: state`, `persistenceClass: persistent`, `sourcePolicyId: credential-entra-state-v1`, `quotaBytes: 65536`, `maxBytes: 65536`, `maxInodes: 256` (all nonzero), `views.main.rights: [read, traverse]`, `identityMarker` present; stateSchema payload schema declares no data fields; Volume survives controller and Provider restart; participates in upgrade/destroy/reset |
| `test_provider_state_set_agent_volume_empty_payload` | User-agent and guest-agent each have a separate durable `kind: state` Volume with `sourcePolicyId: credential-entra-state-v1`, nonzero `quotaBytes`/`maxBytes`/`maxInodes`, `views.main.rights: [read, traverse]`, and identity marker; three components → three non-shared Volumes; guest-agent Volume has `attachments[{transport: virtiofs, executionRef: Guest/<name>}]` (single Volume, no separate attachment Volume); never `persistenceClass: ephemeral`, zero quota, or `access: read-write` |
| `test_exact_consumer_guard` | `EntraCredentialOwner::ExactConsumer` rejects callers not matching `consumerRef`; accepts the exact declared consumer |
| `test_entra_client_state_transitions` | `Ready → InteractionRequired → Ready`; correct error mapping per §6 |
| `test_interaction_required_maps_to_unavailable` | `EntraClientError::InteractionRequired` → `credential-provider-unavailable` (not `credential-operation-denied`) |
| `test_host_system_placement_rejected` | Construction with `host-system` placement fails closed with `credential-placement-mismatch` |
| `test_user_agent_placement_accepted` | Construction with `user-agent` placement succeeds |
| `test_guest_agent_placement_accepted` | Construction with `guest-agent` placement succeeds |
| `test_idempotency_key_derivation` | Same UID+rotationGeneration+operationClass always produces the same key; different inputs produce different keys; key contains no secret bytes |
| `test_max_leases_cap_at_construction` | Config `maxLeases > MAX_LOCAL_LEASES` (256) fails Provider spec validation |
| `test_contains_sensitive_shape_on_error_messages` | Error message strings pass `contains_sensitive_shape` guard |
| `test_debug_redaction_entra_lease_grant` | `EntraLeaseGrant` Debug output contains only type name placeholder; no token bytes |
| `test_debug_redaction_entra_lease_renewal` | Same for `EntraLeaseRenewal` |
| `test_debug_redaction_entra_lease_request` | Same for `EntraLeaseRequest` |

### `tests/` Cargo integration tests (`cargo test -p d2b-provider-credential-entra`)

#### `tests/lifecycle.rs` — end-to-end acquire/refresh/revoke/inspect with `FakeEntraClient`

| Test | Purpose |
| --- | --- |
| `test_acquire_token_lifecycle` | Full acquire→inspect→revoke cycle with `FakeEntraClient`; verify outer DTO non-secret fields |
| `test_refresh_token_lifecycle` | Acquire then refresh; verify `expiresAtUnixMs` updated; no token in outer DTO |
| `test_revoke_token_lifecycle` | Acquire then revoke; verify `leaseState=Revoked`; idempotent second revoke |
| `test_inspect_metadata_after_acquire` | Inspect returns correct `leaseState`, `expiresAtUnixMs`, `sourceVersion` without token bytes |
| `test_sign_challenge_lifecycle` | Sign request with `FakeEntraClient`; verify signature bytes stay in delivery session only |
| `test_acquire_idempotency` | Duplicate acquire with same `idempotency_key` returns same grant without double-issue |
| `test_refresh_after_rotation` | Refresh with stale `rotationGeneration` ref returns `credential-not-found` |
| `test_revoke_and_reacquire` | Post-revoke `AcquireToken` issues fresh grant with incremented `rotationGeneration` |

#### `tests/conformance.rs` — all provider conformance arms pass

| Test | Purpose |
| --- | --- |
| `test_conformance_all_arms` | All conformance arms in `check_provider_conformance` pass for `credential-entra` |
| `test_conformance_secret_service_ops_not_declared` | `sign-challenge` is in the supported set for entra (unlike secret-service) |
| `test_conformance_host_system_rejected` | Conformance arm for placement rejects `host-system` |
| `test_conformance_consumer_ref_required` | Conformance arm for `consumerRef` requirement enforced |

#### `tests/faults.rs` — fault injection and error mapping

| Test | Purpose |
| --- | --- |
| `test_interaction_required_returns_unavailable` | `FakeEntraClient` returns `InteractionRequired`; service returns `credential-provider-unavailable`; no `credential-operation-denied` |
| `test_generation_mismatch_rejected` | Lease ref with stale `rotationGeneration` is rejected; correct error code returned |
| `test_colocated_consumer_rejection` | Caller with wrong `consumerRef` identity receives `credential-consumer-mismatch` before any client call |
| `test_service_unavailable_maps_to_unavailable` | `EntraClientError::ServiceUnavailable` → `credential-provider-unavailable` |
| `test_unauthorized_maps_to_denied` | `EntraClientError::Unauthorized` → `credential-operation-denied` |
| `test_lease_limit_exceeded_maps_to_queue_pressure` | Exceeding `maxLeases` → `credential-queue-pressure` |
| `test_sign_challenge_interaction_required` | `sign-challenge` during `InteractionRequired` state → `credential-provider-unavailable` |

#### `tests/canary.rs` — `credential_canary` and `endpoint_canary` enforcement

| Test | Purpose |
| --- | --- |
| `test_credential_canary_absent_from_acquire_response` | `"entra-token-canary"` value from `FakeEntraClient` never appears in `AcquireTokenResponse` outer DTO |
| `test_credential_canary_absent_from_refresh_response` | Same for `RefreshTokenResponse` outer DTO |
| `test_endpoint_canary_absent_from_status` | `"endpoint-canary"` (provider-internal URL) never appears in `Credential.status` or any service response |
| `test_canary_absent_from_audit_records` | `"entra-token-canary"` and `"endpoint-canary"` absent from all emitted audit record fields |
| `test_canary_absent_from_otel_spans` | Canary values absent from all span attributes and metric label values |
| `test_canary_absent_from_log_lines` | Canary values absent from all log output during FakeEntraClient test runs |
| `test_canary_absent_from_error_messages` | Error message strings returned to callers contain neither canary value |
| `test_canary_absent_from_delivery_session_binding` | Delivery session binding fields (routeDigest, credentialRef, etc.) contain no canary values |

#### `tests/delivery.rs` — Noise KK delivery session contract

| Test | Purpose |
| --- | --- |
| `test_delivery_session_binding_fields` | All binding fields present and correct after handshake |
| `test_delivery_session_kk_only` | NN/NX profile attempt is rejected; channel closed and zeroized |
| `test_delivery_session_zeroizing_buffer` | Token plaintext buffer is zeroed after extraction; `Zeroizing<Vec<u8>>` drop verified |
| `test_delivery_session_replay_safe_sequence` | Replay of prior session's ciphertext at same sequence number is rejected |
| `test_delivery_session_max_token_bytes` | Record exceeding `maxTokenBytes` is rejected; channel closed immediately |
| `test_delivery_session_no_retry_on_ambiguous_outcome` | Provider does not auto-retry delivery after ambiguous disconnect; consumer must re-initiate |
| `test_delivery_session_immediate_close_after_ack` | Channel closed and key material zeroized after consumer ACKs delivery |
| `test_delivery_sign_challenge_session_binding` | `sign-challenge` uses same KK channel contract; binding includes `operationClass=sign-challenge` |

#### `tests/placement.rs` — placement binding enforcement

| Test | Purpose |
| --- | --- |
| `test_host_system_placement_rejected_at_construction` | `host-system` construction fails closed; `PlacementBinding::HostSystem` arm returns `credential-placement-mismatch` |
| `test_user_agent_on_host_accepted` | `user-agent` with `scope.executionRef=Host/<name>` accepted |
| `test_user_agent_on_guest_accepted` | `user-agent` with `scope.executionRef=Guest/<name>` accepted |
| `test_guest_agent_on_guest_accepted` | `guest-agent` with `scope.executionRef=Guest/<name>` accepted |
| `test_guest_agent_on_host_rejected` | `guest-agent` with `scope.executionRef=Host/<name>` rejected; Entra guest-agent requires a Guest |

### `integration/` fixtures

#### `integration/container-service.sh`

Container-backed Provider service start/stop/drain:

- Starts `d2b-provider-credential-entra` binary in a container with a mock
  Entra HTTP endpoint (no real network calls in CI).
- Verifies Provider readiness signal.
- Issues `AcquireToken` → verifies outer DTO non-secret fields.
- Drains the provider (graceful shutdown); verifies in-flight requests complete
  or return bounded error.
- Verifies process exits cleanly with key material zeroized.

#### `integration/guest-placement.nix`

`runNixOSTest` VM fixture for user-domain and system-domain `entra-agent` Process on a Guest:

- Declares a minimal Zone with `Provider/credential-entra` and a `Guest/work-vm`.
- user-domain: `scope.domainFilter=user`, `scope.userRef=User/alice`; verifies
  `entra-agent` Process is created and runs in user domain of `Guest/work-vm`
  (i.e. `executionRef=Guest/work-vm, domain=user, userRef=User/alice`); verifies
  `entra-controller` remains on Zone host (system-domain).
- system-domain (guest-agent): `scope.domainFilter=system`,
  `scope.executionRef=Guest/work-vm`; verifies `entra-agent` Process runs in
  system domain of `Guest/work-vm`.
- host-system rejection: verifies that declaring
  `scope.executionRef=Host/host-system` with `domainFilter=system` fails the
  Nix eval with the expected assertion message.
- `FakeEntraClient` injected at test time; no real Entra endpoint required.

#### `integration/cleanup-rollback.sh`

Nix-generation removal triggers async Delete and `provider-revoke` finalizer:

- NixOS generation N declares `Credential/work-entra` with `Provider/credential-entra`.
- NixOS generation N+1 removes `Credential/work-entra`.
- Verifies: (1) activation for generation N+1 completes (returns Ready status on
  new resources) before `provider-revoke` finalizer finishes (non-blocking
  activation invariant); (2) `Credential/work-entra` reaches `phase=Terminating`
  with `credential.d2b.io/provider-revoke` finalizer running; (3) `leaseState`
  transitions to `Revoked` (under `revocation.onOwnerDelete=immediate`);
  (4) controller calls `UpdateFinalizers` to clear `credential.d2b.io/provider-revoke`
  (last finalizer); core performs one atomic store transaction writing the
  event-only `Deleted` revision and removing the row/indexes — no `Deleted` row
  persists; after that transaction commits the audit subsystem appends
  `ResourceMutation{event="deleted"}` using dedup/exactly-once recovery (audit
  is not part of the store transaction);
  (5) `FakeEntraClient` `revoke_lease` call was invoked exactly once (idempotency);
  (6) no token bytes appear in any audit record or log during the cleanup run.

---

## 19. Upgrade and migration

### Provider generation upgrade

When `spec.config.tenantId` or `spec.config.authorityClass` changes (e.g.
tenant migration):

1. Operator updates `Provider.spec.config.tenantId` (and/or `authorityClass`) in
   Nix config. If migrating from a hypothetical future version that supported a
   bare `authorityUrl` field, replace it with the appropriate `authorityClass`
   value; `"public"` corresponds to the standard Microsoft Entra login endpoints
   and is the correct default for most tenant migrations.
2. Nix eval validates `tenantId` with `OpaqueAzureRef` charset assertion;
   validates `authorityClass` is a known closed enum value or registered
   effect-port alias; rejects any string containing `://`, `.`, port, path, or
   hostname-shaped bytes.
3. `activation-nixos` applies the new Provider resource; `entra-controller`
   detects the generation change.
4. Credential resources with `revocation.onProviderGeneration=immediate`: controller
   revokes all active leases against the old provider state before generation
   transitions.
5. Credential resources with `revocation.onProviderGeneration=drain-leases`:
   active leases expire by natural deadline; new `AcquireToken` calls use the
   new tenant.
6. No v2 credential state is imported; d2b 3.0 is a full reset.

### Removal

`d2b-realm-provider:CredentialProvider` trait and the associated v2
`CredentialProvider` implementation are removed only after:

1. `d2b-provider-credential-entra` controller reaches full reconcile parity with
   the test matrix in §18.
2. `d2b-provider-credential-secret-service` and
   `d2b-provider-credential-managed-identity` controllers also reach full reconcile
   parity (coordinated removal across all three).
3. All callers of the old `CredentialProvider` trait in `d2b-realm-provider`
   have migrated to the v3 `d2b.credential.v3` service interface.
4. All integration tests pass with v3 controllers.

---

## 20. Dependencies and permission claims

| Dependency alias | Bound to | Required |
| --- | --- | --- |
| `runtime` | Guest or Host Provider providing the execution context | Required (via `scope.executionRef`) |
| `credential` | None (this Provider provides credentials; it does not consume them) | Not applicable |
| `transport` | Local Unix/socketpair transport for d2b-bus (provided by Zone runtime) | Required |
| `volume` | Core ProviderDeployment creates and deletes declared component state Volumes (controller and per-agent) before/after component Processes; `Provider/volume-local` is the sole reconciler; `credential-entra` controller does not create, delete, or own Volume resources and does not add `Volume` to its exported ResourceTypes; components only consume their declared view mount | Core-provisioned (not controller-owned) |
| `network` | Both controller and `entra-agent` Process templates declare `networkUsage.allowEgress=false`. No ambient MSAL network claim exists. The agent's `EntraCredentialClient` calls are proxied through the injected effect-port FD provided by the co-located consumer/runtime Provider; that Provider owns the network interface. | Not claimed by this Provider |

### Permission claims

| Claim | Scope | Rationale |
| --- | --- | --- |
| `update-status` on `Credential` | Own `Credential` resources (providerRef match) | Controller writes reconciled status |
| `update-finalizers` on `Credential` | Own `Credential` resources | Controller manages `provider-revoke` finalizer |
| `use-credential` (inbound) | Granted to `consumerRef` Provider | Consumer invokes credential-bound service methods |
| d2b-bus `open-stream` for KK delivery | Consumer Provider component | Delivery session establishment |

The provider does not claim `create`, `delete`, or `update-spec` on any
ResourceType. It does not hold host-global broker operations. It does not
read sibling Provider state. It does not access the Zone host broker, device
subsystem, or Wayland compositor.
