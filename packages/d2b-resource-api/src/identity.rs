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
