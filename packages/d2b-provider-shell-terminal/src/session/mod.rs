//! Session supervisor state, ring buffering, and restart adoption.

mod adopt;
mod ring;

pub use adopt::{
    AdoptionDecision, SupervisorCandidate, SupervisorIdentity, SupervisorObservation,
    adopt_supervisor,
};
pub use ring::{OutputRing, RingReplay};
