use d2b_provider_transport_vsock::{
    OpaqueBindingId, OpaqueEndpointId, OpenTransportRequest, ServiceError, TransportRole,
};

#[test]
fn open_request_deadline_is_bounded() {
    let request = OpenTransportRequest::new(
        OpaqueEndpointId::parse("endpoint-a").unwrap(),
        OpaqueBindingId::parse("binding-a").unwrap(),
        TransportRole::Initiator,
        999,
    );
    assert_eq!(
        request.deadline_ms, 999,
        "the service performs the final range check"
    );
    assert_eq!(ServiceError::InvalidDeadline.code(), "invalid-deadline");
    assert_eq!(
        OpenTransportRequest::from_raw("bad/path", "binding-a", TransportRole::Initiator, 1_000)
            .unwrap_err(),
        ServiceError::InvalidEndpointId
    );
    assert_eq!(
        OpenTransportRequest::from_raw("endpoint-a", "bad/path", TransportRole::Initiator, 1_000)
            .unwrap_err(),
        ServiceError::InvalidBindingId
    );
}
