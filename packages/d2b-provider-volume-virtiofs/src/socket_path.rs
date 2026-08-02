//! Private per-Export virtiofs socket identity and path derivation.
//!
//! The path is created only at the effect boundary.  The controller and
//! status projections carry an opaque identity instead of this value.

use std::fmt;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::execution_policy::BoundedToken;
use sha2::{Digest, Sha256};

/// Linux's maximum Unix-domain socket path length.
pub const MAX_SOCKET_PATH_BYTES: usize = 108;

/// A private, deterministic socket path.
#[derive(Clone, PartialEq, Eq)]
pub struct PrivateSocketPath {
    rendered: String,
    tag: String,
}

impl PrivateSocketPath {
    /// Derive a path from the Zone runtime root and the canonical resource
    /// identities.  No caller-provided Volume path is accepted.
    pub fn derive(
        zone_runtime_dir: &str,
        zone: &BoundedToken,
        volume_ref: &ResourceRef,
        execution_ref: &ResourceRef,
    ) -> Result<Self, SocketPathError> {
        if !zone_runtime_dir.starts_with('/')
            || zone_runtime_dir.ends_with('/')
            || zone_runtime_dir.contains('\0')
        {
            return Err(SocketPathError::InvalidRuntimeRoot);
        }
        if volume_ref.resource_type().as_str() != "Volume" {
            return Err(SocketPathError::InvalidResource);
        }
        if !matches!(execution_ref.resource_type().as_str(), "Guest" | "Host") {
            return Err(SocketPathError::InvalidResource);
        }

        let mut hasher = Sha256::new();
        hasher.update(zone.as_str().as_bytes());
        hasher.update([0]);
        hasher.update(volume_ref.name().as_str().as_bytes());
        hasher.update([0]);
        hasher.update(execution_ref.name().as_str().as_bytes());
        let digest = hasher.finalize();
        let mut tag = String::with_capacity(8);
        for byte in digest[..4].iter().copied() {
            tag.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
            tag.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
        }
        let rendered = format!(
            "{zone_runtime_dir}/vms/{}/vol-{tag}.vfd.sock",
            execution_ref.name().as_str()
        );
        if rendered.len() > MAX_SOCKET_PATH_BYTES {
            return Err(SocketPathError::TooLong);
        }
        Ok(Self { rendered, tag })
    }

    /// Borrow the short public identity, not the private path.
    pub fn tag(&self) -> &str {
        &self.tag
    }

    /// Return the private path length for the kernel-boundary assertion.
    pub fn byte_len(&self) -> usize {
        self.rendered.len()
    }

    pub(crate) fn as_private_str(&self) -> &str {
        &self.rendered
    }
}

impl fmt::Debug for PrivateSocketPath {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("PrivateSocketPath(<redacted>)")
    }
}

/// Closed path-derivation failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocketPathError {
    /// The runtime root was not an absolute, anchored directory.
    InvalidRuntimeRoot,
    /// The resource references were not Volume and Host/Guest identities.
    InvalidResource,
    /// The derived path exceeds the Unix socket limit.
    TooLong,
}

impl SocketPathError {
    /// Return the stable, path-free error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidRuntimeRoot => "virtiofs-socket-runtime-root-invalid",
            Self::InvalidResource => "virtiofs-socket-resource-invalid",
            Self::TooLong => "virtiofs-socket-path-too-long",
        }
    }
}

impl fmt::Display for SocketPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for SocketPathError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs() -> (BoundedToken, ResourceRef, ResourceRef) {
        (
            BoundedToken::parse("dev").unwrap(),
            ResourceRef::parse("Volume/work-state").unwrap(),
            ResourceRef::parse("Guest/work-vm").unwrap(),
        )
    }

    #[test]
    fn socket_identity_is_stable_short_and_bounded() {
        let (zone, volume, guest) = refs();
        let first = PrivateSocketPath::derive("/run/d2b", &zone, &volume, &guest).unwrap();
        let second = PrivateSocketPath::derive("/run/d2b", &zone, &volume, &guest).unwrap();
        assert_eq!(first, second);
        assert_eq!(first.tag().len(), 8);
        assert!(first.byte_len() <= MAX_SOCKET_PATH_BYTES);
        assert_eq!(format!("{first:?}"), "PrivateSocketPath(<redacted>)");
    }

    #[test]
    fn a_runtime_root_that_would_escape_the_socket_namespace_is_rejected() {
        let (zone, volume, guest) = refs();
        assert_eq!(
            PrivateSocketPath::derive("run/d2b", &zone, &volume, &guest),
            Err(SocketPathError::InvalidRuntimeRoot)
        );
    }
}
