//! Per-role minijail validators, ported from the bash gates:
//!   * `tests/minijail-validator-cloud-hypervisor.sh`
//!   * `tests/minijail-validator-virtiofsd.sh`
//!
//! These gates validate the RENDERED minijail role profiles, so they belong in
//! the fixture-contract layer (this crate, gated by the D2B_FIXTURES step in
//! `tests/tools/rust-workspace-checks.sh`), not the doc/source-grep policy layer.
//!
//! Each local-VM workload emits a `cloud-hypervisor` role and a set of
//! `virtiofsd` shares, so both role-profile families are present in the smoke
//! fixture. Other feature-role validators are out of scope here.
//!
//! Layer split (faithful to the bash gates):
//!   * The bash gates' Layer-1 / Phase-1 (eval-only) profile-shape assertions
//!     port here, either as RENDERED checks over the real fixture RoleProfiles
//!     (a strictly stronger guarantee than the bash `jq`/grep over a synthetic
//!     re-eval) or, where the assertion needs a config the single-rendering
//!     fixture cannot express, as a SOURCE-grep over the in-tree Nix module.
//!   * The bash gates' opt-in live phases (`D2B_LIVE=1`: invoking `minijail0`,
//!     `cloud-hypervisor --version`, a ptrace SIGSYS probe, and writing
//!     `/var/lib/d2b/validated/p1-*.json` evidence) are runtime execution
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

use d2b_contract_tests::{load_bundle_resolver_from_env, read_repo_file, repo_path_exists};
use d2b_core::processes::ProcessRole;

const MINIJAIL_PROFILES_NIX: &str = "nixos-modules/minijail-profiles.nix";

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

/// The canonical role profile consumes an allocator-supplied network
/// interface and never grants CAP_NET_ADMIN to the workload runner.
#[test]
fn cloud_hypervisor_role_profile_has_no_network_capability_escape_hatch() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "missing {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);
    assert!(
        src.contains("capabilities ? [ ],")
            && !src.contains("CAP_NET_ADMIN")
            && !src.contains("netHandoffMode"),
        "canonical workload role profiles must default to no capabilities and must not \
         restore a VM-mode CAP_NET_ADMIN escape hatch"
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
                format!(
                    "d2b.slice/r-{}/workloads/w-{}/{}",
                    dag.workload_identity
                        .as_ref()
                        .expect("canonical workload DAG identity")
                        .realm_id,
                    dag.vm,
                    node.id.0
                ),
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
                Some("w1-cloud-hypervisor"),
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

/// The normalized share-profile composer must preserve the ADR 0021
/// broker-pre-established user namespace contract.
#[test]
fn virtiofsd_profile_source_shape_matches_adr_0021() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "missing {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);

    assert!(
        src.contains(r#"role = roleRowFor workload.workloadId "virtiofsd";"#)
            && src.contains(r#"processRole = "virtiofsd";"#),
        "virtiofsd profiles must be derived from the workload's canonical role row"
    );
    assert!(
        src.contains("requiresStartRoot = false;")
            && src.contains("capabilities = [ ];")
            && !src.contains("requiresStartRoot = true;"),
        "virtiofsd profiles must start without root and grant zero host capabilities"
    );
    assert!(
        src.contains("hostUidForZero = uid;")
            && src.contains("hostGidForZero = gid;")
            && src.contains(r#"then "d2b-gctlfs-${workload.workloadId}""#),
        "virtiofsd profiles must map fake root to the role principal, with a narrower \
         guest-control share principal"
    );
    assert!(
        src.contains(r#"seccompPolicyRef = "w1-virtiofsd";"#),
        "virtiofsd profile missing seccompPolicyRef = \"w1-virtiofsd\""
    );
}

/// Layer-1 `assert_installed_profiles`, applied to the REAL rendered fixture
/// RoleProfiles (stronger than the bash gate, which skipped when no host
/// profiles were installed under `/etc/d2b/minijail-profiles/`). Every
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
            if p.profile_id.ends_with("-ro-store") {
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
    assert!(
        src.contains(r#""qemu-media-runner" = "w1-qemu-media";"#)
            && src.contains("capabilities ? [ ],"),
        "qemu-media canonical role profile must use the closed seccomp policy and \
         empty default capability set"
    );
    for cap in [
        "CAP_SYS_ADMIN",
        "CAP_SYS_RAWIO",
        "CAP_DAC_OVERRIDE",
        "CAP_NET_ADMIN",
    ] {
        assert!(
            !src.contains(cap),
            "qemu-media profile must not mention forbidden capability {cap}"
        );
    }
    assert!(
        src.contains(
            r#""d2b.slice/r-${role.realmId}/workloads/w-${role.workloadId}/${role.roleId}""#
        ),
        "qemu-media profile must use canonical realm/workload/role cgroup placement"
    );
}

/// A generic-profile regression briefly let qemu-media-runner inherit
/// cloud-hypervisor's full device-bind list (including
/// /dev/vhost-net, which qemu-media must never expose as a path — vhost-net
/// stays inherited-fd only per docs/reference/privileges.md) and dropped the
/// private pid namespace both qemu-media-runner and video need to contain
/// their forked/reaped child processes. Pin the source shape so both stay
/// role-scoped rather than defaulting to a single shared list/flag.
#[test]
fn qemu_media_device_binds_and_pid_namespace_are_role_scoped() {
    assert!(
        repo_path_exists(MINIJAIL_PROFILES_NIX),
        "missing {MINIJAIL_PROFILES_NIX}"
    );
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);
    assert!(
        src.contains(r#"then [ "/dev/kvm" "/dev/vhost-net" ]"#)
            && src.contains(r#"else if processRole == "qemu-media-runner""#)
            && src.contains(r#"then [ "/dev/kvm" ]"#),
        "deviceBindsFor must give qemu-media-runner ONLY /dev/kvm; \
         /dev/vhost-net must remain cloud-hypervisor-runner-exclusive"
    );
    assert!(
        src.contains("pidNamespace = builtins.elem role.processRole")
            && src.contains(r#"[ "video" "qemu-media-runner" ]"#),
        "video and qemu-media-runner must each get a private pid namespace to \
         contain/reap forked child processes"
    );
}

#[test]
fn qemu_media_profile_is_selected_by_canonical_role_kind() {
    let src = read_repo_file("nixos-modules/role-process-rows.nix");
    assert!(
        src.contains(r#""qemu-media" = "qemu-media-runner";"#)
            && src.contains("else roleName role.roleKind;")
            && src.contains("nodeId = roleId;"),
        "qemu-media confinement must be selected from a normalized canonical role row"
    );
}

#[test]
fn broker_spawn_sets_no_new_privs_before_seccomp() {
    let src = read_repo_file("packages/d2b-priv-broker/src/sys.rs");
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
