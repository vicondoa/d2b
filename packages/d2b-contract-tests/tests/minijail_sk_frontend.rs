use d2b_contract_tests::{load_full_bundle_resolver_from_env, read_repo_file};
use d2b_core::processes::ProcessRole;

const DEVICE_ROWS: &str = "nixos-modules/realm-device-rows.nix";
const ROLE_ROWS: &str = "nixos-modules/role-process-rows.nix";
const MINIJAIL_PROFILES: &str = "nixos-modules/minijail-profiles.nix";
const PROCESSES: &str = "nixos-modules/processes-json.nix";
const DEVICE_PROVIDER: &str = "nixos-modules/provider-registry-v2-extensions/device.nix";

#[test]
fn security_key_frontend_process_role_variant_exists() {
    let role = ProcessRole::SecurityKeyFrontend;
    assert_eq!(
        serde_json::to_string(&role).expect("serialize ProcessRole"),
        "\"security-key-frontend\""
    );
}

#[test]
fn security_key_source_uses_canonical_realm_rows() {
    let devices = read_repo_file(DEVICE_ROWS);
    let roles = read_repo_file(ROLE_ROWS);
    let profiles = read_repo_file(MINIJAIL_PROFILES);
    let processes = read_repo_file(PROCESSES);
    let provider = read_repo_file(DEVICE_PROVIDER);

    for required in [
        r#"fido = "security-key-frontend";"#,
        r#"fido = "fido-ceremony";"#,
        r#"resourceId = "device-security-key-global";"#,
        r#"share = "exclusive";"#,
        r#"attachment = "fd-only";"#,
        r#"broker = "realm-local";"#,
        r#"else if kind == "fido" then "${roleRoot}/security-key.sock""#,
    ] {
        assert!(
            devices.contains(required),
            "FIDO realm resource policy missing {required:?} from {DEVICE_ROWS}"
        );
    }
    assert!(roles.contains("cfg._index.devices.byRoleId"));
    assert!(
        profiles.contains(r#""security-key-frontend" = "w1-security-key-frontend";"#)
            && profiles.contains("profileForRole")
    );
    assert!(
        processes.contains(r#"role.roleKind == "security-key-frontend""#)
            && processes.contains("allocator-owned security-key endpoint")
    );
    assert!(
        provider.contains("deviceResourceIds")
            && provider.contains("row.providerId == provider.providerId")
    );
    for forbidden in ["hidraw", "/dev/bus/usb", "busid", "busId"] {
        assert!(
            !devices.contains(forbidden),
            "FIDO realm resource rows must not expose raw physical selector {forbidden:?}"
        );
    }
}

#[test]
fn security_key_frontend_rendered_profile_is_no_runner_tracker() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset; security-key role fixture unavailable");
        return;
    };
    let mut seen = 0usize;
    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::SecurityKeyFrontend {
                continue;
            }
            seen += 1;
            let identity = dag
                .workload_identity
                .as_ref()
                .expect("security-key node must carry realm workload identity");
            let profile = &node.profile;
            assert_eq!(profile.profile_id, format!("role-{}", node.id.0));
            assert_eq!(
                profile.cgroup_placement.subtree,
                format!(
                    "d2b.slice/r-{}/workloads/w-{}/{}",
                    identity.realm_id.as_str(),
                    identity.workload_id.as_str(),
                    node.id.0
                )
            );
            assert_eq!(
                profile.seccomp_policy_ref.as_deref(),
                Some("w1-security-key-frontend")
            );
            assert!(profile.caps.is_empty());
            assert!(profile.mount_policy.device_binds.is_empty());
            assert!(profile.user_namespace.is_none());
            assert!(node.binary_path.is_none() && node.argv.is_empty());
        }
    }
    assert_eq!(
        seen, 1,
        "fixture-smoke-full must render one allocator-owned security-key tracker"
    );
}
