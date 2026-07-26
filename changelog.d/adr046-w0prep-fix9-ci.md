### Fixed

- The pull-request and local Layer-1 graphs now execute the performance canary
  and migration-ledger drift check instead of allowing those gates to remain
  present but unreachable.
- Shell commands in generated and handwritten GitHub Actions workflows now
  clear inherited shell functions before invoking repository tools.
- The CI coverage guard now accepts evidence only from Make targets that the
  local Layer-1 manifest executes, so legacy non-executing aggregators cannot
  hide an orphaned gate.
