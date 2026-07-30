# volume-local integration fixtures

This directory holds the heavier container and Host fixtures for
`Provider/volume-local`. They need a writable Host tree, a real
filesystem boundary, or a quota-capable backing filesystem, so they
cannot run at the hermetic layer that `tests/` occupies.

The behaviours that belong here, and that the hermetic suite deliberately
proves only at the policy level, are:

| Fixture | What only a real filesystem can prove |
| --- | --- |
| Host-path access | the resolved root is reachable and the allowlist entry is honoured end to end |
| store-view boundary | `st_dev` equality between the host store and the Volume root, and `EXDEV`/`EMLINK` handling |
| marker durability | the zero-length readiness marker survives a controller restart |
| quota enforcement | a backing filesystem that genuinely cannot enforce byte and inode ceilings |
| TPM marker | a root-owned marker outside the Volume tree, and the fail-closed missing-state path |

No fixture is wired into an orchestrator yet: the effect adapter that
these fixtures would drive is owned by ProviderSupervisor and is not
landed. Adding a fixture here without that adapter would assert against a
stub rather than the shipped path.
