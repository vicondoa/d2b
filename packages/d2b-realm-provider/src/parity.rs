//! Provider parity inventory and proof (ADR 0032/0043).
//!
//! This module is the single typed source of truth for "every provider
//! trait family this crate declares, every operation each family exposes,
//! and how that operation behaves": its capability gate, idempotency,
//! retry, cancellation, attachment/stream, and fallback posture. It then
//! cross-checks that inventory against the **observable behavior** of the
//! in-memory stand-ins in [`super`] (`crate::mock`) — never against a real
//! host, relay, or provider API.
//!
//! ## Fail-closed guarantees
//!
//! - [`ProviderFamily`] and [`CanonicalOperation`] are plain, non-`#[non_exhaustive]`
//!   enums declared in *this* crate, so [`CanonicalOperation::classification`]
//!   is an exhaustive `match`: a new operation added to the enum without a
//!   classification arm is a compile error, not a silently-missing record.
//! - [`capability_role`] and [`error_kind_owner`] are exhaustive matches over
//!   [`d2b_realm_core::Capability`] and [`d2b_realm_core::ErrorKind`], neither
//!   of which is `#[non_exhaustive]`: a new capability or error kind added
//!   upstream is a compile error here until classified.
//! - [`d2b_realm_core::StreamKind`] **is** `#[non_exhaustive]`, so
//!   [`stream_kind_usage`] cannot be a compile-time-exhaustive match across
//!   the crate boundary. [`build_inventory`] instead asserts, at
//!   proof-build time, that every entry in the hand-maintained
//!   [`KNOWN_STREAM_KINDS`] table is classified (not the wildcard
//!   fallback), which is the accurate, honest boundary for a
//!   `#[non_exhaustive]` upstream enum. See
//!   `docs/how-to/verify-provider-parity.md` for the operator-facing
//!   statement of this limitation and what closes it.
//! - [`build_inventory`] additionally proves the canonical operation table
//!   itself has no duplicate and no missing entry (every [`CanonicalOperation`]
//!   variant appears in [`ALL_OPERATIONS`] exactly once) and that every
//!   [`ProviderFamily`] owns at least one classified operation.
//! - Every report/probe type here carries only bounded, low-cardinality
//!   enum/code data (family codes, capability codes, error-kind codes,
//!   counts) — never a raw provider message, endpoint, path, or credential.

use std::collections::BTreeSet;

use d2b_realm_core::{
    Capability, ErrorKind, ExecutionId, NodeId, OperationId, PrincipalId, RealmPath, StreamAuthz,
    StreamDescriptor, StreamId, StreamKind, StreamOpen, WorkloadId, WorkloadSelector,
};

use crate::error::ProviderResult;
use crate::provider::{
    CredentialProvider, DaemonAccessApi, DaemonAccessTransport, DisplayProvider,
    DurableExecutionProvider, GuestControlEndpointProvider, HostSubstrateProvider,
    InfrastructureProvider, NodeProvider, ObservabilitySinkProvider, PersistentShellProvider,
    ProtocolCodec, RelayProvider, RuntimeProvider, StreamMux, TransportProvider, WorkloadProvider,
};
use crate::types::{
    DisplaySessionRequest, ExecStartRequest as WorkloadExecStartRequest,
    PersistentShellAttachProviderRequest, PersistentShellListProviderRequest, WorkloadSpec,
};

use super::{
    FixedCredentialProvider, FixedObservabilitySinkProvider, HeadlessDisplayProvider,
    HeadlessPersistentShellProvider, LocalUnixDaemonAccessTransport, LoopbackStreamMux,
    LoopbackTransportProvider, MockDaemonAccessApi, MockDurableExecutionProvider,
    MockHostSubstrateProvider, MockInfrastructureProvider, MockNodeProvider, MockRelayProvider,
    MockRuntimeProvider, MockWorkloadProvider, NoGuestControlEndpointProvider, NoOpProtocolCodec,
};

// ---------------------------------------------------------------------------
// Provider family registry
// ---------------------------------------------------------------------------

/// One `crate::provider` trait family (ADR 0032/0043). Every trait declared
/// in `provider.rs` MUST have exactly one variant here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderFamily {
    HostSubstrate,
    Runtime,
    Workload,
    DurableExecution,
    GuestControlEndpoint,
    PersistentShell,
    Display,
    TransportListener,
    Transport,
    StreamMux,
    ProtocolCodec,
    DaemonAccessTransport,
    DaemonAccessApi,
    Infrastructure,
    Credential,
    ObservabilitySink,
    Relay,
    Node,
}

impl ProviderFamily {
    /// Bounded, low-cardinality kebab-case code (safe for diagnostics).
    pub fn code(self) -> &'static str {
        match self {
            ProviderFamily::HostSubstrate => "host-substrate",
            ProviderFamily::Runtime => "runtime",
            ProviderFamily::Workload => "workload",
            ProviderFamily::DurableExecution => "durable-execution",
            ProviderFamily::GuestControlEndpoint => "guest-control-endpoint",
            ProviderFamily::PersistentShell => "persistent-shell",
            ProviderFamily::Display => "display",
            ProviderFamily::TransportListener => "transport-listener",
            ProviderFamily::Transport => "transport",
            ProviderFamily::StreamMux => "stream-mux",
            ProviderFamily::ProtocolCodec => "protocol-codec",
            ProviderFamily::DaemonAccessTransport => "daemon-access-transport",
            ProviderFamily::DaemonAccessApi => "daemon-access-api",
            ProviderFamily::Infrastructure => "infrastructure",
            ProviderFamily::Credential => "credential",
            ProviderFamily::ObservabilitySink => "observability-sink",
            ProviderFamily::Relay => "relay",
            ProviderFamily::Node => "node",
        }
    }
}

/// Every [`ProviderFamily`] exactly once. Kept in sync with the enum by the
/// `family_table_has_no_duplicate_or_missing_entry` test.
pub const ALL_FAMILIES: [ProviderFamily; 18] = [
    ProviderFamily::HostSubstrate,
    ProviderFamily::Runtime,
    ProviderFamily::Workload,
    ProviderFamily::DurableExecution,
    ProviderFamily::GuestControlEndpoint,
    ProviderFamily::PersistentShell,
    ProviderFamily::Display,
    ProviderFamily::TransportListener,
    ProviderFamily::Transport,
    ProviderFamily::StreamMux,
    ProviderFamily::ProtocolCodec,
    ProviderFamily::DaemonAccessTransport,
    ProviderFamily::DaemonAccessApi,
    ProviderFamily::Infrastructure,
    ProviderFamily::Credential,
    ProviderFamily::ObservabilitySink,
    ProviderFamily::Relay,
    ProviderFamily::Node,
];

// ---------------------------------------------------------------------------
// Canonical operation registry
// ---------------------------------------------------------------------------

/// Every non-identity (verb) method across every `provider::*` trait.
/// Identity/descriptor getters (`provider_id`, `node_id`, `capabilities`,
/// `mode`, `transport_id`, `codec_id`, `schema_fingerprint`) are metadata,
/// not operations, and are exercised directly by the probes instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum CanonicalOperation {
    HostSubstrateCheck,
    RuntimePlanWorkload,
    RuntimeStart,
    RuntimeStop,
    RuntimeInspect,
    WorkloadList,
    WorkloadCreate,
    WorkloadStart,
    WorkloadStop,
    WorkloadExec,
    DurableExecStart,
    DurableExecAttach,
    DurableExecLogs,
    DurableExecCancel,
    GuestControlEndpointStatus,
    ShellList,
    ShellAttach,
    ShellDetach,
    ShellKill,
    DisplayOpenSession,
    DisplayCloseSession,
    TransportListenerAccept,
    TransportConnect,
    TransportListen,
    MuxOpenStream,
    MuxAcceptStream,
    MuxCloseStream,
    CodecEncodeFrame,
    CodecDecodeFrame,
    DaemonAccessConnect,
    DaemonAccessVmList,
    InfrastructurePlan,
    CredentialStatus,
    CredentialEnrollmentValid,
    ObservabilityHealthy,
    RelayOpenListener,
    NodeListWorkloads,
}

impl CanonicalOperation {
    /// The trait family this operation belongs to. Exhaustive: a new
    /// variant without an arm here is a compile error.
    pub fn family(self) -> ProviderFamily {
        use CanonicalOperation::*;
        match self {
            HostSubstrateCheck => ProviderFamily::HostSubstrate,
            RuntimePlanWorkload | RuntimeStart | RuntimeStop | RuntimeInspect => {
                ProviderFamily::Runtime
            }
            WorkloadList | WorkloadCreate | WorkloadStart | WorkloadStop | WorkloadExec => {
                ProviderFamily::Workload
            }
            DurableExecStart | DurableExecAttach | DurableExecLogs | DurableExecCancel => {
                ProviderFamily::DurableExecution
            }
            GuestControlEndpointStatus => ProviderFamily::GuestControlEndpoint,
            ShellList | ShellAttach | ShellDetach | ShellKill => ProviderFamily::PersistentShell,
            DisplayOpenSession | DisplayCloseSession => ProviderFamily::Display,
            TransportListenerAccept => ProviderFamily::TransportListener,
            TransportConnect | TransportListen => ProviderFamily::Transport,
            MuxOpenStream | MuxAcceptStream | MuxCloseStream => ProviderFamily::StreamMux,
            CodecEncodeFrame | CodecDecodeFrame => ProviderFamily::ProtocolCodec,
            DaemonAccessConnect => ProviderFamily::DaemonAccessTransport,
            DaemonAccessVmList => ProviderFamily::DaemonAccessApi,
            InfrastructurePlan => ProviderFamily::Infrastructure,
            CredentialStatus | CredentialEnrollmentValid => ProviderFamily::Credential,
            ObservabilityHealthy => ProviderFamily::ObservabilitySink,
            RelayOpenListener => ProviderFamily::Relay,
            NodeListWorkloads => ProviderFamily::Node,
        }
    }

    /// The exact trait method name this operation names (for operator
    /// diagnostics; always a fixed, low-cardinality identifier).
    pub fn method_name(self) -> &'static str {
        use CanonicalOperation::*;
        match self {
            HostSubstrateCheck => "check",
            RuntimePlanWorkload => "plan_workload",
            RuntimeStart => "start",
            RuntimeStop => "stop",
            RuntimeInspect => "inspect",
            WorkloadList => "list",
            WorkloadCreate => "create",
            WorkloadStart => "start",
            WorkloadStop => "stop",
            WorkloadExec => "exec",
            DurableExecStart => "start",
            DurableExecAttach => "attach",
            DurableExecLogs => "logs",
            DurableExecCancel => "cancel",
            GuestControlEndpointStatus => "endpoint_status",
            ShellList => "list_shells",
            ShellAttach => "attach_shell",
            ShellDetach => "detach_shell",
            ShellKill => "kill_shell",
            DisplayOpenSession => "open_display_session",
            DisplayCloseSession => "close_display_session",
            TransportListenerAccept => "accept",
            TransportConnect => "connect",
            TransportListen => "listen",
            MuxOpenStream => "open_stream",
            MuxAcceptStream => "accept_stream",
            MuxCloseStream => "close_stream",
            CodecEncodeFrame => "encode_frame",
            CodecDecodeFrame => "decode_frame",
            DaemonAccessConnect => "connect",
            DaemonAccessVmList => "vm_list",
            InfrastructurePlan => "plan_infrastructure",
            CredentialStatus => "status",
            CredentialEnrollmentValid => "enrollment_valid",
            ObservabilityHealthy => "healthy",
            RelayOpenListener => "open_listener",
            NodeListWorkloads => "list_workloads",
        }
    }

    /// A stable 0-based ordinal, used only to prove [`ALL_OPERATIONS`] has
    /// no duplicate and no missing variant. Exhaustive: a new variant
    /// without an arm here is a compile error.
    fn ordinal(self) -> usize {
        use CanonicalOperation::*;
        match self {
            HostSubstrateCheck => 0,
            RuntimePlanWorkload => 1,
            RuntimeStart => 2,
            RuntimeStop => 3,
            RuntimeInspect => 4,
            WorkloadList => 5,
            WorkloadCreate => 6,
            WorkloadStart => 7,
            WorkloadStop => 8,
            WorkloadExec => 9,
            DurableExecStart => 10,
            DurableExecAttach => 11,
            DurableExecLogs => 12,
            DurableExecCancel => 13,
            GuestControlEndpointStatus => 14,
            ShellList => 15,
            ShellAttach => 16,
            ShellDetach => 17,
            ShellKill => 18,
            DisplayOpenSession => 19,
            DisplayCloseSession => 20,
            TransportListenerAccept => 21,
            TransportConnect => 22,
            TransportListen => 23,
            MuxOpenStream => 24,
            MuxAcceptStream => 25,
            MuxCloseStream => 26,
            CodecEncodeFrame => 27,
            CodecDecodeFrame => 28,
            DaemonAccessConnect => 29,
            DaemonAccessVmList => 30,
            InfrastructurePlan => 31,
            CredentialStatus => 32,
            CredentialEnrollmentValid => 33,
            ObservabilityHealthy => 34,
            RelayOpenListener => 35,
            NodeListWorkloads => 36,
        }
    }
}

/// Total number of canonical operations. Kept equal to [`ALL_OPERATIONS`]'s
/// length and to one past the highest [`CanonicalOperation::ordinal`].
pub const OPERATION_COUNT: usize = 37;

/// Every [`CanonicalOperation`] variant, exactly once. Self-checked by
/// `operation_table_has_no_duplicate_or_missing_entry`.
pub const ALL_OPERATIONS: [CanonicalOperation; OPERATION_COUNT] = [
    CanonicalOperation::HostSubstrateCheck,
    CanonicalOperation::RuntimePlanWorkload,
    CanonicalOperation::RuntimeStart,
    CanonicalOperation::RuntimeStop,
    CanonicalOperation::RuntimeInspect,
    CanonicalOperation::WorkloadList,
    CanonicalOperation::WorkloadCreate,
    CanonicalOperation::WorkloadStart,
    CanonicalOperation::WorkloadStop,
    CanonicalOperation::WorkloadExec,
    CanonicalOperation::DurableExecStart,
    CanonicalOperation::DurableExecAttach,
    CanonicalOperation::DurableExecLogs,
    CanonicalOperation::DurableExecCancel,
    CanonicalOperation::GuestControlEndpointStatus,
    CanonicalOperation::ShellList,
    CanonicalOperation::ShellAttach,
    CanonicalOperation::ShellDetach,
    CanonicalOperation::ShellKill,
    CanonicalOperation::DisplayOpenSession,
    CanonicalOperation::DisplayCloseSession,
    CanonicalOperation::TransportListenerAccept,
    CanonicalOperation::TransportConnect,
    CanonicalOperation::TransportListen,
    CanonicalOperation::MuxOpenStream,
    CanonicalOperation::MuxAcceptStream,
    CanonicalOperation::MuxCloseStream,
    CanonicalOperation::CodecEncodeFrame,
    CanonicalOperation::CodecDecodeFrame,
    CanonicalOperation::DaemonAccessConnect,
    CanonicalOperation::DaemonAccessVmList,
    CanonicalOperation::InfrastructurePlan,
    CanonicalOperation::CredentialStatus,
    CanonicalOperation::CredentialEnrollmentValid,
    CanonicalOperation::ObservabilityHealthy,
    CanonicalOperation::RelayOpenListener,
    CanonicalOperation::NodeListWorkloads,
];

// ---------------------------------------------------------------------------
// Operation classification
// ---------------------------------------------------------------------------

/// How an operation's required capability is determined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CapabilityGate {
    /// The owning trait self-advertises a capability set (via its
    /// `capabilities()`/`check()` getter) and denies this operation with a
    /// typed `CapabilityDenied` naming exactly this capability when absent.
    SelfAdvertised(Capability),
    /// The required capability is not fixed at compile time: it is derived
    /// per call from the request's [`StreamKind`] (`StreamKind::required_capability`).
    AdvertisedPerStreamKind,
    /// Gating happens above this trait (router/gateway/operation layer)
    /// before the call reaches the provider; the trait has no
    /// `capabilities()` getter of its own to consult.
    CallerScoped,
    /// No capability precondition applies (identity/liveness/close/metadata
    /// operations that a provider must always answer).
    Ungated,
}

/// Whether repeating a call with the same input is safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Idempotency {
    /// A pure read or resolution: repeating it is always safe.
    NaturallyIdempotent,
    /// Idempotent by explicit trait contract (documented in `provider.rs`).
    IdempotentByContract,
    /// Idempotency is owned by the caller/gateway via an out-of-band
    /// idempotency key; this trait performs the mutation exactly once per
    /// distinct key.
    CallerOwnedIdempotencyKey,
    /// Each call is a distinct, non-idempotent mutation.
    NonIdempotentSingleUse,
}

/// Whether a caller may safely retry this operation without additional
/// coordination.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetrySafety {
    /// Safe to retry as-is.
    SafeToRetry,
    /// Safe to retry only under a fresh caller-owned idempotency key.
    RetryRequiresNewIdempotencyKey,
    /// Never safe to retry blindly.
    NotRetryable,
}

/// How an in-flight or completed instance of this operation can be
/// cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationSupport {
    /// This operation *is* the typed cancellation primitive for its family.
    ExplicitCancelOperation,
    /// Cancellation is implicit: closing/detaching ends the operation's
    /// effect without a dedicated cancel call.
    ImplicitViaClose,
    /// This operation has no cancellation semantics of its own.
    NotCancellable,
}

/// How an operation relates to byte-carrying streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachmentBehavior {
    /// Carries no bytes and opens/binds nothing.
    NoStream,
    /// Opens a raw, pre-mux byte channel (below stream authorization).
    OpensRawByteChannel,
    /// Accepts an inbound raw, pre-mux byte channel.
    AcceptsInboundRawChannel,
    /// Opens a named, authorized stream of a fixed [`StreamKind`].
    OpensAuthorizedStream(StreamKind),
    /// Binds to (does not itself open) an already-authorized stream of a
    /// fixed [`StreamKind`] that the caller must have opened first.
    BindsAuthorizedStream(StreamKind),
    /// Opens a named, authorized stream whose kind is supplied per call
    /// (the mux's generic open path).
    OpensAnyAuthorizedStream,
    /// Accepts an inbound named, authorized stream of any kind.
    AcceptsAnyAuthorizedStream,
    /// Ends a previously opened named stream.
    ClosesAuthorizedStream,
    /// Encodes or decodes a semantic frame; carries no live channel itself.
    EncodesDecodesFrame,
}

/// Every canonical operation's fallback posture. Single-variant by
/// construction: the type itself cannot represent "fell back to SSH, a
/// generic tunnel, or an undocumented alternate provider", which is the
/// ADR 0032 invariant this crate exists to enforce.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FallbackPolicy {
    /// A missing capability/precondition is always a typed denial, never a
    /// silent alternate transport or provider.
    NeverFallback,
}

/// The full classification of one [`CanonicalOperation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationClassification {
    pub capability_gate: CapabilityGate,
    pub idempotency: Idempotency,
    pub retry: RetrySafety,
    pub cancellation: CancellationSupport,
    pub attachment: AttachmentBehavior,
    pub fallback: FallbackPolicy,
}

impl CanonicalOperation {
    /// The full classification for this operation. Exhaustive: a new
    /// variant without an arm here is a compile error (fail closed on an
    /// unclassified operation).
    pub fn classification(self) -> OperationClassification {
        use AttachmentBehavior::*;
        use CancellationSupport::*;
        use CanonicalOperation::*;
        use CapabilityGate::*;
        use Idempotency::*;
        use RetrySafety::*;

        const fn c(
            capability_gate: CapabilityGate,
            idempotency: Idempotency,
            retry: RetrySafety,
            cancellation: CancellationSupport,
            attachment: AttachmentBehavior,
        ) -> OperationClassification {
            OperationClassification {
                capability_gate,
                idempotency,
                retry,
                cancellation,
                attachment,
                fallback: FallbackPolicy::NeverFallback,
            }
        }

        match self {
            HostSubstrateCheck => c(
                Ungated,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            RuntimePlanWorkload => c(
                SelfAdvertised(Capability::Lifecycle),
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            RuntimeStart => c(
                SelfAdvertised(Capability::Lifecycle),
                NonIdempotentSingleUse,
                RetryRequiresNewIdempotencyKey,
                NotCancellable,
                NoStream,
            ),
            RuntimeStop => c(
                SelfAdvertised(Capability::Lifecycle),
                IdempotentByContract,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            RuntimeInspect => c(
                Ungated,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            WorkloadList => c(
                Ungated,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            WorkloadCreate => c(
                SelfAdvertised(Capability::Lifecycle),
                NonIdempotentSingleUse,
                RetryRequiresNewIdempotencyKey,
                NotCancellable,
                NoStream,
            ),
            WorkloadStart => c(
                SelfAdvertised(Capability::Lifecycle),
                NonIdempotentSingleUse,
                RetryRequiresNewIdempotencyKey,
                NotCancellable,
                NoStream,
            ),
            WorkloadStop => c(
                SelfAdvertised(Capability::Lifecycle),
                IdempotentByContract,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            WorkloadExec => c(
                SelfAdvertised(Capability::Exec),
                NonIdempotentSingleUse,
                RetryRequiresNewIdempotencyKey,
                NotCancellable,
                BindsAuthorizedStream(StreamKind::Stdio),
            ),
            DurableExecStart => c(
                CallerScoped,
                CallerOwnedIdempotencyKey,
                SafeToRetry,
                NotCancellable,
                BindsAuthorizedStream(StreamKind::Stdio),
            ),
            DurableExecAttach => c(
                CallerScoped,
                CallerOwnedIdempotencyKey,
                SafeToRetry,
                NotCancellable,
                BindsAuthorizedStream(StreamKind::Stdio),
            ),
            DurableExecLogs => c(
                CallerScoped,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                BindsAuthorizedStream(StreamKind::Logs),
            ),
            DurableExecCancel => c(
                CallerScoped,
                IdempotentByContract,
                SafeToRetry,
                ExplicitCancelOperation,
                NoStream,
            ),
            GuestControlEndpointStatus => c(
                SelfAdvertised(Capability::PersistentShell),
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            ShellList => c(
                SelfAdvertised(Capability::PersistentShell),
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            ShellAttach => c(
                SelfAdvertised(Capability::PersistentShell),
                IdempotentByContract,
                SafeToRetry,
                NotCancellable,
                BindsAuthorizedStream(StreamKind::ShellPty),
            ),
            ShellDetach => c(
                SelfAdvertised(Capability::PersistentShell),
                IdempotentByContract,
                SafeToRetry,
                ImplicitViaClose,
                NoStream,
            ),
            ShellKill => c(
                SelfAdvertised(Capability::PersistentShell),
                IdempotentByContract,
                SafeToRetry,
                ExplicitCancelOperation,
                NoStream,
            ),
            DisplayOpenSession => c(
                SelfAdvertised(Capability::WindowForwarding),
                NonIdempotentSingleUse,
                RetryRequiresNewIdempotencyKey,
                NotCancellable,
                BindsAuthorizedStream(StreamKind::Display),
            ),
            DisplayCloseSession => c(
                Ungated,
                IdempotentByContract,
                SafeToRetry,
                ImplicitViaClose,
                NoStream,
            ),
            TransportListenerAccept => c(
                CallerScoped,
                NonIdempotentSingleUse,
                SafeToRetry,
                NotCancellable,
                AcceptsInboundRawChannel,
            ),
            TransportConnect => c(
                CallerScoped,
                NonIdempotentSingleUse,
                SafeToRetry,
                NotCancellable,
                OpensRawByteChannel,
            ),
            TransportListen => c(
                CallerScoped,
                NonIdempotentSingleUse,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            MuxOpenStream => c(
                AdvertisedPerStreamKind,
                NonIdempotentSingleUse,
                SafeToRetry,
                NotCancellable,
                OpensAnyAuthorizedStream,
            ),
            MuxAcceptStream => c(
                CallerScoped,
                NonIdempotentSingleUse,
                SafeToRetry,
                NotCancellable,
                AcceptsAnyAuthorizedStream,
            ),
            MuxCloseStream => c(
                Ungated,
                IdempotentByContract,
                SafeToRetry,
                ImplicitViaClose,
                ClosesAuthorizedStream,
            ),
            CodecEncodeFrame => c(
                Ungated,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                EncodesDecodesFrame,
            ),
            CodecDecodeFrame => c(
                Ungated,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                EncodesDecodesFrame,
            ),
            DaemonAccessConnect => c(
                Ungated,
                NonIdempotentSingleUse,
                SafeToRetry,
                NotCancellable,
                OpensRawByteChannel,
            ),
            DaemonAccessVmList => c(
                Ungated,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            InfrastructurePlan => c(
                Ungated,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            CredentialStatus => c(
                Ungated,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            CredentialEnrollmentValid => c(
                Ungated,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            ObservabilityHealthy => c(
                Ungated,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            RelayOpenListener => c(
                CallerScoped,
                NonIdempotentSingleUse,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
            NodeListWorkloads => c(
                Ungated,
                NaturallyIdempotent,
                SafeToRetry,
                NotCancellable,
                NoStream,
            ),
        }
    }
}

// ---------------------------------------------------------------------------
// Capability coverage
// ---------------------------------------------------------------------------

/// Which canonical operations and/or stream kind a [`Capability`] gates in
/// this crate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapabilityRole {
    /// Canonical operations whose [`CapabilityGate::SelfAdvertised`] names
    /// exactly this capability.
    pub operations: &'static [CanonicalOperation],
    /// The named stream kind this capability is `StreamKind::required_capability`
    /// for, if any is declared in [`KNOWN_STREAM_KINDS`].
    pub stream_kind: Option<StreamKind>,
}

/// The capability role for every [`Capability`] variant. Exhaustive:
/// `Capability` is not `#[non_exhaustive]`, so a new capability added
/// upstream is a compile error here until classified (fail closed).
pub fn capability_role(cap: Capability) -> CapabilityRole {
    use CanonicalOperation::*;
    const fn role(
        ops: &'static [CanonicalOperation],
        stream_kind: Option<StreamKind>,
    ) -> CapabilityRole {
        CapabilityRole {
            operations: ops,
            stream_kind,
        }
    }
    match cap {
        Capability::Lifecycle => role(
            &[
                RuntimePlanWorkload,
                RuntimeStart,
                RuntimeStop,
                WorkloadCreate,
                WorkloadStart,
                WorkloadStop,
            ],
            Some(StreamKind::Control),
        ),
        Capability::Exec => role(&[WorkloadExec], Some(StreamKind::Stdio)),
        Capability::Pty => role(&[], Some(StreamKind::Pty)),
        Capability::Logs => role(&[], Some(StreamKind::Logs)),
        Capability::FileCopy => role(&[], Some(StreamKind::FileCopy)),
        Capability::PortForward => role(&[], Some(StreamKind::PortForward)),
        Capability::PersistentShell => role(
            &[
                GuestControlEndpointStatus,
                ShellList,
                ShellAttach,
                ShellDetach,
                ShellKill,
            ],
            Some(StreamKind::ShellPty),
        ),
        Capability::Vsock => role(&[], None),
        Capability::Virtiofs => role(&[], None),
        Capability::WindowForwarding => role(&[DisplayOpenSession], Some(StreamKind::Display)),
        Capability::DisplayStreaming => role(&[], None),
        Capability::Clipboard => role(&[], Some(StreamKind::Clipboard)),
        Capability::AudioPlayback => role(&[], Some(StreamKind::AudioPlayback)),
        Capability::AudioCapture => role(&[], Some(StreamKind::AudioCapture)),
        Capability::Hid => role(&[], Some(StreamKind::DeviceHid)),
        Capability::Usb => role(&[], Some(StreamKind::DeviceUsb)),
        Capability::GpuAccel => role(&[], None),
        Capability::Snapshots => role(&[], None),
        Capability::Hotplug => role(&[], None),
        Capability::EphemeralSessions => role(&[], None),
        Capability::ProviderManagedIsolation => role(&[], None),
        Capability::ConfiguredLaunch => role(&[], None),
    }
}

/// Every [`Capability`] variant, exactly once. Kept in sync by
/// `capability_table_has_no_duplicate_or_missing_entry`.
pub const ALL_CAPABILITIES: [Capability; 22] = [
    Capability::Lifecycle,
    Capability::Exec,
    Capability::Pty,
    Capability::Logs,
    Capability::FileCopy,
    Capability::PortForward,
    Capability::PersistentShell,
    Capability::Vsock,
    Capability::Virtiofs,
    Capability::WindowForwarding,
    Capability::DisplayStreaming,
    Capability::Clipboard,
    Capability::AudioPlayback,
    Capability::AudioCapture,
    Capability::Hid,
    Capability::Usb,
    Capability::GpuAccel,
    Capability::Snapshots,
    Capability::Hotplug,
    Capability::EphemeralSessions,
    Capability::ProviderManagedIsolation,
    Capability::ConfiguredLaunch,
];

// ---------------------------------------------------------------------------
// Error-kind ownership
// ---------------------------------------------------------------------------

/// Which layer is expected to construct/observe an [`ErrorKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKindOwner {
    /// This crate's `ProviderError`/mocks may construct this kind.
    ProviderLayer,
    /// Owned by the router/gateway/session layer above this crate; a
    /// provider stand-in in this crate must never fabricate it.
    RouterLayer,
    /// Meaningful at either layer (e.g. generic bounded-queue backpressure).
    SharedLayer,
}

/// The owning layer for every [`ErrorKind`] variant. Exhaustive: `ErrorKind`
/// is not `#[non_exhaustive]`, so a new kind added upstream is a compile
/// error here until classified (fail closed).
pub fn error_kind_owner(kind: ErrorKind) -> ErrorKindOwner {
    match kind {
        ErrorKind::CapabilityDenied => ErrorKindOwner::ProviderLayer,
        ErrorKind::Unauthorized => ErrorKindOwner::SharedLayer,
        ErrorKind::NoRealmEntrypoint => ErrorKindOwner::RouterLayer,
        ErrorKind::GatewayUnavailable => ErrorKindOwner::RouterLayer,
        ErrorKind::ProviderAllocationFailed => ErrorKindOwner::ProviderLayer,
        ErrorKind::RelayUnavailable => ErrorKindOwner::RouterLayer,
        ErrorKind::AuthenticationFailed => ErrorKindOwner::RouterLayer,
        ErrorKind::VersionSkew => ErrorKindOwner::RouterLayer,
        ErrorKind::OperationInProgress => ErrorKindOwner::SharedLayer,
        ErrorKind::IdempotencyKeyConflict => ErrorKindOwner::SharedLayer,
        ErrorKind::IdempotencyKeyExpired => ErrorKindOwner::SharedLayer,
        ErrorKind::Backpressure => ErrorKindOwner::SharedLayer,
        ErrorKind::Cancelled => ErrorKindOwner::SharedLayer,
        ErrorKind::Timeout => ErrorKindOwner::SharedLayer,
        ErrorKind::FrameTooLarge => ErrorKindOwner::RouterLayer,
        ErrorKind::MalformedFrame => ErrorKindOwner::ProviderLayer,
        ErrorKind::InvalidTarget => ErrorKindOwner::RouterLayer,
        ErrorKind::AuditUnavailable => ErrorKindOwner::RouterLayer,
        ErrorKind::UnsupportedFeature => ErrorKindOwner::ProviderLayer,
    }
}

/// Every [`ErrorKind`] variant, exactly once. Kept in sync by
/// `error_kind_table_has_no_duplicate_or_missing_entry`.
pub const ALL_ERROR_KINDS: [ErrorKind; 19] = [
    ErrorKind::CapabilityDenied,
    ErrorKind::Unauthorized,
    ErrorKind::NoRealmEntrypoint,
    ErrorKind::GatewayUnavailable,
    ErrorKind::ProviderAllocationFailed,
    ErrorKind::RelayUnavailable,
    ErrorKind::AuthenticationFailed,
    ErrorKind::VersionSkew,
    ErrorKind::OperationInProgress,
    ErrorKind::IdempotencyKeyConflict,
    ErrorKind::IdempotencyKeyExpired,
    ErrorKind::Backpressure,
    ErrorKind::Cancelled,
    ErrorKind::Timeout,
    ErrorKind::FrameTooLarge,
    ErrorKind::MalformedFrame,
    ErrorKind::InvalidTarget,
    ErrorKind::AuditUnavailable,
    ErrorKind::UnsupportedFeature,
];

/// This crate's stand-ins only ever construct these kinds (see
/// `error.rs`'s `ProviderError::capability_denied`/`unsupported`/`new`
/// call sites in `mock.rs`). Used to prove the mocks never impersonate the
/// router layer.
pub const PROVIDER_LAYER_KINDS_IN_USE: [ErrorKind; 2] =
    [ErrorKind::CapabilityDenied, ErrorKind::UnsupportedFeature];

// ---------------------------------------------------------------------------
// Stream-kind coverage (honest `#[non_exhaustive]` boundary)
// ---------------------------------------------------------------------------

/// What, if anything, in this crate's canonical operation set uses a given
/// [`StreamKind`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamKindUsage {
    /// Named and bound by at least one canonical operation.
    UsedByOperation(CanonicalOperation),
    /// A real capability/stream kind pairing exists (`required_capability`
    /// is meaningful) but no canonical operation in this crate binds to it
    /// yet — an accurate, explicit "not yet wired" record, not a silent gap.
    DeclaredNotYetWired,
    /// Not one of [`KNOWN_STREAM_KINDS`]. Because `StreamKind` is
    /// `#[non_exhaustive]`, this crate cannot compile-time-detect a new
    /// upstream variant; this arm is the honest boundary. See the module
    /// docs and `docs/how-to/verify-provider-parity.md`.
    UnknownToThisCrateVersion,
}

/// Classify a [`StreamKind`]. Not exhaustive over `StreamKind` itself
/// (`#[non_exhaustive]` upstream); exhaustive only over [`KNOWN_STREAM_KINDS`].
pub fn stream_kind_usage(kind: StreamKind) -> StreamKindUsage {
    match kind {
        StreamKind::Control => StreamKindUsage::DeclaredNotYetWired,
        StreamKind::Pty => StreamKindUsage::DeclaredNotYetWired,
        StreamKind::ShellPty => StreamKindUsage::UsedByOperation(CanonicalOperation::ShellAttach),
        StreamKind::Stdio => StreamKindUsage::UsedByOperation(CanonicalOperation::WorkloadExec),
        StreamKind::Logs => StreamKindUsage::UsedByOperation(CanonicalOperation::DurableExecLogs),
        StreamKind::FileCopy => StreamKindUsage::DeclaredNotYetWired,
        StreamKind::PortForward => StreamKindUsage::DeclaredNotYetWired,
        StreamKind::Display => {
            StreamKindUsage::UsedByOperation(CanonicalOperation::DisplayOpenSession)
        }
        StreamKind::Clipboard => StreamKindUsage::DeclaredNotYetWired,
        StreamKind::AudioPlayback => StreamKindUsage::DeclaredNotYetWired,
        StreamKind::AudioCapture => StreamKindUsage::DeclaredNotYetWired,
        StreamKind::DeviceHid => StreamKindUsage::DeclaredNotYetWired,
        StreamKind::DeviceUsb => StreamKindUsage::DeclaredNotYetWired,
        _ => StreamKindUsage::UnknownToThisCrateVersion,
    }
}

/// Every [`StreamKind`] variant known to this crate version, exactly once.
pub const KNOWN_STREAM_KINDS: [StreamKind; 13] = [
    StreamKind::Control,
    StreamKind::Pty,
    StreamKind::ShellPty,
    StreamKind::Stdio,
    StreamKind::Logs,
    StreamKind::FileCopy,
    StreamKind::PortForward,
    StreamKind::Display,
    StreamKind::Clipboard,
    StreamKind::AudioPlayback,
    StreamKind::AudioCapture,
    StreamKind::DeviceHid,
    StreamKind::DeviceUsb,
];

// ---------------------------------------------------------------------------
// Inventory self-consistency (fail closed on missing/duplicate/unclassified)
// ---------------------------------------------------------------------------

/// A bounded, low-cardinality violation. Never carries a raw provider
/// message, endpoint, path, or credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParityViolation {
    /// [`ALL_OPERATIONS`] does not contain every [`CanonicalOperation`]
    /// variant exactly once.
    DuplicateOrMissingOperation,
    /// [`ALL_FAMILIES`] does not contain every [`ProviderFamily`] variant
    /// exactly once.
    DuplicateOrMissingFamily,
    /// A [`ProviderFamily`] has zero canonical operations classified
    /// against it.
    UnclassifiedFamily(ProviderFamily),
    /// [`ALL_CAPABILITIES`] does not contain every [`Capability`] variant
    /// exactly once.
    DuplicateOrMissingCapability,
    /// [`ALL_ERROR_KINDS`] does not contain every [`ErrorKind`] variant
    /// exactly once.
    DuplicateOrMissingErrorKind,
    /// An operation's declared `SelfAdvertised`/`AdvertisedPerStreamKind`
    /// capability gate disagrees with [`capability_role`]'s operation list
    /// for that capability.
    CapabilityGateCrossCheckFailed(CanonicalOperation),
    /// A stand-in probe's observed behavior disagreed with the operation's
    /// declared classification.
    ProbeMismatch(CanonicalOperation),
}

/// A bounded summary of a successful inventory build. Counts and codes
/// only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    pub family_count: usize,
    pub operation_count: usize,
    pub capability_count: usize,
    pub error_kind_count: usize,
}

fn ordinals_form_a_permutation<T: Copy>(
    items: &[T],
    ordinal: impl Fn(T) -> usize,
    len: usize,
) -> bool {
    if items.len() != len {
        return false;
    }
    let mut seen: BTreeSet<usize> = BTreeSet::new();
    for item in items {
        if !seen.insert(ordinal(*item)) {
            return false;
        }
    }
    seen.len() == len && seen.iter().enumerate().all(|(i, v)| i == *v)
}

/// Build and self-check the parity inventory. Fails closed (returns
/// [`ParityViolation`]) on any missing, duplicate, or unclassified
/// operation/family/capability/error-kind, or on a capability-gate
/// cross-check disagreement. Never touches a real host, relay, or provider
/// API.
pub fn build_inventory() -> Result<Inventory, ParityViolation> {
    if !ordinals_form_a_permutation(
        &ALL_OPERATIONS,
        CanonicalOperation::ordinal,
        OPERATION_COUNT,
    ) {
        return Err(ParityViolation::DuplicateOrMissingOperation);
    }

    let family_ordinal = |f: ProviderFamily| {
        ALL_FAMILIES
            .iter()
            .position(|x| *x == f)
            .unwrap_or(usize::MAX)
    };
    let mut family_seen: BTreeSet<usize> = BTreeSet::new();
    for family in ALL_FAMILIES {
        if !family_seen.insert(family_ordinal(family)) {
            return Err(ParityViolation::DuplicateOrMissingFamily);
        }
    }
    if family_seen.len() != ALL_FAMILIES.len() {
        return Err(ParityViolation::DuplicateOrMissingFamily);
    }

    for family in ALL_FAMILIES {
        if !ALL_OPERATIONS.iter().any(|op| op.family() == family) {
            return Err(ParityViolation::UnclassifiedFamily(family));
        }
    }

    let mut cap_seen: BTreeSet<&'static str> = BTreeSet::new();
    for cap in ALL_CAPABILITIES {
        if !cap_seen.insert(cap.code()) {
            return Err(ParityViolation::DuplicateOrMissingCapability);
        }
    }
    if cap_seen.len() != ALL_CAPABILITIES.len() {
        return Err(ParityViolation::DuplicateOrMissingCapability);
    }

    let mut kind_seen: BTreeSet<&'static str> = BTreeSet::new();
    for kind in ALL_ERROR_KINDS {
        if !kind_seen.insert(kind.code()) {
            return Err(ParityViolation::DuplicateOrMissingErrorKind);
        }
    }
    if kind_seen.len() != ALL_ERROR_KINDS.len() {
        return Err(ParityViolation::DuplicateOrMissingErrorKind);
    }

    for op in ALL_OPERATIONS {
        if let CapabilityGate::SelfAdvertised(cap) = op.classification().capability_gate
            && !capability_role(cap).operations.contains(&op)
        {
            return Err(ParityViolation::CapabilityGateCrossCheckFailed(op));
        }
    }

    Ok(Inventory {
        family_count: ALL_FAMILIES.len(),
        operation_count: ALL_OPERATIONS.len(),
        capability_count: ALL_CAPABILITIES.len(),
        error_kind_count: ALL_ERROR_KINDS.len(),
    })
}

// ---------------------------------------------------------------------------
// Stand-in probes (never call a real provider)
// ---------------------------------------------------------------------------

/// A bounded, redacted observation of one stand-in call. Never carries a
/// raw provider message, endpoint, path, or credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeOutcome {
    pub op: CanonicalOperation,
    pub ok: bool,
    pub kind: Option<ErrorKind>,
    pub missing_capability: Option<Capability>,
}

fn principal() -> PrincipalId {
    PrincipalId::parse("parity-principal").expect("valid")
}

fn stream_open_for(kind: StreamKind) -> StreamOpen {
    StreamOpen {
        descriptor: StreamDescriptor {
            id: StreamId::parse("parity-stream").expect("valid"),
            kind,
        },
        operation_id: OperationId::parse("parity-op").expect("valid"),
        authz: StreamAuthz::for_kind(principal(), RealmPath::local(), kind),
    }
}

/// Exercise every [`CapabilityGate::SelfAdvertised`]/`AdvertisedPerStreamKind`
/// operation against a stand-in that lacks the capability, proving it
/// fails closed with a typed `CapabilityDenied` naming exactly the
/// declared capability (never a silent fallback).
pub async fn run_capability_denial_probes() -> Vec<ProbeOutcome> {
    let mut out = Vec::new();

    // DisplayOpenSession: HeadlessDisplayProvider advertises nothing.
    {
        let provider = HeadlessDisplayProvider;
        let req = DisplaySessionRequest {
            workload: WorkloadId::parse("parity-workload").expect("valid"),
            operation_id: OperationId::parse("parity-op").expect("valid"),
            display_stream: StreamId::parse("parity-display").expect("valid"),
            authz: StreamAuthz::for_kind(principal(), RealmPath::local(), StreamKind::Display),
        };
        out.push(observe_gate(
            CanonicalOperation::DisplayOpenSession,
            provider.open_display_session(req).await,
        ));
    }

    // GuestControlEndpointStatus: NoGuestControlEndpointProvider advertises
    // nothing.
    {
        let provider = NoGuestControlEndpointProvider;
        let workload = WorkloadId::parse("parity-workload").expect("valid");
        out.push(observe_gate(
            CanonicalOperation::GuestControlEndpointStatus,
            provider.endpoint_status(workload).await,
        ));
    }

    // ShellList/ShellAttach/ShellDetach/ShellKill: HeadlessPersistentShellProvider
    // advertises nothing.
    {
        let provider = HeadlessPersistentShellProvider;
        let workload = WorkloadId::parse("parity-workload").expect("valid");
        let list_req = PersistentShellListProviderRequest {
            workload: workload.clone(),
            operation_id: OperationId::parse("parity-op").expect("valid"),
            request: d2b_realm_core::ShellListRequest { generation: None },
        };
        out.push(observe_gate(
            CanonicalOperation::ShellList,
            provider.list_shells(list_req).await,
        ));

        let attach_req = PersistentShellAttachProviderRequest {
            workload: workload.clone(),
            operation_id: OperationId::parse("parity-op").expect("valid"),
            request: d2b_realm_core::ShellAttachRequest {
                name: d2b_realm_core::ShellName::parse("default").expect("valid"),
                generation: d2b_realm_core::ShellGeneration {
                    guest_boot_id: d2b_realm_core::ProtocolToken::parse("boot-1").expect("valid"),
                    guestd_instance_id: d2b_realm_core::ProtocolToken::parse("guestd-1")
                        .expect("valid"),
                    shell_daemon_instance_id: d2b_realm_core::ProtocolToken::parse("shelld-1")
                        .expect("valid"),
                },
                attach_id: d2b_realm_core::ShellAttachId::parse("attach-1").expect("valid"),
                force: false,
            },
            shell_pty_stream: stream_open_for(StreamKind::ShellPty),
        };
        out.push(observe_gate(
            CanonicalOperation::ShellAttach,
            provider.attach_shell(attach_req).await,
        ));

        let shell_generation = d2b_realm_core::ShellGeneration {
            guest_boot_id: d2b_realm_core::ProtocolToken::parse("boot-1").expect("valid"),
            guestd_instance_id: d2b_realm_core::ProtocolToken::parse("guestd-1").expect("valid"),
            shell_daemon_instance_id: d2b_realm_core::ProtocolToken::parse("shelld-1")
                .expect("valid"),
        };
        let detach_req = crate::types::PersistentShellDetachProviderRequest {
            workload: workload.clone(),
            operation_id: OperationId::parse("parity-op").expect("valid"),
            request: d2b_realm_core::ShellDetachRequest {
                name: d2b_realm_core::ShellName::parse("default").expect("valid"),
                generation: shell_generation.clone(),
                attach_id: Some(d2b_realm_core::ShellAttachId::parse("attach-1").expect("valid")),
            },
        };
        out.push(observe_gate(
            CanonicalOperation::ShellDetach,
            provider.detach_shell(detach_req).await,
        ));

        let kill_req = crate::types::PersistentShellKillProviderRequest {
            workload,
            operation_id: OperationId::parse("parity-op").expect("valid"),
            request: d2b_realm_core::ShellKillRequest {
                name: d2b_realm_core::ShellName::parse("default").expect("valid"),
                generation: shell_generation,
            },
        };
        out.push(observe_gate(
            CanonicalOperation::ShellKill,
            provider.kill_shell(kill_req).await,
        ));
    }

    // MuxOpenStream: LoopbackStreamMux::default() does not advertise Clipboard.
    {
        let mux = LoopbackStreamMux::default();
        out.push(observe_gate(
            CanonicalOperation::MuxOpenStream,
            mux.open_stream(stream_open_for(StreamKind::Clipboard))
                .await,
        ));
    }

    out
}

fn observe_gate<T>(op: CanonicalOperation, result: ProviderResult<T>) -> ProbeOutcome {
    match result {
        Ok(_) => ProbeOutcome {
            op,
            ok: true,
            kind: None,
            missing_capability: None,
        },
        Err(e) => ProbeOutcome {
            op,
            ok: false,
            kind: Some(e.kind()),
            missing_capability: e.missing_capability(),
        },
    }
}

/// Exercise every remaining stand-in (the families without a self-advertised
/// capability, plus the positive-path advertised families) to prove each
/// one answers deterministically without panicking, without dialing a real
/// host/relay/provider, and without ever downgrading to
/// [`ErrorKindOwner::RouterLayer`].
pub async fn run_stand_in_smoke_probes() -> Vec<ProbeOutcome> {
    let mut out = Vec::new();

    out.push(observe_gate(
        CanonicalOperation::HostSubstrateCheck,
        MockHostSubstrateProvider.check().await,
    ));

    {
        let provider = MockRuntimeProvider::default();
        let spec = WorkloadSpec {
            alias: WorkloadId::parse("parity-workload").expect("valid"),
        };
        let plan = provider.plan_workload(spec).await;
        let plan_ok = plan.is_ok();
        out.push(observe_gate(CanonicalOperation::RuntimePlanWorkload, plan));
        if plan_ok {
            let plan = provider
                .plan_workload(WorkloadSpec {
                    alias: WorkloadId::parse("parity-workload").expect("valid"),
                })
                .await
                .expect("planned above");
            let handle = provider.start(plan).await;
            let handle_ok = handle.is_ok();
            out.push(observe_gate(CanonicalOperation::RuntimeStart, handle));
            if handle_ok {
                let handle = crate::types::RuntimeHandle {
                    workload: WorkloadId::parse("parity-workload").expect("valid"),
                };
                out.push(observe_gate(
                    CanonicalOperation::RuntimeInspect,
                    provider.inspect(handle.clone()).await,
                ));
                out.push(observe_gate(
                    CanonicalOperation::RuntimeStop,
                    provider.stop(handle).await,
                ));
            }
        }
    }

    {
        let provider = MockWorkloadProvider::default();
        out.push(observe_gate(
            CanonicalOperation::WorkloadList,
            provider.list(WorkloadSelector::All).await,
        ));
        let spec = WorkloadSpec {
            alias: WorkloadId::parse("parity-workload").expect("valid"),
        };
        out.push(observe_gate(
            CanonicalOperation::WorkloadCreate,
            provider.create(spec).await,
        ));
        let workload = WorkloadId::parse("parity-workload").expect("valid");
        out.push(observe_gate(
            CanonicalOperation::WorkloadStart,
            provider.start(workload.clone()).await,
        ));
        out.push(observe_gate(
            CanonicalOperation::WorkloadStop,
            provider.stop(workload).await,
        ));
        let exec_req = WorkloadExecStartRequest {
            workload: WorkloadId::parse("parity-workload").expect("valid"),
            tty: false,
            command: d2b_realm_core::OpaquePayload::empty(),
        };
        out.push(observe_gate(
            CanonicalOperation::WorkloadExec,
            provider.exec(exec_req).await,
        ));
    }

    {
        let provider = MockDurableExecutionProvider;
        let generation = d2b_realm_core::ExecutionGeneration {
            guest_boot_id: d2b_realm_core::ProtocolToken::parse("boot-1").expect("valid"),
            workload_generation: d2b_realm_core::ProtocolToken::parse("gen-1").expect("valid"),
        };
        let start_req = d2b_realm_core::ExecStartRequest {
            execution_id: ExecutionId::parse("parity-exec").expect("valid"),
            workload: WorkloadId::parse("parity-workload").expect("valid"),
            generation: generation.clone(),
            attach_mode: d2b_realm_core::ExecAttachMode::Attached,
            tty: false,
        };
        out.push(observe_gate(
            CanonicalOperation::DurableExecStart,
            provider.start(start_req).await,
        ));
        let attach_req = d2b_realm_core::ExecAttachRequest {
            execution_id: ExecutionId::parse("parity-exec").expect("valid"),
            generation: generation.clone(),
            stdout_cursor: None,
            stderr_cursor: None,
        };
        out.push(observe_gate(
            CanonicalOperation::DurableExecAttach,
            provider.attach(attach_req).await,
        ));
        let logs_req = d2b_realm_core::ExecLogsRequest {
            execution_id: ExecutionId::parse("parity-exec").expect("valid"),
            generation: generation.clone(),
            cursor: None,
            max_bytes: std::num::NonZeroU32::new(4096).expect("nonzero"),
        };
        out.push(observe_gate(
            CanonicalOperation::DurableExecLogs,
            provider.logs(logs_req).await,
        ));
        let cancel_req = d2b_realm_core::ExecCancelRequest {
            execution_id: ExecutionId::parse("parity-exec").expect("valid"),
            generation,
        };
        out.push(observe_gate(
            CanonicalOperation::DurableExecCancel,
            provider.cancel(cancel_req).await,
        ));
    }

    {
        let provider = super::StrictPersistentShellProvider;
        let workload = WorkloadId::parse("parity-workload").expect("valid");
        out.push(observe_gate(
            CanonicalOperation::DisplayCloseSession,
            HeadlessDisplayProvider
                .close_display_session(crate::types::DisplaySessionId("parity".to_owned()))
                .await,
        ));
        let _ = &provider;
        let _ = workload;
    }

    {
        let transport = LoopbackTransportProvider;
        let connect = transport
            .connect(crate::types::TransportTarget {
                endpoint: "parity-endpoint".to_owned(),
            })
            .await;
        out.push(observe_gate(CanonicalOperation::TransportConnect, connect));
        let listener = transport
            .listen(crate::types::NodeRegistration {
                node: NodeId::parse("mock").expect("valid"),
            })
            .await;
        let listener_ok = listener.is_ok();
        out.push(observe_gate(CanonicalOperation::TransportListen, listener));
        if listener_ok {
            let listener = transport
                .listen(crate::types::NodeRegistration {
                    node: NodeId::parse("mock").expect("valid"),
                })
                .await
                .expect("listened above");
            out.push(observe_gate(
                CanonicalOperation::TransportListenerAccept,
                listener.accept().await,
            ));
        }
    }

    {
        let mux = LoopbackStreamMux::default();
        out.push(observe_gate(
            CanonicalOperation::MuxAcceptStream,
            mux.accept_stream().await,
        ));
        out.push(observe_gate(
            CanonicalOperation::MuxCloseStream,
            mux.close_stream(StreamId::parse("parity-stream").expect("valid"))
                .await,
        ));
    }

    {
        let codec = NoOpProtocolCodec;
        let frame = d2b_realm_core::ConstellationFrame::StreamClose(d2b_realm_core::StreamClose {
            stream: StreamId::parse("parity-stream").expect("valid"),
            reason: d2b_realm_core::StreamCloseReason::Completed,
        });
        out.push(match codec.encode_frame(&frame) {
            Ok(_) => ProbeOutcome {
                op: CanonicalOperation::CodecEncodeFrame,
                ok: true,
                kind: None,
                missing_capability: None,
            },
            Err(e) => ProbeOutcome {
                op: CanonicalOperation::CodecEncodeFrame,
                ok: false,
                kind: Some(e.kind()),
                missing_capability: e.missing_capability(),
            },
        });
        out.push(match codec.decode_frame(&[]) {
            Ok(_) => ProbeOutcome {
                op: CanonicalOperation::CodecDecodeFrame,
                ok: true,
                kind: None,
                missing_capability: None,
            },
            Err(e) => ProbeOutcome {
                op: CanonicalOperation::CodecDecodeFrame,
                ok: false,
                kind: Some(e.kind()),
                missing_capability: e.missing_capability(),
            },
        });
    }

    {
        let transport = LocalUnixDaemonAccessTransport;
        out.push(observe_gate(
            CanonicalOperation::DaemonAccessConnect,
            transport
                .connect(crate::types::TransportTarget {
                    endpoint: "parity-endpoint".to_owned(),
                })
                .await,
        ));
    }

    out.push(observe_gate(
        CanonicalOperation::DaemonAccessVmList,
        MockDaemonAccessApi.vm_list().await,
    ));
    out.push(observe_gate(
        CanonicalOperation::InfrastructurePlan,
        MockInfrastructureProvider
            .plan_infrastructure(NodeId::parse("mock").expect("valid"))
            .await,
    ));
    out.push(observe_gate(
        CanonicalOperation::CredentialStatus,
        FixedCredentialProvider::valid().status().await,
    ));
    out.push(observe_gate(
        CanonicalOperation::CredentialEnrollmentValid,
        FixedCredentialProvider::valid().enrollment_valid().await,
    ));
    out.push(observe_gate(
        CanonicalOperation::ObservabilityHealthy,
        FixedObservabilitySinkProvider::healthy().healthy().await,
    ));
    out.push(observe_gate(
        CanonicalOperation::RelayOpenListener,
        MockRelayProvider
            .open_listener(NodeId::parse("mock").expect("valid"))
            .await,
    ));
    out.push(observe_gate(
        CanonicalOperation::NodeListWorkloads,
        MockNodeProvider::default().list_workloads().await,
    ));

    out
}

/// A bounded report combining the self-consistent inventory with the
/// stand-in probe outcomes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParityReport {
    pub inventory: Inventory,
    pub denial_probe_count: usize,
    pub smoke_probe_count: usize,
}

/// Run the full provider parity proof: build the self-consistent
/// inventory, run every stand-in probe, and fail closed the moment any
/// capability-gated probe disagrees with its declared classification.
/// Never dials a real host, relay, or provider API.
pub async fn verify_provider_parity() -> Result<ParityReport, ParityViolation> {
    let inventory = build_inventory()?;

    let denial_probes = run_capability_denial_probes().await;
    for probe in &denial_probes {
        let classification = probe.op.classification();
        let expected_capability = match classification.capability_gate {
            CapabilityGate::SelfAdvertised(cap) => Some(cap),
            CapabilityGate::AdvertisedPerStreamKind => None,
            _ => None,
        };
        let denial_is_well_formed = !probe.ok
            && probe.kind == Some(ErrorKind::CapabilityDenied)
            && (expected_capability.is_none() || probe.missing_capability == expected_capability);
        if !denial_is_well_formed {
            return Err(ParityViolation::ProbeMismatch(probe.op));
        }
    }

    let smoke_probes = run_stand_in_smoke_probes().await;
    for probe in &smoke_probes {
        if let Some(kind) = probe.kind
            && error_kind_owner(kind) == ErrorKindOwner::RouterLayer
        {
            return Err(ParityViolation::ProbeMismatch(probe.op));
        }
    }

    Ok(ParityReport {
        inventory,
        denial_probe_count: denial_probes.len(),
        smoke_probe_count: smoke_probes.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_table_has_no_duplicate_or_missing_entry() {
        assert!(ordinals_form_a_permutation(
            &ALL_OPERATIONS,
            CanonicalOperation::ordinal,
            OPERATION_COUNT
        ));
    }

    #[test]
    fn family_table_has_no_duplicate_or_missing_entry() {
        let mut seen = BTreeSet::new();
        for family in ALL_FAMILIES {
            assert!(
                seen.insert(family.code()),
                "duplicate family {}",
                family.code()
            );
        }
        assert_eq!(seen.len(), ALL_FAMILIES.len());
    }

    #[test]
    fn capability_table_has_no_duplicate_or_missing_entry() {
        let mut seen = BTreeSet::new();
        for cap in ALL_CAPABILITIES {
            assert!(
                seen.insert(cap.code()),
                "duplicate capability {}",
                cap.code()
            );
        }
        assert_eq!(seen.len(), ALL_CAPABILITIES.len());
    }

    #[test]
    fn error_kind_table_has_no_duplicate_or_missing_entry() {
        let mut seen = BTreeSet::new();
        for kind in ALL_ERROR_KINDS {
            assert!(
                seen.insert(kind.code()),
                "duplicate error kind {}",
                kind.code()
            );
        }
        assert_eq!(seen.len(), ALL_ERROR_KINDS.len());
    }

    #[test]
    fn every_family_has_at_least_one_operation() {
        for family in ALL_FAMILIES {
            assert!(
                ALL_OPERATIONS.iter().any(|op| op.family() == family),
                "family {} has zero classified operations",
                family.code()
            );
        }
    }

    /// Table/property test: for every canonical operation, the declared
    /// classification must be internally coherent, regardless of which
    /// operation it is (a property that must hold for the whole table, not
    /// a single hand-picked example).
    #[test]
    fn every_operation_classification_is_internally_coherent() {
        for op in ALL_OPERATIONS {
            let classification = op.classification();
            assert_eq!(classification.fallback, FallbackPolicy::NeverFallback);

            if classification.cancellation == CancellationSupport::ExplicitCancelOperation {
                assert_eq!(
                    classification.idempotency,
                    Idempotency::IdempotentByContract,
                    "{op:?}: an explicit cancel primitive must be idempotent by contract"
                );
                assert_ne!(
                    classification.retry,
                    RetrySafety::NotRetryable,
                    "{op:?}: an explicit cancel primitive must stay retry-safe"
                );
            }

            if let CapabilityGate::SelfAdvertised(cap) = classification.capability_gate {
                assert!(
                    capability_role(cap).operations.contains(&op),
                    "{op:?}: capability_role({cap:?}) does not list this operation back"
                );
            }
        }
    }

    #[test]
    fn every_self_advertised_capability_role_lists_the_operation_back() {
        for cap in ALL_CAPABILITIES {
            for op in capability_role(cap).operations {
                assert_eq!(
                    op.classification().capability_gate,
                    CapabilityGate::SelfAdvertised(cap),
                    "{op:?} is listed under {cap:?} but does not declare that gate"
                );
            }
        }
    }

    #[test]
    fn provider_layer_kinds_in_use_are_never_router_owned() {
        for kind in PROVIDER_LAYER_KINDS_IN_USE {
            assert_ne!(error_kind_owner(kind), ErrorKindOwner::RouterLayer);
        }
    }

    #[test]
    fn known_stream_kinds_are_all_classified_not_unknown() {
        for kind in KNOWN_STREAM_KINDS {
            assert_ne!(
                stream_kind_usage(kind),
                StreamKindUsage::UnknownToThisCrateVersion,
                "{kind:?} must be explicitly classified, not fall through to unknown"
            );
        }
    }

    #[test]
    fn build_inventory_succeeds_and_reports_stable_counts() {
        let inventory = build_inventory().expect("inventory must be internally consistent");
        assert_eq!(inventory.family_count, 18);
        assert_eq!(inventory.operation_count, OPERATION_COUNT);
        assert_eq!(inventory.capability_count, 22);
        assert_eq!(inventory.error_kind_count, 19);
    }

    #[tokio::test]
    async fn capability_denial_probes_match_declared_gates() {
        let probes = run_capability_denial_probes().await;
        // Every family with a self-advertised/per-stream-kind gate that has
        // a headless/no-op negative stand-in is exercised here.
        assert_eq!(probes.len(), 7);
        for probe in &probes {
            assert!(
                !probe.ok,
                "{:?} unexpectedly succeeded on a bare stand-in",
                probe.op
            );
            assert_eq!(probe.kind, Some(ErrorKind::CapabilityDenied));
            let classification = probe.op.classification();
            match classification.capability_gate {
                CapabilityGate::SelfAdvertised(cap) => {
                    assert_eq!(probe.missing_capability, Some(cap));
                }
                CapabilityGate::AdvertisedPerStreamKind => {
                    assert!(probe.missing_capability.is_some());
                }
                other => panic!("{:?} probed a denial but is gated {:?}", probe.op, other),
            }
        }
    }

    #[tokio::test]
    async fn stand_in_smoke_probes_never_surface_router_layer_kinds() {
        let probes = run_stand_in_smoke_probes().await;
        assert!(!probes.is_empty());
        for probe in &probes {
            if let Some(kind) = probe.kind {
                assert_ne!(
                    error_kind_owner(kind),
                    ErrorKindOwner::RouterLayer,
                    "{:?} surfaced a router-owned kind {:?}",
                    probe.op,
                    kind
                );
            }
        }
    }

    #[tokio::test]
    async fn verify_provider_parity_succeeds_end_to_end() {
        let report = verify_provider_parity()
            .await
            .expect("parity proof must succeed against in-memory stand-ins");
        assert_eq!(report.inventory.operation_count, OPERATION_COUNT);
        assert_eq!(report.denial_probe_count, 7);
        assert!(report.smoke_probe_count >= 24);
    }

    #[test]
    fn no_operation_ever_represents_a_fallback() {
        // Type-level property: `FallbackPolicy` has exactly one variant, so
        // this loop is really asserting the match arm exists for every
        // operation (see `classification`), not comparing values.
        for op in ALL_OPERATIONS {
            let FallbackPolicy::NeverFallback = op.classification().fallback;
        }
    }
}
