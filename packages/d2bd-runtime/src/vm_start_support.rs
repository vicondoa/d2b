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
