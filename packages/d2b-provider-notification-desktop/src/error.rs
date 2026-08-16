//! Stable notification Provider error surface.

/// Stable error code family used by stream and lifecycle adapters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderError {
    /// Session admission failed.
    Session,
    /// A field or category was rejected.
    Schema,
    /// The desktop sink is unavailable.
    SinkUnavailable,
    /// A bounded queue or action capability limit was reached.
    Capacity,
}

impl ProviderError {
    /// Return the stable error slug.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Session => "session-denied",
            Self::Schema => "notification-schema-invalid",
            Self::SinkUnavailable => "sink-unavailable",
            Self::Capacity => "capacity-exceeded",
        }
    }
}

impl core::fmt::Display for ProviderError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::error::Error for ProviderError {}
