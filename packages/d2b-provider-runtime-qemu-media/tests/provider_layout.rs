use d2b_provider_runtime_qemu_media::QemuMediaProviderDescriptor;

#[test]
fn descriptor_declares_guest_and_no_provider_state_volume() {
    let descriptor = QemuMediaProviderDescriptor::default();
    assert!(descriptor.validate().is_ok());
    assert_eq!(descriptor.resource_types(), &["Guest"]);
    assert!(descriptor.state_namespaces().is_empty());
    assert!(
        descriptor
            .process_templates()
            .contains(&"qemu-media-runner")
    );
}
