//! Fixed per-user-session authority for compositor and desktop-session FDs.
//!
//! This module names the authority that owns the compositor, PipeWire, and
//! session-bus open operations. It carries only typed resource references and
//! opaque digests: a socket path, `XDG_RUNTIME_DIR`, display name, seat name,
//! numeric identity, or descriptor never enters the authority index.
//!
//! The authority is intentionally separate from every desktop Provider. A
//! Provider receives an EffectPort/LaunchTicket handoff from this authority;
//! it does not open or retain the session FDs itself.

use std::{
    collections::BTreeMap,
    fmt,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use d2b_contracts::v3::ResourceRef;

/// The fixed authority scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthorityScope {
    /// Bound to one Host, User, and login session.
    Seat,
}

/// Cardinality of an authority class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthorityCardinality {
    /// One incumbent is admitted for a key.
    ExactlyOne,
    /// A class may be absent or have one incumbent per key.
    AtMostOne,
}

/// Arbitration policy for an authority class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthorityArbitration {
    /// A second claimant is rejected before any effect opens.
    Exclusive,
}

/// Whether a holder may cross a Zone boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthorityExportability {
    /// Session descriptors never cross a Zone.
    Forbidden,
}

/// Named desktop/session authority classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AuthorityClass {
    /// The compositor/PipeWire/session-bus opener.
    Session,
    /// Display portal/controller.
    Display,
    /// Clipboard host.
    Clipboard,
    /// Notification sink.
    Notification,
    /// Audio mediator.
    Audio,
    /// systemd user manager.
    SystemdUserManager,
    /// Secret Service/keyring.
    SecretService,
    /// Per-ShellSession supervisor.
    Shell,
    /// Host input and pointer-constraint claimant.
    SeatInput,
}

impl AuthorityClass {
    /// The fixed cardinality for this class.
    pub const fn cardinality(self) -> AuthorityCardinality {
        match self {
            Self::SeatInput => AuthorityCardinality::AtMostOne,
            _ => AuthorityCardinality::ExactlyOne,
        }
    }
}

/// A redacted digest used for session and owner-proof identity.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AuthorityDigest([u8; 32]);

impl AuthorityDigest {
    /// Wrap an opaque digest.
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Whether the digest is the forbidden zero identity.
    pub fn is_zero(self) -> bool {
        self.0 == [0; 32]
    }
}

impl fmt::Debug for AuthorityDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorityDigest(<redacted>)")
    }
}

/// The identity proof for the core/user-agent Process owner.
#[derive(Clone, PartialEq, Eq)]
pub struct OwnerProof {
    process_ref: ResourceRef,
    process_identity: AuthorityDigest,
    session: AuthorityDigest,
}

impl OwnerProof {
    /// Bind a Process identity to one login-session digest.
    pub fn new(
        process_ref: ResourceRef,
        process_identity: AuthorityDigest,
        session: AuthorityDigest,
    ) -> Result<Self, AuthorityError> {
        if process_ref.resource_type().as_str() != "Process"
            || process_identity.is_zero()
            || session.is_zero()
        {
            return Err(AuthorityError::InvalidOwnerProof);
        }
        Ok(Self {
            process_ref,
            process_identity,
            session,
        })
    }

    /// Borrow the owner Process reference.
    pub const fn process_ref(&self) -> &ResourceRef {
        &self.process_ref
    }

    /// Compare two proofs without exposing digest bytes.
    pub fn matches(&self, other: &Self) -> bool {
        self == other
    }
}

impl fmt::Debug for OwnerProof {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("OwnerProof(<redacted>)")
    }
}

/// The immutable descriptor for the fixed session authority.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorityDescriptor {
    /// Authority scope.
    pub scope: AuthorityScope,
    /// Authority cardinality.
    pub cardinality: AuthorityCardinality,
    /// Arbitration policy.
    pub arbitration: AuthorityArbitration,
    /// Exportability policy.
    pub exportability: AuthorityExportability,
}

impl AuthorityDescriptor {
    /// The D097 fixed user-session descriptor.
    pub const fn user_session() -> Self {
        Self {
            scope: AuthorityScope::Seat,
            cardinality: AuthorityCardinality::ExactlyOne,
            arbitration: AuthorityArbitration::Exclusive,
            exportability: AuthorityExportability::Forbidden,
        }
    }
}

/// The key for one authority class under one Host/User/session tuple.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AuthorityKey {
    host: ResourceRef,
    user: ResourceRef,
    session: AuthorityDigest,
    class: AuthorityClass,
}

impl AuthorityKey {
    fn new(
        host: ResourceRef,
        user: ResourceRef,
        session: AuthorityDigest,
        class: AuthorityClass,
    ) -> Result<Self, AuthorityError> {
        if host.resource_type().as_str() != "Host"
            || user.resource_type().as_str() != "User"
            || session.is_zero()
        {
            return Err(AuthorityError::InvalidKey);
        }
        Ok(Self {
            host,
            user,
            session,
            class,
        })
    }
}

impl fmt::Debug for AuthorityKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorityKey(<redacted>)")
    }
}

struct AuthorityRecord {
    owner: OwnerProof,
    guest: Option<ResourceRef>,
    lease_id: u64,
}

/// A request to claim one authority class.
#[derive(Clone, PartialEq, Eq)]
pub struct AuthorityRequest {
    key: AuthorityKey,
    descriptor: AuthorityDescriptor,
    owner: OwnerProof,
    guest: Option<ResourceRef>,
}

impl AuthorityRequest {
    /// Construct a request for the fixed session authority.
    pub fn user_session(
        host: ResourceRef,
        user: ResourceRef,
        session: AuthorityDigest,
        owner: OwnerProof,
    ) -> Result<Self, AuthorityError> {
        Self::new(
            host,
            user,
            session,
            AuthorityClass::Session,
            AuthorityDescriptor::user_session(),
            owner,
            None,
        )
    }

    /// Construct a desktop authority request bound to the same session.
    pub fn new(
        host: ResourceRef,
        user: ResourceRef,
        session: AuthorityDigest,
        class: AuthorityClass,
        descriptor: AuthorityDescriptor,
        owner: OwnerProof,
        guest: Option<ResourceRef>,
    ) -> Result<Self, AuthorityError> {
        if descriptor.scope != AuthorityScope::Seat
            || descriptor.arbitration != AuthorityArbitration::Exclusive
            || descriptor.exportability != AuthorityExportability::Forbidden
            || descriptor.cardinality != class.cardinality()
        {
            return Err(AuthorityError::InvalidDescriptor);
        }
        if let Some(guest) = &guest
            && guest.resource_type().as_str() != "Guest"
        {
            return Err(AuthorityError::InvalidGuestRef);
        }
        if owner.session != session {
            return Err(AuthorityError::OwnerSessionMismatch);
        }
        Ok(Self {
            key: AuthorityKey::new(host, user, session, class)?,
            descriptor,
            owner,
            guest,
        })
    }

    /// The authority class being claimed.
    pub const fn class(&self) -> AuthorityClass {
        self.key.class
    }

    /// The immutable descriptor being claimed.
    pub const fn descriptor(&self) -> AuthorityDescriptor {
        self.descriptor
    }
}

impl fmt::Debug for AuthorityRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorityRequest(<redacted>)")
    }
}

/// A lease proving that this caller owns one admitted authority row.
pub struct AuthorityLease {
    index: Arc<AuthorityIndex>,
    key: AuthorityKey,
    lease_id: u64,
}

impl AuthorityLease {
    /// Whether the lease remains the active incumbent.
    pub fn is_valid(&self) -> bool {
        self.index.is_current(&self.key, self.lease_id)
    }

    /// The class of authority held by this lease.
    pub const fn class(&self) -> AuthorityClass {
        self.key.class
    }
}

impl fmt::Debug for AuthorityLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthorityLease(<redacted>)")
    }
}

impl Drop for AuthorityLease {
    fn drop(&mut self) {
        self.index.release(&self.key, self.lease_id);
    }
}

/// Outcome of attempting restart adoption.
pub enum AdoptionOutcome {
    /// The owner proof matched and the row was adopted.
    Adopted(AuthorityLease),
    /// A row existed but its proof was ambiguous.
    Quarantined,
    /// No row exists for this key.
    Absent,
}

impl fmt::Debug for AdoptionOutcome {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Adopted(_) => "AdoptionOutcome::Adopted(<redacted>)",
            Self::Quarantined => "AdoptionOutcome::Quarantined",
            Self::Absent => "AdoptionOutcome::Absent",
        })
    }
}

/// Core authority index for fixed user-session and dependent authorities.
pub struct AuthorityIndex {
    rows: Mutex<BTreeMap<AuthorityKey, AuthorityRecord>>,
    host_limits: Mutex<BTreeMap<ResourceRef, usize>>,
    next_lease_id: AtomicU64,
}

impl fmt::Debug for AuthorityIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let count = self.rows.lock().map(|rows| rows.len()).unwrap_or_default();
        formatter
            .debug_struct("AuthorityIndex")
            .field("authority_count", &count)
            .finish()
    }
}

impl AuthorityIndex {
    /// Build an empty authority index.
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            rows: Mutex::new(BTreeMap::new()),
            host_limits: Mutex::new(BTreeMap::new()),
            next_lease_id: AtomicU64::new(1),
        })
    }

    /// Set the declared number of user-session authorities for one Host.
    pub fn set_host_limit(&self, host: ResourceRef, limit: usize) -> Result<(), AuthorityError> {
        if host.resource_type().as_str() != "Host" || limit == 0 {
            return Err(AuthorityError::InvalidHostLimit);
        }
        self.host_limits
            .lock()
            .map_err(|_| AuthorityError::StatePoisoned)?
            .insert(host, limit);
        Ok(())
    }

    /// Admit a new authority before any FD-opening effect.
    pub fn admit(
        self: &Arc<Self>,
        request: AuthorityRequest,
    ) -> Result<AuthorityLease, AuthorityError> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityError::StatePoisoned)?;
        if rows.contains_key(&request.key) {
            return Err(AuthorityError::DuplicateConflict);
        }
        if request.class() == AuthorityClass::Session {
            let limit = self
                .host_limits
                .lock()
                .map_err(|_| AuthorityError::StatePoisoned)?
                .get(&request.key.host)
                .copied()
                .unwrap_or(1);
            let used = rows
                .keys()
                .filter(|key| key.host == request.key.host && key.class == AuthorityClass::Session)
                .count();
            if used >= limit {
                return Err(AuthorityError::HostLimitExceeded);
            }
        }
        let lease_id = self.next_lease_id.fetch_add(1, Ordering::AcqRel);
        rows.insert(
            request.key.clone(),
            AuthorityRecord {
                owner: request.owner,
                guest: request.guest,
                lease_id,
            },
        );
        Ok(AuthorityLease {
            index: Arc::clone(self),
            key: request.key,
            lease_id,
        })
    }

    /// Re-adopt an existing row after restart using its exact owner proof.
    pub fn adopt(
        self: &Arc<Self>,
        request: AuthorityRequest,
    ) -> Result<AdoptionOutcome, AuthorityError> {
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityError::StatePoisoned)?;
        let Some(record) = rows.get_mut(&request.key) else {
            return Ok(AdoptionOutcome::Absent);
        };
        if !record.owner.matches(&request.owner) {
            return Ok(AdoptionOutcome::Quarantined);
        }
        let lease_id = self.next_lease_id.fetch_add(1, Ordering::AcqRel);
        record.lease_id = lease_id;
        Ok(AdoptionOutcome::Adopted(AuthorityLease {
            index: Arc::clone(self),
            key: request.key,
            lease_id,
        }))
    }

    /// Invalidate every authority and dependent lease bound to a Guest stop.
    pub fn invalidate_guest(&self, guest: &ResourceRef) -> Result<usize, AuthorityError> {
        if guest.resource_type().as_str() != "Guest" {
            return Err(AuthorityError::InvalidGuestRef);
        }
        let mut rows = self
            .rows
            .lock()
            .map_err(|_| AuthorityError::StatePoisoned)?;
        let before = rows.len();
        rows.retain(|_, record| record.guest.as_ref() != Some(guest));
        Ok(before - rows.len())
    }

    /// Number of currently active authority rows.
    pub fn len(&self) -> Result<usize, AuthorityError> {
        Ok(self
            .rows
            .lock()
            .map_err(|_| AuthorityError::StatePoisoned)?
            .len())
    }

    /// Whether no authority rows are active.
    pub fn is_empty(&self) -> Result<bool, AuthorityError> {
        Ok(self.len()? == 0)
    }

    fn is_current(&self, key: &AuthorityKey, lease_id: u64) -> bool {
        self.rows
            .lock()
            .map(|rows| {
                rows.get(key)
                    .is_some_and(|record| record.lease_id == lease_id)
            })
            .unwrap_or(false)
    }

    fn release(&self, key: &AuthorityKey, lease_id: u64) {
        if let Ok(mut rows) = self.rows.lock()
            && rows
                .get(key)
                .is_some_and(|record| record.lease_id == lease_id)
        {
            rows.remove(key);
        }
    }
}

/// Closed authority-index refusal reasons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityError {
    /// Host/User/session/class tuple is malformed.
    InvalidKey,
    /// The owner proof is not a Process-bound session proof.
    InvalidOwnerProof,
    /// The authority descriptor does not match its fixed class.
    InvalidDescriptor,
    /// A dependent authority names a non-Guest.
    InvalidGuestRef,
    /// The owner proof names another login session.
    OwnerSessionMismatch,
    /// An incumbent already owns this exact authority key.
    DuplicateConflict,
    /// The declared per-Host authority limit is exhausted.
    HostLimitExceeded,
    /// The Host limit is malformed.
    InvalidHostLimit,
    /// The index mutex was poisoned.
    StatePoisoned,
}

impl AuthorityError {
    /// Stable identity-free refusal code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidKey => "authority-key-invalid",
            Self::InvalidOwnerProof => "authority-owner-proof-invalid",
            Self::InvalidDescriptor => "authority-descriptor-invalid",
            Self::InvalidGuestRef => "authority-guest-ref-invalid",
            Self::OwnerSessionMismatch => "authority-owner-session-mismatch",
            Self::DuplicateConflict => "duplicateConflict",
            Self::HostLimitExceeded => "authority-host-limit-exceeded",
            Self::InvalidHostLimit => "authority-host-limit-invalid",
            Self::StatePoisoned => "authority-state-poisoned",
        }
    }
}

impl fmt::Display for AuthorityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for AuthorityError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn host() -> ResourceRef {
        ResourceRef::parse("Host/host-system").unwrap()
    }

    fn user() -> ResourceRef {
        ResourceRef::parse("User/alice").unwrap()
    }

    fn session() -> AuthorityDigest {
        AuthorityDigest::from_bytes([1; 32])
    }

    fn proof(identity: u8) -> OwnerProof {
        OwnerProof::new(
            ResourceRef::parse("Process/user-agent").unwrap(),
            AuthorityDigest::from_bytes([identity; 32]),
            session(),
        )
        .unwrap()
    }

    fn request(identity: u8) -> AuthorityRequest {
        AuthorityRequest::user_session(host(), user(), session(), proof(identity)).unwrap()
    }

    #[test]
    fn duplicate_session_authority_is_rejected_before_effects() {
        let index = AuthorityIndex::new();
        let lease = index.admit(request(2)).unwrap();
        assert!(lease.is_valid());
        assert_eq!(
            index.admit(request(3)).unwrap_err().code(),
            "duplicateConflict"
        );
        drop(lease);
        assert!(index.admit(request(3)).is_ok());
    }

    #[test]
    fn adoption_requires_exact_owner_proof_and_quarantines_ambiguity() {
        let index = AuthorityIndex::new();
        let lease = index.admit(request(2)).unwrap();
        assert!(matches!(
            index.adopt(request(9)).unwrap(),
            AdoptionOutcome::Quarantined
        ));
        let adopted = index.adopt(request(2)).unwrap();
        assert!(matches!(adopted, AdoptionOutcome::Adopted(_)));
        assert!(!lease.is_valid());
    }

    #[test]
    fn host_limit_and_guest_stop_invalidate_dependents() {
        let index = AuthorityIndex::new();
        index.set_host_limit(host(), 1).unwrap();
        let session_lease = index.admit(request(2)).unwrap();
        assert_eq!(
            index.admit(request(3)).unwrap_err(),
            AuthorityError::DuplicateConflict
        );
        let guest = ResourceRef::parse("Guest/dev-vm").unwrap();
        let dependent = AuthorityRequest::new(
            host(),
            user(),
            session(),
            AuthorityClass::Audio,
            AuthorityDescriptor::user_session(),
            proof(2),
            Some(guest.clone()),
        )
        .unwrap();
        let dependent_lease = index.admit(dependent).unwrap();
        assert_eq!(index.invalidate_guest(&guest).unwrap(), 1);
        assert!(!dependent_lease.is_valid());
        assert!(session_lease.is_valid());
    }

    #[test]
    fn malformed_refs_and_descriptor_drift_fail_closed() {
        assert_eq!(
            OwnerProof::new(
                ResourceRef::parse("User/alice").unwrap(),
                AuthorityDigest::from_bytes([1; 32]),
                session(),
            )
            .unwrap_err(),
            AuthorityError::InvalidOwnerProof
        );
        assert_eq!(
            AuthorityRequest::new(
                host(),
                user(),
                session(),
                AuthorityClass::SeatInput,
                AuthorityDescriptor::user_session(),
                proof(2),
                None,
            )
            .unwrap_err(),
            AuthorityError::InvalidDescriptor
        );
    }
}
