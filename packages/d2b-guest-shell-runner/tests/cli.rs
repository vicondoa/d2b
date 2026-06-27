use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn rejects_template_names_before_runtime() {
    let mut cmd = Command::cargo_bin("d2b-guest-shell-runner").unwrap();
    cmd.args([
        "detach",
        "--socket",
        "/run/user/1000/d2b-shpool.sock",
        "--name",
        "work-{workspace}",
        "--json",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("unsupported character '{'"));
}

#[test]
#[cfg(not(feature = "real-libshpool"))]
fn management_json_has_stable_shape() {
    let mut cmd = Command::cargo_bin("d2b-guest-shell-runner").unwrap();
    cmd.args([
        "kill",
        "--socket",
        "/run/user/1000/d2b-shpool.sock",
        "--name",
        "default",
        "--json",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains(
        r#""command":"kill","name":"default","result":"unsupported""#,
    ));
}

#[test]
#[cfg(not(feature = "real-libshpool"))]
fn list_json_reports_unsupported_shape() {
    let mut cmd = Command::cargo_bin("d2b-guest-shell-runner").unwrap();
    cmd.args([
        "list",
        "--socket",
        "/run/user/1000/d2b-shpool.sock",
        "--json",
    ])
    .assert()
    .success()
    .stdout(predicate::str::contains(
        r#""command":"list","name":"","result":"unsupported""#,
    ));
}

#[test]
#[cfg(not(feature = "real-libshpool"))]
fn daemon_stub_reports_neutral_unsupported_error() {
    let mut cmd = Command::cargo_bin("d2b-guest-shell-runner").unwrap();
    cmd.args([
        "daemon",
        "--socket",
        "/run/user/1000/d2b-shpool.sock",
        "--home",
        "/home/alice",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains(
        "persistent shell daemon mode is not enabled in this helper build",
    ))
    .stderr(predicate::str::contains(r#"home="/home/alice""#));
}

#[test]
#[cfg(not(feature = "real-libshpool"))]
fn attach_stub_reports_neutral_unsupported_error() {
    let mut cmd = Command::cargo_bin("d2b-guest-shell-runner").unwrap();
    cmd.args([
        "attach",
        "--socket",
        "/run/user/1000/d2b-shpool.sock",
        "--name",
        "default",
        "--force",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains(
        "persistent shell attach mode is not enabled in this helper build: force=true",
    ));
}

#[test]
#[cfg(not(feature = "real-libshpool"))]
fn non_json_management_outputs_are_stable() {
    for (subcommand, expected) in [
        (
            ["list", "--socket", "/run/user/1000/d2b-shpool.sock"].as_slice(),
            "shell session listing is not implemented in this helper build",
        ),
        (
            [
                "detach",
                "--socket",
                "/run/user/1000/d2b-shpool.sock",
                "--name",
                "default",
            ]
            .as_slice(),
            "detach for 'default' is not implemented in this helper build",
        ),
        (
            [
                "kill",
                "--socket",
                "/run/user/1000/d2b-shpool.sock",
                "--name",
                "default",
            ]
            .as_slice(),
            "kill for 'default' is not implemented in this helper build",
        ),
    ] {
        let mut cmd = Command::cargo_bin("d2b-guest-shell-runner").unwrap();
        cmd.args(subcommand)
            .assert()
            .success()
            .stdout(predicate::str::contains(expected));
    }

    #[test]
    #[cfg(feature = "real-libshpool")]
    fn real_attach_still_validates_name_before_connecting() {
        let mut cmd = Command::cargo_bin("d2b-guest-shell-runner").unwrap();
        cmd.args([
            "attach",
            "--socket",
            "/run/user/1000/d2b-shpool.sock",
            "--name",
            "work-{workspace}",
        ])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported character '{'"));
    }
}

#[test]
fn rejects_overlong_socket_paths() {
    let long_path = format!("/run/user/1000/{}", "a".repeat(120));
    let mut cmd = Command::cargo_bin("d2b-guest-shell-runner").unwrap();
    cmd.args(["list", "--socket", &long_path, "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("sockaddr_un"));
}

#[test]
fn rejects_relative_socket_paths() {
    let mut cmd = Command::cargo_bin("d2b-guest-shell-runner").unwrap();
    cmd.args(["list", "--socket", "relative.sock", "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("socket path must be absolute"));
}
