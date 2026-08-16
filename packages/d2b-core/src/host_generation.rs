//! Core-side source-generation compatibility checks.

/// Stable source-generation floor used by the typed handoff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceGenerationCompatibilityFloorV1 {
    minimum_generation: u64,
    target_fingerprint: [u8; 32],
}

impl SourceGenerationCompatibilityFloorV1 {
    /// Construct a non-empty floor.
    pub fn new(
        minimum_generation: u64,
        target_fingerprint: [u8; 32],
    ) -> Result<Self, HostGenerationError> {
        if minimum_generation == 0 || target_fingerprint == [0; 32] {
            return Err(HostGenerationError::InvalidFloor);
        }
        Ok(Self {
            minimum_generation,
            target_fingerprint,
        })
    }

    /// Validate a target before any mutation.
    pub fn admits(&self, generation: u64, fingerprint: [u8; 32]) -> bool {
        generation >= self.minimum_generation && fingerprint == self.target_fingerprint
    }

    /// Return the fixed protocol name.
    pub const fn protocol(&self) -> &'static str {
        "source-handoff-v1"
    }
}

/// Stable core-side handoff failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostGenerationError {
    /// The floor contained no usable compatibility evidence.
    InvalidFloor,
}

impl core::fmt::Display for HostGenerationError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str("host-generation-floor-invalid")
    }
}

impl std::error::Error for HostGenerationError {}
