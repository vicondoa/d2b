//! Service-manager effect owner adapter and atomic unit identity.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::sync::Mutex;

use d2b_process::{
    BackendLaunch, BackendObservation, IdentityBinding, ObservedIdentity, ProcessEffectBackend,
    ProcessEffectError, ProcessIdentityDigest, ProcessRequest, ProcessStopClass, WaitReapOwner,
};
use sha2::{Digest, Sha256};

const MAX_PENDING_OBSERVATIONS: usize = 1024;

/// Atomic identity read from one active non-forking transient unit or scope.
///
/// The effect owner must obtain the invocation identifier, cgroup identity,
/// main process, and process start time from one coherent active-state query.
/// The template and generation digests bind that runtime tuple to trusted
/// launch configuration. Diagnostics reveal none of those values.
#[derive(Clone, PartialEq, Eq)]
pub struct SystemdInvocationIdentity {
    invocation_id: [u8; 16],
    cgroup_identity: [u8; 32],
    main_pid: NonZeroU32,
    start_time_ticks: u64,
    provider_identity: [u8; 32],
    template_identity: [u8; 32],
    generation: u64,
}

impl SystemdInvocationIdentity {
    /// Construct a complete service-manager identity tuple.
    pub fn new(
        invocation_id: [u8; 16],
        cgroup_identity: [u8; 32],
        main_pid: NonZeroU32,
        start_time_ticks: u64,
        provider_identity: [u8; 32],
        template_identity: [u8; 32],
        generation: u64,
    ) -> Result<Self, ProcessEffectError> {
        if invocation_id == [0; 16]
            || cgroup_identity == [0; 32]
            || start_time_ticks == 0
            || provider_identity == [0; 32]
            || template_identity == [0; 32]
            || generation == 0
        {
            return Err(ProcessEffectError::IdentityChanged);
        }
        Ok(Self {
            invocation_id,
            cgroup_identity,
            main_pid,
            start_time_ticks,
            provider_identity,
            template_identity,
            generation,
        })
    }

    fn digest(&self) -> ProcessIdentityDigest {
        let mut digest = Sha256::new();
        digest.update(b"d2b-systemd-process-identity-v1");
        digest.update(self.invocation_id);
        digest.update(self.cgroup_identity);
        digest.update(self.main_pid.get().to_le_bytes());
        digest.update(self.start_time_ticks.to_le_bytes());
        digest.update(self.provider_identity);
        digest.update(self.template_identity);
        digest.update(self.generation.to_le_bytes());
        ProcessIdentityDigest::from_bytes(digest.finalize().into())
    }

    fn observation(&self) -> BackendObservation {
        BackendObservation::new(
            self.digest(),
            ObservedIdentity::from_verified([
                IdentityBinding::UnitInvocationId,
                IdentityBinding::Cgroup,
                IdentityBinding::UnitMainPid,
                IdentityBinding::ProcessStartTime,
                IdentityBinding::Template,
                IdentityBinding::Generation,
            ]),
            WaitReapOwner::ServiceManager,
        )
    }
}

impl std::fmt::Debug for SystemdInvocationIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SystemdInvocationIdentity(<redacted>)")
    }
}

/// Result of a service-manager launch or descriptor re-open.
pub struct SystemdEffectLaunch<H> {
    identity: SystemdInvocationIdentity,
    handle: H,
}

impl<H> SystemdEffectLaunch<H> {
    /// Bind the atomically observed unit identity to its local descriptor.
    pub fn new(identity: SystemdInvocationIdentity, handle: H) -> Self {
        Self { identity, handle }
    }

    fn into_parts(self) -> (SystemdInvocationIdentity, H) {
        (self.identity, self.handle)
    }
}

impl<H> std::fmt::Debug for SystemdEffectLaunch<H> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SystemdEffectLaunch(<redacted>)")
    }
}

/// Blocking core-owned access to system and verified user managers.
///
/// Implementations resolve the ticket from trusted configuration, create only
/// non-forking transient units or scopes, and return an atomic identity tuple.
/// `reopen` must query the tuple again after opening the descriptor so a unit
/// replacement or main-process reuse cannot be adopted.
pub trait SystemdEffectOwner: Send + Sync + 'static {
    /// Core-local pidfd or equivalent exact-main authority.
    type Handle: Send + Sync + 'static;

    /// Launch one transient unit or verified user scope.
    fn launch(
        &self,
        request: ProcessRequest,
    ) -> Result<SystemdEffectLaunch<Self::Handle>, ProcessEffectError>;

    /// Observe a transient unit without opening local process authority.
    fn observe(
        &self,
        request: ProcessRequest,
    ) -> Result<Option<SystemdInvocationIdentity>, ProcessEffectError>;

    /// Open local authority and atomically re-query the unit identity.
    fn reopen(
        &self,
        expected: &SystemdInvocationIdentity,
    ) -> Result<SystemdEffectLaunch<Self::Handle>, ProcessEffectError>;

    /// Stop only the unit represented by the verified local handle.
    ///
    /// A successful [`ProcessStopClass::Terminate`] result certifies that the
    /// unit's represented process no longer survives.
    fn stop(
        &self,
        handle: &Self::Handle,
        class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError>;
}

/// [`ProcessEffectBackend`] over a real service-manager effect owner.
pub struct SystemdProcessBackend<O: SystemdEffectOwner> {
    owner: O,
    observations: Mutex<BTreeMap<ProcessIdentityDigest, SystemdInvocationIdentity>>,
}

impl<O: SystemdEffectOwner> SystemdProcessBackend<O> {
    /// Wrap a core-owned service-manager effect owner.
    pub fn new(owner: O) -> Self {
        Self {
            owner,
            observations: Mutex::new(BTreeMap::new()),
        }
    }

    fn record(&self, identity: SystemdInvocationIdentity) -> Result<(), ProcessEffectError> {
        let mut observations = self
            .observations
            .lock()
            .map_err(|_| ProcessEffectError::ObserveFailed)?;
        let digest = identity.digest();
        if observations.len() >= MAX_PENDING_OBSERVATIONS
            && !observations.contains_key(&digest)
            && let Some(oldest) = observations.keys().next().copied()
        {
            observations.remove(&oldest);
        }
        observations.insert(digest, identity);
        Ok(())
    }

    fn take_observation(
        &self,
        identity: &ProcessIdentityDigest,
    ) -> Result<SystemdInvocationIdentity, ProcessEffectError> {
        self.observations
            .lock()
            .map_err(|_| ProcessEffectError::ObserveFailed)?
            .remove(identity)
            .ok_or(ProcessEffectError::IdentityChanged)
    }
}

#[cfg(test)]
// Keep focused observation tests beside the state helpers they exercise.
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;

    struct Owner;

    impl SystemdEffectOwner for Owner {
        type Handle = ();

        fn launch(
            &self,
            _request: ProcessRequest,
        ) -> Result<SystemdEffectLaunch<Self::Handle>, ProcessEffectError> {
            Err(ProcessEffectError::LaunchFailed)
        }

        fn observe(
            &self,
            _request: ProcessRequest,
        ) -> Result<Option<SystemdInvocationIdentity>, ProcessEffectError> {
            Ok(None)
        }

        fn reopen(
            &self,
            _expected: &SystemdInvocationIdentity,
        ) -> Result<SystemdEffectLaunch<Self::Handle>, ProcessEffectError> {
            Err(ProcessEffectError::PidfdUnavailable)
        }

        fn stop(
            &self,
            _handle: &Self::Handle,
            _class: ProcessStopClass,
        ) -> Result<(), ProcessEffectError> {
            Ok(())
        }
    }

    fn identity(seed: u32) -> SystemdInvocationIdentity {
        let mut invocation_id = [0; 16];
        invocation_id[..4].copy_from_slice(&(seed + 1).to_le_bytes());
        SystemdInvocationIdentity::new(
            invocation_id,
            [1; 32],
            NonZeroU32::new(seed + 1).unwrap(),
            u64::from(seed) + 1,
            [2; 32],
            [3; 32],
            1,
        )
        .unwrap()
    }

    #[test]
    fn pending_systemd_observations_are_bounded_and_consumed() {
        let backend = SystemdProcessBackend::new(Owner);
        for seed in 0..=u32::try_from(MAX_PENDING_OBSERVATIONS).unwrap() {
            backend.record(identity(seed)).unwrap();
        }
        assert_eq!(
            backend.observations.lock().unwrap().len(),
            MAX_PENDING_OBSERVATIONS
        );
        let digest = identity(u32::try_from(MAX_PENDING_OBSERVATIONS).unwrap()).digest();
        backend.take_observation(&digest).unwrap();
        assert_eq!(
            backend.observations.lock().unwrap().len(),
            MAX_PENDING_OBSERVATIONS - 1
        );
    }

    #[test]
    fn systemd_identity_diagnostics_are_redacted() {
        assert_eq!(
            format!("{:?}", identity(41)),
            "SystemdInvocationIdentity(<redacted>)"
        );
    }
}

impl<O: SystemdEffectOwner> std::fmt::Debug for SystemdProcessBackend<O> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SystemdProcessBackend(<redacted>)")
    }
}

impl<O: SystemdEffectOwner> ProcessEffectBackend for SystemdProcessBackend<O> {
    type Handle = O::Handle;

    fn launch(
        &self,
        request: ProcessRequest,
    ) -> Result<BackendLaunch<Self::Handle>, ProcessEffectError> {
        let launch = self.owner.launch(request)?;
        let (identity, handle) = launch.into_parts();
        let observation = identity.observation();
        Ok(BackendLaunch::new(observation, handle))
    }

    fn observe(
        &self,
        request: ProcessRequest,
    ) -> Result<Option<BackendObservation>, ProcessEffectError> {
        let Some(identity) = self.owner.observe(request)? else {
            return Ok(None);
        };
        let observation = identity.observation();
        self.record(identity)?;
        Ok(Some(observation))
    }

    fn open_pidfd(
        &self,
        observation: BackendObservation,
    ) -> Result<Self::Handle, ProcessEffectError> {
        let expected = self.take_observation(&observation.identity())?;
        let reopened = self.owner.reopen(&expected)?;
        let (actual, handle) = reopened.into_parts();
        if actual != expected || actual.digest() != observation.identity() {
            return Err(ProcessEffectError::IdentityChanged);
        }
        Ok(handle)
    }

    fn stop(
        &self,
        handle: &Self::Handle,
        class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError> {
        self.owner.stop(handle, class)
    }
}
