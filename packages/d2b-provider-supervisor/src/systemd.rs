//! Service-manager effect owner adapter and atomic unit identity.

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::os::fd::OwnedFd;
use std::sync::Mutex;

use d2b_contracts_broker::broker_wire::{
    BrokerCallerRole, GuestExecutionBinding, ObserveUnitResponse, OpenUnitPidfdRequest,
    OpenUnitPidfdResponse, StartTransientUnitResponse, StopUnitRequest, StopUnitResponse,
    UnitStopClass, UnitDomain, UnitIdentity, UnitRequest,
};
use d2b_contracts_broker::kernel_client::{
    KernelInvocation, KernelInvokeError, KernelReply, envelope_invoke_kernel,
};
use d2b_contracts_resource::v3::execution_policy::ExecutionDomain;
use d2b_provider_process::{
    BackendLaunch, BackendObservation, IdentityBinding, ObservedIdentity, ProcessEffectBackend,
    ProcessEffectError, ProcessIdentityDigest, ProcessRequest, ProcessStopClass, WaitReapOwner,
};
use sha2::{Digest, Sha256};
use tracing::{error, warn};

use crate::broker::{
    BrokerLaunchIntent, BrokerLaunchResolver, BundleBackedLaunchResolver, wait_pidfd_observer,
};

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
    bundle_content_identity: String,
    guest_execution: Option<GuestExecutionBinding>,
}

impl SystemdInvocationIdentity {
    /// Construct the trusted identity tuple from an atomically observed unit
    /// identity.
    ///
    /// A zero main pid, zero generation, or an empty bundle content identity
    /// is a drifted runtime tuple, not a launchable identity.
    pub fn new(identity: &UnitIdentity) -> Result<Self, ProcessEffectError> {
        let main_pid =
            NonZeroU32::new(identity.main_pid).ok_or(ProcessEffectError::IdentityChanged)?;
        if identity.invocation_id == [0; 16]
            || identity.cgroup_identity == [0; 32]
            || identity.start_time_ticks == 0
            || identity.provider_identity == [0; 32]
            || identity.template_identity == [0; 32]
            || identity.generation == 0
            || identity.bundle_content_identity.is_empty()
        {
            return Err(ProcessEffectError::IdentityChanged);
        }
        Ok(Self {
            invocation_id: identity.invocation_id,
            cgroup_identity: identity.cgroup_identity,
            main_pid,
            start_time_ticks: identity.start_time_ticks,
            provider_identity: identity.provider_identity,
            template_identity: identity.template_identity,
            generation: identity.generation,
            bundle_content_identity: identity.bundle_content_identity.clone(),
            guest_execution: identity.guest_execution.clone(),
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
        digest.update(self.bundle_content_identity.as_bytes());
        if let Some(binding) = &self.guest_execution {
            digest.update(binding.target_uid.as_str().as_bytes());
            digest.update(binding.boot_identity_digest);
            digest.update(binding.session_generation.to_le_bytes());
            digest.update(binding.assignment_epoch.to_le_bytes());
            digest.update(binding.provider_generation.to_le_bytes());
            digest.update(binding.controller_generation.to_le_bytes());
        }
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

    pub(crate) fn wire_identity(&self) -> UnitIdentity {
        UnitIdentity {
            invocation_id: self.invocation_id,
            cgroup_identity: self.cgroup_identity,
            main_pid: self.main_pid.get(),
            start_time_ticks: self.start_time_ticks,
            provider_identity: self.provider_identity,
            template_identity: self.template_identity,
            generation: self.generation,
            bundle_content_identity: self.bundle_content_identity.clone(),
            guest_execution: self.guest_execution.clone(),
        }
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

    /// Probe a transient unit without retaining adoption state.
    fn probe(
        &self,
        request: ProcessRequest,
    ) -> Result<Option<SystemdInvocationIdentity>, ProcessEffectError> {
        self.observe(request)
    }

    /// Open local authority and atomically re-query the unit identity.
    fn reopen(
        &self,
        expected: &SystemdInvocationIdentity,
    ) -> Result<SystemdEffectLaunch<Self::Handle>, ProcessEffectError>;

    /// Wait for the exact local authority to become readable.
    fn wait(
        &self,
        _handle: &Self::Handle,
        _timeout: std::time::Duration,
    ) -> Result<(), ProcessEffectError> {
        Err(ProcessEffectError::PidfdUnavailable)
    }

    /// Stop only the unit represented by the verified local handle.
    ///
    /// A successful [`ProcessStopClass::Terminate`] result certifies that the
    /// unit's represented process no longer survives.
    fn stop(
        &self,
        handle: &Self::Handle,
        class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError>;

    /// Forget a terminal unit identity after the unit is no longer active.
    fn finalize(&self, _handle: &Self::Handle) -> Result<(), ProcessEffectError> {
        Ok(())
    }
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

    // Sync by construction:the ledger sits behind the sync trait surface;the
    // shared helper's critical section is short and never held across a suspension
    // point.
    fn record(&self, identity: SystemdInvocationIdentity) -> Result<(), ProcessEffectError> {
        crate::observations::record(&self.observations, identity.digest(), identity)
    }

    // Sync by construction: backend ledger behind the sync trait surface (see
    // `record`); critical section short, no suspension inside the guard.

    fn take_observation(
        &self,
        identity: &ProcessIdentityDigest,
    ) -> Result<SystemdInvocationIdentity, ProcessEffectError> {
        crate::observations::take(&self.observations, identity)
    }
}

#[cfg(test)]
// Keep focused observation tests beside the state helpers they exercise.
#[allow(clippy::items_after_test_module)]
mod tests {
    use crate::observations::MAX_PENDING_OBSERVATIONS;

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
        SystemdInvocationIdentity::new(&UnitIdentity {
            invocation_id,
            cgroup_identity: [1; 32],
            main_pid: seed + 1,
            start_time_ticks: u64::from(seed) + 1,
            provider_identity: [2; 32],
            template_identity: [3; 32],
            generation: 1,
            bundle_content_identity: "bundle".to_owned(),
            guest_execution: None,
        })
        .unwrap()
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
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

    #[test]
    fn systemd_adoption_identity_binds_bundle_content_identity() {
        let mut first = identity(41);
        let mut second = identity(41);
        first.bundle_content_identity = "bundle-a".to_owned();
        second.bundle_content_identity = "bundle-b".to_owned();
        assert_ne!(first.digest(), second.digest());
    }

    // -- the broker-backed effect owner ------------------------------------

    use d2b_contracts::types::{BundleOpId, RoleId, VmId};
    use d2b_contracts_broker::broker_wire::{RunnerRole, UnitRequest};
    use d2b_core::bundle::{Bundle, BundleGeneration};
    use d2b_core::bundle_resolver::BundleResolver;
    use d2b_core::host::HostJson;
    use d2b_core::manifest_v04::ManifestV04;
    use d2b_core::processes::ProcessesJson;
    use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
    use std::collections::BTreeMap;

    /// The trusted-bundle fixture the driver tests share: host fixture +
    /// golden v04 manifest, no zone resource bundles. The ledger and
    /// identity-fence legs never consult the bundle, so this is enough.
    fn fixture_bundle() -> BundleResolver {
        let host: HostJson = serde_json::from_str(include_str!(
            "../../../tests/fixtures/deny-unknown/host-valid.json"
        ))
        .expect("host fixture");
        let manifest = ManifestV04::from_slice(
            include_str!("../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
        )
        .expect("manifest fixture");
        BundleResolver::from_artifacts_with_zone_resource_bundles(
            Bundle {
                bundle_version: 1,
                schema_version: "v3".to_owned(),
                privileges_path: "privileges.json".to_owned(),
                storage_path: None,
                realm_workloads_launcher_v2_path: None,
                generation: BundleGeneration {
                    generator: "test".to_owned(),
                    source_revision: None,
                    generated_at: None,
                },
                bundle_hash: Some("sha256:bundle".to_owned()),
                artifact_hashes: None,
            },
            host,
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            manifest,
            BTreeMap::new(),
        )
    }

    fn broker_owner() -> BrokerSystemdEffectOwner {
        BrokerSystemdEffectOwner::with_socket_and_role(
            BundleBackedLaunchResolver::new(fixture_bundle()),
            "/unused",
            std::time::Duration::from_millis(1),
            BrokerCallerRole::NotAuthorized,
        )
    }

    fn unit_request() -> UnitRequest {
        UnitRequest {
            vm_id: VmId::new("vm-a"),
            role_id: RoleId::new("virtiofsd"),
            resource_ref: Some(ResourceRef::parse("Process/worker").unwrap()),
            resource_uid: Some(
                ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            ),
            role: RunnerRole::Virtiofsd,
            bundle_runner_intent_ref: BundleOpId::new("runner:vm:vm-a:role:virtiofsd"),
            bundle_content_identity: "bundle".to_owned(),
            provider_identity: [2; 32],
            template_identity: [3; 32],
            generation: 1,
            domain: UnitDomain::System,
            execution_ref: Some(ResourceRef::parse("Host/host-system").unwrap()),
            user_ref: None,
            guest_execution: None,
            sandbox_plan: None,
            tracing_span_id: None,
        }
    }

    fn wire_identity(seed: u32) -> UnitIdentity {
        UnitIdentity {
            invocation_id: {
                let mut invocation_id = [0; 16];
                invocation_id[..4].copy_from_slice(&(seed + 1).to_le_bytes());
                invocation_id
            },
            cgroup_identity: [1; 32],
            main_pid: seed + 1,
            start_time_ticks: u64::from(seed) + 1,
            provider_identity: [2; 32],
            template_identity: [3; 32],
            generation: 1,
            bundle_content_identity: "bundle".to_owned(),
            guest_execution: None,
        }
    }

    fn launch_intent() -> BrokerLaunchIntent {
        BrokerLaunchIntent {
            vm_id: VmId::new("vm-a"),
            zone: "work".to_owned(),
            zone_uid: None,
            owner_ref: None,
            owner_uid: None,
            runtime_scope: None,
            typed_identity: true,
            provider_ref: ResourceRef::parse("Provider/system-systemd").unwrap(),
            execution_ref: ResourceRef::parse("Host/host-system").unwrap(),
            domain: ExecutionDomain::System,
            user_ref: None,
            role_id: RoleId::new("virtiofsd"),
            role: RunnerRole::Virtiofsd,
            bundle_runner_intent_ref: BundleOpId::new("runner:vm:vm-a:role:virtiofsd"),
            provider_identity: [2; 32],
            template_identity: [3; 32],
            generation: 1,
            resource_ref: ResourceRef::parse("Process/worker").unwrap(),
            resource_uid: ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap(),
            bundle_content_identity: "bundle".to_owned(),
            sandbox_plan: None,
            activation_input: None,
            guest_execution: None,
            accepts_launch_args: false,
            multi_instance: false,
        }
    }

    /// The owner's caller-role fence: an unauthenticated caller is refused
    /// before any socket is dialed, and the refusal projects onto the
    /// transient `LaunchFailed` the driver retries.
    #[test]
    fn broker_owner_refuses_envelope_calls_when_the_caller_is_not_authorized() {
        let owner = broker_owner();
        let refusal = owner
            .envelope_call("StartSystemdUnit", "work", serde_json::json!({}))
            .expect_err("not-authorized caller");
        assert!(
            matches!(&refusal, KernelInvokeError::Refused { code, .. } if code == "not-authorized"),
            "refusal names the caller fence: {refusal:?}"
        );
        assert_eq!(
            response_error(&refusal),
            ProcessEffectError::LaunchFailed
        );
    }

    /// The unit-request ledger: a remembered request is returned exactly
    /// once, a miss is `IdentityChanged` (the unit's identity is no longer
    /// the one the daemon launched), and consumption removes the entry.
    #[test]
    fn broker_owner_ledger_remembers_and_consumes_unit_requests() {
        let owner = broker_owner();
        let identity = identity(7);
        let unit = unit_request();

        assert_eq!(
            owner.request_for(&identity),
            Err(ProcessEffectError::IdentityChanged),
            "an unknown identity is a ledger miss"
        );
        owner
            .remember(&identity, unit.clone(), "work".to_owned())
            .expect("remembered");
        assert_eq!(
            owner.request_for(&identity).expect("lookup"),
            (unit.clone(), "work".to_owned())
        );
        assert_eq!(
            owner.take_request(&identity).expect("taken"),
            (unit, "work".to_owned())
        );
        assert_eq!(
            owner.request_for(&identity),
            Err(ProcessEffectError::IdentityChanged),
            "consumed entries are gone"
        );
        assert_eq!(
            owner.take_request(&identity),
            Err(ProcessEffectError::IdentityChanged),
            "a second take is a miss"
        );
    }

    /// The identity fence: a wire identity whose binding fields disagree
    /// with the resolved intent - or whose main pid is zero - is a drifted
    /// runtime tuple and refuses as `IdentityChanged`, never adopted.
    #[test]
    fn broker_owner_identity_fence_refuses_drifted_wire_identities() {
        let owner = broker_owner();
        let intent = launch_intent();
        assert!(
            owner.identity(&wire_identity(7), &intent).is_ok(),
            "a matching wire identity binds"
        );

        let mut zero_pid = wire_identity(7);
        zero_pid.main_pid = 0;
        assert_eq!(
            owner.identity(&zero_pid, &intent),
            Err(ProcessEffectError::IdentityChanged),
            "a zero main pid is never a launchable identity"
        );

        let mut drifted = wire_identity(7);
        drifted.provider_identity = [9; 32];
        assert_eq!(
            owner.identity(&drifted, &intent),
            Err(ProcessEffectError::IdentityChanged),
            "a provider digest that disagrees with the intent refuses"
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

    fn probe(
        &self,
        request: ProcessRequest,
    ) -> Result<Option<BackendObservation>, ProcessEffectError> {
        let Some(identity) = self.owner.probe(request)? else {
            return Ok(None);
        };
        Ok(Some(identity.observation()))
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

    fn wait(
        &self,
        handle: &Self::Handle,
        timeout: std::time::Duration,
    ) -> Result<(), ProcessEffectError> {
        self.owner.wait(handle, timeout)
    }

    fn stop(
        &self,
        handle: &Self::Handle,
        class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError> {
        self.owner.stop(handle, class)
    }

    fn finalize(&self, handle: &Self::Handle) -> Result<(), ProcessEffectError> {
        self.owner.finalize(handle)
    }
}

/// Broker-backed systemd effect owner used by the daemon's fixed supervisor.
///
/// The owner translates only typed systemd lifecycle requests to the broker's
/// envelope carrier: each call names the committed process-systemd family row
/// and carries the typed unit request as the envelope payload, exactly as the
/// family's forward seam serves it (U15). Unit names, manager connections,
/// cgroup paths, and process descriptors remain on the broker side; the
/// returned handle is retained here solely for exact stop authority.
pub struct BrokerSystemdEffectOwner {
    resolver: BundleBackedLaunchResolver,
    socket_path: std::path::PathBuf,
    io_timeout: std::time::Duration,
    caller_role: BrokerCallerRole,
    /// Unit-request ledger keyed by identity digest, each entry carrying the
    /// authoritative Zone label the launch ran under (the envelope's per-Zone
    /// resolution needs it for reopen/stop legs).
    requests: Mutex<BTreeMap<ProcessIdentityDigest, (UnitRequest, String)>>,
}

impl BrokerSystemdEffectOwner {
    /// Build an owner bound to one fixed caller identity.
    pub fn with_socket_and_role(
        resolver: BundleBackedLaunchResolver,
        socket_path: impl Into<std::path::PathBuf>,
        io_timeout: std::time::Duration,
        caller_role: BrokerCallerRole,
    ) -> Self {
        Self {
            resolver,
            socket_path: socket_path.into(),
            io_timeout,
            caller_role,
            requests: Mutex::new(BTreeMap::new()),
        }
    }

    /// Invoke one committed process-systemd family row over the broker's
    /// origination socket as an envelope frame.
    ///
    /// The family rows are served by the declaring provider (the daemon's
    /// process-systemd handlers) through the forward seam, so the call names
    /// the committed row exactly as the catalog declares it and carries the
    /// canonical typed payload the row's schema admits. The rows admit no
    /// request descriptors (`max_fds: 0`), so no fds are attached and every
    /// leg is a root call.
    fn envelope_call(
        &self,
        operation: &str,
        zone: &str,
        payload: serde_json::Value,
    ) -> Result<KernelReply, KernelInvokeError> {
        if matches!(self.caller_role, BrokerCallerRole::NotAuthorized) {
            warn!(
                provider = "supervisor",
                "broker request refused: caller not authorized for the broker profile"
            );
            return Err(KernelInvokeError::Refused {
                code: "not-authorized".to_owned(),
                detail: None,
            });
        }
        envelope_invoke_kernel(
            &self.socket_path,
            self.io_timeout,
            self.caller_role.clone(),
            KernelInvocation {
                operation,
                zone,
                payload,
                fds: &[],
                chain_root_invocation_id: None,
                chain_identities: None,
            },
        )
    }

    fn intent(
        &self,
        request: &ProcessRequest,
    ) -> Result<(BrokerLaunchIntent, UnitRequest), ProcessEffectError> {
        let intent = self.resolver.resolve(request)?;
        let domain = match request.ticket().domain() {
            ExecutionDomain::System => UnitDomain::System,
            ExecutionDomain::User => UnitDomain::User,
        };
        let unit = UnitRequest {
            execution_ref: Some(intent.execution_ref.clone()),
            user_ref: intent.user_ref.clone(),
            vm_id: intent.vm_id.clone(),
            role_id: intent.role_id.clone(),
            resource_ref: Some(intent.resource_ref.clone()),
            resource_uid: Some(intent.resource_uid.clone()),
            role: intent.role,
            bundle_runner_intent_ref: intent.bundle_runner_intent_ref.clone(),
            bundle_content_identity: intent.bundle_content_identity.clone(),
            provider_identity: intent.provider_identity,
            template_identity: intent.template_identity,
            generation: intent.generation,
            domain,
            guest_execution: intent.guest_execution.clone(),
            sandbox_plan: intent.sandbox_plan.clone(),
            tracing_span_id: None,
        };
        Ok((intent, unit))
    }

    // Sync by construction: unit-request ledger behind the sync wire+trait
// surface on dedicated blocking workers; critical sections short.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn remember(
        &self,
        identity: &SystemdInvocationIdentity,
        request: UnitRequest,
        zone: String,
    ) -> Result<(), ProcessEffectError> {
        self.requests
            .lock()
            .map_err(|_| {
                error!(
                    provider = "supervisor",
                    "systemd unit request ledger lock poisoned; lookup failed"
                );
                ProcessEffectError::ObserveFailed
            })?
            .insert(identity.digest(), (request, zone));
        Ok(())
    }

    // Sync by construction: unit-request ledger behind the sync wire+trait
// surface (see `remember`).
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn request_for(
        &self,
        identity: &SystemdInvocationIdentity,
    ) -> Result<(UnitRequest, String), ProcessEffectError> {
        self.requests
            .lock()
            .map_err(|_| {
                error!(
                    provider = "supervisor",
                    "systemd unit request ledger lock poisoned; lookup failed"
                );
                ProcessEffectError::ObserveFailed
            })?
            .get(&identity.digest())
            .cloned()
            .ok_or(ProcessEffectError::IdentityChanged)
    }

    // Sync by construction: unit-request ledger behind the sync wire+trait
// surface (see `remember`).
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn take_request(
        &self,
        identity: &SystemdInvocationIdentity,
    ) -> Result<(UnitRequest, String), ProcessEffectError> {
        self.requests
            .lock()
            .map_err(|_| {
                error!(
                    provider = "supervisor",
                    "systemd unit request ledger lock poisoned; take failed"
                );
                ProcessEffectError::StopFailed
            })?
            .remove(&identity.digest())
            .ok_or(ProcessEffectError::IdentityChanged)
    }

    fn identity(
        &self,
        wire: &UnitIdentity,
        intent: &BrokerLaunchIntent,
    ) -> Result<SystemdInvocationIdentity, ProcessEffectError> {
        if wire.provider_identity != intent.provider_identity
            || wire.template_identity != intent.template_identity
            || wire.generation != intent.generation
            || wire.bundle_content_identity != intent.bundle_content_identity
            || wire.guest_execution != intent.guest_execution
            || wire.main_pid == 0
        {
            warn!(
                provider = "supervisor",
                "systemd unit identity fields mismatch the resolved intent"
            );
            return Err(ProcessEffectError::IdentityChanged);
        }
        SystemdInvocationIdentity::new(wire)
    }

    /// Query one unit identity, optionally retaining the unit request so a
    /// later descriptor reopen can bind it. An adopt probe must not retain
    /// adoption state.
    fn observed_identity(
        &self,
        request: ProcessRequest,
        retain: bool,
    ) -> Result<Option<SystemdInvocationIdentity>, ProcessEffectError> {
        let (intent, unit) = self.intent(&request)?;
        let payload = serde_json::to_value(&unit).map_err(|_| ProcessEffectError::ObserveFailed)?;
        let reply = self
            .envelope_call("ObserveSystemdUnit", &intent.zone, payload)
            .map_err(|error| response_error(&error))?;
        let response: ObserveUnitResponse = serde_json::from_value(
            reply
                .response
                .result
                .clone()
                .ok_or(ProcessEffectError::ObserveFailed)?,
        )
        .map_err(|_| ProcessEffectError::ObserveFailed)?;
        if response.vm_id != unit.vm_id || response.role_id != unit.role_id {
            warn!(
                provider = "supervisor",
                "systemd unit observe response scope mismatch"
            );
            return Err(ProcessEffectError::IdentityChanged);
        }
        let Some(wire) = response.identity else {
            return Ok(None);
        };
        let identity = self.identity(&wire, &intent)?;
        if retain {
            self.remember(&identity, unit, intent.zone)?;
        }
        Ok(Some(identity))
    }
}

impl std::fmt::Debug for BrokerSystemdEffectOwner {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BrokerSystemdEffectOwner(<redacted>)")
    }
}

/// Core-local systemd pidfd handle.
pub struct BrokerSystemdPidfdHandle {
    pidfd: OwnedFd,
    request: UnitRequest,
    identity: SystemdInvocationIdentity,
    /// The Zone the unit's launch ran under (the stop leg's envelope
    /// carrier names it).
    zone: String,
}

impl std::fmt::Debug for BrokerSystemdPidfdHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("BrokerSystemdPidfdHandle(<redacted>)")
    }
}

impl SystemdEffectOwner for BrokerSystemdEffectOwner {
    type Handle = BrokerSystemdPidfdHandle;

    fn launch(
        &self,
        request: ProcessRequest,
    ) -> Result<SystemdEffectLaunch<Self::Handle>, ProcessEffectError> {
        let (intent, unit) = self.intent(&request)?;
        let payload = serde_json::to_value(&unit).map_err(|_| ProcessEffectError::LaunchFailed)?;
        let mut reply = self
            .envelope_call("StartSystemdUnit", &intent.zone, payload)
            .map_err(|error| response_error(&error))?;
        let response: StartTransientUnitResponse = serde_json::from_value(
            reply
                .response
                .result
                .clone()
                .ok_or(ProcessEffectError::LaunchFailed)?,
        )
        .map_err(|_| ProcessEffectError::LaunchFailed)?;
        if response.vm_id != unit.vm_id || response.role_id != unit.role_id {
            warn!(
                provider = "supervisor",
                "systemd unit start response scope mismatch"
            );
            return Err(ProcessEffectError::IdentityChanged);
        }
        let identity = self.identity(&response.identity, &intent)?;
        let pidfd = crate::broker::reply_take_fd(&mut reply, response.pidfd_index)?;
        self.remember(&identity, unit.clone(), intent.zone.clone())?;
        Ok(SystemdEffectLaunch::new(
            identity.clone(),
            BrokerSystemdPidfdHandle {
                pidfd,
                request: unit,
                identity,
                zone: intent.zone,
            },
        ))
    }

    fn observe(
        &self,
        request: ProcessRequest,
    ) -> Result<Option<SystemdInvocationIdentity>, ProcessEffectError> {
        self.observed_identity(request, true)
    }

    fn probe(
        &self,
        request: ProcessRequest,
    ) -> Result<Option<SystemdInvocationIdentity>, ProcessEffectError> {
        self.observed_identity(request, false)
    }

    fn reopen(
        &self,
        expected: &SystemdInvocationIdentity,
    ) -> Result<SystemdEffectLaunch<Self::Handle>, ProcessEffectError> {
        let (unit, zone) = self.request_for(expected)?;
        let payload =
            serde_json::to_value(OpenUnitPidfdRequest {
                unit: unit.clone(),
                expected: expected.wire_identity(),
            })
            .map_err(|_| ProcessEffectError::IdentityChanged)?;
        let mut reply = self
            .envelope_call("OpenSystemdUnitPidfd", &zone, payload)
            .map_err(|error| response_error(&error))?;
        let response: OpenUnitPidfdResponse = serde_json::from_value(
            reply
                .response
                .result
                .clone()
                .ok_or(ProcessEffectError::IdentityChanged)?,
        )
        .map_err(|_| ProcessEffectError::IdentityChanged)?;
        let actual = SystemdInvocationIdentity::new(&response.identity)?;
        if actual != *expected || response.vm_id != unit.vm_id || response.role_id != unit.role_id {
            warn!(
                provider = "supervisor",
                "systemd unit pidfd reopen identity mismatch"
            );
            return Err(ProcessEffectError::IdentityChanged);
        }
        let pidfd = crate::broker::reply_take_fd(&mut reply, response.pidfd_index)?;
        Ok(SystemdEffectLaunch::new(
            actual.clone(),
            BrokerSystemdPidfdHandle {
                pidfd,
                request: unit,
                identity: actual,
                zone,
            },
        ))
    }

    fn wait(
        &self,
        handle: &Self::Handle,
        timeout: std::time::Duration,
    ) -> Result<(), ProcessEffectError> {
        wait_pidfd_observer(&handle.pidfd, timeout)
    }

    fn stop(
        &self,
        handle: &Self::Handle,
        class: ProcessStopClass,
    ) -> Result<(), ProcessEffectError> {
        let payload = serde_json::to_value(StopUnitRequest {
            unit: handle.request.clone(),
            expected: handle.identity.wire_identity(),
            class: match class {
                ProcessStopClass::Drain => UnitStopClass::Drain,
                ProcessStopClass::Terminate => UnitStopClass::Terminate,
            },
        })
        .map_err(|_| ProcessEffectError::StopFailed)?;
        let reply = self
            .envelope_call("StopSystemdUnit", &handle.zone, payload)
            .map_err(|error| response_error(&error))?;
        let response: StopUnitResponse = serde_json::from_value(
            reply
                .response
                .result
                .clone()
                .ok_or(ProcessEffectError::StopFailed)?,
        )
        .map_err(|_| ProcessEffectError::StopFailed)?;
        if !response.stopped {
            warn!(
                provider = "supervisor",
                "service manager refused the unit stop"
            );
            return Err(ProcessEffectError::StopFailed);
        }
        if class == ProcessStopClass::Terminate {
            let _ = self.take_request(&handle.identity)?;
        }
        Ok(())
    }

    fn finalize(&self, handle: &Self::Handle) -> Result<(), ProcessEffectError> {
        let _ = self.take_request(&handle.identity)?;
        Ok(())
    }
}

fn response_error(error: &KernelInvokeError) -> ProcessEffectError {
    warn!(
        provider = "supervisor",
        error = ?error,
        "broker refused a systemd unit request"
    );
    ProcessEffectError::LaunchFailed
}
