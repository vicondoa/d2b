### Fixed

- The Cloud Hypervisor child relist (`RelistOwnedChildren`) reads exactly the
  session owner's `Process`/`Endpoint`/`Volume` children: the manager leg is
  owner-scoped through the manager's own owner filter instead of listing every
  row of those types in the Zone, so a Zone whose converted rows exceed the
  durable page bound - 85 guests' worth of children crossed 256 - no longer
  fails every guest's relist with `Truncated`, and another owner's rows never
  appear in (or bound) the answer. The manager's complete, unpaginated list is
  folded over the durable leg past the durable 256-row page bound, with the
  manager rendering still winning where both planes hold a reference; the
  durable leg keeps its paging and cap.
- An `AudioBinding` whose `AudioService` (or Guest) row is not committed yet
  defers with the `Pending` phase instead of failing terminally: canonical
  bundle order commits the binding before the service it names, and an
  API-created binding can precede its service entirely, so the old
  `InvalidResource` classification (the terminal `SpecInvalid` driver failure,
  which schedules no requeue) stranded every such binding `Failed` forever. A
  committed row that is not the named dependency keeps the terminal identity
  refusal.
