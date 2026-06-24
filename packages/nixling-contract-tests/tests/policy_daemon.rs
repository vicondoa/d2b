//! Daemon / stop-DAG / processes-json / kernel-modules policy + source/doc
//! lints (the "H-group"), migrated from the `tests/*-eval.sh` bash gates. Each
//! test reads the real repo files (via the `nixling_contract_tests` repo-file
//! helpers) and asserts a structural / documentation invariant. This crate runs
//! only from `tests/tools/rust-workspace-checks.sh` against the real checkout (it is
//! excluded from the hermetic Nix sandbox workspace build), so repo-file access
//! is sound.
//!
//! Migrated gates:
//!   * tests/broker-systemd-unit-eval.sh    -> broker_systemd_unit_declarations
//!   * tests/stop-dag-reconcile-eval.sh      -> stop_dag_reconcile_surface
//!   * tests/processes-json-eval.sh          -> processes_json_consumers_route_through_helpers
//!   * tests/kernel-modules-parity-eval.sh   -> kernel_modules_parity_evaluator_shape

use nixling_contract_tests::{read_repo_file, repo_path_exists};
use regex::Regex;

/// Whether any single line of `content` matches `pattern`. This mirrors `grep`'s
/// per-line evaluation faithfully (so a `\s*` in the pattern can never span a
/// newline boundary, as it could with a whole-file `Regex::is_match`).
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

// ---------------------------------------------------------------------------
// Migrated from tests/broker-systemd-unit-eval.sh.
//
// Asserts nixling-priv-broker.service + nixling-priv-broker.socket are
// unconditionally configured in `nixos-modules/host-broker.nix` (NOT gated
// behind `cfg.daemonExperimental.enable`), and that the canonical
// socket/service shape is preserved: ListenSequentialPacket =
// /run/nixling/priv.sock, SocketGroup = nixlingd, SocketMode = 0660,
// serviceConfig.Type = "notify", and the socket unit wantedBy sockets.target.
// ---------------------------------------------------------------------------
#[test]
fn broker_systemd_unit_declarations() {
    let rel = "nixos-modules/host-broker.nix";
    assert!(
        repo_path_exists(rel),
        "broker-systemd-unit-eval: {rel} missing"
    );
    let module = read_repo_file(rel);

    // (a) gating REMOVED — the module must not wrap its config in
    // `lib.mkIf cfg.daemonExperimental.enable`.
    assert!(
        !any_line_matches(
            &module,
            r#"config\s*=\s*lib\.mkIf\s+cfg\.daemonExperimental\.enable"#
        ),
        "broker-systemd-unit-eval: config still gated behind cfg.daemonExperimental.enable in {rel}"
    );

    // (b) socket declaration present + correct path/group/mode.
    assert!(
        any_line_matches(
            &module,
            r#"ListenSequentialPacket\s*=\s*"/run/nixling/priv\.sock""#
        ),
        r#"broker-systemd-unit-eval: ListenSequentialPacket = "/run/nixling/priv.sock" missing"#
    );
    assert!(
        any_line_matches(&module, r#"SocketGroup\s*=\s*"nixlingd""#),
        r#"broker-systemd-unit-eval: SocketGroup = "nixlingd" missing"#
    );
    assert!(
        any_line_matches(&module, r#"SocketMode\s*=\s*"0660""#),
        r#"broker-systemd-unit-eval: SocketMode = "0660" missing"#
    );

    // (c) serviceConfig.Type = "notify".
    assert!(
        any_line_matches(&module, r#"Type\s*=\s*"notify""#),
        r#"broker-systemd-unit-eval: serviceConfig.Type = "notify" missing"#
    );

    // (d) socket unit must wantedBy sockets.target so it activates at boot
    // without operator intervention.
    assert!(
        any_line_matches(&module, r#"wantedBy\s*=\s*\[\s*"sockets\.target"\s*\]"#),
        "broker-systemd-unit-eval: socket unit not wantedBy sockets.target"
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/stop-dag-reconcile-eval.sh.
//
// Asserts the stop-DAG owner module + docs carry the documented surface and
// that the planner only dispatches through existing broker ops (no new wire
// variants smuggled in via the planner).
// ---------------------------------------------------------------------------
#[test]
fn stop_dag_reconcile_surface() {
    // ==> stop-DAG owner module surface
    let module_rel = "packages/nixlingd/src/supervisor/stop_dag.rs";
    assert!(
        repo_path_exists(module_rel),
        "stop-dag-reconcile-eval: missing {module_rel}"
    );
    let module = read_repo_file(module_rel);
    for sym in [
        "pub struct StopDagOwner",
        "pub struct ObservedHostState",
        "pub struct ReconcileReport",
        "pub struct NftablesReconcileAction",
        "pub struct UsbipReconcileAction",
        "pub enum NftablesDriftReason",
        "pub enum UsbipDriftReason",
        "pub fn reconcile_on_restart",
        "pub fn reconcile(",
    ] {
        assert!(
            module.contains(sym),
            "stop-dag-reconcile-eval: stop_dag.rs missing '{sym}'"
        );
    }

    // ==> supervisor mod wires stop_dag
    let mod_rs = read_repo_file("packages/nixlingd/src/supervisor/mod.rs");
    assert!(
        mod_rs.contains("pub mod stop_dag;"),
        "stop-dag-reconcile-eval: supervisor/mod.rs does not declare stop_dag module"
    );

    // ==> planner uses only existing broker ops (no new wire variants).
    // The planner must not introduce a new BrokerRequest variant; assert the
    // three ops it composes against are present (reused, not redeclared).
    let wire = read_repo_file("packages/nixling-contracts/src/broker_wire.rs");
    for variant in ["ApplyNftables", "UsbipBind", "UsbipUnbind"] {
        assert!(
            wire.contains(&format!("{variant}({variant}Request)")),
            "stop-dag-reconcile-eval: broker_wire.rs missing pre-existing BrokerRequest::{variant}"
        );
    }

    // Negative: the stop_dag module must not declare a `pub enum` / `pub struct`
    // that ends in `Request` (that would be a wire-shape addition smuggled in
    // via the planner).
    assert!(
        !any_line_matches(&module, r"pub (struct|enum) [A-Za-z]+Request\b"),
        "stop-dag-reconcile-eval: stop_dag.rs declares a *Request type; it must dispatch \
         through existing broker wire variants"
    );

    // ==> documentation
    let doc_rel = "docs/reference/stop-dag-reconcile.md";
    assert!(
        repo_path_exists(doc_rel),
        "stop-dag-reconcile-eval: missing {doc_rel}"
    );
    let doc = read_repo_file(doc_rel);
    for marker in [
        "stop-dag-reconcile",
        "StopDagOwner",
        "ApplyNftables",
        "UsbipBind",
        "UsbipUnbind",
        "reconcile_on_restart",
        "ObservedHostState",
    ] {
        assert!(
            doc.contains(marker),
            "stop-dag-reconcile-eval: stop-dag-reconcile.md missing '{marker}'"
        );
    }
}

// ---------------------------------------------------------------------------
// Migrated from tests/processes-json-eval.sh.
//
// Asserts `nixos-modules/processes-json.nix`, `closures-json.nix`,
// `minijail-profiles.nix`, and `store.nix` do NOT directly read
// `config.microvm.vms.<name>.config.config.*` — all per-VM runner config flows
// through the nixling-owned helpers `nl.vmRunner` / `nl.vmToplevel` /
// `nl.vmDeclaredRunner` defined in `nixos-modules/lib.nix`. lib.nix itself is
// allowed to contain the helper bodies (which DO read config.microvm.vms.*);
// the helpers' existence there is asserted explicitly.
// ---------------------------------------------------------------------------
#[test]
fn processes_json_consumers_route_through_helpers() {
    let direct_read = r"config\.microvm\.vms\.\$\{[^}]*\}\.config\.config";
    for f in [
        "processes-json.nix",
        "closures-json.nix",
        "minijail-profiles.nix",
        "store.nix",
    ] {
        let rel = format!("nixos-modules/{f}");
        assert!(repo_path_exists(&rel), "processes-json-eval: {rel} missing");
        let module = read_repo_file(&rel);
        assert!(
            !any_line_matches(&module, direct_read),
            "processes-json-eval: {rel} still reads config.microvm.vms.<name>.config.config.* \
             directly (must route through nl.vmRunner/vmToplevel/vmDeclaredRunner)"
        );
    }

    let lib_module = read_repo_file("nixos-modules/lib.nix");
    for helper in ["vmRunner", "vmToplevel", "vmDeclaredRunner"] {
        assert!(
            any_line_matches(&lib_module, &format!(r"^\s*{helper}\s*=")),
            "processes-json-eval: helper {helper} missing from nixos-modules/lib.nix"
        );
    }
}

// ---------------------------------------------------------------------------
// Migrated from tests/kernel-modules-parity-eval.sh.
//
// Verifies the structural contract for per-VM kernel-modules parity: the
// nixling-owned per-VM evaluator (`nixos-modules/vm-evaluator.nix`) calls the
// standard NixOS `eval-config.nix` entrypoint (the path NixOS uses to compute
// `requiredKernelModules`), and the `nl.vmRunner` helper in `lib.nix` routes
// through `config.nixling._computed` so the per-VM `microvm.*` (incl.
// `microvm.kernel`) attrset resolves.
// ---------------------------------------------------------------------------
#[test]
fn kernel_modules_parity_evaluator_shape() {
    let evaluator_rel = "nixos-modules/vm-evaluator.nix";
    assert!(
        repo_path_exists(evaluator_rel),
        "kernel-modules-parity-eval: {evaluator_rel} missing"
    );
    let evaluator = read_repo_file(evaluator_rel);
    assert!(
        any_line_matches(&evaluator, r"eval-config\.nix"),
        "kernel-modules-parity-eval: {evaluator_rel} does not call eval-config.nix \
         (per-VM kernel-modules computation requires it)"
    );

    let lib_module = read_repo_file("nixos-modules/lib.nix");
    assert!(
        any_line_matches(&lib_module, r"config\.nixling\._computed"),
        "kernel-modules-parity-eval: vmRunner helper does not route through nixling._computed \
         (kernel paths unreadable)"
    );
}
