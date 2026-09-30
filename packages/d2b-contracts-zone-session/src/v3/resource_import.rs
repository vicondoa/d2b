//! The provider-neutral `ResourceImport` contract.
//!
//! An import is a child-Zone route to one remote `ResourceExport`.  It names
//! only the local ZoneLink and an opaque export key.  Core later materializes
//! one same-qualified-type Service projection; this contract never carries a
//! remote ResourceRef, a transport locator, a descriptor, or a grant.
//!
//! The projection is a lease over the owner Zone's admitted semantic service,
//! never a local copy of what backs it.  [`ResourceImportSpec::admit_consumer_use`]
//! is the single decision that keeps it that way: the family shape is checked
//! by the signed catalog, then the exporting Zone's consumer policy,
//! capability ceiling, quota, and lease bound the result.  Nothing in this
//! module can name a `Volume`, `Device`, `Network`, `Endpoint`, `Credential`,
//! or primitive binding, so none can be produced by importing.

use std::collections::BTreeSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::resource_export::{ExportLeaseState, ResourceExportSpec, ShareQuota, is_qualified_service_type};
use d2b_contracts::wire_deserialize;
use d2b_contracts_provider::v3::provider::{
    Exportability, ProjectionFactory, ProviderContractError,
};
use d2b_contracts_provider::v3::semantic_services::{
    AdmittedImportUse, ImportUseRefusal, ImportUseRequest, SemanticPairContract,
};
use d2b_contracts_resource::v3::{
    ResourceName, ResourceRef, ResourceTypeName, SchemaFingerprint, ZoneId,
    execution_policy::{BoundedText, BoundedToken, PrimitiveSpecError},
};

/// The canonical ResourceType name for an import declaration.
pub const RESOURCE_IMPORT_RESOURCE_TYPE: &str = "ResourceImport";
/// The Core finalizer used while an import's projection and Bindings drain.
pub const RESOURCE_IMPORT_DRAIN_FINALIZER: &str = "core.resource-import-drain";
/// Maximum bytes in an opaque export key.
pub const MAX_RESOURCE_IMPORT_EXPORT_KEY_BYTES: usize = 128;
/// Maximum requested capabilities in one import.
pub const MAX_RESOURCE_IMPORT_CAPABILITIES: usize = 64;
/// Maximum active lease summaries represented by one import status.
pub const MAX_RESOURCE_IMPORT_LEASE_COUNT: u32 = 256;

/// Closed validation failures for the `ResourceImport` base contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceImportContractError {
    /// A required field was absent.
    MissingRequiredField,
    /// The route reference named the wrong ResourceType.
    WrongResourceType,
    /// The expected type is not a qualified semantic Service.
    ServiceTypeInvalid,
    /// The opaque export key was empty or malformed.
    ExportKeyInvalid,
    /// A requested capability was invalid or duplicated.
    InvalidCapability,
    /// A bounded collection was too large.
    BoundExceeded,
    /// A list that must be unique contained a duplicate.
    DuplicateEntry,
    /// The requested quota was outside the export quota.
    QuotaInvalid,
    /// Expected Service or fingerprint metadata did not match.
    FingerprintMismatch,
    /// The requesting Zone is not inside the export's consumer policy.
    ConsumerZoneNotAdmitted,
    /// The export's lease no longer admits new use, because it is draining or
    /// has been revoked.
    LeaseNotUsable,
    /// A requested capability is outside the lease the owner Zone granted.
    CapabilityNotLeased,
    /// The request is not an admitted use of the projection's semantic
    /// family, or it is outside what an import may consume at all.
    ///
    /// The family-level reasons are re-exported verbatim from the semantic
    /// catalog, so a refusal names which boundary stopped the request rather
    /// than reporting one opaque import error.
    SemanticFamilyRefusal(ImportUseRefusal),
}

impl ResourceImportContractError {
    /// Return the stable identity-free diagnostic label.
    pub const fn code(self) -> &'static str {
        match self {
            Self::MissingRequiredField => "resource-import-required-field-missing",
            Self::WrongResourceType => "resource-import-reference-type-invalid",
            Self::ServiceTypeInvalid => "resource-import-service-type-invalid",
            Self::ExportKeyInvalid => "resource-import-export-key-invalid",
            Self::InvalidCapability => "resource-import-capability-invalid",
            Self::BoundExceeded => "resource-import-bound-exceeded",
            Self::DuplicateEntry => "resource-import-duplicate-entry",
            Self::QuotaInvalid => "resource-import-quota-invalid",
            Self::FingerprintMismatch => "resource-import-fingerprint-mismatch",
            Self::ConsumerZoneNotAdmitted => "resource-import-consumer-zone-not-admitted",
            Self::LeaseNotUsable => "resource-import-lease-not-usable",
            Self::CapabilityNotLeased => "resource-import-capability-not-leased",
            Self::SemanticFamilyRefusal(refusal) => refusal.as_str(),
        }
    }
}

impl core::fmt::Display for ResourceImportContractError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ResourceImportContractError {}

impl From<PrimitiveSpecError> for ResourceImportContractError {
    fn from(_error: PrimitiveSpecError) -> Self {
        Self::InvalidCapability
    }
}

/// Disconnect behavior for a local projection Service.
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
pub enum ImportDisconnectPolicy {
    /// Keep the projection and mark it degraded until reconnect.
    #[default]
    Degrade,
    /// Tear down the projection after the route is lost.
    Teardown,
}

/// ResourceImport lifecycle state projected into `status.resource`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum ResourceImportState {
    /// The import has not reached the owner Zone.
    Pending,
    /// The export advertisement is reachable.
    Reachable,
    /// A lease and local projection are bound.
    Bound,
    /// The route or projection is impaired.
    Degraded,
    /// The remote lease was revoked.
    Revoked,
}

/// Closed ResourceImport condition names.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum ResourceImportConditionType {
    /// The remote export was found through the ZoneLink.
    ExportReachable,
    /// The remote and local factories agree.
    FactoryMatched,
    /// The projection schema fingerprints agree.
    SchemaMatched,
    /// A remote lease was admitted.
    Bound,
    /// The local projection Service is ready.
    ProjectionReady,
    /// Authored Bindings still reference this import's projection.
    BindingReferencesRemain,
    /// The route or lease is degraded.
    Degraded,
}

/// The provider-neutral desired state of one child-Zone import.
#[derive(Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ResourceImportSpec {
    zone_link_ref: ResourceRef,
    export_key: BoundedText,
    expected_service_type: ResourceTypeName,
    expected_projection_schema_fingerprint: SchemaFingerprint,
    expected_factory_fingerprint: SchemaFingerprint,
    projection_name: ResourceName,
    requested_capabilities: Vec<BoundedToken>,
    requested_quota: ShareQuota,
    disconnect_policy: ImportDisconnectPolicy,
}

impl ResourceImportSpec {
    /// Construct and validate an import's provider-neutral fields.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        zone_link_ref: ResourceRef,
        export_key: BoundedText,
        expected_service_type: ResourceTypeName,
        expected_projection_schema_fingerprint: SchemaFingerprint,
        expected_factory_fingerprint: SchemaFingerprint,
        projection_name: ResourceName,
        mut requested_capabilities: Vec<BoundedToken>,
        requested_quota: ShareQuota,
        disconnect_policy: ImportDisconnectPolicy,
    ) -> Result<Self, ResourceImportContractError> {
        if zone_link_ref.resource_type().as_str() != "ZoneLink" {
            return Err(ResourceImportContractError::WrongResourceType);
        }
        if export_key.as_str().is_empty()
            || export_key.as_str().len() > MAX_RESOURCE_IMPORT_EXPORT_KEY_BYTES
        {
            return Err(ResourceImportContractError::ExportKeyInvalid);
        }
        if !is_qualified_service_type(&expected_service_type) {
            return Err(ResourceImportContractError::ServiceTypeInvalid);
        }
        if requested_capabilities.len() > MAX_RESOURCE_IMPORT_CAPABILITIES {
            return Err(ResourceImportContractError::BoundExceeded);
        }
        requested_capabilities.sort();
        if requested_capabilities
            .windows(2)
            .any(|pair| pair[0] == pair[1])
        {
            return Err(ResourceImportContractError::DuplicateEntry);
        }
        Ok(Self {
            zone_link_ref,
            export_key,
            expected_service_type,
            expected_projection_schema_fingerprint,
            expected_factory_fingerprint,
            projection_name,
            requested_capabilities,
            requested_quota,
            disconnect_policy,
        })
    }

    /// Construct an import with default quota and degrade-on-disconnect policy.
    pub fn minimal(
        zone_link_ref: ResourceRef,
        export_key: impl Into<String>,
        expected_service_type: ResourceTypeName,
        expected_projection_schema_fingerprint: SchemaFingerprint,
        expected_factory_fingerprint: SchemaFingerprint,
        projection_name: ResourceName,
        requested_capabilities: Vec<BoundedToken>,
    ) -> Result<Self, ResourceImportContractError> {
        let export_key = BoundedText::parse(export_key.into())
            .map_err(|_| ResourceImportContractError::ExportKeyInvalid)?;
        Self::new(
            zone_link_ref,
            export_key,
            expected_service_type,
            expected_projection_schema_fingerprint,
            expected_factory_fingerprint,
            projection_name,
            requested_capabilities,
            ShareQuota::default(),
            ImportDisconnectPolicy::Degrade,
        )
    }

    /// Borrow the local ZoneLink route reference.
    pub const fn zone_link_ref(&self) -> &ResourceRef {
        &self.zone_link_ref
    }

    /// Borrow the opaque export key.
    pub const fn export_key(&self) -> &BoundedText {
        &self.export_key
    }

    /// Borrow the expected qualified Service type.
    pub const fn expected_service_type(&self) -> &ResourceTypeName {
        &self.expected_service_type
    }

    /// Borrow the expected projection schema fingerprint.
    pub const fn expected_projection_schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.expected_projection_schema_fingerprint
    }

    /// Borrow the expected semantic factory fingerprint.
    pub const fn expected_factory_fingerprint(&self) -> &SchemaFingerprint {
        &self.expected_factory_fingerprint
    }

    /// Borrow the stable local projection name.
    pub const fn projection_name(&self) -> &ResourceName {
        &self.projection_name
    }

    /// Borrow the requested capability set.
    pub fn requested_capabilities(&self) -> &[BoundedToken] {
        &self.requested_capabilities
    }

    /// Return whether one capability was requested.
    pub fn requests_capability(&self, capability: &BoundedToken) -> bool {
        self.requested_capabilities
            .binary_search(capability)
            .is_ok()
    }

    /// Borrow the requested quota.
    pub const fn requested_quota(&self) -> ShareQuota {
        self.requested_quota
    }

    /// Return the disconnect policy.
    pub const fn disconnect_policy(&self) -> ImportDisconnectPolicy {
        self.disconnect_policy
    }

    /// Return the local same-qualified-type projection reference.
    pub fn projection_service_ref(&self) -> ResourceRef {
        ResourceRef::new(
            self.expected_service_type.clone(),
            self.projection_name.clone(),
        )
    }

    /// Validate expected metadata and requested capabilities against an export.
    pub fn validate_against_export(
        &self,
        export: &ResourceExportSpec,
    ) -> Result<(), ResourceImportContractError> {
        if self.expected_service_type != *export.service_type()
            || self.expected_projection_schema_fingerprint
                != *export.projection_schema_fingerprint()
            || self.expected_factory_fingerprint != *export.factory_fingerprint()
        {
            return Err(ResourceImportContractError::FingerprintMismatch);
        }
        if self
            .requested_capabilities
            .iter()
            .any(|capability| !export.allows_operation(capability))
        {
            return Err(ResourceImportContractError::InvalidCapability);
        }
        if !quota_fits(self.requested_quota, export.quota()) {
            return Err(ResourceImportContractError::QuotaInvalid);
        }
        Ok(())
    }

    /// Validate this import against signed local factory metadata.
    pub fn validate_factory(
        &self,
        factory: &ProjectionFactory,
    ) -> Result<(), ProviderContractError> {
        if factory.exportability() != Exportability::ExplicitExport
            || factory.service_type() != &self.expected_service_type
            || factory.projection_schema_fingerprint()
                != &self.expected_projection_schema_fingerprint
            || factory.factory_fingerprint() != &self.expected_factory_fingerprint
        {
            return Err(if factory.exportability() == Exportability::Forbidden {
                ProviderContractError::ExportForbidden
            } else {
                ProviderContractError::ProjectionFactoryInvalid
            });
        }
        Ok(())
    }

    /// Admit one importing-Zone consumer's use of this import's projection.
    ///
    /// `projection_owner` is the owner reference of the stored projection row
    /// as resolved in the importing Zone, and `lease` is the export-side lease
    /// the owner Zone granted this consumer. Both are observations, never
    /// caller claims: the projection must be owned by an import, and the lease
    /// must already name a consumer Zone the export's policy admits.
    ///
    /// The decision runs in the only order that cannot widen an import. The
    /// family shape comes first, so a request that names a backing resource or
    /// a local physical effect is refused by name; the export's consumer
    /// policy, capability ceiling, quota, and lease then narrow what remains.
    /// A capability the lease does not carry is refused rather than dropped,
    /// because a silently narrowed capability set is how an over-ceiling
    /// request would come to look admitted (AE11, R7, R16).
    ///
    /// # Errors
    ///
    /// Returns [`ResourceImportContractError::ConsumerZoneNotAdmitted`] when
    /// the lease's consumer Zone is outside the export's consumer policy,
    /// [`ResourceImportContractError::LeaseNotUsable`] when the lease is
    /// draining or revoked,
    /// [`ResourceImportContractError::CapabilityNotLeased`] when a requested
    /// capability is outside the lease, the export's operations, or the
    /// import's own requested set, and
    /// [`ResourceImportContractError::SemanticFamilyRefusal`] with the
    /// catalog's own reason when the request is not an admitted use of the
    /// projection.
    pub fn admit_consumer_use(
        &self,
        pair: &SemanticPairContract,
        export: &ResourceExportSpec,
        lease: &ImportLeaseClaim,
        projection_owner: Option<&ResourceRef>,
        request: &ImportUseRequest,
    ) -> Result<AdmittedImportUse, ResourceImportContractError> {
        let family_use =
            pair.admit_import_use(projection_owner, request)
                .map_err(ResourceImportContractError::SemanticFamilyRefusal)?;
        self.validate_against_export(export)?;
        if !export.admits_consumer_zone(lease.consumer_zone()) {
            return Err(ResourceImportContractError::ConsumerZoneNotAdmitted);
        }
        if !lease.admits_new_use() {
            return Err(ResourceImportContractError::LeaseNotUsable);
        }
        let requested = family_use.capabilities();
        if !requested
            .iter()
            .all(|capability| self.requests_capability(capability))
        {
            return Err(ResourceImportContractError::CapabilityNotLeased);
        }
        let narrowed = family_use.narrowed(lease.capabilities());
        if narrowed.capabilities().len() != requested.len() {
            return Err(ResourceImportContractError::CapabilityNotLeased);
        }
        Ok(narrowed)
    }
 }

/// The export-side lease one importing consumer's use runs under.
///
/// This is an observation of what the owner Zone actually granted, not a
/// request. Its capabilities are the grant: an importing consumer can use no
/// more than the lease carries, however much the export's ceiling would allow.
#[derive(Clone, PartialEq, Eq)]
pub struct ImportLeaseClaim {
    consumer_zone: ZoneId,
    capabilities: BTreeSet<BoundedToken>,
    state: ExportLeaseState,
}

impl ImportLeaseClaim {
    /// Construct a lease observation from the owner Zone's own admission.
    ///
    /// # Errors
    ///
    /// Returns [`ResourceImportContractError::BoundExceeded`] when more than
    /// [`MAX_RESOURCE_IMPORT_CAPABILITIES`] capabilities are carried and
    /// [`ResourceImportContractError::DuplicateEntry`] when one is carried
    /// twice. A duplicate fails here so the lease and its admitted use always
    /// agree on the set.
    pub fn new(
        consumer_zone: ZoneId,
        mut capabilities: Vec<BoundedToken>,
        state: ExportLeaseState,
    ) -> Result<Self, ResourceImportContractError> {
        if capabilities.len() > MAX_RESOURCE_IMPORT_CAPABILITIES {
            return Err(ResourceImportContractError::BoundExceeded);
        }
        capabilities.sort();
        if capabilities.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ResourceImportContractError::DuplicateEntry);
        }
        Ok(Self {
            consumer_zone,
            capabilities: capabilities.into_iter().collect(),
            state,
        })
    }

    /// Borrow the Zone the owner Zone admitted this lease for.
    pub const fn consumer_zone(&self) -> &ZoneId {
        &self.consumer_zone
    }

    /// Borrow the granted capability set.
    pub const fn capabilities(&self) -> &BTreeSet<BoundedToken> {
        &self.capabilities
    }

    /// Whether one capability is inside the grant.
    pub fn admits_capability(&self, capability: &BoundedToken) -> bool {
        self.capabilities.contains(capability)
    }

    /// Whether this lease still admits new use.
    ///
    /// A lease that is draining or revoked admits none. This is the fence the
    /// owner's revocation raises first, so a consumer that reconnects after a
    /// revocation cannot pick up where it left off.
    pub const fn admits_new_use(&self) -> bool {
        matches!(self.state, ExportLeaseState::Pending | ExportLeaseState::Active)
    }

    /// Return the observed lease state.
    pub const fn state(&self) -> ExportLeaseState {
        self.state
    }
}

impl core::fmt::Debug for ImportLeaseClaim {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ImportLeaseClaim")
            .field("consumer_zone", &self.consumer_zone)
            .field("capability_count", &self.capabilities.len())
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl core::fmt::Debug for ResourceImportSpec {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ResourceImportSpec")
            .field(
                "requested_capability_count",
                &self.requested_capabilities.len(),
            )
            .field("disconnect_policy", &self.disconnect_policy)
            .finish_non_exhaustive()
    }
}

wire_deserialize!(
    ResourceImportSpec,
    #[serde(rename_all = "camelCase", deny_unknown_fields)]
    Wire {
        zone_link_ref: ResourceRef,
        export_key: BoundedText,
        expected_service_type: ResourceTypeName,
        expected_projection_schema_fingerprint: SchemaFingerprint,
        expected_factory_fingerprint: SchemaFingerprint,
        projection_name: ResourceName,
        requested_capabilities: Vec<BoundedToken>,
        #[serde(default)]
        requested_quota: ShareQuota,
        #[serde(default)]
        disconnect_policy: ImportDisconnectPolicy,
    },
    wire,
    Self::new(
        wire.zone_link_ref,
        wire.export_key,
        wire.expected_service_type,
        wire.expected_projection_schema_fingerprint,
        wire.expected_factory_fingerprint,
        wire.projection_name,
        wire.requested_capabilities,
        wire.requested_quota,
        wire.disconnect_policy,
    )
    .map_err(serde::de::Error::custom)
);

fn quota_fits(requested: ShareQuota, exported: ShareQuota) -> bool {
    bounded_option_fits(requested.max_consumers(), exported.max_consumers())
        && bounded_option_fits(requested.per_consumer_rate(), exported.per_consumer_rate())
        && bounded_option_fits(requested.lease_deadline_ms(), exported.lease_deadline_ms())
}

fn bounded_option_fits<T: Ord>(requested: Option<T>, exported: Option<T>) -> bool {
    match (requested, exported) {
        (Some(requested), Some(exported)) => requested <= exported,
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(digit: char) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", digit.to_string().repeat(64))).unwrap()
    }

    fn import() -> ResourceImportSpec {
        ResourceImportSpec::minimal(
            ResourceRef::parse("ZoneLink/parent").unwrap(),
            "parent/mic",
            ResourceTypeName::parse("audio.d2bus.org.AudioService").unwrap(),
            fingerprint('a'),
            fingerprint('b'),
            ResourceName::parse("mic").unwrap(),
            vec![BoundedToken::parse("capture").unwrap()],
        )
        .unwrap()
    }

    fn export() -> ResourceExportSpec {
        ResourceExportSpec::minimal(
            ResourceRef::parse("audio.d2bus.org.AudioService/mic").unwrap(),
            ResourceTypeName::parse("audio.d2bus.org.AudioService").unwrap(),
            fingerprint('a'),
            fingerprint('b'),
            vec![BoundedToken::parse("capture").unwrap()],
            super::super::resource_export::ExportArbitration::Exclusive,
            super::super::resource_export::ConsumerZonePolicy::new(
                Vec::new(),
                vec![BoundedToken::parse("capture").unwrap()],
            )
            .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn import_is_local_route_and_same_type_projection() {
        let subject = import();
        assert_eq!(
            subject.projection_service_ref(),
            ResourceRef::parse("audio.d2bus.org.AudioService/mic").unwrap()
        );
        assert!(subject.validate_against_export(&export()).is_ok());
        let wire = serde_json::to_value(&subject).unwrap();
        assert!(serde_json::from_value::<ResourceImportSpec>(wire).is_ok());
    }

    #[test]
    fn import_rejects_remote_ref_shape_and_capability_or_quota_widening() {
        assert_eq!(
            ResourceImportSpec::minimal(
                ResourceRef::parse("ResourceExport/remote").unwrap(),
                "key",
                ResourceTypeName::parse("audio.d2bus.org.AudioService").unwrap(),
                fingerprint('a'),
                fingerprint('b'),
                ResourceName::parse("mic").unwrap(),
                Vec::new(),
            ),
            Err(ResourceImportContractError::WrongResourceType)
        );
        let mut subject = import();
        subject
            .requested_capabilities
            .push(BoundedToken::parse("write").unwrap());
        assert_eq!(
            subject.validate_against_export(&export()),
            Err(ResourceImportContractError::InvalidCapability)
        );
    }
}
