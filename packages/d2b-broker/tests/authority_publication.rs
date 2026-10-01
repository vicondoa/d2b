//! The broker's admitted authority projection (U7, KTD6-KTD7).
//!
//! These cases drive the real [`AuthorityProjection`] over its real durable
//! state: every message goes through the production entry points
//! (`open_session` and `serve`), the fences and the effect journal are the
//! broker's own, and a restart reopens the same state root. The plan's U7
//! scenarios are covered here - candidate rejection, the bounded control lane,
//! restart reconciliation, and the reducing-policy race - and every mutation
//! recovery boundary the coordinator can interrupt is exercised against the
//! broker it publishes to.
//!
//! The one thing the fixture supplies is the *prior accepted graph*: a snapshot
//! document carrying a real `AuthorizedRole` and a real `RoleBindingSpec`,
//! built with the contract constructors and serialized, so the graph the
//! evaluator reads is decoded from the same bytes a store would commit.

use d2b_broker::authority_projection::{AuthorityProjection, AuthorityProjectionError};
use d2b_contracts::{decode_frame, encode_frame};
use d2b_contracts_broker::broker_wire::{
    AcceptedAuthority, AuthorityCursor, AuthorityProjectionRow, AuthorityPublicationEnvelope,
    AuthorityPublicationOpen, AuthorityPublicationRequest, AuthorityPublicationResponse,
    AuthoritySnapshot, BeginEffectRequest, BeginSnapshotRequest, CancelTransactionRequest,
    CommitChangeRequest, ControlActionRequest, EffectExitRequest, EndSnapshotRequest,
    OpenPublicationSessionRequest, PrepareChangeRequest, PreparedTransaction,
    PublicationControlKind, PublicationEffectId, PublicationMutationKind, PublicationRefusal,
    PublicationSession, PublicationSessionBinding, PublicationTransactionId, ReleaseEffectRequest,
    ResynchronizeRequest, ZoneAuthorityState, MAX_PUBLICATION_CHUNK_BYTES,
    MAX_PUBLICATION_ROWS, MAX_PUBLICATION_SNAPSHOT_BYTES, PUBLICATION_CONTROL_NOT_BOUND,
    PUBLICATION_DIGEST_MISMATCH, PUBLICATION_DUPLICATE_TRANSACTION, PUBLICATION_EFFECT_UNPROVEN,
    PUBLICATION_FENCE_HELD, PUBLICATION_RECONCILIATION_REQUIRED,
    PUBLICATION_SESSION_BOUND_ELSEWHERE, PUBLICATION_SESSION_INVALID, PUBLICATION_SNAPSHOT_INCOMPLETE,
    PUBLICATION_SNAPSHOT_IN_PROGRESS, PUBLICATION_SNAPSHOT_TOO_LARGE, PUBLICATION_STALE_PREDECESSOR,
    PUBLICATION_UNKNOWN_TRANSACTION, PUBLICATION_WRONG_ZONE, publication_candidate_digest,
    publication_snapshot_bytes, publication_snapshot_digest,
};
use d2b_contracts_resource::v3::{
    AdmissionStage, AuthoritySubject, AuthoritySubjectKind, CanonicalJsonObject, DesiredDigest,
    DesiredRevision, RefusalReason, ResourceRef, ResourceTypeName, StoreIncarnation,
    ZoneDesiredSequence,
};
use d2b_contracts_zone_session::v3::role::{AuthorizedRole, RoleResourceVerb, RoleRule};
use d2b_contracts_zone_session::v3::RoleBindingSpec;
use tempfile::TempDir;

const ZONE: &str = "pubzone";
const OTHER_ZONE: &str = "otherzone";
const STORE: &str = "store-generation-1";
const OTHER_STORE: &str = "store-generation-2";

// Every publication identity is a bounded lower-hex token, so the fixtures use
// hexadecimal names and the expectations read them as such.
const TX_BOOTSTRAP: &str = "a1";
const TX_ONE: &str = "b1";
const TX_TWO: &str = "b2";
const TX_THREE: &str = "b3";
const TX_UNKNOWN: &str = "bf";
const TX_FOREIGN: &str = "be";
const EFFECT_ONE: &str = "e1";
const EFFECT_TWO: &str = "e2";
const EFFECT_UNKNOWN: &str = "ee";

// ---------------------------------------------------------------------------
// Fixture vocabulary
// ---------------------------------------------------------------------------

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("the fixture references are canonical")
}

fn incarnation(value: &str) -> StoreIncarnation {
    StoreIncarnation::parse(value).expect("the fixture incarnations are bounded tokens")
}

fn tx_id(value: &str) -> PublicationTransactionId {
    PublicationTransactionId::parse(value).expect("the fixture transaction ids are canonical")
}

fn effect_id(value: &str) -> PublicationEffectId {
    PublicationEffectId::parse(value).expect("the fixture effect ids are canonical")
}

fn sequence(value: u64) -> ZoneDesiredSequence {
    let mut out = ZoneDesiredSequence::INITIAL;
    for _ in 0..value {
        out = out.try_next().expect("the desired sequence has room");
    }
    out
}

fn revision(value: u64) -> DesiredRevision {
    let mut out = DesiredRevision::INITIAL;
    for _ in 0..value {
        out = out.try_next().expect("the desired revision has room");
    }
    out
}

/// One point on the Zone's desired sequence with the digest committed there.
fn cursor(value: u64) -> AuthorityCursor {
    AuthorityCursor {
        sequence: sequence(value),
        digest: DesiredDigest::of(format!("cursor-{value}").as_bytes()),
    }
}

fn canonical(value: &impl serde::Serialize) -> CanonicalJsonObject {
    CanonicalJsonObject::parse(&serde_json::to_vec(value).expect("the fixture row serializes"))
        .expect("the fixture row is a canonical JSON object")
}

fn projection_row(name: &str, admitted: &CanonicalJsonObject) -> AuthorityProjectionRow {
    AuthorityProjectionRow {
        resource_ref: reference(name),
        desired_revision: revision(1),
        desired_digest: DesiredDigest::of(&admitted.to_canonical_bytes()),
        admitted: admitted.clone(),
        source_uid: None,
        consumer_uid: None,
    }
}

/// The same row, published with the relationship identity the manager
/// resolved: a `Role` or `RoleBinding` never carries one, and a committed
/// binding row without it is an absence the graph refuses.
fn projection_row_with_identity(
    name: &str,
    admitted: &CanonicalJsonObject,
    source_uid: &str,
    consumer_uid: &str,
) -> AuthorityProjectionRow {
    AuthorityProjectionRow {
        source_uid: Some(uid(source_uid)),
        consumer_uid: Some(uid(consumer_uid)),
        ..projection_row(name, admitted)
    }
}

/// One committed `VolumeBinding` row: the source provider's own accepted
/// decision about a relationship, in canonical bytes.
fn volume_binding_admitted() -> CanonicalJsonObject {
    use d2b_contracts_resource::v3::volume::AttachmentAccess;
    use d2b_contracts_resource::v3::{
        BindingArbitration, BindingRealizationFacet, BindingSourceDecision, RequestedRights,
        VolumeBindingSpec,
    };

    let decision = BindingSourceDecision::new(
        vec![RequestedRights::Consume],
        BindingArbitration::Shared,
        vec![BindingRealizationFacet::FilesystemPresentation],
    )
    .expect("the source decision is well formed");
    let spec = VolumeBindingSpec::new(
        reference("Volume/data"),
        reference("Guest/work"),
        "root",
        AttachmentAccess::ReadWrite,
        "/var/lib/d2b/volumes/data",
        decision,
    )
    .expect("the binding row is well formed");
    canonical(&spec)
}

/// The published row, resolved: the two uids its `BindingKey` folds in.
fn volume_binding_row(source_uid: &str, consumer_uid: &str) -> AuthorityProjectionRow {
    projection_row_with_identity(
        "VolumeBinding/data",
        &volume_binding_admitted(),
        source_uid,
        consumer_uid,
    )
}

fn uid(value: &str) -> d2b_contracts_resource::v3::ResourceUid {
    d2b_contracts_resource::v3::ResourceUid::parse(value).expect("the fixture uid is canonical")
}

/// The `Role/reader` row: every CRUD verb on a `Process`, nothing else.
fn reader_role() -> AuthorityProjectionRow {
    let rule = RoleRule::new(
        vec![ResourceTypeName::parse("Process").expect("Process is a standard type")],
        vec![
            RoleResourceVerb::Create,
            RoleResourceVerb::UpdateSpec,
            RoleResourceVerb::UpdateMetadata,
            RoleResourceVerb::Delete,
        ],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("the reader rule is bounded and non-empty");
    let role = AuthorizedRole::new(vec![rule], Vec::new()).expect("the reader role is bounded");
    projection_row("Role/reader", &canonical(&role))
}

/// The `RoleBinding/shell` row: `Process/shell` draws on `Role/reader`.
fn shell_binding() -> AuthorityProjectionRow {
    let binding = RoleBindingSpec::with_facets(
        reference("Role/reader"),
        vec![reference("Process/shell")],
        None,
        None,
        Vec::new(),
        Vec::new(),
        Vec::new(),
        None,
    )
    .expect("the shell binding is bounded");
    projection_row("RoleBinding/shell", &canonical(&binding))
}

/// An ordinary, non-authority row: its bytes are never decoded as authority.
fn process_row(name: &str) -> AuthorityProjectionRow {
    projection_row(
        &format!("Process/{name}"),
        &canonical(&serde_json::json!({ "declared": name })),
    )
}

fn bootstrap() -> AuthoritySubject {
    AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)
}

fn shell() -> AuthoritySubject {
    AuthoritySubject::named(AuthoritySubjectKind::Process, reference("Process/shell"))
}

fn worker() -> ResourceRef {
    reference("Process/worker")
}

// ---------------------------------------------------------------------------
// Assertions
// ---------------------------------------------------------------------------

/// Take the one typed refusal a refused exchange carries.
fn refusal(error: AuthorityProjectionError) -> PublicationRefusal {
    error
        .refusal()
        .expect("the broker answered with a typed refusal")
        .clone()
}

fn assert_refused(
    got: &PublicationRefusal,
    code: &str,
    stage: AdmissionStage,
    reason: RefusalReason,
    fenced: bool,
) {
    assert_eq!(got.code, code, "refusal code");
    assert_eq!(got.stage, stage, "refusal stage");
    assert_eq!(got.reason, reason, "refusal reason");
    assert_eq!(got.fenced, fenced, "refusal fence flag");
}

fn accepted_sequence(state: &ZoneAuthorityState) -> Option<u64> {
    state.accepted().map(|cursor| cursor.sequence.get())
}

fn posture(state: &ZoneAuthorityState) -> &'static str {
    match state {
        ZoneAuthorityState::Unprovisioned => "unprovisioned",
        ZoneAuthorityState::Unfenced { .. } => "unfenced",
        ZoneAuthorityState::Fenced { .. } => "fenced",
        ZoneAuthorityState::SnapshotInProgress { .. } => "snapshot-in-progress",
        ZoneAuthorityState::Reconciling { .. } => "reconciling",
    }
}

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

struct Harness {
    // The projection is dropped before the state root it writes to.
    projection: AuthorityProjection,
    root: TempDir,
    session: PublicationSession,
    binding: PublicationSessionBinding,
}

impl Harness {
    async fn start() -> Self {
        let root = tempfile::tempdir().expect("a scratch state root");
        let projection = AuthorityProjection::open_async(root.path())
            .await
            .expect("the projection opens");
        let mut harness = Self {
            projection,
            root,
            session: PublicationSession::parse("unset-session")
                .expect("the placeholder token is canonical"),
            binding: PublicationSessionBinding {
                zone: ZONE.to_owned(),
                store_incarnation: incarnation(STORE),
                broker_epoch: 0,
                initiating_subject: bootstrap(),
                accepted: AuthorityCursor::initial(),
            },
        };
        harness.reopen().await;
        harness
    }

    /// Re-establish the Zone session against the cursor the broker holds now.
    async fn reopen(&mut self) {
        let accepted = self.projection.status(ZONE).await.accepted().cloned();
        let response = self
            .projection
            .open_session(AuthorityPublicationOpen {
                request: OpenPublicationSessionRequest {
                    zone: ZONE.to_owned(),
                    store_incarnation: incarnation(STORE),
                    broker_epoch: 0,
                    initiating_subject: bootstrap(),
                    accepted: accepted.unwrap_or_else(AuthorityCursor::initial),
                },
            })
            .await
            .expect("the broker mints a session");
        self.session = response.session;
        self.binding = response.binding;
    }

    fn envelope(&self, request: AuthorityPublicationRequest) -> AuthorityPublicationEnvelope {
        AuthorityPublicationEnvelope {
            zone: ZONE.to_owned(),
            session: self.session.clone(),
            request,
        }
    }

    async fn serve(
        &self,
        request: AuthorityPublicationRequest,
    ) -> Result<AuthorityPublicationResponse, AuthorityProjectionError> {
        self.projection.serve(&self.envelope(request)).await
    }

    async fn status(&self) -> ZoneAuthorityState {
        self.projection.status(ZONE).await
    }

    async fn begin_snapshot(
        &self,
        id: &str,
        cursor: AuthorityCursor,
        total_chunks: u32,
        total_bytes: u64,
    ) -> Result<AuthorityPublicationResponse, AuthorityProjectionError> {
        self.serve(AuthorityPublicationRequest::BeginSnapshot(
            BeginSnapshotRequest {
                transaction: tx_id(id),
                store_incarnation: incarnation(STORE),
                cursor,
                total_chunks,
                total_bytes,
            },
        ))
        .await
    }

    async fn chunk(
        &self,
        id: &str,
        ordinal: u32,
        total_chunks: u32,
        payload: Vec<u8>,
    ) -> Result<AuthorityPublicationResponse, AuthorityProjectionError> {
        self.serve(AuthorityPublicationRequest::SnapshotChunk(
            d2b_contracts_broker::broker_wire::SnapshotChunkRequest {
                transaction: tx_id(id),
                ordinal,
                total_chunks,
                payload,
            },
        ))
        .await
    }

    async fn end_snapshot(
        &self,
        id: &str,
        total_chunks: u32,
        digest: DesiredDigest,
    ) -> Result<AuthorityPublicationResponse, AuthorityProjectionError> {
        self.serve(AuthorityPublicationRequest::EndSnapshot(
            EndSnapshotRequest {
                transaction: tx_id(id),
                total_chunks,
                digest,
            },
        ))
        .await
    }

    /// Transfer one whole document in bounded chunks, then re-establish the
    /// session: installing a snapshot moves the accepted cursor the session is
    /// bound to, exactly as a manager whose own view has moved on must.
    async fn publish_snapshot(
        &mut self,
        id: &str,
        cursor: AuthorityCursor,
        rows: Vec<AuthorityProjectionRow>,
        outstanding: Option<&str>,
    ) -> Result<ZoneAuthorityState, AuthorityProjectionError> {
        let snapshot = self.snapshot(cursor, rows, outstanding);
        let bytes = publication_snapshot_bytes(&snapshot);
        let digest = publication_snapshot_digest(&snapshot);
        let total_chunks = bytes.len().div_ceil(MAX_PUBLICATION_CHUNK_BYTES).max(1) as u32;
        self.begin_snapshot(id, snapshot.cursor.clone(), total_chunks, bytes.len() as u64)
            .await?;
        for ordinal in 0..total_chunks {
            let start = ordinal as usize * MAX_PUBLICATION_CHUNK_BYTES;
            let end = bytes.len().min(start + MAX_PUBLICATION_CHUNK_BYTES);
            self.chunk(id, ordinal, total_chunks, bytes[start..end].to_vec())
                .await?;
        }
        let ended = self.end_snapshot(id, total_chunks, digest).await?;
        self.reopen().await;
        Ok(match ended {
            AuthorityPublicationResponse::Progressed(state) => state,
            other => panic!("the snapshot did not progress: {other:?}"),
        })
    }

    fn snapshot(
        &self,
        cursor: AuthorityCursor,
        rows: Vec<AuthorityProjectionRow>,
        outstanding: Option<&str>,
    ) -> AuthoritySnapshot {
        AuthoritySnapshot {
            zone: ZONE.to_owned(),
            store_incarnation: incarnation(STORE),
            cursor,
            root_subject: bootstrap(),
            rows,
            outstanding: outstanding.map(tx_id),
        }
    }

    /// Install the bootstrap graph `Process/shell` mutates under.
    async fn install_reader_grant(&mut self) {
        self.publish_snapshot(
            TX_BOOTSTRAP,
            cursor(1),
            vec![reader_role(), shell_binding()],
            None,
        )
        .await
        .expect("the bootstrap graph installs");
    }

    async fn prepare(&mut self, request: PrepareChangeRequest) -> PreparedTransaction {
        self.reopen().await;
        match self
            .serve(AuthorityPublicationRequest::PrepareChange(request))
            .await
        {
            Ok(AuthorityPublicationResponse::Prepared(prepared)) => prepared,
            other => panic!("the candidate did not prepare: {other:?}"),
        }
    }

    async fn prepare_refused(&mut self, request: PrepareChangeRequest) -> PublicationRefusal {
        self.reopen().await;
        match self
            .serve(AuthorityPublicationRequest::PrepareChange(request))
            .await
        {
            Ok(AuthorityPublicationResponse::Prepared(prepared)) => {
                panic!("the candidate prepared when it must not: {prepared:?}")
            }
            Ok(other) => panic!("the candidate answered: {other:?}"),
            Err(error) => refusal(error),
        }
    }

    #[allow(clippy::too_many_arguments, reason = "the fixture mirrors the wire request")]
    fn prepare_request(
        &self,
        id: &str,
        expected: AuthorityCursor,
        committed: AuthorityCursor,
        kind: PublicationMutationKind,
        candidate: Vec<AuthorityProjectionRow>,
        removed: Vec<ResourceRef>,
        subject: AuthoritySubject,
    ) -> PrepareChangeRequest {
        PrepareChangeRequest {
            transaction: tx_id(id),
            store_incarnation: incarnation(STORE),
            expected,
            committed,
            digest: publication_candidate_digest(&candidate, &removed),
            subject,
            kind,
            candidate,
            removed,
        }
    }

    fn commit_request(
        &self,
        id: &str,
        expected: AuthorityCursor,
        committed: AuthorityCursor,
        rows: Vec<AuthorityProjectionRow>,
        removed: Vec<ResourceRef>,
    ) -> CommitChangeRequest {
        CommitChangeRequest {
            transaction: tx_id(id),
            store_incarnation: incarnation(STORE),
            expected,
            committed,
            digest: publication_candidate_digest(&rows, &removed),
            rows,
            removed,
        }
    }

    /// Commit one candidate and return the accepted revision.
    async fn commit(&mut self, request: CommitChangeRequest) -> AcceptedAuthority {
        self.reopen().await;
        let accepted = match self
            .serve(AuthorityPublicationRequest::CommitChange(request))
            .await
        {
            Ok(AuthorityPublicationResponse::Accepted(accepted)) => accepted,
            other => panic!("the commit was not accepted: {other:?}"),
        };
        // Installing the revision moves the cursor the session is bound to.
        self.reopen().await;
        accepted
    }

    fn begin_effect_request(
        &self,
        effect: &str,
        id: &str,
        accepted: AuthorityCursor,
        target: &str,
    ) -> BeginEffectRequest {
        BeginEffectRequest {
            effect: effect_id(effect),
            transaction: tx_id(id),
            accepted,
            subject: shell(),
            target: reference(target),
        }
    }

    fn control(
        &self,
        id: &str,
        effect: Option<&str>,
        target: &str,
        kind: PublicationControlKind,
    ) -> ControlActionRequest {
        ControlActionRequest {
            transaction: tx_id(id),
            effect: effect.map(effect_id),
            target: reference(target),
            kind,
        }
    }

    /// Close the projection and reopen the same durable state.
    async fn restart(&mut self) {
        let root = self.root.path().to_path_buf();
        let throwaway = tempfile::tempdir().expect("a throwaway root");
        let placeholder = AuthorityProjection::open_async(throwaway.path())
            .await
            .expect("a throwaway projection opens");
        self.projection = placeholder;
        self.projection = AuthorityProjection::open_async(root)
            .await
            .expect("the projection reopens over its own durable state");
        self.reopen().await;
    }
}

// ---------------------------------------------------------------------------
// A Zone the broker has never published for
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_unpublished_zone_reports_no_authority() {
    let root = tempfile::tempdir().expect("a scratch state root");
    let projection = AuthorityProjection::open_async(root.path())
        .await
        .expect("the projection opens");

    for label in [ZONE, OTHER_ZONE] {
        let state = projection.status(label).await;
        assert_eq!(state, ZoneAuthorityState::Unprovisioned, "{label}");
        assert!(!state.is_provisioned(), "{label} holds no authority");
        assert!(state.accepted().is_none(), "{label} has no accepted cursor");
        assert!(state.is_fenced(), "{label} admits nothing before its first publication");
    }
    assert_eq!(projection.epoch().await, 1, "the first open mints epoch one");
}

/// A published binding row rebuilds its accepted source under the exact key
/// its resolved identity produces, through the real publication path.
///
/// This is the end-to-end half of the identity round trip: the manager
/// resolves the two uids, the transfer carries them, the projection persists
/// them, and the graph an effect admission reads finds the source provider's
/// own accepted decision under the key an invocation would name.
#[tokio::test]
async fn a_published_binding_row_is_readable_as_an_accepted_source() {
    use d2b_contracts_resource::v3::{BindingKey, BindingKind, BindingSlot, RequestedRights};

    const SOURCE_UID: &str = "11111111-1111-4111-8111-111111111111";
    const CONSUMER_UID: &str = "22222222-2222-4222-8222-222222222222";

    let mut harness = Harness::start().await;
    harness
        .publish_snapshot(
            TX_BOOTSTRAP,
            cursor(1),
            vec![
                reader_role(),
                shell_binding(),
                volume_binding_row(SOURCE_UID, CONSUMER_UID),
            ],
            None,
        )
        .await
        .expect("the graph with a binding row installs");

    let graph = harness
        .projection
        .accepted_graph(ZONE)
        .await
        .expect("the Zone's published rows decode into a graph");
    let key = BindingKey::new(
        d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("the fixture Zone is canonical"),
        BindingKind::Volume,
        reference("Volume/data"),
        uid(SOURCE_UID),
        reference("Guest/work"),
        uid(CONSUMER_UID),
        BindingSlot::parse("root").expect("the fixture slot is a bounded token"),
    )
    .expect("the relationship key is well formed");
    let source = graph
        .source(&key)
        .expect("the published row is the source provider's accepted decision");
    assert_eq!(
        source.admission().admitted_rights(),
        &[RequestedRights::Consume],
        "the rebuilt source carries the row's own accepted rights"
    );
    assert!(
        graph.role(&reference("Role/reader")).is_some(),
        "the graph still carries the Role the fence decisions read"
    );
}

// ---------------------------------------------------------------------------
// The session is the fence in value form
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_assembled_session_is_refused() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;

    let assembled = PublicationSession::parse("pub-0000-assembled").expect("canonical token");
    let error = harness
        .projection
        .serve(&AuthorityPublicationEnvelope {
            zone: ZONE.to_owned(),
            session: assembled,
            request: AuthorityPublicationRequest::ControlAction(harness.control(
                TX_BOOTSTRAP,
                None,
                "Process/worker",
                PublicationControlKind::Observe,
            )),
        })
        .await
        .expect_err("a value the broker never minted names no session it admits");
    let refused = refusal(error);
    assert_refused(
        &refused,
        PUBLICATION_SESSION_INVALID,
        AdmissionStage::Authorize,
        RefusalReason::IdentityNotAuthorized,
        false,
    );

    // The minted session is served, so the refusal is about the token and not
    // about the message.
    harness
        .serve(AuthorityPublicationRequest::ControlAction(harness.control(
            TX_BOOTSTRAP,
            None,
            "Process/worker",
            PublicationControlKind::Observe,
        )))
        .await
        .expect("the broker's own session is served");
}

#[tokio::test]
async fn a_session_does_not_cross_zones_or_store_generations() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;
    let minted = harness.session.clone();

    // The same token presented for another Zone names a record this broker
    // never opened a session for.
    let error = harness
        .projection
        .serve(&AuthorityPublicationEnvelope {
            zone: OTHER_ZONE.to_owned(),
            session: minted.clone(),
            request: AuthorityPublicationRequest::ControlAction(harness.control(
                TX_BOOTSTRAP,
                None,
                "Process/worker",
                PublicationControlKind::Observe,
            )),
        })
        .await
        .expect_err("a session minted for one Zone cannot drive another");
    assert_refused(
        &refusal(error),
        PUBLICATION_SESSION_INVALID,
        AdmissionStage::Authorize,
        RefusalReason::IdentityNotAuthorized,
        true,
    );

    // A Zone label that is not canonical is refused by name, before a record
    // is ever created for it.
    let error = harness
        .projection
        .open_session(AuthorityPublicationOpen {
            request: OpenPublicationSessionRequest {
                zone: "Pub Zone".to_owned(),
                store_incarnation: incarnation(STORE),
                broker_epoch: 0,
                initiating_subject: bootstrap(),
                accepted: AuthorityCursor::initial(),
            },
        })
        .await
        .expect_err("a non-canonical Zone label is not a Zone");
    assert_refused(
        &refusal(error),
        PUBLICATION_WRONG_ZONE,
        AdmissionStage::Authorize,
        RefusalReason::IdentityNotAuthorized,
        true,
    );
    assert_eq!(
        harness.projection.status("Pub Zone").await,
        ZoneAuthorityState::Unprovisioned,
        "a refused open creates no authority"
    );
}

#[tokio::test]
async fn a_session_is_bound_to_the_accepted_cursor_it_was_minted_at() {
    let mut harness = Harness::start().await;
    let before = harness.session.clone();

    // Move the accepted cursor without re-establishing the session: the broker
    // still holds the binding it minted at the previous cursor.
    let snapshot = harness.snapshot(cursor(1), vec![reader_role(), shell_binding()], None);
    let bytes = publication_snapshot_bytes(&snapshot);
    harness
        .begin_snapshot(TX_BOOTSTRAP, snapshot.cursor.clone(), 1, bytes.len() as u64)
        .await
        .expect("the transfer opens");
    harness
        .chunk(TX_BOOTSTRAP, 0, 1, bytes)
        .await
        .expect("the chunk is accepted");
    harness
        .end_snapshot(TX_BOOTSTRAP, 1, publication_snapshot_digest(&snapshot))
        .await
        .expect("the bootstrap graph installs");

    // The pre-snapshot token is still this broker's own derivation, but it is
    // bound to a cursor the broker has moved past.
    let error = harness
        .projection
        .serve(&AuthorityPublicationEnvelope {
            zone: ZONE.to_owned(),
            session: before.clone(),
            request: AuthorityPublicationRequest::ControlAction(harness.control(
                TX_BOOTSTRAP,
                None,
                "Process/worker",
                PublicationControlKind::Observe,
            )),
        })
        .await
        .expect_err("a session bound to a superseded cursor is not served");
    assert_refused(
        &refusal(error),
        PUBLICATION_SESSION_BOUND_ELSEWHERE,
        AdmissionStage::Authorize,
        RefusalReason::IdentityNotAuthorized,
        false,
    );
    harness.reopen().await;
    assert_eq!(
        harness.binding.accepted,
        cursor(1),
        "the re-established session is bound to the accepted cursor"
    );
    assert_ne!(
        before.as_str(),
        harness.session.as_str(),
        "the session is derived from the accepted cursor"
    );
    harness
        .serve(AuthorityPublicationRequest::ControlAction(harness.control(
            TX_BOOTSTRAP,
            None,
            "Process/worker",
            PublicationControlKind::Observe,
        )))
        .await
        .expect("the re-established session is served");
}

#[tokio::test]
async fn a_publication_frame_round_trips_and_is_served() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;

    let envelope = harness.envelope(AuthorityPublicationRequest::PrepareChange(
        harness.prepare_request(
            TX_ONE,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ),
    ));
    let frame = encode_frame(&envelope).expect("a publication envelope encodes");
    let decoded: AuthorityPublicationEnvelope =
        decode_frame("AuthorityPublicationEnvelope", &frame).expect("and decodes back");
    assert_eq!(decoded, envelope, "the frame preserves every field");

    let prepared = match harness
        .projection
        .serve(&decoded)
        .await
        .expect("the decoded envelope is served")
    {
        AuthorityPublicationResponse::Prepared(prepared) => prepared,
        other => panic!("the decoded envelope did not prepare: {other:?}"),
    };

    let response = AuthorityPublicationResponse::Prepared(prepared.clone());
    let frame = encode_frame(&response).expect("the response encodes");
    let decoded: AuthorityPublicationResponse =
        decode_frame("AuthorityPublicationResponse", &frame).expect("and decodes back");
    assert_eq!(decoded, response, "the accepted fence round trips whole");
}

// ---------------------------------------------------------------------------
// A candidate authorizes nothing by naming itself
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_candidate_cannot_introduce_the_grant_that_authorizes_it() {
    let mut harness = Harness::start().await;
    // The accepted graph starts empty: the deployment root and nothing else.
    harness
        .publish_snapshot(TX_BOOTSTRAP, cursor(1), Vec::new(), None)
        .await
        .expect("an empty bootstrap graph installs");

    // The candidate introduces the very RoleBinding that would authorize it.
    let grant = vec![reader_role(), shell_binding()];
    let error = harness
        .prepare_refused(harness.prepare_request(
            TX_ONE,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            grant.clone(),
            Vec::new(),
            shell(),
        ))
        .await;
    let refused = error;
    assert_eq!(refused.stage, AdmissionStage::Authorize, "refusal stage");
    assert_eq!(
        refused.reason,
        RefusalReason::IdentityNotAuthorized,
        "the prior state holds no grant for this subject"
    );
    assert!(refused.fenced, "a refused candidate leaves the Zone fenced");

    let state = harness.status().await;
    assert!(state.is_fenced(), "the Zone is {}", posture(&state));
    assert_eq!(
        accepted_sequence(&state),
        Some(1),
        "the refused candidate moved nothing"
    );

    // The refused grant is not in the projection: resynchronizing without it
    // leaves the subject exactly as ungranted as it was.
    harness
        .serve(AuthorityPublicationRequest::Resynchronize(
            ResynchronizeRequest {
                transaction: tx_id(TX_THREE),
                store_incarnation: incarnation(STORE),
                accepted_floor: cursor(1),
                cursor: cursor(1),
            },
        ))
        .await
        .expect("the reconciliation is accepted");
    harness
        .publish_snapshot(TX_THREE, cursor(1), Vec::new(), None)
        .await
        .expect("the Zone reconciles to an empty graph");

    let error = harness
        .prepare_refused(harness.prepare_request(
            TX_TWO,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    assert_eq!(
        error.reason,
        RefusalReason::IdentityNotAuthorized,
        "the refused candidate installed no grant"
    );

    // The deployment root can still mutate, so the refusal is about this
    // subject and not about a broken graph.
    harness
        .publish_snapshot(TX_THREE, cursor(1), Vec::new(), None)
        .await
        .expect("the Zone reconciles after the refusal");
    harness
        .prepare(harness.prepare_request(
            TX_TWO,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            grant,
            Vec::new(),
            bootstrap(),
        ))
        .await;
}

#[tokio::test]
async fn a_snapshot_document_for_another_zone_is_refused() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;
    let before = harness.status().await;

    let mut snapshot = harness.snapshot(cursor(2), vec![process_row("worker")], None);
    snapshot.zone = OTHER_ZONE.to_owned();
    let bytes = publication_snapshot_bytes(&snapshot);
    harness
        .begin_snapshot(TX_ONE, snapshot.cursor.clone(), 1, bytes.len() as u64)
        .await
        .expect("the transfer opens");
    harness
        .chunk(TX_ONE, 0, 1, bytes)
        .await
        .expect("the chunk is accepted");
    let error = harness
        .end_snapshot(TX_ONE, 1, publication_snapshot_digest(&snapshot))
        .await
        .expect_err("a document naming another Zone is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_WRONG_ZONE,
        AdmissionStage::Recover,
        RefusalReason::StoreIncarnationMismatch,
        true,
    );

    let after = harness.status().await;
    assert!(after.is_fenced(), "the Zone is {}", posture(&after));
    assert_eq!(
        accepted_sequence(&after),
        accepted_sequence(&before),
        "the refused document installed nothing"
    );
}

#[tokio::test]
async fn a_stale_predecessor_is_refused() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;

    // A change that does not name the accepted cursor exactly is stale.
    let error = harness
        .prepare_refused(harness.prepare_request(
            TX_ONE,
            cursor(7),
            cursor(8),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    assert_refused(
        &error,
        PUBLICATION_STALE_PREDECESSOR,
        AdmissionStage::Authorize,
        RefusalReason::StaleAuthority,
        true,
    );

    // A change that would not move the sequence forward is stale too.
    harness
        .publish_snapshot(TX_THREE, cursor(1), Vec::new(), None)
        .await
        .expect("the Zone reconciles");
    let error = harness
        .prepare_refused(harness.prepare_request(
            TX_TWO,
            cursor(1),
            cursor(1),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    assert_refused(
        &error,
        PUBLICATION_STALE_PREDECESSOR,
        AdmissionStage::Authorize,
        RefusalReason::StaleAuthority,
        true,
    );
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(1),
        "neither stale change moved the accepted cursor"
    );
}

#[tokio::test]
async fn a_missing_or_out_of_order_chunk_leaves_the_zone_fenced() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;
    let snapshot = harness.snapshot(cursor(2), vec![process_row("worker")], None);
    let bytes = publication_snapshot_bytes(&snapshot);
    let digest = publication_snapshot_digest(&snapshot);
    assert!(!bytes.is_empty(), "the fixture document is not empty");

    // A document that ended before all its chunks arrived.
    harness
        .begin_snapshot(TX_ONE, snapshot.cursor.clone(), 2, bytes.len() as u64)
        .await
        .expect("the transfer opens");
    harness
        .chunk(TX_ONE, 0, 2, bytes.clone())
        .await
        .expect("the first chunk is accepted");
    let error = harness
        .end_snapshot(TX_ONE, 2, digest.clone())
        .await
        .expect_err("a document that ended early is not a partial success");
    assert_refused(
        &refusal(error),
        PUBLICATION_SNAPSHOT_INCOMPLETE,
        AdmissionStage::Recover,
        RefusalReason::UnprovenEffect,
        true,
    );
    assert_eq!(posture(&harness.status().await), "reconciling");

    // A gap: the first chunk never arrived.
    harness
        .publish_snapshot(TX_THREE, cursor(1), Vec::new(), None)
        .await
        .expect("the Zone reconciles");
    harness
        .begin_snapshot(TX_TWO, snapshot.cursor.clone(), 2, bytes.len() as u64)
        .await
        .expect("the transfer reopens");
    let error = harness
        .chunk(TX_TWO, 1, 2, bytes.clone())
        .await
        .expect_err("the second ordinal is not the next one");
    assert_refused(
        &refusal(error),
        PUBLICATION_SNAPSHOT_INCOMPLETE,
        AdmissionStage::Recover,
        RefusalReason::UnprovenEffect,
        true,
    );
    assert_eq!(posture(&harness.status().await), "reconciling");

    // A transfer that declares no chunks at all is above the declared floor.
    harness
        .publish_snapshot(TX_THREE, cursor(1), Vec::new(), None)
        .await
        .expect("the Zone reconciles");
    let error = harness
        .begin_snapshot(TX_TWO, snapshot.cursor.clone(), 0, 0)
        .await
        .expect_err("a zero-chunk document is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_SNAPSHOT_TOO_LARGE,
        AdmissionStage::Recover,
        RefusalReason::LimitExceedsCeiling,
        true,
    );
}

#[tokio::test]
async fn an_oversized_snapshot_is_refused_on_every_declared_ceiling() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;
    let snapshot = harness.snapshot(cursor(2), vec![process_row("worker")], None);

    // The declared byte ceiling is checked before the first chunk.
    let error = harness
        .begin_snapshot(
            TX_ONE,
            snapshot.cursor.clone(),
            1,
            MAX_PUBLICATION_SNAPSHOT_BYTES + 1,
        )
        .await
        .expect_err("a document over the byte ceiling is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_SNAPSHOT_TOO_LARGE,
        AdmissionStage::Recover,
        RefusalReason::LimitExceedsCeiling,
        true,
    );

    // So is one chunk over the chunk ceiling.
    harness
        .publish_snapshot(TX_THREE, cursor(1), Vec::new(), None)
        .await
        .expect("the Zone reconciles");
    harness
        .begin_snapshot(TX_TWO, snapshot.cursor.clone(), 1, 1)
        .await
        .expect("the transfer opens");
    let error = harness
        .chunk(
            TX_TWO,
            0,
            1,
            vec![0; MAX_PUBLICATION_CHUNK_BYTES + 1],
        )
        .await
        .expect_err("an oversized chunk is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_SNAPSHOT_TOO_LARGE,
        AdmissionStage::Recover,
        RefusalReason::LimitExceedsCeiling,
        true,
    );

    // And so is a complete, correctly digested document over the row ceiling.
    harness
        .publish_snapshot(TX_THREE, cursor(1), Vec::new(), None)
        .await
        .expect("the Zone reconciles");
    let rows = (0..=MAX_PUBLICATION_ROWS)
        .map(|index| process_row(&format!("p{index}")))
        .collect::<Vec<_>>();
    assert!(rows.len() > MAX_PUBLICATION_ROWS, "the fixture is over the ceiling");
    let big = harness.snapshot(cursor(2), rows, None);
    let big_bytes = publication_snapshot_bytes(&big);
    let big_digest = publication_snapshot_digest(&big);
    let total = big_bytes.len().div_ceil(MAX_PUBLICATION_CHUNK_BYTES).max(1) as u32;
    harness
        .begin_snapshot(TX_TWO, big.cursor.clone(), total, big_bytes.len() as u64)
        .await
        .expect("the oversized transfer opens");
    for ordinal in 0..total {
        let start = ordinal as usize * MAX_PUBLICATION_CHUNK_BYTES;
        let end = big_bytes.len().min(start + MAX_PUBLICATION_CHUNK_BYTES);
        harness
            .chunk(TX_TWO, ordinal, total, big_bytes[start..end].to_vec())
            .await
            .expect("each in-bound chunk is accepted");
    }
    let error = harness
        .end_snapshot(TX_TWO, total, big_digest)
        .await
        .expect_err("a document over the row ceiling installs nothing");
    assert_refused(
        &refusal(error),
        PUBLICATION_SNAPSHOT_TOO_LARGE,
        AdmissionStage::Recover,
        RefusalReason::LimitExceedsCeiling,
        true,
    );
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(1),
        "the oversized document installed nothing"
    );
}

#[tokio::test]
async fn a_duplicate_conflicting_transaction_is_refused() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;

    let prepared = harness
        .prepare(harness.prepare_request(
            TX_ONE,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    assert_eq!(prepared.transaction, tx_id(TX_ONE));
    assert!(!prepared.reducing, "adding a row is not a reducing change");

    // A second identity under one held fence is a conflicting declaration.
    let error = harness
        .prepare_refused(harness.prepare_request(
            TX_TWO,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    assert_refused(
        &error,
        PUBLICATION_DUPLICATE_TRANSACTION,
        AdmissionStage::Authorize,
        RefusalReason::ConflictingDeclaration,
        true,
    );
    match harness.status().await {
        ZoneAuthorityState::Fenced { transaction, .. } => {
            assert_eq!(transaction, tx_id(TX_ONE), "the held fence is unchanged");
        }
        other => panic!("the Zone is {}", posture(&other)),
    }

    // Replaying the exact candidate is idempotent: the same fence comes back.
    let replayed = harness
        .prepare(harness.prepare_request(
            TX_ONE,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    assert_eq!(replayed, prepared, "an exact replay returns the same fence");

    // The same identity with different committed bytes is a different
    // candidate, not a second transaction.
    harness.reopen().await;
    let error = harness
        .serve(AuthorityPublicationRequest::CommitChange(
            harness.commit_request(
                TX_ONE,
                cursor(1),
                cursor(2),
                vec![process_row("other")],
                Vec::new(),
            ),
        ))
        .await
        .expect_err("a commit may only install the bytes the fence was prepared for");
    assert_refused(
        &refusal(error),
        PUBLICATION_DUPLICATE_TRANSACTION,
        AdmissionStage::Authorize,
        RefusalReason::ConflictingDeclaration,
        true,
    );
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(1),
        "no conflicting commit advanced the projection"
    );
}

#[tokio::test]
async fn a_declared_digest_must_match_the_committed_bytes() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;

    let mut request = harness.prepare_request(
        TX_ONE,
        cursor(1),
        cursor(2),
        PublicationMutationKind::Create,
        vec![process_row("worker")],
        Vec::new(),
        shell(),
    );
    request.digest = DesiredDigest::of(b"not-the-candidate");
    let error = harness
        .prepare_refused(request)
        .await;
    assert_refused(
        &error,
        PUBLICATION_DIGEST_MISMATCH,
        AdmissionStage::Authorize,
        RefusalReason::ConflictingDeclaration,
        true,
    );
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(1),
        "a mismatched digest writes no fence"
    );

    // The snapshot leg refuses a document whose bytes do not match its digest
    // the same way, before any row is decoded.
    let snapshot = harness.snapshot(cursor(2), vec![process_row("worker")], None);
    let bytes = publication_snapshot_bytes(&snapshot);
    harness
        .begin_snapshot(TX_TWO, snapshot.cursor.clone(), 1, bytes.len() as u64)
        .await
        .expect("the transfer opens");
    harness
        .chunk(TX_TWO, 0, 1, bytes)
        .await
        .expect("the chunk is accepted");
    let error = harness
        .end_snapshot(TX_TWO, 1, DesiredDigest::of(b"not-the-document"))
        .await
        .expect_err("a document whose bytes do not match its digest is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_DIGEST_MISMATCH,
        AdmissionStage::Recover,
        RefusalReason::ConflictingDeclaration,
        true,
    );
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(1),
        "the mismatched document installed nothing"
    );
}

#[tokio::test]
async fn the_candidate_digest_covers_which_resource_each_row_is_for() {
    // The digest names the exact committed bytes a change installs, so two
    // candidates that differ only in which resource a row is for must not
    // share one identity.
    let one = publication_candidate_digest(&[process_row("worker")], &[]);
    let two = publication_candidate_digest(&[process_row("other")], &[]);
    assert_ne!(one.as_str(), two.as_str(), "the row reference is covered");

    let kept = publication_candidate_digest(&[process_row("worker")], &[]);
    let removed = publication_candidate_digest(
        &[process_row("worker")],
        &[reference("Process/retired")],
    );
    assert_ne!(kept.as_str(), removed.as_str(), "the retirements are covered");
}

#[tokio::test]
async fn a_second_snapshot_is_refused_while_one_is_in_progress() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;
    let snapshot = harness.snapshot(cursor(2), vec![process_row("worker")], None);
    let bytes = publication_snapshot_bytes(&snapshot);

    harness
        .begin_snapshot(TX_ONE, snapshot.cursor.clone(), 1, bytes.len() as u64)
        .await
        .expect("the transfer opens");
    let error = harness
        .begin_snapshot(TX_TWO, snapshot.cursor.clone(), 1, bytes.len() as u64)
        .await
        .expect_err("one snapshot is in progress per Zone");
    assert_refused(
        &refusal(error),
        PUBLICATION_SNAPSHOT_IN_PROGRESS,
        AdmissionStage::Recover,
        RefusalReason::ConflictingDeclaration,
        true,
    );
    match harness.status().await {
        ZoneAuthorityState::SnapshotInProgress {
            transaction,
            received_chunks,
            total_chunks,
            ..
        } => {
            assert_eq!(transaction, tx_id(TX_ONE));
            assert_eq!(received_chunks, 0);
            assert_eq!(total_chunks, 1);
        }
        other => panic!("the Zone is {}", posture(&other)),
    }

    // An ordinary mutation cannot start underneath an open transfer either.
    let error = harness
        .prepare_refused(harness.prepare_request(
            TX_TWO,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    assert_refused(
        &error,
        PUBLICATION_SNAPSHOT_IN_PROGRESS,
        AdmissionStage::Recover,
        RefusalReason::ConflictingDeclaration,
        true,
    );
}

// ---------------------------------------------------------------------------
// The fence blocks new use, not the way out
// ---------------------------------------------------------------------------

/// Reach a Zone that is unfenced at `cursor(2)` with one admitted effect, which
/// is the starting point for every case below that then holds a fence.
async fn unfenced_with_one_effect() -> Harness {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;
    harness
        .prepare(harness.prepare_request(
            TX_ONE,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    let accepted = harness
        .commit(harness.commit_request(
            TX_ONE,
            cursor(1),
            cursor(2),
            vec![process_row("worker")],
            Vec::new(),
        ))
        .await;
    assert_eq!(accepted.sequence, sequence(2), "the first revision is accepted");
    assert!(accepted.unfrozen, "the accepted Zone is unfrozen");
    harness.reopen().await;
    harness
        .serve(AuthorityPublicationRequest::BeginEffect(
            harness.begin_effect_request(EFFECT_ONE, TX_ONE, cursor(2), "Process/worker"),
        ))
        .await
        .expect("an unfenced Zone admits an effect");
    harness
}

#[tokio::test]
async fn a_frozen_zone_refuses_new_effects_and_keeps_the_control_lane() {
    let mut harness = unfenced_with_one_effect().await;
    harness
        .prepare(harness.prepare_request(
            TX_TWO,
            cursor(2),
            cursor(3),
            PublicationMutationKind::UpdateSpec,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    assert_eq!(posture(&harness.status().await), "fenced");

    // New ordinary use is refused by name while the fence is held.
    let error = harness
        .serve(AuthorityPublicationRequest::BeginEffect(
            harness.begin_effect_request(EFFECT_TWO, TX_TWO, cursor(2), "Process/worker"),
        ))
        .await
        .expect_err("a frozen Zone admits no new effect");
    assert_refused(
        &refusal(error),
        PUBLICATION_FENCE_HELD,
        AdmissionStage::Prepare,
        RefusalReason::StaleAuthority,
        true,
    );

    // Every control kind the enumeration names is still serviceable, and none
    // of them can admit new use.
    for kind in PublicationControlKind::ALL {
        assert!(
            !kind.stage().admits_new_use(),
            "{kind:?} must not admit new use"
        );
        harness
            .serve(AuthorityPublicationRequest::ControlAction(harness.control(
                TX_TWO,
                Some(EFFECT_ONE),
                "Process/worker",
                kind,
            )))
            .await
            .unwrap_or_else(|error| panic!("{kind:?} stays serviceable: {error}"));
    }
    assert!(
        harness.status().await.is_fenced(),
        "the control lane did not thaw the Zone"
    );
}

#[tokio::test]
async fn a_control_action_must_be_bound_to_a_known_identity() {
    let mut harness = unfenced_with_one_effect().await;
    harness
        .prepare(harness.prepare_request(
            TX_TWO,
            cursor(2),
            cursor(3),
            PublicationMutationKind::UpdateSpec,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;

    for kind in PublicationControlKind::ALL {
        // An action bound to a transaction this broker never saw is refused.
        let error = harness
            .serve(AuthorityPublicationRequest::ControlAction(harness.control(
                TX_UNKNOWN,
                Some(EFFECT_ONE),
                "Process/worker",
                kind,
            )))
            .await
            .expect_err("{kind:?} on an unknown transaction is refused");
        assert_refused(
            &refusal(error),
            PUBLICATION_CONTROL_NOT_BOUND,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
            true,
        );

        // So is one bound to an effect that is not in the journal.
        let error = harness
            .serve(AuthorityPublicationRequest::ControlAction(harness.control(
                TX_TWO,
                Some(EFFECT_UNKNOWN),
                "Process/worker",
                kind,
            )))
            .await
            .expect_err("{kind:?} on an unknown effect is refused");
        assert_refused(
            &refusal(error),
            PUBLICATION_CONTROL_NOT_BOUND,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
            true,
        );

        // And one that names a relationship the effect does not act on.
        let error = harness
            .serve(AuthorityPublicationRequest::ControlAction(harness.control(
                TX_TWO,
                Some(EFFECT_ONE),
                "Role/reader",
                kind,
            )))
            .await
            .expect_err("{kind:?} on another target is refused");
        assert_refused(
            &refusal(error),
            PUBLICATION_CONTROL_NOT_BOUND,
            AdmissionStage::Recover,
            RefusalReason::UnprovenEffect,
            true,
        );
    }
}

// ---------------------------------------------------------------------------
// The exec-release gate and the reducing proof obligation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_matching_digest_activation_releases_the_pending_launch() {
    let harness = unfenced_with_one_effect().await;

    harness
        .serve(AuthorityPublicationRequest::ReleaseEffect(
            ReleaseEffectRequest {
                effect: effect_id(EFFECT_ONE),
                accepted: cursor(2),
            },
        ))
        .await
        .expect("an unfenced Zone releases the pending launch");
    // A retried release is idempotent, not a second authorization.
    harness
        .serve(AuthorityPublicationRequest::ReleaseEffect(
            ReleaseEffectRequest {
                effect: effect_id(EFFECT_ONE),
                accepted: cursor(2),
            },
        ))
        .await
        .expect("a retried release is idempotent");

    // A release fenced against a superseded cursor is refused by name.
    let error = harness
        .serve(AuthorityPublicationRequest::ReleaseEffect(
            ReleaseEffectRequest {
                effect: effect_id(EFFECT_ONE),
                accepted: cursor(1),
            },
        ))
        .await
        .expect_err("a release against a superseded cursor is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_STALE_PREDECESSOR,
        AdmissionStage::Activate,
        RefusalReason::StaleAuthority,
        false,
    );

    // A child that had reached exec converges as release evidence, and that
    // outcome is not the acceptance of a revision.
    let converged = harness
        .serve(AuthorityPublicationRequest::EffectExit(EffectExitRequest {
            effect: effect_id(EFFECT_ONE),
            transaction: tx_id(TX_ONE),
            reached_exec: true,
        }))
        .await
        .expect("the completion is accepted");
    match converged {
        AuthorityPublicationResponse::RevocationConverged(convergence) => {
            assert!(convergence.proven, "an exited child is proved");
            assert_eq!(convergence.effect, effect_id(EFFECT_ONE));
        }
        other => panic!("a completion is not an accepted revision: {other:?}"),
    }
}

#[tokio::test]
async fn the_fence_beat_the_release_and_a_reducing_commit_waits_for_the_child() {
    let mut harness = unfenced_with_one_effect().await;

    let prepared = harness
        .prepare(harness.prepare_request(
            TX_TWO,
            cursor(2),
            cursor(3),
            PublicationMutationKind::Delete,
            vec![process_row("worker")],
            vec![worker()],
            shell(),
        ))
        .await;
    assert!(prepared.reducing, "a delete is a reducing change");

    // The fence won: the pending launch cannot exec under its earlier
    // BeginEffect, and the release is refused.
    let error = harness
        .serve(AuthorityPublicationRequest::ReleaseEffect(
            ReleaseEffectRequest {
                effect: effect_id(EFFECT_ONE),
                accepted: cursor(2),
            },
        ))
        .await
        .expect_err("a fenced Zone authorizes no exec");
    assert_refused(
        &refusal(error),
        PUBLICATION_FENCE_HELD,
        AdmissionStage::Revoke,
        RefusalReason::StaleAuthority,
        true,
    );

    // The reducing change cannot be accepted while that child is unaccounted.
    let error = harness
        .serve(AuthorityPublicationRequest::CommitChange(
            harness.commit_request(TX_TWO, cursor(2), cursor(3), vec![process_row("worker")], vec![worker()]),
        ))
        .await
        .expect_err("a reducing commit waits for the child");
    assert_refused(
        &refusal(error),
        PUBLICATION_EFFECT_UNPROVEN,
        AdmissionStage::Drain,
        RefusalReason::UnprovenEffect,
        true,
    );
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(2),
        "the unaccepted revision was not installed"
    );

    // The child's completion settles the record; it never reached exec, so it
    // is accounted rather than reported as release evidence.
    let outcome = harness
        .serve(AuthorityPublicationRequest::EffectExit(EffectExitRequest {
            effect: effect_id(EFFECT_ONE),
            transaction: tx_id(TX_ONE),
            reached_exec: false,
        }))
        .await
        .expect("the completion is accepted");
    assert!(
        matches!(outcome, AuthorityPublicationResponse::Progressed(_)),
        "a child that never exec'd is accounted, not converged: {outcome:?}"
    );

    let accepted = harness
        .commit(harness.commit_request(
            TX_TWO,
            cursor(2),
            cursor(3),
            vec![process_row("worker")],
            vec![worker()],
        ))
        .await;
    assert!(accepted.reducing, "the accepted change reduced the Zone");
    assert!(accepted.unfrozen);
    assert_eq!(accepted.sequence, sequence(3));

    // No unaccounted child remains: the removed revision admits nothing.
    let error = harness
        .serve(AuthorityPublicationRequest::BeginEffect(
            harness.begin_effect_request(EFFECT_TWO, TX_TWO, cursor(2), "Process/worker"),
        ))
        .await
        .expect_err("the superseded revision admits no effect");
    assert_refused(
        &refusal(error),
        PUBLICATION_STALE_PREDECESSOR,
        AdmissionStage::Prepare,
        RefusalReason::StaleAuthority,
        true,
    );
}

#[tokio::test]
async fn the_release_beat_the_fence_and_the_reducing_change_accounts_the_child() {
    let mut harness = unfenced_with_one_effect().await;

    // The release authorization was processed first, so the launch is existing
    // use and drains as such.
    harness
        .serve(AuthorityPublicationRequest::ReleaseEffect(
            ReleaseEffectRequest {
                effect: effect_id(EFFECT_ONE),
                accepted: cursor(2),
            },
        ))
        .await
        .expect("an unfenced Zone releases the pending launch");

    harness
        .prepare(harness.prepare_request(
            TX_TWO,
            cursor(2),
            cursor(3),
            PublicationMutationKind::Delete,
            vec![process_row("worker")],
            vec![worker()],
            shell(),
        ))
        .await;
    let accepted = harness
        .commit(harness.commit_request(
            TX_TWO,
            cursor(2),
            cursor(3),
            vec![process_row("worker")],
            vec![worker()],
        ))
        .await;
    assert!(
        accepted.reducing,
        "a pre-fence release is accounted, so the reducing change is accepted"
    );
    assert_eq!(accepted.sequence, sequence(3));
    assert!(harness.status().await.is_fenced().eq(&false), "the Zone is unfenced");
}

#[tokio::test]
async fn an_unmatched_completion_cannot_settle_a_reducing_commit() {
    let mut harness = unfenced_with_one_effect().await;
    harness
        .prepare(harness.prepare_request(
            TX_TWO,
            cursor(2),
            cursor(3),
            PublicationMutationKind::Delete,
            vec![process_row("worker")],
            vec![worker()],
            shell(),
        ))
        .await;

    // A completion for an effect this broker never admitted proves nothing.
    let error = harness
        .serve(AuthorityPublicationRequest::EffectExit(EffectExitRequest {
            effect: effect_id(EFFECT_UNKNOWN),
            transaction: tx_id(TX_TWO),
            reached_exec: true,
        }))
        .await
        .expect_err("an unknown effect cannot be proved gone");
    assert_refused(
        &refusal(error),
        PUBLICATION_UNKNOWN_TRANSACTION,
        AdmissionStage::Recover,
        RefusalReason::UnprovenEffect,
        true,
    );

    // Neither is a completion that names a transaction the effect is not under.
    let error = harness
        .serve(AuthorityPublicationRequest::EffectExit(EffectExitRequest {
            effect: effect_id(EFFECT_ONE),
            transaction: tx_id(TX_FOREIGN),
            reached_exec: true,
        }))
        .await
        .expect_err("a mismatched acknowledgment is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_CONTROL_NOT_BOUND,
        AdmissionStage::Recover,
        RefusalReason::UnprovenEffect,
        true,
    );

    let error = harness
        .serve(AuthorityPublicationRequest::CommitChange(
            harness.commit_request(TX_TWO, cursor(2), cursor(3), vec![process_row("worker")], vec![worker()]),
        ))
        .await
        .expect_err("nothing was settled, so the reducing change still waits");
    assert_refused(
        &refusal(error),
        PUBLICATION_EFFECT_UNPROVEN,
        AdmissionStage::Drain,
        RefusalReason::UnprovenEffect,
        true,
    );
}

// ---------------------------------------------------------------------------
// A commit names a prepared fence
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_commit_without_a_matching_prepared_fence_is_refused() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;

    let error = harness
        .serve(AuthorityPublicationRequest::CommitChange(
            harness.commit_request(
                TX_ONE,
                cursor(1),
                cursor(2),
                vec![process_row("worker")],
                Vec::new(),
            ),
        ))
        .await
        .expect_err("a commit with no fence behind it is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_FENCE_HELD,
        AdmissionStage::Authorize,
        RefusalReason::UnprovenEffect,
        true,
    );
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(1),
        "no commit without a fence installs anything"
    );
    harness
        .publish_snapshot(
            TX_THREE,
            cursor(1),
            vec![reader_role(), shell_binding()],
            None,
        )
        .await
        .expect("the Zone reconciles");

    harness
        .prepare(harness.prepare_request(
            TX_ONE,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    let error = harness
        .serve(AuthorityPublicationRequest::CommitChange(
            harness.commit_request(
                TX_TWO,
                cursor(1),
                cursor(2),
                vec![process_row("worker")],
                Vec::new(),
            ),
        ))
        .await
        .expect_err("a commit must name the prepared identity");
    assert_refused(
        &refusal(error),
        PUBLICATION_UNKNOWN_TRANSACTION,
        AdmissionStage::Authorize,
        RefusalReason::UnprovenEffect,
        true,
    );

    let error = harness
        .serve(AuthorityPublicationRequest::CommitChange(
            harness.commit_request(
                TX_ONE,
                cursor(1),
                cursor(3),
                vec![process_row("worker")],
                Vec::new(),
            ),
        ))
        .await
        .expect_err("a commit must install the cursor the fence named");
    assert_refused(
        &refusal(error),
        PUBLICATION_STALE_PREDECESSOR,
        AdmissionStage::Authorize,
        RefusalReason::StaleAuthority,
        true,
    );
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(1),
        "the accepted cursor never moved"
    );
}

#[tokio::test]
async fn a_cancel_releases_the_fence_only_for_the_exact_identity() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;
    let prepared = harness
        .prepare(harness.prepare_request(
            TX_ONE,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;

    let cancel = |id: &str, digest: DesiredDigest| {
        CancelTransactionRequest {
            transaction: tx_id(id),
            store_incarnation: incarnation(STORE),
            digest,
        }
    };

    let error = harness
        .serve(AuthorityPublicationRequest::CancelTransaction(cancel(
            TX_UNKNOWN,
            prepared.digest.clone(),
        )))
        .await
        .expect_err("a cancel names an existing prepared identity");
    assert_refused(
        &refusal(error),
        PUBLICATION_UNKNOWN_TRANSACTION,
        AdmissionStage::Recover,
        RefusalReason::UnprovenEffect,
        true,
    );
    assert_eq!(posture(&harness.status().await), "fenced");

    let error = harness
        .serve(AuthorityPublicationRequest::CancelTransaction(cancel(
            TX_ONE,
            DesiredDigest::of(b"other-bytes"),
        )))
        .await
        .expect_err("a cancel cannot release a fence prepared for other bytes");
    assert_refused(
        &refusal(error),
        PUBLICATION_DUPLICATE_TRANSACTION,
        AdmissionStage::Recover,
        RefusalReason::ConflictingDeclaration,
        true,
    );
    assert_eq!(posture(&harness.status().await), "fenced");

    harness
        .serve(AuthorityPublicationRequest::CancelTransaction(cancel(
            TX_ONE,
            prepared.digest.clone(),
        )))
        .await
        .expect("the exact identity and digest release the fence");
    assert_eq!(posture(&harness.status().await), "unfenced");
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(1),
        "a cancel that committed nothing moves no cursor"
    );
}

// ---------------------------------------------------------------------------
// The accepted lower bound survives every transfer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_snapshot_cannot_move_the_projection_below_the_accepted_cursor() {
    let mut harness = unfenced_with_one_effect().await;

    let error = harness
        .publish_snapshot(TX_TWO, cursor(1), vec![process_row("worker")], None)
        .await
        .expect_err("a snapshot below the accepted cursor is refused");
    assert_refused(
        &error_refusal(&error),
        PUBLICATION_STALE_PREDECESSOR,
        AdmissionStage::Recover,
        RefusalReason::StaleAuthority,
        true,
    );
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(2),
        "the accepted lower bound did not move"
    );

    // The same sequence with a different digest is a different history.
    harness
        .publish_snapshot(TX_THREE, cursor(2), Vec::new(), None)
        .await
        .expect("the Zone reconciles");
    let mut conflicting = harness.snapshot(cursor(2), Vec::new(), None);
    conflicting.cursor = AuthorityCursor {
        sequence: sequence(2),
        digest: DesiredDigest::of(b"a-different-history"),
    };
    let bytes = publication_snapshot_bytes(&conflicting);
    harness
        .begin_snapshot(TX_TWO, conflicting.cursor.clone(), 1, bytes.len() as u64)
        .await
        .expect("the transfer opens");
    harness.chunk(TX_TWO, 0, 1, bytes).await.expect("the chunk is accepted");
    let error = harness
        .end_snapshot(TX_TWO, 1, publication_snapshot_digest(&conflicting))
        .await
        .expect_err("a different digest at the accepted sequence is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_STALE_PREDECESSOR,
        AdmissionStage::Recover,
        RefusalReason::StaleAuthority,
        true,
    );

    // Forward is still admitted, and the Zone unfreezes at the new cursor.
    harness
        .publish_snapshot(TX_THREE, cursor(2), Vec::new(), None)
        .await
        .expect("the Zone reconciles");
    harness
        .publish_snapshot(TX_TWO, cursor(3), vec![process_row("worker")], None)
        .await
        .expect("a forward snapshot installs");
    let state = harness.status().await;
    assert_eq!(posture(&state), "unfenced");
    assert_eq!(accepted_sequence(&state), Some(3));
}

fn error_refusal(error: &AuthorityProjectionError) -> PublicationRefusal {
    error
        .refusal()
        .expect("the broker answered with a typed refusal")
        .clone()
}

#[tokio::test]
async fn the_resync_floor_cannot_move_below_the_accepted_cursor() {
    let mut harness = unfenced_with_one_effect().await;

    let resync = |floor: AuthorityCursor, cursor: AuthorityCursor| ResynchronizeRequest {
        transaction: tx_id(TX_TWO),
        store_incarnation: incarnation(STORE),
        accepted_floor: floor,
        cursor,
    };

    let error = harness
        .serve(AuthorityPublicationRequest::Resynchronize(resync(
            cursor(1),
            cursor(3),
        )))
        .await
        .expect_err("a floor below the accepted cursor is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_STALE_PREDECESSOR,
        AdmissionStage::Recover,
        RefusalReason::StaleAuthority,
        true,
    );
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(2),
        "the accepted cursor did not move"
    );

    let error = harness
        .serve(AuthorityPublicationRequest::Resynchronize(resync(
            AuthorityCursor {
                sequence: sequence(2),
                digest: DesiredDigest::of(b"a-different-history"),
            },
            cursor(3),
        )))
        .await
        .expect_err("a different digest at the accepted sequence is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_STALE_PREDECESSOR,
        AdmissionStage::Recover,
        RefusalReason::StaleAuthority,
        true,
    );

    harness
        .publish_snapshot(TX_THREE, cursor(2), Vec::new(), None)
        .await
        .expect("the Zone reconciles");
    harness
        .serve(AuthorityPublicationRequest::Resynchronize(resync(cursor(2), cursor(3))))
        .await
        .expect("a floor at the accepted cursor is accepted");
    assert_eq!(posture(&harness.status().await), "reconciling");
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(2),
        "a resynchronization records the intent, not the revision"
    );
}

#[tokio::test]
async fn an_unexpected_store_incarnation_is_refused() {
    let mut harness = unfenced_with_one_effect().await;

    let mut request = harness.prepare_request(
        TX_TWO,
        cursor(2),
        cursor(3),
        PublicationMutationKind::UpdateSpec,
        vec![process_row("worker")],
        Vec::new(),
        shell(),
    );
    request.store_incarnation = incarnation(OTHER_STORE);
    let error = harness
        .prepare_refused(request)
        .await;
    assert_refused(
        &error,
        PUBLICATION_STALE_PREDECESSOR,
        AdmissionStage::Authorize,
        RefusalReason::StoreIncarnationMismatch,
        true,
    );

    harness
        .publish_snapshot(TX_THREE, cursor(2), Vec::new(), None)
        .await
        .expect("the Zone reconciles");
    let mut snapshot = harness.snapshot(cursor(3), vec![process_row("worker")], None);
    snapshot.store_incarnation = incarnation(OTHER_STORE);
    let bytes = publication_snapshot_bytes(&snapshot);
    harness
        .begin_snapshot(TX_TWO, snapshot.cursor.clone(), 1, bytes.len() as u64)
        .await
        .expect("the transfer opens");
    harness.chunk(TX_TWO, 0, 1, bytes).await.expect("the chunk is accepted");
    let error = harness
        .end_snapshot(TX_TWO, 1, publication_snapshot_digest(&snapshot))
        .await
        .expect_err("a document from another store generation is refused");
    assert_refused(
        &refusal(error),
        PUBLICATION_STALE_PREDECESSOR,
        AdmissionStage::Recover,
        RefusalReason::StoreIncarnationMismatch,
        true,
    );
    assert_eq!(
        accepted_sequence(&harness.status().await),
        Some(2),
        "another store generation installed nothing"
    );
}

// ---------------------------------------------------------------------------
// A broker restart refuses its cached authority
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_broker_restart_refuses_cached_authority_until_reconciliation() {
    let mut harness = unfenced_with_one_effect().await;
    let epoch_before = harness.projection.epoch().await;
    let session_before = harness.session.clone();
    let accepted_before = accepted_sequence(&harness.status().await);

    harness.restart().await;

    assert!(
        harness.projection.epoch().await > epoch_before,
        "a restart strictly increments the epoch"
    );
    assert_ne!(
        session_before.as_str(),
        harness.session.as_str(),
        "the re-minted session is derived under the new epoch"
    );
    let state = harness.status().await;
    assert_eq!(posture(&state), "reconciling", "the restarted Zone reconciles");
    assert_eq!(
        accepted_sequence(&state),
        accepted_before,
        "the accepted cursor survives the restart and did not move"
    );

    // No new effect is admitted until the manager resynchronizes.
    let error = harness
        .serve(AuthorityPublicationRequest::BeginEffect(
            harness.begin_effect_request(EFFECT_TWO, TX_ONE, cursor(3), "Process/worker"),
        ))
        .await
        .expect_err("a restarted broker admits no new effect");
    assert_refused(
        &refusal(error),
        PUBLICATION_FENCE_HELD,
        AdmissionStage::Prepare,
        RefusalReason::StaleAuthority,
        true,
    );

    // And no new ordinary mutation is either.
    let error = harness
        .prepare_refused(harness.prepare_request(
            TX_TWO,
            cursor(2),
            cursor(3),
            PublicationMutationKind::UpdateSpec,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    assert_refused(
        &error,
        PUBLICATION_RECONCILIATION_REQUIRED,
        AdmissionStage::Recover,
        RefusalReason::UnprovenEffect,
        true,
    );

    // The prepared identity the broker durably saw survived the restart, so
    // the control lane is still answerable for it.
    harness
        .serve(AuthorityPublicationRequest::ControlAction(harness.control(
            TX_ONE,
            None,
            "Process/worker",
            PublicationControlKind::Observe,
        )))
        .await
        .expect("the control lane is serviceable for a known transaction");
    harness
        .serve(AuthorityPublicationRequest::ControlAction(harness.control(
            TX_UNKNOWN,
            None,
            "Process/worker",
            PublicationControlKind::Observe,
        )))
        .await
        .expect_err("and refuses an identity it never saw");
    assert!(
        harness.status().await.is_fenced(),
        "observation did not unfence the Zone"
    );

    // Reconciliation restores service without moving the accepted cursor.
    harness
        .serve(AuthorityPublicationRequest::Resynchronize(
            ResynchronizeRequest {
                transaction: tx_id(TX_TWO),
                store_incarnation: incarnation(STORE),
                accepted_floor: cursor(2),
                cursor: cursor(2),
            },
        ))
        .await
        .expect("the reconciliation is accepted");
    harness
        .publish_snapshot(TX_TWO, cursor(2), vec![process_row("worker")], None)
        .await
        .expect("the reconciled snapshot installs");
    let state = harness.status().await;
    assert_eq!(posture(&state), "unfenced", "the Zone serves again");
    assert_eq!(accepted_sequence(&state), accepted_before);

    harness
        .serve(AuthorityPublicationRequest::BeginEffect(
            harness.begin_effect_request(EFFECT_TWO, TX_ONE, cursor(2), "Process/worker"),
        ))
        .await
        .expect("a reconciled Zone admits an effect again");
}

// ---------------------------------------------------------------------------
// Mutation recovery: after the broker acknowledged, before the API response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_replayed_commit_after_the_acknowledgment_is_refused_not_reapplied() {
    let mut harness = unfenced_with_one_effect().await;
    harness
        .prepare(harness.prepare_request(
            TX_TWO,
            cursor(2),
            cursor(3),
            PublicationMutationKind::UpdateSpec,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    let commit = harness.commit_request(
        TX_TWO,
        cursor(2),
        cursor(3),
        vec![process_row("worker")],
        Vec::new(),
    );
    let accepted = harness.commit(commit.clone()).await;
    assert_eq!(accepted.sequence, sequence(3));

    // The manager did not see the acknowledgment and replays the exact commit.
    // The broker no longer holds a fence for it, so it refuses by name and
    // leaves the Zone fenced rather than accepting a second time.
    harness.reopen().await;
    let error = harness
        .serve(AuthorityPublicationRequest::CommitChange(commit))
        .await
        .expect_err("a replayed commit does not apply a second time");
    let refused = refusal(error);
    assert_eq!(refused.code, PUBLICATION_FENCE_HELD, "refusal code");
    assert_eq!(refused.stage, AdmissionStage::Authorize, "refusal stage");
    assert_eq!(refused.reason, RefusalReason::UnprovenEffect, "refusal reason");

    let state = harness.status().await;
    assert!(
        state.is_fenced(),
        "an unanswered replay ends in an explicit fenced state, not a second acceptance"
    );
    assert_eq!(
        accepted_sequence(&state),
        Some(3),
        "the accepted revision is the one the first commit installed"
    );

    // The documented recovery for that state is a full resynchronization.
    harness
        .publish_snapshot(TX_THREE, cursor(3), vec![process_row("worker")], None)
        .await
        .expect("the Zone reconciles");
    assert_eq!(posture(&harness.status().await), "unfenced");
    assert_eq!(accepted_sequence(&harness.status().await), Some(3));
}

// ---------------------------------------------------------------------------
// The projection holds a graph, not a second copy of the manager's rows
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_committed_projection_keeps_every_accepted_authority_row() {
    let mut harness = Harness::start().await;
    harness.install_reader_grant().await;

    // Two roles and two bindings, all of them authority the evaluator reads.
    let second_role = || {
        let rule = RoleRule::new(
            vec![ResourceTypeName::parse("Role").expect("Role is a standard type")],
            vec![
                RoleResourceVerb::Get,
                RoleResourceVerb::List,
                RoleResourceVerb::UpdateMetadata,
            ],
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
        .expect("the auditor rule is bounded");
        let role = AuthorizedRole::new(vec![rule], Vec::new()).expect("the auditor role is bounded");
        projection_row("Role/auditor", &canonical(&role))
    };
    let second_binding = || {
        let binding = RoleBindingSpec::with_facets(
            reference("Role/auditor"),
            vec![reference("Process/shell")],
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            None,
        )
        .expect("the second binding is bounded");
        projection_row("RoleBinding/auditor", &canonical(&binding))
    };
    harness
        .publish_snapshot(
            TX_BOOTSTRAP,
            cursor(1),
            vec![
                reader_role(),
                shell_binding(),
                second_role(),
                second_binding(),
                process_row("worker"),
            ],
            None,
        )
        .await
        .expect("the four authority rows install");

    // Both grants are live at once: the reader grant mutates a `Process`, and
    // the auditor grant reaches a `Role`. Neither is visible through the other,
    // and committing one does not evict the other.
    harness
        .prepare(harness.prepare_request(
            TX_ONE,
            cursor(1),
            cursor(2),
            PublicationMutationKind::Create,
            vec![process_row("worker")],
            Vec::new(),
            shell(),
        ))
        .await;
    harness
        .commit(harness.commit_request(
            TX_ONE,
            cursor(1),
            cursor(2),
            vec![process_row("worker")],
            Vec::new(),
        ))
        .await;
    let held = harness
        .prepare(harness.prepare_request(
            TX_TWO,
            cursor(2),
            cursor(3),
            PublicationMutationKind::UpdateMetadata,
            vec![second_role()],
            Vec::new(),
            shell(),
        ))
        .await;
    // Release the fence so the two refusals below are decided by the accepted
    // graph rather than by the fence the Zone is already holding.
    harness
        .serve(AuthorityPublicationRequest::CancelTransaction(
            CancelTransactionRequest {
                transaction: tx_id(TX_TWO),
                store_incarnation: incarnation(STORE),
                digest: held.digest,
            },
        ))
        .await
        .expect("the exact fence is released");

    // The auditor grant does not reach a verb it never declared, and the
    // reader grant does not reach the Role resource type.
    let error = harness
        .prepare_refused(harness.prepare_request(
            TX_THREE,
            cursor(2),
            cursor(3),
            PublicationMutationKind::Delete,
            vec![second_role()],
            Vec::new(),
            shell(),
        ))
        .await;
    assert_eq!(
        error.reason,
        RefusalReason::IdentityNotAuthorized,
        "the refusal is decided against the accepted graph"
    );
    // The refusal fenced the Zone; reconcile it with the same four rows so the
    // next probe is decided by the graph and not by the fence.
    harness
        .publish_snapshot(
            TX_THREE,
            cursor(2),
            vec![reader_role(), shell_binding(), second_role(), second_binding()],
            None,
        )
        .await
        .expect("the Zone reconciles with both grants");
    let error = harness
        .prepare_refused(harness.prepare_request(
            TX_THREE,
            cursor(2),
            cursor(3),
            PublicationMutationKind::UpdateMetadata,
            vec![second_role()],
            Vec::new(),
            AuthoritySubject::named(AuthoritySubjectKind::Process, reference("Process/other")),
        ))
        .await;
    assert_eq!(
        error.reason,
        RefusalReason::IdentityNotAuthorized,
        "an unbound subject is refused"
    );
}
