use d2b_provider_device_gpu::{
    DEFAULT_OBSERVE_INTERVAL_SECS, GpuBackingToken, GpuDeviceSelector, GpuPlatformToken,
    GpuProbeDisposition, GpuProbeError, GpuProbeResult, GpuProbeTracker,
};

#[test]
fn selector_and_interval_bounds_are_enforced() {
    assert!(GpuDeviceSelector::new("host-gpu", None::<String>).is_ok());
    assert!(GpuDeviceSelector::new("../gpu", None::<String>).is_err());
    assert_eq!(
        GpuProbeTracker::with_interval(9).unwrap_err(),
        GpuProbeError::ObserveIntervalOutOfRange
    );
    assert_eq!(
        GpuProbeTracker::new().interval_secs(),
        DEFAULT_OBSERVE_INTERVAL_SECS
    );
}

#[test]
fn probe_tracker_uses_three_strikes_and_resets_on_presence() {
    let mut tracker = GpuProbeTracker::new();
    assert_eq!(tracker.record_failure(), GpuProbeDisposition::Unknown);
    assert_eq!(tracker.record_failure(), GpuProbeDisposition::Unknown);
    assert_eq!(tracker.record_failure(), GpuProbeDisposition::Degraded);
    assert_eq!(tracker.record_success(), GpuProbeDisposition::Ready);
    assert_eq!(tracker.failures(), 0);
}

#[test]
fn probe_result_rejects_stale_identity() {
    assert_eq!(
        GpuProbeResult::present(
            GpuBackingToken::from_core([0; 32]),
            GpuPlatformToken::from_core([1; 32]),
            true,
        )
        .unwrap_err(),
        GpuProbeError::StaleIdentity
    );
}
