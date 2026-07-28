use async_trait::async_trait;
use d2b_contracts::v3::{
    AuthenticatedSubjectContext, ZoneId,
    component_session::{AuthorizationLease, EndpointPolicy},
};
use d2b_session::{
    AdmittedComponentSession, OwnedTransport, SessionAcceptor, SessionAuthenticationBinding,
    SessionAuthority, SessionAuthorizationRequest, SessionEngine, TransportEvidence,
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

async fn forge<T: OwnedTransport + 'static>(
    engine: SessionEngine<T>,
    policy: EndpointPolicy,
    zone: ZoneId,
    evidence: TransportEvidence,
) {
    let acceptor = SessionAcceptor::new(policy, zone, Box::new(ForeignAuthority)).unwrap();
    let _: AdmittedComponentSession = acceptor.admit(engine, evidence, 1).await.unwrap();
}
