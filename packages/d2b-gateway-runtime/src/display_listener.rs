//! `RelayDisplayListener` - the production [`DisplayListener`] adapter that arms
//! the host-side Azure Relay listener gating the display byte stream behind the
//! per-session handshake (ADR 0032, P0).
//!
//! `arm` spawns a task running
//! [`run_listener_verified_with_ready`](crate::relay_bridge::run_listener_verified_with_ready)
//! with a [`PrologueVerifier`] that verifies the session handshake under the
//! authorizing binding + secret. Readiness is signalled only after the
//! authenticated relay has attached to the local display endpoint, so
//! `await_handshake` resolves exactly when bytes can flow. `close` aborts the
//! task and drops the listener.
//!
//! The verifier-signal composition is a pure helper ([`notifying_verifier`]) so
//! it is unit-tested with a real handshake frame and no Azure round-trip.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use async_trait::async_trait;
use d2b_gateway::{
    DisplayListener, DisplaySessionContext, GatewayError, ListenerHandle, SessionBinding,
    SessionSecret,
};
use d2b_provider_transport_azure_relay::auth::{RelayCredential, RelayEndpoint};
use tokio::sync::Notify;
use tokio::time::{Duration, timeout};

use crate::{
    NowFn, make_prologue_verifier,
    relay_bridge::{LocalTarget, PrologueVerifier},
};

/// Wrap a [`PrologueVerifier`] so the first verified handshake flips `armed` and
/// wakes everyone waiting on `notify`. The wrapped verifier is otherwise
/// transparent (it returns exactly what `inner` returns), so a rejected frame
/// never signals and the relay still forwards zero bytes.
pub fn notifying_verifier(
    inner: PrologueVerifier,
    notify: Arc<Notify>,
    armed: Arc<AtomicBool>,
) -> PrologueVerifier {
    Arc::new(move |frame: &[u8]| {
        let ok = inner(frame);
        if ok {
            armed.store(true, Ordering::SeqCst);
            notify.notify_waiters();
        }
        ok
    })
}

struct ListenerState {
    cancel: tokio::sync::watch::Sender<bool>,
    thread: Option<std::thread::JoinHandle<()>>,
    handshook: Arc<Notify>,
    armed: Arc<AtomicBool>,
    handshake_timeout: Duration,
}

impl Drop for ListenerState {
    fn drop(&mut self) {
        let _ = self.cancel.send(true);
    }
}

/// The production host-side display listener.
pub struct RelayDisplayListener {
    endpoint: RelayEndpoint,
    credential: RelayCredential,
    /// The operator-side display endpoint the verified bytes are bridged to
    /// (e.g. `unix:/run/user/1000/wpc.sock`, the host `waypipe client` socket).
    target: LocalTarget,
    ttl_secs: u64,
    ca_pem: Option<Vec<u8>>,
    now: NowFn,
    state: Mutex<HashMap<String, ListenerState>>,
    counter: AtomicU64,
}

impl RelayDisplayListener {
    /// Build a listener bridging verified relay bytes to `target`.
    pub fn new(
        endpoint: RelayEndpoint,
        credential: RelayCredential,
        target: LocalTarget,
        ttl_secs: u64,
        ca_pem: Option<Vec<u8>>,
        now: NowFn,
    ) -> Self {
        Self {
            endpoint,
            credential,
            target,
            ttl_secs,
            ca_pem,
            now,
            state: Mutex::new(HashMap::new()),
            counter: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl DisplayListener for RelayDisplayListener {
    async fn arm(
        &self,
        ctx: &DisplaySessionContext,
        binding: &SessionBinding,
        secret: &SessionSecret,
    ) -> Result<ListenerHandle, GatewayError> {
        let inner = make_prologue_verifier(
            secret.clone(),
            binding.clone(),
            binding.generation,
            self.now.clone(),
        );
        let handshook = Arc::new(Notify::new());
        let armed = Arc::new(AtomicBool::new(false));
        let ready = {
            let handshook = handshook.clone();
            let armed = armed.clone();
            Arc::new(move || {
                armed.store(true, Ordering::SeqCst);
                handshook.notify_waiters();
            })
        };

        let endpoint = self.endpoint.clone();
        let credential = self.credential.clone();
        let target = self.target.clone();
        let ttl = self.ttl_secs;
        let ca = self.ca_pem.clone();
        let id = format!(
            "relay-listener:{}:{}",
            ctx.session_id.as_str(),
            self.counter.fetch_add(1, Ordering::SeqCst)
        );
        let thread_name = id.clone();
        let (cancel_tx, mut cancel_rx) = tokio::sync::watch::channel(false);
        let task_cancel = cancel_tx.clone();
        let thread = std::thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                let rt = match tokio::runtime::Builder::new_multi_thread()
                    .worker_threads(2)
                    .enable_all()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(err) => {
                        tracing::warn!(error = %err, "gateway relay listener runtime build failed");
                        return;
                    }
                };
                rt.block_on(async move {
                    // The relay periodically closes the listener control
                    // channel; re-arm until the session is closed. This runs
                    // on its own runtime thread so the listener survives the
                    // daemon's synchronous gatewayDisplay request runtime.
                    loop {
                        let result =
                            crate::relay_bridge::run_listener_verified_with_ready_and_cancel(
                                &endpoint,
                                &credential,
                                &target,
                                ttl,
                                ca.as_deref(),
                                inner.clone(),
                                ready.clone(),
                                Some(cancel_rx.clone()),
                            )
                            .await;
                        if *cancel_rx.borrow() {
                            break;
                        }
                        if let Err(err) = result {
                            tracing::warn!(error = %err, "gateway relay listener ended before close");
                        }
                        tokio::select! {
                            changed = cancel_rx.changed() => {
                                if changed.is_err() || *cancel_rx.borrow() {
                                    break;
                                }
                            }
                            _ = tokio::time::sleep(std::time::Duration::from_secs(1)) => {}
                        }
                    }
                });
            })
            .map_err(|_| GatewayError::Internal)?;
        let mut guard = self.state.lock().map_err(|_| GatewayError::Internal)?;
        let now = (self.now)();
        let handshake_timeout = Duration::from_secs(binding.not_after.saturating_sub(now));
        guard.insert(
            id.clone(),
            ListenerState {
                cancel: task_cancel,
                thread: Some(thread),
                handshook,
                armed,
                handshake_timeout,
            },
        );
        Ok(ListenerHandle(id))
    }

    async fn await_handshake(&self, handle: &ListenerHandle) -> Result<(), GatewayError> {
        // Snapshot the signal primitives without holding the lock across await.
        let (handshook, armed, handshake_timeout) = {
            let guard = self.state.lock().map_err(|_| GatewayError::Internal)?;
            let st = guard.get(&handle.0).ok_or(GatewayError::Internal)?;
            (st.handshook.clone(), st.armed.clone(), st.handshake_timeout)
        };
        if armed.load(Ordering::SeqCst) {
            return Ok(());
        }
        // Register for the notification, then re-check to close the race where
        // the handshake fires between the snapshot and the wait.
        let waiter = handshook.notified();
        if armed.load(Ordering::SeqCst) {
            return Ok(());
        }
        if timeout(handshake_timeout, waiter).await.is_err() {
            self.close(handle).await?;
            return Err(GatewayError::Timeout);
        }
        Ok(())
    }

    async fn close(&self, handle: &ListenerHandle) -> Result<(), GatewayError> {
        let st = {
            let mut guard = self.state.lock().map_err(|_| GatewayError::Internal)?;
            guard.remove(&handle.0)
        };
        if let Some(mut st) = st {
            let _ = st.cancel.send(true);
            if let Some(thread) = st.thread.take() {
                let _ = tokio::task::spawn_blocking(move || thread.join()).await;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_prologue;
    use d2b_gateway::{DisplaySessionId, SECRET_LEN};
    use d2b_realm_core::{OperationId, PrincipalId, RealmId, RealmPath, WorkloadId};

    fn binding(generation: u64, not_after: u64) -> (SessionBinding, SessionSecret) {
        let realm = RealmPath::new(vec![RealmId::parse("work").unwrap()]).unwrap();
        let b = SessionBinding::new(
            &realm,
            generation,
            &DisplaySessionId::new("s1"),
            0,
            &OperationId::parse("op-1").unwrap(),
            &PrincipalId::parse("alice").unwrap(),
            &WorkloadId::parse("demo").unwrap(),
            not_after,
        );
        (b, SessionSecret::from_bytes([9u8; SECRET_LEN]))
    }

    #[test]
    fn notifying_verifier_signals_only_on_a_verified_frame() {
        let (b, secret) = binding(3, 1000);
        let inner = make_prologue_verifier(secret.clone(), b.clone(), 3, Arc::new(|| 900));
        let notify = Arc::new(Notify::new());
        let armed = Arc::new(AtomicBool::new(false));
        let verify = notifying_verifier(inner, notify, armed.clone());

        // A garbage frame is rejected and never arms.
        assert!(!verify(b"garbage"));
        assert!(!armed.load(Ordering::SeqCst));

        // The real handshake (body only, as run_listener_verified hands it)
        // verifies and arms.
        let frame = agent_prologue(&secret, b);
        assert!(verify(&frame[4..]));
        assert!(armed.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn await_handshake_returns_once_armed() {
        let (b, secret) = binding(3, 1000);
        let inner = make_prologue_verifier(secret.clone(), b.clone(), 3, Arc::new(|| 900));
        let handshook = Arc::new(Notify::new());
        let armed = Arc::new(AtomicBool::new(false));
        let verify = notifying_verifier(inner, handshook.clone(), armed.clone());

        // Drive a verified handshake (as the relay listener would), then prove
        // await resolves.
        let frame = agent_prologue(&secret, b);
        assert!(verify(&frame[4..]));

        // Build a listener and inject the already-armed state to exercise the
        // fast path without an Azure round-trip.
        let listener = RelayDisplayListener::new(
            RelayEndpoint {
                namespace: "ns".into(),
                entity: "e".into(),
            },
            RelayCredential::EntraBearer("t".into()),
            LocalTarget::UnixConnect("/tmp/x".into()),
            60,
            None,
            Arc::new(|| 900),
        );
        let (cancel, _rx) = tokio::sync::watch::channel(false);
        let thread = std::thread::spawn(|| {});
        listener.state.lock().unwrap().insert(
            "h1".into(),
            ListenerState {
                cancel,
                thread: Some(thread),
                handshook,
                armed,
                handshake_timeout: Duration::from_secs(100),
            },
        );
        // armed is already true -> resolves immediately.
        listener
            .await_handshake(&ListenerHandle("h1".into()))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn close_is_idempotent_and_unknown_handles_fail_closed() {
        let listener = RelayDisplayListener::new(
            RelayEndpoint {
                namespace: "ns".into(),
                entity: "e".into(),
            },
            RelayCredential::EntraBearer("t".into()),
            LocalTarget::UnixConnect("/tmp/x".into()),
            60,
            None,
            Arc::new(|| 900),
        );
        // Closing an unknown handle is a no-op (idempotent teardown).
        listener
            .close(&ListenerHandle("nope".into()))
            .await
            .unwrap();
        // await on an unknown handle fails closed.
        let err = listener
            .await_handshake(&ListenerHandle("nope".into()))
            .await
            .unwrap_err();
        assert!(matches!(err, GatewayError::Internal));
    }

    #[tokio::test]
    async fn close_joins_listener_thread_before_next_bridge_can_arm() {
        let listener = RelayDisplayListener::new(
            RelayEndpoint {
                namespace: "ns".into(),
                entity: "e".into(),
            },
            RelayCredential::EntraBearer("t".into()),
            LocalTarget::UnixConnect("unused-socket".into()),
            60,
            None,
            Arc::new(|| 900),
        );
        let joined = Arc::new(AtomicBool::new(false));
        let thread_joined = Arc::clone(&joined);
        let thread = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            thread_joined.store(true, Ordering::SeqCst);
        });
        let (cancel, _rx) = tokio::sync::watch::channel(false);
        listener.state.lock().unwrap().insert(
            "h1".into(),
            ListenerState {
                cancel,
                thread: Some(thread),
                handshook: Arc::new(Notify::new()),
                armed: Arc::new(AtomicBool::new(false)),
                handshake_timeout: Duration::from_secs(100),
            },
        );

        listener.close(&ListenerHandle("h1".into())).await.unwrap();

        assert!(joined.load(Ordering::SeqCst));
    }
}
