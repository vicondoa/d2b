//! Bounded Provider-agent dispatch and service-server adapters.
//!
//! The adapter is intentionally transport-agnostic.  A ComponentSession
//! receive loop supplies a canonical request, while this module owns the
//! fixed 64-call admission ceiling, the 1024-entry diagnostic audit ring,
//! and the bounded shutdown state.

use std::sync::Mutex;

use d2b_contracts::v3::{
    CanonicalJsonObject, ResourceRef, execution_policy::BoundedToken, zone_routing::ZonePath,
};

use crate::{
    DispatchLimiter, ProviderAgentAuditEvent, ProviderAgentAuditLog, ProviderAgentAuditOutcome,
    ProviderToolkitError,
};

/// Validate the strict attachment-index sequence carried by a Provider
/// adapter.  Descriptors are numbered from zero and may not repeat, reorder,
/// or skip an index; rejecting before dispatch prevents an adapter from
/// confusing a stale attachment with a current one.
pub fn validate_attachment_indexes(indexes: &[u32]) -> Result<(), ProviderToolkitError> {
    for (expected, observed) in indexes.iter().enumerate() {
        if *observed != expected as u32 {
            return Err(ProviderToolkitError::NonMonotoneAttachmentIndexes);
        }
    }
    Ok(())
}

/// Provider-specific service implementation behind the generic adapter.
pub trait ProviderService: Send + Sync {
    /// Dispatch one canonical method payload.
    fn dispatch(
        &self,
        method: &BoundedToken,
        payload: &CanonicalJsonObject,
    ) -> Result<CanonicalJsonObject, ProviderToolkitError>;
}

/// Bounded Provider-agent adapter.
pub struct ProviderAgentAdapter<S> {
    service: S,
    dispatch: DispatchLimiter,
    audit: Mutex<ProviderAgentAuditLog>,
}

impl<S> ProviderAgentAdapter<S> {
    /// Construct an adapter with the frozen dispatch and audit bounds.
    pub fn new(service: S) -> Self {
        Self {
            service,
            dispatch: DispatchLimiter::new(),
            audit: Mutex::new(ProviderAgentAuditLog::new()),
        }
    }

    /// Borrow the service implementation.
    pub const fn service(&self) -> &S {
        &self.service
    }

    /// Borrow dispatch accounting.
    pub const fn dispatch_limiter(&self) -> &DispatchLimiter {
        &self.dispatch
    }

    /// Snapshot retained audit events.
    pub fn audit_len(&self) -> usize {
        self.audit
            .lock()
            .map(|audit| audit.len())
            .unwrap_or_default()
    }
}

impl<S> ProviderAgentAdapter<S>
where
    S: ProviderService,
{
    /// Dispatch one request under bounded admission and record its outcome.
    pub fn dispatch(
        &self,
        zone: ZonePath,
        provider_ref: ResourceRef,
        method: BoundedToken,
        payload: CanonicalJsonObject,
    ) -> Result<CanonicalJsonObject, ProviderToolkitError> {
        let _permit = self.dispatch.acquire()?;
        let result = self.service.dispatch(&method, &payload);
        let outcome = if result.is_ok() {
            ProviderAgentAuditOutcome::Accepted
        } else {
            ProviderAgentAuditOutcome::Failed
        };
        if let Ok(mut audit) = self.audit.lock() {
            audit.record(ProviderAgentAuditEvent::new(
                zone,
                provider_ref,
                method,
                outcome,
            ));
        }
        result
    }
}

/// Generated-service registration facade over a Provider agent adapter.
pub struct GeneratedProviderServiceServer<S> {
    adapter: ProviderAgentAdapter<S>,
}

impl<S> GeneratedProviderServiceServer<S> {
    /// Construct the generated service facade.
    pub fn new(service: S) -> Self {
        Self {
            adapter: ProviderAgentAdapter::new(service),
        }
    }

    /// Borrow the bounded adapter.
    pub const fn adapter(&self) -> &ProviderAgentAdapter<S> {
        &self.adapter
    }
}

/// Fixed Provider-agent shutdown state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProviderAgentProcess {
    shutdown_deadline_ms: u32,
    stopping: bool,
}

impl ProviderAgentProcess {
    /// Maximum shutdown deadline accepted by the toolkit.
    pub const MAX_SHUTDOWN_DEADLINE_MS: u32 = 5_000;

    /// Construct a running process state.
    pub fn new(shutdown_deadline_ms: u32) -> Result<Self, ProviderToolkitError> {
        if shutdown_deadline_ms == 0 || shutdown_deadline_ms > Self::MAX_SHUTDOWN_DEADLINE_MS {
            return Err(ProviderToolkitError::CapacityOutOfRange);
        }
        Ok(Self {
            shutdown_deadline_ms,
            stopping: false,
        })
    }

    /// Request bounded shutdown.
    pub fn stop(&mut self) {
        self.stopping = true;
    }

    /// Whether shutdown has been requested.
    pub const fn stopping(&self) -> bool {
        self.stopping
    }

    /// Return the configured shutdown deadline.
    pub const fn shutdown_deadline_ms(&self) -> u32 {
        self.shutdown_deadline_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::zone_routing::ZoneLabelId;
    use d2b_contracts::v3::{ResourceName, ResourceTypeName};

    struct Echo;

    impl ProviderService for Echo {
        fn dispatch(
            &self,
            _method: &BoundedToken,
            payload: &CanonicalJsonObject,
        ) -> Result<CanonicalJsonObject, ProviderToolkitError> {
            Ok(payload.clone())
        }
    }

    fn zone() -> ZonePath {
        ZonePath::new(vec![ZoneLabelId::parse("dev").unwrap()]).unwrap()
    }

    fn provider() -> ResourceRef {
        ResourceRef::new(
            ResourceTypeName::parse("Provider").unwrap(),
            ResourceName::parse("system-core").unwrap(),
        )
    }

    #[test]
    fn adapter_dispatches_and_records_only_bounded_metadata() {
        let adapter = ProviderAgentAdapter::new(Echo);
        let result = adapter
            .dispatch(
                zone(),
                provider(),
                BoundedToken::parse("inspect").unwrap(),
                CanonicalJsonObject::parse(br#"{"ok":true}"#).unwrap(),
            )
            .unwrap();
        assert_eq!(
            result,
            CanonicalJsonObject::parse(br#"{"ok":true}"#).unwrap()
        );
        assert_eq!(adapter.audit_len(), 1);
    }

    #[test]
    fn shutdown_deadline_is_bounded() {
        assert!(ProviderAgentProcess::new(5_001).is_err());
        let mut process = ProviderAgentProcess::new(5_000).unwrap();
        process.stop();
        assert!(process.stopping());
    }

    #[test]
    fn attachment_indexes_are_strictly_monotone() {
        assert!(validate_attachment_indexes(&[0, 1, 2]).is_ok());
        assert_eq!(
            validate_attachment_indexes(&[0, 2]),
            Err(ProviderToolkitError::NonMonotoneAttachmentIndexes)
        );
        assert_eq!(
            validate_attachment_indexes(&[1, 0]),
            Err(ProviderToolkitError::NonMonotoneAttachmentIndexes)
        );
    }
}
