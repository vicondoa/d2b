### Removed

- Removed production reachability for the retired guest-control exec and
  config wires, direct exec owner, and guest-control shell and activation
  adapters. Process resources, ConfigNixos, and ComponentSession named
  streams now own those paths.
- Removed the standalone guest daemon, legacy guest protocol bindings,
  token-share broker operation, obsolete SSH readiness role, and retired
  package-policy inputs. Old peers fail closed at the ComponentSession
  boundary.
- Removed the Wayland proxy's compatibility host-terminal child launcher;
  desktop terminal processes remain owned by their signed Process or
  companion.

### Fixed

- Require a configured non-root workload user for persistent Guest shell
  service wiring and keep ComponentSession evaluation aligned with the active
  `d2b.sshUser` field.
- Fail non-qemu USBIP attach, detach, and VM-start reconciliation closed with
  typed `runtime-capability-unsupported` when the target-local
  `usbip-guest-proxy` Process and ComponentSession path is unavailable. Host
  bind, proxy reconciliation, and claim release are no longer reported or
  performed as successful USB attachment work; VM-stop cleanup preserves the
  claim until Guest detach is available.
