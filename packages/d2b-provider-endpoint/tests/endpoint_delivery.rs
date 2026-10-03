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
    ENDPOINT_BINDING_TYPE_NAME, DeviceWorkerEvidenceSource, EndpointAccessDispatch,
    EndpointAccessDispatchError, EndpointBindingDriverArgs, EndpointBindingDriverFactory,
    EndpointBindingDriverStatus, EndpointDeliveryRefusal, EndpointDriverArgs,
    EndpointDriverEffects, EndpointDriverFactory, EndpointPurposeVocabulary,
    EndpointSocketIdentity, EndpointSocketSource, GuestControlProducer, GuestVmmEvidenceSource,
    canonical_binding_row, declared_endpoint_bindings, VIRTIOFSD_PURPOSE,
    endpoint_binding_descriptor, endpoint_binding_spec_decoder, endpoint_delivery_slot,
    endpoint_spec_decoder,
};
use d2b_resource_runtime::context::{
    ChildEnsure, ManagerEndpoint, RequeueId, RequeueScheduler, ResourceContext, SpecDecoder,
    WatchId, WatchRegistration,
};
use d2b_resource_runtime::driver::ResourceDriverFactory;
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
/// effect is a double.
fn realized_facets() -> d2b_provider_endpoint::EndpointEffectFacets {
    d2b_provider_endpoint::EndpointEffectFacets {
        socket: Arc::new(RealizedSocketEffects),
        guest_vmm: Arc::new(RealizedSocketEffects),
        device_worker: Arc::new(RealizedSocketEffects),
    }
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
        provenance: endpoint.provenance.clone(),
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

    fn new(admitted: &str) -> Self {
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
        let mut listeners = vec![UnixListener::bind(endpoints.join(admitted))
            .expect("bind the admitted endpoint")];
        listeners.push(
            UnixListener::bind(endpoints.join(SIBLING)).expect("bind the sibling endpoint"),
        );
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
