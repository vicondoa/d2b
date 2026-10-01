### Changed

- Quota and EmergencyPolicy are served by their own drivers instead of the
  shared declaration-only metadata driver. Each type now carries a decoder
  that reads its real contract (`QuotaPolicy::decode` for a ceiling row,
  `EmergencyPolicySpec` for a policy row) and a driver that resolves its own
  committed row, so a row that cannot state a ceiling or a policy is refused
  at validation rather than converging as opaque JSON.
- A committed `Quota` row's ceilings and a committed `EmergencyPolicy` row's
  reduction are now published to the manager-boundary admission, which reads
  them per mutation rather than capturing them once. A ceiling an operator
  commits after the manager spawned is therefore enforced on the next
  mutation instead of the next restart.
- `packages/d2bd` installs `GraphLimitsAdmission` as the plane's manager
  admission in place of the zone-local write fence, composed from the prior
  accepted graph the verified deployment publication produced. The same
  admission evaluates the bundle-ingested subject class, so bundle ingest is
  admitted the way it was before.
- A Zone with no verified deployment graph now refuses every mutation with a
  reason naming the missing authority, rather than being admitted by an
  authority nothing established.
- The EmergencyPolicy driver holds the `core.emergency-drain` finalizer while
  an active reduction's open use is still outstanding, so the policy row
  cannot be removed out from under a drain in flight, and releases it once the
  reduction converges or the row is deleted.
