//! The fixed system-core Host/User Zone status projection.

use d2b_contracts::v3::{ZoneHandlerName, ZoneHandlerPhase, ZoneHandlerStatus};

/// The exact serialized handler value for the Host controller.
pub const SYSTEM_CORE_HOST_HANDLER: &str = "system-core-host";
/// The exact serialized handler value for the User controller.
pub const SYSTEM_CORE_USER_HANDLER: &str = "system-core-user";

/// Failure to consume the mandatory system-core handler pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandlerReadinessError {
    /// One or both mandatory records are absent or duplicated.
    PairInvalid,
    /// Both records exist, but at least one is not ready.
    NotReady,
}

impl core::fmt::Display for HandlerReadinessError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::PairInvalid => "system-core-handler-pair-invalid",
            Self::NotReady => "system-core-handler-not-ready",
        })
    }
}

impl std::error::Error for HandlerReadinessError {}

/// Emit exactly one typed Host and one typed User handler record.
pub fn emit_handler_status(
    host_phase: ZoneHandlerPhase,
    user_phase: ZoneHandlerPhase,
    last_reconciled_at: Option<d2b_contracts::v3::Timestamp>,
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

/// Require exactly one ready Host and one ready User handler record.
pub fn require_ready_handlers(handlers: &[ZoneHandlerStatus]) -> Result<(), HandlerReadinessError> {
    let host = handlers
        .iter()
        .filter(|handler| handler.name() == ZoneHandlerName::SystemCoreHost)
        .collect::<Vec<_>>();
    let user = handlers
        .iter()
        .filter(|handler| handler.name() == ZoneHandlerName::SystemCoreUser)
        .collect::<Vec<_>>();
    if host.len() != 1 || user.len() != 1 {
        return Err(HandlerReadinessError::PairInvalid);
    }
    if host[0].phase() != ZoneHandlerPhase::Ready || user[0].phase() != ZoneHandlerPhase::Ready {
        return Err(HandlerReadinessError::NotReady);
    }
    Ok(())
}
