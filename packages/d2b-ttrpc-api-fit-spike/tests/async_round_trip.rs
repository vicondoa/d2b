use std::{
    env, fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use d2b_ttrpc_api_fit_spike::generated::{
    ttrpc_api_fit_spike::{ProbeRequest, ProbeResponse},
    ttrpc_api_fit_spike_ttrpc::{
        AsyncApiFitSpike, AsyncApiFitSpikeClient, create_async_api_fit_spike,
    },
};
use tokio::{
    net::{UnixListener, UnixStream},
    sync::watch,
    time::{sleep, timeout},
};
use ttrpc::r#async::TtrpcContext;

static SCRATCH_COUNTER: AtomicUsize = AtomicUsize::new(0);
const OPERATION_TIMEOUT: Duration = Duration::from_secs(2);

struct SocketFixture {
    dir: PathBuf,
    socket: PathBuf,
}

impl SocketFixture {
    fn new() -> Self {
        let socket_root = env::var_os("D2B_VALIDATION_SOCKET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(env::temp_dir);

        for _ in 0..100 {
            let nonce = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
            let dir = socket_root.join(format!(
                ".ttrpc-api-fit-spike.{}.{nonce}",
                std::process::id()
            ));
            match fs::create_dir(&dir) {
                Ok(()) => {
                    let socket = dir.join("rpc.sock");
                    return Self { dir, socket };
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create API-fit socket directory: {error}"),
            }
        }
        panic!("could not reserve an API-fit socket directory");
    }
}

impl Drop for SocketFixture {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.socket);
        let _ = fs::remove_dir(&self.dir);
    }
}

struct YieldingProbe {
    heartbeat: Arc<AtomicUsize>,
}

#[async_trait]
impl AsyncApiFitSpike for YieldingProbe {
    async fn probe(
        &self,
        _ctx: &TtrpcContext,
        request: ProbeRequest,
    ) -> ttrpc::Result<ProbeResponse> {
        let heartbeat_before = self.heartbeat.load(Ordering::SeqCst);
        let delay = if request.nonce == "force-timeout" {
            Duration::from_millis(200)
        } else {
            Duration::from_millis(40)
        };
        sleep(delay).await;

        let mut response = ProbeResponse::new();
        response.nonce = request.nonce;
        response.handler_awaited = self.heartbeat.load(Ordering::SeqCst) > heartbeat_before;
        Ok(response)
    }
}

#[tokio::test(flavor = "current_thread")]
async fn generated_async_unix_round_trip_yields_and_stops_boundedly() {
    let fixture = SocketFixture::new();
    let heartbeat = Arc::new(AtomicUsize::new(0));
    let (stop_heartbeat, mut heartbeat_stop) = watch::channel(false);
    let heartbeat_task = {
        let heartbeat = Arc::clone(&heartbeat);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    changed = heartbeat_stop.changed() => {
                        if changed.is_err() || *heartbeat_stop.borrow() {
                            break;
                        }
                    }
                    () = sleep(Duration::from_millis(1)) => {
                        heartbeat.fetch_add(1, Ordering::SeqCst);
                    }
                }
            }
        })
    };

    let listener = UnixListener::bind(&fixture.socket).expect("bind API-fit Unix socket");
    let service = Arc::new(YieldingProbe {
        heartbeat: Arc::clone(&heartbeat),
    });
    let mut server = ttrpc::r#async::Server::new()
        .add_listener(ttrpc::r#async::transport::Listener::from(listener))
        .register_service(create_async_api_fit_spike(service));
    timeout(OPERATION_TIMEOUT, server.start())
        .await
        .expect("server start remained bounded")
        .expect("start generated async server");

    // ttrpc 0.9.0 Client::connect reaches a blocking StdUnixStream connect.
    // W3 clients must enter ttrpc through this Tokio-established stream instead.
    let stream = timeout(OPERATION_TIMEOUT, UnixStream::connect(&fixture.socket))
        .await
        .expect("Tokio Unix connect remained bounded")
        .expect("connect API-fit Unix socket");
    let socket = ttrpc::r#async::transport::Socket::new(stream);
    let client = AsyncApiFitSpikeClient::new(ttrpc::r#async::Client::new(socket));

    let mut request = ProbeRequest::new();
    request.nonce = "nonce-1".to_owned();
    let response = timeout(
        OPERATION_TIMEOUT,
        client.probe(
            ttrpc::context::with_duration(Duration::from_secs(1)),
            &request,
        ),
    )
    .await
    .expect("generated async client call remained bounded")
    .expect("generated async client and server round trip");
    assert_eq!(response.nonce, request.nonce);
    assert!(
        response.handler_awaited,
        "heartbeat must progress while the generated handler awaits"
    );

    let mut timeout_request = ProbeRequest::new();
    timeout_request.nonce = "force-timeout".to_owned();
    let timed_out = timeout(
        OPERATION_TIMEOUT,
        client.probe(
            ttrpc::context::with_duration(Duration::from_millis(20)),
            &timeout_request,
        ),
    )
    .await
    .expect("RPC timeout remained bounded");
    assert!(
        matches!(
            timed_out,
            Err(ttrpc::Error::Others(ref message)) if message.contains("timeout")
        ),
        "generated client must report its RPC deadline"
    );

    drop(client);
    timeout(OPERATION_TIMEOUT, server.shutdown())
        .await
        .expect("server shutdown remained bounded")
        .expect("shutdown generated async server");
    stop_heartbeat.send(true).expect("stop heartbeat");
    timeout(OPERATION_TIMEOUT, heartbeat_task)
        .await
        .expect("heartbeat shutdown remained bounded")
        .expect("join heartbeat task");

    let socket_path = fixture.socket.clone();
    drop(fixture);
    assert!(!socket_path.exists(), "socket path must be cleaned up");
}
