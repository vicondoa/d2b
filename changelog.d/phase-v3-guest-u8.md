### Changed

- Move Wayland display proxy lifecycle under durable Host and Guest Process
  resources managed by `d2bd`, preserving signed execution references,
  restart adoption, ordered deletion, and typed clipboard and notification
  streams.
- Keep Host process Providers from treating Guest execution references as
  local launches until authenticated remote process routing is available.
- Persist display Endpoint children through the production Redb resource plane,
  including finalizer-gated deletion and restart reopening.
- Bind display mutation idempotency keys to their request payload or exact
  resource revision, and drain Process children before deleting Endpoints.
- Remove the obsolete display-specific broker routing and Guest systemd
  fallback; production display execution now uses the signed Process path.
