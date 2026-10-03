//! Consumer slot allocation for binding relationships.
//!
//! The slot is the consumer's own local name for a relationship, and this
//! module owns everything that name implies: the token that spells it, the
//! index key that scopes it to one Zone, consumer, and kind, the digest that
//! decides whether two declarations are one relationship, and the index that
//! normalizes declarations against what already occupies the slot.
//!
//! # A slot identifies; a payload does not
//!
//! [`BindingSlot`] is the stable half of a [`BindingKey`], and
//! [`BindingSpecFingerprint`] is the changing half. The index in
//! [`BindingSlotIndex`] reads that difference directly: an identical
//! declaration coalesces, a changed payload is refused while the slot is live
//! and updates it once the old use is closed, and a different source waits for
//! the same release to become the successor. An existing binding therefore
//! never changes owner in place.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::{
    ResourceRef,
    binding::{BindingContractError, BindingKey, BindingKind},
    binding_lifecycle::BindingLifecycleState,
    execution_policy::{
        BoundedToken, PrimitiveSpecError, parsed_deserialize, redacted_debug, string_schema,
    },
    identity::{ResourceUid, ZoneId},
    resource_schema::{canonical_json_bytes, framed_canonical_digest, is_canonical_digest},
};

/// Maximum bytes in one stable consumer slot token.
pub const MAX_BINDING_SLOT_BYTES: usize = 63;

/// The stable consumer slot a request occupies.
///
/// The slot is the consumer's own local name for the relationship. It is what
/// makes a rights or destination update the same relationship rather than a
/// second one, so it identifies; rights, destination, and every other mutable
/// payload field never do.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct BindingSlot(BoundedToken);

impl BindingSlot {
    /// Parse a `^[a-z][a-z0-9-]*$` slot token.
    pub fn parse(value: impl Into<String>) -> Result<Self, PrimitiveSpecError> {
        BoundedToken::parse(value).map(Self)
    }

    /// Borrow the canonical slot token.
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

redacted_debug!(BindingSlot);
parsed_deserialize!(BindingSlot);
string_schema!(BindingSlot, 1, MAX_BINDING_SLOT_BYTES);

/// The consumer slot index key: Zone, consumer identity, kind, and slot.
///
/// It deliberately excludes the source. Two declarations that agree on the
/// source but not on their payload are one relationship; two declarations
/// that disagree on the source are a replacement, and both are refused while
/// the slot is live.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BindingSlotAddress {
    zone: ZoneId,
    consumer_uid: ResourceUid,
    kind: BindingKind,
    slot: BindingSlot,
}

impl BindingSlotAddress {
    /// Derive the index key a relationship occupies.
    pub(crate) const fn new(
        zone: ZoneId,
        consumer_uid: ResourceUid,
        kind: BindingKind,
        slot: BindingSlot,
    ) -> Self {
        Self {
            zone,
            consumer_uid,
            kind,
            slot,
        }
    }

    /// Borrow the Zone.
    pub const fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Borrow the consumer identity.
    pub const fn consumer_uid(&self) -> &ResourceUid {
        &self.consumer_uid
    }

    /// Return the binding kind.
    pub const fn kind(&self) -> BindingKind {
        self.kind
    }

    /// Borrow the stable slot token.
    pub const fn slot(&self) -> &BindingSlot {
        &self.slot
    }
}

redacted_debug!(BindingSlotAddress);

/// The digest of the exact desired bytes one declaration carries.
///
/// Two declarations of the same relationship coalesce only when their
/// canonical bytes match, which is what makes a rights or destination change
/// visible as an update instead of silently reusing the earlier request.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BindingSpecFingerprint(String);

impl BindingSpecFingerprint {
    /// The domain tag framing one binding-request digest.
    pub const DOMAIN_TAG: &'static str = "d2b:v3:binding-request";

    /// Frame the digest of one desired request.
    pub fn from_request<T: Serialize>(request: &T) -> Self {
        let bytes = canonical_json_bytes(request)
            .expect("a typed binding request always renders as canonical bytes");
        Self(framed_canonical_digest(Self::DOMAIN_TAG, &bytes))
    }

    /// Parse a framed request digest.
    pub fn parse(value: impl Into<String>) -> Result<Self, BindingContractError> {
        let value = value.into();
        if is_canonical_digest(&value) {
            Ok(Self(value))
        } else {
            Err(BindingContractError::InvalidField)
        }
    }

    /// Borrow the framed digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

redacted_debug!(BindingSpecFingerprint);

/// What normalizing one declaration against the consumer slot index did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BindingSlotDecision {
    /// The slot was free and this declaration now occupies it.
    Claimed,
    /// An identical declaration already occupies the slot, so the two coalesce.
    Coalesced,
    /// The previous occupant released and this declaration is its successor.
    SuccessorClaimed,
    /// The same relationship's payload changed while its old use was closed.
    PayloadUpdated,
}

/// One observed declaration occupying a consumer slot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingSlotEntry {
    source_ref: ResourceRef,
    source_uid: ResourceUid,
    fingerprint: BindingSpecFingerprint,
    state: BindingLifecycleState,
}

impl BindingSlotEntry {
    /// Borrow the occupying source reference.
    pub const fn source_ref(&self) -> &ResourceRef {
        &self.source_ref
    }

    /// Borrow the occupying source identity.
    pub const fn source_uid(&self) -> &ResourceUid {
        &self.source_uid
    }

    /// Borrow the occupying declaration's digest.
    pub const fn fingerprint(&self) -> &BindingSpecFingerprint {
        &self.fingerprint
    }

    /// Return the observed lifecycle.
    pub const fn state(&self) -> BindingLifecycleState {
        self.state
    }
}

/// The consumer slot index for one Zone's declared relationships.
///
/// At most one binding occupies a live slot, which is what lets a source
/// replacement retire the old source-owned binding before its successor is
/// admitted without ever changing an existing binding's owner.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindingSlotIndex {
    entries: BTreeMap<BindingSlotAddress, BindingSlotEntry>,
}

impl BindingSlotIndex {
    /// Construct an empty index.
    pub fn new() -> Self {
        Self::default()
    }

    /// Borrow the declaration occupying one slot, when there is one.
    pub fn occupant(&self, address: &BindingSlotAddress) -> Option<&BindingSlotEntry> {
        self.entries.get(address)
    }

    /// Borrow every live slot, in index order.
    pub fn entries(&self) -> impl Iterator<Item = (&BindingSlotAddress, &BindingSlotEntry)> {
        self.entries.iter()
    }

    /// Normalize one declaration for its consumer slot.
    ///
    /// An identical declaration coalesces. A different declaration for the
    /// same source is refused before any mutation while the slot is live,
    /// because changing its rights or destination in place is what would
    /// activate old and new access together; once the occupant has released,
    /// the declaration updates it. A declaration naming a different source
    /// waits for the same release and then becomes the successor, so an
    /// existing binding never changes owner.
    pub fn declare(
        &mut self,
        key: &BindingKey,
        fingerprint: &BindingSpecFingerprint,
    ) -> Result<BindingSlotDecision, BindingContractError> {
        let address = key.address();
        match self.entries.get_mut(&address) {
            None => {
                self.entries.insert(
                    address,
                    BindingSlotEntry {
                        source_ref: key.source_ref().clone(),
                        source_uid: key.source_uid().clone(),
                        fingerprint: fingerprint.clone(),
                        state: BindingLifecycleState::Requested,
                    },
                );
                Ok(BindingSlotDecision::Claimed)
            }
            Some(entry) => {
                if entry.source_uid() != key.source_uid() {
                    return if entry.state == BindingLifecycleState::Released {
                        entry.source_ref = key.source_ref().clone();
                        entry.source_uid = key.source_uid().clone();
                        entry.fingerprint = fingerprint.clone();
                        Ok(BindingSlotDecision::SuccessorClaimed)
                    } else {
                        Err(BindingContractError::SourceMismatch)
                    };
                }
                if entry.fingerprint() == fingerprint {
                    return Ok(BindingSlotDecision::Coalesced);
                }
                if entry.state != BindingLifecycleState::Released {
                    return Err(BindingContractError::SlotOccupied);
                }
                entry.fingerprint = fingerprint.clone();
                Ok(BindingSlotDecision::PayloadUpdated)
            }
        }
    }

    /// Apply a rights or destination change to an existing relationship.
    ///
    /// The change is admitted only once the old use is blocked or closed, so
    /// the previous access and the new one are never both live. The slot keeps
    /// its owner across the change.
    pub fn change_payload(
        &mut self,
        key: &BindingKey,
        fingerprint: &BindingSpecFingerprint,
    ) -> Result<BindingSlotDecision, BindingContractError> {
        let entry = self
            .entries
            .get_mut(&key.address())
            .ok_or(BindingContractError::UnexpectedState)?;
        if entry.source_uid() != key.source_uid() {
            return Err(BindingContractError::SourceMismatch);
        }
        if entry.fingerprint() == fingerprint {
            return Ok(BindingSlotDecision::Coalesced);
        }
        if matches!(
            entry.state,
            BindingLifecycleState::Revoking
                | BindingLifecycleState::Draining
                | BindingLifecycleState::Released
        ) {
            entry.fingerprint = fingerprint.clone();
            Ok(BindingSlotDecision::PayloadUpdated)
        } else {
            Err(BindingContractError::SlotOccupied)
        }
    }

    /// Record the observed lifecycle for a declared slot.
    pub fn observe(
        &mut self,
        key: &BindingKey,
        state: BindingLifecycleState,
    ) -> Result<(), BindingContractError> {
        let entry = self
            .entries
            .get_mut(&key.address())
            .ok_or(BindingContractError::UnexpectedState)?;
        if entry.source_uid() != key.source_uid() {
            return Err(BindingContractError::SourceMismatch);
        }
        entry.state = state;
        Ok(())
    }
}
