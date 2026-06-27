//! Host-side runtime-provider adapters.
//!
//! Runtime plans are intentionally opaque provider DTOs. The Cloud
//! Hypervisor adapter validates the existing typed argv input through
//! [`crate::ch_argv::generate_ch_argv`] but never stores argv, host paths,
//! file descriptors, pidfds, cgroup paths, namespace identifiers, or socket
//! paths in [`RuntimePlan`], errors, or debug output.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_constellation_core::{Capability, CapabilitySet, ErrorKind, ProviderId};
use d2b_constellation_provider::{
    RuntimeProvider,
    capabilities::RuntimeCapabilitySet,
    error::{ProviderError, ProviderResult},
    types::{RuntimeHandle, RuntimePlan, RuntimeStatus, WorkloadSpec},
};

use crate::ch_argv::{ChArgvError, ChArgvInput, generate_ch_argv};

/// Default local runtime provider id.
pub const CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID: &str = "local-cloud-hypervisor";
/// Reserved crosvm runtime provider id. Not implemented as a full VM runtime.
pub const CROSVM_RUNTIME_PROVIDER_ID: &str = "crosvm";
/// Reserved QEMU runtime provider id. Not implemented as a full VM runtime.
pub const QEMU_RUNTIME_PROVIDER_ID: &str = "qemu";
/// Reserved Firecracker runtime provider id. Not implemented for desktop d2b workloads.
pub const FIRECRACKER_RUNTIME_PROVIDER_ID: &str = "firecracker";
/// QEMU media remains a sidecar/media runtime, not the full VM runtime.
pub const QEMU_MEDIA_RUNTIME_PROVIDER_ID: &str = "qemu-media";

/// Feature requirements for selecting a runtime profile. This is local policy
/// input, not a serialized provider DTO.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeWorkloadRequirements {
    pub desktop: bool,
    pub graphics: bool,
    pub audio: bool,
    pub usb: bool,
    pub virtiofs: bool,
    pub store_sync: bool,
    pub guest_control: bool,
}

impl RuntimeWorkloadRequirements {
    fn firecracker_incompatible(self) -> Option<Capability> {
        if self.guest_control {
            Some(Capability::Vsock)
        } else if self.virtiofs || self.store_sync {
            Some(Capability::Virtiofs)
        } else if self.graphics || self.desktop {
            Some(Capability::GpuAccel)
        } else if self.audio {
            Some(Capability::AudioPlayback)
        } else if self.usb {
            Some(Capability::Usb)
        } else {
            None
        }
    }
}

/// Validate that `profile` can satisfy `requirements` without falling back.
pub fn validate_runtime_profile(
    provider_id: &ProviderId,
    requirements: RuntimeWorkloadRequirements,
) -> ProviderResult<()> {
    match provider_id.as_str() {
        CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID => Ok(()),
        FIRECRACKER_RUNTIME_PROVIDER_ID => {
            if let Some(capability) = requirements.firecracker_incompatible() {
                Err(ProviderError::unsupported(format!(
                    "runtime provider '{FIRECRACKER_RUNTIME_PROVIDER_ID}' does not support required capability '{}'; drop the incompatible feature or use {CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID}",
                    capability.code()
                )))
            } else {
                Err(ProviderError::unsupported(format!(
                    "runtime provider '{FIRECRACKER_RUNTIME_PROVIDER_ID}' is not implemented; use {CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID}"
                )))
            }
        }
        QEMU_MEDIA_RUNTIME_PROVIDER_ID => Err(ProviderError::unsupported(format!(
            "runtime provider '{QEMU_MEDIA_RUNTIME_PROVIDER_ID}' is a media sidecar, not a full VM runtime; use {CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID}"
        ))),
        CROSVM_RUNTIME_PROVIDER_ID | QEMU_RUNTIME_PROVIDER_ID => {
            Err(ProviderError::unsupported(format!(
                "runtime provider '{}' is not implemented; use {CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID}",
                provider_id
            )))
        }
        other => Err(ProviderError::unsupported(format!(
            "runtime provider '{other}' is not supported; use {CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID}"
        ))),
    }
}

/// Daemon/broker runtime authority used by [`CloudHypervisorRuntimeProvider`].
/// Implementations own runner spawning, pidfds, and inspection. The provider
/// passes typed host-local input to this seam; none of it is serialized into
/// [`RuntimePlan`].
#[async_trait]
pub trait CloudHypervisorRuntimeControl: Send + Sync {
    async fn start(&self, plan: RuntimePlan, input: &ChArgvInput) -> ProviderResult<RuntimeHandle>;
    async fn stop(&self, handle: RuntimeHandle) -> ProviderResult<()>;
    async fn inspect(&self, handle: RuntimeHandle) -> ProviderResult<RuntimeStatus>;
}

/// Runtime provider for the existing Cloud Hypervisor microVM path.
#[derive(Clone)]
pub struct CloudHypervisorRuntimeProvider {
    provider_id: ProviderId,
    ch_input: ChArgvInput,
    control: Option<Arc<dyn CloudHypervisorRuntimeControl>>,
}

impl std::fmt::Debug for CloudHypervisorRuntimeProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CloudHypervisorRuntimeProvider")
            .field("provider_id", &self.provider_id)
            .field("control_attached", &self.control.is_some())
            .finish_non_exhaustive()
    }
}

impl CloudHypervisorRuntimeProvider {
    /// Construct with the canonical Cloud Hypervisor provider id.
    pub fn new(ch_input: ChArgvInput) -> Self {
        Self::with_provider_id(
            static_provider_id(CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID),
            ch_input,
        )
    }

    /// Construct with an explicit provider id.
    pub fn with_provider_id(provider_id: ProviderId, ch_input: ChArgvInput) -> Self {
        Self {
            provider_id,
            ch_input,
            control: None,
        }
    }

    /// Attach the daemon/broker runtime control seam.
    pub fn with_control(mut self, control: Arc<dyn CloudHypervisorRuntimeControl>) -> Self {
        self.control = Some(control);
        self
    }

    /// Borrow the Cloud Hypervisor argv input.
    pub fn ch_input(&self) -> &ChArgvInput {
        &self.ch_input
    }

    /// Render Cloud Hypervisor argv through the existing generator. This is
    /// exposed for conformance tests and daemon-side execution only; the
    /// provider DTOs never carry the rendered argv.
    pub fn cloud_hypervisor_argv(&self) -> ProviderResult<Vec<String>> {
        generate_ch_argv(&self.ch_input).map_err(ch_argv_error)
    }

    fn ensure_workload_matches(&self, spec: &WorkloadSpec) -> ProviderResult<()> {
        if spec.alias.as_str() == self.ch_input.vm_name.as_str() {
            Ok(())
        } else {
            Err(ProviderError::new(
                ErrorKind::InvalidTarget,
                "workload alias does not match the Cloud Hypervisor runtime input",
            ))
        }
    }
}

#[async_trait]
impl RuntimeProvider for CloudHypervisorRuntimeProvider {
    fn provider_id(&self) -> ProviderId {
        self.provider_id.clone()
    }

    fn capabilities(&self) -> RuntimeCapabilitySet {
        let mut caps = CapabilitySet::empty()
            .with(Capability::Vsock)
            .with(Capability::Virtiofs);
        if self.control.is_some() {
            caps = caps.with(Capability::Lifecycle);
        }
        RuntimeCapabilitySet { caps }
    }

    async fn plan_workload(&self, spec: WorkloadSpec) -> ProviderResult<RuntimePlan> {
        self.ensure_workload_matches(&spec)?;
        self.cloud_hypervisor_argv()?;
        Ok(RuntimePlan {
            provider: self.provider_id(),
            workload: spec.alias,
        })
    }

    async fn start(&self, plan: RuntimePlan) -> ProviderResult<RuntimeHandle> {
        if plan.provider != self.provider_id {
            return Err(ProviderError::new(
                ErrorKind::InvalidTarget,
                "runtime plan provider does not match Cloud Hypervisor provider",
            ));
        }
        let control = self.control.as_ref().ok_or_else(|| {
            ProviderError::unsupported(format!(
                "runtime provider '{CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID}' start requires daemon runtime control"
            ))
        })?;
        control.start(plan, &self.ch_input).await
    }

    async fn stop(&self, handle: RuntimeHandle) -> ProviderResult<()> {
        let control = self.control.as_ref().ok_or_else(|| {
            ProviderError::unsupported(format!(
                "runtime provider '{CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID}' stop requires daemon runtime control"
            ))
        })?;
        control.stop(handle).await
    }

    async fn inspect(&self, handle: RuntimeHandle) -> ProviderResult<RuntimeStatus> {
        let control = self.control.as_ref().ok_or_else(|| {
            ProviderError::unsupported(format!(
                "runtime provider '{CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID}' inspect requires daemon runtime control"
            ))
        })?;
        control.inspect(handle).await
    }
}

fn static_provider_id(id: &'static str) -> ProviderId {
    ProviderId::parse(id).expect("static runtime provider id must use the provider-id label shape")
}

fn ch_argv_error(err: ChArgvError) -> ProviderError {
    let message = match err {
        ChArgvError::EmptyVmName => "invalid Cloud Hypervisor runtime input: empty VM name",
        ChArgvError::InvalidChBinaryPath { .. } => {
            "invalid Cloud Hypervisor runtime input: invalid binary path"
        }
        ChArgvError::ZeroCpus => "invalid Cloud Hypervisor runtime input: zero CPUs",
        ChArgvError::EmptyKernelPath => "invalid Cloud Hypervisor runtime input: empty kernel path",
        ChArgvError::TapFdMissing { .. } => {
            "invalid Cloud Hypervisor runtime input: missing TAP file descriptor"
        }
        ChArgvError::TapIfnameMissing { .. } => {
            "invalid Cloud Hypervisor runtime input: missing TAP interface name"
        }
    };
    ProviderError::new(ErrorKind::ProviderAllocationFailed, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ch_argv::{ChFsShare, ChNetHandoff, ChNetIface, ChVsock};
    use d2b_constellation_core::WorkloadId;
    use std::sync::{Arc, Mutex};

    fn workload_id(raw: &str) -> WorkloadId {
        WorkloadId::parse(raw).expect("test workload id")
    }

    fn provider_id(raw: &str) -> ProviderId {
        ProviderId::parse(raw).expect("test provider id")
    }

    fn representative_input() -> ChArgvInput {
        ChArgvInput {
            vm_name: "corp-vm".to_owned(),
            ch_binary_path: "/runtime-test/cloud-hypervisor".to_owned(),
            cpus: 1,
            watchdog: true,
            kernel_path: "/runtime-test/vmlinux".to_owned(),
            initramfs_path: Some("/runtime-test/initrd".to_owned()),
            cmdline: "console=ttyS0 init=/runtime-test/init".to_owned(),
            seccomp: "true".to_owned(),
            memory: "shared=on,size=512M".to_owned(),
            platform_oem_strings: vec!["notify=vsock-stream:2:8888".to_owned()],
            console: "null".to_owned(),
            serial: "tty".to_owned(),
            primary_vsock: Some(ChVsock {
                cid: 10_914_385,
                socket: "notify.vsock".to_owned(),
            }),
            extra_vsock: Vec::new(),
            fs_shares: vec![ChFsShare {
                socket: "runtime-test-virtiofs.sock".to_owned(),
                tag: "ro-store".to_owned(),
            }],
            api_socket_path: "runtime-test-api.sock".to_owned(),
            net_ifaces: vec![ChNetIface {
                mac: "02:76:53:AE:57:0A".to_owned(),
                tap_ifname: "work-l10".to_owned(),
                tap_fd: None,
            }],
            net_handoff: ChNetHandoff::PersistentTap,
            extra_args: Vec::new(),
        }
    }

    #[test]
    fn cloud_hypervisor_debug_redacts_runtime_inputs() {
        let mut input = representative_input();
        input.ch_binary_path = "/runtime-debug/cloud-hypervisor".to_owned();
        input.kernel_path = "/runtime-debug/vmlinux".to_owned();
        input.api_socket_path = "runtime-debug-api.sock".to_owned();
        input.extra_args.push("/runtime-debug/extra".to_owned());
        let rendered = format!("{:?}", CloudHypervisorRuntimeProvider::new(input));
        assert!(rendered.contains("CloudHypervisorRuntimeProvider"));
        assert!(rendered.contains(CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID));
        for forbidden in ["runtime-debug", "vmlinux", "api.sock", "extra"] {
            assert!(
                !rendered.contains(forbidden),
                "runtime provider Debug leaked {forbidden}: {rendered}"
            );
        }
    }

    #[tokio::test]
    async fn cloud_hypervisor_default_plans_opaque_runtime_plan() {
        let provider = CloudHypervisorRuntimeProvider::new(representative_input());
        let plan = provider
            .plan_workload(WorkloadSpec {
                alias: workload_id("corp-vm"),
            })
            .await
            .unwrap();
        assert_eq!(plan.provider.as_str(), CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID);
        assert_eq!(plan.workload.as_str(), "corp-vm");
        let encoded = serde_json::to_string(&plan).unwrap();
        for forbidden in [
            "runtime-test",
            "vmlinux",
            "initrd",
            "console=ttyS0",
            "notify.vsock",
            "work-l10",
            "02:76",
        ] {
            assert!(
                !encoded.contains(forbidden),
                "RuntimePlan leaked runtime input {forbidden}: {encoded}"
            );
        }
    }

    #[tokio::test]
    async fn cloud_hypervisor_planning_reuses_argv_validation() {
        let mut input = representative_input();
        input.cpus = 0;
        let err = CloudHypervisorRuntimeProvider::new(input)
            .plan_workload(WorkloadSpec {
                alias: workload_id("corp-vm"),
            })
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ProviderAllocationFailed);
        assert!(!err.to_string().contains("runtime-test"));
    }

    #[tokio::test]
    async fn cloud_hypervisor_plan_rejects_mismatched_workload_alias() {
        let err = CloudHypervisorRuntimeProvider::new(representative_input())
            .plan_workload(WorkloadSpec {
                alias: workload_id("other-vm"),
            })
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidTarget);
    }

    #[test]
    fn unsupported_profiles_fail_without_fallback() {
        for id in [
            CROSVM_RUNTIME_PROVIDER_ID,
            QEMU_RUNTIME_PROVIDER_ID,
            QEMU_MEDIA_RUNTIME_PROVIDER_ID,
        ] {
            let err =
                validate_runtime_profile(&provider_id(id), RuntimeWorkloadRequirements::default())
                    .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
            assert!(
                err.to_string()
                    .contains(CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID)
            );
        }

        let err = validate_runtime_profile(
            &provider_id("unknown-runtime"),
            RuntimeWorkloadRequirements::default(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
        assert!(
            err.to_string()
                .contains(CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID)
        );
    }

    #[test]
    fn firecracker_denies_desktop_dependencies() {
        for requirements in [
            RuntimeWorkloadRequirements {
                guest_control: true,
                ..Default::default()
            },
            RuntimeWorkloadRequirements {
                virtiofs: true,
                ..Default::default()
            },
            RuntimeWorkloadRequirements {
                store_sync: true,
                ..Default::default()
            },
            RuntimeWorkloadRequirements {
                desktop: true,
                ..Default::default()
            },
            RuntimeWorkloadRequirements {
                graphics: true,
                ..Default::default()
            },
            RuntimeWorkloadRequirements {
                audio: true,
                ..Default::default()
            },
            RuntimeWorkloadRequirements {
                usb: true,
                ..Default::default()
            },
        ] {
            let err = validate_runtime_profile(
                &provider_id(FIRECRACKER_RUNTIME_PROVIDER_ID),
                requirements,
            )
            .unwrap_err();
            assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
            assert!(
                err.to_string()
                    .contains(CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID)
            );
        }
    }

    #[test]
    fn firecracker_default_is_still_unsupported() {
        let err = validate_runtime_profile(
            &provider_id(FIRECRACKER_RUNTIME_PROVIDER_ID),
            RuntimeWorkloadRequirements::default(),
        )
        .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnsupportedFeature);
        assert!(err.to_string().contains("not implemented"));
        assert!(
            err.to_string()
                .contains(CLOUD_HYPERVISOR_RUNTIME_PROVIDER_ID)
        );
    }

    #[derive(Debug, Default)]
    struct RecordingControl {
        started: Mutex<Vec<String>>,
        stopped: Mutex<Vec<String>>,
        inspected: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl CloudHypervisorRuntimeControl for RecordingControl {
        async fn start(
            &self,
            plan: RuntimePlan,
            input: &ChArgvInput,
        ) -> ProviderResult<RuntimeHandle> {
            self.started
                .lock()
                .expect("recording control lock")
                .push(input.vm_name.clone());
            Ok(RuntimeHandle {
                workload: plan.workload,
            })
        }

        async fn stop(&self, handle: RuntimeHandle) -> ProviderResult<()> {
            self.stopped
                .lock()
                .expect("recording control lock")
                .push(handle.workload.as_str().to_owned());
            Ok(())
        }

        async fn inspect(&self, handle: RuntimeHandle) -> ProviderResult<RuntimeStatus> {
            self.inspected
                .lock()
                .expect("recording control lock")
                .push(handle.workload.as_str().to_owned());
            Ok(RuntimeStatus {
                workload: handle.workload,
                running: true,
            })
        }
    }

    #[tokio::test]
    async fn cloud_hypervisor_start_delegates_to_control() {
        let control = Arc::new(RecordingControl::default());
        let provider = CloudHypervisorRuntimeProvider::new(representative_input())
            .with_control(control.clone());
        assert!(provider.capabilities().has(Capability::Lifecycle));
        let plan = provider
            .plan_workload(WorkloadSpec {
                alias: workload_id("corp-vm"),
            })
            .await
            .unwrap();
        let handle = provider.start(plan).await.unwrap();
        assert_eq!(handle.workload.as_str(), "corp-vm");
        assert_eq!(
            control
                .started
                .lock()
                .expect("recording control lock")
                .as_slice(),
            &["corp-vm".to_owned()]
        );
    }

    #[tokio::test]
    async fn cloud_hypervisor_rejects_plan_for_other_provider() {
        let control = Arc::new(RecordingControl::default());
        let provider =
            CloudHypervisorRuntimeProvider::new(representative_input()).with_control(control);
        let err = provider
            .start(RuntimePlan {
                provider: provider_id("other-provider"),
                workload: workload_id("corp-vm"),
            })
            .await
            .unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidTarget);
    }

    #[tokio::test]
    async fn cloud_hypervisor_lifecycle_requires_control_seam() {
        let provider = CloudHypervisorRuntimeProvider::new(representative_input());
        let plan = RuntimePlan {
            provider: provider.provider_id(),
            workload: workload_id("corp-vm"),
        };
        assert_eq!(
            provider.start(plan).await.unwrap_err().kind(),
            ErrorKind::UnsupportedFeature
        );
        let handle = RuntimeHandle {
            workload: workload_id("corp-vm"),
        };
        assert_eq!(
            provider.stop(handle.clone()).await.unwrap_err().kind(),
            ErrorKind::UnsupportedFeature
        );
        assert_eq!(
            provider.inspect(handle).await.unwrap_err().kind(),
            ErrorKind::UnsupportedFeature
        );
    }

    #[tokio::test]
    async fn cloud_hypervisor_stop_and_inspect_delegate_to_control() {
        let control = Arc::new(RecordingControl::default());
        let provider = CloudHypervisorRuntimeProvider::new(representative_input())
            .with_control(control.clone());
        let handle = RuntimeHandle {
            workload: workload_id("corp-vm"),
        };
        provider.stop(handle.clone()).await.unwrap();
        let status = provider.inspect(handle).await.unwrap();
        assert!(status.running);
        assert_eq!(
            control
                .stopped
                .lock()
                .expect("recording control lock")
                .as_slice(),
            &["corp-vm".to_owned()]
        );
        assert_eq!(
            control
                .inspected
                .lock()
                .expect("recording control lock")
                .as_slice(),
            &["corp-vm".to_owned()]
        );
    }
}
