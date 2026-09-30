//! The declaration-derived graph projection (KTD1, U4).
//!
//! Packaging and configuration used to consume three authored views of one
//! provider: a signed manifest, a per-crate registration table, and a Nix
//! inventory. Nothing derived them from each other, so a method could be
//! declared and unregistered, and a provider whose presentation needed a
//! capability only a role name hinted at compiled anyway.
//!
//! This module derives every one of those surfaces from the single
//! serializable declaration [`super::ProviderDeclarationSpec`], plus the typed
//! binding request the consumer's own spec carries. It is a pure function of
//! its arguments: it takes declarations, build-output facts, and consumer
//! requests, and it takes no path to any file. The retired broker-operation
//! merge documents, the handwritten privilege and role-scope tables, and the
//! declaration-free Nix inventory rows are therefore not inputs here at all,
//! and no projection can be re-derived from them.
//!
//! Executable digests arrive from the build output through [`BuiltArtifact`],
//! never from the declaration, and signing stays in the packaging boundary:
//! there is no field here for a signing key, so no provider's own source can
//! carry one.
//!
//! The projection lives beside the declaration rather than in a generator
//! because the declaration's wire format is the thing being projected. Every
//! consumer - the compiler, the packaging generator, and the Nix inventory
//! generator - reads this one type, so packaging and configuration cannot
//! drift into two sources.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::{Deserialize, Serialize};

use d2b_contracts_resource::v3::{
    ArtifactId, BindingContractError, BindingKind, BindingRealizationFacet, BindingSlotDecision,
    BindingSlotIndex, BindingSpecFingerprint, CanonicalJsonValue, OperationImplementation,
    PlacementAnchor, ResourceTypeName, ResourceUid, VolumeBindingRequest, VolumePresentation,
    ZoneId, canonical_digest, canonical_json_bytes,
};

use super::provider::{
    ArtifactDigest, ComponentType, ControllerTargetKind, PresentationCapability,
    ProviderContractError, ProviderDeclarationSpec, SetupRestriction,
};

/// The wire spelling of a closed contract enum.
///
/// Every projection here speaks the wire vocabulary its contract already
/// publishes, so a reader of a generated row never has to know that the enum's
/// Rust spelling is PascalCase.
trait Wire {
    /// The closed set's kebab-case wire token.
    fn wire(&self) -> &'static str;
}

impl Wire for PresentationCapability {
    fn wire(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::FilesystemPresentation => "filesystem-presentation",
            Self::NamespaceFirstServiceSource => "namespace-first-service-source",
        }
    }
}

impl Wire for SetupRestriction {
    fn wire(&self) -> &'static str {
        match self {
            Self::MountTreeBeforeUserNamespace => "mount-tree-before-user-namespace",
            Self::SteadyStateMountNamespace => "steady-state-mount-namespace",
            Self::ZeroHostCapability => "zero-host-capability",
        }
    }
}

impl Wire for BindingKind {
    fn wire(&self) -> &'static str {
        match self {
            Self::Volume => "volume",
            Self::Device => "device",
            Self::Network => "network",
            Self::Endpoint => "endpoint",
            Self::Credential => "credential",
        }
    }
}

impl Wire for ComponentType {
    fn wire(&self) -> &'static str {
        match self {
            Self::Controller => "controller",
            Self::Service => "service",
            Self::Worker => "worker",
        }
    }
}

impl Wire for ControllerTargetKind {
    fn wire(&self) -> &'static str {
        match self {
            Self::Zone => "zone",
            Self::Host => "host",
            Self::Guest => "guest",
        }
    }
}

impl Wire for PlacementAnchor {
    fn wire(&self) -> &'static str {
        match self {
            Self::Zone => "zone",
            Self::ExecutionRef => "execution-ref",
        }
    }
}

impl Wire for BindingRealizationFacet {
    fn wire(&self) -> &'static str {
        match self {
            Self::FilesystemPresentation => "filesystem-presentation",
            Self::ConsumerDeviceSlot => "consumer-device-slot",
            Self::DeviceAttachment => "device-attachment",
            Self::NamespaceInterface => "namespace-interface",
            Self::SharedFabric => "shared-fabric",
            Self::EndpointDescriptor => "endpoint-descriptor",
            Self::EndpointPathname => "endpoint-pathname",
            Self::CredentialDelivery => "credential-delivery",
        }
    }
}

/// The contract version this projection compiles.
///
/// A declaration, a built artifact, or a configuration input that names any
/// other version is refused rather than downgraded: the release is a clean
/// break, so there is no compatibility parser to fall back to.
pub const GRAPH_PROJECTION_CONTRACT_VERSION: &str = "d2b.zone.v3";

/// The domain tag framing a provider declaration projection digest.
pub const DECLARATION_PROJECTION_DOMAIN_TAG: &str = "d2b:v3:provider-declaration";

/// Every source the graph projection reads.
///
/// The list is closed and asserted, not documentation: a source named here is
/// a source the generator is allowed to read, so a regression that
/// reintroduced one of the retired merge documents would have to declare it.
pub const GRAPH_PROJECTION_INPUTS: &[&str] = &["provider-declaration"];

/// What one build produced for one provider artifact.
///
/// The executable digests are the build output's own, the configuration
/// schema is the file the package installed, and the contract version is the
/// one the build emitted. Nothing here is authored by the provider's
/// declaration, and there is deliberately no signing-key field: verification
/// stays in the packaging boundary that signs the exact canonical manifest
/// bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct BuiltArtifact {
    artifact_id: ArtifactId,
    contract_version: String,
    executable_set_digest: ArtifactDigest,
    declared_executable_set_digest: ArtifactDigest,
    config_schema: Vec<u8>,
}

impl BuiltArtifact {
    /// Record one build's output facts for one artifact.
    ///
    /// `executable_set_digest` is the D101 digest the build output's own
    /// executable map hashes to, and `declared_executable_set_digest` is the
    /// digest the verified, signed manifest pins for the same set. The caller
    /// computes the first with its own build boundary rather than reading it
    /// from a declaration: the declaration never carries it.
    pub fn new(
        artifact_id: ArtifactId,
        contract_version: impl Into<String>,
        executable_set_digest: ArtifactDigest,
        declared_executable_set_digest: ArtifactDigest,
        config_schema: Vec<u8>,
    ) -> Self {
        Self {
            artifact_id,
            contract_version: contract_version.into(),
            executable_set_digest,
            declared_executable_set_digest,
            config_schema,
        }
    }

    /// The artifact this build output belongs to.
    pub const fn artifact_id(&self) -> &ArtifactId {
        &self.artifact_id
    }

    /// The contract version the build emitted.
    pub fn contract_version(&self) -> &str {
        &self.contract_version
    }

    /// The executable-set digest the build output hashes to.
    pub const fn executable_set_digest(&self) -> &ArtifactDigest {
        &self.executable_set_digest
    }

    /// The executable-set digest the verified, signed manifest pins.
    pub const fn declared_executable_set_digest(&self) -> &ArtifactDigest {
        &self.declared_executable_set_digest
    }

    /// The digest of the configuration schema the package installed.
    pub fn config_digest(&self) -> ArtifactDigest {
        raw_sha256_digest(&self.config_schema)
    }
}

/// The canonical re-rendering of a schema document, or `None` when the bytes
/// are not a well-formed canonical JSON value at all.
///
/// The projection and the compiler's artifact check both need the same answer,
/// so one parse and one render live here rather than two that could drift on
/// what "canonical" means.
pub fn canonical_schema_bytes(bytes: &[u8]) -> Option<Vec<u8>> {
    let value = CanonicalJsonValue::parse(bytes).ok()?;
    canonical_json_bytes(&value).ok()
}

/// A raw SHA-256 artifact digest in the contract spelling.
fn raw_sha256_digest(bytes: &[u8]) -> ArtifactDigest {
    use sha2::{Digest, Sha256};
    ArtifactDigest::parse(format!("sha256:{}", encode_hex(&Sha256::digest(bytes))))
        .expect("a raw SHA-256 digest is always a canonical digest")
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

/// One canonical consumer request, as a configuration author's shorthand
/// produces it (KTD2).
///
/// The request itself is the canonical declaration: the configuration
/// shorthand compiles into it before admission rather than persisting a
/// second relationship list. The Zone and the two store-assigned identities
/// come from the committed rows, so the projection derives the exact KTD3 key
/// and the exact desired-bytes digest the source controller admits.
#[derive(Clone, PartialEq, Eq)]
pub struct ConsumerRequestInput {
    zone: ZoneId,
    source_uid: ResourceUid,
    consumer_uid: ResourceUid,
    request: VolumeBindingRequest,
}

impl ConsumerRequestInput {
    /// Bind one canonical request to the committed identities it is keyed by.
    pub const fn new(
        zone: ZoneId,
        source_uid: ResourceUid,
        consumer_uid: ResourceUid,
        request: VolumeBindingRequest,
    ) -> Self {
        Self {
            zone,
            source_uid,
            consumer_uid,
            request,
        }
    }

    /// The canonical request this input carries.
    pub const fn request(&self) -> &VolumeBindingRequest {
        &self.request
    }
}

impl fmt::Debug for ConsumerRequestInput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConsumerRequestInput")
            .field("consumer_ref", &self.request.consumer_ref().to_canonical_string())
            .field("slot", &self.request.slot().as_str())
            .finish_non_exhaustive()
    }
}

/// Why a declaration-derived graph projection is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphProjectionError {
    /// The build output names a contract version this release does not
    /// compile.
    ContractVersionRetired {
        /// The artifact whose version is retired.
        artifact_id: String,
        /// The version the input named.
        declared: String,
    },
    /// The build output's executable set does not hash to the digest the
    /// declaration's signed code identity pins.
    ExecutableDigestMismatch {
        /// The artifact whose executable set disagrees.
        artifact_id: String,
        /// The digest the declaration pins.
        declared: String,
        /// The digest the build output hashes to.
        observed: String,
    },
    /// The configuration schema the build installed is not the canonical
    /// schema the declaration digests.
    SchemaMalformed {
        /// The artifact whose schema is malformed.
        artifact_id: String,
        /// The component whose `configDigest` the schema must match.
        component: String,
        /// Why the schema is refused.
        reason: &'static str,
    },
    /// Two different declarations occupy one consumer slot.
    ConsumerSlotDuplicate {
        /// The consumer whose slot is claimed twice.
        consumer: String,
        /// The binding kind of the slot.
        kind: &'static str,
        /// The stable consumer slot.
        slot: String,
    },
    /// A canonical request could not derive its relationship key.
    ConsumerRequestMalformed {
        /// The consumer the refused request names.
        consumer: String,
        /// Why the request is refused.
        reason: BindingContractError,
    },
    /// No build output covers a declared artifact.
    BuildOutputMissing {
        /// The artifact with no build output.
        artifact_id: String,
    },
    /// The declaration could not be rendered as its canonical projection.
    DeclarationNotCanonical {
        /// The artifact whose declaration is not canonicalizable.
        artifact_id: String,
    },
    /// A declared identity could not be projected.
    DeclarationIncomplete {
        /// The artifact whose declaration is incomplete.
        artifact_id: String,
        /// The identity the declaration could not produce.
        reason: ProviderContractError,
    },
}

impl GraphProjectionError {
    /// A stable diagnostic code naming the specific failure.
    pub const fn code(&self) -> &'static str {
        match self {
            Self::ContractVersionRetired { .. } => "provider-graph-contract-version-retired",
            Self::ExecutableDigestMismatch { .. } => "provider-graph-executable-digest-mismatch",
            Self::SchemaMalformed { .. } => "provider-graph-schema-malformed",
            Self::ConsumerSlotDuplicate { .. } => "provider-graph-consumer-slot-duplicate",
            Self::ConsumerRequestMalformed { .. } => "provider-graph-consumer-request-malformed",
            Self::BuildOutputMissing { .. } => "provider-graph-build-output-missing",
            Self::DeclarationNotCanonical { .. } => "provider-graph-declaration-not-canonical",
            Self::DeclarationIncomplete { .. } => "provider-graph-declaration-incomplete",
        }
    }
}

impl fmt::Display for GraphProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContractVersionRetired { artifact_id, declared } => write!(
                formatter,
                "{}: artifact {artifact_id} declares contract version {declared}",
                self.code()
            ),
            Self::ExecutableDigestMismatch {
                artifact_id,
                declared,
                observed,
            } => write!(
                formatter,
                "{}: artifact {artifact_id} executable set is {observed}, declaration pins {declared}",
                self.code()
            ),
            Self::SchemaMalformed {
                artifact_id,
                component,
                reason,
            } => write!(
                formatter,
                "{}: artifact {artifact_id} component {component} schema is {reason}",
                self.code()
            ),
            Self::ConsumerSlotDuplicate {
                consumer,
                kind,
                slot,
            } => write!(
                formatter,
                "{}: consumer {consumer} claims {kind} slot {slot} twice",
                self.code()
            ),
            Self::ConsumerRequestMalformed { consumer, reason } => write!(
                formatter,
                "{}: consumer {consumer} request is malformed: {reason}",
                self.code()
            ),
            Self::BuildOutputMissing { artifact_id } => write!(
                formatter,
                "{}: no build output covers artifact {artifact_id}",
                self.code()
            ),
            Self::DeclarationNotCanonical { artifact_id } => write!(
                formatter,
                "{}: artifact {artifact_id} declaration is not canonicalizable",
                self.code()
            ),
            Self::DeclarationIncomplete { artifact_id, reason } => write!(
                formatter,
                "{}: artifact {artifact_id} declaration is incomplete: {reason}",
                self.code()
            ),
        }
    }
}


/// One component's canonical manifest input.
///
/// The row is the packaging manifest's own view of one declared component: its
/// signed code identity, where it may be placed, what presentation it can
/// realize, and the setup restrictions that presentation requires. The
/// presentation and the restrictions are carried because the manifest is the
/// signed surface the deployment admits; a downstream projection that had to
/// re-derive them from a role name or a seccomp label would be inferring a
/// capability the declaration already states.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestInputComponent {
    component_id: String,
    component_type: String,
    exported_resource_types: Vec<String>,
    exported_methods: Vec<String>,
    config_digest: String,
    presentation_capability: PresentationCapability,
    setup_restrictions: Vec<SetupRestriction>,
    placement_targets: Vec<String>,
    placement_anchor: Option<String>,
}

impl ManifestInputComponent {
    /// The declared component identity.
    pub fn component_id(&self) -> &str {
        &self.component_id
    }

    /// The presentation this component realizes, as declared.
    pub const fn presentation_capability(&self) -> PresentationCapability {
        self.presentation_capability
    }

    /// The setup restrictions the declared presentation requires.
    pub fn setup_restrictions(&self) -> &[SetupRestriction] {
        &self.setup_restrictions
    }

    /// The presentation capability in the contract's own wire vocabulary.
    pub fn presentation_token(&self) -> &'static str {
        self.presentation_capability.wire()
    }

    /// The setup restrictions in the contract's own wire vocabulary.
    pub fn setup_restriction_tokens(&self) -> Vec<&'static str> {
        self.setup_restrictions.iter().map(|value| value.wire()).collect()
    }

    /// The ResourceTypes the component exports.
    pub fn exported_resource_types(&self) -> &[String] {
        &self.exported_resource_types
    }

    /// The methods the component exports.
    pub fn exported_methods(&self) -> &[String] {
        &self.exported_methods
    }
}

/// One resource capability a declaration requires, as a manifest input row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RequiredCapabilityInput {
    resource_type: String,
    capability: String,
}

/// One provider's canonical manifest input.
///
/// `declaration_bytes` is exactly what U3's `emit_declaration_canonical`
/// produces for this declaration - the same
/// `d2b_contracts_resource::v3::canonical_json_bytes` projection - so the
/// bytes a provider signs and the bytes a generator consumes cannot differ.
/// `declaration_digest` frames those bytes so a manifest input can be compared
/// and cached without rehashing the whole document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ManifestInput {
    contract_version: String,
    artifact_id: String,
    provider_ref: String,
    declaration_digest: String,
    declaration_bytes: Vec<u8>,
    executable_set_digest: String,
    config_digest: String,
    components: Vec<ManifestInputComponent>,
    required_capabilities: Vec<RequiredCapabilityInput>,
}

impl ManifestInput {
    /// The artifact this manifest input describes.
    pub fn artifact_id(&self) -> &str {
        &self.artifact_id
    }

    /// The `Provider/...` reference this manifest input describes.
    pub fn provider_ref(&self) -> &str {
        &self.provider_ref
    }

    /// The framed digest of the canonical declaration bytes.
    pub fn declaration_digest(&self) -> &str {
        &self.declaration_digest
    }

    /// The canonical declaration bytes, byte-identical to the projection a
    /// provider signs.
    pub fn declaration_bytes(&self) -> &[u8] {
        &self.declaration_bytes
    }

    /// The build-produced executable-set digest the manifest must carry.
    pub fn executable_set_digest(&self) -> &str {
        &self.executable_set_digest
    }

    /// The digest of the canonical configuration schema the build installed.
    pub fn config_digest(&self) -> &str {
        &self.config_digest
    }

    /// The per-component manifest input rows, in declaration order.
    pub fn components(&self) -> &[ManifestInputComponent] {
        &self.components
    }
}

/// One resource-and-operation graph row.
///
/// The row is the join of the two things a method is reachable through: the
/// ResourceTypes its component exports, and the trusted implementation
/// identity the declaration derives for it. Nothing about the row is
/// authored - a provider that adds a method adds one row here, and a shared
/// family list is not a second place to edit.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct OperationGraphRow {
    artifact_id: String,
    provider_ref: String,
    component_id: String,
    method: String,
    implementation: OperationImplementation,
    resource_types: Vec<String>,
    presentation_capability: PresentationCapability,
    setup_restrictions: Vec<SetupRestriction>,
}

impl OperationGraphRow {
    /// The declared method identity.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// The declared component identity.
    pub fn component_id(&self) -> &str {
        &self.component_id
    }

    /// The trusted implementation identity the declaration derives.
    pub const fn implementation(&self) -> &OperationImplementation {
        &self.implementation
    }

    /// The ResourceTypes the method is reachable from.
    pub fn resource_types(&self) -> &[String] {
        &self.resource_types
    }

    /// The presentation the method needs, as the declaration states it.
    pub const fn presentation_capability(&self) -> PresentationCapability {
        self.presentation_capability
    }

    /// The presentation the method needs, in the contract's wire vocabulary.
    pub fn presentation_token(&self) -> &'static str {
        self.presentation_capability.wire()
    }
}

/// One provider registration row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderRegistrationRow {
    artifact_id: String,
    provider_ref: String,
    components: Vec<String>,
    resource_types: Vec<String>,
    services: Vec<String>,
    methods: Vec<String>,
}

impl ProviderRegistrationRow {
    /// The artifact this registration names.
    pub fn artifact_id(&self) -> &str {
        &self.artifact_id
    }

    /// The `Provider/...` reference this registration names.
    pub fn provider_ref(&self) -> &str {
        &self.provider_ref
    }

    /// The registered service identities, sorted.
    pub fn services(&self) -> &[String] {
        &self.services
    }

    /// The registered method identities, sorted.
    pub fn methods(&self) -> &[String] {
        &self.methods
    }

    /// The registered component identities, in declaration order.
    pub fn components(&self) -> &[String] {
        &self.components
    }

    /// The registered ResourceTypes, sorted.
    pub fn resource_types(&self) -> &[String] {
        &self.resource_types
    }
}

/// One service catalog row: the provider that answers a service identity.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ServiceCatalogRow {
    service_id: String,
    artifact_id: String,
    provider_ref: String,
    component_id: String,
    methods: Vec<String>,
}

impl ServiceCatalogRow {
    /// The service identity this row routes.
    pub fn service_id(&self) -> &str {
        &self.service_id
    }

    /// The provider that answers it.
    pub fn provider_ref(&self) -> &str {
        &self.provider_ref
    }

    /// The declared methods the service answers.
    pub fn methods(&self) -> &[String] {
        &self.methods
    }
}

/// The consumer-slot decision, in its serializable spelling.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BindingSlotDecisionWire {
    /// The slot was free and this request now occupies it.
    Claimed,
    /// An identical request already occupies the slot, so the two coalesce.
    Coalesced,
    /// The previous occupant released and this request is its successor.
    SuccessorClaimed,
    /// The same relationship's payload changed while its old use was closed.
    PayloadUpdated,
}

impl From<BindingSlotDecision> for BindingSlotDecisionWire {
    fn from(decision: BindingSlotDecision) -> Self {
        match decision {
            BindingSlotDecision::Claimed => Self::Claimed,
            BindingSlotDecision::Coalesced => Self::Coalesced,
            BindingSlotDecision::SuccessorClaimed => Self::SuccessorClaimed,
            BindingSlotDecision::PayloadUpdated => Self::PayloadUpdated,
        }
    }
}

/// One compiled canonical consumer request row (KTD2, KTD3).
///
/// The row is the compiled form of one configuration shorthand: the exact
/// relationship key, the exact desired-bytes digest, and the realization
/// facets the consumer's presentation depends on. The compiler derives it
/// from the request; the source controller admits it; nothing persists a
/// second relationship list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConsumerRequestRow {
    zone: String,
    kind: BindingKind,
    source_ref: String,
    source_uid: String,
    consumer_ref: String,
    consumer_uid: String,
    slot: String,
    fingerprint: String,
    decision: BindingSlotDecisionWire,
    required_facets: Vec<String>,
    presentation: VolumePresentation,
}

impl ConsumerRequestRow {
    /// The stable consumer slot this request occupies.
    pub fn slot(&self) -> &str {
        &self.slot
    }

    /// The canonical consumer the request belongs to.
    pub fn consumer_ref(&self) -> &str {
        &self.consumer_ref
    }

    /// The Zone the relationship belongs to.
    pub fn zone(&self) -> &str {
        &self.zone
    }

    /// The binding kind of the relationship.
    pub const fn kind(&self) -> BindingKind {
        self.kind
    }

    /// The binding kind in the contract's own wire vocabulary.
    pub fn kind_token(&self) -> &'static str {
        self.kind.wire()
    }

    /// The framed digest of the exact desired request bytes.
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }

    /// How the consumer slot index resolved this request.
    pub const fn decision(&self) -> BindingSlotDecisionWire {
        self.decision
    }

    /// The realization facets the consumer's presentation depends on.
    pub fn required_facets(&self) -> &[String] {
        &self.required_facets
    }
}

/// The private plan projection: the derived view the compiler emits beside a
/// compiled bundle.
///
/// The plan is an output, not a source. It carries the presentation
/// capability each method needs, exactly as the declaration stated it, plus
/// the compiled consumer requests, so the private plan cannot reintroduce a
/// role-name, seccomp-label, or serving-worker-role inference: there is no
/// such field to read and the value came from the signed manifest input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivatePlanProjection {
    contract_version: String,
    manifest_inputs: Vec<ManifestInput>,
    operations: Vec<OperationGraphRow>,
    registrations: Vec<ProviderRegistrationRow>,
    services: Vec<ServiceCatalogRow>,
    consumer_requests: Vec<ConsumerRequestRow>,
}

impl PrivatePlanProjection {
    /// The exact canonical bytes of this private plan.
    ///
    /// # Errors
    ///
    /// Returns [`GraphProjectionError::DeclarationNotCanonical`] when the
    /// plan cannot be rendered as canonical bytes, which a well-formed plan
    /// always can; the failure is reported rather than defaulted to a
    /// non-canonical rendering.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, GraphProjectionError> {
        canonical_json_bytes(self).map_err(|_| GraphProjectionError::DeclarationNotCanonical {
            artifact_id: self.contract_version.clone(),
        })
    }

    /// The framed digest of the private plan's exact bytes.
    ///
    /// # Errors
    ///
    /// Propagates [`PrivatePlanProjection::canonical_bytes`].
    pub fn digest(&self) -> Result<String, GraphProjectionError> {
        Ok(canonical_digest(
            PRIVATE_PLAN_DOMAIN_TAG,
            &self.canonical_bytes()?,
        ))
    }

    /// The plan's contract version.
    pub fn contract_version(&self) -> &str {
        &self.contract_version
    }

    /// The per-method graph rows, in sorted projection order.
    pub fn operations(&self) -> &[OperationGraphRow] {
        &self.operations
    }

    /// The compiled canonical consumer requests.
    pub fn consumer_requests(&self) -> &[ConsumerRequestRow] {
        &self.consumer_requests
    }

    /// The provider registration rows.
    pub fn registrations(&self) -> &[ProviderRegistrationRow] {
        &self.registrations
    }

    /// The service catalog rows.
    pub fn services(&self) -> &[ServiceCatalogRow] {
        &self.services
    }

    /// The canonical manifest inputs this plan was derived from.
    pub fn manifest_inputs(&self) -> &[ManifestInput] {
        &self.manifest_inputs
    }
}

/// The domain tag framing a private plan projection digest.
pub const PRIVATE_PLAN_DOMAIN_TAG: &str = "d2b:v3:private-plan";

/// Project one set of provider declarations into every derived surface.
///
/// The projection reads nothing but its arguments. There is no path parameter
/// and no file access, so the retired broker-operation merge documents, the
/// handwritten privilege and role-scope tables, and the declaration-free Nix
/// inventory rows are not inputs and cannot be reintroduced as inputs without
/// changing this signature.
///
/// # Errors
///
/// Refuses, with the specific failure named:
///
/// - [`GraphProjectionError::ContractVersionRetired`] when a build output
///   names a contract version this release does not compile.
/// - [`GraphProjectionError::ExecutableDigestMismatch`] when the build's
///   executable set does not hash to the digest the declaration pins.
/// - [`GraphProjectionError::SchemaMalformed`] when the build's configuration
///   schema is not the canonical schema a component's `configDigest` names.
/// - [`GraphProjectionError::ConsumerSlotDuplicate`] when two different
///   declarations claim one consumer slot.
/// - [`GraphProjectionError::BuildOutputMissing`] when a declared artifact has
///   no build output to admit.
pub fn project_provider_graph(
    declarations: &[&ProviderDeclarationSpec],
    built: &[BuiltArtifact],
    requests: &[ConsumerRequestInput],
) -> Result<PrivatePlanProjection, GraphProjectionError> {
    let mut built_by_id: BTreeMap<&str, &BuiltArtifact> = BTreeMap::new();
    for artifact in built {
        built_by_id.insert(artifact.artifact_id.as_str(), artifact);
    }

    let mut manifest_inputs = Vec::new();
    let mut operations = Vec::new();
    let mut registrations = Vec::new();
    let mut services = Vec::new();

    for declaration in declarations {
        let artifact_id = declaration.artifact_id().as_str();
        let built_artifact = built_by_id.get(artifact_id).ok_or_else(|| {
            GraphProjectionError::BuildOutputMissing {
                artifact_id: artifact_id.to_owned(),
            }
        })?;
        if built_artifact.contract_version != GRAPH_PROJECTION_CONTRACT_VERSION {
            return Err(GraphProjectionError::ContractVersionRetired {
                artifact_id: artifact_id.to_owned(),
                declared: built_artifact.contract_version.clone(),
            });
        }

        let declaration_bytes = canonical_json_bytes(*declaration).map_err(|_| {
            GraphProjectionError::DeclarationNotCanonical {
                artifact_id: artifact_id.to_owned(),
            }
        })?;
        let observed_executable_set = built_artifact.executable_set_digest().clone();
        if observed_executable_set != *built_artifact.declared_executable_set_digest() {
            return Err(GraphProjectionError::ExecutableDigestMismatch {
                artifact_id: artifact_id.to_owned(),
                declared: built_artifact
                    .declared_executable_set_digest()
                    .as_str()
                    .to_owned(),
                observed: observed_executable_set.as_str().to_owned(),
            });
        }
        let config_digest = built_artifact.config_digest();
        let first_component = declaration
            .components()
            .first()
            .map_or_else(String::new, |component| {
                component.component().component_id().as_str().to_owned()
            });
        match canonical_schema_bytes(&built_artifact.config_schema) {
            None => {
                return Err(GraphProjectionError::SchemaMalformed {
                    artifact_id: artifact_id.to_owned(),
                    component: first_component,
                    reason: "not-valid-canonical-json",
                });
            }
            Some(canonical) if canonical != built_artifact.config_schema => {
                return Err(GraphProjectionError::SchemaMalformed {
                    artifact_id: artifact_id.to_owned(),
                    component: first_component,
                    reason: "not-canonical",
                });
            }
            Some(_) => {}
        }

        let mut input_components = Vec::new();
        let mut component_ids = Vec::new();
        let mut owned_types: BTreeSet<String> = BTreeSet::new();
        let mut declared_methods: BTreeSet<String> = BTreeSet::new();
        let mut declared_services: BTreeSet<String> = BTreeSet::new();
        for component in declaration.components() {
            let descriptor = component.component();
            let component_id = descriptor.component_id().as_str().to_owned();
            if descriptor.config_digest() != &config_digest {
                return Err(GraphProjectionError::SchemaMalformed {
                    artifact_id: artifact_id.to_owned(),
                    component: component_id,
                    reason: "config-digest-mismatch",
                });
            }
            let resource_types: Vec<String> = descriptor
                .exported_resource_types()
                .iter()
                .map(ResourceTypeName::to_canonical_string)
                .collect();
            let methods: Vec<String> = descriptor
                .exported_methods()
                .iter()
                .map(|method| method.as_str().to_owned())
                .collect();
            component_ids.push(component_id.clone());
            owned_types.extend(resource_types.iter().cloned());
            declared_methods.extend(methods.iter().cloned());
            for service in component.services() {
                declared_services.insert(service.id().as_str().to_owned());
                let service_methods: Vec<String> = service
                    .methods()
                    .iter()
                    .map(|method| method.as_str().to_owned())
                    .collect();
                services.push(ServiceCatalogRow {
                    service_id: service.id().as_str().to_owned(),
                    artifact_id: artifact_id.to_owned(),
                    provider_ref: declaration.provider().to_canonical_string(),
                    component_id: component_id.clone(),
                    methods: service_methods,
                });
            }
            for method in component.methods() {
                let implementation = component
                    .implementation(declaration.provider().clone(), method.name())
                    .map_err(|reason| GraphProjectionError::DeclarationIncomplete {
                        artifact_id: artifact_id.to_owned(),
                        reason,
                    })?;
                operations.push(OperationGraphRow {
                    artifact_id: artifact_id.to_owned(),
                    provider_ref: declaration.provider().to_canonical_string(),
                    component_id: component_id.clone(),
                    method: method.name().as_str().to_owned(),
                    implementation,
                    resource_types: resource_types.clone(),
                    presentation_capability: method.presentation(),
                    setup_restrictions: component
                        .setup_restrictions()
                        .iter()
                        .copied()
                        .collect(),
                });
            }
            input_components.push(ManifestInputComponent {
                component_id,
                component_type: descriptor.component_type().wire().to_owned(),
                exported_resource_types: resource_types,
                exported_methods: methods,
                config_digest: config_digest.as_str().to_owned(),
                presentation_capability: component.presentation(),
                setup_restrictions: component
                    .setup_restrictions()
                    .iter()
                    .copied()
                    .collect(),
                placement_targets: component
                    .placement()
                    .targets()
                    .iter()
                    .map(|target| target.wire().to_owned())
                    .collect(),
                placement_anchor: component
                    .placement()
                    .anchor()
                    .map(|anchor| anchor.wire().to_owned()),
            });
        }
        services.sort_by(|left, right| {
            (&left.service_id, &left.artifact_id).cmp(&(&right.service_id, &right.artifact_id))
        });
        services.dedup();
        operations.sort_by(|left, right| {
            (&left.artifact_id, &left.component_id, &left.method)
                .cmp(&(&right.artifact_id, &right.component_id, &right.method))
        });

        registrations.push(ProviderRegistrationRow {
            artifact_id: artifact_id.to_owned(),
            provider_ref: declaration.provider().to_canonical_string(),
            components: component_ids,
            resource_types: owned_types.iter().cloned().collect(),
            services: declared_services.iter().cloned().collect(),
            methods: declared_methods.iter().cloned().collect(),
        });
        manifest_inputs.push(ManifestInput {
            contract_version: GRAPH_PROJECTION_CONTRACT_VERSION.to_owned(),
            artifact_id: artifact_id.to_owned(),
            provider_ref: declaration.provider().to_canonical_string(),
            declaration_digest: canonical_digest(
                DECLARATION_PROJECTION_DOMAIN_TAG,
                &declaration_bytes,
            ),
            declaration_bytes,
            executable_set_digest: observed_executable_set.as_str().to_owned(),
            config_digest: config_digest.as_str().to_owned(),
            components: input_components,
            required_capabilities: declaration
                .required_capabilities()
                .iter()
                .map(|required| RequiredCapabilityInput {
                    resource_type: required.resource_type().to_canonical_string(),
                    capability: required.capability().as_str().to_owned(),
                })
                .collect(),
        });
    }

    manifest_inputs.sort_by(|left, right| left.artifact_id.cmp(&right.artifact_id));
    registrations.sort_by(|left, right| left.artifact_id.cmp(&right.artifact_id));

    let mut slot_index = BindingSlotIndex::new();
    let mut consumer_requests = Vec::new();
    for input in requests {
        let request = input.request();
        let key = request
            .key(input.zone.clone(), input.source_uid.clone(), input.consumer_uid.clone())
            .map_err(|reason| GraphProjectionError::ConsumerRequestMalformed {
                consumer: request.consumer_ref().to_canonical_string(),
                reason,
            })?;
        let fingerprint = BindingSpecFingerprint::from_request(request);
        let decision = slot_index
            .declare(&key, &fingerprint)
            .map_err(|_| GraphProjectionError::ConsumerSlotDuplicate {
                consumer: request.consumer_ref().to_canonical_string(),
                kind: request.kind().wire(),
                slot: request.slot().as_str().to_owned(),
            })?;
        consumer_requests.push(ConsumerRequestRow {
            zone: input.zone.as_str().to_owned(),
            kind: request.kind(),
            source_ref: key.source_ref().to_canonical_string(),
            source_uid: key.source_uid().as_str().to_owned(),
            consumer_ref: key.consumer_ref().to_canonical_string(),
            consumer_uid: key.consumer_uid().as_str().to_owned(),
            slot: key.slot().as_str().to_owned(),
            fingerprint: fingerprint.as_str().to_owned(),
            decision: decision.into(),
            required_facets: request
                .required_facets()
                .iter()
                .map(|facet| facet.wire().to_owned())
                .collect(),
            presentation: request.presentation().clone(),
        });
    }
    consumer_requests.sort_by(|left, right| {
        (&left.zone, &left.consumer_uid, &left.slot).cmp(&(&right.zone, &right.consumer_uid, &right.slot))
    });

    Ok(PrivatePlanProjection {
        contract_version: GRAPH_PROJECTION_CONTRACT_VERSION.to_owned(),
        manifest_inputs,
        operations,
        registrations,
        services,
        consumer_requests,
    })
}
