//! The `virtiofs.d2bus.org.Export` resource.
//!
//! volume-local translates one virtiofs Volume attachment into one
//! Export. volume-virtiofs reconciles Exports and never writes a Volume
//! row.

use std::fmt;

use serde::Serialize;
use sha2::{Digest, Sha256};

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::volume::{
    AttachmentAccess, AttachmentSettings, AttachmentTransport, VolumeAttachment,
};

use crate::error::VirtiofsExportError;

/// The qualified ResourceType name this Provider owns.
pub const EXPORT_RESOURCE_TYPE: &str = "virtiofs.d2bus.org.Export";

/// The finalizer volume-virtiofs adds to each Export, and to nothing
/// else.
pub const EXPORT_FINALIZER: &str = "volume-virtiofs/export";

/// The opaque identity of one Export's private listening socket.
///
/// The socket path is a generated implementation detail of this
/// Provider. It is never a spec field, a status field, an audit field,
/// or CLI output. Only this digest is public, and the effect adapter
/// alone derives the private path it stands for.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SocketIdentity([u8; 32]);

impl SocketIdentity {
    /// Derive the identity of one Export's socket.
    pub fn derive(
        zone: &BoundedToken,
        volume_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(b"d2b/volume-virtiofs/export-socket/v1");
        hasher.update(zone.as_str().as_bytes());
        hasher.update([0u8]);
        hasher.update(volume_ref.to_canonical_string().as_bytes());
        hasher.update([0u8]);
        hasher.update(execution_ref.to_canonical_string().as_bytes());
        let mut bytes = [0u8; 32];
        bytes.copy_from_slice(&hasher.finalize());
        Self(bytes)
    }

    /// Render the identity as lowercase hex.
    pub fn to_hex(self) -> String {
        let mut out = String::with_capacity(64);
        for byte in self.0 {
            out.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            out.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
        }

        /// Return the eight-character tag used by the private socket filename.
        pub fn short_tag(self) -> String {
            self.to_hex()[..8].to_owned()
        }
        out
    }
}

impl fmt::Debug for SocketIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SocketIdentity(<redacted>)")
    }
}

/// One Export: exactly one Volume view served to exactly one execution
/// target.
#[derive(Clone, PartialEq, Eq)]
pub struct ExportSpec {
    volume_ref: ResourceRef,
    execution_ref: ResourceRef,
    view: BoundedToken,
    access: AttachmentAccess,
    settings: AttachmentSettings,
    mount_path: String,
}

impl ExportSpec {
    /// Construct an Export after checking both references.
    pub fn new(
        volume_ref: ResourceRef,
        execution_ref: ResourceRef,
        view: BoundedToken,
        access: AttachmentAccess,
        settings: AttachmentSettings,
    ) -> Result<Self, VirtiofsExportError> {
        if volume_ref.resource_type().as_str() != "Volume" {
            return Err(VirtiofsExportError::InvalidExport);
        }
        if !matches!(execution_ref.resource_type().as_str(), "Host" | "Guest") {
            return Err(VirtiofsExportError::InvalidExport);
        }
        Ok(Self {
            volume_ref,
            execution_ref,
            view,
            access,
            settings,
            mount_path: "/".to_owned(),
        })
    }

    /// Translate one virtiofs Volume attachment into one Export.
    ///
    /// A `virtio-blk` attachment is not this Provider's concern and is
    /// rejected rather than reinterpreted.
    pub fn from_attachment(
        volume_ref: ResourceRef,
        attachment: &VolumeAttachment,
    ) -> Result<Self, VirtiofsExportError> {
        if attachment.transport() != AttachmentTransport::Virtiofs {
            return Err(VirtiofsExportError::InvalidExport);
        }
        Self::new(
            volume_ref,
            attachment.execution_ref().clone(),
            attachment.view().clone(),
            attachment.access(),
            attachment.settings().clone(),
        )
        .map(|export| export.with_mount_path(attachment.mount_path()))
    }

    /// Borrow the Volume this Export serves.
    pub const fn volume_ref(&self) -> &ResourceRef {
        &self.volume_ref
    }

    /// Borrow the Host or Guest this Export serves.
    pub const fn execution_ref(&self) -> &ResourceRef {
        &self.execution_ref
    }

    /// Borrow the selected named view.
    pub const fn view(&self) -> &BoundedToken {
        &self.view
    }

    /// Return the admitted access level.
    pub const fn access(&self) -> AttachmentAccess {
        self.access
    }

    /// Borrow the typed base attachment options.
    pub const fn settings(&self) -> &AttachmentSettings {
        &self.settings
    }

    /// Borrow the guest-side mount path carried by the attachment.
    pub fn mount_path(&self) -> &str {
        &self.mount_path
    }

    /// Set the typed base mount path while retaining the compatibility
    /// constructor used by older callers.
    pub fn with_mount_path(mut self, mount_path: impl Into<String>) -> Self {
        self.mount_path = mount_path.into();
        self
    }

    /// Derive the stable per-Volume worker principal name.
    pub fn worker_principal(&self) -> Result<BoundedToken, VirtiofsExportError> {
        BoundedToken::parse(format!("vol-{}-vfd", self.volume_ref.name().as_str()))
            .map_err(|_| VirtiofsExportError::InvalidExport)
    }

    /// Derive this Export's private socket identity within a Zone.
    pub fn socket_identity(&self, zone: &BoundedToken) -> SocketIdentity {
        SocketIdentity::derive(zone, &self.volume_ref, &self.execution_ref)
    }
}

impl fmt::Debug for ExportSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ExportSpec(<redacted>)")
    }
}

impl Serialize for SocketIdentity {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_hex())
    }
}
