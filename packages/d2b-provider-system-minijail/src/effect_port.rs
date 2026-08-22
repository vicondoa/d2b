//! Opaque privileged effect-port seam for system-minijail.

use std::future::Future;

use d2b_process_conformance::{
    AdoptionCandidate, CancellationBinding, IdentityBinding, LaunchTicket, LaunchedProcess,
    PidfdEvidence, ProcessConformanceError, ProcessIdentityDigest, ProcessLaunchEffectPort,
    StopClass, WaitReapOwner,
};

/// The core-owned minijail effect port.
pub trait MinijailProcessEffectPort: Send + Sync {
    /// Spawn through the broker's clone3 effect.
    fn spawn(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<LaunchedProcess, ProcessConformanceError>> + Send;

    /// Find a candidate without using pidfd readability as identity.
    fn observe(
        &self,
        ticket: &LaunchTicket,
    ) -> impl Future<Output = Result<Option<AdoptionCandidate>, ProcessConformanceError>> + Send;

    /// Duplicate a verified pidfd without waiting or reaping.
    fn duplicate_pidfd(
        &self,
        candidate: &AdoptionCandidate,
    ) -> impl Future<Output = Result<PidfdEvidence, ProcessConformanceError>> + Send;

    /// Perform exact-main stop and the mandatory anchored-leaf cleanup.
    fn stop(
        &self,
        identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> impl Future<Output = Result<(), ProcessConformanceError>> + Send;
}

/// Adapt the minijail-specific effect port to the neutral conformance seam.
pub struct EffectPortAdapter<P>(pub P);

fn validate_ticket(ticket: &LaunchTicket) -> Result<(), ProcessConformanceError> {
    ticket.validate()?;
    if ticket.has_controller_launch_binding() {
        ticket.validate_controller_launch()?;
    }
    if ticket.has_assignment_binding() {
        ticket.validate_assignment()?;
    }
    if ticket.selected_provider().as_str() != crate::PROVIDER_NAME
        || ticket.provider_ref().to_canonical_string() != "Provider/system-minijail"
    {
        return Err(ProcessConformanceError::ProviderMismatch);
    }
    if ticket.operation().cancellation() == CancellationBinding::Cancelled {
        return Err(ProcessConformanceError::Cancelled);
    }
    Ok(())
}

impl<P> ProcessLaunchEffectPort for EffectPortAdapter<P>
where
    P: MinijailProcessEffectPort,
{
    async fn launch(
        &self,
        ticket: &LaunchTicket,
    ) -> Result<LaunchedProcess, ProcessConformanceError> {
        validate_ticket(ticket)?;
        self.0.spawn(ticket).await
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
    ) -> Result<PidfdEvidence, ProcessConformanceError> {
        if candidate.wait_reap_owner != WaitReapOwner::Local {
            return Err(ProcessConformanceError::WaitOwnerMismatch);
        }
        candidate.validate(&std::collections::BTreeSet::from([
            IdentityBinding::Pid,
            IdentityBinding::ProcessStartTime,
            IdentityBinding::Cgroup,
            IdentityBinding::Executable,
            IdentityBinding::Template,
            IdentityBinding::Generation,
        ]))?;
        self.0.duplicate_pidfd(candidate).await
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

    impl MinijailProcessEffectPort for RecordingPort {
        fn spawn(
            &self,
            _ticket: &LaunchTicket,
        ) -> impl Future<Output = Result<LaunchedProcess, ProcessConformanceError>> + Send {
            self.calls.lock().unwrap().push("spawn");
            std::future::ready(Ok(LaunchedProcess {
                identity: ProcessIdentityDigest::from_bytes([0x11; 32]),
                observed: ObservedIdentity::from_verified([
                    IdentityBinding::Pid,
                    IdentityBinding::ProcessStartTime,
                    IdentityBinding::Cgroup,
                    IdentityBinding::Executable,
                    IdentityBinding::Template,
                    IdentityBinding::Generation,
                ]),
                pidfd: PidfdEvidence::held(),
                wait_reap_owner: WaitReapOwner::Local,
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

        fn duplicate_pidfd(
            &self,
            _candidate: &AdoptionCandidate,
        ) -> impl Future<Output = Result<PidfdEvidence, ProcessConformanceError>> + Send {
            self.calls.lock().unwrap().push("duplicate_pidfd");
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
    fn effect_port_rejects_invalid_tickets_before_the_minijail_boundary() {
        let adapter = EffectPortAdapter(RecordingPort::default());
        let ticket = fixtures::ticket_builder()
            .selected_provider(crate::PROVIDER_NAME)
            .build()
            .unwrap()
            .with_readiness(d2b_process_conformance::ReadinessExpectation::Condition {
                timeout_ms: 0,
            });

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
            wait_reap_owner: WaitReapOwner::Local,
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
