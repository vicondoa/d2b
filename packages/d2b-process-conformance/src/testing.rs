//! Shared test doubles and fixtures for the Process conformance suite.
//!
//! Both Provider crates build their controller over the same scripted
//! effect port so the suite can assert the neutral obligations without a
//! systemd bus, a broker socket, a privileged host, or a real process.

use std::collections::BTreeSet;
use std::future::Future;
use std::pin::pin;
use std::sync::Mutex;
use std::task::{Context, Poll, Waker};

use d2b_contracts::v3::execution_policy::{BoundedToken, ExecutionDomain};
use d2b_contracts::v3::{ControllerGeneration, ResourceGeneration, ResourceRef, ResourceUid};

use crate::error::ProcessConformanceError;
use crate::identity::{
    ConfigurationDigest, IdentityBinding, ObservedIdentity, PidfdEvidence, ProcessIdentityDigest,
    WaitReapOwner,
};
use crate::port::{AdoptionCandidate, LaunchedProcess, ProcessLaunchEffectPort, StopClass};
use crate::ticket::{CompiledDigests, LaunchTicket, OperationBinding};

/// Drive a future to completion on the calling thread.
///
/// The conformance suite is hermetic and never waits on I/O or wall time,
/// so a busy-free single-poll driver is sufficient and keeps the crate free
/// of an async runtime dependency.
pub fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = pin!(future);
    let waker = Waker::noop();
    let mut context = Context::from_waker(waker);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(value) => return value,
            Poll::Pending => std::hint::spin_loop(),
        }
    }
}

/// One recorded effect-port call.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PortCall {
    /// [`ProcessLaunchEffectPort::launch`] was called.
    Launch,
    /// [`ProcessLaunchEffectPort::observe`] was called.
    Observe,
    /// [`ProcessLaunchEffectPort::open_pidfd`] was called.
    OpenPidfd,
    /// [`ProcessLaunchEffectPort::stop`] was called.
    Stop(StopClass),
}

/// A scripted, recording [`ProcessLaunchEffectPort`].
#[derive(Debug)]
pub struct ScriptedEffectPort {
    identity: ProcessIdentityDigest,
    launch_observed: ObservedIdentity,
    launch_wait_owner: WaitReapOwner,
    launch_error: Option<ProcessConformanceError>,
    candidate: Option<AdoptionCandidate>,
    calls: Mutex<Vec<PortCall>>,
}

impl ScriptedEffectPort {
    /// Build a port that launches successfully, verifying `verified`.
    pub fn launching(
        verified: impl IntoIterator<Item = IdentityBinding>,
        wait_owner: WaitReapOwner,
    ) -> Self {
        Self {
            identity: ProcessIdentityDigest::from_bytes([0x11; 32]),
            launch_observed: ObservedIdentity::from_verified(verified),
            launch_wait_owner: wait_owner,
            launch_error: None,
            candidate: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Build a port whose launch fails with `error`.
    pub fn failing(error: ProcessConformanceError, wait_owner: WaitReapOwner) -> Self {
        Self {
            identity: ProcessIdentityDigest::from_bytes([0x11; 32]),
            launch_observed: ObservedIdentity::default(),
            launch_wait_owner: wait_owner,
            launch_error: Some(error),
            candidate: None,
            calls: Mutex::new(Vec::new()),
        }
    }

    /// Script an already running process for adoption.
    pub fn with_candidate(
        mut self,
        verified: impl IntoIterator<Item = IdentityBinding>,
        wait_owner: WaitReapOwner,
    ) -> Self {
        self.candidate = Some(AdoptionCandidate {
            identity: self.identity,
            observed: ObservedIdentity::from_verified(verified),
            wait_reap_owner: wait_owner,
        });
        self
    }

    /// Return every recorded call in order.
    pub fn calls(&self) -> Vec<PortCall> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }

    fn record(&self, call: PortCall) {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(call);
        }
    }
}

impl ProcessLaunchEffectPort for ScriptedEffectPort {
    async fn launch(
        &self,
        _ticket: &LaunchTicket,
    ) -> Result<LaunchedProcess, ProcessConformanceError> {
        self.record(PortCall::Launch);
        if let Some(error) = self.launch_error {
            return Err(error);
        }
        Ok(LaunchedProcess {
            identity: self.identity,
            observed: self.launch_observed.clone(),
            pidfd: PidfdEvidence::held(),
            wait_reap_owner: self.launch_wait_owner,
        })
    }

    async fn observe(
        &self,
        _ticket: &LaunchTicket,
    ) -> Result<Option<AdoptionCandidate>, ProcessConformanceError> {
        self.record(PortCall::Observe);
        Ok(self.candidate.clone())
    }

    async fn open_pidfd(
        &self,
        _candidate: &AdoptionCandidate,
    ) -> Result<PidfdEvidence, ProcessConformanceError> {
        self.record(PortCall::OpenPidfd);
        Ok(PidfdEvidence::held())
    }

    async fn stop(
        &self,
        _identity: &ProcessIdentityDigest,
        class: StopClass,
    ) -> Result<(), ProcessConformanceError> {
        self.record(PortCall::Stop(class));
        Ok(())
    }
}

/// Canonical launch-ticket fixtures.
pub mod fixtures {
    use super::*;

    /// A stable operation UID.
    pub fn operation_uid() -> ResourceUid {
        ResourceUid::parse("6f9619ff-8b86-4d01-b42d-00cf4fc964ff").expect("valid fixture uid")
    }

    fn token(value: &str) -> BoundedToken {
        BoundedToken::parse(value).expect("valid fixture token")
    }

    fn digest(seed: u8) -> ConfigurationDigest {
        ConfigurationDigest::from_bytes([seed; 32])
    }

    /// The canonical compiled digest set.
    pub fn compiled_digests() -> CompiledDigests {
        CompiledDigests {
            sandbox: digest(1),
            budget: digest(2),
            mounts: digest(3),
            devices: digest(4),
            network: digest(5),
            endpoints: digest(6),
            fd_table: digest(7),
        }
    }

    /// A mutable launch-ticket fixture.
    #[derive(Debug, Clone)]
    pub struct TicketBuilder {
        process_ref: ResourceRef,
        execution_ref: ResourceRef,
        domain: ExecutionDomain,
        user_ref: Option<ResourceRef>,
        selected_provider: BoundedToken,
        expected_identity: BTreeSet<IdentityBinding>,
    }

    impl TicketBuilder {
        /// Override the Process reference.
        pub fn process_ref(mut self, value: ResourceRef) -> Self {
            self.process_ref = value;
            self
        }

        /// Override the Host or Guest reference.
        pub fn execution_ref(mut self, value: ResourceRef) -> Self {
            self.execution_ref = value;
            self
        }

        /// Override the execution domain.
        pub fn domain(mut self, value: ExecutionDomain) -> Self {
            self.domain = value;
            self
        }

        /// Override the exact user reference.
        pub fn user_ref(mut self, value: Option<ResourceRef>) -> Self {
            self.user_ref = value;
            self
        }

        /// Override the selected Process Provider.
        pub fn selected_provider(mut self, value: &str) -> Self {
            self.selected_provider = token(value);
            self
        }

        /// Override the expected identity bindings.
        pub fn expected_identity(
            mut self,
            value: impl IntoIterator<Item = IdentityBinding>,
        ) -> Self {
            self.expected_identity = value.into_iter().collect();
            self
        }

        /// Build the ticket.
        pub fn build(self) -> Result<LaunchTicket, ProcessConformanceError> {
            LaunchTicket::new(
                self.process_ref,
                operation_uid(),
                ResourceGeneration::new(1).expect("nonzero"),
                ControllerGeneration::new(1).expect("nonzero"),
                token("system-systemd"),
                token("controller"),
                token("controller-main"),
                self.execution_ref,
                self.domain,
                self.user_ref,
                self.selected_provider,
                compiled_digests(),
                OperationBinding::new(operation_uid(), 30_000)?,
                self.expected_identity,
            )
        }
    }

    /// A system-domain Host ticket selecting `system-systemd`.
    pub fn ticket_builder() -> TicketBuilder {
        TicketBuilder {
            process_ref: ResourceRef::parse("Process/controller-main").expect("valid fixture ref"),
            execution_ref: ResourceRef::parse("Host/host-system").expect("valid fixture ref"),
            domain: ExecutionDomain::System,
            user_ref: None,
            selected_provider: token("system-systemd"),
            expected_identity: BTreeSet::from([IdentityBinding::Cgroup]),
        }
    }
}
