### Changed

- A declared device bind the host does not provide is now a named broker
  refusal: `prepare_device_binds` reports
  `device-bind-missing: <path>` (and `device-bind-unusable` for any other
  errno) instead of a bare `ENOENT`, so the broker journal and audit say which
  grant a launch needed. On a host without a render node the GPU/video worker
  launches now read
  `clone3/spawn failed: device-bind-missing: /dev/dri/renderD128`.
- The Device GPU/video worker rows declared by
  `Provider/device-gpu` carry a restart ceiling
  (`restartPolicy.maxRestarts = 2`, five-minute reset window). A worker whose
  launch is persistently refused (a host device the broker cannot open, a site
  with no projected Wayland socket) previously retried forever, and the
  runtime's `awaiting-restart` pass projected `Ready` for a row no process
  backed. With the ceiling the refusal reaches the closed terminal
  classification `process-start-budget-exhausted` (refused, at
  `reconcile/launch`) while the reset window keeps crash-restart semantics for
  a worker that has been healthy.
