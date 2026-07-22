//! RAII registration for one authenticated inbound ttrpc call.
//!
//! A generated ttrpc service method body decodes the call's
//! [`RequestId`](d2b_contracts::v2_component_session::RequestId) from the
//! request's `common::RequestMetadata` and constructs an
//! [`InboundCallGuard`] before running any handler logic. The guard exposes
//! the engine's per-request [`Cancellation`] token to the handler and
//! guarantees the driver's bookkeeping for that request id is completed or
//! removed exactly once: on the handler's ordinary success path via
//! [`InboundCallGuard::complete`], or automatically — via [`Drop`] — on any
//! early `?`-propagated error, `panic!` unwind, or plain drop.
//!
//! `CancelRequest`/`CancelAck` remain the sole source of truth for
//! cooperative cancellation: [`InboundCallGuard::cancellation`] returns the
//! exact [`Cancellation`] token the engine flips when it acknowledges a
//! `CancelRequest` for this same request id and generation. This module
//! never invents a ttrpc-native "Cancel" pseudo-method and never
//! second-guesses a result the engine already produced.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use d2b_contracts::v2_component_session::{RequestId, SessionErrorCode};
use d2b_contracts::v2_services::common;

use crate::{Cancellation, ComponentSessionDriver, Result, SessionError};

/// RAII handle for one inbound call registered with a
/// [`ComponentSessionDriver`].
///
/// Construct with [`InboundCallGuard::register`] (or
/// [`InboundCallGuard::register_from_metadata`]) immediately after decoding
/// the request's [`RequestId`] and before running any handler logic. Call
/// [`InboundCallGuard::complete`] on the handler's normal return path.
/// Dropping the guard without calling `complete` — an early return via `?`,
/// a panic unwinding through the handler, or any other early exit — removes
/// the registration instead, via [`Drop`].
pub struct InboundCallGuard {
    driver: Arc<dyn ComponentSessionDriver>,
    request_id: RequestId,
    cancellation: Cancellation,
    finished: AtomicBool,
}

impl std::fmt::Debug for InboundCallGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InboundCallGuard")
            .field("request_id", &"<redacted>")
            .field("finished", &self.finished.load(Ordering::Relaxed))
            .finish()
    }
}

impl InboundCallGuard {
    /// Registers `request_id` with `driver` before any handler dispatch.
    ///
    /// Returns the guard together with the exact [`Cancellation`] token the
    /// handler must observe for the remainder of its execution.
    pub async fn register(
        driver: Arc<dyn ComponentSessionDriver>,
        request_id: RequestId,
    ) -> Result<Self> {
        let cancellation = driver.register_inbound_call(request_id.clone()).await?;
        Ok(Self {
            driver,
            request_id,
            cancellation,
            finished: AtomicBool::new(false),
        })
    }

    /// Decodes the common v2 [`RequestId`] out of `metadata.request_id` and
    /// registers it, in one step, before any handler dispatch.
    ///
    /// This is the sanctioned decode-then-register path for a generated
    /// ttrpc service method: the request id is bound to the driver's
    /// bookkeeping before the handler body observes the request at all.
    pub async fn register_from_metadata(
        driver: Arc<dyn ComponentSessionDriver>,
        metadata: &common::RequestMetadata,
    ) -> Result<Self> {
        let request_id = RequestId::new(metadata.request_id.clone())
            .map_err(|_| SessionError::new(SessionErrorCode::RecordMalformed))?;
        Self::register(driver, request_id).await
    }

    /// The exact request id this guard registered.
    pub fn request_id(&self) -> &RequestId {
        &self.request_id
    }

    /// The exact cancellation token the handler must observe. This is the
    /// SAME token the engine flips when it delivers and acknowledges a
    /// `CancelRequest` for this request id and generation; there is no
    /// separate ttrpc-native cancel signal for a handler to poll instead.
    pub fn cancellation(&self) -> &Cancellation {
        &self.cancellation
    }

    /// Marks the call as normally completed and removes its registration.
    ///
    /// Call this exactly once, on the handler's ordinary success path,
    /// after the response has been produced. Any other exit — error, panic,
    /// early return, or simply dropping the guard without calling this —
    /// removes the registration via [`Drop`] instead.
    pub async fn complete(self) -> Result<bool> {
        self.finished.store(true, Ordering::Release);
        self.driver
            .complete_inbound_call(self.request_id.clone())
            .await
    }
}

impl Drop for InboundCallGuard {
    fn drop(&mut self) {
        if self.finished.swap(true, Ordering::AcqRel) {
            return;
        }
        let driver = Arc::clone(&self.driver);
        let request_id = self.request_id.clone();
        tokio::spawn(async move {
            let _ = driver.remove_inbound_call(request_id).await;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{OwnedAttachment, RequestRegistry, SessionEvent, StreamEvent, StreamId};
    use async_trait::async_trait;
    use d2b_contracts::v2_component_session::{CloseReason, Remediation};
    use std::sync::{
        Mutex,
        atomic::{AtomicU32, AtomicUsize},
    };
    use std::time::Instant;

    #[derive(Default)]
    struct FakeDriverState {
        registered: Mutex<Vec<RequestId>>,
        completed: Mutex<Vec<RequestId>>,
        removed: Mutex<Vec<RequestId>>,
        register_calls: AtomicUsize,
    }

    struct FakeDriver {
        state: Arc<FakeDriverState>,
        generation: AtomicU32,
        registry: Mutex<RequestRegistry>,
    }

    impl FakeDriver {
        fn spawn() -> (Arc<dyn ComponentSessionDriver>, Arc<FakeDriverState>) {
            let state = Arc::new(FakeDriverState::default());
            let driver: Arc<dyn ComponentSessionDriver> = Arc::new(Self {
                state: state.clone(),
                generation: AtomicU32::new(1),
                registry: Mutex::new(RequestRegistry::new(1).unwrap()),
            });
            (driver, state)
        }
    }

    #[async_trait]
    impl ComponentSessionDriver for FakeDriver {
        fn generation(&self) -> u64 {
            u64::from(self.generation.load(Ordering::Relaxed))
        }

        async fn start_ttrpc(&self, _request_id: RequestId, _frame: Vec<u8>) -> Result<()> {
            Ok(())
        }

        async fn complete_ttrpc(&self, _request_id: RequestId) -> Result<bool> {
            Ok(true)
        }

        async fn cancel(&self, _generation: u64, _request_id: RequestId) -> Result<()> {
            Ok(())
        }

        async fn send_ttrpc(&self, _frame: Vec<u8>) -> Result<()> {
            Ok(())
        }

        async fn receive_ttrpc(&self) -> Result<Vec<u8>> {
            Ok(Vec::new())
        }

        async fn register_inbound_call(&self, request_id: RequestId) -> Result<Cancellation> {
            self.state.register_calls.fetch_add(1, Ordering::Relaxed);
            self.state
                .registered
                .lock()
                .unwrap()
                .push(request_id.clone());
            self.registry.lock().unwrap().register(request_id)
        }

        async fn complete_inbound_call(&self, request_id: RequestId) -> Result<bool> {
            self.state
                .completed
                .lock()
                .unwrap()
                .push(request_id.clone());
            Ok(self.registry.lock().unwrap().complete(&request_id))
        }

        async fn remove_inbound_call(&self, request_id: RequestId) -> Result<bool> {
            self.state.removed.lock().unwrap().push(request_id.clone());
            Ok(self.registry.lock().unwrap().remove(&request_id))
        }

        async fn send_attachments(&self, _attachments: Vec<OwnedAttachment>) -> Result<()> {
            Ok(())
        }

        async fn receive_attachments(&self) -> Result<Vec<OwnedAttachment>> {
            Ok(Vec::new())
        }

        async fn open_named_stream(
            &self,
            _stream: StreamId,
            _send_credit: u32,
            _receive_credit: u32,
        ) -> Result<()> {
            Ok(())
        }

        async fn send_named_stream(&self, _stream: StreamId, _bytes: Vec<u8>) -> Result<()> {
            Ok(())
        }

        async fn receive_named_stream(&self) -> Result<StreamEvent> {
            Err(SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn grant_named_stream_credit(&self, _stream: StreamId, _bytes: u32) -> Result<()> {
            Ok(())
        }

        async fn close_named_stream(&self, _stream: StreamId) -> Result<()> {
            Ok(())
        }

        async fn reset_named_stream(&self, _stream: StreamId) -> Result<()> {
            Ok(())
        }

        async fn drive_keepalive(&self, _now: Instant) -> Result<()> {
            Ok(())
        }

        async fn receive_control(&self) -> Result<SessionEvent> {
            Err(SessionError::new(SessionErrorCode::Cancelled))
        }

        async fn close(&self, _reason: CloseReason, _remediation: Remediation) -> Result<()> {
            Ok(())
        }
    }

    fn request_id(byte: u8) -> RequestId {
        RequestId::new(vec![byte; 16]).unwrap()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn complete_removes_registration_exactly_once_and_never_double_removes() {
        let (driver, state) = FakeDriver::spawn();
        let id = request_id(1);
        let guard = InboundCallGuard::register(driver, id.clone())
            .await
            .unwrap();
        assert_eq!(
            state.registered.lock().unwrap().as_slice(),
            std::slice::from_ref(&id)
        );
        assert!(guard.complete().await.unwrap());
        assert_eq!(state.completed.lock().unwrap().as_slice(), &[id]);
        assert!(state.removed.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_without_complete_removes_the_registration() {
        let (driver, state) = FakeDriver::spawn();
        let id = request_id(2);
        {
            let _guard = InboundCallGuard::register(driver, id.clone())
                .await
                .unwrap();
            // Early exit / error path: the guard is dropped without ever
            // calling `complete`.
        }
        // Drop spawns the removal as a background task; give it a chance to
        // run before asserting.
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        assert_eq!(state.removed.lock().unwrap().as_slice(), &[id]);
        assert!(state.completed.lock().unwrap().is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_after_an_error_return_still_removes_the_registration() {
        async fn fallible(driver: Arc<dyn ComponentSessionDriver>, id: RequestId) -> Result<()> {
            let _guard = InboundCallGuard::register(driver, id).await?;
            Err(SessionError::new(SessionErrorCode::Cancelled))?
        }

        let (driver, state) = FakeDriver::spawn();
        let id = request_id(3);
        assert!(fallible(driver, id.clone()).await.is_err());
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        assert_eq!(state.removed.lock().unwrap().as_slice(), &[id]);
    }

    #[test]
    fn dropping_during_a_panic_still_removes_the_registration() {
        let (driver, state) = FakeDriver::spawn();
        let id = request_id(4);
        let driver_for_panic = driver.clone();
        let id_for_panic = id.clone();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.block_on(async {
                let _guard = InboundCallGuard::register(driver_for_panic, id_for_panic)
                    .await
                    .unwrap();
                panic!("simulated handler panic after registration");
            })
        }));
        assert!(outcome.is_err());
        runtime.block_on(async {
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
        });
        assert_eq!(state.removed.lock().unwrap().as_slice(), &[id]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn register_from_metadata_decodes_and_binds_the_request_id_before_dispatch() {
        let (driver, state) = FakeDriver::spawn();
        let id = request_id(5);
        let mut metadata = common::RequestMetadata::new();
        metadata.request_id = id.as_bytes().to_vec();
        let guard = InboundCallGuard::register_from_metadata(driver, &metadata)
            .await
            .unwrap();
        assert_eq!(guard.request_id(), &id);
        assert_eq!(
            state.registered.lock().unwrap().as_slice(),
            std::slice::from_ref(&id)
        );
        assert!(guard.complete().await.unwrap());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn register_from_metadata_rejects_a_malformed_request_id() {
        let (driver, _state) = FakeDriver::spawn();
        let mut metadata = common::RequestMetadata::new();
        metadata.request_id = vec![1, 2, 3];
        assert_eq!(
            InboundCallGuard::register_from_metadata(driver, &metadata)
                .await
                .unwrap_err()
                .code(),
            SessionErrorCode::RecordMalformed
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cancellation_token_is_the_exact_token_returned_by_register() {
        let (driver, _state) = FakeDriver::spawn();
        let id = request_id(6);
        let guard = InboundCallGuard::register(driver, id).await.unwrap();
        assert!(!guard.cancellation().is_cancelled());
        guard.cancellation().cancel();
        assert!(guard.cancellation().is_cancelled());
    }

    // The `FakeDriver` tests above exercise `InboundCallGuard`'s RAII
    // bookkeeping in isolation. The test below instead drives it against a
    // REAL `SessionEngine` pair over a real (in-memory) transport, proving
    // the guard's `cancellation()` token is the exact same token the engine
    // flips on a genuine wire-level `CancelRequest`/`CancelAck` round trip —
    // there is no separate ttrpc-native "Cancel" pseudo-method involved.
    mod real_engine_cancellation {
        use super::*;
        use crate::{
            HandshakeCredentials, OwnedTransport, SessionEngine, TransportDescriptor,
            TransportError, TransportPacket,
        };
        use d2b_contracts::v2_component_session::{
            AttachmentPolicy, AttachmentPolicyKind, CancelResult, EndpointPolicy, EndpointPurpose,
            EndpointRole, HandshakeOffer, IdentityEvidenceRequirement, LimitProfile, Locality,
            NoiseProfile, PurposeClass, ServicePackage, TransportBinding, TransportClass,
        };
        use tokio::sync::mpsc;

        struct DuplexTransport {
            sender: mpsc::Sender<TransportPacket>,
            receiver: mpsc::Receiver<TransportPacket>,
        }

        #[async_trait]
        impl OwnedTransport for DuplexTransport {
            fn descriptor(&self) -> TransportDescriptor {
                TransportDescriptor {
                    class: TransportClass::UnixSeqpacket,
                    locality: Locality::HostLocal,
                    packet_atomic: true,
                    supports_attachments: true,
                }
            }

            async fn receive(
                &mut self,
                protected_limit: usize,
            ) -> std::result::Result<TransportPacket, TransportError> {
                let packet = self
                    .receiver
                    .recv()
                    .await
                    .ok_or(TransportError::Disconnected)?;
                if packet.as_bytes().len() > protected_limit {
                    return Err(TransportError::LimitExceeded);
                }
                Ok(packet)
            }

            async fn send(
                &mut self,
                packet: TransportPacket,
            ) -> std::result::Result<(), TransportError> {
                self.sender
                    .send(packet)
                    .await
                    .map_err(|_| TransportError::Disconnected)
            }

            async fn close(&mut self) -> std::result::Result<(), TransportError> {
                Ok(())
            }
        }

        fn duplex_pair() -> (DuplexTransport, DuplexTransport) {
            let (a_to_b, b_from_a) = mpsc::channel(64);
            let (b_to_a, a_from_b) = mpsc::channel(64);
            (
                DuplexTransport {
                    sender: a_to_b,
                    receiver: a_from_b,
                },
                DuplexTransport {
                    sender: b_to_a,
                    receiver: b_from_a,
                },
            )
        }

        fn broker_offer() -> HandshakeOffer {
            HandshakeOffer {
                purpose: EndpointPurpose::PrivilegedBroker,
                purpose_class: PurposeClass::Local,
                initiator_role: EndpointRole::LocalRootController,
                responder_role: EndpointRole::LocalRootBroker,
                service: ServicePackage::BrokerV2,
                schema_fingerprint: [0x11; 32],
                noise_profile: NoiseProfile::Nn25519ChaChaPolySha256,
                limits: LimitProfile::local_default(),
                transport_binding: TransportBinding {
                    transport: TransportClass::UnixSeqpacket,
                    locality: Locality::HostLocal,
                    channel_binding: [0x22; 32],
                    identity_evidence: IdentityEvidenceRequirement::DirectionalUnix,
                },
                reconnect_generation: 7,
                attachment_policy: AttachmentPolicy {
                    kind: AttachmentPolicyKind::PacketAtomic,
                    max_per_packet: 1,
                    max_per_request: 1,
                    max_per_operation: 1,
                    max_per_session: 1,
                    credentials_allowed: true,
                },
            }
        }

        fn broker_policy(offer: &HandshakeOffer) -> EndpointPolicy {
            EndpointPolicy {
                purpose: offer.purpose,
                purpose_class: offer.purpose_class,
                initiator_role: offer.initiator_role,
                responder_role: offer.responder_role,
                service: offer.service,
                schema_fingerprint: offer.schema_fingerprint,
                noise_profile: offer.noise_profile,
                limits: offer.limits,
                transport_binding: offer.transport_binding,
                reconnect_generation: offer.reconnect_generation,
                attachment_policy: offer.attachment_policy,
            }
        }

        async fn established_driver_pair() -> (
            Arc<dyn ComponentSessionDriver>,
            Arc<dyn ComponentSessionDriver>,
        ) {
            let (initiator_transport, responder_transport) = duplex_pair();
            let offer = broker_offer();
            let initiator_policy = broker_policy(&offer);
            let responder_policy = broker_policy(&offer);
            let now = Instant::now();
            let (initiator, responder) = tokio::join!(
                SessionEngine::establish_initiator(
                    initiator_transport,
                    initiator_policy,
                    HandshakeCredentials::Nn,
                    now,
                ),
                SessionEngine::establish_responder(
                    responder_transport,
                    responder_policy,
                    HandshakeCredentials::Nn,
                    now,
                )
            );
            (
                Arc::new(initiator.unwrap().into_driver()),
                Arc::new(responder.unwrap().into_driver()),
            )
        }

        #[tokio::test]
        async fn guard_cancellation_token_is_flipped_by_a_real_cancel_request_and_ack_round_trip() {
            let (initiator, responder) = established_driver_pair().await;
            let id = request_id(9);

            initiator
                .start_ttrpc(id.clone(), b"real-engine-cancel-me".to_vec())
                .await
                .unwrap();
            assert_eq!(
                responder.receive_ttrpc().await.unwrap(),
                b"real-engine-cancel-me"
            );

            let guard = InboundCallGuard::register(Arc::clone(&responder), id.clone())
                .await
                .unwrap();
            assert!(!guard.cancellation().is_cancelled());

            initiator
                .cancel(initiator.generation(), id.clone())
                .await
                .unwrap();

            assert!(matches!(
                responder.receive_control().await.unwrap(),
                SessionEvent::CancelRequest(ack)
                    if ack.result == CancelResult::CancelledBeforeDispatch
            ));
            // This is the exact same token `InboundCallGuard::register` handed
            // back before dispatch — the engine's real CancelRequest handling
            // flipped it, not a second, independently-invented signal.
            assert!(guard.cancellation().is_cancelled());
            assert!(matches!(
                initiator.receive_control().await.unwrap(),
                SessionEvent::CancelAck(ack) if ack.result == CancelResult::CancelledBeforeDispatch
            ));

            // Dropping the guard (rather than calling `complete`) still
            // removes the bookkeeping exactly once, via the same RAII path
            // exercised by the `FakeDriver` tests above — this time against a
            // real driver-backed engine.
            drop(guard);
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
            assert!(!responder.remove_inbound_call(id).await.unwrap());
        }
    }
}
