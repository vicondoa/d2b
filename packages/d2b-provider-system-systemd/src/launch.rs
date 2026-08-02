//! Systemd launch admission helpers.

use d2b_process_conformance::{LaunchTicket, ProcessConformanceError};

use crate::PROVIDER_NAME;

/// Validate the provider binding before a transient unit effect is queued.
pub fn validate_launch_ticket(ticket: &LaunchTicket) -> Result<(), ProcessConformanceError> {
    if ticket.selected_provider().as_str() == PROVIDER_NAME
        && ticket.provider_ref().to_canonical_string() == "Provider/system-systemd"
    {
        Ok(())
    } else {
        Err(ProcessConformanceError::ProviderMismatch)
    }
}
