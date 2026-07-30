//! Zone bus transport credit accounting.
//!
//! The scarce resource this module accounts for is a transferable file
//! descriptor. The per-scope pools, their reservation and rollback discipline,
//! and the per-process derivation from `RLIMIT_NOFILE` are the audited
//! primitives re-exported below from the Unix transport substrate; nothing here
//! reimplements them.
//!
//! What this module adds is the Zone-level admission rule that decides whether
//! a transport is permitted to carry descriptors at all. A ZoneLink transport
//! crosses a Zone boundary, and `SCM_RIGHTS` grants are prohibited across a
//! Zone boundary, so a ZoneLink transport is always attachment-free and owns no
//! credit pools. A within-Zone transport may carry descriptors under a bounded
//! packet-atomic attachment policy.
//!
//! Every refusal is a typed [`CreditError`]; there is no permissive default and
//! no inferred route class.

use d2b_contracts::v3::component_session::{
    AttachmentPolicy, AttachmentPolicyKind, MAX_HOST_ATTACHMENT_CREDITS, MAX_OPERATION_ATTACHMENTS,
    MAX_PACKET_ATTACHMENTS, MAX_PROCESS_ATTACHMENT_CREDITS, MAX_REQUEST_ATTACHMENTS,
    MAX_SESSION_ATTACHMENTS, TransportClass,
};
use std::{error::Error, fmt};

pub use d2b_session_unix::{
    CreditBundle, CreditError as ScopeCreditError, CreditPool, CreditScope, CreditScopeSet,
    ProcessCreditLimit,
};

/// Whether a transport stays inside one Zone or carries a ZoneLink hop.
///
/// The route class is always supplied explicitly by the caller that owns the
/// link. It is never inferred from a locality, a socket kind, or a peer
/// property, because an inference that guessed `WithinZone` would silently
/// enable descriptor passing across a Zone boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteClass {
    /// Both endpoints are components of the same Zone.
    WithinZone,
    /// The transport carries a ZoneLink hop between two Zones.
    ZoneLink,
}

impl RouteClass {
    /// Whether descriptor attachments may ever be admitted on this class.
    pub const fn permits_attachments(self) -> bool {
        matches!(self, Self::WithinZone)
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WithinZone => "within-zone",
            Self::ZoneLink => "zone-link",
        }
    }
}

/// Typed refusal reasons for Zone-level attachment credit admission.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CreditError {
    /// A ZoneLink transport was asked to carry or budget a descriptor.
    ZoneLinkAttachmentsForbidden,
    /// A within-Zone plan was built from an attachment policy that is not the
    /// bounded packet-atomic kind.
    AttachmentPolicyNotPacketAtomic,
    /// The attachment policy is not a valid policy for the transport class.
    AttachmentPolicyInvalidForTransport,
    /// A per-scope attachment allowance is zero or above its frozen bound.
    AttachmentAllowanceOutOfBounds,
    /// The configured host credit ceiling is zero or above its frozen bound.
    HostAllowanceOutOfBounds,
    /// The derived per-process transferable capacity is zero or above its
    /// frozen bound.
    ProcessAllowanceOutOfBounds,
    /// More attachments were presented than the plan permits.
    AttachmentAllowanceExceeded,
    /// A scope pool could not be constructed from the planned allowance.
    ScopePool(ScopeCreditError),
}

impl fmt::Display for CreditError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZoneLinkAttachmentsForbidden => {
                formatter.write_str("transport-credit-zone-link-attachments-forbidden")
            }
            Self::AttachmentPolicyNotPacketAtomic => {
                formatter.write_str("transport-credit-attachment-policy-not-packet-atomic")
            }
            Self::AttachmentPolicyInvalidForTransport => {
                formatter.write_str("transport-credit-attachment-policy-invalid-for-transport")
            }
            Self::AttachmentAllowanceOutOfBounds => {
                formatter.write_str("transport-credit-attachment-allowance-out-of-bounds")
            }
            Self::HostAllowanceOutOfBounds => {
                formatter.write_str("transport-credit-host-allowance-out-of-bounds")
            }
            Self::ProcessAllowanceOutOfBounds => {
                formatter.write_str("transport-credit-process-allowance-out-of-bounds")
            }
            Self::AttachmentAllowanceExceeded => {
                formatter.write_str("transport-credit-attachment-allowance-exceeded")
            }
            Self::ScopePool(inner) => {
                write!(formatter, "transport-credit-scope-pool({inner:?})")
            }
        }
    }
}

impl Error for CreditError {}

/// The Zone-level attachment budget for one transport.
///
/// A plan is immutable once built. It carries no descriptor, no socket, no
/// path, and no peer identity - only the closed route class, the attachment
/// policy the transport must be constructed with, and the per-scope allowances
/// its credit pools are sized from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AttachmentCreditPlan {
    route_class: RouteClass,
    policy: AttachmentPolicy,
    process: usize,
    host: usize,
}

impl AttachmentCreditPlan {
    /// The attachment-free plan every ZoneLink transport uses.
    ///
    /// It owns no credit pools, because a pool would imply some nonzero number
    /// of descriptors could cross the Zone boundary.
    pub const fn zone_link() -> Self {
        Self {
            route_class: RouteClass::ZoneLink,
            policy: AttachmentPolicy::disabled(),
            process: 0,
            host: 0,
        }
    }

    /// A within-Zone plan for a packet-atomic transport class.
    ///
    /// `process` is the transferable capacity derived from the live process
    /// descriptor limit; `host_allowance` is the operator-configured host
    /// ceiling. Both are validated against the frozen contract bounds.
    pub fn within_zone(
        transport: TransportClass,
        policy: AttachmentPolicy,
        process: ProcessCreditLimit,
        host_allowance: usize,
    ) -> Result<Self, CreditError> {
        if policy.kind != AttachmentPolicyKind::PacketAtomic {
            return Err(CreditError::AttachmentPolicyNotPacketAtomic);
        }
        policy
            .validate(transport)
            .map_err(|_| CreditError::AttachmentPolicyInvalidForTransport)?;
        if policy.max_per_packet == 0
            || policy.max_per_packet > MAX_PACKET_ATTACHMENTS
            || policy.max_per_request > MAX_REQUEST_ATTACHMENTS
            || policy.max_per_operation > MAX_OPERATION_ATTACHMENTS
            || policy.max_per_session > MAX_SESSION_ATTACHMENTS
        {
            return Err(CreditError::AttachmentAllowanceOutOfBounds);
        }
        let process = process.transferable();
        if process == 0 || process > usize::from(MAX_PROCESS_ATTACHMENT_CREDITS) {
            return Err(CreditError::ProcessAllowanceOutOfBounds);
        }
        if host_allowance == 0 || host_allowance > usize::from(MAX_HOST_ATTACHMENT_CREDITS) {
            return Err(CreditError::HostAllowanceOutOfBounds);
        }
        Ok(Self {
            route_class: RouteClass::WithinZone,
            policy,
            process,
            host: host_allowance,
        })
    }

    pub const fn route_class(&self) -> RouteClass {
        self.route_class
    }

    /// The attachment policy a transport built under this plan must use.
    pub const fn attachment_policy(&self) -> AttachmentPolicy {
        self.policy
    }

    /// The largest number of descriptors one packet may carry.
    pub const fn max_attachments_per_packet(&self) -> usize {
        self.policy.max_per_packet as usize
    }

    /// Admits a descriptor count against the plan without reserving anything.
    ///
    /// This is the fail-closed gate the transport layer consults before it
    /// touches a credit pool. A ZoneLink plan admits exactly zero.
    pub fn admit(&self, attachments: usize) -> Result<(), CreditError> {
        if attachments == 0 {
            return Ok(());
        }
        if !self.route_class.permits_attachments() {
            return Err(CreditError::ZoneLinkAttachmentsForbidden);
        }
        if attachments > self.max_attachments_per_packet() {
            return Err(CreditError::AttachmentAllowanceExceeded);
        }
        Ok(())
    }

    /// Builds the six per-scope credit pools this plan sizes.
    ///
    /// A ZoneLink plan has no pools and refuses.
    pub fn credit_scopes(&self) -> Result<CreditScopeSet, CreditError> {
        if !self.route_class.permits_attachments() {
            return Err(CreditError::ZoneLinkAttachmentsForbidden);
        }
        let pool = |limit: usize| CreditPool::new(limit).map_err(CreditError::ScopePool);
        Ok(CreditScopeSet::new(
            pool(usize::from(self.policy.max_per_packet))?,
            pool(usize::from(self.policy.max_per_request))?,
            pool(usize::from(self.policy.max_per_operation))?,
            pool(usize::from(self.policy.max_per_session))?,
            pool(self.process)?,
            pool(self.host)?,
        ))
    }

    /// Admits and then reserves ingress credit for a received descriptor count.
    ///
    /// Admission runs first, so a plan that forbids attachments never touches a
    /// pool. The returned bundle owns the reservation: dropping it releases
    /// every scope it holds, and a partial failure inside the reservation
    /// releases every scope already taken.
    pub fn reserve_ingress(
        &self,
        scopes: &CreditScopeSet,
        attachments: usize,
    ) -> Result<CreditBundle, CreditError> {
        self.admit(attachments)?;
        scopes
            .reserve_ingress(attachments)
            .map_err(CreditError::ScopePool)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet_atomic(max_per_packet: u16) -> AttachmentPolicy {
        AttachmentPolicy {
            kind: AttachmentPolicyKind::PacketAtomic,
            max_per_packet,
            max_per_request: max_per_packet,
            max_per_operation: max_per_packet,
            max_per_session: max_per_packet,
            credentials_allowed: false,
        }
    }

    fn process_limit() -> ProcessCreditLimit {
        ProcessCreditLimit::derive(1024, 64).expect("derive a transferable process capacity")
    }

    fn within_zone_plan() -> AttachmentCreditPlan {
        AttachmentCreditPlan::within_zone(
            TransportClass::UnixSeqpacket,
            packet_atomic(4),
            process_limit(),
            256,
        )
        .expect("build a within-Zone credit plan")
    }

    #[test]
    fn zone_link_plans_are_attachment_free_and_own_no_pools() {
        let plan = AttachmentCreditPlan::zone_link();
        assert_eq!(plan.route_class(), RouteClass::ZoneLink);
        assert!(!plan.route_class().permits_attachments());
        assert_eq!(plan.attachment_policy(), AttachmentPolicy::disabled());
        assert_eq!(plan.max_attachments_per_packet(), 0);
        assert_eq!(plan.admit(0), Ok(()));
        assert_eq!(
            plan.admit(1),
            Err(CreditError::ZoneLinkAttachmentsForbidden)
        );
        assert_eq!(
            plan.credit_scopes().err(),
            Some(CreditError::ZoneLinkAttachmentsForbidden)
        );
    }

    #[test]
    fn within_zone_admission_is_bounded_by_the_per_packet_allowance() {
        let plan = within_zone_plan();
        assert_eq!(plan.route_class(), RouteClass::WithinZone);
        assert_eq!(plan.max_attachments_per_packet(), 4);
        for admitted in 0..=4 {
            assert_eq!(plan.admit(admitted), Ok(()));
        }
        assert_eq!(plan.admit(5), Err(CreditError::AttachmentAllowanceExceeded));
        assert_eq!(
            plan.admit(usize::MAX),
            Err(CreditError::AttachmentAllowanceExceeded)
        );
    }

    #[test]
    fn a_disabled_policy_never_yields_a_within_zone_plan() {
        assert_eq!(
            AttachmentCreditPlan::within_zone(
                TransportClass::UnixSeqpacket,
                AttachmentPolicy::disabled(),
                process_limit(),
                256,
            ),
            Err(CreditError::AttachmentPolicyNotPacketAtomic)
        );
    }

    #[test]
    fn a_packet_atomic_policy_is_refused_for_a_non_packet_atomic_transport() {
        assert_eq!(
            AttachmentCreditPlan::within_zone(
                TransportClass::UnixStream,
                packet_atomic(4),
                process_limit(),
                256,
            ),
            Err(CreditError::AttachmentPolicyInvalidForTransport)
        );
    }

    #[test]
    fn out_of_bounds_allowances_are_refused_with_their_own_reason() {
        assert_eq!(
            AttachmentCreditPlan::within_zone(
                TransportClass::UnixSeqpacket,
                packet_atomic(MAX_PACKET_ATTACHMENTS + 1),
                process_limit(),
                256,
            ),
            Err(CreditError::AttachmentPolicyInvalidForTransport)
        );
        assert_eq!(
            AttachmentCreditPlan::within_zone(
                TransportClass::UnixSeqpacket,
                packet_atomic(4),
                process_limit(),
                0,
            ),
            Err(CreditError::HostAllowanceOutOfBounds)
        );
        assert_eq!(
            AttachmentCreditPlan::within_zone(
                TransportClass::UnixSeqpacket,
                packet_atomic(4),
                process_limit(),
                usize::from(MAX_HOST_ATTACHMENT_CREDITS) + 1,
            ),
            Err(CreditError::HostAllowanceOutOfBounds)
        );
    }

    #[test]
    fn a_process_with_no_transferable_headroom_is_refused() {
        assert!(ProcessCreditLimit::derive(64, 0).is_err());
        let exhausted = ProcessCreditLimit::derive(66, 1).expect("one transferable descriptor");
        assert_eq!(exhausted.transferable(), 1);
        assert!(
            AttachmentCreditPlan::within_zone(
                TransportClass::UnixSeqpacket,
                packet_atomic(4),
                exhausted,
                256,
            )
            .is_ok()
        );
    }

    #[test]
    fn planned_scopes_reserve_and_release_at_every_scope() {
        let plan = within_zone_plan();
        let scopes = plan.credit_scopes().expect("build the planned scope set");
        let first = scopes
            .reserve(4)
            .expect("reserve the full packet allowance");
        assert_eq!(
            scopes.reserve(1).err(),
            Some(ScopeCreditError::Exhausted),
            "the packet scope is saturated while the first reservation is live"
        );
        drop(first);
        let second = scopes
            .reserve(4)
            .expect("the released allowance is reusable");
        drop(second);
    }

    #[test]
    fn a_failed_scope_reservation_leaves_no_scope_reserved() {
        let plan = within_zone_plan();
        let scopes = plan.credit_scopes().expect("build the planned scope set");
        let process = plan.process;
        assert!(
            process > 4,
            "the process scope must outsize the packet scope"
        );
        assert_eq!(
            scopes.reserve(5).err(),
            Some(ScopeCreditError::Exhausted),
            "a reservation above the packet allowance fails"
        );
        let recovered = scopes
            .reserve(4)
            .expect("the failed reservation rolled every prior scope back");
        drop(recovered);
    }

    #[test]
    fn ingress_reservation_covers_only_the_ingress_scopes() {
        let plan = within_zone_plan();
        let scopes = plan.credit_scopes().expect("build the planned scope set");
        let mut ingress = scopes.reserve_ingress(4).expect("reserve ingress scopes");
        ingress
            .acquire_dispatch(&scopes, 4)
            .expect("request and operation scopes are still free");
        assert_eq!(
            ingress.acquire_dispatch(&scopes, 1).err(),
            Some(ScopeCreditError::Exhausted),
            "dispatch scopes are acquired exactly once"
        );
        ingress.release(CreditScope::Packet);
        ingress.release(CreditScope::Packet);
        assert_eq!(
            scopes.reserve_ingress(4).err(),
            Some(ScopeCreditError::Exhausted),
            "releasing one scope does not release the rest of the bundle"
        );
        drop(ingress);
        let reused = scopes
            .reserve_ingress(4)
            .expect("dropping the bundle released every remaining scope");
        drop(reused);
    }

    #[test]
    fn ingress_reservation_is_gated_by_admission_before_any_pool_is_touched() {
        let plan = within_zone_plan();
        let scopes = plan.credit_scopes().expect("build the planned scope set");
        assert_eq!(
            plan.reserve_ingress(&scopes, 5).err(),
            Some(CreditError::AttachmentAllowanceExceeded)
        );
        let full = plan
            .reserve_ingress(&scopes, 4)
            .expect("the untouched pools still hold the full allowance");
        assert_eq!(
            plan.reserve_ingress(&scopes, 1).err(),
            Some(CreditError::ScopePool(ScopeCreditError::Exhausted))
        );
        drop(full);
        let reused = plan
            .reserve_ingress(&scopes, 4)
            .expect("dropping the bundle released every scope");
        drop(reused);
    }

    #[test]
    fn a_zone_link_plan_refuses_ingress_reservation_without_a_scope_set() {
        let within_zone = within_zone_plan();
        let scopes = within_zone
            .credit_scopes()
            .expect("build a scope set to offer the ZoneLink plan");
        let zone_link = AttachmentCreditPlan::zone_link();
        assert_eq!(
            zone_link.reserve_ingress(&scopes, 1).err(),
            Some(CreditError::ZoneLinkAttachmentsForbidden),
            "a ZoneLink plan must refuse even when a foreign scope set is offered"
        );
        let empty = zone_link
            .reserve_ingress(&scopes, 0)
            .expect("an attachment-free packet needs no credit");
        drop(empty);
    }

    #[test]
    fn error_reasons_are_stable_and_carry_no_identifiers() {
        for (error, rendered) in [
            (
                CreditError::ZoneLinkAttachmentsForbidden,
                "transport-credit-zone-link-attachments-forbidden",
            ),
            (
                CreditError::AttachmentPolicyNotPacketAtomic,
                "transport-credit-attachment-policy-not-packet-atomic",
            ),
            (
                CreditError::AttachmentPolicyInvalidForTransport,
                "transport-credit-attachment-policy-invalid-for-transport",
            ),
            (
                CreditError::AttachmentAllowanceOutOfBounds,
                "transport-credit-attachment-allowance-out-of-bounds",
            ),
            (
                CreditError::HostAllowanceOutOfBounds,
                "transport-credit-host-allowance-out-of-bounds",
            ),
            (
                CreditError::ProcessAllowanceOutOfBounds,
                "transport-credit-process-allowance-out-of-bounds",
            ),
            (
                CreditError::AttachmentAllowanceExceeded,
                "transport-credit-attachment-allowance-exceeded",
            ),
        ] {
            assert_eq!(error.to_string(), rendered);
        }
        assert_eq!(RouteClass::WithinZone.as_str(), "within-zone");
        assert_eq!(RouteClass::ZoneLink.as_str(), "zone-link");
    }
}
