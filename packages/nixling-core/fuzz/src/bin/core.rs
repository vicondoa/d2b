#[path = "../harness.rs"]
mod harness;

use nixling_core::{
    bundle::{Bundle, BundleGeneration, BundleManagedKeys},
    bundle_resolver::{
        intent_id_activation, intent_id_gc_host, intent_id_hosts_host, intent_id_keys_rotate,
        intent_id_nft_env, intent_id_nft_host, intent_id_nm_unmanaged_host,
        intent_id_rotate_known_host, intent_id_route_env, intent_id_runner, intent_id_socket,
        intent_id_sysctl, intent_id_trust, intent_id_usbip_bind, intent_id_usbip_firewall,
        BundleResolver,
    },
    closures::{ClosureGeneration, ClosureMetadata},
    error::{BrokerOp, Error, SemverRange, Version},
    host::{
        BridgePortFlags, HostJson, HostsFileOwnership, IfName, IfNameError, Ipv6SysctlEntry,
        LanPolicy, NetEnv, NetworkManagerUnmanaged, NftChain, NftablesModel, OwnershipRule,
        SitePolicy, TapRole, UsbipBusidLock, UsbipLockOwner, UsbipLockScope,
    },
    manifest_v04::ManifestV04,
    minijail_profile::CgroupPlacement,
    privileges::{PrivilegesJson, BROKER_OPERATION_AUTHZ, PUBLIC_OPERATION_AUTHZ},
    processes::{
        DagEdge, NodeId, ProcessNode, ProcessRole, ProcessesJson, ReadinessPredicate, VmProcessDag,
        VmProcessInvariants,
    },
};
#[cfg(not(feature = "test-support"))]
use nixling_core::{
    minijail_profile::{MountPolicy, NamespaceSet},
    processes::RoleProfile,
};

fn main() {
    harness::run_named_tests(&[
        (
            "if_name_accepts_safe_linux_names",
            if_name_accepts_safe_linux_names,
        ),
        (
            "if_name_rejects_invalid_names",
            if_name_rejects_invalid_names,
        ),
        (
            "host_json_denies_unknown_fields",
            host_json_denies_unknown_fields,
        ),
        (
            "usbip_busid_lock_round_trips_bus_ids",
            usbip_busid_lock_round_trips_bus_ids,
        ),
        (
            "privileges_json_denies_unknown_fields",
            privileges_json_denies_unknown_fields,
        ),
        (
            "w1_matrix_contains_public_and_broker_rows",
            w1_matrix_contains_public_and_broker_rows,
        ),
        (
            "manifest_baseline_round_trips_compact",
            manifest_baseline_round_trips_compact,
        ),
        (
            "manifest_unknown_reserved_keys_fail_closed",
            manifest_unknown_reserved_keys_fail_closed,
        ),
        (
            "manifest_mismatched_vm_name_is_rejected",
            manifest_mismatched_vm_name_is_rejected,
        ),
        (
            "error_serializes_as_operator_envelope",
            error_serializes_as_operator_envelope,
        ),
        (
            "semver_range_matches_valid_server_version",
            semver_range_matches_valid_server_version,
        ),
        (
            "bundle_op_id_format_strings_are_wire_stable",
            bundle_op_id_format_strings_are_wire_stable,
        ),
        (
            "bundle_resolver_round_trips_nft_intents",
            bundle_resolver_round_trips_nft_intents,
        ),
        (
            "bundle_resolver_round_trips_route_intents",
            bundle_resolver_round_trips_route_intents,
        ),
        (
            "bundle_resolver_round_trips_sysctl_intents",
            bundle_resolver_round_trips_sysctl_intents,
        ),
        (
            "bundle_resolver_round_trips_hosts_intent",
            bundle_resolver_round_trips_hosts_intent,
        ),
        (
            "bundle_resolver_round_trips_nm_unmanaged_intent",
            bundle_resolver_round_trips_nm_unmanaged_intent,
        ),
        (
            "bundle_resolver_round_trips_usbip_intents",
            bundle_resolver_round_trips_usbip_intents,
        ),
        (
            "bundle_resolver_uses_real_bus_ids_when_present",
            bundle_resolver_uses_real_bus_ids_when_present,
        ),
        (
            "bundle_resolver_round_trips_runner_intents",
            bundle_resolver_round_trips_runner_intents,
        ),
        (
            "bundle_resolver_round_trips_socket_intents",
            bundle_resolver_round_trips_socket_intents,
        ),
        (
            "bundle_resolver_round_trips_activation_intents",
            bundle_resolver_round_trips_activation_intents,
        ),
        (
            "bundle_resolver_round_trips_gc_intent",
            bundle_resolver_round_trips_gc_intent,
        ),
        (
            "bundle_resolver_round_trips_key_management_intents",
            bundle_resolver_round_trips_key_management_intents,
        ),
        (
            "bundle_resolver_unknown_intent_returns_none",
            bundle_resolver_unknown_intent_returns_none,
        ),
        (
            "bundle_resolver_intent_ids_are_sorted_and_deterministic",
            bundle_resolver_intent_ids_are_sorted_and_deterministic,
        ),
        (
            "bundle_resolver_minijail_profile_validator_passes_on_fixture",
            bundle_resolver_minijail_profile_validator_passes_on_fixture,
        ),
        (
            "bundle_resolver_minijail_profile_validator_rejects_root_without_carve_out",
            bundle_resolver_minijail_profile_validator_rejects_root_without_carve_out,
        ),
        (
            "bundle_resolver_minijail_profile_validator_rejects_writable_nix_store",
            bundle_resolver_minijail_profile_validator_rejects_writable_nix_store,
        ),
        (
            "bundle_resolver_minijail_validator_rejects_empty_profile_id",
            bundle_resolver_minijail_validator_rejects_empty_profile_id,
        ),
        (
            "bundle_resolver_minijail_validator_rejects_cgroup_outside_nixling",
            bundle_resolver_minijail_validator_rejects_cgroup_outside_nixling,
        ),
        (
            "bundle_resolver_minijail_validator_allows_empty_cgroup_subtree",
            bundle_resolver_minijail_validator_allows_empty_cgroup_subtree,
        ),
        (
            "bundle_resolver_minijail_validator_allows_root_with_carve_out",
            bundle_resolver_minijail_validator_allows_root_with_carve_out,
        ),
        (
            "bundle_resolver_host_runtime_synthesizes_from_ifname_mappings",
            bundle_resolver_host_runtime_synthesizes_from_ifname_mappings,
        ),
    ]);
}

fn if_name_accepts_safe_linux_names() {
    let name = IfName::new("nl-br_1").expect("valid name");
    assert_eq!(name.as_str(), "nl-br_1");
}

fn if_name_rejects_invalid_names() {
    assert_eq!(IfName::new(""), Err(IfNameError::Empty));
    assert_eq!(IfName::new("abcdefghijklmnop"), Err(IfNameError::TooLong));
    assert_eq!(IfName::new("bad.name"), Err(IfNameError::InvalidCharacter));
}

fn host_json_denies_unknown_fields() {
    let err = serde_json::from_str::<HostJson>(r#"{"schemaVersion":"v1","extra":true}"#)
        .expect_err("unknown fields fail closed");
    assert!(err.to_string().contains("unknown field"));
}

fn usbip_busid_lock_round_trips_bus_ids() {
    let lock: UsbipBusidLock = serde_json::from_str(
        r#"{"vm":"work-vm","lockOwner":"daemon","scope":"per-busid","busIds":["1-1.4","2-3"]}"#,
    )
    .expect("busids field deserializes");
    assert_eq!(lock.bus_ids, vec!["1-1.4".to_owned(), "2-3".to_owned()]);
    let rendered = serde_json::to_value(&lock).expect("lock serializes");
    assert_eq!(rendered["busIds"], serde_json::json!(["1-1.4", "2-3"]));
}

fn privileges_json_denies_unknown_fields() {
    let err = serde_json::from_str::<PrivilegesJson>(
        r#"{"schemaVersion":"v1","publicOperations":[],"brokerOperations":[],"extra":true}"#,
    )
    .expect_err("unknown fields fail closed");
    assert!(err.to_string().contains("unknown field"));
}

fn w1_matrix_contains_public_and_broker_rows() {
    let matrix = PrivilegesJson::w1("v1");
    assert_eq!(matrix.public_operations.len(), PUBLIC_OPERATION_AUTHZ.len());
    assert_eq!(matrix.broker_operations.len(), BROKER_OPERATION_AUTHZ.len());
    assert!(matrix
        .broker_operations
        .iter()
        .any(|row| row.operation == "DelegateCgroupV2"));
}

fn manifest_baseline_round_trips_compact() {
    // Embed via include_str! so the fuzz harness does not read outside
    // the cargo sandbox.
    const BASELINE: &str = include_str!("../../../../../tests/golden/vms.json-91d69b0");
    let manifest = ManifestV04::from_slice(BASELINE.as_bytes()).expect("baseline parses");
    let rendered = manifest.to_compact_json().expect("baseline serializes");
    assert_eq!(rendered, BASELINE);
}

fn manifest_unknown_reserved_keys_fail_closed() {
    let error = ManifestV04::from_slice(
        br#"{"_manifest":{"manifestVersion":3},"_observability":{"enabled":false,"vmName":"sys-obs-stack","obsVsockCid":1000,"obsVsockHostSocket":"/var/lib/nixling/vms/sys-obs-stack/vsock.sock","grafanaUrl":"http://10.40.0.10:3000","chExporter":{"listenPort":9101}},"_future":{}}"#,
    )
    .expect_err("reserved keys are closed in v0.4.0 parser");
    assert_eq!(error.kind().as_str(), "manifest-parse-error");
    assert!(error.message().contains("opaque reason: unknown-field"));
}

fn manifest_mismatched_vm_name_is_rejected() {
    let error = ManifestV04::from_slice(
        br#"{"_manifest":{"manifestVersion":3},"_observability":{"enabled":false,"vmName":"sys-obs-stack","obsVsockCid":1000,"obsVsockHostSocket":"/var/lib/nixling/vms/sys-obs-stack/vsock.sock","grafanaUrl":"http://10.40.0.10:3000","chExporter":{"listenPort":9101}},"corp-vm":{"apiSocket":"/var/lib/nixling/vms/corp-vm/corp-vm.sock","audio":false,"audioService":"nixling-corp-vm-snd.service","audioStateFile":"/var/lib/nixling/vms/corp-vm/state/audio-state.json","bridge":"br-work-lan","env":"work","gpuSocket":"/var/lib/nixling/vms/corp-vm/corp-vm-gpu.sock","graphics":false,"isNetVm":false,"name":"wrong-name","netVm":"sys-work-net","observability":{"agentSocket":"/run/nixling/otlp.sock","enabled":false,"vsockCid":110,"vsockHostSocket":"/var/lib/nixling/vms/corp-vm/vsock.sock"},"sshUser":"alice","stateDir":"/var/lib/nixling/vms/corp-vm","staticIp":"10.20.0.10","tap":"work-l10","tpm":false,"tpmSocket":"/run/swtpm/corp-vm/sock","usbipYubikey":false,"usbipdHostIp":"192.0.2.1"}}"#,
    )
    .expect_err("name mismatch fails");
    assert_eq!(error.kind().as_str(), "manifest-parse-error");
    assert!(error.message().contains("opaque reason: name-key-mismatch"));
}

fn error_serializes_as_operator_envelope() {
    let error = Error::broker_unimplemented(BrokerOp::CreateTapFd, 3);
    let json = serde_json::to_value(&error).expect("error serializes");
    assert_eq!(json["kind"], "broker-unimplemented");
    assert_eq!(json["code"], 30);
    assert_eq!(json["owningCommand"], "daemon-api/broker");
    assert_eq!(
        json["docsAnchor"],
        "docs/reference/error-codes.md#broker-unimplemented"
    );
}

fn semver_range_matches_valid_server_version() {
    let range = SemverRange::new(">=0.4.0, <0.5.0").expect("range parses");
    let version = Version::new("0.4.1").expect("version parses");
    assert!(range.allows(&version));
}

fn bundle_op_id_format_strings_are_wire_stable() {
    use nixling_core::bundle_resolver::*;

    assert_eq!(intent_id_nft_host(), "nft:host");
    assert_eq!(intent_id_nft_env("work"), "nft:env:work");
    assert_eq!(intent_id_route_env("work", 0), "route:env:work:0");
    assert_eq!(
        intent_id_sysctl("work", "br-work-lan", "disable_ipv6"),
        "sysctl:env:work:if:br-work-lan:disable_ipv6"
    );
    assert_eq!(intent_id_hosts_host(), "hosts:host");
    assert_eq!(intent_id_nm_unmanaged_host(), "nm-unmanaged:host");
    assert_eq!(
        intent_id_usbip_firewall("work", "1-1.4"),
        "usbip-fw:env:work:bus:1-1.4"
    );
    assert_eq!(
        intent_id_usbip_bind("work", "work-vm", "1-1.4"),
        "usbip-bind:env:work:vm:work-vm:bus:1-1.4"
    );
    assert_eq!(
        intent_id_runner("work-vm", "ch-runner"),
        "runner:vm:work-vm:role:ch-runner"
    );
    assert_eq!(
        intent_id_socket("work-vm", "ch-runner"),
        "socket:vm:work-vm:role:ch-runner"
    );
    assert_eq!(intent_id_installer_host(), "installer:host");
    assert_eq!(intent_id_migrate_host(), "migrate:host");
    assert_eq!(intent_id_activation("work-vm"), "activation:vm:work-vm");
    assert_eq!(intent_id_gc_host(), "gc:host");
    assert_eq!(intent_id_keys_rotate("work-vm"), "keys-rotate:vm:work-vm");
    assert_eq!(intent_id_trust("work-vm"), "trust:vm:work-vm");
    assert_eq!(
        intent_id_rotate_known_host("work-vm"),
        "rotate-known-host:vm:work-vm"
    );
}

// ---------------------------------------------------------------
// Bundle resolver tests.
// ---------------------------------------------------------------

fn build_synthetic_resolver() -> BundleResolver {
    let bundle = Bundle {
        bundle_version: 4,
        schema_version: "v2".to_owned(),
        public_manifest_path: "/run/current-system/sw/share/nixling/vms.json".to_owned(),
        host_path: "/etc/nixling/host.json".to_owned(),
        processes_path: "/etc/nixling/processes.json".to_owned(),
        privileges_path: "/etc/nixling/privileges.json".to_owned(),
        closures: Vec::new(),
        minijail_profiles: Vec::new(),
        managed_keys: BundleManagedKeys::default(),
        generation: BundleGeneration {
            generator: "test-fixture".to_owned(),
            source_revision: None,
            generated_at: None,
        },
        bundle_hash: None,
        artifact_hashes: None,
    };
    let host = HostJson {
        schema_version: "v2".to_owned(),
        site: SitePolicy {
            allow_unsafe_east_west: false,
        },
        environments: vec![NetEnv {
            env: "work".to_owned(),
            bridge: IfName::new("br-work-lan").expect("ifname"),
            mtu: 1500,
            mss_clamp: None,
            lan: LanPolicy {
                allow_east_west: false,
                effective_east_west: false,
            },
            net_vm_forward_blocklist: vec!["192.168.1.0/24".to_owned()],
            bridge_port_flags: vec![BridgePortFlags {
                role: TapRole::WorkloadLan,
                isolated: true,
                neigh_suppress: true,
                learning: Some(true),
                unicast_flood: Some(false),
                rule: "workload isolation".to_owned(),
            }],
            ipv6_sysctls: vec![Ipv6SysctlEntry {
                if_name: IfName::new("br-work-lan").expect("ifname"),
                disable_ipv6: 1,
                accept_ra: 0,
                autoconf: 0,
                addr_gen_mode: 1,
                arp_ignore: 1,
            }],
            usbip_busid_locks: vec![UsbipBusidLock {
                vm: "work-vm".to_owned(),
                lock_owner: UsbipLockOwner::Daemon,
                scope: UsbipLockScope::PerBusid,
                bus_ids: Vec::new(),
                vendor_product_allowlist: Vec::new(),
            }],
        }],
        nftables: NftablesModel {
            family: "inet".to_owned(),
            table: "nixling".to_owned(),
            chains: vec![
                NftChain {
                    name: "input".to_owned(),
                    hook: Some("input".to_owned()),
                    priority: Some(0),
                    policy: Some("drop".to_owned()),
                    purpose: "host ingress firewall".to_owned(),
                },
                NftChain {
                    name: "forward".to_owned(),
                    hook: Some("forward".to_owned()),
                    priority: Some(-5),
                    policy: Some("drop".to_owned()),
                    purpose: "host forward firewall (per-env workload egress)".to_owned(),
                },
            ],
            table_hash_after_apply: None,
            ownership_id: "nl-fixture-001".to_owned(),
        },
        network_manager: NetworkManagerUnmanaged {
            file_path: "/etc/NetworkManager/conf.d/00-nixling.conf".to_owned(),
            match_criteria: vec!["interface-name:nl-*".to_owned()],
            reload_behavior: "atomic-reload".to_owned(),
            ownership: OwnershipRule {
                owner: "root".to_owned(),
                group: "root".to_owned(),
                mode: "0644".to_owned(),
                drift_policy: "refuse".to_owned(),
            },
        },
        hosts_file: HostsFileOwnership {
            start_marker: "# nixling managed begin".to_owned(),
            end_marker: "# nixling managed end".to_owned(),
            rule: "marker-block-only".to_owned(),
        },
        kernel_modules: Vec::new(),
        fd_ownership: Vec::new(),
        cloud_hypervisor_capabilities: Vec::new(),
        if_name_mappings: Vec::new(),
        ch: None,
        firewall_coexistence_policy: None,
    };
    let processes = ProcessesJson {
        schema_version: "v2".to_owned(),
        vms: vec![VmProcessDag {
            vm: "work-vm".to_owned(),
            nodes: vec![ProcessNode {
                id: NodeId("ch-runner".to_owned()),
                role: ProcessRole::CloudHypervisorRunner,
                unit: None,
                binary_path: Some("/nix/store/cloud-hypervisor/bin/cloud-hypervisor".to_owned()),
                argv: vec![
                    "microvm@work-vm".to_owned(),
                    "--api-socket".to_owned(),
                    "/var/lib/nixling/vms/work-vm/work-vm.sock".to_owned(),
                ],
                env: Vec::new(),
                profile: {
                    #[cfg(feature = "test-support")]
                    {
                        nixling_core::test_support::RoleProfileBuilder::new()
                            .with_profile_id("ch-runner-default")
                            .with_uid(5001)
                            .with_gid(5001)
                            .with_namespaces(nixling_core::minijail_profile::NamespaceSet {
                                mount: true,
                                pid: true,
                                net: false,
                                ipc: true,
                                uts: true,
                                user: false,
                            })
                            .with_cgroup_placement(CgroupPlacement {
                                subtree: "nixling/work-vm/ch-runner".to_owned(),
                                controllers: Vec::new(),
                                delegated: true,
                            })
                            .build()
                    }
                    #[cfg(not(feature = "test-support"))]
                    {
                        RoleProfile {
                            profile_id: "ch-runner-default".to_owned(),
                            uid: 5001,
                            gid: 5001,
                            adr_carve_out: None,
                            caps: Vec::new(),
                            namespaces: NamespaceSet {
                                mount: true,
                                pid: true,
                                net: false,
                                ipc: true,
                                uts: true,
                                user: false,
                            },
                            seccomp_policy_ref: None,
                            mount_policy: MountPolicy {
                                read_only_paths: Vec::new(),
                                writable_paths: Vec::new(),
                                nix_store_read_only: true,
                                hide_device_nodes_by_default: true,
                                device_binds: Vec::new(),
                                bind_mounts: Vec::new(),
                            },
                            cgroup_placement: CgroupPlacement {
                                subtree: "nixling/work-vm/ch-runner".to_owned(),
                                controllers: Vec::new(),
                                delegated: true,
                            },
                            user_namespace: None,
                            umask: None,
                        }
                    }
                },
                readiness: vec![ReadinessPredicate::ApiSocketInfo(
                    "/run/nixling/vms/work-vm/api.sock".to_owned(),
                )],
                plan_ops: Vec::new(),
            }],
            edges: Vec::<DagEdge>::new(),
            invariants: VmProcessInvariants {
                swtpm_pre_start_flush: false,
                per_vm_audit_pipeline: true,
                usbip_gating: true,
                tpm_ownership_migration_without_running_vm_mutation: true,
            },
        }],
    };
    const SYNTHETIC_MANIFEST: &str = r#"{"_manifest":{"manifestVersion":3},"_observability":{"enabled":false,"vmName":"sys-obs-stack","obsVsockCid":1000,"obsVsockHostSocket":"/var/lib/nixling/vms/sys-obs-stack/vsock.sock","grafanaUrl":"http://10.40.0.10:3000","chExporter":{"listenPort":9101}},"work-vm":{"apiSocket":"/var/lib/nixling/vms/work-vm/work-vm.sock","audio":false,"audioService":"nixling-work-vm-snd.service","audioStateFile":"/var/lib/nixling/vms/work-vm/state/audio-state.json","bridge":"br-work-lan","env":"work","gpuSocket":"/var/lib/nixling/vms/work-vm/work-vm-gpu.sock","graphics":false,"isNetVm":false,"name":"work-vm","netVm":"sys-work-net","observability":{"agentSocket":"/run/nixling/otlp.sock","enabled":false,"vsockCid":110,"vsockHostSocket":"/var/lib/nixling/vms/work-vm/vsock.sock"},"sshUser":"alice","stateDir":"/var/lib/nixling/vms/work-vm","staticIp":"10.20.0.10","tap":"work-l10","tpm":false,"tpmSocket":"/run/swtpm/work-vm/sock","usbipYubikey":false,"usbipdHostIp":"192.0.2.1"},"sys-work-net":{"apiSocket":"/var/lib/nixling/vms/sys-work-net/sys-work-net.sock","audio":false,"audioService":"nixling-sys-work-net-snd.service","audioStateFile":"/var/lib/nixling/vms/sys-work-net/state/audio-state.json","bridge":"br-work-up","env":"work","gpuSocket":"/var/lib/nixling/vms/sys-work-net/sys-work-net-gpu.sock","graphics":false,"isNetVm":true,"name":"sys-work-net","netVm":null,"observability":{"agentSocket":"/run/nixling/otlp.sock","enabled":false,"vsockCid":110,"vsockHostSocket":"/var/lib/nixling/vms/sys-work-net/vsock.sock"},"sshUser":null,"stateDir":"/var/lib/nixling/vms/sys-work-net","staticIp":"192.0.2.2","tap":"work-u2","tpm":false,"tpmSocket":"/run/swtpm/sys-work-net/sock","usbipYubikey":false,"usbipdHostIp":null}}"#;
    let manifest = ManifestV04::from_slice(SYNTHETIC_MANIFEST.as_bytes()).expect("manifest parses");
    let closures = vec![ClosureMetadata {
        schema_version: "v2".to_owned(),
        vm: "work-vm".to_owned(),
        toplevel: "/nix/store/work-vm-system".to_owned(),
        closure_paths: vec![
            "/nix/store/work-vm-system".to_owned(),
            "/nix/store/work-vm-runner".to_owned(),
        ],
        db_dump_path: "/nix/store/work-vm-registration".to_owned(),
        declared_runner: "/nix/store/work-vm-runner".to_owned(),
        runner_parity_path: "/nix/store/work-vm-runner".to_owned(),
        runner_parity_ok: true,
        generation: ClosureGeneration {
            host_generation: Some(42),
            vm_generation: Some("42".to_owned()),
            source_revision: None,
            generated_at: None,
        },
    }];
    BundleResolver::from_artifacts_with_closures(
        bundle,
        "fnv1a64:synthetic-bundle".to_owned(),
        host,
        processes,
        manifest,
        closures,
    )
}

fn bundle_resolver_round_trips_nft_intents() {
    let r = build_synthetic_resolver();
    let host_intent = r.find_nft_intent(&intent_id_nft_host()).expect("host nft");
    assert_eq!(host_intent.scope_label, "host");
    assert!(host_intent.script_body.contains("table inet nixling"));
    assert!(host_intent.script_body.contains("chain input"));
    assert!(host_intent.desired_hash.starts_with("fnv1a64:"));
    assert_eq!(host_intent.ownership_id, "nl-fixture-001");

    // Per-env workload egress: the forward chain default-drops at
    // priority -5 (runs before nixos-filter-forward at 0), so without
    // an explicit accept-new for each env's `br-<env>-up` interface,
    // SYNs from workload VMs get dropped before they reach the nixos
    // chain. Regression guard for that gap.
    assert!(
        host_intent
            .script_body
            .contains("iifname \"br-work-up\" ct state new accept"),
        "missing per-env forward accept rule in host nft script:\n{}",
        host_intent.script_body
    );

    let env_intent = r
        .find_nft_intent(&intent_id_nft_env("work"))
        .expect("env nft");
    assert_eq!(env_intent.scope_label, "env:work");
    assert!(env_intent.script_body.contains("env nft subset for work"));
}

fn bundle_resolver_round_trips_route_intents() {
    let r = build_synthetic_resolver();
    let route = r
        .find_route_intent(&intent_id_route_env("work", 0))
        .expect("env route 0");
    assert_eq!(route.destination, "192.168.1.0/24");
    assert_eq!(route.device.as_deref(), Some("br-work-up"));
    assert!(route.owned);
}

fn bundle_resolver_round_trips_sysctl_intents() {
    let r = build_synthetic_resolver();
    let disable = r
        .find_sysctl_intent(&intent_id_sysctl("work", "br-work-lan", "disable_ipv6"))
        .expect("disable_ipv6 sysctl");
    assert_eq!(disable.key, "net.ipv6.conf.br-work-lan.disable_ipv6");
    assert_eq!(disable.value, "1");
    let accept = r
        .find_sysctl_intent(&intent_id_sysctl("work", "br-work-lan", "accept_ra"))
        .expect("accept_ra sysctl");
    assert_eq!(accept.value, "0");
}

fn bundle_resolver_round_trips_hosts_intent() {
    let r = build_synthetic_resolver();
    let intent = r.find_hosts_intent(&intent_id_hosts_host()).expect("hosts");
    assert_eq!(intent.path.to_str(), Some("/etc/hosts"));
    assert!(intent.managed_block.contains("nixling managed begin"));
    assert!(intent.managed_block.contains("nixling managed end"));
    assert!(intent.managed_block.contains("env work"));
}

fn bundle_resolver_round_trips_nm_unmanaged_intent() {
    let r = build_synthetic_resolver();
    let intent = r
        .find_nm_unmanaged_intent(&intent_id_nm_unmanaged_host())
        .expect("nm-unmanaged");
    assert_eq!(intent.mode, 0o644);
    assert!(intent.contents.contains("interface-name:nl-*"));
    assert_eq!(intent.owner, "root");
}

fn bundle_resolver_round_trips_usbip_intents() {
    let r = build_synthetic_resolver();
    let fw = r
        .find_usbip_firewall_intent(&intent_id_usbip_firewall("work", "pending"))
        .expect("usbip-fw");
    assert_eq!(fw.env, "work");
    assert_eq!(fw.bus_id, "pending");
    assert!(fw.nft_rule_body.contains("dport 3240 accept"));
    let bind = r
        .find_usbip_bind_intent(&intent_id_usbip_bind("work", "work-vm", "pending"))
        .expect("usbip-bind");
    assert_eq!(bind.vm_name, "work-vm");
    assert_eq!(
        bind.lock_path.to_str(),
        Some("/run/nixling/locks/usbip/pending")
    );
}

fn bundle_resolver_uses_real_bus_ids_when_present() {
    let r = build_resolver_with_usbip_bus_ids(&["1-1.4", "1-1.5"]);
    let fw = r
        .find_usbip_firewall_intent(&intent_id_usbip_firewall("work", "1-1.4"))
        .expect("usbip-fw real busid");
    assert_eq!(fw.bus_id, "1-1.4");
    let bind = r
        .find_usbip_bind_intent(&intent_id_usbip_bind("work", "work-vm", "1-1.5"))
        .expect("usbip-bind real busid");
    assert_eq!(bind.vm_name, "work-vm");
    assert_eq!(
        bind.lock_path.to_str(),
        Some("/run/nixling/locks/usbip/1-1.5")
    );
    assert!(r
        .find_usbip_bind_intent(&intent_id_usbip_bind("work", "work-vm", "pending"))
        .is_none());
}

fn bundle_resolver_round_trips_runner_intents() {
    let r = build_synthetic_resolver();
    let intent = r
        .find_runner_intent(&intent_id_runner("work-vm", "ch-runner"))
        .expect("runner");
    assert_eq!(intent.vm_name, "work-vm");
    assert_eq!(intent.role_id, "ch-runner");
    assert_eq!(intent.uid, 5001);
    assert_eq!(intent.gid, 5001);
    assert!(!intent.root_carve_out);
    assert_eq!(intent.profile_id, "ch-runner-default");
    assert!(intent
        .binary_path
        .to_str()
        .unwrap()
        .ends_with("cloud-hypervisor"));
    assert_eq!(
        intent.argv,
        vec![
            "microvm@work-vm".to_owned(),
            "--api-socket".to_owned(),
            "/var/lib/nixling/vms/work-vm/work-vm.sock".to_owned(),
        ]
    );
    assert!(intent.env.iter().any(|e| e == "NIXLING_VM=work-vm"));
}

fn bundle_resolver_round_trips_socket_intents() {
    let r = build_synthetic_resolver();
    let intent = r
        .find_socket_intent(&intent_id_socket("work-vm", "ch-runner"))
        .expect("socket");
    assert_eq!(
        intent.socket_path.to_str(),
        Some("/run/nixling/vms/work-vm/ch-runner.sock")
    );
    assert_eq!(intent.owner_uid, 5001);
    assert_eq!(intent.mode, 0o660);
}

fn bundle_resolver_round_trips_activation_intents() {
    let r = build_synthetic_resolver();
    let intent = r
        .find_activation_intent(&intent_id_activation("work-vm"))
        .expect("activation");
    assert_eq!(intent.vm, "work-vm");
    assert_eq!(intent.generation_number, Some(42));
    assert_eq!(
        intent.target_generation_path,
        std::path::PathBuf::from("/nix/store/work-vm-system")
    );
}

fn bundle_resolver_round_trips_gc_intent() {
    let r = build_synthetic_resolver();
    let intent = r.find_gc_intent(&intent_id_gc_host()).expect("gc");
    assert_eq!(intent.retained_store_paths.len(), 2);
    assert!(intent
        .retained_store_paths
        .iter()
        .any(|path| path == &std::path::PathBuf::from("/nix/store/work-vm-system")));
}

fn bundle_resolver_round_trips_key_management_intents() {
    let r = build_synthetic_resolver();
    let keys = r
        .find_keys_rotate_intent(&intent_id_keys_rotate("work-vm"))
        .expect("keys rotate");
    assert_eq!(keys.vm, "work-vm");
    assert!(keys.key_path.ends_with("work-vm_ed25519"));

    let trust = r
        .find_host_key_trust_intent(&intent_id_trust("work-vm"))
        .expect("trust");
    assert_eq!(trust.static_ip, "10.20.0.10");
    assert!(trust.known_hosts_path.ends_with("known_hosts.nixling"));
    assert!(trust
        .host_public_key_path
        .ends_with("sshd-host-keys/ssh_host_ed25519_key.pub"));

    let rotate = r
        .find_rotate_known_host_intent(&intent_id_rotate_known_host("work-vm"))
        .expect("rotate-known-host");
    assert_eq!(rotate.static_ip, "10.20.0.10");
    assert!(rotate.known_hosts_path.ends_with("known_hosts.nixling"));
}

fn bundle_resolver_unknown_intent_returns_none() {
    let r = build_synthetic_resolver();
    assert!(r.find_nft_intent("nft:env:does-not-exist").is_none());
    assert!(r.find_route_intent("route:env:work:9999").is_none());
    assert!(r
        .find_runner_intent("runner:vm:no-such-vm:role:x")
        .is_none());
    assert!(r
        .find_activation_intent("activation:vm:no-such-vm")
        .is_none());
    assert!(r.find_gc_intent("gc:missing").is_none());
    assert!(r
        .find_keys_rotate_intent("keys-rotate:vm:no-such-vm")
        .is_none());
}

fn bundle_resolver_intent_ids_are_sorted_and_deterministic() {
    let r1 = build_synthetic_resolver();
    let r2 = build_synthetic_resolver();
    let ids1: Vec<&str> = r1.nft_intent_ids().collect();
    let ids2: Vec<&str> = r2.nft_intent_ids().collect();
    assert_eq!(ids1, ids2);
    // BTreeMap iteration is sorted; just sanity check the order.
    let mut sorted = ids1.clone();
    sorted.sort();
    assert_eq!(ids1, sorted);
}

// ---------------------------------------------------------------
// Minijail profile validator + host-runtime tests.
// ---------------------------------------------------------------

fn bundle_resolver_minijail_profile_validator_passes_on_fixture() {
    let r = build_synthetic_resolver();
    let count = r
        .validate_minijail_profiles()
        .expect("fixture is well-formed");
    assert_eq!(count, 1, "fixture has exactly one VM with one node");
}

fn bundle_resolver_minijail_profile_validator_rejects_root_without_carve_out() {
    use nixling_core::bundle_resolver::MinijailProfileViolation;
    let r = build_resolver_with_root_profile();
    let err = r
        .validate_minijail_profiles()
        .expect_err("root without carve-out must be rejected");
    assert!(matches!(
        err,
        MinijailProfileViolation::RootWithoutCarveOut { uid: 0, gid: 0, .. }
    ));
}

fn bundle_resolver_minijail_profile_validator_rejects_writable_nix_store() {
    use nixling_core::bundle_resolver::MinijailProfileViolation;
    let r = build_resolver_with_writable_nix_store();
    let err = r
        .validate_minijail_profiles()
        .expect_err("writable /nix/store must be rejected");
    assert!(matches!(
        err,
        MinijailProfileViolation::NixStoreNotReadOnly { .. }
    ));
}

fn bundle_resolver_minijail_validator_rejects_empty_profile_id() {
    use nixling_core::bundle_resolver::MinijailProfileViolation;

    let mut r = build_synthetic_resolver();
    r.processes.vms[0].nodes[0].profile.profile_id = "".to_owned();

    let err = r
        .validate_minijail_profiles()
        .expect_err("empty profile_id must be rejected");
    assert_eq!(
        err,
        MinijailProfileViolation::EmptyProfileId {
            vm: "work-vm".to_owned(),
            node: "ch-runner".to_owned(),
        }
    );
}

fn bundle_resolver_minijail_validator_rejects_cgroup_outside_nixling() {
    use nixling_core::bundle_resolver::MinijailProfileViolation;

    let mut r = build_synthetic_resolver();
    r.processes.vms[0].nodes[0].profile.cgroup_placement.subtree = "system.slice/foo".to_owned();

    let err = r
        .validate_minijail_profiles()
        .expect_err("cgroup subtree outside nixling/ must be rejected");
    assert_eq!(
        err,
        MinijailProfileViolation::CgroupSubtreeOutsideNixling {
            profile_id: "ch-runner-default".to_owned(),
            subtree: "system.slice/foo".to_owned(),
        }
    );
}

fn bundle_resolver_minijail_validator_allows_empty_cgroup_subtree() {
    let mut r = build_synthetic_resolver();
    r.processes.vms[0].nodes[0].profile.cgroup_placement.subtree = "".to_owned();

    let count = r
        .validate_minijail_profiles()
        .expect("empty cgroup subtree is an intentional skip path");
    assert_eq!(count, 1, "fixture has exactly one VM with one node");
}

fn bundle_resolver_minijail_validator_allows_root_with_carve_out() {
    let mut r = build_synthetic_resolver();
    let profile = &mut r.processes.vms[0].nodes[0].profile;
    profile.uid = 0;
    profile.gid = 0;
    profile.adr_carve_out = Some("ADR-0003 swtpm pre-start flush".to_owned());
    profile.mount_policy.nix_store_read_only = false;

    let count = r
        .validate_minijail_profiles()
        .expect("root carve-out exempts root and nix-store invariants");
    assert_eq!(count, 1, "fixture has exactly one VM with one node");
}

fn bundle_resolver_host_runtime_synthesizes_from_ifname_mappings() {
    let r = build_resolver_with_ifname_mappings();
    let runtime = r.host_runtime();
    assert_eq!(runtime.schema_version, "v2");
    assert_eq!(runtime.bundle_version, 4);
    assert_eq!(runtime.ifnames.len(), 1);
    let row = &runtime.ifnames[0];
    assert_eq!(row.env, "work");
    assert_eq!(row.user_visible_name, "br-work-lan");
    assert_eq!(row.derived_ifname, "nl-br-a1b2c3d4");
    assert_eq!(row.role_tag, "wkl");
}

fn build_resolver_with_root_profile() -> nixling_core::bundle_resolver::BundleResolver {
    let mut bad_resolver = build_synthetic_resolver();
    bad_resolver.processes.vms[0].nodes[0].profile.uid = 0;
    bad_resolver.processes.vms[0].nodes[0].profile.gid = 0;
    bad_resolver.processes.vms[0].nodes[0].profile.adr_carve_out = None;
    bad_resolver
}

fn build_resolver_with_writable_nix_store() -> nixling_core::bundle_resolver::BundleResolver {
    let mut bad_resolver = build_synthetic_resolver();
    bad_resolver.processes.vms[0].nodes[0]
        .profile
        .mount_policy
        .nix_store_read_only = false;
    bad_resolver
}

fn build_resolver_with_ifname_mappings() -> nixling_core::bundle_resolver::BundleResolver {
    use nixling_core::host::IfNameMapping;
    let mut r = build_synthetic_resolver();
    r.host.if_name_mappings = vec![IfNameMapping {
        env: "work".to_owned(),
        vm: None,
        role: TapRole::WorkloadLan,
        user_visible_name: "br-work-lan".to_owned(),
        derived_ifname: IfName::new("nl-br-a1b2c3d4").expect("ifname"),
    }];
    r
}

fn build_resolver_with_usbip_bus_ids(
    bus_ids: &[&str],
) -> nixling_core::bundle_resolver::BundleResolver {
    let mut r = build_synthetic_resolver();
    r.host.environments[0].usbip_busid_locks[0].bus_ids =
        bus_ids.iter().map(|bus_id| (*bus_id).to_owned()).collect();
    BundleResolver::from_artifacts(r.bundle, r.host, r.processes, r.manifest)
}
