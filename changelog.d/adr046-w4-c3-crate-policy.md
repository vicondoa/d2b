### Added

- Added internal fail-closed admission for optional Provider component state declarations. Empty or unjustified declarations are rejected, but authoritative derivability and schema-custody evidence are not yet available from production Provider deployment.

### Security

- Enforced Provider integration package layout through the existing policy lane and added hermetic checks for bounded, redacted Volume status shapes; this does not make Provider state production-reachable.
