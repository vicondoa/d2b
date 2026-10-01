### Changed

- Quota and EmergencyPolicy are served by their own drivers instead of the
  shared declaration-only metadata driver. Each type now carries a decoder
  that reads its real contract (`QuotaPolicy::decode` for a ceiling row,
  `EmergencyPolicySpec` for a policy row) and a driver that resolves its own
  committed row, so a row that cannot state a ceiling or a policy is refused
  at validation rather than converging as opaque JSON.
- A committed `Quota` row's ceilings and a committed `EmergencyPolicy` row's
  reduction are published into a swappable holder the admission reads per
  mutation, rather than a snapshot captured once at spawn, so a ceiling an
  operator commits after the manager started is measured on the next mutation
  instead of the next restart.
- The manager-boundary admission is **not** yet installed. The plane still
  installs the zone-local write fence, so no committed ceiling or reduction is
  enforced against a mutation today. The admission it will replace the fence
  with exists and is proven, and the holder its drivers publish into is
  wired, but the install waits on a per-Zone accepted graph: the verified
  deployment graph is scoped to the deployment while the admission is
  per-Zone, and installing it today would refuse every mutation in every
  Zone-local plane.
- The bundle-ingested subject class the graph admission evaluates is in place
  and used by that admission's own path; it does not change what the
  currently installed fence admits.
- The EmergencyPolicy driver holds the `core.emergency-drain` finalizer while
  an active reduction's open use is still outstanding, so the policy row
  cannot be removed out from under a drain in flight, and releases it once the
  reduction converges or the row is deleted.
