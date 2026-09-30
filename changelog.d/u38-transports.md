### Added

- Added a graph-bound attach path to the local Unix transport portal.
  `TransportService::open_under_binding` admits an `EndpointBinding`
  relationship and then measures the presented evidence against that
  relationship's committed fence, so a peer that is not under an admitted
  binding key has no attach path at all. A foreign Zone, a different store
  incarnation, a revoked or draining relationship, a stale source or consumer
  generation, an advanced desired revision or Zone desired sequence, and a
  reconnect ordinal below the relationship's floor each refuse with the
  enforcing admission stage and reason rather than with a transport-specific
  error.
- Added a bounded, service-local `TransportBindingRegistry` of admitted
  transport relationships, so the portal no longer keeps any access list of
  its own. The registry ceiling is frozen and refusing an admission is the
  answer at the ceiling; a live key is never silently replaced and a
  finalized or forgotten relationship is re-established only by an explicit
  admission.
- Added an in-process control surface (`ControlPlaneRequest` with
  `Open`/`Close`/`Observe`) that can only be issued against a live admitted
  route, because issuing one requires the opaque `ControlRouteToken` that
  only the route can mint.

### Changed

- The kernel uid/gid a graph-bound transport may be pinned to is now derived
  from the admitted relationship rather than supplied beside the request. A
  relationship pinned to another peer is refused with
  `peer-policy-mismatch`, which is distinct from the
  `peer-credentials-unavailable` refusal that means the kernel would not
  report a peer at all.
- An open that requests descriptor attachments must now have been admitted as
  an `Attach` relationship; a `Connect` or `Listen` relationship refuses with
  `attachment-kind-conflict`. The existing route-class and socket-kind rules
  are unchanged.

### Security

- Stream-carried bytes can no longer become a privileged control operation.
  The one function that could have parsed the data plane for control scans the
  carriage for an operation discriminant and always refuses, because carriage
  can name an operation but cannot carry a route token.
- A revoked relationship is effective at the service rather than at the
  caller's copy of it: the service attaches against the relationship its
  registry currently admits, so a reconnect presenting a value admitted before
  a revoke is refused with `relationship-revoked`.
