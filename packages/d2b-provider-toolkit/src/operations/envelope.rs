//! The provider-side operation envelope.
//!
//! One invocation runs the same five steps in the same order every time:
//! resolve the operation among the handlers the drivers declared, refuse it
//! when nothing declared it, refuse a caller the committed grants do not
//! cover, audit the attempt, and dispatch to the declared handler with an
//! invocation identifier the audit record carries.
//!
//! Two properties are structural rather than conventional. First, the
//! handler table is the drivers' own `operations` declarations, so an
//! operation that was never declared cannot be reached - there is no second
//! registration step to forget. Second, grants are deny-by-default and live
//! outside the descriptor: a handler's presence in a descriptor is never
//! authority, so a caller can only invoke an operation a committed grant
//! covers.
//!
//! Neither step is a substitute for the broker's envelope. The broker
//! validates the payload against the operation row's schema, authorizes
//! against the committed rows, and mints the durable `invocationId`; this
//! envelope is the provider-side path with the same shape, used by the base
//! and by the test harness. Where a fact it does not own is required - the
//! committed payload schema of an operation row - it refuses rather than
//! guessing, and the payload it forwards is the canonical object the caller
//! already validated.

use std::collections::BTreeSet;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::{CanonicalJsonObject, ResourceRef, ZoneId};
use d2b_contracts_zone_session::v3::zone_routing::{ZoneLabelId, ZonePath};
use d2b_resource_types::{
    DriverDescriptor, OperationCtx, OperationFailure, OperationHandler, OperationResult,
    ValidatedPayload,
};

use crate::audit::{ProviderAgentAuditEvent, ProviderAgentAuditLog, ProviderAgentAuditOutcome};
use crate::base::error::ProviderToolkitError;

/// The refusal code for an operation no declared handler serves.
pub const UNCOMMITTED_OPERATION: &str = "uncommitted-operation";
/// The refusal code for a caller no committed grant covers.
pub const UNGRANTED_CALLER: &str = "ungranted-caller";

/// The closed set of codes this envelope itself refuses with.
pub const ENVELOPE_REFUSALS: [&str; 2] = [UNCOMMITTED_OPERATION, UNGRANTED_CALLER];

/// One declared operation and the handler code that serves it.
struct HandlerEntry {
    operation: ResourceRef,
    handler: &'static dyn OperationHandler,
}

/// One committed grant: this caller may invoke this operation.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct GrantEntry {
    caller: ResourceRef,
    operation: ResourceRef,
}

/// The provider-side operation envelope.
pub struct OperationEnvelope {
    provider_ref: ResourceRef,
    zone: ZoneId,
    zone_path: ZonePath,
    handlers: Vec<HandlerEntry>,
    grants: RwLock<BTreeSet<GrantEntry>>,
    audit: Arc<Mutex<ProviderAgentAuditLog>>,
    invocations: AtomicU64,
}

impl fmt::Debug for OperationEnvelope {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OperationEnvelope")
            .field("provider_ref", &self.provider_ref)
            .field("handler_count", &self.handlers.len())
            .field("grant_count", &self.grant_count())
            .finish_non_exhaustive()
    }
}

impl OperationEnvelope {
    /// Build the envelope from the handlers the drivers declared.
    pub fn over(
        zone: ZoneId,
        provider_ref: ResourceRef,
        drivers: &[DriverDescriptor],
        audit: Arc<Mutex<ProviderAgentAuditLog>>,
    ) -> Result<Self, ProviderToolkitError> {
        let mut handlers = Vec::new();
        for driver in drivers {
            for declaration in driver.operations {
                handlers.push(HandlerEntry {
                    operation: declaration.operation_ref.clone(),
                    handler: declaration.handler,
                });
            }
        }
        Self::from_handlers(zone, provider_ref, handlers, audit)
    }

    /// Build the envelope from explicit operation declarations.
    ///
    /// A provider crate assembles its `DriverDescriptor`s once; a caller
    /// that already holds the same declaration rows states them directly.
    /// Both feed one handler table, so an operation is reachable exactly
    /// where it is declared.
    pub fn from_operations(
        zone: ZoneId,
        provider_ref: ResourceRef,
        operations: &'static [d2b_resource_types::OperationDef],
        audit: Arc<Mutex<ProviderAgentAuditLog>>,
    ) -> Result<Self, ProviderToolkitError> {
        let handlers = operations
            .iter()
            .map(|declaration| HandlerEntry {
                operation: declaration.operation_ref.clone(),
                handler: declaration.handler,
            })
            .collect();
        Self::from_handlers(zone, provider_ref, handlers, audit)
    }

    fn from_handlers(
        zone: ZoneId,
        provider_ref: ResourceRef,
        handlers: Vec<HandlerEntry>,
        audit: Arc<Mutex<ProviderAgentAuditLog>>,
    ) -> Result<Self, ProviderToolkitError> {
        let label =
            ZoneLabelId::parse(zone.as_str()).map_err(|_| ProviderToolkitError::WireInvalid)?;
        let zone_path =
            ZonePath::new(vec![label]).map_err(|_| ProviderToolkitError::WireInvalid)?;
        Ok(Self {
            provider_ref,
            zone,
            zone_path,
            handlers,
            grants: RwLock::new(BTreeSet::new()),
            audit,
            invocations: AtomicU64::new(1),
        })
    }

    /// Borrow the provider reference the audit records carry.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the zone the invocations run in.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// The number of committed grants.
    pub fn grant_count(&self) -> usize {
        self.grants.read().map(|grants| grants.len()).unwrap_or(0)
    }

    /// Every operation a declared handler serves.
    pub fn declared_operations(&self) -> impl Iterator<Item = &ResourceRef> + '_ {
        self.handlers.iter().map(|entry| &entry.operation)
    }

    /// Whether any declared handler serves this operation.
    pub fn is_declared(&self, operation: &ResourceRef) -> bool {
        self.handlers
            .iter()
            .any(|entry| entry.operation == *operation)
    }

    /// Whether a committed grant covers this invocation.
    pub fn is_granted(&self, caller: &ResourceRef, operation: &ResourceRef) -> bool {
        self.grants
            .read()
            .map(|grants| {
                grants.contains(&GrantEntry {
                    caller: caller.clone(),
                    operation: operation.clone(),
                })
            })
            .unwrap_or(false)
    }

    /// Commit one grant: this caller may invoke this operation.
    ///
    /// The grant is the only authority the envelope consults, so a caller
    /// with no committed grant is refused even when the operation is
    /// declared.
    pub fn commit_grant(&self, caller: &ResourceRef, operation: &ResourceRef) {
        if let Ok(mut grants) = self.grants.write() {
            grants.insert(GrantEntry {
                caller: caller.clone(),
                operation: operation.clone(),
            });
        }
    }

    /// Revoke one committed grant.
    pub fn revoke_grant(&self, caller: &ResourceRef, operation: &ResourceRef) -> bool {
        self.grants
            .write()
            .map(|mut grants| {
                grants.remove(&GrantEntry {
                    caller: caller.clone(),
                    operation: operation.clone(),
                })
            })
            .unwrap_or(false)
    }

    /// Invoke one operation through the envelope.
    ///
    /// The returned failure is the handler's own when the invocation reached
    /// it, and one of [`ENVELOPE_REFUSALS`] when it did not.
    pub async fn call(
        &self,
        caller: &ResourceRef,
        operation: &ResourceRef,
        payload: CanonicalJsonObject,
    ) -> Result<OperationResult, OperationFailure> {
        let Some(entry) = self
            .handlers
            .iter()
            .find(|entry| entry.operation == *operation)
        else {
            self.audit(operation, ProviderAgentAuditOutcome::Denied);
            return Err(OperationFailure::new(UNCOMMITTED_OPERATION));
        };
        if !self.is_granted(caller, operation) {
            self.audit(operation, ProviderAgentAuditOutcome::Denied);
            return Err(OperationFailure::new(UNGRANTED_CALLER));
        }
        let invocation_id = self.next_invocation_id();
        let ctx = OperationCtx {
            zone: &self.zone,
            caller,
            operation,
            invocation_id: &invocation_id,
        };
        let result = entry
            .handler
            .execute(ctx, ValidatedPayload::new(payload))
            .await;
        let outcome = match &result {
            Ok(_) => ProviderAgentAuditOutcome::Accepted,
            Err(_) => ProviderAgentAuditOutcome::Failed,
        };
        self.audit(operation, outcome);
        result
    }

    fn next_invocation_id(&self) -> String {
        format!(
            "invocation-{}",
            self.invocations.fetch_add(1, Ordering::AcqRel)
        )
    }

    fn audit(&self, operation: &ResourceRef, outcome: ProviderAgentAuditOutcome) {
        let Ok(method) = BoundedToken::parse(operation.name().as_str()) else {
            return;
        };
        if let Ok(mut audit) = self.audit.lock() {
            audit.record(ProviderAgentAuditEvent::new(
                self.zone_path.clone(),
                self.provider_ref.clone(),
                method,
                outcome,
            ));
        }
    }
}
