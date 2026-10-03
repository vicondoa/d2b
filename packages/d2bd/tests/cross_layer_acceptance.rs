//! The cross-layer acceptance seam: one declaration, carried from the compiler
//! to a handler invocation, and the refusal that must stop it there.
//!
//! # Why this suite exists
//!
//! Every layer below is tested on its own, which is exactly how a broken seam
//! between them survives: a compiler that emits a row the manager will not own,
//! a manager that commits a row the broker never accepted, a broker that
//! accepts a candidate the store never staged. Each of those is a green unit
//! test and a broken product.
//!
//! So the chain is driven here as ONE flow over ONE declaration, and the
//! assertion is about the far end rather than about the messages that crossed:
//! the last leg asserts what the production `Provider` driver published after
//! reading the row the compiler generated, not that a publication was sent.
//!
//! # What is real, and what is not
//!
//! Real, unmodified:
//!
//! - the declaration compiler - a signed in-memory Provider artifact through
//!   `compile_artifact`, then the controller projection through
//!   `project_static_controller_processes`, so the `Process` row and its
//!   private `ProcessTemplateBinding` are what the compiler actually emits;
//! - the per-Zone manager actor and its `SpecStore` - the same
//!   `ResourceManager` the daemon runs, driven through its own client;
//! - the publication coordinator, the wire envelopes, and the broker's own
//!   `AuthorityProjection` - every message is the production one and every
//!   answer is the broker's own projection, so a refusal here is a refusal the
//!   daemon reads in production;
//! - both row handlers - the `Provider` type's production driver and the
//!   `Process` family's production driver.
//!
//! Substituted, and only here:
//!
//! - the unix socket, through a link that hands the production envelopes
//!   straight to the projection and records the broker's typed refusals;
//! - the daemon's live controller-session seam, implemented here over one exact
//!   `(process_ref, uid, generation)` triple. That is the production port the
//!   `Provider` driver reads; production fills it from an admitted controller
//!   session the daemon holds, and this suite fills it for exactly the row it
//!   committed.
//!
//! The one thing this suite adds around a handler is an observation point: a
//! wrapper that forwards every call to the production `Provider` driver and
//! reads back the status that driver published. It decides nothing, and every
//! value asserted below is one the real driver computed.
//!
//! The compiler's controller row is registered with a driver that owns its row
//! and publishes nothing. That row's own runtime handler is not this suite's
//! subject - what the Provider handler reads is the committed row and the
//! manager's ownership of it - and the Provider handler reports no readiness
//! that this suite did not earn from the row and the session it resolved.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use d2b_broker::authority_projection::AuthorityProjection;
use d2b_contracts_broker::broker_wire::{
    AuthorityPublicationEnvelope, AuthorityPublicationOpen, AuthorityPublicationResponse,
    PublicationRefusal, ZoneAuthorityState, PUBLICATION_CONTROL_NOT_BOUND,
};
use d2b_contracts_provider::v3::{
    ArtifactDigest, ArtifactDigestSet, CompatibilityRange, ComponentDescriptor,
    ComponentTargetCapability, ComponentType, ControllerInstanceScope, ControllerTargetKind,
    EffectPortClass, PolicyEvaluation, ProviderManifest, ResourceApiBinding, RevocationState,
    SignatureState, StandardCapabilityMatrix, TrustEvidence,
    provider::{
        BinaryRef, ComponentExecution, TargetRuntimeArtifacts, UpgradeDisposition, UpgradePolicy,
    },
};
use d2b_contracts_resource::v3::{
    AdmissionStage, ArtifactId, AuthoritySubject, AuthoritySubjectKind, CanonicalJsonValue,
    RefusalReason, ResourceRef, ResourceTypeName, ResourceUid,
    canonical_json_bytes,
    execution_policy::{BoundedToken, ExecutionDomain},
    identity::SchemaFingerprint,
    resource_schema::{PlacementAnchor, SchemaVersion},
};
use d2b_contracts_zone_session::v3::resource_bundle::ProcessTemplateBinding;
use d2b_core::provider_artifact::{
    AnchoredDir, ExecutableFile, LayoutDir, LayoutError, LayoutPath, ReadableFile,
};
use d2b_provider_provider::{ProviderDriverFactory, ProviderDriverStatus};
use d2b_resource_compiler::{
    ArtifactCatalogEntry, BootstrapBoundary, CONFIG_SCHEMA_PATH, CatalogDigests, MANIFEST_PATH,
    SIGNATURE_PATH, StaticPublisherKeys, VerifiedProviderArtifact, compile_artifact,
    executable_set_digest, project_static_controller_processes, sha256_digest,
};
use d2b_resource_runtime::context::{ResourceContext, SpecDecoder};
use d2b_resource_runtime::driver::{
    DynResourceDriver, ReconcileOutcome, RecoveryOutcome, ResourceDriverFactory,
};
use d2b_resource_runtime::error::DriverFailure;
use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName as RuntimeTypeName};
use d2b_resource_runtime::manager::{
    AllowAll, DesiredResource, MutationSubject, ResourceManager, ResourceManagerArgs,
    ResourceManagerClient,
};
use d2b_resource_runtime::provider::ProviderDirectory;
use d2b_resource_runtime::spec_store::{ResourceProvenance, SpecStore};
use d2b_resource_runtime::target::{TargetDirectory, TargetRef, TargetResolver};
use d2b_resource_runtime::watch::{DEFAULT_RING_CAPACITY, WatchHub};
use d2bd::authority_publication::{
    AuthorityPublicationCoordinator, AuthorityPublicationLink, CoordinatorPublisher,
};
use ring::signature::{Ed25519KeyPair, KeyPair};
use tempfile::TempDir;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// The declaration's own vocabulary
// ---------------------------------------------------------------------------

/// The Zone the admitted declaration lands in.
const ZONE: &str = "work";
/// The Zone the refused declaration lands in.
const REFUSED_ZONE: &str = "lab";
/// The declared Provider artifact id, and the Zone's `Provider` row name.
const ARTIFACT: &str = "provider-acceptance";
/// The manifest's declared publisher.
const PUBLISHER: &str = "first-party";
/// The publisher key id the catalog names.
const SIGNATURE_ID: &str = "test-key";
/// The Host the declared controller executes against.
const HOST: &str = "host";
/// The controller component the manifest declares, and its executable name.
const COMPONENT: &str = "acceptance-controller";
/// The default metadata envelope every seeded row carries.
const METADATA: &[u8] = br#"{"annotations":{},"labels":{},"ownerRef":null}"#;

// ---------------------------------------------------------------------------
// The declaration: a signed Provider artifact, compiled and projected
// ---------------------------------------------------------------------------

#[derive(Clone)]
enum Node {
    File { bytes: Vec<u8>, mode: u32 },
    Directory,
}

#[derive(Default, Clone)]
struct MemoryDir {
    nodes: BTreeMap<String, Node>,
}

impl MemoryDir {
    fn file(mut self, path: &str, bytes: Vec<u8>, mode: u32) -> Self {
        self.nodes.insert(path.to_owned(), Node::File { bytes, mode });
        self
    }

    fn node(mut self, path: &str, node: Node) -> Self {
        self.nodes.insert(path.to_owned(), node);
        self
    }

    fn names(&self, dir: &str) -> Vec<OsString> {
        let prefix = format!("{dir}/");
        let mut names = BTreeSet::new();
        for path in self.nodes.keys() {
            if let Some(rest) = path.strip_prefix(&prefix)
                && let Some(name) = rest.split('/').next()
            {
                names.insert(name.to_owned());
            }
        }
        names.into_iter().map(OsString::from).collect()
    }
}

#[derive(Clone)]
struct MemoryFile {
    bytes: Vec<u8>,
    offset: usize,
}

impl ReadableFile for MemoryFile {
    fn len(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn read_prefix(&mut self, output: &mut [u8]) -> Result<usize, LayoutError> {
        let remaining = &self.bytes[self.offset..];
        let count = remaining.len().min(output.len());
        output[..count].copy_from_slice(&remaining[..count]);
        self.offset += count;
        Ok(count)
    }

    fn read_to_digest(self) -> Result<[u8; 32], LayoutError> {
        let hex = sha256_digest(&self.bytes)
            .as_str()
            .strip_prefix("sha256:")
            .expect("a contract digest")
            .to_owned();
        let mut out = [0_u8; 32];
        for (index, chunk) in hex.as_bytes().chunks_exact(2).enumerate() {
            out[index] = (nibble(chunk[0]) << 4) | nibble(chunk[1]);
        }
        Ok(out)
    }
}

fn nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => panic!("a contract digest is lower hex"),
    }
}

#[derive(Clone)]
struct MemoryExecutable;

impl ExecutableFile for MemoryExecutable {}

impl AnchoredDir for MemoryDir {
    type Readable = MemoryFile;
    type Executable = MemoryExecutable;

    fn open_readable(&self, path: LayoutPath) -> Result<Self::Readable, LayoutError> {
        let path = path.as_str();
        match self.nodes.get(path) {
            Some(Node::File { bytes, mode }) => {
                if path.starts_with("bin/") && mode & 0o111 == 0 {
                    return Err(LayoutError::NotExecutable);
                }
                Ok(MemoryFile {
                    bytes: bytes.clone(),
                    offset: 0,
                })
            }
            Some(Node::Directory) | None => Err(LayoutError::Absent),
        }
    }

    fn open_executable(&self, path: LayoutPath) -> Result<Self::Executable, LayoutError> {
        match self.nodes.get(path.as_str()) {
            Some(Node::File { .. }) => Ok(MemoryExecutable),
            Some(Node::Directory) | None => Err(LayoutError::Absent),
        }
    }

    fn entries(&self, dir: LayoutDir) -> Result<Vec<OsString>, LayoutError> {
        let dir = dir.as_str();
        let prefix = format!("{dir}/");
        let exists = matches!(self.nodes.get(dir), Some(Node::Directory))
            || self.nodes.keys().any(|path| path.starts_with(&prefix));
        if !exists {
            return Err(LayoutError::Absent);
        }
        Ok(self.names(dir))
    }
}


fn token(value: &str) -> BoundedToken {
    BoundedToken::parse(value).expect("a bounded token")
}

fn resource_type(value: &str) -> ResourceTypeName {
    ResourceTypeName::parse(value).expect("a registered resource type")
}

fn digest(value: &str) -> ArtifactDigest {
    ArtifactDigest::parse(value).expect("a canonical artifact digest")
}

fn fingerprint() -> SchemaFingerprint {
    SchemaFingerprint::parse(
        "sha256:0000000000000000000000000000000000000000000000000000000000000001",
    )
    .expect("a schema fingerprint")
}

fn elf() -> Vec<u8> {
    let mut bytes = vec![0_u8; 64];
    bytes[0..4].copy_from_slice(b"\x7fELF");
    bytes[4] = 2;
    bytes[5] = 1;
    bytes[6] = 1;
    bytes[16..18].copy_from_slice(&3_u16.to_le_bytes());
    bytes
}

fn config_schema() -> Vec<u8> {
    canonical_json_bytes(
        &CanonicalJsonValue::parse(br#"{"type":"object"}"#).expect("a canonical schema"),
    )
    .expect("canonical schema bytes")
}

/// The manifest the declaration carries: one launchable controller component
/// whose host target capability pins the exact executable digest the compiler
/// recomputes from the signed package.
fn manifest(
    binary_digest: ArtifactDigest,
    executable_set: ArtifactDigest,
    config_digest: ArtifactDigest,
) -> ProviderManifest {
    let component = ComponentDescriptor::new(
        token(COMPONENT),
        ComponentType::Controller,
        [resource_type("Process")],
        [],
        [ExecutionDomain::System],
        1,
        config_digest.clone(),
        [],
    )
    .expect("the component descriptor is well formed")
    .with_execution(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse(COMPONENT).expect("a bounded binary reference"),
    })
    .with_controller_placement(
        ControllerInstanceScope::PerResourceTarget,
        [ControllerTargetKind::Host],
    )
    .expect("a controller placement the manifest accepts")
    .with_target_capabilities([ComponentTargetCapability::new(
        ControllerTargetKind::Host,
        binary_digest,
        [EffectPortClass::Process],
    )
    .expect("a host target capability")])
    .expect("the component declares its host capability");
    let binding = ResourceApiBinding::new_with_placement(
        resource_type("Process"),
        SchemaVersion::new(1, 0).expect("a schema version"),
        fingerprint(),
        SchemaVersion::new(1, 0).expect("a schema version"),
        fingerprint(),
        StandardCapabilityMatrix::default(),
        None,
        None,
        PlacementAnchor::ExecutionRef,
    )
    .expect("a resource api binding");
    ProviderManifest::new(
        ArtifactId::parse(ARTIFACT).expect("a bounded artifact id"),
        ArtifactDigestSet {
            executable: executable_set.clone(),
            config: config_digest,
            schema: digest(
                "sha256:0000000000000000000000000000000000000000000000000000000000000001",
            ),
            service: digest(
                "sha256:0000000000000000000000000000000000000000000000000000000000000001",
            ),
        },
        TrustEvidence {
            publisher: token(PUBLISHER),
            root_epoch: 1,
            publisher_trusted: true,
            signature: SignatureState::Valid,
            revocation: RevocationState::Clear,
            emergency_deny: false,
            provenance: PolicyEvaluation::Accepted,
            sbom: PolicyEvaluation::Accepted,
            license: PolicyEvaluation::Accepted,
            vulnerability: PolicyEvaluation::Accepted,
            conformance: PolicyEvaluation::Accepted,
            support_channel: token("stable"),
        },
        CompatibilityRange {
            api_major: 3,
            api_minor: 0,
            descriptor_fingerprint: fingerprint(),
            state_schema_version: SchemaVersion::new(1, 0).expect("a schema version"),
        },
        [component],
        [binding],
        [],
        UpgradePolicy {
            drain_before_upgrade: true,
            max_automatic_disposition: UpgradeDisposition::InPlace,
            preserves_durable_state: true,
        },
    )
    .expect("the provider manifest is well formed")
    .with_target_runtime_artifacts([TargetRuntimeArtifacts::new(
        ControllerTargetKind::Host,
        executable_set.clone(),
        executable_set,
    )
    .expect("a host runtime artifact entry")])
    .expect("the manifest declares its host runtime artifacts")
}

fn pem(public_key: &[u8]) -> Vec<u8> {
    let mut der = vec![0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];
    der.extend_from_slice(public_key);
    format!(
        "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
        STANDARD.encode(der)
    )
    .into_bytes()
}

/// The declared bootstrap boundary: no component of this declaration is an
/// in-process bootstrap row, so the projection has one controller to emit.
fn declared() -> BootstrapBoundary {
    BootstrapBoundary::default()
}

/// What the compiler emits for one declared Provider: its Zone graph row and
/// the private template binding that names the row it projected.
struct DeclaredController {
    /// The `Provider` resource the declaration names, as canonical spec bytes.
    provider_spec: Vec<u8>,
    /// The `Process` resource the compiler generated for the controller.
    controller: serde_json::Value,
    /// The private template binding the compiler emitted for it.
    template: ProcessTemplateBinding,
}

fn declaration(zone: &str) -> DeclaredController {
    let binary = elf();
    let binary_digest = sha256_digest(&binary);
    let executable_set =
        executable_set_digest(&BTreeMap::from([(COMPONENT.to_owned(), binary_digest.clone())]))
            .expect("a one-binary executable set digest");
    let schema = config_schema();
    let config_digest = sha256_digest(&schema);
    let manifest = manifest(binary_digest, executable_set.clone(), config_digest.clone());
    let manifest_bytes =
        canonical_json_bytes(&manifest).expect("the manifest serializes canonically");
    let keypair = Ed25519KeyPair::from_seed_unchecked(&[7_u8; 32]).expect("a test signing key");
    let signature = keypair.sign(&manifest_bytes).as_ref().to_vec();
    let mut keys = StaticPublisherKeys::default();
    keys.insert_key(PUBLISHER, SIGNATURE_ID, pem(keypair.public_key().as_ref()));
    let tree = MemoryDir::default()
        .file(MANIFEST_PATH, manifest_bytes.clone(), 0o644)
        .file(SIGNATURE_PATH, signature, 0o644)
        .file(CONFIG_SCHEMA_PATH, schema, 0o644)
        .node("share/d2b/provider", Node::Directory)
        .file(&format!("bin/{COMPONENT}"), binary, 0o755)
        .node("bin", Node::Directory);
    let entry = ArtifactCatalogEntry::new(
        ArtifactId::parse(ARTIFACT).expect("a bounded artifact id"),
        "/nix/store/acceptance-provider",
        PUBLISHER,
        SIGNATURE_ID,
        CatalogDigests::new(
            sha256_digest(b"selected-output-nar"),
            executable_set,
            sha256_digest(&manifest_bytes),
            config_digest,
        ),
    );
    let compiled = compile_artifact(&entry, &tree, &keys, &declared())
        .expect("the signed artifact verifies");
    let resources = vec![
        serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Host",
            "metadata": {"name": HOST, "zone": zone},
            "spec": {}
        }),
        serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Provider",
            "metadata": {"name": ARTIFACT, "zone": zone},
            "spec": {
                "artifactId": ARTIFACT,
                "config": {"controllerExecutionRef": format!("Host/{HOST}")}
            }
        }),
    ];
    let provider_spec = canonical(&resources[1]["spec"]);
    let projection = project_static_controller_processes(
        zone,
        &resources,
        &declared(),
        &[VerifiedProviderArtifact::new(
            entry.artifact_id().clone(),
            entry.store_path().to_path_buf(),
            compiled,
        )],
    )
    .expect("the declaration projects one controller");
    assert_eq!(
        projection.resources.len(),
        1,
        "the compiler projects exactly one controller row"
    );
    let template = projection
        .templates
        .into_iter()
        .next()
        .expect("the compiler emitted one template binding");
    DeclaredController {
        provider_spec,
        controller: projection
            .resources
            .into_iter()
            .next()
            .expect("the projected controller row"),
        template,
    }
}

/// The daemon's live controller-session seam, implemented here over the exact
/// identity the handler must have read.
///
/// The port is production (`ProviderDriverEffects`); production fills it from
/// an admitted controller session the daemon holds. This implementation
/// answers for one exact `(process_ref, uid, generation)` triple and for no
/// other, so a handler that read a different row gets nothing - which is what
/// makes the handler's resolved session evidence a statement about the row it
/// actually observed.
struct SessionEvidence {
    admitted: Mutex<Option<(ResourceRef, ResourceUid, u64, serde_json::Value)>>,
}

impl SessionEvidence {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            admitted: Mutex::new(None),
        })
    }

    /// Admit exactly one controller session.
    async fn admit(
        &self,
        process_ref: ResourceRef,
        process_uid: ResourceUid,
        generation: u64,
        evidence: serde_json::Value,
    ) {
        *self.admitted.lock().await = Some((process_ref, process_uid, generation, evidence));
    }
}

#[async_trait]
impl d2b_provider_provider::ProviderDriverEffects for SessionEvidence {
    fn controller_session_evidence(
        &self,
        process_ref: &ResourceRef,
        process_uid: &d2b_contracts_resource::v3::ResourceUid,
        generation: d2b_contracts_resource::v3::ResourceGeneration,
    ) -> Option<serde_json::Value> {
        let held = self.admitted.try_lock().ok()?;
        let (ref_held, uid_held, generation_held, evidence) = held.as_ref()?;
        if ref_held != process_ref
            || uid_held.as_str() != process_uid.as_str()
            || *generation_held != generation.get()
        {
            return None;
        }
        let mut evidence = evidence.clone();
        // The identity the handler read is stamped into the evidence it
        // receives, so the evidence it validates cannot name any other row.
        evidence["processRef"] = serde_json::Value::String(process_ref.to_canonical_string());
        evidence["processUid"] = serde_json::Value::String(process_uid.as_str().to_owned());
        evidence["processGeneration"] = serde_json::Value::from(generation.get());
        Some(evidence)
    }
}

/// A driver that owns its row and publishes nothing.
///
/// Registered for the controller `Process` row the compiler projected so the
/// manager can commit it and record its ownership. The Provider handler reads
/// the committed row and its ownership through the manager; this driver never
/// reports a phase, so the Provider's readiness is decided by the Provider
/// handler's own observation alone.
struct RowOnlyFactory {
    types: Vec<RuntimeTypeName>,
}

impl RowOnlyFactory {
    fn new(types: Vec<RuntimeTypeName>) -> Arc<Self> {
        Arc::new(Self { types })
    }
}

#[async_trait]
impl ResourceDriverFactory for RowOnlyFactory {
    fn resource_types(&self) -> &[RuntimeTypeName] {
        &self.types
    }

    async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(RowOnlyDriver)
    }
}

struct RowOnlyDriver;

#[async_trait]
impl DynResourceDriver for RowOnlyDriver {
    async fn validate(&mut self, _ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        Ok(())
    }

    async fn recover(&mut self, _ctx: &mut ResourceContext) -> Result<RecoveryOutcome, DriverFailure> {
        Ok(RecoveryOutcome::Missing)
    }

    async fn reconcile(
        &mut self,
        _ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, DriverFailure> {
        Ok(ReconcileOutcome::Satisfied)
    }

    async fn pre_drain(&mut self, _ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        Ok(())
    }

    async fn finalize(&mut self, _ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        Ok(())
    }

    async fn delete(&mut self, _ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        Ok(())
    }
}

/// The canonical bytes of one desired spec object, the shape the store commits
/// and the broker re-evaluates.
fn canonical(value: &serde_json::Value) -> Vec<u8> {
    canonical_json_bytes(
        &CanonicalJsonValue::parse(&serde_json::to_vec(value).expect("spec bytes"))
            .expect("a canonical spec object"),
    )
    .expect("canonical spec bytes")
}

// ---------------------------------------------------------------------------
// The broker leg: the real projection, with the socket removed
// ---------------------------------------------------------------------------

/// The daemon's publication link bound straight to the broker's own projection.
///
/// Every envelope is the production one and every answer is the projection's,
/// so the only thing removing the socket changes is where the bytes go. The
/// link records the broker's typed refusals, which is the one place the
/// manager's rendered error is not the whole answer.
#[derive(Clone)]
struct ProjectionLink {
    projection: Arc<AuthorityProjection>,
    refusals: Arc<Mutex<Vec<PublicationRefusal>>>,
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
        match self.projection.serve(&envelope).await {
            Ok(response) => Ok(response),
            Err(error) => {
                if let Some(refusal) = error.refusal() {
                    self.refusals.lock().await.push(refusal.clone());
                }
                Err(error.to_string())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The handler leg: the production Provider driver, observed where it speaks
// ---------------------------------------------------------------------------

/// One completed pass of the production `Provider` driver, with the row it ran
/// against and the status that driver published.
#[derive(Clone)]
struct HandlerPass {
    key: ResourceKey,
    status: ProviderDriverStatus,
}

/// The `Provider` type's production driver, wrapped so each completed pass is
/// recorded with the status the real driver set.
///
/// Nothing here decides anything: every call is forwarded to the production
/// driver, and the only thing added is reading back the in-memory status that
/// driver published.
struct ObservedProviderFactory {
    inner: Arc<dyn ResourceDriverFactory>,
    journal: Arc<Mutex<Vec<HandlerPass>>>,
}

#[async_trait]
impl ResourceDriverFactory for ObservedProviderFactory {
    fn resource_types(&self) -> &[RuntimeTypeName] {
        self.inner.resource_types()
    }

    async fn create(&self, key: &ResourceKey) -> Box<dyn DynResourceDriver> {
        Box::new(ObservedProviderDriver {
            inner: self.inner.create(key).await,
            key: key.clone(),
            journal: Arc::clone(&self.journal),
        })
    }
}

struct ObservedProviderDriver {
    inner: Box<dyn DynResourceDriver>,
    key: ResourceKey,
    journal: Arc<Mutex<Vec<HandlerPass>>>,
}

#[async_trait]
impl DynResourceDriver for ObservedProviderDriver {
    async fn validate(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        self.inner.validate(ctx).await
    }

    async fn recover(&mut self, ctx: &mut ResourceContext) -> Result<RecoveryOutcome, DriverFailure> {
        self.inner.recover(ctx).await
    }

    async fn reconcile(
        &mut self,
        ctx: &mut ResourceContext,
    ) -> Result<ReconcileOutcome, DriverFailure> {
        let outcome = self.inner.reconcile(ctx).await?;
        if let Some(status) = ctx.status::<ProviderDriverStatus>() {
            self.journal.lock().await.push(HandlerPass {
                key: self.key.clone(),
                status: status.clone(),
            });
        }
        Ok(outcome)
    }

    async fn pre_drain(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        self.inner.pre_drain(ctx).await
    }

    async fn finalize(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        self.inner.finalize(ctx).await
    }

    async fn delete(&mut self, ctx: &mut ResourceContext) -> Result<(), DriverFailure> {
        self.inner.delete(ctx).await
    }
}

// ---------------------------------------------------------------------------
// The plane: manager over store over broker
// ---------------------------------------------------------------------------

struct PassthroughDecoder;

impl SpecDecoder for PassthroughDecoder {
    fn decode(
        &self,
        envelope: &[u8],
    ) -> Result<Box<dyn std::any::Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
        Ok(Box::new(envelope.to_vec()))
    }
}

struct HostOnlyResolver;

impl TargetResolver for HostOnlyResolver {
    fn execution_ref(&self, _key: &ResourceKey, _spec: &[u8]) -> Option<String> {
        None
    }
}

fn bootstrap() -> AuthoritySubject {
    AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)
}

/// The publication identity the refused Zone publishes under. It is a real
/// resource reference that the Zone's accepted graph simply holds no
/// `RoleBinding` for, which is the whole point of the negative case.
fn ungranted() -> AuthoritySubject {
    AuthoritySubject::named(
        AuthoritySubjectKind::Process,
        ResourceRef::parse("Process/ungranted-controller").expect("a canonical reference"),
    )
}

/// One Zone's plane: the store the manager owns, the broker it publishes to,
/// the scripted session port its Provider handler reads, and the journal that
/// handler writes.
struct Plane {
    client: ResourceManagerClient,
    store: Arc<SpecStore>,
    projection: Arc<AuthorityProjection>,
    link: ProjectionLink,
    sessions: Arc<SessionEvidence>,
    journal: Arc<Mutex<Vec<HandlerPass>>>,
    _root: TempDir,
}

impl Drop for Plane {
    fn drop(&mut self) {
        self.client.actor().get_cell().stop(None);
    }
}

impl Plane {
    /// Start one Zone's plane over a shared broker projection, publishing under
    /// `subject`.
    async fn start(projection: Arc<AuthorityProjection>, zone: &str, subject: AuthoritySubject) -> Self {
        let root = tempfile::tempdir().expect("a scratch state root");
        let store = Arc::new(
            SpecStore::open(root.path().join("specs.sqlite")).expect("the store opens"),
        );
        let incarnation = store
            .store_incarnation()
            .await
            .expect("the store carries an incarnation");
        let link = ProjectionLink {
            projection: Arc::clone(&projection),
            refusals: Arc::new(Mutex::new(Vec::new())),
        };
        let publisher = CoordinatorPublisher::new(
            Arc::new(AuthorityPublicationCoordinator::new(
                zone,
                incarnation.clone(),
                subject.clone(),
                Arc::new(link.clone()),
            )),
            incarnation,
            subject,
        );

        let journal = Arc::new(Mutex::new(Vec::new()));
        let sessions = SessionEvidence::new();
        // The controller row's own handler is not this suite's subject; it is
        // registered so the manager can commit and own the row the compiler
        // generated, which is the row the Provider handler then reads. It
        // publishes no status, so nothing about the Provider's readiness rests
        // on it.
        let mut providers = ProviderDirectory::new();
        providers
            .register(RowOnlyFactory::new(vec![
                RuntimeTypeName::new("Process"),
                RuntimeTypeName::new("EphemeralProcess"),
            ]) as Arc<dyn ResourceDriverFactory>)
            .expect("the Process family registers");
        providers
            .register(Arc::new(ObservedProviderFactory {
                inner: Arc::new(ProviderDriverFactory::with_effects(
                    Arc::clone(&sessions) as Arc<dyn d2b_provider_provider::ProviderDriverEffects>,
                )) as Arc<dyn ResourceDriverFactory>,
                journal: Arc::clone(&journal),
            }))
            .expect("the Provider type registers");

        let mut decoders: HashMap<RuntimeTypeName, Arc<dyn SpecDecoder>> = HashMap::new();
        decoders.insert(
            RuntimeTypeName::new(d2b_provider_provider::PROVIDER_TYPE_NAME),
            d2b_provider_provider::provider_spec_decoder(),
        );

        let args = ResourceManagerArgs {
            zone: zone.to_owned(),
            store: Arc::clone(&store),
            authority: publisher,
            providers,
            hub: Arc::new(WatchHub::new(
                &d2b_resource_runtime::revision::SystemClock,
                DEFAULT_RING_CAPACITY,
            )),
            admission: Arc::new(AllowAll),
            decoders,
            default_decoder: Arc::new(PassthroughDecoder),
            targets: Arc::new(TargetDirectory::new()),
            host_target: TargetRef::host("test-host").expect("a host target"),
            target_resolver: Arc::new(HostOnlyResolver),
            backoff: Duration::from_millis(50),
            relation_extractors: d2b_resource_runtime::relations::RelationExtractors::new(),
        };
        let (actor, _join) = ractor::Actor::spawn(None, ResourceManager::new(), args)
            .await
            .expect("the manager starts over the real store and the real broker");
        Self {
            client: ResourceManagerClient::new(actor),
            store,
            projection,
            link,
            sessions,
            journal,
            _root: root,
        }
    }

    /// Every refusal the broker answered, in order.
    async fn refusals(&self) -> Vec<PublicationRefusal> {
        self.link.refusals.lock().await.clone()
    }

    /// Every pass the production Provider driver published, in order.
    async fn passes(&self) -> Vec<HandlerPass> {
        self.journal.lock().await.clone()
    }

    /// The broker's own durable posture for this Zone.
    async fn broker_state(&self, zone: &str) -> ZoneAuthorityState {
        self.projection.status(zone).await
    }

}

fn key(zone: &str, type_name: &str, name: &str) -> ResourceKey {
    ResourceKey::new(zone, type_name, name)
}

fn subject() -> MutationSubject {
    MutationSubject {
        principal: "acceptance".to_owned(),
        origin: ResourceProvenance::Resource,
    }
}

fn desired(zone: &str, type_name: &str, name: &str, spec: &[u8]) -> DesiredResource {
    DesiredResource {
        key: key(zone, type_name, name),
        spec: spec.to_vec(),
        metadata: METADATA.to_vec(),
        provenance: ResourceProvenance::Resource,
    }
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


/// Wait, bounded, until the Provider handler has published a pass that resolved
/// the controller session.
async fn until_resolved(journal: &Arc<Mutex<Vec<HandlerPass>>>, key: &ResourceKey) -> ProviderDriverStatus {
    for _ in 0..1_200 {
        let passes = journal.lock().await;
        if let Some(status) = passes
            .iter()
            .rev()
            .find(|pass| pass.key == *key)
            .map(|pass| &pass.status)
            .filter(|status| status.observation.conformance_valid)
        {
            return status.clone();
        }
        drop(passes);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let passes = journal.lock().await;
    let observed = passes
        .iter()
        .filter(|pass| pass.key == *key)
        .map(|pass| format!("{:?}", pass.status.observation))
        .collect::<Vec<_>>();
    panic!("the Provider handler never resolved the controller session: {observed:?}");
}

// ---------------------------------------------------------------------------
// The chain
// ---------------------------------------------------------------------------

/// One declaration, carried from the compiler to a handler invocation.
///
/// The declaration is a Provider whose signed manifest names a launchable
/// controller. The compiler projects that controller's `Process` row and its
/// private template binding; the manager commits both under the real
/// publication order; the broker fences the candidate and accepts the commit;
/// and the production `Provider` driver reconciles the row and publishes what
/// it observed.
///
/// The assertion is the far end's. The handler's own published observation has
/// to show that it read the row the COMPILER generated - as this Provider's
/// owned child, carrying the compiler's own `processClass`, `providerRef`, and
/// `executionRef`, owned by the uid the store committed - and that it resolved
/// that child's controller session against the Provider's own committed
/// identity and generation. Nothing about a message having been sent is
/// asserted, and the publication's own success is asserted separately, at the
/// broker's durable state rather than at the manager's return value.
#[tokio::test]
async fn a_declared_controller_reaches_its_handler_through_the_broker() {
    let root = tempfile::tempdir().expect("a scratch broker state root");
    let projection = Arc::new(
        AuthorityProjection::open_async(root.path().to_path_buf())
            .await
            .expect("the broker projection opens"),
    );
    let declared = declaration(ZONE);
    let plane = Plane::start(Arc::clone(&projection), ZONE, bootstrap()).await;

    let provider_ref = ResourceRef::parse(&format!("Provider/{ARTIFACT}")).expect("a canonical ref");
    let provider_key = key(ZONE, "Provider", ARTIFACT);
    let controller_name = declared.controller["metadata"]["name"]
        .as_str()
        .expect("the compiler named the controller row")
        .to_owned();
    let controller_ref = ResourceRef::parse(&format!("Process/{controller_name}"))
        .expect("a canonical controller reference");
    let controller_key = key(ZONE, "Process", &controller_name);

    // The private binding the compiler emitted names exactly this row, this
    // declaring Provider, and the Host the declaration executed against.
    assert_eq!(
        declared.template.process_ref(),
        &controller_ref,
        "the compiler's template binding names the row it projected"
    );
    assert_eq!(
        declared.template.owner_ref(),
        &provider_ref,
        "the compiler's template binding is owned by the declaring Provider"
    );
    assert_eq!(
        declared.template.execution_ref(),
        &ResourceRef::parse(&format!("Host/{HOST}")).expect("a canonical Host reference"),
        "the compiler's template binding executes against the declared Host"
    );
    assert_eq!(
        declared.template.binary_ref().as_str(),
        COMPONENT,
        "the compiler's template binding pins the declared executable"
    );

    plane
        .client
        .ensure(
            subject(),
            None,
            desired(ZONE, "Provider", ARTIFACT, &declared.provider_spec),
        )
        .await
        .expect("the declared Provider commits through the broker");
    plane
        .client
        .ensure(
            subject(),
            Some(provider_key.clone()),
            desired(ZONE, "Process", &controller_name, &canonical(&declared.controller["spec"])),
        )
        .await
        .expect("the compiler's controller row commits as the Provider's own child");

    // The broker's own durable answer: the Zone is serving again at the cursor
    // the two commits moved it to, and nothing was refused on the way.
    let status = plane.broker_state(ZONE).await;
    assert!(
        !status.is_fenced(),
        "the broker left the Zone {} after accepting the candidates",
        posture(&status)
    );
    assert_eq!(
        status
            .accepted()
            .map(|cursor| cursor.sequence.get()),
        Some(2),
        "the Provider row and the compiler's controller row are two accepted commits"
    );
    assert!(
        plane.refusals().await.is_empty(),
        "the broker refused nothing on the admitted path"
    );

    let provider_row = plane
        .client
        .get_row(provider_key.clone())
        .await
        .expect("the row read answers")
        .expect("the Provider row is committed");
    let controller_row = plane
        .client
        .get_row(controller_key.clone())
        .await
        .expect("the row read answers")
        .expect("the controller row is committed");
    assert_eq!(
        controller_row.owner_uid,
        Some(provider_row.uid),
        "the compiler's controller row is owned by the Provider's committed uid"
    );
    assert_eq!(
        controller_row.uid,
        d2b_resource_runtime::manager::deterministic_uid(&controller_key),
        "the controller row carries the uid the manager derived from the compiler's row name"
    );

    // The live controller-session seam the Provider handler reads. It is
    // scripted over the row identity the store committed, so the handler can
    // only resolve it by having read that exact row.
    plane
        .sessions
        .admit(
            controller_ref.clone(),
            ResourceUid::from_bytes(&controller_row.uid).expect("a canonical uid"),
            controller_row.generation,
            serde_json::json!({
                "ready": true,
                "providerRef": provider_ref.to_canonical_string(),
                "providerUid": ResourceUid::from_bytes(&provider_row.uid)
                    .expect("a canonical uid")
                    .as_str(),
                "providerGeneration": provider_row.generation,
                "controllerGeneration": 1,
                "sessionGeneration": 1,
                "artifactReady": true,
                "descriptorReady": true,
                "registrationReady": true,
            }),
        )
        .await;

    let observed = until_resolved(&plane.journal, &provider_key).await;
    let observation = observed.observation;
    assert_eq!(
        observed.observed_generation,
        provider_row.generation,
        "the handler's observation is the generation the manager committed"
    );
    assert!(
        observation.package_present,
        "the handler found the declared artifact id on the row it read"
    );
    assert!(
        observation.config_valid,
        "the handler found the declaration's own config object"
    );
    assert!(
        observation.graph_valid,
        "the handler read the compiler's controller row as this Provider's owned \
         child, with the compiler's processClass and providerRef"
    );
    assert!(
        observation.conformance_valid,
        "the handler resolved the controller session for the row the compiler \
         generated, at the uid and generation the store committed"
    );
    assert!(
        !observation.components_drained,
        "a Provider that still owns the compiler's controller row is not drained"
    );
    // The far end's own verdict. The Provider row converges to Ready only
    // because the handler resolved its compiler-generated controller child and
    // that child's admitted session; remove either and the same production
    // policy leaves the row pending.
    assert_eq!(
        observed.phase,
        d2b_provider_provider::providers::ProviderPhase::Ready,
        "a Provider whose declared controller row is committed, owned, and admitted converges"
    );
}

// ---------------------------------------------------------------------------
// The negative
// ---------------------------------------------------------------------------

/// The same chain, stopped by the broker.
///
/// The publication identity is a real controller reference, and the Zone's
/// accepted graph carries no `RoleBinding` for it, so the broker refuses the
/// candidate at authorization. The manager cannot commit, so the row never
/// exists, so there is no actor and the handler never runs. The refusal code,
/// stage, reason, and fence flag are the broker's own typed refusal.
#[tokio::test]
async fn a_declaration_the_broker_refuses_never_reaches_the_handler() {
    let root = tempfile::tempdir().expect("a scratch broker state root");
    let projection = Arc::new(
        AuthorityProjection::open_async(root.path().to_path_buf())
            .await
            .expect("the broker projection opens"),
    );
    let declared = declaration(REFUSED_ZONE);
    let plane = Plane::start(Arc::clone(&projection), REFUSED_ZONE, ungranted()).await;
    let provider_key = key(REFUSED_ZONE, "Provider", ARTIFACT);

    let reported = plane
        .client
        .ensure(
            subject(),
            None,
            desired(REFUSED_ZONE, "Provider", ARTIFACT, &declared.provider_spec),
        )
        .await
        .expect_err("the broker refuses a candidate the accepted graph does not authorize")
        .to_string();
    assert!(
        reported.contains(PUBLICATION_CONTROL_NOT_BOUND),
        "the manager reported the broker's own refusal code: {reported}"
    );

    let refusals = plane.refusals().await;
    assert_eq!(refusals.len(), 1, "exactly one candidate was refused");
    let refusal = &refusals[0];
    assert_eq!(refusal.code, PUBLICATION_CONTROL_NOT_BOUND, "refusal code");
    assert_eq!(refusal.stage, AdmissionStage::Authorize, "refusal stage");
    assert_eq!(
        refusal.reason,
        RefusalReason::IdentityNotAuthorized,
        "the prior accepted graph holds no grant for this identity"
    );
    assert!(refusal.fenced, "a refused candidate leaves the Zone fenced");

    let status = plane.broker_state(REFUSED_ZONE).await;
    assert!(
        status.is_fenced(),
        "the broker left the Zone {} after the refusal",
        posture(&status)
    );
    assert!(
        status
            .accepted()
            .is_none_or(|cursor| cursor.sequence.get() == 0),
        "the refused candidate moved no accepted cursor"
    );
    assert!(
        plane
            .client
            .get_row(provider_key.clone())
            .await
            .expect("the row read answers")
            .is_none(),
        "a refused candidate commits no desired row"
    );
    assert!(
        plane
            .store
            .list(d2b_resource_runtime::spec_store::SpecSelector {
                zone: Some(REFUSED_ZONE.to_owned()),
                type_name: None,
                owner_uid: None,
            })
            .await
            .expect("the store lists")
            .is_empty(),
        "a refused candidate commits no desired row in the store at all"
    );
    assert!(
        plane.passes().await.is_empty(),
        "the handler never ran: no Provider row was ever committed"
    );
}
