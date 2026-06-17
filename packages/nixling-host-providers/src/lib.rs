//! Local host provider adapters for ADR 0032.
//!
//! The adapters in this crate are intentionally thin: they expose the
//! existing `nixling-host` argv generators through the provider trait
//! surface without spawning or changing runtime behavior.

use async_trait::async_trait;
use nixling_constellation_core::{Capability, CapabilitySet, ErrorKind, ProviderId};
use nixling_constellation_provider::{
    capabilities::{DisplayCapabilitySet, RuntimeCapabilitySet},
    error::{ProviderError, ProviderResult},
    types::{
        DisplaySessionHandle, DisplaySessionId, DisplaySessionRequest, RuntimeHandle, RuntimePlan,
        RuntimeStatus, WorkloadSpec,
    },
    DisplayProvider, RuntimeProvider,
};
use nixling_host::{
    ch_argv::{generate_ch_argv, ChArgvError},
    wayland_proxy_argv::{generate_wayland_proxy_argv, WaylandProxyArgvError},
};

pub use nixling_host::{ch_argv::ChArgvInput, wayland_proxy_argv::WaylandProxyArgvInput};

const LOCAL_MICROVM_PROVIDER_ID: &str = "local-microvm";
const LOCAL_CROSS_DOMAIN_WAYLAND_PROVIDER_ID: &str = "local-cross-domain-wayland";

/// RuntimeProvider adapter for the local Cloud Hypervisor microVM path.
#[derive(Debug, Clone)]
pub struct LocalMicroVmProvider {
    provider_id: ProviderId,
    ch_input: ChArgvInput,
}

impl LocalMicroVmProvider {
    /// Construct a local microVM provider using the canonical provider id.
    pub fn new(ch_input: ChArgvInput) -> Self {
        Self::with_provider_id(static_provider_id(LOCAL_MICROVM_PROVIDER_ID), ch_input)
    }

    /// Construct a local microVM provider with an explicit provider id.
    pub fn with_provider_id(provider_id: ProviderId, ch_input: ChArgvInput) -> Self {
        Self {
            provider_id,
            ch_input,
        }
    }

    /// Borrow the Cloud Hypervisor argv generator input this adapter wraps.
    pub fn ch_input(&self) -> &ChArgvInput {
        &self.ch_input
    }

    /// Generate Cloud Hypervisor argv through the existing nixling-host generator.
    pub async fn cloud_hypervisor_argv(&self) -> ProviderResult<Vec<String>> {
        render_ch_argv(self.ch_input.clone()).await
    }

    fn ensure_workload_matches(&self, spec: &WorkloadSpec) -> ProviderResult<()> {
        if spec.alias.as_str() == self.ch_input.vm_name.as_str() {
            Ok(())
        } else {
            Err(ProviderError::new(
                ErrorKind::InvalidTarget,
                "workload alias does not match the local microVM argv input",
            ))
        }
    }
}

#[async_trait]
impl RuntimeProvider for LocalMicroVmProvider {
    fn provider_id(&self) -> ProviderId {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> RuntimeCapabilitySet {
        RuntimeCapabilitySet {
            caps: CapabilitySet::from_caps([
                Capability::Lifecycle,
                Capability::Vsock,
                Capability::Virtiofs,
            ]),
        }
    }

    async fn plan_workload(&self, spec: WorkloadSpec) -> ProviderResult<RuntimePlan> {
        self.ensure_workload_matches(&spec)?;
        self.cloud_hypervisor_argv().await?;
        Ok(RuntimePlan {
            provider: self.provider_id(),
            workload: spec.alias,
        })
    }

    async fn start(&self, _plan: RuntimePlan) -> ProviderResult<RuntimeHandle> {
        Err(ProviderError::unsupported(
            "local microVM start is not wired in this provider adapter",
        ))
    }

    async fn stop(&self, _handle: RuntimeHandle) -> ProviderResult<()> {
        Err(ProviderError::unsupported(
            "local microVM stop is not wired in this provider adapter",
        ))
    }

    async fn inspect(&self, _handle: RuntimeHandle) -> ProviderResult<RuntimeStatus> {
        Err(ProviderError::unsupported(
            "local microVM inspect is not wired in this provider adapter",
        ))
    }
}

/// DisplayProvider adapter for the local cross-domain Wayland proxy path.
#[derive(Debug, Clone)]
pub struct LocalCrossDomainWaylandProvider {
    provider_id: ProviderId,
    argv_input: WaylandProxyArgvInput,
    dmabuf: bool,
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

    /// Construct a provider with an explicit dmabuf advertisement.
    pub fn with_provider_id_and_dmabuf(
        provider_id: ProviderId,
        argv_input: WaylandProxyArgvInput,
        dmabuf: bool,
    ) -> Self {
        Self {
            provider_id,
            argv_input,
            dmabuf,
        }
    }

    /// Borrow the Wayland proxy argv generator input this adapter wraps.
    pub fn argv_input(&self) -> &WaylandProxyArgvInput {
        &self.argv_input
    }

    /// Generate Wayland proxy argv through the existing nixling-host generator.
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
            caps: CapabilitySet::from_caps([Capability::WindowForwarding]),
            shm_buffers: true,
            dmabuf: self.dmabuf,
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

async fn render_ch_argv(input: ChArgvInput) -> ProviderResult<Vec<String>> {
    tokio::task::spawn_blocking(move || generate_ch_argv(&input))
        .await
        .map_err(|_| {
            ProviderError::new(
                ErrorKind::ProviderAllocationFailed,
                "blocking task failed while rendering Cloud Hypervisor argv",
            )
        })?
        .map_err(ch_argv_error)
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

fn ch_argv_error(err: ChArgvError) -> ProviderError {
    let message = match err {
        ChArgvError::EmptyVmName => "invalid Cloud Hypervisor argv input: empty VM name",
        ChArgvError::InvalidChBinaryPath { .. } => {
            "invalid Cloud Hypervisor argv input: invalid binary path"
        }
        ChArgvError::ZeroCpus => "invalid Cloud Hypervisor argv input: zero CPUs",
        ChArgvError::EmptyKernelPath => "invalid Cloud Hypervisor argv input: empty kernel path",
        ChArgvError::TapFdMissing { .. } => {
            "invalid Cloud Hypervisor argv input: missing TAP file descriptor"
        }
        ChArgvError::TapIfnameMissing { .. } => {
            "invalid Cloud Hypervisor argv input: missing TAP interface name"
        }
    };
    ProviderError::new(ErrorKind::ProviderAllocationFailed, message)
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

fn static_provider_id(id: &'static str) -> ProviderId {
    ProviderId::parse(id).expect("static provider id must use the provider-id label shape")
}

#[cfg(test)]
mod tests {
    use super::*;
    use nixling_constellation_core::{ErrorKind, WorkloadId};
    use nixling_constellation_provider::types::{DisplaySessionRequest, WorkloadSpec};
    use nixling_host::{
        ch_argv::{generate_ch_argv, ChFsShare, ChNetHandoff, ChNetIface, ChVsock},
        wayland_proxy_argv::generate_wayland_proxy_argv,
    };

    fn workload_id(raw: &str) -> WorkloadId {
        WorkloadId::parse(raw).expect("test workload id must be valid")
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
                    socket: "corp-vm-virtiofs-nl-meta.sock".to_owned(),
                    tag: "nl-meta".to_owned(),
                },
                ChFsShare {
                    socket: "corp-vm-virtiofs-nl-hkeys.sock".to_owned(),
                    tag: "nl-hkeys".to_owned(),
                },
                ChFsShare {
                    socket: "corp-vm-virtiofs-nl-ssh-host.sock".to_owned(),
                    tag: "nl-ssh-host".to_owned(),
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
            "socket=corp-vm-virtiofs-nl-meta.sock,tag=nl-meta",
            "socket=corp-vm-virtiofs-nl-hkeys.sock,tag=nl-hkeys",
            "socket=corp-vm-virtiofs-nl-ssh-host.sock,tag=nl-ssh-host",
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

    fn expected_wayland_argv() -> Vec<String> {
        [
            "nixling-corp-vm-wlproxy",
            "--listen",
            "/run/nixling-wlproxy/corp-vm/wayland-0",
            "--connect",
            "/run/nixling-wlproxy/corp-vm/upstream",
            "--vm-name",
            "corp-vm",
            "--app-id-prefix",
            "nixling.corp-vm.",
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
            .await
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

        let display_provider = LocalCrossDomainWaylandProvider::new(representative_wayland_input());
        let display_err = DisplayProvider::open_display_session(
            &display_provider,
            DisplaySessionRequest {
                workload: workload_id("corp-vm"),
            },
        )
        .await
        .expect_err("display opening is intentionally not wired");
        assert_eq!(display_err.kind(), ErrorKind::UnsupportedFeature);
    }

    #[test]
    fn local_capabilities_advertise_current_boundaries() {
        let runtime = LocalMicroVmProvider::new(representative_ch_input()).capabilities();
        assert!(runtime.has(Capability::Lifecycle));
        assert!(runtime.has(Capability::Vsock));
        assert!(runtime.has(Capability::Virtiofs));

        let display =
            LocalCrossDomainWaylandProvider::new(representative_wayland_input()).capabilities();
        assert!(display.has(Capability::WindowForwarding));
        assert!(!display.has(Capability::Clipboard));
        assert!(display.shm_buffers);
        assert!(display.dmabuf);
        assert!(!display.reconnect);
    }
}
