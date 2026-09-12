### Fixed

- A deleted Cloud Hypervisor Guest now drains instead of requeueing forever.
  Once the deleting mark was committed the Guest row served the `Deleted`
  tombstone (the closed phase vocabulary has no `Deleting` value), and the
  session-target resolver refused it, so the delete pass ran with no session
  target, the controller's `CloseSession` step answered
  `cloud-hypervisor-resource-authentication`, and the row held its durable
  deleting mark with a retryable `Delete` driver failure. A row in its delete
  pass is now admitted on its own deletion mark - it still exists and still
  owns the committed identity and live session the deletion path reuses - for
  the Guest and, while the Guest is deleting, for the guest-control Endpoint
  the deletion cascade marked with it. A terminal row that is *not* deleting
  is still refused exactly as before.
- The Guest deletion's session steps (`DrainGuestLocal`, `CloseSession`) close
  over the live session the daemon actually holds for the Guest identity, the
  same identity-scoped lookup the planning observation already uses, instead
  of requiring the committed guest-control Endpoint row to still exist: that
  Endpoint is the Guest's owned child and the manager's deletion cascade
  retires it on its own schedule, which can precede the Guest's finalization
  steps. The Guest row and its committed uid stay the fence, and the closed
  keys are recorded as before.
