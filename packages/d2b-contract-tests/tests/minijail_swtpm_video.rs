use d2b_contract_tests::{load_full_bundle_resolver_from_env, read_repo_file};
use d2b_core::processes::{ProcessNode, ProcessRole, VmProcessDag};

const DEVICE_ROWS: &str = "nixos-modules/realm-device-rows.nix";
const ROLE_ROWS: &str = "nixos-modules/role-process-rows.nix";
const MINIJAIL_PROFILES: &str = "nixos-modules/minijail-profiles.nix";
const PROCESSES: &str = "nixos-modules/processes-json.nix";
const DEVICE_PROVIDER: &str = "nixos-modules/provider-registry-v2-extensions/device.nix";

fn full_resolver_or_skip(test: &str) -> Option<d2b_core::bundle_resolver::BundleResolver> {
    match load_full_bundle_resolver_from_env() {
        Some(resolver) => Some(resolver),
        None => {
            eprintln!("SKIP {test}: D2B_FIXTURES_FULL unset");
            None
        }
    }
}

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
}

#[test]
fn tpm_source_uses_persistent_realm_resource_and_exclusive_lease() {
    let devices = read_repo_file(DEVICE_ROWS);
    let roles = read_repo_file(ROLE_ROWS);
    let profiles = read_repo_file(MINIJAIL_PROFILES);
    let processes = read_repo_file(PROCESSES);
    let provider = read_repo_file(DEVICE_PROVIDER);

    for required in [
        r#"tpm = "swtpm";"#,
        r#"tpm = "tpm2-stateful";"#,
        r#"resourceId = "device-tpm-${workload.workloadId}";"#,
        r#"share = "exclusive";"#,
        r#""workload/${workload.workloadId}/tpm""#,
        r#"attachment = "fd-only";"#,
    ] {
        assert!(
            devices.contains(required),
            "TPM realm resource policy missing {required:?} from {DEVICE_ROWS}"
        );
    }
    assert!(roles.contains("cfg._index.devices.byRoleId"));
    assert!(
        profiles.contains(r#""swtpm-pre-start-flush" = "w1-swtpm";"#)
            && profiles.contains(r#"swtpm = "w1-swtpm";"#)
            && profiles.contains(r#"if builtins.elem role.processRole"#)
            && profiles.contains(r#"[ "swtpm" "swtpm-pre-start-flush" ]"#)
            && profiles.contains(r#""${state}/tpm""#)
    );
    assert!(
        processes.contains(r#"role.roleKind == "swtpm-pre-start-flush""#)
            && processes.contains(r#"role.roleKind == "swtpm""#)
            && processes.contains(r#"dir=${workload.stateRoot}/tpm"#)
    );
    assert!(provider.contains("deviceResourceIds"));
    assert!(
        !profiles
            .lines()
            .any(|line| line.contains("tmpfs") && line.contains("tpm")),
        "TPM state must never move to tmpfs"
    );
}

#[test]
fn swtpm_rendered_profiles_preserve_state_and_role_isolation() {
    let Some(resolver) =
        full_resolver_or_skip("swtpm_rendered_profiles_preserve_state_and_role_isolation")
    else {
        return;
    };
    let mut swtpm_seen = 0usize;
    let mut flush_seen = 0usize;
    for dag in &resolver.processes.vms {
        let identity = dag.workload_identity.as_ref();
        for node in &dag.nodes {
            match node.role {
                ProcessRole::Swtpm => {
                    swtpm_seen += 1;
                    assert_canonical_profile(dag, node, "w1-swtpm");
                    let identity = identity.unwrap();
                    let state = format!(
                        "/var/lib/d2b/r/{}/w/{}/tpm",
                        identity.realm_id.as_str(),
                        identity.workload_id.as_str()
                    );
                    assert!(
                        node.profile
                            .mount_policy
                            .writable_paths
                            .iter()
                            .any(|path| path.path == state)
                    );
                    assert!(node.profile.user_namespace.is_none());
                    assert!(node.profile.mount_policy.device_binds.is_empty());
                    assert!(node.profile.mount_policy.hide_device_nodes_by_default);
                    assert!(node.profile.mount_policy.nix_store_read_only);
                    assert_eq!(node.profile.umask, Some(7));
                    assert!(node.argv.iter().any(|arg| arg == &format!("dir={state}")));
                }
                ProcessRole::SwtpmPreStartFlush => {
                    flush_seen += 1;
                    assert_canonical_profile(dag, node, "w1-swtpm");
                    assert!(node.profile.user_namespace.is_none());
                }
                _ => {}
            }
        }
    }
    assert_eq!(
        swtpm_seen, 1,
        "fixture must render one persistent swtpm role"
    );
    assert_eq!(flush_seen, 1, "fixture must render one swtpm flush role");
}

#[test]
fn video_source_uses_shared_render_lease_and_role_endpoint() {
    let devices = read_repo_file(DEVICE_ROWS);
    let profiles = read_repo_file(MINIJAIL_PROFILES);
    let processes = read_repo_file(PROCESSES);

    for required in [
        r#"video = "video";"#,
        r#"video = "video-decode";"#,
        r#"resourceId = "device-render-node-global";"#,
        r#"share = "shared-partition";"#,
        r#"else if kind == "video" then "${roleRoot}/video.sock""#,
        r#"attachment = "fd-only";"#,
    ] {
        assert!(
            devices.contains(required),
            "video realm resource policy missing {required:?} from {DEVICE_ROWS}"
        );
    }
    assert!(
        profiles.contains(r#"else if processRole == "video""#)
            && profiles.contains(r#"then [ "/dev/dri/renderD128" ]"#)
            && !profiles.contains("/dev/nvidia")
    );
    assert!(
        processes.contains(r#"role.roleKind == "video""#)
            && processes.contains(r#""${runtime}/video.sock""#)
            && processes.contains(r#""--backend" "vaapi""#)
    );
}

#[test]
fn video_rendered_profile_is_role_scoped_and_gpu_distinct() {
    let Some(resolver) =
        full_resolver_or_skip("video_rendered_profile_is_role_scoped_and_gpu_distinct")
    else {
        return;
    };
    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        let gpu_uids: Vec<u32> = dag
            .nodes
            .iter()
            .filter(|node| matches!(node.role, ProcessRole::Gpu | ProcessRole::GpuRenderNode))
            .map(|node| node.profile.uid)
            .collect();
        for node in &dag.nodes {
            if node.role != ProcessRole::Video {
                continue;
            }
            seen += 1;
            assert_canonical_profile(dag, node, "w1-video");
            assert_eq!(
                node.profile.mount_policy.device_binds,
                vec!["/dev/dri/renderD128".to_owned()]
            );
            assert!(node.profile.user_namespace.is_none());
            assert_eq!(node.profile.umask, Some(7));
            assert!(!gpu_uids.contains(&node.profile.uid));
            let identity = dag.workload_identity.as_ref().unwrap();
            let endpoint = format!(
                "/run/d2b/r/{}/w/{}/roles/{}/video.sock",
                identity.realm_id.as_str(),
                identity.workload_id.as_str(),
                node.id.0
            );
            assert!(node.argv.iter().any(|arg| arg == &endpoint));
        }
    }
    assert_eq!(seen, 1, "fixture must render one mediated video role");
}
