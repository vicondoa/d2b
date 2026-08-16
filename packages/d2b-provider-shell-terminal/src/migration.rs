//! Provider-local migration boundaries.

/// The shell-terminal Provider declares no durable Provider state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderStateSet {
    /// The controller has no Provider state Volume or persistent state mount.
    Empty,
}

impl ProviderStateSet {
    /// Return the only accepted state declaration.
    pub const fn canonical() -> Self {
        Self::Empty
    }
}

/// Provider-local disposition of superseded protocol concepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationDisposition {
    /// This Provider exposes only ComponentSession services and the named terminal stream.
    NoProviderLegacyProtocol,
}

impl MigrationDisposition {
    /// Return the provider's fixed protocol disposition.
    pub const fn canonical() -> Self {
        Self::NoProviderLegacyProtocol
    }
}
