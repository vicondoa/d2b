//! Per-role minijail validators for the **audio** and **usbip** sidecar roles,
//! ported from the bash gates:
//!   * `tests/minijail-validator-audio.sh`
//!   * `tests/minijail-validator-usbip.sh`
//!
//! Both gates validate the RENDERED minijail role profiles plus a handful of
//! `nixos-modules/minijail-profiles.nix` source-shape pins, so they belong in
//! the fixture-contract layer (this crate, gated by the NL_FIXTURES /
//! NL_FIXTURES_FULL contract steps in `tests/tools/rust-workspace-checks.sh`), not the
//! doc/source-grep policy layer.
//!
//! The audio + usbip profiles are FEATURE-RICH: they only render on a VM with
//! `audio.enable` / `usbip.yubikey` (plus the env's auto-declared usbipd net
//! side). The default fixture-smoke bundle does NOT contain them, so the
//! rendered-profile checks use the feature-rich `fixture-smoke-full`
//! (`NL_FIXTURES_FULL`, the `corp-full` VM with graphics+video+audio+tpm+usbip+
//! observability) via [`load_full_bundle_resolver_from_env`]. When that fixture
//! is unset (the plain `cargo test` pass, or a non-x86_64 host), the rendered
//! tests early-return with a SKIP `eprintln`. The source-grep tests need no
//! fixture and always run.
//!
//! Rendered nodes present in `fixture-smoke-full`'s `processes.json`:
//!   * `corp-full` -> `vm-corp-full-audio`        (role `audio`)
//!   * `sys-work-usbipd` -> `vm-sys-work-usbipd-backend` (role `usbip`)
//!   * `sys-work-usbipd` -> `vm-sys-work-usbipd-proxy`   (role `usbip`)
//!
//! The per-VM `vm-corp-full-usbip` profile (rendered into the
//! `minijail-profiles.nix` profile TABLE when `usbip.yubikey` is set) does NOT
//! emit its own DAG node in `processes.json` — the per-busid attach runs on the
//! env's auto-declared `sys-work-usbipd` net side. The bash gate's `role =
//! "usbip"` / `capabilities = [ "CAP_NET_RAW" ]` greps cover BOTH the per-VM
//! `vm-<name>-usbip` block (the corp-full side) AND the usbipd backend/proxy
//! blocks, all in the same source file; so the corp-full usbip coverage is
//! preserved here as SOURCE-grep, and the rendered checks cover the two usbipd
//! nodes. (This matches "check both corp-full AND the sys-work usbipd nodes as
//! the bash gate does": corp-full via source, usbipd via rendered.)
//!
//! Layer split (faithful to the bash gates):
//!   * Each gate's Layer-1 (always-on) `minijail-profiles.nix` shape assertions
//!     port here, either as RENDERED checks over the real fixture RoleProfiles
//!     (a strictly stronger guarantee than the bash `grep`/`awk` block scan) or,
//!     where the assertion is a source-text pin, as a per-line `grep`-faithful
//!     regex over the in-tree Nix module.
//!   * Each gate's opt-in `NL_LIVE=1` Layer-2 phases (spawning `minijail0` +
//!     `vhost-device-sound` / `usbip version`, the SYS_ptrace -> SIGSYS probe,
//!     optional busid bind/unbind, and writing
//!     `/var/lib/nixling/validated/p1-{audio,usbip}.json` evidence) are runtime
//!     execution tests that require root, a live host, hardware (PipeWire /
//!     usbip-host module), and the role binaries. They are NOT contract-test
//!     material and intentionally do not port. The audio gate's negative-path
//!     seccomp closure is grounded at Layer-1 by the `seccompPolicyRef =
//!     "w1-audio"` pin (asserted both in source and rendered); the usbip gate's
//!     negative-path closure is grounded by the `seccompPolicyRef = "w1-usbip"`
//!     pin (the bash gate's own "negative (layer-1)" equivalent).
//!
//! Spec corrections / smoke-fixture gaps:
//!   * None. Every Layer-1 (always-on) assertion in both bash gates ports
//!     faithfully; the only skipped coverage is the documented `NL_LIVE=1`
//!     runtime-exec / evidence-write phases above.
//!   * The bash gates' "skip when the binary is absent" preflights
//!     (`vhost-device-sound`/`minijail0`/`python3` for audio; `usbip`/`cc` for
//!     usbip) are build-host gates that set `positive_ok=1` and SKIP when the
//!     tool is missing — they are not contract assertions and do not port.

use nixling_contract_tests::{
    load_full_bundle_resolver_from_env, read_repo_file, repo_path_exists,
};
use nixling_core::processes::ProcessRole;
use regex::Regex;

const MINIJAIL_PROFILES_NIX: &str = "nixos-modules/minijail-profiles.nix";

/// Whether any single line of `content` matches `pattern`. Mirrors `grep`'s
/// per-line evaluation faithfully (so a `\s*` can never span a newline
/// boundary). Copied from `tests/policy_daemon.rs::any_line_matches`.
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

/// Extract the inclusive line range from the first line matching `start_pat`
/// through the first subsequent line matching `end_pat`. Mirrors the audio bash
/// gate's `awk '/profileIdFor name "audio"/{inblock=1} inblock{print} inblock &&
/// /^[[:space:]]*};[[:space:]]*$/{exit}'` block extraction (the first `};` line
/// terminates the block — for the audio profile that is the `userNamespace`
/// closer, which still encloses every checked token). Copied from
/// `tests/minijail_roles.rs::extract_block`.
fn extract_block(content: &str, start_pat: &str, end_pat: &str) -> Option<String> {
    let start_re = Regex::new(start_pat).expect("valid start regex");
    let end_re = Regex::new(end_pat).expect("valid end regex");
    let mut active = false;
    let mut block: Vec<&str> = Vec::new();
    for line in content.lines() {
        if !active && start_re.is_match(line) {
            active = true;
        }
        if active {
            block.push(line);
            if end_re.is_match(line) {
                return Some(block.join("\n"));
            }
        }
    }
    None
}

// ===========================================================================
// tests/minijail-validator-audio.sh
// ===========================================================================

/// Layer-1 (always-on) `minijail-profiles.nix` shape assertions for the audio
/// role, ported from the bash gate's unconditional block (lines 42-83):
///   * `minijail-profiles.nix` exists (else "not found" die);
///   * the file has an `profileIdFor name "audio"` block (else "no audio
///     profile block" die);
///   * the extracted audio block keeps host capabilities EMPTY — it must NOT
///     declare `capabilities =` (mkProfile defaults to `[]`);
///   * `seccompPolicyRef = "w1-audio"`;
///   * `namespaces = defaultNamespaces // { net = true; }` (private net NS);
///   * `userNamespace = {` (broker-pre user NS, ADR 0021).
#[test]
fn audio_profile_source_layer1_shape() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "minijail-profiles.nix not found at {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);

    // The audio profile block must be present.
    assert!(
        any_line_matches(&src, r#"profileIdFor name "audio""#),
        "no audio profile block in {MINIJAIL_PROFILES_NIX}"
    );

    // Extract the audio block: from the first `profileIdFor name "audio"` line
    // through the first `};`-only line (the bash awk block extraction).
    let block = extract_block(&src, r#"profileIdFor name "audio""#, r"^\s*};\s*$")
        .expect("could not locate audio profile block in minijail-profiles.nix");

    // Host capabilities must stay empty: the block must NOT declare
    // `capabilities =` (mkProfile's default `[]` is the contract).
    assert!(
        !any_line_matches(&block, r"capabilities\s*="),
        "audio profile declares host capabilities; expected mkProfile default []"
    );

    // Closed-set seccomp policy reference.
    assert!(
        any_line_matches(&block, r#"seccompPolicyRef\s*=\s*"w1-audio""#),
        r#"audio profile seccompPolicyRef != "w1-audio""#
    );

    // Private net namespace (CAP_NET_RAW becomes effective only inside the
    // broker-pre user-NS-owned net NS).
    assert!(
        any_line_matches(
            &block,
            r"namespaces\s*=\s*defaultNamespaces\s*//\s*\{\s*net\s*=\s*true;\s*\}"
        ),
        "audio profile missing namespaces = defaultNamespaces // {{ net = true; }}"
    );

    // Broker-pre user namespace declaration (ADR 0021).
    assert!(
        any_line_matches(&block, r"userNamespace\s*=\s*\{"),
        "audio profile missing userNamespace"
    );
}

/// Layer-1 applied to the REAL rendered fixture RoleProfile (stronger than the
/// bash gate, which only `grep`ed the `.nix` source): the rendered
/// `vm-corp-full-audio` profile MUST carry the broker-pre-NS audio shape —
/// EMPTY host caps, `seccompPolicyRef = "w1-audio"`, a private net namespace
/// combined with a user namespace, a single-entry `userNamespace` mapping in-NS
/// 0 to the `nixling-<vm>-snd` principal's stable ephemeral uid/gid (never the
/// unmapped 0), `umask = 0o007` for the shared `snd.sock`, and the
/// `nixling.slice/<vm>/audio` cgroup subtree.
///
/// Skips when `NL_FIXTURES_FULL` is unset (the audio profile only renders on the
/// feature-rich fixture).
#[test]
fn audio_rendered_profile_broker_pre_ns_shape() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: NL_FIXTURES_FULL unset; audio profile only renders on fixture-smoke-full");
        return;
    };
    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::Audio {
                continue;
            }
            seen += 1;
            let p = &node.profile;

            // EMPTY host caps (mkProfile default []); CAP_NET_RAW is effective
            // only inside the broker-pre user/net NS.
            assert!(
                p.caps.is_empty(),
                "audio {} (vm {}) must declare EMPTY host capabilities; got {:?}",
                p.profile_id,
                dag.vm,
                p.caps
            );

            // Closed-set seccomp reference.
            assert_eq!(
                p.seccomp_policy_ref.as_deref(),
                Some("w1-audio"),
                "audio {} (vm {}) seccompPolicyRef drift",
                p.profile_id,
                dag.vm
            );

            // Private net namespace + user namespace (the ADR-0021 broker-pre
            // user-NS-owned net NS). ipc + mount stay isolated; pid + uts stay
            // shared (defaultNamespaces).
            assert!(
                p.namespaces.net && p.namespaces.user,
                "audio {} (vm {}) must isolate the net + user namespaces; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
            assert!(
                p.namespaces.ipc && p.namespaces.mount,
                "audio {} (vm {}) must isolate the ipc + mount namespaces; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
            assert!(
                !p.namespaces.pid && !p.namespaces.uts,
                "audio {} (vm {}) must NOT isolate the pid/uts namespaces (defaultNamespaces); got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );

            // Single-entry user-NS mapping: in-NS 0 -> the snd principal's
            // stable ephemeral uid/gid (== the profile uid/gid), never 0.
            let user_ns = p.user_namespace.as_ref().unwrap_or_else(|| {
                panic!(
                    "audio {} (vm {}) missing userNamespace (ADR 0021 broker-pre-NS)",
                    p.profile_id, dag.vm
                )
            });
            assert_ne!(
                user_ns.host_uid_for_zero, 0,
                "audio {} (vm {}) userNamespace.hostUidForZero must map to the snd principal uid, not 0",
                p.profile_id, dag.vm
            );
            assert_eq!(
                user_ns.host_uid_for_zero, user_ns.host_gid_for_zero,
                "audio {} (vm {}) userNamespace uid/gid must both map to the snd principal id",
                p.profile_id, dag.vm
            );
            assert_eq!(
                user_ns.host_uid_for_zero, p.uid,
                "audio {} (vm {}) userNamespace.hostUidForZero must equal the profile uid (snd principal)",
                p.profile_id, dag.vm
            );
            assert_eq!(
                p.uid, p.gid,
                "audio {} (vm {}) uid/gid must both be the snd principal id",
                p.profile_id, dag.vm
            );

            // umask 0o007 so the bound snd.sock is mode 0660 (the per-VM runtime
            // default ACL then makes cloud-hypervisor's named-user entry
            // effective).
            assert_eq!(
                p.umask,
                Some(0o007),
                "audio {} (vm {}) must declare umask 0o007 for the shared snd.sock; got {:?}",
                p.profile_id,
                dag.vm,
                p.umask
            );

            // cgroup placement.
            assert_eq!(
                p.cgroup_placement.subtree,
                format!("nixling.slice/{}/audio", dag.vm),
                "audio {} (vm {}) cgroup subtree drift",
                p.profile_id,
                dag.vm
            );
        }
    }
    assert!(
        seen > 0,
        "fixture-smoke-full has no audio node — corp-full enables audio (regression)"
    );
}

// ===========================================================================
// tests/minijail-validator-usbip.sh
// ===========================================================================

/// Layer-1 (always-on) `minijail-profiles.nix` shape pins for the usbip role,
/// ported from the bash gate's unconditional block (lines 56-69) plus its
/// "negative (layer-1)" seccomp pin (lines 188-191):
///   * the usbip role is declared (`role = "usbip"`); else "usbip role
///     missing" fail;
///   * the usbip profile declares `capabilities = [ "CAP_NET_RAW" ]`
///     (kernel-r2-4 cap matrix); else fail;
///   * `seccompPolicyRef = "w1-usbip"` (the closed allowlist the broker loads;
///     ptrace is NOT in it — the negative-path Layer-1 equivalent); else fail.
///
/// These greps are file-scoped (the bash gate's `grep -q` over the whole file),
/// so they are satisfied by the per-VM `vm-<name>-usbip` block (the corp-full
/// side, which emits no DAG node) AND the usbipd backend block. This is the
/// corp-full half of "check both corp-full AND the sys-work usbipd nodes".
#[test]
fn usbip_profile_source_layer1_shape() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "usbip: {MINIJAIL_PROFILES_NIX} missing"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);

    assert!(
        any_line_matches(&src, r#"role\s*=\s*"usbip""#),
        "usbip role missing from {MINIJAIL_PROFILES_NIX}"
    );
    assert!(
        any_line_matches(&src, r#"capabilities\s*=\s*\[\s*"CAP_NET_RAW"\s*\]"#),
        r#"usbip profile must declare capabilities = [ "CAP_NET_RAW" ] (kernel-r2-4)"#
    );
    assert!(
        any_line_matches(&src, r#"seccompPolicyRef\s*=\s*"w1-usbip""#),
        r#"usbip profile must declare seccompPolicyRef = "w1-usbip" (ptrace not in allowlist)"#
    );
}

/// Layer-1 applied to the REAL rendered fixture RoleProfiles (stronger than the
/// bash gate's source greps): the env's auto-declared `sys-work-usbipd` net VM
/// emits the two usbip DAG nodes the per-busid attach runs under —
/// `vm-sys-work-usbipd-backend` and `vm-sys-work-usbipd-proxy`. The backend
/// (host-root usbipd write side) MUST carry exactly `["CAP_NET_RAW"]`,
/// `seccompPolicyRef = "w1-usbip"`, uid/gid 0 with the documented root
/// `adr_carve_out`, and a private pid namespace; the proxy MUST carry EMPTY caps,
/// `seccompPolicyRef = "w1-usbip-proxy"`, and a non-root principal uid. Neither
/// requests a net or user namespace. cgroup placement is
/// `nixling.slice/sys-work-usbipd/{backend,proxy}`.
///
/// Skips when `NL_FIXTURES_FULL` is unset (usbip only renders when an env has a
/// `usbip.yubikey` VM, which the default fixture lacks).
#[test]
fn usbip_rendered_usbipd_backend_proxy_shape() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: NL_FIXTURES_FULL unset; usbipd nodes only render on fixture-smoke-full");
        return;
    };
    let mut backend_seen = 0usize;
    let mut proxy_seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::Usbip {
                continue;
            }
            let p = &node.profile;

            // Common to every rendered usbip node: no net / no user namespace
            // (CAP_NET_RAW is a host cap here, not a user-NS fake-root).
            assert!(
                !p.namespaces.net && !p.namespaces.user,
                "usbip {} (vm {}) must NOT request a net or user namespace; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
            assert!(
                p.namespaces.ipc && p.namespaces.mount,
                "usbip {} (vm {}) must isolate the ipc + mount namespaces; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
            assert!(
                p.user_namespace.is_none(),
                "usbip {} (vm {}) must NOT declare a broker-pre userNamespace; got {:?}",
                p.profile_id,
                dag.vm,
                p.user_namespace
            );

            if p.profile_id.ends_with("-usbipd-backend") {
                backend_seen += 1;

                // kernel-r2-4 cap matrix: backend = CAP_NET_RAW only.
                assert_eq!(
                    p.caps,
                    vec!["CAP_NET_RAW".to_string()],
                    "usbip backend {} (vm {}) caps drift; expected [CAP_NET_RAW], got {:?}",
                    p.profile_id,
                    dag.vm,
                    p.caps
                );
                assert_eq!(
                    p.seccomp_policy_ref.as_deref(),
                    Some("w1-usbip"),
                    "usbip backend {} (vm {}) seccompPolicyRef drift",
                    p.profile_id,
                    dag.vm
                );
                // Host-root usbipd write side (documented carve-out).
                assert_eq!(
                    p.uid, 0,
                    "usbip backend {} (vm {}) must run as host root (usbip_sockfd write)",
                    p.profile_id, dag.vm
                );
                assert_eq!(
                    p.gid, 0,
                    "usbip backend {} (vm {}) must run as host gid 0",
                    p.profile_id, dag.vm
                );
                let carve = p.adr_carve_out.as_deref().unwrap_or("");
                assert!(
                    carve.contains("usbip_sockfd"),
                    "usbip backend {} (vm {}) must document the host-root usbip_sockfd carve-out; got {:?}",
                    p.profile_id,
                    dag.vm,
                    p.adr_carve_out
                );
                // Private pid namespace (defaultNamespaces // { pid = true; }).
                assert!(
                    p.namespaces.pid,
                    "usbip backend {} (vm {}) must isolate the pid namespace; got {:?}",
                    p.profile_id, dag.vm, p.namespaces
                );
                assert_eq!(
                    p.cgroup_placement.subtree,
                    format!("nixling.slice/{}/backend", dag.vm),
                    "usbip backend {} (vm {}) cgroup subtree drift",
                    p.profile_id,
                    dag.vm
                );
            } else if p.profile_id.ends_with("-usbipd-proxy") {
                proxy_seen += 1;

                // Proxy holds no host caps (pre-opened fds / proxy bind only).
                assert!(
                    p.caps.is_empty(),
                    "usbip proxy {} (vm {}) must declare EMPTY host caps; got {:?}",
                    p.profile_id,
                    dag.vm,
                    p.caps
                );
                assert_eq!(
                    p.seccomp_policy_ref.as_deref(),
                    Some("w1-usbip-proxy"),
                    "usbip proxy {} (vm {}) seccompPolicyRef drift",
                    p.profile_id,
                    dag.vm
                );
                // Non-root principal uid (the proxy runs as the per-VM proxy
                // principal, not root).
                assert_ne!(
                    p.uid, 0,
                    "usbip proxy {} (vm {}) must run as a non-root principal, not uid 0",
                    p.profile_id, dag.vm
                );
                assert_eq!(
                    p.cgroup_placement.subtree,
                    format!("nixling.slice/{}/proxy", dag.vm),
                    "usbip proxy {} (vm {}) cgroup subtree drift",
                    p.profile_id,
                    dag.vm
                );
            }
        }
    }
    assert_eq!(
        backend_seen, 1,
        "fixture-smoke-full must render exactly one usbipd-backend node; saw {backend_seen}"
    );
    assert_eq!(
        proxy_seen, 1,
        "fixture-smoke-full must render exactly one usbipd-proxy node; saw {proxy_seen}"
    );
}
