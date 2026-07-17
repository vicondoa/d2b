use d2b_contract_tests::{load_full_bundle_resolver_from_env, read_repo_file};
use d2b_core::processes::{ProcessNode, ProcessRole, VmProcessDag};

const DEVICE_ROWS: &str = "nixos-modules/realm-device-rows.nix";
const ROLE_ROWS: &str = "nixos-modules/role-process-rows.nix";
const MINIJAIL_PROFILES: &str = "nixos-modules/minijail-profiles.nix";
const PROCESSES: &str = "nixos-modules/processes-json.nix";
const DEVICE_PROVIDER: &str = "nixos-modules/provider-registry-v2-extensions/device.nix";

fn assert_canonical_profile(dag: &VmProcessDag, node: &ProcessNode, seccomp: &str) {
    let identity = dag
        .workload_identity
        .as_ref()
        .expect("device role must carry realm workload identity");
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
fn graphics_source_uses_canonical_role_resource_provider_rows() {
    let devices = read_repo_file(DEVICE_ROWS);
    let roles = read_repo_file(ROLE_ROWS);
    let profiles = read_repo_file(MINIJAIL_PROFILES);
    let processes = read_repo_file(PROCESSES);
    let provider = read_repo_file(DEVICE_PROVIDER);

    for required in [
        r#"gpu = "gpu";"#,
        r#""render-node" = "gpu-render-node";"#,
        r#"gpu = "gpu-cross-domain";"#,
        r#""render-node" = "mediated-device";"#,
        r#"resourceId = "device-render-node-global";"#,
        r#"share = "shared-partition";"#,
        r#"attachment = "fd-only";"#,
    ] {
        assert!(
            devices.contains(required),
            "graphics realm resource policy missing {required:?} from {DEVICE_ROWS}"
        );
    }
    assert!(roles.contains("cfg._index.devices.byRoleId"));
    assert!(
        profiles.contains(r#"else if processRole == "gpu""#)
            && profiles.contains(r#"then [ "/dev/dri/renderD128" ]"#)
            && profiles.contains(r#"role.processRole == "gpu-render-node""#)
            && profiles.contains("hostUidForZero = principalId")
    );
    assert!(
        processes.contains(r#"role.roleKind == "gpu-render-node""#)
            && processes.contains(r#""/proc/self/fd/10""#)
            && processes.contains(r#"role.roleKind == "gpu""#)
    );
    assert!(
        provider.contains("deviceResourceIds")
            && provider.contains("row.providerId == provider.providerId")
    );
    for forbidden in ["/dev/nvidia", "/dev/vfio", "/dev/udmabuf"] {
        assert!(
            !profiles.contains(forbidden),
            "canonical minijail profiles must not broadly bind {forbidden}"
        );
    }
}

#[test]
fn graphics_rendered_profiles_are_role_scoped() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset; graphics role fixture unavailable");
        return;
    };
    let mut gpu_seen = 0usize;
    let mut render_seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            match node.role {
                ProcessRole::Gpu => {
                    gpu_seen += 1;
                    assert_canonical_profile(dag, node, "w1-gpu");
                    assert_eq!(
                        node.profile.mount_policy.device_binds,
                        vec!["/dev/dri/renderD128".to_owned()]
                    );
                    assert!(node.profile.user_namespace.is_none());
                    assert_eq!(node.profile.umask, Some(7));
                }
                ProcessRole::GpuRenderNode => {
                    render_seen += 1;
                    assert_canonical_profile(dag, node, "w1-gpu-render-node");
                    assert!(node.profile.mount_policy.device_binds.is_empty());
                    let user_ns = node
                        .profile
                        .user_namespace
                        .as_ref()
                        .expect("render-node profile must use broker-prepared user namespace");
                    assert_eq!(user_ns.host_uid_for_zero, node.profile.uid);
                    assert_eq!(user_ns.host_gid_for_zero, node.profile.gid);
                    assert_eq!(node.profile.umask, Some(7));
                    assert!(node.argv.iter().any(|arg| arg == "/proc/self/fd/10"));
                }
                _ => {}
            }
        }
    }
    assert_eq!(gpu_seen, 1, "fixture-smoke-full must render one GPU role");
    assert_eq!(
        render_seen, 1,
        "fixture-smoke-full must render one alternative render-node role"
    );
}

#[test]
fn wayland_proxy_rendered_profile_is_role_scoped() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset; Wayland role fixture unavailable");
        return;
    };
    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::WaylandProxy {
                continue;
            }
            seen += 1;
            assert_canonical_profile(dag, node, "w1-wayland-proxy");
            assert!(node.profile.mount_policy.device_binds.is_empty());
            assert!(node.profile.mount_policy.bind_mounts.is_empty());
            assert!(node.profile.user_namespace.is_none());
            assert_eq!(node.profile.umask, Some(7));
            let identity = dag.workload_identity.as_ref().unwrap();
            let runtime = format!(
                "/run/d2b/r/{}/w/{}/roles/{}",
                identity.realm_id.as_str(),
                identity.workload_id.as_str(),
                node.id.0
            );
            assert!(node
                .profile
                .mount_policy
                .writable_paths
                .iter()
                .any(|path| path.path == runtime));
        }
    }
    assert_eq!(
        seen, 2,
        "both graphics fixture workloads must render Wayland proxy roles"
    );
}
