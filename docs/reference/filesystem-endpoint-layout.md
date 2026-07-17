# Filesystem and endpoint layout

Canonical IDs below are 20-character lowercase base32 identifiers.

| Scope | Persistent | Runtime | Configuration |
| --- | --- | --- | --- |
| Local root | `/var/lib/d2b` | `/run/d2b` | `/etc/d2b` |
| Realm | `/var/lib/d2b/r/<realm-id>` | `/run/d2b/r/<realm-id>` | `/etc/d2b/r/<realm-id>` |
| Workload | `/var/lib/d2b/r/<realm-id>/w/<workload-id>` | `/run/d2b/r/<realm-id>/w/<workload-id>` | `/etc/d2b/r/<realm-id>/w/<workload-id>` |
| Provider | `.../providers/<provider-id>` | `.../p/<provider-id>` | provider rows in the realm bundle |
| Role | owned by its workload | `.../roles/<role-id>` | role rows in `processes.json` |

The local-root public and broker endpoints are the only PID1 socket-activated
endpoints. For a child host-local realm, the allocator pre-binds both listeners
under the realm runtime root and passes them directly to the parent-spawned
controller and broker.

Persistent paths are broker-created from opaque `storage.json` IDs. Runtime
paths are adopted only after process and ownership proof. Human realm/workload
names never select filesystem objects or endpoints.
