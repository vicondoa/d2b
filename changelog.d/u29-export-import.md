### Added

- A `ResourceImport` is now a typed lease over an admitted qualified semantic
  Service projection and nothing else. `AdmittedImportUse` has no accessor and
  no constructor that yields a `Volume`, `Device`, `Network`, `Endpoint`,
  `Credential`, or primitive binding reference, so the local copy an import
  could never legitimately be is unexpressible rather than merely discouraged.
  The two provider crates publish the closed vocabularies their conversions
  use: the export driver names no subject at all, and the import driver names
  the primitive types it must never materialize locally.
- An importing consumer's use is now one decision. The signed semantic catalog
  admits the family shape - the projection must be import-owned and of the
  family's Service type, and the consuming target must be in the family's
  closed target set - and the exporting Zone's consumer policy, capability
  ceiling, and lease then narrow it. A request that names a backing-resource
  reference or a local physical effect is refused by name rather than narrowed
  away, and a capability outside the lease is refused rather than dropped, so an
  over-ceiling request never looks admitted.
- Every family's projection schema is now checked, family-neutrally, to carry
  none of its own declared backing-reference fields. The USB prohibition stops
  being a fact of one family's field list and becomes an invariant the common
  graph cannot relax when a new family or a new common field arrives.
- Export revocation is typed and evidenced. `ExportRevocation` begins fenced,
  so new use stops being possible before anything else happens, and the
  advertisement is only withdrawn once no lease is still active or draining -
  the same pre-drain ordering the binding lifecycle uses. Forcing remains a
  controller decision the declared policy has to ask for, after its grace
  period.
- A ZoneLink now carries a share scope that is fixed with its owner proof
  rather than claimed per call. A consumer-scoped link refuses to relay the
  share to a third Zone, re-advertise the export, attach to a remote resource,
  or hand it over in a support bundle; a reconnect returns the same scope, and
  a restarted cursor whose durable observation disagrees with the registered
  scope is quarantined instead of adopted.

### Changed

- An export's subject is refused against its stored owner reference, so a
  projection owned by a `ResourceImport` is never re-advertised. A consumer
  Zone holds a lease over the source Zone's Service, not the authority to
  publish it onward.
