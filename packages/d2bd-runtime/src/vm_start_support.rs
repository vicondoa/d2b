use d2b_contracts_broker::broker_wire::RunnerRole;
use d2b_core::bundle_resolver::{BundleResolver, ResolvedStoreViewIntent};
use d2b_core::processes::{ProcessNode, ProcessRole, SpawnRunnerPlanOp};

const VM_RUNNER_ROLE_ID: &str = "ch-runner";

#[derive(Debug, Clone, Copy)]
pub enum VmStartNodeMode {
    ReadinessOnly,
    OneShot(RunnerRole),
    LongLived(RunnerRole),
}

pub fn vm_start_node_mode(role: &ProcessRole) -> VmStartNodeMode {
    match role {
        ProcessRole::SwtpmPreStartFlush => VmStartNodeMode::OneShot(RunnerRole::SwtpmFlush),
        ProcessRole::Swtpm => VmStartNodeMode::LongLived(RunnerRole::Swtpm),
        ProcessRole::Virtiofsd => VmStartNodeMode::LongLived(RunnerRole::Virtiofsd),
        ProcessRole::CloudHypervisorRunner => {
            VmStartNodeMode::LongLived(RunnerRole::CloudHypervisor)
        }
        ProcessRole::QemuMediaRunner => VmStartNodeMode::LongLived(RunnerRole::QemuMedia),
        // Activation runners are target-local EphemeralProcess resources.
        // Their bundle row is used for resource-ticket resolution, but they
        // are never started as part of VM boot.
        ProcessRole::ActivationNixosRunner => VmStartNodeMode::ReadinessOnly,
        ProcessRole::Gpu | ProcessRole::GpuRenderNode => {
            VmStartNodeMode::LongLived(RunnerRole::Gpu)
        }
        ProcessRole::Audio => VmStartNodeMode::LongLived(RunnerRole::Audio),
        ProcessRole::Video => VmStartNodeMode::LongLived(RunnerRole::Video),
        ProcessRole::VsockRelay => VmStartNodeMode::LongLived(RunnerRole::VsockRelay),
        ProcessRole::OtelHostBridge => VmStartNodeMode::LongLived(RunnerRole::OtelHostBridge),
        ProcessRole::Usbip => VmStartNodeMode::LongLived(RunnerRole::Usbip),
        ProcessRole::WaylandProxy => VmStartNodeMode::LongLived(RunnerRole::WaylandProxy),
        ProcessRole::HostReconcile
        | ProcessRole::ProviderController
        | ProcessRole::StoreVirtiofsPreflight
        | ProcessRole::ComponentSessionHealth
        | ProcessRole::SecurityKeyFrontend => VmStartNodeMode::ReadinessOnly,
    }
}

pub fn tracked_role_id(node: &ProcessNode) -> String {
    match node.role {
        ProcessRole::CloudHypervisorRunner => VM_RUNNER_ROLE_ID.to_owned(),
        _ => node.id.0.clone(),
    }
}

/// Return whether this trusted node is owned by a Guest resource controller
/// rather than the legacy host VM-start DAG.
pub fn is_guest_owned_process_node(node: &ProcessNode) -> bool {
    node.execution_ref
        .as_deref()
        .is_some_and(|execution_ref| execution_ref.starts_with("Guest/"))
}

/// Return whether this node is retained as signed template metadata for a
/// durable WaylandSession Process rather than launched by the legacy VM DAG.
///
/// The legacy QEMU-media proxy has the same Host execution reference as the
/// trusted graphics proxy, so use its existing `--provider-kind` process
/// identity to keep it in the VM-start DAG.
pub fn is_durable_wayland_process_node(node: &ProcessNode) -> bool {
    if node.role != ProcessRole::WaylandProxy || node.execution_ref.is_none() {
        return false;
    }

    match node.id.0.as_str() {
        "wayland-frontend-worker" => true,
        "wayland-proxy" => {
            node.execution_ref.as_deref() == Some("Host/host-system")
                && node
                    .argv
                    .windows(2)
                    .any(|pair| pair[0] == "--provider-kind" && pair[1] == "local-vm")
        }
        _ => false,
    }
}

pub fn node_requires_disk_init_dispatch(node: &ProcessNode) -> bool {
    node.plan_ops
        .iter()
        .any(|op| matches!(op, SpawnRunnerPlanOp::DiskInit { .. }))
}

pub fn resolve_store_view_intent_for_vm<'a>(
    resolver: &'a BundleResolver,
    vm: &str,
) -> Result<&'a ResolvedStoreViewIntent, String> {
    resolver
        .find_store_view_intent(vm)
        .ok_or_else(|| "bundle-intent-missing:store-view".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        VmStartNodeMode, is_durable_wayland_process_node, is_guest_owned_process_node,
        vm_start_node_mode,
    };
    use d2b_contracts_broker::broker_wire::RunnerRole;
    use d2b_core::processes::{NodeId, ProcessExecutionDomain, ProcessNode, ProcessRole};

    fn wayland_node(
        id: &str,
        execution_ref: &str,
        execution_domain: Option<ProcessExecutionDomain>,
        argv: &[&str],
    ) -> ProcessNode {
        ProcessNode {
            id: NodeId(id.to_owned()),
            execution_ref: Some(execution_ref.to_owned()),
            execution_domain,
            user_ref: None,
            role: ProcessRole::WaylandProxy,
            unit: None,
            binary_path: Some("/nix/store/d2b-wayland-proxy/bin/d2b-wayland-proxy".to_owned()),
            argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
            env: vec![],
            plan_ops: vec![],
            network_interfaces: vec![],
            profile: d2b_core::test_support::RoleProfileBuilder::new().build(),
            readiness: vec![],
        }
    }

    #[test]
    fn trusted_graphics_proxy_is_resource_backed() {
        let node = wayland_node(
            "wayland-proxy",
            "Host/host-system",
            Some(ProcessExecutionDomain::System),
            &["d2b-vm-wlproxy", "--provider-kind", "local-vm"],
        );

        assert!(is_durable_wayland_process_node(&node));
        assert!(!is_guest_owned_process_node(&node));
    }

    #[test]
    fn qemu_media_proxy_stays_in_legacy_vm_start_dag() {
        let node = wayland_node(
            "wayland-proxy",
            "Host/host-system",
            None,
            &["d2b-vm-wlproxy", "--provider-kind", "qemu-media"],
        );

        assert!(!is_durable_wayland_process_node(&node));
        assert!(!is_guest_owned_process_node(&node));
        assert!(matches!(
            vm_start_node_mode(&node.role),
            VmStartNodeMode::LongLived(RunnerRole::WaylandProxy)
        ));
    }

    #[test]
    fn trusted_guest_frontend_remains_resource_backed() {
        let node = wayland_node(
            "wayland-frontend-worker",
            "Guest/demo-cd",
            Some(ProcessExecutionDomain::System),
            &["d2b-demo-cd-wayland-frontend", "--socket-name", "wayland-1"],
        );

        assert!(is_durable_wayland_process_node(&node));
        assert!(is_guest_owned_process_node(&node));
    }

    #[test]
    fn non_graphics_process_node_is_not_resource_backed_display() {
        let mut node = wayland_node(
            "wayland-proxy",
            "Host/host-system",
            Some(ProcessExecutionDomain::System),
            &["d2b-vm-wlproxy", "--provider-kind", "local-vm"],
        );
        node.role = ProcessRole::Gpu;

        assert!(!is_durable_wayland_process_node(&node));
    }
}
