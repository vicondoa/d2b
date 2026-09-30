//! The broker's admitted authority projection (U7, KTD6-KTD7).
//!
//! # The broker holds a projection, not a second desired store
//!
//! Only the manager owns desired rows. What the broker holds is the
//! *projection* those rows were accepted into: the Zone identity, the verified
//! deployment root, the accepted `Role` and `RoleBinding` rows the
//! authorization decisions read, the projection cursor and its digest, any
//! prepared fence, and the effect and reservation journal. Every decision below
//! runs against the rows that were already accepted, never against a summary
//! the candidate could shape, so a candidate that would introduce its own grant
//! cannot authorize its own introduction (KTD7, R8).
//!
//! # One worker serializes the whole Zone
//!
//! `PrepareChange`, `CommitChange`, cancellation and recovery, and `BeginEffect`
//! all run on one dedicated bounded worker per broker process, in channel
//! order. Channel order is decision order, which is what makes the
//! pending-launch race decidable: the child's exec-release gate and the fence
//! are processed by the same worker, so exactly one of them happened first and
//! the outcome is a property of the serialized order rather than of a race.
//!
//! Nothing on the worker blocks. A child's I/O and reap waits happen outside
//! its mailbox and report a completion message, so the worker stays able to
//! service control and handshake messages while a child finishes.
//!
//! # A fence blocks new use, not the way out
//!
//! A prepared candidate freezes the Zone's *new-effect* admission. It does not
//! freeze the control lane, which admits exact transaction replay, cancel, and
//! resynchronization plus already-owned observation, revoke, stop, detach,
//! helper drain, and reservation release. Every control action is bound to a
//! transaction identity this broker durably saw or to an effect identity in the
//! journal, and every control kind resolves to an
//! [`AdmissionStage`](d2b_contracts_resource::v3::AdmissionStage) that does not
//! admit new use, so the lane cannot express a grant.
//!
//! A control timeout keeps the fence. Nothing here thaws a Zone because a
//! caller went away: the only transitions out of a fence are an exact commit,
//! an exact cancel, and a resynchronization that preserves the accepted lower
//! bound.
//!
//! # Durable state
//!
//! One JSON document under the broker's state root, written as tmp file +
//! fsync + rename + directory fsync, the durable shape the trusted-context store
//! already uses. A broker restart strictly increments the epoch and moves every
//! known Zone to [`ZoneAuthorityState::Reconciling`], so no new effect is
//! admitted until the manager resynchronizes the accepted state and the
//! outstanding transactions. The transient half - the in-flight snapshot
//! reassembly buffer - is deliberately not durable: a restart mid-transfer
//! leaves the Zone fenced and requires a full resynchronization, which is the
//! plan's rule rather than a recoverable partial state.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use d2b_contracts_broker::broker_wire::{
    AcceptedAuthority, AuthorityCursor, AuthorityProjectionRow, AuthorityPublicationEnvelope,
    AuthorityPublicationOpen, AuthorityPublicationRequest, AuthorityPublicationResponse,
    AuthoritySnapshot, BeginEffectRequest, BeginSnapshotRequest, CancelTransactionRequest,
    CommitChangeRequest, ControlActionRequest, EffectExitRequest, EndSnapshotRequest,
    MAX_PUBLICATION_CHUNK_BYTES, MAX_PUBLICATION_CHUNKS, MAX_PUBLICATION_CONTROL_QUEUE,
    MAX_PUBLICATION_QUEUE,
    MAX_PUBLICATION_ROWS, MAX_PUBLICATION_SNAPSHOT_BYTES, OpenPublicationSessionResponse,
    PrepareChangeRequest, PreparedTransaction, PublicationControlKind, PublicationLimits,
    PublicationMutationKind, PublicationRefusal, PublicationSession, PublicationSessionBinding,
    PublicationTransactionId, ReleaseEffectRequest, ResynchronizeRequest, RevocationConvergence,
    SnapshotChunkRequest, ZoneAuthorityState, publication_candidate_digest,
    PUBLICATION_CONTROL_NOT_BOUND, PUBLICATION_DIGEST_MISMATCH,
    PUBLICATION_DUPLICATE_TRANSACTION, PUBLICATION_EFFECT_UNPROVEN, PUBLICATION_FENCE_HELD,
    PUBLICATION_RECONCILIATION_REQUIRED, PUBLICATION_SESSION_BOUND_ELSEWHERE,
    PUBLICATION_SESSION_INVALID, PUBLICATION_SNAPSHOT_INCOMPLETE,
    PUBLICATION_SNAPSHOT_IN_PROGRESS, PUBLICATION_SNAPSHOT_TOO_LARGE,
    PUBLICATION_STALE_PREDECESSOR, PUBLICATION_UNKNOWN_TRANSACTION, PUBLICATION_WRONG_ZONE,
};
use d2b_contracts_resource::v3::{
    AdmissionDecision, AdmissionStage, AuthoritySubject, AuthoritySubjectKind,
    DesiredDigest, DesiredRevision, RefusalReason, ResourceRef, StoreIncarnation, ZoneId,
};
use d2b_core::resource_authority::{
    AcceptedGraph, AcceptedGraphError, AuthorityRowKind, GraphAuthority, GraphMutation,
    MutationKind, MutationSubjectEvidence, ProjectionRow, TransportIdentity,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::sync::{mpsc, oneshot};

/// The store label a Zone record carries before its first publication has
/// established one.
///
/// A Zone with no projection is `Unprovisioned`, not unfenced: this value is
/// only ever a placeholder inside a record the broker has already reported as
/// unprovisioned, and the first publication replaces it before any decision
/// reads it.
const UNSET_STORE: &str = "store-unset";

// ---------------------------------------------------------------------------
// Broker-private session material
// ---------------------------------------------------------------------------

/// The broker-private material every publication session is derived from.
///
/// A session is `sha256(material, binding)`, and the broker recomputes and
/// compares rather than looking the presented value up. That is what makes a
/// copied session useless to a provider handler: the bytes it observed do not
/// reproduce under material it does not hold, and a value it assembles itself
/// reproduces to nothing.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
struct SessionMaterial(String);

impl core::fmt::Debug for SessionMaterial {
    /// The material is broker state; its rendering is not a diagnostic value.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("SessionMaterial(<redacted>)")
    }
}

impl SessionMaterial {
    /// Mint material for a broker that holds none.
    ///
    /// The value is never secret material in the cryptographic sense - it
    /// lives in the broker's own root-owned state directory - but it is state
    /// only the broker holds, which is the property the session derivation
    /// needs.
    fn mint() -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"d2b:v3:publication-session-material");
        hasher.update([0]);
        hasher.update(std::process::id().to_be_bytes());
        hasher.update(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|since| since.as_nanos())
                .unwrap_or_default()
                .to_be_bytes(),
        );
        let digest = hasher.finalize();
        let mut value = String::with_capacity(64);
        for byte in digest {
            use std::fmt::Write;
            let _ = write!(value, "{byte:02x}");
        }
        Self(value)
    }

    /// Derive the token one binding mints.
    fn derive(&self, binding: &PublicationSessionBinding) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"d2b:v3:publication-session");
        hasher.update([0]);
        hasher.update(self.0.as_bytes());
        hasher.update([0]);
        hasher.update(binding.zone.as_bytes());
        hasher.update([0]);
        hasher.update(binding.store_incarnation.as_str().as_bytes());
        hasher.update([0]);
        hasher.update(binding.broker_epoch.to_be_bytes());
        hasher.update([0]);
        hasher.update(format!("{:?}", binding.initiating_subject.kind()).as_bytes());
        hasher.update([0]);
        hasher.update(
            binding
                .initiating_subject
                .resource_ref()
                .map(|reference| reference.to_string())
                .unwrap_or_default()
                .as_bytes(),
        );
        hasher.update([0]);
        hasher.update(binding.accepted.sequence.get().to_be_bytes());
        hasher.update([0]);
        hasher.update(binding.accepted.digest.as_str().as_bytes());
        let digest = hasher.finalize();
        let mut value = String::with_capacity(72);
        use std::fmt::Write;
        let _ = write!(value, "pub-");
        for byte in digest {
            let _ = write!(value, "{byte:02x}");
        }
        value
    }

    /// Mint the session one binding is served under.
    fn mint_session(&self, binding: &PublicationSessionBinding) -> PublicationSession {
        // The derived value uses the canonical token alphabet, so parsing it
        // cannot fail; the expect names that invariant rather than widening
        // `parse` for a value the broker itself produced.
        PublicationSession::parse(self.derive(binding))
            .expect("a broker-derived session token is canonical")
    }
}

// ---------------------------------------------------------------------------
// Durable state
// ---------------------------------------------------------------------------

/// One accepted authority row in the projection.
///
/// Only the rows the authorization decisions read are stored. A `Process` or
/// `Volume` row is not authority this broker re-evaluates, and keeping its
/// spec bytes here would turn the projection into a second copy of the
/// manager's desired store, which KTD7 forbids.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct AcceptedAuthorityRow {
    reference: ResourceRef,
    desired_revision: DesiredRevision,
    desired_digest: DesiredDigest,
    admitted: Vec<u8>,
}

/// How far one accepted launch has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum EffectPhase {
    /// `BeginEffect` passed and the launch has not been released for exec. This
    /// is pending *new* use, not an already-running workload.
    Admitted,
    /// Release authorization won before the fence. The launch is accounted as
    /// existing use and drains as such.
    Released,
    /// The fence won before release. Setup is cancelled and reaped; the child
    /// cannot exec under its earlier `BeginEffect`.
    Cancelled,
    /// The child's completion was reported.
    Exited,
}

impl EffectPhase {
    /// Whether this phase is settled for the purpose of a reducing commit.
    ///
    /// A released launch is accounted as a pre-fence release and an exited one
    /// is proved. An admitted or cancelled one still has a child that has not
    /// been shown gone, and a reducing change cannot become accepted while such
    /// a child could still appear with the removed rights.
    const fn is_settled(self) -> bool {
        matches!(self, Self::Released | Self::Exited)
    }
}

/// One effect in the broker's journal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct EffectRecord {
    transaction: PublicationTransactionId,
    target: ResourceRef,
    /// The accepted projection the effect was admitted under.
    accepted: AuthorityCursor,
    subject: AuthoritySubject,
    phase: EffectPhase,
    /// Whether the child reached exec before it ended.
    reached_exec: bool,
}

/// One reservation the broker has observed, and whether it still holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReservationRecord {
    transaction: PublicationTransactionId,
    target: ResourceRef,
    released: bool,
}

/// What one durably prepared candidate holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PreparedRecord {
    transaction: PublicationTransactionId,
    expected: AuthorityCursor,
    committed: AuthorityCursor,
    /// The digest of the exact candidate bytes the fence was prepared for.
    digest: DesiredDigest,
    kind: PublicationMutationKind,
    /// Whether accepting this candidate could reduce the Zone's authority,
    /// which is what makes the pending-launch proof obligation apply.
    reducing: bool,
}

/// One Zone's durable posture. A snapshot transfer's reassembly buffer is
/// transient by design, so a restart mid-transfer resumes as fenced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "posture", rename_all = "kebab-case")]
enum PersistedPosture {
    /// Ordinary admission is open under the accepted cursor.
    Unfenced,
    /// A prepared candidate holds the Zone's new-effect admission.
    Fenced {
        prepared: Box<PreparedRecord>,
    },
    /// A bounded snapshot transfer is in progress; nothing is active yet.
    SnapshotInProgress {
        transaction: PublicationTransactionId,
        committed: AuthorityCursor,
        received_chunks: u32,
        total_chunks: u32,
    },
    /// The broker restarted and has not been reconciled.
    Reconciling,
}

/// One Zone's durable projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedZone {
    /// The Zone label, carried so the projection is keyed by the exact label
    /// the manager published under.
    zone: String,
    /// The same label as the typed identity the prior graph is built under.
    /// The label is parsed once, when the record is created, so no decision
    /// re-derives it from a string.
    zone_id: ZoneId,
    store_incarnation: StoreIncarnation,
    root_subject: AuthoritySubject,
    accepted: AuthorityCursor,
    /// The last session this broker minted for the Zone. Every message must
    /// present exactly it, so a session cannot be replayed for another Zone,
    /// another store generation, or a cursor the manager has moved past.
    session: Option<PublicationSessionBinding>,
    posture: PersistedPosture,
    /// Accepted authority rows, keyed by the canonical reference rendering.
    rows: BTreeMap<String, AcceptedAuthorityRow>,
    effects: BTreeMap<String, EffectRecord>,
    reservations: BTreeMap<String, ReservationRecord>,
    /// Every transaction identity this broker durably saw for the Zone, which
    /// is what a control action must be bound to.
    transactions: BTreeSet<String>,
}

/// The whole durable projection, one document under the broker state root.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct PersistedProjection {
    epoch: u64,
    material: SessionMaterial,
    zones: BTreeMap<String, PersistedZone>,
}

/// The failure of one projection operation.
#[derive(Debug)]
pub enum AuthorityProjectionError {
    /// The store could not open, persist, or read its durable state.
    Io {
        /// What failed.
        detail: String,
    },
    /// The durable state is not the store's own shape.
    Corrupt {
        /// What failed.
        detail: String,
    },
    /// A request was refused, and the Zone stays as the reply says.
    Refused(Box<PublicationRefusal>),
}

impl std::fmt::Display for AuthorityProjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io { detail } => write!(f, "authority projection I/O: {detail}"),
            Self::Corrupt { detail } => write!(f, "authority projection corrupt: {detail}"),
            Self::Refused(refusal) => write!(
                f,
                "authority publication refused: {} ({:?})",
                refusal.code, refusal.reason
            ),
        }
    }
}

impl std::error::Error for AuthorityProjectionError {}

impl AuthorityProjectionError {
    /// The refusal this error carries, when it carries one.
    pub fn refusal(&self) -> Option<&PublicationRefusal> {
        match self {
            Self::Refused(refusal) => Some(refusal),
            Self::Io { .. } | Self::Corrupt { .. } => None,
        }
    }

    /// Whether the Zone is fenced after this error.
    ///
    /// A store that could not answer is not a thaw: a durable failure leaves
    /// the Zone exactly as fenced as it was.
    pub fn is_fenced(&self) -> bool {
        self.refusal().map_or(true, |refusal| refusal.fenced)
    }

    fn io(detail: String) -> Self {
        Self::Io { detail }
    }

    fn corrupt(detail: String) -> Self {
        Self::Corrupt { detail }
    }
}

/// The broker's own reply type: an outcome, or a typed refusal carrying the
/// state the Zone is left in.
type Reply<T> = Result<T, AuthorityProjectionError>;

// ---------------------------------------------------------------------------
// The worker's transient state
// ---------------------------------------------------------------------------

/// One in-flight bounded snapshot transfer.
///
/// The buffer is bounded by the declared byte ceiling, is never read as
/// authority until the final digest matches, and is dropped whole on any
/// mismatch so no partial snapshot survives to become active.
#[derive(Debug, Clone)]
struct SnapshotTransfer {
    transaction: PublicationTransactionId,
    total_chunks: u32,
    received_chunks: u32,
    bytes: Vec<u8>,
}

/// Everything the worker owns. Touched only on the worker thread.
struct ProjectionWorkerState {
    root: PathBuf,
    durable: PersistedProjection,
    transfers: BTreeMap<String, SnapshotTransfer>,
}

impl ProjectionWorkerState {
    fn state_path(root: &Path) -> PathBuf {
        root.join(AuthorityProjection::STATE_DIR).join("state.json")
    }

    /// The reply shape for one Zone, whatever posture it is in.
    fn public_state(&self, zone: &str) -> ZoneAuthorityState {
        let Some(zone_state) = self.durable.zones.get(zone) else {
            return ZoneAuthorityState::Unprovisioned;
        };
        let store = zone_state.store_incarnation.clone();
        let accepted = zone_state.accepted.clone();
        match &zone_state.posture {
            PersistedPosture::Unfenced => ZoneAuthorityState::Unfenced {
                store_incarnation: store,
                accepted,
            },
            PersistedPosture::Fenced { prepared } => ZoneAuthorityState::Fenced {
                store_incarnation: store,
                accepted,
                transaction: prepared.transaction.clone(),
                committed: prepared.committed.clone(),
            },
            PersistedPosture::SnapshotInProgress {
                transaction,
                received_chunks,
                total_chunks,
                ..
            } => ZoneAuthorityState::SnapshotInProgress {
                store_incarnation: store,
                accepted,
                transaction: transaction.clone(),
                received_chunks: *received_chunks,
                total_chunks: *total_chunks,
            },
            PersistedPosture::Reconciling => ZoneAuthorityState::Reconciling {
                store_incarnation: store,
                accepted,
                transaction: None,
            },
        }
    }

    /// The prior accepted graph this Zone's decisions read.
    fn prior_graph(&self, zone: &PersistedZone) -> Result<AcceptedGraph, AcceptedGraphError> {
        AcceptedGraph::from_canonical_rows(
            zone.zone_id.clone(),
            zone.store_incarnation.clone(),
            zone.root_subject.clone(),
            zone.rows
                .values()
                .map(|row| ProjectionRow::new(&row.reference, &row.admitted)),
        )
    }

    /// The Zone's durable record, created on first sight.
    fn zone_mut(&mut self, zone: &str) -> &mut PersistedZone {
        self.durable.zones.entry(zone.to_owned()).or_insert_with(|| {
            PersistedZone {
                zone: zone.to_owned(),
                zone_id: ZoneId::parse(zone)
                    .expect("a session open refused every non-canonical Zone label"),
                store_incarnation: StoreIncarnation::parse(UNSET_STORE)
                    .expect("the unset store label is canonical"),
                root_subject: AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
                accepted: AuthorityCursor::initial(),
                session: None,
                posture: PersistedPosture::Unfenced,
                rows: BTreeMap::new(),
                effects: BTreeMap::new(),
                reservations: BTreeMap::new(),
                transactions: BTreeSet::new(),
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Shared refusals
// ---------------------------------------------------------------------------

/// One typed refusal, built against the Zone's current public state.
fn refuse(
    state: &ProjectionWorkerState,
    zone: &str,
    code: &'static str,
    stage: AdmissionStage,
    reason: RefusalReason,
    fenced: bool,
) -> AuthorityProjectionError {
    AuthorityProjectionError::Refused(Box::new(PublicationRefusal {
        code: code.to_owned(),
        stage,
        reason,
        fenced,
        state: state.public_state(zone),
    }))
}

/// One refusal that additionally freezes the Zone, whatever it was doing.
///
/// Every snapshot-integrity failure lands here: a gap, a conflicting
/// transaction, a missing chunk, an old epoch, and a digest mismatch all leave
/// the Zone fenced and require a full resynchronization, so they are refused
/// through one path rather than five.
fn fence(
    state: &mut ProjectionWorkerState,
    zone: &str,
    code: &'static str,
    stage: AdmissionStage,
    reason: RefusalReason,
) -> AuthorityProjectionError {
    if let Some(zone_state) = state.durable.zones.get_mut(zone)
        && !matches!(zone_state.posture, PersistedPosture::Fenced { .. })
    {
        // Keep whatever prepared identity was already held: a failed transfer
        // must not quietly drop a fence the manager still has outstanding.
        if !matches!(zone_state.posture, PersistedPosture::Reconciling) {
            zone_state.posture = PersistedPosture::Reconciling;
        }
    }
    refuse(state, zone, code, stage, reason, true)
}

/// Drop an in-flight transfer whole, so no partial snapshot can survive to
/// become active.
fn drop_transfer(state: &mut ProjectionWorkerState, zone: &str) {
    state.transfers.remove(zone);
}

/// The authority rows a published row set contributes to the projection.
fn authority_rows(rows: &[AuthorityProjectionRow]) -> BTreeMap<String, AcceptedAuthorityRow> {
    rows.iter()
        .filter(|row| AuthorityRowKind::of_reference(&row.resource_ref).is_authority())
        .map(|row| {
            (
                row.resource_ref.to_string(),
                AcceptedAuthorityRow {
                    reference: row.resource_ref.clone(),
                    desired_revision: row.desired_revision,
                    desired_digest: row.desired_digest.clone(),
                    admitted: row.admitted.clone(),
                },
            )
        })
        .collect()
}

/// Whether installing this candidate could reduce the Zone's authority.
///
/// The test is structural and conservative in the safe direction: a change that
/// removes an accepted authority row, or rewrites one, is treated as reducing
/// even when the rewrite happens to widen. Only a change that adds authority
/// rows and rewrites nothing already accepted is treated as non-reducing, so
/// the pending-launch proof obligation is never skipped for a change that might
/// have narrowed a grant.
fn candidate_is_reducing(
    current: &BTreeMap<String, AcceptedAuthorityRow>,
    candidate: &[AuthorityProjectionRow],
    removed: &[ResourceRef],
    kind: PublicationMutationKind,
) -> bool {
    if kind.is_reducing() {
        return true;
    }
    if removed
        .iter()
        .any(|reference| current.contains_key(&reference.to_string()))
    {
        return true;
    }
    candidate.iter().any(|row| {
        AuthorityRowKind::of_reference(&row.resource_ref).is_authority()
            && current.contains_key(&row.resource_ref.to_string())
    })
}

/// Resolve the Zone a message is about and admit its session.
///
/// A message is served only under the exact session this broker last minted for
/// that Zone, and only while the accepted cursor the session was bound to is
/// still the accepted cursor: a manager whose view has moved on re-establishes
/// the session instead of publishing against a stale one.
fn admit_zone(
    state: &ProjectionWorkerState,
    session: &PublicationSession,
    zone: &str,
) -> Reply<String> {
    let Some(zone_state) = state.durable.zones.get(zone) else {
        return Err(refuse_session(state, zone, PUBLICATION_SESSION_INVALID));
    };
    let Some(binding) = zone_state.session.clone() else {
        return Err(refuse_session(state, zone, PUBLICATION_SESSION_INVALID));
    };
    if binding.accepted != zone_state.accepted {
        return Err(refuse_session(state, zone, PUBLICATION_SESSION_BOUND_ELSEWHERE));
    }
    if state.durable.material.mint_session(&binding).as_str() != session.as_str() {
        return Err(refuse_session(state, zone, PUBLICATION_SESSION_INVALID));
    }
    Ok(zone.to_owned())
}

/// Resolve the Zone and check the store generation the message names.
///
/// An incarnation is an identity, never an ordered counter: a message naming a
/// generation the broker already accepted a different one describes a different
/// store, not a newer one, and installing it is the explicit
/// ownership-bounded reset's job - never ordinary acceptance.
fn admit_message(
    state: &mut ProjectionWorkerState,
    session: &PublicationSession,
    zone: &str,
    incarnation: &StoreIncarnation,
) -> Reply<String> {
    let resolved = admit_zone(state, session, zone)?;
    let moved = state
        .durable
        .zones
        .get(&resolved)
        .is_some_and(|held| !held.transactions.is_empty() && held.store_incarnation != *incarnation);
    if moved {
        return Err(fence(
            state,
            &resolved,
            PUBLICATION_STALE_PREDECESSOR,
            AdmissionStage::Authorize,
            RefusalReason::StoreIncarnationMismatch,
        ));
    }
    Ok(resolved)
}

/// The refusal every session failure shares. Nothing the broker holds can
/// answer it, and the Zone keeps whatever posture it had.
fn refuse_session(
    state: &ProjectionWorkerState,
    zone: &str,
    code: &'static str,
) -> AuthorityProjectionError {
    refuse(
        state,
        zone,
        code,
        AdmissionStage::Authorize,
        RefusalReason::IdentityNotAuthorized,
        state.public_state(zone).is_fenced(),
    )
}

// ---------------------------------------------------------------------------
// The serialized worker
// ---------------------------------------------------------------------------

/// The caller-side handle of the single writer.
struct ProjectionWriter {
    commands: mpsc::Sender<ProjectionCommand>,
}

/// One serialized command the authority worker executes in channel order.
enum ProjectionCommand {
    /// Load the durable state, bump the epoch, move every known Zone to
    /// reconciling, and persist - the open barrier, before anything is served.
    Bootstrap {
        root: PathBuf,
        reply: oneshot::Sender<Reply<()>>,
    },
    OpenSession {
        open: AuthorityPublicationOpen,
        reply: oneshot::Sender<Reply<OpenPublicationSessionResponse>>,
    },
    /// Every publication message, with the Zone it is about.
    Publication {
        zone: String,
        session: PublicationSession,
        request: AuthorityPublicationRequest,
        reply: oneshot::Sender<Reply<AuthorityPublicationResponse>>,
    },
    Status {
        zone: String,
        reply: oneshot::Sender<ZoneAuthorityState>,
    },
    Epoch {
        reply: oneshot::Sender<u64>,
    },
    Shutdown {
        exited: oneshot::Sender<()>,
    },
}

/// The broker's admitted authority projection.
pub struct AuthorityProjection {
    writer: ProjectionWriter,
}

impl AuthorityProjection {
    /// The directory name under the broker state root the durable state lives
    /// in.
    const STATE_DIR: &'static str = "authority";

    /// The bound on admitted-but-unstarted publication commands.
    const WORKER_QUEUE_DEPTH: usize = MAX_PUBLICATION_QUEUE;

    /// The bound on admitted-but-unstarted control commands.
    ///
    /// The control lane is the fence's way out, so it is bounded separately
    /// and stays serviceable when the ordinary lane is saturated: every control
    /// command can only reduce use or recover known state, so admitting fewer
    /// of them grants nothing.
    const CONTROL_QUEUE_DEPTH: usize = MAX_PUBLICATION_CONTROL_QUEUE;

    /// Open the projection under `root` from a synchronous caller.
    ///
    /// The open barrier - load, bump the epoch, fence every known Zone for
    /// reconciliation, persist - runs on the worker, and its reply gates the
    /// handle: no message is served before the barrier completed.
    #[cfg(test)]
    pub fn open(root: impl Into<PathBuf>) -> Reply<Self> {
        let root = root.into();
        let (commands, receiver) = mpsc::channel::<ProjectionCommand>(Self::WORKER_QUEUE_DEPTH);
        std::thread::Builder::new()
            .name("d2b-broker-authority".to_owned())
            .spawn(move || projection_worker_loop(receiver))
            .map_err(|error| {
                AuthorityProjectionError::io(format!("spawn authority worker: {error}"))
            })?;
        let (reply_tx, reply_rx) = oneshot::channel();
        commands
            .blocking_send(ProjectionCommand::Bootstrap {
                root: root.clone(),
                reply: reply_tx,
            })
            .map_err(|_| {
                AuthorityProjectionError::io("authority worker unavailable".to_owned())
            })?;
        reply_rx.blocking_recv().map_err(|_| {
            AuthorityProjectionError::io("authority worker unavailable".to_owned())
        })??;
        Ok(Self {
            writer: ProjectionWriter { commands },
        })
    }

    /// Open the projection under `root` from an async caller.
    pub async fn open_async(root: impl Into<PathBuf>) -> Reply<Self> {
        let root = root.into();
        let (commands, receiver) = mpsc::channel::<ProjectionCommand>(Self::WORKER_QUEUE_DEPTH);
        std::thread::Builder::new()
            .name("d2b-broker-authority".to_owned())
            .spawn(move || projection_worker_loop(receiver))
            .map_err(|error| {
                AuthorityProjectionError::io(format!("spawn authority worker: {error}"))
            })?;
        let (reply_tx, reply_rx) = oneshot::channel();
        commands
            .send(ProjectionCommand::Bootstrap {
                root: root.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| AuthorityProjectionError::io("authority worker unavailable".to_owned()))?;
        reply_rx
            .await
            .map_err(|_| AuthorityProjectionError::io("authority worker unavailable".to_owned()))??;
        Ok(Self {
            writer: ProjectionWriter { commands },
        })
    }

    /// The epoch this projection is currently minting sessions under.
    pub async fn epoch(&self) -> u64 {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .writer
            .commands
            .send(ProjectionCommand::Epoch { reply: reply_tx })
            .await
            .is_err()
        {
            return 0;
        }
        reply_rx.await.unwrap_or(0)
    }

    /// How one Zone's projection stands, without changing it.
    pub async fn status(&self, zone: &str) -> ZoneAuthorityState {
        let (reply_tx, reply_rx) = oneshot::channel();
        if self
            .writer
            .commands
            .send(ProjectionCommand::Status {
                zone: zone.to_owned(),
                reply: reply_tx,
            })
            .await
            .is_err()
        {
            return ZoneAuthorityState::Unprovisioned;
        }
        reply_rx.await.unwrap_or(ZoneAuthorityState::Unprovisioned)
    }

    /// Open one Zone publication session.
    pub async fn open_session(
        &self,
        open: AuthorityPublicationOpen,
    ) -> Reply<OpenPublicationSessionResponse> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.writer
            .commands
            .send(ProjectionCommand::OpenSession {
                open,
                reply: reply_tx,
            })
            .await
            .map_err(|_| AuthorityProjectionError::io("authority worker unavailable".to_owned()))?;
        reply_rx
            .await
            .map_err(|_| AuthorityProjectionError::io("authority worker unavailable".to_owned()))?
    }

    /// Serve one publication message.
    ///
    /// The single entry every message family uses, so the session check, the
    /// per-message decision, and the durable commit all run on the one worker
    /// in channel order.
    pub async fn serve(
        &self,
        envelope: &AuthorityPublicationEnvelope,
    ) -> Reply<AuthorityPublicationResponse> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.writer
            .commands
            .send(ProjectionCommand::Publication {
                zone: envelope.zone.clone(),
                session: envelope.session.clone(),
                request: envelope.request.clone(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| AuthorityProjectionError::io("authority worker unavailable".to_owned()))?;
        reply_rx
            .await
            .map_err(|_| AuthorityProjectionError::io("authority worker unavailable".to_owned()))?
    }
}

impl Drop for AuthorityProjection {
    /// Ask the single writer to stop.
    ///
    /// The send is non-blocking on purpose: a handle is routinely dropped from
    /// inside an async context, and a blocking send there would park the
    /// executor worker. Nothing is lost by not waiting - every decision
    /// persisted before it answered, and closing the channel is what actually
    /// ends the worker.
    fn drop(&mut self) {
        let _ = self.writer.commands.try_send(ProjectionCommand::Shutdown {
            exited: oneshot::channel().0,
        });
    }
}

fn projection_worker_loop(mut receiver: mpsc::Receiver<ProjectionCommand>) {
    let Some(ProjectionCommand::Bootstrap { root, reply }) = receiver.blocking_recv() else {
        return;
    };
    let mut state = match projection_bootstrap(&root) {
        Ok(state) => state,
        Err(error) => {
            let _ = reply.send(Err(error));
            return;
        }
    };
    let _ = reply.send(Ok(()));
    while let Some(command) = receiver.blocking_recv() {
        match command {
            ProjectionCommand::OpenSession { open, reply } => {
                let result = open_session_locked(&mut state, &open);
                let _ = reply.send(result);
            }
            ProjectionCommand::Publication {
                zone,
                session,
                request,
                reply,
            } => {
                let result = serve_locked(&mut state, &zone, &session, &request);
                let _ = reply.send(result);
            }
            ProjectionCommand::Status { zone, reply } => {
                let _ = reply.send(state.public_state(&zone));
            }
            ProjectionCommand::Epoch { reply } => {
                let _ = reply.send(state.durable.epoch);
            }
            ProjectionCommand::Shutdown { exited } => {
                drop(state);
                let _ = exited.send(());
                return;
            }
            ProjectionCommand::Bootstrap { .. } => {}
        }
    }
}

/// The open barrier: load, strictly bump the epoch, fence every known Zone for
/// reconciliation, and persist before anything is served.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn projection_bootstrap(root: &Path) -> Reply<ProjectionWorkerState> {
    let directory = root.join(AuthorityProjection::STATE_DIR);
    fs::create_dir_all(&directory).map_err(|error| {
        AuthorityProjectionError::io(format!("create {}: {error}", directory.display()))
    })?;
    let path = ProjectionWorkerState::state_path(root);
    let mut durable: PersistedProjection = if path.exists() {
        let bytes =
            fs::read(&path)
                .map_err(|error| {
                    AuthorityProjectionError::io(format!("read {}: {error}", path.display()))
                })?;
        serde_json::from_slice(&bytes).map_err(|error| {
            AuthorityProjectionError::corrupt(format!("{}: {error}", path.display()))
        })?
    } else {
        PersistedProjection {
            epoch: 0,
            material: SessionMaterial::mint(),
            zones: BTreeMap::new(),
        }
    };
    // A fresh instance is a fresh authority lineage. A broker restart must not
    // serve the cached projection it happened to persist, so every known Zone
    // moves to reconciling and denies new effects until the manager
    // resynchronizes the accepted state and the outstanding transactions. The
    // prepared identity stays in the Zone's transaction set, so an exact replay
    // or cancel is still answerable while the Zone waits for its snapshot.
    for zone in durable.zones.values_mut() {
        zone.posture = PersistedPosture::Reconciling;
    }
    durable.epoch = durable.epoch.saturating_add(1);
    persist(&path, &durable)?;
    Ok(ProjectionWorkerState {
        root: root.to_path_buf(),
        durable,
        transfers: BTreeMap::new(),
    })
}

/// The durable persist: tmp file + fsync + rename + directory fsync, on the
/// single writer.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn persist(path: &Path, durable: &PersistedProjection) -> Reply<()> {
    let bytes = serde_json::to_vec(durable)
        .map_err(|error| AuthorityProjectionError::io(format!("serialize: {error}")))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)
            .map_err(|error| {
                AuthorityProjectionError::io(format!("open {}: {error}", tmp.display()))
            })?;
        file.write_all(&bytes).map_err(|error| {
            AuthorityProjectionError::io(format!("write {}: {error}", tmp.display()))
        })?;
        file.sync_all().map_err(|error| {
            AuthorityProjectionError::io(format!("sync {}: {error}", tmp.display()))
        })?;
    }
    fs::rename(&tmp, path).map_err(|error| {
        AuthorityProjectionError::io(format!(
            "rename {} -> {}: {error}",
            tmp.display(),
            path.display()
        ))
    })?;
    if let Some(parent) = path.parent() {
        fs::File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|error| {
                AuthorityProjectionError::io(format!("sync {}: {error}", parent.display()))
            })?;
    }
    Ok(())
}

/// Persist the current durable state, on the single writer.
#[allow(clippy::disallowed_methods, reason = "dedicated bounded worker per plan R4")]
fn commit_durable(state: &ProjectionWorkerState) -> Reply<()> {
    persist(&ProjectionWorkerState::state_path(&state.root), &state.durable)
}

/// Dispatch one publication message on the single writer.
fn serve_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &AuthorityPublicationRequest,
) -> Reply<AuthorityPublicationResponse> {
    match request {
        AuthorityPublicationRequest::BeginSnapshot(request) => {
            begin_snapshot_locked(state, zone, session, request)
                .map(AuthorityPublicationResponse::Progressed)
        }
        AuthorityPublicationRequest::SnapshotChunk(request) => {
            snapshot_chunk_locked(state, zone, session, request)
                .map(AuthorityPublicationResponse::Progressed)
        }
        AuthorityPublicationRequest::EndSnapshot(request) => {
            end_snapshot_locked(state, zone, session, request)
                .map(AuthorityPublicationResponse::Progressed)
        }
        AuthorityPublicationRequest::PrepareChange(request) => {
            prepare_change_locked(state, zone, session, request)
                .map(AuthorityPublicationResponse::Prepared)
        }
        AuthorityPublicationRequest::CommitChange(request) => {
            commit_change_locked(state, zone, session, request)
                .map(AuthorityPublicationResponse::Accepted)
        }
        AuthorityPublicationRequest::CancelTransaction(request) => {
            cancel_transaction_locked(state, zone, session, request)
                .map(AuthorityPublicationResponse::Progressed)
        }
        AuthorityPublicationRequest::Resynchronize(request) => {
            resynchronize_locked(state, zone, session, request)
                .map(AuthorityPublicationResponse::Progressed)
        }
        AuthorityPublicationRequest::ControlAction(request) => {
            control_action_locked(state, zone, session, request)
        }
        AuthorityPublicationRequest::BeginEffect(request) => {
            begin_effect_locked(state, zone, session, request)
        }
        AuthorityPublicationRequest::ReleaseEffect(request) => {
            release_effect_locked(state, zone, session, request)
        }
        AuthorityPublicationRequest::EffectExit(request) => {
            effect_exit_locked(state, zone, session, request)
        }
    }
}

// ---------------------------------------------------------------------------
// Session establishment
// ---------------------------------------------------------------------------

/// Open one Zone publication session on the single writer.
///
/// The session is bound to the Zone, the store generation the broker holds, the
/// epoch it is minting under, the authenticated initiating subject the trusted
/// daemon admission coordinator vouched for, and the accepted cursor this
/// broker held. A Zone this broker has never published for establishes its
/// store generation here, because there is no prior incarnation for a
/// different one to contradict; a Zone it has published for is refused rather
/// than moved onto a generation only the explicit ownership-bounded reset can
/// install.
fn open_session_locked(
    state: &mut ProjectionWorkerState,
    open: &AuthorityPublicationOpen,
) -> Reply<OpenPublicationSessionResponse> {
    let request = &open.request;
    let zone = request.zone.as_str();
    if ZoneId::parse(zone).is_err() {
        return Err(refuse_session(state, zone, PUBLICATION_WRONG_ZONE));
    }
    let moved_generation = state.durable.zones.get(zone).is_some_and(|held| {
        !held.transactions.is_empty() && held.store_incarnation != request.store_incarnation
    });
    if moved_generation {
        return Err(fence(
            state,
            zone,
            PUBLICATION_STALE_PREDECESSOR,
            AdmissionStage::Authorize,
            RefusalReason::StoreIncarnationMismatch,
        ));
    }
    {
        let zone_state = state.zone_mut(zone);
        if zone_state.store_incarnation.as_str() == UNSET_STORE {
            zone_state.store_incarnation = request.store_incarnation.clone();
        }
    }
    let zone_state = state
        .durable
        .zones
        .get(zone)
        .expect("zone_mut just created the record");
    let binding = PublicationSessionBinding {
        zone: zone.to_owned(),
        store_incarnation: zone_state.store_incarnation.clone(),
        broker_epoch: state.durable.epoch,
        initiating_subject: request.initiating_subject.clone(),
        accepted: zone_state.accepted.clone(),
    };
    if let Some(zone_state) = state.durable.zones.get_mut(zone) {
        zone_state.session = Some(binding.clone());
    }
    let session = state.durable.material.mint_session(&binding);
    commit_durable(state)?;
    Ok(OpenPublicationSessionResponse {
        session,
        binding,
        limits: PublicationLimits::default(),
    })
}

// ---------------------------------------------------------------------------
// Bounded snapshot
// ---------------------------------------------------------------------------

/// Open one bounded snapshot transfer.
fn begin_snapshot_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &BeginSnapshotRequest,
) -> Reply<ZoneAuthorityState> {
    let zone = admit_message(state, session, zone, &request.store_incarnation)?;
    if request.total_chunks == 0 || request.total_chunks > MAX_PUBLICATION_CHUNKS {
        return Err(fence(
            state,
            &zone,
            PUBLICATION_SNAPSHOT_TOO_LARGE,
            AdmissionStage::Recover,
            RefusalReason::LimitExceedsCeiling,
        ));
    }
    if request.total_bytes > MAX_PUBLICATION_SNAPSHOT_BYTES {
        return Err(fence(
            state,
            &zone,
            PUBLICATION_SNAPSHOT_TOO_LARGE,
            AdmissionStage::Recover,
            RefusalReason::LimitExceedsCeiling,
        ));
    }
    {
        let zone_state = state
            .durable
            .zones
            .get_mut(&zone)
            .expect("admit_message resolved the zone");
        if matches!(zone_state.posture, PersistedPosture::SnapshotInProgress { .. }) {
            // One snapshot in progress per Zone. A second one is refused rather
            // than interleaved: two reassembly buffers over one accepted cursor
            // would make "the whole document" ambiguous.
            return Err(refuse(
                state,
                &zone,
                PUBLICATION_SNAPSHOT_IN_PROGRESS,
                AdmissionStage::Recover,
                RefusalReason::ConflictingDeclaration,
                true,
            ));
        }
        zone_state.transactions.insert(request.transaction.to_string());
        zone_state.posture = PersistedPosture::SnapshotInProgress {
            transaction: request.transaction.clone(),
            committed: request.cursor.clone(),
            received_chunks: 0,
            total_chunks: request.total_chunks,
        };
    }
    state.transfers.insert(
        zone.clone(),
        SnapshotTransfer {
            transaction: request.transaction.clone(),
            total_chunks: request.total_chunks,
            received_chunks: 0,
            bytes: Vec::new(),
        },
    );
    commit_durable(state)?;
    Ok(state.public_state(&zone))
}

/// Accept one chunk of an in-flight transfer.
///
/// The only ordinal this accepts is the next one: a gap, a repeat, and an
/// out-of-order chunk are the same failure, and each of them drops the whole
/// transfer and leaves the Zone fenced.
fn snapshot_chunk_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &SnapshotChunkRequest,
) -> Reply<ZoneAuthorityState> {
    let zone = admit_zone(state, session, zone)?;
    if request.payload.len() > MAX_PUBLICATION_CHUNK_BYTES {
        drop_transfer(state, &zone);
        return Err(fence(
            state,
            &zone,
            PUBLICATION_SNAPSHOT_TOO_LARGE,
            AdmissionStage::Recover,
            RefusalReason::LimitExceedsCeiling,
        ));
    }
    {
        let Some(transfer) = state.transfers.get(&zone) else {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_SNAPSHOT_INCOMPLETE,
                AdmissionStage::Recover,
                RefusalReason::UnprovenEffect,
            ));
        };
        if transfer.transaction != request.transaction
            || request.total_chunks != transfer.total_chunks
            || request.ordinal != transfer.received_chunks
        {
            drop_transfer(state, &zone);
            return Err(fence(
                state,
                &zone,
                PUBLICATION_SNAPSHOT_INCOMPLETE,
                AdmissionStage::Recover,
                RefusalReason::UnprovenEffect,
            ));
        }
        if transfer.bytes.len() as u64 + request.payload.len() as u64
            > MAX_PUBLICATION_SNAPSHOT_BYTES
        {
            drop_transfer(state, &zone);
            return Err(fence(
                state,
                &zone,
                PUBLICATION_SNAPSHOT_TOO_LARGE,
                AdmissionStage::Recover,
                RefusalReason::LimitExceedsCeiling,
            ));
        }
    }
    if let Some(transfer) = state.transfers.get_mut(&zone) {
        transfer.bytes.extend_from_slice(&request.payload);
        transfer.received_chunks += 1;
        let transaction = transfer.transaction.clone();
        let total_chunks = transfer.total_chunks;
        let received_chunks = transfer.received_chunks;
        if let Some(zone_state) = state.durable.zones.get_mut(&zone) {
            let committed = zone_state.accepted.clone();
            zone_state.posture = PersistedPosture::SnapshotInProgress {
                transaction,
                committed,
                received_chunks,
                total_chunks,
            };
        }
    }
    commit_durable(state)?;
    Ok(state.public_state(&zone))
}

/// Close a transfer and, only if the whole document arrived and its digest
/// matches, install it.
fn end_snapshot_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &EndSnapshotRequest,
) -> Reply<ZoneAuthorityState> {
    let zone = admit_zone(state, session, zone)?;
    let Some(transfer) = state.transfers.get(&zone).cloned() else {
        return Err(fence(
            state,
            &zone,
            PUBLICATION_SNAPSHOT_INCOMPLETE,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
        ));
    };
    if transfer.transaction != request.transaction
        || transfer.received_chunks != transfer.total_chunks
        || request.total_chunks != transfer.total_chunks
    {
        // A missing chunk is not a partial success: the Zone stays fenced and
        // the whole transfer is dropped.
        drop_transfer(state, &zone);
        return Err(fence(
            state,
            &zone,
            PUBLICATION_SNAPSHOT_INCOMPLETE,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
        ));
    }
    if DesiredDigest::of(&transfer.bytes) != request.digest {
        drop_transfer(state, &zone);
        return Err(fence(
            state,
            &zone,
            PUBLICATION_DIGEST_MISMATCH,
            AdmissionStage::Recover,
            RefusalReason::ConflictingDeclaration,
        ));
    }
    let Ok(snapshot) = serde_json::from_slice::<AuthoritySnapshot>(&transfer.bytes) else {
        drop_transfer(state, &zone);
        return Err(fence(
            state,
            &zone,
            PUBLICATION_DIGEST_MISMATCH,
            AdmissionStage::Recover,
            RefusalReason::ConflictingDeclaration,
        ));
    };
    if snapshot.zone != zone {
        drop_transfer(state, &zone);
        return Err(fence(
            state,
            &zone,
            PUBLICATION_WRONG_ZONE,
            AdmissionStage::Recover,
            RefusalReason::StoreIncarnationMismatch,
        ));
    }
    if snapshot.rows.len() > MAX_PUBLICATION_ROWS {
        drop_transfer(state, &zone);
        return Err(fence(
            state,
            &zone,
            PUBLICATION_SNAPSHOT_TOO_LARGE,
            AdmissionStage::Recover,
            RefusalReason::LimitExceedsCeiling,
        ));
    }
    {
        // The accepted lower bound survives a resynchronization: a snapshot may
        // move the projection forward, never below the sequence the broker
        // durably accepted, and never to a different digest at that sequence.
        let Some(zone_state) = state.durable.zones.get(&zone) else {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_SNAPSHOT_INCOMPLETE,
                AdmissionStage::Recover,
                RefusalReason::UnprovenEffect,
            ));
        };
        if snapshot.store_incarnation != zone_state.store_incarnation {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_STALE_PREDECESSOR,
                AdmissionStage::Recover,
                RefusalReason::StoreIncarnationMismatch,
            ));
        }
        if snapshot.cursor.sequence < zone_state.accepted.sequence
            || (snapshot.cursor == zone_state.accepted
                && snapshot.cursor.digest != zone_state.accepted.digest)
        {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_STALE_PREDECESSOR,
                AdmissionStage::Recover,
                RefusalReason::StaleAuthority,
            ));
        }
    }
    // The prepared identity the broker already froze travels with the
    // resynchronization, so reconciling cannot silently clear a fence the
    // manager still has outstanding.
    let outstanding = state
        .durable
        .zones
        .get(&zone)
        .and_then(|zone_state| match &zone_state.posture {
            PersistedPosture::Fenced { prepared } => Some(prepared.transaction.clone()),
            _ => None,
        })
        .or(snapshot.outstanding.clone());
    state.transfers.remove(&zone);
    {
        let Some(zone_state) = state.durable.zones.get_mut(&zone) else {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_SNAPSHOT_INCOMPLETE,
                AdmissionStage::Recover,
                RefusalReason::UnprovenEffect,
            ));
        };
        zone_state.accepted = snapshot.cursor.clone();
        zone_state.root_subject = snapshot.root_subject.clone();
        zone_state.rows = authority_rows(&snapshot.rows);
        zone_state.posture = match &outstanding {
            Some(transaction) => {
                zone_state.transactions.insert(transaction.to_string());
                let committed = zone_state.accepted.clone();
                PersistedPosture::Fenced {
                    prepared: Box::new(PreparedRecord {
                        transaction: transaction.clone(),
                        expected: zone_state.accepted.clone(),
                        committed,
                        digest: snapshot.cursor.digest.clone(),
                        kind: PublicationMutationKind::UpdateSpec,
                        reducing: true,
                    }),
                }
            }
            None => PersistedPosture::Unfenced,
        };
    }
    commit_durable(state)?;
    Ok(state.public_state(&zone))
}

// ---------------------------------------------------------------------------
// Freeze, commit, cancel, resynchronize
// ---------------------------------------------------------------------------

/// Durably freeze the Zone's new-effect admission for one candidate.
///
/// The candidate is evaluated against the prior accepted graph, so a change
/// that would introduce the grant authorizing it is refused: the grant is not
/// in the state the decision reads. Every check - digest, predecessor, size,
/// prior-state evaluation - runs before the fence is written, and any refusal
/// after the write leaves the Zone fenced, because a manager that has been
/// told nothing is better off fenced than thawed.
fn prepare_change_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &PrepareChangeRequest,
) -> Reply<PreparedTransaction> {
    let zone = admit_message(state, session, zone, &request.store_incarnation)?;
    if publication_candidate_digest(&request.candidate, &request.removed) != request.digest {
        // The declared digest is what makes the prepared identity name exact
        // committed bytes; a payload that does not hash to it is refused before
        // any fence is written.
        return Err(fence(
            state,
            &zone,
            PUBLICATION_DIGEST_MISMATCH,
            AdmissionStage::Authorize,
            RefusalReason::ConflictingDeclaration,
        ));
    }
    if request.candidate.len() > MAX_PUBLICATION_ROWS
        || request.removed.len() > MAX_PUBLICATION_ROWS
    {
        return Err(fence(
            state,
            &zone,
            PUBLICATION_SNAPSHOT_TOO_LARGE,
            AdmissionStage::Authorize,
            RefusalReason::LimitExceedsCeiling,
        ));
    }
    {
        // A Zone that already froze one candidate either replays that exact
        // candidate - idempotently, returning the same fence - or is refused:
        // a second identity under one held fence is a conflicting declaration,
        // not a second transaction.
        let replay = match state
            .durable
            .zones
            .get(&zone)
            .map(|zone_state| &zone_state.posture)
        {
            None => None,
            Some(PersistedPosture::Fenced { prepared }) => {
                if prepared.transaction == request.transaction
                    && prepared.committed == request.committed
                    && prepared.digest == request.digest
                {
                    Some(prepared.clone())
                } else {
                    return Err(refuse(
                        state,
                        &zone,
                        PUBLICATION_DUPLICATE_TRANSACTION,
                        AdmissionStage::Authorize,
                        RefusalReason::ConflictingDeclaration,
                        true,
                    ));
                }
            }
            Some(PersistedPosture::Reconciling) => {
                return Err(refuse(
                    state,
                    &zone,
                    PUBLICATION_RECONCILIATION_REQUIRED,
                    AdmissionStage::Recover,
                    RefusalReason::UnprovenEffect,
                    true,
                ));
            }
            Some(PersistedPosture::SnapshotInProgress { .. }) => {
                return Err(refuse(
                    state,
                    &zone,
                    PUBLICATION_SNAPSHOT_IN_PROGRESS,
                    AdmissionStage::Recover,
                    RefusalReason::ConflictingDeclaration,
                    true,
                ));
            }
            Some(PersistedPosture::Unfenced) => None,
        };
        if let Some(prepared) = replay {
            return Ok(PreparedTransaction {
                transaction: request.transaction.clone(),
                expected: prepared.expected,
                committed: prepared.committed,
                digest: prepared.digest,
                reducing: prepared.reducing,
                state: state.public_state(&zone),
            });
        }
    }
    let prior = {
        let Some(zone_state) = state.durable.zones.get(&zone) else {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_UNKNOWN_TRANSACTION,
                AdmissionStage::Authorize,
                RefusalReason::UnprovenEffect,
            ));
        };
        if request.expected != zone_state.accepted
            || request.committed.sequence <= zone_state.accepted.sequence
        {
            // A change that does not name the accepted cursor exactly, or that
            // would not move forward, is stale whatever it contains.
            return Err(fence(
                state,
                &zone,
                PUBLICATION_STALE_PREDECESSOR,
                AdmissionStage::Authorize,
                RefusalReason::StaleAuthority,
            ));
        }
        zone_state.clone()
    };
    let prior = match AcceptedGraph::from_canonical_rows(
        prior.zone_id.clone(),
        prior.store_incarnation.clone(),
        prior.root_subject.clone(),
        prior
            .rows
            .values()
            .map(|row| ProjectionRow::new(&row.reference, &row.admitted)),
    ) {
        Ok(graph) => graph,
        Err(_) => {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_DIGEST_MISMATCH,
                AdmissionStage::Authorize,
                RefusalReason::ConflictingDeclaration,
            ));
        }
    };
    if let Some((code, stage, reason)) =
        evaluate_candidate(&request.candidate, &request.removed, request, &prior)
    {
        // Prior-state authorization, never the candidate's own grants.
        return Err(fence(state, &zone, code, stage, reason));
    }
    let reducing = {
        let Some(zone_state) = state.durable.zones.get_mut(&zone) else {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_UNKNOWN_TRANSACTION,
                AdmissionStage::Authorize,
                RefusalReason::UnprovenEffect,
            ));
        };
        let reducing = candidate_is_reducing(
            &zone_state.rows,
            &request.candidate,
            &request.removed,
            request.kind,
        );
        zone_state.posture = PersistedPosture::Fenced {
            prepared: Box::new(PreparedRecord {
                transaction: request.transaction.clone(),
                expected: request.expected.clone(),
                committed: request.committed.clone(),
                digest: request.digest.clone(),
                kind: request.kind,
                reducing,
            }),
        };
        zone_state.transactions.insert(request.transaction.to_string());
        reducing
    };
    commit_durable(state)?;
    Ok(PreparedTransaction {
        transaction: request.transaction.clone(),
        expected: request.expected.clone(),
        committed: request.committed.clone(),
        digest: request.digest.clone(),
        reducing,
        state: state.public_state(&zone),
    })
}

/// Advance the projection to one exact committed state.
fn commit_change_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &CommitChangeRequest,
) -> Reply<AcceptedAuthority> {
    let zone = admit_message(state, session, zone, &request.store_incarnation)?;
    if publication_candidate_digest(&request.rows, &request.removed) != request.digest {
        return Err(fence(
            state,
            &zone,
            PUBLICATION_DIGEST_MISMATCH,
            AdmissionStage::Authorize,
            RefusalReason::ConflictingDeclaration,
        ));
    }
    let reducing = {
        let Some(zone_state) = state.durable.zones.get_mut(&zone) else {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_UNKNOWN_TRANSACTION,
                AdmissionStage::Authorize,
                RefusalReason::UnprovenEffect,
            ));
        };
        let prepared = match &zone_state.posture {
            PersistedPosture::Fenced { prepared } => prepared.clone(),
            _ => {
                return Err(fence(
                    state,
                    &zone,
                    PUBLICATION_FENCE_HELD,
                    AdmissionStage::Authorize,
                    RefusalReason::UnprovenEffect,
                ));
            }
        };
        if prepared.transaction != request.transaction {
            // The commit must name the exact prepared identity. A different one
            // is not a second transaction; it is a mismatch.
            return Err(refuse(
                state,
                &zone,
                PUBLICATION_UNKNOWN_TRANSACTION,
                AdmissionStage::Authorize,
                RefusalReason::UnprovenEffect,
                true,
            ));
        }
        if prepared.committed != request.committed
            || prepared.expected != zone_state.accepted
            || request.expected != zone_state.accepted
        {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_STALE_PREDECESSOR,
                AdmissionStage::Authorize,
                RefusalReason::StaleAuthority,
            ));
        }
        if prepared.digest != request.digest {
            // The commit must install the exact bytes the fence was prepared
            // for; anything else is a different candidate under one identity.
            return Err(fence(
                state,
                &zone,
                PUBLICATION_DUPLICATE_TRANSACTION,
                AdmissionStage::Authorize,
                RefusalReason::ConflictingDeclaration,
            ));
        }
        let reducing = prepared.reducing;
        if reducing {
            // A reducing change cannot become accepted while an unaccounted
            // pending child could still appear with the removed rights. Every
            // effect admitted under an earlier cursor is affected; a released
            // one is accounted as a pre-fence release and an exited one is
            // proved, but an admitted or cancelled one is not.
            let unproven = zone_state
                .effects
                .values()
                .any(|effect| {
                    effect.accepted.sequence < request.committed.sequence
                        && !effect.phase.is_settled()
                });
            if unproven {
                return Err(refuse(
                    state,
                    &zone,
                    PUBLICATION_EFFECT_UNPROVEN,
                    AdmissionStage::Drain,
                    RefusalReason::UnprovenEffect,
                    true,
                ));
            }
        }
        zone_state.accepted = request.committed.clone();
        zone_state.rows = authority_rows(&request.rows);
        for reference in &request.removed {
            zone_state.rows.remove(&reference.to_string());
        }
        for effect in zone_state.effects.values_mut() {
            effect.transaction = request.transaction.clone();
        }
        zone_state.posture = PersistedPosture::Unfenced;
        reducing
    };
    commit_durable(state)?;
    Ok(AcceptedAuthority {
        transaction: request.transaction.clone(),
        sequence: request.committed.sequence,
        digest: request.digest.clone(),
        reducing,
        unfrozen: true,
        state: state.public_state(&zone),
    })
}

/// Release one prepared identity that committed nothing.
fn cancel_transaction_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &CancelTransactionRequest,
) -> Reply<ZoneAuthorityState> {
    let zone = admit_message(state, session, zone, &request.store_incarnation)?;
    {
        let Some(zone_state) = state.durable.zones.get_mut(&zone) else {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_UNKNOWN_TRANSACTION,
                AdmissionStage::Recover,
                RefusalReason::UnprovenEffect,
            ));
        };
        let matches_prepared = matches!(
            &zone_state.posture,
            PersistedPosture::Fenced { prepared } if prepared.transaction == request.transaction
        );
        if !matches_prepared {
            // A cancel names an existing prepared identity. One this broker does
            // not hold cannot clear a fence, and a Zone left fenced is the
            // correct outcome for a cancel that proves nothing.
            return Err(refuse(
                state,
                &zone,
                PUBLICATION_UNKNOWN_TRANSACTION,
                AdmissionStage::Recover,
                RefusalReason::UnprovenEffect,
                true,
            ));
        }
        let digest_matches = matches!(
            &zone_state.posture,
            PersistedPosture::Fenced { prepared } if prepared.digest == request.digest
        );
        if !digest_matches {
            return Err(refuse(
                state,
                &zone,
                PUBLICATION_DUPLICATE_TRANSACTION,
                AdmissionStage::Recover,
                RefusalReason::ConflictingDeclaration,
                true,
            ));
        }
        zone_state.posture = PersistedPosture::Unfenced;
    }
    commit_durable(state)?;
    Ok(state.public_state(&zone))
}

/// Record the intent to reconcile after a restart or a fenced transfer.
///
/// The accepted lower bound is checked against what the broker holds, so a
/// resynchronization cannot install a cursor behind the projection it already
/// accepted; a later snapshot carries the same floor forward.
fn resynchronize_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &ResynchronizeRequest,
) -> Reply<ZoneAuthorityState> {
    let zone = admit_message(state, session, zone, &request.store_incarnation)?;
    {
        let Some(zone_state) = state.durable.zones.get_mut(&zone) else {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_UNKNOWN_TRANSACTION,
                AdmissionStage::Recover,
                RefusalReason::UnprovenEffect,
            ));
        };
        if request.accepted_floor.sequence < zone_state.accepted.sequence
            || (request.accepted_floor == zone_state.accepted
                && request.accepted_floor.digest != zone_state.accepted.digest)
        {
            return Err(fence(
                state,
                &zone,
                PUBLICATION_STALE_PREDECESSOR,
                AdmissionStage::Recover,
                RefusalReason::StaleAuthority,
            ));
        }
        zone_state.transactions.insert(request.transaction.to_string());
        zone_state.posture = PersistedPosture::Reconciling;
    }
    commit_durable(state)?;
    Ok(state.public_state(&zone))
}

// ---------------------------------------------------------------------------
// The control lane
// ---------------------------------------------------------------------------

/// Serve one bounded control action.
///
/// The fence blocks new ordinary use, not the actions needed to end it, so this
/// lane stays serviceable while the Zone is fenced. It admits only actions
/// bound to a transaction identity this broker durably saw or to an effect in
/// its journal, and only kinds whose lifecycle stage does not admit new use -
/// so it can reduce use or recover known state and can never create a binding,
/// a consumer-serving child privilege, or a source claim.
fn control_action_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &ControlActionRequest,
) -> Reply<AuthorityPublicationResponse> {
    let zone = admit_zone(state, session, zone)?;
    if request.kind.stage().admits_new_use() {
        // Structural guard: the enumeration cannot currently express a
        // granting control kind, and this is where that fact is enforced rather
        // than assumed.
        return Err(refuse(
            state,
            &zone,
            PUBLICATION_CONTROL_NOT_BOUND,
            request.kind.stage(),
            RefusalReason::UntrustedImplementation,
            state.public_state(&zone).is_fenced(),
        ));
    }
    let Some(zone_state) = state.durable.zones.get(&zone) else {
        return Err(refuse(
            state,
            &zone,
            PUBLICATION_CONTROL_NOT_BOUND,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
            true,
        ));
    };
    let known_transaction = zone_state
        .transactions
        .contains(&request.transaction.to_string());
    if !known_transaction {
        return Err(refuse(
            state,
            &zone,
            PUBLICATION_CONTROL_NOT_BOUND,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
            state.public_state(&zone).is_fenced(),
        ));
    }
    let Some(effect_id) = request.effect.clone() else {
        // An observation of existing state: read-only, and always serviceable
        // under a fence.
        return Ok(AuthorityPublicationResponse::Progressed(
            state.public_state(&zone),
        ));
    };
    let Some(effect) = zone_state.effects.get(effect_id.as_str()) else {
        return Err(refuse(
            state,
            &zone,
            PUBLICATION_CONTROL_NOT_BOUND,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
            state.public_state(&zone).is_fenced(),
        ));
    };
    let effect = effect.clone();
    if effect.target != request.target {
        return Err(refuse(
            state,
            &zone,
            PUBLICATION_CONTROL_NOT_BOUND,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
            state.public_state(&zone).is_fenced(),
        ));
    }
    match request.kind {
        PublicationControlKind::Observe => {}
        PublicationControlKind::Revoke
        | PublicationControlKind::Stop
        | PublicationControlKind::Detach
        | PublicationControlKind::HelperDrain => {
            if let Some(record) = state
                .durable
                .zones
                .get_mut(&zone)
                .and_then(|zone_state| zone_state.effects.get_mut(effect_id.as_str()))
                && record.phase == EffectPhase::Admitted
            {
                // Closing new use of an effect that never reached exec: the
                // child cannot appear with the revoked rights, and the record
                // stays `Cancelled` until its completion is reported.
                record.phase = EffectPhase::Cancelled;
            }
        }
        PublicationControlKind::ReservationRelease => {
            let Some(zone_state) = state.durable.zones.get_mut(&zone) else {
                return Err(refuse(
                    state,
                    &zone,
                    PUBLICATION_CONTROL_NOT_BOUND,
                    AdmissionStage::Release,
                    RefusalReason::UnprovenEffect,
                    true,
                ));
            };
            let reservations = zone_state
                .reservations
                .entry(request.target.to_string())
                .or_insert(ReservationRecord {
                    transaction: request.transaction.clone(),
                    target: request.target.clone(),
                    released: false,
                });
            if !reservations.released {
                reservations.released = true;
            }
        }
    }
    commit_durable(state)?;
    if effect.phase == EffectPhase::Exited {
        Ok(AuthorityPublicationResponse::RevocationConverged(
            RevocationConvergence {
                transaction: request.transaction.clone(),
                effect: effect_id,
                target: request.target.clone(),
                proven: true,
                state: state.public_state(&zone),
            },
        ))
    } else {
        Ok(AuthorityPublicationResponse::Progressed(state.public_state(&zone)))
    }
}

// ---------------------------------------------------------------------------
// The effect journal
// ---------------------------------------------------------------------------

/// Admit one effect under the Zone's currently accepted projection.
///
/// This is the only new ordinary use the fence blocks: while the Zone is
/// fenced, reconciled, or mid-snapshot, a new effect is refused by name rather
/// than admitted against a projection that is about to change.
fn begin_effect_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &BeginEffectRequest,
) -> Reply<AuthorityPublicationResponse> {
    let zone = admit_zone(state, session, zone)?;
    {
        let Some(zone_state) = state.durable.zones.get(&zone) else {
            return Err(refuse(
                state,
                &zone,
                PUBLICATION_UNKNOWN_TRANSACTION,
                AdmissionStage::Prepare,
                RefusalReason::UnprovenEffect,
                true,
            ));
        };
        if !matches!(zone_state.posture, PersistedPosture::Unfenced) {
            return Err(refuse(
                state,
                &zone,
                PUBLICATION_FENCE_HELD,
                AdmissionStage::Prepare,
                RefusalReason::StaleAuthority,
                true,
            ));
        }
        if request.accepted != zone_state.accepted {
            return Err(refuse(
                state,
                &zone,
                PUBLICATION_STALE_PREDECESSOR,
                AdmissionStage::Prepare,
                RefusalReason::StaleAuthority,
                true,
            ));
        }
        if !zone_state.transactions.contains(&request.transaction.to_string()) {
            return Err(refuse(
                state,
                &zone,
                PUBLICATION_CONTROL_NOT_BOUND,
                AdmissionStage::Prepare,
                RefusalReason::UnprovenEffect,
                true,
            ));
        }
    }
    let Some(zone_state) = state.durable.zones.get_mut(&zone) else {
        return Err(refuse(
            state,
            &zone,
            PUBLICATION_UNKNOWN_TRANSACTION,
            AdmissionStage::Prepare,
            RefusalReason::UnprovenEffect,
            true,
        ));
    };
    zone_state.reservations.insert(
        request.target.to_string(),
        ReservationRecord {
            transaction: request.transaction.clone(),
            target: request.target.clone(),
            released: false,
        },
    );
    zone_state.effects.insert(
        request.effect.to_string(),
        EffectRecord {
            transaction: request.transaction.clone(),
            target: request.target.clone(),
            accepted: request.accepted.clone(),
            subject: request.subject.clone(),
            // Pending new use: the launch passed admission but has not been
            // released for exec, so it is not yet an already-running workload.
            phase: EffectPhase::Admitted,
            reached_exec: false,
        },
    );
    commit_durable(state)?;
    Ok(AuthorityPublicationResponse::Progressed(state.public_state(&zone)))
}

/// The child's exec-release gate.
///
/// A launch that passed `BeginEffect` but has not completed its exec and
/// registration handshake is pending new use. This gate is where that is
/// decided, and because it runs on the same serialized worker as the fence,
/// exactly one of the two happened first:
///
/// * Release authorization processed first leaves the launch accounted as a
///   pre-fence release, and the fence drains it as existing use.
/// * The fence processed first refuses the release, and the launch is cancelled
///   and reaped: the child cannot exec under its earlier `BeginEffect`.
fn release_effect_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &ReleaseEffectRequest,
) -> Reply<AuthorityPublicationResponse> {
    let zone = admit_zone(state, session, zone)?;
    let decision = {
        let Some(zone_state) = state.durable.zones.get(&zone) else {
            return Err(refuse(
                state,
                &zone,
                PUBLICATION_UNKNOWN_TRANSACTION,
                AdmissionStage::Activate,
                RefusalReason::UnprovenEffect,
                true,
            ));
        };
        let Some(effect) = zone_state.effects.get(request.effect.as_str()) else {
            return Err(refuse(
                state,
                &zone,
                PUBLICATION_UNKNOWN_TRANSACTION,
                AdmissionStage::Activate,
                RefusalReason::UnprovenEffect,
                state.public_state(&zone).is_fenced(),
            ));
        };
        if effect.accepted != request.accepted {
            return Err(refuse(
                state,
                &zone,
                PUBLICATION_STALE_PREDECESSOR,
                AdmissionStage::Activate,
                RefusalReason::StaleAuthority,
                state.public_state(&zone).is_fenced(),
            ));
        }
        if matches!(effect.phase, EffectPhase::Released) {
            // Idempotent: a retried release of an already-released launch
            // returns the same outcome rather than a second authorization.
            ReleaseOutcome::AlreadyReleased
        } else if matches!(effect.phase, EffectPhase::Exited) {
            ReleaseOutcome::Gone
        } else if matches!(zone_state.posture, PersistedPosture::Unfenced) {
            ReleaseOutcome::Authorized
        } else {
            ReleaseOutcome::CancelledByFence
        }
    };
    match decision {
        ReleaseOutcome::Authorized => {
            if let Some(effect) = state
                .durable
                .zones
                .get_mut(&zone)
                .and_then(|zone_state| zone_state.effects.get_mut(request.effect.as_str()))
            {
                effect.phase = EffectPhase::Released;
                effect.reached_exec = true;
            }
            commit_durable(state)?;
            Ok(AuthorityPublicationResponse::Progressed(state.public_state(&zone)))
        }
        ReleaseOutcome::AlreadyReleased => {
            Ok(AuthorityPublicationResponse::Progressed(state.public_state(&zone)))
        }
        ReleaseOutcome::Gone | ReleaseOutcome::CancelledByFence => {
            if matches!(decision, ReleaseOutcome::CancelledByFence) {
                if let Some(effect) = state
                    .durable
                    .zones
                    .get_mut(&zone)
                    .and_then(|zone_state| zone_state.effects.get_mut(request.effect.as_str()))
                {
                    // Setup is cancelled: the child cannot exec under its
                    // earlier BeginEffect, and the record stays unsettled until
                    // its completion is reported.
                    effect.phase = EffectPhase::Cancelled;
                    effect.reached_exec = false;
                }
                commit_durable(state)?;
            }
            Err(refuse(
                state,
                &zone,
                PUBLICATION_FENCE_HELD,
                AdmissionStage::Revoke,
                RefusalReason::StaleAuthority,
                state.public_state(&zone).is_fenced(),
            ))
        }
    }
}

/// How the exec-release gate resolved for one launch.
enum ReleaseOutcome {
    /// Release authorization won before the fence: existing use.
    Authorized,
    /// The launch was already released; a retry is idempotent.
    AlreadyReleased,
    /// The child was already gone; there is nothing left to release.
    Gone,
    /// The fence won: the launch cannot exec and is cancelled and reaped.
    CancelledByFence,
}

/// Report one child's completion from outside the worker's mailbox.
fn effect_exit_locked(
    state: &mut ProjectionWorkerState,
    zone: &str,
    session: &PublicationSession,
    request: &EffectExitRequest,
) -> Reply<AuthorityPublicationResponse> {
    let zone = admit_zone(state, session, zone)?;
    let Some(zone_state) = state.durable.zones.get(&zone) else {
        return Err(refuse(
            state,
            &zone,
            PUBLICATION_UNKNOWN_TRANSACTION,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
            true,
        ));
    };
    let Some(effect) = zone_state.effects.get(request.effect.as_str()) else {
        // A completion for an effect this broker never admitted is not proof
        // of anything, so it cannot be used to settle a reducing commit.
        return Err(refuse(
            state,
            &zone,
            PUBLICATION_UNKNOWN_TRANSACTION,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
            state.public_state(&zone).is_fenced(),
        ));
    };
    let effect = effect.clone();
    if effect.transaction != request.transaction {
        return Err(refuse(
            state,
            &zone,
            PUBLICATION_CONTROL_NOT_BOUND,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
            state.public_state(&zone).is_fenced(),
        ));
    }
    if let Some(record) = state
        .durable
        .zones
        .get_mut(&zone)
        .and_then(|zone_state| zone_state.effects.get_mut(request.effect.as_str()))
    {
        record.phase = EffectPhase::Exited;
        record.reached_exec = request.reached_exec;
    }
    commit_durable(state)?;
    if request.reached_exec {
        // `RevocationConverged` is a separate outcome from `AuthorityAccepted`:
        // the child had already released, so its end is the release evidence a
        // reducing change still owed.
        Ok(AuthorityPublicationResponse::RevocationConverged(
            RevocationConvergence {
                transaction: request.transaction.clone(),
                effect: request.effect.clone(),
                target: effect.target.clone(),
                proven: true,
                state: state.public_state(&zone),
            },
        ))
    } else {
        Ok(AuthorityPublicationResponse::Progressed(state.public_state(&zone)))
    }
}

// ---------------------------------------------------------------------------
// Prior-state evaluation
// ---------------------------------------------------------------------------

/// The wire kind, as the pure evaluator's kind.
fn mutation_kind(kind: PublicationMutationKind) -> MutationKind {
    match kind {
        PublicationMutationKind::Create => MutationKind::Create,
        PublicationMutationKind::UpdateSpec => MutationKind::UpdateSpec,
        PublicationMutationKind::UpdateMetadata => MutationKind::UpdateMetadata,
        PublicationMutationKind::Delete => MutationKind::Delete,
    }
}

/// Evaluate every row one candidate touches against the prior accepted graph.
///
/// The candidate's own rows are never added to the graph the decision reads, so
/// a `RoleBinding` that would grant the very permission the mutation needs
/// authorizes nothing: it is not in the prior state. Returns the refusal code,
/// stage, and reason of the first row refused, in candidate order.
fn evaluate_candidate(
    candidate: &[AuthorityProjectionRow],
    removed: &[ResourceRef],
    request: &PrepareChangeRequest,
    prior: &AcceptedGraph,
) -> Option<(&'static str, AdmissionStage, RefusalReason)> {
    let kind = mutation_kind(request.kind);
    let evidence = MutationSubjectEvidence::new(
        request.subject.clone(),
        TransportIdentity::Broker,
    );
    let mut targets: Vec<&ResourceRef> = candidate
        .iter()
        .map(|row| &row.resource_ref)
        .chain(removed.iter())
        .collect();
    targets.sort();
    targets.dedup();
    for target in targets {
        let mutation = GraphMutation::new(
            prior.zone().clone(),
            evidence.clone(),
            kind,
            (*target).clone(),
        );
        if let AdmissionDecision::Refused { stage, reason } =
            GraphAuthority::admit_mutation(&mutation, prior)
        {
            return Some((
                if stage == AdmissionStage::Authorize {
                    PUBLICATION_CONTROL_NOT_BOUND
                } else {
                    PUBLICATION_STALE_PREDECESSOR
                },
                stage,
                reason,
            ));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// The broker process's projection
// ---------------------------------------------------------------------------

/// The broker process's authority projection.
///
/// The projection is process-lifetime state opened once under the daemon state
/// root, exactly like the trusted-context store: dispatch routes publication
/// traffic through it, and the envelope consults it for the fence. `run_server`
/// is the only production writer; tests initialize it against a scratch root.
static AUTHORITY_PROJECTION: std::sync::OnceLock<AuthorityProjection> =
    std::sync::OnceLock::new();

/// Open the process's authority projection under the broker state root, from a
/// synchronous caller.
///
/// A projection that fails to open fails the broker closed rather than serving
/// admission out of a half-open state.
#[cfg(test)]
pub(crate) fn init_authority_projection(state_dir: &Path) -> Reply<()> {
    let projection = AuthorityProjection::open(state_dir.to_path_buf())?;
    let _ = AUTHORITY_PROJECTION.set(projection);
    Ok(())
}

/// The async twin of [`init_authority_projection`], for the dispatch path that
/// opens the projection lazily on the first publication arrival.
pub(crate) async fn init_authority_projection_async(state_dir: &Path) -> Reply<()> {
    let projection = AuthorityProjection::open_async(state_dir.to_path_buf()).await?;
    let _ = AUTHORITY_PROJECTION.set(projection);
    Ok(())
}

/// The broker process's authority projection, absent until
/// [`init_authority_projection`] runs.
pub(crate) fn authority_projection() -> Option<&'static AuthorityProjection> {
    AUTHORITY_PROJECTION.get()
}
