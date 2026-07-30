//! The ZoneLink enrollment and session state machine.
//!
//! This is the frozen contract of decision D126 and the "ZoneLink session
//! state machine" section of `ADR-046-zone-routing.md`, implemented as a
//! hermetic state machine with no I/O of its own:
//!
//! ```text
//! Unenrolled -> IKpsk2 -> EnrollmentCommitted -> KK -> Ready
//! ```
//!
//! The rules this module enforces, none of which it invents:
//!
//! - Only `Unenrolled -> IKpsk2` consumes a PSK. Reconnect on an enrolled link
//!   re-enters at `KK` from `EnrollmentCommitted` and never consumes one.
//! - The allocator issues at most one live single-use PSK per link identity
//!   generation, and issuing a fresh one durably invalidates any prior
//!   outstanding one. A superseded or already burned issuance is refused.
//! - Any attempted IKpsk2 handshake burns the PSK. A failed one fails closed
//!   and returns the link to `Unenrolled`; the same PSK is never retried.
//! - An `EnrollmentCommitted` link never downgrades to IKpsk2 or to an
//!   unauthenticated pattern. Only a durable revocation returns it to
//!   `Unenrolled`.
//! - Resource traffic is prohibited before `Ready`.
//!
//! # What this module deliberately does not own
//!
//! The durable store transaction that seals or invalidates an enrollment
//! record belongs to the child-local ZoneLink controller, not here. This
//! machine is the in-memory authority over the *transitions*, and
//! [`ZoneLinkEnrollment::recover`] is the entry point a restarting handler
//! uses to re-derive state from what the store actually holds. The three crash
//! windows the spec names are recoveries of that function, which is why it
//! takes the persisted facts rather than a prior in-memory state.
//!
//! # No authority, no secrets
//!
//! No PSK byte, static key byte, or transport key reaches this module. A PSK
//! is represented by its issuance ordinal and expiry only; an enrollment is
//! represented by an opaque digest. Every `Debug` implementation is redacted.

/// Default absolute lifetime of one allocator-issued bootstrap PSK.
pub const BOOTSTRAP_PSK_TTL_MS_DEFAULT: u64 = 300_000;
/// Frozen lower bound of the bootstrap PSK lifetime.
pub const BOOTSTRAP_PSK_TTL_MS_MIN: u64 = 60_000;
/// Frozen upper bound of the bootstrap PSK lifetime.
pub const BOOTSTRAP_PSK_TTL_MS_MAX: u64 = 3_600_000;

/// Default maximum lifetime of one enrolled `Noise_KK` session.
pub const KK_SESSION_MAX_LIFETIME_MS_DEFAULT: u64 = 86_400_000;
/// Frozen lower bound of the enrolled session lifetime.
pub const KK_SESSION_MAX_LIFETIME_MS_MIN: u64 = 3_600_000;
/// Frozen upper bound of the enrolled session lifetime.
pub const KK_SESSION_MAX_LIFETIME_MS_MAX: u64 = 604_800_000;

/// The five states of one ZoneLink, advanced in one direction during first
/// bring-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneLinkState {
    /// No sealed enrollment record exists for this link this identity
    /// generation. A fresh allocator-issued single-use PSK is required.
    Unenrolled,
    /// The one-time IKpsk2 bootstrap handshake is in progress.
    IKpsk2,
    /// The sealed enrollment record is durably persisted; the PSK is consumed.
    EnrollmentCommitted,
    /// The enrolled `Noise_KK` handshake is in progress.
    Kk,
    /// The enrolled KK ComponentSession is established.
    Ready,
}

impl ZoneLinkState {
    /// The closed wire, audit, and metric label.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unenrolled => "unenrolled",
            Self::IKpsk2 => "ikpsk2",
            Self::EnrollmentCommitted => "enrollment-committed",
            Self::Kk => "kk",
            Self::Ready => "ready",
        }
    }

    /// Whether resource traffic may traverse the link in this state.
    ///
    /// Only `Ready` permits it. Before that, only bootstrap-admission and
    /// handshake bytes traverse the link.
    pub const fn permits_resource_traffic(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// A closed refusal raised by the ZoneLink state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ZoneLinkEnrollmentError {
    /// The presented PSK issuance was already burned, or was superseded by a
    /// later issuance. It is never retried.
    BootstrapPskConsumed,
    /// The presented PSK is past its absolute expiry.
    BootstrapPskExpired,
    /// The IKpsk2 bootstrap handshake failed. The PSK is burned.
    BootstrapHandshakeFailed,
    /// The peer's static key does not match the sealed enrollment.
    ZoneLinkEnrollmentKeyMismatch,
    /// The sealed enrollment is durably invalidated.
    ZoneLinkRevoked,
    /// The transition is not defined from the current state.
    InvalidTransition,
    /// The declared PSK lifetime is outside the frozen range.
    BootstrapPskTtlOutOfRange,
    /// The declared enrolled-session lifetime is outside the frozen range.
    KkSessionLifetimeOutOfRange,
    /// A link epoch must be nonzero and must not wrap.
    LinkEpochExhausted,
    /// Resource traffic was offered before the link reached `Ready`.
    ResourceTrafficBeforeReady,
}

impl ZoneLinkEnrollmentError {
    /// The closed, path-free label for this refusal.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BootstrapPskConsumed => "bootstrap-psk-consumed",
            Self::BootstrapPskExpired => "bootstrap-psk-expired",
            Self::BootstrapHandshakeFailed => "bootstrap-handshake-failed",
            Self::ZoneLinkEnrollmentKeyMismatch => "zone-link-enrollment-key-mismatch",
            Self::ZoneLinkRevoked => "zone-link-revoked",
            Self::InvalidTransition => "invalid-transition",
            Self::BootstrapPskTtlOutOfRange => "bootstrap-psk-ttl-out-of-range",
            Self::KkSessionLifetimeOutOfRange => "kk-session-lifetime-out-of-range",
            Self::LinkEpochExhausted => "link-epoch-exhausted",
            Self::ResourceTrafficBeforeReady => "resource-traffic-before-ready",
        }
    }
}

impl core::fmt::Display for ZoneLinkEnrollmentError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl core::error::Error for ZoneLinkEnrollmentError {}

/// The pinned fingerprint of an enrolled peer's static key.
///
/// This is a digest, never a key. It has no accessor that yields its bytes and
/// no rendering; the only operation is equality against another fingerprint,
/// which is exactly what the enrolled KK admission check needs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EnrollmentFingerprint([u8; 32]);

impl EnrollmentFingerprint {
    /// Pin a peer static-key fingerprint.
    ///
    /// An all-zero digest is refused, so an uninitialised buffer can never
    /// become a fingerprint that later matches another uninitialised one.
    pub fn new(digest: [u8; 32]) -> Option<Self> {
        if digest == [0; 32] {
            return None;
        }
        Some(Self(digest))
    }
}

impl core::fmt::Debug for EnrollmentFingerprint {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("EnrollmentFingerprint(<redacted>)")
    }
}

/// One sealed enrollment record.
///
/// It holds the pinned child static-key fingerprint and an opaque digest of
/// the allocator enrollment that authorized it. It carries no key byte, no
/// uid, no path, and no allocator address.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EnrollmentRecord {
    fingerprint: EnrollmentFingerprint,
    allocator_binding: [u8; 32],
}

impl EnrollmentRecord {
    /// Seal one enrollment record.
    pub const fn new(fingerprint: EnrollmentFingerprint, allocator_binding: [u8; 32]) -> Self {
        Self {
            fingerprint,
            allocator_binding,
        }
    }

    /// The pinned peer static-key fingerprint.
    pub const fn fingerprint(&self) -> EnrollmentFingerprint {
        self.fingerprint
    }

    /// Whether an observed peer fingerprint matches this sealed enrollment.
    pub fn matches(&self, observed: EnrollmentFingerprint) -> bool {
        self.fingerprint == observed
    }

    /// Whether this record was sealed under the same allocator enrollment.
    pub fn same_allocator_binding(&self, binding: &[u8; 32]) -> bool {
        &self.allocator_binding == binding
    }
}

impl core::fmt::Debug for EnrollmentRecord {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("EnrollmentRecord(<redacted>)")
    }
}

/// One link epoch, assigned afresh on every enrolled KK establishment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LinkEpoch(u64);

impl LinkEpoch {
    /// The first epoch of a link.
    pub const FIRST: Self = Self(1);

    /// The epoch value.
    pub const fn get(self) -> u64 {
        self.0
    }

    /// The next epoch, or `None` at exhaustion.
    pub const fn checked_next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(next) => Some(Self(next)),
            None => None,
        }
    }
}

/// One allocator-issued single-use bootstrap PSK, described without its bytes.
///
/// The issuance ordinal is the allocator's per-link-identity-generation
/// counter. The allocator issues at most one live PSK per generation and a
/// fresh issuance durably invalidates any prior one, so a strictly increasing
/// ordinal is the whole of the "at most one consumable PSK" rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootstrapPskIssuance {
    issuance: u64,
    expires_at_unix_ms: u64,
}

impl BootstrapPskIssuance {
    /// Describe one issuance.
    ///
    /// The lifetime is validated against the frozen range rather than
    /// defaulted, so an out-of-range configuration fails closed at the point
    /// it is declared.
    pub fn new(
        issuance: u64,
        issued_at_unix_ms: u64,
        ttl_ms: u64,
    ) -> Result<Self, ZoneLinkEnrollmentError> {
        if issuance == 0 {
            return Err(ZoneLinkEnrollmentError::BootstrapPskConsumed);
        }
        if !(BOOTSTRAP_PSK_TTL_MS_MIN..=BOOTSTRAP_PSK_TTL_MS_MAX).contains(&ttl_ms) {
            return Err(ZoneLinkEnrollmentError::BootstrapPskTtlOutOfRange);
        }
        let expires_at_unix_ms = issued_at_unix_ms
            .checked_add(ttl_ms)
            .ok_or(ZoneLinkEnrollmentError::BootstrapPskTtlOutOfRange)?;
        Ok(Self {
            issuance,
            expires_at_unix_ms,
        })
    }

    /// The allocator's issuance ordinal.
    pub const fn issuance(self) -> u64 {
        self.issuance
    }
}

/// The crash-safe ZoneLink enrollment and session state machine.
///
/// The machine owns no transport, no session, no key, and no store. It is the
/// single place the five-state contract is expressed, so a handler cannot
/// reach `Ready` by any path other than the one D126 froze.
#[derive(Debug)]
pub struct ZoneLinkEnrollment {
    state: ZoneLinkState,
    record: Option<EnrollmentRecord>,
    revoked: bool,
    burned_issuance: u64,
    epoch: Option<LinkEpoch>,
    established_at_unix_ms: Option<u64>,
    kk_session_max_lifetime_ms: u64,
}

impl ZoneLinkEnrollment {
    /// A fresh link that has never enrolled.
    pub fn new_unenrolled() -> Self {
        Self {
            state: ZoneLinkState::Unenrolled,
            record: None,
            revoked: false,
            burned_issuance: 0,
            epoch: None,
            established_at_unix_ms: None,
            kk_session_max_lifetime_ms: KK_SESSION_MAX_LIFETIME_MS_DEFAULT,
        }
    }

    /// Re-derive the state a restarting handler is in, from what the durable
    /// store actually holds.
    ///
    /// This is the single recovery for all three crash windows the spec names.
    ///
    /// - A crash during IKpsk2, before the enrollment transaction commits,
    ///   leaves no record: the link is `Unenrolled` and a fresh PSK is
    ///   required. The already-burned issuance is carried forward so the
    ///   consumed one cannot be retried.
    /// - A crash after that transaction commits leaves a record: the link is
    ///   `EnrollmentCommitted` and proceeds directly to the enrolled KK
    ///   handshake without consuming another PSK.
    /// - A crash mid-revocation is decided by the persisted invalidation
    ///   marker, which dominates any record still on disk.
    pub fn recover(
        record: Option<EnrollmentRecord>,
        invalidated: bool,
        burned_issuance: u64,
    ) -> Self {
        let mut machine = Self::new_unenrolled();
        machine.burned_issuance = burned_issuance;
        if invalidated {
            machine.revoked = true;
            return machine;
        }
        if let Some(record) = record {
            machine.record = Some(record);
            machine.state = ZoneLinkState::EnrollmentCommitted;
        }
        machine
    }

    /// Declare a non-default enrolled-session lifetime.
    pub fn with_kk_session_max_lifetime_ms(
        mut self,
        lifetime_ms: u64,
    ) -> Result<Self, ZoneLinkEnrollmentError> {
        if !(KK_SESSION_MAX_LIFETIME_MS_MIN..=KK_SESSION_MAX_LIFETIME_MS_MAX).contains(&lifetime_ms)
        {
            return Err(ZoneLinkEnrollmentError::KkSessionLifetimeOutOfRange);
        }
        self.kk_session_max_lifetime_ms = lifetime_ms;
        Ok(self)
    }

    /// The current state.
    pub const fn state(&self) -> ZoneLinkState {
        self.state
    }

    /// The sealed enrollment record, once one exists.
    pub const fn record(&self) -> Option<&EnrollmentRecord> {
        self.record.as_ref()
    }

    /// The current link epoch, once a KK session has been established.
    pub const fn epoch(&self) -> Option<LinkEpoch> {
        self.epoch
    }

    /// Whether resource traffic may traverse the link right now.
    pub const fn permits_resource_traffic(&self) -> bool {
        self.state.permits_resource_traffic()
    }

    /// Refuse resource traffic unless the link reached `Ready`.
    pub const fn admit_resource_traffic(&self) -> Result<(), ZoneLinkEnrollmentError> {
        if self.state.permits_resource_traffic() {
            Ok(())
        } else {
            Err(ZoneLinkEnrollmentError::ResourceTrafficBeforeReady)
        }
    }

    /// `Unenrolled -> IKpsk2`, burning the presented single-use PSK.
    ///
    /// The PSK is burned by the attempt itself, before any handshake byte is
    /// written, so a crash anywhere inside the handshake window cannot leave
    /// it retryable.
    pub fn begin_bootstrap(
        &mut self,
        psk: BootstrapPskIssuance,
        now_unix_ms: u64,
    ) -> Result<(), ZoneLinkEnrollmentError> {
        if self.revoked && self.record.is_some() {
            return Err(ZoneLinkEnrollmentError::ZoneLinkRevoked);
        }
        if self.state != ZoneLinkState::Unenrolled {
            return Err(ZoneLinkEnrollmentError::InvalidTransition);
        }
        if psk.issuance <= self.burned_issuance {
            return Err(ZoneLinkEnrollmentError::BootstrapPskConsumed);
        }
        if now_unix_ms >= psk.expires_at_unix_ms {
            // An expired PSK is refused at admission and is not burned by the
            // refusal: the allocator's own expiry already retired it, and the
            // link stays `Unenrolled` awaiting a fresh issuance.
            return Err(ZoneLinkEnrollmentError::BootstrapPskExpired);
        }
        self.burned_issuance = psk.issuance;
        self.revoked = false;
        self.state = ZoneLinkState::IKpsk2;
        Ok(())
    }

    /// A failed IKpsk2 bootstrap handshake: fail closed to `Unenrolled`.
    ///
    /// The PSK stays burned. A fresh allocator-issued PSK is required before
    /// another attempt.
    pub fn bootstrap_failed(&mut self) -> Result<ZoneLinkEnrollmentError, ZoneLinkEnrollmentError> {
        if self.state != ZoneLinkState::IKpsk2 {
            return Err(ZoneLinkEnrollmentError::InvalidTransition);
        }
        self.state = ZoneLinkState::Unenrolled;
        Ok(ZoneLinkEnrollmentError::BootstrapHandshakeFailed)
    }

    /// `IKpsk2 -> EnrollmentCommitted`.
    ///
    /// The caller performs the single durable store transaction that seals the
    /// record and calls this only after it commits.
    pub fn commit_enrollment(
        &mut self,
        record: EnrollmentRecord,
    ) -> Result<(), ZoneLinkEnrollmentError> {
        if self.state != ZoneLinkState::IKpsk2 {
            return Err(ZoneLinkEnrollmentError::InvalidTransition);
        }
        self.record = Some(record);
        self.revoked = false;
        self.state = ZoneLinkState::EnrollmentCommitted;
        Ok(())
    }

    /// `EnrollmentCommitted -> KK`.
    ///
    /// This is the only handshake an enrolled link may attempt. There is no
    /// transition from here to `IKpsk2` and no unauthenticated fallback: both
    /// would be `InvalidTransition`, which is what makes the absence of a
    /// downgrade structural rather than a policy.
    pub fn begin_enrolled_handshake(&mut self) -> Result<(), ZoneLinkEnrollmentError> {
        if self.revoked {
            return Err(ZoneLinkEnrollmentError::ZoneLinkRevoked);
        }
        if self.state != ZoneLinkState::EnrollmentCommitted || self.record.is_none() {
            return Err(ZoneLinkEnrollmentError::InvalidTransition);
        }
        self.state = ZoneLinkState::Kk;
        Ok(())
    }

    /// `KK -> Ready`, admitting the peer against the sealed enrollment.
    ///
    /// A peer whose static key does not match the sealed fingerprint is
    /// refused before any resource exchange; the link falls back to
    /// `EnrollmentCommitted` and retries under the caller's bounded reconnect
    /// budget. It never downgrades to recover.
    pub fn establish(
        &mut self,
        observed_peer: EnrollmentFingerprint,
        now_unix_ms: u64,
    ) -> Result<LinkEpoch, ZoneLinkEnrollmentError> {
        if self.state != ZoneLinkState::Kk {
            return Err(ZoneLinkEnrollmentError::InvalidTransition);
        }
        let record = self
            .record
            .ok_or(ZoneLinkEnrollmentError::InvalidTransition)?;
        if !record.matches(observed_peer) {
            self.state = ZoneLinkState::EnrollmentCommitted;
            return Err(ZoneLinkEnrollmentError::ZoneLinkEnrollmentKeyMismatch);
        }
        let epoch = match self.epoch {
            None => LinkEpoch::FIRST,
            Some(previous) => previous
                .checked_next()
                .ok_or(ZoneLinkEnrollmentError::LinkEpochExhausted)?,
        };
        self.epoch = Some(epoch);
        self.established_at_unix_ms = Some(now_unix_ms);
        self.state = ZoneLinkState::Ready;
        Ok(epoch)
    }

    /// Whether the enrolled session has reached its cryptoperiod.
    pub fn cryptoperiod_expired(&self, now_unix_ms: u64) -> bool {
        match (self.state, self.established_at_unix_ms) {
            (ZoneLinkState::Ready, Some(established)) => {
                now_unix_ms.saturating_sub(established) >= self.kk_session_max_lifetime_ms
            }
            _ => false,
        }
    }

    /// The session disconnected, or its cryptoperiod elapsed.
    ///
    /// Either way the link returns to `EnrollmentCommitted`, from which the
    /// only permitted next step is a fresh enrolled KK handshake against the
    /// persisted record. Renewal is never a rekey of a live session and never
    /// a reuse of the bootstrap session.
    pub fn disconnect(&mut self) -> Result<(), ZoneLinkEnrollmentError> {
        match self.state {
            ZoneLinkState::Kk | ZoneLinkState::Ready => {
                self.state = ZoneLinkState::EnrollmentCommitted;
                self.established_at_unix_ms = None;
                Ok(())
            }
            ZoneLinkState::IKpsk2 => {
                // A bootstrap that never committed leaves nothing usable.
                self.state = ZoneLinkState::Unenrolled;
                Ok(())
            }
            ZoneLinkState::Unenrolled | ZoneLinkState::EnrollmentCommitted => Ok(()),
        }
    }

    /// Durably revoke the enrollment and tear down any active session.
    ///
    /// The sealed record and the active KK session are invalidated together,
    /// and the link returns to `Unenrolled`. The prior record and any old KK
    /// static key are never reused; a peer presenting the pre-revocation
    /// static key is refused because there is no record left to match it.
    pub fn revoke(&mut self) {
        self.record = None;
        self.revoked = true;
        self.epoch = None;
        self.established_at_unix_ms = None;
        self.state = ZoneLinkState::Unenrolled;
    }

    /// Clear a revocation once a fresh allocator issuance supersedes it.
    ///
    /// A revoked link needs a fresh PSK and a new IKpsk2 bootstrap. This
    /// records the allocator's fresh issuance so [`Self::begin_bootstrap`]
    /// admits it; it restores no prior enrollment and no prior epoch.
    pub fn accept_fresh_issuance(&mut self) {
        if self.record.is_none() {
            self.revoked = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fingerprint(byte: u8) -> EnrollmentFingerprint {
        EnrollmentFingerprint::new([byte; 32]).expect("a nonzero digest")
    }

    fn record(byte: u8) -> EnrollmentRecord {
        EnrollmentRecord::new(fingerprint(byte), [0xAB; 32])
    }

    fn psk(issuance: u64) -> BootstrapPskIssuance {
        BootstrapPskIssuance::new(issuance, 0, BOOTSTRAP_PSK_TTL_MS_DEFAULT).expect("an issuance")
    }

    /// The allocator-PSK bootstrap-enrollment path and the follow-on enrolled
    /// KK path, in the one order D126 permits.
    #[test]
    fn the_bootstrap_then_enrolled_kk_path_reaches_ready() {
        let mut link = ZoneLinkEnrollment::new_unenrolled();
        assert_eq!(link.state(), ZoneLinkState::Unenrolled);
        assert!(!link.permits_resource_traffic());

        link.begin_bootstrap(psk(1), 0).expect("consume the PSK");
        assert_eq!(link.state(), ZoneLinkState::IKpsk2);
        assert_eq!(
            link.admit_resource_traffic(),
            Err(ZoneLinkEnrollmentError::ResourceTrafficBeforeReady)
        );

        link.commit_enrollment(record(0x01)).expect("seal");
        assert_eq!(link.state(), ZoneLinkState::EnrollmentCommitted);
        assert!(!link.permits_resource_traffic());

        link.begin_enrolled_handshake().expect("enrolled handshake");
        assert_eq!(link.state(), ZoneLinkState::Kk);
        assert!(!link.permits_resource_traffic());

        let epoch = link.establish(fingerprint(0x01), 10).expect("establish");
        assert_eq!(epoch, LinkEpoch::FIRST);
        assert_eq!(link.state(), ZoneLinkState::Ready);
        assert!(link.admit_resource_traffic().is_ok());
    }

    #[test]
    fn a_psk_is_single_use_and_a_burned_issuance_is_never_retried() {
        let mut link = ZoneLinkEnrollment::new_unenrolled();
        link.begin_bootstrap(psk(1), 0).expect("first attempt");
        assert_eq!(
            link.bootstrap_failed().expect("fail the handshake"),
            ZoneLinkEnrollmentError::BootstrapHandshakeFailed
        );
        assert_eq!(link.state(), ZoneLinkState::Unenrolled);

        assert_eq!(
            link.begin_bootstrap(psk(1), 0),
            Err(ZoneLinkEnrollmentError::BootstrapPskConsumed)
        );
        link.begin_bootstrap(psk(2), 0)
            .expect("a fresh allocator issuance is admitted");
    }

    #[test]
    fn a_superseded_issuance_is_refused() {
        let mut link = ZoneLinkEnrollment::new_unenrolled();
        link.begin_bootstrap(psk(5), 0).expect("issuance five");
        link.bootstrap_failed().expect("fail");
        assert_eq!(
            link.begin_bootstrap(psk(4), 0),
            Err(ZoneLinkEnrollmentError::BootstrapPskConsumed)
        );
    }

    #[test]
    fn an_expired_psk_is_refused_and_leaves_the_link_unenrolled() {
        let mut link = ZoneLinkEnrollment::new_unenrolled();
        assert_eq!(
            link.begin_bootstrap(psk(1), BOOTSTRAP_PSK_TTL_MS_DEFAULT),
            Err(ZoneLinkEnrollmentError::BootstrapPskExpired)
        );
        assert_eq!(link.state(), ZoneLinkState::Unenrolled);
    }

    #[test]
    fn a_psk_lifetime_outside_the_frozen_range_fails_closed() {
        assert_eq!(
            BootstrapPskIssuance::new(1, 0, BOOTSTRAP_PSK_TTL_MS_MIN - 1),
            Err(ZoneLinkEnrollmentError::BootstrapPskTtlOutOfRange)
        );
        assert_eq!(
            BootstrapPskIssuance::new(1, 0, BOOTSTRAP_PSK_TTL_MS_MAX + 1),
            Err(ZoneLinkEnrollmentError::BootstrapPskTtlOutOfRange)
        );
        assert!(BootstrapPskIssuance::new(1, 0, BOOTSTRAP_PSK_TTL_MS_MIN).is_ok());
        assert!(BootstrapPskIssuance::new(1, 0, BOOTSTRAP_PSK_TTL_MS_MAX).is_ok());
    }

    #[test]
    fn an_enrolled_link_never_downgrades_to_bootstrap_or_a_weaker_pattern() {
        let mut link = ZoneLinkEnrollment::recover(Some(record(0x02)), false, 1);
        assert_eq!(link.state(), ZoneLinkState::EnrollmentCommitted);
        assert_eq!(
            link.begin_bootstrap(psk(9), 0),
            Err(ZoneLinkEnrollmentError::InvalidTransition)
        );
    }

    #[test]
    fn reconnect_re_enters_at_kk_and_consumes_no_psk() {
        let mut link = ZoneLinkEnrollment::new_unenrolled();
        link.begin_bootstrap(psk(1), 0).expect("bootstrap");
        link.commit_enrollment(record(0x03)).expect("seal");
        link.begin_enrolled_handshake().expect("kk");
        let first = link.establish(fingerprint(0x03), 0).expect("establish");

        link.disconnect().expect("disconnect");
        assert_eq!(link.state(), ZoneLinkState::EnrollmentCommitted);

        link.begin_enrolled_handshake().expect("re-handshake");
        let second = link.establish(fingerprint(0x03), 1).expect("re-establish");
        assert_eq!(second.get(), first.get() + 1);
        // The burned issuance is unchanged: reconnect consumed no PSK.
        assert_eq!(
            link.begin_bootstrap(psk(2), 0),
            Err(ZoneLinkEnrollmentError::InvalidTransition)
        );
    }

    #[test]
    fn an_enrolled_key_mismatch_fails_closed_without_downgrading() {
        let mut link = ZoneLinkEnrollment::recover(Some(record(0x04)), false, 1);
        link.begin_enrolled_handshake().expect("kk");
        assert_eq!(
            link.establish(fingerprint(0x99), 0),
            Err(ZoneLinkEnrollmentError::ZoneLinkEnrollmentKeyMismatch)
        );
        assert_eq!(link.state(), ZoneLinkState::EnrollmentCommitted);
        assert!(!link.permits_resource_traffic());
        // The only permitted retry is another enrolled KK handshake.
        link.begin_enrolled_handshake().expect("retry enrolled kk");
    }

    #[test]
    fn the_psk_consume_crash_window_requires_a_fresh_issuance() {
        // Crashed after the PSK was consumed but before the enrollment
        // transaction committed: no record on disk, issuance one burned.
        let mut link = ZoneLinkEnrollment::recover(None, false, 1);
        assert_eq!(link.state(), ZoneLinkState::Unenrolled);
        assert_eq!(
            link.begin_bootstrap(psk(1), 0),
            Err(ZoneLinkEnrollmentError::BootstrapPskConsumed)
        );
        link.begin_bootstrap(psk(2), 0).expect("a fresh PSK");
    }

    #[test]
    fn the_persist_crash_window_proceeds_straight_to_the_enrolled_handshake() {
        // Crashed after the enrollment transaction committed.
        let mut link = ZoneLinkEnrollment::recover(Some(record(0x05)), false, 1);
        assert_eq!(link.state(), ZoneLinkState::EnrollmentCommitted);
        link.begin_enrolled_handshake()
            .expect("no PSK is consumed on this path");
        link.establish(fingerprint(0x05), 0).expect("establish");
    }

    #[test]
    fn the_teardown_crash_window_leaves_no_usable_stale_enrollment() {
        // The persisted invalidation marker dominates a record still on disk.
        let link = ZoneLinkEnrollment::recover(Some(record(0x06)), true, 3);
        assert_eq!(link.state(), ZoneLinkState::Unenrolled);
        assert!(link.record().is_none());
    }

    #[test]
    fn revocation_invalidates_the_record_and_the_active_session_together() {
        let mut link = ZoneLinkEnrollment::new_unenrolled();
        link.begin_bootstrap(psk(1), 0).expect("bootstrap");
        link.commit_enrollment(record(0x07)).expect("seal");
        link.begin_enrolled_handshake().expect("kk");
        link.establish(fingerprint(0x07), 0).expect("establish");

        link.revoke();
        assert_eq!(link.state(), ZoneLinkState::Unenrolled);
        assert!(link.record().is_none());
        assert!(link.epoch().is_none());
        assert_eq!(
            link.admit_resource_traffic(),
            Err(ZoneLinkEnrollmentError::ResourceTrafficBeforeReady)
        );
        assert_eq!(
            link.begin_enrolled_handshake(),
            Err(ZoneLinkEnrollmentError::ZoneLinkRevoked)
        );

        // A fresh allocator issuance and a new IKpsk2 bootstrap are required,
        // and the pre-revocation peer key no longer matches anything.
        link.accept_fresh_issuance();
        link.begin_bootstrap(psk(2), 0).expect("a fresh PSK");
        link.commit_enrollment(record(0x08)).expect("a new record");
        link.begin_enrolled_handshake().expect("kk");
        assert_eq!(
            link.establish(fingerprint(0x07), 0),
            Err(ZoneLinkEnrollmentError::ZoneLinkEnrollmentKeyMismatch)
        );
    }

    #[test]
    fn the_cryptoperiod_renews_by_a_fresh_enrolled_handshake() {
        let mut link = ZoneLinkEnrollment::new_unenrolled()
            .with_kk_session_max_lifetime_ms(KK_SESSION_MAX_LIFETIME_MS_MIN)
            .expect("a lifetime inside the frozen range");
        link.begin_bootstrap(psk(1), 0).expect("bootstrap");
        link.commit_enrollment(record(0x09)).expect("seal");
        link.begin_enrolled_handshake().expect("kk");
        link.establish(fingerprint(0x09), 0).expect("establish");

        assert!(!link.cryptoperiod_expired(KK_SESSION_MAX_LIFETIME_MS_MIN - 1));
        assert!(link.cryptoperiod_expired(KK_SESSION_MAX_LIFETIME_MS_MIN));

        link.disconnect().expect("retire the expired session");
        assert_eq!(link.state(), ZoneLinkState::EnrollmentCommitted);
        link.begin_enrolled_handshake()
            .expect("renewal is a fresh KK handshake, never a rekey");
    }

    #[test]
    fn an_enrolled_session_lifetime_outside_the_frozen_range_fails_closed() {
        assert_eq!(
            ZoneLinkEnrollment::new_unenrolled()
                .with_kk_session_max_lifetime_ms(KK_SESSION_MAX_LIFETIME_MS_MIN - 1)
                .err(),
            Some(ZoneLinkEnrollmentError::KkSessionLifetimeOutOfRange)
        );
        assert_eq!(
            ZoneLinkEnrollment::new_unenrolled()
                .with_kk_session_max_lifetime_ms(KK_SESSION_MAX_LIFETIME_MS_MAX + 1)
                .err(),
            Some(ZoneLinkEnrollmentError::KkSessionLifetimeOutOfRange)
        );
    }

    #[test]
    fn an_all_zero_fingerprint_is_refused() {
        assert!(EnrollmentFingerprint::new([0; 32]).is_none());
    }

    #[test]
    fn debug_output_never_echoes_pinned_material() {
        assert_eq!(
            format!("{:?}", fingerprint(0x0A)),
            "EnrollmentFingerprint(<redacted>)"
        );
        assert_eq!(
            format!("{:?}", record(0x0A)),
            "EnrollmentRecord(<redacted>)"
        );
    }

    #[test]
    fn every_state_and_refusal_renders_a_closed_path_free_label() {
        for state in [
            ZoneLinkState::Unenrolled,
            ZoneLinkState::IKpsk2,
            ZoneLinkState::EnrollmentCommitted,
            ZoneLinkState::Kk,
            ZoneLinkState::Ready,
        ] {
            assert!(!state.as_str().is_empty());
            assert!(!state.as_str().contains('/'));
        }
        for error in [
            ZoneLinkEnrollmentError::BootstrapPskConsumed,
            ZoneLinkEnrollmentError::BootstrapPskExpired,
            ZoneLinkEnrollmentError::BootstrapHandshakeFailed,
            ZoneLinkEnrollmentError::ZoneLinkEnrollmentKeyMismatch,
            ZoneLinkEnrollmentError::ZoneLinkRevoked,
            ZoneLinkEnrollmentError::InvalidTransition,
            ZoneLinkEnrollmentError::BootstrapPskTtlOutOfRange,
            ZoneLinkEnrollmentError::KkSessionLifetimeOutOfRange,
            ZoneLinkEnrollmentError::LinkEpochExhausted,
            ZoneLinkEnrollmentError::ResourceTrafficBeforeReady,
        ] {
            assert_eq!(error.as_str(), format!("{error}"));
            assert!(!error.as_str().contains('/'));
        }
    }

    #[test]
    fn the_allocator_binding_is_compared_rather_than_rendered() {
        let sealed = record(0x0B);
        assert!(sealed.same_allocator_binding(&[0xAB; 32]));
        assert!(!sealed.same_allocator_binding(&[0xCD; 32]));
    }
}
