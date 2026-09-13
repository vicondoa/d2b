### Added

- `d2b debug <zone> [<type>/<name>]` reports why a Zone resource is stuck:
  the ownership tree of the zone's rows, each row's plane, phase, generation
  and the generation of the status behind that phase, its owned children with
  their phases, and the last structured driver failure with its outcome,
  operation, stage, and likely cause. A Ready subtree collapses to one line
  and `--all` expands it; the machine-readable form carries every row the
  human tree collapsed, and a read that only partly succeeded names the type
  it could not read instead of looking like an empty zone.

### Changed

- A resource row's status now reports the generation it was published for
  alongside the generation it was read at, so a status that no longer
  corresponds to a row's spec is visible rather than reading as a row that
  never reported.
- A failing host-integration fixture now prints the composed `d2b debug`
  explanation for its zone alongside the row dumps it already collected, so a
  failed lane names the stuck row and its structured failure without a
  follow-up run.
