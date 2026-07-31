//! Entra Credential Provider backed by an injected identity-Guest client.
//!
//! The Provider retains no token, login cookie, machine credential, or browser
//! state. Production clients terminate at the configured Entrablau Endpoint;
//! there is no Host login, ambient credential chain, or direct Entra fallback.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod controller;
mod service;

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Wake, Waker};
use std::thread::{self, Thread};

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::{
    CredentialLeaseHandle, CredentialLeaseState, CredentialSourceVersion, OpaqueAzureRef,
    PlacementBinding,
};
use d2b_credential_service::{
    CredentialMetadata, CredentialOutcomeCode, CredentialServiceError, CredentialServiceErrorCode,
};

pub use controller::{EntraController, EntraEndpointPolicy, EntraStatusProjection};

/// Canonical Provider reference.
pub const PROVIDER_REF: &str = "Provider/credential-entra";
/// Canonical identity-Guest login Endpoint purpose.
pub const LOGIN_ENDPOINT_PURPOSE: &str = "credential-entra.d2bus.org/entra-login-token";
/// Maximum active leases per Provider instance.
pub const MAX_LOCAL_LEASES: u32 = 256;

/// Boxed asynchronous result returned by the injected identity-Guest client.
pub type EntraFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, EntraClientError>> + Send + 'a>>;

/// Exact-consumer ownership policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntraCredentialOwner {
    /// Only the configured consumer may be admitted.
    ExactConsumer,
}

/// Closed client state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntraClientState {
    /// Login state can issue leases.
    Ready,
    /// User interaction is required inside the identity Guest.
    InteractionRequired,
}

/// Closed identity-Guest client failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntraClientError {
    /// Login interaction is required.
    InteractionRequired,
    /// Policy denied the operation.
    Denied,
    /// The identity Guest or Endpoint is unavailable.
    Unavailable,
    /// The Endpoint generation differs from the admitted generation.
    GenerationMismatch,
    /// The lease expired.
    LeaseExpired,
    /// The lease was revoked.
    LeaseRevoked,
    /// Completion is ambiguous and must not be replayed automatically.
    CompletionUnknown,
}

impl fmt::Display for EntraClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InteractionRequired | Self::Unavailable => "credential-provider-unavailable",
            Self::Denied => "credential-operation-denied",
            Self::GenerationMismatch | Self::CompletionUnknown => "credential-invariant-failure",
            Self::LeaseExpired => "credential-lease-expired",
            Self::LeaseRevoked => "credential-lease-revoked",
        })
    }
}

impl std::error::Error for EntraClientError {}

/// Validated non-secret Provider configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct EntraConfig {
    tenant_id: OpaqueAzureRef,
    max_leases: u32,
}

impl EntraConfig {
    /// Validate the inline tenant identifier and lease bound.
    pub fn new(tenant_id: impl Into<String>, max_leases: u32) -> Result<Self, EntraProviderError> {
        let tenant_id = OpaqueAzureRef::parse(tenant_id.into())
            .map_err(|_| EntraProviderError::InvalidConfig)?;
        if !(1..=MAX_LOCAL_LEASES).contains(&max_leases) {
            return Err(EntraProviderError::InvalidConfig);
        }
        Ok(Self {
            tenant_id,
            max_leases,
        })
    }

    /// Borrow the validated tenant ID for the injected Endpoint client.
    pub const fn tenant_id(&self) -> &OpaqueAzureRef {
        &self.tenant_id
    }

    /// Return the active-lease ceiling.
    pub const fn max_leases(&self) -> u32 {
        self.max_leases
    }
}

impl fmt::Debug for EntraConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("EntraConfig")
            .field("tenant_id", &"<redacted>")
            .field("max_leases", &self.max_leases)
            .finish()
    }
}

/// Closed construction failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntraProviderError {
    /// Configuration is invalid.
    InvalidConfig,
    /// Host-system or non-Guest placement was requested.
    InvalidPlacement,
    /// A required identity Guest or login Endpoint reference is invalid.
    InvalidEndpoint,
    /// The exact consumer is not a Provider reference.
    InvalidConsumer,
}

impl fmt::Display for EntraProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfig => "credential schema is invalid",
            Self::InvalidPlacement => "credential placement mismatch",
            Self::InvalidEndpoint => "credential endpoint is invalid",
            Self::InvalidConsumer => "credential consumer mismatch",
        })
    }
}

impl std::error::Error for EntraProviderError {}

/// Identity-Guest placement and Endpoint binding.
#[derive(Clone, PartialEq, Eq)]
pub struct EntraPlacement {
    binding: PlacementBinding,
    execution_ref: ResourceRef,
    identity_guest_ref: ResourceRef,
    login_endpoint_ref: ResourceRef,
    endpoint_generation: u64,
}

impl EntraPlacement {
    /// Validate user-agent or guest-agent placement inside a Guest.
    pub fn new(
        binding: PlacementBinding,
        execution_ref: ResourceRef,
        identity_guest_ref: ResourceRef,
        login_endpoint_ref: ResourceRef,
        endpoint_generation: u64,
    ) -> Result<Self, EntraProviderError> {
        if !matches!(
            binding,
            PlacementBinding::UserAgent | PlacementBinding::GuestAgent
        ) || execution_ref.resource_type().as_str() != "Guest"
        {
            return Err(EntraProviderError::InvalidPlacement);
        }
        if identity_guest_ref.resource_type().as_str() != "Guest"
            || login_endpoint_ref.resource_type().as_str() != "Endpoint"
            || endpoint_generation == 0
        {
            return Err(EntraProviderError::InvalidEndpoint);
        }
        Ok(Self {
            binding,
            execution_ref,
            identity_guest_ref,
            login_endpoint_ref,
            endpoint_generation,
        })
    }

    /// Return the placement binding.
    pub const fn binding(&self) -> PlacementBinding {
        self.binding
    }

    /// Borrow the consumer execution Guest.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the identity Guest.
    pub const fn identity_guest_ref(&self) -> &ResourceRef {
        &self.identity_guest_ref
    }

    /// Borrow the login Endpoint.
    pub const fn login_endpoint_ref(&self) -> &ResourceRef {
        &self.login_endpoint_ref
    }

    /// Return the admitted Endpoint generation.
    pub const fn endpoint_generation(&self) -> u64 {
        self.endpoint_generation
    }
}

impl fmt::Debug for EntraPlacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EntraPlacement(<redacted>)")
    }
}

/// Opaque lease request passed to the identity-Guest client.
#[derive(Clone, PartialEq, Eq)]
pub struct EntraLeaseRequest {
    credential_ref: ResourceRef,
    operation_id: String,
    idempotency_key: String,
    requested_expiry_unix_ms: u64,
    endpoint_generation: u64,
}

impl EntraLeaseRequest {
    /// Borrow the routed Credential reference.
    pub const fn credential_ref(&self) -> &ResourceRef {
        &self.credential_ref
    }

    /// Borrow the operation identifier.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// Borrow the idempotency key.
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    /// Return requested expiry.
    pub const fn requested_expiry_unix_ms(&self) -> u64 {
        self.requested_expiry_unix_ms
    }

    /// Return the admitted Endpoint generation.
    pub const fn endpoint_generation(&self) -> u64 {
        self.endpoint_generation
    }
}

impl fmt::Debug for EntraLeaseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EntraLeaseRequest(<redacted>)")
    }
}

/// Opaque lease reference for inspect, refresh, and revoke.
#[derive(Clone, PartialEq, Eq)]
pub struct EntraLeaseRef {
    credential_ref: ResourceRef,
    metadata: CredentialMetadata,
    endpoint_generation: u64,
}

impl EntraLeaseRef {
    /// Borrow the routed Credential reference.
    pub const fn credential_ref(&self) -> &ResourceRef {
        &self.credential_ref
    }

    /// Borrow current metadata.
    pub const fn metadata(&self) -> &CredentialMetadata {
        &self.metadata
    }

    /// Return the admitted Endpoint generation.
    pub const fn endpoint_generation(&self) -> u64 {
        self.endpoint_generation
    }
}

impl fmt::Debug for EntraLeaseRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EntraLeaseRef(<redacted>)")
    }
}

/// Non-secret lease grant.
#[derive(Clone, PartialEq, Eq)]
pub struct EntraLeaseGrant {
    /// Opaque lease handle.
    pub lease_handle: CredentialLeaseHandle,
    /// Opaque source version.
    pub source_version: CredentialSourceVersion,
    /// Rotation generation.
    pub rotation_generation: u64,
    /// Absolute expiry.
    pub expires_at_unix_ms: u64,
}

impl fmt::Debug for EntraLeaseGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EntraLeaseGrant(<redacted>)")
    }
}

/// Non-secret lease inspection.
#[derive(Clone, PartialEq, Eq)]
pub struct EntraLeaseInspection {
    /// Closed lease state.
    pub state: CredentialLeaseState,
    /// Opaque source version.
    pub source_version: CredentialSourceVersion,
    /// Rotation generation.
    pub rotation_generation: u64,
    /// Absolute expiry.
    pub expires_at_unix_ms: u64,
}

impl fmt::Debug for EntraLeaseInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EntraLeaseInspection(<redacted>)")
    }
}

/// Non-secret lease renewal.
pub type EntraLeaseRenewal = EntraLeaseGrant;

/// Idempotent revoke result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntraLeaseRevocation {
    /// This call revoked the lease.
    Revoked,
    /// The lease was already revoked.
    AlreadyRevoked,
}

/// Injected identity-Guest client retaining all token and login material.
pub trait EntraCredentialClient: Send + Sync {
    /// Observe client readiness.
    fn state(&self) -> EntraFuture<'_, EntraClientState>;
    /// Issue one lease.
    fn issue_lease(&self, request: &EntraLeaseRequest) -> EntraFuture<'_, EntraLeaseGrant>;
    /// Inspect one lease.
    fn inspect_lease(&self, lease: &EntraLeaseRef) -> EntraFuture<'_, EntraLeaseInspection>;
    /// Refresh one lease.
    fn refresh_lease(&self, lease: &EntraLeaseRef) -> EntraFuture<'_, EntraLeaseRenewal>;
    /// Revoke one lease.
    fn revoke_lease(&self, lease: &EntraLeaseRef) -> EntraFuture<'_, EntraLeaseRevocation>;
}

/// Factory bound to one exact consumer and identity-Guest Endpoint.
pub struct EntraCredentialProviderFactory {
    config: EntraConfig,
    placement: EntraPlacement,
    consumer_ref: ResourceRef,
    client: Arc<dyn EntraCredentialClient>,
}

impl EntraCredentialProviderFactory {
    /// Validate and construct a factory.
    pub fn new(
        config: EntraConfig,
        placement: EntraPlacement,
        consumer_ref: ResourceRef,
        client: Arc<dyn EntraCredentialClient>,
    ) -> Result<Self, EntraProviderError> {
        if consumer_ref.resource_type().as_str() != "Provider" {
            return Err(EntraProviderError::InvalidConsumer);
        }
        Ok(Self {
            config,
            placement,
            consumer_ref,
            client,
        })
    }

    /// Construct the service Provider.
    pub fn construct(self) -> EntraCredentialProvider {
        EntraCredentialProvider {
            config: self.config,
            placement: self.placement,
            consumer_ref: self.consumer_ref,
            client: self.client,
            leases: Mutex::new(BTreeMap::new()),
        }
    }
}

impl fmt::Debug for EntraCredentialProviderFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EntraCredentialProviderFactory(<redacted>)")
    }
}

#[derive(Clone)]
struct LeaseRecord {
    idempotency_key: String,
    metadata: CredentialMetadata,
}

/// Entra implementation of the prepared Credential service.
pub struct EntraCredentialProvider {
    config: EntraConfig,
    placement: EntraPlacement,
    consumer_ref: ResourceRef,
    client: Arc<dyn EntraCredentialClient>,
    leases: Mutex<BTreeMap<String, LeaseRecord>>,
}

impl EntraCredentialProvider {
    /// Return exact-consumer ownership.
    pub const fn owner(&self) -> EntraCredentialOwner {
        EntraCredentialOwner::ExactConsumer
    }

    /// Borrow the exact consumer required at authenticated admission.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Test an authenticated Provider reference against the exact consumer.
    pub fn authorizes_consumer(&self, authenticated_provider_ref: &ResourceRef) -> bool {
        authenticated_provider_ref == &self.consumer_ref
    }

    /// Borrow the identity-Guest placement.
    pub const fn placement(&self) -> &EntraPlacement {
        &self.placement
    }

    /// Borrow validated configuration.
    pub const fn config(&self) -> &EntraConfig {
        &self.config
    }

    /// Reject a stale observed Endpoint generation.
    pub fn validate_endpoint_generation(
        &self,
        observed_generation: u64,
    ) -> Result<(), CredentialServiceError> {
        if observed_generation == self.placement.endpoint_generation() {
            Ok(())
        } else {
            Err(CredentialServiceError::new(
                CredentialServiceErrorCode::InvariantFailure,
            ))
        }
    }

    pub(crate) fn poll_client<T>(
        mut future: EntraFuture<'_, T>,
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
            match future.as_mut().poll(&mut context) {
                Poll::Ready(result) => return result.map_err(Self::map_client_error),
                Poll::Pending => thread::park(),
            }
        }
    }

    pub(crate) fn map_client_error(error: EntraClientError) -> CredentialServiceError {
        let code = match error {
            EntraClientError::InteractionRequired | EntraClientError::Unavailable => {
                CredentialServiceErrorCode::ProviderUnavailable
            }
            EntraClientError::Denied => CredentialServiceErrorCode::OperationDenied,
            EntraClientError::LeaseExpired => CredentialServiceErrorCode::LeaseExpired,
            EntraClientError::LeaseRevoked => CredentialServiceErrorCode::LeaseRevoked,
            EntraClientError::GenerationMismatch | EntraClientError::CompletionUnknown => {
                CredentialServiceErrorCode::InvariantFailure
            }
        };
        CredentialServiceError::new(code)
    }

    pub(crate) fn grant_metadata(
        grant: EntraLeaseGrant,
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

impl fmt::Debug for EntraCredentialProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EntraCredentialProvider(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_id_reuses_opaque_cloud_reference_validation() {
        assert!(EntraConfig::new("tenant-1234", 64).is_ok());
        assert!(EntraConfig::new("SharedAccessKey=abc/def+ghi==", 64).is_err());
    }

    #[test]
    fn exact_consumer_guard_is_independent_of_request_fields() {
        let expected = ResourceRef::parse("Provider/runtime-azure-container-apps").unwrap();
        let other = ResourceRef::parse("Provider/other").unwrap();
        assert_ne!(expected, other);
    }

    #[test]
    fn host_system_placement_is_rejected() {
        assert_eq!(
            EntraPlacement::new(
                PlacementBinding::HostSystem,
                ResourceRef::parse("Host/workstation").unwrap(),
                ResourceRef::parse("Guest/identity").unwrap(),
                ResourceRef::parse("Endpoint/entra-login").unwrap(),
                1,
            ),
            Err(EntraProviderError::InvalidPlacement)
        );
    }
}
