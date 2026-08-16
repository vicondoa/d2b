//! Daemon / stop-DAG / processes-json / kernel-modules policy + source/doc
//! lints (the "H-group"), migrated from the `tests/*-eval.sh` bash gates. Each
//! test reads the real repo files (via the `d2b_contract_tests` repo-file
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

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use regex::Regex;
use std::process::Command;

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
// Asserts d2b-priv-broker.service + d2b-priv-broker.socket are
// unconditionally configured in `nixos-modules/host-broker.nix` (NOT gated
// behind `cfg.daemonExperimental.enable`), and that the canonical
// socket/service shape is preserved: ListenSequentialPacket =
// /run/d2b/priv.sock, SocketGroup = d2bd, SocketMode = 0660,
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

    // (a) gating REMOVED - the module must not wrap its config in
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
            r#"ListenSequentialPacket\s*=\s*"/run/d2b/priv\.sock""#
        ),
        r#"broker-systemd-unit-eval: ListenSequentialPacket = "/run/d2b/priv.sock" missing"#
    );
    assert!(
        any_line_matches(&module, r#"SocketGroup\s*=\s*"d2bd""#),
        r#"broker-systemd-unit-eval: SocketGroup = "d2bd" missing"#
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
// Leftover StopDagOwner stays deleted: the module and supervisor
// declaration must not return.
// ---------------------------------------------------------------------------
#[test]
fn stop_dag_reconcile_surface() {
    let module_rel = "packages/d2bd/src/supervisor/stop_dag.rs";
    assert!(
        !repo_path_exists(module_rel),
        "stop-dag leftover must stay deleted: {module_rel}"
    );
    let mod_rs = read_repo_file("packages/d2bd/src/supervisor/mod.rs");
    assert!(
        !mod_rs.contains("pub mod stop_dag;"),
        "supervisor/mod.rs must not declare leftover stop_dag"
    );
}

// ---------------------------------------------------------------------------
// Migrated from tests/processes-json-eval.sh.
//
// Asserts `nixos-modules/processes-json.nix`, `closures-json.nix`,
// `minijail-profiles.nix`, and `store.nix` do NOT directly read
// `config.microvm.vms.<name>.config.config.*` - all per-VM runner config flows
// through the d2b-owned helpers `d2bLib.vmRunner` / `d2bLib.vmToplevel` /
// `d2bLib.vmDeclaredRunner` defined in `nixos-modules/lib.nix`. lib.nix itself is
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
             directly (must route through d2bLib.vmRunner/vmToplevel/vmDeclaredRunner)"
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
// d2b-owned per-VM evaluator (`nixos-modules/vm-evaluator.nix`) calls the
// standard NixOS `eval-config.nix` entrypoint (the path NixOS uses to compute
// `requiredKernelModules`), and the `d2bLib.vmRunner` helper in `lib.nix` routes
// through `config.d2b._computed` so the per-VM `microvm.*` (incl.
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
        any_line_matches(&lib_module, r"config\.d2b\._computed"),
        "kernel-modules-parity-eval: vmRunner helper does not route through d2b._computed \
         (kernel paths unreadable)"
    );
}

fn fixed_unit_keys(module: &str, declaration: &str) -> Vec<String> {
    let marker = format!("systemd.{declaration} = {{");
    let mut in_block = false;
    let mut depth = 0_i32;
    let mut keys = Vec::new();
    for line in module.lines() {
        if !in_block {
            if line.contains(&marker) {
                in_block = true;
                depth = 1;
            }
            continue;
        }
        if depth == 1
            && let Some(key) = line
                .trim()
                .strip_suffix("= {")
                .map(str::trim)
                .filter(|key| {
                    !key.is_empty()
                        && key.bytes().all(|byte| {
                            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                        })
                })
        {
            keys.push(key.to_owned());
        }
        depth += line.matches('{').count() as i32;
        depth -= line.matches('}').count() as i32;
        if depth <= 0 {
            break;
        }
    }
    keys
}

#[test]
fn daemon_only_unit_census_is_exact_and_providers_do_not_declare_services() {
    let daemon = read_repo_file("nixos-modules/host-daemon.nix");
    let broker = read_repo_file("nixos-modules/host-broker.nix");
    let services: std::collections::BTreeSet<_> = [
        fixed_unit_keys(&daemon, "services"),
        fixed_unit_keys(&broker, "services"),
    ]
    .concat()
    .into_iter()
    .collect();
    let sockets: std::collections::BTreeSet<_> =
        fixed_unit_keys(&broker, "sockets").into_iter().collect();

    assert_eq!(
        services,
        std::collections::BTreeSet::from(["d2bd".to_owned(), "d2b-priv-broker".to_owned()])
    );
    assert_eq!(
        sockets,
        std::collections::BTreeSet::from(["d2b-priv-broker".to_owned()])
    );

    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
        ])
        .arg("packages/d2b-provider-*")
        .output()
        .expect("enumerate Provider files");
    assert!(output.status.success(), "git ls-files failed");
    let mut violations = Vec::new();
    for path in String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter(|path| path.ends_with(".rs") || path.ends_with(".nix") || path.ends_with(".sh"))
    {
        let content = std::fs::read_to_string(repo_root().join(path))
            .unwrap_or_else(|error| panic!("read Provider file {path}: {error}"));
        for (line_number, line) in content.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//")
                || trimmed.starts_with('#')
                || trimmed.starts_with("/*")
                || trimmed.starts_with('*')
            {
                continue;
            }
            if trimmed.contains("systemd.services")
                || trimmed.contains("systemd.sockets")
                || trimmed.contains("systemctl enable")
            {
                violations.push(format!("{path}:{}: {line}", line_number + 1));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "Provider packages must not own persistent systemd services:\n{}",
        violations.join("\n")
    );
}
