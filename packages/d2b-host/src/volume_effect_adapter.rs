//! Host-runtime adapter for the neutral Volume effect port.
//!
//! The adapter deliberately carries no path, UID, GID, socket, or broker wire
//! field. A Zone runtime supplies the broker-backed implementation of the
//! neutral port; this wrapper is the sole host-side composition point.

use std::future::Future;

use d2b_contracts::v3::effect_port::{
    AccessClass, CleanupTrigger, EffectError, LayoutEntryId, MarkerStatus, ProvisionOutcome,
    QuotaCapacityStatus, QuotaUsage, RepairOutcome, RotateSealingKeyRequest,
    RotateSealingKeyResult, SourcePolicyId, StoreSyncOutcome, UserId, ViewId, VolumeEffectPort,
    VolumeId, VolumeMountToken,
};

/// Host-side composition wrapper for a broker-backed Volume effect port.
#[derive(Debug)]
pub struct VolumeEffectAdapter<B> {
    backend: B,
}

impl<B> VolumeEffectAdapter<B> {
    /// Bind the adapter to the trusted Zone backend.
    pub const fn new(backend: B) -> Self {
        Self { backend }
    }

    /// Borrow the bound backend.
    pub const fn backend(&self) -> &B {
        &self.backend
    }
}

impl<B: VolumeEffectPort> VolumeEffectPort for VolumeEffectAdapter<B> {
    fn provision_layout_entry(
        &self,
        volume: VolumeId,
        entry: LayoutEntryId,
        owner: UserId,
        group: UserId,
    ) -> impl Future<Output = Result<ProvisionOutcome, EffectError>> + Send {
        self.backend
            .provision_layout_entry(volume, entry, owner, group)
    }

    fn repair_layout_entry(
        &self,
        volume: VolumeId,
        entry: LayoutEntryId,
        owner: UserId,
        group: UserId,
    ) -> impl Future<Output = Result<RepairOutcome, EffectError>> + Send {
        self.backend
            .repair_layout_entry(volume, entry, owner, group)
    }

    fn cleanup_layout_entry(
        &self,
        volume: VolumeId,
        entry: LayoutEntryId,
        trigger: CleanupTrigger,
    ) -> impl Future<Output = Result<(), EffectError>> + Send {
        self.backend.cleanup_layout_entry(volume, entry, trigger)
    }

    fn verify_marker(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<MarkerStatus, EffectError>> + Send {
        self.backend.verify_marker(volume)
    }

    fn provision_marker(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send {
        self.backend.provision_marker(volume)
    }

    fn cleanup_marker(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send {
        self.backend.cleanup_marker(volume)
    }

    fn check_quota_capacity(
        &self,
        volume: VolumeId,
        max_bytes: u64,
        max_inodes: Option<u64>,
    ) -> impl Future<Output = Result<QuotaCapacityStatus, EffectError>> + Send {
        self.backend
            .check_quota_capacity(volume, max_bytes, max_inodes)
    }

    fn poll_quota_usage(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<QuotaUsage, EffectError>> + Send {
        self.backend.poll_quota_usage(volume)
    }

    fn provision_block_image(
        &self,
        volume: VolumeId,
        policy: SourcePolicyId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send {
        self.backend.provision_block_image(volume, policy)
    }

    fn mount_tmpfs(
        &self,
        volume: VolumeId,
        policy: SourcePolicyId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send {
        self.backend.mount_tmpfs(volume, policy)
    }

    fn umount_tmpfs(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send {
        self.backend.umount_tmpfs(volume)
    }

    fn run_store_sync(
        &self,
        volume: VolumeId,
        generation: u64,
    ) -> impl Future<Output = Result<StoreSyncOutcome, EffectError>> + Send {
        self.backend.run_store_sync(volume, generation)
    }

    fn rotate_sealing_key(
        &self,
        request: RotateSealingKeyRequest,
    ) -> impl Future<Output = Result<RotateSealingKeyResult, EffectError>> + Send {
        self.backend.rotate_sealing_key(request)
    }

    fn request_mount_token(
        &self,
        volume: VolumeId,
        view: ViewId,
        access: AccessClass,
    ) -> impl Future<Output = Result<VolumeMountToken, EffectError>> + Send {
        self.backend.request_mount_token(volume, view, access)
    }

    fn cleanup_volume_root(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send {
        self.backend.cleanup_volume_root(volume)
    }
}
