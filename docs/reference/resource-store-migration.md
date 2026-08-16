# Resource-store restore and physical-schema migration

The redb resource-store migration owner accepts an already-open parent
directory descriptor and the already-open identity marker descriptor. It does
not accept a filesystem path. Publication names are fixed by the backend:

| Role | Name |
| --- | --- |
| active database | `store.redb` |
| staged database | `store.redb.staged` |
| retained prior database | `store.redb.prior` |

The marker is checked against the requested store slot, store UUID, Zone, Zone
UID, and creation timestamp before any database is opened for migration.
Active, staged, and retained files are opened relative to the parent descriptor
with close-on-exec and no-follow flags. A missing or replaced active file is
not silently provisioned during an upgrade.

## Restore

`restore_owned` validates a canonical `LogicalBackup` against the marker-bound
store identity, restores it into a new staged redb file, validates the complete
physical table set and indexes, and syncs both the file and parent directory.
Only then does it rename the active file to the retained prior name and rename
the staged file to the active name. Each rename is followed by a parent
directory sync. A failed publication attempts to restore the prior active file;
if rollback cannot be proven, the publication is quarantined.

Restore never writes the active database in place. A failure while building or
validating the staged file removes only that staged file and leaves the active
file untouched.

The logical image carries the store UUID, Zone identity, physical schema,
revision metadata, and validated table rows. Restoring it therefore preserves
the ResourceRef, UID, generation, canonical JSON, and payload digest of each
resource. The restored store retains its store identity and advances
`backup_generation`; Provider and process adoption starts only after staged
publication and validation have completed.

## Upgrade

The registered chain is explicit and finite. The current physical schema is
version `2`; supported upgrades are `0 -> 1 -> 2`. Version `0` has an
unversioned metadata record, and version `1` adds the authority lifecycle
shape. Each step copies rows into a new staged database and updates only the
staged metadata before current-schema validation. The public
`upgrade_owned_after_backup` entry point consumes an identity-validated
logical backup first, so a schema advance cannot bypass the recovery boundary.

Versions that are unknown or absent from the registered chain return
`upgrade-required` before staging. No migration guesses a conversion from an
unapproved version.

Logical restore does not provide a downgrade path. A backup whose physical
schema is older than the current registered schema is rejected with
`upgrade-required` before staging or publication; it must be regenerated or
explicitly advanced through a supported migration before it can be restored.
Older binaries are not supported as a downgrade mechanism, and no live runtime
is adopted from an older or partially converted image.

## Crash recovery

The owner classifies the three publication names as one of the closed
publication states. It resumes only states with a validated staged image and
identity-compatible active or prior images:

- staged-only: publish the staged image;
- active-plus-staged: complete the publication;
- staged-plus-prior: complete the staged rename and validate the prior image;
- active-plus-prior: validate the active image and remove the retained prior
  after recovery.

Prior-only, all-three-present, non-regular, corrupt, and identity-mismatched
states are quarantined. Recovery is idempotent: after a successful resume or
finalization, a second call sees a clean publication state.

The owner uses full redb immediate durability, `fsync` on staged files, and
`fsync` on the anchored parent directory at every publication boundary.
Neither restore nor upgrade performs broad ownership, mode, ACL, or path
repairs.
