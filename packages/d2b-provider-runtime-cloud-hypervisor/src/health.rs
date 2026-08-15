//! Authenticated guest-control health contract.

use async_trait::async_trait;

/// Health result for the authenticated guest-control session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestControlHealth {
    /// Process and authenticated probe are ready.
    Ready,
    /// The transport/session is temporarily unavailable.
    Degraded,
    /// Authentication or protocol failed closed.
    Failed,
}

/// Stable guest-control health failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestControlHealthError {
    /// Wrong guest-control identity or CID.
    WrongIdentity,
    /// Signature or replay verification failed.
    AuthenticationFailed,
    /// Probe exceeded its deadline.
    Timeout,
    /// The wire protocol was malformed.
    Protocol,
    /// The endpoint disconnected.
    Disconnected,
}

impl GuestControlHealthError {
    /// Return the stable error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::WrongIdentity => "guest-control-wrong-identity",
            Self::AuthenticationFailed => "guest-control-authentication-failed",
            Self::Timeout => "guest-control-timeout",
            Self::Protocol => "guest-control-protocol",
            Self::Disconnected => "guest-control-disconnected",
        }
    }
}

/// Injected authenticated guest-control probe.
#[async_trait]
pub trait GuestControlProbe: Send + Sync {
    /// Probe one exact guest identity.
    async fn probe(
        &self,
        expected_cid: u32,
        deadline_ms: u32,
    ) -> Result<GuestControlHealth, GuestControlHealthError>;

    /// Close the authenticated guest-control session before VMM teardown.
    async fn close(&self, expected_cid: u32) -> Result<(), GuestControlHealthError>;
}
