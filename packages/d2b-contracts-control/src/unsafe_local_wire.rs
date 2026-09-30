//! Private unsafe-local helper protocol.
//!
//! The authenticated Unix peer credential is the transport identity. It is
//! never the requester's authority: a launch frame names the graph admission
//! that decided the launch, and the transport may only *check* that admission,
//! never supply one. No frame carries a uid, environment, cwd, compositor
//! path, or arbitrary public argv.

use d2b_contracts::{
    configured_argv::ConfiguredArgv, ids::OperationId, token::ProtocolToken,
    workload_identity::WorkloadTarget,
};
use d2b_contracts_resource::v3::resource_schema::{canonical_json_bytes, framed_canonical_digest};
pub use d2b_contracts_resource::v3::binding::BindingRealizationFacet;
pub use d2b_contracts_resource::v3::ZoneResourceIdentity;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Protocol version this wire vocabulary speaks; peers must agree on it.
///
/// Version 4 is the first version whose launch frame carries a
/// [`HelperGraphAdmission`]. A version 3 peer is refused at the greeting
/// rather than served, and a version 3 launch frame no longer decodes, so an
/// old helper or workload frame cannot reach a launch at all (R43, R49).
pub const UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION: u32 = 4;
/// Maximum bytes in one control frame on either direction.
pub const MAX_HELPER_FRAME_SIZE: usize = 256 * 1024;
/// Value requested through `SO_SNDBUF` and `SO_RCVBUF` on both control peers.
pub const HELPER_SOCKET_BUFFER_REQUEST_BYTES: usize = MAX_HELPER_FRAME_SIZE;
/// Minimum value that `getsockopt` must report after Linux doubles the request.
pub const MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES: usize = MAX_HELPER_FRAME_SIZE * 2;
/// Maximum operations the helper queues per control peer before refusing.
pub const MAX_HELPER_QUEUE_DEPTH: usize = 128;
/// Maximum scopes one helper snapshot may carry.
pub const MAX_HELPER_SNAPSHOT_SCOPES: usize = 1024;
/// Maximum completed-operation records the daemon retains per uid.
pub const MAX_COMPLETED_OPERATIONS_PER_UID: usize = 1024;
/// How long a completed-operation record may age before it is dropped.
pub const MAX_COMPLETED_OPERATION_AGE_SECS: u64 = 24 * 60 * 60;
/// The domain tag framing one graph-admission digest.
pub const HELPER_GRAPH_ADMISSION_DOMAIN_TAG: &str = "d2b:v3:unsafe-local-admission";
/// Maximum presentation facets one admitted unsafe-local launch may depend on.
pub const MAX_HELPER_ADMISSION_PRESENTATIONS: usize = 8;

/// Whether a peer protocol version is the one this wire speaks.
pub const fn unsafe_local_helper_protocol_supported(version: u32) -> bool {
    version == UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION
}

/// The isolation posture one admitted unsafe-local launch runs under.
///
/// The vocabulary is deliberately closed and has exactly one member. An
/// unsafe-local `Host` is an explicit *no-isolation* target: it runs the
/// workload with the admitted requester's own host identity and no namespace,
/// mount, capability, or syscall confinement. Naming that posture here keeps
/// the no-isolation meaning a declared value rather than an absence, and it
/// leaves exactly one place to add a confinement this family would later
/// enforce - it never leaves room for a launch that carries no posture at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum UnsafeLocalPosture {
    /// No isolation: the requester's own identity, no confinement.
    ExplicitNoIsolation,
}

/// The graph decision that admitted one unsafe-local launch.
///
/// A launch frame is a *request*; this value is the decision that made it
/// admissible, and it is derived from the graph rather than from the transport
/// that carries it. It names the committed consumer row the launch is fenced
/// against (its Zone, store-assigned identity, and generation all move when the
/// row is replaced or revised), the authenticated subject the admission was
/// made for, the one posture this family runs under, and the presentation
/// facets the launch depends on.
///
/// The presentation set is the load-bearing half. This family has no mount
/// namespace, so it realizes no destination and no named view: an admission
/// that depends on any presentation facet cannot be enforced here and is
/// refused before the workload starts rather than launched with the property
/// silently dropped (R20, R27, AE19).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, try_from = "HelperGraphAdmissionWire")]
pub struct HelperGraphAdmission {
    workload: ZoneResourceIdentity,
    requester_uid: u32,
    posture: UnsafeLocalPosture,
    presentation: Vec<BindingRealizationFacet>,
    admission_digest: String,
}

impl HelperGraphAdmission {
    /// Admit one launch for one committed consumer row and one requester.
    ///
    /// The digest is computed here rather than supplied, so an admission
    /// cannot be assembled from parts that do not describe the launch it
    /// claims to admit.
    ///
    /// # Errors
    ///
    /// Returns [`HelperFailureCode::InvalidRequest`] when the requester is
    /// root or unnamed, or when the launch depends on more presentation
    /// facets than the boundary admits.
    pub fn new(
        workload: ZoneResourceIdentity,
        requester_uid: u32,
        posture: UnsafeLocalPosture,
        presentation: Vec<BindingRealizationFacet>,
    ) -> Result<Self, HelperFailureCode> {
        if requester_uid == 0 || presentation.len() > MAX_HELPER_ADMISSION_PRESENTATIONS {
            return Err(HelperFailureCode::InvalidRequest);
        }
        let admission_digest = Self::digest_of(&workload, requester_uid, posture, &presentation);
        Ok(Self {
            workload,
            requester_uid,
            posture,
            presentation,
            admission_digest,
        })
    }

    fn digest_of(
        workload: &ZoneResourceIdentity,
        requester_uid: u32,
        posture: UnsafeLocalPosture,
        presentation: &[BindingRealizationFacet],
    ) -> String {
        framed_canonical_digest(
            HELPER_GRAPH_ADMISSION_DOMAIN_TAG,
            &canonical_admission_payload(workload, requester_uid, posture, presentation),
        )
    }

    /// The committed consumer row this admission is fenced against.
    pub const fn workload(&self) -> &ZoneResourceIdentity {
        &self.workload
    }

    /// The authenticated subject the admission was made for.
    pub const fn requester_uid(&self) -> u32 {
        self.requester_uid
    }

    /// The posture this admission runs under.
    pub const fn posture(&self) -> UnsafeLocalPosture {
        self.posture
    }

    /// The presentation facets the launch depends on.
    pub fn presentation(&self) -> &[BindingRealizationFacet] {
        &self.presentation
    }

    /// The digest framing this admission.
    pub fn admission_digest(&self) -> &str {
        &self.admission_digest
    }

    /// Whether the recorded digest still describes the recorded admission.
    pub fn is_intact(&self) -> bool {
        self.admission_digest
            == Self::digest_of(
                &self.workload,
                self.requester_uid,
                self.posture,
                &self.presentation,
            )
    }

    /// Whether this admission is the one this launch runs under.
    ///
    /// A frame whose admission names a different committed row than the
    /// workload it launches is refused: the admission fences the exact row it
    /// was issued for, and a row replaced under it no longer matches.
    pub fn admits(&self, workload: &ZoneResourceIdentity) -> bool {
        &self.workload == workload && self.is_intact()
    }

    /// Check one launch against this admission at an effect boundary.
    ///
    /// `transport_uid` is the identity the receiving process proved for
    /// itself. It is used only to *check* the admission's requester: the
    /// requester is whatever the graph admitted, and a transport whose own
    /// identity disagrees is refused rather than believed (R37, AE29).
    ///
    /// # Errors
    ///
    /// Returns [`HelperFailureCode::GraphAdmissionRequired`] when the
    /// admission is absent, does not verify, does not name this launch's
    /// committed row, or depends on a presentation facet this family cannot
    /// enforce; and [`HelperFailureCode::RequesterMismatch`] when the
    /// admission was made for a different subject than the one presenting it.
    pub fn admit_launch(
        &self,
        workload: &ZoneResourceIdentity,
        transport_uid: u32,
    ) -> Result<(), HelperFailureCode> {
        if !self.admits(workload) {
            return Err(HelperFailureCode::GraphAdmissionRequired);
        }
        if self.requester_uid != transport_uid {
            return Err(HelperFailureCode::RequesterMismatch);
        }
        if !self.presentation.is_empty() {
            return Err(HelperFailureCode::GraphAdmissionRequired);
        }
        Ok(())
    }
}

impl fmt::Debug for HelperGraphAdmission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperGraphAdmission")
            .field("workload", &self.workload)
            .field("requester_uid", &"<redacted>")
            .field("posture", &self.posture)
            .field("presentation", &self.presentation)
            .field("admission_digest", &"<redacted>")
            .finish()
    }
}


#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelperGraphAdmissionWire {
    workload: ZoneResourceIdentity,
    requester_uid: u32,
    posture: UnsafeLocalPosture,
    #[serde(default)]
    presentation: Vec<BindingRealizationFacet>,
    admission_digest: String,
}

impl TryFrom<HelperGraphAdmissionWire> for HelperGraphAdmission {
    type Error = &'static str;

    fn try_from(wire: HelperGraphAdmissionWire) -> Result<Self, Self::Error> {
        let admission = Self {
            workload: wire.workload,
            requester_uid: wire.requester_uid,
            posture: wire.posture,
            presentation: wire.presentation,
            admission_digest: wire.admission_digest,
        };
        if admission.requester_uid == 0
            || admission.presentation.len() > MAX_HELPER_ADMISSION_PRESENTATIONS
            || !admission.is_intact()
        {
            return Err("the launch admission does not verify");
        }
        Ok(admission)
    }
}

/// The canonical payload one admission digest is computed over.
#[derive(Serialize)]
struct AdmissionPayload<'a> {
    workload: &'a ZoneResourceIdentity,
    requester_uid: u32,
    posture: UnsafeLocalPosture,
    presentation: &'a [BindingRealizationFacet],
}

fn canonical_admission_payload(
    workload: &ZoneResourceIdentity,
    requester_uid: u32,
    posture: UnsafeLocalPosture,
    presentation: &[BindingRealizationFacet],
) -> Vec<u8> {
    canonical_json_bytes(&AdmissionPayload {
        workload,
        requester_uid,
        posture,
        presentation,
    })
    .expect("an admission payload always renders as canonical JSON")
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Helper-to-daemon greeting naming the protocol version and generation.
pub struct HelperHello {
    pub protocol_version: u32,
    pub generation: u64,
    #[serde(default)]
    pub features: Vec<ProtocolToken>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Daemon-to-helper acceptance of a greeting, pinning interval bounds.
pub struct HelperHelloAccepted {
    pub protocol_version: u32,
    pub generation: u64,
    pub heartbeat_interval_secs: u32,
    pub operation_timeout_secs: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Liveness frame carrying the generation and a monotonic sequence.
pub struct HelperHeartbeat {
    pub generation: u64,
    pub sequence: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
/// The kind of workload scope the helper runs.
pub enum HelperScopeKind {
    /// A launcher application scope.
    LauncherApp,
    /// A Wayland proxy scope.
    WaylandProxy,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Opaque identity of one helper scope; the invocation id is redacted.
pub struct ScopeIdentity {
    pub invocation_id: String,
    pub kind: HelperScopeKind,
}

impl fmt::Debug for ScopeIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScopeIdentity")
            .field("invocation_id", &"<redacted>")
            .field("kind", &self.kind)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
/// Lifecycle state of one helper scope.
pub enum HelperScopeState {
    /// The scope is being set up.
    Starting,
    /// The scope is serving.
    Active,
    /// The scope is tearing down.
    Stopping,
    /// The scope has exited.
    Exited,
    /// The scope is serving degraded.
    Degraded,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// One scope's observed state in a helper snapshot.
pub struct HelperScopeSnapshot {
    pub operation_id: OperationId,
    pub workload: ZoneResourceIdentity,
    pub scope: ScopeIdentity,
    pub state: HelperScopeState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, try_from = "HelperSnapshotWire")]
/// Bounded snapshot of every scope the helper currently runs.
pub struct HelperSnapshot {
    pub generation: u64,
    pub scopes: Vec<HelperScopeSnapshot>,
}

impl HelperSnapshot {
    /// Validate the snapshot bounds and every workload identity.
    ///
    /// # Errors
    ///
    /// Returns [`HelperFailureCode::InvalidRequest`] when the generation is
    /// zero, the scope count exceeds the bound, or a workload identity is
    /// not a helper-owned resource type.
    pub(crate) fn validate(&self) -> Result<(), HelperFailureCode> {
        if self.generation == 0 {
            return Err(HelperFailureCode::InvalidRequest);
        }
        if self.scopes.len() > MAX_HELPER_SNAPSHOT_SCOPES {
            return Err(HelperFailureCode::InvalidRequest);
        }
        self.scopes.iter().try_for_each(|scope| {
            validate_unsafe_local_resource_identity(&scope.workload)
        })
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelperSnapshotWire {
    generation: u64,
    scopes: Vec<HelperScopeSnapshot>,
}

impl TryFrom<HelperSnapshotWire> for HelperSnapshot {
    type Error = &'static str;

    fn try_from(wire: HelperSnapshotWire) -> Result<Self, Self::Error> {
        let snapshot = Self {
            generation: wire.generation,
            scopes: wire.scopes,
        };
        snapshot
            .validate()
            .map_err(|_| "invalid helper snapshot")?;
        Ok(snapshot)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields, try_from = "HelperLaunchRequestWire")]
/// One launch request the daemon commits to the helper.
///
/// The launch is admissible only under the [`HelperGraphAdmission`] it
/// carries. The field is required, so a frame written before the admission
/// existed does not decode into a launch at all, and the requester's identity
/// is the one the graph admitted rather than one a transport can assert.
///
/// The workload target, item id, argv, and requester are redacted in
/// `Debug`.
pub struct HelperLaunchRequest {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub workload: ZoneResourceIdentity,
    pub admission: HelperGraphAdmission,
    pub target: WorkloadTarget,
    pub item_id: ProtocolToken,
    pub argv: ConfiguredArgv,
    pub graphical: bool,
    pub realm_accent_color: RealmAccentColor,
}

impl fmt::Debug for HelperLaunchRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HelperLaunchRequest")
            .field("request_id", &self.request_id)
            .field("operation_id", &self.operation_id)
            .field("workload", &self.workload)
            .field("admission", &self.admission)
            .field("target", &"<redacted>")
            .field("item_id", &self.item_id)
            .field("argv_count", &self.argv.as_slice().len())
            .field("graphical", &self.graphical)
            .field("realm_accent_color", &self.realm_accent_color)
            .finish()
    }
}

impl HelperLaunchRequest {
    /// Validate the workload identity and its admission bounds.
    ///
    /// The admission must be the one issued for this launch's exact committed
    /// row. Whether the admitting graph decision still holds, and whether the
    /// subject presenting it is the admitted requester, is decided at the
    /// effect boundary by [`HelperGraphAdmission::admit_launch`].
    ///
    /// # Errors
    ///
    /// Returns [`HelperFailureCode::InvalidRequest`] when the workload is
    /// not a helper-owned resource type or the admission names a different
    /// row.
    pub(crate) fn validate_bounds(&self) -> Result<(), HelperFailureCode> {
        validate_unsafe_local_resource_identity(&self.workload)?;
        if self.admission.admits(&self.workload) {
            Ok(())
        } else {
            Err(HelperFailureCode::InvalidRequest)
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct HelperLaunchRequestWire {
    request_id: u64,
    operation_id: OperationId,
    workload: ZoneResourceIdentity,
    admission: HelperGraphAdmission,
    target: WorkloadTarget,
    item_id: ProtocolToken,
    argv: ConfiguredArgv,
    graphical: bool,
    realm_accent_color: RealmAccentColor,
}

impl TryFrom<HelperLaunchRequestWire> for HelperLaunchRequest {
    type Error = &'static str;

    fn try_from(wire: HelperLaunchRequestWire) -> Result<Self, Self::Error> {
        let request = Self {
            request_id: wire.request_id,
            operation_id: wire.operation_id,
            workload: wire.workload,
            admission: wire.admission,
            target: wire.target,
            item_id: wire.item_id,
            argv: wire.argv,
            graphical: wire.graphical,
            realm_accent_color: wire.realm_accent_color,
        };
        request
            .validate_bounds()
            .map_err(|_| "invalid helper launch request")?;
        Ok(request)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(transparent)]
/// A validated `#rrggbb` accent color.
pub struct RealmAccentColor(#[schemars(regex(pattern = "^#[0-9a-f]{6}$"))] String);

impl RealmAccentColor {
    /// Validate and construct a `#rrggbb` accent color.
    ///
    /// # Errors
    ///
    /// Returns [`HelperFailureCode::InvalidRequest`] when the value is not
    /// exactly `#` plus six lowercase hex digits.
    pub fn new(value: impl Into<String>) -> Result<Self, HelperFailureCode> {
        let value = value.into();
        let valid = value.len() == 7
            && value.starts_with('#')
            && value[1..]
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
        valid
            .then_some(Self(value))
            .ok_or(HelperFailureCode::InvalidRequest)
    }

    /// Borrow the validated color text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for RealmAccentColor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RealmAccentColor(<validated>)")
    }
}

impl<'de> Deserialize<'de> for RealmAccentColor {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?)
            .map_err(|_| serde::de::Error::custom("realm accent color must match ^#[0-9a-f]{6}$"))
    }
}

/// Refuse identities whose resource type the helper cannot own.
///
/// # Errors
///
/// Returns [`HelperFailureCode::InvalidRequest`] when the resource type is
/// not Host, Guest, Process, or EphemeralProcess.
pub fn validate_unsafe_local_resource_identity(
    identity: &ZoneResourceIdentity,
) -> Result<(), HelperFailureCode> {
    matches!(
        identity.resource_ref().resource_type().as_str(),
        "Host" | "Guest" | "Process" | "EphemeralProcess"
    )
    .then_some(())
    .ok_or(HelperFailureCode::InvalidRequest)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
/// Closed failure code one helper operation can report.
pub enum HelperFailureCode {
    /// The request was malformed or out of bounds.
    InvalidRequest,
    /// The launch carried no admission the graph made, the admission did not
    /// verify, or it depends on a presentation this family cannot enforce.
    ///
    /// The family is default-denied: an unsafe-local workload never starts
    /// because a transport asked it to, only because the graph admitted it.
    GraphAdmissionRequired,
    /// The admission was made for a different subject than the one
    /// presenting it. The transport's own identity is evidence about the
    /// transport, never a stand-in for the requester.
    RequesterMismatch,
    OperationIdConflict,
    QueueFull,
    Timeout,
    UserManagerUnavailable,
    EnvironmentInvalid,
    ExecutableUnavailable,
    ScopeCreateFailed,
    ScopeIdentityMismatch,
    GraphicalSessionInactive,
    WaylandUnavailable,
    ProxyUnavailable,
    FirstClientTimeout,
    Internal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
/// How a helper operation settled.
pub enum HelperOperationDisposition {
    /// The operation was newly committed.
    Committed,
    /// The operation was already committed before.
    AlreadyCommitted,
    /// The operation finished.
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Outcome of one committed helper operation.
pub struct HelperOperationResult {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub disposition: HelperOperationDisposition,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<ScopeIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// Refusal of one helper operation with its closed failure code.
pub struct HelperOperationRejected {
    pub request_id: u64,
    pub operation_id: OperationId,
    pub code: HelperFailureCode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
/// One frame the daemon may send to the helper.
pub enum DaemonToUnsafeLocalHelper {
    /// The greeting was accepted.
    HelloAccepted(HelperHelloAccepted),
    /// A liveness frame.
    Heartbeat(HelperHeartbeat),
    /// A launch request.
    Launch(Box<HelperLaunchRequest>),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", content = "payload", rename_all = "camelCase")]
/// One frame the helper may send to the daemon.
pub enum UnsafeLocalHelperToDaemon {
    /// The initial greeting.
    Hello(HelperHello),
    /// A scope snapshot.
    Snapshot(HelperSnapshot),
    /// A liveness frame.
    Heartbeat(HelperHeartbeat),
    /// A committed operation outcome.
    Operation(HelperOperationResult),
    /// A refused operation.
    Rejected(HelperOperationRejected),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
/// The complete helper wire schema: version plus both frame directions.
pub struct UnsafeLocalHelperWireSchema {
    pub protocol_version: u32,
    pub daemon_to_helper: DaemonToUnsafeLocalHelper,
    pub helper_to_daemon: UnsafeLocalHelperToDaemon,
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::{configured_argv::ConfiguredArgv, workload_identity::WorkloadTarget};
    use d2b_contracts_resource::v3::{
        ResourceGeneration, ResourceRef, ResourceUid, ZoneId, ZoneResourceIdentity, ZoneRevision,
    };
    use serde::de::DeserializeOwned;

    /// The requester every fixture's admission is issued for.
    const REQUESTER_UID: u32 = 1000;

    fn admission_for(identity: &ZoneResourceIdentity) -> HelperGraphAdmission {
        HelperGraphAdmission::new(
            identity.clone(),
            REQUESTER_UID,
            UnsafeLocalPosture::ExplicitNoIsolation,
            Vec::new(),
        )
        .expect("the fixture admission is constructible")
    }

    fn launch(workload: &ZoneResourceIdentity) -> HelperLaunchRequest {
        HelperLaunchRequest {
            request_id: 2,
            operation_id: operation("op-launch"),
            workload: workload.clone(),
            admission: admission_for(workload),
            target: WorkloadTarget::parse("tools.work.d2b").unwrap(),
            item_id: ProtocolToken::parse("browser").unwrap(),
            argv: ConfiguredArgv::new(vec!["browser".to_owned()]).unwrap(),
            graphical: false,
            realm_accent_color: RealmAccentColor::new("#cc3344").unwrap(),
        }
    }

    fn workload() -> ZoneResourceIdentity {
        zone_identity(
            "work",
            "123e4567-e89b-42d3-a456-426614174000",
            "323e4567-e89b-42d3-a456-426614174002",
            1,
        )
    }

    fn zone_identity(
        zone: &str,
        zone_uid: &str,
        resource_uid: &str,
        generation: u64,
    ) -> ZoneResourceIdentity {
        ZoneResourceIdentity::new(
            ZoneId::parse(zone).unwrap(),
            ResourceUid::parse(zone_uid).unwrap(),
            ResourceRef::parse("Process/tools").unwrap(),
            ResourceUid::parse(resource_uid).unwrap(),
            ResourceGeneration::new(generation).unwrap(),
            ZoneRevision::new(1),
        )
    }

    fn operation(value: &str) -> OperationId {
        OperationId::parse(value).unwrap()
    }

    #[test]
    fn zone_identity_fences_same_name_requests_and_excludes_realm_fields() {
        let work = zone_identity(
            "work",
            "123e4567-e89b-42d3-a456-426614174000",
            "323e4567-e89b-42d3-a456-426614174002",
            3,
        );
        let personal = zone_identity(
            "personal",
            "223e4567-e89b-42d3-a456-426614174001",
            "423e4567-e89b-42d3-a456-426614174003",
            3,
        );
        assert_ne!(work, personal);
        let encoded = serde_json::to_value(&work).unwrap();
        assert_eq!(encoded["zone"], "work");
        assert_eq!(encoded["resourceRef"], "Process/tools");
        assert_eq!(encoded["generation"], 3);
        assert!(encoded.get("realmId").is_none());
        assert!(encoded.get("realmPath").is_none());
        assert!(encoded.get("canonicalTarget").is_none());
        assert_eq!(format!("{work:?}"), "ZoneResourceIdentity(<redacted>)");
        let mut legacy = encoded.clone();
        legacy["realmId"] = serde_json::json!("work");
        assert!(serde_json::from_value::<ZoneResourceIdentity>(legacy).is_err());

        let mut launch = launch(&work);
        launch.operation_id = operation("op-zone-launch");
        launch.argv = ConfiguredArgv::new(vec!["private-argv-canary".to_owned()]).unwrap();
        round_trip(&launch);
        assert!(!format!("{launch:?}").contains("tools.work.d2b"));
        assert!(!format!("{launch:?}").contains("private-argv-canary"));
    }

    #[test]
    fn zone_identity_changes_are_not_accepted_as_the_same_resource() {
        let current = zone_identity(
            "work",
            "123e4567-e89b-42d3-a456-426614174000",
            "323e4567-e89b-42d3-a456-426614174002",
            3,
        );
        let stale_uid = zone_identity(
            "work",
            "123e4567-e89b-42d3-a456-426614174000",
            "423e4567-e89b-42d3-a456-426614174003",
            3,
        );
        let stale_generation = zone_identity(
            "work",
            "123e4567-e89b-42d3-a456-426614174000",
            "323e4567-e89b-42d3-a456-426614174002",
            4,
        );
        assert_ne!(current, stale_uid);
        assert_ne!(current, stale_generation);
        assert_eq!(current.resource_ref(), stale_uid.resource_ref());
        assert_eq!(current.resource_ref(), stale_generation.resource_ref());
    }

    #[test]
    fn helper_requests_reject_non_execution_resource_identities() {
        let invalid = ZoneResourceIdentity::new(
            ZoneId::parse("work").unwrap(),
            ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ResourceRef::parse("Volume/secret").unwrap(),
            ResourceUid::parse("323e4567-e89b-42d3-a456-426614174002").unwrap(),
            ResourceGeneration::new(1).unwrap(),
            ZoneRevision::new(1),
        );
        let mut frame = serde_json::to_value(launch(&workload())).unwrap();
        frame["workload"] = serde_json::to_value(&invalid).unwrap();
        assert!(serde_json::from_value::<HelperLaunchRequest>(frame).is_err());

        // An admission issued for one committed row does not admit another:
        // the launch's own workload is replaced under a valid admission.
        let mut rebound = serde_json::to_value(launch(&workload())).unwrap();
        rebound["workload"] = serde_json::to_value(zone_identity(
            "work",
            "123e4567-e89b-42d3-a456-426614174000",
            "523e4567-e89b-42d3-a456-426614174002",
            1,
        ))
        .unwrap();
        assert!(serde_json::from_value::<HelperLaunchRequest>(rebound).is_err());

        // A frame written before the admission existed does not decode into a
        // launch at all: an old workload frame cannot reach a launch.
        let mut old_frame = serde_json::to_value(launch(&workload())).unwrap();
        old_frame
            .as_object_mut()
            .expect("the launch frame is an object")
            .remove("admission");
        assert!(serde_json::from_value::<HelperLaunchRequest>(old_frame).is_err());

        let snapshot = serde_json::json!({
            "generation": 1,
            "scopes": [{
                "operationId": "op-invalid-snapshot",
                "workload": serde_json::to_value(invalid).unwrap(),
                "scope": {
                    "invocationId": "00112233445566778899aabbccddeeff",
                    "kind": "launcher-app"
                },
                "state": "active"
            }]
        });
        assert!(serde_json::from_value::<HelperSnapshot>(snapshot).is_err());
    }

    fn round_trip<T>(value: &T)
    where
        T: Serialize + DeserializeOwned + PartialEq + fmt::Debug,
    {
        let encoded = serde_json::to_vec(value).unwrap();
        let decoded: T = serde_json::from_slice(&encoded).unwrap();
        assert_eq!(&decoded, value);
    }

    #[test]
    fn launch_requests_round_trip_and_correlate() {
        let mut request = launch(&workload());
        request.graphical = true;
        round_trip(&request);
        let encoded = serde_json::to_string(&request).unwrap();
        assert!(!encoded.contains("request_id"));
        assert!(!encoded.contains("operation_id"));
    }

    /// The launch boundary: the graph admitted the launch for one requester
    /// and one committed row, and a transport may only check that. A
    /// different requester, a different row, and a presentation this family
    /// cannot enforce are each refused, and a tampered admission is refused
    /// before any of them.
    #[test]
    fn a_launch_runs_only_under_the_admission_its_own_subject_carries() {
        let committed = workload();
        let admission = admission_for(&committed);
        assert_eq!(
            admission.admit_launch(&committed, REQUESTER_UID),
            Ok(()),
            "the admitted requester may launch its own admitted row"
        );
        assert_eq!(
            admission.admit_launch(&committed, REQUESTER_UID + 1),
            Err(HelperFailureCode::RequesterMismatch),
            "a transport whose own identity differs never supplies the requester"
        );
        assert_eq!(
            admission.admit_launch(
                &zone_identity(
                    "work",
                    "123e4567-e89b-42d3-a456-426614174000",
                    "523e4567-e89b-42d3-a456-426614174002",
                    1,
                ),
                REQUESTER_UID
            ),
            Err(HelperFailureCode::GraphAdmissionRequired),
            "an admission for one row admits no other"
        );

        // A later generation of the same row is a different committed
        // identity: the fence moves with the row.
        assert_eq!(
            admission.admit_launch(
                &zone_identity(
                    "work",
                    "123e4567-e89b-42d3-a456-426614174000",
                    "323e4567-e89b-42d3-a456-426614174002",
                    2,
                ),
                REQUESTER_UID
            ),
            Err(HelperFailureCode::GraphAdmissionRequired)
        );

        let presented = HelperGraphAdmission::new(
            committed.clone(),
            REQUESTER_UID,
            UnsafeLocalPosture::ExplicitNoIsolation,
            vec![BindingRealizationFacet::FilesystemPresentation],
        )
        .expect("a presentation-bearing admission is constructible");
        assert_eq!(
            presented.admit_launch(&committed, REQUESTER_UID),
            Err(HelperFailureCode::GraphAdmissionRequired),
            "a destination or view this family cannot realize refuses before launch"
        );

        let mut tampered = admission_for(&committed);
        tampered.requester_uid = REQUESTER_UID + 1;
        assert_eq!(
            tampered.admit_launch(&committed, REQUESTER_UID + 1),
            Err(HelperFailureCode::GraphAdmissionRequired),
            "an admission whose facts no longer match its digest is refused"
        );
    }

    #[test]
    fn helper_frames_reject_unknown_and_forbidden_fields() {
        let hello = r#"{
          "type":"hello",
          "payload":{"protocolVersion":4,"generation":1,"features":[],"uid":1000}
        }"#;
        assert!(serde_json::from_str::<UnsafeLocalHelperToDaemon>(hello).is_err());

        let launch = serde_json::json!({
            "type": "launch",
            "payload": {
                "requestId": 1,
                "operationId": "op-launch",
                "workload": serde_json::to_value(workload()).unwrap(),
                "admission": serde_json::to_value(admission_for(&workload())).unwrap(),
                "target": "tools.work.d2b",
                "itemId": "browser",
                "argv": ["browser"],
                "graphical": false,
                "realmAccentColor": "#cc3344",
                "cwd": "/forbidden"
            }
        });
        assert!(serde_json::from_value::<DaemonToUnsafeLocalHelper>(launch).is_err());
    }

    #[test]
    fn older_helper_versions_are_rejected() {
        assert_eq!(UNSAFE_LOCAL_HELPER_PROTOCOL_VERSION, 4);
        for older in [1, 2, 3] {
            assert!(
                !unsafe_local_helper_protocol_supported(older),
                "a version {older} peer predates the admitted launch frame"
            );
        }
        assert!(unsafe_local_helper_protocol_supported(4));
    }

    #[test]
    fn realm_accent_color_is_strict_and_canonical() {
        let color = RealmAccentColor::new("#cc3344").unwrap();
        assert_eq!(color.as_str(), "#cc3344");
        for invalid in [
            "cc3344",
            "#CC3344",
            "#123",
            "#1234567",
            "#12345g",
            "#123456\n",
        ] {
            assert!(RealmAccentColor::new(invalid).is_err(), "{invalid:?}");
            assert!(
                serde_json::from_value::<RealmAccentColor>(serde_json::json!(invalid)).is_err(),
                "{invalid:?}"
            );
        }
    }

    #[test]
    fn helper_socket_buffer_floors_stay_closed() {
        assert_eq!(HELPER_SOCKET_BUFFER_REQUEST_BYTES, MAX_HELPER_FRAME_SIZE);
        assert_eq!(
            MIN_EFFECTIVE_HELPER_SOCKET_BUFFER_BYTES,
            MAX_HELPER_FRAME_SIZE * 2
        );
    }
}
