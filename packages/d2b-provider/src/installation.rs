//! Provider installation admission: manifest plus resource row, not package
//! presence.
//!
//! `ADR-046-provider-model-and-packaging` section "Provider resource" opens
//! with the rule this module enforces: package presence alone is not
//! installation, and a `providerRef` resolves only a Ready Provider resource
//! in the same Zone. The registry in this crate answers "is this Provider
//! admitting calls right now"; this module answers the question that comes
//! first, "may this artifact be installed as this Provider at all".
//!
//! The admission is a conjunction and every clause fails closed:
//!
//! - the resource row's `artifactId` selects exactly this manifest;
//! - production trust admission holds, evaluated before compatibility so a
//!   compatible but untrusted artifact can never slip through;
//! - the Provider API major is exact, the minor is additive only, and the
//!   descriptor fingerprint matches the advertisement, with no handshake
//!   downgrade;
//! - the registry descriptor publishes no method the signed component graph
//!   does not export, so a Provider cannot widen its own surface after
//!   signing;
//! - the Provider is Ready.
//!
//! Nothing here mutates host state, opens a socket, or resolves a path, and
//! no type here carries authority. An [`InstalledProvider`] is a decision
//! that has already been reached, not a capability that can be presented.

use std::collections::BTreeSet;

use d2b_contracts_provider::v3::{
    ControllerTargetKind,
    EffectPortClass,
    ProviderManifest,
    ProviderSpec,
};
use d2b_contracts_resource::v3::{
    ResourceRef,
    SchemaFingerprint,
};

use crate::{descriptor::ProviderDescriptor, error::RegistryBuildError};

/// Whether the Provider resource row has reached Ready.
///
/// The variant set is deliberately coarse: a `providerRef` either resolves
/// or it does not, and every non-Ready phase is equally unresolvable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderReadiness {
    /// The Provider resource is Ready and selectable by `providerRef`.
    Ready,
    /// The Provider resource exists but is not Ready. Its components,
    /// dependencies, or exported services have not all come up.
    NotReady,
    /// The Provider is disabled or quarantined by a Zone condition.
    Quarantined,
}

impl ProviderReadiness {
    /// Whether a `providerRef` resolves to this Provider.
    pub const fn resolves(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// The exact Provider API and descriptor identity a Zone requires before it
/// installs any artifact.
///
/// The requirement is supplied by the Zone rather than read from the
/// artifact, because an artifact that stated its own requirement would be
/// checking itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiredProviderApi {
    api_major: u32,
    api_minor: u32,
    descriptor_fingerprint: SchemaFingerprint,
}

impl RequiredProviderApi {
    /// Declare the exact API the Zone requires.
    pub const fn new(
        api_major: u32,
        api_minor: u32,
        descriptor_fingerprint: SchemaFingerprint,
    ) -> Self {
        Self {
            api_major,
            api_minor,
            descriptor_fingerprint,
        }
    }

    /// The required Provider API major version.
    pub const fn api_major(&self) -> u32 {
        self.api_major
    }

    /// The required Provider API minor version.
    pub const fn api_minor(&self) -> u32 {
        self.api_minor
    }

    /// The required descriptor fingerprint.
    pub const fn descriptor_fingerprint(&self) -> &SchemaFingerprint {
        &self.descriptor_fingerprint
    }
}

/// A Provider whose artifact, resource row, and registry descriptor have all
/// been admitted together.
///
/// It carries the `Provider/<name>` reference so a caller can key on it, and
/// nothing else: the manifest stays with the caller that supplied it, so this
/// value cannot be used to re-derive the artifact's trust decision.
#[derive(Clone, PartialEq, Eq)]
pub struct InstalledProvider {
    provider_ref: ResourceRef,
}

/// The fixed EffectPort classes available from one Host or Guest target
/// profile during Provider installation.
#[derive(Clone, PartialEq, Eq)]
pub struct TargetInstallProfile {
    target_kind: ControllerTargetKind,
    supported_effect_classes: BTreeSet<EffectPortClass>,
}

impl TargetInstallProfile {
    /// Construct a target profile from the fixed daemon composition.
    pub fn new(
        target_kind: ControllerTargetKind,
        supported_effect_classes: impl IntoIterator<Item = EffectPortClass>,
    ) -> Self {
        Self {
            target_kind,
            supported_effect_classes: supported_effect_classes.into_iter().collect(),
        }
    }

    /// Return the target kind.
    pub const fn target_kind(&self) -> ControllerTargetKind {
        self.target_kind
    }

    /// Borrow the profile's supported EffectPort classes.
    pub const fn supported_effect_classes(&self) -> &BTreeSet<EffectPortClass> {
        &self.supported_effect_classes
    }
}

impl core::fmt::Debug for TargetInstallProfile {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("TargetInstallProfile")
            .field("target_kind", &self.target_kind)
            .field(
                "effect_class_count",
                &self.supported_effect_classes.len(),
            )
            .finish_non_exhaustive()
    }
}

impl InstalledProvider {
    /// The admitted `Provider/<name>` reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }
}

impl core::fmt::Debug for InstalledProvider {
    /// The exact Provider a Zone installed is routing detail, so the
    /// decision renders as a decision and not as a name.
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("InstalledProvider(<redacted>)")
    }
}

/// Admit one Provider installation, or refuse with a closed reason.
///
/// The order is load bearing. Artifact selection is checked first so a
/// mismatched pair is never evaluated for trust; trust and compatibility are
/// checked next, inside the manifest, which evaluates trust first; the
/// published surface is checked last, because narrowing a surface is only
/// meaningful once the artifact itself is admissible.
pub fn admit_installation(
    spec: &ProviderSpec,
    manifest: &ProviderManifest,
    descriptor: &ProviderDescriptor,
    required_api: &RequiredProviderApi,
    readiness: ProviderReadiness,
) -> Result<InstalledProvider, RegistryBuildError> {
    descriptor.validate()?;
    manifest
        .validate_installation_contract()
        .map_err(|_| RegistryBuildError::ArtifactNotAdmissible)?;
    if spec.artifact_id() != manifest.artifact_id() {
        return Err(RegistryBuildError::ArtifactSelectionMismatch);
    }

    manifest
        .admit(
            required_api.api_major(),
            required_api.api_minor(),
            required_api.descriptor_fingerprint(),
        )
        .map_err(|_| RegistryBuildError::ArtifactNotAdmissible)?;
    if !publishes_only_signed_methods(manifest, descriptor) {
        return Err(RegistryBuildError::UnsignedPublishedMethod);
    }
    if !readiness.resolves() {
        return Err(RegistryBuildError::ProviderNotReady);
    }
    Ok(InstalledProvider {
        provider_ref: descriptor.provider_ref().clone(),
    })
}

/// Admit a Provider installation against one fixed target profile.
pub fn admit_installation_for_target(
    spec: &ProviderSpec,
    manifest: &ProviderManifest,
    descriptor: &ProviderDescriptor,
    required_api: &RequiredProviderApi,
    readiness: ProviderReadiness,
    profile: &TargetInstallProfile,
) -> Result<InstalledProvider, RegistryBuildError> {
    let installed = admit_installation(spec, manifest, descriptor, required_api, readiness)?;
    manifest
        .validate_target_effects(
            profile.target_kind(),
            profile.supported_effect_classes(),
        )
        .map_err(|_| RegistryBuildError::ArtifactNotAdmissible)?;
    Ok(installed)
}

/// Whether every method the registry descriptor publishes appears in the
/// signed component graph's exported method set.
///
/// A Provider may publish fewer methods than it signed, because a component
/// may be absent from a given placement. It may never publish more.
fn publishes_only_signed_methods(
    manifest: &ProviderManifest,
    descriptor: &ProviderDescriptor,
) -> bool {
    descriptor.capabilities().methods().all(|method| {
        manifest.components().iter().any(|component| {
            component
                .exported_methods()
                .iter()
                .any(|exported| exported.as_str() == method.as_str())
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_provider::v3::{
        ArtifactDigest,
        ArtifactDigestSet,
        BinaryRef,
        ComponentDescriptor,
        ComponentExecution,
        ComponentTargetCapability,
        ComponentType,
        ControllerInstanceScope,
        ControllerTargetKind,
        EffectPortClass,
        PolicyEvaluation,
        ResourceApiBinding,
        RevocationState,
        SignatureState,
        StandardCapabilityMatrix,
        TrustEvidence,
        TargetRuntimeArtifacts,
        UpgradeDisposition,
        UpgradePolicy,
    };
    use d2b_contracts_resource::v3::{
        ArtifactId,
        execution_policy::{BoundedToken, ExecutionDomain},
        ConfigurationGeneration,
        ResourceGeneration,
        ResourceTypeName,
        resource_schema::{PlacementAnchor, SchemaVersion},
    };
    use d2b_contracts_resource::v3::identity::ServiceName;
    use d2b_contracts_zone_session::v3::zone_routing::ZonePath;

    use crate::identity::{
        ProviderCapabilitySet, ProviderClass, ProviderImplementationId, ProviderMethodName,
    };

    const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";

    fn fingerprint(tail: &str) -> SchemaFingerprint {
        SchemaFingerprint::parse(format!("sha256:{}{tail}", "0".repeat(63))).unwrap()
    }

    fn trusted() -> TrustEvidence {
        TrustEvidence {
            publisher: BoundedToken::parse("first-party").unwrap(),
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
        }
    }

    fn manifest(trust: TrustEvidence) -> ProviderManifest {
        let component = ComponentDescriptor::new(
            BoundedToken::parse("volume-controller").unwrap(),
            ComponentType::Controller,
            [ResourceTypeName::parse("Volume").unwrap()],
            [
                BoundedToken::parse("assess-update").unwrap(),
                BoundedToken::parse("plan-upgrade").unwrap(),
            ],
            [ExecutionDomain::System],
            1,
            ArtifactDigest::parse(DIGEST).unwrap(),
            [],
            false,
        )
        .unwrap()
        .with_execution(ComponentExecution::Launchable {
            binary_ref: BinaryRef::parse("volume-controller").unwrap(),
        })
        .with_controller_placement(
            ControllerInstanceScope::PerResourceTarget,
            [ControllerTargetKind::Host, ControllerTargetKind::Guest],
        )
        .unwrap()
        .with_target_capabilities([
            ComponentTargetCapability::new(
                ControllerTargetKind::Host,
                ArtifactDigest::parse(DIGEST).unwrap(),
                [EffectPortClass::Storage],
            )
            .unwrap(),
            ComponentTargetCapability::new(
                ControllerTargetKind::Guest,
                ArtifactDigest::parse(DIGEST).unwrap(),
                [EffectPortClass::Storage],
            )
            .unwrap(),
        ])
        .unwrap();
        let binding = ResourceApiBinding::new_with_placement(
            ResourceTypeName::parse("Volume").unwrap(),
            SchemaVersion::new(1, 0).unwrap(),
            fingerprint("2"),
            SchemaVersion::new(1, 0).unwrap(),
            fingerprint("3"),
            StandardCapabilityMatrix::default(),
            None,
            None,
            PlacementAnchor::ExecutionRef,
        )
        .unwrap();
        let digest = ArtifactDigest::parse(DIGEST).unwrap();
        ProviderManifest::new(
            ArtifactId::parse("provider-volume-local").unwrap(),
            ArtifactDigestSet {
                executable: digest.clone(),
                config: digest.clone(),
                schema: digest.clone(),
                service: digest.clone(),
            },
            trust,
            d2b_contracts_provider::v3::provider::CompatibilityRange {
                api_major: 3,
                api_minor: 4,
                descriptor_fingerprint: fingerprint("1"),
                state_schema_version: SchemaVersion::new(1, 0).unwrap(),
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
        .unwrap()
        .with_target_runtime_artifacts([
            TargetRuntimeArtifacts::new(
                ControllerTargetKind::Host,
                digest.clone(),
                digest.clone(),
            )
            .unwrap(),
            TargetRuntimeArtifacts::new(
                ControllerTargetKind::Guest,
                digest.clone(),
                digest.clone(),
            )
            .unwrap(),
        ])
        .unwrap()
    }

    fn descriptor(methods: &[&str]) -> ProviderDescriptor {
        ProviderDescriptor::new(
            ZonePath::local_root(),
            ResourceRef::parse("Provider/volume-local").unwrap(),
            ProviderClass::Storage,
            ProviderImplementationId::parse("volume-local").unwrap(),
            ConfigurationGeneration::new(1).unwrap(),
            ResourceGeneration::new(1).unwrap(),
            ServiceName::parse("d2b.volume.v3").unwrap(),
            ProviderCapabilitySet::new(
                methods
                    .iter()
                    .map(|method| ProviderMethodName::parse(*method).unwrap()),
            )
            .unwrap(),
        )
        .unwrap()
    }

    fn required() -> RequiredProviderApi {
        RequiredProviderApi::new(3, 4, fingerprint("1"))
    }

    fn spec(artifact: &str) -> ProviderSpec {
        ProviderSpec::minimal(ArtifactId::parse(artifact).unwrap())
    }

    #[test]
    fn a_ready_trusted_compatible_provider_is_installed() {
        let installed = admit_installation(
            &spec("provider-volume-local"),
            &manifest(trusted()),
            &descriptor(&["assess-update"]),
            &required(),
            ProviderReadiness::Ready,
        )
        .unwrap();
        assert_eq!(
            installed.provider_ref(),
            &ResourceRef::parse("Provider/volume-local").unwrap()
        );
        assert_eq!(format!("{installed:?}"), "InstalledProvider(<redacted>)");
    }

    #[test]
    fn target_profile_must_support_every_signed_effect_class() {
        let profile = TargetInstallProfile::new(
            ControllerTargetKind::Host,
            [EffectPortClass::Runtime],
        );
        assert_eq!(
            admit_installation_for_target(
                &spec("provider-volume-local"),
                &manifest(trusted()),
                &descriptor(&["assess-update"]),
                &required(),
                ProviderReadiness::Ready,
                &profile,
            )
            .unwrap_err(),
            RegistryBuildError::ArtifactNotAdmissible
        );

        let profile = TargetInstallProfile::new(
            ControllerTargetKind::Host,
            [EffectPortClass::Storage],
        );
        assert!(admit_installation_for_target(
            &spec("provider-volume-local"),
            &manifest(trusted()),
            &descriptor(&["assess-update"]),
            &required(),
            ProviderReadiness::Ready,
            &profile,
        )
        .is_ok());
    }

    #[test]
    fn package_presence_is_not_installation() {
        for readiness in [ProviderReadiness::NotReady, ProviderReadiness::Quarantined] {
            assert_eq!(
                admit_installation(
                    &spec("provider-volume-local"),
                    &manifest(trusted()),
                    &descriptor(&["assess-update"]),
                    &required(),
                    readiness,
                ),
                Err(RegistryBuildError::ProviderNotReady)
            );
            assert!(!readiness.resolves());
        }
        assert!(ProviderReadiness::Ready.resolves());
    }

    #[test]
    fn the_resource_row_must_select_this_exact_artifact() {
        assert_eq!(
            admit_installation(
                &spec("provider-volume-remote"),
                &manifest(trusted()),
                &descriptor(&["assess-update"]),
                &required(),
                ProviderReadiness::Ready,
            ),
            Err(RegistryBuildError::ArtifactSelectionMismatch)
        );
    }

    #[test]
    fn an_untrusted_or_incompatible_artifact_is_never_installed() {
        let mut denied = trusted();
        denied.emergency_deny = true;
        assert_eq!(
            admit_installation(
                &spec("provider-volume-local"),
                &manifest(denied),
                &descriptor(&["assess-update"]),
                &required(),
                ProviderReadiness::Ready,
            ),
            Err(RegistryBuildError::ArtifactNotAdmissible)
        );
        for wrong in [
            RequiredProviderApi::new(2, 4, fingerprint("1")),
            RequiredProviderApi::new(3, 9, fingerprint("1")),
            RequiredProviderApi::new(3, 4, fingerprint("7")),
        ] {
            assert_eq!(
                admit_installation(
                    &spec("provider-volume-local"),
                    &manifest(trusted()),
                    &descriptor(&["assess-update"]),
                    &wrong,
                    ProviderReadiness::Ready,
                ),
                Err(RegistryBuildError::ArtifactNotAdmissible)
            );
        }
    }

    #[test]
    fn a_provider_cannot_publish_a_method_its_signed_graph_does_not_export() {
        assert_eq!(
            admit_installation(
                &spec("provider-volume-local"),
                &manifest(trusted()),
                &descriptor(&["assess-update", "exfiltrate-state"]),
                &required(),
                ProviderReadiness::Ready,
            ),
            Err(RegistryBuildError::UnsignedPublishedMethod)
        );
        // Publishing a strict subset of the signed surface stays legal: a
        // component may be absent from a given placement.
        assert!(
            admit_installation(
                &spec("provider-volume-local"),
                &manifest(trusted()),
                &descriptor(&["plan-upgrade"]),
                &required(),
                ProviderReadiness::Ready,
            )
            .is_ok()
        );
    }
}
