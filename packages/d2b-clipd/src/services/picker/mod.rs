//! Authenticated clipboard-picker service policy.
//!
//! Generated `d2b.clipboard.picker.v2` requests are decoded and admitted by the
//! ComponentSession composition layer before they reach this module. There is
//! no pathname, newline-frame, or protocol-version compatibility path.

use std::collections::{BTreeMap, VecDeque};

use crate::{
    framing::PickerProjectionBounds,
    protocol::{
        CapabilityPreflight, ClipboardTarget, MAX_RETAINED_OFFERS, OfferQuery, OfferSelection,
        OpaquePickerId, PickerOffer,
    },
};

pub const SERVICE_PACKAGE: &str = "d2b.clipboard.picker.v2";
pub const ENDPOINT_PURPOSE: &str = "clipboard-picker";
pub const ENDPOINT_ROLE: &str = "clipboard-picker";
pub const SERVICE_NAME: &str = "ClipboardPickerService";

const MAX_IDEMPOTENCY_KEY_BYTES: usize = 64;
const MAX_RECEIPTS: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum PickerMethod {
    ListOffers,
    SelectOffer,
    CancelSelection,
    Cancel,
}

impl PickerMethod {
    pub const fn name(self) -> &'static str {
        match self {
            Self::ListOffers => "ListOffers",
            Self::SelectOffer => "SelectOffer",
            Self::CancelSelection => "CancelSelection",
            Self::Cancel => "Cancel",
        }
    }

    pub const fn mutating(self) -> bool {
        matches!(self, Self::SelectOffer | Self::CancelSelection)
    }
}

pub const METHODS: &[PickerMethod] = &[
    PickerMethod::ListOffers,
    PickerMethod::SelectOffer,
    PickerMethod::CancelSelection,
    PickerMethod::Cancel,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerTransport {
    ComponentSessionUnixStream,
    ComponentSessionUnixSeqpacket,
}

#[derive(Clone, PartialEq, Eq)]
pub struct PickerSession {
    generation: u64,
    transport: PickerTransport,
}

impl std::fmt::Debug for PickerSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PickerSession")
            .field("generation", &"<redacted>")
            .field("transport", &self.transport)
            .finish()
    }
}

impl PickerSession {
    pub fn admit(
        generation: u64,
        transport: PickerTransport,
        component_session_authenticated: bool,
        host_local: bool,
        attachments_present: bool,
    ) -> Result<Self, PickerServiceError> {
        if generation == 0 {
            return Err(PickerServiceError::GenerationMismatch);
        }
        if !component_session_authenticated || !host_local {
            return Err(PickerServiceError::Unauthenticated);
        }
        if attachments_present {
            return Err(PickerServiceError::AttachmentDenied);
        }
        Ok(Self {
            generation,
            transport,
        })
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct PickerCall {
    request_id: [u8; 16],
    idempotency_key: Option<Vec<u8>>,
    session_generation: u64,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
}

impl std::fmt::Debug for PickerCall {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PickerCall")
            .field("request_id", &"<redacted>")
            .field(
                "idempotency_key",
                &self.idempotency_key.as_ref().map(|_| "<redacted>"),
            )
            .field("session_generation", &self.session_generation)
            .finish_non_exhaustive()
    }
}

impl PickerCall {
    pub fn new(
        request_id: [u8; 16],
        idempotency_key: Option<&[u8]>,
        session_generation: u64,
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<Self, PickerServiceError> {
        if request_id == [0; 16]
            || session_generation == 0
            || issued_at_unix_ms == 0
            || expires_at_unix_ms <= issued_at_unix_ms
            || expires_at_unix_ms - issued_at_unix_ms > 15 * 60 * 1_000
            || idempotency_key
                .is_some_and(|key| key.is_empty() || key.len() > MAX_IDEMPOTENCY_KEY_BYTES)
        {
            return Err(PickerServiceError::MalformedRequest);
        }
        Ok(Self {
            request_id,
            idempotency_key: idempotency_key.map(<[u8]>::to_vec),
            session_generation,
            issued_at_unix_ms,
            expires_at_unix_ms,
        })
    }

    fn admit(
        &self,
        session: &PickerSession,
        method: PickerMethod,
        now_unix_ms: u64,
    ) -> Result<(), PickerServiceError> {
        if self.session_generation != session.generation {
            return Err(PickerServiceError::GenerationMismatch);
        }
        if now_unix_ms < self.issued_at_unix_ms || now_unix_ms >= self.expires_at_unix_ms {
            return Err(PickerServiceError::DeadlineExpired);
        }
        if method.mutating() && self.idempotency_key.is_none() {
            return Err(PickerServiceError::MissingIdempotency);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelSelection {
    pub selection_id: OpaquePickerId,
    pub destination: ClipboardTarget,
}

impl CancelSelection {
    pub fn new(
        selection_id: &str,
        destination: ClipboardTarget,
    ) -> Result<Self, PickerServiceError> {
        Ok(Self {
            selection_id: OpaquePickerId::parse(selection_id)
                .map_err(|_| PickerServiceError::MalformedRequest)?,
            destination,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerOperation {
    List(OfferQuery),
    Select(OfferSelection),
    CancelSelection(CancelSelection),
    Cancel(OpaquePickerId),
}

impl PickerOperation {
    pub const fn method(&self) -> PickerMethod {
        match self {
            Self::List(_) => PickerMethod::ListOffers,
            Self::Select(_) => PickerMethod::SelectOffer,
            Self::CancelSelection(_) => PickerMethod::CancelSelection,
            Self::Cancel(_) => PickerMethod::Cancel,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectionReceipt {
    pub selection_id: OpaquePickerId,
    pub offer_id: OpaquePickerId,
    pub destination: ClipboardTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PickerResponse {
    Offers(Vec<PickerOffer>),
    Selected(SelectionReceipt),
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActiveSelection {
    offer_id: OpaquePickerId,
    destination: ClipboardTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CachedReceipt {
    method: PickerMethod,
    operation: PickerOperation,
    response: PickerResponse,
}

#[derive(Debug, Default)]
pub struct ClipboardPickerService {
    offers: BTreeMap<OpaquePickerId, PickerOffer>,
    selections: BTreeMap<OpaquePickerId, ActiveSelection>,
    receipts: BTreeMap<Vec<u8>, CachedReceipt>,
    receipt_order: VecDeque<Vec<u8>>,
}

impl ClipboardPickerService {
    pub fn publish_offer(&mut self, offer: PickerOffer) -> Result<(), PickerServiceError> {
        if !self.offers.contains_key(offer.offer_id()) && self.offers.len() >= MAX_RETAINED_OFFERS {
            return Err(PickerServiceError::ResourceExhausted);
        }
        self.offers.insert(offer.offer_id().clone(), offer);
        Ok(())
    }

    pub fn remove_offer(&mut self, offer_id: &OpaquePickerId) {
        self.offers.remove(offer_id);
    }

    pub fn invoke(
        &mut self,
        session: &PickerSession,
        call: &PickerCall,
        projection: PickerProjectionBounds,
        operation: PickerOperation,
        now_unix_ms: u64,
    ) -> Result<PickerResponse, PickerServiceError> {
        projection
            .validate()
            .map_err(|_| PickerServiceError::MalformedRequest)?;
        let method = operation.method();
        call.admit(session, method, now_unix_ms)?;

        if let Some(key) = &call.idempotency_key
            && let Some(cached) = self.receipts.get(key)
        {
            return (cached.method == method && cached.operation == operation)
                .then(|| cached.response.clone())
                .ok_or(PickerServiceError::IdempotencyConflict);
        }

        let response = match operation.clone() {
            PickerOperation::List(query) => self.list(query, now_unix_ms),
            PickerOperation::Select(selection) => self.select(selection, now_unix_ms),
            PickerOperation::CancelSelection(cancel) => self.cancel_selection(cancel),
            PickerOperation::Cancel(selection_id) => {
                self.selections.remove(&selection_id);
                Ok(PickerResponse::Cancelled)
            }
        }?;

        if let Some(key) = &call.idempotency_key {
            self.cache_receipt(
                key.clone(),
                CachedReceipt {
                    method,
                    operation,
                    response: response.clone(),
                },
            );
        }
        Ok(response)
    }

    fn list(
        &self,
        query: OfferQuery,
        now_unix_ms: u64,
    ) -> Result<PickerResponse, PickerServiceError> {
        let offers = self
            .offers
            .values()
            .filter(|offer| {
                offer.expires_at_unix_ms() > now_unix_ms
                    && offer.destination() == &query.destination
            })
            .take(query.page_size)
            .cloned()
            .collect();
        Ok(PickerResponse::Offers(offers))
    }

    fn select(
        &mut self,
        selection: OfferSelection,
        now_unix_ms: u64,
    ) -> Result<PickerResponse, PickerServiceError> {
        if self.selections.contains_key(&selection.selection_id) {
            return Err(PickerServiceError::Conflict);
        }
        let offer = self
            .offers
            .get(&selection.offer_id)
            .ok_or(PickerServiceError::NotFound)?;
        if offer.expires_at_unix_ms() <= now_unix_ms {
            return Err(PickerServiceError::DeadlineExpired);
        }
        if offer.destination() != &selection.destination {
            return Err(PickerServiceError::TargetMismatch);
        }
        if offer.capability_preflight() != CapabilityPreflight::Satisfied {
            return Err(PickerServiceError::CapabilityDenied);
        }
        let receipt = SelectionReceipt {
            selection_id: selection.selection_id,
            offer_id: selection.offer_id,
            destination: selection.destination,
        };
        self.selections.insert(
            receipt.selection_id.clone(),
            ActiveSelection {
                offer_id: receipt.offer_id.clone(),
                destination: receipt.destination.clone(),
            },
        );
        Ok(PickerResponse::Selected(receipt))
    }

    fn cancel_selection(
        &mut self,
        cancel: CancelSelection,
    ) -> Result<PickerResponse, PickerServiceError> {
        let active = self
            .selections
            .get(&cancel.selection_id)
            .ok_or(PickerServiceError::NotFound)?;
        if active.destination != cancel.destination {
            return Err(PickerServiceError::TargetMismatch);
        }
        self.selections.remove(&cancel.selection_id);
        Ok(PickerResponse::Cancelled)
    }

    fn cache_receipt(&mut self, key: Vec<u8>, receipt: CachedReceipt) {
        if !self.receipts.contains_key(&key) {
            if self.receipt_order.len() == MAX_RECEIPTS
                && let Some(oldest) = self.receipt_order.pop_front()
            {
                self.receipts.remove(&oldest);
            }
            self.receipt_order.push_back(key.clone());
        }
        self.receipts.insert(key, receipt);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PickerServiceError {
    #[error("clipboard-picker-unauthenticated")]
    Unauthenticated,
    #[error("clipboard-picker-generation-mismatch")]
    GenerationMismatch,
    #[error("clipboard-picker-attachment-denied")]
    AttachmentDenied,
    #[error("clipboard-picker-request-malformed")]
    MalformedRequest,
    #[error("clipboard-picker-deadline-expired")]
    DeadlineExpired,
    #[error("clipboard-picker-idempotency-required")]
    MissingIdempotency,
    #[error("clipboard-picker-idempotency-conflict")]
    IdempotencyConflict,
    #[error("clipboard-picker-not-found")]
    NotFound,
    #[error("clipboard-picker-conflict")]
    Conflict,
    #[error("clipboard-picker-target-mismatch")]
    TargetMismatch,
    #[error("clipboard-picker-capability-denied")]
    CapabilityDenied,
    #[error("clipboard-picker-resource-exhausted")]
    ResourceExhausted,
}

#[cfg(test)]
mod tests {
    use d2b_realm_core::WorkloadProviderKind;

    use super::*;
    use crate::protocol::{
        AttributionQuality, CapabilityPreflight, PickerOfferInput, ProtocolError,
    };

    fn target(value: &str) -> ClipboardTarget {
        ClipboardTarget::workload(value, WorkloadProviderKind::LocalVm).unwrap()
    }

    fn offer(destination: ClipboardTarget) -> PickerOffer {
        PickerOffer::new(PickerOfferInput {
            offer_id: OpaquePickerId::parse("offer-1").unwrap(),
            source: ClipboardTarget::Host,
            destination,
            mime_type: "text/plain".to_owned(),
            preview: Some("safe preview".to_owned()),
            thumbnail_png: None,
            source_application: None,
            attribution: AttributionQuality::ExactClient,
            capability_preflight: CapabilityPreflight::Satisfied,
            byte_count: Some(12),
            observed_at_unix_ms: 1_000,
            expires_at_unix_ms: 10_000,
            confirmation_required: true,
        })
        .unwrap()
    }

    fn session() -> PickerSession {
        PickerSession::admit(
            1,
            PickerTransport::ComponentSessionUnixStream,
            true,
            true,
            false,
        )
        .unwrap()
    }

    fn call(idempotency: Option<&[u8]>) -> PickerCall {
        PickerCall::new([1; 16], idempotency, 1, 1_000, 9_000).unwrap()
    }

    fn projection(offer_count: usize) -> PickerProjectionBounds {
        PickerProjectionBounds {
            encoded_bytes: 256,
            offer_count,
            thumbnail_bytes: 0,
            attachment_count: 0,
        }
    }

    #[test]
    fn contract_methods_match_frozen_picker_service() {
        assert_eq!(SERVICE_PACKAGE, "d2b.clipboard.picker.v2");
        assert_eq!(SERVICE_NAME, "ClipboardPickerService");
        assert_eq!(
            METHODS
                .iter()
                .map(|method| method.name())
                .collect::<Vec<_>>(),
            ["ListOffers", "SelectOffer", "CancelSelection", "Cancel"]
        );
        assert!(!PickerMethod::ListOffers.mutating());
        assert!(PickerMethod::SelectOffer.mutating());
    }

    #[test]
    fn session_admission_is_authenticated_and_attachment_free() {
        assert!(
            PickerSession::admit(
                1,
                PickerTransport::ComponentSessionUnixStream,
                false,
                true,
                false
            )
            .is_err()
        );
        assert!(
            PickerSession::admit(
                1,
                PickerTransport::ComponentSessionUnixSeqpacket,
                true,
                true,
                true
            )
            .is_err()
        );
    }

    #[test]
    fn lists_and_selects_only_exact_canonical_destination() {
        let destination = target("browser.personal.d2b");
        let other = target("editor.work.d2b");
        let mut service = ClipboardPickerService::default();
        service.publish_offer(offer(destination.clone())).unwrap();

        let listed = service
            .invoke(
                &session(),
                &call(None),
                projection(1),
                PickerOperation::List(OfferQuery::new(destination.clone(), 8).unwrap()),
                2_000,
            )
            .unwrap();
        assert!(matches!(listed, PickerResponse::Offers(offers) if offers.len() == 1));

        let mismatch = OfferSelection::new("selection-1", "offer-1", other).unwrap();
        assert_eq!(
            service.invoke(
                &session(),
                &call(Some(b"idem-1")),
                projection(1),
                PickerOperation::Select(mismatch),
                2_000,
            ),
            Err(PickerServiceError::TargetMismatch)
        );
    }

    #[test]
    fn mutating_selection_requires_idempotency_and_is_replay_safe() {
        let destination = target("browser.personal.d2b");
        let mut service = ClipboardPickerService::default();
        service.publish_offer(offer(destination.clone())).unwrap();
        let selection = OfferSelection::new("selection-1", "offer-1", destination.clone()).unwrap();

        assert_eq!(
            service.invoke(
                &session(),
                &call(None),
                projection(1),
                PickerOperation::Select(selection.clone()),
                2_000,
            ),
            Err(PickerServiceError::MissingIdempotency)
        );

        let selected = service
            .invoke(
                &session(),
                &call(Some(b"idem-1")),
                projection(1),
                PickerOperation::Select(selection.clone()),
                2_000,
            )
            .unwrap();
        let replay = service
            .invoke(
                &session(),
                &call(Some(b"idem-1")),
                projection(1),
                PickerOperation::Select(selection),
                2_000,
            )
            .unwrap();
        assert_eq!(selected, replay);

        let conflicting =
            OfferSelection::new("selection-2", "offer-1", target("browser.personal.d2b")).unwrap();
        assert_eq!(
            service.invoke(
                &session(),
                &call(Some(b"idem-1")),
                projection(1),
                PickerOperation::Select(conflicting),
                2_000,
            ),
            Err(PickerServiceError::IdempotencyConflict)
        );
    }

    #[test]
    fn malformed_and_cross_generation_input_fails_closed() {
        assert_eq!(
            PickerCall::new([0; 16], None, 1, 1_000, 2_000),
            Err(PickerServiceError::MalformedRequest)
        );
        let bad_target = ClipboardTarget::workload("browser", WorkloadProviderKind::LocalVm);
        assert_eq!(bad_target, Err(ProtocolError::InvalidTarget));

        let stale = PickerCall::new([1; 16], None, 2, 1_000, 2_000).unwrap();
        let mut service = ClipboardPickerService::default();
        assert_eq!(
            service.invoke(
                &session(),
                &stale,
                projection(0),
                PickerOperation::List(OfferQuery::new(target("browser.personal.d2b"), 8).unwrap()),
                1_500,
            ),
            Err(PickerServiceError::GenerationMismatch)
        );
    }
}
