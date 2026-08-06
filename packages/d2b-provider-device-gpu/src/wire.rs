//! Frozen Cloud Hypervisor/crosvm video wire contract.

/// Virtio media device ID.
pub const VIRTIO_ID_MEDIA: u32 = 48;
/// Number of vhost-user media queues.
pub const VHOST_USER_MEDIA_NUM_QUEUES: u16 = 2;
/// Per-queue descriptor ring size.
pub const VHOST_USER_MEDIA_QUEUE_SIZE: u16 = 256;
/// Shared-memory region length.
pub const VHOST_USER_MEDIA_SHM_REGION_BYTES: u64 = 256 * 1024 * 1024;
/// Forced vring base.
pub const VHOST_USER_MEDIA_VRING_BASE: u64 = 0;
/// Negotiated protocol feature set.
pub const VHOST_USER_MEDIA_PROTOCOL_FLAGS: &str = "BACKEND_REQ|REPLY_ACK|SHMEM_MAP_CROSVM";

/// Render the byte-stable wire contract snapshot.
pub fn wire_contract_snapshot() -> String {
    format!(
        "virtio_id={} num_queues={} queue_size={} shm_region_bytes={} vring_base={} protocol_flags={}",
        VIRTIO_ID_MEDIA,
        VHOST_USER_MEDIA_NUM_QUEUES,
        VHOST_USER_MEDIA_QUEUE_SIZE,
        VHOST_USER_MEDIA_SHM_REGION_BYTES,
        VHOST_USER_MEDIA_VRING_BASE,
        VHOST_USER_MEDIA_PROTOCOL_FLAGS,
    )
}
