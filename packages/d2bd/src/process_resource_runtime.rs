//! What the pre-v3 plane still reads from the Process family.
//!
//! The Process/EphemeralProcess reconciliation itself lives in the family's
//! own crate, together with the one canonical launch-identity resolver every
//! ticket consumer shares. What stays here is exactly what other pre-v3
//! surfaces consume: the restart annotation the interaction composition's
//! Process specs still carry.

pub(crate) const PROCESS_RESTART_ANNOTATION: &str = "d2b.d2bus.org/restart-generation";
