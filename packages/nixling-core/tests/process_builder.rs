#![forbid(unsafe_code)]

use nixling_core::minijail_profile::{CgroupPlacement, MountPolicy, NamespaceSet};
use nixling_core::process_builder::{
    ProcessNodeBuildError, ProcessNodeBuilder, disk_init_plan_op, readiness_api_socket_info,
    readiness_command, readiness_guest_control_health, readiness_tcp_port,
    readiness_unix_socket_exists,
};
use nixling_core::processes::{ProcessRole, ReadinessPredicate, RoleProfile, SpawnRunnerPlanOp};

fn profile() -> RoleProfile {
    RoleProfile {
        profile_id: "profile-test".to_owned(),
        uid: 60_100,
        gid: 60_100,
        adr_carve_out: None,
        caps: vec![],
        namespaces: NamespaceSet {
            mount: true,
            pid: false,
            net: false,
            ipc: false,
            uts: false,
            user: false,
        },
        seccomp_policy_ref: Some("test-policy".to_owned()),
        mount_policy: MountPolicy {
            read_only_paths: vec!["/nix/store".to_owned()],
            writable_paths: vec![],
            nix_store_read_only: true,
            hide_device_nodes_by_default: true,
            device_binds: vec![],
            bind_mounts: vec![],
        },
        cgroup_placement: CgroupPlacement {
            subtree: "nixling.slice/test-vm/cloud-hypervisor".to_owned(),
            controllers: vec!["cpu".to_owned(), "memory".to_owned()],
            delegated: false,
        },
        user_namespace: None,
        umask: Some(0o077),
    }
}

#[test]
fn process_builder_constructs_existing_process_node_shape() {
    let disk_op = disk_init_plan_op(
        "/var/lib/nixling/vms/test-vm/disk.img",
        1024 * 1024,
        0o600,
        60_100,
        60_100,
        true,
    );
    let node = ProcessNodeBuilder::new(ProcessRole::CloudHypervisorRunner, profile())
        .with_id("cloud-hypervisor")
        .with_unit("nixlingd.service")
        .with_binary_path("/run/current-system/sw/bin/cloud-hypervisor")
        .with_argv(["microvm@test-vm", "--api-socket", "/run/nixling/ch.sock"])
        .with_env(["NIXLING_TEST=1"])
        .with_plan_op(disk_op.clone())
        .with_readiness_predicate(readiness_api_socket_info("cloud-hypervisor"))
        .build()
        .expect("builder accepts complete runner node");

    assert_eq!(node.id.0, "cloud-hypervisor");
    assert_eq!(node.role, ProcessRole::CloudHypervisorRunner);
    assert_eq!(node.unit.as_deref(), Some("nixlingd.service"));
    assert_eq!(
        node.binary_path.as_deref(),
        Some("/run/current-system/sw/bin/cloud-hypervisor")
    );
    assert_eq!(node.argv[0], "microvm@test-vm");
    assert_eq!(node.env, ["NIXLING_TEST=1"]);
    assert_eq!(node.plan_ops, [disk_op]);
    assert_eq!(
        node.readiness,
        [ReadinessPredicate::ApiSocketInfo(
            "cloud-hypervisor".to_owned()
        )]
    );

    let json = serde_json::to_value(&node).expect("node serializes");
    assert!(json.get("id").is_some());
    assert!(json.get("role").is_some());
    assert!(json.get("binaryPath").is_some());
    assert!(json.get("planOps").is_some());
    assert!(json.get("readiness").is_some());
    assert!(json.get("builder").is_none());
}

#[test]
fn readiness_and_disk_init_helpers_emit_existing_variants() {
    assert_eq!(
        readiness_tcp_port("127.0.0.1", 22),
        ReadinessPredicate::TcpPort {
            host: "127.0.0.1".to_owned(),
            port: 22
        }
    );
    assert_eq!(
        readiness_unix_socket_exists("/run/nixling/test.sock"),
        ReadinessPredicate::UnixSocketExists("/run/nixling/test.sock".to_owned())
    );
    assert_eq!(
        readiness_command(["/run/current-system/sw/bin/true"]),
        ReadinessPredicate::Command(vec!["/run/current-system/sw/bin/true".to_owned()])
    );
    assert_eq!(
        readiness_guest_control_health("test-vm"),
        ReadinessPredicate::GuestControlHealth {
            vm: "test-vm".to_owned()
        }
    );

    assert_eq!(
        disk_init_plan_op(
            "/var/lib/nixling/vms/test-vm/root.img",
            4096,
            0o600,
            1,
            2,
            true
        ),
        SpawnRunnerPlanOp::DiskInit {
            target_path: "/var/lib/nixling/vms/test-vm/root.img".into(),
            size_bytes: 4096,
            mode: 0o600,
            owner_uid: 1,
            owner_gid: 2,
            if_absent: true,
        }
    );
}

#[test]
fn process_builder_validation_uses_stable_identifiers_only() {
    let err = ProcessNodeBuilder::new(ProcessRole::CloudHypervisorRunner, profile())
        .with_id("cloud-hypervisor")
        .with_binary_path("relative/secret-runner")
        .with_argv(["--token=secret"])
        .with_env(["SECRET_ENV=value"])
        .build()
        .expect_err("relative runner binary is rejected");
    assert_eq!(
        err,
        ProcessNodeBuildError::RelativeRunnerBinaryPath {
            node_id: "cloud-hypervisor".to_owned(),
            role: ProcessRole::CloudHypervisorRunner,
        }
    );
    let message = err.to_string();
    assert!(message.contains("cloud-hypervisor"));
    assert!(message.contains("CloudHypervisorRunner"));
    assert!(!message.contains("relative/secret-runner"));
    assert!(!message.contains("--token=secret"));
    assert!(!message.contains("SECRET_ENV"));
}

#[test]
fn pre_start_hooks_are_not_long_lived_spawnable_runners() {
    let node = ProcessNodeBuilder::new(ProcessRole::SwtpmPreStartFlush, profile())
        .with_id("swtpm-flush")
        .with_binary_path("relative/pre-start-helper")
        .with_argv(["nixling-swtpm-flush@test-vm"])
        .build()
        .expect("pre-start hooks are not validated as long-lived spawnable runners");
    assert_eq!(node.role, ProcessRole::SwtpmPreStartFlush);
    assert_eq!(
        node.binary_path.as_deref(),
        Some("relative/pre-start-helper")
    );
}

#[test]
fn process_builder_rejects_empty_argv_and_duplicate_readiness() {
    let err = ProcessNodeBuilder::new(ProcessRole::Swtpm, profile())
        .with_id("swtpm")
        .with_binary_path("/run/current-system/sw/bin/swtpm")
        .build()
        .expect_err("binary without argv is rejected");
    assert_eq!(
        err,
        ProcessNodeBuildError::RunnerBinaryWithoutArgv {
            node_id: "swtpm".to_owned(),
            role: ProcessRole::Swtpm,
        }
    );

    let err = ProcessNodeBuilder::new(ProcessRole::GuestControlHealth, profile())
        .with_id("guest-control-health")
        .with_readiness_predicate(readiness_guest_control_health("test-vm"))
        .with_readiness_predicate(readiness_guest_control_health("test-vm"))
        .build()
        .expect_err("duplicate readiness is rejected");
    assert_eq!(
        err,
        ProcessNodeBuildError::DuplicateReadiness {
            node_id: "guest-control-health".to_owned(),
            role: ProcessRole::GuestControlHealth,
            readiness_kind: "guest-control-health",
        }
    );
}
