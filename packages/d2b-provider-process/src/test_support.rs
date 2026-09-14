//! Test-support recording double for the [`ProcessDriverEffects`] port.
//!
//! The canonical scripted fake effect port for the Process family: records
//! every call with the exact ticket inputs the driver derived (KTD7) and
//! replays a scripted adoption/liveness/launch sequence. Gated behind the
//! `test-support` Cargo feature (available automatically under `cargo test`),
//! so production consumers never pull it in. The plane tests in `d2bd` reach
//! it through the same public surface.

use std::collections::VecDeque;
use std::time::Duration;

use d2b_contracts_resource::v3::process::{EphemeralProcessSpec, ExecutionSpec, ProcessSpec};
use d2b_contracts_resource::v3::{ResourceRef, ResourceUid, ZoneId};
use d2b_process_conformance::{AdoptionCandidate, ProcessIdentityDigest};
use d2b_resource_runtime::context::ResourceContext;
use parking_lot::Mutex;

use crate::driver::device_worker_family;
use crate::effects::{ProcessDriverEffects, ProviderAdoption, ProviderLiveness};
use crate::identity::{ProcessFamilySpec, ProcessResourceIdentity};
use crate::worker_launch::DeviceWorkerLaunch;

/// One recorded launch with the ticket inputs the driver derived.
#[derive(Clone, Debug)]
pub struct RecordedLaunch {
    /// Which family arm launched: `Process` or `EphemeralProcess`.
    pub kind: &'static str,
    /// The canonical resource reference the launch was recorded against.
    pub resource_ref: String,
    /// The recorded resource uid.
    pub resource_uid: String,
    /// The recorded resource generation.
    pub generation: u64,
    /// The zone the launch was recorded against.
    pub zone: ZoneId,
    /// The zone-authority uid the driver derived, if any.
    pub zone_uid: Option<ResourceUid>,
    /// The zone policy revision the driver derived, if any.
    pub policy_revision: Option<u64>,
    /// The canonical provider reference the launch was recorded against.
    pub provider_ref: String,
    /// The launch template from the spec.
    pub template: String,
    /// The canonical execution reference the launch was recorded against.
    pub execution_ref: String,
    /// The one-shot launch budget (`startDeadline`) the driver derived.
    pub start_deadline_ms: Option<u64>,
}

/// One recorded stop with the timeouts the driver derived.
#[derive(Clone, Debug)]
pub struct RecordedStop {
    /// Which family arm stopped: `Process` or `EphemeralProcess`.
    pub kind: &'static str,
    /// The term timeout the stop was issued with.
    pub term_timeout: Duration,
    /// The kill timeout the stop was issued with.
    pub kill_timeout: Duration,
}

/// Configuration for [`FakeEffects`]: the scripted adoption/liveness/launch
/// outcomes the double replays in order.
#[derive(Clone)]
pub struct FakeEffectsConfig {
    /// Scripted adoption results; the last one repeats once exhausted.
    pub adoption: VecDeque<ProviderAdoption>,
    /// When set, one-shot adoption refuses with this provider error.
    pub adopt_error: Option<String>,
    /// Scripted liveness results; the last one repeats once exhausted
    /// (default `Alive`).
    pub liveness: VecDeque<ProviderLiveness>,
    /// The scripted launch/launch-ephemeral result.
    pub launch: Result<ProcessIdentityDigest, String>,
    /// Whether the fake reports a live retained identity.
    pub active: bool,
    /// When set, the declared Device-worker parameter derivation refuses
    /// with this named code (the launch-only refusal shape).
    pub device_worker_launch: Option<&'static str>,
}

impl Default for FakeEffectsConfig {
    fn default() -> Self {
        Self {
            adoption: VecDeque::from([ProviderAdoption::Absent]),
            adopt_error: None,
            liveness: VecDeque::new(),
            launch: Ok(ProcessIdentityDigest::from_bytes([0x51; 32])),
            active: true,
            device_worker_launch: None,
        }
    }
}

/// Scripted [`ProcessDriverEffects`] double: records every call with the
/// exact ticket inputs the driver derived (KTD7) and replays a scripted
/// adoption sequence.
pub struct FakeEffects {
    config: Mutex<FakeEffectsConfig>,
    calls: Mutex<Vec<&'static str>>,
    launches: Mutex<Vec<RecordedLaunch>>,
    stops: Mutex<Vec<RecordedStop>>,
    finalizes: Mutex<usize>,
}

impl FakeEffects {
    /// Build a double from the given scripted configuration.
    pub fn new(config: FakeEffectsConfig) -> Self {
        Self {
            config: Mutex::new(config),
            calls: Mutex::new(Vec::new()),
            launches: Mutex::new(Vec::new()),
            stops: Mutex::new(Vec::new()),
            finalizes: Mutex::new(0),
        }
    }

    /// Queue one adoption result; the queue empties toward the default.
    pub fn push_adoption(&self, adoption: ProviderAdoption) {
        self.config.lock().adoption.push_back(adoption);
    }

    /// Queue one liveness result; the queue empties toward `Alive`.
    pub fn push_liveness(&self, liveness: ProviderLiveness) {
        self.config.lock().liveness.push_back(liveness);
    }

    /// Script the launch/launch-ephemeral result.
    pub fn set_launch(&self, result: Result<ProcessIdentityDigest, String>) {
        self.config.lock().launch = result;
    }

    /// Flip the retained-identity report (the Provider records one on a
    /// successful launch, so a row can be launched first and then read as
    /// live).
    pub fn set_active(&self, active: bool) {
        self.config.lock().active = active;
    }

    /// The recorded launch calls, in order.
    pub fn launch_calls(&self) -> Vec<RecordedLaunch> {
        self.launches.lock().clone()
    }

    /// The recorded stop calls, in order.
    pub fn stop_calls(&self) -> Vec<RecordedStop> {
        self.stops.lock().clone()
    }

    /// The recorded finalize count.
    pub fn finalize_calls(&self) -> usize {
        *self.finalizes.lock()
    }

    /// The recorded effect call order.
    pub fn call_order(&self) -> Vec<&'static str> {
        self.calls.lock().clone()
    }
}

/// Build a [`RecordedLaunch`] for one recorded launch of the given family
/// arm, from the identity and execution spec the driver derived.
pub fn recorded_launch(
    kind: &'static str,
    identity: &ProcessResourceIdentity,
    execution: &ExecutionSpec,
    start_deadline_ms: Option<u64>,
) -> RecordedLaunch {
    RecordedLaunch {
        kind,
        resource_ref: identity.resource_ref.to_canonical_string(),
        resource_uid: identity.resource_uid.as_str().to_owned(),
        generation: identity.resource_generation.get(),
        zone: identity.zone.clone(),
        zone_uid: identity.zone_uid.clone(),
        policy_revision: identity.policy_revision,
        provider_ref: identity.provider_ref.to_canonical_string(),
        template: execution.template().as_str().to_owned(),
        execution_ref: execution.execution_ref().to_canonical_string(),
        start_deadline_ms,
    }
}

#[async_trait::async_trait]
impl ProcessDriverEffects for FakeEffects {
    async fn launch(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &ProcessSpec,
        _timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String> {
        self.calls.lock().push("launch");
        self.launches.lock().push(recorded_launch(
            "Process",
            identity,
            spec.execution(),
            None,
        ));
        self.config.lock().launch.clone()
    }

    async fn launch_ephemeral(
        &self,
        identity: &ProcessResourceIdentity,
        spec: &EphemeralProcessSpec,
        timeout: Duration,
    ) -> Result<ProcessIdentityDigest, String> {
        self.calls.lock().push("launch-ephemeral");
        assert_eq!(
            timeout,
            Duration::from_millis(spec.start_deadline().as_millis()),
            "the one-shot launch budget is the spec's startDeadline",
        );
        self.launches.lock().push(recorded_launch(
            "EphemeralProcess",
            identity,
            spec.execution(),
            Some(spec.start_deadline().as_millis()),
        ));
        self.config.lock().launch.clone()
    }

    async fn adopt(
        &self,
        _identity: &ProcessResourceIdentity,
        _spec: &ProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.calls.lock().push("adopt");
        let mut config = self.config.lock();
        Ok(config.adoption.pop_front().unwrap_or(ProviderAdoption::Absent))
    }

    async fn probe(
        &self,
        _identity: &ProcessResourceIdentity,
        _spec: &ProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.calls.lock().push("probe");
        let mut config = self.config.lock();
        Ok(config.liveness.pop_front().unwrap_or(ProviderLiveness::Alive))
    }

    async fn adopt_ephemeral(
        &self,
        _identity: &ProcessResourceIdentity,
        _spec: &EphemeralProcessSpec,
    ) -> Result<ProviderAdoption, String> {
        self.calls.lock().push("adopt-ephemeral");
        let mut config = self.config.lock();
        if let Some(error) = config.adopt_error.clone() {
            return Err(error);
        }
        Ok(config.adoption.pop_front().unwrap_or(ProviderAdoption::Absent))
    }

    async fn probe_ephemeral(
        &self,
        _identity: &ProcessResourceIdentity,
        _spec: &EphemeralProcessSpec,
    ) -> Result<ProviderLiveness, String> {
        self.calls.lock().push("probe-ephemeral");
        let mut config = self.config.lock();
        Ok(config.liveness.pop_front().unwrap_or(ProviderLiveness::Alive))
    }

    async fn stop(
        &self,
        _identity: &ProcessResourceIdentity,
        _spec: &ProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.calls.lock().push("stop");
        self.stops.lock().push(RecordedStop {
            kind: "Process",
            term_timeout,
            kill_timeout,
        });
        Ok(true)
    }

    async fn stop_ephemeral(
        &self,
        _identity: &ProcessResourceIdentity,
        _spec: &EphemeralProcessSpec,
        term_timeout: Duration,
        kill_timeout: Duration,
    ) -> Result<bool, String> {
        self.calls.lock().push("stop-ephemeral");
        self.stops.lock().push(RecordedStop {
            kind: "EphemeralProcess",
            term_timeout,
            kill_timeout,
        });
        Ok(true)
    }

    async fn stop_stale(
        &self,
        _provider_ref: &ResourceRef,
        _candidate: &AdoptionCandidate,
    ) -> Result<(), String> {
        self.calls.lock().push("stop-stale");
        Ok(())
    }

    async fn device_worker_launch(
        &self,
        _ctx: &mut ResourceContext,
        _identity: &ProcessResourceIdentity,
        spec: &ProcessFamilySpec,
    ) -> Result<Option<DeviceWorkerLaunch>, &'static str> {
        let template = spec.execution().template().as_str();
        if device_worker_family(template).is_none() {
            return Ok(None);
        }
        self.calls.lock().push("device-worker-launch");
        match self.config.lock().device_worker_launch {
            Some(code) => Err(code),
            None => Ok(None),
        }
    }

    async fn finalize(&self, _identity: &ProcessResourceIdentity) -> Result<(), String> {
        self.calls.lock().push("finalize");
        *self.finalizes.lock() += 1;
        Ok(())
    }

    fn has_active(
        &self,
        _zone: &ZoneId,
        _zone_uid: Option<&ResourceUid>,
        _resource_ref: &ResourceRef,
    ) -> bool {
        self.config.lock().active
    }
}
