//! `d2b-realm-core` is the pure, codec-neutral v2 realm model. The realm-native
//! supersedes ADR 0032's host-centric entrypoint model while preserving the
//! semantic operation, stream, capability, idempotency, relay-as-reachability,
//! and bounded-audit invariants. The crate defines identifiers, realm targets,
//! realm-controller DTOs, provider/workload placement summaries, routing and
//! enrollment metadata, the persistent-shell contract, the audit envelope, the
//! semantic `ConstellationFrame`, a bounded `TraceContext`, and the typed error
//! surface.
//!
//! Invariants:
//!
//! - `#![forbid(unsafe_code)]` (inherited via workspace lints).
//! - **No** dependency on `prost`, generated protobuf, any
//!   `d2b-realm-codec-*`, any transport crate, or any
//!   host-only broker/daemon internals. Codecs map bytes to/from the
//!   semantic [`frame::ConstellationFrame`]; the operation/routing layer
//!   never depends on a wire encoding.
//! - DTOs are `serde` + `schemars` and security-sensitive structures use
//!   `deny_unknown_fields` (ADR 0010 strict wire discipline).

pub mod access;
pub mod allocator;
pub mod allocator_engine;
pub mod audit;
pub mod capability;
pub mod enrollment;
pub mod error;
pub mod execution;
pub mod frame;
pub mod ids;
pub mod migration;
pub mod mux;
pub mod node;
pub mod payload;
pub mod realm;
pub mod registry;
pub mod routing;
pub mod shell;
pub mod stream;
pub mod target;
pub mod token;
pub mod trace_context;
pub mod workload;

pub use access::{
    AccessBindingRef, CapabilityPreflightDenialReason, CapabilityPreflightStatus,
    DefaultRealmSelectionMetadata, DefaultRealmSelectionSource, HostLocalPeerCredentialChecker,
    HostLocalPeerCredentialSemantics, HostLocalPeerCredentialSource, HostLocalProxyStatus,
    RealmAccessAliasBinding, RealmAccessAliasSource, RealmAccessBinding,
    RealmAccessCapabilityPreflight, RealmAccessClientBinding, RealmAccessClientBindingKind,
    RealmAccessClientContract, RealmAccessConflictCandidate, RealmAccessResolverDiagnostic,
    RealmAccessResolverError, RealmAccessResolverRequest, RealmAccessResolverResponse,
    RealmAccessTargetInput, RealmTransportBinding, UnixSocketPath,
};
pub use allocator::{
    ALLOCATOR_REASON_CODE_COUNT, AllocatorConflict, AllocatorEventKind, AllocatorEventMetadata,
    AllocatorLease, AllocatorLeaseState, AllocatorReasonCode, GrantedHostResource,
    HostResourceKind, LeaseAllocationRequest, LeaseAllocationResponse, LeaseAllocationResult,
    LeaseOwner, LeaseResourceRequest, ObservedHostResource, ObservedResourceState,
    PersistedResourceLease, ReconciliationDecision, ReconciliationRecord, ReconciliationReport,
    ResourceAcquisitionKey, ResourceAcquisitionOrder, ResourceDelegation,
    ResourceObservationSource, ResourceShareMode,
};
pub use allocator_engine::{
    AllocatorAllocationDecision, AllocatorEngineAllocation, AllocatorEngineDecision,
    AllocatorEngineOutcome, AllocatorEngineReconciliation, AllocatorMetricEvent,
    AllocatorMetricLabels, AllocatorReconciliationAction, FakeAllocatorLedger,
    FakeAllocatorLiveness, FakeObservedAllocatorState, LocalRootAllocatorEngine,
};
pub use audit::{AdmissionAuditRecord, AuditEnvelope, AuthorizationScope, AuthzDecision};
pub use capability::{Capability, CapabilityNegotiation, CapabilitySet};
pub use enrollment::{
    EnrollmentRecord, EnrollmentStatus, KeyFingerprint, KeyPin, RealmKeyRole, RevocationRecord,
    RevocationStatus, RevocationTarget,
};
pub use error::{ConstellationError, ErrorKind};
pub use execution::{
    ExecAttachMode, ExecAttachRequest, ExecCancelRequest, ExecLogsRequest, ExecStartRequest,
    ExecState, ExecutionGeneration, ExecutionSummary,
};
pub use frame::{
    ConstellationFrame, Handshake, HandshakeAccepted, HandshakeRejected, HandshakeRejectedReason,
    OperationKind, OperationRequest, OperationResponse, PeerContext, StreamClose, StreamData,
    StreamFlow, StreamOpen, StreamResume,
};
pub use ids::{
    AllocatorLeaseId, ControllerGenerationId, CorrelationId, EnrollmentId, ExecutionId, GatewayId,
    HostResourceId, IdempotencyKey, NodeId, OperationId, PrincipalId, ProviderId, RealmId,
    RevocationId, RouteId, StreamCursor, StreamId, WorkloadId,
};
pub use migration::{
    LegacySurface, MigrationErrorEnvelope, MigrationLegacyId, MigrationReasonCode,
};
pub use mux::{DEFAULT_MAX_OPEN_STREAMS, StreamMux};
pub use node::{NodeKind, NodeSummary};
pub use payload::OpaquePayload;
pub use realm::{EntrypointMode, RealmControllerPlacement, RealmPath};
pub use registry::{ProviderRegistryEntry, WorkloadPlacement, WorkloadPlacementSummary};
pub use routing::{
    DescendantRoute, RealmTreeEdge, RouteAdvertisement, RouteSignature, SignatureRef,
};
pub use shell::{
    ShellAttachId, ShellAttachRequest, ShellAttachSummary, ShellCause, ShellDetachRequest,
    ShellEventBatch, ShellEventSummary, ShellGeneration, ShellKillRequest, ShellListRequest,
    ShellListResponse, ShellName, ShellNameError, ShellOpaqueIdError, ShellSessionInstanceId,
    ShellState, ShellSummary,
};
pub use stream::{StreamAuthz, StreamChannel, StreamCloseReason, StreamDescriptor, StreamKind};
pub use target::{
    LegacyNodeQualifiedTarget, RealmTarget, RealmTargetParseError, RealmTargetParser,
    TARGET_SUFFIX, THIS_NODE_ALIAS, TargetName, TargetParseError,
};
pub use token::ProtocolToken;
pub use trace_context::TraceContext;
pub use workload::{WorkloadSelector, WorkloadState, WorkloadSummary};
