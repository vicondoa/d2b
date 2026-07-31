//! Co-located managed identity agent service.

use d2b_credential_service::{
    CredentialAuthorization, CredentialMethod, CredentialProvider, CredentialRequest,
    CredentialResponse, CredentialServiceError,
};

use crate::ManagedIdentityCredentialProvider;

/// Agent role that exclusively owns the injected IMDS client and sensitive
/// delivery path.
pub struct ManagedIdentityAgent {
    provider: ManagedIdentityCredentialProvider,
}

impl ManagedIdentityAgent {
    /// Bind a validated Provider implementation to the agent role.
    pub const fn new(provider: ManagedIdentityCredentialProvider) -> Self {
        Self { provider }
    }

    /// Borrow the configured exact consumer policy.
    pub const fn provider(&self) -> &ManagedIdentityCredentialProvider {
        &self.provider
    }
}

impl CredentialProvider for ManagedIdentityAgent {
    fn dispatch(
        &self,
        method: CredentialMethod,
        request: &CredentialRequest,
        authorization: &CredentialAuthorization,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        self.provider.dispatch(method, request, authorization)
    }
}

impl core::fmt::Debug for ManagedIdentityAgent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("ManagedIdentityAgent(<redacted>)")
    }
}
