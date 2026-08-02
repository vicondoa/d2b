//! Provider instance lifecycle handle.

use std::fmt;

use d2b_contracts::v3::{ResourceGeneration, ResourceRef};

/// Lifecycle state of one authenticated Provider instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderInstanceState {
    /// Calls may be dispatched.
    Ready,
    /// New calls are refused while existing calls drain.
    Draining,
    /// The instance is no longer usable.
    Retired,
}

/// Opaque identity for an installed Provider instance.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderInstance {
    provider_ref: ResourceRef,
    generation: ResourceGeneration,
    state: ProviderInstanceState,
}

impl ProviderInstance {
    /// Construct a ready instance handle.
    pub fn new(
        provider_ref: ResourceRef,
        generation: ResourceGeneration,
    ) -> Result<Self, crate::error::RegistryBuildError> {
        if provider_ref.resource_type().as_str() != "Provider" {
            return Err(crate::error::RegistryBuildError::NotAProviderRef);
        }
        Ok(Self {
            provider_ref,
            generation,
            state: ProviderInstanceState::Ready,
        })
    }

    /// Borrow the Provider reference.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Return the Provider generation.
    pub const fn generation(&self) -> ResourceGeneration {
        self.generation
    }

    /// Return lifecycle state.
    pub const fn state(&self) -> ProviderInstanceState {
        self.state
    }

    /// Transition to draining.
    pub fn drain(&mut self) {
        if self.state == ProviderInstanceState::Ready {
            self.state = ProviderInstanceState::Draining;
        }
    }

    /// Transition to retired.
    pub fn retire(&mut self) {
        self.state = ProviderInstanceState::Retired;
    }
}

impl fmt::Debug for ProviderInstance {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProviderInstance(<redacted>)")
    }
}
