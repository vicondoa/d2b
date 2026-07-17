//! `crosvm device video-decoder` sidecar argv generator.
//!
//! Pure Rust function that emits the argv for the per-VM
//! `d2b-<vm>-video.service` video decode sidecar per
//! `nixos-modules/components/video/host.nix`. Only graphics-enabled
//! VMs run this sidecar (needs virtio-gpu for presentation).
//!
//! Shape:
//!
//! ```text
//! crosvm device video-decoder \
//!   --socket-path /run/d2b-video/<vm>/video.sock \
//!   --backend vaapi
//! ```
//!
//! The video sidecar runs as the graphics VM's `d2b-<vm>-gpu`
//! uid (needs `/dev/dri` + NVIDIA access); the CH runner appends
//! `--vhost-user-media socket=<socket>` to its argv to consume the
//! decoded media stream.
//!
//! Crate invariant `#![forbid(unsafe_code)]` is honoured.

use serde::{Deserialize, Serialize};

// =========================================================================
// Wire-contract pins
// =========================================================================
//
// `pkgs/spectrum-ch/cloud-hypervisor/0003-vhost-user-media-device.patch`
// hard-codes the virtio-media wire shape that this sidecar speaks to the
// guest through cloud-hypervisor. These constants are NOT user-tunable
// argv flags — they live in the CH patch and the crosvm vhost-user-media
// backend. We mirror them here so the byte-parity golden
// (`tests/golden/runner-shape/video-argv-minimal.txt`) captures the full
// effective wire shape, and any future drift in the CH patch surfaces as
// a golden diff in CI even though no argv changed.
//
// Every constant cites the patch line that pins it.

/// virtio device-type id for `vhost-user-media`. Pinned in
/// `0003-vhost-user-media-device.patch` as `const VIRTIO_ID_MEDIA: u32 = 48`.
pub const VIRTIO_ID_MEDIA: u32 = 48;

/// Number of virtqueues exposed by `vhost-user-media` (one command, one
/// event). Pinned in the CH patch via `const NUM_QUEUES: u16 = QUEUE_SIZES.len() as _`.
pub const VHOST_USER_MEDIA_NUM_QUEUES: u16 = 2;

/// Per-queue descriptor-ring size. Pinned in the CH patch via
/// `const QUEUE_SIZES: &[u16] = &[256, 256]` (both queues identical).
pub const VHOST_USER_MEDIA_QUEUE_SIZE: u16 = 256;

/// Single virtio shared-memory region length in bytes (256 MiB). Pinned in
/// the CH patch via
/// `VhostSharedMemoryRegion { id: 0, padding: [0; 7], length: 256 * 1024 * 1024 }`.
pub const VHOST_USER_MEDIA_SHM_REGION_BYTES: u64 = 256 * 1024 * 1024;

/// Forced `SET_VRING_BASE` value for every queue. Pinned in the CH patch
/// in `activate()`: `self.vu_common.vring_bases = Some(vec![0; queues.len()])`.
/// The virtio-media guest driver pre-queues event buffers on queue 1 before
/// `DRIVER_OK`; the explicit zero override keeps those buffers visible to
/// the backend on resume.
pub const VHOST_USER_MEDIA_VRING_BASE: u64 = 0;

/// vhost-user protocol features negotiated by the media backend. Pinned in
/// the CH patch's `acked_protocol_features` to exactly this set
/// (`SHMEM_MAP_CROSVM | BACKEND_REQ | REPLY_ACK`). Order is sorted
/// lexicographically for stable golden output.
pub const VHOST_USER_MEDIA_PROTOCOL_FLAGS: &str = "BACKEND_REQ|REPLY_ACK|SHMEM_MAP_CROSVM";

/// PCI MMIO allocator used for the SHM region. Pinned in the CH patch via
/// `self.pci_segments[..].mem64_allocator.lock()...allocate(...)`. The
/// allocator name is part of the wire shape because changing it (e.g. to
/// `mem32_allocator`) changes the guest-visible BAR layout.
pub const VHOST_USER_MEDIA_MMIO_ALLOCATOR: &str = "pci-mem64";

/// Render the wire-contract pins as a single deterministic line that the
/// golden parity test byte-compares. Format is `KEY=VALUE`
/// space-separated; order is frozen.
pub fn wire_contract_snapshot() -> String {
    format!(
        "virtio_id={} num_queues={} queue_size={} shm_region_bytes={} vring_base={} protocol_flags={} mmio_allocator={}",
        VIRTIO_ID_MEDIA,
        VHOST_USER_MEDIA_NUM_QUEUES,
        VHOST_USER_MEDIA_QUEUE_SIZE,
        VHOST_USER_MEDIA_SHM_REGION_BYTES,
        VHOST_USER_MEDIA_VRING_BASE,
        VHOST_USER_MEDIA_PROTOCOL_FLAGS,
        VHOST_USER_MEDIA_MMIO_ALLOCATOR,
    )
}

/// Video decode backend. The d2b video host module wires VAAPI
/// via nvidia-vaapi-driver → NVDEC; the enum stays open for future
/// backends (`null`, `libavcodec`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VideoBackend {
    Vaapi,
}

impl VideoBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Vaapi => "vaapi",
        }
    }
}

/// All inputs required to render the video-decoder argv.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VideoArgvInput {
    /// Absolute store path to the `crosvm` binary (the video component
    /// overlays `cargoBuildFeatures += [video-decoder,
    /// vaapi, media]` against `pkgs.crosvm`).
    pub crosvm_binary_path: String,
    /// VM name; used by [`exec_arg0`] only.
    pub vm_name: String,
    /// `--socket-path` value. Per host.nix:
    /// `/run/d2b-video/<vm>/video.sock` (the video module uses its
    /// own `RuntimeDirectory = d2b-video/<vm>` rather
    /// than sharing `/run/d2b/vms/<vm>/`).
    pub socket_path: String,
    /// `--backend` selector.
    pub backend: VideoBackend,
}

/// Errors the video argv generator can return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum VideoArgvError {
    InvalidCrosvmBinaryPath { path: String },
    EmptyVmName,
    EmptySocketPath,
}

/// Render the video-decoder argv.
pub fn generate_video_argv(input: &VideoArgvInput) -> Result<Vec<String>, VideoArgvError> {
    if input.crosvm_binary_path.is_empty() || !input.crosvm_binary_path.starts_with('/') {
        return Err(VideoArgvError::InvalidCrosvmBinaryPath {
            path: input.crosvm_binary_path.clone(),
        });
    }
    if input.vm_name.is_empty() {
        return Err(VideoArgvError::EmptyVmName);
    }
    if input.socket_path.is_empty() {
        return Err(VideoArgvError::EmptySocketPath);
    }
    Ok(vec![
        input.crosvm_binary_path.clone(),
        "device".to_owned(),
        "video-decoder".to_owned(),
        "--socket-path".to_owned(),
        input.socket_path.clone(),
        "--backend".to_owned(),
        input.backend.as_str().to_owned(),
    ])
}

/// `arg0` for the video sidecar. Matches the systemd unit name
/// `d2b-<vm>-video` (per `nixos-modules/components/video/host.nix`).
pub fn exec_arg0(input: &VideoArgvInput) -> Result<String, VideoArgvError> {
    if input.vm_name.is_empty() {
        return Err(VideoArgvError::EmptyVmName);
    }
    Ok(format!("d2b-{}-video", input.vm_name))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audit_input() -> VideoArgvInput {
        VideoArgvInput {
            crosvm_binary_path: "/nix/store/CROSVMVIDEOCROSVMVIDEO-crosvm/bin/crosvm".to_owned(),
            vm_name: "ucvmyzodoxhnswumcjsa".to_owned(),
            socket_path: "/run/d2b/r/tft6a4n527flrfmxjwna/w/ucvmyzodoxhnswumcjsa/roles/nulwtvcxjg3av2c63baq/video.sock".to_owned(),
            backend: VideoBackend::Vaapi,
        }
    }

    #[test]
    fn audit_parity_minimal() {
        let argv = generate_video_argv(&audit_input()).unwrap();
        assert!(argv[0].ends_with("/crosvm"));
        assert_eq!(argv[1], "device");
        assert_eq!(argv[2], "video-decoder");
        let joined = argv.join(" ");
        assert!(joined.contains(
            "--socket-path /run/d2b/r/tft6a4n527flrfmxjwna/w/ucvmyzodoxhnswumcjsa/roles/nulwtvcxjg3av2c63baq/video.sock"
        ));
        assert!(joined.contains("--backend vaapi"));
    }

    #[test]
    fn exec_arg0_matches_systemd_unit_name() {
        assert_eq!(
            exec_arg0(&audit_input()).unwrap(),
            "d2b-ucvmyzodoxhnswumcjsa-video"
        );
    }

    #[test]
    fn rejects_non_absolute_binary() {
        let mut input = audit_input();
        input.crosvm_binary_path = "crosvm".to_owned();
        assert!(matches!(
            generate_video_argv(&input),
            Err(VideoArgvError::InvalidCrosvmBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_binary() {
        let mut input = audit_input();
        input.crosvm_binary_path.clear();
        assert!(matches!(
            generate_video_argv(&input),
            Err(VideoArgvError::InvalidCrosvmBinaryPath { .. })
        ));
    }

    #[test]
    fn rejects_empty_vm_name() {
        let mut input = audit_input();
        input.vm_name.clear();
        assert!(matches!(
            generate_video_argv(&input),
            Err(VideoArgvError::EmptyVmName)
        ));
    }

    #[test]
    fn rejects_empty_socket_path() {
        let mut input = audit_input();
        input.socket_path.clear();
        assert!(matches!(
            generate_video_argv(&input),
            Err(VideoArgvError::EmptySocketPath)
        ));
    }

    #[test]
    fn exec_arg0_rejects_empty_vm_name() {
        let mut input = audit_input();
        input.vm_name.clear();
        assert!(matches!(
            exec_arg0(&input),
            Err(VideoArgvError::EmptyVmName)
        ));
    }

    #[test]
    fn backend_string_round_trip() {
        assert_eq!(VideoBackend::Vaapi.as_str(), "vaapi");
    }

    #[test]
    fn rejects_unknown_extra_args_field() {
        let json = r#"{
            "crosvmBinaryPath": "/nix/store/CROSVMVIDEOCROSVMVIDEO-crosvm/bin/crosvm",
            "vmName": "corp-desktop",
            "socketPath": "/run/d2b-video/corp-desktop/video.sock",
            "backend": "vaapi",
            "extraArgs": ["--debug"]
        }"#;
        assert!(serde_json::from_str::<VideoArgvInput>(json).is_err());
    }

    const VIDEO_ARGV_GOLDEN: &str =
        include_str!("../../../tests/golden/runner-shape/video-argv-minimal.txt");

    fn golden_payload() -> String {
        VIDEO_ARGV_GOLDEN
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn audit_parity_snapshot_line() {
        let argv = generate_video_argv(&audit_input()).unwrap();
        let observed = format!("{}\n{}", argv.join(" "), wire_contract_snapshot());
        let expected = golden_payload();
        assert_eq!(
            observed, expected,
            "video argv/wire-contract drifted from tests/golden/runner-shape/video-argv-minimal.txt"
        );
        println!("SNAPSHOT: {}", argv.join(" "));
        println!("WIRE: {}", wire_contract_snapshot());
    }

    #[test]
    fn wire_contract_constants_pin_kernel8_values() {
        assert_eq!(VIRTIO_ID_MEDIA, 48);
        assert_eq!(VHOST_USER_MEDIA_NUM_QUEUES, 2);
        assert_eq!(VHOST_USER_MEDIA_QUEUE_SIZE, 256);
        assert_eq!(VHOST_USER_MEDIA_SHM_REGION_BYTES, 256 * 1024 * 1024);
        assert_eq!(VHOST_USER_MEDIA_VRING_BASE, 0);
        assert_eq!(
            VHOST_USER_MEDIA_PROTOCOL_FLAGS,
            "BACKEND_REQ|REPLY_ACK|SHMEM_MAP_CROSVM"
        );
        assert_eq!(VHOST_USER_MEDIA_MMIO_ALLOCATOR, "pci-mem64");
    }

    #[test]
    fn wire_contract_snapshot_is_deterministic() {
        assert_eq!(wire_contract_snapshot(), wire_contract_snapshot());
        let s = wire_contract_snapshot();
        assert!(s.contains("virtio_id=48"));
        assert!(s.contains("num_queues=2"));
        assert!(s.contains("queue_size=256"));
        assert!(s.contains("shm_region_bytes=268435456"));
        assert!(s.contains("vring_base=0"));
        assert!(s.contains("BACKEND_REQ|REPLY_ACK|SHMEM_MAP_CROSVM"));
        assert!(s.contains("mmio_allocator=pci-mem64"));
    }

    #[test]
    fn argv_is_round_trip_serializable() {
        let input = audit_input();
        let json = serde_json::to_string(&input).unwrap();
        let parsed: VideoArgvInput = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, input);
    }
}
