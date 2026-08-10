//! Systemd adoption identity helpers.

use d2b_process_conformance::{
    AdoptionCandidate, IdentityBinding, LaunchTicket, ProcessConformanceError,
};

use crate::PROVIDER_NAME;

/// Check the stable systemd identity bindings before pidfd acquisition.
pub fn validate_candidate(
    ticket: &LaunchTicket,
    candidate: &AdoptionCandidate,
) -> Result<(), ProcessConformanceError> {
    if ticket.selected_provider().as_str() != PROVIDER_NAME
        || candidate.wait_reap_owner != d2b_process_conformance::WaitReapOwner::ServiceManager
        || candidate
            .validate(&std::collections::BTreeSet::from([
                IdentityBinding::UnitInvocationId,
                IdentityBinding::Cgroup,
                IdentityBinding::UnitMainPid,
                IdentityBinding::ProcessStartTime,
                IdentityBinding::Template,
                IdentityBinding::Generation,
            ]))
            .is_err()
    {
        Err(ProcessConformanceError::IdentityUnverified)
    } else {
        Ok(())
    }
}
