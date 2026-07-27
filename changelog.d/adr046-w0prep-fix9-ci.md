### Fixed

- The pull-request and local Layer-1 graphs now run the manifest jobs for the
  performance canary and migration-ledger drift check instead of leaving them
  unreachable. The performance job is advisory: without
  `D2B_PERF_STABLE=1` on a pinned self-hosted runner it reports `SKIP`, enforces
  nothing, and is not counted as an enforcing green job.
- Shell commands in generated and handwritten GitHub Actions workflows now
  clear inherited shell functions before invoking repository tools.
- The CI coverage guard now accepts evidence only from Make targets that the
  local Layer-1 manifest executes, so legacy non-executing aggregators cannot
  hide an orphaned gate.
