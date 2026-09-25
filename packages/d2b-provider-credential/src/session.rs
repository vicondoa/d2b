//! The Credential family's session and revocation vocabulary.
//!
//! The v3 `ResourceDriver` conversion of the daemon-owned Credential
//! controller path deleted the old reconciler and kept the Provider
//! revocation surface. What remains is family vocabulary: the exact
//! non-secret `RevokeToken` request whose delete path binds one session
//! generation, the confirmed-revocation evidence the driver projects,
//! and the typed session port the daemon's ProviderSupervisor handoff
//! satisfies.
//!
//! The production session adapter (the generated ttrpc client over an
//! authenticated ComponentSession), the handoff registry the daemon
//! populates, and the same-Zone scoped delivery gate stay in the daemon
//! behind this vocabulary: they hold runtime state, and the family declares
//! only what its driver and its declarations must agree on.

use async_trait::async_trait;
use d2b_contracts_provider::v3::credential::CredentialMethod;
use d2b_contracts_provider::v3::credential_controller::{
    CredentialIdempotencyKey, CredentialProviderKind,
};
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId, canonical_digest,
    identity::ReconnectGeneration,
};

/// Stable failures from the Credential session path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialResourceRuntimeError {
    /// A resource or identity failed closed validation.
    InvalidResource,
    /// A typed Credential session refused or could not confirm revocation.
    Revocation,
    /// The dependency-facts read failed (a manager RPC failure), so the
    /// caller can distinguish a failed read from an absent row.
    DependencyFacts,
}

impl core::fmt::Display for CredentialResourceRuntimeError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidResource => "credential-resource-invalid",
            Self::Revocation => "credential-revocation-unconfirmed",
            Self::DependencyFacts => "credential-dependency-facts-unavailable",
        })
    }
}

impl std::error::Error for CredentialResourceRuntimeError {}

/// Exact non-secret inputs for one provider-side RevokeToken call.
///
/// The driver fills these from the durable row, the Provider facts, and the
/// live session; [`CredentialRevocationRequest::new`] owns the closed
/// validation, so an incomplete identity can never reach a Provider.
pub struct CredentialRevocationInputs {
    /// The zone the Credential row lives in.
    pub zone: ZoneId,
    /// The Credential row being torn down.
    pub credential_ref: ResourceRef,
    /// The Credential row's committed uid.
    pub credential_uid: ResourceUid,
    /// The Credential row's current generation.
    pub credential_generation: ResourceGeneration,
    /// The user the credential material belongs to, when it is user-scoped.
    pub user_ref: Option<ResourceRef>,
    /// The Credential Provider serving the row.
    pub provider_ref: ResourceRef,
    /// The Provider row's current generation.
    pub provider_generation: ResourceGeneration,
    /// The zone controller generation the request binds.
    pub controller_generation: ControllerGeneration,
    /// The live Provider session generation the request binds.
    pub session_generation: ReconnectGeneration,
    /// `credential.rotationGeneration`; a missing or zero value keeps the
    /// old status read's default of 1.
    pub rotation_generation: u64,
}

/// Exact non-secret input for one provider-side RevokeToken call.
///
/// Construct one through [`CredentialRevocationRequest::new`], which owns the
/// closed validation and derives the durable operation identity. The identity
/// fields below are the exact values the request was validated with; the
/// derived fields stay behind accessors so a hand-built request cannot claim
/// an operation id this type did not derive.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialRevocationRequest {
    /// The zone the Credential row lives in.
    pub zone: ZoneId,
    /// The Credential row being torn down.
    pub credential_ref: ResourceRef,
    /// The Credential row's committed uid.
    pub credential_uid: ResourceUid,
    /// The Credential row's current generation.
    pub credential_generation: ResourceGeneration,
    /// The user the credential material belongs to, when it is user-scoped.
    pub user_ref: Option<ResourceRef>,
    /// The Credential Provider serving the row.
    pub provider_ref: ResourceRef,
    /// The Provider row's current generation.
    pub provider_generation: ResourceGeneration,
    /// The zone controller generation the request binds.
    pub controller_generation: ControllerGeneration,
    /// The live Provider session generation the request binds.
    pub session_generation: ReconnectGeneration,
    operation_id: String,
    idempotency_key: String,
    deadline_ms: u64,
}

impl core::fmt::Debug for CredentialRevocationRequest {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CredentialRevocationRequest")
            .field("credential_ref", &"<redacted>")
            .field("credential_uid", &"<redacted>")
            .field("credential_generation", &self.credential_generation)
            .field("user_ref", &"<redacted>")
            .field("provider_ref", &"<redacted>")
            .field("provider_generation", &self.provider_generation)
            .field("controller_generation", &self.controller_generation)
            .field("session_generation", &self.session_generation)
            .field("operation_id", &"<redacted>")
            .field("idempotency_key", &"<redacted>")
            .field("deadline_ms", &self.deadline_ms)
            .finish()
    }
}

impl CredentialRevocationRequest {
    /// Validate and bind one revocation identity. The request is only
    /// constructible for a credential Provider with a live (non-zero)
    /// session generation; everything else fails closed as
    /// [`CredentialResourceRuntimeError::InvalidResource`].
    ///
    /// # Errors
    ///
    /// Returns [`CredentialResourceRuntimeError::InvalidResource`] when
    /// the session generation is zero or unknown, the rotation generation
    /// is zero, or the Provider reference does not name a credential
    /// Provider.
    pub fn new(inputs: CredentialRevocationInputs) -> Result<Self, CredentialResourceRuntimeError> {
        if inputs.session_generation.get() == 0
            || inputs.rotation_generation == 0
            || credential_provider_kind(&inputs.provider_ref).is_none()
        {
            return Err(CredentialResourceRuntimeError::InvalidResource);
        }
        let operation_id = credential_revoke_operation_id(&inputs);
        let idempotency_key = CredentialIdempotencyKey::derive(
            &inputs.credential_uid,
            inputs.rotation_generation,
            CredentialMethod::RevokeToken,
        )
        .map_err(|_| CredentialResourceRuntimeError::Revocation)?
        .request_value();
        Ok(Self {
            zone: inputs.zone,
            credential_ref: inputs.credential_ref,
            credential_uid: inputs.credential_uid,
            credential_generation: inputs.credential_generation,
            user_ref: inputs.user_ref,
            provider_ref: inputs.provider_ref,
            provider_generation: inputs.provider_generation,
            controller_generation: inputs.controller_generation,
            session_generation: inputs.session_generation,
            operation_id,
            idempotency_key,
            deadline_ms: 10_000,
        })
    }

    /// The durable operation identity this request re-issues.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// The request's idempotency key.
    pub fn idempotency_key(&self) -> &str {
        &self.idempotency_key
    }

    /// The provider call deadline in milliseconds.
    pub const fn deadline_ms(&self) -> u64 {
        self.deadline_ms
    }

    /// The session generation this request binds.
    pub fn session_generation(&self) -> ReconnectGeneration {
        self.session_generation
    }
}

/// The durable revocation identity the old reconciler derived: the same
/// zone/uid/generation/Provider/generation tuple always yields the same
/// operation id, so a rejoined daemon re-issues the identical request.
fn credential_revoke_operation_id(inputs: &CredentialRevocationInputs) -> String {
    let preimage = format!(
        "{}:{}:{}:{}:{}:{}",
        inputs.zone.as_str(),
        inputs.credential_uid.as_str(),
        inputs.credential_generation.get(),
        inputs.provider_ref.to_canonical_string(),
        inputs.provider_generation.get(),
        inputs.controller_generation.get(),
    );
    format!(
        "credential-revoke-{}",
        canonical_digest("d2b:credential-revoke/v1", preimage.as_bytes())
    )
}

/// Provider-side revocation result. Only confirmed outcomes may unblock
/// Process cleanup and finalizer release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialRevocationOutcome {
    /// The Provider confirmed the lease revoked.
    Revoked,
    /// The Provider confirmed the lease was already revoked.
    AlreadyRevoked,
    /// The Provider could not confirm; cleanup must not proceed.
    Uncertain,
}

/// Confirmed-or-uncertain revocation evidence for one request.
#[derive(Clone, PartialEq, Eq)]
pub struct CredentialRevocationEvidence {
    operation_id: String,
    outcome: CredentialRevocationOutcome,
    session_generation: ReconnectGeneration,
}

impl CredentialRevocationEvidence {
    /// Bind one revocation outcome to the request that produced it.
    pub fn confirmed(
        request: &CredentialRevocationRequest,
        outcome: CredentialRevocationOutcome,
    ) -> Self {
        Self {
            operation_id: request.operation_id.clone(),
            outcome,
            session_generation: request.session_generation,
        }
    }

    /// The durable operation identity the evidence belongs to.
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// The session generation the evidence binds.
    pub fn session_generation(&self) -> ReconnectGeneration {
        self.session_generation
    }

    /// The status outcome code the old durable status published.
    pub fn outcome_code(&self) -> &'static str {
        match self.outcome {
            CredentialRevocationOutcome::Revoked => "revoked",
            CredentialRevocationOutcome::AlreadyRevoked => "already-revoked",
            CredentialRevocationOutcome::Uncertain => "uncertain",
        }
    }
}

impl core::fmt::Debug for CredentialRevocationEvidence {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("CredentialRevocationEvidence")
            .field("operation_id", &"<redacted>")
            .field("outcome", &self.outcome)
            .field("session_generation", &self.session_generation)
            .finish()
    }
}

/// Typed Credential session used by the resource driver's cleanup effect.
#[async_trait]
pub trait CredentialSession: Send + Sync {
    /// The live session generation, or `None` when the session is not live.
    fn session_generation(&self) -> Option<ReconnectGeneration>;

    /// Revoke one credential lease through the authenticated Provider
    /// session.
    ///
    /// # Errors
    ///
    /// Returns [`CredentialResourceRuntimeError::Revocation`] when the
    /// session refuses or cannot confirm the revocation.
    async fn revoke_credential(
        &self,
        request: &CredentialRevocationRequest,
    ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError>;
}

/// The credential Provider kind one Provider reference names, derived from
/// the realizer crates' own exported identities so this closed set cannot
/// drift from the Providers that declare them.
pub fn credential_provider_kind(provider_ref: &ResourceRef) -> Option<CredentialProviderKind> {
    let canonical = provider_ref.to_canonical_string();
    [
        (
            d2b_provider_credential_secret_service::PROVIDER_REF,
            d2b_provider_credential_secret_service::PROVIDER_KIND,
        ),
        (
            d2b_provider_credential_entra::PROVIDER_REF,
            d2b_provider_credential_entra::PROVIDER_KIND,
        ),
        (
            d2b_provider_credential_managed_identity::PROVIDER_REF,
            d2b_provider_credential_managed_identity::PROVIDER_KIND,
        ),
    ]
    .into_iter()
    .find_map(|(reference, kind)| (canonical == reference).then_some(kind))
}

/// Whether one Provider reference names a Credential Provider.
pub fn is_credential_provider_ref(provider_ref: &ResourceRef) -> bool {
    credential_provider_kind(provider_ref).is_some()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use d2b_contracts_resource::v3::identity::ReconnectGeneration;
    use d2b_contracts_resource::v3::{ControllerGeneration, ResourceRef, ResourceUid, ZoneId};
    use parking_lot::Mutex;

    use super::{
        CredentialResourceRuntimeError, CredentialRevocationEvidence, CredentialRevocationInputs,
        CredentialRevocationOutcome, CredentialRevocationRequest, CredentialSession,
        credential_provider_kind, is_credential_provider_ref,
    };

    const MI_PROVIDER: &str = "Provider/credential-managed-identity";

    fn revocation_inputs(session_generation: u64) -> CredentialRevocationInputs {
        CredentialRevocationInputs {
            zone: ZoneId::parse("dev").unwrap(),
            credential_ref: ResourceRef::parse("Credential/relay").unwrap(),
            credential_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            credential_generation: d2b_contracts_resource::v3::ResourceGeneration::new(1).unwrap(),
            user_ref: None,
            provider_ref: ResourceRef::parse(MI_PROVIDER).unwrap(),
            provider_generation: d2b_contracts_resource::v3::ResourceGeneration::new(1).unwrap(),
            controller_generation: ControllerGeneration::new(1).unwrap(),
            session_generation: ReconnectGeneration::new(session_generation).unwrap(),
            rotation_generation: 1,
        }
    }

    fn revocation_request(session_generation: u64) -> CredentialRevocationRequest {
        CredentialRevocationRequest::new(revocation_inputs(session_generation))
            .expect("revocation request")
    }

    /// Every Credential Provider the family owns resolves to its kind, and
    /// nothing else is admitted: the mapping is the realizer crates' own
    /// exported identities, not a second table.
    #[test]
    fn the_provider_kind_mapping_follows_the_declaring_realizers() {
        for (provider_ref, kind) in [
            (
                d2b_provider_credential_secret_service::PROVIDER_REF,
                d2b_contracts_provider::v3::credential_controller::CredentialProviderKind::SecretService,
            ),
            (
                d2b_provider_credential_entra::PROVIDER_REF,
                d2b_contracts_provider::v3::credential_controller::CredentialProviderKind::Entra,
            ),
            (
                d2b_provider_credential_managed_identity::PROVIDER_REF,
                d2b_contracts_provider::v3::credential_controller::CredentialProviderKind::ManagedIdentity,
            ),
        ] {
            let reference = ResourceRef::parse(provider_ref).expect("provider ref");
            assert_eq!(credential_provider_kind(&reference), Some(kind));
            assert!(is_credential_provider_ref(&reference));
        }
        assert!(!is_credential_provider_ref(
            &ResourceRef::parse("Provider/volume-virtiofs").unwrap()
        ));
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn revoke_session_deduplicates_the_fenced_operation_identity() {
        let session = RecordingCredentialSession::default();
        let request = revocation_request(7);
        assert_eq!(
            session.revoke_credential(&request).await.unwrap(),
            CredentialRevocationOutcome::Revoked
        );
        assert_eq!(
            session.revoke_credential(&request).await.unwrap(),
            CredentialRevocationOutcome::AlreadyRevoked
        );
        assert_eq!(session.operations.lock().len(), 1); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
        let debug = format!("{request:?}");
        assert!(!debug.contains("Credential/relay"));
        assert!(!debug.contains("123e4567-e89b-42d3-a456-426614174000"));
        let rejoined = revocation_request(8);
        assert_eq!(request.operation_id, rejoined.operation_id);
        assert_eq!(request.idempotency_key, rejoined.idempotency_key);
        assert_ne!(request.session_generation, rejoined.session_generation);
    }

    #[test]
    fn revocation_request_rejects_zero_rotation_or_foreign_providers() {
        let mut inputs = revocation_inputs(7);
        inputs.rotation_generation = 0;
        assert_eq!(
            CredentialRevocationRequest::new(inputs).unwrap_err(),
            CredentialResourceRuntimeError::InvalidResource
        );

        let mut inputs = revocation_inputs(7);
        inputs.provider_ref = ResourceRef::parse("Provider/volume-virtiofs").unwrap();
        assert_eq!(
            CredentialRevocationRequest::new(inputs).unwrap_err(),
            CredentialResourceRuntimeError::InvalidResource
        );
    }

    #[test]
    fn confirmed_revocation_evidence_is_redacted_and_bounded() {
        let request = revocation_request(7);
        let evidence =
            CredentialRevocationEvidence::confirmed(&request, CredentialRevocationOutcome::Revoked);
        assert_eq!(evidence.operation_id(), request.operation_id.as_str());
        assert_eq!(evidence.session_generation(), request.session_generation);
        assert_eq!(evidence.outcome_code(), "revoked");
        let debug = format!("{evidence:?}");
        assert!(debug.contains("session_generation"));
        assert!(!debug.contains("credential-revoke-"));
        assert!(!debug.contains("123e4567-e89b-42d3-a456-426614174000"));
    }

    #[derive(Clone, Default)]
    struct RecordingCredentialSession {
        operations: Arc<Mutex<std::collections::BTreeMap<String, CredentialRevocationOutcome>>>,
        attempts: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl CredentialSession for RecordingCredentialSession {
        fn session_generation(&self) -> Option<ReconnectGeneration> {
            Some(ReconnectGeneration::new(7).expect("recording session generation"))
        }

        async fn revoke_credential(
            &self,
            request: &CredentialRevocationRequest,
        ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError> {
            self.attempts
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let mut operations = self.operations.lock(); // async-gate-allow: synchronous lock acquisition, no await while the guard is held
            if operations
                .insert(
                    request.operation_id.clone(),
                    CredentialRevocationOutcome::Revoked,
                )
                .is_some()
            {
                Ok(CredentialRevocationOutcome::AlreadyRevoked)
            } else {
                Ok(CredentialRevocationOutcome::Revoked)
            }
        }
    }

    struct UncertainCredentialSession;

    #[async_trait::async_trait]
    impl CredentialSession for UncertainCredentialSession {
        fn session_generation(&self) -> Option<ReconnectGeneration> {
            Some(ReconnectGeneration::new(7).expect("uncertain session generation"))
        }

        async fn revoke_credential(
            &self,
            _request: &CredentialRevocationRequest,
        ) -> Result<CredentialRevocationOutcome, CredentialResourceRuntimeError> {
            Ok(CredentialRevocationOutcome::Uncertain)
        }
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    #[tokio::test]
    async fn uncertain_revocation_never_unblocks_cleanup() {
        // A live session that cannot confirm revocation must keep the
        // credential row alive.
        let session = UncertainCredentialSession;
        assert_eq!(
            session
                .revoke_credential(&revocation_request(7))
                .await
                .unwrap(),
            CredentialRevocationOutcome::Uncertain
        );
    }
}
