//! Opaque identity capability established by an authenticated bus session.

use std::sync::Arc;

use d2b_contracts::v3::AuthenticatedSubjectContext as SessionClaims;

use crate::authz::AuthorizationState;

/// Session claims and policy state issued only after transport authentication.
pub struct AuthenticatedSubjectContext {
    claims: Arc<SessionClaims>,
    authorization_state: AuthorizationState,
}

impl AuthenticatedSubjectContext {
    pub(crate) fn claims(&self) -> &Arc<SessionClaims> {
        &self.claims
    }

    pub(crate) const fn authorization_state(&self) -> &AuthorizationState {
        &self.authorization_state
    }
}

impl core::fmt::Debug for AuthenticatedSubjectContext {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("AuthenticatedSubjectContext(<redacted>)")
    }
}

#[cfg(test)]
pub(crate) fn issue_test_subject(
    claims: Arc<SessionClaims>,
    authorization_state: AuthorizationState,
) -> AuthenticatedSubjectContext {
    struct TestSessionIssuer;

    impl TestSessionIssuer {
        fn issue(
            self,
            claims: Arc<SessionClaims>,
            authorization_state: AuthorizationState,
        ) -> AuthenticatedSubjectContext {
            AuthenticatedSubjectContext {
                claims,
                authorization_state,
            }
        }
    }

    TestSessionIssuer.issue(claims, authorization_state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authz::{AuthorizationState, BootstrapPhase};
    use d2b_contracts::v3::{
        BindingDigest, ConfigurationGeneration, EvidenceClass, Locality, ReconnectGeneration,
        ResourceRef, ResourceUid, SchemaFingerprint, ServiceName, SessionBinding, SessionPurpose,
        TranscriptHash, TransportBinding, ZoneRevision,
    };
    use d2b_resource_store::PolicySnapshot;

    #[test]
    fn authenticated_subject_debug_redacts_every_protected_claim() {
        const ZONE_SENTINEL: &str = "identity-zone-sentinel";
        const REF_SENTINEL: &str = "User/identity-ref-sentinel";
        const UID_SENTINEL: &str = "11111111-1111-4111-8111-111111111111";
        const PURPOSE_SENTINEL: &str = "identity-purpose-sentinel";
        const SERVICE_SENTINEL: &str = "identity.service.sentinel";

        let claims = Arc::new(SessionClaims::new(
            ResourceRef::parse(REF_SENTINEL).unwrap(),
            ResourceUid::parse(UID_SENTINEL).unwrap(),
            ResourceRef::parse(&format!("Zone/{ZONE_SENTINEL}")).unwrap(),
            EvidenceClass::UnixPeer,
            SessionPurpose::parse(PURPOSE_SENTINEL).unwrap(),
            ServiceName::parse(SERVICE_SENTINEL).unwrap(),
            SessionBinding::new(
                SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
                TransportBinding::new(
                    Locality::Local,
                    BindingDigest::parse(format!("sha256:{}", "2".repeat(64))).unwrap(),
                ),
                ReconnectGeneration::new(1).unwrap(),
                TranscriptHash::from_bytes([3; 32]),
            ),
        ));
        let subject = issue_test_subject(
            claims,
            AuthorizationState {
                snapshot: PolicySnapshot {
                    policy_revision: 4,
                    api_catalog_revision: 5,
                    active_configuration_revision: ConfigurationGeneration::new(6).unwrap(),
                    controller_generation: None,
                },
                zone_policy_revision: ZoneRevision::new(7),
                bootstrap_phase: BootstrapPhase::Disabled,
                now_tick: 8,
            },
        );
        let rendered = format!("{subject:?}");

        for sentinel in [
            ZONE_SENTINEL,
            REF_SENTINEL,
            UID_SENTINEL,
            PURPOSE_SENTINEL,
            SERVICE_SENTINEL,
        ] {
            assert!(!rendered.contains(sentinel), "{rendered}");
        }
    }
}
