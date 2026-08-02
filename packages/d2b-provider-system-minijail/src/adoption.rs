//! Minijail adoption identity helpers.

use std::collections::BTreeSet;

use d2b_process_conformance::{
    AdoptionCandidate, IdentityBinding, LaunchTicket, ProcessConformanceError, WaitReapOwner,
};

use crate::PROVIDER_NAME;

/// Verify the original broker-parent identity before pidfd duplication.
pub fn validate_candidate(
    ticket: &LaunchTicket,
    candidate: &AdoptionCandidate,
) -> Result<(), ProcessConformanceError> {
    let required = BTreeSet::from([
        IdentityBinding::Pid,
        IdentityBinding::ProcessStartTime,
        IdentityBinding::Cgroup,
        IdentityBinding::Executable,
        IdentityBinding::Template,
        IdentityBinding::Generation,
    ]);
    if ticket.selected_provider().as_str() != PROVIDER_NAME
        || candidate.wait_reap_owner != WaitReapOwner::Local
        || candidate.validate(&required).is_err()
    {
        Err(ProcessConformanceError::IdentityUnverified)
    } else {
        Ok(())
    }
}
