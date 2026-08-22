### Fixed

- Stop authenticated Relay display rendezvous pumps before listener shutdown,
  cancellation, or reconnect so successive local socket sessions cannot
  overlap.
