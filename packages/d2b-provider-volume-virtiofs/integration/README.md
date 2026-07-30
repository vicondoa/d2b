# volume-virtiofs integration fixtures

This directory holds the heavier fixtures for `Provider/volume-virtiofs`.
They need a real virtiofsd binary, a listening socket, and a booted
guest, so they cannot run at the hermetic layer that `tests/` occupies.

| Fixture | What only a real launch can prove |
| --- | --- |
| worker launch | the frozen argv is accepted by the shipped virtiofsd build |
| user namespace | the single-entry mapping is in place before the worker's first instruction |
| socket readiness | the private socket listens and carries the resolved group |
| guest mount | the guest observes the share at its mount path with the expected access |
| finalizer drain | the mount is gone after the worker is deleted, across a Guest restart |

No fixture is wired into an orchestrator yet: the effect adapter these
fixtures would drive is owned by ProviderSupervisor and is not landed.
