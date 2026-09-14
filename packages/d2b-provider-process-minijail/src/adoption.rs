//! Minijail adoption identity helpers.

use d2b_process_conformance::{
    AdoptionCandidate, IdentityBinding, LaunchTicket, ProcessProviderProfile, WaitReapOwner,
};

/// Whether a candidate has enough broker-verified provenance for an exact
/// stale-process replacement.
pub fn is_stale_candidate(
    ticket: &LaunchTicket,
    candidate: &AdoptionCandidate,
    profile: &ProcessProviderProfile,
) -> bool {
    let required = profile
        .required_identity_bindings()
        .iter()
        .copied()
        .filter(|binding| *binding != IdentityBinding::Executable)
        .collect();
    candidate.wait_reap_owner == WaitReapOwner::Local
        && candidate.observed.covers(&required)
        && !candidate
            .observed
            .verified()
            .contains(&IdentityBinding::Executable)
        && ticket
            .validate_process_identity(&candidate.identity)
            .is_ok()
}
