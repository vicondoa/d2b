//! Authenticated d2b-bus binding for resource clients and transports.

use std::sync::Arc;

use d2b_contracts::v3::AuthenticatedSubjectContext;
use d2b_resource_store::ResourceStore;

use crate::{
    authz::{AuthorizationState, authenticated_relay_hop},
    client::ResourceClient,
    service::{ResourceService, TrustedRequest, UpgradeDispatcher},
};

/// Failure to bind an authenticated ComponentSession route.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdapterBindingError;

impl core::fmt::Display for AdapterBindingError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("authenticated bus route is not valid for the resource API")
    }
}

impl std::error::Error for AdapterBindingError {}

/// Session-scoped adapter created only after d2b-bus authentication succeeds.
#[derive(Debug)]
pub struct AuthenticatedBusAdapter<S, U> {
    service: Arc<ResourceService<S, U>>,
    subject: Arc<AuthenticatedSubjectContext>,
    state: AuthorizationState,
}

impl<S, U> AuthenticatedBusAdapter<S, U>
where
    S: ResourceStore,
    U: UpgradeDispatcher,
{
    /// Seal authenticated identity and policy state to one ComponentSession.
    pub fn bind_authenticated_session(
        service: Arc<ResourceService<S, U>>,
        subject: Arc<AuthenticatedSubjectContext>,
        state: AuthorizationState,
    ) -> Result<Self, AdapterBindingError> {
        authenticated_relay_hop(&subject).map_err(|_| AdapterBindingError)?;
        Ok(Self {
            service,
            subject,
            state,
        })
    }

    /// Return an in-process client bound to the same authenticated session.
    pub fn client(&self) -> ResourceClient<S, U> {
        ResourceClient::from_authenticated_bus(
            Arc::clone(&self.service),
            Arc::clone(&self.subject),
            self.state.clone(),
        )
    }

    pub(crate) fn service(&self) -> &ResourceService<S, U> {
        &self.service
    }

    pub(crate) fn trusted<T>(&self, request: T) -> TrustedRequest<T> {
        TrustedRequest::from_authenticated_bus(
            Arc::clone(&self.subject),
            self.state.clone(),
            request,
        )
    }
}
