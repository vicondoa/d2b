//! Server half with admission before Provider dispatch.

use crate::{
    CredentialMethod, CredentialRequest, CredentialResponse, CredentialServiceError,
    CredentialServiceErrorCode, CredentialTransport,
};

/// Trusted admission boundary implemented by an authenticated bus adapter.
pub trait CredentialAdmission: Send + Sync {
    /// Admit the exact method-derived operation before Provider dispatch.
    fn authorize(
        &self,
        method: CredentialMethod,
        request: &CredentialRequest,
    ) -> Result<(), CredentialServiceError>;
}

/// Provider-side implementation of the five service methods.
pub trait CredentialProvider: Send + Sync {
    /// Dispatch one already-admitted exact method.
    fn dispatch(
        &self,
        method: CredentialMethod,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError>;
}

/// Unregistered server that fixes admission before Provider invocation.
pub struct CredentialServer<P, A> {
    provider: P,
    admission: A,
}

impl<P, A> CredentialServer<P, A>
where
    P: CredentialProvider,
    A: CredentialAdmission,
{
    /// Bind an injected Provider and trusted admission boundary.
    pub const fn new(provider: P, admission: A) -> Self {
        Self {
            provider,
            admission,
        }
    }

    fn dispatch(
        &self,
        method: CredentialMethod,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        self.admission.authorize(method, &request)?;
        let response = self.provider.dispatch(method, request)?;
        if response.method() != method
            || response.delivery_session_params().is_some() != method.requires_delivery()
            || response
                .delivery_session_params()
                .is_some_and(|params| params.operation_class() != method.operation_class())
        {
            return Err(CredentialServiceError::new(
                CredentialServiceErrorCode::InvariantFailure,
            ));
        }
        Ok(response)
    }
}

impl<P, A> CredentialTransport for CredentialServer<P, A>
where
    P: CredentialProvider,
    A: CredentialAdmission,
{
    fn call(
        &self,
        method: CredentialMethod,
        request: CredentialRequest,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        self.dispatch(method, request)
    }
}

impl<P, A> core::fmt::Debug for CredentialServer<P, A> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("CredentialServer(<redacted>)")
    }
}
