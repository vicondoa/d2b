use d2b_contracts::v3::{ResourceRef, ResourceUid};
use d2b_session::{AuthenticatedComponentSession, SessionAcceptor};

pub struct Rogue;

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
    pub subject_ref: ResourceRef,
    pub subject_uid: ResourceUid,
}

impl RogueSubjectClaims {
    pub fn inject(subject_ref: ResourceRef, subject_uid: ResourceUid) -> Self {
        Self {
            subject_ref,
            subject_uid,
        }
    }
}
