use d2b_contracts::v3::{ResourceRef, ResourceUid};
use d2b_provider_device_tpm::{
    TpmResourceController, TpmResourceEffectError, TpmResourceEffectPort, TpmResourceOutcome,
    build_tpm_state_volume_spec,
};

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
