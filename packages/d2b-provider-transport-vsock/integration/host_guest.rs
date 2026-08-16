//! integration-target: container

use d2b_provider_transport_vsock::VsockTransportDescriptor;

#[test]
fn host_guest_transport_descriptor_disallows_fd_transfer() {
    let descriptor = VsockTransportDescriptor::default();
    assert!(!descriptor.supports_attachments);
    assert!(!descriptor.packet_atomic);
}
