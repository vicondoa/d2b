//! Provider DTOs (ADR 0032). Pure-data DTOs are `serde`; runtime handles
//! that carry live byte channels (transport sessions, mux substreams) are
//! deliberately NOT `serde`/`Clone`/`Eq` and redact their contents in
//! `Debug`.

use d2b_realm_core::{
    Capability, CapabilitySet, ExecutionId, NodeId, OpaquePayload, OperationId, ProviderId,
    ShellAttachRequest, ShellAttachSummary, ShellDetachRequest, ShellGeneration, ShellKillRequest,
    ShellListRequest, ShellListResponse, StreamAuthz, StreamCursor, StreamId, WorkloadId,
    WorkloadSelector,
};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::capabilities::WorkloadCapabilitySet;

pub use d2b_realm_core::{StreamKind, StreamOpen};

/// A request to plan/run a workload, addressed by a stable alias.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadSpec {
    /// Stable operator-facing alias for the workload.
    pub alias: WorkloadId,
}

/// An opaque, provider-resolved runtime plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimePlan {
    /// Provider that produced the plan.
    pub provider: ProviderId,
    /// Workload the plan is for.
    pub workload: WorkloadId,
}

/// An opaque handle to a running runtime instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHandle {
    /// Workload the handle refers to.
    pub workload: WorkloadId,
}

/// Coarse runtime status.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeStatus {
    /// Workload.
    pub workload: WorkloadId,
    /// Whether the runtime is currently running.
    pub running: bool,
}

/// Coarse workload status returned by a [`crate::WorkloadProvider`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadStatus {
    /// Workload.
    pub workload: WorkloadId,
    /// Whether the workload is currently running.
    pub running: bool,
}

/// A request to start an execution in a workload. The `command` is an
/// opaque, codec-defined payload (argv + env + stdio policy) so the
/// provider trait never has to model shell semantics; its bytes are never
/// logged/audited as content.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecStartRequest {
    /// Workload to exec in.
    pub workload: WorkloadId,
    /// Whether a TTY is requested.
    pub tty: bool,
    /// Opaque, bounded command payload (argv/env/stdio descriptor).
    pub command: OpaquePayload,
}

/// A request to fetch (and optionally resume) the logs of an execution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecLogsRequest {
    /// Workload the execution belongs to.
    pub workload: WorkloadId,
    /// Execution whose logs are requested.
    pub execution: ExecutionId,
    /// Resume from this durable cursor; `None` streams from the start.
    pub cursor: Option<StreamCursor>,
}

/// A request to cancel a running execution (idempotent at the provider).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecCancelRequest {
    /// Workload the execution belongs to.
    pub workload: WorkloadId,
    /// Execution to cancel.
    pub execution: ExecutionId,
}

/// Non-secret guest-control capability metadata for one provider-managed
/// workload. It is not a socket address, relay URL, credential, or raw
/// guest-control endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestControlEndpointStatus {
    /// Provider that reported this status.
    pub provider: ProviderId,
    /// Node hosting the workload.
    pub node: NodeId,
    /// Workload whose guestd-compatible agent is reachable through the
    /// provider/relay peer transport.
    pub workload: WorkloadId,
    /// Positive capabilities advertised by the workload agent.
    pub capabilities: CapabilitySet,
    /// Current guest/shell generation reported by the agent.
    pub generation: ShellGeneration,
}

/// Request to list persistent shells for a provider-managed workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentShellListProviderRequest {
    /// Workload whose shells are listed.
    pub workload: WorkloadId,
    /// Operation that authorized the list.
    pub operation_id: OperationId,
    /// Bounded shell list DTO from the core contract.
    pub request: ShellListRequest,
}

/// Request to attach to a persistent shell for a provider-managed workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentShellAttachProviderRequest {
    /// Workload whose shell is attached.
    pub workload: WorkloadId,
    /// Operation that authorized the attach and shell PTY stream.
    pub operation_id: OperationId,
    /// Bounded shell attach DTO from the core contract.
    pub request: ShellAttachRequest,
    /// Already-authorized shell PTY stream open. Providers must reject this
    /// request if it is not `StreamKind::ShellPty` and internally consistent.
    pub shell_pty_stream: StreamOpen,
}

impl PersistentShellAttachProviderRequest {
    /// Whether the embedded stream open is a valid shell-authorized PTY open.
    pub fn shell_pty_stream_is_authorized(&self) -> bool {
        self.shell_pty_stream.descriptor.kind == StreamKind::ShellPty
            && self.shell_pty_stream.operation_id == self.operation_id
            && self.shell_pty_stream.is_consistent()
    }
}

/// Request to detach from a persistent shell for a provider-managed workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentShellDetachProviderRequest {
    /// Workload whose shell is detached.
    pub workload: WorkloadId,
    /// Operation that authorized the detach.
    pub operation_id: OperationId,
    /// Bounded shell detach DTO from the core contract.
    pub request: ShellDetachRequest,
}

/// Request to kill a persistent shell for a provider-managed workload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentShellKillProviderRequest {
    /// Workload whose shell is killed.
    pub workload: WorkloadId,
    /// Operation that authorized the kill.
    pub operation_id: OperationId,
    /// Bounded shell kill DTO from the core contract.
    pub request: ShellKillRequest,
}

/// Result returned by a provider-managed persistent-shell detach or kill.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersistentShellStatus {
    /// Current shell summary after the operation.
    pub summary: d2b_realm_core::ShellSummary,
}

/// Result returned by a provider-managed persistent-shell list.
pub type PersistentShellListProviderResponse = ShellListResponse;

/// Result returned by a provider-managed persistent-shell attach.
pub type PersistentShellAttachProviderResponse = ShellAttachSummary;

/// Non-secret provider guestd/agent bootstrap contract for a
/// provider-managed sandbox. This is a capability-advertisement input, not a
/// credential or endpoint DTO: it never carries relay URLs, tokens, image
/// digests, paths, argv, or host-held realm credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderGuestdBootstrapContract {
    /// Where the guestd-compatible agent binary comes from.
    pub guestd_binary: ProviderGuestdBinaryPlacement,
    /// How non-secret auth bootstrap is delivered to the sandbox.
    pub auth_material: ProviderGuestdAuthMaterial,
    /// How the agent learns its peer-transport rendezvous.
    pub relay_endpoint: ProviderGuestdRelayEndpoint,
    /// Workload identity available to the sandbox.
    pub workload_identity: ProviderWorkloadIdentity,
    /// Persistent-shell helper availability.
    pub shell_helper: ProviderShellHelperAvailability,
    /// Effective shell limits reported by the agent.
    pub shell_limits: Option<ProviderShellLimits>,
    /// Whether the agent can answer health/capabilities from inside the
    /// sandbox. Without this, capability advertisement must stay fail-closed.
    pub health_capability_advertisement: bool,
    /// Console capability for this sandbox's guestd-compatible agent (ADR
    /// 0041). `None` means the capability was not declared; routing fails
    /// closed and the daemon returns a typed provider-misconfigured error
    /// rather than falling back to a provider-specific shell side channel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub console: Option<ProviderConsoleCapability>,
    /// Audio enforcement capability for this sandbox (ADR 0041). `None`
    /// means the capability was not declared; routing fails closed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<ProviderAudioCapability>,
}

impl ProviderGuestdBootstrapContract {
    /// Execute-only provider sandboxes start fail-closed for persistent shell,
    /// console, and audio.
    pub fn execute_only_fail_closed() -> Self {
        Self {
            guestd_binary: ProviderGuestdBinaryPlacement::Absent,
            auth_material: ProviderGuestdAuthMaterial::Absent,
            relay_endpoint: ProviderGuestdRelayEndpoint::Absent,
            workload_identity: ProviderWorkloadIdentity::Absent,
            shell_helper: ProviderShellHelperAvailability::Absent,
            shell_limits: None,
            health_capability_advertisement: false,
            console: None,
            audio: None,
        }
    }

    /// True iff every ADR 0039 prerequisite is present and internally bounded.
    pub fn persistent_shell_ready(&self) -> bool {
        matches!(
            self.guestd_binary,
            ProviderGuestdBinaryPlacement::ImageOwned
        ) && matches!(
            self.auth_material,
            ProviderGuestdAuthMaterial::RelayScopedEphemeral
        ) && matches!(
            self.relay_endpoint,
            ProviderGuestdRelayEndpoint::PeerTransport
        ) && matches!(
            self.workload_identity,
            ProviderWorkloadIdentity::ManagedIdentity
        ) && matches!(
            self.shell_helper,
            ProviderShellHelperAvailability::Available
        ) && self
            .shell_limits
            .as_ref()
            .is_some_and(ProviderShellLimits::valid)
            && self.health_capability_advertisement
    }

    /// Positive workload capabilities implied by this bootstrap status.
    pub fn advertised_capabilities(&self) -> WorkloadCapabilitySet {
        let mut caps = CapabilitySet::empty().with(Capability::ProviderManagedIsolation);
        if self.persistent_shell_ready() {
            caps = caps.with(Capability::PersistentShell);
        }
        if self.console_ready() {
            caps = caps.with(Capability::Pty);
        }
        if self.audio_guest_enforcement_ready() {
            caps = caps
                .with(Capability::AudioPlayback)
                .with(Capability::AudioCapture);
        }
        WorkloadCapabilitySet { caps }
    }

    /// True iff the sandbox's guestd-compatible agent advertises a
    /// guestd-backed console capability (ADR 0041).
    pub fn console_ready(&self) -> bool {
        matches!(self.console, Some(ProviderConsoleCapability::GuestdBacked))
    }

    /// True iff guest-side audio enforcement is available via the
    /// guestd-compatible agent (ADR 0041).
    pub fn audio_guest_enforcement_ready(&self) -> bool {
        matches!(
            self.audio,
            Some(ProviderAudioCapability {
                guest_enforcement: ProviderAudioGuestEnforcement::GuestdCapable,
                ..
            })
        )
    }
}

// ---- Provider console/audio capability types (ADR 0041) ---------------------

/// Console capability advertised by a provider-managed sandbox's
/// guestd-compatible agent. This is a typed capability descriptor, not a relay
/// URL, resource identifier, or credential: it never exposes where or how the
/// agent runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderConsoleCapability {
    /// guestd-backed console stream available over the provider peer transport.
    /// The daemon routes `ConsoleOp::Attach` through the authenticated
    /// guest-control relay; no local socket or broker fd is used.
    GuestdBacked,
    /// No console capability is present in this sandbox. The daemon returns a
    /// typed provider-misconfigured error with remediation rather than falling
    /// back to a provider-specific shell side channel.
    Absent,
}

/// Audio enforcement posture for a provider-managed sandbox (ADR 0041).
///
/// Host-side PipeWire/vhost-user-sound enforcement is never available for
/// provider-managed sandboxes; only guest-side guestd policy applies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAudioCapability {
    /// Host audio enforcement: always `Absent` for provider-managed sandboxes
    /// (no local PipeWire node or vhost-user-sound sidecar is created).
    pub host_enforcement: ProviderAudioHostEnforcement,
    /// Guest-side audio enforcement via the guestd-compatible sandbox agent.
    pub guest_enforcement: ProviderAudioGuestEnforcement,
}

impl ProviderAudioCapability {
    /// No audio capability (execute-only / missing-guestd failure posture).
    pub fn absent() -> Self {
        Self {
            host_enforcement: ProviderAudioHostEnforcement::Absent,
            guest_enforcement: ProviderAudioGuestEnforcement::Unsupported,
        }
    }

    /// Full guestd-backed guest enforcement; no host enforcement (the
    /// standard ACA sandbox audio posture).
    pub fn guestd_guest_only() -> Self {
        Self {
            host_enforcement: ProviderAudioHostEnforcement::Absent,
            guest_enforcement: ProviderAudioGuestEnforcement::GuestdCapable,
        }
    }
}

/// Host-side audio enforcement for a provider-managed sandbox.
///
/// Provider sandboxes never control local PipeWire or vhost-user-sound nodes
/// on the host; this enum has a single value to make the posture explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderAudioHostEnforcement {
    /// No host audio enforcement (the only valid value for provider-managed
    /// sandboxes).
    Absent,
}

/// Guest-side audio enforcement for a provider-managed sandbox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderAudioGuestEnforcement {
    /// guestd-capable audio policy over the provider peer transport.
    GuestdCapable,
    /// Guest-side enforcement is unsupported (missing guestd, execute-only
    /// provider, or capability not declared by the sandbox agent).
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderGuestdBinaryPlacement {
    Absent,
    /// The provider image contains the guestd-compatible agent binary at a
    /// provider-defined location.
    ImageOwned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderGuestdAuthMaterial {
    Absent,
    /// Short-lived, relay-scoped bootstrap material delivered to the sandbox;
    /// long-lived realm/provider credentials remain gateway-side only.
    RelayScopedEphemeral,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderGuestdRelayEndpoint {
    Absent,
    /// The agent joins the constellation peer transport; no raw guest-control
    /// or provider-specific shell channel is exposed.
    PeerTransport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderWorkloadIdentity {
    Absent,
    ManagedIdentity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProviderShellHelperAvailability {
    Absent,
    Available,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderShellLimits {
    pub max_sessions: u32,
    pub max_attached: u32,
}

impl ProviderShellLimits {
    pub fn valid(&self) -> bool {
        (1..=256).contains(&self.max_sessions)
            && (1..=64).contains(&self.max_attached)
            && self.max_attached <= self.max_sessions
    }
}

/// A display-session id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySessionId(pub String);

/// A request to open a display session for a workload. The request carries
/// the **authorized display-stream binding** the mux must already hold: a
/// Waypipe byte never flows until there is an accepted `StreamOpen` for
/// `display_stream` under `authz`, bound to `operation_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySessionRequest {
    /// Workload presenting the UI.
    pub workload: WorkloadId,
    /// The operation that authorized this display session (audit +
    /// idempotency binding).
    pub operation_id: OperationId,
    /// The authorized display stream this session drives.
    pub display_stream: StreamId,
    /// The authorization context (principal/realm/derived capability) the
    /// gateway validated for the display stream.
    pub authz: StreamAuthz,
}

/// An opaque handle to an open display session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DisplaySessionHandle {
    /// Session id.
    pub id: DisplaySessionId,
}

/// A selector used by [`crate::WorkloadProvider::list`].
pub type ListSelector = WorkloadSelector;

/// A node registration handle (transport listener side).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRegistration {
    /// Node being registered.
    pub node: NodeId,
}

/// A transport-level target to connect to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransportTarget {
    /// Opaque transport endpoint reference (e.g. a relay rendezvous id).
    pub endpoint: String,
}

/// Maximum length of a [`SafeLabel`].
pub const MAX_LABEL_LEN: usize = 64;

/// A bounded, low-cardinality, non-secret diagnostic label. It MUST carry
/// a stable classification (e.g. `relay-session`, `loopback`), never an
/// endpoint, store path, argv, or secret. The length is bounded so it can
/// never become an unbounded/high-cardinality side channel.
#[derive(Clone, PartialEq, Eq)]
pub struct SafeLabel(String);

impl SafeLabel {
    /// Build a bounded label (truncated to [`MAX_LABEL_LEN`]).
    pub fn new(label: impl Into<String>) -> Self {
        let mut s = label.into();
        if s.len() > MAX_LABEL_LEN {
            let mut end = MAX_LABEL_LEN;
            while end > 0 && !s.is_char_boundary(end) {
                end -= 1;
            }
            s.truncate(end);
        }
        Self(s)
    }

    /// The label text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl core::fmt::Debug for SafeLabel {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "SafeLabel({:?})", self.0)
    }
}

/// A bidirectional byte channel: a connected transport session or an
/// accepted mux substream. Implemented by any `AsyncRead + AsyncWrite`
/// (e.g. a `tokio::io::DuplexStream` for the loopback mock, or a relay
/// WebSocket adapter later).
pub trait ByteStream: AsyncRead + AsyncWrite + Send + Unpin {}
impl<T: AsyncRead + AsyncWrite + Send + Unpin + ?Sized> ByteStream for T {}

/// A connected transport session: a bidirectional byte channel below the
/// mux, plus a bounded non-secret label. Not `Clone`/`Eq`/`serde` (it owns
/// a live stream); `Debug` reveals only the label.
pub struct TransportSession {
    label: SafeLabel,
    stream: Box<dyn ByteStream>,
}

impl TransportSession {
    /// Wrap a connected byte stream with a bounded diagnostic label.
    pub fn new(label: SafeLabel, stream: Box<dyn ByteStream>) -> Self {
        Self { label, stream }
    }

    /// The non-secret diagnostic label.
    pub fn label(&self) -> &str {
        self.label.as_str()
    }

    /// Borrow the underlying byte channel.
    pub fn stream_mut(&mut self) -> &mut dyn ByteStream {
        &mut *self.stream
    }

    /// Take ownership of the underlying byte channel.
    pub fn into_stream(self) -> Box<dyn ByteStream> {
        self.stream
    }
}

impl core::fmt::Debug for TransportSession {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("TransportSession")
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// An opened mux substream: the authorized open plus its byte channel.
pub struct StreamHandle {
    /// The stream id.
    pub id: StreamId,
    stream: Box<dyn ByteStream>,
}

impl StreamHandle {
    /// Wrap an opened substream.
    pub fn new(id: StreamId, stream: Box<dyn ByteStream>) -> Self {
        Self { id, stream }
    }

    /// Borrow the underlying byte channel.
    pub fn stream_mut(&mut self) -> &mut dyn ByteStream {
        &mut *self.stream
    }

    /// Take ownership of the underlying byte channel.
    pub fn into_stream(self) -> Box<dyn ByteStream> {
        self.stream
    }
}

impl core::fmt::Debug for StreamHandle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StreamHandle")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// An accepted inbound mux substream: the (already-authorized) open and
/// its byte channel.
pub struct IncomingStream {
    /// The authorized stream open (descriptor + authz).
    pub open: StreamOpen,
    stream: Box<dyn ByteStream>,
}

impl IncomingStream {
    /// Wrap an accepted substream.
    pub fn new(open: StreamOpen, stream: Box<dyn ByteStream>) -> Self {
        Self { open, stream }
    }

    /// Take ownership of the underlying byte channel.
    pub fn into_stream(self) -> Box<dyn ByteStream> {
        self.stream
    }
}

impl core::fmt::Debug for IncomingStream {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("IncomingStream")
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}

/// A daemon-access transport mode (which `d2bd` transport the CLI uses).
/// Only [`DaemonAccessMode::LocalUnix`] is implemented today; the others are
/// declared slots that fail closed with `UnsupportedFeature` until a later
/// wave implements them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum DaemonAccessMode {
    /// Local `public.sock` Unix-domain socket (current behavior).
    LocalUnix,
    /// Relay-backed (Azure Relay hybrid connection); later wave.
    Relay,
    /// Direct mTLS/QUIC/WebSocket; later wave.
    DirectTls,
    /// Explicit SSH bootstrap; later wave.
    SshBootstrap,
}

impl DaemonAccessMode {
    /// Whether this mode is implemented today.
    pub fn is_implemented(self) -> bool {
        matches!(self, DaemonAccessMode::LocalUnix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_realm_core::{
        Capability, PrincipalId, RealmPath, ShellAttachId, ShellAttachRequest, ShellName,
        StreamDescriptor,
    };

    fn shell_generation() -> ShellGeneration {
        ShellGeneration {
            guest_boot_id: d2b_realm_core::ProtocolToken::parse("boot-a").unwrap(),
            guestd_instance_id: d2b_realm_core::ProtocolToken::parse("guestd-a").unwrap(),
            shell_daemon_instance_id: d2b_realm_core::ProtocolToken::parse("shell-a").unwrap(),
        }
    }

    fn stream_open(kind: StreamKind, authz_capability: Capability) -> StreamOpen {
        StreamOpen {
            descriptor: StreamDescriptor {
                id: StreamId::parse("shell-pty-1").unwrap(),
                kind,
            },
            operation_id: OperationId::parse("op-shell-1").unwrap(),
            authz: StreamAuthz {
                principal: PrincipalId::parse("principal-1").unwrap(),
                realm: RealmPath::local(),
                capability: authz_capability,
            },
        }
    }

    fn attach_request(shell_pty_stream: StreamOpen) -> PersistentShellAttachProviderRequest {
        PersistentShellAttachProviderRequest {
            workload: WorkloadId::parse("demo").unwrap(),
            operation_id: OperationId::parse("op-shell-1").unwrap(),
            request: ShellAttachRequest {
                name: ShellName::parse("default").unwrap(),
                generation: shell_generation(),
                attach_id: ShellAttachId::parse("attach-1").unwrap(),
                force: false,
            },
            shell_pty_stream,
        }
    }

    #[test]
    fn persistent_shell_attach_requires_shell_authorized_pty_stream() {
        let valid = attach_request(stream_open(
            StreamKind::ShellPty,
            Capability::PersistentShell,
        ));
        assert!(valid.shell_pty_stream_is_authorized());

        let forged = attach_request(stream_open(StreamKind::ShellPty, Capability::Pty));
        assert!(!forged.shell_pty_stream_is_authorized());

        let wrong_kind = attach_request(stream_open(StreamKind::Stdio, Capability::Pty));
        assert!(!wrong_kind.shell_pty_stream_is_authorized());

        let mut wrong_op = stream_open(StreamKind::ShellPty, Capability::PersistentShell);
        wrong_op.operation_id = OperationId::parse("op-other").unwrap();
        assert!(!attach_request(wrong_op).shell_pty_stream_is_authorized());
    }

    #[test]
    fn provider_guestd_bootstrap_advertises_shell_only_when_complete() {
        let fail_closed = ProviderGuestdBootstrapContract::execute_only_fail_closed();
        assert!(!fail_closed.persistent_shell_ready());
        assert!(
            !fail_closed
                .advertised_capabilities()
                .has(Capability::PersistentShell)
        );

        let mut ready = ProviderGuestdBootstrapContract {
            guestd_binary: ProviderGuestdBinaryPlacement::ImageOwned,
            auth_material: ProviderGuestdAuthMaterial::RelayScopedEphemeral,
            relay_endpoint: ProviderGuestdRelayEndpoint::PeerTransport,
            workload_identity: ProviderWorkloadIdentity::ManagedIdentity,
            shell_helper: ProviderShellHelperAvailability::Available,
            shell_limits: Some(ProviderShellLimits {
                max_sessions: 8,
                max_attached: 1,
            }),
            health_capability_advertisement: true,
            console: None,
            audio: None,
        };
        assert!(ready.persistent_shell_ready());
        assert!(
            ready
                .advertised_capabilities()
                .has(Capability::PersistentShell)
        );

        ready.shell_limits = Some(ProviderShellLimits {
            max_sessions: 1,
            max_attached: 2,
        });
        assert!(!ready.persistent_shell_ready());
        assert!(
            !ready
                .advertised_capabilities()
                .has(Capability::PersistentShell)
        );
    }

    #[test]
    fn provider_guestd_console_ready_requires_guestd_backed() {
        let mut contract = ProviderGuestdBootstrapContract::execute_only_fail_closed();
        assert!(!contract.console_ready());
        assert!(!contract.advertised_capabilities().has(Capability::Pty));

        contract.console = Some(ProviderConsoleCapability::Absent);
        assert!(!contract.console_ready());

        contract.console = Some(ProviderConsoleCapability::GuestdBacked);
        assert!(contract.console_ready());
        assert!(contract.advertised_capabilities().has(Capability::Pty));
    }

    #[test]
    fn provider_guestd_audio_enforcement_ready_requires_guestd_capable() {
        let mut contract = ProviderGuestdBootstrapContract::execute_only_fail_closed();
        assert!(!contract.audio_guest_enforcement_ready());
        assert!(
            !contract
                .advertised_capabilities()
                .has(Capability::AudioPlayback)
        );
        assert!(
            !contract
                .advertised_capabilities()
                .has(Capability::AudioCapture)
        );

        contract.audio = Some(ProviderAudioCapability::absent());
        assert!(!contract.audio_guest_enforcement_ready());

        contract.audio = Some(ProviderAudioCapability::guestd_guest_only());
        assert!(contract.audio_guest_enforcement_ready());
        assert!(
            contract
                .advertised_capabilities()
                .has(Capability::AudioPlayback)
        );
        assert!(
            contract
                .advertised_capabilities()
                .has(Capability::AudioCapture)
        );
    }

    #[test]
    fn execute_only_fail_closed_has_no_console_or_audio() {
        let contract = ProviderGuestdBootstrapContract::execute_only_fail_closed();
        assert!(contract.console.is_none());
        assert!(contract.audio.is_none());
        assert!(!contract.console_ready());
        assert!(!contract.audio_guest_enforcement_ready());
    }

    #[test]
    fn provider_audio_capability_constructors_match_adr_0041() {
        // absent: no host or guest enforcement.
        let absent = ProviderAudioCapability::absent();
        assert_eq!(
            absent.host_enforcement,
            ProviderAudioHostEnforcement::Absent
        );
        assert_eq!(
            absent.guest_enforcement,
            ProviderAudioGuestEnforcement::Unsupported
        );

        // guestd_guest_only: no host; guest via guestd.
        let guest_only = ProviderAudioCapability::guestd_guest_only();
        assert_eq!(
            guest_only.host_enforcement,
            ProviderAudioHostEnforcement::Absent
        );
        assert_eq!(
            guest_only.guest_enforcement,
            ProviderAudioGuestEnforcement::GuestdCapable
        );
    }

    #[test]
    fn provider_guestd_bootstrap_contract_console_audio_fields_are_optional_in_serde() {
        // Serializing an execute-only contract (console/audio = None) must
        // produce JSON without those keys, and deserializing the old JSON
        // shape (without console/audio keys) must succeed fail-closed.
        let contract = ProviderGuestdBootstrapContract::execute_only_fail_closed();
        let json = serde_json::to_string(&contract).expect("serialize");
        assert!(
            !json.contains("console"),
            "absent console must not appear in JSON"
        );
        assert!(
            !json.contains("audio"),
            "absent audio must not appear in JSON"
        );

        // A JSON blob without console/audio keys (from an older peer) must
        // deserialize with console/audio defaulting to None.
        let old_json = serde_json::json!({
            "guestd_binary": "absent",
            "auth_material": "absent",
            "relay_endpoint": "absent",
            "workload_identity": "absent",
            "shell_helper": "absent",
            "shell_limits": null,
            "health_capability_advertisement": false
        });
        let decoded: ProviderGuestdBootstrapContract =
            serde_json::from_value(old_json).expect("old JSON deserializes");
        assert!(decoded.console.is_none());
        assert!(decoded.audio.is_none());
    }
}
