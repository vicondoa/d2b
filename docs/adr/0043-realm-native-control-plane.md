# ADR 0043: Realm-native control plane

- Status: Accepted (Unreleased)
- Date: 2026-07-05
- Supersedes: [ADR 0032](0032-d2b-v2-constellation-control-plane.md)
- Related: [ADR 0002](0002-non-root-daemon-and-privileged-broker.md)
  (non-root daemon and privileged broker), [ADR 0010](0010-wire-protocol-and-typed-errors.md)
  (wire protocol and typed errors), [ADR 0015](0015-daemon-only-clean-break.md)
  (daemon-only clean break), [ADR 0025](0025-wayland-proxy-host-jailed-role.md)
  (host-jailed Wayland filter proxy role), [ADR 0028](0028-guest-control-plane-over-vsock.md)
  (guest control plane over virtio-vsock), [ADR 0034](0034-storage-lifecycle-restart-and-synchronization.md)
  (storage lifecycle, restart adoption, and synchronization), [ADR 0037](0037-local-hypervisor-runtime-seam.md)
  (local hypervisor runtime seam), [ADR 0038](0038-persistent-guest-shell-sessions.md)
  (persistent named guest shell sessions), [ADR 0039](0039-constellation-persistent-shell-routing.md)
  (constellation persistent shell routing), [ADR 0042](0042-d2b-clipboard-authority-and-picker-split.md)
  (clipboard authority and picker split)

## Context

D2b's local desktop VM experience is strong: a local `d2b` CLI talks to a
local `d2bd`, `d2bd` owns the VM lifecycle DAG, privileged host mutation
flows through `d2b-priv-broker`, guest control is typed, and Wayland
windows run through d2b-owned local virtualization surfaces.

The constellation/realm model is less coherent. [ADR 0032](0032-d2b-v2-constellation-control-plane.md)
correctly introduced semantic daemon-to-daemon operations, capability
negotiation, remote full-host nodes, provider adapters, and relay-agnostic
transport boundaries. It also intentionally avoided raw broker, daemon,
guest-control, SSH, and generic network tunnels. Those invariants remain sound.

The weak point is the ownership model. [ADR 0032](0032-d2b-v2-constellation-control-plane.md)
treats realms as an entrypoint table in front of a host daemon, with
host-resident named realms and gateway-backed realms as different dispatch
modes. That made sense as an
incremental extension from a single-host framework, but it leaves the core
abstraction backwards: d2b is still modeled as a host daemon with some realm
metadata, not as a collection of realm control planes.

That is the wrong long-term shape for:

- local `home`, `dev`, and `work` boundaries on the same computer;
- provider-managed sandboxes such as Azure Container Apps;
- future cloud/full-host realm types such as Azure VM hosts running d2b;
- nested "teleport" realms started from any host or VM capable of running d2b;
- cross-realm Wayland, exec, persistent shell, lifecycle, logs, clipboard, and
  other capability-gated operations.

This ADR selects a realm-native model. It preserves [ADR 0032](0032-d2b-v2-constellation-control-plane.md)'s
semantic protocol posture, but replaces the host-centric entrypoint model with
a stricter invariant: **a realm boundary is a realm controller instance**.

## Decision

D2b will become a collection of realms. A **realm** is the administrative
trust boundary and policy domain for a set of VM, sandbox, or provider-backed
workloads. Each active realm boundary is controlled by its own `d2bd`
instance and its own privileged-broker, socket, state, and audit boundary
where host mutation is available. Host-local realm brokers do not claim
unpartitioned host-global resources directly; a local root host-resource
allocator assigns non-overlapping resources before delegation, and each realm
broker mutates only its delegated partition.

The local host no longer owns every realm as one host-global daemon with a
realm dispatch table. Instead, the local machine has a minimal local
root/anchor realm, and named local realms such as `home`, `dev`, and `work`
are peer realm instances below that root. A sensitive realm such as
`work` may run directly on the host as an isolated realm service; it does not
need a gateway VM solely to become a separate realm boundary.

Gateway VMs remain valid deployment locations for realm controllers, but they
are no longer the public abstraction. The public Nix surface moves from
`d2b.gateways` to `d2b.realms`.

Providers and runtimes sit below a realm controller. Cloud Hypervisor, crosvm,
libkrun, qemu-media, Windows hypervisor, Azure VM full hosts, ACA sandboxes,
and future providers implement the standard d2b semantic contract and
advertise positive capabilities. They do not define provider-specific d2b
protocol forks.

The realm-native cutover is a clean architectural break from the old realm and
ACA sandbox surfaces. Existing local Cloud Hypervisor VMs and ACA sandboxes are
migrated into `d2b.realms` and the shared realm protocol; old ACA sandbox
contracts, gateway-shaped realm functionality, and legacy realm entrypoint
surfaces are removed rather than carried as compatibility modes.

Realm declarations also replace the current user-facing grouping abstraction.
The canonical first local realms are `home`, `dev`, and `work`; VMs move into
realms according to their current group membership or an explicit operator
mapping. Network/env declarations may remain as realm-owned substrate for
bridges, address allocation, and isolation, but they are no longer the
user-facing trust-boundary model. Until the implementation waves land,
`d2b.envs` remains the active configuration key; this ADR defines the target
cutover, not an already-shipped module surface.

Relay infrastructure is a discovery and byte-transport substrate only. Relay
identity never authorizes d2b operations. Every cross-realm operation uses
end-to-end realm identity, session authentication, capability negotiation,
operation/stream policy, idempotency, and bounded audit above the relay.

The operator-facing CLI and desktop tools talk to the **realm layer**, not to a
single implied local `d2bd`. The implementation may keep a local-root realm
access socket as the default entrypoint, but that entrypoint is a resolver, not
a byte proxy for host-local realms. It resolves the target realm, checks local
alias rules, and returns the target realm's access binding. For a host-local
realm, clients connect directly to that realm's Unix socket so OS DAC,
`SO_PEERCRED`, `SCM_RIGHTS`, and local FD-passing semantics are preserved. If an
implementation must bridge an accepted local connection, it must pass the
client connection or equivalent authenticated identity by fd handoff; it must
not proxy bytes in a way that makes the target realm authenticate the local root
instead of the original operator.

For remote or provider-backed realms, the local-root resolver may initiate the
approved realm-tree transport because those paths already use semantic
operation/stream frames and realm-session authentication. The resolver still
does not collapse all realms back into one host daemon or bypass the target
realm's own authorization, broker, state, and audit boundary.

This should be mostly transparent to operators. Existing local commands remain
short through a default realm or explicit aliases, while cross-realm operations
use the realm-qualified `*.d2b` target names. The user-visible change is target
naming and clearer capability denials, not a second CLI or separate
realm-specific toolchain.

## Realm model

A realm has:

- a stable `RealmId`;
- a `RealmPath` in a strict parent/child tree;
- a realm identity key;
- one active realm controller generation;
- policy for local workloads and descendants;
- a node/workload/provider registry owned by that realm controller;
- a capability set derived from the controller and its providers;
- an audit domain;
- configured parent and child trust edges.

Every realm and every ancestor up to the root can constrain nested children.
A creation, route, operation, or stream is allowed only when the applicable
policy chain permits it. A child realm cannot bypass its parent by advertising
an alternate peer route; the initial routing model is tree-only.

### Local root and peer realms

On a local machine, the minimal root realm anchors local operator access,
realm discovery, and peer realm registration. It is not where work/dev
policy collapses.

Example local topology:

```text
local-root
  ├── home
  ├── dev
  └── work
        └── payments
```

`home`, `dev`, and `work` are separate realm instances. Each has a
separate `d2bd` instance and, when it can mutate the host substrate, a separate
broker/socket/state/audit boundary. The root realm can route to the peers
according to policy, but it does not read their provider credentials or become
their lifecycle authority.

### Realm controller placements

The same realm contract can be implemented in different placements:

| Placement | Runs where | Authority |
| --- | --- | --- |
| `host-local` | Isolated service on the physical host | May own local VM/provider lifecycle through that realm's broker. |
| `gateway-vm` | Dedicated local d2b VM | Owns realm policy/credentials inside a microVM boundary. |
| `cloud-full-host` | Cloud VM running full d2b | Owns its local broker, runtimes, and guest-control stack. |
| `provider-controller` | Provider-supported environment | Implements the standard d2b realm protocol with a limited capability set. |
| `provider-agent` | Inside or adjacent to a managed sandbox | Implements workload/stream subsets when a full `d2bd` cannot run. |

The architectural preference is shared contracts and shared daemon
implementation surfaces wherever possible. A constrained provider environment
may expose only a subset of the contract, but the subset is described by
capability advertisement, not by a different protocol.

## Process and broker model

Each realm boundary has a distinct control-plane identity:

```text
realm d2bd
  -> realm public socket
  -> realm broker socket
  -> realm state directory
  -> realm audit log
  -> realm provider/routing configuration
```

For host-local realms, this means multiple `d2bd` and broker instances may run
on one physical host. They must not share a host-global privileged broker.
They also must not race over the same global kernel/filesystem objects. The
local root owns host-resource allocation, while each realm broker owns only the
partition delegated to that realm.

The implementation must define:

- deterministic unit/socket names;
- distinct system users/groups for each host-local realm and OS DAC on each
  realm public and broker socket; the local root realm is not in the critical
  path for authenticating to a peer realm's local socket;
- per-realm state and audit paths;
- credential paths readable only by the owning realm daemon user/group;
- a dedicated systemd slice and delegated cgroup subtree for each host-local
  realm, with no cross-realm cgroup mutation;
- deterministic TAP, bridge, veth, subnet, and interface-name allocation with
  NixOS assertions for declared conflicts and runtime fail-closed checks for
  undeclared drift; generated interface names must fit Linux `IFNAMSIZ`
  limits by using bounded hash-derived names rather than raw realm ids;
- realm-specific nftables tables or a root-owned allocator/lock that guarantees
  atomic updates without one realm flushing another realm's rules;
- NetworkManager, sysctl, and `/etc/hosts` ownership partitions serialized
  through the local root allocator rather than ad hoc per-realm file writes;
- fail-closed startup when two realms claim the same exclusive host resource.

Any host-local realm that claims network isolation must run its network-facing
broker work in a dedicated network namespace connected to the local root through
an explicitly declared veth/bridge boundary. A default-network-namespace mode
may exist only for non-isolated diagnostics; it provides no
network isolation, cannot safely partition network sysctls, and must not be
presented as equivalent to a dedicated namespace.

Host-resource arbitration uses an explicit local-root allocator API exposed on
a root-owned Unix socket. Realm brokers request typed leases for global host
resources and receive only their delegated names, file descriptors, or
partition ids. The allocator persists leases in the local root state directory,
but kernel state is the source of truth on restart: the allocator reconciles
persisted leases against netlink-visible interfaces/bridges and nftables API
state before reusing, deleting, or reassigning resources. It uses a total
acquisition order for multi-resource allocations and serializes shared
host-file updates itself. Realm brokers must not independently claim
unallocated global resources with best-effort filesystem locks.

The local-root allocator or host systemd/NixOS unit creates cross-namespace
network plumbing. It creates the veth pair, attaches the root-side endpoint to
the selected host bridge or namespace boundary, moves the realm-side endpoint
into the realm network namespace, then delegates only the realm-side resources
to the realm broker, and brings up required namespace-local plumbing such as
loopback. Realm brokers confined to their own network namespace are not
responsible for mutating the root namespace.

Allocator leases are tied to the realm broker lifecycle with pidfd tracking,
Unix-socket disconnects, or an equivalent identity-bound liveness mechanism.
Unexpected broker death triggers bounded reclamation or quarantine of
non-FD-backed resources. Realm-specific nftables tables with non-colliding
names may be updated directly by the owning realm broker through atomic
nftables/netlink transactions; the local-root allocator serializes only shared
or cross-table resources.

The existing [ADR 0015](0015-daemon-only-clean-break.md) daemon-only invariant
remains: per-VM work lives inside the owning realm daemon's lifecycle executor,
and privileged effects go through that realm's broker. This ADR changes the
unit of ownership from "one host daemon" to "one realm daemon".

## Addressing and target resolution

The public operator address names a workload inside a realm, not a physical
node:

```text
<vm>.<realm>[.<ancestor>...].d2b
```

Examples:

```text
builder.dev.d2b
browser.work.d2b
api.payments.work.d2b
build.teleport.work.d2b
```

VM names are unique within a realm. The realm controller resolves the
workload's current provider, host, node, or sandbox placement internally.
Physical node labels are not part of the normal operator address. The first
label is always the VM/workload name; every remaining label before `.d2b` is
the realm path, written leaf-to-root. Workload and realm labels are dotless
d2b labels (`^[a-z][a-z0-9-]*$`), so parsing is deterministic. If an
implementation needs a diagnostic node-qualified form, it must be a separate
diagnostic surface and must not become the default public target grammar.

Bare VM names remain convenience aliases only within a statically configured
default realm or explicit local alias table. Dynamic relay discovery never
changes the meaning of a bare target. If a bare alias conflicts with another
configured local alias, the command fails closed with a helpful error listing
the conflicting canonical addresses. The resolver also rejects unqualified
targets that collide with any configured/running local realm workload visible to
the local root, even when one match is in the default realm. Remote relay
discovery does not participate in bare-name lookup.

```text
d2b vm up dev
d2b vm exec browser.work.d2b -- hostname
d2b shell api.payments.work.d2b
```

Inventory and machine-readable output should include the canonical address and
the resolved placement metadata separately:

```text
ADDRESS                    REALM          PLACEMENT         STATE
builder.dev.d2b            dev            host-local        running
browser.work.d2b           work           host-local        running
api.payments.work.d2b      work/payments  azure-vm:build    running
session.aca.work.d2b       work           aca:sandbox       running
```

The CLI parser, shell clients, and desktop helpers must all consume this same
target model. They must not scrape human output, infer the current VM from a
single local daemon, or assume a local socket path uniquely identifies the
target realm. Machine-readable APIs expose canonical realm addresses, resolved
placement metadata, access bindings, and capability preflight status so tools
can remain transparent when a VM moves from local host placement to a
remote/provider placement inside the same realm.

## Trust bootstrap and identity

Every realm has a long-lived realm identity key. That key signs
per-controller/per-generation keys used for runtime sessions. This lets a
realm rotate or revoke controller session keys without changing the realm
identity itself.

Bootstrap is provider-specific, but one invariant is common: the parent realm's
public key is the trust anchor. A supported deployment method must install,
inject, or otherwise bind the parent public key into the child realm before the
child can join. The first invite/enrollment exchange may return or bind the
child realm public key.

Examples:

- a local host-created realm receives the local root public key through the
  host-local realm provisioning path;
- an Azure VM realm receives the parent public key through cloud-init, custom
  data, image metadata, or a provider-supported secret injection path;
- an ACA sandbox realm receives the parent public key through its image or
  workload identity bootstrap;
- a nested realm started inside a VM receives the parent key through the
  parent realm's approved provisioning flow.

Relay credentials are reachability credentials only. They do not become realm
identity, daemon admin identity, or local broker authorization.

The ADR implementation plan must include:

- realm identity key generation;
- controller-generation key issuance;
- enrollment records;
- parent/child key pinning;
- rotation of realm keys and controller-generation keys;
- revocation and session teardown;
- short-lived controller-generation credentials or active parent confirmation
  at session establishment, plus a parent-pushed revocation list through the
  routing tree for early invalidation;
- audit records for enrollment, rotation, and revocation;
- recovery flow for a lost or compromised child controller key.

Revocation has immediate runtime effects. When a realm key, controller
generation, policy grant, or stream capability is revoked, active operation and
stream sessions depending on that grant are forcefully terminated and audited.
Routine controller-generation rotation that preserves the same valid policy
grant may rekey without disrupting active streams.

## Discovery and routing

Dynamic relay discovery is part of the initial architecture. A parent and child
realm discover each other by broadcasting and replying over the supported d2b
relay protocol, but discovery is not authorization. A discovered child is
admitted only after key binding, policy checks, capability negotiation, and
replay-protected session establishment succeed.

Routing is a strict parent/child tree:

- a realm has one parent, except the local/root realm;
- a child may have children;
- route advertisements describe only descendants below the advertising realm;
- a parent validates every child advertisement against the namespace allocated
  to that child, so a child cannot advertise a sibling, parent, or unrelated
  realm path;
- route advertisements are bounded, signed by the advertising realm
  controller generation, and expire;
- loops and multi-parent routes fail closed;
- a realm does not select among arbitrary peer/DAG routes.

Discovery, enrollment, and route advertisements are pre-authentication attack
surfaces until the realm key is verified. Implementations must apply memory
bounds, per-relay and per-unverified-peer rate limits, replay windows, and a
drop-new policy when unauthenticated queues are full. Unverified relay peer
identity, raw advertisement data, and relay endpoint strings must not become
metric labels.

When a source realm addresses a target in a different branch, messages travel
through the nearest common ancestor permitted by policy, then down the target
branch. Direct transport shortcuts may be used only when the same parent/child
route is authorized and both ancestors' policies permit that direct stream.
The shortcut does not create a new trust edge or DAG route. A direct shortcut
relies only on native underlay reachability already available to both peers; it
must not introduce STUN/ICE, overlay routing, a VPN, or NAT-traversal machinery.
If the direct path is unavailable, the operation uses the authorized parent
relay path or fails with a typed transport error. The authorizing ancestor logs
shortcut establishment and teardown with the same bounded correlation id even
when stream bytes do not traverse that ancestor.

Example:

```text
local-root
  ├── dev
  └── work
        └── payments
```

`dev` can address `api.payments.work.d2b` only if:

1. `dev` policy allows the operation toward `work/payments`;
2. `local-root` policy allows that cross-branch operation;
3. `work` policy allows access to its child `payments`;
4. `payments` policy allows the target workload operation;
5. every route advertisement and session key in the path is current.

The protocol routes semantic d2b operation and stream frames. It does not
create a flat L3/L4 overlay, VPN, raw port-forward default, SSH fallback, raw
guest-control tunnel, raw broker tunnel, pidfd passing, or generic socket
proxy.

Every routed operation carries a bounded correlation id aligned with W3C Trace
Context so normal OpenTelemetry tooling can correlate realm hops without a
d2b-specific tracing format. Audit records may store only the fixed-size
`trace-id`, `span-id`, and d2b correlation id; `tracestate` is stripped or
strictly size-bounded and never copied wholesale. Each realm writes the same
correlation id to its own audit log so operators can reconstruct a route without
centralizing secret-bearing audit state. Route-decision audit records include
the bounded policy rule id that allowed or denied each branch traversal or
direct shortcut. Capability denials include the correlation id and missing
capability so the caller can give the target realm administrator actionable
context.

Each realm controller exposes low-cardinality health SLIs for API latency and
errors, discovery queue depth, drop-new counts, pre-auth rate-limit hits,
route-advertisement acceptance/denial counts, and revocation/session teardown
counts. Telemetry export uses explicit per-realm observability configuration:
either a realm-local secured metrics endpoint, OTLP export from the realm, or a
local-root scrape/forwarding path authorized for metrics only. The export path
must not grant access to realm provider credentials, raw audit payloads, or
control sockets. Desktop clients that initiate realm operations create or
propagate W3C Trace Context and log the correlation id locally through their
normal diagnostics channel before calling the realm access layer. Canonical
target addresses may appear in audit records, but metrics must not label on the
full `<vm>.<realm>...d2b` address; metric labels use bounded operation kind,
realm class, placement kind, and outcome enums. Static operator-declared realms
may contribute a bounded configured realm label only when the configured set is
known at startup; dynamically discovered, nested, provider-created, or ephemeral
realms must be rolled up to bounded classes rather than emitted as raw realm
paths. The local-root host-resource allocator also emits bounded audit records
and low-cardinality metrics for allocation grants, denials, conflicts,
reconciliation, reclamation, and quarantine decisions.

## Operation and stream contract

Cross-realm d2b operations use semantic operation frames and named authorized
streams. The current [ADR 0032](0032-d2b-v2-constellation-control-plane.md) /
[ADR 0039](0039-constellation-persistent-shell-routing.md) invariants remain:

- operation kind is typed and closed;
- required capability is derived from trusted code;
- mutating operations require idempotency keys;
- idempotency records are bounded by count and TTL, survive expected transport
  reconnects, and leave no unbounded in-memory replay surface;
- idempotency records for mutating host operations are durably persisted in the
  owning realm state directory, or the operation must use a provider-native
  idempotency primitive with equivalent replay protection;
- streams are opened only after an authorizing operation;
- stream kind and capability must match;
- missing capability returns typed denial, not fallback behavior;
- audit labels are bounded and metadata-only;
- payload bytes, argv, stdio, provider endpoints, host paths, relay
  credentials, and provider tokens are not audit or metric labels.

The architecture should pursue local d2b parity where feasible:

| Operation family | Realm-native target |
| --- | --- |
| Lifecycle | Core capability for realms that own workload lifecycle. |
| Exec | Core typed operation; provider API exec is allowed only as a capability-scoped implementation. |
| Persistent shell | Core where a guestd-compatible agent or full d2bd can provide [ADR 0038](0038-persistent-guest-shell-sessions.md) semantics. |
| Logs | Core bounded stream/summary capability. |
| Wayland/display | Core capability for desktop use, implemented through d2b-owned proxy/stream surfaces. |
| Clipboard | Capability-gated and must preserve [ADR 0042](0042-d2b-clipboard-authority-and-picker-split.md) picker/clipd authority. |
| File copy / ports | Capability-gated; no generic tunnel fallback. |
| USB/HID/audio/GPU | Capability-gated and may be unsupported across provider or realm boundaries. |

Some local features cannot cross every provider boundary safely. The correct
behavior is an explicit capability denial with actionable operator text, not a
best-effort fallback.

### Desktop and companion tools

Realm-native routing applies to every d2b-facing desktop tool, not just the
`d2b` binary:

- `d2b-wlcontrol` queries and acts through the realm access layer so realm,
  VM/window identity, capability status, and policy denials match CLI behavior.
- `d2b-clip-picker` and the trusted clipboard authority use canonical realm VM
  addresses in picker metadata, provenance labels, policy decisions, and audit
  records. Picker selection never implies direct host-to-VM clipboard access;
  [ADR 0042](0042-d2b-clipboard-authority-and-picker-split.md) authority still
  decides whether the selected transfer is allowed. Clipboard target lists
  include preflight capability status so denied destinations can be hidden,
  disabled, or explained before the operator selects them.
- `d2b-wlterm` resolves shell/session targets through the realm access layer and
  receives canonical target addresses from public APIs rather than assuming the
  local daemon's VM namespace.

These tools should preserve the local UX when a target is in the default realm,
but every user-visible object that can cross a realm boundary must carry or
derive the canonical `<vm>.<realm>[.<ancestor>...].d2b` identity. A tool that
does not understand realm-qualified identities must fail closed for
cross-realm targets rather than silently falling back to local-only behavior.
Canonical realm identities shown by Wayland, clipboard, and desktop tools are
asserted only by trusted d2b components such as the realm daemon, broker, or
Wayland/clipboard proxy. Guest-provided window titles, app ids, MIME
parameters, clipboard metadata, and other payload-derived labels must never be
accepted as authoritative realm or VM identity and must not become audit or
metric labels.

## Provider model

Providers implement standard d2b contracts below a realm controller.

### Local runtime providers

Cloud Hypervisor, crosvm, qemu-media, libkrun, and future Windows hypervisor
support are local runtime providers. They plan/start/stop/inspect workloads
for the owning realm daemon. They are not separate realm types by themselves.
The existing local Cloud Hypervisor path migrates into this provider model: CH
VMs are adopted by a realm controller, retain their local desktop fast path when
they live in the default/local realm, and no longer assume one host-global
daemon namespace.

### Cloud full-host providers

An Azure VM realm type provisions or adopts a VM capable of running d2b. The
provider may create the infrastructure and inject bootstrap material, but once
the VM boots, the cloud host runs its own realm controller and broker. Local
effects are re-originated inside that cloud host's d2bd/broker/guest-control
stack.

### Provider-managed sandboxes

An ACA-like provider may not have KVM, systemd, a broker, cgroups, or vsock.
It can still participate when it implements the standard d2b semantic
contract subset and advertises only the capabilities it can support. A
persistent-shell-capable sandbox must run a guestd-compatible d2b agent; a
provider-native one-shot command API is not a persistent shell.

Existing ACA sandbox support migrates into this model. Old ACA sandbox
contracts, provider-specific shell/exec behavior that bypasses the realm
operation model, and pre-realm ACA state layouts are not supported after the
realm-native cutover. Operators must recreate, re-enroll, or explicitly migrate
ACA sandboxes into a `d2b.realms` provider declaration with the new realm
identity and capability contract.

Because this is a clean cutover, rolling mixed-protocol operation is not a
goal. Old clients, old gateway routing formats, old ACA sandbox sessions, and
old realm entrypoint protocols fail closed with migration errors rather than
being drained through compatibility routing. Operators who need zero downtime
must stand up replacement realm-native workloads, validate them, and switch
traffic at the provider/load-balancer layer outside the old d2b protocol.

### Provider protocol rule

There is no ACA-specific, Azure-VM-specific, or hypervisor-specific d2b
protocol. Providers can have provider-specific provisioning APIs, but the d2b
operation/stream contract above the provider boundary is shared.

## Nix and configuration surface

The public architecture target is `d2b.realms`, not `d2b.gateways`.

The new surface should describe:

- realm id and parent;
- placement (`host-local`, `gateway-vm`, `cloud-full-host`,
  `provider-controller`, or provider-specific placement);
- local state/audit/socket paths derived from the realm id;
- broker instance configuration where host mutation is available;
- provider declarations;
- relay/discovery configuration;
- policy bundle;
- key/enrollment material references;
- default workload namespace and env/network membership.

The current grouping surface is removed in the same cutover and replaced by
realm membership. `home`, `dev`, and `work` become first-class realm
declarations. Any VM that cannot be mapped from its existing group into a realm
must be assigned explicitly by the operator or fail evaluation with a typed
migration error; it must not be silently adopted into an arbitrary default
realm. During the transition, existing `d2b.envs` terminology remains the
current code and documentation truth until the realm Nix surface and migration
errors are implemented.

The client/user-session surface must also be declarative. A `programs.d2b` or
equivalent module configures the CLI and desktop helpers with the local-root
realm access socket, default realm, explicit aliases, and any per-user desktop
integration settings. `d2b.realms.<name>` exposes an `allowedUsers` or
equivalent authorization surface that provisions the distinct Unix groups or
ACLs needed for those users to connect directly to host-local realm sockets
without routing through the local root as a proxy.

NixOS module evaluation is the primary place to reject declared conflicts:

- duplicate realm ids or parent cycles;
- duplicate host-local bridge/TAP/veth names;
- overlapping static IPv4 subnets, IPv6 ULA subnets, or address ranges;
- conflicting nftables ownership ids or table names;
- duplicate realm socket/state/audit paths;
- child realms without a declared parent;
- child realm units that would start before their local parent.

Runtime arbitration remains fail-closed for drift, hand-edited host state, and
provider-created resources not visible at eval time. Host-local child realm
systemd units start after their local parent realm. The exact unit spelling is
an implementation detail, but it must be deterministic and greppable.

On NixOS, declarative configuration remains authoritative for immutable host
files. The local root allocator must not overwrite NixOS-managed `/etc/hosts`
or `/etc/NetworkManager/conf.d` files at runtime. Static realm hostnames and
NetworkManager unmanaged rules are emitted through NixOS evaluation
(`networking.hosts`, NetworkManager unmanaged options, or equivalent module
outputs). Dynamic/transient realm discovery uses immutable-friendly runtime
surfaces such as `systemd-resolved`, an NSS helper, or `/run/NetworkManager/conf.d`
drop-ins instead of mutating `/etc`.

`d2b.gateways`, the old realm/ACA sandbox surfaces, and the old user-facing
grouping surface are removed as public configuration. The migration path is an
explicit cutover into `d2b.realms`, not a compatibility transform. A generation
that still declares the old surfaces fails with a typed migration error pointing
at the new realm declaration shape.
On NixOS, removed options use explicit removed-option modules or equivalent
assertion errors so operators see the migration message at evaluation time
rather than a generic unknown-option failure. Realm parent-cycle detection uses
an explicit visited set or bounded topological sort; it must not rely on
unbounded recursive parent traversal.

Large disk/state moves for existing local Cloud Hypervisor VMs are not
performed implicitly inside `nixos-rebuild switch` activation. They run through
an explicit migration command or daemon-owned adoption workflow so activation
does not time out or leave partially moved state.

## Rust architecture consequences

Realm-native d2b changes the Rust crate boundaries. The implementation plan
must decide the exact names, but the dependency direction is fixed:

```text
realm DTOs / target parser / operation frames
  <- provider traits / transport traits
    <- router / idempotency / policy evaluation
      <- d2bd runtime integration
```

Core DTO crates must not depend on `d2bd`, provider implementations, transport
implementations, `prost`-generated codec internals, or host-only broker code.
Provider traits can depend on realm DTOs, but provider implementations cannot
pull host daemon internals into provider-agnostic crates.

The current `d2b-constellation-*` crates are renamed around realm terminology
before runtime work starts rather than kept behind compatibility aliases.
Gateway-specific runtime crates and modules should be
absorbed into realm/provider crates or retired when `d2b.gateways` and old ACA
sandbox support are removed.

The target parser needs a new type-safe representation:

```text
RealmTarget {
  workload: WorkloadId,
  realm: RealmPath,   // leaf-to-root labels from `<vm>.<realm>...d2b`
}
```

The legacy node-qualified parser should remain only long enough to emit typed
migration diagnostics. New routing code must not accept
ambiguous optional-node targets as a normal path.

Provider/session traits must preserve the existing invariants in type shapes:

- mutating operation constructors require idempotency keys;
- operation kinds derive required capabilities in trusted code;
- negotiated capability sets are explicit inputs to routing decisions;
- stream-open types encode the authorizing operation and stream capability;
- provider-specific request bodies remain opaque payloads or typed DTOs below
  the standard d2b operation envelope.

## Security consequences

This ADR deliberately tightens isolation:

- named local realms no longer share one host-global daemon/broker boundary;
- host-local work and dev realm credentials are not colocated in one
  daemon's policy store;
- parent/child routing is tree-only, avoiding surprising transitive DAG trust;
- dynamic discovery is admitted only after realm-key and policy checks;
- provider capability advertisement prevents "works by fallback" behavior;
- relay identity remains non-authoritative;
- display, shell, clipboard, and device features are separate capabilities.

It also introduces new risks:

- multiple host-local brokers need safe arbitration for global host resources;
- process/unit naming must stay understandable and greppable;
- per-realm audit split can make cross-realm incident reconstruction harder
  without a bounded audit-chain model;
- key rotation and revocation become mandatory, not optional polish;
- dynamic discovery can become a resource-exhaustion surface if advertisements,
  enrollment attempts, and route tables are not bounded;
- the local UX can regress if realm-native routing adds latency or ambiguity to
  common bare-VM workflows.

The implementation must treat those risks as first-wave design constraints,
not later hardening work.

## Migration from ADR 0032

This ADR supersedes [ADR 0032](0032-d2b-v2-constellation-control-plane.md)'s
host-centric entrypoint model:

| [ADR 0032](0032-d2b-v2-constellation-control-plane.md) concept | Realm-native replacement |
| --- | --- |
| Host `d2bd` with realm entrypoint table | Minimal local root plus separate realm controller instances. |
| `host-resident` named realm inside the host daemon | Host-local realm daemon with its own broker/socket/state/audit boundary. |
| `gateway-backed` as public abstraction | Realm placement may be `gateway-vm`; public surface is still `d2b.realms`. |
| `d2b.gateways` Nix surface | `d2b.realms` with provider/placement fields. |
| `<workload>.<node>.<realm>.d2b` target | `<vm>.<realm>[.<ancestor>...].d2b`; placement resolved inside the realm. |
| Remote node registry as gateway-owned state | Realm-owned provider/node/workload registry. |
| CLI talking directly to one local daemon namespace | CLI and desktop tools talk to a realm access layer that resolves and dispatches to the owning realm controller. |

Existing local VM state must be migrated deliberately. A release implementing
this ADR adopts pre-realm local VMs into `home`, `dev`, or `work` realms when
their current group membership is unambiguous. If that is not possible for a
specific deployment, the release must choose one of these explicit paths and
test it:

- require an operator-supplied mapping from old VM names to target realms;
- fail closed with a migration command that moves disks, state, audit pointers,
  and runtime metadata into the selected realm.

Local Cloud Hypervisor VMs are part of this required migration. Their disks,
per-VM state, guest-control identity, display/Wayland proxy identity, audit
pointers, and runtime metadata move under the owning realm while preserving the
local fast path for default-realm use.

Gateway and ACA state are not preserved through compatibility mode. The
implementation may provide one-shot migration/import tooling for non-secret
coordinates and operator-approved state, but old gateway-backed realm state,
old ACA sandbox registrations, old ACA provider-specific command/session
contracts, and old sealed credential layouts are not live runtime inputs after
the cutover. If such state is found, d2b fails closed with a typed migration
error rather than silently adapting it.

Migration and import tooling emits structured audit records into the target
realm's audit log describing the adopted resource ids, source generation,
operator principal, outcome, and bounded correlation id. It never records
credential material, provider tokens, raw endpoints, command payloads, disk
contents, or old sealed credential bytes. Fail-closed migration errors and
legacy parser diagnostics also emit low-cardinality telemetry such as
`legacy-surface-detected`, `migration-required`, and `import-failed` reason
codes so operators can find blockers across a fleet. Successful import tooling
should prompt for, or provide an explicit flag for, secure cleanup of legacy
state directories after re-enrollment.

The following [ADR 0032](0032-d2b-v2-constellation-control-plane.md) decisions
remain valid and should be carried forward:

- semantic operation/stream frames;
- no raw broker/daemon/guest-control tunneling;
- relay is untrusted reachability;
- capability negotiation and typed denial;
- idempotency for mutating operations;
- bounded/redacted audit and telemetry;
- provider adapters must not imply full-host authority when absent.

## Implementation outline

The first implementation plan after ADR acceptance should proceed in waves:

1. Define the realm controller DTOs, `d2b.realms` schema, and migration errors;
   regenerate schemas with `xtask gen-schemas` and satisfy the drift gates.
2. Remove old realm/gateway/ACA public surfaces and add typed migration errors
   for configs or runtime state that still use them.
3. Split host-local daemon/broker instances by realm with deterministic
   socket/state/audit paths and global host-resource arbitration.
4. Replace the current target parser/resolver with realm-qualified
   `<vm>.<realm>[.<ancestor>...].d2b` semantics.
5. Move CLI, public API clients, and desktop helpers (`d2b-wlcontrol`,
   `d2b-clip-picker` integration, `d2b-wlterm`) onto the realm access layer so
   they consume canonical addresses and capability-denial results instead of a
   single local daemon namespace.
6. Add realm identity, enrollment, controller-generation keys, rotation, and
   revocation.
7. Implement dynamic relay discovery and tree route admission using loopback
   and local-TCP tests first.
8. Route core operation families: lifecycle, exec, persistent shell, logs, and
   Wayland/display.
9. Migrate local Cloud Hypervisor runtime support behind the realm-local runtime
   provider model.
10. Migrate ACA sandbox support behind the shared realm protocol and capability
    model, with no live compatibility for old ACA sandbox contracts.
11. Update the Diataxis documentation tree: explanation docs for realm-native
   architecture, reference docs for `d2b.realms` and target addresses, how-to
   migration guidance for existing local Cloud Hypervisor VM users and old
   ACA/gateway users, and the CHANGELOG breaking-change entry.

Each implementation wave must include its own validation rather than deferring
testing to the end. Required validation includes:

- NixOS assertion tests for duplicate host resource claims and parent-before-child
  activation ordering;
- hermetic host-resource arbitration tests using mocked system interfaces or
  isolated user/network namespaces rather than shared live host state;
- a lightweight mock topology harness using loopback/local-TCP transports and
  mock providers for nested realm policy, tree-only routing, multi-parent
  route rejection, and direct-shortcut denial;
- capability-denial tests proving unsupported operations return typed d2b
  denials instead of SSH, generic network, provider-native, or timeout
  fallbacks;
- positive routing tests proving core operations such as lifecycle, exec,
  persistent shell, logs, and Wayland/display successfully traverse authorized
  realm boundaries with mock or hermetic providers before full VM tests;
- client contract tests proving the CLI, `d2b-wlcontrol`, clipboard picker
  integration, and `d2b-wlterm` consume canonical realm target addresses and
  fail closed on unsupported cross-realm targets; host-local client tests must
  prove direct realm-socket connection preserves `SO_PEERCRED` and `SCM_RIGHTS`
  rather than proxying through the local root;
- trust-lifecycle tests for realm key rotation, controller-generation
  revocation, parent-pushed revocation lists, and forced teardown of active
  sessions/streams on revocation;
- discovery resilience tests for pre-auth queue bounds, per-relay rate limits,
  replay windows, and drop-new behavior under simulated load;
- migration tests for existing local Cloud Hypervisor VM state adoption,
  fail-closed typed errors for old realm/gateway/ACA configuration or state, and
  explicit ACA sandbox import/re-enrollment tooling where provided;
- migration observability tests proving import/re-enrollment writes structured
  audit records and fail-closed legacy-surface errors expose bounded telemetry
  reason codes.

## Alternatives considered

### Keep ADR 0032 and amend it

Rejected. [ADR 0032](0032-d2b-v2-constellation-control-plane.md) has good
protocol invariants, but its entrypoint model keeps the host daemon as the
conceptual owner of named realms. That is the bolt-on shape this ADR is
correcting.

### Keep one host broker with realm-scoped authorization

Rejected. A single host broker would be simpler, but it would keep a
host-global privileged mutation surface across `home`, `dev`, and `work`.
The realm boundary should be visible in process, socket, state, and audit
ownership.

### Require gateway VMs for sensitive realms

Rejected as the default. Gateway VMs remain useful, but requiring one for every
real realm boundary makes local realm isolation heavier than necessary and
preserves the idea that host-local realm daemons are second-class.

### Use DAG routing with least-hop path selection

Rejected for the initial architecture. DAG routing is flexible, but it creates
multi-parent trust edges, route-selection ambiguity, and difficult transitive
policy questions. Strict parent/child tree routing is easier to audit and
matches the realm hierarchy.

### Keep node-qualified public addresses

Rejected. Node-qualified addresses leak placement into the user-facing target
grammar. VM names should be unique within a realm, and the realm controller
should own placement.

### Provider-specific d2b protocols

Rejected. Provider APIs differ below the provider boundary, but d2b operations
above that boundary must remain standard semantic d2b contracts with
capability advertisement.

### Rolling compatibility for old realm and ACA protocols

Rejected. Keeping the legacy optional-node parser, old gateway routing, or old
ACA command/session contracts during rollout would preserve the bolt-on realm
model this ADR removes. It would also keep stale credential layouts and
provider-specific behavior alive inside the new security boundary. Operators
who need continuity must run old and new deployments side-by-side and move
traffic outside the old protocol; d2b itself fails closed on old realm/gateway
and ACA surfaces after the cutover.

## Accepted cutover clarifications

- Non-secret gateway or ACA coordinates may be imported only through explicit
  one-shot tooling. Secret material, sealed credential layouts, old sessions,
  and provider-specific command/session contracts are never live compatibility
  inputs; operators must recreate or re-enroll them.
- Provider environments that can run full d2b should run a full realm
  controller. Constrained provider environments may run a provider agent that
  shares DTOs and the semantic operation/stream contract, but it advertises only
  the capabilities it can actually provide and never gains host-lifecycle
  authority by implication.
- The first cross-realm Wayland/display path is the d2b-owned semantic
  display/stream path with trusted realm identity metadata. Local default-realm
  display keeps the current fast path while it is migrated behind the same
  capability model. Guest-provided window titles, app ids, MIME metadata, and
  payload labels remain non-authoritative.
