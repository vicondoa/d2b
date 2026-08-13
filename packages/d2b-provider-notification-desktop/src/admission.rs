//! ComponentSession admission for notification streams.

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

/// Session evidence supplied by ComponentSession.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionEvidence {
    established: bool,
    authenticated: bool,
    service_package: String,
    purpose: AdmissionPurpose,
    transport: TransportClass,
}

impl SessionEvidence {
    /// Construct evidence for one session establishment attempt.
    pub fn new(
        established: bool,
        authenticated: bool,
        service_package: impl Into<String>,
        purpose: AdmissionPurpose,
        transport: TransportClass,
    ) -> Self {
        Self {
            established,
            authenticated,
            service_package: service_package.into(),
            purpose,
            transport,
        }
    }

    /// Check all fixed service, transport, and authentication requirements.
    pub fn admit(&self) -> Result<(), AdmissionError> {
        if !self.established {
            return Err(AdmissionError::SessionNotEstablished);
        }
        if !self.authenticated {
            return Err(AdmissionError::SessionUnauthenticated);
        }
        if self.service_package != crate::SERVICE_PACKAGE {
            return Err(AdmissionError::ServiceMismatch);
        }
        match (self.purpose, self.transport) {
            (AdmissionPurpose::NotificationSource, TransportClass::EnrolledNoiseKk)
            | (AdmissionPurpose::DesktopObserver, TransportClass::UnixSeqpacket) => Ok(()),
            _ => Err(AdmissionError::TransportMismatch),
        }
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
