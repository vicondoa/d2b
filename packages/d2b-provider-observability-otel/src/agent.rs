//! Session-bound Provider agent and bounded diagnostic audit bridge.

use std::collections::VecDeque;

use d2b_contracts::v3::{
    ResourceRef,
    execution_policy::BoundedToken,
    zone_routing::{ZoneLabelId, ZonePath},
};
use d2b_provider_toolkit::{ProviderAgentAuditEvent as ToolkitAuditEvent, ProviderAgentAuditLog};
use serde::Serialize;

/// Closed Provider-agent errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderAgentError {
    /// Session admission failed.
    SessionDenied,
    /// The bounded audit ring is full.
    AuditBackpressure,
    /// A required field was malformed.
    InvalidInput,
}

impl core::fmt::Display for ProviderAgentError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::SessionDenied => "provider-agent-session-denied",
            Self::AuditBackpressure => "provider-agent-audit-backpressure",
            Self::InvalidInput => "provider-agent-input-invalid",
        })
    }
}

impl std::error::Error for ProviderAgentError {}

/// The closed outcome class for one diagnostic agent event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ProviderAgentAuditOutcome {
    /// The session or effect was accepted.
    Accepted,
    /// The session or effect was denied.
    Denied,
    /// The session or effect failed.
    Failed,
}

impl ProviderAgentAuditOutcome {
    fn toolkit(self) -> d2b_provider_toolkit::ProviderAgentAuditOutcome {
        match self {
            Self::Accepted => d2b_provider_toolkit::ProviderAgentAuditOutcome::Accepted,
            Self::Denied => d2b_provider_toolkit::ProviderAgentAuditOutcome::Denied,
            Self::Failed => d2b_provider_toolkit::ProviderAgentAuditOutcome::Failed,
        }
    }
}

/// One non-authoritative Provider agent diagnostic event.
#[derive(Clone, Serialize)]
pub struct ProviderAgentAuditEvent {
    zone: String,
    source: String,
    record_class: &'static str,
    event: String,
    transport_class: &'static str,
    authz_decision: Option<String>,
    provider: Option<String>,
    domain: Option<String>,
    outcome: ProviderAgentAuditOutcome,
}

impl core::fmt::Debug for ProviderAgentAuditEvent {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter
            .debug_struct("ProviderAgentAuditEvent")
            .field("record_class", &self.record_class)
            .field("transport_class", &self.transport_class)
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

impl ProviderAgentAuditEvent {
    fn session_connect(
        zone: &str,
        source: &str,
        event: String,
        authz_decision: String,
        outcome: ProviderAgentAuditOutcome,
    ) -> Self {
        Self {
            zone: zone.to_owned(),
            source: source.to_owned(),
            record_class: "session-connect",
            event,
            transport_class: "zone_link",
            authz_decision: Some(authz_decision),
            provider: None,
            domain: None,
            outcome,
        }
    }

    fn process_effect(
        zone: &str,
        source: &str,
        event: String,
        provider: String,
        domain: String,
        outcome: ProviderAgentAuditOutcome,
    ) -> Self {
        Self {
            zone: zone.to_owned(),
            source: source.to_owned(),
            record_class: "process-effect",
            event,
            transport_class: "zone_link",
            authz_decision: None,
            provider: Some(provider),
            domain: Some(domain),
            outcome,
        }
    }

    /// Borrow the bounded Zone identity.
    pub fn zone(&self) -> &str {
        &self.zone
    }

    /// Borrow the bounded Provider source.
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Borrow the event token.
    pub fn event(&self) -> &str {
        &self.event
    }

    /// Borrow the event token through the Provider-agent terminology.
    pub fn method(&self) -> &str {
        self.event()
    }

    /// Return the closed record class.
    pub const fn record_class(&self) -> &'static str {
        self.record_class
    }

    /// Return the transport class.
    pub const fn transport_class(&self) -> &'static str {
        self.transport_class
    }

    /// Return the closed outcome.
    pub const fn outcome(&self) -> ProviderAgentAuditOutcome {
        self.outcome
    }
}

/// A Provider agent bound to one Zone and one Provider reference.
///
/// The toolkit owns the event shape and bounded ring. This agent only adapts
/// the Provider's session lifecycle inputs to that admitted boundary; it does
/// not construct authoritative audit records or hold session authority.
#[derive(Debug)]
pub struct ProviderAgentProcess {
    zone: ZonePath,
    provider_ref: ResourceRef,
    audit: ProviderAgentAuditLog,
    events: VecDeque<ProviderAgentAuditEvent>,
    zone_name: String,
    source_name: String,
    capacity: usize,
}

impl ProviderAgentProcess {
    /// Construct a session-bound agent.
    pub fn new(
        zone: impl Into<String>,
        source: impl Into<String>,
        capacity: usize,
    ) -> Result<Self, ProviderAgentError> {
        let zone_name = zone.into();
        let source_name = source.into();
        if zone_name.is_empty() || source_name.is_empty() || capacity == 0 {
            return Err(ProviderAgentError::InvalidInput);
        }
        let zone = ZonePath::new(vec![
            ZoneLabelId::parse(&zone_name).map_err(|_| ProviderAgentError::InvalidInput)?,
        ])
        .map_err(|_| ProviderAgentError::InvalidInput)?;
        let provider_ref = ResourceRef::parse(&format!("Provider/{source_name}"))
            .map_err(|_| ProviderAgentError::InvalidInput)?;
        let audit = ProviderAgentAuditLog::with_capacity(capacity)
            .map_err(|_| ProviderAgentError::InvalidInput)?;
        Ok(Self {
            zone,
            provider_ref,
            audit,
            events: VecDeque::with_capacity(capacity),
            zone_name,
            source_name,
            capacity,
        })
    }

    /// Record a successful or denied session connection.
    pub fn session_connect(
        &mut self,
        event: impl Into<String>,
        authz_decision: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Result<(), ProviderAgentError> {
        let event = event.into();
        let authz_decision = authz_decision.into();
        let outcome = outcome.into();
        let method = parse_token(event.clone())?;
        let authz_decision = parse_token(authz_decision.clone())?;
        let outcome = parse_outcome(outcome)?;
        self.push(
            method,
            outcome,
            ProviderAgentAuditEvent::session_connect(
                &self.zone_name,
                &self.source_name,
                event,
                authz_decision.as_str().to_owned(),
                outcome,
            ),
        )
    }

    /// Record a ProcessEffect generated by the Provider agent.
    pub fn process_effect(
        &mut self,
        event: impl Into<String>,
        provider: impl Into<String>,
        domain: impl Into<String>,
        outcome: impl Into<String>,
    ) -> Result<(), ProviderAgentError> {
        let event = event.into();
        let provider = provider.into();
        let domain = domain.into();
        let outcome = outcome.into();
        let method = parse_token(event.clone())?;
        let provider = parse_token(provider.clone())?;
        let domain = parse_token(domain.clone())?;
        let outcome = parse_outcome(outcome)?;
        self.push(
            method,
            outcome,
            ProviderAgentAuditEvent::process_effect(
                &self.zone_name,
                &self.source_name,
                event,
                provider.as_str().to_owned(),
                domain.as_str().to_owned(),
                outcome,
            ),
        )
    }

    /// Drain the bounded diagnostic snapshot.
    pub fn drain(&mut self) -> impl Iterator<Item = ProviderAgentAuditEvent> {
        let events = self.events.drain(..);
        self.audit = ProviderAgentAuditLog::with_capacity(self.capacity)
            .expect("the existing Provider agent capacity remains valid");
        events
    }

    /// Number of retained events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Whether the diagnostic ring is empty.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    fn push(
        &mut self,
        method: BoundedToken,
        outcome: ProviderAgentAuditOutcome,
        event: ProviderAgentAuditEvent,
    ) -> Result<(), ProviderAgentError> {
        if self.events.len() >= self.capacity {
            return Err(ProviderAgentError::AuditBackpressure);
        }
        self.audit.record(ToolkitAuditEvent::new(
            self.zone.clone(),
            self.provider_ref.clone(),
            method,
            outcome.toolkit(),
        ));
        self.events.push_back(event);
        Ok(())
    }
}

fn parse_token(value: impl Into<String>) -> Result<BoundedToken, ProviderAgentError> {
    BoundedToken::parse(value).map_err(|_| ProviderAgentError::InvalidInput)
}

fn parse_outcome(
    value: impl Into<String>,
) -> Result<ProviderAgentAuditOutcome, ProviderAgentError> {
    match value.into().as_str() {
        "ok" | "allowed" | "accepted" => Ok(ProviderAgentAuditOutcome::Accepted),
        "denied" | "rejected" => Ok(ProviderAgentAuditOutcome::Denied),
        "error" | "failed" => Ok(ProviderAgentAuditOutcome::Failed),
        _ => Err(ProviderAgentError::InvalidInput),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_emits_session_connect_event_without_payload() {
        let mut agent = ProviderAgentProcess::new("work", "observability-otel", 2).unwrap();
        agent.session_connect("connect", "allowed", "ok").unwrap();
        let event = agent.drain().next().unwrap();
        assert_eq!(event.method(), "connect");
        assert_eq!(event.outcome(), ProviderAgentAuditOutcome::Accepted);
        assert_eq!(event.record_class(), "session-connect");
        assert_eq!(event.transport_class(), "zone_link");
        assert_eq!(event.zone(), "work");
        assert!(
            serde_json::to_string(&event)
                .unwrap()
                .contains("session-connect")
        );
    }

    #[test]
    fn agent_rejects_unknown_outcomes_without_retaining_input() {
        let mut agent = ProviderAgentProcess::new("work", "observability-otel", 2).unwrap();
        assert_eq!(
            agent.session_connect("connect", "allowed", "unexpected-value"),
            Err(ProviderAgentError::InvalidInput)
        );
        assert!(agent.is_empty());
    }
}
