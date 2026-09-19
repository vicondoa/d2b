//! The GPU family's declared vocabulary.
//!
//! Every fact here is owned by this crate: the host device-matrix classes
//! the Gpu role claims, the grant-class labels the authority-fenced effect
//! digests are derived over, and the inventory `busClass` the family's
//! Device rows are selected under. Shared consumers read these constants
//! instead of restating the spellings.

/// The host device-matrix classes the Gpu role claims.
///
/// This is the P1 matrix the posture lookup resolves for the Gpu and
/// GpuRenderNode roles: `/dev/kvm`, `/dev/dri/renderD128`, `/dev/nvidiactl`,
/// `/dev/nvidia0` (nvidia-render), `/dev/nvidia-uvm`, `/dev/udmabuf`.
pub const GPU_DEVICE_CLASSES: &[&str] = &[
    "kvm",
    "dri",
    "nvidia-ctl",
    "nvidia-uvm",
    "nvidia-render",
    "udmabuf",
];

/// The grant-class labels the GPU effect digests are derived over.
///
/// The labels are the committed `d2b:gpu-device-grant/v2` digest vocabulary;
/// they match [`GPU_DEVICE_CLASSES`] except for the NVIDIA primary node,
/// which the grant path spells `nvidia-device`.
pub const GPU_GRANT_CLASSES: &[&str] = &["dri", "kvm", "udmabuf"];

/// The grant-class labels the render-node-only mode keeps: the render node
/// is fd-passed, so the bind-mount classes (`kvm`, `udmabuf`) are dropped.
pub const GPU_RENDER_NODE_GRANT_CLASSES: &[&str] = &["dri"];

/// The grant-class labels the video/NVIDIA-decode arm adds to the effect
/// digest set.
pub const GPU_VIDEO_GRANT_CLASSES: &[&str] = &["nvidia-ctl", "nvidia-device", "nvidia-uvm"];

/// The inventory `busClass` the family's Device rows are selected under.
pub const GPU_BUS_CLASS: &str = "drm";