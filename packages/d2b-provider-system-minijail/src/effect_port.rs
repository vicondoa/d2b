//! Opaque privileged effect-port seam for system-minijail.

use std::future::Future;

use d2b_process_conformance::{
    AdoptionCandidate, LaunchTicket, LaunchedProcess, PidfdEvidence, ProcessConformanceError,
    ProcessIdentityDigest, ProcessLaunchEffectPort, StopClass,
};

/// The core-owned minijail effect port.
pub trait MinijailProcessEffectPort: Send + Sync {
    /// Spawn through the broker's clone3 effect.
    fn spawn(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<LaunchedProcess, ProcessConformanceError>> + Send;

    /// Find a candidate without using pidfd readability as identity.
    fn observe(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<Option<AdoptionCandidate>, ProcessConformanceError>> + Send;

    /// Duplicate a verified pidfd without waiting or reaping.
    fn duplicate_pidfd(
        &self,
        candidate: &AdoptionCandidate,
    ) -> impl Future<Output = Result<PidfdEvidence, ProcessConformanceError>> + Send;

    /// Perform exact-main stop and the mandatory anchored-leaf cleanup.
    fn stop(
        &self,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> impl Future<Output = Result<(), ProcessConformanceError>> + Send;
}

/// Adapt the minijail-specific effect port to the neutral conformance seam.
pub struct EffectPortAdapter<P>(pub P);

impl<P> ProcessLaunchEffectPort for EffectPortAdapter<P>
where
    P: MinijailProcessEffectPort,
{
    async fn launch(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<LaunchedProcess, ProcessConformanceError> {
        self.0.spawn(ticket).await
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
    ) -> Result<PidfdEvidence, ProcessConformanceError> {
        self.0.duplicate_pidfd(candidate).await
    }

    async fn stop(
        &self,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> Result<(), ProcessConformanceError> {
        self.0.stop(identity, class).await
    }
}
