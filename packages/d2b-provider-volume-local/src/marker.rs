//! Identity-bound provisioning marker protocol.
//!
//! Markers live under a broker-maintained root outside the Volume tree. A
//! marker that outlives its Volume root is evidence of loss, not permission to
//! recreate empty state. A replaced root likewise fails closed.

use std::fmt;

use d2b_contracts::v3::{
    MarkerStatus, ResourceUid, SchemaFingerprint, SchemaVersion, VolumeStateSchemaId,
};
use serde::{Deserialize, Serialize};

/// Current marker payload version.
pub const MARKER_VERSION: u32 = 1;

/// Filesystem identity of an opened Volume root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VolumeRootIdentity {
    /// Filesystem device number observed from the held root descriptor.
    pub device: u64,
    /// Inode number observed from the held root descriptor.
    pub inode: u64,
}

/// Marker fields bound at first provision.
#[derive(Clone, PartialEq, Eq)]
pub struct MarkerBinding {
    volume_uid: ResourceUid,
    root: VolumeRootIdentity,
    schema_id: VolumeStateSchemaId,
    installed_schema_version: SchemaVersion,
    schema_digest: SchemaFingerprint,
}

impl MarkerBinding {
    /// Bind a marker to one Volume, opened root identity, and installed schema.
    pub const fn new(
        volume_uid: ResourceUid,
        root: VolumeRootIdentity,
        schema_id: VolumeStateSchemaId,
        installed_schema_version: SchemaVersion,
        schema_digest: SchemaFingerprint,
    ) -> Self {
        Self {
            volume_uid,
            root,
            schema_id,
            installed_schema_version,
            schema_digest,
        }
    }

    /// Borrow the immutable Volume identity.
    pub const fn volume_uid(&self) -> &ResourceUid {
        &self.volume_uid
    }
}

impl fmt::Debug for MarkerBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("MarkerBinding(<redacted>)")
    }
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct MarkerDocument {
    version: u32,
    volume_uid: ResourceUid,
    device: u64,
    inode: u64,
    schema_id: VolumeStateSchemaId,
    installed_schema_version: SchemaVersion,
    schema_digest: SchemaFingerprint,
}

impl MarkerDocument {
    fn from_binding(binding: &MarkerBinding) -> Self {
        Self {
            version: MARKER_VERSION,
            volume_uid: binding.volume_uid.clone(),
            device: binding.root.device,
            inode: binding.root.inode,
            schema_id: binding.schema_id.clone(),
            installed_schema_version: binding.installed_schema_version,
            schema_digest: binding.schema_digest.clone(),
        }
    }

    fn matches(&self, binding: &MarkerBinding) -> bool {
        self.version == MARKER_VERSION
            && self.volume_uid == binding.volume_uid
            && self.device == binding.root.device
            && self.inode == binding.root.inode
            && self.schema_id == binding.schema_id
            && self.installed_schema_version == binding.installed_schema_version
            && self.schema_digest == binding.schema_digest
    }
}

/// Result of reconciling the external marker and live Volume root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerDisposition {
    /// Neither root nor marker exists. First provision may proceed.
    Unprovisioned,
    /// The marker and root identity match.
    Verified,
}

/// A fail-closed marker failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarkerError {
    /// A marker exists but its previously provisioned Volume root is missing.
    PreviouslyProvisionedStateMissing,
    /// A live Volume root exists but the external marker is absent.
    MarkerMissing,
    /// The root identity no longer matches its first-provision marker.
    RootReplaced,
    /// The marker file is malformed, has unsafe metadata, or fails its binding.
    MarkerInvalid,
    /// Exclusive marker creation failed.
    MarkerWriteFailed,
    /// The marker backend failed to inspect the trusted root.
    MarkerReadFailed,
}

impl MarkerError {
    /// Return the stable, path-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::PreviouslyProvisionedStateMissing => {
                "previously-provisioned-volume-state-missing"
            }
            Self::MarkerMissing => "volume-marker-missing",
            Self::RootReplaced => "volume-marker-root-replaced",
            Self::MarkerInvalid => "volume-marker-invalid",
            Self::MarkerWriteFailed => "volume-marker-write-failed",
            Self::MarkerReadFailed => "volume-marker-read-failed",
        }
    }

    /// Return the public marker observation for status projection.
    pub const fn status(self) -> MarkerStatus {
        match self {
            Self::PreviouslyProvisionedStateMissing | Self::MarkerMissing => MarkerStatus::Missing,
            Self::RootReplaced => MarkerStatus::Replaced,
            Self::MarkerInvalid | Self::MarkerWriteFailed | Self::MarkerReadFailed => {
                MarkerStatus::Unknown
            }
        }
    }
}

impl fmt::Display for MarkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for MarkerError {}

/// Marker bytes plus adapter-verified regular-file posture.
pub struct VerifiedMarkerFile {
    bytes: Vec<u8>,
}

impl VerifiedMarkerFile {
    /// Issue verified marker bytes from the broker-maintained marker adapter.
    ///
    /// The adapter must establish a regular file, trusted owner, mode `0600`,
    /// no-follow open, and held-fd metadata recheck before calling this.
    pub fn from_verified_regular_file(bytes: Vec<u8>) -> Self {
        Self { bytes }
    }
}

impl fmt::Debug for VerifiedMarkerFile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("VerifiedMarkerFile(<redacted>)")
    }
}

/// Broker-maintained marker storage over anchored descriptors.
pub trait MarkerStore {
    /// Inspect the trusted marker, returning only adapter-verified bytes.
    fn read_marker(
        &mut self,
        volume_uid: &ResourceUid,
    ) -> Result<Option<VerifiedMarkerFile>, MarkerError>;

    /// Write a marker with exclusive create, mode `0600`, file fsync, and
    /// marker-parent fsync. Existing markers must never be overwritten.
    fn create_marker_exclusive(
        &mut self,
        volume_uid: &ResourceUid,
        bytes: &[u8],
    ) -> Result<(), MarkerError>;
}

/// Reconcile marker and root presence before any provision or mount.
pub fn verify_marker<S: MarkerStore>(
    store: &mut S,
    root: Option<VolumeRootIdentity>,
    expected: &MarkerBinding,
) -> Result<MarkerDisposition, MarkerError> {
    let marker = store.read_marker(expected.volume_uid())?;
    match (root, marker) {
        (None, None) => Ok(MarkerDisposition::Unprovisioned),
        (None, Some(_)) => Err(MarkerError::PreviouslyProvisionedStateMissing),
        (Some(_), None) => Err(MarkerError::MarkerMissing),
        (Some(root), Some(marker)) => {
            let document: MarkerDocument =
                serde_json::from_slice(&marker.bytes).map_err(|_| MarkerError::MarkerInvalid)?;
            if document.device != root.device || document.inode != root.inode {
                return Err(MarkerError::RootReplaced);
            }
            if !document.matches(expected) {
                return Err(MarkerError::MarkerInvalid);
            }
            Ok(MarkerDisposition::Verified)
        }
    }
}

/// Write the first-provision marker after the Volume root is fully durable.
pub fn provision_marker<S: MarkerStore>(
    store: &mut S,
    binding: &MarkerBinding,
) -> Result<(), MarkerError> {
    if store.read_marker(binding.volume_uid())?.is_some() {
        return Err(MarkerError::MarkerWriteFailed);
    }
    let document = MarkerDocument::from_binding(binding);
    let bytes = serde_json::to_vec(&document).map_err(|_| MarkerError::MarkerWriteFailed)?;
    store.create_marker_exclusive(binding.volume_uid(), &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemoryStore {
        marker: Option<Vec<u8>>,
    }

    impl MarkerStore for MemoryStore {
        fn read_marker(
            &mut self,
            _volume_uid: &ResourceUid,
        ) -> Result<Option<VerifiedMarkerFile>, MarkerError> {
            Ok(self
                .marker
                .clone()
                .map(VerifiedMarkerFile::from_verified_regular_file))
        }

        fn create_marker_exclusive(
            &mut self,
            _volume_uid: &ResourceUid,
            bytes: &[u8],
        ) -> Result<(), MarkerError> {
            if self.marker.is_some() {
                return Err(MarkerError::MarkerWriteFailed);
            }
            self.marker = Some(bytes.to_vec());
            Ok(())
        }
    }

    fn binding(root: VolumeRootIdentity) -> MarkerBinding {
        MarkerBinding::new(
            ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").unwrap(),
            root,
            VolumeStateSchemaId::parse("example-provider.d2bus.org/controller/main-state").unwrap(),
            SchemaVersion::new(1, 0).unwrap(),
            SchemaFingerprint::parse(format!("sha256:{}", "1".repeat(64))).unwrap(),
        )
    }

    #[test]
    fn marker_outliving_a_missing_root_fails_closed() {
        let root = VolumeRootIdentity {
            device: 7,
            inode: 11,
        };
        let expected = binding(root);
        let mut store = MemoryStore::default();
        provision_marker(&mut store, &expected).unwrap();
        assert_eq!(
            verify_marker(&mut store, None, &expected),
            Err(MarkerError::PreviouslyProvisionedStateMissing)
        );
    }

    #[test]
    fn correct_owner_empty_replacement_fails_identity_check() {
        let original = VolumeRootIdentity {
            device: 7,
            inode: 11,
        };
        let expected = binding(original);
        let mut store = MemoryStore::default();
        provision_marker(&mut store, &expected).unwrap();
        assert_eq!(
            verify_marker(
                &mut store,
                Some(VolumeRootIdentity {
                    device: 7,
                    inode: 12,
                }),
                &expected,
            ),
            Err(MarkerError::RootReplaced)
        );
    }

    #[test]
    fn tampered_marker_schema_binding_fails_closed() {
        let root = VolumeRootIdentity {
            device: 7,
            inode: 11,
        };
        let expected = binding(root);
        let mut store = MemoryStore::default();
        provision_marker(&mut store, &expected).unwrap();
        let marker = store.marker.as_mut().unwrap();
        let mut value: serde_json::Value = serde_json::from_slice(marker).unwrap();
        value["installedSchemaVersion"] = serde_json::json!("2.0");
        *marker = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            verify_marker(&mut store, Some(root), &expected),
            Err(MarkerError::MarkerInvalid)
        );
    }
}
