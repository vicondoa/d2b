//! Core-owned process effect backend contract.

use std::fmt;

use d2b_process_conformance::{
    LaunchTicket, ObservedIdentity, ProcessIdentityDigest, WaitReapOwner,
};

/// One owned request passed from the async adapter to a blocking effect owner.
///
/// The request deliberately wraps the validated ticket instead of expanding it
/// into host values. A broker or service-manager resolver must derive every
/// operating-system detail from trusted configuration.
#[derive(Clone)]
pub struct ProcessRequest {
    ticket: LaunchTicket,
}

impl ProcessRequest {
    /// Build a backend request from a validated launch ticket.
    pub fn new(ticket: LaunchTicket) -> Self {
        Self { ticket }
    }

    /// Borrow the validated launch ticket.
    pub const fn ticket(&self) -> &LaunchTicket {
        &self.ticket
    }
}

impl fmt::Debug for ProcessRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProcessRequest(<redacted>)")
    }
}

/// A stable process observation produced before a local handle is opened.
///
/// Sensitive identity material remains inside the effect owner. The Process
/// Provider sees only the digest, the closed verified-binding set, and the
/// closed wait/reap owner.
#[derive(Clone, PartialEq, Eq)]
pub struct BackendObservation {
    identity: ProcessIdentityDigest,
    observed: ObservedIdentity,
    wait_reap_owner: WaitReapOwner,
}

impl BackendObservation {
    /// Record a stable observation after the effect owner verified it.
    pub fn new(
        identity: ProcessIdentityDigest,
        observed: ObservedIdentity,
        wait_reap_owner: WaitReapOwner,
    ) -> Self {
        Self {
            identity,
            observed,
            wait_reap_owner,
        }
    }

    /// Return the opaque stable process identity.
    pub const fn identity(&self) -> ProcessIdentityDigest {
        self.identity
    }

    /// Borrow the exact set of verified identity bindings.
    pub const fn observed(&self) -> &ObservedIdentity {
        &self.observed
    }

    /// Return the effect owner responsible for wait and reap.
    pub const fn wait_reap_owner(&self) -> WaitReapOwner {
        self.wait_reap_owner
    }
}

impl fmt::Debug for BackendObservation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackendObservation(<redacted>)")
    }
}

/// A successful backend launch with its locally held process handle.
///
/// For a broker launch the handle owns the descriptor received through
/// `SCM_RIGHTS`. For a service-manager launch it owns the locally verified
/// descriptor associated with the atomic unit observation.
pub struct BackendLaunch<H> {
    observation: BackendObservation,
    handle: H,
}

impl<H> BackendLaunch<H> {
    /// Bind a verified launch observation to its local handle.
    pub fn new(observation: BackendObservation, handle: H) -> Self {
        Self {
            observation,
            handle,
        }
    }

    /// Borrow the verified launch observation.
    pub const fn observation(&self) -> &BackendObservation {
        &self.observation
    }

    /// Split the launch into its observation and local handle.
    pub fn into_parts(self) -> (BackendObservation, H) {
        (self.observation, self.handle)
    }
}

impl<H> fmt::Debug for BackendLaunch<H> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BackendLaunch(<redacted>)")
    }
}

/// Stop class understood by a blocking process effect owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessStopClass {
    /// Request bounded graceful drain.
    Drain,
    /// Terminate the exact verified identity.
    Terminate,
}

/// Closed failures from a core process effect owner.
///
/// Variants carry no caller or host value, making both `Debug` and `Display`
/// safe for errors, audit summaries, and bounded telemetry labels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ProcessEffectError {
    /// The selected Process Provider has no production effect owner.
    UnsupportedProvider,
    /// Trusted launch configuration could not be resolved.
    ResolutionFailed,
    /// The broker or service manager refused or failed the launch.
    LaunchFailed,
    /// The process could not be observed safely.
    ObserveFailed,
    /// Stable identity changed or could not be verified.
    IdentityChanged,
    /// A verified local descriptor could not be obtained.
    PidfdUnavailable,
    /// The expected process or transient unit no longer exists.
    Vanished,
    /// Wait/reap ownership disagreed with the selected Provider.
    WaitOwnerMismatch,
    /// The bounded blocking adapter had no admission capacity.
    Busy,
    /// The effect did not complete inside the ticket deadline.
    DeadlineExceeded,
    /// The exact verified process could not be stopped.
    StopFailed,
}

impl ProcessEffectError {
    /// Return the stable lower-kebab error code.
    pub const fn code(self) -> &'static str {
        match self {
            Self::UnsupportedProvider => "unsupported-provider",
            Self::ResolutionFailed => "resolution-failed",
            Self::LaunchFailed => "launch-failed",
            Self::ObserveFailed => "observe-failed",
            Self::IdentityChanged => "identity-changed",
            Self::PidfdUnavailable => "pidfd-unavailable",
            Self::Vanished => "process-vanished",
            Self::WaitOwnerMismatch => "wait-owner-mismatch",
            Self::Busy => "effect-adapter-busy",
            Self::DeadlineExceeded => "deadline-exceeded",
            Self::StopFailed => "stop-failed",
        }
    }
}

impl fmt::Display for ProcessEffectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for ProcessEffectError {}

/// Blocking core-owned process effect operations.
///
/// Implementations may perform broker socket, service-manager, kernel, and
/// filesystem I/O. [`d2b_provider_supervisor`](https://docs.rs/d2b-provider-supervisor)
/// always invokes these methods on its bounded blocking adapter, never on the
/// Process controller's async executor thread.
pub trait ProcessEffectBackend: Send + Sync + 'static {
    /// Local process authority retained by the core adapter.
    type Handle: Send + Sync + 'static;

    /// Resolve and launch one ticket, returning mandatory local authority.
    fn launch(
        &self,
        request: ProcessRequest,
    ) -> Result<BackendLaunch<Self::Handle>, ProcessEffectError>;

    /// Observe a candidate without opening a new local descriptor.
    fn observe(
        &self,
        request: ProcessRequest,
    ) -> Result<Option<BackendObservation>, ProcessEffectError>;

    /// Re-verify an observation and open fresh local authority.
    fn open_pidfd(
        &self,
        observation: BackendObservation,
    ) -> Result<Self::Handle, ProcessEffectError>;

    /// Stop only the exact process represented by `handle`.
    ///
    /// A successful [`ProcessStopClass::Terminate`] result certifies that the
    /// represented process no longer survives. Accepting a signal or stop
    /// request without confirming exit is not success.
    fn stop(
        &self,
        handle: &Self::Handle,
        class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_process_conformance::testing::fixtures;

    #[test]
    fn diagnostics_are_value_free() {
        let request = ProcessRequest::new(fixtures::ticket_builder().build().unwrap());
        let observation = BackendObservation::new(
            ProcessIdentityDigest::from_bytes([7; 32]),
            ObservedIdentity::default(),
            WaitReapOwner::Local,
        );
        assert_eq!(format!("{request:?}"), "ProcessRequest(<redacted>)");
        assert_eq!(format!("{observation:?}"), "BackendObservation(<redacted>)");
        assert_eq!(
            format!("{:?}", BackendLaunch::new(observation, ())),
            "BackendLaunch(<redacted>)"
        );
        for error in [
            ProcessEffectError::ResolutionFailed,
            ProcessEffectError::LaunchFailed,
            ProcessEffectError::IdentityChanged,
            ProcessEffectError::PidfdUnavailable,
            ProcessEffectError::Vanished,
            ProcessEffectError::WaitOwnerMismatch,
            ProcessEffectError::Busy,
            ProcessEffectError::DeadlineExceeded,
            ProcessEffectError::StopFailed,
        ] {
            assert_eq!(error.to_string(), error.code());
        }
    }
}
