# ADR 0046 Zone routing

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-zone-routing` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-zone-routing`, `d2b-bus` Zone route engine, core-controller ZoneLink handler |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-componentsession-and-bus`, `ADR-046-resource-api-and-authorization`, `ADR-046-resource-object-model`, `ADR-046-resource-reconciliation` |
| Supersedes | `RealmEntrypointTable`/`EntrypointMode`/`RealmControllerPlacement` enum (→ compiler-only `parentZone` topology + ZoneLink transport + private Zone runtime bootstrap placement); `RouteTreeEngine`/`routing.rs` route contracts (adapted, not deleted); `RemoteNodeRegistry` (→ ZoneLink controller); `OperationRouter` (→ ZoneLinkIdempotencyKey in d2b-bus); `realm_access_resolver` module (→ ZoneEntrypointResolver); `realm-controllers.json`/`realm-identity.json` bundle artifacts (→ Nix-authored Zone settings/ZoneLink resources); `d2b.realms.*` Nix option namespace (→ `d2b.zones.*`); `WorkloadId` routing targets (→ typed Guest/Host resource refs); `CapabilitySet`-only routing authz (→ per-hop RBAC narrowing + allocator-issued capability scope) |

## Source and reuse policy

The pre-ADR-0045 v3 baseline has a mature pure tree-routing engine in
`d2b-realm-core/src/route_engine.rs` and a matching routing contract in
`routing.rs`. These are the primary baselines. The `d2b-realm-router`
operation router owns principal/capability/idempotency and is the second
primary baseline.

Main commit `a1cc0b2da4a08ca3240a770a972fe4da6f912bef` contains an
in-progress ADR 0045 ComponentSession and Provider implementation. It is
**not** current v3 behavior - it lives on main and not in the pre-ADR45
baseline - but it is the primary reuse source for bus/session/transport/
client/provider work. The per-crate inventory follows. Each entry records
exact file/symbol/test, selected behavior, v3 destination, and the ADR45
assumptions that are excluded.

## Main commit reuse inventory

All sources in this section are from main commit
`a1cc0b2da4a08ca3240a770a972fe4da6f912bef`. They are NOT currently present
in the pre-ADR45 v3 baseline. Do not cite them as v3 baseline behavior.

### d2b-session - ComponentSession v2 runtime

**Crate**: `packages/d2b-session/`

| Module | Key symbols | Selected behavior |
| --- | --- | --- |
| `src/handshake.rs` | `NoiseHandshake`, `HandshakeCredentials` (Nn/Kk/IKpsk2), `EstablishedHandshake`, `NegotiatedOffer`, `encode_offer`, `negotiate_offer`, generation discovery functions | Three Noise profiles (25519·ChaChaPoly·SHA256); prologue = preface‖canonical‑offer binds offer to transcript; INIT/ACCEPT authenticated payloads; generation discovery pre-handshake (SHA256 binding); fail-closed on all key/payload/step errors |
| `src/lifecycle.rs` | `SessionLifecycle`, `SessionPhase` (Established/Disconnected/Reconnecting/Closing/Closed), `KeepaliveAction` | Keepalive ping/pong with nonce tracking; reconnect up to `MAX_RECONNECT_ATTEMPTS=10` within `MAX_RECONNECT_WINDOW_MS=300000`; generation increment on each reconnect; deterministic close record with `CloseReason`/`Remediation` |
| `src/streams.rs` | `NamedStreamMux`, `StreamId`, `StreamPhase` (Open/HalfClosedLocal/HalfClosedRemote/Closed/Reset), `StreamEvent` | Credit-based named-stream state machine; max `MAX_ACTIVE_NAMED_STREAMS=128`; per-stream send/receive credit; half-close, reset, remove-terminal; send-credit reservation/refund; receive-credit release |
| `src/cancellation.rs` | `Cancellation` (Arc+AtomicBool+Notify), `RequestRegistry` | Per-generation request registry; cancel-before/after-dispatch distinction; `CancelResult` (5 variants); `cancel_all` on session close; zero allocation `cancelled()` async wait |
| `src/attachment.rs` | `OwnedAttachment`, `AttachmentPayload` (trait: `validate_descriptor`/`close`/`as_any`/`into_any`), `AttachmentValidationError` | Transport-observed properties validated against authenticated descriptor post-decryption; close exactly once (Drop impl guards double-close); unbound/pre-bound states; transfer without close via `into_payload` |
| `src/driver.rs` | `ComponentSessionDriver` (trait, 20 async methods), `SessionDriverHandle` (clonable Arc wrapper) | Object-safe async trait; ttrpc send/receive; inbound call registration/completion/removal; attachment send/receive; named stream open/send/receive/credit/close/reset; keepalive drive; control receive; session close; handle serializes requests via `Arc<Mutex>` |
| `src/engine.rs` | `SessionEngine`, `SessionEvent` | Full async drive loop over `OwnedTransport`; reassembles fragments; dispatches record kinds (ttrpc, named-stream, attachment, keepalive, cancel, close); integrates scheduler/mux/protector |
| `src/transport.rs` | `OwnedTransport` (trait), `TransportPacket`, `TransportDescriptor`, `TransportError` | Transport-agnostic packet send/receive/close; packet carries bytes + `Vec<OwnedAttachment>`; descriptor declares class/locality/atomic/attachment-support |
| `src/scheduler.rs` | `FairScheduler`, `QueueClass` (Control/Named/Attachment), `OutboundFrame` | Round-robin interleaving across named-stream queues; credit-gated named sends; control queue bypass; bounded queues per `LimitProfile` |
| `src/record.rs` | `RecordProtector`, `ProtectedRecord` | AEAD protect/unprotect with directional sequence number; replay cache (1024 entries); sequence window enforcement |
| `src/bootstrap.rs` | `Secret32` (zeroizing), `BootstrapPsk`, `AdmittedBootstrapPsk`, `BootstrapAdmission` | Single-use PSK admission; binding to operation; consumed-once checked at runtime |
| `src/metrics.rs` | `MetricEvent`, `MetricsSink`, `NoopMetrics` | Transport/session/scheduler metric events; injected via generic; default no-op |

**Tests** (`packages/d2b-session/tests/component_session.rs`,
`tests/noise_vectors.rs`):

| Test function | Covers |
| --- | --- |
| `fixed_negotiation_and_all_noise_profiles_are_strict` | Offer/policy negotiation; schema fingerprint binding |
| `public_daemon_handshake_rejects_a_guest_only_schema_peer` | Cross-purpose handshake rejection |
| `protected_records_are_directional_sequenced_and_replay_safe` | Record protect/unprotect/replay |
| `protected_record_boundaries_and_tampering_fail_closed` | Truncation/tamper detection |
| `fragmentation_is_bounded_and_rejects_reordering` | Fragment reassembly ordering |
| `deadline_intersects_wall_monotonic_and_ttrpc_budgets` | DeadlineBudget wall/mono/ttrpc intersection |
| `cancellation_is_generation_bound_and_shared` | RequestRegistry per-generation; cancel before/after dispatch |
| `lifecycle_keepalive_close_and_reconnect_change_generation` | SessionLifecycle state transitions |
| `named_stream_state_and_scheduler_have_independent_credit_and_fairness` | NamedStreamMux + FairScheduler credit |
| `bootstrap_is_operation_bound_expiring_single_use_and_redacted` | BootstrapAdmission single-use |
| `local_generation_discovery_establishes_the_authenticated_generation` | Pre-handshake generation discovery |
| `engine_drives_fragmented_ttrpc_and_request_cancellation` | SessionEngine ttrpc + cancel |
| `driver_handle_is_clonable_object_safe_and_leaves_ttrpc_correlation_to_adapters` | ComponentSessionDriver object safety |
| `driver_fragments_one_mib_logical_stream_under_256_kib_credit` | 1 MiB fragmentation |
| `driver_withholds_logical_delivery_credit_until_grant` | Named-stream credit backpressure |
| `engine_binds_acknowledges_and_releases_owned_attachments` | Attachment bind/ack/close |
| `engine_reconnect_rehandshakes_with_the_next_generation` | Reconnect + generation increment |
| `every_canonical_w2_vector_verifies_exactly_with_snow_0_10` | Noise test vectors (canonical) |

**V3 destination**: `packages/d2b-bus/src/session/` - adapt as d2b-bus
ComponentSession layer; rename where ADR45-specific names appear.

**Excluded ADR45 assumptions**:
- `serve_ttrpc_services` binds the ADR45 fixed 4-unit local-root endpoint
  set; v3 uses allocator-issued sockets per Zone. The function is excluded;
  the socket-plumbing pattern is adapted per Zone transport Provider.
- `EndpointPurpose::GuestBootstrap`/`GuestDirect` are ADR45 guest bootstrap
  paths; v3 guest enrollment goes through Zone resource model. Purposes are
  extended/renamed in v3 contracts; see ADR046-routing-009.
- `GUEST_SESSION_CREDENTIAL_*` constants and credential embedding in the
  handshake offer are ADR45 guest bootstrap wire; excluded from v3
  ZoneLink session bootstrap.

---

### d2b-contracts/src/v2_component_session.rs - wire contract

**File**: `packages/d2b-contracts/src/v2_component_session.rs`

| Symbol class | Key symbols | Selected behavior |
| --- | --- | --- |
| Protocol constants | `PREFACE_LEN=16`, `PREFACE_MAGIC`, `COMPONENT_SESSION_MAJOR/MINOR=2.0`, `MAX_ACTIVE_NAMED_STREAMS=128`, `MAX_PACKET_ATTACHMENTS=32`, `MAX_REQUEST_ATTACHMENTS=64`, `MAX_OPERATION_ATTACHMENTS=128`, `MAX_SESSION_ATTACHMENTS=256`, `MAX_PROCESS_ATTACHMENT_CREDITS=2048`, `MAX_HOST_ATTACHMENT_CREDITS=8192`, `RESERVED_CONTROL_FDS=64`, `MAX_NAMED_STREAM_QUEUE_BYTES=262144`, `MAX_AGGREGATE_NAMED_STREAM_QUEUE_BYTES=4194304`, `MAX_LOGICAL_MESSAGE_BYTES=1048576`, `LOCAL_HANDSHAKE_DEADLINE_MS=5000`, `REMOTE_HANDSHAKE_DEADLINE_MS=15000`, `MAX_RECONNECT_ATTEMPTS=10`, `MAX_RECONNECT_WINDOW_MS=300000`, `MAX_REQUEST_LIFETIME_MS=900000` | Hard-coded limits carried as const; all validation is fail-closed against these |
| Preface | `ComponentSessionPreface` (`PREFACE_MAGIC` + 2.0 + offer_len); `PrefaceError` | 16-byte fixed-length prefix before canonical offer; version check fail-closed |
| Offer / policy | `HandshakeOffer` (canonical 148-byte encoding), `EndpointPolicy`, `EndpointPolicyIdentity` (140 bytes), `NoiseProfile` (Nn/Kk/IKpsk2), `LimitProfile` (14 fields), `AttachmentPolicy`, `TransportBinding` | Offer must round-trip through canonical binary encoding; prologue = preface‖canonical-offer binds to Noise transcript |
| Enumerations | `EndpointPurpose`, `PurposeClass`, `EndpointRole`, `ServicePackage`, `Locality`, `TransportClass`, `AttachmentPolicyKind` | Closed enumerations with stable u8 tags; `closed_enum!` macro enforces exhaustive matching |
| Attachment | `AttachmentDescriptor`, `AttachmentKind`, `KernelObjectType`, `AttachmentAccess`, `AttachmentCreditClass`, `AttachmentPacket` | Descriptor is encrypted inside the protected record; attachment indexes are packet-sequence-bound; credit classes are Packet/Request/Operation/Session/Process/Host |
| Channel / record | `ChannelId` (control=0, named≥0x0100), `RecordHeader` (24 bytes), `FragmentHeader` (24 bytes), `RecordKind` | Named-stream channel IDs start at 0x0100; record header binds kind/channel/sequence/length |
| Session control | `SessionErrorCode` (20 codes), `CloseReason`, `Remediation`, `CancelRequest/Ack/Result`, `KeepaliveRecord` | Wire-stable error codes; remediation advises Retry/ReplaceGeneration/Permanent |
| Misc | `BoundedVec<T,MIN,MAX>`, `ContractError`, `BinaryError` | Deserialize-time bounds enforcement; fail-closed BinaryError propagation |

**V3 destination**: `packages/d2b-contracts/src/v3/zone_session.rs` - copy
constants and types; re-freeze protobuf field numbers separately for v3;
add v3-specific `ServicePackage` variants (`d2b.zone.v3`, `d2b.resource.v3`);
rename `EndpointRole` variants where ADR45 names appear.

**Excluded ADR45 assumptions**:
- `GUEST_SESSION_CREDENTIAL_*` constants/types are ADR45 guest bootstrap
  wire; excluded from v3 ZoneLink sessions.
- `GUEST_BOOTSTRAP_CREDENTIAL_*` types excluded.
- Protobuf field numbers for existing v2 services must NOT be reused;
  v3 service IDs freeze independently.
- `EndpointPurpose` tag values for ADR45 purposes (e.g. `GuestBootstrap`,
  `ChildRealmController`) are excluded; v3 purposes freeze new tags.

---

### d2b-session-unix - Unix transport and FD credit

**Crate**: `packages/d2b-session-unix/`

| Module | Key symbols | Selected behavior |
| --- | --- | --- |
| `src/adapter.rs` | `UnixSeqpacketTransport`, `UnixStreamTransport`, `PeerIdentityPolicy` (Accepted/Pathname/InheritedSocketpair), `UnixAttachmentPayload`, `OwnedUnixAttachment` | Seqpacket: atomic packet + ancillary FDs; stream: framed with 2-byte record-length prefix; peer credentials verified on first packet for inherited sockets; pathname transport verifies provenance |
| `src/credit.rs` | `CreditPool`, `CreditScopeSet`, `CreditBundle`, `ProcessCreditLimit`, `CreditScope` (6 variants: Packet/Request/Operation/Session/Process/Host) | Multi-scope FD credit reservation with rollback; process limit derived from observed open-FD count; emergency headroom reserved for Process/Host scopes |
| `src/descriptor.rs` | `PeerCredentials`, `PidfdIdentityPolicy`, `DescriptorPolicy`, `VerifiedPacket`, `ObjectIdentity`, `AcceptedAttachment` | Pidfd identity: requires live launch evidence (`/proc/<pid>/fdinfo`); same-kernel-object check via `st_dev`/`st_ino`/`file_type`; descriptor validated post-authentication |
| `src/socket.rs` | `SeqpacketSocket`, `StreamSocket` | Async wrappers over OS sockets |
| `src/systemd.rs` | `InheritedSocketTransport` | SD_LISTEN_FDS activation (ADR45 path - see exclusions) |
| `src/vsock.rs` | vsock transports | NativeVsock/CloudHypervisorVsock (ADR45/Provider transport - see exclusions) |

**Tests** (`packages/d2b-session-unix/tests/unix_session.rs`):

| Test function | Covers |
| --- | --- |
| `ancillary_capacity_is_derived_from_closed_hard_bounds` | CreditScopeSet capacity derivation |
| `process_limit_preserves_emergency_headroom` | ProcessCreditLimit headroom |
| `failed_multiscope_reservation_rolls_back_every_prior_scope` | CreditBundle rollback |
| `staged_credit_reservations_release_once_at_each_scope` | Release idempotency |
| `inherited_passcred_is_verified_but_never_repaired` | SO_PASSCRED passthrough |
| `first_packet_has_exact_directional_credentials` | First-packet credential check |
| `seqpacket_transfer_is_atomic_cloexec_and_object_exact` | FD transfer atomicity |
| `duplicate_kernel_objects_are_rejected_and_cleaned_up` | Duplicate FD rejection |
| `owned_transport_adapters_transfer_packets_and_owned_files_end_to_end` | End-to-end seqpacket |
| `stream_transport_reassembles_partial_and_coalesced_records` | Stream framing |
| `pidfd_identity_requires_live_launch_evidence_and_rejects_unrelated_process` | Pidfd identity |
| `payload_and_control_truncation_scavenge_received_files` | FD scavenge on truncation |

**V3 destination**: `packages/d2b-bus/src/transport/unix.rs` - copy
`UnixSeqpacketTransport`/`UnixStreamTransport`/credit modules verbatim;
adapt `PeerIdentityPolicy` for v3 Zone principal model.

**Excluded ADR45 assumptions**:
- `src/systemd.rs` (`InheritedSocketTransport`, SD_LISTEN_FDS) is tied to
  the ADR45 fixed 4-unit PID1 socket activation. In v3, Zone local sockets
  are pre-bound by the allocator and handed as FDs to the Zone runtime;
  they are inherited-socket, not systemd-SD_LISTEN_FDS. Adapt
  `InheritedSocketTransport` to receive the allocator-issued FD directly
  instead of from the SD_LISTEN_FDS environment variable.
- `src/vsock.rs` vsock transports are transport-Provider-specific paths in
  v3; they are not hardcoded as Zone 0 transport. Adapt as a vsock
  `TransportProvider` implementation.

---

### d2b-client - async typed client

**Crate**: `packages/d2b-client/`

| Module | Key symbols | Selected behavior |
| --- | --- | --- |
| `src/client.rs` | `Client<R,C,W>`, `ConnectedClient`, `CallOptions`, `CancellationToken`, `RetryPolicy`, `MetadataInput`, `Response` | Generic over `TargetResolver`+`ComponentSessionConnector`+`WallClock`; `connect()` returns `ConnectedClient`; `invoke` / `invoke_with_attachments` with retry and cancellation; named-stream helper `named_stream`; relative timeout from wall clock + request lifetime |
| `src/service.rs` | `ServiceHandle`, `ServiceKind` (25 variants), `GeneratedClient` (25 variants), `MethodHandle` | Service inventory driven by `SERVICE_INVENTORY`; generated ttrpc client per service kind; raw ttrpc invocation with timeout |
| `src/session.rs` | `ConnectedSession`, `ComponentSessionConnector` (trait), `SessionFailure`, `StreamDispatcher`, `SessionCall`, `SessionReply` | Session connector trait; stream dispatcher owns per-stream inbox/outbox with credit accounting; `SessionFailure` (BeforeDispatch/Retryable/Ambiguous/Disconnected/Deadline/Cancelled/Protocol) |
| `src/target.rs` | `ServiceOwner`, `TargetInput`, `TransportKind`, `ResolvedTarget`, `RouteRecord`, `RouteTable`, `TargetResolver` (trait), `TransportSelection` | Static route table resolver; exact-transport selection; fail-closed on missing/ambiguous routes |
| `src/daemon_service.rs` | `DaemonClient`, `DaemonMethod`, `DaemonLifecycleRequest`, `DaemonTerminal` | Typed daemon service proxy |
| `src/guest_service.rs` | `GuestClient`, `GuestOperation`, `GuestInspectCall`, `GuestCancelCall`, `GuestRetainedLogCall` | Typed guest service proxy |
| `src/host_socket.rs` | `HostSocketConnector`, `local_daemon_endpoint_identity`, ttrpc pump | Local Unix daemon connection; MAX_IN_FLIGHT_REQUESTS=128; ttrpc bridge over ComponentSession driver |
| `src/error.rs` | `ClientError`, `RemoteErrorKind`, `RetryClass` | Typed error hierarchy; retry classification |

**Tests** (`packages/d2b-client/tests/client.rs`):

| Test function | Covers |
| --- | --- |
| `daemon_guest_proxy_reuses_the_authenticated_session` | Session reuse for guest proxy |
| `absent_daemon_guest_proxy_fails_closed_without_reconnecting` | Fail-closed on missing proxy |

**V3 destination**: `packages/d2b-resource-client/` - adapt as v3 typed
`ResourceClient` and `ZoneServiceClient`; rename `ServiceOwner/TargetInput`
variants from `Realm/Workload` to `Zone/Guest/Host`; add
`TargetInput::ZoneService(ZonePath, ZoneServiceKind)` variant;
replace `ServiceKind` with v3 service inventory.

**Excluded ADR45 assumptions**:
- `ServiceOwner::LocalRoot/Realm/Workload/Provider` ADR45 naming → v3
  uses `Zone/ZonePath/Guest/Host`; the variants are renamed, not reused.
- `ServiceKind` list contains ADR45-specific services (`Realm`, `Broker`,
  `RuntimeSystemdUser`, `Shell`, etc.). V3 extends with `Resource`,
  `Zone`, and `ZoneLink` service kinds; the existing 25 variants are
  evaluated case-by-case. The service inventory serialization format
  (tag assignment) must be re-frozen for v3.
- `HostSocketConnector` derives the daemon peer uid from `d2bd` system
  username; v3 Zone sockets use allocator-issued FDs with authenticated
  static-key pinning, not uid-based trust.
- `CONTROLLER_PIDFD_ATTACHMENT_INDEX`/`BROKER_PIDFD_ATTACHMENT_INDEX`
  attachment conventions are ADR45 pidfd supervision wire; excluded from
  v3 Zone session bootstrap.

---

### d2b-provider/src - Provider registry and RPC proxy

**Crate**: `packages/d2b-provider/src/`

| Module | Key symbols | Selected behavior |
| --- | --- | --- |
| `registry.rs` | `ProviderRegistry`, `ProviderRegistryBuilder`, `ProviderRegistryManager`, `RegistryLimits` (total_in_flight, per_provider_in_flight), `AdmissionOptions`, `InFlightPermit`, `AdmittedProvider` | Registry with lifecycle (Accepting→Draining→Retired); per-provider and total in-flight concurrency caps; `admit()` returns `InFlightPermit` RAII guard; async `shutdown()` waits for drain; `ProviderRegistryManager` handles live registry swap |
| `rpc.rs` | `AuthenticatedProviderRpc` (trait: `session_identity`/`invoke`), `RpcProviderProxy`, `RpcCall`, `RpcResponse`, `RpcOperation`, `RpcPayload`, `SessionIdentity` | Client-side proxy: translates `RpcCall` → typed Provider operation → ttrpc; validates session identity against descriptor; validates capability, health, observation, handle, mutation, lease, plan before dispatch |
| `instance.rs` | `ProviderInstance` (variants: Runtime/Infrastructure/Transport/Substrate/Credential/Display/Network/Storage/Device/Audio/Observability) | Sum type over all provider trait objects |
| `context.rs` | `OwnedOperationContext`, `ProviderCallContext` | Per-call context (metadata, idempotency, deadline, principal) |
| `error.rs` | `ProviderResult<T>`, `ProviderFailure`, `ProviderRuntimeError` | Typed provider error hierarchy |

**V3 destination**: `packages/d2b-provider/` (largely retained and adapted) -
`ProviderRegistry`/`ProviderRegistryBuilder`/`ProviderRegistryManager`
adapt directly; `RpcProviderProxy` adapts for v3 session identity
(ZonePath instead of RealmId); `ProviderInstance` variants extend for
v3 Provider ResourceType model.

**Excluded ADR45 assumptions**:
- `SessionIdentity` carries `provider_id: ProviderId` and
  `provider_type: ProviderType`; v3 providers are identified by
  `Provider/<name>` resource path; `ProviderId` is renamed/reclassified.
- `AdmissionOptions::peer_role: EndpointRole` uses ADR45 role enumeration;
  v3 maps to Zone principal + RBAC binding.
- `PROVIDER_BUNDLE_VERSION=13` (in `d2bd/src/provider_registry.rs`) is
  the ADR45 bundle version; v3 provider catalogs use a separate versioning
  scheme rooted in Zone resource generation.

---

### d2b-provider-toolkit - Provider agent server and conformance

**Crate**: `packages/d2b-provider-toolkit/src/`

| Module | Key symbols | Selected behavior |
| --- | --- | --- |
| `server.rs` | `GeneratedProviderServiceServer` | ttrpc service dispatch for all Provider types; routes by service/method; validates call context, capability, attachment indexes; emits capability/observation/failure responses; concurrency limited via `Semaphore` |
| `adapter.rs` | `ProviderAgentAdapter` (implements `AuthenticatedProviderRpc`) | Client-side adapter for a provider running in a remote agent process; translates `RpcCall` → serialized ttrpc frame → agent ComponentSession |
| `conformance.rs` | `check_descriptor_conformance`, `check_provider_conformance`, `ConformanceError` | Structural conformance check against descriptor; live `inspect` call conformance |
| `registration.rs` | `register_exact_instances`, `ToolkitError` | One-shot factory→registry registration |
| `fixture.rs` | Test fixture helpers | Used in conformance tests |
| `redaction.rs` | Log/metric redaction helpers | Strips sensitive provider data from log/metric surfaces |

**V3 destination**: `packages/d2b-provider-toolkit/` (adapted) - `GeneratedProviderServiceServer`
serves the v3 Provider agent process; `ProviderAgentAdapter` is the
d2b-bus proxy for a Provider running in a Guest/Host Process; `conformance`
module becomes the Provider resource conformance kit.

**Excluded ADR45 assumptions**:
- `run_registered`/`run` standalone entrypoints use ADR45 process
  registration via `SD_LISTEN_FDS` + priv-broker handshake; v3 provider
  agents receive their ComponentSession FD via Zone allocator-issued
  bootstrap binding.
- `ProviderAgentProcess::from_registry` uses ADR45 `RealmId` routing;
  v3 uses ZonePath-rooted service identity.
- The ttrpc service/method name strings (`d2b.provider.runtime.v2.*` etc.)
  are ADR45 protobuf service names; v3 re-freezes names under `d2b.provider.*.v3`.

---

### d2b-realm-router/src/service_v2.rs - RealmServiceServer

**File**: `packages/d2b-realm-router/src/service_v2.rs`

| Symbol | Description |
| --- | --- |
| `RealmServiceServer` | `d2b.realm.v2.RealmService` ttrpc handler; methods: `bootstrap`, `enroll`, `resolve_route`, `authorize_shortcut`, `revoke_shortcut`, `report_shortcut_close`, `inspect`, `cancel` |
| `RealmServiceProcess` | Drive loop: serves with `MAX_DISPATCH_IN_FLIGHT=64` concurrent requests |
| `RealmSessionAuthority` | Per-session identity: realm, peer_role, locality, purpose, credential custody |
| `CredentialCustody` | `None` (host-local) / `GatewayGuest` (relay-backed) |
| `RealmServiceLimits` | `max_bindings=256`, `max_shortcuts=256`, `max_mutation_records=1024`, `audit_capacity=1024` |
| `RealmAuditEvent`, `RealmMethod`, `RealmAuditOutcome` | Per-method audit records |
| `BootstrapBinding`, `EnrollmentBinding`, `ShortcutBinding` | Per-session state tracked in `RealmState` |
| `MutationRecord` | Idempotent mutation dedup record |

Constants: `REALM_SERVICE_NAME="d2b.realm.v2.RealmService"`,
`DEFAULT_MAX_REALM_BINDINGS=256`, `DEFAULT_MAX_SHORTCUTS=256`,
`DEFAULT_MAX_MUTATION_RECORDS=1024`, `DEFAULT_AUDIT_CAPACITY=1024`,
`MAX_CONFIGURED_BOUND=4096`, `MAX_DISPATCH_IN_FLIGHT=64`,
`SHUTDOWN_TIMEOUT=5s`.

**V3 destination**: `packages/d2b-zone-routing/src/service.rs` - adapt
`RealmServiceServer` as `ZoneServiceServer` serving `d2b.zone.v3.ZoneService`;
rename methods (bootstrap→zone-bootstrap, enroll→zone-enroll,
resolve_route→resolve-zone-route, authorize_shortcut→authorize-zone-shortcut);
replace `RealmSessionAuthority` with Zone principal + RBAC binding;
replace `BootstrapBinding`/`EnrollmentBinding` with v3 ZoneLink enrollment
records; add independent `relay` and target-verb checks per forwarding hop;
adapt shortcut model to ZonePath.

**Excluded ADR45 assumptions**:
- `CredentialCustody::GatewayGuest` maps to the ADR45 constellation gateway
  pattern where a gateway guest VM terminates auth and proxies; v3 ZoneLink
  sessions use direct KK between adjacent Zone controllers.
- `RealmSessionAuthority::gateway_peer` with `Locality::Remote` and
  `CredentialCustody::GatewayGuest` is the ADR45 relay path; excluded.
  V3 ZoneLink sessions are always direct controller-to-controller.
- `REALM_SERVICE_NAME` and protobuf package `d2b.realm.v2` are ADR45 wire
  identifiers; v3 uses `d2b.zone.v3.ZoneService`.
- Bootstrap/enrollment credential embedding in `BootstrapBinding` uses
  ADR45 `GuestSessionCredential` wire; v3 Zone bootstrap uses
  allocator-issued ZoneLink bootstrap PSK.

---

### d2bd/src/provider_registry.rs + provider_effects.rs - host Provider composition

**Files**: `packages/d2bd/src/provider_registry.rs`,
`packages/d2bd/src/provider_effects.rs`

| Symbol | File | Description |
| --- | --- | --- |
| `compose_host_provider_registry` | provider_registry.rs | Construct `ProviderRegistry` from `HostProviderComposition` (bindings + effects) |
| `compose_agent_provider_registry` | provider_registry.rs | Construct `ProviderRegistry` from `AgentProviderComposition` |
| `HostProviderBinding`, `HostProviderComposition` | provider_registry.rs | Per-binding factory dispatch for 9 implementation IDs |
| `AgentProviderBinding`, `AgentProviderComposition` | provider_registry.rs | Per-binding factory dispatch for agent/relay providers |
| `StartupProviderRegistry` | provider_registry.rs | `registry()`, `runtime_route()`, `lifecycle_dispatch()`, `begin_lifecycle_invocation()` |
| `DaemonEffectAdapters` | provider_effects.rs | 9 typed effect adapters (runtime/transport/substrate/display/network/storage/device/audio/observability) |
| `ProviderLifecycleDispatch`, `ProviderLifecycleInvocationHandle` | provider_effects.rs | Per-invocation lifecycle tracking with `MAX_TRACKED_LIFECYCLE_MUTATIONS=256` |

**V3 destination**: `packages/d2b-core-controller/src/providers.rs` and
`packages/d2b-core-controller/src/provider_effects.rs` -
`compose_host_provider_registry` adapts as the fixed core-controller
Provider lifecycle handler; `DaemonEffectAdapters` pattern adapts as the
Zone-local Provider effect port set. `ProviderLifecycleDispatch` logic
adapts as the Zone controller's Provider lifecycle handler.

**Excluded ADR45 assumptions**:
- `PROVIDER_BUNDLE_VERSION=13`, `PROVIDER_BUNDLE_SCHEMA_VERSION="v2"` are
  ADR45 bundle version constants; v3 Provider catalogs use Zone-resource-
  generation-bound versioning.
- `AZURE_VM_IMPLEMENTATION_ID` and Azure-specific binding paths are
  ADR45 ACA provider paths; v3 ACA becomes an `InfrastructureProvider`
  implementation under the standard Provider registry, not a special case.
- `validate_host_descriptor` validates against a closed set of
  implementation IDs; v3 Provider resource `spec.implementationId` is
  open-set validated by the Provider factory registry.
- `CLOUD_HYPERVISOR_IMPLEMENTATION_ID`/`QEMU_MEDIA_IMPLEMENTATION_ID`
  paths are v2 substrate runner IDs; v3 runners use Process resources
  with typed `executionRef`.

---

Unrelated ADR 0045 assumptions excluded from all reuse:

- ADR45 child-realm spawn contracts (`child_realm_controller_bootstrap.rs`,
  realm-spawn pidfd protocols, `d2b-contracts/src/generated_v2_services/realm.rs`
  realm-controller child-spawn wire).
- ADR45 delivery seals (xtask delivery wave / panel / seal process).
- ADR45 fixed 4-unit PID1 endpoint inventory (`d2bd.socket`, `d2bd.service`,
  `d2b-priv-broker.socket`, `d2b-priv-broker.service` as invariants); v3
  Zone runtime sockets are allocator-issued, not PID1-owned.
- ADR45 `d2b-contracts/src/generated_v2_services/realm.rs` protobuf field
  assignments; v3 re-freezes independently.
- ADR45 controller static key credential path
  (`d2b-controller-static-v2` systemd credential); v3 controller keys come
  through Zone bootstrap resource enrollment.
- ADR45 `d2b-state/` advisory-lock and audit-segment model; v3 inherits
  the storage lifecycle contract from ADR 0034 but does not embed ADR45
  lock-file paths.

## Baseline Realm architecture and mapping

The pre-ADR-0045 v3 baseline uses `Realm`/`RealmId`/`RealmPath` throughout.
`Zone` has zero code matches in that baseline; the implementation term is
`Realm`. This section documents the **architecture** behind each baseline
symbol before mapping it to the v3 Zone target. Several mappings are
**not** textual renames; they require schema, contract, or runtime changes.

### Identifier and path model

`RealmPath` is a `Vec<RealmId>` written most-specific-first (DNS-style):
`payments.work` = child `payments` under parent `work`. Internally the
`storage_form()` is slash-separated parent-first (`work/payments`).
`RealmPath` grammar: max 16 labels, max 255 rendered bytes, lowercase
`^[a-z][a-z0-9-]*$` labels.

`RealmId` is one label of a `RealmPath`.
`WorkloadId` is the label identifying a workload (VM, session, sandbox)
within a `Realm`.

**ADR 0046 mapping:**
- `RealmPath` → `ZonePath`. The label grammar is preserved. The public
  _target address_ changes from the DNS-form `workload.realm.d2b` string
  to the v3 `Zone/<name>` resource reference model.
- `RealmId` (one label) → `ZoneLabelId`.
- `WorkloadId` → **split**. VM/sandbox workloads become `Guest` resource
  name; bare-metal/local execution becomes `Host` resource name. Semantic
  classification per workload is required; there is no mechanical rename.
- `NodeId` → `Host` resource name or implicit in Zone-local addressing;
  not separately surfaced in the routing contract.

### Placement model

`RealmControllerPlacement` is an enum with six variants:

| Current variant | ADR 0046 Zone mapping |
| --- | --- |
| `HostLocal` | Private Zone runtime bootstrap: local controller, no external transport needed |
| `GatewayVm` | Private Zone runtime bootstrap: gateway-VM host; child identity via the child-local uplink ZoneLink plus allocator binding |
| `CloudFullHost` | Private Zone runtime bootstrap: cloud/remote host; child identity via the child-local uplink ZoneLink |
| `ProviderController { provider }` | Private Zone runtime bootstrap: Provider-managed controller; child identity via the child-local uplink ZoneLink |
| `ProviderAgent { provider }` | Private Zone runtime bootstrap: Provider agent; child identity via the child-local uplink ZoneLink |
| `ProviderSpecific { provider, placement }` | Private Zone runtime bootstrap: provider-schema-validated placement; child identity via the child-local uplink ZoneLink |

This is an **architectural change**: the static placement enum is replaced by
private per-Zone bootstrap configuration plus two explicit Nix inputs:
compiler-only `d2b.zones.<zone>.parentZone` selects the allocator owner, while
the child-local uplink ZoneLink's `childZoneName` and `transportProviderRef`
supply transport and local route/session state. The selected parent allocator
binds the ZoneLink UID and child identity to that private parent edge; no
reciprocal parent-store ZoneLink exists. Neither placement nor `parentZone` is
a public field in `Zone.spec`. `Zone.spec` is `{}` - it carries no authored
fields.

`EntrypointMode` (`HostResident`/`GatewayBacked`) is used in the CLI
target router (`d2b/src/target_routing.rs`) to classify routing decisions.
In v3 this mode enum is subsumed into ZoneLink `spec.transportProviderRef`;
the child alone reads that local transport binding. Parent-side CLI routing and
inspection consume the sealed compiler topology and authenticated
route/projection status, never a parent-local ZoneLink resource.

### Provider trait model

`d2b-realm-provider` defines a family of traits (`WorkloadProvider`,
`DisplayProvider`, `RuntimeProvider`, `PersistentShellProvider`, etc.)
with capability advertising and rate-limit plumbing. These traits exist,
have mock/conformance implementations, and are tested, but:

- `d2b-host-providers` implements some traits (`HostSubstrateProvider`,
  `RuntimeProvider`, `DisplayProvider`) but no production binary depends
  on `d2b-host-providers`.
- `d2b-provider-aca` (`AcaWorkloadProvider`) and `d2b-provider-relay`
  ARE used in d2bd, but for the **ACA gateway display session path**
  (`new_gateway_display_runtime_from_config` at `d2bd/src/lib.rs:1396`),
  not for general workload routing.

**ADR 0046 mapping:**
The provider trait family becomes `Provider` ResourceType with
controller/service/worker Processes. The capability advertisement fields
(`CapabilitySet`, `WorkloadCapabilitySet`, etc.) survive in the v3
allocator-issued capability-scope model. The trait boundary itself is replaced by typed
ResourceSpec/Status fields on the Provider resource.

### CapabilitySet model

Current routing authz is entirely `CapabilitySet`-based: the route engine
propagates allocated capability scopes downward and checks that a requested
capability is covered. There is no per-hop RBAC subject/verb check.

**ADR 0046 mapping:**
v3 adds a **RBAC layer above the allocated capability scope**: at each hop, the
intermediate Zone evaluates both the target verb and a separate `relay` verb
against the authenticated adjacent-Zone subject's RoleBinding.
Capability-scope propagation from `RouteNamespaceAllocation` is preserved.
The dual RBAC check is genuinely new; it does not exist anywhere in the current
baseline.

### Realm config artifacts

Two bundle artifacts are loaded at d2bd and priv-broker startup but are
explicitly inert for routing:

- `realm-controllers.json` (from `realm-controller-config-json.nix`):
  loaded at `d2bd/src/lib.rs:1408`; logs "runtime routing remains
  inert". Carries `RealmControllersJson` with placement, socket paths,
  provider metadata.
- `realm-identity.json` (from `realm-identity-config-json.nix`):
  loaded at `d2bd/src/lib.rs:1425`; logs "runtime trust sessions remain
  inert". Carries per-realm key refs/fingerprints.

Both artifacts ARE valid bundle artifacts (installed root:d2bd 0640,
validated at startup). Their contents are **data** that v3 Zone/ZoneLink
resources will supersede; the daemon/broker never act on routing or
trust-session operations based on them today.

**ADR 0046 mapping:**
- `realm-controllers.json` → superseded by compiler-only `parentZone` topology,
  Nix-authored child-local ZoneLink resources, and the runtime-created Zone
  self-resource. The `RealmControllersJson` schema retires when sealed allocator
  topology owns placement/path and ZoneLinks own transport/route state.
- `realm-identity.json` → superseded by the allocator-sealed ZoneLink bootstrap
  identity and the child-local `spec.transportCredentials` references defined
  by the canonical ZoneLink schema. The `RealmIdentityConfigJson` schema
  retires when ComponentSession enrollment handles key pinning.

### Realm access resolver

`d2bd/src/realm_access_resolver.rs` is a complete implementation of
`resolve_local_root_realm_access()` - it maps a `RealmAccessResolverRequest`
(target string + alias bindings + client capabilities) to a
`RealmAccessResolverResponse` (socket path, controller generation,
placement, capability preflight). The module is declared `pub mod` at
`d2bd/src/lib.rs:117` but **has no callers** in the running daemon;
it is implemented-but-unwired.

**ADR 0046 mapping:**
The access resolver logic is replaced by `ZoneEntrypointResolver` in
work item ADR046-routing-003. The domain expands from "local host-local
socket lookup" to "ZoneLink-based multi-hop routing decision".

### WorkloadTargetIndex (implemented-and-reachable)

`d2bd/src/workload_target_index.rs` (`WorkloadTargetIndex`) builds a
canonical-target→VM-name reverse-lookup index from `realm-controllers.json`
and IS called from `d2bd/src/lib.rs:16745` in `PublicRequestArtifacts`
construction (the request handler path). This is a **live bridge** between
the Realm workload metadata and the legacy VM-name-based dispatch.

**ADR 0046 mapping:**
`WorkloadTargetIndex` retires when Guest/Host resource lookups replace the
legacy VM-name dispatch.

### Wire protocol

`d2b-realm-core/src/frame.rs` (`ConstellationFrame`, `Handshake`,
`OperationRequest`, `OperationResponse`, `StreamOpen`, `StreamData`,
`StreamFlow`, etc.) is the current wire protocol. `d2b-realm-codec-protobuf`
serializes it. `d2b-realm-router` (`PeerSession`, `SecurePeerSession`,
`MuxSession`) implements sessions over this protocol.

The session/mux layer IS used internally within `d2b-realm-router` tests
but is NOT imported by `d2bd` or the CLI (except for the display-session
path in `d2b-gateway-runtime` which uses individual frame types directly).

**ADR 0046 mapping:**
`ConstellationFrame` variants map to v3 d2b-bus frame types. The KK
handshake from `SecurePeerSession` is adapted as the ComponentSession
Noise KK profile. The codec is adapted; the protobuf field assignments must
be re-frozen for v3.

### Gateway display vs. general routing

`d2b-gateway` and `d2b-gateway-runtime` use `RealmPath`, `WorkloadId`,
`RealmId`, `AuthzDecision`, `OperationId`, and `PrincipalId` from
`d2b-realm-core` - but only for the **display-session HMAC handshake
and ACA/Azure Relay Wayland session** orchestration. They do NOT
implement general realm routing. `AcaWorkloadProvider` (from
`d2b-provider-aca`) is instantiated in d2bd for the ACA gateway path
only (`d2bd/src/lib.rs:4127`).

The `RouteRealmClass` variant `GatewayBacked` is a metric label for
existing gateway-backed routing; it is not an implementation of the
gateway routing path itself.

## Overview

Zone routing is the mechanism by which one Zone's resource-plane clients
and runtime service callers reach resources, services, and controllers
homed in a different Zone. The d2b v3 model is intentionally constrained:

- Every resource belongs to exactly one Zone. Resources never migrate.
- Ordinary `*Ref` fields never cross Zone boundaries.
- Every non-root Zone declares exactly one compiler-only scalar `parentZone`;
  `local-root` declares none. The value is a plain declared Zone name, not a
  ResourceRef, and compiles into the sealed allocator-bootstrap topology.
- Every non-root child Zone declares at most one local `ZoneLink/<name>`
  uplink resource, enabled or disabled. It supplies transport and local
  route/session state; the allocator selected by `parentZone` binds that
  resource to the direct parent/child edge in sealed bootstrap state.
- A parent calls the child Zone's `d2b.resource.v3` service through an
  authenticated ZoneLink ComponentSession; it does not obtain a database
  handle, process credential, host path, or cross-Zone resource reference.
- Cross-Zone routing traverses the Zone tree through sessions established by
  each child's local uplink. No direct lateral (sibling-to-sibling) route is
  provisioned without going through a common ancestor.
- The d2b-bus resolves the outbound route for every service call and
  enforces native RBAC at each hop.

```
Nix topology: k1.parentZone = local-root; k2.parentZone = k1

Zone/local-root (K0 parent allocator/route state)
  -> Zone/k1 (child): child-local ZoneLink/k1-uplink
       -> Zone/k2 (grandchild): child-local ZoneLink/k2-uplink
```

A call from a process in K0 that targets K2 traverses K0→K1→K2. Each
hop uses a separate KK-authenticated ComponentSession; no hop receives
authority from prior hops beyond what its own enrolled RoleBinding grants.

## ZoneLink resource

### Authoritative schema and routing interpretation

[`ADR-046-resources-zone-control.md` §3](ADR-046-resources-zone-control.md)
is the sole normative ZoneLink ResourceType schema. A ZoneLink `spec` has
exactly these six top-level fields:

1. `childZoneName`
2. `transportProviderRef`
3. `transportSettings`
4. `transportCredentials`
5. `disabled`
6. `limits`

Unknown fields are rejected. This document does not define a second routing
schema. It only adds these routing interpretations:

- `childZoneName` identifies the enclosing child Zone and must equal
  `metadata.zone`; local root has no uplink and a non-root Zone has at most one
  ZoneLink, enabled or disabled.
- `transportProviderRef` resolves to a Ready Provider in that same child Zone.
  The Provider validates `transportSettings`, while `transportCredentials`
  contains only same-Zone `Credential/<name>` refs.
- `disabled: true` closes the session, withdraws admitted route projections,
  and suppresses reconnect until re-enabled.
- `limits.maxPendingIntents`, `limits.maxActiveStreams`,
  `limits.reconnectMaxAttempts`, and `limits.reconnectWindowSecs` bound routing
  queues, streams, and reconnect activity. Transport-specific backoff remains
  Provider-internal and bounded by those limits.
- The allocator-selected parent supplies the sealed ComponentSession identity,
  route namespace, and allowed capability scope. None is an additional
  ZoneLink spec field.
- The routing wire contract has one fixed, bounded hop budget; it is not
  operator-configurable per ZoneLink.

The canonical D088 status shape is also owned by
`ADR-046-resources-zone-control.md` §3.4. Universal `ResourceStatus` fields
remain directly under `status`; ZoneLink-common observations are under
`status.resource`; optional implementation observations are under
`status.provider`. Routing consumes the universal base plus
`status.resource`, including `childZoneUid`, `connected`, connection
timestamps, revision cursors, `linkEpoch`, `pendingLocalIntents`, and
`childAuthorized`. It never introduces another status container.

Status phases:

- `Pending`: link resource created, session not yet established or child
  not yet reachable.
- `Ready`: KK session established, child Zone API probed, route
  advertisement current.
- `Degraded`: session is up but one or more conditions are impaired (e.g.,
  route renewal overdue or intent queue high-watermark reached).
- `Failed`: session cannot be established and retry policy is exhausted.
- `Unknown`: controller cannot currently prove session state.

The child-local core-controller ZoneLink handler owns status. It never writes
`Ready` until the parent allocator has authenticated the child, acknowledged
the route allocation, and successfully probed the child's `d2b.resource.v3`
service within the current link epoch. The parent keeps private
allocation/route state, not a second resource row.

## Zone tree identity and prefix naming

Each Zone has exactly one authoritative `Zone/<zone-name>` resource in
its own store. The name is the Zone's local self-name. A non-root child
authors its local `ZoneLink/<link-name>` uplink; the link name need not equal
the child's self-name, while `spec.childZoneName` must. The provisioning
parent allocator assigns the private tree edge and verifies the child self-name
during KK enrollment. The acknowledged UID is recorded as
`status.resource.childZoneUid`.

Zone tree positions are described as ordered label paths from the local
root, most-specific first, matching the `RealmPath` grammar from
`d2b-realm-core/src/realm.rs` (max 16 labels, max 255 rendered bytes,
lowercase `^[a-z][a-z0-9-]*$` labels). The routing engine uses this
parent-first storage form internally (`work/payments` for child `payments`
under parent `work`); the public v3 addressing form is the Zone resource
path (`Zone/work`, `Zone/payments` under `Zone/work`), not the DNS-target
form (`payments.work.d2b`). The routing keys inside the `ZoneRouteEngine`
use the ordered-label path model from the baseline.

```
k0                     # local root
k1.k0                  # child k1 under parent k0
k2.k1.k0               # grandchild k2
```

Each Zone runtime maintains a local `RouteTreeEngine` (adapted from
`d2b-realm-core/src/route_engine.rs`) keyed by these Zone tree paths. It
tracks:

- **Parent entries**: for each child Zone, the immediate parent path,
  optional route id, allocated capability scope, and expiry.
- **Route entries**: for each descendant Zone reachable through this
  node, the advertising Zone, next-hop child label, route id, capability
  set, and expiry.

This engine is the single authoritative source for route decisions. It
performs no I/O; callers supply all metadata.

### Tree path constraints (retained from baseline)

- Max 32 hops per decision path (`MAX_ROUTE_PATH_HOPS`).
- Max 4096 parent entries per engine (`MAX_PARENT_ENTRIES`).
- Max 4096 route entries per engine (`MAX_ROUTE_ENTRIES`).
- Max 16 Zone names per compiler-authored ancestry path
  (`MAX_REALM_LABELS`; local root counts as one).
- Sibling/parent route advertisements are rejected with
  `sibling-or-parent-route-advert`.
- Nix rejects missing/unknown/self parents, cycles, and over-depth topology
  before sealing bootstrap state.
- Conflicting `parentZone` scalar definitions fail through normal Nix module
  merging; runtime advertisements that claim a conflicting parent are rejected
  with `multi-parent`.

## Nix configuration

The Nix authoring shape mirrors the canonical ResourceSpec schema. Every
resource is declared as `d2b.zones.<zone>.resources.<name> = { type =
"<ResourceType>"; spec = { <exact-spec-fields> }; };`. The `spec` object
uses exactly the same field names, nesting, and types as the canonical
`spec` object in the JSON ResourceSpec for that ResourceType. There is no
second bespoke Nix vocabulary, no field renaming, and no additional nesting
beyond what the canonical schema has. `status` is omitted - it is
read-only and filled by the Zone runtime.

`d2b.zones.<zone>.parentZone` is the one deliberate Zone-level compiler input,
not part of that schema mirror. It is a plain Zone attrset key: required for
every non-root Zone, forbidden on `local-root`, and never emitted into a
ResourceSpec or `Zone.spec`. The compiler canonicalizes the resolved
child→parent rows, validates the complete graph, and seals them into the private
allocator-bootstrap topology. A parent change updates that private topology and
forces release/reallocation of the affected edge even though the Zone resource
bundle remains unchanged.

### Metadata derivation

The Nix emitter serializes core-derived metadata (`name`, `zone`, `apiVersion`)
plus the optional authored fields below. Management metadata
(`managedBy`, `configurationGeneration`, `uid`, etc.) is absent from the
bundle and set only by the configuration service/core - never by the emitter.

| JSON field | Source |
| --- | --- |
| `metadata.name` | `<name>` attribute key in `resources.<name>` |
| `metadata.zone` | `<zone>` attribute key in `zones.<zone>` |
| `apiVersion` | Fixed: `"resources.d2bus.org/v3"` |
| `metadata.managedBy` | Absent from bundle; set only by configuration service/core when activating the bundle |
| `metadata.configurationGeneration` | Absent from bundle; set only by configuration service/core on Create/UpdateSpec |
| `metadata.uid`, `metadata.generation`, `metadata.revision`, timestamps | Assigned by the Zone runtime on first Create; absent from the bundle |
| `metadata.ownerRef` | Optional authored field; typed `ResourceRef`; omit if none |
| `metadata.labels` | Optional authored field; key-value map; omit if none |
| `metadata.annotations` | Optional authored field; key-value map; omit if none |

### Option schema

The base Nix option type for every resource is structural:

```nix
# nixos-modules/options-zones.nix (structural base; type-specific options are
# generated by xtask gen-zone-nix-options from ResourceTypeSchema JSON)
let
  # Generated from ADR-046-resource-object-model's canonical 19-type registry
  # plus installed qualified Provider schemas. The drift test asserts that the
  # standard subset is exactly the canonical registry.
  registeredResourceTypes = import ./generated/resource-types.nix;
in {
  d2b.zones.<zone>.parentZone = mkOption {
    # No default. Required for non-root Zones and forbidden on local-root.
    type = types.strMatching "^[a-z][a-z0-9-]*$";
    description = ''
      Compiler-only parent Zone name. This is not a ResourceRef and is emitted
      only into sealed allocator bootstrap topology, never Zone.spec.
    '';
  };

  d2b.zones.<zone>.resources.<name> = {
    type = mkOption {
      type = types.enum registeredResourceTypes;
      description = "ResourceType for this resource.";
    };
    spec = mkOption {
      # Freeform at the structural level.  xtask gen-zone-nix-options emits
      # a type-specific submodule for each ResourceType so that the spec
      # subfields carry proper types, defaults, and docs from the committed
      # ResourceTypeSchema JSON.  Build-time validation then compares the
      # canonical rendered JSON against the same schema.
      type = types.attrs;
      default = {};
      description = ''
        Spec fields for this ResourceType.  Field names, types, and defaults
        must match the canonical ResourceTypeSchema for `type`.  Secrets must
        appear only as Credential/<name> refs; no inline key material.
      '';
    };
    # Optional authored metadata sub-fields.
    # managedBy, configurationGeneration, uid, generation, revision,
    # and timestamps are set only by the configuration service/core;
    # they must not appear in the authored option tree.
    metadata = {
      ownerRef = mkOption {
        type = types.nullOr types.attrs;
        default = null;
        description = "Optional ResourceRef of the owning resource, if any.";
      };
      labels = mkOption {
        type = types.attrsOf types.str;
        default = {};
        description = "Optional key-value label map.";
      };
      annotations = mkOption {
        type = types.attrsOf types.str;
        default = {};
        description = "Optional key-value annotation map.";
      };
    };
  };
}
```

`xtask gen-zone-nix-options` derives `generated/resource-types.nix` from the
same committed ResourceTypeSchema catalog used by API and bundle validation.
Its drift test compares the standard subset byte-for-byte with the canonical
19-type registry and rejects an omission, addition, duplicate, or reordered
entry; qualified Provider types are appended only from installed signed
schemas.

`xtask gen-zone-nix-options` reads `docs/reference/schemas/v3/<Type>.schema.json`
and emits a generated `nixos-modules/generated/options-zones-<type>.nix` for each
ResourceType, overlaying typed submodule options onto `spec`. These generated files
are committed and kept in sync by `make test-drift`. Field-level type errors (wrong
enum value, out-of-range integer, malformed ResourceRef, etc.) are therefore caught
at `nix eval` time via the generated option type, not by explicit assertions.

#### Generated ZoneLink spec options (illustrative excerpt)

```nix
# nixos-modules/generated/options-zones-ZoneLink.nix  (generated; do not hand-edit)
{
  d2b.zones.<zone>.resources.<name>.spec = {
    childZoneName = mkOption {
      type = types.strMatching "^[a-z][a-z0-9-]*$";
      description = "Self-reported name of the child Zone.  Verified during KK enrollment.";
    };
    transportProviderRef = mkOption {
      # Required; no default.  Must always be explicitly declared.
      type = types.strMatching "^Provider/[a-z][a-z0-9-]*$";
      description = "Provider/<name> resource that owns the transport session for this link.  Always explicit; no default or inference.";
    };
    transportSettings = mkOption {
      # Freeform; validated at build time against the transport Provider's
      # transportSettingsSchema.  No socketPath, hostPath, password, token,
      # or key top-level keys permitted.
      type = types.attrs;
      default = {};
    };
    transportCredentials = mkOption {
      type = types.listOf (types.strMatching "^Credential/[a-z][a-z0-9-]*$");
      default = [];
      description = "Same-Zone Credential refs resolved for ComponentSession establishment.";
    };
    disabled = mkOption {
      type = types.bool;
      default = false;
    };
    limits = {
      maxPendingIntents = mkOption {
        type = types.ints.between 0 1024;
        default = 256;
      };
      maxActiveStreams = mkOption {
        type = types.ints.between 1 128;
        default = 32;
      };
      reconnectMaxAttempts = mkOption {
        type = types.ints.positive;
        default = 10;
      };
      reconnectWindowSecs = mkOption {
        type = types.ints.positive;
        default = 300;
      };
    };
  };
}
```

This is an exact mirror of the six-field schema owned by
`ADR-046-resources-zone-control.md` §3. The generator rejects any additional
ZoneLink field rather than extending this excerpt.

### Eval-time assertions

These invariants require cross-resource context and cannot be expressed as
generated per-field option types. They live in `nixos-modules/assertions.nix`.
Field-level type, bounds, and enum assertions are handled by the generated
option types from `xtask gen-zone-nix-options` and are not repeated here.

| Assertion | Error message |
| --- | --- |
| `<zone>` key matches `^[a-z][a-z0-9-]*$` | `zones: zone key must match ^[a-z][a-z0-9-]*$` |
| `<zone>` key not `sys-*` or `launcher` | `zones: zone key uses reserved prefix or exact name` |
| `parentZone` omitted on `local-root` and defined once on every other Zone | `zones.<zone>: parentZone is required for non-root Zones and forbidden on local-root` |
| `parentZone` resolves to a declared Zone and does not equal `<zone>` | `zones.<zone>.parentZone: parent must exist and differ from child` |
| Complete `parentZone` graph is acyclic and each ancestry path contains at most 16 Zone names | `zones: parentZone topology has a cycle or exceeds depth 16` |
| `<name>` key matches `^[a-z][a-z0-9-]*$` | `zones.<zone>.resources: resource key must match ^[a-z][a-z0-9-]*$` |
| No operator-authored `type = "Zone"` under `resources` | `zones.<zone>.resources.<name>: Zone self-resource is runtime-created` |
| For `type = "ZoneLink"`: `spec.childZoneName` equals `<zone>`; at most one uplink resource exists in a non-root Zone; local root has none | `zones.<zone>: ZoneLink must be the sole child-local uplink and childZoneName must equal its Zone` |
| For `type = "ZoneLink"`: `spec.transportProviderRef` resolves to a declared `Provider` resource in the same `<zone>` | `zones.<zone>.resources.<name>: transportProviderRef does not resolve to a declared Provider resource` |
| Total `d2b.zones` keys ≤ 64 | `zones: zone count exceeds host limit of 64` |
| `resources` count per `<zone>` ≤ 1024 | `zones.<zone>.resources: resource count exceeds zone limit of 1024` |
| `spec.transportSettings` for `type = "ZoneLink"` has no top-level key named `socketPath`, `hostPath`, `password`, `token`, or `key` | `zones.<zone>.resources.<name>: transportSettings must not contain host paths, socket paths, or secret material` |

### Example configurations

**K0 with local Unix-transport child K1 (K0 = Host, K1 = Guest)**:

```nix
# K0 is the distinguished local-root Zone. parentZone is forbidden here.
# The runtime creates Zone/local-root with spec = {}; it is not bundle-authored.
d2b.zones.local-root = {};

# Unix transport Provider is local to child K1 because the ZoneLink and all of
# its refs resolve in K1.
d2b.zones.k1.parentZone = "local-root";

d2b.zones.k1.resources.transport-unix = {
  type = "Provider";
  spec = {
    kind        = "transport-unix";
    description = "Allocator-issued Unix socket transport for local child zones";
  };
};

# K1's local uplink supplies transport/route state for the allocator selected
# by k1.parentZone.
d2b.zones.k1.resources.k1-uplink = {
  type = "ZoneLink";
  spec = {
    childZoneName        = "k1";
    transportProviderRef = "Provider/transport-unix";
    transportSettings    = {}; # allocator-issued FD; no path config
    transportCredentials = [];
    disabled             = false;
    limits = {
      maxPendingIntents    = 256;
      maxActiveStreams     = 32;
      reconnectMaxAttempts = 10;
      reconnectWindowSecs  = 300;
    };
  };
};
```

**K2 child-local uplink to K1 via Azure Relay transport**:

```nix
# Compiler-only topology makes K1 the one allocator owner for K2. The runtime
# creates Zone/k2 with spec = {}; parentZone is not emitted there.
d2b.zones.k2.parentZone = "k1";

# Azure Relay transport Provider is local to K2.
d2b.zones.k2.resources.transport-azure-relay = {
  type = "Provider";
  spec = {
    kind        = "transport-azure-relay";
    description = "Azure Relay transport for K1→K2 link";
  };
};

# Credential ref for the relay SAS token - no inline secret.
d2b.zones.k2.resources.relay-sas-k2 = {
  type = "Credential";
  spec = {
    kind        = "opaque";
    description = "Azure Relay SAS token for d2b-k2 relay";
    # The secret itself is stored in a separate Credential store; only the ref lives here
  };
};

# K2's local uplink supplies transport/route state for the allocator selected
# by k2.parentZone.
d2b.zones.k2.resources.k2-uplink = {
  type = "ZoneLink";
  spec = {
    childZoneName        = "k2";
    transportProviderRef = "Provider/transport-azure-relay";
    transportSettings = {
      relayNamespaceId = "relns-d2b-prod";
      relayEntityId    = "hc-d2b-k2";
    };
    transportCredentials = [ "Credential/relay-sas-k2" ];
    disabled = false;
    limits = {
      maxPendingIntents    = 128;
      maxActiveStreams     = 32;
      reconnectMaxAttempts = 10;
      reconnectWindowSecs  = 300;
    };
  };
};
```

The relay SAS token is referenced only through
`spec.transportCredentials`; `transportSettings` contains non-secret Provider
configuration. The build emitter rejects settings annotated `"secret": true`
and resolves every credential ref in the same child Zone.

### Build-time validation

The Nix build phase runs `xtask gen-zone-resources` which:

1. **Topology validation and compilation**: resolves every non-root
   `parentZone` against declared Zones; rejects a value on `local-root`,
   missing, unknown, self, conflicting, cyclic, or over-16-name ancestry; and
   canonicalizes the child→parent map into sealed private allocator-bootstrap
   topology. `parentZone` is never copied into the resource bundle.

2. **Schema validation**: validates every emitted resource object against
   the committed JSON Schema files:
   - `docs/reference/schemas/v3/Zone.schema.json`
   - `docs/reference/schemas/v3/ZoneLink.schema.json`
   - `docs/reference/schemas/v3/<Type>.schema.json` for any other declared type
   The `make test-drift` gate enforces `xtask gen-zone-schemas && git diff
   --exit-code` so schema, Rust types, and generated Nix option modules stay
   in sync. A separate `xtask gen-zone-nix-options && git diff --exit-code`
   step ensures the generated Nix option modules match the current schema.

3. **Provider binding validation**: for each `ZoneLink`, the emitter fetches
   the transport Provider's `transportSettingsSchema` (signed, committed under
   `docs/reference/schemas/v3/providers/<provider-name>.transport-binding.json`)
   and validates `spec.transportSettings` against it. Unknown keys are rejected.
   Fields annotated `"secret": true` are rejected; those annotated
   `"credentialRef": true` must be `"Credential/<name>"` strings.

4. **Ref resolution**: all `*Ref` and `*ProviderRef` values, including every
   `transportCredentials` entry, are resolved
   against resources declared in the same `d2b.zones.<zone>.resources` and
   fail the build if unresolvable.

5. **Conflict check**: duplicate `(type, zone, name)` tuples across the
   entire emitted bundle fail the build.

### Canonical ResourceSpec JSON shapes

The `spec` object in Nix and in the emitted JSON are identical in field
names, nesting, and defaults. The emitter does not rename or restructure
any spec field. Resources are sorted by `(type, zone, name)` for
determinism before the integrity digest is computed.

**Zone resource** - runtime-created from the declaration
`d2b.zones.local-root = {};`, not emitted in the resource bundle:

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "Zone",
  "metadata": {
    "name": "local-root",
    "zone": "local-root"
  },
  "spec": {}
}
```

The runtime gives this self-resource core-populated fields such as
`metadata.managedBy`, `metadata.uid`, and `metadata.generation`.
`metadata.configurationGeneration` is absent because the resource is
controller-created. Compiler-only `parentZone` is absent from both this object
and the Nix-rendered resource bundle.

**ZoneLink resource** - emitted from
`d2b.zones.k1.resources.k1-uplink = { type = "ZoneLink"; spec = { ... }; };`:

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "ZoneLink",
  "metadata": {
    "name": "k1-uplink",
    "zone": "k1"
  },
  "spec": {
    "childZoneName": "k1",
    "disabled": false,
    "limits": {
      "maxActiveStreams": 32,
      "maxPendingIntents": 256,
      "reconnectMaxAttempts": 10,
      "reconnectWindowSecs": 300
    },
    "transportCredentials": [],
    "transportProviderRef": "Provider/transport-unix",
    "transportSettings": {}
  }
}
```

The canonical form sorts all object keys lexicographically. Array ordering
depends on the array's schema classification: order-significant arrays
(e.g. `transportCredentials`, named-stream lists) are preserved as authored;
schema-declared set-like arrays (e.g. `resourceTypes` in capability specs) are
sorted lexicographically. All fields with defaults are always emitted; no key
omission. The `status` object is absent from the emitted bundle entirely.

### Zone resource bundle format

The build installs `/etc/d2b/zones/<zone>/resource-bundle.json` (root:d2bd
0640). For child K1, the local bundle contains its Provider and uplink. The
controller-created Zone self-resource is not emitted:

```json
{
  "schemaVersion": 1,
  "generationId": "<sha256-hex-of-canonical-sorted-resources>",
  "resourceCount": 2,
  "resources": [
    { "type": "Provider", "metadata": { "name": "transport-unix", "zone": "k1" }, "...": "..." },
    { "type": "ZoneLink", "metadata": { "name": "k1-uplink",      "zone": "k1" }, "...": "..." }
  ],
  "integrity": "sha256-<base64url>"
}
```

Bundle rules:

- `generationId` is the SHA-256 (lower hex) of the UTF-8 bytes of the
  canonical sorted `resources` array JSON (not the envelope fields).
  Two identical Nix configurations always produce the same `generationId`
  regardless of host name or wall-clock time.
- `integrity` is computed by serializing the bundle with `integrity` set
  to the all-zeros placeholder `"sha256-"`, computing SHA-256 of the
  result, encoding as base64url without padding, then replacing the
  placeholder with the final value.
- The bundle is the single source of truth for what Nix owns. The Zone
  runtime detects changes by comparing `generationId` on startup and on
  SIGHUP.
- Transport Provider binding schemas referenced in build validation are
  committed separately under `docs/reference/schemas/v3/providers/` and
  are not inlined in the bundle.

## Configuration ownership and cleanup contract

### Management classification

The Zone runtime tracks cleanup authority through two canonical metadata
fields set by the emitter or runtime, never authored in the Nix option tree:

| Field | Set by | Meaning |
| --- | --- | --- |
| `metadata.managedBy` | Configuration service/core; set when activating the validated bundle | Identifies the management agent that owns the resource's lifecycle. Core-defined value; absent from bundle JSON. |
| `metadata.configurationGeneration` | Configuration service/core; set only by core on Create/UpdateSpec | Absent from bundle JSON; absent on controller-created or API-created resources. |

Cleanup requires **both** conditions: `managedBy` must equal the configuration
service's value **and** `configurationGeneration` must match a known bundle
generation. A resource whose `managedBy` is `controller` or `api` is never
touched by the diff even if it coincidentally shares a name with a bundle
resource; core fails closed on any attempt to seize such a resource without
an explicit mutation.

### New generation activation flow

When a new bundle is installed (e.g. after `nixos-rebuild switch`):

```
1. Zone runtime reads `/etc/d2b/zones/<zone>/resource-bundle.json` and verifies integrity.
2. Computes diff against the active generation by generationId.
   If generationId is unchanged, no action required.

3. For each resource in the new bundle:
   a. Absent from store → queue Create intent (sets configurationGeneration).
   b. Present, spec changed → queue UpdateSpec intent (updates configurationGeneration).
   c. Present, spec unchanged → no-op (configurationGeneration refreshed in place).

4. For each resource in the prior bundle whose configurationGeneration matches
   the prior generationId and that is absent from the new bundle:
   → queue Delete intent (asynchronous, non-blocking).
   Core sets `deletionRequestedAt` on the resource immediately and adds
   a Pending condition to signal cleanup is in progress.

5. Activation completes synchronously once all intents are queued.
   The Zone runtime begins applying intents asynchronously.

6. Zone Resource phase after diff:
   - Pending  → while any Create or UpdateSpec intent is in-flight.
   - Degraded → all Creates/Updates done; one or more Delete intents pending.
   - Ready    → all intents complete; no pending cleanup.

7. Prior generation bundles are retained in a capped ring
   (default 3, range 1..16, no TTL) until explicitly pruned or rolled back.
   See "Prior generation retention and rollback" below.
```

### Degraded / pending-cleanup status

New generation activation is **non-blocking**: the Zone runtime begins
serving the new configuration immediately. Clients accessing a resource
with `deletionRequestedAt` set receive `resource-pending-cleanup` on
Create/UpdateSpec calls; Get/List/Watch continue to succeed until
deletion completes and core atomically removes the row and index.

Status fields on the active generation resource in the runtime store:

```yaml
status:
  generationId: <new-id>
  phase: Degraded          # Pending while creates/updates run; Ready when clean
  conditions:
    - type: ConfigApplied
      status: "True"
      reason: all-creates-and-updates-done
    - type: CleanupComplete
      status: "False"
      reason: pending-delete-intents
      message: "2 resources pending deletion"
  pendingCleanup:
    - { type: ZoneLink, name: old-link,     zone: k1, deletionRequestedAt: 2026-07-22T21:00:00Z }
    - { type: Zone,     name: removed-zone, zone: removed-zone, deletionRequestedAt: 2026-07-22T21:00:00Z }
  priorConfigurationGeneration: <prior-id>
  lastGenerationChange: 2026-07-22T21:00:00Z
```

Per-resource cleanup tracking fields (set by core on the pending-delete resource):

| Field | Type | Meaning |
| --- | --- | --- |
| `deletionRequestedAt` | RFC3339 | When core queued the Delete intent; presence signals pending deletion |
| `cleanupConfigGeneration` | string | The `configurationGeneration` that triggered the Delete intent |
| `cleanupError` | string? | Last error from the Delete attempt, if any |
| `cleanupAttempt` | u32 | Attempt count |

### Delete lifecycle for removed resources

A resource absent from the new bundle and matching the prior bundle's
`configurationGeneration` receives an async Delete intent:

1. **Finalizer drain**: the runtime checks for registered finalizers. If
   any finalizer is present, core sets `deletionRequestedAt` on the resource
   (adding a Pending condition) and notifies each finalizer holder to
   release it. Deletion proceeds only after all finalizers clear.

2. **Controller-child cascade**: before completing deletion of a parent
   resource (Zone or ZoneLink), the runtime notifies the owning controller.
   The ZoneLink controller closes its session, withdraws its advertisements,
   and deletes its own route entries. Zone controllers signal their Process
   and EphemeralProcess children for graceful stop. These are
   controller-initiated deletions, not bundle-diff deletions.

3. **Atomic store commit**: after all controller-created children acknowledge
   teardown, core executes one store transaction: writes the `Deleted`
   revision/change event and removes the resource row and all index entries.
   Once the transaction commits, the authoritative audit record
   (`zone-resource-cleanup`) is appended from the committed revision with
   dedup/exactly-once recovery; the audit append is not part of the store
   transaction itself.

4. **Failure**: if a Delete intent exhausts retries, the generation status
   is `Failed` with a Degraded condition `CleanupFailed: resource-delete-failed`.
   Prior generation bundles are retained until the failure is resolved.

### Boundary invariants

- A generation diff **never** deletes a resource that has no
  `configurationGeneration` field (controller-created or API-created).
- A generation diff **never** deletes a resource whose `managedBy` does not
  equal the configuration service's value, even if `configurationGeneration`
  appears to match (collision guard - core fails closed without mutation).
- A generation diff **never** deletes a resource whose
  `configurationGeneration` does not match the prior bundle's `generationId`
  (guards against races between concurrent generation switches).
- A generation diff **never** deletes a resource that still has a live
  controller-created child that has not acknowledged teardown.
- Dynamic children (route entries, ephemeral Processes) are deleted by their
  owning controller when the parent is deleted - not by the generation diff.

### Prior generation retention and rollback

The Zone runtime keeps a capped ring of prior generation bundles on disk
under `/var/lib/d2b/zones/<zone>/configuration/prior/`:

```
/etc/d2b/zones/<zone>/resource-bundle.json                              ← active input bundle
/var/lib/d2b/zones/<zone>/configuration/prior/<gen-id-1>.json
/var/lib/d2b/zones/<zone>/configuration/prior/<gen-id-2>.json
/var/lib/d2b/zones/<zone>/configuration/prior/<gen-id-3>.json           ← up to retentionCount
```

| Parameter | Default | Range |
| --- | --- | --- |
| `retentionCount` | 3 | 1..16 |
| TTL | none | - |

Retention is count-only: the oldest bundle is pruned when a new generation
is added and the count would exceed `retentionCount`. No time-based expiry.

- **Rollback**: writing a retained bundle back to
  `/etc/d2b/zones/<zone>/resource-bundle.json` (e.g. via `nixos-rebuild switch`
  to a previous NixOS generation) triggers a reverse diff. Resources with
  `deletionRequestedAt` set have the field cleared and their Pending
  condition removed (Delete intent cancelled if not yet executed);
  resources added in the now-superseded generation receive Delete intents.
- **Pruning on cleanup failure**: prior bundles are never forcibly pruned
  while a Delete intent originating from their configurationGeneration is
  still in flight.

### Cleanup audit events

| Audit event kind | Fields | Trigger |
| --- | --- | --- |
| `zone-generation-activate` | `generationId`, `priorGenerationId`, `resourceCount`, `creates`, `updates`, `deletes` | New bundle processed |
| `zone-resource-cleanup` | `type`, `name`, `zone`, `configurationGeneration`, `durationMs` | Appended from committed `Deleted` revision with dedup/exactly-once recovery after the store transaction that removes row/index commits |
| `zone-resource-cleanup-failed` | `type`, `name`, `zone`, `configurationGeneration`, `reason`, `attempt` | Delete intent failure |
| `zone-generation-ready` | `generationId`, `durationMs` | Generation reaches Ready |
| `zone-generation-failed` | `generationId`, `reason` | Generation reaches Failed |
| `zone-generation-rollback` | `generationId`, `priorGenerationId` | Rollback initiated |

All audit events are emitted to the Zone runtime's audit log under category
`zone-config` and carried in OTEL spans under `d2b.zone.config.generation`.

### Eval and build tests

Required before ADR046-routing-011 through ADR046-routing-013 are complete:

| Test ID | Kind | What it proves |
| --- | --- | --- |
| `nix-unit: zone-name-regex` | nix-unit eval | Zone name regex and reserved-name assertions fire |
| `nix-unit: zone-parent-required-root-forbidden` | nix-unit eval | Every non-root Zone requires `parentZone`; `local-root` rejects it |
| `nix-unit: zone-parent-resolves` | nix-unit eval | Unknown and self-valued `parentZone` settings reject |
| `nix-unit: zone-parent-one-parent` | nix-unit eval | Conflicting scalar definitions reject through normal Nix module merging |
| `nix-unit: zone-parent-cycle` | nix-unit eval | Two-node and longer `parentZone` cycles reject before bootstrap publication |
| `nix-unit: zone-parent-depth` | nix-unit eval | Sixteen-name ancestry succeeds and seventeen-name ancestry rejects |
| `nix-unit: zone-link-credential-ref` | nix-unit eval | Malformed or cross-Zone `transportCredentials` refs rejected |
| `nix-unit: zone-link-child-name` | nix-unit eval | `childZoneName` unequal to enclosing child Zone rejected |
| `nix-unit: zone-link-one-uplink` | nix-unit eval | Second uplink (even disabled) and any local-root uplink rejected |
| `nix-unit: zone-link-closed-spec` | nix-unit eval | Any ZoneLink top-level spec field outside the canonical six is rejected |
| `nix-unit: zone-link-limits` | nix-unit eval | Queue, stream, reconnect-attempt, and reconnect-window bounds enforced |
| `nix-unit: transport-binding-secret-key` | nix-unit eval | Binding with `key =` rejected at eval |
| `drift: zone-resource-schema` | `make test-drift` | `xtask gen-zone-schemas && git diff --exit-code` passes |
| `build: zone-bundle-deterministic` | flake check | Two identical configs produce identical `generationId` |
| `build: transport-binding-unknown-field` | flake check | Unknown Provider binding key fails build |
| `build: allocator-capability-scope` | flake check | Route capability scope wider than the sealed parent allocation fails build |
| `build: missing-transport-provider` | flake check | Unresolvable `transportProviderRef` fails build |
| `build: parent-topology-sealed` | flake check | Valid sorted `parentZone` rows compile only into the sealed allocator bootstrap input and never into `Zone.spec` or resource bundles |
| `host-integration: cleanup-removed-zonelink` | NixOS test | Switch removes ZoneLink; assert `deletionRequestedAt` set; store transaction commits `Deleted` revision and removes row/index; audit record appended from committed revision with exactly-once recovery; generation reaches Ready; dynamic route entries deleted by ZoneLink controller teardown, not by generation diff |
| `host-integration: rollback-restores-zonelink` | NixOS test | After cleanup switch, rollback re-activates ZoneLink; generation diff reverses |
| `host-integration: dynamic-child-not-deleted` | NixOS test | Parent allocator/route entries are NOT diff-owned resources; removing the child-local ZoneLink invokes controller teardown and allocation release |
| `host-integration: zonelink-no-reciprocal-row` | NixOS test | Activating a child-local uplink creates no ZoneLink row in the parent store |

## Authenticated advertisements, withdrawal, and renewal

The child-local ZoneLink handler signs each advertisement with its enrolled KK
key. The parent validates the signature before admitting it to the parent's
`RouteTreeEngine`; this creates authenticated in-memory route projection state,
not a parent-store ZoneLink row. The v3 contract adapts the existing
`RouteAdvertisement` / `RouteNamespaceAllocation` types.

### Advertisement envelope (v3 adaptation)

```text
ZoneLinkRouteAdvertisement {
  advertisingZone: ZonePath           // self-path of the advertising Zone
  treeEdge: { parent: ZonePath, child: ZonePath }
  controllerGeneration: <opaque>      // bound to the child controller lease
  routes: [
    { descendant: ZonePath, nextHopChild: ZoneLabelId, routeId: <opaque>,
      capabilities: CapabilitySet }   // narrowed by allocator policy
  ]                                   // 1–64 routes
  issuedAtUnixSeconds: u64
  expiresAtUnixSeconds: u64           // > issuedAt; max 7200 s
  signature: {
    algorithm: "ed25519-blake3"
    keyRole: zone-controller-routing
    signingKeyFingerprint: <sha256-hex>
    signatureRef: <detached-sig-ref>  // opaque; no key bytes
  }
}
```

Admission rules (adapted from `RouteTreeEngine::admit_advertisement`):

1. `treeEdge.child == advertisingZone`.
2. Routes non-empty; at most 64 routes.
3. `expiresAt > issuedAt`; current time < `expiresAt`; current time >=
   `issuedAt`.
4. `treeEdge.parent` must equal the local root or have an existing
   non-expired parent entry.
5. Replay: signature ref + advertising zone + controller generation +
   issuedAt must not duplicate a non-expired replay-window entry.
6. Namespace check: `treeEdge` and `controllerGeneration` must match the
   allocated `ZoneLinkNamespaceAllocation`; route count ≤ `maxRoutes`.
7. Each route's `descendant` must be a strict descendant of
   `advertisingZone`; next-hop label must be the immediate child of
   `advertisingZone` toward `descendant`.
8. Each route's `capabilities` must be a subset of `allowedCapabilities`
   in the private `ZoneLinkNamespaceAllocation`.
9. Capacity check: projected physical entries after admission must not
   exceed `MAX_PARENT_ENTRIES` / `MAX_ROUTE_ENTRIES`; if admission would
   overflow, prune expired entries first; fail closed with
   `queue-full-drop-new` if pruning is insufficient.

Admitted advertisements update parent and route entries atomically. A
replay-window key is recorded. Admission is a pure in-memory operation;
the parent Zone store is not mutated.

### Withdrawal

A withdrawal message removes one or more specific `routeId` values
from the engine. It must be:

- signed by the same controller generation as the advertisement it
  withdraws;
- issued at a time ≥ the advertisement's `issuedAt`;
- carries the exact `routeId` set to remove.

The engine removes matching route entries immediately and emits a
`route-withdrawn` audit event. Partial withdrawal is allowed; routes
not named in the withdrawal remain. A withdrawal for an already-expired
or unknown `routeId` is silently accepted (idempotent).

### Renewal

Advertisements expire. The child's ZoneLink controller issues a renewal
advertisement before expiry using the bounded protocol scheduler. Renewal
timing is internal routing behavior, not a ZoneLink spec field. A
renewal carries:

- a new `issuedAt` / `expiresAt` window;
- a new `signatureRef` (distinct from prior);
- optionally updated `capabilities` (may narrow but not widen beyond the
  parent's private allocation).

The engine admits the renewal exactly as a fresh advertisement, replacing
the old entry for each `routeId` named in the renewal. The prior replay
key is superseded and removed. If the renewal arrives after expiry, the
route is treated as a new advertisement.

### Namespace allocation

A parent allocates a `ZoneLinkNamespaceAllocation` when the child-local
ZoneLink requests activation and whenever the parent allocator's private
route policy changes:

```text
ZoneLinkNamespaceAllocation {
  treeEdge: { parent: ZonePath, child: ZonePath }
  allocatedToGeneration: <controller-generation>
  allowedPrefixes: [ZonePath]    // child zone or descendants; 1–16
  maxRoutes: u32                 // 1–64
  allowedCapabilities: CapabilitySet
}
```

The child controller must sign advertisements that exactly match the
allocated edge, generation, allowed prefixes, and capability scope.
Allocation changes (e.g. capability narrowing) require the child to issue
a new advertisement under the new generation.

The `ZoneLinkNamespaceAllocation` above **is** the explicit ZoneLink range
capacity/quota (D097 hardware-audit finding): `allowedPrefixes` (1–16) and
`maxRoutes` (1–64) are the bounded per-edge capacity; a child exceeding either
bound is rejected, so the parent allocator's route namespace cannot be
exhausted by one child edge.

### Global vsock CID and fixed-port authority (D097)

vsock CID allocation is a **Host-global** authority (keyed by `(Host, …)` in the
core authority index): every CID is globally unique across all Zones on the host
and a CID never crosses a Zone boundary. The historical hardcoded host-CID
assumption (`CID = 2`) is migrated to this global allocation authority - no
component assumes a fixed CID; the allocator assigns and the transport resolves
the CID under authorization, never as a public locator. Fixed listener ports
(vsock/Unix/TCP) are modeled as `Endpoint` resources with an `exactly-one`-per
Host-global port authority (see `ADR-046-resources-zone-control` §8B.3); a second
binder of the same fixed port is a `duplicateConflict`.

## ResourceExport advertisement and ResourceImport routing (D096)

`ResourceExport` and `ResourceImport` (defined in
[`ADR-046-resources-zone-control.md` §8A](ADR-046-resources-zone-control.md))
use the mechanisms above unchanged; this section states how they compose. No new
transport, cross-Zone reference, or FD-forwarding path is introduced.

- **Advertisement.** The owner Zone's core export/import controller advertises a
  `ResourceExport` to the exact `consumerZonePolicy` selector over the existing
  authenticated advertisement envelope, carrying only the bounded `exportKey`,
  qualified semantic/provider-neutral Service type, signed projection-schema
  and factory fingerprints, the closed operation set, arbitration, and
  capability ceiling - never Provider/adapter identity, `spec.provider`, the local
  owner-Service `resourceRef`, its Device/Endpoint/backend refs, a path, address,
  secret, or bytes. Withdrawal and renewal reuse the withdrawal/renewal
  machinery; export removal or ceiling narrowing issues a new generation.
- **Import matching.** The consumer Zone's `ResourceImport` names only its local
  `zoneLinkRef` plus `exportKey`; core matches `exportKey` +
  `expectedServiceType` + the projection-schema/factory fingerprints against the
  advertisement and local installed Provider factory. Missing metadata, a
  mismatch, unauthorized Zone, or absent advertisement fails closed. This
  preserves the "No cross-Zone resource references" invariant. The consumer's
  local `providerRef` independently selects a conformant implementation; route
  matching preserves the semantic Service type exactly and never copies the
  owner's implementation extension. Core creates the projection with semantic
  base/import fields only and rejects `spec.provider`; routing derives from the
  signed local Provider descriptor, `providerRef`, and ResourceImport record,
  while implementation observations may appear only in `status.provider`.
- **Capability/RBAC ceiling.** Every hop applies the ceiling-propagation and
  RBAC-narrowing rules; `requestedCapabilities` is clamped to the export
  capability ceiling and to the ZoneLink allocation. No import can exceed the
  advertised operation set.
- **Payload carriage.** Shared bytes flow only over the bounded encrypted named
  streams described in "Named streams over ZoneLink", with a per-import session
  generation, credits/backpressure, cancel, deadline, and idempotency;
  intermediate controllers see ciphertext. The "No FD, credential, or host path
  forwarding" invariant holds - no device FD, socket, or token crosses a Zone.
- **Projection and lifecycle.** The export target is always the qualified owner
  `*Service`, never a Device, Endpoint, or `*Binding`. Core owns exactly one
  same-qualified-type local projection Service per import
  (`ownerRef: ResourceImport/<name>`). Operator/Nix-authored same-Zone Bindings
  reference that Service and a consuming Guest/User/Zone; their Provider
  controller owns Process/Endpoint children. Binding spec is desired intent
  only; all observations belong in status. Import never creates or exports a
  Binding. Link failure, revocation, or withdrawal degrades the projection Service;
  reconnect revalidates the remote generation and both fingerprints. D091
  currency propagates owner Service → export → import → projection Service →
  Binding → children.
- **Single authority (D097).** The exported backing has exactly one authority
  owner in the owner Zone (its signed `AuthorityDescriptor`, tracked in that
  Zone's core authority index). Cross-Zone import never creates a second
  authority or duplicate open in the consumer Zone - the projection Service is
  an explicitly non-authoritative route to the owner. `exportability` and signed
  projection-factory presence gate whether a Service may be shared at all
  (`forbidden` authorities such as the audit chain, broker, and resource store
  are never advertised over a ZoneLink).

High-churn leases, sessions, ceremonies, transfers, named streams, and stream
handles remain controller/session-internal records; routing never promotes them
to resources or advertises them.

The frozen routeable pairs are `audio.d2bus.org.AudioService` +
`audio.d2bus.org.AudioBinding`, `security-key.d2bus.org.SecurityKeyService` +
`security-key.d2bus.org.SecurityKeyBinding`,
`telemetry.d2bus.org.TelemetryService` +
`telemetry.d2bus.org.TelemetryBinding`, and the policy-gated
`usb.d2bus.org.UsbService` + `usb.d2bus.org.UsbBinding`. PipeWire, CTAPHID,
OTEL, and USBIP are implementation details and never route keys, base fields,
conditions, errors, or advertised status.

## Nearest-common-ancestor (NCA) algorithm

The route decision algorithm is a direct adaptation of
`RouteTreeEngine::build_path_at` (baseline
`packages/d2b-realm-core/src/route_engine.rs`). It is a pure in-memory
tree walk:

```
Input:  source Zone tree path S, target Zone tree path T, current time
Output: TreeRoutePath or RouteFailClosedReason

1. Both S and T must be known (local root, or have a non-expired
   parent entry or route entry).
2. Compute the nearest common ancestor (NCA) of S and T:
     a. Build ancestor sets for S and T by walking up the parent chain.
     b. The NCA is the deepest label position shared by both chains.
     c. Return UnknownParent if no common ancestor is found.
3. Build the upward hops from S to NCA:
     a. Follow parent entries from S, recording each hop.
     b. Detect cycles (visited set); return Loop if a cycle is found.
     c. Return UnknownParent if any parent entry is missing or expired.
4. Build the downward hops from NCA to T:
     a. Walk from NCA toward T using descendant route entries.
     b. Verify each step's parent entry; return MultiParent if ambiguous.
     c. Return UnknownParent if any step is missing or expired.
5. Return the concatenated hop list with NCA recorded.
```

After a path is built, the required `CapabilitySet` for the requested
operation kind is checked against the target Zone's advertised
capabilities:

- If the required capability is absent: `MissingCapability`.
- If the target Zone has no route entry: `UnknownParent`.

The algorithm returns a `ZoneRoutePath`:

```text
ZoneRoutePath {
  sourceZone: ZonePath
  targetZone: ZonePath
  nearestCommonAncestor: ZonePath
  hops: [ ZoneRouteHop ]  // max 32
}

ZoneRouteHop {
  from: ZonePath
  to:   ZonePath
  edge: { parent: ZonePath, child: ZonePath }
  direction: up-to-parent | down-to-child
  routeId: <opaque>?
}
```

The result is immutable route metadata. It carries no transport socket,
relay endpoint, credential, or host path. d2b-bus uses the hop list to
compose the sequence of ComponentSession calls that forward the request.

## Capability and RBAC narrowing

### Allocated capability propagation

The parent allocator's private `ZoneLinkNamespaceAllocation` declares
`allowedCapabilities`, the maximum `CapabilitySet` the parent will route to
the child. It is not part of the ZoneLink ResourceSpec. The scope is enforced
at two points:

1. **Advertisement admission**: the child's advertised `capabilities` for
   each route must be a subset of `allowedCapabilities` in the namespace
   allocation (`CapabilitySet::is_subset_of`). A route advertising
   capabilities beyond the allocation is rejected with
   `namespace-violation`.
2. **Bus route decision**: the `required_capability` for the operation is
   checked against the route's advertised capabilities at the target Zone.
   A missing capability returns `MissingCapability`.

A parent cannot route a call to a child for an operation type the child
did not advertise (even if the parent itself holds that capability). This
makes capability propagation opt-in downward.

### RBAC narrowing

When the parent d2b-bus forwards a resource API call to the child Zone,
the call is authorized using the child's own native RBAC engine with a
subject mapped from the ZoneLink's enrolled identity:

```
child-local subject = child RoleBinding that:
  - matches the parent Zone's enrolled link principal with a trusted exact
    externalPrincipalSelector
  - grants only the verbs/resourceTypes/names declared in the binding
  - has a capability scope no wider than the allocator-issued scope
```

The parent cannot self-assert a subject, verb, or resource name in the
child. The child's ResourceAPI authorization evaluates
`AuthenticatedSubjectContext.subjectRef` which is set from the KK-enrolled
parent identity, not from the forwarded request payload.

Authorization at each hop is independent:

- The source Zone authorizes the caller's target verb before selecting a route.
- Each intermediate Zone authenticates the inbound adjacent-Zone transport
  subject, then independently authorizes both `relay` and the immutable target
  verb using its own RBAC.
- The target child authenticates its inbound adjacent-Zone subject and
  authorizes the final target verb using child-local RBAC.

A child that has not bound a matching parent RoleBinding refuses the call
with `authorization-denied`. A forwarding Zone without the separate `relay`
grant returns `relay-denied`. `relay` is core-generated/ZoneLink-scoped and
permits only one route-selected next hop; it grants no CRUD, identity mapping,
capability widening, attachment/credential access, or local lifecycle
authority. No ambient cross-Zone authority exists.

### Capability floor at local root

The local Zone root (`Zone/<zone-name>`) advertises its own capabilities
to parent Zones. These reflect the installed Provider catalog, active
Hosts/Guests, and runtime state. A capability absent from the local root's
advertised set is not reachable from any ancestor Zone.

## Resource API forwarding

### Forwarding model

A parent ResourceClient targeting a child Zone routes through d2b-bus:

```
Parent ResourceClient
  -> parent d2b-bus (route decision: local or allocator-bound child edge)
     |-- local Zone: direct to local d2b.resource.v3
     |-- child Zone: ComponentSession represented by child's local ZoneLink
           -> intermediate Zone d2b-bus (hop relay)
              -> child d2b.resource.v3
```

At each intermediate hop, d2b-bus:

1. Verifies the inbound ComponentSession's `AuthenticatedSubjectContext`.
2. Evaluates local RBAC for the forwarded target verb with the immutable
   ResourceType/service and target Zone. Named methods retain one exact resource
   name. Nameless `List`/`Watch` retain an exact non-empty authorized
   `resourceNames` set and bounded filters whose possible results are a subset
   of that set.
3. Separately evaluates local RBAC for the `relay` session verb against the
   authenticated inbound Zone transport subject, governing ZoneLink, exact
   target bounds, and route-selected next hop.
4. Fails closed if either check or policy state is missing; no grant is inferred
   from a prior hop.
5. Decrements the hop counter; refuses if zero.
6. Opens the allocator-bound ComponentSession represented by the next child
   Zone's local uplink.
7. Re-serializes the request with the decremented hop counter; preserves
   the named target or nameless selector, all filters/pagination/watch cursor,
   and the original operation/idempotency/correlation/trace IDs unchanged.
8. Returns the child response to the inbound caller.

Relay is a distinct RBAC verb. Intermediate Zones may deny relay without
blocking local resource calls. A relay allow never supplies the target-verb
allow.

### No cross-Zone resource references

A resource returned from the child Zone contains only child-Zone
`*Ref` values. The parent sees the resource opaquely. The parent may call
`Get`, `List`, `Watch`, `UpdateSpec`, `UpdateStatus`, `Delete`, and
`CommitBatch` on child resources, but:

- cannot create a parent-local `*Ref` pointing at a child resource;
- cannot use a child resource's `metadata.uid` in a parent resource spec;
- cannot resolve `ResolveRef` across Zone boundaries;
- cannot inject ownerRef cycles that span Zones.

An ownerRef in a child resource must resolve to a resource in the same
child Zone. An attempt to set `ownerRef` to a parent-Zone resource is
rejected at child admission with `resource-ref-invalid`.

### Watch forwarding and revision cursor resync

A parent that opens a `Watch` on a child Zone resource type uses the
child's own revision cursors:

1. Parent sends `Watch(resourceType, filters, afterRevision)` over the
   ZoneLink ComponentSession.
2. Child streams `ResourceWatchEvent` items, each carrying the child's
   revision token.
3. If the ZoneLink session disconnects, parent route-session state records the
   last seen child revision; no parent-store ZoneLink row is created.
4. On reconnect, the parent re-issues `Watch` with the identical ResourceType,
   authorized name set, and filters plus `afterRevision=<last-seen>`.
5. If the child reports `revision-expired` (cursor too old), the parent
   re-issues `List` with that identical selector/filter set to obtain a fresh
   snapshot, then re-opens `Watch` with those same filters after the snapshot
   revision.

The parent may not merge child revisions with its own Zone revision
namespace. Parent-local watchers for child resources must be driven
separately from parent-local watches. Watch delivery uses the same
named-stream credit mechanism as local watches; backpressure on the
parent-side does not stall child-local delivery.

### Batch forwarding

`CommitBatch` targeting a single child Zone is forwarded atomically:
the child commits all mutations in one Zone transaction. A batch that
spans multiple Zones is rejected at the parent d2b-bus before any
forwarding; the caller must split it.

## Runtime service calls over ZoneLink

Runtime service calls (ComponentSession connect/invoke/open-stream and the
exact diagnostic methods authorized by `audit-export` or `support-bundle`) may
be forwarded through a ZoneLink if:

- the target service is declared in a child Zone Provider's service
  descriptor;
- the call's `purpose` class is `remote-zone`;
- RBAC at each forwarding hop grants both `relay` for the authenticated
  adjacent-Zone subject and the target session verb (`connect`, `invoke`,
  `open-stream`, `audit-export`, or `support-bundle`) for the exact
  service/method/stream. The diagnostic verbs remain bound only to
  `d2b.audit.v3.AuditService/Export` and
  `d2b.support.v3.SupportService/GenerateBundle` and grant no resource
  authority;
- the hop count does not exceed the fixed protocol limit of 16.

The forwarded session carries:

- original `AuthenticatedSubjectContext` digest (opaque; child cannot
  expand it);
- operation, idempotency, correlation, and trace IDs unchanged;
- decremented hop count in the session prologue;
- no raw credential, FD, or host path in any forwarded frame.

Named streams opened through a ZoneLink follow the same credit-forwarding
model as local named streams. Each hop maintains independent backpressure.
A blocked intermediate hop cannot cause unbounded memory growth at the
originating Zone.

## Operation lifecycle, idempotency, cancellation, and pinned reverse path

### Idempotency

Every mutating resource API call forwarded through a ZoneLink carries a
`ZoneLinkIdempotencyKey`:

```
ZoneLinkIdempotencyKey {
  operationId:        <caller-assigned opaque>
  idempotencyKey:     <caller-assigned opaque>
  sourceZonePath:     ZonePath
  targetZonePath:     ZonePath
  operationKind:      ResourceApiMethodKind
  principalDigest:    <sha256-hex of subject ref>
}
```

The dedup namespace is the full 6-tuple; the same opaque
`idempotencyKey` reused under a different source/target/operation/
principal cannot collide. The child-Zone resource API dedup engine is the
single dedup owner for calls that reach it. Intermediate hop relays do
not deduplicate; they forward.

Retention window: completed dedup records are retained for 15 minutes
(matching `DEFAULT_RETENTION` in `d2b-realm-router/src/lib.rs`). A
tombstone is kept for 60 minutes after that window (`DEFAULT_NO_REUSE_HORIZON`)
to ensure post-retention reuse fails closed.

States:
- `InProgress`: same key/request is still running; returns original
  `operationId`.
- `Replay`: same key/request completed; returns recorded result.
- `Conflict`: same key, different request fingerprint; refuses.
- `Expired`: key reused after retention window.

### Cancellation

A cancel message sent over the ZoneLink for a specific `operationId`:

1. Parent d2b-bus sends `Cancel(operationId)` downstream on the pinned
   reverse path.
2. Each intermediate hop relays the cancel to the next hop.
3. The target Zone's resource API cancels the outstanding operation if it
   is still running.
4. Cancellation is best-effort: a completed operation is not un-committed.
5. Cancel delivery failure does not extend the caller's deadline.

### Pinned reverse path

The route decision that admitted the operation produces a `ZoneRoutePath`.
This path is pinned for the lifetime of the operation:

- reply traffic from the child follows the same hops in reverse
  (`down-to-child` hops become `up-to-parent`);
- cancel and status-poll messages use the same pinned path;
- intermediate hops may not reroute in-flight traffic;
- if a hop on the pinned path fails, the operation fails with
  `zone-link-disconnected`; it is not silently rerouted.

The pinned path metadata carries no transport socket or credential; it is
a sequence of Zone tree edges plus the session generation bound to each
hop. If a hop's session generation changes (reconnect), the path is
invalidated and the operation fails with `zone-link-disconnected`. The
caller may retry with a new idempotency key after reconnect.

## Named streams and direct shortcuts

### Named streams over ZoneLink

A named stream opened through a ZoneLink occupies one stream slot on every
ComponentSession in the hop chain. Credit is independently managed at each
hop:

```
Source Zone                 Intermediate Zone         Target Zone
  stream writer
    |-- ZoneLink CS1 --->   relay stream
    |   (credit: parent)      |-- ZoneLink CS2 --->  target stream
    |                         |   (credit: relay)      (credit: child)
```

Backpressure from the target propagates inward hop by hop; no hop grants
more credit than its downstream grants it. A source that exceeds its hop-1
credit budget blocks rather than buffering unboundedly.

Named streams inherit the `purpose`, `service`, and `schema fingerprint`
of the originating session. At every hop the adjacent ZoneLink transport
subject is authenticated by that hop's KK ComponentSession and local RBAC
separately requires `relay` plus `open-stream` for the immutable target. No
forwarded payload may self-assert the subject or either authorization result.

### Direct shortcuts

A direct shortcut allows two Zones with an established path to bypass
the hop relay for subsequent operations, using a pre-authorized shortcut
ComponentSession established directly between source and target.

Shortcuts are authorized using `ZoneRouteEngine::decide_direct_shortcut`
(adapted from `RouteTreeEngine::decide_direct_shortcut` in
`d2b-realm-core/src/route_engine.rs`):

1. A `ZoneLinkShortcutAuthorizationRequest` is issued with a bounded
   expiry (max 1 hour).
2. The NCA Zone (the common ancestor on the tree path) authorizes the
   shortcut if:
   - the existing tree path is currently `Allowed`;
   - the requesting policy rule explicitly permits shortcuts
     (`allowDirectShortcut: true`);
   - the shortcut metadata does not carry transport endpoints or
     credentials (opaque `shortcutId` only).
3. The authorized `shortcutId` is presented to the target Zone's transport
   provider to establish a direct ComponentSession outside the tree relay
   chain.
4. The shortcut expiry and capability set cannot exceed those of the
   authorized tree path.
5. Shortcut teardown reasons: `completed`, `expired`, `policy-revoked`,
   `link-failure`.

A shortcut is optional optimization metadata. Absence of a shortcut does
not break routing; the tree relay path remains authoritative. Shortcuts
are not provisioned without explicit policy authorization.
Shortcut establishment does not convert `relay` into target authority: every
tree hop must admit the setup under its bounded relay and target scope, and
every operation on the resulting direct session is authorized for its target
verb at the destination.

## No FD, credential, or host path forwarding

Zone routing deliberately excludes the following from all inter-Zone
frames:

- **File descriptors (FDs)**: SCM_RIGHTS is local Unix only. A ZoneLink
  ComponentSession uses vsock or an Azure Relay transport; neither carries
  FDs. A request that requires FD transfer is rejected at the source
  d2b-bus with `attachment-not-permitted-over-zone-link`.
- **Credentials and secrets**: no token, session PSK, private key,
  bearer token, enrollment secret, or credential lease byte may appear in
  a forwarded frame payload, routing metadata, or named stream content.
  Credential resources contain only opaque `leaseId` handles, never
  credential material.
- **Host paths**: no filesystem path, socket path, device path, or
  store path may appear in routing metadata. Transport bindings reference
  only provider-schema-validated binding descriptors without raw paths.
- **PIDs, pidfds, and broker ops**: not propagated across Zones. A
  child Zone's Process lifecycle is managed entirely by child-local
  controllers.

Violation of these constraints is a structural check failure at the
serialization boundary, not a runtime policy decision.

## Link failure, restart, revocation, and loop and hop limits

### Link failure and restart

When a child-local ZoneLink ComponentSession disconnects:

1. The ZoneLink controller sets the link's `SessionEstablished` condition
   to `False` and phase to `Degraded` (or `Unknown` if no session was ever
   established this generation).
2. In-flight operations with a pinned path through this link fail
   immediately with `zone-link-disconnected`.
3. Child-to-parent intents may accumulate in the child store up to
   `spec.limits.maxPendingIntents`; parent-to-child operations fail
   immediately.
4. The child-local controller schedules bounded Provider-internal reconnect
   attempts, limited by `spec.limits.reconnectMaxAttempts` within
   `spec.limits.reconnectWindowSecs`.
5. On reconnect:
   - a new KK handshake is performed against the allocator-sealed enrolled
     identity and resolved `spec.transportCredentials`;
   - a new link epoch is assigned;
   - all pinned-path tracking for the old epoch is cleared;
   - queued intents are replayed (with original idempotency keys if the
     queued mutation is younger than the retention window, or as new
     operations if older);
   - the child-local ZoneLink handler re-issues parent route/export
     advertisement watches from the last known parent revision or triggers
     `List` + reopen if the cursor expired.
6. After reconnect, `status.resource.linkEpoch` is incremented and
   `status.resource.lastConnectedAt` is updated atomically with the cursor
   fields.

During disconnected recovery, the ZoneLink controller updates only its own
local status and outbound intent queue. It does not infer or mutate other
child-local resources from stale parent state, and it performs no remote
cleanup or status correction without a live session.

### Revocation

A ZoneLink may be administratively revoked by setting
`spec.disabled: true` and then deleting the resource.

Revocation sequence:

1. Spec update: `disabled` set to `true`; the child handler stops reconnecting.
2. Child-local ZoneLink controller issues a route withdrawal for all advertised
   `routeId` values.
3. The parent's `RouteTreeEngine` removes all entries for the child.
4. d2b-bus immediately returns `zone-link-revoked` for any new call
   targeting the child Zone or any descendant reachable only through
   this link.
5. Long-lived streams on this link receive `zone-link-revoked` and close.
6. In-flight operations are cancelled (best-effort); already-committed
   child-Zone operations are not rolled back.
7. The child-local ZoneLink resource deletion proceeds through normal finalizer
   policy and releases the parent allocator binding; no reciprocal parent
   resource requires deletion.

Authorization lease revocation: when a Role or RoleBinding governing
cross-Zone access changes, the parent's d2b-bus authorization engine
invalidates cached decisions immediately. New forwarded calls require
fresh authorization; outstanding long-lived streams receive a short
reauthorization deadline.

### Loop detection

The NCA algorithm detects loops using a `visited` set during the upward
walk from source to NCA. If the same Zone path appears twice in the
walk, the decision returns `loop`.

Additionally, the parent's `RouteTreeEngine` enforces:

- A route for `descendant` D must name exactly one `next_hop_child`; if
  two advertisements claim different next-hop children for the same
  descendant, the later one is rejected with `multi-parent`.
- A parent entry for child C must have `parent == local_root` or a
  non-expired parent entry for the intermediate path. A chain that
  refers back to its own `advertisingZone` is rejected at admission.

The hop counter enforced at source d2b-bus provides a belt-and-suspenders
limit independent of the tree walk. The protocol-wide initial budget is 16
and is not configurable in ZoneLink spec.

### Hop limits

Hop counter enforcement:

- Source d2b-bus decrements the counter before forwarding.
- An operation that arrives at d2b-bus with `remainingHops == 0` is
  refused with `hop-limit-exceeded`.
- The remaining-hops field is verified at each intermediate hop; a frame
  that claims more remaining hops than it arrived with is rejected with
  `malformed-hop-count`.
- The counter is scoped per-call, not per-session.

## Local intents while disconnected

The child-local ZoneLink handler may enqueue `UpdateSpec`, `Create`, and
`Delete` intents directed from the child to parent/ancestor resources while
the uplink is disconnected. Parent-to-child mutations are not queued in the
child and fail at the parent route boundary.
Behavior:

- Intents are stored in the child Zone as bounded `ZoneLinkIntent` records, not
  resource mutations. No parent resource state is assumed.
- `spec.limits.maxPendingIntents` limits the queue. When the queue is full,
  new intents fail with `zone-link-intent-queue-full`.
- Intent records carry the original operation/idempotency/correlation IDs
  and a `queuedAt` timestamp.
- On reconnect, intents are replayed in order with their original
  idempotency keys if within the retention window; older intents are
  replayed with fresh operation IDs and marked as `late-replay`.
- A replayed intent that receives `resource-conflict` (stale revision) is
  not retried automatically; the caller is notified with the conflict.
- A `Get`, `List`, or `Watch` call traversing a disconnected uplink always
  returns `zone-link-disconnected` immediately; it is never queued.
- If `maxPendingIntents` is zero, mutating calls fail immediately with
  `zone-link-disconnected`; intents are never silently dropped.

Local intent queueing does not constitute a claim of parent-Zone state.
The ZoneLink handler does not infer parent resource phase, condition, or
status from locally queued intents.

## Topology diagrams

### K0 / K1 / K2 example (Host/Guest terminology)

```
K0: parent Zone (Host-based, runs on physical host)
  Host/host-system
  sealed topology/route state only (no ZoneLink resource or handler)

K1: child Zone (Guest VM, cloud-hypervisor)
  ZoneLink/k1-uplink -> K0 allocator (transport: unix socket)
  Guest/dev-vm
  Host/host-system    (accessible only within K1)

K2: grandchild Zone (nested VM or ACA container)
  ZoneLink/k2-uplink -> K1 allocator (transport: vsock CID 5)
  Guest/work-container
  Process/web-server  (K2-local only)
```

Route tree from K0's perspective:

```
K0 (local root allocator)
 └── K1 (child-local ZoneLink/k1-uplink; advertises itself + K2)
      └── K2 (child-local ZoneLink/k2-uplink; next-hop K1)
```

A call from K0 to `Process/web-server` in K2:
1. K0 d2b-bus: NCA=K0, path K0→K1→K2, 2 hops.
2. K0 opens the allocator-bound ComponentSession represented by K1's local
   `ZoneLink/k1-uplink`.
3. K1 d2b-bus: hop count decremented (1 remaining), relays to K2.
4. K2 d2b-bus: hop count decremented (0 remaining), dispatches to
   local `d2b.resource.v3`.
5. Reply returns K2→K1→K0 on the pinned reverse path.

RBAC at each hop:

```
K0:  subject=User/alice verb=get resourceType=Process zone=K2
     -> K0 Role admits the target operation before route selection
K1:  subject=Zone/k0 sessionVerbs=[relay] verbs=[get] resourceType=Process
     resourceNames=[web-server] zone=K2
     -> K1 RoleBinding independently allows one-hop forwarding and target get
K2:  subject=Zone/k1 verb=get resourceType=Process name=web-server
     -> K2 Role allows the authenticated adjacent Zone to get Process/web-server
```

### Simple K0/K1 example (local Host + remote Guest)

```
K0 (Host/host-system, local Zone)
  sealed topology/route state only (no ZoneLink resource or handler)

K1 (Guest/workvm, cloud-hypervisor VM)
  ZoneLink/k1-uplink -> K0 allocator
  Guest/workvm
  Process/wayland-proxy
  Process/shell-session
```

A watch from K0 on K1 Processes:

```
1. K0 ResourceClient: Watch(Process,
      resourceNames=[wayland-proxy,shell-session], zone=K1,
      afterRevision=none)
2. K0 d2b-bus: route to K1 via the allocation represented by K1's
   ZoneLink/k1-uplink
3. K1: List with the identical resourceNames filter → revision R1;
   Watch with that same filter and afterRevision=R1
4. K1 streams events to K0 named-stream (credit-bounded)
5. Disconnect: K0 records lastRevision=R1
6. Reconnect: K0 re-opens Watch with the identical filter and afterRevision=R1
7. K1 delivers matching events after R1 or returns revision-expired → K0
   relists with the identical filter
```

## Audit, OTEL, errors, and security

### Audit records

Zone routing emits the following audit event types (adapted from
`RouteAuditEventKind` in `routing.rs`):

| Event | Contents |
| --- | --- |
| `zone-route-allowed` | sourceZone path digest, targetZone path digest, NCA digest, operation kind, hop count, policy rule id, session generation, correlation id |
| `zone-route-denied` | same as above + `RouteFailClosedReason` |
| `zone-advertisement-accepted` | advertising zone digest, controller generation ref, route count, expiry, replay window id |
| `zone-advertisement-denied` | same as above + reason |
| `zone-advertisement-withdrawn` | advertising zone digest, withdrawn route ids, issuer generation ref |
| `zone-link-session-established` | link name digest, child zone name digest, session generation |
| `zone-link-session-failed` | link name digest, reason code |
| `zone-link-intent-queued` | link name digest, operation kind, intent count |
| `zone-link-shortcut-authorized` | shortcut id, NCA digest, source/target zone digests |
| `zone-link-shortcut-torn-down` | shortcut id, teardown reason |
| `zone-link-revoked` | link name digest, revocation trigger |
| `zone-link-relay-admitted` | hop source/target zone digests, operation kind, session generation |
| `zone-link-relay-denied` | hop source/target zone digests, reason |

Audit records exclude:

- raw Zone tree paths (replaced by opaque digests);
- transport endpoints, socket paths, or relay addresses;
- resource payloads, spec/status content, or Provider diagnostics;
- credentials, tokens, or key material;
- host paths, process identities, or PIDs.

### OTEL metrics

Metric labels use closed low-cardinality sets:

| Metric | Labels |
| --- | --- |
| `d2b.zone_route.decision.total` | `outcome` (allowed/denied), `operation_kind`, `hop_count_bucket`, `reason_code` |
| `d2b.zone_route.advertisement.total` | `outcome` (accepted/denied/withdrawn/replayed), `reason_code` |
| `d2b.zone_link.session.state` | `phase` (pending/ready/degraded/failed/unknown) |
| `d2b.zone_link.reconnect.total` | `reason` |
| `d2b.zone_link.intent.queued` | (none) |
| `d2b.zone_link.relay.total` | `outcome`, `operation_kind` |
| `d2b.zone_route.shortcut.total` | `outcome` (authorized/denied/torn-down), `teardown_reason` |

Prohibited labels: Zone names/UIDs, ZoneLink names or hashes/digests derived
from them, resource names, subject refs, provider diagnostics, host paths,
session keys, or advertisement payload. Zone identity remains the `d2b.zone`
OTEL resource attribute; ZoneLink identity remains available in authorized
audit records, never metric labels.

### OTEL spans

Spans are emitted for:

- per-call route decision (leaf span);
- per-hop relay (child span);
- ZoneLink session establish/reconnect (span with generation attribute);
- advertisement admission (span with replay-window depth attribute).

Span attributes include operation kind, hop count, outcome code, and
correlation/trace IDs. No resource payload, zone name, or endpoint is
included in span attributes.

### Stable error codes

| Error code | Meaning |
| --- | --- |
| `zone-link-disconnected` | ZoneLink session not established; call rejected |
| `zone-link-revoked` | ZoneLink was administratively revoked |
| `zone-link-intent-queue-full` | Intent queue reached `spec.limits.maxPendingIntents` |
| `hop-limit-exceeded` | Forwarded call has no remaining hops |
| `malformed-hop-count` | Hop counter in inbound frame claims more hops than allowed |
| `relay-denied` | Intermediate Zone lacks the exact ZoneLink-scoped relay grant |
| `authorization-denied` | A source, intermediate, or target Zone denied the forwarded operation's target verb |
| `attachment-not-permitted-over-zone-link` | FD attachment rejected on ZoneLink transport |
| `zone-route-not-found` | No ZoneLink path exists to the target Zone |
| `zone-route-capability-denied` | Required capability absent from route |
| `zone-route-loop` | NCA algorithm detected a cycle |
| `zone-route-multi-parent` | Route table has conflicting parents for one child Zone |
| `zone-advertisement-namespace-violation` | Advertisement exceeds allocated namespace or capability scope |
| `zone-advertisement-replay` | Duplicate advertisement (replay rejection) |
| `zone-advertisement-expired` | Advertisement received after expiry |
| `zone-advertisement-malformed` | Advertisement fails structural invariants |
| `zone-shortcut-denied` | NCA did not authorize the shortcut |
| `zone-shortcut-expired` | Shortcut token expired |

Error messages are bounded, UTF-8/control-character validated, and must
not contain resource payloads, zone paths, transport endpoints,
credentials, or provider diagnostics.

### Security invariants

1. A Zone never grants authority beyond its own Role/RoleBinding evaluation.
   Forwarded calls require the target verb at each hop and a separate `relay`
   allow at each forwarding hop.
2. A child Zone's resource status is not inferred from local intents; the
   parent only learns child state from authenticated responses.
3. The KK handshake verifies the allocator-sealed enrolled identity and
   resolved transport credentials; a different identity is refused before
   any resource exchange.
4. Allocated capability scopes narrow monotonically downward; a child cannot
   advertise capabilities beyond what its parent allocated.
5. No FD, credential, or host path is forwarded. Transport bindings are
   provider-schema-validated opaque values.
6. Loop detection and hop limits bound resource consumption; a malformed
   route tree cannot cause unbounded forwarding.
7. Route advertisements are signed and expiring. A replay of a prior
   valid advertisement fails the replay-window check.
8. Zone tree paths in audit records are replaced by opaque digests;
   raw path strings never enter low-cardinality metrics.

## Current-code fit

The following table classifies every Realm-related current baseline symbol.
Evidence classes: **A** = implemented-and-reachable from production binary,
**B** = implemented-but-unwired, **C** = generated-or-eval-contract,
**D** = test-only.

### Core routing types

| Symbol | File | Class | Production callers | ADR 0046 mapping |
| --- | --- | --- | --- | --- |
| `RouteTreeEngine` (NCA, `admit_advertisement`, `decide_route`) | `d2b-realm-core/src/route_engine.rs` | **B** | None in d2bd/CLI - inline test suite at line 1202 (45 functions) | → `ZoneRouteEngine` (adapt/rename) |
| `RouteAdvertisement`, `RouteNamespaceAllocation`, `RealmTreeEdge`, `DescendantRoute` | `routing.rs` | **B** | Only in `route_engine.rs` tests | → v3 advertisement envelope with ZonePath |
| `TreeRoutePath`, `TreeRouteHop` | `routing.rs` | **B** | Same | → `ZoneRoutePath`/`ZoneRouteHop` (rename + ZonePath) |
| `RouteFailClosedReason` | `routing.rs` | **B** | Same | → preserved + extended |
| `DirectShortcutAuthorizationRequest/Decision/Teardown` | `routing.rs` | **B** | Same | → ZoneLinkShortcutAuthorization* (adapt) |
| `RouteAuditEventKind` (12 variants) | `routing.rs` | **B** | Same | → Zone-prefixed audit events |
| `RouteRealmClass` (`LocalRoot`…`EphemeralDiscovered`) | `routing.rs` | **B** | Same | → `ZoneClass` metric label (variants map to private Zone runtime bootstrap placement types) |
| `RouteId`, `ControllerGenerationId` | `ids.rs` | **A** | Used in gateway, d2bd, CLI via `d2b_realm_core` | → `ZoneRouteId`, `ZoneLinkControllerGeneration` |

### Identifier types

| Symbol | File | Class | Production callers | ADR 0046 mapping |
| --- | --- | --- | --- | --- |
| `RealmId` (one label) | `ids.rs` | **A** | d2bd, d2b CLI, d2b-gateway, priv-broker | → `ZoneLabelId` |
| `RealmPath` (Vec<RealmId>) | `realm.rs` | **A** | d2bd, d2b CLI, d2b-gateway | → `ZonePath`; label grammar identical; public address format changes |
| `WorkloadId` | `ids.rs` | **A** | d2bd, d2b CLI, d2b-gateway, realm-router | → split: `Guest/<name>` or `Host/<name>` per semantic classification |
| `NodeId` | `ids.rs` | **A** | d2b-realm-core, d2b-gateway | → Host resource name or Zone-local implicit address |
| `ProviderId`, `ExecutionId`, `StreamId`, `PrincipalId`, etc. | `ids.rs` | **A** | Throughout | → preserved with v3 names; secret-marker reject logic retained |
| `EntrypointMode` (`HostResident`/`GatewayBacked`) | `realm.rs` | **A** | `d2b/src/target_routing.rs` | → subsumed into ZoneLink `spec.transportProviderRef` |
| `RealmControllerPlacement` (6 variants) | `realm.rs` | **A** | `d2b-realm-core`, bundle artifacts | → private Zone runtime bootstrap placement; compiler-only `parentZone` selects the allocator owner while child-local uplink identity/transportProviderRef supplies transport state (none is a public Zone.spec parent field) |
| `TargetName` (DNS-form target string) | `target.rs` | **A** | d2bd, d2b-gateway | → v3 resource ref path |
| `RealmTarget` (workload + realm path) | `target.rs` | **A** | d2bd/realm_access_resolver.rs | → Zone-local resource lookup |

### Access resolver and CLI routing

| Symbol | File | Class | Production callers | ADR 0046 mapping |
| --- | --- | --- | --- | --- |
| `RealmAccessBinding`, `RealmTransportBinding`, `RealmAccessResolverRequest/Response/Error` | `access.rs` | **B** | `realm_access_resolver.rs` only (which is itself unwired) | → ZoneAccessBinding; ZoneEntrypointResolver |
| `resolve_local_root_realm_access()` | `d2bd/src/realm_access_resolver.rs` | **B** | `pub mod` declared at `d2bd/src/lib.rs:117`; no callers in running daemon | → ZoneEntrypointResolver (routing-003) |
| `RealmEntrypointTable`, `DispatchTarget` | `d2b-realm-router/src/target_resolver.rs` | **A** | `d2b/src/lib.rs:5240` (`load_realm_entrypoint_table()`); `d2b/src/target_routing.rs` | → ZoneEntrypointResolver (routing-003) |
| `Route::Local`/`Route::GatewayBacked` dispatch | `d2b/src/target_routing.rs` | **A** | CLI live routing path | → sealed topology + authenticated `ZoneRouteEngine` projection routing |
| `realm list`, `realm inspect` CLI commands | `d2b/src/lib.rs:5942` | **A** | CLI; reads `realm-entrypoints.json` | → `zone list`/`zone inspect` from compiler topology joined with authenticated route/projection status; never a parent ZoneLink list |

### Operation routing and session layer

| Symbol | File | Class | Production callers | ADR 0046 mapping |
| --- | --- | --- | --- | --- |
| `OperationRouter`, `DEFAULT_RETENTION/NO_REUSE_HORIZON/MAX_DEDUP_RECORDS` | `d2b-realm-router/src/lib.rs` | **B** | Only `d2bd/src/realm_stubs.rs` (`#[allow(dead_code)]`, declared `pub mod` at `d2bd/src/lib.rs:249`) | → ZoneLinkIdempotencyKey 6-tuple in d2b-bus (routing-005) |
| `RemoteNodeRegistry`, `RemoteNodeEntry`, `RemoteNodeAvailability` | `d2b-realm-router/src/remote_node.rs` | **B** | Same dead_code seam only | → ZoneLink controller handler (routing-004) |
| `SessionLifecycle`, `SessionPhase` | `d2b-realm-router/src/session_lifecycle.rs` | **B** | Same dead_code seam only | → ZoneLink session state machine (routing-004) |
| `MuxSession` stream/operation forwarding | `d2b-realm-router/src/mux_session.rs` | **B** for zone relay; **D** for test suite | No zone relay callers; only used in realm-router internal tests and display transport | → d2b-bus Zone relay path (routing-005) |
| `PeerSession<C>`, `SecurePeerSession<C>` | `d2b-realm-router/src/session.rs`, `secure_session.rs` | **B** (from d2bd); reachable within realm-router tests | No d2bd/CLI import | → ComponentSession/Noise KK (d2b-bus) |
| `DisplayTransportBinding`, `verify_display_preface` | `d2b-realm-router/src/display_transport.rs` | **A** within realm-router | Display-session path only | Not Zone routing; remains display-specific |

### Wire protocol

| Symbol | File | Class | Production callers | ADR 0046 mapping |
| --- | --- | --- | --- | --- |
| `ConstellationFrame` (all variants), `Handshake*`, `OperationRequest/Response`, `StreamOpen/Data/Flow/Close/Resume` | `d2b-realm-core/src/frame.rs` | **B** from d2bd; **A** within realm-router and d2b-gateway-runtime (individual types) | `d2b-gateway-runtime` uses individual frame types; d2b-realm-codec-protobuf serializes via realm-router | → v3 d2b-bus frame variants; codec protobuf numbers refrozen |
| `ProtobufCodec` | `d2b-realm-codec-protobuf/src/lib.rs` | **A** within realm-router; **B** from d2bd | `d2b-realm-router` session/mux layer only | → v3 codec (routing-002 + bus spec) |
| `StreamMux` (open stream table) | `d2b-realm-core/src/mux.rs` | **B** from d2bd; **A** within realm-router tests | realm-router internal tests | → v3 named-stream multiplexing |

### Enrollment and identity

| Symbol | File | Class | Production callers | ADR 0046 mapping |
| --- | --- | --- | --- | --- |
| `EnrollmentRecord`, `KeyRotationPlan`, `RevocationRecord`, `SessionTeardownDirective` | `d2b-realm-core/src/enrollment.rs` | **B** | `RealmIdentityStore` only (itself unwired) | → ZoneLink controller session/revocation lifecycle |
| `RealmIdentityStore` | `d2b-realm-core/src/identity_store.rs` | **B** | No production callers found | → ZoneLink controller enrollment state machine |
| `RealmIdentityConfigJson` (schema v2) | `d2b-realm-core/src/identity_config.rs` | **A** | Loaded at d2bd startup (lib.rs:1425) and priv-broker startup (runtime.rs:704); logs "runtime trust sessions remain inert" | → allocator-sealed ComponentSession enrollment identity plus ZoneLink `spec.transportCredentials`; artifact retires |

### Allocator engine

| Symbol | File | Class | Production callers | ADR 0046 mapping |
| --- | --- | --- | --- | --- |
| `LocalRootAllocatorEngine`, `AllocatorAllocationDecision`, `FakeAllocatorLedger` | `d2b-realm-core/src/allocator_engine.rs` | **B** | No production callers outside crate | → Zone resource allocation engine (core-controller) |
| `LeaseAllocationRequest`, `AllocatorLease`, `GrantedHostResource` | `d2b-realm-core/src/allocator.rs` | **B** | No production callers outside crate | → Zone/Provider resource spec fields |
| `AllocatorJson`, `AllocatorRealm`, `RealmPlacement` | `d2b-core/src/allocator_config.rs` | **C** | Loaded by bundle resolver; read by priv-broker/d2bd as eval contract | → Zone resource spec in Nix |

### Provider traits

| Symbol | File | Class | Production callers | ADR 0046 mapping |
| --- | --- | --- | --- | --- |
| `WorkloadProvider`, `DisplayProvider`, `RuntimeProvider`, `PersistentShellProvider` traits | `d2b-realm-provider/src/provider.rs` | **B** | `d2b-host-providers` implements (itself unwired); `d2b-provider-aca`/`relay` implement and ARE wired for display session only | → Provider ResourceType controller/service processes |
| `AcaWorkloadProvider` | `d2b-provider-aca/src/lib.rs` | **A** (display session path) | d2bd `new_gateway_display_runtime_from_config` (lib.rs:4165); ACA display gateway only | Not Zone routing; display path remains display-specific |
| `CapabilitySet`, `WorkloadCapabilitySet`, `DisplayCapabilitySet` | `d2b-realm-provider/src/capabilities.rs` | **B** | Traits only; d2b-gateway-runtime uses `DisplayCapabilitySet` | → v3 private route-allocation capability fields |
| `ProviderCircuitBreaker` | `d2b-realm-provider/src/rate_limit.rs` | **B** | `d2b-provider-aca` only (display path) | → Provider rate-limit policy |

### Bundle artifacts and Nix options

| Symbol | File | Class | Production callers | ADR 0046 mapping |
| --- | --- | --- | --- | --- |
| `RealmControllersJson` (schema v2) | `d2b-core/src/realm_controller_config.rs` | **C** | Loaded at d2bd startup (lib.rs:1408) and priv-broker (runtime.rs:687); logs "runtime routing remains inert" | → compiler-only `parentZone` bootstrap topology + runtime Zone self-resource + child-local ZoneLink resources; artifact retires |
| `WorkloadTargetIndex` | `d2bd/src/workload_target_index.rs` | **A** | d2bd `lib.rs:16745` in `PublicRequestArtifacts`; maps canonical targets → VM names for exec dispatch | → Guest/Host resource lookup; retires with legacy VM dispatch |
| `realm-workloads-launcher-v2.json` | `nixos-modules/realm-workloads-launcher-v2-json.nix` | **C** | Active bundle artifact (installed root:d2bd 0640); consumed by launcher clients | → v3 workload catalog from Zone resources |
| `realm-workloads-launcher.json` (v1) | `nixos-modules/realm-workloads-launcher-json.nix` | **C** | **NOT installed** by `bundle.nix`; declared artifact but dead | Retire |
| `d2b.realms.*` Nix options | `nixos-modules/options-realms.nix`, `options-realms-workloads.nix`, `options-realms-network.nix` | **C** | Eval-time; drives JSON artifact emission | → `d2b.zones.*` with compiler-only `parentZone` plus schema-mirrored resources |

### Architecture change summary

The following transitions are NOT simple textual renames:

1. **`RealmControllerPlacement` enum → private Zone runtime bootstrap placement + explicit parent topology + child-local uplink identity**: 6 variants collapse into per-Zone bootstrap configuration; compiler-only `parentZone` selects the allocator owner, and the child-local ZoneLink's `childZoneName`/`transportProviderRef` supplies transport state. The compiler seals the parent edge into allocator state. Placement and `parentZone` are not public `Zone.spec` fields; `Zone.spec` is `{}`.
2. **`WorkloadId` → Guest/Host split**: VM/sandbox workloads become `Guest`; local/bare-metal become `Host`. Classification is semantic, not mechanical.
3. **`CapabilitySet`-only authz → RBAC + allocated capability scope**: current engine checks a route allocation scope only. Independent per-hop target-verb and `relay` RBAC is new.
4. **`RealmPath` DNS target form → Zone resource path**: grammar preserved; wire address format changes.
5. **`EntrypointMode` enum → child-local ZoneLink transport plus topology projection**: `HostResident`/`GatewayBacked` mode is replaced by the child's transport spec; parent routing and CLI inspection use sealed topology and authenticated route projection state.
6. **`realm-controllers.json`/`realm-identity.json` → sealed parent topology + child-local Zone/ZoneLink state**: data is loaded today but routing/trust sessions are explicitly inert.
7. **`d2b-realm-provider` trait family → Provider ResourceType**: trait dispatch is replaced by typed resource controllers.

| Item | Treatment |
| --- | --- |
| Behavior retained | Pure in-memory NCA tree-walk; loop/multi-parent detection; advertisement replay-window; allocated capability-scope propagation; idempotency dedup full 6-tuple `(realm, principal, node, operation_kind, idempotency_key)`; fail-closed on unknown realm/route; bounded audit label cardinality; `TreeRoutePath`/`TreeRouteHop` already exist (rename to ZoneRoutePath/ZoneRouteHop); `DirectShortcut*` machinery already exists; `RouteAuditEventKind` event set already exists |
| Required delta | Consume the canonical ZoneLink ResourceType spec/status/intent contract from ADR046-zone-control-002; compiler-only validated `parentZone` map sealed into allocator bootstrap topology; independent target-verb plus canonical `relay` RBAC checks per intermediate hop; watch cursor resync over ZoneLink; named-stream credit forwarding; hop counter byte in wire frames; no-FD/credential structural rejection at serialization boundary; private Zone runtime bootstrap placement (replaces placement enum, not a public spec field); per-hop subject narrowing |
| Reuse path | Copy and adapt `RouteTreeEngine` (rename RealmPath→ZonePath, RouteId→ZoneRouteId); copy `RouteAdvertisement`/`RouteNamespaceAllocation`/`TreeRoutePath`/`TreeRouteHop`/`RouteFailClosedReason`/`DirectShortcut*`/`RouteAuditEventKind`; copy `OperationRouter` idempotency dedup; extract `RealmEntrypointTable` suffix-match into ZoneEntrypointResolver; adapt KK handshake from `SecurePeerSession`/`PeerSession` |
| Replacement/deletion | `RealmEntrypointTable`/`RouteTreeEngine` on RealmPath types retire after ZoneRouteEngine live; `RemoteNodeRegistry` retires when ZoneLink controller live; `WorkloadTargetIndex` retires when Guest/Host resource lookup live; CLI `Route::GatewayBacked` retires when ZoneLink handles all cross-Zone routing; `realm-controllers.json`/`realm-identity.json` retire when sealed `parentZone` topology, runtime Zone identity, and ZoneLink transport state replace them |
| Feasibility proof | `route_engine.rs` inline test suite (45 functions at line 1202) proves NCA, advertisement, loop, capability, replay, DirectShortcut; `target_resolver.rs` tests prove suffix match; `lib.rs` tests prove idempotency dedup namespace |
| Future owner | Work items below |

## Implementation work items

### ADR046-routing-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-001` |
| Dependency/owner | W0/W1a; zone routing contract owner |
| Current source | `packages/d2b-realm-core/src/routing.rs`: `RouteAdvertisement`, `RouteNamespaceAllocation`, `RealmTreeEdge`, `DescendantRoute`, `TreeRoutePath`, `TreeRouteHop`, `RouteFailClosedReason`, `DirectShortcutAuthorizationRequest`, `DirectShortcutAuthorizationDecision`, `DirectShortcutTeardown`, `DirectShortcutTeardownReason`, `RouteAuditEventKind`, `RouteRealmClass`, `RoutePlacementClass`, `RouteAuditEventMetadata`, all route newtypes; `packages/d2b-realm-core/src/realm.rs`: `RealmPath`, `MAX_REALM_LABELS`, `MAX_REALM_PATH_BYTES`, `RealmControllerPlacement`, `EntrypointMode`; `packages/d2b-realm-core/src/ids.rs`: `RealmId`, `RouteId`, `ControllerGenerationId`, `WorkloadId`, `NodeId`, `ProviderId` (evidence: **A** for ids.rs - used in production; **B** for routing.rs - types exist with tests but no production daemon routing callers) |
| Reuse source | Same v3 baseline commit `b5ddbed6` |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/zone_routing.rs` |
| Detailed design | Rename RealmPath → ZonePath, RealmId → ZoneLabelId, RouteId → ZoneRouteId, ControllerGenerationId → ZoneLinkControllerGeneration; preserve all bounds/validation/serde; add ZoneLink-specific advertisement envelope fields (v3 schema version and private allocated-capability field); preserve `RouteFailClosedReason` + add `zone-link-disconnected`, `hop-limit-exceeded`, `relay-denied`, `attachment-not-permitted-over-zone-link`; freeze v3 protobuf numbers separately from v2 Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | d2b-bus zone route engine and ZoneLink controller consume these types |
| Data migration | Full reset; no v2 Realm route compatibility |
| Validation | Golden advertisement/path/failure vectors shared by Rust/Nix; property tests for NCA/loop/allocated-capability narrowing; replay-window tests; hop-count tests |
| Removal proof | v3 old `RealmPath` route types retired after zone-routing engine is live and all callers switched |

### ADR046-routing-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-002` |
| Dependency/owner | ADR046-routing-001; zone route engine owner |
| Current source | `packages/d2b-realm-core/src/route_engine.rs`: full `RouteTreeEngine` struct and impl; `RouteInventoryEntry`, `RoutePruneReport`, `DirectShortcutAuthorizationRequest/Decision/Teardown`; all helper functions |
| Reuse source | Same v3 baseline commit `b5ddbed6` |
| Reuse action | adapt |
| Destination | `packages/d2b-zone-routing/src/engine.rs` |
| Detailed design | Adapt RouteTreeEngine to ZoneRouteEngine using ZonePath/ZoneRouteId/ZoneLinkControllerGeneration from ADR046-routing-001; preserve all NCA/loop/multi-parent/capability/replay/capacity logic; add `decide_route` enforcement of the fixed protocol hop budget; add hop-counter decrement in relay path; expose `ZoneRoutePath` in v3 types Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | d2b-bus calls `ZoneRouteEngine::decide_route` for every cross-Zone ResourceClient call; ZoneLink controller calls `admit_advertisement`/`admit_withdrawal` |
| Data migration | None (pure in-memory engine) |
| Validation | Copy exact `route_engine.rs` test suite adapted to ZonePath; add relay/hop-count/RBAC-narrowing/shortcut integration tests |
| Removal proof | `RouteTreeEngine` on v3 RealmPath types retired after ZoneRouteEngine is exercised in all bus routing paths |

### ADR046-routing-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-003` |
| Dependency/owner | ADR046-routing-001, ADR046-routing-002; Zone route resolver owner |
| Current source | `packages/d2b-realm-core/src/routing.rs`: `RouteNamespaceAllocation`; `packages/d2b-realm-core/src/access.rs`: `RealmAccessResolverRequest/Response/Error`, `RealmAccessBinding`, `RealmTransportBinding`, `RealmAccessClientContract`, `UnixSocketPath`, `AccessBindingRef`, all access types (evidence: **B** - complete implementation, no production callers); `packages/d2bd/src/realm_access_resolver.rs`: `resolve_local_root_realm_access()`, `local_root_realm_access_client_contract()` (evidence: **B** - `pub mod` at `d2bd/src/lib.rs:117`, no callers from running daemon); `packages/d2b-realm-router/src/target_resolver.rs`: `RealmEntrypointTable`, `DispatchTarget`, `RealmEntrypoint`, `ResolveError` (evidence: **A**); `packages/d2b/src/lib.rs:5240`: `load_realm_entrypoint_table()` (evidence: **A**); `packages/d2b/src/target_routing.rs`: `Route::Local`/`Route::GatewayBacked` dispatch (evidence: **A**); `packages/d2b-realm-core/src/realm.rs`: `EntrypointMode`, `RealmControllerPlacement` (evidence: **A** as types; routing use **B**); `packages/d2b-core/src/realm_controller_config.rs`: `RealmControllersJson` (evidence: **C**); `nixos-modules/realm-controller-config-json.nix` (evidence: **C**) |
| Reuse source | Same v3 baseline commit `b5ddbed6` |
| Reuse action | adapt |
| Destination | `packages/d2b-zone-routing/src/resolver.rs` (ZoneEntrypointResolver) |
| Detailed design | Consume the canonical ZoneLink spec/status/intent types owned by ADR046-zone-control-002; ZoneLinkNamespaceAllocation is issued by the exact parent allocator selected in sealed Nix-compiled `parentZone` topology; ZoneEntrypointResolver uses longest-suffix match over ZonePath (adapted from `RealmEntrypointTable::resolve`) and is driven only by sealed `{ childZone, parentZone }` rows plus authenticated admitted route projections; no reciprocal parent-store resource or parent ZoneLink handler; fail closed on unknown topology, absent/stale projection, or unauthenticated route Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Child core-controller ZoneLink handler manages child-store ZoneLink resources; parent d2b-bus feeds sealed topology and authenticated `ZoneRouteEngine` projection state to ZoneEntrypointResolver for per-call dispatch |
| Data migration | None; ZoneLink resources created from Nix configuration at v3 reset |
| Validation | Longest-suffix match vectors over sealed topology; child-local ZoneLink spec validation; resolver rejects unknown/stale/withdrawn/unauthenticated route projections; parent-store fixture contains no ZoneLink row or handler |
| Removal proof | `RealmEntrypointTable` retired after all host-daemon routing paths use ZoneEntrypointResolver |

### ADR046-routing-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-004` |
| Dependency/owner | ADR046-routing-003; core-controller ZoneLink handler owner |
| Current source | `packages/d2b-realm-router/src/remote_node.rs`: `RemoteNodeRegistry`, `RemoteNodeEntry`, `RemoteNodeAvailability`, `RemoteNodeErrorKind`, `RemoteRetryAction`, `ensure_remote_execution_generation` (evidence: **B** - only in `d2bd/src/realm_stubs.rs` dead_code seam); `packages/d2b-realm-router/src/session_lifecycle.rs`: `SessionLifecycle`, `SessionPhase` (evidence: **B** - same seam); `packages/d2bd/src/realm_stubs.rs`: compile-only seam (`#[allow(dead_code)]`, declared at `d2bd/src/lib.rs:249`); `packages/d2b-realm-core/src/enrollment.rs`: `EnrollmentRecord`, `EnrollmentStatus`, `KeyRotationPlan`, `RevocationRecord`, `SessionTeardownDirective`, `RecoveryProcedure`, `IdentityAuditEventKind` (evidence: **B** - consumed by `RealmIdentityStore` which itself has no production callers); `packages/d2b-realm-core/src/identity_store.rs`: `RealmIdentityStore` (evidence: **B** - no production callers); `packages/d2b-realm-core/src/identity_config.rs`: `RealmIdentityConfigJson` (evidence: **A** - loaded at d2bd/priv-broker startup, routing inert); `nixos-modules/realm-identity-config-json.nix` (evidence: **C**); `packages/d2bd/src/workload_target_index.rs`: `WorkloadTargetIndex` (evidence: **A** - called at `d2bd/src/lib.rs:16745`; this is the live bridge from realm metadata to VM-name dispatch; retires with Guest/Host resource lookups) |
| Main reuse source | `packages/d2b-session/src/lifecycle.rs` (`SessionLifecycle`, `SessionPhase`, `KeepaliveAction`, `poll_keepalive`, `disconnect`, `begin_reconnect`, `reconnect_established`, `close`; limits: `MAX_RECONNECT_ATTEMPTS=10`, `MAX_RECONNECT_WINDOW_MS=300000`; test: `lifecycle_keepalive_close_and_reconnect_change_generation`) - adapt as the ZoneLink session state machine inside the ZoneLink handler; generation-increment logic maps to `status.resource.linkEpoch`; reconnect bounds come from `spec.limits.reconnectMaxAttempts` and `spec.limits.reconnectWindowSecs` |
| Reuse source | Same v3 baseline commit `b5ddbed6` |
| Reuse action | adapt |
| Destination | `packages/d2b-core-controller/src/zone_links.rs` |
| Detailed design | Child-local ZoneLink handler in core-controller: consumes the exact six-field ZoneLink schema from ADR046-zone-control-002 and manages local ResourceSpec→allocator-bound session→advertisement lifecycle; session state machine (Pending/Established/Disconnected/Reconnecting/Revoked); Provider-internal reconnect backoff bounded by `spec.limits`; advertisement issuance/renewal/withdrawal using enrolled KK ComponentSession; child-store route cursor and bounded outbound intent queue; private allocator capability-scope changes; D088 `status.resource` writer; aggregate metrics use only closed semantic phase/reason/outcome labels and never `link_name_hash` or another ZoneLink/Zone/resource identity label; Nix-compiled `parentZone` selects the parent allocator, which alone owns privileged listeners, placement, and route namespace and creates no reciprocal resource. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Child core-controller process → local transport Provider → sealed binding for the allocator selected by `parentZone` → d2b-bus ComponentSession; child ZoneLink handler exchanges advertisements while that parent ZoneRouteEngine admits/withdraws them |
| Data migration | New ZoneLink resources from Nix configuration; no prior enrollment compatibility |
| Validation | Session lifecycle tests; reconnect/disabled/revocation/allocator-policy-change; intent queue drain; cursor resync; advertisement renewal timing; fake-child tests; structural metric descriptor test asserts `vm`, `zone`, `zone_id`, `zone_uid`, and `link_name_hash` are absent and a ZoneLink-name canary never enters label values |
| Removal proof | `RemoteNodeRegistry` retired after all enrolled peer routing moves to ZoneLink handler |

### ADR046-routing-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-005` |
| Dependency/owner | ADR046-routing-002, ADR046-routing-007 (from ComponentSession spec); d2b-bus owner |
| Current source | `packages/d2b-realm-router/src/lib.rs`: `OperationRouter`, `RouteDecision`, `DEFAULT_RETENTION`, `DEFAULT_NO_REUSE_HORIZON`, `DEFAULT_MAX_DEDUP_RECORDS` (evidence: **B** - only in dead_code seam); `packages/d2b-realm-router/src/mux_session.rs`: `MuxSession` stream/operation forwarding (evidence: **B** for zone relay - stream-forwarding path for Zone relay does not exist yet; **D** within realm-router display-session tests); `packages/d2b-realm-core/src/frame.rs`: `ConstellationFrame`, `Handshake*`, `OperationRequest/Response`, `StreamOpen/Data/Flow/Close/Resume`, `OperationKind` (evidence: **B** from d2bd routing perspective; **A** within realm-router + `d2b-gateway-runtime` for individual types); `packages/d2b-realm-router/src/session.rs`: `PeerSession<C>` (evidence: **B** from d2bd; **A** within realm-router tests); `packages/d2b-realm-router/src/secure_session.rs`: `SecurePeerSession<C>`, `SecureSessionKey`, `NonceReplayGuard` (evidence: **B** from d2bd; reachable within realm-router) |
| Main reuse source | `packages/d2b-session/src/cancellation.rs` (`Cancellation`, `RequestRegistry`, `cancel_generated`, `CancelResult` 5 variants; test: `cancellation_is_generation_bound_and_shared`) - copy for cross-Zone cancellation forwarding; generation-bound registry maps to per-hop session generation; `packages/d2b-session/src/streams.rs` (`NamedStreamMux`, `StreamId`, credit model; tests: `named_stream_state_and_scheduler_have_independent_credit_and_fairness`, `driver_fragments_one_mib_logical_stream_under_256_kib_credit`) - credit state machine for ZoneLink named-stream forwarding; `packages/d2b-realm-router/src/lib.rs` `OperationRouter`/`DEFAULT_RETENTION=15min`/`DEFAULT_NO_REUSE_HORIZON=60min`/`DEFAULT_MAX_DEDUP_RECORDS=65536` idempotency dedup constants - adapt full 6-tuple dedup namespace `(zone, principal, node, operation_kind, idempotency_key)` as `ZoneLinkIdempotencyKey` |
| Reuse source | Same v3 baseline commit `b5ddbed6` |
| Reuse action | adapt |
| Destination | `packages/d2b-bus/src/zone_route.rs` (cross-Zone bus routing), `packages/d2b-bus/src/relay.rs` (per-hop relay handler) |
| Detailed design | Cross-Zone routing path in d2b-bus: ZoneEntrypointResolver consumes sealed topology plus authenticated route projections → ZoneRouteEngine::decide_route → admitted ComponentSession established by each next-hop child's local ZoneLink; hop-counter decrement and enforcement; independent target-verb plus canonical ZoneLink-scoped `relay` checks at each intermediate hop; idempotency key namespace (full 6-tuple) in ZoneLinkIdempotencyKey; pinned reverse path tracking; cancellation forwarding; watch cursor forwarding and revision-expired handling; no-FD/credential structural rejection at serialization boundary. No parent route step performs Resource API Get/List/Watch on ZoneLink Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | ResourceClient → d2b-bus → ZoneLink CS → intermediate zone → target zone; cancel/watch/stream all use the same routing path |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | End-to-end K0→K1→K2 resource call; relay-missing, target-verb-missing, wildcard/self-asserted relay, hop-limit, and FD-rejection tests; prove relay alone grants no CRUD/local lifecycle; idempotency namespace collision tests; cancellation delivery tests; watch resync tests |
| Removal proof | Old direct-dispatch and gateway-backed paths retired per bus routing parity |

### ADR046-routing-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-006` |
| Dependency/owner | ADR046-routing-002; benchmark owner |
| Current source | `packages/d2b-realm-core/src/route_engine.rs` inline `#[cfg(test)] mod tests` block at line 1202 (45 test functions covering NCA, advertisement admission/withdrawal, loop/multi-parent detection, capability ceiling, replay window, DirectShortcut authorization/teardown; evidence: **implemented-but-unwired** - tests are in-file, not in a separate `tests/` directory; no external test crate at `packages/d2b-realm-core/tests/*.rs`) |
| Reuse source | Same v3 baseline commit `b5ddbed6` |
| Reuse action | adapt |
| Destination | `packages/d2b-zone-routing/tests/route_engine_vectors.rs`, `packages/d2b-zone-routing/benches/route_decision.rs` |
| Detailed design | Copy exact advertisement/NCA/loop/capability/replay test vectors adapted to ZonePath; add K0/K1/K2 topology scenarios; add hop-count boundary tests; benchmark: p95 route decision for 1/10/100 active Zone tree entries <= 1 ms |
| Integration | Zone route engine correctness gate; bus relay integration tests |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | All v3 baseline route_engine test cases must pass; p95 benchmark gate |
| Removal proof | Not applicable |

### ADR046-routing-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-007` |
| Dependency/owner | W0; d2b-bus owner |
| Current source | None in v3 pre-ADR45 baseline. |
| Main reuse source | `packages/d2b-session/` (commit `a1cc0b2d`): `SessionEngine`, `ComponentSessionDriver` (20 async methods), `SessionDriverHandle`, `NoiseHandshake` (Nn/Kk/IKpsk2), `EstablishedHandshake`, `NegotiatedOffer`, `SessionLifecycle`/`SessionPhase`, `NamedStreamMux`/`StreamId`/`StreamPhase`, `Cancellation`/`RequestRegistry`, `OwnedAttachment`/`AttachmentPayload`, `FairScheduler`/`QueueClass`/`OutboundFrame`, `RecordProtector`/`ProtectedRecord`, `BootstrapAdmission`, `OwnedTransport`/`TransportPacket`/`TransportDescriptor`, `MetricsSink`/`NoopMetrics`; `packages/d2b-contracts/src/v2_component_session.rs`: all protocol constants and wire types; `packages/d2b-session/tests/component_session.rs` (all 18+ test functions) and `tests/noise_vectors.rs` (canonical Noise vectors). |
| Reuse action | adapt |
| Destination | `packages/d2b-bus/src/session/` |
| Detailed design | Copy `d2b-session` crate wholesale into `d2b-bus/src/session/`; adapt `EndpointPurpose`/`EndpointRole`/`ServicePackage` closed-enum tags for v3 purposes; strip `GUEST_SESSION_CREDENTIAL_*` types; strip `serve_ttrpc_services` fixed-endpoint binding (replaced by allocator-issued FD bootstrap); adapt `SessionEngine` as ZoneLink session drive loop; keep all Noise profiles (Nn/Kk/IKpsk2), generation discovery, record/fragment/keepalive/credit/cancellation/attachment logic verbatim Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | ZoneLink controller instantiates one `ComponentSessionDriver` per ZoneLink, typed as Kk for enrolled peers and Nn for initial bootstrap; d2b-bus routes ResourceClient calls through these drivers |
| Data migration | None (new infrastructure) |
| Validation | Port all `component_session.rs` tests; port `noise_vectors.rs`; add ZoneLink-specific KK enrollment test; add ZoneLink reconnect/revocation integration test |
| Removal proof | Not applicable (new crate) |

### ADR046-routing-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-008` |
| Dependency/owner | ADR046-routing-007; transport-provider owner |
| Current source | None in v3 pre-ADR45 baseline (vsock and seqpacket paths are provider-specific in v3). |
| Main reuse source | `packages/d2b-session-unix/` (commit `a1cc0b2d`): `UnixSeqpacketTransport`, `UnixStreamTransport`, `PeerIdentityPolicy`, `UnixAttachmentPayload`, `OwnedUnixAttachment`; `CreditPool`/`CreditScopeSet`/`CreditBundle`/`ProcessCreditLimit`/`CreditScope` (6 scopes); `PeerCredentials`/`PidfdIdentityPolicy`/`DescriptorPolicy`/`VerifiedPacket`/`ObjectIdentity`; `SeqpacketSocket`/`StreamSocket`; `tests/unix_session.rs` (all 12+ test functions). |
| Reuse action | adapt |
| Destination | `packages/d2b-bus/src/transport/unix.rs`, `packages/d2b-bus/src/transport/credit.rs` |
| Detailed design | Copy `UnixSeqpacketTransport`/`UnixStreamTransport`/credit modules verbatim; adapt `PeerIdentityPolicy` for v3 Zone principal model; adapt `InheritedSocketTransport` to receive allocator-issued FD directly (not SD_LISTEN_FDS); vsock paths adapted as transport-Provider-specific implementations (not hardcoded); `PidfdIdentityPolicy` adapted for v3 Process resource pidfd model Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | Unix transport Provider instantiates `UnixSeqpacketTransport`; vsock transport Provider instantiates vsock transports; both implement `OwnedTransport` consumed by `SessionEngine` in ADR046-routing-007 |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Port all `unix_session.rs` tests; add allocator-issued FD handoff test; add inherited-socket no-SD-listen test |
| Removal proof | Not applicable (new infrastructure) |

### ADR046-routing-009

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-009` |
| Dependency/owner | ADR046-routing-007; contracts owner |
| Current source | None in v3 pre-ADR45 baseline. |
| Main reuse source | `packages/d2b-contracts/src/v2_component_session.rs` (commit `a1cc0b2d`): all protocol constants (`PREFACE_LEN=16`, `MAX_ACTIVE_NAMED_STREAMS=128`, all limit constants), `ComponentSessionPreface`, `HandshakeOffer`/`EndpointPolicy`/`EndpointPolicyIdentity`, `NoiseProfile`, `LimitProfile`, `AttachmentPolicy`/`AttachmentDescriptor`/`AttachmentKind`/`AttachmentCreditClass`, `ChannelId`/`RecordHeader`/`FragmentHeader`, `SessionErrorCode`/`CloseReason`/`Remediation`, `BoundedVec<T,MIN,MAX>`, `BinaryError`/`ContractError`; `closed_enum!` macro; `v2_component_session` test coverage via `noise_vectors.rs`. |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/zone_session.rs` |
| Detailed design | Copy all protocol constants verbatim; copy `BoundedVec`, `BinaryError`, `ContractError`; copy `ComponentSessionPreface`, `LimitProfile`, `AttachmentDescriptor`/`AttachmentKind`/`AttachmentCreditClass`/`ChannelId`/`RecordHeader`/`FragmentHeader`/`SessionErrorCode`/`CloseReason`/`Remediation` verbatim; extend `ServicePackage` closed-enum with `ZoneV3`/`ResourceV3`/`ZoneLinkV3` variants at new tag values; extend `EndpointRole` with `ZoneController`/`ZoneRelay`/`ZoneBootstrap` variants; extend `EndpointPurpose`/`PurposeClass` with v3 Zone purposes; strip `GUEST_SESSION_CREDENTIAL_*` constants and types; re-freeze protobuf field numbers for v3 services independently from v2 assignments Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | All d2b-bus session/transport code imports from `d2b-contracts::v3::zone_session` |
| Data migration | None (new contract module) |
| Validation | Updated `negotiate_offer`/`validate_exact` round-trip tests for v3 purposes; canonical encoding stability test; closed-enum exhaustiveness tests |
| Removal proof | v2 contracts remain; v3 module is additive |

### ADR046-routing-010

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-010` |
| Dependency/owner | ADR046-routing-007, ADR046-routing-009; resource-client owner |
| Current source | None in v3 pre-ADR45 baseline. |
| Main reuse source | `packages/d2b-client/` (commit `a1cc0b2d`): `Client<R,C,W>`, `ConnectedClient`, `CallOptions`/`CancellationToken`/`RetryPolicy`/`MetadataInput`, `Response`; `ServiceHandle`/`ServiceKind`/`GeneratedClient`/`MethodHandle`; `ConnectedSession`/`ComponentSessionConnector`/`SessionFailure`/`StreamDispatcher`; `ServiceOwner`/`TargetInput`/`TransportKind`/`ResolvedTarget`/`RouteRecord`/`RouteTable`/`TargetResolver`/`TransportSelection`; `DaemonClient`/`GuestClient`; `HostSocketConnector`/`local_daemon_endpoint_identity`; `ClientError`/`RemoteErrorKind`/`RetryClass`; `tests/client.rs` (all test functions). |
| Reuse action | adapt |
| Destination | `packages/d2b-resource-client/` |
| Detailed design | Copy `Client<R,C,W>` generic structure; rename `ServiceOwner::Realm(RealmId)` → `ServiceOwner::Zone(ZonePath)`, `ServiceOwner::Workload{realm,workload}` → `ServiceOwner::Guest{zone,guest}`, `ServiceOwner::LocalRoot` → `ServiceOwner::ZoneLocal`; rename `TargetInput` variants to match; add `TargetInput::ZoneService(ZonePath, ZoneServiceKind)` for cross-Zone service targeting; replace `ServiceKind` (25 ADR45 variants) with v3 service inventory (`Resource`, `Zone`, `ZoneLink`, `Provider`, plus retained guest/daemon variants); adapt `RouteTable` to route by `ZonePath`; replace `HostSocketConnector` uid-based trust with allocator-issued FD + KK static key pinning; keep `SessionFailure`/retry/cancellation/`MetadataInput`/`RetryPolicy` logic verbatim Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | Zone runtime uses `ResourceClient` for all cross-Zone ResourceType calls; d2b-bus wraps `ComponentSessionDriver` (ADR046-routing-007) as the underlying session |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Port `client.rs` tests; add ZonePath routing test; add cross-Zone K0→K1 end-to-end test; add retry/cancellation forwarding test |
| Removal proof | v2 `d2b-client` package remains for ADR45 callers; v3 `d2b-resource-client` is additive |

### ADR046-routing-014

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-014` |
| Dependency/owner | ADR046-routing-007; Provider resource owner |
| Current source | None in v3 pre-ADR45 baseline (provider traits exist in `d2b-realm-provider` but are unregistered). |
| Main reuse source | `packages/d2b-provider/src/` (commit `a1cc0b2d`): `ProviderRegistry`/`ProviderRegistryBuilder`/`ProviderRegistryManager`; `RegistryLimits`/`AdmissionOptions`/`InFlightPermit`/`AdmittedProvider`; `AuthenticatedProviderRpc` (trait)/`RpcProviderProxy`/`RpcCall`/`RpcResponse`/`RpcOperation`/`RpcPayload`/`SessionIdentity`; `ProviderInstance` (11 variants); `OwnedOperationContext`/`ProviderCallContext`; `ProviderResult<T>`/`ProviderFailure`/`ProviderRuntimeError`; `packages/d2b-contracts/src/v2_provider.rs`: `ProviderDescriptor`/`ProviderCapabilitySet`/`ProviderHealth`/`ProviderAuthority`/`ProviderPlacement`/`AgentPlacementBinding`/`ProviderOperationContext`/`ProviderCallContext` etc.; inline registry tests (`prove_final_drop_between_check_and_await_completes`, `shutdown_closes_final_permit_notify_race`). |
| Reuse action | adapt |
| Destination | `packages/d2b-provider/src/` (adapted in place) |
| Detailed design | Retain `ProviderRegistry`/`ProviderRegistryBuilder`/`ProviderRegistryManager` lifecycle verbatim; adapt `SessionIdentity` to carry `ZonePath` instead of `RealmId`; adapt `AdmissionOptions::peer_role` to v3 Zone principal + RBAC binding; adapt `ProviderDescriptor` schema version to v3; `RegistryLimits` unchanged; `RpcProviderProxy` field adaptations to match v3 session identity; `ProviderInstance` variants retain all 11 types Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | Zone runtime `ProviderComposition` builds a `ProviderRegistry` per Zone; Provider resource controller admits calls through `ProviderRegistry::admit()` |
| Data migration | None (pure runtime) |
| Validation | Port inline registry lifecycle/drain/shutdown tests; add v3 ZonePath routing test; prove provider admission cannot self-assert relay and each forward requires relay plus the target verb |
| Removal proof | Provider registry is v3 core infrastructure; no retirement |

### ADR046-routing-015

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-015` |
| Dependency/owner | ADR046-routing-014; Provider agent process owner |
| Current source | None in v3 pre-ADR45 baseline. |
| Main reuse source | `packages/d2b-provider-toolkit/src/` (commit `a1cc0b2d`): `GeneratedProviderServiceServer`/`ProviderAgentProcess`; `ProviderAgentAdapter`; `check_descriptor_conformance`/`check_provider_conformance`/`ConformanceError`; `register_exact_instances`/`ToolkitError`; redaction helpers; `packages/d2b-gateway-runtime/src/provider_agent.rs`: `ProviderAgentProcess::from_registry`/`from_registry_with`, `MAX_DISPATCH_IN_FLIGHT=64`, `DEFAULT_AUDIT_CAPACITY=1024`, `ProviderAgentAuditEvent`/`ProviderAgentError`; `run_registered`/`run`; test `audit_capacity_is_closed_and_bounded`. |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-toolkit/src/` (adapted in place) |
| Detailed design | Retain `GeneratedProviderServiceServer` ttrpc dispatch verbatim; adapt `ProviderAgentProcess::from_registry` to receive ComponentSession FD from Zone allocator bootstrap instead of SD_LISTEN_FDS; adapt audit event types for v3 Zone principal; `ProviderAgentAdapter` (client-side proxy) adapted for v3 ZoneLink session; conformance kit extended for v3 Provider resource conformance checks; redaction helpers unchanged Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | Provider Processes (EphemeralProcess or Process resources with `executionRef`) spawn the provider agent entrypoint; Zone bus instantiates `ProviderAgentAdapter` as the proxy inside the Zone runtime |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | Port `audit_capacity_is_closed_and_bounded`; add v3 bootstrap-via-allocator test; add conformance test for new Provider ResourceType schema |
| Removal proof | Provider toolkit is v3 core infrastructure; no retirement |

### ADR046-routing-016

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-016` |
| Dependency/owner | ADR046-routing-007, ADR046-routing-002, ADR046-routing-004, ADR046-routing-012; Zone service owner |
| Current source | `packages/d2b-realm-router/src/service_v2.rs` (v3 baseline `b5ddbed6`): `RealmServiceServer` (bootstrap/enroll/resolve_route/authorize_shortcut/revoke_shortcut/report_shortcut_close/inspect/cancel), `RealmServiceProcess`, `RealmSessionAuthority`, `CredentialCustody`, `RealmServiceLimits`, `RealmAuditEvent`/`RealmMethod`/`RealmAuditOutcome`, `BootstrapBinding`/`EnrollmentBinding`/`ShortcutBinding`/`MutationRecord`; constants `DEFAULT_MAX_REALM_BINDINGS=256`, `DEFAULT_MAX_SHORTCUTS=256`, `DEFAULT_MAX_MUTATION_RECORDS=1024`, `DEFAULT_AUDIT_CAPACITY=1024`, `MAX_CONFIGURED_BOUND=4096`, `MAX_DISPATCH_IN_FLIGHT=64`, `SHUTDOWN_TIMEOUT=5s` (evidence: v3 baseline, not main; see Baseline section - **B** from d2bd/CLI perspective, **A** within realm-router display-session use) |
| Main reuse source | `packages/d2b-realm-router/src/service_v2.rs` (commit `a1cc0b2d`): same symbols, unchanged from v3 baseline in the main commit. All evidence class notes apply equally to main. |
| Reuse action | adapt |
| Destination | `packages/d2b-zone-routing/src/service.rs` |
| Detailed design | Rename `RealmServiceServer` → `ZoneServiceServer`; service wire name `d2b.realm.v2.RealmService` → `d2b.zone.v3.ZoneService`; rename methods (bootstrap→zone-bootstrap, enroll→zone-enroll, resolve_route→resolve-zone-route, authorize_shortcut→authorize-zone-shortcut, revoke_shortcut→revoke-zone-shortcut, report_shortcut_close→report-zone-shortcut-close, inspect→zone-inspect) and add list/watch topology-projection methods. The read-only projection starts from the sealed sorted `{ childZone, parentZone }` compiler input and joins only authenticated, admitted `ZoneRouteEngine` route/projection status. It exposes no ZoneLink resource name, UID, spec, status, Provider ref, fingerprint, transport setting, or handle. Replace `RealmSessionAuthority` with Zone principal + RBAC binding; replace `BootstrapBinding` with allocator-issued PSK binding associated with the child's local ZoneLink; replace `EnrollmentBinding` with the corresponding KK enrollment record; add independent target-verb plus canonical `relay` RBAC checks per forwarding hop; adapt shortcut model to ZonePath addressing; `RealmServiceLimits` defaults preserved; `MAX_DISPATCH_IN_FLIGHT=64`, `SHUTDOWN_TIMEOUT=5s` preserved; `CredentialCustody::GatewayGuest` excluded (all ZoneLink sessions are direct KK) Primary reuse disposition: `adapt`. Preserved source-plan detail: copy and adapt. |
| Integration | Zone runtime instantiates one `ZoneServiceServer` per Zone; d2b-bus routes `d2b.zone.v3.ZoneService` calls to this server; CLI uses `ZoneServiceClient` (from ADR046-routing-010) for topology list/inspect/watch, enrollment, and route resolution |
| Data migration | None; v3 Zone service is new; no v2 realm-service compatibility |
| Validation | Bootstrap/enroll/resolve-route/shortcut integration tests against a child-local fake ZoneLink; topology list/inspect/watch golden vectors contain exact `{ childZone, parentZone }` rows plus authenticated status and no ZoneLink fields; stale/withdrawn/unauthenticated projection tests; parent-store no-row/no-handler test; relay-plus-target-verb RBAC tests; KK enrollment test; shortcut ZonePath addressing test; concurrent dispatch bound test (64 in-flight) |
| Removal proof | `RealmServiceServer` on `d2b.realm.v2` retires after `ZoneServiceServer` handles all routing; display-session path migrates separately as part of Provider resource work |

### ADR046-routing-011

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-011` |
| Dependency/owner | ADR046-routing-001; Nix module owner |
| Current source | `nixos-modules/options-realms-workloads.nix` (v3 baseline `b5ddbed6`): `d2b.realms.<name>.*` option declarations (evidence: **C** - eval contract; bespoke field names that do NOT mirror canonical ResourceSpec); `nixos-modules/assertions.nix`: realm-name regex, platform-gate, CIDR assertions (evidence: **A**); `nixos-modules/realm-controller-config-json.nix`: `realmControllersJson` emitter, bundle.nix:59 wiring (evidence: **C**) |
| Reuse source | Same v3 baseline `b5ddbed6`; `assertions.nix` pattern reused for Zone assertions; `realm-controller-config-json.nix` is the structural template |
| Reuse action | adapt |
| Destination | `nixos-modules/options-zones.nix` (new structural base), `nixos-modules/generated/resource-types.nix` (generated registry), `nixos-modules/generated/options-zones-<Type>.nix` (generated per ResourceType by `xtask gen-zone-nix-options`), `nixos-modules/assertions.nix` (new Zone assertions) |
| Detailed design | Declare compiler-only scalar `d2b.zones.<zone>.parentZone` plus structural `d2b.zones.<zone>.resources.<name> = { type = ...; spec = {}; }` as specified in the "Option schema" section above. `parentZone` has no default, is required for every non-root Zone, forbidden on `local-root`, resolves to one declared Zone, and never enters a ResourceSpec. Wire `options-zones.nix` and all `generated/options-zones-*.nix` files into `nixos-modules/default.nix`. Add a new `xtask gen-zone-nix-options` command that reads `docs/reference/schemas/v3/<Type>.schema.json` for each ResourceType, derives `generated/resource-types.nix`, and emits a generated submodule overlaying typed spec options (types, bounds, enum constraints, defaults, docs) onto `d2b.zones.<zone>.resources.<name>.spec`. The generated registry's standard subset must equal the canonical 19-type registry from `ADR-046-resource-object-model` exactly; installed signed Provider schemas may append qualified types. These generated modules are committed and kept in sync by `xtask gen-zone-nix-options && git diff --exit-code` wired into `make test-drift`. The ZoneLink module is generated from the exact six-field schema owned by ADR046-zone-control-002 and rejects any seventh field. Because the generated options carry field-level type constraints, field-level eval errors (wrong enum, out-of-range int, malformed ref) are caught without explicit assertions. Explicit assertions in `nixos-modules/assertions.nix` cover cross-resource invariants only: zone/resource key name regex, reserved names, `parentZone` required/forbidden/existence/self/cycle/16-name-depth constraints, child-local `childZoneName == zone`, at most one uplink resource per non-root Zone and none in local root, ref resolution, count limits, and transportSettings secret-key exclusion (listed in the eval-time assertions table). Conflicting parent scalar definitions fail through standard Nix module merging. Each new assertion must have a matching case in `tests/unit/nix/cases/zone-assertions.nix` (nix-unit auto-discovered). `d2b.realms` option namespace is NOT removed in this work item. Primary reuse disposition: `adapt`. Preserved source-plan detail: new module following same pattern. |
| Integration | ADR046-routing-012 consumes the validated `parentZone` map for private allocator bootstrap sealing and iterates `d2b.zones.<zone>.resources.*` for resource-bundle emission |
| Data migration | None; Zone options are new; Realm options retained until migration PR |
| Validation | `nix-unit: zone-name-regex`, all five `nix-unit: zone-parent-*` vectors, `nix-unit: zone-link-credential-ref`, `nix-unit: zone-link-child-name`, `nix-unit: zone-link-one-uplink`, `nix-unit: zone-link-closed-spec`, `nix-unit: zone-link-limits`, `nix-unit: transport-settings-secret-key`; add `drift: standard-resource-type-registry` asserting the generated standard subset is exactly all 19 canonical types with no omission/addition/duplicate/reordering, plus `drift: zone-nix-options` (`xtask gen-zone-nix-options && git diff --exit-code`); run `make nix-unit-pin` after adding eval cases |
| Removal proof | `nixos-modules/options-realms-workloads.nix` `d2b.realms` namespace retires after all hosts migrate to `d2b.zones` |

### ADR046-routing-012

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-012` |
| Dependency/owner | ADR046-routing-011, ADR046-routing-001; bundle emitter owner |
| Current source | `nixos-modules/realm-controller-config-json.nix` (v3 baseline `b5ddbed6`): `builtins.toJSON` emitter for `realm-controllers.json` (bundle.nix:59); `nixos-modules/bundle-artifacts.nix`: install table (root:d2bd 0640); `nixos-modules/bundle.nix`: artifact wiring (evidence: **C**); `packages/xtask/src/main.rs` `gen-schemas` subcommand (evidence: **A** - wired into `make test-drift`) |
| Reuse source | `realm-controller-config-json.nix` structural template; `xtask gen-schemas` extension point (main `a1cc0b2d` unchanged in this area) |
| Reuse action | adapt |
| Destination | `nixos-modules/zone-resources-json.nix` (new), private local-root allocator bootstrap compiler/sealer input (not a ResourceSpec or public bundle), `nixos-modules/bundle-artifacts.nix` (new row for per-Zone `resource-bundle.json`), `packages/xtask/src/main.rs` (`gen-zone-schemas` subcommand emitting `docs/reference/schemas/v3/<Type>.schema.json` for Zone and ZoneLink; `gen-zone-nix-options` subcommand emitting `nixos-modules/generated/options-zones-<Type>.nix`) |
| Detailed design | `zone-resources-json.nix` iterates `d2b.zones.<zone>.resources.*` to produce the canonical sorted resource list: for each entry, render `{ apiVersion, type, metadata: { name, zone, ownerRef: <if-authored>, labels: <if-authored>, annotations: <if-authored> }, spec: <spec-attrs-canonical> }`. Separately canonicalize sorted `{ childZone, parentZone }` rows from the compiler-only topology and seal them into the private allocator bootstrap input; `parentZone` never enters a resource bundle or `Zone.spec`, and a topology digest change releases/reallocates affected edges independently of resource `generationId`. Per-Zone generation is strict: local root's generated bundle contains no ZoneLink; a non-root Zone's enabled uplink and referenced transport Provider appear together only in that child's bundle; no emitter copies either resource into the selected parent's bundle. The bundle JSON omits `managedBy` and `configurationGeneration`; the configuration service/core sets those fields when activating the validated bundle. Sort all resources by `(type, zone, name)`. Compute `generationId` as SHA-256 (lower hex) of the UTF-8 bytes of the sorted `resources` array JSON. Compute `integrity` as SHA-256 (base64url, no padding) of the full bundle JSON with integrity field zeroed. Install at `/etc/d2b/zones/<zone>/resource-bundle.json` root:d2bd 0640. Canonical form: all object keys sorted lexicographically; order-significant arrays preserved; schema-declared set-like arrays sorted lexicographically; all optional fields emitted with defaults; no field renaming or restructuring. Build-time validation runs in a Nix derivation: (1) validate the complete parent map (non-root required, local-root forbidden, declared target, one scalar parent, not self, acyclic, max 16 names); (2) validate each resource against the committed JSON Schema, including the exact six-field ZoneLink schema from ADR046-zone-control-002; (3) validate `transportSettings` for each child-local ZoneLink against its same-Zone Provider's `transportSettingsSchema` - `transportProviderRef` is always explicit, never inferred or defaulted; (4) resolve every same-Zone `transportCredentials` ref; (5) verify `childZoneName == metadata.zone`, at most one uplink resource per non-root Zone, and no local-root uplink; (6) check for duplicate `(type, zone, name)` tuples. Private route capability policy is sealed in allocator bootstrap state and is not a ZoneLink ResourceSpec field. Providers MUST commit their `transportSettingsSchema` before any ZoneLink can reference them. Drift gates: `xtask gen-zone-schemas && git diff --exit-code` and `xtask gen-zone-nix-options && git diff --exit-code` both wired into `make test-drift`. Add `checks.${system}.zone-schema-drift` to `flake.nix`. Primary reuse disposition: `adapt`. Preserved source-plan detail: extend and adapt. |
| Integration | The local-root allocator consumes sealed parent topology independently of resource bundles; `nixos-modules/bundle-artifacts.nix` installs each per-Zone `resource-bundle.json`; ADR046-routing-013 Zone runtime reads it on startup |
| Data migration | None; new artifact file |
| Validation | `drift: zone-resource-schema`, `drift: zone-nix-options`, `build: zone-bundle-deterministic`, `build: parent-topology-sealed`, `build: child-local-zonelink-bundle` (K0 has no ZoneLink; K1 contains its self-matching ZoneLink and same-Zone transport Provider; neither is copied to K0), `build: zone-link-exact-six-fields`, `build: transport-settings-unknown-field`, `build: transport-credential-ref`, `build: missing-transport-provider`; run `make flake-matrix-pin` after adding flake checks |
| Removal proof | `realm-controllers.json` artifact retires after Zone runtime is live and all hosts migrated |

### ADR046-routing-013

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-routing-013` |
| Dependency/owner | ADR046-routing-012, ADR046-routing-003; `d2b-core-controller` owner (ADR-046-core-controllers) |
| Current source | `packages/d2b-realm-core/src/realm_controller_config.rs`: `RealmControllersJson`, `RealmControllerRow`, `RealmControllerConfigError` (evidence: **C** - loaded but routing inert); `packages/d2bd/src/realm_access_resolver.rs`: `resolve_local_root_realm_access()`, `RealmAccessResolverRequest`, `RealmAccessBinding` (evidence: **B** - pub mod at lib.rs:117, no callers); `packages/d2b-state/src/` (both baselines): atomic state, OFD locks, lease primitives (evidence: **A** for locks/leases, **B** for realm-specific storage); `nixos-modules/host-daemon.nix:220–221`: bundle artifact install paths, daemon SIGHUP wiring (evidence: **A**) |
| Main reuse source | `packages/d2b-state/src/` (main `a1cc0b2d`): atomic state, audit segment primitives adapted for generation tracking |
| Reuse action | adapt |
| Destination | `packages/d2b-core-controller/src/configuration.rs` (defined by ADR-046-core-controllers); shared bundle DTOs may live in `packages/d2b-core/` |
| Detailed design | Implement the configuration ownership and cleanup contract from the "Configuration ownership and cleanup contract" section. `configuration.rs` owns: (1) reading and integrity-verifying `/etc/d2b/zones/<zone>/resource-bundle.json` on startup and SIGHUP; (2) diffing against active generation by `generationId` (no-op if unchanged); (3) queuing Create/UpdateSpec/Delete intents - core sets `configurationGeneration` and `managedBy` when applying Create/UpdateSpec; Delete targets only resources where BOTH `managedBy` equals the configuration service's value AND `configurationGeneration` matches the prior bundle - resources with `managedBy=controller` or `managedBy=api` are never seized; (4) setting `deletionRequestedAt` on pending-delete resources immediately and adding a Pending condition; (5) writing the prior bundle into the capped ring at `/var/lib/d2b/zones/<zone>/configuration/prior/<gen-id>.json` (default retentionCount=3, range 1..16, no TTL; prune oldest when count would exceed limit); (6) enforcing boundary invariants (no diff-delete for absent `configurationGeneration`, `managedBy` collision guard, live controller-child teardown guard); (7) driving finalizer drain + controller-child cascade before completing a Delete; (8) on successful deletion: one store transaction writes the `Deleted` revision/change event and removes the resource row and all index entries; the authoritative audit record (`zone-resource-cleanup`) is appended from the committed revision with dedup/exactly-once recovery and is NOT part of the store transaction; (9) tracking `deletionRequestedAt`/`cleanupConfigGeneration`/`cleanupError`/`cleanupAttempt` per resource; (10) on rollback: clearing `deletionRequestedAt` and Pending condition for revived resources; (11) never pruning a prior bundle while a Delete intent from its `configurationGeneration` is in flight. OFD lock on the bundle file prevents concurrent activation races. Generation state persisted atomically at `/var/lib/d2b/zones/<zone>/configuration/generation.json` (root:d2bd 0640). The `spec` object comparison for UpdateSpec detection uses the canonical JSON form so two identical specs always compare equal regardless of Nix rendering order. Resource phase transitions: Pending while Create/UpdateSpec in-flight; Degraded while cleanup pending; Ready when clean; Failed on permanent error. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | `d2b-core-controller` configuration service activates on bundle install and SIGHUP; zone-controller reconcile loops in `d2b-core-controller` consume the queued intents; d2b-bus resource API exposes `status.phase` and `pendingCleanup` via Get/Watch on the active generation resource |
| Data migration | None; new runtime component |
| Validation | `host-integration: cleanup-removed-zonelink`, `host-integration: rollback-restores-zonelink`, `host-integration: dynamic-child-not-deleted`, `host-integration: zonelink-no-reciprocal-row`; unit tests: deterministic generationId, no-op on same generationId, cross-ownership invariant enforcement, prior-bundle write/prune cycle, UpdateSpec canonical comparison, store-transaction-then-audit-append ordering, exactly-once audit dedup |
| Removal proof | `realm_access_resolver.rs` (B) retires after `d2b-core-controller` configuration tracking is live; `RealmControllersJson` (C) retires after all hosts migrated to Zone bundles |
