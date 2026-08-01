use async_trait::async_trait;
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, ZoneId,
    component_session::{AuthorizationLease, EndpointPolicy},
};
use d2b_bus::ZoneRegistrar;
use d2b_session::{
    SessionAuthenticationBinding, SessionAuthority, SessionAuthorizationRequest,
    TransportEvidence,
};

struct ForeignAuthority;

#[async_trait]
impl SessionAuthority for ForeignAuthority {
    async fn authenticate_connect(
        &mut self,
        _evidence: TransportEvidence,
        _binding: &SessionAuthenticationBinding,
        _expected_zone: &ZoneId,
        _now_tick: u64,
    ) -> d2b_session::Result<(AuthenticatedSubjectContext, AuthorizationLease)> {
        unimplemented!()
    }

    async fn authorize(
        &mut self,
        _subject: &AuthenticatedSubjectContext,
        _request: &SessionAuthorizationRequest,
        _previous_lease: AuthorizationLease,
        _now_tick: u64,
    ) -> d2b_session::Result<AuthorizationLease> {
        unimplemented!()
    }
}

fn forge(
    registrar: &ZoneRegistrar,
    policy: EndpointPolicy,
) {
    let _acceptor = registrar
        .component_session_acceptor(policy, Box::new(ForeignAuthority))
        .unwrap();
}
