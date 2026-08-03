//! Shared D096 export/import adapter contracts.
//!
//! Core owns `ResourceExport`/`ResourceImport` lifecycle and ZoneLink routing.
//! A Provider adapter owns only semantic admission and its signed factory
//! metadata.  The adapter traits below therefore do not expose a transport,
//! stream, file descriptor, path, backing handle, or remote reference.

use d2b_contracts::v3::{
    BindingTargetType, Exportability, ProjectionFactory, ProviderContractError, ResourceEnvelope,
    ResourceExportSpec, ResourceImportSpec, ResourceRef, ResourceTypeName,
    SEMANTIC_PROJECTION_PROTOCOL_VERSION, SemanticProjectionProtocolVersion,
};

/// Why a Provider-side share admission was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShareAdapterError {
    /// The export base contract failed.
    ExportContract,
    /// The import base contract failed.
    ImportContract,
    /// The exact stored row is owned by a ResourceImport.
    ImportOwnedOriginRejected,
    /// The declared projection protocol differs from the local protocol.
    ProjectionProtocolVersionMismatch,
    /// The factory metadata is absent, inconsistent, or not catalog-derived.
    ProjectionFactoryInvalid,
    /// Factory fingerprints do not match their signed metadata.
    DescriptorFingerprintMismatch,
    /// The capability is marked non-exportable.
    ExportForbidden,
    /// A Provider-specific admission failed with a closed Provider reason.
    ProviderContract(ProviderContractError),
    /// A Binding target is outside the signed target allowlist.
    BindingTargetNotAllowed,
    /// An import requests a capability outside the export ceiling.
    CapabilityNotAllowed,
    /// A ResourceRef is not one of the local references allowed by D096.
    ReferenceNotAllowed,
}

impl ShareAdapterError {
    /// Return the stable identity-free diagnostic label.
    pub const fn code(self) -> &'static str {
        match self {
            Self::ExportContract => "share-export-contract-invalid",
            Self::ImportContract => "share-import-contract-invalid",
            Self::ImportOwnedOriginRejected => "provider-import-owned-origin-rejected",
            Self::ProjectionProtocolVersionMismatch => {
                "provider-projection-protocol-version-mismatch"
            }
            Self::ProjectionFactoryInvalid => "provider-projection-factory-invalid",
            Self::DescriptorFingerprintMismatch => "provider-descriptor-fingerprint-mismatch",
            Self::ExportForbidden => "provider-export-forbidden",
            Self::ProviderContract(error) => error.code(),
            Self::BindingTargetNotAllowed => "share-binding-target-not-allowed",
            Self::CapabilityNotAllowed => "share-capability-not-allowed",
            Self::ReferenceNotAllowed => "share-reference-not-allowed",
        }
    }
}

impl core::fmt::Display for ShareAdapterError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ShareAdapterError {}

fn map_provider_error(error: ProviderContractError) -> ShareAdapterError {
    match error {
        ProviderContractError::ImportOwnedOriginRejected => {
            ShareAdapterError::ImportOwnedOriginRejected
        }
        ProviderContractError::ProjectionProtocolVersionMismatch => {
            ShareAdapterError::ProjectionProtocolVersionMismatch
        }
        ProviderContractError::ProjectionFactoryInvalid => {
            ShareAdapterError::ProjectionFactoryInvalid
        }
        ProviderContractError::DescriptorFingerprintMismatch => {
            ShareAdapterError::DescriptorFingerprintMismatch
        }
        ProviderContractError::ExportForbidden => ShareAdapterError::ExportForbidden,
        other => ShareAdapterError::ProviderContract(other),
    }
}

/// Compare two signed projection factories in the normative order.
///
/// Protocol version is checked first so version skew is not misreported as
/// tampering.  The declared identity, reference sets, and exportability are
/// then checked before fingerprints.  This mirrors Provider installation
/// admission while remaining useful to the Core routing seam.
pub fn admit_factory_pair(
    remote: &ProjectionFactory,
    local: &ProjectionFactory,
) -> Result<(), ShareAdapterError> {
    let installed = SemanticProjectionProtocolVersion::parse(SEMANTIC_PROJECTION_PROTOCOL_VERSION)
        .expect("the installed projection protocol version is valid");
    if remote.projection_protocol_version() != &installed
        || local.projection_protocol_version() != &installed
    {
        return Err(ShareAdapterError::ProjectionProtocolVersionMismatch);
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
                ShareAdapterError::ExportForbidden
            } else {
                ShareAdapterError::ProjectionFactoryInvalid
            },
        );
    }
    if remote.projection_schema_fingerprint() != local.projection_schema_fingerprint()
        || remote.factory_fingerprint() != local.factory_fingerprint()
    {
        return Err(ShareAdapterError::DescriptorFingerprintMismatch);
    }
    Ok(())
}

/// Admit one export against a stored owner Service and stored backing rows.
///
/// The target and each backing row are envelopes, not caller-supplied
/// discriminants.  In particular, an import-owned row reaches the dedicated
/// `ImportOwnedOriginRejected` result from the Provider contract.
pub fn admit_export(
    export: &ResourceExportSpec,
    target: &ResourceEnvelope,
    backing: &[&ResourceEnvelope],
    factory: &ProjectionFactory,
) -> Result<(), ShareAdapterError> {
    export
        .validate_target(target)
        .map_err(|_| ShareAdapterError::ExportContract)?;
    export
        .validate_factory(factory)
        .map_err(map_provider_error)?;
    factory
        .admits_export_target(target)
        .map_err(map_provider_error)?;
    for resource in backing {
        factory
            .admits_backing_ref(resource)
            .map_err(map_provider_error)?;
    }
    Ok(())
}

/// Admit one import against the remote export and the local signed factory.
///
/// The remote factory is the metadata carried by the owner Provider
/// advertisement; `local_factory` is the selected consumer Provider's signed
/// descriptor.  Both must match before a lease or projection can be created.
pub fn admit_import(
    import: &ResourceImportSpec,
    export: &ResourceExportSpec,
    remote_factory: &ProjectionFactory,
    local_factory: &ProjectionFactory,
) -> Result<(), ShareAdapterError> {
    export
        .validate_factory(remote_factory)
        .map_err(map_provider_error)?;
    import
        .validate_against_export(export)
        .map_err(|error| match error {
            d2b_contracts::v3::ResourceImportContractError::InvalidCapability => {
                ShareAdapterError::CapabilityNotAllowed
            }
            _ => ShareAdapterError::ImportContract,
        })?;
    import
        .validate_factory(local_factory)
        .map_err(map_provider_error)?;
    admit_factory_pair(remote_factory, local_factory)
}

/// Admit a Binding target against the factory's closed target ResourceTypes.
pub fn admit_binding_target(
    factory: &ProjectionFactory,
    target: &ResourceRef,
) -> Result<(), ShareAdapterError> {
    let target_type = match target.resource_type().as_str() {
        "Guest" => BindingTargetType::Guest,
        "User" => BindingTargetType::User,
        "Zone" => BindingTargetType::Zone,
        _ => return Err(ShareAdapterError::ReferenceNotAllowed),
    };
    if factory
        .allowed_binding_target_ref_types()
        .contains(&target_type)
    {
        Ok(())
    } else {
        Err(ShareAdapterError::BindingTargetNotAllowed)
    }
}

/// A Provider export adapter with one immutable signed projection factory.
pub trait ExportAdapter: Send + Sync {
    /// Borrow the factory signed in this Provider artifact.
    fn projection_factory(&self) -> &ProjectionFactory;

    /// Run common export admission before Provider-specific arbitration.
    fn admit_export(
        &self,
        export: &ResourceExportSpec,
        target: &ResourceEnvelope,
        backing: &[&ResourceEnvelope],
    ) -> Result<(), ShareAdapterError> {
        admit_export(export, target, backing, self.projection_factory())
    }
}

/// A Provider import adapter with one immutable signed projection factory.
pub trait ImportAdapter: Send + Sync {
    /// Borrow the factory signed in this Provider artifact.
    fn projection_factory(&self) -> &ProjectionFactory;

    /// Run common import admission before Provider-specific route setup.
    fn admit_import(
        &self,
        import: &ResourceImportSpec,
        export: &ResourceExportSpec,
        remote_factory: &ProjectionFactory,
    ) -> Result<(), ShareAdapterError> {
        admit_import(import, export, remote_factory, self.projection_factory())
    }
}

/// A Provider that uses the same implementation for both export and import.
pub trait ShareAdapter: ExportAdapter + ImportAdapter {}

impl<T> ShareAdapter for T where T: ExportAdapter + ImportAdapter {}

/// Return a factory's declared protocol version without exposing its
/// implementation identity.
pub const fn projection_protocol_version(
    factory: &ProjectionFactory,
) -> &SemanticProjectionProtocolVersion {
    factory.projection_protocol_version()
}

/// Return the exact qualified Service type carried by a factory.
pub const fn service_type(factory: &ProjectionFactory) -> &ResourceTypeName {
    factory.service_type()
}

#[cfg(test)]
mod tests {
    use d2b_contracts::v3::{
        BindingTargetType, ConsumerZonePolicy, ExportArbitration, ResourceExportSpec,
        ResourceImportSpec, ResourceName, ResourceTypeName, SchemaFingerprint,
        execution_policy::BoundedToken,
    };

    use super::*;

    struct FakeAdapter {
        factory: ProjectionFactory,
    }

    impl ExportAdapter for FakeAdapter {
        fn projection_factory(&self) -> &ProjectionFactory {
            &self.factory
        }
    }

    impl ImportAdapter for FakeAdapter {
        fn projection_factory(&self) -> &ProjectionFactory {
            &self.factory
        }
    }

    fn fingerprint(digit: char) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}", digit.to_string().repeat(64))).unwrap()
    }

    fn factory() -> ProjectionFactory {
        ProjectionFactory::new(
            ResourceTypeName::parse("audio.d2bus.org.AudioService").unwrap(),
            ResourceTypeName::parse("audio.d2bus.org.AudioBinding").unwrap(),
            [ResourceTypeName::parse("Endpoint").unwrap()],
            [BindingTargetType::Guest],
            fingerprint('a'),
            fingerprint('b'),
            Exportability::ExplicitExport,
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
            ExportArbitration::Exclusive,
            ConsumerZonePolicy::new(Vec::new(), vec![BoundedToken::parse("capture").unwrap()])
                .unwrap(),
        )
        .unwrap()
    }

    fn import() -> ResourceImportSpec {
        ResourceImportSpec::minimal(
            ResourceRef::parse("ZoneLink/parent").unwrap(),
            "owner/mic",
            ResourceTypeName::parse("audio.d2bus.org.AudioService").unwrap(),
            fingerprint('a'),
            fingerprint('b'),
            ResourceName::parse("mic").unwrap(),
            vec![BoundedToken::parse("capture").unwrap()],
        )
        .unwrap()
    }

    #[test]
    fn import_and_target_allowlist_are_positive_controls() {
        let adapter = FakeAdapter { factory: factory() };
        assert_eq!(
            adapter.admit_import(&import(), &export(), &adapter.factory),
            Ok(())
        );
        assert_eq!(
            admit_binding_target(
                &adapter.factory,
                &ResourceRef::parse("Guest/workstation").unwrap()
            ),
            Ok(())
        );
        assert_eq!(
            admit_binding_target(&adapter.factory, &ResourceRef::parse("User/alice").unwrap()),
            Err(ShareAdapterError::BindingTargetNotAllowed)
        );
    }

    #[test]
    fn wrong_factory_and_unexportable_factories_fail_closed() {
        let wrong = ProjectionFactory::new(
            ResourceTypeName::parse("audio.d2bus.org.AudioService").unwrap(),
            ResourceTypeName::parse("audio.d2bus.org.AudioBinding").unwrap(),
            [ResourceTypeName::parse("Endpoint").unwrap()],
            [BindingTargetType::Guest],
            fingerprint('a'),
            fingerprint('c'),
            Exportability::ExplicitExport,
        )
        .unwrap();
        assert_eq!(
            admit_factory_pair(&wrong, &factory()),
            Err(ShareAdapterError::DescriptorFingerprintMismatch)
        );
        assert_eq!(
            ShareAdapterError::ExportForbidden.code(),
            "provider-export-forbidden"
        );
    }
}
