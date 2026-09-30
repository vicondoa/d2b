//! Bounded Process and binding adoption outcomes reported to the Guest
//! controller.
//!
//! Restart adoption has to answer one question for every thing the Guest
//! depends on: does the row I am looking at still stand for the thing this
//! controller declared? A cached `Ready` is not an answer. For the VMM
//! Process that question is asked by the Process Provider and reported as
//! [`ProcessAdoptionStatus`]; for an admitted binding it is asked here, over
//! the exact identities the relationship was admitted under (R41).
//!
//! The two are the same rule in two vocabularies. What differs is what a
//! mismatch costs: a stale Process is quarantined, and a stale binding
//! relationship is refused rather than re-adopted, because re-adopting it
//! would hand the Guest access under a fence that no longer describes it.

use d2b_contracts_resource::v3::{BoundedToken, ResourceRef, ResourceUid};

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

/// Bounded adoption result for one admitted binding relationship.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingAdoptionStatus {
    /// The relationship was already recorded under the fence being reported,
    /// so the recorded evidence is unchanged.
    Current,
    /// The relationship is recorded for the first time under this fence.
    Adopted,
    /// No row remains for the relationship.
    Absent,
    /// The row describes a different consumer, source, or view, so it is not
    /// this relationship's evidence and nothing was recorded.
    Refused,
    /// The observation could not be completed.
    Unavailable,
}

/// The exact identities one relationship must still match before restart
/// adopts it.
///
/// The fence deliberately names the source, its view, and both store-assigned
/// identities rather than a readiness value: a row that is Ready under a
/// different source UID or a different consumer UID is a different
/// relationship's row, and adopting it would mint access the admission never
/// granted (R35, R41).
#[derive(Clone, PartialEq, Eq)]
pub struct BindingAdoptionFence {
    source_ref: ResourceRef,
    source_uid: ResourceUid,
    view: BoundedToken,
    consumer_ref: ResourceRef,
    consumer_uid: ResourceUid,
}

impl BindingAdoptionFence {
    /// Construct the fence one admitted relationship was admitted under.
    pub fn new(
        source_ref: ResourceRef,
        source_uid: ResourceUid,
        view: BoundedToken,
        consumer_ref: ResourceRef,
        consumer_uid: ResourceUid,
    ) -> Self {
        Self {
            source_ref,
            source_uid,
            view,
            consumer_ref,
            consumer_uid,
        }
    }

    /// Whether one observed row still stands for this relationship.
    ///
    /// Both halves have to match. A row whose source UID differs is a
    /// replaced source and a row whose consumer UID differs is a reassigned
    /// Guest; either one makes the observed readiness a statement about
    /// something else.
    pub fn matches(&self, row: ObservedBindingRow<'_>) -> bool {
        &self.source_ref == row.source_ref
            && &self.source_uid == row.source_uid
            && &self.view == row.view
            && &self.consumer_ref == row.consumer_ref
            && &self.consumer_uid == row.consumer_uid
    }
}

impl std::fmt::Debug for BindingAdoptionFence {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BindingAdoptionFence")
            .field("source_ref", &self.source_ref)
            .field("has_source_uid", &true)
            .field("view", &self.view)
            .field("consumer_ref", &self.consumer_ref)
            .field("has_consumer_uid", &true)
            .finish()
    }
}

/// One observed `VolumeBinding` row, read back under its committed
/// identities.
///
/// It is a view of committed bytes rather than a decision: nothing here says
/// the row is ready, only which relationship it is.
#[derive(Debug, Clone, Copy)]
pub struct ObservedBindingRow<'a> {
    /// The exact source Volume.
    pub source_ref: &'a ResourceRef,
    /// The source's store-assigned identity.
    pub source_uid: &'a ResourceUid,
    /// The named Volume view.
    pub view: &'a BoundedToken,
    /// The exact consumer.
    pub consumer_ref: &'a ResourceRef,
    /// The consumer's store-assigned identity.
    pub consumer_uid: &'a ResourceUid,
}

/// Classify one observed binding row against the fence it must match.
///
/// `observed` is `None` when no row remains for the relationship, which is
/// the honest answer after a restart and not a failure: the Guest's use is
/// gone and nothing has to be re-admitted.
pub fn classify_binding_adoption(
    fence: &BindingAdoptionFence,
    observed: Option<ObservedBindingRow<'_>>,
) -> BindingAdoptionStatus {
    match observed {
        None => BindingAdoptionStatus::Absent,
        Some(row) if fence.matches(row) => BindingAdoptionStatus::Adopted,
        Some(_) => BindingAdoptionStatus::Refused,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(value: &str) -> ResourceRef {
        ResourceRef::parse(value).expect("valid fixture ref")
    }

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).expect("valid fixture uid")
    }

    fn view(value: &str) -> BoundedToken {
        BoundedToken::parse(value).expect("valid view")
    }

    const SOURCE_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
    const CONSUMER_UID: &str = "223e4567-e89b-42d3-a456-426614174000";

    struct Fixture {
        fence: BindingAdoptionFence,
        source: ResourceRef,
        source_uid: ResourceUid,
        view: BoundedToken,
        consumer: ResourceRef,
        consumer_uid: ResourceUid,
    }

    fn fixture() -> Fixture {
        Fixture {
            fence: BindingAdoptionFence::new(
                reference("Volume/system"),
                uid(SOURCE_UID),
                view("system"),
                reference("Guest/work-vm"),
                uid(CONSUMER_UID),
            ),
            source: reference("Volume/system"),
            source_uid: uid(SOURCE_UID),
            view: view("system"),
            consumer: reference("Guest/work-vm"),
            consumer_uid: uid(CONSUMER_UID),
        }
    }

    impl Fixture {
        fn row(&self) -> ObservedBindingRow<'_> {
            ObservedBindingRow {
                source_ref: &self.source,
                source_uid: &self.source_uid,
                view: &self.view,
                consumer_ref: &self.consumer,
                consumer_uid: &self.consumer_uid,
            }
        }
    }

    #[test]
    fn a_matching_row_is_adopted_and_an_absent_one_is_not_a_failure() {
        let fixture = fixture();
        assert_eq!(
            classify_binding_adoption(&fixture.fence, Some(fixture.row())),
            BindingAdoptionStatus::Adopted
        );
        assert_eq!(
            classify_binding_adoption(&fixture.fence, None),
            BindingAdoptionStatus::Absent
        );
    }

    #[test]
    fn a_reassigned_guest_or_replaced_source_is_refused_not_adopted() {
        let mut replaced = fixture();
        replaced.source_uid = uid("323e4567-e89b-42d3-a456-426614174000");
        assert_eq!(
            classify_binding_adoption(&replaced.fence, Some(replaced.row())),
            BindingAdoptionStatus::Refused,
            "a replaced source is a different relationship's row"
        );

        let mut reassigned = fixture();
        reassigned.consumer_uid = uid("423e4567-e89b-42d3-a456-426614174000");
        assert_eq!(
            classify_binding_adoption(&reassigned.fence, Some(reassigned.row())),
            BindingAdoptionStatus::Refused,
            "a reassigned Guest is a different relationship's row"
        );

        let mut other_view = fixture();
        other_view.view = view("scratch");
        assert_eq!(
            classify_binding_adoption(&other_view.fence, Some(other_view.row())),
            BindingAdoptionStatus::Refused,
            "another view of the same Volume is a different relationship"
        );
    }
}