//! Realm-local Relay credential delivery.

use std::fmt;

use async_trait::async_trait;
use zeroize::{Zeroize, Zeroizing};

/// Relay credential role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayCredentialRole {
    /// Listener credential.
    Listen,
    /// Sender credential.
    Send,
}

/// Bounded zeroizing secret.
pub struct RelaySecret(Zeroizing<Vec<u8>>);

impl RelaySecret {
    /// Construct a non-empty bounded secret.
    pub fn new(bytes: impl Into<Vec<u8>>) -> Result<Self, RelayCredentialError> {
        let mut bytes = bytes.into();
        if bytes.is_empty() || bytes.len() > 16 * 1024 {
            bytes.zeroize();
            return Err(RelayCredentialError::InvalidSecret);
        }
        Ok(Self(Zeroizing::new(bytes)))
    }

    /// Borrow bytes only inside the gateway effect adapter.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl Clone for RelaySecret {
    fn clone(&self) -> Self {
        Self(Zeroizing::new(self.0.to_vec()))
    }
}

impl fmt::Debug for RelaySecret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RelaySecret(<redacted>)")
    }
}

/// Secret material available only inside the gateway Guest.
pub enum RelayCredentialMaterial {
    /// SAS rule key material.
    SasRule {
        /// Rule name.
        key_name: RelaySecret,
        /// Rule key.
        key: RelaySecret,
    },
    /// A pre-minted SAS token.
    SasToken(RelaySecret),
    /// An Entra bearer.
    EntraBearer(RelaySecret),
}

impl fmt::Debug for RelayCredentialMaterial {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::SasRule { .. } => "RelayCredentialMaterial::SasRule(<redacted>)",
            Self::SasToken(_) => "RelayCredentialMaterial::SasToken(<redacted>)",
            Self::EntraBearer(_) => "RelayCredentialMaterial::EntraBearer(<redacted>)",
        })
    }
}

/// One short-lived credential lease.
pub struct RelayCredentialLease {
    material: RelayCredentialMaterial,
    role: RelayCredentialRole,
    expires_at_unix_ms: u64,
}

impl RelayCredentialLease {
    /// Construct a lease inside the credential Provider.
    pub fn new(
        material: RelayCredentialMaterial,
        role: RelayCredentialRole,
        expires_at_unix_ms: u64,
    ) -> Self {
        Self {
            material,
            role,
            expires_at_unix_ms,
        }
    }

    /// Return the lease role.
    pub const fn role(&self) -> RelayCredentialRole {
        self.role
    }

    /// Return the expiry.
    pub const fn expires_at_unix_ms(&self) -> u64 {
        self.expires_at_unix_ms
    }
}

impl fmt::Debug for RelayCredentialLease {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RelayCredentialLease")
            .field("material", &self.material)
            .field("role", &self.role)
            .field("expires_at_unix_ms", &"<redacted>")
            .finish()
    }
}

/// Credential Provider failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayCredentialError {
    /// Secret or lease data was invalid.
    InvalidSecret,
    /// No lease is available.
    Unavailable,
    /// Lease is expired.
    Expired,
    /// Lease has the wrong role.
    RoleMismatch,
}

impl fmt::Display for RelayCredentialError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidSecret => "relay-credential-invalid",
            Self::Unavailable => "relay-credential-unavailable",
            Self::Expired => "relay-credential-expired",
            Self::RoleMismatch => "relay-credential-role-mismatch",
        })
    }
}

impl std::error::Error for RelayCredentialError {}

/// Typed credential effect boundary.
#[async_trait]
pub trait RelayCredentialPort: Send + Sync {
    /// Acquire a short-lived lease for one role.
    async fn acquire(
        &self,
        role: RelayCredentialRole,
        deadline_ms: u32,
    ) -> Result<RelayCredentialLease, RelayCredentialError>;
    /// Revoke one exact lease.
    async fn revoke(&self, lease: RelayCredentialLease) -> Result<(), RelayCredentialError>;
}
