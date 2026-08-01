//! Server half with admission before Provider dispatch.

use crate::{
    CredentialAuthorization, CredentialMethod, CredentialProvider, CredentialRequest,
    CredentialResponse, CredentialServiceError, CredentialTransport, dispatch_authorized_provider,
};

/// Trusted admission boundary implemented by an authenticated bus adapter.
pub trait CredentialAdmission: Send + Sync {
    /// Admit the exact method and derive its binding from authenticated evidence.
    fn authorize(
        &self,
        method: CredentialMethod,
        request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError>;
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
        let authorization = self.admission.authorize(method, &request)?;
        dispatch_authorized_provider(&self.provider, method, &request, &authorization)
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
