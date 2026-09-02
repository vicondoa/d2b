# Historical realm tree routing

**Diataxis category:** historical explanation.

The realm-tree routing model is retained for ADR and migration context only.
It is not a current route, ResourceRef, Gateway credential, or authorization
source.

Current routing is Zone-scoped and uses authenticated ZoneLink/Resource
sessions with exact Resource identity, Provider generation, and revision
fencing. See [`../reference/zone-cli-contract.md`](../reference/zone-cli-contract.md)
and [`daemon-lifecycle.md`](./daemon-lifecycle.md).
