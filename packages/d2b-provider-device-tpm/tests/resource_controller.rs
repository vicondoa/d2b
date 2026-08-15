use d2b_contracts::v3::{ResourceRef, ResourceUid};
use d2b_provider_device_tpm::{
    TpmResourceController, TpmResourceEffectError, TpmResourceEffectPort, TpmResourceOutcome,
    build_tpm_state_volume_spec,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[test]
fn controller_uses_opaque_resource_effects_and_preserves_volume_on_finalize() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let spec = build_tpm_state_volume_spec(&device, &execution).unwrap();
    assert_eq!(spec["source"]["settings"]["kind"], "local-path");
    assert!(spec.get("hostPath").is_none());

    fn assert_port<P: TpmResourceEffectPort>() {}
    assert_port::<NoopEffects>();
    assert_eq!(TpmResourceOutcome::VolumeRetained.code(), "volume-retained");

    let mut controller = TpmResourceController::new(device, execution).unwrap();
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
fn controller_rejects_non_host_execution_refs() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let execution = ResourceRef::parse("Zone/zone-a").unwrap();

    assert!(matches!(
        TpmResourceController::new(device, execution),
        Err(d2b_provider_device_tpm::TpmResourceControllerError::Effect(
            TpmResourceEffectError::InvalidExecutionRef
        ))
    ));
}

#[test]
fn controller_finalize_before_reconcile_is_invalid() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller = TpmResourceController::new(device, execution).unwrap();

    assert_eq!(
        block_on(controller.finalize(&NoopEffects)),
        Err(d2b_provider_device_tpm::TpmResourceControllerError::InvalidState)
    );
}

#[test]
fn controller_finalizes_children_after_endpoint_watch_failure() {
    let device = ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").unwrap();
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller = TpmResourceController::new(device, execution).unwrap();
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
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller = TpmResourceController::new(device, execution).unwrap();
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
    let execution = ResourceRef::parse("Host/host-system").unwrap();
    let mut controller = TpmResourceController::new(device, execution).unwrap();
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

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};
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

struct NoopEffects;

#[derive(Default)]
struct ScriptedEffects {
    endpoint_fails: bool,
    delete_failures: AtomicUsize,
    stop_calls: AtomicUsize,
    delete_calls: AtomicUsize,
    stop_fails: AtomicBool,
}

impl TpmResourceEffectPort for ScriptedEffects {
    async fn ensure_state_volume(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        Ok(ResourceRef::parse("Volume/device-state").unwrap())
    }

    async fn request_swtpm_process(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
        _: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        Ok(ResourceRef::parse("Process/device-swtpm").unwrap())
    }

    async fn request_flush_process(
        &self,
        _: &ResourceUid,
        _: &ResourceRef,
        _: &ResourceRef,
    ) -> Result<ResourceRef, TpmResourceEffectError> {
        Ok(ResourceRef::parse("EphemeralProcess/device-flush").unwrap())
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
        _: &ResourceRef,
    ) -> impl std::future::Future<Output = Result<ResourceRef, TpmResourceEffectError>> + Send {
        async { Ok(ResourceRef::parse("Volume/device-state").unwrap()) }
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
