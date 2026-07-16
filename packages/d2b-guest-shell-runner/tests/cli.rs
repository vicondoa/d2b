use assert_cmd::Command;
use predicates::prelude::*;

const SOCKET: &str = "/run/user/1000/d2b-shpool.sock";

fn binary() -> Command {
    Command::cargo_bin("d2b-guest-shell-runner").unwrap()
}

#[test]
fn help_identifies_internal_data_plane_only() {
    binary()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Internal libshpool data-plane helper for d2b guest service",
        ))
        .stdout(predicate::str::contains("component-session").not())
        .stdout(predicate::str::contains("--json").not());
}

#[test]
fn rejects_legacy_json_control_flag() {
    for subcommand in ["list", "detach", "kill"] {
        let mut args = vec![subcommand, "--socket", SOCKET];
        if subcommand != "list" {
            args.extend(["--name", "default"]);
        }
        args.push("--json");
        binary()
            .args(args)
            .assert()
            .failure()
            .stderr(predicate::str::contains("unexpected argument '--json'"));
    }
}

#[test]
fn rejects_template_names_before_backend_access() {
    for subcommand in ["attach", "detach", "kill"] {
        binary()
            .args([subcommand, "--socket", SOCKET, "--name", "work-{workspace}"])
            .assert()
            .failure()
            .stdout(predicate::str::is_empty())
            .stderr(predicate::str::contains("unsupported character '{'"));
    }
}

#[test]
#[cfg(not(feature = "real-libshpool"))]
fn every_backend_mode_fails_closed_without_libshpool() {
    let commands = [
        vec!["daemon", "--socket", SOCKET, "--home", "/home/alice"],
        vec!["attach", "--socket", SOCKET, "--name", "default"],
        vec!["list", "--socket", SOCKET],
        vec!["detach", "--socket", SOCKET, "--name", "default"],
        vec!["kill", "--socket", SOCKET, "--name", "default"],
    ];

    for args in commands {
        binary()
            .args(args)
            .assert()
            .failure()
            .stdout(predicate::str::is_empty())
            .stderr(predicate::str::contains(
                "retained shell libshpool backend is unavailable",
            ))
            .stderr(predicate::str::contains(r#""command":"#).not())
            .stderr(predicate::str::contains("/home/alice").not())
            .stderr(predicate::str::contains(SOCKET).not());
    }
}

#[test]
#[cfg(feature = "real-libshpool")]
fn real_attach_still_validates_name_before_connecting() {
    binary()
        .args(["attach", "--socket", SOCKET, "--name", "work-{workspace}"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("unsupported character '{'"));
}

#[test]
fn rejects_overlong_external_socket_paths_without_reflecting_them() {
    let long_path = format!("/run/user/1000/{}", "a".repeat(120));
    binary()
        .args(["list", "--socket", &long_path])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains("sockaddr_un"))
        .stderr(predicate::str::contains(&long_path).not());
}

#[test]
fn rejects_relative_external_socket_paths_without_reflecting_them() {
    binary()
        .args(["list", "--socket", "relative.sock"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "libshpool socket path must be absolute",
        ))
        .stderr(predicate::str::contains("relative.sock").not());
}

#[test]
fn rejects_relative_home_paths_without_reflecting_them() {
    binary()
        .args(["daemon", "--socket", SOCKET, "--home", "home/alice"])
        .assert()
        .failure()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::contains(
            "libshpool home path must be absolute",
        ))
        .stderr(predicate::str::contains("home/alice").not());
}
