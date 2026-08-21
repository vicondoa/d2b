//! Out-of-band cutover runner capability and lifecycle primitives.
//!
//! This module owns only the runner boundary. It does not perform a host
//! mutation and it never exposes the durable journal to a client. The
//! broker-created child consumes [`BootstrapCapability`] once, then uses the
//! owner-authenticated socket for status and safety holds while the daemon is
//! unavailable.

use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::{
        fd::{AsRawFd, RawFd},
        unix::{
            fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
            net::{UnixListener, UnixStream},
        },
    },
    path::{Path, PathBuf},
    time::Duration,
};

use d2b_contracts_resource::v3::{CanonicalJsonValue, canonical_json_bytes};
use nix::{
    fcntl::{FcntlArg, fcntl},
    libc,
    sys::socket::{getsockopt, sockopt::PeerCredentials},
    unistd::{Gid, Uid, chown, geteuid},
};
use serde::{Deserialize, Serialize};

use crate::{
    ArtifactId, CandidateId, Consent, CutoverPhase, CutoverPreview, Digest, EffectAllowlist,
    EffectId, EffectKind, Journal, JournalBinding, Operation, OperationId, OperationKind,
    OperationRequest, OperationState, OperatorId, RecoveryAttestation, ReplayClass, StepId, ZoneId,
};

/// Protocol version for the runner bootstrap and owner socket.
pub const RUNNER_PROTOCOL_VERSION: u16 = 1;
/// Maximum bytes accepted from a broker bootstrap fd or owner socket frame.
pub const MAX_RUNNER_FRAME_BYTES: usize = 64 * 1024;
/// Maximum lifetime of a bootstrap capability.
pub const MAX_BOOTSTRAP_LIFETIME_MS: u64 = 15 * 60 * 1000;
/// Fixed fd number used by the runner child for its one-shot bootstrap.
pub const RUNNER_BOOTSTRAP_FD: RawFd = 3;
/// Maximum duration for one owner-socket round trip.
pub const RUNNER_SOCKET_TIMEOUT: Duration = Duration::from_secs(10);

/// A single-use capability transferred from the live broker to the runner.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapCapability {
    version: u16,
    operation_id: OperationId,
    candidate_id: CandidateId,
    operator_id: OperatorId,
    operation_kind: OperationKind,
    effect_allowlist: EffectAllowlist,
    nonce: Digest,
    issued_at_ms: u64,
    expires_at_ms: u64,
    /// Uid of the bound operator, used only for socket authentication.
    operator_uid: u32,
    /// Admin uids admitted to set a safety hold during drain.
    admin_uids: BTreeSet<u32>,
    /// Optional lifecycle group gid used for the socket filesystem gate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lifecycle_gid: Option<u32>,
}

impl BootstrapCapability {
    /// Construct a bounded capability for one operation.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation_id: OperationId,
        candidate_id: CandidateId,
        operator_id: OperatorId,
        operation_kind: OperationKind,
        nonce: Digest,
        issued_at_ms: u64,
        expires_at_ms: u64,
    ) -> Result<Self, RunnerCapabilityError> {
        Self::new_with_identity(
            operation_id,
            candidate_id,
            operator_id,
            operation_kind,
            nonce,
            issued_at_ms,
            expires_at_ms,
            0,
            BTreeSet::new(),
        )
    }

    /// Construct a capability with the socket identity allowlist.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_identity(
        operation_id: OperationId,
        candidate_id: CandidateId,
        operator_id: OperatorId,
        operation_kind: OperationKind,
        nonce: Digest,
        issued_at_ms: u64,
        expires_at_ms: u64,
        operator_uid: u32,
        admin_uids: BTreeSet<u32>,
    ) -> Result<Self, RunnerCapabilityError> {
        validate_time_window(issued_at_ms, expires_at_ms)?;
        Self::new_with_identity_and_group(
            operation_id,
            candidate_id,
            operator_id,
            operation_kind,
            nonce,
            issued_at_ms,
            expires_at_ms,
            operator_uid,
            admin_uids,
            None,
        )
    }

    /// Construct a capability with identity and lifecycle group bindings.
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_identity_and_group(
        operation_id: OperationId,
        candidate_id: CandidateId,
        operator_id: OperatorId,
        operation_kind: OperationKind,
        nonce: Digest,
        issued_at_ms: u64,
        expires_at_ms: u64,
        operator_uid: u32,
        admin_uids: BTreeSet<u32>,
        lifecycle_gid: Option<u32>,
    ) -> Result<Self, RunnerCapabilityError> {
        validate_time_window(issued_at_ms, expires_at_ms)?;
        Ok(Self {
            version: RUNNER_PROTOCOL_VERSION,
            operation_id,
            candidate_id,
            operator_id,
            operation_kind,
            effect_allowlist: EffectAllowlist::for_operation(operation_kind),
            nonce,
            issued_at_ms,
            expires_at_ms,
            operator_uid,
            admin_uids,
            lifecycle_gid,
        })
    }

    /// Render the exact canonical capability bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, RunnerCapabilityError> {
        canonical_json_bytes(self).map_err(|_| RunnerCapabilityError::CanonicalJson)
    }

    /// Derive the capability proof presented to an adapted broker.
    pub fn binding_digest(&self) -> Digest {
        let bytes = self
            .canonical_bytes()
            .expect("validated capability serializes");
        Digest::derive("d2b:cutover:runner-capability:v1", &bytes)
    }

    /// Decode and consume one capability from canonical bytes.
    pub fn decode_and_consume(
        bytes: &[u8],
        now_ms: u64,
        ledger: &mut CapabilityLedger,
    ) -> Result<ConsumedCapability, RunnerCapabilityError> {
        CanonicalJsonValue::parse(bytes).map_err(|_| RunnerCapabilityError::CanonicalJson)?;
        let capability: Self =
            serde_json::from_slice(bytes).map_err(|_| RunnerCapabilityError::Malformed)?;
        capability.consume(now_ms, ledger)
    }

    /// Borrow the operation identity.
    pub fn operation_id(&self) -> &OperationId {
        &self.operation_id
    }

    /// Borrow the candidate identity.
    pub fn candidate_id(&self) -> &CandidateId {
        &self.candidate_id
    }

    /// Borrow the bound operator identity.
    pub fn operator_id(&self) -> &OperatorId {
        &self.operator_id
    }

    /// Return the operation authority.
    pub const fn operation_kind(&self) -> OperationKind {
        self.operation_kind
    }

    /// Borrow the closed effect allowlist.
    pub fn effect_allowlist(&self) -> &EffectAllowlist {
        &self.effect_allowlist
    }

    /// Return the bound operator uid.
    pub const fn operator_uid(&self) -> u32 {
        self.operator_uid
    }

    /// Return whether a uid is one of the admitted Admin peers.
    pub fn is_admin_uid(&self, uid: u32) -> bool {
        self.admin_uids.contains(&uid)
    }

    /// Return the optional lifecycle group gid.
    pub const fn lifecycle_gid(&self) -> Option<u32> {
        self.lifecycle_gid
    }

    /// Return the capability expiry.
    pub const fn expires_at_ms(&self) -> u64 {
        self.expires_at_ms
    }

    fn consume(
        self,
        now_ms: u64,
        ledger: &mut CapabilityLedger,
    ) -> Result<ConsumedCapability, RunnerCapabilityError> {
        if self.version != RUNNER_PROTOCOL_VERSION {
            return Err(RunnerCapabilityError::Version);
        }
        validate_time_window(self.issued_at_ms, self.expires_at_ms)?;
        if now_ms < self.issued_at_ms || now_ms > self.expires_at_ms {
            return Err(RunnerCapabilityError::Expired);
        }
        if self.effect_allowlist != EffectAllowlist::for_operation(self.operation_kind) {
            return Err(RunnerCapabilityError::EffectAllowlistMismatch);
        }
        if !ledger.nonces.insert(self.nonce.clone()) {
            return Err(RunnerCapabilityError::AlreadyConsumed);
        }
        Ok(ConsumedCapability(self))
    }
}

/// An in-process replay ledger for bootstrap capabilities.
#[derive(Debug, Default)]
pub struct CapabilityLedger {
    nonces: BTreeSet<Digest>,
}

/// The capability after its one-time consumption boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConsumedCapability(BootstrapCapability);

impl ConsumedCapability {
    /// Return the operation authority.
    pub const fn operation_kind(&self) -> OperationKind {
        self.0.operation_kind
    }

    /// Borrow the underlying operation identity.
    pub fn operation_id(&self) -> &OperationId {
        &self.0.operation_id
    }

    /// Borrow the candidate identity.
    pub fn candidate_id(&self) -> &CandidateId {
        &self.0.candidate_id
    }

    /// Borrow the bound operator identity.
    pub fn operator_id(&self) -> &OperatorId {
        &self.0.operator_id
    }

    /// Return whether this capability admits a peer uid as an Admin.
    pub fn is_admin_uid(&self, uid: u32) -> bool {
        self.0.is_admin_uid(uid)
    }

    /// Return the optional lifecycle group gid.
    pub const fn lifecycle_gid(&self) -> Option<u32> {
        self.0.lifecycle_gid
    }

    /// Return the bound operator uid.
    pub const fn operator_uid(&self) -> u32 {
        self.0.operator_uid
    }

    /// Borrow the closed effect allowlist.
    pub fn effect_allowlist(&self) -> &EffectAllowlist {
        &self.0.effect_allowlist
    }

    /// Derive the capability binding digest for restart adoption checks.
    pub fn binding_digest(&self) -> Digest {
        self.0.binding_digest()
    }

    /// Return the expiry.
    pub const fn expires_at_ms(&self) -> u64 {
        self.0.expires_at_ms
    }
}

/// The complete bootstrap payload sent over the single-use fd.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerBootstrap {
    /// Capability consumed before any runner state is opened.
    pub capability: BootstrapCapability,
    /// Immutable operation request bound to the preview.
    pub request: OperationRequest,
    /// Canonical preview used to reconstruct the pure engine.
    pub preview: CutoverPreview,
    /// Exact single-use apply consent, when this runner is admitted to apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub consent: Option<Consent>,
    /// Separate destructive consent for durable-Volume reset effects.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive_consent: Option<Consent>,
    /// Qualified external recovery evidence for cutover apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<RecoveryAttestation>,
    /// Host identity digest bound by the recovery evidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_digest: Option<Digest>,
}

/// One approved legacy artifact disposition for phase-10 finalization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalizationArtifact {
    /// Opaque artifact identity.
    pub artifact_id: ArtifactId,
    /// Digest of the disposition approved by the operation inventory.
    pub disposition_digest: Digest,
}

/// Phase-10 finalization payload bound to one operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FinalizationPlan {
    /// Approved artifact dispositions.
    pub artifacts: Vec<FinalizationArtifact>,
}

/// One redaction-safe post-activation Zone observation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerZoneVerification {
    /// Opaque Zone identity.
    pub zone_id: ZoneId,
    /// Whether the Zone passed its authoritative checks.
    pub healthy: bool,
}

/// Authoritative observations required by phase-9 verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerVerificationInput {
    /// All configured Zone observations.
    pub zones: Vec<RunnerZoneVerification>,
    /// Whether every preserved source remained intact.
    pub sources_preserved: bool,
    /// Whether identity digests matched the cutover snapshot.
    pub identity_digests_match: bool,
    /// Whether the candidate remains current.
    pub candidate_current: bool,
}

impl RunnerBootstrap {
    /// Validate the bootstrap's cross-object identity bindings.
    pub fn validate(&self) -> Result<(), RunnerCapabilityError> {
        let capability = &self.capability;
        if capability.operation_id() != self.request.operation_id()
            || capability.candidate_id() != self.request.candidate_id()
            || capability.operator_id() != self.request.operator_id()
            || capability.operation_kind() != self.request.operation_kind()
        {
            return Err(RunnerCapabilityError::IdentityMismatch);
        }
        if self.preview.operation_id() != self.request.operation_id()
            || self.preview.candidate_id() != self.request.candidate_id()
            || self.preview.revision_plan_id() != self.request.revision_plan_id()
            || self.preview.operation_kind() != self.request.operation_kind()
            || self
                .preview
                .digest()
                .map_err(|_| RunnerCapabilityError::Preview)?
                != *self.request.preview_digest()
        {
            return Err(RunnerCapabilityError::Preview);
        }
        if !self
            .request
            .request_digest_matches()
            .map_err(|_| RunnerCapabilityError::Request)?
        {
            return Err(RunnerCapabilityError::Request);
        }
        if self.request.system_artifact_id() != self.preview.system_artifact_id()
            || self.request.source_system_artifact_id() != self.preview.source_system_artifact_id()
        {
            return Err(RunnerCapabilityError::Preview);
        }
        Operation::new(self.request.clone(), &self.preview)
            .map(|_| ())
            .map_err(|_| RunnerCapabilityError::Request)?;
        if self.request.operation_kind().is_cutover() {
            let Some(consent) = self.consent.as_ref() else {
                return Err(RunnerCapabilityError::Consent);
            };
            let Some(recovery) = self.recovery.as_ref() else {
                return Err(RunnerCapabilityError::Recovery);
            };
            let Some(host_digest) = self.host_digest.as_ref() else {
                return Err(RunnerCapabilityError::Recovery);
            };
            if consent.binding() != &self.request.consent_binding()
                || recovery
                    .digest()
                    .map_err(|_| RunnerCapabilityError::Recovery)?
                    != *self
                        .request
                        .recovery_digest()
                        .ok_or(RunnerCapabilityError::Recovery)?
                || recovery.candidate_id() != self.request.candidate_id()
                || recovery.preview_digest() != self.request.preview_digest()
                || recovery.operator_id() != self.request.operator_id()
            {
                return Err(RunnerCapabilityError::Recovery);
            }
            let _ = host_digest;
        }
        if matches!(
            self.request.inventory(),
            crate::OperationInventory::Reset(inventory)
                if inventory.allows_destroy_durable_volumes()
        ) {
            let Some(consent) = self.destructive_consent.as_ref() else {
                return Err(RunnerCapabilityError::Consent);
            };
            if consent.binding() != &self.request.consent_binding() {
                return Err(RunnerCapabilityError::Consent);
            }
        }
        Ok(())
    }

    /// Render the exact canonical bootstrap bytes.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, RunnerCapabilityError> {
        self.validate()?;
        canonical_json_bytes(self).map_err(|_| RunnerCapabilityError::CanonicalJson)
    }

    /// Decode and consume a bootstrap from a trusted fd payload.
    pub fn decode_and_consume(
        bytes: &[u8],
        now_ms: u64,
        ledger: &mut CapabilityLedger,
    ) -> Result<(Self, ConsumedCapability), RunnerCapabilityError> {
        CanonicalJsonValue::parse(bytes).map_err(|_| RunnerCapabilityError::CanonicalJson)?;
        let bootstrap: Self =
            serde_json::from_slice(bytes).map_err(|_| RunnerCapabilityError::Malformed)?;
        let capability_bytes = bootstrap
            .capability
            .canonical_bytes()
            .map_err(|_| RunnerCapabilityError::CanonicalJson)?;
        let consumed = BootstrapCapability::decode_and_consume(&capability_bytes, now_ms, ledger)?;
        bootstrap.validate()?;
        Ok((bootstrap, consumed))
    }
}

/// Durable runner paths derived from one opaque operation id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunnerPaths {
    root: PathBuf,
    operation_dir: PathBuf,
    socket_dir: PathBuf,
    /// Root-owned journal path.
    pub journal: PathBuf,
    /// Host-wide OFD lock path.
    pub lock: PathBuf,
    /// Owner-authenticated status socket.
    pub socket: PathBuf,
}

impl RunnerPaths {
    /// Derive anchored paths without touching the filesystem.
    pub fn new(root: impl Into<PathBuf>, operation_id: &OperationId) -> Self {
        let root = root.into();
        Self::new_with_socket_root(root.clone(), root, operation_id)
    }

    /// Derive state paths and a separate runtime socket path.
    pub fn new_with_socket_root(
        root: impl Into<PathBuf>,
        socket_root: impl Into<PathBuf>,
        operation_id: &OperationId,
    ) -> Self {
        let root = root.into();
        let operation_dir = root.join("cutover").join(operation_id.as_str());
        let socket_dir = socket_root
            .into()
            .join("cutover")
            .join(operation_id.as_str());
        Self {
            journal: operation_dir.join("journal.json"),
            lock: operation_dir.join("operation.lock"),
            socket: socket_dir.join("runner.sock"),
            root,
            operation_dir,
            socket_dir,
        }
    }

    /// Create the private operation directory with a bounded mode.
    pub fn ensure_directory(&self) -> io::Result<()> {
        ensure_owned_directory(&self.operation_dir, 0o700)
    }

    /// Borrow the operation directory.
    pub fn operation_dir(&self) -> &Path {
        &self.operation_dir
    }

    /// Borrow the runtime socket directory.
    pub fn socket_dir(&self) -> &Path {
        &self.socket_dir
    }

    /// Borrow the state root.
    pub fn root(&self) -> &Path {
        &self.root
    }
}

/// Acquire the operation's host-wide OFD lock.
pub fn acquire_operation_lock(paths: &RunnerPaths) -> Result<File, RunnerLockError> {
    paths.ensure_directory().map_err(|_| RunnerLockError::Io)?;
    if let Ok(metadata) = fs::symlink_metadata(&paths.lock)
        && (!metadata.is_file()
            || metadata.uid() != geteuid().as_raw()
            || metadata.permissions().mode() & 0o777 != 0o600)
    {
        return Err(RunnerLockError::Io);
    }
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .mode(0o600)
        .open(&paths.lock)
        .map_err(|_| RunnerLockError::Io)?;
    let lock = libc::flock {
        l_type: libc::F_WRLCK as i16,
        l_whence: libc::SEEK_SET as i16,
        l_start: 0,
        l_len: 0,
        l_pid: 0,
    };
    fcntl(file.as_raw_fd(), FcntlArg::F_OFD_SETLK(&lock))
        .map_err(|_| RunnerLockError::Contended)?;
    Ok(file)
}

/// Persist a root-only runner journal envelope atomically.
pub fn persist_journal(path: &Path, bootstrap: &RunnerBootstrap, records: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "runner journal has no parent")
    })?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (!metadata.is_file()
            || metadata.uid() != geteuid().as_raw()
            || metadata.permissions().mode() & 0o777 != 0o600)
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runner journal has foreign ownership",
        ));
    }
    fs::create_dir_all(parent)?;
    let record_values = canonical_record_values(records)?;
    let payload = serde_json::json!({
        "bootstrap": bootstrap,
        "records": record_values,
    });
    let bytes = canonical_json_bytes(&payload).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "runner journal is not canonical",
        )
    })?;
    if bytes.len() > MAX_RUNNER_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runner journal exceeds size limit",
        ));
    }
    let temp = path.with_extension("journal.tmp");
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&temp, path)?;
    File::open(parent)?.sync_all()?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

/// Read and validate a runner journal envelope without exposing it to a CLI.
pub fn load_journal(path: &Path) -> io::Result<(RunnerBootstrap, Vec<u8>)> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = file.metadata()?;
    if metadata.uid() != geteuid().as_raw() || metadata.permissions().mode() & 0o777 != 0o600 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runner journal mode is not 0600",
        ));
    }
    let mut bytes = Vec::new();
    file.take((MAX_RUNNER_FRAME_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RUNNER_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runner journal exceeds size limit",
        ));
    }
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runner journal malformed"))?;
    CanonicalJsonValue::parse(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runner journal not canonical"))?;
    let bootstrap: RunnerBootstrap =
        serde_json::from_value(value.get("bootstrap").cloned().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "runner bootstrap missing")
        })?)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runner bootstrap malformed"))?;
    bootstrap
        .validate()
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runner bootstrap invalid"))?;
    let record_values: Vec<CanonicalJsonValue> = value
        .get("records")
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "runner journal records missing"))
        .and_then(|value| {
            serde_json::from_value(value).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "runner journal records malformed",
                )
            })
        })?;
    let mut records = Vec::new();
    for value in record_values {
        records.extend(canonical_json_bytes(&value).map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "runner journal record not canonical",
            )
        })?);
        records.push(b'\n');
    }
    Journal::from_bytes(
        JournalBinding::new(
            bootstrap.request.operation_id().clone(),
            bootstrap.request.revision_plan_id().clone(),
            bootstrap.request.request_digest().clone(),
        ),
        &records,
    )
    .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runner journal chain invalid"))?;
    Ok((bootstrap, records))
}

fn canonical_record_values(records: &[u8]) -> io::Result<Vec<CanonicalJsonValue>> {
    if records.is_empty() {
        return Ok(Vec::new());
    }
    if !records.ends_with(b"\n") {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runner journal records are truncated",
        ));
    }
    records
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            CanonicalJsonValue::parse(line).map_err(|_| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    "runner journal record not canonical",
                )
            })
        })
        .collect()
}

/// One owner-socket command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "command", deny_unknown_fields)]
pub enum RunnerCommand {
    /// Read redacted operation status.
    Status,
    /// Apply one typed closure activation after consent and drain admission.
    Apply {
        /// Existing typed host-generation handoff contract.
        handoff: d2b_contracts_broker::host_generation::ApplyHostGenerationHandoff,
    },
    /// Dispatch one closed U3 effect through the adapted broker peer.
    Effect {
        /// Stable effect identity.
        effect_id: EffectId,
        /// Stable step identity.
        step_id: StepId,
        /// Closed U3 effect kind.
        kind: EffectKind,
        /// Crash-replay class.
        replay_class: ReplayClass,
        /// Phase reached after durable completion.
        advance_to: Option<CutoverPhase>,
        /// Existing identity for identity-bearing effects.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        identity: Option<ArtifactId>,
        /// Typed generation handoff for closure activation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        handoff: Option<d2b_contracts_broker::host_generation::ApplyHostGenerationHandoff>,
        /// Existing typed broker operation payload.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        payload: Option<d2b_contracts_broker::broker_wire::CutoverEffectPayload>,
    },
    /// Roll back while the native rollback boundary is still open.
    Rollback {
        /// Optional typed handoff that restores the pre-apply generation.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        handoff: Option<d2b_contracts_broker::host_generation::ApplyHostGenerationHandoff>,
    },
    /// Request phase-9 verification through the runner boundary.
    Verify {
        /// Authoritative post-activation observations.
        observations: RunnerVerificationInput,
    },
    /// Consume the separately issued phase-10 finalization consent.
    Finalize {
        /// Canonical finalization consent artifact.
        consent: crate::FinalizationConsent,
        /// Approved artifact disposition payload.
        plan: FinalizationPlan,
    },
    /// Request an operator hold.
    Hold {
        /// Bounded operator reason.
        reason: String,
    },
    /// Resume after an owner or fresh-consent check.
    Resume {
        /// Optional digest-bound fresh consent for a non-owner Admin.
        #[serde(default)]
        fresh_consent: Option<Digest>,
    },
}

/// Redaction-safe status returned over the owner socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerStatus {
    /// Opaque operation identity.
    pub operation_id: OperationId,
    /// Preview digest admitted for this operation.
    pub preview_digest: Digest,
    /// Current pure-engine state.
    pub state: OperationState,
    /// Current phase.
    pub phase: CutoverPhase,
    /// Whether a hold blocks the next effect.
    pub hold_active: bool,
    /// Whether the runner has reached a terminal outcome.
    pub terminal: bool,
}

/// Response envelope for owner-socket commands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RunnerResponse {
    /// Whether the command was accepted.
    pub accepted: bool,
    /// Redacted status snapshot.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<RunnerStatus>,
    /// Stable failure code, never a raw error string.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<RunnerSocketError>,
}

/// Stable owner-socket refusal classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunnerSocketError {
    /// The peer is not an admitted lifecycle identity.
    Unauthorized,
    /// The peer is not the bound operator or did not supply fresh consent.
    OperatorMismatch,
    /// The handoff artifact does not match the admitted cutover binding.
    ArtifactBindingMismatch,
    /// The command is not valid for the current operation state.
    InvalidTransition,
    /// The command frame was malformed or oversized.
    Malformed,
    /// The privileged audit sink did not return durable evidence.
    AuditUnavailable,
    /// The journal could not be durably advanced.
    JournalUnavailable,
}

/// Owner-authenticated runner socket.
pub struct RunnerSocket {
    listener: UnixListener,
    capability: ConsumedCapability,
}

fn ensure_owned_directory(path: &Path, mode: u32) -> io::Result<()> {
    let owner = geteuid().as_raw();
    let mut missing = Vec::new();
    let mut current = path;
    loop {
        match fs::symlink_metadata(current) {
            Ok(metadata) => {
                if !metadata.is_dir() || metadata.uid() != owner {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "runner directory has foreign ownership",
                    ));
                }
                if current == path && metadata.permissions().mode() & 0o777 != mode {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "runner directory has an unexpected mode",
                    ));
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(current.to_path_buf());
                current = current.parent().ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "runner directory has no owned parent",
                    )
                })?;
            }
            Err(error) => return Err(error),
        }
    }
    for directory in missing.into_iter().rev() {
        fs::create_dir(&directory)?;
        fs::set_permissions(&directory, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

fn validate_runtime_anchor(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.uid() != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime socket anchor has foreign ownership",
        ));
    }
    Ok(())
}

fn validate_runtime_directory(path: &Path, gid: u32) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir()
        || metadata.uid() != 0
        || metadata.gid() != gid
        || metadata.permissions().mode() & 0o777 != 0o710
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime socket directory has foreign ownership or mode",
        ));
    }
    Ok(())
}

fn ensure_runtime_directory(path: &Path, gid: u32) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => validate_runtime_directory(path, gid),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "runtime socket directory has no parent",
                )
            })?;
            validate_runtime_anchor(parent)?;
            fs::create_dir(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o710))?;
            chown(path, Some(Uid::from_raw(0)), Some(Gid::from_raw(gid)))
                .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
            validate_runtime_directory(path, gid)
        }
        Err(error) => Err(error),
    }
}

fn ensure_runtime_socket_directories(paths: &RunnerPaths, gid: u32) -> io::Result<()> {
    let shared = paths.socket_dir().parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runtime socket directory has no shared parent",
        )
    })?;
    ensure_runtime_directory(shared, gid)?;
    ensure_runtime_directory(paths.socket_dir(), gid)
}

fn validate_socket_path(path: &Path, lifecycle_gid: Option<u32>) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    let expected_uid = if lifecycle_gid.is_some() {
        0
    } else {
        geteuid().as_raw()
    };
    let expected_mode = if lifecycle_gid.is_some() {
        0o660
    } else {
        0o600
    };
    if !metadata.file_type().is_socket()
        || metadata.uid() != expected_uid
        || lifecycle_gid.is_some_and(|gid| metadata.gid() != gid)
        || metadata.permissions().mode() & 0o777 != expected_mode
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "runner socket has foreign ownership or mode",
        ));
    }
    Ok(())
}

impl RunnerSocket {
    /// Bind one owner-authenticated socket. Existing foreign paths fail closed.
    pub fn bind(paths: &RunnerPaths, capability: ConsumedCapability) -> io::Result<Self> {
        paths.ensure_directory()?;
        let lifecycle_gid = capability.lifecycle_gid();
        if let Some(gid) = lifecycle_gid {
            ensure_runtime_socket_directories(paths, gid)?;
        } else {
            ensure_owned_directory(paths.socket_dir(), 0o700)?;
        }

        match fs::symlink_metadata(&paths.socket) {
            Ok(_) => {
                validate_socket_path(&paths.socket, lifecycle_gid)?;
                fs::remove_file(&paths.socket)?;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let listener = UnixListener::bind(&paths.socket)?;
        let socket_mode = if lifecycle_gid.is_some() {
            0o660
        } else {
            0o600
        };
        fs::set_permissions(&paths.socket, fs::Permissions::from_mode(socket_mode))?;
        if let Some(gid) = lifecycle_gid {
            chown(
                &paths.socket,
                Some(Uid::from_raw(0)),
                Some(Gid::from_raw(gid)),
            )
            .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
        }
        validate_socket_path(&paths.socket, lifecycle_gid)?;
        Ok(Self {
            listener,
            capability,
        })
    }

    /// Return the bound socket path.
    pub fn path(&self) -> io::Result<PathBuf> {
        self.listener.local_addr().and_then(|addr| {
            addr.as_pathname()
                .map(PathBuf::from)
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "unnamed runner socket"))
        })
    }

    /// Accept one authenticated request and return its typed command.
    pub fn accept_command(&self) -> io::Result<(UnixStream, RunnerCommand, RunnerPeer)> {
        let (mut stream, _) = self.listener.accept()?;
        let credentials = getsockopt(&stream, PeerCredentials)
            .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
        authorize_peer(credentials.uid(), &self.capability, &RunnerCommand::Status).map_err(
            |error| io::Error::new(io::ErrorKind::PermissionDenied, format!("{error:?}")),
        )?;
        let command = read_json_frame(&mut stream)?;
        let peer =
            authorize_peer(credentials.uid(), &self.capability, &command).map_err(|error| {
                io::Error::new(io::ErrorKind::PermissionDenied, format!("{error:?}"))
            })?;
        Ok((stream, command, peer))
    }

    /// Authenticate a peer for one command without accepting a socket.
    pub fn authorize(
        &self,
        peer_uid: u32,
        command: &RunnerCommand,
    ) -> Result<RunnerPeer, RunnerSocketError> {
        authorize_peer(peer_uid, &self.capability, command)
    }
}

/// The peer class proven by lifecycle `SO_PEERCRED`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerPeer {
    /// The bound operator.
    Owner,
    /// Another configured Admin.
    Admin,
}

fn authorize_peer(
    peer_uid: u32,
    capability: &ConsumedCapability,
    command: &RunnerCommand,
) -> Result<RunnerPeer, RunnerSocketError> {
    let peer = if peer_uid == capability.operator_uid() {
        RunnerPeer::Owner
    } else if capability.is_admin_uid(peer_uid) {
        RunnerPeer::Admin
    } else {
        return Err(RunnerSocketError::Unauthorized);
    };
    match command {
        RunnerCommand::Status => Ok(peer),
        RunnerCommand::Apply { .. } => {
            if matches!(peer, RunnerPeer::Owner) {
                Ok(peer)
            } else {
                Err(RunnerSocketError::OperatorMismatch)
            }
        }
        RunnerCommand::Effect { .. }
        | RunnerCommand::Rollback { .. }
        | RunnerCommand::Verify { .. }
        | RunnerCommand::Finalize { .. } => {
            if matches!(peer, RunnerPeer::Owner) {
                Ok(peer)
            } else {
                Err(RunnerSocketError::OperatorMismatch)
            }
        }
        RunnerCommand::Hold { .. } => Ok(peer),
        RunnerCommand::Resume { fresh_consent } => {
            if matches!(peer, RunnerPeer::Owner) || fresh_consent.is_some() {
                Ok(peer)
            } else {
                Err(RunnerSocketError::OperatorMismatch)
            }
        }
    }
}

fn read_json_frame(stream: &mut UnixStream) -> io::Result<RunnerCommand> {
    let mut bytes = Vec::new();
    stream
        .take((MAX_RUNNER_FRAME_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_RUNNER_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runner command exceeds size limit",
        ));
    }
    serde_json::from_slice(&bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runner command malformed"))
}

/// Send one owner-socket command and decode its response.
pub fn send_command(path: &Path, command: &RunnerCommand) -> io::Result<RunnerResponse> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_write_timeout(Some(RUNNER_SOCKET_TIMEOUT))?;
    stream.set_read_timeout(Some(RUNNER_SOCKET_TIMEOUT))?;
    let bytes = serde_json::to_vec(command)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runner command encode failed"))?;
    stream.write_all(&bytes)?;
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut response = Vec::new();
    stream
        .take((MAX_RUNNER_FRAME_BYTES + 1) as u64)
        .read_to_end(&mut response)?;
    if response.len() > MAX_RUNNER_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "runner response exceeds size limit",
        ));
    }
    serde_json::from_slice(&response)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runner response malformed"))
}

/// Write one owner-socket response.
pub fn write_response(stream: &mut UnixStream, response: &RunnerResponse) -> io::Result<()> {
    let bytes = serde_json::to_vec(response)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "runner response encode failed"))?;
    stream.write_all(&bytes)
}

/// Runner capability validation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerCapabilityError {
    /// Canonical JSON was rejected.
    CanonicalJson,
    /// The JSON shape was not the closed contract.
    Malformed,
    /// The protocol version is unsupported.
    Version,
    /// The capability lifetime is invalid.
    InvalidLifetime,
    /// The capability is expired at consumption time.
    Expired,
    /// The capability nonce was already consumed.
    AlreadyConsumed,
    /// The effect allowlist did not match the operation kind.
    EffectAllowlistMismatch,
    /// The bootstrap identities did not match.
    IdentityMismatch,
    /// The bootstrap preview did not match the operation request.
    Preview,
    /// The operation request was not accepted by the U3 engine.
    Request,
    /// Exact apply consent was not present or bound.
    Consent,
    /// Qualified recovery evidence was not present or bound.
    Recovery,
}

impl std::fmt::Display for RunnerCapabilityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::CanonicalJson => "cutover bootstrap capability is not canonical JSON",
            Self::Malformed => "cutover bootstrap capability is malformed",
            Self::Version => "cutover bootstrap capability version is unsupported",
            Self::InvalidLifetime => "cutover bootstrap capability lifetime is invalid",
            Self::Expired => "cutover bootstrap capability expired",
            Self::AlreadyConsumed => "cutover bootstrap capability already consumed",
            Self::EffectAllowlistMismatch => {
                "cutover bootstrap capability effect allowlist mismatch"
            }
            Self::IdentityMismatch => "cutover bootstrap capability identity mismatch",
            Self::Preview => "cutover bootstrap capability preview mismatch",
            Self::Request => "cutover bootstrap capability request rejected",
            Self::Consent => "cutover bootstrap apply consent rejected",
            Self::Recovery => "cutover bootstrap recovery evidence rejected",
        })
    }
}

impl std::error::Error for RunnerCapabilityError {}

/// Operation lock failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerLockError {
    /// Filesystem I/O failed.
    Io,
    /// Another operation owns the OFD lock.
    Contended,
}

impl std::fmt::Display for RunnerLockError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Io => "cutover operation lock I/O failed",
            Self::Contended => "cutover operation lock is already held",
        })
    }
}

impl std::error::Error for RunnerLockError {}

fn validate_time_window(
    issued_at_ms: u64,
    expires_at_ms: u64,
) -> Result<(), RunnerCapabilityError> {
    if expires_at_ms <= issued_at_ms
        || expires_at_ms.saturating_sub(issued_at_ms) > MAX_BOOTSTRAP_LIFETIME_MS
    {
        return Err(RunnerCapabilityError::InvalidLifetime);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_paths_keep_opaque_ids_anchored() {
        let id = OperationId::new("op-path-test").expect("id");
        let paths = RunnerPaths::new("/var/lib/d2b", &id);
        assert_eq!(
            paths.socket,
            PathBuf::from("/var/lib/d2b/cutover/op-path-test/runner.sock")
        );
        assert!(!paths.socket.to_string_lossy().contains(".."));
    }

    #[test]
    fn runtime_socket_directory_contract_is_fail_closed_and_traversal_only() {
        let source = include_str!("runner.rs");
        assert!(source.contains("ensure_runtime_socket_directories(paths, gid)"));
        assert!(source.contains("metadata.gid() != gid"));
        assert!(source.contains("metadata.permissions().mode() & 0o777 != 0o710"));
        assert!(source.contains("metadata.permissions().mode() & 0o777 != expected_mode"));
        assert!(!source.contains("chown(\n                paths.socket_dir()"));
    }
}
