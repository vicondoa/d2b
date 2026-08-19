### Fixed

- Host-integration Crane builds now overlay real sources per package and its
  path-dep closure, so a daemon edit no longer rebuilds the resource compiler,
  Wayland proxy, or other unrelated host tools. Broker deps use a separate
  dummy-source artifact cache.
