//! Per-Zone Unix datagram receiver for bounded telemetry frames.

use std::{
    collections::VecDeque,
    fs, io,
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
};

/// Receiver state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiverReadiness {
    /// The socket exists and at least one drain cycle completed.
    Ready,
    /// The socket is not yet available.
    Pending,
    /// The socket exists but a drain failed.
    Failed,
}

/// A bounded datagram receiver.
pub struct EmitterSocket {
    path: PathBuf,
    socket: UnixDatagram,
    frames: VecDeque<Vec<u8>>,
    capacity_bytes: usize,
    queued_bytes: usize,
    dropped: u64,
    readiness: ReceiverReadiness,
}

impl core::fmt::Debug for EmitterSocket {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("EmitterSocket")
            .field("ready", &self.readiness)
            .field("queued_frames", &self.frames.len())
            .field("dropped", &self.dropped)
            .finish()
    }
}

impl EmitterSocket {
    /// Bind a per-Zone socket without replacing an existing pathname.
    pub fn bind(path: impl AsRef<Path>, capacity_bytes: usize) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let socket = UnixDatagram::bind(&path)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            path,
            socket,
            frames: VecDeque::new(),
            capacity_bytes: capacity_bytes.max(1),
            queued_bytes: 0,
            dropped: 0,
            readiness: ReceiverReadiness::Pending,
        })
    }

    /// Drain available datagrams into the bounded FIFO.
    pub fn drain_once(&mut self) -> io::Result<usize> {
        let mut drained = 0;
        loop {
            let mut bytes = vec![0_u8; 4 * 1024 * 1024];
            match self.socket.recv(&mut bytes) {
                Ok(size) => {
                    bytes.truncate(size);
                    while self.queued_bytes.saturating_add(bytes.len()) > self.capacity_bytes {
                        let Some(oldest) = self.frames.pop_front() else {
                            break;
                        };
                        self.queued_bytes = self.queued_bytes.saturating_sub(oldest.len());
                        self.dropped = self.dropped.saturating_add(1);
                    }
                    if bytes.len() > self.capacity_bytes {
                        self.dropped = self.dropped.saturating_add(1);
                    } else {
                        self.queued_bytes += bytes.len();
                        self.frames.push_back(bytes);
                    }
                    drained += 1;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => {
                    self.readiness = ReceiverReadiness::Failed;
                    return Err(error);
                }
            }
        }
        self.readiness = ReceiverReadiness::Ready;
        Ok(drained)
    }

    /// Pop the oldest received frame.
    pub fn pop(&mut self) -> Option<Vec<u8>> {
        let frame = self.frames.pop_front()?;
        self.queued_bytes = self.queued_bytes.saturating_sub(frame.len());
        Some(frame)
    }

    /// Current receiver readiness.
    pub const fn readiness(&self) -> ReceiverReadiness {
        self.readiness
    }

    /// Number of frames dropped due to bounded storage.
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Number of queued frames.
    pub fn queued(&self) -> usize {
        self.frames.len()
    }

    /// Borrow the owned socket path for activation diagnostics.
    pub fn socket_path(&self) -> &Path {
        &self.path
    }
}

impl Drop for EmitterSocket {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        os::unix::net::UnixDatagram,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn receiver_drains_datagrams_and_reports_ready() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("d2b-otel-emitter-{nonce}.sock"));
        let mut receiver = EmitterSocket::bind(&path, 128).unwrap();
        let sender = UnixDatagram::unbound().unwrap();
        sender.send_to(b"frame", &path).unwrap();
        assert_eq!(receiver.drain_once().unwrap(), 1);
        assert_eq!(receiver.readiness(), ReceiverReadiness::Ready);
        assert_eq!(receiver.pop().as_deref(), Some(&b"frame"[..]));
    }
}
