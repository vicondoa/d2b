//! The Zone resource client facade.

use std::sync::Arc;

use crate::{
    CallDriver, CallOptions, ClientError, MethodProfile, ResolvedTarget, SystemClock, TargetInput,
    TargetResolver, TransportSelection, WallClock, ZoneServiceKind,
};

/// The Zone-addressed resource client.
///
/// The client owns exactly two responsibilities: resolving a Zone-addressed
/// target to an exact route, and minting the per-call driver that enforces the
/// deadline, retry, and cancellation policy for one call. It establishes no
/// session, holds no descriptor, and mints no authority; the session driver in
/// `d2b-bus` carries the request over the route this client resolved.
#[derive(Debug)]
pub struct ResourceClient<R, W = SystemClock> {
    resolver: R,
    clock: Arc<W>,
}

impl<R> ResourceClient<R, SystemClock> {
    /// Build a client over a resolver and the system clock.
    pub fn new(resolver: R) -> Self {
        Self {
            resolver,
            clock: Arc::new(SystemClock),
        }
    }
}

impl<R, W> ResourceClient<R, W> {
    /// Build a client over a resolver and an explicit clock.
    pub fn with_clock(resolver: R, clock: W) -> Self {
        Self {
            resolver,
            clock: Arc::new(clock),
        }
    }

    /// Borrow the resolver.
    pub const fn resolver(&self) -> &R {
        &self.resolver
    }
}

impl<R, W> ResourceClient<R, W>
where
    R: TargetResolver,
    W: WallClock,
{
    /// Resolve one Zone-addressed target to an exact route.
    pub fn resolve(
        &self,
        target: &TargetInput,
        service: ZoneServiceKind,
        selection: TransportSelection,
    ) -> Result<ResolvedTarget, ClientError> {
        self.resolver.resolve(target, service, selection)
    }

    /// Admit one call against an already resolved route.
    pub fn prepare_call(
        &self,
        target: &ResolvedTarget,
        profile: MethodProfile,
        options: CallOptions,
        has_attachments: bool,
    ) -> Result<CallDriver<W>, ClientError> {
        CallDriver::new(
            target,
            profile,
            options,
            has_attachments,
            Arc::clone(&self.clock),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AttemptDisposition, CancellationToken, MetadataInput, REQUEST_ID_BYTES, RetryPolicy,
        RouteRecord, RouteTable, ServiceOwner, SessionFailure, TransportKind,
        target::fixtures::zone,
    };

    const ISSUED: u64 = 1_000;

    #[derive(Debug)]
    struct FixedClock(u64);

    impl WallClock for FixedClock {
        fn now_unix_ms(&self) -> u64 {
            self.0
        }
    }

    fn options(attempts: u8) -> CallOptions {
        CallOptions {
            metadata: MetadataInput::new([1; REQUEST_ID_BYTES], ISSUED, ISSUED + 60_000).unwrap(),
            retry: RetryPolicy::new(attempts).unwrap(),
        }
    }

    /// K0 is the local root; K1 is a child Zone reached over K1's uplink.
    fn k0_client() -> ResourceClient<RouteTable, FixedClock> {
        let k0 = zone(&["k0"]);
        let k1 = zone(&["k1", "k0"]);
        ResourceClient::with_clock(
            RouteTable::new(vec![
                RouteRecord::new(ServiceOwner::ZoneLocal(k0), TransportKind::LocalUnix),
                RouteRecord::new(ServiceOwner::Zone(k1), TransportKind::ZoneLink),
            ]),
            FixedClock(ISSUED),
        )
    }

    #[test]
    fn a_k0_caller_reaches_k1_over_the_uplink_and_k0_locally() {
        let client = k0_client();
        let profile = MethodProfile::new(ZoneServiceKind::Resource, false, false, 30_000).unwrap();

        let local = client
            .resolve(
                &TargetInput::ZoneLocal(zone(&["k0"])),
                ZoneServiceKind::Resource,
                TransportSelection::exact(TransportKind::LocalUnix),
            )
            .expect("local Zone route");
        assert_eq!(local.transport(), TransportKind::LocalUnix);

        let cross = client
            .resolve(
                &TargetInput::ZoneService(zone(&["k1", "k0"]), ZoneServiceKind::Resource),
                ZoneServiceKind::Resource,
                TransportSelection::exact(TransportKind::ZoneLink),
            )
            .expect("child Zone route");
        assert_eq!(cross.transport(), TransportKind::ZoneLink);
        assert_eq!(cross.owner().zone(), &zone(&["k1", "k0"]));

        // Both routes drive the identical call policy.
        for target in [&local, &cross] {
            let mut driver = client
                .prepare_call(target, profile, options(2), false)
                .expect("call admitted");
            let token = CancellationToken::default();
            assert_eq!(driver.begin_attempt(&token).unwrap().attempt(), 1);
            assert_eq!(
                driver.record_session_failure(SessionFailure::Disconnected),
                AttemptDisposition::RetryNow
            );
            assert_eq!(driver.begin_attempt(&token).unwrap().attempt(), 2);
            assert_eq!(
                driver.begin_attempt(&token).unwrap_err(),
                ClientError::RetryLimitExceeded
            );
        }
        assert_eq!(client.resolver().records().len(), 2);
    }

    #[test]
    fn a_zone_outside_the_table_is_refused_before_any_call_is_prepared() {
        let client = k0_client();
        assert_eq!(
            client
                .resolve(
                    &TargetInput::ZoneService(zone(&["k2", "k1", "k0"]), ZoneServiceKind::Resource),
                    ZoneServiceKind::Resource,
                    TransportSelection::exact(TransportKind::ZoneLink),
                )
                .unwrap_err(),
            ClientError::RouteUnavailable
        );
    }

    #[test]
    fn a_cancelled_cross_zone_call_is_refused_at_the_client_boundary() {
        let client = k0_client();
        let target = client
            .resolve(
                &TargetInput::ZoneService(zone(&["k1", "k0"]), ZoneServiceKind::Resource),
                ZoneServiceKind::Resource,
                TransportSelection::exact(TransportKind::ZoneLink),
            )
            .unwrap();
        let profile = MethodProfile::new(ZoneServiceKind::Resource, false, false, 30_000).unwrap();
        let mut driver = client
            .prepare_call(&target, profile, options(4), false)
            .unwrap();
        let token = CancellationToken::default();
        token.cancel();
        assert_eq!(
            driver.begin_attempt(&token).unwrap_err(),
            ClientError::Cancelled
        );
    }
}
