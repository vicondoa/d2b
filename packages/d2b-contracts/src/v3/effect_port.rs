//! Neutral opaque Volume effect-port contracts.

use core::fmt;
use std::future::Future;

use serde::{Deserialize, Serialize};

use crate::types::BundleOpId;

/// Maximum serialized opaque effect-id length.
pub const MAX_VOLUME_EFFECT_WIRE_ID_BYTES: usize = 128;

macro_rules! opaque_id {
    ($name:ident) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            /// Mint an ID at the trusted Core boundary.
            pub fn from_core(value: impl Into<String>) -> Result<Self, VolumeEffectIdError> {
                Self::try_from(value.into())
            }
        }

        impl TryFrom<String> for $name {
            type Error = VolumeEffectIdError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                if value.is_empty()
                    || value.len() > MAX_VOLUME_EFFECT_WIRE_ID_BYTES
                    || !value.bytes().all(|byte| byte.is_ascii_graphic())
                {
                    return Err(VolumeEffectIdError);
                }
                Ok(Self(value))
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                Self::try_from(String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($name), "([redacted])"))
            }
        }
    };
}

opaque_id!(VolumeId);
opaque_id!(SourcePolicyId);
opaque_id!(LayoutEntryId);
opaque_id!(UserId);
opaque_id!(ViewId);
opaque_id!(SealingPolicyId);

/// Invalid opaque effect ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeEffectIdError;

impl fmt::Display for VolumeEffectIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("volume-effect-id-invalid")
    }
}

impl std::error::Error for VolumeEffectIdError {}

/// Access requested for an opaque Volume mount token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccessClass {
    ReadOnly,
    ReadWrite,
}

/// Marker observation returned by the trusted adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkerStatus {
    NeverProvisioned,
    Verified,
    Missing,
    Replaced,
    Tampered,
}

/// Layout-entry provisioning result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProvisionOutcome {
    Created,
    AlreadyPresent,
    Reconciled,
}

/// Layout-entry repair result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepairOutcome {
    Unchanged,
    Repaired,
    Quarantined,
}

/// Cleanup trigger supplied by Core's dependency graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CleanupTrigger {
    DesiredStateRemoval,
    ProcessExitWithProof,
    VmStopWithProof,
}

/// Quota-capacity result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QuotaCapacityStatus {
    Enforceable,
    Unenforceable,
}

/// Bounded quota usage projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaUsage {
    /// Bytes currently consumed.
    pub bytes: u64,
    /// Inodes currently consumed.
    pub inodes: u64,
}

/// Store-view sync result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum StoreSyncOutcome {
    Complete,
    AlreadyCurrent,
}

/// Opaque mount authorization handle.
pub struct VolumeMountToken {
    volume: VolumeId,
    view: ViewId,
    access: AccessClass,
    token: String,
}

impl VolumeMountToken {
    /// Mint a token at the Core adapter boundary.
    pub fn from_core(
        volume: VolumeId,
        view: ViewId,
        access: AccessClass,
        token: impl Into<String>,
    ) -> Result<Self, VolumeEffectIdError> {
        let token = token.into();
        if token.is_empty()
            || token.len() > MAX_VOLUME_EFFECT_WIRE_ID_BYTES
            || !token.bytes().all(|byte| byte.is_ascii_graphic())
        {
            return Err(VolumeEffectIdError);
        }
        Ok(Self {
            volume,
            view,
            access,
            token,
        })
    }

    /// Borrow the selected Volume.
    pub const fn volume(&self) -> &VolumeId {
        &self.volume
    }

    /// Borrow the selected view.
    pub const fn view(&self) -> &ViewId {
        &self.view
    }

    /// Return the admitted access class.
    pub const fn access(&self) -> AccessClass {
        self.access
    }

    /// Whether Core attached a non-empty authorization token.
    pub fn is_bound(&self) -> bool {
        !self.token.is_empty()
    }
}

impl fmt::Debug for VolumeMountToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VolumeMountToken([redacted])")
    }
}

/// Closed effect failure. No variant carries backend paths or identities.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectError {
    Unauthorized,
    MarkerFailed,
    QuotaUnavailable,
    PathSafetyViolation,
    Conflict,
    Transient,
    BackendUnavailable,
}

impl EffectError {
    /// Stable error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Unauthorized => "volume-effect-unauthorized",
            Self::MarkerFailed => "volume-effect-marker-failed",
            Self::QuotaUnavailable => "volume-effect-quota-unavailable",
            Self::PathSafetyViolation => "volume-effect-path-safety-violation",
            Self::Conflict => "volume-effect-conflict",
            Self::Transient => "volume-effect-transient",
            Self::BackendUnavailable => "volume-effect-backend-unavailable",
        }
    }
}

impl fmt::Display for EffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for EffectError {}

/// Canonical sealing-key rotation request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RotateSealingKeyRequest {
    pub volume: VolumeId,
    pub policy: SealingPolicyId,
    pub expected_volume_generation: u64,
    pub expected_resource_revision: u64,
    pub expected_current_key_generation: u64,
    pub target_key_generation: u64,
    pub operation_id: BundleOpId,
}

/// Sealing-key rotation disposition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RotateSealingKeyDisposition {
    Rotated,
    AlreadyCommitted,
    RecoveredCommitted,
}

/// Successful sealing-key rotation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RotateSealingKeyResult {
    pub disposition: RotateSealingKeyDisposition,
    pub volume_generation: u64,
    pub active_key_generation: u64,
}

/// Typed Volume effect boundary.
pub trait VolumeEffectPort: Send + Sync + 'static {
    fn provision_layout_entry(
        &self,
        volume: VolumeId,
        entry: LayoutEntryId,
        owner: UserId,
        group: UserId,
    ) -> impl Future<Output = Result<ProvisionOutcome, EffectError>> + Send;

    fn repair_layout_entry(
        &self,
        volume: VolumeId,
        entry: LayoutEntryId,
        owner: UserId,
        group: UserId,
    ) -> impl Future<Output = Result<RepairOutcome, EffectError>> + Send;

    fn cleanup_layout_entry(
        &self,
        volume: VolumeId,
        entry: LayoutEntryId,
        trigger: CleanupTrigger,
    ) -> impl Future<Output = Result<(), EffectError>> + Send;

    fn verify_marker(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<MarkerStatus, EffectError>> + Send;

    fn provision_marker(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send;

    fn cleanup_marker(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send;

    fn check_quota_capacity(
        &self,
        volume: VolumeId,
        max_bytes: u64,
        max_inodes: Option<u64>,
    ) -> impl Future<Output = Result<QuotaCapacityStatus, EffectError>> + Send;

    fn poll_quota_usage(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<QuotaUsage, EffectError>> + Send;

    fn provision_block_image(
        &self,
        volume: VolumeId,
        policy: SourcePolicyId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send;

    fn mount_tmpfs(
        &self,
        volume: VolumeId,
        policy: SourcePolicyId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send;

    fn umount_tmpfs(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send;

    fn run_store_sync(
        &self,
        volume: VolumeId,
        generation: u64,
    ) -> impl Future<Output = Result<StoreSyncOutcome, EffectError>> + Send;

    fn rotate_sealing_key(
        &self,
        request: RotateSealingKeyRequest,
    ) -> impl Future<Output = Result<RotateSealingKeyResult, EffectError>> + Send;

    fn request_mount_token(
        &self,
        volume: VolumeId,
        view: ViewId,
        access: AccessClass,
    ) -> impl Future<Output = Result<VolumeMountToken, EffectError>> + Send;

    fn cleanup_volume_root(
        &self,
        volume: VolumeId,
    ) -> impl Future<Output = Result<(), EffectError>> + Send;
}
