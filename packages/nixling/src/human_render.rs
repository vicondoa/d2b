use std::fmt::Write as _;

use nixling_ipc::cli_output::{
    AuthRoleV2, AuthStatusOutputV2, HostCheckOutputV2, HostCheckSeverityV2, ListOutputV2,
    StatusInventoryOutputV2, StatusVmOutputV2,
};

use crate::{
    BridgeHealthRow, BundleContext, Context, ManifestDocument, ManifestVm, collect_bridge_rows,
};

pub(crate) fn render_list_human(output: &ListOutputV2) -> String {
    let mut text = String::from(
        "NAME               ENV       GRAPHICS  TPM   USBIP   STATIC_IP       STATUS\n",
    );
    for item in &output.0 {
        let status = if item.is_net_vm {
            format!("{} (net-vm)", item.status)
        } else if item.runtime_kind.as_deref() == Some("qemu-media") {
            let mut label = format!("{} (qemu-media, manual-only)", item.status);
            if let Some(qemu) = &item.qemu_media {
                label.push_str(&format!(
                    ", qmp={}",
                    qemu.runner.qmp_readiness.as_deref().unwrap_or("unknown")
                ));
            }
            if !item.unsupported_capabilities.is_empty() {
                label.push_str(&format!(
                    ", unsupported={}",
                    item.unsupported_capabilities.join(",")
                ));
            }
            if !item.runtime_capabilities.is_empty() {
                label.push_str(&format!(", caps={}", item.runtime_capabilities.join(",")));
            }
            label
        } else {
            item.status.clone()
        };
        let static_ip = item.static_ip.clone().unwrap_or_else(|| "-".to_owned());
        let _ = writeln!(
            text,
            "{:<18} {:<9} {:<9} {:<5} {:<7} {:<15} {}",
            item.name,
            item.env.clone().unwrap_or_else(|| "-".to_owned()),
            item.graphics,
            item.tpm,
            item.usbip,
            static_ip,
            status,
        );
    }
    text
}

pub(crate) fn render_status_vm_human(
    output: &StatusVmOutputV2,
    manifest_vm: &ManifestVm,
    bridge_rows: Vec<BridgeHealthRow>,
) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "=== {} ===", output.name);
    if let Some(env) = &output.env {
        let _ = writeln!(text, "env: {env}");
    }
    let _ = writeln!(text, "runtime: {}", output.runtime);
    if let Some(kind) = &output.runtime_kind {
        let _ = writeln!(text, "runtime kind: {kind}");
    }
    if let Some(autostart) = &output.autostart {
        let _ = writeln!(text, "autostart: {} ({})", autostart.mode, autostart.reason);
    }
    let _ = writeln!(text, "nixling@{}: {}", output.name, output.services.nixling);
    if let Some(qemu) = &output.qemu_media {
        let _ = writeln!(
            text,
            "qemu-media runner: {}",
            output
                .services
                .qemu_media
                .clone()
                .unwrap_or_else(|| qemu.runner.state.clone())
        );
        let _ = writeln!(text, "firmware mode: {}", qemu.firmware_mode);
        let _ = writeln!(
            text,
            "qmp readiness: {}",
            qemu.runner.qmp_readiness.as_deref().unwrap_or("unknown")
        );
        let _ = writeln!(text, "pre-cont progress: {}", qemu.runner.pre_cont_progress);
        if qemu.media.is_empty() {
            let _ = writeln!(text, "media: no declared qemu-media sources");
        } else {
            text.push_str("media:\n");
            for source in &qemu.media {
                let _ = writeln!(
                    text,
                    "  - slot={} ref={} kind={} format={} readOnly={} registry={}",
                    source.slot,
                    source.media_ref,
                    source.source_kind,
                    source.format,
                    source.read_only,
                    source.registry.state,
                );
                if let Some(remediation) = &source.registry.remediation {
                    let _ = writeln!(text, "    remediation: {remediation}");
                }
            }
        }
        if !output.unsupported_capabilities.is_empty() {
            let _ = writeln!(
                text,
                "unsupported capabilities: {}",
                output.unsupported_capabilities.join(", ")
            );
        }
        if !output.runtime_capabilities.is_empty() {
            let _ = writeln!(
                text,
                "runtime capabilities: {}",
                output.runtime_capabilities.join(", ")
            );
        }
        if !output.service_capabilities.is_empty() {
            let _ = writeln!(
                text,
                "service capabilities: {}",
                output.service_capabilities.join(", ")
            );
        }
    } else {
        let _ = writeln!(
            text,
            "microvm@{} (backend): {}",
            output.name, output.services.microvm
        );
        let _ = writeln!(text, "virtiofsd: {}", output.services.virtiofsd);
        let _ = writeln!(
            text,
            "interactive: {}",
            output
                .services
                .gpu
                .clone()
                .unwrap_or_else(|| "stopped".to_owned())
        );
    }
    if let Some(video) = &output.services.video {
        let _ = writeln!(text, "video: {video}");
    }
    if manifest_vm.ssh_user.is_some() && manifest_vm.static_ip.is_some() {
        let _ = writeln!(text, "ssh: declared");
    }
    let _ = writeln!(
        text,
        "pending-restart: {}",
        if output.pending_restart { "yes" } else { "no" }
    );
    let _ = writeln!(
        text,
        "current: {}",
        output
            .current
            .clone()
            .unwrap_or_else(|| "(missing)".to_owned())
    );
    let _ = writeln!(
        text,
        "booted: {}",
        output
            .booted
            .clone()
            .unwrap_or_else(|| "(missing)".to_owned())
    );
    if !output.declared_roles.is_empty() {
        let _ = writeln!(text, "declared roles: {}", output.declared_roles.join(", "));
    }
    if !output.readiness.is_empty() {
        let _ = writeln!(text, "readiness: {}", output.readiness.join(", "));
    }
    if let Some(runner_parity) = &output.runner_parity {
        let _ = writeln!(
            text,
            "runner parity: {} ({})",
            if runner_parity.runner_parity_ok {
                "ok"
            } else {
                "drift"
            },
            runner_parity.runner_parity_path,
        );
    }
    if let Some(integrity) = &output.live_pool_integrity {
        let _ = writeln!(text, "live-pool integrity: {}", integrity.status);
        if let Some(reason) = &integrity.unknown_reason {
            let _ = writeln!(text, "live-pool unknown reason: {reason}");
        }
        if let Some(remediation) = &integrity.remediation {
            let _ = writeln!(text, "live-pool remediation: {remediation}");
        }
    }
    text.push_str("\n=== Bridge health ===\n");
    text.push_str("BRIDGE               STATE      ADMIN   EXPECTED     RESULT\n");
    for row in bridge_rows {
        let _ = writeln!(
            text,
            "{:<20} {:<10} {:<7} {:<12} {}",
            row.name, row.state, row.admin, row.expected_carrier, row.result
        );
    }
    text
}

pub(crate) fn render_status_inventory_human(
    output: &StatusInventoryOutputV2,
    manifest: &ManifestDocument,
    context: &Context,
    bundle: Option<&BundleContext>,
) -> String {
    let mut text = String::new();
    let _ = writeln!(text, "runtime: {}", output.runtime);
    text.push('\n');
    for vm in &output.vms {
        if let Some(manifest_vm) = manifest.get_vm(&vm.name) {
            text.push_str(&render_status_vm_human(
                vm,
                manifest_vm,
                collect_bridge_rows(context, manifest, bundle),
            ));
            text.push('\n');
        }
    }
    text
}

pub(crate) fn render_host_check_human(output: &HostCheckOutputV2) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "mode: {}\nstrict: {}\nsummary: pass={} warn={} fail={}\nexit-code: {}\n",
        output.mode,
        output.strict,
        output.summary.pass,
        output.summary.warn,
        output.summary.fail,
        output.exit_code
    );
    for severity in [
        HostCheckSeverityV2::Pass,
        HostCheckSeverityV2::Warn,
        HostCheckSeverityV2::Fail,
    ] {
        let label = match severity {
            HostCheckSeverityV2::Pass => "PASS",
            HostCheckSeverityV2::Warn => "WARN",
            HostCheckSeverityV2::Fail => "FAIL",
        };
        let matching = output
            .findings
            .iter()
            .filter(|finding| finding.severity == severity)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            continue;
        }
        let _ = writeln!(text, "{label}");
        for finding in matching {
            if let Some(vm) = &finding.vm {
                let _ = writeln!(text, "- [{}] {}: {}", vm, finding.id, finding.message);
            } else {
                let _ = writeln!(text, "- {}: {}", finding.id, finding.message);
            }
            let _ = writeln!(text, "  hint: {}", finding.remediation);
        }
        text.push('\n');
    }
    text
}

pub(crate) fn render_auth_status_human(output: &AuthStatusOutputV2) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "role: {}",
        match output.role {
            AuthRoleV2::None => "none",
            AuthRoleV2::Launcher => "launcher",
            AuthRoleV2::Admin => "admin",
        }
    );
    let _ = writeln!(text, "effective uid: {}", output.effective_uid);
    text.push_str("sockets:\n");
    for socket in &output.sockets {
        let _ = writeln!(
            text,
            "- {}: {}{}",
            socket.name,
            if socket.reachable {
                "reachable"
            } else {
                "unreachable"
            },
            socket
                .version
                .as_ref()
                .map(|version| format!(" (version {version})"))
                .unwrap_or_default(),
        );
    }
    let _ = writeln!(
        text,
        "allowed subcommands: {}",
        output.allowed_subcommands.join(", ")
    );
    if !output.denied_subcommands.is_empty() {
        text.push_str("denied subcommands:\n");
        for denied in &output.denied_subcommands {
            let _ = writeln!(text, "- {}: {}", denied.name, denied.reason);
        }
    }
    text
}
