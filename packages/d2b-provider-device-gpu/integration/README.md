# Integration fixtures

The declared fixture directories name the real device-GPU scenarios:

- `gpu_worker_start/` - the GPU sidecar starts against its declared device
  nodes and private socket.
- `render_node_shared/` - the render node the Device declared is the one the
  worker receives.
- `video_dependency/` - the video sidecar resolves the dependency it declares.

Each fixture directory carries its own README; none is executable wiring yet.
