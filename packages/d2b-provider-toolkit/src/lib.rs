//! The Provider authoring framework.
//!
//! Every Provider in the frozen catalog is an independently buildable crate
//! that binds one or more ResourceTypes, runs as one or more Processes, and
//! reaches host state only through an injected effect port. This crate owns
//! the provider-neutral half of that, as a framework rather than a menu: a
//! provider implements [`ProviderBase`], states its entrypoint facts, and
//! calls [`run`]. The bootstrap, readiness, admission, plane attach,
//! startup order, service loop, operation envelope, audit ring, drain
//! ordering, and the test harness are toolkit-owned, so two providers cannot
//! diverge on any of them.
//!
//! The layout is the framework:
//!
//! - [`declaration`] - the declaration vocabulary every provider publishes,
//!   plus the canonical manifest and root-schema emitters.
//! - [`base`] - [`ProviderBase`], [`run`]/[`run_guest`], and the lifecycle
//!   driver behind them.
//! - [`server`] - the authenticated service loop: frame codec, bounded
//!   dispatch adapter, readiness handshake, drain.
//! - [`service`] - the provider service contract: the envelope's real
//!   payload and the capability object built from a method's declared
//!   facets, which the daemon hosts and providers implement.
//! - [`shared_provider`] - the shared host-provider driver machinery: the
//!   declaration row shape, the effect request and outcome vocabulary, the
//!   manager-routed child surface, and the driver every shared family's
//!   descriptor registers. The families own their rows, children, dependency
//!   references, and effect ports; the flow they share lives here.
//! - [`credential`] - the Credential realization the three Credential
//!   Provider binaries share: the one audit record, the one telemetry frame,
//!   and the one dispatch seam, keyed on the shared Provider kind.
//! - [`operations`] - the envelope-side types and the envelope that runs a
//!   declared handler.
//! - [`plane`] - the zone-plane handle, the child-creation fence, the drain
//!   deadline, and the cause-carrying reconcile seam.
//! - [`audit`] - the bounded audit ring and the redaction wrapper.
//! - [`testing`] - the harness, fakes, faults, deterministic clock, and the
//!   conformance kit.
//!
//! What this crate deliberately does not do, because
//! `ADR-046-provider-model-and-packaging` forbids it for a Provider and for
//! a common Provider library:
//!
//! - It registers no Provider identity of its own and composes no Provider.
//!   It is a common library, so it can never become a hidden multi-Provider
//!   binary.
//! - It performs no privileged mutation. It opens no broker, D-Bus, or
//!   systemd socket, resolves no host path, spawns no process, and offers no
//!   direct-effect escape. A Provider validates semantics and calls its own
//!   injected typed effect port, which the fixed core effect adapter alone
//!   implements; the broker stays the sole privileged executor and
//!   independent audit owner of every host mutation.
//! - It defines no type that carries authority. The identity a bootstrap
//!   check returns names who the agent is so it can label an audit event
//!   and refuse a Zone it was not placed in; it authorizes no call, route,
//!   or effect. Authorization stays with ComponentSession admission and the
//!   Zone RBAC binding.
//! - It imports no daemon, broker, Zone-store, Nix-emitter, or Provider
//!   implementation internals. It depends on the shared v3 contract catalog,
//!   the neutral Provider registry SDK, the declaration vocabulary, and the
//!   transport-agnostic ComponentSession driver.
//!
//! No file descriptor, numeric UID or GID, device node, store path, socket
//! path, or host path appears in any type here. A bootstrap binding names a
//! Zone path, a `Provider/<name>` reference, a session purpose, and an
//! opaque channel-binding digest, and nothing else.

#![deny(missing_docs)]

pub mod audit;
pub mod base;
pub mod credential;
pub mod declaration;
pub mod operations;
pub mod plane;
pub mod server;
pub mod service;
pub mod shared_provider;
pub mod testing;

pub use audit::{
    DEFAULT_AUDIT_CAPACITY, ProviderAgentAuditEvent, ProviderAgentAuditLog,
    ProviderAgentAuditOutcome, Redacted,
};
#[cfg(feature = "unix-transport")]
pub use base::fd10::{
    CredentialDeliveryKeyHandoff, CredentialDeliveryKeyMaterial, CredentialSensitiveBytes,
    GUEST_CREDENTIAL_BACKEND_FD, GUEST_CREDENTIAL_BACKEND_PROTOCOL,
    GUEST_CREDENTIAL_BACKEND_SERVICE, GuestCredentialBackend, GuestCredentialBackendError,
    GuestCredentialBackendHandler, GuestCredentialBackendHandlerError,
    GuestCredentialBackendHandlerFuture, GuestCredentialBackendReply,
    GuestCredentialBackendResponderLease, GuestCredentialBackendResponse,
    PROVIDER_BOOTSTRAP_STREAM_CREDIT, PROVIDER_BOOTSTRAP_STREAM_ID,
    PROVIDER_DELIVERY_KEY_STREAM_CREDIT, PROVIDER_DELIVERY_KEY_STREAM_ID, ProviderFd10Spec,
    ProviderSessionMetadata, SupervisedRoute, establish_supervised_route, run_from_fd10,
    spawn_guest_credential_backend_responder, zeroizing_bytes,
};
pub use base::{
    AllocatorEnrollment, AllocatorSessionBinding, AttachError, AuthenticatedRoute,
    DEFAULT_DRAIN_BUDGET_MS, DrainError, EnrolledRoute, EnrollmentRequest, GUEST_RECONNECT_ATTEMPTS,
    GUEST_RECONNECT_INITIAL_MS, GUEST_RECONNECT_MAX_MS, GUEST_SESSION_MAX_FRAME_BYTES, GuestAgent,
    GuestEnrollment, GuestError, GuestFrame, GuestLink, GuestLinkFuture, GuestPlacement, Lifecycle,
    PROVIDER_RESOURCE_TYPE, ProviderAdmission, ProviderAgentBootstrap, ProviderAgentIdentity,
    ProviderBase, ProviderEntrypoint, ProviderLifecycle, ProviderRunError, ProviderRuntimeError,
    ProviderSessionAdmission, ProviderToolkitError, ServiceMethods, ServiceSurface, StartupError,
    StartupPlan, StartupPlanRefusal, StartupStepError, StartupStepExecutor, SupervisedProvider,
    run, run_guest, run_with_startup,
};
pub use d2b_session::{
    AuthenticatedComponentSession, AuthenticatedSessionRouteBinding, Cancellation,
    ComponentSessionDriver, StreamEvent, StreamId,
};
pub use declaration::{
    AllowedSources, Cardinality, ChildCreation, ChildCustody, DriverDescriptor, IsolationPosture,
    MethodFdContract, OperationDef, OperationHandler, PlaneAdapter, PrincipalName,
    ProviderDeclaration, SelfBinding, ServiceDecl, ServiceMethod, StartupStep, StorageRoot,
    WellKnownType,
};
pub use operations::{
    OperationCtx, OperationEnvelope, OperationFailure, OperationResult, ValidatedPayload,
};
pub use plane::{
    ChildCreationFailure, ChildCreationFence, CreateChild, CreationRefusal, CreationTable,
    DrainDeadline, MAX_DRAIN_BUDGET_MS, MAX_REQUEUE_AFTER_MS, PlaneError, ReconcileCause,
    ReconcileCtx, ReconcileOutcome, ReconcileRefusal, ReconcileTarget, SystemClock,
    UnavailablePlanePort, ZonePlaneHandle, ZonePlanePort,
};
pub use server::{
    AuthenticatedProviderFrameCodec, AuthenticatedProviderRequest, CredentialAuthorizationSource,
    CredentialRequestMetadata, DispatchLimiter, DispatchPermit, GeneratedProviderServiceServer,
    GeneratedServiceDescriptor, MAX_DISPATCH_IN_FLIGHT, MAX_SERVER_IN_FLIGHT,
    PROVIDER_READY_MARKER, PROVIDER_READY_STREAM_CREDIT, PROVIDER_READY_STREAM_ID,
    ProviderAgentAdapter, ProviderFrameCodec, ProviderRequest, ProviderService,
    RouteCredentialAuthorization, ServerError, ServerRequestPermit, credential_service,
    run_authenticated_credential_provider, run_authenticated_provider,
    serve_authenticated_component_session, serve_authenticated_route, validate_attachment_indexes,
    validate_provider_route,
};
pub use service::{
    EffectRequest, EffectResponse, EffectService, EffectServiceError, EffectServiceFactory,
    ServiceInvocation,
};
pub use shared_provider::{
    ContextChildSurface, HOST_REF, ProviderRow, SharedProviderChildSurface,
    SharedProviderDeclarationError, SharedProviderDriver, SharedProviderDriverArgs,
    SharedProviderDriverError, SharedProviderDriverErrorKind, SharedProviderDriverFactory,
    SharedProviderDriverStatus, SharedProviderEffectError, SharedProviderEffectOutcome,
    SharedProviderEffectPhase, SharedProviderEffectRequest, SharedProviderFamily,
    SharedProviderFinalize, SharedProviderSpecDecodeError, SharedProviderSpecEnvelope,
    decode_metadata, key_ref, owner_ref, resource_uid, shared_provider_spec_decoder,
};
pub use testing::{
    AdmissionRefusal, AdmittedRow, DeterministicClock, FIXTURE_NOW_UNIX_MS, FakeBus,
    FakeCoreClient, FakeEffectPort, FakePortError, FakeProvider, FakeResourceStore, FakeSupervisor,
    FaultInjector, FaultPlan, Fixture, HarnessDeclarations, MAX_RECORDED_CALLS, PlaneCall,
    RecordingPlanePort, RowPhase, RowStatus, SampleLeaseRequest, SharedLog, TestHarness, block_on,
    sample_lease_request,
};

/// Audited Unix attachment types used by Provider-specific transport adapters.
#[cfg(feature = "unix-transport")]
pub mod unix {
    pub use d2b_session_unix::{
        AcceptedAttachment, CreditBundle, VerifiedPacket, credential_provider_endpoint_policy,
    };
}
