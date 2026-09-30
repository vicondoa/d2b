//! The provider-neutral `ResourceExport` contract.
//!
//! A `ResourceExport` is an owner-Zone advertisement for one qualified
//! semantic Service.  It is deliberately not a transport record: the
//! backing resource, remote Zone, session, stream, and lease handles stay
//! outside this resource contract.

use d2b_contracts::wire_deserialize;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use d2b_contracts_provider::v3::provider::{
    Exportability, ProjectionFactory, ProviderContractError, RESOURCE_IMPORT_OWNER_TYPE,
};
use d2b_contracts_resource::v3::identity::BindingDigest;
use d2b_contracts_resource::v3::{
    ResourceRef, ResourceTypeName, SchemaFingerprint, ZoneId,
    execution_policy::{BoundedToken, PrimitiveSpecError, redacted_debug},
    resource::ResourceEnvelope,
};

/// The canonical ResourceType name for an export declaration.
pub const RESOURCE_EXPORT_RESOURCE_TYPE: &str = "ResourceExport";
/// The Core finalizer used while an export's leases drain.
pub const RESOURCE_EXPORT_DRAIN_FINALIZER: &str = "core.resource-export-drain";
/// Maximum operations in one export capability ceiling.
pub const MAX_RESOURCE_EXPORT_OPERATIONS: usize = 64;
/// Maximum consumer Zones in one export policy.
pub const MAX_RESOURCE_EXPORT_ZONES: usize = 64;
/// Maximum capability tokens in one export policy.
pub const MAX_RESOURCE_EXPORT_CAPABILITIES: usize = 64;
/// Maximum simultaneous consumers admitted by one export quota.
pub const MAX_RESOURCE_EXPORT_CONSUMERS: u32 = 256;
/// Maximum per-consumer rate admitted by one export quota.
pub const MAX_RESOURCE_EXPORT_RATE: u32 = 1_000_000;
/// Maximum lease deadline in milliseconds.
pub const MAX_RESOURCE_EXPORT_LEASE_DEADLINE_MS: u64 = 900_000;
/// Maximum forced-revocation grace period in milliseconds.
pub const MAX_RESOURCE_EXPORT_REVOCATION_GRACE_MS: u64 = 900_000;
/// Maximum lease summaries retained in one status projection.
pub const MAX_RESOURCE_EXPORT_LEASE_SUMMARIES: usize = 64;

/// Closed validation failures for the `ResourceExport` base contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceExportContractError {
    /// A required field was absent.
    MissingRequiredField,
    /// A ResourceRef named the wrong ResourceType.
    WrongResourceType,
    /// The target type is not a qualified semantic Service.
    ServiceTypeInvalid,
    /// The ResourceRef and stored target identity do not agree.
    ResourceReferenceMismatch,
    /// A bounded operation or capability token was invalid.
    InvalidOperation,
    /// An operation, Zone, or capability bound was exceeded.
    BoundExceeded,
    /// A list that must be unique contained a duplicate.
    DuplicateEntry,
    /// A quota or revocation policy was outside its bounds.
    PolicyInvalid,
    /// A projection schema or factory fingerprint did not match.
    FingerprintMismatch,
    /// The export subject is owned by a `ResourceImport`. A lease over a
    /// remote Service is never re-exportable, so a consumer Zone cannot
    /// launder the source Zone's authority by re-advertising what it imported.
    ImportOwnedOriginRejected,
}

impl ResourceExportContractError {
    /// Return the stable identity-free diagnostic label.
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingRequiredField => "resource-export-required-field-missing",
            Self::WrongResourceType => "resource-export-reference-type-invalid",
            Self::ServiceTypeInvalid => "resource-export-service-type-invalid",
            Self::ResourceReferenceMismatch => "resource-export-reference-mismatch",
            Self::InvalidOperation => "resource-export-operation-invalid",
            Self::BoundExceeded => "resource-export-bound-exceeded",
            Self::DuplicateEntry => "resource-export-duplicate-entry",
            Self::PolicyInvalid => "resource-export-policy-invalid",
            Self::FingerprintMismatch => "resource-export-fingerprint-mismatch",
            Self::ImportOwnedOriginRejected => "resource-export-import-owned-origin-rejected",
        }
    }
}

impl core::fmt::Display for ResourceExportContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ResourceExportContractError {}

impl From<PrimitiveSpecError> for ResourceExportContractError {
    fn from(_error: PrimitiveSpecError) -> Self {
        Self::InvalidOperation
    }
}

/// Arbitration mode for one exported capability.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ExportArbitration {
    /// At most one consumer lease is active.
    Exclusive,
    /// Multiple consumers may hold bounded leases.
    Shared,
    /// Multiple consumers share one owner-side mediator.
    Multiplexed,
}

/// Visibility scope of an export advertisement.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ExportVisibility {
    /// Only descendants in the declared ZoneLink tree may consume it.
    ChildZones,
    /// Only the explicitly listed child Zones may consume it.
    NamedZones,
}

fn default_export_visibility() -> ExportVisibility {
    ExportVisibility::ChildZones
}

/// Fairness policy for bounded consumer admission.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ShareFairness {
    /// Admit consumers in arrival order.
    #[default]
    Fifo,
    /// Use the Provider's signed priority policy.
    Priority,
    /// Use the Provider's signed weighted policy.
    Weighted,
}

/// Bounded quota and deadline policy shared by export and import requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ShareQuota {
    max_consumers: Option<u32>,
    per_consumer_rate: Option<u32>,
    fairness: ShareFairness,
    lease_deadline_ms: Option<u64>,
}

impl ShareQuota {
    /// Construct a bounded quota policy.
    pub const fn new(
        max_consumers: Option<u32>,
        per_consumer_rate: Option<u32>,
        fairness: ShareFairness,
        lease_deadline_ms: Option<u64>,
    ) -> Result<Self, ResourceExportContractError> {
        if invalid_optional_u32(max_consumers, MAX_RESOURCE_EXPORT_CONSUMERS)
            || invalid_optional_u32(per_consumer_rate, MAX_RESOURCE_EXPORT_RATE)
            || invalid_optional_u64(lease_deadline_ms, MAX_RESOURCE_EXPORT_LEASE_DEADLINE_MS)
        {
            return Err(ResourceExportContractError::PolicyInvalid);
        }

        const fn invalid_optional_u32(value: Option<u32>, maximum: u32) -> bool {
            match value {
                Some(value) => value == 0 || value > maximum,
                None => false,
            }
        }

        const fn invalid_optional_u64(value: Option<u64>, maximum: u64) -> bool {
            match value {
                Some(value) => value == 0 || value > maximum,
                None => false,
            }
        }
        Ok(Self {
            max_consumers,
            per_consumer_rate,
            fairness,
            lease_deadline_ms,
        })
    }

    /// Return the optional maximum active-consumer count.
    pub const fn max_consumers(self) -> Option<u32> {
        self.max_consumers
    }

    /// Return the optional per-consumer rate ceiling.
    pub const fn per_consumer_rate(self) -> Option<u32> {
        self.per_consumer_rate
    }

    /// Return the fairness policy.
    pub const fn fairness(self) -> ShareFairness {
        self.fairness
    }

    /// Return the optional lease deadline.
    pub const fn lease_deadline_ms(self) -> Option<u64> {
        self.lease_deadline_ms
    }
}

wire_deserialize!(
    ShareQuota,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        #[serde(default)]
        max_consumers: Option<u32>,
        #[serde(default)]
        per_consumer_rate: Option<u32>,
        #[serde(default)]
        fairness: ShareFairness,
        #[serde(default)]
        lease_deadline_ms: Option<u64>,
    },
    wire,
    Self::new(
        wire.max_consumers,
        wire.per_consumer_rate,
        wire.fairness,
        wire.lease_deadline_ms,
    )
    .map_err(serde::de::Error::custom)
);

/// Consumer-Zone and capability ceiling carried by an export.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ConsumerZonePolicy {
    zones: Vec<ZoneId>,
    capability_ceiling: Vec<BoundedToken>,
}

impl ConsumerZonePolicy {
    /// Construct a bounded, canonicalized consumer policy.
    pub fn new(
        mut zones: Vec<ZoneId>,
        mut capability_ceiling: Vec<BoundedToken>,
    ) -> Result<Self, ResourceExportContractError> {
        if zones.len() > MAX_RESOURCE_EXPORT_ZONES
            || capability_ceiling.len() > MAX_RESOURCE_EXPORT_CAPABILITIES
        {
            return Err(ResourceExportContractError::BoundExceeded);
        }
        zones.sort();
        capability_ceiling.sort();
        if zones.windows(2).any(|pair| pair[0] == pair[1])
            || capability_ceiling.windows(2).any(|pair| pair[0] == pair[1])
        {
            return Err(ResourceExportContractError::DuplicateEntry);
        }
        Ok(Self {
            zones,
            capability_ceiling,
        })
    }

    /// Borrow the explicitly allowed consumer Zones.
    pub fn zones(&self) -> &[ZoneId] {
        &self.zones
    }

    /// Borrow the closed capability ceiling.
    pub fn capability_ceiling(&self) -> &[BoundedToken] {
        &self.capability_ceiling
    }

    /// Whether a capability is inside the export ceiling.
    pub fn allows_capability(&self, capability: &BoundedToken) -> bool {
        self.capability_ceiling.binary_search(capability).is_ok()
    }

    /// Whether one Zone is explicitly named by this export's policy.
    ///
    /// This is the list half only. [`ResourceExportSpec::admits_consumer_zone`]
    /// combines it with the export's visibility, because a child-Zone export
    /// deliberately names no Zone and is bounded by the Zone tree instead.
    pub fn names_zone(&self, zone: &ZoneId) -> bool {
        self.zones.binary_search(zone).is_ok()
    }
}

impl core::fmt::Debug for ConsumerZonePolicy {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ConsumerZonePolicy")
            .field("zone_count", &self.zones.len())
            .field("capability_count", &self.capability_ceiling.len())
            .finish()
    }
}

wire_deserialize!(
    ConsumerZonePolicy,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        #[serde(default)]
        zones: Vec<ZoneId>,
        #[serde(default)]
        capability_ceiling: Vec<BoundedToken>,
    },
    wire,
    Self::new(wire.zones, wire.capability_ceiling).map_err(serde::de::Error::custom)
);

/// Export deletion and ZoneLink-loss policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RevocationPolicy {
    grace_period_ms: u64,
    force_revoke: bool,
}

impl RevocationPolicy {
    /// Construct a bounded revocation policy.
    pub const fn new(
        grace_period_ms: u64,
        force_revoke: bool,
    ) -> Result<Self, ResourceExportContractError> {
        if grace_period_ms > MAX_RESOURCE_EXPORT_REVOCATION_GRACE_MS {
            return Err(ResourceExportContractError::PolicyInvalid);
        }
        Ok(Self {
            grace_period_ms,
            force_revoke,
        })
    }

    /// Return the bounded grace period.
    pub const fn grace_period_ms(self) -> u64 {
        self.grace_period_ms
    }

    /// Whether the controller may force revoke after the grace period.
    pub const fn force_revoke(self) -> bool {
        self.force_revoke
    }
}

wire_deserialize!(
    RevocationPolicy,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        #[serde(default)]
        grace_period_ms: u64,
        #[serde(default)]
        force_revoke: bool,
    },
    wire,
    Self::new(wire.grace_period_ms, wire.force_revoke).map_err(serde::de::Error::custom)
);

/// The provider-neutral desired state of one owner-Zone export.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceExportSpec {
    resource_ref: ResourceRef,
    service_type: ResourceTypeName,
    projection_schema_fingerprint: SchemaFingerprint,
    factory_fingerprint: SchemaFingerprint,
    operations: Vec<BoundedToken>,
    arbitration: ExportArbitration,
    quota: ShareQuota,
    consumer_zone_policy: ConsumerZonePolicy,
    #[serde(default = "default_export_visibility")]
    visibility: ExportVisibility,
    revocation_policy: RevocationPolicy,
}

impl ResourceExportSpec {
    /// Construct and validate an export's provider-neutral fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        resource_ref: ResourceRef,
        service_type: ResourceTypeName,
        projection_schema_fingerprint: SchemaFingerprint,
        factory_fingerprint: SchemaFingerprint,
        mut operations: Vec<BoundedToken>,
        arbitration: ExportArbitration,
        quota: ShareQuota,
        consumer_zone_policy: ConsumerZonePolicy,
        visibility: ExportVisibility,
        revocation_policy: RevocationPolicy,
    ) -> Result<Self, ResourceExportContractError> {
        if !is_qualified_service_type(&service_type) {
            return Err(ResourceExportContractError::ServiceTypeInvalid);
        }
        if resource_ref.resource_type() != &service_type {
            return Err(ResourceExportContractError::WrongResourceType);
        }
        if operations.len() > MAX_RESOURCE_EXPORT_OPERATIONS {
            return Err(ResourceExportContractError::BoundExceeded);
        }
        operations.sort();
        if operations.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ResourceExportContractError::DuplicateEntry);
        }
        if visibility == ExportVisibility::NamedZones && consumer_zone_policy.zones().is_empty() {
            return Err(ResourceExportContractError::PolicyInvalid);
        }
        if operations
            .iter()
            .any(|operation| !consumer_zone_policy.allows_capability(operation))
        {
            return Err(ResourceExportContractError::PolicyInvalid);
        }
        Ok(Self {
            resource_ref,
            service_type,
            projection_schema_fingerprint,
            factory_fingerprint,
            operations,
            arbitration,
            quota,
            consumer_zone_policy,
            visibility,
            revocation_policy,
        })
    }

    /// Construct an export with the normative empty optional policies.
    pub fn minimal(
        resource_ref: ResourceRef,
        service_type: ResourceTypeName,
        projection_schema_fingerprint: SchemaFingerprint,
        factory_fingerprint: SchemaFingerprint,
        operations: Vec<BoundedToken>,
        arbitration: ExportArbitration,
        consumer_zone_policy: ConsumerZonePolicy,
    ) -> Result<Self, ResourceExportContractError> {
        Self::new(
            resource_ref,
            service_type,
            projection_schema_fingerprint,
            factory_fingerprint,
            operations,
            arbitration,
            ShareQuota::default(),
            consumer_zone_policy,
            ExportVisibility::ChildZones,
            RevocationPolicy::default(),
        )
    }

    /// Borrow the local owner Service reference.
    pub const fn resource_ref(&self) -> &ResourceRef {
        &self.resource_ref
    }

    /// Borrow the exact qualified Service type.
    pub const fn service_type(&self) -> &ResourceTypeName {
        &self.service_type
    }

    /// Borrow the expected projection schema fingerprint.
    pub const fn projection_schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.projection_schema_fingerprint
    }

    /// Borrow the expected semantic factory fingerprint.
    pub const fn factory_fingerprint(&self) -> &SchemaFingerprint {
        &self.factory_fingerprint
    }

    /// Borrow the canonical operation ceiling.
    pub fn operations(&self) -> &[BoundedToken] {
        &self.operations
    }

    /// Whether one operation is advertised.
    pub fn allows_operation(&self, operation: &BoundedToken) -> bool {
        self.operations.binary_search(operation).is_ok()
    }

    /// Return the arbitration mode.
    pub const fn arbitration(&self) -> ExportArbitration {
        self.arbitration
    }

    /// Borrow the quota policy.
    pub const fn quota(&self) -> ShareQuota {
        self.quota
    }

    /// Borrow the consumer-Zone policy.
    pub const fn consumer_zone_policy(&self) -> &ConsumerZonePolicy {
        &self.consumer_zone_policy
    }

    /// Return the visibility scope.
    pub const fn visibility(&self) -> ExportVisibility {
        self.visibility
    }

    /// Borrow the revocation policy.
    pub const fn revocation_policy(&self) -> RevocationPolicy {
        self.revocation_policy
    }

    /// Validate the identity and Service-only shape of a stored target.
    ///
    /// Origin is intentionally checked by
    /// [`ProjectionFactory::admits_export_target`], which receives the stored
    /// envelope and can return the dedicated import-owned error.
    pub fn validate_target(
        &self,
        target: &ResourceEnvelope,
    ) -> Result<(), ResourceExportContractError> {
        if !is_qualified_service_type(target.resource_type()) {
            return Err(ResourceExportContractError::ServiceTypeInvalid);
        }
        if target.resource_type() != &self.service_type {
            return Err(ResourceExportContractError::WrongResourceType);
        }
        if ResourceRef::from(target) != self.resource_ref {
            return Err(ResourceExportContractError::ResourceReferenceMismatch);
        }
        Ok(())
    }

    /// Admit one consumer Zone against this export's consumer policy.
    ///
    /// A `NamedZones` export admits only the Zones it lists, which is how an
    /// owner narrows a share below every child. A `ChildZones` export names no
    /// Zone and is bounded instead by the Zone tree, which the caller's own
    /// admission establishes; nothing here widens it past the named list when
    /// one exists (R7).
    pub fn admits_consumer_zone(&self, zone: &ZoneId) -> bool {
        match self.visibility {
            ExportVisibility::ChildZones => true,
            ExportVisibility::NamedZones => self.consumer_zone_policy.names_zone(zone),
        }
    }

    /// Admit the origin of one stored export subject.
    ///
    /// The subject is resolved in the owner Zone and handed over as its
    /// stored envelope, because ownership is a property of the row rather
    /// than of the reference the author typed. An import-owned projection is
    /// refused: a consumer Zone holds a lease over the source Zone's Service,
    /// not the authority to advertise it onward, so re-exporting an import is
    /// how source authority would otherwise be laundered (AE11).
    ///
    /// # Errors
    ///
    /// Returns [`ResourceExportContractError::ImportOwnedOriginRejected`] when
    /// the stored subject's owner reference names a `ResourceImport`.
    pub fn admits_export_subject_origin(
        &self,
        subject: &ResourceEnvelope,
    ) -> Result<(), ResourceExportContractError> {
        if subject
            .metadata()
            .owner_ref()
            .is_some_and(|owner| owner.resource_type().as_str() == RESOURCE_IMPORT_OWNER_TYPE)
        {
            return Err(ResourceExportContractError::ImportOwnedOriginRejected);
        }
        Ok(())
    }

    /// Validate this declaration against signed factory metadata.
    pub fn validate_factory(
        &self,
        factory: &ProjectionFactory,
    ) -> Result<(), ProviderContractError> {
        if factory.exportability() != Exportability::ExplicitExport
            || factory.service_type() != &self.service_type
            || factory.projection_schema_fingerprint() != &self.projection_schema_fingerprint
            || factory.factory_fingerprint() != &self.factory_fingerprint
        {
            return Err(if factory.exportability() == Exportability::Forbidden {
                ProviderContractError::ExportForbidden
            } else {
                ProviderContractError::ProjectionFactoryInvalid
            });
        }
        Ok(())
    }
}

impl core::fmt::Debug for ResourceExportSpec {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ResourceExportSpec")
            .field("operation_count", &self.operations.len())
            .field("arbitration", &self.arbitration)
            .field("visibility", &self.visibility)
            .finish_non_exhaustive()
    }
}

wire_deserialize!(
    ResourceExportSpec,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        resource_ref: ResourceRef,
        service_type: ResourceTypeName,
        projection_schema_fingerprint: SchemaFingerprint,
        factory_fingerprint: SchemaFingerprint,
        operations: Vec<BoundedToken>,
        arbitration: ExportArbitration,
        #[serde(default)]
        quota: ShareQuota,
        consumer_zone_policy: ConsumerZonePolicy,
        #[serde(default = "default_export_visibility")]
        visibility: ExportVisibility,
        #[serde(default)]
        revocation_policy: RevocationPolicy,
    },
    wire,
    Self::new(
        wire.resource_ref,
        wire.service_type,
        wire.projection_schema_fingerprint,
        wire.factory_fingerprint,
        wire.operations,
        wire.arbitration,
        wire.quota,
        wire.consumer_zone_policy,
        wire.visibility,
        wire.revocation_policy,
    )
    .map_err(serde::de::Error::custom)
);

/// ResourceExport lifecycle state projected into `status.resource`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ResourceExportState {
    /// The export has not yet been advertised.
    Pending,
    /// The owner Service is advertised but no lease is ready.
    Advertised,
    /// The export accepts consumer leases.
    Ready,
    /// New consumers are refused while existing leases drain.
    Revoking,
    /// The owner or route is impaired.
    Degraded,
    /// The advertisement has been withdrawn.
    Withdrawn,
}

/// Closed ResourceExport condition names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceExportConditionType {
    /// The export metadata is visible to eligible child Zones.
    ExportAdvertised,
    /// The owner Service is ready.
    AuthorityReady,
    /// At least one consumer is admitted.
    ConsumersAdmitted,
    /// The export is draining or has been revoked.
    Revoking,
}

/// Closed state for a bounded lease summary.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ExportLeaseState {
    /// The lease is waiting for arbitration.
    Pending,
    /// The lease is active.
    Active,
    /// The lease is draining.
    Revoking,
    /// The lease was revoked.
    Revoked,
}

/// Identity-free per-consumer lease summary.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ExportLeaseSummary {
    consumer_zone: ZoneId,
    capability_count: u32,
    lease_state: ExportLeaseState,
    lease_id_digest: BindingDigest,
}

impl ExportLeaseSummary {
    /// Construct a redacted, bounded lease summary.
    pub const fn new(
        consumer_zone: ZoneId,
        capability_count: u32,
        lease_state: ExportLeaseState,
        lease_id_digest: BindingDigest,
    ) -> Self {
        Self {
            consumer_zone,
            capability_count,
            lease_state,
            lease_id_digest,
        }
    }

    /// Borrow the consumer Zone.
    pub const fn consumer_zone(&self) -> &ZoneId {
        &self.consumer_zone
    }

    /// Return the number of capabilities in the lease.
    pub const fn capability_count(&self) -> u32 {
        self.capability_count
    }

    /// Return the lease state.
    pub const fn lease_state(&self) -> ExportLeaseState {
        self.lease_state
    }

    /// Borrow the opaque lease digest.
    pub const fn lease_id_digest(&self) -> &BindingDigest {
        &self.lease_id_digest
    }
}

redacted_debug!(ExportLeaseSummary);

/// One stage of a revoking export's drain.
///
/// The stages are separate rather than collapsed into "draining" because the
/// ordering is the requirement: new use is blocked and outstanding lease use
/// is driven to its declared safe state first, and only then is the
/// advertisement withdrawn and the export row retired. The protected
/// resources - the owner Service and the bindings that realize its backing -
/// belong to the owner Zone and are never touched by the drain itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ExportRevocationStage {
    /// New leases are refused and every outstanding lease is being released.
    PreDrain,
    /// No lease is outstanding, so the advertisement may be withdrawn.
    AdvertisementWithdrawn,
    /// Both stages are evidenced and the export row may retire.
    Complete,
}

/// A revocation always begins fenced: constructing one is the act of
/// refusing new leases, so no separate caller step can forget to do it.
pub struct ExportRevocation {
    policy: RevocationPolicy,
    stage: ExportRevocationStage,
    outstanding_leases: u32,
}

impl ExportRevocation {
    /// Begin revoking one export.
    ///
    /// The returned value blocks new use from this point; the owner Zone
    /// observes the leases it actually holds through
    /// [`ExportRevocation::observe_leases`].
    pub fn begin(spec: &ResourceExportSpec) -> Self {
        Self {
            policy: spec.revocation_policy(),
            stage: ExportRevocationStage::PreDrain,
            outstanding_leases: 0,
        }
    }

    /// Whether new consumer use is blocked.
    ///
    /// Always true. It is stated rather than inferred so a caller cannot read
    /// a draining export as still accepting leases.
    pub const fn blocks_new_use(&self) -> bool {
        true
    }

    /// The furthest stage this revocation has evidenced.
    pub const fn stage(&self) -> ExportRevocationStage {
        self.stage
    }

    /// The number of leases still holding the export.
    pub const fn outstanding_leases(&self) -> u32 {
        self.outstanding_leases
    }

    /// Whether the export row may retire.
    pub const fn is_complete(&self) -> bool {
        matches!(self.stage, ExportRevocationStage::Complete)
    }

    /// Whether the declared policy permits forcing the drain now.
    ///
    /// Forcing is a controller decision taken after the grace period, and it
    /// is refused when the policy did not ask for it, so a slow lease can never
    /// be cut off by a policy that promised it time.
    pub const fn force_permitted(&self, elapsed_ms: u64) -> bool {
        self.policy.force_revoke() && elapsed_ms >= self.policy.grace_period_ms()
    }

    /// Observe the export's leases and return the stage to perform next.
    ///
    /// A lease that is still `Active` or `Revoking` counts as outstanding, so
    /// the advertisement cannot be withdrawn while a consumer is still using
    /// the shared authority. A lease that never became active has nothing to
    /// release and does not hold the export open.
    pub fn observe_leases(&mut self, leases: &[ExportLeaseSummary]) -> ExportRevocationStage {
        self.outstanding_leases = leases
            .iter()
            .filter(|lease| {
                matches!(
                    lease.lease_state(),
                    ExportLeaseState::Active | ExportLeaseState::Revoking
                )
            })
            .count()
            .try_into()
            .unwrap_or(u32::MAX);
        self.stage = match self.stage {
            ExportRevocationStage::PreDrain if self.outstanding_leases == 0 => {
                ExportRevocationStage::AdvertisementWithdrawn
            }
            ExportRevocationStage::AdvertisementWithdrawn => ExportRevocationStage::Complete,
            reached => reached,
        };
        self.stage
    }
}

impl core::fmt::Debug for ExportRevocation {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ExportRevocation")
            .field("stage", &self.stage)
            .field("outstanding_leases", &self.outstanding_leases)
            .finish_non_exhaustive()
    }
}

/// Whether a ResourceType is a qualified semantic Service.
pub(crate) fn is_qualified_service_type(resource_type: &ResourceTypeName) -> bool {
    let value = resource_type.as_str();
    value.contains(".d2bus.org.")
        && value
            .rsplit('.')
            .next()
            .is_some_and(|name| name.ends_with("Service"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(digit: char) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", digit.to_string().repeat(64))).unwrap()
    }

    fn service_ref() -> ResourceRef {
        ResourceRef::parse("audio.d2bus.org.AudioService/mic").unwrap()
    }

    #[test]
    fn export_spec_is_strict_and_canonicalizes_lists() {
        let spec = ResourceExportSpec::minimal(
            service_ref(),
            ResourceTypeName::parse("audio.d2bus.org.AudioService").unwrap(),
            fingerprint('a'),
            fingerprint('b'),
            vec![
                BoundedToken::parse("capture").unwrap(),
                BoundedToken::parse("observe").unwrap(),
            ],
            ExportArbitration::Exclusive,
            ConsumerZonePolicy::new(
                vec![ZoneId::parse("child").unwrap()],
                vec![
                    BoundedToken::parse("capture").unwrap(),
                    BoundedToken::parse("observe").unwrap(),
                ],
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(spec.operations()[0].as_str(), "capture");
        let wire = serde_json::to_value(&spec).unwrap();
        let parsed: ResourceExportSpec = serde_json::from_value(wire).unwrap();
        assert_eq!(parsed, spec);
    }

    #[test]
    fn service_only_and_duplicate_controls_are_live() {
        let wrong_ref = ResourceRef::parse("Device/mic").unwrap();
        assert_eq!(
            ResourceExportSpec::minimal(
                wrong_ref,
                ResourceTypeName::parse("audio.d2bus.org.AudioService").unwrap(),
                fingerprint('a'),
                fingerprint('b'),
                Vec::new(),
                ExportArbitration::Exclusive,
                ConsumerZonePolicy::new(Vec::new(), Vec::new()).unwrap(),
            ),
            Err(ResourceExportContractError::WrongResourceType)
        );
        let duplicate = ResourceExportSpec::minimal(
            service_ref(),
            ResourceTypeName::parse("audio.d2bus.org.AudioService").unwrap(),
            fingerprint('a'),
            fingerprint('b'),
            vec![
                BoundedToken::parse("capture").unwrap(),
                BoundedToken::parse("capture").unwrap(),
            ],
            ExportArbitration::Exclusive,
            ConsumerZonePolicy::new(Vec::new(), Vec::new()).unwrap(),
        );
        assert_eq!(duplicate, Err(ResourceExportContractError::DuplicateEntry));
    }

    #[test]
    fn policy_bounds_and_redaction_are_enforced() {
        assert_eq!(
            ShareQuota::new(Some(0), None, ShareFairness::Fifo, None),
            Err(ResourceExportContractError::PolicyInvalid)
        );
        let policy = ConsumerZonePolicy::new(
            vec![ZoneId::parse("child").unwrap()],
            vec![BoundedToken::parse("capture").unwrap()],
        )
        .unwrap();
        let rendered = format!("{policy:?}");
        assert!(!rendered.contains("child"));
        assert!(!rendered.contains("capture"));
    }
}
