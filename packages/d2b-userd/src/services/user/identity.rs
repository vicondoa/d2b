use std::{fmt, sync::Arc};

use d2b_contracts::{
    v2_component_session::EndpointRole,
    v2_identity::{RealmId, RoleId},
    v2_provider::{CredentialPlacementBinding, Generation},
};
use d2b_session_unix::PeerCredentials;

#[derive(Clone, PartialEq, Eq)]
pub struct OwnerBinding {
    uid: u32,
    realm_id: RealmId,
    role_id: RoleId,
    agent_generation: Generation,
}

impl OwnerBinding {
    pub fn new(uid: u32, realm_id: RealmId, role_id: RoleId, agent_generation: Generation) -> Self {
        Self {
            uid,
            realm_id,
            role_id,
            agent_generation,
        }
    }

    pub const fn uid(&self) -> u32 {
        self.uid
    }

    pub const fn agent_generation(&self) -> Generation {
        self.agent_generation
    }

    pub fn realm_id(&self) -> &RealmId {
        &self.realm_id
    }

    pub fn role_id(&self) -> &RoleId {
        &self.role_id
    }

    pub fn placement(&self) -> CredentialPlacementBinding {
        CredentialPlacementBinding::UserAgent {
            realm_id: self.realm_id.clone(),
            role_id: self.role_id.clone(),
            agent_generation: self.agent_generation,
        }
    }

    pub fn owns_placement(&self, placement: &CredentialPlacementBinding) -> bool {
        self.placement() == *placement
    }

    pub fn authorize(&self, request: &AuthenticatedUser) -> Result<(), UserSecretError> {
        if request.peer_uid != self.uid
            || request.realm_id != self.realm_id
            || request.peer_role != EndpointRole::CommandClient
        {
            return Err(UserSecretError::Unauthorized);
        }
        Ok(())
    }
}

impl fmt::Debug for OwnerBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnerBinding")
            .field("uid", &"<redacted>")
            .field("realm_id", &"<redacted>")
            .field("role_id", &"<redacted>")
            .field("agent_generation", &self.agent_generation)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AuthenticatedUser {
    peer_uid: u32,
    realm_id: RealmId,
    peer_role: EndpointRole,
    session_generation: Generation,
}

impl AuthenticatedUser {
    pub fn from_verified_peer(
        peer: PeerCredentials,
        realm_id: RealmId,
        peer_role: EndpointRole,
        session_generation: Generation,
    ) -> Self {
        Self {
            peer_uid: peer.uid().as_raw(),
            realm_id,
            peer_role,
            session_generation,
        }
    }

    #[cfg(test)]
    pub(crate) fn command_client(
        peer_uid: u32,
        realm_id: RealmId,
        session_generation: Generation,
    ) -> Self {
        Self {
            peer_uid,
            realm_id,
            peer_role: EndpointRole::CommandClient,
            session_generation,
        }
    }

    #[cfg(test)]
    pub(crate) fn for_test(
        peer_uid: u32,
        realm_id: RealmId,
        peer_role: EndpointRole,
        session_generation: Generation,
    ) -> Self {
        Self {
            peer_uid,
            realm_id,
            peer_role,
            session_generation,
        }
    }

    pub const fn peer_uid(&self) -> u32 {
        self.peer_uid
    }

    pub fn realm_id(&self) -> &RealmId {
        &self.realm_id
    }

    pub const fn session_generation(&self) -> Generation {
        self.session_generation
    }
}

impl fmt::Debug for AuthenticatedUser {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthenticatedUser")
            .field("peer_uid", &"<redacted>")
            .field("realm_id", &"<redacted>")
            .field("peer_role", &self.peer_role)
            .field("session_generation", &self.session_generation)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserSecretError {
    InvalidRequest,
    Unauthorized,
    Locked,
    NotFound,
    Conflict,
    ResourceExhausted,
    DeadlineExpired,
    Unavailable,
    AmbiguousMutation,
    InvariantViolation,
}

impl fmt::Display for UserSecretError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "user-secret-invalid-request",
            Self::Unauthorized => "user-secret-unauthorized",
            Self::Locked => "user-secret-locked",
            Self::NotFound => "user-secret-not-found",
            Self::Conflict => "user-secret-conflict",
            Self::ResourceExhausted => "user-secret-resource-exhausted",
            Self::DeadlineExpired => "user-secret-deadline-expired",
            Self::Unavailable => "user-secret-unavailable",
            Self::AmbiguousMutation => "user-secret-ambiguous-mutation",
            Self::InvariantViolation => "user-secret-invariant-violation",
        })
    }
}

impl std::error::Error for UserSecretError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecretOperation {
    Status,
    AcquireLease,
    InspectLease,
    RefreshLease,
    RevokeLease,
    DeleteCredential,
    ExportCredential,
    InspectExport,
    RevokeExport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosedOutcome {
    Succeeded,
    AlreadyApplied,
    Denied,
    Locked,
    NotFound,
    Failed,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretMetricEvent {
    pub operation: SecretOperation,
    pub outcome: ClosedOutcome,
}

pub trait SecretMetricSink: Send + Sync {
    fn record(&self, event: SecretMetricEvent);
}

#[derive(Debug, Default)]
pub struct NoopSecretMetrics;

impl SecretMetricSink for NoopSecretMetrics {
    fn record(&self, _: SecretMetricEvent) {}
}

pub(crate) fn record_result<T>(
    metrics: &Arc<dyn SecretMetricSink>,
    operation: SecretOperation,
    result: &Result<T, UserSecretError>,
) {
    let outcome = match result {
        Ok(_) => ClosedOutcome::Succeeded,
        Err(UserSecretError::Unauthorized) => ClosedOutcome::Denied,
        Err(UserSecretError::Locked) => ClosedOutcome::Locked,
        Err(UserSecretError::NotFound) => ClosedOutcome::NotFound,
        Err(UserSecretError::AmbiguousMutation) => ClosedOutcome::Ambiguous,
        Err(_) => ClosedOutcome::Failed,
    };
    metrics.record(SecretMetricEvent { operation, outcome });
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v2_identity::{RealmPath, RoleKind, WorkloadId, WorkloadName};

    fn owner() -> OwnerBinding {
        let realm = RealmId::derive(&RealmPath::root());
        let workload = WorkloadId::derive(
            &realm,
            &WorkloadName::parse("user-agent").expect("workload"),
        );
        OwnerBinding::new(
            1000,
            realm.clone(),
            RoleId::derive(&realm, &workload, RoleKind::WaylandProxy),
            Generation::new(7).expect("generation"),
        )
    }

    #[test]
    fn owner_requires_exact_uid_realm_role_and_generation() {
        let owner = owner();
        let exact = AuthenticatedUser::command_client(
            1000,
            owner.realm_id().clone(),
            owner.agent_generation(),
        );
        assert!(owner.authorize(&exact).is_ok());

        let wrong_uid = AuthenticatedUser::command_client(
            1001,
            owner.realm_id().clone(),
            owner.agent_generation(),
        );
        assert_eq!(
            owner.authorize(&wrong_uid),
            Err(UserSecretError::Unauthorized)
        );

        let wrong_role = AuthenticatedUser::for_test(
            1000,
            owner.realm_id().clone(),
            EndpointRole::LocalRootController,
            owner.agent_generation(),
        );
        assert_eq!(
            owner.authorize(&wrong_role),
            Err(UserSecretError::Unauthorized)
        );
    }

    #[test]
    fn identity_debug_is_redacted() {
        let owner = owner();
        let debug = format!("{owner:?}");
        assert!(!debug.contains("1000"));
        assert!(!debug.contains(owner.realm_id().as_str()));
    }
}
