# ADR 0045: Workload-hosted realm controllers and shared-relay shortcuts

- Status: Proposed
- Date: 2026-07-10
- Refines: [ADR 0035](0035-efficiency-and-simplification-roadmap.md)
  (provider naming and workspace simplification), [ADR 0043](0043-realm-native-control-plane.md)
  (realm-native control plane), [ADR 0044](0044-unsafe-local-runtime-provider.md)
  (unsafe-local runtime provider)
- Related: [ADR 0010](0010-wire-protocol-and-typed-errors.md)
  (wire protocol and typed errors), [ADR 0028](0028-guest-control-plane-over-vsock.md)
  (guest control plane over virtio-vsock), [ADR 0034](0034-storage-lifecycle-restart-and-synchronization.md)
  (storage lifecycle, restart adoption, and synchronization),
  [ADR 0037](0037-local-hypervisor-runtime-seam.md)
  (local hypervisor runtime seam)

## Context

ADR 0043 gives every realm one controller generation and allows that controller
to run on the host, in a gateway VM, on a cloud full host, or in a constrained
provider environment. Its current placement vocabulary describes where the
controller runs, but it does not fully model the controller as a workload
managed by another realm.

That leaves several ambiguities:

- a gateway VM looks like a special realm object even though it is a VM with a
  controller role;
- the controller VM can appear to be owned by the realm it must bring into
  existence, creating a lifecycle cycle;
- `provider` can mean a workload runtime, infrastructure provisioner, realm
  controller host, or relay transport;
- the existing `d2b.realms.<realm>.providers` records do not distinguish those
  responsibilities;
- relay configuration says how a realm is reachable, but not which workload
  owns interactive credentials or opens the connector;
- ADR 0043 authorizes direct transport shortcuts only over native underlay
  reachability, even when every participant already uses one shared relay
  fabric.

The concrete motivating case is a work realm backed by Azure:

1. Azure Resource Manager and Azure Relay authentication require Microsoft
   Entra credentials acquired inside an Entra-joined, Intune-managed local
   Cloud Hypervisor VM.
2. Interactive authentication requires a physical YubiKey.
3. The same YubiKey is also used for browser authentication in a work desktop
   VM and for delegated developer authentication in another work VM.
4. A remote Azure VM may run the full work realm controller.
5. Code running locally or remotely needs access to Azure resources without
   receiving the controller's ARM or Relay token cache.
6. Nested realms may all use the same Azure Relay fabric. Relaying every byte
   through each intermediate controller would add latency and failure domains
   without adding policy value.

The existing security invariants remain binding:

- host `d2bd` and `d2b-priv-broker` hold no realm provider, Relay, or Entra
  credentials;
- relay authentication establishes reachability only and never maps to local
  `Admin`, broker authority, or realm identity;
- one remote, work, or provider realm has one credential boundary by default;
- a realm controller cannot own or repair the infrastructure on which that
  controller itself runs;
- parent and ancestor policy must constrain every cross-realm operation even
  when bytes do not traverse those controllers;
- the same physical authenticator may serve multiple isolated VMs, but token
  caches, refresh tokens, and managed identities are never shared between
  them.

## Decision

### Providers and workload roles are separate concepts

D2b will use the following vocabulary:

| Concept | Responsibility |
| --- | --- |
| Workload runtime provider | Starts, stops, inspects, and executes one workload. |
| Infrastructure provider | Provisions or adopts infrastructure on which workloads, including controller workloads, run. |
| Relay provider | Supplies rendezvous and byte transport for authenticated d2b peer sessions. |
| Workload role | Declares an authority-bearing function performed by a workload, such as running a realm controller. |

The unqualified term `realm provider` is too ambiguous for a public schema and
must not name a new catch-all interface. An adapter may implement more than one
provider trait, but each binding names the trait being used. For example:

- `azure-vm` can provision a remote VM as an infrastructure provider and can
  supervise that VM as a workload runtime provider;
- `azure-relay` is a relay provider;
- a VM created by `azure-vm` may carry the `realmController` workload role;
- Cloud Hypervisor, QEMU, Bubblewrap, and Minijail are workload runtime
  providers, not realm controllers by themselves.

Provider identifiers describe adapters, not security claims. Isolation,
execution identity, environment source, display routing, networking, device
access, and persistence remain typed execution-posture fields.

### Provider crates use a type-first sortable namespace

Every provider implementation crate uses this grammar:

```text
d2b-provider-<provider-type>-<implementation>
```

The provider type immediately follows `d2b-provider-` so workspace listings,
dependency graphs, generated inventories, and code search group all
implementations of the same authority together.

The selected provider crate namespaces are:

| Crate prefix | Standard interface | Responsibility |
| --- | --- | --- |
| `d2b-provider-api` | Interface crate | Provider descriptors, typed errors, operation context, capability DTOs, and provider traits. Contains no provider SDK or runtime implementation. |
| `d2b-provider-runtime-<implementation>` | `RuntimeProvider` | Plans, starts, stops, adopts, and inspects workloads. |
| `d2b-provider-infrastructure-<implementation>` | `InfrastructureProvider` | Provisions, adopts, inspects, and deletes infrastructure that hosts workloads or realm controllers. |
| `d2b-provider-relay-<implementation>` | `RelayProvider` | Opens relay sessions and creates or revokes scoped rendezvous bindings. |
| `d2b-provider-substrate-<implementation>` | `SubstrateProvider` | Checks and prepares a full-host OS substrate such as NixOS or generic Linux. |
| `d2b-provider-credential-<implementation>` | `CredentialProvider` | Acquires or reports credentials inside the configured credential-owning workload without exporting them to the host. |
| `d2b-provider-display-<implementation>` | `DisplayProvider` | Implements a reusable display/session adapter independent of one runtime backend. |
| `d2b-provider-testkit` | Conformance harness | Provider mocks, deterministic fixtures, and reusable conformance suites. It is not linked into production binaries. |

The words after the provider type name the canonical implementation, not a
configured instance. The provider type is not repeated in that segment:

- `d2b-provider-runtime-cloud-hypervisor`;
- `d2b-provider-runtime-qemu-media`;
- `d2b-provider-runtime-systemd-user`;
- `d2b-provider-runtime-bubblewrap`;
- `d2b-provider-runtime-azure-container-apps`;
- `d2b-provider-infrastructure-azure-vm`;
- `d2b-provider-relay-azure`;
- `d2b-provider-credential-entra`;
- `d2b-provider-substrate-nixos`;
- `d2b-provider-display-wayland`.

For example, `d2b-provider-relay-azure` has
`ProviderType::Relay` and implementation id `azure`; its complete public
provider kind may still render as `azure-relay`. A configured deployment may
then assign instance ids such as `work-relay` or `payments-relay` without
changing the crate or implementation id.

Abbreviated or axis-free crate names such as `d2b-provider-aca`,
`d2b-provider-relay`, `d2b-provider-azure`, and
`d2b-host-providers` are forbidden after the cutover. A vendor with multiple
provider types gets one crate per authority boundary. For example, ordinary
Azure VM workload lifecycle and Azure VM infrastructure provisioning sort
separately:

```text
d2b-provider-runtime-azure-vm
d2b-provider-infrastructure-azure-vm
```

Vendor SDK plumbing shared by those implementations stays in a private module
of one implementation until two real consumers justify a narrow shared crate.
If a shared crate is required, its name must describe the specific SDK
capability; `common`, `util`, `manager`, and an axis-free
`d2b-provider-azure` remain disallowed.

This type-first grammar supersedes ADR 0035's examples
`d2b-provider-hypervisor-<name>`, `d2b-provider-<name>`, and
`d2b-constellation-transport-<name>`. Hypervisors are runtime providers and
relay transports are relay providers, so they sort under the same type axes as
their peers.

### Every provider implements a standard base interface

`d2b-provider-api` replaces `d2b-realm-provider` as the narrow,
implementation-free interface crate. It depends only on codec-neutral realm
DTOs and the minimum async/I/O traits needed by the interfaces. It must not
depend on:

- `d2bd`, the privileged broker, or host mutation implementations;
- a cloud SDK, HTTP client, TLS implementation, or concrete transport;
- a protocol codec;
- a provider implementation crate;
- test mocks or live-provider fixtures.

Every registered provider implements the common base interface:

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    async fn health(&self) -> ProviderResult<ProviderHealth>;
}
```

`ProviderDescriptor` is bounded, non-secret data containing:

- the configured `ProviderId`;
- one closed `ProviderType`;
- the canonical implementation id;
- the provider API version;
- positive capability assertions;
- the implementation's configuration-schema fingerprint.

It never contains credentials, token subjects, endpoints, resource ids, command
arguments, host paths, or provider response bodies.

The closed primary provider types are:

```rust
pub enum ProviderType {
    Runtime,
    Infrastructure,
    Relay,
    Substrate,
    Credential,
    Display,
}
```

Each configured provider instance has exactly one primary provider type and is
registered in the matching typed registry. Implementations may additionally
implement optional capability interfaces such as persistent shell, durable
execution, console, audio, guest-control endpoint, or observability export.
Those capabilities do not change the provider's primary authority type.

The specialized interfaces extend `Provider`:

| Interface | Required semantic surface |
| --- | --- |
| `RuntimeProvider` | Capability description; plan; idempotent ensure/start; stop; inspect/adopt; destroy when the runtime owns durable workload state. |
| `InfrastructureProvider` | Capability description; plan; apply; adopt; inspect; bootstrap binding; destroy. |
| `RelayProvider` | Capability description; connect/listen; issue scoped rendezvous binding; revoke binding; inspect transport health. |
| `SubstrateProvider` | Capability description; check; plan remediation; apply only through the authorized substrate owner. |
| `CredentialProvider` | Non-secret status; interaction requirement; acquire or refresh only for a co-located typed consumer; revoke. |
| `DisplayProvider` | Capability description; open and close an already-authorized display session. |

The existing local-only `RuntimeProvider` and provider-managed
`WorkloadProvider` split is retired. Local VMMs, host-user runtimes, container
sandboxes, provider-managed sandboxes, and remote VM runtimes implement one
`RuntimeProvider` lifecycle contract. Exec, persistent shell, display, console,
audio, and guest-control remain optional capability interfaces rather than
being folded into runtime lifecycle.

Every mutating specialized-provider method receives a bounded
`ProviderOperationContext` containing:

- the stable operation id and idempotency key;
- the already-authorized realm and workload/controller identity;
- the required capability;
- deadline and cancellation state;
- a redacted trace correlation id.

The provider does not reinterpret local users, authorize a principal, or choose
another realm. The realm controller performs authorization before dispatch.
The provider verifies that the operation context matches its configured scope,
then performs only its typed action.

All provider interfaces share these semantics:

1. Unsupported behavior returns a typed capability denial. It never falls back
   to SSH, a shell command, generic TCP, ambient developer credentials, or
   another provider.
2. Mutations are idempotent by operation id and return typed observed state.
3. Plans and handles contain opaque provider refs, not raw credentials or
   unbounded provider responses.
4. `Debug`, tracing, audit, and metrics redact credentials, endpoints, resource
   ids, user identities, and provider payloads.
5. Timeouts, cancellation, retry classification, and degraded state are part of
   the interface contract.
6. Restart adoption verifies provider identity and operation binding before
   accepting observed state.
7. A provider implementation cannot call the broker directly. Host mutations
   are re-originated by the owning daemon through typed broker operations.

### Conformance is mandatory

`d2b-provider-testkit` owns reusable conformance suites for every primary
provider interface. An implementation crate is not registered or advertised as
supported until it passes the suite for its primary type and every optional
capability it advertises.

The suites prove at minimum:

- descriptor type and implementation id match the crate axis;
- capability advertisement is positive and unsupported operations fail closed;
- operation ids make retries idempotent;
- cancellation and deadlines are bounded;
- inspect/adopt rejects identity or generation mismatch;
- no secret-shaped value appears in debug, error, audit, or metric output;
- handles and plans survive their documented serialization boundary;
- provider-specific errors map to stable `ProviderError` kinds and retry hints;
- a provider cannot widen the authorized realm, workload, operation, or
  capability from `ProviderOperationContext`.

Cloud implementation crates keep live tests explicitly opt-in. Hermetic
conformance uses fake SDK clients and transports supplied by the implementation
crate, while shared mocks and assertions remain in `d2b-provider-testkit`.

### Existing provider crates migrate explicitly

The implementation cutover uses this map:

| Current crate | Selected replacement |
| --- | --- |
| `d2b-realm-provider` | `d2b-provider-api` |
| `d2b-host-providers` | Split into `d2b-provider-runtime-cloud-hypervisor`, `d2b-provider-runtime-qemu-media`, `d2b-provider-substrate-nixos`, `d2b-provider-substrate-linux`, and `d2b-provider-display-wayland`. |
| `d2b-provider-aca` | `d2b-provider-runtime-azure-container-apps` |
| `d2b-provider-relay` | `d2b-provider-relay-azure` |
| Provider conformance code in production crates | `d2b-provider-testkit` |
| Loopback relay implementation used only by tests | `d2b-provider-testkit` |
| Provider implementations in `d2b-realm-transport` | Move to the matching `d2b-provider-relay-<implementation>` crate; protocol-neutral session DTOs remain in realm-core. |
| `d2b-gateway` | Move generic authorization, ledger, and session state into the realm controller/router crates; move provider-specific behavior into typed provider implementations; delete the gateway-named crate. |
| `d2b-gateway-runtime` | Delete after the realm controller composes typed provider registries directly. |

Protocol-neutral realm routing and stream DTOs stay in realm-core crates.
Provider interfaces and descriptors move to `d2b-provider-api`. Concrete
provider dependencies stay in their type-first implementation crates. A
provider implementation depends inward on the API and realm DTO crates; the API
and realm crates never depend outward on implementations.

The rename is one coordinated workspace cutover. No compatibility wrapper crates
or re-export-only packages preserve the old names. Cargo manifests, lockfiles,
Nix package construction, source policy, docs, tests, and dependency-direction
gates move together.

ADR 0044's no-isolation warning remains mandatory, but this ADR separates that
warning from the runtime-provider identifier:

- `systemd-user` identifies direct host-user execution in verified transient
  user scopes;
- `unsafe-local` remains the closed isolation posture and required user-facing
  warning;
- `systemd-user-sandbox`, `bubblewrap`, and `minijail` are distinct provider
  identifiers whose actual posture is derived from the selected profile;
- no provider name alone is sufficient evidence for an `isolated` posture.

The current `providerKind = "unsafe-local"` wire and bundle value remains code
canon until a coordinated schema and bundle-version cutover implements this
split. That cutover must not add an implicit alias or fallback.

### Canonical provider identifier catalogue

The provider registry remains extensible. The following identifiers are
reserved as canonical spellings when those adapters are implemented. Listing an
identifier here does not claim current support.

| Family | Canonical provider identifiers |
| --- | --- |
| Host process | `systemd-user`, `systemd-user-sandbox`, `bubblewrap`, `minijail` |
| Local container | `podman`, `docker`, `systemd-nspawn`, `lxc`, `kata-containers`, `gvisor` |
| Hypervisor or VMM | `cloud-hypervisor`, `qemu-kvm`, `qemu-media`, `firecracker`, `crosvm`, `libkrun`, `xen`, `bhyve`, `hyper-v`, `vmware-vsphere`, `virtualbox`, `apple-virtualization`, `nutanix-ahv` |
| Virtualization control plane | `libvirt`, `proxmox`, `kubevirt` |
| Cloud VM | `aws-ec2`, `azure-vm`, `gcp-compute-engine`, `openstack-nova`, `oracle-compute`, `alibaba-ecs`, `ibm-vpc`, `digitalocean-droplet`, `hetzner-cloud`, `akamai-linode`, `vultr`, `scaleway-instance` |
| Managed cloud sandbox | `aws-fargate`, `azure-container-apps`, `azure-container-apps-sessions`, `gcp-cloud-run`, `fly-machines`, `e2b`, `modal`, `daytona`, `codesandbox`, `github-codespaces` |
| Generic scheduler | `kubernetes-pod`, `nomad-allocation` |

Adapters that do not support d2b's semantic operation or stream contracts
advertise only the capabilities they actually implement. Brand or product
recognition never implies persistent shell, display, device, networking, or
full-controller support.

### A gateway VM is a workload with a realm-controller role

A local gateway is not a separate workload kind. It is a generic workload whose
parent-owned declaration carries a typed controller role:

```nix
d2b.realms.local-root.workloads.work-controller = {
  kind = "local-vm";

  roles.realmController = {
    enable = true;
    forRealm = "work";
  };

  localVm = {
    autostart = true;
    tpm.enable = true;
    graphics.enable = true;
  };

  usb.securityKey.enable = true;
};

d2b.realms.work = {
  parent = "local-root";
};
```

The option spelling above is the selected public shape. Implementation may add
typed sub-options, but it must not replace the role with an arbitrary command,
free-form service definition, or provider-specific controller option.

The declaration has these invariants:

1. The controller workload is owned by the direct parent realm of
   `forRealm`.
2. A workload cannot control its owning realm, an ancestor, a sibling, or an
   unrelated realm.
3. Exactly one active workload/controller generation controls a realm.
4. One controller workload controls one realm by default. Multi-realm
   credential collapse requires a future explicit decision and is not enabled
   by a list-valued shortcut.
5. The workload runtime provider must advertise full realm-controller support.
   Direct host-user providers with `isolation = "unsafe-local"` cannot carry
   credential-bearing realm-controller authority.
6. The target realm's controller placement is derived from the workload's
   provider and location. `gateway-vm` remains a placement class for status and
   telemetry; it is not a separate lifecycle object.
7. The controller never manages the substrate on which it runs. Its parent
   realm and the parent's infrastructure provider retain create, adopt,
   power, replace, and delete authority for that workload.

The parent materializes the role by:

- installing the realm-scoped controller implementation in the guest;
- injecting the parent realm public trust anchor and one-time enrollment
  material;
- providing non-secret provider and relay configuration references;
- preparing the child realm's bootstrap network attachment;
- starting the controller workload before publishing the child route;
- authenticating the controller generation over the standard realm protocol;
- draining the child realm and withdrawing its route before stopping or
  replacing the controller workload.

The child realm identity private key is generated inside the controller
workload. It is never rendered into Nix, the host bundle, cloud-init plaintext,
or a parent-readable state ledger. Persistent controller identity, provider
state, token caches, and realm audit remain inside the controller workload's
storage and TPM boundary.

### Remote controller workloads use the same role

A controller workload may be remote from its parent. Its runtime and
infrastructure provider change; its role and realm protocol do not:

```nix
d2b.realms.local-root.infrastructureProviders.azure = {
  kind = "azure-vm";
  executor.workload = "work-connector.local-root.d2b";
  credentialRef = "entra-azure-control";
};

d2b.realms.local-root.workloads.work-controller = {
  kind = "azure-vm";
  provider = "azure";

  roles.realmController = {
    enable = true;
    forRealm = "work";
  };
};
```

The exact infrastructure-provider option is new schema introduced by this
decision. Existing inert `d2b.realms.<realm>.providers` records must be split
or migrated into typed runtime and infrastructure provider bindings when this
decision is implemented.

The parent-owned infrastructure provider:

1. authorizes the create operation under parent policy;
2. acquires provider credentials only in its configured executor workload;
3. creates or adopts the remote controller VM;
4. injects the parent public key and one-time enrollment material;
5. records only opaque provider resource references and bounded lifecycle
   state;
6. verifies that the enrolling controller matches the expected operation and
   resource binding;
7. retains lifecycle authority after enrollment.

The executor workload must itself be startable by the parent without the child
controller. An ordinary workload already owned by `work` cannot provision the
`work` controller because that would recreate the bootstrap cycle. If an
existing interactive VM such as `work-aad` is selected as the executor, its
controller-facing role and lifecycle must be parent-owned; otherwise a
dedicated `work-connector` workload is required.

The remote VM generates its realm identity, starts the full controller, and
enrolls with the parent. A cloud managed identity may authenticate that VM to
provider APIs or Relay, but managed identity evidence is bootstrap or transport
evidence only. The d2b realm key remains the peer identity used for operations
and policy.

### Provider-agent and relay-connector placement is derived

Three responsibilities may be co-located but are not the same role:

| Responsibility | Authority |
| --- | --- |
| Realm controller | Realm policy, registry, audit, routing, and semantic operations. |
| Infrastructure provider executor | Provider API calls and lifecycle of provider-hosted workloads. |
| Relay connector | Relay authentication and outbound transport session. |

Only `roles.realmController` is asserted directly by a generic workload.
Infrastructure-provider and relay-connector behavior is derived from the
provider or relay binding that references the executor workload. This avoids
two independently configurable declarations claiming the same authority.

For a local controller, all three responsibilities may live in one dedicated,
Entra-managed controller VM. For a remote controller, a local Entra-managed VM
may remain the provider executor and Relay connector while the remote VM owns
the realm controller:

```text
local parent realm
  -> dedicated work connector (local Cloud Hypervisor)
       -> Entra token acquisition
       -> Azure infrastructure-provider executor
       -> Azure Relay connector
            -> remote Azure VM
                 -> work realm controller
```

Combining these functions with an interactive desktop VM is permitted only
when that VM is parent-owned and policy explicitly accepts the availability
and blast-radius tradeoff. A dedicated Entra/Intune-managed connector or
controller workload is preferred.

### Realm relay configuration remains the transport source of truth

Relay placement is configured from `d2b.realms.<realm>.relay`; a separate
gateway or realm-entrypoint object is not introduced.

The relay schema is extended conceptually as follows:

```nix
d2b.realms.work.relay = {
  enable = true;
  provider = "azure-relay";
  fabricRef = "work-relay";

  connector = {
    workload = "work-connector.local-root.d2b";
    credentialRef = "entra-work-relay";
    authentication = "entra-user";
  };

  controllerAuthentication = "managed-identity";
  descendantAccess = "delegated";
  peerShortcuts.enable = true;
};

d2b.realms.payments.relay.inheritFrom = "work";
```

`fabricRef`, endpoint references, and credential references are opaque,
non-secret identifiers. The connector resolves its credential reference inside
the selected workload. The host and parent bundle never resolve or copy the
credential.

Nested realms may inherit the same relay provider and fabric from an ancestor,
but they receive distinct peer identities and scoped transport credentials.
Inheritance does not share controller token caches, realm private keys, or a
single authorization identity.

If both peers have direct authenticated connectivity, they may use a direct
transport without a connector workload. If Relay authentication or work
credentials are required, the configured connector owns that side of the
transport. There is no root, `sudo`, host-token, or direct-network fallback.

### Shared-relay peer shortcuts separate control and data paths

Strict tree routing remains the authorization model. Shared-relay shortcuts
optimize only the data path:

```text
control:
  source -> source controller -> applicable ancestors -> target controller

data after authorization:
  source peer -> shared relay fabric -> target peer
```

Every applicable source, ancestor, and target policy is evaluated before a
shortcut is issued. The nearest common ancestor issues a short-lived signed
`PeerShortcutGrant` only after the normal tree route is authorized.

The grant is scoped to:

- source realm and authenticated source principal;
- target realm and authenticated target principal;
- operation or stream kind;
- required capability;
- digest of the authorized tree path;
- controller generations and policy epochs used for the decision;
- bounded correlation and shortcut identifiers;
- issue and expiry times;
- a replay nonce.

The grant contains no token cache, Relay credential, raw endpoint, provider
resource id, command payload, stream data, or user-supplied label.

Source and target bind the grant digest into their end-to-end peer-session
handshake. The relay adapter supplies an opaque, one-time rendezvous binding
below the policy DTO. Session keys and d2b identities authenticate and encrypt
the stream end to end; the relay authenticates transport access only.

An established shortcut:

- does not create a new realm edge, alternate parent, or DAG route;
- is valid only for the authorized operation or stream;
- cannot be reused for generic IP forwarding, port forwarding, VPN traffic, or
  an unrelated d2b operation;
- expires with the grant, route advertisement, controller generation, or
  policy epoch, whichever expires first;
- is torn down on policy revocation, route revocation, peer disconnect,
  transport failure, or operation completion;
- produces bounded establishment and teardown audit records at the authorizing
  ancestor and participating peers even though stream bytes bypass
  intermediate controllers.

### Existing direct-shortcut contracts are generalized

The route engine already contains `DirectShortcutAuthorizationMetadata`,
`DirectShortcutState`, and typed teardown reasons. Implementation of this ADR
generalizes that foundation to peer transport shortcuts:

```text
PeerShortcutTransport
  = native-direct
  | shared-relay
```

Authorization metadata remains transport-address-free. Provider-specific
rendezvous data belongs in a separate bounded transport binding keyed by the
shortcut id.

This decision refines ADR 0043's restriction that direct shortcuts use only
native underlay reachability. Native direct transport still must not add
STUN/ICE, NAT traversal, a VPN, or an overlay. A `shared-relay` shortcut is
allowed only when both peers already participate in the same configured relay
fabric and the relay provider advertises shortcut support. It does not discover
or construct a new network path.

If a shortcut cannot be established, the operation follows an explicitly
authorized parent relay path or fails with a typed transport error. There is no
silent direct-network, provider-native, SSH, or generic tunnel fallback.

### Workloads may participate directly in a shared relay

Eliminating controller hops requires the source and target workload agents, not
only their controllers, to participate in the relay fabric.

A workload may advertise `relay-client-v1` only when its runtime supplies a
guestd-compatible or d2b peer agent. The controller delegates a short-lived,
workload-scoped relay access grant or the workload authenticates independently
through a provider-supported identity such as Azure Managed Identity.

The workload never receives:

- the controller's Relay credential;
- an Entra refresh token from another VM;
- a realm identity private key;
- authority to advertise descendants;
- a grant broader than its workload identity and negotiated operations.

If the relay provider cannot issue or validate a workload-scoped transport
identity, that workload cannot use a direct shared-relay shortcut. Its traffic
continues through the authorized controller/connector path.

### Entra, YubiKey, browser, and developer authentication

The physical YubiKey is shared at the CTAP ceremony layer, not at the token
cache layer.

D2b's security-key proxy may expose persistent virtual FIDO devices to the
controller or connector VM, a work browser VM, and a work development VM. The
host broker serializes ceremonies so only one transaction uses the physical key
at a time. Each VM performs its own Entra authentication and stores its own
tokens.

The expected credential split is:

| Workload | Credential use |
| --- | --- |
| Controller or provider-executor VM | ARM and Relay tokens obtained inside that VM. |
| Work browser VM | Browser session tokens obtained by its own WebAuthn ceremony. |
| Local work development VM | Its own delegated `az`/SDK login when direct Azure data-plane access is required. |
| Remote Azure VM | Azure Managed Identity for Key Vault, storage, database, and other Azure resource access where supported. |

Infrastructure control from a development workload should use typed d2b
operations routed to the infrastructure-provider executor. The executor's ARM
token is never returned to that workload. Code running in Azure should prefer
Managed Identity. Code requiring delegated user access authenticates in its own
workload and does not mount or import another VM's token cache.

When controller or connector authentication requires user interaction, provider
status reports `interaction-required` and d2b may open a provider-owned
authentication window through the Wayland proxy. It must not expose a direct
host compositor fallback.

The CTAP proxy covers FIDO/WebAuthn only. PIV, CCID, OTP, and OpenPGP interfaces
still require exclusive USB ownership. A single physical key cannot
simultaneously use CTAP proxying and USBIP for those interfaces; use explicit
handoff or a second key.

## Authorization and security invariants

Implementation must preserve all of the following:

1. A controller role is parent-authorized configuration, not a capability a
   guest can self-assert.
2. Controller, provider-executor, and relay-connector peer identities are
   authenticated before any state lookup, token resolution, provisioning, or
   route publication.
3. Provider and Relay tokens stay in the configured credential-owning workload.
4. Entra, managed identity, and Relay identities are never mapped to local
   daemon roles or broker authorization.
5. The remote controller re-originates privileged local effects through its own
   broker; no remote peer receives the broker wire protocol.
6. A parent may stop or replace the controller workload but cannot use its
   storage ledger as authority to repair child realm state.
7. One shared relay fabric does not merge realm policy, identity, audit, or
   credential domains.
8. Shortcut authorization follows the same parent/child policy chain as the
   non-shortcut route.
9. Raw Relay endpoints, provider resource ids, token subjects, user ids, device
   ids, command data, and stream payloads are forbidden as metric labels.
10. Ambiguous controller identity, route generation, shortcut state, or
    infrastructure ownership is preserved and reported degraded rather than
    guessed, killed by PID, or broadly cleaned up.

## Failure and continuation behavior

- If an infrastructure-provider executor is unavailable, existing remote
  controllers continue running, but create, power, replace, and delete
  operations fail visibly.
- If a local Relay connector is unavailable, the remote realm may continue
  operating, but local reachability through that connector is unavailable.
- If the remote controller is unavailable, the realm is unavailable even when
  its infrastructure and Relay endpoint still exist.
- If Relay is unavailable, no provider-native, SSH, or direct-network fallback
  is attempted unless a separately configured and authorized transport exists.
- Daemon, connector, and controller restarts are continuation events.
  Reconnection uses realm identity, controller generation, operation ids, route
  generations, and shortcut ids rather than persisted sockets or pidfds.
- Expired or superseded shortcut grants are not adopted.
- Losing or replacing persistent controller identity is an explicit
  re-enrollment event, not an automatic repair.

## Public and private contract changes

Implementation requires coordinated changes across:

- the `d2b-provider-api` base and specialized provider interfaces;
- type-first provider implementation crates and typed registries;
- `d2b-provider-testkit` conformance suites and provider naming policy;
- `d2b.realms.<realm>.workloads.<workload>.roles.realmController`;
- typed runtime and infrastructure provider bindings replacing the ambiguous
  inert provider record;
- `d2b.realms.<realm>.relay` provider, fabric inheritance, connector, and
  shortcut policy;
- workload, controller, provider-executor, and relay-client capability
  advertisements;
- controller-workload bootstrap and generation DTOs;
- peer shortcut authorization and transport-binding DTOs;
- realm-controller, workload, and relay status output;
- bundle artifacts, generated schemas, reference documentation, and migration
  guidance.

Security-sensitive schema changes require the normal bundle/schema version
bumps. Existing `unsafe-local` provider-kind values and old inert provider
records receive explicit migration errors after the selected cutover; no
success-shaped compatibility fallback is added.

## Validation requirements

Implementation is incomplete without:

- source-policy tests proving provider crate names use a recognized type-first
  axis and the implementation id matches the descriptor;
- dependency-direction tests proving `d2b-provider-api` does not depend on an
  implementation, cloud SDK, daemon, broker, codec, or concrete transport;
- conformance tests for every registered provider's primary interface and
  advertised optional capabilities;
- Nix evaluation tests for one-controller-per-realm, direct-parent ownership,
  cycle rejection, provider capability gating, and derived placement;
- tests proving a controller cannot control its own substrate;
- bootstrap tests proving only public trust anchors and one-time enrollment
  material cross the parent/provider boundary;
- provider-executor tests proving credentials are resolved only inside the
  selected workload;
- Relay inheritance tests proving nested realms receive distinct identities and
  no copied credential references;
- route-engine tests for shared-relay shortcut authorization, replay,
  expiration, policy epoch changes, route revocation, and teardown;
- end-to-end tests proving shortcut bytes bypass intermediate controllers while
  every policy boundary records the decision;
- negative tests proving a relay-authenticated peer is not local `Admin`;
- YubiKey contention tests proving controller, browser, and developer
  ceremonies serialize without token-cache sharing;
- restart/adoption tests for local and remote controller workloads;
- redaction tests covering provider ids, endpoints, Entra identities, Azure
  resource ids, and shortcut metadata.

## Consequences

### Positive

- Gateway VMs become ordinary workloads with a precise role.
- Provider crates sort by authority type and expose one recognizable interface
  and conformance contract.
- Local and remote realm controllers use one configuration and protocol model.
- Controller hosting lifecycle cannot become self-referential.
- Provider, Relay, and controller responsibilities have explicit credential and
  authority boundaries.
- Nested realms can share one relay fabric without forcing stream bytes through
  every controller.
- The existing route-shortcut policy engine remains useful and gains a
  provider-neutral relay transport binding.
- One physical FIDO key can serve multiple work VMs while each retains an
  independent Entra session.

### Negative

- Provider configuration becomes more explicit and requires migration from the
  current inert `providers` records.
- The workspace-wide provider rename is intentionally disruptive and must update
  every manifest, Nix build, policy gate, and documentation reference together.
- Remote controllers need a parent-owned provider executor even after
  enrollment.
- Direct workload shortcuts require relay-capable guest agents and scoped
  transport identities.
- Shared-relay revocation and audit are more complex than hop-by-hop byte
  forwarding.
- A controller requiring interactive Entra renewal may temporarily block
  provider operations even while the realm itself remains reachable.

## Alternatives considered

### Keep provider crates named by vendor or product only

Rejected. Names such as `d2b-provider-aca`, `d2b-provider-relay`, and
`d2b-provider-azure` do not reveal whether the crate executes workloads,
provisions infrastructure, owns credentials, or transports bytes. Type-first
names make authority visible in workspace listings and dependency review.

### Use one universal provider trait with optional methods

Rejected. A catch-all interface would allow unsupported methods to accumulate,
blur authority boundaries, and make capability claims difficult to verify. A
small common `Provider` base plus mandatory primary-type interfaces preserves
shared status/error semantics without pretending every provider can perform
every operation.

### Keep gateway VMs as a separate object

Rejected. It duplicates workload lifecycle, runtime-provider, component,
network, and status configuration. The special behavior is the controller role,
not the VM kind.

### Declare the controller workload inside the realm it controls

Rejected. The realm controller would be required to start the workload that
must start the controller. Parent ownership provides an acyclic bootstrap and a
clear infrastructure repair owner.

### Let a controller manage its own cloud VM

Rejected. Loss or corruption of that controller would also remove the only
authority able to inspect or replace its substrate. Parent-owned infrastructure
providers preserve recovery authority.

### Store Entra, provider, or Relay credentials on the physical host

Rejected. This violates the realm credential boundary and ADR 0043's rule that
the host control plane does not hold work/provider credentials.

### Forward one VM's Entra token cache to other work VMs

Rejected. Token forwarding collapses execution identities and turns the
controller into a credential vending service. Each workload authenticates
independently or uses a provider-managed workload identity.

### Send every nested stream through every realm controller

Rejected as the only data path. Controllers remain policy decision points, but
forcing them into the byte path adds latency, bandwidth cost, and cascading
failure without strengthening an already-authorized end-to-end stream.

### Treat shared Relay membership as authorization

Rejected. Relay credentials establish transport access only. Realm keys,
controller generations, tree policy, operation capabilities, and scoped
shortcut grants remain authoritative.

### Add a generic network tunnel for nested realms

Rejected. D2b routes semantic operations and named streams, not arbitrary
cross-realm IP traffic. Shared-relay shortcuts are operation-scoped and do not
create a VPN or alternate realm topology.
