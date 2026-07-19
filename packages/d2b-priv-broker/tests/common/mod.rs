use d2b_priv_broker::service_v2::{BrokerPeerRole, broker_endpoint_policy};

pub fn local_root_policy() -> d2b_contracts::v2_component_session::EndpointPolicy {
    broker_endpoint_policy(
        BrokerPeerRole::LocalRootController,
        d2b_contracts::v2_component_session::EndpointRole::LocalRootBroker,
        1,
        [1; 32],
    )
    .expect("local-root broker endpoint policy")
}
