### Changed

- Hardened the canonical ACA, Azure VM, Cloud Hypervisor, and Azure Relay
  Providers with bounded readiness, startup, credential, bootstrap, and
  reconnect behavior.

### Fixed

- Prevented failed bootstrap deliveries, unhealthy guest control, expired
  leases, and relay socket loss from being reported as ready or leaking
  session capacity.
