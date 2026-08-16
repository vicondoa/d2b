use async_trait::async_trait;
use d2b_contracts::v3::{ResourceRef, ZoneId};
use d2b_provider_transport_vsock::{
    GuestControlKey, GuestIdentity, NativeGuestRelay, PeerCid, RelayBinding, RelayEffectError,
    RelayEffectPort, RelayObservation, RelayPhase, SessionAuthority, SessionProof,
};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct FakeRelayPort {
    calls: Arc<Mutex<Vec<&'static str>>>,
    fail_release: Arc<Mutex<bool>>,
    observed: Arc<Mutex<Option<RelayObservation<u64, u64>>>>,
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
        Ok(1)
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
        Ok(3)
    }

    async fn close_relay(&self, _: &Self::RelayProcess) -> Result<(), RelayEffectError> {
        self.calls.lock().unwrap().push("close-relay");
        Ok(())
    }

    async fn close_listener(&self, _: &Self::Listener) -> Result<(), RelayEffectError> {
        self.calls.lock().unwrap().push("close-listener");
        Ok(())
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
            .authenticate(SessionProof::sign(&key, &guest, [1; 32], 1))
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
        relay.adopt().await.unwrap();
        assert_eq!(relay.phase(), RelayPhase::Ready);

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
            relay.adopt().await.unwrap_err(),
            RelayEffectError::RestartMismatch
        );
        assert_eq!(relay.phase(), RelayPhase::Degraded);
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
            .authenticate(SessionProof::sign(&key, &guest, [3; 32], 1))
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
