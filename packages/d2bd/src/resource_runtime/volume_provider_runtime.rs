//! Shared-Runner composition for the storage Providers.
//!
//! `volume-local` is the sole Volume owner. `volume-virtiofs` owns only
//! VolumeBinding resources and their Process/Endpoint children. The
//! runtime adapter keeps Resource API fencing in Core and sends host
//! mutations through the anchored Volume effect adapter.

use std::{
    collections::{BTreeMap, BTreeSet},
    os::fd::OwnedFd,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    CanonicalJsonValue, ControllerGeneration, ResourceGeneration, ResourceName, ResourceRef,
    ResourceTypeName, ResourceUid, VOLUME_BINDING_RESOURCE_TYPE, ZoneId,
    execution_policy::BoundedToken, identity::ReconnectGeneration,
    resource_status::StatusCode,
    volume::{SourceKind, VolumeSpec},
    volume_binding::{VolumeBindingReadinessFence, VolumeBindingSpec, VolumeBindingStatusResource},
};
use d2b_core_controller::{
    ControllerDescriptor, ControllerExecutionPolicy, ControllerIdentity, ControllerSelector,
    ControllerVerb, CoreControllerSource, DependencySnapshot, DisruptionClass, DrainResult,
    FinalizeResult, HandlerFailure, ObservationResult, ReconcileContext, ReconcileDisposition,
    ReconcilePlan, ReconcileReason, ReconcileResult, ResourceKey, ResourceMutationBatch,
    ResourceReconciler, ResourceRegistration, ResourceSnapshot, ResyncPolicy, Runner, RunnerConfig,
    SelectorField, SourceError, StatusPersistence, UpdateAssessment, UpdateAssessmentState,
    UpgradePlan, UpgradeStage, ValidationResult,
};
use d2b_provider_volume_local::{
    VolumeLayoutEffectPort, VolumeLocalController, VolumeLocalProfile, VolumeRunnerContract,
    VolumeSourceEffectPort, desired_binding_intents, marker_path,
};
use d2b_provider_volume_virtiofs::{
    LaunchedWorker, StoredBinding, VirtiofsBindingController, VirtiofsBindingEffectPort,
    VirtiofsRunnerContract, VirtiofsdWorkerPlan,
};
use d2b_resource_api::registered::RedbRegisteredControllerApi;
use d2b_resource_store::{
    ResourceAssignmentFence, ResourceAssignmentScope, StoreErrorKind, StoreGetRequest,
    StoreOperationContext, StoreProjection, StoredResource,
};
use d2bd_runtime::resource_runtime_support::retry_transient_store_read;
use rustix::fs::{Mode, OFlags, ResolveFlags, open, openat2};
use serde_json::{Value, json};

use super::{
    AssignmentFenceResolver, ZoneResourceRuntime,
    volume_effect_adapter::{AnchoredVolumeEffectAdapter, ResolvedVolumeRoot, VolumeRootResolver},
};

const CORE_CONTROLLER_HOST_REF: &str = "Host/host-system";
const VOLUME_LOCAL_CONTROLLER_REF: &str = "Process/volume-local-controller";
const VOLUME_VIRTIOFS_CONTROLLER_REF: &str = "Process/volume-virtiofs-controller";
const VOLUME_LOCAL_PROVIDER_REF: &str = "Provider/volume-local";
const VOLUME_VIRTIOFS_PROVIDER_REF: &str = "Provider/volume-virtiofs";

/// One Provider ResourceType attached to the shared Runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SharedVolumeRunnerRegistration {
    /// Controller Process reference.
    pub controller_ref: &'static str,
    /// Provider reference selected by the resource spec.
    pub provider_ref: &'static str,
    /// ResourceType owned by the controller.
    pub resource_type: &'static str,
    /// Exact owner finalizer.
    pub finalizer: &'static str,
    /// Bounded repair interval in seconds.
    pub repair_interval_secs: u64,
    /// Watched configuration is dependency-only.
    pub watched_configuration_is_dependency: bool,
}

/// The two U7 ResourceTypes attached to the production shared Runner.
pub const U7_SHARED_PROVIDER_RUNNERS: [SharedVolumeRunnerRegistration; 2] = [
    SharedVolumeRunnerRegistration {
        controller_ref: VOLUME_LOCAL_CONTROLLER_REF,
        provider_ref: VOLUME_LOCAL_PROVIDER_REF,
        resource_type: "Volume",
        finalizer: d2b_provider_volume_local::VOLUME_FINALIZER,
        repair_interval_secs: d2b_provider_local_contract().repair_interval_secs,
        watched_configuration_is_dependency: d2b_provider_local_contract()
            .watched_configuration_is_dependency,
    },
    SharedVolumeRunnerRegistration {
        controller_ref: VOLUME_VIRTIOFS_CONTROLLER_REF,
        provider_ref: VOLUME_VIRTIOFS_PROVIDER_REF,
        resource_type: d2b_contracts_resource::v3::VOLUME_BINDING_RESOURCE_TYPE,
        finalizer: d2b_provider_volume_virtiofs::VOLUME_BINDING_FINALIZER,
        repair_interval_secs: d2b_provider_virtiofs_contract().repair_interval_secs,
        watched_configuration_is_dependency: d2b_provider_virtiofs_contract()
            .watched_configuration_is_dependency,
    },
];

const fn d2b_provider_local_contract() -> VolumeRunnerContract {
    d2b_provider_volume_local::volume_runner_contract()
}

const fn d2b_provider_virtiofs_contract() -> VirtiofsRunnerContract {
    d2b_provider_volume_virtiofs::virtiofs_runner_contract()
}

/// Compose the exact U7 Provider descriptors from authoritative generations.
pub fn compose_shared_volume_runner_descriptors(
    registrations: impl IntoIterator<Item = SharedVolumeRunnerRegistration>,
    zone: ZoneId,
    controller_generation: ControllerGeneration,
    provider_generations: &BTreeMap<ResourceRef, ResourceGeneration>,
    _session_generation: ReconnectGeneration,
) -> Result<Vec<(SharedVolumeRunnerRegistration, ControllerDescriptor)>, super::ResourceRuntimeError>
{
    registrations
        .into_iter()
        .map(|registration| {
            if !registration.watched_configuration_is_dependency
                || !(30..=60).contains(&registration.repair_interval_secs)
            {
                return Err(super::ResourceRuntimeError::HandlerNotReady);
            }
            let provider_ref = ResourceRef::parse(registration.provider_ref)
                .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?;
            let provider_generation = provider_generations
                .get(&provider_ref)
                .copied()
                .ok_or(super::ResourceRuntimeError::HandlerNotReady)?;
            let controller_ref = ResourceRef::parse(registration.controller_ref)
                .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?;
            let resource_type = ResourceTypeName::parse(registration.resource_type.to_owned())
                .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?;
            let identity = ControllerIdentity::new(
                zone.clone(),
                controller_ref.clone(),
                controller_generation,
                provider_ref,
                provider_generation,
                controller_ref,
                ResourceRef::parse(CORE_CONTROLLER_HOST_REF)
                    .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?,
                None,
            )
            .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?;
            let registration_body =
                ResourceRegistration::new(resource_type.clone(), vec![1], 5_000, 3)
                    .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?;
            let mut selectors = vec![
                ControllerSelector::new(
                    resource_type.clone(),
                    SelectorField::Spec,
                    Some(registration.provider_ref.to_owned()),
                )
                .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?,
            ];
            for field in [
                SelectorField::Status,
                SelectorField::Metadata,
                SelectorField::Finalizers,
                SelectorField::Deletion,
            ] {
                selectors.push(
                    ControllerSelector::new(resource_type.clone(), field, None)
                        .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?,
                );
            }
            let execution = ControllerExecutionPolicy::new(
                16,
                4,
                256,
                8,
                256,
                ResyncPolicy::new(
                    Some(registration.repair_interval_secs * 1_000),
                    registration.repair_interval_secs * 1_000,
                )
                .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?,
            )
            .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?;
            let descriptor = ControllerDescriptor::new(
                identity,
                vec![registration_body],
                vec!["resource-api".to_owned()],
                vec!["system".to_owned()],
                vec![
                    ControllerVerb::ReadSpec,
                    ControllerVerb::ReadStatus,
                    ControllerVerb::WriteStatus,
                    ControllerVerb::AddFinalizer,
                    ControllerVerb::RemoveFinalizer,
                ],
                selectors,
                Vec::new(),
                true,
                vec![registration.finalizer.to_owned()],
                vec!["d2b.resource.v3".to_owned()],
                vec!["resources.d2bus.org/v3".to_owned()],
                execution,
            )
            .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?;
            Ok((registration, descriptor))
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SharedVolumeResourceKind {
    Volume,
    Binding,
}

impl SharedVolumeResourceKind {
    fn from_registration(
        registration: SharedVolumeRunnerRegistration,
    ) -> Result<Self, super::ResourceRuntimeError> {
        match (
            registration.controller_ref,
            registration.provider_ref,
            registration.resource_type,
        ) {
            (VOLUME_LOCAL_CONTROLLER_REF, VOLUME_LOCAL_PROVIDER_REF, "Volume") => Ok(Self::Volume),
            (
                VOLUME_VIRTIOFS_CONTROLLER_REF,
                VOLUME_VIRTIOFS_PROVIDER_REF,
                VOLUME_BINDING_RESOURCE_TYPE,
            ) => Ok(Self::Binding),
            _ => Err(super::ResourceRuntimeError::HandlerNotReady),
        }
    }

    const fn effect_id(self) -> &'static str {
        match self {
            Self::Volume => "volume-local",
            Self::Binding => "volume-virtiofs-binding",
        }
    }

    const fn provider_ref(self) -> &'static str {
        match self {
            Self::Volume => VOLUME_LOCAL_PROVIDER_REF,
            Self::Binding => VOLUME_VIRTIOFS_PROVIDER_REF,
        }
    }

    const fn resource_type(self) -> &'static str {
        match self {
            Self::Volume => "Volume",
            Self::Binding => VOLUME_BINDING_RESOURCE_TYPE,
        }
    }
}

#[derive(Clone)]
struct SharedVolumeEffectContext {
    identity: ControllerIdentity,
    target: ResourceKey,
    operation_id: String,
}

enum SharedVolumeEffectPhase {
    Ready,
    Pending,
    /// A terminal admission or reconcile failure surfaced per KTD5.
    Failed,
}

struct SharedVolumeEffectResult {
    phase: SharedVolumeEffectPhase,
    resource_projection: Option<Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedVolumeEffectError {
    Unavailable,
    InvalidResource,
}

impl core::fmt::Display for SharedVolumeEffectError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::Unavailable => "shared-volume-effect-unavailable",
            Self::InvalidResource => "shared-volume-resource-invalid",
        })
    }
}

impl std::error::Error for SharedVolumeEffectError {}

#[async_trait]
trait SharedVolumeEffectExecutor: Send + Sync {
    async fn reconcile(
        &self,
        kind: SharedVolumeResourceKind,
        context: &SharedVolumeEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<SharedVolumeEffectPhase, SharedVolumeEffectError>;

    async fn reconcile_with_projection(
        &self,
        kind: SharedVolumeResourceKind,
        context: &SharedVolumeEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<SharedVolumeEffectResult, SharedVolumeEffectError> {
        self.reconcile(kind, context, resource)
            .await
            .map(|phase| SharedVolumeEffectResult {
                phase,
                resource_projection: None,
            })
    }

    async fn finalize(
        &self,
        kind: SharedVolumeResourceKind,
        context: &SharedVolumeEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<(), SharedVolumeEffectError>;
}

/// Production typed storage Provider effects.
pub(crate) struct DaemonVolumeProviderEffects {
    state: Arc<crate::ServerState>,
    zone: ZoneId,
}

impl DaemonVolumeProviderEffects {
    pub(crate) fn new(state: Arc<crate::ServerState>, zone: ZoneId) -> Self {
        Self { state, zone }
    }

    fn runtime(&self) -> Result<Arc<ZoneResourceRuntime>, SharedVolumeEffectError> {
        self.state
            .resource_plane
            .lock()
            .ok()
            .and_then(|plane| plane.as_ref().and_then(|plane| plane.zone(&self.zone).ok()))
            .ok_or(SharedVolumeEffectError::Unavailable)
    }

    fn validate(
        &self,
        kind: SharedVolumeResourceKind,
        context: &SharedVolumeEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<Value, SharedVolumeEffectError> {
        if context.target != *resource.key()
            || context.identity.zone() != resource.key().zone()
            || resource.key().zone() != &self.zone
            || resource.key().resource_ref().resource_type().as_str() != kind.resource_type()
            || context.identity.controller_ref()
                != &ResourceRef::parse(match kind {
                    SharedVolumeResourceKind::Volume => VOLUME_LOCAL_CONTROLLER_REF,
                    SharedVolumeResourceKind::Binding => VOLUME_VIRTIOFS_CONTROLLER_REF,
                })
                .map_err(|_| SharedVolumeEffectError::InvalidResource)?
        {
            return Err(SharedVolumeEffectError::InvalidResource);
        }
        let value = serde_json::from_slice::<Value>(resource.canonical_json())
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        if value.pointer("/spec/providerRef").and_then(Value::as_str) != Some(kind.provider_ref()) {
            return Err(SharedVolumeEffectError::InvalidResource);
        }
        Ok(value)
    }

    async fn stored(
        &self,
        runtime: &ZoneResourceRuntime,
        resource: &ResourceSnapshot,
    ) -> Result<StoredResource, SharedVolumeEffectError> {
        runtime
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "u7-volume-stored-resource".to_owned(),
                    idempotency_key: None,
                    correlation_id: "u7-volume-stored-resource".to_owned(),
                    trace_id: None,
                    deadline_ms: 30_000,
                },
                zone: self.zone.clone(),
                target: resource.key().resource_ref().clone(),
                expected_uid: Some(resource.key().uid().clone()),
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| SharedVolumeEffectError::Unavailable)
    }

    async fn committed_resource(
        &self,
        runtime: &ZoneResourceRuntime,
        target: ResourceRef,
        operation_id: &str,
    ) -> Result<StoredResource, SharedVolumeEffectError> {
        let resource = runtime
            .store
            .get(StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: operation_id.to_owned(),
                    idempotency_key: None,
                    correlation_id: operation_id.to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: self.zone.clone(),
                target,
                expected_uid: None,
                projection: StoreProjection::Full,
            })
            .await
            .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        if resource.zone != self.zone {
            return Err(SharedVolumeEffectError::InvalidResource);
        }
        Ok(resource)
    }

    fn volume_spec(value: &Value) -> Result<VolumeSpec, SharedVolumeEffectError> {
        let mut spec = value
            .get("spec")
            .cloned()
            .ok_or(SharedVolumeEffectError::InvalidResource)?;
        let object = spec
            .as_object_mut()
            .ok_or(SharedVolumeEffectError::InvalidResource)?;
        for field in ["providerRef", "updatePolicy", "provider"] {
            object.remove(field);
        }
        serde_json::from_value(spec).map_err(|_| SharedVolumeEffectError::InvalidResource)
    }

    fn child_resource(
        zone: &ZoneId,
        target: &ResourceRef,
        owner: &ResourceRef,
        spec: Value,
    ) -> Result<Vec<u8>, SharedVolumeEffectError> {
        let status_resource = if target.resource_type().as_str() == VOLUME_BINDING_RESOURCE_TYPE {
            // Fail-closed typed binding projection: the zero UID never
            // matches a live fence, so only virtiofs's fenced projection
            // (KTD3) can ever report readiness.
            serde_json::to_value(VolumeBindingStatusResource {
                ready: false,
                fence: VolumeBindingReadinessFence {
                    uid: ResourceUid::parse("00000000-0000-4000-8000-000000000000")
                        .map_err(|_| SharedVolumeEffectError::InvalidResource)?,
                    generation: ResourceGeneration::new(1)
                        .map_err(|_| SharedVolumeEffectError::InvalidResource)?,
                    revision: d2b_contracts_resource::v3::ZoneRevision::new(0),
                },
                reason: None,
            })
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?
        } else {
            json!({})
        };
        let value = json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": target.resource_type().as_str(),
            "metadata": {
                "name": target.name().as_str(),
                "zone": zone.as_str(),
                "ownerRef": owner.to_canonical_string(),
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "1970-01-01T00:00:00.000Z",
                "updatedAt": "1970-01-01T00:00:00.000Z",
                "generation": 1,
                "revision": 1,
                "managedBy": "controller"
            },
            "spec": spec,
            "status": {
                "observedGeneration": 0,
                "phase": "Pending",
                "conditions": [],
                "lastReconciledAt": null,
                "startedAt": null,
                "completedAt": null,
                "outcome": null,
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
                },
                "resource": status_resource
            }
        });
        let canonical = CanonicalJsonValue::parse(
            &serde_json::to_vec(&value).map_err(|_| SharedVolumeEffectError::InvalidResource)?,
        )
        .map(|value| value.to_canonical_bytes())
        .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let mut validation_value = value;
        validation_value["metadata"]["uid"] =
            serde_json::Value::String("00000000-0000-4000-8000-000000000000".to_owned());
        let validation_bytes = serde_json::to_vec(&validation_value)
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        if let Err(error) =
            d2b_contracts_resource::v3::ResourceEnvelope::from_json(&validation_bytes)
        {
            tracing::warn!(
                resource = %target.to_canonical_string(),
                error = %error,
                "U7 child payload failed local envelope validation",
            );
            if let Err(error) =
                serde_json::from_slice::<d2b_contracts_resource::v3::ResourceEnvelope>(
                    &validation_bytes,
                )
            {
                tracing::warn!(
                    resource = %target.to_canonical_string(),
                    serde_error = %error,
                    "U7 child payload serde validation detail",
                );
            }
            if let Some(metadata) = validation_value.get("metadata")
                && let Err(error) =
                    serde_json::from_value::<d2b_contracts_resource::v3::ResourceMetadata>(
                        metadata.clone(),
                    )
            {
                tracing::warn!(
                    resource = %target.to_canonical_string(),
                    serde_error = %error,
                    "U7 child metadata validation detail",
                );
            }
            if let Some(spec) = validation_value.get("spec")
                && let Err(error) =
                    serde_json::from_value::<d2b_contracts_resource::v3::ResourceSpec>(
                        spec.clone(),
                    )
            {
                tracing::warn!(
                    resource = %target.to_canonical_string(),
                    serde_error = %error,
                    "U7 child spec validation detail",
                );
            }
            if let Some(status) = validation_value.get("status")
                && let Err(error) =
                    serde_json::from_value::<d2b_contracts_resource::v3::ResourceStatus>(
                        status.clone(),
                    )
            {
                tracing::warn!(
                    resource = %target.to_canonical_string(),
                    serde_error = %error,
                    "U7 child status validation detail",
                );
            }
        }
        Ok(canonical)
    }

    fn owned_intent(
        zone: &ZoneId,
        target: ResourceRef,
        owner: &ResourceRef,
        spec: Value,
        dependencies: impl IntoIterator<Item = ResourceRef>,
    ) -> Result<d2b_core_controller::OwnedChildIntent, SharedVolumeEffectError> {
        let canonical = Self::child_resource(zone, &target, owner, spec)?;
        let digest = d2b_core_controller::semantic_child_digest(&canonical)
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        d2b_core_controller::OwnedChildIntent::new(target, canonical, digest)
            .and_then(|child| child.with_dependencies(dependencies))
            .map_err(|_| SharedVolumeEffectError::InvalidResource)
    }

    fn binding_children(
        &self,
        zone: &ZoneId,
        binding_ref: &ResourceRef,
        binding: &StoredBinding,
        plan: &VirtiofsdWorkerPlan,
        principal: &BoundedToken,
    ) -> Result<Vec<d2b_core_controller::OwnedChildIntent>, SharedVolumeEffectError> {
        let process_ref = binding
            .worker_process_ref()
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let endpoint_ref = binding
            .endpoint_ref()
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let process_spec = json!({
            "providerRef": "Provider/system-minijail",
            "executionRef": "Host/host-system",
            "domain": "system",
            "processClass": "worker",
            "template": d2b_provider_volume_virtiofs::WORKER_TEMPLATE,
            "userRef": format!("User/{}", principal.as_str()),
            "threadPoolSize": plan.thread_pool_size,
            "readonly": plan.readonly,
            "cache": serde_json::to_value(plan.cache)
                .map_err(|_| SharedVolumeEffectError::InvalidResource)?,
            "volumeRef": binding.spec().volume_ref().to_canonical_string(),
            "view": binding.spec().view().as_str(),
            "mountPath": binding.spec().mount_path(),
            "desiredLifecycle": "running",
            "sandbox": {
                "capabilityClasses": [],
                "startRoot": false,
                "namespaceClasses": ["user"],
                "noNewPrivileges": true
            }
        });
        let endpoint_spec = json!({
            "providerRef": VOLUME_VIRTIOFS_PROVIDER_REF,
            "producerRef": process_ref.to_canonical_string(),
            "endpointClass": "service",
            "transport": "vhost-user-fs",
            "purpose": "virtiofsd",
            "locality": "host-local",
            "visibility": "private"
        });
        Ok(vec![
            Self::owned_intent(zone, process_ref, binding_ref, process_spec, [])?,
            Self::owned_intent(
                zone,
                endpoint_ref,
                binding_ref,
                endpoint_spec,
                [binding
                    .worker_process_ref()
                    .map_err(|_| SharedVolumeEffectError::InvalidResource)?],
            )?,
        ])
    }

    fn volume_children(
        zone: &ZoneId,
        volume_ref: &ResourceRef,
        spec: &VolumeSpec,
    ) -> Result<Vec<d2b_core_controller::OwnedChildIntent>, SharedVolumeEffectError> {
        desired_binding_intents(volume_ref.clone(), spec, false)
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?
            .into_iter()
            .map(|intent| {
                // Neutral binding payload only (KTD1): access mode and
                // mount intent.  The envelope carries no provider
                // extension or settings: the standard catalog admits no
                // extension path for the type, and the serving posture
                // is the frozen default.
                let binding = VolumeBindingSpec::new(
                    intent.volume_ref().clone(),
                    intent.execution_ref().clone(),
                    intent.view().as_str(),
                    intent.access(),
                    intent.mount_path(),
                )
                .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
                let mut binding_spec = serde_json::to_value(&binding)
                    .map_err(|_| SharedVolumeEffectError::InvalidResource)?
                    .as_object_mut()
                    .ok_or(SharedVolumeEffectError::InvalidResource)?
                    .clone();
                binding_spec.insert(
                    "providerRef".to_owned(),
                    Value::String(VOLUME_VIRTIOFS_PROVIDER_REF.to_owned()),
                );
                Self::owned_intent(
                    zone,
                    ResourceRef::new(
                        ResourceTypeName::parse(VOLUME_BINDING_RESOURCE_TYPE.to_owned())
                            .map_err(|_| SharedVolumeEffectError::InvalidResource)?,
                        ResourceName::parse(intent.name().as_str())
                            .map_err(|_| SharedVolumeEffectError::InvalidResource)?,
                    ),
                    volume_ref,
                    Value::Object(binding_spec),
                    [],
                )
            })
            .collect()
    }

    async fn reconcile_volume(
        &self,
        context: &SharedVolumeEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<SharedVolumeEffectResult, SharedVolumeEffectError> {
        let value = self.validate(SharedVolumeResourceKind::Volume, context, resource)?;
        let spec = Self::volume_spec(&value)?;
        let runtime = self.runtime()?;
        let provider = value.pointer("/spec/provider");
        let owner_ref = value
            .pointer("/metadata/ownerRef")
            .and_then(Value::as_str)
            .and_then(|owner| ResourceRef::parse(owner).ok());
        let nix_identity = (spec.source().settings().kind() == SourceKind::NixClosure)
            .then(|| nix_closure_volume_identity(resource.key().resource_ref(), &value, &spec))
            .transpose()
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let guest_ref = nix_identity
            .as_ref()
            .map(|identity| identity.guest_ref.clone());
        let nix_closure_role = nix_identity.as_ref().map(|identity| identity.role);
        let resolver = DaemonVolumeRootResolver::new(
            &self.state,
            self.zone.clone(),
            resource.key().resource_ref().clone(),
            resource.key().uid().clone(),
            guest_ref,
            nix_closure_role,
        )
        .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        let adapter = AnchoredVolumeEffectAdapter::new(resolver);
        let controller =
            VolumeLocalController::new(VolumeLocalProfile::shipped(), &adapter, &adapter);
        let report = controller
            .reconcile(resource.key().uid(), &spec, provider, owner_ref.as_ref())
            .await
            .map_err(|error| {
                tracing::warn!(
                    resource = %resource.key().resource_ref().to_canonical_string(),
                    error = ?error,
                    "U7 VolumeLocal reconcile failed",
                );
                SharedVolumeEffectError::Unavailable
            })?;
        let desired = Self::volume_children(&self.zone, resource.key().resource_ref(), &spec)?;
        let owner = self.stored(&runtime, resource).await?;
        let client = runtime
            .status_client()
            .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        let converged = crate::binding_child_resource_runtime::reconcile_owned_children(
            &runtime.store,
            client.as_ref(),
            &self.zone,
            &[crate::binding_child_resource_runtime::OwnedChildOwner {
                resource: owner.clone(),
                desired: Some(desired.clone()),
                fenced: false,
            }],
        )
        .await
        .map_err(|error| {
            tracing::warn!(
                resource = %resource.key().resource_ref().to_canonical_string(),
                error = ?error,
                "U7 Volume child reconciliation failed",
            );
            SharedVolumeEffectError::Unavailable
        })?;
        let phase = if report.layout_phase == d2b_provider_volume_local::LayoutPhase::Ready
            && converged.contains(resource.key().resource_ref())
        {
            SharedVolumeEffectPhase::Ready
        } else {
            SharedVolumeEffectPhase::Pending
        };
        let resource_projection =
            serde_json::to_value(&report).map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        Ok(SharedVolumeEffectResult {
            phase,
            resource_projection: Some(resource_projection),
        })
    }

    async fn reconcile_binding(
        &self,
        context: &SharedVolumeEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<SharedVolumeEffectResult, SharedVolumeEffectError> {
        let value = self.validate(SharedVolumeResourceKind::Binding, context, resource)?;
        // U4: the reconciler input is the stored binding envelope
        // itself, parsed strictly against the neutral binding contract.
        let binding = match StoredBinding::from_resource_spec(&value) {
            Ok(binding) => binding,
            Err(reason) => return Ok(Self::failed_binding_result(resource, reason)),
        };
        let runtime = self.runtime()?;
        let volume_resource = self
            .committed_resource(
                &runtime,
                binding.spec().volume_ref().clone(),
                &context.operation_id,
            )
            .await?;
        if volume_resource.resource_ref != *binding.spec().volume_ref() {
            return Err(SharedVolumeEffectError::InvalidResource);
        }
        // Dependency-only Volume read (AE2): the envelope is parsed and
        // the view resolved; nothing in this path writes the Volume.
        let volume_value = serde_json::from_slice::<Value>(&volume_resource.canonical_json)
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let volume_spec = Self::volume_spec(&volume_value)?;
        let guest_value = runtime
            .committed_resource_value(
                &binding.spec().execution_ref().clone(),
                &context.operation_id,
            )
            .await
            .ok();
        let vcpu_count = guest_value
            .as_ref()
            .and_then(|value| value.pointer("/spec/provider/settings/vcpus"))
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
            .unwrap_or(1);
        let principal = binding
            .worker_principal()
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let view = match d2b_provider_volume_virtiofs::resolve_view(&volume_spec, &binding) {
            Ok(view) => view,
            Err(reason) => return Ok(Self::failed_binding_result(resource, reason)),
        };
        let plan =
            match VirtiofsdWorkerPlan::for_binding(&binding, view, vcpu_count, principal.clone()) {
                Ok(plan) => plan,
                Err(reason) => return Ok(Self::failed_binding_result(resource, reason)),
            };
        let store_view_marker_ready = if binding.spec().view().as_str() == "ro-store" {
            let nix_identity =
                (volume_spec.source().settings().kind() == SourceKind::NixClosure)
                .then(|| {
                    nix_closure_volume_identity(
                        binding.spec().volume_ref(),
                        &volume_value,
                        &volume_spec,
                    )
                })
                .transpose()
                .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
            if volume_spec.source().settings().kind() == SourceKind::NixClosure
                && nix_identity
                    .as_ref()
                    .is_none_or(|identity| identity.role != NixClosureVolumeRole::StoreView)
            {
                return Err(SharedVolumeEffectError::InvalidResource);
            }
            let guest_ref = (volume_spec.source().settings().kind() == SourceKind::NixClosure)
                .then(|| {
                    nix_identity
                        .as_ref()
                        .map(|identity| identity.guest_ref.clone())
                })
                .flatten();
            if volume_spec.source().settings().kind() == SourceKind::NixClosure
                && guest_ref.as_ref() != Some(binding.spec().execution_ref())
            {
                return Err(SharedVolumeEffectError::InvalidResource);
            }
            let nix_closure_role = nix_identity
                .as_ref()
                .map(|identity| identity.role);
            let resolver = DaemonVolumeRootResolver::new(
                &self.state,
                self.zone.clone(),
                binding.spec().volume_ref().clone(),
                volume_resource.uid.clone(),
                guest_ref,
                nix_closure_role,
            )
            .map_err(|_| SharedVolumeEffectError::Unavailable)?;
            let adapter = AnchoredVolumeEffectAdapter::new(resolver);
            let settings = volume_spec.source().settings();
            let root = adapter
                .resolve_root_for(
                    &volume_resource.uid,
                    settings.source_policy_id(),
                    settings.system_artifact_id(),
                    settings.kind(),
                )
                .await
                .map_err(|_| SharedVolumeEffectError::Unavailable)?;
            let guest = BoundedToken::parse(
                binding.spec().execution_ref().name().as_str().to_owned(),
            )
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
            let evidence = adapter
                .observe_store_view_marker(&root, &marker_path(&guest))
                .await
                .map_err(|_| SharedVolumeEffectError::Unavailable)?;
            evidence.present && evidence.zero_length
        } else {
            true
        };
        if binding.spec().view().as_str() == "ro-store" && !store_view_marker_ready {
            return Ok(SharedVolumeEffectResult {
                phase: SharedVolumeEffectPhase::Pending,
                resource_projection: None,
            });
        }
        let desired = self.binding_children(
            &self.zone,
            context.target.resource_ref(),
            &binding,
            &plan,
            &principal,
        )?;
        let owner = self.stored(&runtime, resource).await?;
        let client = runtime
            .status_client()
            .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        let converged = crate::binding_child_resource_runtime::reconcile_owned_children(
            &runtime.store,
            &client,
            &self.zone,
            &[crate::binding_child_resource_runtime::OwnedChildOwner {
                resource: owner.clone(),
                desired: Some(desired.clone()),
                fenced: false,
            }],
        )
        .await
        .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        if !converged.contains(context.target.resource_ref()) {
            return Ok(SharedVolumeEffectResult {
                phase: SharedVolumeEffectPhase::Pending,
                resource_projection: None,
            });
        }
        let children = crate::binding_child_resource_runtime::list_binding_children(
            &runtime.store,
            &self.zone,
        )
        .await
        .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        let child_owner = crate::binding_child_resource_runtime::OwnedChildOwner {
            resource: owner,
            desired: Some(desired),
            fenced: false,
        };
        if !crate::binding_child_resource_runtime::owned_children_ready(&child_owner, &children) {
            return Ok(SharedVolumeEffectResult {
                phase: SharedVolumeEffectPhase::Pending,
                resource_projection: None,
            });
        }
        let process_ref = binding
            .worker_process_ref()
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let endpoint_ref = binding
            .endpoint_ref()
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let process_ready = child_phase(&children, &process_ref).as_deref() == Some("Ready");
        let endpoint_ready = child_phase(&children, &endpoint_ref).as_deref() == Some("Ready");
        let zone_token = BoundedToken::parse(self.zone.as_str().to_owned())
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let port = ChildReadinessPort {
            process_ref,
            endpoint_ref,
            socket_ready: process_ready,
            guest_mount_ready: endpoint_ready,
            store_view_marker: store_view_marker_ready,
            zone: zone_token,
            fence: binding.fence(),
        };
        let report = VirtiofsBindingController::new(port)
            .reconcile(&binding, &volume_spec, vcpu_count, principal)
            .await
            .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        let phase = match report.phase {
            d2b_provider_volume_virtiofs::BindingPhase::Ready => SharedVolumeEffectPhase::Ready,
            d2b_provider_volume_virtiofs::BindingPhase::Failed => SharedVolumeEffectPhase::Failed,
            _ => SharedVolumeEffectPhase::Pending,
        };
        // KTD3: the typed fenced projection travels with every
        // reconcile so the binding status carries it.
        let projection = serde_json::to_value(&report.projection)
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        Ok(SharedVolumeEffectResult {
            phase,
            resource_projection: Some(projection),
        })
    }

    /// A terminal admission failure surfaced per KTD5: Failed phase with
    /// a stable reason, fenced by the stored binding identity. The
    /// projection never reports ready.
    fn failed_binding_result(
        resource: &ResourceSnapshot,
        reason: d2b_provider_volume_virtiofs::VirtiofsBindingError,
    ) -> SharedVolumeEffectResult {
        let projection = VolumeBindingStatusResource {
            ready: false,
            fence: VolumeBindingReadinessFence {
                uid: resource.key().uid().clone(),
                generation: resource.generation(),
                revision: resource.revision(),
            },
            reason: Some(
                StatusCode::parse(reason.code())
                    .expect("frozen binding error codes are valid status codes"),
            ),
        };
        let resource_projection =
            serde_json::to_value(&projection).expect("typed projection serializes");
        SharedVolumeEffectResult {
            phase: SharedVolumeEffectPhase::Failed,
            resource_projection: Some(resource_projection),
        }
    }

    async fn finalize_volume(
        &self,
        context: &SharedVolumeEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<(), SharedVolumeEffectError> {
        let value = self.validate(SharedVolumeResourceKind::Volume, context, resource)?;
        let spec = Self::volume_spec(&value)?;
        let runtime = self.runtime()?;
        let owner = self.stored(&runtime, resource).await?;
        let client = runtime
            .status_client()
            .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        let converged = crate::binding_child_resource_runtime::reconcile_owned_children(
            &runtime.store,
            &client,
            &self.zone,
            &[crate::binding_child_resource_runtime::OwnedChildOwner {
                resource: owner,
                desired: None,
                fenced: false,
            }],
        )
        .await
        .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        if !converged.contains(context.target.resource_ref()) {
            return Err(SharedVolumeEffectError::Unavailable);
        }
        let nix_identity = (spec.source().settings().kind() == SourceKind::NixClosure)
            .then(|| nix_closure_volume_identity(resource.key().resource_ref(), &value, &spec))
            .transpose()
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let guest_ref = nix_identity
            .as_ref()
            .map(|identity| identity.guest_ref.clone());
        let nix_closure_role = nix_identity.as_ref().map(|identity| identity.role);
        let resolver = DaemonVolumeRootResolver::new(
            &self.state,
            self.zone.clone(),
            resource.key().resource_ref().clone(),
            resource.key().uid().clone(),
            guest_ref,
            nix_closure_role,
        )
        .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        let adapter = AnchoredVolumeEffectAdapter::new(resolver);
        let controller =
            VolumeLocalController::new(VolumeLocalProfile::shipped(), &adapter, &adapter);
        controller
            .cleanup(context.target.uid(), &spec)
            .await
            .map(|_| ())
            .map_err(|_| SharedVolumeEffectError::Unavailable)
    }

    async fn finalize_binding(
        &self,
        context: &SharedVolumeEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<(), SharedVolumeEffectError> {
        let value = self.validate(SharedVolumeResourceKind::Binding, context, resource)?;
        let binding = StoredBinding::from_resource_spec(&value)
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let runtime = self.runtime()?;
        let owner = self.stored(&runtime, resource).await?;
        let client = runtime
            .status_client()
            .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        // The owned worker and Endpoint are drained first; the drain
        // converges only once both children are gone.
        let converged = crate::binding_child_resource_runtime::reconcile_owned_children(
            &runtime.store,
            &client,
            &self.zone,
            &[crate::binding_child_resource_runtime::OwnedChildOwner {
                resource: owner,
                desired: None,
                fenced: false,
            }],
        )
        .await
        .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        if !converged.contains(context.target.resource_ref()) {
            return Err(SharedVolumeEffectError::Unavailable);
        }
        // KTD6: the controller drain check gates finalizer removal — a
        // guest mount that is still present blocks the drain (AE6). The
        // mount observation is the published Endpoint: it is only Ready
        // while the guest-side share is served.
        let children = crate::binding_child_resource_runtime::list_binding_children(
            &runtime.store,
            &self.zone,
        )
        .await
        .map_err(|_| SharedVolumeEffectError::Unavailable)?;
        let endpoint_ref = binding
            .endpoint_ref()
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let zone_token = BoundedToken::parse(self.zone.as_str().to_owned())
            .map_err(|_| SharedVolumeEffectError::InvalidResource)?;
        let port = ChildReadinessPort {
            process_ref: binding
                .worker_process_ref()
                .map_err(|_| SharedVolumeEffectError::InvalidResource)?,
            endpoint_ref: endpoint_ref.clone(),
            socket_ready: false,
            guest_mount_ready: child_phase(&children, &endpoint_ref).as_deref()
                == Some("Ready"),
            store_view_marker: true,
            zone: zone_token.clone(),
            fence: binding.fence(),
        };
        let worker = LaunchedWorker {
            process_ref: binding
                .worker_process_ref()
                .map_err(|_| SharedVolumeEffectError::InvalidResource)?,
            socket: binding.socket_identity(&zone_token),
        };
        VirtiofsBindingController::new(port)
            .drain(&binding, &worker)
            .await
            .map_err(|_| SharedVolumeEffectError::Unavailable)
    }
}

#[async_trait]
impl SharedVolumeEffectExecutor for DaemonVolumeProviderEffects {
    async fn reconcile(
        &self,
        kind: SharedVolumeResourceKind,
        context: &SharedVolumeEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<SharedVolumeEffectPhase, SharedVolumeEffectError> {
        match kind {
            SharedVolumeResourceKind::Volume => self
                .reconcile_volume(context, resource)
                .await
                .map(|result| result.phase),
            SharedVolumeResourceKind::Binding => {
                self.reconcile_binding(context, resource).await.map(|result| result.phase)
            }
        }
    }

    async fn reconcile_with_projection(
        &self,
        kind: SharedVolumeResourceKind,
        context: &SharedVolumeEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<SharedVolumeEffectResult, SharedVolumeEffectError> {
        match kind {
            SharedVolumeResourceKind::Volume => self.reconcile_volume(context, resource).await,
            SharedVolumeResourceKind::Binding => self.reconcile_binding(context, resource).await,
        }
    }

    async fn finalize(
        &self,
        kind: SharedVolumeResourceKind,
        context: &SharedVolumeEffectContext,
        resource: &ResourceSnapshot,
    ) -> Result<(), SharedVolumeEffectError> {
        match kind {
            SharedVolumeResourceKind::Volume => self.finalize_volume(context, resource).await,
            SharedVolumeResourceKind::Binding => self.finalize_binding(context, resource).await,
        }
    }
}

struct ChildReadinessPort {
    process_ref: ResourceRef,
    endpoint_ref: ResourceRef,
    socket_ready: bool,
    guest_mount_ready: bool,
    store_view_marker: bool,
    zone: BoundedToken,
    /// The stored binding identity the fenced projection is validated
    /// against (KTD3 server-side fence validation).
    fence: VolumeBindingReadinessFence,
}

impl VirtiofsBindingEffectPort for ChildReadinessPort {
    async fn launch_worker(
        &self,
        binding: &StoredBinding,
        _plan: &VirtiofsdWorkerPlan,
    ) -> Result<LaunchedWorker, d2b_provider_volume_virtiofs::VirtiofsBindingError> {
        Ok(LaunchedWorker {
            process_ref: self.process_ref.clone(),
            socket: binding.socket_identity(&self.zone),
        })
    }

    async fn observe_socket(
        &self,
        _worker: &LaunchedWorker,
    ) -> Result<bool, d2b_provider_volume_virtiofs::VirtiofsBindingError> {
        Ok(self.socket_ready)
    }

    async fn observe_guest_mount(
        &self,
        _binding: &StoredBinding,
    ) -> Result<bool, d2b_provider_volume_virtiofs::VirtiofsBindingError> {
        Ok(self.guest_mount_ready)
    }

    async fn observe_store_view_marker(
        &self,
        _binding: &StoredBinding,
    ) -> Result<bool, d2b_provider_volume_virtiofs::VirtiofsBindingError> {
        Ok(self.store_view_marker)
    }

    async fn delete_worker(
        &self,
        _worker: &LaunchedWorker,
    ) -> Result<(), d2b_provider_volume_virtiofs::VirtiofsBindingError> {
        let _ = &self.endpoint_ref;
        Ok(())
    }

    async fn write_binding_status(
        &self,
        writer: &BoundedToken,
        binding: &StoredBinding,
        _projection: &VolumeBindingStatusResource,
    ) -> Result<(), d2b_provider_volume_virtiofs::VirtiofsBindingError> {
        // Server-side validation at the controller boundary (KTD3):
        // only the virtiofs controller identity may write, and only
        // under the fence of the stored binding it reconciled.
        if writer.as_str() != "volume-virtiofs" {
            return Err(d2b_provider_volume_virtiofs::VirtiofsBindingError::UnauthorizedWriter);
        }
        if !binding.fence().matches(
            &self.fence.uid,
            self.fence.generation,
            self.fence.revision,
        ) {
            return Err(d2b_provider_volume_virtiofs::VirtiofsBindingError::StaleFence);
        }
        Ok(())
    }
}

fn child_phase(children: &[StoredResource], target: &ResourceRef) -> Option<String> {
    children
        .iter()
        .find(|child| child.resource_ref == *target)
        .and_then(|child| {
            serde_json::from_slice::<Value>(&child.canonical_json)
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/status/phase")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned)
                })
        })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum NixClosureVolumeRole {
    StoreView,
    SystemVolume,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NixClosureVolumeIdentity {
    guest_ref: ResourceRef,
    role: NixClosureVolumeRole,
}

fn nix_closure_volume_identity(
    volume_ref: &ResourceRef,
    value: &Value,
    spec: &VolumeSpec,
) -> Result<NixClosureVolumeIdentity, d2b_provider_volume_local::VolumeLocalError> {
    let owner_ref = value
        .pointer("/metadata/ownerRef")
        .and_then(Value::as_str)
        .map(|owner| {
            ResourceRef::parse(owner)
                .map_err(|_| d2b_provider_volume_local::VolumeLocalError::InvalidSpec)
        })
        .transpose()?;
    let mut attachment_guest = None;
    for attachment in spec.attachments() {
        if attachment.execution_ref().resource_type().as_str() != "Guest" {
            return Err(d2b_provider_volume_local::VolumeLocalError::InvalidSpec);
        }
        if attachment_guest
            .replace(attachment.execution_ref().clone())
            .is_some_and(|guest| guest != *attachment.execution_ref())
        {
            return Err(d2b_provider_volume_local::VolumeLocalError::InvalidSpec);
        }
    }

    match (owner_ref, attachment_guest) {
        (Some(owner), None)
            if owner.resource_type().as_str() == "Guest"
                && volume_ref.name().as_str() == format!("{}-system", owner.name().as_str()) =>
        {
            Ok(NixClosureVolumeIdentity {
                guest_ref: owner,
                role: NixClosureVolumeRole::SystemVolume,
            })
        }
        (None, Some(guest))
            if volume_ref.name().as_str() == format!("store-view-{}", guest.name().as_str()) =>
        {
            Ok(NixClosureVolumeIdentity {
                guest_ref: guest,
                role: NixClosureVolumeRole::StoreView,
            })
        }
        _ => Err(d2b_provider_volume_local::VolumeLocalError::InvalidSpec),
    }
}

fn validate_store_view_identity(
    zone: &ZoneId,
    guest_ref: &ResourceRef,
    system_artifact_id: &BoundedToken,
    descriptor_artifact_id: &str,
    intent: &d2b_core::bundle_resolver::ResolvedStoreViewIntent,
) -> Result<(), d2b_provider_volume_local::VolumeLocalError> {
    if guest_ref.resource_type().as_str() != "Guest"
        || intent.vm != guest_ref.name().as_str()
        || intent.intent_id
            != d2b_core::bundle_resolver::intent_id_store_view(zone, guest_ref.name().as_str())
        || descriptor_artifact_id != system_artifact_id.as_str()
    {
        return Err(d2b_provider_volume_local::VolumeLocalError::SourceUnresolved);
    }
    Ok(())
}

/// Trusted daemon-side resolver for bundle-selected Volume roots.
struct DaemonVolumeRootResolver {
    state: Arc<crate::ServerState>,
    resolver: d2b_core::bundle_resolver::BundleResolver,
    zone: ZoneId,
    volume_ref: ResourceRef,
    volume_uid: ResourceUid,
    guest_ref: Option<ResourceRef>,
    nix_closure_role: Option<NixClosureVolumeRole>,
}

impl DaemonVolumeRootResolver {
    fn new(
        state: &Arc<crate::ServerState>,
        zone: ZoneId,
        volume_ref: ResourceRef,
        volume_uid: ResourceUid,
        guest_ref: Option<ResourceRef>,
        nix_closure_role: Option<NixClosureVolumeRole>,
    ) -> Result<Self, super::ResourceRuntimeError> {
        Ok(Self {
            state: Arc::clone(state),
            resolver: crate::load_bundle_resolver(state)
                .map_err(|_| super::ResourceRuntimeError::ProviderPathUnavailable)?,
            zone,
            volume_ref,
            volume_uid,
            guest_ref,
            nix_closure_role,
        })
    }

    fn marker_root(&self) -> Result<OwnedFd, d2b_provider_volume_local::VolumeLocalError> {
        let marker_root = self
            .state
            .daemon_state_dir
            .parent()
            .map(|root| root.join("volume-local-markers"))
            .ok_or(d2b_provider_volume_local::VolumeLocalError::SourceUnresolved)?;
        open_anchored_directory(&marker_root)
            .map_err(|_| d2b_provider_volume_local::VolumeLocalError::SourceUnresolved)
    }

    fn source_unresolved(
        &self,
        stage: &'static str,
    ) -> d2b_provider_volume_local::VolumeLocalError {
        tracing::warn!(
            resource = %self.volume_ref.to_canonical_string(),
            stage,
            "U7 Volume source resolution failed",
        );
        d2b_provider_volume_local::VolumeLocalError::SourceUnresolved
    }

    fn sync_store_view(
        &self,
        guest_ref: &ResourceRef,
        intent: &d2b_core::bundle_resolver::ResolvedStoreViewIntent,
        generation_token: u32,
    ) -> Result<
        d2b_contracts_broker::broker_wire::StoreSyncResponse,
        d2b_provider_volume_local::VolumeLocalError,
    > {
        let response = crate::dispatch_broker_request_as(
            &self.state,
            d2b_contracts_broker::broker_wire::BrokerRequest::StoreSync(
                d2b_contracts_broker::broker_wire::StoreSyncRequest {
                    vm_id: d2b_contracts::types::VmId::new(guest_ref.name().as_str()),
                    bundle_closure_ref: d2b_contracts::types::BundleClosureRef::new(
                        intent.intent_id.clone(),
                    ),
                    generation_token,
                    tracing_span_id: None,
                },
            ),
            d2b_contracts_broker::broker_wire::BrokerCallerRole::AdminUid {
                uid: self.state.daemon_uid,
            },
        )
        .map_err(|_| d2b_provider_volume_local::VolumeLocalError::EffectFailed)?;
        match response {
            d2b_contracts_broker::broker_wire::BrokerResponse::StoreSync(response) => Ok(response),
            _ => Err(d2b_provider_volume_local::VolumeLocalError::EffectFailed),
        }
    }

    fn validate_store_sync_response(
        &self,
        intent: &d2b_core::bundle_resolver::ResolvedStoreViewIntent,
        generation_token: u32,
        response: &d2b_contracts_broker::broker_wire::StoreSyncResponse,
    ) -> Result<PathBuf, d2b_provider_volume_local::VolumeLocalError> {
        let expected_generation_id = d2b_host::hardlink_farm::generation_id(
            &intent.closure_paths,
            d2b_host::hardlink_farm::system_store_path(&intent.closure_paths),
        );
        let farm_path = PathBuf::from(&response.hardlink_farm_path);
        if response.vm != intent.vm
            || response.generation_id != expected_generation_id
            || response.generation_token != generation_token
            || response.closure_count
                != u32::try_from(intent.closure_paths.len()).unwrap_or(u32::MAX)
            || farm_path != intent.hardlink_farm_path
        {
            return Err(self.source_unresolved("store-sync-response"));
        }
        Ok(farm_path)
    }

    fn resolve_nix_closure_root(
        &self,
        system_artifact_id: &BoundedToken,
    ) -> Result<ResolvedVolumeRoot, d2b_provider_volume_local::VolumeLocalError> {
        let guest_ref = self
            .guest_ref
            .as_ref()
            .ok_or_else(|| self.source_unresolved("guest-reference"))?;
        let descriptor = self
            .resolver
            .guest_setup_descriptor_bytes(self.zone.as_str(), guest_ref.name().as_str())
            .ok_or_else(|| self.source_unresolved("guest-setup-descriptor"))?;
        let descriptor = serde_json::from_slice::<Value>(descriptor)
            .map_err(|_| self.source_unresolved("guest-setup-descriptor-decode"))?;
        let descriptor_artifact_id = descriptor
            .get("systemArtifactId")
            .and_then(Value::as_str)
            .ok_or_else(|| self.source_unresolved("guest-setup-artifact-id"))?;
        let intent = self
            .resolver
            .find_store_view_intent_for_zone(&self.zone, guest_ref.name().as_str())
            .ok_or_else(|| self.source_unresolved("store-view-intent"))?;
        validate_store_view_identity(
            &self.zone,
            guest_ref,
            system_artifact_id,
            descriptor_artifact_id,
            intent,
        )
        .map_err(|_| self.source_unresolved("store-view-identity"))?;
        if intent.vm != guest_ref.name().as_str() {
            return Err(self.source_unresolved("store-view-guest"));
        }
        let generation_token = u32::try_from(intent.generation)
            .map_err(|_| self.source_unresolved("store-view-generation"))?;
        if self.nix_closure_role == Some(NixClosureVolumeRole::SystemVolume) {
            let response = self.sync_store_view(guest_ref, intent, generation_token)?;
            let farm_path =
                self.validate_store_sync_response(intent, generation_token, &response)?;
            let file = open_anchored_directory(&farm_path)
                .map_err(|_| self.source_unresolved("store-view-system-open"))?;
            let marker_root = self.marker_root()?;
            return Ok(
                ResolvedVolumeRoot::new(file, self.volume_uid.clone())?
                    .with_marker_root(marker_root)?
                    .with_preexisting_state(),
            );
        }
        let response = self.sync_store_view(guest_ref, intent, generation_token)?;
        let farm_path = self.validate_store_sync_response(intent, generation_token, &response)?;
        let file = open_anchored_directory(&farm_path)
            .map_err(|_| self.source_unresolved("store-view-open"))?;
        let marker_root = self.marker_root()?;
        Ok(ResolvedVolumeRoot::new(file, self.volume_uid.clone())?
            .with_marker_root(marker_root)?
            .with_preexisting_state())
    }
}

impl VolumeRootResolver for DaemonVolumeRootResolver {
    fn resolve_root(
        &self,
        volume_uid: &ResourceUid,
        source_policy_id: Option<&BoundedToken>,
        system_artifact_id: Option<&BoundedToken>,
        kind: SourceKind,
    ) -> Result<ResolvedVolumeRoot, d2b_provider_volume_local::VolumeLocalError> {
        if volume_uid != &self.volume_uid || self.volume_ref.resource_type().as_str() != "Volume" {
            return Err(d2b_provider_volume_local::VolumeLocalError::SourceUnresolved);
        }
        if kind == SourceKind::NixClosure {
            if source_policy_id.is_some() {
                return Err(d2b_provider_volume_local::VolumeLocalError::InvalidSpec);
            }
            return self.resolve_nix_closure_root(
                system_artifact_id
                    .ok_or(d2b_provider_volume_local::VolumeLocalError::SourceUnresolved)?,
            );
        }
        let policy = source_policy_id
            .map(BoundedToken::as_str)
            .ok_or(d2b_provider_volume_local::VolumeLocalError::SourceUnresolved)?;
        let storage_id = if policy == "state-root" || policy == "default-state" {
            "path:state-root".to_owned()
        } else {
            format!("path:{policy}")
        };
        let path = self
            .resolver
            .find_storage_path_spec(&storage_id)
            .map(|spec| spec.path_template.as_str().to_owned())
            .ok_or_else(|| self.source_unresolved("storage-path"))?;
        let path = Path::new(&path);
        if !path.is_absolute()
            || path
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(self.source_unresolved("storage-path-safety"));
        }
        let file = open_anchored_directory(path)
            .map_err(|_| self.source_unresolved("storage-path-open"))?;
        let marker_file = self.marker_root()?;
        ResolvedVolumeRoot::new(file.into(), volume_uid.clone())?
            .with_marker_root(marker_file.into())
    }

    fn resolve_principal(
        &self,
        reference: &ResourceRef,
    ) -> Result<u32, d2b_provider_volume_local::VolumeLocalError> {
        if reference.resource_type().as_str() != "User" {
            return Err(d2b_provider_volume_local::VolumeLocalError::InvalidSpec);
        }
        nix::unistd::User::from_name(reference.name().as_str())
            .map_err(|_| d2b_provider_volume_local::VolumeLocalError::EffectFailed)?
            .map(|user| user.uid.as_raw())
            .ok_or(d2b_provider_volume_local::VolumeLocalError::EffectFailed)
    }

    fn resolve_group(
        &self,
        reference: &ResourceRef,
    ) -> Result<u32, d2b_provider_volume_local::VolumeLocalError> {
        if reference.resource_type().as_str() != "User" {
            return Err(d2b_provider_volume_local::VolumeLocalError::InvalidSpec);
        }
        nix::unistd::User::from_name(reference.name().as_str())
            .map_err(|_| d2b_provider_volume_local::VolumeLocalError::EffectFailed)?
            .map(|user| user.gid.as_raw())
            .ok_or(d2b_provider_volume_local::VolumeLocalError::EffectFailed)
    }
}

fn open_anchored_directory(path: &Path) -> std::io::Result<OwnedFd> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unanchored directory",
        ));
    }
    let mut current = open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        current = openat2(
            &current,
            name,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
            ResolveFlags::BENEATH
                | ResolveFlags::NO_SYMLINKS
                | ResolveFlags::NO_MAGICLINKS
                | ResolveFlags::NO_XDEV,
        )
        .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
    }
    Ok(current)
}

#[cfg(test)]
mod store_view_identity_tests {
    use super::{
        NixClosureVolumeRole, nix_closure_volume_identity, validate_store_view_identity,
    };
    use d2b_contracts_resource::v3::{ResourceRef, ZoneId, execution_policy::BoundedToken};
    use d2b_core::bundle_resolver::{ResolvedStoreViewIntent, intent_id_store_view};
    use d2b_provider_volume_local::testing::fixtures;
    use std::path::PathBuf;

    fn intent(zone: &ZoneId, guest: &str) -> ResolvedStoreViewIntent {
        ResolvedStoreViewIntent {
            intent_id: intent_id_store_view(zone, guest),
            vm: guest.to_owned(),
            generation: 1,
            hardlink_farm_path: PathBuf::from(format!(
                "/var/lib/d2b/zones/{}/guests/{guest}/store-view",
                zone.as_str()
            )),
            target_view_path: PathBuf::from(format!(
                "/var/lib/d2b/zones/{}/guests/{guest}/store-view/live/system",
                zone.as_str()
            )),
            closure_paths: vec![PathBuf::from("/nix/store/acceptance-system")],
            db_dump_path: PathBuf::from("/var/lib/d2b/closure.db"),
        }
    }

    #[test]
    fn store_view_identity_rejects_wrong_artifact_guest_and_zone() {
        let zone = ZoneId::parse("work").expect("Zone");
        let guest = ResourceRef::parse("Guest/acceptance-guest").expect("Guest");
        let artifact = BoundedToken::parse("acceptance-system").expect("artifact");
        let intent = intent(&zone, "acceptance-guest");
        assert!(
            validate_store_view_identity(&zone, &guest, &artifact, "acceptance-system", &intent)
                .is_ok()
        );
        assert!(
            validate_store_view_identity(&zone, &guest, &artifact, "other-system", &intent)
                .is_err()
        );
        let other_guest = ResourceRef::parse("Guest/other-guest").expect("Guest");
        assert!(
            validate_store_view_identity(
                &zone,
                &other_guest,
                &artifact,
                "acceptance-system",
                &intent
            )
            .is_err()
        );
        let other_zone = ZoneId::parse("personal").expect("Zone");
        assert!(
            validate_store_view_identity(
                &other_zone,
                &guest,
                &artifact,
                "acceptance-system",
                &intent
            )
            .is_err()
        );
    }

    #[test]
    fn only_the_canonical_store_view_name_can_claim_the_guest_farm() {
        let mut system_volume = serde_json::to_value(fixtures::nix_closure_store_view_volume())
            .expect("Volume fixture");
        system_volume["attachments"] = serde_json::json!([]);
        let system_spec = serde_json::from_value(system_volume).expect("system Volume spec");
        let system_value = serde_json::json!({
            "metadata": {
                "name": "work-vm-system",
                "ownerRef": "Guest/work-vm"
            }
        });
        let system_identity = nix_closure_volume_identity(
            &ResourceRef::parse("Volume/work-vm-system").expect("Volume"),
            &system_value,
            &system_spec,
        )
        .expect("system Volume identity");
        assert_eq!(system_identity.role, NixClosureVolumeRole::SystemVolume);

        let store_value = serde_json::json!({
            "metadata": {
                "name": "store-view-work-vm",
                "ownerRef": null
            }
        });
        let store_identity = nix_closure_volume_identity(
            &ResourceRef::parse("Volume/store-view-work-vm").expect("Volume"),
            &store_value,
            &fixtures::nix_closure_store_view_volume(),
        )
        .expect("store-view Volume identity");
        assert_eq!(store_identity.role, NixClosureVolumeRole::StoreView);
    }
}

/// Shared Runner reconciler for one U7 Provider owner.
pub(crate) struct SharedVolumeResourceReconciler {
    descriptor: ControllerDescriptor,
    kind: SharedVolumeResourceKind,
    effects: Arc<dyn SharedVolumeEffectExecutor>,
}

impl SharedVolumeResourceReconciler {
    fn new(
        descriptor: ControllerDescriptor,
        kind: SharedVolumeResourceKind,
        effects: Arc<dyn SharedVolumeEffectExecutor>,
    ) -> Arc<Self> {
        Arc::new(Self {
            descriptor,
            kind,
            effects,
        })
    }

    fn effect_context(&self, context: &ReconcileContext) -> SharedVolumeEffectContext {
        SharedVolumeEffectContext {
            identity: context.identity().clone(),
            target: context.target().clone(),
            operation_id: context.operation().operation_id().to_owned(),
        }
    }

    fn status_candidate(
        resource: &ResourceSnapshot,
        result: &SharedVolumeEffectResult,
    ) -> Result<Vec<u8>, SharedVolumeReconcileError> {
        let mut value = serde_json::from_slice::<Value>(resource.canonical_json())
            .map_err(|_| SharedVolumeReconcileError::InvalidResource)?;
        let status = value
            .get_mut("status")
            .and_then(Value::as_object_mut)
            .ok_or(SharedVolumeReconcileError::InvalidResource)?;
        status.insert(
            "phase".to_owned(),
            Value::String(
                match result.phase {
                    SharedVolumeEffectPhase::Ready => "Ready".to_owned(),
                    SharedVolumeEffectPhase::Pending => "Pending".to_owned(),
                    SharedVolumeEffectPhase::Failed => "Failed".to_owned(),
                }
            ),
        );
        if let Some(projection) = &result.resource_projection {
            status.insert("resource".to_owned(), projection.clone());
        }
        serde_json::to_vec(status).map_err(|_| SharedVolumeReconcileError::InvalidResource)
    }

    fn finalizer_mutation(
        resource: &ResourceSnapshot,
        finalizer: &str,
        add: bool,
    ) -> Result<ResourceMutationBatch, SharedVolumeReconcileError> {
        let canonical = finalizer_candidate(resource.canonical_json(), finalizer, add)?;
        let mutation = d2b_core_controller::MutationIntent::new(
            resource.key().resource_ref().clone(),
            Some(resource.key().uid().clone()),
            Some(resource.revision()),
            d2b_core_controller::MutationIntentKind::UpdateFinalizers,
            Some(canonical),
        )
        .map_err(|_| SharedVolumeReconcileError::InvalidResource)?;
        ResourceMutationBatch::new(vec![mutation])
            .map_err(|_| SharedVolumeReconcileError::InvalidResource)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SharedVolumeReconcileError {
    InvalidResource,
    Effect(SharedVolumeEffectError),
}

impl core::fmt::Display for SharedVolumeReconcileError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidResource => formatter.write_str("shared-volume-resource-invalid"),
            Self::Effect(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for SharedVolumeReconcileError {}

impl ResourceReconciler for SharedVolumeResourceReconciler {
    type Error = SharedVolumeReconcileError;

    fn classify_error(&self, error: &Self::Error) -> HandlerFailure {
        match error {
            SharedVolumeReconcileError::InvalidResource => HandlerFailure::terminal(),
            SharedVolumeReconcileError::Effect(_) => HandlerFailure::retryable(),
        }
    }

    fn describe(
        &self,
    ) -> impl std::future::Future<Output = Result<ControllerDescriptor, Self::Error>> + Send {
        std::future::ready(Ok(self.descriptor.clone()))
    }

    fn validate_spec(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
    ) -> impl std::future::Future<Output = Result<ValidationResult, Self::Error>> + Send {
        let valid = context.identity().zone() == resource.key().zone()
            && resource.key().resource_ref().resource_type().as_str() == self.kind.resource_type()
            && serde_json::from_slice::<Value>(resource.canonical_json())
                .ok()
                .and_then(|value| {
                    value
                        .pointer("/spec/providerRef")
                        .and_then(Value::as_str)
                        .map(|provider| provider == self.kind.provider_ref())
                })
                == Some(true);
        std::future::ready(Ok(if valid {
            ValidationResult::Valid
        } else {
            ValidationResult::Invalid {
                reason: ReconcileReason::InvalidSpec,
            }
        }))
    }

    async fn plan(
        &self,
        _context: &ReconcileContext,
        _resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> Result<ReconcilePlan, Self::Error> {
        ReconcilePlan::new(vec![self.kind.effect_id().to_owned()], false)
            .map_err(|_| SharedVolumeReconcileError::InvalidResource)
    }

    fn reconcile(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
        _plan: &ReconcilePlan,
    ) -> impl std::future::Future<Output = Result<ReconcileResult, Self::Error>> + Send {
        let result = (|| {
            context.authorize_effect().map_err(|_| {
                SharedVolumeReconcileError::Effect(SharedVolumeEffectError::Unavailable)
            })?;
            let finalizer = self
                .descriptor
                .finalizers()
                .first()
                .ok_or(SharedVolumeReconcileError::InvalidResource)?;
            if !resource.deleting() && !resource_has_finalizer(resource, finalizer)? {
                return Ok(ReconcileResult::new(
                    resource.revision(),
                    resource.generation(),
                    Some(Self::finalizer_mutation(resource, finalizer, true)?),
                    None,
                    ReconcileDisposition::Pending,
                    None,
                    None,
                    StatusPersistence::NotRequested,
                )
                .map_err(|_| SharedVolumeReconcileError::InvalidResource)?);
            }
            Ok(ReconcileResult::converged(
                resource.revision(),
                resource.generation(),
            ))
        })();
        std::future::ready(result)
    }

    async fn execute_effect(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
        _plan: &ReconcilePlan,
    ) -> Result<ReconcileResult, Self::Error> {
        let _permit = context.authorize_effect().map_err(|_| {
            SharedVolumeReconcileError::Effect(SharedVolumeEffectError::Unavailable)
        })?;
        let result = self
            .effects
            .reconcile_with_projection(self.kind, &self.effect_context(context), resource)
            .await
            .map_err(|error| {
                tracing::warn!(
                    resource = %resource.key().resource_ref().to_canonical_string(),
                    provider = self.kind.provider_ref(),
                    error = %error,
                    "U7 volume effect failed",
                );
                SharedVolumeReconcileError::Effect(error)
            })?;
        ReconcileResult::new(
            resource.revision(),
            resource.generation(),
            None,
            Some(Self::status_candidate(resource, &result)?),
            ReconcileDisposition::Pending,
            None,
            None,
            StatusPersistence::Pending,
        )
        .map_err(|_| SharedVolumeReconcileError::InvalidResource)
    }

    async fn observe(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
    ) -> Result<ObservationResult, Self::Error> {
        let _permit = context.authorize_effect().map_err(|_| {
            SharedVolumeReconcileError::Effect(SharedVolumeEffectError::Unavailable)
        })?;
        let result = self
            .effects
            .reconcile_with_projection(self.kind, &self.effect_context(context), resource)
            .await
            .map_err(SharedVolumeReconcileError::Effect)?;
        let result = ReconcileResult::new(
            resource.revision(),
            resource.generation(),
            None,
            Some(Self::status_candidate(resource, &result)?),
            ReconcileDisposition::Pending,
            None,
            None,
            StatusPersistence::Pending,
        )
        .map_err(|_| SharedVolumeReconcileError::InvalidResource)?;
        Ok(ObservationResult::new(result))
    }

    fn finalize(
        &self,
        _context: &ReconcileContext,
        deleting_resource: &ResourceSnapshot,
    ) -> impl std::future::Future<Output = Result<FinalizeResult, Self::Error>> + Send {
        std::future::ready(Ok(FinalizeResult::new(ReconcileResult::converged(
            deleting_resource.revision(),
            deleting_resource.generation(),
        ))))
    }

    fn prepare_finalize(
        &self,
        context: &ReconcileContext,
        deleting_resource: &ResourceSnapshot,
    ) -> impl std::future::Future<Output = Result<ReconcileResult, Self::Error>> + Send {
        let finalizer = self
            .descriptor
            .finalizers()
            .first()
            .cloned()
            .ok_or(SharedVolumeReconcileError::InvalidResource);
        let future = async move {
            context.authorize_effect().map_err(|_| {
                SharedVolumeReconcileError::Effect(SharedVolumeEffectError::Unavailable)
            })?;
            let finalizer = finalizer?;
            if !resource_has_finalizer(deleting_resource, &finalizer)? {
                return Ok(ReconcileResult::converged(
                    deleting_resource.revision(),
                    deleting_resource.generation(),
                ));
            }
            self.effects
                .finalize(self.kind, &self.effect_context(context), deleting_resource)
                .await
                .map_err(SharedVolumeReconcileError::Effect)?;
            ReconcileResult::new(
                deleting_resource.revision(),
                deleting_resource.generation(),
                Some(Self::finalizer_mutation(
                    deleting_resource,
                    &finalizer,
                    false,
                )?),
                None,
                ReconcileDisposition::Pending,
                None,
                None,
                StatusPersistence::NotRequested,
            )
            .map_err(|_| SharedVolumeReconcileError::InvalidResource)
        };
        future
    }

    async fn execute_finalize(
        &self,
        _context: &ReconcileContext,
        deleting_resource: &ResourceSnapshot,
    ) -> Result<ReconcileResult, Self::Error> {
        Ok(ReconcileResult::converged(
            deleting_resource.revision(),
            deleting_resource.generation(),
        ))
    }

    fn health(
        &self,
    ) -> impl std::future::Future<
        Output = Result<d2b_core_controller::ControllerHealth, Self::Error>,
    > + Send {
        std::future::ready(Ok(d2b_core_controller::ControllerHealth::Healthy))
    }

    fn drain(
        &self,
        _deadline_tick: u64,
    ) -> impl std::future::Future<Output = Result<DrainResult, Self::Error>> + Send {
        std::future::ready(Ok(DrainResult::Drained))
    }

    fn assess_update(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> impl std::future::Future<Output = Result<UpdateAssessment, Self::Error>> + Send {
        let current = serde_json::from_slice::<Value>(resource.canonical_json())
            .ok()
            .and_then(|value| {
                value
                    .pointer("/status/observedGeneration")
                    .and_then(Value::as_u64)
            })
            == Some(resource.generation().get());
        std::future::ready(
            UpdateAssessment::new(
                if current {
                    UpdateAssessmentState::Current
                } else {
                    UpdateAssessmentState::UpgradeRequired
                },
                Vec::new(),
                true,
            )
            .map_err(|_| SharedVolumeReconcileError::InvalidResource),
        )
    }

    fn plan_upgrade(
        &self,
        _context: &ReconcileContext,
        resource: &ResourceSnapshot,
        _dependencies: &[DependencySnapshot],
    ) -> impl std::future::Future<Output = Result<UpgradePlan, Self::Error>> + Send {
        std::future::ready(
            UpgradePlan::new(
                DisruptionClass::Restart,
                true,
                vec![UpgradeStage::Restart(resource.key().resource_ref().clone())],
            )
            .map_err(|_| SharedVolumeReconcileError::InvalidResource),
        )
    }

    async fn execute_upgrade(
        &self,
        context: &ReconcileContext,
        resource: &ResourceSnapshot,
        dependencies: &[DependencySnapshot],
        _plan: &UpgradePlan,
    ) -> Result<ReconcileResult, Self::Error> {
        self.execute_effect(
            context,
            resource,
            dependencies,
            &ReconcilePlan::new(vec![self.kind.effect_id().to_owned()], false)
                .map_err(|_| SharedVolumeReconcileError::InvalidResource)?,
        )
        .await
    }
}

type SharedCoreSource = CoreControllerSource<RedbRegisteredControllerApi>;

/// Start the U7 Runner wave for one Zone.
pub(crate) async fn start(
    runtime: &ZoneResourceRuntime,
    state: Arc<crate::ServerState>,
) -> Result<bool, super::ResourceRuntimeError> {
    if !runtime.readiness.resource_api_ready {
        return Ok(false);
    }
    let subject_context = runtime
        .core_controller_subject
        .lock()
        .map_err(|_| super::ResourceRuntimeError::AuthenticationUnavailable)?
        .clone()
        .ok_or(super::ResourceRuntimeError::AuthenticationUnavailable)?;
    let authorization_state = runtime
        .authorization_state
        .lock()
        .map_err(|_| super::ResourceRuntimeError::AuthenticationUnavailable)?
        .clone()
        .ok_or(super::ResourceRuntimeError::AuthenticationUnavailable)?;
    let controller_generation = runtime
        .store_metadata
        .policy_snapshot
        .controller_generation
        .ok_or(super::ResourceRuntimeError::HandlerNotReady)?;
    let session_generation = subject_context.reconnect_generation();
    let (active_registrations, provider_generations) = provider_generations(runtime).await?;
    if active_registrations.is_empty() {
        return Ok(false);
    }
    let descriptors = compose_shared_volume_runner_descriptors(
        active_registrations,
        runtime.zone.clone(),
        controller_generation,
        &provider_generations,
        session_generation,
    )?;
    let effects: Arc<dyn SharedVolumeEffectExecutor> = Arc::new(DaemonVolumeProviderEffects::new(
        state,
        runtime.zone.clone(),
    ));
    let mut tasks = Vec::new();
    for (registration, descriptor) in descriptors {
        let kind = SharedVolumeResourceKind::from_registration(registration)?;
        let provider_ref = ResourceRef::parse(registration.provider_ref)
            .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?;
        let controller_ref = ResourceRef::parse(registration.controller_ref)
            .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?;
        let (assignments, authority) = runtime
            .u12_controller_assignments(
                &descriptor,
                controller_ref.clone(),
                *provider_generations
                    .get(&provider_ref)
                    .ok_or(super::ResourceRuntimeError::HandlerNotReady)?,
                controller_generation,
                session_generation,
            )
            .await?;
        let subject = runtime
            .authorizer
            .issue_authenticated_subject(subject_context.clone(), authorization_state.clone())
            .map_err(|_| super::ResourceRuntimeError::AuthorizationUnavailable)?;
        let api = runtime
            .api
            .registered_controller_api(subject, authorization_state.clone(), assignments)
            .map_err(|_| super::ResourceRuntimeError::ResourceApiBindFailed)?;
        let allowed_types = descriptor
            .resource_types()
            .cloned()
            .collect::<BTreeSet<_>>();
        let resolver_store = Arc::clone(&runtime.store);
        let resolver_zone = runtime.zone.clone();
        let resolver_authority = Arc::clone(&authority);
        let resolver: AssignmentFenceResolver = Arc::new(move |target, uid, revision| {
            let store = Arc::clone(&resolver_store);
            let zone = resolver_zone.clone();
            let authority = Arc::clone(&resolver_authority);
            let allowed_types = allowed_types.clone();
            Box::pin(async move {
                if !allowed_types.contains(target.resource_type()) {
                    return Err(SourceError::Integrity);
                }
                if let Some(stored) = store.assignment_fence(zone, target.clone()).await.map_err(
                    |error| match error.kind() {
                        StoreErrorKind::Backpressure | StoreErrorKind::StoreBackpressure => {
                            SourceError::Backpressure
                        }
                        StoreErrorKind::Timeout => SourceError::Timeout,
                        _ => SourceError::Unavailable,
                    },
                )? {
                    if stored.resource_uid != uid
                        || stored.epoch > authority.epoch
                        || (stored.epoch == authority.epoch
                            && (stored.provider_generation != authority.provider_generation
                                || stored.controller_generation != authority.controller_generation
                                || stored.controller_role != authority.controller_role
                                || stored.target != authority.target
                                || stored.session_generation != authority.session_generation))
                    {
                        return Err(SourceError::Integrity);
                    }
                    if stored.epoch == authority.epoch && stored.resource_revision != revision {
                        return Err(SourceError::Conflict(stored.resource_revision));
                    }
                }
                Ok(ResourceAssignmentFence {
                    resource_uid: uid,
                    resource_revision: revision,
                    provider_generation: authority.provider_generation,
                    controller_generation: authority.controller_generation,
                    controller_role: authority.controller_role.clone(),
                    target: authority.target.clone(),
                    session_generation: authority.session_generation,
                    epoch: authority.epoch,
                    scope: ResourceAssignmentScope::Primary,
                })
            })
        });
        let api = api.with_assignment_fence_resolver(resolver);
        let source = SharedCoreSource::new(descriptor.clone(), Arc::new(api));
        let reconciler =
            SharedVolumeResourceReconciler::new(descriptor, kind, Arc::clone(&effects));
        let runner = Runner::new(
            reconciler,
            source,
            RunnerConfig {
                policy_revision: authorization_state.snapshot.policy_revision,
                api_revision: authorization_state.snapshot.api_catalog_revision,
                configuration_revision: authorization_state.snapshot.active_configuration_revision,
                deadline_tick: 5_000,
                max_attempts: 3,
            },
        );
        tasks.push(tokio::spawn(async move {
            if let Err(error) = runner.run().await {
                tracing::warn!(
                    error = %error,
                    failed_resource = ?error.failed_key()
                        .map(|key| key.resource_ref().to_canonical_string()),
                    failed_operation = ?error.failed_operation(),
                    "U7 shared Volume Runner stopped",
                );
            }
        }));
    }
    let mut slot = runtime
        .u7_runner_tasks
        .lock()
        .map_err(|_| super::ResourceRuntimeError::WatchUnavailable)?;
    slot.extend(tasks);
    Ok(true)
}

fn resource_has_finalizer(
    resource: &ResourceSnapshot,
    finalizer: &str,
) -> Result<bool, SharedVolumeReconcileError> {
    let value = serde_json::from_slice::<Value>(resource.canonical_json())
        .map_err(|_| SharedVolumeReconcileError::InvalidResource)?;
    Ok(value
        .pointer("/metadata/finalizers")
        .and_then(Value::as_array)
        .is_some_and(|finalizers| {
            finalizers
                .iter()
                .any(|value| value.as_str() == Some(finalizer))
        }))
}

fn finalizer_candidate(
    canonical_json: &[u8],
    finalizer: &str,
    add: bool,
) -> Result<Vec<u8>, SharedVolumeReconcileError> {
    let value = serde_json::from_slice::<Value>(canonical_json)
        .map_err(|_| SharedVolumeReconcileError::InvalidResource)?;
    let finalizers = value
        .pointer("/metadata/finalizers")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut finalizers = finalizers
        .into_iter()
        .filter(|value| !(!add && value.as_str() == Some(finalizer)))
        .collect::<Vec<_>>();
    if add
        && !finalizers
            .iter()
            .any(|value| value.as_str() == Some(finalizer))
    {
        finalizers.push(Value::String(finalizer.to_owned()));
    }
    let bytes = serde_json::to_vec(&json!({
        "metadata": {"finalizers": finalizers}
    }))
    .map_err(|_| SharedVolumeReconcileError::InvalidResource)?;
    CanonicalJsonValue::parse(&bytes)
        .map(|value| value.to_canonical_bytes())
        .map_err(|_| SharedVolumeReconcileError::InvalidResource)
}

async fn provider_generations(
    runtime: &ZoneResourceRuntime,
) -> Result<
    (
        Vec<SharedVolumeRunnerRegistration>,
        BTreeMap<ResourceRef, ResourceGeneration>,
    ),
    super::ResourceRuntimeError,
> {
    let mut generations = BTreeMap::new();
    let mut active = Vec::new();
    for registration in U7_SHARED_PROVIDER_RUNNERS {
        let provider_ref = ResourceRef::parse(registration.provider_ref)
            .map_err(|_| super::ResourceRuntimeError::HandlerNotReady)?;
        let request = StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "u7-provider-generation".to_owned(),
                    idempotency_key: None,
                    correlation_id: "u7-provider-generation".to_owned(),
                    trace_id: None,
                    deadline_ms: 10_000,
                },
                zone: runtime.zone.clone(),
                target: provider_ref.clone(),
                expected_uid: None,
                projection: StoreProjection::MetadataOnly,
            };
        match retry_transient_store_read(
            &runtime.zone,
            "u7-provider-generation",
            || runtime.store.get(request.clone()),
        )
        .await
        {
            Ok(provider) if provider.zone == runtime.zone && provider.generation.get() > 0 => {
                generations.insert(provider_ref, provider.generation);
                active.push(registration);
            }
            Err(error) if error.kind() == StoreErrorKind::ResourceNotFound => {
                if runtime
                    .provider_resources_present(
                        registration.provider_ref,
                        &[registration.resource_type],
                    )
                    .await?
                {
                    return Err(super::ResourceRuntimeError::ProviderPathUnavailable);
                }
            }
            Err(_) => return Err(super::ResourceRuntimeError::StoreReadFailed),
            _ => return Err(super::ResourceRuntimeError::HandlerNotReady),
        }
    }
    Ok((active, generations))
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_broker::broker_wire::OpenZoneStoreResponse;
    use d2bd_runtime::resource_store_runtime::OpenedZoneStore;
    #[test]
    fn production_child_readiness_port_fails_closed_before_ro_store_launch() {
        let binding = d2b_provider_volume_virtiofs::testing::fixtures::binding("read-only");
        let volume = d2b_provider_volume_local::testing::fixtures::store_view_volume();
        let port = ChildReadinessPort {
            process_ref: ResourceRef::parse("Process/virtiofs-worker").expect("process ref"),
            endpoint_ref: ResourceRef::parse("Endpoint/virtiofs-worker").expect("endpoint ref"),
            socket_ready: true,
            guest_mount_ready: true,
            store_view_marker: false,
            zone: d2b_provider_volume_virtiofs::testing::fixtures::zone(),
            fence: binding.fence(),
        };
        let report = d2b_provider_volume_virtiofs::testing::block_on(
            VirtiofsBindingController::new(port).reconcile(
                &binding,
                &volume,
                4,
                d2b_provider_volume_virtiofs::testing::fixtures::principal(),
            ),
        )
        .expect("binding report");
        assert_eq!(
            report.phase,
            d2b_provider_volume_virtiofs::BindingPhase::Pending
        );
        assert!(report.worker_process_ref.is_none());
    }

    #[test]
    fn binding_child_payload_starts_with_typed_status_projection() {
        let zone = ZoneId::parse("work").expect("Zone");
        let target = ResourceRef::parse("VolumeBinding/vol-binding-test").expect("binding ref");
        let owner = ResourceRef::parse("Volume/store-view-work-vm").expect("Volume ref");
        let canonical = DaemonVolumeProviderEffects::child_resource(
            &zone,
            &target,
            &owner,
            json!({
                "providerRef": "Provider/volume-virtiofs",
                "volumeRef": owner.to_canonical_string(),
                "executionRef": "Guest/work-vm",
                "view": "ro-store",
                "access": "read-only",
                "mountPath": "/nix/.ro-store"
            }),
        )
        .expect("binding child payload");
        let value: Value = serde_json::from_slice(&canonical).expect("canonical JSON");
        // The neutral envelope opens fail-closed: ready is false and the
        // zero-UID fence never matches a live binding (KTD3).
        assert_eq!(value["status"]["resource"]["ready"], false);
        assert_eq!(
            value["status"]["resource"]["fence"]["uid"],
            "00000000-0000-4000-8000-000000000000"
        );
        assert_eq!(value["status"]["resource"]["fence"]["generation"], 1);
        assert_eq!(value["status"]["resource"]["fence"]["revision"], 0);
        // The payload parses as the typed projection.
        let projection: VolumeBindingStatusResource =
            serde_json::from_value(value["status"]["resource"].clone()).expect("projection");
        assert!(!projection.ready);
    }

    fn test_stored_resource(zone: &ZoneId, resource_ref: &ResourceRef) -> StoredResource {
        let value = json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": resource_ref.resource_type().as_str(),
            "metadata": {
                "name": resource_ref.name().as_str(),
                "zone": zone.as_str(),
                "ownerRef": null,
                "finalizers": [],
                "deletionRequestedAt": null,
                "createdAt": "1970-01-01T00:00:00.000Z",
                "updatedAt": "1970-01-01T00:00:00.000Z",
                "generation": 1,
                "revision": 1,
                "managedBy": "controller",
                "uid": "123e4567-e89b-42d3-a456-426614174000"
            },
            "spec": {},
            "status": {
                "observedGeneration": 0,
                "phase": "Pending",
                "conditions": [],
                "lastReconciledAt": null,
                "startedAt": null,
                "completedAt": null,
                "outcome": null,
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
                },
                "resource": {}
            }
        });
        let canonical = CanonicalJsonValue::parse(
            &serde_json::to_vec(&value).expect("resource serialization"),
        )
        .expect("canonical resource")
        .to_canonical_bytes();
        StoredResource {
            resource_ref: resource_ref.clone(),
            zone: zone.clone(),
            uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("uid"),
            owner_uid: None,
            owner_generation: None,
            generation: ResourceGeneration::new(1).expect("generation"),
            revision: d2b_contracts_resource::v3::ZoneRevision::new(1),
            canonical_json: canonical,
            payload_digest: "sha256:test".to_owned(),
        }
    }

    // Convergence is observable on the second pass: the first reconcile mints
    // the binding child, the second finds no pending mutations and reports converged.
    #[tokio::test]
    async fn volume_with_attachments_converges_volume_owned_children() {
        let (directory, runtime, _broker) =
            crate::resource_runtime::tests::open_production_guest_runtime_for_test().await;
        let zone = runtime.zone.clone();
        let volume_ref = ResourceRef::parse("Volume/store-view-work-vm").expect("volume ref");
        let spec = d2b_provider_volume_local::testing::fixtures::store_view_volume();
        let mut volume_spec = serde_json::to_value(&spec).expect("volume spec");
        volume_spec["providerRef"] = json!("Provider/volume-local");
        crate::resource_runtime::tests::materialize_test_bundle(
            &runtime,
            vec![crate::resource_runtime::tests::bundle_resource(
                "Volume",
                volume_ref.name().as_str(),
                &zone,
                &serde_json::to_string(&volume_spec).expect("volume spec string"),
            )],
        )
        .await;
        let desired = DaemonVolumeProviderEffects::volume_children(&zone, &volume_ref, &spec)
            .expect("binding children");
        assert_eq!(desired.len(), 1, "one virtiofs attachment mints one binding");

        let owner = test_stored_resource(&zone, &volume_ref);
        let client = runtime.status_client().expect("status client");
        let owners = || {
            vec![crate::binding_child_resource_runtime::OwnedChildOwner {
                resource: owner.clone(),
                desired: Some(desired.clone()),
                fenced: false,
            }]
        };
        crate::binding_child_resource_runtime::reconcile_owned_children(
            &runtime.store,
            client.as_ref(),
            &zone,
            &owners(),
        )
        .await
        .expect("binding child reconciliation");
        // The batch operation id derives from the owner revision, so the second
        // pass advances it the way a production re-read would; reusing the same
        // revision trips the service idempotency guard instead of converging.
        let mut reconverge_owner = owner.clone();
        reconverge_owner.revision = d2b_contracts_resource::v3::ZoneRevision::new(2);
        let converged = crate::binding_child_resource_runtime::reconcile_owned_children(
            &runtime.store,
            client.as_ref(),
            &zone,
            &[crate::binding_child_resource_runtime::OwnedChildOwner {
                resource: reconverge_owner,
                desired: Some(desired.clone()),
                fenced: false,
            }],
        )
        .await
        .expect("binding child reconvergence");
        assert!(converged.contains(&volume_ref), "Volume-owned children converge");

        let children = crate::binding_child_resource_runtime::list_binding_children(
            &runtime.store,
            &zone,
        )
        .await
        .expect("children relist");
        let binding = children
            .iter()
            .find(|child| {
                child.resource_ref.resource_type().as_str() == VOLUME_BINDING_RESOURCE_TYPE
            })
            .expect("stored VolumeBinding child");
        let value: Value =
            serde_json::from_slice(&binding.canonical_json).expect("binding envelope");
        assert_eq!(
            value["metadata"]["ownerRef"],
            volume_ref.to_canonical_string(),
            "binding is owned by the Volume"
        );
        let projection: VolumeBindingStatusResource =
            serde_json::from_value(value["status"]["resource"].clone()).expect("projection");
        assert!(!projection.ready, "fresh binding opens fail-closed");
    }
}
