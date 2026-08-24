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
        | ProcessRole::StoreVirtiofsPreflight
        | ProcessRole::GuestSshReadiness
        | ProcessRole::GuestControlHealth
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
/// QEMU-media keeps its legacy `wayland-proxy` node without an execution
/// reference, so it is intentionally not classified here.
pub fn is_durable_wayland_process_node(node: &ProcessNode) -> bool {
    node.role == ProcessRole::WaylandProxy
        && matches!(
            node.id.0.as_str(),
            "wayland-proxy" | "wayland-frontend-worker"
        )
        && node.execution_ref.is_some()
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
