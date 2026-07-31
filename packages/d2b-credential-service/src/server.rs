//! Server half with admission before Provider dispatch.

use crate::{
    CredentialMethod, CredentialRequest, CredentialResponse, CredentialServiceError,
    CredentialServiceErrorCode, CredentialTransport, DeliverySessionParams,
};

/// Authorization result derived by the authenticated bus adapter.
///
/// For sensitive-output methods this carries the complete delivery binding
/// constructed during route authorization. The Provider may use it but cannot
/// replace any of its authority-bearing fields.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialAuthorization {
    delivery_session_params: Option<DeliverySessionParams>,
}

impl CredentialAuthorization {
    /// Construct an authorization result for one exact method.
    pub fn new(
        method: CredentialMethod,
        delivery_session_params: Option<DeliverySessionParams>,
    ) -> Result<Self, CredentialServiceError> {
        if delivery_session_params.is_some() != method.requires_delivery()
            || delivery_session_params
                .as_ref()
                .is_some_and(|params| params.operation_class() != method.operation_class())
        {
            return Err(CredentialServiceError::new(
                CredentialServiceErrorCode::InvariantFailure,
            ));
        }
        Ok(Self {
            delivery_session_params,
        })
    }

    /// Borrow the bus-authorized delivery binding, when the method needs one.
    pub const fn delivery_session_params(&self) -> Option<&DeliverySessionParams> {
        self.delivery_session_params.as_ref()
    }
}

impl core::fmt::Debug for CredentialAuthorization {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("CredentialAuthorization(<redacted>)")
    }
}

/// Trusted admission boundary implemented by an authenticated bus adapter.
pub trait CredentialAdmission: Send + Sync {
    /// Admit the exact method and derive its binding from authenticated evidence.
    fn authorize(
        &self,
        method: CredentialMethod,
        request: &CredentialRequest,
    ) -> Result<CredentialAuthorization, CredentialServiceError>;
}

/// Provider-side implementation of the five service methods.
pub trait CredentialProvider: Send + Sync {
    /// Dispatch one already-admitted exact method.
    fn dispatch(
        &self,
        method: CredentialMethod,
        request: &CredentialRequest,
        authorization: &CredentialAuthorization,
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
        let authorization = self.admission.authorize(method, &request)?;
        let response = self.provider.dispatch(method, &request, &authorization)?;
        if response.method() != method
            || response.delivery_session_params() != authorization.delivery_session_params()
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
