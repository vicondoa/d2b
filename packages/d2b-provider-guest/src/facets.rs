//! The declared facets the provider-owned Guest effects service reaches
//! daemon state through (U10).
//!
//! The Guest family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The daemon state that
//! implementation holds - the zone's manager view (the live rows and their
//! committed Provider identities), the live controller-session generation,
//! and the Cloud Hypervisor controller-session machinery (target-session
//! establishment and the controller-owned reconcile) - crosses the provider
//! boundary as declared facets rather than as a daemon handle: every facet
//! here is a type this crate declares, an implementation of it is supplied
//! by the daemon host through the composition root (never derived from
//! caller input), and the family crate holds no daemon state type.
//!
//! The framework state machines for the qemu-media, azure-container-apps,
//! and azure-virtual-machine kinds are this crate's own in-memory
//! controllers (the same adapters the daemon drove before the move), so
//! only the manager-view reads and the Cloud Hypervisor controller session
//! cross the boundary as facets.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_resource::v3::identity::ReconnectGeneration;
use d2b_contracts_resource::v3::{
    ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid, ZoneId,
};
use d2b_resource_runtime::identity::ResourceKey;
use d2b_resource_runtime::manager::ResourceView;

use crate::driver::GuestStatusSink;

/// One Cloud Hypervisor reconcile outcome the daemon's controller session
/// reported for a Guest (the closed phase the driver effects gate on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestCloudHypervisorOutcome {
    /// The controller session reported the Guest converged.
    Ready,
    /// The controller session is still converging.
    Pending,
}

/// The daemon-supplied manager view one zone's Guest effects read through
/// (U10).
///
/// The daemon implements this trait in its composition root over the
/// zone's v3 plane: the manager view of one row (the same classified read
/// the old effects answered from the manager), the committed Provider
/// identities the plane's registry publishes (KTD7), and the live
/// controller-session generation. The family crate receives the bounded
/// reads, never a daemon state handle.
///
/// The `Err(())` arm is the closed fail-closed refusal: an unanswerable
/// plane, never a diagnostic carrier (the daemon's own facet impls hold no
/// error type to pass; a refusal is a retryable `Unavailable`, and the
/// effects map it themselves).
#[allow(clippy::result_unit_err, reason = "the closed fail-closed refusal surface: Err(()) is an unanswerable plane, never a diagnostic carrier")]
#[async_trait]
pub trait GuestManagerView: Send + Sync + 'static {
    /// The manager view of one row: `Ok(Some(view))` when the manager holds
    /// the row, `Ok(None)` when it answered that it holds no such row, and
    /// `Err` when the plane could not answer.
    async fn row_view(&self, key: &ResourceKey) -> Result<Option<ResourceView>, ()>;

    /// The committed Provider identity for one canonical Provider
    /// reference (KTD7): the plane's registry is the authority. `Err` when
    /// the plane could not answer; `Ok(None)` when it holds no committed
    /// row for the reference.
    fn committed_provider_identity(
        &self,
        provider_ref: &ResourceRef,
    ) -> Result<Option<(ResourceUid, ResourceGeneration)>, ()>;

    /// The zone's live controller-session reconnect generation, when one is
    /// enrolled. `Err` when the plane could not answer; `Ok(None)` when no
    /// session is enrolled.
    fn controller_session_generation(&self) -> Result<Option<ReconnectGeneration>, ()>;
}

/// The daemon-supplied Cloud Hypervisor controller session one zone's Guest
/// effects drive through (U10).
///
/// The daemon implements this trait in its composition root: the session
/// establishment (U13 target-session binding) and the controller-owned
/// reconcile of one Cloud Hypervisor Guest through the plane's child
/// bridge. The family crate receives the bounded calls, never a daemon
/// state handle.
#[async_trait]
pub trait CloudHypervisorGuestRuntime: Send + Sync + 'static {
    /// Establish (or re-establish) the authenticated ComponentSession of
    /// one manager-served Cloud Hypervisor Guest and register its live
    /// generation as the Zone target directory's realization authority.
    /// The reason names the closed refusal when the session could not be
    /// established.
    async fn ensure_target_session(&self, guest_ref: &ResourceRef) -> Result<(), String>;

    /// Reconcile one Cloud Hypervisor Guest through the controller session,
    /// capturing the Provider controller's status write into the sink. The
    /// error names the failure the session reported.
    async fn reconcile_guest(
        &self,
        guest_ref: &ResourceRef,
        status_sink: Option<GuestStatusSink>,
    ) -> Result<GuestCloudHypervisorOutcome, String>;
}

/// The daemon-supplied facet set one zone's Guest effects service (and the
/// driver factory that serves it) is built from (U10).
///
/// Every facet is a provider-declared trait object the daemon host supplies
/// through the composition root; none is derived from caller input, and
/// none is a daemon state type (R2, KTD7). The zone identity and the
/// controller generation the effect fences bind are part of the facet set,
/// so the factory and the driver build the same closed service from one
/// value.
#[derive(Clone)]
pub struct GuestEffectFacets {
    /// The zone the plane serves.
    pub zone: ZoneId,
    /// The controller generation every effect call binds (KTD7).
    pub controller_generation: ControllerGeneration,
    /// The zone's manager view: live rows, committed Provider identities,
    /// and the controller-session generation.
    pub manager: Arc<dyn GuestManagerView>,
    /// The zone's Cloud Hypervisor controller session.
    pub cloud_hypervisor: Arc<dyn CloudHypervisorGuestRuntime>,
}