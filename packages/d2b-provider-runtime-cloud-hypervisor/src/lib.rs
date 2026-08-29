//! Canonical `Provider/runtime-cloud-hypervisor` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

use d2b_contracts_resource::v3::ResourceRef;

pub mod adoption;
pub mod audit;
pub mod bootstrap_graph;
pub mod config;
pub mod controller;
mod controller_session;
pub mod health;
pub mod metrics;
pub mod state;
pub mod vmm_argv;

pub use config::{CloudHypervisorConfig, CloudHypervisorGuestSettings, ConsoleType};
pub use controller::{
    CloudHypervisorClock, CloudHypervisorController, CloudHypervisorEffectPort,
    CloudHypervisorError, CloudHypervisorPhase, CloudHypervisorReconcileOutcome,
    CloudHypervisorRecoveryState, SystemCloudHypervisorClock,
};
pub use health::{
    GuestSessionError, GuestSessionEvidence, GuestSessionEvidenceProbe, GuestSessionHealth,
};
pub use vmm_argv::{
    ChArgvError, ChArgvInput, ChNetIface, ChVsock, exec_arg0, generate_ch_argv,
};

/// Stable Provider implementation identifier.
pub const CLOUD_HYPERVISOR_IMPLEMENTATION_ID: &str = "cloud-hypervisor";
/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/runtime-cloud-hypervisor";
/// Controller Process role declared by the Provider contract.
pub const CONTROLLER_ROLE_REF: &str = "Process/cloud-hypervisor-controller";
/// Controller binary declared by the Provider manifest.
pub const CONTROLLER_BINARY: &str = "d2b-cloud-hypervisor-controller";
/// Exit status used when authenticated controller-session wiring fails.
pub const RUNTIME_UNAVAILABLE_EXIT: i32 = 78;

/// Return whether a ResourceRef names this Provider implementation.
pub fn is_provider_ref(reference: &ResourceRef) -> bool {
    reference.resource_type().as_str() == "Provider"
        && reference.name().as_str() == "runtime-cloud-hypervisor"
}

/// Build the controller role contract used by this Provider package.
///
/// The package manifest artifact is generated only when the final Provider
/// package is assembled; this semantic constructor keeps the controller
/// session seam usable before that generated artifact is available.
pub fn provider_manifest() -> Result<d2b_contracts_provider::v3::ProviderManifest, serde_json::Error> {
    use d2b_contracts_provider::v3::{
        ArtifactDigest, ArtifactDigestSet, BinaryRef, CompatibilityRange, ComponentDescriptor,
        ComponentExecution, ComponentTargetCapability, ComponentType, ControllerInstanceScope,
        ControllerTargetKind, EffectPortClass, PolicyEvaluation, ProviderManifest,
        ResourceApiBinding, RevocationState, SignatureState, TargetRuntimeArtifacts,
        TrustEvidence, UpgradeDisposition, UpgradePolicy,
    };
    use d2b_contracts_resource::v3::execution_policy::BoundedToken;
    use d2b_contracts_resource::v3::{PlacementAnchor, ResourceTypeName, SchemaFingerprint, SchemaVersion};

    let digest = ArtifactDigest::parse(format!("sha256:{}", "a".repeat(64)))
        .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?;
    let fingerprint = SchemaFingerprint::parse(format!("sha256:{}", "b".repeat(64)))
        .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?;
    let guest = ResourceTypeName::parse("Guest")
        .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?;
    let component = ComponentDescriptor::new(
        BoundedToken::parse("cloud-hypervisor-controller")
            .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?,
        ComponentType::Controller,
        [guest.clone()],
        [BoundedToken::parse("assess-update")
            .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?],
        [d2b_contracts_resource::v3::ExecutionDomain::System],
        1,
        digest.clone(),
        [],
        false,
    )
    .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?
    .with_execution(ComponentExecution::Launchable {
        binary_ref: BinaryRef::parse(CONTROLLER_BINARY)
            .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?,
    })
    .with_controller_placement(
        ControllerInstanceScope::FixedExecutionTarget,
        [ControllerTargetKind::Host],
    )
    .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?
    .with_target_capabilities([
        ComponentTargetCapability::new(
            ControllerTargetKind::Host,
            digest.clone(),
            [EffectPortClass::Process],
        )
        .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?,
    ])
    .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?;
    let binding = ResourceApiBinding::new_with_placement(
        guest,
        SchemaVersion::new(1, 0)
            .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?,
        fingerprint.clone(),
        SchemaVersion::new(1, 0)
            .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?,
        fingerprint.clone(),
        Default::default(),
        None,
        None,
        PlacementAnchor::ExecutionRef,
    )
    .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?;
    let trust = TrustEvidence {
        publisher: BoundedToken::parse("d2b-cloud-hypervisor")
            .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?,
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
        support_channel: BoundedToken::parse("stable")
            .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?,
    };
    let host_artifacts = TargetRuntimeArtifacts::new(
        ControllerTargetKind::Host,
        digest.clone(),
        digest.clone(),
    )
    .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?;
    let manifest = ProviderManifest::new(
        d2b_contracts_resource::v3::ArtifactId::parse("runtime-cloud-hypervisor")
            .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?,
        ArtifactDigestSet {
            executable: digest.clone(),
            config: digest.clone(),
            schema: digest.clone(),
            service: digest.clone(),
        },
        trust,
        CompatibilityRange {
            api_major: 3,
            api_minor: 0,
            descriptor_fingerprint: fingerprint.clone(),
            state_schema_version: SchemaVersion::new(1, 0)
                .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?,
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
    .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))?;
    manifest
        .with_target_runtime_artifacts([host_artifacts])
        .map_err(|error| serde_json::Error::io(std::io::Error::other(error.to_string())))
}

/// Enter the controller role.
pub fn controller_binary_entrypoint() -> i32 {
    controller_session::run_from_fd10()
}
