### Changed

- Cloud Hypervisor Guest bootstrap planning now derives a deterministic,
  UID-free direct-child graph and keeps VMM lifecycle eligibility pure until
  Device, Network, Volume, Export, and setup dependencies are ready.
