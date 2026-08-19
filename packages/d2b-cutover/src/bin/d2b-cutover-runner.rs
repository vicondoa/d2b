//! One-shot, out-of-band cutover owner.
//!
//! The broker launches this process before control-plane drain and transfers
//! the bootstrap over one close-on-exec-controlled fd. The runner owns the
//! durable journal, OFD lock, and drain-window socket. It never accepts a
//! raw host path or performs a host mutation directly.

use std::{
    env,
    fs::OpenOptions,
    io::Read as _,
    os::{fd::AsRawFd, unix::fs::OpenOptionsExt},
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

use d2b_contracts::{
    MAX_FRAME_SIZE,
    broker_wire::{
        BrokerCallerRole, BrokerRequest, BrokerRequestEnvelope, BrokerResponse,
        CanonicalAuditDigest, CutoverAuditRequest, CutoverAuditTransition, CutoverEffectAuthority,
        CutoverEffectKind, CutoverEffectOutcome, CutoverEffectPayload, CutoverEffectRequest,
        CutoverReplayClass,
    },
    types::BundleOpId,
};
use d2b_cutover::{
    ApplyContext, ArtifactId, AuditEvidence, CapabilityLedger, CompletionEvidence, CutoverPhase,
    EffectEvidence, EffectKind, EffectRequest, HoldReason, HostLockContract,
    MAX_RUNNER_FRAME_BYTES, Operation, OperationInventory, OperationKind, RUNNER_BOOTSTRAP_FD,
    ReadOnlyEvidence, ReplayClass, RunnerBootstrap, RunnerCommand, RunnerPaths, RunnerPeer,
    RunnerResponse, RunnerSocket, RunnerStatus, StepId, acquire_operation_lock, persist_journal,
    write_response,
};
use nix::{
    fcntl::{FcntlArg, FdFlag, fcntl},
    libc,
    sys::socket::{
        AddressFamily, MsgFlags, SockFlag, SockType, UnixAddr, connect, recv, send, socket,
    },
    unistd::setsid,
};

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("d2b-cutover-runner: {error}");
            std::process::ExitCode::from(78)
        }
    }
}

fn run() -> Result<(), RunnerError> {
    if nix::unistd::geteuid().as_raw() != 0 {
        return Err(RunnerError::NotRoot);
    }
    setsid().map_err(|_| RunnerError::Session)?;

    let state_root = env::var_os("D2B_CUTOVER_STATE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/d2b"));
    let socket_root = env::var_os("D2B_CUTOVER_SOCKET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/d2b"));
    let bootstrap_fd = env::var("D2B_CUTOVER_BOOTSTRAP_FD")
        .ok()
        .and_then(|value| value.parse::<i32>().ok())
        .unwrap_or(RUNNER_BOOTSTRAP_FD);
    let bootstrap_bytes = read_bootstrap_fd(bootstrap_fd)?;
    let mut ledger = CapabilityLedger::default();
    let (bootstrap, consumed) =
        RunnerBootstrap::decode_and_consume(&bootstrap_bytes, now_ms(), &mut ledger)
            .map_err(|_| RunnerError::Bootstrap)?;
    let paths = RunnerPaths::new_with_socket_root(state_root, socket_root, consumed.operation_id());
    if paths.journal.exists() {
        return Err(RunnerError::Replay);
    }
    let lock = acquire_operation_lock(&paths).map_err(|_| RunnerError::Lock)?;
    let operation = Operation::new(bootstrap.request.clone(), &bootstrap.preview)
        .map_err(|_| RunnerError::Request)?;
    persist_journal(
        &paths.journal,
        &bootstrap,
        &operation
            .journal_bytes()
            .map_err(|_| RunnerError::Journal)?,
    )
    .map_err(|_| RunnerError::Journal)?;

    let socket = RunnerSocket::bind(&paths, consumed).map_err(|_| RunnerError::Socket)?;
    let audit_sink = Box::new(BrokerAuditSink::new(
        &bootstrap,
        env::var_os("D2B_BROKER_SOCKET_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/run/d2b/priv.sock")),
    )?);
    let effect_sink = Box::new(BrokerEffectSink::new(
        &bootstrap,
        env::var_os("D2B_BROKER_SOCKET_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/run/d2b/priv.sock")),
    )?);
    let mut runtime = Runtime {
        bootstrap,
        operation,
        _lock: lock,
        paths,
        socket,
        audit_sink,
        effect_sink,
        _host_lock: HostLockContract::new(),
    };
    runtime.start_apply()?;
    runtime.serve()
}

fn read_bootstrap_fd(fd: i32) -> Result<Vec<u8>, RunnerError> {
    if fd < 3 {
        return Err(RunnerError::Bootstrap);
    }
    let flags = fcntl(fd, FcntlArg::F_GETFD).map_err(|_| RunnerError::Bootstrap)?;
    if !FdFlag::from_bits_truncate(flags).contains(FdFlag::FD_CLOEXEC) {
        return Err(RunnerError::Bootstrap);
    }
    let path = format!("/proc/self/fd/{fd}");
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC)
        .open(path)
        .map_err(|_| RunnerError::Bootstrap)?;
    let mut bytes = Vec::new();
    file.by_ref()
        .take((MAX_RUNNER_FRAME_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| RunnerError::Bootstrap)?;
    if bytes.len() > MAX_RUNNER_FRAME_BYTES {
        return Err(RunnerError::Bootstrap);
    }
    nix::unistd::close(fd).map_err(|_| RunnerError::Bootstrap)?;
    Ok(bytes)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

trait AuditSink {
    fn publish(
        &mut self,
        transition: CutoverAuditTransition,
        phase: u8,
        reason_digest: Option<d2b_cutover::Digest>,
    ) -> Result<AuditEvidence, AuditSinkError>;
}

trait EffectSink {
    fn execute(
        &mut self,
        request: &EffectRequest,
        phase: CutoverPhase,
        handoff: Option<d2b_contracts::host_generation::ApplyHostGenerationHandoff>,
        payload: Option<CutoverEffectPayload>,
    ) -> Result<CompletionEvidence, EffectSinkError>;
}

#[derive(Debug)]
enum AuditSinkError {
    Unavailable,
    Protocol,
}

#[derive(Debug)]
enum EffectSinkError {
    Unavailable,
    Protocol,
    NotAllowed,
}

struct BrokerAuditSink {
    socket_path: PathBuf,
    operation_id: BundleOpId,
    capability_digest: CanonicalAuditDigest,
    request_digest: CanonicalAuditDigest,
}

impl BrokerAuditSink {
    fn new(bootstrap: &RunnerBootstrap, socket_path: PathBuf) -> Result<Self, RunnerError> {
        let operation_id = BundleOpId::new(bootstrap.request.operation_id().as_str());
        let capability_digest =
            CanonicalAuditDigest::parse(bootstrap.capability.binding_digest().as_str().to_owned())
                .map_err(|_| RunnerError::Audit)?;
        let request_digest =
            CanonicalAuditDigest::parse(bootstrap.request.request_digest().as_str().to_owned())
                .map_err(|_| RunnerError::Audit)?;
        Ok(Self {
            socket_path,
            operation_id,
            capability_digest,
            request_digest,
        })
    }
}

impl AuditSink for BrokerAuditSink {
    fn publish(
        &mut self,
        transition: CutoverAuditTransition,
        phase: u8,
        reason_digest: Option<d2b_cutover::Digest>,
    ) -> Result<AuditEvidence, AuditSinkError> {
        let reason_digest = reason_digest
            .map(|digest| CanonicalAuditDigest::parse(digest.as_str().to_owned()))
            .transpose()
            .map_err(|_| AuditSinkError::Protocol)?;
        let request = BrokerRequest::CutoverAudit(CutoverAuditRequest {
            operation_id: self.operation_id.clone(),
            phase,
            transition,
            request_digest: self.request_digest.clone(),
            reason_digest,
        });
        let envelope = BrokerRequestEnvelope {
            request,
            caller_role: BrokerCallerRole::CutoverRunner {
                operation_id: self.operation_id.clone(),
                capability_digest: self.capability_digest.clone(),
            },
            test_peer_uid: None,
            audit_join: None,
        };
        let response = broker_round_trip(&self.socket_path, &envelope)?;
        let BrokerResponse::CutoverAudit(response) = response else {
            return Err(AuditSinkError::Protocol);
        };
        AuditEvidence::durable(response.record_id.as_str().to_owned())
            .map_err(|_| AuditSinkError::Protocol)
    }
}

struct BrokerEffectSink {
    socket_path: PathBuf,
    operation_id: BundleOpId,
    capability_digest: CanonicalAuditDigest,
    request_digest: CanonicalAuditDigest,
    authority: CutoverEffectAuthority,
}

impl BrokerEffectSink {
    fn new(bootstrap: &RunnerBootstrap, socket_path: PathBuf) -> Result<Self, RunnerError> {
        let operation_id = BundleOpId::new(bootstrap.request.operation_id().as_str());
        let capability_digest =
            CanonicalAuditDigest::parse(bootstrap.capability.binding_digest().as_str().to_owned())
                .map_err(|_| RunnerError::Audit)?;
        let request_digest =
            CanonicalAuditDigest::parse(bootstrap.request.request_digest().as_str().to_owned())
                .map_err(|_| RunnerError::Audit)?;
        let authority = match bootstrap.request.operation_kind() {
            OperationKind::Cutover => CutoverEffectAuthority::Cutover,
            OperationKind::ScopedReset(d2b_cutover::ResetScope::Zone) => {
                CutoverEffectAuthority::ResetZone
            }
            OperationKind::ScopedReset(d2b_cutover::ResetScope::Provider) => {
                CutoverEffectAuthority::ResetProvider
            }
            OperationKind::ScopedReset(d2b_cutover::ResetScope::Guest) => {
                CutoverEffectAuthority::ResetGuest
            }
        };
        Ok(Self {
            socket_path,
            operation_id,
            capability_digest,
            request_digest,
            authority,
        })
    }
}

impl EffectSink for BrokerEffectSink {
    fn execute(
        &mut self,
        request: &EffectRequest,
        phase: CutoverPhase,
        handoff: Option<d2b_contracts::host_generation::ApplyHostGenerationHandoff>,
        payload: Option<CutoverEffectPayload>,
    ) -> Result<CompletionEvidence, EffectSinkError> {
        let effect = cutover_effect_kind(request.kind()).ok_or(EffectSinkError::NotAllowed)?;
        if !self.authority.permits(effect) {
            return Err(EffectSinkError::NotAllowed);
        }
        if request.kind() == EffectKind::ClosureActivation && handoff.is_none() {
            return Err(EffectSinkError::Protocol);
        }
        let identity = request
            .journaled_identity()
            .map(|identity| BundleOpId::new(identity.as_str()));
        let response = broker_round_trip(
            &self.socket_path,
            &BrokerRequestEnvelope {
                request: BrokerRequest::CutoverEffect(CutoverEffectRequest {
                    operation_id: self.operation_id.clone(),
                    authority: self.authority,
                    phase: phase.number(),
                    effect_id: BundleOpId::new(request.effect_id().as_str()),
                    effect,
                    replay_class: match request.replay_class() {
                        ReplayClass::Repeatable => CutoverReplayClass::Repeatable,
                        ReplayClass::ReopenByJournaledIdentity => {
                            CutoverReplayClass::ReopenByJournaledIdentity
                        }
                        ReplayClass::QuarantineOnly => CutoverReplayClass::QuarantineOnly,
                    },
                    request_digest: self.request_digest.clone(),
                    capability_digest: self.capability_digest.clone(),
                    identity,
                    handoff,
                    payload,
                }),
                caller_role: BrokerCallerRole::CutoverRunner {
                    operation_id: self.operation_id.clone(),
                    capability_digest: self.capability_digest.clone(),
                },
                test_peer_uid: None,
                audit_join: None,
            },
        )
        .map_err(|_| EffectSinkError::Unavailable)?;
        let BrokerResponse::CutoverEffect(response) = response else {
            return Err(EffectSinkError::Protocol);
        };
        let effect = match response.outcome {
            CutoverEffectOutcome::Succeeded => match response.identity {
                Some(identity) => EffectEvidence::succeeded_with_identity(identity.as_str())
                    .map_err(|_| EffectSinkError::Protocol)?,
                None => EffectEvidence::succeeded(),
            },
            CutoverEffectOutcome::Failed => EffectEvidence::failed(),
            CutoverEffectOutcome::Ambiguous => EffectEvidence::ambiguous(),
        };
        let audit = AuditEvidence::durable(response.audit_record_id.as_str().to_owned())
            .map_err(|_| EffectSinkError::Protocol)?;
        Ok(CompletionEvidence { effect, audit })
    }
}

fn cutover_effect_kind(kind: EffectKind) -> Option<CutoverEffectKind> {
    Some(match kind {
        EffectKind::HostDrain => CutoverEffectKind::HostDrain,
        EffectKind::CutoverDisposition => CutoverEffectKind::CutoverDisposition,
        EffectKind::ResourceStoreCreate => CutoverEffectKind::ResourceStoreCreate,
        EffectKind::ProviderInstall => CutoverEffectKind::ProviderInstall,
        EffectKind::ZoneActivation => CutoverEffectKind::ZoneActivation,
        EffectKind::GuestActivation => CutoverEffectKind::GuestActivation,
        EffectKind::Verification => CutoverEffectKind::Verification,
        EffectKind::CutoverFinalization => CutoverEffectKind::CutoverFinalization,
        EffectKind::ScopedZoneReset => CutoverEffectKind::ScopedZoneReset,
        EffectKind::ScopedProviderReset => CutoverEffectKind::ScopedProviderReset,
        EffectKind::ScopedGuestReset => CutoverEffectKind::ScopedGuestReset,
        EffectKind::DestroyDurableVolume => CutoverEffectKind::DestroyDurableVolume,
        EffectKind::PreserveSource => CutoverEffectKind::PreserveSource,
        EffectKind::QuarantineDestination => CutoverEffectKind::QuarantineDestination,
        EffectKind::CutoverBroker => return None,
        EffectKind::ClosureActivation => CutoverEffectKind::ClosureActivation,
    })
}

fn broker_round_trip(
    socket_path: &std::path::Path,
    envelope: &BrokerRequestEnvelope,
) -> Result<BrokerResponse, AuditSinkError> {
    let fd = socket(
        AddressFamily::Unix,
        SockType::SeqPacket,
        SockFlag::SOCK_CLOEXEC,
        None,
    )
    .map_err(|_| AuditSinkError::Unavailable)?;
    let address = UnixAddr::new(socket_path).map_err(|_| AuditSinkError::Unavailable)?;
    connect(fd.as_raw_fd(), &address).map_err(|_| AuditSinkError::Unavailable)?;
    let body = serde_json::to_vec(envelope).map_err(|_| AuditSinkError::Protocol)?;
    if body.len() > MAX_FRAME_SIZE {
        return Err(AuditSinkError::Protocol);
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(&body);
    let sent =
        send(fd.as_raw_fd(), &frame, MsgFlags::empty()).map_err(|_| AuditSinkError::Unavailable)?;
    if sent != frame.len() {
        return Err(AuditSinkError::Unavailable);
    }
    let mut response = vec![0_u8; MAX_FRAME_SIZE + 4];
    let received = recv(fd.as_raw_fd(), &mut response, MsgFlags::empty())
        .map_err(|_| AuditSinkError::Unavailable)?;
    if received < 4 {
        return Err(AuditSinkError::Protocol);
    }
    let declared = u32::from_le_bytes(
        response[..4]
            .try_into()
            .map_err(|_| AuditSinkError::Protocol)?,
    ) as usize;
    if declared > MAX_FRAME_SIZE || declared != received - 4 {
        return Err(AuditSinkError::Protocol);
    }
    serde_json::from_slice(&response[4..received]).map_err(|_| AuditSinkError::Protocol)
}

struct Runtime {
    bootstrap: RunnerBootstrap,
    operation: Operation,
    _lock: std::fs::File,
    paths: RunnerPaths,
    socket: RunnerSocket,
    audit_sink: Box<dyn AuditSink>,
    effect_sink: Box<dyn EffectSink>,
    _host_lock: HostLockContract,
}

impl Runtime {
    fn start_apply(&mut self) -> Result<(), RunnerError> {
        let Some(mut consent) = self.bootstrap.consent.clone() else {
            return Ok(());
        };
        let mut lock = HostLockContract::new();
        self.operation
            .acquire_host_lock(&mut lock)
            .map_err(|_| RunnerError::Request)?;
        let context = match (
            self.bootstrap.recovery.clone(),
            self.bootstrap.host_digest.clone(),
        ) {
            (Some(recovery), Some(host_digest)) => ApplyContext::cutover(
                now_ms(),
                self.bootstrap.request.inventory_digest().clone(),
                true,
                true,
                true,
                recovery,
                host_digest,
            ),
            _ => ApplyContext::reset(
                now_ms(),
                self.bootstrap.request.inventory_digest().clone(),
                true,
                true,
                true,
            ),
        };
        if matches!(
            self.bootstrap.request.inventory(),
            OperationInventory::Reset(inventory) if inventory.allows_destroy_durable_volumes()
        ) {
            let Some(mut destructive_consent) = self.bootstrap.destructive_consent.clone() else {
                return Err(RunnerError::Request);
            };
            destructive_consent
                .consume(&self.bootstrap.request.consent_binding(), now_ms())
                .map_err(|_| RunnerError::Request)?;
            self.bootstrap.destructive_consent = Some(destructive_consent);
        }
        let audit = self
            .audit_sink
            .publish(
                CutoverAuditTransition::PhaseStarted,
                self.operation.phase().number(),
                None,
            )
            .map_err(|_| RunnerError::Audit)?;
        let previous = self.operation.clone();
        if self.operation.begin_apply(&mut consent, &context).is_err() {
            self.operation = previous.clone();
            return Err(RunnerError::Request);
        }
        if self.persist().is_err() {
            self.operation = previous;
            return Err(RunnerError::Journal);
        }
        let _ = audit;
        self._host_lock = lock;
        Ok(())
    }

    fn serve(&mut self) -> Result<(), RunnerError> {
        loop {
            let (mut stream, command, peer) = match self.socket.accept_command() {
                Ok(accepted) => accepted,
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::PermissionDenied
                            | std::io::ErrorKind::InvalidData
                            | std::io::ErrorKind::UnexpectedEof
                            | std::io::ErrorKind::ConnectionReset
                            | std::io::ErrorKind::BrokenPipe
                    ) =>
                {
                    // A disconnected or unauthorized client is scoped to one
                    // accepted stream; it must never terminate the runner.
                    continue;
                }
                Err(_) => return Err(RunnerError::Socket),
            };
            let response = self.handle(command, peer);
            if write_response(&mut stream, &response).is_err() {
                continue;
            }
            if response.error == Some(d2b_cutover::RunnerSocketError::JournalUnavailable) {
                return Err(RunnerError::Journal);
            }
            if self.operation.state().is_terminal() {
                return Ok(());
            }
        }
    }

    fn handle(&mut self, command: RunnerCommand, peer: RunnerPeer) -> RunnerResponse {
        match command {
            RunnerCommand::Status => RunnerResponse {
                accepted: true,
                status: Some(self.status()),
                error: None,
            },
            RunnerCommand::Apply { handoff } => self.apply_closure(handoff),
            RunnerCommand::Effect {
                effect_id,
                step_id,
                kind,
                replay_class,
                advance_to,
                identity,
                handoff,
                payload,
            } => {
                let effect = EffectRequest::new(effect_id, step_id, kind, replay_class, advance_to);
                let effect = match identity {
                    Some(identity) => effect.with_identity(Some(identity), None),
                    None => effect,
                };
                self.dispatch_effect(effect, handoff, payload)
            }
            RunnerCommand::Rollback { handoff } => {
                if self.operation.phase() == CutoverPhase::Disposition {
                    let Some(handoff) = handoff else {
                        return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                    };
                    let identity = match ArtifactId::new(handoff.intent.system_artifact_id.as_str())
                    {
                        Ok(identity) => identity,
                        Err(_) => {
                            return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                        }
                    };
                    let restore = EffectRequest::new(
                        d2b_cutover::EffectId::new("rollback-generation").expect("effect id"),
                        StepId::new("rollback-generation").expect("step id"),
                        EffectKind::ClosureActivation,
                        ReplayClass::ReopenByJournaledIdentity,
                        None,
                    )
                    .with_identity(Some(identity), None);
                    let response = self.dispatch_effect(restore, Some(handoff), None);
                    if !response.accepted {
                        return response;
                    }
                }
                let audit = match self.audit_sink.publish(
                    CutoverAuditTransition::Terminal,
                    self.operation.phase().number(),
                    None,
                ) {
                    Ok(audit) => audit,
                    Err(_) => {
                        return self.failure(d2b_cutover::RunnerSocketError::AuditUnavailable);
                    }
                };
                let previous = self.operation.clone();
                if self.operation.rollback(audit).is_err() {
                    self.operation = previous;
                    return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                }
                if self.persist().is_err() {
                    self.operation = previous;
                    return self.failure(d2b_cutover::RunnerSocketError::JournalUnavailable);
                }
                RunnerResponse {
                    accepted: true,
                    status: Some(self.status()),
                    error: None,
                }
            }
            RunnerCommand::Verify { observations } => {
                let audit = match self.audit_sink.publish(
                    CutoverAuditTransition::PhaseCompleted,
                    self.operation.phase().number(),
                    None,
                ) {
                    Ok(audit) => audit,
                    Err(_) => {
                        return self.failure(d2b_cutover::RunnerSocketError::AuditUnavailable);
                    }
                };
                let input = d2b_cutover::VerificationInput::new(
                    observations
                        .zones
                        .into_iter()
                        .map(|zone| d2b_cutover::ZoneVerification::new(zone.zone_id, zone.healthy)),
                    observations.sources_preserved,
                    observations.identity_digests_match,
                    audit.is_durable(),
                    observations.candidate_current,
                );
                let previous = self.operation.clone();
                if self.operation.verify(&input).is_err() {
                    self.operation = previous;
                    return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                }
                if self.persist().is_err() {
                    self.operation = previous;
                    return self.failure(d2b_cutover::RunnerSocketError::JournalUnavailable);
                }
                RunnerResponse {
                    accepted: true,
                    status: Some(self.status()),
                    error: None,
                }
            }
            RunnerCommand::Finalize { mut consent, plan } => {
                let consent_digest = match consent.digest() {
                    Ok(digest) => digest,
                    Err(_) => {
                        return self.failure(d2b_cutover::RunnerSocketError::Malformed);
                    }
                };
                let audit = match self.audit_sink.publish(
                    CutoverAuditTransition::PhaseStarted,
                    self.operation.phase().number(),
                    None,
                ) {
                    Ok(audit) => audit,
                    Err(_) => {
                        return self.failure(d2b_cutover::RunnerSocketError::AuditUnavailable);
                    }
                };
                let previous = self.operation.clone();
                if self
                    .operation
                    .begin_finalization(&mut consent, now_ms())
                    .is_err()
                {
                    self.operation = previous;
                    return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                }
                if self.persist().is_err() {
                    self.operation = previous;
                    return self.failure(d2b_cutover::RunnerSocketError::JournalUnavailable);
                }
                let artifacts = match plan
                    .artifacts
                    .iter()
                    .map(|artifact| {
                        d2b_contracts::v3::ArtifactId::parse(artifact.artifact_id.as_str())
                            .map_err(|_| d2b_cutover::RunnerSocketError::Malformed)
                    })
                    .collect::<Result<Vec<_>, _>>()
                {
                    Ok(artifacts) => artifacts,
                    Err(error) => return self.failure(error),
                };
                let disposition_digest = d2b_cutover::Digest::derive(
                    "d2b:cutover:finalization-dispositions:v1",
                    &serde_json::to_vec(&plan).expect("finalization plan serializes"),
                );
                let effect = EffectRequest::new(
                    d2b_cutover::EffectId::new("phase-10-finalization").expect("effect id"),
                    StepId::new("phase-10-finalization").expect("step id"),
                    EffectKind::CutoverFinalization,
                    ReplayClass::QuarantineOnly,
                    None,
                );
                let response = self.dispatch_effect(
                    effect,
                    None,
                    Some(CutoverEffectPayload::Finalization {
                        artifacts,
                        disposition_digest: CanonicalAuditDigest::parse(
                            disposition_digest.as_str().to_owned(),
                        )
                        .expect("canonical disposition digest"),
                        consent_digest: CanonicalAuditDigest::parse(
                            consent_digest.as_str().to_owned(),
                        )
                        .expect("canonical consent digest"),
                    }),
                );
                let _ = audit;
                response
            }
            RunnerCommand::Hold { reason } => {
                let reason = match HoldReason::new(reason) {
                    Ok(reason) => reason,
                    Err(_) => {
                        return RunnerResponse {
                            accepted: false,
                            status: Some(self.status()),
                            error: Some(d2b_cutover::RunnerSocketError::Malformed),
                        };
                    }
                };
                let requested_by = match peer {
                    RunnerPeer::Owner => self.bootstrap.request.operator_id().clone(),
                    RunnerPeer::Admin => d2b_cutover::OperatorId::new("admin-peer")
                        .expect("static operator identity"),
                };
                // The reason digest is request metadata; only the broker's
                // durable record id is accepted as audit evidence below.
                let reason_digest = d2b_cutover::Digest::derive(
                    "d2b:cutover:hold-reason:v1",
                    reason.as_str().as_bytes(),
                );
                let audit = match self.audit_sink.publish(
                    CutoverAuditTransition::HoldRequested,
                    self.operation.phase().number(),
                    Some(reason_digest),
                ) {
                    Ok(audit) => audit,
                    Err(_) => {
                        return self.failure(d2b_cutover::RunnerSocketError::AuditUnavailable);
                    }
                };
                let previous = self.operation.clone();
                match self.operation.request_hold(requested_by, reason, audit) {
                    Ok(()) => {}
                    Err(d2b_cutover::OperationError::AuditNotDurable) => {
                        return self.failure(d2b_cutover::RunnerSocketError::AuditUnavailable);
                    }
                    Err(_) => {
                        return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                    }
                }

                if self.persist().is_err() {
                    self.operation = previous;
                    return self.failure(d2b_cutover::RunnerSocketError::JournalUnavailable);
                }
                RunnerResponse {
                    accepted: true,
                    status: Some(self.status()),
                    error: None,
                }
            }
            RunnerCommand::Resume { fresh_consent } => {
                if matches!(peer, RunnerPeer::Admin) {
                    let expected = self
                        .bootstrap
                        .consent
                        .as_ref()
                        .and_then(|consent| consent.digest().ok())
                        .unwrap_or_else(|| {
                            d2b_cutover::Digest::derive(
                                "d2b:cutover:resume-consent",
                                self.bootstrap.request.request_digest().as_str().as_bytes(),
                            )
                        });
                    if fresh_consent.as_ref() != Some(&expected) {
                        return self.failure(d2b_cutover::RunnerSocketError::OperatorMismatch);
                    }
                }
                let context = match self.bootstrap.request.operation_kind() {
                    OperationKind::ScopedReset(_) => ApplyContext::reset(
                        now_ms(),
                        self.bootstrap.request.inventory_digest().clone(),
                        true,
                        true,
                        true,
                    ),
                    OperationKind::Cutover => {
                        let (Some(recovery), Some(host_digest)) = (
                            self.bootstrap.recovery.clone(),
                            self.bootstrap.host_digest.clone(),
                        ) else {
                            return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                        };
                        ApplyContext::cutover(
                            now_ms(),
                            self.bootstrap.request.inventory_digest().clone(),
                            true,
                            true,
                            true,
                            recovery,
                            host_digest,
                        )
                    }
                };
                // Resume has the same durability requirement as hold.
                let audit = match self.audit_sink.publish(
                    CutoverAuditTransition::HoldCleared,
                    self.operation.phase().number(),
                    None,
                ) {
                    Ok(audit) => audit,
                    Err(_) => {
                        return self.failure(d2b_cutover::RunnerSocketError::AuditUnavailable);
                    }
                };
                let previous = self.operation.clone();
                match self
                    .operation
                    .resume(self.bootstrap.request.operator_id(), &context, audit)
                {
                    Ok(()) => {}
                    Err(d2b_cutover::OperationError::AuditNotDurable) => {
                        return self.failure(d2b_cutover::RunnerSocketError::AuditUnavailable);
                    }
                    Err(_) => {
                        return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                    }
                }
                if self.persist().is_err() {
                    self.operation = previous;
                    return self.failure(d2b_cutover::RunnerSocketError::JournalUnavailable);
                }
                RunnerResponse {
                    accepted: true,
                    status: Some(self.status()),
                    error: None,
                }
            }
        }
    }

    fn apply_closure(
        &mut self,
        handoff: d2b_contracts::host_generation::ApplyHostGenerationHandoff,
    ) -> RunnerResponse {
        if let Err(error) = self.complete_read_only_prefix() {
            return self.failure(error);
        }
        if self.operation.phase() == CutoverPhase::Drain {
            let drain = EffectRequest::new(
                d2b_cutover::EffectId::new("host-drain").expect("effect id"),
                StepId::new("phase-3-host-drain").expect("step id"),
                EffectKind::HostDrain,
                ReplayClass::Repeatable,
                Some(CutoverPhase::Disposition),
            );
            let response = self.dispatch_effect(drain, None, None);
            if !response.accepted {
                return response;
            }
        }
        if self.operation.phase() != CutoverPhase::Disposition {
            return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
        }
        let identity = match ArtifactId::new(handoff.intent.system_artifact_id.as_str()) {
            Ok(identity) => identity,
            Err(_) => return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition),
        };
        let effect = EffectRequest::new(
            d2b_cutover::EffectId::new("closure-activation").expect("effect id"),
            StepId::new("phase-closure-activation").expect("step id"),
            EffectKind::ClosureActivation,
            ReplayClass::ReopenByJournaledIdentity,
            Some(CutoverPhase::ResourceStore),
        )
        .with_identity(Some(identity), None);
        self.dispatch_effect(effect, Some(handoff), None)
    }

    fn complete_read_only_prefix(&mut self) -> Result<(), d2b_cutover::RunnerSocketError> {
        while self.operation.phase().number() <= CutoverPhase::Inventory.number() {
            let phase = self.operation.phase();
            let audit = self
                .audit_sink
                .publish(CutoverAuditTransition::PhaseCompleted, phase.number(), None)
                .map_err(|_| d2b_cutover::RunnerSocketError::AuditUnavailable)?;
            self.operation
                .complete_read_only_phase(
                    phase,
                    ReadOnlyEvidence {
                        predicates_hold: true,
                        audit,
                    },
                )
                .map_err(|_| d2b_cutover::RunnerSocketError::InvalidTransition)?;
            self.persist()
                .map_err(|_| d2b_cutover::RunnerSocketError::JournalUnavailable)?;
        }
        Ok(())
    }

    fn effect_phase_allowed(kind: EffectKind, phase: CutoverPhase) -> bool {
        match kind {
            EffectKind::HostDrain => phase == CutoverPhase::Drain,
            EffectKind::CutoverDisposition
            | EffectKind::ClosureActivation
            | EffectKind::PreserveSource
            | EffectKind::QuarantineDestination
            | EffectKind::ScopedZoneReset
            | EffectKind::ScopedProviderReset
            | EffectKind::ScopedGuestReset
            | EffectKind::DestroyDurableVolume => phase == CutoverPhase::Disposition,
            EffectKind::ResourceStoreCreate => phase == CutoverPhase::ResourceStore,
            EffectKind::ProviderInstall => phase == CutoverPhase::ProviderInstall,
            EffectKind::ZoneActivation => phase == CutoverPhase::ZoneCutover,
            EffectKind::GuestActivation => phase == CutoverPhase::Activation,
            EffectKind::Verification => phase == CutoverPhase::Verification,
            EffectKind::CutoverFinalization => phase == CutoverPhase::Finalization,
            EffectKind::CutoverBroker => false,
        }
    }

    fn dispatch_effect(
        &mut self,
        effect: EffectRequest,
        handoff: Option<d2b_contracts::host_generation::ApplyHostGenerationHandoff>,
        payload: Option<CutoverEffectPayload>,
    ) -> RunnerResponse {
        if effect.kind() == EffectKind::DestroyDurableVolume {
            let valid = match (
                self.bootstrap.destructive_consent.as_ref(),
                payload.as_ref(),
            ) {
                (
                    Some(consent),
                    Some(CutoverEffectPayload::DestroyDurableVolume { consent_digest, .. }),
                ) => consent
                    .digest()
                    .ok()
                    .is_some_and(|digest| digest.as_str() == consent_digest.as_str()),
                _ => false,
            };
            if !valid {
                return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
            }
        }
        if !Self::effect_phase_allowed(effect.kind(), self.operation.phase()) {
            return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
        }
        let previous = self.operation.clone();
        if self.operation.start_effect(effect.clone()).is_err() {
            return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
        }
        // The started record is the replay boundary. It must reach the
        // root-owned journal before the broker is allowed to mutate anything.
        if self.persist().is_err() {
            self.operation = previous;
            return self.failure(d2b_cutover::RunnerSocketError::JournalUnavailable);
        }
        if self
            .audit_sink
            .publish(
                CutoverAuditTransition::EffectStarted,
                self.operation.phase().number(),
                None,
            )
            .is_err()
        {
            return self.failure(d2b_cutover::RunnerSocketError::AuditUnavailable);
        }
        let evidence =
            match self
                .effect_sink
                .execute(&effect, self.operation.phase(), handoff, payload)
            {
                Ok(evidence) => evidence,
                Err(EffectSinkError::Unavailable)
                | Err(EffectSinkError::Protocol)
                | Err(EffectSinkError::NotAllowed) => {
                    if !self
                        .operation
                        .phase()
                        .is_before_or_at_native_rollback_boundary()
                    {
                        return self.require_external_restore();
                    }
                    return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
                }
            };
        if self
            .operation
            .complete_effect(effect.effect_id(), evidence)
            .is_err()
        {
            return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
        }
        if self.persist().is_err() {
            return self.failure(d2b_cutover::RunnerSocketError::JournalUnavailable);
        }
        RunnerResponse {
            accepted: true,
            status: Some(self.status()),
            error: None,
        }
    }

    fn require_external_restore(&mut self) -> RunnerResponse {
        let audit = match self.audit_sink.publish(
            CutoverAuditTransition::Terminal,
            self.operation.phase().number(),
            None,
        ) {
            Ok(audit) => audit,
            Err(_) => return self.failure(d2b_cutover::RunnerSocketError::AuditUnavailable),
        };
        let previous = self.operation.clone();
        if self.operation.require_external_restore(audit).is_err() {
            self.operation = previous;
            return self.failure(d2b_cutover::RunnerSocketError::InvalidTransition);
        }
        if self.persist().is_err() {
            self.operation = previous;
            return self.failure(d2b_cutover::RunnerSocketError::JournalUnavailable);
        }
        RunnerResponse {
            accepted: false,
            status: Some(self.status()),
            error: Some(d2b_cutover::RunnerSocketError::InvalidTransition),
        }
    }

    fn persist(&self) -> Result<(), RunnerError> {
        persist_journal(
            &self.paths.journal,
            &self.bootstrap,
            &self
                .operation
                .journal_bytes()
                .map_err(|_| RunnerError::Journal)?,
        )
        .map_err(|_| RunnerError::Journal)
    }

    fn status(&self) -> RunnerStatus {
        RunnerStatus {
            operation_id: self.bootstrap.request.operation_id().clone(),
            state: self.operation.state(),
            phase: self.operation.phase(),
            hold_active: matches!(self.operation.state(), d2b_cutover::OperationState::Held),
            terminal: self.operation.state().is_terminal(),
        }
    }

    fn failure(&self, error: d2b_cutover::RunnerSocketError) -> RunnerResponse {
        RunnerResponse {
            accepted: false,
            status: Some(self.status()),
            error: Some(error),
        }
    }
}

#[derive(Debug)]
enum RunnerError {
    NotRoot,
    Session,
    Bootstrap,
    Replay,
    Lock,
    Request,
    Audit,
    Journal,
    Socket,
}

impl std::fmt::Display for RunnerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::NotRoot => "runner must start as uid 0",
            Self::Session => "runner could not create a new session",
            Self::Bootstrap => "bootstrap capability was rejected",
            Self::Replay => "bootstrap capability or journal was already consumed",
            Self::Lock => "operation lock was unavailable",
            Self::Request => "operation request was rejected",
            Self::Audit => "cutover audit sink unavailable",
            Self::Journal => "journal durability failed",
            Self::Socket => "runner socket failed",
        })
    }
}

impl std::error::Error for RunnerError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;
    use std::sync::{Arc, Mutex};

    use d2b_cutover::{
        CandidateId, CutoverPreview, FinalizationArtifact, FinalizationPlan, HoldReason,
        HostInventory, HostLockContract, OperationRequest, OperationState, RecoveryAttestation,
        RecoveryId, ResetInventory, ResetScope, RevisionPlanId, RunnerSocketError,
        RunnerVerificationInput, RunnerZoneVerification, ZoneId, ZoneInventory,
    };

    struct UnavailableAuditSink;

    impl AuditSink for UnavailableAuditSink {
        fn publish(
            &mut self,
            _transition: CutoverAuditTransition,
            _phase: u8,
            _reason_digest: Option<d2b_cutover::Digest>,
        ) -> Result<AuditEvidence, AuditSinkError> {
            Err(AuditSinkError::Unavailable)
        }
    }

    struct UnavailableEffectSink;

    impl EffectSink for UnavailableEffectSink {
        fn execute(
            &mut self,
            _request: &EffectRequest,
            _phase: CutoverPhase,
            _handoff: Option<d2b_contracts::host_generation::ApplyHostGenerationHandoff>,
            _payload: Option<CutoverEffectPayload>,
        ) -> Result<CompletionEvidence, EffectSinkError> {
            Err(EffectSinkError::Unavailable)
        }
    }

    struct ScriptedEffectSink {
        kinds: Arc<Mutex<Vec<EffectKind>>>,
    }

    impl EffectSink for ScriptedEffectSink {
        fn execute(
            &mut self,
            request: &EffectRequest,
            _phase: CutoverPhase,
            _handoff: Option<d2b_contracts::host_generation::ApplyHostGenerationHandoff>,
            _payload: Option<CutoverEffectPayload>,
        ) -> Result<CompletionEvidence, EffectSinkError> {
            self.kinds
                .lock()
                .expect("effect call lock")
                .push(request.kind());
            let effect = if request.identity_bearing() {
                EffectEvidence::succeeded_with_identity(
                    request
                        .journaled_identity()
                        .expect("identity-bearing effect identity")
                        .as_str(),
                )
                .map_err(|_| EffectSinkError::Protocol)?
            } else {
                EffectEvidence::succeeded()
            };
            Ok(CompletionEvidence {
                effect,
                audit: AuditEvidence::durable("scripted-effect-audit")
                    .map_err(|_| EffectSinkError::Protocol)?,
            })
        }
    }

    fn runtime(label: &str) -> Runtime {
        runtime_with_audit(label, Box::new(UnavailableAuditSink))
    }

    fn runtime_with_audit(label: &str, audit_sink: Box<dyn AuditSink>) -> Runtime {
        std::fs::create_dir_all(".scratch").expect("create test scratch root");
        let operation_id = d2b_cutover::OperationId::new("op-audit-boundary").expect("operation");
        let candidate_id = CandidateId::new("candidate-audit-boundary").expect("candidate");
        let revision_plan_id =
            d2b_cutover::RevisionPlanId::new("plan-audit-boundary").expect("plan");
        let operator_id =
            d2b_cutover::OperatorId::new("operator-audit-boundary").expect("operator");
        let inventory =
            ResetInventory::new(ResetScope::Zone, "zone-audit-boundary").expect("inventory");
        let preview = CutoverPreview::new_reset(
            operation_id.clone(),
            OperationKind::ScopedReset(ResetScope::Zone),
            candidate_id.clone(),
            revision_plan_id.clone(),
            inventory.clone(),
        )
        .expect("preview");
        let request = OperationRequest::new_reset(
            operation_id.clone(),
            ResetScope::Zone,
            candidate_id,
            revision_plan_id,
            operator_id.clone(),
            preview.digest().expect("preview digest"),
            inventory,
        )
        .expect("request");
        let capability = d2b_cutover::BootstrapCapability::new_with_identity(
            operation_id.clone(),
            request.candidate_id().clone(),
            operator_id,
            OperationKind::ScopedReset(ResetScope::Zone),
            d2b_cutover::Digest::derive("d2b:test:runner-audit", b"nonce"),
            100,
            200,
            1000,
            BTreeSet::new(),
        )
        .expect("capability");
        let bootstrap = RunnerBootstrap {
            capability,
            request: request.clone(),
            preview,
            consent: None,
            destructive_consent: None,
            recovery: None,
            host_digest: None,
        };
        let mut ledger = CapabilityLedger::default();
        let capability_bytes = bootstrap
            .capability
            .canonical_bytes()
            .expect("capability bytes");
        let consumed = d2b_cutover::BootstrapCapability::decode_and_consume(
            &capability_bytes,
            150,
            &mut ledger,
        )
        .expect("consume capability");
        let root =
            PathBuf::from(".scratch").join(format!("runner-audit-{label}-{}", std::process::id()));
        let paths = RunnerPaths::new(&root, &operation_id);
        let lock = acquire_operation_lock(&paths).expect("lock");
        let socket = RunnerSocket::bind(&paths, consumed).expect("socket");
        let operation = Operation::new(request, &bootstrap.preview).expect("engine");
        persist_journal(
            &paths.journal,
            &bootstrap,
            &operation.journal_bytes().expect("initial journal"),
        )
        .expect("persist initial journal");
        Runtime {
            bootstrap,
            operation,
            _lock: lock,
            paths,
            socket,
            audit_sink,
            effect_sink: Box::new(UnavailableEffectSink),
            _host_lock: HostLockContract::new(),
        }
    }

    fn cutover_runtime_for_apply(
        label: &str,
    ) -> (
        Runtime,
        d2b_contracts::host_generation::ApplyHostGenerationHandoff,
        Arc<Mutex<Vec<EffectKind>>>,
    ) {
        std::fs::create_dir_all(".scratch").expect("create test scratch root");
        let operation_id =
            d2b_cutover::OperationId::new(format!("op-apply-sequence-{label}")).expect("operation");
        let candidate_id =
            CandidateId::new(format!("candidate-apply-sequence-{label}")).expect("candidate");
        let revision_plan_id =
            RevisionPlanId::new(format!("plan-apply-sequence-{label}")).expect("plan");
        let operator_id = d2b_cutover::OperatorId::new(format!("operator-apply-sequence-{label}"))
            .expect("operator");
        let inventory = HostInventory::build(
            [ZoneId::new("zone-apply").expect("zone")],
            [ZoneInventory::new("zone-apply", true, []).expect("zone inventory")],
            [],
        )
        .expect("inventory");
        let preview = CutoverPreview::new(
            operation_id.clone(),
            OperationKind::Cutover,
            candidate_id.clone(),
            revision_plan_id.clone(),
            inventory.clone(),
            None,
        )
        .expect("preview");
        let preview_digest = preview.digest().expect("preview digest");
        let recovery_now = now_ms();
        let recovery = RecoveryAttestation::new(
            RecoveryId::new(format!("recovery-apply-sequence-{label}")).expect("recovery"),
            candidate_id.clone(),
            d2b_cutover::Digest::derive("d2b:test:host", b"host"),
            preview_digest.clone(),
            operator_id.clone(),
            d2b_cutover::Digest::derive("d2b:test:restore", b"restore"),
            recovery_now.saturating_sub(1_000),
            recovery_now.saturating_add(600_000),
            true,
        )
        .expect("recovery");
        let recovery_digest = recovery.digest().expect("recovery digest");
        let request = OperationRequest::new_cutover(
            operation_id.clone(),
            candidate_id.clone(),
            revision_plan_id,
            operator_id.clone(),
            preview_digest,
            recovery_digest,
            inventory.clone(),
        )
        .expect("request");
        let capability = d2b_cutover::BootstrapCapability::new_with_identity(
            operation_id.clone(),
            candidate_id,
            operator_id.clone(),
            OperationKind::Cutover,
            d2b_cutover::Digest::derive("d2b:test:apply-sequence", label.as_bytes()),
            100,
            200,
            nix::unistd::geteuid().as_raw(),
            BTreeSet::new(),
        )
        .expect("capability");
        let mut bootstrap = RunnerBootstrap {
            capability,
            request: request.clone(),
            preview,
            consent: Some(
                d2b_cutover::Consent::issue(
                    request.consent_binding(),
                    recovery_now.saturating_sub(1_000),
                    recovery_now.saturating_add(600_000),
                )
                .expect("consent"),
            ),
            destructive_consent: None,
            recovery: Some(recovery.clone()),
            host_digest: Some(d2b_cutover::Digest::derive("d2b:test:host", b"host")),
        };
        let capability_bytes = bootstrap
            .capability
            .canonical_bytes()
            .expect("capability bytes");
        let mut ledger = CapabilityLedger::default();
        let consumed = d2b_cutover::BootstrapCapability::decode_and_consume(
            &capability_bytes,
            150,
            &mut ledger,
        )
        .expect("consume capability");
        let root = PathBuf::from(".scratch").join(format!(
            "runner-apply-sequence-{label}-{}",
            std::process::id()
        ));
        let paths = RunnerPaths::new(&root, &operation_id);
        let lock = acquire_operation_lock(&paths).expect("lock");
        let socket = RunnerSocket::bind(&paths, consumed).expect("socket");
        let mut operation = Operation::new(request, &bootstrap.preview).expect("engine");
        let mut host_lock = HostLockContract::new();
        operation
            .acquire_host_lock(&mut host_lock)
            .expect("host lock");
        operation
            .begin_apply(
                bootstrap.consent.as_mut().expect("consent"),
                &ApplyContext::cutover(
                    recovery_now,
                    inventory.digest().expect("inventory digest"),
                    true,
                    true,
                    true,
                    recovery,
                    d2b_cutover::Digest::derive("d2b:test:host", b"host"),
                ),
            )
            .expect("begin apply");
        persist_journal(
            &paths.journal,
            &bootstrap,
            &operation.journal_bytes().expect("journal"),
        )
        .expect("persist");
        let handoff = {
            let target = d2b_contracts::v3::ResourceRef::parse("Host/host-system").expect("target");
            let artifact = d2b_contracts::v3::ArtifactId::parse("host-system").expect("artifact");
            let fingerprint =
                d2b_contracts::host_generation::target_fingerprint(&target, &artifact, 8);
            d2b_contracts::host_generation::ApplyHostGenerationHandoff {
                caller_role: d2b_contracts::host_generation::HandoffCallerRole::Lifecycle,
                target,
                intent: d2b_contracts::host_generation::HostGenerationHandoffIntent {
                    source_generation: 7,
                    target_generation: 8,
                    system_artifact_id: artifact,
                    activation_mode: d2b_contracts::v3::ActivationMode::Switch,
                    compatibility:
                        d2b_contracts::host_generation::SourceGenerationCompatibilityFloorV1::new(
                            7,
                            fingerprint,
                        )
                        .expect("compatibility"),
                },
            }
        };
        let kinds = Arc::new(Mutex::new(Vec::new()));
        (
            Runtime {
                bootstrap,
                operation,
                _lock: lock,
                paths,
                socket,
                audit_sink: Box::new(FixedAuditSink),
                effect_sink: Box::new(ScriptedEffectSink {
                    kinds: kinds.clone(),
                }),
                _host_lock: host_lock,
            },
            handoff,
            kinds,
        )
    }

    #[test]
    fn apply_command_runs_host_drain_before_closure_activation() {
        let (mut runtime, handoff, kinds) = cutover_runtime_for_apply("apply");
        let response = runtime.handle(RunnerCommand::Apply { handoff }, RunnerPeer::Owner);
        assert!(response.accepted, "apply response: {:?}", response.error);
        assert_eq!(runtime.operation.phase(), CutoverPhase::ResourceStore);
        assert_eq!(
            *kinds.lock().expect("effect calls"),
            [EffectKind::HostDrain, EffectKind::ClosureActivation]
        );
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn rollback_after_host_drain_restores_the_bound_generation_before_closing() {
        let (mut runtime, handoff, kinds) = cutover_runtime_for_apply("rollback");
        runtime
            .complete_read_only_prefix()
            .expect("read-only prefix");
        let drain = EffectRequest::new(
            d2b_cutover::EffectId::new("host-drain").expect("effect id"),
            StepId::new("phase-3-host-drain").expect("step id"),
            EffectKind::HostDrain,
            ReplayClass::Repeatable,
            Some(CutoverPhase::Disposition),
        );
        let response = runtime.dispatch_effect(drain, None, None);
        assert!(response.accepted);
        let response = runtime.handle(
            RunnerCommand::Rollback {
                handoff: Some(handoff),
            },
            RunnerPeer::Owner,
        );
        assert!(response.accepted, "rollback response: {:?}", response.error);
        assert_eq!(runtime.operation.state(), OperationState::RolledBack);
        assert_eq!(
            *kinds.lock().expect("effect calls"),
            [EffectKind::HostDrain, EffectKind::ClosureActivation]
        );
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn effect_dispatch_rejects_out_of_order_phase_requests() {
        let (mut runtime, _handoff, kinds) = cutover_runtime_for_apply("phase-order");
        runtime
            .complete_read_only_prefix()
            .expect("read-only prefix");
        let response = runtime.handle(
            RunnerCommand::Effect {
                effect_id: d2b_cutover::EffectId::new("provider-install-too-early")
                    .expect("effect id"),
                step_id: StepId::new("provider-install-too-early").expect("step id"),
                kind: EffectKind::ProviderInstall,
                replay_class: ReplayClass::Repeatable,
                advance_to: Some(CutoverPhase::ZoneCutover),
                identity: None,
                handoff: None,
                payload: None,
            },
            RunnerPeer::Owner,
        );
        assert_eq!(
            response.error,
            Some(d2b_cutover::RunnerSocketError::InvalidTransition)
        );
        assert_eq!(runtime.operation.phase(), CutoverPhase::Drain);
        assert!(kinds.lock().expect("effect calls").is_empty());
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn typed_effects_progress_the_remaining_u4_phases_in_order() {
        let (mut runtime, handoff, _kinds) = cutover_runtime_for_apply("phase-chain");
        let response = runtime.handle(RunnerCommand::Apply { handoff }, RunnerPeer::Owner);
        assert!(response.accepted);
        for (number, kind, advance_to) in [
            (
                "resource-store",
                EffectKind::ResourceStoreCreate,
                CutoverPhase::ProviderInstall,
            ),
            (
                "provider",
                EffectKind::ProviderInstall,
                CutoverPhase::ZoneCutover,
            ),
            ("zone", EffectKind::ZoneActivation, CutoverPhase::Activation),
            (
                "guest",
                EffectKind::GuestActivation,
                CutoverPhase::Verification,
            ),
            (
                "verify",
                EffectKind::Verification,
                CutoverPhase::Finalization,
            ),
        ] {
            let response = runtime.handle(
                RunnerCommand::Effect {
                    effect_id: d2b_cutover::EffectId::new(format!("effect-{number}"))
                        .expect("effect id"),
                    step_id: StepId::new(format!("step-{number}")).expect("step id"),
                    kind,
                    replay_class: ReplayClass::Repeatable,
                    advance_to: Some(advance_to),
                    identity: None,
                    handoff: None,
                    payload: None,
                },
                RunnerPeer::Owner,
            );
            assert!(response.accepted, "{number}: {:?}", response.error);
            assert_eq!(runtime.operation.phase(), advance_to);
        }
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn phase_five_effect_failure_publishes_external_restore_required() {
        let (mut runtime, handoff, _kinds) = cutover_runtime_for_apply("r5");
        let response = runtime.handle(RunnerCommand::Apply { handoff }, RunnerPeer::Owner);
        assert!(response.accepted);
        runtime.effect_sink = Box::new(UnavailableEffectSink);

        let response = runtime.handle(
            RunnerCommand::Effect {
                effect_id: d2b_cutover::EffectId::new("phase-five-failure").expect("effect id"),
                step_id: StepId::new("phase-five-failure").expect("step id"),
                kind: EffectKind::ResourceStoreCreate,
                replay_class: ReplayClass::ReopenByJournaledIdentity,
                advance_to: Some(CutoverPhase::ProviderInstall),
                identity: Some(ArtifactId::new("store-identity").expect("identity")),
                handoff: None,
                payload: None,
            },
            RunnerPeer::Owner,
        );
        assert!(!response.accepted);
        assert_eq!(
            response.error,
            Some(d2b_cutover::RunnerSocketError::InvalidTransition)
        );
        assert_eq!(runtime.operation.state(), OperationState::RestoreRequired);
        let (_, records) = d2b_cutover::load_journal(&runtime.paths.journal).expect("journal");
        assert!(String::from_utf8_lossy(&records).contains("\"restore-required\""));
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn finalization_requires_separate_consent_and_closes_after_audited_effect() {
        let (mut runtime, handoff, _kinds) = cutover_runtime_for_apply("finalization");
        let response = runtime.handle(RunnerCommand::Apply { handoff }, RunnerPeer::Owner);
        assert!(response.accepted);
        for (number, kind, advance_to) in [
            (
                "resource-store-finalization",
                EffectKind::ResourceStoreCreate,
                CutoverPhase::ProviderInstall,
            ),
            (
                "provider-finalization",
                EffectKind::ProviderInstall,
                CutoverPhase::ZoneCutover,
            ),
            (
                "zone-finalization",
                EffectKind::ZoneActivation,
                CutoverPhase::Activation,
            ),
            (
                "guest-finalization",
                EffectKind::GuestActivation,
                CutoverPhase::Verification,
            ),
        ] {
            let response = runtime.handle(
                RunnerCommand::Effect {
                    effect_id: d2b_cutover::EffectId::new(format!("effect-{number}"))
                        .expect("effect id"),
                    step_id: StepId::new(format!("step-{number}")).expect("step id"),
                    kind,
                    replay_class: ReplayClass::Repeatable,
                    advance_to: Some(advance_to),
                    identity: None,
                    handoff: None,
                    payload: None,
                },
                RunnerPeer::Owner,
            );
            assert!(response.accepted, "{number}: {:?}", response.error);
        }
        let response = runtime.handle(
            RunnerCommand::Verify {
                observations: RunnerVerificationInput {
                    zones: vec![RunnerZoneVerification {
                        zone_id: ZoneId::new("zone-apply").expect("zone"),
                        healthy: true,
                    }],
                    sources_preserved: true,
                    identity_digests_match: true,
                    candidate_current: true,
                },
            },
            RunnerPeer::Owner,
        );
        assert!(response.accepted, "verify: {:?}", response.error);
        assert_eq!(runtime.operation.state(), OperationState::CutoverSucceeded);
        let consent = d2b_cutover::FinalizationConsent::issue(
            runtime.bootstrap.request.finalization_binding(),
            now_ms(),
            now_ms().saturating_add(10_000),
        )
        .expect("finalization consent");
        let plan = FinalizationPlan {
            artifacts: vec![FinalizationArtifact {
                artifact_id: d2b_cutover::ArtifactId::new("legacy-one").expect("artifact"),
                disposition_digest: d2b_cutover::Digest::derive(
                    "d2b:test:disposition",
                    b"legacy-one",
                ),
            }],
        };
        let response = runtime.handle(RunnerCommand::Finalize { consent, plan }, RunnerPeer::Owner);
        assert!(response.accepted, "finalization: {:?}", response.error);
        assert_eq!(runtime.operation.state(), OperationState::Closed);
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn finalization_audit_failure_does_not_advance() {
        let (mut runtime, handoff, _kinds) = cutover_runtime_for_apply("fin-audit");
        assert!(
            runtime
                .handle(RunnerCommand::Apply { handoff }, RunnerPeer::Owner)
                .accepted
        );
        for (number, kind, advance_to) in [
            (
                "resource-store-finalization-audit",
                EffectKind::ResourceStoreCreate,
                CutoverPhase::ProviderInstall,
            ),
            (
                "provider-finalization-audit",
                EffectKind::ProviderInstall,
                CutoverPhase::ZoneCutover,
            ),
            (
                "zone-finalization-audit",
                EffectKind::ZoneActivation,
                CutoverPhase::Activation,
            ),
            (
                "guest-finalization-audit",
                EffectKind::GuestActivation,
                CutoverPhase::Verification,
            ),
        ] {
            assert!(
                runtime
                    .handle(
                        RunnerCommand::Effect {
                            effect_id: d2b_cutover::EffectId::new(format!("effect-{number}"))
                                .expect("effect id"),
                            step_id: StepId::new(format!("step-{number}")).expect("step id"),
                            kind,
                            replay_class: ReplayClass::Repeatable,
                            advance_to: Some(advance_to),
                            identity: None,
                            handoff: None,
                            payload: None,
                        },
                        RunnerPeer::Owner,
                    )
                    .accepted
            );
        }
        runtime
            .operation
            .verify(&d2b_cutover::VerificationInput::new(
                [d2b_cutover::ZoneVerification::new(
                    ZoneId::new("zone-apply").expect("zone"),
                    true,
                )],
                true,
                true,
                true,
                true,
            ))
            .expect("verify");
        runtime.audit_sink = Box::new(UnavailableAuditSink);
        let consent = d2b_cutover::FinalizationConsent::issue(
            runtime.bootstrap.request.finalization_binding(),
            now_ms(),
            now_ms().saturating_add(10_000),
        )
        .expect("consent");
        let response = runtime.handle(
            RunnerCommand::Finalize {
                consent,
                plan: FinalizationPlan {
                    artifacts: Vec::new(),
                },
            },
            RunnerPeer::Owner,
        );
        assert_eq!(
            response.error,
            Some(d2b_cutover::RunnerSocketError::AuditUnavailable)
        );
        assert_eq!(runtime.operation.state(), OperationState::CutoverSucceeded);
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn hold_refuses_without_privileged_audit_and_does_not_advance() {
        let mut runtime = runtime("hold");
        let response = runtime.handle(
            RunnerCommand::Hold {
                reason: "incident".to_owned(),
            },
            RunnerPeer::Owner,
        );
        assert!(!response.accepted);
        assert_eq!(response.error, Some(RunnerSocketError::AuditUnavailable));
        assert_eq!(runtime.operation.state(), OperationState::Planned);
        assert!(runtime.operation.journal().records().is_empty());
        let (_, records) = d2b_cutover::load_journal(&runtime.paths.journal).expect("journal");
        assert!(records.is_empty());
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn resume_refuses_without_privileged_audit_and_does_not_advance() {
        let mut runtime = runtime("resume");
        runtime
            .operation
            .request_hold(
                runtime.bootstrap.request.operator_id().clone(),
                HoldReason::new("setup").expect("reason"),
                // Test fixture only: establish a pre-existing held journal.
                AuditEvidence::durable("test-setup-audit").expect("setup evidence"),
            )
            .expect("setup hold");
        let journal_bytes = runtime
            .operation
            .journal_bytes()
            .expect("setup journal bytes");
        d2b_cutover::persist_journal(&runtime.paths.journal, &runtime.bootstrap, &journal_bytes)
            .unwrap_or_else(|error| panic!("persist setup hold failed: {error}"));
        let records_before = runtime.operation.journal().records().len();
        let (_, records_on_disk_before) =
            d2b_cutover::load_journal(&runtime.paths.journal).expect("journal before resume");
        let response = runtime.handle(
            RunnerCommand::Resume {
                fresh_consent: None,
            },
            RunnerPeer::Owner,
        );
        assert!(!response.accepted);
        assert_eq!(response.error, Some(RunnerSocketError::AuditUnavailable));
        assert_eq!(runtime.operation.state(), OperationState::Held);
        assert_eq!(runtime.operation.journal().records().len(), records_before);
        let (_, records_on_disk_after) =
            d2b_cutover::load_journal(&runtime.paths.journal).expect("journal after resume");
        assert_eq!(records_on_disk_after, records_on_disk_before);
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    struct FixedAuditSink;

    impl AuditSink for FixedAuditSink {
        fn publish(
            &mut self,
            _transition: d2b_contracts::broker_wire::CutoverAuditTransition,
            _phase: u8,
            _reason_digest: Option<d2b_cutover::Digest>,
        ) -> Result<AuditEvidence, AuditSinkError> {
            AuditEvidence::durable("typed-audit-record").map_err(|_| AuditSinkError::Unavailable)
        }
    }

    #[test]
    fn durable_audit_evidence_allows_hold_and_persists_it() {
        let mut runtime = runtime_with_audit("durable", Box::new(FixedAuditSink));
        let response = runtime.handle(
            RunnerCommand::Hold {
                reason: "incident".to_owned(),
            },
            RunnerPeer::Owner,
        );
        assert!(response.accepted);
        assert_eq!(runtime.operation.state(), OperationState::Held);
        let (_, records) = d2b_cutover::load_journal(&runtime.paths.journal).expect("journal");
        assert!(!records.is_empty());
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn cutover_resume_revalidates_recovery_and_clears_audited_hold() {
        let (mut runtime, _handoff, _kinds) = cutover_runtime_for_apply("resume-cutover");
        runtime.audit_sink = Box::new(FixedAuditSink);
        runtime
            .operation
            .request_hold(
                runtime.bootstrap.request.operator_id().clone(),
                HoldReason::new("inspect").expect("reason"),
                AuditEvidence::durable("hold-audit").expect("hold audit"),
            )
            .expect("hold");
        runtime.persist().expect("persist hold");

        let response = runtime.handle(
            RunnerCommand::Resume {
                fresh_consent: None,
            },
            RunnerPeer::Owner,
        );
        assert!(response.accepted, "resume response: {:?}", response.error);
        assert_eq!(
            runtime.operation.state(),
            OperationState::Applying(CutoverPhase::Preflight)
        );
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn non_owner_resume_requires_the_bound_fresh_consent_digest() {
        let mut runtime = runtime_with_audit("fresh-consent", Box::new(FixedAuditSink));
        runtime
            .operation
            .request_hold(
                runtime.bootstrap.request.operator_id().clone(),
                HoldReason::new("inspect").expect("reason"),
                AuditEvidence::durable("setup-audit").expect("audit"),
            )
            .expect("hold");
        let response = runtime.handle(
            RunnerCommand::Resume {
                fresh_consent: Some(d2b_cutover::Digest::derive(
                    "d2b:test:wrong-consent",
                    b"wrong",
                )),
            },
            RunnerPeer::Admin,
        );
        assert!(!response.accepted);
        assert_eq!(response.error, Some(RunnerSocketError::OperatorMismatch));
        assert_eq!(runtime.operation.state(), OperationState::Held);
        let _ = std::fs::remove_dir_all(runtime.paths.root());
    }

    #[test]
    fn closed_effect_mapping_excludes_the_internal_broker_marker() {
        assert_eq!(
            cutover_effect_kind(EffectKind::ClosureActivation),
            Some(CutoverEffectKind::ClosureActivation)
        );
        assert_eq!(cutover_effect_kind(EffectKind::CutoverBroker), None);
        assert_eq!(
            cutover_effect_kind(EffectKind::ScopedGuestReset),
            Some(CutoverEffectKind::ScopedGuestReset)
        );
    }
}
