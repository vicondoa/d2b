#![doc = "Guest-side nixling control daemon primitives."]

pub mod auth;
pub mod detached;
pub mod detached_registry;
pub mod exec;
pub mod exec_linux;
pub mod generated;
pub mod service;

use nixling_ipc::guest_wire::{
    ExecCreateRequest, ExecId, GuestBootId, GuestCapability, GuestControlErrorKind,
    GuestExecRequestMetadata, GuestSubsystem, HealthOrigin, HealthReason, HealthRemediation,
    HealthResponse, HealthState, OutputStream, ReadOutputRequest, ReadOutputResponse,
    GUEST_CONTROL_PROTOCOL_VERSION,
};

pub const MAX_HEALTH_CAPABILITIES: usize = 32;
pub const MAX_DEGRADED_SUBSYSTEMS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestHealthError {
    EmptyDegradedSubsystems,
    TooManyCapabilities,
    TooManyDegradedSubsystems,
    HostSynthesizedHealth,
    ProtocolVersionMismatch,
    HealthyWithDegradedSubsystems,
    InvalidGuestReportedMapping,
}

pub fn healthy(capabilities: Vec<GuestCapability>) -> Result<HealthResponse, GuestHealthError> {
    guest_reported(HealthResponse {
        origin: HealthOrigin::GuestReported,
        state: HealthState::Healthy,
        reason: HealthReason::None,
        remediation: HealthRemediation::None,
        protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
        capabilities,
        degraded_subsystems: Vec::new(),
    })
}

pub fn degraded(
    reason: HealthReason,
    remediation: HealthRemediation,
    capabilities: Vec<GuestCapability>,
    degraded_subsystems: Vec<GuestSubsystem>,
) -> Result<HealthResponse, GuestHealthError> {
    if degraded_subsystems.is_empty() {
        return Err(GuestHealthError::EmptyDegradedSubsystems);
    }
    guest_reported(HealthResponse {
        origin: HealthOrigin::GuestReported,
        state: HealthState::Degraded,
        reason,
        remediation,
        protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
        capabilities,
        degraded_subsystems,
    })
}

pub fn guest_reported(response: HealthResponse) -> Result<HealthResponse, GuestHealthError> {
    if response.origin != HealthOrigin::GuestReported {
        return Err(GuestHealthError::HostSynthesizedHealth);
    }
    if response.protocol_version != GUEST_CONTROL_PROTOCOL_VERSION {
        return Err(GuestHealthError::ProtocolVersionMismatch);
    }
    if response.state == HealthState::Healthy && !response.degraded_subsystems.is_empty() {
        return Err(GuestHealthError::HealthyWithDegradedSubsystems);
    }
    if response.state == HealthState::Degraded && response.degraded_subsystems.is_empty() {
        return Err(GuestHealthError::EmptyDegradedSubsystems);
    }
    if response.capabilities.len() > MAX_HEALTH_CAPABILITIES {
        return Err(GuestHealthError::TooManyCapabilities);
    }
    if response.degraded_subsystems.len() > MAX_DEGRADED_SUBSYSTEMS {
        return Err(GuestHealthError::TooManyDegradedSubsystems);
    }
    if !response.is_valid_mapping() {
        Err(GuestHealthError::InvalidGuestReportedMapping)
    } else {
        Ok(response)
    }
}

pub trait TokenSource {
    fn verify_tag(&self, transcript: &[u8], tag: &[u8]) -> Result<(), AuthError>;
    fn sign_tag(&self, transcript: &[u8]) -> Result<[u8; auth::AUTH_TAG_LEN], AuthError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthError {
    TokenUnavailable,
    MacRejected,
}

pub trait UserDirectory {
    fn resolve_user(&self, user: &str) -> Result<GuestUserIdentity, UserDirectoryError>;
}

#[derive(Clone, PartialEq, Eq)]
pub struct GuestUserIdentity {
    pub uid: u32,
    pub gid: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UserDirectoryError {
    NotFound,
    Denied,
}

pub trait ExecRuntime {
    fn create(&self, request: &ExecCreateRequest) -> Result<ExecId, GuestControlErrorKind>;
    fn cancel(&self, metadata: &GuestExecRequestMetadata) -> Result<(), GuestControlErrorKind>;
}

pub trait LogStore {
    fn read(
        &self,
        request: &ReadOutputRequest,
    ) -> Result<ReadOutputResponse, GuestControlErrorKind>;
}

pub struct GuestDaemon<H> {
    health: H,
}

impl<H> GuestDaemon<H> {
    pub fn new(health: H) -> Self {
        Self { health }
    }
}

impl<H> GuestDaemon<H>
where
    H: GuestHealthProbe,
{
    pub fn health(&self) -> Result<HealthResponse, GuestHealthError> {
        guest_reported(self.health.health()?)
    }
}

pub trait GuestHealthProbe {
    fn health(&self) -> Result<HealthResponse, GuestHealthError>;
}

pub struct StaticHealthy {
    pub capabilities: Vec<GuestCapability>,
}

impl GuestHealthProbe for StaticHealthy {
    fn health(&self) -> Result<HealthResponse, GuestHealthError> {
        healthy(self.capabilities.clone())
    }
}

pub struct LogReadCapability {
    pub exec_id: ExecId,
    pub stream: OutputStream,
    pub guest_boot_id: GuestBootId,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guestd_health_is_guest_reported_only_for_healthy() {
        let response = healthy(vec![GuestCapability::Health]).unwrap();
        assert_eq!(response.origin, HealthOrigin::GuestReported);
        assert_eq!(response.state, HealthState::Healthy);
        assert!(response.is_valid_mapping());
    }

    #[test]
    fn guestd_degraded_rejects_host_synthesized_reasons() {
        assert!(matches!(
            degraded(
                HealthReason::ListenerAbsent,
                HealthRemediation::CheckGuestdService,
                vec![GuestCapability::Health],
                vec![GuestSubsystem::Guestd],
            ),
            Err(GuestHealthError::InvalidGuestReportedMapping)
        ));
    }

    #[test]
    fn guestd_degraded_requires_bounded_subsystem() {
        assert!(matches!(
            degraded(
                HealthReason::ExecSubsystemUnavailable,
                HealthRemediation::Retry,
                vec![GuestCapability::Health],
                Vec::new(),
            ),
            Err(GuestHealthError::EmptyDegradedSubsystems)
        ));
        let response = degraded(
            HealthReason::ExecSubsystemUnavailable,
            HealthRemediation::Retry,
            vec![GuestCapability::Health],
            vec![GuestSubsystem::Exec],
        )
        .unwrap();
        assert_eq!(response.origin, HealthOrigin::GuestReported);
        assert_eq!(response.state, HealthState::Degraded);
    }

    #[test]
    fn guestd_health_rejects_unbounded_or_host_synthesized_probe_output() {
        assert!(matches!(
            healthy(vec![GuestCapability::Health; MAX_HEALTH_CAPABILITIES + 1]),
            Err(GuestHealthError::TooManyCapabilities)
        ));
        assert!(matches!(
            degraded(
                HealthReason::ExecSubsystemUnavailable,
                HealthRemediation::Retry,
                vec![GuestCapability::Health],
                vec![GuestSubsystem::Exec; MAX_DEGRADED_SUBSYSTEMS + 1],
            ),
            Err(GuestHealthError::TooManyDegradedSubsystems)
        ));
        let host_synthesized = HealthResponse {
            origin: HealthOrigin::HostSynthesized,
            state: HealthState::ListenerAbsent,
            reason: HealthReason::ListenerAbsent,
            remediation: HealthRemediation::CheckGuestdService,
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
            capabilities: Vec::new(),
            degraded_subsystems: Vec::new(),
        };
        assert!(matches!(
            guest_reported(host_synthesized),
            Err(GuestHealthError::HostSynthesizedHealth)
        ));
        let wrong_version = HealthResponse {
            origin: HealthOrigin::GuestReported,
            state: HealthState::Healthy,
            reason: HealthReason::None,
            remediation: HealthRemediation::None,
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION + 1,
            capabilities: Vec::new(),
            degraded_subsystems: Vec::new(),
        };
        assert!(matches!(
            guest_reported(wrong_version),
            Err(GuestHealthError::ProtocolVersionMismatch)
        ));
        let healthy_with_degraded = HealthResponse {
            origin: HealthOrigin::GuestReported,
            state: HealthState::Healthy,
            reason: HealthReason::None,
            remediation: HealthRemediation::None,
            protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
            capabilities: Vec::new(),
            degraded_subsystems: vec![GuestSubsystem::Exec],
        };
        assert!(matches!(
            guest_reported(healthy_with_degraded),
            Err(GuestHealthError::HealthyWithDegradedSubsystems)
        ));
    }
}
