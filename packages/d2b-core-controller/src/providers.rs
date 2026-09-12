//! Provider lifecycle validation and child-resource planning.

use std::collections::BTreeSet;

use d2b_contracts_provider::v3::{ComponentType, ProviderManifest};
use d2b_contracts_resource::v3::{ResourceRef, SchemaFingerprint};
use d2b_contracts_zone_session::v3::ZoneStatusResource;
use d2b_controller_toolkit::{DependencySnapshot, ResourceKey, ResourceSnapshot};

/// Provider lifecycle phase derived from exact child observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderPhase {
    Pending,
    Ready,
    Draining,
    Degraded,
    Failed,
    Unknown,
}

/// Requested Provider lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderIntent {
    Enable,
    Update,
    Disable,
    Delete,
}

/// One child-resource action. The plan never spawns a process directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderChildAction {
    EnsureComponent(ComponentType),
    EnsureDeclaredStateVolume,
    WithdrawExports,
    RevokeComponents,
    RequestComponentDeletion,
}

/// Effect-free Provider lifecycle plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderPlan {
    phase: ProviderPhase,
    actions: Vec<ProviderChildAction>,
    publish_exports: bool,
}

impl ProviderPlan {
    /// Return the projected aggregate phase.
    pub const fn phase(&self) -> ProviderPhase {
        self.phase
    }

    /// Borrow the child-resource actions.
    pub fn actions(&self) -> &[ProviderChildAction] {
        &self.actions
    }

    /// Whether exported ResourceTypes and services may be published.
    pub const fn publish_exports(&self) -> bool {
        self.publish_exports
    }
}

/// Trusted observations needed to plan one Provider pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderObservation {
    pub package_present: bool,
    pub config_valid: bool,
    pub graph_valid: bool,
    pub conformance_valid: bool,
    pub required_dependencies_ready: bool,
    pub required_components_ready: bool,
    pub optional_components_degraded: bool,
    pub components_drained: bool,
}

/// Closed Provider handler failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderError {
    WrongResourceType,
    TrustOrCompatibilityDenied,
    PackageUnavailable,
    ConfigInvalid,
    GraphInvalid,
    ConformanceInvalid,
}

impl ProviderError {
    /// Return a stable, identity-free reason code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::WrongResourceType => "provider-resource-type-invalid",
            Self::TrustOrCompatibilityDenied => "provider-admission-denied",
            Self::PackageUnavailable => "provider-package-unavailable",
            Self::ConfigInvalid => "provider-config-invalid",
            Self::GraphInvalid => "provider-graph-invalid",
            Self::ConformanceInvalid => "provider-conformance-invalid",
        }
    }
}

impl core::fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ProviderError {}

/// Pure Provider lifecycle planner.
pub struct ProviderHandler;

impl ProviderHandler {
    /// Validate and plan an external Provider from its signed manifest.
    #[allow(clippy::too_many_arguments)]
    pub fn plan_external(
        provider_ref: &ResourceRef,
        manifest: &ProviderManifest,
        required_api_major: u32,
        required_api_minor: u32,
        required_descriptor_fingerprint: &SchemaFingerprint,
        intent: ProviderIntent,
        observation: ProviderObservation,
    ) -> Result<ProviderPlan, ProviderError> {
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(ProviderError::WrongResourceType);
        }
        manifest
            .admit(
                required_api_major,
                required_api_minor,
                required_descriptor_fingerprint,
            )
            .map_err(|_| ProviderError::TrustOrCompatibilityDenied)?;
        manifest
            .validate_installation_contract()
            .map_err(|_| ProviderError::GraphInvalid)?;
        if !observation.package_present {
            return Err(ProviderError::PackageUnavailable);
        }
        if !observation.config_valid {
            return Err(ProviderError::ConfigInvalid);
        }
        if !observation.graph_valid {
            return Err(ProviderError::GraphInvalid);
        }
        if !observation.conformance_valid {
            return Err(ProviderError::ConformanceInvalid);
        }

        let mut plan = Self::plan_observed(provider_ref, intent, observation)?;
        if matches!(intent, ProviderIntent::Enable | ProviderIntent::Update) {
            plan.actions = manifest
                .components()
                .iter()
                .map(|component| {
                    ProviderChildAction::EnsureComponent(component.component_type())
                })
                .collect();
            if manifest.declares_state_volume() {
                plan.actions
                    .push(ProviderChildAction::EnsureDeclaredStateVolume);
            }
        }
        Ok(plan)
    }

    /// Project an already admitted Provider from its trusted runtime evidence.
    ///
    /// Manifest admission remains the authority for artifact, descriptor, and
    /// registration identity. This entry point is used by the active Core
    /// handler after those facts have been established by the runtime and
    /// represented in the owned Process/session observations.
    pub fn plan_observed(
        provider_ref: &ResourceRef,
        intent: ProviderIntent,
        observation: ProviderObservation,
    ) -> Result<ProviderPlan, ProviderError> {
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(ProviderError::WrongResourceType);
        }
        if !observation.package_present {
            return Err(ProviderError::PackageUnavailable);
        }
        if !observation.config_valid {
            return Err(ProviderError::ConfigInvalid);
        }
        if !observation.graph_valid {
            return Err(ProviderError::GraphInvalid);
        }
        if !observation.conformance_valid {
            return Err(ProviderError::ConformanceInvalid);
        }

        match intent {
            ProviderIntent::Enable | ProviderIntent::Update => {
                let ready = observation.required_dependencies_ready
                    && observation.required_components_ready;
                Ok(ProviderPlan {
                    phase: if ready {
                        if observation.optional_components_degraded {
                            ProviderPhase::Degraded
                        } else {
                            ProviderPhase::Ready
                        }
                    } else {
                        ProviderPhase::Pending
                    },
                    actions: Vec::new(),
                    publish_exports: ready,
                })
            }
            ProviderIntent::Disable | ProviderIntent::Delete => Ok(ProviderPlan {
                phase: if observation.components_drained {
                    ProviderPhase::Pending
                } else {
                    ProviderPhase::Draining
                },
                actions: vec![
                    ProviderChildAction::WithdrawExports,
                    ProviderChildAction::RevokeComponents,
                    ProviderChildAction::RequestComponentDeletion,
                ],
                publish_exports: false,
            }),
        }
    }

    /// Plan the fixed system-core bootstrap exception.
    ///
    /// It is hosted internally and therefore never receives a Process child.
    pub fn plan_system_core(required_handlers_ready: bool) -> ProviderPlan {
        ProviderPlan {
            phase: if required_handlers_ready {
                ProviderPhase::Ready
            } else {
                ProviderPhase::Pending
            },
            actions: Vec::new(),
            publish_exports: required_handlers_ready,
        }
    }
}

// ---------------------------------------------------------------------------
// Pure Core `Provider` observation policy (retained from the deleted
// store-driven `runtime.rs`; the v3 Core driver in `d2bd` consumes it).
// ---------------------------------------------------------------------------

fn resource_field(key: &ResourceKey) -> String {
    key.resource_ref().to_canonical_string()
}

/// Core reconcile adapter error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CoreReconcileError;

impl core::fmt::Display for CoreReconcileError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("core reconcile contract failed")
    }
}

impl std::error::Error for CoreReconcileError {}

fn provider_process_session_ready(
    provider_ref: &str,
    provider_uid: &str,
    provider_generation: u64,
    process: &ResourceSnapshot,
) -> bool {
    let process_value =
        match serde_json::from_slice::<serde_json::Value>(process.canonical_json()) {
            Ok(value) => value,
            Err(error) => {
                tracing::debug!(
                    resource = resource_field(process.key()),
                    reason = %error,
                    "controller session treated as not ready: process canonical JSON did not parse",
                );
                return false;
            }
        };
    let session = process_value
        .pointer("/status/resource/controllerSession")
        .and_then(serde_json::Value::as_object);
    let Some(session) = session else {
        return false;
    };
    let expected_process_ref = process.key().resource_ref().to_canonical_string();
    let process_uid = process.key().uid().as_str();
    session.get("ready").and_then(serde_json::Value::as_bool) == Some(true)
        && session.get("providerRef").and_then(serde_json::Value::as_str)
            == Some(provider_ref)
        && session.get("providerUid").and_then(serde_json::Value::as_str)
            == Some(provider_uid)
        && session.get("processRef").and_then(serde_json::Value::as_str)
            == Some(expected_process_ref.as_str())
        && session.get("processUid").and_then(serde_json::Value::as_str) == Some(process_uid)
        && session
            .get("processGeneration")
            .and_then(serde_json::Value::as_u64)
            == Some(process.generation().get())
        && session
            .get("providerGeneration")
            .and_then(serde_json::Value::as_u64)
            == Some(provider_generation)
        && session
            .get("controllerGeneration")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|generation| generation > 0)
        && session
            .get("sessionGeneration")
            .and_then(serde_json::Value::as_u64)
            .is_some_and(|generation| generation > 0)
        && session
            .get("artifactReady")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        && session
            .get("descriptorReady")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
        && session
            .get("registrationReady")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
}

const SYSTEM_CORE_PROVIDER_REF: &str = "Provider/system-core";
const SYSTEM_MINIJAIL_PROVIDER_REF: &str = "Provider/system-minijail";
const SYSTEM_CORE_HOST_REF: &str = "Host/host-system";

/// Whether the fixed system-core handlers are ready, read from the `Zone`
/// dependency of the `Provider/system-core` row.
///
/// Public with `provider_observation`: the v3 Core driver (U12) applies the
/// same predicate over manager-served dependency rows, so the policy has one
/// home.
pub fn fixed_system_core_handlers_ready(dependencies: &[DependencySnapshot]) -> bool {
    dependencies.iter().any(|dependency| {
        let resource = dependency.resource();
        if resource.key().resource_ref().resource_type().as_str() != "Zone" {
            return false;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(resource.canonical_json())
        else {
            tracing::debug!(
                resource = resource_field(resource.key()),
                "mandatory-handler readiness treated as false: Zone canonical JSON did not parse",
            );
            return false;
        };
        let Some(status) = value.pointer("/status/resource").cloned() else {
            return false;
        };
        serde_json::from_value::<ZoneStatusResource>(status)
            .is_ok_and(|status| status.mandatory_handlers_ready())
    })
}

fn fixed_provider_host_ready(dependencies: &[DependencySnapshot]) -> bool {
    dependencies.iter().any(|dependency| {
        let resource = dependency.resource();
        if resource.key().resource_ref().to_canonical_string() != SYSTEM_CORE_HOST_REF {
            return false;
        }
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(resource.canonical_json())
        else {
            tracing::debug!(
                resource = resource_field(resource.key()),
                "host readiness treated as false: Host canonical JSON did not parse",
            );
            return false;
        };
        value
            .pointer("/spec/providerRef")
            .and_then(serde_json::Value::as_str)
            == Some(SYSTEM_CORE_PROVIDER_REF)
            && value
                .pointer("/status/phase")
                .and_then(serde_json::Value::as_str)
                == Some("Ready")
            && value
                .pointer("/status/observedGeneration")
                .and_then(serde_json::Value::as_u64)
                == Some(resource.generation().get())
    })
}

fn expected_provider_volume_refs(provider: &serde_json::Value) -> BTreeSet<String> {
    ["/status/resource/owned/refs", "/status/update/owned/refs"]
        .into_iter()
        .filter_map(|path| provider.pointer(path))
        .filter_map(serde_json::Value::as_array)
        .flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|resource_ref| resource_ref.starts_with("Volume/"))
        .map(str::to_owned)
        .collect()
}

/// The pure observation the Core `Provider` handler applies to its
/// dependency list. Public so the bridge that merges manager-served
/// dependency rows (G5) is pinned against the exact consumer of that list.
pub fn provider_observation(
    resource: &ResourceSnapshot,
    dependencies: &[DependencySnapshot],
) -> Result<ProviderObservation, CoreReconcileError> {
    let provider = serde_json::from_slice::<serde_json::Value>(resource.canonical_json())
        .map_err(|error| {
            tracing::warn!(
                resource = resource_field(resource.key()),
                reason = %error,
                "provider observation failed: canonical JSON did not parse",
            );
            CoreReconcileError
        })?;
    let provider_ref = resource.key().resource_ref().to_canonical_string();
    let spec = provider
        .get("spec")
        .and_then(serde_json::Value::as_object)
        .ok_or(CoreReconcileError)?;
    let package_present = spec
        .get("artifactId")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|artifact| !artifact.is_empty());
    let config_valid = spec
        .get("config")
        .is_some_and(serde_json::Value::is_object);
    let fixed_host_ready = match provider_ref.as_str() {
        SYSTEM_CORE_PROVIDER_REF => Some(fixed_system_core_handlers_ready(dependencies)),
        SYSTEM_MINIJAIL_PROVIDER_REF => Some(fixed_provider_host_ready(dependencies)),
        _ => None,
    };
    if let Some(required_ready) = fixed_host_ready {
        return Ok(ProviderObservation {
            package_present: provider_ref == SYSTEM_CORE_PROVIDER_REF || package_present,
            config_valid,
            graph_valid: true,
            conformance_valid: true,
            required_dependencies_ready: required_ready,
            required_components_ready: required_ready,
            optional_components_degraded: false,
            components_drained: true,
        });
    }
    let mut process_count = 0_usize;
    let mut graph_valid = true;
    let mut conformance_valid = true;
    let mut required_components_ready = true;
    let mut required_dependencies_ready = true;
    let mut optional_components_degraded = false;
    let expected_volume_refs = expected_provider_volume_refs(&provider);
    let mut observed_volume_refs = BTreeSet::new();

    for dependency in dependencies {
        let dependency_resource = dependency.resource();
        let value = serde_json::from_slice::<serde_json::Value>(
            dependency_resource.canonical_json(),
        )
        .map_err(|error| {
            tracing::warn!(
                resource = resource_field(dependency_resource.key()),
                reason = %error,
                "provider observation failed: dependency canonical JSON did not parse",
            );
            CoreReconcileError
        })?;
        let owner_ref = value
            .pointer("/metadata/ownerRef")
            .and_then(serde_json::Value::as_str);
        if owner_ref != Some(provider_ref.as_str()) {
            continue;
        }
        match dependency_resource
            .key()
            .resource_ref()
            .resource_type()
            .as_str()
        {
            "Process" => {
                if dependency_resource.owner_uid() != Some(resource.key().uid()) {
                    graph_valid = false;
                    conformance_valid = false;
                    required_components_ready = false;
                    required_dependencies_ready = false;
                    continue;
                }
                process_count += 1;
                let process_spec = value
                    .get("spec")
                    .and_then(serde_json::Value::as_object);
                let process_valid = process_spec
                    .and_then(|spec| spec.get("processClass"))
                    .and_then(serde_json::Value::as_str)
                    == Some("controller")
                    && process_spec
                        .and_then(|spec| spec.get("providerRef"))
                        .and_then(serde_json::Value::as_str)
                            .is_some_and(|provider| {
                                matches!(
                                    provider,
                                    "Provider/system-minijail" | "Provider/system-systemd"
                                )
                            });
                let phase = value
                    .pointer("/status/phase")
                    .and_then(serde_json::Value::as_str);
                let process_ready = phase == Some("Ready")
                    && value
                        .pointer("/status/observedGeneration")
                        .and_then(serde_json::Value::as_u64)
                        == Some(dependency_resource.generation().get());
                let provider_uid = resource.key().uid().as_str();
                let session_ready = provider_process_session_ready(
                    provider_ref.as_str(),
                    provider_uid,
                    resource.generation().get(),
                    dependency_resource,
                );
                graph_valid &= process_valid;
                conformance_valid &= process_valid && session_ready;
                required_components_ready &= process_ready && session_ready;
                required_dependencies_ready &= process_ready;
                optional_components_degraded |= phase == Some("Degraded");
            }
            "Volume" => {
                let volume_ref = dependency_resource
                    .key()
                    .resource_ref()
                    .to_canonical_string();
                if !expected_volume_refs.contains(&volume_ref) {
                    continue;
                }
                observed_volume_refs.insert(volume_ref);
                let owner_uid_matches =
                    dependency_resource.owner_uid() == Some(resource.key().uid());
                if !owner_uid_matches {
                    required_dependencies_ready = false;
                    continue;
                }
                let volume_ready = value
                    .pointer("/status/phase")
                    .and_then(serde_json::Value::as_str)
                    == Some("Ready")
                    && value
                        .pointer("/status/observedGeneration")
                        .and_then(serde_json::Value::as_u64)
                        == Some(dependency_resource.generation().get());
                required_dependencies_ready &= volume_ready;
            }
            _ => {}
        }
    }
    if !expected_volume_refs.is_empty()
        && !expected_volume_refs.is_subset(&observed_volume_refs)
    {
        required_dependencies_ready = false;
    }

    let has_processes = process_count > 0;
    Ok(ProviderObservation {
        package_present: package_present && has_processes,
        config_valid,
        graph_valid: graph_valid && has_processes,
        conformance_valid: conformance_valid && has_processes,
        required_dependencies_ready: required_dependencies_ready && has_processes,
        required_components_ready: required_components_ready && has_processes,
        optional_components_degraded,
        components_drained: !has_processes,
    })
}

#[cfg(test)]
mod tests {
    use d2b_contracts_provider::v3::UpgradePolicy as ProviderUpgradePolicy;
    use d2b_contracts_provider::v3::{
        ArtifactDigest, ArtifactDigestSet, BinaryRef, CompatibilityRange, ComponentDescriptor,
        ComponentExecution, ComponentTargetCapability, ControllerTargetKind, EffectPortClass,
        PolicyEvaluation, RevocationState, SignatureState, TargetRuntimeArtifacts, TrustEvidence,
        UpgradeDisposition,
    };
    use d2b_contracts_resource::v3::{
        ArtifactId,
        execution_policy::{BoundedToken, ExecutionDomain},
    };

    use super::*;

    const DIGEST: &str = "sha256:0000000000000000000000000000000000000000000000000000000000000001";

    fn fingerprint() -> SchemaFingerprint {
        SchemaFingerprint::parse(DIGEST).unwrap()
    }

    fn manifest(trusted: bool) -> ProviderManifest {
        let digest = || ArtifactDigest::parse(DIGEST).unwrap();
        ProviderManifest::new(
            ArtifactId::parse("provider").unwrap(),
            ArtifactDigestSet {
                executable: digest(),
                config: digest(),
                schema: digest(),
                service: digest(),
            },
            TrustEvidence {
                publisher: BoundedToken::parse("trusted").unwrap(),
                root_epoch: 1,
                publisher_trusted: trusted,
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
                descriptor_fingerprint: fingerprint(),
                state_schema_version: d2b_contracts_resource::v3::SchemaVersion::new(1, 0).unwrap(),
            },
            [ComponentDescriptor::new(
                BoundedToken::parse("service").unwrap(),
                ComponentType::Service,
                [],
                [BoundedToken::parse("observe").unwrap()],
                [ExecutionDomain::System],
                1,
                digest(),
                [],
                false,
            )
            .unwrap()
            .with_execution(ComponentExecution::Launchable {
                binary_ref: BinaryRef::parse("service").unwrap(),
            })
            .with_target_capabilities([
                ComponentTargetCapability::new(
                    ControllerTargetKind::Host,
                    digest(),
                    [EffectPortClass::Runtime],
                )
                .unwrap(),
                ComponentTargetCapability::new(
                    ControllerTargetKind::Guest,
                    digest(),
                    [EffectPortClass::Runtime],
                )
                .unwrap(),
            ])
            .unwrap()],
            [],
            [],
            ProviderUpgradePolicy {
                drain_before_upgrade: true,
                max_automatic_disposition: UpgradeDisposition::InPlace,
                preserves_durable_state: true,
            },
        )
        .unwrap()
        .with_target_runtime_artifacts([
            TargetRuntimeArtifacts::new(ControllerTargetKind::Host, digest(), digest()).unwrap(),
            TargetRuntimeArtifacts::new(ControllerTargetKind::Guest, digest(), digest()).unwrap(),
        ])
        .unwrap()
    }

    fn observation() -> ProviderObservation {
        ProviderObservation {
            package_present: true,
            config_valid: true,
            graph_valid: true,
            conformance_valid: true,
            required_dependencies_ready: true,
            required_components_ready: true,
            optional_components_degraded: false,
            components_drained: false,
        }
    }

    #[test]
    fn ready_external_provider_publishes_only_after_children_are_ready() {
        let plan = ProviderHandler::plan_external(
            &ResourceRef::parse("Provider/example").unwrap(),
            &manifest(true),
            3,
            0,
            &fingerprint(),
            ProviderIntent::Enable,
            observation(),
        )
        .unwrap();
        assert_eq!(plan.phase(), ProviderPhase::Ready);
        assert!(plan.publish_exports());
        assert_eq!(
            plan.actions(),
            &[ProviderChildAction::EnsureComponent(ComponentType::Service)]
        );
    }

    #[test]
    fn missing_dependency_keeps_exports_withdrawn() {
        let mut observed = observation();
        observed.required_dependencies_ready = false;
        let plan = ProviderHandler::plan_external(
            &ResourceRef::parse("Provider/example").unwrap(),
            &manifest(true),
            3,
            0,
            &fingerprint(),
            ProviderIntent::Enable,
            observed,
        )
        .unwrap();
        assert_eq!(plan.phase(), ProviderPhase::Pending);
        assert!(!plan.publish_exports());
    }

    #[test]
    fn untrusted_provider_is_rejected_before_child_planning() {
        let manifest = manifest(false);
        assert_eq!(
            ProviderHandler::plan_external(
                &ResourceRef::parse("Provider/example").unwrap(),
                &manifest,
                3,
                0,
                &fingerprint(),
                ProviderIntent::Enable,
                observation(),
            )
            .unwrap_err(),
            ProviderError::TrustOrCompatibilityDenied
        );
    }

    #[test]
    fn fixed_system_core_never_plans_a_process_child() {
        let plan = ProviderHandler::plan_system_core(true);
        assert_eq!(plan.phase(), ProviderPhase::Ready);
        assert!(plan.actions().is_empty());
    }
}
