# `storage-lifecycle-report.json` schema (`v2`)

Schema: [`storage-lifecycle-report.json`](./storage-lifecycle-report.json)

`storage-lifecycle-report.json` is the host-local daemon startup report on
storage/restart/sync contract posture, written by `d2bd` (`packages/d2bd/src/
storage_lifecycle.rs`) to `/var/lib/d2b/daemon-state/storage-lifecycle-report.json`.
See [`docs/reference/storage-lifecycle-report.md`](../../storage-lifecycle-report.md)
for the operator-facing description of the artifact's role, location, and
diagnostic-only authority. This page documents the generated schema itself.

## Top-level fields

- `schemaVersion` — schema version for this artifact (currently `"v2"`).
- `storageContractPresent` / `syncContractPresent` — whether the active
  bundle carried a `storage.json` / `sync.json` contract at all.
- `pathCount` / `lockCount` / `restartPolicyCount` — counts from the parsed
  contracts, for quick doctor/status summaries.
- `issues` — the closed `StorageLifecycleIssue` taxonomy below; empty when
  the daemon's read-only contract check found no problems.

## `StorageLifecycleIssue` kinds

A closed, tagged-union `kind` taxonomy. Every variant is fully bounded —
never a raw managed path, only bundle-scoped ids and closed reason slugs:

- `missing-storage-contract` / `missing-sync-contract` — the active bundle
  did not carry the corresponding contract at all.
- `legacy-bundle-contracts-unavailable` (`bundleVersion`) — the bundle
  predates the contracts this report validates.
- `bundle-resolver-unavailable` — the daemon could not resolve a bundle to
  check in the first place.
- `storage-contract-invalid` (`contractId`, `reason`, optional `offendingId`)
  — `reason` is a `StorageContractValidationReason` slug.
- `sync-contract-invalid` (`contractId`, `reason`, optional `offendingId`) —
  `reason` is a `SyncContractValidationReason` slug.
- `missing-restart-policy` / `adoptable-missing-cgroup-leaf` (`roleId`, `vm`)
  — restart/adoption posture gaps for a specific VM/role.

## `SyncContractValidationReason`

Closed reason slugs for a `sync-contract-invalid` issue:

- `duplicate-lock-id` — the same lock id appears more than once in
  `sync.json`.
- `ofd-lock-missing-cloexec` — a lock row declares `cloexecRequired: false`
  (or omits it), which the generated-row acquisition bridge
  (`d2b_state::LockSet::acquire_from_generated`, documented in
  [`sync.md`](./sync.md)) always fails closed on.
- `fd-passing-missing-lease-transfer-record` — a lock declares an fd-passing
  mechanism (`ScmRights`/`ExplicitFdMapping`) without the paired
  inheritance-policy record the mechanism requires.
- `duplicate-acquire-order` — two locks resolve to the same total-order rank
  under `SyncJson::global_order_rank`.
- `lock-missing-protected-resource-id` — a lock row's `resourceId` (the
  *protected state* resource this lock guards — see the `resourceId`
  contract note in [`sync.md`](./sync.md)) is null, or does not resolve to
  exactly one row in the paired `storage.json`. Every generated lock must
  authorize a real, uniquely identified protected resource before the
  generated-row runtime bridge can drive it; a lock that fails this check has
  no runtime-acquirable protected-resource identity at all, and is
  classified under this reason rather than falling through to
  `unclassified`.
- `unclassified` — a validation failure the classifier could not attribute
  to a more specific reason above. Any newly detected failure mode gets its
  own named reason instead of silently accumulating under `unclassified`.

## `StorageContractValidationReason`

Closed reason slugs for a `storage-contract-invalid` issue:

- `duplicate-storage-path-id` — the same storage path id appears more than
  once in `storage.json`.
- `duplicate-restart-policy` — the same restart policy key appears more than
  once.
- `duplicate-degraded-reason` — the same degraded-state reason slug appears
  more than once.
- `unclassified` — as above, a fallback that new failure modes must not
  accumulate under.

## Schema-version compatibility

Adding `lock-missing-protected-resource-id` to `SyncContractValidationReason`
is an additive, backward-compatible change: it is a new enum member, not a
renamed or removed one, and `schemaVersion` remains `"v2"`. Older readers that
do not recognize the new slug still parse every other field; only doctor/CLI
surfaces that exhaustively match on `SyncContractValidationReason` (see
`packages/d2b/src/doctor.rs`) needed a corresponding non-breaking update to
render it distinctly instead of panicking or silently dropping it.
