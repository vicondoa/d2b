use std::fmt;

use d2b_contracts::v2_component_session::{
    BootstrapPskBinding, BootstrapPskState, HandshakeRejectReason, OperationId, SessionErrorCode,
};

use crate::{Result, SessionError};

pub struct Secret32([u8; 32]);

impl Secret32 {
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
        self.0.fill(0);
    }
}

pub struct BootstrapPsk(Secret32);

impl BootstrapPsk {
    pub fn new(bytes: [u8; 32]) -> Result<Self> {
        Secret32::new(bytes).map(Self)
    }
}

impl fmt::Debug for BootstrapPsk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BootstrapPsk(<redacted>)")
    }
}

pub struct AdmittedBootstrapPsk(BootstrapPsk);

impl AdmittedBootstrapPsk {
    pub(crate) fn expose(&self) -> &[u8; 32] {
        self.0.0.expose()
    }
}

impl fmt::Debug for AdmittedBootstrapPsk {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AdmittedBootstrapPsk(<redacted>)")
    }
}

pub struct BootstrapAdmission {
    state: BootstrapPskState,
    psk: Option<BootstrapPsk>,
}

impl BootstrapAdmission {
    pub fn new(binding: BootstrapPskBinding, psk: BootstrapPsk) -> Result<Self> {
        Ok(Self {
            state: BootstrapPskState::new(binding)?,
            psk: Some(psk),
        })
    }

    pub fn consume(
        &mut self,
        operation_id: &OperationId,
        replay_nonce: &[u8; 32],
        now_unix_ms: u64,
    ) -> Result<AdmittedBootstrapPsk> {
        self.state
            .admit(operation_id, replay_nonce, now_unix_ms)
            .map_err(SessionError::from)?;
        self.psk
            .take()
            .map(AdmittedBootstrapPsk)
            .ok_or_else(|| SessionError::from(HandshakeRejectReason::BootstrapReplayed))
    }

    pub fn is_consumed(&self) -> bool {
        self.state.is_consumed()
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
