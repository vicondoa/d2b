use d2b_contract_tests::{load_full_bundle_resolver_from_env, read_repo_file};
use d2b_core::processes::{ProcessNode, ProcessRole, VmProcessDag};

const MINIJAIL_PROFILES: &str = "nixos-modules/minijail-profiles.nix";
const ROLE_ROWS: &str = "nixos-modules/role-process-rows.nix";
const DEVICE_ROWS: &str = "nixos-modules/realm-device-rows.nix";
const DEVICE_PROVIDER: &str = "nixos-modules/provider-registry-v2-extensions/device.nix";
const AUDIO_ROWS: &str = "nixos-modules/realm-audio-rows.nix";
const AUDIO_PROVIDER: &str = "nixos-modules/provider-registry-v2-extensions/audio.nix";

fn assert_role_profile(dag: &VmProcessDag, node: &ProcessNode, seccomp: &str) {
    let identity = dag
        .workload_identity
        .as_ref()
        .expect("realm role must carry workload identity");
    assert_eq!(node.profile.profile_id, format!("role-{}", node.id.0));
    assert_eq!(
        node.profile.cgroup_placement.subtree,
        format!(
            "d2b.slice/r-{}/workloads/w-{}/{}",
            identity.realm_id.as_str(),
            identity.workload_id.as_str(),
            node.id.0
        )
    );
    assert_eq!(node.profile.seccomp_policy_ref.as_deref(), Some(seccomp));
    assert!(node.profile.caps.is_empty());
    assert!(node.profile.mount_policy.hide_device_nodes_by_default);
    assert!(node.profile.mount_policy.nix_store_read_only);
}

#[test]
fn audio_source_uses_canonical_role_resource_provider_rows() {
    let audio = read_repo_file(AUDIO_ROWS);
    let roles = read_repo_file(ROLE_ROWS);
    let profiles = read_repo_file(MINIJAIL_PROFILES);
    let provider = read_repo_file(AUDIO_PROVIDER);

    for required in [
        r#"roleKind == "audio""#,
        r#"kind = "vhost-user-sound";"#,
        r#"kind = "pipewire-session-endpoint";"#,
        r#"share = "shared-partition";"#,
        r#"supervision = "realm-controller-pidfd";"#,
        r#"seccompPolicyRef = "w1-audio";"#,
        r#"parentRuntimeVisible = false;"#,
    ] {
        assert!(
            audio.contains(required),
            "canonical audio policy missing {required:?} from {AUDIO_ROWS}"
        );
    }
    assert!(
        roles.contains(r#"audio = "audio";"#)
            && profiles.contains(r#"audio = "w1-audio";"#)
            && profiles.contains(r#"else if role.processRole == "audio""#)
            && profiles
                .contains(r#"[ "swtpm" "gpu" "gpu-render-node" "video" "audio" "wayland-proxy" ]"#)
    );
    assert!(
        provider.contains(r#"axis = "local-audio";"#)
            && provider.contains(r#"implementationId = "pipewire-vhost-user";"#)
            && provider.contains("leaseId")
    );
}

#[test]
fn audio_rendered_profile_is_realm_role_scoped() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset; audio role fixture unavailable");
        return;
    };
    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::Audio {
                continue;
            }
            seen += 1;
            assert_role_profile(dag, node, "w1-audio");
            assert!(node.profile.mount_policy.device_binds.is_empty());
            assert!(node.profile.mount_policy.bind_mounts.is_empty());
            assert!(node.profile.user_namespace.is_none());
            assert!(node.profile.namespaces.ipc && node.profile.namespaces.mount);
            assert!(!node.profile.namespaces.net);
            assert!(!node.profile.namespaces.pid);
            assert!(!node.profile.namespaces.user);
            assert!(!node.profile.namespaces.uts);
            assert_eq!(node.profile.umask, Some(7));

            let identity = dag.workload_identity.as_ref().unwrap();
            let run_root = format!(
                "/run/d2b/r/{}/w/{}",
                identity.realm_id.as_str(),
                identity.workload_id.as_str()
            );
            let role_root = format!("{run_root}/roles/{}", node.id.0);
            let writable: Vec<String> = node
                .profile
                .mount_policy
                .writable_paths
                .iter()
                .map(|path| path.path.clone())
                .collect();
            assert_eq!(
                writable,
                vec![
                    format!("{run_root}/sockets"),
                    format!("{role_root}/pipewire")
                ]
            );
            assert_eq!(
                node.argv,
                vec![
                    format!("{role_root}/d2b-audio-{}", identity.workload_id.as_str()),
                    "--socket".to_owned(),
                    format!("{run_root}/sockets/audio.sock"),
                    "--backend".to_owned(),
                    "pipewire".to_owned(),
                ]
            );
            assert!(
                node.env
                    .iter()
                    .any(|entry| entry == &format!("PIPEWIRE_RUNTIME_DIR={role_root}/pipewire"))
            );
            assert!(!node.env.iter().any(|entry| entry.contains("/run/user/")));
        }
    }
    assert_eq!(seen, 1, "fixture must render one mediated audio role");
}

#[test]
fn usbip_source_uses_canonical_role_resource_provider_rows() {
    let devices = read_repo_file(DEVICE_ROWS);
    let roles = read_repo_file(ROLE_ROWS);
    let profiles = read_repo_file(MINIJAIL_PROFILES);
    let provider = read_repo_file(DEVICE_PROVIDER);

    for required in [
        r#"usbip = "usbip";"#,
        r#"usbip = "usbip-exclusive";"#,
        r#"[ "usbip" "fido" ]"#,
        r#"leaseId = "lease-device-security-key-global";"#,
        r#"share = "exclusive";"#,
        r#"attachment = "fd-only";"#,
        r#"broker = "realm-local";"#,
    ] {
        assert!(
            devices.contains(required),
            "canonical USBIP policy missing {required:?} from {DEVICE_ROWS}"
        );
    }
    assert!(roles.contains("cfg._index.devices.byRoleId"));
    assert!(
        profiles.contains(r#"usbip = "w1-usbip";"#)
            && profiles.contains("deviceBindsFor role.processRole")
            && profiles.contains(
                r#""d2b.slice/r-${role.realmId}/workloads/w-${role.workloadId}/${role.roleId}""#
            )
    );
    assert!(
        provider.contains("deviceResourceIds")
            && provider.contains("row.providerId == provider.providerId")
    );
    for forbidden in ["busid", "busId", "/dev/bus/usb", "hidraw"] {
        assert!(
            !devices.contains(forbidden),
            "realm USBIP rows must not expose {forbidden:?}"
        );
    }
}

#[test]
fn usbip_rendered_profile_is_allocator_tracker_only() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset; USBIP role fixture unavailable");
        return;
    };
    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::Usbip {
                continue;
            }
            seen += 1;
            assert_role_profile(dag, node, "w1-usbip");
            assert!(node.profile.mount_policy.device_binds.is_empty());
            assert!(node.profile.user_namespace.is_none());
            assert!(node.binary_path.is_none() && node.argv.is_empty());
        }
    }
    assert_eq!(
        seen, 1,
        "fixture must render one allocator-mediated USBIP tracker"
    );
}
