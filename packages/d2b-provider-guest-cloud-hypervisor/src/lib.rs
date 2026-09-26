//! Canonical `Provider/runtime-cloud-hypervisor` implementation.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

use std::path::PathBuf;

use d2b_contracts_resource::v3::ResourceRef;

mod adoption;
pub mod bootstrap_graph;
pub mod config;
pub mod controller;
mod controller_session;
pub mod descriptor;
pub mod guest_local;
pub mod health;
pub mod identity;
pub mod shutdown;
pub mod state;

pub use adoption::ProcessAdoptionStatus;
pub use bootstrap_graph::{
    BootstrapGraph, BootstrapGraphError, DependencyReadiness, GuestChildGraphPlan,
    VmmLifecycleEligibility,
};
pub use config::{CloudHypervisorConfig, ConfigValidationError, MachineType};
pub use controller::{CLOUD_HYPERVISOR_REPAIR_INTERVAL_SECS, GUEST_CONTROLLER_FINALIZER};
pub use controller::{
    AuthenticatedResourceApiAdapter, AuthenticatedResourceSession, ChildSpecUpdate,
    CloudHypervisorController, CloudHypervisorControllerRegistration, CloudHypervisorError,
    CloudHypervisorReconcileOutcome, CloudHypervisorResourceApi, CloudHypervisorResourceApiError,
    CloudHypervisorResourceRequest, CloudHypervisorResourceResponse, GuestChildCommitResponse,
    GuestChildCreateBatch, GuestCondition, GuestDependencySnapshot, GuestSnapshot,
    GuestStatusProjection, OwnedChildSnapshot,
};
pub use descriptor::{
    BootstrapHandoff, DescriptorSignature, GuestSeedContract, GuestSetupDescriptor,
    GuestSetupDescriptorError, GuestSetupDescriptorVerifier, OpaqueDescriptorSignature,
    SignatureAlgorithm, VerifiedGuestSetupDescriptor,
};
pub use guest_local::{GUEST_SEED_RESOURCE_TYPES, GuestControlEndpoint, GuestLocalError};
pub use health::{
    GuestSessionError, GuestSessionEvidence, GuestSessionEvidenceBinding, GuestSessionHealth,
};
pub use identity::{
    ChildCreateBody, ChildIdentityError, ChildMutation, ChildRole, ChildRoleSet,
    CommitResponseError, CommittedChild, CommittedChildren, CreatePrecondition, EndpointCreateBody,
    GuestChildBatch, ProcessCreateBody, VolumeCreateBody, deterministic_child_name,
    deterministic_child_ref, map_commit_response,
};
pub use shutdown::{
    FencedChild, FinalizationBlockReason, FinalizationDisposition, FinalizationStep,
    GuestFinalizationInput, GuestFinalizationPlan, GuestUpgradePlan, LifecyclePlanError,
    ProcessState, SessionState, UpgradeReason, child_role_for_ref, plan_finalization, plan_upgrade,
};
pub use state::{
    GuestGenerationSet, GuestRuntimeStatus, GuestStatusObservation, GuestStatusPhase,
    finalization_eligible, reduce_status,
};

/// Stable Provider resource reference.
pub const PROVIDER_REF: &str = "Provider/runtime-cloud-hypervisor";
/// Controller Process role declared by the Provider contract.
pub const CONTROLLER_ROLE_REF: &str = "Process/cloud-hypervisor-controller";
/// Exit status used when authenticated controller-session wiring fails.
pub const RUNTIME_UNAVAILABLE_EXIT: i32 = 78;

/// Return whether a ResourceRef names this Provider implementation.
pub fn is_provider_ref(reference: &ResourceRef) -> bool {
    reference.resource_type().as_str() == "Provider"
        && reference.name().as_str() == "runtime-cloud-hypervisor"
}

/// Parse the canonical Provider manifest packaged with this Provider.
pub fn provider_manifest() -> Result<d2b_contracts_provider::v3::ProviderManifest, serde_json::Error>
{
    serde_json::from_slice(include_bytes!("../provider-manifest.json"))
}

/// Enter the controller role.
pub fn controller_binary_entrypoint() -> i32 {
    controller_session::run_from_fd10()
}

/// The `--api-socket` value of a cloud-hypervisor argv, when present.
pub fn api_socket_path(argv: &[String]) -> Option<PathBuf> {
    let mut iter = argv.iter();
    while let Some(argument) = iter.next() {
        if argument == "--api-socket"
            && let Some(path) = iter.next()
        {
            return Some(PathBuf::from(path));
        }
        if let Some(path) = argument.strip_prefix("--api-socket=") {
            return Some(PathBuf::from(path));
        }
    }
    None
}

/// The `socket=` field of the `--vsock` device, when present.
pub fn vsock_socket_path(argv: &[String]) -> Option<PathBuf> {
    let mut iter = argv.iter();
    while let Some(argument) = iter.next() {
        if argument == "--vsock"
            && let Some(spec) = iter.next()
        {
            for field in spec.split(',') {
                if let Some(path) = field.strip_prefix("socket=") {
                    return Some(PathBuf::from(path));
                }
            }
        }
    }
    None
}

/// Every socket-carrying argv path the broker's stale-socket preflight
/// unlinks before spawning this Provider's runner.
pub fn preflight_socket_paths(argv: &[String]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(path) = api_socket_path(argv) {
        paths.push(path);
    }
    if let Some(path) = vsock_socket_path(argv) {
        paths.push(path);
    }
    paths
}

/// This Provider's audio capability row: PipeWire vhost-user-sound host
/// enforcement plus a signed target-local audio Process.
pub fn audio_capability() -> d2b_core::provider_capabilities::AudioProviderCapability {
    d2b_core::provider_capabilities::AudioProviderCapability {
        host_enforcement: d2b_core::provider_capabilities::AudioHostEnforcementKind::PipeWireVhostUserSound,
        guest_enforcement: d2b_core::provider_capabilities::AudioGuestEnforcementKind::ProcessCapable,
        needs_local_state_file: true,
    }
}
