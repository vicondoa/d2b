//! Opaque systemd effect-port contract.
//!
//! The core ProviderSupervisor implements this trait.  The systemd Provider
//! itself sees only LaunchTicket and typed observations; it never receives a
//! D-Bus connection, unit name, PID, cgroup path, or user-manager handle.

use std::future::Future;

use d2b_process_conformance::{
    AdoptionCandidate, LaunchTicket, LaunchedProcess, ProcessConformanceError,
    ProcessIdentityDigest, ProcessLaunchEffectPort,
};

/// The systemd-specific effect seam.
pub trait SystemdProcessEffectPort: Send + Sync {
    /// Start a verified transient `Type=exec` service or user scope.
    fn start(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<LaunchedProcess, ProcessConformanceError>> + Send;

    /// Observe a candidate without opening a pidfd.
    fn observe(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<Option<AdoptionCandidate>, ProcessConformanceError>> + Send;

    /// Open a pidfd only after the core adapter verified the candidate.
    fn open_pidfd(
        &self,
        candidate: &AdoptionCandidate,
    ) -> impl Future<
        Output = Result<d2b_process_conformance::PidfdEvidence, ProcessConformanceError>,
    > + Send;

    /// Stop the exact opaque identity.
    fn stop(
        &self,
        identity: &ProcessIdentityDigest,
    ) -> impl Future<Output = Result<(), ProcessConformanceError>> + Send;
}

/// Adapt a systemd effect port to the neutral Process Provider seam.
pub struct EffectPortAdapter<P>(pub P);

impl<P> ProcessLaunchEffectPort for EffectPortAdapter<P>
where
    P: SystemdProcessEffectPort,
{
    async fn launch(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<LaunchedProcess, ProcessConformanceError> {
        self.0.start(ticket).await
    }

    async fn observe(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<Option<AdoptionCandidate>, ProcessConformanceError> {
        self.0.observe(ticket).await
    }

    async fn open_pidfd(
        &self,
        candidate: &AdoptionCandidate,
    ) -> Result<d2b_process_conformance::PidfdEvidence, ProcessConformanceError> {
        self.0.open_pidfd(candidate).await
    }

    async fn stop(
        &self,
        identity: &ProcessIdentityDigest,
        _class: d2b_process_conformance::StopClass,
    ) -> Result<(), ProcessConformanceError> {
        self.0.stop(identity).await
    }
}
