use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use d2b_contracts::{
    v2_component_session::{EndpointRole, ServicePackage},
    v2_provider::{ProviderCallContext, ProviderOperationContext},
};

use crate::ProviderRuntimeError;

#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
pub struct OwnedOperationContext {
    operation: Arc<ProviderOperationContext>,
    peer_role: EndpointRole,
    service: ServicePackage,
    deadline: Instant,
    cancellations: Arc<[CancellationToken]>,
}

impl fmt::Debug for OwnedOperationContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedOperationContext")
            .field("provider_type", &self.operation.provider_type)
            .field("method", &self.operation.method)
            .field("provider_generation", &self.operation.provider_generation)
            .field("peer_role", &self.peer_role)
            .field("service", &self.service)
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}

impl OwnedOperationContext {
    pub fn new(
        operation: ProviderOperationContext,
        peer_role: EndpointRole,
        service: ServicePackage,
        deadline_after: Duration,
        cancellation: CancellationToken,
    ) -> Result<Self, ProviderRuntimeError> {
        Self::new_linked(
            operation,
            peer_role,
            service,
            deadline_after,
            vec![cancellation],
        )
    }

    pub(crate) fn new_linked(
        operation: ProviderOperationContext,
        peer_role: EndpointRole,
        service: ServicePackage,
        deadline_after: Duration,
        cancellations: Vec<CancellationToken>,
    ) -> Result<Self, ProviderRuntimeError> {
        if deadline_after.is_zero() || deadline_after.as_millis() > u128::from(u32::MAX) {
            return Err(ProviderRuntimeError::DeadlineExpired);
        }
        if cancellations.is_empty() {
            return Err(ProviderRuntimeError::Cancelled);
        }
        let deadline = Instant::now()
            .checked_add(deadline_after)
            .ok_or(ProviderRuntimeError::DeadlineExpired)?;
        Ok(Self {
            operation: Arc::new(operation),
            peer_role,
            service,
            deadline,
            cancellations: cancellations.into(),
        })
    }

    pub fn operation(&self) -> &ProviderOperationContext {
        &self.operation
    }

    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellations[0]
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellations
            .iter()
            .any(CancellationToken::is_cancelled)
    }

    pub fn remaining(&self) -> Result<Duration, ProviderRuntimeError> {
        if self.is_cancelled() {
            return Err(ProviderRuntimeError::Cancelled);
        }
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(ProviderRuntimeError::DeadlineExpired)
    }

    pub fn call_context(&self) -> Result<ProviderCallContext<'_>, ProviderRuntimeError> {
        let remaining = self.remaining()?;
        let remaining_ms = remaining.as_millis().clamp(1, u128::from(u32::MAX)) as u32;
        let context = ProviderCallContext {
            operation: &self.operation,
            peer_role: self.peer_role,
            service: self.service,
            monotonic_deadline_remaining_ms: remaining_ms,
            cancelled: false,
        };
        context.validate()?;
        Ok(context)
    }
}
