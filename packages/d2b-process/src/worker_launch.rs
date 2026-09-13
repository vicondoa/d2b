//! Typed launch parameters for the declared Device- and binding-owned worker
//! rows.
//!
//! The Device Providers declare their worker rows path-free and the Process
//! spec is argv-free by contract, so the host inputs the device argv
//! generators need are derived by the Process controller and travel to the
//! provider composition as these typed parameters - never as argv on the row
//! or the spec.

use std::path::PathBuf;

use d2b_contracts_resource::v3::ResourceRef;
use d2b_contracts_resource::v3::execution_policy::BoundedToken;
use d2b_contracts_resource::v3::volume::{AttachmentAccess, AttachmentCache};

/// Where one serving worker's served view root comes from.
///
/// A local-path Volume's root is derived from the trusted storage path row
/// its source policy names; a `nix-closure` Volume's bytes live in the
/// broker-managed per-Guest store-view hardlink farm instead, which the
/// bundle names through the Guest's store-view intent (`store-view/live`,
/// the `ro-store` share's preserved redirect). A source kind neither of
/// those names keeps no root at all: the ticket refuses rather than serving
/// an unnamed subtree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServingWorkerRoot {
    /// Trusted storage path row id (`path:<policy>`) for a local-path source.
    StoragePath(String),
    /// The broker-managed store-view farm of the ticket's target Guest.
    StoreViewFarm,
}

/// Binding-declared launch inputs for one binding-owned serving worker.
///
/// The Process controller composes the worker's arguments from the owning
/// VolumeBinding and the Volume it serves (KD13: the binding controller
/// declares what to serve through its resources; the Process controller owns
/// the launch). Nothing here names an executable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServingWorkerLaunch {
    /// Volume whose view the worker serves.
    pub volume_ref: ResourceRef,
    /// Named view served to the target Guest.
    pub view: BoundedToken,
    /// Attachment execution target (the ticket's guest target ref).
    pub guest_ref: ResourceRef,
    /// Trusted root the served view is anchored at, when the source kind has
    /// one this daemon may derive.
    pub root: Option<ServingWorkerRoot>,
    /// View-relative path within the Volume root.
    pub view_path: String,
    /// Attachment access declared by the binding.
    pub access: AttachmentAccess,
    /// Resolved worker thread pool (attachment tuning or the declared default).
    pub thread_pool_size: u32,
    /// Whether POSIX ACLs are served.
    pub posix_acl: bool,
    /// Whether extended attributes are served.
    pub xattr: bool,
    /// Page-cache mode.
    pub cache: AttachmentCache,
    /// Optional resolved socket group name.
    pub socket_group: Option<String>,
}

/// Typed launch parameters for one Device-owned worker row.
///
/// The Device Providers declare their worker rows path-free (`Process/swtpm-<device>`,
/// `Process/gpu-<device>`, ...), and the Process spec is argv-free by
/// contract, so the host inputs the device argv generators need are derived
/// by the Process controller and travel to the provider composition as these
/// typed parameters - never as argv on the row or the spec (U17 gap
/// closure).
///
/// The executable is carried only so the owning Provider's own argv
/// generator validates it: the trusted template pins the binary and the
/// broker composes `argv[0]`, so the composed argument list starts after it.
#[derive(Debug, Clone, PartialEq)]
pub enum DeviceWorkerLaunch {
    /// `Process/swtpm-<device>` (`swtpm-socket`).
    Swtpm(Box<SwtpmWorkerParams>),
    /// `EphemeralProcess/swtpm-flush-<device>` (`swtpm-init-flush`).
    SwtpmFlush(Box<SwtpmFlushParams>),
    /// `Process/gpu-<device>` (`gpu-worker` / `gpu-render-node`).
    Gpu(Box<GpuWorkerParams>),
    /// `Process/video-<device>` (`video-worker`).
    Video(Box<VideoWorkerParams>),
}

/// Long-lived `swtpm socket` inputs: the per-Device state directory, the two
/// sockets swtpm binds, and the socket owner ids it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwtpmWorkerParams {
    /// Trusted `swtpm` binary the declared template pins.
    pub binary_path: PathBuf,
    /// VM the Device belongs to (the socket root and the process title).
    pub vm_name: String,
    /// State directory the controller-owned state Volume is backed by.
    pub state_dir: PathBuf,
    /// `--ctrl` socket (daemon-only; the guest VMM never connects to it).
    pub ctrl_socket_path: PathBuf,
    /// `--server` socket the guest VMM connects to.
    pub server_socket_path: PathBuf,
    /// The identity the worker holds in the namespace its posture installs:
    /// in-namespace `0` under the ADR 0021 single-entry mapping (the host
    /// principal is unmapped there, so naming it makes swtpm's socket chown
    /// fail with `EINVAL`), the host principal for a posture without one.
    pub uid: u32,
    /// The group the worker holds in that same namespace.
    pub gid: u32,
    /// `--log level=<N>`; the Device Provider's own bounded default.
    pub log_level: u8,
}

/// Pre-start flush inputs: `swtpm_ioctl -i --unix <ctrl>`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SwtpmFlushParams {
    /// Trusted `swtpm-ioctl` binary the declared template pins.
    pub ioctl_binary_path: PathBuf,
    /// VM the flushed Device belongs to.
    pub vm_name: String,
    /// The same `--ctrl` socket the long-lived worker binds.
    pub ctrl_socket_path: PathBuf,
}

/// `crosvm device gpu` inputs: the private sidecar socket, the host Wayland
/// socket the sidecar renders into, and the Device's declared context shape.
#[derive(Debug, Clone, PartialEq)]
pub struct GpuWorkerParams {
    /// Trusted `crosvm` binary the declared template pins.
    pub binary_path: PathBuf,
    /// VM the Device belongs to.
    pub vm_name: String,
    /// `--socket` value the guest VMM's `--gpu socket=` argument names.
    pub socket_path: PathBuf,
    /// `--wayland-sock` value.
    pub wayland_sock: PathBuf,
    /// `--params` payload, from the owning Device's declared settings, as the
    /// canonical JSON of those settings.
    pub params: serde_json::Value,
}

/// `crosvm device video-decoder` inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoWorkerParams {
    /// Trusted `crosvm` binary the declared template pins.
    pub binary_path: PathBuf,
    /// VM the video Device belongs to.
    pub vm_name: String,
    /// `--socket-path` value the guest VMM's `--vhost-user-media socket=`
    /// argument names.
    pub socket_path: PathBuf,
}
