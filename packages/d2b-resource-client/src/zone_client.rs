//! Typed Zone client and local socket connector seam.
//!
//! The connector owns authenticated transport establishment; the client owns
//! only Resource verb selection, target routing, and bounded retry policy.
//! This keeps peer identity pinning explicit without importing socket paths or
//! file descriptors into the public client API.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{fmt, future::Future};

use d2b_contracts::v3::RetryClass;
use d2b_contracts::v3::{CanonicalJsonObject, ResourceRef, ZoneId};

use crate::{
    AttemptDisposition, CallDriver, CallOptions, ClientError, MethodProfile, ResolvedTarget,
    ResourceClient, SessionFailure, SystemClock, TargetInput, TargetResolver, TransportSelection,
    WallClock, ZoneServiceKind,
};

/// The closed v3 Resource verb table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ResourceVerb {
    /// Get one resource.
    Get,
    /// List resources.
    List,
    /// Watch resources.
    Watch,
    /// Create a resource.
    Create,
    /// Update desired spec.
    UpdateSpec,
    /// Update observed status.
    UpdateStatus,
    /// Delete a resource.
    Delete,
}

impl ResourceVerb {
    /// Stable wire method name.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Get => "resource-get",
            Self::List => "resource-list",
            Self::Watch => "resource-watch",
            Self::Create => "resource-create",
            Self::UpdateSpec => "resource-update-spec",
            Self::UpdateStatus => "resource-update-status",
            Self::Delete => "resource-delete",
        }
    }
}

/// Kernel-observed local Zone runtime peer identity.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ZonePeerIdentity {
    uid: u32,
}

impl ZonePeerIdentity {
    /// Construct evidence from the local transport adapter.
    pub const fn from_observed_uid(uid: u32) -> Self {
        Self { uid }
    }

    /// Return the observed uid for an internal pin comparison.
    pub const fn uid(self) -> u32 {
        self.uid
    }
}

impl fmt::Debug for ZonePeerIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ZonePeerIdentity(<redacted>)")
    }
}

/// Local d2b-zonert peer pin.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZoneSocketConnector {
    expected_uid: u32,
}

impl ZoneSocketConnector {
    /// Build a connector from a trusted service-manager uid.
    pub const fn new(expected_uid: u32) -> Self {
        Self { expected_uid }
    }

    /// Verify the kernel-observed peer before any request is sent.
    pub const fn verify_peer(self, peer: ZonePeerIdentity) -> Result<(), ClientError> {
        if peer.uid() == self.expected_uid {
            Ok(())
        } else {
            Err(ClientError::TransportFailed)
        }
    }

    /// Return the pinned runtime uid for diagnostics-free internal setup.
    pub const fn expected_uid(self) -> u32 {
        self.expected_uid
    }

    /// Return the endpoint identity pinned for the local Zone runtime.
    ///
    /// The returned value is still only transport evidence; it is not a
    /// caller-selectable subject or authorization grant.
    pub const fn local_daemon_endpoint_identity(self) -> ZonePeerIdentity {
        ZonePeerIdentity::from_observed_uid(self.expected_uid)
    }
}

/// One authenticated Zone session supplied by a connector.
pub trait ConnectedZoneSession: Send + Sync {
    /// Issue one canonical Resource request.
    fn call(
        &self,
        verb: ResourceVerb,
        target: Option<ResourceRef>,
        payload: CanonicalJsonObject,
    ) -> impl Future<Output = Result<CanonicalJsonObject, ClientError>> + Send;
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
/// callers must call [`ResourceWatch::close`] when they stop consuming; the
/// closed flag makes repeated teardown safe during cancellation races.
pub struct ResourceWatch<S> {
    transport: S,
    closed: Arc<AtomicBool>,
}

impl<S> ResourceWatch<S> {
    /// Bind a transport-owned named Watch stream.
    pub fn new(transport: S) -> Self {
        Self {
            transport,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Borrow the underlying stream adapter.
    pub const fn transport(&self) -> &S {
        &self.transport
    }

    /// Whether close has been requested.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
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
        if self.is_closed() {
            return Ok(None);
        }
        let event = self.transport.receive_watch_event().await?;
        if event.is_none() {
            self.closed.store(true, Ordering::Release);
        }
        Ok(event)
    }

    /// Close the Watch stream exactly once.
    pub async fn close(&self) -> Result<(), ClientError> {
        if self
            .closed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            self.transport.close_watch().await
        } else {
            Ok(())
        }
    }
}

/// A ResourceClient facade that also binds a typed session connector.
pub struct ZoneClient<R, C, W = SystemClock> {
    resource: ResourceClient<R, W>,
    connector: C,
}

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
        let lifetime_ms = u32::try_from(
            options
                .metadata
                .expires_at_unix_ms()
                .saturating_sub(options.metadata.issued_at_unix_ms()),
        )
        .map_err(|_| ClientError::InvalidMetadata)?;
        let mutating = matches!(
            verb,
            ResourceVerb::Create
                | ResourceVerb::UpdateSpec
                | ResourceVerb::UpdateStatus
                | ResourceVerb::Delete
        );
        let profile =
            MethodProfile::new(ZoneServiceKind::Resource, mutating, mutating, lifetime_ms)?;
        let driver = self
            .resource
            .prepare_call(&resolved, profile, options, has_attachments)?;
        Ok((resolved, driver))
    }

    /// Execute one typed Resource call over an already-authenticated Zone
    /// session, applying the bounded retry and cancellation policy.
    pub async fn call_resource(
        &self,
        session: &C,
        target: &TargetInput,
        verb: ResourceVerb,
        options: CallOptions,
        selection: TransportSelection,
        payload: CanonicalJsonObject,
        has_attachments: bool,
        cancellation: &crate::CancellationToken,
    ) -> Result<CanonicalJsonObject, ClientError>
    where
        C: ConnectedZoneSession,
    {
        let (_resolved, mut driver) =
            self.prepare_resource_call(target, verb, options, selection, has_attachments)?;
        let resource_target = target_resource_ref(target);
        loop {
            let _attempt = driver.begin_attempt(cancellation)?;
            match session
                .call(verb, resource_target.clone(), payload.clone())
                .await
            {
                Ok(response) => return Ok(response),
                Err(error) => {
                    let disposition = match error {
                        ClientError::SessionLost => {
                            driver.record_session_failure(SessionFailure::Disconnected)
                        }
                        ClientError::TransportFailed => {
                            driver.record_session_failure(SessionFailure::Retryable)
                        }
                        ClientError::DeadlineExpired => {
                            driver.record_session_failure(SessionFailure::Deadline)
                        }
                        ClientError::Cancelled => {
                            driver.record_session_failure(SessionFailure::Cancelled)
                        }
                        ClientError::ContractViolation => {
                            driver.record_session_failure(SessionFailure::Protocol)
                        }
                        ClientError::Remote {
                            retry: RetryClass::Immediate,
                            ..
                        } => driver.record_session_failure(SessionFailure::Retryable),
                        other => AttemptDisposition::Fail(other),
                    };
                    match disposition {
                        AttemptDisposition::RetryNow => continue,
                        AttemptDisposition::RetryAfterMs(delay) => {
                            tokio::time::sleep(std::time::Duration::from_millis(u64::from(delay)))
                                .await;
                            continue;
                        }
                        AttemptDisposition::Fail(error) => return Err(error),
                    }
                }
            }
        }
    }
}

fn target_resource_ref(target: &TargetInput) -> Option<ResourceRef> {
    match target {
        TargetInput::Guest { guest, .. } => Some(ResourceRef::new(
            d2b_contracts::v3::ResourceTypeName::parse("Guest").ok()?,
            guest.clone(),
        )),
        TargetInput::Provider { provider, .. } => Some(ResourceRef::new(
            d2b_contracts::v3::ResourceTypeName::parse("Provider").ok()?,
            provider.clone(),
        )),
        TargetInput::Service { owner, .. } => match owner {
            crate::ServiceOwner::Guest { guest, .. } => Some(ResourceRef::new(
                d2b_contracts::v3::ResourceTypeName::parse("Guest").ok()?,
                guest.clone(),
            )),
            crate::ServiceOwner::Provider { provider, .. } => Some(ResourceRef::new(
                d2b_contracts::v3::ResourceTypeName::parse("Provider").ok()?,
                provider.clone(),
            )),
            _ => None,
        },
        _ => None,
    }
}

/// Local Zone session wrapper used by Process attachment clients.
pub struct LocalZoneSession<S> {
    zone: ZoneId,
    session: S,
}

impl<S> LocalZoneSession<S> {
    /// Bind a session to its authenticated Zone identity.
    pub const fn new(zone: ZoneId, session: S) -> Self {
        Self { zone, session }
    }

    /// Borrow the Zone identity.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
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

    #[test]
    fn peer_uid_mismatch_fails_before_calls() {
        let connector = ZoneSocketConnector::new(1000);
        assert!(
            connector
                .verify_peer(ZonePeerIdentity::from_observed_uid(1000))
                .is_ok()
        );
        assert_eq!(
            connector.verify_peer(ZonePeerIdentity::from_observed_uid(1001)),
            Err(ClientError::TransportFailed)
        );
        assert_eq!(ResourceVerb::UpdateSpec.as_str(), "resource-update-spec");
    }
}
