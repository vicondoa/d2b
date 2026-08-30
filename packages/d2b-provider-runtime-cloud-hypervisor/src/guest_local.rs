//! Authenticated Guest-local Resource API seeding.
//!
//! The Cloud Hypervisor controller owns only the desired seed set and the
//! bounded session/readiness state. Endpoint carriage, credentials, and target
//! effects remain behind the authenticated session and its effect owner.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ControllerGeneration, ResourceEnvelope, ResourceGeneration, ResourcePhase,
    ResourceRef, ResourceUid, SchemaFingerprint, ZoneId, ZoneRevision,
    activation_nixos::NIXOS_GENERATION_RESOURCE_TYPE, identity::ReconnectGeneration,
};
use d2b_session::TransportDescriptor;

use crate::{
    health::{GuestSessionEvidence, GuestSessionEvidenceBinding},
    state::{GuestGenerationSet, GuestStatusObservation, GuestStatusPhase, reduce_status},
};

/// Resource types admitted by the signed Guest seed schema.
pub const GUEST_SEED_RESOURCE_TYPES: &[&str] = &[
    "Process",
    "EphemeralProcess",
    "Endpoint",
    NIXOS_GENERATION_RESOURCE_TYPE,
];

const MAX_SEED_RESOURCES: usize = 128;
const MAX_OPERATION_ID_BYTES: usize = 128;
const SEED_DIGEST_DOMAIN: &str = "d2b-guest-local-seed-v1";

/// Failures at the Guest-local session and seed boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestLocalError {
    /// The authorized Endpoint could not be resolved.
    EndpointUnavailable,
    /// Endpoint identity or generation did not match the Guest contract.
    EndpointMismatch,
    /// The ComponentSession transport was not the enrolled Guest-local vsock.
    TransportMismatch,
    /// ComponentSession authentication or authorization failed.
    SessionAuthentication,
    /// The authenticated session carried the wrong identity or generation.
    SessionBindingMismatch,
    /// A reconnect attempted to reuse an old session generation.
    SessionGenerationStale,
    /// The current session disconnected while work was in flight.
    SessionLost,
    /// The target-local Resource API denied the operation.
    AuthorizationDenied,
    /// The seed request or response violated its bounded contract.
    SeedInvalid,
    /// A seed operation id was reused with different content.
    OperationReused,
    /// The target-local Resource API returned an invalid watch.
    WatchInvalid,
}

impl GuestLocalError {
    /// Return the stable identity-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::EndpointUnavailable => "guest-local-endpoint-unavailable",
            Self::EndpointMismatch => "guest-local-endpoint-mismatch",
            Self::TransportMismatch => "guest-local-transport-mismatch",
            Self::SessionAuthentication => "guest-local-session-authentication-failed",
            Self::SessionBindingMismatch => "guest-local-session-binding-mismatch",
            Self::SessionGenerationStale => "guest-local-session-generation-stale",
            Self::SessionLost => "guest-local-session-lost",
            Self::AuthorizationDenied => "guest-local-authorization-denied",
            Self::SeedInvalid => "guest-local-seed-invalid",
            Self::OperationReused => "guest-local-operation-reused",
            Self::WatchInvalid => "guest-local-watch-invalid",
        }
    }
}

impl fmt::Display for GuestLocalError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for GuestLocalError {}

/// Failures while validating one UID-free target-local seed Resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestLocalSeedResourceError {
    /// The ResourceType is not in the signed target-local seed allowlist.
    TypeNotApproved,
    /// The canonical ResourceRef and payload identity differ.
    IdentityMismatch,
    /// The resource owner is not the authenticated Guest.
    OwnerMismatch,
    /// A target-local execution or producer reference is not the Guest.
    RelationshipMismatch,
    /// A store-assigned UID was supplied before the target-local commit.
    UidNotAllowed,
    /// A private effect input was present in the public seed payload.
    PrivateField,
    /// The payload is not canonical JSON.
    NonCanonical,
    /// The payload is structurally malformed.
    Malformed,
}

/// One complete UID-free, name-addressed target-local Resource intent.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestLocalSeedResource {
    resource_ref: ResourceRef,
    owner_ref: ResourceRef,
    zone: ZoneId,
    canonical_json: Vec<u8>,
    digest: String,
}

impl GuestLocalSeedResource {
    /// Validate and construct one seed Resource.
    pub fn new(
        resource_ref: ResourceRef,
        owner_ref: ResourceRef,
        canonical_json: Vec<u8>,
    ) -> Result<Self, GuestLocalSeedResourceError> {
        if owner_ref.resource_type().as_str() != "Guest"
            || !is_approved_seed_type(resource_ref.resource_type().as_str())
        {
            return Err(GuestLocalSeedResourceError::TypeNotApproved);
        }
        let canonical = CanonicalJsonValue::parse(&canonical_json)
            .map_err(|_| GuestLocalSeedResourceError::NonCanonical)?
            .to_canonical_bytes();
        if canonical != canonical_json {
            return Err(GuestLocalSeedResourceError::NonCanonical);
        }
        let value: serde_json::Value = serde_json::from_slice(&canonical)
            .map_err(|_| GuestLocalSeedResourceError::Malformed)?;
        validate_seed_value(&value, &resource_ref, &owner_ref)?;
        parse_uid_free_envelope(&canonical).map_err(|_| GuestLocalSeedResourceError::Malformed)?;
        let zone = seed_payload_zone(&canonical).ok_or(GuestLocalSeedResourceError::Malformed)?;
        let digest = d2b_contracts_resource::v3::canonical_digest(SEED_DIGEST_DOMAIN, &canonical);
        Ok(Self {
            resource_ref,
            owner_ref,
            zone,
            canonical_json,
            digest,
        })
    }

    /// Borrow the exact name-addressed ResourceRef.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the authenticated Guest owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the Zone asserted by the UID-free payload.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the canonical UID-free payload.
    pub fn canonical_json(&self) -> &[u8] {
        &self.canonical_json
    }

    /// Borrow the semantic payload digest used for operation idempotency.
    pub fn digest(&self) -> &str {
        &self.digest
    }
}

impl fmt::Debug for GuestLocalSeedResource {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestLocalSeedResource")
            .field("resource_type", &self.resource_ref.resource_type())
            .field("has_owner", &true)
            .field("payload_bytes", &self.canonical_json.len())
            .field("has_digest", &true)
            .finish()
    }
}

/// One complete Guest-local seed operation.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestLocalSeedBatch {
    guest_ref: ResourceRef,
    guest_uid: ResourceUid,
    zone: ZoneId,
    descriptor_digest: SchemaFingerprint,
    operation_id: String,
    resources: Vec<GuestLocalSeedResource>,
    idempotency_key: String,
}

impl GuestLocalSeedBatch {
    /// Construct a complete bounded seed batch for one Guest incarnation.
    pub fn new(
        guest_ref: ResourceRef,
        guest_uid: ResourceUid,
        descriptor_digest: SchemaFingerprint,
        operation_id: impl Into<String>,
        mut resources: Vec<GuestLocalSeedResource>,
    ) -> Result<Self, GuestLocalError> {
        if guest_ref.resource_type().as_str() != "Guest"
            || resources.is_empty()
            || resources.len() > MAX_SEED_RESOURCES
        {
            return Err(GuestLocalError::SeedInvalid);
        }
        let operation_id = operation_id.into();
        if !valid_operation_id(&operation_id) {
            return Err(GuestLocalError::SeedInvalid);
        }
        if resources
            .iter()
            .any(|resource| resource.owner_ref() != &guest_ref)
        {
            return Err(GuestLocalError::SeedInvalid);
        }
        let zone = resources
            .first()
            .map(GuestLocalSeedResource::zone)
            .ok_or(GuestLocalError::SeedInvalid)?
            .clone();
        if resources.iter().any(|resource| resource.zone() != &zone) {
            return Err(GuestLocalError::SeedInvalid);
        }
        resources.sort_by(|left, right| left.resource_ref.cmp(&right.resource_ref));
        if resources
            .windows(2)
            .any(|pair| pair[0].resource_ref == pair[1].resource_ref)
        {
            return Err(GuestLocalError::SeedInvalid);
        }
        let idempotency_key =
            seed_idempotency_key(&guest_uid, &descriptor_digest, &operation_id, &resources);
        Ok(Self {
            guest_ref,
            guest_uid,
            zone,
            descriptor_digest,
            operation_id,
            resources,
            idempotency_key,
        })
    }

    /// Borrow the Guest ResourceRef.
    pub const fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    /// Borrow the Guest UID fence.
    pub const fn guest_uid(&self) -> &ResourceUid {
        &self.guest_uid
    }

    /// Borrow the Zone asserted by the complete seed set.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the signed setup descriptor digest.
    pub const fn descriptor_digest(&self) -> &SchemaFingerprint {
        &self.descriptor_digest
    }

    /// Borrow the caller-provided operation identity.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Borrow the complete stable-order seed set.
    pub fn resources(&self) -> &[GuestLocalSeedResource] {
        &self.resources
    }

    /// Return the opaque idempotency key for this Guest/descriptor operation.
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    fn resource_refs(&self) -> Vec<ResourceRef> {
        self.resources
            .iter()
            .map(|resource| resource.resource_ref.clone())
            .collect()
    }
}

impl fmt::Debug for GuestLocalSeedBatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestLocalSeedBatch")
            .field("resource_count", &self.resources.len())
            .field("has_guest_uid", &true)
            .field("has_descriptor_digest", &true)
            .field("has_operation_id", &true)
            .field("has_idempotency_key", &true)
            .finish()
    }
}

/// One bounded status projection returned by the target-local Resource API.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestLocalResourceStatus {
    resource_ref: ResourceRef,
    uid: ResourceUid,
    owner_uid: ResourceUid,
    generation: ResourceGeneration,
    revision: ZoneRevision,
    phase: ResourcePhase,
    healthy: bool,
}

impl GuestLocalResourceStatus {
    /// Construct one target-local child status.
    pub fn new(
        resource_ref: ResourceRef,
        uid: ResourceUid,
        owner_uid: ResourceUid,
        generation: ResourceGeneration,
        revision: ZoneRevision,
        phase: ResourcePhase,
        healthy: bool,
    ) -> Result<Self, GuestLocalError> {
        if !is_approved_seed_type(resource_ref.resource_type().as_str())
            || generation.get() == 0
            || revision.get() == 0
        {
            return Err(GuestLocalError::SeedInvalid);
        }
        Ok(Self {
            resource_ref,
            uid,
            owner_uid,
            generation,
            revision,
            phase,
            healthy,
        })
    }

    /// Borrow the status ResourceRef.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the store-assigned child UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Borrow the Guest UID asserted by the target-local API.
    pub const fn owner_uid(&self) -> &ResourceUid {
        &self.owner_uid
    }

    /// Return the observed target-local generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// Return the target-local revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Return the universal resource phase.
    pub const fn phase(&self) -> ResourcePhase {
        self.phase
    }

    /// Whether the target-local child is healthy.
    pub const fn healthy(&self) -> bool {
        self.healthy
    }
}

impl fmt::Debug for GuestLocalResourceStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestLocalResourceStatus")
            .field("resource_type", &self.resource_ref.resource_type())
            .field("phase", &self.phase)
            .field("healthy", &self.healthy)
            .field("generation", &self.generation)
            .field("revision", &self.revision)
            .finish()
    }
}

/// Result of one target-local seed CommitBatch.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestLocalSeedResult {
    operation_id: String,
    guest_uid: ResourceUid,
    descriptor_digest: SchemaFingerprint,
    revision: ZoneRevision,
    seed_generation: ResourceGeneration,
    resources: Vec<GuestLocalResourceStatus>,
    ready: bool,
}

impl GuestLocalSeedResult {
    /// Construct a bounded seed result.
    pub fn new(
        operation_id: impl Into<String>,
        guest_uid: ResourceUid,
        descriptor_digest: SchemaFingerprint,
        revision: ZoneRevision,
        seed_generation: ResourceGeneration,
        resources: Vec<GuestLocalResourceStatus>,
    ) -> Result<Self, GuestLocalError> {
        let operation_id = operation_id.into();
        if !valid_operation_id(&operation_id)
            || revision.get() == 0
            || seed_generation.get() == 0
            || resources.is_empty()
        {
            return Err(GuestLocalError::SeedInvalid);
        }
        Ok(Self {
            operation_id,
            guest_uid,
            descriptor_digest,
            revision,
            seed_generation,
            resources,
            ready: false,
        })
    }

    /// Mark the result as current and Ready after response validation.
    pub fn with_ready(mut self, ready: bool) -> Self {
        self.ready = ready;
        self
    }

    /// Borrow the operation identity.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Borrow the Guest UID.
    pub const fn guest_uid(&self) -> &ResourceUid {
        &self.guest_uid
    }

    /// Borrow the descriptor digest.
    pub const fn descriptor_digest(&self) -> &SchemaFingerprint {
        &self.descriptor_digest
    }

    /// Return the target-local commit revision.
    pub const fn revision(&self) -> ZoneRevision {
        self.revision
    }

    /// Return the seed generation.
    pub const fn seed_generation(&self) -> ResourceGeneration {
        self.seed_generation
    }

    /// Borrow the returned child statuses.
    pub fn resources(&self) -> &[GuestLocalResourceStatus] {
        &self.resources
    }

    /// Whether every returned child is current and Ready.
    pub const fn ready(&self) -> bool {
        self.ready
    }

    fn replace_statuses(
        &mut self,
        statuses: Vec<GuestLocalResourceStatus>,
        revision: ZoneRevision,
    ) {
        self.resources = statuses;
        self.revision = revision;
        self.ready = self
            .resources
            .iter()
            .all(|resource| resource.phase() == ResourcePhase::Ready && resource.healthy());
    }
}

impl fmt::Debug for GuestLocalSeedResult {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestLocalSeedResult")
            .field("resource_count", &self.resources.len())
            .field("revision", &self.revision)
            .field("seed_generation", &self.seed_generation)
            .field("ready", &self.ready)
            .finish()
    }
}

/// A revision-resumable target-local watch response.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestLocalWatch {
    after_revision: ZoneRevision,
    snapshot_revision: ZoneRevision,
    resources: Vec<GuestLocalResourceStatus>,
}

impl GuestLocalWatch {
    /// Construct one bounded watch response.
    pub fn new(
        after_revision: ZoneRevision,
        snapshot_revision: ZoneRevision,
        resources: Vec<GuestLocalResourceStatus>,
    ) -> Result<Self, GuestLocalError> {
        if snapshot_revision < after_revision {
            return Err(GuestLocalError::WatchInvalid);
        }
        Ok(Self {
            after_revision,
            snapshot_revision,
            resources,
        })
    }

    /// Return the requested resume revision.
    pub const fn after_revision(&self) -> ZoneRevision {
        self.after_revision
    }

    /// Return the authoritative target-local snapshot revision.
    pub const fn snapshot_revision(&self) -> ZoneRevision {
        self.snapshot_revision
    }

    /// Borrow bounded watch statuses.
    pub fn resources(&self) -> &[GuestLocalResourceStatus] {
        &self.resources
    }
}

impl fmt::Debug for GuestLocalWatch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestLocalWatch")
            .field("after_revision", &self.after_revision)
            .field("snapshot_revision", &self.snapshot_revision)
            .field("resource_count", &self.resources.len())
            .finish()
    }
}

/// Exact identity and generation contract expected from one Guest session.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestLocalSessionExpectation {
    guest_ref: ResourceRef,
    guest_uid: ResourceUid,
    zone: ZoneId,
    endpoint_ref: ResourceRef,
    descriptor_digest: SchemaFingerprint,
    schema_digest: SchemaFingerprint,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    reconnect_generation: ReconnectGeneration,
    boot_identity_digest: String,
    generations: GuestGenerationSet,
}

impl GuestLocalSessionExpectation {
    /// Construct the immutable expectation used for Endpoint and session
    /// binding.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        guest_ref: ResourceRef,
        guest_uid: ResourceUid,
        zone: ZoneId,
        endpoint_ref: ResourceRef,
        descriptor_digest: SchemaFingerprint,
        schema_digest: SchemaFingerprint,
        provider_generation: ResourceGeneration,
        controller_generation: ControllerGeneration,
        reconnect_generation: ReconnectGeneration,
        boot_identity_digest: impl Into<String>,
        generations: GuestGenerationSet,
    ) -> Result<Self, GuestLocalError> {
        let boot_identity_digest = boot_identity_digest.into();
        if guest_ref.resource_type().as_str() != "Guest"
            || zone.as_str().is_empty()
            || endpoint_ref.resource_type().as_str() != "Endpoint"
            || provider_generation.get() == 0
            || controller_generation.get() == 0
            || reconnect_generation.get() == 0
            || !valid_digest(&boot_identity_digest)
        {
            return Err(GuestLocalError::SessionBindingMismatch);
        }
        Ok(Self {
            guest_ref,
            guest_uid,
            zone,
            endpoint_ref,
            descriptor_digest,
            schema_digest,
            provider_generation,
            controller_generation,
            reconnect_generation,
            boot_identity_digest,
            generations,
        })
    }

    /// Borrow the Guest ResourceRef.
    pub const fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    /// Borrow the Guest UID.
    pub const fn guest_uid(&self) -> &ResourceUid {
        &self.guest_uid
    }

    /// Borrow the exact session Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the expected Guest-control Endpoint ResourceRef.
    pub const fn endpoint_ref(&self) -> &ResourceRef {
        &self.endpoint_ref
    }

    /// Borrow the descriptor digest.
    pub const fn descriptor_digest(&self) -> &SchemaFingerprint {
        &self.descriptor_digest
    }

    /// Borrow the target-local seed schema digest.
    pub const fn schema_digest(&self) -> &SchemaFingerprint {
        &self.schema_digest
    }

    /// Return the Provider generation.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Return the controller generation.
    pub const fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }

    /// Return the minimum reconnect generation.
    pub const fn reconnect_generation(&self) -> ReconnectGeneration {
        self.reconnect_generation
    }

    /// Borrow the redacted boot identity commitment.
    pub fn boot_identity_digest(&self) -> &str {
        &self.boot_identity_digest
    }

    /// Return the host-side generation tuple consumed by the status reducer.
    pub const fn generations(&self) -> GuestGenerationSet {
        self.generations
    }
}

impl fmt::Debug for GuestLocalSessionExpectation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestLocalSessionExpectation")
            .field("has_guest_uid", &true)
            .field("has_endpoint_ref", &true)
            .field("has_descriptor_digest", &true)
            .field("has_schema_digest", &true)
            .field("provider_generation", &self.provider_generation)
            .field("controller_generation", &self.controller_generation)
            .field("reconnect_generation", &self.reconnect_generation)
            .field("has_boot_identity", &true)
            .finish()
    }
}

/// Exact authenticated binding retained by a Guest-control session.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestLocalSessionBinding {
    guest_ref: ResourceRef,
    guest_uid: ResourceUid,
    zone: ZoneId,
    endpoint_ref: ResourceRef,
    endpoint_uid: ResourceUid,
    endpoint_resource_generation: ResourceGeneration,
    endpoint_generation: ResourceGeneration,
    descriptor_digest: SchemaFingerprint,
    schema_digest: SchemaFingerprint,
    provider_generation: ResourceGeneration,
    controller_generation: ControllerGeneration,
    reconnect_generation: ReconnectGeneration,
    session_generation: ReconnectGeneration,
    boot_identity_digest: String,
}

impl GuestLocalSessionBinding {
    /// Construct the exact identity tuple returned by an authenticated
    /// Guest-control session.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        guest_ref: ResourceRef,
        guest_uid: ResourceUid,
        zone: ZoneId,
        endpoint_ref: ResourceRef,
        endpoint_uid: ResourceUid,
        endpoint_resource_generation: ResourceGeneration,
        endpoint_generation: ResourceGeneration,
        descriptor_digest: SchemaFingerprint,
        schema_digest: SchemaFingerprint,
        provider_generation: ResourceGeneration,
        controller_generation: ControllerGeneration,
        reconnect_generation: ReconnectGeneration,
        session_generation: ReconnectGeneration,
        boot_identity_digest: impl Into<String>,
    ) -> Result<Self, GuestLocalError> {
        let boot_identity_digest = boot_identity_digest.into();
        if guest_ref.resource_type().as_str() != "Guest"
            || zone.as_str().is_empty()
            || endpoint_ref.resource_type().as_str() != "Endpoint"
            || endpoint_resource_generation.get() == 0
            || endpoint_generation.get() == 0
            || provider_generation.get() == 0
            || controller_generation.get() == 0
            || reconnect_generation.get() == 0
            || session_generation.get() == 0
            || reconnect_generation != session_generation
            || !valid_digest(&boot_identity_digest)
        {
            return Err(GuestLocalError::SessionBindingMismatch);
        }
        Ok(Self {
            guest_ref,
            guest_uid,
            zone,
            endpoint_ref,
            endpoint_uid,
            endpoint_resource_generation,
            endpoint_generation,
            descriptor_digest,
            schema_digest,
            provider_generation,
            controller_generation,
            reconnect_generation,
            session_generation,
            boot_identity_digest,
        })
    }

    /// Borrow the Guest ResourceRef.
    pub const fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    /// Borrow the Guest UID.
    pub const fn guest_uid(&self) -> &ResourceUid {
        &self.guest_uid
    }

    /// Borrow the exact authenticated session Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the Endpoint ResourceRef.
    pub const fn endpoint_ref(&self) -> &ResourceRef {
        &self.endpoint_ref
    }

    /// Borrow the Endpoint UID.
    pub const fn endpoint_uid(&self) -> &ResourceUid {
        &self.endpoint_uid
    }

    /// Return the Endpoint Resource generation.
    pub const fn endpoint_resource_generation(&self) -> ResourceGeneration {
        self.endpoint_resource_generation
    }

    /// Return the producer-derived Endpoint generation.
    pub const fn endpoint_generation(&self) -> ResourceGeneration {
        self.endpoint_generation
    }

    /// Borrow the descriptor digest.
    pub const fn descriptor_digest(&self) -> &SchemaFingerprint {
        &self.descriptor_digest
    }

    /// Borrow the target-local schema digest.
    pub const fn schema_digest(&self) -> &SchemaFingerprint {
        &self.schema_digest
    }

    /// Return the Provider generation.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Return the controller generation.
    pub const fn controller_generation(&self) -> ControllerGeneration {
        self.controller_generation
    }

    /// Return the reconnect generation.
    pub const fn reconnect_generation(&self) -> ReconnectGeneration {
        self.reconnect_generation
    }

    /// Return the session generation.
    pub const fn session_generation(&self) -> ReconnectGeneration {
        self.session_generation
    }

    /// Borrow the boot identity commitment.
    pub fn boot_identity_digest(&self) -> &str {
        &self.boot_identity_digest
    }

    /// Validate this binding against one exact Guest session expectation.
    pub fn validate_against(
        &self,
        expected: &GuestLocalSessionExpectation,
    ) -> Result<(), GuestLocalError> {
        if self.guest_ref != expected.guest_ref
            || self.guest_uid != expected.guest_uid
            || self.zone != expected.zone
            || self.endpoint_ref != expected.endpoint_ref
            || self.descriptor_digest != expected.descriptor_digest
            || self.schema_digest != expected.schema_digest
            || self.provider_generation != expected.provider_generation
            || self.controller_generation != expected.controller_generation
            || self.reconnect_generation < expected.reconnect_generation
            || self.session_generation != self.reconnect_generation
            || self.boot_identity_digest != expected.boot_identity_digest
        {
            return Err(GuestLocalError::SessionBindingMismatch);
        }
        Ok(())
    }

    fn evidence_binding(
        &self,
        seed_generation: ResourceGeneration,
    ) -> Result<GuestSessionEvidenceBinding, GuestLocalError> {
        GuestSessionEvidenceBinding::new(
            self.guest_uid.as_str(),
            self.descriptor_digest.as_str(),
            self.schema_digest.as_str(),
            self.provider_generation.get(),
            self.controller_generation.get(),
            self.session_generation.get(),
            self.reconnect_generation.get(),
            self.endpoint_generation.get(),
            seed_generation.get(),
        )
        .map_err(|_| GuestLocalError::SessionBindingMismatch)
    }
}

impl fmt::Debug for GuestLocalSessionBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestLocalSessionBinding")
            .field("has_guest_uid", &true)
            .field("has_endpoint_uid", &true)
            .field(
                "endpoint_resource_generation",
                &self.endpoint_resource_generation,
            )
            .field("endpoint_generation", &self.endpoint_generation)
            .field("has_descriptor_digest", &true)
            .field("has_schema_digest", &true)
            .field("provider_generation", &self.provider_generation)
            .field("controller_generation", &self.controller_generation)
            .field("reconnect_generation", &self.reconnect_generation)
            .field("session_generation", &self.session_generation)
            .field("has_boot_identity", &true)
            .finish()
    }
}

/// Authenticated, locator-free identity of a Guest-control Endpoint.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestControlEndpoint {
    endpoint_ref: ResourceRef,
    guest_ref: ResourceRef,
    zone: ZoneId,
    uid: ResourceUid,
    resource_generation: ResourceGeneration,
    endpoint_generation: ResourceGeneration,
    provider_generation: ResourceGeneration,
    schema_digest: SchemaFingerprint,
    ready: bool,
}

impl GuestControlEndpoint {
    /// Construct one resolved Guest-control Endpoint identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        endpoint_ref: ResourceRef,
        guest_ref: ResourceRef,
        zone: ZoneId,
        uid: ResourceUid,
        resource_generation: ResourceGeneration,
        endpoint_generation: ResourceGeneration,
        provider_generation: ResourceGeneration,
        schema_digest: SchemaFingerprint,
        ready: bool,
    ) -> Result<Self, GuestLocalError> {
        if endpoint_ref.resource_type().as_str() != "Endpoint"
            || guest_ref.resource_type().as_str() != "Guest"
            || zone.as_str().is_empty()
            || resource_generation.get() == 0
            || endpoint_generation.get() == 0
            || provider_generation.get() == 0
            || !ready
        {
            return Err(GuestLocalError::EndpointMismatch);
        }
        Ok(Self {
            endpoint_ref,
            guest_ref,
            zone,
            uid,
            resource_generation,
            endpoint_generation,
            provider_generation,
            schema_digest,
            ready,
        })
    }

    /// Borrow the exact Endpoint ResourceRef.
    pub const fn endpoint_ref(&self) -> &ResourceRef {
        &self.endpoint_ref
    }

    /// Borrow the producing Guest ResourceRef.
    pub const fn guest_ref(&self) -> &ResourceRef {
        &self.guest_ref
    }

    /// Borrow the exact Endpoint Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the store-assigned Endpoint UID.
    pub const fn uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Borrow the store-assigned Endpoint UID.
    pub const fn endpoint_uid(&self) -> &ResourceUid {
        &self.uid
    }

    /// Return the Endpoint Resource generation.
    pub const fn resource_generation(&self) -> ResourceGeneration {
        self.resource_generation
    }

    /// Return the producer-derived Endpoint generation.
    pub const fn endpoint_generation(&self) -> ResourceGeneration {
        self.endpoint_generation
    }

    /// Return the Provider generation observed with the Endpoint.
    pub const fn provider_generation(&self) -> ResourceGeneration {
        self.provider_generation
    }

    /// Borrow the target-local schema commitment.
    pub const fn schema_digest(&self) -> &SchemaFingerprint {
        &self.schema_digest
    }

    /// Whether the Endpoint is currently ready for an authenticated session.
    pub const fn ready(&self) -> bool {
        self.ready
    }

    /// Validate this resolution against one exact Guest and Provider contract.
    pub fn validate_for(
        &self,
        endpoint_ref: &ResourceRef,
        guest_ref: &ResourceRef,
        provider_generation: ResourceGeneration,
        schema_digest: &SchemaFingerprint,
    ) -> Result<(), GuestLocalError> {
        if !self.ready
            || &self.endpoint_ref != endpoint_ref
            || &self.guest_ref != guest_ref
            || self.provider_generation != provider_generation
            || &self.schema_digest != schema_digest
        {
            return Err(GuestLocalError::EndpointMismatch);
        }
        Ok(())
    }
}

impl fmt::Debug for GuestControlEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestControlEndpoint")
            .field("ready", &self.ready)
            .field("has_endpoint_uid", &true)
            .field("resource_generation", &self.resource_generation)
            .field("endpoint_generation", &self.endpoint_generation)
            .field("provider_generation", &self.provider_generation)
            .field("has_schema_digest", &true)
            .finish()
    }
}

/// A resolved Guest-control Endpoint from an authorized Endpoint path.
#[async_trait]
pub trait GuestControlEndpointResolver: Send + Sync {
    /// Resolve the exact Endpoint identity without exposing its carriage.
    async fn resolve_guest_control_endpoint(
        &self,
        endpoint_ref: &ResourceRef,
    ) -> Result<GuestControlEndpoint, GuestLocalError>;
}

/// An authenticated Guest-control ComponentSession.
#[async_trait]
pub trait GuestLocalSession: Send + Sync {
    /// Borrow the exact authenticated identity tuple.
    fn binding(&self) -> &GuestLocalSessionBinding;

    /// Return the transport evidence for this session.
    fn transport_descriptor(&self) -> TransportDescriptor;

    /// Whether the session's liveness marker remains valid.
    fn is_live(&self) -> bool;

    /// Submit one descriptor-approved UID-free seed batch.
    async fn commit_seed_batch(
        &self,
        batch: &GuestLocalSeedBatch,
    ) -> Result<GuestLocalSeedResult, GuestLocalError>;

    /// Resume the target-local watch at the last committed revision.
    async fn resume_seed_watch(
        &self,
        after_revision: ZoneRevision,
        resources: &[ResourceRef],
    ) -> Result<GuestLocalWatch, GuestLocalError>;
}

/// Constructs an authenticated Guest-control session for one Endpoint.
#[async_trait]
pub trait GuestControlSessionConnector: Send + Sync {
    /// The concrete authenticated session.
    type Session: GuestLocalSession;

    /// Connect at or above the requested reconnect generation.
    async fn connect_guest_control(
        &self,
        endpoint: &GuestControlEndpoint,
        minimum_generation: ReconnectGeneration,
    ) -> Result<Self::Session, GuestLocalError>;
}

/// Public Guest status returned by the local seeding reconciler.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct GuestLocalStatus {
    phase: GuestStatusPhase,
    runtime_ready: bool,
    bootstrap_ready: bool,
    session_healthy: bool,
}

impl GuestLocalStatus {
    /// Construct a bounded status projection for tests and adapters.
    pub const fn new(
        phase: GuestStatusPhase,
        runtime_ready: bool,
        bootstrap_ready: bool,
        session_healthy: bool,
    ) -> Self {
        Self {
            phase,
            runtime_ready,
            bootstrap_ready,
            session_healthy,
        }
    }

    /// Return the public Guest phase.
    pub const fn phase(self) -> GuestStatusPhase {
        self.phase
    }

    /// Return whether host runtime readiness is current.
    pub const fn runtime_ready(self) -> bool {
        self.runtime_ready
    }

    /// Return whether endpoint/session/seed bootstrap is current.
    pub const fn bootstrap_ready(self) -> bool {
        self.bootstrap_ready
    }

    /// Return whether the current session is healthy.
    pub const fn session_healthy(self) -> bool {
        self.session_healthy
    }
}

impl fmt::Debug for GuestLocalStatus {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestLocalStatus")
            .field("phase", &self.phase)
            .field("runtime_ready", &self.runtime_ready)
            .field("bootstrap_ready", &self.bootstrap_ready)
            .field("session_healthy", &self.session_healthy)
            .finish()
    }
}

/// Result of one Guest-local reconcile pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestLocalReconcileOutcome {
    /// Host or target-local readiness remains incomplete.
    Pending(GuestLocalStatus),
    /// Host and target-local seed generations are current and healthy.
    Ready(GuestLocalStatus),
    /// A previously ready session or target-local child lost health.
    Degraded(GuestLocalStatus),
}

impl GuestLocalReconcileOutcome {
    /// Borrow the public status.
    pub const fn status(self) -> GuestLocalStatus {
        match self {
            Self::Pending(status) | Self::Ready(status) | Self::Degraded(status) => status,
        }
    }

    /// Return the public phase.
    pub const fn phase(self) -> GuestStatusPhase {
        self.status().phase()
    }
}

/// Stateful Guest-local seeding and readiness reconciler.
pub struct GuestLocalController<R, C>
where
    R: GuestControlEndpointResolver,
    C: GuestControlSessionConnector,
{
    expectation: GuestLocalSessionExpectation,
    resolver: R,
    connector: C,
    session: Option<C::Session>,
    last_binding: Option<GuestLocalSessionBinding>,
    last_seed: Option<GuestLocalSeedResult>,
    last_batch_key: Option<String>,
    operations: BTreeMap<String, String>,
    last_revision: ZoneRevision,
    was_ready: bool,
}

impl<R, C> GuestLocalController<R, C>
where
    R: GuestControlEndpointResolver,
    C: GuestControlSessionConnector,
{
    /// Construct a reconciler with no active session or seed authority.
    pub fn new(expectation: GuestLocalSessionExpectation, resolver: R, connector: C) -> Self {
        Self {
            expectation,
            resolver,
            connector,
            session: None,
            last_binding: None,
            last_seed: None,
            last_batch_key: None,
            operations: BTreeMap::new(),
            last_revision: ZoneRevision::new(0),
            was_ready: false,
        }
    }

    /// Borrow the immutable session expectation.
    pub const fn expectation(&self) -> &GuestLocalSessionExpectation {
        &self.expectation
    }

    /// Return the last target-local revision observed by this controller.
    pub const fn last_revision(&self) -> ZoneRevision {
        self.last_revision
    }

    /// Return bounded U23 session evidence for the current observation.
    pub fn session_evidence(&self) -> Option<GuestSessionEvidence> {
        let binding = self.last_binding.as_ref()?;
        let seed_generation = self
            .last_seed
            .as_ref()
            .map(GuestLocalSeedResult::seed_generation)
            .unwrap_or(self.expectation.provider_generation);
        let evidence_binding = binding.evidence_binding(seed_generation).ok()?;
        let live = self
            .session
            .as_ref()
            .is_some_and(GuestLocalSession::is_live);
        if live {
            GuestSessionEvidence::current_bound(
                self.expectation.guest_ref.clone(),
                self.expectation.boot_identity_digest.clone(),
                ["resource-commit".to_owned(), "resource-watch".to_owned()],
                true,
                true,
                self.last_seed
                    .as_ref()
                    .is_some_and(GuestLocalSeedResult::ready),
                evidence_binding,
            )
            .ok()
        } else {
            GuestSessionEvidence::current_bound(
                self.expectation.guest_ref.clone(),
                self.expectation.boot_identity_digest.clone(),
                ["resource-commit".to_owned(), "resource-watch".to_owned()],
                true,
                true,
                false,
                evidence_binding,
            )
            .ok()
        }
    }

    /// Project the current bounded local readiness into U24's reducer.
    pub fn status(&self, host: &GuestStatusObservation) -> GuestLocalStatus {
        let host_current = host.generations == self.expectation.generations;
        let seed_current = self.last_seed.as_ref().is_some_and(|seed| {
            seed.guest_uid() == self.expectation.guest_uid()
                && seed.descriptor_digest() == self.expectation.descriptor_digest()
                && seed.seed_generation() == self.expectation.provider_generation
                && seed.ready()
                && self.last_batch_key.is_some()
        });
        let seed_pending = self.last_seed.as_ref().is_some_and(|seed| {
            seed.resources()
                .iter()
                .any(|resource| resource.phase() == ResourcePhase::Pending)
        });
        let seed_degraded = self.last_seed.as_ref().is_some_and(|seed| {
            seed.resources()
                .iter()
                .any(|resource| resource.phase() != ResourcePhase::Ready || !resource.healthy())
        });
        let session_live = self
            .session
            .as_ref()
            .is_some_and(GuestLocalSession::is_live);
        let retained_ready =
            self.was_ready && self.last_seed.is_some() && self.last_batch_key.is_some();
        let local_ready = seed_current || (retained_ready && !seed_pending);
        let mut observation = *host;
        observation.session_ready = host_current && local_ready;
        observation.seed_ready = host_current && local_ready;
        observation.session_healthy = host_current && session_live;
        if !host_current {
            observation.session_ready = false;
            observation.seed_ready = false;
            observation.session_healthy = false;
        }
        observation.required_children_healthy =
            observation.required_children_healthy && !(retained_ready && seed_degraded);
        let status = reduce_status(&observation);
        GuestLocalStatus::new(
            status.phase,
            status.runtime_ready,
            status.bootstrap_ready,
            observation.session_healthy,
        )
    }

    /// Mark the active session disconnected while retaining only its last
    /// bounded target-local observation.
    pub fn mark_session_lost(&mut self) {
        self.session = None;
    }

    /// Reconcile host readiness, Endpoint resolution, session binding, and a
    /// complete target-local seed batch.
    pub async fn reconcile(
        &mut self,
        host: &GuestStatusObservation,
        batch: GuestLocalSeedBatch,
    ) -> Result<GuestLocalReconcileOutcome, GuestLocalError> {
        if !host.generations.is_exact()
            || host.generations != self.expectation.generations
            || !host.dependencies_ready
            || !host.process_ready
            || !host.endpoint_ready
        {
            return Ok(GuestLocalReconcileOutcome::Pending(self.status(host)));
        }
        self.validate_batch(&batch)?;
        let batch_key = batch.idempotency_key().to_owned();
        if self
            .operations
            .get(batch.operation_id())
            .is_some_and(|prior| prior != &batch_key)
        {
            return Err(GuestLocalError::OperationReused);
        }
        let endpoint = self
            .resolver
            .resolve_guest_control_endpoint(self.expectation.endpoint_ref())
            .await?;
        endpoint
            .validate_for(
                self.expectation.endpoint_ref(),
                self.expectation.guest_ref(),
                self.expectation.provider_generation,
                self.expectation.schema_digest(),
            )
            .map_err(|_| GuestLocalError::EndpointMismatch)?;
        if endpoint.zone() != self.expectation.zone() || batch.zone() != self.expectation.zone() {
            return Err(GuestLocalError::EndpointMismatch);
        }
        if self
            .session
            .as_ref()
            .is_none_or(|session| !session.is_live())
        {
            self.session = None;
            let minimum_generation = self
                .last_binding
                .as_ref()
                .map(|binding| {
                    ReconnectGeneration::new(binding.session_generation().get().saturating_add(1))
                        .unwrap_or(self.expectation.reconnect_generation)
                })
                .unwrap_or(self.expectation.reconnect_generation);
            let session = match self
                .connector
                .connect_guest_control(&endpoint, minimum_generation)
                .await
            {
                Ok(session) => session,
                Err(GuestLocalError::SessionLost) if self.was_ready => {
                    return Ok(self.session_loss_outcome(host));
                }
                Err(error) => return Err(error),
            };
            self.validate_session(&session, &endpoint, minimum_generation)?;
            self.last_binding = Some(session.binding().clone());
            self.session = Some(session);
        }
        if self.last_seed.is_some() && self.last_revision.get() != 0 {
            let watch = match {
                let session = self.session.as_ref().ok_or(GuestLocalError::SessionLost)?;
                session
                    .resume_seed_watch(self.last_revision, &batch.resource_refs())
                    .await
            } {
                Ok(watch) => watch,
                Err(GuestLocalError::SessionLost) => {
                    self.mark_session_lost();
                    return Ok(self.session_loss_outcome(host));
                }
                Err(error) => return Err(error),
            };
            self.apply_watch(&batch, watch)?;
        }
        let needs_commit = self.last_seed.is_none()
            || self.last_batch_key.as_deref() != Some(batch.idempotency_key());
        if needs_commit {
            let result = match {
                let session = self.session.as_ref().ok_or(GuestLocalError::SessionLost)?;
                session.commit_seed_batch(&batch).await
            } {
                Ok(result) => result,
                Err(GuestLocalError::SessionLost) => {
                    self.mark_session_lost();
                    return Ok(self.session_loss_outcome(host));
                }
                Err(error) => return Err(error),
            };
            self.validate_result(&batch, &result)?;
            self.last_revision = result.revision();
            self.last_seed = Some(result);
            self.operations
                .insert(batch.operation_id().to_owned(), batch_key.clone());
            self.last_batch_key = Some(batch_key);
        }
        let status = self.status(host);
        self.was_ready |= status.phase() == GuestStatusPhase::Ready;
        Ok(match status.phase() {
            GuestStatusPhase::Ready => GuestLocalReconcileOutcome::Ready(status),
            GuestStatusPhase::Degraded => GuestLocalReconcileOutcome::Degraded(status),
            GuestStatusPhase::Pending | GuestStatusPhase::Draining => {
                GuestLocalReconcileOutcome::Pending(status)
            }
        })
    }

    fn validate_batch(&self, batch: &GuestLocalSeedBatch) -> Result<(), GuestLocalError> {
        if batch.guest_ref() != self.expectation.guest_ref()
            || batch.guest_uid() != self.expectation.guest_uid()
            || batch.zone() != self.expectation.zone()
            || batch.descriptor_digest() != self.expectation.descriptor_digest()
        {
            return Err(GuestLocalError::SeedInvalid);
        }
        Ok(())
    }

    fn session_loss_outcome(&self, host: &GuestStatusObservation) -> GuestLocalReconcileOutcome {
        let status = self.status(host);
        if status.phase() == GuestStatusPhase::Degraded {
            GuestLocalReconcileOutcome::Degraded(status)
        } else {
            GuestLocalReconcileOutcome::Pending(status)
        }
    }

    fn validate_session(
        &self,
        session: &C::Session,
        endpoint: &GuestControlEndpoint,
        minimum_generation: ReconnectGeneration,
    ) -> Result<(), GuestLocalError> {
        if !d2b_session_unix::is_guest_control_transport(session.transport_descriptor()) {
            return Err(GuestLocalError::TransportMismatch);
        }
        let binding = session.binding();
        binding.validate_against(&self.expectation)?;
        if binding.session_generation() < minimum_generation
            || self.last_binding.as_ref().is_some_and(|previous| {
                binding.session_generation() <= previous.session_generation()
            })
            || binding.endpoint_uid() != endpoint.uid()
            || binding.endpoint_resource_generation() != endpoint.resource_generation()
            || binding.endpoint_generation() != endpoint.endpoint_generation()
            || binding.provider_generation() != endpoint.provider_generation()
        {
            return Err(GuestLocalError::SessionGenerationStale);
        }
        Ok(())
    }

    fn validate_result(
        &self,
        batch: &GuestLocalSeedBatch,
        result: &GuestLocalSeedResult,
    ) -> Result<(), GuestLocalError> {
        if result.operation_id() != batch.operation_id()
            || result.guest_uid() != self.expectation.guest_uid()
            || result.descriptor_digest() != self.expectation.descriptor_digest()
            || result.seed_generation() != self.expectation.provider_generation
            || result.resources().len() != batch.resources().len()
        {
            return Err(GuestLocalError::SeedInvalid);
        }
        let expected = batch
            .resources()
            .iter()
            .map(|resource| resource.resource_ref())
            .collect::<BTreeSet<_>>();
        let mut seen = BTreeSet::new();
        for status in result.resources() {
            if !expected.contains(&status.resource_ref())
                || status.owner_uid() != self.expectation.guest_uid()
                || status.generation() != self.expectation.provider_generation
                || !seen.insert(status.resource_ref())
            {
                return Err(GuestLocalError::SeedInvalid);
            }
        }
        if seen != expected {
            return Err(GuestLocalError::SeedInvalid);
        }
        let result_ready = result
            .resources()
            .iter()
            .all(|status| status.phase() == ResourcePhase::Ready && status.healthy());
        if result.ready() != result_ready {
            return Err(GuestLocalError::SeedInvalid);
        }
        Ok(())
    }

    fn apply_watch(
        &mut self,
        batch: &GuestLocalSeedBatch,
        watch: GuestLocalWatch,
    ) -> Result<(), GuestLocalError> {
        if watch.after_revision() != self.last_revision
            || watch.snapshot_revision() < self.last_revision
        {
            return Err(GuestLocalError::WatchInvalid);
        }
        if watch.resources().is_empty() {
            self.last_revision = watch.snapshot_revision();
            return Ok(());
        }
        let expected = batch
            .resources()
            .iter()
            .map(|resource| resource.resource_ref())
            .collect::<BTreeSet<_>>();
        let mut statuses = BTreeMap::new();
        for status in watch.resources() {
            if !expected.contains(&status.resource_ref())
                || status.owner_uid() != self.expectation.guest_uid()
                || status.generation() != self.expectation.provider_generation
                || statuses
                    .insert(status.resource_ref().clone(), status.clone())
                    .is_some()
            {
                return Err(GuestLocalError::WatchInvalid);
            }
        }
        if statuses.len() != expected.len() {
            return Err(GuestLocalError::WatchInvalid);
        }
        if let Some(seed) = self.last_seed.as_mut() {
            seed.replace_statuses(statuses.into_values().collect(), watch.snapshot_revision());
        }
        self.last_revision = watch.snapshot_revision();
        Ok(())
    }
}

impl<R, C> fmt::Debug for GuestLocalController<R, C>
where
    R: GuestControlEndpointResolver,
    C: GuestControlSessionConnector,
{
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestLocalController")
            .field("has_session", &self.session.is_some())
            .field("has_binding", &self.last_binding.is_some())
            .field("has_seed", &self.last_seed.is_some())
            .field("last_revision", &self.last_revision)
            .field("was_ready", &self.was_ready)
            .finish()
    }
}

fn is_approved_seed_type(resource_type: &str) -> bool {
    GUEST_SEED_RESOURCE_TYPES.contains(&resource_type)
}

fn valid_operation_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OPERATION_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b':'))
}

fn valid_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn seed_payload_zone(canonical: &[u8]) -> Option<ZoneId> {
    let value: serde_json::Value = serde_json::from_slice(canonical).ok()?;
    let zone = value
        .get("metadata")
        .and_then(|metadata| metadata.get("zone"))
        .and_then(serde_json::Value::as_str)?;
    ZoneId::parse(zone).ok()
}

fn seed_idempotency_key(
    guest_uid: &ResourceUid,
    descriptor_digest: &SchemaFingerprint,
    operation_id: &str,
    resources: &[GuestLocalSeedResource],
) -> String {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(guest_uid.as_str().as_bytes());
    bytes.extend_from_slice(descriptor_digest.as_str().as_bytes());
    bytes.extend_from_slice(operation_id.as_bytes());
    for resource in resources {
        bytes.extend_from_slice(resource.resource_ref().to_canonical_string().as_bytes());
        bytes.extend_from_slice(resource.digest().as_bytes());
    }
    d2b_contracts_resource::v3::canonical_digest(SEED_DIGEST_DOMAIN, &bytes)
}

fn validate_seed_value(
    value: &serde_json::Value,
    resource_ref: &ResourceRef,
    owner_ref: &ResourceRef,
) -> Result<(), GuestLocalSeedResourceError> {
    let object = value
        .as_object()
        .ok_or(GuestLocalSeedResourceError::Malformed)?;
    if object.get("apiVersion").and_then(serde_json::Value::as_str)
        != Some("resources.d2bus.org/v3")
        || !object.contains_key("status")
    {
        return Err(GuestLocalSeedResourceError::Malformed);
    }

    let resource_type = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or(GuestLocalSeedResourceError::Malformed)?;
    if resource_type != resource_ref.resource_type().as_str() {
        return Err(GuestLocalSeedResourceError::IdentityMismatch);
    }
    let metadata = object
        .get("metadata")
        .and_then(serde_json::Value::as_object)
        .ok_or(GuestLocalSeedResourceError::Malformed)?;
    if metadata.get("uid").is_some() {
        return Err(GuestLocalSeedResourceError::UidNotAllowed);
    }
    if metadata.get("name").and_then(serde_json::Value::as_str)
        != Some(resource_ref.name().as_str())
    {
        return Err(GuestLocalSeedResourceError::IdentityMismatch);
    }
    if metadata.get("ownerRef").and_then(serde_json::Value::as_str)
        != Some(owner_ref.to_canonical_string().as_str())
    {
        return Err(GuestLocalSeedResourceError::OwnerMismatch);
    }
    if contains_private_field(value) {
        return Err(GuestLocalSeedResourceError::PrivateField);
    }
    let spec = object
        .get("spec")
        .and_then(serde_json::Value::as_object)
        .ok_or(GuestLocalSeedResourceError::Malformed)?;
    let relationship = if resource_ref.resource_type().as_str() == "Endpoint" {
        spec.get("producerRef")
    } else {
        spec.get("executionRef")
    };
    if relationship.and_then(serde_json::Value::as_str)
        != Some(owner_ref.to_canonical_string().as_str())
    {
        return Err(GuestLocalSeedResourceError::RelationshipMismatch);
    }
    Ok(())
}

fn parse_uid_free_envelope(canonical: &[u8]) -> Result<ResourceEnvelope, ()> {
    if let Ok(envelope) = ResourceEnvelope::from_json(canonical) {
        return Ok(envelope);
    }
    let mut value: serde_json::Value = serde_json::from_slice(canonical).map_err(|_| ())?;
    let metadata = value
        .get_mut("metadata")
        .and_then(serde_json::Value::as_object_mut)
        .ok_or(())?;
    if metadata.contains_key("uid") {
        return Err(());
    }
    metadata.insert(
        "uid".to_owned(),
        serde_json::Value::String("00000000-0000-4000-8000-000000000000".to_owned()),
    );
    let with_uid = serde_json::to_vec(&value).map_err(|_| ())?;
    let with_uid = CanonicalJsonValue::parse(&with_uid)
        .map_err(|_| ())?
        .to_canonical_bytes();
    ResourceEnvelope::from_json(&with_uid).map_err(|_| ())
}

fn contains_private_field(value: &serde_json::Value) -> bool {
    const PRIVATE_KEYS: &[&str] = &[
        "argv",
        "cid",
        "credential",
        "credentials",
        "environment",
        "endpoint",
        "fd",
        "gid",
        "hostpath",
        "key",
        "locator",
        "password",
        "path",
        "pid",
        "port",
        "secret",
        "socket",
        "socketpath",
        "storepath",
        "token",
        "uid",
        "vsock",
    ];
    match value {
        serde_json::Value::Object(object) => object.iter().any(|(key, value)| {
            let normalized = key.to_ascii_lowercase();
            PRIVATE_KEYS.contains(&normalized.as_str()) || contains_private_field(value)
        }),
        serde_json::Value::Array(values) => values.iter().any(contains_private_field),
        serde_json::Value::String(value) => value.starts_with('/') || value.contains("/nix/store/"),
        _ => false,
    }
}
