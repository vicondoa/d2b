use std::fs;
use std::process::Command;

#[test]
fn check_config_validates_json_and_paths_without_runtime_side_effects() {
    let temp = std::env::temp_dir().join(format!(
        "d2b-clipd-test-{}-{}.json",
        std::process::id(),
        unique_suffix()
    ));
    fs::write(&temp, r#"{"version":1}"#).expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_d2b-clipd"))
        .arg("--config")
        .arg(&temp)
        .arg("--bridge-root")
        .arg("/run/d2b/clipd")
        .arg("--check-config")
        .output()
        .expect("spawn d2b-clipd");
    let _ = fs::remove_file(&temp);

    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("config ok"));
}

#[test]
fn check_config_rejects_relative_picker_path() {
    let temp = std::env::temp_dir().join(format!(
        "d2b-clipd-test-{}-{}.json",
        std::process::id(),
        unique_suffix()
    ));
    fs::write(&temp, r#"{"version":1}"#).expect("write config");

    let output = Command::new(env!("CARGO_BIN_EXE_d2b-clipd"))
        .arg("--config")
        .arg(&temp)
        .arg("--bridge-root")
        .arg("/run/d2b/clipd")
        .arg("--picker")
        .arg("relative-picker")
        .arg("--check-config")
        .output()
        .expect("spawn d2b-clipd");
    let _ = fs::remove_file(&temp);

    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("must be absolute"));
}

fn unique_suffix() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos()
}
