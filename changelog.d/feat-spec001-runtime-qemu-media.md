### Added

- Add a typed qemu-media Guest runtime with broker-owned process launch,
  Host-global KVM admission, QMP health and hotplug handling, restart
  adoption, ordered finalization, and redacted audit/telemetry projections.

### Changed

- Treat pause-at-boot as an initial QMP proof and bound greeting timeouts to
  fresh launches while adopted runners retry through health degradation.
