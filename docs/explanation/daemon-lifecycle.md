# Daemon lifecycle

d2bd supervises the current Zone resource plane. It owns Zone runtime
reconciliation and observes the committed rows a Guest's lifecycle depends on:
its Process, Endpoint, Volume, Network, Device, Credential, and binding rows,
the Provider rows that assign them, and session status. `d2b-broker` performs
the approved host mutations and is the sole parent and reaper for
broker-spawned runners.

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
Process, Endpoint, Volume, Network, Device, Credential, Provider, and binding
controllers remain the effect owners.

A semantic controller owns the shape of its own child graph. The display
family is the worked case: one admitted `WaylandSession` derives its worker
`Process` rows and their private `Endpoint` rows from its own row identity and
spec, through its own durable derivation, and the manager turns those intents
into child rows. `d2bd` authors none of them and keeps no copy of the
vocabulary - not the worker templates, not the endpoint shapes, not the
restart annotation. The daemon supplies only the child-intent source that
hands the Provider's derivation to the manager, and the admission vocabulary
the `Endpoint` family classifies against, so an endpoint shape is admitted
only because the Provider that derived it commits that exact spec. A
look-alike differing on any one structural field is a terminal refusal, not
an admission with a warning.

Readiness is published by the row's own actor, never stamped on its behalf.
An `Endpoint` actor publishes that endpoint's readiness and the realization
incarnation it proved; the `EndpointBinding` actor publishes that
relationship's delivery state; the `Process` actor publishes its own. No row's
status is written by anything but its owner, and nothing writes one durably:
each transition is fenced on the row generation its actor holds and lives in
that actor until the next one replaces it. A host socket realization is
daemon state, so what crosses the boundary is an opaque incarnation handle and
a closed connectability state - never a path, a device/inode pair, or a host
error - and the handle rotates when the socket behind the endpoint is
replaced.

A binding is committed, not inlined. A consumer publishes a typed binding
request, the source provider admits it against its own decision, the
`Role` and `RoleBinding` authorization evidence, and the realization its
selected backend declares, and the admitted relationship is committed as a row
of its own type - `VolumeBinding`, `DeviceBinding`, `EndpointBinding`,
`NetworkBinding`, or `CredentialBinding` - that a dedicated Provider serves
and releases.

Calls are declared rather than scripted. An `Operation` row names a declared
`Provider` method or a provider-owned executable template, never a command
row, host path, or argv. `Role` is authorization-only: bounded rules plus the
`Operation` rows the holder may create, with no posture, mount, or command
facet. `ExecutionPolicy` and `SeccompProfile` state confinement and the
syscall filter respectively; neither grants resource access, which only an
admitted binding row carries.

## Readiness

A Guest remains `Pending` until the current-generation dependency graph is
observable:

- required Provider assignments and artifact commitments are current;
- host-side Process, Endpoint, Volume, Network, and Device resources are
  Ready, and the binding rows their sources derived are ensured, with the
  ones no longer derived retired;
- the VMM Process is current and running;
- the private Guest-control Endpoint is connected; and
- the authenticated ComponentSession and any target-local seed Resources are
  ready.

Readiness for a mediated endpoint is a delivery, not a status something
asserted. A `Process` that needs one derives the relationships it requires
from each `Endpoint` row's own publication intent, then requires that exact
`Endpoint` to be Ready and that exact `EndpointBinding` to be `Delivered` at
one matching realization incarnation on both sides. Endpoint readiness for one
incarnation paired with a delivery of another is not readiness: the launch
defers. The sealed authority the gate derives from that evidence is
revalidated immediately before every launch and every adoption, so an
authorization or endpoint change in the window fails the effect closed rather
than starting a helper over access that no longer holds.

Losing a relationship is an authority change, not a not-yet. A revoked,
replaced, draining, or undelivered binding stops the live helper rather than
only deferring the next launch: the row stops reading ready in the same pass,
and nothing relaunches until the gate opens again against fresh evidence.
Evidence that cannot be interpreted at all, or evidence naming a relationship
this launch does not expect, is terminal instead, because retrying the same
evidence cannot change either answer.

An endpoint reached from inside a Guest crosses to it over the authenticated
target-control session rather than a host path, and that session's generation
fences every frame; the prepared bindings travel to the target only after the
lease revalidates, carrying the same opaque incarnation tokens rather than any
socket name or path.

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

Teardown is relationship-first, and it never reports a success it did not
prove. The durable deleting mark is the fence: it commits before any cleanup
runs, so nothing new is admitted against the row while its subtree comes
down. Stopping the live `Process` effect is deliberately separate from
retiring the `Process` row, so a revoke can still name the consumer it is
revoking from. A relationship then retires only on positive evidence - a
revoke the broker actually performed, or proof that no grant was ever
standing for that exact consumer and socket. The broker does not separate
those two cases yet: a missing endpoint and an already-removed entry reach
the caller as one closed refusal class rather than as the distinct no-grant
answer the relationship reads, so today only a performed revoke converges and
the no-grant branch is not reachable end to end. An unreadable row, a missing
parent or consumer, or a broker that did not answer proves nothing at all;
the relationship retains its ownership and the row above it waits.

Rows retire leaf-first: `Endpoint` and `Process` rows go before the session
row that owns them, and a parent's row - deleting mark included - is held
until its last child has retired, so no owner ever disappears ahead of what
it owns. A display session's own cleanup adds one more ordering constraint:
it revokes the session's admitted endpoint access before it stops either
worker, so no helper is stopped and no child retires while that access is
still standing.

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
- [ADR 0055](../adr/0055-unified-resource-graph-bindings-operations-and-authority.md)
