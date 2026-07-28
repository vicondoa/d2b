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
