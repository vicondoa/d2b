//! The whole Volume binding chain, end to end (U14).
//!
//! One `Volume` row with an attachment is committed, the producing half admits
//! the relationships the source owns and mints one `VolumeBinding` row per
//! admitted relationship, and the registered `d2b-provider-volume-binding`
//! driver decodes that committed row and reconciles it - deriving its worker
//! Process and Endpoint children over the same bytes.
//!
//! Every stage runs the real path: the driver the plane registers through its
//! [`DriverDescriptor`], the real source-side admission, the real child
//! surface, and the real serving reconcile.  The bytes the serving driver
//! reads are the ones the producing half committed, copied whole; nothing in
//! this file rebuilds a row by hand.
//!
//! The second case is the block-device attachment: the same chain commits it,
//! and the virtiofs serving family refuses it by name rather than serving it
//! at a destination invented for it.

use std::sync::Arc;

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume::AttachmentAccess;
use d2b_contracts_resource::v3::{
    BindingAuthorization, BindingRealizationFacet, BindingRealizationSupport, BindingSlot,
    BindingSourceDecision, DesiredDigest, DesiredRevision, FreshnessTuple, RequestedRights,
    ResourceRef, ResourceUid, StoreIncarnation, VolumeBindingRequest, VolumeBindingSpec,
    VolumePresentation, ZoneId,
};
use d2b_provider_toolkit::testing::fakes::{RecordingManagerEndpoint, RecordingRequeue};
use d2b_provider_volume::{
    VolumeDriverArgs, volume_descriptor, volume_spec_decoder,
};
use d2b_provider_volume::test_support::{RecordingRuntime, recording_facets};
use d2b_provider_volume_binding::{
    BindingDriverArgs, binding_descriptor, binding_spec_decoder,
};
use d2b_provider_volume_binding::test_support::FakeServingEffects;
use d2b_provider_volume_local::{
    VolumeAdmissionGrant, VolumeAdmissionSource, VolumeConsumerRequest, admit_consumer_requests,
};
use d2b_resource_runtime::context::ResourceContext;
use d2b_resource_runtime::driver::{DynResourceDriver, ReconcileOutcome};
use d2b_resource_runtime::error::FailureClass;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName, StoredDesiredResource};
use d2b_resource_runtime::provider::ProviderDirectory;
use d2b_resource_runtime::relations::DecodedBindingRequest;

const ZONE: &str = "work";
const VOLUME_TYPE: &str = "Volume";
const BINDING_TYPE: &str = "VolumeBinding";

/// The Zone-local identity the producing half commits its children under, in
/// both the wire spelling the admission fences on and the raw bytes a manager
/// row carries.
const VOLUME_UID_BYTES: [u8; 16] = [0x42; 16];
const CONSUMER_UID: &str = "52525252-5252-4525-8525-525252525252";

fn uid(hex: &str) -> ResourceUid {
    ResourceUid::parse(hex).expect("canonical resource uid")
}

fn reference(value: &str) -> ResourceRef {
    ResourceRef::parse(value).expect("registered resource reference")
}

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("zone")
}

/// The committed `Volume` row: one named view a consumer may write, one
/// read-only raw view a block attachment may consume, and one virtiofs
/// attachment at `/mnt/data`.
fn volume_spec_bytes() -> Vec<u8> {
    serde_json::json!({
        "providerRef": "Provider/volume-local",
        "source": {
            "executionRef": "Host/host-system",
            "settings": { "kind": "local-path", "sourcePolicyId": "state-root" },
        },
        "kind": "durable",
        "layout": [],
        "views": {
            "controller": { "path": "", "rights": ["read", "write", "traverse"] },
            "raw": { "path": "raw", "rights": ["read", "traverse"] },
        },
        "attachments": [{
            "executionRef": "Guest/work-vm",
            "transport": "virtiofs",
            "view": "controller",
            "access": "read-write",
            "mountPath": "/mnt/data",
            "settings": {},
        }],
    })
    .to_string()
    .into_bytes()
}

/// The committed `Volume` row's own spec: the envelope's reserved
/// `providerRef` stripped, exactly as a reader of the stored envelope sees it.
/// The committed row's closed base spec: the reserved envelope fields the
/// producing half stamps beside it are attributed first, exactly as both real
/// readers do.
fn committed_base_spec(bytes: &[u8]) -> VolumeBindingSpec {
    let mut value: serde_json::Value = serde_json::from_slice(bytes).expect("committed row value");
    let object = value.as_object_mut().expect("a committed row is an object");
    for field in ["providerRef", "updatePolicy", "provider"] {
        object.remove(field);
    }
    serde_json::from_value(value).expect("the committed row is the closed row contract")
}

fn volume_spec() -> d2b_contracts_resource::v3::volume::VolumeSpec {
    let mut value: serde_json::Value =
        serde_json::from_slice(&volume_spec_bytes()).expect("the committed Volume spec");
    value
        .as_object_mut()
        .expect("the Volume spec is an object")
        .remove("providerRef");
    serde_json::from_value(value).expect("the committed Volume spec")
}

fn volume_row() -> StoredDesiredResource {
    StoredDesiredResource {
        key: ResourceKey::new(ZONE, VOLUME_TYPE, "data"),
        uid: VOLUME_UID_BYTES,
        generation: 3,
        owner_uid: None,
        provenance: d2b_resource_runtime::identity::ResourceProvenance::Api,
        deleting: false,
        spec: volume_spec_bytes(),
        metadata: Vec::new(),
        created_at: 0,
    }
}

/// The declaration a consumer authors for the declared attachment.
fn attachment_request() -> VolumeBindingRequest {
    VolumeBindingRequest::new(
        reference("Volume/data"),
        reference("Guest/work-vm"),
        BindingSlot::parse("state").expect("bounded consumer slot"),
        BoundedToken::parse("controller").expect("bounded view token"),
        AttachmentAccess::ReadWrite,
        VolumePresentation::filesystem("/mnt/data").expect("consumer destination"),
    )
    .expect("an admitted consumer declaration")
}

/// The declaration a consumer authors for a block-device attachment.
fn block_request() -> VolumeBindingRequest {
    VolumeBindingRequest::new(
        reference("Volume/data"),
        reference("Guest/work-vm"),
        BindingSlot::parse("raw").expect("bounded consumer slot"),
        BoundedToken::parse("raw").expect("bounded view token"),
        AttachmentAccess::ReadOnly,
        VolumePresentation::block_device(1).expect("bounded device slot"),
    )
    .expect("an admitted consumer declaration")
}

/// Admit one declaration through the one source-side admission path, against
/// the authorization and freshness evidence the daemon's authority path holds.
fn admitted(request: &VolumeBindingRequest) -> Vec<d2b_provider_volume_local::AdmittedVolumeBinding> {
    let support = BindingRealizationSupport::new(vec![
        BindingRealizationFacet::FilesystemPresentation,
        BindingRealizationFacet::ConsumerDeviceSlot,
    ])
    .expect("the realization support the family declares");
    let volume_uid = ResourceUid::from_bytes(&VOLUME_UID_BYTES).expect("the row's own uid");
    let consumer_uid = uid(CONSUMER_UID);
    let fence = [
        ("Volume/data", volume_uid.clone()),
        ("Guest/work-vm", consumer_uid.clone()),
    ]
    .into_iter()
    .map(|(name, identity)| {
        FreshnessTuple::new(
            zone(),
            StoreIncarnation::parse("store-one").expect("store incarnation"),
            reference(name),
            identity,
            DesiredRevision::INITIAL,
            DesiredDigest::of(name.as_bytes()),
        )
    })
    .collect::<Vec<_>>();
    let authorization = BindingAuthorization::granted();
    let grant = VolumeAdmissionGrant::new(&support, &authorization, &fence);
    let spec = volume_spec();
    let zone = zone();
    let volume_ref = reference("Volume/data");
    let source = VolumeAdmissionSource::new(
        &zone,
        &volume_ref,
        &volume_uid,
        &spec,
        false,
        &grant,
    );
    admit_consumer_requests(
        &source,
        &[VolumeConsumerRequest::new(consumer_uid, request.clone())],
    )
    .expect("the source admits its own relationship")
}

/// Reconcile the registered `Volume` driver until the layout effect completes,
/// then return the manager it committed its children through.
async fn producing_pass(
    admitted: Vec<d2b_provider_volume_local::AdmittedVolumeBinding>,
) -> RecordingManagerEndpoint {
    let runtime = RecordingRuntime::new();
    runtime.set_admitted(admitted).await;

    let mut providers = ProviderDirectory::new();
    providers
        .register_driver(&volume_descriptor(VolumeDriverArgs {
            facets: recording_facets(runtime.clone()),
        }))
        .expect("the plane registers the Volume type");
    let factory = providers
        .lookup(&ResourceTypeName::new(VOLUME_TYPE))
        .expect("the registry serves the declared factory");
    let mut driver: Box<dyn DynResourceDriver> = factory
        .create(&ResourceKey::new(ZONE, VOLUME_TYPE, "data"))
        .await;

    let manager = RecordingManagerEndpoint::new().with_owner_uid(VOLUME_UID_BYTES);
    let (effects_tx, mut effects) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify) = tokio::sync::mpsc::unbounded_channel();
    let mut ctx = ResourceContext::new(
        volume_row(),
        volume_spec_decoder(),
        Arc::new(manager.clone()),
        Arc::new(RecordingRequeue::default()),
        effects_tx,
        notify_tx,
    );

    driver.validate(&mut ctx).await.expect("the Volume row validates");
    // Pass one spawns the layout effect; the actor re-reconciles when it
    // completes, and that pass mints the binding rows.
    let first = driver.reconcile(&mut ctx).await.expect("the layout pass");
    assert!(matches!(first, ReconcileOutcome::InProgress { .. }), "{first:?}");
    effects.recv().await.expect("the layout effect completes");
    let second = driver.reconcile(&mut ctx).await.expect("the child pass");
    assert_eq!(second, ReconcileOutcome::Satisfied);
    manager
}

/// The committed `VolumeBinding` row the admitted-relationship derivation
/// minted, copied whole out of the manager the producing pass committed it to.
fn committed_admitted_row(manager: &RecordingManagerEndpoint) -> StoredDesiredResource {
    let rows: Vec<StoredDesiredResource> = manager
        .rows()
        .into_iter()
        .filter(|row| {
            row.key.type_name == BINDING_TYPE
                && d2b_provider_volume_local::is_admitted_binding_row_name(&row.key.name)
        })
        .collect();
    assert_eq!(
        rows.len(),
        1,
        "exactly one row per admitted relationship, named from its KTD3 key"
    );
    rows.into_iter().next().expect("the admitted relationship's row")
}

/// Reconcile the registered `VolumeBinding` driver over one committed row.
async fn serving_pass(row: StoredDesiredResource) -> (Result<ReconcileOutcome, ()>, RecordingManagerEndpoint) {
    let manager = RecordingManagerEndpoint::new()
        .with_owner_uid(VOLUME_UID_BYTES)
        .with_parent(VOLUME_UID_BYTES, &volume_spec_bytes());
    let effects = FakeServingEffects::shared(manager.log_handle());

    let mut providers = ProviderDirectory::new();
    providers
        .register_driver(&binding_descriptor(BindingDriverArgs {
            zone: zone(),
            facets: effects.facet_set(),
            vcpu_count: 2,
        }))
        .expect("the plane registers the VolumeBinding type");
    let factory = providers
        .lookup(&ResourceTypeName::new(BINDING_TYPE))
        .expect("the registry serves the declared factory");
    let mut driver: Box<dyn DynResourceDriver> = factory
        .create(&ResourceKey::new(ZONE, BINDING_TYPE, &row.key.name))
        .await;

    let (effects_tx, _effects) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify) = tokio::sync::mpsc::unbounded_channel();
    let mut ctx = ResourceContext::new(
        row,
        binding_spec_decoder(),
        Arc::new(manager.clone()),
        Arc::new(RecordingRequeue::default()),
        effects_tx,
        notify_tx,
    );

    driver.validate(&mut ctx).await.expect("the committed binding row validates");
    let outcome = driver
        .reconcile(&mut ctx)
        .await
        .map_err(|failure| assert_eq!(failure.class(), FailureClass::Terminal));
    (outcome, manager)
}

/// The chain: a committed `Volume` row with an attachment produces a committed
/// `VolumeBinding` row, and the registered serving driver decodes and
/// reconciles exactly those bytes.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_committed_volume_row_produces_a_binding_row_the_registered_driver_serves() {
    let request = attachment_request();
    let producing = producing_pass(admitted(&request)).await;
    let committed = committed_admitted_row(&producing);

    // The committed bytes are the row the manager holds, not a copy a test
    // rebuilt: the serving stage below reads them straight through.
    assert_eq!(committed.spec, producing.row(&committed.key).expect("committed row").spec);
    let row = committed_base_spec(&committed.spec);
    assert_eq!(row.volume_ref(), &reference("Volume/data"));
    assert_eq!(row.execution_ref(), &reference("Guest/work-vm"));
    assert_eq!(row.view().as_str(), "controller");
    assert_eq!(row.slot().as_str(), "state");
    assert_eq!(row.presentation().destination(), Some("/mnt/data"));
    assert_eq!(row.source().admitted_rights(), &[RequestedRights::Mutate]);
    // The manager's own relation-index decoder reads the producing half's
    // bytes as the same relationship, so the row is indexed and served as one
    // relationship rather than two.
    let decoded = DecodedBindingRequest::decode(BINDING_TYPE, &committed.spec)
        .expect("the committed row is an indexable relationship");
    assert_eq!(decoded.source_ref(), row.volume_ref());
    assert_eq!(decoded.consumer_ref(), row.execution_ref());
    assert_eq!(decoded.slot().as_str(), "state");
    assert_eq!(decoded.rights(), RequestedRights::Mutate);

    // The serving stage runs the row exactly as the producing half committed
    // it: same key, uid, generation, owner, and desired bytes.
    let (outcome, serving) = serving_pass(committed).await;
    assert_eq!(
        outcome.expect("the serving driver converged its own work over the committed row"),
        ReconcileOutcome::Satisfied,
    );
    let order = serving.call_order();
    let worker = order
        .iter()
        .position(|entry| entry.starts_with("ensure:Process/"))
        .unwrap_or_else(|| panic!("the worker Process child is committed: {order:?}"));
    let endpoint = order
        .iter()
        .position(|entry| entry.starts_with("ensure:Endpoint/"))
        .unwrap_or_else(|| panic!("the Endpoint child is committed: {order:?}"));
    assert!(worker < endpoint, "the worker is committed before the socket it serves: {order:?}");
    assert!(order.iter().any(|entry| entry == "socket-ready"));
    assert!(order.iter().any(|entry| entry == "guest-mount"));
}

/// The same chain with a block-device attachment: the relationship is a
/// representable committed row, and the virtiofs serving family refuses it by
/// name instead of serving it at a destination invented for it.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_block_device_attachment_commits_its_device_slot_and_is_refused_by_name() {
    let request = block_request();
    let producing = producing_pass(admitted(&request)).await;
    let committed = committed_admitted_row(&producing);

    let row = committed_base_spec(&committed.spec);
    assert_eq!(row.presentation().device_slot(), Some(1));
    assert_eq!(
        row.presentation().destination(),
        None,
        "a block attachment commits its device slot and no destination at all"
    );
    assert_eq!(
        row.source().realized_facets(),
        &[BindingRealizationFacet::ConsumerDeviceSlot]
    );
    let decoded = DecodedBindingRequest::decode(BINDING_TYPE, &committed.spec)
        .expect("a block attachment is an indexable relationship");
    assert_eq!(
        decoded.required_facets(),
        &[BindingRealizationFacet::ConsumerDeviceSlot]
    );

    let (outcome, serving) = serving_pass(committed).await;
    assert!(
        outcome.is_err(),
        "the serving family refuses this row rather than serving it"
    );
    assert_eq!(
        serving
            .call_order()
            .iter()
            .filter(|entry| entry.starts_with("ensure:"))
            .count(),
        0,
        "an unrealizable presentation mints no worker or endpoint child"
    );
}

/// The refused block row is terminal, and its reason is the family's own
/// stable code rather than a generic failure.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_block_device_row_is_refused_terminally_by_the_registered_driver() {
    let producing = producing_pass(admitted(&block_request())).await;
    let committed = committed_admitted_row(&producing);
    let manager = RecordingManagerEndpoint::new()
        .with_parent(VOLUME_UID_BYTES, &volume_spec_bytes());
    let effects = FakeServingEffects::shared(manager.log_handle());

    let mut providers = ProviderDirectory::new();
    providers
        .register_driver(&binding_descriptor(BindingDriverArgs {
            zone: zone(),
            facets: effects.facet_set(),
            vcpu_count: 2,
        }))
        .expect("the plane registers the VolumeBinding type");
    let factory = providers
        .lookup(&ResourceTypeName::new(BINDING_TYPE))
        .expect("the registry serves the declared factory");
    let mut driver: Box<dyn DynResourceDriver> = factory
        .create(&ResourceKey::new(ZONE, BINDING_TYPE, &committed.key.name))
        .await;

    let (effects_tx, _effects) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify) = tokio::sync::mpsc::unbounded_channel();
    let mut ctx = ResourceContext::new(
        committed,
        binding_spec_decoder(),
        Arc::new(manager.clone()),
        Arc::new(RecordingRequeue::default()),
        effects_tx,
        notify_tx,
    );
    driver.validate(&mut ctx).await.expect("the committed row itself decodes");

    let failure = driver
        .reconcile(&mut ctx)
        .await
        .expect_err("a block presentation is not this family's to serve");
    assert_eq!(failure.class(), FailureClass::Terminal);
    assert!(
        manager.call_order().iter().all(|entry| !entry.starts_with("ensure:")),
        "the refusal mints no child"
    );
}

/// The row the source commits carries the source's own accepted decision, so a
/// boundary rebuilding the accepted graph from committed rows alone recovers
/// the rights the source admitted.
#[test]
fn the_committed_row_carries_the_source_s_accepted_decision() {
    let request = attachment_request();
    let admitted = admitted(&request);
    let row: VolumeBindingSpec = serde_json::from_slice(
        &d2b_contracts_resource::v3::canonical_json_bytes(
            &VolumeBindingSpec::new(
                request.source_ref().clone(),
                request.consumer_ref().clone(),
                request.view().as_str(),
                request.access(),
                request.presentation().clone(),
                request.slot().as_str(),
                BindingSourceDecision::new(
                    vec![RequestedRights::Mutate],
                    d2b_contracts_resource::v3::BindingArbitration::Exclusive,
                    vec![BindingRealizationFacet::FilesystemPresentation],
                )
                .expect("the admission's own decision"),
            )
            .expect("the committed row"),
        )
        .expect("canonical row bytes"),
    )
    .expect("the committed row is the closed row contract");
    assert_eq!(row.source().arbitration(), admitted[0].admission().arbitration());
    assert_eq!(
        row.source().admitted_rights(),
        std::slice::from_ref(&admitted[0].admission().rights()),
    );
    assert_eq!(
        row.source().realized_facets(),
        admitted[0].realized_facets(),
        "the row commits the facets the admission proved, not the request's wish"
    );
}
