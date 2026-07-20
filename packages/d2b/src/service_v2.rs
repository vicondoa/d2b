//! Synchronous CLI adapter for the authenticated daemon ComponentSession.

use std::{collections::BTreeSet, fmt::Write as _, net::IpAddr, path::Path};

use d2b_contracts::{
    cli_output::{
        ApiReadyErrorV1, ApiReadySimple, ApiReadyStatusV1, ListItemOutputV2, ListOutputV2,
        LivePoolIntegrityOutputV1, RealmPolicyOutputV1, RunnerParityOutputV2,
        StatusInventoryOutputV2, StatusServicesOutputV2, StatusVmOutputV2,
    },
    public_wire::{
        QemuMediaRegistryStatus, QemuMediaRunnerStatus, QemuMediaSourceStatus, QemuMediaStatus,
        UsbProbeEntryKind, UsbipDurableClaimState, UsbipDurableClaimStatus,
        UsbipProbeDegradedReason, UsbipProbeDegradedReasonCode, UsbipProbeEntry, UsbipProbeStatus,
        UsbipVmStatus, VmAutostartPosture, VmLifecycleState,
    },
    v2_services::{MAX_PAGE_SIZE, common, daemon},
};
use d2b_daemon_access::{
    LocalUnixDaemonAccess,
    component_session::{
        CancellationToken, ClientError, DaemonLifecycleRequest, DaemonMethod, DaemonTerminal,
        GuestClient, LocalDaemonSession, Response, daemon_call_options,
    },
};
use protobuf::Enum;
use sha2::{Digest, Sha256};
use tokio::runtime::{Builder, Runtime};

use super::CliFailure;

const MAX_PAGES: usize = 1024;

pub(crate) struct DaemonService {
    runtime: Runtime,
    session: LocalDaemonSession,
}

impl std::fmt::Debug for DaemonService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("DaemonService([authenticated])")
    }
}

impl DaemonService {
    pub(crate) fn connect(path: &Path) -> Result<Self, CliFailure> {
        let runtime = Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .map_err(|error| {
                CliFailure::new(69, format!("failed to start client runtime: {error}"))
            })?;
        let access = LocalUnixDaemonAccess::with_socket_path(path);
        let session = runtime
            .block_on(access.connect_component_session())
            .map_err(client_failure)?;
        Ok(Self { runtime, session })
    }

    pub(crate) fn list_realms(&self) -> Result<Vec<daemon::RealmProjection>, CliFailure> {
        let cancellation = CancellationToken::default();
        collect_pages(
            |cursor| {
                self.runtime.block_on(self.session.daemon().list_realms(
                    MAX_PAGE_SIZE,
                    cursor,
                    daemon_call_options(false)?,
                    &cancellation,
                ))
            },
            |response| {
                let page = response.page.into_option().ok_or_else(|| {
                    CliFailure::new(76, "daemon omitted ListRealms pagination metadata")
                })?;
                Ok((response.realms, page))
            },
        )
    }

    pub(crate) fn list_workloads(
        &self,
        resource_id: Option<&str>,
    ) -> Result<Vec<daemon::WorkloadProjection>, CliFailure> {
        let cancellation = CancellationToken::default();
        collect_pages(
            |cursor| {
                self.runtime.block_on(self.session.daemon().list_workloads(
                    resource_id,
                    MAX_PAGE_SIZE,
                    cursor,
                    daemon_call_options(false)?,
                    &cancellation,
                ))
            },
            |response| {
                let page = response.page.into_option().ok_or_else(|| {
                    CliFailure::new(76, "daemon omitted ListWorkloads pagination metadata")
                })?;
                Ok((response.workloads, page))
            },
        )
    }

    pub(crate) fn inspect(
        &self,
        resource_id: Option<&str>,
    ) -> Result<(Vec<daemon::WorkloadProjection>, String), CliFailure> {
        let cancellation = CancellationToken::default();
        let mut read_model = None;
        let workloads = collect_pages(
            |cursor| {
                self.runtime.block_on(self.session.daemon().inspect(
                    resource_id,
                    MAX_PAGE_SIZE,
                    cursor,
                    daemon_call_options(false)?,
                    &cancellation,
                ))
            },
            |response| {
                if let Some(current) = &read_model {
                    if current != &response.read_model {
                        return Err(CliFailure::new(
                            76,
                            "daemon changed the inspect read model between pages",
                        ));
                    }
                } else {
                    read_model = Some(response.read_model.clone());
                }
                let page = response.page.into_option().ok_or_else(|| {
                    CliFailure::new(76, "daemon omitted Inspect pagination metadata")
                })?;
                Ok((response.workloads, page))
            },
        )?;
        Ok((workloads, read_model.unwrap_or_default()))
    }

    pub(crate) fn lifecycle(
        &self,
        method: DaemonMethod,
        resource_id: &str,
        desired_state: common::DesiredState,
        operation_id: &str,
    ) -> Result<Response, CliFailure> {
        let mut digest = Sha256::new();
        digest.update(b"d2b-cli-lifecycle-v2\0");
        digest.update(resource_id.as_bytes());
        digest.update(operation_id.as_bytes());
        digest.update(desired_state.value().to_be_bytes());
        self.runtime
            .block_on(self.session.daemon().lifecycle(
                DaemonLifecycleRequest {
                    method,
                    resource_id,
                    desired_state,
                    operation_id,
                    request_digest: digest.finalize().into(),
                },
                daemon_call_options(true).map_err(client_failure)?,
                &CancellationToken::default(),
            ))
            .map_err(client_failure)
    }

    pub(crate) fn open_terminal(
        &self,
        method: DaemonMethod,
        resource_id: &str,
        operation_id: &str,
        selection: d2b_contracts::v2_services::terminal::TerminalSelection,
    ) -> Result<DaemonTerminal, CliFailure> {
        self.open_terminal_typed(method, resource_id, operation_id, selection)
            .map_err(client_failure)
    }

    pub(crate) fn open_terminal_typed(
        &self,
        method: DaemonMethod,
        resource_id: &str,
        operation_id: &str,
        selection: d2b_contracts::v2_services::terminal::TerminalSelection,
    ) -> Result<DaemonTerminal, ClientError> {
        self.runtime.block_on(self.session.daemon().open_terminal(
            method,
            resource_id,
            operation_id,
            selection,
            daemon_call_options(true)?,
            &CancellationToken::default(),
        ))
    }

    pub(crate) fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    pub(crate) fn guest_typed(&self, workload: &str) -> Result<GuestClient, ClientError> {
        self.session.guest(workload)
    }
}

fn collect_pages<T, R>(
    mut request: impl FnMut(Option<&str>) -> Result<R, ClientError>,
    mut split: impl FnMut(R) -> Result<(Vec<T>, daemon::PageInfo), CliFailure>,
) -> Result<Vec<T>, CliFailure> {
    let mut cursor = None;
    let mut seen = BTreeSet::new();
    let mut entries = Vec::new();
    for _ in 0..MAX_PAGES {
        let response = request(cursor.as_deref()).map_err(client_failure)?;
        let (mut page_entries, page) = split(response)?;
        entries.append(&mut page_entries);
        if !page.truncated {
            return Ok(entries);
        }
        if page.next_page_cursor.is_empty() || !seen.insert(page.next_page_cursor.clone()) {
            return Err(CliFailure::new(
                76,
                "daemon returned a repeated or empty pagination cursor",
            ));
        }
        cursor = Some(page.next_page_cursor);
    }
    Err(CliFailure::new(
        76,
        "daemon pagination exceeded the bounded client page limit",
    ))
}

pub(crate) fn list_output(
    workloads: &[daemon::WorkloadProjection],
) -> Result<ListOutputV2, CliFailure> {
    workloads
        .iter()
        .map(list_item)
        .collect::<Result<Vec<_>, _>>()
        .map(ListOutputV2)
}

pub(crate) fn vm_lifecycle_state(
    workload: &daemon::WorkloadProjection,
) -> Result<VmLifecycleState, CliFailure> {
    let lifecycle = workload
        .lifecycle
        .as_ref()
        .ok_or_else(|| CliFailure::new(76, "workload response omitted lifecycle"))?;
    Ok(match lifecycle.state.enum_value_or_default() {
        daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_STOPPED => {
            VmLifecycleState::Stopped
        }
        daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_STARTING => {
            VmLifecycleState::Starting
        }
        daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_BOOTED => VmLifecycleState::Booted,
        daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_RUNNING => {
            VmLifecycleState::Running
        }
        daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_STOPPING => {
            VmLifecycleState::Stopping
        }
        daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_RESTARTING => {
            VmLifecycleState::Restarting
        }
        daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_FAILED => VmLifecycleState::Failed,
        _ => VmLifecycleState::Unknown,
    })
}

pub(crate) fn runtime_detail(workload: &daemon::WorkloadProjection) -> Result<&str, CliFailure> {
    workload
        .runtime
        .as_ref()
        .map(|runtime| runtime.detail.as_str())
        .ok_or_else(|| CliFailure::new(76, "workload response omitted runtime"))
}

/// Find the workload uniquely matching a bare id/name among an already
/// fetched typed `ListWorkloads` projection, disambiguating like the CLI's
/// other bare-name lookups (`gateway_lifecycle_state`). Used to resolve a
/// bare `d2b launch`/`d2b shell` target to its canonical
/// `<workload>.<realm>.d2b` address, since the v2 typed router only routes
/// a `ConfiguredLaunch` exec to the daemon when `resource_id` already ends
/// in `.d2b`.
pub(crate) fn match_workload_by_bare_id(
    workloads: Vec<daemon::WorkloadProjection>,
    requested: &str,
) -> Result<daemon::WorkloadProjection, CliFailure> {
    let mut candidates = workloads.into_iter().filter(|workload| {
        workload.name == requested
            || workload
                .identity
                .as_ref()
                .is_some_and(|identity| identity.workload_name == requested)
    });
    let Some(first) = candidates.next() else {
        return Err(CliFailure::new(
            2,
            format!("workload target `{requested}` was not found"),
        ));
    };
    if candidates.next().is_some() {
        return Err(CliFailure::new(
            2,
            format!(
                "workload id `{requested}` is ambiguous; use its canonical `<workload>.<realm>.d2b` target"
            ),
        ));
    }
    Ok(first)
}

fn list_item(workload: &daemon::WorkloadProjection) -> Result<ListItemOutputV2, CliFailure> {
    let identity = workload
        .identity
        .as_ref()
        .ok_or_else(|| CliFailure::new(76, "workload response omitted identity"))?;
    let lifecycle = workload
        .lifecycle
        .as_ref()
        .ok_or_else(|| CliFailure::new(76, "workload response omitted lifecycle"))?;
    let runtime = workload
        .runtime
        .as_ref()
        .ok_or_else(|| CliFailure::new(76, "workload response omitted runtime"))?;
    Ok(ListItemOutputV2 {
        name: workload.name.clone(),
        env: nonempty(&workload.environment),
        graphics: workload.graphics,
        tpm: workload.tpm,
        usbip: workload.usbip,
        static_ip: ip_address(&workload.static_ip)?,
        status: if lifecycle.degraded {
            format!(
                "{} (degraded)",
                lifecycle_state(lifecycle.state.enum_value_or_default())
            )
        } else {
            lifecycle_state(lifecycle.state.enum_value_or_default()).to_owned()
        },
        is_net_vm: workload.is_net_workload,
        guest_closure_out_path: workload
            .deployment
            .as_ref()
            .and_then(|deployment| nonempty(&deployment.declared_guest_closure)),
        runtime_kind: Some(runtime_kind(runtime.kind.enum_value_or_default()).to_owned()),
        autostart: workload
            .autostart
            .as_ref()
            .map(|autostart| VmAutostartPosture {
                mode: autostart_mode(autostart.mode.enum_value_or_default()).to_owned(),
                reason: autostart.reason.clone(),
            }),
        runtime_capabilities: capabilities(&runtime.supported_capabilities),
        service_capabilities: service_capabilities(&workload.services),
        unsupported_capabilities: capabilities(&runtime.unsupported_capabilities),
        qemu_media: workload.qemu_media.as_ref().map(qemu_media),
        runner_parity_ok: workload
            .runner_parity
            .as_ref()
            .map(|parity| parity.parity_ok),
        canonical_target: Some(identity.canonical_target.clone()),
    })
}

pub(crate) struct StatusProjection {
    pub(crate) output: StatusVmOutputV2,
    pub(crate) ssh_configured: bool,
    pub(crate) bridges: Vec<daemon::BridgeProjection>,
    pub(crate) degraded_reasons: Vec<daemon::DegradedReason>,
}

pub(crate) fn render_status(projection: &StatusProjection) -> String {
    let output = &projection.output;
    let mut text = String::new();
    let _ = writeln!(text, "=== {} ===", output.name);
    if let Some(target) = &output.canonical_target {
        let _ = writeln!(text, "workload target: {target}");
    }
    if let Some(environment) = &output.env {
        let _ = writeln!(text, "env: {environment}");
    }
    let _ = writeln!(text, "runtime: {}", output.runtime);
    if let Some(kind) = &output.runtime_kind {
        let _ = writeln!(text, "runtime kind: {kind}");
    }
    if let Some(autostart) = &output.autostart {
        let _ = writeln!(text, "autostart: {} ({})", autostart.mode, autostart.reason);
    }
    let _ = writeln!(text, "daemon: {}", output.services.d2b);
    let _ = writeln!(text, "backend-runner: {}", output.services.microvm);
    let _ = writeln!(text, "virtiofsd: {}", output.services.virtiofsd);
    for (name, state) in [
        ("qemu-media", output.services.qemu_media.as_deref()),
        ("gpu-runner", output.services.gpu.as_deref()),
        ("video", output.services.video.as_deref()),
        ("audio", output.services.snd.as_deref()),
        ("swtpm", output.services.swtpm.as_deref()),
    ] {
        if let Some(state) = state {
            let _ = writeln!(text, "{name}: {state}");
        }
    }
    if projection.ssh_configured {
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
        output.current.as_deref().unwrap_or("(missing)")
    );
    let _ = writeln!(
        text,
        "booted: {}",
        output.booted.as_deref().unwrap_or("(missing)")
    );
    if !output.runtime_capabilities.is_empty() {
        let _ = writeln!(
            text,
            "runtime capabilities: {}",
            output.runtime_capabilities.join(", ")
        );
    }
    if !output.unsupported_capabilities.is_empty() {
        let _ = writeln!(
            text,
            "unsupported capabilities: {}",
            output.unsupported_capabilities.join(", ")
        );
    }
    if !output.service_capabilities.is_empty() {
        let _ = writeln!(
            text,
            "service capabilities: {}",
            output.service_capabilities.join(", ")
        );
    }
    if !output.declared_roles.is_empty() {
        let _ = writeln!(text, "declared roles: {}", output.declared_roles.join(", "));
    }
    if !output.readiness.is_empty() {
        let _ = writeln!(text, "readiness: {}", output.readiness.join(", "));
    }
    for reason in &projection.degraded_reasons {
        let _ = writeln!(text, "degraded: {} - {}", reason.reason, reason.remediation);
    }
    if let Some(parity) = &output.runner_parity {
        let _ = writeln!(
            text,
            "runner parity: {} ({})",
            if parity.runner_parity_ok {
                "ok"
            } else {
                "drift"
            },
            parity.runner_parity_path
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
    text.push_str("BRIDGE               STATE      TAP\n");
    for bridge in &projection.bridges {
        let _ = writeln!(
            text,
            "{:<20} {:<10} {}",
            bridge.bridge,
            if bridge.present { "present" } else { "absent" },
            if bridge.tap.is_empty() {
                "-"
            } else {
                &bridge.tap
            }
        );
    }
    text
}

pub(crate) fn render_status_inventory(projections: &[StatusProjection]) -> String {
    let mut text = String::from("runtime: daemon-component-session-v2\n\n");
    for projection in projections {
        text.push_str(&render_status(projection));
        text.push('\n');
    }
    text
}

pub(crate) fn status_output(
    workloads: &[daemon::WorkloadProjection],
    read_model: &str,
) -> Result<(StatusInventoryOutputV2, Vec<StatusProjection>), CliFailure> {
    let projections = workloads
        .iter()
        .map(status_projection)
        .collect::<Result<Vec<_>, _>>()?;
    let inventory = StatusInventoryOutputV2 {
        runtime: "daemon-component-session-v2".to_owned(),
        read_model: nonempty(read_model).map(|source_fingerprint| {
            d2b_contracts::public_wire::PublicReadModelMetadata {
                schema_version: 2,
                kind: "daemon-component-session-v2".to_owned(),
                generation: projections
                    .iter()
                    .filter_map(|projection| {
                        workloads
                            .iter()
                            .find(|workload| workload.name == projection.output.name)
                            .and_then(|workload| workload.lifecycle.as_ref())
                            .map(|lifecycle| lifecycle.generation)
                    })
                    .max()
                    .unwrap_or(1),
                source_fingerprint,
                updated_at_unix_ms: 0,
                freshness: "current".to_owned(),
                deep_refresh: "not-requested".to_owned(),
            }
        }),
        vms: projections
            .iter()
            .map(|projection| projection.output.clone())
            .collect(),
    };
    Ok((inventory, projections))
}

fn status_projection(
    workload: &daemon::WorkloadProjection,
) -> Result<StatusProjection, CliFailure> {
    let identity = workload
        .identity
        .as_ref()
        .ok_or_else(|| CliFailure::new(76, "workload response omitted identity"))?;
    let lifecycle = workload
        .lifecycle
        .as_ref()
        .ok_or_else(|| CliFailure::new(76, "workload response omitted lifecycle"))?;
    let runtime = workload
        .runtime
        .as_ref()
        .ok_or_else(|| CliFailure::new(76, "workload response omitted runtime"))?;
    let services = status_services(&workload.services);
    let deployment = workload.deployment.as_ref();
    Ok(StatusProjection {
        output: StatusVmOutputV2 {
            name: workload.name.clone(),
            env: nonempty(&workload.environment),
            services,
            current: deployment.and_then(|value| nonempty(&value.current_generation)),
            booted: deployment.and_then(|value| nonempty(&value.booted_generation)),
            pending_restart: lifecycle.pending_restart,
            runtime: lifecycle_state(lifecycle.state.enum_value_or_default()).to_owned(),
            runtime_kind: Some(runtime_kind(runtime.kind.enum_value_or_default()).to_owned()),
            autostart: workload
                .autostart
                .as_ref()
                .map(|autostart| VmAutostartPosture {
                    mode: autostart_mode(autostart.mode.enum_value_or_default()).to_owned(),
                    reason: autostart.reason.clone(),
                }),
            runtime_capabilities: capabilities(&runtime.supported_capabilities),
            service_capabilities: service_capabilities(&workload.services),
            unsupported_capabilities: capabilities(&runtime.unsupported_capabilities),
            qemu_media: workload.qemu_media.as_ref().map(qemu_media),
            declared_roles: workload.declared_roles.clone(),
            readiness: workload
                .readiness
                .iter()
                .map(|value| {
                    format!(
                        "{}:{}={}",
                        value.role_id,
                        value.predicate_id,
                        service_state(value.state.enum_value_or_default())
                    )
                })
                .collect(),
            api_ready: api_ready(workload.api_ready.enum_value_or_default()),
            runner_parity: workload
                .runner_parity
                .as_ref()
                .map(|parity| RunnerParityOutputV2 {
                    declared_runner: parity.declared_runner.clone(),
                    runner_parity_path: parity.parity_reference.clone(),
                    runner_parity_ok: parity.parity_ok,
                }),
            live_pool_integrity: workload.live_pool_integrity.as_ref().map(|integrity| {
                LivePoolIntegrityOutputV1 {
                    status: service_state(integrity.state.enum_value_or_default()).to_owned(),
                    unknown_reason: nonempty(&integrity.reason),
                    audit_ref: nonempty(&integrity.audit_reference),
                    repair_attempted: integrity.repair_attempted,
                    remediation: nonempty(&integrity.remediation),
                }
            }),
            usb: workload.usb.as_ref().map(|usb| usb_status(workload, usb)),
            canonical_target: Some(identity.canonical_target.clone()),
        },
        ssh_configured: workload.ssh_configured,
        bridges: workload.bridge_checks.clone(),
        degraded_reasons: lifecycle.degraded_reasons.clone(),
    })
}

pub(crate) fn realm_rows(realms: &[daemon::RealmProjection]) -> Vec<RealmPolicyOutputV1> {
    realms
        .iter()
        .map(|realm| RealmPolicyOutputV1 {
            realm: realm.realm_path.clone(),
            mode: realm_mode(realm.mode.enum_value_or_default()).to_owned(),
            gateway_vm: nonempty(&realm.gateway_workload_id),
            gateway_target: nonempty(&realm.gateway_target),
            gateway_state: realm_state(realm.state.enum_value_or_default()).to_owned(),
            cross_realm_policy: cross_realm_policy(
                realm.cross_realm_policy.enum_value_or_default(),
            )
            .to_owned(),
            credential_boundary: credential_boundary(
                realm.credential_boundary.enum_value_or_default(),
            )
            .to_owned(),
        })
        .collect()
}

fn status_services(services: &[daemon::ServiceProjection]) -> StatusServicesOutputV2 {
    let find = |kind| {
        services
            .iter()
            .find(|service| service.kind.enum_value_or_default() == kind)
            .map(|service| service_state(service.state.enum_value_or_default()).to_owned())
    };
    StatusServicesOutputV2 {
        d2b: find(daemon::ServiceKind::SERVICE_KIND_DAEMON)
            .unwrap_or_else(|| "unsupported".to_owned()),
        microvm: find(daemon::ServiceKind::SERVICE_KIND_HYPERVISOR)
            .unwrap_or_else(|| "unsupported".to_owned()),
        virtiofsd: find(daemon::ServiceKind::SERVICE_KIND_VIRTIOFSD)
            .unwrap_or_else(|| "unsupported".to_owned()),
        qemu_media: find(daemon::ServiceKind::SERVICE_KIND_QEMU_MEDIA),
        gpu: find(daemon::ServiceKind::SERVICE_KIND_GPU),
        video: find(daemon::ServiceKind::SERVICE_KIND_VIDEO),
        snd: find(daemon::ServiceKind::SERVICE_KIND_AUDIO),
        swtpm: find(daemon::ServiceKind::SERVICE_KIND_SWTPM),
    }
}

fn service_capabilities(services: &[daemon::ServiceProjection]) -> Vec<String> {
    services
        .iter()
        .map(|service| service_kind(service.kind.enum_value_or_default()).to_owned())
        .collect()
}

fn capabilities(values: &[protobuf::EnumOrUnknown<daemon::RuntimeCapability>]) -> Vec<String> {
    values
        .iter()
        .map(|value| runtime_capability(value.enum_value_or_default()).to_owned())
        .collect()
}

fn qemu_media(value: &daemon::QemuMediaProjection) -> QemuMediaStatus {
    QemuMediaStatus {
        firmware_mode: qemu_firmware(value.firmware_mode.enum_value_or_default()).to_owned(),
        media: value
            .media
            .iter()
            .map(|source| QemuMediaSourceStatus {
                format: qemu_format(source.format.enum_value_or_default()).to_owned(),
                media_ref: source.media_ref.clone(),
                read_only: source.read_only,
                registry: QemuMediaRegistryStatus {
                    remediation: source
                        .registry
                        .as_ref()
                        .and_then(|registry| nonempty(&registry.remediation)),
                    state: source
                        .registry
                        .as_ref()
                        .map(|registry| service_state(registry.state.enum_value_or_default()))
                        .unwrap_or("unknown")
                        .to_owned(),
                },
                slot: source.slot.clone(),
                source_kind: qemu_source(source.source_kind.enum_value_or_default()).to_owned(),
            })
            .collect(),
        runner: QemuMediaRunnerStatus {
            pre_cont_progress: qemu_progress(value.pre_cont_progress.enum_value_or_default())
                .to_owned(),
            qmp_readiness: Some(
                qemu_readiness(value.qmp_readiness.enum_value_or_default()).to_owned(),
            ),
            role: "qemu-media".to_owned(),
            state: service_state(value.runner_state.enum_value_or_default()).to_owned(),
        },
    }
}

fn usb_status(
    workload: &daemon::WorkloadProjection,
    value: &daemon::UsbProjection,
) -> UsbipVmStatus {
    UsbipVmStatus {
        degraded: value.degraded,
        entries: value
            .devices
            .iter()
            .map(|device| {
                let state = device.state.enum_value_or_default();
                let owner = nonempty(&device.owner_workload_id);
                UsbipProbeEntry {
                    kind: UsbProbeEntryKind::Usbip,
                    vm: workload.name.clone(),
                    env: workload.environment.clone(),
                    bus_id: device.device_id.clone(),
                    lock_path: "broker-owned".to_owned(),
                    status: usb_state(state),
                    owner_vm: owner.clone(),
                    slot: nonempty(&device.slot),
                    media_ref: None,
                    source_kind: None,
                    candidate_bus_ids: device.candidate_device_ids.clone(),
                    follow_up_command: None,
                    durable_claim: UsbipDurableClaimStatus {
                        state: if owner.is_some() {
                            UsbipDurableClaimState::HeldByDesiredOwner
                        } else {
                            UsbipDurableClaimState::Missing
                        },
                        owner_vm: owner,
                    },
                    degraded_reasons: device
                        .degraded_reasons
                        .iter()
                        .map(|reason| UsbipProbeDegradedReason {
                            code: UsbipProbeDegradedReasonCode::Unknown,
                            summary: reason.reason.clone(),
                            remediation: reason.remediation.clone(),
                        })
                        .collect(),
                    remediation_commands: device
                        .degraded_reasons
                        .iter()
                        .map(|reason| reason.remediation.clone())
                        .collect(),
                    ..Default::default()
                }
            })
            .collect(),
    }
}

fn ip_address(bytes: &[u8]) -> Result<Option<String>, CliFailure> {
    match bytes {
        [] => Ok(None),
        [a, b, c, d] => Ok(Some(IpAddr::from([*a, *b, *c, *d]).to_string())),
        bytes if bytes.len() == 16 => {
            let address: [u8; 16] = bytes
                .try_into()
                .map_err(|_| CliFailure::new(76, "invalid typed IP address"))?;
            Ok(Some(IpAddr::from(address).to_string()))
        }
        _ => Err(CliFailure::new(76, "invalid typed IP address")),
    }
}

fn nonempty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn client_failure(error: ClientError) -> CliFailure {
    let exit = match error {
        ClientError::Remote {
            kind: d2b_daemon_access::component_session::RemoteErrorKind::NotFound,
            ..
        } => 1,
        ClientError::Cancelled => 130,
        ClientError::InvalidMetadata
        | ClientError::InvalidTarget
        | ClientError::InvalidMethod
        | ClientError::InvalidService => 2,
        ClientError::ConnectFailed
        | ClientError::TransportFailed
        | ClientError::SessionLost
        | ClientError::SessionEstablishment(_) => 69,
        _ => 76,
    };
    CliFailure::new(exit, error.to_string())
}

macro_rules! enum_names {
    ($function:ident, $type:ty, { $($variant:path => $name:literal),+ $(,)? }) => {
        fn $function(value: $type) -> &'static str {
            match value {
                $($variant => $name,)+
                _ => "unknown",
            }
        }
    };
}

enum_names!(lifecycle_state, daemon::WorkloadLifecycleState, {
    daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_STOPPED => "stopped",
    daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_STARTING => "starting",
    daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_BOOTED => "booted",
    daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_RUNNING => "running",
    daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_STOPPING => "stopping",
    daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_RESTARTING => "restarting",
    daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_FAILED => "failed",
    daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_UNKNOWN => "unknown"
});
enum_names!(runtime_kind, daemon::RuntimeKind, {
    daemon::RuntimeKind::RUNTIME_KIND_NIXOS => "nixos",
    daemon::RuntimeKind::RUNTIME_KIND_QEMU_MEDIA => "qemu-media",
    daemon::RuntimeKind::RUNTIME_KIND_UNSAFE_LOCAL => "unsafe-local",
    daemon::RuntimeKind::RUNTIME_KIND_ACA_SANDBOX => "aca-sandbox",
    daemon::RuntimeKind::RUNTIME_KIND_REMOTE => "remote"
});
enum_names!(runtime_capability, daemon::RuntimeCapability, {
    daemon::RuntimeCapability::RUNTIME_CAPABILITY_LIFECYCLE => "lifecycle",
    daemon::RuntimeCapability::RUNTIME_CAPABILITY_DISPLAY => "display",
    daemon::RuntimeCapability::RUNTIME_CAPABILITY_USB_HOTPLUG => "usb-hotplug",
    daemon::RuntimeCapability::RUNTIME_CAPABILITY_CONFIG_SYNC => "config-sync",
    daemon::RuntimeCapability::RUNTIME_CAPABILITY_EXEC => "exec",
    daemon::RuntimeCapability::RUNTIME_CAPABILITY_GUEST_CONTROL => "guest-control",
    daemon::RuntimeCapability::RUNTIME_CAPABILITY_IN_GUEST_OBSERVABILITY => "in-guest-observability",
    daemon::RuntimeCapability::RUNTIME_CAPABILITY_KEYS => "keys",
    daemon::RuntimeCapability::RUNTIME_CAPABILITY_SHELL => "shell",
    daemon::RuntimeCapability::RUNTIME_CAPABILITY_STORE_SYNC => "store-sync"
});
enum_names!(service_kind, daemon::ServiceKind, {
    daemon::ServiceKind::SERVICE_KIND_DAEMON => "d2b",
    daemon::ServiceKind::SERVICE_KIND_HYPERVISOR => "microvm",
    daemon::ServiceKind::SERVICE_KIND_QEMU_MEDIA => "qemu-media",
    daemon::ServiceKind::SERVICE_KIND_VIRTIOFSD => "virtiofsd",
    daemon::ServiceKind::SERVICE_KIND_GPU => "gpu",
    daemon::ServiceKind::SERVICE_KIND_VIDEO => "video",
    daemon::ServiceKind::SERVICE_KIND_AUDIO => "audio",
    daemon::ServiceKind::SERVICE_KIND_SWTPM => "swtpm",
    daemon::ServiceKind::SERVICE_KIND_GUEST_CONTROL => "guest-control",
    daemon::ServiceKind::SERVICE_KIND_OBSERVABILITY => "observability"
});
enum_names!(service_state, daemon::ServiceState, {
    daemon::ServiceState::SERVICE_STATE_ACTIVE => "active",
    daemon::ServiceState::SERVICE_STATE_INACTIVE => "inactive",
    daemon::ServiceState::SERVICE_STATE_STARTING => "starting",
    daemon::ServiceState::SERVICE_STATE_STOPPING => "stopping",
    daemon::ServiceState::SERVICE_STATE_FAILED => "failed",
    daemon::ServiceState::SERVICE_STATE_UNAVAILABLE => "unavailable",
    daemon::ServiceState::SERVICE_STATE_UNSUPPORTED => "unsupported",
    daemon::ServiceState::SERVICE_STATE_UNKNOWN => "unknown"
});
enum_names!(autostart_mode, daemon::AutostartMode, {
    daemon::AutostartMode::AUTOSTART_MODE_ENABLED => "enabled",
    daemon::AutostartMode::AUTOSTART_MODE_DISABLED => "disabled",
    daemon::AutostartMode::AUTOSTART_MODE_MANUAL_ONLY => "manual-only"
});
enum_names!(realm_mode, daemon::RealmMode, {
    daemon::RealmMode::REALM_MODE_HOST_LOCAL => "host-local",
    daemon::RealmMode::REALM_MODE_GATEWAY_BACKED => "gateway-backed"
});
enum_names!(realm_state, daemon::RealmState, {
    daemon::RealmState::REALM_STATE_READY => "ready",
    daemon::RealmState::REALM_STATE_DEGRADED => "degraded",
    daemon::RealmState::REALM_STATE_STOPPED => "stopped",
    daemon::RealmState::REALM_STATE_UNAVAILABLE => "unavailable"
});
enum_names!(cross_realm_policy, daemon::CrossRealmPolicy, {
    daemon::CrossRealmPolicy::CROSS_REALM_POLICY_DEFAULT_DENY => "default-deny"
});
enum_names!(credential_boundary, daemon::CredentialBoundary, {
    daemon::CredentialBoundary::CREDENTIAL_BOUNDARY_HOST_LOCAL => "host-local",
    daemon::CredentialBoundary::CREDENTIAL_BOUNDARY_GATEWAY_GUEST => "gateway-owned"
});
enum_names!(qemu_firmware, daemon::QemuMediaFirmwareMode, {
    daemon::QemuMediaFirmwareMode::QEMU_MEDIA_FIRMWARE_MODE_NONE => "none",
    daemon::QemuMediaFirmwareMode::QEMU_MEDIA_FIRMWARE_MODE_UEFI => "uefi"
});
enum_names!(qemu_readiness, daemon::QemuMediaReadiness, {
    daemon::QemuMediaReadiness::QEMU_MEDIA_READINESS_NOT_STARTED => "not-started",
    daemon::QemuMediaReadiness::QEMU_MEDIA_READINESS_PENDING => "pending",
    daemon::QemuMediaReadiness::QEMU_MEDIA_READINESS_READY => "ready"
});
enum_names!(qemu_progress, daemon::QemuMediaProgress, {
    daemon::QemuMediaProgress::QEMU_MEDIA_PROGRESS_NOT_STARTED => "not-started",
    daemon::QemuMediaProgress::QEMU_MEDIA_PROGRESS_WAITING_FOR_QMP => "waiting-for-qmp",
    daemon::QemuMediaProgress::QEMU_MEDIA_PROGRESS_PAUSED_BEFORE_CONT => "paused-before-cont",
    daemon::QemuMediaProgress::QEMU_MEDIA_PROGRESS_RUNNING => "running"
});
enum_names!(qemu_source, daemon::QemuMediaSourceKind, {
    daemon::QemuMediaSourceKind::QEMU_MEDIA_SOURCE_KIND_PHYSICAL_USB => "physical-usb",
    daemon::QemuMediaSourceKind::QEMU_MEDIA_SOURCE_KIND_IMAGE_FILE => "image-file"
});
enum_names!(qemu_format, daemon::QemuMediaFormat, {
    daemon::QemuMediaFormat::QEMU_MEDIA_FORMAT_RAW => "raw",
    daemon::QemuMediaFormat::QEMU_MEDIA_FORMAT_QCOW2 => "qcow2",
    daemon::QemuMediaFormat::QEMU_MEDIA_FORMAT_ISO => "iso"
});

fn api_ready(value: daemon::ApiReadyState) -> Option<ApiReadyStatusV1> {
    match value {
        daemon::ApiReadyState::API_READY_STATE_UNSPECIFIED => None,
        daemon::ApiReadyState::API_READY_STATE_READY => {
            Some(ApiReadyStatusV1::Simple(ApiReadySimple::Yes))
        }
        daemon::ApiReadyState::API_READY_STATE_PENDING => {
            Some(ApiReadyStatusV1::Simple(ApiReadySimple::Pending))
        }
        daemon::ApiReadyState::API_READY_STATE_TIMEOUT => {
            Some(ApiReadyStatusV1::Simple(ApiReadySimple::Timeout))
        }
        daemon::ApiReadyState::API_READY_STATE_ERROR => {
            Some(ApiReadyStatusV1::WithError(ApiReadyErrorV1 {
                error: "daemon-reported-api-ready-error".to_owned(),
            }))
        }
    }
}

fn usb_state(value: daemon::UsbDeviceState) -> UsbipProbeStatus {
    match value {
        daemon::UsbDeviceState::USB_DEVICE_STATE_ATTACHED => UsbipProbeStatus::Bound,
        daemon::UsbDeviceState::USB_DEVICE_STATE_READY => UsbipProbeStatus::Enrollable,
        daemon::UsbDeviceState::USB_DEVICE_STATE_DETACHED => UsbipProbeStatus::Unbound,
        daemon::UsbDeviceState::USB_DEVICE_STATE_CONFLICT
        | daemon::UsbDeviceState::USB_DEVICE_STATE_DEGRADED => UsbipProbeStatus::Degraded,
        daemon::UsbDeviceState::USB_DEVICE_STATE_UNAVAILABLE => UsbipProbeStatus::Stale,
        _ => UsbipProbeStatus::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protobuf::{EnumOrUnknown, MessageField};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn workload() -> daemon::WorkloadProjection {
        daemon::WorkloadProjection {
            identity: MessageField::some(daemon::WorkloadIdentityProjection {
                realm_id: "aaaaaaaaaaaaaaaaaaaa".to_owned(),
                workload_id: "bbbbbbbbbbbbbbbbbbba".to_owned(),
                realm_path: "local-root".to_owned(),
                workload_name: "workload".to_owned(),
                canonical_target: "workload.local-root.d2b".to_owned(),
                ..Default::default()
            }),
            name: "workload".to_owned(),
            environment: "work".to_owned(),
            graphics: true,
            tpm: true,
            static_ip: vec![10, 42, 0, 2],
            ssh_configured: true,
            lifecycle: MessageField::some(daemon::WorkloadLifecycleProjection {
                state: EnumOrUnknown::new(
                    daemon::WorkloadLifecycleState::WORKLOAD_LIFECYCLE_STATE_RUNNING,
                ),
                pending_restart: true,
                generation: 17,
                ..Default::default()
            }),
            runtime: MessageField::some(daemon::RuntimeProjection {
                kind: EnumOrUnknown::new(daemon::RuntimeKind::RUNTIME_KIND_NIXOS),
                detail: "running".to_owned(),
                supported_capabilities: vec![EnumOrUnknown::new(
                    daemon::RuntimeCapability::RUNTIME_CAPABILITY_EXEC,
                )],
                ..Default::default()
            }),
            services: vec![
                daemon::ServiceProjection {
                    kind: EnumOrUnknown::new(daemon::ServiceKind::SERVICE_KIND_DAEMON),
                    role_id: "daemon".to_owned(),
                    state: EnumOrUnknown::new(daemon::ServiceState::SERVICE_STATE_ACTIVE),
                    ..Default::default()
                },
                daemon::ServiceProjection {
                    kind: EnumOrUnknown::new(daemon::ServiceKind::SERVICE_KIND_HYPERVISOR),
                    role_id: "hypervisor".to_owned(),
                    state: EnumOrUnknown::new(daemon::ServiceState::SERVICE_STATE_ACTIVE),
                    ..Default::default()
                },
            ],
            deployment: MessageField::some(daemon::DeploymentProjection {
                declared_guest_closure: "/nix/store/declared".to_owned(),
                current_generation: "/nix/store/current".to_owned(),
                booted_generation: "/nix/store/booted".to_owned(),
                ..Default::default()
            }),
            declared_roles: vec!["daemon".to_owned(), "hypervisor".to_owned()],
            readiness: vec![daemon::ReadinessProjection {
                role_id: "daemon".to_owned(),
                predicate_id: "ready".to_owned(),
                state: EnumOrUnknown::new(daemon::ServiceState::SERVICE_STATE_ACTIVE),
                ..Default::default()
            }],
            api_ready: EnumOrUnknown::new(daemon::ApiReadyState::API_READY_STATE_READY),
            ..Default::default()
        }
    }

    #[test]
    fn typed_list_preserves_nonempty_json_fields() {
        let output = list_output(&[workload()]).expect("typed list output");
        let value = serde_json::to_value(output).expect("list JSON");
        assert_eq!(value[0]["name"], "workload");
        assert_eq!(value[0]["env"], "work");
        assert_eq!(value[0]["status"], "running");
        assert_eq!(value[0]["staticIp"], "10.42.0.2");
        assert_eq!(value[0]["canonicalTarget"], "workload.local-root.d2b");
        assert_eq!(value[0]["runtimeCapabilities"][0], "exec");
    }

    #[test]
    fn match_workload_by_bare_id_resolves_unique_match_to_canonical_target() {
        let matched = match_workload_by_bare_id(vec![workload()], "workload")
            .expect("unique bare id match resolves");
        assert_eq!(
            matched
                .identity
                .as_ref()
                .expect("fixture has identity")
                .canonical_target
                .as_str(),
            "workload.local-root.d2b"
        );
    }

    #[test]
    fn match_workload_by_bare_id_fails_closed_when_not_found() {
        let error =
            match_workload_by_bare_id(vec![workload()], "missing").expect_err("no match found");
        assert_eq!(error.exit_code, 2);
        assert!(error.message.contains("was not found"));
    }

    #[test]
    fn match_workload_by_bare_id_fails_closed_on_ambiguous_name() {
        let mut other = workload();
        let identity = other.identity.as_mut().expect("fixture has identity");
        identity.realm_id = "cccccccccccccccccccc".to_owned();
        identity.realm_path = "other-root".to_owned();
        identity.canonical_target = "workload.other-root.d2b".to_owned();
        let error = match_workload_by_bare_id(vec![workload(), other], "workload")
            .expect_err("ambiguous bare id is rejected");
        assert_eq!(error.exit_code, 2);
        assert!(error.message.contains("ambiguous"));
        assert!(error.message.contains("canonical"));
    }

    #[test]
    fn cli_runtime_drives_unix_io() {
        let runtime = Builder::new_current_thread()
            .enable_io()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let (mut writer, mut reader) = tokio::net::UnixStream::pair().unwrap();
            writer.write_all(b"io-ready").await.unwrap();
            let mut bytes = [0_u8; 8];
            reader.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes, b"io-ready");
        });
    }

    #[test]
    fn typed_status_preserves_json_and_human_fields() {
        let workload = workload();
        let (inventory, projections) =
            status_output(&[workload], "fingerprint-1").expect("typed status");
        let value = serde_json::to_value(&inventory).expect("status JSON");
        assert_eq!(value["runtime"], "daemon-component-session-v2");
        assert_eq!(value["vms"][0]["name"], "workload");
        assert_eq!(value["vms"][0]["runtime"], "running");
        assert_eq!(value["vms"][0]["current"], "/nix/store/current");
        assert_eq!(value["vms"][0]["apiReady"], "yes");
        let human = render_status(&projections[0]);
        assert!(human.contains("=== workload ==="));
        assert!(human.contains("runtime: running"));
        assert!(human.contains("workload target: workload.local-root.d2b"));
        assert!(human.contains("pending-restart: yes"));
    }

    #[test]
    fn typed_outputs_preserve_qemu_media_fields() {
        let mut workload = workload();
        workload.runtime.as_mut().unwrap().kind =
            EnumOrUnknown::new(daemon::RuntimeKind::RUNTIME_KIND_QEMU_MEDIA);
        workload.qemu_media = MessageField::some(daemon::QemuMediaProjection {
            firmware_mode: EnumOrUnknown::new(
                daemon::QemuMediaFirmwareMode::QEMU_MEDIA_FIRMWARE_MODE_UEFI,
            ),
            runner_state: EnumOrUnknown::new(daemon::ServiceState::SERVICE_STATE_ACTIVE),
            qmp_readiness: EnumOrUnknown::new(
                daemon::QemuMediaReadiness::QEMU_MEDIA_READINESS_READY,
            ),
            pre_cont_progress: EnumOrUnknown::new(
                daemon::QemuMediaProgress::QEMU_MEDIA_PROGRESS_RUNNING,
            ),
            media: vec![daemon::QemuMediaSourceProjection {
                media_ref: "installer".to_owned(),
                slot: "cdrom".to_owned(),
                source_kind: EnumOrUnknown::new(
                    daemon::QemuMediaSourceKind::QEMU_MEDIA_SOURCE_KIND_IMAGE_FILE,
                ),
                format: EnumOrUnknown::new(daemon::QemuMediaFormat::QEMU_MEDIA_FORMAT_ISO),
                read_only: true,
                registry: MessageField::some(daemon::QemuMediaRegistryProjection {
                    state: EnumOrUnknown::new(daemon::ServiceState::SERVICE_STATE_ACTIVE),
                    ..Default::default()
                }),
                ..Default::default()
            }],
            ..Default::default()
        });
        let list = serde_json::to_value(list_output(&[workload.clone()]).unwrap()).unwrap();
        assert_eq!(list[0]["runtimeKind"], "qemu-media");
        assert_eq!(list[0]["qemuMedia"]["firmwareMode"], "uefi");
        assert_eq!(list[0]["qemuMedia"]["media"][0]["mediaRef"], "installer");
        let (status, _) = status_output(&[workload], "fingerprint").unwrap();
        let status = serde_json::to_value(status).unwrap();
        assert_eq!(status["vms"][0]["qemuMedia"]["runner"]["state"], "active");
        assert_eq!(
            status["vms"][0]["qemuMedia"]["runner"]["qmpReadiness"],
            "ready"
        );
    }

    #[test]
    fn pagination_consumes_truncation_and_rejects_repeated_cursors() {
        let mut calls = 0;
        let values = collect_pages(
            |_| {
                calls += 1;
                Ok::<_, ClientError>(calls)
            },
            |call| {
                Ok((
                    vec![call],
                    daemon::PageInfo {
                        truncated: call == 1,
                        next_page_cursor: if call == 1 {
                            "next".to_owned()
                        } else {
                            String::new()
                        },
                        returned_items: 1,
                        total_items_known: true,
                        total_items: 2,
                        ..Default::default()
                    },
                ))
            },
        )
        .expect("two pages");
        assert_eq!(values, vec![1, 2]);

        let failure = collect_pages(
            |_| Ok::<_, ClientError>(()),
            |_| {
                Ok((
                    vec![()],
                    daemon::PageInfo {
                        truncated: true,
                        next_page_cursor: "same".to_owned(),
                        returned_items: 1,
                        ..Default::default()
                    },
                ))
            },
        )
        .unwrap_err();
        assert_eq!(failure.exit_code, 76);
    }
}
