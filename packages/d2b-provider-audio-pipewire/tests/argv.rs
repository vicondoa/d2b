use d2b_provider_audio_pipewire::{AudioComponentTemplate, AudioTemplateError};

#[test]
fn component_template_has_no_socket_or_live_process_argv() {
    let template = AudioComponentTemplate::new("dev-vm", "/run/d2b/vms/dev-vm/d2b-dev-vm").unwrap();
    let rendered = template.render();
    assert_eq!(rendered.executable_ref, "vhost-user-sound-worker");
    assert!(!rendered.argv.iter().any(|arg| arg == "--socket"));
    assert!(rendered.process_spec_json.get("argv").is_none());
}

#[test]
fn template_rejects_store_paths_and_other_guest_copies() {
    assert!(matches!(
        AudioComponentTemplate::new(
            "dev-vm",
            "/nix/store/hash-vhost-device-sound/bin/vhost-device-sound"
        ),
        Err(AudioTemplateError::NotPerGuestCopy)
    ));
    assert!(matches!(
        AudioComponentTemplate::new("dev-vm", "/run/d2b/vms/other/d2b-other"),
        Err(AudioTemplateError::NotPerGuestCopy)
    ));
}
