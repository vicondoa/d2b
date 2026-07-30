//! The Provider agent's bounded audit log.
//!
//! A Provider agent records one event per dispatched method so an operator
//! can see what the agent was asked to do. The log is a bounded ring with a
//! frozen ceiling: it is diagnostic, never the authority for what happened.
//! The broker remains the independent audit owner of every privileged
//! mutation, so losing the oldest agent event can never lose the record of
//! a host effect.
//!
//! An event names only the Zone principal (the Zone path and the
//! `Provider/<name>` reference), the method token, and the closed outcome
//! class. It carries no argument, payload, digest, path, credential, or
//! caller-supplied text.

use std::collections::VecDeque;

use d2b_contracts::v3::ResourceRef;
use d2b_contracts::v3::execution_policy::BoundedToken;
use d2b_contracts::v3::zone_routing::ZonePath;

use crate::error::ProviderToolkitError;

/// The frozen audit ring capacity, and its only permitted value.
///
/// Capacity is closed rather than configurable: an operator cannot raise it
/// to make the agent hold unbounded memory, and cannot lower it to hide
/// recent events from a doctor pass.
pub const DEFAULT_AUDIT_CAPACITY: usize = 1024;

/// The closed outcome class of one dispatched Provider agent method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ProviderAgentAuditOutcome {
    /// The method completed.
    Accepted,
    /// The method was refused before any work was attempted.
    Denied,
    /// The method was attempted and failed.
    Failed,
}

impl ProviderAgentAuditOutcome {
    /// Return the stable lower-kebab code for this outcome.
    pub const fn code(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Denied => "denied",
            Self::Failed => "failed",
        }
    }
}

/// One recorded Provider agent dispatch, bound to the v3 Zone principal.
#[derive(Clone, PartialEq, Eq)]
pub struct ProviderAgentAuditEvent {
    zone: ZonePath,
    provider_ref: ResourceRef,
    method: BoundedToken,
    outcome: ProviderAgentAuditOutcome,
}

impl ProviderAgentAuditEvent {
    /// Record one dispatch.
    pub const fn new(
        zone: ZonePath,
        provider_ref: ResourceRef,
        method: BoundedToken,
        outcome: ProviderAgentAuditOutcome,
    ) -> Self {
        Self {
            zone,
            provider_ref,
            method,
            outcome,
        }
    }

    /// Borrow the Zone the agent serves.
    pub const fn zone(&self) -> &ZonePath {
        &self.zone
    }

    /// Borrow the `Provider/<name>` reference the agent implements.
    pub const fn provider_ref(&self) -> &ResourceRef {
        &self.provider_ref
    }

    /// Borrow the dispatched method token.
    pub const fn method(&self) -> &BoundedToken {
        &self.method
    }

    /// Return the closed outcome class.
    pub const fn outcome(&self) -> ProviderAgentAuditOutcome {
        self.outcome
    }
}

impl core::fmt::Debug for ProviderAgentAuditEvent {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ProviderAgentAuditEvent")
            .field("outcome", &self.outcome)
            .finish_non_exhaustive()
    }
}

/// A bounded ring of Provider agent audit events.
#[derive(Debug)]
pub struct ProviderAgentAuditLog {
    events: VecDeque<ProviderAgentAuditEvent>,
    capacity: usize,
    dropped: u64,
}

impl ProviderAgentAuditLog {
    /// Build a log at the frozen capacity.
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_AUDIT_CAPACITY)
            .expect("the frozen default capacity is in range")
    }

    /// Build a log at an explicit capacity.
    ///
    /// The capacity is closed: zero and anything above
    /// [`DEFAULT_AUDIT_CAPACITY`] are rejected, so no caller can widen the
    /// ring or disable it.
    pub fn with_capacity(capacity: usize) -> Result<Self, ProviderToolkitError> {
        if capacity == 0 || capacity > DEFAULT_AUDIT_CAPACITY {
            return Err(ProviderToolkitError::CapacityOutOfRange);
        }
        Ok(Self {
            events: VecDeque::with_capacity(capacity),
            capacity,
            dropped: 0,
        })
    }

    /// Record one event, evicting the oldest when the ring is full.
    pub fn record(&mut self, event: ProviderAgentAuditEvent) {
        if self.events.len() == self.capacity {
            self.events.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.events.push_back(event);
    }

    /// Return the ring capacity.
    pub const fn capacity(&self) -> usize {
        self.capacity
    }

    /// Return the number of retained events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Return whether the ring holds no event.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Return how many events were evicted since the log was created.
    ///
    /// A nonzero count is itself a signal: it tells a reader the ring is
    /// not a complete history.
    pub const fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Iterate the retained events, oldest first.
    pub fn events(&self) -> impl Iterator<Item = &ProviderAgentAuditEvent> {
        self.events.iter()
    }
}

impl Default for ProviderAgentAuditLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts::v3::zone_routing::ZoneLabelId;
    use d2b_contracts::v3::{ResourceName, ResourceTypeName};

    fn zone() -> ZonePath {
        ZonePath::new(vec![ZoneLabelId::parse("work").expect("valid label")])
            .expect("valid zone path")
    }

    fn provider_ref() -> ResourceRef {
        ResourceRef::new(
            ResourceTypeName::parse("Provider").expect("valid type"),
            ResourceName::parse("volume-local").expect("valid name"),
        )
    }

    fn event(method: &str) -> ProviderAgentAuditEvent {
        ProviderAgentAuditEvent::new(
            zone(),
            provider_ref(),
            BoundedToken::parse(method).expect("valid method token"),
            ProviderAgentAuditOutcome::Accepted,
        )
    }

    // Ported from the ADR45 provider agent test of the same name.
    #[test]
    fn audit_capacity_is_closed_and_bounded() {
        assert_eq!(
            ProviderAgentAuditLog::with_capacity(0).unwrap_err(),
            ProviderToolkitError::CapacityOutOfRange
        );
        assert_eq!(
            ProviderAgentAuditLog::with_capacity(DEFAULT_AUDIT_CAPACITY + 1).unwrap_err(),
            ProviderToolkitError::CapacityOutOfRange
        );

        let mut log = ProviderAgentAuditLog::with_capacity(4).expect("valid capacity");
        assert!(log.is_empty());
        for index in 0..16 {
            log.record(event(&format!("method-{index}")));
        }
        assert_eq!(log.capacity(), 4);
        assert_eq!(log.len(), 4);
        assert_eq!(log.dropped(), 12);
        let retained: Vec<&str> = log.events().map(|event| event.method().as_str()).collect();
        assert_eq!(
            retained,
            ["method-12", "method-13", "method-14", "method-15"]
        );
    }

    #[test]
    fn the_default_log_uses_the_frozen_capacity() {
        let log = ProviderAgentAuditLog::new();
        assert_eq!(log.capacity(), DEFAULT_AUDIT_CAPACITY);
        assert_eq!(log.dropped(), 0);
    }

    #[test]
    fn an_event_never_renders_its_principal_or_method_in_debug() {
        let rendered = format!("{:?}", event("launch"));
        assert!(!rendered.contains("launch"));
        assert!(!rendered.contains("work"));
        assert!(!rendered.contains("volume-local"));
    }
}
