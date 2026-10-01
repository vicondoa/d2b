//! Coverage for the `NetworkBinding` serving driver.
//!
//! The committed row is produced by this family's own admission
//! ([`canonical_binding_rows`]) rather than hand-written, so every case here
//! runs the exact bytes a source controller would commit. What each case pins
//! is a property the driver exists to hold: a committed row reaches validate
//! and reconcile, a row whose committed decision does not cover the claim it
//! makes is refused terminally rather than served, a row whose declared parent
//! is owned by another resource never silently re-parents, and a row whose
//! consumer is not observable yet defers instead of failing terminal.
//!
//! The privileged realization verb has no broker operation behind it (see
//! [`MISSING_CREATE_MEMBERSHIP_TAP`]), and the cases below assert that refusal
//! by name rather than asserting a success the broker cannot produce.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use d2b_contracts_resource::v3::{
    DhcpSpec, DnsSpec, Ipv4Cidr, IsolationSpec, MdnsSpec, NetworkAttachmentEntry,
    NetworkBindingSpec, NetworkPresentation, NetworkProvenance, NetworkSpec, ResourceBundleGenerationId,
    ResourceGeneration, ResourceRef, ResourceUid, RoutingSpec, BoundedToken, ZoneId,
};
use d2b_provider_network_local::{
    MISSING_CREATE_MEMBERSHIP_TAP, NetworkAdmittedConsumer, NetworkBindingDriverArgs,
    NetworkBindingDriverEffects, NetworkBindingDriverFactory, NetworkBindingError,
    NetworkBindingSource, canonical_binding_rows,
    broker::NetworkEffectContext,
    controller::{NetworkAdmissionIntent, NetworkAdmissionKey},
    network_binding_descriptor, network_binding_spec_decoder,
};
use d2b_provider_toolkit::testing::fakes::{RecordingManagerEndpoint, RecordingRequeue};
use d2b_resource_runtime::context::ResourceContext;
use d2b_resource_runtime::driver::{DynResourceDriver, ResourceDriverFactory};
use d2b_resource_runtime::error::{FailureKinds, FailureOutcome};
use d2b_resource_runtime::identity::{ResourceKey, ResourceProvenance, StoredDesiredResource};

const ZONE: &str = "work";
const NETWORK_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
const HOST_UID: &str = "523e4567-e89b-42d3-a456-426614174004";
const GUEST_UID: &str = "423e4567-e89b-42d3-a456-426614174003";
const INSTALLED_GENERATION: &str =
    "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("registered resource reference")
}

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).expect("canonical resource uid")
}

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("zone")
}

fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("bounded token")
}

fn network_spec() -> NetworkSpec {
    NetworkSpec::new(
        Ipv4Cidr::parse("10.20.0.0/24").expect("lan cidr"),
        Ipv4Cidr::parse("10.20.1.0/30").expect("uplink cidr"),
        None,
        false,
        IsolationSpec::default(),
        RoutingSpec::default(),
        DhcpSpec::default(),
        DnsSpec::default(),
        None,
        MdnsSpec::default(),
        None,
        token("net-vm"),
        vec![NetworkAttachmentEntry::new(reference("Guest/work-vm"), 3, None)
            .expect("reserved attachment entry")],
    )
    .expect("network spec declaring its attached consumer")
}

fn intent() -> NetworkAdmissionIntent {
    NetworkAdmissionIntent::new(
        NetworkAdmissionKey::new(
            uid(HOST_UID),
            uid(NETWORK_UID),
            ResourceGeneration::new(3).expect("network generation"),
            ResourceGeneration::new(7).expect("attachment generation"),
            ResourceBundleGenerationId::parse(INSTALLED_GENERATION).expect("installed generation"),
        ),
        network_spec(),
        vec![uid(GUEST_UID)],
    )
    .expect("root-owned host intent")
}

fn provenance() -> NetworkProvenance {
    intent().key().provenance()
}

/// The committed `NetworkBinding` row this family's own admission derives for
/// the Guest consumer of the fixture Network.
fn committed_binding_row() -> (String, Vec<u8>) {
    let network_ref = reference("Network/lan");
    let spec = network_spec();
    let provenance = provenance();
    let consumers = vec![
        NetworkAdmittedConsumer::new(
            reference("Guest/work-vm"),
            uid(GUEST_UID),
            NetworkPresentation::namespace_interface("eth0")
                .expect("namespace presentation"),
        )
        .expect("admitted fabric consumer"),
    ];
    let rows = canonical_binding_rows(&NetworkBindingSource {
        network_ref: &network_ref,
        zone: &zone(),
        provenance: &provenance,
        spec: &spec,
        consumers: &consumers,
    })
    .expect("the committed Network row derives its binding rows");
    assert_eq!(rows.len(), 1, "one attached consumer implies one binding row");
    (
        rows[0].name().as_str().to_owned(),
        rows[0].spec().to_vec(),
    )
}

/// The stored envelope for one committed binding row: the canonical binding
/// bytes under the serving Provider reference the family declares.
fn stored_row(owner_uid: [u8; 16]) -> StoredDesiredResource {
    let (name, binding_bytes) = committed_binding_row();
    let decoded: NetworkBindingSpec =
        serde_json::from_slice(&binding_bytes).expect("the committed bytes are a NetworkBinding row");
    // The stored envelope is the committed binding bytes with the serving
    // Provider reference added; the base the driver decodes is the rest.
    let mut spec = serde_json::to_value(&decoded).expect("the committed row serializes");
    spec.as_object_mut()
        .expect("a NetworkBinding row is an object")
        .insert(
            "providerRef".to_owned(),
            serde_json::Value::String("Provider/network-local".to_owned()),
        );
    StoredDesiredResource {
        key: ResourceKey::new(ZONE, "NetworkBinding", name),
        uid: [0x42; 16],
        generation: 1,
        owner_uid: Some(owner_uid),
        provenance: ResourceProvenance::Resource,
        deleting: false,
        spec: spec.to_string().into_bytes(),
        metadata: Vec::new(),
        created_at: 0,
    }
}

/// The stored envelope for the parent Network row the binding declares.
fn parent_network_row(uid: [u8; 16]) -> StoredDesiredResource {
    let base = serde_json::to_value(network_spec()).expect("the Network spec serializes");
    StoredDesiredResource {
        key: ResourceKey::new(ZONE, "Network", "lan"),
        uid,
        generation: 3,
        owner_uid: None,
        provenance: ResourceProvenance::Resource,
        deleting: false,
        // The stored envelope carries the serving Provider reference beside the
        // row's own base fields, exactly as the family commits it.
        spec: {
            let mut envelope = base.as_object().cloned().expect("a spec is an object");
            envelope.insert(
                "providerRef".to_owned(),
                serde_json::Value::String("Provider/network-local".to_owned()),
            );
            serde_json::Value::Object(envelope)
        }
        .to_string()
        .into_bytes(),
        metadata: Vec::new(),
        created_at: 0,
    }
}

// ---------------------------------------------------------------------------
// The effect port double
// ---------------------------------------------------------------------------

/// A recording effect port: it answers the admission verb with a real
/// `NetworkEffectContext` bound to the fixture's proof, records every
/// realization and release, and can be told to refuse the realization the way
/// the production implementation does.
struct RecordingEffects {
    realized: AtomicUsize,
    released: AtomicUsize,
    context: NetworkEffectContext,
}

impl RecordingEffects {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            realized: AtomicUsize::new(0),
            released: AtomicUsize::new(0),
            context: NetworkEffectContext::for_network(
                intent().proof(),
                d2b_contracts::types::VmId::new("net-vm"),
                d2b_contracts::types::BundleOpId::new("network-bridge:z:n:token"),
                d2b_contracts::types::BundleOpId::new("network-firewall:z:n:token"),
                d2b_contracts::types::BundleOpId::new("nm-unmanaged:host"),
                d2b_contracts::types::BundleOpId::new("network-hosts:z:n:token"),
                Vec::new(),
                Vec::new(),
                ResourceBundleGenerationId::parse(INSTALLED_GENERATION)
                    .expect("installed generation"),
                [0; 32],
                false,
            ),
        })
    }
}

#[async_trait::async_trait]
impl NetworkBindingDriverEffects for RecordingEffects {
    async fn admission(
        &self,
        _network: &ResourceRef,
    ) -> Result<NetworkEffectContext, NetworkBindingError> {
        Ok(self.context.clone())
    }

    async fn realize_membership(
        &self,
        _context: &NetworkEffectContext,
        _membership: &d2b_provider_network_local::DerivedMembership,
    ) -> Result<(), NetworkBindingError> {
        // The production implementation refuses here because the broker has no
        // operation a binding row's identity can invoke; the double refuses the
        // same way so the driver's handling of that refusal is what the case
        // observes.
        self.realized.fetch_add(1, Ordering::SeqCst);
        Err(NetworkBindingError::MembershipEffectUnavailable)
    }

    async fn release_membership(
        &self,
        _context: &NetworkEffectContext,
        _membership: &d2b_provider_network_local::DerivedMembership,
    ) -> Result<(), NetworkBindingError> {
        self.released.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

struct Fixture {
    ctx: ResourceContext,
}

fn fixture(row: StoredDesiredResource, manager: RecordingManagerEndpoint) -> Fixture {
    let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
    let ctx = ResourceContext::new(
        row,
        network_binding_spec_decoder(),
        Arc::new(manager),
        Arc::new(RecordingRequeue::default()),
        effects_tx,
        notify_tx,
    );
    Fixture { ctx }
}

async fn driver(effects: Arc<RecordingEffects>) -> Box<dyn DynResourceDriver> {
    let factory = NetworkBindingDriverFactory::new(NetworkBindingDriverArgs {
        zone: zone(),
        effects,
    });
    factory
        .create(&ResourceKey::new(ZONE, "NetworkBinding", "row"))
        .await
}

/// A manager holding the parent Network row (owned by the binding's own uid)
/// and the consumer row.
fn consumer_row() -> StoredDesiredResource {
    StoredDesiredResource {
        key: ResourceKey::new(ZONE, "Guest", "work-vm"),
        uid: [0x24; 16],
        generation: 1,
        owner_uid: None,
        provenance: ResourceProvenance::Resource,
        deleting: false,
        spec: b"{}".to_vec(),
        metadata: Vec::new(),
        created_at: 0,
    }
}

/// A manager holding the parent Network row (owned by the binding's own uid)
/// and the consumer row.
fn manager_with_both_rows() -> RecordingManagerEndpoint {
    RecordingManagerEndpoint::new()
        .with_row(parent_network_row([0x42; 16]))
        .with_row(consumer_row())
}

// ---------------------------------------------------------------------------
// The driver serves a committed row
// ---------------------------------------------------------------------------

/// A committed `NetworkBinding` row reaches `validate` and `reconcile`, and
/// the reconcile pass derives the exact membership this family's own
/// admission committed rather than a second one.
#[tokio::test]
async fn a_committed_binding_row_reaches_validate_and_reconcile() {
    let manager = manager_with_both_rows();
    let effects = RecordingEffects::new();
    let mut f = fixture(stored_row([0x42; 16]), manager);
    let mut d = driver(effects.clone()).await;

    d.validate(&mut f.ctx).await.expect("validate accepts the committed row");

    // The realization is refused by name, so the pass reports the serving
    // effect failure rather than claiming a membership the broker never made.
    let failure = d
        .reconcile(&mut f.ctx)
        .await
        .expect_err("reconcile reports the unroutable realization");
    assert_eq!(
        failure.kind(),
        FailureKinds::BINDING_SERVING_EFFECT_FAILED,
        "the missing kernel is reported as the serving-effect failure"
    );
    assert_eq!(
        effects.realized.load(Ordering::SeqCst),
        1,
        "the pass reached the privileged realization verb exactly once"
    );
}

/// The refusal the reconcile pass reports names the missing broker operation,
/// so the gap is diagnosable from the row's own failure rather than guessed
/// at.
#[tokio::test]
async fn the_unroutable_realization_names_the_missing_broker_operation() {
    assert_eq!(
        MISSING_CREATE_MEMBERSHIP_TAP, "create-membership-tap",
        "the missing kernel has one stable name"
    );
    assert_eq!(
        NetworkBindingError::MembershipEffectUnavailable.code(),
        d2b_provider_network_local::MEMBERSHIP_TAP_UNAVAILABLE,
        "the refusal carries its own stable code"
    );
}

// ---------------------------------------------------------------------------
// The committed decision is held to
// ---------------------------------------------------------------------------

/// A row whose committed decision admits no right this family admits for a
/// network relationship is refused terminally, even though every other field
/// decodes and both joined rows resolve. The source would refuse that claim
/// itself, so no retry can make the committed decision true.
#[tokio::test]
async fn a_row_whose_committed_rights_do_not_cover_the_claim_is_refused() {
    let manager = manager_with_both_rows();
    let effects = RecordingEffects::new();
    // Forge the stored row's committed decision: keep the whole valid
    // relationship, admit only a right `BindingKind::Network` does not admit.
    let mut row = stored_row([0x42; 16]);
    let mut envelope: serde_json::Value =
        serde_json::from_slice(&row.spec).expect("the stored envelope is JSON");
    envelope
        .as_object_mut()
        .expect("a stored envelope is an object")
        .insert(
            "source".to_owned(),
            serde_json::json!({
                "admittedRights": ["share"],
                "arbitration": "shared",
                "realizedFacets": ["namespace-interface"],
            }),
        );
    row.spec = envelope.to_string().into_bytes();
    let mut f = fixture(row, manager);
    let mut d = driver(effects).await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("the committed decision does not admit the claim");
    assert_eq!(
        failure.kind(),
        FailureKinds::DRIVER_REFUSED,
        "an uncovered claim is a terminal refusal, not a retry"
    );
    assert_eq!(failure.outcome(), FailureOutcome::Refused);
}

/// A row whose committed decision declares an exclusive claim is refused: the
/// fabric is shared, and this provider arbitrates a membership alongside its
/// peers and never exclusively.
#[tokio::test]
async fn a_row_whose_committed_arbitration_is_exclusive_is_refused() {
    let manager = manager_with_both_rows();
    let effects = RecordingEffects::new();
    let mut row = stored_row([0x42; 16]);
    let mut envelope: serde_json::Value =
        serde_json::from_slice(&row.spec).expect("the stored envelope is JSON");
    envelope
        .as_object_mut()
        .expect("a stored envelope is an object")
        .insert(
            "source".to_owned(),
            serde_json::json!({
                "admittedRights": ["consume"],
                "arbitration": "exclusive",
                "realizedFacets": ["namespace-interface"],
            }),
        );
    row.spec = envelope.to_string().into_bytes();
    let mut f = fixture(row, manager);
    let mut d = driver(effects).await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("an exclusive claim is refused on a shared fabric");
    assert_eq!(failure.kind(), FailureKinds::DRIVER_REFUSED);
    assert_eq!(failure.outcome(), FailureOutcome::Refused);
}

/// A committed row whose declared parent Network is owned by another resource
/// is refused: adopting it would silently re-parent the binding.
#[tokio::test]
async fn a_row_whose_declared_parent_is_owned_elsewhere_is_refused() {
    // The parent row's uid differs from the binding's declared owner.
    let manager = RecordingManagerEndpoint::new()
        .with_row(parent_network_row([0x99; 16]))
        .with_row(consumer_row());
    let effects = RecordingEffects::new();
    let mut f = fixture(stored_row([0x42; 16]), manager);
    let mut d = driver(effects).await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("the owner mismatch is refused");
    assert_eq!(
        failure.kind(),
        FailureKinds::BINDING_OWNER_MISMATCH,
        "a binding whose declared parent is owned elsewhere never re-parents"
    );
    assert_eq!(failure.outcome(), FailureOutcome::Refused);
}

/// A committed row whose consumer row is not committed yet defers retryably
/// rather than failing terminal: the consumer may simply not exist yet.
#[tokio::test]
async fn a_row_whose_consumer_is_not_committed_yet_defers() {
    // Only the parent Network is present; the consumer row is not.
    let manager = RecordingManagerEndpoint::new().with_row(parent_network_row([0x42; 16]));
    let effects = RecordingEffects::new();
    let mut f = fixture(stored_row([0x42; 16]), manager);
    let mut d = driver(effects).await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("the absent consumer defers");
    assert_eq!(
        failure.kind(),
        FailureKinds::TARGET_UNAVAILABLE,
        "an absent consumer is a deferral, never a terminal refusal"
    );
    assert!(
        failure.defers(),
        "issue #511: absence is not terminal"
    );
}

/// A committed row whose parent Network row is not committed yet defers the
/// same way, for the same reason.
#[tokio::test]
async fn a_row_whose_parent_network_is_not_committed_yet_defers() {
    let manager = RecordingManagerEndpoint::new();
    let effects = RecordingEffects::new();
    let mut f = fixture(stored_row([0x42; 16]), manager);
    let mut d = driver(effects).await;

    let failure = d
        .validate(&mut f.ctx)
        .await
        .expect_err("the absent parent defers");
    assert_eq!(failure.kind(), FailureKinds::BINDING_PARENT_UNAVAILABLE);
    assert!(failure.defers(), "issue #511: absence is not terminal");
}

// ---------------------------------------------------------------------------
// Pre-drain and delete
// ---------------------------------------------------------------------------

/// Pre-drain releases the consumer interface through the effect port and
/// converges idempotently: a second pass over the same state releases once
/// more without failing, and the fabric is never touched.
#[tokio::test]
async fn pre_drain_releases_the_consumer_interface_idempotently() {
    let manager = manager_with_both_rows();
    let effects = RecordingEffects::new();
    let mut f = fixture(stored_row([0x42; 16]), manager);
    let mut d = driver(effects.clone()).await;

    d.pre_drain(&mut f.ctx).await.expect("pre-drain");
    assert_eq!(
        effects.released.load(Ordering::SeqCst),
        1,
        "the interface is released through the broker-backed port"
    );

    // Idempotent under retry: the actor may requeue a pre-drain pass, and a
    // second one over the same state converges rather than failing.
    d.pre_drain(&mut f.ctx).await.expect("pre-drain is idempotent");
    d.delete(&mut f.ctx).await.expect("delete converges after pre-drain");
}

/// Delete converges without effects: the pre-drain stage owns the release, so
/// teardown never re-drives the broker.
#[tokio::test]
async fn delete_converges_without_driving_the_broker_again() {
    let manager = manager_with_both_rows();
    let effects = RecordingEffects::new();
    let mut f = fixture(stored_row([0x42; 16]), manager);
    let mut d = driver(effects.clone()).await;

    d.delete(&mut f.ctx).await.expect("delete converges");
    assert_eq!(
        effects.released.load(Ordering::SeqCst),
        0,
        "delete itself drives no broker effect; pre-drain owns the release"
    );
}

// ---------------------------------------------------------------------------
// The type's declaration
// ---------------------------------------------------------------------------

/// The descriptor declares the type, its converted verb surface, and the
/// family operations the membership effects ride.
#[test]
fn the_descriptor_declares_the_binding_type_and_its_family_operations() {
    let descriptor = network_binding_descriptor(NetworkBindingDriverArgs {
        zone: zone(),
        effects: RecordingEffects::new(),
    });
    assert_eq!(
        descriptor.resource_type.to_resource_type_name().as_str(),
        "NetworkBinding"
    );
    assert!(
        descriptor.operations.iter().any(|operation| {
            operation.operation_ref.to_canonical_string() == "Operation/create-persistent-tap"
        }),
        "the membership effects ride the family's committed operation rows"
    );
    assert!(
        descriptor.creations.is_empty(),
        "a membership owns no child row: it holds one interface on a fabric the \
         Network row already realized"
    );
}