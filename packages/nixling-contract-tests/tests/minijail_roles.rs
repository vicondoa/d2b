//! Per-role minijail validators, ported from the bash gates:
//!   * `tests/minijail-validator-cloud-hypervisor.sh`
//!   * `tests/minijail-validator-virtiofsd.sh`
//!
//! These gates validate the RENDERED minijail role profiles, so they belong in
//! the fixture-contract layer (this crate, gated by the NL_FIXTURES step in
//! `tests/tools/rust-workspace-checks.sh`), not the doc/source-grep policy layer.
//!
//! Each VM emits a `cloud-hypervisor` runner and a set of `virtiofsd` shares,
//! so both roles' profiles are present in the fixture-smoke bundle (corp-vm +
//! sys-work-net). The other minijail-validators (gpu/swtpm/audio/video/usbip)
//! need feature-enabled VMs that the smoke fixture does not contain and are out
//! of scope here.
//!
//! Layer split (faithful to the bash gates):
//!   * The bash gates' Layer-1 / Phase-1 (eval-only) profile-shape assertions
//!     port here, either as RENDERED checks over the real fixture RoleProfiles
//!     (a strictly stronger guarantee than the bash `jq`/grep over a synthetic
//!     re-eval) or, where the assertion needs a config the single-rendering
//!     fixture cannot express, as a SOURCE-grep over the in-tree Nix module.
//!   * The bash gates' opt-in live phases (`NL_LIVE=1`: invoking `minijail0`,
//!     `cloud-hypervisor --version`, a ptrace SIGSYS probe, and writing
//!     `/var/lib/nixling/validated/p1-*.json` evidence) are runtime execution
//!     tests that require root, a live host, and the role binaries. They are
//!     NOT contract-test material and intentionally do not port. (The virtiofsd
//!     Layer-2 negative path is, in fact, skipped even by the bash gate itself
//!     — it returns early with "seccomp blob not materialized".)
//!
//! Spec corrections / smoke-fixture gaps:
//!   * cloud-hypervisor Phase-1b (persistent-tap mode -> `["CAP_NET_ADMIN"]`)
//!     re-evaluates the flake with `site.ch.netHandoffMode = "persistent-tap"`.
//!     The fixture-smoke bundle is a SINGLE rendering in the default `tap-fd`
//!     mode, so the persistent-tap branch cannot be reached from the rendered
//!     artifacts. Its coverage is preserved as a SOURCE-grep over
//!     `nixos-modules/minijail-profiles.nix` asserting the exact
//!     `lib.optionals (… == "persistent-tap") [ "CAP_NET_ADMIN" ]` conditional,
//!     which grounds BOTH the tap-fd (empty) and persistent-tap branches.
//!   * The virtiofsd `requires_start_root == false` and `exceptionRef =
//!     virtiofsdRootException` fields live on the `MinijailProfile` DTO; the
//!     per-VM bundle ships RoleProfiles (in processes.json) and references the
//!     standalone profile JSONs by path WITHOUT bundling them, so those two
//!     fields are not present in the rendered fixture. The bash gate also reads
//!     them from source (`assert_profile_source` greps the `.nix`), so they
//!     port faithfully as SOURCE-grep assertions.

use nixling_contract_tests::{load_bundle_resolver_from_env, read_repo_file, repo_path_exists};
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

/// Whether `pattern` matches anywhere in `content`, allowing matches to span
/// newlines (the pattern is responsible for using `\s*`/`\s+` between tokens).
/// Used to assert a multi-line Nix expression exists verbatim.
fn whole_text_matches(content: &str, pattern: &str) -> bool {
    let re = Regex::new(pattern).expect("valid regex");
    re.is_match(content)
}

/// Extract the inclusive line range from the first line matching `start_pat`
/// through the first subsequent line matching `end_pat`. Mirrors the bash
/// gate's `awk '/start/{active=1} active{print} active&&/end/{exit}'` block
/// extraction.
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
// tests/minijail-validator-cloud-hypervisor.sh
// ===========================================================================

/// Phase-1a (default `tap-fd` mode): the rendered cloud-hypervisor runner
/// profile MUST declare EMPTY host capabilities. In tap-fd mode the broker's
/// `CreateTapFd` op opens `/dev/net/tun` + calls TUNSETIFF pre-spawn and passes
/// the TAP fd to CH via SCM_RIGHTS, so CH needs no `CAP_NET_ADMIN`; the
/// bounding-set drop is enforced by the profile granting zero capabilities.
#[test]
fn cloud_hypervisor_tap_fd_profile_declares_empty_capabilities() {
    let resolver = load_bundle_resolver_from_env();
    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::CloudHypervisorRunner {
                continue;
            }
            seen += 1;
            assert!(
                node.profile.caps.is_empty(),
                "cloud-hypervisor runner {} (vm {}) must declare EMPTY capabilities in the \
                 default tap-fd handoff mode (D4a bounding-set drop); got {:?}",
                node.profile.profile_id,
                dag.vm,
                node.profile.caps,
            );
        }
    }
    assert!(
        seen > 0,
        "fixture has no cloud-hypervisor-runner node — every VM emits one (regression)"
    );
}

/// Phase-1b (persistent-tap fallback): the cloud-hypervisor runner profile must
/// retain EXACTLY `["CAP_NET_ADMIN"]`. The fixture-smoke bundle renders only the
/// default tap-fd mode, so the persistent-tap branch is unreachable from the
/// artifacts; its coverage is preserved by asserting the exact conditional in
/// `nixos-modules/minijail-profiles.nix` that grounds both branches:
///
/// ```nix
/// capabilities = lib.optionals
///   (cfg.site.ch.netHandoffMode == "persistent-tap")
///   [ "CAP_NET_ADMIN" ];
/// ```
#[test]
fn cloud_hypervisor_persistent_tap_cap_net_admin_source_logic() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "missing {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);
    let pattern = r#"capabilities\s*=\s*lib\.optionals\s*\(\s*cfg\.site\.ch\.netHandoffMode\s*==\s*"persistent-tap"\s*\)\s*\[\s*"CAP_NET_ADMIN"\s*\]\s*;"#;
    assert!(
        whole_text_matches(&src, pattern),
        "cloud-hypervisor profile must grant CAP_NET_ADMIN iff netHandoffMode == \
         \"persistent-tap\" (empty otherwise); the canonical \
         `lib.optionals (… == \"persistent-tap\") [ \"CAP_NET_ADMIN\" ]` conditional was not \
         found in {MINIJAIL_PROFILES_NIX}"
    );
}

/// The rendered cloud-hypervisor jail shape (the broader role invariants the
/// gate documents: device binds, cgroup placement, namespace isolation,
/// seccomp reference). In the default tap-fd mode CH never touches
/// `/dev/net/tun` (the broker pre-opens it), so the device-bind set must NOT
/// expose the tun node — the security-relevant half of the D4a cap-drop.
#[test]
fn cloud_hypervisor_rendered_jail_shape() {
    let resolver = load_bundle_resolver_from_env();
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::CloudHypervisorRunner {
                continue;
            }
            let p = &node.profile;
            let dev = &p.mount_policy.device_binds;
            assert!(
                dev.iter().any(|d| d == "/dev/kvm"),
                "cloud-hypervisor {} (vm {}) must bind /dev/kvm; got {dev:?}",
                p.profile_id,
                dag.vm
            );
            assert!(
                dev.iter().any(|d| d == "/dev/vhost-net"),
                "cloud-hypervisor {} (vm {}) must bind /dev/vhost-net; got {dev:?}",
                p.profile_id,
                dag.vm
            );
            assert!(
                !dev.iter().any(|d| d == "/dev/net/tun"),
                "cloud-hypervisor {} (vm {}) must NOT bind /dev/net/tun in tap-fd mode \
                 (broker pre-opens it); got {dev:?}",
                p.profile_id,
                dag.vm
            );
            assert_eq!(
                p.cgroup_placement.subtree,
                format!("nixling.slice/{}/cloud-hypervisor", dag.vm),
                "cloud-hypervisor {} (vm {}) cgroup subtree drift",
                p.profile_id,
                dag.vm
            );
            assert!(
                p.namespaces.mount && p.namespaces.ipc,
                "cloud-hypervisor {} (vm {}) must isolate the mount + ipc namespaces; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
            assert!(
                !p.namespaces.net && !p.namespaces.user,
                "cloud-hypervisor {} (vm {}) must not request a net or user namespace; got {:?}",
                p.profile_id,
                dag.vm,
                p.namespaces
            );
            assert_eq!(
                p.seccomp_policy_ref.as_deref(),
                Some("w1-cloud-hypervisor-runner"),
                "cloud-hypervisor {} (vm {}) seccompPolicyRef drift",
                p.profile_id,
                dag.vm
            );
        }
    }
}

// ===========================================================================
// tests/minijail-validator-virtiofsd.sh
// ===========================================================================

/// Layer-1 `assert_profile_source`: the virtiofsd profile in
/// `nixos-modules/minijail-profiles.nix` MUST match the ADR-0021 broker-pre-NS
/// shape exactly — the carve-out marker string + `exceptionRef`, zero host
/// `CAP_*` tokens inside the profile block, `requiresStartRoot = false`, a
/// `userNamespace = { hostUidForZero, hostGidForZero }` mapping, and the closed
/// `seccompPolicyRef = "w1-virtiofsd"` allowlist.
#[test]
fn virtiofsd_profile_source_shape_matches_adr_0021() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "missing {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);

    // The carve-out marker string + exceptionRef anchor (anywhere in file).
    let carve_out = "ADR 0021 v1.1.1fu14 virtiofsd fake-root via broker pre-established user NS";
    assert!(
        src.contains(carve_out),
        "virtiofsdRootException string '{carve_out}' not found in {MINIJAIL_PROFILES_NIX}"
    );
    assert!(
        any_line_matches(&src, r"exceptionRef\s*=\s*virtiofsdRootException"),
        "virtiofsd profile is missing exceptionRef = virtiofsdRootException in {MINIJAIL_PROFILES_NIX}"
    );

    // Extract the virtiofsd profile block: from `role = "virtiofsd";` through
    // the closing `exceptionRef = virtiofsdRootException;` line (the bash awk
    // block extraction).
    let block = extract_block(
        &src,
        r#"role\s*=\s*"virtiofsd";"#,
        r"exceptionRef\s*=\s*virtiofsdRootException;",
    )
    .expect("could not locate virtiofsd profile block in minijail-profiles.nix");

    // Zero host CAP_* tokens inside the block (ADR 0021: broker-pre-NS gives
    // full caps inside the user NS; the host needs none).
    let cap_token = Regex::new(r#""CAP_[A-Z_]+""#).expect("valid regex");
    let cap_hits: Vec<&str> = block.lines().filter(|l| cap_token.is_match(l)).collect();
    assert!(
        cap_hits.is_empty(),
        "virtiofsd profile must declare ZERO host caps (ADR 0021); found CAP_* tokens: {cap_hits:?}"
    );

    // requiresStartRoot MUST be false (never `= true`).
    assert!(
        !any_line_matches(&block, r"requiresStartRoot\s*=\s*true"),
        "virtiofsd profile must declare requiresStartRoot = false (ADR 0021 retires the root carve-out)"
    );

    // userNamespace single-entry mapping must be declared.
    assert!(
        any_line_matches(&block, r"userNamespace\s*="),
        "virtiofsd profile must declare userNamespace = {{ ... }} (ADR 0021)"
    );
    assert!(
        any_line_matches(&block, r"hostUidForZero\s*="),
        "virtiofsd profile userNamespace must include hostUidForZero (ADR 0021)"
    );
    assert!(
        any_line_matches(&block, r"hostGidForZero\s*="),
        "virtiofsd profile userNamespace must include hostGidForZero (ADR 0021)"
    );

    // Steady-state seccomp policy reference must be the closed w1-virtiofsd allowlist.
    assert!(
        any_line_matches(&block, r#"seccompPolicyRef\s*=\s*"w1-virtiofsd""#),
        "virtiofsd profile missing seccompPolicyRef = \"w1-virtiofsd\""
    );
}

/// Layer-1 `assert_installed_profiles`, applied to the REAL rendered fixture
/// RoleProfiles (stronger than the bash gate, which skipped when no host
/// profiles were installed under `/etc/nixling/minijail-profiles/`). Every
/// rendered virtiofsd role profile MUST carry the broker-pre-NS shape: empty
/// host caps, a numeric single-entry `userNamespace`, and the `w1-virtiofsd`
/// seccomp reference.
#[test]
fn virtiofsd_rendered_profiles_match_broker_pre_ns_shape() {
    let resolver = load_bundle_resolver_from_env();
    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::Virtiofsd {
                continue;
            }
            seen += 1;
            let p = &node.profile;
            assert!(
                p.caps.is_empty(),
                "virtiofsd {} (vm {}) caps drift; expected [] (ADR 0021 broker-pre-NS), got {:?}",
                p.profile_id,
                dag.vm,
                p.caps
            );
            let user_ns = p.user_namespace.as_ref().unwrap_or_else(|| {
                panic!(
                    "virtiofsd {} (vm {}) missing userNamespace (ADR 0021 requires a single-entry uid_map)",
                    p.profile_id, dag.vm
                )
            });
            // host_uid_for_zero / host_gid_for_zero are typed u32 in the DTO, so
            // their mere presence (deserialization above) satisfies the bash
            // `type == "number"` check; assert they are real principal ids, not
            // the unmapped 0 that would defeat the fake-root mapping.
            assert_ne!(
                user_ns.host_uid_for_zero, 0,
                "virtiofsd {} (vm {}) userNamespace.hostUidForZero must map to a real principal uid, not 0",
                p.profile_id, dag.vm
            );
            assert_ne!(
                user_ns.host_gid_for_zero, 0,
                "virtiofsd {} (vm {}) userNamespace.hostGidForZero must map to a real principal gid, not 0",
                p.profile_id, dag.vm
            );
            assert_eq!(
                p.seccomp_policy_ref.as_deref(),
                Some("w1-virtiofsd"),
                "virtiofsd {} (vm {}) seccompPolicyRef drift",
                p.profile_id,
                dag.vm
            );
            assert!(
                p.namespaces.user,
                "virtiofsd {} (vm {}) must request a user namespace (ADR 0021 broker-pre-NS); got {:?}",
                p.profile_id, dag.vm, p.namespaces
            );
        }
    }
    assert!(
        seen > 0,
        "fixture has no virtiofsd node — every VM emits virtiofs shares (regression)"
    );
}

/// The rendered read-only-store virtiofsd shares carry the ADR-0021 read-only
/// serving invariants: `/nix/store` mounted read-only and the argv shape
/// `--sandbox=chroot --inode-file-handles=never` with `--readonly` on the
/// read-only shares. (Argv-shape coverage is owned canonically by the
/// `virtiofsd_argv` unit test; this rendered-fixture check is the per-role
/// minijail-validator's ADR-0021 ro-store invariant, kept here so retiring the
/// bash gate does not lose the rendered-layer guarantee.)
#[test]
fn virtiofsd_ro_store_rendered_adr_0021_invariants() {
    let resolver = load_bundle_resolver_from_env();
    let mut ro_store_seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::Virtiofsd {
                continue;
            }
            let p = &node.profile;
            // Common ADR-0021 chroot-sandbox argv on every virtiofsd share.
            assert!(
                node.argv.iter().any(|a| a == "--sandbox=chroot"),
                "virtiofsd {} (vm {}) argv must include --sandbox=chroot (ADR 0021); got {:?}",
                p.profile_id,
                dag.vm,
                node.argv
            );
            assert!(
                node.argv.iter().any(|a| a == "--inode-file-handles=never"),
                "virtiofsd {} (vm {}) argv must include --inode-file-handles=never (ADR 0021); got {:?}",
                p.profile_id,
                dag.vm,
                node.argv
            );

            // The read-only store share additionally serves /nix/store read-only
            // and passes --readonly.
            if p.profile_id.ends_with("-virtiofsd-ro-store") {
                ro_store_seen += 1;
                assert!(
                    p.mount_policy.nix_store_read_only,
                    "virtiofsd ro-store {} (vm {}) must set nixStoreReadOnly = true",
                    p.profile_id, dag.vm
                );
                assert!(
                    p.mount_policy
                        .read_only_paths
                        .iter()
                        .any(|rp| rp == "/nix/store"),
                    "virtiofsd ro-store {} (vm {}) must mount /nix/store read-only; got {:?}",
                    p.profile_id,
                    dag.vm,
                    p.mount_policy.read_only_paths
                );
                assert!(
                    node.argv.iter().any(|a| a == "--readonly"),
                    "virtiofsd ro-store {} (vm {}) argv must include --readonly; got {:?}",
                    p.profile_id,
                    dag.vm,
                    node.argv
                );
            }
        }
    }
    assert!(
        ro_store_seen > 0,
        "fixture has no virtiofsd ro-store share — every VM serves a read-only store (regression)"
    );
}

// ===========================================================================
// qemu-media fd-backed runner profile
// ===========================================================================

#[test]
fn qemu_media_profile_source_is_fd_backed_and_device_closed() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "missing {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);
    let block = extract_block(
        &src,
        r#"role\s*=\s*"qemu-media-runner";"#,
        r"controllers\s*=\s*serviceControllers;",
    )
    .expect("could not locate qemu-media profile block in minijail-profiles.nix");

    assert!(
        any_line_matches(&block, r"capabilities\s*=\s*\[\s*\]"),
        "qemu-media profile must declare an empty host capability set"
    );
    for cap in [
        "CAP_SYS_ADMIN",
        "CAP_SYS_RAWIO",
        "CAP_DAC_OVERRIDE",
        "CAP_NET_ADMIN",
    ] {
        assert!(
            !block.contains(cap),
            "qemu-media profile must not mention forbidden capability {cap}"
        );
    }
    assert!(
        any_line_matches(&block, r#"seccompPolicyRef\s*=\s*"w1-qemu-media";"#),
        "qemu-media profile must declare the mandatory w1-qemu-media seccomp policy"
    );
    assert!(
        any_line_matches(
            &block,
            r"namespaces\s*=\s*defaultNamespaces\s*//\s*\{\s*pid\s*=\s*true;\s*\};"
        ),
        "qemu-media profile must request a private pid namespace so /dev masking installs a private procfs"
    );
    assert!(
        any_line_matches(&block, r#"readOnlyPaths\s*=\s*\[\s*"/"\s*\];"#),
        "qemu-media profile must make the root mount read-only"
    );
    assert!(
        any_line_matches(&block, r#"deviceBinds\s*=\s*\[\s*"/dev/kvm"\s*\]"#),
        "qemu-media fd-backed mode must expose only /dev/kvm by path"
    );
    assert!(
        any_line_matches(&block, r"bindMounts\s*=\s*\[\s*\]"),
        "qemu-media fd-backed mode must not bind broad media/display paths"
    );
    for forbidden in ["/dev/bus/usb", "/dev/net/tun", "/dev/vhost-net"] {
        assert!(
            !block.contains(forbidden),
            "qemu-media profile must not bind forbidden device path {forbidden}"
        );
    }
    assert!(
        block.contains(r#""/run/nixling/vms/${name}""#) && block.contains("(stateDirOf name)"),
        "qemu-media writable paths must stay scoped to per-VM runtime/state directories"
    );
}

#[test]
fn qemu_media_activation_acl_source_splits_kvm_from_vhost_net() {
    let src = read_repo_file("nixos-modules/host-activation.nix");
    assert!(
        src.contains(
            r#"select(.role == "cloud-hypervisor-runner" or .role == "gpu" or .role == "qemu-media-runner")"#
        ),
        "qemu-media runner UID must be classified as KVM-consuming for focused /dev/kvm ACLs"
    );
    assert!(
        src.contains(r#"select(.role == "cloud-hypervisor-runner" or .role == "gpu")"#),
        "vhost-net ACL classifier must remain separate from KVM consumers"
    );
    assert!(
        src.contains("stale_qemu_media_uid"),
        "activation must revoke stale qemu-media display/KVM ACLs"
    );
    assert!(
        src.contains("setfacl -x \"u:$stale_qemu_media_uid\" /dev/vhost-net"),
        "qemu-media fd-backed mode must actively revoke stale /dev/vhost-net ACLs"
    );
}

#[test]
fn broker_spawn_sets_no_new_privs_before_seccomp() {
    let src = read_repo_file("packages/nixling-priv-broker/src/sys.rs");
    let no_new_privs = src
        .find("libc::PR_SET_NO_NEW_PRIVS")
        .expect("broker spawn child must call PR_SET_NO_NEW_PRIVS");
    let seccomp = src
        .find("apply_seccomp(program)")
        .expect("broker spawn child must install seccomp");

    assert!(
        no_new_privs < seccomp,
        "broker spawn must set no-new-privileges before installing seccomp"
    );
    assert!(
        src.contains("libc::_exit(CHILD_EXIT_PRCTL_NO_NEW_PRIVS)")
            && src.contains("libc::_exit(CHILD_EXIT_SECCOMP)"),
        "broker spawn must fail closed when no-new-privs or seccomp installation fails"
    );
}
