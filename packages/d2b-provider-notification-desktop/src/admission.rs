//! ComponentSession admission for notification streams.

use d2b_contracts::v3::{EvidenceClass, ResourceRef, ZoneId};
use d2b_provider_toolkit::{AuthenticatedComponentSession, AuthenticatedSessionRouteBinding};

/// Stream admission purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionPurpose {
    /// Guest source to host sink stream.
    NotificationSource,
    /// Local desktop observer stream.
    DesktopObserver,
}

/// Transport class used by a notification session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportClass {
    /// Enrolled Noise KK transport.
    EnrolledNoiseKk,
    /// Local Unix seqpacket with SO_PEERCRED admission.
    UnixSeqpacket,
    /// Any other transport.
    Other,
}

/// Authenticated notification session projected from ComponentSession.
///
/// There is no public constructor from booleans, strings, or observer labels.
/// The two admission constructors consume the route metadata produced by the
/// canonical session authority.
#[derive(Debug, PartialEq, Eq)]
pub struct SessionEvidence {
    subject_ref: ResourceRef,
    zone: ZoneId,
    generation: u64,
    purpose: AdmissionPurpose,
    transport: TransportClass,
}

impl SessionEvidence {
    /// Admit a notification session directly from the canonical ComponentSession.
    pub fn from_component_session<C>(
        session: &AuthenticatedComponentSession<C>,
    ) -> Result<Self, AdmissionError> {
        let evidence = Self::from_route_binding(session.route_binding())?;
        evidence.admit()?;
        Ok(evidence)
    }

    /// Admit a Guest source from an authenticated enrolled session.
    #[allow(dead_code)]
    pub(crate) fn from_source_route(
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<Self, AdmissionError> {
        validate_route(&route, EvidenceClass::EnrolledKk, false)?;
        Ok(Self {
            subject_ref: route.subject_ref().clone(),
            zone: route.zone().clone(),
            generation: route.reconnect_generation().get(),
            purpose: AdmissionPurpose::NotificationSource,
            transport: TransportClass::EnrolledNoiseKk,
        })
    }

    /// Admit a local desktop observer from authenticated Unix peer evidence.
    #[allow(dead_code)]
    pub(crate) fn from_observer_route(
        route: AuthenticatedSessionRouteBinding,
    ) -> Result<Self, AdmissionError> {
        validate_route(&route, EvidenceClass::UnixPeer, true)?;
        Ok(Self {
            subject_ref: route.subject_ref().clone(),
            zone: route.zone().clone(),
            generation: route.reconnect_generation().get(),
            purpose: AdmissionPurpose::DesktopObserver,
            transport: TransportClass::UnixSeqpacket,
        })
    }

    /// Check all fixed service, transport, and authentication requirements.
    pub fn admit(&self) -> Result<(), AdmissionError> {
        if self.generation == 0 {
            return Err(AdmissionError::SessionNotEstablished);
        }
        match (self.purpose, self.transport) {
            (AdmissionPurpose::NotificationSource, TransportClass::EnrolledNoiseKk)
            | (AdmissionPurpose::DesktopObserver, TransportClass::UnixSeqpacket) => Ok(()),
            _ => Err(AdmissionError::TransportMismatch),
        }
    }

    fn from_route_binding(route: AuthenticatedSessionRouteBinding) -> Result<Self, AdmissionError> {
        let subject_type = route.subject_ref().resource_type().as_str();
        let (purpose, transport, expected_evidence, local_only) = match subject_type {
            "Guest" => (
                AdmissionPurpose::NotificationSource,
                TransportClass::EnrolledNoiseKk,
                EvidenceClass::EnrolledKk,
                false,
            ),
            "User" => (
                AdmissionPurpose::DesktopObserver,
                TransportClass::UnixSeqpacket,
                EvidenceClass::UnixPeer,
                true,
            ),
            _ => return Err(AdmissionError::SessionUnauthenticated),
        };
        validate_route(&route, expected_evidence, local_only)?;
        Ok(Self {
            subject_ref: route.subject_ref().clone(),
            zone: route.zone().clone(),
            generation: route.reconnect_generation().get(),
            purpose,
            transport,
        })
    }

    /// Admit the session specifically for host-observer delivery and action
    /// invocation.
    pub fn admit_observer(&self) -> Result<(), AdmissionError> {
        self.admit()?;
        if self.purpose == AdmissionPurpose::DesktopObserver {
            Ok(())
        } else {
            Err(AdmissionError::TransportMismatch)
        }
    }

    /// Admit the session specifically for a Guest notification source.
    pub fn admit_source(&self) -> Result<(), AdmissionError> {
        self.admit()?;
        if self.purpose == AdmissionPurpose::NotificationSource {
            Ok(())
        } else {
            Err(AdmissionError::TransportMismatch)
        }
    }

    /// Return the subject/Zone/generation binding used for nonce state.
    pub fn session_key(&self) -> String {
        format!(
            "{}@{}#{}",
            self.subject_ref.to_canonical_string(),
            self.zone.as_str(),
            self.generation
        )
    }

    /// Borrow the authenticated subject reference for exact source binding.
    pub const fn subject_ref(&self) -> &ResourceRef {
        &self.subject_ref
    }

    /// Whether this evidence is a Guest source session.
    pub const fn is_source(&self) -> bool {
        matches!(self.purpose, AdmissionPurpose::NotificationSource)
    }

    /// Borrow the authenticated Zone.
    pub fn zone(&self) -> &ZoneId {
        &self.zone
    }

    /// Return the authenticated reconnect generation.
    pub const fn generation(&self) -> u64 {
        self.generation
    }
}

#[cfg(test)]
pub(crate) fn test_observer(subject: &str) -> SessionEvidence {
    let subject_ref = format!("User/{subject}");
    SessionEvidence {
        subject_ref: ResourceRef::parse(&subject_ref).unwrap(),
        zone: ZoneId::parse("work").unwrap(),
        generation: 1,
        purpose: AdmissionPurpose::DesktopObserver,
        transport: TransportClass::UnixSeqpacket,
    }
}

#[cfg(test)]
pub(crate) fn test_source(subject: &str) -> SessionEvidence {
    test_source_at(subject, 1)
}

#[cfg(test)]
pub(crate) fn test_source_at(subject: &str, generation: u64) -> SessionEvidence {
    test_source_at_zone(subject, generation, "work")
}

#[cfg(test)]
pub(crate) fn test_source_at_zone(subject: &str, generation: u64, zone: &str) -> SessionEvidence {
    let subject_ref = format!("Guest/{subject}");
    SessionEvidence {
        subject_ref: ResourceRef::parse(&subject_ref).unwrap(),
        zone: ZoneId::parse(zone).unwrap(),
        generation,
        purpose: AdmissionPurpose::NotificationSource,
        transport: TransportClass::EnrolledNoiseKk,
    }
}

fn validate_route(
    route: &AuthenticatedSessionRouteBinding,
    expected_evidence: EvidenceClass,
    local_only: bool,
) -> Result<(), AdmissionError> {
    if route.service().as_str() != crate::SERVICE_PACKAGE {
        return Err(AdmissionError::ServiceMismatch);
    }
    if route
        .provider_ref()
        .is_none_or(|provider| provider.to_canonical_string() != crate::PROVIDER_REF)
        || route.provider_generation().is_none()
    {
        return Err(AdmissionError::SessionUnauthenticated);
    }
    if route.evidence_class() != expected_evidence
        || (local_only && route.locality() != d2b_contracts::v3::Locality::Local)
    {
        return Err(AdmissionError::TransportMismatch);
    }
    let subject_type = route.subject_ref().resource_type().as_str();
    if (local_only && subject_type != "User") || (!local_only && subject_type != "Guest") {
        return Err(AdmissionError::SessionUnauthenticated);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn purpose_specific_admission_rejects_cross_role_reuse() {
        let observer = test_observer("alice");
        let source = test_source("guest");
        assert!(observer.admit_observer().is_ok());
        assert!(observer.admit_source().is_err());
        assert!(source.admit_source().is_ok());
        assert!(source.admit_observer().is_err());
    }

    #[test]
    fn session_key_binds_subject_zone_and_reconnect_generation() {
        let first = test_observer("alice");
        let second = test_observer("bob");
        assert_ne!(first.session_key(), second.session_key());
        assert_eq!(first.zone().as_str(), "work");
        assert_eq!(first.generation(), 1);
    }

    #[test]
    fn zero_reconnect_generation_is_not_admitted() {
        assert_eq!(
            test_source_at("guest", 0).admit(),
            Err(AdmissionError::SessionNotEstablished)
        );
    }
}

/// Stable session admission failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmissionError {
    /// Session handshake is incomplete.
    SessionNotEstablished,
    /// Session has no authenticated caller.
    SessionUnauthenticated,
    /// Service package is not the v3 package.
    ServiceMismatch,
    /// Transport is not permitted for the selected purpose.
    TransportMismatch,
}

impl core::fmt::Display for AdmissionError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        formatter.write_str(match self {
            Self::SessionNotEstablished => "session-not-established",
            Self::SessionUnauthenticated => "session-unauthenticated",
            Self::ServiceMismatch => "session-service-mismatch",
            Self::TransportMismatch => "session-untrusted-transport",
        })
    }
}

impl std::error::Error for AdmissionError {}
