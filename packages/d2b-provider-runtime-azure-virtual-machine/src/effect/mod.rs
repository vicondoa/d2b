//! Opaque Azure ARM effect port.

use std::fmt;

use async_trait::async_trait;
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

use crate::{
    config::{AzureVmGuestSettings, DataDiskSpec},
    error::AzureVmError,
};

/// Opaque VM handle owned by the effect adapter.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AzureVmHandle(String);

impl AzureVmHandle {
    /// Construct an opaque handle from the effect adapter.
    pub fn from_core(value: impl Into<String>) -> Result<Self, AzureVmError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 128
            || value.contains('/')
            || value.contains(':')
            || !value.bytes().all(|b| b.is_ascii_graphic())
        {
            return Err(AzureVmError::InvalidOperationHandle);
        }
        Ok(Self(value))
    }

    /// Borrow the opaque handle for the effect adapter.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AzureVmHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AzureVmHandle(<opaque>)")
    }
}

/// Opaque ARM long-running-operation handle.
#[derive(Clone, PartialEq, Eq)]
pub struct AzureOperationHandle(Vec<u8>);

impl AzureOperationHandle {
    /// Construct a bounded handle at the Core effect boundary.
    pub fn from_core(bytes: impl Into<Vec<u8>>) -> Result<Self, AzureVmError> {
        let bytes = bytes.into();
        if bytes.is_empty() || bytes.len() > 256 {
            return Err(AzureVmError::InvalidOperationHandle);
        }
        Ok(Self(bytes))
    }

    /// Return a one-way digest for status and ledger joins.
    pub fn digest(&self) -> [u8; 32] {
        Sha256::digest(&self.0).into()
    }
}

impl Serialize for AzureOperationHandle {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for AzureOperationHandle {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        let bytes = STANDARD.decode(encoded).map_err(serde::de::Error::custom)?;
        Self::from_core(bytes).map_err(serde::de::Error::custom)
    }
}

impl fmt::Debug for AzureOperationHandle {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AzureOperationHandle(<opaque>)")
    }
}

/// Opaque digest of the d2b ownership tags.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct TagDigest([u8; 32]);

impl TagDigest {
    /// Construct a digest from Core.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the digest bytes for exact comparison.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Derive the expected ownership digest from the canonical tag pairs.
    pub fn from_tags(tags: &[(String, String)]) -> Self {
        let mut canonical = tags.to_vec();
        canonical.sort_unstable();
        let mut digest = Sha256::new();
        digest.update(b"d2b:azure-vm-tags:v1");
        for (key, value) in canonical {
            digest.update([0]);
            digest.update(key.as_bytes());
            digest.update([0]);
            digest.update(value.as_bytes());
        }
        Self(digest.finalize().into())
    }
}

impl fmt::Debug for TagDigest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TagDigest(<redacted>)")
    }
}

/// Observed Azure VM state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AzureVmState {
    /// No matching VM exists.
    Absent,
    /// Azure is provisioning the VM.
    Provisioning,
    /// The VM is running.
    Running,
    /// The VM is stopped.
    Stopped,
    /// Azure reported failure.
    Failed,
    /// Observation was ambiguous.
    Unknown,
}

/// ARM long-running-operation status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LroStatus {
    /// Poll again after a bounded delay.
    InProgress {
        /// Suggested delay.
        after_ms: u32,
    },
    /// The operation completed.
    Succeeded,
    /// The operation failed.
    Failed,
}

/// One-time PSK payload. It is zeroized on drop and never implements `Debug`.
pub struct PskExtensionPayload(Zeroizing<Vec<u8>>);

impl PskExtensionPayload {
    /// Construct a bounded payload from a one-time secret.
    pub fn from_secret(bytes: impl Into<Vec<u8>>) -> Result<Self, AzureVmError> {
        let mut bytes = bytes.into();
        if bytes.is_empty() || bytes.len() > 8_192 {
            bytes.zeroize();
            return Err(AzureVmError::InvalidConfiguration);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// Return the bounded payload length without exposing bytes.
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Return whether the payload is empty.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Token material delivered over an enrolled credential session.
pub type AzureAccessToken = Zeroizing<Vec<u8>>;

/// Typed Azure effect boundary.
#[async_trait]
pub trait AzureEffectPort: Send + Sync {
    /// Start VM provisioning.
    async fn start_vm_provision(
        &self,
        settings: &AzureVmGuestSettings,
        operation_id: &str,
    ) -> Result<AzureOperationHandle, AzureVmError>;
    /// Poll an opaque operation.
    async fn poll_lro(&self, operation: &AzureOperationHandle) -> Result<LroStatus, AzureVmError>;
    /// Re-derive VM state from external reality.
    async fn get_vm_state(
        &self,
        settings: &AzureVmGuestSettings,
    ) -> Result<(AzureVmState, Option<AzureVmHandle>, Option<TagDigest>), AzureVmError>;
    /// Deliver a one-time bootstrap payload.
    async fn put_vm_extension(
        &self,
        handle: &AzureVmHandle,
        payload: PskExtensionPayload,
    ) -> Result<AzureOperationHandle, AzureVmError>;
    /// Start a resize operation.
    async fn start_vm_resize(
        &self,
        handle: &AzureVmHandle,
        size: &str,
        operation_id: &str,
    ) -> Result<AzureOperationHandle, AzureVmError>;
    /// Start VM deletion.
    async fn start_vm_delete(
        &self,
        handle: &AzureVmHandle,
        operation_id: &str,
    ) -> Result<AzureOperationHandle, AzureVmError>;
    /// Attach a provider-owned data disk.
    async fn start_disk_attach(
        &self,
        handle: &AzureVmHandle,
        disk: &DataDiskSpec,
        operation_id: &str,
    ) -> Result<AzureOperationHandle, AzureVmError>;
    /// Detach a provider-owned data disk.
    async fn start_disk_detach(
        &self,
        handle: &AzureVmHandle,
        lun: u8,
        operation_id: &str,
    ) -> Result<AzureOperationHandle, AzureVmError>;
    /// Update operator tags.
    async fn update_vm_tags(
        &self,
        handle: &AzureVmHandle,
        tags: &[(String, String)],
        operation_id: &str,
    ) -> Result<AzureOperationHandle, AzureVmError>;
}

/// Typed credential delivery boundary.
#[async_trait]
pub trait AzureCredentialPort: Send + Sync {
    /// Acquire one short-lived ARM token.
    async fn acquire_token(
        &self,
        audience: &str,
        deadline_ms: u32,
    ) -> Result<AzureAccessToken, AzureVmError>;
}
