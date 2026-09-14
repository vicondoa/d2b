//! Bounded Process adoption outcome reported to the Guest controller.

/// Bounded adoption result exposed to the Guest controller.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessAdoptionStatus {
    /// The Process Provider has already established current status.
    Current,
    /// The Process Provider verified and adopted the exact process locally.
    Adopted,
    /// No process realization remains.
    Absent,
    /// Identity was stale or ambiguous and is quarantined.
    Quarantined,
    /// The Process Provider could not complete a safe observation.
    Unavailable,
}
