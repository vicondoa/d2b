//! Bounded desktop-observer service core.
//!
//! The composition layer supplies an already-established ComponentSession over
//! a pre-authorized local transport. This module performs no endpoint
//! discovery and has no alternate transport or file-based control path.

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::{
    events::{EventError, SecurityKeyEvent},
    notifications::{Notification, notification_for_event},
    state::SkNotifyState,
};

pub const SERVICE_PACKAGE: &str = "d2b.notify.v2";
pub const ENDPOINT_PURPOSE: &str = "desktop-observer";
pub const ENDPOINT_ROLE: &str = "desktop-observer";
pub const SUBSCRIBE_METHOD: &str = "Subscribe";
pub const ACKNOWLEDGE_METHOD: &str = "Acknowledge";

pub const MAX_QUEUED_EVENTS: usize = 64;
pub const MAX_QUEUE_BYTES: usize = 64 * 1024;
pub const MAX_SUBSCRIPTION_EVENTS: u16 = 32;
pub const MAX_SUBSCRIPTION_BYTES: usize = 16 * 1024;
pub const MAX_OBSERVABILITY_MEASURES: usize = 4;

/// Read-only evidence exposed by the composition adapter for an established
/// ComponentSession. Implementations must derive these values from negotiated
/// session state, never request payloads.
pub trait EstablishedComponentSession {
    fn service_package(&self) -> &str;
    fn endpoint_purpose(&self) -> &str;
    fn endpoint_role(&self) -> &str;
    fn is_authenticated(&self) -> bool;
    fn uses_pre_authorized_transport(&self) -> bool;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ObserverSession {
    _private: (),
}

impl std::fmt::Debug for ObserverSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ObserverSession(<authenticated>)")
    }
}

impl ObserverSession {
    pub fn admit(
        established: &impl EstablishedComponentSession,
    ) -> Result<Self, SessionAdmissionError> {
        if !established.is_authenticated() {
            return Err(SessionAdmissionError::Unauthenticated);
        }
        if !established.uses_pre_authorized_transport() {
            return Err(SessionAdmissionError::UntrustedTransport);
        }
        if established.service_package() != SERVICE_PACKAGE
            || established.endpoint_purpose() != ENDPOINT_PURPOSE
            || established.endpoint_role() != ENDPOINT_ROLE
        {
            return Err(SessionAdmissionError::ContractMismatch);
        }
        Ok(Self { _private: () })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionAdmissionError {
    Unauthenticated,
    UntrustedTransport,
    ContractMismatch,
}

impl std::fmt::Display for SessionAdmissionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthenticated => formatter.write_str("observer session is unauthenticated"),
            Self::UntrustedTransport => {
                formatter.write_str("observer transport is not pre-authorized")
            }
            Self::ContractMismatch => {
                formatter.write_str("observer ComponentSession contract mismatch")
            }
        }
    }
}

impl std::error::Error for SessionAdmissionError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubscribeRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_sequence: Option<u64>,
    pub limit: u16,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObserverEvent {
    pub sequence: u64,
    pub observed_at_unix_ms: u64,
    pub event: SecurityKeyEvent,
}

impl std::fmt::Debug for ObserverEvent {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObserverEvent")
            .field("sequence", &self.sequence)
            .field("observed_at_unix_ms", &self.observed_at_unix_ms)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SubscriptionPage {
    pub events: Vec<ObserverEvent>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_sequence: Option<u64>,
    pub gap_before_page: bool,
    pub truncated: bool,
}

impl std::fmt::Debug for SubscriptionPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SubscriptionPage")
            .field("event_count", &self.events.len())
            .field("next_sequence", &self.next_sequence)
            .field("gap_before_page", &self.gap_before_page)
            .field("truncated", &self.truncated)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserverMeasureKind {
    AcceptedEvents,
    DroppedEvents,
    QueueDepth,
    ProjectionEntries,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ObserverMeasure {
    pub kind: ObserverMeasureKind,
    pub value: u64,
}

/// Closed, low-cardinality input for the frozen local observability provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserverObservability {
    measures: [ObserverMeasure; MAX_OBSERVABILITY_MEASURES],
}

impl ObserverObservability {
    pub fn measures(&self) -> &[ObserverMeasure] {
        &self.measures
    }
}

/// Adapter boundary implemented by composition with the frozen bounded local
/// observability provider. The sink receives only closed measures.
pub trait LocalObservabilitySink {
    type Error;

    fn project(&mut self, measures: &[ObserverMeasure]) -> Result<(), Self::Error>;
}

pub struct ObserverService {
    _session: ObserverSession,
    queue: VecDeque<ObserverEvent>,
    queue_bytes: usize,
    next_sequence: u64,
    acknowledged_through: u64,
    accepted_events: u64,
    dropped_events: u64,
    projection: SkNotifyState,
}

impl std::fmt::Debug for ObserverService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ObserverService")
            .field("queue_depth", &self.queue.len())
            .field("queue_bytes", &self.queue_bytes)
            .field("next_sequence", &self.next_sequence)
            .field("acknowledged_through", &self.acknowledged_through)
            .field("accepted_events", &self.accepted_events)
            .field("dropped_events", &self.dropped_events)
            .finish_non_exhaustive()
    }
}

impl ObserverService {
    pub fn new(session: ObserverSession, now_secs: u64) -> Self {
        Self {
            _session: session,
            queue: VecDeque::new(),
            queue_bytes: 0,
            next_sequence: 1,
            acknowledged_through: 0,
            accepted_events: 0,
            dropped_events: 0,
            projection: SkNotifyState::empty(now_secs),
        }
    }

    /// Accept an event delivered by the established service session.
    pub fn observe(
        &mut self,
        event: SecurityKeyEvent,
        observed_at_unix_ms: u64,
    ) -> Result<Option<Notification>, ObserverError> {
        event.validate().map_err(ObserverError::InvalidEvent)?;
        let sequence = self.next_sequence;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or(ObserverError::SequenceExhausted)?;
        let queued = ObserverEvent {
            sequence,
            observed_at_unix_ms,
            event,
        };
        let encoded_bytes = serde_json::to_vec(&queued)
            .map_err(ObserverError::Encoding)?
            .len();
        if encoded_bytes > MAX_SUBSCRIPTION_BYTES {
            return Err(ObserverError::EventTooLarge);
        }

        while self.queue.len() >= MAX_QUEUED_EVENTS
            || self.queue_bytes.saturating_add(encoded_bytes) > MAX_QUEUE_BYTES
        {
            let Some(discarded) = self.queue.pop_front() else {
                return Err(ObserverError::EventTooLarge);
            };
            self.queue_bytes = self
                .queue_bytes
                .saturating_sub(encoded_event_bytes(&discarded));
            self.dropped_events = self.dropped_events.saturating_add(1);
        }

        self.queue_bytes = self.queue_bytes.saturating_add(encoded_bytes);
        self.accepted_events = self.accepted_events.saturating_add(1);
        let notification = notification_for_event(&queued.event);
        let previous = std::mem::replace(
            &mut self.projection,
            SkNotifyState::empty(observed_at_unix_ms / 1_000),
        );
        self.projection = previous.apply(&queued.event, observed_at_unix_ms / 1_000);
        self.queue.push_back(queued);
        Ok(notification)
    }

    pub fn subscribe(&self, request: SubscribeRequest) -> Result<SubscriptionPage, ObserverError> {
        if request.limit == 0 || request.limit > MAX_SUBSCRIPTION_EVENTS {
            return Err(ObserverError::InvalidLimit);
        }
        let after = request.after_sequence.unwrap_or(self.acknowledged_through);
        let oldest = self.queue.front().map(|event| event.sequence);
        let gap_before_page = oldest.is_some_and(|oldest| after.saturating_add(1) < oldest);
        let mut events = Vec::new();
        let mut bytes = 2usize;
        let mut more_available = false;

        for event in self.queue.iter().filter(|event| event.sequence > after) {
            let event_bytes = encoded_event_bytes(event);
            if events.len() == usize::from(request.limit)
                || bytes.saturating_add(event_bytes) > MAX_SUBSCRIPTION_BYTES
            {
                more_available = true;
                break;
            }
            bytes = bytes.saturating_add(event_bytes);
            events.push(event.clone());
        }
        let next_sequence = events.last().map(|event| event.sequence);
        Ok(SubscriptionPage {
            events,
            next_sequence,
            gap_before_page,
            truncated: more_available,
        })
    }

    /// Idempotently acknowledge all events through `sequence`.
    pub fn acknowledge(&mut self, sequence: u64) -> Result<(), ObserverError> {
        let highest_published = self.next_sequence.saturating_sub(1);
        if sequence > highest_published {
            return Err(ObserverError::UnknownSequence);
        }
        if sequence <= self.acknowledged_through {
            return Ok(());
        }
        self.acknowledged_through = sequence;
        while self
            .queue
            .front()
            .is_some_and(|event| event.sequence <= sequence)
        {
            if let Some(event) = self.queue.pop_front() {
                self.queue_bytes = self.queue_bytes.saturating_sub(encoded_event_bytes(&event));
            }
        }
        Ok(())
    }

    pub fn projection(&self) -> &SkNotifyState {
        &self.projection
    }

    pub fn observability(&self) -> ObserverObservability {
        let projection_entries =
            self.projection.active.len() + self.projection.recent_terminal.len();
        ObserverObservability {
            measures: [
                ObserverMeasure {
                    kind: ObserverMeasureKind::AcceptedEvents,
                    value: self.accepted_events,
                },
                ObserverMeasure {
                    kind: ObserverMeasureKind::DroppedEvents,
                    value: self.dropped_events,
                },
                ObserverMeasure {
                    kind: ObserverMeasureKind::QueueDepth,
                    value: self.queue.len() as u64,
                },
                ObserverMeasure {
                    kind: ObserverMeasureKind::ProjectionEntries,
                    value: projection_entries as u64,
                },
            ],
        }
    }

    pub fn project_observability<S: LocalObservabilitySink>(
        &self,
        sink: &mut S,
    ) -> Result<(), S::Error> {
        let observability = self.observability();
        sink.project(observability.measures())
    }
}

fn encoded_event_bytes(event: &ObserverEvent) -> usize {
    serde_json::to_vec(event).map_or(MAX_SUBSCRIPTION_BYTES, |encoded| encoded.len())
}

#[derive(Debug)]
pub enum ObserverError {
    InvalidEvent(EventError),
    InvalidLimit,
    UnknownSequence,
    SequenceExhausted,
    EventTooLarge,
    Encoding(serde_json::Error),
}

impl std::fmt::Display for ObserverError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidEvent(error) => write!(formatter, "observer rejected event: {error}"),
            Self::InvalidLimit => formatter.write_str("invalid subscription bound"),
            Self::UnknownSequence => formatter.write_str("unknown observer sequence"),
            Self::SequenceExhausted => formatter.write_str("observer sequence exhausted"),
            Self::EventTooLarge => formatter.write_str("observer event exceeds projection bound"),
            Self::Encoding(_) => formatter.write_str("observer event encoding failed"),
        }
    }
}

impl std::error::Error for ObserverError {}

#[cfg(test)]
mod tests {
    use super::*;

    struct Session {
        package: &'static str,
        purpose: &'static str,
        role: &'static str,
        authenticated: bool,
        pre_authorized: bool,
    }

    impl EstablishedComponentSession for Session {
        fn service_package(&self) -> &str {
            self.package
        }

        fn endpoint_purpose(&self) -> &str {
            self.purpose
        }

        fn endpoint_role(&self) -> &str {
            self.role
        }

        fn is_authenticated(&self) -> bool {
            self.authenticated
        }

        fn uses_pre_authorized_transport(&self) -> bool {
            self.pre_authorized
        }
    }

    fn admitted() -> ObserverSession {
        ObserverSession::admit(&Session {
            package: SERVICE_PACKAGE,
            purpose: ENDPOINT_PURPOSE,
            role: ENDPOINT_ROLE,
            authenticated: true,
            pre_authorized: true,
        })
        .unwrap()
    }

    fn started(index: usize) -> SecurityKeyEvent {
        SecurityKeyEvent::Started {
            session_id: format!("s{index}"),
            vm_name: "personal".to_owned(),
            rp_id: None,
        }
    }

    #[test]
    fn admission_requires_exact_authenticated_frozen_contract() {
        let mut session = Session {
            package: SERVICE_PACKAGE,
            purpose: ENDPOINT_PURPOSE,
            role: ENDPOINT_ROLE,
            authenticated: false,
            pre_authorized: true,
        };
        assert_eq!(
            ObserverSession::admit(&session),
            Err(SessionAdmissionError::Unauthenticated)
        );
        session.authenticated = true;
        session.package = "d2b.notify.v1";
        assert_eq!(
            ObserverSession::admit(&session),
            Err(SessionAdmissionError::ContractMismatch)
        );
        session.package = SERVICE_PACKAGE;
        session.pre_authorized = false;
        assert_eq!(
            ObserverSession::admit(&session),
            Err(SessionAdmissionError::UntrustedTransport)
        );
    }

    #[test]
    fn subscription_is_count_and_byte_bounded() {
        let mut observer = ObserverService::new(admitted(), 1);
        for index in 0..(MAX_SUBSCRIPTION_EVENTS as usize + 3) {
            observer
                .observe(started(index), 1_000 + index as u64)
                .unwrap();
        }
        let page = observer
            .subscribe(SubscribeRequest {
                after_sequence: Some(0),
                limit: MAX_SUBSCRIPTION_EVENTS,
            })
            .unwrap();
        assert!(page.events.len() <= usize::from(MAX_SUBSCRIPTION_EVENTS));
        assert!(serde_json::to_vec(&page).unwrap().len() <= MAX_SUBSCRIPTION_BYTES);
        assert!(page.truncated);
    }

    #[test]
    fn overflow_is_visible_as_gap_and_closed_observation() {
        let mut observer = ObserverService::new(admitted(), 1);
        for index in 0..(MAX_QUEUED_EVENTS + 2) {
            observer
                .observe(started(index), 1_000 + index as u64)
                .unwrap();
        }
        let page = observer
            .subscribe(SubscribeRequest {
                after_sequence: Some(0),
                limit: 1,
            })
            .unwrap();
        assert!(page.gap_before_page);
        let observations = observer.observability();
        assert_eq!(observations.measures().len(), MAX_OBSERVABILITY_MEASURES);
        assert!(
            observations
                .measures()
                .iter()
                .any(|measure| measure.kind == ObserverMeasureKind::DroppedEvents
                    && measure.value > 0)
        );
        assert!(!format!("{observations:?}").contains("personal"));
    }

    #[test]
    fn observability_adapter_receives_only_the_closed_bounded_projection() {
        #[derive(Default)]
        struct Sink(Vec<ObserverMeasure>);

        impl LocalObservabilitySink for Sink {
            type Error = std::convert::Infallible;

            fn project(&mut self, measures: &[ObserverMeasure]) -> Result<(), Self::Error> {
                self.0.extend_from_slice(measures);
                Ok(())
            }
        }

        let observer = ObserverService::new(admitted(), 1);
        let mut sink = Sink::default();
        observer.project_observability(&mut sink).unwrap();
        assert_eq!(sink.0.len(), MAX_OBSERVABILITY_MEASURES);
    }

    #[test]
    fn acknowledge_is_idempotent_and_releases_queue_capacity() {
        let mut observer = ObserverService::new(admitted(), 1);
        observer.observe(started(1), 1_000).unwrap();
        observer.acknowledge(1).unwrap();
        observer.acknowledge(1).unwrap();
        assert!(
            observer
                .subscribe(SubscribeRequest {
                    after_sequence: Some(0),
                    limit: 1,
                })
                .unwrap()
                .events
                .is_empty()
        );
    }

    #[test]
    fn owned_observer_paths_have_no_legacy_endpoint_or_callback_fallback() {
        let sources = [
            include_str!("../../events.rs"),
            include_str!("../../notifications.rs"),
            include_str!("../../state.rs"),
            include_str!("../../waybar.rs"),
            include_str!("../../bin/waybar_helper.rs"),
        ]
        .join("\n");
        for forbidden in [
            "/run/d2b/usb-sk",
            "ActionNonce",
            "D2B_PUBLIC_SOCKET",
            "host_socket",
            "UnixStream::connect",
        ] {
            assert!(
                !sources.contains(forbidden),
                "legacy observer fallback: {forbidden}"
            );
        }
    }

    #[test]
    fn service_keys_exist_in_all_frozen_contract_inputs() {
        let component_session =
            include_str!("../../../../d2b-contracts/src/v2_component_session.rs");
        let services = include_str!("../../../../d2b-contracts/src/v2_services.rs");
        let observability = include_str!("../../../../d2b-provider-observability-local/src/lib.rs");
        let transport = include_str!("../../../../d2b-provider-transport-local/src/lib.rs");

        assert!(component_session.contains("DesktopObserver = 15 => \"desktop-observer\""));
        assert!(component_session.contains("NotifyV2 = 11 => \"d2b.notify.v2\""));
        assert!(services.contains("\"Subscribe\" => true, \"Acknowledge\" => true"));
        assert!(observability.contains("pub struct BoundedProjection"));
        assert!(transport.contains("never discovers endpoints"));
    }
}
