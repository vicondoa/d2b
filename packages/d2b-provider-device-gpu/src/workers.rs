//! Signed semantic Process worker declarations.

use core::fmt;

use crate::{
    GpuProcessRole, GpuProcessSelectionError, GpuSettings, process::GpuProcessDeclaration,
};
use d2b_contracts::v3::ResourceUid;

/// Closed device grant classes used by GPU worker templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GpuDeviceNode {
    /// Shared KVM grant for a full GPU worker.
    Kvm,
    /// DRM render node grant.
    Dri,
    /// DMA buffer grant for full GPU workers.
    Udmabuf,
    /// NVIDIA control device.
    NvidiaCtl,
    /// NVIDIA device node.
    NvidiaDevice,
    /// NVIDIA UVM device.
    NvidiaUvm,
}

impl GpuDeviceNode {
    /// Return the signed semantic device token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kvm => "kvm",
            Self::Dri => "dri",
            Self::Udmabuf => "udmabuf",
            Self::NvidiaCtl => "nvidia-ctl",
            Self::NvidiaDevice => "nvidia-device",
            Self::NvidiaUvm => "nvidia-uvm",
        }
    }
}

/// Signed semantic GPU worker specification.
#[derive(Clone, PartialEq, Eq)]
pub struct GpuWorkerSpec {
    process: GpuProcessDeclaration,
    template: &'static str,
    seccomp_class: &'static str,
    namespaces: &'static [&'static str],
    device_nodes: Vec<GpuDeviceNode>,
    user_namespace: bool,
    capabilities: &'static [&'static str],
    state_mounts: &'static [&'static str],
}

impl GpuWorkerSpec {
    /// Build the fixed GPU or render-node worker shape.
    pub fn gpu(
        device_uid: &ResourceUid,
        settings: &GpuSettings,
    ) -> Result<Self, GpuProcessSelectionError> {
        let role = if settings.render_node_only {
            GpuProcessRole::RenderNode
        } else {
            GpuProcessRole::FullGpu
        };
        let process = GpuProcessDeclaration::new(device_uid, role)?;
        let (template, seccomp_class, namespaces, device_nodes) = match role {
            GpuProcessRole::FullGpu => (
                "gpu-worker",
                "w1-gpu",
                &["mount", "pid", "ipc", "uts", "cgroup", "user"][..],
                vec![
                    GpuDeviceNode::Kvm,
                    GpuDeviceNode::Dri,
                    GpuDeviceNode::Udmabuf,
                ],
            ),
            GpuProcessRole::RenderNode => (
                "render-node-worker",
                "w1-gpu-render-node",
                &["mount", "pid", "ipc", "uts", "cgroup", "user"][..],
                vec![GpuDeviceNode::Dri],
            ),
            GpuProcessRole::Video => unreachable!("GPU settings select a GPU role"),
        };
        Ok(Self {
            process,
            template,
            seccomp_class,
            namespaces,
            device_nodes,
            user_namespace: true,
            capabilities: &[],
            state_mounts: &[],
        })
    }

    /// Borrow the deterministic Process declaration.
    pub const fn process(&self) -> &GpuProcessDeclaration {
        &self.process
    }

    /// Return the signed component template.
    pub const fn template(&self) -> &'static str {
        self.template
    }

    /// Return the signed seccomp class.
    pub const fn seccomp_class(&self) -> &'static str {
        self.seccomp_class
    }

    /// Return the semantic namespace classes.
    pub const fn namespaces(&self) -> &'static [&'static str] {
        self.namespaces
    }

    /// Borrow the closed device allowlist.
    pub fn device_nodes(&self) -> &[GpuDeviceNode] {
        &self.device_nodes
    }

    /// Whether Core must resolve the process-principal user namespace.
    pub const fn user_namespace(&self) -> bool {
        self.user_namespace
    }

    /// Borrow the host capability allowlist, which is intentionally empty.
    pub const fn capabilities(&self) -> &'static [&'static str] {
        self.capabilities
    }

    /// Borrow declared state mounts, which are intentionally empty.
    pub const fn state_mounts(&self) -> &'static [&'static str] {
        self.state_mounts
    }
}

impl fmt::Debug for GpuWorkerSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GpuWorkerSpec")
            .field("process", &self.process)
            .field("template", &self.template)
            .field("seccomp_class", &self.seccomp_class)
            .field("device_node_count", &self.device_nodes.len())
            .field("user_namespace", &self.user_namespace)
            .finish()
    }
}

/// Signed semantic video worker specification.
#[derive(Clone, PartialEq, Eq)]
pub struct VideoWorkerSpec {
    process: GpuProcessDeclaration,
    device_nodes: Vec<GpuDeviceNode>,
}

impl VideoWorkerSpec {
    /// Build the separate video worker shape.
    pub fn new(
        device_uid: &ResourceUid,
        settings: &GpuSettings,
    ) -> Result<Self, GpuProcessSelectionError> {
        let process = GpuProcessDeclaration::new(device_uid, GpuProcessRole::Video)?;
        let mut device_nodes = vec![GpuDeviceNode::Dri];
        if settings.video_nvidia_decode {
            device_nodes.extend([
                GpuDeviceNode::NvidiaCtl,
                GpuDeviceNode::NvidiaDevice,
                GpuDeviceNode::NvidiaUvm,
            ]);
        }
        Ok(Self {
            process,
            device_nodes,
        })
    }

    /// Borrow the deterministic Process declaration.
    pub const fn process(&self) -> &GpuProcessDeclaration {
        &self.process
    }

    /// Return the signed component template.
    pub const fn template(&self) -> &'static str {
        "video-worker"
    }

    /// Return the signed seccomp class.
    pub const fn seccomp_class(&self) -> &'static str {
        "w1-video"
    }

    /// Return the semantic namespace classes.
    pub const fn namespaces(&self) -> &'static [&'static str] {
        &["mount", "pid", "ipc", "uts", "cgroup"]
    }

    /// Borrow the video device allowlist.
    pub fn device_nodes(&self) -> &[GpuDeviceNode] {
        &self.device_nodes
    }

    /// Video never requests a user namespace.
    pub const fn user_namespace(&self) -> bool {
        false
    }

    /// Video has no host capabilities.
    pub const fn capabilities(&self) -> &'static [&'static str] {
        &[]
    }

    /// Video has no Provider state mount.
    pub const fn state_mounts(&self) -> &'static [&'static str] {
        &[]
    }
}

impl fmt::Debug for VideoWorkerSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("VideoWorkerSpec")
            .field("process", &self.process)
            .field("device_node_count", &self.device_nodes.len())
            .finish()
    }
}
