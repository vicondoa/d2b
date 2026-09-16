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

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use d2b_audit::evidence_chain::{
    ChainAuditSink, ChainLeg, ChainOutcome, ChainRecord, ChainRecordClass, EvidenceChain,
    MAX_NESTED_DEPTH,
};
use d2b_contracts_broker::broker_wire::{
    BrokerCallerRole, DEFAULT_CONTEXT_DEADLINE_MS, FdKind, ForwardContext, MAX_FRAME_FDS,
    PublishTrustedContextValues, PublishTrustedContextResponse,
};
use d2b_contracts_resource::v3::CanonicalJsonObject;
use serde_json::Value;
use std::os::fd::{AsRawFd, OwnedFd};

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

/// The refusal code for a request or response whose forward-carrier fd
/// attachments disagree with their declarations, or exceed the bounded
/// ceiling.
 ///
/// The code is the shared carrier code (`d2b_contracts_broker::broker_wire::
/// FD_LEG`), declared once beside the wire shapes both legs carry.
pub const FD_LEG: &str = d2b_contracts_broker::broker_wire::FD_LEG;

/// The refusal code for a broker-attested context the envelope cannot mint
/// or the peer cannot validate.
///
/// The code is the shared carrier code (`d2b_contracts_broker::broker_wire::
/// STALE_CONTEXT`): the broker refuses with it when it holds no published
/// values for the Zone a call names (minting is impossible until the daemon
/// publishes), and the rendezvous refuses with it when a context's epoch,
/// Zone, revision, or generations do not match its own current values.
pub const STALE_CONTEXT: &str = d2b_contracts_broker::broker_wire::STALE_CONTEXT;

/// The refusal code for a handler that deliberately refused its invocation.
///
/// The peer codes are the closed dispatch-failure vocabulary (KTD7): a
/// failure a handler chose, a failure a handler hit, a budget a handler
/// overran, and a crash a handler died in each keep their own code when
/// they surface from a dispatch, never flattened into the missing-handler
/// refusal.
pub const HANDLER_REFUSED: &str = "handler-refused";

/// The refusal code for a handler that ran and failed.
pub const HANDLER_ERRORED: &str = "handler-errored";

/// The refusal code for a handler that did not finish within its effective
/// budget.
///
/// The budget is the broker-attested context deadline when the call carries
/// one, and the envelope or table default otherwise; the dispatch aborts the
/// handler task at expiry, so a non-yielding handler is refused by name
/// rather than served forever.
pub const HANDLER_TIMED_OUT: &str = "handler-timed-out";

/// The refusal code for a handler that panicked mid-dispatch.
///
/// The dispatch wraps the handler so the crash is a named refusal with the
/// panic's message as detail, never a panic escaping into the caller's
/// socket path.
pub const HANDLER_CRASHED: &str = "handler-crashed";

/// The refusal code for a wire variant a peer's Hello-negotiated wire
/// version does not admit.
///
/// The code is this envelope's stable entry for the taxonomy's stale-wire
/// half (KTD7): the gate that refuses a straggler peer's call with it before
/// dispatch - rather than a pre-dispatch malformed-wire drop - lands with
/// the wire-version gate (U4 item 4); this file carries the code and its
/// closed-set entry alone.
pub const STALE_WIRE_VERSION: &str = "stale-wire-version";

/// The refusal code for a nested call whose evidence chain exceeds the
/// depth cap.
///
/// The code is the shared chain code (KTD6): a call loop trips this
/// dedicated loop-refusal code instead of growing its chain without bound,
/// and both execution legs refuse with the same spelling.
pub const NESTED_DEPTH_EXCEEDED: &str = d2b_audit::evidence_chain::NESTED_DEPTH_EXCEEDED;

/// The closed set of codes the envelope itself refuses with.
///
/// The set is closed: the envelope codes above the dispatch failures that
/// can name a refusal, the carrier's fd-leg and stale-context codes, the
/// peer codes a dispatch failure can carry (KTD7), the stale-wire-version
/// code a retiring gate will refuse with, and the nested-call depth-cap
/// code (KTD6). A code outside the set is never surfaced as the
/// caller-visible code; it rides in the refusal's detail.
pub const ENVELOPE_REFUSALS: [&str; 14] = [
    UNKNOWN_OPERATION,
    UNCOMMITTED_OPERATION,
    UNGRANTED_CALLER,
    WIRE_INHERITED_OPERATION,
    INVALID_PAYLOAD,
    UNREGISTERED_HANDLER,
    FD_LEG,
    STALE_CONTEXT,
    HANDLER_REFUSED,
    HANDLER_ERRORED,
    HANDLER_TIMED_OUT,
    HANDLER_CRASHED,
    STALE_WIRE_VERSION,
    NESTED_DEPTH_EXCEEDED,
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
    /// A provider's own authority: the provider-class name the committed
    /// grants are written in.
    ///
    /// A provider-originated call (a handler's nested leg presenting its
    /// provider's identity) is covered by the committed rows that grant
    /// that provider by name - it never re-presents as the daemon class
    /// (KTD6).
    Provider(&'static str),
    /// A caller no committed grant admits.
    Unauthorized,
}

impl CallerAuthority {
    /// The authority classes the caller carries, in the vocabulary the
    /// committed grants are written in.
    ///
    /// Each class carries exactly the grants its class implies: a launcher
    /// is not the daemon, so a row granted to `d2bd` alone refuses it rather
    /// than admitting it through the daemon's class. A provider class is
    /// exactly the name the row grants.
    pub fn classes(self) -> BTreeSet<&'static str> {
        match self {
            Self::Daemon => BTreeSet::from(["d2bd"]),
            Self::Admin => BTreeSet::from(["d2bd", "d2b-admin"]),
            Self::Launcher => BTreeSet::from(["d2b-launcher"]),
            Self::Provider(provider) => BTreeSet::from([provider]),
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

    /// Classify one attested identity back to the authority class the
    /// broker could have attested it under.
    ///
    /// The broker-class identities map back to their classes; a provider
    /// identity is not a broker class, so it classifies to `None` and is
    /// checked by name against the committed grants instead (the identity
    /// is the provider-class name the row grants, KTD6).
    pub fn classify_identity(identity: &str) -> Option<Self> {
        match identity {
            "daemon" => Some(Self::Daemon),
            "admin" => Some(Self::Admin),
            "launcher" => Some(Self::Launcher),
            "unauthorized" => Some(Self::Unauthorized),
            _ => None,
        }
    }

    /// The initiating identity a broker-minted context carries for this
    /// caller: the stable class name the attestation records.
    ///
    /// The identity is the broker's own classification, never a caller-
    /// supplied spelling, so a context cannot re-present a caller as a
    /// class it was not authenticated under.
    pub fn identity(self) -> &'static str {
        match self {
            Self::Daemon => "daemon",
            Self::Admin => "admin",
            Self::Launcher => "launcher",
            Self::Provider(provider) => provider,
            Self::Unauthorized => "unauthorized",
        }
    }
}

/// One Zone's daemon-published attestation values, as the broker cached
/// them.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct ZoneAttestation {
    provider_set_revision: u64,
    controller_generation: u64,
    guest_generation: u64,
}

/// The durable shape of the broker's trusted-context state.
///
/// One file under the broker state root, persisted atomically (temp file +
/// rename + fsync) exactly like the broker's other durable records, so a
/// crash never leaves a half-written epoch or a half-applied publication.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct PersistedTrustedContext {
    /// The broker-epoch nonce this store currently mints with.
    epoch: u64,
    /// The daemon-published values the broker holds, per Zone.
    zones: BTreeMap<String, ZoneAttestation>,
}

/// The failure of one trusted-context store operation.
#[derive(Debug)]
pub enum TrustedContextStoreError {
    /// A publication that would move the cached values backwards.
    StaleFreshness(&'static str),
    /// The store could not open, persist, or read its durable state.
    Io { detail: String },
    /// The durable state file is not the store's own shape.
    Corrupt { detail: String },
}

impl std::fmt::Display for TrustedContextStoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleFreshness(code) => write!(formatter, "stale freshness: {code}"),
            Self::Io { detail } => write!(formatter, "trusted-context store I/O: {detail}"),
            Self::Corrupt { detail } => {
                write!(formatter, "trusted-context store corrupt: {detail}")
            }
        }
    }
}

impl std::error::Error for TrustedContextStoreError {}

impl TrustedContextStoreError {
    /// The closed stale-context code a freshness refusal surfaces under.
    pub fn code(&self) -> &'static str {
        match self {
            Self::StaleFreshness(code) => code,
            Self::Io { .. } | Self::Corrupt { .. } => STALE_CONTEXT,
        }
    }
}

/// The broker-held cache the daemon's publications fill and the envelope
/// mints contexts from.
///
/// The broker is the sole minter of the carrier's context block, and it
/// mints from values the daemon owns: the daemon publishes its current
/// provider-set revision and controller/guest generations over the
/// established origination leg, and this store caches them as durable,
/// monotonically increasing broker state.
///
/// Two properties are structural. The store refuses to mint until the
/// daemon has published the Zone a call names - a call into a Zone the
/// broker holds no values for is refused with the stale-context code rather
/// than attested blind. And the broker-epoch nonce is a durable counter a
/// fresh store instance strictly increments on open, so a broker restart is
/// a fresh nonce: every context minted before the restart fails the
/// rendezvous's epoch check regardless of generation equality, and no
/// previously minted context can be re-minted into validity, because the
/// old context still carries the old epoch.
pub struct TrustedContextStore {
    root: PathBuf,
    state: Mutex<PersistedTrustedContext>,
}

impl TrustedContextStore {
    /// The directory name under the store root the durable state lives in.
    const STATE_DIR: &'static str = "trusted-context";

    /// Open the store under `root`, claiming a fresh broker epoch.
    ///
    /// The epoch is loaded from the durable state when one exists and
    /// strictly incremented before anything mints, so no two broker
    /// instances ever mint under one epoch; the fresh epoch is persisted
    /// before the store is usable. The daemon's last-published values are
    /// loaded with it, so a restarting broker still holds the values it
    /// published for while minting under a nonce no prior context carries.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self, TrustedContextStoreError> {
        let root = root.into();
        let directory = root.join(Self::STATE_DIR);
        fs::create_dir_all(&directory).map_err(|error| TrustedContextStoreError::Io {
            detail: format!("create {}: {error}", directory.display()),
        })?;
        let path = directory.join("state.json");
        let mut state = if path.exists() {
            let bytes = fs::read(&path).map_err(|error| TrustedContextStoreError::Io {
                detail: format!("read {}: {error}", path.display()),
            })?;
            serde_json::from_slice(&bytes).map_err(|error| TrustedContextStoreError::Corrupt {
                detail: format!("{}: {error}", path.display()),
            })?
        } else {
            PersistedTrustedContext {
                epoch: 0,
                zones: BTreeMap::new(),
            }
        };
        // A fresh instance is a fresh attestation lineage: strictly bump the
        // durable counter before the store can mint, and persist the bump so
        // even a crash before the first mint cannot make the next instance
        // reuse this epoch.
        state.epoch = state.epoch.saturating_add(1);
        Self::persist(&path, &state)?;
        Ok(Self {
            root,
            state: Mutex::new(state),
        })
    }

    /// The epoch this store is currently minting with.
    pub fn epoch(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .epoch
    }

    /// Whether the broker holds published values for one Zone.
    ///
    /// The fail-closed side of the attestation: a Zone the daemon has not
    /// published is a Zone the broker cannot attest, so the envelope refuses
    /// to mint for it rather than attest blind.
    pub fn holds_zone(&self, zone: &str) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .zones
            .contains_key(zone)
    }

    /// Cache one daemon publication, monotonically.
    ///
    /// The cache is monotonically increasing: a publication for a Zone the
    /// broker already holds values for must not move any of the three values
    /// backwards - a rollback means the daemon is re-publishing older state
    /// over newer, and the publication is refused with the stale-context
    /// code rather than attested. Returns the epoch the broker is currently
    /// minting with, so the daemon's receiving leg learns the nonce every
    /// context it validates must carry.
    pub fn publish(
        &self,
        values: &PublishTrustedContextValues,
    ) -> Result<u64, TrustedContextStoreError> {
        let mut state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(existing) = state.zones.get(&values.zone)
            && (existing.provider_set_revision > values.provider_set_revision
                || existing.controller_generation > values.controller_generation
                || existing.guest_generation > values.guest_generation)
        {
            return Err(TrustedContextStoreError::StaleFreshness(STALE_CONTEXT));
        }
        let inserted = ZoneAttestation {
            provider_set_revision: values.provider_set_revision,
            controller_generation: values.controller_generation,
            guest_generation: values.guest_generation,
        };
        let changed = state.zones.get(&values.zone) != Some(&inserted);
        if changed {
            state.zones.insert(values.zone.clone(), inserted);
            self.persist_locked(&state)?;
        }
        Ok(state.epoch)
    }

    /// Mint one context block for a Zone the broker holds values for.
    ///
    /// The envelope's mint call: refuses with the stale-context code until
    /// the daemon has published the Zone, then attests the Zone's cached
    /// revision and generations under the store's current epoch with the
    /// broker's own classification of the caller and the operation's
    /// deadline budget.
    pub fn mint(
        &self,
        zone: &str,
        initiating_identity: &str,
        deadline_ms: u64,
    ) -> Result<ForwardContext, &'static str> {
        let state = self.state.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(attestation) = state.zones.get(zone) else {
            // Nothing published for this Zone: the broker cannot attest a
            // call it holds no values for, so the call is refused with the
            // dedicated code - the same code the receiving leg refuses a
            // stale or mismatched context with.
            return Err(STALE_CONTEXT);
        };
        Ok(ForwardContext {
            broker_epoch: state.epoch,
            zone: zone.to_owned(),
            provider_set_revision: attestation.provider_set_revision,
            controller_generation: attestation.controller_generation,
            guest_generation: attestation.guest_generation,
            initiating_identity: initiating_identity.to_owned(),
            deadline_ms,
        })
    }

    /// A publication ack a daemon-side receiver hands back to its caller.
    ///
    /// The wire shape is the acknowledgement the daemon reads its epoch
    /// from; production dispatch routes the publish variant through this
    /// same method, so the wire and the store never disagree.
    pub fn publication_reply(&self, values: &PublishTrustedContextValues) -> Result<PublishTrustedContextResponse, TrustedContextStoreError> {
        let epoch = self.publish(values)?;
        Ok(PublishTrustedContextResponse {
            broker_epoch: epoch,
        })
    }

    fn persist_locked(&self, state: &PersistedTrustedContext) -> Result<(), TrustedContextStoreError> {
        Self::persist(&self.root.join(Self::STATE_DIR).join("state.json"), state)
    }

    fn persist(
        path: &Path,
        state: &PersistedTrustedContext,
    ) -> Result<(), TrustedContextStoreError> {
        let bytes = serde_json::to_vec(state).map_err(|error| TrustedContextStoreError::Io {
            detail: format!("serialize {}: {error}", path.display()),
        })?;
        let tmp = path.with_extension("json.tmp");
        {
            let mut file = fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(&tmp)
                .map_err(|error| TrustedContextStoreError::Io {
                    detail: format!("open {}: {error}", tmp.display()),
                })?;
            file.write_all(&bytes)
                .map_err(|error| TrustedContextStoreError::Io {
                    detail: format!("write {}: {error}", tmp.display()),
                })?;
            file.sync_all()
                .map_err(|error| TrustedContextStoreError::Io {
                    detail: format!("sync {}: {error}", tmp.display()),
                })?;
        }
        fs::rename(&tmp, path).map_err(|error| TrustedContextStoreError::Io {
            detail: format!("rename {} -> {}: {error}", tmp.display(), path.display()),
        })?;
        if let Some(parent) = path.parent() {
            fs::File::open(parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| TrustedContextStoreError::Io {
                    detail: format!("sync {}: {error}", parent.display()),
                })?;
        }
        Ok(())
    }
}

/// The broker process's trusted-context store.
///
/// The store is process-lifetime state opened once under the daemon state
/// root before the accept loop starts, exactly like the state-cell store:
/// dispatch routes the publish wire variant through it, and the envelope
/// would mint from it. `run_server` is the only production writer; tests
/// initialize it against a scratch root.
static TRUSTED_CONTEXT_STORE: std::sync::OnceLock<TrustedContextStore> = std::sync::OnceLock::new();

/// Open the process's trusted-context store under the daemon state root.
///
/// Called once in `run_server` before the broker serves; a store that fails
/// to open fails the broker closed at startup rather than attesting or
/// caching under a half-open state.
pub(crate) fn init_trusted_context_store(
    state_dir: &Path,
) -> Result<(), TrustedContextStoreError> {
    let store = TrustedContextStore::open(state_dir)?;
    let _ = TRUSTED_CONTEXT_STORE.set(store);
    Ok(())
}

/// The broker process's trusted-context store, absent until
/// [`init_trusted_context_store`] runs.
pub(crate) fn trusted_context_store() -> Option<&'static TrustedContextStore> {
    TRUSTED_CONTEXT_STORE.get()
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
    /// The evidence chain this leg runs under: the root invocation id and
    /// the ordered identities, root first.
    ///
    /// A handler that calls another provider's service presents the chain
    /// with its own invoking identity appended ([`EvidenceChain::nested`]),
    /// never re-presenting as the daemon class (KTD6).
    pub chain: &'a EvidenceChain,
    /// Whether this leg is a nested call under an existing invocation
    /// (U10, KTD6).
    ///
    /// A nested call presents a chain even when the chain carries a single
    /// identity (the initiating principal alone), so the record class is
    /// decided by this flag rather than by the chain's depth: the leg
    /// executing the root operation writes exactly one root record per
    /// root invocation, and every nested leg writes a correlation record
    /// keyed by the root invocation id and its depth.
    pub nested: bool,
}

/// The result of one dispatched invocation.
#[derive(Debug)]
pub struct DispatchOutcome {
    /// The canonical result payload.
    pub result: CanonicalJsonObject,
    /// The descriptors the answering peer minted this invocation,when the
    /// operation's result carries any.where
    ///
    /// Formal fd provenance tracking is the answering peer's job (KTD7):the
    /// carrier only refuses a descriptor that is one of the call's own attached
    /// fds, never a fresh mint. The caller owns the returned descriptors;the
    /// forwarder closes them on any refusal.


    pub fds: Vec<OwnedFd>,
}

/// Why one dispatch produced no result.
///
/// The cases stay apart because the envelope reports them differently: an
/// operation no handler serves is the fail-closed [`UNREGISTERED_HANDLER`]
/// refusal, while a failure a handler or peer decided carries its own code
/// from the closed peer set (KTD7) and must never be reported as a missing
/// handler.
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
#[derive(Debug)]
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
    /// The broker-attested context block the envelope minted for this
    /// invocation, when the broker holds a context store. A local handler
    /// sees the same block the forward carrier would carry.
    pub context: Option<&'a ForwardContext>,
    /// The descriptors the caller attached to this invocation,when any.
    /// The caller owns them;the invocation borrows them for its duration.
    pub fds: &'a [OwnedFd],

    /// The kernel kind the row's fd facet declares,when it declares one.
    pub fd_kind: Option<FdKind>,
}

/// The handler-table seam of one committed operation.
///
/// The broker holds the committed rows; the handler code lives in the
/// declaring crate's process. An implementation either serves the operation
/// locally or forwards it, and refuses an operation it was not given a
/// handler for - never serving a row it does not implement.
/// The future one dispatch completes on.
///
/// The envelope holds its dispatcher as a trait object, so the future is
/// boxed rather than an associated type: a handler that answers locally can
/// hand back a ready future, and a forwarder hands back the peer leg's.
pub type DispatchFuture<'a> = std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<DispatchOutcome, DispatchFailure>> + Send + 'a>,
>;

pub trait OperationDispatcher: Send + Sync {
    /// Run one validated, authorized invocation.
    ///
    /// The dispatch is async because one of its two legs is: a family row's
    /// handler runs in the declaring process, and the call crosses to it over
    /// an async dial bounded by the forwarder's own round-trip budget.
    fn dispatch<'a>(&'a self, invocation: DirectInvocation<'a>) -> DispatchFuture<'a>;

    /// Whether this dispatcher executes handlers inside the broker's own
    /// process (the in-broker leg) rather than handing every dispatch to
    /// the peer process (the forwarded leg).
    ///
    /// The audit rule (KTD6) reads this to decide which side records a
    /// leg: in-broker executions audit broker-side only, forwarded ops
    /// audit daemon-side only, never both.
    fn serves_locally(&self) -> bool {
        false
    }

    /// Whether this dispatcher executes ONE operation inside the broker's
    /// own process.
    ///
    /// A mixed dispatcher (the U10 kernel seam) serves some operations
    /// in-broker and forwards the rest: the per-operation answer decides
    /// which side owns the leg's audit record for THAT operation, so a
    /// single envelope over both legs never double-records (KTD6). The
    /// default is the dispatcher-wide [`Self::serves_locally`] answer.
    fn serves_operation(&self, operation: &str) -> bool {
        let _ = operation;
        self.serves_locally()
    }
}

/// The broker-side operation envelope.
pub struct BrokerEnvelope {
    rows: Vec<BrokerOperationRow>,
    committed: BTreeSet<&'static str>,
    profile: BrokerProfileId,
    dispatcher: Box<dyn OperationDispatcher>,
    invocations: AtomicU64,
    /// The broker-held attestation cache, when the broker attests calls.
    ///
    /// Absent, the envelope runs the context-free carrier: no context block
    /// is minted and the forwarded request crosses as the pre-attestation
    /// shape. Present, every call is minted a broker-attested context block
    /// before dispatch, and a call the store cannot attest is refused with
    /// the stale-context code.
    context_store: Option<Arc<TrustedContextStore>>,
    /// The sink the envelope's broker-side leg writes evidence-chain audit
    /// records to, when one is wired.
    ///
    /// Absent, the envelope serves without chain records (the seam's
    /// unwired state; the production broker wires its daily audit log at
    /// its composition point). The audit rule (KTD6): the leg executing
    /// the root operation writes exactly one root record per root
    /// invocation, each nested leg writes a correlation record keyed by
    /// the root invocation id and its depth, and forwarded ops never
    /// produce broker-side records - they are the daemon side's alone.
    chain_audit: Option<Arc<dyn ChainAuditSink>>,
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
            context_store: None,
            chain_audit: None,
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
    /// follow a denied invocation in the audit log. The call is async
    /// because the dispatch step is: a forwarded row's handler runs in the
    /// declaring process and is reached over an async dial.
    pub async fn call(
        &self,
        caller: CallerAuthority,
        operation: &str,
        zone: &str,
        payload: &Value,
    ) -> Result<Invocation, EnvelopeRefusal> {
        self.call_with_fds(caller, operation, zone, payload, &[]).await
    }

    /// Invoke one operation through the envelope with descriptors attached to it.
    ///
    /// The fd leg rides the forward carrier:zero-or-more of the request
    /// frame's SCM_RIGHTS attachments are the operation's descriptors,validated
    /// here against the row's declared fd facet before dispatch,so an
    /// oversized-but-transport-legal set is refused with the fd-leg code
    /// rather than truncated by the transport.where
    pub async fn call_with_fds(
        &self,
        caller: CallerAuthority,
        operation: &str,
        zone: &str,
        payload: &Value,
        fds: &[OwnedFd],
    ) -> Result<Invocation, EnvelopeRefusal> {
        let invocation_id = format!(
            "invocation-{}",
            self.invocations.fetch_add(1, Ordering::AcqRel)
        );
        // The root chain: the broker-minted invocation id and the caller's
        // attested identity. A root call is authorized against the caller's
        // own class; a nested call presents a chain and is authorized
        // against the chain's initiating principal instead (KTD6).
        let chain = EvidenceChain::root(invocation_id, caller.identity().to_owned());
        self.invoke_chain(
            &chain,
            |row| Self::granted(row, caller),
            operation,
            zone,
            payload,
            fds,
            false,
        )
        .await
    }

    /// Invoke one operation as a handler's nested call, presenting the
    /// evidence chain.
    ///
    /// The graft rule (KTD6): a handler may call another provider's service
    /// only when a committed row and its grants cover the call under the
    /// chain's initiating principal - the envelope's authz check applies to
    /// the initiating principal, never the daemon class the handler's
    /// process re-presents. A chain past the depth cap is refused with the
    /// dedicated loop-refusal code before anything else, so a call loop
    /// trips its own code rather than an authz or payload refusal.
    pub async fn call_nested(
        &self,
        chain: EvidenceChain,
        operation: &str,
        zone: &str,
        payload: &Value,
    ) -> Result<Invocation, EnvelopeRefusal> {
        self.call_nested_with_fds(chain, operation, zone, payload, &[])
            .await
    }

    /// Invoke one nested operation with descriptors attached to it.
    pub async fn call_nested_with_fds(
        &self,
        chain: EvidenceChain,
        operation: &str,
        zone: &str,
        payload: &Value,
        fds: &[OwnedFd],
    ) -> Result<Invocation, EnvelopeRefusal> {
        if chain.depth() > MAX_NESTED_DEPTH {
            // The refusing leg still records its correlation record when
            // the broker's own process is the leg (the in-broker leg): the
            // one-record-per-leg invariant covers refusals, so an operator
            // sees the loop trip under the dedicated code. A forwarded
            // leg's record is the daemon side's, never also written here
            // (KTD6).
            let record = ChainRecord {
                ts_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                record_class: ChainRecordClass::Correlation,
                leg: ChainLeg::Broker,
                invocation_id: chain.root_invocation_id().to_owned(),
                depth: chain.depth() as u32,
                initiating_identity: chain.initiating_identity().to_owned(),
                invoking_identity: chain.invoking_identity().to_owned(),
                operation: operation.to_owned(),
                zone: zone.to_owned(),
                outcome: ChainOutcome::Refused,
                code: Some(NESTED_DEPTH_EXCEEDED.to_owned()),
            };
            if self.dispatcher.serves_operation(operation) {
                self.record_chain_outcome(record);
            }
            return Err(EnvelopeRefusal::new(
                chain.root_invocation_id().to_owned(),
                operation,
                NESTED_DEPTH_EXCEEDED,
            ));
        }
        self.invoke_chain(
            &chain,
            |row| Self::chain_granted(row, &chain),
            operation,
            zone,
            payload,
            fds,
            true,
        )
        .await
    }

    /// Run one admission and dispatch for a chain-bearing invocation.
    ///
    /// The admission is the same for a root call and a nested call - resolve
    /// the row, authorize, validate, attest, dispatch - with the one
    /// difference that `granted` decides the authorization: the caller's
    /// own class for a root call, the chain's initiating principal for a
    /// nested one. The whole admission runs in one block so the leg's audit
    /// record is written exactly once per invocation, whatever the outcome
    /// (KTD6).
    async fn invoke_chain(
        &self,
        chain: &EvidenceChain,
        granted: impl Fn(&BrokerOperationRow) -> bool,
        operation: &str,
        zone: &str,
        payload: &Value,
        fds: &[OwnedFd],
        nested: bool,
    ) -> Result<Invocation, EnvelopeRefusal> {
        let probe_start = std::time::Instant::now();
        tracing::info!(
            operation = operation,
            zone = zone,
            nested = nested,
            "envelope invocation start"
        );
        // The audit rule needs the executing leg: the envelope writes
        // broker-side records only for in-broker executions, and nothing
        // for forwarded ops - those are the daemon side's records alone
        // (KTD6). A mixed dispatcher answers per operation, never
        // dispatcher-wide, so one envelope over both legs cannot
        // double-record a forwarded call.
        let serves_locally = self.dispatcher.serves_operation(operation);
        let dispatched: Result<(DispatchOutcome, Option<String>), EnvelopeRefusal> = async {
            let Some(row) = self.rows.iter().find(|row| row.operation == operation) else {
                return Err(EnvelopeRefusal::new(
                    chain.root_invocation_id().to_owned(),
                    operation,
                    UNKNOWN_OPERATION,
                ));
            };
            if !self.committed.contains(row.operation) || !row.admits_profile(self.profile) {
                return Err(EnvelopeRefusal::new(
                    chain.root_invocation_id().to_owned(),
                    operation,
                    UNCOMMITTED_OPERATION,
                ));
            }
            if row.payload_provenance == PayloadProvenance::Wire {
                return Err(EnvelopeRefusal::new(
                    chain.root_invocation_id().to_owned(),
                    operation,
                    WIRE_INHERITED_OPERATION,
                ));
            }
            if !granted(row) {
                return Err(EnvelopeRefusal::new(
                    chain.root_invocation_id().to_owned(),
                    operation,
                    UNGRANTED_CALLER,
                ));
            }
            if !Self::request_fds_admitted(row, fds) {
                // An oversized-but-transport-legal leg (or a kind mismatch, or a
                // row whose declared facet exceeds the frame ceiling) is refused
                // here, so it never reaches the transport where the cmsg buffer would
                // truncate an oversized set anonymously..
                return Err(EnvelopeRefusal::new(
                    chain.root_invocation_id().to_owned(),
                    operation,
                    FD_LEG,
                ));
            }
            let payload = Self::validate(row, payload).map_err(|_| {
                EnvelopeRefusal::new(
                    chain.root_invocation_id().to_owned(),
                    operation,
                    INVALID_PAYLOAD,
                )
            })?;
            // The broker attests the call before dispatch when it holds a
            // context store: the mint names the Zone's published revision and
            // generations under the current broker epoch, the caller's
            // identity - the chain's invoking identity for a nested call, so
            // the attestation never re-presents a handler's call as the
            // daemon class (KTD6) - and the operation's deadline budget.
            // A store that holds no values for the Zone refuses to mint, and
            // the call is refused with the stale-context code rather than
            // attested blind. The budget is the row's declared deadline tier
            // (KTD4): both execution legs serve the minted budget as the
            // per-call handler deadline, so a row's tier binds here, on the
            // forwarded leg, and on the local leg alike - never a flat
            // per-leg constant.
            let context = match &self.context_store {
                Some(store) => Some(
                    store
                        .mint(zone, chain.invoking_identity(), row.deadline_tier.budget_ms())
                        .map_err(|_| {
                            EnvelopeRefusal::new(
                                chain.root_invocation_id().to_owned(),
                                operation,
                                STALE_CONTEXT,
                            )
                        })?,
                ),
                None => None,
            };
            let ctx = InvocationCtx {
                operation: row.operation,
                zone,
                invocation_id: chain.root_invocation_id(),
                chain,
                nested,
            };
            let outcome = self
                .dispatcher
                .dispatch(DirectInvocation {
                    ctx,
                    payload: &payload,
                    context: context.as_ref(),
                    fds,
                    fd_kind: row.fd_kind,
                })
                .await
                .map_err(|failure| {
                    // Every dispatch failure keeps its own code: a refusal a
                    // handler chose, an error a handler hit, a budget a handler
                    // overran, a crash a handler died in, and a depth cap a
                    // nested chain tripped all name themselves (KTD7, KTD6),
                    // so no dispatch failure is flattened into the
                    // missing-handler refusal. The peer codes are entries of
                    // the envelope's closed set; a failure that carries a code
                    // outside it (a non-conforming peer) is refused under the
                    // envelope's own errored code, and the peer's code rides
                    // in the record's detail so an operator still sees it.
                    match ENVELOPE_REFUSALS
                        .iter()
                        .find(|code| **code == failure.code)
                        .copied()
                    {
                        Some(code) => EnvelopeRefusal::with_detail(
                            chain.root_invocation_id().to_owned(),
                            operation,
                            code,
                            failure.detail,
                        ),
                        None => EnvelopeRefusal::with_detail(
                            chain.root_invocation_id().to_owned(),
                            operation,
                            ERRORED,
                            Some(failure.detail.unwrap_or(failure.code)),
                        ),
                    }
                })?;
            Ok((outcome, row.audit_join_identity(&payload)))
        }
        .await;
        let settled_outcome = match &dispatched {
            Ok(_) => "ok".to_owned(),
            Err(refusal) => refusal.code.clone(),
        };
        tracing::info!(
            operation = operation,
            zone = zone,
            nested = nested,
            elapsed_ms = probe_start.elapsed().as_millis(),
            serving = serves_locally,
            outcome = settled_outcome.as_str(),
            "envelope invocation settled"
        );
        if serves_locally {
            // Exactly one record per invocation on the broker side: the
            // root leg writes the root record, a nested leg writes its
            // correlation record keyed by the root invocation id and
            // its depth, and the outcome is the admission's - never a
            // second record for one leg (KTD6).
            let (outcome, code) = match &dispatched {
                Ok(_) => (ChainOutcome::Succeeded, None),
                Err(refusal) => (ChainOutcome::Refused, Some(refusal.code.to_owned())),
            };
            self.record_chain_outcome(ChainRecord {
                ts_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                record_class: if nested {
                    ChainRecordClass::Correlation
                } else {
                    ChainRecordClass::Root
                },
                leg: ChainLeg::Broker,
                invocation_id: chain.root_invocation_id().to_owned(),
                depth: chain.depth() as u32,
                initiating_identity: chain.initiating_identity().to_owned(),
                invoking_identity: chain.invoking_identity().to_owned(),
                operation: operation.to_owned(),
                zone: zone.to_owned(),
                outcome,
                code,
            });
        }
        match dispatched {
            Ok((outcome, audit_join_identity)) => Ok(Invocation {
                audit_join_identity,
                invocation_id: chain.root_invocation_id().to_owned(),
                outcome,
            }),
            Err(refusal) => Err(refusal),
        }
    }

    /// Append one chain record to the wired sink, logging an append
    /// failure rather than changing the call's outcome: the audit record
    /// must never become a refusal the caller sees.
    fn record_chain_outcome(&self, record: ChainRecord) {
        if let Some(sink) = &self.chain_audit
            && let Err(error) = sink.record(&record)
        {
            tracing::error!(error = %error, "broker-side chain audit record failed");
        }
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

    /// Whether the committed grants of one row cover one presented evidence
    /// chain.
    ///
    /// The graft rule (KTD6): a handler may call another provider's service
    /// only when a committed row and its grants cover the call under the
    /// chain's initiating principal - the envelope's authz check applies to
    /// the initiating principal, never the daemon class the handler's
    /// process re-presents. A broker-class identity is checked as its
    /// class; a provider identity is the provider-class name the committed
    /// grants are written in, so it is checked by name.
    pub fn chain_granted(row: &BrokerOperationRow, chain: &EvidenceChain) -> bool {
        let identity = chain.initiating_identity();
        match CallerAuthority::classify_identity(identity) {
            Some(authority) => Self::granted(row, authority),
            None => row.authz.allowed_groups.contains(&identity),
        }
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

    /// Whether one request's attached fd set is admitted by the row's fd
    /// facet, before dispatch。
    ///
    /// A row that declares no fd carriage admits only the empty set;an
    /// oversized-but-transport-legal set (count over the row's declared max,
    /// kind mismatch, or a row whose facet exceeds the frame ceiling) is
    /// refused with the fd-leg code rather than let the transport truncate an
    /// anonymous oversized frame.where
    fn request_fds_admitted(row: &BrokerOperationRow, fds: &[OwnedFd]) -> bool {
        if fds.len() > usize::from(row.max_fds) {
            return false;
        }
        if !fds.is_empty() {
            if row.max_fds as usize > MAX_FRAME_FDS {
                return false;
            }
            let Some(kind) = row.fd_kind else {
                return false;
            };
            // An `Any` row admits every attached descriptor regardless of
            // fstat kind - the mixed or anon-inode legs (an fd-inheriting
            // spawn, pidfds) cannot be reduced to one named kernel kind
            // (U10).
            if kind != FdKind::Any
                && !fds.iter().all(|fd| Self::fd_kind_of(fd) == Some(kind))
            {
                return false;
            }
        }
        true
    }

    /// The kernel kind one descriptor presents,or None when its fstat
    /// reports a kind the carrier vocabulary does not carry..
    fn fd_kind_of(fd: &OwnedFd) -> Option<FdKind> {
        use nix::libc;
        let stat = nix::sys::stat::fstat(fd.as_raw_fd()).ok()?;
        match stat.st_mode & libc::S_IFMT {
            libc::S_IFIFO => Some(FdKind::Fifo),
            libc::S_IFSOCK => Some(FdKind::Socket),
            libc::S_IFCHR => Some(FdKind::CharDevice),
            libc::S_IFBLK => Some(FdKind::BlockDevice),
            libc::S_IFREG => Some(FdKind::Regular),
            libc::S_IFDIR => Some(FdKind::Directory),
            _ => None,
        }
    }
}

/// Assemble one [`BrokerEnvelope`].
pub struct BrokerEnvelopeBuilder {
    profile: BrokerProfileId,
    dispatcher: Box<dyn OperationDispatcher>,
    committed: Vec<&'static str>,
    extras: Vec<BrokerOperationRow>,
    context_store: Option<Arc<TrustedContextStore>>,
    chain_audit: Option<Arc<dyn ChainAuditSink>>,
}

impl BrokerEnvelopeBuilder {
    /// Attest every call through a trusted-context store.
    ///
    /// The envelope mints a broker-attested context block for each call
    /// before dispatch and refuses with the stale-context code any call the
    /// store cannot attest - a Zone the daemon has not published is refused
    /// rather than attested blind.
    pub fn with_trusted_context(mut self, store: Arc<TrustedContextStore>) -> Self {
        self.context_store = Some(store);
        self
    }

    /// Write the chain audit records of the envelope's broker-side leg to
    /// `sink`.
    ///
    /// The KTD6 audit rule: the leg executing the root operation writes
    /// exactly one root record per root invocation, each nested leg writes
    /// a correlation record keyed by the root invocation id and its depth,
    /// and forwarded ops never produce broker-side records - they are the
    /// daemon side's alone. A wire that executes no in-broker operations
    /// (the fail-closed no-handler shape) writes nothing.
    pub fn with_chain_audit(mut self, sink: Arc<dyn ChainAuditSink>) -> Self {
        self.chain_audit = Some(sink);
        self
    }

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
            context_store: self.context_store,
            chain_audit: self.chain_audit,
        }
    }
}

/// A registered operation handler.
pub type OperationHandler =
    Box<dyn Fn(&DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> + Send + Sync>;

/// A registered operation handler, shared into the abortable task that runs
/// it.
type SharedOperationHandler =
    Arc<dyn Fn(&DirectInvocation<'_>) -> Result<DispatchOutcome, DispatchFailure> + Send + Sync>;

/// The broker's handler worker set: the bounded pool in-broker handlers run
/// on.
///
/// A local handler runs as an abortable task on this set rather than inline
/// on the accept loop's executor: the call's effective budget (the
/// broker-attested context deadline, or the handler table's own default) is
/// enforced by task abort at expiry, so a non-yielding handler cannot starve
/// the accept loop or a concurrent innocent operation. The set is one
/// bounded multi-threaded runtime for the process, built the same way the
/// broker's other runtimes are (`enable_all`, named workers); the existing
/// blocking dispatch pool remains for non-async adapters.
static HANDLER_WORKER_SET: std::sync::LazyLock<tokio::runtime::Runtime> =
    std::sync::LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("d2b-broker-handler")
            .enable_all()
            .build()
            .expect("broker handler worker set")
    });

fn handler_worker_set() -> &'static tokio::runtime::Runtime {
    &HANDLER_WORKER_SET
}

/// The message one handler panic carried, when it carried a message.
fn panic_message(payload: Box<dyn std::any::Any + Send>) -> Option<String> {
    match payload.downcast_ref::<&str>() {
        Some(message) => Some((*message).to_owned()),
        None => payload.downcast_ref::<String>().cloned(),
    }
}

/// The owned shape one local handler task runs on.
///
/// A handler is handed borrowed invocation data; an abortable task owns its
/// inputs, so the borrowed invocation is materialized - payload, context,
/// chain, and descriptor dups - at the task boundary, exactly as a spawned
/// forward leg owns its frame.
struct OwnedInvocation {
    operation: String,
    zone: String,
    invocation_id: String,
    chain: EvidenceChain,
    nested: bool,
    payload: CanonicalJsonObject,
    context: Option<ForwardContext>,
    fds: Vec<OwnedFd>,
    fd_kind: Option<FdKind>,
}

/// A dispatcher that serves an explicit handler table.
///
/// Used by the broker's own operations and by the tests; a family row is
/// served by the declaring crate's process, not here.
///
/// A local handler runs as an abortable task on the broker's handler worker
/// set under the invocation's effective budget: the broker-attested context
/// deadline when the call carries one, the table's own deadline (the carrier
/// default) otherwise. The budget is enforced by task abort at expiry, so a
/// non-yielding handler cannot starve the accept loop or a concurrent
/// innocent operation, and a panicking handler is a named crash refusal
/// rather than a panic escaping into the caller's socket path.
pub struct HandlerTable {
    handlers: Vec<(&'static str, SharedOperationHandler)>,
    /// The budget one local handler may consume when the call carries no
    /// context block; a context block's deadline supersedes it.
    deadline: Duration,
}

impl std::fmt::Debug for HandlerTable {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HandlerTable")
            .field("handlers", &self.handlers.len())
            .field("deadline_ms", &self.deadline.as_millis())
            .finish_non_exhaustive()
    }
}

impl Default for HandlerTable {
    /// An empty handler table whose local handlers run under the carrier
    /// default budget.
    fn default() -> Self {
        Self {
            handlers: Vec::new(),
            deadline: Duration::from_millis(DEFAULT_CONTEXT_DEADLINE_MS),
        }
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
        self.handlers.push((operation, Arc::new(handler)));
        self
    }

    /// Bound every local handler to `deadline` when the call carries no
    /// context block.
    ///
    /// The carrier's default is the table's own default; a broker-attested
    /// context block's deadline supersedes the table's value whenever the
    /// call mints one, which is how the row's declared deadline tier (KTD4)
    /// binds the local leg: the mint puts the tier's budget into the block,
    /// and the task serves the block's budget, not the table's.
    pub fn with_deadline(mut self, deadline: Duration) -> Self {
        self.deadline = deadline;
        self
    }
}

impl HandlerTable {
    /// Whether a handler is registered for `operation`.
    ///
    /// The public probe a mixed dispatcher uses to route one invocation to
    /// its in-broker leg without duplicating the table's private entries.
    pub fn serves(&self, operation: &str) -> bool {
        self.handlers
            .iter()
            .any(|(registered, _)| *registered == operation)
    }
}

impl OperationDispatcher for HandlerTable {
    /// The handler table executes its handlers in the broker's own process:
    /// this is the in-broker leg, so the envelope audits its executions
    /// broker-side.
    ///
    /// `serves_operation` keeps the dispatcher-wide answer (the default):
    /// every outcome the table's leg produces - a handler's result, an
    /// admission refusal, or the table's own unregistered-handler refusal -
    /// is produced inside the broker's process, so the broker side records
    /// it. The per-operation [`Self::serves`] probe exists for the mixed
    /// [`KernelDispatcher`], whose forwarded half must not record
    /// broker-side.
    fn serves_locally(&self) -> bool {
        true
    }

    fn dispatch<'a>(&'a self, invocation: DirectInvocation<'a>) -> DispatchFuture<'a> {
        let Some((_, handler)) = self
            .handlers
            .iter()
            .find(|(operation, _)| *operation == invocation.ctx.operation)
        else {
            let operation = invocation.ctx.operation;
            return Box::pin(async move {
                Err(DispatchFailure::unregistered_handler(format!(
                    "no local handler for {operation}"
                )))
            });
        };
        // A local handler runs as an abortable task on the handler worker
        // set rather than inline on the caller's executor: the effective
        // budget (the context block's deadline when the call carries one,
        // the table's default otherwise) is enforced by task abort at
        // expiry, and a panic inside the handler is caught at the task
        // boundary and becomes a named crash refusal.
        let handler = Arc::clone(handler);
        let operation = invocation.ctx.operation.to_owned();
        let panic_fallback = format!("handler panicked dispatching {operation}");
        let zone = invocation.ctx.zone.to_owned();
        let invocation_id = invocation.ctx.invocation_id.to_owned();
        let chain = invocation.ctx.chain.clone();
        let nested = invocation.ctx.nested;
        let payload = invocation.payload.clone();
        let context = invocation.context.cloned();
        let fd_kind = invocation.fd_kind;
        let budget = invocation
            .context
            .map(|context| Duration::from_millis(context.deadline_ms))
            .unwrap_or(self.deadline);
        // The task owns its inputs, so the caller's descriptors are
        // duplicated; a dup shares the open file description, which is
        // exactly the visibility an inline dispatch would have had.
        let fds = match invocation
            .fds
            .iter()
            .map(|fd| fd.try_clone())
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(fds) => fds,
            Err(error) => {
                return Box::pin(async move {
                    Err(DispatchFailure::with_detail(
                        ERRORED,
                        format!("duplicate invocation descriptors: {error}"),
                    ))
                })
            }
        };
        Box::pin(async move {
            let owned = OwnedInvocation {
                operation,
                zone,
                invocation_id,
                chain,
                nested,
                payload,
                context,
                fds,
                fd_kind,
            };
            let joined = handler_worker_set().spawn(async move {
                let invocation = DirectInvocation {
                    ctx: InvocationCtx {
                        operation: &owned.operation,
                        zone: &owned.zone,
                        invocation_id: &owned.invocation_id,
                        chain: &owned.chain,
                        nested: owned.nested,
                    },
                    payload: &owned.payload,
                    context: owned.context.as_ref(),
                    fds: &owned.fds,
                    fd_kind: owned.fd_kind,
                };
                handler(&invocation)
            });
            // The task is aborted the moment its budget expires and the call
            // is refused by name; a handler that never yields keeps its own
            // worker busy but cannot starve the accept loop or queue-block
            // an innocent operation running on another worker.
            let abort = joined.abort_handle();
            match tokio::time::timeout(budget, joined).await {
                Ok(Ok(Ok(outcome))) => Ok(outcome),
                Ok(Ok(Err(failure))) => Err(failure),
                Ok(Err(join)) if join.is_panic() => Err(DispatchFailure::with_detail(
                    HANDLER_CRASHED,
                    panic_message(join.into_panic()).unwrap_or(panic_fallback),
                )),
                Ok(Err(join)) => Err(DispatchFailure::with_detail(
                    ERRORED,
                    format!("handler task ended without a result: {join}"),
                )),
                Err(_elapsed) => {
                    abort.abort();
                    Err(DispatchFailure::with_detail(
                        HANDLER_TIMED_OUT,
                        format!("handler exceeded its {} ms budget", budget.as_millis()),
                    ))
                }
            }
        })
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
            context_store: None,
            chain_audit: None,
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
    fn dispatch<'a>(&'a self, invocation: DirectInvocation<'a>) -> DispatchFuture<'a> {
        self.forwarder
            .forward(crate::forwarding::ForwardedOperation {
                operation: invocation.ctx.operation,
                zone: invocation.ctx.zone,
                invocation_id: invocation.ctx.invocation_id,
                chain: invocation.ctx.chain,
                nested: invocation.ctx.nested,
                payload: invocation.payload,
                context: invocation.context,
                fds: invocation.fds,
                fd_kind: invocation.fd_kind,
            })
    }
}

/// The broker's own-process kernel seam (U10).
///
/// The U10 sandwich serves each process-family operation's privileged,
/// resource-agnostic kernel in-broker as a broker-generic committed row
/// while the family operation itself stays forwarded to the declaring
/// process. This dispatcher routes an in-broker registered kernel to the
/// local handler table and every other operation to the forward carrier,
/// and answers the audit-rule question per operation - so one envelope
/// over both legs records the in-broker leg broker-side and leaves the
/// forwarded leg's record to the daemon side, never both (KTD6).
pub struct KernelDispatcher {
    kernels: HandlerTable,
    forwarded: ForwardingDispatcher,
}

impl std::fmt::Debug for KernelDispatcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KernelDispatcher")
            .finish_non_exhaustive()
    }
}

impl KernelDispatcher {
    /// Build the mixed dispatcher over `kernels` and the forward carrier.
    pub fn new(kernels: HandlerTable, forwarded: ForwardingDispatcher) -> Self {
        Self { kernels, forwarded }
    }
}

impl OperationDispatcher for KernelDispatcher {
    fn serves_locally(&self) -> bool {
        false
    }

    fn serves_operation(&self, operation: &str) -> bool {
        self.kernels.serves(operation)
    }

    fn dispatch<'a>(&'a self, invocation: DirectInvocation<'a>) -> DispatchFuture<'a> {
        if self.kernels.serves(invocation.ctx.operation) {
            self.kernels.dispatch(invocation)
        } else {
            self.forwarded.dispatch(invocation)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_audit::evidence_chain::root_record_count;
    use crate::catalog::{BrokerAuthzFacets, DeadlineTier, OperationOwner};
    use crate::forwarding::{ForwardFuture, ForwardedOperation, OperationForwarder, SocketForwarder};
    use std::io;
    use std::os::fd::{AsRawFd, OwnedFd};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// One multi-thread runtime for these tests: the envelope's call is async
    /// because the peer leg is, so a test drives it the way the broker does.
    fn runtime() -> &'static tokio::runtime::Runtime {
        static RUNTIME: std::sync::LazyLock<tokio::runtime::Runtime> =
            std::sync::LazyLock::new(|| {
                tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                    .expect("envelope test runtime")
            });
        &RUNTIME
    }

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
            let Some((request, request_fds)) = crate::protocol::recv_json_frame_with_fds::<
                d2b_contracts_broker::broker_wire::ForwardOperationRequest,
            >(fd.as_raw_fd())
            .expect("read forwarded call") else {
                continue;
            };
            observed.fetch_add(1, Ordering::AcqRel);
            let (response, response_fds) = answer_from(
                dispatcher.as_ref(),
                &request.operation,
                &request.zone,
                &request.invocation_id,
                request.payload,
                &request_fds,
                request.fd_kinds.first().copied(),
                request.context.as_ref(),
            );
            let raw: Vec<std::os::fd::RawFd> =
                response_fds.iter().map(|fd| fd.as_raw_fd()).collect();
            crate::protocol::send_json_frame_with_fds(fd.as_raw_fd(), &response, &raw)
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
        fds: &[OwnedFd],
        fd_kind: Option<FdKind>,
        context: Option<&d2b_contracts_broker::broker_wire::ForwardContext>,
    ) -> (d2b_contracts_broker::broker_wire::ForwardOperationResponse, Vec<OwnedFd>) {
        use d2b_contracts_broker::broker_wire::{
            ForwardOperationOutcome, ForwardOperationResponse,
        };
        let payload: CanonicalJsonObject =
            serde_json::from_value(payload).expect("canonical payload");
        // The loopback peer re-roots a chain from the carrier: the request's
        // invocation id and the attestation's initiating identity (or the
        // daemon class on the context-free carrier), exactly what the wire
        // preserves for the daemon-side leg.
        let chain = EvidenceChain::root(
            invocation_id.to_owned(),
            context
                .map(|context| context.initiating_identity.as_str())
                .unwrap_or("daemon")
                .to_owned(),
        );
        let (outcome, response_fds) = match runtime().block_on(dispatcher.dispatch(DirectInvocation {
            ctx: InvocationCtx {
                operation,
                zone,
                invocation_id,
                chain: &chain,
                // The loopback peer re-roots the chain, so this leg is the
                // root leg of its own invocation.
                nested: false,
            },
            payload: &payload,
            context,
            fds,
            fd_kind,
        })) {
            Ok(DispatchOutcome { result, fds: result_fds }) => {
                let kinds = result_fds
                    .iter()
                    .map(|fd| {
                        BrokerEnvelope::fd_kind_of(fd)
                            .expect("a test peer returns a known-kernel-kind fd")
                    })
                    .collect::<Vec<_>>();
                (
                    ForwardOperationOutcome::Result {
                        result: serde_json::to_value(&result).expect("render result"),
                        fd_indexes: (0..result_fds.len() as u32).collect(),
                        fd_kinds: kinds,
                    },
                    result_fds,
                )
            }
            Err(failure) => (
                ForwardOperationOutcome::Refused {
                    code: failure.code,
                },
                Vec::new(),
            ),
        };
        (ForwardOperationResponse { outcome }, response_fds)
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
            justification: Some("test row the broker owns"),
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
            max_fds: 0,
            fd_kind: None,
            state_cell: None,
            cell_durability: None,
            deadline_tier: DeadlineTier::Standard,
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
                fds: Vec::new(),
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
        let refusal = runtime().block_on(envelope
            .call(
                CallerAuthority::Daemon,
                "NoSuchOperation",
                "zone-a",
                &serde_json::json!({}),
            ))
            .expect_err("an unknown operation is refused");
        assert_eq!(refusal.code, UNKNOWN_OPERATION);
        assert_eq!(refusal.operation, "NoSuchOperation");
    }

    #[test]
    fn an_uncommitted_row_is_refused() {
        let envelope = probe_envelope();
        // The committed catalog declares the operation; this broker does not
        // serve it, and deny-by-default refuses it rather than guessing.
        let refusal = runtime().block_on(envelope
            .call(
                CallerAuthority::Daemon,
                "ApplySysctl",
                "zone-a",
                &serde_json::json!({}),
            ))
            .expect_err("an uncommitted row is refused");
        assert_eq!(refusal.code, UNCOMMITTED_OPERATION);
        assert_eq!(refusal.operation, "ApplySysctl");
    }

    #[test]
    fn an_ungranted_caller_is_refused() {
        let envelope = probe_envelope();
        let refusal = runtime().block_on(envelope
            .call(
                CallerAuthority::Unauthorized,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
            .expect_err("an ungranted caller is refused");
        assert_eq!(refusal.code, UNGRANTED_CALLER);
        assert_eq!(refusal.operation, "ProbeOperation");
    }

    #[test]
    fn a_wire_inherited_operation_is_refused() {
        // The typed wire rows (U12's network-fds family rows among the
        // retired set) keep `payload_provenance: Wire` while the family
        // kernels ride the `Request`-provenance envelope surface;
        // `StartSystemdUnit` still carries the typed wire contract, so it
        // is the fixture of an operation the generic envelope cannot carry.
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(echo_table()))
            .commit_all()
            .build();
        let refusal = runtime().block_on(envelope
            .call(
                CallerAuthority::Daemon,
                "StartSystemdUnit",
                "zone-a",
                &serde_json::json!({}),
            ))
            .expect_err("a wire-inherited operation is not carried generically");
        assert_eq!(refusal.code, WIRE_INHERITED_OPERATION);
    }

    #[test]
    fn a_payload_outside_the_row_schema_is_refused() {
        let envelope = probe_envelope();
        let undeclared = runtime().block_on(envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x", "extra": 1 }),
            ))
            .expect_err("an undeclared field is refused");
        assert_eq!(undeclared.code, INVALID_PAYLOAD);
        let missing = runtime().block_on(envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({}),
            ))
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
        let refusal = runtime().block_on(envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
            .expect_err("a row with no handler is refused");
        assert_eq!(refusal.code, UNREGISTERED_HANDLER);
    }

    #[test]
    fn the_closed_set_carries_the_peer_codes() {
        // The taxonomy's peer codes and the stale-wire entry are closed-set
        // members (KTD7): a dispatch failure carrying one surfaces under its
        // own code, never flattened into the missing-handler refusal.
        for code in [
            HANDLER_REFUSED,
            HANDLER_ERRORED,
            HANDLER_TIMED_OUT,
            HANDLER_CRASHED,
            STALE_WIRE_VERSION,
            NESTED_DEPTH_EXCEEDED,
        ] {
            assert!(
                ENVELOPE_REFUSALS.contains(&code),
                "{code} must be a closed-set entry"
            );
        }
    }

    #[test]
    fn a_handler_refusal_and_a_handler_error_each_keep_their_own_codes() {
        // A refusal a handler chose and a failure a handler hit are two
        // cases with two codes: the caller can tell a deliberate refusal
        // from a broken handler, and neither is a missing handler.
        let refused = HandlerTable::new().with("ProbeOperation", |_invocation| {
            Err(DispatchFailure::with_detail(HANDLER_REFUSED, "grant exhausted"))
        });
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(refused))
            .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
            .build();
        let refusal = runtime().block_on(envelope.call(
            CallerAuthority::Daemon,
            "ProbeOperation",
            "zone-a",
            &serde_json::json!({ "label": "x" }),
        ))
        .expect_err("a handler that refuses keeps its own code");
        assert_eq!(refusal.code, HANDLER_REFUSED);
        assert_eq!(refusal.detail.as_deref(), Some("grant exhausted"));
        assert_eq!(refusal.audit_fields()["reason"], HANDLER_REFUSED);

        let errored = HandlerTable::new().with("ProbeOperation", |_invocation| {
            Err(DispatchFailure::with_detail(HANDLER_ERRORED, "backend failed"))
        });
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(errored))
            .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
            .build();
        let refusal = runtime().block_on(envelope.call(
            CallerAuthority::Daemon,
            "ProbeOperation",
            "zone-a",
            &serde_json::json!({ "label": "x" }),
        ))
        .expect_err("a handler that failed keeps its own code");
        assert_eq!(refusal.code, HANDLER_ERRORED);
        assert_eq!(refusal.detail.as_deref(), Some("backend failed"));
    }

    #[test]
    fn a_forwarded_peer_failure_keeps_its_own_code() {
        // The peer's dispatch failure crosses the socket and back under its
        // own code: a handler refusal in the declaring process is reported
        // as a refusal, not flattened into a missing handler here.
        let peer = loopback_peer(HandlerTable::new().with("ProbeOperation", |_invocation| {
            Err(DispatchFailure::with_detail(
                HANDLER_REFUSED,
                "the declaring process refused",
            ))
        }));
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
        .build();
        let refusal = runtime().block_on(envelope.call(
            CallerAuthority::Daemon,
            "ProbeOperation",
            "zone-a",
            &serde_json::json!({ "label": "x" }),
        ))
        .expect_err("a peer refusal keeps its own code");
        assert_eq!(refusal.code, HANDLER_REFUSED);
        assert_eq!(peer.calls(), 1, "the call must have crossed the socket");
    }

    #[test]
    fn a_failure_code_outside_the_closed_set_is_refused_as_errored_with_the_code_as_detail() {
        // The caller's vocabulary is the envelope's closed set: a peer code
        // the set does not carry is refused under the envelope's own errored
        // code, and the peer's spelling rides in the record's detail so an
        // operator still sees it.
        let peer = loopback_peer(HandlerTable::new().with("ProbeOperation", |_invocation| {
            Err(DispatchFailure::new("family-own-code"))
        }));
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
        .build();
        let refusal = runtime().block_on(envelope.call(
            CallerAuthority::Daemon,
            "ProbeOperation",
            "zone-a",
            &serde_json::json!({ "label": "x" }),
        ))
        .expect_err("a code outside the closed set is refused");
        assert_eq!(refusal.code, ERRORED);
        assert_eq!(refusal.detail.as_deref(), Some("family-own-code"));
        assert_eq!(peer.calls(), 1);
    }

    #[test]
    fn a_panicking_local_handler_writes_a_handler_crashed_refusal_and_the_envelope_keeps_serving() {
        // A crash inside a local handler is caught at the task boundary: the
        // caller sees a typed crash refusal carrying the panic's message,
        // never a dropped caller, and the envelope keeps serving.
        let table = HandlerTable::new().with("ProbeOperation", |_invocation| {
            panic!("probe blew up");
        });
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(table))
            .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
            .build();
        let refusal = runtime().block_on(envelope.call(
            CallerAuthority::Daemon,
            "ProbeOperation",
            "zone-a",
            &serde_json::json!({ "label": "x" }),
        ))
        .expect_err("a panicking handler is a typed refusal, never a dropped caller");
        assert_eq!(refusal.code, HANDLER_CRASHED);
        assert_eq!(refusal.detail.as_deref(), Some("probe blew up"));
        assert!(refusal.invocation_id.starts_with("invocation-"));
        let fields = refusal.audit_fields();
        assert_eq!(fields["reason"], HANDLER_CRASHED);
        assert_eq!(fields["invocation_id"], refusal.invocation_id);
        assert_eq!(fields["operation"], "ProbeOperation");
        // The panic never left the handler task: the same envelope turns the
        // next crash into the same typed refusal instead of dropping the
        // caller.
        let again = runtime().block_on(envelope.call(
            CallerAuthority::Daemon,
            "ProbeOperation",
            "zone-a",
            &serde_json::json!({ "label": "x" }),
        ))
        .expect_err("a second crash is the same typed refusal");
        assert_eq!(again.code, HANDLER_CRASHED);
    }

    #[test]
    fn a_spinning_handler_is_aborted_to_timed_out_while_an_innocent_operation_answers() {
        // A local handler that never awaits is aborted at its effective
        // budget: the call is refused by name while the handler hogs its own
        // worker, and an innocent operation on the same envelope answers
        // from another worker.
        const BUDGET: Duration = Duration::from_millis(400);
        let mut spin_row = declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]);
        spin_row.operation = "SpinningOperation";
        let envelope = Arc::new(
            BrokerEnvelope::over(
                BrokerProfileId::Host,
                Box::new(
                    HandlerTable::new()
                        .with_deadline(BUDGET)
                        .with("SpinningOperation", |_invocation| {
                            // A handler that never yields: it would starve an
                            // inline executor, so the task must be aborted
                            // for the call to end at its budget.
                            let start = std::time::Instant::now();
                            while start.elapsed() < Duration::from_secs(2) {
                                std::hint::spin_loop();
                            }
                            Ok(DispatchOutcome {
                                result: serde_json::from_value(serde_json::json!({
                                    "done": true
                                }))
                                .expect("canonical"),
                                fds: Vec::new(),
                            })
                        })
                        .with("ProbeOperation", |_invocation| {
                            Ok(DispatchOutcome {
                                result: serde_json::from_value(
                                    serde_json::json!({ "echo": "ok" }),
                                )
                                .expect("canonical"),
                                fds: Vec::new(),
                            })
                        }),
                ),
            )
            .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
            .declare(spin_row)
            .build(),
        );
        let spinner = Arc::clone(&envelope);
        let started = std::time::Instant::now();
        let spinning = runtime().spawn(async move {
            spinner
                .call(
                    CallerAuthority::Daemon,
                    "SpinningOperation",
                    "zone-a",
                    &serde_json::json!({ "label": "x" }),
                )
                .await
        });
        // The spinner is on its own worker by now; the innocent operation
        // answers while it is still in flight.
        std::thread::sleep(Duration::from_millis(60));
        runtime()
            .block_on(envelope.call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
            .expect("the innocent operation answers while the spinner is in flight");
        let refusal = runtime()
            .block_on(spinning)
            .expect("the spinning call joined")
            .expect_err("the spinning handler is aborted at its budget");
        assert_eq!(refusal.code, HANDLER_TIMED_OUT);
        let elapsed = started.elapsed();
        assert!(
            elapsed >= BUDGET && elapsed < Duration::from_secs(2),
            "the refusal is the budget's, not the handler's: {elapsed:?}"
        );
    }

    #[test]
    fn a_declared_operation_dispatches_with_an_invocation_id() {
        let envelope = probe_envelope();
        let invocation = runtime().block_on(envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
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
        let refusal = runtime().block_on(envelope
            .call(
                CallerAuthority::Unauthorized,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
            .expect_err("an ungranted caller is refused");
        assert!(refusal.invocation_id.starts_with("invocation-"));
        let fields = refusal.audit_fields();
        assert_eq!(fields["invocation_id"], refusal.invocation_id);
        assert_eq!(fields["operation"], "ProbeOperation");
        assert_eq!(fields["reason"], UNGRANTED_CALLER);
        // Two refusals are two named invocations, not one anonymous denial.
        let other = runtime().block_on(envelope
            .call(
                CallerAuthority::Unauthorized,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
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
            runtime().block_on(envelope
                .call(
                    CallerAuthority::Daemon,
                    "ProbeOperation",
                    "zone-a",
                    &serde_json::json!({ "label": "x", "kind": kind }),
                ))
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
        let plain = runtime().block_on(envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
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
        let invocation = runtime().block_on(envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
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
        let refusal = runtime().block_on(envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
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
        let refusal = runtime().block_on(envelope
            .call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
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

    use d2b_contracts_broker::broker_wire::{FD_LEG as WIRE_FD_LEG, FdKind, MAX_FRAME_FDS};

    /// A row a test declares plus the fd-leg facets a forward carrier
    /// test needs.
    ///
    /// The closure returns the row as the carrier: the two fd-leg facets
    /// (max count + kernel kind) live on the committed row like every
    /// other per-operation facet, so a test sets them the same way it sets
    /// an audit join.
    fn fd_declared_row(max_fds: u8, kind: FdKind) -> BrokerOperationRow {
        let mut row = declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]);
        row.max_fds = max_fds;

        row.fd_kind = Some(kind);
        row
    }

    #[test]
    fn fd_round_trips_through_the_loopback_peer_and_reads_back() {
        use nix::unistd::{pipe, read, write};
        let (request_read, request_write) = pipe().expect("request pipe");
        write(&request_write, b"ping").expect("write request bytes");

        // The answering peer mints its own response descriptor and keeps its
        // write end, so the broker caller can read back what it wrote.

        let returned_write_end: Arc<std::sync::Mutex<Option<OwnedFd>>> = Arc::default();
        let write_slot = Arc::clone(&returned_write_end);
        let peer = loopback_peer(HandlerTable::new().with("ProbeOperation", move |invocation| {
            // Request leg:the descriptor the caller attached crossed the socket
            // and reads back what the caller wrote.to
            let mut echoed = [0_u8; 4];
            let n = read(invocation.fds[0].as_raw_fd(), &mut echoed).expect("read request fd");
            assert_eq!(&echoed[..n], b"ping");
            // Response leg:answer with a fresh descriptor the peer minted.to
            let (answer_read, answer_write) = pipe().expect("answer pipe");
            *Arc::clone(&returned_write_end).lock().expect("slot") = Some(answer_write);
            Ok(DispatchOutcome {
                result: serde_json::from_value(serde_json::json!({ "echo": "ok" }))
                    .expect("canonical"),
                fds: vec![answer_read],
            })
        }));
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .declare(fd_declared_row(1, FdKind::Fifo))
        .build();
        let invocation = runtime()
            .block_on(envelope.call_with_fds(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
                &[request_read],
            ))
            .expect("the peer answered with the fd leg");
        assert_eq!(peer.calls(), 1, "the call must have crossed the socket");
        assert_eq!(invocation.outcome.fds.len(), 1, "the peer's minted descriptor must cross back");

        // Read back what the peer wrote through the returned descriptor:the
        // minted write end stays on the peer's side,and the returned read end
        // is a working duplicated handle, exactly as w12 asserts.to
        let write_end = write_slot.lock().expect("slot").take().expect("peer minted write end");
        write(&write_end, b"pong").expect("write answer bytes");
        let mut read_back = [0_u8; 4];
        let n = read(invocation.outcome.fds[0].as_raw_fd(), &mut read_back)
            .expect("read returned fd");
        assert_eq!(&read_back[..n], b"pong");
    }

    #[test]
    fn a_zero_fd_response_to_an_fd_declaring_operation_is_a_valid_empty_set() {
        let peer = loopback_peer(HandlerTable::new().with("ProbeOperation", |_invocation| {
            Ok(DispatchOutcome {
                result: serde_json::from_value(serde_json::json!({ "echo": "none" }))
                    .expect("canonical"),
                fds: Vec::new(),
            })
        }));
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .declare(fd_declared_row(1, FdKind::Fifo))
        .build();
        let invocation = runtime()
            .block_on(envelope.call_with_fds(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
                &[],
            ))
            .expect("an fd-declaring operation may answer with zero fds");
        assert!(invocation.outcome.fds.is_empty());
        assert_eq!(peer.calls(), 1);
    }

    #[test]
    fn an_oversized_request_fd_set_is_refused_with_the_fd_leg_code_before_dispatch() {
        use nix::unistd::pipe;
        let peer = loopback_peer(echo_table());
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .declare(fd_declared_row(2, FdKind::Fifo))
        .build();
        let mut fds = Vec::new();
        for _ in 0..3 {
            let fd = pipe().expect("pipe").0;
            fds.push(fd);
        }
        let refusal = runtime()
            .block_on(envelope.call_with_fds(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
                &fds,
            ))
            .expect_err("an oversized leg is refused before dispatch");
        assert_eq!(refusal.code, FD_LEG);
        assert_eq!(peer.calls(), 0, "the oversized leg must not reach the peer");
    }

    #[test]
    fn a_row_declaring_more_than_the_frame_ceiling_refuses_an_fd_leg_with_the_fd_leg_code() {
        use nix::unistd::pipe;
        let peer = loopback_peer(echo_table());
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .declare(fd_declared_row(MAX_FRAME_FDS as u8 + 1, FdKind::Fifo))
        .build();
        let fd = pipe().expect("pipe").0;
        let refusal = runtime()
            .block_on(envelope.call_with_fds(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
                &[fd],
            ))
            .expect_err("a row over the frame ceiling cannot carry fds");
        assert_eq!(refusal.code, FD_LEG);
        assert_eq!(peer.calls(), 0);
    }

    #[test]
    fn a_request_fd_that_mismatches_the_declared_kind_is_refused_with_the_fd_leg_code() {
        use nix::sys::socket::{AddressFamily, SockFlag, SockType, socketpair};
        let peer = loopback_peer(echo_table());
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .declare(fd_declared_row(1, FdKind::Fifo))
        .build();
        let (socket, _peer) = socketpair(
            AddressFamily::Unix,
            SockType::SeqPacket,
            None,
            SockFlag::SOCK_CLOEXEC,
        )
        .expect("socketpair");
        let refusal = runtime()
            .block_on(envelope.call_with_fds(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
                &[socket],
            ))
            .expect_err("a socket where the row declares a fifo is refused");
        assert_eq!(refusal.code, FD_LEG);
        assert_eq!(peer.calls(), 0);
    }

    #[test]
    fn a_peer_that_returns_an_fd_it_did_not_mint_this_call_is_refused_with_the_fd_leg_code() {
        use nix::unistd::pipe;
        let (request_read, _request_write) = pipe().expect("request pipe");
        let peer = loopback_peer(HandlerTable::new().with("ProbeOperation", move |invocation| {
            Ok(DispatchOutcome {
                result: serde_json::from_value(serde_json::json!({ "echo": "stolen" }))
                    .expect("canonical"),
                // Return the call's own descriptor - a descriptor the peer did
                // not mint this call. The carrier spoils the theft at the wire
                // boundary rather than at the handler.se
                fds: vec![invocation.fds[0].try_clone().expect("dup request fd")],
            })
        }));
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .declare(fd_declared_row(1, FdKind::Fifo))
        .build();
        let refusal = runtime()
            .block_on(envelope.call_with_fds(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
                &[request_read],
            ))
            .expect_err("returning the call's own descriptor is not minting");
        assert_eq!(refusal.code, FD_LEG);  // (also = WIRE_FD_LEG)
        assert_eq!(WIRE_FD_LEG, "fd-leg");
        assert_eq!(peer.calls(), 1);
    }

    use d2b_contracts_broker::broker_wire::{
        ForwardContext, PublishTrustedContextValues, STALE_CONTEXT as WIRE_STALE_CONTEXT,
    };

    /// A daemon publication for a test Zone, stated once.
    fn published(zone: &str) -> PublishTrustedContextValues {
        PublishTrustedContextValues {
            zone: zone.to_owned(),
            provider_set_revision: 2,
            controller_generation: 4,
            guest_generation: 7,
        }
    }

    fn context_store() -> (tempfile::TempDir, Arc<TrustedContextStore>) {
        let dir = tempfile::tempdir().expect("store dir");
        let store = Arc::new(TrustedContextStore::open(dir.path()).expect("open the store"));
        (dir, store)
    }

    #[test]
    fn the_envelope_refuses_to_mint_until_the_daemon_publishes() {
        // The broker refuses to attest a call into a Zone it holds no
        // published values for: the call is refused with the stale-context
        // code before dispatch, so it never reaches the forwarding peer.
        let peer = loopback_peer(echo_table());
        let (_dir, store) = context_store();
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .with_trusted_context(store)
        .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
        .build();
        let refusal = runtime()
            .block_on(envelope.call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
            .expect_err("a Zone with no published values cannot be attested");
        assert_eq!(refusal.code, STALE_CONTEXT);
        assert_eq!(refusal.code, WIRE_STALE_CONTEXT);
        assert_eq!(peer.calls(), 0, "the refusal must come before dispatch");
        // And once the daemon publishes the Zone, the same envelope mints.
        let (_dir, store) = context_store();
        store
            .publish(&published("zone-a"))
            .expect("the daemon published the Zone");
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .with_trusted_context(store)
        .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
        .build();
        runtime()
            .block_on(envelope.call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
            .expect("a published Zone is attested");
    }

    #[test]
    fn the_minted_context_reaches_the_forwarding_peer_verbatim() {
        use d2b_contracts_broker::broker_wire::DEFAULT_CONTEXT_DEADLINE_MS;
        // The context the broker mints must cross the socket and reach the
        // declaring process as the same closed block: the peer's handler
        // reads it back through its invocation, and an envelope that minted
        // a different block, or none, could not produce these values.
        let observed: Arc<Mutex<Option<ForwardContext>>> = Arc::default();
        let slot = Arc::clone(&observed);
        let peer = loopback_peer(HandlerTable::new().with("ProbeOperation", move |invocation| {
            *Arc::clone(&slot).lock().expect("slot") = invocation.context.cloned();
            Ok(DispatchOutcome {
                result: serde_json::from_value(serde_json::json!({ "echo": "ok" }))
                    .expect("canonical"),
                fds: Vec::new(),
            })
        }));
        let (_dir, store) = context_store();
        store
            .publish(&published("zone-a"))
            .expect("the daemon published the Zone");
        let envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(peer.forwarder())),
        )
        .with_trusted_context(store)
        .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
        .build();
        runtime()
            .block_on(envelope.call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
            .expect("the peer served the attested call");
        assert_eq!(peer.calls(), 1);
        let context = observed
            .lock()
            .expect("slot")
            .take()
            .expect("the peer received the minted context");
        assert_eq!(context.broker_epoch, 1, "the first store instance mints under epoch one");
        assert_eq!(context.zone, "zone-a");
        assert_eq!(context.provider_set_revision, 2);
        assert_eq!(context.controller_generation, 4);
        assert_eq!(context.guest_generation, 7);
        assert_eq!(context.initiating_identity, "daemon");
        assert_eq!(context.deadline_ms, DEFAULT_CONTEXT_DEADLINE_MS);
    }

    /// The row's declared deadline tier is the budget the mint attests: a
    /// Standard row mints the carrier default, an Extended row mints the
    /// shared ceiling - and both disagree with the flat per-leg constant a
    /// tier-less broker would use, because KTD4 puts the budget on the
    /// context block, where both execution legs read it.
    #[test]
    fn the_row_deadline_tier_is_the_budget_the_mint_attests() {
        use d2b_contracts_broker::broker_wire::{
            DEFAULT_CONTEXT_DEADLINE_MS, MAX_CONTEXT_DEADLINE_MS,
        };
        let observe = |tier: DeadlineTier| {
            let observed: Arc<Mutex<Option<ForwardContext>>> = Arc::default();
            let slot = Arc::clone(&observed);
            let peer = loopback_peer(HandlerTable::new().with("ProbeOperation", move |invocation| {
                *Arc::clone(&slot).lock().expect("slot") = invocation.context.cloned();
                Ok(DispatchOutcome {
                    result: serde_json::from_value(serde_json::json!({ "echo": "ok" }))
                        .expect("canonical"),
                    fds: Vec::new(),
                })
            }));
            let (_dir, store) = context_store();
            store
                .publish(&published("zone-a"))
                .expect("the daemon published the Zone");
            let mut row = declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]);
            row.deadline_tier = tier;
            let envelope = BrokerEnvelope::over(
                BrokerProfileId::Host,
                Box::new(ForwardingDispatcher::new(peer.forwarder())),
            )
            .with_trusted_context(store)
            .declare(row)
            .build();
            runtime()
                .block_on(envelope.call(
                    CallerAuthority::Daemon,
                    "ProbeOperation",
                    "zone-a",
                    &serde_json::json!({ "label": "x" }),
                ))
                .expect("the peer served the attested call");
            observed
                .lock()
                .expect("slot")
                .take()
                .expect("the peer received the minted context")
        };

        let standard = observe(DeadlineTier::Standard);
        assert_eq!(standard.deadline_ms, DEFAULT_CONTEXT_DEADLINE_MS);
        assert_eq!(standard.deadline_ms, DeadlineTier::Standard.budget_ms());
        let extended = observe(DeadlineTier::Extended);
        assert_eq!(extended.deadline_ms, MAX_CONTEXT_DEADLINE_MS);
        assert_eq!(extended.deadline_ms, DeadlineTier::Extended.budget_ms());
        // The tier's budgets are the shared carrier's closed contract: the
        // extended tier is the largest budget the carrier admits, so no
        // tier can exceed the receiving leg's absolute ceiling.
        assert!(DeadlineTier::Standard.budget_ms() < DeadlineTier::Extended.budget_ms());
        assert!(DeadlineTier::Extended.budget_ms() <= MAX_CONTEXT_DEADLINE_MS);
    }

    #[test]
    fn the_store_cache_is_monotonic_and_durable() {
        let (dir, store) = context_store();
        store
            .publish(&published("zone-a"))
            .expect("the first publication lands");
        // A rollback is not a fresh publication: re-publishing older state
        // over newer is refused with the stale-context code and leaves the
        // cached values where they were.
        let mut rolled_back = published("zone-a");
        rolled_back.guest_generation = 1;
        let error = store
            .publish(&rolled_back)
            .expect_err("a publication that moves a generation backwards is refused");
        assert_eq!(error.code(), STALE_CONTEXT);
        assert_eq!(store.epoch(), 1);

        // A reopen is a fresh broker instance: the cache survives it, and
        // the epoch strictly advances so no prior context can be re-minted
        // into validity.
        drop(store);
        let reopened = TrustedContextStore::open(dir.path()).expect("reopen the store");
        assert_eq!(reopened.epoch(), 2, "restart mints under a fresh epoch");
        assert!(reopened.holds_zone("zone-a"), "published values survive restart");
        let context = reopened
            .mint("zone-a", "daemon", DEFAULT_CONTEXT_DEADLINE_MS)
            .expect("the reopened store mints the Zone");
        assert_eq!(context.broker_epoch, 2);
        assert_eq!(context.provider_set_revision, 2);
        assert_eq!(context.guest_generation, 7);
    }

    #[test]
    fn a_restarting_broker_invalidates_every_previous_context_via_the_epoch() {
        // Any context minted before a restart fails it regardless of
        // generation equality: the reopened store mints under a strictly
        // larger epoch, so a previously minted block cannot be re-minted -
        // it still carries the old epoch.
        let (dir, store) = context_store();
        store
            .publish(&published("zone-a"))
            .expect("the daemon published the Zone");
        let before = store
            .mint("zone-a", "daemon", DEFAULT_CONTEXT_DEADLINE_MS)
            .expect("the store mints before restart");
        assert_eq!(before.broker_epoch, 1);
        drop(store);

        let reopened = TrustedContextStore::open(dir.path()).expect("the broker restarts");
        assert_eq!(reopened.epoch(), 2);
        // The pre-restart context was minted under epoch one; the restarted
        // broker's epoch is two, so the old block can never be admitted
        // again - re-minting it is the restarted broker's mint, which
        // carries the fresh nonce.
        let after = reopened
            .mint("zone-a", "daemon", DEFAULT_CONTEXT_DEADLINE_MS)
            .expect("the restarted broker mints");
        assert_eq!(after.broker_epoch, 2);
        assert_ne!(before.broker_epoch, after.broker_epoch);
        assert_ne!(before, after, "the contexts differ in the epoch alone");
    }

    /// An in-memory chain audit sink that accumulates records for
    /// assertion. Mirrors the daemon-side `RecordingChainSink` shape.
    #[derive(Default)]
    struct RecordingChainSink(Arc<Mutex<Vec<ChainRecord>>>);

    impl RecordingChainSink {
        fn snapshot(&self) -> Vec<ChainRecord> {
            self.0.lock().expect("chain sink").clone()
        }
    }

    impl ChainAuditSink for RecordingChainSink {
        fn record(&self, record: &ChainRecord) -> io::Result<()> {
            self.0.lock().expect("chain sink").push(record.clone());
            Ok(())
        }
    }

    /// A forwarder that captures the chain its peer would have answered
    /// under and answers the call locally, so a test can assert exactly
    /// what crossed the forward seam.
    struct CapturingForwarder {
        chains: Arc<Mutex<Vec<EvidenceChain>>>,
    }

    impl OperationForwarder for CapturingForwarder {
        fn forward<'a>(&'a self, invocation: ForwardedOperation<'a>) -> ForwardFuture<'a> {
            self.chains.lock().expect("chains").push(invocation.chain.clone());
            Box::pin(async move {
                Ok(DispatchOutcome {
                    result: serde_json::from_value(serde_json::json!({ "forwarded": true }))
                    .expect("canonical forwarded result"),
                    fds: Vec::new(),
                })
            })
        }
    }

    #[test]
    fn a_nested_call_records_one_root_and_one_correlation_under_the_initiating_provider() {
        // KTD6 happy path: a handler calls another provider's service and
        // the audit records name the initiating provider - the root record
        // once, the nested leg's correlation record keyed on the root
        // invocation id and depth one - and the handler's trusted context
        // carried the chain it was called under.
        let recorder = Arc::new(RecordingChainSink::default());
        let sink: Arc<dyn ChainAuditSink> = Arc::clone(&recorder) as Arc<dyn ChainAuditSink>;
        let captured: Arc<Mutex<Vec<EvidenceChain>>> = Arc::default();
        let captured_handle = Arc::clone(&captured);
        let table = HandlerTable::new().with("AlphaService", move |invocation| {
            captured_handle
                .lock()
                .expect("captured")
                .push(invocation.ctx.chain.clone());
            Ok(DispatchOutcome {
                result: serde_json::from_value(serde_json::json!({ "label": "x" }))
                    .expect("canonical result"),
                fds: Vec::new(),
            })
        }).with("BetaService", |invocation| {
            Ok(DispatchOutcome {
                result: serde_json::from_value(serde_json::json!({ "label": invocation.ctx.zone }))
                    .expect("canonical result"),
                fds: Vec::new(),
            })
        });
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(table))
            .with_chain_audit(Arc::clone(&sink))
            .declare(declared_row("AlphaService", &["label"], &["label"], &["provider-alpha"]))
            .declare(declared_row("BetaService", &["label"], &["label"], &["provider-alpha"]))
            .build();
        let root = runtime()
            .block_on(envelope.call(
                CallerAuthority::Provider("provider-alpha"),
                "AlphaService",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
            .expect("the provider-rooted call dispatches");
        let root_id = root.invocation_id.clone();
        let seen = captured
            .lock()
            .expect("captured")
            .last()
            .expect("the handler saw a chain")
            .clone();
        assert_eq!(seen.root_invocation_id(), root_id);
        assert!(!seen.is_nested());
        assert_eq!(seen.initiating_identity(), "provider-alpha");
        assert_eq!(seen.invoking_identity(), "provider-alpha");
        let chain = EvidenceChain::root(root_id.clone(), "provider-alpha")
            .nested("provider-alpha")
            .nested("provider-beta");
        let nested = runtime()
            .block_on(envelope.call_nested(
                chain,
                "BetaService",
                "zone-a",
                &serde_json::json!({ "label": "y" }),
            ))
            .expect("a covered nested call dispatches");
        assert_eq!(
            nested.invocation_id, root_id,
            "a nested leg shares the root invocation id"
        );
        let records = recorder.snapshot();
        assert_eq!(records.len(), 2, "{records:?}");
        let root_record = records
            .iter()
            .find(|record| record.is_root())
            .expect("one root record");
        assert_eq!(root_record.invocation_id, root_id);
        assert_eq!(root_record.depth, 0);
        assert_eq!(root_record.leg, ChainLeg::Broker);
        assert_eq!(root_record.initiating_identity, "provider-alpha");
        assert_eq!(root_record.invoking_identity, "provider-alpha");
        assert_eq!(root_record.operation, "AlphaService");
        assert_eq!(root_record.outcome, ChainOutcome::Succeeded);
        let correlation = records
            .iter()
            .find(|record| !record.is_root())
            .expect("one correlation record");
        assert_eq!(correlation.correlation_key(), (root_id.as_str(), 2));
        assert_eq!(correlation.initiating_identity, "provider-alpha");
        assert_eq!(correlation.invoking_identity, "provider-beta");
        assert_eq!(correlation.operation, "BetaService");
        assert_eq!(correlation.outcome, ChainOutcome::Succeeded);
        assert_eq!(
            root_record_count(&records, &root_id),
            1,
            "exactly one root record per invocation"
        );
    }

    #[test]
    fn a_self_reentrant_call_without_a_granting_row_refuses_under_the_initiating_principal() {
        // Graft rule (KTD6): a handler may call another provider's service
        // only when a committed row and its grants cover the call under
        // the chain's initiating principal. The daemon-rooted chain covers
        // rows granted to the daemon class; the handler's own provider-only
        // row is not among them, so the self-re-entrant call refuses with
        // the ungranted-caller code and still records its refusing leg.
        let recorder = Arc::new(RecordingChainSink::default());
        let sink: Arc<dyn ChainAuditSink> = Arc::clone(&recorder) as Arc<dyn ChainAuditSink>;
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(echo_table()))
            .with_chain_audit(Arc::clone(&sink))
            .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
            .declare(declared_row("GraftService", &[], &[], &["provider-alpha"]))
            .build();
        let root = runtime()
            .block_on(envelope.call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
            .expect("the daemon-rooted call dispatches");
        // The handler (provider-alpha) re-enters its own service: the chain
        // is [daemon, provider-alpha], and the daemon's grants do not cover
        // a row granted to the provider alone.
        let chain = EvidenceChain::root(root.invocation_id.clone(), "daemon")
            .nested("provider-alpha");
        let refusal = runtime()
            .block_on(envelope.call_nested(
                chain,
                "GraftService",
                "zone-a",
                &serde_json::json!({}),
            ))
            .expect_err("a self-re-entrant call without a granting row refuses");
        assert_eq!(refusal.code, UNGRANTED_CALLER);
        let records = recorder.snapshot();
        assert_eq!(records.len(), 2, "{records:?}");
        let refused = records
            .iter()
            .find(|record| !record.is_root())
            .expect("the refused leg recorded");
        assert_eq!(refused.correlation_key(), (root.invocation_id.as_str(), 1));
        assert_eq!(refused.leg, ChainLeg::Broker);
        assert_eq!(refused.initiating_identity, "daemon");
        assert_eq!(refused.invoking_identity, "provider-alpha");
        assert_eq!(refused.outcome, ChainOutcome::Refused);
        assert_eq!(refused.code.as_deref(), Some(UNGRANTED_CALLER));
        assert_eq!(root_record_count(&records, &root.invocation_id), 1);
    }

    #[test]
    fn a_nested_loop_trips_the_depth_cap_with_the_loop_code_and_never_a_second_root() {
        // KTD6 loop rule: a chain deeper than the cap refuses with the
        // dedicated closed-set code, every admitted leg recorded exactly
        // one correlation record, and the root record is never repeated.
        let recorder = Arc::new(RecordingChainSink::default());
        let sink: Arc<dyn ChainAuditSink> = Arc::clone(&recorder) as Arc<dyn ChainAuditSink>;
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(echo_table()))
            .with_chain_audit(Arc::clone(&sink))
            .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
            .build();
        let root = runtime()
            .block_on(envelope.call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
            .expect("the daemon-rooted call dispatches");
        let root_id = root.invocation_id.clone();
        let mut chain = EvidenceChain::root(root_id.clone(), "daemon");
        for depth in 1..=(MAX_NESTED_DEPTH + 1) {
            chain = chain.nested("provider-alpha");
            let call = runtime().block_on(envelope.call_nested(
                chain.clone(),
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ));
            if depth <= MAX_NESTED_DEPTH {
                call.expect("a chain within the cap dispatches");
            } else {
                let refusal = call.expect_err("a chain past the cap is refused");
                assert_eq!(refusal.code, NESTED_DEPTH_EXCEEDED);
                assert_eq!(refusal.invocation_id, root_id);
            }
        }
        let records = recorder.snapshot();
        let max = MAX_NESTED_DEPTH as u32;
        assert_eq!(
            records.len(),
            1 + MAX_NESTED_DEPTH + 1,
            "one root, one correlation per admitted leg, one refusing leg: {records:?}"
        );
        for depth in 0..=max {
            assert!(
                records
                    .iter()
                    .any(|record| record.correlation_key() == (root_id.as_str(), depth)
                        || (depth == 0 && record.is_root())),
                "depth {depth} recorded"
            );
        }
        let refusing = records
            .iter()
            .find(|record| record.depth == max + 1)
            .expect("the refusing leg recorded");
        assert_eq!(refusing.outcome, ChainOutcome::Refused);
        assert_eq!(refusing.code.as_deref(), Some(NESTED_DEPTH_EXCEEDED));
        assert_eq!(refusing.leg, ChainLeg::Broker);
        assert_eq!(
            root_record_count(&records, &root_id),
            1,
            "a loop never writes a second root record"
        );
    }

    #[test]
    fn a_mixed_leg_chain_records_one_root_for_the_in_broker_leg_only() {
        // KTD6 mixed-leg shape (in-broker root, forwarded nested leg): the
        // in-broker leg writes the one root record; the forwarded leg
        // crosses the forward seam carrying the chain the daemon-side leg
        // will correlate under, and the broker writes nothing for the
        // forwarded leg (forwarded ops audit daemon-side only, KTD6).
        let recorder = Arc::new(RecordingChainSink::default());
        let sink: Arc<dyn ChainAuditSink> = Arc::clone(&recorder) as Arc<dyn ChainAuditSink>;
        let chains: Arc<Mutex<Vec<EvidenceChain>>> = Arc::default();
        let table = HandlerTable::new().with("RootService", |_invocation| {
            Ok(DispatchOutcome {
                result: serde_json::from_value(serde_json::json!({ "root": true }))
                    .expect("canonical result"),
                fds: Vec::new(),
            })
        });
        let root_envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(table))
            .with_chain_audit(Arc::clone(&sink))
            .declare(declared_row("RootService", &[], &[], &["d2bd"]))
            .build();
        let root = runtime()
            .block_on(root_envelope.call(
                CallerAuthority::Daemon,
                "RootService",
                "zone-a",
                &serde_json::json!({}),
            ))
            .expect("the in-broker root dispatches");
        let root_id = root.invocation_id.clone();
        let nested_chain = EvidenceChain::root(root_id.clone(), "daemon")
            .nested("provider-alpha");
        let forwarded_envelope = BrokerEnvelope::over(
            BrokerProfileId::Host,
            Box::new(ForwardingDispatcher::new(CapturingForwarder {
                chains: Arc::clone(&chains),
            })),
        )
        .with_chain_audit(Arc::clone(&sink))
        .declare(declared_row("ForwardedService", &[], &[], &["d2bd"]))
        .build();
        let nested = runtime()
            .block_on(forwarded_envelope.call_nested(
                nested_chain,
                "ForwardedService",
                "zone-a",
                &serde_json::json!({}),
            ))
            .expect("the forwarded nested call is answered by its forwarder");
        assert_eq!(nested.invocation_id, root_id);
        let records = recorder.snapshot();
        assert_eq!(
            records.len(),
            1,
            "the broker wrote only the in-broker root record: {records:?}"
        );
        assert!(records[0].is_root());
        assert_eq!(records[0].invocation_id, root_id);
        assert_eq!(records[0].leg, ChainLeg::Broker);
        assert_eq!(root_record_count(&records, &root_id), 1);
        // The forwarded leg crossed the seam with the chain; the daemon-side
        // leg records its correlation under exactly this key (covered on
        // the daemon side by forward_rendezvous tests).
        let chains = chains.lock().expect("chains");
        assert_eq!(chains.len(), 1, "the forwarder saw the one nested call");
        assert_eq!(chains[0].root_invocation_id(), root_id);
        assert_eq!(chains[0].depth(), 1);
        assert_eq!(chains[0].initiating_identity(), "daemon");
        assert_eq!(chains[0].invoking_identity(), "provider-alpha");
    }

    #[test]
    fn in_broker_refusals_record_exactly_one_root_record_each() {
        // The one-record-per-invocation invariant holds across outcomes:
        // a successful call, a refused call, and an unknown operation
        // each leave exactly one root record under their own invocation id.
        let recorder = Arc::new(RecordingChainSink::default());
        let sink: Arc<dyn ChainAuditSink> = Arc::clone(&recorder) as Arc<dyn ChainAuditSink>;
        let envelope = BrokerEnvelope::over(BrokerProfileId::Host, Box::new(echo_table()))
            .with_chain_audit(Arc::clone(&sink))
            .declare(declared_row("ProbeOperation", &["label"], &["label"], &["d2bd"]))
            .build();
        let ok = runtime()
            .block_on(envelope.call(
                CallerAuthority::Daemon,
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "x" }),
            ))
            .expect("the granted call dispatches");
        let refused = runtime()
            .block_on(envelope.call(
                CallerAuthority::Provider("provider-alpha"),
                "ProbeOperation",
                "zone-a",
                &serde_json::json!({ "label": "y" }),
            ))
            .expect_err("a provider without a granting row is refused");
        assert_eq!(refused.code, UNGRANTED_CALLER);
        let unknown = runtime()
            .block_on(envelope.call(
                CallerAuthority::Daemon,
                "NoSuchService",
                "zone-a",
                &serde_json::json!({}),
            ))
            .expect_err("an undeclared operation is refused");
        assert_eq!(unknown.code, UNKNOWN_OPERATION);
        let records = recorder.snapshot();
        assert_eq!(records.len(), 3, "{records:?}");
        let ok_record = records
            .iter()
            .find(|record| record.invocation_id == ok.invocation_id)
            .expect("the successful call recorded");
        assert!(ok_record.is_root());
        assert_eq!(ok_record.outcome, ChainOutcome::Succeeded);
        let refused_record = records
            .iter()
            .find(|record| record.invocation_id == refused.invocation_id)
            .expect("the refused call recorded");
        assert!(refused_record.is_root());
        assert_eq!(refused_record.outcome, ChainOutcome::Refused);
        assert_eq!(refused_record.code.as_deref(), Some(UNGRANTED_CALLER));
        let unknown_record = records
            .iter()
            .find(|record| record.invocation_id == unknown.invocation_id)
            .expect("the unknown call recorded");
        assert!(unknown_record.is_root());
        assert_eq!(unknown_record.outcome, ChainOutcome::Refused);
        assert_eq!(unknown_record.code.as_deref(), Some(UNKNOWN_OPERATION));
        for invocation_id in [
            &ok.invocation_id,
            &refused.invocation_id,
            &unknown.invocation_id,
        ] {
            assert_eq!(
                root_record_count(&records, invocation_id),
                1,
                "exactly one root record per invocation"
            );
        }
    }
}
