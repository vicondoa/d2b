use d2b_contract_tests::load_full_bundle_resolver_from_env;
use d2b_core::{
    bundle_resolver::BundleResolver,
    processes::{ProcessNode, ProcessRole, VmProcessDag},
};

fn resolver_or_skip(test: &str) -> Option<BundleResolver> {
    match load_full_bundle_resolver_from_env() {
        Some(resolver) => Some(resolver),
        None => {
            eprintln!("SKIP {test}: D2B_FIXTURES_FULL unset");
            None
        }
    }
}

fn nodes_with_role(
    resolver: &BundleResolver,
    role: ProcessRole,
) -> Vec<(&VmProcessDag, &ProcessNode)> {
    resolver
        .processes
        .vms
        .iter()
        .flat_map(|dag| {
            let role = role.clone();
            dag.nodes
                .iter()
                .filter(move |node| node.role == role)
                .map(move |node| (dag, node))
        })
        .collect()
}

fn identity(dag: &VmProcessDag) -> (&str, &str) {
    let identity = dag
        .workload_identity
        .as_ref()
        .expect("realm runner must carry workload identity");
    (identity.realm_id.as_str(), identity.workload_id.as_str())
}

fn has_arg_fragment(node: &ProcessNode, fragment: &str) -> bool {
    node.argv.iter().any(|arg| arg.contains(fragment))
}

#[test]
fn cloud_hypervisor_runner_shape_matches_realm_roles() {
    let test = "cloud_hypervisor_runner_shape_matches_realm_roles";
    let Some(resolver) = resolver_or_skip(test) else {
        return;
    };
    let nodes = nodes_with_role(&resolver, ProcessRole::CloudHypervisorRunner);
    assert!(
        !nodes.is_empty(),
        "{test}: fixture has no Cloud Hypervisor role"
    );
    for (dag, node) in nodes {
        let (realm_id, workload_id) = identity(dag);
        assert_eq!(node.argv.first(), Some(&format!("microvm@{workload_id}")));
        assert_eq!(
            node.profile.mount_policy.device_binds,
            vec!["/dev/kvm".to_owned(), "/dev/vhost-net".to_owned()]
        );
        assert!(has_arg_fragment(
            node,
            &format!("/var/lib/d2b/r/{realm_id}/w/{workload_id}/vsock.sock")
        ));
        assert!(has_arg_fragment(
            node,
            &format!("/run/d2b/r/{realm_id}/w/{workload_id}/roles/")
        ));
        for forbidden in ["/var/lib/d2b/vms/", "/run/d2b/vms/", "/run/d2b-video/"] {
            assert!(
                !node.argv.iter().any(|arg| arg.contains(forbidden)),
                "{test}: canonical argv leaked legacy path {forbidden:?}"
            );
        }
    }
}

#[test]
fn virtiofsd_runner_shape_preserves_adr0021_and_store_farm() {
    let test = "virtiofsd_runner_shape_preserves_adr0021_and_store_farm";
    let Some(resolver) = resolver_or_skip(test) else {
        return;
    };
    let nodes = nodes_with_role(&resolver, ProcessRole::Virtiofsd);
    assert!(!nodes.is_empty(), "{test}: fixture has no virtiofsd roles");
    for (dag, node) in nodes {
        let (realm_id, workload_id) = identity(dag);
        assert!(node.profile.caps.is_empty());
        assert_eq!(
            node.profile.seccomp_policy_ref.as_deref(),
            Some("w1-virtiofsd")
        );
        let user_ns = node
            .profile
            .user_namespace
            .as_ref()
            .expect("ADR 0021 requires broker-prepared virtiofsd user namespace");
        assert_eq!(user_ns.host_uid_for_zero, node.profile.uid);
        assert_eq!(user_ns.host_gid_for_zero, node.profile.gid);
        assert!(node.argv.iter().any(|arg| arg == "--sandbox=chroot"));
        assert!(
            node.argv
                .iter()
                .any(|arg| arg == "--inode-file-handles=never")
        );
        assert!(has_arg_fragment(
            node,
            &format!("/run/d2b/r/{realm_id}/w/{workload_id}/roles/")
        ));
        if node
            .argv
            .first()
            .is_some_and(|arg| arg.ends_with("-ro-store"))
        {
            assert!(has_arg_fragment(
                node,
                &format!("/var/lib/d2b/r/{realm_id}/w/{workload_id}/store-view/live")
            ));
            assert!(node.argv.iter().any(|arg| arg == "--readonly"));
        }
        assert!(!has_arg_fragment(node, "--shared-dir=/nix/store"));
    }
}

#[test]
fn swtpm_runner_shape_uses_persistent_realm_state() {
    let test = "swtpm_runner_shape_uses_persistent_realm_state";
    let Some(resolver) = resolver_or_skip(test) else {
        return;
    };
    let nodes = nodes_with_role(&resolver, ProcessRole::Swtpm);
    assert_eq!(nodes.len(), 1, "{test}: fixture must render one swtpm role");
    let (dag, node) = nodes[0];
    let (realm_id, workload_id) = identity(dag);
    assert_eq!(
        node.argv.first(),
        Some(&format!("microvm-swtpm@{workload_id}"))
    );
    assert!(has_arg_fragment(
        node,
        &format!("dir=/var/lib/d2b/r/{realm_id}/w/{workload_id}/tpm")
    ));
    assert!(has_arg_fragment(
        node,
        &format!(
            "/run/d2b/r/{realm_id}/w/{workload_id}/roles/{}/tpm.sock",
            node.id.0
        )
    ));
    assert!(!node.argv.iter().any(|arg| arg.contains("/swtpm")));
}

#[test]
fn graphics_video_and_usbip_shapes_are_mediated() {
    let test = "graphics_video_and_usbip_shapes_are_mediated";
    let Some(resolver) = resolver_or_skip(test) else {
        return;
    };
    let gpu = nodes_with_role(&resolver, ProcessRole::Gpu);
    let video = nodes_with_role(&resolver, ProcessRole::Video);
    let usbip = nodes_with_role(&resolver, ProcessRole::Usbip);
    assert!(!gpu.is_empty(), "{test}: fixture has no GPU role");
    assert_eq!(video.len(), 1, "{test}: fixture must have one video role");
    assert_eq!(usbip.len(), 1, "{test}: fixture must have one USBIP role");
    for (_, node) in gpu.into_iter().chain(video) {
        assert!(node.profile.mount_policy.device_binds.is_empty());
        assert!(
            node.argv.iter().any(|arg| arg == "/proc/self/fd/10")
                || node
                    .env
                    .iter()
                    .any(|entry| entry == "LIBVA_DRM_DEVICE=/proc/self/fd/10")
        );
        for forbidden in ["/dev/nvidia", "/dev/vfio", "/dev/udmabuf"] {
            assert!(!node.argv.iter().any(|arg| arg.contains(forbidden)));
        }
    }
    let (_, usbip) = usbip[0];
    assert!(usbip.binary_path.is_none() && usbip.argv.is_empty());
    assert!(usbip.profile.mount_policy.device_binds.is_empty());
}
