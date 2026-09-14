//! Opaque state-directory and tamper-marker contracts.

use core::fmt;

/// A Core-derived state-directory identity.
#[derive(Clone, PartialEq, Eq)]
pub struct StateDirectoryToken([u8; 32]);

impl StateDirectoryToken {
    /// Construct a token at the Core effect-adapter boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the token for equality checks at the effect boundary.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for StateDirectoryToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StateDirectoryToken(<redacted>)")
    }
}

/// A Core-derived identity-bound tamper marker.
#[derive(Clone, PartialEq, Eq)]
pub struct TamperMarkerToken([u8; 32]);

impl TamperMarkerToken {
    /// Construct a token at the Core effect-adapter boundary.
    pub const fn from_core(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Borrow the token for equality checks at the effect boundary.
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for TamperMarkerToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TamperMarkerToken(<redacted>)")
    }
}

/// An opaque owner identity for the swtpm state principal.
#[derive(Clone, PartialEq, Eq)]
pub struct StateOwnerToken([u8; 16]);

impl StateOwnerToken {
    /// Construct a token at the Core effect-adapter boundary.
    pub const fn from_core(bytes: [u8; 16]) -> Self {
        Self(bytes)
    }

    /// Borrow the owner identity for Core-side ticket binding.
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Debug for StateOwnerToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StateOwnerToken(<redacted>)")
    }
}

/// The only state-directory intent a TPM Provider may submit.
#[derive(Clone, PartialEq, Eq)]
pub struct StateDirIntent {
    directory: StateDirectoryToken,
    marker: TamperMarkerToken,
    owner: StateOwnerToken,
}

impl StateDirIntent {
    /// Construct an opaque state-directory hardening request.
    pub const fn new(
        directory: StateDirectoryToken,
        marker: TamperMarkerToken,
        owner: StateOwnerToken,
    ) -> Self {
        Self {
            directory,
            marker,
            owner,
        }
    }

    /// Borrow the state-directory identity.
    pub const fn directory(&self) -> &StateDirectoryToken {
        &self.directory
    }

    /// Borrow the identity-bound marker token.
    pub const fn marker(&self) -> &TamperMarkerToken {
        &self.marker
    }

    /// Borrow the expected state owner token.
    pub const fn owner(&self) -> &StateOwnerToken {
        &self.owner
    }
}

impl fmt::Debug for StateDirIntent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("StateDirIntent(<redacted>)")
    }
}
