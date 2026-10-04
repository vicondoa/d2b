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

// ---------------------------------------------------------------------------
// Guest-target Process realization (R18, R29, KTD6)
// ---------------------------------------------------------------------------

/// One prepared `EndpointBinding` relationship a Guest-target Process
/// realization delivers to the target-local process.
///
/// Every field is the fact the launch gate sealed into its
/// [`crate::effects::BindingAuthorityLease`] for this exact consumer: the
/// relationship row, the endpoint that published it, the canonical consumer
/// slot, and the opaque realization-incarnation token the endpoint published.
/// Nothing host-shaped travels here - no socket name, no path, no `(dev,
/// ino)` pair - so a delivery can be compared, logged, and applied on the
/// target without leaking anything about the host that prepared it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuestBindingDelivery {
    /// The canonical `EndpointBinding` row this relationship lives in.
    binding_ref: String,
    /// The canonical `Endpoint` row that published the realization.
    endpoint_ref: String,
    /// The canonical consumer slot the delivery applies at.
    slot: String,
    /// The opaque realization-incarnation token the endpoint published.
    incarnation: String,
}

impl GuestBindingDelivery {
    /// Record one prepared delivery for this consumer.
    ///
    /// # Errors
    ///
    /// Returns [`GuestRealizationError::Incomplete`] when any field is
    /// empty. A delivery assembled from an incomplete fact would compare
    /// against evidence nothing can publish, so it is refused at the point
    /// it is built rather than deferred forever on the target.
    pub fn new(
        binding_ref: impl Into<String>,
        endpoint_ref: impl Into<String>,
        slot: impl Into<String>,
        incarnation: impl Into<String>,
    ) -> Result<Self, GuestRealizationError> {
        let delivery = Self {
            binding_ref: binding_ref.into(),
            endpoint_ref: endpoint_ref.into(),
            slot: slot.into(),
            incarnation: incarnation.into(),
        };
        if delivery.binding_ref.is_empty()
            || delivery.endpoint_ref.is_empty()
            || delivery.slot.is_empty()
            || delivery.incarnation.is_empty()
        {
            return Err(GuestRealizationError::Incomplete);
        }
        Ok(delivery)
    }

    /// The canonical relationship row this delivery belongs to.
    pub fn binding_ref(&self) -> &str {
        &self.binding_ref
    }

    /// The canonical endpoint row that published the realization.
    pub fn endpoint_ref(&self) -> &str {
        &self.endpoint_ref
    }

    /// The canonical consumer slot.
    pub fn slot(&self) -> &str {
        &self.slot
    }

    /// The opaque realization-incarnation token.
    pub fn incarnation(&self) -> &str {
        &self.incarnation
    }
}

/// Why one Guest-target realization could not be assembled or read back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestRealizationError {
    /// A required fact was missing, so the realization is refused rather
    /// than sent with a hole the target would have to guess at.
    Incomplete,
    /// The bytes carried are not a realization this Host wrote.
    Unreadable,
}

impl GuestRealizationError {
    /// The closed, host-free slug this refusal reports under.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Incomplete => "process-guest-realization-incomplete",
            Self::Unreadable => "process-guest-realization-unreadable",
        }
    }
}

impl core::fmt::Display for GuestRealizationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for GuestRealizationError {}

/// The host-resolved target-local realization of one `Process` row that runs
/// on a Guest target (R18, R29).
///
/// This is the exact byte sequence the Host sends as the realize frame's
/// spec and the Guest applies verbatim. It has two halves and the Guest
/// composes neither:
///
/// - the resolved `Process` spec, exactly as the Host decoded and
///   re-serialized it for the target; and
/// - the prepared `EndpointBinding` deliveries a sealed authority lease
///   carried at the moment the launch was admitted (KTD6). The Host sends
///   them only after that lease revalidated, so a stale or revoked lease
///   reaches the Guest as no realization at all rather than as a process
///   started over bindings that moved.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct GuestProcessRealization {
    /// The canonical Host-zone `Process` row this realization belongs to.
    process_ref: String,
    /// The exact resolved Process spec bytes the target applies.
    spec: Vec<u8>,
    /// The prepared endpoint relationships delivered with this realization.
    deliveries: Vec<GuestBindingDelivery>,
}

impl GuestProcessRealization {
    /// Assemble one target-local realization.
    pub fn new(
        process_ref: impl Into<String>,
        spec: Vec<u8>,
        deliveries: Vec<GuestBindingDelivery>,
    ) -> Self {
        Self { process_ref: process_ref.into(), spec, deliveries }
    }

    /// The canonical Host-zone `Process` row this realization belongs to.
    pub fn process_ref(&self) -> &str {
        &self.process_ref
    }

    /// The exact resolved Process spec bytes the target applies.
    pub fn spec(&self) -> &[u8] {
        &self.spec
    }

    /// The prepared endpoint relationships delivered with this realization.
    pub fn deliveries(&self) -> &[GuestBindingDelivery] {
        &self.deliveries
    }

    /// The wire bytes of this realization: the spec of the realize frame.
    ///
    /// The frame's commitment is [`Self::spec_digest`] over exactly these
    /// bytes, so a substituted or truncated realization is refused on the
    /// target before its effect code sees it.
    pub fn encode(&self) -> Vec<u8> {
        // Serialization of a struct whose fields are all owned strings, byte
        // vectors, and strings cannot fail.
        serde_json::to_vec(self).unwrap_or_default()
    }

    /// The commitment the Host attaches to [`Self::encode`].
    pub fn spec_digest(&self) -> String {
        d2b_resource_runtime::guest_target::target_local_spec_digest(&self.encode())
    }

    /// Read back a realization the Host wrote.
    ///
    /// # Errors
    ///
    /// Returns [`GuestRealizationError::Unreadable`] when the bytes are not
    /// a realization this Host writes, or
    /// [`GuestRealizationError::Incomplete`] when a required field is
    /// empty.
    pub fn decode(bytes: &[u8]) -> Result<Self, GuestRealizationError> {
        let realization: Self =
            serde_json::from_slice(bytes).map_err(|_| GuestRealizationError::Unreadable)?;
        if realization.process_ref.is_empty() || realization.spec.is_empty() {
            return Err(GuestRealizationError::Incomplete);
        }
        if realization.deliveries.iter().any(|delivery| {
            delivery.binding_ref.is_empty()
                || delivery.endpoint_ref.is_empty()
                || delivery.slot.is_empty()
                || delivery.incarnation.is_empty()
        }) {
            return Err(GuestRealizationError::Incomplete);
        }
        Ok(realization)
    }
}
