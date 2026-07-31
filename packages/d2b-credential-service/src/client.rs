//! Client half of the unregistered Credential service contract.

use crate::service::{
    CredentialMethod, CredentialRequest, CredentialResponse, CredentialServiceError,
};

/// Transport boundary used by a future authenticated bus adapter.
pub trait CredentialTransport: Send + Sync {
    /// Dispatch one strictly typed method and request.
    fn call(
        &self,
        method: CredentialMethod,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError>;
}

/// Typed client that never accepts a caller-selected operation class.
pub struct CredentialClient<T> {
    transport: T,
}

impl<T> CredentialClient<T>
where
    T: CredentialTransport,
{
    /// Bind a client to one injected transport.
    pub const fn new(transport: T) -> Self {
        Self { transport }
    }

    /// Invoke `AcquireToken`.
    pub fn acquire_token(
        &self,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        self.transport.call(CredentialMethod::AcquireToken, request)
    }

    /// Invoke `RefreshToken`.
    pub fn refresh_token(
        &self,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        self.transport.call(CredentialMethod::RefreshToken, request)
    }

    /// Invoke `RevokeToken`.
    pub fn revoke_token(
        &self,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        self.transport.call(CredentialMethod::RevokeToken, request)
    }

    /// Invoke `SignChallenge`.
    pub fn sign_challenge(
        &self,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        self.transport
            .call(CredentialMethod::SignChallenge, request)
    }

    /// Invoke `InspectMetadata`.
    pub fn inspect_metadata(
        &self,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        self.transport
            .call(CredentialMethod::InspectMetadata, request)
    }
}

impl<T> core::fmt::Debug for CredentialClient<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("CredentialClient(<redacted>)")
    }
}
