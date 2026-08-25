//! Authenticated component-session health contract.

use std::fmt;

use async_trait::async_trait;
use d2b_contracts_resource::v3::ResourceRef;

/// Health result for the authenticated component-session session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestSessionHealth {
    /// Process and authenticated probe are ready.
    Ready,
    /// The transport/session is temporarily unavailable.
    Degraded,
    /// Authentication or protocol failed closed.
    Failed,
}

/// Redacted evidence produced by an authenticated Guest ComponentSession.
///
/// The evidence binds the Guest, boot identity, reconnect generation,
/// capabilities, and controller readiness together.
#[derive(Clone, PartialEq, Eq)]
pub struct GuestSessionEvidence {
    guest_ref: Option<ResourceRef>,
    boot_identity_digest: Option<String>,
    reconnect_generation: Option<u64>,
    capabilities: Vec<String>,
    controller_ready: bool,
    endpoint_ready: bool,
    health: GuestSessionHealth,
}

impl GuestSessionEvidence {
    /// Construct current evidence from an authenticated ComponentSession.
    pub fn current(
        guest_ref: ResourceRef,
        boot_identity_digest: impl Into<String>,
        reconnect_generation: u64,
        capabilities: impl IntoIterator<Item = String>,
        controller_ready: bool,
        endpoint_ready: bool,
    ) -> Result<Self, GuestSessionError> {
        let boot_identity_digest = boot_identity_digest.into();
        if guest_ref.resource_type().as_str() != "Guest"
            || guest_ref.name().as_str().is_empty()
            || reconnect_generation == 0
            || !valid_digest(&boot_identity_digest)
        {
            return Err(GuestSessionError::AuthenticationFailed);
        }
        let capabilities = validate_capabilities(capabilities)?;
        let health = if controller_ready && endpoint_ready {
            GuestSessionHealth::Ready
        } else {
            GuestSessionHealth::Degraded
        };
        Ok(Self {
            guest_ref: Some(guest_ref),
            boot_identity_digest: Some(boot_identity_digest),
            reconnect_generation: Some(reconnect_generation),
            capabilities,
            controller_ready,
            endpoint_ready,
            health,
        })
    }

    /// Construct a stale evidence snapshot after a disconnected session.
    pub fn stale(
        guest_ref: ResourceRef,
        reconnect_generation: u64,
    ) -> Result<Self, GuestSessionError> {
        let mut evidence = Self::current(
            guest_ref,
            "sha256:".to_owned() + &"0".repeat(64),
            reconnect_generation,
            [],
            false,
            false,
        )?;
        evidence.health = GuestSessionHealth::Degraded;
        Ok(evidence)
    }

    /// Return the current health projection.
    pub const fn health(&self) -> GuestSessionHealth {
        self.health
    }

    /// Return the bound Guest identity, when available.
    pub fn guest_ref(&self) -> Option<&ResourceRef> {
        self.guest_ref.as_ref()
    }

    /// Return the redacted boot-identity commitment, when available.
    pub fn boot_identity_digest(&self) -> Option<&str> {
        self.boot_identity_digest.as_deref()
    }

    /// Return the authenticated reconnect generation, when available.
    pub const fn reconnect_generation(&self) -> Option<u64> {
        self.reconnect_generation
    }

    /// Return the bounded capability names.
    pub fn capabilities(&self) -> &[String] {
        &self.capabilities
    }

    /// Return whether the target-local controller reported readiness.
    pub const fn controller_ready(&self) -> bool {
        self.controller_ready
    }

    /// Return whether the authenticated Endpoint reported readiness.
    pub const fn endpoint_ready(&self) -> bool {
        self.endpoint_ready
    }

    pub(crate) fn failed() -> Self {
        Self {
            guest_ref: None,
            boot_identity_digest: None,
            reconnect_generation: None,
            capabilities: Vec::new(),
            controller_ready: false,
            endpoint_ready: false,
            health: GuestSessionHealth::Failed,
        }
    }
}


impl fmt::Debug for GuestSessionEvidence {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GuestSessionEvidence")
            .field("guest_ref", &"<redacted>")
            .field("boot_identity_digest", &"<redacted>")
            .field("reconnect_generation", &self.reconnect_generation)
            .field("capabilities", &self.capabilities.len())
            .field("controller_ready", &self.controller_ready)
            .field("endpoint_ready", &self.endpoint_ready)
            .field("health", &self.health)
            .finish()
    }
}

fn valid_digest(value: &str) -> bool {
    let Some(hex) = value.strip_prefix("sha256:") else {
        return false;
    };
    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn validate_capabilities(
    capabilities: impl IntoIterator<Item = String>,
) -> Result<Vec<String>, GuestSessionError> {
    let capabilities = capabilities.into_iter().collect::<Vec<_>>();
    if capabilities.len() > 64
        || capabilities.iter().any(|capability| {
            capability.is_empty()
                || capability.len() > 128
                || !capability.is_ascii()
                || capability.chars().any(char::is_whitespace)
        })
    {
        return Err(GuestSessionError::Protocol);
    }
    Ok(capabilities)
}

/// Stable component-session health failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestSessionError {
    /// Wrong component-session identity or CID.
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

impl GuestSessionError {
    /// Return the stable error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::WrongIdentity => "component-session-wrong-identity",
            Self::AuthenticationFailed => "component-session-authentication-failed",
            Self::Timeout => "component-session-timeout",
            Self::Protocol => "component-session-protocol",
            Self::Disconnected => "component-session-disconnected",
        }
    }
}

/// Authenticated Guest ComponentSession evidence probe.
#[async_trait]
pub trait GuestSessionEvidenceProbe: Send + Sync {
    /// Observe the current authenticated Guest session and its capabilities.
    async fn observe(
        &self,
        expected_cid: u32,
        deadline_ms: u32,
    ) -> Result<GuestSessionEvidence, GuestSessionError>;

    /// Close the authenticated Guest session before VMM teardown.
    async fn close(&self, expected_cid: u32) -> Result<(), GuestSessionError>;
}
