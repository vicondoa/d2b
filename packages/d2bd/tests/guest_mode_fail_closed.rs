use std::{fs, path::PathBuf, process::Command};

fn scratch(name: &str) -> PathBuf {
    let root = std::env::current_dir()
        .expect("current directory")
        .join("target")
        .join(format!("u3-guest-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create project scratch");
    root
}

fn guest_args(root: &PathBuf) -> Vec<String> {
    vec![
        "guest".to_owned(),
        "--guest-ref".to_owned(),
        "Guest/workload".to_owned(),
        "--guest-uid".to_owned(),
        "123e4567-e89b-42d3-a456-426614174000".to_owned(),
        "--zone".to_owned(),
        "work".to_owned(),
        "--schema-fingerprint".to_owned(),
        "sha256:0000000000000000000000000000000000000000000000000000000000000001".to_owned(),
        "--broker-socket".to_owned(),
        root.join("guest-broker.sock").display().to_string(),
        "--state-dir".to_owned(),
        root.join("state").display().to_string(),
        "--boot-id-path".to_owned(),
        root.join("boot-id").display().to_string(),
        "--validate-only".to_owned(),
    ]
}

#[test]
fn guest_validation_never_materializes_host_store_public_or_realm_surfaces() {
    let root = scratch("surfaces");
    fs::write(root.join("boot-id"), "boot-id-u3\n").expect("write boot identity");
    let output = Command::new(env!("CARGO_BIN_EXE_d2bd"))
        .args(guest_args(&root))
        .env("D2B_PUBLIC_SOCKET", root.join("forbidden-public.sock"))
        .env("D2B_REALM_IDENTITY", root.join("forbidden-realm.json"))
        .output()
        .expect("run d2bd guest");
    assert!(
        output.status.success(),
        "guest validation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!root.join("forbidden-public.sock").exists());
    assert!(!root.join("forbidden-realm.json").exists());
    assert!(!root.join("state").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn guest_mode_rejects_host_style_config_flags_at_process_start() {
    let output = Command::new(env!("CARGO_BIN_EXE_d2bd"))
        .args([
            "guest",
            "--config",
            "/etc/d2b/daemon-config.json",
            "--guest-ref",
            "Guest/workload",
        ])
        .output()
        .expect("run d2bd guest");
    assert_eq!(output.status.code(), Some(2));
}
