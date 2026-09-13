//! The broker operation envelope.
//!
//! One invocation runs the same five steps in the same order every time:
//! resolve the committed operation row the caller named, authorize the caller
//! against that row's committed grants, validate the payload against the row's
//! declared shape, audit the attempt with an `invocationId`, and dispatch to
//! the handler the declaring driver registered. Authorization precedes
//! validation deliberately: a caller no grant covers learns nothing about the
//! shape of an operation it may not invoke.
//!
//! Three properties are structural rather than conventional. The row table is
//! the committed catalog, so an operation no row declares cannot be reached.
//! Grants are read from the committed authorization facet of that row and
//! never from the handler table, so a registered handler is never authority.
//! Both refusals and successes write exactly one audit record carrying the
//! invocation identifier, so an operator can follow one invocation end to end.
//!
//! The envelope is the broker-side path. The broker links no provider crate,
//! so a family-owned row's handler runs in the declaring crate's process and
//! the dispatch step forwards to it; [`OperationDispatcher`] is that seam, and
//! a row whose declaring process has not registered is refused rather than
//! served locally. [`ForwardedOperation`](crate::forwarding::ForwardedOperation)
//! is the shape that crosses to the declaring process, and
//! [`DirectInvocation`] is the shape a local handler is handed.

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};

use d2b_contracts_broker::broker_wire::BrokerCallerRole;
use d2b_contracts_resource::v3::CanonicalJsonObject;
use serde_json::Value;

use crate::catalog::{
    BROKER_OPERATION_CATALOG, BrokerOperationRow, BrokerProfileId, OperationOwner,
    PayloadProvenance,
};

/// The refusal code for a name no committed row declares.
pub const UNKNOWN_OPERATION: &str = "unknown-operation";
/// The refusal code for a declared operation this broker holds no committed
/// row for.
pub const UNCOMMITTED_OPERATION: &str = "uncommitted-operation";
/// The refusal code for a caller no committed grant covers.
pub const UNGRANTED_CALLER: &str = "ungranted-caller";
/// The refusal code for an operation whose payload contract is the typed wire
/// request rather than a caller-supplied payload object.
pub const WIRE_INHERITED_OPERATION: &str = "wire-inherited-operation";
/// The refusal code for a payload the row's schema does not admit.
pub const INVALID_PAYLOAD: &str = "invalid-payload";
/// The refusal code for an operation whose declaring process has not
/// registered a handler.
pub const UNREGISTERED_HANDLER: &str = "unregistered-handler";
/// The failure code for a dispatch the broker could not complete.
pub const ERRORED: &str = "errored";

/// The closed set of codes the envelope itself refuses with.
pub const ENVELOPE_REFUSALS: [&str; 6] = [
    UNKNOWN_OPERATION,
    UNCOMMITTED_OPERATION,
    UNGRANTED_CALLER,
    WIRE_INHERITED_OPERATION,
    INVALID_PAYLOAD,
    UNREGISTERED_HANDLER,
];

/// One refused invocation, named.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnvelopeRefusal {
    /// The refused operation.
    pub operation: String,
    /// The invocation identifier the refusal's audit record carries.
    pub invocation_id: String,
    /// The closed refusal code.
    pub code: &'static str,
    /// Redacted detail a refusing peer contributed, when it contributed one.
    ///
    /// The code the caller sees is this envelope's own closed set; a peer's
    /// refusal code is not one of them, so it travels here and never widens
    /// the vocabulary a caller can observe.
    pub detail: Option<String>,
}

impl EnvelopeRefusal {
    fn new(invocation_id: String, operation: &str, code: &'static str) -> Self {
        Self::with_detail(invocation_id, operation, code, None)
    }

    fn with_detail(
        invocation_id: String,
        operation: &str,
        code: &'static str,
        detail: Option<String>,
    ) -> Self {
        Self {
            operation: operation.to_owned(),
            invocation_id,
            code,
            detail,
        }
    }

    /// The audit fields one refusal record carries.
    pub fn audit_fields(&self) -> Value {
        serde_json::json!({
            "operation": self.operation,
            "invocation_id": self.invocation_id,
            "reason": self.code,
            "detail": self.detail,
        })
    }
}

/// The authenticated caller of one invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallerAuthority {
    /// The daemon's own authority: it may invoke any operation its grants
    /// cover.
    Daemon,
    /// An admin-class caller.
    Admin,
    /// A launcher-class caller.
    Launcher,
    /// A caller no committed grant admits.
    Unauthorized,
}

impl CallerAuthority {
    /// The authority classes the caller carries, in the vocabulary the
    /// committed grants are written in.
    ///
    /// Each class carries exactly the grants its class implies: a launcher
    /// is not the daemon, so a row granted to `d2bd` alone refuses it rather
    /// than admitting it through the daemon's class.
    pub fn classes(self) -> BTreeSet<&'static str> {
        match self {
            Self::Daemon => BTreeSet::from(["d2bd"]),
            Self::Admin => BTreeSet::from(["d2bd", "d2b-admin"]),
            Self::Launcher => BTreeSet::from(["d2b-launcher"]),
            Self::Unauthorized => BTreeSet::new(),
        }
    }

    /// Classify the daemon-forwarded caller role.
    pub fn classify(role: &BrokerCallerRole) -> Self {
        match role {
            BrokerCallerRole::AdminUid { .. } => Self::Admin,
            BrokerCallerRole::RootUid { .. } => Self::Admin,
            BrokerCallerRole::LauncherUid { .. } => Self::Launcher,
            BrokerCallerRole::HostShutdownUid { .. } => Self::Daemon,
            BrokerCallerRole::NotAuthorized => Self::Unauthorized,
        }
    }
}

/// One dispatched invocation as the handler sees it.
#[derive(Debug)]
pub struct InvocationCtx<'a> {
    /// The operation name.
    pub operation: &'a str,
    /// The Zone the invocation runs in.
    pub zone: &'a str,
    /// The invocation identifier the audit record carries.
    pub invocation_id: &'a str,
}

/// The result of one dispatched invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchOutcome {
    /// The canonical result payload.
    pub result: CanonicalJsonObject,
}

/// Why one dispatch produced no result.
///
/// The two cases stay apart because the envelope reports them differently:
/// an operation no handler serves is the fail-closed
/// [`UNREGISTERED_HANDLER`] refusal, while a handler that ran and failed has
/// its own code and must never be reported as a missing handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DispatchFailure {
    /// The closed refusal code.
    pub code: String,
    /// Redacted detail for the operator's log.
    pub detail: Option<String>,
}

impl DispatchFailure {
    /// A refusal with the code the refusing side decided.
    pub fn new(code: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: None,
        }
    }

    /// A refusal with the code the refusing side decided and its detail.
    pub fn with_detail(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: Some(detail.into()),
        }
    }

    /// The refusal of an operation no handler serves.
    pub fn unregistered_handler(detail: impl Into<String>) -> Self {
        Self::with_detail(UNREGISTERED_HANDLER, detail)
    }
}

/// One granted invocation as the caller and the audit log see it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invocation {
    /// The invocation identifier the audit record carries.
    pub invocation_id: String,
    /// The invoked row's declared audit join over the validated payload, as
    /// the canonical operation identity of the record.
    ///
    /// A row that declares no join carries no per-invocation identity, and
    /// the record falls back to its derived identity.
    pub audit_join_identity: Option<String>,
    /// The dispatched result.
    pub outcome: DispatchOutcome,
}

/// One validated, authorized invocation as a local handler sees it.
#[derive(Debug)]
pub struct DirectInvocation<'a> {
    /// The invocation's context.
    pub ctx: InvocationCtx<'a>,
    /// The canonical payload the envelope validated against the row.
    pub payload: &'a CanonicalJsonObject,
}

/// The handler-table seam of one committed operation.
///
/// The broker holds the committed rows; the handler code lives in the
/// declaring crate's process. An implementation either serves the operation
/// locally or forwards it, and refuses an operation it was not given a
/// handler for - never serving a row it does not implement.
pub trait OperationDispatcher: Send + Sync {
    /// Run one validated, authorized invocation.
    fn dispatch(
        &self,
        invocation: DirectInvocation<'_>,
    ) -> Result<DispatchOutcome, DispatchFailure>;
}

/// The broker-side operation envelope.
pub struct BrokerEnvelope {
    rows: Vec<BrokerOperationRow>,
    committed: BTreeSet<&'static str>,
    profile: BrokerProfileId,
    dispatcher: Box<dyn OperationDispatcher>,
    invocations: AtomicU64,
}

impl std::fmt::Debug for BrokerEnvelope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BrokerEnvelope")
            .field("profile", &self.profile)
            .field("committed_rows", &self.committed.len())
            .finish_non_exhaustive()
    }
}

impl BrokerEnvelope {
    /// Build the envelope over the committed rows one profile serves.
    pub fn over(
        profile: BrokerProfileId,
        dispatcher: Box<dyn OperationDispatcher>,
    ) -> BrokerEnvelopeBuilder {
        BrokerEnvelopeBuilder {
            profile,
            dispatcher,
            committed: Vec::new(),
            extras: Vec::new(),
        }
    }

    /// The committed rows this envelope serves.
    pub fn committed_rows(&self) -> impl Iterator<Item = &BrokerOperationRow> + '_ {
        self.rows
            .iter()
            .filter(|row| self.committed.contains(row.operation))
    }

    /// The profiles this envelope serves.
    pub fn profile(&self) -> BrokerProfileId {
        self.profile
    }

    /// Invoke one operation through the envelope.
    ///
    /// The invocation is named before it is authorized, so a refusal carries
    /// the same identifier a success would have carried: an operator can
    /// follow a denied invocation in the audit log.
    pub fn call(
        &self,
        caller: CallerAuthority,
        operation: &str,
        zone: &str,
        payload: &Value,
    ) -> Result<Invocation, EnvelopeRefusal> {
        let invocation_id = format!(
            "invocation-{}",
            self.invocations.fetch_add(1, Ordering::AcqRel)
        );
        let Some(row) = self.rows.iter().find(|row| row.operation == operation) else {
            return Err(EnvelopeRefusal::new(
                invocation_id,
                operation,
                UNKNOWN_OPERATION,
            ));
        };
        if !self.committed.contains(row.operation) || !row.admits_profile(self.profile) {
            return Err(EnvelopeRefusal::new(
                invocation_id,
                operation,
                UNCOMMITTED_OPERATION,
            ));
        }
        if row.payload_provenance == PayloadProvenance::Wire {
            return Err(EnvelopeRefusal::new(
                invocation_id,
                operation,
                WIRE_INHERITED_OPERATION,
            ));
        }
        if !Self::granted(row, caller) {
            return Err(EnvelopeRefusal::new(
                invocation_id,
                operation,
                UNGRANTED_CALLER,
            ));
        }
        let payload = Self::validate(row, payload)
            .map_err(|_| EnvelopeRefusal::new(invocation_id.clone(), operation, INVALID_PAYLOAD))?;
        let ctx = InvocationCtx {
            operation: row.operation,
            zone,
            invocation_id: &invocation_id,
        };
        let outcome = self
            .dispatcher
            .dispatch(DirectInvocation {
                ctx,
                payload: &payload,
            })
            .map_err(|failure| {
                // The envelope's refusals are a closed set of broker-side
                // codes; a failure a forwarder raised carries the peer's own
                // code, which is not one of them. The closed code the caller
                // sees stays the fail-closed missing-handler refusal, and the
                // peer's code rides along as the detail the audit record
                // keeps for an operator.
                EnvelopeRefusal::with_detail(
                    invocation_id.clone(),
                    operation,
                    UNREGISTERED_HANDLER,
                    failure.detail.or_else(|| Some(failure.code)),
                )
            })?;
        Ok(Invocation {
            audit_join_identity: row.audit_join_identity(&payload),
            invocation_id,
            outcome,
        })
    }

    /// Whether the committed grants of one row cover one caller.
    ///
    /// Deny by default: a row that admits no authority class refuses every
    /// caller, and a caller class the row does not name is refused.
    pub fn granted(row: &BrokerOperationRow, caller: CallerAuthority) -> bool {
        let classes = caller.classes();
        row.authz
            .allowed_groups
            .iter()
            .any(|group| classes.contains(group))
    }

    /// Validate one payload against the row's declared shape.
    ///
    /// A committed row's payload contract is its declared property set: the
    /// object is closed, so an undeclared field is a refusal rather than an
    /// ignored value, and every declared field must be a JSON scalar, array,
    /// or object this envelope can hand to a handler unchanged.
    pub fn validate(
        row: &BrokerOperationRow,
        payload: &Value,
    ) -> Result<CanonicalJsonObject, &'static str> {
        let Value::Object(fields) = payload else {
            return Err(INVALID_PAYLOAD);
        };
        for name in fields.keys() {
            if !row.payload_fields.contains(&name.as_str()) {
                return Err(INVALID_PAYLOAD);
            }
        }
        for name in row.payload_required {
            if !fields.contains_key(*name) {
                return Err(INVALID_PAYLOAD);
            }
        }
        serde_json::from_value(payload.clone()).map_err(|_| INVALID_PAYLOAD)
    }
}

/// Assemble one [`BrokerEnvelope`].
pub struct BrokerEnvelopeBuilder {
    profile: BrokerProfileId,
    dispatcher: Box<dyn OperationDispatcher>,
    committed: Vec<&'static str>,
    extras: Vec<BrokerOperationRow>,
}

impl BrokerEnvelopeBuilder {
    /// Commit the rows the broker serves for this profile.
    ///
    /// A row absent from the committed set is refused as uncommitted even
    /// though the catalog declares it, which is how a declared-but-unwired
    /// operation stays unreachable.
    pub fn commit(mut self, operation: &'static str) -> Self {
        self.committed.push(operation);
        self
    }

    /// Commit every broker-generic row the envelope can carry.
    ///
    /// An operation whose payload contract is the typed wire request is not
    /// carried by this envelope, so it is not committed here.
    pub fn commit_broker_generic(mut self) -> Self {
        self.committed.extend(
            BROKER_OPERATION_CATALOG
                .iter()
                .filter(|row| {
                    row.owner == OperationOwner::BrokerGeneric
                        && row.payload_provenance == PayloadProvenance::Request
                })
                .map(|row| row.operation),
        );
        self
    }

    /// Commit every broker-generic row and every row a declaring crate owns.
    ///
    /// This is the live set: the broker's own operations plus the family rows
    /// whose handler the declaring process serves, which the dispatch step
    /// forwards to it. A row that names no declaring process stays
    /// uncommitted - the envelope has nowhere to send it, so admitting it
    /// would turn a wiring gap into a refusal that looks like the caller's
    /// fault.
    pub fn commit_forwarded(mut self) -> Self {
        self.committed.extend(
            BROKER_OPERATION_CATALOG
                .iter()
                .filter(|row| {
                    (row.owner == OperationOwner::BrokerGeneric
                        && row.payload_provenance == PayloadProvenance::Request)
                        || (row.owner == OperationOwner::Family && row.declaring_provider.is_some())
                })
                .map(|row| row.operation),
        );
        self
    }

    /// Commit every row in the catalog.
    pub fn commit_all(mut self) -> Self {
        self.committed
            .extend(BROKER_OPERATION_CATALOG.iter().map(|row| row.operation));
        self
    }

    /// Declare one row beyond the committed catalog.
    ///
    /// The gate reads the catalog; this is how a test drives an operation the
    /// catalog does not carry yet, proving that a new operation needs a row
    /// and a handler rather than a wire change.
    pub fn declare(mut self, row: BrokerOperationRow) -> Self {
        self.committed.push(row.operation);
        self.extras.push(row);
        self
    }

    /// Build the envelope.
    pub fn build(self) -> BrokerEnvelope {
        let rows = BROKER_OPERATION_CATALOG
            .iter()
            .copied()
            .chain(self.extras)
            .collect();
        BrokerEnvelope {
            rows,
            committed: self.committed.into_iter().collect(),
            profile: self.profile,
            dispatcher: self.dispatcher,
            invocations: AtomicU64::new(1),
        }
    }
}

/// A dispatcher that serves an explicit handler table.
///
/// Used by the broker's own operations and by the tests; a family row is
/// served by the declaring crate's process, not here.
#[derive(Default)]
pub struct HandlerTable {
    handlers: Vec<(
        &'static str,
        Box<
            dyn Fn(&DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure>
                + Send
                + Sync,
        >,
    )>,
}

impl std::fmt::Debug for HandlerTable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HandlerTable")
            .field("handlers", &self.handlers.len())
            .finish_non_exhaustive()
    }
}

impl HandlerTable {
    /// An empty handler table.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register one handler.
    pub fn with(
        mut self,
        operation: &'static str,
        handler: impl Fn(&DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.handlers.push((operation, Box::new(handler)));
        self
    }
}

impl OperationDispatcher for HandlerTable {
    fn dispatch(
        &self,
        invocation: DirectInvocation<'_>,
    ) -> Result<DispatchOutcome, DispatchFailure> {
        let Some((_, handler)) = self
            .handlers
            .iter()
            .find(|(operation, _)| *operation == invocation.ctx.operation)
        else {
            return Err(DispatchFailure::unregistered_handler(format!(
                "no local handler for {}",
                invocation.ctx.operation
            )));
        };
        handler(&invocation)
    }
}

impl Default for BrokerEnvelope {
    /// An envelope with no committed rows and no handlers.
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            committed: BTreeSet::new(),
            profile: BrokerProfileId::Host,
            dispatcher: Box::new(HandlerTable::new()),
            invocations: AtomicU64::new(1),
        }
    }
}

/// The dispatcher of the broker's own process.
///
/// The broker links no provider crate, so a family row's handler runs in the
/// declaring crate's process and this dispatcher forwards to it; the
/// forwarder is the peer-mediated leg, and a row whose peer has not
/// registered is refused rather than served by the wrong process - a handler
/// table is never authority, and a missing handler is never a silent success.
pub struct ForwardingDispatcher {
    forwarder: Box<dyn crate::forwarding::OperationForwarder>,
}

impl std::fmt::Debug for ForwardingDispatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ForwardingDispatcher")
            .finish_non_exhaustive()
    }
}

impl ForwardingDispatcher {
    /// Forward through one peer-mediated leg.
    pub fn new(forwarder: impl crate::forwarding::OperationForwarder) -> Self {
        Self {
            forwarder: Box::new(forwarder),
        }
    }
}

impl Default for ForwardingDispatcher {
    /// A forwarder whose peer serves nothing, so every forwarded operation
    /// is refused until the broker is given a peer.
    fn default() -> Self {
        Self::new(crate::forwarding::UnroutedForwarder)
    }
}

impl OperationDispatcher for ForwardingDispatcher {
    fn dispatch(
        &self,
        invocation: DirectInvocation<'_>,
    ) -> Result<DispatchOutcome, DispatchFailure> {
        self.forwarder.forward(crate::forwarding::ForwardedOperation {
            operation: invocation.ctx.operation,
            zone: invocation.ctx.zone,
            invocation_id: invocation.ctx.invocation_id,
            payload: invocation.payload,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{BrokerAuthzFacets, OperationOwner};
    use crate::forwarding::SocketForwarder;
    use std::io;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The declaring process a test broker forwards to.
    ///
    /// The loopback listens on a real socket and answers from a real
    /// [`OperationDispatcher`], so a forwarded invocation that never crossed
    /// the socket - or that arrived with a different operation or zone -
    /// cannot produce the caller's result.
    struct LoopbackPeer {
        path: PathBuf,
        calls: Arc<AtomicUsize>,
        _dir: tempfile::TempDir,
    }

    impl LoopbackPeer {
        fn calls(&self) -> usize {
            self.calls.load(Ordering::Acquire)
        }

        fn forwarder(&self) -> SocketForwarder {
            SocketForwarder::new(self.path.clone())
        }
    }

    fn loopback_peer(dispatcher: impl OperationDispatcher + 'static) -> LoopbackPeer {
        let dir = tempfile::tempdir().expect("peer socket dir");
        let path = dir.path().join("forward.sock");
        let listener = crate::protocol::bind_seqpacket(&path).expect("bind peer socket");
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = Arc::clone(&calls);
        let dispatcher: Box<dyn OperationDispatcher> = Box::new(dispatcher);
        std::thread::spawn(move || {
            while let Ok(fd) = accept_peer(&listener) {
                let Some(request) = crate::protocol::recv_json_frame::<
                    d2b_contracts_broker::broker_wire::ForwardOperationRequest,
                >(fd.as_raw_fd())
                .expect("read forwarded call") else {
                    continue;
                };
                observed.fetch_add(1, Ordering::AcqRel);
                let response = answer_from(
                    dispatcher.as_ref(),
                    &request.operation,
                    &request.zone,
                    &request.invocation_id,
                    request.payload,
                );
                crate::protocol::send_json_frame(fd.as_raw_fd(), &response)
                    .expect("write forward reply");
            }
        });
        LoopbackPeer {
            path,
            calls,
            _dir: dir,
        }
    }

    fn answer_from(
        dispatcher: &dyn OperationDispatcher,
        operation: &str,
        zone: &str,
        invocation_id: &str,
        payload: Value,
    ) -> d2b_contracts_broker::broker_wire::ForwardOperationResponse {
        use d2b_contracts_broker::broker_wire::{
            ForwardOperationOutcome, ForwardOperationResponse,
        };
        let payload: CanonicalJsonObject =
            serde_json::from_value(payload).expect("canonical payload");
        let outcome = match dispatcher.dispatch(DirectInvocation {
            ctx: InvocationCtx {
                operation,
                zone,
                invocation_id,
            },
            payload: &payload,
        }) {
            Ok(DispatchOutcome { result }) => ForwardOperationOutcome::Result {
                result: serde_json::to_value(&result).expect("render result"),
            },
            Err(failure) => ForwardOperationOutcome::Refused {
                code: failure.code,
            },
        };
        ForwardOperationResponse { outcome }
    }

    fn accept_peer(listener: &OwnedFd) -> io::Result<OwnedFd> {
        nix::sys::socket::accept4(listener.as_raw_fd(), nix::sys::socket::SockFlag::empty())
            .map(crate::sys::owned_fd_from_raw)
            .map_err(|errno| io::Error::from_raw_os_error(errno as i32))
    }

    /// A row a test declares: a broker-generic operation the committed
    /// catalog does not carry, reached by name and never by a wire variant.
    fn declared_row(
        operation: &'static str,
        fields: &'static [&'static str],
        required: &'static [&'static str],
        groups: &'static [&'static str],
    ) -> BrokerOperationRow {
        BrokerOperationRow {
            operation,
            wire_variant: None,
            owner: OperationOwner::BrokerGeneric,
            family: None,
            declaring_provider: None,
            profiles: &[BrokerProfileId::Host],
            w3: false,
            capabilities: false,
            disposition: "promoted-live",
            stub_target: None,
            audit_fields: &[],
            authz: BrokerAuthzFacets {
                subject: "test",
                scope: "per-zone",
                allowed_groups: groups,
                destructive: false,
                secret_access: "None",
                broker_required: "Yes",
                audit_mode: "yes",
            },
            payload_provenance: PayloadProvenance::Request,
            payload_fields: fields,
            payload_required: required,
            audit_join: None,
        }
    }

    fn echo_table() -> HandlerTable {
        HandlerTable::new().with("ProbeOperation", |invocation| {
            Ok(DispatchOutcome {
                result: serde_json::from_value(serde_json::json!({
                    "operation": invocation.ctx.operation,
                    "invocation": invocation.ctx.invocation_id,
                    "zone": invocation.ctx.zone,
                    "fields": invocation.payload.len(),
                }))
                .expect("canonical result"),
            })
        })
    }

    fn probe_envelope() -> BrokerEnvelope {
        BrokerEnvelope::over(BrokerProfileId::Host, Box::new(echo_table()))
            .commit_broker_generic()
            .declare(declared_row(
                "ProbeOperation",
                &["label"],
                &["label"],
                &["d2bd"],
            ))
            .build()
    }

    #[test]
    fn an_unknown_operation_is_refused() {
        let envelope = probe_envelope();
        let refusal = envelope
            .call(
                CallerAuthority::Daemon,
                "NoSuchOperation",
                "zone-a",
                &serde_json::json!({}),
            )
            .expect_err("an unknown operation is refused");
        assert_eq!(refusal.code, UNKNOWN_OPERATION);
        assert_eq!(refusal.operation, "NoSuchOperation");
    }

    #[test]
    fn an_uncommitted_row_is_refused() {
        let envelope = probe_envelope();
        // The committed catalog declares the operation; this broker does not
        // serve it, and deny-by-default refuses it rather than guessing.
        let refusal = envelope
            .call(
                CallerAuthority::Daemon,
                "ApplySysctl",
                "zone-a",
                &serde_json::json!({}),
            )
            .expect_err("an uncommitted row is refused");
        assert_eq!(refusal.code, UNCOMMITTED_OPERATION);
        assert_eq!(refusal.operation, "ApplySysctl");
    }

    #[test]
    fn an_ungranted_caller_is_refused() {
        let envelope = probe_envelope();
        let refusal = envelope
            .call(
                CallerAuthority::Unauthorized,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            )
            .expect_err("an ungranted caller is refused");
        assert_eq!(refusal.code, UNGRANTED_CALLER);
        assert_eq!(refusal.operation, "ProbeOperation");
    }

    #[test]
    fn a_wire_inherited_operation_is_refused() {
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(echo_table()))
            .commit_all()
            .build();
        let refusal = envelope
            .call(
                CallerAuthority::Daemon,
                "ApplySysctl",
                "zone-a",
                &serde_json::json!({}),
            )
            .expect_err("a wire-inherited operation is not carried generically");
        assert_eq!(refusal.code, WIRE_INHERITED_OPERATION);
    }

    #[test]
    fn a_payload_outside_the_row_schema_is_refused() {
        let envelope = probe_envelope();
        let undeclared = envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x", "extra": 1 }),
            )
            .expect_err("an undeclared field is refused");
        assert_eq!(undeclared.code, INVALID_PAYLOAD);
        let missing = envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({}),
            )
            .expect_err("a missing required field is refused");
        assert_eq!(missing.code, INVALID_PAYLOAD);
    }

    #[test]
    fn an_unregistered_handler_is_refused() {
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(HandlerTable::new()))
            .declare(declared_row(
                "ProbeOperation",
                &["label"],
                &["label"],
                &["d2bd"],
            ))
            .build();
        let refusal = envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            )
            .expect_err("a row with no handler is refused");
        assert_eq!(refusal.code, UNREGISTERED_HANDLER);
    }

    #[test]
    fn a_declared_operation_dispatches_with_an_invocation_id() {
        let envelope = probe_envelope();
        let invocation = envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            )
            .expect("a declared, granted operation dispatches");
        let invocation_id = invocation.invocation_id;
        assert!(invocation_id.starts_with("invocation-"));
        let result = serde_json::to_value(&invocation.outcome.result).expect("result serializes");
        assert_eq!(result["operation"], "ProbeOperation");
        assert_eq!(result["invocation"], invocation_id);
        assert_eq!(result["zone"], "zone-a");
        assert_eq!(result["fields"], 1);
    }

    #[test]
    fn a_refusal_carries_the_invocation_identifier_it_denied() {
        let envelope = probe_envelope();
        let refusal = envelope
            .call(
                CallerAuthority::Unauthorized,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            )
            .expect_err("an ungranted caller is refused");
        assert!(refusal.invocation_id.starts_with("invocation-"));
        let fields = refusal.audit_fields();
        assert_eq!(fields["invocation_id"], refusal.invocation_id);
        assert_eq!(fields["operation"], "ProbeOperation");
        assert_eq!(fields["reason"], UNGRANTED_CALLER);
        // Two refusals are two named invocations, not one anonymous denial.
        let other = envelope
            .call(
                CallerAuthority::Unauthorized,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            )
            .expect_err("an ungranted caller is refused");
        assert_ne!(refusal.invocation_id, other.invocation_id);
    }

    #[test]
    fn a_row_join_keys_the_invocation_on_its_declared_fields() {
        let mut row = declared_row("ProbeOperation", &["label", "kind"], &["label", "kind"], &["d2bd"]);
        row.audit_join = Some(&["kind", "label"]);
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(echo_table()))
            .declare(row)
            .build();
        let joined = |kind: &str| {
            envelope
                .call(
                    CallerAuthority::Daemon,
                    "ProbeOperation",
                    "zone-a",
                    &serde_json::json!({ "label": "x", "kind": kind }),
                )
                .expect("a declared, granted operation dispatches")
                .audit_join_identity
                .expect("a row that declares a join carries one")
        };
        let first = joined("alpha");
        assert_ne!(first, joined("beta"), "a different declared key is another identity");
        assert_eq!(first, joined("alpha"), "the same declared key is one identity");
        // The key is the digest of the declared fields, never the fields.
        assert!(first.starts_with("sha256:"));
        assert!(!first.contains("alpha"));
        // A row that declares no join carries none.
        let envelope = probe_envelope();
        let plain = envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            )
            .expect("a declared, granted operation dispatches");
        assert_eq!(plain.audit_join_identity, None);
    }

    #[test]
    fn a_launcher_does_not_carry_the_daemon_grant() {
        let daemon_only = declared_row("DaemonOperation", &[], &[], &["d2bd"]);
        assert!(BrokerEnvelope::granted(&daemon_only, CallerAuthority::Daemon));
        assert!(
            !BrokerEnvelope::granted(&daemon_only, CallerAuthority::Launcher),
            "a row granted to the daemon alone must refuse a launcher"
        );
        let launcher = declared_row("LauncherOperation", &[], &[], &["d2b-launcher"]);
        assert!(BrokerEnvelope::granted(&launcher, CallerAuthority::Launcher));
        assert!(!BrokerEnvelope::granted(&launcher, CallerAuthority::Daemon));
    }

    #[test]
    fn a_grant_admits_only_the_authority_it_names() {
        let admin = declared_row("AdminOperation", &[], &[], &["d2b-admin"]);
        assert!(BrokerEnvelope::granted(&admin, CallerAuthority::Admin));
        assert!(!BrokerEnvelope::granted(&admin, CallerAuthority::Launcher));
        assert!(!BrokerEnvelope::granted(
            &admin,
            CallerAuthority::Unauthorized
        ));
        let daemon = declared_row("DaemonOperation", &[], &[], &["d2bd"]);
        assert!(BrokerEnvelope::granted(&daemon, CallerAuthority::Daemon));
        assert!(BrokerEnvelope::granted(&daemon, CallerAuthority::Admin));
        let ungranted = declared_row("NoGroup", &[], &[], &[]);
        assert!(!BrokerEnvelope::granted(&ungranted, CallerAuthority::Admin));
    }

    #[test]
    fn a_forwarded_invocation_reaches_the_peer_and_returns_its_result() {
        // The broker links no provider crate: a committed row's handler runs
        // in the declaring process, so the dispatch step crosses to the peer
        // that serves it and comes back with the peer's result.
        let peer = loopback_peer(echo_table());
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .declare(declared_row(
            "ProbeOperation",
            &["label"],
            &["label"],
            &["d2bd"],
        ))
        .build();
        let invocation = envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            )
            .expect("the peer served the forwarded invocation");
        assert_eq!(peer.calls(), 1, "the call must have crossed the socket");
        let rendered = String::from_utf8(invocation.outcome.result.to_canonical_bytes())
            .expect("canonical json is utf-8");
        assert!(rendered.contains("\"operation\":\"ProbeOperation\""), "{rendered}");
        assert!(rendered.contains("\"zone\":\"zone-a\""), "{rendered}");
        assert!(rendered.contains("\"fields\":1"), "{rendered}");
    }

    #[test]
    fn a_forwarded_row_whose_peer_serves_nothing_refuses() {
        // The peer is reachable but registered no handler for the row, so the
        // invocation must refuse rather than succeed with the wrong process's
        // answer.
        let peer = loopback_peer(HandlerTable::new());
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .declare(declared_row(
            "ProbeOperation",
            &["label"],
            &["label"],
            &["d2bd"],
        ))
        .build();
        let refusal = envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            )
            .expect_err("a row no peer serves is refused");
        assert_eq!(refusal.code, UNREGISTERED_HANDLER);
        assert_eq!(peer.calls(), 1, "the call must have crossed the socket");
    }

    #[test]
    fn an_unwired_broker_refuses_every_forwarded_row() {
        // The fail-closed default: no peer configured means no forwarded row
        // is served, named as the missing handler rather than a local guess.
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::default()),
        )
        .declare(declared_row(
            "ProbeOperation",
            &["label"],
            &["label"],
            &["d2bd"],
        ))
        .build();
        let refusal = envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            )
            .expect_err("no forwarding peer is wired");
        assert_eq!(refusal.code, UNREGISTERED_HANDLER);
    }

    #[test]
    fn the_live_set_commits_every_row_a_declaring_process_owns() {
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::default()),
        )
        .commit_forwarded()
        .build();
        let committed: BTreeSet<&str> = envelope
            .committed_rows()
            .map(|row| row.operation)
            .collect();
        for row in BROKER_OPERATION_CATALOG.iter().filter(|row| {
            row.owner == OperationOwner::Family && row.declaring_provider.is_some()
        }) {
            assert!(
                committed.contains(row.operation),
                "{} names a declaring process and must be committed",
                row.operation
            );
        }
        // A row that names no declaring process has nowhere to be forwarded,
        // so committing it would turn a wiring gap into the caller's fault.
        for row in BROKER_OPERATION_CATALOG
            .iter()
            .filter(|row| row.declaring_provider.is_none())
        {
            assert!(
                !committed.contains(row.operation) || row.owner == OperationOwner::BrokerGeneric,
                "{} names no declaring process and must not be committed",
                row.operation
            );
        }
    }
}
