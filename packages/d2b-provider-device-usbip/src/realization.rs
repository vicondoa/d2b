//! The converted USB Service realization (U20).
//!
//! This module is the family half of the `DeviceHelperLeg` production path. The
//! `Device` source mints the admitted relationship and the bounded leg (see
//! [`crate::facets::UsbipHelperLegSource`]); this module asks for that pair,
//! hands it to the family's own [`UsbipController`], and reports the closed
//! phase the controller reached.
//!
//! Three properties are the point of the shape:
//!
//! - the family names only what its committed Service row declares. The relay's
//!   store-assigned identity, the store incarnation, the Network dependency,
//!   and the leg itself are all read daemon-side and arrive together;
//! - the leg is verified against the claim before the port is reached, so a
//!   cross-Zone, stale, or foreign-device claim leaves no relay, no listener,
//!   and no firewall rule behind;
//! - the controller is retained per committed row across passes, so a second
//!   pass over an unchanged row re-uses the relay authority it already holds
//!   instead of starting a second one.

use std::sync::Arc;

use parking_lot::Mutex;

use d2b_contracts_resource::v3::{ResourceGeneration, ResourceRef, ZoneId};
use d2b_provider_toolkit::{SharedProviderEffectError, SharedProviderEffectPhase};

use crate::controller::{UsbipController, UsbipControllerError, UsbipServiceClaim, UsbipServicePhase};
use crate::facets::{UsbipClaimPortSource, UsbipHelperLeg, UsbipHelperLegRequest, UsbipHelperLegSource};

/// The per-row state one committed USB Service row's converted realization
/// retains.
///
/// One value per driver instance, and the plane keys one driver per resource
/// key, so the retained relay authority and projection token belong to exactly
/// the Service row that acquired them (R6, R11).
#[derive(Default)]
pub struct UsbipServiceRealization {
    controller: Mutex<Option<UsbipController>>,
}

impl UsbipServiceRealization {
    /// Reconcile one committed USB Service row through its converted claim
    /// path.
    ///
    /// The admission facet is consulted first and asynchronously, before the
    /// retained state is touched: an unavailable authority is a `Pending` pass
    /// with no effect attempted, not a controller left half-driven.
    ///
    /// # Errors
    ///
    /// Returns [`SharedProviderEffectError::Unavailable`] when the graph
    /// authority holds no evidence for this relationship, and
    /// [`SharedProviderEffectError::InvalidResource`] when the committed row
    /// does not decode as one this family's request admits. A claim the family
    /// refuses is not an error: it is a `Pending` pass with the controller's
    /// closed phase recorded, because a refusal is a settled answer, not a
    /// fault.
    pub async fn reconcile(
        &self,
        admission: &dyn UsbipHelperLegSource,
        claims: &dyn UsbipClaimPortSource,
        zone: &ZoneId,
        device_ref: &ResourceRef,
        helper: &ResourceRef,
        service_generation: ResourceGeneration,
    ) -> Result<SharedProviderEffectPhase, SharedProviderEffectError> {
        let request = UsbipHelperLegRequest::new(zone, device_ref, helper);
        let leg = admission.helper_leg(&request).await?;
        let mut port = claims.claim_port();
        Ok(self.drive(&leg, &request, service_generation, port.as_mut()))
    }

    /// Drain one committed USB Service row's converted realization.
    ///
    /// The order is the contract: the projection is removed and the relay leg
    /// stopped while the reservation is still held, and only then is the
    /// relationship handed back to the source.
    ///
    /// # Errors
    ///
    /// Returns the effect's own error when a teardown step does not confirm.
    /// Retained state is left intact on failure so a retry cannot release a
    /// reservation whose effects are still live.
    pub fn finalize(&self, port: &mut dyn crate::firewall::UsbipClaimPort) -> Result<(), UsbipControllerError> {
        let Some(mut controller) = self.controller.lock().take() else {
            return Ok(());
        };
        controller.finalize_claim(port)
    }

    /// The closed phase the retained controller is standing in.
    pub fn phase(&self) -> Option<UsbipServicePhase> {
        self.controller.lock().as_ref().map(UsbipController::phase)
    }

    /// Verify the minted leg against the claim and drive the converted pass.
    ///
    /// Everything below runs under a synchronous lock and no guard is held
    /// across an await: the retained controller is moved out, driven, and moved
    /// back.
    fn drive(
        &self,
        leg: &UsbipHelperLeg,
        request: &UsbipHelperLegRequest<'_>,
        service_generation: ResourceGeneration,
        port: &mut dyn crate::firewall::UsbipClaimPort,
    ) -> SharedProviderEffectPhase {
        let mut guard = self.controller.lock();
        let mut controller = guard.take().unwrap_or_else(|| {
            UsbipController::new(
                leg.service().clone(),
                service_generation,
                leg.device_uid().clone(),
            )
        });
        let admitted = UsbipServiceClaim::new(
            request.zone(),
            leg.store(),
            request.device_ref(),
            leg.claim(),
            request.helper(),
            leg.helper_uid(),
        );
        let outcome = controller.reconcile_claim(
            &admitted,
            leg.leg(),
            leg.network().clone(),
            port,
        );
        let phase = controller.phase();
        *guard = Some(controller);
        if let Err(error) = outcome {
            tracing::warn!(
                device = %request.device_ref().to_canonical_string(),
                helper = %request.helper().to_canonical_string(),
                error = %error,
                "usbip converted claim pass refused",
            );
        }
        match phase {
            UsbipServicePhase::Ready => SharedProviderEffectPhase::Ready,
            UsbipServicePhase::WaitingForNetwork
            | UsbipServicePhase::Applying
            | UsbipServicePhase::Drifted
            | UsbipServicePhase::Releasing => SharedProviderEffectPhase::Pending,
            // A blocked realization is a refusal the row reports, not a fault
            // the plane retries: the claim was refused by name and no effect
            // was attempted behind it.
            UsbipServicePhase::Blocked => SharedProviderEffectPhase::Pending,
        }
    }
}

/// The converted USB Service realization built from one zone's facet set.
///
/// The daemon constructs this once per zone and the family's driver holds it
/// as its per-row state.
#[derive(Clone)]
pub struct UsbipServiceRealizations {
    admission: Arc<dyn UsbipHelperLegSource>,
    claims: Arc<dyn UsbipClaimPortSource>,
}

impl UsbipServiceRealizations {
    /// Bind the realization to the daemon-supplied facets of one zone.
    pub const fn new(
        admission: Arc<dyn UsbipHelperLegSource>,
        claims: Arc<dyn UsbipClaimPortSource>,
    ) -> Self {
        Self { admission, claims }
    }

    /// One retained realization for one committed USB Service row.
    pub fn row(&self) -> UsbipServiceRealization {
        UsbipServiceRealization::default()
    }

    /// Reconcile one committed USB Service row through its converted claim
    /// path.
    ///
    /// # Errors
    ///
    /// Returns [`SharedProviderEffectError::Unavailable`] when the graph
    /// authority holds no evidence for this relationship, and
    /// [`SharedProviderEffectError::InvalidResource`] when the committed row
    /// does not decode as one this family's request admits.
    pub async fn reconcile(
        &self,
        state: &UsbipServiceRealization,
        zone: &ZoneId,
        device_ref: &ResourceRef,
        helper: &ResourceRef,
        service_generation: ResourceGeneration,
    ) -> Result<SharedProviderEffectPhase, SharedProviderEffectError> {
        state
            .reconcile(
                self.admission.as_ref(),
                self.claims.as_ref(),
                zone,
                device_ref,
                helper,
                service_generation,
            )
            .await
    }

    /// Reconcile one committed USB Service row from its own effect request.
    ///
    /// The row is the only input: its spec names the backing `Device`, and
    /// the relay worker is this family's own declared helper. Everything the
    /// claim is fenced against comes back from the admission facet, so a row
    /// cannot name a device or a helper the source did not admit.
    ///
    /// # Errors
    ///
    /// Returns [`SharedProviderEffectError::InvalidResource`] when the
    /// committed row does not name a backing `Device`, and the admission
    /// facet's own refusal when the graph authority holds no evidence for the
    /// relationship.
    pub async fn reconcile_service(
        &self,
        state: &UsbipServiceRealization,
        request: &d2b_provider_toolkit::SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectPhase, SharedProviderEffectError> {
        let device_ref = ResourceRef::parse(
            request
                .spec
                .get("backingDeviceRef")
                .and_then(serde_json::Value::as_str)
                .ok_or(SharedProviderEffectError::InvalidResource)?,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let helper = ResourceRef::parse(crate::USBIP_RELAY_CONTROLLER_REF)
            .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        self.reconcile(state, &request.zone, &device_ref, &helper, request.generation)
            .await
    }

    /// Drain one committed USB Service row's converted realization.
    ///
    /// # Errors
    ///
    /// Returns the effect's own error when a teardown step does not confirm.
    pub fn finalize(
        &self,
        state: &UsbipServiceRealization,
    ) -> Result<(), UsbipControllerError> {
        let mut port = self.claims.claim_port();
        state.finalize(port.as_mut())
    }
}