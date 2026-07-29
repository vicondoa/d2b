use std::ops::Deref;

use d2b_bus::router::ComponentSessionAdmission;
use d2b_session::{AuthenticatedComponentSession, SessionAcceptor};
use opaque_claims::{PrincipalClaim, SerialClaim};

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
