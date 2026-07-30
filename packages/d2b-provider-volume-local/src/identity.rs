//! Opaque layout identity, root-handle evidence, and owner proof.
//!
//! No host path, source policy ID, anchored entry path, numeric UID or
//! GID, or ACL value is public here. An entry is named in public status
//! only by its digest.

use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

use d2b_contracts::v3::ResourceUid;

/// The opaque public identity of one layout entry.
///
/// It is derived from the Volume UID and the anchored relative entry
/// path, so it is stable across reconciles while never disclosing the
/// path itself.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryDigest([u8; 32]);

impl EntryDigest {
    /// Derive the digest of one entry of one Volume.
    pub fn derive(volume_uid: &ResourceUid, anchored_path: &str) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"d2b/volume-local/entry/v1");
        hasher.update(volume_uid.as_str().as_bytes());
        hasher.update([0u8]);
        hasher.update(anchored_path.as_bytes());
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&hasher.finalize());
        Self(bytes)
    }

    /// Render the digest as lowercase hex.
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
        }
        out
    }
}

impl fmt::Debug for EntryDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EntryDigest(<redacted>)")
    }
}

impl Serialize for EntryDigest {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}

/// Proof that the effect adapter resolved the Volume source policy ID
/// against its private allowlist and holds the resulting root directory
/// descriptor.
///
/// The descriptor and the resolved path never reach controller code.
/// This value is deliberately not `Clone`, not `Copy`, not `Default`, not
/// `Serialize`, and carries no accessor: it is never persisted, never
/// public status, and never crosses a Zone boundary. It is dropped and
/// re-derived after a controller restart.
pub struct VolumeRootHandle {
    _private: (),
}

impl VolumeRootHandle {
    /// Record that a validated Volume root descriptor is held.
    ///
    /// Only an effect adapter calls this, immediately after it resolved
    /// the opaque source policy ID against the private allowlist policy.
    pub const fn held() -> Self {
        Self { _private: () }
    }
}

impl fmt::Debug for VolumeRootHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("VolumeRootHandle(<redacted>)")
    }
}

/// What the effect adapter could prove about the live owner of an
/// existing entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum OwnerProof {
    /// The entry declares no lease, so no owner proof is required.
    NotApplicable,
    /// A live lease was verified for the declared lease class.
    Live,
    /// The lease class was verified and the owner is gone.
    Dead,
    /// The owner could not be determined. Ambiguity quarantines.
    Unknown,
}

/// Whether a `create-if-never-provisioned` marker exists for the Volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum MarkerState {
    /// No prior provision was ever recorded.
    NeverProvisioned,
    /// A prior provision marker exists and matches the trusted record.
    Provisioned,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uid() -> ResourceUid {
        ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").expect("valid fixture uid")
    }

    #[test]
    fn entry_digests_are_stable_distinct_and_redacted() {
        let root = EntryDigest::derive(&uid(), "");
        let live = EntryDigest::derive(&uid(), "live");
        assert_eq!(root, EntryDigest::derive(&uid(), ""));
        assert_ne!(root, live);
        assert_eq!(root.to_hex().len(), 64);
        assert!(root.to_hex().bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(format!("{root:?}"), "EntryDigest(<redacted>)");
        assert!(!serde_json::to_string(&live).unwrap().contains("live"));
    }

    #[test]
    fn the_root_handle_is_opaque_in_diagnostics() {
        assert_eq!(
            format!("{:?}", VolumeRootHandle::held()),
            "VolumeRootHandle(<redacted>)"
        );
    }
}
