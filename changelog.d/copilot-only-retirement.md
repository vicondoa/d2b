### Changed

- Copilot is now the sole authoritative repository integration. Its native
  agents, skills, and explicitly bound Task lanes are the supported process
  surface.

### Removed

- Retired the legacy integration and its tracked command, agent, and package
  files; stale integration state now fails closed instead of selecting a
  compatibility path.
