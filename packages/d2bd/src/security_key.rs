//! Host-side CTAPHID security-key proxy session management.
//!
//! ## Architecture
//!
//! `d2b-priv-broker` opens the physical hidraw node and hands the fd
//! to `d2bd` via `SCM_RIGHTS` (see
//! `d2b_contracts::broker_wire::OpenHidrawSecurityKey`). This module
//! manages the long-lived session state on top of that fd: it accepts
//! per-VM connections over per-VM Unix sockets (Cloud Hypervisor
//! bridges guest VSOCK to a host Unix socket), authenticates the
//! connection by verifying the peer process credentials (`SO_PEERCRED`
//! — never an in-band guest claim), and relays 64-byte CTAPHID reports
//! between the guest and the physical token.
//!
//! ## Security properties
//!
//! - **CID isolation**: every guest connection is assigned a fresh
//!   host-side channel ID (CID) by [`CidTranslator`]. Guest-provided
//!   CIDs in CTAPHID packets are translated to host-assigned CIDs
//!   before forwarding to the token, and reversed in responses. Two
//!   guest frontends cannot collide on the physical token's CID
//!   namespace.
//! - **One active ceremony**: at most one CTAPHID transaction is
//!   in-flight per physical key ([`SecurityKeyState::try_acquire_lease`]).
//!   A second requester is refused with `ERR_CHANNEL_BUSY`
//!   ([`CTAPHID_ERR_CHANNEL_BUSY`]) until the active ceremony
//!   completes or its [`CEREMONY_TIMEOUT`] elapses.
//! - **Queue-wait bound**: callers waiting for a busy lease should give
//!   up after [`QUEUE_WAIT_TIMEOUT`] rather than blocking indefinitely.
//! - **Lease lifetime**: tied to the guest's connection lifetime. On
//!   disconnect mid-ceremony, the caller must send `CTAPHID_CANCEL`
//!   ([`build_cancel_packet`]) to the token before releasing the lease.
//! - **Peer authentication**: the connecting socket's `SO_PEERCRED` uid
//!   must match the expected per-VM owner ([`authenticate_peer`]).
//!   Socket path (≡ VM identity) is never trusted in-band.
//! - **Log scrubbing**: raw CTAP payloads, PINs, assertions, and
//!   signatures are never logged by this module. Only VM identity,
//!   selector label, high-level op type, and lease lifecycle events
//!   are emitted via `tracing`.
//!
//! ## Blocking I/O
//!
//! [`HidrawDevice`] wraps the fd handed off by the broker in a plain
//! `std::fs::File` (a safe `OwnedFd -> File` conversion — this crate's
//! workspace lints `forbid(unsafe_code)`, so no raw fd reconstruction
//! is used). Reads/writes against it should be dispatched via
//! `tokio::task::spawn_blocking` so the async executor is never
//! stalled waiting for user touch.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::OwnedFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tracing::{debug, info};

// ---------------------------------------------------------------------------
// CTAPHID constants
// ---------------------------------------------------------------------------

/// Fixed size of every CTAPHID report (HID interrupt transfer size).
pub const CTAPHID_REPORT_SIZE: usize = 64;

/// CTAPHID initialization command (CMD byte with continuation-bit set).
pub const CTAPHID_INIT: u8 = 0x86;
/// CTAPHID PING command.
pub const CTAPHID_PING: u8 = 0x81;
/// CTAPHID CANCEL command (client requests cancel of in-progress op).
pub const CTAPHID_CANCEL: u8 = 0x91;
/// CTAPHID ERROR command.
pub const CTAPHID_ERROR: u8 = 0xBF;
/// CTAPHID CBOR command (CTAP2 CBOR messages).
pub const CTAPHID_CBOR: u8 = 0x90;
/// CTAPHID MSG command (U2F/CTAP1 messages).
pub const CTAPHID_MSG: u8 = 0x83;
/// CTAPHID WINK command.
pub const CTAPHID_WINK: u8 = 0x88;
/// CTAPHID KEEPALIVE command.
pub const CTAPHID_KEEPALIVE: u8 = 0xBB;

/// Broadcast CID used in CTAPHID_INIT requests.
pub const CTAPHID_BROADCAST_CID: u32 = 0xFFFF_FFFF;
/// Marker bit that distinguishes initialization packets from continuation.
pub const CTAPHID_INIT_PKT_BIT: u8 = 0x80;
/// CTAPHID ERR_CHANNEL_BUSY error code.
pub const CTAPHID_ERR_CHANNEL_BUSY: u8 = 0x06;
/// CTAPHID ERR_INVALID_COMMAND error code.
pub const CTAPHID_ERR_INVALID_CMD: u8 = 0x01;
/// CTAPHID ERR_INVALID_SEQ error code.
pub const CTAPHID_ERR_INVALID_SEQ: u8 = 0x04;

/// Active-ceremony timeout: how long a single VM may hold the
/// physical-key lease before it is force-expired.
pub const CEREMONY_TIMEOUT: Duration = Duration::from_secs(120);
/// Queue-wait timeout: how long a second requester should wait for a
/// busy lease before giving up.
pub const QUEUE_WAIT_TIMEOUT: Duration = Duration::from_secs(15);

/// A single 64-byte CTAPHID report.
pub type CtaphidReport = [u8; CTAPHID_REPORT_SIZE];

// ---------------------------------------------------------------------------
// Packet parsing
// ---------------------------------------------------------------------------

/// Parsed CTAPHID initialization packet header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtaphidInitPacket {
    pub cid: u32,
    pub cmd: u8,
    pub bcnt: u16,
    pub data: Vec<u8>,
}

/// Parsed CTAPHID continuation packet header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtaphidContPacket {
    pub cid: u32,
    pub seq: u8,
    pub data: Vec<u8>,
}

/// Parsed CTAPHID packet (init or continuation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CtaphidPacket {
    Init(CtaphidInitPacket),
    Cont(CtaphidContPacket),
}

/// Parse a 64-byte raw buffer into a [`CtaphidPacket`].
///
/// CTAPHID framing:
/// - Initialization packet: `CID(4) CMD(1, bit7=1) BCNTH(1) BCNTL(1) DATA(57)`
/// - Continuation packet:   `CID(4) SEQ(1, bit7=0) DATA(59)`
pub fn parse_ctaphid_report(buf: &CtaphidReport) -> CtaphidPacket {
    let cid = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let byte4 = buf[4];
    if byte4 & CTAPHID_INIT_PKT_BIT != 0 {
        let bcnt = u16::from_be_bytes([buf[5], buf[6]]);
        CtaphidPacket::Init(CtaphidInitPacket {
            cid,
            cmd: byte4,
            bcnt,
            data: buf[7..].to_vec(),
        })
    } else {
        CtaphidPacket::Cont(CtaphidContPacket {
            cid,
            seq: byte4,
            data: buf[5..].to_vec(),
        })
    }
}

/// Build a raw 64-byte CTAPHID initialization packet.
pub fn build_init_packet(cid: u32, cmd: u8, bcnt: u16, payload: &[u8]) -> CtaphidReport {
    let mut buf = [0u8; CTAPHID_REPORT_SIZE];
    buf[0..4].copy_from_slice(&cid.to_be_bytes());
    buf[4] = cmd;
    buf[5..7].copy_from_slice(&bcnt.to_be_bytes());
    let copy_len = payload.len().min(57);
    buf[7..7 + copy_len].copy_from_slice(&payload[..copy_len]);
    buf
}

/// Build a 64-byte CTAPHID error report for the given channel ID.
pub fn build_error_report(cid: u32, error_code: u8) -> CtaphidReport {
    build_init_packet(cid, CTAPHID_ERROR, 1, &[error_code])
}

/// Build a 64-byte CTAPHID_CANCEL packet for the given channel ID. Sent
/// to the physical token when a guest disconnects mid-ceremony.
pub fn build_cancel_packet(cid: u32) -> CtaphidReport {
    build_init_packet(cid, CTAPHID_CANCEL, 0, &[])
}

// ---------------------------------------------------------------------------
// CID translation table
// ---------------------------------------------------------------------------

/// Maps guest-side CIDs to host-assigned physical-token CIDs and back.
///
/// Each VM gets a separate namespace. When a guest sends an INIT on
/// the broadcast CID, the broker assigns a fresh host CID and records
/// the mapping. Subsequent packets from the guest with that CID are
/// translated before forwarding to the token, and responses coming
/// back on the host CID are translated back to the guest CID.
#[derive(Debug, Default)]
pub struct CidTranslator {
    /// guest_cid → host_cid
    guest_to_host: HashMap<u32, u32>,
    /// host_cid → guest_cid
    host_to_guest: HashMap<u32, u32>,
    /// Monotonically increasing counter for fresh host CIDs.
    next_host_cid: u32,
}

impl CidTranslator {
    pub fn new() -> Self {
        Self {
            next_host_cid: 1,
            ..Default::default()
        }
    }

    /// Allocate a fresh host-side CID for a guest's newly established
    /// channel (in response to `CTAPHID_INIT` on the broadcast CID).
    pub fn alloc_host_cid(&mut self, guest_cid: u32) -> u32 {
        loop {
            let candidate = self.next_host_cid;
            self.next_host_cid = self.next_host_cid.wrapping_add(1);
            if candidate == 0 || candidate == CTAPHID_BROADCAST_CID {
                continue;
            }
            if !self.host_to_guest.contains_key(&candidate) {
                self.guest_to_host.insert(guest_cid, candidate);
                self.host_to_guest.insert(candidate, guest_cid);
                return candidate;
            }
        }
    }

    /// Translate a guest CID to the corresponding host-side CID.
    pub fn guest_to_host(&self, guest_cid: u32) -> Option<u32> {
        if guest_cid == CTAPHID_BROADCAST_CID {
            return Some(CTAPHID_BROADCAST_CID);
        }
        self.guest_to_host.get(&guest_cid).copied()
    }

    /// Translate a host CID back to the corresponding guest CID.
    pub fn host_to_guest(&self, host_cid: u32) -> Option<u32> {
        if host_cid == CTAPHID_BROADCAST_CID {
            return Some(CTAPHID_BROADCAST_CID);
        }
        self.host_to_guest.get(&host_cid).copied()
    }

    /// Remove a guest channel's CID mapping on channel close.
    pub fn release_guest_cid(&mut self, guest_cid: u32) {
        if let Some(host_cid) = self.guest_to_host.remove(&guest_cid) {
            self.host_to_guest.remove(&host_cid);
        }
    }
}

// ---------------------------------------------------------------------------
// Lease state machine
// ---------------------------------------------------------------------------

/// A unique lease identifier (opaque, process-local monotonic counter).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LeaseId(u64);

impl LeaseId {
    fn new() -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        Self(COUNTER.fetch_add(1, Ordering::Relaxed))
    }

    pub fn as_u64(self) -> u64 {
        self.0
    }
}

/// State of the physical-key lease.
#[derive(Debug)]
pub enum LeaseState {
    /// No active ceremony; key is available.
    Available,
    /// A ceremony is in progress for the named VM.
    Leased {
        vm_id: String,
        lease_id: LeaseId,
        started_at: Instant,
        timeout: Duration,
    },
}

impl LeaseState {
    /// Returns `true` if the lease has been held past its timeout.
    pub fn is_expired(&self) -> bool {
        match self {
            Self::Available => false,
            Self::Leased {
                started_at, timeout, ..
            } => started_at.elapsed() > *timeout,
        }
    }

    /// Returns the VM ID holding the lease, if any.
    pub fn holder(&self) -> Option<&str> {
        match self {
            Self::Available => None,
            Self::Leased { vm_id, .. } => Some(vm_id.as_str()),
        }
    }
}

/// Shared state protected by whatever mutex the caller wraps it in
/// (`d2bd`'s accept-loop uses `parking_lot::Mutex` elsewhere in this
/// crate). Sessions may run concurrently across VMs, but only one may
/// hold the active-ceremony lease at a time.
#[derive(Debug)]
pub struct SecurityKeyState {
    /// VMs that are configured to use the security-key proxy.
    pub enabled_vms: HashSet<String>,
    /// Current lease state for the physical key.
    pub lease: LeaseState,
    /// Stable selector label for log/audit messages (no raw path).
    pub selector_label: String,
}

impl SecurityKeyState {
    /// Create a new, empty security-key state for the given resolved
    /// selector label (as returned by
    /// `OpenHidrawSecurityKeyResponse::selector_resolved`).
    pub fn new(selector_label: impl Into<String>) -> Self {
        Self {
            enabled_vms: HashSet::new(),
            lease: LeaseState::Available,
            selector_label: selector_label.into(),
        }
    }

    /// Try to acquire the lease for `vm_id`. Returns the [`LeaseId`] on
    /// success, or `None` if another VM holds an unexpired lease.
    pub fn try_acquire_lease(&mut self, vm_id: &str) -> Option<LeaseId> {
        if self.lease.is_expired() {
            info!(
                vm = vm_id,
                selector = self.selector_label.as_str(),
                "security-key: expiring stale lease"
            );
            self.lease = LeaseState::Available;
        }
        match &self.lease {
            LeaseState::Available => {
                let id = LeaseId::new();
                self.lease = LeaseState::Leased {
                    vm_id: vm_id.to_owned(),
                    lease_id: id,
                    started_at: Instant::now(),
                    timeout: CEREMONY_TIMEOUT,
                };
                info!(
                    vm = vm_id,
                    lease_id = id.as_u64(),
                    selector = self.selector_label.as_str(),
                    "security-key: lease acquired"
                );
                Some(id)
            }
            LeaseState::Leased { vm_id: holder, .. } => {
                debug!(
                    vm = vm_id,
                    holder = holder.as_str(),
                    "security-key: lease busy"
                );
                None
            }
        }
    }

    /// Release the lease if held by `vm_id` with `lease_id`. A
    /// mismatched caller is a no-op (defence against a straggling
    /// disconnect racing a fresh lease).
    pub fn release_lease(&mut self, vm_id: &str, lease_id: LeaseId) {
        if let LeaseState::Leased {
            vm_id: holder,
            lease_id: held_id,
            ..
        } = &self.lease
            && holder == vm_id
            && *held_id == lease_id
        {
            self.lease = LeaseState::Available;
            info!(
                vm = vm_id,
                lease_id = lease_id.as_u64(),
                selector = self.selector_label.as_str(),
                "security-key: lease released"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Hidraw device handle
// ---------------------------------------------------------------------------

/// Wrapper around the hidraw fd handed off by `d2b-priv-broker` via
/// `SCM_RIGHTS`.
///
/// Built from an [`OwnedFd`] via the safe `std::fs::File: From<OwnedFd>`
/// conversion — this crate's workspace lints `forbid(unsafe_code)`, so
/// no raw fd reconstruction is used anywhere in this module.
#[derive(Debug)]
pub struct HidrawDevice {
    file: File,
}

impl HidrawDevice {
    pub fn from_owned_fd(fd: OwnedFd) -> Self {
        Self { file: File::from(fd) }
    }

    /// Blocking read of a single 64-byte CTAPHID report from the
    /// physical token. Call from `tokio::task::spawn_blocking`.
    pub fn read_report(&self) -> std::io::Result<CtaphidReport> {
        let mut report = [0u8; CTAPHID_REPORT_SIZE];
        (&self.file).read_exact(&mut report)?;
        Ok(report)
    }

    /// Blocking write of a single 64-byte CTAPHID report to the
    /// physical token. Call from `tokio::task::spawn_blocking`.
    pub fn write_report(&self, report: &CtaphidReport) -> std::io::Result<()> {
        (&self.file).write_all(report)
    }
}

// ---------------------------------------------------------------------------
// Framing over the per-VM relay stream
// ---------------------------------------------------------------------------

/// Read a single 64-byte CTAPHID report from a length-prefixed stream.
///
/// The per-VM relay transport uses a 4-byte little-endian length
/// prefix so partial reads are handled cleanly; the length must be
/// exactly [`CTAPHID_REPORT_SIZE`].
pub fn recv_report<R: Read>(stream: &mut R) -> std::io::Result<CtaphidReport> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    if len != CTAPHID_REPORT_SIZE {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("expected {CTAPHID_REPORT_SIZE}-byte CTAPHID report, got {len}"),
        ));
    }
    let mut report = [0u8; CTAPHID_REPORT_SIZE];
    stream.read_exact(&mut report)?;
    Ok(report)
}

/// Write a single 64-byte CTAPHID report to a length-prefixed stream.
pub fn send_report<W: Write>(stream: &mut W, report: &CtaphidReport) -> std::io::Result<()> {
    stream.write_all(&(CTAPHID_REPORT_SIZE as u32).to_le_bytes())?;
    stream.write_all(report)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Peer authentication
// ---------------------------------------------------------------------------

/// Failure returned by [`authenticate_peer`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerAuthError {
    /// `SO_PEERCRED` could not be read from the connected socket.
    PeerCredentialIo { detail: String },
    /// The connecting peer's uid/gid did not match the per-VM socket's
    /// expected owner (the CH VSOCK↔Unix bridge process for that VM).
    PeerCredentialMismatch { peer_uid: u32, peer_gid: u32 },
}

impl std::fmt::Display for PeerAuthError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::PeerCredentialIo { detail } => write!(f, "SO_PEERCRED read failed: {detail}"),
            Self::PeerCredentialMismatch { peer_uid, peer_gid } => write!(
                f,
                "peer credential mismatch: uid={peer_uid} gid={peer_gid} not the expected per-VM owner"
            ),
        }
    }
}

impl std::error::Error for PeerAuthError {}

/// Verify that the connecting peer's `SO_PEERCRED` identity matches
/// the expected per-VM socket owner.
///
/// The per-VM socket *path* is never trusted as identity on its own —
/// the CH VSOCK↔Unix bridge process's real kernel-verified uid/gid must
/// match what the trusted bundle recorded for that VM's socket.
pub fn authenticate_peer<F: std::os::fd::AsFd>(
    socket: &F,
    expected_uid: u32,
    expected_gid: u32,
) -> Result<(), PeerAuthError> {
    use nix::sys::socket::{getsockopt, sockopt::PeerCredentials};
    let peer = getsockopt(socket, PeerCredentials).map_err(|err| PeerAuthError::PeerCredentialIo {
        detail: err.to_string(),
    })?;
    let peer_uid = peer.uid();
    let peer_gid = peer.gid();
    if peer_uid != expected_uid || peer_gid != expected_gid {
        return Err(PeerAuthError::PeerCredentialMismatch {
            peer_uid,
            peer_gid,
        });
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    // -----------------------------------------------------------------------
    // CTAPHID packet parsing
    // -----------------------------------------------------------------------

    #[test]
    fn parse_init_packet_identifies_cmd_and_cid() {
        let mut buf = [0u8; CTAPHID_REPORT_SIZE];
        buf[0..4].copy_from_slice(&[0x01, 0x02, 0x03, 0x04]);
        buf[4] = CTAPHID_INIT;
        buf[5] = 0x00;
        buf[6] = 0x08;

        let pkt = parse_ctaphid_report(&buf);
        match pkt {
            CtaphidPacket::Init(p) => {
                assert_eq!(p.cid, 0x0102_0304);
                assert_eq!(p.cmd, CTAPHID_INIT);
                assert_eq!(p.bcnt, 8);
            }
            _ => panic!("expected Init packet"),
        }
    }

    #[test]
    fn parse_continuation_packet_identifies_seq_and_cid() {
        let mut buf = [0u8; CTAPHID_REPORT_SIZE];
        buf[0..4].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        buf[4] = 0x03;
        buf[5] = 0xAB;

        let pkt = parse_ctaphid_report(&buf);
        match pkt {
            CtaphidPacket::Cont(p) => {
                assert_eq!(p.cid, 0xDEAD_BEEF);
                assert_eq!(p.seq, 3);
                assert_eq!(p.data[0], 0xAB);
            }
            _ => panic!("expected Cont packet"),
        }
    }

    #[test]
    fn broadcast_cid_parsed_in_init_packet() {
        let mut buf = [0u8; CTAPHID_REPORT_SIZE];
        buf[0..4].copy_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]);
        buf[4] = CTAPHID_INIT;
        let pkt = parse_ctaphid_report(&buf);
        match pkt {
            CtaphidPacket::Init(p) => assert_eq!(p.cid, CTAPHID_BROADCAST_CID),
            _ => panic!("expected Init with broadcast CID"),
        }
    }

    // -----------------------------------------------------------------------
    // CID translation
    // -----------------------------------------------------------------------

    #[test]
    fn cid_translator_allocs_fresh_host_cid() {
        let mut t = CidTranslator::new();
        let host = t.alloc_host_cid(42);
        assert_ne!(host, 0);
        assert_ne!(host, CTAPHID_BROADCAST_CID);
        assert_eq!(t.guest_to_host(42), Some(host));
        assert_eq!(t.host_to_guest(host), Some(42));
    }

    #[test]
    fn cid_translator_two_guests_get_different_host_cids() {
        let mut t = CidTranslator::new();
        let host1 = t.alloc_host_cid(10);
        let host2 = t.alloc_host_cid(20);
        assert_ne!(host1, host2);
    }

    #[test]
    fn cid_translator_broadcast_passes_through() {
        let t = CidTranslator::new();
        assert_eq!(
            t.guest_to_host(CTAPHID_BROADCAST_CID),
            Some(CTAPHID_BROADCAST_CID)
        );
        assert_eq!(
            t.host_to_guest(CTAPHID_BROADCAST_CID),
            Some(CTAPHID_BROADCAST_CID)
        );
    }

    #[test]
    fn cid_translator_release_removes_mapping() {
        let mut t = CidTranslator::new();
        t.alloc_host_cid(77);
        t.release_guest_cid(77);
        assert_eq!(t.guest_to_host(77), None);
    }

    // -----------------------------------------------------------------------
    // Lease state machine
    // -----------------------------------------------------------------------

    #[test]
    fn lease_acquire_succeeds_when_available() {
        let mut state = SecurityKeyState::new("test-selector");
        let id = state.try_acquire_lease("vm-a");
        assert!(id.is_some());
        assert!(matches!(state.lease, LeaseState::Leased { .. }));
    }

    #[test]
    fn lease_acquire_fails_when_held_by_other_vm() {
        let mut state = SecurityKeyState::new("test-selector");
        let _ = state.try_acquire_lease("vm-a");
        let second = state.try_acquire_lease("vm-b");
        assert!(
            second.is_none(),
            "second VM must not acquire lease while first holds it"
        );
    }

    #[test]
    fn lease_release_makes_key_available() {
        let mut state = SecurityKeyState::new("test-selector");
        let id = state.try_acquire_lease("vm-a").unwrap();
        state.release_lease("vm-a", id);
        assert!(matches!(state.lease, LeaseState::Available));
    }

    #[test]
    fn lease_release_wrong_vm_does_not_release() {
        let mut state = SecurityKeyState::new("test-selector");
        let id = state.try_acquire_lease("vm-a").unwrap();
        // vm-b tries to release vm-a's lease — must be a no-op.
        state.release_lease("vm-b", id);
        assert!(matches!(state.lease, LeaseState::Leased { .. }));
    }

    #[test]
    fn lease_expired_returns_is_expired_true() {
        let lease = LeaseState::Leased {
            vm_id: "vm-a".to_owned(),
            lease_id: LeaseId(1),
            started_at: Instant::now() - Duration::from_secs(200),
            timeout: Duration::from_secs(120),
        };
        assert!(lease.is_expired());
    }

    #[test]
    fn expired_lease_is_evicted_on_next_acquire() {
        let mut state = SecurityKeyState::new("test-selector");
        state.lease = LeaseState::Leased {
            vm_id: "vm-a".to_owned(),
            lease_id: LeaseId(1),
            started_at: Instant::now() - Duration::from_secs(200),
            timeout: Duration::from_secs(120),
        };
        let id = state.try_acquire_lease("vm-b");
        assert!(
            id.is_some(),
            "expired lease must be evicted and new acquirer must succeed"
        );
    }

    #[test]
    fn contention_second_vm_cannot_acquire_active_lease() {
        let mut state = SecurityKeyState::new("test-selector");
        state.enabled_vms.insert("vm-a".to_owned());
        state.enabled_vms.insert("vm-b".to_owned());
        let id = state.try_acquire_lease("vm-a").unwrap();
        assert!(id.as_u64() > 0);
        let id2 = state.try_acquire_lease("vm-b");
        assert!(id2.is_none());
    }

    #[test]
    fn disabled_vm_is_not_registered_in_enabled_set() {
        let state = SecurityKeyState::new("test-selector");
        // The relay accept-loop must check `enabled_vms` before ever
        // calling `try_acquire_lease` for a VM not configured to use
        // the security-key proxy.
        assert!(!state.enabled_vms.contains("vm-a"));
    }

    // -----------------------------------------------------------------------
    // CTAPHID framing (recv_report / send_report round-trip)
    // -----------------------------------------------------------------------

    #[test]
    fn framing_round_trip_over_buffer() {
        let mut buf = [0u8; CTAPHID_REPORT_SIZE];
        buf[0..4].copy_from_slice(&[0x01, 0x00, 0x00, 0x01]);
        buf[4] = CTAPHID_CBOR;
        buf[5] = 0x00;
        buf[6] = 0x01;
        buf[7] = 0x04; // CBOR authenticatorGetInfo

        let mut wire: Vec<u8> = Vec::new();
        send_report(&mut wire, &buf).unwrap();

        let mut cursor = Cursor::new(wire);
        let received = recv_report(&mut cursor).unwrap();
        assert_eq!(received, buf);
    }

    #[test]
    fn framing_rejects_wrong_length_prefix() {
        let mut wire: Vec<u8> = Vec::new();
        wire.extend_from_slice(&32u32.to_le_bytes()); // wrong: 32 instead of 64
        wire.extend_from_slice(&[0u8; 32]);

        let mut cursor = Cursor::new(wire);
        let result = recv_report(&mut cursor);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Error / cancel report builders
    // -----------------------------------------------------------------------

    #[test]
    fn build_error_report_sets_correct_fields() {
        let report = build_error_report(0x0A0B_0C0D, CTAPHID_ERR_CHANNEL_BUSY);
        let pkt = parse_ctaphid_report(&report);
        match pkt {
            CtaphidPacket::Init(p) => {
                assert_eq!(p.cid, 0x0A0B_0C0D);
                assert_eq!(p.cmd, CTAPHID_ERROR);
                assert_eq!(p.bcnt, 1);
                assert_eq!(p.data[0], CTAPHID_ERR_CHANNEL_BUSY);
            }
            _ => panic!("expected Init (error) packet"),
        }
    }

    #[test]
    fn build_cancel_packet_targets_given_cid() {
        let report = build_cancel_packet(0xAABB_CCDD);
        let pkt = parse_ctaphid_report(&report);
        match pkt {
            CtaphidPacket::Init(p) => {
                assert_eq!(p.cid, 0xAABB_CCDD);
                assert_eq!(p.cmd, CTAPHID_CANCEL);
                assert_eq!(p.bcnt, 0);
            }
            _ => panic!("expected Init (cancel) packet"),
        }
    }

    // -----------------------------------------------------------------------
    // Hidraw device wrapper (hermetic: uses /dev/null, no unsafe)
    // -----------------------------------------------------------------------

    #[test]
    fn hidraw_device_from_owned_fd_wraps_without_unsafe() {
        let file = File::open("/dev/null").expect("open /dev/null");
        let fd: OwnedFd = file.into();
        let device = HidrawDevice::from_owned_fd(fd);
        // /dev/null reads always return EOF (0 bytes); read_exact on a
        // 64-byte buffer against it must fail with UnexpectedEof, not
        // panic or hang.
        let err = device.read_report().unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn hidraw_device_write_report_to_dev_null_succeeds() {
        let file = File::options()
            .write(true)
            .open("/dev/null")
            .expect("open /dev/null for write");
        let fd: OwnedFd = file.into();
        let device = HidrawDevice::from_owned_fd(fd);
        let report = build_error_report(1, CTAPHID_ERR_INVALID_CMD);
        device.write_report(&report).expect("write to /dev/null");
    }

    // -----------------------------------------------------------------------
    // Peer authentication
    // -----------------------------------------------------------------------

    #[test]
    fn authenticate_peer_accepts_matching_current_process_credentials() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().expect("socketpair");
        let uid = nix::unistd::geteuid().as_raw();
        let gid = nix::unistd::getegid().as_raw();
        authenticate_peer(&a, uid, gid).expect("current process credentials should match");
    }

    #[test]
    fn authenticate_peer_rejects_mismatched_expected_identity() {
        let (a, _b) = std::os::unix::net::UnixStream::pair().expect("socketpair");
        let result = authenticate_peer(&a, 999_999, 999_999);
        assert!(matches!(
            result,
            Err(PeerAuthError::PeerCredentialMismatch { .. })
        ));
    }
}
