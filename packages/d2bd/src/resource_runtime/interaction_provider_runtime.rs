//! Shared-Runner composition for the U9 interaction and shell Providers.
//!
//! display-wayland, audio-pipewire, and shell-terminal register their
//! ResourceTypes here, together with the Provider-generation read and the
//! runner lifecycle that attaches them to the production shared Runner.
//! Clipboard and notification delivery stay ComponentSession services and are
//! intentionally not registered as ResourceTypes.
//!
//! The typed effect executor and the shared Runner reconciler live in
//! [`super::shared_provider_runtime`]: the interaction effect arms are part of
//! that single closed trait impl and are not separable from it.

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{
        Arc,
        atomic::Ordering,
    },
};

use d2b_contracts_resource::v3::{ResourceGeneration, ResourceRef, ResourceTypeName};
use d2b_core_controller::{CoreControllerSource, Runner, RunnerConfig, RunnerError, SourceError};
use d2b_resource_api::registered::AssignmentFenceResolver;
use d2b_resource_store::{
    ResourceAssignmentFence, ResourceAssignmentScope, StoreErrorKind, StoreGetRequest,
    StoreOperationContext, StoreProjection,
};
use d2bd_runtime::resource_runtime_support::retry_transient_store_read;

use super::{
    ASSIGNMENT_EPOCH, ControllerRunnerFailure, ResourceRuntimeError, SharedProviderRunnerRegistration,
    ZoneResourceRuntime, assignment_fence_conflict, compose_shared_provider_runner_descriptors,
    push_runner_failure,
    shared_provider_runtime::{
        DaemonSharedProviderEffects, SharedProviderEffectExecutor, SharedProviderResourceKind,
        SharedProviderResourceReconciler,
    },
};

/// The U9 interaction and shell ResourceTypes attached to the production
/// shared Runner. Clipboard and notification delivery remain typed
/// ComponentSession services and therefore have no ResourceType registration.
pub const U9_SHARED_PROVIDER_RUNNERS: [SharedProviderRunnerRegistration; 6] = [
    SharedProviderRunnerRegistration {
        controller_ref: "Process/display-wayland-controller",
        provider_ref: "Provider/display-wayland",
        resource_type: "display-wayland.d2bus.org.WaylandPolicy",
        finalizer: "",
        repair_interval_ticks:
            d2b_provider_display_wayland::DISPLAY_REPAIR_INTERVAL_SECS * 1_000,
        watched_configuration_is_dependency:
            d2b_provider_display_wayland::display_runner_contract()
                .watched_configuration_is_dependency(),
    },
    SharedProviderRunnerRegistration {
        controller_ref: "Process/display-wayland-controller",
        provider_ref: "Provider/display-wayland",
        resource_type: "display-wayland.d2bus.org.WaylandSession",
        finalizer: d2b_provider_display_wayland::FINALIZER,
        repair_interval_ticks:
            d2b_provider_display_wayland::DISPLAY_REPAIR_INTERVAL_SECS * 1_000,
        watched_configuration_is_dependency:
            d2b_provider_display_wayland::display_runner_contract()
                .watched_configuration_is_dependency(),
    },
    SharedProviderRunnerRegistration {
        controller_ref: "Process/audio-pipewire-controller",
        provider_ref: "Provider/audio-pipewire",
        resource_type: "audio.d2bus.org.AudioService",
        finalizer: d2b_provider_audio_pipewire::AUDIO_SERVICE_FINALIZER,
        repair_interval_ticks:
            d2b_provider_audio_pipewire::AUDIO_REPAIR_INTERVAL_SECS * 1_000,
        watched_configuration_is_dependency:
            d2b_provider_audio_pipewire::audio_runner_contract()
                .watched_configuration_is_dependency(),
    },
    SharedProviderRunnerRegistration {
        controller_ref: "Process/audio-pipewire-controller",
        provider_ref: "Provider/audio-pipewire",
        resource_type: "audio.d2bus.org.AudioBinding",
        finalizer: d2b_provider_audio_pipewire::AUDIO_BINDING_FINALIZER,
        repair_interval_ticks:
            d2b_provider_audio_pipewire::AUDIO_REPAIR_INTERVAL_SECS * 1_000,
        watched_configuration_is_dependency:
            d2b_provider_audio_pipewire::audio_runner_contract()
                .watched_configuration_is_dependency(),
    },
    SharedProviderRunnerRegistration {
        controller_ref: "Process/shell-terminal-controller",
        provider_ref: "Provider/shell-terminal",
        resource_type: "shell-terminal.d2bus.org.ShellPool",
        finalizer: d2b_provider_shell_terminal::SHELL_POOL_FINALIZER,
        repair_interval_ticks: d2b_provider_shell_terminal::SHELL_REPAIR_INTERVAL_SECS * 1_000,
        watched_configuration_is_dependency:
            d2b_provider_shell_terminal::shell_runner_contract()
                .watched_configuration_is_dependency(),
    },
    SharedProviderRunnerRegistration {
        controller_ref: "Process/shell-terminal-controller",
        provider_ref: "Provider/shell-terminal",
        resource_type: "shell-terminal.d2bus.org.ShellSession",
        finalizer: d2b_provider_shell_terminal::SHELL_SESSION_FINALIZER,
        repair_interval_ticks: d2b_provider_shell_terminal::SHELL_REPAIR_INTERVAL_SECS * 1_000,
        watched_configuration_is_dependency:
            d2b_provider_shell_terminal::shell_runner_contract()
                .watched_configuration_is_dependency(),
    },
];

pub(super) const U9_PROVIDER_REFS: [&str; 5] = [
    "Provider/display-wayland",
    "Provider/audio-pipewire",
    "Provider/clipboard-wayland",
    "Provider/notification-desktop",
    "Provider/shell-terminal",
];

pub(super) async fn abort_u9_runner_tasks(tasks: &mut Vec<tokio::task::JoinHandle<()>>) {
    for task in tasks.drain(..) {
        task.abort();
        let _ = task.await;
    }
}

pub(super) fn u9_runner_tasks_are_live(tasks: &[tokio::task::JoinHandle<()>]) -> bool {
    !tasks.is_empty() && tasks.iter().all(|task| !task.is_finished())
}

async fn u9_provider_generations(
    runtime: &ZoneResourceRuntime,
) -> Result<
    (
        Vec<SharedProviderRunnerRegistration>,
        BTreeMap<ResourceRef, ResourceGeneration>,
    ),
    ResourceRuntimeError,
> {
    let mut generations = BTreeMap::new();
    let mut active = Vec::new();
    let mut seen = BTreeSet::new();
    for registration in U9_SHARED_PROVIDER_RUNNERS {
        if !seen.insert(registration.provider_ref) {
            continue;
        }
        let provider_ref = ResourceRef::parse(registration.provider_ref)
            .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
        let request = StoreGetRequest {
                operation: StoreOperationContext {
                    operation_id: "u9-provider-generation".to_owned(),
                    idempotency_key: None,
                    correlation_id: "u9-provider-generation".to_owned(),
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
            "u9-provider-generation",
            || runtime.store.get(request.clone()),
        )
        .await
        {
            Ok(provider) if provider.zone == runtime.zone && provider.generation.get() > 0 => {
                generations.insert(provider_ref, provider.generation);
                active.extend(
                    U9_SHARED_PROVIDER_RUNNERS
                        .iter()
                        .copied()
                        .filter(|candidate| candidate.provider_ref == registration.provider_ref),
                );
            }
            Err(error) if error.kind() == StoreErrorKind::ResourceNotFound => {
                let owned_resource = runtime
                    .provider_resources_present(
                        registration.provider_ref,
                        &[registration.resource_type],
                    )
                    .await?
                    || (registration.resource_type.starts_with("display-wayland.")
                        && !runtime
                            .committed_resources_of_type(registration.resource_type)
                            .await?
                            .is_empty());
                if owned_resource {
                    return Err(ResourceRuntimeError::ProviderPathUnavailable);
                }
            }
            Err(error) => {
                tracing::debug!(
                    error = %error,
                    provider = %registration.provider_ref,
                    "U9 provider generation read failed",
                );
                return Err(ResourceRuntimeError::StoreReadFailed);
            }
            _ => return Err(ResourceRuntimeError::HandlerNotReady),
        }
    }
    active.sort_by_key(|registration| {
        (
            registration.provider_ref,
            registration.resource_type,
            registration.controller_ref,
        )
    });
    active.dedup_by_key(|registration| {
        (
            registration.provider_ref,
            registration.resource_type,
            registration.controller_ref,
        )
    });
    Ok((active, generations))
}

impl ZoneResourceRuntime {
    pub(super) async fn stop_u9_controller_runners_locked(&self) -> Result<(), ResourceRuntimeError> {
        let tasks = {
            let mut tasks = self
                .u9_runner_tasks
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            std::mem::take(&mut *tasks)
        };
        for task in tasks {
            task.abort();
            let _ = task.await;
        }
        if let Ok(mut failures) = self.u9_runner_failures.lock() {
            failures.clear();
        }
        self.u9_required.store(false, Ordering::Release);
        Ok(())
    }

    /// Attach interaction and shell resource owners to the production shared
    /// Runner. Clipboard and notification streams remain ComponentSession
    /// services and are intentionally not registered as ResourceTypes.
    pub(crate) async fn start_u9_controller_runners(
        &self,
        state: Arc<crate::ServerState>,
    ) -> Result<(), ResourceRuntimeError> {
        let _runner_guard = self.u9_runner_lock.lock().await;
        let result = self
            .start_u9_controller_runners_locked(Arc::clone(&state))
            .await;
        if result.is_ok() {
            match self.u9_state.lock() {
                Ok(mut current) => *current = Some(state),
                Err(_) => {
                    tracing::warn!(
                        controller = "interaction",
                        "U9 controller runner state lock poisoned; stopping runners",
                    );
                    self.stop_u9_controller_runners_locked().await?;
                    return Err(ResourceRuntimeError::AuthenticationUnavailable);
                }
            }
        }
        result
    }

    pub(super) async fn start_u9_controller_runners_locked(
        &self,
        state: Arc<crate::ServerState>,
    ) -> Result<(), ResourceRuntimeError> {
        if !self.readiness.resource_api_ready {
            return Ok(());
        }
        {
            let tasks = self
                .u9_runner_tasks
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            if u9_runner_tasks_are_live(&tasks) {
                return Ok(());
            }
        }
        let stale = {
            let mut tasks = self
                .u9_runner_tasks
                .lock()
                .map_err(|_| ResourceRuntimeError::WatchUnavailable)?;
            std::mem::take(&mut *tasks)
        };
        let mut stale = stale;
        abort_u9_runner_tasks(&mut stale).await;
        let subject_context = self
            .core_controller_subject
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        let authorization_state = self
            .authorization_state
            .lock()
            .map_err(|_| ResourceRuntimeError::AuthenticationUnavailable)?
            .clone()
            .ok_or(ResourceRuntimeError::AuthenticationUnavailable)?;
        let controller_generation = self
            .store_metadata
            .policy_snapshot
            .controller_generation
            .ok_or(ResourceRuntimeError::HandlerNotReady)?;
        let session_generation = subject_context.reconnect_generation();
        let (active_registrations, provider_generations) =
            u9_provider_generations(self).await?;
        if active_registrations.is_empty() {
            if let Ok(mut failures) = self.u9_runner_failures.lock() {
                failures.clear();
            }
            self.u9_required.store(false, Ordering::Release);
            return Ok(());
        }
        self.u9_required.store(true, Ordering::Release);
        if let Ok(mut failures) = self.u9_runner_failures.lock() {
            failures.clear();
        }
        let descriptors = compose_shared_provider_runner_descriptors(
            active_registrations,
            self.zone.clone(),
            controller_generation,
            &provider_generations,
            session_generation,
        )?;
        let effects: Arc<dyn SharedProviderEffectExecutor> = Arc::new(
            DaemonSharedProviderEffects::new(Arc::clone(&state), self.zone.clone()),
        );
        let mut new_tasks = Vec::with_capacity(descriptors.len());
        let mut startup_receivers = Vec::with_capacity(descriptors.len());
        for (registration, descriptor) in descriptors {
            let task = async {
                let kind = SharedProviderResourceKind::from_registration(registration)?;
                let provider_ref = ResourceRef::parse(registration.provider_ref)
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
                let controller_ref = ResourceRef::parse(registration.controller_ref)
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
                let provider_generation = *provider_generations
                    .get(&provider_ref)
                    .ok_or(ResourceRuntimeError::HandlerNotReady)?;
                let (assignments, authority) = self
                    .u12_controller_assignments(
                        &descriptor,
                        controller_ref.clone(),
                        provider_generation,
                        controller_generation,
                        session_generation,
                    )
                    .await?;
                let subject = self
                    .authorizer
                    .issue_authenticated_subject(
                        subject_context.clone(),
                        authorization_state.clone(),
                    )
                    .map_err(|_| ResourceRuntimeError::AuthorizationUnavailable)?;
                let api = self
                    .api
                    .registered_controller_api(
                        subject,
                        authorization_state.clone(),
                        assignments,
                    )
                    .map_err(|_| ResourceRuntimeError::ResourceApiBindFailed)?;
                let allowed_types = descriptor
                    .resource_types()
                    .cloned()
                    .collect::<BTreeSet<_>>();
                let resolver_store = Arc::clone(&self.store);
                let resolver_zone = self.zone.clone();
                let resolver_authority = Arc::clone(&authority);
                let resolver: AssignmentFenceResolver =
                    Arc::new(move |target, uid, revision| {
                        let store = Arc::clone(&resolver_store);
                        let zone = resolver_zone.clone();
                        let authority = Arc::clone(&resolver_authority);
                        let allowed_types = allowed_types.clone();
                        Box::pin(async move {
                            if !allowed_types.contains(target.resource_type()) {
                                return Err(SourceError::Integrity);
                            }
                            if let Some(stored) = store
                                .assignment_fence(zone, target.clone())
                                .await
                                .map_err(|error| match error.kind() {
                                    StoreErrorKind::Backpressure
                                    | StoreErrorKind::StoreBackpressure => {
                                        SourceError::Backpressure
                                    }
                                    StoreErrorKind::Timeout => SourceError::Timeout,
                                    _ => SourceError::Unavailable,
                                })?
                            {
                                if assignment_fence_conflict(&stored, &uid, &authority) {
                                    return Err(SourceError::Integrity);
                                }
                                if stored.resource_revision != revision {
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
                                epoch: ASSIGNMENT_EPOCH,
                                scope: ResourceAssignmentScope::Primary,
                            })
                        })
                    });
                let api = api.with_assignment_fence_resolver(resolver);
                let source = CoreControllerSource::new(descriptor.clone(), Arc::new(api));
                let reconciler = SharedProviderResourceReconciler::new(
                    descriptor.clone(),
                    kind,
                    Arc::clone(&effects),
                );
                let runner = Runner::new(
                    reconciler,
                    source,
                    RunnerConfig {
                        policy_revision: authorization_state.snapshot.policy_revision,
                        api_revision: authorization_state.snapshot.api_catalog_revision,
                        configuration_revision: authorization_state
                            .snapshot
                            .active_configuration_revision,
                        deadline_tick: 30_000,
                        max_attempts: 10,
                    },
                );
                let resource_type = registration.resource_type;
                let controller = registration.controller_ref;
                let diagnostic_controller = controller_ref.clone();
                let diagnostic_resource_type = ResourceTypeName::parse(resource_type)
                    .map_err(|_| ResourceRuntimeError::HandlerNotReady)?;
                let (startup_tx, startup_rx) = tokio::sync::oneshot::channel();
                let startup_tx = Arc::new(tokio::sync::Mutex::new(Some(startup_tx)));
                let failure_slot = Arc::clone(&self.u9_runner_failures);
                let startup_failure_slot = Arc::clone(&failure_slot);
                let callback_controller = diagnostic_controller.clone();
                let callback_resource_type = diagnostic_resource_type.clone();
                let task = tokio::spawn(async move {
                    // Respawn on transient source failures with capped
                    // backoff: a dead interaction runner wedges clipboard,
                    // notification, and audio reconciliation forever.
                    let mut backoff_ms = 500u64;
                    let result = loop {
                        let outcome = runner
                            .run_with_startup({
                                let startup_failure_slot =
                                    Arc::clone(&startup_failure_slot);
                                let callback_controller =
                                    callback_controller.clone();
                                let callback_resource_type =
                                    callback_resource_type.clone();
                                let startup_tx = Arc::clone(&startup_tx);
                                move |startup| {
                                    if let Err(error) = startup {
                                        push_runner_failure(
                                            &startup_failure_slot,
                                            ControllerRunnerFailure::new(
                                                callback_controller,
                                                [callback_resource_type],
                                                error,
                                            ),
                                        );
                                    }
                                    if let Ok(mut slot) = startup_tx.try_lock() {
                                        if let Some(sender) = slot.take() {
                                            let _ = sender.send(startup);
                                        }
                                    }
                                }
                            })
                            .await;
                        let transient = matches!(
                            &outcome,
                            Err(failure) if matches!(
                                failure.error(),
                                RunnerError::Source(
                                    SourceError::Timeout
                                        | SourceError::Unavailable
                                        | SourceError::Backpressure
                                )
                            )
                        );
                        if !transient {
                            break outcome;
                        }
                        tracing::warn!(
                            backoff_ms,
                            "U9 interaction shared Runner retrying after transient failure",
                        );
                        tokio::time::sleep(std::time::Duration::from_millis(
                            backoff_ms,
                        ))
                        .await;
                        backoff_ms = (backoff_ms * 2).min(5_000);
                    };
                    match result {
                        Ok(report) => tracing::debug!(
                            controller,
                            resource_type,
                            dispatched = report.dispatched,
                            relists = report.relists,
                            "U9 interaction shared Runner stopped",
                        ),
                        Err(error) => {
                            let diagnostic = ControllerRunnerFailure::new(
                                diagnostic_controller,
                                [diagnostic_resource_type],
                                error.error(),
                            );
                            push_runner_failure(&failure_slot, diagnostic.clone());
                            let report = error.report();
                            tracing::warn!(
                                controller = %diagnostic.controller().to_canonical_string(),
                                resource_types = ?diagnostic
                                    .resource_types()
                                    .iter()
                                    .map(ResourceTypeName::as_str)
                                    .collect::<Vec<_>>(),
                                error_kind = ?diagnostic.error(),
                                error = %diagnostic.error(),
                                dispatched = report.dispatched,
                                relists = report.relists,
                                checkpointed = report.checkpointed,
                                failed_resource = ?error
                                    .failed_key()
                                    .map(|key| key.resource_ref().to_canonical_string()),
                                failed_operation = ?error.failed_operation(),
                                "U9 interaction shared Runner failed",
                            );
                        }
                    }
                });
                Ok::<_, ResourceRuntimeError>((task, startup_rx))
            }
            .await;
            match task {
                Ok((task, startup_rx)) => {
                    new_tasks.push(task);
                    startup_receivers.push(startup_rx);
                }
                Err(error) => {
                    abort_u9_runner_tasks(&mut new_tasks).await;
                    return Err(error);
                }
            }
        }
        for startup in startup_receivers {
            match startup.await {
                Ok(Ok(())) => {}
                Ok(Err(error)) => {
                    tracing::warn!(
                        error = ?error,
                        "U9 shared runner startup failed",
                    );
                    abort_u9_runner_tasks(&mut new_tasks).await;
                    return Err(ResourceRuntimeError::HandlerNotReady);
                }
                Err(_) => {
                    tracing::warn!("U9 shared runner startup channel closed");
                    abort_u9_runner_tasks(&mut new_tasks).await;
                    return Err(ResourceRuntimeError::HandlerNotReady);
                }
            }
        }
        let mut tasks = match self.u9_runner_tasks.lock() {
            Ok(tasks) => tasks,
            Err(_) => {
                tracing::warn!(
                    controller = "interaction",
                    "U9 runner task store lock poisoned; aborting new runner tasks",
                );
                abort_u9_runner_tasks(&mut new_tasks).await;
                return Err(ResourceRuntimeError::WatchUnavailable);
            }
        };
        tasks.extend(new_tasks);
        self.u9_required.store(true, Ordering::Release);
        Ok(())
    }
}
