//! The managed-identity Credential Provider's admitted-relationship delivery
//! path.
//!
//! The Provider's agent handshake, lease custody, and refresh semantics are
//! unchanged. What this module adds is the one question that had been answered
//! from the authenticated route alone: whether the delivery session being handed
//! back is the one the admitted `CredentialBinding` relationship currently
//! authorizes. The answer comes from the family-wide gate in
//! `d2b_contracts_provider::v3::credential`, so this crate, the Entra Provider,
//! and the Secret Service Provider cannot drift into three different rules for
//! the same relationship (R24, R35, R41).
//!
//! `dispatch_admitted` is an *additional* entry point. The supervised service
//! loop still reaches `dispatch` through the route-derived authorization, and
//! both reach the same lease custody code below this gate.

use d2b_contracts_provider::v3::credential::{
    AdmittedCredentialDelivery, CredentialAuthorization, CredentialMethod, CredentialRequest,
    CredentialResponse, CredentialServiceError, admit_credential_delivery,
    dispatch_authorized_provider_async, observed_delivery_evidence,
};
use d2b_provider_toolkit::credential::now_unix_ms;

use crate::ManagedIdentityCredentialProvider;

impl ManagedIdentityCredentialProvider {
    /// Dispatch one already-admitted method under an admitted `CredentialBinding`
    /// relationship.
    ///
    /// For a delivery-bearing method the shared gate answers first and refuses
    /// before the managed-identity agent is asked for anything when the
    /// relationship's fence moved, when the presented session's audience or
    /// operation class is outside the admitted policy, or when the presented
    /// session is not the relationship's current one. `revoke-token` and
    /// `inspect-metadata` carry no delivery session, so the relationship is not
    /// consulted for them.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialServiceErrorCode::OperationDenied`] when the
    /// admitted relationship refuses the delivery, and otherwise whatever the
    /// Provider's own dispatch returns for the method.
    pub async fn dispatch_admitted(
        &self,
        method: CredentialMethod,
        request: &CredentialRequest,
        authorization: &CredentialAuthorization,
        relationship: &dyn AdmittedCredentialDelivery,
    ) -> Result<CredentialResponse, CredentialServiceError> {
        if method.requires_delivery() {
            let evidence = observed_delivery_evidence(authorization, now_unix_ms())?;
            admit_credential_delivery(authorization, method, relationship, &evidence)
                .map_err(|refusal| {
                    tracing::warn!(
                        provider = crate::PROVIDER_REF,
                        refusal = refusal.code(),
                        "managed-identity credential delivery refused by the admitted relationship"
                    );
                    refusal.service_error()
                })?;
        }
        dispatch_authorized_provider_async(self, method, request, authorization).await
    }
}