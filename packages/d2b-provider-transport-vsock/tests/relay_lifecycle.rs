use async_trait::async_trait;
use d2b_contracts_resource::v3::{
    ResourceRef,
    ZoneId,
};
use d2b_provider_transport_vsock::{
    GuestControlKey, GuestIdentity, NativeGuestRelay, PeerCid, RelayBinding, RelayEffectError,
    RelayEffectPort, RelayObservation, RelayPhase, SessionAuthority, SessionProof,
};
use ring::rand::{SystemRandom, generate};
use std::sync::{Arc, Mutex};

fn nonce() -> [u8; 32] {
    generate::<[u8; 32]>(&SystemRandom::new()).unwrap().expose()
}

#[derive(Default)]
struct FakeRelayPort {
    calls: Arc<Mutex<Vec<&'static str>>>,
    fail_reserve: Arc<Mutex<bool>>,
    fail_spawn: Arc<Mutex<bool>>,
    fail_close_listener: Arc<Mutex<bool>>,
    fail_release: Arc<Mutex<bool>>,
    observed: Arc<Mutex<Option<RelayObservation<u64, u64>>>>,
    observe_error: Arc<Mutex<Option<RelayEffectError>>>,
}

#[async_trait]
impl RelayEffectPort for FakeRelayPort {
    type CidReservation = u64;
    type Listener = u64;
    type RelayProcess = u64;

    async fn reserve_cid(
        &self,
        _: &RelayBinding,
    ) -> Result<Self::CidReservation, RelayEffectError> {
        self.calls.lock().unwrap().push("reserve-cid");
        if *self.fail_reserve.lock().unwrap() {
            Err(RelayEffectError::CidAuthorityConflict)
        } else {
            Ok(1)
        }
    }

    async fn bind_listener(
        &self,
        _: &RelayBinding,
        _: &Self::CidReservation,
    ) -> Result<Self::Listener, RelayEffectError> {
        self.calls.lock().unwrap().push("bind-listener");
        Ok(2)
    }

    async fn spawn_relay(
        &self,
        _: &RelayBinding,
        _: &Self::Listener,
        _: &Self::CidReservation,
    ) -> Result<Self::RelayProcess, RelayEffectError> {
        self.calls.lock().unwrap().push("spawn-relay");
        if *self.fail_spawn.lock().unwrap() {
            Err(RelayEffectError::ProcessUnavailable)
        } else {
            Ok(3)
        }
    }

    async fn close_relay(&self, _: &Self::RelayProcess) -> Result<(), RelayEffectError> {
        self.calls.lock().unwrap().push("close-relay");
        Ok(())
    }

    async fn close_listener(&self, _: &Self::Listener) -> Result<(), RelayEffectError> {
        self.calls.lock().unwrap().push("close-listener");
        if *self.fail_close_listener.lock().unwrap() {
            Err(RelayEffectError::CloseUnconfirmed)
        } else {
            Ok(())
        }
    }

    async fn release_cid(&self, _: &Self::CidReservation) -> Result<(), RelayEffectError> {
        self.calls.lock().unwrap().push("release-cid");
        if *self.fail_release.lock().unwrap() {
            Err(RelayEffectError::CloseUnconfirmed)
        } else {
            Ok(())
        }
    }

    async fn observe(
        &self,
        _: &RelayBinding,
    ) -> Result<Option<RelayObservation<Self::Listener, Self::RelayProcess>>, RelayEffectError>
    {
        if let Some(error) = *self.observe_error.lock().unwrap() {
            return Err(error);
        }
        Ok(self.observed.lock().unwrap().clone())
    }
}

fn binding() -> RelayBinding {
    RelayBinding::new(
        GuestIdentity::new(
            ResourceRef::parse("Guest/guest-a").unwrap(),
            ZoneId::parse("work").unwrap(),
            PeerCid::from_core(42).unwrap(),
            "boot-a",
        )
        .unwrap(),
        [11; 16],
    )
}

#[test]
fn finalization_closes_relay_before_releasing_cid_authority() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let port = FakeRelayPort::default();
        let calls = Arc::clone(&port.calls);
        let key = GuestControlKey::from_core([7; 32]);
        let guest = binding().guest().clone();
        let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 1);
        let session = authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &guest, nonce(), 1),
            )
            .unwrap();
        let mut relay = NativeGuestRelay::new(port, binding());
        relay.start(&session).await.unwrap();
        relay.finalize().await.unwrap();
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                "reserve-cid",
                "bind-listener",
                "spawn-relay",
                "close-relay",
                "close-listener",
                "release-cid",
            ]
        );
        assert_eq!(relay.phase(), RelayPhase::Closed);
    });
}

#[test]
fn restart_adopts_only_the_matching_listener_and_process() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let binding = binding();
        let port = FakeRelayPort::default();
        *port.observed.lock().unwrap() = Some(RelayObservation {
            binding: binding.clone(),
            listener: 2,
            process: 3,
        });
        let mut relay = NativeGuestRelay::new(port, binding.clone());
        relay.adopt(1).await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Ready);
        relay.finalize().await.unwrap();

        let port = FakeRelayPort::default();
        *port.observed.lock().unwrap() = Some(RelayObservation {
            binding: RelayBinding::new(
                GuestIdentity::new(
                    ResourceRef::parse("Guest/other").unwrap(),
                    ZoneId::parse("work").unwrap(),
                    PeerCid::from_core(42).unwrap(),
                    "boot-a",
                )
                .unwrap(),
                [12; 16],
            ),
            listener: 2,
            process: 3,
        });
        let mut relay = NativeGuestRelay::new(port, binding);
        assert_eq!(
            relay.adopt(1).await.unwrap_err(),
            RelayEffectError::RestartMismatch
        );
        assert_eq!(relay.phase(), RelayPhase::Degraded);
    });
}

#[test]
fn reserve_failure_leaves_relay_retryable() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let port = FakeRelayPort::default();
        let fail_reserve = Arc::clone(&port.fail_reserve);
        *port.fail_reserve.lock().unwrap() = true;
        let key = GuestControlKey::from_core([7; 32]);
        let guest = binding().guest().clone();
        let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 1);
        let session = authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &guest, nonce(), 1),
            )
            .unwrap();
        let mut relay = NativeGuestRelay::new(port, binding());
        assert_eq!(
            relay.start(&session).await.unwrap_err(),
            RelayEffectError::CidAuthorityConflict
        );
        assert_eq!(relay.phase(), RelayPhase::Idle);

        relay.finalize().await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Closed);

        *fail_reserve.lock().unwrap() = false;
        relay.start(&session).await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Ready);
        relay.finalize().await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Closed);
    });
}

#[test]
fn failed_cid_release_retains_authority_for_retry() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let port = FakeRelayPort::default();
        let fail_release = Arc::clone(&port.fail_release);
        let key = GuestControlKey::from_core([7; 32]);
        let guest = binding().guest().clone();
        let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 1);
        let session = authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &guest, nonce(), 1),
            )
            .unwrap();
        let mut relay = NativeGuestRelay::new(port, binding());
        relay.start(&session).await.unwrap();
        *fail_release.lock().unwrap() = true;
        assert_eq!(
            relay.finalize().await.unwrap_err(),
            RelayEffectError::CloseUnconfirmed
        );
        assert_eq!(relay.phase(), RelayPhase::Finalizing);
        *fail_release.lock().unwrap() = false;
        relay.finalize().await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Closed);
    });
}

#[test]
fn listener_close_failure_keeps_cid_authority_for_retry() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let port = FakeRelayPort::default();
        *port.fail_spawn.lock().unwrap() = true;
        *port.fail_close_listener.lock().unwrap() = true;
        let calls = Arc::clone(&port.calls);
        let fail_close_listener = Arc::clone(&port.fail_close_listener);
        let key = GuestControlKey::from_core([7; 32]);
        let guest = binding().guest().clone();
        let mut authority = SessionAuthority::new(guest.clone(), key.clone(), 1);
        let session = authority
            .authenticate(
                PeerCid::from_core(42).unwrap(),
                SessionProof::sign(&key, &guest, nonce(), 1),
            )
            .unwrap();
        let mut relay = NativeGuestRelay::new(port, binding());

        assert_eq!(
            relay.start(&session).await.unwrap_err(),
            RelayEffectError::ProcessUnavailable
        );
        assert_eq!(relay.phase(), RelayPhase::Degraded);
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                "reserve-cid",
                "bind-listener",
                "spawn-relay",
                "close-listener",
            ]
        );

        *fail_close_listener.lock().unwrap() = false;
        relay.finalize().await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Closed);
        assert_eq!(
            *calls.lock().unwrap(),
            vec![
                "reserve-cid",
                "bind-listener",
                "spawn-relay",
                "close-listener",
                "close-listener",
                "release-cid",
            ]
        );
    });
}

#[test]
fn restart_observation_error_degrades_without_adoption() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .unwrap();
    runtime.block_on(async {
        let port = FakeRelayPort::default();
        *port.observe_error.lock().unwrap() = Some(RelayEffectError::Transient);
        let mut relay = NativeGuestRelay::new(port, binding());

        assert_eq!(
            relay.adopt(1).await.unwrap_err(),
            RelayEffectError::Transient
        );
        assert_eq!(relay.phase(), RelayPhase::Degraded);
    });
}

#[test]
fn relay_observation_debug_is_redacted() {
    let observation = RelayObservation {
        binding: binding(),
        listener: 1234_u64,
        process: 5678_u64,
    };
    let rendered = format!("{observation:?}");
    assert!(!rendered.contains("1234"));
    assert!(!rendered.contains("5678"));
    assert!(!rendered.contains("guest-a"));
}
