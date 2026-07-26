# ADR 0046 Provider dossier: `runtime-azure-virtual-machine`

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-runtime-azure-virtual-machine` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 3 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-runtime-azure-virtual-machine` crate owner, Guest contracts, Nix integration |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-primitive-resource-composition`, `ADR-046-componentsession-and-bus`, `ADR-046-resources-credential`, `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-network`, `ADR-046-nix-configuration`, `ADR-046-components-processes-and-sandbox`, `ADR-046-telemetry-audit-and-support`, `ADR-046-zone-routing`, `ADR-046-provider-state` |
| Supersedes | Current `InfrastructureProvider` trait (`d2b-realm-provider/src/provider.rs`); `AzureVmForbidden` explicit rejection in `provider_registry.rs`; `AZURE_VM_IMPLEMENTATION_ID` constant; `WorkloadProviderKind::ProviderManaged` paths for Azure VM workloads |

---

## Source and reuse policy

The pre-ADR45 v3 baseline (`b5ddbed6`) contains no live Azure VM provider
implementation.

### Baseline sources

| Baseline source | Reachability | Selected behavior |
| --- | --- | --- |
| `packages/d2bd/src/provider_registry.rs`: `AzureVmForbidden`, `AZURE_VM_IMPLEMENTATION_ID: &str = "azure-vm"` | `production-reachable` | Explicit fail-closed rejection; `"azure-vm"` retained as internal implementation ID string constant in this crate |
| `packages/d2b-realm-provider/src/credential.rs`: `AzureControlPlaneRef { tenant_id: OpaqueAzureRef, subscription_id: OpaqueAzureRef, region: OpaqueAzureRef }`, `OpaqueAzureRef` | `implemented-and-reachable` | Opaque non-secret identifier fields; `OpaqueAzureRef` charset `^[A-Za-z0-9._-]+$`; bounded length; retained directly as v3 config field types |
| `packages/d2b-realm-provider/src/credential.rs`: `ManagedIdentityRef { client_id: OpaqueAzureRef }` | `implemented-and-reachable` | Source type for `clientId` config field |
| `packages/d2b-realm-provider/src/provider.rs`: `InfrastructureProvider` trait | `dead-reachable` | DELETE after this Provider is operational |
| `packages/d2b-realm-provider/src/types.rs`: `ProviderWorkloadIdentity::ManagedIdentity { client_id }` in `ProviderGuestdBootstrapContract` | `implemented-and-reachable` | Bootstrap identity pattern; `client_id: OpaqueAzureRef` field type retained |
| `packages/d2b-realm-core/src/workload.rs`: `WorkloadProviderKind::ProviderManaged` | `production-reachable` | Maps to `Guest.spec.providerRef = Provider/runtime-azure-virtual-machine` |
| `packages/d2b-realm-provider/src/rate_limit.rs`: `ProviderRateLimit`, `RateLimiter` | `implemented-and-reachable` | Backoff shape retained and adapted for `AzureEffectPort` ARM call retries |
| `packages/d2b-realm-provider/src/conformance.rs`: `ConformanceReport`, `ConformanceCheck` | `implemented-and-reachable` | Replaced by `d2b-provider-toolkit::conformance::check_provider_conformance` |

### ADR-0032 gateway enforcement

ADR 0032 (retained by ADR 0045 and ADR 0046) requires that host-local sessions
hold no realm credentials (`CredentialCustody::None`). Evidence:
`packages/d2bd/src/provider_registry.rs` `RealmSessionAuthority` enforces this
at runtime; `ADR-046-nix-configuration.md` reuse item at line 2300 confirms
"host-local sessions hold no realm credentials while gateway sessions hold
`GatewayGuest` custody".

**Consequence:** the controller and bootstrap-service components run in a gateway
Guest whose identity is declared in `spec.config.controllerExecutionRef`. All ARM
credential acquisition and bootstrap session handling occur inside that gateway
Guest. No Host-placed Process in this Provider holds an ARM Credential lease or
bootstrap PSK.

### Main reuse sources

Main commit `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` is an unrestricted
implementation reuse source (D041).

| Main source | Selected behavior | Excluded ADR 0045 assumptions |
| --- | --- | --- |
| `packages/d2b-session/src/{handshake,bootstrap,record,engine,scheduler,streams,lifecycle,transport}.rs` | Full ComponentSession: NN/KK/IKpsk2, prologue binding, record protection, streams, reconnect, cancellation | `serve_ttrpc_services` fixed 4-unit endpoint set; `EndpointPurpose::GuestBootstrap`/`GuestDirect`; `GUEST_SESSION_CREDENTIAL_*` |
| `packages/d2b-provider-toolkit/src/{conformance,fake_core,fake_bus,fault_injection,reconciler_loop,resource_client,volume_state}.rs` | Conformance harness, fake core/bus, reconciler loop, resource client, Volume-backed state helpers | Main-specific registry V3 enrollment paths |
| `packages/d2b-session/tests/{component_session.rs,noise_vectors.rs}` | snow 0.10 NN/KK/IKpsk2 test vectors and session lifecycle tests | None |

---

## Provider identity

| Field | Value |
| --- | --- |
| `providerRef` | `Provider/runtime-azure-virtual-machine` |
| Internal implementation ID | `azure-vm` |
| ResourceTypes implemented | `Guest` |
| Component types | 1 controller, 1 service |
| Crate | `packages/d2b-provider-runtime-azure-virtual-machine/` |
| Package | `d2b-provider-runtime-azure-virtual-machine` |
| Nix artifact ID | `provider-runtime-azure-vm` |

---

## Crate layout

```text
packages/d2b-provider-runtime-azure-virtual-machine/
  src/
    main_controller.rs       # azure-vm-controller binary entry point
    main_bootstrap.rs        # azure-vm-bootstrap-svc binary entry point
    controller/
      mod.rs                 # GuestController trait impl; reconcile/observe/finalize
      lifecycle.rs           # ARM LRO state machine (non-blocking; requeue-at driven)
      adoption.rs            # adopt pre-existing VM by d2b ARM resource tags
      bootstrap.rs           # IKpsk2 PSK generation; sealed PSK storage; enrollment
      idempotency.rs         # deterministic op IDs; AzureOperationHandle persistence
    effect/
      mod.rs                 # AzureEffectPort trait; opaque AzureOperationHandle
      real.rs                # azure_core/azure_mgmt_compute backing impl
      fake.rs                # FakeAzureEffectPort for hermetic tests
      rate_limit.rs          # ARM 429/503 backoff (adapted from baseline shape)
    bootstrap_svc/
      mod.rs                 # BootstrapService IKpsk2 → enrolled KK handler
      enrollment.rs          # enrolled key registration; controller notification
      admission.rs           # typed PSK admission service (receives sealed PSK via
                             # GrantBootstrapAdmission from controller over internal bus)
    credential.rs            # ARM credential acquisition via enrolled KK session
    telemetry.rs             # d2b-telemetry lightweight emitter + OTEL span helpers
    audit.rs                 # authoritative audit record emission
    error.rs                 # AzureVmError enum; bounded/redacted ProviderError mapping
    config.rs                # Provider spec.config schema struct
    schema.rs                # Guest spec.provider.settings schema struct + serde validation
    lib.rs                   # crate root; re-exports
  tests/
    conformance.rs
    lifecycle_hermetic.rs
    bootstrap_hermetic.rs
    credential_hermetic.rs
    idempotency.rs
    error_redaction.rs
    schema_validation.rs
    fault_injection.rs
  integration/
    README.md
    mock_arm_server/
    lifecycle_container.rs
    adoption_container.rs
    bootstrap_roundtrip.rs
    live/
      README.md
      lifecycle_live.rs
      adoption_live.rs
  README.md
```

---

## Provider spec.config schema

The Provider resource `spec.config` is validated against this Provider's
signed JSON Schema before the Provider reaches `Ready`. The shape follows
`ADR-046-provider-model-and-packaging §Configuration projection`: `spec`
contains `artifactId` and `config`.

No field in `spec.config` carries secret bytes, credential material,
connection strings, or raw numeric UIDs.

### YAML reference

```yaml
spec:
  artifactId: "provider-runtime-azure-vm"
  config:
    tenantId: "2f8e1c3a-1234-5678-9abc-def012345678"
    # OpaqueAzureRef | null. Azure AD tenant GUID. Not a secret. Not a ResourceRef.
    # Max 128 chars; charset ^[A-Za-z0-9._-]+$.
    # Required when armCredentialRef resolves credential-entra.

    clientId: null
    # OpaqueAzureRef | null. User-assigned managed identity client GUID or
    # service principal appId. Not a secret. Null = system-assigned MI.

    armCredentialRef: "Credential/arm-azure-vm"
    # ResourceRef. Resolves a Credential granting ARM API access.
    # Must resolve Provider/credential-managed-identity or Provider/credential-entra.
    # Must include allowedOperations: [acquire-token, refresh-token].
    # Token bytes delivered via enrolled KK session only (D055/D056).

    controllerExecutionRef: "Guest/azure-relay-gateway"
    # ResourceRef. Gateway Guest in which the controller and bootstrap-service
    # Processes run. Required. ADR-0032 enforcement: no Host-placed Process in
    # this Provider holds an ARM credential or bootstrap PSK.

    networkRef: null
    # ResourceRef | null. Network to attach to gateway Guest Processes.
    # Null uses the gateway Guest's default network attachment.
```

### spec.config field table

| Field | Type | Required | Bounds | Notes |
| --- | --- | --- | --- | --- |
| `tenantId` | OpaqueAzureRef \| null | Conditional | Max 128; `^[A-Za-z0-9._-]+$` | Required with credential-entra. Not a secret. |
| `clientId` | OpaqueAzureRef \| null | No | Max 128; `^[A-Za-z0-9._-]+$` | User-assigned MI client ID. Not a secret. Null = system-assigned. |
| `armCredentialRef` | ResourceRef | Yes | `Credential/<name>` | ARM API credential. Providers: credential-managed-identity, credential-entra. |
| `controllerExecutionRef` | ResourceRef | Yes | `Guest/<name>` | Gateway Guest for controller and bootstrap-svc Processes. ADR-0032. |
| `networkRef` | ResourceRef \| null | No | `Network/<name>` | Network attachment for gateway Guest Processes. Null = gateway Guest default. |

`tenantId` reuses `OpaqueAzureRef` from
`d2b-realm-provider/src/credential.rs::AzureControlPlaneRef.tenant_id`.
`clientId` reuses `ManagedIdentityRef.client_id` from the same source.

---

## Guest `spec.provider.settings` schema

Each Guest resource targeting this Provider carries implementation-only desired
configuration in `spec.provider.settings` with the following fields. Unknown
fields are rejected.

**D089 spec extension contract:** this Provider's implementation-only desired
configuration is carried in `spec.provider.settings` under
`runtime-azure-virtual-machine.d2bus.org/Guest/spec`; the schema is
registered/signed in the manifest, deny-unknown, bounded, versioned, and
validated against `spec.providerRef` at Nix build and API admission. Base fields
stay at `spec.*`; shared semantics are promoted to the Guest base and never
placed in `spec.provider`. This Provider implements the exact base spec/status
schema version/fingerprint, accepts the canonical minimal valid base Spec, and
rejects an unsupported optional base capability only through its signed
capability matrix plus provider-neutral `unsupported-capability`.
`spec.provider` aligns with `status.provider` for
`Provider/runtime-azure-virtual-machine`.

`spec.systemArtifactId` is always `null` for Azure VM Guests (cloud image boot,
no Nix system artifact). Enforced at eval time.

### YAML reference

```yaml
provider:
  schemaId: runtime-azure-virtual-machine.d2bus.org/Guest/spec
  schemaVersion: 1.0.0
  settings:
    # Azure placement
    subscriptionId: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"
    resourceGroup: "d2b-workloads-rg"
    region: "eastus"

    # VM shape
    vmSize: "Standard_D4s_v5"
    imageRef: "Canonical:ubuntu-24_04-lts:server:latest"
    diskSku: "Premium_LRS"           # enum: Premium_LRS|StandardSSD_LRS|Standard_LRS|UltraSSD_LRS
    osDiskSizeGb: 64                 # u32 | null; null = image default

    # Admin user (ARM osProfile requirement; not used for SSH access)
    adminUser: "azureuser"           # bounded string

    # Network (Azure-internal; no Network Provider dependency per D045)
    vnetSubscriptionId: null         # OpaqueAzureRef | null; null = subscriptionId
    vnetResourceGroup: null          # OpaqueAzureRef | null; null = resourceGroup
    vnetName: "d2b-workloads-vnet"
    subnetName: "guests"
    assignPublicIp: false

    # Disks (provider-owned ARM children; no Volume ResourceRef or cloud URI)
    dataDisks: []
    # list of DataDiskSpec; max 16 entries; see below

    # Bootstrap
    bootstrapPskDelivery: "vm-extension"  # enum: vm-extension|user-data
    bootstrapDeadlineMs: 600000

    # Optional child Zone hosting
    childZoneHosting: false

    # Tags (d2b:* keys reserved; rejected at eval time)
    azureTags: {}
```

### `spec.provider.settings` field table

| Field | Type | Required | Default | Bounds | Notes |
| --- | --- | --- | --- | --- | --- |
| `subscriptionId` | OpaqueAzureRef | Yes | - | Max 128 | Azure subscription GUID. Not a secret. |
| `resourceGroup` | OpaqueAzureRef | Yes | - | Max 90 | Target resource group name. |
| `region` | OpaqueAzureRef | Yes | - | Max 64 | Azure region. |
| `vmSize` | OpaqueAzureRef | Yes | - | Max 64 | VM size SKU. Resize requires deallocation. |
| `imageRef` | OpaqueAzureRef | Yes | - | Max 512 | Image URN or gallery path. Change requires re-provision. |
| `diskSku` | enum | No | `Premium_LRS` | Closed set | OS disk SKU. |
| `osDiskSizeGb` | u32 \| null | No | null | 30..4095 | OS disk GiB. Null = image default. |
| `adminUser` | string | Yes | - | Max 64; `^[a-z_][a-z0-9_-]*$` | Linux admin username for ARM osProfile. Not used for SSH access. |
| `vnetSubscriptionId` | OpaqueAzureRef \| null | No | null | Max 128 | VNet subscription. Null = subscriptionId. |
| `vnetResourceGroup` | OpaqueAzureRef \| null | No | null | Max 90 | VNet resource group. Null = resourceGroup. |
| `vnetName` | OpaqueAzureRef | Yes | - | Max 64 | VNet name. |
| `subnetName` | OpaqueAzureRef | Yes | - | Max 80 | Subnet name. |
| `assignPublicIp` | bool | No | `false` | - | Discouraged for production. |
| `dataDisks` | list | No | `[]` | 0..16; LUN 0..63 unique | Provider-owned ARM child disk intents. No Volume ResourceRef. No cloud resource URI. See DataDiskSpec. |
| `bootstrapPskDelivery` | enum | No | `vm-extension` | `vm-extension`\|`user-data` | One-time PSK delivery mechanism over ARM control plane. |
| `bootstrapDeadlineMs` | u64 | No | `600000` | 60000..3600000 | Bootstrap enrollment deadline. |
| `childZoneHosting` | bool | No | `false` | - | When true, VM hosts a child Zone; creates ZoneLink after enrollment. |
| `azureTags` | map | No | `{}` | Max 50; key ≤512; val ≤256 | Azure resource tags. `d2b:*` keys rejected at eval time. |

### DataDiskSpec

```yaml
lun: 0                   # u8; 0..63; unique within this Guest's spec.provider.settings
diskClass: "high-perf"   # OpaqueAzureRef | null; advisory class label; max 64 chars
sizeGb: 128              # u32; 1..32767 GiB
label: null              # string | null; advisory label; max 64 chars; no cloud URI
```

`DataDiskSpec` entries are provider-owned ARM child resources. They are Azure
Managed Disks whose full lifecycle is owned by this Provider's controller.
They have no Volume ResourceRef and no `volume-local` backing. The frozen
catalog contains no Azure Volume Provider; no standard Volume substitution
exists. No cloud resource URI appears in public status or audit records;
the controller identifies disks internally by opaque handle
(`AzureOperationHandle`) returned by `AzureEffectPort`.

---

## ResourceTypes implemented

### Guest

| `Guest.spec` field | Support | Notes |
| --- | --- | --- |
| `systemArtifactId` | `null` only | Cloud image boot; no Nix system artifact. Enforced at eval time. |
| `defaultDomain` | `system` only | Azure VM Guests do not expose a user manager to d2b. |
| `allowedDomains` | `[system]` only | Same restriction. |
| `defaultUserRef` | `null` only | User domain not supported. |
| `budget` | Yes | Aggregate budget; applies to child Processes on this Guest. |
| `networkAttachments` | No | Azure VNet/subnet in `spec.provider.settings`; no Network Provider per D045. |
| `deviceAttachments` | No | No Device Provider attachment for Azure VMs. |

Provider capability descriptor: `isolationClass: "provider-managed-vm"`.

### ResourceTypes consumed

| ResourceType | Purpose |
| --- | --- |
| `Credential` (`armCredentialRef`) | ARM API token via enrolled KK (D055/D056) |
| `Process` | Controller and bootstrap-service child Processes |
| `Provider` | `spec.config.networkRef` dependency readiness |

---

## Provider payload state (ProviderStateSet)

A **ProviderStateSet** is the optional, query-time set of the *declared* Volume
resources in a Zone whose `metadata.ownerRef` resolves to
`Provider/runtime-azure-virtual-machine`. It is a logical query-time grouping,
not a ResourceType and not a stored artifact. Bounded non-secret operational
state belongs in the owning resource's `status` subresource and the core
Operation ledger by default (D087); a state Volume is declared only for a
payload that passes the storage-need test.

For this Provider, **ARM operation handles, checkpoint/idempotency records, and
all non-secret observed cloud state move to `Guest.status` and the core
Operation ledger** - the core Operation ledger owns in-flight ARM
idempotency/retry/transaction progress, and `Guest.status` owns the latest
bounded observed cloud phase (opaque, non-authorizing `AzureOperationHandle`
digests only; never a poll URL, resource URI, or endpoint). This Provider
retains **exactly one** guest-local, sealed Provider state Volume, and only for
the **secret bootstrap PSK / admission / enrolled private recovery material**
that cannot enter status. That Volume passes the storage-need test as secret,
sensitive private recovery data. The Volume is an ordinary `Volume` resource
created by **core ProviderDeployment** - before component Processes start - from
the single `stateNamespaces` declaration in the controller component descriptor.
It is not authored in the Zone bundle by the operator and does not appear in Nix
configuration. `Provider/volume-local` is the sole Volume reconciler; the
semantic controller does not own, create, or delete Volume resources and does
not add `Volume` to its exported ResourceTypes. The controller consumes its
required view dirfd only.

### Guest-local placement

The retained sealed state Volume uses manifest-frozen **guest-local** placement.
The `Provider/volume-local` instance reconciling and hosting the Volume is the
one running **inside the gateway Guest** named by
`spec.config.controllerExecutionRef`, not a host-side volume-local instance. The
`source.executionRef` field carries the same gateway Guest ref and is the
authoritative reconciliation target.

Custody invariants:

- The **host MUST NOT hold** any of the following state for this Provider:
  remote/cloud ARM operation bindings, bootstrap admission records, PSK
  ciphertext or plaintext, enrolled key material, or idempotency handles.
- There is **no virtiofs or host-to-guest attachment** of provider state.
  The sealed recovery file is written and read exclusively by the controller
  process running inside the gateway Guest; the host filesystem is never the
  backing store.
- The **manifest freezes guest-local placement**; the Volume expresses this with
  `source.executionRef: Guest/<gateway>`, with no fallback to host-backed
  storage and no runtime override.
- Credential/audit/remote-node/cloud-control schemas require `guest-local`;
  `host-backed-guest` is rejected with `guest-local-required`.

The retained sealed Volume is `persistenceClass: persistent` with a nonzero byte
quota and a broker-maintained identity marker. It survives component and
Provider restart and participates in the Provider upgrade, destroy, and reset
lifecycle. `sensitivityClass: private` means it is mounted by exactly one
Process at a time; any attempt to mount it from a different component or domain
fails with `volume-domain-mismatch`. The controller receives only the dirfd for
its own declared view; no cross-component shared dirfd or raw path is ever
handed out.

### Layout principals

The Volume `layout[*].ownerRef` and `layout[*].groupRef` reference a
Nix-preprovisioned `User/azure-vm-controller` resource declared in the Zone's
Users configuration alongside the Provider. The controller never creates User
resources itself. Numeric UID/GID strings are rejected at eval time by the
volume-local Provider schema.

### Volume naming convention

`<provider-name>--<component-id>--<namespace-id>--<execution-ref-short>`

Enforced at runtime: the Volume in the ProviderStateSet is checked against the
declared `stateNamespace` and execution target in the component descriptor.

### Controller sealed recovery Volume (secret PSK / admission / enrollment)

```yaml
# Example name: runtime-azure-virtual-machine--azure-vm-controller--recovery-state--gw
apiVersion: resources.d2bus.org/v3
type: Volume
metadata:
  name: runtime-azure-virtual-machine--azure-vm-controller--recovery-state--gw
  zone: dev
  ownerRef: Provider/runtime-azure-virtual-machine
spec:
  providerRef: Provider/volume-local
  kind: state
  persistenceClass: persistent
  sensitivityClass: private       # single-process; volume-domain-mismatch on any other mount
  stateSchema:
    schemaId: runtime-azure-virtual-machine.d2bus.org/controller/recovery-state
    schemaVersion: "1.0"
    schemaDigest: sha256:<hex>
    migrationPolicy: pre-launch-required
  storageNeed: secret             # sealed PSK / admission / enrolled private recovery material
  quotaBytes: 1048576             # 1 MiB; nonzero required
  quota:
    maxBytes: 1048576             # hard byte cap (= quotaBytes for provider-state)
    maxInodes: 512                # hard inode cap; nonzero required
    enforcement: none
  sealingCredentialRef: Credential/azure-vm-controller-state-key
  source:
    executionRef: Guest/<gateway> # resolved from spec.config.controllerExecutionRef
    settings:
      kind: local-path
      sourcePolicyId: runtime-azure-virtual-machine-controller-state
  layout:
    - path: state
      type: directory
      ownerRef: User/azure-vm-controller     # Nix-preprovisioned; not a ComponentPrincipal
      groupRef: User/azure-vm-controller
      mode: "0700"
      sensitivity: private
      createPolicy: create-if-never-provisioned
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
      noFollow: true
  views:
    main:
      path: state
      rights: [read, write, create, delete, traverse]
  identityMarker:
    class: broker-maintained
    markerRoot: provider-state-markers
  snapshotPolicy: null
  retentionPolicy: null
```

**State contents** (sealed at rest by `sealingCredentialRef`; secret private
recovery material only):

| Path | Content | Notes |
| --- | --- | --- |
| `state/enrollment/<guest-uid>.json` | Enrolled static key recovery digest, enrollment timestamp | Sealed; no Noise private key plaintext |
| `state/bootstrap-psk/<guest-uid>.bin` | Sealed ciphertext PSK admission record | PSK bytes exist ONLY as sealed ciphertext; plaintext never written |
| `state/admission/<guest-uid>.bin` | Sealed admission grant record | Session-scoped admission material the controller mediates for the bootstrap service |

ARM operation handles, idempotency records, and checkpoints do **not** live in
this Volume: in-flight idempotency/retry/transaction progress is owned by the
core Operation ledger, and the latest bounded observed cloud phase (opaque,
non-authorizing `AzureOperationHandle` digest only) is owned by `Guest.status`.
No ARM poll URL, ARM resource URI, or ARM endpoint appears anywhere in
resources, status, or this Volume; `AzureEffectPort` holds that mapping in
process memory.

### Bootstrap-service state

The `azure-vm-bootstrap-svc` service declares **no** Provider state Volume. Its
admission session state is transient in process memory; it obtains PSK admission
through an authenticated typed internal bus call to the controller process
(`GrantBootstrapAdmission` on `d2b.azure-vm.controller.v1`). The controller
decrypts the sealed PSK/admission record from its own recovery Volume, validates
the request, and delivers the admission token over that single session. The
bootstrap service's bounded non-secret operational state (readiness, session
counters, closed-enum error detail) lives in `status`/the core Operation ledger
(D087). There is no bootstrap-svc admission-state Volume and no
`User/azure-vm-bootstrap-svc` state-layout principal.

---

## Components

### azure-vm-controller (controller)

| Field | Value |
| --- | --- |
| Component ID | `azure-vm-controller` |
| Component type | controller |
| Binary | `azure-vm-controller` |
| Owns ResourceTypes | `Guest` with `providerRef = Provider/runtime-azure-virtual-machine` |
| Execution placement | Inside `spec.config.controllerExecutionRef` (ADR-0032 gateway Guest) |
| Domain | `system` |
| Cardinality | 1 per Zone |
| Watch selectors | `type=Guest, spec.providerRef=Provider/runtime-azure-virtual-machine` |
| Dependency aliases | `credential` → `spec.config.armCredentialRef` |
| Internal service | `d2b.azure-vm.controller.v1` |
| Status authority | `update-status` authorized for all Guest resources with this Provider; writes top-level `status.phase`, common Guest `status.resource`, and typed Azure `status.provider.details` atomically |
| State Volume | one guest-local **sealed** Provider state Volume (created by core ProviderDeployment before first Process start from the controller `stateNamespaces` descriptor) holding only the secret PSK/admission/enrolled private recovery material that cannot enter status; ARM operation/idempotency and non-secret observed cloud state live in `Guest.status`/the core Operation ledger (D087); controller consumes required view dirfd only; does not own/create/reconcile the Volume; `sensitivityClass: private`; Nix-preprovisioned `User/azure-vm-controller` layout principal |
| Reconcile concurrency | 4 concurrent Guest resources |
| Maximum pending | 64 Guest resources |
| Injected ports | `AzureEffectPort`, `CredentialEffectPort`, `TransportEffectPort` |

### azure-vm-bootstrap-svc (service)

| Field | Value |
| --- | --- |
| Component ID | `azure-vm-bootstrap-svc` |
| Component type | service |
| Binary | `azure-vm-bootstrap-svc` |
| Execution placement | Same `spec.config.controllerExecutionRef` (ADR-0032 gateway Guest) |
| Domain | `system` |
| Service | `d2b.azure-vm.bootstrap.v1` (IKpsk2 handshake → enrolled KK) |
| Cardinality | 1 per Zone |
| Session limit | 1 active bootstrap session per Guest; max 16 concurrent |
| State Volume | none - the bootstrap service declares no Provider state Volume; its session state is transient in process memory and it obtains sealed PSK/admission from the controller via `GrantBootstrapAdmission`; bounded non-secret operational state lives in `status`/the core Operation ledger (D087) |
| Injected ports | `ControllerServicePort` (d2b.azure-vm.controller.v1), `TransportEffectPort` |

---

## Process resources (canonical form)

Both component Processes use `Provider/system-minijail` and are placed on the
gateway Guest from `spec.config.controllerExecutionRef`. The canonical
SandboxSpec uses semantic classes; no raw seccomp BPF, capability numbers,
minijail argument strings, or environment variables appear in spec.

Controller and bootstrap-svc use injected `AzureEffectPort`,
`CredentialEffectPort`, and `TransportEffectPort` only. No ambient network
access is declared (`allowEgress: false`). All external calls go through
the injected async effect port traits over local FD channels.

### azure-vm-controller Process

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: azure-vm-controller-process
  ownerRef: Provider/runtime-azure-virtual-machine
spec:
  providerRef: Provider/system-minijail
  executionRef: Guest/<gateway>     # resolved from spec.config.controllerExecutionRef
  domain: system
  processClass: controller
  template: azure-vm-controller
  configRef: null
  credentialRefs:
    - Credential/arm-azure-vm       # armCredentialRef; acquire-token only via enrolled KK
  mounts:
    - volumeRef: Volume/runtime-azure-virtual-machine--azure-vm-controller--recovery-state--gw
      view: main
      mountPath: /state
      access: read-write
      required: true
  sandbox:
    namespaceClasses: [mount, ipc, uts, network, pid]
    capabilityClasses: []           # no host capabilities required
    seccompClass: strict            # minimal allow-list for controller process class
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    memory:
      request: "96Mi"
      limit: "128Mi"
    pids:
      limit: 256
    fds:
      limit: 1024
  networkUsage:
    networkRef: null                # resolved from spec.config.networkRef; null = default
    allowEgress: false              # no ambient network; all ARM calls via AzureEffectPort
    ports: []
  deviceUsage: []
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: false
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "60s"
    backoffMultiplierMilli: 2000
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    class: ready-condition
    initialDelay: "0s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
```

Stable control/service surfaces are owned Endpoint resources rather than inline
Process fields:

```yaml
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: azure-vm-controller
  zone: system
  ownerRef: Provider/runtime-azure-virtual-machine
spec:
  providerRef: Provider/runtime-azure-virtual-machine
  producerRef: Process/azure-vm-controller-process
  endpointClass: control
  transport: unix
  purpose: d2b-bus-controller-endpoint
  serviceFingerprint: runtime-azure-virtual-machine.d2bus.org/controller/v1
  locality: guest-local
  visibility: provider
  attachmentPolicy: launch-ticket-only
  consumerPolicy:
    allowedSubjects: [Provider/runtime-azure-virtual-machine]
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
---
apiVersion: resources.d2bus.org/v3
type: Endpoint
metadata:
  name: azure-vm-bootstrap
  zone: system
  ownerRef: Provider/runtime-azure-virtual-machine
spec:
  providerRef: Provider/runtime-azure-virtual-machine
  producerRef: Process/azure-vm-bootstrap-svc-process
  endpointClass: service
  transport: unix
  purpose: d2b-bus-service-endpoint
  serviceFingerprint: runtime-azure-virtual-machine.d2bus.org/bootstrap/v1
  locality: guest-local
  visibility: provider
  attachmentPolicy: launch-ticket-only
  consumerPolicy:
    allowedSubjects: [Provider/runtime-azure-virtual-machine]
    allowedOperations: [resolve]
  lifecyclePolicy: recycle-with-producer
```

### azure-vm-bootstrap-svc Process

```yaml
apiVersion: resources.d2bus.org/v3
type: Process
metadata:
  name: azure-vm-bootstrap-svc-process
  ownerRef: Provider/runtime-azure-virtual-machine
spec:
  providerRef: Provider/system-minijail
  executionRef: Guest/<gateway>     # resolved from spec.config.controllerExecutionRef
  domain: system
  processClass: service
  template: azure-vm-bootstrap-svc
  configRef: null
  credentialRefs: []                # no direct credential access; obtains PSK via
                                    # GrantBootstrapAdmission from controller only
  mounts: []                        # no Provider state Volume; session state in process memory; operational state in status/core ledger (D087)
  sandbox:
    namespaceClasses: [mount, ipc, uts, network, pid]
    capabilityClasses: []
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    environmentClass: minimal
    readOnlyRoot: true
  budget:
    memory:
      request: "48Mi"
      limit: "64Mi"
    pids:
      limit: 128
    fds:
      limit: 512
  networkUsage:
    networkRef: null                # no ambient network; relay transport via injected port
    allowEgress: false
    ports: []
  deviceUsage: []
  telemetry:
    metricsEnabled: true
    tracingEnabled: true
    logLevel: info
    sensitiveLabels: false
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "30s"
    backoffMultiplierMilli: 2000
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    class: ready-condition
    initialDelay: "0s"
    timeout: "15s"
    failureThreshold: 3
    successThreshold: 1
  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
```

### Endpoint resources (D092)

`Provider/runtime-azure-virtual-machine` declares conformance to the standard
`Endpoint` base schema. Stable controller and bootstrap service endpoints are
owned `Endpoint` resources with `ownerRef` and `producerRef`; consumers use
`Endpoint/<name>` ResourceRefs. No raw Unix path, relay URL, fd, credential, ARM
URL, or guest-control locator appears in resource spec/status or CLI output.
Resolution is only via authorized EffectPort/LaunchTicket flows; unauthorized
resolve fails `endpoint-resolve-denied`. A producer restart bumps
`endpointGeneration` and causes dependent consumers to receive
`dependency-changed`. Per-session relay/transport handles remain internal and
are not Endpoints.

### Retained opaque handles

Retained opaque values are `AzureOperationHandle`, `VmHandle`, credential lease
handles, idempotency keys, operation IDs, per-session ComponentSession/relay
transports, and LaunchTicket fd indexes. They are controller-internal,
high-churn, non-authorizing, or lack independent lifecycle, so D092 keeps them
opaque instead of promoting them to resources.

---

## AzureEffectPort and opaque operation handles

All ARM API calls go through the `AzureEffectPort` injected async trait. The
controller never holds a raw HTTP client, ARM poll URL, or ARM resource URI.
The effect port is the sole owner of the mapping from opaque handle to LRO
continuation.

```text
trait AzureEffectPort (async):
  start_vm_provision(request: ProvisionRequest) -> Result<AzureOperationHandle, AzureVmError>
  start_vm_resize(handle: VmHandle, size: OpaqueAzureRef) -> Result<AzureOperationHandle, AzureVmError>
  start_vm_delete(handle: VmHandle) -> Result<AzureOperationHandle, AzureVmError>
  start_disk_attach(handle: VmHandle, disk: DiskIntent) -> Result<AzureOperationHandle, AzureVmError>
  start_disk_detach(handle: VmHandle, lun: u8) -> Result<AzureOperationHandle, AzureVmError>
  poll_lro(op: &AzureOperationHandle) -> Result<LroStatus, AzureVmError>
  get_vm_state(handle: &VmHandle) -> Result<AzureVmState, AzureVmError>
  put_vm_extension(handle: &VmHandle, payload: PskExtensionPayload) -> Result<AzureOperationHandle, AzureVmError>
  delete_vm_extension(handle: &VmHandle, name: &str) -> Result<(), AzureVmError>
  update_vm_tags(handle: &VmHandle, tags: TagMap) -> Result<AzureOperationHandle, AzureVmError>
```

`AzureOperationHandle`: bounded opaque bytes (max 256 bytes); no ARM URL or
cloud resource path. Serialized as base64-encoded bytes in state files. The
real `AzureEffectPort` impl maps handles to ARM LRO poll URLs internally; the
controller never observes those URLs.

`FakeAzureEffectPort`: implements the trait with scripted LRO sequences. Used
in all hermetic tests. Never opens a real HTTP socket. Records all calls for
assertion. Does not expose ARM URLs to test assertions.

---

## Cloud Guest lifecycle

### Common Guest phase

`Guest.status.phase` uses the standard common values:
`Pending | Ready | Succeeded | Degraded | Failed | Deleted | Unknown`.

The controller is the **authorized `update-status` writer** for all Guest
resources with `providerRef = Provider/runtime-azure-virtual-machine`. Per D088
it writes the top-level common `phase`, Guest ResourceType-common
`status.resource`, and Azure-specific `status.provider.details` in one atomic
`UpdateStatus`. Core aggregates Provider resource status (the Provider's own
component status, not per-Guest status). Shared fields are never duplicated into
`status.provider`; the strict, ≤32 KiB, redacted extension schema is registered
and signed in the Provider manifest.

`Deleted` is a terminal event-only phase; the resource row is removed after
the audit record is appended post-commit (core deletion contract).

### providerPhase values

| providerPhase | Common phase | Meaning |
| --- | --- | --- |
| `Absent` | `Pending` | No ARM VM correlated to this Guest |
| `Provisioning` | `Pending` | ARM VM PUT submitted; awaiting LRO completion |
| `ProvisionFailed` | `Failed` | ARM provisioning reached a failed state |
| `PskDelivering` | `Pending` | ARM VM provisioned; PSK extension PUT submitted |
| `Bootstrapping` | `Pending` | PSK delivered; awaiting IKpsk2 session from VM |
| `BootstrapFailed` | `Failed` | Bootstrap deadline expired after retry |
| `Ready` | `Ready` | Enrolled KK active; ARM VM running |
| `Reconfiguring` | `Degraded` | Spec change in progress |
| `Draining` | `Degraded` | Drain requested |
| `Deleting` | `Pending` | ARM DELETE submitted; awaiting LRO |

### Non-blocking LRO contract

The reconcile handler MUST NOT block on ARM polling loops. No blocking
`sleep`, synchronous HTTP poll, or multi-second await is permitted inside
any `reconcile`, `observe`, or `finalize` handler.

The contract:

1. **Start**: call `AzureEffectPort::start_*(...)`, receive `AzureOperationHandle`.
   Persist the idempotency record (op class + opaque handle digest) in the
   **core Operation ledger** (D087), and record the latest bounded observed
   phase (opaque handle digest) in `Guest.status`. Advance `providerPhase`.
   Return `requeue-at: now + poll_interval` immediately.
2. **Poll ticks**: on requeue-at trigger, read the in-flight handle from the
   core Operation ledger. Call `AzureEffectPort::poll_lro(&handle)`:
   - `InProgress`: return `requeue-at: now + next_interval`
   - `Succeeded`: advance to next phase; clear the ledger op record; proceed
   - `Failed`: transition to failure providerPhase; clear the ledger op record
3. **After controller restart** (ledger op record present but handle semantics
   unclear): call `AzureEffectPort::get_vm_state` to re-derive state from
   external reality (treating `Guest.status` as observation, never authority);
   continue.
4. **Between ticks**: other Guest resources are processed concurrently.

### Provisioning sequence

1. Derive idempotency key: `sha256(<zone-uid>:<guest-uid>:<guest-generation>)`
   encoded as 20-char base32.
2. Call `AzureEffectPort::start_vm_provision(request)` → receive `AzureOperationHandle`.
   Persist op record. Set `providerPhase: Provisioning`. Return `requeue-at`.
3. On LRO Succeeded: call
   `AzureEffectPort::put_vm_extension(handle, psk_payload)` → PSK delivery.
   Set `providerPhase: PskDelivering`. Return `requeue-at`.
4. On extension LRO Succeeded: set `providerPhase: Bootstrapping`. Return
   `requeue-at: now + bootstrapDeadlineMs`.

### ARM VM resource naming

- VM: `d2b-<zone-id>-<guest-id>` (20-char base32 UIDs)
- NIC: `d2b-<zone-id>-<guest-id>-nic`
- OS disk: `d2b-<zone-id>-<guest-id>-osdisk`
- Data disks: `d2b-<zone-id>-<guest-id>-disk-<lun>`

All ARM resources receive d2b-reserved tags:
`d2b:zone-uid`, `d2b:guest-uid`, `d2b:managed-by=d2b-azure-vm-controller`.
Plus operator `azureTags`.

No ARM resource ID path, ARM resource URI, or cloud endpoint URL appears in
any public status, audit record, or OTEL attribute.

### PSK generation and sealing

PSK plaintext is held only in the controller process address space during the
delivery window. It is sealed with `sealingCredentialRef` before writing to
the state Volume. The Volume stores only ciphertext.

The bootstrap-service never reads the controller's Volume. It receives the
admission token via `GrantBootstrapAdmission` on the controller's internal
bus (see §Bus protocols). The controller decrypts the sealed ciphertext, validates
the request, and delivers plaintext PSK over that single session only. The
plaintext is immediately zeroized after delivery.

PSK delivery mechanisms:

**`vm-extension`**: ARM Custom Script Extension PUT with PSK bytes base64-encoded.
Extension metadata is deleted by the controller after enrollment or expiry.

**`user-data`**: base64-encoded PSK in VM `userData` at provisioning. Cloud-init
reads it from IMDS; VM agent clears it on use.

### Adoption

1. `AzureEffectPort::get_vm_state` query.
2. Tags match + enrolled key exists → `providerPhase: Ready`.
3. Tags match, no enrolled key → `providerPhase: Bootstrapping`; re-deliver PSK.
4. Tags mismatch → condition `AdoptionZoneMismatch`; no ARM mutation.

### Reconfiguring

| Changed field | Effect |
| --- | --- |
| `azureTags` | ARM tag update LRO |
| `vmSize` | Dealloc + resize + start LROs (requeue-at driven) |
| `osDiskSizeGb` (increase) | ARM disk-resize LRO + restart |
| `imageRef` | Drain → Delete → Provision cycle; requires `imageChangeConfirm` guard |
| `dataDisks` (add) | ARM attach-disk LRO |
| `dataDisks` (remove) | Drain first; then ARM detach LRO |

### Deleting

1. `AzureEffectPort::start_vm_delete` → persist handle → return `requeue-at`.
2. On LRO Succeeded or 404: delete NIC, OS disk, data disks via effect port.
3. After all deletes complete: remove enrolled key; delete op records.

**Finalizer**: cleared only after ARM deletion LRO confirms success or 404-absent.
Ambiguity keeps finalizer open; sets condition `DeletionAmbiguous`. Core's
Deleted phase and row removal follow finalizer release. Audit record
(`azure-vm-deleted`) is appended by the audit subsystem after the durable store
commit (core deletion contract); it is NOT part of the store transaction.

---

## Idempotency

Deterministic operation IDs: `sha256(<zone-uid>:<guest-uid>:<generation>:<op-class>)`,
20-char base32. Passed as `x-ms-client-request-id` in all ARM calls.

ARM 409 `Conflict`: call `get_vm_state` → check `d2b:guest-uid` tag → adopt
or set `AzureVmConflict` condition.

Controller restart: op record present → read handle → `poll_lro` to re-derive
state.

---

## Bootstrap protocol: IKpsk2 → enrolled KK

```
Controller (gateway Guest)              Azure VM agent
    │
    │  generate + seal PSK (ciphertext only in state Volume)
    │  GrantBootstrapAdmission → bootstrap-svc (plaintext PSK, single session)
    │
    │── AzureEffectPort::put_vm_extension ──────►│ PSK plaintext one-way via ARM
    │                                             │
    │◄─ Noise_IKpsk2_25519_ChaChaPoly_SHA256 ────│
    │   initiator static s_i = VM static key
    │   PSK = one-time bootstrap PSK
    │   prologue = preface ‖ canonical offer
    │
    │  bootstrap-svc calls GrantBootstrapAdmission to controller (bus)
    │  controller decrypts sealed PSK ciphertext; validates + consumes
    │  controller validates PSK match (single-use)
    │
    │  bootstrap-svc registers s_i in Zone identity registry
    │  bootstrap-svc emits bootstrap-enrollment-complete audit event
    │  controller clears admission token; writes enrollment record (sealed)
    │
    │── subsequent KK sessions (Noise_KK_25519_ChaChaPoly_SHA256) ──────────►│
```

The bootstrap-service's own static Noise private key is provided at process
launch via the Zone's process-credential mechanism (sealed credential in
`$CREDENTIALS_DIRECTORY`). It is zeroizing in-process memory and never written
to any Volume.

---

## Bus protocols

### d2b.azure-vm.controller.v1 (controller internal service)

Sessions: local NN within the gateway Guest.

| Method | Direction | Purpose |
| --- | --- | --- |
| `Health` | request/reply | Liveness and dependency readiness |
| `GrantBootstrapAdmission` | request/reply | Bootstrap-svc requests PSK admission; controller decrypts sealed PSK, validates op/nonce/expiry/subject, delivers one-time plaintext over this session; consumed-once |
| `NotifyEnrollment` | event (bootstrap-svc → controller) | Enrolled KK active for a Guest |
| `ForceRereconcile` | request/reply | Admin-only; immediate reconcile |

`GrantBootstrapAdmission` is the ONLY path by which the bootstrap-service
obtains a PSK plaintext. The plaintext PSK is zeroized in the controller
process address space immediately after delivery.

### d2b.azure-vm.bootstrap.v1 (bootstrap service)

| Method | Phase | Notes |
| --- | --- | --- |
| IKpsk2 session auth | Bootstrap phase | Session authentication IS the method |
| `CompleteEnrollment` | Post-IKpsk2 enrolled KK | VM confirms enrollment |

### d2b.credential.v3 (consumed from credential Provider)

ARM tokens acquired by the controller over enrolled KK.
`credential-managed-identity` placement is `guest-agent` (runs in gateway Guest).
`credential-entra` placement is `guest-agent` or `user-agent`.
Neither supports `host-system` placement for ARM credential delivery.

**D093 `Provider/credential-entra` consumer note:** when `armCredentialRef` resolves to `Provider/credential-entra`, the Azure VM controller acquires the ARM access-token lease from the Entrablau identity Guest named by that Credential `identityGuestRef` and `loginEndpointRef`. The token is delivered from that Guest to the exact Azure VM controller consumer over end-to-end `Noise_KK` records; the controller never performs a Host login, never holds refresh tokens, and never uses `DefaultAzureCredential`, environment variables, DBus, filesystem token paths, or a browser fallback. The Host and bus see ciphertext only. `Provider/credential-managed-identity` behavior remains unchanged.

| Method | Credential | Notes |
| --- | --- | --- |
| `AcquireToken(audience, leaseHandle)` | `armCredentialRef` | ARM token; enrolled KK; zeroized after ARM call via `AzureEffectPort` |

---

## Credential E2E KK contract

```
credential-managed-identity service (gateway Guest)
  OR D093 Entrablau service (Credential.identityGuestRef)
        │  enrolled Noise_KK ComponentSession
        │  prologue binds: Credential UID/gen, identityGuestRef/loginEndpointRef
        │  when present, audience, opId, schema, deadline, consumerRef
        ▼
azure-vm-controller (gateway Guest; exact consumer)
        │  ARM token bytes: zeroizing buffer; passed to AzureEffectPort only
        ▼
AzureEffectPort (real impl: azure_core/azure_mgmt_compute)
        │  TLS to management.azure.com; ****** in Authorization header
        ▼
Azure Resource Manager
```

**Invariants:**

1. Token bytes never appear in `Guest.status`, Zone store, audit records,
   OTEL spans/metrics, or log lines.
2. Token bytes are zeroizing in memory.
3. Controller never stores a token in the state Volume.
4. Credential acquisition is per-reconcile tick; no cross-tick token reuse.
5. No ambient credential fallback: no `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`,
   `MSI_ENDPOINT`, SDK environment credential chain, `DefaultAzureCredential`,
   Host login, DBus path, filesystem token path, or browser fallback.
6. ARM-consuming controller operations run inside the gateway Guest (ADR-0032).
   D093 Entra token issuance may originate in the same-Zone Entrablau identity
   Guest, but the Host and bus see only end-to-end KK ciphertext and refresh
   tokens never leave that identity Guest.

---

## RBAC and effect ports

### d2b Role/RoleBinding (controller subject)

```yaml
type: Role
metadata:
  name: azure-vm-controller
  zone: <zone>
spec:
  rules:
    - resourceTypes: [Guest]
      verbs: [get, list, watch, update-status, update-finalizers]
      subresources: []
      resourceNames: []
      zones: [<zone>]
      executionRefs: [Guest/<gateway-name>]
      sessionVerbs: []
    - resourceTypes: [Credential]
      verbs: [get, use-credential]
      subresources: [acquire-token, refresh-token]
      resourceNames: [<armCredentialRef-name>]
      zones: [<zone>]
      executionRefs: [Guest/<gateway-name>]
      sessionVerbs: []
    - resourceTypes: [Provider]
      verbs: [get]
      subresources: []
      resourceNames: [transport-azure-relay]
      zones: [<zone>]
      executionRefs: []
      sessionVerbs: []
    - resourceTypes: [Process]
      verbs: [get, create, update-spec, update-status, delete]
      subresources: []
      resourceNames: []
      zones: [<zone>]
      executionRefs: [Guest/<gateway-name>]
      sessionVerbs: []
---
type: RoleBinding
metadata:
  name: azure-vm-controller
  zone: <zone>
spec:
  roleRef: Role/azure-vm-controller
  subjects: [Process/azure-vm-controller]
  externalPrincipalSelector: null
  scopeNarrowing: null
```

Credential acquisition is not an `acquire-token` resource verb. Each call uses
`use-credential` with the exact matching subresource and is further narrowed by
`Credential.spec.allowedOperations`, `consumerRef`, scope, and structural
component checks. Finalizer changes use `update-finalizers`; there are no
`add-finalizer`, `remove-finalizer`, or `request-deletion` aliases.

### Azure RBAC

| Azure Role | Scope | Purpose |
| --- | --- | --- |
| `Virtual Machine Contributor` | Resource group | VM CRUD, extensions, disk operations |
| `Network Contributor` | VNet resource group | NIC operations |
| `Disk Contributor` | Resource group | OS and data disk management |

### Effect ports

| Port | Direction | Protocol | Notes |
| --- | --- | --- | --- |
| `AzureEffectPort` | Egress from gateway Guest via injected FD | ARM REST (HTTPS) | Never directly accessible from process network namespace; injected adapter only |
| Azure Relay hybrid connection | Egress from gateway Guest; ingress from VM | Noise over Relay WebSocket | Transport Provider owns relay credential/auth; no relay credential in this Provider |
| Zone bootstrap service listener | Ingress (transport-provided) | ComponentSession (Noise IKpsk2 then KK) | Never plain TCP |

The transport Provider (`transport-azure-relay`) owns its relay credentials.
This Provider declares no relay credential reference in `spec.config`.

---

## Status, errors, audit, and OTEL

### Guest.status extensions

Azure VM-specific ARM/session phase and opaque non-authorizing operation or
enrollment digests live only in `status.provider.details` with `providerRef:
Provider/runtime-azure-virtual-machine`, qualified `schemaId`
(`runtime-azure-virtual-machine.d2bus.org/Guest/status`), `schemaVersion`, and
`observedProviderGeneration`. Guest runtime readiness, capabilities, observed
lifecycle phase, bootstrap readiness, and active process count are promoted to
`status.resource` and remain identical to sibling Guest runtime providers.

```yaml
status:
  phase: Ready
  conditions:
    - type: AzureVmReady
      status: "True"
      reason: vm-running
    - type: BootstrapEnrolled
      status: "True"
      reason: kk-enrolled
    - type: CredentialReady
      status: "True"
      reason: arm-credential-active
  resource:
    observedLifecyclePhase: running
    runtimeReady: true
    bootstrapReady: true
    activeProcessCount: 0
  provider:
    providerRef: Provider/runtime-azure-virtual-machine
    schemaId: runtime-azure-virtual-machine.d2bus.org/Guest/status
    schemaVersion: 1.0.0
    observedProviderGeneration: 1
    details:
      providerPhase: Ready
      guestIdentityDigest: sha256:<hex-of-enrolled-noise-static-pubkey>
      azureOperationHandleDigest: sha256:<bounded-hex>
```

No ARM resource ID path, ARM resource URI, cloud subscription/tenant IDs,
poll URLs, PSK material, Noise key material, or raw operation handles appear in
status.

### Currency and expedited reconcile (D091/D090)

D091 currency is universal status, not Azure VM provider detail. The controller
implements `assess_update`, `plan_upgrade`, and `execute_upgrade`, populates
universal `status.update`, and keeps shared currency fields out of
`status.provider`; Azure-specific observations may appear only under
`status.provider.details`. Provider generation, VM image generation, or
security-policy changes set `status.update.state = UpdateAvailable` for
non-disruptive currency and `UpgradeRequired` for disruptive currency, with
`reasons = [ProviderGenerationChanged]`, `[ImageOrSystemGenerationChanged]`, or
`[SecurityPolicyChanged]`, `disruption = Recycle|Replace`, and
`preserveState = true` rather than applying disruption in place. Non-disruptive
changes reconcile normally. `execute_upgrade` recycles the Azure VM realization
while preserving the Guest UID/spec identity, sealed recovery Volume, enrolled
identity, and data Volumes; `Replace` is allowed only with explicit
ownership/state transfer. ARM operation/idempotency remains in the core
Operation ledger, and no secret enters `status.update`.

D090 expedited `waitForReconcile` on `Create`/`UpdateSpec`/`Delete` performs no
external effect, finalizer change, or status mutation until Core supplies
`CommittedRevisionProof {resourceUid,generation,revision,operationId}`. The
one-pass response returns the committed object, projected layered status,
disposition `Converged|Progressing|Blocked|UpgradeRequired|Failed`, and
`statusPersistence = pending|committed`; the durable commit is never rolled back
after a reconcile timeout. Effect idempotency keys derive from
`(UID,generation,revision,operationId)`, and the expedited pass uses the bounded
priority lane inside the same per-resource single-flight.

### Stable error codes

Closed set; never contain ARM error bodies, token text, internal paths,
subscription/tenant IDs, or ARM resource URIs.

| Error code | Meaning |
| --- | --- |
| `arm-quota-exceeded` | Azure subscription quota limit reached |
| `arm-resource-conflict` | ARM 409; VM with matching name but non-matching d2b tags |
| `arm-provisioning-failed` | ARM VM reached a failed provisioning state |
| `arm-network-unavailable` | VNet/subnet not found or not accessible |
| `arm-credential-denied` | ARM returned 401 or 403 |
| `arm-throttled` | ARM 429; retry in progress |
| `bootstrap-psk-expired` | PSK expired before VM connected |
| `bootstrap-psk-replayed` | Already-consumed PSK presented |
| `bootstrap-enrollment-failed` | IKpsk2 handshake failed |
| `bootstrap-failed` | Bootstrap deadline expired |
| `credential-unavailable` | `armCredentialRef` Credential not Ready |
| `deletion-ambiguous` | ARM deletion LRO unreachable; state unknown |
| `child-zone-drain-timeout` | Child Zone drain deadline exceeded |
| `image-change-requires-confirm` | `imageRef` change without `imageChangeConfirm` guard |
| `opaque-azure-ref-invalid` | A spec.provider.settings field failed OpaqueAzureRef validation |
| `adoption-zone-mismatch` | ARM VM tagged to a different Zone |

### Audit events

All pre-operation records are committed before the operation they describe.
The deletion audit record is appended after the durable store commit.

| Event code | Durability | Payload fields |
| --- | --- | --- |
| `azure-vm-provisioning-started` | informational | `zone`, `guestRef`, `operationId`, `region`, `subscriptionId` |
| `azure-vm-provisioning-complete` | durable | `zone`, `guestRef`, `operationId`, `result`, `errorCode?` |
| `azure-vm-psk-issued` | durable | `zone`, `guestRef`, `operationId`, `pskDigest: sha256(<psk-bytes>)` |
| `azure-vm-bootstrap-enrollment-complete` | durable-privileged | `zone`, `guestRef`, `operationId`, `enrolledKeyDigest: sha256(<pubkey-hex>)` |
| `azure-vm-adopted` | durable | `zone`, `guestRef`, `operationId` |
| `azure-vm-reconfigured` | durable | `zone`, `guestRef`, `operationId`, `changedFields: [field-names-only]` |
| `azure-vm-draining` | informational | `zone`, `guestRef`, `operationId` |
| `azure-vm-deleted` | durable | `zone`, `guestRef`, `operationId`, `result: success/partial/ambiguous` |

**Invariants:** no ARM token bytes, PSK plaintext, Noise private key, ARM
resource URI, ARM error body, subscription ID, tenant ID, or cloud endpoint
URL in any payload field. `pskDigest` is sha256 of PSK bytes, not the PSK itself.

### OTEL metrics

| Metric | Kind | Labels | Notes |
| --- | --- | --- | --- |
| `d2b_azure_vm_provision_total` | Counter | `result`, `error_code` | Closed label values only |
| `d2b_azure_vm_bootstrap_total` | Counter | `result` | - |
| `d2b_azure_vm_lro_poll_total` | Counter | `op_class`, `result` | `op_class` from closed operation table |
| `d2b_azure_vm_reconcile_duration_ms` | Histogram | `phase` | `phase` from closed providerPhase table |
| `d2b_azure_vm_credential_acquire_total` | Counter | `result` | ARM credential only |
| `d2b_azure_vm_active_guests` | Gauge | - | Count of Ready Azure VM Guests |
| `d2b_telemetry_drop_total` | Counter | `subsystem: azure-vm` | Dropped frames |

No VM name, resource group, subscription ID, tenant ID, ARM resource ID, ARM
URI, or OpaqueAzureRef value appears in any label.

---

## Quotas, backoff, and performance

### ARM backoff

| Status | Initial wait | Factor | Max retries | Max wait |
| --- | --- | --- | --- | --- |
| 429 `ThrottledRequest` | Per `Retry-After` (min 1 s; max 60 s) | 2.0× | 5 | 120 s |
| 503 `ServiceUnavailable` | 5 s | 2.0× | 3 | 60 s |
| 500 `InternalServerError` | 10 s | 1.5× | 2 | 30 s |

LRO poll intervals: 10 s initial; 1.2× backoff; 60 s cap.
Adapted from `d2b-realm-provider/src/rate_limit.rs` shape.

### Bootstrap timeouts

| Stage | Deadline |
| --- | --- |
| PSK generation + extension PUT | 120 s |
| VM execution + PSK delivery | `bootstrapDeadlineMs - 120 s` |
| IKpsk2 session establishment | 30 s |
| `CompleteEnrollment` KK call | 15 s |

PSK retry: one re-delivery after first deadline. Second expiry → `BootstrapFailed`.

---

## Nix artifact configuration

### Artifact catalog

```nix
d2b.artifacts.provider-runtime-azure-vm = {
  package = inputs.d2b-provider-azure-vm.packages.${system}.default;
  type = "provider";
};
```

### Provider resource (canonical spec.config shape)

```nix
d2b.zones.dev.resources.runtime-azure-virtual-machine = {
  type = "Provider";
  spec = {
    artifactId = "provider-runtime-azure-vm";
    config = {
      tenantId               = "2f8e1c3a-1234-5678-9abc-def012345678";
      clientId               = null;
      armCredentialRef       = "Credential/arm-azure-vm";
      controllerExecutionRef = "Guest/azure-relay-gateway";
      networkRef             = null;
    };
  };
};
```

### Credential declarations

```nix
# ARM credential - credential-managed-identity, guest-agent placement
d2b.zones.dev.resources.arm-azure-vm = {
  type = "Credential";
  spec = {
    providerRef = "Provider/credential-managed-identity";
    scope.executionRef = "Guest/azure-relay-gateway";
    audience    = "https://management.azure.com/";
    allowedOperations = [ "acquire-token" "refresh-token" ];
    rotation.policy = "proactive";
    rotation.proactiveWindowMs = 300000;
    consumerRef = "Provider/runtime-azure-virtual-machine";
  };
};
```

### Guest declaration (example)

```nix
d2b.zones.dev.resources.corp-vm = {
  type = "Guest";
  spec = {
    providerRef      = "Provider/runtime-azure-virtual-machine";
    systemArtifactId = null;     # cloud image; always null for Azure VM Guests
    defaultDomain    = "system";
    allowedDomains   = [ "system" ];
    defaultUserRef   = null;
    budget           = {};       # no d2b-side budget for remote VM Guests
    provider = {
      schemaId = "runtime-azure-virtual-machine.d2bus.org/Guest/spec";
      schemaVersion = "1.0.0";
      settings = {
        subscriptionId  = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        resourceGroup   = "d2b-workloads";
        region          = "eastus";
        vmSize          = "Standard_D4s_v5";
        imageRef        = "Canonical:ubuntu-24_04-lts:server:latest";
        diskSku         = "Premium_LRS";
        osDiskSizeGb    = 64;
        adminUser       = "azureuser";
        vnetName        = "d2b-workloads-vnet";
        subnetName      = "guests";
        assignPublicIp  = false;
        dataDisks       = [
          { lun = 0; diskClass = "high-perf"; sizeGb = 256; label = null; }
        ];
        bootstrapPskDelivery = "vm-extension";
        bootstrapDeadlineMs  = 600000;
        childZoneHosting     = false;
        azureTags = {
          environment = "development";
          owner       = "platform-team";
        };
      };
    };
  };
};
```

### Eval-time validation

1. Every `*Ref` resolves a declared resource of the stated type in the same Zone.
2. `subscriptionId`, `resourceGroup`, `region`, `vmSize`, `imageRef`,
   `vnetName`, `subnetName` pass OpaqueAzureRef charset and length bounds.
3. `adminUser` passes `^[a-z_][a-z0-9_-]*$` and max 64 chars.
4. `azureTags` keys have no `d2b:` prefix.
5. `dataDisks[*].lun` values are unique within the list (0..63).
6. `allowedDomains` contains only `system`.
7. `systemArtifactId = null` for any Guest whose `providerRef` resolves
   `Provider/runtime-azure-virtual-machine`.
8. `armCredentialRef` resolves a Credential with `allowedOperations` including
   `acquire-token`.
9. `tenantId` is non-null when `armCredentialRef` resolves `credential-entra`.
10. `spec.config.controllerExecutionRef` resolves a Guest in the same Zone.
11. Credential `scope.executionRef` matches `spec.config.controllerExecutionRef`
    for `credential-managed-identity` (guest-agent placement enforced).

---

## Upgrade and drain

### Controller upgrade (zero VM downtime)

1. New Provider resource generation applied.
2. Controller reads the sealed recovery Volume (enrollment/PSK/admission
   recovery material) and reads in-flight op records from the core Operation
   ledger.
3. `startup-relist` trigger; already-enrolled VMs adopted immediately.
4. In-progress op records: read the in-flight handle from the core Operation
   ledger → `AzureEffectPort::poll_lro` to re-derive state.
5. Bootstrap-svc restarts; enrolled KK sessions re-established.

### VM changes

- **Size change**: ARM dealloc + resize LROs (requeue-at). `providerPhase: Reconfiguring`.
- **Image change**: requires `imageChangeConfirm` guard. Without it: condition
  `ImageChangeRequiresConfirm`. With it: Drain → Delete → Provision cycle.

---

## Work items

### ADR046-azure-vm-001

| Field | Value |
| --- | --- |
| Dependency/owner | Provider contract owner |
| Current source | `d2bd/src/provider_registry.rs`: `AzureVmForbidden`, `AZURE_VM_IMPLEMENTATION_ID`; `d2b-realm-provider/src/provider.rs`: `InfrastructureProvider` (dead-reachable) |
| Reuse action | adapt |
| Destination | `src/{lib.rs,config.rs,schema.rs,error.rs,effect/mod.rs}` |
| Detailed design | Provider descriptor/manifest; `spec.config` schema; Guest spec.provider.settings schema; `AzureEffectPort` trait + `AzureOperationHandle`; `AzureVmError` enum; `SandboxSpec` with semantic classes; `BudgetSpec` with SI suffix memory fields; `restartPolicy` class/backoffBase/backoffMax; `networkUsage.allowEgress=false`; Endpoint ResourceType templates with name/transport/purpose Primary reuse disposition: `adapt`. Preserved source-plan detail: Extract and adapt; DELETE `InfrastructureProvider` after this Provider is operational. |
| Integration | ProviderDeployment loads the descriptor/catalog and ResourceType schemas; Nix and Guest specs reference the provider settings; controller and EffectPort modules consume the shared config/error types. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import. Existing registry sentinels are deleted only after the Provider resource model replaces them. |
| Validation | Provider catalog; descriptor fingerprint; schema/conformance tests |
| Removal proof | `InfrastructureProvider` deleted; `AzureVmForbidden` removed after Provider resource model replaces registry |

### ADR046-azure-vm-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-azure-vm-001 |
| Current source | `d2b-realm-provider/src/rate_limit.rs` (implemented-and-reachable) |
| Reuse action | adapt |
| Destination | `src/effect/{mod.rs,real.rs,fake.rs,rate_limit.rs}` |
| Detailed design | `AzureEffectPort` async trait; opaque `AzureOperationHandle` (bounded bytes, no poll URL); real `azure_core`/`azure_mgmt_compute` impl; `FakeAzureEffectPort` for hermetic tests; ARM 429/503/409 handling Primary reuse disposition: `adapt`. Preserved source-plan detail: Copy and adapt. |
| Integration | Azure VM controller lifecycle/idempotency code calls `AzureEffectPort`; the real implementation talks to ARM in production and `FakeAzureEffectPort` drives hermetic lifecycle tests. |
| Data migration | No persistent data migration; in-flight ARM operation handles are new v3 status/core-ledger records and are re-derived or adopted on reconcile when absent. |
| Validation | `tests/lifecycle_hermetic.rs`; all ARM paths via `FakeAzureEffectPort`; no ARM URL in test assertions |
| Removal proof | Old `InfrastructureProvider` ARM simulation deleted after parity |

### ADR046-azure-vm-003

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-azure-vm-001; ADR046-azure-vm-002; Guest ResourceType controller contract |
| Current source | `d2b-realm-provider/src/conformance.rs`; main `a1cc0b2d`: `d2b-provider-toolkit/src/reconciler_loop.rs` |
| Reuse action | adapt |
| Destination | `src/controller/{mod.rs,lifecycle.rs,idempotency.rs}` |
| Detailed design | Non-blocking reconcile: `start_*(...)` → persist `AzureOperationHandle` → `requeue-at`; `poll_lro` on subsequent ticks; controller as authorized `update-status` writer for Guest resources; finalizer held until ARM delete confirmed; top-level `phase`, `status.resource`, and Azure `status.provider.details.providerPhase` written atomically Primary reuse disposition: `adapt`. Preserved source-plan detail: Copy/adapt main toolkit; adapt conformance shape. |
| Integration | Zone core dispatches Guest resource events to the Azure VM controller; ResourceClient updates status/finalizers; `AzureEffectPort` starts, polls, and deletes ARM LROs. |
| Data migration | Full d2b 3.0 reset; old WorkloadProvider lifecycle state is not imported. Existing ARM resources may be adopted by tag/idempotency checks during reconcile. |
| Validation | `tests/lifecycle_hermetic.rs`; `tests/conformance.rs` |
| Removal proof | Old `WorkloadProvider::provision`/`deprovision` paths retired |

### ADR046-azure-vm-004

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-azure-vm-001; ComponentSession IKpsk2 |
| Current source | `d2b-realm-provider/src/types.rs`: `ProviderGuestdBootstrapContract` (implemented-and-reachable); main `a1cc0b2d`: `d2b-session/src/bootstrap.rs` |
| Reuse action | adapt |
| Destination | `src/controller/bootstrap.rs`; `src/bootstrap_svc/{mod.rs,admission.rs,enrollment.rs}` |
| Detailed design | PSK generation; sealed PSK/admission/enrollment recovery material (ciphertext) in the controller's single guest-local sealed recovery Volume; `GrantBootstrapAdmission` typed bus call; IKpsk2 in bootstrap-svc; enrollment record; enrolled KK; the bootstrap-svc declares **no** state Volume (session state in process memory; obtains sealed PSK/admission from the controller only); the controller's sealed recovery Volume is an ordinary Volume resource created by core ProviderDeployment (before component Process start) from the controller's single `stateNamespaces` declaration with a Nix-preprovisioned `User/azure-vm-controller` layout principal; ARM operation/idempotency records live in the core Operation ledger and non-secret observed cloud phase lives in `Guest.status` (D087); controller does not own, create, or add Volume to exported ResourceTypes; it consumes its view dirfd only Primary reuse disposition: `adapt`. Preserved source-plan detail: Copy/adapt main `BootstrapPsk`/`BootstrapAdmission`. |
| Integration | Controller creates and seals recovery material in its state Volume, grants bootstrap admission over the bus, and bootstrap-svc performs IKpsk2 enrollment for Guest sessions. |
| Data migration | No v2 bootstrap state import; the new sealed recovery Volume is initialized on first v3 activation, and old vsock bootstrap material is retired at cutover. |
| Validation | `tests/bootstrap_hermetic.rs`; `tests/error_redaction.rs` |
| Removal proof | Old vsock bootstrap path removed at v3 cutover |

### ADR046-azure-vm-005

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-azure-vm-003; Credential ResourceType; D055/D056 |
| Current source | `d2b-realm-provider/src/credential.rs`: `AzureControlPlaneRef`, `OpaqueAzureRef`, `ManagedIdentityRef` (implemented-and-reachable) |
| Reuse action | adapt |
| Destination | `src/credential.rs` |
| Detailed design | ARM credential via enrolled KK `AcquireToken`; zeroizing token handling; no ambient credential fallback; `credential-managed-identity` guest-agent placement Primary reuse disposition: `adapt`. Preserved source-plan detail: Retain `OpaqueAzureRef` directly; adapt credential acquisition to enrolled KK. |
| Integration | Controller obtains ARM credentials through enrolled KK and the Credential ResourceType before EffectPort operations; the credential-managed-identity guest agent provides the token source. |
| Data migration | No ambient credential migration; v3 requires ResourceType Credential/ManagedIdentityRef plus enrolled KK, and the old direct IMDS fallback is removed. |
| Validation | `tests/credential_hermetic.rs`; `tests/error_redaction.rs` |
| Removal proof | Old direct IMDS calls from controller removed |

### ADR046-azure-vm-006

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-azure-vm-003 |
| Current source | `d2bd/src/provider_registry.rs`: `NEXT_LIFECYCLE_OPERATION_ID: AtomicU64` (production-reachable) |
| Reuse action | adapt |
| Destination | `src/controller/idempotency.rs` |
| Detailed design | Deterministic ARM request ID derivation; `AzureOperationHandle` opaque persistence (no poll URL in state); ARM 409 adoption; finalizer held through async deletion Primary reuse disposition: `adapt`. Preserved source-plan detail: Adapt to deterministic per-Guest keys. |
| Integration | Lifecycle controller stores deterministic request IDs and opaque handles in the core Operation ledger/status; restart recovery reads them before polling or adopting ARM operations. |
| Data migration | Old `AtomicU64` operation IDs are not imported; v3 operations use deterministic keys, while missing handles are re-derived or adopted from ARM. |
| Validation | `tests/idempotency.rs`; restart-recovery scenario |
| Removal proof | `AtomicU64` lifecycle op ID removed after all ARM callers migrate |

### ADR046-azure-vm-007

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-azure-vm-001; ADR-046-provider-state; ADR-046-nix-configuration |
| Current source | `nixos-modules/options-realms-workloads.nix`: `WorkloadProviderKind::ProviderManaged` |
| Reuse action | adapt |
| Destination | `nixos-modules/` (Provider/Guest resource emitters); crate Nix build |
| Detailed design | Nix `spec.config` shape; `controllerExecutionRef`/`networkRef` eval-time assertions; no Volume refs for data disks; `systemArtifactId=null` enforcement; the single controller sealed recovery Volume is an ordinary Volume resource created by core ProviderDeployment (not in Zone bundle; not operator-authored); the bootstrap-svc declares no state Volume; guest-local placement - reconciled by the Guest-local volume-local instance and expressed by `source.executionRef` = config gateway Guest; host MUST NOT hold ARM binding, admission, PSK, or operation state; ARM operation/idempotency records live in the core Operation ledger and non-secret observed cloud phase in `Guest.status` (D087); no virtiofs or host-to-guest attachment; manifest freezes guest-local with no fallback; controller does not create, own, or list Volume in exported ResourceTypes; `Provider/volume-local` is the sole Volume reconciler; controller consumes required view dirfd only; **the recovery Volume is `kind: state`, `persistenceClass: persistent`, `storageNeed: secret`, sealed via `sealingCredentialRef`, with nonzero `quotaBytes`, `quota.maxBytes`, `quota.maxInodes`, and `source.settings.sourcePolicyId`; `persistenceClass: ephemeral` and zero quotas are rejected**; it survives component/Provider restart and participates in upgrade/destroy/reset; full canonical Volume spec including `stateSchema`, `source`, `layout` with a Nix-preprovisioned `User/<name>` principal (not ComponentPrincipal), `views`, `identityMarker`, `snapshotPolicy: null`, `retentionPolicy: null`; `sensitivityClass: private` and `volume-domain-mismatch` isolation enforced; canonical `SandboxSpec` fields with `namespaceClasses`/`capabilityClasses`/`seccompClass`/`noNewPrivileges`/`startRoot`/`environmentClass`/`readOnlyRoot`; `BudgetSpec` with SI suffix; `restartPolicy` class/backoffBase/backoffMax; Endpoint ResourceType templates with name/transport/purpose |
| Integration | Nix emitters produce Provider, Guest, Volume, and Endpoint resource specs consumed by ProviderDeployment, `Provider/volume-local`, the Process Provider, and the Azure VM controller. |
| Data migration | Full d2b 3.0 reset; old `d2b.realms.<r>.workloads.<w>` config is replaced by v3 resource authoring with no automatic v2 config import. |
| Validation | Nix eval tests; `make test-flake`; `make test-drift` |
| Removal proof | `d2b.realms.<r>.workloads.<w>` removed at v3 cutover |

### ADR046-azure-vm-008

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-azure-vm-003; ADR-046-telemetry-audit-and-support |
| Current source | `d2bd/src/metrics.rs` (production-reachable) |
| Reuse action | adapt |
| Destination | `src/{telemetry.rs,audit.rs}` |
| Detailed design | Closed metric labels; OTEL span attributes; audit durability classes; `azure-vm-deleted` appended post-commit; no ARM URI, ARM resource ID, or cloud endpoint in any telemetry surface Primary reuse disposition: `adapt`. Preserved source-plan detail: Adapt audit shape; replace Prometheus with d2b-telemetry emitter. |
| Integration | Controller/error paths call telemetry and audit emitters after status commits; d2b-telemetry consumes the metrics/spans and policy_observability enforces redaction. |
| Data migration | No metrics/audit data migration; new OTEL/audit surfaces start at v3 cutover and the old Prometheus registry is retired. |
| Validation | `tests/error_redaction.rs`; `d2b-contract-tests/tests/policy_observability.rs` updated |
| Removal proof | `d2bd/src/metrics.rs` hand-rolled registry removed after observability-otel Provider integration |

### ADR046-azure-vm-009

| Field | Value |
| --- | --- |
| Dependency/owner | All ADR046-azure-vm-* |
| Current source | No existing Azure VM tests at baseline; fake/hermetic patterns from main `a1cc0b2d` |
| Reuse action | adapt |
| Destination | `tests/`; `integration/` |
| Detailed design | See §Test requirements Primary reuse disposition: `adapt`. Preserved source-plan detail: Copy/adapt fake toolkit; write new tests. |
| Integration | Provider crate tests, fake toolkit, and integration harness run under cargo/Layer-1 and validate all ADR046-azure-vm-* outputs together. |
| Data migration | None - test-only work; no runtime state. Old mock tests are removed only after parity. |
| Validation | All tests pass |
| Removal proof | Old `InfrastructureProvider` mock tests deleted after parity |

---

## Test requirements

### Fast hermetic execution and test placement (D094)

Per D094 and `ADR-046-validation-and-delivery` §10.16, this Provider's `src/`
unit tests and `tests/*.rs` hermetic suite are fast, in-process, deterministic,
and parallel-safe: an individual normal test has an advisory wall-clock p95
diagnostic threshold of <=50 ms; gate enforcement is aggregate per-crate
process CPU only. There is no wall-clock
sleep, and `cargo test -p d2b-provider-runtime-azure-virtual-machine --lib --tests`
completes in ≤2 s warm-cache execution time (compilation excluded). They use a
deterministic fake clock/RNG and the toolkit fakes/FakeEffectPort only - no
process spawn, container, network, DBus, systemd, broker daemon, Nix eval/build,
KVM, USB/GPU/TPM hardware, or live cloud, and no filesystem tree beyond tiny
temp fixtures. Any scenario needing those lives only in `integration/`, which
keeps a lane timeout/budget, parallel isolation, and fake external services by
default; such a need is re-placed into `integration/`, never given a sleep,
larger timeout, or `#[ignore]`. Bounded crypto/property tests are the only
classified exception, each named with a capped case count and a declared higher
per-test advisory threshold.

### tests/ (hermetic; no external processes)

Run with `cargo test -p d2b-provider-runtime-azure-virtual-machine`.

| File | Coverage |
| --- | --- |
| `tests/conformance.rs` | `d2b-provider-toolkit::conformance::check_provider_conformance`; descriptor; schema exports; `SandboxSpec` semantic class fields present; Endpoint ResourceType template includes `purpose` field; `Volume` absent from exported ResourceTypes (only `Guest` exported; Volume owned/reconciled by core/volume-local) |
| `tests/lifecycle_hermetic.rs` | `FakeAzureEffectPort`; full non-blocking LRO state machine via `requeue-at`: `Absent → Provisioning → PskDelivering → Bootstrapping → Ready → Reconfiguring → Draining → Deleting → Finalized`; ARM 429 retry; ARM 409 → adoption; `ProvisionFailed`; `BootstrapFailed`; `DeletionAmbiguous` finalizer hold; post-commit audit append; controller as authorized `update-status` writer; `FakeAzureEffectPort` never exposes ARM poll URL to assertions |
| `tests/bootstrap_hermetic.rs` | PSK single-use; sealed PSK ciphertext in Volume (plaintext never in Volume); `GrantBootstrapAdmission` single-session delivery; IKpsk2 snow 0.10 vectors; PSK expiry; tampered IKpsk2 rejected; enrollment record sealed; bootstrap-svc mount of controller Volume rejected with `volume-domain-mismatch`; bootstrap-svc receives only its own `admission` view dirfd |
| `tests/credential_hermetic.rs` | Fake enrolled KK; `AcquireToken` → token bytes via `AzureEffectPort` only; token bytes absent from status/audit/OTEL; zeroized after ARM call; no ambient fallback fires |
| `tests/idempotency.rs` | Deterministic request ID derivation; same input → same ID; `AzureOperationHandle` opaque (no URL leaked); ARM 409 → adoption; restart recovery (handle present → `poll_lro`; handle absent → `get_vm_state`); finalizer held through ambiguity |
| `tests/error_redaction.rs` | Canary bytes: ARM error body, ARM token, PSK plaintext, enrolled Noise pubkey, ARM poll URL, ARM resource URI. Must not appear in `Guest.status`, audit records, OTEL attributes, metric labels, or log lines. Hard test error on any match. |
| `tests/schema_validation.rs` | `spec.config` JSON Schema; `spec.provider.settings` JSON Schema; OpaqueAzureRef charset; `adminUser` charset; `diskSku` closed enum; `dataDisks` LUN uniqueness; no `sshCredentialRef` field accepted; no Volume refs in `dataDisks`; `systemArtifactId=null`; `azureTags` `d2b:*` prefix rejection; the controller declares exactly one guest-local sealed recovery Volume (`storageNeed: secret`, `sealingCredentialRef` set) round-trip: guest-local placement is manifest-frozen and expressed by `source.executionRef` = gateway Guest, layout `ownerRef: User/<name>` (numeric UID string rejected), `views`, `identityMarker`, `snapshotPolicy: null`, `retentionPolicy: null`, `quotaBytes`/`quota.maxBytes`/`quota.maxInodes`/`source.settings.sourcePolicyId` present and nonzero; the bootstrap-svc declares no state Volume; `sensitivityClass: private`; `persistenceClass: persistent`; `persistenceClass: ephemeral` rejected; zero `quota.maxBytes`/`quota.maxInodes`/`quotaBytes` rejected; host-backed placement rejected (`guest-local-required`) |
| `tests/fault_injection.rs` | ARM 429 → retry + succeed; ARM 503 persistent → `ProvisionFailed`; PSK first expiry → retry; PSK second expiry → `BootstrapFailed`; ARM credential unavailable → `CredentialUnavailable`; enrollment PSK replay rejected; controller restart mid-LRO → handle recovery |

### integration/ layout and boundaries

```text
integration/
  README.md
    # Container tests: podman/docker; no Azure subscription required.
    # mock_arm_server: scripted ARM REST HTTP server in mock_arm_server/.
    # FakeAzureEffectPort: used in hermetic tests (tests/); NOT in container tests.
    # Container tests use mock_arm_server (real HTTP, no ARM credentials).
    # Live tests: D2B_AZURE_VM_LIVE_TEST=1 plus env vars below.
    # Live env vars required:
    #   D2B_AZURE_TEST_SUBSCRIPTION   OpaqueAzureRef
    #   D2B_AZURE_TEST_RESOURCE_GROUP OpaqueAzureRef
    #   D2B_AZURE_TEST_REGION         OpaqueAzureRef
    #   D2B_AZURE_TEST_VNET           OpaqueAzureRef
    #   D2B_AZURE_TEST_SUBNET         OpaqueAzureRef
    #   D2B_AZURE_TEST_MI_CLIENT_ID   OpaqueAzureRef (managed identity client ID)
    # Pre-requisites: Azure RBAC provisioned; gateway Guest running;
    #   managed identity Credential registered in Zone.
    # Live tests incur Azure billing costs. Each test deletes provisioned
    #   resources on completion (success or failure).
    # Build: cargo build -p d2b-provider-runtime-azure-virtual-machine --bins
    # Container run: cargo test -p ... --test lifecycle_container
  mock_arm_server/
    main.rs              # Hyper HTTP server; scripted ARM REST responses; never
                         # called from hermetic tests (FakeAzureEffectPort used there)
    scenarios/
      happy_path.json
      conflict.json      # VM PUT → 409
      throttled.json     # VM PUT → 429 then 201
      partial_delete.json
      lro_in_progress.json
  lifecycle_container.rs
    # mock_arm_server as child process.
    # Controller + bootstrap-svc binaries with fake gateway Guest context.
    # Full non-blocking lifecycle: Provisioning (requeue-at) →
    # PskDelivering (requeue-at) → Bootstrapping (fake IKpsk2 inline) → Ready
    # → Reconfiguring → Draining → Deleting (requeue-at) → Finalized.
    # Asserts: audit events ordered; status conditions correct; finalizer cleared
    # only after delete LRO confirms; no ARM poll URL in test-observable state.
  adoption_container.rs
    # mock_arm_server seeded with pre-existing VM with correct d2b tags.
    # Verifies controller adopts without re-provisioning.
  bootstrap_roundtrip.rs
    # bootstrap-svc as subprocess; no mock_arm_server.
    # Inline Rust VM agent: IKpsk2 (fake PSK via GrantBootstrapAdmission) →
    # enrolled KK enrollment.
    # Verifies: enrollment record sealed+written; identity registry updated;
    # PSK replay rejected; bootstrap-svc has no file access to controller Volume.
  live/
    README.md
    lifecycle_live.rs    # provision → bootstrap → ready → size-change → drain → delete
    adoption_live.rs     # pre-create ARM VM with d2b tags; verify adoption
```

**`FakeAzureEffectPort` / `mock_arm_server` boundary:**

- **Hermetic tests** (`tests/`): use `FakeAzureEffectPort` (in-process; no HTTP).
  All ARM paths exercised without network. `FakeAzureEffectPort` never exposes
  ARM poll URLs or ARM resource URIs to test assertions.
- **Container integration tests** (`integration/*_container.rs`): use
  `mock_arm_server` (real HTTP to local process). Controllers run as separate
  binaries with injected test configuration pointing to `mock_arm_server`.
- **Live tests** (`integration/live/`): use real Azure ARM. Require env vars
  and Azure RBAC pre-provisioned. Guarded by `D2B_AZURE_VM_LIVE_TEST=1`.

The `FakeAzureEffectPort` is defined behind `#[cfg(test)]`. No `azure_core`
dependency is required for hermetic test builds.

---

## Removal proof

| Item | Current path | Removal condition |
| --- | --- | --- |
| `InfrastructureProvider` trait | `d2b-realm-provider/src/provider.rs` | After controller handles all prior `WorkloadProviderKind::ProviderManaged` Azure VM workloads |
| `AzureVmForbidden` error variant | `d2bd/src/provider_registry.rs` | After registry composition replaced by Provider resources |
| `AZURE_VM_IMPLEMENTATION_ID` constant | `d2bd/src/provider_registry.rs` | Same |
| `WorkloadProviderKind::ProviderManaged` (Azure VM branch) | `d2b-realm-core/src/workload.rs` | After `Guest.spec.providerRef` handles all former `ProviderManaged` paths |
| `ProviderGuestdBootstrapContract` vsock path for Azure VMs | `d2b-realm-provider/src/types.rs` | After v3 IKpsk2 bootstrap fully replaces vsock/SSH path |
| `d2b.realms.<r>.workloads.<w>` Nix option | `nixos-modules/options-realms-workloads.nix` | After all Azure VM workloads use `d2b.zones.<z>.resources.<name>` |

No removal happens before the successor is integrated and tested. Each removal
requires a dedicated commit with the appropriate wave-tag per AGENTS.md
§Commit conventions.

Per D094, each replaced current-code test is retired with an explicit
keep/adapt/move/delete disposition and a removal gate: the minimum reusable
semantic assertions migrate into this crate's hermetic `tests/`, and the old
duplicate tests, shell gates, fixtures, static artifacts, CI jobs, and manifest
entries are deleted once successor coverage and the removal proof pass -
updating `tests/layer1-jobs.json`, the closed gate manifests, the
flake/matrix/Nix-unit pins, the generated ledgers, and the CI workflow shards.
Old and new suites never run in parallel indefinitely.
