//! The `Device` source's producing half: the `DeviceBinding` rows one
//! committed `Device` row owns.
//!
//! The derivation (`canonical_binding_rows`) and the serving driver were both
//! in place while no verb called it, so no row was ever committed. These tests
//! pin the three properties that verb has to earn, over the manager's own child
//! surface:
//!
//! 1. the first pass commits exactly the row the admitted relationship
//!    derives, named from the relationship key rather than from a declaration
//!    order;
//! 2. a second pass over an unchanged row commits nothing new, because the
//!    name and the bytes both derive;
//! 3. a row that no longer derives is retired - whether the declaration was
//!    withdrawn or the host stopped backing the capability - and nothing else
//!    the source owns is touched.
//!
//! And the negative case, which is what the daemon can reach today: with no
//! admission evidence the pass commits nothing, retires nothing, and names the
//! fact that is missing instead of self-granting it.

use std::sync::Arc;

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::{
    BindingArbitration, BindingAuthorization, BindingRealizationFacet, BindingSlot,
    BindingSourceDecision, ControllerGeneration, DesiredDigest, DesiredRevision, DeviceAttachmentMode,
    DeviceBindingRequest, DeviceBindingSpec, DeviceClaimRequest, DeviceFunction, FreshnessTuple,
    RequestedRights, ResourceGeneration, ResourceRef, ResourceUid, StoreIncarnation, ZoneId,
    canonical_json_bytes,
};
use d2b_provider_device::binding::{DeviceInventory, DevicePresence, binding_row_name};
use d2b_provider_device::test_support::{RecordingInventory, RecordingRuntime, fixed_facets};
use d2b_provider_device::{
    BindingProduction, BindingProductionRefusal, DEVICE_BINDING_TYPE_NAME, DeviceComponent,
    DeviceDeclaredBindings, DeviceDriverArgs, device_descriptor, produce_binding_rows,
};
use d2b_provider_toolkit::shared_provider::{
    ContextChildSurface, SharedProviderEffectRequest, shared_provider_spec_decoder,
};
use d2b_provider_toolkit::testing::fakes::{RecordingManagerEndpoint, RecordingRequeue};
use d2b_resource_runtime::context::ResourceContext;
use d2b_resource_runtime::driver::DynResourceDriver;
use d2b_resource_runtime::identity::{
    ResourceKey, ResourceProvenance, StoredDesiredResource,
};
use d2b_resource_runtime::manager::deterministic_uid;

/// The zone every row in this file lives in.
const ZONE: &str = "dev";
/// The `Device` row the whole family is exercised through.
const DEVICE: &str = "gpu-zero";
/// The consumer the relationship is admitted for.
const WORKER: &str = "gpu-worker";
/// The GPU Provider the parent `Device` row's own `providerRef` names.
const GPU_PROVIDER: &str = d2b_provider_device_gpu::PROVIDER_REF;
/// The named capability the relationship claims.
const CLAIMED: &str = "render-node";
/// The consumer slot the request occupies.
const SLOT: &str = "gpu-render";

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("bounded zone")
}

fn store() -> StoreIncarnation {
    StoreIncarnation::parse("store-one").expect("bounded store incarnation")
}

fn device_key() -> ResourceKey {
    ResourceKey::new(ZONE, "Device", DEVICE)
}

fn device_ref() -> ResourceRef {
    ResourceRef::parse(&format!("Device/{DEVICE}")).expect("typed reference")
}

fn worker_ref() -> ResourceRef {
    ResourceRef::parse(&format!("Process/{WORKER}")).expect("typed reference")
}

fn worker_uid() -> ResourceUid {
    ResourceUid::parse("223e4567-e89b-42d3-a456-426614174009").expect("canonical uid")
}

fn claimed() -> DeviceFunction {
    DeviceFunction::parse(CLAIMED).expect("bounded function token")
}

/// The committed `Device` row: the stored envelope carries the Provider its
/// delivery is scoped to, which is the one thing that selects the realizing
/// component, plus the DRM selector whose label the recording inventory
/// resolves under.
fn device_row_bytes() -> Vec<u8> {
    serde_json::json!({
        "providerRef": GPU_PROVIDER,
        "deviceClass": "physical",
        "arbitration": "exclusive",
        "maxConcurrentClaims": 1,
        "inventory": { "selector": { "busClass": "drm", "label": "recorded", "pciSlot": null } }
    })
    .to_string()
    .into_bytes()
}

/// The committed `Device` row the driver reconciles.
fn device_row() -> StoredDesiredResource {
    StoredDesiredResource {
        key: device_key(),
        uid: deterministic_uid(&device_key()),
        generation: 3,
        owner_uid: None,
        provenance: ResourceProvenance::Api,
        deleting: false,
        spec: device_row_bytes(),
        metadata: Vec::new(),
        created_at: 0,
    }
}

/// The store-assigned identity the committed row carries.
fn device_uid() -> ResourceUid {
    ResourceUid::from_bytes(&device_row().uid).expect("the row's own durable identity")
}

/// The dependency fence one admission is evaluated against: the source itself
/// and the consumer the request names.
fn fence() -> Vec<FreshnessTuple> {
    let mut dependencies = vec![FreshnessTuple::new(
        zone(),
        store(),
        device_ref(),
        device_uid(),
        DesiredRevision::INITIAL,
        DesiredDigest::of(b"device"),
    )];
    dependencies.push(FreshnessTuple::new(
        zone(),
        store(),
        worker_ref(),
        worker_uid(),
        DesiredRevision::INITIAL,
        DesiredDigest::of(b"worker"),
    ));
    dependencies
}

/// The one consumer request the Zone declares for this source.
fn worker_request() -> DeviceBindingRequest {
    DeviceBindingRequest::new(
        device_ref(),
        worker_ref(),
        BindingSlot::parse(SLOT).expect("bounded slot token"),
        claimed(),
        DeviceClaimRequest::Exclusive,
        DeviceAttachmentMode::Descriptor,
    )
    .expect("an exclusive device claim over an admitted consumer is constructible")
}

/// The name the committed row carries: derived from the relationship key, so
/// the assertion is the family's own derivation and not a literal.
fn derived_row_name() -> String {
    let key = worker_request()
        .key(zone(), device_uid(), worker_uid())
        .expect("the request names a key");
    binding_row_name(&key)
        .expect("a bounded row name")
        .as_str()
        .to_owned()
}

fn binding_key() -> ResourceKey {
    ResourceKey::new(ZONE, DEVICE_BINDING_TYPE_NAME, derived_row_name())
}

/// The manager the pass commits through, keyed under this row's own uid so an
/// owned child is visible to the next pass.
fn manager() -> RecordingManagerEndpoint {
    RecordingManagerEndpoint::new()
        .with_zone(ZONE)
        .with_owner_uid(device_row().uid)
        .with_row(device_row())
}

fn device_fixture(manager: RecordingManagerEndpoint) -> ResourceContext {
    let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
    ResourceContext::new(
        device_row(),
        shared_provider_spec_decoder(),
        Arc::new(manager),
        Arc::new(RecordingRequeue::default()),
        effects_tx,
        notify_tx,
    )
}

/// One producing pass over the row, through the manager's own child surface.
async fn one_pass(
    ctx: &mut ResourceContext,
    inventory: &DeviceInventory,
    declared: &DeviceDeclaredBindings,
) -> BindingProduction {
    let owned = ctx.children().await.expect("the manager answers the owned set");
    let surface = ContextChildSurface::new(ctx);
    let spec: serde_json::Value =
        serde_json::from_slice(&device_row_bytes()).expect("the stored row is canonical json");
    let request = SharedProviderEffectRequest {
        zone: zone(),
        target: device_key(),
        uid: device_uid(),
        generation: ResourceGeneration::new(3).expect("bounded generation"),
        operation_id: format!("device-binding:{DEVICE}"),
        spec: &spec,
        metadata: serde_json::Value::Object(serde_json::Map::new()),
        status: None,
        children: &surface,
    };
    produce_binding_rows(
        &request,
        DeviceComponent::Gpu,
        inventory,
        declared,
        &owned,
    )
    .await
    .expect("the producing pass runs")
}

/// The inventory with every named capability of the family's vocabulary
/// present.
fn present_inventory() -> DeviceInventory {
    RecordingInventory
        .resolved_for(GPU_PROVIDER)
        .expect("the recording inventory resolves the row's declared selector")
}

/// The same inventory with the claimed capability gone from the host.
fn absent_inventory() -> DeviceInventory {
    present_inventory()
        .with_presence(&claimed(), DevicePresence::Absent)
        .expect("the claimed capability is one this inventory resolved")
}

/// The declarations with the evidence an authority would supply.
fn declared(requests: Vec<(ResourceUid, DeviceBindingRequest)>) -> DeviceDeclaredBindings {
    DeviceDeclaredBindings::admitted(requests, BindingAuthorization::granted(), fence())
}

/// The first pass commits the row the admitted relationship derives, and a
/// second pass over the unchanged row commits nothing new.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn an_admitted_relationship_commits_one_row_and_a_second_pass_commits_nothing_new() {
    let inventory = present_inventory();
    let declaration = declared(vec![(worker_uid(), worker_request())]);
    let mut ctx = device_fixture(manager());

    let first = one_pass(&mut ctx, &inventory, &declaration).await;
    assert_eq!(first.refusal(), None, "the evidence is present, so nothing refused");
    assert_eq!(
        first
            .committed()
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>(),
        vec![derived_row_name()],
        "exactly the row the relationship key derives"
    );
    assert!(first.retired().is_empty(), "the source owned no row yet");

    // The committed row is the family's own `DeviceBindingSpec`, carrying the
    // request's identities and the source's accepted decision.
    let key = binding_key();
    let committed = ctx
        .get(&key)
        .await
        .expect("the manager answers the read")
        .expect("the row committed through the child surface");
    assert_eq!(committed.generation, 1, "a first commit");
    let spec: DeviceBindingSpec =
        serde_json::from_slice(&committed.spec).expect("the committed row is the family's spec");
    assert_eq!(spec.device_ref().name().as_str(), DEVICE);
    assert_eq!(
        spec.execution_ref().to_canonical_string(),
        worker_ref().to_canonical_string()
    );
    assert_eq!(spec.function(), &claimed());
    assert_eq!(spec.claim(), &DeviceClaimRequest::Exclusive);
    assert_eq!(spec.slot().as_str(), SLOT);

    // A second pass over the unchanged row: same name, same bytes, so the
    // manager answers `Unchanged` and the row is not rewritten.
    let second = one_pass(&mut ctx, &inventory, &declaration).await;
    assert_eq!(second.refusal(), None);
    assert_eq!(
        second
            .committed()
            .iter()
            .map(|name| name.as_str())
            .collect::<Vec<_>>(),
        vec![derived_row_name()],
        "the derived set is unchanged"
    );
    assert!(second.retired().is_empty());
    assert!(!second.mutated(), "an unchanged parent mutates nothing");
    let after = ctx
        .get(&key)
        .await
        .expect("the manager answers the read")
        .expect("the row survives");
    assert_eq!(after.generation, 1, "an unchanged ensure does not rewrite the row");
    assert_eq!(after.spec, committed.spec, "the committed bytes are the derived ones");
}

/// A `Device` row whose own spec does not decode as a device spec declares no
/// capability vocabulary. The pass then derives nothing and proves nothing,
/// so it retires nothing either: the row is broken for the rest of this
/// family, and the producing half does not get to decide that on its own.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_source_row_that_does_not_decode_derives_nothing_and_retires_nothing() {
    let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
    let row = StoredDesiredResource {
        spec: serde_json::json!({ "providerRef": GPU_PROVIDER })
            .to_string()
            .into_bytes(),
        ..device_row()
    };
    let manager = RecordingManagerEndpoint::new()
        .with_zone(ZONE)
        .with_owner_uid(device_row().uid)
        .with_row(row.clone())
        .with_row(committed_binding_row());
    let mut ctx = ResourceContext::new(
        row.clone(),
        shared_provider_spec_decoder(),
        Arc::new(manager),
        Arc::new(RecordingRequeue::default()),
        effects_tx,
        notify_tx,
    );
    let owned = ctx.children().await.expect("the manager answers the owned set");
    let surface = ContextChildSurface::new(&mut ctx);
    let spec: serde_json::Value =
        serde_json::from_slice(&row.spec).expect("the stored row is canonical json");
    let request = SharedProviderEffectRequest {
        zone: zone(),
        target: device_key(),
        uid: device_uid(),
        generation: ResourceGeneration::new(3).expect("bounded generation"),
        operation_id: format!("device-binding:{DEVICE}"),
        spec: &spec,
        metadata: serde_json::Value::Object(serde_json::Map::new()),
        status: None,
        children: &surface,
    };

    let pass = produce_binding_rows(
        &request,
        DeviceComponent::Gpu,
        &present_inventory(),
        &declared(vec![(worker_uid(), worker_request())]),
        &owned,
    )
    .await
    .expect("the pass runs");
    assert_eq!(pass.refusal(), Some(BindingProductionRefusal::SourceSpecUndecodable));
    assert!(pass.committed().is_empty());
    assert!(pass.retired().is_empty(), "a row this pass cannot read is not this pass's to retire");
}

/// Withdrawing the declaration retires the row the source still owns: the
/// derived set shrank to nothing, so nothing this row owns stays behind.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_withdrawn_declaration_retires_the_row_the_source_owns() {
    let inventory = present_inventory();
    let mut ctx = device_fixture(manager());
    let key = binding_key();

    one_pass(&mut ctx, &inventory, &declared(vec![(worker_uid(), worker_request())])).await;

    // The Zone still authorizes this source and now declares no relationship
    // for it. That is an answer, not a missing one.
    let pass = one_pass(&mut ctx, &inventory, &declared(Vec::new())).await;
    assert_eq!(pass.refusal(), None, "an empty declaration is evaluated, not refused");
    assert!(pass.committed().is_empty());
    assert_eq!(
        pass.retired().to_vec(),
        vec![key.clone()],
        "the row no longer derives, so it is retired"
    );
    assert!(
        ctx.get(&key).await.expect("the manager answers the read").is_none(),
        "the manager retired the row"
    );
}

/// A capability the host no longer backs retires its row even when the
/// admission could not be evaluated: presence is an observation and needs no
/// authority. A capability that is still backed is kept, because the pass
/// cannot read a withdrawal it was never told about.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn a_capability_the_host_no_longer_backs_retires_its_row() {
    let declaration = declared(vec![(worker_uid(), worker_request())]);
    let key = binding_key();

    let mut revoked = device_fixture(manager());
    one_pass(&mut revoked, &present_inventory(), &declaration).await;
    let gone = one_pass(
        &mut revoked,
        &absent_inventory(),
        &DeviceDeclaredBindings::undeclared(),
    )
    .await;
    assert_eq!(
        gone.refusal(),
        Some(BindingProductionRefusal::AuthorizationEvidenceAbsent),
        "the pass names the evidence it was not given"
    );
    assert!(gone.committed().is_empty(), "an absent observation admits nothing");
    assert_eq!(
        gone.retired(),
        std::slice::from_ref(&key),
        "the revoked capability retires its row"
    );
    assert!(revoked.get(&key).await.expect("the manager answers the read").is_none());

    let mut kept = device_fixture(manager());
    one_pass(&mut kept, &present_inventory(), &declaration).await;
    let retained = one_pass(
        &mut kept,
        &present_inventory(),
        &DeviceDeclaredBindings::undeclared(),
    )
    .await;
    assert!(retained.retired().is_empty(), "a backed capability keeps its row");
    assert!(kept.get(&key).await.expect("the manager answers the read").is_some());
}

/// Without its admission evidence the source commits nothing, retires
/// nothing, and says which fact is missing.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn without_evidence_nothing_is_committed_and_the_refusal_names_what_is_missing() {
    let inventory = present_inventory();

    // No authorization: the graph authority mints nothing a driver could read.
    let mut ctx = device_fixture(manager());
    let refused = one_pass(&mut ctx, &inventory, &DeviceDeclaredBindings::undeclared()).await;
    assert_eq!(
        refused.refusal(),
        Some(BindingProductionRefusal::AuthorizationEvidenceAbsent)
    );
    assert!(refused.committed().is_empty());
    assert!(refused.retired().is_empty());
    assert!(!refused.mutated());
    assert!(
        ctx.children().await.expect("the manager answers the owned set").is_empty(),
        "a self-granting source would have committed a row here"
    );

    // Authorization without the freshness fence is refused by name too: the
    // fence's desired revision and digest are the authority journal's.
    let unfenced = DeviceDeclaredBindings::admitted(
        vec![(worker_uid(), worker_request())],
        BindingAuthorization::granted(),
        Vec::new(),
    );
    let mut unfenced_ctx = device_fixture(manager());
    let refused = one_pass(&mut unfenced_ctx, &inventory, &unfenced).await;
    assert_eq!(
        refused.refusal(),
        Some(BindingProductionRefusal::FreshnessFenceAbsent)
    );
    assert!(refused.committed().is_empty());
    assert!(refused.retired().is_empty());
}

/// The committed bytes of one already-admitted relationship, as the family's
/// own row renders them.
fn committed_binding_bytes() -> Vec<u8> {
    let decision = BindingSourceDecision::new(
        vec![RequestedRights::Exclusive],
        BindingArbitration::Exclusive,
        vec![BindingRealizationFacet::DeviceAttachment],
    )
    .expect("the source's own decision is well formed");
    let spec = DeviceBindingSpec::new(
        device_ref(),
        worker_ref(),
        claimed(),
        DeviceClaimRequest::Exclusive,
        BoundedToken::parse(SLOT).expect("bounded slot token"),
        decision,
    )
    .expect("the family spec is well formed");
    canonical_json_bytes(&spec).expect("the row renders")
}

/// One already-committed relationship, owned by the `Device` row.
fn committed_binding_row() -> StoredDesiredResource {
    let key = binding_key();
    StoredDesiredResource {
        uid: deterministic_uid(&key),
        generation: 1,
        owner_uid: Some(device_row().uid),
        provenance: ResourceProvenance::Resource,
        deleting: false,
        spec: committed_binding_bytes(),
        metadata: Vec::new(),
        created_at: 0,
        key,
    }
}

/// The Zone-declared worker row the Device row owns. It is `Nix` provenance
/// with its own metadata, and nothing this family's producing pass may touch.
fn declared_worker_row() -> StoredDesiredResource {
    let key = ResourceKey::new(ZONE, "Process", WORKER);
    StoredDesiredResource {
        uid: deterministic_uid(&key),
        generation: 1,
        owner_uid: Some(device_row().uid),
        provenance: ResourceProvenance::Nix,
        deleting: false,
        spec: b"{}".to_vec(),
        metadata: b"{\"declared\":\"zone\"}".to_vec(),
        created_at: 0,
        key,
    }
}

/// A manager holding the `Device` row plus the given owned rows.
fn manager_with(owned: Vec<StoredDesiredResource>) -> RecordingManagerEndpoint {
    let manager = RecordingManagerEndpoint::new()
        .with_zone(ZONE)
        .with_owner_uid(device_row().uid)
        .with_row(device_row());
    owned.into_iter().fold(manager, |manager, row| manager.with_row(row))
}

/// The `Device` driver over the production facet construction.
async fn device_driver(
    runtime: Arc<RecordingRuntime>,
    inventory: DeviceInventory,
) -> Box<dyn DynResourceDriver> {
    device_descriptor(DeviceDriverArgs {
        zone: zone(),
        controller_generation: ControllerGeneration::new(1).expect("bounded generation"),
        facets: fixed_facets(runtime, inventory),
    })
    .factory
    .create(&device_key())
    .await
}

/// The driver's own reconcile verb, over the production construction, runs the
/// producing pass: it commits nothing without evidence, and it retires a
/// relationship whose capability the host no longer backs while leaving the
/// Zone-declared rows this source owns alone.
#[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
#[tokio::test]
async fn the_driver_reconcile_retires_a_revoked_row_and_commits_nothing_without_evidence() {
    let runtime = Arc::new(RecordingRuntime::default());

    // Nothing owned yet, and no evidence to admit anything: the pass is inert.
    let mut empty = device_fixture(manager());
    device_driver(Arc::clone(&runtime), present_inventory())
        .await
        .reconcile(&mut empty)
        .await
        .expect("the pass converges");
    assert!(empty.children().await.expect("the manager answers").is_empty());

    // A committed relationship whose capability the host no longer backs
    // retires, and the Zone-declared worker row beside it does not.
    let mut revoked = device_fixture(manager_with(vec![
        committed_binding_row(),
        declared_worker_row(),
    ]));
    device_driver(Arc::clone(&runtime), absent_inventory())
        .await
        .reconcile(&mut revoked)
        .await
        .expect("the pass converges");
    assert!(
        revoked
            .get(&binding_key())
            .await
            .expect("the manager answers the read")
            .is_none(),
        "the revoked capability retires its binding row"
    );
    let worker = declared_worker_row().key;
    assert!(
        revoked.get(&worker).await.expect("the manager answers the read").is_some(),
        "the Zone-declared worker row is not this pass's to retire"
    );

    // The same row over an inventory that still backs the capability survives:
    // an evaluation that could not run is not a withdrawal.
    let mut kept = device_fixture(manager_with(vec![committed_binding_row()]));
    device_driver(runtime, present_inventory())
        .await
        .reconcile(&mut kept)
        .await
        .expect("the pass converges");
    assert!(
        kept.get(&binding_key())
            .await
            .expect("the manager answers the read")
            .is_some(),
        "a capability the host still backs keeps its row"
    );
}
