use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_provider_device_tpm::{
    TpmResourceController, TpmResourceEffectError, TpmResourceEffectPort, TpmResourceOutcome,
    build_tpm_state_volume_spec,
};
use d2b_provider_toolkit::testing::block_on;
use d2b_contracts_resource::v3::ZoneId;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

/// The Zone every fixture in this file places its Device in.
const ZONE: &str = "dev";

fn zone() -> ZoneId {
    ZoneId::parse(ZONE).expect("valid test zone")
}

#[test]
fn controller_uses_opaque_resource_effects_and_preserves_volume_on_finalize() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let spec = build_tpm_state_volume_spec(&device_ref, "dev", &execution).unwrap();
    assert_eq!(spec["source"]["settings"]["kind"], "local-path");
    assert!(spec.get("hostPath").is_none());
    assert_eq!(spec["layout"][0]["ownerRef"], "User/d2bd");
    assert_eq!(
        spec["layout"][0]["accessAcl"][0],
        serde_json::json!({
            "principal": { "ref": "User/d2b-dev-work-tpm-swtpm" },
            "permissions": "rwx"
        })
    );

    fn assert_port<P: TpmResourceEffectPort>() {}
    assert_port::<NoopEffects>();
    assert_eq!(TpmResourceOutcome::VolumeRetained.code(), "volume-retained");

    let mut controller = TpmResourceController::new(zone(), device, device_ref, execution).unwrap();
    let effects = NoopEffects;
    assert_eq!(
        block_on(controller.reconcile(&effects)).unwrap(),
        TpmResourceOutcome::Ready
    );
    assert_eq!(
        block_on(controller.finalize(&effects)).unwrap(),
        TpmResourceOutcome::VolumeRetained
    );
    assert!(!controller.finalizer_installed());
}

#[test]
fn repeated_reconcile_reuses_the_declared_children() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let effects = ScriptedEffects::default();
    let mut controller = TpmResourceController::new(zone(), device, device_ref, execution).unwrap();

    block_on(controller.reconcile(&effects)).unwrap();
    block_on(controller.reconcile(&effects)).unwrap();

    assert_eq!(
        effects.events.try_lock().unwrap().as_slice(),
        ["volume", "flush", "process", "endpoint", "endpoint"]
    );
}

#[test]
fn tpm_runner_contract_disables_legacy_scheduling() {
    let contract = d2b_provider_device_tpm::tpm_runner_contract();
    assert_eq!(contract.resource_type(), "Device");
    assert_eq!(contract.finalizer(), d2b_provider_device_tpm::DEVICE_TPM_FINALIZER);
    assert!(contract.watched_configuration_is_dependency());
    assert!((30..=60).contains(&contract.repair_interval_secs()));
}

#[test]
fn controller_rejects_non_host_execution_refs() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Zone/zone-a").unwrap();

    assert!(matches!(
        TpmResourceController::new(zone(), device, device_ref, execution),
        Err(d2b_provider_device_tpm::TpmResourceControllerError::Effect(
            TpmResourceEffectError::InvalidExecutionRef
        ))
    ));
}

#[test]
fn controller_finalize_before_reconcile_is_invalid() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller = TpmResourceController::new(zone(), device, device_ref, execution).unwrap();

    assert_eq!(
        block_on(controller.finalize(&NoopEffects)),
        Err(d2b_provider_device_tpm::TpmResourceControllerError::InvalidState)
    );
}

#[test]
fn controller_finalizes_the_swtpm_process_after_endpoint_watch_failure() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller = TpmResourceController::new(zone(), device, device_ref, execution).unwrap();
    let effects = ScriptedEffects {
        endpoint_fails: true,
        ..ScriptedEffects::default()
    };

    assert_eq!(
        block_on(controller.reconcile(&effects)),
        Err(d2b_provider_device_tpm::TpmResourceControllerError::Effect(
            TpmResourceEffectError::Transient
        ))
    );
    assert_eq!(
        block_on(controller.finalize(&effects)).unwrap(),
        TpmResourceOutcome::VolumeRetained
    );
    assert_eq!(effects.stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(effects.delete_calls.load(Ordering::SeqCst), 1);
    assert!(!controller.finalizer_installed());
}

#[test]
fn controller_retains_process_when_stop_fails_during_finalize() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller = TpmResourceController::new(zone(), device, device_ref, execution).unwrap();
    let effects = ScriptedEffects {
        stop_fails: AtomicBool::new(true),
        ..ScriptedEffects::default()
    };

    assert_eq!(
        block_on(controller.reconcile(&effects)).unwrap(),
        TpmResourceOutcome::Ready
    );
    assert_eq!(
        block_on(controller.finalize(&effects)),
        Err(d2b_provider_device_tpm::TpmResourceControllerError::Effect(
            TpmResourceEffectError::Transient
        ))
    );
    assert!(controller.finalizer_installed());
    assert_eq!(
        controller.phase(),
        d2b_provider_device_tpm::TpmResourcePhase::Degraded
    );
    assert_eq!(effects.stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(effects.delete_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn controller_does_not_repeat_stop_after_flush_delete_retry() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller = TpmResourceController::new(zone(), device, device_ref, execution).unwrap();
    let effects = ScriptedEffects {
        delete_failures: AtomicUsize::new(1),
        ..ScriptedEffects::default()
    };

    assert_eq!(
        block_on(controller.reconcile(&effects)).unwrap(),
        TpmResourceOutcome::Ready
    );
    assert_eq!(
        block_on(controller.finalize(&effects)),
        Err(d2b_provider_device_tpm::TpmResourceControllerError::Effect(
            TpmResourceEffectError::Transient
        ))
    );
    assert_eq!(
        block_on(controller.finalize(&effects)).unwrap(),
        TpmResourceOutcome::VolumeRetained
    );
    assert_eq!(effects.stop_calls.load(Ordering::SeqCst), 1);
    assert_eq!(effects.delete_calls.load(Ordering::SeqCst), 2);
    assert!(!controller.finalizer_installed());
}

#[test]
fn flush_failure_stops_the_long_lived_process_and_retains_state() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller = TpmResourceController::new(zone(), device, device_ref, execution).unwrap();
    let effects = ScriptedEffects {
        flush_fails: true,
        ..ScriptedEffects::default()
    };

    assert_eq!(
        block_on(controller.reconcile(&effects)),
        Err(d2b_provider_device_tpm::TpmResourceControllerError::Effect(
            TpmResourceEffectError::Transient
        ))
    );
    assert_eq!(
        effects.events.try_lock().unwrap().as_slice(),
        ["volume", "flush"]
    );
    assert_eq!(effects.stop_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn controller_flushes_before_starting_swtpm_and_waits_for_endpoint() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller = TpmResourceController::new(zone(), device, device_ref, execution).unwrap();
    let effects = ScriptedEffects::default();

    assert_eq!(
        block_on(controller.reconcile(&effects)).unwrap(),
        TpmResourceOutcome::Ready
    );
    assert_eq!(
        effects.events.try_lock().unwrap().as_slice(),
        ["volume", "flush", "process", "endpoint"]
    );
}

/// A TPM worker reaches only its own state view.
///
/// State access is an ordinary `VolumeBinding` pair, not a template grant:
/// the long-lived worker claims the `swtpm-process` view read-write, the
/// one-shot flush claims the `controller` view read-only, and neither request
/// names the other's view. The flush's claim is read-only because it runs
/// `swtpm_ioctl -i` and writes no NVRAM, so a regression that promoted it to
/// a writer would hand the one-shot helper the worker's whole state.
#[test]
fn a_tpm_worker_reaches_only_its_own_state_view() {
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let worker = ResourceRef::parse("Process/swtpm-work-tpm").unwrap();
    let flush = ResourceRef::parse("EphemeralProcess/swtpm-flush-work-tpm").unwrap();

    let worker_request =
        d2b_provider_device_tpm::build_tpm_worker_state_request(ZONE, &device_ref, &worker)
            .unwrap();
    let flush_request =
        d2b_provider_device_tpm::build_tpm_flush_state_request(ZONE, &device_ref, &flush).unwrap();

    assert_eq!(
        worker_request.view().as_str(),
        "swtpm-process",
        "the worker claims the view it writes"
    );
    assert_eq!(
        worker_request.requested_rights(),
        d2b_contracts_resource::v3::RequestedRights::Mutate,
        "the worker writes NVRAM, so it is the writer on its own view"
    );
    assert_eq!(
        flush_request.view().as_str(),
        "controller",
        "the flush claims the control view, not the worker's"
    );
    assert_eq!(
        flush_request.requested_rights(),
        d2b_contracts_resource::v3::RequestedRights::Observe,
        "the one-shot flush changes no NVRAM and is read-only"
    );
    assert_ne!(
        worker_request.view(),
        flush_request.view(),
        "the two consumers must not share one state claim"
    );
    assert_ne!(
        worker_request.slot(),
        flush_request.slot(),
        "each consumer's claim occupies its own stable slot"
    );
    assert_eq!(
        worker_request.consumer_ref().to_canonical_string(),
        "Process/swtpm-work-tpm"
    );
    assert_eq!(
        flush_request.consumer_ref().to_canonical_string(),
        "EphemeralProcess/swtpm-flush-work-tpm"
    );
    assert_eq!(
        worker_request.source_ref(),
        flush_request.source_ref(),
        "both claims name the same state Volume"
    );
    assert_eq!(
        worker_request.presentation().destination(),
        Some("/state"),
        "the admitted destination is the one the declared mount uses"
    );
}

/// A restarted controller re-admits the same state, it does not mint a new one.
///
/// The state Volume name embeds the owning Device's durable uid, and both
/// claims are derived from that same identity, so a controller rebuilt after
/// a daemon restart observes byte-identical relationships. If the state
/// identity were re-minted per controller, the second Device's NVRAM and
/// tamper marker would land in a fresh directory and the retained state would
/// silently stop existing.
#[test]
fn a_restarted_controller_keeps_the_same_state_identity() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let effects = ScriptedEffects::default();

    let mut before = TpmResourceController::new(zone(), device.clone(), device_ref.clone(), execution.clone())
        .unwrap();
    block_on(before.reconcile(&effects)).unwrap();
    let first = before.state_identity().clone();
    let first_fingerprint = first.fingerprint();
    drop(before);

    let mut after =
        TpmResourceController::new(zone(), device, device_ref, execution).unwrap();
    block_on(after.reconcile(&effects)).unwrap();
    let second = after.state_identity();

    assert_eq!(
        second.volume_ref(),
        first.volume_ref(),
        "the retained state Volume is the same row after a restart"
    );
    assert_eq!(
        second.worker_request(),
        first.worker_request(),
        "the worker's state claim survives a restart unchanged"
    );
    assert_eq!(
        second.flush_request(),
        first.flush_request(),
        "the flush's state claim survives a restart unchanged"
    );
    assert_eq!(
        second.fingerprint(),
        first_fingerprint,
        "the durable digest of the two claims is stable across a restart"
    );
    assert!(
        second
            .volume_ref()
            .to_canonical_string()
            .starts_with("Volume/device-"),
        "the state row is still this Device's own scoped child"
    );
}

/// A state Volume that is not this Device's own derived row is refused.
///
/// A framework that resolves any other Volume as this Device's state is not
/// describing this Device's NVRAM. Adopting it would move the persistent
/// state onto a row this Device never claimed, so the reconcile fails closed
/// instead of reporting Ready.
#[test]
fn a_foreign_state_volume_is_refused_as_a_state_integrity_failure() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let device_ref = ResourceRef::parse("Device/work-tpm").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller =
        TpmResourceController::new(zone(), device, device_ref, execution).unwrap();
    let effects = ForeignStateEffects;

    assert_eq!(
        block_on(controller.reconcile(&effects)),
        Err(d2b_provider_device_tpm::TpmResourceControllerError::Effect(
            TpmResourceEffectError::StateIntegrity
        ))
    );
    assert_eq!(
        controller.phase(),
        d2b_provider_device_tpm::TpmResourcePhase::Failed,
        "a foreign state row is a terminal failure, not a degraded retry"
    );
}

/// A port that resolves a state Volume belonging to no Device.
struct ForeignStateEffects;

#[allow(clippy::manual_async_fn)]
impl TpmResourceEffectPort for ForeignStateEffects {
    fn ensure_state_volume(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send {
        async { Ok(ResourceRef::parse("Volume/someone-elses-state").unwrap()) }
    }

    fn request_swtpm_process(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send {
        async { unreachable!("a refused state volume never reaches the worker request") }
    }

    fn request_flush_process(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send {
        async { unreachable!("a refused state volume never reaches the flush request") }
    }

    fn stop_swtpm_process(
        &self,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<(), TpmResourceEffectError>> + Send {
        async { Ok(()) }
    }

    fn delete_flush_process(
        &self,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<(), TpmResourceEffectError>> + Send {
        async { Ok(()) }
    }

    fn watch_tpm_endpoint(
        &self,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send {
        async { unreachable!("a refused state volume never reaches the endpoint") }
    }
}

struct NoopEffects;

#[derive(Default)]
struct ScriptedEffects {
    endpoint_fails: bool,
    flush_fails: bool,
    delete_failures: AtomicUsize,
    stop_calls: AtomicUsize,
    delete_calls: AtomicUsize,
    stop_fails: AtomicBool,
    events: Mutex<Vec<&'static str>>,
}

impl TpmResourceEffectPort for ScriptedEffects {
    async fn ensure_state_volume(
        &self,
        _: &ResourceUid,
        device_ref: &ResourceRef,
        _: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        assert_eq!(device_ref.to_canonical_string(), "Device/work-tpm");
        self.events.try_lock().unwrap().push("volume");
        Ok(d2b_provider_device_tpm::tpm_state_volume_ref(ZONE, device_ref).unwrap())
    }

    async fn request_swtpm_process(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
        _: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        self.events.try_lock().unwrap().push("process");
        Ok(ResourceRef::parse("Process/device-swtpm").unwrap())
    }

    async fn request_flush_process(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        self.events.try_lock().unwrap().push("flush");
        if self.flush_fails {
            Err(TpmResourceEffectError::Transient)
        } else {
            Ok(ResourceRef::parse("EphemeralProcess/device-flush").unwrap())
        }
    }

    fn stop_swtpm_process(
        &self,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<(), TpmResourceEffectError>> + Send {
        self.stop_calls.fetch_add(1, Ordering::SeqCst);
        let fails = self.stop_fails.load(Ordering::SeqCst);
        async move {
            if fails {
                Err(TpmResourceEffectError::Transient)
            } else {
                Ok(())
            }
        }
    }

    fn delete_flush_process(
        &self,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<(), TpmResourceEffectError>> + Send {
        self.delete_calls.fetch_add(1, Ordering::SeqCst);
        let should_fail = self
            .delete_failures
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |remaining| {
                if remaining > 0 {
                    Some(remaining - 1)
                } else {
                    None
                }
            })
            .is_ok();
        async move {
            if should_fail {
                Err(TpmResourceEffectError::Transient)
            } else {
                Ok(())
            }
        }
    }

    fn watch_tpm_endpoint(
        &self,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send {
        let fails = self.endpoint_fails;
        self.events.try_lock().unwrap().push("endpoint");
        async move {
            if fails {
                Err(TpmResourceEffectError::Transient)
            } else {
                Ok(ResourceRef::parse("Endpoint/device-tpm").unwrap())
            }
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl TpmResourceEffectPort for NoopEffects {
    fn ensure_state_volume(
        &self,
        _: &ResourceUid,
        device_ref: &ResourceRef,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send {
        let state = d2b_provider_device_tpm::tpm_state_volume_ref(ZONE, device_ref)
            .expect("the derived state volume");
        async move { Ok(state) }
    }

    fn request_swtpm_process(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send {
        async { Ok(ResourceRef::parse("Process/device-swtpm").unwrap()) }
    }

    fn request_flush_process(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send {
        async { Ok(ResourceRef::parse("EphemeralProcess/device-flush").unwrap()) }
    }

    fn stop_swtpm_process(
        &self,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<(), TpmResourceEffectError>> + Send {
        async { Ok(()) }
    }

    fn delete_flush_process(
        &self,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<(), TpmResourceEffectError>> + Send {
        async { Ok(()) }
    }

    fn watch_tpm_endpoint(
        &self,
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send {
        async { Ok(ResourceRef::parse("Endpoint/device-tpm").unwrap()) }
    }
}
