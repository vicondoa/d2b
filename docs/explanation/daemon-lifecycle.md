# Daemon lifecycle

d2bd supervises the current Zone resource plane. It owns Zone runtime
reconciliation and observes Guest, Process, Endpoint, Volume, Provider, and
session status; `d2b-broker` performs the approved host mutations and is the
sole parent and reaper for broker-spawned runners.

## Control-plane ownership

```text
Nix -> Zone bundle -> d2bd -> Guest controller
                         |       |
                         |       +-> child Resource controllers
                         +-> d2b-broker -> host effects
```

Nix authors a Guest's semantic spec, selected immutable artifacts, and
compiler-only Zone topology. It does not author the Guest's controller-owned
child graph. The Guest controller derives deterministic child ResourceRefs,
uses name-based addresses before UIDs exist, and fences every mutation with
the Guest UID, child UID, generation, and revision.

The Guest controller never spawns a process, mounts storage, binds a socket,
provisions a device, handles credentials, or calls the broker directly.
Process, Endpoint, Volume, Network, Device, Credential, and Provider
controllers remain the effect owners.

## Readiness

A Guest remains `Pending` until the current-generation dependency graph is
observable:

- required Provider assignments and artifact commitments are current;
- host-side Process, Endpoint, Volume, Network, and Device resources are
  Ready;
- the VMM Process is current and running;
- the private Guest-control Endpoint is connected; and
- the authenticated ComponentSession and any target-local seed Resources are
  ready.

Session loss is a typed degraded state. It revokes session-bound seed and
relay authority, preserves the Guest identity, and permits reconnect by
revision. The daemon does not hide an unavailable dependency by starting a
duplicate process or consulting a static manifest.

## Start, stop, and restart

The public operations are:

```text
d2b guest start <name> --zone <zone> --apply
d2b guest stop <name> --zone <zone> --apply
d2b guest restart <name> --zone <zone> --apply
```

Start and restart reconcile the desired child set idempotently before
requesting Process start. Stop first closes admissions and the authenticated
Guest session, then drains children in reverse dependency order. A Guest
finalizer clears only after owned descendants are absent and no uncertain
broker or session state can still mutate the incarnation. `--force` changes
only the provider-aware graceful wait; it does not bypass ownership,
generation, or finalizer checks.

## Supervisor and broker boundary

d2bd sends typed broker requests for approved Process effects and receives
pidfds or bounded status evidence. The broker:

1. resolves private runtime identity from immutable Zone and Guest identity;
2. verifies the signed Provider/template and resource commitments;
3. places the runner in its delegated cgroup leaf;
4. returns the pidfd over the broker socket; and
5. remains the sole reaper for the spawned child.

Raw PIDs, argv, host paths, credentials, namespace IDs, and cgroup paths are
not public lifecycle inputs. Reconciliation may compare a persisted
`(pid, start_time_ticks)` pair while reopening a pidfd, but signal delivery
then uses the pidfd exclusively.

## Root-visible units

d2b declares exactly:

```text
d2bd.service
d2b-broker.socket
d2b-broker.service
```

There are no framework-owned per-Guest systemd units, host-singleton
lifecycle services, or shell fallback wrappers. A manual `d2bd.service`
restart is a continuation event: the daemon rebinds the public socket,
adopts structurally valid current runners, quarantines stale identity, and
reports readiness only after the control plane is usable.

## Restart adoption

On restart, d2bd relists the current Zone resources and private broker
observations before cleanup:

- matching immutable identity is adopted;
- a PID/start-time mismatch is quarantined and never controlled;
- a missing runner is reconciled from desired Resource state; and
- uncertain broker responses are retried or held in a typed degraded state.

Adoption is identity-first, not name-first. A same-named Guest reincarnation
cannot inherit an older Guest's Process, Endpoint, session, credential, or
broker scope.

## Deletion and repair

Deletion is dependency-ordered and status-first. The controller requests
child deletion, waits for transitive descendants and Provider finalizers, and
retains `FinalizationBlocked` when proof is incomplete. A single named repair
owner controls each host-mutable path or lock surface; foreign ownership
markers fail closed and are never overwritten.

The broker owns delegated cgroup mutation, pidfd reaping, host socket/device
access, and typed cleanup. d2bd does not sweep `/run/d2b`, change ownership
recursively, or perform an unscoped host cleanup.

## Inspection

```text
d2b guest status <name> --zone <zone>
d2b process list --zone <zone>
d2b endpoint list --zone <zone>
d2b host doctor --read-only
d2b op inspect --json
```

These views report bounded status, generation, revision, capability, and
degraded-state metadata. They do not expose private runtime scope or broker
credentials.

## References

- [Zone CLI contract](../reference/zone-cli-contract.md)
- [Manifest bundle](../reference/manifest-bundle.md)
- [Storage lifecycle](../reference/store-lifecycle.md)
- [ADR 0015](../adr/0015-daemon-only-clean-break.md)
- [ADR 0034](../adr/0034-storage-lifecycle-restart-and-synchronization.md)
