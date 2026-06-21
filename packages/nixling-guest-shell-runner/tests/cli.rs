use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn rejects_template_names_before_runtime() {
    let mut cmd = Command::cargo_bin("nixling-guest-shell-runner").unwrap();
    cmd.args([
        "detach",
        "--socket",
        "/run/user/1000/nl-shpool.sock",
        "--name",
        "work-{workspace}",
        "--json",
    ])
    .assert()
    .failure()
    .stderr(predicate::str::contains("unsupported character '{'"));
}

#[test]
fn management_json_has_stable_shape() {
    let mut cmd = Command::cargo_bin("nixling-guest-shell-runner").unwrap();
    cmd.args([
        "kill",
        "--socket",
        "/run/user/1000/nl-shpool.sock",
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
fn rejects_overlong_socket_paths() {
    let long_path = format!("/run/user/1000/{}", "a".repeat(120));
    let mut cmd = Command::cargo_bin("nixling-guest-shell-runner").unwrap();
    cmd.args(["list", "--socket", &long_path, "--json"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("sockaddr_un"));
}
