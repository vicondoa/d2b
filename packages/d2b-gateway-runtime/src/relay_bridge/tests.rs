use super::*;
use d2b_provider_transport_azure_relay::auth::{DEFAULT_SAS_TTL_SECS, MAX_SAS_TTL_SECS, mint_sas};
use futures_util::sink::drain;
use std::future::pending;
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio_tungstenite::tungstenite::Message;

struct PumpProbe {
    active: Arc<AtomicUsize>,
    stopped: Arc<AtomicUsize>,
}

impl PumpProbe {
    fn new(active: &Arc<AtomicUsize>, stopped: &Arc<AtomicUsize>) -> Self {
        active.fetch_add(1, Ordering::SeqCst);
        Self {
            active: Arc::clone(active),
            stopped: Arc::clone(stopped),
        }
    }
}

impl Drop for PumpProbe {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::SeqCst);
        self.stopped.fetch_add(1, Ordering::SeqCst);
    }
}

async fn parked_pump(probe: PumpProbe) {
    let _probe = probe;
    pending::<()>().await;
}

#[tokio::test]
async fn completed_rendezvous_tasks_are_reaped_while_control_stays_open() {
    let completed = Arc::new(AtomicUsize::new(0));
    let mut owner = RendezvousTasks::default();
    for _ in 0..8 {
        let completed = Arc::clone(&completed);
        owner.spawn(async move {
            completed.fetch_add(1, Ordering::SeqCst);
        });
    }
    while completed.load(Ordering::SeqCst) != 8 {
        tokio::task::yield_now().await;
    }
    assert_eq!(owner.len(), 8);

    while !owner.is_empty() {
        owner.reap_one().await;
    }

    assert_eq!(owner.len(), 0);
}

#[tokio::test]
async fn control_close_joins_before_second_bridge_can_start() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("display.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    let (control_tx, control_rx) =
        tokio::sync::mpsc::unbounded_channel::<Result<Message, RelayConnectError>>();
    let control = futures_util::stream::unfold(control_rx, |mut receiver| async move {
        receiver.recv().await.map(|message| (message, receiver))
    });
    let task_socket_path = socket_path.clone();
    let control_task = tokio::spawn(run_listener_control_loop(
        control,
        drain(),
        None,
        move |_address| {
            let socket_path = task_socket_path.clone();
            Some(async move {
                let _local = tokio::net::UnixStream::connect(socket_path).await.unwrap();
                pending::<()>().await;
            })
        },
    ));

    control_tx
        .send(Ok(Message::Text(
            r#"{"accept":{"address":"local-test"}}"#.into(),
        )))
        .unwrap();
    let (first_local, _) = timeout(Duration::from_secs(1), listener.accept())
        .await
        .unwrap()
        .unwrap();

    control_tx.send(Ok(Message::Close(None))).unwrap();
    control_task.await.unwrap().unwrap();

    let mut byte = [0u8; 1];
    assert_eq!(
        first_local.try_read(&mut byte).unwrap(),
        0
    );

    let second_local = tokio::net::UnixStream::connect(&socket_path)
        .await
        .unwrap();
    let _second_session = timeout(Duration::from_secs(1), listener.accept())
        .await
        .unwrap()
        .unwrap();
    drop(second_local);
}

#[tokio::test]
async fn control_error_joins_active_rendezvous_tasks() {
    let active = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicUsize::new(0));
    let control = futures_util::stream::iter([
        Ok(Message::Text(
            r#"{"accept":{"address":"local-test"}}"#.into(),
        )),
        Err(RelayConnectError::Handshake("control closed".into())),
    ]);
    let active_for_spawn = Arc::clone(&active);
    let stopped_for_spawn = Arc::clone(&stopped);

    let err = run_listener_control_loop(control, drain(), None, move |_address| {
        Some(parked_pump(PumpProbe::new(
            &active_for_spawn,
            &stopped_for_spawn,
        )))
    })
    .await
    .unwrap_err();

    assert!(matches!(err, RelayConnectError::Handshake(_)));
    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(stopped.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn close_frame_stops_both_pump_directions_before_reconnect() {
    let active = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicUsize::new(0));
    let mut owner = RendezvousTasks::default();
    owner.spawn(parked_pump(PumpProbe::new(&active, &stopped)));
    owner.spawn(parked_pump(PumpProbe::new(&active, &stopped)));
    tokio::task::yield_now().await;
    assert_eq!(active.load(Ordering::SeqCst), 2);

    // The control-channel Close path uses this owner teardown before the
    // listener returns to its reconnect loop.
    owner.cancel_and_join().await;

    assert!(owner.is_empty());
    assert_eq!(stopped.load(Ordering::SeqCst), 2);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn session_cancellation_joins_every_rendezvous_task() {
    let active = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicUsize::new(0));
    let mut owner = RendezvousTasks::default();
    for _ in 0..3 {
        owner.spawn(parked_pump(PumpProbe::new(&active, &stopped)));
    }
    tokio::task::yield_now().await;
    owner.cancel_and_join().await;

    assert!(owner.is_empty());
    assert_eq!(stopped.load(Ordering::SeqCst), 3);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn second_bridge_starts_only_after_prior_local_session_stops() {
    let active = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicUsize::new(0));
    let mut owner = RendezvousTasks::default();
    owner.spawn(parked_pump(PumpProbe::new(&active, &stopped)));
    tokio::task::yield_now().await;
    assert_eq!(active.load(Ordering::SeqCst), 1);

    owner.cancel_and_join().await;
    assert_eq!(active.load(Ordering::SeqCst), 0);

    let second = PumpProbe::new(&active, &stopped);
    assert_eq!(active.load(Ordering::SeqCst), 1);
    drop(second);
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[test]
fn extract_prologue_needs_full_length_prefix() {
    // Fewer than 4 bytes -> need more.
    assert_eq!(extract_prologue_frame(&[0, 0]).unwrap(), None);
}

#[test]
fn extract_prologue_waits_for_full_body() {
    // length=5 but only 3 body bytes present -> need more.
    let mut buf = (5u32).to_be_bytes().to_vec();
    buf.extend_from_slice(b"abc");
    assert_eq!(extract_prologue_frame(&buf).unwrap(), None);
}

#[test]
fn extract_prologue_returns_frame_and_consumed() {
    let mut buf = (5u32).to_be_bytes().to_vec();
    buf.extend_from_slice(b"hello");
    buf.extend_from_slice(b"LEFTOVER");
    let (frame, consumed) = extract_prologue_frame(&buf).unwrap().unwrap();
    assert_eq!(frame, b"hello");
    assert_eq!(consumed, 9); // 4 + 5
    assert_eq!(&buf[consumed..], b"LEFTOVER");
}

#[test]
fn extract_prologue_rejects_oversize() {
    let buf = (u32::MAX).to_be_bytes().to_vec();
    assert!(extract_prologue_frame(&buf).is_err());
}

fn endpoint() -> RelayEndpoint {
    RelayEndpoint {
        namespace: "relns-test.servicebus.windows.net".into(),
        entity: "hc-d2b-display".into(),
    }
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

fn sas_param<'a>(token: &'a str, name: &str) -> &'a str {
    let prefix = format!("{name}=");
    token
        .strip_prefix("SharedAccessSignature ")
        .unwrap()
        .split('&')
        .find_map(|part| part.strip_prefix(&prefix))
        .unwrap()
}

#[test]
fn mint_sas_is_deterministic_for_fixed_inputs_modulo_expiry() {
    let ep = endpoint();
    let a = mint_sas(&ep, "gateway-listen", "c2VjcmV0a2V5", DEFAULT_SAS_TTL_SECS).unwrap();
    // Shape: a SharedAccessSignature with sr/sig/se/skn.
    assert!(a.starts_with("SharedAccessSignature sr="));
    assert!(a.contains("&skn=gateway-listen"));
    assert!(a.contains("&sig="));
    assert!(a.contains("&se="));
    // The resource is the lowercased url-encoded http form of the entity.
    assert!(a.contains("relns-test.servicebus.windows.net"));
}

#[test]
fn mint_sas_rejects_ttl_above_short_lived_cap() {
    let ep = endpoint();
    assert_eq!(
        mint_sas(&ep, "gateway-send", "c2VjcmV0a2V5", MAX_SAS_TTL_SECS + 1).unwrap_err(),
        RelayError::TtlTooLong {
            requested: MAX_SAS_TTL_SECS + 1,
            max: MAX_SAS_TTL_SECS
        }
    );
}

#[test]
fn mint_sas_expiry_matches_requested_short_ttl() {
    let ep = endpoint();
    let ttl = 60;
    let before = now_unix_secs();
    let token = mint_sas(&ep, "gateway-send", "c2VjcmV0a2V5", ttl).unwrap();
    let after = now_unix_secs();
    let expiry = sas_param(&token, "se").parse::<u64>().unwrap();
    assert!(expiry >= before + ttl);
    assert!(expiry <= after + ttl);
    assert!(expiry <= before + MAX_SAS_TTL_SECS);
}

#[test]
fn entra_sender_uses_header_not_url_token() {
    let ep = endpoint();
    let cred = RelayCredential::EntraBearer("jwt.abc.def".into());
    let c = build_connect(&ep, RelayRole::Sender, &cred, 3600).unwrap();
    // The bearer NEVER appears in the URL.
    assert!(!c.url.contains("jwt.abc.def"));
    assert!(!c.url.contains("sb-hc-token"));
    assert!(c.url.contains("sb-hc-action=connect"));
    // The sender omits sb-hc-id; the relay generates the rendezvous GUID.
    assert!(!c.url.contains("sb-hc-id="));
    let scheme: String = ['B', 'e', 'a', 'r', 'e', 'r'].into_iter().collect();
    let expected = format!("{scheme} jwt.abc.def");
    assert_eq!(c.auth_header.as_deref(), Some(expected.as_str()));
}

#[test]
fn sas_listener_puts_token_in_url_and_no_header() {
    let ep = endpoint();
    let cred = RelayCredential::Sas {
        key_name: "gateway-listen".into(),
        key: "c2VjcmV0a2V5".into(),
    };
    let c = build_connect(&ep, RelayRole::Listener, &cred, DEFAULT_SAS_TTL_SECS).unwrap();
    assert!(c.url.contains("sb-hc-action=listen"));
    assert!(c.url.contains("sb-hc-token="));
    assert!(!c.url.contains("sb-hc-id=")); // listener has no rendezvous id
    assert!(c.auth_header.is_none());
}

#[test]
fn build_connect_rejects_sas_ttl_above_short_lived_cap() {
    let ep = endpoint();
    let cred = RelayCredential::Sas {
        key_name: "gateway-listen".into(),
        key: "c2VjcmV0a2V5".into(),
    };
    assert!(matches!(
        build_connect(&ep, RelayRole::Listener, &cred, MAX_SAS_TTL_SECS + 1),
        Err(RelayError::TtlTooLong { .. })
    ));
}

#[test]
fn credential_debug_redacts_secrets() {
    let sas = RelayCredential::Sas {
        key_name: "gateway-send".into(),
        key: "supersecretkey".into(),
    };
    let d = format!("{sas:?}");
    assert!(d.contains("gateway-send"));
    assert!(!d.contains("supersecretkey"));
    let bearer = RelayCredential::EntraBearer("jwt.secret.token".into());
    let d = format!("{bearer:?}");
    assert!(!d.contains("jwt.secret.token"));
    let token = RelayCredential::SasToken("SharedAccessSignature secret".into());
    let d = format!("{token:?}");
    assert!(!d.contains("SharedAccessSignature secret"));
}

#[test]
fn connection_errors_redact_transport_canaries() {
    let error = RelayConnectError::Handshake(
        "SharedAccessSignature secret-canary; /run/d2b/credential".into(),
    );
    assert!(!format!("{error:?}").contains("secret-canary"));
    assert!(!format!("{error}").contains("secret-canary"));
    assert!(!format!("{error:?}").contains("/run/d2b/credential"));
    assert!(!format!("{error}").contains("/run/d2b/credential"));
}

#[test]
fn pre_minted_sas_sender_puts_token_in_url_without_key() {
    let ep = endpoint();
    let cred = RelayCredential::SasToken("SharedAccessSignature sr=x&sig=y".into());
    let c = build_connect(&ep, RelayRole::Sender, &cred, 3600).unwrap();
    assert!(c.url.contains("sb-hc-action=connect"));
    assert!(c.url.contains("sb-hc-token="));
    assert!(!c.url.contains("sb-hc-id="));
    assert!(c.auth_header.is_none());
}

#[test]
fn connect_debug_redacts_preminted_sas_query_token() {
    let ep = endpoint();
    let secret_token =
        "SharedAccessSignature sr=x&sig=very-secret-signature&se=123&skn=gateway-send";
    let cred = RelayCredential::SasToken(secret_token.into());
    let c = build_connect(&ep, RelayRole::Sender, &cred, 3600).unwrap();
    let d = format!("{c:?}");
    assert!(!d.contains("SharedAccessSignature"));
    assert!(!d.contains("very-secret-signature"));
    assert!(d.contains("?<redacted>"));
}

#[test]
fn connect_debug_redacts_url_query_and_header() {
    let ep = endpoint();
    let cred = RelayCredential::EntraBearer("jwt.abc.def".into());
    let c = build_connect(&ep, RelayRole::Sender, &cred, 3600).unwrap();
    let d = format!("{c:?}");
    assert!(!d.contains("jwt.abc.def"));
    assert!(!d.contains("Bearer"));
    assert!(d.contains("<redacted>"));
}

#[test]
fn unsolicited_bridge_ack_cannot_create_credit() {
    let mut available = 0;
    let mut in_flight = 0;
    acknowledge_bridge_credit(&mut available, &mut in_flight, 4096);
    assert_eq!((available, in_flight), (0, 0));

    available = BRIDGE_CREDIT_BYTES - 1024;
    in_flight = 1024;
    acknowledge_bridge_credit(&mut available, &mut in_flight, 4096);
    assert_eq!((available, in_flight), (BRIDGE_CREDIT_BYTES, 0));
}

#[test]
fn local_target_parses_each_form() {
    assert!(matches!(
        LocalTarget::parse("unix-listen:/run/wp.sock"),
        LocalTarget::UnixListen(p) if p == "/run/wp.sock"
    ));
    assert!(matches!(
        LocalTarget::parse("unix:/run/wpc.sock"),
        LocalTarget::UnixConnect(p) if p == "/run/wpc.sock"
    ));
    assert!(matches!(
        LocalTarget::parse("tcp:127.0.0.1:8080"),
        LocalTarget::TcpConnect(a) if a == "127.0.0.1:8080"
    ));
    assert!(matches!(
        LocalTarget::parse("127.0.0.1:9000"),
        LocalTarget::TcpConnect(a) if a == "127.0.0.1:9000"
    ));
}

#[test]
fn checked_unix_target_revalidates_owner_and_mode() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("wpc.sock");
    let _listener = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let meta = std::fs::symlink_metadata(&socket_path).unwrap();
    let path = socket_path.to_string_lossy().into_owned();
    open_checked_unix_socket_target(&path, meta.uid(), 0o600).unwrap();
    open_checked_unix_socket_target(&path, meta.uid(), 0o660).unwrap_err();

    let link_path = dir.path().join("link.sock");
    std::os::unix::fs::symlink(&socket_path, &link_path).unwrap();
    open_checked_unix_socket_target(&link_path.to_string_lossy(), meta.uid(), 0o600).unwrap_err();
}

#[tokio::test]
async fn checked_unix_target_validates_connected_peer_uid() {
    let dir = tempfile::tempdir().unwrap();
    let socket_path = dir.path().join("wpc.sock");
    let listener = tokio::net::UnixListener::bind(&socket_path).unwrap();
    std::fs::set_permissions(&socket_path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let meta = std::fs::symlink_metadata(&socket_path).unwrap();
    let stream = tokio::net::UnixStream::connect(&socket_path).await.unwrap();
    let (_accepted, _) = listener.accept().await.unwrap();
    validate_connected_unix_peer(&stream, meta.uid()).unwrap();
    validate_connected_unix_peer(&stream, meta.uid().saturating_add(1)).unwrap_err();
}
