use std::collections::{BTreeMap, VecDeque};

use crate::{
    audit::{AuditDecision, AuditEvent, AuditQueue, AuditQueueConfig},
    policy::{ClipboardPolicy, ReasonCode, TransferRequest},
};

const ID_BYTES: usize = 16;
const DIGEST_BYTES: usize = 32;
const MAX_OPAQUE_ID_BYTES: usize = 64;
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ControlMethod {
    Offer,
    InspectOffer,
    AcceptTransfer,
    CompleteTransfer,
    CancelTransfer,
    BridgeReady,
    Cancel,
}

impl ControlMethod {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Offer => "Offer",
            Self::InspectOffer => "InspectOffer",
            Self::AcceptTransfer => "AcceptTransfer",
            Self::CompleteTransfer => "CompleteTransfer",
            Self::CancelTransfer => "CancelTransfer",
            Self::BridgeReady => "BridgeReady",
            Self::Cancel => "Cancel",
        }
    }

    pub const fn mutating(self) -> bool {
        matches!(
            self,
            Self::Offer | Self::AcceptTransfer | Self::CompleteTransfer | Self::CancelTransfer
        )
    }

    pub const fn operation(self) -> ControlOperation {
        match self {
            Self::Offer => ControlOperation::SetState,
            Self::InspectOffer => ControlOperation::Inspect,
            Self::AcceptTransfer => ControlOperation::Attach,
            Self::CompleteTransfer => ControlOperation::SetState,
            Self::CancelTransfer | Self::Cancel => ControlOperation::Detach,
            Self::BridgeReady => ControlOperation::Health,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlPeer {
    CommandClient,
    ClipboardBridge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlTransport {
    ComponentSessionUnixSeqpacket,
    ComponentSessionUnixStream,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ControlSession {
    generation: u64,
    peer: ControlPeer,
    authenticated_realm: Option<String>,
    transport: ControlTransport,
}

impl std::fmt::Debug for ControlSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ControlSession")
            .field("generation", &"<redacted>")
            .field("peer", &self.peer)
            .field("authenticated_realm", &"<redacted>")
            .field("transport", &self.transport)
            .finish()
    }
}

impl ControlSession {
    pub fn admit(
        generation: u64,
        peer: ControlPeer,
        authenticated_realm: Option<&str>,
        transport: ControlTransport,
        component_session_authenticated: bool,
        host_local: bool,
    ) -> Result<Self, ControlError> {
        if generation == 0 {
            return Err(ControlError::GenerationMismatch);
        }
        if !component_session_authenticated || !host_local {
            return Err(ControlError::UnauthenticatedSession);
        }
        match (peer, authenticated_realm) {
            (ControlPeer::CommandClient, None) => {}
            (ControlPeer::ClipboardBridge, Some(realm)) if valid_opaque_id(realm) => {}
            _ => return Err(ControlError::Unauthorized),
        }
        Ok(Self {
            generation,
            peer,
            authenticated_realm: authenticated_realm.map(str::to_owned),
            transport,
        })
    }

    pub const fn generation(&self) -> u64 {
        self.generation
    }

    pub const fn peer(&self) -> ControlPeer {
        self.peer
    }

    pub const fn transport(&self) -> ControlTransport {
        self.transport
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct AdmittedCall {
    request_id: [u8; ID_BYTES],
    idempotency_key: Option<BoundedIdempotencyKey>,
    session_generation: u64,
    issued_at_unix_ms: u64,
    expires_at_unix_ms: u64,
}

impl std::fmt::Debug for AdmittedCall {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdmittedCall")
            .field("request_id", &"<redacted>")
            .field(
                "idempotency_key",
                &self.idempotency_key.map(|_| "<redacted>"),
            )
            .field("session_generation", &self.session_generation)
            .field("issued_at_unix_ms", &self.issued_at_unix_ms)
            .field("expires_at_unix_ms", &self.expires_at_unix_ms)
            .finish()
    }
}

impl AdmittedCall {
    pub fn new(
        request_id: [u8; ID_BYTES],
        idempotency_key: Option<&[u8]>,
        session_generation: u64,
        issued_at_unix_ms: u64,
        expires_at_unix_ms: u64,
    ) -> Result<Self, ControlError> {
        if request_id == [0; ID_BYTES] || session_generation == 0 {
            return Err(ControlError::InvalidRequest);
        }
        if issued_at_unix_ms == 0
            || expires_at_unix_ms <= issued_at_unix_ms
            || expires_at_unix_ms - issued_at_unix_ms > 15 * 60 * 1_000
        {
            return Err(ControlError::InvalidDeadline);
        }
        let idempotency_key = idempotency_key
            .map(BoundedIdempotencyKey::new)
            .transpose()?;
        Ok(Self {
            request_id,
            idempotency_key,
            session_generation,
            issued_at_unix_ms,
            expires_at_unix_ms,
        })
    }

    fn admit(
        self,
        session: &ControlSession,
        method: ControlMethod,
        now_unix_ms: u64,
    ) -> Result<(), ControlError> {
        if self.session_generation != session.generation {
            return Err(ControlError::GenerationMismatch);
        }
        if now_unix_ms < self.issued_at_unix_ms || now_unix_ms >= self.expires_at_unix_ms {
            return Err(ControlError::DeadlineExpired);
        }
        if method.mutating() && self.idempotency_key.is_none() {
            return Err(ControlError::MissingIdempotency);
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct BoundedIdempotencyKey {
    bytes: [u8; MAX_IDEMPOTENCY_KEY_BYTES],
    len: u8,
}

impl BoundedIdempotencyKey {
    fn new(value: &[u8]) -> Result<Self, ControlError> {
        let len = u8::try_from(value.len()).map_err(|_| ControlError::InvalidRequest)?;
        if len == 0 || value.len() > MAX_IDEMPOTENCY_KEY_BYTES {
            return Err(ControlError::InvalidRequest);
        }
        let mut bytes = [0; MAX_IDEMPOTENCY_KEY_BYTES];
        bytes[..value.len()].copy_from_slice(value);
        Ok(Self { bytes, len })
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct OfferIntent {
    pub offer_id: String,
    pub operation_id: String,
    pub source_realm: String,
    pub destination_realm: String,
    pub mime_type: String,
    pub byte_count: u64,
    pub request_digest: [u8; DIGEST_BYTES],
    pub expires_at_unix_ms: u64,
    pub explicit_cross_realm_allow: bool,
    pub trusted_paste_intent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfferState {
    Offered,
    Accepted,
    Completed,
    Cancelled,
}

#[derive(Clone, PartialEq, Eq)]
struct OfferRecord {
    intent: OfferIntent,
    state: OfferState,
}

#[derive(Clone, PartialEq, Eq)]
struct ReceiptBinding {
    method: ControlMethod,
    resource_id: Option<String>,
    peer: ControlPeer,
    authenticated_realm: Option<String>,
    offer_digest: Option<[u8; DIGEST_BYTES]>,
}

#[derive(Clone, PartialEq, Eq)]
struct ReceiptValue {
    binding: ReceiptBinding,
    response: ControlResponse,
}

#[derive(Clone, PartialEq, Eq)]
pub enum ControlInput {
    Offer(OfferIntent),
    InspectOffer { offer_id: String },
    AcceptTransfer { offer_id: String },
    CompleteTransfer { offer_id: String },
    CancelTransfer { offer_id: String },
    BridgeReady,
    Cancel { request_id: [u8; ID_BYTES] },
}

impl ControlInput {
    pub const fn method(&self) -> ControlMethod {
        match self {
            Self::Offer(_) => ControlMethod::Offer,
            Self::InspectOffer { .. } => ControlMethod::InspectOffer,
            Self::AcceptTransfer { .. } => ControlMethod::AcceptTransfer,
            Self::CompleteTransfer { .. } => ControlMethod::CompleteTransfer,
            Self::CancelTransfer { .. } => ControlMethod::CancelTransfer,
            Self::BridgeReady => ControlMethod::BridgeReady,
            Self::Cancel { .. } => ControlMethod::Cancel,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlOutcome {
    Succeeded,
    AlreadyApplied,
    Denied,
    Cancelled,
    DeadlineExpired,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlOperation {
    Health,
    Attach,
    Detach,
    Inspect,
    SetState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ControlResponse {
    pub outcome: ControlOutcome,
    pub offer_id: Option<String>,
    pub state: Option<OfferState>,
    pub reason: ReasonCode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ControlObservation {
    pub operation: ControlOperation,
    pub outcome: ControlOutcome,
    pub observed_at_unix_ms: u64,
    pub value: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClipboardControlConfig {
    pub max_active_offers: usize,
    pub max_observations: usize,
    pub max_idempotency_receipts: usize,
    pub audit_per_realm_quota: usize,
    pub policy: ClipboardPolicy,
}

impl Default for ClipboardControlConfig {
    fn default() -> Self {
        Self {
            max_active_offers: 64,
            max_observations: 256,
            max_idempotency_receipts: 1_024,
            audit_per_realm_quota: 1_024,
            policy: ClipboardPolicy::default(),
        }
    }
}

pub struct ClipboardControl {
    config: ClipboardControlConfig,
    offers: BTreeMap<String, OfferRecord>,
    receipts: VecDeque<BoundedIdempotencyKey>,
    receipt_values: BTreeMap<BoundedIdempotencyKey, ReceiptValue>,
    observations: VecDeque<ControlObservation>,
    audit: AuditQueue,
}

impl ClipboardControl {
    pub fn new(config: ClipboardControlConfig) -> Result<Self, ControlError> {
        if config.max_active_offers == 0
            || config.max_observations == 0
            || config.max_idempotency_receipts == 0
            || config.audit_per_realm_quota == 0
            || config.policy.max_item_bytes == 0
            || config.policy.max_held_transfers == 0
        {
            return Err(ControlError::InvalidConfig);
        }
        Ok(Self {
            audit: AuditQueue::new(AuditQueueConfig {
                per_realm_quota: config.audit_per_realm_quota,
            }),
            config,
            offers: BTreeMap::new(),
            receipts: VecDeque::new(),
            receipt_values: BTreeMap::new(),
            observations: VecDeque::new(),
        })
    }

    pub fn handle(
        &mut self,
        session: ControlSession,
        call: AdmittedCall,
        input: ControlInput,
        now_unix_ms: u64,
    ) -> Result<ControlResponse, ControlError> {
        let method = input.method();
        call.admit(&session, method, now_unix_ms)?;
        self.authorize_peer(session.peer, method)?;
        let receipt_binding = ReceiptBinding {
            method,
            resource_id: Self::input_resource_id(&input).map(ToOwned::to_owned),
            peer: session.peer,
            authenticated_realm: session.authenticated_realm.clone(),
            offer_digest: match &input {
                ControlInput::Offer(intent) => Some(intent.request_digest),
                _ => None,
            },
        };

        if let Some(key) = call.idempotency_key
            && let Some(receipt) = self.receipt_values.get(&key).cloned()
        {
            if receipt.binding != receipt_binding {
                return Err(ControlError::Conflict);
            }
            self.observe(method, ControlOutcome::AlreadyApplied, now_unix_ms);
            return Ok(receipt.response);
        }

        self.expire_offers(now_unix_ms);
        self.authorize_input(&session, &input)?;
        let response = match input {
            ControlInput::Offer(intent) => self.offer(intent, now_unix_ms)?,
            ControlInput::InspectOffer { offer_id } => self.inspect(&offer_id)?,
            ControlInput::AcceptTransfer { offer_id } => {
                self.accept(call.request_id, &offer_id, now_unix_ms)?
            }
            ControlInput::CompleteTransfer { offer_id } => self.complete(&offer_id)?,
            ControlInput::CancelTransfer { offer_id } => self.cancel_transfer(&offer_id)?,
            ControlInput::BridgeReady => ControlResponse {
                outcome: ControlOutcome::Succeeded,
                offer_id: None,
                state: None,
                reason: ReasonCode::Allowed,
            },
            ControlInput::Cancel { request_id } => {
                let _ = request_id;
                ControlResponse {
                    outcome: ControlOutcome::Cancelled,
                    offer_id: None,
                    state: None,
                    reason: ReasonCode::Allowed,
                }
            }
        };

        if method.mutating() {
            self.remember_receipt(
                call.idempotency_key.expect("admitted mutation key"),
                receipt_binding,
                response.clone(),
            );
        }
        self.observe(method, response.outcome, now_unix_ms);
        Ok(response)
    }

    pub fn drain_audit(&mut self, max_events: usize) -> Vec<AuditEvent> {
        self.audit.drain_bounded(max_events)
    }

    pub fn observations(&self) -> impl ExactSizeIterator<Item = &ControlObservation> {
        self.observations.iter()
    }

    pub fn active_offer_count(&self) -> usize {
        self.offers.len()
    }

    fn authorize_peer(&self, peer: ControlPeer, method: ControlMethod) -> Result<(), ControlError> {
        let allowed = match peer {
            ControlPeer::CommandClient => matches!(
                method,
                ControlMethod::InspectOffer
                    | ControlMethod::AcceptTransfer
                    | ControlMethod::CancelTransfer
                    | ControlMethod::Cancel
            ),
            ControlPeer::ClipboardBridge => matches!(
                method,
                ControlMethod::Offer
                    | ControlMethod::InspectOffer
                    | ControlMethod::CompleteTransfer
                    | ControlMethod::CancelTransfer
                    | ControlMethod::BridgeReady
                    | ControlMethod::Cancel
            ),
        };
        allowed.then_some(()).ok_or(ControlError::Unauthorized)
    }

    fn authorize_input(
        &self,
        session: &ControlSession,
        input: &ControlInput,
    ) -> Result<(), ControlError> {
        if session.peer != ControlPeer::ClipboardBridge {
            return Ok(());
        }
        let authenticated_realm = session
            .authenticated_realm
            .as_deref()
            .ok_or(ControlError::Unauthorized)?;
        let source_realm = match input {
            ControlInput::Offer(intent) => Some(intent.source_realm.as_str()),
            ControlInput::InspectOffer { offer_id }
            | ControlInput::CompleteTransfer { offer_id }
            | ControlInput::CancelTransfer { offer_id } => Some(
                self.offers
                    .get(offer_id)
                    .ok_or(ControlError::NotFound)?
                    .intent
                    .source_realm
                    .as_str(),
            ),
            ControlInput::BridgeReady | ControlInput::Cancel { .. } => None,
            ControlInput::AcceptTransfer { .. } => return Err(ControlError::Unauthorized),
        };
        if source_realm.is_some_and(|source| source != authenticated_realm) {
            return Err(ControlError::Unauthorized);
        }
        Ok(())
    }

    fn offer(
        &mut self,
        intent: OfferIntent,
        now_unix_ms: u64,
    ) -> Result<ControlResponse, ControlError> {
        validate_offer(&intent)?;
        if intent.expires_at_unix_ms <= now_unix_ms {
            return Err(ControlError::DeadlineExpired);
        }
        if self.offers.len() >= self.config.max_active_offers {
            return Err(ControlError::ResourceExhausted);
        }
        if self.offers.contains_key(&intent.offer_id) {
            return Err(ControlError::Conflict);
        }
        self.config
            .policy
            .validate_offer(intent.byte_count, &intent.mime_type)
            .map_err(ControlError::Policy)?;
        let offer_id = intent.offer_id.clone();
        self.offers.insert(
            offer_id.clone(),
            OfferRecord {
                intent,
                state: OfferState::Offered,
            },
        );
        Ok(ControlResponse {
            outcome: ControlOutcome::Succeeded,
            offer_id: Some(offer_id),
            state: Some(OfferState::Offered),
            reason: ReasonCode::Allowed,
        })
    }

    fn inspect(&self, offer_id: &str) -> Result<ControlResponse, ControlError> {
        let offer = self.offers.get(offer_id).ok_or(ControlError::NotFound)?;
        Ok(ControlResponse {
            outcome: ControlOutcome::Succeeded,
            offer_id: Some(offer_id.to_owned()),
            state: Some(offer.state),
            reason: ReasonCode::Allowed,
        })
    }

    fn accept(
        &mut self,
        request_id: [u8; ID_BYTES],
        offer_id: &str,
        now_unix_ms: u64,
    ) -> Result<ControlResponse, ControlError> {
        if self
            .offers
            .values()
            .filter(|offer| offer.state == OfferState::Accepted)
            .count()
            >= self.config.policy.max_held_transfers
        {
            return Err(ControlError::ResourceExhausted);
        }
        let offer = self
            .offers
            .get_mut(offer_id)
            .ok_or(ControlError::NotFound)?;
        if offer.state != OfferState::Offered {
            return Err(ControlError::Conflict);
        }
        let decision = self.config.policy.decide_transfer(&TransferRequest {
            source_realm: &offer.intent.source_realm,
            destination_realm: &offer.intent.destination_realm,
            mime_type: &offer.intent.mime_type,
            byte_count: offer.intent.byte_count,
            explicit_cross_realm_allow: offer.intent.explicit_cross_realm_allow,
            trusted_paste_intent: offer.intent.trusted_paste_intent,
            audit_available: true,
        });
        let audit = AuditEvent {
            request_id: encode_id(request_id),
            source_realm: offer.intent.source_realm.clone(),
            destination_realm: offer.intent.destination_realm.clone(),
            mime_type: offer.intent.mime_type.clone(),
            byte_count: offer.intent.byte_count,
            decision: if decision.is_ok() {
                AuditDecision::Allow
            } else {
                AuditDecision::Deny
            },
            attribution: crate::policy::AttributionQuality::ExactClient,
            reason: decision
                .as_ref()
                .err()
                .copied()
                .unwrap_or(ReasonCode::Allowed),
            timestamp_unix_ms: now_unix_ms,
        };
        self.audit
            .enqueue_fail_closed(audit)
            .map_err(ControlError::Policy)?;
        if let Err(reason) = decision {
            return Ok(ControlResponse {
                outcome: ControlOutcome::Denied,
                offer_id: Some(offer_id.to_owned()),
                state: Some(offer.state),
                reason,
            });
        }
        offer.state = OfferState::Accepted;
        Ok(ControlResponse {
            outcome: ControlOutcome::Succeeded,
            offer_id: Some(offer_id.to_owned()),
            state: Some(OfferState::Accepted),
            reason: ReasonCode::Allowed,
        })
    }

    fn complete(&mut self, offer_id: &str) -> Result<ControlResponse, ControlError> {
        let offer = self
            .offers
            .get_mut(offer_id)
            .ok_or(ControlError::NotFound)?;
        if offer.state != OfferState::Accepted {
            return Err(ControlError::Conflict);
        }
        offer.state = OfferState::Completed;
        Ok(ControlResponse {
            outcome: ControlOutcome::Succeeded,
            offer_id: Some(offer_id.to_owned()),
            state: Some(OfferState::Completed),
            reason: ReasonCode::Allowed,
        })
    }

    fn cancel_transfer(&mut self, offer_id: &str) -> Result<ControlResponse, ControlError> {
        let offer = self
            .offers
            .get_mut(offer_id)
            .ok_or(ControlError::NotFound)?;
        if matches!(offer.state, OfferState::Completed | OfferState::Cancelled) {
            return Err(ControlError::Conflict);
        }
        offer.state = OfferState::Cancelled;
        Ok(ControlResponse {
            outcome: ControlOutcome::Cancelled,
            offer_id: Some(offer_id.to_owned()),
            state: Some(OfferState::Cancelled),
            reason: ReasonCode::Allowed,
        })
    }

    fn expire_offers(&mut self, now_unix_ms: u64) {
        self.offers.retain(|_, offer| {
            offer.intent.expires_at_unix_ms > now_unix_ms
                && !matches!(offer.state, OfferState::Completed | OfferState::Cancelled)
        });
    }

    fn remember_receipt(
        &mut self,
        key: BoundedIdempotencyKey,
        binding: ReceiptBinding,
        response: ControlResponse,
    ) {
        if self
            .receipt_values
            .insert(key, ReceiptValue { binding, response })
            .is_some()
        {
            return;
        }
        self.receipts.push_back(key);
        while self.receipts.len() > self.config.max_idempotency_receipts {
            if let Some(expired) = self.receipts.pop_front() {
                self.receipt_values.remove(&expired);
            }
        }
    }

    fn input_resource_id(input: &ControlInput) -> Option<&str> {
        match input {
            ControlInput::Offer(intent) => Some(&intent.offer_id),
            ControlInput::InspectOffer { offer_id }
            | ControlInput::AcceptTransfer { offer_id }
            | ControlInput::CompleteTransfer { offer_id }
            | ControlInput::CancelTransfer { offer_id } => Some(offer_id),
            ControlInput::BridgeReady | ControlInput::Cancel { .. } => None,
        }
    }

    fn observe(
        &mut self,
        method: ControlMethod,
        outcome: ControlOutcome,
        observed_at_unix_ms: u64,
    ) {
        if self.observations.len() == self.config.max_observations {
            self.observations.pop_front();
        }
        self.observations.push_back(ControlObservation {
            operation: method.operation(),
            outcome,
            observed_at_unix_ms,
            value: 1,
        });
    }
}

fn validate_offer(intent: &OfferIntent) -> Result<(), ControlError> {
    if !valid_opaque_id(&intent.offer_id)
        || !valid_opaque_id(&intent.operation_id)
        || !valid_opaque_id(&intent.source_realm)
        || !valid_opaque_id(&intent.destination_realm)
        || intent.request_digest == [0; DIGEST_BYTES]
        || intent.expires_at_unix_ms == 0
    {
        return Err(ControlError::InvalidRequest);
    }
    Ok(())
}

fn valid_opaque_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_OPAQUE_ID_BYTES
        && value.as_bytes()[0].is_ascii_lowercase()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn encode_id(id: [u8; ID_BYTES]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(ID_BYTES * 2);
    for byte in id {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlError {
    InvalidConfig,
    InvalidRequest,
    InvalidDeadline,
    DeadlineExpired,
    MissingIdempotency,
    GenerationMismatch,
    UnauthenticatedSession,
    Unauthorized,
    NotFound,
    Conflict,
    ResourceExhausted,
    Policy(ReasonCode),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(peer: ControlPeer) -> ControlSession {
        let authenticated_realm = match peer {
            ControlPeer::CommandClient => None,
            ControlPeer::ClipboardBridge => Some("work"),
        };
        ControlSession::admit(
            7,
            peer,
            authenticated_realm,
            ControlTransport::ComponentSessionUnixSeqpacket,
            true,
            true,
        )
        .unwrap()
    }

    fn call(key: u8) -> AdmittedCall {
        AdmittedCall::new([key; 16], Some(&[key; 16]), 7, 1_000, 2_000).unwrap()
    }

    fn offer() -> OfferIntent {
        OfferIntent {
            offer_id: "offer-1".to_owned(),
            operation_id: "operation-1".to_owned(),
            source_realm: "work".to_owned(),
            destination_realm: "personal".to_owned(),
            mime_type: "text/plain".to_owned(),
            byte_count: 12,
            request_digest: [9; 32],
            expires_at_unix_ms: 10_000,
            explicit_cross_realm_allow: true,
            trusted_paste_intent: true,
        }
    }

    #[test]
    fn method_inventory_matches_frozen_service() {
        assert_eq!(
            super::super::METHODS
                .iter()
                .map(|method| method.name())
                .collect::<Vec<_>>(),
            [
                "Offer",
                "InspectOffer",
                "AcceptTransfer",
                "CompleteTransfer",
                "CancelTransfer",
                "BridgeReady",
                "Cancel",
            ]
        );
        assert!(ControlMethod::Offer.mutating());
        assert!(!ControlMethod::InspectOffer.mutating());
        assert!(!ControlMethod::BridgeReady.mutating());
    }

    #[test]
    fn session_admission_has_no_legacy_or_unauthenticated_mode() {
        assert_eq!(
            ControlSession::admit(
                7,
                ControlPeer::CommandClient,
                None,
                ControlTransport::ComponentSessionUnixStream,
                false,
                true,
            ),
            Err(ControlError::UnauthenticatedSession)
        );
        assert_eq!(
            ControlSession::admit(
                7,
                ControlPeer::ClipboardBridge,
                None,
                ControlTransport::ComponentSessionUnixStream,
                true,
                true,
            ),
            Err(ControlError::Unauthorized)
        );
        assert_eq!(
            ControlSession::admit(
                7,
                ControlPeer::CommandClient,
                None,
                ControlTransport::ComponentSessionUnixStream,
                true,
                false,
            ),
            Err(ControlError::UnauthenticatedSession)
        );
    }

    #[test]
    fn mutating_calls_require_idempotency_and_current_generation() {
        let no_key = AdmittedCall::new([1; 16], None, 7, 1_000, 2_000).unwrap();
        assert_eq!(
            no_key.admit(
                &session(ControlPeer::ClipboardBridge),
                ControlMethod::Offer,
                1_500
            ),
            Err(ControlError::MissingIdempotency)
        );
        let stale = AdmittedCall::new([1; 16], Some(&[1; 16]), 8, 1_000, 2_000).unwrap();
        assert_eq!(
            stale.admit(
                &session(ControlPeer::ClipboardBridge),
                ControlMethod::Offer,
                1_500
            ),
            Err(ControlError::GenerationMismatch)
        );
        assert!(AdmittedCall::new([1; 16], Some(&[1; 64]), 7, 1_000, 2_000).is_ok());
        assert_eq!(
            AdmittedCall::new([1; 16], Some(&[1; 65]), 7, 1_000, 2_000),
            Err(ControlError::InvalidRequest)
        );
    }

    #[test]
    fn offer_accept_complete_is_bounded_and_audited() {
        let mut service = ClipboardControl::new(ClipboardControlConfig::default()).unwrap();
        let offered = service
            .handle(
                session(ControlPeer::ClipboardBridge),
                call(1),
                ControlInput::Offer(offer()),
                1_500,
            )
            .unwrap();
        assert_eq!(offered.state, Some(OfferState::Offered));

        let accepted = service
            .handle(
                session(ControlPeer::CommandClient),
                call(2),
                ControlInput::AcceptTransfer {
                    offer_id: "offer-1".to_owned(),
                },
                1_500,
            )
            .unwrap();
        assert_eq!(accepted.state, Some(OfferState::Accepted));
        let audit = service.drain_audit(8);
        assert_eq!(audit.len(), 1);
        assert_eq!(audit[0].decision, AuditDecision::Allow);

        let completed = service
            .handle(
                session(ControlPeer::ClipboardBridge),
                call(3),
                ControlInput::CompleteTransfer {
                    offer_id: "offer-1".to_owned(),
                },
                1_500,
            )
            .unwrap();
        assert_eq!(completed.state, Some(OfferState::Completed));
        assert_eq!(service.observations().len(), 3);
    }

    #[test]
    fn cross_realm_denial_is_audited_before_transfer() {
        let mut service = ClipboardControl::new(ClipboardControlConfig::default()).unwrap();
        let mut denied_offer = offer();
        denied_offer.explicit_cross_realm_allow = false;
        service
            .handle(
                session(ControlPeer::ClipboardBridge),
                call(1),
                ControlInput::Offer(denied_offer),
                1_500,
            )
            .unwrap();
        let denied = service
            .handle(
                session(ControlPeer::CommandClient),
                call(2),
                ControlInput::AcceptTransfer {
                    offer_id: "offer-1".to_owned(),
                },
                1_500,
            )
            .unwrap();
        assert_eq!(denied.outcome, ControlOutcome::Denied);
        assert_eq!(denied.reason, ReasonCode::PolicyDenied);
        assert_eq!(service.drain_audit(8)[0].decision, AuditDecision::Deny);
    }

    #[test]
    fn peer_method_matrix_rejects_authority_confusion() {
        let mut service = ClipboardControl::new(ClipboardControlConfig::default()).unwrap();
        assert_eq!(
            service.handle(
                session(ControlPeer::CommandClient),
                call(1),
                ControlInput::Offer(offer()),
                1_500,
            ),
            Err(ControlError::Unauthorized)
        );
        assert_eq!(
            service.handle(
                session(ControlPeer::ClipboardBridge),
                call(2),
                ControlInput::AcceptTransfer {
                    offer_id: "offer-1".to_owned(),
                },
                1_500,
            ),
            Err(ControlError::Unauthorized)
        );
    }

    #[test]
    fn bridge_identity_cannot_spoof_or_inspect_another_realm() {
        let mut service = ClipboardControl::new(ClipboardControlConfig::default()).unwrap();
        let personal = ControlSession::admit(
            7,
            ControlPeer::ClipboardBridge,
            Some("personal"),
            ControlTransport::ComponentSessionUnixSeqpacket,
            true,
            true,
        )
        .unwrap();
        assert_eq!(
            service.handle(
                personal.clone(),
                call(1),
                ControlInput::Offer(offer()),
                1_500,
            ),
            Err(ControlError::Unauthorized)
        );
        service
            .handle(
                session(ControlPeer::ClipboardBridge),
                call(2),
                ControlInput::Offer(offer()),
                1_500,
            )
            .unwrap();
        assert_eq!(
            service.handle(
                personal,
                AdmittedCall::new([3; 16], None, 7, 1_000, 2_000).unwrap(),
                ControlInput::InspectOffer {
                    offer_id: "offer-1".to_owned(),
                },
                1_500,
            ),
            Err(ControlError::Unauthorized)
        );
    }

    #[test]
    fn idempotency_receipts_are_bounded_and_do_not_repeat_mutation() {
        let config = ClipboardControlConfig {
            max_idempotency_receipts: 1,
            ..ClipboardControlConfig::default()
        };
        let mut service = ClipboardControl::new(config).unwrap();
        service
            .handle(
                session(ControlPeer::ClipboardBridge),
                call(1),
                ControlInput::Offer(offer()),
                1_500,
            )
            .unwrap();
        let duplicate = service
            .handle(
                session(ControlPeer::ClipboardBridge),
                call(1),
                ControlInput::Offer(offer()),
                1_500,
            )
            .unwrap();
        assert_eq!(duplicate.outcome, ControlOutcome::Succeeded);
        assert_eq!(duplicate.state, Some(OfferState::Offered));
        assert_eq!(service.active_offer_count(), 1);
    }
}
