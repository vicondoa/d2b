use d2b_provider_device_usbip::{
    AttachSource, EphemeralProcessIntent, EphemeralProcessKind, UsbipDaemonProcess,
};

#[test]
fn declared_and_explicit_bind_are_distinct_ephemeral_paths() {
    let declared = EphemeralProcessIntent::from_core(
        EphemeralProcessKind::Bind,
        AttachSource::Declared,
        [1; 16],
    );
    let explicit = EphemeralProcessIntent::from_core(
        EphemeralProcessKind::Bind,
        AttachSource::Explicit,
        [2; 16],
    );
    assert_ne!(declared.source(), explicit.source());
    assert_eq!(UsbipDaemonProcess::declaration().template, "usbip-daemon");
}
