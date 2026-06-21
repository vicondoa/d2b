//! Transport-neutral CLI-to-`nixlingd` daemon access (ADR 0032).
//!
//! The local binding intentionally speaks the existing public daemon wire:
//! AF_UNIX `SOCK_SEQPACKET`, one 4-byte little-endian length-prefixed JSON
//! body per packet, `hello` negotiation, then the current type-tagged `list`
//! request. The primary `vm_list` API returns a daemon-access-local shape that
//! preserves the public-wire list response exactly; the v2 [`WorkloadSummary`]
//! projection remains available only as an explicitly lossy compatibility
//! helper.

pub mod direct_tls;
pub mod relay;

use std::{
    fmt, io,
    os::{fd::AsRawFd, unix::net::UnixStream as StdUnixStream},
    path::{Path, PathBuf},
};

use async_trait::async_trait;
use nix::sys::socket::{AddressFamily, SockFlag, SockType, UnixAddr, connect, socket};
use nixling_constellation_core::{
    Capability, CapabilitySet, ErrorKind, NodeId, PrincipalId, ProviderId, RealmPath, WorkloadId,
    WorkloadState, WorkloadSummary,
};
use nixling_constellation_provider::{
    error::{ProviderError, ProviderResult},
    provider::{DaemonAccessApi, DaemonAccessTransport},
    types::{DaemonAccessMode, SafeLabel, TransportSession, TransportTarget},
};
use nixling_ipc::{
    FeatureFlag, Hello, HelloOk, HelloRejected, KnownFeatureFlag, MAX_FRAME_SIZE,
    PUBLIC_SOCKET_PATH, SemverRange,
    public_wire::{
        ListEntry, ListRequest, ListResponse, PublicVmServices, RuntimeSummary, VmLifecycle,
        VmLifecycleState,
    },
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub use direct_tls::DirectTlsDaemonAccess;
pub use relay::RelayDaemonAccess;

/// Default daemon public socket used by the current CLI and `nixlingd`.
pub const DEFAULT_PUBLIC_SOCKET_PATH: &str = PUBLIC_SOCKET_PATH;
/// Stable transport id for the local Unix daemon-access binding.
pub const LOCAL_UNIX_DAEMON_ACCESS_TRANSPORT_ID: &str = "local-unix-daemon-access";
/// Stable transport id for the declared relay daemon-access slot.
pub const RELAY_DAEMON_ACCESS_TRANSPORT_ID: &str = "relay-daemon-access";
/// Stable transport id for the declared direct-TLS daemon-access slot.
pub const DIRECT_TLS_DAEMON_ACCESS_TRANSPORT_ID: &str = "direct-tls-daemon-access";
/// Credential byte-length metadata is capped before serialization.
pub const MAX_REDACTED_CREDENTIAL_BYTES: u16 = u16::MAX;
const DEFAULT_CLIENT_VERSION_RANGE: &str = ">=0.4.0, <0.5.0";
const LOCAL_NODE_ID: &str = "local";

/// Redacted evidence that a credential was presented.
///
/// The daemon-access admission/audit model records only presence and bounded
/// byte length. It never stores certificate PEM/DER, bearer tokens, browser
/// cookies, CSRF values, or relay secrets.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RedactedDaemonAccessCredential {
    pub present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub byte_len: Option<u16>,
}

impl RedactedDaemonAccessCredential {
    /// No credential material was presented.
    pub const fn absent() -> Self {
        Self {
            present: false,
            byte_len: None,
        }
    }

    /// Record credential presence while discarding the material itself.
    pub fn from_secret(secret: impl AsRef<[u8]>) -> Self {
        let byte_len = secret
            .as_ref()
            .len()
            .min(MAX_REDACTED_CREDENTIAL_BYTES as usize) as u16;
        Self {
            present: true,
            byte_len: Some(byte_len),
        }
    }
}

impl fmt::Debug for RedactedDaemonAccessCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.present {
            f.debug_tuple("RedactedDaemonAccessCredential")
                .field(&format_args!(
                    "<{} bytes redacted>",
                    self.byte_len.unwrap_or_default()
                ))
                .finish()
        } else {
            f.write_str("RedactedDaemonAccessCredential(<absent>)")
        }
    }
}

/// Credential evidence for a remote direct daemon-access frontend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum RemoteDaemonAccessCredential {
    None,
    MutualTls {
        certificate: RedactedDaemonAccessCredential,
    },
    BearerToken {
        token: RedactedDaemonAccessCredential,
    },
    MutualTlsAndBearerToken {
        certificate: RedactedDaemonAccessCredential,
        token: RedactedDaemonAccessCredential,
    },
}

impl RemoteDaemonAccessCredential {
    /// No authenticated daemon-access credential was presented.
    pub const fn unauthenticated() -> Self {
        Self::None
    }

    /// mTLS credential evidence; certificate material is discarded.
    pub fn mutual_tls(certificate: impl AsRef<[u8]>) -> Self {
        Self::MutualTls {
            certificate: RedactedDaemonAccessCredential::from_secret(certificate),
        }
    }

    /// Bearer-token credential evidence; token material is discarded.
    pub fn bearer_token(token: impl AsRef<[u8]>) -> Self {
        Self::BearerToken {
            token: RedactedDaemonAccessCredential::from_secret(token),
        }
    }

    /// Combined mTLS + bearer-token evidence; both inputs are discarded.
    pub fn mutual_tls_and_bearer_token(
        certificate: impl AsRef<[u8]>,
        token: impl AsRef<[u8]>,
    ) -> Self {
        Self::MutualTlsAndBearerToken {
            certificate: RedactedDaemonAccessCredential::from_secret(certificate),
            token: RedactedDaemonAccessCredential::from_secret(token),
        }
    }
}

/// Relay daemon-access credential evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelayDaemonAccessCredential {
    pub relay_credential: RedactedDaemonAccessCredential,
    pub daemon_credential: RemoteDaemonAccessCredential,
}

impl RelayDaemonAccessCredential {
    /// Relay reachability without an authenticated daemon principal.
    pub const fn unauthenticated() -> Self {
        Self {
            relay_credential: RedactedDaemonAccessCredential::absent(),
            daemon_credential: RemoteDaemonAccessCredential::unauthenticated(),
        }
    }

    /// Relay-backed credential evidence; relay secret material is discarded.
    pub fn from_relay_secret(
        relay_secret: impl AsRef<[u8]>,
        daemon_credential: RemoteDaemonAccessCredential,
    ) -> Self {
        Self {
            relay_credential: RedactedDaemonAccessCredential::from_secret(relay_secret),
            daemon_credential,
        }
    }
}

/// Browser daemon-access credential evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum BrowserDaemonAccessCredential {
    None,
    CookieSession {
        cookie: RedactedDaemonAccessCredential,
        csrf_token: RedactedDaemonAccessCredential,
    },
    BearerSession {
        token: RedactedDaemonAccessCredential,
        csrf_token: RedactedDaemonAccessCredential,
    },
}

impl BrowserDaemonAccessCredential {
    /// No authenticated browser credential was presented.
    pub const fn unauthenticated() -> Self {
        Self::None
    }

    /// Cookie-backed browser session evidence; cookie and CSRF material are discarded.
    pub fn cookie_session(cookie: impl AsRef<[u8]>, csrf_token: impl AsRef<[u8]>) -> Self {
        Self::CookieSession {
            cookie: RedactedDaemonAccessCredential::from_secret(cookie),
            csrf_token: RedactedDaemonAccessCredential::from_secret(csrf_token),
        }
    }

    /// Bearer-backed browser session evidence; token and CSRF material are discarded.
    pub fn bearer_session(token: impl AsRef<[u8]>, csrf_token: impl AsRef<[u8]>) -> Self {
        Self::BearerSession {
            token: RedactedDaemonAccessCredential::from_secret(token),
            csrf_token: RedactedDaemonAccessCredential::from_secret(csrf_token),
        }
    }
}

/// Local Unix `SO_PEERCRED` admission source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalUnixAdmissionSource {
    pub uid: u32,
    pub gid: u32,
}

impl LocalUnixAdmissionSource {
    /// Construct a local Unix peer credential source.
    pub const fn new(uid: u32, gid: u32) -> Self {
        Self { uid, gid }
    }
}

/// Remote direct daemon-access admission source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RemoteDaemonAccessAdmissionSource {
    pub transport_id: ProviderId,
    pub mode: DaemonAccessMode,
    pub credential: RemoteDaemonAccessCredential,
    pub principal_id: Option<PrincipalId>,
}

impl RemoteDaemonAccessAdmissionSource {
    /// Construct a remote admission source from transport-advertised metadata.
    pub fn new(
        transport_id: ProviderId,
        mode: DaemonAccessMode,
        credential: RemoteDaemonAccessCredential,
        principal_id: Option<PrincipalId>,
    ) -> Self {
        Self {
            transport_id,
            mode,
            credential,
            principal_id,
        }
    }
}

/// Relay-backed daemon-access admission source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelayDaemonAccessAdmissionSource {
    pub transport_id: ProviderId,
    pub mode: DaemonAccessMode,
    pub credential: RelayDaemonAccessCredential,
    pub principal_id: Option<PrincipalId>,
}

impl RelayDaemonAccessAdmissionSource {
    /// Construct a relay admission source from transport-advertised metadata.
    pub fn new(
        transport_id: ProviderId,
        mode: DaemonAccessMode,
        credential: RelayDaemonAccessCredential,
        principal_id: Option<PrincipalId>,
    ) -> Self {
        Self {
            transport_id,
            mode,
            credential,
            principal_id,
        }
    }
}

/// Browser web-session daemon-access admission source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BrowserDaemonAccessAdmissionSource {
    pub credential: BrowserDaemonAccessCredential,
    pub session_id: Option<PrincipalId>,
}

impl BrowserDaemonAccessAdmissionSource {
    /// Construct a browser admission source.
    pub fn new(credential: BrowserDaemonAccessCredential, session_id: Option<PrincipalId>) -> Self {
        Self {
            credential,
            session_id,
        }
    }
}

/// Transport-neutral daemon-access admission source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source", rename_all = "camelCase")]
pub enum DaemonAccessAdmissionSource {
    LocalUnix(LocalUnixAdmissionSource),
    Remote(RemoteDaemonAccessAdmissionSource),
    Relay(RelayDaemonAccessAdmissionSource),
    Browser(BrowserDaemonAccessAdmissionSource),
}

impl DaemonAccessAdmissionSource {
    /// Construct a local Unix source from `SO_PEERCRED`.
    pub const fn local_unix(uid: u32, gid: u32) -> Self {
        Self::LocalUnix(LocalUnixAdmissionSource::new(uid, gid))
    }

    /// Construct a direct-TLS source using the declared direct-TLS transport id.
    pub fn direct_tls(
        credential: RemoteDaemonAccessCredential,
        principal_id: Option<PrincipalId>,
    ) -> Self {
        Self::Remote(RemoteDaemonAccessAdmissionSource::new(
            ProviderId::parse(DIRECT_TLS_DAEMON_ACCESS_TRANSPORT_ID)
                .expect("static provider id is valid"),
            DaemonAccessMode::DirectTls,
            credential,
            principal_id,
        ))
    }

    /// Construct a relay source using the declared relay transport id.
    pub fn relay(
        credential: RelayDaemonAccessCredential,
        principal_id: Option<PrincipalId>,
    ) -> Self {
        Self::Relay(RelayDaemonAccessAdmissionSource::new(
            ProviderId::parse(RELAY_DAEMON_ACCESS_TRANSPORT_ID)
                .expect("static provider id is valid"),
            DaemonAccessMode::Relay,
            credential,
            principal_id,
        ))
    }
}

/// Resolved local Unix allowlist/group role.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LocalUnixAllowlistRole {
    Admin,
    Launcher,
    Denied,
}

/// Explicit daemon-access policy role for non-local principals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonAccessPolicyRole {
    RealmAdmin,
    RealmOperator,
    ReadOnly,
    Scoped,
}

/// Bounded fail-closed admission denial reason.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DaemonAccessDenyReason {
    LocalNotAllowlisted,
    Unauthenticated,
    Unmapped,
}

/// Admission decision for daemon-access principals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "camelCase")]
pub enum DaemonAccessDecision {
    Authorized { role: DaemonAccessPolicyRole },
    Denied { reason: DaemonAccessDenyReason },
}

impl DaemonAccessDecision {
    /// An authorized admission with an explicit daemon-access policy role.
    pub const fn authorized(role: DaemonAccessPolicyRole) -> Self {
        Self::Authorized { role }
    }

    /// A denied admission with a bounded reason.
    pub const fn denied(reason: DaemonAccessDenyReason) -> Self {
        Self::Denied { reason }
    }

    /// Whether this admission was denied.
    pub const fn is_denied(&self) -> bool {
        matches!(self, Self::Denied { .. })
    }
}

/// Transport-neutral mapped principal result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum MappedDaemonAccessPrincipal {
    LocalAdmin {
        uid: u32,
    },
    LocalLauncher {
        uid: u32,
    },
    LocalDenied {
        uid: u32,
        reason: DaemonAccessDenyReason,
    },
    RemotePrincipal {
        principal_id: Option<PrincipalId>,
        decision: DaemonAccessDecision,
    },
    BrowserSession {
        session_id: Option<PrincipalId>,
        decision: DaemonAccessDecision,
    },
    RelayPrincipal {
        principal_id: Option<PrincipalId>,
        decision: DaemonAccessDecision,
    },
}

impl MappedDaemonAccessPrincipal {
    /// True only for local `SO_PEERCRED` identities admitted by the local gate.
    pub const fn is_local_admin_or_launcher(&self) -> bool {
        matches!(self, Self::LocalAdmin { .. } | Self::LocalLauncher { .. })
    }

    /// Whether this principal mapping was denied.
    pub const fn is_denied(&self) -> bool {
        match self {
            Self::LocalDenied { .. } => true,
            Self::RemotePrincipal { decision, .. }
            | Self::BrowserSession { decision, .. }
            | Self::RelayPrincipal { decision, .. } => decision.is_denied(),
            Self::LocalAdmin { .. } | Self::LocalLauncher { .. } => false,
        }
    }
}

/// Auditable admission record pairing source and mapped principal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaemonAccessAdmissionRecord {
    pub source: DaemonAccessAdmissionSource,
    pub principal: MappedDaemonAccessPrincipal,
}

/// Map a local Unix `SO_PEERCRED` source using the existing allowlist role.
pub fn map_local_unix_daemon_access(
    source: &LocalUnixAdmissionSource,
    role: LocalUnixAllowlistRole,
) -> MappedDaemonAccessPrincipal {
    match role {
        LocalUnixAllowlistRole::Admin => {
            MappedDaemonAccessPrincipal::LocalAdmin { uid: source.uid }
        }
        LocalUnixAllowlistRole::Launcher => {
            MappedDaemonAccessPrincipal::LocalLauncher { uid: source.uid }
        }
        LocalUnixAllowlistRole::Denied => MappedDaemonAccessPrincipal::LocalDenied {
            uid: source.uid,
            reason: DaemonAccessDenyReason::LocalNotAllowlisted,
        },
    }
}

/// Map a remote direct daemon-access source through explicit daemon policy.
pub fn map_remote_daemon_access(
    source: &RemoteDaemonAccessAdmissionSource,
    policy_role: Option<DaemonAccessPolicyRole>,
) -> MappedDaemonAccessPrincipal {
    MappedDaemonAccessPrincipal::RemotePrincipal {
        principal_id: source.principal_id.clone(),
        decision: non_local_decision(source.principal_id.as_ref(), policy_role),
    }
}

/// Map a browser daemon-access source through explicit daemon policy.
pub fn map_browser_daemon_access(
    source: &BrowserDaemonAccessAdmissionSource,
    policy_role: Option<DaemonAccessPolicyRole>,
) -> MappedDaemonAccessPrincipal {
    MappedDaemonAccessPrincipal::BrowserSession {
        session_id: source.session_id.clone(),
        decision: non_local_decision(source.session_id.as_ref(), policy_role),
    }
}

/// Map a relay-backed daemon-access source through explicit daemon policy.
pub fn map_relay_daemon_access(
    source: &RelayDaemonAccessAdmissionSource,
    policy_role: Option<DaemonAccessPolicyRole>,
) -> MappedDaemonAccessPrincipal {
    MappedDaemonAccessPrincipal::RelayPrincipal {
        principal_id: source.principal_id.clone(),
        decision: non_local_decision(source.principal_id.as_ref(), policy_role),
    }
}

/// Build a complete audit/admission record from a source and policy outcome.
pub fn map_daemon_access_admission(
    source: DaemonAccessAdmissionSource,
    local_role: Option<LocalUnixAllowlistRole>,
    policy_role: Option<DaemonAccessPolicyRole>,
) -> DaemonAccessAdmissionRecord {
    let principal = match &source {
        DaemonAccessAdmissionSource::LocalUnix(local) => map_local_unix_daemon_access(
            local,
            local_role.unwrap_or(LocalUnixAllowlistRole::Denied),
        ),
        DaemonAccessAdmissionSource::Remote(remote) => {
            map_remote_daemon_access(remote, policy_role)
        }
        DaemonAccessAdmissionSource::Relay(relay) => map_relay_daemon_access(relay, policy_role),
        DaemonAccessAdmissionSource::Browser(browser) => {
            map_browser_daemon_access(browser, policy_role)
        }
    };
    DaemonAccessAdmissionRecord { source, principal }
}

fn non_local_decision(
    principal_id: Option<&PrincipalId>,
    policy_role: Option<DaemonAccessPolicyRole>,
) -> DaemonAccessDecision {
    match (principal_id, policy_role) {
        (None, _) => DaemonAccessDecision::denied(DaemonAccessDenyReason::Unauthenticated),
        (Some(_), None) => DaemonAccessDecision::denied(DaemonAccessDenyReason::Unmapped),
        (Some(_), Some(role)) => DaemonAccessDecision::authorized(role),
    }
}
/// Lossless daemon list response for the current public socket contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaemonVmList {
    pub vms: Vec<DaemonVmListEntry>,
}

impl DaemonVmList {
    /// Borrow the daemon-reported VM entries.
    pub fn entries(&self) -> &[DaemonVmListEntry] {
        &self.vms
    }

    /// Consume the list into its daemon-reported VM entries.
    pub fn into_entries(self) -> Vec<DaemonVmListEntry> {
        self.vms
    }

    /// Project the lossless daemon list into v2 workload summaries.
    ///
    /// This projection is intentionally lossy: the v2 summary type has no
    /// fields for daemon runtime detail or pending-restart and cannot
    /// distinguish every daemon lifecycle state.
    pub fn workload_summaries_lossy(
        &self,
        node_id: &NodeId,
    ) -> ProviderResult<Vec<WorkloadSummary>> {
        self.vms
            .iter()
            .map(|entry| workload_summary_lossy(entry, node_id))
            .collect()
    }
}

impl From<ListResponse> for DaemonVmList {
    fn from(response: ListResponse) -> Self {
        Self {
            vms: response.vms.into_iter().map(Into::into).collect(),
        }
    }
}

/// Lossless daemon list entry for one VM.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaemonVmListEntry {
    pub env: Option<String>,
    pub graphics: bool,
    pub is_net_vm: bool,
    pub lifecycle: DaemonVmLifecycle,
    pub name: String,
    pub runtime: DaemonRuntimeSummary,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub runtime_capabilities: Vec<String>,
    pub services: DaemonPublicVmServices,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub service_capabilities: Vec<String>,
    pub ssh_user: Option<String>,
    pub static_ip: Option<String>,
    pub tpm: bool,
    pub usbip: bool,
    pub vm: String,
}

impl From<ListEntry> for DaemonVmListEntry {
    fn from(entry: ListEntry) -> Self {
        Self {
            env: entry.env,
            graphics: entry.graphics,
            is_net_vm: entry.is_net_vm,
            lifecycle: entry.lifecycle.into(),
            name: entry.name,
            runtime: entry.runtime.into(),
            runtime_capabilities: entry.runtime_capabilities,
            services: entry.services.into(),
            service_capabilities: entry.service_capabilities,
            ssh_user: entry.ssh_user,
            static_ip: entry.static_ip,
            tpm: entry.tpm,
            usbip: entry.usbip,
            vm: entry.vm,
        }
    }
}

/// Lossless daemon lifecycle state and pending-restart flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaemonVmLifecycle {
    pub pending_restart: bool,
    pub state: DaemonVmLifecycleState,
}

impl From<VmLifecycle> for DaemonVmLifecycle {
    fn from(lifecycle: VmLifecycle) -> Self {
        Self {
            pending_restart: lifecycle.pending_restart,
            state: lifecycle.state.into(),
        }
    }
}

/// Lossless daemon lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DaemonVmLifecycleState {
    Stopped,
    Starting,
    Booted,
    Running,
    Stopping,
    Restarting,
    Failed,
    Unknown,
}

impl From<VmLifecycleState> for DaemonVmLifecycleState {
    fn from(state: VmLifecycleState) -> Self {
        match state {
            VmLifecycleState::Stopped => Self::Stopped,
            VmLifecycleState::Starting => Self::Starting,
            VmLifecycleState::Booted => Self::Booted,
            VmLifecycleState::Running => Self::Running,
            VmLifecycleState::Stopping => Self::Stopping,
            VmLifecycleState::Restarting => Self::Restarting,
            VmLifecycleState::Failed => Self::Failed,
            VmLifecycleState::Unknown => Self::Unknown,
        }
    }
}

/// Lossless daemon runtime detail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaemonRuntimeSummary {
    pub detail: String,
}

impl From<RuntimeSummary> for DaemonRuntimeSummary {
    fn from(runtime: RuntimeSummary) -> Self {
        Self {
            detail: runtime.detail,
        }
    }
}

/// Lossless daemon service-state summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DaemonPublicVmServices {
    pub gpu: Option<String>,
    pub microvm: String,
    pub nixling: String,
    pub snd: Option<String>,
    pub swtpm: Option<String>,
    pub video: Option<String>,
    pub virtiofsd: String,
}

impl From<PublicVmServices> for DaemonPublicVmServices {
    fn from(services: PublicVmServices) -> Self {
        Self {
            gpu: services.gpu,
            microvm: services.microvm,
            nixling: services.nixling,
            snd: services.snd,
            swtpm: services.swtpm,
            video: services.video,
            virtiofsd: services.virtiofsd,
        }
    }
}

/// Local public-socket daemon access.
#[derive(Clone)]
pub struct LocalUnixDaemonAccess {
    socket_path: PathBuf,
    transport_id: ProviderId,
    node_id: NodeId,
}

impl fmt::Debug for LocalUnixDaemonAccess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalUnixDaemonAccess")
            .finish_non_exhaustive()
    }
}

impl LocalUnixDaemonAccess {
    /// Construct access using the framework default public socket.
    pub fn new() -> Self {
        Self::with_socket_path(DEFAULT_PUBLIC_SOCKET_PATH)
    }

    /// Construct access using an explicit public-socket path.
    pub fn with_socket_path(path: impl Into<PathBuf>) -> Self {
        Self {
            socket_path: path.into(),
            transport_id: ProviderId::parse(LOCAL_UNIX_DAEMON_ACCESS_TRANSPORT_ID)
                .expect("static provider id is valid"),
            node_id: NodeId::parse(LOCAL_NODE_ID).expect("static node id is valid"),
        }
    }

    /// The configured public-socket path.
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    /// List VMs through the local daemon socket without losing public-wire
    /// fields or lifecycle states.
    pub async fn vm_list(&self) -> ProviderResult<DaemonVmList> {
        self.raw_vm_list().await.map(Into::into)
    }

    async fn raw_vm_list(&self) -> ProviderResult<ListResponse> {
        let request = encode_type_tagged_message(
            "list",
            &ListRequest {
                env: None,
                vm: None,
            },
        )?;
        let response = self.request("list", &request).await?;
        parse_list_response(&response)
    }

    async fn request(&self, request_type: &'static str, payload: &[u8]) -> ProviderResult<Vec<u8>> {
        let mut session = self
            .connect(TransportTarget {
                endpoint: LOCAL_NODE_ID.to_owned(),
            })
            .await?;
        let stream = session.stream_mut();

        let hello = encode_type_tagged_message(
            "hello",
            &Hello {
                client_version: SemverRange::new(DEFAULT_CLIENT_VERSION_RANGE).map_err(|err| {
                    ProviderError::new(
                        ErrorKind::VersionSkew,
                        format!("invalid daemon client version range: {err}"),
                    )
                })?,
                supported_features: daemon_supported_features(),
            },
        )?;
        send_frame(stream, &hello).await?;
        let hello_response = recv_frame(stream).await?;
        parse_hello_reply(&hello_response)?;

        send_frame(stream, payload).await?;
        let response = recv_frame(stream).await?;
        reject_error_frame(request_type, &response)?;
        Ok(response)
    }
}

impl Default for LocalUnixDaemonAccess {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DaemonAccessTransport for LocalUnixDaemonAccess {
    fn transport_id(&self) -> ProviderId {
        self.transport_id.clone()
    }

    fn mode(&self) -> DaemonAccessMode {
        DaemonAccessMode::LocalUnix
    }

    async fn connect(&self, _endpoint: TransportTarget) -> ProviderResult<TransportSession> {
        let path = self.socket_path.clone();
        let stream = tokio::task::spawn_blocking(move || connect_seqpacket(&path))
            .await
            .map_err(|err| {
                ProviderError::new(
                    ErrorKind::GatewayUnavailable,
                    format!("local daemon socket connect task failed: {err}"),
                )
            })?
            .map_err(|err| {
                ProviderError::new(
                    ErrorKind::GatewayUnavailable,
                    format!("local daemon public socket unavailable: {}", err.kind()),
                )
            })?;
        let stream = tokio::net::UnixStream::from_std(stream).map_err(|err| {
            ProviderError::new(
                ErrorKind::GatewayUnavailable,
                format!("local daemon socket registration failed: {}", err.kind()),
            )
        })?;
        Ok(TransportSession::new(
            SafeLabel::new("local-unix-public-sock"),
            Box::new(stream),
        ))
    }
}

#[async_trait]
impl DaemonAccessApi for LocalUnixDaemonAccess {
    async fn vm_list(&self) -> ProviderResult<Vec<WorkloadSummary>> {
        LocalUnixDaemonAccess::vm_list(self)
            .await?
            .workload_summaries_lossy(&self.node_id)
    }
}

/// Lossily project the current daemon list response into v2 workload summaries.
pub fn workload_summaries_lossy_from_list_response(
    response: ListResponse,
    node_id: &NodeId,
) -> ProviderResult<Vec<WorkloadSummary>> {
    DaemonVmList::from(response).workload_summaries_lossy(node_id)
}

/// Lossily project one daemon list entry into a v2 workload summary.
pub fn workload_summary_lossy(
    entry: &DaemonVmListEntry,
    node_id: &NodeId,
) -> ProviderResult<WorkloadSummary> {
    let id = WorkloadId::parse(entry.vm.clone()).map_err(|err| {
        ProviderError::new(
            ErrorKind::InvalidTarget,
            format!("daemon list entry carried invalid VM id: {err}"),
        )
    })?;
    Ok(WorkloadSummary {
        id,
        realm: RealmPath::local(),
        node: node_id.clone(),
        state: workload_state_from_lifecycle(entry.lifecycle.state),
        capabilities: capabilities_from_list_entry(entry),
    })
}

fn capabilities_from_list_entry(entry: &DaemonVmListEntry) -> CapabilitySet {
    let mut capabilities = CapabilitySet::empty()
        .with(Capability::Lifecycle)
        .with(Capability::Virtiofs)
        .with(Capability::Vsock);
    if entry.graphics {
        capabilities = capabilities
            .with(Capability::WindowForwarding)
            .with(Capability::GpuAccel);
    }
    if entry.usbip {
        capabilities = capabilities.with(Capability::Usb);
    }
    capabilities
}

fn workload_state_from_lifecycle(state: DaemonVmLifecycleState) -> WorkloadState {
    match state {
        DaemonVmLifecycleState::Stopped => WorkloadState::Stopped,
        DaemonVmLifecycleState::Starting => WorkloadState::Starting,
        DaemonVmLifecycleState::Booted | DaemonVmLifecycleState::Running => WorkloadState::Running,
        DaemonVmLifecycleState::Stopping => WorkloadState::Stopping,
        DaemonVmLifecycleState::Restarting => WorkloadState::Starting,
        DaemonVmLifecycleState::Failed | DaemonVmLifecycleState::Unknown => WorkloadState::Failed,
    }
}

fn connect_seqpacket(path: &Path) -> io::Result<StdUnixStream> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(nix_err_to_io)?;
    let addr = UnixAddr::new(path).map_err(nix_err_to_io)?;
    connect(fd.as_raw_fd(), &addr).map_err(nix_err_to_io)?;
    let stream = StdUnixStream::from(fd);
    stream.set_nonblocking(true)?;
    Ok(stream)
}

async fn send_frame(
    stream: &mut dyn nixling_constellation_provider::types::ByteStream,
    payload: &[u8],
) -> ProviderResult<()> {
    if payload.len() > MAX_FRAME_SIZE {
        return Err(ProviderError::new(
            ErrorKind::FrameTooLarge,
            "daemon request frame exceeds public socket limit",
        ));
    }
    let mut frame = Vec::with_capacity(payload.len() + 4);
    frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    frame.extend_from_slice(payload);
    let written = stream.write(&frame).await.map_err(|err| {
        ProviderError::new(
            ErrorKind::GatewayUnavailable,
            format!("daemon socket write failed: {}", err.kind()),
        )
    })?;
    if written != frame.len() {
        return Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon socket accepted a short seqpacket write",
        ));
    }
    Ok(())
}

async fn recv_frame(
    stream: &mut dyn nixling_constellation_provider::types::ByteStream,
) -> ProviderResult<Vec<u8>> {
    let mut buffer = vec![0_u8; MAX_FRAME_SIZE + 5];
    let received = stream.read(&mut buffer).await.map_err(|err| {
        ProviderError::new(
            ErrorKind::GatewayUnavailable,
            format!("daemon socket read failed: {}", err.kind()),
        )
    })?;
    if received == 0 {
        return Err(ProviderError::new(
            ErrorKind::GatewayUnavailable,
            "daemon closed the public socket before returning a frame",
        ));
    }
    if received < 4 {
        return Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon returned a short public socket frame",
        ));
    }
    let expected = u32::from_le_bytes(buffer[..4].try_into().expect("frame prefix")) as usize;
    if expected > MAX_FRAME_SIZE {
        return Err(ProviderError::new(
            ErrorKind::FrameTooLarge,
            "daemon declared a public socket frame above the allowed limit",
        ));
    }
    if received != expected + 4 {
        return Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon returned a malformed public socket frame",
        ));
    }
    Ok(buffer[4..4 + expected].to_vec())
}

fn encode_type_tagged_message<T>(type_name: &str, message: &T) -> ProviderResult<Vec<u8>>
where
    T: Serialize,
{
    let mut value = serde_json::to_value(message).map_err(|err| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            format!("failed to encode daemon request: {err}"),
        )
    })?;
    value
        .as_object_mut()
        .ok_or_else(|| {
            ProviderError::new(
                ErrorKind::MalformedFrame,
                "daemon request payload must encode as a JSON object",
            )
        })?
        .insert("type".to_owned(), Value::String(type_name.to_owned()));
    serde_json::to_vec(&value).map_err(|err| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            format!("failed to serialize daemon request: {err}"),
        )
    })
}

fn daemon_supported_features() -> Vec<FeatureFlag> {
    vec![
        KnownFeatureFlag::TypedErrors.wire_value(),
        KnownFeatureFlag::ExportBrokerAudit.wire_value(),
    ]
}

fn parse_hello_reply(response: &[u8]) -> ProviderResult<HelloOk> {
    let value = parse_json(response, "hello reply")?;
    match frame_type(&value)? {
        "helloOk" => decode_value::<HelloOkFrame>(value).map(|frame| frame.payload),
        "helloRejected" => decode_value::<HelloRejectedFrame>(value)
            .and_then(|frame| Err(provider_error_from_daemon_error(frame.error))),
        "error" => decode_value::<ErrorFrame>(value)
            .and_then(|frame| Err(provider_error_from_daemon_error(frame.error))),
        _ => Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon returned an unexpected hello reply",
        )),
    }
}

fn parse_list_response(response: &[u8]) -> ProviderResult<ListResponse> {
    let value = parse_json(response, "list response")?;
    if frame_type(&value)? != "listResponse" {
        return Err(ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon returned an unexpected list reply",
        ));
    }
    decode_value::<ListResponseFrame>(value).map(|frame| ListResponse { vms: frame.vms })
}

fn reject_error_frame(request_type: &'static str, response: &[u8]) -> ProviderResult<()> {
    let value = parse_json(response, request_type)?;
    if frame_type(&value)? == "error" {
        let frame = decode_value::<ErrorFrame>(value)?;
        return Err(provider_error_from_daemon_error(frame.error));
    }
    Ok(())
}

fn parse_json(bytes: &[u8], context: &'static str) -> ProviderResult<Value> {
    serde_json::from_slice(bytes).map_err(|err| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            format!("failed to parse daemon {context}: {err}"),
        )
    })
}

fn frame_type(value: &Value) -> ProviderResult<&str> {
    value.get("type").and_then(Value::as_str).ok_or_else(|| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            "daemon frame missing type discriminator",
        )
    })
}

fn decode_value<T>(value: Value) -> ProviderResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(value).map_err(|err| {
        ProviderError::new(
            ErrorKind::MalformedFrame,
            format!("failed to decode daemon frame: {err}"),
        )
    })
}

fn provider_error_from_daemon_error(error: DaemonErrorEnvelope) -> ProviderError {
    let (kind, message) = provider_error_kind_and_message(error.kind.as_str());
    ProviderError::new(kind, message)
}

fn provider_error_kind_and_message(daemon_kind: &str) -> (ErrorKind, &'static str) {
    match daemon_kind {
        "authz-not-a-launcher" | "authz-audit-requires-admin" => (
            ErrorKind::Unauthorized,
            "daemon refused the request because the peer is not authorized",
        ),
        "wire-version-mismatch" => (
            ErrorKind::VersionSkew,
            "daemon wire version is incompatible with this client",
        ),
        "wire-frame-too-large" => (
            ErrorKind::FrameTooLarge,
            "daemon rejected a public socket frame above the allowed limit",
        ),
        "wire-unknown-field" | "wire-ifname-invalid" | "wire-malformed-json" => (
            ErrorKind::MalformedFrame,
            "daemon reported a malformed public socket frame",
        ),
        "broker-unimplemented" => (
            ErrorKind::UnsupportedFeature,
            "daemon reported that the requested broker feature is unavailable",
        ),
        "broker-validation-failed" => (
            ErrorKind::ProviderAllocationFailed,
            "daemon rejected the requested provider operation",
        ),
        "manifest-parse-error" | "manifest-version-mismatch" => (
            ErrorKind::MalformedFrame,
            "daemon reported an incompatible or malformed manifest contract",
        ),
        "internal-io" | "bundle-tampered" => (
            ErrorKind::GatewayUnavailable,
            "daemon reported that required host state is unavailable",
        ),
        _ => (
            ErrorKind::MalformedFrame,
            "daemon returned an unrecognized typed error kind",
        ),
    }
}

fn nix_err_to_io(err: nix::errno::Errno) -> io::Error {
    io::Error::from_raw_os_error(err as i32)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelloOkFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    payload: HelloOk,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HelloRejectedFrame {
    #[serde(rename = "type")]
    _type_name: String,
    #[serde(flatten)]
    _payload: HelloRejected,
    error: DaemonErrorEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ErrorFrame {
    #[serde(rename = "type")]
    _type_name: String,
    error: DaemonErrorEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DaemonErrorEnvelope {
    kind: String,
    #[serde(rename = "message")]
    _message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListResponseFrame {
    #[serde(rename = "type")]
    _type_name: String,
    vms: Vec<ListEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use nix::{
        sys::socket::{Backlog, MsgFlags, accept4, bind, listen, send},
        unistd::close,
    };
    use nixling_ipc::{
        Version,
        public_wire::{PublicVmServices, RuntimeSummary, VmLifecycle},
    };
    use std::{
        fs,
        os::fd::RawFd,
        sync::atomic::{AtomicUsize, Ordering},
        thread,
    };

    static TEST_SOCKET_COUNTER: AtomicUsize = AtomicUsize::new(0);

    #[test]
    fn local_mode_is_implemented() {
        let access = LocalUnixDaemonAccess::new();
        assert_eq!(access.mode(), DaemonAccessMode::LocalUnix);
        assert!(access.mode().is_implemented());
        assert_eq!(access.socket_path(), Path::new(DEFAULT_PUBLIC_SOCKET_PATH));
    }

    #[test]
    fn local_unix_mapping_matches_allowlist_roles() {
        let source = LocalUnixAdmissionSource::new(1000, 100);

        assert_eq!(
            map_local_unix_daemon_access(&source, LocalUnixAllowlistRole::Admin),
            MappedDaemonAccessPrincipal::LocalAdmin { uid: 1000 }
        );
        assert_eq!(
            map_local_unix_daemon_access(&source, LocalUnixAllowlistRole::Launcher),
            MappedDaemonAccessPrincipal::LocalLauncher { uid: 1000 }
        );
        assert_eq!(
            map_local_unix_daemon_access(&source, LocalUnixAllowlistRole::Denied),
            MappedDaemonAccessPrincipal::LocalDenied {
                uid: 1000,
                reason: DaemonAccessDenyReason::LocalNotAllowlisted,
            }
        );
    }

    #[test]
    fn remote_direct_tls_mapping_denies_without_auth_or_policy() {
        let direct_tls = DirectTlsDaemonAccess::new();
        let unauthenticated = remote_source(
            direct_tls.admission_source(RemoteDaemonAccessCredential::unauthenticated(), None),
        );
        assert_eq!(
            unauthenticated.transport_id.as_str(),
            DIRECT_TLS_DAEMON_ACCESS_TRANSPORT_ID
        );
        assert_eq!(unauthenticated.mode, DaemonAccessMode::DirectTls);

        let denied =
            map_remote_daemon_access(&unauthenticated, Some(DaemonAccessPolicyRole::RealmAdmin));
        assert_eq!(
            denied,
            MappedDaemonAccessPrincipal::RemotePrincipal {
                principal_id: None,
                decision: DaemonAccessDecision::denied(DaemonAccessDenyReason::Unauthenticated),
            }
        );
        assert!(denied.is_denied());
        assert!(!denied.is_local_admin_or_launcher());

        let authenticated_unmapped = remote_source(direct_tls.admission_source(
            RemoteDaemonAccessCredential::mutual_tls("cert-material"),
            Some(principal_id("remote-principal")),
        ));
        let denied = map_remote_daemon_access(&authenticated_unmapped, None);
        assert_eq!(
            denied,
            MappedDaemonAccessPrincipal::RemotePrincipal {
                principal_id: Some(principal_id("remote-principal")),
                decision: DaemonAccessDecision::denied(DaemonAccessDenyReason::Unmapped),
            }
        );
        assert!(denied.is_denied());
        assert!(!denied.is_local_admin_or_launcher());

        let explicitly_mapped = map_remote_daemon_access(
            &authenticated_unmapped,
            Some(DaemonAccessPolicyRole::RealmAdmin),
        );
        assert!(matches!(
            explicitly_mapped,
            MappedDaemonAccessPrincipal::RemotePrincipal {
                decision: DaemonAccessDecision::Authorized {
                    role: DaemonAccessPolicyRole::RealmAdmin
                },
                ..
            }
        ));
        assert!(!explicitly_mapped.is_local_admin_or_launcher());
    }

    #[test]
    fn browser_and_relay_mapping_denies_without_auth_or_policy() {
        let browser_unauthenticated = BrowserDaemonAccessAdmissionSource::new(
            BrowserDaemonAccessCredential::unauthenticated(),
            None,
        );
        let denied = map_browser_daemon_access(
            &browser_unauthenticated,
            Some(DaemonAccessPolicyRole::RealmOperator),
        );
        assert_eq!(
            denied,
            MappedDaemonAccessPrincipal::BrowserSession {
                session_id: None,
                decision: DaemonAccessDecision::denied(DaemonAccessDenyReason::Unauthenticated),
            }
        );
        assert!(!denied.is_local_admin_or_launcher());

        let browser_unmapped = BrowserDaemonAccessAdmissionSource::new(
            BrowserDaemonAccessCredential::cookie_session("cookie-material", "csrf-material"),
            Some(principal_id("browser-principal")),
        );
        let denied = map_browser_daemon_access(&browser_unmapped, None);
        assert_eq!(
            denied,
            MappedDaemonAccessPrincipal::BrowserSession {
                session_id: Some(principal_id("browser-principal")),
                decision: DaemonAccessDecision::denied(DaemonAccessDenyReason::Unmapped),
            }
        );
        assert!(!denied.is_local_admin_or_launcher());

        let relay = RelayDaemonAccess::new();
        let relay_unauthenticated = relay_source(
            relay.admission_source(RelayDaemonAccessCredential::unauthenticated(), None),
        );
        assert_eq!(
            relay_unauthenticated.transport_id.as_str(),
            RELAY_DAEMON_ACCESS_TRANSPORT_ID
        );
        assert_eq!(relay_unauthenticated.mode, DaemonAccessMode::Relay);
        let denied = map_relay_daemon_access(
            &relay_unauthenticated,
            Some(DaemonAccessPolicyRole::RealmOperator),
        );
        assert_eq!(
            denied,
            MappedDaemonAccessPrincipal::RelayPrincipal {
                principal_id: None,
                decision: DaemonAccessDecision::denied(DaemonAccessDenyReason::Unauthenticated),
            }
        );
        assert!(!denied.is_local_admin_or_launcher());

        let relay_unmapped = relay_source(relay.admission_source(
            RelayDaemonAccessCredential::from_relay_secret(
                "relay-secret-material",
                RemoteDaemonAccessCredential::bearer_token("daemon-token-material"),
            ),
            Some(principal_id("relay-principal")),
        ));
        let denied = map_relay_daemon_access(&relay_unmapped, None);
        assert_eq!(
            denied,
            MappedDaemonAccessPrincipal::RelayPrincipal {
                principal_id: Some(principal_id("relay-principal")),
                decision: DaemonAccessDecision::denied(DaemonAccessDenyReason::Unmapped),
            }
        );
        assert!(!denied.is_local_admin_or_launcher());
    }

    #[test]
    fn admission_record_serialization_redacts_credential_material() {
        let cert = "-----BEGIN CERTIFICATE-----cert-canary-material-----END CERTIFICATE-----";
        let token = "Bearer token-canary-material";
        let cookie = "session=cookie-canary-material";
        let csrf = "csrf-canary-material";
        let relay_secret = "SharedAccessSignature sig=relay-canary-material";

        let records = [
            map_daemon_access_admission(
                DaemonAccessAdmissionSource::direct_tls(
                    RemoteDaemonAccessCredential::mutual_tls_and_bearer_token(cert, token),
                    Some(principal_id("remote-principal")),
                ),
                None,
                None,
            ),
            map_daemon_access_admission(
                DaemonAccessAdmissionSource::relay(
                    RelayDaemonAccessCredential::from_relay_secret(
                        relay_secret,
                        RemoteDaemonAccessCredential::bearer_token(token),
                    ),
                    Some(principal_id("relay-principal")),
                ),
                None,
                None,
            ),
            map_daemon_access_admission(
                DaemonAccessAdmissionSource::Browser(BrowserDaemonAccessAdmissionSource::new(
                    BrowserDaemonAccessCredential::cookie_session(cookie, csrf),
                    Some(principal_id("browser-principal")),
                )),
                None,
                None,
            ),
        ];

        for record in records {
            let serialized = serde_json::to_string(&record).expect("serialize admission record");
            for secret in [cert, token, cookie, csrf, relay_secret] {
                assert!(
                    !serialized.contains(secret),
                    "serialized admission record leaked {secret:?}: {serialized}"
                );
            }
            for canary in [
                "cert-canary-material",
                "token-canary-material",
                "cookie-canary-material",
                "csrf-canary-material",
                "relay-canary-material",
            ] {
                assert!(
                    !serialized.contains(canary),
                    "serialized admission record leaked {canary:?}: {serialized}"
                );
            }
        }
    }

    #[test]
    fn redacted_credential_length_is_capped_for_oversized_inputs() {
        let oversized = vec![b'x'; usize::from(MAX_REDACTED_CREDENTIAL_BYTES) + 4096];
        let redacted = RedactedDaemonAccessCredential::from_secret(&oversized);
        assert!(redacted.present);
        assert_eq!(redacted.byte_len, Some(MAX_REDACTED_CREDENTIAL_BYTES));
        let serialized = serde_json::to_string(&redacted).expect("serialize redacted credential");
        assert!(!serialized.contains(&"x".repeat(256)));
    }

    #[tokio::test]
    async fn relay_and_direct_tls_fail_closed() {
        let target = TransportTarget {
            endpoint: "example".to_owned(),
        };

        let relay = RelayDaemonAccess::new();
        assert_eq!(relay.mode(), DaemonAccessMode::Relay);
        assert!(!relay.mode().is_implemented());
        let relay_error = relay
            .connect(target.clone())
            .await
            .expect_err("relay is not implemented");
        assert_eq!(relay_error.kind(), ErrorKind::UnsupportedFeature);

        let direct_tls = DirectTlsDaemonAccess::new();
        assert_eq!(direct_tls.mode(), DaemonAccessMode::DirectTls);
        assert!(!direct_tls.mode().is_implemented());
        let direct_tls_error = direct_tls
            .connect(target)
            .await
            .expect_err("direct-tls is not implemented");
        assert_eq!(direct_tls_error.kind(), ErrorKind::UnsupportedFeature);
    }

    #[test]
    fn local_unix_debug_redacts_socket_path_and_node_id() {
        let socket_path = test_socket_path("debug-redaction");
        let access = LocalUnixDaemonAccess::with_socket_path(&socket_path);

        let rendered = format!("{access:?}");

        assert_eq!(rendered, "LocalUnixDaemonAccess { .. }");
        assert!(!rendered.contains(&socket_path.display().to_string()));
        assert!(
            !rendered.contains(
                socket_path
                    .file_name()
                    .expect("socket file name")
                    .to_string_lossy()
                    .as_ref()
            )
        );
        assert!(!rendered.contains(LOCAL_NODE_ID));
    }

    #[test]
    fn daemon_error_mapping_redacts_dynamic_message() {
        let error = provider_error_from_daemon_error(DaemonErrorEnvelope {
            kind: "internal-io".to_owned(),
            _message: "open /home/alice/private-vm/secret.sock failed".to_owned(),
        });

        assert_eq!(error.kind(), ErrorKind::GatewayUnavailable);
        assert_eq!(
            error.message(),
            "daemon reported that required host state is unavailable"
        );
        assert!(!error.message().contains("/home/alice"));
        assert!(!format!("{error:?}").contains("private-vm"));
    }

    #[tokio::test]
    async fn recv_frame_rejects_trailing_bytes() {
        let mut frame = prefixed_frame(2, b"{}");
        frame.push(b'!');

        let error = recv_frame_from_bytes(frame)
            .await
            .expect_err("trailing bytes are rejected");

        assert_eq!(error.kind(), ErrorKind::MalformedFrame);
    }

    #[tokio::test]
    async fn recv_frame_rejects_truncated_body() {
        let frame = prefixed_frame(4, b"{}");

        let error = recv_frame_from_bytes(frame)
            .await
            .expect_err("truncated body is rejected");

        assert_eq!(error.kind(), ErrorKind::MalformedFrame);
    }

    #[tokio::test]
    async fn recv_frame_rejects_declared_oversize() {
        let frame = prefixed_frame(MAX_FRAME_SIZE + 1, b"");

        let error = recv_frame_from_bytes(frame)
            .await
            .expect_err("oversize declaration is rejected");

        assert_eq!(error.kind(), ErrorKind::FrameTooLarge);
    }

    #[tokio::test]
    async fn recv_frame_rejects_max_sized_packet_with_extra_byte() {
        let mut frame = prefixed_frame(MAX_FRAME_SIZE, &vec![0_u8; MAX_FRAME_SIZE]);
        frame.push(0);

        let error = recv_frame_from_bytes(frame)
            .await
            .expect_err("max frame with trailing byte is rejected");

        assert_eq!(error.kind(), ErrorKind::MalformedFrame);
    }

    #[test]
    fn workload_summary_lossy_projection_is_explicitly_separate() {
        let entry =
            DaemonVmListEntry::from(list_entry("work", VmLifecycleState::Running, true, true));
        let node = NodeId::parse("local").expect("node id");
        let summary = workload_summary_lossy(&entry, &node).expect("summary");

        assert_eq!(summary.id.as_str(), "work");
        assert_eq!(summary.node.as_str(), "local");
        assert_eq!(summary.state, WorkloadState::Running);
        assert!(summary.capabilities.has(Capability::Lifecycle));
        assert!(summary.capabilities.has(Capability::Virtiofs));
        assert!(summary.capabilities.has(Capability::Vsock));
        assert!(summary.capabilities.has(Capability::WindowForwarding));
        assert!(summary.capabilities.has(Capability::GpuAccel));
        assert!(summary.capabilities.has(Capability::Usb));
    }

    #[tokio::test]
    async fn local_unix_vm_list_preserves_all_lifecycle_states_and_runtime_details() {
        let socket_path = test_socket_path("vmlist");
        let listener = bind_seqpacket_listener(&socket_path);
        let cases = [
            (
                "vm-stopped",
                VmLifecycleState::Stopped,
                false,
                "stopped detail",
            ),
            (
                "vm-starting",
                VmLifecycleState::Starting,
                false,
                "starting detail",
            ),
            (
                "vm-booted",
                VmLifecycleState::Booted,
                false,
                "booted detail",
            ),
            (
                "vm-running",
                VmLifecycleState::Running,
                false,
                "running detail",
            ),
            (
                "vm-stopping",
                VmLifecycleState::Stopping,
                false,
                "stopping detail",
            ),
            (
                "vm-restarting",
                VmLifecycleState::Restarting,
                false,
                "restarting detail",
            ),
            (
                "vm-failed",
                VmLifecycleState::Failed,
                false,
                "failed detail",
            ),
            (
                "vm-unknown",
                VmLifecycleState::Unknown,
                false,
                "unknown detail",
            ),
            (
                "vm-pending",
                VmLifecycleState::Running,
                true,
                "pending restart detail",
            ),
        ];
        let entries: Vec<_> = cases
            .iter()
            .enumerate()
            .map(|(index, (vm, state, pending_restart, runtime_detail))| {
                let mut entry = list_entry_with(
                    vm,
                    *state,
                    *pending_restart,
                    runtime_detail,
                    index % 2 == 0,
                    index % 3 == 0,
                );
                entry.is_net_vm = *vm == "vm-booted";
                entry.tpm = index % 2 == 1;
                entry.services.snd = Some(format!("nixling-{vm}-snd.service"));
                entry.services.swtpm = entry.tpm.then(|| format!("nixling-{vm}-swtpm.service"));
                entry.services.video = entry
                    .graphics
                    .then(|| format!("nixling-{vm}-video.service"));
                entry.ssh_user = (index % 2 == 0).then(|| "alice".to_owned());
                entry.static_ip = Some(format!("10.20.0.{}", index + 10));
                entry
            })
            .collect();
        let expected = DaemonVmList::from(ListResponse {
            vms: entries.clone(),
        });
        let server = thread::spawn({
            let response_entries = entries.clone();
            move || serve_one_list_round_trip(listener, response_entries)
        });

        let access = LocalUnixDaemonAccess::with_socket_path(&socket_path);
        let list = access.vm_list().await.expect("vm_list response");

        server
            .join()
            .expect("server thread")
            .expect("server exchange");
        let _ = fs::remove_file(&socket_path);

        assert_eq!(list, expected);
        let states: Vec<_> = list.vms.iter().map(|entry| entry.lifecycle.state).collect();
        assert_eq!(
            states,
            cases
                .iter()
                .map(|(_, state, _, _)| DaemonVmLifecycleState::from(*state))
                .collect::<Vec<_>>()
        );
        for (entry, (_, _, pending_restart, runtime_detail)) in list.vms.iter().zip(cases) {
            assert_eq!(entry.lifecycle.pending_restart, pending_restart);
            assert_eq!(entry.runtime.detail, runtime_detail);
        }
        assert!(
            list.vms
                .iter()
                .any(|entry| entry.lifecycle.state == DaemonVmLifecycleState::Restarting)
        );
        assert!(
            list.vms
                .iter()
                .any(|entry| entry.lifecycle.state == DaemonVmLifecycleState::Unknown)
        );
        assert!(list.vms.iter().any(|entry| entry.lifecycle.pending_restart));
    }

    #[tokio::test]
    async fn local_unix_vm_list_unavailable_socket_returns_typed_error() {
        let socket_path = test_socket_path("missing-vmlist");
        let _ = fs::remove_file(&socket_path);
        let access = LocalUnixDaemonAccess::with_socket_path(&socket_path);

        let error = access
            .vm_list()
            .await
            .expect_err("unavailable socket returns a typed error");

        assert_eq!(error.kind(), ErrorKind::GatewayUnavailable);
    }

    fn principal_id(raw: &str) -> PrincipalId {
        PrincipalId::parse(raw).expect("principal id")
    }

    fn remote_source(source: DaemonAccessAdmissionSource) -> RemoteDaemonAccessAdmissionSource {
        match source {
            DaemonAccessAdmissionSource::Remote(source) => source,
            other => panic!("expected remote source, got {other:?}"),
        }
    }

    fn relay_source(source: DaemonAccessAdmissionSource) -> RelayDaemonAccessAdmissionSource {
        match source {
            DaemonAccessAdmissionSource::Relay(source) => source,
            other => panic!("expected relay source, got {other:?}"),
        }
    }
    fn list_entry(vm: &str, state: VmLifecycleState, graphics: bool, usbip: bool) -> ListEntry {
        list_entry_with(vm, state, false, "running", graphics, usbip)
    }

    fn list_entry_with(
        vm: &str,
        state: VmLifecycleState,
        pending_restart: bool,
        runtime_detail: &str,
        graphics: bool,
        usbip: bool,
    ) -> ListEntry {
        ListEntry {
            env: Some("dev".to_owned()),
            graphics,
            is_net_vm: false,
            lifecycle: VmLifecycle {
                pending_restart,
                state,
            },
            name: vm.to_owned(),
            autostart: None,
            qemu_media: None,
            runtime: RuntimeSummary {
                detail: runtime_detail.to_owned(),
                kind: None,
                operation_capabilities: Default::default(),
                services: Vec::new(),
            },
            services: PublicVmServices {
                gpu: graphics.then(|| format!("nixling-{vm}-gpu.service")),
                microvm: format!("microvm@{vm}.service"),
                nixling: format!("nixling@{vm}.service"),
                qemu_media: None,
                snd: None,
                swtpm: None,
                video: None,
                virtiofsd: format!("virtiofsd-{vm}.service"),
            },
            ssh_user: Some("alice".to_owned()),
            static_ip: Some("10.20.0.10".to_owned()),
            tpm: false,
            runtime_capabilities: Vec::new(),
            service_capabilities: Vec::new(),
            unsupported_capabilities: Vec::new(),
            usbip,
            vm: vm.to_owned(),
        }
    }

    fn prefixed_frame(declared: usize, body: &[u8]) -> Vec<u8> {
        let mut frame = Vec::with_capacity(4 + body.len());
        frame.extend_from_slice(&(declared as u32).to_le_bytes());
        frame.extend_from_slice(body);
        frame
    }

    async fn recv_frame_from_bytes(bytes: Vec<u8>) -> ProviderResult<Vec<u8>> {
        let (mut stream, mut peer) = tokio::io::duplex(bytes.len().max(1));
        peer.write_all(&bytes).await.expect("write test frame");
        drop(peer);
        recv_frame(&mut stream).await
    }

    fn bind_seqpacket_listener(path: &Path) -> std::os::fd::OwnedFd {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create socket parent");
        }
        let _ = fs::remove_file(path);
        let listener = socket(
            AddressFamily::Unix,
            SockType::SeqPacket,
            SockFlag::SOCK_CLOEXEC,
            None,
        )
        .expect("listener socket");
        let addr = UnixAddr::new(path).expect("unix addr");
        bind(listener.as_raw_fd(), &addr).expect("bind listener");
        listen(&listener, Backlog::new(1).expect("backlog")).expect("listen");
        listener
    }

    fn serve_one_list_round_trip(
        listener: std::os::fd::OwnedFd,
        response_entries: Vec<ListEntry>,
    ) -> io::Result<()> {
        let accepted =
            accept4(listener.as_raw_fd(), SockFlag::SOCK_CLOEXEC).map_err(nix_err_to_io)?;
        let result = (|| -> io::Result<()> {
            let hello = recv_test_frame(accepted)?;
            let hello: Value = serde_json::from_slice(&hello)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            assert_eq!(hello.get("type").and_then(Value::as_str), Some("hello"));

            let hello_reply = encode_type_tagged_message(
                "helloOk",
                &HelloOk {
                    server_version: Version::new("0.4.0").expect("server version"),
                    selected_version: Version::new("0.4.0").expect("selected version"),
                    capabilities: daemon_supported_features(),
                },
            )
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            send_test_frame(accepted, &hello_reply)?;

            let request = recv_test_frame(accepted)?;
            let request: Value = serde_json::from_slice(&request)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            assert_eq!(request.get("type").and_then(Value::as_str), Some("list"));
            assert_eq!(request.get("env"), Some(&Value::Null));
            assert_eq!(request.get("vm"), Some(&Value::Null));

            let mut response = serde_json::json!({ "type": "listResponse" });
            response["vms"] = serde_json::to_value(response_entries)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            let response = serde_json::to_vec(&response)
                .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
            send_test_frame(accepted, &response)
        })();
        close(accepted).map_err(nix_err_to_io)?;
        result
    }

    fn test_socket_path(prefix: &str) -> PathBuf {
        let counter = TEST_SOCKET_COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!("{prefix}-{}-{counter}.s", std::process::id()))
    }

    fn recv_test_frame(fd: RawFd) -> io::Result<Vec<u8>> {
        let mut buffer = vec![0_u8; MAX_FRAME_SIZE + 5];
        let received =
            nix::sys::socket::recv(fd, &mut buffer, MsgFlags::empty()).map_err(nix_err_to_io)?;
        if received < 4 {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "short frame from seqpacket socket",
            ));
        }
        let expected = u32::from_le_bytes(buffer[..4].try_into().expect("frame prefix")) as usize;
        if expected > MAX_FRAME_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "oversize seqpacket frame",
            ));
        }
        if expected + 4 != received {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "malformed seqpacket frame",
            ));
        }
        Ok(buffer[4..4 + expected].to_vec())
    }

    fn send_test_frame(fd: RawFd, payload: &[u8]) -> io::Result<()> {
        let mut frame = Vec::with_capacity(payload.len() + 4);
        frame.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        frame.extend_from_slice(payload);
        let sent = send(fd, &frame, MsgFlags::empty()).map_err(nix_err_to_io)?;
        if sent != frame.len() {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short write on seqpacket socket",
            ));
        }
        Ok(())
    }
}
