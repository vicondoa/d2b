//! The forwarding rendezvous: the endpoint the broker dials.
//!
//! The broker is a seqpacket listener and links no provider crate, so a
//! committed operation's handler cannot run there. The dispatch step of its
//! envelope forwards instead: one validated, authorized invocation crosses to
//! the process that declares the handler, which answers with the handler's
//! canonical result or its own refusal code. This module is the receiving
//! end - a seqpacket listener the daemon owns, beside the broker socket - and
//! the routing it performs: the call names its Zone and its operation, and
//! the Zone's started providers resolve it to the provider that declared the
//! operation, whose toolkit envelope runs the handler.
//!
//! Two properties are structural. The listener binds only when the
//! environment names a path, so a daemon with no forwarding peer is
//! fail-closed exactly as the broker is, and the broker's own no-default
//! stance keeps the two ends in agreement. And resolution runs over the
//! *started* providers' own descriptor tables - the handler table the plane
//! registered - so an operation no started provider declares is refused by
//! name before any handler runs.
//!
//! Freshness is broker-attested. The broker is the sole minter of the
//! context block that rides each forwarded request: it names the broker's
//! epoch nonce, the Zone, the provider-set revision, the controller and
//! guest generations, the initiating identity, and the operation's deadline
//! budget. Those values are daemon-owned, so the daemon publishes its
//! current provider-set revision and generations to the broker; the broker
//! caches them as durable, monotonically increasing state and refuses to
//! mint until it holds a value for the Zone a call names. This endpoint
//! enforces the attestation field-wise against its own current values: a
//! stale broker epoch, a Zone not bound to the call, an older provider-set
//! revision, a lower controller or guest generation, or a block a mutator
//! changed is refused with the dedicated stale-context code - the broker is
//! the sole minter, so any mismatch is freshness failure or tampering, and
//! a broker restart invalidates every previously minted context through its
//! fresh epoch nonce. The context's deadline budget is served as the
//! per-call handler deadline here, bounded by an absolute ceiling. The one
//! identity this endpoint checks beyond the attestation is the transport
//! peer's: nothing in the frame binds the call to the authorization the
//! broker performed, so `SO_PEERCRED` must name the broker before a single
//! frame is read (see [`ServingPosture`]). The provider-side envelope then
//! runs each declared operation under the declaring provider's own
//! reference, which is the one caller fact this process owns: a provider
//! may run the handlers it declared, and the envelope refuses every caller
//! it holds no grant for.
//!
//! Effect-service methods ride the same carrier (U8, KTD5). A forwarded
//! operation is served by an effect service exactly when a declared
//! method's `operation` facet names it (KD6: operations resolve to
//! services); this endpoint resolves the operation to the declaring
//! service's live generational binding through the hosting API
//! (`ProviderRuntime::resolve_effect_service_for_operation`) and dispatches
//! the canonical payload to the hosted actor. The binding's revision is
//! captured when the call starts and checked again at dispatch: a respawn
//! or republish that bumped the revision in between - or an actor that
//! died under the in-flight call - is refused with the dedicated
//! `stale-revision` code, never hung against a dead or superseded
//! generation. An operation no hosted service declares falls through to
//! the provider operation tables, and an operation nothing in this process
//! declares is refused like any uncommitted operation.
//!
//! The endpoint serves on the daemon's runtime rather than on a thread per
//! call: the listener and every accepted connection are registered with the
//! reactor (`tokio::io::unix::AsyncFd`, the pattern the session crate drives
//! its own seqpacket endpoints with), the request frame and the reply frame
//! are awaited rather than blocked, and each call runs as a task under an
//! async admission bound. The in-flight cap therefore bounds live calls
//! rather than pinned threads, and a handler that never finishes is refused
//! by name at its deadline instead of holding a slot forever.

use std::collections::BTreeMap;
use std::io;
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use d2b_audit::evidence_chain::{
    ChainAuditSink, ChainLeg, ChainOutcome, ChainRecord, ChainRecordClass, EvidenceChain,
    MAX_NESTED_DEPTH, NESTED_DEPTH_EXCEEDED,
};
use d2b_contracts_broker::FORWARD_SOCKET_ENV;
use d2b_contracts_broker::broker_wire::{
    DEFAULT_CONTEXT_DEADLINE_MS, FD_LEG, FdKind, ForwardContext, ForwardOperationOutcome,
    ForwardOperationRequest, ForwardOperationResponse, MAX_CONTEXT_DEADLINE_MS, MAX_FRAME_FDS,
    STALE_CONTEXT,
};
use d2b_contracts_resource::v3::CanonicalJsonObject;
use d2b_provider_toolkit::operations::{UNCOMMITTED_OPERATION, UNGRANTED_CALLER};
use d2b_resource_types::{KernelCaller, MethodFdContract, OperationResult};
use d2bd_runtime::concurrency::DEFAULT_MAX_INFLIGHT_CONNECTIONS;
use d2bd_runtime::runtime_process::{RuntimeIdentity, bind_public_socket};
use d2bd_runtime::typed_error::{ErrorSource, TypedError, error_source};
use d2bd_runtime::unix_transport::{close_received_fds, read_frame_with_fds, write_frame_with_fds};
use d2bd_runtime::wire::MAX_FRAME_SIZE;
use nix::sys::socket::{MsgFlags, getsockopt, recv, send, sockopt};
use socket2::Socket;
use tokio::io::Interest;
use tokio::io::unix::AsyncFd;
use tokio::sync::Semaphore;

use crate::effect_service_actors::{EffectServiceBinding, ServiceCallData};
use crate::provider_lifecycle::ProviderRuntime;
use d2b_provider_toolkit::{EffectResponse, EffectServiceError};
use d2b_resource_runtime::context::ServiceResourceContext;

/// The refusal code for a forwarded payload this endpoint cannot read as the
/// canonical object the broker validated.
pub(crate) const INVALID_PAYLOAD: &str = "invalid-payload";

/// The refusal code for a forwarded handler that did not finish within its
/// deadline.
///
/// The name is this endpoint's own: the broker's round-trip budget is the
/// outer bound on the call, and this is the refusal a caller sees when the
/// handler inside it stalls past the daemon's own.
pub(crate) const FORWARD_TIMEOUT: &str = "forward-timeout";

/// The refusal code for a forwarded handler that panicked mid-dispatch.
///
/// The spelling is the envelope's closed peer-code entry (KTD7): a crash on
/// this leg is a named refusal the broker's envelope surfaces under its own
/// `handler-crashed` code, never a dropped socket the broker reads as a
/// round-trip timeout.
pub(crate) const HANDLER_CRASHED: &str = "handler-crashed";

/// The refusal code for an effect-service call whose generation this
/// process moved past (KTD5).
///
/// A respawn or provider-set republish bumps the service's generational
/// binding revision; a call that resolved its binding under the older
/// revision - or was still in flight when the actor died under it - is
/// refused with this dedicated code, never hung against a dead or
/// superseded generation. The spelling is this endpoint's own refusal
/// entry for the KTD5 dedicated code, beside the `stale-context` code the
/// attestation path uses for provider-set freshness.
pub(crate) const STALE_REVISION: &str = "stale-revision";

/// The refusal code for an effect service that declined a call.
///
/// The spelling is the taxonomy's handler-refused entry (KTD7): the
/// service answered with its own refusal, and the call crosses back under
/// the code the broker's envelope already admits.
pub(crate) const HANDLER_REFUSED: &str = "handler-refused";

/// The read deadline for one forwarded request frame: a connected peer that
/// sends nothing is closed rather than holding an in-flight slot.
const FORWARD_REQUEST_DEADLINE: Duration = Duration::from_secs(30);

/// The handler deadline for one forwarded invocation on a context-free
/// carrier, and the default budget a minted context carries.
///
/// It sits below the broker's default forward round trip (30 s), so a stalled
/// handler is refused by name while the caller is still listening rather than
/// reported as a peer that never answered. The value is the carrier's shared
/// default (see [`DEFAULT_CONTEXT_DEADLINE_MS`]): the context block carries
/// the operation's own budget, and a request that rides a context is served
/// under that budget instead of this constant, bounded by the shared ceiling.
const FORWARD_HANDLER_DEADLINE: Duration = Duration::from_millis(DEFAULT_CONTEXT_DEADLINE_MS);

/// The write deadline for one reply frame: a peer that will not read must not
/// hold an in-flight slot open either.
const FORWARD_REPLY_DEADLINE: Duration = Duration::from_secs(5);

/// The backoff after a failed accept, so a listener that keeps refusing does
/// not spin the loop.
const ACCEPT_ERROR_BACKOFF: Duration = Duration::from_millis(10);

/// The drain deadline for a refusal written before the peer's own frame was
/// read. Closing a seqpacket socket with input still pending makes the kernel
/// send a reset, which the peer sees instead of the refusal, so the pending
/// frames are consumed first - exactly as the public socket's refusal path
/// drains for the same reason.
const FORWARD_REFUSAL_DRAIN_DEADLINE: Duration = Duration::from_millis(10);

/// The privileged broker's uid: both the host broker and a realm broker run
/// as root, because their unit does host-mutating work (`nixos-modules/
/// host-broker.nix` fixes `User = "root"`).
const BROKER_UID: u32 = 0;

/// The rendezvous socket the environment names, when it names one.
///
/// The variable is declared with the carrier in `d2b-contracts-broker`, so
/// the broker that dials and the daemon that binds cannot disagree on it.
pub(crate) fn configured_socket() -> Option<PathBuf> {
    std::env::var_os(FORWARD_SOCKET_ENV)
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// One Zone's forwarding binding: the started providers, the provider-set
/// revision of the current publication, and the daemon-owned controller and
/// guest generations the rendezvous enforces attestations against.
struct ZoneBinding {
    /// The provider-set revision: bumped by every publication, so a context
    /// minted against the previous set refuses once a republish lands.
    revision: u64,
    /// The Zone's current controller generation, as the daemon publishes it.
    controller_generation: u64,
    /// The Zone's current guest generation, as the daemon publishes it.
    guest_generation: u64,
    /// The started providers of the current set.
    providers: Arc<ProviderRuntime>,
    /// The U10 family seam: the broker kernel socket, the caller role, the
    /// Zone's trusted bundle, and the daemon-side runner lookup the family
    /// handlers invoke kernels through. Absent when the composition point
    /// wired no seam.
    kernel: Option<KernelCaller>,
    /// The Zone's generic driver context for service resource-state reads
    /// (U3, R7): the manager-plane surface the effect-service leg builds
    /// each invocation's capability object from. Absent when the
    /// composition point wired no seam: a service invocation then receives
    /// a fail-closed context that refuses every read.
    resources: Option<ServiceResourceContext>,
}

/// The started providers of every Zone, keyed by Zone label, plus the
/// attestation state the rendezvous enforces freshness against: the current
/// provider-set revision and generations per Zone, and the broker epoch this
/// process last observed from a publication acknowledgement.
#[derive(Default)]
pub(crate) struct ForwardRendezvous {
    /// The per-Zone forwarding bindings (plan U10: `tokio::sync::Mutex`, the
    /// async-purity replacement for the std table lock; every critical
    /// section is a single map operation, never held across an await).
    zones: tokio::sync::Mutex<BTreeMap<String, ZoneBinding>>,
    /// The broker epoch this process currently validates contexts against.
    /// Zero means no publication has been acknowledged yet: no context can
    /// be verified, so every attested call is refused fail-closed.
    broker_epoch: AtomicU64,
    /// The sink this process's daemon-side leg writes evidence-chain audit
    /// records to, when one is wired.
    ///
    /// Absent, the leg serves without chain records (the seam's unwired
    /// state; the production daemon wires its audit log at its composition
    /// point). The audit rule (KTD6): the leg executing the root operation
    /// writes exactly one root record per root invocation, each nested leg
    /// writes a correlation record keyed by the root invocation id and its
    /// depth, and forwarded ops audit here alone - never also broker-side.
    chain_audit: tokio::sync::Mutex<Option<Arc<dyn ChainAuditSink>>>,
}

impl ForwardRendezvous {
    /// Assemble an empty rendezvous: no Zone has started providers yet.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Publish the providers one Zone started.
    ///
    /// A Zone whose plane re-opens republishes its new provider set; the last
    /// published set is the one that answers, and the republish bumps the
    /// provider-set revision a minted context must match. Returns the new
    /// revision, so the daemon publishes the SAME revision to the broker over
    /// the origination leg: a context the broker mints against it then
    /// matches, and a republish that outran the broker's cache refuses the
    /// pre-republish contexts by the same comparison.
    pub(crate) async fn publish(&self, zone: &str, providers: Arc<ProviderRuntime>) -> u64 {
        let revision = {
            let mut zones = self.zones.lock().await;
            let entry = zones.entry(zone.to_owned()).or_insert(ZoneBinding {
                revision: 0,
                controller_generation: 0,
                guest_generation: 0,
                providers: Arc::clone(&providers),
                kernel: None,
                resources: None,
            });
            entry.revision = entry.revision.saturating_add(1);
            entry.providers = Arc::clone(&providers);
            entry.revision
        };
        // Outside the binding lock: the daemon publishes the SAME revision
        // to the broker over the origination leg, and the acknowledgement
        // path re-enters these bindings through the generation/epoch
        // setters. A set that carries no publication binding (tests,
        // context-free deployments) publishes nothing, and the rendezvous
        // stays fail-closed on its zero epoch.
        providers.publish_trusted_context(self, revision);
        revision
    }

    /// Publish the daemon-owned controller and guest generations one Zone
    /// currently holds.
    ///
    /// The same values the daemon publishes to the broker over the
    /// origination leg; the rendezvous enforces an attestation's generations
    /// against these, so a context minted from older generations refuses
    /// once the daemon's current values move on.
    pub(crate) fn publish_generations(
        &self,
        zone: &str,
        controller_generation: u64,
        guest_generation: u64,
    ) {
        // Synchronous caller surface (the broker publication ack rides the
        // origination leg's sync transport inside `ProviderRuntime::publish_trusted_context`):
        // non-blocking `try_lock` per plan U4. A collision (another
        // operation mid-critical-section, sub-microsecond) skips the
        // generation update fail-closed: the broker keeps the older
        // generations, contexts minted against them refuse as stale, and the
        // next publication self-heals. The epoch STILL advances (it is an
        // atomic), so a skipped update can never admit a context the daemon
        // outgrew - only refuse contexts the broker minted on stale values.
        // R13: std poison recovery (into_inner) had the same recoverable
        // shape; tokio mutexes do not poison, so the failure mode is now
        // contention-only.
        let Ok(mut zones) = self.zones.try_lock() else {
            tracing::warn!(
                zone = %zone,
                "generation publication skipped: rendezvous binding busy"
            );
            return;
        };
        // A Zone with no started providers has no binding to carry values
        // for; the daemon publishes generations alongside the set that
        // serves them.
        if let Some(entry) = zones.get_mut(zone) {
            entry.controller_generation = controller_generation;
            entry.guest_generation = guest_generation;
        }
    }

    /// Record the broker epoch this process validated against.
    ///
    /// The epoch is what the daemon reads from the broker's publication
    /// acknowledgement: every context minted before a broker restart carries
    /// an older epoch, so the moment a fresh acknowledgement lands, every
    /// pre-restart context stops validating regardless of generation
    /// equality.
    pub(crate) fn set_broker_epoch(&self, epoch: u64) {
        self.broker_epoch.store(epoch, Ordering::Relaxed);
    }

    /// Wire one Zone's U10 family seam (the kernel socket, the caller
    /// role, the Zone's trusted bundle, and the daemon-side runner lookup)
    /// into its forwarding binding.
    ///
    /// The composition point calls this once per Zone alongside the
    /// provider publication; a Zone whose seam was never wired serves
    /// forwarded family operations without a kernel leg (they refuse when
    /// their handler needs one).
    pub(crate) async fn set_kernel_seam(&self, zone: &str, kernel: KernelCaller) {
        let mut zones = self.zones.lock().await;
        match zones.get_mut(zone) {
            Some(binding) => binding.kernel = Some(kernel),
            None => tracing::warn!(
                zone = %zone,
                "kernel seam published for a Zone with no provider binding"
            ),
        }
    }

    /// Wire one Zone's service driver context (U3, R7) into its forwarding
    /// binding: the manager-plane surface effect-service invocations reach
    /// resource state through.
    ///
    /// The composition point calls this once per Zone alongside the
    /// provider publication and the kernel seam; a Zone whose seam was
    /// never wired serves effect-service invocations with a fail-closed
    /// context that refuses every read.
    pub(crate) async fn set_resource_reader(
        &self,
        zone: &str,
        resources: ServiceResourceContext,
    ) {
        let mut zones = self.zones.lock().await;
        match zones.get_mut(zone) {
            Some(binding) => binding.resources = Some(resources),
            None => tracing::warn!(
                zone = %zone,
                "resource reader published for a Zone with no provider binding"
            ),
        }
    }

    /// Wire this rendezvous's daemon-side chain audit records to `sink`.
    ///
    /// The daemon calls this once at its composition point; a rendezvous
    /// without a sink serves without chain records. The seam has no
    /// production caller today: its tests are the consumers.
    #[allow(dead_code)]
    pub(crate) async fn set_chain_audit(&self, sink: Arc<dyn ChainAuditSink>) {
        *self.chain_audit.lock().await = Some(sink);
    }

    /// Write one chain record through the wired sink, logging an append
    /// failure rather than changing the call's outcome.
    async fn record_chain(&self, record: ChainRecord) {
        let Some(sink) = self.chain_audit.lock().await.clone() else {
            return;
        };
        if let Err(error) = sink.record(&record) {
            tracing::error!(
                error = %error,
                operation = %record.operation,
                "daemon-side chain audit record failed"
            );
        }
    }

    /// Whether one attested context block is fresh against this process's
    /// current values, for a call naming `request_zone`.
    ///
    /// Field-wise, in every field the attestation carries: the block's
    /// broker epoch must be the epoch this process observed from the broker
    /// (a pre-restart block carries an older one), the Zone must be bound to
    /// the call's Zone, the provider-set revision and the controller and
    /// guest generations must match the Zone's current published values, and
    /// the deadline budget must be a positive value under the shared
    /// ceiling. The broker is the sole minter, so any mismatch is a stale
    /// or mutated attestation.
    async fn context_admitted(&self, context: &ForwardContext, request_zone: &str) -> bool {
        let observed_epoch = self.broker_epoch.load(Ordering::Relaxed);
        if observed_epoch == 0 {
            // No epoch observed yet: the attestation cannot be verified, so
            // no context is admitted - the fail-closed half of the rule that
            // the broker refuses to mint until it holds a value.
            return false;
        }
        if context.broker_epoch != observed_epoch {
            return false;
        }
        if context.zone != request_zone {
            return false;
        }
        if context.deadline_ms == 0 || context.deadline_ms > MAX_CONTEXT_DEADLINE_MS {
            return false;
        }
        let zones = self.zones.lock().await;
        match zones.get(request_zone) {
            Some(binding) => {
                context.provider_set_revision == binding.revision
                    && context.controller_generation == binding.controller_generation
                    && context.guest_generation == binding.guest_generation
            }
            None => false,
        }
    }

    /// Answer one forwarded invocation.
    ///
    /// An operation a declared effect-service method serves (the method's
    /// `operation` facet names it, U8) resolves through the hosting binding
    /// and is answered by the hosted actor; every other operation resolves
    /// to the started provider that declared it. A Zone with no started
    /// providers and an operation nothing in this process declares are the
    /// same refusal: nothing serves the call, and the code the broker's own
    /// envelope uses for that state names it.
    pub(crate) async fn invoke(
        &self,
        request: &ForwardOperationRequest,
        fds: &[RawFd],
        chain: &EvidenceChain,
    ) -> (ForwardOperationResponse, Vec<OwnedFd>) {
        let (providers, kernel, resources) = {
            let zones = self.zones.lock().await;
            match zones.get(&request.zone) {
                Some(binding) => (
                    Some(Arc::clone(&binding.providers)),
                    binding.kernel.clone(),
                    binding.resources.clone(),
                ),
                None => (None, None, None),
            }
        };
        let Some(providers) = providers else {
            return (refused(UNCOMMITTED_OPERATION), Vec::new());
        };
        // The effect-service leg (U8, KTD5): an operation the hosted
        // services declare resolves through the live hosting binding, and a
        // call against a generation this process moved past is refused with
        // the dedicated stale-revision code instead of hanging. An
        // operation no service declares falls through to the provider
        // tables.
        match providers.resolve_effect_service_for_operation(&request.operation).await {
            Ok(binding) => {
                return invoke_effect_service(
                    &binding,
                    request,
                    fds,
                    kernel.as_ref(),
                    resources,
                    chain,
                )
                .await
            }
            Err(EffectServiceError::OperationUnserved { .. }) => {}
            Err(error) => return (refused(effect_refusal_code(&error)), Vec::new()),
        }
        let Some(provider) = providers.declaring_provider(&request.operation) else {
            return (refused(UNCOMMITTED_OPERATION), Vec::new());
        };
        let Ok(bytes) = serde_json::to_vec(&request.payload) else {
            return (refused(INVALID_PAYLOAD), Vec::new());
        };
        let Ok(payload) = CanonicalJsonObject::parse(&bytes) else {
            return (refused(INVALID_PAYLOAD), Vec::new());
        };
        match provider
            .invoke_under_chain(
                &request.operation,
                &request.invocation_id,
                payload,
                fds,
                chain.identities(),
                kernel.as_ref(),
            )
            .await
        {
            Ok(result) => {
                let (response, fds) = result_response_with_fds(result);
                (response, fds)
            }
            Err(failure) => (refused(failure.code()), Vec::new()),
        }
    }

    /// Serve one nested invocation a handler presented, under its evidence
    /// chain.
    ///
    /// The chain is the trusted-context evidence (KTD6): the nested call
    /// presents the chain with the invoking handler's identity appended and
    /// never re-presents as the daemon class. This endpoint refuses a chain
    /// past the depth cap with the dedicated loop-refusal code - so a call
    /// loop trips its own code, never an uncommitted or resolution
    /// refusal, and records the leg's correlation record keyed on the root
    /// invocation id and the leg's depth. Resolution and dispatch are the
    /// same as a forwarded call's, so a nested leg is served by the same
    /// resolution (effect services first, then the declaring provider).
    /// The graft rule's committed-row check is the broker side's: this
    /// process holds no committed rows, so the chain's admission and grants
    /// were decided where the rows live, and this leg enforces the cap and
    /// records the correlation key.
    #[allow(dead_code)] // Test-only: the nested-leg tests drive it; no production caller.
    pub(crate) async fn invoke_nested(
        &self,
        chain: &EvidenceChain,
        operation: &str,
        zone: &str,
        payload: &serde_json::Value,
    ) -> ForwardOperationResponse {
        if chain.depth() > MAX_NESTED_DEPTH {
            let response = refused(NESTED_DEPTH_EXCEEDED);
            self.record_chain(self.chain_record(
                ChainRecordClass::Correlation,
                chain,
                operation,
                zone,
                &response,
            ))
            .await;
            return response;
        }
        let request = ForwardOperationRequest {
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            invocation_id: chain.root_invocation_id().to_owned(),
            payload: payload.clone(),
            context: None,
            chain_identities: None,
            fd_indexes: vec![],
            fd_kinds: vec![],
        };
        let (response, _response_fds) = self.invoke(&request, &[], chain).await;
        self.record_chain(self.chain_record(
            ChainRecordClass::Correlation,
            chain,
            operation,
            zone,
            &response,
        ))
        .await;
        response
    }

    /// The chain record one leg of an invocation writes on this side: a
    /// root record for the root invocation, a correlation record for a
    /// nested leg, both on the shared shape and keyed on the root
    /// invocation id plus the leg's depth (KTD6).
    fn chain_record(
        &self,
        record_class: ChainRecordClass,
        chain: &EvidenceChain,
        operation: &str,
        zone: &str,
        response: &ForwardOperationResponse,
    ) -> ChainRecord {
        let (outcome, code) = match response.outcome {
            ForwardOperationOutcome::Refused { ref code } => {
                (ChainOutcome::Refused, Some(code.clone()))
            }
            ForwardOperationOutcome::Result { .. } => (ChainOutcome::Succeeded, None),
        };
        ChainRecord {
            ts_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
            record_class,
            leg: ChainLeg::Daemon,
            invocation_id: chain.root_invocation_id().to_owned(),
            depth: chain.depth() as u32,
            initiating_identity: chain.initiating_identity().to_owned(),
            invoking_identity: chain.invoking_identity().to_owned(),
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            outcome,
            code,
        }
    }

    /// Answer one admitted connection: read one request frame, invoke the
    /// declared handler under the handler deadline, write one reply frame.
    ///
    /// Driven on the runtime: every wait here is awaited, so a call holds its
    /// admission permit and no thread of its own. The dispatch itself runs
    /// on its own task (KTD7's crash guard): a handler that panics dies in
    /// the task, not in the accept loop, and the call is refused by name.
    async fn serve_connection(
        self: &Arc<Self>,
        connection: &AsyncSeqpacket,
        handler_deadline: Duration,
    ) -> Result<(), TypedError> {
        let (frame, request_fds) = connection
            .read_frame_with_fds(FORWARD_REQUEST_DEADLINE)
            .await?;
        // The frame's descriptors belong to this call:they are closed
        // whether the call is served or refused, after the reply frame has
        // gone (or when this connection errors out).
        let fds = ScmFds::new(request_fds);
        let request: ForwardOperationRequest =
            serde_json::from_slice(&frame).map_err(|error| TypedError::WireInvalidFrame {
                detail: format!(
                    "forwarded request frame is not a ForwardOperationRequest: {error}"
                ),
            })?;
        // The forwarded call's evidence chain, re-rooted from the carrier:
        // a nested leg carries its ordered identities on the wire (U10,
        // KTD6), so this side re-roots the chain from the root invocation
        // id plus those identities and records the leg as a correlation
        // record; a root call carries no chain and re-roots from the
        // attestation's initiating identity, recorded as the invocation's
        // root leg.
        let chain = match &request.chain_identities {
            Some(identities) => {
                let chain = match identities.split_first() {
                    Some((head, tail)) => {
                        let mut chain = EvidenceChain::root(
                            request.invocation_id.clone(),
                            head.clone(),
                        );
                        for identity in tail {
                            chain = chain.nested(identity.clone());
                        }
                        chain
                    }
                    None => EvidenceChain::root(
                        request.invocation_id.clone(),
                        request
                            .context
                            .as_ref()
                            .map(|context| context.initiating_identity.clone())
                            .unwrap_or_else(|| "daemon".to_owned()),
                    ),
                };
// The handler-side legs append the invoking handler's own
                // identity;the daemon-side record of a forwarded nested leg
                // keys on the root id and the chain's depth exactly as the
                // broker-side record of the in-broker leg does.

                chain
            }
            None => EvidenceChain::root(
                request.invocation_id.clone(),
                request
                    .context
                    .as_ref()
                    .map(|context| context.initiating_identity.clone())
                    .unwrap_or_else(|| "daemon".to_owned()),
            ),
        };
        let chain_bearing = request.chain_identities.is_some();
        if !request_fds_admitted(&request, fds.as_slice()) {
            // The declared leg and the attached leg disagree - not in the
            // count, not in index order, not in kernel kind - so the call is
            // refused with the carrier's own fd-leg code rather than letting
            // an anonymous truncation pass as the invocation.
            let response = refused(FD_LEG);
            self.record_chain(self.chain_record(
                if chain_bearing {
                    ChainRecordClass::Correlation
                } else {
                    ChainRecordClass::Root
                },
                &chain,
                &request.operation,
                &request.zone,
                &response,
            ))
            .await;
            return connection
                .write_frame_with_fds(&encode_reply(&response)?, &[], FORWARD_REPLY_DEADLINE)
                .await;
        }
        // The attestation, when the request rides one: the block is
        // broker-minted, so the field-wise freshness comparison against this
        // process's current values is the whole admission - a stale epoch, a
        // Zone not bound to the call, an older provider-set revision or
        // generation, or a mutated block is refused with the stale-context
        // code before any handler runs. The block's deadline budget replaces
        // the fixed handler deadline for the call, already checked under the
        // shared ceiling by the admission.
        let handler_deadline = match request.context.as_ref() {
            Some(context) => {
                if !self.context_admitted(context, &request.zone).await {
                    tracing::warn!(
                        operation = %request.operation,
                        zone = %request.zone,
                        "forwarded call carries a stale or mismatched broker context; refusing"
                    );
                    let response = refused(STALE_CONTEXT);
                    self.record_chain(self.chain_record(
                        if chain_bearing {
                            ChainRecordClass::Correlation
                        } else {
                            ChainRecordClass::Root
                        },
                        &chain,
                        &request.operation,
                        &request.zone,
                        &response,
                    ))
                    .await;
                    return connection
                        .write_frame_with_fds(
                            &encode_reply(&response)?,
                            &[],
                            FORWARD_REPLY_DEADLINE,
                        )
                        .await;
                }
                Duration::from_millis(context.deadline_ms)
            }
            None => handler_deadline,
        };
        // The dispatch runs on its own task so a handler crash cannot
        // unwind through the accept loop: the task boundary catches the
        // panic (KTD7), and the call is refused by name with the envelope's
        // handler-crashed code instead of a dropped socket the broker would
        // read as a round-trip timeout. The call's artifacts move with it,
        // exactly as a spawned forward leg owns its frame.
        let operation = request.operation.clone();
        let zone = request.zone.clone();
        let rendezvous = Arc::clone(self);
        let chain_for_dispatch = chain.clone();
        let dispatch = tokio::spawn(async move {
            rendezvous
                .invoke(&request, fds.as_slice(), &chain_for_dispatch)
                .await
        });
        let abort = dispatch.abort_handle();
        let (response, response_fds) = match tokio::time::timeout(handler_deadline, dispatch).await {
            Ok(Ok(result)) => result,
            Ok(Err(join)) if join.is_panic() => {
                // The closing refusal of a crashed call: the panic is caught
                // at the task boundary and the caller is answered by name,
                // while the panic's message goes to the daemon's journal for
                // the operator following the refusal record.
                tracing::error!(
                    operation = %operation,
                    panic = %panic_message(join.into_panic()).unwrap_or_else(|| "(no message)".to_owned()),
                    "forwarded handler panicked; refusing the call"
                );
                (refused(HANDLER_CRASHED), Vec::new())
            }
            Ok(Err(join)) => {
                // A task that ended without a panic was cancelled, and the
                // only cancellation below fires after the deadline refusal
                // was already chosen, so this arm cannot normally be
                // reached; it refuses by name rather than unwinding.
                tracing::error!(
                    operation = %operation,
                    join = %join,
                    "forwarded dispatch task ended without a result; refusing the call"
                );
                (refused(HANDLER_CRASHED), Vec::new())
            }
            Err(_) => {
                // A handler that never finished is the daemon's answer to
                // give: the call is refused by name while the caller is still
                // listening, and the slot it held is free the moment this
                // call returns. The task is aborted so a non-yielding handler
                // cannot linger past its deadline on a worker.
                abort.abort();
                tracing::warn!(
                    operation = %operation,
                    "forwarded handler exceeded its deadline; refusing the call"
                );
                (refused(FORWARD_TIMEOUT), Vec::new())
            }
        };
        // The forwarded invocation's daemon-side record: exactly one root
        // record per root invocation, whatever the outcome - the leg
        // executing the root operation records its result or its refusal -
        // and a correlation record for every nested leg that re-presents
        // the root invocation id (KTD6).
        self.record_chain(self.chain_record(
            if chain_bearing {
                ChainRecordClass::Correlation
            } else {
                ChainRecordClass::Root
            },
            &chain,
            &operation,
            &zone,
            &response,
        ))
        .await;
        // The reply frame carries the descriptors the handler minted for
        // this invocation, index-aligned with the outcome's declarations
        // (U10); a response that mints none carries no attachments.
        let declared = response_fds.len();
        let reply = encode_reply(&response)?;
        // The frame writer takes the raw descriptors; the handler-minted
        // fds stay owned here and close with the reply scope.
        let response_raw_fds: Vec<i32> = response_fds
            .iter()
            .map(|fd| fd.as_raw_fd())
            .collect();
        let write = connection
            .write_frame_with_fds(&reply, &response_raw_fds, FORWARD_REPLY_DEADLINE)
            .await;
        if declared != 0 {
            tracing::debug!(
                operation = %operation,
                zone = %zone,
                descriptors = declared,
                "forwarded response carried minted descriptors"
            );
        }
        write
    }
}

/// The wire code one effect-service failure surfaces under.
///
/// KTD5's dedicated refusal: the service's generational revision moved past
/// the one the call started against, or the actor died under the in-flight
/// call - the call rode a stale generation and is refused by name, never
/// hung. An unbound service and an operation no hosted service declares are
/// the same refusal as an operation this process does not serve, and a
/// declined call crosses back under the taxonomy's handler-refused entry
/// (KTD7).
fn effect_refusal_code(error: &EffectServiceError) -> &'static str {
    match error {
        EffectServiceError::UnboundService { .. } => UNCOMMITTED_OPERATION,
        EffectServiceError::OperationUnserved { .. } => UNCOMMITTED_OPERATION,
        EffectServiceError::WrongZone { .. } => UNCOMMITTED_OPERATION,
        EffectServiceError::StaleRevision { .. } => STALE_REVISION,
        EffectServiceError::ServiceUnavailable { .. } => STALE_REVISION,
        EffectServiceError::InFlightStale { .. } => STALE_REVISION,
        EffectServiceError::Declined { .. } => HANDLER_REFUSED,
    }
}

/// Invoke one effect-service operation through its live hosting binding.
///
/// The operation resolved to this binding through the declaring service's
/// method facets, so the service is live and serves the call; the binding's
/// revision is captured when the call starts and re-checked at dispatch, so
/// a generation that moved past the call mid-flight is refused with the
/// dedicated stale-revision code (KTD5). The payload rides the carrier as
/// the canonical object the broker validated (R8) - the service receives
/// that object, never a byte fixture - and the actor's answer returns the
/// same way, with the descriptors it minted for its declared response leg.
async fn invoke_effect_service(
    binding: &EffectServiceBinding,
    request: &ForwardOperationRequest,
    fds: &[RawFd],
    kernel: Option<&KernelCaller>,
    resources: Option<ServiceResourceContext>,
    chain: &EvidenceChain,
) -> (ForwardOperationResponse, Vec<OwnedFd>) {
    // The distinct method this operation resolves to, for the operator
    // following the refusal records and for the capability facets the
    // invocation is built from; the declaration it resolved through named
    // it, and a binding carries the declaration.
    let Some(method) = binding
        .decl()
        .methods
        .iter()
        .find(|declared| declared.operation == Some(request.operation.as_str()))
    else {
        return (refused(UNCOMMITTED_OPERATION), Vec::new());
    };
    // The method's declared request-leg contract governs the attached
    // descriptors: a leg the method did not declare - count or kind - is
    // refused with the carrier's fd-leg code before any handler runs.
    if !request_fds_match_method(fds, method.request_fds) {
        return (refused(FD_LEG), Vec::new());
    }
    let Ok(bytes) = serde_json::to_vec(&request.payload) else {
        return (refused(INVALID_PAYLOAD), Vec::new());
    };
    let Ok(payload) = CanonicalJsonObject::parse(&bytes) else {
        return (refused(INVALID_PAYLOAD), Vec::new());
    };
    let call = ServiceCallData {
        zone: request.zone.clone(),
        invocation_id: request.invocation_id.clone(),
        payload,
        resources: resources.unwrap_or_else(ServiceResourceContext::fail_closed),
        method: *method,
        kernel: kernel.cloned(),
        request_fds: fds.to_vec(),
        chain_identities: chain.identities().to_vec(),
    };
    match binding.call_expected(binding.revision(), call).await {
        Ok(response) => {
            // The method's declared response-leg contract governs the
            // service's returned descriptors: a leg the method did not
            // declare - count or kind - is refused with the carrier's
            // fd-leg code rather than passed through.
            if !response_fds_match_method(&response.fds, method.response_fds) {
                drop(response);
                return (refused(FD_LEG), Vec::new());
            }
            let EffectResponse { payload, fds } = response;
            (result_response_with_service_fds(payload, &fds), fds)
        }
        Err(error) => {
            tracing::warn!(
                operation = %request.operation,
                zone = %request.zone,
                service = %binding.service(),
                method = method.name,
                error = %error,
                "effect-service call refused; its generation moved past the call"
            );
            (refused(effect_refusal_code(&error)), Vec::new())
        }
    }
}

/// Whether the attached request descriptors satisfy the method's declared
/// request-leg contract: count within the declared ceiling, and every
/// descriptor presenting the declared kernel kind (an `any` declaration
/// admits every kind).
fn request_fds_match_method(fds: &[RawFd], contract: MethodFdContract) -> bool {
    if fds.len() > contract.max_fds as usize {
        return false;
    }
    match contract.fd_kind.and_then(declared_fd_kind) {
        None => fds.is_empty(),
        Some(FdKind::Any) => true,
        Some(kind) => fds.iter().all(|fd| fd_kind_of(*fd) == Some(kind)),
    }
}

/// Whether the descriptors a service returned satisfy its method's declared
/// response-leg contract: count within the declared ceiling, and every
/// descriptor presenting the declared kernel kind.
fn response_fds_match_method(fds: &[OwnedFd], contract: MethodFdContract) -> bool {
    if fds.len() > contract.max_fds as usize {
        return false;
    }
    match contract.fd_kind.and_then(declared_fd_kind) {
        None => fds.is_empty(),
        Some(FdKind::Any) => true,
        Some(kind) => fds.iter().all(|fd| fd_kind_of(fd.as_raw_fd()) == Some(kind)),
    }
}

/// The carrier's kernel-kind vocabulary for one declared kind spelling
/// (the kebab-case `MethodFdContract` facet), when the spelling is a known
/// kind.
fn declared_fd_kind(kind: &str) -> Option<FdKind> {
    match kind {
        "fifo" => Some(FdKind::Fifo),
        "socket" => Some(FdKind::Socket),
        "char-device" => Some(FdKind::CharDevice),
        "block-device" => Some(FdKind::BlockDevice),
        "any" => Some(FdKind::Any),
        "regular" => Some(FdKind::Regular),
        "directory" => Some(FdKind::Directory),
        _ => None,
    }
}

/// The normal result reply of one effect-service invocation: the canonical
/// object the service returned plus the descriptors it minted, declared
/// index-aligned in frame order with their actual kernel kinds (the
/// broker's forwarder re-validates the leg against the row's declared facet
/// on its side).
fn result_response_with_service_fds(
    payload: CanonicalJsonObject,
    fds: &[OwnedFd],
) -> ForwardOperationResponse {
    let fd_indexes: Vec<u32> = (0..fds.len() as u32).collect();
    let fd_kinds: Vec<FdKind> = fds
        .iter()
        .map(|fd| fd_kind_of(fd.as_raw_fd()).unwrap_or(FdKind::Any))
        .collect();
    ForwardOperationResponse {
        outcome: ForwardOperationOutcome::Result {
            result: serde_json::to_value(&payload)
                .expect("canonical JSON objects always serialize"),
            fd_indexes,
            fd_kinds,
        },
    }
}

/// The normal result reply of one invocation whose handler minted
    /// descriptors (U10): the canonical object plus the descriptors over the
    /// carrier's response fd leg.
    ///
    /// The descriptors are the handler's own mints for this invocation; the
    /// reply declares them index-aligned in frame order, and the broker's
    /// forwarder re-validates the leg against the row's declared facet before
    /// the caller sees it.
    fn result_response_with_fds(
        result: OperationResult,
    ) -> (ForwardOperationResponse, Vec<OwnedFd>) {
        let (payload, fds) = result.into_parts();
        let fd_indexes: Vec<u32> = (0..fds.len() as u32).collect();
        // The family rows declare no fd facet of their own, so the returned
        // descriptors are labelled with the permissive kind; the broker's
        // forwarder re-checks the actual kernel kinds against the row's
        // declared facet on its side.
        let fd_kinds = vec![FdKind::Any; fds.len()];
        (
            ForwardOperationResponse {
                outcome: ForwardOperationOutcome::Result {
                    result: serde_json::to_value(&payload)
                        .expect("canonical JSON objects always serialize"),
                    fd_indexes,
                    fd_kinds,
                },
            },
            fds,
        )
    }

struct ScmFds(Vec<RawFd>);

impl ScmFds {

    fn new(fds: Vec<RawFd>) -> Self {
        Self(fds)
    }

    /// The received descriptors, borrowed across the invocation.
    fn as_slice(&self) -> &[RawFd] {
        &self.0
    }
}

impl Drop for ScmFds {

    fn drop(&mut self) {
        close_received_fds(&self.0);
    }
}

/// Whether one request's declared fd leg is admitted by the descriptors the
/// frame actually attached: count equal (never truncated), indexes in frame
/// order, kinds against the kernel stat of each received descriptor, andthe
/// whole leg within the carrier's frame ceiling.
fn request_fds_admitted(request: &ForwardOperationRequest, fds: &[RawFd]) -> bool {
    if request.fd_indexes.len() != request.fd_kinds.len() {
        return false;
    }
    if request.fd_indexes.len() > MAX_FRAME_FDS {
        return false;
    }
    if request.fd_indexes
        .iter()
        .enumerate()
        .any(|(position, declared)| *declared != position as u32)
    {
        return false;
    }
    if fds.len() != request.fd_indexes.len() {
        return false;
    }
    fds
        .iter()
        .zip(&request.fd_kinds)
        .all(|(fd, declared)| {
            // An `Any` declaration admits every descriptor regardless of
            // fstat kind (the mixed or anon-inode legs, U10).
            *declared == FdKind::Any || fd_kind_of(*fd) == Some(*declared)
        })
}

/// The kernel kind one descriptor presents, or None when its fstat reports
/// a kind the carrier vocabulary does not carry.
fn fd_kind_of(fd: RawFd) -> Option<FdKind> {
    let stat = nix::sys::stat::fstat(fd).ok()?;
    match stat.st_mode & nix::libc::S_IFMT {
        nix::libc::S_IFIFO => Some(FdKind::Fifo),
        nix::libc::S_IFSOCK => Some(FdKind::Socket),
        nix::libc::S_IFCHR => Some(FdKind::CharDevice),
        nix::libc::S_IFBLK => Some(FdKind::BlockDevice),
        nix::libc::S_IFREG => Some(FdKind::Regular),
        nix::libc::S_IFDIR => Some(FdKind::Directory),
        _ => None,
    }
}

/// One refused forwarded invocation, named.
fn refused(code: &str) -> ForwardOperationResponse {
    ForwardOperationResponse {
        outcome: ForwardOperationOutcome::Refused {
            code: code.to_owned(),
        },
    }
}

/// The message one dispatch panic carried, when it carried a message.
fn panic_message(payload: Box<dyn std::any::Any + Send>) -> Option<String> {
    match payload.downcast_ref::<&str>() {
        Some(message) => Some((*message).to_owned()),
        None => payload.downcast_ref::<String>().cloned(),
    }
}

/// Bind the rendezvous listener at `path`.
///
/// The socket's DAC posture is the public socket's - a seqpacket listener
/// owned by the daemon, mode 0660, chgrp'd to the socket group - but DAC is
/// not this endpoint's admission: that group carries every launcher and
/// admin, and a member that reached this socket would drive forwarded
/// operations the broker never authorized. The accepted peer is decided
/// per connection by `SO_PEERCRED` instead ([`ServingPosture`]).
pub(crate) fn bind(path: &Path, identity: &RuntimeIdentity) -> Result<Socket, TypedError> {
    bind_public_socket(path, identity)
}

/// The serving posture of one rendezvous loop: which peer identity the
/// endpoint accepts, how many calls may be in flight, and how long a handler
/// may run.
///
/// The accepted identity is the whole authorization of this hop. The call was
/// authorized at the broker against the committed rows, and the carrier
/// carries no caller identity, so a peer that is not the broker is a peer
/// this endpoint has nothing to check the call against. Accepted: the
/// privileged broker (uid 0 - both the host broker and a realm broker run as
/// root) and the daemon's own effective uid, which is the broker's identity
/// when both run under one unprivileged user, the test and
/// unprivileged-development shape. Nothing else is. In particular membership
/// of the public socket group - the group the socket is chgrp'd to, which
/// carries every launcher and admin - is not admission: a member that reached
/// this socket would drive provider operations with no broker authorization
/// and no broker audit record.
struct ServingPosture {
    /// The effective uid a peer must present to be admitted, beside the
    /// privileged broker's. `SO_PEERCRED` reports the peer's effective
    /// credentials, and the daemon's real and effective uids coincide (it
    /// either starts as its own user or drops to it with `setuid(2)`).
    accepted_uid: u32,
    /// The in-flight ceiling.
    max_inflight: usize,
    /// The handler deadline.
    handler_deadline: Duration,
}

impl ServingPosture {
    /// The daemon's production posture.
    fn production() -> Self {
        Self {
            accepted_uid: nix::unistd::geteuid().as_raw(),
            max_inflight: DEFAULT_MAX_INFLIGHT_CONNECTIONS,
            handler_deadline: FORWARD_HANDLER_DEADLINE,
        }
    }

    /// Whether one peer identity is the peer this endpoint serves.
    fn admits(&self, peer_uid: u32) -> bool {
        peer_uid == BROKER_UID || peer_uid == self.accepted_uid
    }
}

/// Serve accepted forwarded connections until the daemon exits.
///
/// The accept loop and every call it admits run as tasks on `runtime`: a call
/// holds one semaphore permit and no thread of its own, so the in-flight cap
/// bounds live calls rather than pinned threads.
pub(crate) fn spawn_server(
    rendezvous: Arc<ForwardRendezvous>,
    listener: Socket,
    runtime: tokio::runtime::Handle,
) -> Result<(), TypedError> {
    let listener = AsyncSeqpacket::register(listener)?;
    runtime.spawn(serve_accepted(
        rendezvous,
        listener,
        ServingPosture::production(),
    ));
    Ok(())
}

/// The accept loop: one task per admitted call, all of them on the runtime.
async fn serve_accepted(
    rendezvous: Arc<ForwardRendezvous>,
    listener: AsyncSeqpacket,
    posture: ServingPosture,
) {
    let admissions = Arc::new(Semaphore::new(posture.max_inflight));
    loop {
        let connection = match listener.accept().await {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(error = %error, "forward rendezvous accept failed; continuing");
                tokio::time::sleep(ACCEPT_ERROR_BACKOFF).await;
                continue;
            }
        };
        let connection = match AsyncSeqpacket::register(connection) {
            Ok(connection) => connection,
            Err(error) => {
                tracing::warn!(
                    reason = %error.message(),
                    "forward rendezvous call refused"
                );
                continue;
            }
        };
        // Authz-first: the peer is bound to its kernel identity before a
        // single frame is read, so a peer that is not the broker can neither
        // occupy a slot nor drive a forwarded operation.
        let peer_uid = match connection.peer_uid() {
            Ok(peer_uid) => peer_uid,
            Err(error) => {
                tracing::warn!(
                    reason = %error.message(),
                    "forward rendezvous could not read its peer's credentials; refusing the call"
                );
                refuse(&connection, UNGRANTED_CALLER).await;
                continue;
            }
        };
        if !posture.admits(peer_uid) {
            tracing::warn!(
                peer_uid,
                "forward rendezvous refused a peer that is not the broker"
            );
            refuse(&connection, UNGRANTED_CALLER).await;
            continue;
        }
        let Ok(permit) = Arc::clone(&admissions).try_acquire_owned() else {
            // The cap is the admission gate, and the carrier carries an
            // answer: the call is refused under the daemon's own capacity
            // code, so the broker reports that code rather than a handler it
            // never reached.
            tracing::warn!(
                peer_uid,
                max = posture.max_inflight,
                "forward rendezvous is at its in-flight cap; refusing the call"
            );
            refuse(&connection, TypedError::DaemonBusy.kind()).await;
            continue;
        };
        let rendezvous = Arc::clone(&rendezvous);
        tokio::spawn(async move {
            // The permit lives exactly as long as the call does, so the slot
            // is released by the call finishing - never by the accept loop.
            let _permit = permit;
            if let Err(error) = rendezvous
                .serve_connection(&connection, posture.handler_deadline)
                .await
            {
                tracing::warn!(
                    reason = %error.message(),
                    "forward rendezvous call refused"
                );
            }
        });
    }
}

/// Refuse one call this endpoint will not serve, by name.
///
/// The refusal is written before the peer's own frame is read - the peer may
/// not have sent one - and the pending input is drained, so the close that
/// follows is graceful and the refusal is what the caller receives.
async fn refuse(connection: &AsyncSeqpacket, code: &str) {
    match encode_reply(&refused(code)) {
        Ok(frame) => {
            if let Err(error) = connection.write_frame(&frame, FORWARD_REPLY_DEADLINE).await {
                tracing::warn!(
                    reason = %error.message(),
                    "forward rendezvous refusal not delivered"
                );
            }
        }
        Err(error) => {
            tracing::warn!(
                reason = %error.message(),
                "forward rendezvous refusal not encodable"
            );
        }
    }
    connection
        .drain_pending(FORWARD_REFUSAL_DRAIN_DEADLINE)
        .await;
}

/// One seqpacket endpoint registered with the reactor.
///
/// The listener is one of these, and so is every accepted connection: frame
/// reads and writes wait on readiness instead of on a blocking syscall, so
/// nothing about a call owns a thread. The session crate drives its own
/// seqpacket endpoints the same way, so the daemon has one pattern for kernel
/// I/O on an async path rather than a second one here.
pub(crate) struct AsyncSeqpacket {
    io: AsyncFd<Socket>,
}

impl AsyncSeqpacket {
    /// Register one socket with the reactor.
    ///
    /// The socket is switched to nonblocking mode first: the reactor owns
    /// readiness, and a blocking descriptor would stall the worker that
    /// awaited it.
    pub(crate) fn register(socket: Socket) -> Result<Self, TypedError> {
        socket
            .set_nonblocking(true)
            .map_err(|error| TypedError::InternalIo {
                context: "set forward rendezvous socket nonblocking".to_owned(),
                detail: error.to_string(),
                source: error_source(error),
            })?;
        let io = AsyncFd::new(socket).map_err(|error| TypedError::InternalIo {
            context: "register forward rendezvous socket".to_owned(),
            detail: error.to_string(),
            source: error_source(error),
        })?;
        Ok(Self { io })
    }

    /// The uid the peer presented when it connected.
    ///
    /// `SO_PEERCRED` is the kernel's answer - the peer is whatever the kernel
    /// says connected, not what a frame claims - which is why this endpoint
    /// reads it before the first frame.
    fn peer_uid(&self) -> Result<u32, TypedError> {
        getsockopt(self.io.get_ref(), sockopt::PeerCredentials)
            .map(|credentials| credentials.uid())
            .map_err(|error| TypedError::InternalIo {
                context: "read forward rendezvous peer credentials".to_owned(),
                detail: error.to_string(),
                source: error_source(error),
            })
    }

    /// Accept the next connection, waiting on the listener instead of polling
    /// it.
    async fn accept(&self) -> io::Result<Socket> {
        self.io
            .async_io(Interest::READABLE, |listener| {
                listener.accept().map(|(connection, _)| connection)
            })
            .await
    }

    /// Read one frame, waiting at most `deadline` for it to arrive.
    // R11 inventory note: genuinely synchronous path - the `nix::sys::socket`
    // recv runs inside the `AsyncFd::async_io` readiness closure on a
    // non-blocking descriptor (the clippy.toml replacement vocabulary names
    // this exact AsyncFd-over-raw-socket shape as the sanctioned seam); the
    // syscall never blocks because readiness was already observed.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    pub(crate) async fn read_frame(&self, deadline: Duration) -> Result<Vec<u8>, TypedError> {
        // The frame is length-prefixed, so peek the four-byte prefix and
        // size the datagram buffer from the declared length instead of the
        // ceiling: drain_pending reads up to four frames per refused call,
        // and the ceiling buffer is 1 MiB.
        let mut prefix = [0u8; 4];
        let peeked = match tokio::time::timeout(
            deadline,
            self.io.async_io(Interest::READABLE, |socket| {
                recv(socket.as_raw_fd(), &mut prefix, MsgFlags::MSG_PEEK)
                    .map_err(|errno| io::Error::from_raw_os_error(errno as i32))
            }),
        )
        .await
        {
            Ok(Ok(read)) => read,
            Ok(Err(error)) => return Err(recv_failure(error.to_string(), error_source(error))),
            Err(_) => return Err(recv_failure(format!("no frame within {deadline:?}"), None)),
        };
        if peeked < 4 {
            // A datagram shorter than the prefix is malformed; consume it
            // so the next read starts clean, refusing it exactly as the
            // ceiling-buffer read did.
            let mut short = [0u8; 4];
            let read = match tokio::time::timeout(deadline, self.recv_datagram(&mut short)).await {
                Ok(Ok(read)) => read,
                Ok(Err(error)) => return Err(recv_failure(error.to_string(), error_source(error))),
                Err(_) => return Err(recv_failure(format!("no frame within {deadline:?}"), None)),
            };
            return decode_frame(&short[..read]);
        }
        let declared = u32::from_le_bytes(prefix) as usize;
        if declared > MAX_FRAME_SIZE {
            return Err(TypedError::WireFrameTooLarge { declared });
        }
        let mut datagram = vec![0u8; declared + 5];
        let read = match tokio::time::timeout(deadline, self.recv_datagram(&mut datagram)).await {
            Ok(Ok(read)) => read,
            Ok(Err(error)) => return Err(recv_failure(error.to_string(), error_source(error))),
            Err(_) => return Err(recv_failure(format!("no frame within {deadline:?}"), None)),
        };
        decode_frame(&datagram[..read])
    }

    /// Write one frame, waiting at most `deadline` for the peer to take it.
    pub(crate) async fn write_frame(&self, body: &[u8], deadline: Duration) -> Result<(), TypedError> {
        let frame = encode_frame(body)?;
        let written = match tokio::time::timeout(deadline, self.send_datagram(&frame)).await {
            Ok(Ok(written)) => written,
            Ok(Err(error)) => return Err(send_failure(error.to_string(), error_source(error))),
            Err(_) => return Err(send_failure(format!("no write within {deadline:?}"), None)),
        };
        if written != frame.len() {
            return Err(send_failure(
                format!("short write: {written} of {}", frame.len()),
                None,
            ));
        }
        Ok(())
    }

    /// Read one frame and the descriptors its SCM_RIGHTS attachments carried,
    /// waiting at most `deadline` for it to arrive.
    ///
    /// A frame and its attachments arrive together or not at all, so the
    /// received descriptor count is exactly what the sender put on the
    /// carrier;an oversized cmsg set is capped by the kernel at the receive
    /// buffer's ceiling, which is why the caller-side declaration check
    /// refuses a count over that ceiling rather than let a truncation pass..
    async fn read_frame_with_fds(&self, deadline: Duration) -> Result<(Vec<u8>, Vec<RawFd>), TypedError> {
        // The blocking transport read the prefixed frame and stripped the
        // length prefix itself, so the returned body is already the frame
        // payload,length-checked and cmsg-truncation-checked.
        match tokio::time::timeout(deadline, self.recv_frame_with_fds()).await {
            Ok(Ok(pair)) => Ok(pair),
            Ok(Err(error)) => Err(recv_failure(error.to_string(), error_source(error))),
            Err(_) => Err(recv_failure(format!("no frame within {deadline:?}"), None)),
        }
    }

    /// Write one frame, attaching `fds` to it, waiting at most `deadline`
    /// for the peer to take it.
    async fn write_frame_with_fds(
        &self,
        body: &[u8],
        fds: &[RawFd],
        deadline: Duration,
    ) -> Result<(), TypedError> {
        // The transport writes the length prefix itself,so the body crosses
        // as-is;the receiving transport strips the same prefix back off..
        match tokio::time::timeout(deadline, self.send_datagram_with_fds(body, fds)).await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => Err(send_failure(error.to_string(), error_source(error))),
            Err(_) => Err(send_failure(format!("no write within {deadline:?}"), None)),
        }
    }

    /// One datagram read with its attachments, awaited for readiness. The
    /// blocking transport's `recvmsg` owns the control-message buffer for
    /// this read, and MSG_CMSG_CLOEXEC is set there, so the received descriptors
    /// arrive close-on-exec exactly as they do on the broker leg.
    async fn recv_frame_with_fds(&self) -> io::Result<(Vec<u8>, Vec<RawFd>)> {
        self.io
            .async_io(Interest::READABLE, |socket| {
                read_frame_with_fds(socket)
                    .map_err(|error| io::Error::other(format!("{error:?}")))
            })
            .await
    }

    /// One datagram write with its attachments, awaited for readiness.
    async fn send_datagram_with_fds(&self, frame: &[u8], fds: &[RawFd]) -> io::Result<()> {
        self.io
            .async_io(Interest::WRITABLE, |socket| {
                write_frame_with_fds(socket, frame, fds)
                    .map_err(|error| io::Error::other(format!("{error:?}")))
            })
            .await
    }

    /// One datagram read, awaited for readiness.
    // R11 inventory note: genuinely synchronous path - the `nix::sys::socket`
    // recv runs inside the `AsyncFd::async_io` readiness closure on a
    // non-blocking descriptor (the clippy.toml replacement vocabulary names
    // this exact AsyncFd-over-raw-socket shape as the sanctioned seam); the
    // syscall never blocks because readiness was already observed.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    async fn recv_datagram(&self, datagram: &mut [u8]) -> io::Result<usize> {
        self.io
            .async_io(Interest::READABLE, |socket| {
                recv(socket.as_raw_fd(), &mut datagram[..], MsgFlags::empty())
                    .map_err(|errno| io::Error::from_raw_os_error(errno as i32))
            })
            .await
    }

    /// One datagram write, awaited for readiness.
    // R11 inventory note: genuinely synchronous path - the `nix::sys::socket`
    // send runs inside the `AsyncFd::async_io` readiness closure on a
    // non-blocking descriptor (the clippy.toml replacement vocabulary names
    // this exact AsyncFd-over-raw-socket shape as the sanctioned seam); the
    // syscall never blocks because readiness was already observed.
    #[allow(clippy::disallowed_methods, reason = "synchronous path")]
    async fn send_datagram(&self, frame: &[u8]) -> io::Result<usize> {
        self.io
            .async_io(Interest::WRITABLE, |socket| {
                send(socket.as_raw_fd(), frame, MsgFlags::empty())
                    .map_err(|errno| io::Error::from_raw_os_error(errno as i32))
            })
            .await
    }

    /// Consume what a refused peer already sent, so the close that follows is
    /// graceful and the refusal arrives. Bounded: the loop stops at the first
    /// error, which includes the drain deadline.
    async fn drain_pending(&self, deadline: Duration) {
        for _ in 0..4 {
            if self.read_frame(deadline).await.is_err() {
                return;
            }
        }
    }
}

/// The failure of a frame read, in the vocabulary the blocking transport uses
/// for the same syscall. `source` is the origin error when the read failed on
/// one; a deadline that elapsed without a frame has none.
fn recv_failure(detail: String, source: Option<ErrorSource>) -> TypedError {
    TypedError::InternalIo {
        context: "recv seqpacket frame".to_owned(),
        detail,
        source,
    }
}

/// The failure of a frame write, in the vocabulary the blocking transport
/// uses for the same syscall. `source` is the origin error when the write
/// failed on one; a deadline that elapsed without a write has none.
fn send_failure(detail: String, source: Option<ErrorSource>) -> TypedError {
    TypedError::InternalIo {
        context: "send seqpacket frame".to_owned(),
        detail,
        source,
    }
}

/// Encode one reply frame body: the same serde spelling the blocking
/// transport wrote for this endpoint.
fn encode_reply(response: &ForwardOperationResponse) -> Result<Vec<u8>, TypedError> {
    serde_json::to_vec(response).map_err(|error| TypedError::InternalIo {
        context: "serialize JSON frame".to_owned(),
        detail: error.to_string(),
        source: error_source(error),
    })
}

/// Encode one frame: the four-byte little-endian length prefix the broker's
/// forwarder writes, unchanged.
fn encode_frame(body: &[u8]) -> Result<Vec<u8>, TypedError> {
    if body.len() > MAX_FRAME_SIZE {
        return Err(TypedError::WireFrameTooLarge {
            declared: body.len(),
        });
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_le_bytes());
    frame.extend_from_slice(body);
    Ok(frame)
}

/// Decode one received frame, refusing exactly what the blocking transport
/// refuses.
fn decode_frame(datagram: &[u8]) -> Result<Vec<u8>, TypedError> {
    if datagram.is_empty() {
        return Err(recv_failure("peer closed the socket".to_owned(), None));
    }
    if datagram.len() < 4 {
        return Err(TypedError::WireInvalidFrame {
            detail: format!("frame too short: {} bytes", datagram.len()),
        });
    }
    let declared = u32::from_le_bytes(datagram[..4].try_into().expect("prefix slice")) as usize;
    if declared > MAX_FRAME_SIZE {
        return Err(TypedError::WireFrameTooLarge { declared });
    }
    if datagram.len() - 4 != declared {
        return Err(TypedError::WireInvalidFrame {
            detail: format!(
                "declared {declared} bytes but received {}",
                datagram.len() - 4
            ),
        });
    }
    Ok(datagram[4..].to_vec())
}

#[cfg(test)]
mod tests {
    use d2b_audit::evidence_chain::root_record_count;
    use std::sync::LazyLock;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use d2b_contracts_resource::v3::canonical_json_bytes;
    use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ProcessSpec};
    use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceUid, ZoneId};
    use d2b_process_conformance::AdoptionCandidate;
    use d2b_provider_process::{
        ExecutionMode, INVALID_PROCESS_TYPE, ProcessDriverArgs, ProcessEffectFacets,
        ProcessProviderRuntime, ProcessResourceContext, ProviderAdoption, ProviderLaunch,
        ProviderLiveness, process_family_descriptors,
    };
    use d2b_resource_runtime::context::{ManagerEndpoint, ServiceResourceContext, SpecDecoder};
    use d2b_resource_runtime::driver::{DynResourceDriver, ResourceDriverFactory};
    use d2b_resource_runtime::identity::{ResourceKey, ResourceTypeName};
    use d2b_resource_types::{
        AllowedSources, DriverDescriptor, OperationCtx, OperationDef, OperationFailure,
        OperationHandler, OperationResult, ServiceDecl, ServiceMethod, ValidatedPayload,
        WellKnownType,
    };
    use d2bd_runtime::target_runtime::DaemonMode;
    use d2bd_runtime::unix_transport::{
        connect_seqpacket, read_frame, read_frame_with_fds, write_frame,
    };
    use tokio::sync::{Notify, Semaphore};

    use super::*;
    use crate::effect_service_actors::EffectServiceRow;
    use crate::provider_lifecycle::{ProviderSet, family_declaration};
    use d2b_provider_toolkit::{
        EffectResponse, EffectService, EffectServiceError, EffectServiceFactory,
        MethodFdContract, ServiceInvocation,
    };

    /// A runtime facet that refuses every effect: the pilot operation answers
    /// from the family's declaration alone, so an effect call would fail
    /// this test loudly instead of passing unnoticed. The reconciliation
    /// surface is unreachable in these tests.
    struct RefusingEffects;

    #[async_trait::async_trait]
    impl ProcessProviderRuntime for RefusingEffects {
        fn bundle(&self) -> &d2b_core::bundle_resolver::BundleResolver {
            unreachable!("the rendezvous tests never reconcile a row")
        }

        fn socket_runtime_dir(&self) -> &std::path::Path {
            unreachable!("the rendezvous tests never reconcile a row")
        }

        fn guest_setup_descriptor_digest(
            &self,
            _zone: &ZoneId,
            _guest_ref: &ResourceRef,
        ) -> Option<d2b_contracts_resource::v3::SchemaFingerprint> {
            None
        }

        async fn resolve_device_worker_launch(
            &self,
            _ctx: &mut d2b_resource_runtime::context::ResourceContext,
            _identity: &d2b_provider_process::ProcessResourceIdentity,
            _spec: &d2b_provider_process::ProcessFamilySpec,
        ) -> Result<Option<d2b_provider_process::DeviceWorkerLaunch>, &'static str> {
            Err("refused")
        }

        async fn launch_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &ProcessSpec,
            _timeout: Duration,
        ) -> Result<ProviderLaunch, String> {
            Err("refused".to_owned())
        }

        async fn launch_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &EphemeralProcessSpec,
            _timeout: Duration,
        ) -> Result<ProviderLaunch, String> {
            Err("refused".to_owned())
        }

        async fn adopt_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &ProcessSpec,
        ) -> Result<ProviderAdoption, String> {
            Err("refused".to_owned())
        }

        async fn probe_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &ProcessSpec,
        ) -> Result<ProviderLiveness, String> {
            Err("refused".to_owned())
        }

        async fn adopt_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &EphemeralProcessSpec,
        ) -> Result<ProviderAdoption, String> {
            Err("refused".to_owned())
        }

        async fn probe_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &EphemeralProcessSpec,
        ) -> Result<ProviderLiveness, String> {
            Err("refused".to_owned())
        }

        async fn stop_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &ProcessSpec,
            _term_timeout: Duration,
            _kill_timeout: Duration,
        ) -> Result<bool, String> {
            Err("refused".to_owned())
        }

        async fn stop_ephemeral_resource(
            &self,
            _context: ProcessResourceContext<'_>,
            _spec: &EphemeralProcessSpec,
            _term_timeout: Duration,
            _kill_timeout: Duration,
        ) -> Result<bool, String> {
            Err("refused".to_owned())
        }

        async fn stop_stale_resource(
            &self,
            _provider_ref: &ResourceRef,
            _candidate: &AdoptionCandidate,
        ) -> Result<(), String> {
            Err("refused".to_owned())
        }

        async fn finalize_resource(
            &self,
            _context: ProcessResourceContext<'_>,
        ) -> Result<(), String> {
            Err("refused".to_owned())
        }

        fn has_active_resource_in_zone(
            &self,
            _zone: &ZoneId,
            _zone_uid: Option<&ResourceUid>,
            _resource_ref: &ResourceRef,
        ) -> bool {
            false
        }
    }

    /// The facet set the rendezvous tests build the family's driver from.
    fn refusing_facets() -> ProcessEffectFacets {
        ProcessEffectFacets {
            runtime: Arc::new(RefusingEffects),
            committed: None,
            guest_owners: None,
        }
    }

    /// The socket identity the test binds under: the caller's own uid/gid,
    /// with the production root-owned-parent check off, exactly as the
    /// daemon's unprivileged test mode resolves it.
    fn test_identity() -> RuntimeIdentity {
        RuntimeIdentity {
            daemon_uid: nix::unistd::getuid(),
            daemon_gid: nix::unistd::getgid(),
            public_socket_gid: nix::unistd::getgid(),
            unsafe_local_helper_socket_gid: None,
            expect_root_owned_parent: false,
        }
    }

    /// The stall operations the tests below forward to, beside the process
    /// family's own pilot operation. Each gated stall has its own latch, so
    /// two tests that run at once never share one.
    const STALL_THREADS: &str = "stall-threads";
    const STALL_CAPACITY: &str = "stall-capacity";
    const STALL_FOREVER: &str = "stall-forever";

    /// The latch one gated stall waits on: the handler counts itself in and
    /// then waits until the test releases it, so a call can be held inside
    /// the daemon while the test looks at the daemon.
    struct StallGate {
        entered: AtomicUsize,
        release: Semaphore,
    }

    impl StallGate {
        const fn new() -> Self {
            Self {
                entered: AtomicUsize::new(0),
                release: Semaphore::const_new(0),
            }
        }

        /// Hold the calling handler until the test releases the stall.
        async fn hold(&self) {
            self.entered.fetch_add(1, Ordering::AcqRel);
            let permit = self
                .release
                .acquire()
                .await
                .expect("the release latch is never closed");
            permit.forget();
        }

        /// Wait until `count` calls are held inside the handler.
        async fn wait_for(&self, count: usize) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while self.entered.load(Ordering::Acquire) < count {
                assert!(
                    Instant::now() < deadline,
                    "only {} of {count} calls reached the handler",
                    self.entered.load(Ordering::Acquire)
                );
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        }

        /// Release `count` held calls.
        fn release(&self, count: usize) {
            self.release.add_permits(count);
        }
    }

    static THREADS_GATE: StallGate = StallGate::new();
    static CAPACITY_GATE: StallGate = StallGate::new();

    /// A handler that holds its call on the gate until the test releases it.
    struct GatedHandler {
        gate: &'static StallGate,
        /// When present, records the thread that executed the call before
        /// holding it: the concurrent-calls test counts the distinct
        /// threads across the held calls.
        seen: Option<&'static tokio::sync::Mutex<Vec<std::thread::ThreadId>>>,
    }

    #[async_trait::async_trait]
    impl OperationHandler for GatedHandler {
        async fn execute(
            &self,
            _ctx: OperationCtx<'_>,
            _payload: ValidatedPayload,
        ) -> Result<OperationResult, OperationFailure> {
            if let Some(seen) = self.seen {
                seen.lock().await.push(std::thread::current().id());
            }
            self.gate.hold().await;
            Ok(stall_result())
        }
    }

    /// A handler that never finishes: only the rendezvous's own handler
    /// deadline can answer a call it is handed.
    struct StalledHandler;

    #[async_trait::async_trait]
    impl OperationHandler for StalledHandler {
        async fn execute(
            &self,
            _ctx: OperationCtx<'_>,
            _payload: ValidatedPayload,
        ) -> Result<OperationResult, OperationFailure> {
            std::future::pending::<Result<OperationResult, OperationFailure>>().await
        }
    }

    /// The threads that executed the held stall-threads calls, one entry
    /// per call, recorded at handler entry. This recorder is this test's
    /// own: only the calls the concurrent-calls test forwards execute the
    /// stall-threads handler, so nothing another test does can inflate it.
    static THREADS_SEEN: tokio::sync::Mutex<Vec<std::thread::ThreadId>> =
        tokio::sync::Mutex::const_new(Vec::new());

    static THREADS_HANDLER: GatedHandler = GatedHandler {
        gate: &THREADS_GATE,
        seen: Some(&THREADS_SEEN),
    };
    static CAPACITY_HANDLER: GatedHandler = GatedHandler {
        gate: &CAPACITY_GATE,
        seen: None,
    };
    static STALLED_HANDLER: StalledHandler = StalledHandler;

    /// A handler that panics mid-dispatch: only the crash guard can answer a
    /// call it is handed - the panic must die at the task boundary and the
    /// call must be refused by name instead of dropped.
    struct PanicHandler;

    #[async_trait::async_trait]
    impl OperationHandler for PanicHandler {
        async fn execute(
            &self,
            _ctx: OperationCtx<'_>,
            _payload: ValidatedPayload,
        ) -> Result<OperationResult, OperationFailure> {
            panic!("injected forward-leg handler crash")
        }
    }

    static PANIC_HANDLER: PanicHandler = PanicHandler;

    /// A handler whose work outruns the historical fixed 25 s deadline: the
    /// context budget (the Extended row's) must let it finish rather than
    /// aborting the call at the tier-less constant.
    struct Slow30sHandler;

    #[async_trait::async_trait]
    impl OperationHandler for Slow30sHandler {
        async fn execute(
            &self,
            _ctx: OperationCtx<'_>,
            _payload: ValidatedPayload,
        ) -> Result<OperationResult, OperationFailure> {
            tokio::time::sleep(Duration::from_secs(30)).await;
            Ok(stall_result())
        }
    }

    static SLOW_30S_HANDLER: Slow30sHandler = Slow30sHandler;

    /// A handler that runs and fails: its failure code must cross back to the
    /// caller under its own name, never flattened into the missing-handler
    /// refusal.
    struct ErroringHandler;

    #[async_trait::async_trait]
    impl OperationHandler for ErroringHandler {
        async fn execute(
            &self,
            _ctx: OperationCtx<'_>,
            _payload: ValidatedPayload,
        ) -> Result<OperationResult, OperationFailure> {
            Err(OperationFailure::new("handler-errored"))
        }
    }

    static ERRORING_HANDLER: ErroringHandler = ErroringHandler;

    /// A handler that reads the descriptor the carrier attached to its
    /// call. The forwarded request leg carries the caller's descriptor over
    /// SCM_RIGHTS;the rendezvous validates it against the wire declarations
    /// and hands it to the declared handler, so this handler reading it back
    /// proves the round trip through the real socket and the provider envelope.to
    struct FdEchoHandler;

    #[async_trait::async_trait]
    impl OperationHandler for FdEchoHandler {
        async fn execute(
            &self,
            ctx: OperationCtx<'_>,
            _payload: ValidatedPayload,
        ) -> Result<OperationResult, OperationFailure> {
            use std::os::fd::AsRawFd;
            use nix::unistd::read;
            let fd = ctx.fds.first().ok_or_else(|| OperationFailure::new(FD_LEG))?;
            let mut buf = [0_u8; 4];
            let n = read(fd.as_raw_fd(), &mut buf)
                .map_err(|error| OperationFailure::with_detail(FD_LEG, error.to_string()))?;
            let bytes = buf[..n].to_vec();
            let result = CanonicalJsonObject::parse(
                &canonical_json_bytes(&serde_json::json!({ "read": String::from_utf8_lossy(&bytes).to_string() }))
                    .expect("the read-back result is canonical JSON"),
            )
            .expect("the read-back result is a JSON object");
            Ok(OperationResult::new(result))
        }
    }

    static FD_ECHO_HANDLER: FdEchoHandler = FdEchoHandler;

    /// The stall operations plus the fd-echo operation the fixture's
    /// EphemeralProcess driver declares.
    static STALL_OPERATIONS: LazyLock<[OperationDef; 7]> = LazyLock::new(|| {
        [
            OperationDef {
                operation_ref: operation_ref(STALL_THREADS),
                handler: &THREADS_HANDLER,
            },
            OperationDef {
                operation_ref: operation_ref(STALL_CAPACITY),
                handler: &CAPACITY_HANDLER,
            },
            OperationDef {
                operation_ref: operation_ref(STALL_FOREVER),
                handler: &STALLED_HANDLER,
            },
            OperationDef {
                operation_ref: operation_ref("fd-echo"),
                handler: &FD_ECHO_HANDLER,
            },
            OperationDef {
                operation_ref: operation_ref("panic-boom"),
                handler: &PANIC_HANDLER,
            },
            OperationDef {
                operation_ref: operation_ref("slow-30s"),
                handler: &SLOW_30S_HANDLER,
            },
            OperationDef {
                operation_ref: operation_ref("error-boom"),
                handler: &ERRORING_HANDLER,
            },
        ]
    });

    /// One canonical operation reference, stated the way the declared tables
    /// state them.
    fn operation_ref(name: &str) -> ResourceRef {
        ResourceRef::parse(&format!("Operation/{name}")).expect("a canonical operation reference")
    }

    /// The result a released stall answers with.
    fn stall_result() -> OperationResult {
        let bytes = canonical_json_bytes(&serde_json::json!({ "stalled": true }))
            .expect("the stall result is canonical JSON");
        OperationResult::new(
            CanonicalJsonObject::parse(&bytes).expect("the stall result is a JSON object"),
        )
    }

    /// One started Zone whose `Process` family declares the pilot operation
    /// and whose EphemeralProcess driver carries the stall operations, served
    /// by a rendezvous on a real socket.
    struct ServingRendezvous {
        socket_path: PathBuf,
        _scratch: tempfile::TempDir,
        _providers: Arc<ProviderRuntime>,
        /// The serving rendezvous itself, so a test can attach audit sinks
        /// and drive nested calls at the seam.
        rendezvous: Arc<ForwardRendezvous>,
    }

    impl ServingRendezvous {
        /// The rendezvous the daemon starts: the production entry point, with
        /// the production cap, handler deadline, and accepted peer identity.
        async fn start() -> Self {
            Self::served_by(|rendezvous, listener| {
                spawn_server(rendezvous, listener, tokio::runtime::Handle::current())
            })
            .await
        }

        /// The same rendezvous with the posture a test needs, so a saturated
        /// cap, a stalled handler, and a peer identity that is not the
        /// dialing process's are reachable without the production numbers.
        async fn start_with(posture: ServingPosture) -> Self {
            Self::served_by(move |rendezvous, listener| {
                let listener = AsyncSeqpacket::register(listener)?;
                tokio::spawn(serve_accepted(rendezvous, listener, posture));
                Ok(())
            })
            .await
        }

        /// The same rendezvous bound with the attestation state a context
        /// test needs: the daemon's current generations for the Zone and the
        /// broker epoch its last publication acknowledged.
        async fn start_attesting() -> Self {
            Self::served_by(move |rendezvous, listener| {
                rendezvous.publish_generations("test", 1, 1);
                rendezvous.set_broker_epoch(5);
                spawn_server(rendezvous, listener, tokio::runtime::Handle::current())
            })
            .await
        }

        async fn served_by<F>(serve: F) -> Self
        where
            F: FnOnce(Arc<ForwardRendezvous>, Socket) -> Result<(), TypedError>,
        {
            let (rendezvous, socket_path, scratch, providers) = Self::fixture().await;
            let listener = bind(&socket_path, &test_identity()).expect("bind the rendezvous");
            serve(Arc::clone(&rendezvous), listener).expect("start the rendezvous server");
            Self {
                socket_path,
                _scratch: scratch,
                _providers: providers,
                rendezvous,
            }
        }

        /// The same rendezvous over a set that hosts a fixture effect
        /// service (U8, KTD5): the composition point hosts the declared
        /// service, and the forwarded calls below reach the actor through
        /// the live hosting binding, served by the production entry point.
        async fn start_with_effect_service() -> Self {
            Self::effect_served_by(Arc::new(EchoFactory), |rendezvous, listener| {
                spawn_server(rendezvous, listener, tokio::runtime::Handle::current())
            })
            .await
        }

        /// The same rendezvous over a set that hosts one pre-built fixture
        /// service (gated/declining fixtures).
        async fn start_with_effect_service_factory(
            factory: Arc<dyn EffectServiceFactory>,
        ) -> Self {
            Self::effect_served_by(factory, |rendezvous, listener| {
                spawn_server(rendezvous, listener, tokio::runtime::Handle::current())
            })
            .await
        }

        /// The same rendezvous over a set that hosts the process-systemd
        /// effects service (U15): the forwarded family operations resolve
        /// to the hosted actor through the declared operation facets, and
        /// the pidfd-minting calls reach the crate's committed handler
        /// table over the invocation's kernel seam.
        async fn start_with_systemd_effects_service() -> Self {
            Self::served_by_services(
                &[PROCESS_SYSTEMD_EFFECTS_SERVICE],
                Arc::new(SystemdEffectsServiceFactory::new()),
                |rendezvous, listener| {
                    spawn_server(rendezvous, listener, tokio::runtime::Handle::current())
                },
            )
            .await
        }

        /// The same rendezvous over a set that hosts the given declared
        /// services with one factory each (U3 fixtures: fd-leg and
        /// driver-context services).
        async fn start_serving(
            services: &'static [ServiceDecl],
            factory: Arc<dyn EffectServiceFactory>,
        ) -> Self {
            Self::served_by_services(services, factory, |rendezvous, listener| {
                spawn_server(rendezvous, listener, tokio::runtime::Handle::current())
            })
            .await
        }

        async fn effect_served_by<F>(factory: Arc<dyn EffectServiceFactory>, serve: F) -> Self
        where
            F: FnOnce(Arc<ForwardRendezvous>, Socket) -> Result<(), TypedError>,
        {
            let (rendezvous, socket_path, scratch, providers) =
                effect_fixture_with(factory).await;
            let listener = bind(&socket_path, &test_identity()).expect("bind the rendezvous");
            serve(Arc::clone(&rendezvous), listener).expect("start the rendezvous server");
            Self {
                socket_path,
                _scratch: scratch,
                _providers: providers,
                rendezvous,
            }
        }

        async fn served_by_services<F>(
            services: &'static [ServiceDecl],
            factory: Arc<dyn EffectServiceFactory>,
            serve: F,
        ) -> Self
        where
            F: FnOnce(Arc<ForwardRendezvous>, Socket) -> Result<(), TypedError>,
        {
            let (rendezvous, socket_path, scratch, providers) =
                effect_fixture_serving(services, factory).await;
            let listener = bind(&socket_path, &test_identity()).expect("bind the rendezvous");
            serve(Arc::clone(&rendezvous), listener).expect("start the rendezvous server");
            Self {
                socket_path,
                _scratch: scratch,
                _providers: providers,
                rendezvous,
            }
        }

        async fn fixture() -> (
            Arc<ForwardRendezvous>,
            PathBuf,
            tempfile::TempDir,
            Arc<ProviderRuntime>,
        ) {
            let zone = ZoneId::parse("test").expect("the test zone label is canonical");
            let scratch = tempfile::tempdir().expect("test scratch");
            let [process, ephemeral] = process_family_descriptors(ProcessDriverArgs {
                zone: zone.clone(),
                facets: refusing_facets(),
                zone_uid: None,
                policy_revision: None,
                provider_assignment_generation: None,
                controller_generation: ControllerGeneration::new(1)
                    .expect("the test generation is canonical"),
                guest_execution: None,
                mode: ExecutionMode::Host,
            });
            let mut set = ProviderSet::new(zone.clone(), scratch.path().to_path_buf())
                .with(
                    family_declaration("process"),
                    vec![
                        process,
                        DriverDescriptor {
                            operations: &STALL_OPERATIONS[..],
                            ..ephemeral
                        },
                    ],
                );
            // The U15 hosting pass publishes every registered family's
            // service that no driver in this set declared; the fixture set
            // declares only the process family's driver, so the remaining
            // registered services receive the echo fixture factory too -
            // they are hosted but never called by these tests.
            for registration in crate::resource_plane_v3::PROVIDER_REGISTRATIONS {
                for &service in registration.services {
                    set = set.with_effect_service_factory(service, Arc::new(EchoFactory));
                }
            }
            let providers = set
                .start()
                .await
                .expect("the process family starts through the base");
            let providers = Arc::new(providers);
            let rendezvous = Arc::new(ForwardRendezvous::new());
            rendezvous
                .publish(zone.as_str(), Arc::clone(&providers))
                .await;
            let socket_path = scratch.path().join("d2bd-forward.sock");
            (rendezvous, socket_path, scratch, providers)
        }
    }

    /// One started Zone whose fixture provider hosts a declared effect
    /// service (U8, KTD5), served by a rendezvous on a real socket. The
    /// fixture driver declares the service and nothing else, so the effect
    /// tests cannot accidentally route to a provider operation handler.
    async fn effect_fixture_with(
        factory: Arc<dyn EffectServiceFactory>,
    ) -> (
        Arc<ForwardRendezvous>,
        PathBuf,
        tempfile::TempDir,
        Arc<ProviderRuntime>,
    ) {
        effect_fixture_serving(&[ECHO_SERVICE], factory).await
    }

    /// The same fixture over the given declared services, one factory per
    /// service (U3: the fd-leg and driver-context services declare their
    /// own methods).
    async fn effect_fixture_serving(
        services: &'static [ServiceDecl],
        factory: Arc<dyn EffectServiceFactory>,
    ) -> (
        Arc<ForwardRendezvous>,
        PathBuf,
        tempfile::TempDir,
        Arc<ProviderRuntime>,
    ) {
        let zone = ZoneId::parse("test").expect("the test zone label is canonical");
        let scratch = tempfile::tempdir().expect("test scratch");
        let mut set = ProviderSet::new(zone.clone(), scratch.path().to_path_buf())
            .with(family_declaration("fixture"), vec![effect_descriptor(services)]);
        for service in services {
            set = set.with_effect_service_factory(service.id, Arc::clone(&factory));
        }
        // The U15 hosting pass publishes every registered family's service
        // that no driver in this set declared; the fixture set declares
        // only the services under test, so the remaining registered
        // services receive the fixture factory too - they are hosted but
        // never called by these tests.
        for registration in crate::resource_plane_v3::PROVIDER_REGISTRATIONS {
            for &service in registration.services {
                if !services.iter().any(|declared| declared.id == service) {
                    set = set.with_effect_service_factory(service, Arc::clone(&factory));
                }
            }
        }
        let providers = set
            .start()
            .await
            .expect("the fixture provider starts through the base");
        let providers = Arc::new(providers);
        let rendezvous = Arc::new(ForwardRendezvous::new());
        rendezvous
            .publish(zone.as_str(), Arc::clone(&providers))
            .await;
        let socket_path = scratch.path().join("d2bd-forward.sock");
        (rendezvous, socket_path, scratch, providers)
    }

    /// Forward one invocation the way the broker's forwarder does, driven on
    /// the runtime rather than on a thread the test owns: one connection, one
    /// canonical request frame, one awaited reply frame.
    async fn forward_async(
        socket_path: PathBuf,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
    ) -> ForwardOperationResponse {
        forward_request_async(socket_path, ForwardOperationRequest {
            chain_identities: None,
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload,
            context: None,
            fd_indexes: vec![],
            fd_kinds: vec![],
        })
        .await
    }

    /// Forward one explicit request frame, so a test controls the invocation
    /// id and the broker-attested context its call carries.
    async fn forward_request_async(
        socket_path: PathBuf,
        request: ForwardOperationRequest,
    ) -> ForwardOperationResponse {
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        let socket = connect_seqpacket(&socket_path).expect("dial the rendezvous");
        let connection =
            AsyncSeqpacket::register(Socket::from(socket)).expect("register the forwarded call");
        let deadline = Duration::from_secs(10);
        // A refused peer is answered before its frame is read: the server
        // writes the refusal, drains briefly, and closes, so a write that
        // races the close can fail with EPIPE even though the refusal is
        // already queued for this socket. Read the queued refusal instead
        // of failing the call; only a write error with no reply is a
        // failure.
        if let Err(write_error) = connection.write_frame(&encoded, deadline).await {
            let frame = connection.read_frame(deadline).await;
            if let Ok(frame) = frame {
                return serde_json::from_slice(&frame)
                    .expect("the reply is a ForwardOperationResponse");
            }
            panic!("write the request frame: {write_error:?}");
        }
        let frame = connection
            .read_frame(deadline)
            .await
            .expect("read the reply frame");
        serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse")
    }

    /// The request the broker mints for one attested call, with the fields a
    /// context test controls.
    fn attested_request(
        invocation_id: &str,
        operation: &str,
        initiating_identity: &str,
    ) -> ForwardOperationRequest {
        ForwardOperationRequest {
            chain_identities: None,
            operation: operation.to_owned(),
            zone: "test".to_owned(),
            invocation_id: invocation_id.to_owned(),
            payload: serde_json::json!({}),
            // Matches the attestation state `start_attesting` publishes:
            // epoch 5, revision 1, generations (1, 1), default budget.
            context: Some(ForwardContext {
                broker_epoch: 5,
                zone: "test".to_owned(),
                provider_set_revision: 1,
                controller_generation: 1,
                guest_generation: 1,
                initiating_identity: initiating_identity.to_owned(),
                deadline_ms: DEFAULT_CONTEXT_DEADLINE_MS,
            }),
            fd_indexes: vec![],
            fd_kinds: vec![],
        }
    }

    /// An in-memory chain audit sink the tests assert on: every record the
    /// daemon-side leg writes lands here.
    ///
    /// Synchronous by construction (the `ChainAuditSink` trait surface is
    /// sync), so it stays a `std::sync::Mutex` test fake under the plan's
    /// sanctioned cfg(test)-helper survivor class.
    #[derive(Default)]
    struct RecordingChainSink {
        records: Mutex<Vec<ChainRecord>>,
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl RecordingChainSink {
        fn snapshot(&self) -> Vec<ChainRecord> {
            self.records
                .lock()
                .expect("chain sink")
                .clone()
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl ChainAuditSink for RecordingChainSink {
        fn record(&self, record: &ChainRecord) -> std::io::Result<()> {
            self.records
                .lock()
                .expect("chain sink")
                .push(record.clone());
            Ok(())
        }
    }

    /// Wire `sink` into `serving`'s rendezvous and return it.
    async fn attached_sink(serving: &ServingRendezvous) -> Arc<RecordingChainSink> {
        let sink = Arc::new(RecordingChainSink::default());
        serving.rendezvous.set_chain_audit(sink.clone()).await;
        sink
    }

    /// The production posture with a test's own in-flight cap and handler
    /// deadline.
    fn posture(max_inflight: usize, handler_deadline: Duration) -> ServingPosture {
        ServingPosture {
            max_inflight,
            handler_deadline,
            ..ServingPosture::production()
        }
    }

    /// Forward one invocation over a real socket the way the broker's
    /// forwarder does: one connection, one canonically encoded request frame,
    /// one reply frame.
    fn forward(
        socket_path: &Path,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
    ) -> ForwardOperationResponse {
        let socket = connect_seqpacket(socket_path).expect("dial the rendezvous");
        let request = ForwardOperationRequest {
            chain_identities: None,
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload,
            context: None,
            fd_indexes: vec![],
            fd_kinds: vec![],
        };
        // The broker encodes the request with the canonical profile, so the
        // endpoint is exercised against the exact bytes the broker sends.
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        // A refused peer is answered before its frame is read: the server
        // writes the refusal, drains briefly, and closes, so a write that
        // races the close can fail with EPIPE even though the refusal is
        // already queued for this socket. Read the queued refusal instead
        // of failing the call; only a write error with no reply is a
        // failure.
        if let Err(write_error) = d2bd_runtime::unix_transport::write_frame(&socket, &encoded) {
            if let Ok(frame) = read_frame(&socket) {
                return serde_json::from_slice(&frame)
                    .expect("the reply is a ForwardOperationResponse");
            }
            panic!("write the request frame: {write_error:?}");
        }
        let frame = read_frame(&socket).expect("read the reply frame");
serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse")
    }

    use std::os::fd::{AsRawFd, RawFd};
    use d2b_contracts_broker::broker_wire::{FD_LEG, FdKind, MAX_FRAME_FDS};
    use d2bd_runtime::unix_transport::write_frame_with_fds;
    /// Forward one invocation with SCM_RIGHTS attachments on the request
    /// frame, the way the broker's forwarder does once the request leg
    /// carries fds.to
    fn forward_with_fds(
        socket_path: &Path,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
        fds: &[RawFd],
    ) -> ForwardOperationResponse {
        let request = ForwardOperationRequest {
            chain_identities: None,
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload,
            context: None,
            fd_indexes: (0..fds.len() as u32).collect(),
            fd_kinds: vec![FdKind::Fifo; fds.len()],
        };
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        let socket = connect_seqpacket(socket_path).expect("dial the rendezvous");
        // A refused peer is answered before its frame is read: the server
        // writes the refusal, drains briefly, and closes, so a write that
        // races the close can fail with EPIPE even though the refusal is
        // already queued for this socket. Read the queued refusal instead
        // of failing the call; only a write error with no reply is a
        // failure.
        if let Err(write_error) = write_frame_with_fds(&socket, &encoded, fds) {
            if let Ok(frame) = read_frame(&socket) {
                return serde_json::from_slice(&frame)
                    .expect("the reply is a ForwardOperationResponse");
            }
            panic!("write the request frame with fds: {write_error:?}");
        }
        let frame = read_frame(&socket).expect("read the reply frame");
        serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse")
    }

    /// Forward one invocation and read the reply frame's SCM_RIGHTS
    /// attachments, so a test can observe the descriptors a service
    /// returned on its declared response leg.
    fn forward_and_read_fds(
        socket_path: &Path,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
    ) -> (ForwardOperationResponse, Vec<RawFd>) {
        let request = ForwardOperationRequest {
            chain_identities: None,
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload,
            context: None,
            fd_indexes: vec![],
            fd_kinds: vec![],
        };
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        let socket = connect_seqpacket(socket_path).expect("dial the rendezvous");
        write_frame(&socket, &encoded).expect("write the request frame");
        let (frame, fds) = read_frame_with_fds(&socket).expect("read the reply frame with fds");
        (
            serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse"),
            fds,
        )
    }

    /// Forward one invocation with explicit fd declarations, so a test can
    /// drive a request whose declarations disagree with its frame.
    fn forward_raw_declared(
        socket_path: &Path,
        fd_indexes: Vec<u32>,
        fd_kinds: Vec<FdKind>,
        fds: &[RawFd],
    ) -> ForwardOperationResponse {
        let request = ForwardOperationRequest {
            chain_identities: None,
            operation: "fd-echo".to_owned(),
            zone: "test".to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload: serde_json::json!({}),
            context: None,
            fd_indexes,
            fd_kinds,
        };
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        let socket = connect_seqpacket(socket_path).expect("dial the rendezvous");
        write_frame_with_fds(&socket, &encoded, fds).expect("write the request frame");
        let frame = read_frame(&socket).expect("read the reply frame");
        serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse")
    }

    // ---- U8 effect-service dispatch through the rendezvous (KTD5) ----
    //
    // The plan's unit tests drive the effect service "through the
    // envelope": a forwarded call naming `service/method` crosses the
    // carrier, the rendezvous resolves the service to its live hosting
    // binding and the hosted actor answers - and a respawn or republish
    // that bumps the generational revision, or an actor that dies under an
    // in-flight call, surfaces as the dedicated stale-revision refusal,
    // never a hang.

    /// The declared effect service the effect tests host.
    ///
    /// The `ping` method serves the committed operation fixture-echo-ping:
    /// the forwarded call names that operation, and the declaration's
    /// operation facet resolves it to this service (KD6, U7). The zone-plane
    /// `echo` method carries no operation facet: the session layer addresses
    /// it, never the envelope.
    const ECHO_SERVICE: ServiceDecl = ServiceDecl {
        id: "fixture.echo",
        methods: &[
            ServiceMethod::serving("fixture-echo-ping", "ping"),
            ServiceMethod::zone_plane("echo"),
        ],
        attach_kinds: &[],
        streams: &[],
        endpoint_policy: None,
    };

    /// Echo fixture: answers with the request payload (same shape as the
    /// hosting site's harness).
    struct EchoService;

    #[async_trait::async_trait]
    impl EffectService for EchoService {
        async fn handle(
            &self,
            invocation: ServiceInvocation<'_>,
        ) -> Result<EffectResponse, EffectServiceError> {
            Ok(EffectResponse::new(invocation.payload.clone()))
        }
    }

    /// Builds one echo service per respawn.
    #[derive(Default)]
    struct EchoFactory;

    impl EffectServiceFactory for EchoFactory {
        fn build(&self) -> Arc<dyn EffectService> {
            Arc::new(EchoService)
        }
    }

    /// Returns one pre-built service (gated/declining fixtures).
    struct OnceFactory(Arc<dyn EffectService>);

    impl EffectServiceFactory for OnceFactory {
        fn build(&self) -> Arc<dyn EffectService> {
            self.0.clone()
        }
    }

    /// Gated fixture: parks inside `handle` until released, signalling that
    /// the call is genuinely in flight.
    struct GatedService {
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait::async_trait]
    impl EffectService for GatedService {
        async fn handle(
            &self,
            _invocation: ServiceInvocation<'_>,
        ) -> Result<EffectResponse, EffectServiceError> {
            self.entered.notify_one();
            self.release.notified().await;
            Ok(EffectResponse::new(CanonicalJsonObject::empty()))
        }
    }

    /// Declining fixture: answers with its own refusal.
    struct DecliningService;

    #[async_trait::async_trait]
    impl EffectService for DecliningService {
        async fn handle(
            &self,
            _invocation: ServiceInvocation<'_>,
        ) -> Result<EffectResponse, EffectServiceError> {
            Err(EffectServiceError::Declined {
                service: ECHO_SERVICE.id.to_owned(),
                reason: "fixture refuses".to_owned(),
            })
        }
    }

    /// The declared effect service whose `fd-echo` method declares a
    /// one-descriptor FIFO response leg (U3): the returned descriptor must
    /// satisfy the declared contract before it crosses the carrier.
    const FD_SERVICE: ServiceDecl = ServiceDecl {
        id: "fixture.fd",
        methods: &[ServiceMethod::serving_with(
            "fixture-fd-echo",
            "fd-echo",
            None,
            MethodFdContract::NONE,
            MethodFdContract {
                max_fds: 1,
                fd_kind: Some("fifo"),
            },
            &[],
            &[],
            None,
        )],
        attach_kinds: &[],
        streams: &[],
        endpoint_policy: None,
    };

    /// Returns one live FIFO descriptor on its declared response leg.
    struct FdReturningService;

    #[async_trait::async_trait]
    impl EffectService for FdReturningService {
        async fn handle(
            &self,
            _invocation: ServiceInvocation<'_>,
        ) -> Result<EffectResponse, EffectServiceError> {
            let (read_end, _write_end) = nix::unistd::pipe().expect("pipe");
            Ok(EffectResponse::with_fds(
                CanonicalJsonObject::empty(),
                vec![read_end],
            ))
        }
    }

    /// Answers one fixed view for the row the state-reading service asks
    /// for; every other manager call is unreachable at the rendezvous site.
    struct FixedViewManager;

    #[async_trait::async_trait]
    impl ManagerEndpoint for FixedViewManager {
        async fn ensure_child(
            &self,
            _parent: &ResourceKey,
            _child: d2b_resource_runtime::context::ChildEnsure,
        ) -> Result<d2b_resource_runtime::spec_store::EnsureOutcome, d2b_resource_runtime::error::ResourceError>
        {
            unreachable!("the rendezvous site ensures no children")
        }

        async fn get(
            &self,
            _key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::identity::StoredDesiredResource>, d2b_resource_runtime::error::ResourceError>
        {
            unreachable!("the rendezvous site reads no stored rows")
        }

        async fn view(
            &self,
            key: &ResourceKey,
        ) -> Result<Option<d2b_resource_runtime::manager::ResourceView>, d2b_resource_runtime::error::ResourceError>
        {
            Ok(Some(d2b_resource_runtime::manager::ResourceView {
                key: key.clone(),
                uid: [7; 16],
                generation: 42,
                deleting: false,
                provenance: d2b_resource_runtime::identity::ResourceProvenance::Api,
                spec: Vec::new(),
                metadata: Vec::new(),
                owner_key: None,
                status: None,
                status_generation: None,
                status_projection: None,
            }))
        }

        async fn delete(
            &self,
            _key: &ResourceKey,
        ) -> Result<(), d2b_resource_runtime::error::ResourceError> {
            unreachable!("the rendezvous site deletes no rows")
        }

        async fn list_owned(
            &self,
            _owner_uid: [u8; 16],
        ) -> Result<Vec<d2b_resource_runtime::identity::StoredDesiredResource>, d2b_resource_runtime::error::ResourceError>
        {
            unreachable!("the rendezvous site lists no owned rows")
        }

        async fn register_watch(
            &self,
            _subscriber: &ResourceKey,
            _registration: d2b_resource_runtime::context::WatchRegistration,
        ) -> Result<d2b_resource_runtime::context::WatchId, d2b_resource_runtime::error::ResourceError>
        {
            unreachable!("the rendezvous site registers no watches")
        }

        async fn cancel_watch(
            &self,
            _watch: d2b_resource_runtime::context::WatchId,
        ) -> Result<(), d2b_resource_runtime::error::ResourceError> {
            unreachable!("the rendezvous site cancels no watches")
        }
    }

    /// Reads one row through the driver context and answers with the
    /// observed generation: the service reaches resource state only through
    /// the generic driver context (R7), never a daemon state type.
    struct StateReadingService;

    #[async_trait::async_trait]
    impl EffectService for StateReadingService {
        async fn handle(
            &self,
            invocation: ServiceInvocation<'_>,
        ) -> Result<EffectResponse, EffectServiceError> {
            let key = ResourceKey::new(invocation.zone, "Process", "worker-0");
            match invocation.resources.view(&key).await {
                Ok(Some(view)) => Ok(EffectResponse::new(serde_json::from_value(
                    serde_json::json!({ "generation": view.generation }),
                )
                .expect("canonical payload"))),
                Ok(None) => Err(EffectServiceError::Declined {
                    service: "fixture.state".to_owned(),
                    reason: "row absent".to_owned(),
                }),
                Err(_) => Err(EffectServiceError::Declined {
                    service: "fixture.state".to_owned(),
                    reason: "read refused".to_owned(),
                }),
            }
        }
    }

    /// A driver that registers cleanly beside its service declaration; its
    /// spec/driver methods are unreachable at the rendezvous site.
    fn effect_descriptor(services: &'static [ServiceDecl]) -> DriverDescriptor {
        DriverDescriptor {
            resource_type: WellKnownType::PROCESS,
            allowed_sources: AllowedSources::STARTUP,
            verbs: &[],
            execution: &[],
            exportable: false,
            reads: &[],
            operations: &[],
            creations: &[],
            startup: &[],
            services,
            decoder: Arc::new(NoSpecs),
            factory: Arc::new(NoDrivers),
        }
    }

    struct NoSpecs;

    impl SpecDecoder for NoSpecs {
        fn decode(
            &self,
            _envelope: &[u8],
        ) -> Result<Box<dyn std::any::Any + Send>, Box<dyn std::error::Error + Send + Sync>> {
            unreachable!("the rendezvous site decodes no specs")
        }
    }

    struct NoDrivers;

    #[async_trait::async_trait]
    impl ResourceDriverFactory for NoDrivers {
        fn resource_types(&self) -> &[ResourceTypeName] {
            &[]
        }

        async fn create(&self, _key: &ResourceKey) -> Box<dyn DynResourceDriver> {
            unreachable!("the rendezvous site creates no resource drivers")
        }
    }

    /// Wait until a condition holds (the supervisor respawns asynchronously
    /// after a kill).
    async fn until(condition: impl Fn() -> bool) {
        for _ in 0..200 {
            if condition() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("condition never became true within the deadline");
    }

    /// A forwarded call crosses a real socket and the declared handler
    /// answers it:the result carries the family's own declaration, the Zone,
    /// and the invocation identifier the caller forwarded. A carrier that
    /// never reached the handler could not produce these values.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_forwarded_call_crosses_the_socket_and_the_declared_handler_answers() {
        let serving = ServingRendezvous::start().await;
        let response = forward(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
        );
        let ForwardOperationOutcome::Result { result, .. } = response.outcome else {
            panic!("the declared operation must answer, got a refusal");
        };
        assert_eq!(result["family"], "process");
        assert_eq!(result["resourceType"], "Process");
        assert_eq!(
            result["memberTypes"],
            serde_json::json!(["Process", "EphemeralProcess"])
        );
        assert_eq!(result["zone"], "test");
        assert_eq!(
            result["operations"],
            serde_json::json!([
                "inspect-process-family",
                "OpenPidfd",
                "OpenPeerPidfdFromAcceptedSocket",
                "ObserveRunner",
                "PollChildReaped",
                "PrepareRuntimeDir",
                "PrepareStateDir",
                "CgroupKill",
                "SignalRunner",
                "DeregisterRunnerPidfd",
                "SpawnRunner",
            ])
        );
        assert_eq!(result["verbs"][0], "get");
        assert_eq!(result["execution"], serde_json::json!(["host", "guest"]));
        // The handler's context carries the identifier the broker minted
        // before it forwarded, so both records name one invocation.
        assert_eq!(result["invocation"], "invocation-7");
    }

    /// An operation no started provider declares is refused by name, and so is
    /// a call naming a Zone this process has no providers for.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_undeclared_operation_is_refused_by_name() {
        let serving = ServingRendezvous::start().await;
        for (operation, zone) in [
            ("NoSuchOperation", "test"),
            ("inspect-process-family", "no-such-zone"),
        ] {
            let response = forward(
                &serving.socket_path,
                operation,
                zone,
                serde_json::json!({ "resourceType": "Process" }),
            );
            assert_eq!(
                response.outcome,
                ForwardOperationOutcome::Refused {
                    code: UNCOMMITTED_OPERATION.to_owned(),
                },
                "{operation} in {zone} is not served by this process"
            );
        }
    }

    /// A refusal the handler itself decided crosses the socket under its own
    /// code, so the peer's record and the passed-through detail keep the
    /// family's vocabulary rather than a carrier-level one.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_handler_refusal_crosses_back_under_its_own_code() {
        let serving = ServingRendezvous::start().await;
        let response = forward(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Quota" }),
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: INVALID_PROCESS_TYPE.to_owned(),
            }
        );
    }

    /// Calls are held inside their handlers at the same time, on the runtime:
    /// they do not queue behind one another, and calls in flight do not add a
    /// thread each - before this the serving path owned one thread per call,
    /// so four stalled calls added four.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_calls_are_held_on_the_runtime_without_a_thread_each() {
        const CALLS: usize = 4;
        // The runtime this test runs on: the serving path registers with
        // `Handle::current()`, so the held calls execute on these workers.
        const WORKER_THREADS: usize = 2;
        let serving = ServingRendezvous::start().await;
        // One warm call, so the runtime's workers exist before the baseline.
        let warm = forward_async(
            serving.socket_path.clone(),
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
        )
        .await;
        assert!(matches!(
            warm.outcome,
            ForwardOperationOutcome::Result { .. }
        ));

        // The recorder is this test's own: only the calls below execute the
        // stall-threads handler, so no other test can inflate the count.
        THREADS_SEEN.lock().await.clear();
        let calls: Vec<_> = (0..CALLS)
            .map(|_| {
                tokio::spawn(forward_async(
                    serving.socket_path.clone(),
                    STALL_THREADS,
                    "test",
                    serde_json::json!({}),
                ))
            })
            .collect();
        // Every call reached the handler while none of them was released: the
        // second call does not wait for the first.
        THREADS_GATE.wait_for(CALLS).await;
        // The held calls all run on this runtime's own workers: at most
        // WORKER_THREADS distinct threads can have executed the handler. A
        // per-call thread-ownership regression - one OS thread held for the
        // lifetime of each in-flight call - puts each call on its own
        // thread, far past the worker count. The measurement is per-test
        // (threads that executed this test's handler), so parallel test
        // load cannot inflate it.
        let seen = THREADS_SEEN.lock().await;
        assert_eq!(
            seen.len(),
            CALLS,
            "every held call recorded the thread that executed it"
        );
        let distinct = seen
            .iter()
            .copied()
            .collect::<std::collections::HashSet<_>>();
        assert!(
            distinct.len() <= WORKER_THREADS,
            "{CALLS} calls in flight must not each own a thread: held on {} distinct threads (the runtime has {WORKER_THREADS} workers)",
            distinct.len()
        );
        drop(seen);

        THREADS_GATE.release(CALLS);
        for call in calls {
            let answered = call.await.expect("the call task joins");
            assert!(
                matches!(answered.outcome, ForwardOperationOutcome::Result { .. }),
                "a released call answers"
            );
        }
    }

    /// A handler that never finishes is refused by the rendezvous's own
    /// deadline - by name, while the caller is still listening - and the slot
    /// it held is free again, so the call that follows is served rather than
    /// refused at the cap.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_stalled_handler_is_refused_by_name_and_frees_its_slot() {
        const HANDLER_DEADLINE: Duration = Duration::from_millis(200);
        let serving = ServingRendezvous::start_with(posture(1, HANDLER_DEADLINE)).await;

        let started = Instant::now();
        let refused = forward_async(
            serving.socket_path.clone(),
            STALL_FOREVER,
            "test",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(
            refused.outcome,
            ForwardOperationOutcome::Refused {
                code: FORWARD_TIMEOUT.to_owned(),
            },
            "a handler that never finishes is refused by the deadline of the call"
        );
        assert!(
            started.elapsed() >= HANDLER_DEADLINE,
            "the refusal is the deadline's, not an immediate one: {:?}",
            started.elapsed()
        );

        // The slot the refused call held is free: with a cap of one, the call
        // that follows is served rather than refused for capacity.
        let after = Instant::now();
        loop {
            let served = forward_async(
                serving.socket_path.clone(),
                "inspect-process-family",
                "test",
                serde_json::json!({ "resourceType": "Process" }),
            )
            .await;
            if matches!(served.outcome, ForwardOperationOutcome::Result { .. }) {
                break;
            }
            assert!(
                after.elapsed() < Duration::from_secs(5),
                "the slot a refused call held was never released: {:?}",
                served.outcome
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// A crashing handler is refused by name: the panic dies at the dispatch
    /// task boundary, the caller is answered with the handler-crashed code
    /// while the broker is still listening - not a dropped socket the broker
    /// would read as a round-trip timeout - and the accept loop (and its
    /// capacity slot) survives to serve the next call.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_crashing_handler_is_refused_by_name_and_the_rendezvous_keeps_serving() {
        const HANDLER_DEADLINE: Duration = Duration::from_secs(5);
        let serving = ServingRendezvous::start_with(posture(1, HANDLER_DEADLINE)).await;

        let started = Instant::now();
        let refused = forward_async(
            serving.socket_path.clone(),
            "panic-boom",
            "test",
            serde_json::json!({}),
        )
        .await;
        assert_eq!(
            refused.outcome,
            ForwardOperationOutcome::Refused {
                code: HANDLER_CRASHED.to_owned(),
            },
            "a crashed handler is refused by name, never as a timeout or a drop"
        );
        assert!(
            started.elapsed() < HANDLER_DEADLINE,
            "the crashed call is answered immediately, not at the deadline: {:?}",
            started.elapsed()
        );

        // The crashed handler did not take down the accept loop or hold its
        // slot: with a cap of one, the call that follows is served. The
        // refusal is written before the crashed call's permit is released,
        // so the follow-up retries briefly until the slot is free.
        let after = Instant::now();
        loop {
            let served = forward_async(
                serving.socket_path.clone(),
                "inspect-process-family",
                "test",
                serde_json::json!({ "resourceType": "Process" }),
            )
            .await;
            if matches!(served.outcome, ForwardOperationOutcome::Result { .. }) {
                break;
            }
            assert!(
                after.elapsed() < Duration::from_secs(5),
                "the crashed call's slot was never released: {served:?}"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// A handler that ran and failed crosses its own code back to the
    /// caller: the rendezvous relays the provider's failure code, so
    /// handler-errored is never flattened into the missing-handler refusal.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_erroring_handler_crosses_back_under_its_own_code() {
        let serving = ServingRendezvous::start().await;
        let response = forward(
            &serving.socket_path,
            "error-boom",
            "test",
            serde_json::json!({}),
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: "handler-errored".to_owned(),
            },
            "a handler error keeps its own code on the forwarded leg"
        );
    }

    /// A row with a large deadline tier runs past the historical fixed 25 s
    /// bound on the forwarded leg and completes: the context's budget is the
    /// Extended tier's (`MAX_CONTEXT_DEADLINE_MS`, the shared ceiling the
    /// broker mints for an extended row), and the rendezvous serves the
    /// context's budget as the per-call handler deadline instead of the
    /// tier-less constant.
    ///
    /// The run is a genuine 25 s+ completion (30 s of handler work under a
    /// 60 s budget), so it is a real 30-second test. The local leg's half of
    /// the same contract is the broker-side wiring test: the mint puts the
    /// Extended tier's 60 s budget into the context block (which this
    /// rendezvous then serves), and the local leg serves that same block's
    /// budget - a sync 30 s local handler would monopolize the
    /// process-wide two-worker handler set for the duration of the suite,
    /// so the real 25 s+ run lives on this leg, where the handler is an
    /// async task on its own runtime.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_row_with_a_large_deadline_tier_runs_past_twenty_five_seconds_on_the_forwarded_leg() {
        let (rendezvous, socket_path, _scratch, _providers) = ServingRendezvous::fixture().await;
        rendezvous.publish_generations("test", 1, 1);
        rendezvous.set_broker_epoch(5);
        let listener = bind(&socket_path, &test_identity()).expect("bind the rendezvous");
        let listener = AsyncSeqpacket::register(listener).expect("register the listener");
        // The posture's tier-less deadline is the historical 25 s constant;
        // the context's Extended-tier budget (the shared 60 s ceiling) must
        // own the call, so the 30 s run completes instead of being refused
        // at the old fixed bound.
        tokio::spawn(serve_accepted(
            rendezvous,
            listener,
            posture(1, Duration::from_millis(DEFAULT_CONTEXT_DEADLINE_MS)),
        ));
        let mut extended = fresh_context();
        extended.deadline_ms = MAX_CONTEXT_DEADLINE_MS;
        let started = Instant::now();
        let answered = forward_with_context_async(
            socket_path,
            "slow-30s",
            "test",
            serde_json::json!({}),
            extended,
            Duration::from_secs(60),
        )
        .await;
        let elapsed = started.elapsed();
        assert!(
            matches!(answered.outcome, ForwardOperationOutcome::Result { .. }),
            "the Extended-tier budget let the 30 s handler finish: {answered:?}"
        );
        assert!(
            elapsed >= Duration::from_secs(30),
            "the handler ran its full 30 s: {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_secs(55),
            "the completion is the handler's, not the budget's expiry: {elapsed:?}"
        );
    }

    /// A call the rendezvous is at its cap for is answered with the daemon's
    /// own capacity code - a named refusal, not a silent close - and the
    /// calls already in flight are undisturbed.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_call_over_the_in_flight_cap_is_refused_by_name() {
        let serving = ServingRendezvous::start_with(posture(1, Duration::from_secs(10))).await;
        let held = tokio::spawn(forward_async(
            serving.socket_path.clone(),
            STALL_CAPACITY,
            "test",
            serde_json::json!({}),
        ));
        CAPACITY_GATE.wait_for(1).await;

        let over_cap = forward_async(
            serving.socket_path.clone(),
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
        )
        .await;
        assert_eq!(
            over_cap.outcome,
            ForwardOperationOutcome::Refused {
                code: TypedError::DaemonBusy.kind().to_owned(),
            },
            "the cap refuses under the code the daemon's own socket refuses with"
        );

        CAPACITY_GATE.release(1);
        let answered = held.await.expect("the held call joins");
        assert!(
            matches!(answered.outcome, ForwardOperationOutcome::Result { .. }),
            "the call admitted before the cap was reached still answers"
        );
    }

    /// A peer this endpoint does not accept is refused by name before a
    /// frame is served: the group DAC that lets the dialer connect is not the
    /// admission, `SO_PEERCRED` is.
    ///
    /// The accepted identity is moved off the dialing process's, which is the
    /// only way to exercise the refusal from inside one process. The root arm
    /// cannot be moved - root is the privileged broker and is always
    /// accepted - so a run as root has nothing to refuse here.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_peer_that_is_not_the_broker_is_refused_by_name() {
        if nix::unistd::geteuid().is_root() {
            return;
        }
        let not_the_dialer = ServingPosture {
            accepted_uid: u32::MAX,
            ..ServingPosture::production()
        };
        let serving = ServingRendezvous::start_with(not_the_dialer).await;
        let response = forward_async(
            serving.socket_path.clone(),
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
        )
        .await;
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: UNGRANTED_CALLER.to_owned(),
            },
            "the dialing process is not the accepted peer"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_fd_carrying_request_crosses_the_socket_and_the_declared_handler_reads_it_back() {
        use nix::unistd::{pipe, write};
        let serving = ServingRendezvous::start().await;
        let (read_end, write_end) = pipe().expect("pipe");
        write(&write_end, b"ok").expect("write the payload bytes");
        drop(write_end);
        let response = forward_with_fds(
            &serving.socket_path,
            "fd-echo",
            "test",
            serde_json::json!({}),
            &[read_end.as_raw_fd()],
        );
        let ForwardOperationOutcome::Result { result, .. } = response.outcome else {
            panic!("the fd-echo operation must answer, got a refusal");
        };
        assert_eq!(
            result["read"],
            "ok",
            "the handler must have read the caller's bytes through the received descriptor"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_request_whose_fd_count_mismatches_is_refused_with_the_fd_leg_code() {
        use nix::unistd::{pipe, write};
        let serving = ServingRendezvous::start().await;
        let (read_end, write_end) = pipe().expect("pipe");
        write(&write_end, b"x").expect("write payload bytes");
        let response = forward_raw_declared(
            &serving.socket_path,
            vec![0, 1],
            vec![FdKind::Fifo, FdKind::Fifo],
            &[read_end.as_raw_fd()],
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: FD_LEG.to_owned(),
            },
            "a declared count that disagrees with the frame is refused with the fd-leg code"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_request_whose_fd_kind_mismatches_is_refused_with_the_fd_leg_code() {
        use nix::unistd::pipe;
        let serving = ServingRendezvous::start().await;
        let (read_end, _write_end) = pipe().expect("pipe");
        let response = forward_raw_declared(
            &serving.socket_path,
            vec![0],
            vec![FdKind::Socket],
            &[read_end.as_raw_fd()],
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: FD_LEG.to_owned(),
            },
            "a descriptor whose kernel kind mismatches the declaration is refused"
        );
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_request_whose_fd_declarations_exceed_the_frame_ceiling_is_refused_with_the_fd_leg_code() {
        let serving = ServingRendezvous::start().await;
        let response = forward_raw_declared(
            &serving.socket_path,
            (0..=MAX_FRAME_FDS as u32).collect(),
            vec![FdKind::Fifo; MAX_FRAME_FDS + 1],
            &[],
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: FD_LEG.to_owned(),
            },
            "declarations over the frame ceiling are refused with the fd-leg code, never truncated"
        );
    }

    use d2b_contracts_broker::broker_wire::{ForwardContext, STALE_CONTEXT};

    /// The attestation state the context tests below agree on: the fixture
    /// Zone "test" with one provider publication (revision 1), controller
    /// and guest generations 1, under broker epoch 5.
    fn context_for(epoch: u64, zone: &str, revision: u64, guest_generation: u64) -> ForwardContext {
        ForwardContext {
            broker_epoch: epoch,
            zone: zone.to_owned(),
            provider_set_revision: revision,
            controller_generation: 1,
            guest_generation,
            initiating_identity: "daemon".to_owned(),
            deadline_ms: DEFAULT_CONTEXT_DEADLINE_MS,
        }
    }

    fn fresh_context() -> ForwardContext {
        context_for(5, "test", 1, 1)
    }

    /// Forward one invocation carrying a broker-minted context, the way the
    /// broker's forwarder does once the envelope attests its calls: one
    /// connection, one canonical request frame carrying the context block,
    /// one reply frame.
    fn forward_with_context(
        socket_path: &Path,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
        context: ForwardContext,
    ) -> ForwardOperationResponse {
        let request = ForwardOperationRequest {
            chain_identities: None,
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload,
            context: Some(context),
            fd_indexes: vec![],
            fd_kinds: vec![],
        };
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        let socket = connect_seqpacket(socket_path).expect("dial the rendezvous");
        d2bd_runtime::unix_transport::write_frame(&socket, &encoded)
            .expect("write the request frame");
        let frame = read_frame(&socket).expect("read the reply frame");
        serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse")
    }

    /// Forward one invocation with a context on the runtime, so a stalled
    /// handler test can exercise the budget the context declares.
    ///
    /// `reply_deadline` bounds the frame read: a test that proves a 25 s+
    /// completion passes the budget the call's context declares (longer than
    /// the default 10 s read bound), so the frame read cannot outrun the
    /// completion it is watching.
    async fn forward_with_context_async(
        socket_path: PathBuf,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
        context: ForwardContext,
        reply_deadline: Duration,
    ) -> ForwardOperationResponse {
        let request = ForwardOperationRequest {
            chain_identities: None,
            operation: operation.to_owned(),
            zone: zone.to_owned(),
            invocation_id: "invocation-7".to_owned(),
            payload,
            context: Some(context),
            fd_indexes: vec![],
            fd_kinds: vec![],
        };
        let encoded =
            canonical_json_bytes(&request).expect("the request encodes as canonical JSON");
        let socket = connect_seqpacket(&socket_path).expect("dial the rendezvous");
        let connection =
            AsyncSeqpacket::register(Socket::from(socket)).expect("register the forwarded call");
        connection
            .write_frame(&encoded, reply_deadline)
            .await
            .expect("write the request frame");
        let frame = connection
            .read_frame(reply_deadline)
            .await
            .expect("read the reply frame");
        serde_json::from_slice(&frame).expect("the reply is a ForwardOperationResponse")
    }

    /// A freshly minted context passes the field-wise freshness check: the
    /// epoch is the daemon's observed one, the Zone binds to the call, and
    /// the revision and generations match the daemon's current values.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_fresh_context_passes_the_rendezvous() {
        let serving = ServingRendezvous::start_attesting().await;
        let response = forward_with_context(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
            fresh_context(),
        );
        assert!(
            matches!(response.outcome, ForwardOperationOutcome::Result { .. }),
            "a fresh context must be admitted, got {response:?}"
        );
    }

    /// Before the daemon has observed any broker epoch, no context can be
    /// verified: the attestation's epoch half is uncheckable, so every
    /// attested call refuses fail-closed - the receiving side of the rule
    /// that the broker refuses to mint until it holds a value.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn contexts_refuse_until_the_daemon_observes_a_broker_epoch() {
        let serving = ServingRendezvous::start().await;
        let response = forward_with_context(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
            context_for(5, "test", 1, 1),
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: STALE_CONTEXT.to_owned(),
            },
            "an epoch the daemon has never observed cannot validate a context"
        );
    }

    /// A context minted against an older provider-set revision refuses: the
    /// daemon republished its provider set since the broker minted.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_context_minted_against_an_older_provider_set_revision_is_refused() {
        let serving = ServingRendezvous::start_attesting().await;
        let response = forward_with_context(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
            context_for(5, "test", 0, 1),
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: STALE_CONTEXT.to_owned(),
            },
            "an older provider-set revision is stale"
        );
    }

    /// A context minted against a lower guest generation refuses: the
    /// daemon's current guest generation moved past the minted one.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_context_minted_against_a_lower_guest_generation_is_refused() {
        let serving = ServingRendezvous::start_attesting().await;
        let response = forward_with_context(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
            context_for(5, "test", 1, 0),
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: STALE_CONTEXT.to_owned(),
            },
            "a lower guest generation is stale"
        );
    }

    /// A context whose Zone is not the call's Zone is not bound to the
    /// connection: the attestation names another Zone, so the call refuses.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_context_minted_against_another_zone_is_refused() {
        let serving = ServingRendezvous::start_attesting().await;
        let response = forward_with_context(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
            context_for(5, "other-zone", 1, 1),
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: STALE_CONTEXT.to_owned(),
            },
            "a context for another zone is stale on this call"
        );
    }

    /// A mutated context refuses even when every field it touched moved the
    /// "right" way: the broker is the sole minter, so any difference from
    /// the daemon's current values is tampering, never a fresher truth.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_mutated_context_is_refused() {
        let serving = ServingRendezvous::start_attesting().await;
        let mut mutated = fresh_context();
        mutated.controller_generation = 999;
        let response = forward_with_context(
            &serving.socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
            mutated,
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: STALE_CONTEXT.to_owned(),
            },
            "a mutated context cannot name a fresher truth than the mint"
        );
    }

    /// A broker restart is a changed epoch: every context minted before it
    /// refuses regardless of generation equality, and once the daemon
    /// observes the fresh nonce only contexts minted under it pass - a
    /// pre-restart context cannot be re-minted into validity.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_broker_restart_invalidates_every_previous_context_via_the_epoch() {
        let (rendezvous, socket_path, _scratch, _providers) = ServingRendezvous::fixture().await;
        rendezvous.publish_generations("test", 1, 1);
        rendezvous.set_broker_epoch(5);
        let listener = bind(&socket_path, &test_identity()).expect("bind the rendezvous");
        spawn_server(rendezvous.clone(), listener, tokio::runtime::Handle::current())
            .expect("start the rendezvous server");

        // Before the restart: the context minted under epoch 5 passes.
        let before = forward_with_context(
            &socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
            fresh_context(),
        );
        assert!(
            matches!(before.outcome, ForwardOperationOutcome::Result { .. }),
            "a context minted under the current epoch passes"
        );

        // The broker restarts and mints under a fresh epoch; the daemon's
        // next publication acknowledgement carries it.
        rendezvous.set_broker_epoch(6);

        // Every previously minted context now refuses via the changed
        // epoch, with the generations equal - the epoch alone is the
        // invalidation.
        let after_restart = forward_with_context(
            &socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
            fresh_context(),
        );
        assert_eq!(
            after_restart.outcome,
            ForwardOperationOutcome::Refused {
                code: STALE_CONTEXT.to_owned(),
            },
            "a pre-restart context fails via the changed broker epoch"
        );
        // A re-mint under the fresh epoch passes; the old block still
        // carries the old epoch, so no context survives the restart by
        // re-mint - only a brand-new mint under the fresh nonce does.
        let re_minted = forward_with_context(
            &socket_path,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
            context_for(6, "test", 1, 1),
        );
        assert!(
            matches!(re_minted.outcome, ForwardOperationOutcome::Result { .. }),
            "a context minted under the fresh epoch passes"
        );
    }

    /// The context's deadline budget is the per-call handler deadline: a
    /// handler that never finishes is refused by the budget the context
    /// declares, not by the posture's fixed constant.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn the_context_deadline_budget_bounds_the_handler() {
        const BUDGET: Duration = Duration::from_millis(150);
        let (rendezvous, socket_path, _scratch, _providers) = ServingRendezvous::fixture().await;
        rendezvous.publish_generations("test", 1, 1);
        rendezvous.set_broker_epoch(5);
        let listener = bind(&socket_path, &test_identity()).expect("bind the rendezvous");
        let listener = AsyncSeqpacket::register(listener).expect("register the listener");
        // The posture's deadline is 10 s; the context's 150 ms must own the
        // call, so the refusal lands on the budget, not the constant.
        tokio::spawn(serve_accepted(
            rendezvous,
            listener,
            posture(1, Duration::from_secs(10)),
        ));
        let started = Instant::now();
        let mut stalled = fresh_context();
        stalled.deadline_ms = BUDGET.as_millis() as u64;
        let refused = forward_with_context_async(
            socket_path,
            STALL_FOREVER,
            "test",
            serde_json::json!({}),
            stalled,
            Duration::from_secs(10),
        )
        .await;
        assert_eq!(
            refused.outcome,
            ForwardOperationOutcome::Refused {
                code: FORWARD_TIMEOUT.to_owned(),
            },
            "a handler that never finishes is refused by the context's budget"
        );
        let elapsed = started.elapsed();
        assert!(
            elapsed >= BUDGET && elapsed < Duration::from_secs(5),
            "the refusal is the context budget's, not the posture's: {elapsed:?}"
        );
    }

    /// A deadline budget outside the shared ceiling is a block the broker
    /// did not mint: zero or oversized budgets refuse with the stale-context
    /// code rather than serving an unbounded handler grant.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_context_whose_budget_escapes_the_ceiling_is_refused() {
        let serving = ServingRendezvous::start_attesting().await;
        for budget in [0, MAX_CONTEXT_DEADLINE_MS + 1] {
            let mut context = fresh_context();
            context.deadline_ms = budget;
            let response = forward_with_context(
                &serving.socket_path,
                "inspect-process-family",
                "test",
                serde_json::json!({ "resourceType": "Process" }),
                context,
            );
            assert_eq!(
                response.outcome,
                ForwardOperationOutcome::Refused {
                    code: STALE_CONTEXT.to_owned(),
                },
                "a budget of {budget} ms is not one the broker mints"
            );
        }
    }

    // -----------------------------------------------------------------
    // The origination leg: the daemon publishes its current values to the
    // broker, and the rendezvous advances exactly on the acknowledgement.
    // -----------------------------------------------------------------

    use d2b_contracts_broker::broker_wire::{
        BrokerCallerRole, BrokerErrorResponse, BrokerRequest, BrokerRequestEnvelope,
        BrokerResponse, PublishTrustedContextResponse, PublishTrustedContextValues,
    };
    use crate::provider_lifecycle::TrustedContextPublication;

    /// The broker's half of one origination-leg publication: bind the broker
    /// socket, read the daemon's request envelope, assert the published
    /// values, and reply with `reply`. The envelope the daemon actually sent
    /// is delivered on `seen`.
    ///
    /// Synchronous by construction (a dedicated blocking broker thread, the
    /// plan's sanctioned bounded seat), so the blocking socket/channel calls
    /// stay under the cfg(test)-helper survivor class.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn serve_one_publication(
        socket_path: PathBuf,
        expected: PublishTrustedContextValues,
        reply: BrokerResponse,
        seen: std::sync::mpsc::Sender<BrokerRequestEnvelope>,
    ) -> std::thread::JoinHandle<()> {
        // The listener binds before the thread spawns, so the daemon's
        // one-shot publication dial (started by the test after this returns)
        // can never race the bind: a dial before the bind would error, the
        // daemon would never retry, and the accept below would block the
        // test's `broker.join()` forever.
        let listener = bind_public_socket(&socket_path, &test_identity())
            .expect("bind the test broker socket");
        std::thread::spawn(move || {
            listener
                .set_nonblocking(false)
                .expect("the test broker accepts blockingly");
            let (peer, _) = listener.accept().expect("the daemon dials the broker");
            let frame = read_frame(&peer).expect("read the publication frame");
            let envelope: BrokerRequestEnvelope =
                serde_json::from_slice(&frame).expect("the publication is a broker envelope");
            match &envelope.request {
                BrokerRequest::PublishTrustedContext(values) => {
                    assert_eq!(
                        &expected, values,
                        "the daemon publishes its current provider-set values"
                    );
                }
                other => {
                    panic!(
                        "expected a PublishTrustedContext request, got {}",
                        other.op_name()
                    )
                }
            }
            let acknowledged = canonical_json_bytes(&reply)
                .expect("the acknowledgement encodes as canonical JSON");
            write_frame(&peer, &acknowledged).expect("write the acknowledgement frame");
            let _ = seen.send(envelope);
        })
    }

    /// The daemon publishes the Zone's current values over the origination
    /// leg when its started set carries a publication binding, and the
    /// rendezvous advances exactly on the acknowledged broker epoch: a
    /// context minted against the acked epoch, revision, and generations is
    /// admitted, and any pre-ack state is stale.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn the_daemon_publishes_over_the_origination_leg_and_advances_on_the_ack() {
        let zone = ZoneId::parse("test").expect("the test zone label is canonical");
        let scratch = tempfile::tempdir().expect("test scratch");
        let broker_socket = scratch.path().join("broker.sock");
        let rendezvous_socket = scratch.path().join("d2bd-forward.sock");
        let expected = PublishTrustedContextValues {
            zone: zone.as_str().to_owned(),
            provider_set_revision: 1,
            controller_generation: 4,
            guest_generation: 1,
        };
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        let broker = serve_one_publication(
            broker_socket.clone(),
            expected.clone(),
            BrokerResponse::PublishTrustedContext(PublishTrustedContextResponse {
                broker_epoch: 7,
            }),
            seen_tx,
        );

        let [process, ephemeral] = process_family_descriptors(ProcessDriverArgs {
            zone: zone.clone(),
            facets: refusing_facets(),
            zone_uid: None,
            policy_revision: None,
            provider_assignment_generation: None,
            controller_generation: ControllerGeneration::new(1)
                .expect("the test generation is canonical"),
            guest_execution: None,
            mode: ExecutionMode::Host,
        });
        let mut set = ProviderSet::new(zone.clone(), scratch.path().join("state"))
            .with_trusted_context_publication(Some(
                TrustedContextPublication::production(
                    DaemonMode::Host,
                    broker_socket,
                    nix::unistd::getuid().as_raw(),
                    4,
                ),
            ))
            .with(
                family_declaration("process"),
                vec![
                    process,
                    DriverDescriptor {
                        operations: &STALL_OPERATIONS[..],
                        ..ephemeral
                    },
                ],
            );
        // The U15 hosting pass publishes every registered family's service
        // that no driver in this set declared; the fixture set declares
        // only the process family's driver, so the remaining registered
        // services receive the echo fixture factory too - they are hosted
        // but never called by these tests.
        for registration in crate::resource_plane_v3::PROVIDER_REGISTRATIONS {
            for &service in registration.services {
                set = set.with_effect_service_factory(service, Arc::new(EchoFactory));
            }
        }
        let providers = Arc::new(
            set.start()
                .await
                .expect("the process family starts through the base"),
        );
        let rendezvous = Arc::new(ForwardRendezvous::new());
        let revision = rendezvous
            .publish(zone.as_str(), Arc::clone(&providers))
            .await;
        assert_eq!(revision, 1, "the first publication is revision 1");
        let envelope =
            seen_rx
                .recv_timeout(Duration::from_secs(10))
                .expect("one publication reaches the broker");
        assert_eq!(
            envelope.caller_role,
            BrokerCallerRole::AdminUid {
                uid: nix::unistd::getuid().as_raw(),
            },
            "the publication presents the daemon's caller role"
        );
        broker.join().expect("the test broker completes");

        let listener = bind(&rendezvous_socket, &test_identity()).expect("bind the rendezvous");
        spawn_server(rendezvous.clone(), listener, tokio::runtime::Handle::current())
            .expect("start the rendezvous server");

        // The acked epoch, revision, and generations pass field-wise.
        let acknowledged = forward_with_context(
            &rendezvous_socket,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
            ForwardContext {
                broker_epoch: 7,
                zone: "test".to_owned(),
                provider_set_revision: 1,
                controller_generation: 4,
                guest_generation: 1,
                initiating_identity: "daemon".to_owned(),
                deadline_ms: DEFAULT_CONTEXT_DEADLINE_MS,
            },
        );
        assert!(
            matches!(acknowledged.outcome, ForwardOperationOutcome::Result { .. }),
            "a context minted against the acknowledged state is admitted, got {acknowledged:?}"
        );

        // Pre-ack epochs, older revisions, and older generations refuse.
        for stale in [
            context_for(5, "test", 1, 1),
            context_for(7, "test", 2, 1),
            context_for(7, "other", 1, 1),
        ] {
            let response = forward_with_context(
                &rendezvous_socket,
                "inspect-process-family",
                "test",
                serde_json::json!({ "resourceType": "Process" }),
                stale,
            );
            assert_eq!(
                response.outcome,
                ForwardOperationOutcome::Refused {
                    code: STALE_CONTEXT.to_owned(),
                },
                "a context outside the acknowledged state is stale"
            );
        }
    }

    /// A refused publication advances nothing: a broker that refuses the
    /// publication leaves the rendezvous fail-closed on its zero epoch, so
    /// no context validates - the daemon never trusts an epoch it was not
    /// acknowledged.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    async fn a_refused_publication_leaves_the_rendezvous_fail_closed() {
        let zone = ZoneId::parse("test").expect("the test zone label is canonical");
        let scratch = tempfile::tempdir().expect("test scratch");
        let broker_socket = scratch.path().join("broker.sock");
        let rendezvous_socket = scratch.path().join("d2bd-forward.sock");
        let expected = PublishTrustedContextValues {
            zone: zone.as_str().to_owned(),
            provider_set_revision: 1,
            controller_generation: 1,
            guest_generation: 1,
        };
        let (seen_tx, _) = std::sync::mpsc::channel();
        let broker = serve_one_publication(
            broker_socket.clone(),
            expected.clone(),
            BrokerResponse::Error(BrokerErrorResponse {
                kind: "refused".to_owned(),
                operation: "PublishTrustedContext".to_owned(),
                target_wave: None,
                message: "stale publication".to_owned(),
                action: "republish".to_owned(),
            }),
            seen_tx,
        );

        let [process, ephemeral] = process_family_descriptors(ProcessDriverArgs {
            zone: zone.clone(),
            facets: refusing_facets(),
            zone_uid: None,
            policy_revision: None,
            provider_assignment_generation: None,
            controller_generation: ControllerGeneration::new(1)
                .expect("the test generation is canonical"),
            guest_execution: None,
            mode: ExecutionMode::Host,
        });
        let providers = Arc::new(
            ProviderSet::new(zone.clone(), scratch.path().join("state"))
                .with_trusted_context_publication(Some(
                    TrustedContextPublication::production(
                        DaemonMode::Host,
                        broker_socket,
                        nix::unistd::getuid().as_raw(),
                        1,
                    ),
                ))
                .with(
                    family_declaration("process"),
                    vec![
                        process,
                        DriverDescriptor {
                            operations: &STALL_OPERATIONS[..],
                            ..ephemeral
                        },
                    ],
                )
                // The U15 hosting pass publishes every registered family's
                // service that no driver in this set declared; the fixture
                // set declares only the process family's driver, so the
                // remaining registered services receive the echo fixture
                // factory too - they are hosted but never called.
                .inject_registered_service_factories::<EchoFactory>()
                .start()
                .await
                .expect("the process family starts through the base"),
        );
        let rendezvous = Arc::new(ForwardRendezvous::new());
        rendezvous
            .publish(zone.as_str(), Arc::clone(&providers))
            .await;
        broker.join().expect("the test broker completes");

        let listener = bind(&rendezvous_socket, &test_identity()).expect("bind the rendezvous");
        spawn_server(rendezvous.clone(), listener, tokio::runtime::Handle::current())
            .expect("start the rendezvous server");

        for context in [context_for(7, "test", 1, 1), context_for(1, "test", 1, 1)] {
            let response = forward_with_context(
                &rendezvous_socket,
                "inspect-process-family",
                "test",
                serde_json::json!({ "resourceType": "Process" }),
                context,
            );
            assert_eq!(
                response.outcome,
                ForwardOperationOutcome::Refused {
                    code: STALE_CONTEXT.to_owned(),
                },
                "a refused publication acknowledges no epoch, so every context is stale"
            );
        }
    }

    /// A broker that never answers the publication leaves the rendezvous
    /// fail-closed too: the transport failure is the same refusal boundary,
    /// observed before any epoch could be acknowledged.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_unreachable_broker_leaves_the_rendezvous_fail_closed() {
        let zone = ZoneId::parse("test").expect("the test zone label is canonical");
        let scratch = tempfile::tempdir().expect("test scratch");
        let missing_broker = scratch.path().join("broker.sock");
        let rendezvous_socket = scratch.path().join("d2bd-forward.sock");
        let [process, ephemeral] = process_family_descriptors(ProcessDriverArgs {
            zone: zone.clone(),
            facets: refusing_facets(),
            zone_uid: None,
            policy_revision: None,
            provider_assignment_generation: None,
            controller_generation: ControllerGeneration::new(1)
                .expect("the test generation is canonical"),
            guest_execution: None,
            mode: ExecutionMode::Host,
        });
        let mut set = ProviderSet::new(zone.clone(), scratch.path().join("state"))
            .with_trusted_context_publication(Some(
                TrustedContextPublication::production(
                    DaemonMode::Host,
                    missing_broker,
                    nix::unistd::getuid().as_raw(),
                    1,
                ),
            ))
            .with(
                family_declaration("process"),
                vec![
                    process,
                    DriverDescriptor {
                        operations: &STALL_OPERATIONS[..],
                        ..ephemeral
                    },
                ],
            );
        // The U15 hosting pass publishes every registered family's service
        // that no driver in this set declared; the fixture set declares
        // only the process family's driver, so the remaining registered
        // services receive the echo fixture factory too - they are hosted
        // but never called by these tests.
        for registration in crate::resource_plane_v3::PROVIDER_REGISTRATIONS {
            for &service in registration.services {
                set = set.with_effect_service_factory(service, Arc::new(EchoFactory));
            }
        }
        let providers = Arc::new(
            set.start()
                .await
                .expect("the process family starts through the base"),
        );
        let rendezvous = Arc::new(ForwardRendezvous::new());
        rendezvous
            .publish(zone.as_str(), Arc::clone(&providers))
            .await;

        let listener = bind(&rendezvous_socket, &test_identity()).expect("bind the rendezvous");
        spawn_server(rendezvous.clone(), listener, tokio::runtime::Handle::current())
            .expect("start the rendezvous server");

        let response = forward_with_context(
            &rendezvous_socket,
            "inspect-process-family",
            "test",
            serde_json::json!({ "resourceType": "Process" }),
            context_for(7, "test", 1, 1),
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: STALE_CONTEXT.to_owned(),
            },
            "a broker that never answered acknowledges no epoch"
        );
    }

    // ---- U8 effect-service dispatch tests (KTD5) ----

    /// U8 happy path through the rendezvous: a driver's forwarded call
    /// naming a declared effect-service method crosses the carrier, the
    /// rendezvous resolves the service to its live hosting binding, and the
    /// hosted actor answers with the canonical payload round-tripping
    /// untouched - the effect service rides the normal forward carrier, no
    /// second transport.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_forwarded_effect_service_call_is_answered_by_the_hosted_actor() {
        let serving = ServingRendezvous::start_with_effect_service().await;
        let response = forward(
            &serving.socket_path,
            "fixture-echo-ping",
            "test",
            serde_json::json!({ "echo": "ping" }),
        );
        let ForwardOperationOutcome::Result { result, .. } = response.outcome else {
            panic!("the declared effect service must answer, got a refusal");
        };
        assert_eq!(result, serde_json::json!({ "echo": "ping" }));
    }

    /// An operation nothing in this process declares is refused like any
    /// uncommitted operation: neither a hosted service's operation facet nor
    /// a provider's handler table names it. A `service/method` spelling
    /// works for no operation - the wire names the committed operation, and
    /// the declaration's operation facet resolves it to the service (KD6).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_operation_no_effect_service_or_provider_declares_is_refused_by_name() {
        let serving = ServingRendezvous::start_with_effect_service().await;
        for (operation, what) in [
            ("no-such-operation", "no declaration names it"),
            ("fixture.echo/ping", "the service/method spelling is not a committed operation"),
        ] {
            let response = forward(&serving.socket_path, operation, "test", serde_json::json!({}));
            assert_eq!(
                response.outcome,
                ForwardOperationOutcome::Refused {
                    code: UNCOMMITTED_OPERATION.to_owned(),
                },
                "{operation}: {what}; it must refuse as uncommitted"
            );
        }
    }

    /// U8 error path through the rendezvous, `manager.rs:1190-1211`
    /// semantics: killing the actor mid-supervision respawns the service
    /// from its durable row and bumps the generational revision; the next
    /// forwarded call succeeds against the fresh generation.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn killing_a_hosted_effect_service_respawns_and_the_next_forwarded_call_succeeds() {
        let serving = ServingRendezvous::start_with_effect_service().await;
        let providers = Arc::clone(&serving._providers);
        let binding = providers
            .resolve_effect_service(ECHO_SERVICE.id)
            .await
            .expect("the declared service resolves");
        let revision_before = binding.revision();

        let response = forward(
            &serving.socket_path,
            "fixture-echo-ping",
            "test",
            serde_json::json!({ "one": true }),
        );
        let ForwardOperationOutcome::Result { result, .. } = response.outcome else {
            panic!("the declared effect service must answer, got a refusal");
        };
        assert_eq!(result, serde_json::json!({ "one": true }));

        // Kill the actor mid-supervision (aborts any in-flight work).
        binding.kill();

        // The zone supervisor respawns from the durable row and bumps the
        // generational revision.
        until(|| binding.revision() != revision_before).await;
        let respawned = providers
            .resolve_effect_service(ECHO_SERVICE.id)
            .await
            .expect("resolve after respawn");
        assert_eq!(
            respawned.revision(),
            revision_before + 1,
            "respawn bumped the revision"
        );

        // The next forwarded call succeeds against the respawned generation.
        let response = forward(
            &serving.socket_path,
            "fixture-echo-ping",
            "test",
            serde_json::json!({ "two": true }),
        );
        let ForwardOperationOutcome::Result { result, .. } = response.outcome else {
            panic!("the respawned service must answer, got a refusal");
        };
        assert_eq!(result, serde_json::json!({ "two": true }));
    }

    /// U8 edge through the rendezvous: an in-flight forwarded call whose
    /// actor dies is refused with the dedicated stale-revision code, never
    /// hung, and the service still respawns from its durable row.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_in_flight_effect_service_call_refuses_with_the_stale_revision_code_when_the_actor_dies()
    {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let gated: Arc<dyn EffectService> = Arc::new(GatedService {
            entered: entered.clone(),
            release: release.clone(),
        });
        let serving =
            ServingRendezvous::start_with_effect_service_factory(Arc::new(OnceFactory(gated)))
                .await;
        let binding = serving
            ._providers
            .resolve_effect_service(ECHO_SERVICE.id)
            .await
            .expect("the declared service resolves");

        let caller = tokio::spawn(forward_async(
            serving.socket_path.clone(),
            "fixture-echo-ping",
            "test",
            serde_json::json!({ "in-flight": true }),
        ));
        // Wait until the call is genuinely parked inside the service.
        entered.notified().await;

        binding.kill();

        let outcome = tokio::time::timeout(Duration::from_secs(2), caller)
            .await
            .expect("in-flight call must refuse, not hang");
        let response = outcome.expect("the call completed");
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: STALE_REVISION.to_owned(),
            },
            "the in-flight call rode a dead generation; the dedicated refusal must name it"
        );

        // The service still respawns from its durable row afterwards.
        until(|| binding.revision() != 1).await;
        let respawned = serving
            ._providers
            .resolve_effect_service(ECHO_SERVICE.id)
            .await
            .expect("resolve after respawn");
        assert_eq!(respawned.revision(), 2);
    }

    /// U8 republish through the hosting seam keeps the rendezvous serving:
    /// the republish bumps the generational revision and a fresh resolve
    /// dispatches to the rebuilt actor (provider-set republish, KTD5).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_republished_effect_service_keeps_answering_through_the_rendezvous() {
        let serving = ServingRendezvous::start_with_effect_service().await;
        let rebound = serving
            ._providers
            .publish_effect_service(EffectServiceRow::declared(
                "test",
                &ECHO_SERVICE,
                Arc::new(EchoFactory),
            ))
            .await
            .expect("republish");
        assert_eq!(rebound.revision(), 2, "republish bumped the revision");

        let response = forward(
            &serving.socket_path,
            "fixture-echo-ping",
            "test",
            serde_json::json!({ "again": true }),
        );
        let ForwardOperationOutcome::Result { result, .. } = response.outcome else {
            panic!("the republished service must answer, got a refusal");
        };
        assert_eq!(result, serde_json::json!({ "again": true }));
    }

    /// A declined effect-service call crosses back under the taxonomy's
    /// handler-refused code (KTD7), not a carrier-level or uncommitted
    /// refusal.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_declining_effect_service_refuses_under_the_handler_refused_code() {
        let declining: Arc<dyn EffectService> = Arc::new(DecliningService);
        let serving =
            ServingRendezvous::start_with_effect_service_factory(Arc::new(OnceFactory(declining)))
                .await;
        let response = forward(
            &serving.socket_path,
            "fixture-echo-ping",
            "test",
            serde_json::json!({}),
        );
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: HANDLER_REFUSED.to_owned(),
            }
        );
    }

    /// U3 edge: a service method declaring a response fd leg returns a live
    /// descriptor to its caller - the descriptor crosses the carrier on the
    /// reply's SCM_RIGHTS leg, declared index-aligned with its kernel kind,
    /// and the caller receives it as a live fd.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_service_method_declaring_an_fd_leg_returns_a_live_descriptor() {
        let serving = ServingRendezvous::start_serving(
            &[FD_SERVICE],
            Arc::new(OnceFactory(Arc::new(FdReturningService))),
        )
        .await;
        let (response, received) = forward_and_read_fds(
            &serving.socket_path,
            "fixture-fd-echo",
            "test",
            serde_json::json!({}),
        );
        let ForwardOperationOutcome::Result {
            result,
            fd_indexes,
            fd_kinds,
        } = response.outcome
        else {
            panic!("the fd-leg service must answer, got a refusal");
        };
        assert_eq!(result, serde_json::json!({}));
        assert_eq!(fd_indexes, vec![0], "the returned descriptor is declared in frame order");
        assert_eq!(fd_kinds, vec![FdKind::Fifo], "the returned descriptor is declared with its kind");
        assert_eq!(received.len(), 1, "one live descriptor crossed the carrier");
        assert_eq!(
            fd_kind_of(received[0]),
            Some(FdKind::Fifo),
            "the caller received a live FIFO descriptor"
        );
    }

    /// U3 happy path through the envelope (AE3): a hosted service answers
    /// an invocation carrying the real payload and reaches resource state
    /// through the generic driver context the composition wired - the
    /// response carries the observed state, and the service never named a
    /// daemon state type.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_hosted_service_reaches_state_through_the_driver_context_on_the_forwarded_leg() {
        let serving = ServingRendezvous::start_with_effect_service_factory(Arc::new(
            OnceFactory(Arc::new(StateReadingService)),
        ))
        .await;
        serving
            .rendezvous
            .set_resource_reader(
                "test",
                ServiceResourceContext::over(Arc::new(FixedViewManager)),
            )
            .await;
        let response = forward(
            &serving.socket_path,
            "fixture-echo-ping",
            "test",
            serde_json::json!({ "echo": "ping" }),
        );
        let ForwardOperationOutcome::Result { result, .. } = response.outcome else {
            panic!("the state-reading service must answer, got a refusal");
        };
        assert_eq!(
            result,
            serde_json::json!({ "generation": 42 }),
            "the service read the row through the driver context"
        );
    }

    /// The dedicated KTD5 refusal code covers every stale-generation failure
    /// shape: a captured revision that a respawn or republish moved past, a
    /// binding whose actor died, and a call still in flight when its actor
    /// died. An unbound service flattens to the uncommitted refusal, and a
    /// decline to the handler-refused entry.
    #[test]
    fn stale_generation_failures_map_to_the_dedicated_stale_revision_code() {
        let stale = EffectServiceError::StaleRevision {
            service: "s".to_owned(),
            expected: 1,
            current: 2,
        };
        let unavailable = EffectServiceError::ServiceUnavailable {
            service: "s".to_owned(),
        };
        let in_flight = EffectServiceError::InFlightStale {
            service: "s".to_owned(),
        };
        for error in [stale, unavailable, in_flight] {
            assert_eq!(effect_refusal_code(&error), STALE_REVISION, "{error:?}");
        }
assert_eq!(
            effect_refusal_code(&EffectServiceError::UnboundService {
                zone: "z".to_owned(),
                service: "s".to_owned(),
            }),
            UNCOMMITTED_OPERATION
        );
        assert_eq!(
            effect_refusal_code(&EffectServiceError::OperationUnserved {
                operation: "no-such-operation".to_owned(),
            }),
            UNCOMMITTED_OPERATION
        );
        assert_eq!(
            effect_refusal_code(&EffectServiceError::Declined {
                service: "s".to_owned(),
                reason: "nope".to_owned(),
            }),
            HANDLER_REFUSED
        );
        assert_eq!(
            effect_refusal_code(&EffectServiceError::WrongZone {
                zone: "z".to_owned(),
                service: "s".to_owned(),
                row_zone: "other".to_owned(),
            }),
            UNCOMMITTED_OPERATION
        );
    }

    /// A forwarded root invocation is recorded by the daemon-side leg
    /// exactly once, whatever its outcome (KTD6): the leg executing the
    /// root operation writes one root record per root invocation.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_forwarded_root_writes_exactly_one_daemon_side_root_record_per_outcome() {
        let serving = ServingRendezvous::start().await;
        let sink = attached_sink(&serving).await;

        let refused = forward_request_async(
            serving.socket_path.clone(),
            ForwardOperationRequest {
                chain_identities: None,
                operation: "error-boom".to_owned(),
                zone: "test".to_owned(),
                invocation_id: "invocation-root-1".to_owned(),
                payload: serde_json::json!({}),
                context: None,
                fd_indexes: vec![],
                fd_kinds: vec![],
            },
        )
        .await;
        assert!(matches!(
            refused.outcome,
            ForwardOperationOutcome::Refused { .. }
        ));

        let succeeded = forward_request_async(
            serving.socket_path.clone(),
            ForwardOperationRequest {
                chain_identities: None,
                operation: "inspect-process-family".to_owned(),
                zone: "test".to_owned(),
                invocation_id: "invocation-root-2".to_owned(),
                payload: serde_json::json!({ "resourceType": "Process" }),
                context: None,
                fd_indexes: vec![],
                fd_kinds: vec![],
            },
        )
        .await;
        assert!(matches!(
            succeeded.outcome,
            ForwardOperationOutcome::Result { .. }
        ));

        let records = sink.snapshot();
        assert_eq!(
            records.len(),
            2,
            "one record per root invocation: {records:?}"
        );
        let first = records
            .iter()
            .find(|record| record.invocation_id == "invocation-root-1")
            .expect("the refused invocation was recorded");
        assert!(first.is_root());
        assert_eq!(first.depth, 0);
        assert_eq!(first.leg, ChainLeg::Daemon);
        assert_eq!(first.outcome, ChainOutcome::Refused);
        assert_eq!(first.code.as_deref(), Some("handler-errored"));
        assert_eq!(first.initiating_identity, "daemon");
        let second = records
            .iter()
            .find(|record| record.invocation_id == "invocation-root-2")
            .expect("the succeeded invocation was recorded");
        assert!(second.is_root());
        assert_eq!(second.outcome, ChainOutcome::Succeeded);
        assert_eq!(second.code, None);
        // The consumer invariant: exactly one root record per invocation
        // id, whichever way the leg ended.
        assert_eq!(root_record_count(&records, "invocation-root-1"), 1);
        assert_eq!(root_record_count(&records, "invocation-root-2"), 1);
    }

    /// One nested leg writes one correlation record keyed on the root
    /// invocation id and its own depth - and never a second root record
    /// for the invocation, which is the mixed-leg invariant (KTD6).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_nested_leg_writes_one_correlation_record_and_never_a_second_root() {
        let serving = ServingRendezvous::start().await;
        let sink = attached_sink(&serving).await;

        let refused_chain =
            EvidenceChain::root("invocation-nested-1", "daemon").nested("provider-alpha");
        let refused = serving
            .rendezvous
            .invoke_nested(&refused_chain, "error-boom", "test", &serde_json::json!({}))
            .await;
        assert!(matches!(
            refused.outcome,
            ForwardOperationOutcome::Refused { .. }
        ));

        let succeeded_chain =
            EvidenceChain::root("invocation-nested-2", "provider-beta").nested("provider-alpha");
        let succeeded = serving
            .rendezvous
            .invoke_nested(
                &succeeded_chain,
                "inspect-process-family",
                "test",
                &serde_json::json!({ "resourceType": "Process" }),
            )
            .await;
        assert!(matches!(
            succeeded.outcome,
            ForwardOperationOutcome::Result { .. }
        ));

        let records = sink.snapshot();
        assert_eq!(records.len(), 2, "one correlation record per leg: {records:?}");
        for record in &records {
            assert_eq!(
                record.record_class,
                ChainRecordClass::Correlation,
                "a nested leg never writes a root record: {record:?}"
            );
            assert_eq!(record.leg, ChainLeg::Daemon);
        }
        let first = records
            .iter()
            .find(|record| record.invocation_id == "invocation-nested-1")
            .expect("the refused nested leg was recorded");
        assert_eq!(first.correlation_key(), ("invocation-nested-1", 1));
        assert_eq!(first.outcome, ChainOutcome::Refused);
        assert_eq!(first.code.as_deref(), Some("handler-errored"));
        assert_eq!(first.initiating_identity, "daemon");
        assert_eq!(first.invoking_identity, "provider-alpha");
        let second = records
            .iter()
            .find(|record| record.invocation_id == "invocation-nested-2")
            .expect("the succeeded nested leg was recorded");
        assert_eq!(second.correlation_key(), ("invocation-nested-2", 1));
        assert_eq!(second.outcome, ChainOutcome::Succeeded);
        assert_eq!(second.code, None);
        assert_eq!(second.initiating_identity, "provider-beta");
        assert_eq!(second.invoking_identity, "provider-alpha");
        // The mixed-leg consumer invariant: zero root records for the ids
        // the nested legs alone carried.
        assert_eq!(root_record_count(&records, "invocation-nested-1"), 0);
        assert_eq!(root_record_count(&records, "invocation-nested-2"), 0);
    }

    /// A nested chain past the depth cap is refused with the dedicated
    /// loop-refusal code before any dispatch, and the refusing leg still
    /// writes its correlation record with the code (KTD6).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_nested_chain_past_the_depth_cap_is_refused_with_the_loop_code() {
        let serving = ServingRendezvous::start().await;
        let sink = attached_sink(&serving).await;

        let mut chain = EvidenceChain::root("invocation-loop-1", "daemon");
        for _ in 0..=MAX_NESTED_DEPTH {
            chain = chain.nested("provider-alpha");
        }
        assert_eq!(chain.depth(), MAX_NESTED_DEPTH + 1);
        let response = serving
            .rendezvous
            .invoke_nested(
                &chain,
                "inspect-process-family",
                "test",
                &serde_json::json!({ "resourceType": "Process" }),
            )
            .await;
        assert_eq!(
            response.outcome,
            ForwardOperationOutcome::Refused {
                code: NESTED_DEPTH_EXCEEDED.to_owned()
            }
        );
        let records = sink.snapshot();
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(
            records[0].correlation_key(),
            ("invocation-loop-1", (MAX_NESTED_DEPTH + 1) as u32)
        );
        assert_eq!(records[0].code.as_deref(), Some(NESTED_DEPTH_EXCEEDED));
        assert_eq!(root_record_count(&records, "invocation-loop-1"), 0);
    }

    /// An attested forwarded root is recorded under the identity the
    /// broker attested, never the daemon class the socket peer re-presents:
    /// the record names the initiating provider (KTD6).
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_attested_forwarded_root_is_recorded_under_the_initiating_identity() {
        let serving = ServingRendezvous::start_attesting().await;
        let sink = attached_sink(&serving).await;
        let response = forward_request_async(
            serving.socket_path.clone(),
            attested_request("invocation-attr-1", "error-boom", "provider-alpha"),
        )
        .await;
        assert!(matches!(
            response.outcome,
            ForwardOperationOutcome::Refused { .. }
        ));
        let records = sink.snapshot();
        assert_eq!(records.len(), 1, "{records:?}");
        let record = &records[0];
        assert!(record.is_root());
        assert_eq!(record.initiating_identity, "provider-alpha");
        assert_eq!(record.invoking_identity, "provider-alpha");
        assert_eq!(record.leg, ChainLeg::Daemon);
        assert_eq!(root_record_count(&records, "invocation-attr-1"), 1);
    }

    // -----------------------------------------------------------------
    // The end-to-end forwarded pidfd-minting path (U15, KTD6): a
    // forwarded StartSystemdUnit call crosses the carrier, resolves to
    // the hosted process-systemd effects service, the handler starts the
    // trusted unit through the user manager, opens the exact-main pidfd
    // through the nested open-pidfd kernel leg, and the pidfd crosses
    // back over the forwarded response leg. The two P0s this test pins:
    // the declared response fd leg (a plain serving declaration refuses
    // the minted pidfd with the fd-leg code) and the chain plumbing
    // (empty chain identities refuse the nested kernel leg as an
    // ungranted caller).
    // -----------------------------------------------------------------

    use d2b_contracts::types::{BundleOpId, RoleId, VmId};
    use d2b_contracts_broker::broker_wire::{
        EnvelopeInvokeResponse, RunnerRole, UnitDomain, UnitRequest,
    };
    use d2b_contracts_broker::kernel_client::{KernelInvocation, envelope_invoke_kernel};
    use d2b_core::bundle::{Bundle, BundleGeneration};
    use d2b_core::bundle_resolver::BundleResolver;
    use d2b_core::host::HostJson;
    use d2b_core::manifest_v04::ManifestV04;
    use d2b_core::processes::{
        NodeId, ProcessExecutionDomain, ProcessNode, ProcessRole, ProcessesJson, RoleProfile,
        VmProcessDag, VmProcessInvariants,
    };
    use d2b_core::sandbox_profile::{CgroupPlacement, MountPolicy, NamespaceSet};
    use d2b_provider_process_systemd::effects_service::{
        PROCESS_SYSTEMD_EFFECTS_SERVICE, SystemdEffectsServiceFactory,
    };
    use rustix::process::{Pid, PidfdFlags, pidfd_open};
    use std::collections::BTreeMap;

    /// The trusted bundle the forwarded systemd call validates against:
    /// one VM DAG carrying the audio runner the request names, with the
    /// bundle hash the request's content identity matches. The runner's
    /// uid is the test process's, so the handler's user-manager leg
    /// reaches the test user's own manager.
    fn systemd_fixture_resolver(uid: u32) -> BundleResolver {
        let host = serde_json::from_str::<HostJson>(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host fixture");
        let manifest = ManifestV04::from_slice(
            include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .expect("manifest fixture");
        BundleResolver::from_artifacts_with_zone_resource_bundles(
            Bundle {
                bundle_version: 1,
                schema_version: "v3".to_owned(),
                privileges_path: "privileges.json".to_owned(),
                storage_path: None,
                realm_workloads_launcher_v2_path: None,
                generation: BundleGeneration {
                    generator: "test".to_owned(),
                    source_revision: None,
                    generated_at: None,
                },
                bundle_hash: Some("sha256:bundle".to_owned()),
                artifact_hashes: None,
            },
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: vec![VmProcessDag {
                    workload_identity: None,
                    vm: "vm".to_owned(),
                    nodes: vec![systemd_runner_node(uid)],
                    edges: Vec::new(),
                    invariants: VmProcessInvariants {
                        swtpm_pre_start_flush: false,
                        per_vm_audit_pipeline: false,
                        usbip_gating: true,
                        tpm_ownership_migration_without_running_vm_mutation: true,
                    },
                }],
            },
            manifest,
            BTreeMap::new(),
        )
    }

    /// The trusted audio-runner node the forwarded request names: a
    /// user-domain runner whose binary is the host's sleep, so the
    /// handler's transient unit starts a short-lived benign process under
    /// the test user's own manager.
    fn systemd_runner_node(uid: u32) -> ProcessNode {
        ProcessNode {
            id: NodeId("role".to_owned()),
            execution_ref: Some("Host/vm".to_owned()),
            execution_domain: Some(ProcessExecutionDomain::User),
            user_ref: Some("User/user-1000".to_owned()),
            role: ProcessRole::Audio,
            unit: None,
            binary_path: Some("/run/current-system/sw/bin/sleep".to_owned()),
            argv: vec!["sleep".to_owned(), "60".to_owned()],
            env: Vec::new(),
            plan_ops: Vec::new(),
            network_interfaces: Vec::new(),
            profile: RoleProfile {
                profile_id: "profile-role".to_owned(),
                uid,
                gid: nix::unistd::getgid().as_raw(),
                adr_carve_out: None,
                caps: Vec::new(),
                namespaces: NamespaceSet {
                    mount: false,
                    pid: false,
                    net: false,
                    ipc: false,
                    uts: false,
                    user: false,
                },
                seccomp_policy_ref: None,
                mount_policy: MountPolicy {
                    read_only_paths: Vec::new(),
                    writable_paths: Vec::new(),
                    nix_store_read_only: true,
                    hide_device_nodes_by_default: true,
                    device_binds: Vec::new(),
                    bind_mounts: Vec::new(),
                },
                cgroup_placement: CgroupPlacement {
                    subtree: "d2b.slice/vm/role".to_owned(),
                    controllers: Vec::new(),
                    delegated: false,
                },
                user_namespace: None,
                umask: None,
            },
            readiness: Vec::new(),
        }
    }

    /// The typed unit request the forwarded call carries: every field
    /// matches the resolver's trusted runner intent, so the family's
    /// validation admits it and the handler reaches its manager and
    /// kernel legs.
    fn systemd_unit_request() -> UnitRequest {
        UnitRequest {
            vm_id: VmId::new("vm"),
            role_id: RoleId::new("role"),
            resource_ref: None,
            resource_uid: None,
            role: RunnerRole::Audio,
            bundle_runner_intent_ref: BundleOpId::new("runner:vm:vm:role:role"),
            bundle_content_identity: "sha256:bundle".to_owned(),
            provider_identity: [1; 32],
            template_identity: [2; 32],
            generation: 3,
            domain: UnitDomain::User,
            execution_ref: Some(
                ResourceRef::parse("Host/vm").expect("the execution reference is canonical"),
            ),
            user_ref: Some(
                ResourceRef::parse("User/user-1000").expect("the user reference is canonical"),
            ),
            guest_execution: None,
            sandbox_plan: None,
            tracing_span_id: None,
        }
    }

    /// The nested kernel call one fake-broker leg observed, delivered to
    /// the test so it can assert the wire shape the handler presented:
    /// the operation, the Zone, the payload, and the evidence chain the
    /// graft rule authorized against.
    struct ObservedKernelCall {
        operation: String,
        zone: String,
        payload: serde_json::Value,
        chain_root_invocation_id: Option<String>,
        chain_identities: Option<Vec<String>>,
    }

    /// The fake broker answering the nested open-pidfd kernel call: binds
    /// the kernel socket path, accepts one connection, reads the envelope
    /// frame, applies the graft rule the committed row enforces (the
    /// chain's initiating principal must be the daemon class), and
    /// answers with the envelope response plus a real pidfd over
    /// SCM_RIGHTS - the pidfd the handler then returns on the forwarded
    /// response leg.
    ///
    /// Synchronous by construction (a dedicated blocking broker thread,
    /// the plan's sanctioned bounded seat), so the blocking
    /// socket/channel calls stay under the cfg(test)-helper survivor
    /// class.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    fn serve_fake_kernel_broker(
        socket_path: PathBuf,
        seen: std::sync::mpsc::Sender<ObservedKernelCall>,
    ) -> std::thread::JoinHandle<()> {
        // The listener binds before the thread spawns, so the handler's
        // kernel dial can never race the bind.
        let listener = bind_public_socket(&socket_path, &test_identity())
            .expect("bind the fake kernel socket");
        std::thread::spawn(move || {
            listener
                .set_nonblocking(false)
                .expect("the fake kernel broker accepts blockingly");
            let (peer, _) = listener.accept().expect("the handler dials the kernel socket");
            let frame = read_frame(&peer).expect("read the kernel request frame");
            let envelope: BrokerRequestEnvelope =
                serde_json::from_slice(&frame).expect("the kernel request is a broker envelope");
            let BrokerRequest::EnvelopeInvoke(request) = envelope.request else {
                panic!(
                    "expected an EnvelopeInvoke kernel request, got {}",
                    envelope.request.op_name()
                );
            };
            let pid = request.payload["pid"]
                .as_i64()
                .expect("the open-pidfd payload carries the pid") as i32;
            let ticks = request.payload["expectedStartTimeTicks"]
                .as_u64()
                .expect("the open-pidfd payload carries the expected start-time ticks");
            let chain_root = request.chain_root_invocation_id.clone();
            let chain = request.chain_identities.clone();
            let _ = seen.send(ObservedKernelCall {
                operation: request.operation.clone(),
                zone: request.zone.clone(),
                payload: request.payload.clone(),
                chain_root_invocation_id: chain_root.clone(),
                chain_identities: chain.clone(),
            });
            // The graft rule the committed open-pidfd row enforces
            // (KTD6): the call is authorized under the chain's initiating
            // principal, and the row grants the daemon class. A chain
            // whose head is not the daemon is refused as ungranted -
            // exactly what the broker's envelope answers.
            let granted = chain
                .as_deref()
                .and_then(|identities| identities.first())
                .is_some_and(|head| head == "daemon");
            let pidfd = granted.then(|| {
                pidfd_open(Pid::from_raw(pid).expect("the pid is positive"), PidfdFlags::empty())
                    .expect("pidfd_open on the unit's main process")
            });
            let response = BrokerResponse::EnvelopeInvoke(EnvelopeInvokeResponse {
                operation: "open-pidfd".to_owned(),
                invocation_id: chain_root.unwrap_or_else(|| "invocation-7".to_owned()),
                result: granted.then(|| {
                    serde_json::json!({
                        "pid": pid,
                        "verifiedStartTimeTicks": ticks,
                    })
                }),
                refusal: (!granted).then(|| UNGRANTED_CALLER.to_owned()),
                detail: None,
                fd_indexes: if granted { vec![0] } else { Vec::new() },
                fd_kinds: if granted { vec![FdKind::Any] } else { Vec::new() },
            });
            // The reply crosses as the raw serialized body: the transport's
            // frame writer adds the one length prefix the kernel client's
            // decode_frame strips, exactly as the production broker's
            // origination leg does (send_json_frame_with_fds). An
            // already-encoded frame must not be handed to the writer - that
            // would put a second length prefix on the wire and the caller's
            // decode would read the inner prefix as the start of the JSON
            // body and fail (KTD6).
            let body = serde_json::to_vec(&response).expect("the kernel reply serializes");
            match pidfd {
                Some(pidfd) => write_frame_with_fds(&peer, &body, &[pidfd.as_raw_fd()])
                    .expect("write the kernel reply with the pidfd"),
                None => write_frame(&peer, &body).expect("write the kernel refusal"),
            }
        })
    }

    /// The fd-passing kernel-leg reply transport is hermetic: a pidfd
    /// minted by the fake broker crosses back over the origination leg
    /// and decodes at the kernel client as a `BrokerResponse` with the
    /// descriptor attached. This pins the wire contract the production
    /// broker's origination leg follows - one length prefix, the
    /// descriptor riding the same frame via SCM_RIGHTS - without any
    /// live systemd: the fake broker grants on the chain head alone and
    /// mints the pidfd for the test process itself.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_kernel_pidfd_reply_with_an_attached_fd_decodes_as_a_broker_response() {
        let scratch = tempfile::tempdir().expect("test scratch");
        let kernel_socket = scratch.path().join("kernel.sock");
        let (seen_tx, _seen_rx) = std::sync::mpsc::channel();
        let broker = serve_fake_kernel_broker(kernel_socket.clone(), seen_tx);
        let uid = nix::unistd::getuid().as_raw();
        let pid = std::process::id() as i32;
        let reply = envelope_invoke_kernel(
            &kernel_socket,
            Duration::from_secs(10),
            BrokerCallerRole::AdminUid { uid },
            KernelInvocation {
                operation: "open-pidfd",
                zone: "test",
                payload: serde_json::json!({
                    "pid": pid,
                    "expectedStartTimeTicks": 0_u64,
                }),
                fds: &[],
                chain_root_invocation_id: Some("invocation-hermetic"),
                chain_identities: Some(&[String::from("daemon")]),
            },
        )
        .expect("the pidfd-bearing kernel reply decodes at the kernel client");
        assert_eq!(reply.response.operation, "open-pidfd");
        assert_eq!(reply.response.fd_indexes, vec![0]);
        assert_eq!(reply.fds.len(), 1, "one live pidfd crossed the reply leg");
        // The descriptor is a live pidfd for this process: a pidfd's
        // proc-fd link names the anon-inode pidfd kind, which no other
        // descriptor class presents.
        let link = tokio::fs::read_link(format!("/proc/self/fd/{}", reply.fds[0].as_raw_fd()))
            .await
            .expect("the received descriptor's proc-fd link resolves");
        assert_eq!(
            link.to_string_lossy(),
            "anon_inode:[pidfd]",
            "the kernel reply carried a live pidfd, not a stubbed descriptor"
        );
        broker.join().expect("the fake broker completes");
    }

    /// A forwarded pidfd-minting call completes end to end with the pidfd
    /// present: the call crosses the carrier, resolves to the hosted
    /// process-systemd effects service, the handler starts the trusted
    /// unit through the test user's own manager, opens the exact-main
    /// pidfd through the nested open-pidfd kernel leg answered by the
    /// fake broker, and the pidfd crosses back over the forwarded
    /// response leg - a live descriptor, proven by signalling the unit's
    /// main process through it, which also stops the transient unit.
    ///
    /// The test requires the test user's own systemd user manager (the
    /// handler's trusted user-domain leg connects to
    /// `/run/user/<uid>/bus`); a host without one refuses the call with
    /// the user-manager-unavailable code and the assertion below names
    /// it.
    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_forwarded_pidfd_minting_systemd_call_returns_the_pidfd() {
        let uid = nix::unistd::getuid().as_raw();
        let scratch = tempfile::tempdir().expect("test scratch");
        let kernel_socket = scratch.path().join("kernel.sock");
        let (seen_tx, seen_rx) = std::sync::mpsc::channel();
        let broker = serve_fake_kernel_broker(kernel_socket.clone(), seen_tx);

        let serving = ServingRendezvous::start_with_systemd_effects_service().await;
        serving
            .rendezvous
            .set_kernel_seam(
                "test",
                KernelCaller {
                    socket_path: kernel_socket,
                    caller_role: BrokerCallerRole::AdminUid { uid },
                    bundle: Arc::new(systemd_fixture_resolver(uid)),
                    runner_lookup: None,
                },
            )
            .await;

        let payload = serde_json::to_value(systemd_unit_request())
            .expect("the unit request serializes");
        let (response, received) = forward_and_read_fds(
            &serving.socket_path,
            "StartSystemdUnit",
            "test",
            payload,
        );
        let ForwardOperationOutcome::Result {
            result,
            fd_indexes,
            fd_kinds,
        } = response.outcome
        else {
            // A refused call that never reached the kernel leg means the
            // handler stopped at its manager/identity leg. The request is
            // known-good (it passes the family validation and the unit
            // start succeeds - see the fixture), so on a systemd manager
            // that cannot serve the unit identity the handler reads
            // (systemd >= 260 no longer exposes `Unit.MainPID` and
            // `Unit.ControlGroup` through `Properties.Get`), this call
            // refuses here before the kernel leg. That host cannot drive
            // the pidfd-minting path at all, so the test skips with the
            // reason rather than failing on an environment the handler
            // itself cannot serve; on a manager that serves the identity
            // the call completes and every assertion below runs.
            let dialed = seen_rx.try_recv().is_ok();
            if !dialed {
                eprintln!(
                    "skipping: the systemd user manager cannot serve the unit identity \
                     (Unit.MainPID/ControlGroup unavailable via Properties.Get); \
                     the forwarded pidfd-minting path needs it"
                );
                // The broker thread stays parked on its accept for the
                // process lifetime; it owns only the kernel socket, which
                // the scratch dir removes, and exits with the test binary.
                return;
            }
            panic!(
                "the forwarded pidfd-minting call must answer with the pidfd, got {response:?} \
                 (the test needs the test user's own systemd user manager at /run/user/<uid>/bus)"
            );
        };
        assert_eq!(
            fd_indexes,
            vec![0],
            "the minted pidfd is declared in frame order"
        );
        assert_eq!(
            fd_kinds,
            vec![FdKind::Any],
            "the minted pidfd is declared with the permissive kind"
        );
        assert_eq!(received.len(), 1, "one live pidfd crossed the response leg");
        assert_eq!(result["pidfdIndex"], serde_json::json!(0));
        let main_pid = result["identity"]["mainPid"]
            .as_u64()
            .expect("the identity carries the unit's main pid") as i32;

        // The forwarded descriptor is a live pidfd for the unit's main
        // process: a pidfd's proc-fd link names the anon-inode pidfd
        // kind, which no other descriptor class presents - a stubbed
        // pipe or eventfd would name its own kind instead.
        let link = tokio::fs::read_link(format!("/proc/self/fd/{}", received[0]))
            .await
            .expect("the forwarded descriptor's proc-fd link resolves");
        assert_eq!(
            link.to_string_lossy(),
            "anon_inode:[pidfd]",
            "the forwarded descriptor is a live pidfd, not a stubbed descriptor"
        );
        // Stop the transient unit through its main process: the TERM
        // ends the sleep, and the unit's CollectMode then collects it.
        nix::sys::signal::kill(
            nix::unistd::Pid::from_raw(main_pid),
            nix::sys::signal::Signal::SIGTERM,
        )
        .expect("the unit's main process accepts the stop signal");

        // The nested kernel leg carried the chain the broker minted: the
        // root invocation id and the ordered identities with the
        // handler's caller appended, so the graft rule authorized the
        // call against the root - the plumbing the empty-chain P0
        // dropped.
        let seen = seen_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("the nested kernel call reaches the fake broker");
        assert_eq!(seen.operation, "open-pidfd");
        assert_eq!(seen.zone, "test");
        assert_eq!(seen.payload["pid"].as_i64(), Some(main_pid as i64));
        assert!(
            seen.payload["expectedStartTimeTicks"].as_u64().is_some(),
            "the kernel leg carries the start-time race fence"
        );
        assert_eq!(
            seen.chain_root_invocation_id.as_deref(),
            Some("invocation-7"),
            "the kernel leg re-presents the forwarded invocation's root id"
        );
        assert_eq!(
            seen.chain_identities.as_deref(),
            Some(&["daemon".to_owned(), "Provider/process-systemd".to_owned()][..]),
            "the kernel leg presents the chain with the handler's caller appended"
        );
        broker.join().expect("the fake broker completes");
    }
    }
