//! Pure CTAPHID framing, CID isolation, and ceremony lease state.
//!
//! Host file descriptors, socket binding, peer credentials, and relay task
//! supervision remain in the daemon effect adapter.
//!
//! The ceremony lease is a graph decision now, not a membership check against
//! a list this relay keeps. `SecurityKeyState::enabled_vms` and
//! `SecurityKeyState::enable_vm` are the pre-graph access list: a VM id string
//! that was inserted by configuration decided who could hold the key, and the
//! accept loop's membership test was the whole of the admission. U34 deletes
//! that list and its check. The converted path is [`AdmittedCeremony`]: a
//! ceremony is admitted only while the Guest's `EndpointBinding` to the relay
//! and the Service's exclusive `DeviceBinding` are both current and consistent,
//! so a stale or foreign session cannot pick up a lease - including one a
//! reappearing device's previous owner left behind.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use d2b_contracts_resource::v3::{
    AdmissionStage, BindingConsumerKind, BindingEvidence, BindingKey, BindingKind, BindingRefusal,
    RefusalReason, ResourceRef, StoreIncarnation,
};

use crate::lease::AdmittedDeviceClaim;

use tracing::{debug, info};

// ---------------------------------------------------------------------------
// CTAPHID constants
// ---------------------------------------------------------------------------

/// Fixed size of every CTAPHID report (HID interrupt transfer size).
pub const CTAPHID_REPORT_SIZE: usize = 64;

/// CTAPHID initialization command (CMD byte with continuation-bit set).
pub const CTAPHID_INIT: u8 = 0x86;
/// CTAPHID CANCEL command (client requests cancel of in-progress op).
pub const CTAPHID_CANCEL: u8 = 0x91;
/// CTAPHID ERROR command.
pub const CTAPHID_ERROR: u8 = 0xBF;

/// Broadcast CID used in CTAPHID_INIT requests.
pub const CTAPHID_BROADCAST_CID: u32 = 0xFFFF_FFFF;
/// Marker bit that distinguishes initialization packets from continuation.
pub const CTAPHID_INIT_PKT_BIT: u8 = 0x80;
/// CTAPHID ERR_CHANNEL_BUSY error code.
pub const CTAPHID_ERR_CHANNEL_BUSY: u8 = 0x06;
/// CTAPHID ERR_INVALID_COMMAND error code.
pub const CTAPHID_ERR_INVALID_CMD: u8 = 0x01;

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
    /// Channel identifier the packet is addressed to.
    pub cid: u32,
    /// Command byte with the initialization bit set.
    pub cmd: u8,
    /// Big-endian payload byte count.
    pub bcnt: u16,
    /// Payload bytes carried after the header.
    pub data: Vec<u8>,
}

/// Parsed CTAPHID continuation packet header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtaphidContPacket {
    /// Channel identifier the packet is addressed to.
    pub cid: u32,
    /// Continuation sequence number.
    pub seq: u8,
    /// Payload bytes carried after the header.
    pub data: Vec<u8>,
}

/// Parsed CTAPHID packet (init or continuation).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CtaphidPacket {
    /// An initialization packet starting a new channel.
    Init(CtaphidInitPacket),
    /// A continuation packet of an in-progress message.
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
    /// Construct an empty translation table.
    pub fn new() -> Self {
        Self {
            next_host_cid: 1,
            ..Default::default()
        }
    }

    /// Allocate a fresh host-side CID for a guest's newly established
    /// channel (in response to `CTAPHID_INIT` on the broadcast CID).
    pub fn alloc_host_cid(&mut self, guest_cid: u32) -> u32 {
        if let Some(old_host_cid) = self.guest_to_host.remove(&guest_cid) {
            self.host_to_guest.remove(&old_host_cid);
        }
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
// Admitted ceremony
// ---------------------------------------------------------------------------

/// The two admitted relationships one Guest ceremony rides.
///
/// A ceremony is bounded use of the Service's exclusive device claim, delivered
/// to one Guest through that Guest's own Endpoint relationship. Both keys are
/// the graph's identities and the holder is a `Guest` reference, so a ceremony
/// cannot be built from a VM id, a session name, or anything else the relay or a
/// caller chose. The helper the endpoint is delivered to is read off the
/// relationship rather than supplied, so it is always the consumer the Endpoint
/// source admitted.
#[derive(Clone, PartialEq, Eq)]
pub struct AdmittedCeremony {
    device: BindingKey,
    endpoint: BindingKey,
    guest: ResourceRef,
    helper: ResourceRef,
    epoch: StoreIncarnation,
}

impl AdmittedCeremony {
    /// Bind one Guest's Endpoint relationship to the Service's device claim.
    ///
    /// `guest` is the Binding's own per-Guest identity; the delivery itself
    /// belongs to the frontend helper the Endpoint source admitted, which this
    /// constructor reads off the relationship.
    ///
    /// # Errors
    ///
    /// Returns [`AdmissionStage::Admit`] with
    /// [`RefusalReason::SourcePolicyRefused`] when either relationship is not
    /// currently admitted for use, when the endpoint relationship is not an
    /// `EndpointBinding`, or when `guest` is not a `Guest`, and
    /// [`AdmissionStage::Reserve`] with [`RefusalReason::StaleAuthority`] when
    /// the two relationships were admitted under different store incarnations.
    pub fn new(
        device: &AdmittedDeviceClaim,
        endpoint: &BindingEvidence,
        guest: &ResourceRef,
    ) -> Result<Self, BindingRefusal> {
        if !device.admits_new_use()
            || !endpoint.state().admits_new_use()
            || !endpoint.state().proves_effect()
        {
            return Err(BindingRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::SourcePolicyRefused,
            ));
        }
        if endpoint.key().kind() != BindingKind::Endpoint
            || guest.resource_type().as_str()
                != BindingConsumerKind::Guest.resource_type()
        {
            return Err(BindingRefusal::new(
                AdmissionStage::Admit,
                RefusalReason::SourcePolicyRefused,
            ));
        }
        let Some(epoch) = device.epoch() else {
            return Err(BindingRefusal::new(
                AdmissionStage::Reserve,
                RefusalReason::StaleAuthority,
            ));
        };
        let endpoint_epoch = endpoint
            .admission()
            .dependencies()
            .first()
            .map(|dependency| dependency.store_incarnation());
        if endpoint_epoch != Some(epoch) {
            return Err(BindingRefusal::new(
                AdmissionStage::Reserve,
                RefusalReason::StaleAuthority,
            ));
        }
        Ok(Self {
            device: device.key().clone(),
            endpoint: endpoint.key().clone(),
            guest: guest.clone(),
            helper: endpoint.key().consumer_ref().clone(),
            epoch: epoch.clone(),
        })
    }

    /// The admitted `Device` claim this ceremony rides.
    pub const fn device_key(&self) -> &BindingKey {
        &self.device
    }

    /// The admitted `EndpointBinding` this ceremony is delivered through.
    pub const fn endpoint_key(&self) -> &BindingKey {
        &self.endpoint
    }

    /// The Guest identity this ceremony is held for.
    pub const fn guest(&self) -> &ResourceRef {
        &self.guest
    }

    /// The helper the Endpoint relationship is delivered to.
    pub const fn helper(&self) -> &ResourceRef {
        &self.helper
    }

    /// The store incarnation both relationships were admitted under.
    pub const fn epoch(&self) -> &StoreIncarnation {
        &self.epoch
    }

    /// Whether both live relationships still are the ones this ceremony was
    /// admitted against.
    ///
    /// A device that reappears under another authority, a replaced store, or a
    /// revoked endpoint all make this false, which is what stops a ceremony from
    /// being adopted across the change.
    pub fn matches(&self, device: &AdmittedDeviceClaim, endpoint: &BindingEvidence) -> bool {
        device.key() == &self.device
            && device.epoch() == Some(&self.epoch)
            && device.admits_new_use()
            && endpoint.key() == &self.endpoint
            && endpoint.state().admits_new_use()
            && endpoint.state().proves_effect()
    }
}

impl fmt::Debug for AdmittedCeremony {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AdmittedCeremony")
            .field("device_slot", &self.device.slot())
            .field("endpoint_slot", &self.endpoint.slot())
            .field("guest", &self.guest.to_canonical_string())
            .finish_non_exhaustive()
    }
}

/// Admit one ceremony against the live relationships and take the lease.
///
/// The lease is keyed on the admitted Guest identity rather than on a supplied
/// name, so the only thing that can hold the key is a Guest the Endpoint source
/// admitted and whose Service claim is still current. `None` means no ceremony
/// was admitted: the caller does not get the key and the relay keeps its state.
pub fn admit_ceremony(
    state: &mut SecurityKeyState,
    ceremony: &AdmittedCeremony,
    device: &AdmittedDeviceClaim,
    endpoint: &BindingEvidence,
) -> Option<LeaseId> {
    if !ceremony.matches(device, endpoint) {
        debug!(
            guest = ceremony.guest().to_canonical_string().as_str(),
            reason = "device claim or endpoint relationship is not the admitted one",
            "security-key: ceremony refused"
        );
        return None;
    }
    state.try_acquire_lease(&ceremony.guest().to_canonical_string())
}

/// Release a ceremony's lease, and only for the Guest it was admitted for.
///
/// A mismatched caller is a no-op, which is what keeps a straggling disconnect
/// from releasing a fresh ceremony's lease.
pub fn release_ceremony(state: &mut SecurityKeyState, ceremony: &AdmittedCeremony, lease: LeaseId) {
    state.release_lease(&ceremony.guest().to_canonical_string(), lease);
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

    /// Return the raw counter value.
    pub fn as_u64(self) -> u64 {
        self.0
    }

    /// Construct a lease id from a raw counter value.
    pub const fn from_u64(value: u64) -> Self {
        Self(value)
    }
}

/// State of the physical-key lease.
#[derive(Debug)]
pub enum LeaseState {
    /// No active ceremony; key is available.
    Available,
    /// A ceremony is in progress for the named VM.
    Leased {
        /// VM holding the lease.
        vm_id: String,
        /// Unique identifier of the held lease.
        lease_id: LeaseId,
        /// When the ceremony started.
        started_at: Instant,
        /// How long the ceremony may hold the key before expiry.
        timeout: Duration,
    },
}

impl LeaseState {
    /// Returns `true` if the lease has been held past its timeout.
    pub fn is_expired(&self) -> bool {
        match self {
            Self::Available => false,
            Self::Leased {
                started_at,
                timeout,
                ..
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
    ///
    /// This is the pre-graph access list and it is queued for deletion in U34.
    /// [`admit_ceremony`] is the converted gate: the lease is keyed on the
    /// admitted Guest identity and gated on the current `DeviceBinding` and
    /// `EndpointBinding` rather than on a string this field happens to hold.
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

    /// Authorize a VM to use this relay's resolved physical key.
    ///
    /// Pre-graph path; U34 deletes it with `enabled_vms` and the accept loop's
    /// membership test.
    pub fn enable_vm(&mut self, vm_id: impl Into<String>) {
        self.enabled_vms.insert(vm_id.into());
    }

    /// Try to acquire the lease for `vm_id`. Returns the [`LeaseId`] on
    /// success, or `None` if another VM holds an unexpired lease.
    pub fn try_acquire_lease(&mut self, vm_id: &str) -> Option<LeaseId> {
        if self.lease.is_expired() {
            let expired_holder = self.lease.holder().unwrap_or("<unknown>").to_owned();
            info!(
                vm = expired_holder.as_str(),
                requester = vm_id,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_initialization_and_continuation_reports() {
        let init = build_init_packet(0x0102_0304, CTAPHID_INIT, 3, &[1, 2, 3]);
        assert!(
            matches!(parse_ctaphid_report(&init), CtaphidPacket::Init(packet)
            if packet.cid == 0x0102_0304 && packet.cmd == CTAPHID_INIT && packet.bcnt == 3)
        );

        let mut continuation = [0; CTAPHID_REPORT_SIZE];
        continuation[..4].copy_from_slice(&0x0102_0304u32.to_be_bytes());
        continuation[4] = 2;
        assert!(
            matches!(parse_ctaphid_report(&continuation), CtaphidPacket::Cont(packet)
            if packet.cid == 0x0102_0304 && packet.seq == 2)
        );
    }

    #[test]
    fn cid_translation_isolated_and_released() {
        let mut translator = CidTranslator::new();
        let host = translator.alloc_host_cid(7);
        assert_ne!(host, CTAPHID_BROADCAST_CID);
        assert_eq!(translator.guest_to_host(7), Some(host));
        assert_eq!(translator.host_to_guest(host), Some(7));
        translator.release_guest_cid(7);
        assert_eq!(translator.guest_to_host(7), None);
        assert_eq!(translator.host_to_guest(host), None);
    }

    #[test]
    fn lease_busy_and_release_are_owner_bound() {
        let mut state = SecurityKeyState::new("selector");
        let lease = state.try_acquire_lease("vm-a").expect("first lease");
        assert!(state.try_acquire_lease("vm-b").is_none());
        state.release_lease("vm-b", lease);
        assert!(state.try_acquire_lease("vm-b").is_none());
        state.release_lease("vm-a", lease);
        assert!(state.try_acquire_lease("vm-b").is_some());
    }
}
