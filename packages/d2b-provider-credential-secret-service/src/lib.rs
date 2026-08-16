//! User-session Secret Service Credential Provider.
//!
//! Credential material remains inside the injected [`Oo7SecretServicePort`].
//! This crate handles only validated configuration, opaque lease metadata, and
//! adapter-authorized delivery bindings.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod controller;
mod service;

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

use d2b_contracts::v3::credential::{
    CredentialAuthorization, CredentialLeaseHandle, CredentialLeaseState, CredentialMetadata,
    CredentialOutcomeCode, CredentialServiceError, CredentialServiceErrorCode,
    CredentialSourceVersion, PlacementBinding,
};
use d2b_contracts::v3::{ResourceGeneration, ResourceRef, ZoneId};

pub use controller::{
    SecretServiceController, SecretServiceControllerHealth, SecretServiceStatusProjection,
};

/// Canonical Provider reference.
pub const PROVIDER_REF: &str = "Provider/credential-secret-service";
/// Maximum active leases supported by one Provider instance.
pub const MAX_LOCAL_LEASES: u32 = 256;
/// Maximum bytes in a Secret Service collection alias.
pub const MAX_COLLECTION_ALIAS_BYTES: usize = 128;

/// A boxed asynchronous result returned by the injected port.
pub type SecretServiceFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, SecretServicePortError>> + Send + 'a>>;

/// The only supported process owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceOwner {
    /// The authenticated user-domain process.
    Userd,
}

/// Locked-keyring behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockPolicy {
    /// Fail each operation while the keyring is locked.
    FailClosed,
    /// Project degraded health while the keyring is locked.
    FailDegraded,
}

/// Closed Secret Service state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceState {
    /// The backing collection is locked.
    Locked,
    /// The backing collection is ready.
    Unlocked,
}

/// Closed backing-service failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServicePortError {
    /// The backing collection is locked.
    Locked,
    /// The requested secret is absent from the backing collection.
    Missing,
    /// Backing policy denied the operation.
    Denied,
    /// The backing service is unavailable.
    Unavailable,
    /// The lease expired.
    LeaseExpired,
    /// The lease was revoked.
    LeaseRevoked,
    /// Completion is ambiguous and must not be replayed automatically.
    CompletionUnknown,
}

impl fmt::Display for SecretServicePortError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Locked | Self::Missing | Self::Unavailable => "credential-provider-unavailable",
            Self::Denied => "credential-operation-denied",
            Self::LeaseExpired => "credential-lease-expired",
            Self::LeaseRevoked => "credential-lease-revoked",
            Self::CompletionUnknown => "credential-invariant-failure",
        })
    }
}

impl std::error::Error for SecretServicePortError {}

/// Provider configuration containing no credential material.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretServiceConfig {
    collection_alias: String,
    max_leases: u32,
    lock_policy: LockPolicy,
}

impl SecretServiceConfig {
    /// Validate configuration. Collection aliases may contain spaces but not
    /// controls, quotes, or backslashes.
    pub fn new(
        collection_alias: impl Into<String>,
        max_leases: u32,
        lock_policy: LockPolicy,
    ) -> Result<Self, SecretServiceProviderError> {
        let collection_alias = collection_alias.into();
        if collection_alias.is_empty()
            || collection_alias.len() > MAX_COLLECTION_ALIAS_BYTES
            || !collection_alias
                .bytes()
                .all(|byte| matches!(byte, 0x20..=0x7e) && !matches!(byte, b'"' | b'\\'))
            || !(1..=MAX_LOCAL_LEASES).contains(&max_leases)
        {
            return Err(SecretServiceProviderError::InvalidConfig);
        }
        Ok(Self {
            collection_alias,
            max_leases,
            lock_policy,
        })
    }

    /// Return the configured lease limit.
    pub const fn max_leases(&self) -> u32 {
        self.max_leases
    }

    /// Return locked-keyring behavior.
    pub const fn lock_policy(&self) -> LockPolicy {
        self.lock_policy
    }

    /// Borrow the validated collection alias for the injected port.
    pub fn collection_alias(&self) -> &str {
        &self.collection_alias
    }
}

impl fmt::Debug for SecretServiceConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecretServiceConfig")
            .field("collection_alias", &"<redacted>")
            .field("max_leases", &self.max_leases)
            .field("lock_policy", &self.lock_policy)
            .finish()
    }
}

/// Construction failures with no caller-controlled fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceProviderError {
    /// Configuration failed validation.
    InvalidConfig,
    /// Only user-agent placement is accepted.
    InvalidPlacement,
    /// The execution or user reference has the wrong ResourceType.
    InvalidScope,
    /// The declared consumer is not a Provider reference.
    InvalidConsumer,
    /// The provider-owned session authority could not allocate an identity.
    AuthorityUnavailable,
}

impl fmt::Display for SecretServiceProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfig => "credential schema is invalid",
            Self::InvalidPlacement | Self::InvalidScope => "credential placement mismatch",
            Self::InvalidConsumer => "credential consumer mismatch",
            Self::AuthorityUnavailable => "credential provider unavailable",
        })
    }
}

impl std::error::Error for SecretServiceProviderError {}

/// User-domain placement fixed by the Provider factory.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretServicePlacement {
    zone: ZoneId,
    execution_ref: ResourceRef,
    user_ref: ResourceRef,
}

impl SecretServicePlacement {
    /// Validate user-agent placement on a Host or Guest execution context.
    pub fn new(
        zone: ZoneId,
        binding: PlacementBinding,
        execution_ref: ResourceRef,
        user_ref: ResourceRef,
    ) -> Result<Self, SecretServiceProviderError> {
        if binding != PlacementBinding::UserAgent {
            return Err(SecretServiceProviderError::InvalidPlacement);
        }
        if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest")
            || user_ref.resource_type().as_str() != "User"
        {
            return Err(SecretServiceProviderError::InvalidScope);
        }
        Ok(Self {
            zone,
            execution_ref,
            user_ref,
        })
    }

    /// Borrow the fixed Zone binding.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the fixed execution context.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the fixed user identity.
    pub const fn user_ref(&self) -> &ResourceRef {
        &self.user_ref
    }
}

impl fmt::Debug for SecretServicePlacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretServicePlacement(<redacted>)")
    }
}

/// Opaque acquire request passed to the Secret Service adapter.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretServiceLeaseRequest {
    credential_ref: ResourceRef,
    operation_id: String,
    idempotency_key: String,
    requested_expiry_unix_ms: u64,
}

impl SecretServiceLeaseRequest {
    /// Borrow the routed Credential reference.
    pub const fn credential_ref(&self) -> &ResourceRef {
        &self.credential_ref
    }

    /// Borrow the operation identifier.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Borrow the replay-safe idempotency key.
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    /// Return the requested absolute expiry.
    pub const fn requested_expiry_unix_ms(&self) -> u64 {
        self.requested_expiry_unix_ms
    }
}

impl fmt::Debug for SecretServiceLeaseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretServiceLeaseRequest(<redacted>)")
    }
}

/// Opaque lease reference passed to inspect, refresh, and revoke calls.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretServiceLeaseRef {
    credential_ref: ResourceRef,
    metadata: CredentialMetadata,
}

impl SecretServiceLeaseRef {
    /// Borrow the routed Credential reference.
    pub const fn credential_ref(&self) -> &ResourceRef {
        &self.credential_ref
    }

    /// Borrow the current non-secret metadata.
    pub const fn metadata(&self) -> &CredentialMetadata {
        &self.metadata
    }
}

impl fmt::Debug for SecretServiceLeaseRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretServiceLeaseRef(<redacted>)")
    }
}

/// Non-secret metadata returned after the port retains credential material.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretServiceLeaseGrant {
    /// Opaque non-authorizing lease handle.
    pub lease_handle: CredentialLeaseHandle,
    /// Opaque source version.
    pub source_version: CredentialSourceVersion,
    /// Monotonic rotation generation.
    pub rotation_generation: u64,
    /// Absolute lease expiry.
    pub expires_at_unix_ms: u64,
}

impl fmt::Debug for SecretServiceLeaseGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretServiceLeaseGrant(<redacted>)")
    }
}

/// Non-secret lease inspection.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretServiceLeaseInspection {
    /// Closed lease state.
    pub state: CredentialLeaseState,
    /// Opaque source version.
    pub source_version: CredentialSourceVersion,
    /// Monotonic rotation generation.
    pub rotation_generation: u64,
    /// Absolute lease expiry.
    pub expires_at_unix_ms: u64,
}

impl fmt::Debug for SecretServiceLeaseInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretServiceLeaseInspection(<redacted>)")
    }
}

/// Non-secret refresh result.
pub type SecretServiceLeaseRenewal = SecretServiceLeaseGrant;

/// Idempotent revoke result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretServiceLeaseRevocation {
    /// This call revoked the lease.
    Revoked,
    /// The lease was already revoked.
    AlreadyRevoked,
}

/// Asynchronous semantic boundary implemented by the `oo7` adapter.
///
/// No method accepts or returns credential bytes, object paths, endpoints,
/// file descriptors, or arbitrary diagnostics.
pub trait Oo7SecretServicePort: Send + Sync {
    /// Observe locked or unlocked state.
    fn state(&self) -> SecretServiceFuture<'_, SecretServiceState>;
    /// Retain a new credential lease and return opaque metadata.
    fn issue_lease(
        &self,
        request: &SecretServiceLeaseRequest,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseGrant>;
    /// Inspect one retained lease.
    fn inspect_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseInspection>;
    /// Refresh one retained lease.
    fn refresh_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseRenewal>;
    /// Revoke one retained lease.
    fn revoke_lease(
        &self,
        lease: &SecretServiceLeaseRef,
    ) -> SecretServiceFuture<'_, SecretServiceLeaseRevocation>;
}

#[derive(Clone, PartialEq, Eq)]
struct SessionBinding {
    zone: ZoneId,
    workload: ResourceRef,
    subject: ResourceRef,
    consumer: ResourceRef,
    generation: ResourceGeneration,
}

#[derive(Clone)]
struct AuthoritySession {
    binding: SessionBinding,
    consumed_presentation: Option<u64>,
}

struct SessionAuthorityState {
    next_capability: AtomicU64,
    next_presentation: AtomicU64,
    sessions: Mutex<BTreeMap<u64, AuthoritySession>>,
}

#[derive(Clone)]
struct SessionAuthority {
    identity: u64,
    state: Arc<SessionAuthorityState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionAuthorityError {
    Invalid,
    AlreadyConsumed,
    Released,
    Exhausted,
    InvalidBinding,
}

impl SessionAuthority {
    fn new() -> Result<Self, SecretServiceProviderError> {
        let identity = next_counter(&NEXT_AUTHORITY_ID)
            .map_err(|_| SecretServiceProviderError::AuthorityUnavailable)?;
        Ok(Self {
            identity,
            state: Arc::new(SessionAuthorityState {
                next_capability: AtomicU64::new(0),
                next_presentation: AtomicU64::new(0),
                sessions: Mutex::new(BTreeMap::new()),
            }),
        })
    }

    fn issue(
        &self,
        binding: SessionBinding,
    ) -> Result<SecretServiceSessionCapability, SessionAuthorityError> {
        if !matches!(binding.workload.resource_type().as_str(), "Host" | "Guest")
            || binding.subject.resource_type().as_str() != "User"
            || binding.consumer.resource_type().as_str() != "Provider"
        {
            return Err(SessionAuthorityError::InvalidBinding);
        }
        let capability_id = next_counter(&self.state.next_capability)
            .map_err(|_| SessionAuthorityError::Exhausted)?;
        let presentation = next_counter(&self.state.next_presentation)
            .map_err(|_| SessionAuthorityError::Exhausted)?;
        self.state
            .sessions
            .lock()
            .map_err(|_| SessionAuthorityError::Invalid)?
            .insert(
                capability_id,
                AuthoritySession {
                    binding: binding.clone(),
                    consumed_presentation: None,
                },
            );
        Ok(SecretServiceSessionCapability {
            authority: self.clone(),
            capability_id,
            presentation,
            binding,
        })
    }

    fn consume(
        &self,
        capability: &SecretServiceSessionCapability,
    ) -> Result<(), SessionAuthorityError> {
        if capability.authority.identity != self.identity {
            return Err(SessionAuthorityError::Invalid);
        }
        let mut sessions = self
            .state
            .sessions
            .lock()
            .map_err(|_| SessionAuthorityError::Invalid)?;
        let record = sessions
            .get_mut(&capability.capability_id)
            .ok_or(SessionAuthorityError::Released)?;
        if record.binding != capability.binding {
            return Err(SessionAuthorityError::Invalid);
        }
        if record.consumed_presentation.is_some() {
            return Err(SessionAuthorityError::AlreadyConsumed);
        }
        record.consumed_presentation = Some(capability.presentation);
        Ok(())
    }

    fn release_key(&self, key: SessionKey) -> Result<(), SessionAuthorityError> {
        if key.authority != self.identity {
            return Err(SessionAuthorityError::Invalid);
        }
        let mut sessions = self
            .state
            .sessions
            .lock()
            .map_err(|_| SessionAuthorityError::Invalid)?;
        let record = sessions
            .get(&key.capability_id)
            .ok_or(SessionAuthorityError::Released)?;
        if record.consumed_presentation != Some(key.presentation) {
            return Err(SessionAuthorityError::Invalid);
        }
        sessions.remove(&key.capability_id);
        Ok(())
    }

    fn discard_unconsumed(&self, capability: &SecretServiceSessionCapability) {
        if capability.authority.identity != self.identity {
            return;
        }
        if let Ok(mut sessions) = self.state.sessions.lock()
            && sessions
                .get(&capability.capability_id)
                .is_some_and(|record| record.consumed_presentation.is_none())
        {
            sessions.remove(&capability.capability_id);
        }
    }

    fn clear(&self) -> Result<(), SessionAuthorityError> {
        self.state
            .sessions
            .lock()
            .map_err(|_| SessionAuthorityError::Invalid)?
            .clear();
        Ok(())
    }

    fn discard_key(&self, key: SessionKey) -> Result<(), SessionAuthorityError> {
        if key.authority != self.identity {
            return Err(SessionAuthorityError::Invalid);
        }
        let mut sessions = self
            .state
            .sessions
            .lock()
            .map_err(|_| SessionAuthorityError::Invalid)?;
        let Some(record) = sessions.get(&key.capability_id) else {
            return Ok(());
        };
        if record.consumed_presentation.is_some() {
            return Err(SessionAuthorityError::Invalid);
        }
        sessions.remove(&key.capability_id);
        Ok(())
    }
}

static NEXT_AUTHORITY_ID: AtomicU64 = AtomicU64::new(0);

fn next_counter(counter: &AtomicU64) -> Result<u64, SessionAuthorityError> {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let next = current
            .checked_add(1)
            .ok_or(SessionAuthorityError::Exhausted)?;
        match counter.compare_exchange(current, next, Ordering::AcqRel, Ordering::Relaxed) {
            Ok(_) => return Ok(next),
            Err(observed) => current = observed,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) struct SessionKey {
    authority: u64,
    capability_id: u64,
    presentation: u64,
}

impl fmt::Debug for SessionKey {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SessionKey(<redacted>)")
    }
}

/// Provider-owned, non-Clone session capability.
///
/// The capability has no public constructor. It is issued only by the
/// provider that retains its authority, and the provider authenticates the
/// authority identity before admitting it.
///
/// ```compile_fail
/// # use d2b_provider_credential_secret_service::SecretServiceSessionCapability;
/// fn cannot_clone(capability: SecretServiceSessionCapability) {
///     let _ = capability.clone();
/// }
/// ```
pub struct SecretServiceSessionCapability {
    authority: SessionAuthority,
    capability_id: u64,
    presentation: u64,
    binding: SessionBinding,
}

impl SecretServiceSessionCapability {
    fn session_key(&self) -> SessionKey {
        SessionKey {
            authority: self.authority.identity,
            capability_id: self.capability_id,
            presentation: self.presentation,
        }
    }

    fn binding(&self) -> &SessionBinding {
        &self.binding
    }
}

impl Drop for SecretServiceSessionCapability {
    fn drop(&mut self) {
        self.authority.discard_unconsumed(self);
    }
}

impl fmt::Debug for SecretServiceSessionCapability {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretServiceSessionCapability(<redacted>)")
    }
}

/// Factory fixed to one user placement and exact consumer.
pub struct SecretServiceCredentialProviderFactory {
    config: SecretServiceConfig,
    placement: SecretServicePlacement,
    consumer_ref: ResourceRef,
    generation: ResourceGeneration,
    port: Arc<dyn Oo7SecretServicePort>,
}

impl SecretServiceCredentialProviderFactory {
    /// Build a factory. A present consumer must be a Provider reference;
    /// absence selects this Provider's canonical reference.
    pub fn new(
        config: SecretServiceConfig,
        placement: SecretServicePlacement,
        consumer_ref: Option<ResourceRef>,
        port: Arc<dyn Oo7SecretServicePort>,
    ) -> Result<Self, SecretServiceProviderError> {
        let consumer_ref = match consumer_ref {
            Some(reference) => reference,
            None => ResourceRef::parse(PROVIDER_REF)
                .map_err(|_| SecretServiceProviderError::InvalidConsumer)?,
        };
        if consumer_ref.resource_type().as_str() != "Provider" {
            return Err(SecretServiceProviderError::InvalidConsumer);
        }
        Ok(Self {
            config,
            placement,
            consumer_ref,
            generation: ResourceGeneration::new(1)
                .map_err(|_| SecretServiceProviderError::InvalidScope)?,
            port,
        })
    }

    /// Pin the authority-issued session generation for this Provider.
    pub fn with_generation(mut self, generation: ResourceGeneration) -> Self {
        self.generation = generation;
        self
    }

    /// Construct the service Provider.
    pub fn construct(self) -> Result<SecretServiceCredentialProvider, SecretServiceProviderError> {
        Ok(SecretServiceCredentialProvider {
            config: self.config,
            placement: self.placement,
            consumer_ref: self.consumer_ref,
            generation: self.generation,
            port: self.port,
            authority: SessionAuthority::new()?,
            sessions: Mutex::new(BTreeMap::new()),
            leases: Mutex::new(BTreeMap::new()),
            mutation_gate: Mutex::new(()),
            finalized: AtomicBool::new(false),
        })
    }
}

impl fmt::Debug for SecretServiceCredentialProviderFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretServiceCredentialProviderFactory(<redacted>)")
    }
}

#[derive(Clone)]
struct LeaseRecord {
    idempotency_key: String,
    metadata: CredentialMetadata,
}

/// Secret Service implementation of the prepared Credential service.
pub struct SecretServiceCredentialProvider {
    config: SecretServiceConfig,
    placement: SecretServicePlacement,
    consumer_ref: ResourceRef,
    generation: ResourceGeneration,
    port: Arc<dyn Oo7SecretServicePort>,
    authority: SessionAuthority,
    sessions: Mutex<BTreeMap<SessionKey, ()>>,
    leases: Mutex<BTreeMap<(SessionKey, String), LeaseRecord>>,
    mutation_gate: Mutex<()>,
    finalized: AtomicBool,
}

impl SecretServiceCredentialProvider {
    /// Return the fixed owner classification.
    pub const fn owner(&self) -> SecretServiceOwner {
        SecretServiceOwner::Userd
    }

    /// Borrow the exact consumer expected by authenticated admission.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Borrow the fixed placement.
    pub const fn placement(&self) -> &SecretServicePlacement {
        &self.placement
    }

    /// Borrow validated configuration.
    pub const fn config(&self) -> &SecretServiceConfig {
        &self.config
    }

    /// Issue one authority-backed capability for this exact placement and
    /// configured consumer.
    pub fn issue_session_capability(
        &self,
        generation: ResourceGeneration,
    ) -> Result<SecretServiceSessionCapability, SecretServiceProviderError> {
        let _lifecycle = self
            .blocking_mutation_guard()
            .map_err(|_| SecretServiceProviderError::AuthorityUnavailable)?;
        if self.finalized.load(Ordering::Acquire) || generation != self.generation {
            return Err(SecretServiceProviderError::InvalidScope);
        }
        self.authority
            .issue(SessionBinding {
                zone: self.placement.zone().clone(),
                workload: self.placement.execution_ref().clone(),
                subject: self.placement.user_ref().clone(),
                consumer: self.consumer_ref.clone(),
                generation,
            })
            .map_err(|_| SecretServiceProviderError::AuthorityUnavailable)
    }

    pub(crate) fn authorize_session_locked(
        &self,
        authorization: &CredentialAuthorization,
    ) -> Result<SessionKey, CredentialServiceError> {
        if self.finalized.load(Ordering::Acquire) {
            return Err(CredentialServiceError::new(
                CredentialServiceErrorCode::OperationDenied,
            ));
        }
        let capability = self.session_capability(authorization)?;
        let key = capability.session_key();
        let mut sessions = self.sessions.lock().map_err(|_| {
            CredentialServiceError::new(CredentialServiceErrorCode::InvariantFailure)
        })?;
        if sessions.contains_key(&key) {
            return Ok(key);
        }
        self.authority.consume(capability).map_err(|_| {
            CredentialServiceError::new(CredentialServiceErrorCode::OperationDenied)
        })?;
        sessions.insert(key, ());
        Ok(key)
    }

    pub(crate) fn session_capability<'a>(
        &self,
        authorization: &'a CredentialAuthorization,
    ) -> Result<&'a SecretServiceSessionCapability, CredentialServiceError> {
        let capability = authorization
            .session_proof::<SecretServiceSessionCapability>()
            .ok_or_else(|| {
                CredentialServiceError::new(CredentialServiceErrorCode::OperationDenied)
            })?;
        if capability.authority.identity != self.authority.identity
            || &capability.binding().zone != self.placement.zone()
            || &capability.binding().workload != self.placement.execution_ref()
            || &capability.binding().subject != self.placement.user_ref()
            || capability.binding().consumer != self.consumer_ref
            || capability.binding().generation != self.generation
        {
            return Err(CredentialServiceError::new(
                CredentialServiceErrorCode::OperationDenied,
            ));
        }
        Ok(capability)
    }

    pub(crate) fn map_port_error(error: SecretServicePortError) -> CredentialServiceError {
        let code = match error {
            SecretServicePortError::Locked
            | SecretServicePortError::Missing
            | SecretServicePortError::Unavailable => {
                CredentialServiceErrorCode::ProviderUnavailable
            }
            SecretServicePortError::Denied => CredentialServiceErrorCode::OperationDenied,
            SecretServicePortError::LeaseExpired => CredentialServiceErrorCode::LeaseExpired,
            SecretServicePortError::LeaseRevoked => CredentialServiceErrorCode::LeaseRevoked,
            SecretServicePortError::CompletionUnknown => {
                CredentialServiceErrorCode::InvariantFailure
            }
        };
        CredentialServiceError::new(code)
    }

    pub(crate) fn mutation_guard(&self) -> Result<MutexGuard<'_, ()>, CredentialServiceError> {
        match self.mutation_gate.try_lock() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::WouldBlock) => Err(CredentialServiceError::new(
                CredentialServiceErrorCode::ProviderUnavailable,
            )),
            Err(TryLockError::Poisoned(_)) => Err(CredentialServiceError::new(
                CredentialServiceErrorCode::InvariantFailure,
            )),
        }
    }

    pub(crate) fn blocking_mutation_guard(
        &self,
    ) -> Result<MutexGuard<'_, ()>, CredentialServiceError> {
        self.mutation_gate
            .lock()
            .map_err(|_| CredentialServiceError::new(CredentialServiceErrorCode::InvariantFailure))
    }

    pub(crate) fn release_session_key(
        &self,
        key: SessionKey,
    ) -> Result<(), CredentialServiceError> {
        self.authority
            .release_key(key)
            .map_err(|_| CredentialServiceError::new(CredentialServiceErrorCode::InvariantFailure))
    }

    pub(crate) fn discard_session_key(
        &self,
        key: SessionKey,
    ) -> Result<(), CredentialServiceError> {
        self.authority
            .discard_key(key)
            .map_err(|_| CredentialServiceError::new(CredentialServiceErrorCode::InvariantFailure))
    }

    pub(crate) fn operation_deadline(deadline_ms: u64) -> Result<Instant, CredentialServiceError> {
        Instant::now()
            .checked_add(Duration::from_millis(deadline_ms))
            .ok_or_else(|| {
                CredentialServiceError::new(CredentialServiceErrorCode::DeadlineExceeded)
            })
    }

    pub(crate) fn poll_port<T>(
        mut future: SecretServiceFuture<'_, T>,
        deadline: Instant,
    ) -> Result<T, CredentialServiceError> {
        struct ThreadWake(Thread);
        impl Wake for ThreadWake {
            fn wake(self: Arc<Self>) {
                self.0.unpark();
            }

            fn wake_by_ref(self: &Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = Waker::from(Arc::new(ThreadWake(thread::current())));
        let mut context = Context::from_waker(&waker);
        loop {
            if Instant::now() >= deadline {
                return Err(CredentialServiceError::new(
                    CredentialServiceErrorCode::DeadlineExceeded,
                ));
            }
            match future.as_mut().poll(&mut context) {
                Poll::Ready(result) => return result.map_err(Self::map_port_error),
                Poll::Pending => {
                    let remaining =
                        deadline
                            .checked_duration_since(Instant::now())
                            .ok_or_else(|| {
                                CredentialServiceError::new(
                                    CredentialServiceErrorCode::DeadlineExceeded,
                                )
                            })?;
                    thread::park_timeout(remaining);
                }
            }
        }
    }

    pub(crate) fn grant_metadata(
        grant: SecretServiceLeaseGrant,
        requested_expiry_unix_ms: u64,
    ) -> Result<CredentialMetadata, CredentialServiceError> {
        if grant.rotation_generation == 0
            || grant.expires_at_unix_ms == 0
            || grant.expires_at_unix_ms > requested_expiry_unix_ms
        {
            return Err(CredentialServiceError::new(
                CredentialServiceErrorCode::InvariantFailure,
            ));
        }
        Ok(CredentialMetadata {
            lease_handle: grant.lease_handle,
            rotation_generation: grant.rotation_generation,
            source_version: grant.source_version,
            expires_at_unix_ms: grant.expires_at_unix_ms,
            state: CredentialLeaseState::Active,
            outcome: CredentialOutcomeCode::Success,
        })
    }
}

impl fmt::Debug for SecretServiceCredentialProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretServiceCredentialProvider(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::credential::CredentialMethod;
    use std::sync::Arc;
    use std::thread;

    #[test]
    fn collection_alias_accepts_spaces_and_rejects_unsafe_text() {
        assert!(SecretServiceConfig::new("login collection", 64, LockPolicy::FailClosed).is_ok());
        for rejected in ["", "bad\nname", "bad\\name", "bad\"name"] {
            assert!(SecretServiceConfig::new(rejected, 64, LockPolicy::FailClosed).is_err());
        }
    }

    #[test]
    fn placement_is_user_agent_only() {
        let host = ResourceRef::parse("Host/workstation").unwrap();
        let user = ResourceRef::parse("User/alice").unwrap();
        assert!(
            SecretServicePlacement::new(
                ZoneId::parse("user-zone").unwrap(),
                PlacementBinding::UserAgent,
                host.clone(),
                user.clone(),
            )
            .is_ok()
        );
        assert_eq!(
            SecretServicePlacement::new(
                ZoneId::parse("user-zone").unwrap(),
                PlacementBinding::HostSystem,
                host,
                user,
            ),
            Err(SecretServiceProviderError::InvalidPlacement)
        );
    }

    #[test]
    fn configuration_debug_redacts_collection_alias() {
        let marker = format!("collection-canary-{:x}", std::process::id());
        let config = SecretServiceConfig::new(&marker, 64, LockPolicy::FailClosed).unwrap();
        assert!(!format!("{config:?}").contains(&marker));
        assert_eq!(config.collection_alias(), marker);
    }

    #[test]
    fn same_presentation_concurrent_first_admission_is_idempotent() {
        struct NoopPort;

        impl Oo7SecretServicePort for NoopPort {
            fn state(&self) -> SecretServiceFuture<'_, SecretServiceState> {
                Box::pin(async { Ok(SecretServiceState::Unlocked) })
            }

            fn issue_lease(
                &self,
                _request: &SecretServiceLeaseRequest,
            ) -> SecretServiceFuture<'_, SecretServiceLeaseGrant> {
                Box::pin(async { Err(SecretServicePortError::Unavailable) })
            }

            fn inspect_lease(
                &self,
                _lease: &SecretServiceLeaseRef,
            ) -> SecretServiceFuture<'_, SecretServiceLeaseInspection> {
                Box::pin(async { Err(SecretServicePortError::Unavailable) })
            }

            fn refresh_lease(
                &self,
                _lease: &SecretServiceLeaseRef,
            ) -> SecretServiceFuture<'_, SecretServiceLeaseRenewal> {
                Box::pin(async { Err(SecretServicePortError::Unavailable) })
            }

            fn revoke_lease(
                &self,
                _lease: &SecretServiceLeaseRef,
            ) -> SecretServiceFuture<'_, SecretServiceLeaseRevocation> {
                Box::pin(async { Err(SecretServicePortError::Unavailable) })
            }
        }

        let provider = SecretServiceCredentialProviderFactory::new(
            SecretServiceConfig::new("login", 8, LockPolicy::FailClosed).unwrap(),
            SecretServicePlacement::new(
                ZoneId::parse("user-zone").unwrap(),
                PlacementBinding::UserAgent,
                ResourceRef::parse("Host/workstation").unwrap(),
                ResourceRef::parse("User/alice").unwrap(),
            )
            .unwrap(),
            None,
            Arc::new(NoopPort),
        )
        .unwrap()
        .construct()
        .unwrap();
        let capability = Arc::new(
            provider
                .issue_session_capability(ResourceGeneration::new(1).unwrap())
                .unwrap(),
        );
        let authorization = Arc::new(
            CredentialAuthorization::new(CredentialMethod::InspectMetadata, None)
                .unwrap()
                .with_shared_session_proof(capability),
        );
        let provider = Arc::new(provider);
        let first = {
            let provider = provider.clone();
            let authorization = authorization.clone();
            thread::spawn(move || provider.authorize_session_locked(&authorization))
        };
        let second = {
            let provider = provider.clone();
            let authorization = authorization.clone();
            thread::spawn(move || provider.authorize_session_locked(&authorization))
        };
        let first = first.join().unwrap().unwrap();
        let second = second.join().unwrap().unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn counter_exhaustion_is_fallible() {
        let counter = AtomicU64::new(u64::MAX);
        assert_eq!(
            next_counter(&counter),
            Err(SessionAuthorityError::Exhausted)
        );
    }
}
