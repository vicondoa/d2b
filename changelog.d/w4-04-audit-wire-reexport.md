### Fixed

- The broker contract crate now re-exports the audit-export wire types (`AuditExportCursor`, `AuditExportEntry`, `AuditExportErrorCode`) from the crate root instead of the `broker_wire` module, so each type has a single canonical re-export path.