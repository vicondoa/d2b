//! integration-target: container
//! Fake-host integration fixtures for the qemu-media Provider.

mod scaffold;

#[cfg(test)]
mod tests {
    use super::scaffold;

    #[test]
    fn fixture_contains_guest_runtime_volume_and_process() {
        let (guest, volume, process) = scaffold::fixture();
        assert_eq!(
            guest.provider_ref.to_canonical_string(),
            "Provider/runtime-qemu-media"
        );
        assert_eq!(volume.cleanup_policy(), "vm-stop-with-proof");
        assert_eq!(process.template, "qemu-media-runner");
    }
}
