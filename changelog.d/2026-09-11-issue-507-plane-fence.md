### Changed

- The resource plane partition is now enforced by one authority in
  `d2b-contracts` (`ResourcePlane`, `resource_plane`, the converted-type
  registry) instead of per call site, and the API store facades and the
  daemon's child bridges consult it. A converted type can no longer be read
  or written through the legacy store: the access fails with a named,
  non-retryable `WrongPlane` error that carries the resource type and the
  caller instead of being reported as absence or as a retryable read failure.
- The daemon's Cloud Hypervisor session lists converted types through the
  manager plane, so a controller that relists its owned children no longer
  races the legacy mirror.
- The fence arms when the zone's manager plane is published; before that the
  durable path keeps serving, so first boot and unit fixtures are unaffected.

### Fixed

- A wrong-plane access is no longer indistinguishable from "row absent": the
  refusal is logged with the type and the caller and never retried.
