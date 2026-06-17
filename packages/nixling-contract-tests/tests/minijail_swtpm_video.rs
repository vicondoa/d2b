//! Per-role minijail validators for the swtpm + video sidecars, ported from
//! the bash gates:
//!   * `tests/minijail-validator-swtpm.sh`
//!   * `tests/minijail-validator-video.sh`
//!
//! These gates validate the RENDERED minijail role profiles for two
//! feature-enabled sidecars (per-VM swtpm 2.0 + the daemon-spawned
//! vhost-user-media video decoder), so they belong in the fixture-contract
//! layer (this crate, gated by the NL_FIXTURES / NL_FIXTURES_FULL contract
//! step in `tests/tools/rust-workspace-checks.sh`), not the doc/source-grep policy
//! layer.
//!
//! Fixture split (KEY): swtpm + video profiles only render on a VM that
//! enables tpm + graphics/video. The MINIMAL `fixture-smoke` bundle has
//! neither, so every RENDERED check here uses the feature-rich
//! `fixture-smoke-full` resolver (NL_FIXTURES_FULL; the `corp-full` VM emits
//! `vm-corp-full-swtpm`, `vm-corp-full-swtpm-flush`, and `vm-corp-full-video`).
//! That resolver is `None` when NL_FIXTURES_FULL is unset (a plain `cargo
//! test` pass, or a non-x86_64 host where the graphics platform gate makes the
//! fixture unavailable); rendered tests early-return with a structured SKIP in
//! that case. SOURCE-grep checks need no fixture and always run.
//!
//! Layer split (faithful to the bash gates):
//!   * The bash gates' always-on shape assertions (swtpm Phase-1, video
//!     Layer-1) port here, either as RENDERED checks over the real fixture
//!     RoleProfiles (a strictly stronger guarantee than the bash `jq`/grep
//!     over a synthetic re-eval) or, where the assertion needs config the
//!     single rendering cannot express (a conditional branch, a principal
//!     name, host-activation ACL plumbing), as a SOURCE-grep over the in-tree
//!     Nix module.
//!   * The bash gates' opt-in live phases (`NL_LIVE=1`) are runtime execution
//!     tests that require root, a live host, `minijail0`, `swtpm`/`tpm2-tools`,
//!     and write `/var/lib/nixling/validated/p1-*.json` evidence. They are NOT
//!     contract-test material and intentionally do not port. See the per-gate
//!     "Live phase NOT ported" notes below.
//!
//! Spec corrections / smoke-fixture gaps:
//!   * swtpm Phase-1 `pass_check "minijail-profiles.nix present"` is folded
//!     into the source tests (`read_repo_file` panics with a clear message if
//!     the module is absent, so a missing file fails rather than silently
//!     skips).
//!   * swtpm Phase-1 awk "swtpm/swtpm-flush declare no `capabilities` attr"
//!     asserted the SOURCE relies on the `mkProfile` empty-caps default. The
//!     rendered `caps == []` check here is strictly stronger (it grounds the
//!     actual kernel-r2-4 contract regardless of how the Nix expresses it), so
//!     the source-awk is subsumed, not dropped.
//!   * The swtpm/video Phase-1 cgroup-subtree intent documented in the bash
//!     gate headers (but only partially asserted in the bash body) ports as a
//!     rendered `cgroup_placement.subtree` field check.

use nixling_contract_tests::{
    load_full_bundle_resolver_from_env, read_repo_file, repo_path_exists,
};
use nixling_core::bundle_resolver::BundleResolver;
use nixling_core::processes::ProcessRole;
use regex::Regex;

const MINIJAIL_PROFILES_NIX: &str = "nixos-modules/minijail-profiles.nix";
const HOST_ACTIVATION_NIX: &str = "nixos-modules/host-activation.nix";

/// Whether any single line of `content` matches `pattern`. Mirrors `grep`'s
/// per-line evaluation faithfully (so a `\s*` can never span a newline
/// boundary). Copied from `tests/policy_daemon.rs::any_line_matches`.
fn any_line_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    content.lines().any(|line| re.is_match(line))
}

/// The matched line plus the following `n` lines, joined — the faithful port
/// of `grep -A <n> '<start_pat>'` over the first match. Returns `None` when no
/// line matches.
fn lines_after(content: &str, start_pat: &str, n: usize) -> Option<String> {
    let re = Regex::new(start_pat).expect("valid regex");
    let lines: Vec<&str> = content.lines().collect();
    let idx = lines.iter().position(|l| re.is_match(l))?;
    let end = (idx + 1 + n).min(lines.len());
    Some(lines[idx..end].join("\n"))
}

/// Load the feature-rich resolver or emit a structured SKIP and return `None`.
/// All rendered checks funnel through this so the skip wording is identical.
fn full_resolver_or_skip(test: &str) -> Option<BundleResolver> {
    match load_full_bundle_resolver_from_env() {
        Some(r) => Some(r),
        None => {
            eprintln!("SKIP {test}: NL_FIXTURES_FULL unset (feature-rich fixture unavailable)");
            None
        }
    }
}

// ===========================================================================
// tests/minijail-validator-swtpm.sh
//
// Phase 1 (eval-only, always-on) ports below. Phase 2 (NL_LIVE=1) is NOT
// ported: it boots `swtpm` under a hand-rolled `minijail0` profile against a
// tempdir state dir, writes a TPM 2.0 NVRAM index via `tpm2_nvdefine`/
// `tpm2_nvwrite`, restarts swtpm, reads the index back to prove byte-identical
// persistence across restart (the AGENTS.md critical-subsystem invariant for
// /var/lib/nixling/vms/<vm>/swtpm), probes an undeclared syscall (ptrace) for a
// SIGSYS seccomp kill, and writes /var/lib/nixling/validated/p1-swtpm.json
// evidence. That is a root + live-host + swtpm/tpm2-tools/minijail0 runtime
// layer, not contract-test material.
// ===========================================================================

/// swtpm Phase-1 S2: both the long-lived `swtpm` sidecar and the one-shot
/// `swtpm-flush` pre-start helper MUST carry an EMPTY host capability set
/// (plan kernel-r2-4). The bash awk asserted the source declares no
/// `capabilities` attr (relying on the `mkProfile` default); the rendered
/// `caps == []` check grounds the actual contract and is strictly stronger.
/// Also pins the closed `w1-swtpm` seccomp reference both profiles share.
#[test]
fn swtpm_rendered_empty_caps_both_profiles() {
    let Some(resolver) = full_resolver_or_skip("swtpm_rendered_empty_caps_both_profiles") else {
        return;
    };
    let mut swtpm_seen = 0usize;
    let mut flush_seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            let is_swtpm = node.role == ProcessRole::Swtpm;
            let is_flush = node.role == ProcessRole::SwtpmPreStartFlush;
            if !is_swtpm && !is_flush {
                continue;
            }
            let p = &node.profile;
            assert!(
                p.caps.is_empty(),
                "{} (vm {}) must declare EMPTY capabilities (kernel-r2-4); got {:?}",
                p.profile_id,
                dag.vm,
                p.caps
            );
            assert_eq!(
                p.seccomp_policy_ref.as_deref(),
                Some("w1-swtpm"),
                "{} (vm {}) seccompPolicyRef drift",
                p.profile_id,
                dag.vm
            );
            if is_swtpm {
                swtpm_seen += 1;
            } else {
                flush_seen += 1;
            }
        }
    }
    assert!(
        swtpm_seen > 0,
        "feature-rich fixture has no swtpm sidecar node (corp-full enables tpm — regression)"
    );
    assert!(
        flush_seen > 0,
        "feature-rich fixture has no swtpm-flush node (corp-full enables tpm — regression)"
    );
}

/// swtpm Phase-1 S3/S4: the long-lived `swtpm` sidecar MUST declare a
/// `userNamespace` block (ADR 0021 broker-pre-NS) mapping in-NS UID/GID 0 to
/// the swtpm principal's stable ephemeral UID, with a real user namespace
/// requested; the one-shot `swtpm-flush` wrapper MUST NOT (only the long-lived
/// sidecar gets the broker-pre-NS treatment). Rendered cross-check of S4: the
/// mapping targets the swtpm principal's OWN uid/gid (not 0, not the runner
/// principal). The source name-binding half of S4 is asserted in
/// `swtpm_source_principal_and_no_tmpfs`.
#[test]
fn swtpm_rendered_user_namespace_long_lived_sidecar_only() {
    let Some(resolver) =
        full_resolver_or_skip("swtpm_rendered_user_namespace_long_lived_sidecar_only")
    else {
        return;
    };
    let mut swtpm_seen = 0usize;
    let mut flush_seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            let p = &node.profile;
            match node.role {
                ProcessRole::Swtpm => {
                    swtpm_seen += 1;
                    let user_ns = p.user_namespace.as_ref().unwrap_or_else(|| {
                        panic!(
                            "swtpm sidecar {} (vm {}) MISSING userNamespace block — ADR 0021 \
                             broker-pre-NS not applied",
                            p.profile_id, dag.vm
                        )
                    });
                    assert!(
                        p.namespaces.user,
                        "swtpm sidecar {} (vm {}) must request a user namespace; got {:?}",
                        p.profile_id, dag.vm, p.namespaces
                    );
                    assert_eq!(
                        user_ns.host_uid_for_zero, p.uid,
                        "swtpm sidecar {} (vm {}) userNamespace.hostUidForZero must map in-NS \
                         uid 0 to the swtpm principal's own uid (not the runner principal)",
                        p.profile_id, dag.vm
                    );
                    assert_eq!(
                        user_ns.host_gid_for_zero, p.gid,
                        "swtpm sidecar {} (vm {}) userNamespace.hostGidForZero must map in-NS \
                         gid 0 to the swtpm principal's own gid",
                        p.profile_id, dag.vm
                    );
                    assert_ne!(
                        user_ns.host_uid_for_zero, 0,
                        "swtpm sidecar {} (vm {}) userNamespace must map to a real principal uid, \
                         not the unmapped 0",
                        p.profile_id, dag.vm
                    );
                }
                ProcessRole::SwtpmPreStartFlush => {
                    flush_seen += 1;
                    assert!(
                        p.user_namespace.is_none(),
                        "swtpm-flush {} (vm {}) MUST NOT carry a userNamespace block — only the \
                         long-lived sidecar gets broker-pre-NS (ADR 0021 swtpm-portion scope)",
                        p.profile_id,
                        dag.vm
                    );
                    assert!(
                        !p.namespaces.user,
                        "swtpm-flush {} (vm {}) MUST NOT request a user namespace; got {:?}",
                        p.profile_id, dag.vm, p.namespaces
                    );
                }
                _ => {}
            }
        }
    }
    assert!(
        swtpm_seen > 0,
        "feature-rich fixture has no swtpm sidecar node (regression)"
    );
    assert!(
        flush_seen > 0,
        "feature-rich fixture has no swtpm-flush node (regression)"
    );
}

/// swtpm Phase-1 S5/S6 + cgroup intent: the long-lived `swtpm` sidecar retains
/// `umask = 0o007` (the fu36 socket-ACL requirement, which must survive the
/// ADR-0021 userNamespace addition), and BOTH swtpm profiles bind the per-VM
/// persistent state dir `/var/lib/nixling/vms/<vm>/swtpm` as a writable path
/// (NOT tmpfs — tmpfs would silently lose the TPM NVRAM on every daemon
/// restart and force Entra/Intune re-enrollment). The defence-in-depth
/// negative (no tmpfs declaration) is a source check in
/// `swtpm_source_principal_and_no_tmpfs`. Also pins each profile's
/// `nixling.slice/<vm>/{swtpm,swtpm-flush}` cgroup subtree.
#[test]
fn swtpm_rendered_umask_state_dir_cgroup() {
    let Some(resolver) = full_resolver_or_skip("swtpm_rendered_umask_state_dir_cgroup") else {
        return;
    };
    let mut swtpm_seen = 0usize;
    let mut flush_seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            let p = &node.profile;
            let state_dir = format!("/var/lib/nixling/vms/{}/swtpm", dag.vm);
            let has_state_bind = p
                .mount_policy
                .writable_paths
                .iter()
                .any(|w| w.path == state_dir);
            match node.role {
                ProcessRole::Swtpm => {
                    swtpm_seen += 1;
                    assert_eq!(
                        p.umask,
                        Some(7),
                        "swtpm sidecar {} (vm {}) must retain umask = 0o007 (fu36 socket-ACL \
                         requirement preserved post-ADR-0021); got {:?}",
                        p.profile_id,
                        dag.vm,
                        p.umask
                    );
                    assert!(
                        has_state_bind,
                        "swtpm sidecar {} (vm {}) must bind {state_dir} as a writable path (no \
                         tmpfs); writablePaths = {:?}",
                        p.profile_id, dag.vm, p.mount_policy.writable_paths
                    );
                    assert_eq!(
                        p.cgroup_placement.subtree,
                        format!("nixling.slice/{}/swtpm", dag.vm),
                        "swtpm sidecar {} (vm {}) cgroup subtree drift",
                        p.profile_id,
                        dag.vm
                    );
                }
                ProcessRole::SwtpmPreStartFlush => {
                    flush_seen += 1;
                    assert!(
                        has_state_bind,
                        "swtpm-flush {} (vm {}) must bind {state_dir} as a writable path (no \
                         tmpfs); writablePaths = {:?}",
                        p.profile_id, dag.vm, p.mount_policy.writable_paths
                    );
                    assert_eq!(
                        p.cgroup_placement.subtree,
                        format!("nixling.slice/{}/swtpm-flush", dag.vm),
                        "swtpm-flush {} (vm {}) cgroup subtree drift",
                        p.profile_id,
                        dag.vm
                    );
                }
                _ => {}
            }
        }
    }
    assert!(
        swtpm_seen > 0,
        "feature-rich fixture has no swtpm sidecar node (regression)"
    );
    assert!(
        flush_seen > 0,
        "feature-rich fixture has no swtpm-flush node (regression)"
    );
}

/// swtpm Phase-1 S1/S4/S7 source half: the `swtpm` userNamespace must
/// reference the swtpm principal `nixling-${name}-swtpm` (NOT the runner
/// principal — S4 name-binding), and there must be NO tmpfs declaration for
/// swtpm state anywhere in `minijail-profiles.nix` (S7 defence-in-depth;
/// tmpfs forces Entra/Intune re-enrollment). The `read_repo_file` call also
/// covers S1 ("minijail-profiles.nix present") — it panics with a clear
/// message if the module is missing.
#[test]
fn swtpm_source_principal_and_no_tmpfs() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "missing {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);

    // S4 (source): swtpm userNamespace maps from the swtpm principal, asserted
    // verbatim as the bash `grep -q 'stablePrincipalId "nixling-${name}-swtpm"'`.
    assert!(
        src.contains(r#"stablePrincipalId "nixling-${name}-swtpm""#),
        "swtpm userNamespace must reference the swtpm principal \
         (stablePrincipalId \"nixling-${{name}}-swtpm\"), not the runner principal, in \
         {MINIJAIL_PROFILES_NIX}"
    );

    // S7 (source, negative): no non-comment line declares tmpfs for swtpm
    // state. Ports the bash regex
    // `^[[:space:]]*[^#/[:space:]].*(tmpfs.*swtpm|swtpm.*tmpfs)` per-line and
    // asserts zero matches (comment lines starting with # or / are excluded).
    let tmpfs_pat = r"^\s*[^#/\s].*(tmpfs.*swtpm|swtpm.*tmpfs)";
    assert!(
        !any_line_matches(&src, tmpfs_pat),
        "swtpm state appears to use tmpfs in {MINIJAIL_PROFILES_NIX} — REGRESSION; tmpfs loses \
         the TPM NVRAM on every daemon restart and forces Entra/Intune re-enrollment"
    );
}

// ===========================================================================
// tests/minijail-validator-video.sh
//
// Layer-1 (always-on) ports below. Layer-2 (NL_LIVE=1) is NOT ported: it
// pre-checks the live host's render node / cgroup leaf / cloud-hypervisor uid,
// runs a benign vhost-user-media bind probe under `minijail0` (positive path)
// and an undeclared-syscall (ptrace) probe expecting a SIGSYS seccomp kill
// (negative path), asserts the bound socket inherits the cloud-hypervisor uid
// ACL, and writes /var/lib/nixling/validated/p1-video.json evidence. That is a
// root + live-host + minijail0 runtime layer, not contract-test material.
// ===========================================================================

/// video Layer-1 V1/V2/V3/V4(device-mask)/V5: the rendered `video` role
/// profile MUST exist with an EMPTY capability set, `seccompPolicyRef =
/// "w1-video"`, masked device nodes (`hideDeviceNodesByDefault = true`)
/// exposing ONLY the default `/dev/dri/renderD128` allowlist (NVIDIA nodes are
/// gated behind `videoNvidiaDecode`, which the corp-full fixture leaves off —
/// so the rendered allowlist is exactly `["/dev/dri/renderD128"]`), a private
/// PID namespace, and `umask = 0o007` for CH socket-ACL inheritance. Also pins
/// the `nixling.slice/<vm>/video` cgroup leaf and confirms video uses a
/// DEDICATED principal distinct from the gpu principal (AGENTS: dedicated
/// `nixling-<vm>-video` principal, NOT gpu). The `videoNvidiaDecode` gating
/// CONDITIONAL (V4 opt-in branch) is a source check in
/// `video_source_nvidia_gating_namespace_principal`.
#[test]
fn video_rendered_profile_shape() {
    let Some(resolver) = full_resolver_or_skip("video_rendered_profile_shape") else {
        return;
    };
    let mut video_seen = 0usize;
    for dag in &resolver.processes.vms {
        // Collect gpu uids in this VM to prove the video principal is dedicated.
        let gpu_uids: Vec<u32> = dag
            .nodes
            .iter()
            .filter(|n| n.role == ProcessRole::Gpu || n.role == ProcessRole::GpuRenderNode)
            .map(|n| n.profile.uid)
            .collect();
        for node in &dag.nodes {
            if node.role != ProcessRole::Video {
                continue;
            }
            video_seen += 1;
            let p = &node.profile;
            assert!(
                p.profile_id.ends_with("-video"),
                "video profile id {} (vm {}) must end with -video",
                p.profile_id,
                dag.vm
            );
            assert!(
                p.caps.is_empty(),
                "video {} (vm {}) must declare EMPTY caps (kernel-r2-4); got {:?}",
                p.profile_id,
                dag.vm,
                p.caps
            );
            assert_eq!(
                p.seccomp_policy_ref.as_deref(),
                Some("w1-video"),
                "video {} (vm {}) seccompPolicyRef must be \"w1-video\"",
                p.profile_id,
                dag.vm
            );
            assert!(
                p.mount_policy.hide_device_nodes_by_default,
                "video {} (vm {}) must mask /dev (hideDeviceNodesByDefault = true)",
                p.profile_id, dag.vm
            );
            assert_eq!(
                p.mount_policy.device_binds,
                vec!["/dev/dri/renderD128".to_string()],
                "video {} (vm {}) device allowlist must be exactly [/dev/dri/renderD128] when \
                 videoNvidiaDecode is off (NVIDIA nodes gated behind the opt-in); got {:?}",
                p.profile_id,
                dag.vm,
                p.mount_policy.device_binds
            );
            assert!(
                p.namespaces.pid,
                "video {} (vm {}) must use a private PID namespace; got {:?}",
                p.profile_id, dag.vm, p.namespaces
            );
            assert_eq!(
                p.umask,
                Some(7),
                "video {} (vm {}) must set umask = 0o007 so CH connects through the inherited \
                 default ACL; got {:?}",
                p.profile_id,
                dag.vm,
                p.umask
            );
            assert_eq!(
                p.cgroup_placement.subtree,
                format!("nixling.slice/{}/video", dag.vm),
                "video {} (vm {}) cgroup subtree drift",
                p.profile_id,
                dag.vm
            );
            assert!(
                !gpu_uids.contains(&p.uid),
                "video {} (vm {}) must run as a DEDICATED principal distinct from gpu \
                 (AGENTS: nixling-<vm>-video, NOT gpu); video uid {} collides with a gpu uid \
                 in {:?}",
                p.profile_id,
                dag.vm,
                p.uid,
                gpu_uids
            );
        }
    }
    assert!(
        video_seen > 0,
        "feature-rich fixture has no video sidecar node (corp-full enables graphics/video — \
         regression)"
    );
}

/// video Layer-1 V1/V4(source) gating + namespace + principal: the
/// `minijail-profiles.nix` video block MUST gate the NVIDIA device nodes
/// (`/dev/nvidiactl`, `/dev/nvidia0`, `/dev/nvidia-uvm`) behind
/// `lib.optionals (vm.graphics.videoNvidiaDecode or false) [...]`, use the
/// private-PID namespace expression `defaultNamespaces // { pid = true; }`,
/// and bind the role to the dedicated `nixling-${name}-video` principal. The
/// bash gate scoped these with `grep -A 45 'profileIdFor name "video"'`; this
/// port extracts the same window (header + 45 lines) so the earlier gpu block
/// (which also lists `/dev/nvidiactl`) cannot satisfy the check.
#[test]
fn video_source_nvidia_gating_namespace_principal() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "missing {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);
    let window = lines_after(&src, r#"profileIdFor name "video""#, 45)
        .expect("could not locate video profile block in minijail-profiles.nix");

    assert!(
        window.contains("videoNvidiaDecode"),
        "video profile must gate NVIDIA nodes behind videoNvidiaDecode in {MINIJAIL_PROFILES_NIX}"
    );
    for node in ["/dev/nvidiactl", "/dev/nvidia0", "/dev/nvidia-uvm"] {
        assert!(
            window.contains(node),
            "video profile videoNvidiaDecode opt-in must list {node} in {MINIJAIL_PROFILES_NIX}"
        );
    }
    assert!(
        window.contains("namespaces = defaultNamespaces // { pid = true; }"),
        "video profile must use a private PID namespace (defaultNamespaces // {{ pid = true; }}) \
         in {MINIJAIL_PROFILES_NIX}"
    );
    assert!(
        window.contains(r#"principal = "nixling-${name}-video""#),
        "video profile must bind the dedicated nixling-${{name}}-video principal (NOT gpu) in \
         {MINIJAIL_PROFILES_NIX}"
    );
}

/// video Layer-1 V6: `host-activation.nix` MUST limit the video runtime dir
/// ACL to the cloud-hypervisor + video UIDs and EXCLUDE video from the host
/// session-socket ACLs (gpu/audio session sockets). Ports the bash gate's ten
/// `grep -q` substrings verbatim: the per-role uid collections
/// (`video_media_uids=` over `cloud-hypervisor-runner`/`video`,
/// `gpu_session_uids=` over `gpu`/`gpu-render-node`, `audio_session_uids=`
/// over `audio`), the default-ACL removal `setfacl -d -x "u:$uid"
/// /run/nixling-video`, the `u:$uid:---` deny entries, and the
/// `stale_video_uid=` / `u:$stale_video_uid:---` stale-video reaping. These
/// are literal `grep -q` substrings (single-quoted in bash, so `$uid` is
/// literal), ported as exact `contains` checks.
#[test]
fn video_host_activation_runtime_and_session_acls() {
    assert!(
        repo_path_exists(HOST_ACTIVATION_NIX),
        "missing {HOST_ACTIVATION_NIX}"
    );
    let src = read_repo_file(HOST_ACTIVATION_NIX);
    let needles = [
        "video_media_uids=",
        "gpu_session_uids=",
        "audio_session_uids=",
        r#"select(.role == "gpu" or .role == "gpu-render-node")"#,
        r#"select(.role == "audio")"#,
        r#"select(.role == "cloud-hypervisor-runner" or .role == "video")"#,
        r#"setfacl -d -x "u:$uid" /run/nixling-video"#,
        "u:$uid:---",
        "stale_video_uid=",
        "u:$stale_video_uid:---",
    ];
    for needle in needles {
        assert!(
            src.contains(needle),
            "video runtime/session ACL plumbing missing `{needle}` in {HOST_ACTIVATION_NIX}: the \
             video runtime dir ACL must grant only cloud-hypervisor/video runtime access and \
             video must be excluded from host session sockets"
        );
    }
}
