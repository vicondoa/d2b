use std::ops::Deref;

use d2b_bus::router::ComponentSessionAdmission;
use d2b_session::{AuthenticatedComponentSession, SessionAcceptor};
use opaque_claims::{PrincipalClaim, SerialClaim};

#[path = "../trait-impl-noncapability-direct-renamed-module.rs"]
mod noncapability_direct_module_alias;

#[path = "../trait-impl-noncapability-self-renamed-module.rs"]
mod noncapability_self_module_alias;

#[path = "../trait-impl-noncapability-plain-module.rs"]
mod noncapability_plain_module_alias;

#[path = "../trait-impl-noncapability-plain-self-module.rs"]
mod noncapability_plain_self_module_alias;

#[path = "../trait-impl-noncapability-chained-reexport-module.rs"]
mod noncapability_chained_reexport_module_alias;

#[cfg_attr(all(), doc = "Ordinary module with inert conditional attributes.")]
#[cfg_attr(all(), cfg_attr(all(), allow(dead_code)))]
#[path = "../module-cfg-attr-inert.rs"]
mod inert_module_cfg_attr;

pub fn rogue_admission() -> Option<ComponentSessionAdmission> {
    None
}

#[doc(hidden)]
pub fn hidden_rogue_admission() -> Option<ComponentSessionAdmission> {
    None
}

pub struct Rogue(RogueTarget);

pub struct RogueTarget;

impl RogueTarget {
    pub fn inherited_method(&self) {}
}

impl Deref for Rogue {
    type Target = RogueTarget;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

pub trait ConstructingTrait {
    fn construct(&self) -> Option<SessionAcceptor<()>>;
}

impl ConstructingTrait for Rogue {
    fn construct(&self) -> Option<SessionAcceptor<()>> {
        None
    }
}

impl Rogue {
    pub fn capability(&self) -> Option<AuthenticatedComponentSession<()>> {
        None
    }
}

pub struct RogueSubjectClaims {
    pub principal: PrincipalClaim,
    pub serial: SerialClaim,
}

impl RogueSubjectClaims {
    pub fn inject(principal: PrincipalClaim, serial: SerialClaim) -> Self {
        Self { principal, serial }
    }
}
