# Storage lifecycle report

**Diataxis category:** reference.

`nixlingd` writes a host-local startup contract report at:

```text
/var/lib/nixling/daemon-state/storage-lifecycle-report.json
```

The report records the daemon's read-only check of the generated
storage, restart, and synchronization contracts in the active bundle.
It is diagnostic evidence for host doctor/status surfaces; privileged
repair still resolves trusted bundle IDs through the broker and must
not trust this report as authority.

The report never contains raw managed paths. Issue entries use a closed
`kind` taxonomy plus bounded contract, VM, role, and offending contract-row
identifiers from the bundle. The schema is generated at
[`docs/reference/schemas/v2/storage-lifecycle-report.json`](./schemas/v2/storage-lifecycle-report.json).
