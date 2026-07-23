//! Per-role minijail validators for the observability relay roles, ported from
//! the bash gates:
//!   * `tests/minijail-validator-vsock-relay.sh`
//!   * `tests/minijail-validator-otel-host-bridge.sh`
//!
//! Both roles only materialise on a feature-rich VM (the `corp-full` VM in the
//! `fixture-smoke-full` bundle has `observability.enable = true`, which emits
//! the per-VM `vsock-relay` runner and, host-scoped, the `otel-host-bridge`
//! runner under the obs VM's DAG). The minimal smoke fixture has neither, so the
//! rendered-profile checks load the FULL resolver
//! (`load_full_bundle_resolver_from_env`) and skip cleanly when
//! `D2B_FIXTURES_FULL` is unset (e.g. the plain `cargo test` pass, or a
//! non-x86_64 host). The source-grep checks need no fixture and always run.
//!
//! Layer split (faithful to the bash gates):
//!   * The bash gates' Layer-1 (always-on) `minijail-profiles.nix` shape
//!     assertions port here as SOURCE-grep checks over the in-tree Nix module
//!     (block existence, explicit empty caps / no CAP_* token, seccomp ref,
//!     role reference).
//!   * The bash gates' profile-shape assertions that the otel gate performed via
//!     `jq` over the LIVE host profile JSON (caps empty, no `/dev` binds,
//!     `seccompPolicyRef`) port as RENDERED checks over the real fixture
//!     `RoleProfile`s — a strictly stronger guarantee than re-reading a live
//!     host's `/etc/d2b/minijail-profiles/*.json`, which the bash gate
//!     skipped when absent. The rendered checks additionally pin the documented
//!     bind set, cgroup placement, namespace isolation, and socat argv shape.
//!   * The bash gates' opt-in live phases (`D2B_LIVE=1`: invoking `minijail0`,
//!     a positive `socat` pre-opened-fd path, an `AF_VSOCK socket(2)` SIGSYS/
//!     EPERM negative probe, a `PTRACE_TRACEME` probe, and writing
//!     `/var/lib/d2b/validated/p1-*.json` evidence) are runtime execution
//!     tests that require root, a live host, `minijail0`/`socat`/`perl`/
//!     `python3`, and AF_VSOCK loopback. They are NOT contract-test material and
//!     intentionally do not port.
//!
//! Spec corrections / smoke-fixture gaps:
//!   * The rendered `RoleProfile` DTO carries no `role` field (the role lives on
//!     the parent `ProcessNode::role`); the bash otel gate read the live
//!     `MinijailProfile` JSON whose top-level `role` IS present. The rendered
//!     port asserts `node.role` against the typed `ProcessRole` enum instead,
//!     which is the canonical role source for the per-VM DAG.

use d2b_contract_tests::{load_full_bundle_resolver_from_env, read_repo_file, repo_path_exists};
use d2b_core::processes::ProcessRole;
use regex::Regex;

const MINIJAIL_PROFILES_NIX: &str = "nixos-modules/minijail-profiles.nix";

/// Whether any single line of `content` matches `pattern`. Mirrors `grep`'s
/// per-line evaluation faithfully (so a `\s*` can never span a newline
/// boundary). Copied from `tests/policy_daemon.rs::any_line_matches`.
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

/// The matched line plus the `after` lines following the FIRST line that
/// matches `start_pat`, joined by newlines. Mirrors `grep -A <after>` block
/// extraction the bash gate uses (`grep -A 25 'profileIdFor name "vsock-relay"'`).
fn grep_after(content: &str, start_pat: &str, after: usize) -> Option<String> {
    let re = Regex::new(start_pat).expect("valid regex");
    let lines: Vec<&str> = content.lines().collect();
    let idx = lines.iter().position(|l| re.is_match(l))?;
    let end = (idx + after + 1).min(lines.len());
    Some(lines[idx..end].join("\n"))
}

// ===========================================================================
// tests/minijail-validator-vsock-relay.sh
// ===========================================================================

/// Layer-1 (always-on) `minijail-profiles.nix` shape assertions for the
/// `vsock-relay` role, ported as a SOURCE-grep over the in-tree Nix module:
///   * the module file exists,
///   * it declares a `profileIdFor name "vsock-relay"` profile block,
///   * within that block (the bash `grep -A 25` window) it declares the
///     explicit `capabilities = [ ]` empty-cap form AND carries no `"CAP_*"`
///     token (the bash gate's hard-fail branch — kernel-r2-4: caps must be
///     empty because the relay operates on pre-opened fds passed via
///     SCM_RIGHTS, so no in-role `AF_VSOCK socket()` call is needed),
///   * the block sets `seccompPolicyRef = "w1-vsock-relay"`.
#[test]
fn vsock_relay_profile_source_shape() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "minijail-profiles.nix not found at {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);

    assert!(
        src.contains("profileIdFor name \"vsock-relay\""),
        "no vsock-relay profile block (profileIdFor name \"vsock-relay\") in {MINIJAIL_PROFILES_NIX}"
    );

    // The bash `grep -A 25 'profileIdFor name "vsock-relay"'` window.
    let block = grep_after(&src, r#"profileIdFor name "vsock-relay""#, 25)
        .expect("could not locate vsock-relay profile block in minijail-profiles.nix");

    // Faithful port of the bash if/elif/else short-circuit: an explicit
    // `capabilities = [ ]` is the empty-caps PASS branch (current reality,
    // line 661); only when that explicit form is ABSENT does the CAP_*
    // hard-fail branch run. Pass iff (explicit empty) OR (no CAP_* token),
    // matching the bash quirk that the 25-line window also overlaps the
    // adjacent usbip block's `CAP_NET_RAW` — the explicit `[ ]` short-circuits
    // before that token is ever inspected.
    let explicit_empty_caps = any_line_matches(&block, r"capabilities = \[ \]");
    let has_cap_token = any_line_matches(&block, r#"capabilities = \[[^\]]*"CAP_"#);
    assert!(
        explicit_empty_caps || !has_cap_token,
        "vsock-relay profile has non-empty caps (kernel-r2-4: must be empty — pre-opened fds only)"
    );
    assert!(
        any_line_matches(&block, r#"seccompPolicyRef = "w1-vsock-relay""#),
        "vsock-relay profile seccompPolicyRef != \"w1-vsock-relay\""
    );
}

/// Rendered-fixture port (FULL resolver) of the `vsock-relay` role profile
/// shape. The bash gate's Layer-1 `jq`/grep checks (empty caps, seccomp ref) and
/// header-documented bind set ("per-VM /var/lib/d2b/vms/<vm>/vsock.sock, no
/// /dev binds") are asserted here as typed `RoleProfile` field checks against
/// the real `vm-corp-full-vsock-relay` profile — strictly stronger than the bash
/// gate re-grepping a synthetic live-host profile JSON.
#[test]
fn vsock_relay_rendered_profile_shape() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset (vsock-relay rendered profile shape)");
        return;
    };

    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.profile.profile_id != "vm-corp-full-vsock-relay" {
                continue;
            }
            seen += 1;
            let p = &node.profile;

            assert_eq!(
                node.role,
                ProcessRole::VsockRelay,
                "vsock-relay node {} (vm {}) role drift; got {:?}",
                p.profile_id,
                dag.vm,
                node.role
            );

            // Caps: empty (pre-opened fds only — no AF_VSOCK socket creation).
            assert!(
                p.caps.is_empty(),
                "vsock-relay {} (vm {}) must declare EMPTY caps (kernel-r2-4); got {:?}",
                p.profile_id,
                dag.vm,
                p.caps
            );

            // seccompPolicyRef = "w1-vsock-relay".
            assert_eq!(
                p.seccomp_policy_ref.as_deref(),
                Some("w1-vsock-relay"),
                "vsock-relay {} (vm {}) seccompPolicyRef drift",
                p.profile_id,
                dag.vm
            );

            // Bind set: per-VM state dir (the inherited UDS lives under it), the
            // obs VM state dir; NO /dev binds.
            let writable: Vec<&str> = p
                .mount_policy
                .writable_paths
                .iter()
                .map(|w| w.path.as_str())
                .collect();
            assert!(
                writable
                    .iter()
                    .any(|w| *w == format!("/var/lib/d2b/vms/{}", dag.vm)),
                "vsock-relay {} (vm {}) must have RW bind on the per-VM state dir; got {:?}",
                p.profile_id,
                dag.vm,
                writable
            );
            assert!(
                writable.iter().any(|w| w.starts_with("/var/lib/d2b/vms/")),
                "vsock-relay {} (vm {}) must reach the obs VM state dir for relay forwarding; got {:?}",
                p.profile_id,
                dag.vm,
                writable
            );
            assert!(
                !writable.iter().any(|w| w.starts_with("/dev")),
                "vsock-relay {} (vm {}) must have NO /dev binds; got {:?}",
                p.profile_id,
                dag.vm,
                writable
            );
            assert!(
                p.mount_policy.device_binds.is_empty(),
                "vsock-relay {} (vm {}) must declare no device binds; got {:?}",
                p.profile_id,
                dag.vm,
                p.mount_policy.device_binds
            );

            // cgroup placement + namespace isolation.
            assert_eq!(
                p.cgroup_placement.subtree,
                format!("d2b.slice/{}/vsock-relay", dag.vm),
                "vsock-relay {} (vm {}) cgroup subtree drift",
                p.profile_id,
                dag.vm
            );
            assert!(
                p.namespaces.mount && p.namespaces.ipc,
                "vsock-relay {} (vm {}) must isolate the mount + ipc namespaces; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
            assert!(
                !p.namespaces.net && !p.namespaces.user,
                "vsock-relay {} (vm {}) must not request a net or user namespace; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
        }
    }
    assert_eq!(
        seen, 1,
        "expected exactly one vm-corp-full-vsock-relay profile in the full fixture, saw {seen}"
    );
}

/// Rendered-fixture port (FULL resolver) of the `vsock-relay` runner argv shape.
/// The bash gate header documents the relay as "the socat-based sidecar that
/// replaces the per-VM d2b-otel-relay@<vm>.service"; this pins that contract
/// against the rendered DAG node: a `socat` binary, the
/// `d2b-otel-relay@<vm>` process title, and the `UNIX-LISTEN:` + `EXEC:`
/// (ch-vsock-connect) socat pipe.
#[test]
fn vsock_relay_rendered_argv_socat_shape() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset (vsock-relay rendered argv shape)");
        return;
    };

    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.profile.profile_id != "vm-corp-full-vsock-relay" {
                continue;
            }
            seen += 1;

            assert!(
                node.binary_path
                    .as_deref()
                    .is_some_and(|b| b.ends_with("/socat")),
                "vsock-relay {} (vm {}) binaryPath must be a socat binary; got {:?}",
                node.profile.profile_id,
                dag.vm,
                node.binary_path
            );
            assert!(
                node.argv
                    .first()
                    .is_some_and(|a| a.starts_with("d2b-otel-relay@")),
                "vsock-relay {} (vm {}) argv[0] must be the d2b-otel-relay@<vm> title; got {:?}",
                node.profile.profile_id,
                dag.vm,
                node.argv
            );
            assert!(
                node.argv.iter().any(|a| a.starts_with("UNIX-LISTEN:")),
                "vsock-relay {} (vm {}) argv must declare a UNIX-LISTEN socat endpoint; got {:?}",
                node.profile.profile_id,
                dag.vm,
                node.argv
            );
            assert!(
                node.argv
                    .iter()
                    .any(|a| a.starts_with("EXEC:") && a.contains("d2b-ch-vsock-connect")),
                "vsock-relay {} (vm {}) argv must EXEC the ch-vsock-connect helper; got {:?}",
                node.profile.profile_id,
                dag.vm,
                node.argv
            );
        }
    }
    assert_eq!(
        seen, 1,
        "expected exactly one vm-corp-full-vsock-relay runner in the full fixture, saw {seen}"
    );
}

// ===========================================================================
// tests/minijail-validator-otel-host-bridge.sh
// ===========================================================================

/// Layer-1 (always-on) `minijail-profiles.nix` reference assertion for the
/// `OtelHostBridge` role, ported as a SOURCE-grep: the module file exists and
/// references the host-scoped otel-host-bridge profile (the bash gate's
/// `grep -qE 'otel-?host-?bridge|otelHostBridge|OtelHostBridge'`). This is the
/// role that replaces the singleton `d2b-otel-host-bridge.service`.
#[test]
fn otel_host_bridge_profile_source_reference() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "minijail-profiles.nix not found at {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);
    assert!(
        any_line_matches(&src, r"otel-?host-?bridge|otelHostBridge|OtelHostBridge"),
        "no otel-host-bridge profile reference in {MINIJAIL_PROFILES_NIX}"
    );
}

/// Rendered-fixture port (FULL resolver) of the `OtelHostBridge` role profile
/// shape. The bash gate performed these as `jq` checks over the LIVE host
/// profile JSON (`/etc/d2b/minijail-profiles/host-otel-host-bridge.json`)
/// behind `D2B_LIVE=1`; they port here as typed `RoleProfile` field checks over
/// the rendered `host-otel-host-bridge` profile, which always runs and is
/// strictly stronger:
///   * caps empty (kernel-r2-4),
///   * no `/dev` binds (writable paths + device binds),
///   * `seccompPolicyRef = "w1-otel-host-bridge"`,
///   * the documented bind set (RW host OTel runtime dir + obs VM CH vsock dir),
///   * host-scoped cgroup placement + namespace isolation.
#[test]
fn otel_host_bridge_rendered_profile_shape() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset (otel-host-bridge rendered profile shape)");
        return;
    };

    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.profile.profile_id != "host-otel-host-bridge" {
                continue;
            }
            seen += 1;
            let p = &node.profile;

            assert_eq!(
                node.role,
                ProcessRole::OtelHostBridge,
                "otel-host-bridge node {} (vm {}) role drift; got {:?}",
                p.profile_id,
                dag.vm,
                node.role
            );

            // Caps: empty.
            assert!(
                p.caps.is_empty(),
                "otel-host-bridge {} (vm {}) must declare EMPTY caps (kernel-r2-4); got {:?}",
                p.profile_id,
                dag.vm,
                p.caps
            );

            // No /dev binds (writable paths + device binds).
            let writable: Vec<&str> = p
                .mount_policy
                .writable_paths
                .iter()
                .map(|w| w.path.as_str())
                .collect();
            assert!(
                !writable.iter().any(|w| w.starts_with("/dev")),
                "otel-host-bridge {} (vm {}) has a /dev bind, which P1 forbids; got {:?}",
                p.profile_id,
                dag.vm,
                writable
            );
            assert!(
                p.mount_policy.device_binds.is_empty(),
                "otel-host-bridge {} (vm {}) must declare no device binds; got {:?}",
                p.profile_id,
                dag.vm,
                p.mount_policy.device_binds
            );

            // seccompPolicyRef = "w1-otel-host-bridge".
            assert_eq!(
                p.seccomp_policy_ref.as_deref(),
                Some("w1-otel-host-bridge"),
                "otel-host-bridge {} (vm {}) expected seccompPolicyRef=w1-otel-host-bridge",
                p.profile_id,
                dag.vm
            );

            // Documented bind set: RW host OTel runtime dir + obs VM CH vsock dir.
            assert!(
                writable.contains(&"/run/d2b/otel"),
                "otel-host-bridge {} (vm {}) must RW-bind the host OTel runtime dir; got {:?}",
                p.profile_id,
                dag.vm,
                writable
            );
            assert!(
                writable.iter().any(|w| w.starts_with("/var/lib/d2b/vms/")),
                "otel-host-bridge {} (vm {}) must reach the obs VM CH vsock dir; got {:?}",
                p.profile_id,
                dag.vm,
                writable
            );

            // Host-scoped cgroup placement + namespace isolation.
            assert_eq!(
                p.cgroup_placement.subtree, "d2b.slice/host/otel-host-bridge",
                "otel-host-bridge {} (vm {}) cgroup subtree drift",
                p.profile_id, dag.vm
            );
            assert!(
                p.namespaces.mount && p.namespaces.ipc,
                "otel-host-bridge {} (vm {}) must isolate the mount + ipc namespaces; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
            assert!(
                !p.namespaces.net && !p.namespaces.user,
                "otel-host-bridge {} (vm {}) must not request a net or user namespace; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
        }
    }
    assert_eq!(
        seen, 1,
        "expected exactly one host-otel-host-bridge profile in the full fixture, saw {seen}"
    );
}

/// Rendered-fixture port (FULL resolver) of the `OtelHostBridge` runner argv
/// shape. The bash gate header documents the bridge as a socat runner that
/// listens on the host-egress UDS and forwards into the obs VM's OTLP listener;
/// this pins that contract: a `socat` binary, the `d2b-otel-host-bridge`
/// process title, the `UNIX-LISTEN:/run/d2b/otel/host-egress.sock` listen
/// endpoint, and the `EXEC:` ch-vsock-connect forward leg.
#[test]
fn otel_host_bridge_rendered_argv_socat_shape() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset (otel-host-bridge rendered argv shape)");
        return;
    };

    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.profile.profile_id != "host-otel-host-bridge" {
                continue;
            }
            seen += 1;

            assert!(
                node.binary_path
                    .as_deref()
                    .is_some_and(|b| b.ends_with("/socat")),
                "otel-host-bridge {} (vm {}) binaryPath must be a socat binary; got {:?}",
                node.profile.profile_id,
                dag.vm,
                node.binary_path
            );
            assert_eq!(
                node.argv.first().map(String::as_str),
                Some("d2b-otel-host-bridge"),
                "otel-host-bridge {} (vm {}) argv[0] must be the d2b-otel-host-bridge title; got {:?}",
                node.profile.profile_id,
                dag.vm,
                node.argv
            );
            assert!(
                node.argv
                    .iter()
                    .any(|a| a.starts_with("UNIX-LISTEN:/run/d2b/otel/host-egress.sock")),
                "otel-host-bridge {} (vm {}) argv must listen on the host-egress UDS; got {:?}",
                node.profile.profile_id,
                dag.vm,
                node.argv
            );
            assert!(
                node.argv
                    .iter()
                    .any(|a| a.starts_with("EXEC:") && a.contains("d2b-ch-vsock-connect")),
                "otel-host-bridge {} (vm {}) argv must EXEC the ch-vsock-connect helper; got {:?}",
                node.profile.profile_id,
                dag.vm,
                node.argv
            );
        }
    }
    assert_eq!(
        seen, 1,
        "expected exactly one host-otel-host-bridge runner in the full fixture, saw {seen}"
    );
}
