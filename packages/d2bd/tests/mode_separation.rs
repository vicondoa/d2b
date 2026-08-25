use std::{fs, path::PathBuf, process::Command};

use d2bd::StaticProviderComposition;
use d2bd_runtime::{
    broker_transport::ModeBoundBrokerAdapter,
    target_runtime::{AdmissionLimits, DaemonMode},
};

#[test]
fn host_and_guest_modes_are_process_start_surfaces() {
    let host = Command::new(env!("CARGO_BIN_EXE_d2bd"))
        .args(["host", "--help"])
        .output()
        .expect("host help");
    let guest = Command::new(env!("CARGO_BIN_EXE_d2bd"))
        .args(["guest", "--help"])
        .output()
        .expect("guest help");
    assert!(host.status.success());
    assert!(guest.status.success());
    assert_ne!(host.stdout, guest.stdout);
}

#[test]
fn host_and_guest_compositions_share_the_artifact_family_but_not_effect_profile() {
    let host = StaticProviderComposition::new(
        DaemonMode::Host,
        PathBuf::from("/run/d2b/host-broker.sock"),
        997,
        AdmissionLimits::host_default(),
    )
    .expect("Host composition");
    let guest = StaticProviderComposition::new(
        DaemonMode::Guest,
        PathBuf::from("/run/d2b/guest-broker.sock"),
        997,
        AdmissionLimits::guest_default(),
    )
    .expect("Guest composition");
    assert_eq!(host.artifact_digest(), guest.artifact_digest());
    assert_eq!(host.effects().broker_profile().as_str(), "host");
    assert_eq!(guest.effects().broker_profile().as_str(), "guest");
    assert!(
        host.deployment().admission().limits().max_sessions
            > guest.deployment().admission().limits().max_sessions
    );
}

#[test]
fn guest_identity_validation_uses_kernel_boot_id_not_writable_guest_state() {
    let root = std::env::current_dir()
        .expect("current directory")
        .join("target")
        .join(format!("u3-boot-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("scratch");
    fs::write(root.join("boot-id"), "kernel-boot-id").expect("boot id");
    let first =
        d2bd_runtime::guest_mode::BootIdentity::read(root.join("boot-id")).expect("kernel boot id");
    fs::write(root.join("guest-state"), "forged-boot-id").expect("state");
    let second =
        d2bd_runtime::guest_mode::BootIdentity::read(root.join("boot-id")).expect("kernel boot id");
    assert_eq!(first, second);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn guest_effect_adapter_rejects_the_host_broker_instance() {
    let guest = ModeBoundBrokerAdapter::guest(PathBuf::from("/run/d2b/priv.sock"), 997);
    assert!(guest.validate_instance().is_err());
    let guest = ModeBoundBrokerAdapter::guest(PathBuf::from("/run/d2b/guest-broker.sock"), 997);
    assert!(guest.validate_instance().is_ok());
}
