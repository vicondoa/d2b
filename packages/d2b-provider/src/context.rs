//! Per-call cancellation and the owned operation context.

use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use crate::{error::ProviderRuntimeError, identity::ProviderMethodName, session::SessionIdentity};

/// A cooperative cancellation flag shared by a caller and a registry.
#[derive(Clone, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
}

impl CancellationToken {
    /// A fresh, uncancelled token.
    pub fn new() -> Self {
        Self::default()
    }

    /// Cancel every holder of this token.
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    /// Whether the token is cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// The immutable context of one admitted Provider operation.
///
/// The context links the caller's cancellation token to the registry's own,
/// so retiring a registry generation cancels every call it admitted.
#[derive(Clone)]
pub struct OwnedOperationContext {
    identity: SessionIdentity,
    method: ProviderMethodName,
    deadline: Instant,
    cancellations: Arc<[CancellationToken]>,
}

impl OwnedOperationContext {
    pub(crate) fn new_linked(
        identity: SessionIdentity,
        method: ProviderMethodName,
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
            identity,
            method,
            deadline,
            cancellations: cancellations.into(),
        })
    }

    /// The authenticated identity this operation runs under.
    pub const fn identity(&self) -> &SessionIdentity {
        &self.identity
    }

    /// The method being dispatched.
    pub const fn method(&self) -> &ProviderMethodName {
        &self.method
    }

    /// Whether any linked token is cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.cancellations
            .iter()
            .any(CancellationToken::is_cancelled)
    }

    /// The remaining budget, or the reason there is none.
    pub fn remaining(&self) -> Result<Duration, ProviderRuntimeError> {
        if self.is_cancelled() {
            return Err(ProviderRuntimeError::Cancelled);
        }
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or(ProviderRuntimeError::DeadlineExpired)
    }
}

impl fmt::Debug for OwnedOperationContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedOperationContext")
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}
