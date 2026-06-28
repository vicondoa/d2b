//! Console session table and ring-buffer drainer (ADR 0041).
//!
//! Each VM that has an active console drainer gets one [`ConsoleSession`]
//! entry in the [`ConsoleSessionTable`].  Operator clients create lightweight
//! [`ConsoleClientHandle`] tokens that share the ring-buffer view; the
//! drainer runs continuously regardless of whether any client is attached.
//!
//! # Drainer model
//!
//! The daemon runs an async drainer task for every session in the table.
//! For Cloud Hypervisor VMs the task connects to the `--serial
//! socket=<path>` UNIX stream socket that CH creates, then reads console
//! output into the ring buffer.  For qemu-media VMs the task owns the host
//! end of the broker-created socketpair, which is passed in at session-
//! creation time.  Either way the drainer holds the exclusive fd/stream;
//! operator clients are secondary ring-buffer readers.
//!
//! Drainer restart safety: Cloud Hypervisor's serial socket is proven
//! safe to drop and reconnect (CH keeps running), so d2bd can reconnect
//! after restart.  For qemu-media, the broker re-provides the fd on daemon
//! restart via the normal SpawnRunner adoption path.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use d2b_contracts::public_wire::{ConsoleProviderKind, ConsoleReadOutputResult};
use d2b_core::console_ring::RingBuffer;

/// Maximum number of console sessions allowed across all VMs.
const MAX_SESSIONS: usize = 64;

/// Default ring-buffer capacity per VM (256 KiB).
const RING_CAPACITY: usize = 256 * 1024;

/// Drainer source: where console bytes come from.
#[derive(Debug)]
pub enum DrainerSource {
    /// Connect to a UNIX stream socket path created by the hypervisor
    /// (`--serial socket=<path>`). The drainer reconnects after drops.
    UnixSocket(String),
    /// Read from a pre-opened UNIX stream socket (used for testing or
    /// for cases where the socket is already connected).
    #[allow(dead_code)]
    Connected(tokio::net::UnixStream),
}

/// Shared ring buffer state for one VM's console stream.
#[derive(Debug)]
pub struct ConsoleRing {
    pub ring: RingBuffer,
    /// Notified whenever new bytes are pushed or EOF is set, so waiters
    /// can wake without polling.
    pub notify: Arc<tokio::sync::Notify>,
}

impl ConsoleRing {
    fn new() -> Self {
        Self {
            ring: RingBuffer::new(RING_CAPACITY),
            notify: Arc::new(tokio::sync::Notify::new()),
        }
    }
}

/// Per-VM console session.
#[derive(Debug)]
pub struct ConsoleSession {
    pub provider_kind: ConsoleProviderKind,
    pub ring: Arc<Mutex<ConsoleRing>>,
    /// Handle for the drainer task; Some while the drainer is running.
    pub drainer: Option<tokio::task::JoinHandle<()>>,
    /// Optional stdin fd/sink for writing to the console (None for
    /// read-only backends like provider-relay).
    pub stdin_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
}

impl ConsoleSession {
    fn new(
        provider_kind: ConsoleProviderKind,
        ring: Arc<Mutex<ConsoleRing>>,
        drainer: Option<tokio::task::JoinHandle<()>>,
        stdin_tx: Option<tokio::sync::mpsc::Sender<Vec<u8>>>,
    ) -> Self {
        Self {
            provider_kind,
            ring,
            drainer,
            stdin_tx,
        }
    }
}

/// Opaque per-client session token (UUID string).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ConsoleClientHandle(pub String);

impl ConsoleClientHandle {
    pub fn new() -> Self {
        let mut raw = [0u8; 16];
        getrandom::getrandom(&mut raw).unwrap_or(());
        let hex: String = raw.iter().map(|b| format!("{b:02x}")).collect();
        Self(format!("console-{hex}"))
    }
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for ConsoleClientHandle {
    fn default() -> Self {
        Self::new()
    }
}

/// In-daemon console session table.
///
/// One entry per running VM that has an active drainer.  Multiple clients
/// share the same ring buffer for a given VM.
#[derive(Debug)]
pub struct ConsoleSessionTable {
    /// VM-name → active session.
    sessions: HashMap<String, ConsoleSession>,
    /// Active client handles → VM name.
    clients: HashMap<ConsoleClientHandle, String>,
}

impl ConsoleSessionTable {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            clients: HashMap::new(),
        }
    }
}

impl Default for ConsoleSessionTable {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleSessionTable {
    /// drainer.  Replaces any existing session (e.g. after a VM restart).
    pub fn register_session(&mut self, vm: String, session: ConsoleSession) {
        // Abort any previous drainer for this VM.
        if let Some(old) = self.sessions.remove(&vm) {
            if let Some(task) = old.drainer {
                task.abort();
            }
            // Remove all client handles for the old session.
            self.clients.retain(|_, v| v != &vm);
        }
        self.sessions.insert(vm, session);
    }

    /// Attach a new client to the VM's session, returning an opaque handle
    /// and the ring-buffer start offset at attach time.
    ///
    /// Returns `None` when no session exists for `vm`.
    pub fn attach(&mut self, vm: &str) -> Option<(ConsoleClientHandle, ConsoleProviderKind, u64)> {
        if self.clients.len() >= MAX_SESSIONS {
            return None;
        }
        let session = self.sessions.get(vm)?;
        let start_offset = {
            let guard = session.ring.lock().unwrap();
            guard.ring.base_offset()
        };
        let handle = ConsoleClientHandle::new();
        self.clients.insert(handle.clone(), vm.to_owned());
        Some((handle, session.provider_kind, start_offset))
    }

    /// Read up to `max_len` bytes from the ring buffer for `session_handle`
    /// starting at `offset`.
    ///
    /// Returns `None` when the handle is not found.  When no data is
    /// available at `offset` but the stream has not ended, the caller should
    /// wait on the ring's `notify` before retrying.
    pub fn read_output(
        &self,
        session_handle: &str,
        offset: u64,
        max_len: u64,
    ) -> Option<ConsoleReadOutput> {
        let vm = self
            .clients
            .get(&ConsoleClientHandle(session_handle.to_owned()))?;
        let session = self.sessions.get(vm)?;
        let (result, notify) = {
            let guard = session.ring.lock().unwrap();
            let snap = guard.ring.read_at(offset, max_len);
            let notify = Arc::clone(&guard.notify);
            (snap, notify)
        };
        Some(ConsoleReadOutput {
            vm: vm.clone(),
            provider_kind: session.provider_kind,
            snap: result,
            notify,
        })
    }

    /// Write bytes to the console stdin for `session_handle`.
    ///
    /// Returns `true` when the write was accepted, `false` when no session
    /// is found, and `Err` when the stdin channel is full or closed.
    pub fn write_stdin(&self, session_handle: &str, bytes: Vec<u8>) -> Option<bool> {
        let vm = self
            .clients
            .get(&ConsoleClientHandle(session_handle.to_owned()))?;
        let session = self.sessions.get(vm)?;
        let Some(ref tx) = session.stdin_tx else {
            return Some(false);
        };
        // Non-blocking send: drop on full rather than blocking the daemon.
        Some(tx.try_send(bytes).is_ok())
    }

    /// Close (detach) a client session.  The VM's drainer keeps running.
    pub fn close(&mut self, session_handle: &str) -> bool {
        self.clients
            .remove(&ConsoleClientHandle(session_handle.to_owned()))
            .is_some()
    }

    /// Whether a session exists for `vm`.
    pub fn has_session(&self, vm: &str) -> bool {
        self.sessions.contains_key(vm)
    }

    /// Access the ring [`tokio::sync::Notify`] for a client handle so the
    /// caller can efficiently wait for new data.
    pub fn ring_notify(&self, session_handle: &str) -> Option<Arc<tokio::sync::Notify>> {
        let vm = self
            .clients
            .get(&ConsoleClientHandle(session_handle.to_owned()))?;
        let session = self.sessions.get(vm)?;
        let guard = session.ring.lock().unwrap();
        Some(Arc::clone(&guard.notify))
    }
}

/// Output from [`ConsoleSessionTable::read_output`].
pub struct ConsoleReadOutput {
    pub vm: String,
    pub provider_kind: ConsoleProviderKind,
    pub snap: Option<d2b_core::console_ring::RingReadResult>,
    pub notify: Arc<tokio::sync::Notify>,
}

/// Spawn a drainer task for a Cloud Hypervisor serial socket.
///
/// The task connects to `socket_path` (CH's `--serial socket=<path>`), reads
/// bytes into the ring, and reconnects if CH closes the connection (e.g. after
/// a VM reboot). The ring's `notify` is triggered on each new chunk.
pub fn spawn_ch_serial_drainer(
    _vm: String,
    socket_path: String,
    ring: Arc<Mutex<ConsoleRing>>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn(async move {
        const RECONNECT_DELAY: Duration = Duration::from_millis(500);
        loop {
            match tokio::net::UnixStream::connect(&socket_path).await {
                Err(_) => {
                    tokio::time::sleep(RECONNECT_DELAY).await;
                    continue;
                }
                Ok(mut stream) => {
                    use tokio::io::AsyncReadExt;
                    let mut buf = vec![0u8; 4096];
                    loop {
                        match stream.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                let notify = {
                                    let mut guard = ring.lock().unwrap();
                                    guard.ring.push_bytes(&buf[..n]);
                                    Arc::clone(&guard.notify)
                                };
                                notify.notify_waiters();
                            }
                        }
                    }
                    // CH closed the connection; try to reconnect.
                }
            }
            tokio::time::sleep(RECONNECT_DELAY).await;
        }
    })
}

/// Spawn a drainer task for a pre-opened async stream (e.g. the host end of a
/// qemu-media socketpair, converted to `tokio::net::UnixStream`).
///
/// Unlike [`spawn_ch_serial_drainer`], this does not reconnect after EOF: the
/// socketpair fd is unique and cannot be re-created without broker involvement.
/// On EOF the ring is marked `is_eof = true` and the task exits.
pub fn spawn_fd_drainer(
    vm: String,
    stream: tokio::net::UnixStream,
    ring: Arc<Mutex<ConsoleRing>>,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn(async move {
        use tokio::io::AsyncReadExt;
        let mut stream = stream;
        let mut buf = vec![0u8; 4096];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => {
                    let notify = {
                        let mut guard = ring.lock().unwrap();
                        guard.ring.is_eof = true;
                        Arc::clone(&guard.notify)
                    };
                    notify.notify_waiters();
                    tracing::debug!(vm = %vm, "console fd drainer reached EOF");
                    break;
                }
                Ok(n) => {
                    let notify = {
                        let mut guard = ring.lock().unwrap();
                        guard.ring.push_bytes(&buf[..n]);
                        Arc::clone(&guard.notify)
                    };
                    notify.notify_waiters();
                }
            }
        }
    })
}

/// Create a new [`ConsoleSession`] for a Cloud Hypervisor VM using its serial
/// socket path.
pub fn create_ch_session(socket_path: String) -> ConsoleSession {
    let ring = Arc::new(Mutex::new(ConsoleRing::new()));
    let drainer = spawn_ch_serial_drainer("ch-console".to_owned(), socket_path, Arc::clone(&ring));
    ConsoleSession::new(
        ConsoleProviderKind::LocalHypervisor,
        ring,
        Some(drainer),
        None,
    )
}

/// Create a new [`ConsoleSession`] for a qemu-media VM using a pre-opened
/// UNIX stream socket for the host end of the console socketpair.
///
/// The caller must have already created the stream and converted it from
/// the raw fd (the broker side handles the unsafe `from_raw_fd` conversion
/// and passes the `UnixStream` value here).
pub fn create_qemu_session(std_stream: std::os::unix::net::UnixStream) -> ConsoleSession {
    std_stream.set_nonblocking(true).ok();
    let ring = Arc::new(Mutex::new(ConsoleRing::new()));
    let (stdin_tx, mut stdin_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(32);
    let ring_clone = Arc::clone(&ring);

    // We need both a reader and a writer over the same socket.  Split
    // via tokio's UnixStream from the std socket.
    let drainer = tokio::task::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let stream = match tokio::net::UnixStream::from_std(std_stream) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!("qemu console: failed to convert fd to tokio stream: {e}");
                let mut g = ring_clone.lock().unwrap();
                g.ring.is_eof = true;
                g.notify.notify_waiters();
                return;
            }
        };
        let (mut reader, mut writer) = stream.into_split();
        let ring_write = Arc::clone(&ring_clone);
        // Drive stdin writes to QEMU in a separate spawned task.
        tokio::spawn(async move {
            while let Some(bytes) = stdin_rx.recv().await {
                if writer.write_all(&bytes).await.is_err() {
                    break;
                }
            }
        });
        let mut buf = vec![0u8; 4096];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => {
                    let notify = {
                        let mut g = ring_write.lock().unwrap();
                        g.ring.is_eof = true;
                        Arc::clone(&g.notify)
                    };
                    notify.notify_waiters();
                    break;
                }
                Ok(n) => {
                    let notify = {
                        let mut g = ring_write.lock().unwrap();
                        g.ring.push_bytes(&buf[..n]);
                        Arc::clone(&g.notify)
                    };
                    notify.notify_waiters();
                }
            }
        }
    });
    ConsoleSession::new(
        ConsoleProviderKind::QemuMedia,
        ring,
        Some(drainer),
        Some(stdin_tx),
    )
}

/// Build a [`ConsoleReadOutputResult`] from the output of
/// [`ConsoleSessionTable::read_output`], encoding bytes as base64.
pub fn build_read_output_result(
    session_handle: &str,
    stream: d2b_contracts::terminal_wire::TerminalStream,
    output: ConsoleReadOutput,
) -> ConsoleReadOutputResult {
    match output.snap {
        Some(snap) => ConsoleReadOutputResult {
            session: session_handle.to_owned(),
            stream,
            offset: snap.actual_offset,
            chunk_base64: d2b_core::base64_codec::encode(&snap.data),
            ring_buffer_start_offset: snap.base_offset,
            dropped_bytes: snap.dropped_bytes,
            is_eof: snap.is_eof,
        },
        None => {
            // No data available yet.
            ConsoleReadOutputResult {
                session: session_handle.to_owned(),
                stream,
                offset: 0,
                chunk_base64: String::new(),
                ring_buffer_start_offset: 0,
                dropped_bytes: 0,
                is_eof: false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::public_wire::ConsoleProviderKind;

    fn make_session(provider: ConsoleProviderKind) -> ConsoleSession {
        let ring = Arc::new(Mutex::new(ConsoleRing::new()));
        ConsoleSession::new(provider, ring, None, None)
    }

    #[test]
    fn attach_returns_handle_and_offset() {
        let mut table = ConsoleSessionTable::new();
        table.register_session(
            "vm-a".into(),
            make_session(ConsoleProviderKind::LocalHypervisor),
        );
        let (handle, kind, offset) = table.attach("vm-a").unwrap();
        assert!(!handle.as_str().is_empty());
        assert_eq!(kind, ConsoleProviderKind::LocalHypervisor);
        assert_eq!(offset, 0);
    }

    #[test]
    fn attach_unknown_vm_returns_none() {
        let mut table = ConsoleSessionTable::new();
        assert!(table.attach("no-such-vm").is_none());
    }

    #[test]
    fn close_removes_client() {
        let mut table = ConsoleSessionTable::new();
        table.register_session("vm-b".into(), make_session(ConsoleProviderKind::QemuMedia));
        let (handle, _, _) = table.attach("vm-b").unwrap();
        assert!(table.close(handle.as_str()));
        // closing again is idempotent
        assert!(!table.close(handle.as_str()));
    }

    #[test]
    fn read_output_after_bytes_pushed() {
        let mut table = ConsoleSessionTable::new();
        let ring = Arc::new(Mutex::new(ConsoleRing::new()));
        {
            let mut g = ring.lock().unwrap();
            g.ring.push_bytes(b"hello console");
        }
        let session = ConsoleSession::new(
            ConsoleProviderKind::LocalHypervisor,
            Arc::clone(&ring),
            None,
            None,
        );
        table.register_session("vm-c".into(), session);
        let (handle, _, _) = table.attach("vm-c").unwrap();
        let out = table.read_output(handle.as_str(), 0, 64).unwrap();
        let snap = out.snap.unwrap();
        assert_eq!(snap.data, b"hello console");
    }

    #[test]
    fn read_output_stale_handle_returns_none() {
        let table = ConsoleSessionTable::new();
        assert!(table.read_output("stale-handle", 0, 64).is_none());
    }

    #[test]
    fn register_session_replaces_existing() {
        let mut table = ConsoleSessionTable::new();
        table.register_session(
            "vm-d".into(),
            make_session(ConsoleProviderKind::LocalHypervisor),
        );
        let (handle, _, _) = table.attach("vm-d").unwrap();
        // Replace with a fresh session.
        table.register_session("vm-d".into(), make_session(ConsoleProviderKind::QemuMedia));
        // Old handle should be gone.
        assert!(table.read_output(handle.as_str(), 0, 64).is_none());
    }

    #[test]
    fn slow_client_detects_dropped_bytes() {
        let mut table = ConsoleSessionTable::new();
        let ring = Arc::new(Mutex::new(ConsoleRing::new()));
        let session = ConsoleSession::new(
            ConsoleProviderKind::LocalHypervisor,
            Arc::clone(&ring),
            None,
            None,
        );
        table.register_session("vm-e".into(), session);
        let (handle, _, _) = table.attach("vm-e").unwrap();

        // Fill well past ring capacity to force drops.
        {
            let mut g = ring.lock().unwrap();
            for _ in 0..300 {
                g.ring.push_bytes(&[b'X'; 1024]);
            }
        }
        let out = table.read_output(handle.as_str(), 0, 64).unwrap();
        let snap = out.snap.unwrap();
        assert!(
            snap.dropped_bytes > 0,
            "slow client should detect dropped bytes"
        );
        assert!(
            snap.actual_offset > 0,
            "fast-forward should set actual_offset > 0"
        );
    }
}
