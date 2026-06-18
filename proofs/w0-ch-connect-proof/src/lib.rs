#![forbid(unsafe_code)]
#![cfg(test)]

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;
use ttrpc::asynchronous::transport::{Listener, Socket};
use ttrpc::asynchronous::{Client, MethodHandler, Server, Service, TtrpcContext};
use ttrpc::proto::{Code, Request, Response};
use ttrpc::{Result as TtrpcResult, get_status};

const SERVICE: &str = "proof.Echo";
const METHOD: &str = "Ping";
const PORT: u32 = 1024;
const LOCAL_PORT: u32 = 1 << 30;
const SMALL_LOCAL_PORT: u32 = 7;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_millis(250);

static NEXT_SOCKET: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Copy)]
enum FakeChMode {
    Ttrpc,
    TtrpcSmallLocalPort,
    MalformedOk,
    TooLargeLocalPort,
    OkPrefixJunk,
    CloseBeforeOk,
    NeverReply,
}

impl FakeChMode {
    fn is_ttrpc(self) -> bool {
        matches!(self, Self::Ttrpc | Self::TtrpcSmallLocalPort)
    }

    fn local_port(self) -> u32 {
        match self {
            Self::Ttrpc => LOCAL_PORT,
            Self::TtrpcSmallLocalPort => SMALL_LOCAL_PORT,
            _ => 0,
        }
    }
}

struct EchoHandler;

impl MethodHandler for EchoHandler {
    fn handler<'life0, 'async_trait>(
        &'life0 self,
        _ctx: TtrpcContext,
        req: Request,
    ) -> Pin<Box<dyn Future<Output = TtrpcResult<Response>> + Send + 'async_trait>>
    where
        Self: 'async_trait,
        'life0: 'async_trait,
    {
        Box::pin(async move {
            let mut res = Response::new();
            res.set_status(get_status(Code::OK, ""));
            res.payload = req.payload;
            Ok(res)
        })
    }
}

fn service_map() -> HashMap<String, Service> {
    let mut methods = HashMap::new();
    methods.insert(METHOD.to_string(), Box::new(EchoHandler) as _);

    let mut services = HashMap::new();
    services.insert(
        SERVICE.to_string(),
        Service {
            methods,
            streams: HashMap::new(),
        },
    );
    services
}

async fn connect_ch(base_socket: &Path, port: u32, timeout: Duration) -> io::Result<UnixStream> {
    tokio::time::timeout(timeout, async {
        let mut stream = UnixStream::connect(base_socket).await?;
        stream
            .write_all(format!("CONNECT {port}\n").as_bytes())
            .await?;
        stream.flush().await?;

        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            let n = stream.read(&mut byte).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "eof-before-ack",
                ));
            }
            line.push(byte[0]);
            if line.len() > 128 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "ack-too-long"));
            }
            if byte[0] == b'\n' {
                break;
            }
        }

        let line = String::from_utf8(line)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let Some(local_port) = line
            .strip_prefix("OK ")
            .and_then(|rest| rest.strip_suffix('\n'))
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "connect-refused",
            ));
        };
        if local_port.is_empty()
            || !local_port.bytes().all(|byte| byte.is_ascii_digit())
            || local_port.parse::<u32>().is_err()
        {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "malformed-ack"));
        }
        Ok(stream)
    })
    .await
    .map_err(|_| {
        io::Error::new(
            io::ErrorKind::TimedOut,
            "Cloud Hypervisor CONNECT timed out",
        )
    })?
}

async fn spawn_fake_ch(
    mode: FakeChMode,
) -> io::Result<(
    PathBuf,
    Option<Server>,
    tokio::task::JoinHandle<io::Result<()>>,
)> {
    let socket_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/proof-sockets");
    tokio::fs::create_dir_all(&socket_dir).await?;
    let socket_path = socket_dir.join(format!(
        "ch-{}-{}.sock",
        std::process::id(),
        NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = tokio::fs::remove_file(&socket_path).await;

    let listener = UnixListener::bind(&socket_path)?;
    let (tx, rx) = mpsc::channel::<io::Result<UnixStream>>(1);
    let server = if mode.is_ttrpc() {
        let listener = Listener::new(ReceiverStream::new(rx));
        let mut server = Server::new()
            .add_listener(listener)
            .register_service(service_map());
        server.start().await.map_err(io::Error::other)?;
        Some(server)
    } else {
        drop(rx);
        None
    };

    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            let n = stream.read(&mut byte).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "client closed before CONNECT line",
                ));
            }
            line.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }

        if line != format!("CONNECT {PORT}\n").as_bytes() {
            stream.write_all(b"ERR no such guest port\n").await?;
            stream.shutdown().await?;
            return Ok(());
        }

        match mode {
            FakeChMode::Ttrpc | FakeChMode::TtrpcSmallLocalPort => {
                stream
                    .write_all(format!("OK {}\n", mode.local_port()).as_bytes())
                    .await?;
                stream.flush().await?;
                tx.send(Ok(stream)).await.map_err(|_| {
                    io::Error::new(io::ErrorKind::BrokenPipe, "ttrpc listener was dropped")
                })?;
            }
            FakeChMode::MalformedOk => {
                stream.write_all(b"OK not-a-local-port\n").await?;
                stream.shutdown().await?;
            }
            FakeChMode::TooLargeLocalPort => {
                stream.write_all(b"OK 4294967296\n").await?;
                stream.shutdown().await?;
            }
            FakeChMode::OkPrefixJunk => {
                stream.write_all(b"OKAY\n").await?;
                stream.shutdown().await?;
            }
            FakeChMode::CloseBeforeOk => {
                stream.shutdown().await?;
            }
            FakeChMode::NeverReply => tokio::time::sleep(Duration::from_millis(500)).await,
        }
        Ok(())
    });

    Ok((socket_path, server, task))
}

async fn shutdown(_server: Option<&mut Server>, task: tokio::task::JoinHandle<io::Result<()>>) {
    let _ = task.await;
}

#[tokio::test]
async fn post_connect_unix_stream_wraps_ttrpc_client_and_server() {
    let (socket_path, mut server, task) = spawn_fake_ch(FakeChMode::Ttrpc).await.unwrap();
    let stream = connect_ch(&socket_path, PORT, HANDSHAKE_TIMEOUT)
        .await
        .unwrap();
    let client = Client::new(Socket::new(stream));

    let response = client
        .request(Request {
            service: SERVICE.to_string(),
            method: METHOD.to_string(),
            payload: b"post-connect ttrpc".to_vec(),
            timeout_nano: 1_000_000_000,
            ..Request::new()
        })
        .await
        .unwrap();

    assert_eq!(response.payload, b"post-connect ttrpc");
    shutdown(server.as_mut(), task).await;
    let _ = tokio::fs::remove_file(socket_path).await;
}

#[tokio::test]
async fn ok_local_port_is_not_used_as_a_buffer_limit() {
    let (socket_path, mut server, task) = spawn_fake_ch(FakeChMode::TtrpcSmallLocalPort)
        .await
        .unwrap();
    let stream = connect_ch(&socket_path, PORT, HANDSHAKE_TIMEOUT)
        .await
        .unwrap();
    let client = Client::new(Socket::new(stream));

    let response = client
        .request(Request {
            service: SERVICE.to_string(),
            method: METHOD.to_string(),
            payload: b"payload longer than the fake local port".to_vec(),
            timeout_nano: 1_000_000_000,
            ..Request::new()
        })
        .await
        .unwrap();

    assert_eq!(response.payload, b"payload longer than the fake local port");
    shutdown(server.as_mut(), task).await;
    let _ = tokio::fs::remove_file(socket_path).await;
}

#[tokio::test]
async fn post_ok_host_write_eof_still_allows_guest_output() {
    let socket_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/proof-sockets");
    tokio::fs::create_dir_all(&socket_dir).await.unwrap();
    let socket_path = socket_dir.join(format!(
        "h-{}-{}.sock",
        std::process::id(),
        NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = tokio::fs::remove_file(&socket_path).await;

    let listener = UnixListener::bind(&socket_path).unwrap();
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await?;
        let mut line = Vec::new();
        loop {
            let mut byte = [0u8; 1];
            let n = stream.read(&mut byte).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "client closed before CONNECT",
                ));
            }
            line.push(byte[0]);
            if byte[0] == b'\n' {
                break;
            }
        }
        assert_eq!(line, format!("CONNECT {PORT}\n").as_bytes());
        stream
            .write_all(format!("OK {LOCAL_PORT}\n").as_bytes())
            .await?;
        stream.flush().await?;

        let mut stdin = Vec::new();
        stream.read_to_end(&mut stdin).await?;
        assert_eq!(stdin, b"host stdin");
        stream.write_all(b"guest output after stdin eof").await?;
        stream.shutdown().await?;
        Ok::<_, io::Error>(())
    });

    let mut stream = connect_ch(&socket_path, PORT, HANDSHAKE_TIMEOUT)
        .await
        .unwrap();
    stream.write_all(b"host stdin").await.unwrap();
    stream.shutdown().await.unwrap();
    let mut output = Vec::new();
    stream.read_to_end(&mut output).await.unwrap();
    assert_eq!(output, b"guest output after stdin eof");
    server.await.unwrap().unwrap();
    let _ = tokio::fs::remove_file(socket_path).await;
}

#[tokio::test]
async fn wrong_port_is_refused_before_ttrpc_starts() {
    let (socket_path, mut server, task) = spawn_fake_ch(FakeChMode::Ttrpc).await.unwrap();
    let err = connect_ch(&socket_path, PORT + 1, HANDSHAKE_TIMEOUT)
        .await
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "connect-refused");
    shutdown(server.as_mut(), task).await;
    let _ = tokio::fs::remove_file(socket_path).await;
}

#[tokio::test]
async fn malformed_ok_is_rejected_before_ttrpc_starts() {
    let (socket_path, mut server, task) = spawn_fake_ch(FakeChMode::MalformedOk).await.unwrap();
    let err = connect_ch(&socket_path, PORT, HANDSHAKE_TIMEOUT)
        .await
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "malformed-ack");
    shutdown(server.as_mut(), task).await;
    let _ = tokio::fs::remove_file(socket_path).await;
}

#[tokio::test]
async fn too_large_local_port_is_rejected_before_ttrpc_starts() {
    let (socket_path, mut server, task) =
        spawn_fake_ch(FakeChMode::TooLargeLocalPort).await.unwrap();
    let err = connect_ch(&socket_path, PORT, HANDSHAKE_TIMEOUT)
        .await
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "malformed-ack");
    shutdown(server.as_mut(), task).await;
    let _ = tokio::fs::remove_file(socket_path).await;
}

#[tokio::test]
async fn ok_prefix_without_space_is_rejected_before_ttrpc_starts() {
    let (socket_path, mut server, task) = spawn_fake_ch(FakeChMode::OkPrefixJunk).await.unwrap();
    let err = connect_ch(&socket_path, PORT, HANDSHAKE_TIMEOUT)
        .await
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    assert_eq!(err.to_string(), "connect-refused");
    shutdown(server.as_mut(), task).await;
    let _ = tokio::fs::remove_file(socket_path).await;
}

#[tokio::test]
async fn eof_before_ok_is_reported() {
    let (socket_path, mut server, task) = spawn_fake_ch(FakeChMode::CloseBeforeOk).await.unwrap();
    let err = connect_ch(&socket_path, PORT, HANDSHAKE_TIMEOUT)
        .await
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
    shutdown(server.as_mut(), task).await;
    let _ = tokio::fs::remove_file(socket_path).await;
}

#[tokio::test]
async fn missing_ok_times_out() {
    let (socket_path, mut server, task) = spawn_fake_ch(FakeChMode::NeverReply).await.unwrap();
    let err = connect_ch(&socket_path, PORT, Duration::from_millis(50))
        .await
        .unwrap_err();

    assert_eq!(err.kind(), io::ErrorKind::TimedOut);
    shutdown(server.as_mut(), task).await;
    let _ = tokio::fs::remove_file(socket_path).await;
}
