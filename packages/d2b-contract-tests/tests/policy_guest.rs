//! Guest-control + OTel policy/source-lint gates (part of the W3 "H-group"),
//! migrated from three `tests/*.sh` bash gates:
//!
//!   * `tests/guest-control-auth-nongoals.sh`
//!   * `tests/guest-control-vsock-helper-static.sh`
//!   * `tests/otel-acl-migration-eval.sh`
//!
//! Each test reads the real repo files (via the `d2b_contract_tests`
//! helpers) and asserts a structural source/doc invariant. This crate runs
//! only from `tests/tools/rust-workspace-checks.sh` against the real checkout (it is
//! excluded from the hermetic Nix sandbox workspace build), so repo-file
//! access is sound.
//!
//! Spec correction (`guest-control-auth-nongoals.sh`): the bash gate no longer
//! greps doc/source for the historical "non-goals". Per the gate's own comment,
//! the guest-control-health readiness non-goal was retired in W15 (framework
//! readiness now rides the authenticated guest-control Health probe) and the
//! `d2b exec` CLI non-goal was retired in W16 (the admin-only `vm exec` /
//! `vm konsole` surface landed). The gate degenerated to a `nix eval` smoke of
//! `tests/unit/nix/eval-cases/guest-control-auth-eval.nix`, whose evaluation is already covered by
//! the nix-unit case `tests/unit/nix/cases/guest-control-auth.nix` (see
//! `tests/migration-state.d/guest-control-auth-eval.toml`). Because this
//! pure-Rust contract crate does not run `nix`, the port asserts the
//! still-source-greppable current reality: the two former non-goals are now
//! shipped goals, and the runtime ComponentSession credential contract the
//! smoke eval exercises is present in the module source. The bash gate's
//! `rg`-availability precondition is a shell-tooling guard with no Rust
//! analogue and is intentionally dropped.

use d2b_contract_tests::{read_repo_file, repo_root};
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
// other d2bd source file is an unsanctioned wiring of the raw transport
// helper.
#[test]
fn guest_control_vsock_helper_stays_transport_confined() {
    const SANCTIONED: [&str; 3] = [
        "packages/d2bd/src/guest_control_vsock.rs",
        "packages/d2bd/src/guest_control_bridge.rs",
        "packages/d2bd/src/exec_session_real.rs",
    ];
    let mut violations = Vec::new();
    for rel in rust_sources_under("packages/d2bd/src") {
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

    let broker_runtime = read_repo_file("packages/d2b-priv-broker/src/runtime.rs");
    assert!(
        broker_runtime.contains("RunnerRole::OtelHostBridge"),
        "otel-acl-migration-eval: broker runtime missing OtelHostBridge handler in \
         packages/d2b-priv-broker/src/runtime.rs"
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
    let processes = read_repo_file("packages/d2b-core/src/processes.rs");
    assert!(
        processes.contains("GuestControlHealth"),
        "processes.rs must declare the GuestControlHealth readiness role"
    );
    assert!(
        processes.contains("GuestControlHealth { vm: String }"),
        "processes.rs must declare the GuestControlHealth readiness predicate"
    );

    // W16: the `d2b exec` CLI non-goal became the admin-only guest-control
    // `vm exec` surface. (The earlier `vm konsole` verb was superseded by
    // `vm exec` upstream and is intentionally absent.)
    let cli = read_repo_file("packages/d2b/src/lib.rs");
    assert!(
        cli.contains("Exec(VmExecArgs)"),
        "the CLI must expose the admin-only `vm exec` guest-control verb"
    );
}

#[test]
fn guest_session_credential_contract_present() {
    let workloads = read_repo_file("nixos-modules/workload-process-rows.nix");
    assert!(
        workloads.contains(r#"tag = "d2b-gctl""#)
            && workloads.contains(r#"mountPoint = "/run/d2b-guest-control-host""#)
            && workloads.contains(r#""workload-runtime").path}/guest-session""#)
            && workloads.contains("readOnly = true;"),
        "workload-process-rows.nix must expose only the workload runtime credential directory"
    );

    let host_credential = read_repo_file("nixos-modules/guest-control-host.nix");
    assert!(
        host_credential.contains("options.d2b._workloadGuestSessionCredentialRows")
            && host_credential.contains(r#"format = "GuestSessionCredentialV1""#)
            && host_credential.contains("schemaVersion = 1;")
            && host_credential.contains("generation = row.controller;")
            && host_credential.contains("materialization = row.broker;")
            && host_credential.contains("materializedByHostActivation = false;")
            && host_credential.contains("bundleArtifact = false;")
            && host_credential.contains("derivationMaterial = false;"),
        "guest-control-host.nix must declare runtime-only realm-confined session credentials"
    );

    let guest_control = read_repo_file("nixos-modules/guest-control.nix");
    assert!(
        guest_control.contains("d2b-guestd"),
        "guest-control.nix must declare the d2b-guestd service"
    );
    assert!(
        guest_control.contains(r#"type = lib.types.enum [ "d2b-guest-session-v2" ];"#)
            && guest_control
                .contains(r#""${cfg.sessionCredential.name}:${cfg.sessionCredential.sourcePath}""#),
        "guest-control.nix must deliver only the fixed ComponentSession credential"
    );

    assert!(
        !host_credential.contains("auth.tokenFile")
            && !host_credential.contains("guest_control_token"),
        "guest session rows must not retain a caller path or legacy token"
    );
}

#[test]
fn guest_session_credential_has_exact_public_bindings_and_private_delivery() {
    let host_credential = read_repo_file("nixos-modules/guest-control-host.nix");
    for field in [
        r#""sessionGeneration""#,
        r#""parentPublicKey""#,
        r#""channelBinding""#,
        r#""guestIdentity""#,
        r#""guestPublicKey""#,
        r#""operationId""#,
        r#""realmId""#,
        r#""workloadId""#,
        r#""controllerGeneration""#,
        r#""workloadGeneration""#,
        r#""runtimeInstanceHandleDigest""#,
        r#""transportEndpointDigest""#,
        r#""purpose""#,
        r#""replayNonce""#,
        r#""expiresAtUnixMs""#,
        r#""binding""#,
        r#""secret""#,
    ] {
        assert!(
            host_credential.contains(field),
            "GuestSessionCredentialV1 is missing {field}"
        );
    }
    for forbidden in [r#""parentPrivateKey""#, r#""guestPrivateKey""#] {
        assert!(
            host_credential.contains(forbidden),
            "GuestSessionCredentialV1 must explicitly forbid {forbidden}"
        );
    }
    for invariant in [
        r#"directoryMode = "0750""#,
        r#"mode = "0440""#,
        r#"mode = "0400""#,
        "ambientFallback = false;",
        r#"mechanism = "authenticated-component-session-fd""#,
        r#"service = "d2b.broker.v2""#,
        r#"method = "Apply""#,
        "methodId = 2253834528;",
        "attachmentCount = 1;",
        r#"attachmentKind = "file-descriptor""#,
        r#"descriptor = "memfd""#,
        r#"access = "read-only""#,
        r#"purpose = "request-input""#,
        "sealedRequired = true;",
        "cloexecRequired = true;",
        "exactStorageRefRequired = true;",
        "pathPayloadAllowed = false;",
        "operationPskAllOrNone = true;",
        "operationPskSingleUse = true;",
        r#"restart = "rotate-before-publish""#,
        r#"adoption = "exact-binding-or-quarantine""#,
        r#"stale = "fail-closed""#,
        r#"ambiguous = "fail-closed""#,
    ] {
        assert!(
            host_credential.contains(invariant),
            "guest session credential contract is missing {invariant}"
        );
    }
}

#[test]
fn legacy_guest_token_materializer_and_signing_authority_are_absent() {
    assert!(
        !repo_root()
            .join("nixos-modules/guest-control-token-materialize.py")
            .exists(),
        "the long-lived guest token materializer must be deleted"
    );
    assert!(
        !repo_root()
            .join("packages/d2b-host/tests/guest_control_token_materializer.rs")
            .exists(),
        "the retired materializer test must be deleted with its implementation"
    );

    for path in [
        "nixos-modules/guest-control-host.nix",
        "nixos-modules/host.nix",
        "nixos-modules/host-activation.nix",
        "nixos-modules/guest-control.nix",
        "nixos-modules/privileges-json.nix",
    ] {
        let source = read_repo_file(path);
        assert!(
            !source.contains("guest_control_token") && !source.contains("guest-control-token"),
            "{path} retains a legacy token surface"
        );
    }
    assert!(
        !read_repo_file("nixos-modules/privileges-json.nix").contains("GuestControlSign"),
        "the emitted privileges matrix must omit GuestControlSign"
    );

    let legacy_storage = read_repo_file("nixos-modules/storage-json.nix");
    assert!(
        !legacy_storage.contains("path:vm-run-guest-control")
            && !legacy_storage.contains(r#"/run/d2b/vms/${name}/guest-control"#),
        "the inactive legacy storage emitter must not retain a guest token directory"
    );

    let public_manifest = read_repo_file("nixos-modules/bundle-artifacts.nix");
    for forbidden in [
        "GuestSessionCredentialV1",
        "d2b-guest-session-v2",
        "parentPrivateKey",
        "guestPrivateKey",
        "operationPsk",
    ] {
        assert!(
            !public_manifest.contains(forbidden),
            "the public manifest emitter must not contain {forbidden}"
        );
    }
}

#[test]
fn privileges_parity_does_not_hide_live_guest_control_sign() {
    let parity = read_repo_file("packages/d2b-contract-tests/tests/privileges_parity.rs");
    assert!(
        !parity.contains("GuestControlSign")
            && !parity.contains(".retain(|row|")
            && !parity.contains(".filter(|row|"),
        "privileges parity must compare the complete live Rust matrix without filtering"
    );
}

#[test]
fn guest_activation_stays_guest_systemd_only_and_restart_safe() {
    let guest_control = read_repo_file("nixos-modules/guest-control.nix");
    assert!(
        guest_control.contains("restartIfChanged = false;"),
        "d2b-guestd.service must opt out of restartIfChanged so guest activation does not restart guestd"
    );
    assert!(
        guest_control.contains("d /run/d2b-guestd/activations 0700 root root -"),
        "guest-control.nix must declare the root-owned activation status directory"
    );
    assert!(
        guest_control.contains("--activation-systemd-run-path")
            && guest_control.contains("--activation-systemctl-path"),
        "guestd must receive explicit in-guest systemd-run/systemctl paths for activation"
    );

    let service = read_repo_file("packages/d2b-guestd/src/service.rs");
    assert!(
        service.contains("GUEST_CAPABILITY_SYSTEM_ACTIVATION"),
        "guestd must advertise the closed system-activation capability when usable"
    );
    assert!(
        service.contains("KillMode=control-group") && service.contains("RuntimeMaxSec="),
        "activation transient units must use systemd control-group lifetime and runtime ceilings"
    );
    assert!(
        !service.contains(r#".arg("--collect")"#),
        "activation transient units must remain queryable until guestd records terminal status"
    );
    assert!(
        !service.contains(r#".arg("sh")"#) && !service.contains(r#".arg("-c")"#),
        "activation must not route through a shell wrapper"
    );
    assert!(
        !service.contains("d2bd.service") && !service.contains("d2b-priv-broker"),
        "guest activation support must not orchestrate host daemon/broker services"
    );
}
