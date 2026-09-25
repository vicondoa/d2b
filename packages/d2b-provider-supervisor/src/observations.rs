//! Bounded pending-observation ledger shared by the process backends.

use std::collections::BTreeMap;
use std::sync::Mutex;

use d2b_provider_process::{ProcessEffectError, ProcessIdentityDigest};
use tracing::error;

/// Upper bound on pending observations retained per backend.
pub(crate) const MAX_PENDING_OBSERVATIONS: usize = 1024;

/// Insert one pending observation, evicting the oldest entry when the ledger
/// is at its bound and the identity is new.
///
/// Sync by construction: the ledger sits behind the sync
/// `ProcessEffectBackend` trait surface, invoked only from the dedicated
/// blocking workers (or sync test harnesses); the critical section is short
/// and never held across a suspension point.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub(crate) fn record<V>(
    observations: &Mutex<BTreeMap<ProcessIdentityDigest, V>>,
    identity: ProcessIdentityDigest,
    observed: V,
) -> Result<(), ProcessEffectError> {
    let mut observations = observations.lock().map_err(|_| {
        error!(
            provider = "supervisor",
            "observation ledger lock poisoned; observe failed"
        );
        ProcessEffectError::ObserveFailed
    })?;
    if observations.len() >= MAX_PENDING_OBSERVATIONS
        && !observations.contains_key(&identity)
        && let Some(oldest) = observations.keys().next().copied()
    {
        observations.remove(&oldest);
    }
    observations.insert(identity, observed);
    Ok(())
}

/// Take one pending observation out of the ledger.
///
/// Sync by construction: backend ledger behind the sync trait surface (see
/// `record`); critical section short, no suspension inside the guard.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
pub(crate) fn take<V>(
    observations: &Mutex<BTreeMap<ProcessIdentityDigest, V>>,
    identity: &ProcessIdentityDigest,
) -> Result<V, ProcessEffectError> {
    observations
        .lock()
        .map_err(|_| {
            error!(
                provider = "supervisor",
                "observation ledger lock poisoned; observation lookup failed"
            );
            ProcessEffectError::ObserveFailed
        })?
        .remove(identity)
        .ok_or(ProcessEffectError::IdentityChanged)
}