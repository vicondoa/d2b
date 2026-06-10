#![doc = "Guest-side nixling control daemon primitives."]

use nixling_ipc::guest_wire::{
    ExecCreateRequest, ExecId, GuestBootId, GuestCapability, GuestControlErrorKind,
    GuestExecRequestMetadata, GuestSubsystem, HealthOrigin, HealthReason, HealthRemediation,
    HealthResponse, HealthState, OutputStream, ReadOutputRequest, ReadOutputResponse,
    GUEST_CONTROL_PROTOCOL_VERSION,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestHealthError {
    EmptyDegradedSubsystems,
    InvalidGuestReportedMapping,
}

pub fn healthy(capabilities: Vec<GuestCapability>) -> HealthResponse {
    HealthResponse {
        origin: HealthOrigin::GuestReported,
        state: HealthState::Healthy,
        reason: HealthReason::None,
        remediation: HealthRemediation::None,
        protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
        capabilities,
        degraded_subsystems: Vec::new(),
    }
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
    let response = HealthResponse {
        origin: HealthOrigin::GuestReported,
        state: HealthState::Degraded,
        reason,
        remediation,
        protocol_version: GUEST_CONTROL_PROTOCOL_VERSION,
        capabilities,
        degraded_subsystems,
    };
    if response.is_valid_mapping() {
        Ok(response)
    } else {
        Err(GuestHealthError::InvalidGuestReportedMapping)
    }
}

pub trait TokenSource {
    fn verify_transcript(&self, transcript: &[u8], mac: &[u8]) -> Result<(), AuthError>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
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
    pub fn health(&self) -> HealthResponse {
        self.health.health()
    }
}

pub trait GuestHealthProbe {
    fn health(&self) -> HealthResponse;
}

pub struct StaticHealthy {
    pub capabilities: Vec<GuestCapability>,
}

impl GuestHealthProbe for StaticHealthy {
    fn health(&self) -> HealthResponse {
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
        let response = healthy(vec![GuestCapability::Health]);
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
}
