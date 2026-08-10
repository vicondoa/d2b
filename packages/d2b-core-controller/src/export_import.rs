//! Core admission for D096 ResourceExport and ResourceImport routing.
//!
//! This module stops at the Core/Provider seam.  It validates stored owner
//! and backing envelopes, signed factory metadata, local route references,
//! capability ceilings, and projection identity.  Projection lifecycle,
//! status persistence, lease/stream state, and Binding cleanup belong to the
//! following projection-controller surface and are intentionally not
//! implemented here.

use d2b_contracts::v3::{
    BindingTargetType, Exportability, ProjectionFactory, ProviderContractError, ResourceEnvelope,
    ResourceExportSpec, ResourceImportSpec, ResourceRef, ResourceTypeName,
    SEMANTIC_PROJECTION_PROTOCOL_VERSION, SchemaFingerprint, SemanticProjectionProtocolVersion,
};

/// Why Core refused an export or import admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportImportError {
    /// The ResourceExport base contract is invalid.
    ExportContract,
    /// The ResourceImport base contract is invalid.
    ImportContract,
    /// The target is not the exact Service named by the export.
    ResourceReferenceMismatch,
    /// The target is not a qualified semantic Service.
    ServiceTargetInvalid,
    /// A stored row is owned by a ResourceImport and cannot carry authority.
    ImportOwnedOriginRejected,
    /// The factory metadata is not catalog-consistent.
    ProjectionFactoryInvalid,
    /// The remote and local factories use different protocol versions.
    ProjectionProtocolVersionMismatch,
    /// The advertised fingerprints do not match.
    DescriptorFingerprintMismatch,
    /// The capability is not exportable.
    ExportForbidden,
    /// A backing envelope is outside the signed backing allowlist.
    BackingReferenceNotAllowed,
    /// A Binding target is outside the signed target allowlist.
    BindingTargetNotAllowed,
    /// An import requests an operation outside the export ceiling.
    CapabilityNotAllowed,
    /// A required ResourceRef names the wrong ResourceType.
    WrongResourceType,
}

impl ExportImportError {
    /// Return the stable identity-free diagnostic label.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ExportContract => "core-export-contract-invalid",
            Self::ImportContract => "core-import-contract-invalid",
            Self::ResourceReferenceMismatch => "core-resource-reference-mismatch",
            Self::ServiceTargetInvalid => "core-service-target-invalid",
            Self::ImportOwnedOriginRejected => "provider-import-owned-origin-rejected",
            Self::ProjectionFactoryInvalid => "provider-projection-factory-invalid",
            Self::ProjectionProtocolVersionMismatch => {
                "provider-projection-protocol-version-mismatch"
            }
            Self::DescriptorFingerprintMismatch => "provider-descriptor-fingerprint-mismatch",
            Self::ExportForbidden => "provider-export-forbidden",
            Self::BackingReferenceNotAllowed => "core-backing-reference-not-allowed",
            Self::BindingTargetNotAllowed => "core-binding-target-not-allowed",
            Self::CapabilityNotAllowed => "core-capability-not-allowed",
            Self::WrongResourceType => "core-reference-type-invalid",
        }
    }
}

impl core::fmt::Display for ExportImportError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ExportImportError {}

fn map_provider_error(error: ProviderContractError) -> ExportImportError {
    match error {
        ProviderContractError::ImportOwnedOriginRejected => {
            ExportImportError::ImportOwnedOriginRejected
        }
        ProviderContractError::ProjectionProtocolVersionMismatch => {
            ExportImportError::ProjectionProtocolVersionMismatch
        }
        ProviderContractError::ProjectionFactoryInvalid => {
            ExportImportError::ProjectionFactoryInvalid
        }
        ProviderContractError::DescriptorFingerprintMismatch => {
            ExportImportError::DescriptorFingerprintMismatch
        }
        ProviderContractError::ExportForbidden => ExportImportError::ExportForbidden,
        ProviderContractError::WrongResourceType => ExportImportError::WrongResourceType,
        _ => ExportImportError::ProjectionFactoryInvalid,
    }
}

/// The bounded identity Core passes to the projection lifecycle controller.
#[derive(Clone, PartialEq, Eq)]
pub struct ProjectionServiceIdentity {
    service_type: ResourceTypeName,
    projection_ref: ResourceRef,
    owner_ref: ResourceRef,
    projection_schema_fingerprint: SchemaFingerprint,
    factory_fingerprint: SchemaFingerprint,
    projection_protocol_version: SemanticProjectionProtocolVersion,
}

impl ProjectionServiceIdentity {
    /// Borrow the exact same-qualified Service type.
    pub const fn service_type(&self) -> &ResourceTypeName {
        &self.service_type
    }

    /// Borrow the local projection Service reference.
    pub const fn projection_ref(&self) -> &ResourceRef {
        &self.projection_ref
    }

    /// Borrow the ResourceImport owner reference.
    pub const fn owner_ref(&self) -> &ResourceRef {
        &self.owner_ref
    }

    /// Borrow the projection schema fingerprint.
    pub const fn projection_schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.projection_schema_fingerprint
    }

    /// Borrow the semantic factory fingerprint.
    pub const fn factory_fingerprint(&self) -> &SchemaFingerprint {
        &self.factory_fingerprint
    }

    /// Borrow the declared projection protocol version.
    pub const fn projection_protocol_version(&self) -> &SemanticProjectionProtocolVersion {
        &self.projection_protocol_version
    }
}

impl core::fmt::Debug for ProjectionServiceIdentity {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProjectionServiceIdentity")
            .field("has_service_type", &true)
            .field("has_projection_ref", &true)
            .field("has_owner_ref", &true)
            .finish_non_exhaustive()
    }
}

/// The immutable metadata admitted for an owner export.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedExport {
    service_type: ResourceTypeName,
    projection_schema_fingerprint: SchemaFingerprint,
    factory_fingerprint: SchemaFingerprint,
    projection_protocol_version: SemanticProjectionProtocolVersion,
    operation_count: usize,
}

impl AdmittedExport {
    /// Borrow the exported qualified Service type.
    pub const fn service_type(&self) -> &ResourceTypeName {
        &self.service_type
    }

    /// Borrow the projection schema fingerprint.
    pub const fn projection_schema_fingerprint(&self) -> &SchemaFingerprint {
        &self.projection_schema_fingerprint
    }

    /// Borrow the factory fingerprint.
    pub const fn factory_fingerprint(&self) -> &SchemaFingerprint {
        &self.factory_fingerprint
    }

    /// Borrow the protocol version.
    pub const fn projection_protocol_version(&self) -> &SemanticProjectionProtocolVersion {
        &self.projection_protocol_version
    }

    /// Return the number of advertised operations.
    pub const fn operation_count(&self) -> usize {
        self.operation_count
    }
}

impl core::fmt::Debug for AdmittedExport {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AdmittedExport")
            .field("operation_count", &self.operation_count)
            .finish_non_exhaustive()
    }
}

/// The immutable metadata admitted for a consumer import.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedImport {
    service_type: ResourceTypeName,
    projection_ref: ResourceRef,
    projection_schema_fingerprint: SchemaFingerprint,
    factory_fingerprint: SchemaFingerprint,
    projection_protocol_version: SemanticProjectionProtocolVersion,
    requested_capability_count: usize,
}

impl AdmittedImport {
    /// Bind the admitted metadata to the stored ResourceImport owner row.
    ///
    /// The owner reference is accepted only after Core has checked the
    /// committed row's ResourceType. No placeholder or caller-supplied
    /// origin is retained in an admission result.
    pub fn projection_identity(
        &self,
        import_ref: &ResourceRef,
    ) -> Result<ProjectionServiceIdentity, ExportImportError> {
        if import_ref.resource_type().as_str() != "ResourceImport" {
            return Err(ExportImportError::WrongResourceType);
        }
        Ok(ProjectionServiceIdentity {
            service_type: self.service_type.clone(),
            projection_ref: self.projection_ref.clone(),
            owner_ref: import_ref.clone(),
            projection_schema_fingerprint: self.projection_schema_fingerprint.clone(),
            factory_fingerprint: self.factory_fingerprint.clone(),
            projection_protocol_version: self.projection_protocol_version.clone(),
        })
    }

    /// Return the number of requested capabilities.
    pub const fn requested_capability_count(&self) -> usize {
        self.requested_capability_count
    }
}

impl core::fmt::Debug for AdmittedImport {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("AdmittedImport")
            .field(
                "requested_capability_count",
                &self.requested_capability_count,
            )
            .finish_non_exhaustive()
    }
}

/// Admit an owner export against the committed target and backing rows.
///
/// `target` and every entry in `backing` are stored envelopes.  The caller
/// cannot replace their origin with a mode or boolean.  A row owned by
/// `ResourceImport` therefore returns the dedicated
/// `ImportOwnedOriginRejected` variant.
pub fn admit_export(
    export: &ResourceExportSpec,
    target: &ResourceEnvelope,
    backing: &[&ResourceEnvelope],
    factory: &ProjectionFactory,
) -> Result<AdmittedExport, ExportImportError> {
    export
        .validate_target(target)
        .map_err(|error| match error {
            d2b_contracts::v3::ResourceExportContractError::ResourceReferenceMismatch => {
                ExportImportError::ResourceReferenceMismatch
            }
            d2b_contracts::v3::ResourceExportContractError::WrongResourceType => {
                ExportImportError::WrongResourceType
            }
            d2b_contracts::v3::ResourceExportContractError::ServiceTypeInvalid => {
                ExportImportError::ServiceTargetInvalid
            }
            _ => ExportImportError::ExportContract,
        })?;
    export
        .validate_factory(factory)
        .map_err(map_provider_error)?;
    factory
        .admits_export_target(target)
        .map_err(map_provider_error)?;
    for resource in backing {
        factory
            .admits_backing_ref(resource)
            .map_err(|error| match error {
                ProviderContractError::ImportOwnedOriginRejected => {
                    ExportImportError::ImportOwnedOriginRejected
                }
                ProviderContractError::ProjectionFactoryInvalid => {
                    ExportImportError::BackingReferenceNotAllowed
                }
                other => map_provider_error(other),
            })?;
    }
    Ok(AdmittedExport {
        service_type: export.service_type().clone(),
        projection_schema_fingerprint: export.projection_schema_fingerprint().clone(),
        factory_fingerprint: export.factory_fingerprint().clone(),
        projection_protocol_version: factory.projection_protocol_version().clone(),
        operation_count: export.operations().len(),
    })
}

/// Admit an import against the owner advertisement and both signed factories.
pub fn admit_import(
    import: &ResourceImportSpec,
    export: &ResourceExportSpec,
    remote_factory: &ProjectionFactory,
    local_factory: &ProjectionFactory,
) -> Result<AdmittedImport, ExportImportError> {
    export
        .validate_factory(remote_factory)
        .map_err(map_provider_error)?;
    import
        .validate_against_export(export)
        .map_err(|error| match error {
            d2b_contracts::v3::ResourceImportContractError::InvalidCapability => {
                ExportImportError::CapabilityNotAllowed
            }
            _ => ExportImportError::ImportContract,
        })?;
    import
        .validate_factory(local_factory)
        .map_err(map_provider_error)?;
    admit_factory_pair(remote_factory, local_factory)?;
    Ok(AdmittedImport {
        service_type: local_factory.service_type().clone(),
        projection_ref: import.projection_service_ref(),
        projection_schema_fingerprint: local_factory.projection_schema_fingerprint().clone(),
        factory_fingerprint: local_factory.factory_fingerprint().clone(),
        projection_protocol_version: local_factory.projection_protocol_version().clone(),
        requested_capability_count: import.requested_capabilities().len(),
    })
}

/// Compare remote and local signed factories in the normative order.
pub fn admit_factory_pair(
    remote: &ProjectionFactory,
    local: &ProjectionFactory,
) -> Result<(), ExportImportError> {
    let installed = SemanticProjectionProtocolVersion::parse(SEMANTIC_PROJECTION_PROTOCOL_VERSION)
        .expect("the installed projection protocol version is valid");
    if remote.projection_protocol_version() != &installed
        || local.projection_protocol_version() != &installed
    {
        return Err(ExportImportError::ProjectionProtocolVersionMismatch);
    }
    if remote.service_type() != local.service_type()
        || remote.binding_type() != local.binding_type()
        || remote.allowed_backing_ref_types() != local.allowed_backing_ref_types()
        || remote.allowed_binding_target_ref_types() != local.allowed_binding_target_ref_types()
        || remote.exportability() != local.exportability()
    {
        return Err(
            if remote.exportability() == Exportability::Forbidden
                || local.exportability() == Exportability::Forbidden
            {
                ExportImportError::ExportForbidden
            } else {
                ExportImportError::ProjectionFactoryInvalid
            },
        );
    }
    if remote.projection_schema_fingerprint() != local.projection_schema_fingerprint()
        || remote.factory_fingerprint() != local.factory_fingerprint()
    {
        return Err(ExportImportError::DescriptorFingerprintMismatch);
    }
    Ok(())
}

/// Admit a Binding target against a signed factory's closed target set.
pub fn admit_binding_target(
    factory: &ProjectionFactory,
    target: &ResourceRef,
) -> Result<(), ExportImportError> {
    let target_type = match target.resource_type().as_str() {
        "Guest" => BindingTargetType::Guest,
        "User" => BindingTargetType::User,
        "Zone" => BindingTargetType::Zone,
        _ => return Err(ExportImportError::WrongResourceType),
    };
    if factory
        .allowed_binding_target_ref_types()
        .contains(&target_type)
    {
        Ok(())
    } else {
        Err(ExportImportError::BindingTargetNotAllowed)
    }
}

/// Bind the ResourceImport owner reference after its identity is known.
///
/// This function is the hand-off used by the projection lifecycle controller.
pub fn projection_identity(
    import_ref: &ResourceRef,
    import: &ResourceImportSpec,
    factory: &ProjectionFactory,
) -> Result<ProjectionServiceIdentity, ExportImportError> {
    if import_ref.resource_type().as_str() != "ResourceImport" {
        return Err(ExportImportError::WrongResourceType);
    }
    import
        .validate_factory(factory)
        .map_err(map_provider_error)?;
    Ok(ProjectionServiceIdentity {
        service_type: factory.service_type().clone(),
        projection_ref: import.projection_service_ref(),
        owner_ref: import_ref.clone(),
        projection_schema_fingerprint: factory.projection_schema_fingerprint().clone(),
        factory_fingerprint: factory.factory_fingerprint().clone(),
        projection_protocol_version: factory.projection_protocol_version().clone(),
    })
}

#[cfg(test)]
mod tests {
    use d2b_contracts::v3::{
        BindingTargetType, ConsumerZonePolicy, ExportArbitration, Exportability,
        ResourceExportSpec, ResourceImportSpec, ResourceName, ResourceTypeName, SchemaFingerprint,
        execution_policy::BoundedToken,
    };

    use super::*;

    fn fingerprint(digit: char) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", digit.to_string().repeat(64))).unwrap()
    }

    fn factory(backing: &[&str]) -> ProjectionFactory {
        ProjectionFactory::new(
            ResourceTypeName::parse("security-key.d2bus.org.SecurityKeyService").unwrap(),
            ResourceTypeName::parse("security-key.d2bus.org.SecurityKeyBinding").unwrap(),
            backing
                .iter()
                .map(|value| ResourceTypeName::parse(*value).unwrap()),
            [BindingTargetType::Guest, BindingTargetType::User],
            fingerprint('a'),
            fingerprint('b'),
            Exportability::ExplicitExport,
        )
        .unwrap()
    }

    fn export(factory: &ProjectionFactory) -> ResourceExportSpec {
        ResourceExportSpec::minimal(
            ResourceRef::parse("security-key.d2bus.org.SecurityKeyService/key").unwrap(),
            factory.service_type().clone(),
            factory.projection_schema_fingerprint().clone(),
            factory.factory_fingerprint().clone(),
            vec![BoundedToken::parse("use").unwrap()],
            ExportArbitration::Exclusive,
            ConsumerZonePolicy::new(
                vec![d2b_contracts::v3::ZoneId::parse("child").unwrap()],
                vec![BoundedToken::parse("use").unwrap()],
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn import(factory: &ProjectionFactory) -> ResourceImportSpec {
        ResourceImportSpec::minimal(
            ResourceRef::parse("ZoneLink/parent").unwrap(),
            "parent/key",
            factory.service_type().clone(),
            factory.projection_schema_fingerprint().clone(),
            factory.factory_fingerprint().clone(),
            ResourceName::parse("key").unwrap(),
            vec![BoundedToken::parse("use").unwrap()],
        )
        .unwrap()
    }

    fn envelope(resource_type: &str, owner_ref: Option<&str>) -> ResourceEnvelope {
        let owner_ref = owner_ref
            .map(serde_json::Value::from)
            .unwrap_or(serde_json::Value::Null);
        let value = serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": resource_type,
            "metadata": {
                "name": "key",
                "zone": "owner",
                "uid": "123e4567-e89b-42d3-a456-426614174000",
                "generation": 1,
                "revision": 1,
                "ownerRef": owner_ref,
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "2026-07-22T00:00:00.000Z",
                "updatedAt": "2026-07-22T00:00:00.000Z",
                "managedBy": "controller",
                "configurationGeneration": null,
                "controllerGeneration": null,
                "providerGeneration": null
            },
            "spec": {},
            "status": {
                "completedAt": null,
                "conditions": [],
                "lastReconciledAt": null,
                "observedGeneration": 0,
                "outcome": null,
                "phase": "Pending",
                "resource": {},
                "startedAt": null,
                "update": {
                    "dependencies": {"count": 0, "refs": []},
                    "disruption": "None",
                    "lastAssessedAt": null,
                    "observedGeneration": 0,
                    "operationId": null,
                    "owned": {"count": 0, "refs": []},
                    "preserveState": true,
                    "reasons": [],
                    "state": "Unknown",
                    "targetGeneration": 1
                }
            }
        });
        ResourceEnvelope::from_json(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    #[test]
    fn owner_service_export_is_admitted_and_wrong_rows_are_not() {
        let factory = factory(&["Device"]);
        let export = export(&factory);
        assert_eq!(
            admit_export(
                &export,
                &envelope("security-key.d2bus.org.SecurityKeyService", None),
                &[],
                &factory
            )
            .unwrap()
            .operation_count(),
            1
        );
        assert_eq!(
            admit_export(&export, &envelope("Device", None), &[], &factory),
            Err(ExportImportError::ServiceTargetInvalid)
        );
    }

    #[test]
    fn import_owned_service_and_backing_rows_are_rejected_with_exact_variant() {
        let factory = factory(&["Device"]);
        let export = export(&factory);
        assert_eq!(
            admit_export(
                &export,
                &envelope(
                    "security-key.d2bus.org.SecurityKeyService",
                    Some("ResourceImport/key")
                ),
                &[],
                &factory,
            ),
            Err(ExportImportError::ImportOwnedOriginRejected)
        );
        assert_eq!(
            admit_export(
                &export,
                &envelope("security-key.d2bus.org.SecurityKeyService", None),
                &[&envelope("Device", Some("ResourceImport/key"))],
                &factory,
            ),
            Err(ExportImportError::ImportOwnedOriginRejected)
        );
    }

    #[test]
    fn empty_backing_allowlist_denies_specified_backing_but_positive_control_admits() {
        let empty_factory = factory(&[]);
        let empty_export = export(&empty_factory);
        assert_eq!(
            admit_export(
                &empty_export,
                &envelope("security-key.d2bus.org.SecurityKeyService", None),
                &[&envelope("Device", None)],
                &empty_factory,
            ),
            Err(ExportImportError::BackingReferenceNotAllowed)
        );
        let allowed_factory = factory(&["Device"]);
        let allowed_export = export(&allowed_factory);
        assert!(
            admit_export(
                &allowed_export,
                &envelope("security-key.d2bus.org.SecurityKeyService", None),
                &[&envelope("Device", None)],
                &allowed_factory,
            )
            .is_ok()
        );
    }

    #[test]
    fn import_factory_and_projection_identity_preserve_service_type() {
        let factory = factory(&[]);
        let export = export(&factory);
        let import = import(&factory);
        let admitted = admit_import(&import, &export, &factory, &factory).unwrap();
        let import_ref = ResourceRef::parse("ResourceImport/key").unwrap();
        let identity = projection_identity(&import_ref, &import, &factory).unwrap();
        assert_eq!(
            admitted
                .projection_identity(&import_ref)
                .unwrap()
                .service_type(),
            factory.service_type()
        );
        assert_eq!(identity.projection_ref(), &import.projection_service_ref());
        assert_eq!(identity.owner_ref(), &import_ref);
        assert_eq!(
            admit_binding_target(&factory, &ResourceRef::parse("Guest/work").unwrap()),
            Ok(())
        );
        assert_eq!(
            admit_binding_target(&factory, &ResourceRef::parse("Zone/work").unwrap()),
            Err(ExportImportError::BindingTargetNotAllowed)
        );
    }

    #[test]
    fn factory_protocol_skew_is_not_reported_as_fingerprint_tamper() {
        let expected = factory(&[]);
        let mut wire = serde_json::to_value(&expected).unwrap();
        wire["projectionProtocolVersion"] = serde_json::json!("1.0");
        let legacy: ProjectionFactory = serde_json::from_value(wire).unwrap();
        assert_eq!(
            admit_factory_pair(&legacy, &expected),
            Err(ExportImportError::ProjectionProtocolVersionMismatch)
        );
    }
}
