//! The declared facets the provider-owned security-key effects service
//! reaches daemon state through (U12 security-key step).
//!
//! The security-key family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The daemon state that
//! implementation holds - the reconcile and finalize orchestration over the
//! daemon's admission, child rows, and readiness state - crosses the
//! provider boundary as a declared facet rather than as a daemon handle:
//! every facet here is a type the provider crate declares, an
//! implementation of it is supplied by the daemon host through the
//! composition root (never derived from caller input), and the family crate
//! holds no daemon state type.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectRequest,
    SharedProviderFinalize,
};

use crate::driver::SecurityKeyComponent;

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, StoreIncarnation, ZoneId};

use crate::lease::{AdmittedDeviceClaim, BoundDeviceLeg, SecurityKeySessionId};

/// The daemon-supplied facet set the provider-owned security-key effects are
/// built from (U12 security-key step).
///
/// The composition root supplies the object; the driver never holds a
/// daemon state type (R2).
#[derive(Clone)]
pub struct SecurityKeyEffectFacets {
    /// The daemon's security-key runtime: the orchestration the driver
    /// effects delegate to, over the daemon's own admission, child rows,
    /// and readiness state.
    pub runtime: Arc<dyn SecurityKeyRuntime>,
    /// The daemon's `Device` source: the one place a committed security-key
    /// row's admitted relationship and the bounded leg its host relay
    /// realizes are minted (U20).
    pub admission: Arc<dyn SecurityKeyHelperLegSource>,
    /// The daemon-hosted privileged effects one admitted security-key
    /// relationship drives (U20).
    pub claims: Arc<dyn SecurityKeyClaimPortSource>,
}

/// The request this family makes for one committed security-key Binding row's
/// converted `Device` realization.
///
/// Every field is an identity the committed row already declares: the Zone
/// the Binding row lives in, the security-key Service it attaches to, and the
/// `Guest` whose Binding rides the relationship. The backing `Device` is
/// resolved daemon-side from that Service, so a Binding cannot reach a device
/// the source did not admit; likewise the relay's store-assigned identity, the
/// store incarnation, the session handle, and the leg itself all come back in
/// the answer, and every one of them is the daemon's to read.
///
/// The relay helper is not asked for either: its `Process` row is named by
/// this family's own [`security_key_relay_ref`] over the admitted
/// relationship's device identity, so neither a row nor a caller can choose it.
#[derive(Clone, Copy)]
pub struct SecurityKeyHelperLegRequest<'a> {
    zone: &'a ZoneId,
    service_ref: &'a ResourceRef,
    holder: &'a ResourceRef,
}

impl<'a> SecurityKeyHelperLegRequest<'a> {
    /// Bind the request to the facts one committed security-key Binding row
    /// declares.
    pub const fn new(
        zone: &'a ZoneId,
        service_ref: &'a ResourceRef,
        holder: &'a ResourceRef,
    ) -> Self {
        Self { zone, service_ref, holder }
    }

    /// The Zone the Binding row lives in.
    pub const fn zone(&self) -> &'a ZoneId {
        self.zone
    }

    /// The security-key Service the Binding attaches to.
    pub const fn service_ref(&self) -> &'a ResourceRef {
        self.service_ref
    }

    /// The `Guest` whose Binding rides the relationship.
    pub const fn holder(&self) -> &'a ResourceRef {
        self.holder
    }
}

/// The host relay `Process` row this family's own vocabulary names for one
/// admitted relationship.
///
/// This is the helper [`crate::SECURITY_KEY_RELAY_OPERATIONS`] names, and it
/// is derived from the admitted relationship's device identity rather than
/// stated: a row that named a different relay would be naming a `Process`
/// that does not exist for that device.
///
/// # Errors
///
/// Returns [`crate::process::ProcessDeclarationError`] when the admitted
/// device identity does not derive a bounded `Process` row name.
pub fn security_key_relay_ref(
    device_uid: &ResourceUid,
) -> Result<ResourceRef, crate::process::ProcessDeclarationError> {
    let name =
        crate::security_key_process_name(device_uid, crate::SecurityKeyProcessRole::HostRelay)?;
    ResourceRef::parse(&format!("Process/{name}"))
        .map_err(|_| crate::process::ProcessDeclarationError::InvalidUid)
}

/// One admitted `Device` relationship and the bounded leg its host relay
/// realizes.
///
/// The two halves are minted together and travel together on purpose: a claim
/// without its leg grants nothing, and a leg without its claim is verified
/// against nothing. [`Self::mint`] is the seam the `Device` source mints
/// through - the only way either half enters this crate - and it takes the
/// claim and the leg as one pair so the daemon cannot pair a claim from one
/// relationship with a leg bound to another.
pub struct SecurityKeyHelperLeg {
    /// The backing `Device` the daemon resolved from the Service row this
    /// Binding names.
    ///
    /// The Binding row does not carry it, so it is read daemon-side from the
    /// committed Service row rather than taken from the row being served. The
    /// family verifies the admitted claim against this before it opens
    /// anything, so a claim admitted for a different `Device` is refused.
    device_ref: ResourceRef,
    claim: AdmittedDeviceClaim,
    helper_uid: ResourceUid,
    store: StoreIncarnation,
    session: SecurityKeySessionId,
    leg: Box<dyn BoundDeviceLeg + Send + Sync>,
}

impl SecurityKeyHelperLeg {
    /// Pair one admitted claim with the bounded leg and host facts the
    /// `Device` source read for it.
    ///
    /// This is the daemon-side seam, not a family constructor: the claim can
    /// only be admitted through the shared binding contract against an
    /// authorization and a freshness fence the family never holds.
    pub const fn mint(
        device_ref: ResourceRef,
        claim: AdmittedDeviceClaim,
        helper_uid: ResourceUid,
        store: StoreIncarnation,
        session: SecurityKeySessionId,
        leg: Box<dyn BoundDeviceLeg + Send + Sync>,
    ) -> Self {
        Self { device_ref, claim, helper_uid, store, session, leg }
    }

    /// The backing `Device` the daemon resolved from the committed Service row.
    pub const fn device_ref(&self) -> &ResourceRef {
        &self.device_ref
    }

    /// The admitted `Device` relationship the host relay realizes.
    pub const fn claim(&self) -> &AdmittedDeviceClaim {
        &self.claim
    }

    /// The store-assigned identity of the relay `Process` row the `Device`
    /// source bound the leg to.
    pub const fn helper_uid(&self) -> &ResourceUid {
        &self.helper_uid
    }

    /// The store incarnation the admission was fenced against.
    pub const fn store(&self) -> &StoreIncarnation {
        &self.store
    }

    /// The bounded session handle this committed row's realization runs under.
    ///
    /// It derives from the committed row's own identity and generation, so a
    /// second pass over an unchanged row opens the same session and a changed
    /// row opens a different one. It is not authority: the lease refuses the
    /// acquire unless the claim and the leg both verify first.
    pub const fn session(&self) -> &SecurityKeySessionId {
        &self.session
    }

    /// The bounded leg the `Device` source bound to the host relay.
    pub fn leg(&self) -> &(dyn BoundDeviceLeg + Send + Sync) {
        self.leg.as_ref()
    }
}

/// The daemon's `Device` source: the one mint for a committed security-key
/// row's admitted relationship and its bounded helper leg (U20).
///
/// The implementation is supplied by the composition root over the graph
/// authority's own verdict and the trusted host inventory - never from the
/// row being served, and never from a caller. A plane with no authority
/// behind it holds [`UnwiredSecurityKeyHelperLegs`], which refuses every
/// request by name and grants nothing.
#[async_trait]
pub trait SecurityKeyHelperLegSource: Send + Sync + 'static {
    /// Admit the relationship the committed security-key row declares and bind
    /// the bounded leg its host relay realizes.
    ///
    /// # Errors
    ///
    /// Returns [`SharedProviderEffectError::Unavailable`] when the graph
    /// authority holds no evidence for this relationship, and
    /// [`SharedProviderEffectError::InvalidResource`] when the committed row
    /// does not decode as one this family's request admits.
    async fn helper_leg(
        &self,
 request: &SecurityKeyHelperLegRequest<'_>,
    ) -> Result<SecurityKeyHelperLeg, SharedProviderEffectError>;
}

/// The daemon-hosted privileged effects one admitted security-key
/// relationship drives (U20).
///
/// The implementation is supplied by the composition root and resolves each
/// intent privately - the hidraw node, the udev rules, and the relay launch -
/// from the graph rather than from a resource identity the Provider chose. A
/// plane with no privileged path behind it holds
/// [`UnwiredSecurityKeyClaimPorts`], which refuses every verb by name.
pub trait SecurityKeyClaimPortSource: Send + Sync + 'static {
    /// One bounded effect port for this plane.
    fn claim_port(&self) -> Box<dyn crate::lease::SecurityKeyClaimPort>;
}

/// The no-authority security-key helper leg source: refuses every request by
/// name.
///
/// This is what a plane with no graph authority behind it holds. It grants
/// nothing and admits nothing: the refusal names the missing authority rather
/// than letting a row stand in for it.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnwiredSecurityKeyHelperLegs;

#[async_trait]
impl SecurityKeyHelperLegSource for UnwiredSecurityKeyHelperLegs {
    async fn helper_leg(
        &self,
        request: &SecurityKeyHelperLegRequest<'_>,
    ) -> Result<SecurityKeyHelperLeg, SharedProviderEffectError> {
        Err(SharedProviderEffectError::Unavailable).inspect_err(|_| {
            tracing::warn!(
                service = %request.service_ref().to_canonical_string(),
                reason = "the security-key helper leg is daemon-minted and this plane has no graph authority to admit the relationship",
                "security-key helper leg refused: unwired authority",
            );
        })
    }
}

/// The no-privileged-path security-key claim port source: refuses every verb
/// by name.
///
/// Where a plane has no broker-backed privileged path, every claim effect
/// fails closed instead of opening a hidraw node that does not exist.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnwiredSecurityKeyClaimPorts;

impl SecurityKeyClaimPortSource for UnwiredSecurityKeyClaimPorts {
    fn claim_port(&self) -> Box<dyn crate::lease::SecurityKeyClaimPort> {
        Box::new(UnwiredSecurityKeyClaimPort)
    }
}

/// The no-privileged-path security-key claim port: every verb is a named
/// refusal.
struct UnwiredSecurityKeyClaimPort;

impl crate::lease::SecurityKeyClaimPort for UnwiredSecurityKeyClaimPort {
    fn open_hidraw_leg(
        &mut self,
        _claim: &AdmittedDeviceClaim,
        _helper: &ResourceRef,
        _intent: &crate::authority::SecurityKeyOpenIntent,
    ) -> Result<crate::authority::RelayLaunchTicket, crate::authority::SecurityKeyEffectError> {
        tracing::warn!(
            verb = "open-hidraw-leg",
            reason = "the security-key claim effects are daemon-hosted and this plane has no privileged path to them",
            "security-key claim effect refused: unwired privileged path",
        );
        Err(crate::authority::SecurityKeyEffectError::BrokerInaccessible)
    }

    fn stop_relay_leg(
        &mut self,
        _claim: &AdmittedDeviceClaim,
        _helper: &ResourceRef,
        _ticket: crate::authority::RelayLaunchTicket,
    ) -> Result<(), crate::authority::SecurityKeyEffectError> {
        tracing::warn!(
            verb = "stop-relay-leg",
            reason = "the security-key claim effects are daemon-hosted and this plane has no privileged path to them",
            "security-key claim effect refused: unwired privileged path",
        );
        Err(crate::authority::SecurityKeyEffectError::BrokerInaccessible)
    }

    fn release_claim(
        &mut self,
        _claim: &AdmittedDeviceClaim,
    ) -> Result<(), crate::authority::SecurityKeyEffectError> {
        tracing::warn!(
            verb = "release-claim",
            reason = "the security-key claim effects are daemon-hosted and this plane has no privileged path to them",
            "security-key claim effect refused: unwired privileged path",
        );
        Err(crate::authority::SecurityKeyEffectError::BrokerInaccessible)
    }
}

/// The daemon-hosted security-key runtime one zone's effects run over (U12
/// security-key step).
///
/// The daemon implements this trait in its composition root (the same
/// shared-provider effects adapter that serves the other shared families),
/// supplying the reconcile/finalize orchestration over the daemon's own
/// admission, child rows, and readiness state. The family crate's effects
/// service delegates the driver seam to it.
#[async_trait]
pub trait SecurityKeyRuntime: Send + Sync + 'static {
    /// Reconcile one security-key Service or Binding row through the
    /// family's typed lifecycle controller.
    async fn reconcile_security_key(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Advance one security-key row's Provider teardown stage (old
    /// `execute_finalize`).
    async fn finalize(
        &self,
        component: SecurityKeyComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError>;
}