//! Production Zone status projection with the mandatory system-core pair.

use d2b_contracts::v3::{ResourcePhase, ZoneHandlerPhase, ZoneHandlerStatus, ZoneStatusResource};
use d2b_provider_system_core::emit_handler_status;

/// Inputs consumed by the production Zone status emitter.
#[derive(Debug, Clone)]
pub struct ZoneStatusInput {
    core_phase: ResourcePhase,
    handlers: Vec<ZoneHandlerStatus>,
    host_phase: ZoneHandlerPhase,
    user_phase: ZoneHandlerPhase,
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
        let host_records = input
            .handlers
            .iter()
            .filter(|handler| handler.name() == d2b_contracts::v3::ZoneHandlerName::SystemCoreHost)
            .collect::<Vec<_>>();
        let user_records = input
            .handlers
            .iter()
            .filter(|handler| handler.name() == d2b_contracts::v3::ZoneHandlerName::SystemCoreUser)
            .collect::<Vec<_>>();
        let (host_phase, user_phase) = if host_records.is_empty() && user_records.is_empty() {
            (input.host_phase, input.user_phase)
        } else {
            if host_records.len() > 1 || user_records.len() > 1 {
                return Err(ZoneStatusProjectionError::Contract);
            }
            (
                host_records
                    .first()
                    .map_or(ZoneHandlerPhase::Pending, |handler| handler.phase()),
                user_records
                    .first()
                    .map_or(ZoneHandlerPhase::Pending, |handler| handler.phase()),
            )
        };
        let mut handlers = input
            .handlers
            .into_iter()
            .filter(|handler| {
                !matches!(
                    handler.name(),
                    d2b_contracts::v3::ZoneHandlerName::SystemCoreHost
                        | d2b_contracts::v3::ZoneHandlerName::SystemCoreUser
                )
            })
            .collect::<Vec<_>>();
        handlers.extend(emit_handler_status(host_phase, user_phase, None));
        ZoneStatusResource::new(0, 0, 0, input.core_phase, handlers, 0, 0, 0, 1, false, 0)
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
