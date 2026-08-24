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
