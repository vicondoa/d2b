### Added

- U17 slice 2 (the port conversions) with the regression tests that pin them:
  five TPM port row tests and one Process-driver `NeverAdopt` regression
  (proven to fail before its fix). The TPM effect port and the GPU lifecycle
  port now realize their workers as the Device Providers' declared Process
  rows and observe the phases their Process controllers publish; the daemon
  has no direct broker spawn left.

### Changed

- The TPM effect port (`tpm_effect_port.rs`) reads and retires the declared
  `Process/swtpm-<device>` / `EphemeralProcess/swtpm-flush-<device>` /
  `Endpoint/tpm-<device>` rows through the Device's manager child surface and
  ensures the controller-owned `Volume/device-<32hex>-tpm-state` child, so the
  Process controller owns the launch, restart, adoption, drain and teardown of
  swtpm and the pre-start flush. The two broker calls that remain are the
  one-time legacy state migration and the broker-owned state-directory
  preparation, neither of which launches a process. The retirement of the
  synthetic `device-<12hex>-*` refs replaces the durable-snapshot adoption
  gate: the row's owner (the requiring Device) and its published phase carry
  the classification now, which is what the port's new tests pin.
- The GPU lifecycle port (`shared_provider_effects.rs`) reads the declared
  `Process/gpu-<device>` / `Process/video-<device>` rows as the worker
  identity: the opaque process token is derived from the row's durable uid and
  generation, `observe_worker` recomputes it from the row (so a replaced row
  reads as `StaleIdentity`, not `Missing`), and `stop_worker` deletes the row
  and reports closure only once the row is gone. `open_authorized_devices` no
  longer opens device classes - the grants travel in the launch intent's
  declared posture (the closed `device_worker_posture` binds plus the
  broker's render-node pre-open).
- `SharedProviderChildSurface` gained `view` (the manager's live row view) so
  a Provider effect can gate on a child row's published phase without holding
  a broker handle; `view_phase` is the one phase projection both the driver
  and the effect ports read.
- The TPM provider's typed row builders now carry the declared template
  posture: `build_swtpm_process_spec` uses umask `0007` (the socket-ACL
  umask the broker profile selects) and `build_swtpm_flush_spec` declares no
  user namespace (the preserved user-NS-long-lived-only contract), matching
  the Nix-projected rows the compiler fences against.
- The GPU README and the driver/effect module docs describe the row-owned
  launch instead of the retired `OpenDevice`/`SpawnRunner` mapping.

### Removed

- The TPM direct spawn path: `LiveTpmEffectExecutor` (its
  `BrokerRequest::SpawnRunner` request, the pidfd reservation and
  registration, the runner-snapshot adoption classification, the launch
  tickets) and the `CoreTpmEffectExecutor`/`TpmEffectPort` implementation
  that existed to drive it, plus the now-orphaned
  `stop_unregistered_spawned_runner` / `signal_unregistered_spawned_runner` /
  `wait_unregistered_spawned_runner_reaped` helpers and their broker-double
  test. `write_runner_snapshot[_with_authorization]` survives as test-double
  seeding only.
- The GPU direct spawn path: `spawn_worker`, its `SpawnRunner` request with
  `inherited_fd_count`, `open_device_classes`, the intent lookup through
  `find_runner_intent_for_process_in_vm`, and the `gpu_processes` /
  `gpu_opened_devices` per-resource state maps with their take/retain
  helpers.

### Known gap (reported, not worked around)

- Both converted families launch with the trusted template's pinned argv
  (`argv[0]` only): the parameters their generators need are host paths no
  typed row carries (`generate_swtpm_argv`: state dir + ctrl/server sockets;
  `generate_gpu_argv`/`generate_video_argv`: crosvm socket + Wayland socket),
  and the Process spec is argv-free while the declared rows are path-free by
  contract. The `launch_args` channel exists end to end, but its only
  composition point is the daemon's provider runtime, which has no typed
  source for those values. Closing it needs a typed device-worker launch
  parameterization; until then the declared rows launch bare and fail closed
  on their readiness/endpoint gates.
- The one-shot flush's exit is not observable through the row contract: the
  Process driver records the completion in memory and both a clean exit and a
  failed one leave the row `Ready` for its retention TTL, so the port reports
  the flush complete when the declared row leaves `Pending`.
