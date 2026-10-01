//! The whole exact-endpoint relationship, end to end through production code
//! (U18, R23).
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
//!    [`accept_endpoint_access`] - over the wire codec, so the grant, the
//!    principal the verified Zone bundle derives, and the pinned inode are the
//!    broker's answers and not this test's;
//! 5. the serving pass reconciles against the pinned `(device, inode)` and the
//!    effective rights the broker read back from the KERNEL.
//!
//! The negatives are the same relationship seen from the other side: a request
//! whose authority key does not reproduce its own committed facts is refused
//! before any path is resolved, a grant lands on the one admitted inode and
//! leaves a sibling socket and an alternate absolute socket untouched, and an
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
};
use d2b_resource_types::WellKnownType;

use d2b_provider_endpoint::{
    ENDPOINT_BINDING_TYPE_NAME, DeviceWorkerEvidenceSource, EndpointAccessDispatch,
    EndpointAccessDispatchError, EndpointBindingDriverArgs, EndpointBindingDriverFactory,
    EndpointBindingDriverStatus, EndpointDriverArgs, EndpointDriverEffects, EndpointDriverFactory,
    EndpointPurposeVocabulary, EndpointSocketIdentity, EndpointSocketSource, GuestControlProducer,
    GuestVmmEvidenceSource, canonical_binding_row, declared_endpoint_bindings,
    VIRTIOFSD_PURPOSE, endpoint_binding_descriptor, endpoint_binding_spec_decoder,
    endpoint_delivery_slot, endpoint_spec_decoder,
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
    ensured: tokio::sync::Mutex<Vec<ChildEnsure>>,
    deleted: tokio::sync::Mutex<Vec<ResourceKey>>,
    watched: tokio::sync::Mutex<Vec<ResourceKey>>,
}

impl RecordingManager {
    fn with(rows: Vec<StoredDesiredResource>) -> Arc<Self> {
        Arc::new(Self {
            rows: tokio::sync::Mutex::new(rows),
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

    async fn view(&self, _key: &ResourceKey) -> Result<Option<ResourceView>, ResourceError> {
        Ok(None)
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
    answers: Mutex<Vec<EndpointAccessResponse>>,
}

impl BrokerBackedDispatch {
    fn new(runtime_root: PathBuf, resolver: BundleResolver) -> Arc<Self> {
        Arc::new(Self {
            runtime_root,
            resolver: Arc::new(resolver),
            sent: Mutex::new(Vec::new()),
            answers: Mutex::new(Vec::new()),
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

    /// Record one answer the accept path returned.
    fn record_answer(&self, answer: &EndpointAccessResponse) {
        self.answers.lock().expect("answers lock").push(answer.clone());
    }

    fn sent(&self) -> Vec<(EndpointAccessVerb, EndpointAccessRequest)> {
        self.sent.lock().expect("sent lock").clone()
    }

    /// The principal the broker derived for the consumer, as its own answer
    /// reported it. The case never assumes a number: it reads the value the
    /// verified Zone bundle produced, which is what the ACL names.
    fn consumer_uid(&self) -> u32 {
        self.answers
            .lock()
            .expect("answers lock")
            .first()
            .expect("the broker answered at least once")
            .consumer_uid
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
            EndpointAccessDispatchError::Refused(match error {
                EndpointAccessError::ConsumerPrincipal { code } => {
                    format!("endpoint-access-consumer-principal/{code}")
                }
                other => other.code().to_owned(),
            })
        })?;
        self.record_answer(&answer);
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
            privileges_path: "privileges.json".to_owned(),
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
fn endpoint_spec(subjects: Vec<ResourceRef>) -> EndpointSpec {
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
        // enumerate it would hold the directory authority R23 removed, so the
        // host tree under test is built in the posture a correct grant has to
        // survive rather than in one that would hand listing back.
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

/// The permission the kernel actually applies to `uid` at `path`, read through
/// the ACL the broker wrote rather than through the mode it asked for.
///
/// `getfacl` reports the named entry already ANDed with the ACL mask - the
/// value an access check actually uses - so this reads the EFFECTIVE
/// permission independently of the broker's own xattr parser. `None` when the
/// path carries no entry for `uid` at all.
fn effective_permission(path: &Path, uid: u32) -> Option<u32> {
    let tool = acl_tool("getfacl").unwrap_or_else(|| {
        panic!(
            "getfacl is required to read the effective permission the kernel applies at {}",
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
    let rendered = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut mask = 0o7;
    let mut named_user = None;
    for line in rendered.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("mask::") {
            mask = parse_acl_permission(rest);
        }
        if let Some(rest) = line.strip_prefix(&format!("user:{uid}:")) {
            named_user = Some(parse_acl_permission(rest));
        }
    }
    named_user.map(|perm| perm & mask)
}

/// Parse one `getfacl` permission triplet, ignoring an `#effective:` note.
fn parse_acl_permission(rendered: &str) -> u32 {
    let triplet = rendered.split_whitespace().next().unwrap_or_default();
    triplet
        .bytes()
        .map(|symbol| match symbol {
            b'r' => 4,
            b'w' => 2,
            b'x' => 1,
            b'-' => 0,
            other => panic!("unknown getfacl permission symbol {other} in {rendered:?}"),
        })
        .sum()
}

// ---------------------------------------------------------------------------
// The case
// ---------------------------------------------------------------------------

/// The whole relationship: one committed `Endpoint` row derives its
/// `EndpointBinding`, the row commits idempotently, the serving pass's delivery
/// reaches the broker's own accept path, and the serving pass reconciles
/// against the pinned inode and the effective rights the kernel applies.
///
/// AE7 and AE19 are both answered here from the kernel's side: the sibling and
/// alternate sockets gain nothing, and the relationship is held to the bits the
/// broker read back rather than the mode it asked for.
#[tokio::test]
async fn a_committed_endpoint_derives_commits_and_delivers_the_exact_endpoint() {
    let endpoint_ref = ResourceRef::parse(ENDPOINT).expect("endpoint ref");
    let consumer_ref = ResourceRef::parse(CONSUMER).expect("consumer ref");
    let zone = d2b_contracts_resource::v3::ZoneId::parse(ZONE).expect("zone id");
    let spec = endpoint_spec(vec![consumer_ref.clone()]);

    // The slot the source derives, and therefore the socket the broker will
    // resolve: one endpoint, one socket name.
    let slot = endpoint_delivery_slot(&zone, &endpoint_ref).expect("derived delivery slot");
    let host = HostEndpoints::new(slot.as_str());
    let socket_path = host.admitted(slot.as_str());

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
    //    accept path.
    let dispatch = BrokerBackedDispatch::new(host.runtime_root.clone(), resolver());
    let binding_row = manager.bindings().await.remove(0);
    let status = deliver(graph(&spec, &binding_row), Arc::clone(&dispatch)).await;
    let (delivered_socket, effective_rights) = match status {
        EndpointBindingDriverStatus::Delivered {
            socket,
            effective_rights,
        } => (socket, effective_rights),
        other => panic!("the exact endpoint is delivered, not {other:?}"),
    };

    // The pinned identity is the one the KERNEL has, read back from the inode
    // the broker resolved rather than recomputed here.
    assert_eq!(
        delivered_socket,
        pinned_identity(&socket_path),
        "the delivery is held to the inode the broker pinned"
    );
    let consumer_uid = dispatch.consumer_uid();
    assert_eq!(
        effective_permission(&socket_path, consumer_uid),
        Some(effective_rights),
        "the effective rights are what the kernel applies, not the mode asked for"
    );
    assert_eq!(effective_rights & 0o6, 0o6, "a connect needs read and write");

    // 5. AE7: the grant lands on the one admitted inode and nowhere else.
    let sibling = host.admitted(SIBLING);
    let alternate = host.alternate();
    // `None` means the path carries no named entry for the principal at all,
    // which is the strongest form of "nothing was granted here": the kernel
    // applies nothing on the consumer's behalf.
    assert_eq!(
        effective_permission(&sibling, consumer_uid).unwrap_or(0),
        0,
        "a grant that reached a sibling socket would be a broader relationship"
    );
    assert_eq!(
        effective_permission(&alternate, consumer_uid).unwrap_or(0),
        0,
        "an alternate absolute socket is not reachable through this relationship"
    );

    // 6. The relationship is re-observed, not re-granted, on the next pass:
    //    an unchanged endpoint stays delivered against the same inode.
    let statuses = deliver_passes(
        graph(&spec, &binding_row),
        Arc::clone(&dispatch),
        2,
        None,
    )
    .await;
    assert!(matches!(
        statuses[1],
        EndpointBindingDriverStatus::Delivered { socket, .. } if socket == delivered_socket
    ));
    assert_eq!(
        statuses.iter().filter(|status| **status
            == EndpointBindingDriverStatus::Delivered {
                socket: delivered_socket,
                effective_rights
            })
        .count(),
        2,
        "the second pass observed the standing grant rather than replacing it"
    );

    // 7. A replaced inode is reported as replaced, not as the access that used
    //    to be there, and the grant lands on the new one. The replacement
    //    happens BETWEEN two passes of the SAME actor, because the fence a
    //    standing grant is compared against is the row's own in-memory status
    //    and nothing durable records which inode it pinned.
    drop(manager);
    // The rebound listener has to outlive both passes - keeping it bound IS
    // what makes the second pass observe a replaced inode - so the hook shares
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
    let replacement_identity = pinned_identity(&socket_path);
    assert_ne!(
        replacement_identity, delivered_socket,
        "the rebound socket is a different inode"
    );
    assert_eq!(
        statuses[1],
        EndpointBindingDriverStatus::EndpointReplaced {
            socket: replacement_identity,
        },
        "a producer that replaced its socket is visible as a different inode"
    );
    assert_eq!(
        effective_permission(&socket_path, consumer_uid).unwrap_or(0),
        0o6,
        "the re-grant lands on the new inode"
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
/// in-memory status slot (R11), so a second pass is only meaningful over the
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
    let manager = RecordingManager::with(graph);
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
