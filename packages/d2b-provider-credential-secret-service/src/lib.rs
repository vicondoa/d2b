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
use std::sync::{Arc, Mutex, MutexGuard, TryLockError};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};
use std::time::{Duration, Instant};

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::{
    CredentialLeaseHandle, CredentialLeaseState, CredentialMetadata, CredentialOutcomeCode,
    CredentialServiceError, CredentialServiceErrorCode, CredentialSourceVersion, PlacementBinding,
};

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
            Self::Locked | Self::Unavailable => "credential-provider-unavailable",
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
}

impl fmt::Display for SecretServiceProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfig => "credential schema is invalid",
            Self::InvalidPlacement | Self::InvalidScope => "credential placement mismatch",
            Self::InvalidConsumer => "credential consumer mismatch",
        })
    }
}

impl std::error::Error for SecretServiceProviderError {}

/// User-domain placement fixed by the Provider factory.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretServicePlacement {
    execution_ref: ResourceRef,
    user_ref: ResourceRef,
}

impl SecretServicePlacement {
    /// Validate user-agent placement on a Host or Guest execution context.
    pub fn new(
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
            execution_ref,
            user_ref,
        })
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

/// Factory fixed to one user placement and optional exact consumer.
pub struct SecretServiceCredentialProviderFactory {
    config: SecretServiceConfig,
    placement: SecretServicePlacement,
    consumer_ref: Option<ResourceRef>,
    port: Arc<dyn Oo7SecretServicePort>,
}

impl SecretServiceCredentialProviderFactory {
    /// Build a factory. A present consumer must be a Provider reference.
    pub fn new(
        config: SecretServiceConfig,
        placement: SecretServicePlacement,
        consumer_ref: Option<ResourceRef>,
        port: Arc<dyn Oo7SecretServicePort>,
    ) -> Result<Self, SecretServiceProviderError> {
        if consumer_ref
            .as_ref()
            .is_some_and(|reference| reference.resource_type().as_str() != "Provider")
        {
            return Err(SecretServiceProviderError::InvalidConsumer);
        }
        Ok(Self {
            config,
            placement,
            consumer_ref,
            port,
        })
    }

    /// Construct the service Provider.
    pub fn construct(self) -> SecretServiceCredentialProvider {
        SecretServiceCredentialProvider {
            config: self.config,
            placement: self.placement,
            consumer_ref: self.consumer_ref,
            port: self.port,
            leases: Mutex::new(BTreeMap::new()),
            mutation_gate: Mutex::new(()),
        }
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
    consumer_ref: Option<ResourceRef>,
    port: Arc<dyn Oo7SecretServicePort>,
    leases: Mutex<BTreeMap<String, LeaseRecord>>,
    mutation_gate: Mutex<()>,
}

impl SecretServiceCredentialProvider {
    /// Return the fixed owner classification.
    pub const fn owner(&self) -> SecretServiceOwner {
        SecretServiceOwner::Userd
    }

    /// Borrow the optional exact consumer expected by authenticated admission.
    pub const fn consumer_ref(&self) -> Option<&ResourceRef> {
        self.consumer_ref.as_ref()
    }

    /// Borrow the fixed placement.
    pub const fn placement(&self) -> &SecretServicePlacement {
        &self.placement
    }

    /// Borrow validated configuration.
    pub const fn config(&self) -> &SecretServiceConfig {
        &self.config
    }

    pub(crate) fn map_port_error(error: SecretServicePortError) -> CredentialServiceError {
        let code = match error {
            SecretServicePortError::Locked | SecretServicePortError::Unavailable => {
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
            SecretServicePlacement::new(PlacementBinding::UserAgent, host.clone(), user.clone())
                .is_ok()
        );
        assert_eq!(
            SecretServicePlacement::new(PlacementBinding::HostSystem, host, user),
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
}
