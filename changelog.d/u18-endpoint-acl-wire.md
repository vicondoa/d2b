### Added

- The broker wire now carries the exact-endpoint ACL surface (U18, R23) as
  three variants over one shared request: `EndpointObserve`,
  `EndpointGrantAccess`, and `EndpointRevokeAccess` answer
  `BrokerResponse::EndpointAccess`. The request names the exact endpoint, the
  committed consumer, the Zone, a socket NAME, the POSIX rights, and an
  authority key, and carries no path anywhere: the socket is a bounded token
  whose grammar admits no `/`, no `.`, and no `..`, so it can only ever select
  a direct child of the directory the broker derives from its own serve-time
  configuration. The broker recomputes
  `endpoint_access_authority_binding` before any path is resolved, derives the
  consumer's numeric principal from the verified Zone bundle, and reports the
  pinned `(dev, ino)` it applied the effect to along with the kernel's
  effective rights, the ancestor traverse, and the container's listing
  authority.
