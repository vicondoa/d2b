//! Identity-free drift observation and reconcile decisions.

use crate::controller::NetworkEffectError;

/// One bounded host and guest-agent observation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetworkObservation {
    /// Projection-scoped host firewall digest matches.
    pub firewall_matches: bool,
    /// Every bridge IPv6 sysctl matches.
    pub sysctls_match: bool,
    /// Every bridge-port isolation flag matches.
    pub bridge_ports_match: bool,
    /// Peer CIDRs remain conflict free.
    pub cidrs_conflict_free: bool,
    /// The external physical-NIC authority proof remains valid.
    pub external_authority_ready: bool,
    /// The guest agent confirmed dnsmasq binding.
    pub dnsmasq_bound: bool,
    /// The guest agent confirmed firewall application.
    pub guest_firewall_applied: bool,
}

/// Closed observation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObserveDecision {
    /// No drift was found.
    Current,
    /// Reconcile is required.
    Requeue,
    /// External authority ambiguity blocks recreation.
    Blocked,
}

/// Evaluate one observation without exposing any observed value.
pub fn evaluate_observation(
    observation: NetworkObservation,
) -> Result<ObserveDecision, NetworkEffectError> {
    if !observation.external_authority_ready {
        return Ok(ObserveDecision::Blocked);
    }
    if !observation.cidrs_conflict_free {
        return Err(NetworkEffectError::CidrConflict);
    }
    if observation.firewall_matches
        && observation.sysctls_match
        && observation.bridge_ports_match
        && observation.dnsmasq_bound
        && observation.guest_firewall_applied
    {
        Ok(ObserveDecision::Current)
    } else {
        Ok(ObserveDecision::Requeue)
    }
}
