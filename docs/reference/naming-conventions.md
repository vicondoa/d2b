# Naming conventions

Canonical reference for current host-visible d2b names. A name is an
operator-facing address; private runtime scope, broker handles, pidfds, and
host paths are derived separately and are never accepted as public aliases.

## ResourceRefs

ResourceRefs use `ResourceType/name`:

```text
Zone/local-root
Guest/work-app
Process/work-app-worker
EphemeralProcess/exec-1
Provider/runtime-cloud-hypervisor
Network/work-lan
```

Resource names match `^[a-z][a-z0-9-]{0,62}$`. ResourceRefs are Zone-local
unless a contract explicitly names a public cross-Zone projection. UIDs,
generations, and revisions fence lifecycle operations after the name resolves.

## Zones and Guests

- The distinguished root Zone is `local-root`.
- A child Zone names its compiler-owned `parentZone`.
- A Guest name is unique only within its owning Zone.
- Host-global runtime identity derives from Zone UID plus Guest UID, never from
  a Guest name alone.
- A Guest controller owns direct child names and ownerRefs; specialized
  controllers own descendant effects.

Generic context labels such as `work` and `personal` are ordinary Zone names,
not a second hierarchy. Provider and system artifact IDs are plain bounded
lowercase IDs, not ResourceRefs or filesystem paths.

## Root-visible units and users

| Resource | Current name |
| --- | --- |
| Zone daemon | `d2bd.service` |
| Privileged broker socket | `d2b-broker.socket` |
| Privileged broker service | `d2b-broker.service` |
| Public daemon socket | `/run/d2b/public.sock` |
| Broker socket | `/run/d2b/priv.sock` |
| Lifecycle group | `d2b` |
| Unsafe-local helper socket | `/run/d2b/unsafe-local-helper.sock` |

There are no framework-owned per-Guest systemd units or host-singleton
lifecycle services. Broker-spawned runners use delegated leaves below
`/sys/fs/cgroup/d2b.slice/<vm>/<role>/`; `<vm>` is the broker-private runtime
identity derived from Zone and Guest state, not a public Guest-name lookup.

## Provider and artifact IDs

Artifact IDs match `^[a-z][a-z0-9-]*$` and identify a consumer-declared
immutable package in `d2b.artifacts`. Provider resources select an artifact by
`spec.artifactId`; Guest resources select a NixOS system by
`spec.systemArtifactId`.

The bootstrap Providers `Provider/system-core` and
`Provider/system-minijail` are projected by the framework and must not be
hand-authored.

## Shell and operation IDs

Persistent shell names use a bounded ASCII token:

- first byte `[A-Za-z0-9_]`;
- remaining bytes `[A-Za-z0-9._-]`;
- no whitespace, slash, braces, or leading `-`.

Shell names are scoped to a Zone session. Operation IDs, stream cursors,
execution IDs, Resource UIDs, and broker handles are opaque bounded tokens.
They are not filesystem paths, PIDs, cgroup names, or authorization claims.

## Host interfaces

Host Network and Device Providers derive interface names from immutable Zone
and Resource identity. Operators must not select a private interface name or
reconstruct it from a Guest name. The broker refuses a collision, an overlong
name, or a foreign ownership marker.

The current public inspection path is:

```bash
d2b host check --json
d2b guest status <name> --zone <zone> --json
```

These outputs contain bounded mapping and status data only; they do not expose
private locators, credentials, namespace IDs, or raw host paths.

## Historical names

Older Realm, environment, VM-first, Gateway-daemon, and per-workload unit
names remain in ADRs, migration notes, and compatibility fixtures. They are
historical classifications, not current ResourceRefs or configuration paths.

## Related docs

- [Zone and Volume Nix authoring](./zone-volume-nix.md)
- [Zone CLI contract](./zone-cli-contract.md)
- [CLI contract](./cli-contract.md)
- [Host preparation](../how-to/host-prepare.md)
- [AGENTS.md](../../AGENTS.md)
