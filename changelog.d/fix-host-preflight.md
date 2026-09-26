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
