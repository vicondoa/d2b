//! The declared facets the provider-owned USBIP effects service reaches
//! daemon state through (U12 usbip step).
//!
//! The USBIP family's driver effects are served by this crate's own
//! implementation (see [`crate::effects_service`]). The daemon state that
//! implementation holds - the reconcile and finalize orchestration over the
//! daemon's admission, child rows, and readiness state, and the privileged
//! typed USBIP bind/unbind dispatch - crosses the provider boundary as
//! declared facets rather than as a daemon handle: every facet here is a
//! type the provider crate declares, an implementation of it is supplied by
//! the daemon host through the composition root (never derived from caller
//! input), and the family crate holds no daemon state type.
//!
//! Two facet surfaces share one daemon-supplied runtime value:
//!
//! - [`UsbipRuntime`] is the orchestration facade the driver effects
//!   delegate to: the daemon implements the reconcile and finalize
//!   machinery (admission, dependency barriers, child rows, the typed
//!   lifecycle controllers) over its own plane state.
//! - [`UsbipBrokerDispatch`] is the privileged broker-dispatch facet the
//!   crate's kernel dispatcher ([`crate::broker::KernelUsbipDispatcher`])
//!   sends its typed `UsbipBind`/`UsbipUnbind` requests through: the daemon
//!   supplies the authenticated origination dispatch over its broker
//!   socket, so the privileged wire path stays daemon-hosted while the
//!   dispatcher logic (the authority ledger, the lease fences, the
//!   bind/unbind sequencing) lives in the declaring crate.

use std::sync::Arc;

use async_trait::async_trait;
use d2b_contracts_broker::broker_wire::BrokerRequest;
use d2b_provider_toolkit::{
    SharedProviderEffectError, SharedProviderEffectOutcome, SharedProviderEffectRequest,
    SharedProviderFinalize,
};

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, StoreIncarnation, ZoneId};

use crate::arbitration::{AdmittedDeviceClaim, BoundDeviceLeg};
use crate::controller::{NetworkDependency, ScopedResourceUid};
use crate::driver::UsbipComponent;
use crate::firewall::UsbipClaimPort;
use crate::lifecycle::ServiceLifecycleError;

/// The daemon-supplied facet set the provider-owned USBIP effects are built
/// from (U12 usbip step).
///
/// The composition root supplies the objects; the driver never holds a
/// daemon state type (R2).
#[derive(Clone)]
pub struct UsbipEffectFacets {
    /// The daemon's USBIP runtime: the orchestration the driver effects
    /// delegate to, over the daemon's own admission, child rows, and
    /// readiness state.
    pub runtime: Arc<dyn UsbipRuntime>,
    /// The daemon-supplied broker facets the crate's kernel dispatcher is
    /// built from: the authenticated dispatch leg and the caller authority
    /// one typed bind/unbind request presents.
    pub broker: UsbipBrokerFacets,
    /// The daemon's `Device` source: the one place a committed USB Service
    /// row's admitted relationship and the bounded leg its relay realizes
    /// are minted (U20).
    pub admission: Arc<dyn UsbipHelperLegSource>,
    /// The daemon-hosted privileged effects one admitted USB relationship
    /// drives (U20).
    pub claims: Arc<dyn UsbipClaimPortSource>,
}

/// The request this family makes for one committed USB Service row's
/// converted `Device` realization.
///
/// Every field is an identity the committed row already declares: the Zone
/// the Service row lives in, the backing `Device` it names, and the relay
/// `Process` row the family's own vocabulary says realizes it
/// ([`crate::USBIP_RELAY_OPERATIONS`]). The family never states the helper's
/// store-assigned identity, the store incarnation the admission was
/// evaluated under, the Network the relay projects onto, or any host state -
/// all four come back in the answer, and every one of them is the daemon's to
/// read.
#[derive(Clone, Copy)]
pub struct UsbipHelperLegRequest<'a> {
    zone: &'a ZoneId,
    device_ref: &'a ResourceRef,
    helper: &'a ResourceRef,
}

impl<'a> UsbipHelperLegRequest<'a> {
    /// Bind the request to the facts one committed USB Service row declares.
    pub const fn new(
        zone: &'a ZoneId,
        device_ref: &'a ResourceRef,
        helper: &'a ResourceRef,
    ) -> Self {
        Self { zone, device_ref, helper }
    }

    /// The Zone the Service row lives in.
    pub const fn zone(&self) -> &'a ZoneId {
        self.zone
    }

    /// The backing `Device` the Service row declares.
    pub const fn device_ref(&self) -> &'a ResourceRef {
        self.device_ref
    }

    /// The relay `Process` row whose bounded leg realizes the claim.
    pub const fn helper(&self) -> &'a ResourceRef {
        self.helper
    }
}

/// One admitted `Device` relationship and the bounded leg its relay realizes.
///
/// The two halves are minted together and travel together on purpose: a claim
/// without its leg grants nothing, and a leg without its claim is verified
/// against nothing. [`Self::mint`] is the seam the `Device` source mints
/// through - the only way either half enters this crate - and it takes the
/// claim and the leg as one pair so the daemon cannot pair a claim from one
/// relationship with a leg bound to another.
pub struct UsbipHelperLeg {
    service: ScopedResourceUid,
    device_uid: ResourceUid,
    claim: AdmittedDeviceClaim,
    helper_uid: ResourceUid,
    store: StoreIncarnation,
    network: NetworkDependency,
    leg: Box<dyn BoundDeviceLeg + Send + Sync>,
}

impl UsbipHelperLeg {
    /// Pair one admitted claim with the bounded leg and relay facts the
    /// `Device` source read for it.
    ///
    /// This is the daemon-side seam, not a family constructor: the claim can
    /// only be admitted through the shared binding contract against an
    /// authorization and a freshness fence the family never holds.
    #[allow(clippy::too_many_arguments, reason = "one constructor, one boundary")]
    pub const fn mint(
        service: ScopedResourceUid,
        device_uid: ResourceUid,
        claim: AdmittedDeviceClaim,
        helper_uid: ResourceUid,
        store: StoreIncarnation,
        network: NetworkDependency,
        leg: Box<dyn BoundDeviceLeg + Send + Sync>,
    ) -> Self {
        Self { service, device_uid, claim, helper_uid, store, network, leg }
    }

    /// The Zone-scoped identity of the Service row this relationship realizes.
    ///
    /// The Zone uid is the daemon's read of the Zone row, never the row's own
    /// `metadata.zone` string: the controller fences every effect against this
    /// identity, so it is authority rather than a declaration.
    pub const fn service(&self) -> &ScopedResourceUid {
        &self.service
    }

    /// The store-assigned identity of the backing `Device` row.
    pub const fn device_uid(&self) -> &ResourceUid {
        &self.device_uid
    }

    /// The admitted `Device` relationship the relay realizes.
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

    /// The relay's Network dependency, as the daemon resolved it.
    pub const fn network(&self) -> &NetworkDependency {
        &self.network
    }

    /// The bounded leg the `Device` source bound to the relay.
    pub fn leg(&self) -> &(dyn BoundDeviceLeg + Send + Sync) {
        self.leg.as_ref()
    }
}

/// The daemon's `Device` source: the one mint for a committed USB Service
/// row's admitted relationship and its bounded helper leg (U20).
///
/// The implementation is supplied by the composition root over the graph
/// authority's own verdict and the trusted host inventory - never from the
/// row being served, and never from a caller. A plane with no authority
/// behind it holds [`UnwiredUsbipHelperLegs`], which refuses every request by
/// name and grants nothing.
#[async_trait]
pub trait UsbipHelperLegSource: Send + Sync + 'static {
    /// Admit the relationship the committed USB Service row declares and bind
    /// the bounded leg its relay realizes.
    ///
    /// # Errors
    ///
    /// Returns [`SharedProviderEffectError::Unavailable`] when the graph
    /// authority holds no evidence for this relationship, and
    /// [`SharedProviderEffectError::InvalidResource`] when the committed row
    /// does not decode as one this family's request admits.
    async fn helper_leg(
        &self,
        request: &UsbipHelperLegRequest<'_>,
    ) -> Result<UsbipHelperLeg, SharedProviderEffectError>;
}

/// The daemon-hosted privileged effects one admitted USB relationship drives
/// (U20).
///
/// The implementation is supplied by the composition root and resolves each
/// intent privately - the ownership marker, the host module, the listener,
/// and the projection intent - from the graph rather than from a resource
/// identity the Provider chose. A plane with no privileged path behind it
/// holds [`UnwiredUsbipClaimPorts`], which refuses every verb by name.
pub trait UsbipClaimPortSource: Send + Sync + 'static {
    /// One bounded effect port for this plane.
    fn claim_port(&self) -> Box<dyn UsbipClaimPort>;
}

/// The no-authority USB helper leg source: refuses every request by name.
///
/// This is what a plane with no graph authority behind it holds. It grants
/// nothing and admits nothing: the refusal names the missing authority rather
/// than letting a row stand in for it.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnwiredUsbipHelperLegs;

#[async_trait]
impl UsbipHelperLegSource for UnwiredUsbipHelperLegs {
    async fn helper_leg(
        &self,
        request: &UsbipHelperLegRequest<'_>,
    ) -> Result<UsbipHelperLeg, SharedProviderEffectError> {
        Err(SharedProviderEffectError::Unavailable).inspect_err(|_| {
            tracing::warn!(
                device = %request.device_ref().to_canonical_string(),
                reason = "the USB helper leg is daemon-minted and this plane has no graph authority to admit the relationship",
                "usbip helper leg refused: unwired authority",
            );
        })
    }
}

/// The no-privileged-path USB claim port source: refuses every verb by name.
///
/// Where a plane has no broker-backed privileged path, every claim effect
/// fails closed instead of reporting a relay, a listener, or a firewall rule
/// that does not exist.
#[derive(Clone, Copy, Debug, Default)]
pub struct UnwiredUsbipClaimPorts;

impl UsbipClaimPortSource for UnwiredUsbipClaimPorts {
    fn claim_port(&self) -> Box<dyn UsbipClaimPort> {
        Box::new(UnwiredUsbipClaimPort)
    }
}

/// The no-privileged-path USB claim port: every verb is a named refusal.
struct UnwiredUsbipClaimPort;

impl crate::firewall::UsbipClaimPort for UnwiredUsbipClaimPort {
    fn start_relay_leg(
        &mut self,
        _claim: &AdmittedDeviceClaim,
        _helper: &ResourceRef,
        _network_uid: &ResourceUid,
        _fence: &crate::firewall::ClaimProjectionFence,
    ) -> Result<crate::firewall::RelayAuthorityLease, crate::firewall::UsbipEffectError> {
        Err(unwired_claim_effect("start-relay-leg"))
    }

    fn mutate_claim_firewall(
        &mut self,
        _claim: &AdmittedDeviceClaim,
        _network_uid: &ResourceUid,
        _action: crate::firewall::FirewallProjectionAction,
        _fence: &crate::firewall::ClaimProjectionFence,
        _retained_token: Option<&crate::firewall::FirewallToken>,
    ) -> Result<crate::firewall::FirewallConfirmation, crate::firewall::UsbipEffectError> {
        Err(unwired_claim_effect("mutate-claim-firewall"))
    }

    fn observe_claim_firewall(
        &mut self,
        _claim: &AdmittedDeviceClaim,
        _network_uid: &ResourceUid,
        _fence: &crate::firewall::ClaimProjectionFence,
        _token: &crate::firewall::FirewallToken,
    ) -> Result<crate::firewall::FirewallObservation, crate::firewall::UsbipEffectError> {
        Err(unwired_claim_effect("observe-claim-firewall"))
    }

    fn stop_relay_leg(
        &mut self,
        _claim: &AdmittedDeviceClaim,
        _helper: &ResourceRef,
    ) -> Result<(), crate::firewall::UsbipEffectError> {
        Err(unwired_claim_effect("stop-relay-leg"))
    }

    fn release_claim(
        &mut self,
        _claim: &AdmittedDeviceClaim,
    ) -> Result<(), crate::firewall::UsbipEffectError> {
        Err(unwired_claim_effect("release-claim"))
    }
}

/// The refusal a plane with no privileged USB path reports for one verb.
fn unwired_claim_effect(
    verb: &'static str,
) -> crate::firewall::UsbipEffectError {
    tracing::warn!(
        verb,
        reason = "the USB claim effects are daemon-hosted and this plane has no privileged path to them",
        "usbip claim effect refused: unwired privileged path",
    );
    crate::firewall::UsbipEffectError::EffectRejected
}

/// The daemon-supplied broker facets one USBIP kernel dispatcher is built
/// from (U12 usbip step).
#[derive(Clone)]
pub struct UsbipBrokerFacets {
    /// The authenticated daemon-to-broker dispatch leg one typed USBIP
    /// bind/unbind request goes over. The daemon implementation presents
    /// the same `AdminUid` authority the retired daemon dispatcher
    /// presented.
    pub dispatch: Arc<dyn UsbipBrokerDispatch>,
}

/// The daemon-hosted USBIP runtime one zone's effects run over (U12 usbip
/// step).
///
/// The daemon implements this trait in its composition root (the same
/// shared-provider effects adapter that serves the other shared families),
/// supplying the reconcile/finalize orchestration over the daemon's own
/// admission, child rows, and readiness state. The family crate's effects
/// service delegates the driver seam to it; the privileged bind/unbind
/// invocations themselves run through this crate's
/// [`crate::broker::KernelUsbipDispatcher`], which the runtime constructs
/// from the broker facets ([`UsbipBrokerFacets`]).
#[async_trait]
pub trait UsbipRuntime: Send + Sync + 'static {
    /// Reconcile one USB Service or Binding row through the family's typed
    /// lifecycle controller.
    async fn reconcile_usbip(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderEffectOutcome, SharedProviderEffectError>;

    /// Advance one USB row's Provider teardown stage (old
    /// `execute_finalize`).
    async fn finalize(
        &self,
        component: UsbipComponent,
        request: &SharedProviderEffectRequest<'_>,
    ) -> Result<SharedProviderFinalize, SharedProviderEffectError>;
}

/// The privileged typed USBIP broker dispatch one kernel dispatcher sends
/// its bind/unbind requests through (U12 usbip step).
///
/// The daemon implements this facet over its authenticated broker socket;
/// the crate's dispatcher holds no socket, path, or caller authority.
pub trait UsbipBrokerDispatch: Send + Sync + 'static {
    /// Dispatch one typed USBIP broker request and confirm its acceptance.
    fn ack(&self, request: BrokerRequest) -> Result<(), ServiceLifecycleError>;
}