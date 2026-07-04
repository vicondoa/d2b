use std::{fs, io, path::PathBuf};

use d2b_contracts::{
    cli_output::*,
    public_wire::{
        ListEntry as IpcListEntry, PublicVmServices,
        QemuMediaRegistryStatus as IpcQemuMediaRegistryStatus,
        QemuMediaRunnerStatus as IpcQemuMediaRunnerStatus,
        QemuMediaSourceStatus as IpcQemuMediaSourceStatus, QemuMediaStatus as IpcQemuMediaStatus,
        VmAutostartPosture as IpcVmAutostartPosture, VmLifecycleState as IpcVmLifecycleState,
        VmStatus as IpcVmStatus,
    },
};
use serde::Deserialize;
use serde_json::Value;

use super::{
    BundleContext, Context, ManifestDocument, ManifestVm, RUNTIME_UNKNOWN,
    read_live_pool_integrity, read_symlink_target, read_vm_api_ready, systemctl_state,
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct QemuMediaRegistryRecord {
    vm: String,
    media_ref: String,
    source_kind: String,
    format: String,
    read_only: bool,
    #[serde(default, rename = "schemaVersion")]
    _schema_version: Option<u32>,
    #[serde(default, rename = "identity")]
    _identity: Option<Value>,
}

pub(crate) fn list_output_from_manifest(
    context: &Context,
    manifest: &ManifestDocument,
    bundle: Option<&BundleContext>,
) -> ListOutputV2 {
    ListOutputV2(
        manifest
            .vms()
            .into_iter()
            .map(|vm| {
                let current = current_symlink(context, vm);
                let booted = booted_symlink(context, vm);
                let process_vm = bundle
                    .and_then(|bundle| bundle.processes.as_ref())
                    .and_then(|processes| processes.vms.iter().find(|entry| entry.vm == vm.name));
                let services = vm_service_states(context, vm, process_vm);
                let pending_restart =
                    is_pending_restart(vm, &services, current.as_deref(), booted.as_deref());
                let qemu_media = qemu_media_status(context, vm, bundle, process_vm, &services);
                let declared_guest_closure_out_path = bundle
                    .and_then(|bundle| bundle.closures.get(&vm.name))
                    .map(|closure| closure.toplevel.clone());
                ListItemOutputV2 {
                    name: vm.name.clone(),
                    env: vm.env.clone(),
                    graphics: vm.graphics,
                    tpm: vm.tpm,
                    usbip: vm.usbip_yubikey,
                    static_ip: vm.static_ip.clone(),
                    status: list_status_label(vm, &services, pending_restart),
                    is_net_vm: vm.is_net_vm,
                    guest_closure_out_path: if pending_restart {
                        booted
                            .as_ref()
                            .filter(|path| path.starts_with('/'))
                            .cloned()
                            .or(declared_guest_closure_out_path)
                    } else {
                        declared_guest_closure_out_path
                    },
                    runtime_kind: output_runtime_kind(vm),
                    autostart: output_autostart_posture(vm),
                    runtime_capabilities: output_runtime_capabilities(vm),
                    service_capabilities: output_service_capabilities(&services),
                    unsupported_capabilities: output_unsupported_capabilities(vm),
                    qemu_media,
                    runner_parity_ok: bundle
                        .and_then(|bundle| bundle.closures.get(&vm.name))
                        .map(|closure| closure.runner_parity_ok),
                }
            })
            .collect(),
    )
}

pub(crate) fn list_output_from_public_entries(
    entries: &[IpcListEntry],
    bundle: Option<&BundleContext>,
) -> ListOutputV2 {
    ListOutputV2(
        entries
            .iter()
            .map(|entry| ListItemOutputV2 {
                name: entry.name.clone(),
                env: entry.env.clone(),
                graphics: entry.graphics,
                tpm: entry.tpm,
                usbip: entry.usbip,
                static_ip: entry.static_ip.clone(),
                status: public_lifecycle_list_status_label(&entry.lifecycle),
                is_net_vm: entry.is_net_vm,
                guest_closure_out_path: entry.guest_closure_out_path.clone().or_else(|| {
                    bundle
                        .and_then(|bundle| bundle.closures.get(&entry.vm))
                        .map(|closure| closure.toplevel.clone())
                }),
                runtime_kind: entry.runtime.kind.clone(),
                autostart: entry.autostart.clone(),
                runtime_capabilities: entry.runtime_capabilities.clone(),
                service_capabilities: entry.service_capabilities.clone(),
                unsupported_capabilities: entry.unsupported_capabilities.clone(),
                qemu_media: entry.qemu_media.clone(),
                runner_parity_ok: bundle
                    .and_then(|bundle| bundle.closures.get(&entry.vm))
                    .map(|closure| closure.runner_parity_ok),
            })
            .collect(),
    )
}

fn manifest_runtime_kind(vm: &ManifestVm) -> &str {
    vm.runtime
        .as_ref()
        .map(|runtime| runtime.kind.as_str())
        .unwrap_or("nixos")
}

pub(crate) fn is_qemu_media_vm(vm: &ManifestVm) -> bool {
    manifest_runtime_kind(vm) == "qemu-media"
}

fn output_runtime_kind(vm: &ManifestVm) -> Option<String> {
    is_qemu_media_vm(vm).then(|| manifest_runtime_kind(vm).to_owned())
}

fn output_autostart_posture(vm: &ManifestVm) -> Option<IpcVmAutostartPosture> {
    is_qemu_media_vm(vm).then(|| IpcVmAutostartPosture {
        mode: "manual-only".to_owned(),
        reason: "qemu-media VMs are intentionally skipped by daemon autostart; start them explicitly with `d2b vm start <vm> --apply`".to_owned(),
    })
}

const QEMU_MEDIA_DEFAULT_RUNTIME_CAPABILITIES: &[&str] = &["display", "lifecycle", "usb-hotplug"];
const QEMU_MEDIA_DEFAULT_UNSUPPORTED_CAPABILITIES: &[&str] = &[
    "config-sync",
    "exec",
    "guest-control",
    "in-guest-observability",
    "keys",
    concat!("s", "sh"),
    "store-sync",
];

fn output_runtime_capabilities(vm: &ManifestVm) -> Vec<String> {
    output_runtime_capabilities_by_support(vm, true, QEMU_MEDIA_DEFAULT_RUNTIME_CAPABILITIES)
}

fn output_unsupported_capabilities(vm: &ManifestVm) -> Vec<String> {
    output_runtime_capabilities_by_support(vm, false, QEMU_MEDIA_DEFAULT_UNSUPPORTED_CAPABILITIES)
}

fn output_runtime_capabilities_by_support(
    vm: &ManifestVm,
    supported: bool,
    qemu_media_default: &[&str],
) -> Vec<String> {
    let Some(runtime) = vm.runtime.as_ref() else {
        return Vec::new();
    };
    let mut capabilities = runtime
        .capabilities
        .iter()
        .filter(|(_capability, is_supported)| **is_supported == supported)
        .map(|(capability, _is_supported)| capability_name_for_output(capability).to_owned())
        .collect::<Vec<_>>();
    if capabilities.is_empty() && runtime.kind == "qemu-media" && runtime.capabilities.is_empty() {
        capabilities = qemu_media_default
            .iter()
            .map(|value| (*value).to_owned())
            .collect();
    }
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

pub(crate) fn output_service_capabilities(services: &StatusServicesOutputV2) -> Vec<String> {
    let mut capabilities = vec!["d2b".to_owned()];
    if services.microvm != "unsupported" {
        capabilities.push("microvm".to_owned());
    }
    if services.qemu_media.is_some() {
        capabilities.push("qemu-media".to_owned());
    }
    if services.virtiofsd != "unsupported" {
        capabilities.push("virtiofsd".to_owned());
    }
    if services.gpu.is_some() {
        capabilities.push("gpu".to_owned());
    }
    if services.video.is_some() {
        capabilities.push("video".to_owned());
    }
    if services.snd.is_some() {
        capabilities.push("audio".to_owned());
    }
    if services.swtpm.is_some() {
        capabilities.push("swtpm".to_owned());
    }
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

fn capability_name_for_output(capability: &str) -> &str {
    match capability {
        "configSync" => "config-sync",
        "guestControl" => "guest-control",
        "inGuestObservability" => "in-guest-observability",
        "storeSync" => "store-sync",
        "usbHotplug" => "usb-hotplug",
        other => other,
    }
}

fn qemu_media_status(
    context: &Context,
    vm: &ManifestVm,
    bundle: Option<&BundleContext>,
    process_vm: Option<&d2b_core::processes::VmProcessDag>,
    services: &StatusServicesOutputV2,
) -> Option<IpcQemuMediaStatus> {
    if !is_qemu_media_vm(vm) {
        return None;
    }
    let runner_state = services
        .qemu_media
        .clone()
        .unwrap_or_else(|| services.microvm.clone());
    let qmp_socket = qemu_media_qmp_socket(process_vm)
        .or_else(|| Some(format!("/run/d2b/vms/{}/qmp.sock", vm.name)));
    let qmp_readiness = qmp_socket.as_deref().map(|path| {
        if unix_socket_listening_for_status(path) {
            "ready".to_owned()
        } else if service_state_counts_as_running(&runner_state) {
            "pending".to_owned()
        } else {
            "not-started".to_owned()
        }
    });
    let pre_cont_progress = match qmp_readiness.as_deref() {
        Some("ready") if service_state_counts_as_running(&runner_state) => "paused-before-cont",
        Some("pending") if service_state_counts_as_running(&runner_state) => "waiting-for-qmp",
        _ => "not-started",
    }
    .to_owned();
    let media = qemu_media_sources_for_vm(bundle, &vm.name)
        .into_iter()
        .map(|source| qemu_media_source_status(context, bundle, source))
        .collect();

    Some(IpcQemuMediaStatus {
        firmware_mode: "none".to_owned(),
        runner: IpcQemuMediaRunnerStatus {
            role: "qemu-media".to_owned(),
            state: runner_state,
            qmp_readiness,
            pre_cont_progress,
        },
        media,
    })
}

fn qemu_media_qmp_socket(process_vm: Option<&d2b_core::processes::VmProcessDag>) -> Option<String> {
    process_vm?
        .nodes
        .iter()
        .find(|node| node.role == d2b_core::processes::ProcessRole::QemuMediaRunner)?
        .readiness
        .iter()
        .find_map(|readiness| match readiness {
            d2b_core::processes::ReadinessPredicate::UnixSocketListening(path)
            | d2b_core::processes::ReadinessPredicate::UnixSocketExists(path) => Some(path.clone()),
            _ => None,
        })
}

fn qemu_media_sources_for_vm<'a>(
    bundle: Option<&'a BundleContext>,
    vm: &str,
) -> Vec<&'a d2b_core::host::QemuMediaSourceIntent> {
    bundle
        .and_then(|bundle| bundle.host.as_ref())
        .and_then(|host| host.qemu_media.as_ref())
        .map(|media| {
            media
                .sources
                .iter()
                .filter(|source| source.vm == vm)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn qemu_media_source_status(
    context: &Context,
    bundle: Option<&BundleContext>,
    source: &d2b_core::host::QemuMediaSourceIntent,
) -> IpcQemuMediaSourceStatus {
    IpcQemuMediaSourceStatus {
        media_ref: source.media_ref.clone(),
        slot: source.slot.clone(),
        source_kind: qemu_media_source_kind_name(source.source_kind).to_owned(),
        format: qemu_media_format_name(source.format).to_owned(),
        read_only: source.read_only,
        registry: qemu_media_registry_status(context, bundle, source),
    }
}

fn qemu_media_source_kind_name(kind: d2b_core::host::QemuMediaSourceKind) -> &'static str {
    match kind {
        d2b_core::host::QemuMediaSourceKind::PhysicalUsb => "physical-usb",
        d2b_core::host::QemuMediaSourceKind::ImageFile => "image-file",
    }
}

fn qemu_media_format_name(format: d2b_core::host::QemuMediaFormat) -> &'static str {
    match format {
        d2b_core::host::QemuMediaFormat::Raw => "raw",
        d2b_core::host::QemuMediaFormat::Qcow2 => "qcow2",
        d2b_core::host::QemuMediaFormat::Iso => "iso",
    }
}

fn qemu_media_registry_status(
    _context: &Context,
    bundle: Option<&BundleContext>,
    source: &d2b_core::host::QemuMediaSourceIntent,
) -> IpcQemuMediaRegistryStatus {
    if source.source_kind != d2b_core::host::QemuMediaSourceKind::PhysicalUsb {
        return IpcQemuMediaRegistryStatus {
            state: "direct-config".to_owned(),
            remediation: None,
        };
    }
    let Some(registry_dir) = bundle
        .and_then(|bundle| bundle.host.as_ref())
        .and_then(|host| host.qemu_media.as_ref())
        .map(|media| PathBuf::from(&media.registry_dir))
    else {
        return IpcQemuMediaRegistryStatus {
            state: "unavailable".to_owned(),
            remediation: Some(
                "load the private bundle host.json so qemu-media registry entries can be checked"
                    .to_owned(),
            ),
        };
    };
    let path = registry_dir
        .join(&source.vm)
        .join(format!("{}.json", source.media_ref));
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            return IpcQemuMediaRegistryStatus {
                state: "missing".to_owned(),
                remediation: Some(format!(
                    "declare the boot-drive physical USB source for vm `{}` in config, then run `d2b usb probe` to verify the runtime selector for `{}`",
                    source.vm, source.media_ref
                )),
            };
        }
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
            return IpcQemuMediaRegistryStatus {
                state: "unreadable".to_owned(),
                remediation: Some(
                    "the qemu-media registry is root-only; use daemon status or re-run as an authorized operator after updating config and probing USB media".to_owned(),
                ),
            };
        }
        Err(_) => {
            return IpcQemuMediaRegistryStatus {
                state: "unknown".to_owned(),
                remediation: Some(
                    "inspect the qemu-media registry and broker audit log, then retry status"
                        .to_owned(),
                ),
            };
        }
    };
    let Ok(record) = serde_json::from_slice::<QemuMediaRegistryRecord>(&bytes) else {
        return IpcQemuMediaRegistryStatus {
            state: "stale".to_owned(),
            remediation: Some(format!(
                "remove the malformed qemu-media registry entry for `{}` on vm `{}`, update config if needed, then run `d2b usb probe`",
                source.media_ref, source.vm
            )),
        };
    };
    let expected_kind = qemu_media_source_kind_name(source.source_kind);
    let expected_format = qemu_media_format_name(source.format);
    if record.vm != source.vm
        || record.media_ref != source.media_ref
        || record.source_kind != expected_kind
        || record.format != expected_format
        || record.read_only != source.read_only
    {
        return IpcQemuMediaRegistryStatus {
            state: "stale".to_owned(),
            remediation: Some(format!(
                "update the qemu-media config for `{}` on vm `{}` so the root-only registry matches the active bundle policy, then run `d2b usb probe`",
                source.media_ref, source.vm
            )),
        };
    }
    IpcQemuMediaRegistryStatus {
        state: "present".to_owned(),
        remediation: None,
    }
}

fn unix_socket_listening_for_status(path: &str) -> bool {
    const SO_ACCEPTCON: u64 = 0x0001_0000;
    let Ok(contents) = fs::read_to_string("/proc/net/unix") else {
        return false;
    };
    contents.lines().skip(1).any(|line| {
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 8 {
            return false;
        }
        let flags = u64::from_str_radix(fields[3], 16).unwrap_or(0);
        let socket_type = fields[4];
        let socket_path = fields[7];
        socket_path == path && socket_type == "0001" && (flags & SO_ACCEPTCON) != 0
    })
}

pub(crate) fn build_vm_status_output(
    context: &Context,
    vm: &ManifestVm,
    bundle: Option<&BundleContext>,
) -> StatusVmOutputV2 {
    let process_vm = bundle
        .and_then(|bundle| bundle.processes.as_ref())
        .and_then(|processes| processes.vms.iter().find(|entry| entry.vm == vm.name));
    let service_states = vm_service_states(context, vm, process_vm);
    let current = current_symlink(context, vm);
    let booted = booted_symlink(context, vm);
    let pending_restart =
        is_pending_restart(vm, &service_states, current.as_deref(), booted.as_deref());
    let qemu_media = qemu_media_status(context, vm, bundle, process_vm, &service_states);
    let service_capabilities = output_service_capabilities(&service_states);
    let declared_roles = process_vm
        .map(|entry| {
            entry
                .nodes
                .iter()
                .map(|node| process_role_name(&node.role))
                .collect()
        })
        .unwrap_or_default();
    let readiness: Vec<String> = process_vm
        .map(|entry| {
            entry
                .nodes
                .iter()
                .flat_map(|node| {
                    node.readiness
                        .iter()
                        .map(move |readiness| readiness_name_for_node(node, readiness))
                })
                .collect()
        })
        .unwrap_or_default();
    let runner_parity = bundle
        .and_then(|bundle| bundle.closures.get(&vm.name))
        .map(|closure| RunnerParityOutputV2 {
            declared_runner: closure.declared_runner.clone(),
            runner_parity_path: closure.runner_parity_path.clone(),
            runner_parity_ok: closure.runner_parity_ok,
        });

    StatusVmOutputV2 {
        name: vm.name.clone(),
        env: vm.env.clone(),
        services: service_states,
        current,
        booted,
        pending_restart,
        runtime: RUNTIME_UNKNOWN.to_owned(),
        runtime_kind: output_runtime_kind(vm),
        autostart: output_autostart_posture(vm),
        runtime_capabilities: output_runtime_capabilities(vm),
        service_capabilities,
        unsupported_capabilities: output_unsupported_capabilities(vm),
        qemu_media,
        usb: None,
        declared_roles,
        readiness,
        api_ready: read_vm_api_ready(&context.daemon_state_dir, &vm.name),
        runner_parity,
        live_pool_integrity: read_live_pool_integrity(context, vm),
    }
}

pub(crate) fn build_vm_status_output_from_public(
    context: &Context,
    vm: &ManifestVm,
    bundle: Option<&BundleContext>,
    public: &IpcVmStatus,
) -> StatusVmOutputV2 {
    let process_vm = bundle
        .and_then(|bundle| bundle.processes.as_ref())
        .and_then(|processes| processes.vms.iter().find(|entry| entry.vm == vm.name));
    let declared_roles = process_vm
        .map(|entry| {
            entry
                .nodes
                .iter()
                .map(|node| process_role_name(&node.role))
                .collect()
        })
        .unwrap_or_default();
    let readiness: Vec<String> = process_vm
        .map(|entry| {
            entry
                .nodes
                .iter()
                .flat_map(|node| {
                    node.readiness
                        .iter()
                        .map(move |readiness| readiness_name_for_node(node, readiness))
                })
                .collect()
        })
        .unwrap_or_default();
    let runner_parity = bundle
        .and_then(|bundle| bundle.closures.get(&vm.name))
        .map(|closure| RunnerParityOutputV2 {
            declared_runner: closure.declared_runner.clone(),
            runner_parity_path: closure.runner_parity_path.clone(),
            runner_parity_ok: closure.runner_parity_ok,
        });

    let services = status_services_from_public(&public.services);
    StatusVmOutputV2 {
        name: vm.name.clone(),
        env: public.env.clone().or_else(|| vm.env.clone()),
        services: services.clone(),
        current: current_symlink(context, vm),
        booted: booted_symlink(context, vm),
        pending_restart: public.lifecycle.pending_restart,
        runtime: public.runtime.detail.clone(),
        runtime_kind: public
            .runtime
            .kind
            .clone()
            .or_else(|| output_runtime_kind(vm)),
        autostart: public
            .autostart
            .clone()
            .or_else(|| output_autostart_posture(vm)),
        runtime_capabilities: if public.runtime_capabilities.is_empty() {
            output_runtime_capabilities(vm)
        } else {
            public.runtime_capabilities.clone()
        },
        service_capabilities: if public.service_capabilities.is_empty() {
            output_service_capabilities(&services)
        } else {
            public.service_capabilities.clone()
        },
        unsupported_capabilities: if public.unsupported_capabilities.is_empty() {
            output_unsupported_capabilities(vm)
        } else {
            public.unsupported_capabilities.clone()
        },
        qemu_media: public
            .qemu_media
            .clone()
            .or_else(|| qemu_media_status(context, vm, bundle, process_vm, &services)),
        usb: public.usb.clone(),
        declared_roles,
        readiness,
        api_ready: read_vm_api_ready(&context.daemon_state_dir, &vm.name),
        runner_parity,
        live_pool_integrity: read_live_pool_integrity(context, vm),
    }
}

fn status_services_from_public(services: &PublicVmServices) -> StatusServicesOutputV2 {
    StatusServicesOutputV2 {
        d2b: services.d2b.clone(),
        microvm: services.microvm.clone(),
        virtiofsd: services.virtiofsd.clone(),
        qemu_media: services.qemu_media.clone(),
        gpu: services.gpu.clone(),
        video: services.video.clone(),
        snd: services.snd.clone(),
        swtpm: services.swtpm.clone(),
    }
}

pub(crate) fn vm_service_states(
    context: &Context,
    vm: &ManifestVm,
    process_vm: Option<&d2b_core::processes::VmProcessDag>,
) -> StatusServicesOutputV2 {
    let has_role = |role: d2b_core::processes::ProcessRole| {
        process_vm
            .map(|entry| entry.nodes.iter().any(|node| node.role == role))
            .unwrap_or(false)
    };
    let gpu_role_id = if has_role(d2b_core::processes::ProcessRole::GpuRenderNode) {
        Some("gpu-render-node")
    } else if has_role(d2b_core::processes::ProcessRole::Gpu) || vm.graphics {
        Some("gpu")
    } else {
        None
    };
    let runner_role_id = vm_runner_role_id(process_vm, vm);
    let qemu_media_state =
        is_qemu_media_vm(vm).then(|| pidfd_role_state(context, &vm.name, &runner_role_id));
    StatusServicesOutputV2 {
        d2b: systemctl_state(context, "d2bd.service"),
        microvm: if is_qemu_media_vm(vm) {
            "unsupported".to_owned()
        } else {
            pidfd_role_state(context, &vm.name, &runner_role_id)
        },
        virtiofsd: pidfd_role_prefix_state(context, &vm.name, "virtiofsd"),
        qemu_media: qemu_media_state,
        gpu: gpu_role_id.map(|role| pidfd_role_state(context, &vm.name, role)),
        video: has_role(d2b_core::processes::ProcessRole::Video)
            .then(|| pidfd_role_state(context, &vm.name, "video")),
        snd: (has_role(d2b_core::processes::ProcessRole::Audio) || vm.audio)
            .then(|| pidfd_role_state(context, &vm.name, "audio")),
        swtpm: (has_role(d2b_core::processes::ProcessRole::Swtpm) || vm.tpm)
            .then(|| pidfd_role_state(context, &vm.name, "swtpm")),
    }
}

fn vm_runner_role_id(
    process_vm: Option<&d2b_core::processes::VmProcessDag>,
    vm: &ManifestVm,
) -> String {
    if process_vm
        .map(|entry| {
            entry
                .nodes
                .iter()
                .any(|node| node.role == d2b_core::processes::ProcessRole::QemuMediaRunner)
        })
        .unwrap_or(false)
        || is_qemu_media_vm(vm)
    {
        "qemu-media".to_owned()
    } else {
        "ch-runner".to_owned()
    }
}

pub(crate) fn pidfd_role_state(context: &Context, vm: &str, role: &str) -> String {
    pidfd_role_state_matching(context, vm, |candidate| candidate == role)
}

fn pidfd_role_prefix_state(context: &Context, vm: &str, prefix: &str) -> String {
    pidfd_role_state_matching(context, vm, |candidate| candidate.starts_with(prefix))
}

fn pidfd_role_state_matching<F>(context: &Context, vm: &str, role_matches: F) -> String
where
    F: Fn(&str) -> bool,
{
    let path = context.daemon_state_dir.join("pidfd-table.json");
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return "stopped".to_owned(),
        Err(_) => return "unknown".to_owned(),
    };
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return "unknown".to_owned();
    };
    let running = value
        .get("entries")
        .and_then(Value::as_array)
        .map(|entries| {
            entries.iter().any(|entry| {
                entry.get("vm").and_then(Value::as_str) == Some(vm)
                    && entry
                        .get("role")
                        .and_then(Value::as_str)
                        .map(&role_matches)
                        .unwrap_or(false)
            })
        })
        .unwrap_or(false);
    if running { "running" } else { "stopped" }.to_owned()
}

pub(crate) fn current_symlink(context: &Context, vm: &ManifestVm) -> Option<String> {
    read_symlink_target(&vm_state_dir(context, vm).join("current"))
}

pub(crate) fn booted_symlink(context: &Context, vm: &ManifestVm) -> Option<String> {
    read_symlink_target(&vm_state_dir(context, vm).join("booted"))
}

pub(crate) fn vm_state_dir(context: &Context, vm: &ManifestVm) -> PathBuf {
    context
        .state_root
        .as_ref()
        .map(|state_root| state_root.join(&vm.name))
        .unwrap_or_else(|| PathBuf::from(&vm.state_dir))
}

fn is_pending_restart(
    vm: &ManifestVm,
    services: &StatusServicesOutputV2,
    current: Option<&str>,
    booted: Option<&str>,
) -> bool {
    current
        .zip(booted)
        .map(|(current, booted)| current != booted)
        .unwrap_or(false)
        && vm_counts_as_running(vm, services)
}

fn vm_counts_as_running(vm: &ManifestVm, services: &StatusServicesOutputV2) -> bool {
    if vm.is_net_vm {
        return true;
    }
    if is_qemu_media_vm(vm) {
        return services
            .qemu_media
            .as_deref()
            .is_some_and(service_state_counts_as_running);
    }
    [
        Some(services.d2b.as_str()),
        Some(services.microvm.as_str()),
        services.gpu.as_deref(),
    ]
    .into_iter()
    .flatten()
    .any(service_state_counts_as_running)
}

fn service_state_counts_as_running(state: &str) -> bool {
    matches!(state, "active" | "activating" | "reloading" | "running")
}

pub(crate) fn list_status_label(
    vm: &ManifestVm,
    services: &StatusServicesOutputV2,
    pending_restart: bool,
) -> String {
    if vm.is_net_vm {
        "running".to_owned()
    } else if pending_restart {
        "pending-restart".to_owned()
    } else if services.microvm == "unknown"
        || (is_qemu_media_vm(vm) && services.qemu_media.as_deref() == Some("unknown"))
    {
        "unknown".to_owned()
    } else if vm_counts_as_running(vm, services) {
        "running".to_owned()
    } else {
        "stopped".to_owned()
    }
}

pub(crate) fn public_lifecycle_status_label(
    lifecycle: &d2b_contracts::public_wire::VmLifecycle,
) -> String {
    if lifecycle.pending_restart {
        return "pending-restart".to_owned();
    }
    match lifecycle.state {
        IpcVmLifecycleState::Stopped => "stopped",
        IpcVmLifecycleState::Starting => "starting",
        IpcVmLifecycleState::Booted | IpcVmLifecycleState::Running => "running",
        IpcVmLifecycleState::Stopping => "stopping",
        IpcVmLifecycleState::Restarting => "restarting",
        IpcVmLifecycleState::Failed => "failed",
        IpcVmLifecycleState::Unknown => "unknown",
    }
    .to_owned()
}

pub(crate) fn public_lifecycle_list_status_label(
    lifecycle: &d2b_contracts::public_wire::VmLifecycle,
) -> String {
    if lifecycle.pending_restart {
        return "pending-restart".to_owned();
    }
    match lifecycle.state {
        IpcVmLifecycleState::Stopped => "stopped",
        IpcVmLifecycleState::Booted
        | IpcVmLifecycleState::Running
        | IpcVmLifecycleState::Starting
        | IpcVmLifecycleState::Stopping
        | IpcVmLifecycleState::Restarting => "running",
        IpcVmLifecycleState::Failed => "failed",
        IpcVmLifecycleState::Unknown => "unknown",
    }
    .to_owned()
}

fn process_role_name(role: &d2b_core::processes::ProcessRole) -> String {
    match role {
        d2b_core::processes::ProcessRole::HostReconcile => "host-reconcile",
        d2b_core::processes::ProcessRole::StoreVirtiofsPreflight => "store-virtiofs-preflight",
        d2b_core::processes::ProcessRole::SwtpmPreStartFlush => "swtpm-pre-start-flush",
        d2b_core::processes::ProcessRole::Swtpm => "swtpm",
        d2b_core::processes::ProcessRole::Virtiofsd => "virtiofsd",
        d2b_core::processes::ProcessRole::Video => "video",
        d2b_core::processes::ProcessRole::Gpu => "gpu",
        d2b_core::processes::ProcessRole::GpuRenderNode => "gpu-render-node",
        d2b_core::processes::ProcessRole::Audio => "audio",
        d2b_core::processes::ProcessRole::CloudHypervisorRunner => "cloud-hypervisor-runner",
        d2b_core::processes::ProcessRole::QemuMediaRunner => "qemu-media-runner",
        d2b_core::processes::ProcessRole::VsockRelay => "vsock-relay",
        d2b_core::processes::ProcessRole::OtelHostBridge => "otel-host-bridge",
        d2b_core::processes::ProcessRole::GuestSshReadiness => "guest-ssh-readiness",
        d2b_core::processes::ProcessRole::GuestControlHealth => "guest-control-health",
        d2b_core::processes::ProcessRole::Usbip => "usbip",
        d2b_core::processes::ProcessRole::WaylandProxy => "wayland-proxy",
        d2b_core::processes::ProcessRole::SecurityKeyFrontend => "security-key-frontend",
    }
    .to_owned()
}

fn readiness_name(readiness: &d2b_core::processes::ReadinessPredicate) -> String {
    match readiness {
        d2b_core::processes::ReadinessPredicate::ApiSocketInfo(value) => {
            format!("api-socket-info:{value}")
        }
        d2b_core::processes::ReadinessPredicate::VsockNotify(value) => {
            format!("vsock-notify:{value}")
        }
        d2b_core::processes::ReadinessPredicate::UnixSocketExists(value) => {
            format!("unix-socket-exists:{value}")
        }
        d2b_core::processes::ReadinessPredicate::UnixSocketListening(value) => {
            format!("unix-socket-listening:{value}")
        }
        d2b_core::processes::ReadinessPredicate::TcpPort { host, port } => {
            format!("tcp-port:{host}:{port}")
        }
        d2b_core::processes::ReadinessPredicate::Command(argv) => {
            format!("command:{}", argv.join(" "))
        }
        d2b_core::processes::ReadinessPredicate::ComponentSpecific(value) => {
            format!("component-specific:{value}")
        }
        d2b_core::processes::ReadinessPredicate::GuestControlHealth { .. } => {
            "guest-control-health".to_owned()
        }
    }
}

fn readiness_name_for_node(
    node: &d2b_core::processes::ProcessNode,
    readiness: &d2b_core::processes::ReadinessPredicate,
) -> String {
    if node.role == d2b_core::processes::ProcessRole::QemuMediaRunner {
        match readiness {
            d2b_core::processes::ReadinessPredicate::UnixSocketListening(_)
            | d2b_core::processes::ReadinessPredicate::UnixSocketExists(_) => {
                return "qmp-listening".to_owned();
            }
            _ => {}
        }
    }
    readiness_name(readiness)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::ManifestRuntime;

    fn manifest_vm_with_runtime(kind: &str, capabilities: BTreeMap<String, bool>) -> ManifestVm {
        ManifestVm {
            name: "installer".to_owned(),
            env: Some("dev".to_owned()),
            graphics: false,
            tpm: false,
            audio: false,
            usbip_yubikey: false,
            static_ip: None,
            is_net_vm: false,
            state_dir: "/var/lib/d2b/vms/installer".to_owned(),
            bridge: "br-dev-lan".to_owned(),
            ssh_user: None,
            runtime: Some(ManifestRuntime {
                kind: kind.to_owned(),
                capabilities,
            }),
        }
    }

    #[test]
    fn qemu_media_default_capability_projection_is_stable() {
        let vm = manifest_vm_with_runtime("qemu-media", BTreeMap::new());

        assert_eq!(
            output_runtime_capabilities(&vm),
            vec!["display", "lifecycle", "usb-hotplug"]
        );
        assert_eq!(
            output_unsupported_capabilities(&vm),
            vec![
                "config-sync",
                "exec",
                "guest-control",
                "in-guest-observability",
                "keys",
                "ssh",
                "store-sync"
            ]
        );
    }

    #[test]
    fn explicit_runtime_capability_projection_overrides_qemu_defaults() {
        let vm = manifest_vm_with_runtime(
            "qemu-media",
            BTreeMap::from([
                ("guestControl".to_owned(), false),
                ("usbHotplug".to_owned(), true),
            ]),
        );

        assert_eq!(output_runtime_capabilities(&vm), vec!["usb-hotplug"]);
        assert_eq!(output_unsupported_capabilities(&vm), vec!["guest-control"]);
    }
}
