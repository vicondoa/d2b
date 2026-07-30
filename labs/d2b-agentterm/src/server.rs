//! The agent-facing unix socket.
//!
//! Same-uid only, enforced with `SO_PEERCRED` at accept time rather than by
//! trusting filesystem permissions alone. This mirrors how d2b's unsafe-local
//! helper guards its private shell socket: the socket is created `0600` in a
//! runtime directory, and the peer's credentials are still checked, because
//! mode bits alone are a weak guarantee if the directory is ever wrong.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

use crate::keys::{keys_to_bytes, text_to_bytes};
use crate::protocol::{Applied, PROTOCOL_VERSION, Request, Response, SessionInfo, encode_line};
use crate::session::Session;
use crate::tty::TtySize;

/// Longest request line accepted, to bound memory from a hostile client.
const MAX_LINE_BYTES: u64 = 1 << 20;

/// Commands the socket sends to the pump.
#[derive(Debug)]
pub enum PumpCommand {
    /// Bytes to write to the child's input.
    Input(Vec<u8>),
    /// An advisory resize request.
    Resize(TtySize),
}

/// Shared handle to session state.
pub type SharedSession = Arc<Mutex<Session>>;

/// Default socket path for a given pid.
pub fn default_socket_path(pid: u32) -> PathBuf {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("d2b-agentterm-{pid}.sock"))
}

/// Bind the agent socket with restrictive permissions.
///
/// Removes a stale socket at the same path first; a leftover file from a
/// crashed session would otherwise make every start fail.
pub fn bind(path: &Path) -> anyhow::Result<UnixListener> {
    if path.exists() {
        // Only unlink something that is actually a socket, so a mistyped
        // --socket cannot delete a regular file.
        let meta = std::fs::symlink_metadata(path)?;
        if !is_socket(&meta) {
            anyhow::bail!("refusing to replace non-socket path {}", path.display());
        }
        std::fs::remove_file(path)?;
    }

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let listener = UnixListener::bind(path)?;
    set_socket_mode(path)?;
    Ok(listener)
}

fn is_socket(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::FileTypeExt;
    meta.file_type().is_socket()
}

fn set_socket_mode(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

/// Verify the peer runs as the same uid as this process.
fn check_peer(stream: &UnixStream) -> anyhow::Result<()> {
    let creds = nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)?;

    let ours = nix::unistd::getuid().as_raw();
    if creds.uid() != ours {
        anyhow::bail!(
            "peer uid {} does not match session uid {}",
            creds.uid(),
            ours
        );
    }

    Ok(())
}

/// Serve the agent socket until the listener is dropped.
pub async fn serve(
    listener: UnixListener,
    session: SharedSession,
    commands: mpsc::UnboundedSender<PumpCommand>,
) {
    loop {
        let stream = match listener.accept().await {
            Ok((stream, _addr)) => stream,
            Err(err) => {
                eprintln!("d2b-agentterm: accept failed: {err}");
                tokio::time::sleep(Duration::from_millis(50)).await;
                continue;
            }
        };

        if let Err(err) = check_peer(&stream) {
            eprintln!("d2b-agentterm: rejecting connection: {err}");
            continue;
        }

        let session = Arc::clone(&session);
        let commands = commands.clone();
        tokio::spawn(async move {
            if let Err(err) = handle(stream, session, commands).await {
                eprintln!("d2b-agentterm: connection error: {err}");
            }
        });
    }
}

async fn handle(
    stream: UnixStream,
    session: SharedSession,
    commands: mpsc::UnboundedSender<PumpCommand>,
) -> anyhow::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
    // Bound the reader before buffering it, so a client that never sends a
    // newline cannot make us allocate without limit.
    let mut lines = BufReader::new(read_half.take(MAX_LINE_BYTES)).lines();

    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }

        let response = match serde_json::from_str::<Request>(&line) {
            Ok(request) => dispatch(request, &session, &commands),
            Err(err) => Response::error(format!("malformed request: {err}")),
        };

        let encoded = encode_line(&response)
            .unwrap_or_else(|_| "{\"type\":\"error\",\"message\":\"encode failed\"}\n".into());
        write_half.write_all(encoded.as_bytes()).await?;
        write_half.flush().await?;
    }

    Ok(())
}

fn dispatch(
    request: Request,
    session: &SharedSession,
    commands: &mpsc::UnboundedSender<PumpCommand>,
) -> Response {
    // The lock is taken and released within each arm and never held across an
    // await, so a slow client cannot stall the pump.
    match request {
        Request::Info => match session.lock() {
            Ok(s) => Response::Info(SessionInfo {
                protocol_version: PROTOCOL_VERSION,
                child_pid: s.child_pid(),
                cols: s.size().cols,
                rows: s.size().rows,
                alt_screen: s.alt_screen(),
                bracketed_paste: s.bracketed_paste(),
                cursor_key_app_mode: s.cursor_key_app_mode(),
                uptime_ms: s.uptime().as_millis() as u64,
                exit_status: s.exit_status(),
            }),
            Err(_) => Response::error("session state poisoned"),
        },

        Request::Screen => match session.lock() {
            Ok(s) => Response::Screen(s.snapshot()),
            Err(_) => Response::error("session state poisoned"),
        },

        Request::Delta { window_ms } => match session.lock() {
            Ok(s) => Response::Delta(Box::new(s.delta(Duration::from_millis(window_ms)))),
            Err(_) => Response::error("session state poisoned"),
        },

        Request::Dump => match session.lock() {
            Ok(s) => Response::Dump { seq: s.dump() },
            Err(_) => Response::error("session state poisoned"),
        },

        Request::Keys { keys } => {
            let (app_mode, size) = match session.lock() {
                Ok(s) => (s.cursor_key_app_mode(), s.size()),
                Err(_) => return Response::error("session state poisoned"),
            };

            match keys_to_bytes(&keys, app_mode) {
                Ok(bytes) => send_input(bytes, false, size, commands),
                Err(err) => Response::error(err.to_string()),
            }
        }

        Request::Text { text } => {
            let (bracketed, size) = match session.lock() {
                Ok(s) => (s.bracketed_paste(), s.size()),
                Err(_) => return Response::error("session state poisoned"),
            };

            let bytes = text_to_bytes(&text, bracketed);
            send_input(bytes, bracketed, size, commands)
        }

        Request::Raw { data } => {
            let size = match session.lock() {
                Ok(s) => s.size(),
                Err(_) => return Response::error("session state poisoned"),
            };
            send_input(data.into_bytes(), false, size, commands)
        }

        Request::Resize { cols, rows } => {
            if cols == 0 || rows == 0 {
                return Response::error("resize dimensions must be non-zero");
            }

            if commands
                .send(PumpCommand::Resize(TtySize::new(cols, rows)))
                .is_err()
            {
                return Response::error("session has ended");
            }

            // The pump owns the authoritative size, and the human's terminal
            // wins any conflict, so this is reported as advisory rather than
            // echoing back what was asked for.
            Response::Applied(Applied {
                bytes: 0,
                bracketed: false,
                cols,
                rows,
                note: Some(
                    "resize is advisory; the attached terminal's size takes precedence \
                     and will override this on its next SIGWINCH"
                        .into(),
                ),
            })
        }
    }
}

fn send_input(
    bytes: Vec<u8>,
    bracketed: bool,
    size: TtySize,
    commands: &mpsc::UnboundedSender<PumpCommand>,
) -> Response {
    let count = bytes.len();
    if commands.send(PumpCommand::Input(bytes)).is_err() {
        return Response::error("session has ended");
    }

    Response::Applied(Applied {
        bytes: count,
        bracketed,
        cols: size.cols,
        rows: size.rows,
        note: None,
    })
}

#[cfg(test)]
mod tests {
    use super::{bind, default_socket_path};

    #[test]
    fn socket_path_includes_the_pid() {
        let path = default_socket_path(4321);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        assert_eq!(name, "d2b-agentterm-4321.sock");
    }

    #[test]
    fn bind_creates_a_socket_with_owner_only_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("agentterm-test-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("s.sock");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let Ok(rt) = rt else { return };

        rt.block_on(async {
            let listener = bind(&path);
            assert!(listener.is_ok(), "bind failed: {:?}", listener.err());

            let mode = std::fs::metadata(&path)
                .map(|m| m.permissions().mode() & 0o777)
                .unwrap_or(0);
            assert_eq!(mode, 0o600);
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bind_replaces_a_stale_socket() {
        let dir = std::env::temp_dir().join(format!("agentterm-stale-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("s.sock");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let Ok(rt) = rt else { return };

        rt.block_on(async {
            let first = bind(&path);
            assert!(first.is_ok());
            drop(first);
            // A crashed session leaves the socket file behind; rebinding must
            // succeed rather than failing with EADDRINUSE forever.
            let second = bind(&path);
            assert!(second.is_ok(), "rebind failed: {:?}", second.err());
        });

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bind_refuses_to_clobber_a_regular_file() {
        let dir = std::env::temp_dir().join(format!("agentterm-file-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("not-a-socket");
        let _ = std::fs::write(&path, b"important");

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build();
        let Ok(rt) = rt else { return };

        rt.block_on(async {
            assert!(bind(&path).is_err());
            // The file must still be there.
            assert_eq!(
                std::fs::read(&path).unwrap_or_default(),
                b"important".to_vec()
            );
        });

        let _ = std::fs::remove_dir_all(&dir);
    }
}
