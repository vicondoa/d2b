### Fixed

- The daemon's Process-family broker clients poll their calls for the family's
  own declared per-call deadline instead of a hardcoded 10 seconds. The three
  budgets in `ProductionProcessProviders::new_for_mode`
  (`packages/d2bd/src/process_provider_runtime.rs`: the adoption observation
  socket, `BrokerProcessBackend::with_socket_profile_and_role`, and
  `BrokerSystemdEffectOwner::with_socket_and_role`) now derive from
  `BROKER_IO_TIMEOUT`, which is the carrier's `DEFAULT_CONTEXT_DEADLINE_MS`
  (25 s) - the same constant `DeadlineTier::Standard::budget_ms()` returns, so
  no literal is restated. Polling shorter than the budget the call is served
  under abandoned spawns the broker was still running, and an abandoned spawn
  is not inert: the controller child the broker had already created stayed
  live holding its runner registration, so every relaunch was refused as a
  duplicate (`handler-refused`) and the Process wedged in `Pending` until the
  `runtime-cloud-hypervisor-guest-preflight` host-integration wait expired.
  The daemon now receives the broker's own verdict on those legs, ordered
  under the launch ticket's deadline (30 s at the call sites), whose
  late-launch path stops a process that outruns it.

- The Guest lifecycle claim (`consume_lifecycle_lease`) reports an unanswered
  broker as `StateUnavailable` rather than `EffectRejected`, and polls the
  `consume-cell`/`complete-cell` rows under their declared
  `DEFAULT_CONTEXT_DEADLINE_MS` (25 s, `LIFECYCLE_CELL_IO_TIMEOUT`) instead of
  the legacy 10 s `KERNEL_IO_TIMEOUT`. A lost claim reply used to arrive as the
  permanent spelling the Device Providers refuse to retry, so the TPM device
  controller spun on `Effect(EffectRejected)` for the rest of the run (100 to
  855 lines in the failing `device-worker-launch` runs against none in most
  passing ones) and the one-shot `EphemeralProcess/swtpm-flush-tpm0` stayed
  refused, making the fixture's flush-outcome wait unsatisfiable. The served
  replay refusal still carries `EffectRejected`, so a genuinely spent claim
  keeps its permanent classification.
