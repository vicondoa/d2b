//! The provider-facing typed effects seam the Process driver needs (U1).
//!
//! The family's own implementation ([`crate::effects_service`]) serves the
//! seam over the daemon-supplied declared facets
//! ([`crate::facets::ProcessEffectFacets`]); test doubles implement the same
//! seam (R4: the conversion is mechanical, the provider effects are
//! preserved). The classification types below are the closed results that
//! seam reports.

use std::time::Duration;

use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ProcessSpec};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};
use d2b_process_conformance::{AdoptionCandidate, ProcessIdentityDigest, ProcessStatusReport};
use d2b_resource_runtime::context::ResourceContext;

use crate::identity::{ProcessFamilySpec, ProcessResourceIdentity};
use crate::worker_launch::DeviceWorkerLaunch;

/// The provider-facing effect surface the Process driver needs. The
/// family's implementation ([`crate::effects_service::ProcessEffectsService`])
/// runs over the daemon-supplied facets; test doubles implement the same
/// seam (R4: the conversion is mechanical, the provider effects are
/// preserved).
///
/// Object-erased on purpose: the driver holds the surface as
/// `Arc<dyn ProcessDriverEffects>` so one factory serves every Process row.
#[async_trait::async_trait]
pub trait ProcessDriverEffects: Send + Sync + 'static {
    /// Launch through the signed provider-ticket path.
    async fn launch(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String>;

    /// Launch one one-shot process through the preserved ephemeral ticket
    /// (old `launch_ephemeral_resource`; `start_deadline` is the timeout).
    async fn launch_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
        timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String>;

    /// Probe-and-adopt over pidfd/proc evidence with the preserved
    /// Adopt/Stale/Quarantined classification.
    async fn adopt(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String>;

    /// Probe one already-started durable process (old `probe_record`): the
    /// Alive/Exited/Unknown liveness classification drives the steady-state
    /// observation of a process this actor adopted or launched, and the
    /// provider clears its exact local authority when the process is gone.
    async fn probe(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String>;

    /// Probe-and-adopt one one-shot process (old
    /// `adopt_ephemeral_resource`): `Absent` is also the observed exit of a
    /// process this driver launched, because the provider clears its local
    /// authority for the missing identity.
    async fn adopt_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String>;

    /// Probe one already-started one-shot identity (old
    /// `probe_ephemeral_resource`): the Alive/Exited/Unknown liveness
    /// classification drives the steady-state observation, and the provider
    /// clears its local authority when the exact process is gone.
    async fn probe_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String>;

    /// Preserved term-then-kill escalation with pidfd retry; `Ok(killed)`
    /// reports whether the kill stage ran.
    async fn stop(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String>;

    /// Derive the typed launch parameters of one declared Device-owned worker
    /// row (`U17` gap closure). The daemon host resolves the
    /// Device-family-specific inputs behind the runtime facet (it may name
    /// the device families), and this crate receives the already-resolved
    /// typed parameters; a row no Device worker template declares yields
    /// `Ok(None)`, and a declared template whose trusted inputs cannot be
    /// resolved yields the named refusal code.
    async fn device_worker_launch(
        &self,
        _ctx: &mut ResourceContext,
        _identity: &ProcessResourceIdentity,
        _spec: &ProcessFamilySpec,
    ) -> Result<Option<DeviceWorkerLaunch>, &'static str> {
        Ok(None)
    }

    /// Stop one exact one-shot identity (old `stop_ephemeral_resource`).
    async fn stop_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String>;

    /// Stop one exactly-identified stale candidate before a fresh launch.
    async fn stop_stale(
        &self,
        provider_ref: &ResourceRef,
        candidate: &AdoptionCandidate,
    ) -> Result<(), String>;

    /// Remove the provider's exact local authority after a terminal exit.
    async fn finalize(&self, identity: &ProcessResourceIdentity) -> Result<(), String>;

    /// Whether this zone retains a verified identity for the resource.
    fn has_active(
        &self,
        zone: &ZoneId,
        zone_uid: Option<&ResourceUid>,
        resource_ref: &ResourceRef,
    ) -> bool;
}

/// Result of a Provider-backed adoption attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAdoption {
    /// No process matching the trusted ticket is running.
    Absent,
    /// The exact process was adopted.
    Adopted(ProcessStatusReport),
    /// A static Provider controller was found without its exact bootstrap
    /// endpoint retained by this daemon.
    ControllerBootstrapMissing,
    /// A uniquely identified stale process is available for exact replacement.
    Stale {
        /// Opaque effect-owner evidence for the exact stale process.
        candidate: AdoptionCandidate,
    },
    /// A candidate was present but identity was ambiguous and quarantined.
    Quarantined(ProcessStatusReport),
}

/// Provider-backed liveness result used by the daemon readiness loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderLiveness {
    /// The exact process is still present.
    Alive,
    /// The exact process is absent.
    Exited,
    /// Identity could not be established safely.
    Unknown,
}
