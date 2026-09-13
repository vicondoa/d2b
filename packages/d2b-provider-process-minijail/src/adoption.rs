//! Minijail adoption identity helpers.

use d2b_process_conformance::{
    AdoptionCandidate, IdentityBinding, LaunchTicket, ProcessConformanceError,
    ProcessProviderProfile, WaitReapOwner,
};

use crate::PROVIDER_NAME;

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

/// Verify the original broker-parent identity before pidfd duplication.
pub fn validate_candidate(
    ticket: &LaunchTicket,
    candidate: &AdoptionCandidate,
) -> Result<(), ProcessConformanceError> {
    let required = std::collections::BTreeSet::from([
        IdentityBinding::Pid,
        IdentityBinding::ProcessStartTime,
        IdentityBinding::Cgroup,
        IdentityBinding::Executable,
        IdentityBinding::Template,
        IdentityBinding::Generation,
    ]);
    if ticket.selected_provider().as_str() != PROVIDER_NAME
        || ticket.provider_ref().to_canonical_string() != "Provider/system-minijail"
        || candidate.wait_reap_owner != WaitReapOwner::Local
        || candidate.validate(&required).is_err()
    {
        Err(ProcessConformanceError::IdentityUnverified)
    } else {
        Ok(())
    }
}
