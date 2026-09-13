### Changed

- A declared device bind the host does not provide is now a named broker
  refusal: `prepare_device_binds` reports
  `device-bind-missing: <path>` (and `device-bind-unusable` for any other
  errno) instead of a bare `ENOENT`, so the broker journal and audit say which
  grant a launch needed. On a host without a render node the GPU/video worker
  launches now read
  `clone3/spawn failed: device-bind-missing: /dev/dri/renderD128`.
- `videoNvidiaDecode` selects the closed `video-worker-nvidia` worker
  template instead of being dropped or refused: the Device's video row
  declares that template (`packages/d2b-provider-device-gpu/nix/default.nix`)
  and its launch carries the reviewed bind set (`/dev/dri/renderD128`,
  `/dev/nvidiactl`, `/dev/nvidia-uvm`, `/dev/nvidia0`) from `d2b-core`'s
  posture table. A host that lacks one of those nodes refuses by name
  (`device-bind-missing: <path>`; `/dev/nvidia0` on the documented single-GPU
  default), and the ceiling below makes a persistent refusal terminal instead
  of an endless requeue.
- The Device GPU/video worker rows declared by
  `Provider/device-gpu` carry a restart ceiling
  (`restartPolicy.maxRestarts = 2`). A worker whose launch is persistently
  refused (a host device the broker cannot open, a site with no projected
  Wayland socket) previously retried forever. With the ceiling the refusal
  reaches the closed terminal classification `process-start-budget-exhausted`
  (refused, at `reconcile/launch`) instead of an endless requeue. The ceiling
  is a per-daemon-lifetime launch counter, not a window: the driver's
  in-memory `RestartBudget` counts up and never resets, so `resetAfter` has
  no effect (`packages/d2bd/src/process_driver.rs`).
