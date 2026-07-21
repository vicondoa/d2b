//! Contract coverage for realm-owned observability relay profiles.

use std::collections::BTreeSet;

use d2b_contract_tests::{load_full_bundle_resolver_from_env, read_repo_file};
use d2b_contracts::broker_wire::RunnerRole;
use d2b_contracts::v2_identity::{RealmId, RealmPath, RoleId, RoleKind, WorkloadId, WorkloadName};
use d2b_core::processes::ProcessRole;
use d2b_host::otel_host_bridge_argv::{OtelHostBridgeArgvInputs, generate_otel_host_bridge_argv};

const MINIJAIL_PROFILES_NIX: &str = "nixos-modules/minijail-profiles.nix";
const PRIV_BROKER_RUNTIME_RS: &str = "packages/d2b-priv-broker/src/runtime.rs";
const PRIV_BROKER_LIVE_HANDLERS_RS: &str = "packages/d2b-priv-broker/src/live_handlers.rs";

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
            if !matches!(
                node.role,
                ProcessRole::VsockRelay | ProcessRole::OtelHostBridge
            ) {
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
                Some(match node.role {
                    ProcessRole::OtelHostBridge => "w1-otel-host-bridge",
                    ProcessRole::VsockRelay => "w1-vsock-relay",
                    _ => unreachable!("filtered relay role"),
                }),
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
            let workload_state = format!("/var/lib/d2b/r/{realm_id}/w/{}", dag.vm);
            let expected_writable_paths = match node.role {
                ProcessRole::OtelHostBridge => {
                    vec![role_runtime.as_str(), workload_state.as_str()]
                }
                ProcessRole::VsockRelay => vec![role_runtime.as_str()],
                _ => unreachable!("filtered relay role"),
            };
            assert_eq!(
                profile
                    .mount_policy
                    .writable_paths
                    .iter()
                    .map(|row| row.path.as_str())
                    .collect::<Vec<_>>(),
                expected_writable_paths,
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
            match node.role {
                ProcessRole::OtelHostBridge => assert!(
                    node.binary_path.is_some() && !node.argv.is_empty() && node.unit.is_none(),
                    "OTel host bridge {} must be a controller-owned process node",
                    node.id.0
                ),
                ProcessRole::VsockRelay => assert!(
                    node.binary_path.is_none() && node.argv.is_empty() && node.unit.is_none(),
                    "realm relay {} must remain a declarative controller-owned role row",
                    node.id.0
                ),
                _ => unreachable!("filtered relay role"),
            }
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

// ---------------------------------------------------------------------------
// ProcessRole::OtelHostBridge runner/matrix coverage.
//
// The host-bridge sidecar is rendered as the observability workload's
// controller-owned process node. This complementary test proves the closed-set
// Rust contract end to end: the real argv generator emits the pre-opened-fd-only
// socat shape the broker expects, and the broker's SpawnRunner dispatch/seccomp
// source classifies and gates that role exactly.
// ---------------------------------------------------------------------------
#[test]
fn runner_process_roles_have_builder_and_matrix_contract_coverage_otel_host_bridge() {
    // The generator is the actual `d2b-host` argv builder wired for
    // ProcessRole::OtelHostBridge — exercise it with realistic canonical
    // realm-role paths rather than asserting source text alone.
    let inputs = OtelHostBridgeArgvInputs {
        socat_path: "/run/current-system/sw/bin/socat".to_owned(),
        host_egress_socket:
            "/run/d2b/r/cvudgfqzh442wwtozs7q/w/jagsccyorsii4fm3u6vq/roles/chgqvca2e5gtb6vypzza/host-egress.sock"
                .to_owned(),
        obs_vsock_host_socket:
            "/var/lib/d2b/r/cvudgfqzh442wwtozs7q/w/jagsccyorsii4fm3u6vq/vsock.sock".to_owned(),
        obs_otlp_port: 14317,
        ch_vsock_connect_path: "/run/current-system/sw/bin/d2b-ch-vsock-connect".to_owned(),
    };
    let argv = generate_otel_host_bridge_argv(&inputs)
        .expect("closed-set OtelHostBridge inputs must produce argv");
    assert_eq!(argv.first(), Some(&inputs.socat_path));
    assert!(
        argv.iter()
            .any(|arg| arg.starts_with("UNIX-LISTEN:") && arg.contains(&inputs.host_egress_socket)),
        "OtelHostBridge argv must listen on its canonical role-owned host-egress socket"
    );
    assert!(
        argv.iter().any(|arg| arg.starts_with("EXEC:")
            && arg.contains(&inputs.ch_vsock_connect_path)
            && arg.contains(&inputs.obs_vsock_host_socket)
            && arg.contains(&inputs.obs_otlp_port.to_string())),
        "OtelHostBridge argv must bridge to the obs VM's vsock socket via the CH connect helper"
    );
    assert!(
        !argv.iter().any(|arg| arg.contains("/dev")),
        "OtelHostBridge argv must never reference a host device path (pre-opened fds only)"
    );

    // The broker's own SpawnRunner classification must route
    // ProcessRole::OtelHostBridge to RunnerRole::OtelHostBridge — the same
    // dispatch table entry the VsockRelay role above uses.
    let runtime_src = read_repo_file(PRIV_BROKER_RUNTIME_RS);
    assert!(
        runtime_src.contains("ProcessRole::OtelHostBridge => Some(RunnerRole::OtelHostBridge)"),
        "the broker must classify ProcessRole::OtelHostBridge as RunnerRole::OtelHostBridge for SpawnRunner dispatch"
    );
    assert!(
        runtime_src.contains("d2b_contracts::broker_wire::RunnerRole::OtelHostBridge"),
        "SpawnRunner dispatch must reference the wire RunnerRole::OtelHostBridge variant"
    );
    assert!(
        runtime_src.contains("intent.vm_name != resolver.manifest.observability.vm_name"),
        "SpawnRunner must refuse an OtelHostBridge intent whose VM disagrees with the bundle's observability VM"
    );
    assert!(
        runtime_src.contains("OtelHostBridgeIntentInvalid"),
        "an OtelHostBridge/obs-VM mismatch must surface the typed OtelHostBridgeIntentInvalid error"
    );
    assert_eq!(
        RunnerRole::OtelHostBridge.as_str(),
        "otel-host-bridge",
        "RunnerRole::OtelHostBridge wire tag must stay stable for broker audit/dispatch"
    );

    // Seccomp posture: the folded-in bridge role must keep the same
    // pre-opened-fd-only, device-ioctl-free BPF matrix as VsockRelay.
    let live_handlers_src = read_repo_file(PRIV_BROKER_LIVE_HANDLERS_RS);
    assert!(
        live_handlers_src.contains(r#""w1-vsock-relay" | "w1-otel-host-bridge" => Some(&[])"#),
        "w1-otel-host-bridge must keep the device-ioctl-free BPF matrix alongside w1-vsock-relay"
    );
}
