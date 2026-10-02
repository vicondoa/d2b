//! A broker restarted under a durable store, reconciled by the manager (U7,
//! KTD6-KTD7).
//!
//! These cases drive the REAL [`AuthorityProjection`] over its real durable
//! state and the REAL [`SpecStore`] behind it. The only substitution is the
//! unix socket: every envelope is the production one and every answer is the
//! broker's own projection's, so a refusal here is a refusal the daemon reads
//! in production.
//!
//! What has to hold is the whole resynchronization, in both directions. A
//! broker restart moves every known Zone to reconciling and refuses every
//! ordinary message, so the manager has to restate the projection the broker
//! already accepted - and the broker has to PROVE that projection against what
//! it holds rather than install it. A daemon that presents a document the
//! broker cannot derive from its own durable state is refused by name and the
//! Zone stays fenced.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_broker::authority_projection::AuthorityProjection;
use d2b_contracts_broker::broker_wire::{
    AuthorityPublicationEnvelope, AuthorityPublicationOpen, AuthorityPublicationResponse,
    ZoneAuthorityState, PUBLICATION_PROJECTION_UNPROVEN,
};
use d2b_contracts_resource::v3::{
    AuthoritySubject, AuthoritySubjectKind, CanonicalJsonObject, DesiredDigest, DesiredRevision,
    RefusalReason, ResourceRef, ResourceTypeName,
};
use d2b_contracts_zone_session::v3::role::{AuthorizedRole, RoleResourceVerb, RoleRule};
use d2b_contracts_zone_session::v3::RoleBindingSpec;
use d2b_resource_runtime::authority_journal::DesiredMutation;
use d2b_resource_runtime::spec_store::{
    ResourceKey, ResourceProvenance, SpecStore, StoredDesiredResource,
};
use d2b_resource_runtime::{
    AuthorityPublisher, PublishedRow, ZoneProjection, publish, resynchronize,
};
use d2bd::authority_publication::{
    AuthorityPublicationCoordinator, AuthorityPublicationLink, CoordinatorPublisher,
};
use tempfile::TempDir;

const ZONE: &str = "pubzone";

/// The link the daemon publishes over, bound to one real broker projection.
///
/// This is the daemon's own seam with the socket removed: every envelope is
/// the production one and every answer is the broker's own projection's, so
/// the transport is the only thing here that is not real - and the protocol it
/// carries is entirely real.
#[derive(Clone)]
struct ProjectionLink {
    projection: Arc<AuthorityProjection>,
}

impl std::fmt::Debug for ProjectionLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ProjectionLink(<the real broker projection>)")
    }
}

#[async_trait]
impl AuthorityPublicationLink for ProjectionLink {
    async fn open_session(
        &self,
        open: AuthorityPublicationOpen,
    ) -> Result<AuthorityPublicationResponse, String> {
        self.projection
            .open_session(open)
            .await
            .map_err(|error| error.to_string())
    }

    async fn serve(
        &self,
        envelope: AuthorityPublicationEnvelope,
    ) -> Result<AuthorityPublicationResponse, String> {
        self.projection
            .serve(&envelope)
            .await
            .map_err(|error| error.to_string())
    }
}

/// One store, one broker, and the publisher that publishes the store to it.
///
/// The publisher is built per use rather than held, so a restart can drop the
/// only handle on the old broker before a new one opens the same durable root:
/// exactly one writer ever owns that state, which is what makes the restarted
/// broker's refusal of its own cached authority the real thing rather than two
/// processes disagreeing.
struct Fixture {
    root: TempDir,
    /// A scratch root whose projection is only ever a handle to drop.
    scratch: TempDir,
    store: Arc<SpecStore>,
    projection: Arc<AuthorityProjection>,
}

impl Fixture {
    async fn start() -> Self {
        let root = tempfile::tempdir().expect("a scratch state root");
        let scratch = tempfile::tempdir().expect("a throwaway state root");
        let store = Arc::new(
            SpecStore::open(root.path().join("specs.sqlite")).expect("the store opens"),
        );
        let projection = Arc::new(
            AuthorityProjection::open_async(root.path())
                .await
                .expect("the projection opens"),
        );
        Self {
            root,
            scratch,
            store,
            projection,
        }
    }

    /// A publisher bound to this fixture's store and live broker, under the
    /// store's real incarnation.
    async fn publisher(&self) -> Arc<CoordinatorPublisher> {
        let incarnation = self
            .store
            .store_incarnation()
            .await
            .expect("the store carries an incarnation");
        CoordinatorPublisher::new(
            Arc::new(AuthorityPublicationCoordinator::new(
                ZONE,
                incarnation.clone(),
                bootstrap(),
                Arc::new(ProjectionLink {
                    projection: Arc::clone(&self.projection),
                }),
            )),
            incarnation,
            bootstrap(),
        )
    }

    /// Restart the broker over the same durable state, the way a service
    /// restart does.
    async fn restart_broker(&mut self) {
        let root = self.root.path().to_path_buf();
        let scratch = self.scratch.path().to_path_buf();
        // The old broker is dropped first, through a projection over a root it
        // never wrote to, so exactly one writer ever owns the real state.
        self.projection = Arc::new(
            AuthorityProjection::open_async(&scratch)
                .await
                .expect("a throwaway projection opens"),
        );
        self.projection = Arc::new(
            AuthorityProjection::open_async(root)
                .await
                .expect("the projection reopens over its own durable state"),
        );
    }

    async fn posture(&self) -> &'static str {
        match self.projection.status(ZONE).await {
            ZoneAuthorityState::Unprovisioned => "unprovisioned",
            ZoneAuthorityState::Unfenced { .. } => "unfenced",
            ZoneAuthorityState::Fenced { .. } => "fenced",
            ZoneAuthorityState::SnapshotInProgress { .. } => "snapshot-in-progress",
            ZoneAuthorityState::Reconciling { .. } => "reconciling",
        }
    }

    async fn accepted_sequence(&self) -> u64 {
        self.projection
            .status(ZONE)
            .await
            .accepted()
            .map(|cursor| cursor.sequence.get())
            .unwrap_or_default()
    }

    async fn publish(&self, desired: StoredDesiredResource) {
        publish(
            &self.store,
            DesiredMutation::Ensure(desired),
            self.publisher().await.as_ref(),
        )
        .await
        .unwrap_or_else(|error| panic!("the store publishes its own committed row: {error}"));
    }

    /// Publish the Zone's authority grant: a Role and the RoleBinding that
    /// draws on it. Both are authority the broker stores and re-evaluates, so
    /// a reconciliation has to restate them identically rather than claim them.
    async fn publish_authority(&self) {
        self.publish(reader_role()).await;
        self.publish(shell_binding()).await;
    }
}

fn bootstrap() -> AuthoritySubject {
    AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("the fixture references are canonical")
}

fn canonical(value: &impl serde::Serialize) -> CanonicalJsonObject {
    CanonicalJsonObject::parse(&serde_json::to_vec(value).expect("the fixture row serializes"))
        .expect("the fixture row is a canonical JSON object")
}

fn row(type_name: &str, name: &str, seed: u8, spec: &[u8]) -> StoredDesiredResource {
    StoredDesiredResource {
        key: ResourceKey::new(ZONE, type_name, name),
        uid: [seed; 16],
        generation: 0,
        owner_uid: None,
        provenance: ResourceProvenance::Api,
        deleting: false,
        spec: spec.to_vec(),
        metadata: b"meta".to_vec(),
        created_at: 0,
    }
}

/// `Role/reader`: every CRUD verb on a `Process`, nothing else.
fn reader_role() -> StoredDesiredResource {
    let rule = RoleRule::new(
        vec![ResourceTypeName::parse("Process").expect("Process is a standard type")],
        vec![
            RoleResourceVerb::Get,
            RoleResourceVerb::List,
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
    .expect("the reader rule is bounded");
    let role = AuthorizedRole::new(vec![rule], Vec::new()).expect("the reader role is bounded");
    row("Role", "reader", 0x11, &canonical(&role).to_canonical_bytes())
}

/// `RoleBinding/shell`: `Process/shell` draws on `Role/reader`.
fn shell_binding() -> StoredDesiredResource {
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
    row(
        "RoleBinding",
        "shell",
        0x12,
        &canonical(&binding).to_canonical_bytes(),
    )
}

/// One ordinary, non-authority row: the broker never reads its bytes as
/// authority, so it is the row whose survival proves the transfer worked
/// without changing what the Zone may do.
fn worker_row() -> StoredDesiredResource {
    row(
        "Process",
        "worker",
        0x13,
        &serde_json::to_vec(&serde_json::json!({ "declared": "worker" }))
            .expect("the worker row renders"),
    )
}

#[tokio::test]
async fn a_broker_restart_serves_again_once_the_manager_resynchronizes() {
    let mut fixture = Fixture::start().await;
    fixture.publish_authority().await;
    fixture.publish(worker_row()).await;
    assert_eq!(fixture.posture().await, "unfenced");
    let accepted_before = fixture.accepted_sequence().await;
    assert_eq!(
        accepted_before, 3,
        "three publications reached the broker"
    );

    fixture.restart_broker().await;
    assert_eq!(
        fixture.posture().await,
        "reconciling",
        "a restarted broker refuses its own cached authority"
    );
    assert_eq!(
        fixture.accepted_sequence().await,
        accepted_before,
        "the accepted cursor survives the restart and did not move"
    );

    // The manager reconciles the Zone from its durable store. This is the whole
    // recovery path: adopt first (nothing is outstanding here), then restate
    // the accepted projection.
    resynchronize(&fixture.store, ZONE, fixture.publisher().await.as_ref())
        .await
        .expect("the reconciliation is proved and accepted");

    assert_eq!(
        fixture.posture().await,
        "unfenced",
        "a proved reconciliation serves again"
    );
    assert_eq!(
        fixture.accepted_sequence().await,
        accepted_before,
        "the reconciliation restated the accepted cursor and did not move it"
    );

    // And the Zone really does publish again: a further mutation fences and
    // commits against the reconciled projection.
    fixture
        .publish(row(
            "Process",
            "second",
            0x14,
            &serde_json::to_vec(&serde_json::json!({ "declared": "second" }))
                .expect("the second row renders"),
        ))
        .await;
    assert_eq!(
        fixture.accepted_sequence().await,
        accepted_before + 1,
        "the post-restart publication advanced the accepted cursor"
    );
}

#[tokio::test]
async fn a_daemon_that_cannot_prove_its_projection_is_refused_and_stays_fenced() {
    let mut fixture = Fixture::start().await;
    fixture.publish_authority().await;
    fixture.restart_broker().await;
    assert_eq!(fixture.posture().await, "reconciling");
    let accepted_before = fixture.accepted_sequence().await;

    let honest = fixture
        .store
        .zone_projection(ZONE)
        .await
        .expect("the store holds a projection");
    assert!(
        honest
            .rows
            .iter()
            .any(|published| published.row.key.type_name == "Role"),
        "the store's projection carries the accepted role"
    );
    assert!(
        honest
            .rows
            .iter()
            .any(|published| published.row.key.type_name == "RoleBinding"),
        "the store's projection carries the accepted binding"
    );

    // Three documents the broker holds no fact about. Each is refused by name
    // and each leaves the Zone fenced, which is the whole point: the broker
    // will not serve authority it cannot derive from its own durable state.
    for (what, document, reason) in [
        (
            "an accepted grant restated with different bytes",
            rewritten_role(&honest),
            RefusalReason::UnprovenEffect,
        ),
        (
            "an authority grant the broker never accepted",
            with_invented_grant(&honest),
            RefusalReason::UnprovenEffect,
        ),
    ] {
        let error = fixture
            .publisher()
            .await
            .resynchronize(&document)
            .await
            .expect_err("a document the broker cannot prove is refused");
        assert_refusal(&error, reason, what);
        assert_eq!(
            fixture.posture().await,
            "reconciling",
            "{what} left the Zone fenced"
        );
    }
    assert_eq!(
        fixture.accepted_sequence().await,
        accepted_before,
        "no unprovable document moved the accepted cursor"
    );

    // A store that acknowledged a publication the broker never accepted cannot
    // be reconciled at all, and says so before it sends anything: there is no
    // document that proves a cursor this broker holds no fact about.
    let error = fixture
        .publisher()
        .await
        .resynchronize(&ahead_of(&honest))
        .await
        .expect_err("a store ahead of the broker cannot be reconciled");
    assert!(
        error.to_string().contains("acknowledged sequence"),
        "the store and the broker disagreeing is named: {error}"
    );
    assert_eq!(fixture.posture().await, "reconciling");

    // The honest document still reconciles: the refusals were about the
    // documents, not about the Zone being unrecoverable.
    resynchronize(&fixture.store, ZONE, fixture.publisher().await.as_ref())
        .await
        .expect("the store's own projection reconciles after the refusals");
    assert_eq!(fixture.posture().await, "unfenced");
}

/// The one refusal a reconciliation the broker cannot prove carries.
fn assert_refusal(
    error: &d2b_resource_runtime::PublicationRefusal,
    reason: RefusalReason,
    what: &str,
) {
    let detail = error.to_string();
    assert!(
        detail.contains(PUBLICATION_PROJECTION_UNPROVEN),
        "{what} is refused as an unprovable projection: {detail}"
    );
    assert!(
        detail.contains(&format!("{reason:?}")),
        "{what} carries its typed reason: {detail}"
    );
}

/// The same projection, with one accepted `Role` restated at different bytes.
fn rewritten_role(honest: &ZoneProjection) -> ZoneProjection {
    let mut rewritten = honest.clone();
    let role = rewritten
        .rows
        .iter_mut()
        .find(|published| published.row.key.type_name == "Role")
        .expect("the projection carries the accepted role");
    let admitted = CanonicalJsonObject::parse(
        &serde_json::to_vec(&serde_json::json!({ "declared": "not-the-accepted-role" }))
            .expect("the rewritten row renders"),
    )
    .expect("the rewritten row is a canonical JSON object");
    role.row.spec = admitted.to_canonical_bytes();
    role.digest = DesiredDigest::of(&role.row.spec);
    rewritten
}

/// The same projection, claiming a cursor the broker never accepted.
fn ahead_of(honest: &ZoneProjection) -> ZoneProjection {
    let mut ahead = honest.clone();
    ahead.accepted.sequence = honest
        .accepted
        .sequence
        .try_next()
        .expect("the desired sequence has room");
    ahead
}

/// The same projection plus an authority row the broker never accepted.
fn with_invented_grant(honest: &ZoneProjection) -> ZoneProjection {
    let mut invented = honest.clone();
    let rule = RoleRule::new(
        vec![ResourceTypeName::parse("Role").expect("Role is a standard type")],
        vec![RoleResourceVerb::List],
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
        Vec::new(),
    )
    .expect("the invented rule is bounded");
    let role = AuthorizedRole::new(vec![rule], Vec::new()).expect("the invented role is bounded");
    let spec = canonical(&role).to_canonical_bytes();
    invented.rows.push(PublishedRow {
        row: StoredDesiredResource {
            key: ResourceKey::new(ZONE, "Role", "invented"),
            uid: [0x20; 16],
            generation: 1,
            owner_uid: None,
            provenance: ResourceProvenance::Api,
            deleting: false,
            spec: spec.clone(),
            metadata: b"meta".to_vec(),
            created_at: 0,
        },
        revision: DesiredRevision::INITIAL,
        digest: DesiredDigest::of(&spec),
        source_uid: None,
        consumer_uid: None,
    });
    invented
}