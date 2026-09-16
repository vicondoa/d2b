//! The provider-side operation envelope.
//!
//! One invocation runs the same five steps in the same order every time:
//! resolve the operation among the handlers the drivers declared, refuse it
//! when nothing declared it, refuse a caller the committed grants do not
//! cover, audit the attempt, and dispatch to the declared handler with an
//! invocation identifier the audit record carries.
//!
//! Resolution is the row → service+method link (U7/KD6): operations resolve
//! to services, never the other way around. Every declared operation finds
//! its declaring service's declared method on the driver's `services`
//! declarations at build time; a row no declared method serves keeps its
//! operation-keyed entry with no service link, and a row two declared
//! methods claim fails the build instead of dispatching arbitrarily.
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
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::{CanonicalJsonObject, ResourceRef, ZoneId};
use d2b_contracts_zone_session::v3::zone_routing::{ZoneLabelId, ZonePath};
use d2b_resource_types::{
    DriverDescriptor, OperationCtx, OperationFailure, OperationHandler, OperationResult,
    ValidatedPayload,
};

use d2b_resource_types::{ServiceDecl, ServiceMethod};

use crate::audit::{ProviderAgentAuditEvent, ProviderAgentAuditLog, ProviderAgentAuditOutcome};
use crate::base::error::ProviderToolkitError;

/// The refusal code for an operation no declared handler serves.
pub const UNCOMMITTED_OPERATION: &str = "uncommitted-operation";
/// The refusal code for a caller no committed grant covers.
pub const UNGRANTED_CALLER: &str = "ungranted-caller";

/// One wire/declared operation spelling. The committed rows name
/// operations in PascalCase while family `ResourceRef` labels are
/// lowercase and dash-separated; both spellings canonicalize to the same
/// token (U10 seam).
trait CanonicalOperationEq {
    fn canonical_eq(&self, other: &str) -> bool;
}

impl CanonicalOperationEq for str {
    fn canonical_eq(&self, other: &str) -> bool {
        self.chars()
            .filter(|c| *c != '-')
            .flat_map(char::to_lowercase)
            .eq(other
                .chars()
                .filter(|c| *c != '-')
                .flat_map(char::to_lowercase))
    }
}

/// The closed set of codes this envelope itself refuses with.
pub const ENVELOPE_REFUSALS: [&str; 2] = [UNCOMMITTED_OPERATION, UNGRANTED_CALLER];

/// One declared operation, resolved to the declaring service's declared
/// method, and the handler code that serves it.
///
/// The handler table stays the execution source: the resolution names where
/// the operation lives on the declaration surface, it never moves the
/// handler code. The method is the declaring `ServiceDecl`'s own method
/// object, so its contract facets (schema reference, fd contracts, state
/// cells, privileges, deadline tier) ride the entry without a second copy.
///
/// Resolution is permissive: a driver that declares operations but no
/// service methods keeps an operation-keyed entry with no service link
/// (the forward path dispatches by committed operation name), and a method
/// that IS declared fixes the link. Only an ambiguous declaration - two
/// declared methods claiming one row - fails the build.
struct HandlerEntry {
    operation: ResourceRef,
    /// The declaring service id, when the operation resolves to a declared
    /// service method.
    service: Option<&'static str>,
    /// The declared method that serves this operation, when the operation
    /// resolves to one.
    method: Option<&'static ServiceMethod>,
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
    ///
    /// Every declared operation resolves to the declaring service's declared
    /// method when the driver declares one (U7/KD6): the driver's `services`
    /// declarations name the method that serves the operation's committed
    /// row. Resolution is permissive - an operation no declared method
    /// serves keeps its operation-keyed entry with no service link, exactly
    /// as the forward path dispatches it by committed operation name - and
    /// only an ambiguous declaration (two declared methods claiming one row)
    /// fails the build. The handler table stays the execution source: the
    /// resolved service+method names where the operation lives on the
    /// declaration surface, it never moves the handler code.
    pub fn over(
        zone: ZoneId,
        provider_ref: ResourceRef,
        drivers: &[DriverDescriptor],
        audit: Arc<Mutex<ProviderAgentAuditLog>>,
    ) -> Result<Self, ProviderToolkitError> {
        let mut handlers = Vec::new();
        for driver in drivers {
            for declaration in driver.operations {
                let resolved = resolve_method(driver.services, &declaration.operation_ref)?;
                handlers.push(HandlerEntry {
                    operation: declaration.operation_ref.clone(),
                    service: resolved.as_ref().map(|resolved| resolved.service),
                    method: resolved.as_ref().map(|resolved| resolved.method),
                    handler: declaration.handler,
                });
            }
        }
        Self::from_handlers(zone, provider_ref, handlers, audit)
    }

    /// Build the envelope from explicit service and operation declarations.
    ///
    /// A provider crate assembles its `DriverDescriptor`s once; a caller
    /// that already holds the same declaration rows states them directly.
    /// Both feed one handler table, so an operation is reachable exactly
    /// where it is declared, and both resolve every operation to the
    /// declaring service's declared method when one is declared.
    pub fn from_operations(
        zone: ZoneId,
        provider_ref: ResourceRef,
        services: &'static [ServiceDecl],
        operations: &'static [d2b_resource_types::OperationDef],
        audit: Arc<Mutex<ProviderAgentAuditLog>>,
    ) -> Result<Self, ProviderToolkitError> {
        let mut handlers = Vec::new();
        for declaration in operations {
            let resolved = resolve_method(services, &declaration.operation_ref)?;
            handlers.push(HandlerEntry {
                operation: declaration.operation_ref.clone(),
                service: resolved.as_ref().map(|resolved| resolved.service),
                method: resolved.as_ref().map(|resolved| resolved.method),
                handler: declaration.handler,
            });
        }
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

    /// The declaring service id and declared method one committed operation
    /// resolves to (U7/KD6), when a declared method serves it.
    ///
    /// The service id is the session-layer address of the service whose
    /// declaration names this method; the method object carries the
    /// contract facets (schema reference, fd contracts, state cells,
    /// privileges, deadline tier) the declaration states for the row. A
    /// driver that declares the operation without a service method leaves
    /// the resolution absent - the row dispatches by operation name, with
    /// no service surface link.
    pub fn resolved_method(
        &self,
        operation: &ResourceRef,
    ) -> Option<(&'static str, &'static ServiceMethod)> {
        self.handlers.iter().find_map(|entry| {
            (entry.operation == *operation).then_some((entry.service?, entry.method?))
        })
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
    /// The envelope names the invocation: it mints the identifier the
    /// handler's context carries. A caller that already owns an invocation
    /// identifier - the broker-forwarded path - states it through
    /// [`OperationEnvelope::invoke_named`] instead.
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
        let invocation_id = self.next_invocation_id();
        self.run(&invocation_id, caller, entry, payload, &[], &[], None)
            .await
    }

    /// Whether a declared handler serves this operation name.
    ///
    /// The committed rows spell the family operations in the catalog's
    /// PascalCase wire names while a `ResourceRef` name is a lowercase,
    /// dash-separated label (`Operation/open-pidfd`), so the match
    /// canonicalizes both spellings - case AND dashes - before comparing
    /// (U10): the forwarded wire name `OpenPidfd` resolves the declared
    /// `Operation/open-pidfd` entry.
    pub fn declares(&self, operation: &str) -> bool {
        self.handlers
            .iter()
            .any(|entry| entry.operation.name().as_str().canonical_eq(operation))
    }

    /// Run one operation a bare operation name selects, under an invocation
    /// identifier the caller already owns.
    ///
    /// This is the service surface a forwarded call arrives on: the caller
    /// names the operation the way the committed row spells it, and the
    /// identifier is the one the broker's record already carries, so the
    /// provider-side record and the broker's record name one invocation. A
    /// name no declared handler serves is refused with
    /// [`UNCOMMITTED_OPERATION`] and audited, exactly as an undeclared
    /// reference is.
    pub async fn invoke_named(
        &self,
        operation: &str,
        invocation_id: &str,
        caller: &ResourceRef,
        payload: CanonicalJsonObject,
    ) -> Result<OperationResult, OperationFailure> {
        self.invoke_named_with_fds(operation, invocation_id, caller, payload, &[])
            .await
    }

    /// Invoke as [`Self::invoke_named`], with the descriptors the request
    /// frame attached to this invocation.
    ///
    /// The descriptors belong to the transport's frame,not to the handler:
    /// they are borrowed for the invocation only, and the caller closes them
    /// once it has read the reply.
    pub async fn invoke_named_with_fds(
        &self,
        operation: &str,
        invocation_id: &str,
        caller: &ResourceRef,
        payload: CanonicalJsonObject,
        fds: &[RawFd],
    ) -> Result<OperationResult, OperationFailure> {
        self.invoke_named_with_fds_under_chain(
            operation,
            invocation_id,
            caller,
            payload,
            fds,
            &[],
            None,
        )
        .await
    }

    /// Invoke as [`Self::invoke_named_with_fds`], under the evidence chain
    /// the forwarded invocation runs on and the U10 family seam.
    ///
    /// The chain identities (root first) are the ones the broker minted
    /// for the root call; the handler presents them - with its own identity
    /// appended - when it invokes a broker-generic kernel as the nested
    /// core of its family operation, so the graft rule authorizes the
    /// kernel call against the chain's initiating principal and the
    /// in-broker leg records the correlation leg (KTD6). The kernel caller
    /// carries the broker socket, the caller role, the Zone's trusted
    /// bundle, and the daemon-side runner lookup; absent for direct and
    /// test invocations.
    // The parameter set is the documented kernel-carrier surface; bundling
    // it would obscure the wire mapping each field names (KTD6 chain graft).
    #[allow(clippy::too_many_arguments)]
    pub async fn invoke_named_with_fds_under_chain(
        &self,
        operation: &str,
        invocation_id: &str,
        caller: &ResourceRef,
        payload: CanonicalJsonObject,
        fds: &[RawFd],
        chain_identities: &[String],
        kernel: Option<&d2b_resource_types::KernelCaller>,
    ) -> Result<OperationResult, OperationFailure> {
        // The committed rows spell the family operations in the catalog's
        // PascalCase wire names while a `ResourceRef` name is a lowercase
        // label, so the match is case-insensitive (U10): the forwarded wire
        // name `OpenPidfd` resolves the declared `Operation/open-pidfd`
        // entry.
        let Some(entry) = self
            .handlers
            .iter()
            .find(|entry| entry.operation.name().as_str().canonical_eq(operation))
        else {
            self.audit_named(operation, ProviderAgentAuditOutcome::Denied);
            return Err(OperationFailure::new(UNCOMMITTED_OPERATION));
        };
        self.run(
            invocation_id,
            caller,
            entry,
            payload,
            fds,
            chain_identities,
            kernel,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn run(
        &self,
        invocation_id: &str,
        caller: &ResourceRef,
        entry: &HandlerEntry,
        payload: CanonicalJsonObject,
        fds: &[RawFd],
        chain_identities: &[String],
        kernel: Option<&d2b_resource_types::KernelCaller>,
    ) -> Result<OperationResult, OperationFailure> {
        let operation = &entry.operation;
        if !self.is_granted(caller, operation) {
            self.audit(operation, ProviderAgentAuditOutcome::Denied);
            return Err(OperationFailure::new(UNGRANTED_CALLER));
        }
        let ctx = OperationCtx {
            zone: &self.zone,
            caller,
            operation,
            invocation_id,
            fds,
            chain_identities,
            kernel,
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
        self.audit_named(operation.name().as_str(), outcome);
    }

    fn audit_named(&self, operation: &str, outcome: ProviderAgentAuditOutcome) {
        let Ok(method) = BoundedToken::parse(operation) else {
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

/// One operation's resolution on the declaration surface.
struct ResolvedMethod {
    /// The declaring service id.
    service: &'static str,
    /// The declared method that serves the operation.
    method: &'static ServiceMethod,
}

/// The deadline tiers a declared method may sit on; a method that declares
/// none sits on the standard tier. The tier names are the committed rows'
/// own closed set, so the declaration cannot name a tier the rows do not
/// know.
const DEADLINE_TIERS: [&str; 2] = ["standard", "extended"];

/// Resolve one committed operation row to the declaring service's declared
/// method (U7/KD6).
///
/// Resolution walks the declaration surface, never a second table: a method
/// that names the row's operation on its service declaration is the
/// resolution. Zero candidates leave the operation unlinked - the envelope
/// keeps dispatching it by committed operation name, so a driver that
/// declares operations without service methods (the forward path's shape
/// today) is untouched. Two or more candidates fail the build: a row two
/// methods claim would dispatch arbitrarily.
fn resolve_method(
        services: &'static [ServiceDecl],
        operation: &ResourceRef,
    ) -> Result<Option<ResolvedMethod>, ProviderToolkitError> {
    let mut candidates = services
        .iter()
        .flat_map(|service| {
            service
                .methods
                .iter()
                .filter(|method| method.operation == Some(operation.name().as_str()))
                .map(move |method| ResolvedMethod {
                    service: service.id,
                    method,
                })
        });
    let Some(resolved) = candidates.next() else {
        return Ok(None);
    };
    if candidates.next().is_some() {
        return Err(ProviderToolkitError::OperationAmbiguous);
    }
    validate_facets(resolved.method)?;
    Ok(Some(resolved))
}

/// The closed facet set one declared method must stay inside.
///
/// A facet a committed row could not state - an unknown deadline tier, an
/// fd contract without the descriptor kind its ceiling requires, an empty
/// schema reference or method name - fails the build at resolution instead
/// of meaning something the rows never sanctioned.
fn validate_facets(method: &ServiceMethod) -> Result<(), ProviderToolkitError> {
    let invalid = || ProviderToolkitError::OperationFacetInvalid;
    if method.name.is_empty() {
        return Err(invalid());
    }
    if method.payload_schema.is_some_and(|schema| schema.is_empty()) {
        return Err(invalid());
    }
    if method
        .deadline_tier
        .is_some_and(|tier| !DEADLINE_TIERS.contains(&tier))
    {
        return Err(invalid());
    }
    for contract in [&method.request_fds, &method.response_fds] {
        if contract.max_fds > 0 && contract.fd_kind.is_none_or(|kind| kind.is_empty()) {
            return Err(invalid());
        }
    }
    Ok(())
}
