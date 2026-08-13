//! Opaque legacy swtpm adoption contract.

/// Closed outcome of the broker-owned one-time legacy state adoption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyMigrationOutcome {
    /// The legacy payload and marker were moved and committed.
    Migrated,
    /// The broker journal already proves the move was committed.
    AlreadyMigrated,
    /// Trusted inventory proved the Device was never provisioned.
    NotApplicable,
    /// A journal or lock is still in progress and can be replayed.
    Pending,
    /// The broker could not safely complete the migration.
    Failed,
    /// Source, destination, marker, or owner evidence was ambiguous.
    Ambiguous,
}

impl LegacyMigrationOutcome {
    /// Whether the outcome permits the first state Volume ensure.
    pub const fn permits_ensure(self) -> bool {
        matches!(
            self,
            Self::Migrated | Self::AlreadyMigrated | Self::NotApplicable
        )
    }

    /// Whether the outcome is terminal without permitting ensure.
    pub const fn is_terminal_failure(self) -> bool {
        matches!(self, Self::Failed | Self::Ambiguous)
    }
}
