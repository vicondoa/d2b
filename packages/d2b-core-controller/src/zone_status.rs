//! Production Zone status projection with the mandatory system-core pair.

use d2b_contracts_zone_session::v3::ZoneHandlerName;
use d2b_contracts_zone_session::v3::{
    ZoneHandlerPhase,
    ZoneHandlerStatus,
    ZoneStatusResource,
};
use d2b_contracts_resource::v3::{
    ResourcePhase,
    Timestamp,
};

fn emit_handler_status(
    host_phase: ZoneHandlerPhase,
    user_phase: ZoneHandlerPhase,
    last_reconciled_at: Option<Timestamp>,
) -> Vec<ZoneHandlerStatus> {
    vec![
        ZoneHandlerStatus::new(
            ZoneHandlerName::SystemCoreHost,
            host_phase,
            last_reconciled_at.clone(),
        ),
        ZoneHandlerStatus::new(
            ZoneHandlerName::SystemCoreUser,
            user_phase,
            last_reconciled_at,
        ),
    ]
}

/// Live revisions, counts, and reconcile metadata projected into Zone status.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ZoneRuntimeMetadata {
    pub api_catalog_revision: u64,
    pub policy_revision: u64,
    pub configuration_revision: u64,
    pub installed_provider_count: u32,
    pub ready_provider_count: u32,
    pub total_resource_count: u32,
    pub active_configuration_generation: u64,
    pub generation_cleanup_pending: bool,
    pub cleanup_pending_count: u32,
    pub last_reconciled_at: Option<Timestamp>,
}

/// Inputs consumed by the production Zone status emitter.
#[derive(Debug, Clone)]
pub struct ZoneStatusInput {
    core_phase: ResourcePhase,
    handlers: Vec<ZoneHandlerStatus>,
    host_phase: ZoneHandlerPhase,
    user_phase: ZoneHandlerPhase,
    runtime: ZoneRuntimeMetadata,
}

impl ZoneStatusInput {
    /// Construct input using one aggregate phase for both bootstrap handlers.
    pub fn new(core_phase: ResourcePhase, handlers: Vec<ZoneHandlerStatus>) -> Self {
        let phase = phase_for(core_phase);
        Self {
            core_phase,
            handlers,
            host_phase: phase,
            user_phase: phase,
            runtime: ZoneRuntimeMetadata::default(),
        }
    }

    /// Override the two system-core handler phases independently.
    #[must_use]
    pub fn with_system_core_phases(
        mut self,
        host_phase: ZoneHandlerPhase,
        user_phase: ZoneHandlerPhase,
    ) -> Self {
        self.host_phase = host_phase;
        self.user_phase = user_phase;
        self
    }

    /// Attach live store/provider metadata to the projected status.
    #[must_use]
    pub fn with_runtime_metadata(mut self, runtime: ZoneRuntimeMetadata) -> Self {
        self.runtime = runtime;
        self
    }
}

/// Errors while projecting a Zone status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneStatusProjectionError {
    /// The common Zone status contract rejected the projection.
    Contract,
}

impl core::fmt::Display for ZoneStatusProjectionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("zone-status-projection-invalid")
    }
}

impl std::error::Error for ZoneStatusProjectionError {}

/// Fixed production emitter owned by core-controller infrastructure.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemCoreStatusEmitter;

impl SystemCoreStatusEmitter {
    /// Construct the emitter.
    pub const fn new() -> Self {
        Self
    }

    /// Emit a validated Zone status containing exactly one Host and User
    /// system-core record.
    pub fn emit(
        &self,
        input: ZoneStatusInput,
    ) -> Result<ZoneStatusResource, ZoneStatusProjectionError> {
        let ZoneStatusInput {
            core_phase,
            handlers: input_handlers,
            host_phase: configured_host_phase,
            user_phase: configured_user_phase,
            runtime,
        } = input;
        let mut handlers = Vec::with_capacity(input_handlers.len() + 2);
        let mut host_phase = None;
        let mut user_phase = None;

        for handler in input_handlers {
            match handler.name() {
                d2b_contracts_zone_session::v3::ZoneHandlerName::SystemCoreHost => {
                    if host_phase.replace(handler.phase()).is_some() {
                        return Err(ZoneStatusProjectionError::Contract);
                    }
                }
                d2b_contracts_zone_session::v3::ZoneHandlerName::SystemCoreUser => {
                    if user_phase.replace(handler.phase()).is_some() {
                        return Err(ZoneStatusProjectionError::Contract);
                    }
                }
                _ => handlers.push(handler),
            }
        }

        let (host_phase, user_phase) = match (host_phase, user_phase) {
            (None, None) => (configured_host_phase, configured_user_phase),
            (host_phase, user_phase) => (
                host_phase.unwrap_or(ZoneHandlerPhase::Pending),
                user_phase.unwrap_or(ZoneHandlerPhase::Pending),
            ),
        };
        handlers.extend(emit_handler_status(
            host_phase,
            user_phase,
            runtime.last_reconciled_at.clone(),
        ));
        ZoneStatusResource::new(
            runtime.api_catalog_revision,
            runtime.policy_revision,
            runtime.configuration_revision,
            core_phase,
            handlers,
            runtime.installed_provider_count,
            runtime.ready_provider_count,
            runtime.total_resource_count,
            runtime.active_configuration_generation,
            runtime.generation_cleanup_pending,
            runtime.cleanup_pending_count,
        )
        .map_err(|_| ZoneStatusProjectionError::Contract)
    }
}

fn phase_for(phase: ResourcePhase) -> ZoneHandlerPhase {
    match phase {
        ResourcePhase::Ready => ZoneHandlerPhase::Ready,
        ResourcePhase::Degraded => ZoneHandlerPhase::Degraded,
        ResourcePhase::Failed => ZoneHandlerPhase::Failed,
        ResourcePhase::Unknown => ZoneHandlerPhase::Unknown,
        ResourcePhase::Pending | ResourcePhase::Succeeded | ResourcePhase::Deleted => {
            ZoneHandlerPhase::Pending
        }
    }
}
