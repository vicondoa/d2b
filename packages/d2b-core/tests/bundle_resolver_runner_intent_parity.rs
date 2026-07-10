#![forbid(unsafe_code)]

use d2b_core::bundle::{Bundle, BundleGeneration};
use d2b_core::bundle_resolver::{BundleResolver, ResolvedRunnerIntent};
use d2b_core::host::HostJson;
use d2b_core::manifest_v04::ManifestV04;
use d2b_core::minijail_profile::{CgroupPlacement, MountPolicy, NamespaceSet};
use d2b_core::process_builder::ProcessNodeBuilder;
use d2b_core::processes::{
    ProcessNode, ProcessRole, ProcessesJson, RoleProfile, RoleUserNamespace, VmProcessDag,
    VmProcessInvariants,
};

const VM: &str = "test-vm";

fn host() -> HostJson {
    serde_json::from_value(serde_json::json!({
        "schemaVersion": "v2",
        "site": { "allowUnsafeEastWest": false },
        "environments": [],
        "nftables": {
            "family": "inet",
            "table": "d2b",
            "chains": [],
            "tableHashAfterApply": null,
            "ownershipId": "test"
        },
        "networkManager": {
            "filePath": "/etc/NetworkManager/conf.d/00-d2b-unmanaged.conf",
            "matchCriteria": [],
            "reloadBehavior": "atomic-reload",
            "ownership": {
                "owner": "root",
                "group": "root",
                "mode": "0644",
                "driftPolicy": "replace"
            }
        },
        "hostsFile": {
            "startMarker": "# d2b-managed begin",
            "endMarker": "# d2b-managed end",
            "rule": "replace-managed-block"
        },
        "kernelModules": [],
        "fdOwnership": [],
        "runtimeProviders": [],
        "vmRuntimes": [],
        "cloudHypervisorCapabilities": [],
        "ifNameMappings": [],
        "qemuMedia": null,
        "ch": null,
        "firewallCoexistencePolicy": null
    }))
    .expect("host fixture parses")
}

fn manifest() -> ManifestV04 {
    ManifestV04::from_slice(
        serde_json::to_vec(&serde_json::json!({
            "_manifest": { "manifestVersion": 6 },
            "_observability": {
                "enabled": false,
                "signozUrl": "http://127.0.0.1:8080",
                "signozOtlpGrpcPort": 4317,
                "signozOtlpHttpPort": 4318,
                "obsVsockCid": 0,
                "obsVsockHostSocket": "",
                "vmName": ""
            }
        }))
        .expect("manifest json serializes")
        .as_slice(),
    )
    .expect("manifest fixture parses")
}

fn bundle() -> Bundle {
    Bundle {
        bundle_version: 4,
        schema_version: "v2".to_owned(),
        public_manifest_path: "vms.json".to_owned(),
        host_path: "host.json".to_owned(),
        processes_path: "processes.json".to_owned(),
        privileges_path: "privileges.json".to_owned(),
        storage_path: None,
        sync_path: None,
        allocator_path: None,
        realm_controllers_path: None,
        realm_identity_path: None,
        unsafe_local_workloads_path: None,
        closures: Vec::new(),
        minijail_profiles: Vec::new(),
        managed_keys: Default::default(),
        generation: BundleGeneration {
            generator: "test".to_owned(),
            source_revision: None,
            generated_at: None,
        },
        bundle_hash: None,
        artifact_hashes: None,
    }
}

fn profile(role_id: &str) -> RoleProfile {
    RoleProfile {
        profile_id: format!("profile-{role_id}"),
        uid: 60_100,
        gid: 60_100,
        adr_carve_out: None,
        caps: vec![],
        namespaces: NamespaceSet {
            mount: true,
            pid: false,
            net: role_id == "audio",
            ipc: false,
            uts: false,
            user: matches!(role_id, "virtiofsd" | "swtpm" | "gpu" | "audio" | "video"),
        },
        seccomp_policy_ref: Some(format!("policy-{role_id}")),
        mount_policy: MountPolicy {
            read_only_paths: vec!["/nix/store".to_owned()],
            writable_paths: vec![],
            nix_store_read_only: true,
            hide_device_nodes_by_default: true,
            device_binds: vec![],
            bind_mounts: vec![],
        },
        cgroup_placement: CgroupPlacement {
            subtree: format!("d2b.slice/{VM}/{role_id}"),
            controllers: vec!["cpu".to_owned(), "memory".to_owned()],
            delegated: false,
        },
        user_namespace: matches!(role_id, "virtiofsd" | "swtpm" | "gpu" | "audio" | "video")
            .then_some(RoleUserNamespace {
                host_uid_for_zero: 60_100,
                host_gid_for_zero: 60_100,
            }),
        umask: Some(0o007),
    }
}

fn runner_node(
    id: &str,
    role: ProcessRole,
    binary_path: &str,
    argv: &[&str],
    env: &[&str],
) -> ProcessNode {
    ProcessNodeBuilder::new(role, profile(id))
        .with_id(id)
        .with_binary_path(binary_path)
        .with_argv(argv.iter().copied())
        .with_env(env.iter().copied())
        .build()
        .expect("runner node fixture builds")
}

fn runner_nodes() -> Vec<ProcessNode> {
    vec![
        runner_node(
            "cloud-hypervisor",
            ProcessRole::CloudHypervisorRunner,
            "/run/current-system/sw/bin/cloud-hypervisor",
            &["microvm@test-vm"],
            &[],
        ),
        runner_node(
            "virtiofsd",
            ProcessRole::Virtiofsd,
            "/run/current-system/sw/bin/virtiofsd",
            &["microvm-virtiofsd@test-vm"],
            &[],
        ),
        runner_node(
            "swtpm",
            ProcessRole::Swtpm,
            "/run/current-system/sw/bin/swtpm",
            &["microvm-swtpm@test-vm"],
            &[],
        ),
        runner_node(
            "gpu",
            ProcessRole::Gpu,
            "/run/current-system/sw/bin/crosvm",
            &["d2b-test-vm-gpu"],
            &["XDG_RUNTIME_DIR=/run/d2b/test-vm/gpu"],
        ),
        runner_node(
            "audio",
            ProcessRole::Audio,
            "/run/current-system/sw/bin/vhost-device-sound",
            &["d2b-test-vm-snd"],
            &["PIPEWIRE_RUNTIME_DIR=/run/d2b/test-vm/audio"],
        ),
        runner_node(
            "video",
            ProcessRole::Video,
            "/run/current-system/sw/bin/crosvm",
            &["d2b-test-vm-video"],
            &["XDG_RUNTIME_DIR=/run/d2b/test-vm/video"],
        ),
        runner_node(
            "qemu-media",
            ProcessRole::QemuMediaRunner,
            "/run/current-system/sw/bin/qemu-system-x86_64",
            &["qemu-system-x86_64", "-S", "-qmp", "unix:/run/d2b/qmp.sock"],
            &[],
        ),
        runner_node(
            "vsock-relay",
            ProcessRole::VsockRelay,
            "/run/current-system/sw/bin/socat",
            &[
                "d2b-otel-relay@test-vm",
                "-d",
                "-d",
                "UNIX-LISTEN:/var/lib/d2b/vms/test-vm/vsock.sock_14317,fork,max-children=16,reuseaddr,mode=0660",
                "EXEC:/run/current-system/sw/bin/d2b-ch-vsock-connect /var/lib/d2b/vms/sys-obs/vsock.sock 14318",
            ],
            &[],
        ),
        runner_node(
            "usbip",
            ProcessRole::Usbip,
            "/run/current-system/sw/bin/d2b-usbip-proxy",
            &["d2b-usbip-proxy", "--vm", VM],
            &[],
        ),
        runner_node(
            "otel-host-bridge",
            ProcessRole::OtelHostBridge,
            "/run/current-system/sw/bin/socat",
            &[
                "d2b-otel-host-bridge",
                "-d",
                "-d",
                "UNIX-LISTEN:/run/d2b/otel/host-egress.sock,fork,reuseaddr,mode=0660",
                "EXEC:\"/run/current-system/sw/bin/d2b-ch-vsock-connect /var/lib/d2b/vms/sys-obs/vsock.sock 14317\"",
            ],
            &[],
        ),
    ]
}

#[test]
fn bundle_resolver_runner_intents_match_typed_process_node_helper_for_runner_roles() {
    let nodes = runner_nodes();
    let processes = ProcessesJson {
        schema_version: "v2".to_owned(),
        vms: vec![VmProcessDag {
            workload_identity: None,
            vm: VM.to_owned(),
            nodes: nodes.clone(),
            edges: Vec::new(),
            invariants: VmProcessInvariants {
                swtpm_pre_start_flush: false,
                per_vm_audit_pipeline: false,
                usbip_gating: true,
                tpm_ownership_migration_without_running_vm_mutation: true,
            },
        }],
    };
    let resolver = BundleResolver::from_artifacts(bundle(), host(), processes, manifest());

    for node in &nodes {
        let expected = ResolvedRunnerIntent::from_process_node(VM, node)
            .expect("spawnable role resolves through helper");
        let actual = resolver
            .find_runner_intent(&expected.intent_id)
            .expect("resolver exposes helper-derived runner intent");
        assert_eq!(
            actual, &expected,
            "runner intent parity for {:?}",
            node.role
        );
    }
    assert_eq!(resolver.runner_intent_ids().count(), nodes.len());
}
