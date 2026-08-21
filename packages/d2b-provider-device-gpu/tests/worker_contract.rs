use d2b_contracts_resource::v3::ResourceUid;
use d2b_provider_device_gpu::{
    GpuComponentDescriptor, GpuDeviceNode, GpuSettings, GpuStatus, GpuStatusPhase, GpuWorkerSpec,
    VideoWorkerSpec,
};

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

#[test]
fn worker_allowlists_and_sandbox_shapes_are_closed() {
    let uid = uid("123e4567-e89b-42d3-a456-426614174000");
    let full = GpuWorkerSpec::gpu(&uid, &GpuSettings::default()).unwrap();
    assert_eq!(full.template(), "gpu-worker");
    assert_eq!(full.seccomp_class(), "w1-gpu");
    assert!(full.user_namespace());
    assert!(full.capabilities().is_empty());
    assert!(full.state_mounts().is_empty());
    assert_eq!(
        full.device_nodes(),
        &[
            GpuDeviceNode::Kvm,
            GpuDeviceNode::Dri,
            GpuDeviceNode::Udmabuf
        ]
    );

    let video = VideoWorkerSpec::new(
        &uid,
        &GpuSettings {
            video_sidecar: true,
            video_nvidia_decode: true,
            ..GpuSettings::default()
        },
    )
    .unwrap();
    assert_eq!(video.template(), "video-worker");
    assert_eq!(video.seccomp_class(), "w1-video");
    assert!(!video.user_namespace());
    assert_eq!(
        video.device_nodes(),
        &[
            GpuDeviceNode::Dri,
            GpuDeviceNode::NvidiaCtl,
            GpuDeviceNode::NvidiaDevice,
            GpuDeviceNode::NvidiaUvm
        ]
    );
}

#[test]
fn provider_descriptor_has_no_state_volume_or_state_mount() {
    let descriptor = GpuComponentDescriptor::new();
    assert!(descriptor.provider_state_empty());
    assert!(descriptor.validate().is_ok());
    assert!(descriptor.controller_mounts().is_empty());
    assert!(descriptor.worker_mounts().is_empty());
}

#[test]
fn status_becomes_ready_only_after_required_workers() {
    let mut status = GpuStatus::new(true);
    status.observe_device(true, true);
    status.set_gpu_worker(d2b_provider_device_gpu::GpuConditionState::True);
    assert_ne!(status.phase(), GpuStatusPhase::Ready);
    status.set_video_worker(d2b_provider_device_gpu::GpuConditionState::True);
    assert_eq!(status.phase(), GpuStatusPhase::Ready);
    assert!(status.set_diagnostic("/dev/dri/renderD128").is_err());
}
