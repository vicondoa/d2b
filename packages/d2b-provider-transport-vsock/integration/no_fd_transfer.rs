//! integration-target: container

use d2b_provider_transport_vsock::VsockTransportDescriptor;

#[test]
fn no_fd_transfer_over_vsock_is_structural() {
    assert!(!VsockTransportDescriptor::default().supports_attachments);
}
