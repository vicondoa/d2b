//! Secret-free Entra controller projections.

#[path = "audit.rs"]
mod audit;
#[path = "telemetry.rs"]
mod telemetry;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::credential::{
    CredentialInteractionState, CredentialLeaseStatus, CredentialMetadata, CredentialServiceError,
    CredentialServiceErrorCode, CredentialStatus,
};
use d2b_contracts::v3::credential_controller::{
    CredentialAuditOutcome, CredentialAuditRecord, CredentialControllerDecision,
    CredentialControllerError, CredentialControllerHandlers, CredentialControllerHealth,
    CredentialObservabilityError, CredentialObserveInput, CredentialReconcileInput,
    CredentialRevocationInput, CredentialSingleFlight, CredentialTelemetryFrame,
    CredentialTelemetryOperation, CredentialTelemetryOutcome, observe_credential,
    reconcile_credential, revoke_credential,
};

use crate::{EntraClientState, EntraPlacement, LOGIN_ENDPOINT_PURPOSE, PROVIDER_REF};

/// Canonical provider-visible Endpoint policy for the Entrablau service.
#[derive(Clone, PartialEq, Eq)]
pub struct EntraEndpointPolicy {
    provider_ref: ResourceRef,
    consumer_ref: ResourceRef,
}

impl EntraEndpointPolicy {
    /// Require canonical provider visibility and exact orchestration and
    /// consumer subjects.
    pub fn new(
        visibility: &str,
        provider_ref: ResourceRef,
        consumer_ref: ResourceRef,
    ) -> Result<Self, CredentialServiceError> {
        if visibility != "provider"
            || provider_ref.to_canonical_string() != PROVIDER_REF
            || consumer_ref.resource_type().as_str() != "Provider"
        {
            return Err(CredentialServiceError::new(
                CredentialServiceErrorCode::OperationDenied,
            ));
        }
        Ok(Self {
            provider_ref,
            consumer_ref,
        })
    }

    /// Return the fixed Endpoint purpose.
    pub const fn purpose(&self) -> &'static str {
        LOGIN_ENDPOINT_PURPOSE
    }

    /// Check one exact subject against the two allowed subjects.
    pub fn allows_subject(&self, subject: &ResourceRef) -> bool {
        subject == &self.provider_ref || subject == &self.consumer_ref
    }
}

impl core::fmt::Debug for EntraEndpointPolicy {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("EntraEndpointPolicy(<redacted>)")
    }
}

/// Common status plus client state.
#[derive(Clone, PartialEq, Eq)]
pub struct EntraStatusProjection {
    /// Credential common status.
    pub status: CredentialStatus,
    /// Closed client state.
    pub client_state: EntraClientState,
}

impl core::fmt::Debug for EntraStatusProjection {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("EntraStatusProjection(<redacted>)")
    }
}

/// Stateless status-first Entra controller.
pub struct EntraController {
    placement: EntraPlacement,
    single_flight: CredentialSingleFlight,
}

impl EntraController {
    /// Bind the controller to one identity-Guest placement.
    pub fn new(placement: EntraPlacement) -> Self {
        Self {
            placement,
            single_flight: CredentialSingleFlight::new(),
        }
    }

    /// Project bounded non-secret state.
    pub fn reconcile(
        &self,
        client_state: EntraClientState,
        metadata: Option<&CredentialMetadata>,
    ) -> Result<EntraStatusProjection, CredentialServiceError> {
        let lease = metadata
            .map(|metadata| {
                CredentialLeaseStatus::new(
                    metadata.lease_handle.clone(),
                    metadata.state,
                    metadata.rotation_generation,
                    metadata.source_version.clone(),
                    metadata.expires_at_unix_ms,
                    1,
                    None,
                    None,
                    self.placement.binding(),
                )
            })
            .transpose()
            .map_err(|_| invariant())?;
        let interaction = match client_state {
            EntraClientState::Ready => CredentialInteractionState::NotRequired,
            EntraClientState::InteractionRequired => CredentialInteractionState::Required,
        };
        let status =
            CredentialStatus::new(interaction, None, None, lease).map_err(|_| invariant())?;
        Ok(EntraStatusProjection {
            status,
            client_state,
        })
    }

    /// Build a caller-initiated audit record after the authorization decision.
    #[allow(clippy::too_many_arguments)]
    pub fn authorized_service_audit(
        &self,
        authorized: bool,
        zone: &str,
        subject_identity: &[u8],
        credential_name: &[u8],
        method: d2b_contracts::v3::credential::CredentialMethod,
        outcome: CredentialAuditOutcome,
        rotation_generation: u64,
        idempotency_key: Option<&[u8]>,
    ) -> Result<Option<CredentialAuditRecord>, CredentialObservabilityError> {
        audit::authorized_service_record(
            authorized,
            zone,
            subject_identity,
            credential_name,
            method,
            outcome,
            rotation_generation,
            idempotency_key,
        )
    }

    /// Build one complete closed Credential telemetry frame.
    pub fn telemetry(
        &self,
        zone: &str,
        operation: CredentialTelemetryOperation,
        outcome: CredentialTelemetryOutcome,
        rotation_generation: u64,
    ) -> Result<CredentialTelemetryFrame, CredentialObservabilityError> {
        telemetry::frame(
            zone,
            operation,
            outcome,
            self.placement.binding(),
            rotation_generation,
        )
    }
}

impl core::fmt::Debug for EntraController {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("EntraController(<redacted>)")
    }
}

impl CredentialControllerHandlers for EntraController {
    fn reconcile_handler(
        &self,
        input: &CredentialReconcileInput,
    ) -> Result<CredentialControllerDecision, CredentialControllerError> {
        let _guard = self
            .single_flight
            .try_enter(input.credential_uid().clone())?;
        reconcile_credential(input)
    }

    fn observe(
        &self,
        input: &CredentialObserveInput,
    ) -> Result<CredentialControllerDecision, CredentialControllerError> {
        let _guard = self
            .single_flight
            .try_enter(input.credential_uid().clone())?;
        observe_credential(input)
    }

    fn finalize(
        &self,
        input: &CredentialRevocationInput,
    ) -> Result<CredentialControllerDecision, CredentialControllerError> {
        let _guard = self
            .single_flight
            .try_enter(input.credential_uid().clone())?;
        revoke_credential(input)
    }

    fn drain(
        &self,
        input: &CredentialRevocationInput,
    ) -> Result<CredentialControllerDecision, CredentialControllerError> {
        let _guard = self
            .single_flight
            .try_enter(input.credential_uid().clone())?;
        revoke_credential(input)
    }

    fn health(
        &self,
        provider_process_reachable: bool,
        active_leases: u32,
        locked_count: u32,
    ) -> Result<CredentialControllerHealth, CredentialControllerError> {
        CredentialControllerHealth::derive(provider_process_reachable, active_leases, locked_count)
    }
}

fn invariant() -> CredentialServiceError {
    CredentialServiceError::new(CredentialServiceErrorCode::InvariantFailure)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::credential::PlacementBinding;

    fn controller() -> EntraController {
        EntraController::new(
            EntraPlacement::new(
                PlacementBinding::GuestAgent,
                ResourceRef::parse("Guest/consumer").unwrap(),
                ResourceRef::parse("Guest/identity").unwrap(),
                ResourceRef::parse("Endpoint/entra-login").unwrap(),
                1,
            )
            .unwrap(),
        )
    }

    #[test]
    fn client_state_projects_interaction_required_without_a_denial() {
        let required = controller()
            .reconcile(EntraClientState::InteractionRequired, None)
            .unwrap();
        assert_eq!(
            required.status.interaction_state(),
            CredentialInteractionState::Required
        );
        let ready = controller()
            .reconcile(EntraClientState::Ready, None)
            .unwrap();
        assert_eq!(
            ready.status.interaction_state(),
            CredentialInteractionState::NotRequired
        );
    }
}
