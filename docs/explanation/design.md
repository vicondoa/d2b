# d2b design

**Diataxis category:** explanation.

d2b is a single-user NixOS Wayland framework for running untrusted workloads
inside isolated Guests while keeping one trusted host desktop. The active
product model is **Zone and Zone-owned Resources**. A Zone is the boundary for
identity, policy, routing, state, audit, and all resources it owns.

## Trust model

The host kernel, NixOS configuration, compositor, d2b daemon, and broker are
trusted. Guest workloads, their files, their network peers, and external
Provider inputs are not. d2b reduces exposure; it does not make a compromised
host safe and it is not a multi-tenant security OS.

The public CLI and daemon socket expose only typed, bounded, redacted
information. Private bundle data, credentials, host paths, runtime locators,
pidfds, cgroup paths, and namespace identifiers remain behind the daemon and
broker boundaries.

## Control-plane ownership

```text
Nix compiler
  -> Zone and semantic Resource bundle
  -> d2bd
  -> Guest controller
  -> specialized Resource controllers
  -> d2b-broker
  -> audited host effects
```

Nix declares:

- Zone topology and trusted publisher roots;
- Guest, Provider, Process, Endpoint, Volume, Network, Device, Credential,
  and policy specifications; and
- immutable Guest system and Provider artifacts.

The Guest controller derives its direct child Resource graph from the Guest
name, Provider contract, and signed private setup descriptor. It may create a
related graph before child UIDs exist because relationships use deterministic
name-based ResourceRefs. Returned UIDs, generations, and revisions fence
incarnation and adoption.

The Guest controller does not spawn, mount, provision, bind, or call the
broker. Process, Endpoint, Volume, Network, Device, Credential, Provider, and
binding controllers are the effect owners. The broker accepts only typed
operations after the daemon has committed and authorized the corresponding
Resource intent.

## Bindings

A binding is a relationship between a source resource - a Volume, Device,
Network, Endpoint, or Credential - and one consumer component. Those five
kinds are a closed vocabulary, and each is authored in two halves that never
merge into one permission bag.

The consumer publishes a typed binding *request*: which source, which rights,
which stable consumer slot, and where it intends to use it. A request is not
authority. It names an exact typed reference and a bounded destination, never
a raw host path, a numerical host principal, secret material, or a free-form
command line.

The source provider admits that request against its own decision, against
authorization evidence produced by the `Role` and `RoleBinding` contracts, and
against the realization its selected backend declares it can enforce. The
admission names the exact dependency revisions it was evaluated against, so an
ownership, view, consumer, or provider-assignment change that did not advance a
generation still invalidates the earlier use.

The admitted relationship is then committed as a row of its own served
ResourceType - `VolumeBinding`, `DeviceBinding`, `EndpointBinding`,
`NetworkBinding`, or `CredentialBinding`. A dedicated Provider serves that
type: it registers the driver, decodes the spec, and reconciles the row into
the actual host, device, network, or credential realization, releasing it on
finalize or delete. A source ensures every binding row it derived and retires
the ones it no longer derives; its readiness reads the child rows' phases
rather than overriding them, so a relationship that never committed is not
reported as served.

Each source declares the closed set of presentation facets its selected
backend can realize. A filesystem presentation, a consumer device slot, a
device attachment, a namespace interface, a shared fabric, an endpoint
descriptor, an endpoint pathname, and credential delivery are the whole
vocabulary. A presentation the backend cannot enforce is refused with a named
reason: an unapplied mount, an unclaimed device, and an unproven endpoint are
not successes.

## Execution and authority

An `Operation` row is one externally callable declaration, and its
implementation is a declared identity rather than a row. It names either a
method of a declared `Provider` component or a trusted executable template
that Provider owns. A caller does not choose code, and a provider resource
that names an untrusted artifact cannot introduce a compiled privileged
handler; compiling a declared implementation into a callable handler is the
deployment's job.

Confinement is stated separately from access. An `ExecutionPolicy` row states
what an execution instance is allowed to be and nothing more: the namespaces
it runs under, its capability ceiling, whether new privileges are forbidden,
the identity it may resolve to, its root-filesystem restrictions, the syscall
filter it must load, and its umask. A `SeccompProfile` row states the syscall
allowlist and its default action, and nothing else. Neither grants storage,
device, network, endpoint, or credential access; those are separate typed
binding relationships, and a policy that named them would be a second
authority. Composition is field-wise and a conflict is a refusal, never a
silently relaxed intersection.

`Role` is authorization-only. It states which resource verbs a subject may
use and which declared `Operation` rows its holder may create. It carries no
posture, mount, or command facet, and a row that still carries one is rejected
rather than decoded with its authority dropped. Selecting an `ExecutionPolicy`
is itself a request: without `Role` and `RoleBinding` evidence the selection
is refused even when every other rule would admit it.

## Guest lifecycle

A Guest is `Pending` until its current-generation dependencies are ready:

1. Provider assignment and artifact commitments are valid;
2. required host-side resources report Ready;
3. the VMM Process and private Endpoints are current;
4. the authenticated Guest-control ComponentSession is established; and
5. target-local Guest resources have been seeded and observed.

Session loss is a typed degraded state. It revokes session-bound seed and
relay authority, preserves identity, and allows reconnect by revision. A
restart relists the committed Zone graph and adopts only matching immutable
identity; stale or uncertain state is quarantined rather than guessed.

Deletion drains in reverse dependency order. The Guest finalizer clears only
after the authenticated session is closed, all owned descendants are absent,
and no uncertain broker or Provider state can mutate the old incarnation.

## Identity and isolation

ResourceRefs are Zone-local addresses such as `Guest/work-app`. A Guest name
does not identify a host process, socket, cgroup, or credential. Private
runtime identity is derived from immutable Zone and Guest UIDs and Provider
generations, so same-named Guests in different Zones cannot collide.

One relationship's identity is its Zone, source, consumer, binding kind, and
stable consumer slot. Rights, destination, and presentation are deliberately
outside that identity, so changing what a consumer asks for updates the
existing relationship instead of minting a second one beside it.

Every Resource mutation carries the exact owner, UID, generation, revision,
assignment, and session evidence required by the current controller. Caller
claims are never accepted as authoritative subject identity.

## Store and host state

Each Guest receives a closure-only `/nix/store` view. The host's complete
store is never exposed to a Guest. Store synchronization uses anchored paths,
OFD locks, explicit fd transfer, restart adoption before cleanup, typed
degraded state, and one named repair owner per mutable path.

Host NetworkManager, nftables, systemd-networkd, cgroup, TPM, socket, and
device state is changed only under d2b's ownership marker. Foreign or replaced
state fails closed and remains byte-for-byte untouched.

## Broker and runner security

The framework declares exactly three root-visible units:

```text
d2bd.service
d2b-broker.socket
d2b-broker.service
```

The broker resolves private paths and credentials, verifies signed Provider
templates, places runners in delegated cgroup leaves, hands pidfds over
`SCM_RIGHTS`, and reaps its children. d2bd observes and reconciles; it is not
the parent of broker-spawned runners.

Raw PID handling is restricted to the adoption check for a persisted
`(pid, start_time_ticks)` pair. After a pidfd is reopened, signal delivery
uses the pidfd. No public request can choose a PID, cgroup, namespace, host
path, or broker handle.

## Gateway-backed isolation

When a Zone uses gateway-backed transport, the Gateway Guest is an execution
context, not a second d2b control plane. Gateway credentials, remote
registries, Provider configuration, and Zone audit stay inside that Guest.
Relay identity is not local authorization. Separate Zones do not share a
Gateway Guest or L2 bridge.

## Provider and Guest contracts

Provider manifests publish closed placement, target-kind, component-artifact,
and effect-class contracts. A Provider can create transitive resources only
through its assigned controller and authenticated EffectPort. Provider
configuration is semantic and private; it never places argv, raw locators, or
secrets in a public Resource spec.

Guest-local resources are submitted only after authenticated session
establishment. The target-local API receives bounded semantic data and
revision cursors, not host credentials or Nix store paths.

## UI and desktop integration

Wayland, audio, clipboard, graphics, video, USBIP, security-key, and shell
features are Provider projections. They use Zone and Resource identity and
return bounded capability/enforcement status. Presentation metadata is not
authorization and must remain usable when an optional artifact is absent.

## Failure posture

d2b fails closed on:

- missing or stale owner, UID, generation, revision, or session evidence;
- unsigned or artifact-mismatched Provider descriptors;
- foreign ownership markers or replaced persistent state;
- invalid ResourceRefs, unbounded fields, raw paths, credentials, argv, or
  numeric host identifiers; and
- uncertain cleanup where complete drain cannot be proven.

Typed degraded states are preferable to unsafe fallback. There is no legacy
shell, SSH, static process graph, name-only runtime lookup, or compatibility
service that can satisfy a current Guest lifecycle request.

## Historical context

Older ADRs and migration notes describe the retired Realm, environment,
VM-first, and Gateway-daemon models. They explain why the current ownership
boundaries exist but are not current configuration instructions. The current
code, Zone references, and generated contracts are authoritative.

## Related docs

- [`../../README.md`](../../README.md)
- [`../reference/zone-control-nix.md`](../reference/zone-control-nix.md)
- [`../reference/zone-cli-contract.md`](../reference/zone-cli-contract.md)
- [`daemon-lifecycle.md`](./daemon-lifecycle.md)
- [`../adr/0055-unified-resource-graph-bindings-operations-and-authority.md`](../adr/0055-unified-resource-graph-bindings-operations-and-authority.md)
- [`../contributing/critical-subsystems.md`](../contributing/critical-subsystems.md)
