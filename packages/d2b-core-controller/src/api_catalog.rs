//! Resource API catalog staging and atomic publication policy.

use std::collections::{BTreeMap, BTreeSet};

use d2b_contracts::v3::{
    ProviderManifest, ResourceApiBinding, ResourceRef, ResourceTypeName, SchemaFingerprint,
};

/// Closed reason a catalog candidate was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiCatalogError {
    EmptyCandidate,
    InvalidRevision,
    ProviderNotAdmitted,
    ResourceTypeCollision,
    MissingController,
    PolicyDenied,
    CompatibilityNotProven,
    WithdrawalBlocked,
}

impl ApiCatalogError {
    /// Return the stable, cardinality-bounded reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::EmptyCandidate => "api-catalog-empty",
            Self::InvalidRevision => "api-catalog-revision-invalid",
            Self::ProviderNotAdmitted => "api-catalog-provider-not-admitted",
            Self::ResourceTypeCollision => "api-catalog-resource-type-collision",
            Self::MissingController => "api-catalog-controller-missing",
            Self::PolicyDenied => "api-catalog-policy-denied",
            Self::CompatibilityNotProven => "api-catalog-compatibility-unproven",
            Self::WithdrawalBlocked => "api-catalog-withdrawal-blocked",
        }
    }
}

impl core::fmt::Display for ApiCatalogError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ApiCatalogError {}

/// Trusted policy inputs used while validating one Provider manifest.
///
/// Compatibility evidence is supplied by the strict schema validator. The
/// core handler does not infer additive compatibility from version numbers.
#[derive(Clone)]
pub struct CatalogAdmission<'a> {
    pub required_api_major: u32,
    pub required_api_minor: u32,
    pub required_descriptor_fingerprint: &'a SchemaFingerprint,
    pub permitted_resource_types: &'a BTreeSet<ResourceTypeName>,
    pub additive_compatibility_proven: bool,
}

impl core::fmt::Debug for CatalogAdmission<'_> {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CatalogAdmission")
            .field(
                "permitted_resource_type_count",
                &self.permitted_resource_types.len(),
            )
            .field(
                "additive_compatibility_proven",
                &self.additive_compatibility_proven,
            )
            .finish_non_exhaustive()
    }
}

/// One Provider-derived API catalog entry.
#[derive(Clone, PartialEq, Eq)]
pub struct CatalogEntry {
    provider_ref: ResourceRef,
    binding: ResourceApiBinding,
}

impl CatalogEntry {
    /// Borrow the Provider that owns this binding.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the exact signed API binding.
    pub const fn binding(&self) -> &ResourceApiBinding {
        &self.binding
    }
}

impl core::fmt::Debug for CatalogEntry {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CatalogEntry")
            .field("has_resource_type", &true)
            .field("has_provider", &true)
            .finish()
    }
}

/// Inactive catalog prepared for one durable commit.
#[derive(Clone)]
pub struct StagedApiCatalog {
    revision: u64,
    entries: BTreeMap<ResourceTypeName, CatalogEntry>,
}

impl core::fmt::Debug for StagedApiCatalog {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("StagedApiCatalog")
            .field("revision", &self.revision)
            .field("entry_count", &self.entries.len())
            .finish()
    }
}

impl StagedApiCatalog {
    /// Return the revision this candidate must be committed at.
    pub const fn revision(&self) -> u64 {
        self.revision
    }

    /// Return the number of exact ResourceType bindings.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether this candidate has no bindings.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Active immutable API validator/catalog view.
#[derive(Default)]
pub struct ApiCatalogHandler {
    revision: u64,
    entries: BTreeMap<ResourceTypeName, CatalogEntry>,
}

impl core::fmt::Debug for ApiCatalogHandler {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ApiCatalogHandler")
            .field("revision", &self.revision)
            .field("entry_count", &self.entries.len())
            .finish()
    }
}

impl ApiCatalogHandler {
    /// Prepare a complete replacement without changing the active catalog.
    ///
    /// `withdrawal_blockers` contains types that still have resources or
    /// finalizers. Such a type cannot disappear from the candidate.
    pub fn stage<'a>(
        &self,
        revision: u64,
        providers: impl IntoIterator<Item = (ResourceRef, ProviderManifest, CatalogAdmission<'a>)>,
        withdrawal_blockers: &BTreeSet<ResourceTypeName>,
    ) -> Result<StagedApiCatalog, ApiCatalogError> {
        if revision == 0 || revision <= self.revision {
            return Err(ApiCatalogError::InvalidRevision);
        }

        let mut entries = BTreeMap::new();
        for (provider_ref, manifest, admission) in providers {
            if provider_ref.resource_type().as_str() != "Provider"
                || manifest
                    .admit(
                        admission.required_api_major,
                        admission.required_api_minor,
                        admission.required_descriptor_fingerprint,
                    )
                    .is_err()
            {
                return Err(ApiCatalogError::ProviderNotAdmitted);
            }
            if !admission.additive_compatibility_proven {
                return Err(ApiCatalogError::CompatibilityNotProven);
            }

            for binding in manifest.api_bindings() {
                if !admission
                    .permitted_resource_types
                    .contains(binding.resource_type())
                {
                    return Err(ApiCatalogError::PolicyDenied);
                }
                let owns_type = manifest.components().iter().any(|component| {
                    component
                        .exported_resource_types()
                        .contains(binding.resource_type())
                });
                if !owns_type {
                    return Err(ApiCatalogError::MissingController);
                }
                let entry = CatalogEntry {
                    provider_ref: provider_ref.clone(),
                    binding: binding.clone(),
                };
                if entries
                    .insert(binding.resource_type().clone(), entry)
                    .is_some()
                {
                    return Err(ApiCatalogError::ResourceTypeCollision);
                }
            }
        }
        if entries.is_empty() {
            return Err(ApiCatalogError::EmptyCandidate);
        }
        if withdrawal_blockers
            .iter()
            .any(|resource_type| !entries.contains_key(resource_type))
        {
            return Err(ApiCatalogError::WithdrawalBlocked);
        }
        Ok(StagedApiCatalog { revision, entries })
    }

    /// Swap the active validator/catalog only after its durable commit.
    pub fn activate(&mut self, staged: StagedApiCatalog, committed_revision: u64) -> bool {
        if staged.revision != committed_revision || committed_revision <= self.revision {
            return false;
        }
        self.revision = committed_revision;
        self.entries = staged.entries;
        true
    }

    /// Resolve one exact active ResourceType binding.
    pub fn binding(&self, resource_type: &ResourceTypeName) -> Option<&CatalogEntry> {
        self.entries.get(resource_type)
    }

    /// Return the active catalog revision.
    pub const fn revision(&self) -> u64 {
        self.revision
    }
}

#[cfg(test)]
mod tests {
    use d2b_contracts::v3::{
        ArtifactDigest, ArtifactDigestSet, ArtifactId, CompatibilityRange, ComponentDescriptor,
        ComponentType, PolicyEvaluation, ProviderUpgradePolicy, RevocationState, SchemaVersion,
        SignatureState, StandardCapabilityMatrix, TrustEvidence, UpgradeDisposition,
        execution_policy::{BoundedToken, ExecutionDomain},
    };

    use super::*;

    const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";

    fn fingerprint(last: char) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}{last}", "0".repeat(63))).unwrap()
    }

    fn manifest(resource_type: &str) -> ProviderManifest {
        let resource_type = ResourceTypeName::parse(resource_type).unwrap();
        let digest = || ArtifactDigest::parse(DIGEST).unwrap();
        ProviderManifest::new(
            ArtifactId::parse("provider").unwrap(),
            ArtifactDigestSet {
                package: digest(),
                executable: digest(),
                manifest: digest(),
                config: digest(),
                schema: digest(),
                service: digest(),
            },
            TrustEvidence {
                publisher: BoundedToken::parse("trusted").unwrap(),
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
                support_channel: BoundedToken::parse("stable").unwrap(),
            },
            CompatibilityRange {
                api_major: 3,
                api_minor: 0,
                descriptor_fingerprint: fingerprint('1'),
                state_schema_version: SchemaVersion::new(1, 0).unwrap(),
            },
            [ComponentDescriptor::new(
                BoundedToken::parse("controller").unwrap(),
                ComponentType::Controller,
                [resource_type.clone()],
                [],
                [ExecutionDomain::System],
                1,
                digest(),
                [],
                false,
            )
            .unwrap()],
            [ResourceApiBinding::new(
                resource_type,
                SchemaVersion::new(1, 0).unwrap(),
                fingerprint('2'),
                SchemaVersion::new(1, 0).unwrap(),
                fingerprint('3'),
                StandardCapabilityMatrix::default(),
                None,
                None,
            )
            .unwrap()],
            [],
            ProviderUpgradePolicy {
                drain_before_upgrade: true,
                max_automatic_disposition: UpgradeDisposition::InPlace,
                preserves_durable_state: true,
            },
        )
        .unwrap()
    }

    fn admission<'a>(
        fingerprint: &'a SchemaFingerprint,
        permitted: &'a BTreeSet<ResourceTypeName>,
    ) -> CatalogAdmission<'a> {
        CatalogAdmission {
            required_api_major: 3,
            required_api_minor: 0,
            required_descriptor_fingerprint: fingerprint,
            permitted_resource_types: permitted,
            additive_compatibility_proven: true,
        }
    }

    #[test]
    fn durable_commit_atomically_activates_the_candidate() {
        let mut handler = ApiCatalogHandler::default();
        let required = fingerprint('1');
        let resource_type = ResourceTypeName::parse("Volume").unwrap();
        let permitted = BTreeSet::from([resource_type.clone()]);
        let staged = handler
            .stage(
                1,
                [(
                    ResourceRef::parse("Provider/volume").unwrap(),
                    manifest("Volume"),
                    admission(&required, &permitted),
                )],
                &BTreeSet::new(),
            )
            .unwrap();
        assert!(handler.binding(&resource_type).is_none());
        assert!(handler.activate(staged, 1));
        assert!(handler.binding(&resource_type).is_some());
    }

    #[test]
    fn collision_is_rejected_without_changing_the_active_catalog() {
        let handler = ApiCatalogHandler::default();
        let required = fingerprint('1');
        let permitted = BTreeSet::from([ResourceTypeName::parse("Volume").unwrap()]);
        let providers = ["one", "two"].map(|name| {
            (
                ResourceRef::parse(&format!("Provider/{name}")).unwrap(),
                manifest("Volume"),
                admission(&required, &permitted),
            )
        });
        assert_eq!(
            handler.stage(1, providers, &BTreeSet::new()).unwrap_err(),
            ApiCatalogError::ResourceTypeCollision
        );
        assert_eq!(handler.revision(), 0);
    }

    #[test]
    fn unproven_compatibility_and_blocked_withdrawal_are_rejected() {
        let handler = ApiCatalogHandler::default();
        let required = fingerprint('1');
        let volume = ResourceTypeName::parse("Volume").unwrap();
        let permitted = BTreeSet::from([volume.clone()]);
        let mut unproven = admission(&required, &permitted);
        unproven.additive_compatibility_proven = false;
        assert_eq!(
            handler
                .stage(
                    1,
                    [(
                        ResourceRef::parse("Provider/volume").unwrap(),
                        manifest("Volume"),
                        unproven
                    )],
                    &BTreeSet::new(),
                )
                .unwrap_err(),
            ApiCatalogError::CompatibilityNotProven
        );

        let process = ResourceTypeName::parse("Process").unwrap();
        assert_eq!(
            handler
                .stage(
                    1,
                    [(
                        ResourceRef::parse("Provider/volume").unwrap(),
                        manifest("Volume"),
                        admission(&required, &permitted)
                    )],
                    &BTreeSet::from([process]),
                )
                .unwrap_err(),
            ApiCatalogError::WithdrawalBlocked
        );
    }
}
