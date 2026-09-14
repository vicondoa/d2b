use d2b_contracts_resource::v3::ResourceUid;
use d2b_provider_device_gpu::{GpuProcessRole, GpuSettings, GpuWorkerSpec, VideoWorkerSpec};

fn uid(value: &str) -> ResourceUid {
    ResourceUid::parse(value).unwrap()
}

#[test]
fn worker_templates_follow_the_selected_role_and_declared_settings() {
    let uid = uid("123e4567-e89b-42d3-a456-426614174000");
    let full = GpuWorkerSpec::gpu(&uid, &GpuSettings::default()).unwrap();
    assert_eq!(full.template(), "gpu-worker");
    assert_eq!(full.process().role(), GpuProcessRole::FullGpu);

    let render_node = GpuWorkerSpec::gpu(
        &uid,
        &GpuSettings {
            render_node_only: true,
            ..GpuSettings::default()
        },
    )
    .unwrap();
    assert_eq!(render_node.template(), "gpu-render-node");
    assert_eq!(render_node.process().role(), GpuProcessRole::RenderNode);

    let video = VideoWorkerSpec::new(
        &uid,
        &GpuSettings {
            video_sidecar: true,
            video_nvidia_decode: true,
            ..GpuSettings::default()
        },
    )
    .unwrap();
    assert_eq!(video.template(), "video-worker-nvidia");
    assert_eq!(video.process().role(), GpuProcessRole::Video);

    let plain = VideoWorkerSpec::new(
        &uid,
        &GpuSettings {
            video_sidecar: true,
            ..GpuSettings::default()
        },
    )
    .unwrap();
    assert_eq!(plain.template(), "video-worker");
    assert_eq!(plain.process().role(), GpuProcessRole::Video);
}
