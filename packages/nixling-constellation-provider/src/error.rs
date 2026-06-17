//! Typed provider errors (ADR 0032). A provider that cannot support a
//! feature returns a typed capability denial, never a silent fallback.

use nixling_constellation_core::{Capability, ConstellationError, ErrorKind};

/// A provider-layer error. Wraps the codec-neutral
/// [`ConstellationError`] so providers and the operation layer share one
/// typed-error vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError(pub ConstellationError);

impl ProviderError {
    /// A typed capability denial (the provider does not advertise `cap`).
    pub fn capability_denied(cap: Capability) -> Self {
        Self(ConstellationError::capability_denied(cap))
    }

    /// A typed "feature/transport mode not implemented in this build"
    /// refusal (never a silent fallback).
    pub fn unsupported(feature: impl Into<String>) -> Self {
        Self(ConstellationError::new(
            ErrorKind::UnsupportedFeature,
            feature.into(),
        ))
    }

    /// A generic typed error.
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self(ConstellationError::new(kind, message))
    }

    /// The underlying error kind.
    pub fn kind(&self) -> ErrorKind {
        self.0.kind
    }

    /// The structured missing capability, if this is a capability denial.
    pub fn missing_capability(&self) -> Option<Capability> {
        self.0.missing_capability()
    }
}

impl core::fmt::Display for ProviderError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ProviderError {}

impl From<ConstellationError> for ProviderError {
    fn from(e: ConstellationError) -> Self {
        Self(e)
    }
}

/// Provider result alias.
pub type ProviderResult<T> = Result<T, ProviderError>;
