//! Local host provider adapters for ADR 0032.
//!
//! The adapters in this crate are intentionally thin: they expose the
//! existing `d2b-host` argv generators through the provider trait
//! surface without spawning or changing runtime behavior.

use async_trait::async_trait;
use d2b_core::host::HostJson;
use d2b_core::host_check::{self, HostCheckReport, HostCheckSeverity};
use d2b_host::{
    qemu_media_argv::{QemuMediaArgvError, generate_qemu_media_argv},
    wayland_proxy_argv::{WaylandProxyArgvError, generate_wayland_proxy_argv},
};
use d2b_realm_core::{Capability, CapabilitySet, ErrorKind, ProviderId};
use d2b_realm_provider::{
    DisplayProvider, HostSubstrateProvider, RuntimeProvider,
    capabilities::{
        DisplayCapabilitySet, HostSubstrateKind, NodeCapabilitySet, RuntimeCapabilitySet,
    },
    error::{ProviderError, ProviderResult},
    types::{
        DisplaySessionHandle, DisplaySessionId, DisplaySessionRequest, RuntimeHandle, RuntimePlan,
        RuntimeStatus, WorkloadSpec,
    },
};

pub use d2b_host::{
    ch_argv::ChArgvInput,
    qemu_media_argv::QemuMediaArgvInput,
    runtime_provider::{
        CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID,
        CloudHypervisorRuntimeProvider as LocalMicroVmProvider, RuntimeWorkloadRequirements,
        validate_runtime_profile,
    },
    wayland_proxy_argv::WaylandProxyArgvInput,
};

const LOCAL_QEMU_MEDIA_RUNTIME_PROVIDER_ID: &str = "local-qemu-media";
const LOCAL_CROSS_DOMAIN_WAYLAND_PROVIDER_ID: &str = "local-cross-domain-wayland";
const NIXOS_HOST_SUBSTRATE_PROVIDER_ID: &str = "nixos-host-substrate";
const GENERIC_LINUX_HOST_SUBSTRATE_PROVIDER_ID: &str = "generic-linux-host-substrate";

/// Host substrate check adapter backed by the existing `host_check` report.
#[derive(Clone)]
pub struct HostCheckSubstrateProvider {
    provider_id: ProviderId,
    host: HostJson,
    strict: bool,
    substrate: HostSubstrateKind,
    substrate_version: Option<String>,
}

impl std::fmt::Debug for HostCheckSubstrateProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostCheckSubstrateProvider")
            .field("provider_id", &self.provider_id)
            .field("strict", &self.strict)
            .field("substrate", &self.substrate)
            .field("substrate_version", &self.substrate_version)
            .finish_non_exhaustive()
    }
}

impl HostCheckSubstrateProvider {
    /// NixOS host-substrate provider.
    pub fn nixos(host: HostJson) -> Self {
        Self::with_provider_id_and_substrate(
            static_provider_id(NIXOS_HOST_SUBSTRATE_PROVIDER_ID),
            host,
            HostSubstrateKind::NixOs,
            None,
        )
    }

    /// Generic Linux host-substrate provider.
    pub fn generic_linux(host: HostJson) -> Self {
        Self::with_provider_id_and_substrate(
            static_provider_id(GENERIC_LINUX_HOST_SUBSTRATE_PROVIDER_ID),
            host,
            HostSubstrateKind::GenericLinux,
            None,
        )
    }

    /// Generic Linux host-substrate provider with `os-release` detection.
    pub fn generic_linux_from_os_release(host: HostJson, os_release: &str) -> Self {
        let detected = detect_os_release(os_release);
        Self::with_provider_id_and_substrate(
            static_provider_id(GENERIC_LINUX_HOST_SUBSTRATE_PROVIDER_ID),
            host,
            detected.kind,
            detected.version,
        )
    }

    /// Construct with an explicit provider id.
    pub fn with_provider_id(provider_id: ProviderId, host: HostJson) -> Self {
        Self::with_provider_id_and_substrate(
            provider_id,
            host,
            HostSubstrateKind::GenericLinux,
            None,
        )
    }

    /// Construct with an explicit provider id and substrate metadata.
    pub fn with_provider_id_and_substrate(
        provider_id: ProviderId,
        host: HostJson,
        substrate: HostSubstrateKind,
        substrate_version: Option<String>,
    ) -> Self {
        Self {
            provider_id,
            host,
            strict: true,
            substrate,
            substrate_version,
        }
    }

    /// Override strict host-check behavior.
    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Run the existing host-check report without changing host state.
    pub async fn check_report(&self) -> ProviderResult<HostCheckReport> {
        let host = self.host.clone();
        let strict = self.strict;
        tokio::task::spawn_blocking(move || host_check::run(&host, std::iter::empty(), strict))
            .await
            .map_err(|_| {
                ProviderError::new(
                    ErrorKind::ProviderAllocationFailed,
                    "blocking task failed while checking host substrate",
                )
            })?
            .map_err(|_| {
                ProviderError::new(
                    ErrorKind::ProviderAllocationFailed,
                    "host substrate probe failed",
                )
            })
    }
}

#[async_trait]
impl HostSubstrateProvider for HostCheckSubstrateProvider {
    fn provider_id(&self) -> ProviderId {
        self.provider_id.clone()
    }

    async fn check(&self) -> ProviderResult<NodeCapabilitySet> {
        let report = self.check_report().await?;
        if report.summary.fail > 0 {
            return Err(host_check_failed(report));
        }
        Ok(self.host_check_capabilities(&report))
    }
}

impl HostCheckSubstrateProvider {
    fn host_check_capabilities(&self, report: &HostCheckReport) -> NodeCapabilitySet {
        let mut caps = CapabilitySet::empty();
        if report.summary.fail == 0 {
            caps = caps
                .with(Capability::Lifecycle)
                .with(Capability::Vsock)
                .with(Capability::Virtiofs);
        }
        NodeCapabilitySet {
            caps,
            substrate: Some(self.substrate),
            substrate_version: self.substrate_version.clone(),
            userns_available: report.summary.fail == 0,
            vhost_acceleration: report.summary.fail == 0,
            lsm: Some("unknown".to_owned()),
        }
    }
}

fn host_check_failed(report: HostCheckReport) -> ProviderError {
    let first = report
        .findings
        .iter()
        .find(|finding| finding.severity == HostCheckSeverity::Fail);
    let message = first
        .map(|finding| {
            format!(
                "host substrate check failed: {}; remediation: {}",
                finding.id, finding.remediation
            )
        })
        .unwrap_or_else(|| "host substrate check failed".to_owned());
    ProviderError::new(ErrorKind::ProviderAllocationFailed, message)
}

/// RuntimeProvider adapter for the local qemu-media runner scaffold.
#[derive(Clone)]
pub struct LocalQemuMediaRuntimeProvider {
    provider_id: ProviderId,
    argv_input: QemuMediaArgvInput,
}

impl std::fmt::Debug for LocalQemuMediaRuntimeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalQemuMediaRuntimeProvider")
            .field("provider_id", &self.provider_id)
            .finish_non_exhaustive()
    }
}

impl LocalQemuMediaRuntimeProvider {
    /// Construct a qemu-media provider using the realm metadata provider id.
    pub fn new(argv_input: QemuMediaArgvInput) -> Self {
        Self::with_provider_id(
            static_provider_id(LOCAL_QEMU_MEDIA_RUNTIME_PROVIDER_ID),
            argv_input,
        )
    }

    /// Construct a qemu-media provider with an explicit id.
    pub fn with_provider_id(provider_id: ProviderId, argv_input: QemuMediaArgvInput) -> Self {
        Self {
            provider_id,
            argv_input,
        }
    }

    /// Borrow the qemu-media argv input this adapter wraps.
    pub fn argv_input(&self) -> &QemuMediaArgvInput {
        &self.argv_input
    }

    /// Generate qemu-media argv through the existing d2b-host generator.
    pub async fn qemu_media_argv(&self) -> ProviderResult<Vec<String>> {
        render_qemu_media_argv(self.argv_input.clone()).await
    }

    fn ensure_workload_matches(&self, spec: &WorkloadSpec) -> ProviderResult<()> {
        if spec.alias.as_str() == self.argv_input.vm_name.as_str() {
            Ok(())
        } else {
            Err(ProviderError::new(
                ErrorKind::InvalidTarget,
                "workload alias does not match the local qemu-media runtime input",
            ))
        }
    }
}

#[async_trait]
impl RuntimeProvider for LocalQemuMediaRuntimeProvider {
    fn provider_id(&self) -> ProviderId {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> RuntimeCapabilitySet {
        RuntimeCapabilitySet {
            caps: CapabilitySet::empty(),
        }
    }

    async fn plan_workload(&self, spec: WorkloadSpec) -> ProviderResult<RuntimePlan> {
        self.ensure_workload_matches(&spec)?;
        self.qemu_media_argv().await?;
        Ok(RuntimePlan {
            provider: self.provider_id(),
            workload: spec.alias,
        })
    }

    async fn start(&self, plan: RuntimePlan) -> ProviderResult<RuntimeHandle> {
        if plan.provider != self.provider_id {
            return Err(ProviderError::new(
                ErrorKind::InvalidTarget,
                "runtime plan provider does not match local qemu-media provider",
            ));
        }
        Err(ProviderError::unsupported(format!(
            "runtime provider '{LOCAL_QEMU_MEDIA_RUNTIME_PROVIDER_ID}' start requires daemon runtime control"
        )))
    }

    async fn stop(&self, _handle: RuntimeHandle) -> ProviderResult<()> {
        Err(ProviderError::unsupported(format!(
            "runtime provider '{LOCAL_QEMU_MEDIA_RUNTIME_PROVIDER_ID}' stop requires daemon runtime control"
        )))
    }

    async fn inspect(&self, _handle: RuntimeHandle) -> ProviderResult<RuntimeStatus> {
        Err(ProviderError::unsupported(format!(
            "runtime provider '{LOCAL_QEMU_MEDIA_RUNTIME_PROVIDER_ID}' inspect requires daemon runtime control"
        )))
    }
}

/// Result of parsing `/etc/os-release`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedHostSubstrate {
    /// Detected substrate family.
    pub kind: HostSubstrateKind,
    /// Detected VERSION_ID when present.
    pub version: Option<String>,
}

/// Parse `/etc/os-release` contents into a low-cardinality substrate family.
pub fn detect_os_release(contents: &str) -> DetectedHostSubstrate {
    let mut id = None;
    let mut version = None;
    for line in contents.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim_matches('"').to_owned();
        match key {
            "ID" => id = Some(value),
            "VERSION_ID" => version = Some(value),
            _ => {}
        }
    }
    let kind = match id.as_deref() {
        Some("nixos") => HostSubstrateKind::NixOs,
        Some("ubuntu") => HostSubstrateKind::Ubuntu,
        _ => HostSubstrateKind::GenericLinux,
    };
    DetectedHostSubstrate { kind, version }
}

/// DisplayProvider adapter for the local cross-domain Wayland proxy path.
#[derive(Clone)]
pub struct LocalCrossDomainWaylandProvider {
    provider_id: ProviderId,
    argv_input: WaylandProxyArgvInput,
}

impl std::fmt::Debug for LocalCrossDomainWaylandProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalCrossDomainWaylandProvider")
            .field("provider_id", &self.provider_id)
            .finish_non_exhaustive()
    }
}

impl LocalCrossDomainWaylandProvider {
    /// Construct a local cross-domain Wayland provider using the canonical id.
    pub fn new(argv_input: WaylandProxyArgvInput) -> Self {
        Self::with_provider_id_and_dmabuf(
            static_provider_id(LOCAL_CROSS_DOMAIN_WAYLAND_PROVIDER_ID),
            argv_input,
            true,
        )
    }

    /// Construct a local cross-domain Wayland provider with an explicit id.
    pub fn with_provider_id(provider_id: ProviderId, argv_input: WaylandProxyArgvInput) -> Self {
        Self::with_provider_id_and_dmabuf(provider_id, argv_input, true)
    }

    /// Construct a provider with an explicit dmabuf preference.
    pub fn with_provider_id_and_dmabuf(
        provider_id: ProviderId,
        argv_input: WaylandProxyArgvInput,
        _dmabuf: bool,
    ) -> Self {
        Self {
            provider_id,
            argv_input,
        }
    }

    /// Borrow the Wayland proxy argv generator input this adapter wraps.
    pub fn argv_input(&self) -> &WaylandProxyArgvInput {
        &self.argv_input
    }

    /// Generate Wayland proxy argv through the existing d2b-host generator.
    pub async fn wayland_proxy_argv(&self) -> ProviderResult<Vec<String>> {
        render_wayland_proxy_argv(self.argv_input.clone()).await
    }

    fn ensure_workload_matches(&self, req: &DisplaySessionRequest) -> ProviderResult<()> {
        if req.workload.as_str() == self.argv_input.vm_name.as_str() {
            Ok(())
        } else {
            Err(ProviderError::new(
                ErrorKind::InvalidTarget,
                "workload alias does not match the local Wayland proxy argv input",
            ))
        }
    }
}

#[async_trait]
impl DisplayProvider for LocalCrossDomainWaylandProvider {
    fn provider_id(&self) -> ProviderId {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> DisplayCapabilitySet {
        DisplayCapabilitySet {
            caps: CapabilitySet::from_caps([]),
            shm_buffers: false,
            dmabuf: false,
            reconnect: false,
        }
    }

    async fn open_display_session(
        &self,
        req: DisplaySessionRequest,
    ) -> ProviderResult<DisplaySessionHandle> {
        self.ensure_workload_matches(&req)?;
        self.wayland_proxy_argv().await?;
        Err(ProviderError::unsupported(
            "local cross-domain Wayland session opening is not wired in this provider adapter",
        ))
    }

    async fn close_display_session(&self, _id: DisplaySessionId) -> ProviderResult<()> {
        Err(ProviderError::unsupported(
            "local cross-domain Wayland session closing is not wired in this provider adapter",
        ))
    }
}

async fn render_wayland_proxy_argv(input: WaylandProxyArgvInput) -> ProviderResult<Vec<String>> {
    tokio::task::spawn_blocking(move || generate_wayland_proxy_argv(&input))
        .await
        .map_err(|_| {
            ProviderError::new(
                ErrorKind::ProviderAllocationFailed,
                "blocking task failed while rendering Wayland proxy argv",
            )
        })?
        .map_err(wayland_proxy_argv_error)
}

async fn render_qemu_media_argv(input: QemuMediaArgvInput) -> ProviderResult<Vec<String>> {
    tokio::task::spawn_blocking(move || generate_qemu_media_argv(&input))
        .await
        .map_err(|_| {
            ProviderError::new(
                ErrorKind::ProviderAllocationFailed,
                "blocking task failed while rendering qemu-media argv",
            )
        })?
        .map_err(qemu_media_argv_error)
}

fn wayland_proxy_argv_error(err: WaylandProxyArgvError) -> ProviderError {
    let message = match err {
        WaylandProxyArgvError::EmptyVmName => "invalid Wayland proxy argv input: empty VM name",
        WaylandProxyArgvError::RelativeSocketPath { .. } => {
            "invalid Wayland proxy argv input: relative socket path"
        }
    };
    ProviderError::new(ErrorKind::ProviderAllocationFailed, message)
}

fn qemu_media_argv_error(err: QemuMediaArgvError) -> ProviderError {
    let message = match err {
        QemuMediaArgvError::InvalidQemuBinaryPath { .. } => {
            "invalid qemu-media argv input: invalid QEMU binary path"
        }
        QemuMediaArgvError::EmptyVmName => "invalid qemu-media argv input: empty VM name",
        QemuMediaArgvError::InvalidQmpSocketPath { .. } => {
            "invalid qemu-media argv input: invalid QMP socket path"
        }
        QemuMediaArgvError::InvalidMacAddress { .. } => {
            "invalid qemu-media argv input: invalid MAC address"
        }
        QemuMediaArgvError::InvalidTapFd { .. } => "invalid qemu-media argv input: invalid TAP fd",
        QemuMediaArgvError::InvalidMemoryMiB { .. } => {
            "invalid qemu-media argv input: invalid memory size"
        }
        QemuMediaArgvError::InvalidVcpu { .. } => {
            "invalid qemu-media argv input: invalid vCPU count"
        }
        QemuMediaArgvError::InvalidConsoleFd { .. } => {
            "invalid qemu-media argv input: invalid console fd"
        }
        QemuMediaArgvError::ConsoleFdConflictsWithTapFd { .. } => {
            "invalid qemu-media argv input: console fd conflicts with TAP fd"
        }
    };
    ProviderError::new(ErrorKind::ProviderAllocationFailed, message)
}

fn static_provider_id(id: &'static str) -> ProviderId {
    ProviderId::parse(id).expect("static provider id must use the provider-id label shape")
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_core::host_check::{
        HostCheckFinding, HostCheckReport, HostCheckSeverity, HostCheckSummary,
    };
    use d2b_host::{
        ch_argv::{ChFsShare, ChNetHandoff, ChNetIface, ChVsock, generate_ch_argv},
        qemu_media_argv::generate_qemu_media_argv,
        wayland_proxy_argv::generate_wayland_proxy_argv,
    };
    use d2b_realm_core::{ErrorKind, WorkloadId};
    use d2b_realm_provider::RuntimeProvider;
    use d2b_realm_provider::types::{DisplaySessionRequest, WorkloadSpec};

    fn workload_id(raw: &str) -> WorkloadId {
        WorkloadId::parse(raw).expect("test workload id must be valid")
    }

    fn host_fixture() -> HostJson {
        serde_json::from_str(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host-valid fixture must deserialize")
    }

    fn report_with_failures(fail: u32) -> HostCheckReport {
        HostCheckReport {
            strict: true,
            summary: HostCheckSummary {
                pass: 1,
                warn: 0,
                fail,
            },
            findings: if fail == 0 {
                vec![HostCheckFinding {
                    id: "kernel-version".to_owned(),
                    severity: HostCheckSeverity::Pass,
                    message: "kernel ok".to_owned(),
                    remediation: "No action required.".to_owned(),
                    vm: None,
                    detail: None,
                    details: Default::default(),
                }]
            } else {
                vec![HostCheckFinding {
                    id: "cgroup-v2".to_owned(),
                    severity: HostCheckSeverity::Fail,
                    message: "missing cgroup v2".to_owned(),
                    remediation: "Boot with unified cgroup-v2 enabled.".to_owned(),
                    vm: None,
                    detail: None,
                    details: Default::default(),
                }]
            },
        }
    }

    fn representative_ch_input() -> ChArgvInput {
        ChArgvInput {
            vm_name: "corp-vm".to_owned(),
            ch_binary_path:
                "/nix/store/5dp5ya1q03ab3indxnd7x3pwixifw5rn-cloud-hypervisor-52.0/bin/cloud-hypervisor"
                    .to_owned(),
            cpus: 1,
            watchdog: true,
            kernel_path:
                "/nix/store/6p1aazl39927kp22ajw4h8bqa6j5g4vz-linux-6.18.31-dev/vmlinux"
                    .to_owned(),
            initramfs_path: Some(
                "/nix/store/qdrg2rycwnqw7b5m69v12pizvf3p19yr-initrd-linux-6.18.31/initrd"
                    .to_owned(),
            ),
            cmdline:
                "earlyprintk=ttyS0 console=ttyS0 reboot=t panic=-1 8250.nr_uarts=1 \
                 root=fstab loglevel=4 lsm=landlock,yama,bpf \
                 init=/nix/store/5ycspc2h3zhl9qiq2axsc1hvirr5pm02-nixos-system-corp-vm-26.05pre-git/init \
                 regInfo=/nix/store/ldfmwp9xh6av69d5bvz7j898m6kqlgzm-closure-info/registration"
                    .to_owned(),
            seccomp: "true".to_owned(),
            memory: "shared=on,size=512M".to_owned(),
            platform_oem_strings: vec![
                "io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888".to_owned(),
            ],
            console: "null".to_owned(),
            serial: "tty".to_owned(),
            primary_vsock: Some(ChVsock {
                cid: 10_914_385,
                socket: "notify.vsock".to_owned(),
            }),
            extra_vsock: Vec::new(),
            fs_shares: vec![
                ChFsShare {
                    socket: "corp-vm-virtiofs-ro-store.sock".to_owned(),
                    tag: "ro-store".to_owned(),
                },
                ChFsShare {
                    socket: "corp-vm-virtiofs-d2b-meta.sock".to_owned(),
                    tag: "d2b-meta".to_owned(),
                },
                ChFsShare {
                    socket: "corp-vm-virtiofs-d2b-hkeys.sock".to_owned(),
                    tag: "d2b-hkeys".to_owned(),
                },
                ChFsShare {
                    socket: "corp-vm-virtiofs-d2b-ssh-host.sock".to_owned(),
                    tag: "d2b-ssh-host".to_owned(),
                },
            ],
            api_socket_path: "corp-vm.sock".to_owned(),
            net_ifaces: vec![ChNetIface {
                mac: "02:76:53:AE:57:0A".to_owned(),
                tap_ifname: "work-l10".to_owned(),
                tap_fd: None,
            }],
            net_handoff: ChNetHandoff::PersistentTap,
            extra_args: Vec::new(),
        }
    }

    fn expected_ch_argv() -> Vec<String> {
        [
            "/nix/store/5dp5ya1q03ab3indxnd7x3pwixifw5rn-cloud-hypervisor-52.0/bin/cloud-hypervisor",
            "--cpus",
            "boot=1",
            "--watchdog",
            "--kernel",
            "/nix/store/6p1aazl39927kp22ajw4h8bqa6j5g4vz-linux-6.18.31-dev/vmlinux",
            "--initramfs",
            "/nix/store/qdrg2rycwnqw7b5m69v12pizvf3p19yr-initrd-linux-6.18.31/initrd",
            "--cmdline",
            "earlyprintk=ttyS0 console=ttyS0 reboot=t panic=-1 8250.nr_uarts=1 root=fstab loglevel=4 lsm=landlock,yama,bpf init=/nix/store/5ycspc2h3zhl9qiq2axsc1hvirr5pm02-nixos-system-corp-vm-26.05pre-git/init regInfo=/nix/store/ldfmwp9xh6av69d5bvz7j898m6kqlgzm-closure-info/registration",
            "--seccomp",
            "true",
            "--memory",
            "shared=on,size=512M",
            "--platform",
            "oem_strings=[io.systemd.credential:vmm.notify_socket=vsock-stream:2:8888]",
            "--console",
            "null",
            "--serial",
            "tty",
            "--vsock",
            "cid=10914385,socket=notify.vsock",
            "--fs",
            "socket=corp-vm-virtiofs-ro-store.sock,tag=ro-store",
            "socket=corp-vm-virtiofs-d2b-meta.sock,tag=d2b-meta",
            "socket=corp-vm-virtiofs-d2b-hkeys.sock,tag=d2b-hkeys",
            "socket=corp-vm-virtiofs-d2b-ssh-host.sock,tag=d2b-ssh-host",
            "--api-socket",
            "corp-vm.sock",
            "--net",
            "mac=02:76:53:AE:57:0A,tap=work-l10",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    fn representative_wayland_input() -> WaylandProxyArgvInput {
        WaylandProxyArgvInput::for_vm("corp-vm")
    }

    fn representative_qemu_media_input() -> QemuMediaArgvInput {
        QemuMediaArgvInput {
            qemu_binary_path: "/nix/store/QEMUQEMUQEMU-qemu/bin/qemu-system-x86_64".to_owned(),
            vm_name: "media-vm".to_owned(),
            qmp_socket_path: "/run/d2b/vms/media-vm/qmp.sock".to_owned(),
            mac_address: "02:76:53:AE:57:2A".to_owned(),
            tap_fd: 10,
            memory_mib: 4096,
            vcpu: 2,
            lock_memory: false,
            exclude_memory_from_core_dump: true,
            disable_memory_merge: true,
            console_fd: Some(11),
        }
    }

    fn expected_wayland_argv() -> Vec<String> {
        [
            "d2b-corp-vm-wlproxy",
            "--listen",
            "/run/d2b-wlproxy/corp-vm/wayland-0",
            "--connect",
            "/run/d2b-wlproxy/corp-vm/upstream",
            "--vm-name",
            "corp-vm",
            "--app-id-prefix",
            "d2b.corp-vm.",
            "--title-prefix",
            "[corp-vm] ",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn local_microvm_adapter_matches_ch_generator_byte_for_byte() {
        let input = representative_ch_input();
        let direct = generate_ch_argv(&input).expect("representative CH input is valid");
        let adapter = LocalMicroVmProvider::new(input);
        let from_adapter = adapter
            .cloud_hypervisor_argv()
            .expect("adapter delegates to CH generator");

        assert_eq!(direct, expected_ch_argv());
        assert_eq!(from_adapter, direct);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn local_wayland_adapter_matches_generator_byte_for_byte() {
        let input = representative_wayland_input();
        let direct =
            generate_wayland_proxy_argv(&input).expect("representative Wayland input is valid");
        let adapter = LocalCrossDomainWaylandProvider::new(input);
        let from_adapter = adapter
            .wayland_proxy_argv()
            .await
            .expect("adapter delegates to Wayland proxy generator");

        assert_eq!(direct, expected_wayland_argv());
        assert_eq!(from_adapter, direct);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn local_qemu_media_adapter_matches_generator_byte_for_byte() {
        let input = representative_qemu_media_input();
        let direct = generate_qemu_media_argv(&input).expect("representative qemu input is valid");
        let adapter = LocalQemuMediaRuntimeProvider::new(input);
        let from_adapter = adapter
            .qemu_media_argv()
            .await
            .expect("adapter delegates to qemu-media generator");

        assert_eq!(from_adapter, direct);
    }

    #[test]
    fn local_provider_debug_redacts_argv_inputs() {
        let mut ch_input = representative_ch_input();
        ch_input.ch_binary_path = "/nix/store/debug-redact-ch-bin/bin/cloud-hypervisor".to_owned();
        ch_input.kernel_path = "/nix/store/debug-redact-kernel/vmlinux".to_owned();
        ch_input.cmdline = "debug-redact-cmdline init=/nix/store/debug-redact-init".to_owned();
        ch_input.api_socket_path = "debug-redact-api.sock".to_owned();
        ch_input.extra_args = vec![
            "--debug-redact-extra-flag".to_owned(),
            "/run/d2b/debug-redact-extra-path".to_owned(),
        ];
        ch_input.fs_shares.push(ChFsShare {
            socket: "debug-redact-virtiofs.sock".to_owned(),
            tag: "debug-redact-tag".to_owned(),
        });

        let runtime_debug = format!("{:?}", LocalMicroVmProvider::new(ch_input));
        assert!(runtime_debug.contains("CloudHypervisorRuntimeProvider"));
        assert!(runtime_debug.contains(CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID));
        for forbidden in [
            "debug-redact-ch-bin",
            "debug-redact-kernel",
            "debug-redact-cmdline",
            "debug-redact-init",
            "debug-redact-api.sock",
            "debug-redact-extra-flag",
            "debug-redact-extra-path",
            "debug-redact-virtiofs.sock",
            "debug-redact-tag",
        ] {
            assert!(
                !runtime_debug.contains(forbidden),
                "runtime provider Debug leaked {forbidden}: {runtime_debug}"
            );
        }

        let wayland_debug = format!(
            "{:?}",
            LocalCrossDomainWaylandProvider::new(WaylandProxyArgvInput::for_vm(
                "debug-redact-wayland"
            ))
        );
        assert!(wayland_debug.contains("LocalCrossDomainWaylandProvider"));
        assert!(wayland_debug.contains(LOCAL_CROSS_DOMAIN_WAYLAND_PROVIDER_ID));
        for forbidden in [
            "debug-redact-wayland",
            "/run/d2b-wlproxy/debug-redact-wayland",
            "d2b-debug-redact-wayland-wlproxy",
        ] {
            assert!(
                !wayland_debug.contains(forbidden),
                "Wayland provider Debug leaked {forbidden}: {wayland_debug}"
            );
        }

        let mut qemu_input = representative_qemu_media_input();
        qemu_input.qemu_binary_path =
            "/nix/store/debug-redact-qemu/bin/qemu-system-x86_64".to_owned();
        qemu_input.qmp_socket_path = "/run/d2b/vms/debug-redact-media/qmp.sock".to_owned();
        qemu_input.vm_name = "debug-redact-media".to_owned();

        let qemu_debug = format!("{:?}", LocalQemuMediaRuntimeProvider::new(qemu_input));
        assert!(qemu_debug.contains("LocalQemuMediaRuntimeProvider"));
        assert!(qemu_debug.contains(LOCAL_QEMU_MEDIA_RUNTIME_PROVIDER_ID));
        for forbidden in [
            "debug-redact-qemu",
            "/run/d2b/vms/debug-redact-media/qmp.sock",
            "debug-redact-media",
        ] {
            assert!(
                !qemu_debug.contains(forbidden),
                "qemu-media provider Debug leaked {forbidden}: {qemu_debug}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn async_trait_methods_complete_on_current_thread_runtime() {
        let runtime_provider = LocalMicroVmProvider::new(representative_ch_input());
        let plan = RuntimeProvider::plan_workload(
            &runtime_provider,
            WorkloadSpec {
                alias: workload_id("corp-vm"),
            },
        )
        .await
        .expect("planning delegates through spawn_blocking");
        assert_eq!(plan.provider, runtime_provider.provider_id());
        assert_eq!(plan.workload.as_str(), "corp-vm");

        let start_err = RuntimeProvider::start(&runtime_provider, plan)
            .await
            .expect_err("start is intentionally not wired");
        assert_eq!(start_err.kind(), ErrorKind::UnsupportedFeature);
        assert!(!runtime_provider.capabilities().has(Capability::Lifecycle));

        let display_provider = LocalCrossDomainWaylandProvider::new(representative_wayland_input());
        let display_err = DisplayProvider::open_display_session(
            &display_provider,
            DisplaySessionRequest {
                workload: workload_id("corp-vm"),
                operation_id: d2b_realm_core::OperationId::parse("op-display-1").unwrap(),
                display_stream: d2b_realm_core::StreamId::parse("disp-1").unwrap(),
                authz: d2b_realm_core::StreamAuthz::for_kind(
                    d2b_realm_core::PrincipalId::parse("principal-1").unwrap(),
                    d2b_realm_core::RealmPath::local(),
                    d2b_realm_core::StreamKind::Display,
                ),
            },
        )
        .await
        .expect_err("display opening is intentionally not wired");
        assert_eq!(display_err.kind(), ErrorKind::UnsupportedFeature);
        assert!(
            !display_provider
                .capabilities()
                .has(Capability::WindowForwarding)
        );

        let qemu_provider = LocalQemuMediaRuntimeProvider::new(representative_qemu_media_input());
        let qemu_plan = RuntimeProvider::plan_workload(
            &qemu_provider,
            WorkloadSpec {
                alias: workload_id("media-vm"),
            },
        )
        .await
        .expect("qemu-media planning validates argv through spawn_blocking");
        assert_eq!(qemu_plan.provider, qemu_provider.provider_id());
        assert_eq!(qemu_plan.workload.as_str(), "media-vm");

        let qemu_start_err = RuntimeProvider::start(&qemu_provider, qemu_plan)
            .await
            .expect_err("qemu-media start is intentionally not wired");
        assert_eq!(qemu_start_err.kind(), ErrorKind::UnsupportedFeature);
        assert!(!qemu_provider.capabilities().has(Capability::Lifecycle));
    }

    #[test]
    fn local_capabilities_advertise_current_boundaries() {
        let runtime = LocalMicroVmProvider::new(representative_ch_input()).capabilities();
        assert!(!runtime.has(Capability::Lifecycle));
        assert!(runtime.has(Capability::Vsock));
        assert!(runtime.has(Capability::Virtiofs));

        let display =
            LocalCrossDomainWaylandProvider::new(representative_wayland_input()).capabilities();
        assert!(!display.has(Capability::WindowForwarding));
        assert!(!display.has(Capability::Clipboard));
        assert!(!display.shm_buffers);
        assert!(!display.dmabuf);
        assert!(!display.reconnect);

        let qemu =
            LocalQemuMediaRuntimeProvider::new(representative_qemu_media_input()).capabilities();
        assert!(!qemu.has(Capability::Lifecycle));
        assert!(!qemu.has(Capability::Vsock));
        assert!(!qemu.has(Capability::Virtiofs));
        assert!(!qemu.has(Capability::WindowForwarding));
        assert!(!qemu.has(Capability::Usb));
    }

    #[test]
    fn host_substrate_provider_ids_and_debug_are_bounded() {
        let provider = HostCheckSubstrateProvider::nixos(host_fixture()).with_strict(false);
        assert_eq!(
            provider.provider_id().as_str(),
            NIXOS_HOST_SUBSTRATE_PROVIDER_ID
        );
        let rendered = format!("{provider:?}");
        assert!(rendered.contains("HostCheckSubstrateProvider"));
        assert!(rendered.contains(NIXOS_HOST_SUBSTRATE_PROVIDER_ID));
        assert!(!rendered.contains("nftables"));
        assert!(!rendered.contains("networkManager"));

        let generic = HostCheckSubstrateProvider::generic_linux(host_fixture());
        assert_eq!(
            generic.provider_id().as_str(),
            GENERIC_LINUX_HOST_SUBSTRATE_PROVIDER_ID
        );
    }

    #[test]
    fn host_substrate_capabilities_require_zero_failures() {
        let provider = HostCheckSubstrateProvider::nixos(host_fixture());
        let caps = provider.host_check_capabilities(&report_with_failures(0));
        assert!(caps.has(Capability::Lifecycle));
        assert!(caps.has(Capability::Vsock));
        assert!(caps.has(Capability::Virtiofs));
        assert_eq!(caps.substrate, Some(HostSubstrateKind::NixOs));
        assert!(caps.userns_available);
        assert!(caps.vhost_acceleration);

        let caps = provider.host_check_capabilities(&report_with_failures(1));
        assert!(!caps.has(Capability::Lifecycle));
        assert!(!caps.has(Capability::Vsock));
        assert!(!caps.has(Capability::Virtiofs));
        assert_eq!(caps.substrate, Some(HostSubstrateKind::NixOs));
        assert!(!caps.userns_available);
        assert!(!caps.vhost_acceleration);
    }

    #[test]
    fn host_substrate_failure_mentions_id_and_remediation() {
        let err = host_check_failed(report_with_failures(1));
        assert_eq!(err.kind(), ErrorKind::ProviderAllocationFailed);
        let rendered = err.to_string();
        assert!(rendered.contains("cgroup-v2"));
        assert!(rendered.contains("unified cgroup-v2"));
    }

    #[test]
    fn os_release_detection_classifies_nixos_ubuntu_and_generic() {
        assert_eq!(
            detect_os_release("ID=nixos\nVERSION_ID=\"26.05\"\n"),
            DetectedHostSubstrate {
                kind: HostSubstrateKind::NixOs,
                version: Some("26.05".to_owned())
            }
        );
        assert_eq!(
            detect_os_release("ID=ubuntu\nVERSION_ID=\"24.04\"\n"),
            DetectedHostSubstrate {
                kind: HostSubstrateKind::Ubuntu,
                version: Some("24.04".to_owned())
            }
        );
        assert_eq!(
            detect_os_release("ID=debian\n"),
            DetectedHostSubstrate {
                kind: HostSubstrateKind::GenericLinux,
                version: None
            }
        );
    }

    #[test]
    fn generic_linux_provider_uses_os_release_metadata() {
        let provider = HostCheckSubstrateProvider::generic_linux_from_os_release(
            host_fixture(),
            "ID=ubuntu\nVERSION_ID=\"24.04\"\n",
        );
        let caps = provider.host_check_capabilities(&report_with_failures(0));
        assert_eq!(caps.substrate, Some(HostSubstrateKind::Ubuntu));
        assert_eq!(caps.substrate_version.as_deref(), Some("24.04"));
    }
}
