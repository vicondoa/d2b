use std::fmt;

use d2b_contracts_zone_session::v3::component_session::{
    BootstrapIdentityBinding, BootstrapPskBinding, HandshakeRejectReason, OperationId,
    SessionErrorCode,
};
use zeroize::Zeroize;

use crate::{Result, SessionError};

/// A zeroized 32-byte secret.
pub struct Secret32([u8; 32]);

impl Secret32 {
    /// Construct a secret, rejecting the all-zero value.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::AuthenticationFailed`] when `bytes` is all zero.
    pub fn new(bytes: [u8; 32]) -> Result<Self> {
        if bytes == [0; 32] {
            return Err(SessionError::new(SessionErrorCode::AuthenticationFailed));
        }
        Ok(Self(bytes))
    }

    pub(crate) fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for Secret32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Secret32(<redacted>)")
    }
}

impl Drop for Secret32 {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// A bootstrap pre-shared key.
pub struct BootstrapPsk(Secret32);

impl BootstrapPsk {
    /// Construct a bootstrap PSK, rejecting the all-zero value.
    ///
    /// # Errors
    ///
    /// Returns [`SessionErrorCode::AuthenticationFailed`] when `bytes` is all zero.
    pub fn new(bytes: [u8; 32]) -> Result<Self> {
        Secret32::new(bytes).map(Self)
    }
}

impl fmt::Debug for BootstrapPsk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BootstrapPsk(<redacted>)")
    }
}

/// A bootstrap PSK admitted against a matching identity binding.
pub struct AdmittedBootstrapPsk {
    psk: BootstrapPsk,
    identity: BootstrapIdentityBinding,
}

impl AdmittedBootstrapPsk {
    pub(crate) fn expose(&self) -> &[u8; 32] {
        self.psk.0.expose()
    }

    pub(crate) fn into_identity(self) -> BootstrapIdentityBinding {
        self.identity
    }
}

impl fmt::Debug for AdmittedBootstrapPsk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AdmittedBootstrapPsk(<redacted>)")
    }
}

/// A single-use bootstrap admission bound to an operation, nonce, and identity.
pub struct BootstrapAdmission {
    binding: BootstrapPskBinding,
    psk: Option<BootstrapPsk>,
}

impl BootstrapAdmission {
    /// Construct an admission for one bootstrap operation.
    ///
    /// # Errors
    ///
    /// Returns the binding validation error when `binding` is invalid.
    pub fn new(binding: BootstrapPskBinding, psk: BootstrapPsk) -> Result<Self> {
        binding.validate().map_err(SessionError::from)?;
        Ok(Self {
            binding,
            psk: Some(psk),
        })
    }

    /// Consume the admission, releasing the PSK when the proof matches.
    ///
    /// # Errors
    ///
    /// Returns [`HandshakeRejectReason::BootstrapOperationMismatch`] when the
    /// operation, nonce, or identity does not match, [`HandshakeRejectReason::BootstrapExpired`]
    /// when the admission expired, and [`HandshakeRejectReason::BootstrapReplayed`]
    /// when the admission was already consumed.
    pub fn consume(
        &mut self,
        operation_id: &OperationId,
        replay_nonce: &[u8; 32],
        identity: BootstrapIdentityBinding,
        now_unix_ms: u64,
    ) -> Result<AdmittedBootstrapPsk> {
        if operation_id != &self.binding.operation_id
            || replay_nonce != &self.binding.replay_nonce
            || identity != self.binding.identity
        {
            return Err(SessionError::from(
                HandshakeRejectReason::BootstrapOperationMismatch,
            ));
        }
        if now_unix_ms >= self.binding.expires_at_unix_ms {
            return Err(SessionError::from(HandshakeRejectReason::BootstrapExpired));
        }
        self.psk
            .take()
            .map(|psk| AdmittedBootstrapPsk { psk, identity })
            .ok_or_else(|| SessionError::from(HandshakeRejectReason::BootstrapReplayed))
    }

    /// Return whether the admission was already consumed.
    pub fn is_consumed(&self) -> bool {
        self.psk.is_none()
    }
}

impl fmt::Debug for BootstrapAdmission {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BootstrapAdmission")
            .field("consumed", &self.is_consumed())
            .field("psk", &"<redacted>")
            .finish()
    }
}
