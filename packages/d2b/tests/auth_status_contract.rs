//! W3 CLI-contract integration test, migrated from
//! tests/cli-rust-native-auth-status.sh.
//!
//! Spawns the real `d2b` binary and drives `auth status` across the three
//! authorization roles (launcher / none / admin) using the env-file fixture
//! contract (`D2B_AUTH_STATUS_FIXTURE` + `D2B_TEST_{LAUNCHER,ADMIN}_UIDS`
//! + `--test-uid`). It asserts that:
//!   * `auth status --json` deserializes strictly into
//!     `d2b_contracts::cli_output::AuthStatusOutputV2` (`deny_unknown_fields` makes a successful
//!     typed deserialize equivalent to the schema check the bash gate did via
//!     docs/reference/cli-output/auth-status.schema.json);
//!   * the per-role allowed/denied subcommand authz surface matches the binary's
//!     contract (launcher gets `up` but keeps `audit` denied; `none` stays
//!     read-only; admin gains `audit` and denies nothing);
//!   * `auth status --human` summarizes the role and the denied `audit` access.
//!
//! Unlike the `list` gate, `auth status` is driven entirely by env-file fixtures
//! and never reads the bundle/manifest, so this test needs no D2B_FIXTURES and
//! always runs (no skip path).

use std::process::Command;

use d2b_contracts::cli_output::{AuthRoleV2, AuthStatusOutputV2};

// Fixture JSON shapes, reproduced from tests/cli-rust-native-common.sh's
// d2b_write_auth_status_fixture. They drive the public/broker socket probes;
// the role itself is selected by the *_UIDS env vars + --test-uid, not by the
// fixture, so launcher and admin share the same reachable-public shape.
const LAUNCHER_FIXTURE: &str = r#"{
  "publicReachable": true,
  "publicVersion": "0.4.0-test",
  "brokerReachable": false,
  "brokerVersion": null
}"#;

const NONE_FIXTURE: &str = r#"{
  "publicReachable": false,
  "publicVersion": null,
  "brokerReachable": false,
  "brokerVersion": null
}"#;

const ADMIN_FIXTURE: &str = r#"{
  "publicReachable": true,
  "publicVersion": "0.4.0-test",
  "brokerReachable": false,
  "brokerVersion": null
}"#;

fn write_fixture(dir: &std::path::Path, name: &str, body: &str) -> std::path::PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, body).expect("write auth-status fixture");
    path
}

fn run_auth_status(
    fixture: &std::path::Path,
    test_uid: u32,
    role_env: &[(&str, &str)],
    json: bool,
) -> std::process::Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_d2b"));
    cmd.args(["auth", "status", "--test-uid"])
        .arg(test_uid.to_string())
        .arg(if json { "--json" } else { "--human" })
        .env("D2B_AUTH_STATUS_FIXTURE", fixture);
    for (key, value) in role_env {
        cmd.env(key, value);
    }
    cmd.output().expect("spawn d2b auth status")
}

fn parse_json(out: &std::process::Output) -> AuthStatusOutputV2 {
    assert!(
        out.status.success(),
        "`d2b auth status --json` exited {:?}; stderr:\n{}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr)
    );
    // Strict schema validation: AuthStatusOutputV2 (and its nested DTOs) are
    // deny_unknown_fields, so a successful typed deserialize is equivalent to
    // validating against docs/reference/cli-output/auth-status.schema.json.
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "auth status --json did not match the AuthStatusOutputV2 schema: {err}\noutput:\n{}",
            String::from_utf8_lossy(&out.stdout)
        )
    })
}

#[test]
fn auth_status_roles_match_schema_and_authz() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let launcher_fixture = write_fixture(tmp.path(), "auth-launcher.json", LAUNCHER_FIXTURE);
    let none_fixture = write_fixture(tmp.path(), "auth-none.json", NONE_FIXTURE);
    let admin_fixture = write_fixture(tmp.path(), "auth-admin.json", ADMIN_FIXTURE);

    // Case 1 — launcher: gains launcher-allowed verbs (e.g. `up`) but keeps
    // `audit` denied.
    let launcher = parse_json(&run_auth_status(
        &launcher_fixture,
        1000,
        &[("D2B_TEST_LAUNCHER_UIDS", "1000")],
        true,
    ));
    assert_eq!(launcher.role, AuthRoleV2::Launcher, "uid 1000 -> launcher");
    assert_eq!(launcher.effective_uid, 1000);
    assert!(
        launcher.allowed_subcommands.iter().any(|c| c == "up"),
        "launcher allows `up`; got {:?}",
        launcher.allowed_subcommands
    );
    assert!(
        launcher
            .denied_subcommands
            .iter()
            .any(|d| d.name == "audit"),
        "launcher keeps `audit` denied; got {:?}",
        launcher.denied_subcommands
    );

    // Case 2 — none: read-only surface only.
    let none = parse_json(&run_auth_status(&none_fixture, 2000, &[], true));
    assert_eq!(none.role, AuthRoleV2::None, "uid 2000 -> none");
    let mut none_allowed = none.allowed_subcommands.clone();
    none_allowed.sort();
    assert_eq!(
        none_allowed,
        vec![
            "auth status",
            "host check",
            "list",
            "op inspect",
            "realm inspect",
            "realm list",
            "status",
        ],
        "none role stays read-only"
    );

    // Case 3 — admin: gains `audit`, denies nothing.
    let admin = parse_json(&run_auth_status(
        &admin_fixture,
        2001,
        &[("D2B_TEST_ADMIN_UIDS", "2001")],
        true,
    ));
    assert_eq!(admin.role, AuthRoleV2::Admin, "uid 2001 -> admin");
    assert!(
        admin.denied_subcommands.is_empty(),
        "admin denies nothing; got {:?}",
        admin.denied_subcommands
    );
    assert!(
        admin.allowed_subcommands.iter().any(|c| c == "audit"),
        "admin gains `audit`; got {:?}",
        admin.allowed_subcommands
    );

    // Case 4 — --human (launcher): summarizes the role and the denied audit.
    let human = run_auth_status(
        &launcher_fixture,
        1000,
        &[("D2B_TEST_LAUNCHER_UIDS", "1000")],
        false,
    );
    assert!(
        human.status.success(),
        "`d2b auth status --human` exited {:?}; stderr:\n{}",
        human.status.code(),
        String::from_utf8_lossy(&human.stderr)
    );
    let human_out = String::from_utf8_lossy(&human.stdout);
    assert!(
        human_out.contains("role: launcher"),
        "human output reports the role; got:\n{human_out}"
    );
    assert!(
        human_out.contains("audit:"),
        "human output lists the denied audit subcommand; got:\n{human_out}"
    );
}
