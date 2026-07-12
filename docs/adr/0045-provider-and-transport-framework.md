# ADR 0045: Provider and transport framework

- Status: Proposed
- Date: 2026-07-10
- Refines: [ADR 0035](0035-efficiency-and-simplification-roadmap.md)
  (provider naming and workspace simplification), [ADR 0043](0043-realm-native-control-plane.md)
  (realm-native control plane), [ADR 0044](0044-unsafe-local-runtime-provider.md)
  (unsafe-local runtime provider)
- Related: [ADR 0010](0010-wire-protocol-and-typed-errors.md)
  (wire protocol and typed errors), [ADR 0028](0028-guest-control-plane-over-vsock.md)
  (guest control plane over virtio-vsock),
  [ADR 0034](0034-storage-lifecycle-restart-and-synchronization.md)
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
  controller host, transport, protocol codec, node client, or daemon-access
  adapter;
- the existing `d2b.realms.<realm>.providers` records do not distinguish those
  responsibilities;
- `d2b-realm-provider` exports provider authorities beside
  `TransportProvider`, `ProtocolCodec`, `StreamMux`, daemon-access, and node
  client traits even though those seams have different ownership;
- Azure Relay already implements the generic `TransportProvider`, while the
  narrower `RelayProvider` has no production implementation;
- realm relay configuration says how a realm is reachable, but not how Unix
  streams, vsock, direct TLS/QUIC, and Relay share one transport contract;
- public daemon, realm-peer, and guest-control protocols independently
  implement framing, version negotiation, authentication, capability exchange,
  typed errors, deadlines, and request correlation;
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
| Transport provider | Produces connected bidirectional byte sessions over Unix streams, vsock, direct network transports, or relay fabrics. |
| Component session | Adds fixed framing, protocol negotiation, authentication, encryption, peer identity, limits, and liveness above a byte transport. |
| Service protocol | Defines typed daemon, realm, guest, or provider APIs above an authenticated component session. |
| Workload role | Declares an authority-bearing function performed by a workload, such as running a realm controller. |

The unqualified term `realm provider` is too ambiguous for a public schema and
must not name a new catch-all interface. An adapter may implement more than one
provider trait, but each binding names the trait being used. For example:

- `azure-vm` can provision a remote VM as an infrastructure provider and can
  supervise that VM as a workload runtime provider;
- `azure-relay` is a transport-provider implementation;
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
| `d2b-contracts` | Contract crate | Canonical serialized provider descriptors, operation contexts, capability DTOs, plans, observed-state envelopes, stable errors, and generated schemas. |
| `d2b-provider` | Interface crate | In-process async Rust provider traits, typed registries, and runtime error wrappers over `d2b-contracts` types. Contains no duplicate contract DTO, provider SDK, or implementation. |
| `d2b-provider-runtime-<implementation>` | `RuntimeProvider` | Plans, starts, stops, adopts, and inspects workloads. |
| `d2b-provider-infrastructure-<implementation>` | `InfrastructureProvider` | Provisions, adopts, inspects, and deletes infrastructure that hosts workloads or realm controllers. |
| `d2b-provider-transport-<implementation>` | `TransportProvider` | Connects or accepts bounded bidirectional byte sessions and advertises purpose, rendezvous, reconnect, and revocation capabilities. |
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
- `d2b-provider-transport-azure-relay`;
- `d2b-provider-transport-unix-stream`;
- `d2b-provider-transport-cloud-hypervisor-vsock`;
- `d2b-provider-credential-entra`;
- `d2b-provider-substrate-nixos`;
- `d2b-provider-display-wayland`.

For example, `d2b-provider-transport-azure-relay` has
`ProviderType::Transport` and implementation id `azure-relay`; its public
provider kind remains `azure-relay`. A configured deployment may then assign
instance ids such as `work-transport` or `payments-transport` without
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
relay, Unix-stream, and vsock transports sort under the common transport axis.

### Every provider implements a standard base interface

Existing `d2b-contracts` remains the canonical owner of provider data that is
serialized, persisted, generated from Nix, sent over a wire, or shared by
independently compiled components. Provider contracts live under a focused
module such as `d2b_contracts::provider`.

That module owns at least:

- `ProviderDescriptor`, `ProviderType`, and `ProviderHealth`;
- the serializable `ProviderOperationContext`;
- primary and optional capability descriptors;
- provider plans, opaque handles, and observed-state envelopes that cross a
  crate, process, persistence, or wire boundary;
- stable provider error kinds, retry hints, and redacted error envelopes;
- schema versions and generated JSON/protobuf schemas where applicable.

Provider implementations must import these canonical types. They must not
declare provider-local copies with the same semantic fields.

The interface crate is named `d2b-provider`, not `d2b-provider-api`. This
matches the repository convention: `d2b-contracts` owns serialized contracts,
`*-core` crates own pure models, and a bare domain crate names the trait and
composition boundary. The project does not otherwise use an `-api` crate suffix
for this kind of in-process interface.

`d2b-provider` replaces only the trait, registry, and in-process runtime
portion of `d2b-realm-provider`. It depends inward on `d2b-contracts` plus the
minimum async/I/O traits needed by the interfaces. It may define non-serialized
trait-object adapters, `ProviderResult`, and a runtime `ProviderError` wrapper,
but that wrapper exposes the stable error envelope from `d2b-contracts`.

`d2b-provider` must not depend on:

- `d2bd`, the privileged broker, or host mutation implementations;
- a cloud SDK, HTTP client, TLS implementation, or concrete transport;
- a protocol codec;
- a provider implementation crate;
- test mocks or live-provider fixtures.

`d2b-contracts` must not depend on `d2b-provider` or any provider
implementation. If the current monolithic contract crate would otherwise force
unrelated guest protobuf or codec dependencies into every provider build, those
dependencies must be feature-gated or split behind contract-only modules rather
than copying provider DTOs into the interface crate.

Every registered provider implements the common base interface:

```rust
#[async_trait]
pub trait Provider: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    async fn health(&self) -> ProviderResult<ProviderHealth>;
}
```

The canonical `d2b_contracts::provider::ProviderDescriptor` is bounded,
non-secret data containing:

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
    Transport,
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

Optional capability dispatch uses parallel capability-specific registries keyed
by the same `ProviderId`. Rust trait-object downcasting or peer-trait casting is
not part of the design. Registry construction verifies both directions:

- every advertised optional capability has an implementation registered in the
  matching capability registry;
- every registered capability implementation is advertised by the provider
  descriptor.

A mismatch fails provider registration before the provider can receive an
operation.

The specialized interfaces extend `Provider`:

| Interface | Required semantic surface |
| --- | --- |
| `RuntimeProvider` | Capability description; plan; idempotent ensure/start; stop; inspect/adopt; destroy when the runtime owns durable workload state. |
| `InfrastructureProvider` | Capability description; plan; apply; adopt; inspect; bootstrap binding; destroy. |
| `TransportProvider` | Capability description; connect/listen; return a bounded byte session; issue and revoke a binding when supported; inspect transport health. |
| `SubstrateProvider` | Capability description; check; plan remediation; apply only through the authorized substrate owner. |
| `CredentialProvider` | Non-secret status; interaction requirement; acquire or refresh only for a co-located typed consumer; revoke. |
| `DisplayProvider` | Capability description; open and close an already-authorized display session. |

`TransportProvider` is the common byte-carriage authority. Its capability
descriptor states:

- accepted purposes (`realm-peer`, `workload-peer`, `daemon-access`,
  `guest-control`, or `bootstrap`);
- whether it can connect, listen, or provide rendezvous;
- reconnect and liveness behavior;
- whether established sessions support active revocation;
- maximum session lifetime;
- whether kernel peer evidence is available.

Unix-stream, native-vsock, Cloud-Hypervisor-vsock, direct-TLS/QUIC, loopback,
and Azure Relay implementations all return the same `TransportSession`. Relay
authentication and rendezvous remain Azure-Relay implementation details; they
never become d2b principal authentication.

The existing local-only `RuntimeProvider` and provider-managed
`WorkloadProvider` split is retired. Local VMMs, host-user runtimes, container
sandboxes, provider-managed sandboxes, and remote VM runtimes implement one
`RuntimeProvider` lifecycle contract. Exec, persistent shell, display, console,
audio, and guest-control remain optional capability interfaces rather than
being folded into runtime lifecycle.

### Provider and non-provider seams are explicit

The current `d2b-realm-provider` surface mixes provider authorities with
protocol machinery. The cutover classifies them as follows:

| Current interface | Selected treatment |
| --- | --- |
| `HostSubstrateProvider` | Rename to primary `SubstrateProvider`. |
| `RuntimeProvider` | Primary runtime provider. |
| `WorkloadProvider` | Fold into `RuntimeProvider`; exec becomes an optional capability. |
| `InfrastructureProvider` | Primary infrastructure provider. |
| `CredentialProvider` | Primary credential provider. |
| `DisplayProvider` | Primary display provider. |
| `TransportProvider` / `TransportListener` | Primary transport provider plus supporting listener type. |
| `RelayProvider` | Remove; relay rendezvous is a transport capability. |
| `DurableExecutionProvider` | Optional runtime capability. |
| `PersistentShellProvider` | Optional runtime capability. |
| `GuestControlEndpointProvider` | Rename to optional `GuestControlEndpointResolver`; it discovers an endpoint and is not a primary provider. |
| `ObservabilitySinkProvider` | Rename to `ObservabilitySink` until an independently configured external sink justifies a primary provider type. |
| `NodeProvider` | Rename to `NodeClient` or `NodeInventory`; node registration and workload listing are realm services, not provider authority. |
| `ProtocolCodec` | Move to the component-session/codec layer. |
| `StreamMux` | Move to the component-session/realm-router layer. |
| `DaemonAccessTransport` | Replace with daemon-access composition over `TransportProvider`. |
| `DaemonAccessApi` | Keep as a semantic daemon service contract. |

Internal dependency-injection seams such as `NodeRunner`, `ShellBackend`,
`TerminalBackend`, `HostAudioController`, `ExecRuntime`, `LogStore`,
`TokenSource`, `CapabilitiesProvider`, `ShellEventSink`, `CgroupBackend`,
`NetlinkBackend`, and `ModprobeBackend` do not implement `Provider` and do not
appear in provider registries. They are local strategies owned by a daemon,
guest, broker, or test harness.

Serialized runtime locality and driver values are descriptor dimensions, not
provider types. `local`, `cloud-hypervisor`, `crosvm`, and `qemu` become
`RuntimeLocality` and `RuntimeImplementationKind` fields rather than a second
Rust type also named `RuntimeProvider`.

`d2b-contracts` defines a bounded, serializable `ProviderOperationContext`
containing:

- the stable operation id and idempotency key;
- the already-authorized realm and workload/controller identity;
- the required capability;
- the wall-clock expiry used across process or wire boundaries;
- an opaque, non-secret trace id suitable for W3C-compatible correlation.

`d2b-provider` defines a non-serializable `ProviderCallContext` that wraps the
contract DTO with the local monotonic deadline and cancellation signal. Tokio
tokens, channels, `Instant`, file descriptors, and other runtime state never
enter `d2b-contracts`.

The provider does not reinterpret local users, authorize a principal, or choose
another realm. The realm controller performs authorization before dispatch.
The provider verifies that the call context matches its configured scope,
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
  capability from `ProviderCallContext`;
- optional capability registry entries and descriptor claims match exactly.

Cloud implementation crates keep live tests explicitly opt-in. Hermetic
conformance uses fake SDK clients and transports supplied by the implementation
crate, while shared mocks and assertions remain in `d2b-provider-testkit`.

### Transport is a byte-stream boundary only

`TransportProvider` returns a connected, reliable, ordered, bidirectional byte
session. It does not:

- authenticate a d2b principal;
- negotiate a d2b service or schema;
- authorize an operation or stream;
- interpret API payloads;
- map Relay, TLS, vsock, or Unix identity to a daemon role;
- bridge one transport to another.

`TransportTarget` becomes a typed provider reference, opaque endpoint reference,
and closed purpose rather than an unbounded endpoint string. Accepted sessions
carry:

- transport provider id and implementation kind;
- opaque binding/session id;
- closed transport purpose;
- bounded local transport evidence when available;
- active-revocation handle when advertised;
- the connected byte stream.

Transport evidence is not authorization. Unix `SO_PEERCRED`, a VMM process uid,
vsock CID, TLS certificate, managed identity, Relay SAS, and rendezvous id have
different meanings and are consumed only by the authenticator selected for the
session purpose.

The generic transport boundary includes:

- Unix streams used for direct daemon or realm-peer connections;
- native AF_VSOCK;
- Cloud Hypervisor's host-to-guest vsock adapter after its `CONNECT`/`OK`
  handshake;
- loopback/local TCP conformance transports;
- direct TLS/QUIC/WebSocket transports;
- Azure Relay WebSocket rendezvous.

The public daemon Unix listener, privileged broker socket, and unsafe-local
helper remain specialized IPC endpoints where seqpacket boundaries,
`SO_PEERCRED`, socket activation, `SCM_RIGHTS`, exact FD validation, or helper
generation semantics are load-bearing. Their local framing helpers may be
shared, but they do not implement `TransportProvider`.

### One component-session protocol sits above transport

D2b will add a standard component-session layer:

```text
TransportSession
  -> ComponentSession
       fixed preface and framing
       protocol/service negotiation
       Noise or local mechanism authentication
       encryption and replay protection
       peer identity and channel binding
       capabilities and effective limits
       keepalive, deadline, and close semantics
    -> typed service protocol
```

Serialized session contracts live in `d2b-contracts::session`. The in-process
state machine, authenticator traits, framing, and runtime contexts live in a
bare `d2b-session` crate. `d2b-session` depends inward on contracts and crypto
primitives; semantic daemon, realm, guest, and provider services depend on it.

The bootstrap preface and handshake use one fixed canonical encoding. A peer
does not encode the handshake through the codec it is attempting to negotiate.
After authentication, the selected service may use protobuf or another
explicitly negotiated codec.

Noise handshake and transport records use fixed 16-bit length framing and stay
within Noise's 65,535-byte message limit. ComponentSession fragments larger
ttrpc or named-stream frames into bounded encrypted records and reassembles
them only up to the negotiated d2b hard frame limit. Record sequence, fragment
count, and total plaintext length are authenticated; truncation, duplication,
reordering, or over-limit reassembly closes the session.

The handshake exchanges and binds:

- component-session protocol version;
- session purpose and endpoint roles;
- supported and selected service protocols;
- supported and selected authentication mechanism;
- codec id and schema fingerprint;
- hard frame and stream limits;
- positive capabilities;
- both nonces;
- transport channel-binding evidence;
- expected realm, node, workload, or daemon identity where applicable.

All offered and selected values are covered by the authentication transcript so
an intermediary cannot downgrade authentication, codec, schema, limits, or
capabilities. No semantic API request, event, or stream is exposed before the
session reaches `Accepted`.

### Noise is the standard non-local peer authenticator

Non-local realm, controller, node, workload-agent, and provider-agent sessions
use the [Noise Protocol Framework](https://noiseprotocol.org/) above the
transport byte stream.

The initial mechanism set is:

| Mechanism | Purpose |
| --- | --- |
| `local-peercred` | Direct local Unix daemon access; kernel credentials are mapped by local daemon policy. |
| `noise-realm` | Enrolled realm/controller/node/workload peers using realm-bound static keys and ephemeral session keys. |
| `noise-guest-psk` | Guest-control sessions using the existing per-VM secret plus boot/CID/direction/purpose transcript claims. |
| `noise-bootstrap` | One-time controller or child-realm enrollment bound to parent trust, operation id, expected resource, expiry, and replay nonce. |
| `mutual-tls` | Optional direct daemon-access interoperability when a configured certificate authority owns the identity mapping. |

The initial Noise profiles are:

```text
noise-realm     = Noise_KK_25519_ChaChaPoly_SHA256
noise-bootstrap = Noise_IK_25519_ChaChaPoly_SHA256
noise-guest-psk = Noise_NNpsk0_25519_ChaChaPoly_SHA256
```

Changing a pattern or primitive suite is a component-session protocol-version
change, not an implementation-local preference.

Enrolled realm peers know each other's bound static transport keys and use
`KK`. A bootstrapping child knows the parent's seed-provided static key and uses
`IK`; the parent binds the newly presented child static key only after the
operation/resource/enrollment checks succeed.

Production endpoints never accept `none` or silently retry a weaker mechanism.
The endpoint purpose fixes the minimum acceptable mechanism. Relay, managed
identity, or TLS transport authentication may be included as channel-binding
evidence, but never substitutes for the selected d2b peer authenticator.

Noise static keys are not themselves realm identities. Enrollment binds each
Noise public key to the canonical realm/node/workload identity and controller
generation. The Noise handshake hash becomes the component-session channel
binding used by service authorization and audit.

### ttrpc/protobuf is the common control-RPC protocol

Authenticated component sessions use
[ttrpc](https://github.com/containerd/ttrpc/blob/main/PROTOCOL.md) framing and
protobuf service definitions for bounded control RPCs. This reuses the existing
guest-control dependency and supplies:

- service and method dispatch;
- request/response correlation;
- unary and streaming frame forms;
- protobuf schemas and generated Rust bindings;
- explicit stream closure.

The d2b hard frame cap remains 1 MiB even though ttrpc permits larger frames.
Declared lengths above the negotiated d2b cap are rejected before allocation.

Ttrpc is not the transport or security layer. Its specification intentionally
omits authentication, unreliable-network recovery, ping/reset behavior, and
flow control. ComponentSession therefore owns authentication, keepalive,
connection generation, reconnect, hard limits, and session teardown.

The initial services are:

```text
d2b.daemon.v1
d2b.realm.v1
d2b.guest.v1
d2b.provider.v1
```

Every request envelope carries a bounded request id, correlation/trace context,
service and method id, absolute deadline, and idempotency key when mutating.
The authenticated principal is session state, not a caller-controlled request
field. Required capability is derived from trusted service/method metadata.
Responses use the shared typed error envelope.

High-volume or reconnectable PTY, display, clipboard, file-copy, logs, audio,
and port-forward data continues to use d2b's named stream mux. The mux binds
each stream to an already-authorized operation, enforces credit/backpressure,
and supplies resume/close semantics that ttrpc intentionally does not provide.
Ttrpc is the control plane for opening and managing those streams, not a second
unbounded data tunnel.

Exactly one ComponentSession driver reads and writes the encrypted transport.
Its post-auth record header distinguishes:

```text
session-control
ttrpc-control
named-stream
```

Ttrpc runs over one bounded virtual control channel and multiplexes RPC calls
inside that channel. The d2b mux owns named data channels. Ttrpc and a named
stream handler never read the underlying transport concurrently, and a blocked
data stream cannot consume the reserved control credit required for
cancellation, revocation, keepalive, or close.

### Existing component protocols migrate behind the session layer

- `PeerSession` version/codec/capability negotiation and
  `SecurePeerSession` authentication/encryption merge into
  `ComponentSession`.
- Guest-control keeps its protobuf service semantics, while its nonce/HMAC
  transcript becomes the `noise-guest-psk` authenticator and its vsock path
  becomes a transport implementation.
- `DaemonAccessTransport` implementations become daemon-access clients composed
  over a selected `TransportProvider` and `ComponentSession`.
- The local public Unix socket keeps direct `SO_PEERCRED` admission and its
  compatibility wire until an explicit migration moves it to
  `d2b.daemon.v1`.
- `ProtocolCodec` moves to the session/codec boundary; semantic services do not
  depend on concrete codec implementations.
- `StreamMux` moves to `d2b-session` or `d2b-realm-router` and remains above
  authenticated sessions.
- Broker and unsafe-local helper protocols retain specialized seqpacket and FD
  semantics. They may reuse shared IDs/errors but are not component-session
  byte transports.

### Existing provider crates migrate explicitly

The implementation cutover uses this map:

| Current crate | Selected replacement |
| --- | --- |
| `d2b-realm-provider` | Split serialized DTOs/capabilities/stable errors into `d2b-contracts::provider`, provider traits/registries into `d2b-provider`, session/codec/mux traits into `d2b-session` or realm-router, and mocks/conformance into `d2b-provider-testkit`. |
| `d2b-host-providers` | Split into `d2b-provider-runtime-cloud-hypervisor`, `d2b-provider-runtime-qemu-media`, `d2b-provider-substrate-nixos`, `d2b-provider-substrate-linux`, and `d2b-provider-display-wayland`. |
| `d2b-provider-aca` | `d2b-provider-runtime-azure-container-apps` |
| `d2b-provider-relay` | `d2b-provider-transport-azure-relay` |
| Provider conformance code in production crates | `d2b-provider-testkit` |
| Loopback transport implementation used only by tests | `d2b-provider-testkit` |
| Implementations in `d2b-realm-transport` | Move to matching `d2b-provider-transport-<implementation>` crates; move common session runtime to `d2b-session`; keep semantic route DTOs in realm-core. |
| `d2b-realm-router::{PeerSession,SecurePeerSession}` | Merge into `d2b-session::ComponentSession`; keep realm route policy and operation-bound mux orchestration in realm-router. |
| `d2b-daemon-access` transport implementations | Compose daemon-access service clients over typed transport providers and component sessions; keep local Unix compatibility admission until migrated. |
| `d2b-gateway` | Move generic authorization, ledger, and session state into the realm controller/router crates; move provider-specific behavior into typed provider implementations; delete the gateway-named crate. |
| `d2b-gateway-runtime` | Delete after the realm controller composes typed provider registries directly. |

Protocol-neutral realm routing and stream DTOs stay in realm-core crates.
Serialized provider DTOs and schemas move to `d2b-contracts`; in-process
interfaces move to `d2b-provider`. Concrete provider dependencies stay in
their type-first implementation crates. A provider implementation depends
inward on the interface and contract crates; contract, interface, and realm
crates never depend outward on implementations.

The rename is one coordinated workspace cutover. No compatibility wrapper crates
or re-export-only packages preserve the old names. Cargo manifests, lockfiles,
Nix package construction, source policy, docs, tests, and dependency-direction
gates move together.

The generic error code `relay-unavailable` becomes `transport-unavailable`.
Azure Relay authentication, rendezvous, or provider-specific failures retain
typed Azure-Relay diagnostic classes beneath that transport-level result.

ADR 0044's no-isolation warning remains mandatory, but this ADR separates that
warning from the runtime-provider identifier:

- `systemd-user` identifies direct host-user execution in verified transient
  user scopes;
- `unsafe-local` remains the closed isolation posture and required user-facing
  warning;
- `systemd-user-service`, `bubblewrap`, and `minijail` are distinct provider
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
| Host process | `systemd-user`, `systemd-user-service`, `bubblewrap`, `minijail` |
| Local container | `podman`, `docker`, `systemd-nspawn`, `lxc`, `kata-containers`, `gvisor` |
| Hypervisor or VMM | `cloud-hypervisor`, `qemu-kvm`, `qemu-media`, `firecracker`, `crosvm`, `libkrun`, `xen`, `bhyve`, `hyper-v`, `vmware-vsphere`, `virtualbox`, `apple-virtualization`, `nutanix-ahv` |
| Virtualization control plane | `libvirt`, `proxmox`, `kubevirt` |
| Cloud VM | `aws-ec2`, `azure-vm`, `gcp-compute-engine`, `openstack-nova`, `oracle-compute`, `alibaba-ecs`, `ibm-vpc`, `digitalocean-droplet`, `hetzner-cloud`, `akamai-linode`, `vultr`, `scaleway-instance` |
| Managed cloud sandbox | `aws-fargate`, `azure-container-apps`, `azure-container-apps-sessions`, `gcp-cloud-run`, `fly-machines`, `e2b`, `modal`, `daytona`, `codesandbox`, `github-codespaces` |
| Generic scheduler | `kubernetes-pod`, `nomad-allocation` |
| Transport | `unix-stream`, `native-vsock`, `cloud-hypervisor-vsock`, `direct-tls`, `quic`, `azure-relay` |

Adapters that do not support d2b's semantic operation or stream contracts
advertise only the capabilities they actually implement. Brand or product
recognition never implies persistent shell, display, device, networking, or
full-controller support.

### Runtime providers declare kernel and adoption posture

Every `RuntimeProvider` descriptor includes closed posture fields for:

- process and restart-adoption authority;
- network namespace ownership;
- user namespace construction;
- persistent identity protection;
- cgroup ownership;
- device mediation.

`systemd-user` and `systemd-user-service` workloads are owned by the
authenticated user's systemd manager. Their processes live under user-manager
scopes, not `/sys/fs/cgroup/d2b.slice`. Restart adoption therefore uses the
verified systemd `InvocationID`, exact scope control-group identity, and the
same-UID helper generation selected by ADR 0044. They are not swept or adopted
through the VM runner cgroup algorithm from ADR 0034. The provider ledger is
diagnostic; a live verified systemd scope remains the adoption authority.

This provider-specific adoption rule is allowed only because the runtime
descriptor names it and conformance verifies it. It does not relax the
`d2b.slice` rule for broker-spawned VM and sidecar runners.

Host-process runtimes also declare one of these network postures:

| Posture | Meaning |
| --- | --- |
| `host-shared` | Shares the host network namespace and is explicitly reported as non-isolated. It cannot satisfy realm network isolation. |
| `none` | Has no network namespace interfaces beyond loopback. |
| `isolated-namespace` | Uses a dedicated network namespace with broker-owned veth/TAP attachment and realm firewall policy. |

Bubblewrap or Minijail naming does not imply `isolated-namespace`; the selected
profile must request and prove it. A runtime that uses `host-shared` networking
cannot host a realm controller or satisfy an isolated workload policy.

User namespace construction is also explicit:

- `broker-preestablished` uses a typed broker operation to create mappings and
  any privileged mount setup before the provider process executes;
- `unprivileged-self-managed` is permitted only when the host allows
  unprivileged `CLONE_NEWUSER`, the mapping uses no privileged ids or
  capabilities, and conformance proves no broker-owned surface is bypassed;
- `none` creates no user namespace.

The virtiofsd-specific namespace path from ADR 0021 is not silently reused for
Bubblewrap or Minijail. A new privileged mapping or mount requirement needs a
typed broker contract.

Finally, a runtime may advertise `realm-controller-host-v1` only when it
provides persistent identity storage with an explicit tamper-resistance
posture. Initially this requires a hypervisor/VMM workload with a persistent
TPM-backed identity. `systemd-user`, `systemd-user-service`, Bubblewrap, and
Minijail do not qualify merely because they can start a process.

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
    usb.securityKey.enable = true;
  };
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
3. Exactly one controller workload is declared for a realm. At runtime, at most
   one authenticated controller generation is authoritative. If competing,
   partitioned, or otherwise ambiguous generations are observed, no new route
   is published and the realm is reported degraded until parent-authorized
   reconciliation selects a generation.
4. `roles.realmController.forRealm` is a scalar realm path. Lists are rejected
   at evaluation, so one declaration cannot collapse multiple realm credential
   domains.
5. The workload runtime provider must advertise full realm-controller support.
   The provider must also advertise `realm-controller-host-v1` and its
   persistent identity protection. Host-user and host-process sandbox providers
   cannot carry credential-bearing realm-controller authority without a future
   accepted identity-protection design.
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
- providing non-secret provider and transport configuration references;
- preparing the child realm's bootstrap network attachment;
- starting the controller workload before publishing the child route;
- authenticating the controller generation over the standard realm protocol;
- draining the child realm and withdrawing its route before stopping or
  replacing the controller workload.

Parent ownership does not make the controller dual-homed. A local controller VM
has one data-plane NIC on the child realm network; parent control uses the
authenticated vsock bootstrap/control channel. It is not attached to the
parent's L2 bridge. If a future provider requires more than one interface, the
runtime must set `net.ipv4.ip_forward = 0`,
`net.ipv6.conf.all.forwarding = 0`, and per-interface
`net.ipv6.conf.<if>.accept_ra = 0`; disable proxy ARP and proxy NDP; install
default-deny cross-interface firewall policy; and prove that no bridge, route,
or network namespace joins the parent and child realm networks.

Controller enrollment material must be available before authenticated guestd
or realm protocol traffic can begin. For local Cloud Hypervisor controllers,
the parent creates a dedicated one-shot controller seed in parent-owned runtime
state, exposes it through a read-only boot-time seed share, and withdraws that
share after the controller acknowledges consumption. The seed contains the
parent public trust anchor, expected controller identity coordinates, operation
binding, expiry, and replay nonce. It is not a Nix-store artifact, persistent
virtiofs share, command-line secret, or general guest-control channel.
After consuming the seed, parent and controller establish a
Cloud-Hypervisor-vsock transport and complete `noise-bootstrap`; ordinary realm
or guest service traffic is unavailable until that ComponentSession succeeds.

The child realm identity private key is generated inside the controller
workload. It is never rendered into Nix, the host bundle, cloud-init plaintext,
or a parent-readable state ledger. Persistent controller identity, provider
state, token caches, and realm audit remain inside the controller workload's
storage and TPM boundary.

### Remote controller workloads use the same role

A controller workload may be remote from its parent. Its runtime and
infrastructure provider change; its role and realm protocol do not:

```nix
d2b.realms.local-root.workloads.work-connector = {
  kind = "local-vm";

  localVm = {
    autostart = true;
    tpm.enable = true;
    graphics.enable = true;
    usb.securityKey.enable = true;
  };
};

d2b.realms.local-root.infrastructureProviders.azure = {
  kind = "azure-vm";
  executor.workload = "work-connector.local-root.d2b";
  credentialRef = "entra-azure-control";
};

d2b.realms.local-root.runtimeProviders.azure-controller = {
  kind = "azure-vm";
  infrastructureProvider = "azure";
};

d2b.realms.local-root.workloads.work-controller = {
  kind = "provider-managed";
  provider = "azure-controller";

  roles.realmController = {
    enable = true;
    forRealm = "work";
  };
};
```

`provider-managed` is the selected generic workload configuration variant for a
workload whose runtime comes from `runtimeProviders`; `azure-vm` is the runtime
or infrastructure provider implementation id, not another workload `kind`.
This replaces the current schema-only `provider-placeholder` variant when live
provider dispatch lands.

The exact typed runtime and infrastructure provider options are new schema
introduced by this decision. Existing inert
`d2b.realms.<realm>.providers` records must be split or migrated into those
bindings when this decision is implemented.

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

Remote enrollment uses a temporary, operation-bound rendezvous on the
configured relay fabric. The parent connector opens the enrollment listener
before infrastructure creation. The infrastructure provider injects only the
parent public key, rendezvous reference, expiry, replay nonce, and expected
resource binding through the provider's approved bootstrap mechanism. The new
VM authenticates to Relay with its managed identity, proves the expected cloud
resource binding, generates its realm key, and completes the d2b enrollment
through the `noise-bootstrap` ComponentSession mechanism. The rendezvous is
revoked before the normal realm route is published. No inbound route to the
local parent and no pre-existing child route is required.

### Provider-agent and transport-connector placement is derived

Three responsibilities may be co-located but are not the same role:

| Responsibility | Authority |
| --- | --- |
| Realm controller | Realm policy, registry, audit, routing, and semantic operations. |
| Infrastructure provider executor | Provider API calls and lifecycle of provider-hosted workloads. |
| Transport connector | Provider-specific transport authentication and outbound session establishment. |

Only `roles.realmController` is asserted directly by a generic workload.
Infrastructure-provider and transport-connector behavior is derived from the
provider or transport binding that references the executor workload. This avoids
two independently configurable declarations claiming the same authority.

For a local controller, all three responsibilities may live in one dedicated,
Entra-managed controller VM. For a remote controller, a local Entra-managed VM
may remain the provider executor and Azure Relay transport connector while the
remote VM owns the realm controller:

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

### Realm transport configuration is the connectivity source of truth

Connectivity is configured from `d2b.realms.<realm>.transport`; a separate
gateway or realm-entrypoint object is not introduced.

`transport.provider` is a reference to a typed `transportProviders` instance,
not an inline provider kind:

```nix
d2b.realms.work.transportProviders.azure-work = {
  kind = "azure-relay";

  connector = {
    workload = "work-connector.local-root.d2b";
    credentialRef = "entra-work-relay";
    transportAuthentication = "entra-user";
  };

  controllerTransportAuthentication = "managed-identity";
};

d2b.realms.work.transport = {
  enable = true;
  provider = "azure-work";
  fabricRef = "work-relay";
  descendantAccess = "delegated";
  peerShortcuts.enable = true;
};

d2b.realms.payments.transport.inheritFrom = "work";
```

The two `*TransportAuthentication` fields authenticate endpoints to Azure
Relay only. The d2b peers still complete `noise-realm` ComponentSession
authentication before semantic traffic.

`fabricRef`, endpoint references, and credential references are opaque,
non-secret identifiers. The connector resolves its credential reference inside
the selected workload. The host and parent bundle never resolve or copy the
credential.

Nested realms may inherit the same transport provider and fabric from an
ancestor, but they receive distinct peer identities and scoped transport
credentials. Inheritance does not share controller token caches, realm private
keys, or a single authorization identity.

If both peers have direct authenticated connectivity, they may use a direct
transport without a connector workload. If Relay authentication or work
credentials are required, the configured connector owns that side of the
transport. There is no root, `sudo`, host-token, or direct-network fallback.

### Shared-transport peer shortcuts separate control and data paths

Strict tree routing remains the authorization model. Shared-transport shortcuts
optimize only the data path:

```text
control:
  source -> source controller -> applicable ancestors -> target controller

data after authorization:
  source peer -> shared transport fabric -> target peer
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

The grant contains no token cache, transport credential, raw endpoint, provider
resource id, command payload, stream data, or user-supplied label.

Source and target bind the grant digest into their end-to-end peer-session
handshake. The transport provider supplies an opaque, one-time binding below
the policy DTO. ComponentSession authenticates and encrypts the stream end to
end; provider-specific transport authentication establishes reachability only.

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

Each endpoint sends a signed `PeerShortcutClosed` control message to the
authorizing ancestor when it observes normal completion, peer disconnect, local
cancellation, or transport failure. The message binds the shortcut id, endpoint
role, controller/workload generation, terminal reason, byte-count class, and
local close time; it contains no payload or provider endpoint. The authorizing
ancestor records endpoint reports independently and emits the final teardown
record when both arrive.

If one or both reports never arrive, the ancestor closes its authorization
lease at grant expiry or route/policy revocation and records an
`endpoint-unconfirmed` terminal class. Absence of a peer report is never
converted into a successful completion. Participating peers always retain their
own local establishment and teardown records.

The endpoint-reported byte-count class is diagnostic and untrusted. It cannot
prove how much data crossed the transport and must not drive security policy,
billing, exfiltration detection, or compliance conclusions. A transport provider
may expose separate provider-attested counters when available, but those are a
distinct typed observation with an explicit trust posture.

### Existing direct-shortcut contracts are generalized

The route engine already contains `DirectShortcutAuthorizationMetadata`,
`DirectShortcutState`, and typed teardown reasons. Implementation of this ADR
generalizes that foundation to peer transport shortcuts:

```text
PeerShortcutTransport
  = native-direct
  | shared-fabric
```

Authorization metadata remains transport-address-free. Provider-specific
rendezvous data belongs in a separate bounded transport binding keyed by the
shortcut id.

This decision refines ADR 0043's restriction that direct shortcuts use only
native underlay reachability. Native direct transport still must not add
STUN/ICE, NAT traversal, a VPN, or an overlay. A `shared-fabric` shortcut is
allowed only when both peers already participate in the same configured
transport fabric and the transport provider advertises shortcut support. It
does not discover or construct a new network path.

Policy or route revocation applies to established streams, not only future
rendezvous. A transport provider advertising `active-shortcut-revoke-v1` must
close the established binding when the authorizing ancestor revokes it, while
both peer muxes close the named stream. Providers without active revocation may
support shared-fabric shortcuts only with a maximum 60-second session grant and
policy-authorized renewal; expiration closes the stream before renewal.
Provider credential or listener revocation that affects only future
connections is insufficient by itself.

If a shortcut cannot be established, the operation follows an explicitly
authorized parent transport path or fails with a typed transport error. There is no
silent direct-network, provider-native, SSH, or generic tunnel fallback.

### Workloads may participate directly in a shared transport

Eliminating controller hops requires the source and target workload agents, not
only their controllers, to participate in the transport fabric.

A workload may advertise `transport-client-v1` only when its runtime supplies a
guestd-compatible or d2b peer agent. The controller delegates a short-lived,
workload-scoped transport access grant or the workload authenticates the
transport independently through a provider-supported identity such as Azure
Managed Identity.

The workload never receives:

- the controller's transport credential;
- an Entra refresh token from another VM;
- a realm identity private key;
- authority to advertise descendants;
- a grant broader than its workload identity and negotiated operations.

If the transport provider cannot issue or validate a workload-scoped transport
identity, that workload cannot use a direct shared-fabric shortcut. Its traffic
continues through the authorized controller/connector path.

### Entra, YubiKey, browser, and developer authentication

The physical YubiKey is shared at the CTAP ceremony layer, not at the token
cache layer.

D2b's security-key proxy may expose persistent virtual FIDO devices to the
controller or connector VM, a work browser VM, and a work development VM. The
host broker serializes ceremonies so only one transaction uses the physical key
at a time. Each VM performs its own Entra authentication and stores its own
tokens.

Physical touch alone is not sufficient intent when multiple VMs can queue a
ceremony. Before forwarding a CTAP operation that requires user presence or
verification, the proxy requires explicit approval through a trusted d2b
surface showing the source realm, workload, operation class, and RP id when the
RP id can be safely parsed. A background VM cannot consume a touch intended for
the foreground VM. Approval is single-ceremony, expires with the queue entry,
and defaults to deny. Window focus may improve presentation but is never the
authorization signal.

The CTAP proxy uses a closed command policy. The default browser/provider
profile permits discovery, credential creation, assertion, assertion
continuation, PIN/user-verification exchange, and other explicitly enumerated
non-destructive commands required by supported WebAuthn clients. It denies
authenticator reset, credential deletion/management, biometric enrollment,
authenticator configuration, vendor commands, and unknown CTAP commands. A
future administrative authenticator-management flow requires a separate
exclusive operation, explicit trusted confirmation, and its own audit event.

Ceremony and queue leases remain bounded. The current 120-second active
ceremony timeout and 15-second contention wait remain finite defaults. The
schema defines hard upper bounds; operators may reduce them but cannot disable
timeouts or configure an unbounded lock. Disconnect, denial, or timeout sends
cancellation where the device protocol supports it and releases the
physical-key lease.

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
status reports `interaction-required`. D2b never opens an authentication window
merely because status changed. The operator explicitly starts the flow with
`d2b realm provider authenticate <realm-path> <provider-id>` or an equivalent
deliberate desktop action. Only then may d2b open a provider-owned
authentication window through the Wayland proxy. It must not steal focus,
retry indefinitely, or expose a direct host compositor fallback.

`interaction-required`, `interaction-started`, `interaction-completed`, and
`interaction-failed` are bounded status/event classes exported to CLI status,
desktop notifications, tracing, and metrics. Labels may include provider type,
realm class, and result class, but never a user identity, token subject, RP id,
or provider endpoint.

The CTAP proxy covers FIDO/WebAuthn only. PIV, CCID, OTP, and OpenPGP interfaces
still require exclusive USB ownership. A single physical key cannot
simultaneously use CTAP proxying and USBIP for those interfaces; use explicit
handoff or a second key.

## Authorization and security invariants

Implementation must preserve all of the following:

1. A controller role is parent-authorized configuration, not a capability a
   guest can self-assert.
2. Controller, provider-executor, and transport-connector peer identities are
   authenticated before any state lookup, token resolution, provisioning, or
   route publication.
3. Provider and transport tokens stay in the configured credential-owning workload.
4. Entra, managed identity, TLS, Relay, vsock, and Unix transport identities
   are never mapped to local daemon roles or broker authorization.
5. The remote controller re-originates privileged local effects through its own
   broker; no remote peer receives the broker wire protocol.
6. A parent may stop or replace the controller workload but cannot use its
   storage ledger as authority to repair child realm state.
7. Transport reachability does not expose semantic traffic before
   ComponentSession authentication and does not merge realm policy, identity,
   audit, or credential domains.
8. Shortcut authorization follows the same parent/child policy chain as the
   non-shortcut route.
9. Raw transport endpoints, provider resource ids, token subjects, user ids,
   device ids, command data, and stream payloads are forbidden as metric labels.
10. Ambiguous controller identity, route generation, shortcut state, or
    infrastructure ownership is preserved and reported degraded rather than
    guessed, killed by PID, or broadly cleaned up.
11. Noise static private keys stay inside their realm/workload identity
    boundary. Ephemeral keys, transport cipher state, record nonces, sockets,
    and session keys are never persisted or adopted across reconnect.
12. The selected authentication mechanism, service protocol, codec, schema,
    limits, and transport channel binding are covered by the authenticated
    session transcript; downgrade or mismatch fails before API dispatch.
13. API requests inherit the authenticated session principal. A payload cannot
    replace or widen that identity, and method-required capabilities are
    derived from trusted service metadata.

## Audit retention and export

Every realm controller owns an append-only local audit log with bounded
rotation and retention. `d2b.realms.<realm>.audit.retentionDays` inherits
`d2b.site.audit.retentionDays` and therefore defaults to 14 days unless the
realm selects a stricter policy. Disabling retention or silently discarding
records is not a supported remote-controller posture.

Remote and replaceable controller workloads must advertise
`realm-audit-export-v1`. This is a semantic, authenticated export operation to
the observer or archive sink selected by realm policy; it is not generic file
access and does not make raw realm audit readable by the physical host or an
ancestor by default. Exported batches are signed, sequence-bounded, encrypted
to the selected sink, and acknowledged by checkpoint digest.

Before a planned controller replacement, the parent requires a signed audit
checkpoint and drains the old controller. The parent records only the
checkpoint digest, sequence range, controller generation, and result class. If
forced loss makes export impossible, replacement may proceed only through an
explicit recovery operation that emits an `audit-gap` event and reports the
realm degraded until acknowledged. Shortcut grants, peer close reports, policy
decisions, credential interaction boundaries, and provider lifecycle
operations are included in the retained/exported audit sequence. Component
session establishment, selected mechanism/service classes, rejection class,
replay, downgrade refusal, and terminal reason are audited with bounded
identities; keys, proofs, nonces, raw endpoints, and payloads are excluded.

## Failure and continuation behavior

- If an infrastructure-provider executor is unavailable, existing remote
  controllers continue running, but create, power, replace, and delete
  operations fail visibly.
- If a local transport connector is unavailable, the remote realm may continue
  operating, but local reachability through that connector is unavailable.
- If the remote controller is unavailable, the realm is unavailable even when
  its infrastructure and Relay endpoint still exist.
- If the selected transport is unavailable, no provider-native, SSH, or
  alternate-network fallback is attempted unless a separately configured and
  authorized transport binding exists.
- Daemon, connector, and controller restarts are continuation events.
  Reconnection uses realm identity, controller generation, operation ids, route
  generations, and shortcut ids rather than persisted sockets or pidfds.
- Every reconnect performs a fresh Noise or local-auth handshake. In-flight
  ttrpc calls fail or retry through operation idempotency; named streams resume
  only through their explicit generation/cursor contract.
- Expired or superseded shortcut grants are not adopted.
- Losing or replacing persistent controller identity is an explicit
  re-enrollment event, not an automatic repair.

## Public and private contract changes

Implementation requires coordinated changes across:

- the canonical `d2b-contracts::provider` DTO and schema module;
- the canonical `d2b-contracts::session` handshake, authentication, envelope,
  limits, and typed-error module;
- the `d2b-provider` base and specialized provider interfaces;
- the `d2b-session` component-session runtime and authenticator interfaces;
- ttrpc/protobuf daemon, realm, guest, and provider service contracts;
- type-first provider implementation crates and typed registries;
- `d2b-provider-testkit` conformance suites and provider naming policy;
- `d2b.realms.<realm>.workloads.<workload>.roles.realmController`;
- typed runtime and infrastructure provider bindings replacing the ambiguous
  inert provider record;
- `d2b.realms.<realm>.transport` provider, fabric inheritance, connector, and
  shortcut policy;
- workload, controller, provider-executor, and transport-client capability
  advertisements;
- controller-workload bootstrap and generation DTOs;
- peer shortcut authorization and transport-binding DTOs;
- realm-controller, workload, session, and transport status output;
- bundle artifacts, generated schemas, reference documentation, and migration
  guidance.

Security-sensitive schema changes require the normal bundle/schema version
bumps. Existing `unsafe-local` provider-kind values and old inert provider
records receive explicit migration errors after the selected cutover; no
success-shaped compatibility fallback is added. Every migration error names
the obsolete option or value, its exact replacement, and the provider/realm
migration guide.

## Validation requirements

Implementation is incomplete without:

- source-policy tests proving provider crate names use a recognized type-first
  axis and the implementation id matches the descriptor;
- dependency-direction tests proving `d2b-contracts` does not depend on
  `d2b-provider` or an implementation, and `d2b-provider` does not
  depend on an implementation, cloud SDK, daemon, broker, codec, or concrete
  transport;
- dependency-direction tests proving every
  `d2b-provider-<type>-<implementation>` crate is a leaf adapter that does not
  depend on `d2bd`, `d2b-priv-broker`, or another provider implementation;
- dependency-direction tests proving `d2b-session` depends only on contracts,
  provider interfaces, codec-neutral service DTOs, and approved crypto/runtime
  primitives, never daemon, guest, broker, or concrete transport code;
- policy tests proving provider implementations reuse
  `d2b-contracts::provider` DTOs rather than declaring shadow serialized types;
- conformance tests for every registered provider's primary interface and
  advertised optional capabilities;
- registry tests proving optional capability descriptor claims and
  capability-specific trait registrations match exactly without trait-object
  downcasting;
- contract tests proving `ProviderOperationContext` remains serializable and
  runtime cancellation/deadline state exists only in `ProviderCallContext`;
- transport conformance tests for Unix-stream, native-vsock,
  Cloud-Hypervisor-vsock, loopback, direct-network, and Azure-Relay adapters,
  including purpose gating, bounded endpoints, liveness, reconnect, and active
  revocation claims;
- component-session tests for fixed bootstrap framing, Noise transcript
  binding, downgrade rejection, channel binding, replay, hard limits,
  keepalive, close, record fragmentation/reassembly, and forbidden pre-auth
  semantic traffic;
- fixed Noise profile test vectors plus fuzz/property tests for handshake and
  encrypted-record parsers, including truncation, duplicate/reordered
  fragments, oversized ttrpc frames, nonce exhaustion, and reconnect;
- authenticator tests for local peer credentials, realm Noise keys, guest PSKs,
  one-time bootstrap, and optional mTLS, including rejection of `none` and
  cross-purpose credential reuse;
- ttrpc/protobuf contract tests for all four service ids, method-derived
  capabilities, deadlines, idempotency, typed errors, and operation-bound named
  stream opens, with the d2b 1 MiB cap enforced before allocation;
- source-policy tests proving public daemon, broker, and unsafe-local helper
  seqpacket/SCM_RIGHTS endpoints are not silently registered as generic byte
  transports;
- Nix evaluation tests for one-controller-per-realm, direct-parent ownership,
  cycle rejection, scalar-only `forRealm`, provider capability gating, typed
  transport-provider references, ancestor-only transport inheritance, and
  derived placement;
- runtime route tests proving competing, partitioned, or otherwise ambiguous
  controller generations publish no route, report the realm degraded, and
  reject superseded generation grants until parent-authorized reconciliation;
- Nix evaluation tests proving obsolete `unsafe-local` provider-kind values,
  inert provider records, old gateway declarations, and compatibility aliases
  fail with the documented actionable migration errors;
- tests proving a controller cannot control its own substrate;
- bootstrap tests proving the local pre-guestd seed share and remote temporary
  relay rendezvous carry only bounded operation-bound enrollment material, are
  replay protected, and are withdrawn before route publication;
- network tests proving parent-owned controller VMs are not parent/child
  dual-homed; IPv4/IPv6 forwarding, IPv6 RA acceptance, proxy ARP, and proxy NDP
  are disabled; and any explicit multi-interface provider installs
  default-deny cross-interface policy;
- sandbox runtime tests covering `host-shared`, `none`, and
  `isolated-namespace` networking plus broker-preestablished and unprivileged
  self-managed user namespace modes;
- provider-executor tests proving credentials are resolved only inside the
  selected workload;
- transport inheritance tests proving nested realms receive distinct identities
  and no copied credential references;
- route-engine tests for shared-fabric shortcut authorization, replay,
  expiration, policy epoch changes, route revocation, signed endpoint-close
  reports, untrusted endpoint byte counts, missing-report expiry, active
  shortcut revocation, maximum-lifetime fallback, and teardown;
- end-to-end tests proving shortcut bytes bypass intermediate controllers while
  every policy boundary records the decision;
- negative end-to-end tests proving shortcut failure either uses the already
  authorized parent transport route or returns a typed transport error without
  probing direct networks, SSH, provider-native APIs, generic TCP, or tunnels;
- negative tests proving a transport-authenticated peer is not local `Admin`;
- negative tests proving no remote realm, workload, transport, or provider peer
  can receive or invoke the local privileged broker wire protocol;
- YubiKey tests proving controller, browser, and developer ceremonies require
  trusted per-ceremony intent, serialize without token-cache sharing, reject
  destructive/unknown CTAP commands, cancel on disconnect, and release leases
  at the bounded ceremony and queue timeouts;
- restart/adoption tests for local and remote controller workloads, including
  systemd-user scope adoption outside `d2b.slice`, rejection of expired or
  superseded shortcut grants, and mandatory re-enrollment after controller
  identity loss;
- audit tests covering retention/rotation, signed export checkpoints, planned
  replacement drain, forced-loss `audit-gap`, and shortcut audit completeness;
- status/telemetry tests proving `interaction-required` is visible without
  leaking user, token, RP, or endpoint identity;
- redaction tests covering provider ids, endpoints, Entra identities, Azure
  resource ids, and shortcut metadata.

## Consequences

### Positive

- Gateway VMs become ordinary workloads with a precise role.
- Provider crates sort by authority type and expose one recognizable interface
  and conformance contract.
- Existing contract ownership is preserved: provider implementations and
  traits share one serialized DTO/schema source in `d2b-contracts`.
- Unix, vsock, direct-network, and Azure Relay connectivity share one byte
  transport contract without sharing authentication semantics.
- Noise-authenticated ComponentSession and ttrpc/protobuf services replace
  duplicated framing, hello, authentication, and request-correlation code.
- Local and remote realm controllers use one configuration and protocol model.
- Controller hosting lifecycle cannot become self-referential.
- Provider, transport, session, and controller responsibilities have explicit
  credential and authority boundaries.
- Nested realms can share one transport fabric without forcing stream bytes
  through every controller.
- The existing route-shortcut policy engine remains useful and gains a
  provider-neutral transport binding.
- One physical FIDO key can serve multiple work VMs while each retains an
  independent Entra session.

### Negative

- Provider configuration becomes more explicit and requires migration from the
  current inert `providers` records.
- The workspace-wide provider rename is intentionally disruptive and must update
  every manifest, Nix build, policy gate, and documentation reference together.
- Component-session migration touches public daemon, realm-peer, guest-control,
  codec, mux, generated protobuf, and error contracts and therefore requires a
  coordinated versioned cutover.
- Noise key enrollment and rotation become new persistent identity lifecycle
  responsibilities.
- Remote controllers need a parent-owned provider executor even after
  enrollment.
- Direct workload shortcuts require transport-capable guest agents and scoped
  transport identities.
- Shared-transport revocation and audit are more complex than hop-by-hop byte
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

### Name the interface crate `d2b-provider-api`

Rejected. The suffix is not used for equivalent Rust crate boundaries in this
repository and adds no information beyond the `provider` domain noun. The
serialized API is already named `d2b-contracts`; `d2b-provider` is the
in-process provider trait surface.

### Put async provider traits directly in `d2b-contracts`

Rejected. `d2b-contracts` owns serialized, versioned data shared across process
and crate boundaries. Async trait objects, typed registries, and runtime error
wrappers are an in-process Rust plug-in API with different evolution and
dependency requirements. Keeping those traits in `d2b-provider` prevents
Tokio/runtime concerns from becoming part of the wire-contract layer while
still requiring every trait method to consume and return canonical
`d2b-contracts` types.

### Use 9P as the component API

Rejected as the authoritative control protocol. 9P supplies bounded binary
messages, version/msize negotiation, concurrent request tags, `Tflush`
cancellation, and authentication fids, but its semantic model is
attach/walk/open/read/write/clunk over a file tree. `Tauth` deliberately leaves
the authentication exchange unspecified, and 9P supplies no encryption,
realm/node/workload identity, method-derived capability policy, idempotency,
typed operation errors, or native event/stream semantics.

A future read-only or tightly constrained filesystem projection of d2b status,
logs, or artifacts may use 9P. Such a projection is a client of typed d2b
services and never becomes repair or authorization authority.

### Use gRPC over HTTP/2

Rejected for the internal component baseline. gRPC provides mature streaming,
flow control, deadlines, cancellation, metadata, and mTLS, but requires the
HTTP/2 protocol stack. Carrying HTTP/2 inside Cloud-Hypervisor-vsock and Azure
Relay streams would duplicate d2b transport/session/mux concerns and increase
dependency and parser attack surface. A future external API gateway may expose
gRPC without changing the internal component session.

### Use Cap'n Proto RPC

Rejected for this cutover. Cap'n Proto supports arbitrary streams,
capability-secure object references, multiplexing, and promise pipelining, but
adopting its distributed-object model would redesign d2b authorization around
delegable connection-scoped object capabilities. Its automatic direct
third-party connectivity also requires constraints beyond the selected strict
realm tree. The current operation/idempotency/audit model remains canonical.

### Use the full libp2p or SSH stack

Rejected. Both provide proven layered transport authentication and stream
multiplexing, but their product models are broader than d2b's requirements.
Libp2p brings swarm, peer discovery, multiaddress, and optional NAT-traversal
concepts that conflict with strict realm-tree routing. SSH brings user-login,
shell, exec, and generic forwarding semantics that d2b explicitly excludes from
its control plane. D2b follows the useful transport -> Noise/TLS -> protocol
negotiation -> mux layering without adopting either runtime.

### Use ttrpc without ComponentSession

Rejected. Ttrpc intentionally targets reliable low-latency process connections
and omits authentication, handshake, ping/reset, network recovery, and flow
control. It is selected only as the typed control-RPC framing and service layer
inside an authenticated, bounded ComponentSession.

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

### Treat shared transport membership as authorization

Rejected. Transport credentials establish transport access only. Realm keys,
controller generations, tree policy, operation capabilities, and scoped
shortcut grants remain authoritative.

### Add a generic network tunnel for nested realms

Rejected. D2b routes semantic operations and named streams, not arbitrary
cross-realm IP traffic. Shared-fabric shortcuts are operation-scoped and do not
create a VPN or alternate realm topology.
