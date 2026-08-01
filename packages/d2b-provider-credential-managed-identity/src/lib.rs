//! Managed identity Credential Provider for an exact SDK consumer.
//!
//! The injected client owns IMDS access and all token bytes. This crate has no
//! ambient credential chain, environment fallback, endpoint URL input, or
//! developer-tool fallback.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod agent;
mod audit;
mod controller;
mod service;
mod telemetry;

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
    CredentialServiceError, CredentialServiceErrorCode, CredentialSourceVersion, OpaqueAzureRef,
    PlacementBinding,
};

pub use agent::ManagedIdentityAgent;
pub use audit::{
    ManagedIdentityAuditError, ManagedIdentityAuditOperation, ManagedIdentityAuditOutcome,
    ManagedIdentityAuditRecord,
};
pub use controller::{
    AgentProcessSpec, ManagedIdentityController, ManagedIdentityRoute,
    ManagedIdentityStatusProjection, ManagedIdentityTeardownPlan,
};
pub use telemetry::{
    ManagedIdentityTelemetryFrame, ManagedIdentityTelemetryOperation,
    ManagedIdentityTelemetryOutcome, TelemetryField, TelemetryFrameError,
};

/// Canonical Provider reference.
pub const PROVIDER_REF: &str = "Provider/credential-managed-identity";
/// Maximum active leases per Provider instance.
pub const MAX_LOCAL_LEASES: u32 = 256;
/// Secret-free controller binary declared by the Provider dossier.
pub const CONTROLLER_BINARY: &str = "d2b-managed-identity-controller";
/// Co-located client-holding agent binary declared by the Provider dossier.
pub const AGENT_BINARY: &str = "d2b-managed-identity-agent";
/// Exit status used while production Zone runtime registration is unavailable.
pub const RUNTIME_UNAVAILABLE_EXIT: i32 = 78;

/// Enter the secret-free controller role.
///
/// Production registration is deliberately fail-closed until the authenticated
/// Zone runtime supplies the controller adapter.
pub const fn controller_binary_entrypoint() -> i32 {
    RUNTIME_UNAVAILABLE_EXIT
}

/// Enter the client-holding agent role.
///
/// Production registration is deliberately fail-closed until an authenticated
/// LaunchTicket supplies the explicit effect-port client.
pub const fn agent_binary_entrypoint() -> i32 {
    RUNTIME_UNAVAILABLE_EXIT
}

/// Boxed asynchronous result returned by the injected IMDS client.
pub type ManagedIdentityFuture<'a, T> =
    Pin<Box<dyn Future<Output = Result<T, ManagedIdentityClientError>> + Send + 'a>>;

/// Exact-consumer ownership policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityCredentialOwner {
    /// Only the authenticated configured SDK consumer may be admitted.
    ExactSdkConsumer,
}

/// Closed IMDS endpoint categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImdsEndpointAlias {
    /// Standard Azure Instance Metadata Service.
    AzureImds,
    /// Azure Container Apps sidecar metadata service.
    AzureImdsAca,
}

impl ImdsEndpointAlias {
    /// Parse a closed alias without accepting a URL or path.
    pub fn parse(value: &str) -> Result<Self, ManagedIdentityProviderError> {
        match value {
            "azure-imds" => Ok(Self::AzureImds),
            "azure-imds-aca" => Ok(Self::AzureImdsAca),
            _ => Err(ManagedIdentityProviderError::InvalidConfig),
        }
    }

    /// Return the stable alias.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AzureImds => "azure-imds",
            Self::AzureImdsAca => "azure-imds-aca",
        }
    }
}

/// Closed injected-client state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityClientState {
    /// IMDS can issue leases.
    Ready,
    /// IMDS is unavailable.
    Unavailable,
}

/// Closed client failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityClientError {
    /// Policy denied the operation.
    Denied,
    /// IMDS is unavailable.
    Unavailable,
    /// The lease expired.
    LeaseExpired,
    /// The lease was revoked.
    LeaseRevoked,
    /// Completion is ambiguous and must not be replayed automatically.
    CompletionUnknown,
}

impl fmt::Display for ManagedIdentityClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Denied => "credential-operation-denied",
            Self::Unavailable => "credential-provider-unavailable",
            Self::LeaseExpired => "credential-lease-expired",
            Self::LeaseRevoked => "credential-lease-revoked",
            Self::CompletionUnknown => "credential-invariant-failure",
        })
    }
}

impl std::error::Error for ManagedIdentityClientError {}

/// Validated non-secret client configuration.
#[derive(Clone, PartialEq, Eq)]
pub struct ManagedIdentityClientConfig {
    client_id: OpaqueAzureRef,
    endpoint_alias: ImdsEndpointAlias,
    max_leases: u32,
}

impl ManagedIdentityClientConfig {
    /// Validate the inline client ID, closed alias, and lease ceiling.
    pub fn new(
        client_id: impl Into<String>,
        endpoint_alias: &str,
        max_leases: u32,
    ) -> Result<Self, ManagedIdentityProviderError> {
        let client_id = OpaqueAzureRef::parse(client_id.into())
            .map_err(|_| ManagedIdentityProviderError::InvalidConfig)?;
        let endpoint_alias = ImdsEndpointAlias::parse(endpoint_alias)?;
        if !(1..=MAX_LOCAL_LEASES).contains(&max_leases) {
            return Err(ManagedIdentityProviderError::InvalidConfig);
        }
        Ok(Self {
            client_id,
            endpoint_alias,
            max_leases,
        })
    }

    /// Borrow the validated client ID for the injected client.
    pub const fn client_id(&self) -> &OpaqueAzureRef {
        &self.client_id
    }

    /// Return the closed endpoint category.
    pub const fn endpoint_alias(&self) -> ImdsEndpointAlias {
        self.endpoint_alias
    }

    /// Return the active-lease ceiling.
    pub const fn max_leases(&self) -> u32 {
        self.max_leases
    }
}

impl fmt::Debug for ManagedIdentityClientConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ManagedIdentityClientConfig")
            .field("client_id", &"<redacted>")
            .field("endpoint_alias", &self.endpoint_alias)
            .field("max_leases", &self.max_leases)
            .finish()
    }
}

/// Closed construction failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityProviderError {
    /// Configuration is invalid.
    InvalidConfig,
    /// User-agent or incompatible machine placement was requested.
    InvalidPlacement,
    /// The exact consumer is not a Provider reference.
    InvalidConsumer,
}

impl fmt::Display for ManagedIdentityProviderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidConfig => "credential schema is invalid",
            Self::InvalidPlacement => "credential placement mismatch",
            Self::InvalidConsumer => "credential consumer mismatch",
        })
    }
}

impl std::error::Error for ManagedIdentityProviderError {}

/// Machine-local Host or Guest placement.
#[derive(Clone, PartialEq, Eq)]
pub struct ManagedIdentityPlacement {
    binding: PlacementBinding,
    execution_ref: ResourceRef,
}

impl ManagedIdentityPlacement {
    /// Validate host-system or guest-agent placement.
    pub fn new(
        binding: PlacementBinding,
        execution_ref: ResourceRef,
    ) -> Result<Self, ManagedIdentityProviderError> {
        let valid = matches!(
            (binding, execution_ref.resource_type().as_str()),
            (PlacementBinding::HostSystem, "Host") | (PlacementBinding::GuestAgent, "Guest")
        );
        if !valid {
            return Err(ManagedIdentityProviderError::InvalidPlacement);
        }
        Ok(Self {
            binding,
            execution_ref,
        })
    }

    /// Return the placement binding.
    pub const fn binding(&self) -> PlacementBinding {
        self.binding
    }

    /// Borrow the execution context.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }
}

impl fmt::Debug for ManagedIdentityPlacement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ManagedIdentityPlacement(<redacted>)")
    }
}

/// Opaque acquire request passed to the injected client.
#[derive(Clone, PartialEq, Eq)]
pub struct ManagedIdentityLeaseRequest {
    credential_ref: ResourceRef,
    operation_id: String,
    idempotency_key: String,
    requested_expiry_unix_ms: u64,
}

impl ManagedIdentityLeaseRequest {
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
}

impl fmt::Debug for ManagedIdentityLeaseRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ManagedIdentityLeaseRequest(<redacted>)")
    }
}

/// Opaque lease reference for inspect, refresh, and revoke.
#[derive(Clone, PartialEq, Eq)]
pub struct ManagedIdentityLeaseRef {
    credential_ref: ResourceRef,
    metadata: CredentialMetadata,
}

impl ManagedIdentityLeaseRef {
    /// Borrow the routed Credential reference.
    pub const fn credential_ref(&self) -> &ResourceRef {
        &self.credential_ref
    }

    /// Borrow current metadata.
    pub const fn metadata(&self) -> &CredentialMetadata {
        &self.metadata
    }
}

impl fmt::Debug for ManagedIdentityLeaseRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ManagedIdentityLeaseRef(<redacted>)")
    }
}

/// Non-secret lease grant.
#[derive(Clone, PartialEq, Eq)]
pub struct ManagedIdentityLeaseGrant {
    /// Opaque lease handle.
    pub lease_handle: CredentialLeaseHandle,
    /// Opaque source version.
    pub source_version: CredentialSourceVersion,
    /// Rotation generation.
    pub rotation_generation: u64,
    /// Absolute expiry.
    pub expires_at_unix_ms: u64,
}

impl fmt::Debug for ManagedIdentityLeaseGrant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ManagedIdentityLeaseGrant(<redacted>)")
    }
}

/// Non-secret lease inspection.
#[derive(Clone, PartialEq, Eq)]
pub struct ManagedIdentityLeaseInspection {
    /// Closed lease state.
    pub state: CredentialLeaseState,
    /// Opaque source version.
    pub source_version: CredentialSourceVersion,
    /// Rotation generation.
    pub rotation_generation: u64,
    /// Absolute expiry.
    pub expires_at_unix_ms: u64,
}

impl fmt::Debug for ManagedIdentityLeaseInspection {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ManagedIdentityLeaseInspection(<redacted>)")
    }
}

/// Non-secret lease renewal.
pub type ManagedIdentityLeaseRenewal = ManagedIdentityLeaseGrant;

/// Idempotent revoke result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManagedIdentityLeaseRevocation {
    /// This call marked the lease revoked.
    Revoked,
    /// The lease was already revoked.
    AlreadyRevoked,
}

/// Injected client that owns IMDS access and token bytes.
pub trait ManagedIdentityCredentialClient: Send + Sync {
    /// Observe IMDS readiness.
    fn state(&self) -> ManagedIdentityFuture<'_, ManagedIdentityClientState>;
    /// Issue one lease.
    fn issue_lease(
        &self,
        request: &ManagedIdentityLeaseRequest,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseGrant>;
    /// Inspect one lease.
    fn inspect_lease(
        &self,
        lease: &ManagedIdentityLeaseRef,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseInspection>;
    /// Refresh one lease.
    fn refresh_lease(
        &self,
        lease: &ManagedIdentityLeaseRef,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseRenewal>;
    /// Revoke one lease locally.
    fn revoke_lease(
        &self,
        lease: &ManagedIdentityLeaseRef,
    ) -> ManagedIdentityFuture<'_, ManagedIdentityLeaseRevocation>;
}

/// Factory bound to one machine placement and exact SDK consumer.
pub struct ManagedIdentityCredentialProviderFactory {
    config: ManagedIdentityClientConfig,
    placement: ManagedIdentityPlacement,
    consumer_ref: ResourceRef,
    client: Arc<dyn ManagedIdentityCredentialClient>,
}

impl ManagedIdentityCredentialProviderFactory {
    /// Validate and construct the factory.
    pub fn new(
        config: ManagedIdentityClientConfig,
        placement: ManagedIdentityPlacement,
        consumer_ref: ResourceRef,
        client: Arc<dyn ManagedIdentityCredentialClient>,
    ) -> Result<Self, ManagedIdentityProviderError> {
        if consumer_ref.resource_type().as_str() != "Provider" {
            return Err(ManagedIdentityProviderError::InvalidConsumer);
        }
        Ok(Self {
            config,
            placement,
            consumer_ref,
            client,
        })
    }

    /// Construct the service Provider.
    pub fn construct(self) -> ManagedIdentityCredentialProvider {
        ManagedIdentityCredentialProvider {
            config: self.config,
            placement: self.placement,
            consumer_ref: self.consumer_ref,
            client: self.client,
            leases: Mutex::new(BTreeMap::new()),
            mutation_gate: Mutex::new(()),
        }
    }
}

impl fmt::Debug for ManagedIdentityCredentialProviderFactory {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ManagedIdentityCredentialProviderFactory(<redacted>)")
    }
}

#[derive(Clone)]
struct LeaseRecord {
    idempotency_key: String,
    metadata: CredentialMetadata,
}

/// Managed identity implementation of the prepared Credential service.
pub struct ManagedIdentityCredentialProvider {
    config: ManagedIdentityClientConfig,
    placement: ManagedIdentityPlacement,
    consumer_ref: ResourceRef,
    client: Arc<dyn ManagedIdentityCredentialClient>,
    leases: Mutex<BTreeMap<String, LeaseRecord>>,
    mutation_gate: Mutex<()>,
}

impl ManagedIdentityCredentialProvider {
    /// Return exact SDK-consumer ownership.
    pub const fn owner(&self) -> ManagedIdentityCredentialOwner {
        ManagedIdentityCredentialOwner::ExactSdkConsumer
    }

    /// Borrow the exact consumer required at authenticated admission.
    pub const fn consumer_ref(&self) -> &ResourceRef {
        &self.consumer_ref
    }

    /// Check an authenticated Provider identity against the exact consumer.
    pub fn authorizes_consumer(&self, authenticated_provider_ref: &ResourceRef) -> bool {
        authenticated_provider_ref == &self.consumer_ref
    }

    /// Borrow machine placement.
    pub const fn placement(&self) -> &ManagedIdentityPlacement {
        &self.placement
    }

    /// Borrow validated client configuration.
    pub const fn config(&self) -> &ManagedIdentityClientConfig {
        &self.config
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

    pub(crate) fn poll_client<T>(
        mut future: ManagedIdentityFuture<'_, T>,
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
                Poll::Ready(result) => return result.map_err(Self::map_client_error),
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

    pub(crate) fn map_client_error(error: ManagedIdentityClientError) -> CredentialServiceError {
        let code = match error {
            ManagedIdentityClientError::Denied => CredentialServiceErrorCode::OperationDenied,
            ManagedIdentityClientError::Unavailable => {
                CredentialServiceErrorCode::ProviderUnavailable
            }
            ManagedIdentityClientError::LeaseExpired => CredentialServiceErrorCode::LeaseExpired,
            ManagedIdentityClientError::LeaseRevoked => CredentialServiceErrorCode::LeaseRevoked,
            ManagedIdentityClientError::CompletionUnknown => {
                CredentialServiceErrorCode::InvariantFailure
            }
        };
        CredentialServiceError::new(code)
    }

    pub(crate) fn grant_metadata(
        grant: ManagedIdentityLeaseGrant,
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

impl fmt::Debug for ManagedIdentityCredentialProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ManagedIdentityCredentialProvider(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_and_alias_validation_fail_closed() {
        assert!(ManagedIdentityClientConfig::new("client-1234", "azure-imds", 64).is_ok());
        assert!(
            ManagedIdentityClientConfig::new("SharedAccessKey=abc/def+ghi==", "azure-imds", 64,)
                .is_err()
        );
        assert!(ManagedIdentityClientConfig::new("client-1234", "http://imds", 64).is_err());
    }

    #[test]
    fn user_agent_placement_is_rejected() {
        assert_eq!(
            ManagedIdentityPlacement::new(
                PlacementBinding::UserAgent,
                ResourceRef::parse("Host/workstation").unwrap(),
            ),
            Err(ManagedIdentityProviderError::InvalidPlacement)
        );
    }

    #[test]
    fn client_id_is_redacted_from_debug() {
        let marker = format!("client-canary-{:x}", std::process::id());
        let config = ManagedIdentityClientConfig::new(&marker, "azure-imds", 64).unwrap();
        assert!(!format!("{config:?}").contains(&marker));
        assert_eq!(config.client_id().as_str(), marker);
    }
}
