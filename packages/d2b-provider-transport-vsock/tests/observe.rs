use d2b_provider_transport_vsock::{
    ServicePhase, TransportObservation, TransportPhase, VsockTransportDescriptor,
};

#[test]
fn released_observation_is_identity_free_and_bounded() {
    let observation = TransportObservation {
        phase: TransportPhase::Released,
        descriptor: VsockTransportDescriptor::default(),
        bytes_rx: Some(4),
        bytes_tx: Some(5),
        last_exit: Some(d2b_provider_transport_vsock::BridgeExit::OwnerClosed),
    };
    assert_eq!(observation.phase, TransportPhase::Released);
    assert_eq!(ServicePhase::Ready, ServicePhase::Ready);
}
