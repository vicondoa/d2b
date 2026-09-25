//! Provider lifecycle validation and child-resource planning.

use std::collections::BTreeSet;

use d2b_contracts_provider::v3::ComponentType;
use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_zone_session::v3::ZoneStatusResource;
use d2b_controller_toolkit::{DependencySnapshot, OwnerIdentity, ResourceKey, ResourceSnapshot};

use crate::driver::{
    SYSTEM_CORE_HOST_REF, SYSTEM_CORE_PROVIDER_REF, SYSTEM_MINIJAIL_PROVIDER_REF,
};

/// Provider lifecycle phase derived from exact child observations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderPhase {
    Pending,
    Ready,
    Degraded,
    Failed,
    Unknown,
}

/// Requested Provider lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderIntent {
    Enable,
    Update,
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
    /// The Provider package is committed for this row.
    pub package_present: bool,
    /// The declared config passes the Provider's own validation.
    pub config_valid: bool,
    /// The declared dependency graph is well-formed.
    pub graph_valid: bool,
    /// The row satisfies the process-conformance contract.
    pub conformance_valid: bool,
    /// Every required dependency is ready.
    pub required_dependencies_ready: bool,
    /// Every required component is ready.
    pub required_components_ready: bool,
    /// An optional component is degraded.
    pub optional_components_degraded: bool,
    /// The row's components have drained.
    pub components_drained: bool,
}

/// Closed Provider handler failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderError {
    WrongResourceType,
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

/// Whether the fixed system-core handlers are ready, read from the `Zone`
/// dependency of the `Provider/system-core` row.
///
/// Public with `provider_observation`: this crate's driver applies the same
/// predicate over manager-served dependency rows, so the policy has one home.
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
                if dependency_resource.owner().map(OwnerIdentity::uid) != Some(resource.key().uid()) {
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
                let owner_uid_matches = dependency_resource
                    .owner()
                    .map(OwnerIdentity::uid)
                    == Some(resource.key().uid());
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
    use super::*;

    #[test]
    fn fixed_system_core_never_plans_a_process_child() {
        let plan = ProviderHandler::plan_system_core(true);
        assert_eq!(plan.phase(), ProviderPhase::Ready);
        assert!(plan.actions().is_empty());
    }

    #[test]
    fn plan_observed_projects_degraded_when_optional_components_degrade() {
        let provider_ref = ResourceRef::parse("Provider/runtime").expect("fixture reference");
        let observation = ProviderObservation {
            package_present: true,
            config_valid: true,
            graph_valid: true,
            conformance_valid: true,
            required_dependencies_ready: true,
            required_components_ready: true,
            optional_components_degraded: true,
            components_drained: true,
        };
        for intent in [ProviderIntent::Enable, ProviderIntent::Update] {
            let plan = ProviderHandler::plan_observed(&provider_ref, intent, observation)
                .expect("a ready observation plans");
            assert_eq!(
                plan.phase(),
                ProviderPhase::Degraded,
                "{intent:?} with a degraded optional component must project Degraded"
            );
            assert!(
                plan.publish_exports(),
                "{intent:?} with a degraded optional component must keep exports published"
            );
        }
    }
}
