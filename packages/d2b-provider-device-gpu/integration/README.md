# device-gpu integration fixtures

Heavier scenarios live in:

- `gpu_worker_start/` for LaunchTicket-only device grants;
- `render_node_shared/` for two shared render-node claims;
- `video_dependency/` for video-after-GPU readiness.

They require the existing container or Host/Guest integration lane.

