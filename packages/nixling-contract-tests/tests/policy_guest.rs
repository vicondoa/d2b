//! Guest-control + OTel policy/source-lint gates (part of the W3 "H-group"),
//! migrated from three `tests/*.sh` bash gates:
//!
//!   * `tests/guest-control-auth-nongoals.sh`
//!   * `tests/guest-control-vsock-helper-static.sh`
//!   * `tests/otel-acl-migration-eval.sh`
//!
//! Each test reads the real repo files (via the `nixling_contract_tests`
//! helpers) and asserts a structural source/doc invariant. This crate runs
//! only from `tests/rust-workspace-checks.sh` against the real checkout (it is
//! excluded from the hermetic Nix sandbox workspace build), so repo-file
//! access is sound.
//!
//! Spec correction (`guest-control-auth-nongoals.sh`): the bash gate no longer
//! greps doc/source for the historical "non-goals". Per the gate's own comment,
//! the guest-control-health readiness non-goal was retired in W15 (framework
//! readiness now rides the authenticated guest-control Health probe) and the
//! `nixling exec` CLI non-goal was retired in W16 (the admin-only `vm exec` /
//! `vm konsole` surface landed). The gate degenerated to a `nix eval` smoke of
//! `tests/guest-control-auth-eval.nix`, whose evaluation is already covered by
//! the nix-unit case `tests/nix-unit/cases/guest-control-auth.nix` (see
//! `tests/migration-state.d/guest-control-auth-eval.toml`). Because this
//! pure-Rust contract crate does not run `nix`, the port asserts the
//! still-source-greppable current reality: the two former non-goals are now
//! shipped goals, and the auth-core token-share / LoadCredential contract the
//! smoke eval exercises is present in the module source. The bash gate's
//! `rg`-availability precondition is a shell-tooling guard with no Rust
//! analogue and is intentionally dropped.

use nixling_contract_tests::{read_repo_file, repo_root};
use regex::Regex;

/// Recursively collect every `*.rs` file under a repo-relative directory,
/// returning repo-relative, forward-slash-separated path strings.
fn rust_sources_under(rel_dir: &str) -> Vec<String> {
    let root = repo_root();
    let mut out = Vec::new();
    let mut stack = vec![root.join(rel_dir)];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir)
            .unwrap_or_else(|err| panic!("policy-lint: cannot read dir {}: {err}", dir.display()));
        for entry in entries {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                let rel = path
                    .strip_prefix(&root)
                    .expect("path under repo root")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.push(rel);
            }
        }
    }
    out.sort();
    out
}

// Migrated from tests/guest-control-vsock-helper-static.sh.
//
// The raw vsock CONNECT helper `connect_guest_control_vsock*` may appear only
// in the transport module itself and its two sanctioned consumers: the
// guest-control bridge (W15) and the exec connector (W16). A reference in any
// other nixlingd source file is an unsanctioned wiring of the raw transport
// helper.
#[test]
fn guest_control_vsock_helper_stays_transport_confined() {
    const SANCTIONED: [&str; 3] = [
        "packages/nixlingd/src/guest_control_vsock.rs",
        "packages/nixlingd/src/guest_control_bridge.rs",
        "packages/nixlingd/src/exec_session_real.rs",
    ];
    let mut violations = Vec::new();
    for rel in rust_sources_under("packages/nixlingd/src") {
        if SANCTIONED.contains(&rel.as_str()) {
            continue;
        }
        let body = read_repo_file(&rel);
        for (idx, line) in body.lines().enumerate() {
            if line.contains("connect_guest_control_vsock") {
                violations.push(format!("{rel}:{}", idx + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "guest-control-vsock-helper-static: connect_guest_control_vsock is wired \
         outside its transport module + sanctioned consumers: {violations:?}"
    );
}

// Migrated from tests/otel-acl-migration-eval.sh.
//
// v1.1 invariant: the retired host-otel-relay-acl.nix module is no longer
// imported via the public nixos-modules/default.nix entry point (the OTel
// host-bridge + per-VM relay ACL contract migrated into the broker pre-spawn
// pipeline), and the broker runtime still carries the OtelHostBridge spawn
// handler so the migration is meaningful.
#[test]
fn otel_relay_acl_module_not_publicly_imported_and_broker_handler_present() {
    let defaults = read_repo_file("nixos-modules/default.nix");
    // Match a NON-commented import: a line starting with optional whitespace
    // then `./host-otel-relay-acl.nix` (a `#`-prefixed line never matches).
    let uncommented_import = Regex::new(r"^\s*\./host-otel-relay-acl\.nix").unwrap();
    let offending: Vec<&str> = defaults
        .lines()
        .filter(|line| uncommented_import.is_match(line))
        .collect();
    assert!(
        offending.is_empty(),
        "otel-acl-migration-eval: host-otel-relay-acl.nix still imported by \
         nixos-modules/default.nix: {offending:?}"
    );

    let broker_runtime = read_repo_file("packages/nixling-priv-broker/src/runtime.rs");
    assert!(
        broker_runtime.contains("RunnerRole::OtelHostBridge"),
        "otel-acl-migration-eval: broker runtime missing OtelHostBridge handler in \
         packages/nixling-priv-broker/src/runtime.rs"
    );
}

// Migrated from tests/guest-control-auth-nongoals.sh (see the module-level Spec
// correction). The two former guest-control auth non-goals are now shipped
// goals in source.
#[test]
fn retired_guest_control_nongoals_are_now_shipped_goals() {
    // W15: the guest-control-health readiness non-goal became the framework
    // readiness gate (ProcessRole::GuestControlHealth + the readiness
    // predicate), superseding the retired SSH-readiness probe.
    let processes = read_repo_file("packages/nixling-core/src/processes.rs");
    assert!(
        processes.contains("GuestControlHealth"),
        "processes.rs must declare the GuestControlHealth readiness role"
    );
    assert!(
        processes.contains("GuestControlHealth { vm: String }"),
        "processes.rs must declare the GuestControlHealth readiness predicate"
    );

    // W16: the `nixling exec` CLI non-goal became the admin-only guest-control
    // `vm exec` / `vm konsole` surface.
    let cli = read_repo_file("packages/nixling/src/lib.rs");
    assert!(
        cli.contains("Exec(VmExecArgs)"),
        "the CLI must expose the admin-only `vm exec` guest-control verb"
    );
    assert!(
        cli.contains("Konsole(VmKonsoleArgs)"),
        "the CLI must expose the admin-only `vm konsole` guest-control verb"
    );
}

// Migrated from tests/guest-control-auth-nongoals.sh. The auth-core
// token-share / LoadCredential contract the smoke eval
// (tests/guest-control-auth-eval.nix) exercises is present in the module
// source.
#[test]
fn guest_control_auth_core_token_share_contract_present() {
    // Host-side per-VM guest-control credential share: read-only `nl-gctl`
    // virtiofs share mounted at the host-control mount point.
    let host = read_repo_file("nixos-modules/host.nix");
    assert!(
        host.contains(r#"tag = "nl-gctl""#),
        "host.nix must declare the nl-gctl guest-control credential share"
    );
    assert!(
        host.contains(r#"mountPoint = "/run/nixling-guest-control-host""#),
        "host.nix must mount the guest-control share at /run/nixling-guest-control-host"
    );

    // guestd service LoadCredential delivering the token over the share.
    let guest_control = read_repo_file("nixos-modules/guest-control.nix");
    assert!(
        guest_control.contains("nixling-guestd"),
        "guest-control.nix must declare the nixling-guestd service"
    );
    assert!(
        guest_control.contains(r#""guest_control_token:/run/nixling-guest-control-host/token""#),
        "guest-control.nix must LoadCredential guest_control_token from the host share"
    );

    // Operator-facing auth.tokenFile option + the absolute-path / outside
    // /nix/store validation the eval's negative cases cover.
    let host_token = read_repo_file("nixos-modules/guest-control-host.nix");
    assert!(
        host_token.contains("vm.guest.control.auth.tokenFile"),
        "guest-control-host.nix must wire vm.guest.control.auth.tokenFile"
    );
    assert!(
        host_token.contains(r#"lib.hasPrefix "/nix/store/" vm.guest.control.auth.tokenFile"#),
        "guest-control-host.nix must reject tokenFile paths inside /nix/store"
    );
    let options = read_repo_file("nixos-modules/options-vms.nix");
    assert!(
        options.contains("auth.tokenFile = lib.mkOption"),
        "options-vms.nix must declare the auth.tokenFile option"
    );
}
