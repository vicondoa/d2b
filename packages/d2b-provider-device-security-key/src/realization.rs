//! The converted security-key realization (U20).
//!
//! This module is the family half of the `DeviceHelperLeg` production path. The
//! `Device` source mints the admitted relationship and the bounded leg (see
//! [`crate::facets::SecurityKeyHelperLegSource`]); this module asks for that
//! pair, hands it to the family's own [`SecurityKeyLease`], and reports the
//! closed phase the lease reached.
//!
//! Three properties are the point of the shape:
//!
//! - the family names only what its committed Binding row declares. The relay's
//!   store-assigned identity, the store incarnation, the session handle, and
//!   the leg itself are all read daemon-side and arrive together;
//! - the claim and the leg are verified before the hidraw open is attempted, so
//!   a cross-Zone, stale, or foreign-device claim leaves no relay, no open node,
//!   and no reserved reservation behind;
//! - the lease is retained per committed row across passes, so a second pass
//!   over an unchanged row re-uses the session it already holds instead of
//!   opening a second one.

use std::sync::Arc;

use parking_lot::Mutex;

use d2b_contracts_resource::v3::{ResourceRef, ZoneId};
use d2b_provider_toolkit::{SharedProviderEffectError, SharedProviderEffectPhase};

use crate::facets::{
    SecurityKeyClaimPortSource, SecurityKeyHelperLeg, SecurityKeyHelperLegRequest,
    SecurityKeyHelperLegSource, security_key_relay_ref,
};
use crate::lease::{LeaseState, SecurityKeyClaimPort, SecurityKeyClaimRequest, SecurityKeyLease};

/// The per-row state one committed security-key row's converted realization
/// retains.
///
/// One value per driver instance, and the plane keys one driver per resource
/// key, so the retained session and relay ticket belong to exactly the row
/// that opened them (R6, R11).
#[derive(Default)]
pub struct SecurityKeyBindingRealization {
    lease: Mutex<Option<SecurityKeyLease>>,
}

impl SecurityKeyBindingRealization {
    /// The closed phase the retained lease is standing in.
    pub fn phase(&self) -> Option<LeaseState> {
        self.lease.lock().as_ref().map(SecurityKeyLease::state)
    }

    /// Whether the retained lease still holds an open relay.
    pub fn is_active(&self) -> bool {
        self.lease
            .lock()
            .as_ref()
            .is_some_and(|lease| lease.state() == LeaseState::Active)
    }

    /// Close the retained session, if one is open.
    ///
    /// # Errors
    ///
    /// Returns the lease's own error when a teardown step does not confirm.
    /// Retained state is left intact on failure so a retry cannot release a
    /// reservation whose relay is still live.
    pub fn finalize(&self, port: &mut dyn SecurityKeyClaimPort) -> Result<(), crate::lease::SecurityKeyLeaseError> {
        let Some(mut lease) = self.lease.lock().take() else {
            return Ok(());
        };
        if lease.state() == LeaseState::Active {
            lease.complete_bound(port)?;
        }
        Ok(())
    }

    /// Verify the minted leg against the claim and drive the converted pass.
    ///
    /// Everything below runs under a synchronous lock and no guard is held
    /// across an await: the retained lease is moved out, driven, and moved
    /// back.
    fn drive(
        &self,
        leg: &SecurityKeyHelperLeg,
        request: &SecurityKeyHelperLegRequest<'_>,
        port: &mut dyn SecurityKeyClaimPort,
    ) -> SharedProviderEffectPhase {
        let mut guard = self.lease.lock();
        let mut lease = guard.take().unwrap_or_else(|| {
            SecurityKeyLease::new_admitted(leg.claim().key().source_uid().clone())
        });
        let outcome = self.admit_and_acquire(&mut lease, leg, request, port);
        let active = lease.state() == LeaseState::Active;
        *guard = Some(lease);
        if let Err(error) = outcome {
            tracing::warn!(
                service = %request.service_ref().to_canonical_string(),
                holder = %request.holder().to_canonical_string(),
                error = %error,
                "security-key converted claim pass refused",
            );
        }
        if active {
            SharedProviderEffectPhase::Ready
        } else {
            SharedProviderEffectPhase::Pending
        }
    }

    /// Admit the claim, then open the bounded session the leg authorizes.
    ///
    /// The relay helper is this family's own derivation over the admitted
    /// relationship's device identity, so neither the committed row nor a
    /// caller chooses which `Process` row realizes the claim. Both checks run
    /// before the port is reached: a cross-Zone, stale, or foreign-device
    /// claim opens nothing.
    fn admit_and_acquire(
        &self,
        lease: &mut SecurityKeyLease,
        leg: &SecurityKeyHelperLeg,
        request: &SecurityKeyHelperLegRequest<'_>,
        port: &mut dyn SecurityKeyClaimPort,
    ) -> Result<(), crate::lease::SecurityKeyLeaseError> {
        let helper = security_key_relay_ref(leg.claim().key().source_uid())
            .map_err(|_| crate::lease::SecurityKeyLeaseError::AuthorizationDenied)?;
        lease.admit_relay_claim(
            request.zone(),
            leg.device_ref(),
            leg.store(),
            &helper,
            leg.claim(),
        )?;
        if lease.state() == LeaseState::Active {
            return Ok(());
        }
        let admitted = SecurityKeyClaimRequest::new(
            request.zone(),
            leg.device_ref(),
            request.holder(),
            &helper,
            leg.helper_uid(),
        );
        lease.acquire_bound(*leg.session(), &admitted, leg.leg(), port)
    }
}

/// The converted security-key realization built from one zone's facet set.
///
/// The daemon constructs this once per zone and the family's driver holds it
/// as its per-row state.
#[derive(Clone)]
pub struct SecurityKeyBindingRealizations {
    admission: Arc<dyn SecurityKeyHelperLegSource>,
    claims: Arc<dyn SecurityKeyClaimPortSource>,
}

impl SecurityKeyBindingRealizations {
    /// Bind the realization to the daemon-supplied facets of one zone.
    pub const fn new(
        admission: Arc<dyn SecurityKeyHelperLegSource>,
        claims: Arc<dyn SecurityKeyClaimPortSource>,
    ) -> Self {
        Self { admission, claims }
    }

    /// One retained realization for one committed security-key row.
    pub fn row(&self) -> SecurityKeyBindingRealization {
        SecurityKeyBindingRealization::default()
    }

    /// Reconcile one committed security-key Binding row through its converted
    /// claim path.
    ///
    /// # Errors
    ///
    /// Returns [`SharedProviderEffectError::Unavailable`] when the graph
    /// authority holds no evidence for this relationship, and
    /// [`SharedProviderEffectError::InvalidResource`] when the committed row
    /// does not decode as one this family's request admits. A claim the family
    /// refuses is not an error: it is a `Pending` pass, because a refusal is a
    /// settled answer, not a fault.
    pub async fn reconcile(
        &self,
        state: &SecurityKeyBindingRealization,
        zone: &ZoneId,
        service_ref: &ResourceRef,
        holder: &ResourceRef,
    ) -> Result<SharedProviderEffectPhase, SharedProviderEffectError> {
        let request = SecurityKeyHelperLegRequest::new(zone, service_ref, holder);
        let leg = self.admission.helper_leg(&request).await?;
        let mut port = self.claims.claim_port();
        Ok(state.drive(&leg, &request, port.as_mut()))
    }

    /// Reconcile one committed security-key Binding row from its own effect
    /// request.
    ///
    /// The row is the only input: its spec names the Service it attaches to and
    /// the `Guest` it delivers to. The backing `Device` is resolved
    /// daemon-side from that Service, so the Binding cannot reach a device the
    /// source did not admit.
    ///
    /// # Errors
    ///
    /// Returns [`SharedProviderEffectError::InvalidResource`] when the
    /// committed row does not name its Service or target `Guest`, and the
    /// admission facet's own refusal when the graph
    /// authority holds no evidence for the relationship.
    pub async fn reconcile_binding(
        &self,
        state: &SecurityKeyBindingRealization,
        request: &d2b_provider_toolkit::SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectPhase, SharedProviderEffectError> {
        let service_ref = ResourceRef::parse(
            request
                .spec
                .get("serviceRef")
                .and_then(serde_json::Value::as_str)
                .ok_or(SharedProviderEffectError::InvalidResource)?,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        let holder = ResourceRef::parse(
            request
                .spec
                .get("target")
                .and_then(|target| target.get("guestRef"))
                .and_then(serde_json::Value::as_str)
                .ok_or(SharedProviderEffectError::InvalidResource)?,
        )
        .map_err(|_| SharedProviderEffectError::InvalidResource)?;
        self.reconcile(state, &request.zone, &service_ref, &holder).await
    }

    /// Close one committed security-key row's converted session.
    ///
    /// # Errors
    ///
    /// Returns the lease's own error when a teardown step does not confirm.
    pub fn finalize(
        &self,
        state: &SecurityKeyBindingRealization,
    ) -> Result<(), crate::lease::SecurityKeyLeaseError> {
        let mut port = self.claims.claim_port();
        state.finalize(port.as_mut())
    }
}