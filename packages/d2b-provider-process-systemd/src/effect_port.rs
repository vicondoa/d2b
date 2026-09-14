//! Opaque systemd effect-port contract.
//!
//! The core ProviderSupervisor implements this trait.  The systemd Provider
//! itself sees only LaunchTicket and typed observations; it never receives a
//! D-Bus connection, unit name, PID, cgroup path, or user-manager handle.

use std::future::Future;

use d2b_process_conformance::{
    AdoptionCandidate, CancellationBinding, IdentityBinding, LaunchTicket, LaunchedProcess,
    ProcessConformanceError, ProcessIdentityDigest, ProcessLaunchEffectPort, StopClass,
    WaitReapOwner,
};

/// The systemd-specific effect seam.
pub trait SystemdProcessEffectPort: Send + Sync {
    /// Start a verified transient `Type=exec` service or user scope.
    fn start(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<LaunchedProcess, ProcessConformanceError>> + Send;

    /// Observe a candidate without opening a pidfd.
    fn observe(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<Option<AdoptionCandidate>, ProcessConformanceError>> + Send;

    /// Open a pidfd only after the core adapter verified the candidate.
    fn open_pidfd(
        &self,
        candidate: &AdoptionCandidate,
    ) -> impl Future<
        Output = Result<d2b_process_conformance::PidfdEvidence, ProcessConformanceError>,
    > + Send;

    /// Stop the exact opaque identity.
    fn stop(
        &self,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> impl Future<Output = Result<(), ProcessConformanceError>> + Send;
}

/// Adapt a systemd effect port to the neutral Process Provider seam.
pub struct EffectPortAdapter<P>(pub P);

fn validate_ticket(ticket: &LaunchTicket) -> Result<(), ProcessConformanceError> {
    ticket.validate()?;
    if ticket.has_controller_launch_binding() {
        ticket.validate_controller_launch()?;
    }
    if ticket.has_assignment_binding() {
        ticket.validate_assignment()?;
    }
    crate::launch::validate_launch_ticket(ticket)?;
    if ticket.operation().cancellation() == CancellationBinding::Cancelled {
        return Err(ProcessConformanceError::Cancelled);
    }
    Ok(())
}

impl<P> ProcessLaunchEffectPort for EffectPortAdapter<P>
where
    P: SystemdProcessEffectPort,
{
    async fn launch(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<LaunchedProcess, ProcessConformanceError> {
        validate_ticket(ticket)?;
        self.0.start(ticket).await
    }

    async fn observe(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<Option<AdoptionCandidate>, ProcessConformanceError> {
        validate_ticket(ticket)?;
        self.0.observe(ticket).await
    }

    async fn open_pidfd(
        &self,
        candidate: &AdoptionCandidate,
    ) -> Result<d2b_process_conformance::PidfdEvidence, ProcessConformanceError> {
        if candidate.wait_reap_owner != WaitReapOwner::ServiceManager {
            return Err(ProcessConformanceError::WaitOwnerMismatch);
        }
        candidate.validate(&std::collections::BTreeSet::from([
            IdentityBinding::UnitInvocationId,
            IdentityBinding::Cgroup,
            IdentityBinding::UnitMainPid,
            IdentityBinding::ProcessStartTime,
            IdentityBinding::Template,
            IdentityBinding::Generation,
        ]))?;
        self.0.open_pidfd(candidate).await
    }

    async fn stop(
        &self,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> Result<(), ProcessConformanceError> {
        if identity.is_zero() {
            return Err(ProcessConformanceError::IdentityUnverified);
        }
        self.0.stop(identity, class).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use d2b_process_conformance::testing::fixtures;
    use d2b_process_conformance::{
        ObservedIdentity, PidfdEvidence, ProcessIdentityDigest, WaitReapOwner,
    };

    use super::*;

    #[derive(Debug, Default)]
    struct RecordingPort {
        calls: Mutex<Vec<&'static str>>,
    }

    impl SystemdProcessEffectPort for RecordingPort {
        fn start(
            &self,
            _ticket: &LaunchTicket,
        ) -> impl Future<Output = Result<LaunchedProcess, ProcessConformanceError>> + Send {
            self.calls.lock().unwrap().push("start");
            std::future::ready(Ok(LaunchedProcess {
                identity: ProcessIdentityDigest::from_bytes([0x11; 32]),
                observed: ObservedIdentity::from_verified([
                    IdentityBinding::UnitInvocationId,
                    IdentityBinding::Cgroup,
                    IdentityBinding::UnitMainPid,
                    IdentityBinding::ProcessStartTime,
                    IdentityBinding::Template,
                    IdentityBinding::Generation,
                ]),
                pidfd: PidfdEvidence::held(),
                wait_reap_owner: WaitReapOwner::ServiceManager,
            }))
        }

        fn observe(
            &self,
            _ticket: &LaunchTicket,
        ) -> impl Future<Output = Result<Option<AdoptionCandidate>, ProcessConformanceError>> + Send
        {
            self.calls.lock().unwrap().push("observe");
            std::future::ready(Ok(None))
        }

        fn open_pidfd(
            &self,
            _candidate: &AdoptionCandidate,
        ) -> impl Future<
            Output = Result<d2b_process_conformance::PidfdEvidence, ProcessConformanceError>,
        > + Send {
            self.calls.lock().unwrap().push("open_pidfd");
            std::future::ready(Ok(PidfdEvidence::held()))
        }

        fn stop(
            &self,
            _identity: &ProcessIdentityDigest,
            _class: StopClass,
        ) -> impl Future<Output = Result<(), ProcessConformanceError>> + Send {
            self.calls.lock().unwrap().push("stop");
            std::future::ready(Ok(()))
        }
    }

    #[test]
    fn effect_port_rejects_invalid_tickets_before_the_systemd_boundary() {
        let adapter = EffectPortAdapter(RecordingPort::default());
        let ticket = fixtures::ticket_builder().build().unwrap().with_readiness(
            d2b_process_conformance::ReadinessExpectation::Condition { timeout_ms: 0 },
        );

        assert_eq!(
            d2b_process_conformance::testing::block_on(adapter.launch(&ticket)).unwrap_err(),
            ProcessConformanceError::InvalidTicket
        );
        assert!(adapter.0.calls.lock().unwrap().is_empty());
    }

    #[test]
    fn effect_port_rejects_unverified_pidfd_candidates_and_zero_stops() {
        let adapter = EffectPortAdapter(RecordingPort::default());
        let candidate = AdoptionCandidate {
            identity: ProcessIdentityDigest::from_bytes([0x11; 32]),
            observed: ObservedIdentity::default(),
            wait_reap_owner: WaitReapOwner::ServiceManager,
        };
        assert_eq!(
            d2b_process_conformance::testing::block_on(adapter.open_pidfd(&candidate)).unwrap_err(),
            ProcessConformanceError::IdentityUnverified
        );
        assert_eq!(
            d2b_process_conformance::testing::block_on(adapter.stop(
                &ProcessIdentityDigest::from_bytes([0; 32]),
                StopClass::Terminate,
            ))
            .unwrap_err(),
            ProcessConformanceError::IdentityUnverified
        );
        assert!(adapter.0.calls.lock().unwrap().is_empty());
    }
}
