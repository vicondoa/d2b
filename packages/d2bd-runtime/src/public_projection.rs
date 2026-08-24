use std::{collections::HashSet, fs, path::Path};

use d2b_contracts_broker::broker_wire::RunnerRole;
use d2b_core::processes::{ProcessNode, ProcessRole, VmProcessDag};
use serde::Serialize;
use serde_json::{Value, json};

use crate::supervisor::pidfd_table::PidfdTable;
use crate::workload_target_index::{TargetResolution, TargetResolutionError, WorkloadTargetIndex};

const DEFAULT_VM_RUNNER_ROLE_ID: &str = "ch-runner";
const QEMU_MEDIA_DEFAULT_RUNTIME_CAPABILITIES: &[&str] = &["display", "lifecycle", "usb-hotplug"];
const QEMU_MEDIA_DEFAULT_UNSUPPORTED_CAPABILITIES: &[&str] = &[
    "config-sync",
    "exec",
    "in-guest-observability",
    "keys",
    concat!("s", "sh"),
    "store-sync",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicResolverLoad {
    NotNeeded,
    Optional,
    Required,
}

pub fn public_request_resolver_load(
    qemu_media_declared: bool,
    include_usb_status: bool,
    include_closure_metadata: bool,
) -> PublicResolverLoad {
    if qemu_media_declared {
        PublicResolverLoad::Required
    } else if include_usb_status || include_closure_metadata {
        PublicResolverLoad::Optional
    } else {
        PublicResolverLoad::NotNeeded
    }
}

pub fn public_pending_restart(manifest_entry: &Value) -> bool {
    let Some(state_dir) = manifest_entry.get("stateDir").and_then(Value::as_str) else {
        return false;
    };
    let state_dir = Path::new(state_dir);
    let current = fs::read_link(state_dir.join("current")).ok();
    let booted = fs::read_link(state_dir.join("booted")).ok();
    matches!((current, booted), (Some(current), Some(booted)) if current != booted)
}

pub fn public_guest_closure_out_path(
    manifest_entry: &Value,
    lifecycle: &Value,
    declared: Option<String>,
) -> Option<String> {
    let pending_restart = lifecycle
        .get("pendingRestart")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if pending_restart
        && let Some(state_dir) = manifest_entry.get("stateDir").and_then(Value::as_str)
        && let Ok(booted) = fs::read_link(Path::new(state_dir).join("booted"))
        && booted.is_absolute()
    {
        return Some(booted.to_string_lossy().into_owned());
    }
    declared
}

pub fn public_runtime_summary(lifecycle: &Value, manifest_entry: &Value) -> Value {
    let detail = lifecycle
        .get("state")
        .and_then(Value::as_str)
        .unwrap_or("Unknown")
        .to_ascii_lowercase();
    let mut runtime = serde_json::Map::new();
    runtime.insert("detail".to_owned(), Value::String(detail));
    if let Some(kind) = public_runtime_kind(manifest_entry) {
        runtime.insert("kind".to_owned(), Value::String(kind));
    }
    if let Some(operation_capabilities) = manifest_entry.pointer("/runtime/operationCapabilities") {
        runtime.insert(
            "operationCapabilities".to_owned(),
            operation_capabilities.clone(),
        );
    }
    if let Some(services) = manifest_entry.pointer("/runtime/services") {
        runtime.insert("services".to_owned(), services.clone());
    }
    Value::Object(runtime)
}

pub fn public_runtime_kind(manifest_entry: &Value) -> Option<String> {
    manifest_entry
        .pointer("/runtime/kind")
        .and_then(Value::as_str)
        .map(str::to_owned)
}

pub fn public_is_qemu_media(manifest_entry: &Value) -> bool {
    public_runtime_kind(manifest_entry).as_deref() == Some("qemu-media")
}

pub fn public_autostart_posture(manifest_entry: &Value) -> Option<Value> {
    public_is_qemu_media(manifest_entry).then(|| {
        json!({
            "mode": "manual-only",
            "reason": "qemu-media VMs are intentionally skipped by daemon autostart; start them explicitly with `d2b vm start <vm> --apply`"
        })
    })
}

pub fn public_runtime_capabilities(manifest_entry: &Value) -> Vec<String> {
    public_runtime_capabilities_by_support(
        manifest_entry,
        true,
        QEMU_MEDIA_DEFAULT_RUNTIME_CAPABILITIES,
    )
}

pub fn public_unsupported_capabilities(manifest_entry: &Value) -> Vec<String> {
    public_runtime_capabilities_by_support(
        manifest_entry,
        false,
        QEMU_MEDIA_DEFAULT_UNSUPPORTED_CAPABILITIES,
    )
}

fn public_runtime_capabilities_by_support(
    manifest_entry: &Value,
    supported: bool,
    qemu_media_default: &[&str],
) -> Vec<String> {
    let Some(capabilities) = manifest_entry
        .pointer("/runtime/capabilities")
        .and_then(Value::as_object)
    else {
        return if public_is_qemu_media(manifest_entry) {
            qemu_media_default
                .iter()
                .map(|value| (*value).to_owned())
                .collect()
        } else {
            Vec::new()
        };
    };
    let mut output = capabilities
        .iter()
        .filter(|(_name, value)| value.as_bool() == Some(supported))
        .map(|(name, _value)| capability_name_for_public_output(name))
        .collect::<Vec<_>>();
    output.sort();
    output.dedup();
    output
}

fn capability_name_for_public_output(name: &str) -> String {
    match name {
        "configSync" => "config-sync",
        "inGuestObservability" => "in-guest-observability",
        "storeSync" => "store-sync",
        "usbHotplug" => "usb-hotplug",
        other => other,
    }
    .to_owned()
}

pub fn public_service_capabilities(services: &Value) -> Vec<String> {
    let Some(services) = services.as_object() else {
        return Vec::new();
    };
    let mut capabilities = services
        .iter()
        .filter_map(|(name, state)| {
            if state.is_null() || state.as_str() == Some("unsupported") {
                None
            } else {
                Some(service_capability_name_for_public_output(name))
            }
        })
        .collect::<Vec<_>>();
    capabilities.sort();
    capabilities.dedup();
    capabilities
}

fn service_capability_name_for_public_output(name: &str) -> String {
    match name {
        "qemuMedia" => "qemu-media",
        "snd" => "audio",
        other => other,
    }
    .to_owned()
}

pub fn public_vm_runner_role_id(
    process_vm: Option<&VmProcessDag>,
    manifest_entry: &Value,
) -> String {
    if process_vm
        .map(|entry| {
            entry
                .nodes
                .iter()
                .any(|node| node.role == ProcessRole::QemuMediaRunner)
        })
        .unwrap_or(false)
        || public_is_qemu_media(manifest_entry)
    {
        RunnerRole::QemuMedia.as_str().to_owned()
    } else {
        DEFAULT_VM_RUNNER_ROLE_ID.to_owned()
    }
}

pub fn resolve_vm_filter_target(
    vm: Option<&str>,
    workload_index: Option<&WorkloadTargetIndex>,
    manifest: &serde_json::Map<String, Value>,
) -> Result<Option<String>, TargetResolutionError> {
    let Some(vm) = vm else {
        return Ok(None);
    };
    let known_legacy: HashSet<String> = manifest.keys().cloned().collect();
    let resolution = if let Some(index) = workload_index {
        index.resolve_target(vm, &known_legacy)?
    } else {
        TargetResolution::LegacyVmName(vm.to_owned())
    };
    Ok(Some(resolution.vm_name().to_owned()))
}

pub fn public_service_states(
    pidfd_table: &PidfdTable,
    vm: &str,
    manifest_entry: &Value,
    process_vm: Option<&VmProcessDag>,
) -> Value {
    let has_role = |role: ProcessRole| {
        process_vm
            .map(|entry| entry.nodes.iter().any(|node| node.role == role))
            .unwrap_or(false)
    };
    let gpu_role_id = if has_role(ProcessRole::GpuRenderNode) {
        Some("gpu-render-node")
    } else if has_role(ProcessRole::Gpu)
        || manifest_entry
            .get("graphics")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    {
        Some("gpu")
    } else {
        None
    };

    json!({
        "d2b": "active",
        "microvm": if public_is_qemu_media(manifest_entry) {
            "unsupported".to_owned()
        } else {
            public_pidfd_role_state(pidfd_table, vm, DEFAULT_VM_RUNNER_ROLE_ID)
        },
        "qemuMedia": public_is_qemu_media(manifest_entry)
            .then(|| public_pidfd_role_state(pidfd_table, vm, "qemu-media")),
        "virtiofsd": if public_is_qemu_media(manifest_entry) {
            "unsupported".to_owned()
        } else {
            public_pidfd_role_prefix_state(pidfd_table, vm, "virtiofsd")
        },
        "gpu": gpu_role_id.map(|role| public_pidfd_role_state(pidfd_table, vm, role)),
        "video": has_role(ProcessRole::Video)
            .then(|| public_pidfd_role_state(pidfd_table, vm, "video")),
        "snd": (has_role(ProcessRole::Audio)
            || manifest_entry.get("audio").and_then(Value::as_bool).unwrap_or(false))
            .then(|| public_pidfd_role_state(pidfd_table, vm, "audio")),
        "swtpm": (has_role(ProcessRole::Swtpm)
            || manifest_entry.get("tpm").and_then(Value::as_bool).unwrap_or(false))
            .then(|| public_pidfd_role_state(pidfd_table, vm, "swtpm")),
    })
}

pub fn public_pidfd_role_state(pidfd_table: &PidfdTable, vm: &str, role: &str) -> String {
    public_pidfd_role_state_matching(pidfd_table, vm, |candidate| candidate == role)
}

fn public_pidfd_role_prefix_state(pidfd_table: &PidfdTable, vm: &str, prefix: &str) -> String {
    public_pidfd_role_state_matching(pidfd_table, vm, |candidate| candidate.starts_with(prefix))
}

fn public_pidfd_role_state_matching<F>(
    pidfd_table: &PidfdTable,
    vm: &str,
    role_matches: F,
) -> String
where
    F: Fn(&str) -> bool,
{
    let running = pidfd_table.list_for_vm(vm).into_iter().any(|registration| {
        role_matches(&registration.role)
            && pidfd_table.still_alive_same_start_time(vm, &registration.role)
    });
    if running {
        "running".to_owned()
    } else {
        "stopped".to_owned()
    }
}

pub fn qemu_media_qmp_socket(node: &ProcessNode) -> Option<String> {
    node.readiness.iter().find_map(|predicate| match predicate {
        d2b_core::processes::ReadinessPredicate::UnixSocketListening(path)
        | d2b_core::processes::ReadinessPredicate::UnixSocketExists(path) => Some(path.clone()),
        _ => None,
    })
}

pub fn qemu_media_unix_socket_listening(path: &str) -> bool {
    const SO_ACCEPTCON: &str = "00010000";
    let Ok(contents) = fs::read_to_string("/proc/net/unix") else {
        return false;
    };
    contents.lines().skip(1).any(|line| {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        fields.get(3).copied() == Some(SO_ACCEPTCON) && fields.last().copied() == Some(path)
    })
}

pub fn serde_kebab_string<T: Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::supervisor::pidfd_table::{PidfdEntry, PidfdTable};
    use crate::supervisor::state::parse_proc_stat_starttime;
    use std::fs::{self, File};
    use std::os::fd::OwnedFd;

    fn current_process_entry() -> PidfdEntry {
        let pid = std::process::id() as i32;
        let stat = fs::read_to_string(format!("/proc/{pid}/stat")).expect("read current stat");
        let start_time_ticks = parse_proc_stat_starttime(&stat).expect("parse current start time");
        let pidfd: OwnedFd = File::open("/dev/null").expect("open dummy fd").into();
        PidfdEntry {
            pidfd,
            pid,
            start_time_ticks,
        }
    }

    #[test]
    fn public_service_states_follow_pidfd_roles() {
        let dir = tempfile::tempdir().expect("pidfd table dir");
        let pidfd_table = PidfdTable::new(dir.path().join("pidfd-table.json"));
        pidfd_table
            .register(
                "vm-a".to_owned(),
                "ch-runner".to_owned(),
                current_process_entry(),
            )
            .expect("register ch runner");
        pidfd_table
            .register(
                "vm-a".to_owned(),
                "virtiofsd-ro-store".to_owned(),
                current_process_entry(),
            )
            .expect("register virtiofsd");

        let services = public_service_states(
            &pidfd_table,
            "vm-a",
            &json!({ "graphics": false, "audio": false, "tpm": false }),
            None,
        );
        assert_eq!(services.get("d2b").and_then(Value::as_str), Some("active"));
        assert_eq!(
            services.get("microvm").and_then(Value::as_str),
            Some("running")
        );
        assert_eq!(
            services.get("virtiofsd").and_then(Value::as_str),
            Some("running")
        );
    }
}
