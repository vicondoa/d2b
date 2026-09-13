### Changed

- Converted the system-core Host/User family to the v3 plane: a new
  `SystemCoreDriver`/`SystemCoreDriverFactory`
  (`packages/d2bd/src/system_core_driver.rs`) serves `Host` and `User` over
  the bounded probe/NSS-discovery effects port. Validate checks the Host
  provider reference and decodes both specs; reconcile observes the host
  once per generation (with the degraded fallback) or discovers the local
  user, publishes the typed status in memory (R11) and short-circuits at the
  current generation; delete converges with no effects or children. `Host`
  and `User` join `V3_CONVERTED_RESOURCE_TYPES` and are served only by the
  manager plane. The family's durable status projection is consequently
  no longer store-visible; the read view serves the actor's in-memory
  phase.

### Removed

- Deleted the typed Host/User handler and its runner: the
  `SystemCoreResourceReconciler` + `ResourceReconciler` impl, the
  `SystemCoreHostProbe` / `SystemCoreUserDiscovery` port implementations
  (moved into the driver), the system-core descriptor/fence/API/source
  block, and the Host/User runner spawn and transient-retry loop from
  `resource_runtime.rs`.
