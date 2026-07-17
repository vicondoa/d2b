//! Contract coverage for realm-owned observability relay profiles.

use std::collections::BTreeSet;

use d2b_contract_tests::{load_full_bundle_resolver_from_env, read_repo_file};
use d2b_contracts::v2_identity::{RealmId, RealmPath, RoleId, RoleKind, WorkloadId, WorkloadName};
use d2b_core::processes::ProcessRole;

const MINIJAIL_PROFILES_NIX: &str = "nixos-modules/minijail-profiles.nix";

fn observability_role_id() -> RoleId {
    let realm_id = RealmId::derive(&RealmPath::root());
    let workload_id = WorkloadId::derive(
        &realm_id,
        &WorkloadName::parse("sys-obs").expect("valid fixture workload name"),
    );
    RoleId::derive(&realm_id, &workload_id, RoleKind::VsockRelay)
}

fn realm_id_from_subtree<'a>(subtree: &'a str, workload_id: &str, role_id: &str) -> &'a str {
    let suffix = format!("/workloads/w-{workload_id}/{role_id}");
    let realm = subtree
        .strip_prefix("d2b.slice/r-")
        .and_then(|value| value.strip_suffix(&suffix))
        .unwrap_or_else(|| {
            panic!(
                "relay cgroup subtree must use the canonical realm/workload/role shape: {subtree}"
            )
        });
    assert_eq!(realm.len(), 20, "realm id must be a canonical short id");
    realm
}

#[test]
fn relay_profile_source_uses_canonical_role_rows() {
    let src = read_repo_file(MINIJAIL_PROFILES_NIX);

    for token in [
        r#""vsock-relay" = "w1-vsock-relay";"#,
        "profileForRole = role:",
        "profileId = profileIdFor role.nodeId;",
        r#""d2b.slice/r-${role.realmId}/workloads/w-${role.workloadId}/${role.roleId}""#,
        r#"(writable roleRuntime "Create only this role's runtime endpoints.")"#,
    ] {
        assert!(
            src.contains(token),
            "realm relay profiles must be derived from canonical role rows; missing {token:?}"
        );
    }

    assert!(
        !src.contains("host-otel-host-bridge")
            && !src.contains("vm-corp-full-vsock-relay")
            && !src.contains("d2b.slice/host/otel-host-bridge"),
        "realm relay profiles must not retain singleton host/VM profile identities"
    );
}

#[test]
fn rendered_relay_profiles_are_realm_scoped_and_declarative() {
    let Some(resolver) = load_full_bundle_resolver_from_env() else {
        eprintln!("SKIP: D2B_FIXTURES_FULL unset (realm relay profile shape)");
        return;
    };

    let expected_observability_role = observability_role_id();
    let mut role_ids = BTreeSet::new();
    let mut workload_ids = BTreeSet::new();

    for dag in &resolver.processes.vms {
        for node in &dag.nodes {
            if node.role != ProcessRole::VsockRelay {
                continue;
            }

            assert!(
                role_ids.insert(node.id.0.clone()),
                "relay role id {} must be globally unique",
                node.id.0
            );
            workload_ids.insert(dag.vm.clone());

            let profile = &node.profile;
            assert_eq!(
                profile.profile_id,
                format!("role-{}", node.id.0),
                "relay profile id must be derived from its canonical role id"
            );
            assert!(
                profile.caps.is_empty(),
                "realm relay {} must use pre-opened descriptors with no capabilities",
                node.id.0
            );
            assert_eq!(
                profile.seccomp_policy_ref.as_deref(),
                Some("w1-vsock-relay"),
                "realm relay {} seccomp policy drift",
                node.id.0
            );
            assert!(
                profile.mount_policy.device_binds.is_empty()
                    && !profile
                        .mount_policy
                        .writable_paths
                        .iter()
                        .any(|row| row.path.starts_with("/dev")),
                "realm relay {} must not receive device paths",
                node.id.0
            );

            let realm_id =
                realm_id_from_subtree(&profile.cgroup_placement.subtree, &dag.vm, &node.id.0);
            let role_runtime = format!("/run/d2b/r/{realm_id}/w/{}/roles/{}", dag.vm, node.id.0);
            assert_eq!(
                profile
                    .mount_policy
                    .writable_paths
                    .iter()
                    .map(|row| row.path.as_str())
                    .collect::<Vec<_>>(),
                vec![role_runtime.as_str()],
                "realm relay {} must write only its canonical role runtime",
                node.id.0
            );
            assert!(
                profile.namespaces.mount
                    && profile.namespaces.ipc
                    && !profile.namespaces.net
                    && !profile.namespaces.user,
                "realm relay {} namespace posture drift",
                node.id.0
            );
            assert!(
                node.binary_path.is_none() && node.argv.is_empty() && node.unit.is_none(),
                "realm relay {} must remain a declarative controller-owned role row",
                node.id.0
            );
        }
    }

    assert!(
        role_ids.contains(expected_observability_role.as_str()),
        "the realm-owned observability workload must expose its canonical relay role"
    );
    assert!(
        workload_ids.len() > 1,
        "the feature-rich fixture must cover relay roles without a singleton workload assumption"
    );
}
