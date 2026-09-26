//! Storage-neutral resource store errors.

use crate::v3::{MAX_BATCH_MUTATIONS, ResourceErrorKind, RetryClass, ZoneRevision};

/// Upper bound on the stores one composition root may open.
pub const MAX_STORE_SLOTS: usize = 64;

/// Zero-based index of a mutation in a bounded commit batch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MutationOrdinal(u8);

impl MutationOrdinal {
    /// Construct an ordinal inside the frozen batch bound.
    pub fn new(value: u32) -> Result<Self, MutationOrdinalError> {
        if usize::try_from(value).map_or(true, |value| value >= MAX_BATCH_MUTATIONS) {
            return Err(MutationOrdinalError);
        }
        Ok(Self(u8::try_from(value).map_err(|_| MutationOrdinalError)?))
    }

    /// Read the zero-based batch index.
    pub const fn get(self) -> u32 {
        self.0 as u32
    }
}

/// Mutation ordinal exceeded the frozen commit-batch bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MutationOrdinalError;

impl core::fmt::Display for MutationOrdinalError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("mutation ordinal exceeds the commit-batch bound")
    }
}

impl std::error::Error for MutationOrdinalError {}

/// Zero-based position of one store in a composition root's declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StoreSlot(u8);

impl StoreSlot {
    pub fn new(index: u32) -> Result<Self, StoreSlotError> {
        if usize::try_from(index).map_or(true, |index| index >= MAX_STORE_SLOTS) {
            return Err(StoreSlotError);
        }
        Ok(Self(u8::try_from(index).map_err(|_| StoreSlotError)?))
    }

    /// Read the zero-based store position.
    pub const fn get(self) -> u32 {
        self.0 as u32
    }
}

impl core::fmt::Display for StoreSlot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.get().fmt(f)
    }
}

/// Store slot exceeded the composition bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreSlotError;

impl core::fmt::Display for StoreSlotError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("store slot exceeds the composition bound")
    }
}

impl std::error::Error for StoreSlotError {}

/// Which declared component of a seal identity disagreed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealIdentityMismatch {
    Zone,
    Store,
    Epoch,
}

impl SealIdentityMismatch {
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::Zone => "mutation-seal-acceptor-zone-mismatch",
            Self::Store => "mutation-seal-acceptor-store-mismatch",
            Self::Epoch => "mutation-seal-acceptor-store-epoch-mismatch",
        }
    }
}

/// Closed store error classification.
///
/// The resource-plane half of the set is the shared
/// [`ResourceErrorKind`] classification itself, so one resource-plane kind
/// has exactly one spelling and one mapping; the three store-only kinds are
/// the store machinery failures with no resource-plane counterpart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StoreErrorKind {
    /// A resource-plane error, classified by the closed API set.
    Resource(ResourceErrorKind),
    /// The store's own integrity machinery failed.
    StoreIntegrityFailure,
    /// The store's internal queue is overfull.
    StoreBackpressure,
    /// The store quarantined the operation.
    StoreQuarantined,
}

impl StoreErrorKind {
    /// Exact stable contract spelling.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Resource(kind) => kind.as_str(),
            Self::StoreIntegrityFailure => "store-integrity-failure",
            Self::StoreBackpressure => "store-backpressure",
            Self::StoreQuarantined => "store-quarantined",
        }
    }

    /// The store-only kinds, in stable order.
    pub const fn all() -> &'static [Self; 3] {
        &[
            Self::StoreIntegrityFailure,
            Self::StoreBackpressure,
            Self::StoreQuarantined,
        ]
    }
}

/// Store error with only API-safe optional metadata.
#[derive(Clone, PartialEq, Eq)]
pub struct StoreError {
    kind: StoreErrorKind,
    current_revision: Option<ZoneRevision>,
    mutation_ordinal: Option<MutationOrdinal>,
    store_slot: Option<StoreSlot>,
    retry_after_ms: Option<u32>,
    retry_class: RetryClass,
    reason_code: &'static str,
}

impl StoreError {
    /// Construct a store error from a fixed, non-sensitive reason code.
    pub const fn new(
        kind: StoreErrorKind,
        current_revision: Option<ZoneRevision>,
        retry_after_ms: Option<u32>,
        retry_class: RetryClass,
        reason_code: &'static str,
    ) -> Self {
        Self {
            kind,
            current_revision,
            mutation_ordinal: None,
            store_slot: None,
            retry_after_ms,
            retry_class,
            reason_code,
        }
    }

    /// Construct a conflict that identifies the stale mutation in a batch.
    pub const fn batch_conflict(
        current_revision: ZoneRevision,
        mutation_ordinal: MutationOrdinal,
        retry_class: RetryClass,
        reason_code: &'static str,
    ) -> Self {
        Self {
            kind: StoreErrorKind::Resource(ResourceErrorKind::ResourceConflict),
            current_revision: Some(current_revision),
            mutation_ordinal: Some(mutation_ordinal),
            store_slot: None,
            retry_after_ms: None,
            retry_class,
            reason_code,
        }
    }

    /// Read the error kind.
    pub const fn kind(&self) -> StoreErrorKind {
        self.kind
    }

    /// Read the revision that caused the conflict, when present.
    pub const fn current_revision(&self) -> Option<ZoneRevision> {
        self.current_revision
    }

    /// Read the batch ordinal that caused the conflict, when present.
    pub const fn mutation_ordinal(&self) -> Option<MutationOrdinal> {
        self.mutation_ordinal
    }

    /// Read the store slot the error refers to, when present.
    pub const fn store_slot(&self) -> Option<StoreSlot> {
        self.store_slot
    }

    /// Bind the error to the store slot being operated on.
    pub const fn with_store_slot(mut self, store_slot: StoreSlot) -> Self {
        self.store_slot = Some(store_slot);
        self
    }

    /// Read the suggested retry delay, when the error is retryable.
    pub const fn retry_after_ms(&self) -> Option<u32> {
        self.retry_after_ms
    }

    /// Read the retry class.
    pub const fn retry_class(&self) -> RetryClass {
        self.retry_class
    }

    /// Read the stable reason code.
    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }
}

impl core::fmt::Debug for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("StoreError")
            .field("kind", &self.kind)
            .field("current_revision", &self.current_revision)
            .field("mutation_ordinal", &self.mutation_ordinal)
            .field("store_slot", &self.store_slot)
            .field("retry_after_ms", &self.retry_after_ms)
            .field("retry_class", &self.retry_class)
            .field("reason_code", &self.reason_code)
            .finish()
    }
}

impl core::fmt::Display for StoreError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.kind.as_str())
    }
}

impl std::error::Error for StoreError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mutation_ordinal_is_zero_based_and_bounded_by_batch_limit() {
        assert_eq!(MutationOrdinal::new(0).unwrap().get(), 0);
        assert_eq!(
            MutationOrdinal::new(u32::try_from(MAX_BATCH_MUTATIONS - 1).unwrap())
                .unwrap()
                .get(),
            u32::try_from(MAX_BATCH_MUTATIONS - 1).unwrap()
        );
        assert_eq!(
            MutationOrdinal::new(u32::try_from(MAX_BATCH_MUTATIONS).unwrap()),
            Err(MutationOrdinalError)
        );
    }

    #[test]
    fn store_slot_rejects_an_index_at_the_composition_bound() {
        assert_eq!(StoreSlot::new(0).unwrap().get(), 0);
        assert_eq!(
            StoreSlot::new(u32::try_from(MAX_STORE_SLOTS - 1).unwrap())
                .unwrap()
                .get(),
            u32::try_from(MAX_STORE_SLOTS - 1).unwrap()
        );
        assert_eq!(
            StoreSlot::new(u32::try_from(MAX_STORE_SLOTS).unwrap()),
            Err(StoreSlotError)
        );
    }

    #[test]
    fn batch_conflict_carries_only_revision_and_bounded_ordinal() {
        let error = StoreError::batch_conflict(
            ZoneRevision::new(9),
            MutationOrdinal::new(3).unwrap(),
            RetryClass::Reauthorize,
            "revision-changed",
        );

        assert_eq!(
            error.kind(),
            StoreErrorKind::Resource(ResourceErrorKind::ResourceConflict)
        );
        assert_eq!(error.current_revision(), Some(ZoneRevision::new(9)));
        assert_eq!(error.mutation_ordinal().unwrap().get(), 3);
        assert_eq!(error.retry_after_ms(), None);
        assert_eq!(error.reason_code(), "revision-changed");
    }
}
