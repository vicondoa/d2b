//! Typed v3 Zone resource calls.
//!
//! This module is the small caller-side seam between the canonical
//! [`ResourceClient`] policy engine and an authenticated ComponentSession
//! supplied by the Zone runtime. It does not establish Noise, resolve a
//! subject, own a socket, or carry a credential. Those operations stay in the
//! session and bus layers.

use core::future::Future;
use std::{
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use d2b_contracts::v3::{CanonicalJsonObject, ResourceRef};

use crate::{
    AttemptDisposition, CallDriver, CallOptions, ClientError, MethodProfile, ResolvedTarget,
    ResourceClient, SessionFailure, SystemClock, TargetInput, TargetResolver, TransportKind,
    TransportSelection, WallClock, ZoneServiceKind, call::REQUEST_ID_BYTES,
};

/// The exact v3 ResourceService method inventory.
///
/// This is an alias of the contract-owned catalogue rather than a second
/// client-side enum, so adding or removing a method changes the canonical
/// service descriptor and this API together.
pub use d2b_contracts::v3::ResourceMethod as ResourceVerb;

/// Whether a ResourceService method can change durable Resource state.
pub const fn resource_verb_is_mutating(verb: ResourceVerb) -> bool {
    matches!(
        verb,
        ResourceVerb::Create
            | ResourceVerb::UpdateSpec
            | ResourceVerb::UpdateStatus
            | ResourceVerb::UpdateMetadata
            | ResourceVerb::UpdateFinalizers
            | ResourceVerb::Delete
            | ResourceVerb::CommitBatch
            | ResourceVerb::Upgrade
    )
}

/// Evidence for one authenticated Zone peer.
///
/// The static-key value is a fingerprint supplied by the transport/session
/// adapter, not key material. Constructing this evidence does not construct a
/// subject or an authorization decision; the bus/session authority remains the
/// only owner of those decisions.
#[derive(Clone, PartialEq, Eq)]
pub struct ZonePeerIdentity {
    zone: d2b_contracts::v3::zone_routing::ZonePath,
    static_key_fingerprint: [u8; 32],
}

impl ZonePeerIdentity {
    /// Construct transport evidence observed for an authenticated peer.
    pub const fn from_observed_static_key(
        zone: d2b_contracts::v3::zone_routing::ZonePath,
        static_key_fingerprint: [u8; 32],
    ) -> Self {
        Self {
            zone,
            static_key_fingerprint,
        }
    }

    /// Construct transport evidence from an enrolled peer key fingerprint.
    pub const fn from_enrolled_peer(
        zone: d2b_contracts::v3::zone_routing::ZonePath,
        static_key_fingerprint: [u8; 32],
    ) -> Self {
        Self::from_observed_static_key(zone, static_key_fingerprint)
    }

    /// Borrow the exact Zone route identity established by the adapter.
    pub const fn zone(&self) -> &d2b_contracts::v3::zone_routing::ZonePath {
        &self.zone
    }

    /// Borrow the static-key fingerprint used for the pin comparison.
    pub const fn static_key_fingerprint(&self) -> &[u8; 32] {
        &self.static_key_fingerprint
    }
}

impl fmt::Debug for ZonePeerIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ZonePeerIdentity(<redacted>)")
    }
}

/// Immutable pin for one authenticated Zone session.
///
/// A session is usable only for the exact Zone route, service, carriage,
/// reconnect generation, and authenticated transcript it was established
/// for. The pin is evidence carried out of the session adapter; it is not a
/// caller-supplied subject or a capability.
#[derive(Clone, PartialEq, Eq)]
pub struct ZoneSessionPin {
    peer: ZonePeerIdentity,
    service: ZoneServiceKind,
    transport: TransportKind,
    reconnect_generation: u64,
    transcript_hash: [u8; 32],
}

impl ZoneSessionPin {
    /// Bind authenticated session evidence to one exact route.
    pub fn new(
        peer: ZonePeerIdentity,
        service: ZoneServiceKind,
        transport: TransportKind,
        reconnect_generation: u64,
        transcript_hash: [u8; 32],
    ) -> Result<Self, ClientError> {
        if reconnect_generation == 0
            || peer.static_key_fingerprint() == &[0; 32]
            || transcript_hash == [0; 32]
        {
            return Err(ClientError::ContractViolation);
        }
        Ok(Self {
            peer,
            service,
            transport,
            reconnect_generation,
            transcript_hash,
        })
    }

    /// Borrow the authenticated peer evidence.
    pub const fn peer(&self) -> &ZonePeerIdentity {
        &self.peer
    }

    /// The service bound into the session transcript.
    pub const fn service(&self) -> ZoneServiceKind {
        self.service
    }

    /// The carriage class bound into the session transcript.
    pub const fn transport(&self) -> TransportKind {
        self.transport
    }

    /// The nonzero reconnect generation bound into the session.
    pub const fn reconnect_generation(&self) -> u64 {
        self.reconnect_generation
    }

    /// Borrow the opaque transcript hash for an internal equality check.
    pub const fn transcript_hash(&self) -> &[u8; 32] {
        &self.transcript_hash
    }

    /// Check that this authenticated session is pinned to one resolved route.
    pub fn matches_target(&self, target: &ResolvedTarget, service: ZoneServiceKind) -> bool {
        self.service == service
            && self.transport == target.transport()
            && self.peer.zone() == target.owner().zone()
    }
}

impl fmt::Debug for ZoneSessionPin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ZoneSessionPin(<redacted>)")
    }
}

/// A local Zone peer pin verifier.
///
/// The allocator/session adapter supplies the observed peer identity and
/// performs the authenticated static-key handshake. This type only compares
/// the resulting evidence before a request is admitted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneSocketConnector {
    expected_peer: ZonePeerIdentity,
}

impl ZoneSocketConnector {
    /// Build a verifier from trusted allocator configuration.
    pub const fn new(expected_peer: ZonePeerIdentity) -> Self {
        Self { expected_peer }
    }

    /// Return the configured peer pin.
    pub const fn expected_peer(&self) -> &ZonePeerIdentity {
        &self.expected_peer
    }

    /// Verify the transport-observed peer before any request is sent.
    pub fn verify_peer(&self, peer: &ZonePeerIdentity) -> Result<(), ClientError> {
        if peer == &self.expected_peer {
            Ok(())
        } else {
            Err(ClientError::TransportPolicyMismatch)
        }
    }

    /// Verify the peer portion of an authenticated session pin.
    pub fn verify_session_pin(&self, pin: &ZoneSessionPin) -> Result<(), ClientError> {
        self.verify_peer(pin.peer())
    }

    /// Return the endpoint identity pinned for the local Zone runtime.
    pub fn local_daemon_endpoint_identity(&self) -> ZonePeerIdentity {
        self.expected_peer.clone()
    }
}

/// One authenticated Zone session supplied by the session adapter.
///
/// The default timeout and cancellation hooks preserve compatibility with
/// small test adapters while allowing the real ComponentSession bridge to
/// forward the exact deadline and request-cancel operation.
pub trait ConnectedZoneSession: Send + Sync {
    /// Issue one canonical Resource request.
    fn call(
        &self,
        verb: ResourceVerb,
        target: Option<ResourceRef>,
        payload: CanonicalJsonObject,
    ) -> impl Future<Output = Result<CanonicalJsonObject, ClientError>> + Send;

    /// Issue one request with the attempt's monotonic timeout.
    fn call_with_timeout(
        &self,
        verb: ResourceVerb,
        target: Option<ResourceRef>,
        payload: CanonicalJsonObject,
        _relative_timeout_nanos: u64,
    ) -> impl Future<Output = Result<CanonicalJsonObject, ClientError>> + Send {
        self.call(verb, target, payload)
    }

    /// Forward cancellation for the request currently in flight.
    fn cancel(
        &self,
        _request_id: [u8; REQUEST_ID_BYTES],
    ) -> impl Future<Output = Result<(), ClientError>> + Send {
        core::future::ready(Ok(()))
    }
}

/// Opens an authenticated Zone session for an already resolved route.
///
/// The connector returns the session evidence separately from the session
/// object. [`ZoneClient`] checks that evidence against the resolved target
/// before exposing the connected handle.
pub trait ZoneSessionConnector: Send + Sync {
    /// The concrete session returned by this connector.
    type Session: ConnectedZoneSession;

    /// Establish one session over the exact resolved route.
    fn connect(
        &self,
        target: &ResolvedTarget,
        service: ZoneServiceKind,
    ) -> impl Future<Output = Result<(Self::Session, ZoneSessionPin), ClientError>> + Send;
}

/// Compatibility name for the transport-neutral ComponentSession seam.
pub use ZoneSessionConnector as ComponentSessionConnector;

/// A connected session whose route and authentication pin cannot be changed.
pub struct ConnectedZoneClient<S> {
    target: ResolvedTarget,
    pin: ZoneSessionPin,
    session: S,
}

impl<S> ConnectedZoneClient<S> {
    /// Borrow the exact route selected before session establishment.
    pub const fn target(&self) -> &ResolvedTarget {
        &self.target
    }

    /// Borrow the authenticated session pin.
    pub const fn session_pin(&self) -> &ZoneSessionPin {
        &self.pin
    }

    /// Borrow the session adapter.
    pub const fn session(&self) -> &S {
        &self.session
    }
}

impl<S> fmt::Debug for ConnectedZoneClient<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ConnectedZoneClient")
            .field("target", &self.target)
            .field("service", &self.pin.service())
            .field("session", &"<authenticated>")
            .finish()
    }
}

/// Options for one typed Resource call.
pub struct ResourceCallOptions<'a> {
    /// Canonical request payload.
    pub payload: CanonicalJsonObject,
    /// Whether the call carries attachments and therefore cannot be retried.
    pub has_attachments: bool,
    /// Cooperative cancellation state for the call.
    pub cancellation: &'a crate::CancellationToken,
}

impl<'a> ResourceCallOptions<'a> {
    /// Assemble options for one typed Resource call.
    pub const fn new(
        payload: CanonicalJsonObject,
        has_attachments: bool,
        cancellation: &'a crate::CancellationToken,
    ) -> Self {
        Self {
            payload,
            has_attachments,
            cancellation,
        }
    }
}

/// A named Resource Watch stream supplied by the authenticated session.
pub trait ResourceWatchTransport: Send + Sync {
    /// Receive one bounded canonical event, or `None` after terminal close.
    fn receive_watch_event(
        &self,
    ) -> impl Future<Output = Result<Option<CanonicalJsonObject>, ClientError>> + Send;

    /// Close the server stream and release its session credits.
    fn close_watch(&self) -> impl Future<Output = Result<(), ClientError>> + Send;
}

/// Client-side ownership of one Resource Watch stream.
///
/// Closing is idempotent. Dropping the wrapper cannot perform async I/O, so
/// callers must call [`ResourceWatch::close`] when they stop consuming.
pub struct ResourceWatch<S> {
    transport: S,
    state: Arc<AtomicBool>,
    closing: Arc<AtomicBool>,
}

impl<S> ResourceWatch<S> {
    /// Bind a transport-owned named Watch stream.
    pub fn new(transport: S) -> Self {
        Self {
            transport,
            state: Arc::new(AtomicBool::new(false)),
            closing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Borrow the underlying stream adapter.
    pub const fn transport(&self) -> &S {
        &self.transport
    }

    /// Whether close has completed or the peer has ended the stream.
    pub fn is_closed(&self) -> bool {
        self.state.load(Ordering::Acquire)
    }

    fn is_open(&self) -> bool {
        !self.state.load(Ordering::Acquire) && !self.closing.load(Ordering::Acquire)
    }
}

impl<S> fmt::Debug for ResourceWatch<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ResourceWatch")
            .field("closed", &self.is_closed())
            .finish_non_exhaustive()
    }
}

impl<S> ResourceWatch<S>
where
    S: ResourceWatchTransport,
{
    /// Receive one Watch event.
    pub async fn next(&self) -> Result<Option<CanonicalJsonObject>, ClientError> {
        if !self.is_open() {
            return Ok(None);
        }
        let event = self.transport.receive_watch_event().await?;
        if event.is_none() {
            self.state.store(true, Ordering::Release);
        }
        Ok(event)
    }

    /// Close the Watch stream exactly once after a successful remote close.
    pub async fn close(&self) -> Result<(), ClientError> {
        if self.state.load(Ordering::Acquire) {
            return Ok(());
        }
        if self
            .closing
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Ok(());
        }
        match self.transport.close_watch().await {
            Ok(()) => {
                self.state.store(true, Ordering::Release);
                Ok(())
            }
            Err(error) => {
                self.closing.store(false, Ordering::Release);
                Err(error)
            }
        }
    }
}

/// A ResourceClient facade that also binds a typed Zone session connector.
pub struct ZoneClient<R, C, W = SystemClock> {
    resource: ResourceClient<R, W>,
    connector: C,
}

/// Descriptive alias for callers using the Zone service terminology.
pub type ZoneServiceClient<R, C, W = SystemClock> = ZoneClient<R, C, W>;

impl<R, C> ZoneClient<R, C, SystemClock> {
    /// Construct a Zone client with the system wall clock.
    pub fn new(resolver: R, connector: C) -> Self {
        Self {
            resource: ResourceClient::new(resolver),
            connector,
        }
    }
}

impl<R, C, W> ZoneClient<R, C, W> {
    /// Construct a Zone client with an explicit wall clock.
    pub fn with_clock(resolver: R, connector: C, clock: W) -> Self {
        Self {
            resource: ResourceClient::with_clock(resolver, clock),
            connector,
        }
    }

    /// Borrow the underlying route/retry client.
    pub const fn resource_client(&self) -> &ResourceClient<R, W> {
        &self.resource
    }

    /// Borrow the connector.
    pub const fn connector(&self) -> &C {
        &self.connector
    }
}

impl<R, C, W> ZoneClient<R, C, W>
where
    R: TargetResolver,
    W: WallClock,
{
    /// Resolve a target and prepare one bounded Resource call.
    pub fn prepare_resource_call(
        &self,
        target: &TargetInput,
        verb: ResourceVerb,
        options: CallOptions,
        selection: TransportSelection,
        has_attachments: bool,
    ) -> Result<(ResolvedTarget, CallDriver<W>), ClientError> {
        let resolved = self
            .resource
            .resolve(target, ZoneServiceKind::Resource, selection)?;
        let profile = method_profile_for_service(ZoneServiceKind::Resource, verb, &options)?;
        let driver = self
            .resource
            .prepare_call(&resolved, profile, options, has_attachments)?;
        Ok((resolved, driver))
    }

    /// Establish a session over the exact route selected by the resolver.
    pub async fn connect(
        &self,
        target: &TargetInput,
        service: ZoneServiceKind,
        selection: TransportSelection,
    ) -> Result<ConnectedZoneClient<C::Session>, ClientError>
    where
        C: ZoneSessionConnector,
    {
        let resolved = self.resource.resolve(target, service, selection)?;
        let (session, pin) = self.connector.connect(&resolved, service).await?;
        if !pin.matches_target(&resolved, service) {
            return Err(ClientError::TransportPolicyMismatch);
        }
        Ok(ConnectedZoneClient {
            target: resolved,
            pin,
            session,
        })
    }

    /// Execute one Resource call over a caller-supplied authenticated session.
    ///
    /// New callers should prefer [`Self::connect`] plus
    /// [`Self::call_connected`], which binds the session to the route pin.
    /// This lower-level form remains useful to the bus adapter, which already
    /// owns the authenticated session binding.
    pub async fn call_resource<S>(
        &self,
        session: &S,
        target: &TargetInput,
        verb: ResourceVerb,
        options: CallOptions,
        selection: TransportSelection,
        request: ResourceCallOptions<'_>,
    ) -> Result<CanonicalJsonObject, ClientError>
    where
        S: ConnectedZoneSession,
    {
        let (resolved, _driver) =
            self.prepare_resource_call(target, verb, options, selection, request.has_attachments)?;
        execute_resource_call(&self.resource, session, &resolved, verb, _driver, request).await
    }

    /// Execute a typed call over a handle whose authenticated route pin was
    /// checked by [`Self::connect`].
    pub async fn call_connected(
        &self,
        connection: &ConnectedZoneClient<C::Session>,
        verb: ResourceVerb,
        options: CallOptions,
        request: ResourceCallOptions<'_>,
    ) -> Result<CanonicalJsonObject, ClientError>
    where
        C: ZoneSessionConnector,
    {
        if !connection
            .session_pin()
            .matches_target(connection.target(), connection.target().service())
        {
            return Err(ClientError::TransportPolicyMismatch);
        }
        let profile = method_profile_for_service(connection.target().service(), verb, &options)?;
        let driver = self.resource.prepare_call(
            connection.target(),
            profile,
            options,
            request.has_attachments,
        )?;
        execute_resource_call(
            &self.resource,
            connection.session(),
            connection.target(),
            verb,
            driver,
            request,
        )
        .await
    }
}

fn method_profile_for_service(
    service: ZoneServiceKind,
    verb: ResourceVerb,
    options: &CallOptions,
) -> Result<MethodProfile, ClientError> {
    let lifetime_ms = options
        .metadata
        .expires_at_unix_ms()
        .checked_sub(options.metadata.issued_at_unix_ms())
        .and_then(|value| u32::try_from(value).ok())
        .ok_or(ClientError::InvalidMetadata)?;
    MethodProfile::new(
        service,
        resource_verb_is_mutating(verb),
        resource_verb_is_mutating(verb),
        lifetime_ms,
    )
}

async fn execute_resource_call<S, R, W>(
    _resource: &ResourceClient<R, W>,
    session: &S,
    target: &ResolvedTarget,
    verb: ResourceVerb,
    mut driver: CallDriver<W>,
    request: ResourceCallOptions<'_>,
) -> Result<CanonicalJsonObject, ClientError>
where
    S: ConnectedZoneSession,
    W: WallClock,
{
    let ResourceCallOptions {
        payload,
        has_attachments: _,
        cancellation,
    } = request;
    let resource_target = target.resource_ref();
    let request_id = *driver.request_id();

    loop {
        let attempt = driver.begin_attempt(cancellation)?;
        let call = session.call_with_timeout(
            verb,
            resource_target.clone(),
            payload.clone(),
            attempt.relative_timeout_nanos(),
        );
        let result = match await_with_cancellation(call, cancellation).await {
            Ok(result) => result,
            Err(ClientError::Cancelled) => {
                let _ = session.cancel(request_id).await;
                return Err(ClientError::Cancelled);
            }
            Err(error) => return Err(error),
        };

        match result {
            Ok(response) => return Ok(response),
            Err(error) => match classify_session_error(&driver, error) {
                AttemptDisposition::RetryNow => continue,
                AttemptDisposition::RetryAfterMs(delay) => {
                    let sleep = tokio::time::sleep(Duration::from_millis(u64::from(delay)));
                    await_with_cancellation(sleep, cancellation).await?;
                }
                AttemptDisposition::Fail(error) => return Err(error),
            },
        }
    }

    async fn await_with_cancellation<F, T>(
        future: F,
        cancellation: &crate::CancellationToken,
    ) -> Result<T, ClientError>
    where
        F: Future<Output = T> + Send,
    {
        let mut future = Box::pin(future);
        let mut cancelled = Box::pin(cancellation.cancelled());
        core::future::poll_fn(move |context| {
            if let core::task::Poll::Ready(value) = future.as_mut().poll(context) {
                return core::task::Poll::Ready(Ok(value));
            }
            if let core::task::Poll::Ready(()) = cancelled.as_mut().poll(context) {
                return core::task::Poll::Ready(Err(ClientError::Cancelled));
            }
            core::task::Poll::Pending
        })
        .await
    }
}

fn classify_session_error<W: WallClock>(
    driver: &CallDriver<W>,
    error: ClientError,
) -> AttemptDisposition {
    match error {
        ClientError::SessionLost => driver.record_session_failure(SessionFailure::Disconnected),
        ClientError::TransportFailed => driver.record_session_failure(SessionFailure::Retryable),
        ClientError::DeadlineExpired => driver.record_session_failure(SessionFailure::Deadline),
        ClientError::Cancelled => driver.record_session_failure(SessionFailure::Cancelled),
        ClientError::ContractViolation => driver.record_session_failure(SessionFailure::Protocol),
        ClientError::Remote { kind, retry } => driver.record_remote_verdict(kind, retry),
        other => AttemptDisposition::Fail(other),
    }
}

/// Local Zone session wrapper used by attachment clients.
pub struct LocalZoneSession<S> {
    pin: ZoneSessionPin,
    session: S,
}

impl<S> LocalZoneSession<S> {
    /// Bind a session to evidence produced by the authenticated adapter.
    pub const fn new(pin: ZoneSessionPin, session: S) -> Self {
        Self { pin, session }
    }

    /// Borrow the authenticated session pin.
    pub const fn session_pin(&self) -> &ZoneSessionPin {
        &self.pin
    }

    /// Borrow the Zone path in the pin.
    pub const fn zone(&self) -> &d2b_contracts::v3::zone_routing::ZonePath {
        self.pin.peer().zone()
    }

    /// Borrow the connected session.
    pub const fn session(&self) -> &S {
        &self.session
    }
}

impl<S> fmt::Debug for LocalZoneSession<S> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("LocalZoneSession(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ServiceOwner;
    use crate::target::fixtures::zone;

    fn peer(zone: &str, key: u8) -> ZonePeerIdentity {
        ZonePeerIdentity::from_observed_static_key(
            crate::target::fixtures::zone(&[zone]),
            [key; 32],
        )
    }

    #[test]
    fn resource_method_inventory_is_the_contract_catalogue() {
        assert_eq!(ResourceVerb::ALL.len(), 13);
        assert_eq!(ResourceVerb::UpdateSpec.as_str(), "UpdateSpec");
        assert!(resource_verb_is_mutating(ResourceVerb::CommitBatch));
        assert!(!resource_verb_is_mutating(ResourceVerb::InspectSchema));
    }

    #[test]
    fn peer_and_session_pins_are_exact_and_redacted() {
        let first_peer = peer("k1", 1);
        let second_peer = peer("k1", 2);
        let pin = ZoneSessionPin::new(
            first_peer.clone(),
            ZoneServiceKind::Resource,
            TransportKind::ZoneLink,
            1,
            [7; 32],
        )
        .unwrap();
        let target = crate::RouteTable::new(vec![crate::RouteRecord::new(
            ServiceOwner::Zone(zone(&["k1"])),
            TransportKind::ZoneLink,
        )])
        .resolve(
            &TargetInput::ZoneService(zone(&["k1"]), ZoneServiceKind::Resource),
            ZoneServiceKind::Resource,
            TransportSelection::exact(TransportKind::ZoneLink),
        )
        .unwrap();
        assert!(pin.matches_target(&target, ZoneServiceKind::Resource));
        assert!(
            !ZoneSocketConnector::new(second_peer)
                .verify_session_pin(&pin)
                .is_ok()
        );
        assert_eq!(format!("{first_peer:?}"), "ZonePeerIdentity(<redacted>)");
        assert_eq!(format!("{pin:?}"), "ZoneSessionPin(<redacted>)");
    }

    #[test]
    fn zero_session_generation_is_rejected() {
        assert_eq!(
            ZoneSessionPin::new(
                peer("k1", 1),
                ZoneServiceKind::Resource,
                TransportKind::ZoneLink,
                0,
                [0; 32],
            )
            .unwrap_err(),
            ClientError::ContractViolation
        );
    }
}
