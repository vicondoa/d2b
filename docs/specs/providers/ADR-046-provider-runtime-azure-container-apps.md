# ADR 0046 Provider dossier: runtime-azure-container-apps

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-runtime-azure-container-apps` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Main reuse | `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` |
| Normative | Yes |
| Owner | `packages/d2b-provider-runtime-azure-container-apps/` |
| Depends on | `ADR-046-provider-model-and-packaging`, `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-credential`, `ADR-046-componentsession-and-bus`, `ADR-046-zone-routing`, `ADR-046-provider-state`, `ADR-046-telemetry-audit-and-support`, `ADR-046-nix-configuration` |
| Supersedes | `packages/d2b-provider-aca/` (`AcaWorkloadProvider`, `GuestControlEndpointProvider`), `AcaRelayTransportConfig`, direct vsock guest-control path |

---

## 1 Provider identity

| Field | Value |
| --- | --- |
| Provider name | `runtime-azure-container-apps` |
| ResourceRef | `Provider/runtime-azure-container-apps` |
| Implementation ID | `azure-container-apps` |
| Crate | `packages/d2b-provider-runtime-azure-container-apps/` |
| Implements | `Guest` ResourceType |
| Domain | `system` only |
| Placement | System-domain Processes inside the dedicated gateway Guest |

The Provider owns exactly one ResourceType: `Guest`. Two component Processes — controller and deployment service — run as system-domain Process resources **inside a dedicated gateway Guest** (`spec.config.gatewayExecutionRef`), never on the local physical Host. This is required by ADR-032: realm/cloud credentials, remote node state, and cloud-plane I/O belong inside a per-realm gateway guest VM, never in host processes, the broker, the host bundle, or host-readable storage.

The Host holds only the opaque ZoneLink transport binding to the gateway Guest (standard `runtime-cloud-hypervisor` or equivalent relationship). The Host has no Process resources from this Provider, no Credential resources scoped to the ACA environment, no Azure management HTTP sockets, and no co-located `AcaControl` implementation. There is no Host-fallback mode.

All interaction with Azure Container Apps APIs crosses the injected async effect port (`AcaControl`) and the injected credential lease client (`AcaCredentialLeaseClient`), both running inside the gateway Guest. Neither interface is reachable via ambient environment variable, SDK default chain, or fallback discovery.

---

## 2 Remote/cloud Guest semantics

A `Guest` resource backed by `Provider/runtime-azure-container-apps` represents a **remote cloud sandbox**. This Provider operates a **two-tier Guest model**:

- **Tier 1 — gateway Guest** (`spec.config.gatewayExecutionRef`): A Zone-local Guest VM (backed by `Provider/runtime-cloud-hypervisor` or equivalent) that runs the ACA controller and deployment service Processes. This is the credential and cloud-control boundary. Azure credentials, Azure HTTP sockets, the `AcaControl` implementation, and the `AcaCredentialLeaseClient` all live exclusively inside this gateway Guest.
- **Tier 2 — managed ACA sandbox** (the `Guest` resources this Provider reconciles): Remote Azure Container Apps sandboxes. The controller, running inside the gateway Guest, manages the lifecycle of these remote sandboxes via the injected `AcaControl` port and reaches them over the `Provider/transport-azure-relay` ZoneLink.

A managed ACA sandbox `Guest` differs from a local VM Guest in the following fixed ways:

| Property | Local VM Guest | Managed ACA sandbox Guest |
| --- | --- | --- |
| Execution substrate | Cloud Hypervisor or QEMU process on Host | Azure Container Apps sandbox in a remote ACA environment |
| `spec.systemArtifactId` | NixOS system closure artifact ID | `null` — no Nix-built system; image is declared via `providerSettings.configuredImageId` or `providerSettings.configuredDiskId` |
| Bootstrap process | VMM Process + virtiofsd on Host | None on Host; bootstrap is the ACA sandbox reaching the gateway Guest via `Provider/transport-azure-relay` ZoneLink |
| `spec.allowedDomains` | `[system, user]` typical | `[system]` — no local PAM user manager; user domain processes inside the sandbox are not d2b Process resources |
| Attachment types | virtiofs Volume, local Network bridge | ZoneLink only; no local virtiofsd, no Host-local bridge attachment |
| Controller location | VMM controller runs on Host | Controller runs inside gateway Guest; Host has no Process from this Provider |
| `status.bootstrapReady` | Set after VMM + guest-control vsock reach Ready | Set after the enrolled Noise KK ComponentSession from the gateway Guest to the ACA sandbox becomes established and the sandbox passes the Provider's authenticated health check |
| `status.guestIdentityDigest` | Boot-token + generation digest | SHA-256 hex of `(sandboxId_bytes \|\| providerGeneration_be64 \|\| configFingerprint_bytes)` — not the ACA resource ID string |
| Process/EphemeralProcess | Full d2b Process resource model inside guest | Only controller-managed service processes declared in the Provider's own component templates; arbitrary guest-side processes are not d2b resources |

The controller (inside the gateway Guest) is the exclusive authority for each managed sandbox's lifecycle. No external agent, operator script, or sibling Provider may mutate the ACA sandbox directly. Finalization revokes all active Credential leases, stops the sandbox, and deletes the ACA resource before the `Deleted` revision and row-removal transaction.

---

## 3 Provider.spec.config schema

The signed root configuration for `Provider/runtime-azure-container-apps` is validated against the Provider's exported JSON Schema before the Provider resource reaches `Ready`. No field in this schema carries secret material.

```yaml
# Provider.spec.config — validated at Provider resource admission
config:
  # Gateway Guest execution boundary — REQUIRED; all controller Processes run inside this Guest
  gatewayExecutionRef: "Guest/aca-gateway"   # required; must resolve to a Ready Guest resource

  # Azure AD identifiers — plain opaque IDs, never secrets
  tenantId: "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"    # max 36 chars; UUID-shaped opaque ID
  clientId: "yyyyyyyy-yyyy-yyyy-yyyy-yyyyyyyyyyyy"    # max 36 chars; UUID-shaped opaque ID
  subscriptionId: "zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz"  # max 36 chars

  # Credential refs — declare which Credential resources the controller may acquire.
  # Both must have scope.executionRef matching gatewayExecutionRef (enforced at eval time).
  controlCredentialRef: "Credential/aca-managed-identity"   # required; used for sandbox lifecycle calls
  pullCredentialRef: "Credential/aca-pull-identity"         # optional; used for container image pull

  # ACA environment placement — opaque bounded IDs, not secrets
  environmentId: "env-xxxxxxxxxxxxxxxxxxxxxxxx"  # max 60 chars; [a-z0-9-] with lowercase lead
  resourceGroupId: "rg-workloads"               # max 60 chars; [a-z0-9-] with lowercase lead

  # Default sandbox profile defaults (overridden per Guest by providerSettings)
  defaults:
    cpuMillis: 500                  # integer; 250..4000 in steps of 250
    memoryMiB: 2048                 # integer; 512..16384 in steps of 256
    autoSuspendSecs: 300            # integer; 60..86400
    planTtlMs: 300000               # integer; 1..300000; operation plan lifetime cap

  # Readiness probe
  readiness:
    attempts: 30                    # integer; 1..60
    intervalMs: 5000                # integer; 1..10000

  # Operation ledger capacity
  completedOperationCapacity: 512   # integer; 1..1024

  # Network reference — required for deployment service egress (all ACA API calls)
  networkRef: "Network/aca-gateway-egress"  # optional; null disables egress in deployment service

  # ZoneLink transport alias (must match a declared ZoneLink dependency alias in the Zone config)
  zoneLinkAlias: "aca-relay"
```

### Field constraints

| Field | Type | Required | Rules |
| --- | --- | --- | --- |
| `gatewayExecutionRef` | ResourceRef | **Yes** | Resolves `Guest/<name>` in same Zone; that Guest must be Ready; both component Processes must have `executionRef` matching this field; **no Host fallback** |
| `tenantId` | string | Yes | Max 36 chars; allowed charset `[0-9a-f-]`; UUID-shaped opaque ID; not a secret |
| `clientId` | string | Yes | Same constraints as `tenantId` |
| `subscriptionId` | string | Yes | Same constraints as `tenantId` |
| `controlCredentialRef` | ResourceRef | Yes | Resolves `Credential/<name>` in same Zone; must allow `acquire-token` operation class; must be backed by `Provider/credential-managed-identity` or `Provider/credential-entra`; **`scope.executionRef` must match `gatewayExecutionRef`** |
| `pullCredentialRef` | ResourceRef | No | Same rules as `controlCredentialRef`; null = no image pull credential; if present, `scope.executionRef` must match `gatewayExecutionRef` |
| `environmentId` | string | Yes | Max 60 chars; `[a-z0-9-]`; lowercase lead; `OpaqueAzureRef`-shaped; not a secret |
| `resourceGroupId` | string | Yes | Max 60 chars; `[a-z0-9-]`; lowercase lead; not a secret |
| `defaults.cpuMillis` | u16 | No | 250..4000 in steps of 250; default 500 |
| `defaults.memoryMiB` | u32 | No | 512..16384 in steps of 256; default 2048 |
| `defaults.autoSuspendSecs` | u32 | No | 60..86400; default 300 |
| `defaults.planTtlMs` | u32 | No | 1..300000; default 300000 |
| `readiness.attempts` | u8 | No | 1..60; default 30 |
| `readiness.intervalMs` | u32 | No | 1..10000; default 5000 |
| `completedOperationCapacity` | usize | No | 1..1024; default 512 |
| `networkRef` | ResourceRef | No | Resolves `Network/<name>` in same Zone; passed to deployment service `networkUsage.networkRef`; null = no egress (deployment service `allowEgress: false`) |

No field may carry a subscription key, SAS token, client secret, certificate bytes, connection string, or any value that functions as an authentication secret. Fields `tenantId`, `clientId`, `subscriptionId`, `environmentId`, and `resourceGroupId` are opaque plain identifiers, not credentials.

**Co-location invariant**: If `controlCredentialRef` or `pullCredentialRef` resolves to a Credential whose `scope.executionRef` does not match `gatewayExecutionRef`, the Provider resource is rejected at admission with `InvalidConfiguration(credential-execution-mismatch)`. This invariant is enforced at both Nix eval time (schema assertion) and runtime admission (Provider validation pass). There is no runtime override and no Host-level fallback execution path.

---

## 4 Guest providerSettings schema

The `providerSettings` object inside a `Guest.spec` when `spec.providerRef = Provider/runtime-azure-container-apps`. Validated against the Provider's exported Guest schema extension.

```yaml
spec:
  providerRef: Provider/runtime-azure-container-apps
  providerSettings:
    # Disk image source — exactly one of configuredDiskId or configuredImageId
    configuredDiskId: "img-xxxxxxxxxxxxxxxxxxxxxxxx"        # max 64 chars; [a-z0-9-], lowercase lead
    configuredImageId: null                                 # null when configuredDiskId is set
    diskName: null                                          # required when configuredImageId is set; max 64 chars
    pullIdentityBindingId: null                             # optional; max 64 chars; [a-z0-9-], lowercase lead

    # Sandbox compute (override Provider defaults)
    cpuMillis: 500                                          # 250..4000 in steps of 250
    memoryMiB: 2048                                         # 512..16384 in steps of 256

    # Lifecycle
    autoSuspendSecs: 300                                    # 60..86400
    sandboxIdentityBindingId: "mid-xxxxxxxxxx"             # optional; max 64 chars; managed identity binding
```

| Field | Type | Required | Rules |
| --- | --- | --- | --- |
| `configuredDiskId` | string | Conditional | Max 64 chars; `[a-z0-9-]`; lowercase lead; mutually exclusive with `configuredImageId` |
| `configuredImageId` | string | Conditional | Max 64 chars; `[a-z0-9-]`; lowercase lead; mutually exclusive with `configuredDiskId` |
| `diskName` | string | Conditional | Required when `configuredImageId` set; max 64 chars; `[a-z0-9-]`; lowercase lead |
| `pullIdentityBindingId` | string | No | Max 64 chars; `[a-z0-9-]`; lowercase lead; names a managed identity in `providerSettings.sandboxIdentityBindingId` binding space |
| `cpuMillis` | u16 | No | 250..4000 in steps of 250; null = Provider default |
| `memoryMiB` | u32 | No | 512..16384 in steps of 256; null = Provider default |
| `autoSuspendSecs` | u32 | No | 60..86400; null = Provider default |
| `sandboxIdentityBindingId` | string | No | Max 64 chars; `[a-z0-9-]`; lowercase lead; managed identity binding assigned to the sandbox at creation |

No field carries an Azure resource path, subscription scope, endpoint URL, raw ID returned from a prior API call, or any value echoed from Azure API responses. All identifiers are Nix-time operator-declared opaque identifiers.

---

## 5 Credential model

### 5.1 No ambient Azure credential chain

This Provider never:

- reads `AZURE_CLIENT_SECRET`, `AZURE_TENANT_ID`, `AZURE_CLIENT_CERTIFICATE_PATH`, or any environment variable that feeds the Azure SDK default credential chain;
- calls `DefaultAzureCredential::new()` or any discovery-based credential builder;
- opens `~/.azure/`, instance metadata endpoints, workload identity projection files, or managed identity URIs directly;
- stores a token, bearer string, refresh token, certificate private key, or SAS token in any field, log line, trace span, metric label, resource store row, or audit record.

### 5.2 Credential resource contract

The gateway Guest (`spec.config.gatewayExecutionRef`) is the exclusive execution boundary for all Credential operations performed by this Provider. Both the `AcaCredentialLeaseClient` implementation and the ACA controller Process run inside the same gateway Guest. No Credential lease acquisition, token delivery, or revocation crosses a gateway boundary to the Host or to the managed ACA sandbox.

The controller acquires credentials exclusively via the `AcaCredentialLeaseClient` injected interface. This interface returns an opaque `AcaCredentialLease` struct whose only accessible field is the `CredentialLease` metadata handle — an opaque non-secret bounded string that the co-located credential module uses internally to map to the actual bearer token. The token bytes are never visible to the ACA controller Process itself.

The required `Credential` resource (referenced by `config.controlCredentialRef`) must be backed by one of:

- `Provider/credential-managed-identity` — for ACA sandbox lifecycle calls using an Azure-assigned managed identity
- `Provider/credential-entra` — for Entra-authenticated service principal flows

Both providers deliver raw token bytes exclusively via the end-to-end Noise KK ComponentSession channel described in §5.3. Both Credential resources must have `scope.executionRef` matching `config.gatewayExecutionRef`; resources scoped to `Host/*` or to a different Guest are rejected at admission.

### 5.3 Raw-token delivery: end-to-end Noise KK

When the authorized consumer Process (the ACA controller's credential-consuming component) needs the actual bearer token to pass to the ACA SDK adapter, the delivery happens entirely within the gateway Guest:

- **Both endpoints are co-located inside the gateway Guest**: the credential Provider Process and the ACA controller Process are both system-domain Processes with `executionRef: Guest/<aca-gateway-name>`. No token bytes cross the gateway Guest boundary at any point.
- Initiator and responder are fully enrolled Provider/component identities with registered KK static public keys, both resolved within the gateway Guest's component namespace.
- Profile is `Noise_KK_25519_ChaChaPoly_SHA256`; NN and IKpsk2 are forbidden for this channel.
- The Noise prologue binds: Credential ResourceRef/UID/generation, consumer Provider/component generations, audience token (non-secret opaque string), route, schema fingerprint, limits, expiry/deadline, and authorization revisions.
- d2b-bus ZoneLink intermediaries (between the gateway Guest and the managed ACA sandbox) authorize route establishment but forward opaque Noise-encrypted records for the outer guest-to-sandbox session; the inner credential KK channel is a separate co-located session entirely within the gateway Guest and does not traverse any ZoneLink.
- Token payload has a strict small bound (`MAX_TOKEN_PAYLOAD_BYTES = 8192`), zeroizing buffers, redacted Debug, replay-safe sequence counter, no logging, no audit, no metrics, and immediate close with zeroize after delivery.
- Ambiguous delivery is never treated as success and is not automatically replayed outside the credential method's explicit idempotency contract.

### 5.4 Credential lease lifecycle

The controller acquires a lease per operation call via `AcaCredentialLeaseClient::acquire`. Each lease:

- has a bounded expiry (`requested_expiry_unix_ms` set to the call deadline); the credential module may grant a shorter lifetime;
- is revoked immediately after the operation completes or the deadline expires (`AcaCredentialLeaseClient::revoke`);
- revocation is idempotent and failure-tolerant: `MAX_ACA_LEASE_CLEANUP_MS = 1000` timeout; timeout revocations are logged at `d2b_provider_runtime_azure_container_apps::credential_lease_cleanup` target with outcome label only;
- is never stored in the resource store, status, audit record, or any durable artifact.

The completed-operation ledger records the opaque `OperationBinding` — not the lease handle or token bytes.

---

## 6 Component architecture

### 6.1 Component summary

| Component ID | Type | Description |
| --- | --- | --- |
| `aca-controller` | controller | Owns `Guest` ResourceType; async reconcile/observe/finalize loop; calls ACA API exclusively through injected `AcaControl`/`AcaCredentialLeaseClient` ports |
| `aca-deployment-service` | service | Serves typed deployment + environment health ComponentSession methods; holds ACA effect port authority; co-located in gateway Guest |

Both components run as system-domain Process resources **inside the dedicated gateway Guest** (`spec.config.gatewayExecutionRef`). The framework `ProviderDeployment` creates these two static component Processes and their `ProviderStateSet` compartments. The ACA controller never creates its own peer Processes and never writes Provider resource status directly. No component runs on the local Host. No component runs inside the managed ACA sandbox.

### 6.2 Controller component descriptor

```yaml
componentId: aca-controller
type: controller
resourceTypes:
  - Guest
supportedHostCapabilities: []
supportedGuestCapabilities: []
allowedDomains: [system]
cardinality: one-per-zone
requiredDependencies:
  - alias: credential        # resolves Provider/credential-managed-identity or credential-entra
  - alias: zone-link         # resolves Provider/transport-azure-relay
optionalDependencies: []
stateNamespaces:
  - id: sandbox-state
    schemaId: io.d2b.runtime-azure-container-apps/aca-controller/sandbox-state
    schemaVersion: "1.0"
    persistenceClass: persistent
    sensitivityClass: private
    migrationPolicy: pre-launch-required
    quotaBytes: 10485760      # 10 MiB; opaque ACA binding/adoption metadata only; no token/endpoint bytes
    views:
      main:
        rights: [read, write, create, delete, traverse]
  # operation-ledger stateNamespace removed: the core Operation ledger owns operation/requeue truth.
  # The provider reads requeue/operation state via the core adapter; no provider-owned compartment needed.
process:
  sandbox:
    namespaceClasses: [mount, ipc, pid]
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    cpu:
      request: "100m"
      limit: "500m"
    memory:
      request: "32Mi"
      limit: "128Mi"
    fds:
      limit: 256
  networkUsage: null
```

The `sandbox-state` stateNamespace is declared in the controller's signed component descriptor. Core `ProviderDeployment` creates the required `sandbox-state` Volume (`providerRef: Provider/volume-local`, `source.executionRef: Guest/<aca-gateway-name>`; guest-local placement; reconciled by the Guest-local `volume-local` instance inside the gateway Guest; see §7.3) before starting the controller Process, and deletes it after the controller Process finalizes on Provider removal. The Host never holds cloud binding, admission, or operation state. The controller does not create, own, or reconcile Volume resources; `Volume` is not in the controller's exported `resourceTypes`; `Provider/volume-local` is the sole Volume reconciler. The framework delivers the mounted view dirfd to the controller at startup. The core Operation ledger adapter provides operation/requeue tracking; the `operation-ledger` stateNamespace has been removed.

### 6.3 Deployment service component descriptor

```yaml
componentId: aca-deployment-service
type: service
exportedMethods:
  - service: d2b.aca.v3.deployment
    methods:
      - GuestProvision
      - GuestStart
      - GuestStop
      - GuestDestroy
      - GuestAdopt
      - GuestInspect
      - GuestHealth
allowedDomains: [system]
cardinality: one-per-zone
stateNamespaces:
  - id: service-state
    schemaId: io.d2b.runtime-azure-container-apps/aca-deployment-service/service-state
    schemaVersion: "1.0"
    schemaDigest: sha256:<hex>
    persistenceClass: persistent
    sensitivityClass: private
    migrationPolicy: none               # empty payload schema; no migration worker
    quotaBytes: 65536                   # 64 KiB; identity marker only
    views:
      main:
        rights: [read, write, create, delete, traverse]
process:
  sandbox:
    namespaceClasses: [mount, ipc, pid]
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    cpu:
      request: "100m"
      limit: "500m"
    memory:
      request: "32Mi"
      limit: "128Mi"
    fds:
      limit: 256
  networkUsage:
    networkRef: <resolved from spec.config.networkRef>
    ports: []
    allowEgress: true
```

`GuestHealth` polls the ACA environment reachability endpoint through the injected `AcaControl` port. No network call is made on any ambient endpoint, SDK default, or host-level socket. The deployment service holds the ACA effect port authority; the controller invokes health probing via `d2b.aca.v3.deployment/GuestHealth` over the internal ComponentSession.

---

## 7 Process templates and placement

The framework `ProviderDeployment` creates both component Processes as static resources when the Provider is admitted. The ACA controller never instantiates its own peer Processes and never writes `Provider` resource status directly. Both Processes use `providerRef: Provider/system-minijail`; the framework assigns each a dedicated core `ComponentPrincipal` (private minijail launch identity; never a ResourceRef). Core `ProviderDeployment` creates one provider-owned Volume per declared stateNamespace per component before each Process starts, and deletes them after Processes finalize. Each Volume uses guest-local placement: `providerRef: Provider/volume-local`, `source.executionRef: Guest/<aca-gateway-name>`, reconciled by the Guest-local `volume-local` instance inside the gateway Guest. There is no Host-backed source, no virtiofs attachment, and no `attachments[]` array. The Host must never hold cloud binding, admission, PSK, or operation state. The ACA controller does not create, own, or add `Volume` to its exported `resourceTypes`; `Provider/volume-local` is the sole Volume reconciler. The framework delivers each mounted view dirfd to its Process at startup. No cross-component Volume sharing, no `accessAcl`/`defaultAcl` entries with external principals, and no OS-account plumbing beyond Nix provisioning are required.

`spec.template` is the signed component ID resolved at runtime by the `ProviderDeployment` of `metadata.ownerRef` (`Provider/runtime-azure-container-apps`). There is no `componentRef` field. Component-to-component calls use d2b-bus ComponentSession exclusively.

### 7.1 Controller Process template

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: aca-controller
  zone: <zone>
  ownerRef: Provider/runtime-azure-container-apps
spec:
  providerRef: Provider/system-minijail
  executionRef: Guest/<aca-gateway-name>
  domain: system
  processClass: controller
  template: aca-controller
  sandbox:
    namespaceClasses: [mount, ipc, pid]
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    cpu: { request: "100m", limit: "500m" }
    memory: { request: "32Mi", limit: "128Mi" }
    fds: { limit: 256 }
    pids: { limit: 256 }
  networkUsage: null
  endpoints: []
  readiness:
    initialDelay: "0s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
    class: ready-condition
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
  mounts:
    - volumeRef: Volume/runtime-azure-container-apps--aca-controller--sandbox-state--<aca-gateway-name>
      view: main
      mountPath: /state/sandbox-state
      access: read-write
      required: true
```

The controller makes no direct network calls. All Azure API calls are dispatched through the injected `AcaControl` port, which executes inside the deployment service. Long-running cloud operations return `progressing` or a `requeue-at` timestamp immediately and never block the controller's watch read loop; the controller re-enqueues the resource and reads updated status on the next reconcile iteration.

### 7.2 Deployment service Process template

The deployment service is the sole bearer of ACA effect port authority. It serves the `d2b.aca.v3.deployment` ComponentSession schema — including `GuestHealth` — and makes all outbound Azure API calls through the injected `AcaControl` port. It holds no controller authority and cannot write `Guest` resource status. Health probing (previously a separate worker) is an ordinary service method here; no separate health component or shared Volume is needed.

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: aca-deployment-service
  zone: <zone>
  ownerRef: Provider/runtime-azure-container-apps
spec:
  providerRef: Provider/system-minijail
  executionRef: Guest/<aca-gateway-name>
  domain: system
  processClass: service
  template: aca-deployment-service
  sandbox:
    namespaceClasses: [mount, ipc, pid]
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    cpu: { request: "100m", limit: "500m" }
    memory: { request: "32Mi", limit: "128Mi" }
    fds: { limit: 256 }
    pids: { limit: 256 }
  networkUsage:
    networkRef: <resolved from spec.config.networkRef>
    ports: []
    allowEgress: true   # ACA management API via injected AcaControl; no ambient endpoint
  endpoints:
    - name: bus
      transport: unix
      purpose: d2b.aca.v3.deployment
  readiness:
    initialDelay: "0s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
    class: ready-condition
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
  mounts:
    - volumeRef: Volume/runtime-azure-container-apps--aca-deployment-service--service-state--<aca-gateway-name>
      view: main
      mountPath: /state/service-state
      access: read-write
      required: true
```

### 7.3 State Volume resources

**ProviderStateSet** is a query-time grouping — not a ResourceType or stored artifact. It is defined as the set of all `Volume` resources in a Zone whose `metadata.ownerRef` resolves to `Provider/runtime-azure-container-apps`. Core `ProviderDeployment` creates every Volume in the ProviderStateSet before the owning component Process starts, and deletes them after the Process finalizes on Provider removal. Semantic Provider controllers do not own Volume resources, do not add `Volume` to their exported `resourceTypes`, and do not create their own prerequisite state Volumes. `Provider/volume-local` is the sole Volume reconciler; the component only consumes its required view via the framework-delivered dirfd. `ComponentPrincipal` is the private minijail launch identity and is never used as a Volume layout `ownerRef`, `groupRef`, or any ResourceRef.

**Empty payload schema rule**: a component that declares a stateNamespace with no payload fields (schema is an empty object) uses `migrationPolicy: none` and requires no migration worker EphemeralProcess. The Volume is still durable (`kind: state`, `persistenceClass: persistent`, minimal nonzero `quotaBytes`, broker-maintained `identityMarker`).

**Volume architecture**: each semantic component (`aca-controller` and `aca-deployment-service`) receives exactly **one** provider-owned Volume with **guest-local** placement: `providerRef: Provider/volume-local`, `source.executionRef: Guest/<aca-gateway-name>`. The Guest-local `volume-local` instance running inside the gateway Guest reconciles it. There is no Host-backed source, no virtiofs attachment, and no `attachments[]` array. The manifest freezes guest-local placement; there is no fallback to Host-side storage. The Host must never hold remote or cloud binding, admission, PSK, or operation state. All Volumes carry `ownerRef: Provider/runtime-azure-container-apps` and Nix-provisioned `User/<name>` layout principals (one system user per component; see §7.3.3). The Process mount is satisfied when the Volume is Ready; the framework delivers the view dirfd at startup.

Layout principals are the Nix-provisioned system user `User/d2b-aca-controller` (declared in the gateway Guest's system-core Nix module). No cross-component Volume sharing and no `accessAcl`/`defaultAcl` entries with external principals are required.

#### 7.3.1 Sandbox state — source Volume

```yaml
apiVersion: resources.d2b.io/v3
type: Volume
metadata:
  name: runtime-azure-container-apps--aca-controller--sandbox-state--<aca-gateway-name>
  zone: <zone>
  ownerRef: Provider/runtime-azure-container-apps
spec:
  providerRef: Provider/volume-local         # Guest-local volume-local instance inside gateway Guest
  source:
    executionRef: Guest/<aca-gateway-name>   # guest-local; manifest-frozen; no Host fallback
    settings:
      kind: local-path
      sourcePolicyId: runtime-azure-container-apps-sandbox-state
  kind: state
  persistenceClass: persistent          # durable: survives component/Provider restart, upgrade, destroy/reset
  sensitivityClass: private
  stateSchema:
    schemaId: io.d2b.runtime-azure-container-apps/aca-controller/sandbox-state
    schemaVersion: "1.0"
    schemaDigest: sha256:<hex>
    migrationPolicy: pre-launch-required
  quotaBytes: 10485760                  # 10 MiB nonzero; opaque binding/adoption metadata only
  quota:
    maxBytes: 10485760
    maxInodes: 4096
    enforcement: none
  sealingCredentialRef: null
  layout:
    - path: ""
      type: directory
      ownerRef: User/d2b-aca-controller   # Nix-provisioned system user in gateway Guest
      groupRef: User/d2b-aca-controller
      mode: "0700"
      noFollow: true
      sensitivity: private
      accessAcl: []
      defaultAcl: []
      createPolicy: create-if-never-provisioned
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
      adoptionPolicy: quarantine-on-ambiguity
      restartPolicy: preserve-across-controller-restart
      leaseClass: none
      invariants: [no-symlink, broker-opaque-id-only]
    - path: sandboxes
      type: directory
      ownerRef: User/d2b-aca-controller
      groupRef: User/d2b-aca-controller
      mode: "0700"
      noFollow: true
      sensitivity: private
      accessAcl: []
      defaultAcl: []
      createPolicy: create-if-absent
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
      adoptionPolicy: adopt-with-live-owner-proof
      restartPolicy: preserve-across-controller-restart
      leaseClass: none
      invariants: [no-symlink, broker-opaque-id-only]
  views:
    main:
      path: ""
      rights: [read, write, create, delete, traverse]
  identityMarker:
    class: broker-maintained
    markerRoot: provider-state-markers
  snapshotPolicy: null
  retentionPolicy: null
```

Guest-local `volume-local` reconciles this Volume; all state stays inside the gateway Guest. The controller Process mounts this Volume (see §7.1 `mounts`); the framework delivers the `main`-view dirfd at startup.

#### 7.3.2 Deployment service state Volume

Empty payload schema; `migrationPolicy: none`; minimal quota for the identity marker only. No migration worker EphemeralProcess is ever created for this Volume.

```yaml
apiVersion: resources.d2b.io/v3
type: Volume
metadata:
  name: runtime-azure-container-apps--aca-deployment-service--service-state--<aca-gateway-name>
  zone: <zone>
  ownerRef: Provider/runtime-azure-container-apps
spec:
  providerRef: Provider/volume-local         # Guest-local volume-local instance inside gateway Guest
  source:
    executionRef: Guest/<aca-gateway-name>   # guest-local; manifest-frozen; no Host fallback
    settings:
      kind: local-path
      sourcePolicyId: runtime-azure-container-apps-service-state
  kind: state
  persistenceClass: persistent          # durable even for empty payload schema
  sensitivityClass: private
  stateSchema:
    schemaId: io.d2b.runtime-azure-container-apps/aca-deployment-service/service-state
    schemaVersion: "1.0"
    schemaDigest: sha256:<hex>
    migrationPolicy: none               # empty payload schema; no migration worker
  quotaBytes: 65536                     # 64 KiB; identity marker only
  quota:
    maxBytes: 65536
    maxInodes: 256
    enforcement: none
  sealingCredentialRef: null
  layout:
    - path: ""
      type: directory
      ownerRef: User/d2b-aca-deployment-service   # Nix-provisioned system user in gateway Guest
      groupRef: User/d2b-aca-deployment-service
      mode: "0700"
      noFollow: true
      sensitivity: private
      accessAcl: []
      defaultAcl: []
      createPolicy: create-if-never-provisioned
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
      adoptionPolicy: quarantine-on-ambiguity
      restartPolicy: preserve-across-controller-restart
      leaseClass: none
      invariants: [no-symlink, broker-opaque-id-only]
  views:
    main:
      path: ""
      rights: [read, write, create, delete, traverse]
  identityMarker:
    class: broker-maintained
    markerRoot: provider-state-markers
  snapshotPolicy: null
  retentionPolicy: null
```

Guest-local `volume-local` reconciles this Volume; the identity marker is maintained inside the gateway Guest. The deployment service Process mounts this Volume (see §7.2 `mounts`); the framework delivers the `main`-view dirfd at startup.

#### 7.3.3 User resource provisioning

One Nix-provisioned system user per component, declared in the gateway Guest's system-core Nix module:

```nix
d2b.vms.<aca-gateway-name>.config.users.users."d2b-aca-controller" = {
  isSystemUser = true;
  group = "d2b-aca-controller";
};
d2b.vms.<aca-gateway-name>.config.users.groups."d2b-aca-controller" = {};

d2b.vms.<aca-gateway-name>.config.users.users."d2b-aca-deployment-service" = {
  isSystemUser = true;
  group = "d2b-aca-deployment-service";
};
d2b.vms.<aca-gateway-name>.config.users.groups."d2b-aca-deployment-service" = {};
```

The volume-local Provider running inside the gateway Guest resolves each `User/<name>` to its provisioned uid/gid via the Guest's own User resource registry when performing layout operations. NSS is used only for verification; no PAM session or shell is granted. Because storage is guest-local, no Host-side User resource or uid/gid mapping is involved.

---

## 8 Async effect port

The ACA controller drives all Azure API operations exclusively through two injected async traits inside the deployment service. Both are constructor-injected into the deployment service; no implementation is instantiated via ambient discovery, SDK default chain, or environment variable. The controller calls the deployment service over d2b-bus; all ACA API I/O remains inside the deployment service process.

### 8.1 `AcaCredentialLeaseClient`

```rust
// packages/d2b-contracts/src/provider_effects/aca.rs
// Shared d2b-contracts provider-effects module; no separate d2b-aca-contracts crate.
pub trait AcaCredentialLeaseClient: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    async fn acquire(&self, request: &AcaCredentialLeaseRequest)
        -> Result<AcaCredentialLease, AcaControlError>;
    async fn revoke(&self, lease: &AcaCredentialLease)
        -> Result<(), AcaControlError>;
}
```

Returns opaque `AcaCredentialLease` — no token bytes are returned. `AcaCredentialLease` carries only the `CredentialLease` metadata handle.

### 8.2 `AcaControl`

```rust
// packages/d2b-contracts/src/provider_effects/aca.rs
// Shared d2b-contracts provider-effects module; no separate d2b-aca-contracts crate.
pub trait AcaControl: Send + Sync {
    async fn health(
        &self, lease: &AcaCredentialLease, context: &AcaControlContext,
    ) -> Result<AcaControlHealth, AcaControlError>;

    async fn find_sandboxes(
        &self, lease: &AcaCredentialLease, context: &AcaControlContext,
        query: &AcaWorkloadQuery,
    ) -> Result<AcaSandboxCandidates, AcaControlError>;

    async fn find_disk_images(
        &self, lease: &AcaCredentialLease, context: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageCandidates, AcaControlError>;

    async fn resolve_configured_disk(
        &self, lease: &AcaCredentialLease, context: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageRecord, AcaControlError>;

    async fn create_disk_image(
        &self, lease: &AcaCredentialLease, context: &AcaControlContext,
        desired: &AcaDesiredDiskImage,
    ) -> Result<AcaDiskImageRecord, AcaControlError>;

    async fn create_sandbox(
        &self, lease: &AcaCredentialLease, context: &AcaControlContext,
        desired: &AcaDesiredSandbox,
    ) -> Result<AcaSandboxRecord, AcaControlError>;

    async fn resume_sandbox(
        &self, lease: &AcaCredentialLease, context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError>;

    async fn stop_sandbox(
        &self, lease: &AcaCredentialLease, context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaSandboxRecord, AcaControlError>;

    async fn delete_sandbox(
        &self, lease: &AcaCredentialLease, context: &AcaControlContext,
        sandbox_id: &AcaSandboxId,
    ) -> Result<AcaDeleteOutcome, AcaControlError>;
}
```

All methods are:
- fully async; no `block_on` or nested runtime;
- bounded by `AcaControlContext.deadline_remaining_ms`;
- fail-closed on `AcaControlErrorKind::Ambiguous` — the caller must not assume partial success;
- do not return Azure subscription IDs, resource group ARMs, management hostnames, or any data that functioned as a credential;
- return `progressing` or a `requeue-at` response when the cloud operation is still in flight, so the deployment service never blocks a watch read loop waiting for an ACA API to complete.

`AcaSandboxId` and `AcaDiskImageId` are opaque bounded identifiers (`max 60 chars`; `[a-z0-9-]`) with redacted `Debug` impls. They are the controller's internal adoption keys; they are never written to resource status, audit records, or OTEL spans.

### 8.3 Operation class to API method mapping

| `AcaCredentialPurpose` | Required SDK operation classes | `AcaControl` methods used |
| --- | --- | --- |
| `Health` | Authenticate, Read | `health` |
| `Ensure` | Authenticate, Discover, Read, Create | `find_sandboxes`, `find_disk_images`, `resolve_configured_disk`, `create_disk_image`, `create_sandbox` |
| `Start` | Authenticate, Discover, Read, Power | `find_sandboxes`, `resume_sandbox` |
| `Stop` | Authenticate, Discover, Read, Power | `find_sandboxes`, `stop_sandbox` |
| `Inspect` | Authenticate, Discover, Read | `find_sandboxes`, `find_disk_images` |
| `Adopt` | Authenticate, Discover, Read | `find_sandboxes` |
| `Destroy` | Authenticate, Discover, Read, Delete | `find_sandboxes`, `delete_sandbox` |

---

## 9 State, idempotency, and adoption

### 9.1 Sandbox state machine

`AcaSandboxLifecycle` is the observed side-effect state of the remote ACA sandbox. It does not replace the Guest resource phase.

```
Provisioning → Ready → Running ↔ Idle → Stopping → Stopped
                                ↓                      ↓
                               Failed ←————————————→ Deleted
                               Unknown
```

| State | Meaning |
| --- | --- |
| `Provisioning` | ACA API accepted create; sandbox not yet healthy |
| `Ready` | Sandbox healthy; not yet accepting guest-control sessions |
| `Running` | Sandbox accepting ComponentSession connections |
| `Idle` | Sandbox suspended due to `autoSuspendSecs`; resume required |
| `Stopping` | Stop command accepted; sandbox draining |
| `Stopped` | Sandbox stopped; resume available |
| `Failed` | Sandbox entered terminal failure; must delete and re-create |
| `Deleted` | Sandbox no longer exists at ACA |
| `Unknown` | Lifecycle state cannot be determined; inspect required |

The controller maps `AcaSandboxLifecycle` to `Guest.status.providerPhase` using stable lower-kebab-case labels: `provisioning`, `ready`, `running`, `idle`, `stopping`, `stopped`, `failed`, `deleted`, `unknown`.

### 9.2 Core Operation ledger adapter

The provider reads operation/requeue state exclusively through the core Operation ledger adapter. The provider does **not** own an `operation-ledger` `ProviderStateSet` compartment; the core Operation ledger owns operation/requeue truth. The `operation-ledger` stateNamespace has been removed from the controller component descriptor.

The provider retains only the `sandbox-state` Volume (opaque ACA binding/adoption metadata — no credential bytes, endpoint URLs, or poll URLs). The controller persists ACA sandbox binding identifiers and adoption keys in the `sandbox-state` Volume's `main` view to survive daemon restarts; the framework delivers the view dirfd at startup; no bytes are exposed in resource status, audit records, or telemetry.

Each completed operation context is keyed by `OperationId` in the core ledger and carries:

- `ProviderOperationContext` (method, operation ID, expiry);
- `CachedResponse` (Plan/Handle/Observation/Receipt variant);
- `expires_at_unix_ms` (the lease expiry, not a secret);
- `observation_satisfied` flag.

Entries with `RetryClass::SameOperation` are not recorded. When the ledger reaches capacity (`completedOperationCapacity`; 1..1024; default 512), the oldest expired entry is evicted.

### 9.3 Idempotency contract

All seven controller methods (`RuntimePlan`, `RuntimeEnsure`, `RuntimeStart`, `RuntimeStop`, `RuntimeInspect`, `RuntimeAdopt`, `RuntimeDestroy`) are idempotent within the operation ledger TTL (`planTtlMs` for Plan; fixed 10 min for others):

- A second call with the same `OperationId` returns the cached result without re-driving the `AcaControl` port.
- `AcaControlErrorKind::Ambiguous` is never cached; the caller retries with a new operation ID.
- `AcaControlErrorKind::Conflict` produces a cached failure; the controller transitions the Guest to `Degraded` and sets `GuestProvisioned=False` with `reason: provider-error`.

### 9.4 Adoption

The `RuntimeAdopt` method reconciles a Guest resource that was created with a prior Provider generation or by a prior controller restart:

1. Acquire a `Adopt`-purpose credential lease.
2. Call `find_sandboxes` with the `AcaWorkloadQuery` derived from the current `AcaResourceBinding` (realm/zone ID, workload/guest UID, provider generation, configuration fingerprint).
3. If exactly one candidate matches by binding: record `AcaSandboxRecord` in the `sandbox-state` Volume `main` view (via framework-delivered dirfd), transition sandbox lifecycle to observed state, set `status.bootstrapReady = false` until the enrolled ComponentSession is re-established.
4. If zero candidates match: the controller proceeds with fresh `RuntimeEnsure`.
5. If multiple candidates match: set `Guest.status.phase = Degraded` with `reason: ambiguous-adoption`, emit a `critical`-severity bounded audit record (`AuditEventKind::GuestAdoptionAmbiguous`), and requeue. Do not proceed with any candidate.

Adoption is driven on every controller restart before any `RuntimeEnsure` is attempted.

---

## 10 d2b-bus service methods

The `d2b.aca.v3.deployment` service schema fingerprint is frozen at Provider package digest. No method may be added without a minor version increment and Provider schema update.

| Method | Verb | Direction | Description |
| --- | --- | --- | --- |
| `GuestProvision` | invoke | request/reply | Plan and ensure disk image + sandbox; returns bounded `ProviderHandle` |
| `GuestStart` | invoke | request/reply | Resume or start sandbox; returns updated `ProviderObservation` |
| `GuestStop` | invoke | request/reply | Stop sandbox; returns `MutationReceipt` |
| `GuestDestroy` | invoke | request/reply | Stop and delete sandbox; returns `MutationReceipt` |
| `GuestAdopt` | invoke | request/reply | Adopt existing sandbox by binding; returns `AdoptionState` |
| `GuestInspect` | invoke | request/reply | Observe current sandbox state; returns `ProviderObservation` |
| `GuestHealth` | invoke | request/reply | Check ACA environment reachability; returns `AcaControlHealth` |

All methods:
- carry a `ProviderCallContext` with monotonic deadline and operation binding;
- are RBAC-authorized before dispatch; unauthorized calls return `ErrorCode::PermissionDenied` without leaking state;
- are delivered via d2b-bus `ComponentSession`; no direct Unix path, socket, or fd is exposed;
- produce no log lines, trace spans, or metric events containing sandbox IDs, Azure resource identifiers, or lease handles.

---

## 11 RBAC

### 11.1 Required role bindings

All Role/RoleBinding subjects for this Provider are Process resources with `executionRef: Guest/<aca-gateway-name>` — that is, inside the gateway Guest. No Host-domain subject is granted any authority from this Provider. No RoleBinding subject has `executionRef: Host/*`.

The `Provider/runtime-azure-container-apps` controller Process requires the following Role/RoleBinding resources in its Zone:

```yaml
# Role — controller resource authority
apiVersion: resources.d2b.io/v3
type: Role
metadata:
  name: aca-controller-role
  zone: <zone>
spec:
  rules:
    - resourceTypes: [Guest]
      verbs: [get, list, watch, update-spec, update-status, delete]
      providerRef: Provider/runtime-azure-container-apps
    - resourceTypes: [Credential]
      verbs: [get, update-status]
      providerRef: Provider/credential-managed-identity
    - resourceTypes: [ZoneLink]
      verbs: [get]
      resourceNames: ["<zone-link-name>"]
```

```yaml
# RoleBinding — binds Role to controller component Process subject inside the gateway Guest
apiVersion: resources.d2b.io/v3
type: RoleBinding
metadata:
  name: aca-controller-binding
  zone: <zone>
spec:
  roleRef: Role/aca-controller-role
  subjects:
    - kind: process
      componentRef: aca-controller
      providerRef: Provider/runtime-azure-container-apps
      executionRef: Guest/<aca-gateway-name>    # inside gateway Guest; no Host subject permitted
```

### 11.2 Deployment service authority

The `aca-deployment-service` component may only invoke methods on the `d2b.aca.v3.deployment` service (including `GuestHealth`). It must not hold a `Guest` resource write verb; the controller owns all status and spec mutations. No Role or RoleBinding is required beyond the framework-default service component policy.

---

## 12 Transport dependencies

### 12.1 ZoneLink placement and requirements

The ZoneLink connecting to managed ACA sandboxes originates from the **gateway Guest** (`spec.config.gatewayExecutionRef`), not from the Host. The Host has no ZoneLink for this Provider and no direct path to Azure Container Apps APIs. This is the correct layering for ADR-032 compliance: the azure relay transport credential and relay connection state live inside the gateway Guest, not on the Host.

A `Provider/runtime-azure-container-apps`-backed Guest uses a `ZoneLink` resource to connect the gateway Guest runtime to the remote ACA sandbox. The ZoneLink:

- is declared by the operator via Nix, scoped to the Zone;
- references `Provider/transport-azure-relay` as its `transportProviderRef`;
- carries the enrolled KK static public key fingerprint of the ACA sandbox agent component;
- requires the `aca-relay` dependency alias declared in the Provider manifest to resolve this ZoneLink at runtime inside the gateway Guest.
- its `relayCredentialRef` must resolve to a Credential resource whose `scope.executionRef` matches `config.gatewayExecutionRef`.

```yaml
apiVersion: resources.d2b.io/v3
type: ZoneLink
metadata:
  name: aca-relay-link
  zone: <zone>
spec:
  transportProviderRef: Provider/transport-azure-relay
  childStaticKeyFingerprint: "sha256:<64-hex-chars>"
  childZoneName: null      # ACA-backed zones are leaf zones; no further child Zone hierarchy
  relayNamespace: "d2b-relay"
  relayHybridConnection: "aca-<zone-name>"
  transportSettings:
    relayCredentialRef: "Credential/aca-managed-identity"
```

### 12.2 Noise profiles for remote sessions

All ComponentSession connections between the gateway Guest's ACA controller and the ACA sandbox agent use `Noise_KK_25519_ChaChaPoly_SHA256`:

- Both static public keys are known before handshake: the local Provider/controller key from the gateway Guest's KK registry; the ACA sandbox agent key from `spec.childStaticKeyFingerprint`.
- No NN session is permitted for cross-ZoneLink channels.
- Bootstrap IKpsk2 is used only for first enrollment of a newly created sandbox; the PSK is bound to the `GuestProvision` operation ID, the sandbox UID, and a bounded expiry; it is consumed exactly once.
- Replay, expiry, wrong operation/subject/purpose: fail closed.

### 12.3 No Host-level Azure transport from this Provider

The Host holds **no** process, credential, socket, or transport binding for Azure Container Apps from this Provider. Specifically:

- No Host Process with `providerRef: Provider/runtime-azure-container-apps`
- No Host-scoped Credential (`scope.executionRef: Host/*`) for this Provider
- No Azure management HTTPS socket opened on the Host by this Provider
- No ZoneLink connecting the Host directly to the managed ACA sandbox through this Provider

The controller (`aca-controller`) Process, running inside the gateway Guest, holds no Azure management SDK HTTP client, no HTTPS socket to `management.azure.com`, and no direct path to ACA APIs in the Host network namespace. All Azure API calls are mediated by the co-located `AcaControl` implementation (which is instantiated by the Provider supervisor ticket inside the gateway Guest, not by the controller itself). The controller's only outbound IPC surfaces are:

- d2b-bus ComponentSession to credential and ZoneLink transport Providers (both inside the gateway Guest);
- d2b-bus ComponentSession to the `aca-deployment-service` (also inside the gateway Guest);
- Volume read/write via kernel filesystem operations inside the gateway Guest.

---

## 13 Status, errors, audit, and OTEL redaction

### 13.1 Guest status exposed fields

| Field | Allowed content | Forbidden content |
| --- | --- | --- |
| `status.providerPhase` | One of: `provisioning`, `ready`, `running`, `idle`, `stopping`, `stopped`, `failed`, `deleted`, `unknown` | Azure resource ID, ACA sandbox URL, container name, management ARM path |
| `status.guestIdentityDigest` | SHA-256 hex of `(sandboxId_bytes \|\| providerGeneration_be64 \|\| configFingerprint_bytes)` | The raw sandbox ID string, container group name, subscription scope |
| `status.bootstrapReady` | Boolean | Any credential byte or token prefix |
| `status.activeProcessCount` | Integer count of non-terminal Process resources targeting this Guest | — |
| Condition messages | Bounded stable reason codes (max 256 chars); no dynamic data | Token values, ARM IDs, sandbox hostnames, internal error messages |

### 13.2 Error codes (stable, bounded)

`AcaControlErrorKind` maps to stable Provider error codes:

| Kind | Stable code | Retry eligible |
| --- | --- | --- |
| `Authentication` | `aca-control-authentication` | No |
| `Authorization` | `aca-control-authorization` | No |
| `RateLimited` | `aca-control-rate-limited` | Yes; honor `retry_after_ms` |
| `Unavailable` | `aca-control-unavailable` | Yes; exponential backoff |
| `Conflict` | `aca-control-conflict` | No; requires reconcile |
| `NotFound` | `aca-control-not-found` | No; adoption required |
| `InvalidResponse` | `aca-control-invalid-response` | No; fail closed |
| `Cancelled` | `aca-control-cancelled` | Caller decision |
| `DeadlineExpired` | `aca-control-deadline-expired` | Caller decision |
| `Ambiguous` | `aca-control-ambiguous` | Caller must use new operation ID |

Error strings are bounded to these stable codes. No Azure HTTP status line, ARM response body, response header, or SDK diagnostic is propagated to any public surface.

### 13.3 Audit records

The controller emits bounded audit records for the following events:

| Event kind | Required fields | Forbidden fields |
| --- | --- | --- |
| `GuestProvisionStarted` | `zone`, `guest_uid`, `operation_id`, `provider_generation`, `config_fingerprint_digest` | Sandbox ID, ARM IDs, image URLs, token bytes |
| `GuestProvisionCompleted` | Same + `outcome: success\|failure`, `error_code` (stable) | — |
| `GuestAdoptionAmbiguous` | `zone`, `guest_uid`, `candidate_count` (integer), `operation_id` | Any candidate ID string |
| `GuestDestroyStarted` | `zone`, `guest_uid`, `operation_id` | Sandbox ID |
| `GuestDestroyCompleted` | Same + `outcome`, `error_code` | — |
| `CredentialLeaseCleanup` | `component: credential-lease`, `operation: revoke`, `outcome` (stable string) | Lease handle, token bytes |

All audit records commit to the Zone audit stream before the operation they describe completes. Audit records with `AuditDurabilityClass::Critical` (adoption-ambiguous, destroy-completed, provision-completed) fail the operation closed if the commit fails.

### 13.4 Deletion/finalization audit contract

The finalization path follows the cleanup normalization from the resource store contract (ref `c8f47656`):

1. **Finalizer check**: If the Guest resource has any active finalizers, the controller transitions to `Pending/Degraded` with `reason: active-finalizers`. Finalization is not forced. There is no `--force-remove-finalizers` CLI or broker op.

2. **Final store transaction**: Writes an event-only `Deleted` revision (no spec/status body) and atomically removes the resource row and all its indexes in a single transaction. `managedBy` is set to `"configuration"` (not `"nix"`) in the final revision event.

3. **Post-commit audit append**: After the store transaction commits successfully, the controller appends `ResourceMutation { event: "deleted", trigger: "config-cleanup", zone: <zone_uid>, resource_uid: <guest_uid> }` to the Zone audit log using a dedup/exactly-once recovery key `(resource_uid, "deleted", generation)`. If the append fails, the controller retries using the recovery key; the record is appended exactly once even if the controller restarts after the store commit.

4. **Revocation before finalization**: All active Credential leases are revoked, the ACA sandbox is stopped and the ACA resource deleted via `AcaControl.delete_sandbox`, the enrolled KK ComponentSession is closed, and the ZoneLink transport is released — all before the final store transaction.

5. **Generation retention**: Audit revision records use count-based retention (default 3, range 1..16). No TTL-based retention. The count applies to the resource's audit revision trail, not to the ACA resource history.

No `AuditDurabilityClass::Critical` deletion record is committed as part of the store transaction; the commit is event-only. The post-commit audit append is separate and recovery-idempotent.

### 13.4 OTEL metric labels

Closed allowlist for all metrics emitted by `d2b-provider-runtime-azure-container-apps`:

| Allowed label | Values |
| --- | --- |
| `provider` | `runtime-azure-container-apps` (literal) |
| `component` | `aca-controller`, `aca-deployment-service` |
| `operation` | stable method name: `provision`, `start`, `stop`, `inspect`, `adopt`, `destroy`, `health` |
| `outcome` | `success`, `failure`, `cancelled`, `deadline-expired` |
| `error` | stable error code from §13.2, or `none` |

No metric label may carry: Zone name, Guest resource name, sandbox ID, ACA environment ID, subscription ID, tenant ID, client ID, token audience, credential lease handle, container group name, image name, or any value derived from an Azure API response.

### 13.5 OTEL trace span attributes

Spans emitted by the controller carry only:

- `d2b.provider = "runtime-azure-container-apps"`
- `d2b.component` = one of the two component IDs (`aca-controller`, `aca-deployment-service`)
- `d2b.operation` = stable operation name
- `d2b.outcome` = `success|failure|cancelled`

No span carries the Azure sandbox ID, ARM resource path, management hostname, subscription scope, or credential-adjacent value. Trace context (`traceparent`/`tracestate`) is propagated over the ZoneLink ComponentSession channel as opaque bytes; it does not carry identity material.

---

## 14 Quotas, backoff, and performance

### 14.1 ACA resource bounds

| Resource | Minimum | Maximum | Quantum/step |
| --- | --- | --- | --- |
| CPU | 250 millicpus | 4000 millicpus | 250 millicpus |
| Memory | 512 MiB | 16384 MiB | 256 MiB |
| Auto-suspend | 60 s | 86400 s (24 h) | 1 s |
| Sandbox ID length | — | 60 chars | — |
| Disk/image/profile ID length | — | 64 chars | — |
| Candidates per query | 0 | 8 | — |
| Completed operation ledger | 1 | 1024 entries | — |
| Operation plan TTL | 1 ms | 300000 ms (5 min) | — |
| Readiness poll attempts | 1 | 60 | — |
| Readiness poll interval | 1 ms | 10000 ms (10 s) | — |
| Credential lease cleanup timeout | — | 1000 ms | — |

### 14.2 Retry and backoff

Rate-limited responses (`AcaControlErrorKind::RateLimited`) carry an optional `retry_after_ms` bounded to `MAX_ACA_RETRY_AFTER_MS = 300000` (5 min). The controller:

- honors the provider-supplied `retry_after_ms` when present;
- applies exponential backoff with jitter capped at 5 min when no hint is given: `min(max_backoff, base * 2^attempt + jitter)` where `base = 1000ms`, `max_backoff = 300000ms`;
- requeues the Guest resource with `requeue_at = now + retry_after_ms`;
- propagates `RetryClass::SameOperation` for transient failures so the caller can retry with the same operation ID within the ledger TTL.

`AcaControlErrorKind::Unavailable` applies the same exponential backoff. `AcaControlErrorKind::Conflict` and `AcaControlErrorKind::NotFound` are not retried without a spec generation change.

### 14.3 Controller process concurrency

The `aca-controller` component descriptor declares:

- `reconcileConcurrency: 4` — at most 4 Guest resources reconcile concurrently;
- `observeConcurrency: 8` — at most 8 observe calls concurrent with reconcile;
- `maxPendingResources: 256` — controller rejects new hints when backlog exceeds this threshold.

### 14.4 Credential lease budget

A single long-lived credential lease is acquired at the start of each reconcile or observe step and revoked before the step returns. Lease lifetime is capped at `deadline_remaining_ms`. Revocation failure triggers a background cleanup job with its own `MAX_ACA_LEASE_CLEANUP_MS = 1000` timeout. The cleanup job queue is bounded; when the queue is full, the job is dropped and an `outcome: saturated` event is logged.

---

## 15 Nix artifact and configuration

### 15.1 Artifact declaration

```nix
# nixos-modules (or consumer flake)
d2b.artifacts.provider-runtime-aca = {
  package = inputs.d2b-provider-runtime-aca.packages.${system}.default;
  type = "provider";
};
```

The `artifactId = "provider-runtime-aca"` string is used in the Provider ResourceSpec. It is a plain bounded ID; it is not a ResourceRef and does not appear in resource status or audit records.

### 15.2 Provider resource

```nix
d2b.zones.my-zone.resources = {
  runtime-azure-container-apps = {
    type = "Provider";
    spec = {
      artifactId = "provider-runtime-aca";
      config = {
        gatewayExecutionRef = "Guest/aca-gateway";    # required; all Processes run inside this Guest
        tenantId = "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx";
        clientId = "yyyyyyyy-yyyy-yyyy-yyyy-yyyyyyyyyyyy";
        subscriptionId = "zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz";
        controlCredentialRef = "Credential/aca-managed-identity";
        environmentId = "env-xxxxxxxxxxxxxxxxxxxxxxxx";
        resourceGroupId = "rg-workloads";
        defaults = {
          cpuMillis = 500;
          memoryMiB = 2048;
          autoSuspendSecs = 300;
          planTtlMs = 300000;
        };
        readiness = {
          attempts = 30;
          intervalMs = 5000;
        };
        completedOperationCapacity = 512;
        zoneLinkAlias = "aca-relay";
      };
    };
  };
};
```

### 15.3 Credential resource

```nix
d2b.zones.my-zone.resources = {
  aca-managed-identity = {
    type = "Credential";
    spec = {
      providerRef = "Provider/credential-managed-identity";
      scope = {
        # Must match spec.config.gatewayExecutionRef — scoped to gateway Guest, never Host
        executionRef = "Guest/aca-gateway";
        domainFilter = "system";
      };
      audience = "https://management.azure.com/";
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
};
```

### 15.4 ZoneLink resource

The ZoneLink is declared in the Zone that contains the gateway Guest. The relay credential must be scoped to the gateway Guest (matching `gatewayExecutionRef`). The ZoneLink connects the gateway Guest to the managed ACA sandboxes — it is NOT a Host-to-sandbox link.

```nix
d2b.zones.my-zone.resources = {
  aca-relay-link = {
    type = "ZoneLink";
    spec = {
      transportProviderRef = "Provider/transport-azure-relay";
      childStaticKeyFingerprint = "sha256:<64-hex-chars>";
      transportSettings = {
        relayNamespace = "d2b-relay";
        relayHybridConnection = "aca-my-zone";
        # Relay credential must also have scope.executionRef = "Guest/aca-gateway"
        relayCredentialRef = "Credential/aca-managed-identity";
      };
    };
  };
};
```

### 15.5 Guest resource

```nix
d2b.zones.my-zone.resources = {
  aca-sandbox = {
    type = "Guest";
    spec = {
      providerRef = "Provider/runtime-azure-container-apps";
      defaultDomain = "system";
      allowedDomains = [ "system" ];
      systemArtifactId = null;  # null for cloud Guests
      providerSettings = {
        configuredDiskId = "img-xxxxxxxxxxxxxxxxxxxxxxxx";
        cpuMillis = 500;
        memoryMiB = 2048;
        autoSuspendSecs = 300;
        sandboxIdentityBindingId = "mid-xxxxxxxxxx";
      };
    };
  };
};
```

### 15.6 Gateway Guest Nix declaration

The `aca-gateway` Guest VM that hosts all ACA controller Processes must be declared separately from the managed ACA sandboxes. It is a local VM Guest backed by a local VM Provider (e.g., `runtime-cloud-hypervisor`):

```nix
d2b.zones.my-zone.resources = {
  # Gateway Guest — hosts all ACA Provider component Processes
  aca-gateway = {
    type = "Guest";
    spec = {
      providerRef = "Provider/runtime-cloud-hypervisor";    # or equivalent local VM Provider
      defaultDomain = "system";
      allowedDomains = [ "system" ];
      systemArtifactId = "aca-gateway-system";             # NixOS system closure for the gateway VM
    };
  };
};
```

The `aca-gateway` Guest's NixOS system closure must include the two ACA Provider component binaries (`d2b-aca-controller`, `d2b-aca-deployment-service`) and their runtime dependencies. Component principals (`ComponentPrincipal/aca-controller`, `ComponentPrincipal/aca-deployment-service`) are assigned by `Provider/system-minijail` at Process start time — they are not OS accounts and require no `users.users.*` NixOS declarations. The `gatewayExecutionRef = "Guest/aca-gateway"` in the Provider config ties all controller Processes to this specific Guest resource.

### 15.7 Eval-time validation

The Nix compiler enforces at eval time:

- `config.gatewayExecutionRef` resolves to a declared `Guest` resource in the same Zone; rejected with assertion if absent or resolves to `Host/*`;
- `config.controlCredentialRef` resolves to a declared `Credential` resource in the same Zone whose `scope.executionRef` matches `config.gatewayExecutionRef`; any mismatch is rejected with `credential-execution-mismatch` assertion;
- `config.pullCredentialRef`, if present, has the same `scope.executionRef` constraint as `controlCredentialRef`;
- `spec.systemArtifactId` is `null` when `spec.providerRef = "Provider/runtime-azure-container-apps"`;
- `spec.allowedDomains` does not include `user` for ACA-backed Guests (unsupported; rejected with a clear assertion message);
- exactly one of `configuredDiskId` or `configuredImageId` is set in `providerSettings`;
- `diskName` is present when `configuredImageId` is set;
- all `providerSettings` string IDs match the `^[a-z][a-z0-9-]*$` pattern and respect length bounds;
- `config.zoneLinkAlias` is declared in the Provider manifest's dependency aliases.

No Host-scoped Credential (`scope.executionRef` matching `Host/*`) is accepted for `controlCredentialRef` or `pullCredentialRef`. The assertion fires at eval time with a descriptive message that names the offending credential resource and the required `gatewayExecutionRef`.

---

## 16 Lifecycle and upgrades

### 16.1 Normal lifecycle

```
Provider Pending
  → package/trust/conformance check (Nix eval-time for digest; runtime for conformance)
  → gatewayExecutionRef Guest becomes Ready (prerequisite; Provider stays Pending until met)
  → ProviderDeployment creates controller and deployment-service Processes INSIDE gateway Guest
  → Core ProviderDeployment creates one Volume per component (aca-controller sandbox-state + aca-deployment-service service-state; each providerRef=volume-local, source.executionRef=Guest/<aca-gateway-name>, guest-local, no attachments[]; ownerRef: Provider/runtime-azure-container-apps) before Processes start
  → Provider Ready
  → Guest resources reconciled by aca-controller (running inside gateway Guest)
```

The Host is not involved in any of these steps beyond its standard hypervisor relationship to the gateway Guest. If the gateway Guest transitions to `Degraded` or `Stopped`, the Provider transitions to `Degraded` with `reason: gateway-guest-unavailable`. There is no fallback to running any controller Process on the Host.

### 16.2 Provider generation change

A Provider generation change (new package digest, config update, or credential ref change) triggers:

1. Controller drains in-flight reconcile operations (bounded `DRAIN_TIMEOUT = 30 s`).
2. All active Credential leases are revoked (per `revocation.onProviderGeneration = immediate`).
3. ProviderDeployment replaces controller and deployment-service Processes under the new generation.
4. The operation ledger is retained across generation changes; entries bound to old generation IDs remain valid for their remaining TTL but are not returned to callers on new-generation operations.
5. All Guest resources receive a reconcile hint; each adopts or re-provisions under the new generation.

### 16.3 State schema migration

State schema version increments (`schemaVersion` major bump) require `migrationPolicy: pre-launch-required`. The controller Process is not started until the migration script (declared in the Provider package) completes successfully. A migration failure leaves the Provider in `Degraded` with `reason: state-migration-failed`; the old generation's Volumes are archived, not deleted.

### 16.4 Upgrade path from current code

The current `AcaWorkloadProvider` / `GuestControlEndpointProvider` implementation uses the v2 `WorkloadProvider` / `GuestControlEndpointProvider` traits and direct vsock guest-control. These traits have no compatibility window in d2b 3.0:

- The old vsock guest-control path is inert at d2b 3.0 cutover.
- `AcaRelayTransportConfig` is adapted into `ZoneLink` transport settings.
- The provider agent binary (`d2b-gateway-runtime/src/provider_agent.rs`) serves as reuse source for the deployment service component; the ACA-specific `aca_workload.rs` is excluded (see §17).

### 16.5 Removal gate

`packages/d2b-provider-aca/` is removed only after:

- `Provider/runtime-azure-container-apps` passes the full controller conformance suite;
- all test coverage from the old `d2b-provider-aca/src/tests.rs` is ported to `d2b-provider-runtime-azure-container-apps/tests/`;
- the migration map removal proof for `ADR046-aca-001` through `ADR046-aca-006` is complete.

---

## 17 Reuse ledger

All sources in this section are from main commit `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` unless otherwise noted. They are not currently present in the pre-ADR45 v3 baseline. Current v3 sources are from baseline `b5ddbed67867d9244bf33390868101bd9b053e49`.

| Source | Current location | Evidence class | Disposition | v3 Destination |
| --- | --- | --- | --- | --- |
| `AcaWorkloadProvider` + `GuestControlEndpointProvider impl` | `packages/d2b-provider-aca/src/lib.rs` | production-reachable | REPLACE | `d2b-provider-runtime-azure-container-apps/src/controller.rs` — new async `Guest` reconcile loop; vsock path retired |
| `AcaRelayTransportConfig` | `packages/d2b-provider-aca/src/lib.rs` (relay transport config) | production-reachable | ADAPT | ZoneLink transport settings under Provider config §15.4 |
| `AcaControl` trait (9 methods) | `packages/d2b-provider-runtime-azure-container-apps/src/control.rs` (main) | test-only at v3 baseline | RETAIN+ADAPT | Move to `packages/d2b-contracts/src/provider_effects/aca.rs` (shared provider-effects module; no new crate); no direct provider implementation dependency from core; adapt `OperationBinding` to v3 `ProviderOperationContext` contract |
| `AcaCredentialLeaseClient` trait | `packages/d2b-provider-runtime-azure-container-apps/src/control.rs` (main) | test-only at v3 baseline | RETAIN+ADAPT | Move to `packages/d2b-contracts/src/provider_effects/aca.rs`; adapt `CredentialLease` to v3 Credential resource model; provider crate remains one package |
| `AcaRuntimeConfig` / `AcaSandboxProfile` / bounds constants | `packages/d2b-provider-runtime-azure-container-apps/src/types.rs` (main) | test-only at v3 baseline | RETAIN+ADAPT | Adapt to v3 `providerSettings` schema fields; all bounds constants preserved |
| `AcaResourceBinding` / `AcaWorkloadQuery` | `packages/d2b-provider-runtime-azure-container-apps/src/types.rs` (main) | test-only at v3 baseline | ADAPT | Replace `RealmId`/`WorkloadId` fields with v3 `Zone`/`Guest` resource UID; retain redacted Debug |
| Operation ledger (`CompletedOperation`, `OperationLedger`) | `packages/d2b-provider-runtime-azure-container-apps/src/provider.rs` (main) | test-only at v3 baseline | ADAPT | Adapt operation ID type to v3; delegate to core Operation ledger adapter; provider has two state Volumes (sandbox-state + service-state), each providerRef=volume-local, source.executionRef=Guest/<aca-gateway-name>, guest-local, no attachments[]; core ProviderDeployment creates/deletes both; Volume absent from controller resourceTypes |
| Lease cleanup job/executor pattern | `packages/d2b-provider-runtime-azure-container-apps/src/provider.rs` (main) | test-only at v3 baseline | RETAIN | Retain `LeaseCleanupJob`/`LeaseCleanupExecutor`/`TracingLeaseCleanupObserver` verbatim; target tracing key unchanged |
| Retry/backoff (`AcaControlErrorKind` + `RetryClass`) | `packages/d2b-provider-runtime-azure-container-apps/src/control.rs` (main) | test-only at v3 baseline | RETAIN | Retain all error kind/diagnostic variants and `MAX_ACA_RETRY_AFTER_MS` |
| Provider agent process entry point | `packages/d2b-gateway-runtime/src/provider_agent.rs` (main) | production-reachable at main | COPY/ADAPT (partial) | Adapt `ProviderAgentProcess`/`run_registered`/`run` as deployment service binary skeleton; exclude `aca_workload.rs` |
| `AzureRelayTransportProvider` | `packages/d2b-provider-relay/src/lib.rs` (v3 baseline) | production-reachable | REPLACE | Moved to `Provider/transport-azure-relay`; ACA-specific relay config becomes ZoneLink `transportSettings` |
| v3 `d2b-provider-aca/src/tests.rs` | `packages/d2b-provider-aca/src/tests.rs` | test-only | EXTRACT+PORT | Port all test coverage to `packages/d2b-provider-runtime-azure-container-apps/tests/` |

**Excluded from reuse:**

- `packages/d2b-gateway-runtime/src/aca_workload.rs` — ACA-specific workload lifecycle using the main ACA Provider V2 registration path; not compatible with v3 resource model.
- `packages/d2b-daemon-access/src/relay.rs` (main) — relay credential format changed; v3 uses `d2b-provider-relay` ZoneLink format.
- `packages/d2b-gateway/` orchestrator — uses main's Zone model; excluded.
- Any main code that references `GUEST_SESSION_CREDENTIAL_*` handshake constants or `EndpointRole::GuestBootstrap`/`GuestDirect` — these are ADR45 guest bootstrap paths; v3 guest enrollment uses the ZoneLink resource model.

---

## 18 Implementation work items

### ADR046-aca-001

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-provider-001; runtime-aca owner |
| Current source | `packages/d2b-provider-aca/src/lib.rs`: `AcaWorkloadProvider`, 2841 lines production-reachable; `packages/d2b-provider-runtime-azure-container-apps/src/provider.rs`: `AzureContainerAppsRuntimeProvider`, 2796 lines (test-only at v3 baseline) |
| Reuse action | REPLACE (old) + ADAPT (main types/traits) |
| Destination | `packages/d2b-provider-runtime-azure-container-apps/src/controller.rs` |
| Detailed design | Async `Guest` reconcile loop: `describe` → `validateSpec` → `plan` → `reconcile` → `observe` → `finalize`. Adoption before first `RuntimeEnsure`. Operation ledger persisted to Volume inside gateway Guest. Credential lease acquire/revoke per call. Noise KK enrollment via ZoneLink (from gateway Guest to managed ACA sandbox). `providerPhase` and `guestIdentityDigest` in status; no raw endpoint/path in any status field. **ProviderDeployment creates both static Processes; ACA controller never instantiates its own Processes and never writes Provider status directly. All Processes run inside the gateway Guest. No Host Process, no Host Credential, no Host Azure HTTP socket. Long-running cloud ops return `progressing`/`requeue-at` immediately; never block watch loop.** |
| Integration | Zone ResourceClient → ProviderDeployment → Process launch inside gateway Guest → d2b-bus → deployment service |
| Data migration | Full reset; no v2 provider state compatibility |
| Validation | Controller conformance suite; adoption/ambiguity tests; deadline/cancellation matrix; redaction coverage; **gateway Guest placement validation: assert no Process has `executionRef: Host/*`**; Process spec field schema tests (`spec.template`, canonical `sandbox`/`budget`/`networkUsage`/`endpoints`/`readiness`/`restartPolicy` fields, `mounts` with `required: true`, `providerRef: Provider/system-minijail`); ProviderDeployment creates both Processes (controller never self-spawns); no raw endpoint/path in Guest status |
| Removal proof | `packages/d2b-provider-aca/` removed only after conformance suite green

### ADR046-aca-007

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-aca-001; Nix/gateway wiring owner |
| Current source | n/a — new requirement (gateway Guest placement) |
| Reuse action | NEW |
| Destination | `nixos-modules/` (gateway Guest declaration, Process template wiring, Credential scope assertion); eval-time validation module |
| Detailed design | Nix eval-time assertions for: (a) `gatewayExecutionRef` resolves to a `Guest` resource, not `Host/*`; (b) Credential `scope.executionRef` matches `gatewayExecutionRef`; (c) all Process templates emitted for this Provider have `executionRef` equal to `gatewayExecutionRef`. No `User` resource or `users.users.*` declarations required — component principals are framework-assigned and not OS accounts. Gateway Guest NixOS closure includes only the two ACA component binaries (§15.6). Assertion error messages name the offending resource and the required `gatewayExecutionRef`. |
| Integration | Nix eval gate; `d2b.zones.*.resources` validation pass; consumer flake usage example |
| Data migration | None |
| Validation | Nix eval assertion tests (wrong `executionRef` → assertion fires; correct setup → passes); §15.7 assertion coverage tests |
| Removal proof | n/a — ongoing eval-time constraint |

### ADR046-aca-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-aca-001; deployment service owner |
| Current source | `packages/d2b-gateway-runtime/src/provider_agent.rs` (main) |
| Reuse action | COPY/ADAPT (partial); exclude `aca_workload.rs` |
| Destination | `packages/d2b-provider-runtime-azure-container-apps/src/deployment_service.rs` |
| Detailed design | `ProviderAgentProcess`-shaped binary; bounded dispatch (64 in-flight); bounded audit ring (1024 capacity); shutdown within 5 s; serves `d2b.aca.v3.deployment` service schema including `GuestHealth` (health probing folded in from former health worker). All ACA API calls go through the injected `AcaControl` port — no ambient network call, no SDK default chain. Long-running ops return `progressing`/`requeue-at` to the caller; no blocking on Azure API completion. |
| Integration | ProviderDeployment spawns service; d2b-bus routes GuestProvision/Start/Stop/Destroy/Adopt/Inspect/Health methods |
| Data migration | None |
| Validation | Service dispatch matrix; RBAC refusal tests; redaction tests; shutdown deadline tests |
| Removal proof | Old `GuestControlEndpointProvider` dispatch removed per ADR046-aca-001 |

### ADR046-aca-003

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-aca-001; credential integration owner |
| Current source | `packages/d2b-provider-runtime-azure-container-apps/src/control.rs` (main): `AcaCredentialLeaseClient`, `AcaCredentialLease`, `AcaCredentialLeaseRequest`, `AcaCredentialPurpose` |
| Reuse action | RETAIN+ADAPT |
| Destination | `packages/d2b-contracts/src/provider_effects/aca.rs` (shared `d2b-contracts` provider-effects module; no new crate; provider crate remains one package) |
| Detailed design | `AcaCredentialLeaseClient`, `AcaCredentialLease`, `AcaCredentialLeaseRequest`, and `AcaCredentialPurpose` live in the shared `d2b-contracts` provider-effects module. Adapt `CredentialLease` to v3 Credential resource opaque lease handle. `AcaCredentialPurpose` maps to `allowedOperations` check against `Credential.spec`. Lease expiry capped at call deadline. Cleanup job pattern retained verbatim. |
| Integration | Controller acquires lease per reconcile step via injected `AcaCredentialLeaseClient`; raw token delivered only via Noise KK E2E channel through `d2b.credential.v3.AcquireToken` method |
| Data migration | None |
| Validation | Mock credential client tests; lease cleanup timeout tests; token non-exposure assertion |
| Removal proof | Old `CredentialProvider` trait deleted after `credential-managed-identity` Provider conformance |

### ADR046-aca-004

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-session-001; ZoneLink transport owner |
| Current source | `packages/d2b-provider-relay/src/lib.rs`: `AzureRelayTransportProvider`; `packages/d2b-provider-aca/src/lib.rs`: `AcaRelayTransportConfig` |
| Reuse action | REPLACE (both); ADAPT config fields to ZoneLink transport settings |
| Destination | ZoneLink resource `spec.transportSettings` (§15.4); `Provider/transport-azure-relay` dossier (separate) |
| Detailed design | `AcaRelayTransportConfig` fields mapped to `transportSettings.relayNamespace`, `.relayHybridConnection`, `.relayCredentialRef`. KK enrollment replaces ad-hoc vsock CID. |
| Integration | ZoneLink controller resolves transport Provider; ACA controller establishes enrolled KK ComponentSession over relay transport after `GuestProvision` |
| Data migration | No relay session compatibility; re-enroll on first `RuntimeAdopt` |
| Validation | ZoneLink enrollment tests; relay unavailability tests; KK re-enrollment after sandbox restart |
| Removal proof | `packages/d2b-provider-relay/` removed after `transport-azure-relay` Provider conformance |

### ADR046-aca-005

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-aca-001; state/migration owner |
| Current source | `packages/d2b-provider-runtime-azure-container-apps/src/types.rs` (main): `AcaRuntimeConfig`, `AcaSandboxProfile`, `AcaResourceBinding`, `AcaWorkloadQuery` — test-only at v3 baseline |
| Reuse action | RETAIN+ADAPT |
| Destination | `packages/d2b-provider-runtime-azure-container-apps/src/types.rs` |
| Detailed design | Replace `RealmId`/`WorkloadId` with v3 `Zone`/`Guest` UID types. `AcaResourceBinding` keys the adoption query. One provider-owned Volume per component: controller holds opaque ACA binding/adoption metadata in `sandbox-state` Volume; deployment service holds empty schema in `service-state` Volume. Each Volume: `providerRef: Provider/volume-local`, `source.executionRef: Guest/<aca-gateway-name>` (guest-local; manifest-frozen; no Host fallback; no `attachments[]`); Guest-local `volume-local` instance reconciles. Layout owners: `User/d2b-aca-controller` and `User/d2b-aca-deployment-service` (Nix-provisioned in gateway Guest; no `ComponentPrincipal` ResourceRef). Both Processes mount their own Volume's `main` view (`required: true`); framework delivers view dirfd. Core `ProviderDeployment` creates/deletes both Volumes (not the ACA controller); `Volume` absent from controller `resourceTypes`; `Provider/volume-local` sole reconciler. Operation/requeue state delegated to core Operation ledger adapter; `operation-ledger` stateNamespace removed. No cross-component Volume sharing. Host never holds cloud binding, admission, PSK, or operation state. |
| Integration | Core ProviderDeployment creates both Volumes (sandbox-state + service-state; each `providerRef=volume-local`, `source.executionRef=Guest/<aca-gateway-name>`, guest-local, no `attachments[]`) before Processes start; Guest-local Provider/volume-local reconciles; `stateSchemaPhase: current` and Volume `status.ready: true` gate Process launch |
| Data migration | sandbox-state v1.0 initial; `migrationPolicy: pre-launch-required`; service-state `migrationPolicy: none` (empty payload; no migration worker) |
| Validation | sandbox-state Volume round-trip (framework-delivered dirfd; `main` view); service-state Volume present with `migrationPolicy: none` and `quotaBytes: 65536`; adoption from persisted binding test; `kind: state`, `persistenceClass: persistent`, nonzero `quotaBytes`, broker-maintained `identityMarker` for both Volumes; `source.executionRef: Guest/<aca-gateway-name>` on each Volume; no `attachments[]` on any Volume; `Volume` absent from controller `resourceTypes`; controller does not create/update/watch Volume resources; `Provider/volume-local` sole reconciler; no ComponentPrincipal ResourceRef; `ProviderDeployment` creates/deletes Volumes; core Operation ledger adapter integration test |
| Removal proof | Old in-memory-only operation ledger removed after core Operation ledger adapter passes; `operation-ledger` stateNamespace absent from component descriptor |

### ADR046-aca-006

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-aca-001; Nix/telemetry owner |
| Current source | `nixos-modules/options-realms-workloads.nix`: `kind = "ProviderManaged"` → ACA; `packages/d2b-provider-aca/src/lib.rs`: tracing fields |
| Reuse action | REPLACE (Nix emitter) + ADAPT (metric/audit shapes) |
| Destination | `nixos-modules/` (generated Guest resource options); `packages/d2b-provider-runtime-azure-container-apps/src/{audit,metrics}.rs` |
| Detailed design | Eval-time assertions for ACA-specific invariants (§15.6). Closed OTEL label set (§13.4). Audit event schema (§13.3). Tracing target constant `d2b_provider_runtime_azure_container_apps::credential_lease_cleanup` retained. |
| Integration | Nix eval gate; `observability-otel` Provider OTEL pipeline |
| Data migration | None |
| Validation | Label cardinality policy test; audit commit-before-complete test; Nix assertion eval tests |
| Removal proof | Old Nix `ProviderManaged` workload options retired after Guest resource Nix emitter parity |

---

## 19 Tests

### 19.1 Required test layout

The only required file is:

```
packages/d2b-provider-runtime-azure-container-apps/
  src/
    tests/
      integration/
        README.md    # see §19.3
```

All other test files (`tests/controller_conformance.rs`, `tests/control_port.rs`, `tests/idempotency.rs`, `tests/adoption.rs`, `tests/credential.rs`, `tests/redaction.rs`, `tests/schema.rs`, `tests/error_codes.rs`, `tests/backoff.rs`, `integration/mock_azure/`, `integration/provider_system/`) are conventional and strongly recommended, but are not individually mandated as a layout requirement.

### 19.2 Unit test requirements

Every test in `tests/` must:

- compile and run with `cargo test -p d2b-provider-runtime-azure-container-apps` in a standard CI environment without network access, Azure credentials, or running Azure services;
- use only injected `AcaControl` and `AcaCredentialLeaseClient` mock implementations; no real SDK adapter is linked;
- be safe to run in parallel (`#[tokio::test]` with no shared mutable global state);
- not write to any path outside the test's temporary directory (injected as a fake Volume `main`-view dirfd);
- pass the `no_secret_bytes_in_any_public_surface` assertion for every public struct's `Debug` output.

### 19.3 Integration README

`packages/d2b-provider-runtime-azure-container-apps/integration/README.md` must document:

- how to run the mock Azure integration tests: `cargo test -p d2b-provider-runtime-azure-container-apps --test integration mock_azure`;
- how to run the provider system tests: `cargo test -p d2b-provider-runtime-azure-container-apps --test integration provider_system`;
- prerequisites: no Azure account, no real credentials, no network; all tests run against the in-process mock;
- how to enable live Azure integration (manual only; gated by `D2B_LIVE_ACA_TESTS=1` env var; never runs in CI);
- how to add new mock scenarios;
- test isolation: each test case creates an independent mock server bound to a random port on `127.0.0.1`; no shared global state.

### 19.4 Conformance requirements

The controller must pass the toolkit's black-box conformance suite (`d2b-provider-toolkit/src/conformance/`) for:

- `Guest` ResourceType spec validation (invalid cpuMillis, missing disk source, user domain rejection);
- reconcile/observe/finalize happy path;
- adoption (match, no-match, ambiguous);
- credential lease acquire/revoke around each method;
- status field redaction (guestIdentityDigest contains no raw sandbox ID string);
- error code stability under all `AcaControlErrorKind` variants;
- operation ledger TTL expiry and capacity eviction (via core Operation ledger adapter);
- sandbox-state and service-state Volume round-trip: framework-delivered `main`-view dirfd; `User/d2b-aca-{controller,deployment-service}` layout owners; `providerRef: Provider/volume-local`; `source.executionRef: Guest/<aca-gateway-name>` (guest-local); no `attachments[]`; `stateSchemaPhase: current` and Volume `status.ready: true` gate Process launch; `Volume` absent from controller `resourceTypes`; controller does not create/update Volume resources.

### 19.5 Mocked Azure test suite

The `integration/mock_azure/` module implements an in-process HTTP server that responds to ACA management API calls. The mock is deterministic: scenarios are parameterized by:

- `available: bool` — whether the mock returns 503 or 200;
- `rate_limit_after: Option<u32>` — returns 429 with `retry-after-ms` header after N calls;
- `ambiguous_candidates: bool` — `find_sandboxes` returns multiple matches;
- `lifecycle_sequence: Vec<AcaSandboxLifecycle>` — mock advances through states on successive reads.

The mock verifies that:

- no call to the mock server carries a raw credential byte in any header or body (the `AcaControl` implementation under test must redact all credential material before the HTTP call);
- all sandbox IDs in API responses are treated as opaque and are never echoed into resource status or audit records.
