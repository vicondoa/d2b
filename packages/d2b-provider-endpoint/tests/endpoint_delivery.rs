//! The whole exact-endpoint relationship, end to end through production code.
//!
//! One case drives the entire path a committed `Endpoint` row owns:
//!
//! 1. the real `EndpointDriver` reconcile pass derives the `EndpointBinding`
//!    rows the row's own consumer policy declares and commits them through the
//!    manager's child surface, as the exact canonical row bytes;
//! 2. a repeat pass over the unchanged row commits nothing new, and a row whose
//!    consumer policy no longer names that consumer has its relationship
//!    retired;
//! 3. the real `EndpointBindingDriver` serving pass builds the typed wire
//!    request and hands it to the declared dispatch facet;
//! 4. that facet drives the broker's OWN resolution path -
//!    [`accept_endpoint_access`] - over the wire codec, so the answer is the
//!    broker's and not this test's;
//! 5. the serving pass reconciles against that answer and publishes it.
//!
//! What the broker answers depends on the consumer row's host account. A
//! committed consumer's principal is resolved from the real account it runs as
//! through the host account database, so on a host that has provisioned none
//! for this fixture's row the accept path refuses it by name and no ACL entry
//! is written anywhere in the tree; the delivery half of this lane is a
//! host-lane proof and is stated as such rather than asserted around.
//!
//! The negatives are the same relationship seen from the other side: a request
//! whose authority key does not reproduce its own committed facts is refused
//! before any path is resolved, a relationship whose consumer resolves to no
//! account is refused before any socket path is resolved either, and an
//! `Endpoint` row that is gone takes its relationship with it.


use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use d2b_broker::ops::endpoint_access::{EndpointAccessError, accept_endpoint_access};
use d2b_contracts_broker::broker_wire::{
    BrokerRequest, EndpointAccessRequest, EndpointAccessResponse, EndpointAccessVerb,
    endpoint_access_authority_binding,
};
use d2b_contracts_resource::v3::endpoint_binding::EndpointBindingSpec;
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::resource_schema::{
    CanonicalJsonValue, canonical_json_bytes, framed_canonical_digest,
};
use d2b_contracts_resource::v3::{BoundedText, ResourceRef, ResourceUid};
use d2b_core::bundle::{Bundle, BundleGeneration};
use d2b_core::bundle_resolver::BundleResolver;
use d2b_core::manifest_v04::ManifestV04;
use d2b_core::processes::ProcessesJson;
use d2b_provider_endpoint::endpoint::{
    EndpointAttachmentPolicy, EndpointClass, EndpointConsumerPolicy, EndpointLifecyclePolicy,
    EndpointLocality, EndpointOperation, EndpointSpec, EndpointTransport, EndpointVisibility,
    RealizationIncarnation,
};
use d2b_resource_types::WellKnownType;

use d2b_provider_endpoint::{
    CommittedEndpointShape, CommittedEndpointShapeSource, DeviceWorkerEvidenceSource,
    ENDPOINT_BINDING_TYPE_NAME, EndpointAccessDispatch, EndpointAccessDispatchError,
    EndpointBindingDriverArgs, EndpointBindingDriverFactory, EndpointBindingDriverStatus,
    EndpointDeliveryRefusal, EndpointDriverArgs, EndpointDriverEffects, EndpointDriverFactory,
    EndpointDriverStatus, EndpointPurposeVocabulary, EndpointRealization, EndpointSocketIdentity,
    EndpointSocketSource, GuestControlProducer, GuestVmmEvidenceSource, VIRTIOFSD_PURPOSE,
    canonical_binding_row, declared_endpoint_bindings, endpoint_binding_descriptor,
    endpoint_binding_spec_decoder, endpoint_delivery_slot, endpoint_spec_decoder,
};
use d2b_resource_runtime::context::{
    ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, SpecDecoder,
    WatchId, WatchRegistration,
};
use d2b_resource_runtime::driver::{DynResourceDriver, ResourceDriverFactory};
use d2b_resource_runtime::error::{FailureClass, FailureKinds};
use d2b_resource_runtime::error::ResourceError;
use d2b_resource_runtime::identity::{
    ResourceKey, ResourceProvenance, StoredDesiredResource,
};
use d2b_resource_runtime::manager::ResourceView;
use d2b_resource_runtime::resource::ResourceStatus;
use d2b_resource_runtime::spec_store::EnsureOutcome;

// ---------------------------------------------------------------------------
// Fixture facts
// ---------------------------------------------------------------------------

const ZONE: &str = "work";
const ZONE_UID: &str = "11111111-1111-4111-8111-111111111111";
const ENDPOINT: &str = "Endpoint/compositor";
const PRODUCER: &str = "Process/compositor";
const CONSUMER: &str = "Process/frontend";
/// A second consumer of the same endpoint: what the owning row's narrowed
/// policy still admits once the fixture's own consumer is withdrawn from it.
const OTHER_CONSUMER: &str = "Process/shell";
/// A sibling socket in the broker's own endpoint directory, which the admitted
/// consumer must never gain anything on.
const SIBLING: &str = "endpoint-slot-ffffffffffff";
/// An alternate absolute socket outside the broker's tree entirely.
const ALTERNATE_ABSOLUTE: &str = "attacker.sock";

/// The refusal the broker's own accept path answers with when the committed
/// consumer row's principal resolves to no host account: the closed
/// consumer-principal class, then the broker's slug for a committed row it
/// could not resolve a principal for.
const REFUSED_CONSUMER_PRINCIPAL: &str =
    "endpoint-access-consumer-principal/consumer-principal-row-unresolved";

// ---------------------------------------------------------------------------
// The socket effect port
// ---------------------------------------------------------------------------

/// The declared host socket port, scripted to report the endpoint as already
/// realized so the source pass reaches the child-surface commit without
/// waiting on a socket bind this case is not about.
struct RealizedSocketEffects;

#[async_trait::async_trait]
impl EndpointPurposeVocabulary for RealizedSocketEffects {
    fn guest_control_producer(&self, _purpose: &str) -> Option<GuestControlProducer> {
        None
    }

    fn device_worker_endpoint_class(&self, _purpose: &str) -> Option<EndpointClass> {
        None
    }
}

#[async_trait::async_trait]
impl EndpointSocketSource for RealizedSocketEffects {
    async fn present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
        true
    }

    async fn ensure(&self, _producer_ref: &ResourceRef, _purpose: &str) -> Result<(), String> {
        Ok(())
    }

    async fn remove(&self, _producer_ref: &ResourceRef, _purpose: &str) -> Result<(), String> {
        Ok(())
    }
}

#[async_trait::async_trait]
impl GuestVmmEvidenceSource for RealizedSocketEffects {
    async fn present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
        true
    }
}

#[async_trait::async_trait]
impl DeviceWorkerEvidenceSource for RealizedSocketEffects {
    async fn present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
        true
    }
}

#[async_trait::async_trait]
impl EndpointDriverEffects for RealizedSocketEffects {
    async fn socket_present(&self, _producer_ref: &ResourceRef, _purpose: &str) -> bool {
        true
    }

    async fn ensure_socket(&self, _producer_ref: &ResourceRef, _purpose: &str) -> Result<(), String> {
        Ok(())
    }

    async fn remove_socket(&self, _producer_ref: &ResourceRef, _purpose: &str) -> Result<(), String> {
        Ok(())
    }
}

/// The daemon-supplied facet set the driver is built from.
///
/// The socket and both evidence facets answer through the one scripted double,
/// so the driver under test is the production construction and only the host
/// effect is a double. No Provider vocabulary is installed here: this is the
/// closed composition, and the Provider-committed lane below installs one.
fn realized_facets() -> d2b_provider_endpoint::EndpointEffectFacets {
    d2b_provider_endpoint::EndpointEffectFacets::new(
        Arc::new(RealizedSocketEffects),
        Arc::new(RealizedSocketEffects),
        Arc::new(RealizedSocketEffects),
    )
}

// ---------------------------------------------------------------------------
// The committed graph, over an in-memory manager
// ---------------------------------------------------------------------------

/// The manager endpoint one committed row's children and reads ride.
///
/// This is the production `ResourceContext` construction with the manager seam
/// held by a recording double, so the driver under test is the real one and
/// every child mutation it performs is observable.
struct RecordingManager {
    rows: tokio::sync::Mutex<Vec<StoredDesiredResource>>,
    views: tokio::sync::Mutex<Vec<ResourceView>>,
    ensured: tokio::sync::Mutex<Vec<ChildEnsure>>,
    deleted: tokio::sync::Mutex<Vec<ResourceKey>>,
    watched: tokio::sync::Mutex<Vec<ResourceKey>>,
}

impl RecordingManager {
    fn with(rows: Vec<StoredDesiredResource>) -> Arc<Self> {
        Self::with_views(rows, Vec::new())
    }

    fn with_views(
        rows: Vec<StoredDesiredResource>,
        views: Vec<ResourceView>,
    ) -> Arc<Self> {
        Arc::new(Self {
            rows: tokio::sync::Mutex::new(rows),
            views: tokio::sync::Mutex::new(views),
            ensured: tokio::sync::Mutex::new(Vec::new()),
            deleted: tokio::sync::Mutex::new(Vec::new()),
            watched: tokio::sync::Mutex::new(Vec::new()),
        })
    }


    async fn ensured(&self) -> Vec<ChildEnsure> {
        self.ensured.lock().await.to_vec()
    }

    async fn deleted(&self) -> Vec<ResourceKey> {
        self.deleted.lock().await.to_vec()
    }

    async fn rows(&self) -> Vec<StoredDesiredResource> {
        self.rows.lock().await.to_vec()
    }

    async fn bindings(&self) -> Vec<StoredDesiredResource> {
        self.rows().await
            .into_iter()
            .filter(|row| row.key.type_name == ENDPOINT_BINDING_TYPE_NAME)
            .collect()
    }

    /// Drop one committed row, the way a consumer that has gone away leaves
    /// the store while a relationship that names it is still committed.
    async fn remove_row(&self, key: &ResourceKey) {
        self.rows.lock().await.retain(|row| row.key != *key);
    }
}

#[async_trait::async_trait]
impl ManagerEndpoint for RecordingManager {
    async fn ensure_child(
        &self,
        parent: &ResourceKey,
        child: ChildEnsure,
    ) -> Result<EnsureOutcome, ResourceError> {
        let key = ResourceKey::new(&parent.zone, child.type_name.as_str(), &child.name);
        self.ensured.lock().await.push(child.clone());
        let mut rows = self.rows.lock().await;
        let committed = |generation: u64| StoredDesiredResource {
            key: key.clone(),
            uid: [0x51; 16],
            generation,
            owner_uid: Some([0x42; 16]),
            provenance: ResourceProvenance::Resource,
            deleting: false,
            spec: child.spec.clone(),
            metadata: child.metadata.clone(),
            created_at: 0,
        };
        match rows.iter().position(|row| row.key == key) {
            // Byte-identical desired state is the manager's `Unchanged` answer
            // and leaves the row's generation where it was: that is what makes
            // a second pass over an unchanged source commit nothing new.
            Some(index)
                if rows[index].spec == child.spec && rows[index].metadata == child.metadata =>
            {
                Ok(EnsureOutcome::Unchanged(rows[index].clone()))
            }
            Some(index) => {
                let row = committed(rows[index].generation + 1);
                rows[index] = row.clone();
                Ok(EnsureOutcome::Updated(row))
            }
            None => {
                let row = committed(1);
                rows.push(row.clone());
                Ok(EnsureOutcome::Created(row))
            }
        }
    }

    async fn get(&self, key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
        Ok(self
            .rows
            .lock().await
            .iter()
            .find(|row| row.key == *key)
            .cloned())
    }

    async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
        Ok(self
            .views
            .lock()
            .await
            .iter()
            .find(|view| view.key == *key)
            .cloned())
    }


    async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
        self.deleted.lock().await.push(key.clone());
        for row in self.rows.lock().await.iter_mut() {
            if row.key == *key {
                row.deleting = true;
            }
        }
        Ok(())
    }

    async fn list_owned(
        &self,
        owner_uid: [u8; 16],
    ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        Ok(self
            .rows().await
            .into_iter()
            .filter(|row| row.owner_uid == Some(owner_uid))
            .collect())
    }

    async fn register_watch(
        &self,
        _subscriber: &ResourceKey,
        registration: WatchRegistration,
    ) -> Result<WatchId, ResourceError> {
        self.watched
            .lock().await
            .push(registration.target);
        Ok(WatchId(self.watched.lock().await.len() as u64))
    }

    async fn cancel_watch(&self, _watch: WatchId) -> Result<(), ResourceError> {
        Ok(())
    }
}

/// The production requeue scheduler seam: a pass that requeues records the
/// delay and does nothing else, which is what lets the test assert that an
/// unprovable relationship keeps re-checking.
struct RecordingRequeue(Mutex<Vec<(ResourceKey, std::time::Duration)>>);

impl RequeueScheduler for RecordingRequeue {
    fn schedule(&self, key: ResourceKey, after: std::time::Duration) -> RequeueId {
        let mut scheduled = self.0.lock().expect("requeue lock");
        scheduled.push((key, after));
        RequeueId(scheduled.len() as u64)
    }

    fn cancel(&self, _id: RequeueId) {}
}

/// Build the production `ResourceContext` for one committed row.
fn context(
    row: StoredDesiredResource,
    decoder: Arc<dyn SpecDecoder>,
    manager: Arc<dyn ManagerEndpoint>,
) -> (ResourceContext, Arc<RecordingRequeue>) {
    let (effects_tx, _effects_rx) = tokio::sync::mpsc::unbounded_channel();
    let (notify_tx, _notify_rx) = tokio::sync::mpsc::unbounded_channel();
    let requeue = Arc::new(RecordingRequeue(Mutex::new(Vec::new())));
    (
        ResourceContext::new(
            row,
            decoder,
            manager,
            Arc::clone(&requeue) as Arc<dyn RequeueScheduler>,
            effects_tx,
            notify_tx,
        ),
        requeue,
    )
}

// ---------------------------------------------------------------------------
// The broker-backed dispatch facet
// ---------------------------------------------------------------------------

/// The declared dispatch facet, implemented over the broker's own resolution.
///
/// This is the production seam the daemon supplies: it carries the typed request
/// across the broker's own wire codec, so the value the accept path sees is the
/// value the wire encodes, and it records every request so a case can assert on
/// the bytes the driver actually built.
struct BrokerBackedDispatch {
    runtime_root: PathBuf,
    resolver: Arc<BundleResolver>,
    sent: Mutex<Vec<(EndpointAccessVerb, EndpointAccessRequest)>>,
    /// What the accept path refused, as the dispatch reported it upward. The
    /// fixture's consumer row has no host account on this host, so the accept
    /// path answers with a named refusal and this is what it said.
    refusals: Mutex<Vec<String>>,
}

impl BrokerBackedDispatch {
    fn new(runtime_root: PathBuf, resolver: BundleResolver) -> Arc<Self> {
        Arc::new(Self {
            runtime_root,
            resolver: Arc::new(resolver),
            sent: Mutex::new(Vec::new()),
            refusals: Mutex::new(Vec::new()),
        })
    }

    /// Record one request the dispatch encoded for the wire.
    ///
    /// Synchronous on purpose: the async dispatch calls this rather than
    /// taking the guard itself, so no `std::sync::Mutex` guard is ever held
    /// across an await.
    fn record_sent(&self, verb: EndpointAccessVerb, request: EndpointAccessRequest) {
        self.sent.lock().expect("sent lock").push((verb, request));
    }

    /// Record one refusal the accept path returned, spelled the way the
    /// dispatch reports it to the driver.
    fn record_refusal(&self, refusal: String) {
        self.refusals.lock().expect("refusals lock").push(refusal);
    }

    fn sent(&self) -> Vec<(EndpointAccessVerb, EndpointAccessRequest)> {
        self.sent.lock().expect("sent lock").clone()
    }

    /// Every refusal the broker's own accept path returned, in order.
    ///
    /// The case reads the refusal from the broker rather than from the
    /// status the driver published, so "the driver published what the broker
    /// said" is a statement about the production path and not an echo of a
    /// value this harness chose.
    fn refusals(&self) -> Vec<String> {
        self.refusals.lock().expect("refusals lock").clone()
    }
}

/// The wire variant one verb travels as, so the accept path is driven by the
/// same dispatch arm the broker's own runtime drives.
fn wire_variant(verb: EndpointAccessVerb, request: EndpointAccessRequest) -> BrokerRequest {
    match verb {
        EndpointAccessVerb::Observe => BrokerRequest::EndpointObserve(request),
        EndpointAccessVerb::Grant => BrokerRequest::EndpointGrantAccess(request),
        EndpointAccessVerb::Revoke => BrokerRequest::EndpointRevokeAccess(request),
    }
}

#[async_trait::async_trait]
impl EndpointAccessDispatch for BrokerBackedDispatch {
    async fn dispatch(
        &self,
        verb: EndpointAccessVerb,
        request: EndpointAccessRequest,
    ) -> Result<EndpointAccessResponse, EndpointAccessDispatchError> {
        self.record_sent(verb, request.clone());
        // Across the wire: encode the variant and decode it again, so the
        // accept path is proven against the value it receives.
        let frame = serde_json::to_vec(&wire_variant(verb, request))
            .expect("the exact-endpoint request encodes for the wire");
        let decoded: BrokerRequest = serde_json::from_slice(&frame)
            .expect("the exact-endpoint frame decodes back into a request");
        let answer =
            accept_endpoint_access(&decoded, &self.runtime_root, &self.resolver).map_err(|error| {
                let refusal = match error {
                    EndpointAccessError::ConsumerPrincipal { code } => {
                        format!("endpoint-access-consumer-principal/{code}")
                    }
                    other => other.code().to_owned(),
                };
                self.record_refusal(refusal.clone());
                EndpointAccessDispatchError::Refused(refusal)
            })?;
        Ok(answer)
    }
}

// ---------------------------------------------------------------------------
// The verified Zone bundle the broker derives the principal from
// ---------------------------------------------------------------------------

fn fixture_content_hash(resources: &[serde_json::Value]) -> String {
    let canonical = CanonicalJsonValue::parse(
        &serde_json::to_vec(&serde_json::Value::Array(resources.to_vec()))
            .expect("fixture resources serialize"),
    )
    .expect("fixture resources are canonical JSON");
    framed_canonical_digest(
        "d2b:v3:resource-bundle",
        &canonical_json_bytes(&canonical).expect("fixture resources encode"),
    )
}

fn zone_bundle() -> Vec<u8> {
    let provider = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "Provider",
        "metadata": { "name": "display-wayland", "zone": ZONE },
        "spec": { "artifactId": "display-wayland" },
    });
    let consumer = serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": "Process",
        "metadata": {
            "name": "frontend",
            "zone": ZONE,
            "ownerRef": "Provider/display-wayland",
        },
        "spec": {
            "domain": "system",
            "executionRef": "Host/work-host",
            "processClass": "controller",
            "providerRef": "Provider/system-minijail",
            "template": "consumer-worker",
        },
    });
    let mut resources = vec![consumer, provider];
    resources.sort_by_key(|value| {
        (
            value["type"].as_str().unwrap_or_default().to_owned(),
            value["metadata"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        )
    });
    let binding = serde_json::json!({
        "processRef": CONSUMER,
        "ownerRef": "Provider/display-wayland",
        "executionRef": "Host/work-host",
        "template": "consumer-worker",
        "artifactId": "display-wayland",
        "binaryRef": "d2b-wayland-proxy",
        "artifactDigest": format!("sha256:{}", "a".repeat(64)),
        "binaryPath": "/nix/store/display-wayland/bin/d2b-wayland-proxy",
    });
    serde_json::to_vec(&serde_json::json!({
        "schemaVersion": 3,
        "bundleVersion": 1,
        "zone": ZONE,
        "zoneUid": ZONE_UID,
        "contentHash": fixture_content_hash(&resources),
        "artifactCatalogDigest": format!("sha256:{}", "c".repeat(64)),
        "schemaFingerprints": {},
        "providerSchemaDigests": {},
        "resources": resources,
        "processTemplates": [binding],
        "generatedAt": "1970-01-01T00:00:00.000Z",
    }))
    .expect("fixture zone resource bundle serializes")
}

fn resolver() -> BundleResolver {
    BundleResolver::from_artifacts_with_zone_resource_bundles(
        Bundle {
            bundle_version: 11,
            schema_version: "v2".to_owned(),
            storage_path: None,
            realm_workloads_launcher_v2_path: None,
            generation: BundleGeneration {
                generator: "test".to_owned(),
                source_revision: None,
                generated_at: None,
            },
            bundle_hash: None,
            artifact_hashes: None,
        },
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host fixture parses"),
        ProcessesJson {
            schema_version: "v2".to_owned(),
            vms: Vec::new(),
        },
        ManifestV04::from_slice(
            include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .expect("manifest fixture parses"),
        BTreeMap::from_iter([(ZONE.to_owned(), zone_bundle())]),
    )
}

// ---------------------------------------------------------------------------
// The committed `Endpoint` row
// ---------------------------------------------------------------------------

/// A realized host endpoint: the binding-owned virtiofsd socket, consumed by
/// exactly one subject over exactly the `resolve` operation.
///
/// The shape is one this driver's own closed realization set admits, so the
/// derivation runs against a committed `Endpoint` row the plane really serves
/// rather than against a look-alike.
///
/// The publication intent is stated, not inferred: the consumer allowlist says
/// who MAY hold the endpoint, while the intent says who this endpoint
/// PUBLISHES a relationship to. A fixture that named only the former derives
/// no row at all.
fn endpoint_spec(subjects: Vec<ResourceRef>) -> EndpointSpec {
    let published = subjects.clone();
    EndpointSpec::new(
        ResourceRef::parse("Provider/display-wayland").expect("provider ref"),
        ResourceRef::parse(PRODUCER).expect("producer ref"),
        EndpointClass::Service,
        EndpointTransport::Unix,
        BoundedToken::parse(VIRTIOFSD_PURPOSE).expect("bounded purpose"),
        Some(BoundedText::parse("sha256:compositor").expect("bounded fingerprint")),
        EndpointLocality::HostLocal,
        EndpointVisibility::Owner,
        EndpointAttachmentPolicy::new(false, 0).expect("attachment policy"),
        EndpointConsumerPolicy::new(subjects, Vec::new(), vec![EndpointOperation::Resolve])
            .expect("consumer policy"),
        EndpointLifecyclePolicy::RecycleWithProducer,
    )
    .expect("endpoint spec")
    .publishing_to(published)
    .expect("the endpoint publishes a binding to its declared consumer")
}

/// The canonical spec-store envelope one committed row is stored as.
///
/// The envelope is the contract's own shape, built over a deterministic uid per
/// row so a case can read a row back by identity.
fn envelope(
    type_name: &str,
    name: &str,
    uid: u8,
    owner: Option<&str>,
    spec: serde_json::Value,
) -> Vec<u8> {
    let bytes = serde_json::to_vec(&serde_json::json!({
        "apiVersion": "resources.d2bus.org/v3",
        "type": type_name,
        "metadata": {
            "name": name,
            "zone": ZONE,
            "uid": format!("{uid:02x}{uid:02x}{uid:02x}{uid:02x}-0000-4000-8000-000000000000"),
            "generation": 1,
            "revision": 1,
            "ownerRef": owner,
            "finalizers": [],
            "deletionRequestedAt": null,
            "createdAt": "1970-01-01T00:00:00.000Z",
            "updatedAt": "1970-01-01T00:00:00.000Z",
            "managedBy": "controller",
        },
        "spec": spec,
    }))
    .expect("fixture envelope serializes");
    d2b_contracts_resource::v3::resource_schema::CanonicalJsonValue::parse(&bytes)
        .expect("fixture envelope is canonical JSON")
        .to_canonical_bytes()
}

fn stored(key: ResourceKey, uid: [u8; 16], owner_uid: Option<[u8; 16]>, spec: Vec<u8>) -> StoredDesiredResource {
    StoredDesiredResource {
        key,
        uid,
        generation: 1,
        owner_uid,
        provenance: ResourceProvenance::Resource,
        deleting: false,
        spec,
        metadata: Vec::new(),
        created_at: 0,
    }
}

/// The committed rows the whole relationship hangs off: the Zone self row whose
/// bundle declares the consumer, the consumer row, and the `Endpoint` row
/// itself.
fn committed_rows(spec: &EndpointSpec) -> Vec<StoredDesiredResource> {
    vec![
        stored(
            ResourceKey::new(ZONE, "Zone", ZONE),
            zone_uid_bytes(),
            None,
            envelope("Zone", ZONE, 0x11, None, serde_json::json!({ "display": "compositor" })),
        ),
        stored(
            ResourceKey::new(ZONE, "Process", "frontend"),
            [0x31; 16],
            Some([0x42; 16]),
            envelope(
                "Process",
                "frontend",
                0x31,
                Some("Process/compositor"),
                serde_json::json!({ "domain": "system" }),
            ),
        ),
        stored(
            ResourceKey::new(ZONE, "Endpoint", "compositor"),
            [0x42; 16],
            Some([0x30; 16]),
            serde_json::to_vec(spec).expect("endpoint spec bytes"),
        ),
    ]
}

/// The whole committed neighbourhood one serving pass reads: the Zone self
/// row, the consumer row, the owning `Endpoint` row, and the relationship.
fn graph(spec: &EndpointSpec, binding: &StoredDesiredResource) -> Vec<StoredDesiredResource> {
    let mut rows = committed_rows(spec);
    rows.push(binding.clone());
    rows
}

/// The manager the serving pass reads, carrying the owning `Endpoint` row's
/// OWN published readiness.
///
/// Delivery is granted over one exact realization, so the serving actor reads
/// the endpoint's published status and its published incarnation token rather
/// than re-deriving either. A manager that answered no view would leave the
/// relationship with nothing to prove it is delivered over, which is the
/// fail-closed answer and not the one these cases are about.
fn serving_manager(rows: Vec<StoredDesiredResource>) -> Arc<RecordingManager> {
    let endpoint = rows
        .iter()
        .find(|row| row.key.type_name == "Endpoint")
        .expect("the graph carries the owning Endpoint row");
    let view = endpoint_readiness_view(endpoint);
    RecordingManager::with_views(rows, vec![view])
}

/// The view an `Endpoint` actor publishes for one realized row: the observed
/// status, and the projection carrying the row's own opaque incarnation token.
fn endpoint_readiness_view(endpoint: &StoredDesiredResource) -> ResourceView {
    let incarnation = RealizationIncarnation::derive(
        ZONE,
        &ResourceRef::parse(ENDPOINT).expect("endpoint ref"),
        endpoint.generation,
        &hex(&[0x31; 16]),
        1,
        0,
        Some("sha256:compositor"),
    )
    .expect("the fixture realizes into a bounded token");
    let projection = serde_json::json!({
        "endpoint": {
            "readiness": "realized",
            "generation": endpoint.generation,
            "incarnation": incarnation.as_str(),
        }
    });
    ResourceView {
        key: endpoint.key.clone(),
        uid: endpoint.uid,
        generation: endpoint.generation,
        deleting: endpoint.deleting,
        provenance: endpoint.provenance,
        spec: endpoint.spec.clone(),
        metadata: endpoint.metadata.clone(),
        owner_key: None,
        status: Some(ResourceStatus::Ready),
        status_generation: Some(endpoint.generation),
        status_projection: Some(projection),
    }
}

/// The canonical lowercase hex of a resource uid.
fn hex(bytes: &[u8; 16]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The bytes of the Zone self row's committed uid.
///
/// Read back from the same string the fixture bundle declares, so the driver
/// resolves the Zone identity from a committed row rather than from a literal
/// the test chose separately.
fn zone_uid_bytes() -> [u8; 16] {
    let hex: String = ZONE_UID.chars().filter(|c| *c != '-').collect();
    let mut bytes = [0_u8; 16];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).expect("hex uid");
    }
    ResourceUid::from_bytes(&bytes).expect("fixture zone uid");
    bytes
}

// ---------------------------------------------------------------------------
// The host endpoint tree the broker resolves against
// ---------------------------------------------------------------------------

struct HostEndpoints {
    root: tempfile::TempDir,
    runtime_root: PathBuf,
    endpoints: PathBuf,
    _listeners: Vec<UnixListener>,
}

impl HostEndpoints {
    /// A temporary directory directly under `/tmp`, not under `TMPDIR`.
    ///
    /// The broker answers `ancestors_traversable` from a FULL-path read, so
    /// every directory from `/` down to the broker's own runtime root has to
    /// be traversable for a correct grant to be reported delivered - that is
    /// the production shape (`/run/d2b` under `/run` and `/`). Bazel seals
    /// `TMPDIR` inside a non-traversable directory, which would make the
    /// correct answer "not traversable" for reasons that have nothing to do
    /// with the grant under test.
    fn tempdir_in_tmp() -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix("d2b-endpoint-delivery-")
            .tempdir_in("/tmp")
            .expect("a temporary directory under /tmp for the endpoint tree")
    }

    /// The broker-owned tree, binding exactly `sockets` inside its own
    /// endpoint directory.
    fn with_sockets(sockets: &[&str]) -> Self {
        let root = Self::tempdir_in_tmp();
        let runtime_root = root.path().join("run");
        let endpoints = runtime_root.join("endpoints");
        fs::create_dir_all(&endpoints).expect("create the broker endpoint directory");
        // The broker-owned endpoint directory is 0700: a consumer that could
        // enumerate it would hold the directory authority the exact-endpoint
        // design withdrew, so the host tree under test is built in the posture
        // a correct grant has to survive rather than in one that would hand
        // listing back.
        for directory in [&runtime_root, &endpoints] {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
                .expect("restrict the broker endpoint tree");
        }
        let mut listeners = sockets
            .iter()
            .map(|name| {
                UnixListener::bind(endpoints.join(name))
                    .unwrap_or_else(|error| panic!("bind the endpoint {name}: {error}"))
            })
            .collect::<Vec<UnixListener>>();
        listeners.push(
            UnixListener::bind(root.path().join(ALTERNATE_ABSOLUTE))
                .expect("bind the alternate absolute socket"),
        );
        Self {
            root,
            runtime_root,
            endpoints,
            _listeners: listeners,
        }
    }

    fn new(admitted: &str) -> Self {
        Self::with_sockets(&[admitted, SIBLING])
    }

    /// The same tree with NOTHING bound inside the broker's own endpoint
    /// directory.
    ///
    /// This is the state a second cleanup pass over an already-revoked - or
    /// never granted - relationship finds, and it is the one the broker's own
    /// accept path answers with its own absent class rather than with an
    /// effect failure.
    fn without_socket() -> Self {
        Self::with_sockets(&[])
    }

    fn admitted(&self, name: &str) -> PathBuf {
        self.endpoints.join(name)
    }

    fn alternate(&self) -> PathBuf {
        self.root.path().join(ALTERNATE_ABSOLUTE)
    }
}

fn pinned_identity(path: &Path) -> EndpointSocketIdentity {
    let meta = fs::metadata(path).expect("pinned metadata");
    EndpointSocketIdentity::new(meta.dev(), meta.ino())
}

/// The `getfacl` binary, or the lane's honest limit.
fn acl_tool(name: &str) -> Option<PathBuf> {
    ["/run/current-system/sw/bin", "/usr/bin", "/bin"]
        .iter()
        .map(|dir| Path::new(dir).join(name))
        .find(|candidate| candidate.is_file())
}

/// Every named-user ACL entry the kernel holds at `path`, read through
/// `getfacl` rather than through the broker's own xattr parser.
///
/// The question this lane can still ask is whether ANY principal was written
/// an entry, which is the strongest form of "the request granted nothing
/// here": an entry for a uid nothing holds is exactly what the broker's
/// grant path used to be able to write, so reading the whole set rather than
/// one uid's row is what catches it. The per-uid effective rights a granted
/// relationship is held to are a host-lane proof and no longer reachable
/// here; the refusal that replaces them names the row it could not resolve.
fn named_user_entries(path: &Path) -> Vec<u32> {
    let tool = acl_tool("getfacl").unwrap_or_else(|| {
        panic!(
            "getfacl is required to read the ACL entries the kernel holds at {}",
            path.display()
        )
    });
    let output = std::process::Command::new(tool)
        .arg("--omit-header")
        .arg("--numeric")
        .arg("--absolute-names")
        .arg("--no-effective")
        .arg(path)
        .output()
        .unwrap_or_else(|error| panic!("getfacl on {}: {error}", path.display()));
    assert!(
        output.status.success(),
        "getfacl on {} failed: {}",
        path.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().strip_prefix("user:"))
        .filter_map(|rest| rest.split(':').next())
        .filter_map(|uid| uid.parse::<u32>().ok())
        .collect()
}

// ---------------------------------------------------------------------------
// The case
// ---------------------------------------------------------------------------

/// The whole relationship: one committed `Endpoint` row derives its
/// `EndpointBinding`, the row commits idempotently and is retired again when
/// its consumer is withdrawn - and the serving pass that would deliver it
/// answers with the broker's own named refusal, on a host that has provisioned
/// no account for the fixture's consumer row.
///
/// What this now proves is the end of the chain in both directions. The source
/// half is untouched by account provisioning and is asserted exactly as
/// before: the derivation, the canonical row bytes the real driver commits,
/// the strict decoder that reads them back, the idempotent repeat pass, and the
/// withdrawal that retires the row. The serving half reaches the broker's own
/// `accept_endpoint_access` over the wire codec and is answered with
/// `Undelivered { Refused("endpoint-access-consumer-principal/...") }`, because
/// the consumer's principal is resolved from a real host account and this host
/// holds none for that row. Nothing is written to any inode, on every pass,
/// and the refusal is reached before the broker resolves a socket path at all -
/// which is the negative of the older "lands on one inode and nowhere else"
/// proof rather than a weaker version of it.
///
/// The positive delivery half is a host-lane proof and is no longer reachable
/// here: the grant landing on the exact pinned inode, the kernel's effective
/// rights read back for the consumer's own ids, the sibling and alternate
/// sockets carrying nothing FOR THAT PRINCIPAL, the standing grant being
/// observed rather than re-applied, and a rebound socket reported as replaced.
/// Each needs a consumer whose account the host has provisioned.
#[tokio::test]
async fn a_committed_endpoint_derives_and_commits_its_exact_row_and_delivers_nothing_without_a_host_account() {
    let endpoint_ref = ResourceRef::parse(ENDPOINT).expect("endpoint ref");
    let consumer_ref = ResourceRef::parse(CONSUMER).expect("consumer ref");
    let zone = d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("zone id");
    let spec = endpoint_spec(vec![consumer_ref.clone()]);

    // The slot the source derives, and therefore the socket the broker will
    // resolve: one endpoint, one socket name.
    let slot = endpoint_delivery_slot(&zone, &endpoint_ref).expect("derived delivery slot");
    let host = HostEndpoints::new(slot.as_str());
    let socket_path = host.admitted(slot.as_str());
    // The inode the admitted socket holds before anything is asked about it,
    // read from the filesystem rather than from a value the broker echoed:
    // a producer that rebinds it below must land on a different one.
    let admitted_identity = pinned_identity(&socket_path);

    // 1. The source's own derivation.
    let deliveries =
        declared_endpoint_bindings(&zone, &spec, &endpoint_ref).expect("declared deliveries");
    assert_eq!(deliveries.len(), 1, "one declared consumer, one delivery");
    let derived = canonical_binding_row(&zone, &spec, &endpoint_ref, &deliveries[0])
        .expect("the source derives its committed row");

    // 2. The real driver's reconcile pass commits exactly those bytes.
    let manager = RecordingManager::with(committed_rows(&spec));
    {
        let (mut ctx, _requeue) = context(
            committed_rows(&spec)[2].clone(),
            endpoint_spec_decoder(),
            Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
        );
        let mut driver = EndpointDriverFactory::new(EndpointDriverArgs {
            zone: ZONE.to_owned(),
            facets: realized_facets(),
        })
        .create(&ResourceKey::new(ZONE, "Endpoint", "compositor"))
        .await;
        driver.reconcile(&mut ctx).await.expect("the source pass reconciles");
    }
    let ensured = manager.ensured().await;
    assert_eq!(
        ensured.len(),
        1,
        "one declared consumer commits exactly one relationship row"
    );
    assert_eq!(ensured[0].type_name.as_str(), ENDPOINT_BINDING_TYPE_NAME);
    assert_eq!(ensured[0].name, derived.name().as_str());
    assert_eq!(
        ensured[0].spec, derived.spec(),
        "the committed bytes are the derivation's own canonical row bytes"
    );

    // The committed row round-trips through the family's own strict decoder,
    // so what a boundary reads back is what the source minted.
    let decoded: EndpointBindingSpec =
        serde_json::from_slice(&ensured[0].spec).expect("canonical EndpointBindingSpec bytes");
    assert_eq!(decoded.endpoint_ref(), &endpoint_ref);
    assert_eq!(decoded.execution_ref(), &consumer_ref);
    assert_eq!(decoded.slot().as_str(), slot.as_str());

    // 3. A repeat pass over the unchanged row commits nothing new.
    {
        let (mut ctx, _requeue) = context(
            committed_rows(&spec)[2].clone(),
            endpoint_spec_decoder(),
            Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
        );
        let mut driver = EndpointDriverFactory::new(EndpointDriverArgs {
            zone: ZONE.to_owned(),
            facets: realized_facets(),
        })
        .create(&ResourceKey::new(ZONE, "Endpoint", "compositor"))
        .await;
        driver.reconcile(&mut ctx).await.expect("the repeat pass reconciles");
    }
    assert_eq!(
        manager.ensured().await.len(),
        2,
        "the second pass re-ensures the same row rather than adding one"
    );
    assert_eq!(
        manager.bindings().await.len(),
        1,
        "an unchanged source row still owns exactly one relationship"
    );

    // 4. The serving pass over the committed row, through the broker's own
    //    accept path. The consumer this fixture declares is a template-bound
    //    row, so the principal the ACL would name resolves only where the host
    //    has provisioned its account. This host provisions none, so the pass
    //    reaches the broker and the broker answers by name.
    let dispatch = BrokerBackedDispatch::new(host.runtime_root.clone(), resolver());
    let binding_row = manager.bindings().await.remove(0);
    let refused = || EndpointBindingDriverStatus::Undelivered {
        reason: EndpointDeliveryRefusal::Refused(REFUSED_CONSUMER_PRINCIPAL.to_owned()),
    };
    let status = deliver(graph(&spec, &binding_row), Arc::clone(&dispatch)).await;
    assert_eq!(
        status,
        refused(),
        "a consumer row with no host account is delivered to nobody"
    );
    // The refusal is the broker's own answer, read where it was produced and
    // not restated from the status the driver published over it.
    assert_eq!(
        dispatch.refusals(),
        vec![REFUSED_CONSUMER_PRINCIPAL.to_owned()],
        "the accept path refuses the consumer principal by name"
    );
    // The request really did reach the broker asking about this row's own
    // slot, so the refusal is about the principal and not about an endpoint
    // the broker's directory did not hold.
    let sent = dispatch.sent();
    assert_eq!(sent.len(), 1, "one serving pass asks the broker once");
    assert_eq!(sent[0].0, EndpointAccessVerb::Grant);
    assert_eq!(
        sent[0].1.socket.as_str(),
        slot.as_str(),
        "and it names the row's own committed slot"
    );

    // 5. Nothing was written anywhere in the tree. The consumer principal is
    //    resolved before the broker looks a socket up, so the sibling and the
    //    alternate absolute socket are not "left alone" by a narrow grant -
    //    they are never resolved at all, and no inode carries an entry for any
    //    principal, which is the strongest form of "nothing was granted here".
    let sibling = host.admitted(SIBLING);
    let alternate = host.alternate();
    for (label, path) in [
        ("the admitted endpoint", &socket_path),
        ("the sibling endpoint", &sibling),
        ("the alternate absolute socket", &alternate),
    ] {
        assert_eq!(
            named_user_entries(path),
            Vec::<u32>::new(),
            "{label} carries no named-user entry at all, because no principal was resolved to grant one"
        );
    }

    // The ordering that makes that unanswerable as "one inode and nowhere
    // else" rather than merely "nothing anywhere": the principal is resolved
    // before the broker looks a socket up, so a request naming a DIFFERENT
    // socket in the broker's own directory is refused at the principal too,
    // and not as an endpoint that directory does not hold. The sibling is a
    // live socket there, so the two are indistinguishable from outside.
    let sibling_socket = BoundedToken::parse(SIBLING).expect("bounded sibling name");
    let zone_uid = ResourceUid::from_bytes(&zone_uid_bytes()).expect("zone uid");
    let repointed = EndpointAccessRequest {
        endpoint_ref: endpoint_ref.clone(),
        consumer_ref: consumer_ref.clone(),
        authority_key: endpoint_access_authority_binding(
            &endpoint_ref,
            &consumer_ref,
            &zone_uid,
            &sibling_socket,
            EndpointAccessVerb::Grant,
        ),
        socket: sibling_socket,
        socket_rights: 0o6,
        claimed_principal: None,
        tracing_span_id: None,
        zone_uid,
    };
    match accept_endpoint_access(
        &wire_variant(EndpointAccessVerb::Grant, repointed),
        &host.runtime_root,
        &resolver(),
    )
    .expect_err("a request naming another socket is refused the same way, so no socket is ever resolved")
    {
        EndpointAccessError::ConsumerPrincipal { code } => assert_eq!(
            format!("endpoint-access-consumer-principal/{code}"),
            REFUSED_CONSUMER_PRINCIPAL,
            "the same named refusal the serving pass published, so it is the principal and not the socket lookup"
        ),
        other => panic!("the principal is resolved before any path is, not {other:?}"),
    }

    // 6. The next pass over the SAME actor refuses the same way. A standing
    //    grant would be observed first; there is none, so the pass re-derives
    //    and the broker answers identically - a retry is not a second, weaker
    //    answer, and it still writes nothing.
    let statuses = deliver_passes(
        graph(&spec, &binding_row),
        Arc::clone(&dispatch),
        2,
        None,
    )
    .await;
    assert_eq!(
        statuses,
        vec![refused(), refused()],
        "both passes publish the refusal, and the first of them published it rather than observing a grant"
    );
    assert_eq!(
        named_user_entries(&socket_path),
        Vec::<u32>::new(),
        "and the retry still wrote no entry on the exact endpoint"
    );

    // 7. A producer that replaces its socket between two passes of the same
    //    actor is answered by name, never as `EndpointReplaced`: the fence
    //    that reports a replaced inode compares the new pin against the row's
    //    OWN in-memory status, and a relationship that was never delivered
    //    has no standing inode to have been replaced. The replacement happens
    //    between the passes of one actor for that reason, exactly as before.
    drop(manager);
    // The rebound listener has to outlive both passes - keeping it bound IS
    // what makes the second pass see a different inode - so the hook shares
    // it rather than handing it back.
    let rebound: Arc<tokio::sync::Mutex<Option<UnixListener>>> =
        Arc::new(tokio::sync::Mutex::new(None));
    let hook_socket = Arc::new(socket_path.clone());
    let hook_rebound = Arc::clone(&rebound);
    let statuses = deliver_passes(
        graph(&spec, &binding_row),
        Arc::clone(&dispatch),
        2,
        Some(&mut move |pass: usize| {
            let socket_path = Arc::clone(&hook_socket);
            let rebound = Arc::clone(&hook_rebound);
            Box::pin(async move {
                if pass == 1 {
                    tokio::fs::remove_file(socket_path.as_ref())
                        .await
                        .expect("unlink the replaced endpoint");
                    *rebound.lock().await = Some(
                        UnixListener::bind(socket_path.as_ref()).expect("rebind the endpoint"),
                    );
                }
            })
        }),
    )
    .await;
    let rebound = rebound
        .lock()
        .await
        .take()
        .expect("the producer rebound its socket between the passes");
    assert_ne!(
        pinned_identity(&socket_path),
        admitted_identity,
        "the rebound socket is a different inode"
    );
    assert_eq!(
        statuses[1],
        refused(),
        "a producer that replaced its socket is refused, not reported as a replacement: \
         nothing was ever delivered, so there is no standing inode to have been replaced"
    );
    assert_eq!(
        named_user_entries(&socket_path),
        Vec::<u32>::new(),
        "and the rebound inode is granted nothing either"
    );
    drop(rebound);

    // 8. A withdrawn consumer takes the relationship with it: the source no
    //    longer derives the row, so the manager retires it.
    let withdrawn = endpoint_spec(Vec::new());
    let manager = RecordingManager::with({
        let mut rows = committed_rows(&withdrawn);
        rows.push(binding_row);
        rows
    });
    let withdrawn_row = manager.bindings().await.remove(0);
    assert_eq!(
        withdrawn_row.key.name,
        derived.name().as_str(),
        "the retired row is the one the source derived"
    );
    {
        let (mut ctx, _requeue) = context(
            committed_rows(&withdrawn)[2].clone(),
            endpoint_spec_decoder(),
            Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
        );
        let mut driver = EndpointDriverFactory::new(EndpointDriverArgs {
            zone: ZONE.to_owned(),
            facets: realized_facets(),
        })
        .create(&ResourceKey::new(ZONE, "Endpoint", "compositor"))
        .await;
        driver.reconcile(&mut ctx).await.expect("the narrowed pass reconciles");
    }
    assert_eq!(manager.ensured().await.len(), 0, "a withdrawn consumer derives no row");
    assert_eq!(
        manager.deleted().await,
        vec![ResourceKey::new(
            ZONE,
            ENDPOINT_BINDING_TYPE_NAME,
            derived.name().as_str()
        )],
        "the relationship the source no longer derives is retired"
    );
}

/// Drive the real serving driver over one committed `EndpointBinding` row and
/// read back what it published.
///
/// `graph` is the whole committed neighbourhood: the Zone self row whose bundle
/// declares the consumer, the consumer row, the owning `Endpoint` row, and the
/// relationship row itself. The driver reads all four through the same manager
/// seam the source pass wrote through.
async fn deliver(
    graph: Vec<StoredDesiredResource>,
    dispatch: Arc<BrokerBackedDispatch>,
) -> EndpointBindingDriverStatus {
    deliver_passes(graph, dispatch, 1, None).await[0].clone()
}

/// Drive `passes` consecutive reconcile passes over ONE actor and read back
/// what each published.
///
/// The passes share one driver and one context, which is how the plane runs a
/// row: the fence a standing grant is compared against lives in the row's own
/// in-memory status slot, so a second pass is only meaningful over the
/// same actor.
/// A hook run between two serving passes, awaited so it can do filesystem
/// work (a producer replacing its own socket) off the runtime workers.
type BetweenPasses<'a> =
    dyn FnMut(usize) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> + Send + 'a;

async fn deliver_passes(
    graph: Vec<StoredDesiredResource>,
    dispatch: Arc<BrokerBackedDispatch>,
    passes: usize,
    mut between_passes: Option<&mut BetweenPasses<'_>>,
) -> Vec<EndpointBindingDriverStatus> {
    let row = graph
        .iter()
        .find(|row| row.key.type_name == ENDPOINT_BINDING_TYPE_NAME)
        .expect("the graph carries the committed relationship")
        .clone();
    let manager = serving_manager(graph);
    let (mut ctx, _requeue) = context(
        row.clone(),
        endpoint_binding_spec_decoder(),
        manager as Arc<dyn ManagerEndpoint>,
    );
    let mut driver = EndpointBindingDriverFactory::new(EndpointBindingDriverArgs {
        zone: d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("zone id"),
        access: dispatch,
    })
    .create(&ResourceKey::new(ZONE, ENDPOINT_BINDING_TYPE_NAME, &row.key.name))
    .await;
    let mut statuses = Vec::with_capacity(passes);
    for pass in 0..passes {
        if pass > 0
            && let Some(hook) = between_passes.as_deref_mut()
        {
            hook(pass).await;
        }
        driver
            .reconcile(&mut ctx)
            .await
            .expect("the serving pass reconciles");
        statuses.push(
            ctx.status::<EndpointBindingDriverStatus>()
                .cloned()
                .expect("the serving pass publishes a status"),
        );
    }
    statuses
}

/// The request the serving pass built names the exact endpoint, the exact
/// consumer, the Zone whose bundle declares them, and a socket that is the
/// row's own committed slot - and the key over those five facts is the one the
/// broker recomputes.
#[tokio::test]
async fn the_request_the_driver_builds_is_bound_to_its_own_committed_facts() {
    let endpoint_ref = ResourceRef::parse(ENDPOINT).expect("endpoint ref");
    let consumer_ref = ResourceRef::parse(CONSUMER).expect("consumer ref");
    let zone = d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("zone id");
    let spec = endpoint_spec(vec![consumer_ref.clone()]);
    let slot = endpoint_delivery_slot(&zone, &endpoint_ref).expect("derived delivery slot");
    let host = HostEndpoints::new(slot.as_str());

    let manager = RecordingManager::with(committed_rows(&spec));
    let (mut ctx, _requeue) = context(
        committed_rows(&spec)[2].clone(),
        endpoint_spec_decoder(),
        Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
    );
    EndpointDriverFactory::new(EndpointDriverArgs {
        zone: ZONE.to_owned(),
        facets: realized_facets(),
    })
    .create(&ResourceKey::new(ZONE, "Endpoint", "compositor"))
    .await
    .reconcile(&mut ctx)
    .await
    .expect("the source pass reconciles");
    let binding_row = manager.bindings().await.remove(0);

    let dispatch = BrokerBackedDispatch::new(host.runtime_root.clone(), resolver());
    deliver(graph(&spec, &binding_row), Arc::clone(&dispatch)).await;

    let sent = dispatch.sent();
    let grant = sent
        .iter()
        .find(|(verb, _)| *verb == EndpointAccessVerb::Grant)
        .expect("the serving pass applied a grant");
    let request = &grant.1;
    assert_eq!(request.endpoint_ref, endpoint_ref);
    assert_eq!(request.consumer_ref, consumer_ref);
    assert_eq!(request.socket.as_str(), slot.as_str(), "no path, only the row's slot");
    assert_eq!(request.socket_rights, 0o6);
    assert_eq!(request.zone_uid.to_canonical_string(), ZONE_UID);
    assert_eq!(
        request.authority_key,
        endpoint_access_authority_binding(
            &endpoint_ref,
            &consumer_ref,
            &ResourceUid::from_bytes(&zone_uid_bytes()).expect("zone uid"),
            &BoundedToken::parse(slot.as_str()).expect("bounded slot"),
            EndpointAccessVerb::Grant,
        ),
        "the key is the wire's own binding over the five facts the request carries"
    );

    // A key that does not reproduce the request's own committed facts is
    // refused before any path is resolved: repointing the same grant at a
    // sibling socket reproduces no key.
    let mut repointed = request.clone();
    repointed.socket = BoundedToken::parse(SIBLING).expect("bounded sibling name");
    let error = accept_endpoint_access(
        &wire_variant(EndpointAccessVerb::Grant, repointed),
        &host.runtime_root,
        &resolver(),
    )
    .expect_err("a grant whose key does not reproduce itself is refused");
    assert_eq!(
        error.code(),
        "endpoint-access-authority-mismatch",
        "the binding is checked before the path is resolved"
    );
}

/// The `EndpointBinding` type's declaration is a real serving driver: it names
/// the type, it reads the owning `Endpoint` and the `Zone` self row, and it
/// registers the strict decoder its own rows are stored under.
#[test]
fn the_binding_type_is_registered_with_a_real_driver() {
    let dispatch = BrokerBackedDispatch::new(PathBuf::from("/run/d2b"), resolver());
    let descriptor = endpoint_binding_descriptor(EndpointBindingDriverArgs {
        zone: d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("zone id"),
        access: dispatch,
    });
    assert_eq!(descriptor.resource_type, WellKnownType::ENDPOINT_BINDING);
    assert!(
        !descriptor.exportable,
        "a relationship is never an export subject"
    );
    assert_eq!(
        descriptor.reads,
        &[WellKnownType::ENDPOINT, WellKnownType::ZONE],
    );
}

// ---------------------------------------------------------------------------
// The Provider-committed shape lane (U5, KTD5, R14)
// ---------------------------------------------------------------------------

/// The vocabulary a declaring Provider publishes and the composition root
/// installs: the shapes that Provider committed, matched in full.
///
/// This is the whole shape of any Provider's own implementation - a set of
/// committed shapes and one exact comparison - which is why the driver admits
/// a row on the Provider's verdict and on nothing else, and never names the
/// Provider that produced it (KTD5).
struct InstalledVocabulary {
    committed: Vec<(EndpointSpec, CommittedEndpointShape)>,
}

impl CommittedEndpointShapeSource for InstalledVocabulary {
    fn committed_endpoint_shape(&self, spec: &EndpointSpec) -> Option<CommittedEndpointShape> {
        self.committed
            .iter()
            .find(|(committed, _)| committed == spec)
            .map(|(_, shape)| *shape)
    }
}

/// One Provider-committed shape: the private cross-domain data carriage a
/// worker the declaring Provider launched realizes, consumed by exactly one
/// subject over the attach and resolve operations.
///
/// The purpose is outside every family this crate derives for itself, so the
/// only thing that can classify it is the Provider that committed it.
fn provider_committed_spec(producer: &ResourceRef, subjects: Vec<ResourceRef>) -> EndpointSpec {
    let published = subjects.clone();
    EndpointSpec::new(
        ResourceRef::parse("Provider/display-wayland").expect("provider ref"),
        producer.clone(),
        EndpointClass::Data,
        EndpointTransport::FdAttachment,
        BoundedToken::parse("wayland-cross-domain").expect("bounded purpose"),
        Some(BoundedText::parse("display-wayland-data-v3-r3").expect("bounded fingerprint")),
        EndpointLocality::CrossDomain,
        EndpointVisibility::Owner,
        EndpointAttachmentPolicy::new(true, 1).expect("attachment policy"),
        EndpointConsumerPolicy::new(
            subjects,
            Vec::new(),
            vec![EndpointOperation::Attach, EndpointOperation::Resolve],
        )
        .expect("consumer policy"),
        EndpointLifecyclePolicy::RecycleWithProducer,
    )
    .expect("endpoint spec")
    .publishing_to(published)
    .expect("the endpoint publishes a binding to its declared consumer")
}

/// One committed shape with a single field edited, built through the
/// contract's own wire form so the result still decodes as a valid Endpoint
/// spec: a look-alike, not a malformed row.
fn mutated_spec(spec: &EndpointSpec, field: &str, value: serde_json::Value) -> EndpointSpec {
    let mut wire = serde_json::to_value(spec).expect("the committed spec encodes");
    wire.as_object_mut()
        .expect("the committed spec is an object")
        .insert(field.to_owned(), value);
    serde_json::from_value(wire).expect("the edited spec is still a valid Endpoint spec")
}

/// The view of the worker row the Provider launched, reporting `Ready` at its
/// own current generation: this row IS the shape's realization.
fn producer_ready_view() -> ResourceView {
    ResourceView {
        key: ResourceKey::new(ZONE, "Process", "proxy"),
        uid: [0x51; 16],
        generation: 1,
        deleting: false,
        provenance: ResourceProvenance::Resource,
        spec: envelope(
            "Process",
            "proxy",
            0x51,
            Some(ENDPOINT),
            serde_json::json!({ "domain": "system" }),
        ),
        metadata: Vec::new(),
        owner_key: None,
        status: Some(ResourceStatus::Ready),
        status_generation: Some(1),
        status_projection: None,
    }
}

/// The committed neighbourhood of a Provider-committed row: the Zone self row,
/// the consumer row, the worker row the shape is realized behind, and the
/// `Endpoint` row itself (index 3).
fn provider_committed_rows(spec: &EndpointSpec) -> Vec<StoredDesiredResource> {
    vec![
        stored(
            ResourceKey::new(ZONE, "Zone", ZONE),
            zone_uid_bytes(),
            None,
            envelope(
                "Zone",
                ZONE,
                0x11,
                None,
                serde_json::json!({ "display": "compositor" }),
            ),
        ),
        stored(
            ResourceKey::new(ZONE, "Process", "frontend"),
            [0x31; 16],
            Some([0x42; 16]),
            envelope(
                "Process",
                "frontend",
                0x31,
                Some(ENDPOINT),
                serde_json::json!({ "domain": "system" }),
            ),
        ),
        stored(
            ResourceKey::new(ZONE, "Process", "proxy"),
            [0x51; 16],
            Some([0x30; 16]),
            envelope(
                "Process",
                "proxy",
                0x51,
                Some(ENDPOINT),
                serde_json::json!({ "domain": "system" }),
            ),
        ),
        stored(
            ResourceKey::new(ZONE, "Endpoint", "compositor"),
            [0x42; 16],
            Some([0x30; 16]),
            serde_json::to_vec(spec).expect("endpoint spec bytes"),
        ),
    ]
}

/// Drive one reconcile pass over `rows[3]` with `facets`, the production
/// construction throughout: the driver factory builds this crate's own
/// effects service from the composition's facet set.
async fn reconcile_committed(
    rows: Vec<StoredDesiredResource>,
    facets: d2b_provider_endpoint::EndpointEffectFacets,
) -> Result<ResourceContext, d2b_resource_runtime::error::DriverFailure> {
    let manager = RecordingManager::with_views(rows.clone(), vec![producer_ready_view()]);
    let (mut ctx, _requeue) = context(
        rows[3].clone(),
        endpoint_spec_decoder(),
        Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
    );
    let mut driver = EndpointDriverFactory::new(EndpointDriverArgs {
        zone: ZONE.to_owned(),
        facets,
    })
    .create(&ResourceKey::new(ZONE, "Endpoint", "compositor"))
    .await;
    driver.reconcile(&mut ctx).await?;
    Ok(ctx)
}

/// The production composition admits a Provider-committed shape by the
/// Provider's own exact match, and refuses a look-alike terminally.
///
/// `EndpointDriverFactory` builds this crate's own effects service from the
/// facet set, so the only stand-in is the vocabulary a declaring Provider
/// publishes - the object a composition root installs (KTD5). Nothing here
/// writes a status from outside the actor: the `ManagerEndpoint` the driver
/// holds declares no status verb at all, and what the pass published is the
/// projection it set on its own context.
#[tokio::test]
async fn a_provider_committed_row_is_admitted_by_the_installed_vocabulary_and_its_look_alike_is_refused()
 {
    let consumer_ref = ResourceRef::parse(CONSUMER).expect("consumer ref");
    let producer_ref = ResourceRef::parse("Process/proxy").expect("producer ref");
    let spec = provider_committed_spec(&producer_ref, vec![consumer_ref.clone()]);
    let shape = CommittedEndpointShape::new(EndpointRealization::WorkerDataAttachment, 3);

    // The closed composition: with no Provider vocabulary installed, the very
    // same committed row is not a shape anything realizes.
    let closed = reconcile_committed(provider_committed_rows(&spec), realized_facets())
        .await
        .err()
        .expect("a composition that injected no vocabulary admits no Provider shape");
    assert_eq!(closed.kind().code(), "endpoint-shape-unsupported");

    // The composition that installed the Provider's own vocabulary.
    let vocabulary = Arc::new(InstalledVocabulary {
        committed: vec![(spec.clone(), shape)],
    });
    let rows = provider_committed_rows(&spec);
    let manager = RecordingManager::with_views(rows.clone(), vec![producer_ready_view()]);
    let (mut ctx, _requeue) = context(
        rows[3].clone(),
        endpoint_spec_decoder(),
        Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
    );
    let mut driver = EndpointDriverFactory::new(EndpointDriverArgs {
        zone: ZONE.to_owned(),
        facets: realized_facets().with_committed_shapes(vocabulary),
    })
    .create(&ResourceKey::new(ZONE, "Endpoint", "compositor"))
    .await;
    driver
        .reconcile(&mut ctx)
        .await
        .expect("the committed shape reaches a satisfied pass");

    assert_eq!(
        ctx.status::<EndpointDriverStatus>(),
        Some(&EndpointDriverStatus::Realized),
        "the actor publishes its own realized status"
    );
    let layer = ctx
        .take_status_projection()
        .expect("the actor publishes its own status layer");
    assert_eq!(
        layer["endpoint"]["readiness"],
        serde_json::json!("realized")
    );
    assert_eq!(
        layer["endpoint"]["observedProducerGeneration"],
        serde_json::json!(1),
        "the readiness is proved against the producer row's own current generation"
    );
    assert_eq!(
        layer["endpoint"]["connectionAvailability"],
        serde_json::Value::Null,
        "a worker shape publishes no host connectability state"
    );

    // The whole set of mutations this pass issued.
    let ensured = manager.ensured().await;
    assert_eq!(
        ensured.len(),
        1,
        "the pass committed the one relationship row it derives and nothing else"
    );
    assert_eq!(ensured[0].type_name.as_str(), ENDPOINT_BINDING_TYPE_NAME);
    assert!(
        manager.deleted().await.is_empty(),
        "a first pass retires nothing"
    );

    // One structural field changed is not a shape this Provider commits: the
    // driver refuses it terminally rather than repairing the near miss into
    // an admission.
    let look_alike = mutated_spec(&spec, "endpointClass", serde_json::json!("service"));
    assert_ne!(&look_alike, &spec, "the look-alike fixture is edited");
    let rejected = reconcile_committed(
        provider_committed_rows(&look_alike),
        realized_facets().with_committed_shapes(Arc::new(InstalledVocabulary {
            committed: vec![(spec.clone(), shape)],
        })),
    )
    .await
    .err()
    .expect("a look-alike on one committed axis is refused");
    assert_eq!(
        rejected.kind().code(),
        "endpoint-shape-unsupported",
        "the refusal names the shape, not an effect"
    );
    assert_eq!(
        rejected.class(),
        d2b_resource_runtime::error::FailureClass::Terminal,
        "and it is terminal: a retry cannot turn the near miss into an admission"
    );
}

// ---------------------------------------------------------------------------
// Cleanup: the revoke proof a retirement needs
// ---------------------------------------------------------------------------

/// The wire's own closed refusal class for "no exact endpoint is standing at
/// the resolved path".
///
/// Read out of the broker's own enum rather than restated as a literal, so
/// "the cleanup converged on a no-grant proof" stays a statement about the
/// class the broker itself produces.
fn endpoint_absent_code() -> &'static str {
    EndpointAccessError::EndpointAbsent.code()
}

/// One dispatch answer, scripted.
#[derive(Clone)]
enum ScriptedAnswer {
    /// The broker answered the verb.
    Answered,
    /// The broker refused, under this wire class.
    Refused(String),
    /// The privileged leg never answered at all.
    Unanswered,
}

/// The dispatch a cleanup pass drives.
///
/// The broker-backed facet beside it proves a request reaches the real
/// resolution; this one states the ANSWER instead, because what cleanup turns
/// on is the answer and not the path that produced it. An answered revoke, the
/// wire's absent class, some other refusal, and a dispatch that never answered
/// are four different proofs, and only the first two may retire a row.
struct ScriptedDispatch {
    answer: tokio::sync::Mutex<ScriptedAnswer>,
    /// The inode this dispatch pins on the next answer. A case moves it
    /// between passes so a producer that replaced its socket reads as a
    /// replacement rather than as the same delivery twice.
    pinned: tokio::sync::Mutex<u64>,
    sent: tokio::sync::Mutex<Vec<EndpointAccessVerb>>,
}

impl ScriptedDispatch {
    fn new(answer: ScriptedAnswer) -> Arc<Self> {
        Arc::new(Self {
            answer: tokio::sync::Mutex::new(answer),
            pinned: tokio::sync::Mutex::new(0x5150),
            sent: tokio::sync::Mutex::new(Vec::new()),
        })
    }

    /// The broker answered, which is what a removal that took effect looks
    /// like: it reports the inode the removal landed on.
    fn answered() -> Arc<Self> {
        Self::new(ScriptedAnswer::Answered)
    }

    fn refused(code: &str) -> Arc<Self> {
        Self::new(ScriptedAnswer::Refused(code.to_owned()))
    }

    fn unavailable() -> Arc<Self> {
        Self::new(ScriptedAnswer::Unanswered)
    }

    /// Move the inode this dispatch pins, the way a producer that replaced
    /// its socket moves it.
    async fn pin(&self, inode: u64) {
        *self.pinned.lock().await = inode;
    }

    /// Every verb this dispatch was asked for, in order.
    async fn sent(&self) -> Vec<EndpointAccessVerb> {
        self.sent.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl EndpointAccessDispatch for ScriptedDispatch {
    async fn dispatch(
        &self,
        verb: EndpointAccessVerb,
        request: EndpointAccessRequest,
    ) -> Result<EndpointAccessResponse, EndpointAccessDispatchError> {
        self.sent.lock().await.push(verb);
        let answer = self.answer.lock().await.clone();
        match answer {
            ScriptedAnswer::Answered => Ok(EndpointAccessResponse {
                endpoint_ref: request.endpoint_ref.clone(),
                consumer_ref: request.consumer_ref.clone(),
                socket: request.socket.clone(),
                socket_device: 0xfd00,
                socket_inode: *self.pinned.lock().await,
                socket_effective_rights: 0o6,
                ancestors_traversable: true,
                parent_listable: false,
                consumer_uid: 0,
                consumer_gid: 0,
            }),
            ScriptedAnswer::Refused(code) => Err(EndpointAccessDispatchError::Refused(code)),
            ScriptedAnswer::Unanswered => Err(EndpointAccessDispatchError::Unavailable(
                "the privileged leg did not answer".to_owned(),
            )),
        }
    }
}

/// The committed relationship the cleanup cases are driven against: the
/// canonical row the source itself derives, over the whole neighbourhood it
/// reads.
fn committed_relationship() -> (EndpointSpec, StoredDesiredResource) {
    let zone = d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("zone id");
    let endpoint_ref = ResourceRef::parse(ENDPOINT).expect("endpoint ref");
    let spec = endpoint_spec(vec![ResourceRef::parse(CONSUMER).expect("consumer ref")]);
    let deliveries =
        declared_endpoint_bindings(&zone, &spec, &endpoint_ref).expect("declared deliveries");
    let derived = canonical_binding_row(&zone, &spec, &endpoint_ref, &deliveries[0])
        .expect("the source derives its committed row");
    let binding = stored(
        ResourceKey::new(ZONE, ENDPOINT_BINDING_TYPE_NAME, derived.name().as_str()),
        [0x61; 16],
        Some([0x42; 16]),
        derived.spec().to_vec(),
    );
    (spec, binding)
}

/// Drive one real `EndpointBindingDriver` cleanup pass over `graph` and read
/// back what it reported.
///
/// A fresh driver is built for every call, which is what a restart looks like
/// from the row's side: nothing in memory survives, so every pass has to
/// re-derive the relationship and re-earn its proof. The neighbourhood is
/// exactly what the case hands over - a missing owning `Endpoint` row and a
/// missing consumer row are states the graph really can be in, so the view
/// this builds is empty exactly when the row it reads is absent.
async fn cleanup_pass(
    graph: Vec<StoredDesiredResource>,
    dispatch: Arc<dyn EndpointAccessDispatch>,
) -> Result<(), d2b_resource_runtime::error::DriverFailure> {
    let row = graph
        .iter()
        .find(|row| row.key.type_name == ENDPOINT_BINDING_TYPE_NAME)
        .expect("the graph carries the committed relationship")
        .clone();
    let views = graph
        .iter()
        .find(|row| row.key.type_name == "Endpoint")
        .map(endpoint_readiness_view)
        .into_iter()
        .collect();
    let manager = RecordingManager::with_views(graph, views);
    let (mut ctx, _requeue) = context(
        row.clone(),
        endpoint_binding_spec_decoder(),
        Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
    );
    let mut driver = EndpointBindingDriverFactory::new(EndpointBindingDriverArgs {
        zone: d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("zone id"),
        access: dispatch,
    })
    .create(&ResourceKey::new(ZONE, ENDPOINT_BINDING_TYPE_NAME, &row.key.name))
    .await;
    driver.delete(&mut ctx).await
}

/// Cleanup never reports success on ambiguity (AE15, R22, R36).
///
/// Six states, one real driver, one committed relationship. In each of them
/// the row's OWN committed bytes are the only thing that names the entry a
/// revoke would remove, and in each of them the answer the broker gives is
/// what decides whether the row may retire:
///
/// - delivered, then the source narrowed its own policy;
/// - the owning `Endpoint` row is gone;
/// - the consumer row is gone;
/// - the row's own bytes are malformed;
/// - a restart, which leaves the driver with no memory of what it granted;
/// - a broker that never answers.
///
/// The first three and the last are the ambiguity this barrier exists for:
/// each of them leaves an entry that MAY be standing, and each of them must
/// retain ownership - never report a release it cannot prove. They also must
/// still reach the broker: a source that stopped admitting the relationship
/// and a parent that stopped existing are states of the world AROUND it, not
/// evidence that nothing was ever installed.
///
/// The malformed row is refused before any verb is built, because bytes that
/// are not this relationship cannot name an entry at all.
///
/// The two answers that MAY retire a row are an answered revoke and the
/// wire's own absent class - a broker that positively reports no entry for
/// this consumer over this socket. The same pass converges on each.
#[tokio::test]
async fn cleanup_retains_ownership_until_the_broker_proves_the_release() {
    let (spec, binding) = committed_relationship();
    let endpoint_key = ResourceKey::new(ZONE, "Endpoint", "compositor");
    let consumer_key = ResourceKey::new(ZONE, "Process", "frontend");
    let committed = graph(&spec, &binding);

    // 1. Delivered, then the source narrowed its own consumer policy: the
    //    parent no longer admits this consumer, and the entry it admitted
    //    once is still installed.
    let narrowed = {
        let mut rows = committed.clone();
        let parent = rows
            .iter_mut()
            .find(|row| row.key == endpoint_key)
            .expect("the graph carries the owning endpoint");
        parent.spec = serde_json::to_vec(&endpoint_spec(vec![
            ResourceRef::parse(OTHER_CONSUMER).expect("the surviving consumer"),
        ]))
        .expect("narrowed spec bytes");
        rows
    };
    let dispatch = ScriptedDispatch::unavailable();
    assert!(
        cleanup_pass(narrowed, Arc::clone(&dispatch) as Arc<dyn EndpointAccessDispatch>)
            .await
            .is_err(),
        "a narrowed policy retains the relationship: the entry it once admitted may be standing"
    );
    assert_eq!(
        dispatch.sent().await,
        vec![EndpointAccessVerb::Revoke],
        "and the revoke still reached the broker rather than being skipped"
    );

    // 2. The owning `Endpoint` row is gone.
    let orphan = committed
        .iter()
        .filter(|row| row.key != endpoint_key)
        .cloned()
        .collect::<Vec<_>>();
    let dispatch = ScriptedDispatch::unavailable();
    assert!(
        cleanup_pass(orphan, Arc::clone(&dispatch) as Arc<dyn EndpointAccessDispatch>)
            .await
            .is_err(),
        "a missing parent retains the relationship: an absent source is not an absent grant"
    );
    assert_eq!(
        dispatch.sent().await,
        vec![EndpointAccessVerb::Revoke],
        "the revoke is derived from the row, not from the parent that is gone"
    );

    // 3. The consumer row is gone, so the broker can no longer derive the
    //    principal a revoke names.
    let orphaned = committed
        .iter()
        .filter(|row| row.key != consumer_key)
        .cloned()
        .collect::<Vec<_>>();
    assert!(
        cleanup_pass(orphaned, ScriptedDispatch::unavailable() as Arc<dyn EndpointAccessDispatch>)
            .await
            .is_err(),
        "a missing consumer retains the relationship"
    );

    // 4. The row's own bytes are malformed: no relationship can be named, so
    //    no revoke can be built and nothing is asked of the broker.
    let mut malformed_row = binding.clone();
    malformed_row.spec = b"{\"endpointRef\":\"Endpoint/compositor\"}".to_vec();
    let mut malformed = committed.clone();
    malformed[3] = malformed_row;
    let dispatch = ScriptedDispatch::answered();
    assert!(
        cleanup_pass(malformed, Arc::clone(&dispatch) as Arc<dyn EndpointAccessDispatch>)
            .await
            .is_err(),
        "a malformed row retains the relationship even from an answering broker"
    );
    assert!(
        dispatch.sent().await.is_empty(),
        "and it asks nothing: bytes that are not this relationship name no entry"
    );

    // 5. A restart, and a broker that never answers: the fresh driver holds no
    //    record of what it granted, so nothing is standing by its account.
    let dispatch = ScriptedDispatch::unavailable();
    assert!(
        cleanup_pass(committed.clone(), Arc::clone(&dispatch) as Arc<dyn EndpointAccessDispatch>)
            .await
            .is_err(),
        "a restart over an unprovable revoke retains the relationship"
    );

    // 6. The two proofs. The same pass converges on an answered revoke...
    let dispatch = ScriptedDispatch::answered();
    assert!(
        cleanup_pass(committed.clone(), Arc::clone(&dispatch) as Arc<dyn EndpointAccessDispatch>)
            .await
            .is_ok(),
        "an answered revoke is positive proof the entry was removed"
    );
    // ...and on the wire's own absent class, which is proof there was none.
    assert!(
        cleanup_pass(
            committed.clone(),
            ScriptedDispatch::refused(endpoint_absent_code()) as Arc<dyn EndpointAccessDispatch>
        )
        .await
        .is_ok(),
        "the wire's absent class is positive proof no entry was standing"
    );
    // Any other refusal is not proof, and neither is an unanswered dispatch.
    for refused in [
        "endpoint-access-consumer-principal/consumer-principal-row-unresolved",
        "endpoint-access-authority-mismatch",
        "endpoint-access-effect-failed",
    ] {
        assert!(
            cleanup_pass(
                committed.clone(),
                ScriptedDispatch::refused(refused) as Arc<dyn EndpointAccessDispatch>
            )
            .await
            .is_err(),
            "{refused} proves nothing about a standing entry, so the row is retained"
        );
    }
}

// ---------------------------------------------------------------------------
// Retirement barriers: what an owning row waits for
// ---------------------------------------------------------------------------

/// A manager double over the REAL cleanup contract: a row retires only when
/// the driver's own cleanup pass for it CONVERGED, and an owned listing
/// reports exactly the rows that are still committed.
///
/// The production `finalize_owned_resources` reads this listing and reports
/// `ChildrenDraining` while it is non-empty, and that is the barrier an
/// owning `Endpoint` row's own retirement depends on. Nothing here decides a
/// verdict: every driver reached through it is the production one, and this
/// double only reflects what that driver reported.
struct RetiringManager {
    rows: tokio::sync::Mutex<Vec<StoredDesiredResource>>,
    views: tokio::sync::Mutex<Vec<ResourceView>>,
    requested: tokio::sync::Mutex<Vec<ResourceKey>>,
}

impl RetiringManager {
    fn new(rows: Vec<StoredDesiredResource>, views: Vec<ResourceView>) -> Arc<Self> {
        Arc::new(Self {
            rows: tokio::sync::Mutex::new(rows),
            views: tokio::sync::Mutex::new(views),
            requested: tokio::sync::Mutex::new(Vec::new()),
        })
    }

    /// Mark one row deleting, which is what the manager's durable mark commits
    /// before the cleanup pass runs.
    async fn request(&self, key: &ResourceKey) {
        self.requested.lock().await.push(key.clone());
        let mut rows = self.rows.lock().await;
        if let Some(row) = rows.iter_mut().find(|row| row.key == *key) {
            row.deleting = true;
        }
    }

    /// Remove one row, which is what the manager does once that row's cleanup
    /// converged.
    async fn retire(&self, key: &ResourceKey) {
        self.rows.lock().await.retain(|row| row.key != *key);
        self.views.lock().await.retain(|view| view.key != *key);
    }

    /// Every key still committed under the owner uid the given row carries.
    async fn committed(&self, owner: &[u8; 16]) -> Vec<ResourceKey> {
        self.rows
            .lock()
            .await
            .iter()
            .filter(|row| row.owner_uid == Some(*owner))
            .map(|row| row.key.clone())
            .collect()
    }

    /// Every deletion the cleanup passes asked for, in order.
    async fn requested(&self) -> Vec<ResourceKey> {
        self.requested.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl ManagerEndpoint for RetiringManager {
    async fn ensure_child(
        &self,
        _parent: &ResourceKey,
        _child: ChildEnsure,
    ) -> Result<EnsureOutcome, ResourceError> {
        Err(ResourceError::ManagerRejected {
            reason: "this lane drives teardown, not the child surface".into(),
        })
    }

    async fn get(&self, key: &ResourceKey) -> Result<Option<StoredDesiredResource>, ResourceError> {
        Ok(self.rows.lock().await.iter().find(|row| row.key == *key).cloned())
    }

    async fn view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
        Ok(self.views.lock().await.iter().find(|view| view.key == *key).cloned())
    }

    async fn delete(&self, key: &ResourceKey) -> Result<(), ResourceError> {
        self.request(key).await;
        Ok(())
    }

    async fn list_owned(
        &self,
        owner_uid: [u8; 16],
    ) -> Result<Vec<StoredDesiredResource>, ResourceError> {
        Ok(self
            .rows
            .lock()
            .await
            .iter()
            .filter(|row| row.owner_uid == Some(owner_uid))
            .cloned()
            .collect())
    }

    async fn register_watch(
        &self,
        _subscriber: &ResourceKey,
        _registration: WatchRegistration,
    ) -> Result<WatchId, ResourceError> {
        Ok(WatchId(1))
    }

    async fn cancel_watch(&self, _id: WatchId) -> Result<(), ResourceError> {
        Ok(())
    }
}

/// One production `EndpointBinding` driver over the retiring manager.
async fn relationship_driver(
    manager: &Arc<RetiringManager>,
    binding: &StoredDesiredResource,
    dispatch: Arc<dyn EndpointAccessDispatch>,
) -> (
    ResourceContext,
    Box<dyn DynResourceDriver>,
    Arc<RecordingRequeue>,
) {
    let (ctx, requeue) = context(
        binding.clone(),
        endpoint_binding_spec_decoder(),
        Arc::clone(manager) as Arc<dyn ManagerEndpoint>,
    );
    let driver = EndpointBindingDriverFactory::new(EndpointBindingDriverArgs {
        zone: d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("zone id"),
        access: dispatch,
    })
    .create(&binding.key)
    .await;
    (ctx, driver, requeue)
}

/// One production `Endpoint` driver over the retiring manager.
async fn endpoint_driver(
    manager: &Arc<RetiringManager>,
    endpoint_row: &StoredDesiredResource,
) -> (ResourceContext, Box<dyn DynResourceDriver>, Arc<RecordingRequeue>) {
    let (ctx, requeue) = context(
        endpoint_row.clone(),
        endpoint_spec_decoder(),
        Arc::clone(manager) as Arc<dyn ManagerEndpoint>,
    );
    let driver = EndpointDriverFactory::new(EndpointDriverArgs {
        zone: ZONE.to_owned(),
        facets: realized_facets(),
    })
    .create(&endpoint_row.key)
    .await;
    (ctx, driver, requeue)
}

/// The whole neighbourhood the retirement barriers are read over, with each
/// row owned by exactly one thing.
///
/// The owning `Endpoint` row owns the relationship and nothing else, so the
/// owned listing a parent's barrier reads contains the relationship and
/// nothing else; the consumer row belongs to the same session the endpoint
/// belongs to, exactly as a display session owns both.
fn barrier_graph() -> (StoredDesiredResource, StoredDesiredResource) {
    let zone = d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("zone id");
    let endpoint_ref = ResourceRef::parse(ENDPOINT).expect("endpoint ref");
    let session = [0x30; 16];
    let endpoint = [0x42; 16];
    let spec = endpoint_spec(vec![ResourceRef::parse(CONSUMER).expect("consumer ref")]);
    let deliveries =
        declared_endpoint_bindings(&zone, &spec, &endpoint_ref).expect("declared deliveries");
    let derived = canonical_binding_row(&zone, &spec, &endpoint_ref, &deliveries[0])
        .expect("the source derives its committed row");
    let parent = stored(
        ResourceKey::new(ZONE, "Endpoint", "compositor"),
        endpoint,
        Some(session),
        serde_json::to_vec(&spec).expect("endpoint spec bytes"),
    );
    let binding = stored(
        ResourceKey::new(ZONE, ENDPOINT_BINDING_TYPE_NAME, derived.name().as_str()),
        [0x61; 16],
        Some(endpoint),
        derived.spec().to_vec(),
    );
    (parent, binding)
}

/// The manager the barrier cases share: the whole neighbourhood, with the
/// owning row's own published readiness.
fn barrier_manager(
    parent: &StoredDesiredResource,
    binding: &StoredDesiredResource,
) -> Arc<RetiringManager> {
    RetiringManager::new(
        vec![
            stored(
                ResourceKey::new(ZONE, "Zone", ZONE),
                zone_uid_bytes(),
                None,
                envelope("Zone", ZONE, 0x11, None, serde_json::json!({ "display": "compositor" })),
            ),
            stored(
                ResourceKey::new(ZONE, "Process", "frontend"),
                [0x31; 16],
                Some([0x30; 16]),
                envelope(
                    "Process",
                    "frontend",
                    0x31,
                    Some(ENDPOINT),
                    serde_json::json!({ "domain": "system" }),
                ),
            ),
            parent.clone(),
            binding.clone(),
        ],
        vec![endpoint_readiness_view(parent)],
    )
}


/// Assert that the owning `Endpoint` row is held by its own children-first
/// finalization and never reaches its teardown.
async fn endpoint_is_held(
    manager: &Arc<RetiringManager>,
    parent: &StoredDesiredResource,
    label: &str,
) {
    let (mut ctx, mut driver, _requeue) = endpoint_driver(manager, parent).await;
    let failure = driver
        .finalize(&mut ctx)
        .await
        .err()
        .unwrap_or_else(|| {
            panic!("the owning endpoint retires while a relationship child is live ({label})")
        });
    assert_eq!(
        failure.class(),
        FailureClass::Retryable,
        "{label}: the barrier defers the owning row, it never fails it terminally"
    );
    assert_eq!(
        failure.kind().code(),
        FailureKinds::CHILDREN_DRAINING.code(),
        "{label}: the owning row waits on its children, not on its own teardown"
    );
}

/// An `Endpoint` row may not retire while any relationship it owns still
/// holds delivery, a drain, a replacement, or unproven authority (R22).
///
/// Five states of ONE real relationship row, each reached by the production
/// driver and each keeping that row committed. The barrier is the production
/// children-first finalization: it reports `ChildrenDraining` while any owned
/// row is committed, and the owning row's teardown does not run until that
/// listing empties. Only the last case - a revoke the broker positively
/// proves - empties it.
#[tokio::test]
async fn an_endpoint_waits_for_every_relationship_it_still_owns() {
    let (parent, binding) = barrier_graph();
    let manager = barrier_manager(&parent, &binding);
    let endpoint_uid = parent.uid;

    // 1. DELIVERED. A standing grant the broker answered for.
    let dispatch = ScriptedDispatch::answered();
    let (mut ctx, mut driver, _requeue) = relationship_driver(
        &manager,
        &binding,
        Arc::clone(&dispatch) as Arc<dyn EndpointAccessDispatch>,
    )
    .await;
    driver.reconcile(&mut ctx).await.expect("the delivered pass converges");
    assert!(
        matches!(
            ctx.status::<EndpointBindingDriverStatus>(),
            Some(EndpointBindingDriverStatus::Delivered { .. })
        ),
        "the broker answered for the admitted right, so the relationship is delivered"
    );
    endpoint_is_held(&manager, &parent, "delivered").await;

    // 2. REPLACED. A producer rebound its socket: the grant landed on a new
    //    inode, and the consumer holding the old one has to re-derive.
    let (mut ctx, mut driver, _requeue) = relationship_driver(
        &manager,
        &binding,
        Arc::clone(&dispatch) as Arc<dyn EndpointAccessDispatch>,
    )
    .await;
    driver.reconcile(&mut ctx).await.expect("the first pass converges");
    dispatch.pin(0x5151).await;
    driver.reconcile(&mut ctx).await.expect("the rebound pass converges");
    assert!(
        matches!(
            ctx.status::<EndpointBindingDriverStatus>(),
            Some(EndpointBindingDriverStatus::EndpointReplaced { .. })
        ),
        "a producer that replaced its socket reads as a replacement"
    );
    endpoint_is_held(&manager, &parent, "replaced").await;

    // 3. DRAINING. The pre-drain fence is published before any teardown, and
    //    the revoke has not been proved yet.
    let unavailable = ScriptedDispatch::unavailable();
    let (mut ctx, mut driver, _requeue) = relationship_driver(
        &manager,
        &binding,
        Arc::clone(&unavailable) as Arc<dyn EndpointAccessDispatch>,
    )
    .await;
    driver.pre_drain(&mut ctx).await.expect("the pre-drain fence publishes");
    assert_eq!(
        ctx.status::<EndpointBindingDriverStatus>(),
        Some(&EndpointBindingDriverStatus::Draining),
        "new use is fenced before anything is torn down"
    );
    assert!(
        driver.delete(&mut ctx).await.is_err(),
        "an unanswered revoke retains the relationship"
    );
    endpoint_is_held(&manager, &parent, "draining, revoke unproved").await;

    // 4. AMBIGUOUS. A broker that refused for a reason that proves nothing
    //    about whether an entry is standing.
    let ambiguous = ScriptedDispatch::refused("endpoint-access-effect-failed");
    let (mut ctx, mut driver, _requeue) = relationship_driver(
        &manager,
        &binding,
        Arc::clone(&ambiguous) as Arc<dyn EndpointAccessDispatch>,
    )
    .await;
    driver.pre_drain(&mut ctx).await.expect("the pre-drain fence publishes");
    assert!(
        driver.delete(&mut ctx).await.is_err(),
        "an effect failure retains the relationship"
    );
    endpoint_is_held(&manager, &parent, "ambiguous revoke").await;

    // Every state above left the same row committed, and the owning endpoint
    // is still held by exactly that row.
    assert_eq!(
        manager.committed(&endpoint_uid).await,
        vec![binding.key.clone()],
        "the one relationship the owning endpoint still has is what holds it"
    );
    assert!(
        manager
            .requested()
            .await
            .iter()
            .all(|key| *key == binding.key),
        "the owning row only ever asks for this relationship to retire: the repeated requests \
         are the idempotent nudge a children-first finalization issues"
    );

    // 5. ALREADY REVOKED. The broker positively reports that no entry is
    //    standing, the cleanup converges, and the owning row may retire.
    let absent = ScriptedDispatch::refused(endpoint_absent_code());
    let (mut ctx, mut driver, _requeue) = relationship_driver(
        &manager,
        &binding,
        Arc::clone(&absent) as Arc<dyn EndpointAccessDispatch>,
    )
    .await;
    assert!(
        driver.delete(&mut ctx).await.is_ok(),
        "the absent class is positive proof there is nothing left to release"
    );
    manager.retire(&binding.key).await;
    assert!(
        manager.committed(&endpoint_uid).await.is_empty(),
        "the relationship retired on proof"
    );

    let (mut ctx, mut driver, _requeue) = endpoint_driver(&manager, &parent).await;
    driver
        .finalize(&mut ctx)
        .await
        .expect("the owning row converges once its last relationship is gone");
    driver
        .delete(&mut ctx)
        .await
        .expect("the owning row's own teardown runs last");
}

// ---------------------------------------------------------------------------
// The authorization fence and the no-grant proof, each on the production path
// ---------------------------------------------------------------------------

/// A live relationship whose consumer is gone still fences new use.
///
/// The fence is the row's own in-memory status, so a relationship that
/// reached `Delivered` with its consumer intact and lost that consumer
/// afterwards must publish `Draining` rather than skip the fence: the ACL
/// entry it installed may still be standing, and handing a consumer back a
/// delivery it may no longer start is exactly what the fence exists to stop.
///
/// Only an UNDECODABLE relationship converges without a fence. It cannot name
/// its own entry, so there is no delivery for it to hand out either - and
/// that is the case cleanup converges on, which is why the asymmetry with the
/// revoke below is the point rather than an inconsistency.
#[tokio::test]
async fn a_missing_consumer_fences_a_live_relationship_before_its_revoke() {
    let (spec, binding) = committed_relationship();
    let consumer_key = ResourceKey::new(ZONE, "Process", "frontend");
    let manager = serving_manager(graph(&spec, &binding));
    let dispatch = ScriptedDispatch::answered();
    let (mut ctx, _requeue) = context(
        binding.clone(),
        endpoint_binding_spec_decoder(),
        Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
    );
    let mut driver = EndpointBindingDriverFactory::new(EndpointBindingDriverArgs {
        zone: d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("zone id"),
        access: Arc::clone(&dispatch) as Arc<dyn EndpointAccessDispatch>,
    })
    .create(&ResourceKey::new(ZONE, ENDPOINT_BINDING_TYPE_NAME, &binding.key.name))
    .await;

    driver.reconcile(&mut ctx).await.expect("the delivered pass converges");
    assert!(
        matches!(
            ctx.status::<EndpointBindingDriverStatus>(),
            Some(EndpointBindingDriverStatus::Delivered { .. })
        ),
        "the grant is standing, so there is a delivery to stop handing out"
    );

    // The consumer row goes away and nothing else does: the owning `Endpoint`
    // row is byte-identical and its generation is exactly where it was, so
    // nothing here is a source that narrowed its own policy.
    manager.remove_row(&consumer_key).await;

    assert!(
        driver.pre_drain(&mut ctx).await.is_err(),
        "a relationship whose consumer is gone defers to its revoke rather than \
         reporting that it converged"
    );
    assert_eq!(
        ctx.status::<EndpointBindingDriverStatus>(),
        Some(&EndpointBindingDriverStatus::Draining),
        "the fence is published even though the pass could not re-prove the \
         relationship: the entry it installed may still be standing"
    );

    // Cleanup is unaffected, and that asymmetry is deliberate: `delete` derives
    // the revoke from the row's OWN committed bytes precisely so it keeps
    // working when everything around the row has moved.
    assert!(
        driver.delete(&mut ctx).await.is_ok(),
        "the revoke is still derived from the row itself, so a missing consumer \
         does not strand the relationship"
    );
}

/// The positive no-grant proof is the broker's OWN absent class.
///
/// The cleanup barrier retires a row on two proofs: a revoke the broker
/// answered, and a revoke the broker reports as leaving nothing standing. The
/// second one is not a slug this crate invented - it is the closed code the
/// broker's own accept path returns, read here where the broker renders it, so
/// a rename on either side shows up as a test failure rather than as a row
/// that silently stops converging.
///
/// The pass is then driven over the REAL dispatch - the broker's own
/// `accept_endpoint_access` across the broker's own wire codec - against a
/// broker tree that holds no socket at all. The verdict is whatever the broker
/// answered: on a host that has provisioned an account for this fixture's
/// consumer row that answer IS the absent class and the row retires; on a host
/// that has provisioned none, the accept path refuses the consumer principal
/// first, by name, and the row is retained. Both are the broker's own code and
/// the driver acts on exactly that one of them.
#[tokio::test]
async fn the_no_grant_proof_is_the_brokers_own_absent_class() {
    assert_eq!(
        EndpointAccessError::EndpointAbsent.code(),
        endpoint_absent_code(),
        "the positive no-grant proof is exactly the code the broker's own accept \
         path returns when no exact endpoint is standing"
    );

    let (spec, binding) = committed_relationship();
    let host = HostEndpoints::without_socket();
    let dispatch = BrokerBackedDispatch::new(host.runtime_root.clone(), resolver());
    let verdict = cleanup_pass(
        graph(&spec, &binding),
        Arc::clone(&dispatch) as Arc<dyn EndpointAccessDispatch>,
    )
    .await;

    let refusals = dispatch.refusals();
    assert_eq!(
        refusals.len(),
        1,
        "the revoke reached the broker's own resolution path exactly once"
    );
    assert_eq!(
        dispatch
            .sent()
            .iter()
            .map(|(verb, _)| *verb)
            .collect::<Vec<_>>(),
        vec![EndpointAccessVerb::Revoke],
        "and it asked about the row's own slot with the revoke verb, not with a grant"
    );
    assert_eq!(
        verdict.is_ok(),
        refusals[0] == endpoint_absent_code(),
        "the row retires on the absent class and on nothing else; this host answered {}",
        refusals[0]
    );
}

// ---------------------------------------------------------------------------
// AE13: the published authorization digest IS the authorization fence
// ---------------------------------------------------------------------------

/// One committed consumer row whose Provider assignment the case states.
///
/// `providerRef` is the UNIVERSAL desired-state layer, so it rides beside the
/// type's own base spec rather than inside it - which is where
/// `ResourceSpec`, and therefore every reader of a committed row, looks for
/// it. Building it through the fixture's own envelope keeps the row canonical
/// and identical to the one the rest of this lane commits.
fn consumer_row_with_provider(provider: Option<&str>) -> StoredDesiredResource {
    let base = envelope(
        "Process",
        "frontend",
        0x31,
        Some("Process/compositor"),
        serde_json::json!({ "domain": "system" }),
    );
    let mut resource: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(&base).expect("the fixture envelope decodes as an object");
    if let Some(provider) = provider {
        resource.insert(
            "providerRef".to_owned(),
            serde_json::Value::String(provider.to_owned()),
        );
    }
    stored(
        ResourceKey::new(ZONE, "Process", "frontend"),
        [0x31; 16],
        Some([0x42; 16]),
        CanonicalJsonValue::parse(
            &serde_json::to_vec(&serde_json::Value::Object(resource))
                .expect("fixture envelope serializes"),
        )
        .expect("fixture envelope is canonical JSON")
        .to_canonical_bytes(),
    )
}

/// The committed graph with its consumer row replaced by `consumer`, and
/// nothing else touched.
fn graph_with_consumer(spec: &EndpointSpec, consumer: StoredDesiredResource) -> Vec<StoredDesiredResource> {
    committed_rows(spec)
        .into_iter()
        .map(|row| {
            if row.key.type_name == "Process" {
                consumer.clone()
            } else {
                row
            }
        })
        .collect()
}

/// One real `EndpointDriver` pass over `rows`, and the authorization digest it
/// PUBLISHED for the one relationship this endpoint declares.
///
/// The value is read out of the `/endpoint/bindings` layer - the projection a
/// launch gate actually compares - rather than out of the function that mints
/// it, so this lane cannot pass while the publication is still stale.
async fn published_authorization_digest(rows: Vec<StoredDesiredResource>) -> String {
    let endpoint_row = rows
        .iter()
        .find(|row| row.key.type_name == "Endpoint")
        .expect("the graph carries the owning Endpoint row")
        .clone();
    let manager = RecordingManager::with(rows);
    let (mut ctx, _requeue) = context(
        endpoint_row,
        endpoint_spec_decoder(),
        Arc::clone(&manager) as Arc<dyn ManagerEndpoint>,
    );
    EndpointDriverFactory::new(EndpointDriverArgs {
        zone: ZONE.to_owned(),
        facets: realized_facets(),
    })
    .create(&ResourceKey::new(ZONE, "Endpoint", "compositor"))
    .await
    .reconcile(&mut ctx)
    .await
    .expect("the source pass reconciles");
    ctx.take_status_projection()
        .expect("the pass publishes the endpoint layer")
        .pointer("/endpoint/bindings")
        .and_then(serde_json::Value::as_array)
        .and_then(|entries| entries.first())
        .and_then(|entry| entry.pointer("/authorizationDigest"))
        .and_then(serde_json::Value::as_str)
        .expect("every published relationship carries its authorization digest")
        .to_owned()
}

/// AE13, end to end: the PUBLISHED authorization digest moves when the
/// consumer's authorization moves, at an unchanged endpoint row generation.
///
/// This is the property the finding said did not exist. Every input the
/// publication is derived from except the consumer's own authorization is
/// pinned: the endpoint row is byte-identical across all four passes, its
/// generation is the same, and the Zone, the consumer reference, the canonical
/// slot and the publication intent are the same facts. So the digest can only
/// be moving because the authorization moved - which is what lets a launch
/// gate see an authorization-only change with no endpoint generation bump
/// (R16, R18, AE13).
///
/// The delivery withdrawal half is the same read seen from the serving actor:
/// `a_missing_consumer_fences_a_live_relationship_before_its_revoke` above is
/// where a relationship whose consumer stopped being readable stops
/// publishing `Delivered` and publishes `Draining` instead.
#[tokio::test]
async fn the_published_digest_moves_with_the_consumers_authorization_alone() {
    let spec = endpoint_spec(vec![ResourceRef::parse(CONSUMER).expect("consumer ref")]);

    // The baseline is a consumer row that carries BOTH halves of an
    // authorization, so either half can be moved on its own and the only
    // thing left in every other framed input is identical.
    let granted_row = consumer_row_with_provider(Some("Provider/display"));
    let granted = published_authorization_digest(graph_with_consumer(&spec, granted_row.clone())).await;

    // The consumer row is RE-OWNED. Nothing else moves: the owning endpoint is
    // the same row at the same generation.
    let mut reowned = granted_row.clone();
    reowned.owner_uid = Some([0x71; 16]);
    assert_ne!(
        granted,
        published_authorization_digest(graph_with_consumer(&spec, reowned)).await,
        "a consumer owner change is an authorization-only change, and it moves the published \
         digest with the endpoint row generation sitting exactly where it was"
    );

    // The consumer row is RE-ASSIGNED to another Provider, and to none at all.
    assert_ne!(
        granted,
        published_authorization_digest(graph_with_consumer(
            &spec,
            consumer_row_with_provider(Some("Provider/other")),
        ))
        .await,
        "a provider reassignment moves it too"
    );
    let unassigned = consumer_row_with_provider(None);
    assert_ne!(
        granted,
        published_authorization_digest(graph_with_consumer(&spec, unassigned.clone())).await,
        "and withdrawing the Provider the consumer row is assigned to moves it as well"
    );

    // An unchanged graph publishes the unchanged digest: the digest is a
    // function of the authorization, not of the pass that read it.
    assert_eq!(
        granted,
        published_authorization_digest(graph_with_consumer(&spec, granted_row.clone())).await,
        "an unchanged authorization keeps the same published digest across passes"
    );

    // A consumer row a pass CANNOT read is its own answer. It must never
    // produce the digest of a readable relationship - least of all one that
    // was read and carries nothing, which is the collision that would let an
    // unreadable row authorize itself.
    let mut unreadable = granted_row.clone();
    unreadable.spec = b"not the committed row".to_vec();
    let unread = published_authorization_digest(graph_with_consumer(&spec, unreadable)).await;
    assert_ne!(
        granted, unread,
        "a consumer row whose authorization cannot be read never publishes a readable digest"
    );
    assert_ne!(
        published_authorization_digest(graph_with_consumer(&spec, unassigned)).await,
        unread,
        "and it is not the digest of a row that WAS read and carries neither an owner nor a \
         Provider either"
    );

    // A consumer row that is GONE is the same distinct answer, read through the
    // same derivation - and it is the state the serving fence refuses, which is
    // where the delivery stops.
    let orphaned: Vec<StoredDesiredResource> =
        graph_with_consumer(&spec, granted_row.clone())
            .into_iter()
            .filter(|row| row.key.type_name != "Process")
            .collect();
    assert_eq!(
        unread,
        published_authorization_digest(orphaned).await,
        "a consumer row that is absent is reported as unread, never as authorized"
    );

    // The endpoint row generation is the same in every pass above: none of
    // these is a source that changed its own spec.
    assert_eq!(
        committed_rows(&spec)[2].generation,
        1,
        "the endpoint row generation never moved, so every digest change above was \
         authorization-only"
    );
}
