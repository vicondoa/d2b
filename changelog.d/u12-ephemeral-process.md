### Changed

- Converted `EphemeralProcess` onto the Process driver: `ProcessDriverFactory`
  now serves both Process-family types (`Process` and `EphemeralProcess`), the
  row's own type name selects the typed spec decode, and the one-shot arm runs
  the preserved ephemeral provider effects (`launch_ephemeral_resource`,
  `adopt_ephemeral_resource`, `probe_ephemeral_resource`,
  `stop_ephemeral_resource`) through the same `ProcessDriverEffects` port. A
  refused one-shot launch is terminal (the type carries no restart policy); an
  `Exited` probe reports `Succeeded`; an over-running process past
  `runtimeDeadline` stops through the fixed 30s/30s escalation and reports
  `Failed`; and `successfulTtl`/`failedTtl` retention is a runtime-only clock
  (no durable `completedAt`/`cleanupEligibleAt` write, R11) that asks the
  manager to retire the row when it elapses, with `incidentHold` keeping a
  failed row until an explicit release.
- Deleted the last production `ResourceReconciler` implementor:
  `ProcessResourceReconciler`, `ProcessResourceRuntime`,
  `process_controller_descriptor`, the guest-local typed Runner
  (`run_guest_process_reconciliation`, `GuestProcessSource`) and the
  `serve_guest` composition that spawned it. `process_resource_runtime.rs` now
  keeps only the canonical launch-identity resolver (`resolve_launch_identity`,
  `guest_runtime_process_matches`), the generic Process list the
  controller-session fences read, and `PROCESS_RESTART_ANNOTATION`.
- Wired the `EphemeralProcess` type into the new plane:
  `V3_CONVERTED_RESOURCE_TYPES` 32 -> 33, the plane's decoder map, and the
  factory's registration; the partition and child-mutation-route tests now
  expect the type on the manager plane.
- The activation-runner mint (KTD13) is unchanged: the NixosGeneration driver
  still mints the runner as an owned `EphemeralProcess` child carrying the
  typed `activationInput`, and its settle watch now sees a manager-served row
  whose one-shot lifecycle reports `Succeeded` on exit and retires through the
  manager.
- Retired the liveness-waiter machinery (`spawn_resource_waiter`, the waiter
  poll loop, `RESOURCE_WAITER_POLL`) whose only caller was the deleted runner;
  the one-shot arm observes through the preserved probe on its own 5s resync
  requeue. The Guest-local credential backend responder composition is
  retained (marked dead-code) for the Guest-side realization follow-on.
