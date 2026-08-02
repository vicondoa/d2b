use d2b_provider_device_gpu::{
    VHOST_USER_MEDIA_NUM_QUEUES, VHOST_USER_MEDIA_PROTOCOL_FLAGS, VHOST_USER_MEDIA_QUEUE_SIZE,
    VHOST_USER_MEDIA_SHM_REGION_BYTES, VHOST_USER_MEDIA_VRING_BASE, VIRTIO_ID_MEDIA,
    wire_contract_snapshot,
};

#[test]
fn media_wire_constants_are_frozen() {
    assert_eq!(VIRTIO_ID_MEDIA, 48);
    assert_eq!(VHOST_USER_MEDIA_NUM_QUEUES, 2);
    assert_eq!(VHOST_USER_MEDIA_QUEUE_SIZE, 256);
    assert_eq!(VHOST_USER_MEDIA_SHM_REGION_BYTES, 268_435_456);
    assert_eq!(VHOST_USER_MEDIA_VRING_BASE, 0);
    assert_eq!(
        VHOST_USER_MEDIA_PROTOCOL_FLAGS,
        "BACKEND_REQ|REPLY_ACK|SHMEM_MAP_CROSVM"
    );
    assert_eq!(
        wire_contract_snapshot(),
        "virtio_id=48 num_queues=2 queue_size=256 shm_region_bytes=268435456 vring_base=0 protocol_flags=BACKEND_REQ|REPLY_ACK|SHMEM_MAP_CROSVM"
    );
}
