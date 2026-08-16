//! Guest status projection helpers.

use crate::types::{GuestPhase, GuestStatus, ProviderPhase};

/// Build a bounded status from process and QMP observations.
pub fn status_for(process_ready: bool, qmp_ready: bool, paused_at_boot: bool) -> GuestStatus {
    let phase = if process_ready && qmp_ready {
        GuestPhase::Ready
    } else {
        GuestPhase::Pending
    };
    let provider_phase = if !process_ready {
        ProviderPhase::LaunchingRunner
    } else if !qmp_ready {
        ProviderPhase::WaitingQmp
    } else if paused_at_boot {
        ProviderPhase::PausedAtBoot
    } else {
        ProviderPhase::Running
    };
    let mut status = GuestStatus::new(phase, provider_phase);
    status.resource.runtime_ready = process_ready;
    status.resource.bootstrap_ready = qmp_ready;
    status.resource.active_process_count = u16::from(process_ready);
    status
}
