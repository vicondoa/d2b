//! Production composition seam for the GPU Provider.
//!
//! The daemon supplies one implementation that talks to Core's authority
//! index and typed broker dispatcher. This crate never imports broker DTOs or
//! receives host paths.

use d2b_contracts::v3::ResourceGeneration;

use crate::{
    GpuAuthorityAdmission, GpuAuthorityLease, GpuClosureProof, GpuEffectError, GpuEffectTokenSet,
    GpuLaunchTicket, GpuLifecycleEffectPort, GpuPlatformToken, GpuProcessIdentity,
    GpuProcessObservation, GpuWorkerSpec, VideoWorkerSpec,
};

/// Daemon-owned GPU dispatch surface.
///
/// Implementations must reserve the Host-global key before opening devices or
/// spawning a worker, retain the lease across asynchronous broker effects, and
/// release it only after exact process closure.
pub trait GpuBrokerDispatcher {
    /// Reserve the Core-derived Host-global GPU authority.
    fn reserve_gpu_authority(
        &mut self,
        admission: &GpuAuthorityAdmission,
    ) -> Result<GpuAuthorityLease, GpuEffectError>;

    /// Open the Core-resolved device grants.
    fn open_gpu_devices(
        &mut self,
        admission: &GpuAuthorityAdmission,
        tokens: &GpuEffectTokenSet,
    ) -> Result<GpuLaunchTicket, GpuEffectError>;

    /// Start the GPU or render-node worker through SpawnRunner.
    fn spawn_gpu_worker(
        &mut self,
        spec: &GpuWorkerSpec,
        ticket: &GpuLaunchTicket,
        principal: &crate::GpuPrincipalToken,
        platform: &GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> Result<GpuProcessIdentity, GpuEffectError>;

    /// Start the separate video worker through SpawnRunner.
    fn spawn_video_worker(
        &mut self,
        spec: &VideoWorkerSpec,
        ticket: &GpuLaunchTicket,
        principal: &crate::GpuPrincipalToken,
        platform: &GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> Result<GpuProcessIdentity, GpuEffectError>;

    /// Observe one exact pidfd-backed worker identity.
    fn observe_gpu_worker(
        &mut self,
        identity: &GpuProcessIdentity,
    ) -> Result<GpuProcessObservation, GpuEffectError>;

    /// Stop one exact pidfd-backed worker identity.
    fn stop_gpu_worker(
        &mut self,
        identity: &GpuProcessIdentity,
    ) -> Result<GpuClosureProof, GpuEffectError>;

    /// Release authority after all owned worker closure proofs are present.
    fn release_gpu_authority(
        &mut self,
        lease: GpuAuthorityLease,
        closures: &[GpuClosureProof],
    ) -> Result<(), GpuEffectError>;
}

/// Typed Provider port backed by one daemon dispatcher.
pub struct ProductionPort<D> {
    dispatcher: D,
}

impl<D> ProductionPort<D> {
    /// Bind the Provider to a daemon-owned dispatcher.
    pub const fn new(dispatcher: D) -> Self {
        Self { dispatcher }
    }

    /// Borrow the dispatcher for diagnostics and tests.
    pub const fn dispatcher(&self) -> &D {
        &self.dispatcher
    }

    /// Mutably borrow the dispatcher for supervisor integration.
    pub const fn dispatcher_mut(&mut self) -> &mut D {
        &mut self.dispatcher
    }
}

impl<D: GpuBrokerDispatcher> GpuLifecycleEffectPort for ProductionPort<D> {
    fn reserve_authority(
        &mut self,
        admission: &GpuAuthorityAdmission,
    ) -> Result<GpuAuthorityLease, GpuEffectError> {
        self.dispatcher.reserve_gpu_authority(admission)
    }

    fn open_authorized_devices(
        &mut self,
        admission: &GpuAuthorityAdmission,
        tokens: &GpuEffectTokenSet,
    ) -> Result<GpuLaunchTicket, GpuEffectError> {
        self.dispatcher.open_gpu_devices(admission, tokens)
    }

    fn start_gpu_worker(
        &mut self,
        spec: &GpuWorkerSpec,
        ticket: &GpuLaunchTicket,
        principal: &crate::GpuPrincipalToken,
        platform: &GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> Result<GpuProcessIdentity, GpuEffectError> {
        self.dispatcher
            .spawn_gpu_worker(spec, ticket, principal, platform, generation)
    }

    fn start_video_worker(
        &mut self,
        spec: &VideoWorkerSpec,
        ticket: &GpuLaunchTicket,
        principal: &crate::GpuPrincipalToken,
        platform: &GpuPlatformToken,
        generation: ResourceGeneration,
    ) -> Result<GpuProcessIdentity, GpuEffectError> {
        self.dispatcher
            .spawn_video_worker(spec, ticket, principal, platform, generation)
    }

    fn observe_worker(
        &mut self,
        identity: &GpuProcessIdentity,
    ) -> Result<GpuProcessObservation, GpuEffectError> {
        self.dispatcher.observe_gpu_worker(identity)
    }

    fn stop_worker(
        &mut self,
        identity: &GpuProcessIdentity,
    ) -> Result<GpuClosureProof, GpuEffectError> {
        self.dispatcher.stop_gpu_worker(identity)
    }

    fn release_authority(
        &mut self,
        lease: GpuAuthorityLease,
        closures: &[GpuClosureProof],
    ) -> Result<(), GpuEffectError> {
        self.dispatcher.release_gpu_authority(lease, closures)
    }
}
