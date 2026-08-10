# ADR 0046 resources: Network

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resources-network` |
| Parent | ADR 0046 |
| Status | Accepted |
| Version | 3 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-network-local`, `d2b-contracts` network types, Nix network emitter |
| Depends on | `ADR-046-resource-object-model`, `ADR-046-primitive-resource-composition`, `ADR-046-resource-reconciliation`, `ADR-046-provider-model-and-packaging`, `ADR-046-terminology-and-identities` |
| Supersedes | `d2b.envs.<env>` and `d2b.realms.<realm>.network` Nix surfaces (v3 reset) |

## Purpose

This spec defines the `Network` ResourceType, its provider `Provider/network-local`,
and the complete contract between the network fabric, its Host/Guest attachments,
and the controllers that reconcile it.

A Network is an independently shared, lifecycle-managed local fabric. It owns:

- two host kernel bridges (LAN and uplink) and their tap dispatch;
- one auto-declared owned net-VM Guest that runs NAT/DHCP/DNS/firewall;
- DHCP address reservations and DNS forwarding for all attached Guests;
- stateful nftables firewall and NAT rules on the host and inside the net VM;
- per-attachment isolation policy, with unsafe east-west reachability admitted
  only by the Network and Host/site double opt-in;
- optional external network attachment (macvtap) with port-forward and egress.

`Provider/network-local` is the only initial Network Provider. Azure Container
Apps and Azure Virtual Machine networking remain inside their respective Guest
Providers (`Provider/runtime-azure-container-apps` and
`Provider/runtime-azure-virtual-machine`) until those networking surfaces
require independent sharing across Guests. See [Azure/ACA scope boundary](#azureaca-scope-boundary).

## Version 3 prospective Wave 6 authority amendment

Version 3 is the accepted prospective Wave 6 authority. It specifies work that
must land before the production Network path may be claimed; it does not
reinterpret historical Wave 4 evidence as implementation. Existing reachable
code and ADRs remain canon until the prospective work lands. In particular,
ADR 0012's double opt-in, the current ownership-marker rules, and the
daemon-only three-unit boundary are binding.

This amendment supersedes lower-version wording in this file that described a
single-factor east-west opt-in, a per-Network writer for Host-global files
or nftables state, a public attachment handle, or dnsmasq as an untracked child
of another process.

### One canonical Network schema

There is one Network base schema:
`core.d2bus.org_Network.schema.json`, generated from
`d2b_contracts::v3::network::NetworkSpec`. The Nix emitter, Resource API,
controller, and every Provider consume its exact version and fingerprint.
`Provider/network-local` does not define a second full Network schema.

The optional selected-Provider envelope remains
`spec.provider = { schemaId, schemaVersion, settings }`. For the initial
`network-local` implementation its signed desired-settings schema is a strict
empty object. It may be omitted; if present, `settings` must be `{}`. A field
already defined by the base schema is rejected in the Provider envelope as
`spec-provider-shadow`. Adding a genuine implementation-only setting requires
a versioned Provider-schema amendment and must not change base Network
semantics.

`d2b.site.allowUnsafeEastWest` is Host/site admission policy, not a Network
base field and not Provider settings. It remains false by default and is
supplied to Core from trusted evaluated Host configuration.

### Host-global admission before effects

Core performs one atomic Host-global admission before any bridge, tap, route,
address, nftables, Process, or Guest effect. The reservation covers:

- every Network LAN and uplink CIDR across all Zones on the Host, plus the
  Host's observed non-d2b network inventory;
- every derived kernel interface name on the Host, including bridge, tap, and
  macvtap child names;
- the net-VM Guest's Host-global vsock CID, allocated by the Core allocator and
  never derived from a Network CIDR or exposed as a public locator; and
- the external physical-NIC authority described below.

Admission is compare-and-reserve in the durable Core authority index. A
concurrent claimant cannot pass a peer scan and race to effects. Restart adopts
only an exact owner proof; ambiguity quarantines the claimant. CIDR, name, and
vsock reservations are released only after dependent Guests stop, their FDs
close, routes and addresses are removed, and owned links are gone.

Conflicts fail before effects with `network-cidr-conflict`,
`ifname-collision`, or `vsock-cid-conflict`. The error names the conflicting
class and the remediation: choose disjoint CIDRs, rename the Network or
conflicting Guest, or release/reallocate the stale vsock reservation.

### Exact east-west double opt-in

The effective predicate is exactly:

```text
Network.spec.isolation.allowEastWest && d2b.site.allowUnsafeEastWest
```

Both inputs default to false. No Provider setting, Zone field, environment
variable, status mutation, or historical evidence substitutes for either
input.

| Network `allowEastWest` | Site `allowUnsafeEastWest` | Production result |
| --- | --- | --- |
| false | false | Admitted with workload taps isolated and no east-west accept rule |
| false | true | Admitted with workload taps isolated and no east-west accept rule |
| true | false | Refused before mutation with `east-west-site-opt-in-required`; existing taps remain or return isolated |
| true | true | Admitted; workload taps may be non-isolated and the typed guest firewall plan may include east-west forwarding |

Every production test of this matrix enters through evaluated Host/site policy,
Core admission, the real controller adapter, broker effects, and readback.
Testing a helper predicate or a fake effect port alone is insufficient.

### Single Host-global projection owners

Per-Network controllers publish typed desired contributions; they do not write
Host-global state independently.

- One in-process Core nftables dispatcher owns composition for the Host. It
  combines Network and device-usbip contributions into the shared `inet d2b`
  table, sorts them deterministically, validates each ownership ID, and sends
  one generation-fenced desired projection to the broker under the ordered OFD
  lock. It is not a systemd service or another root-visible unit.
- The broker mutates only d2b-owned chains and rules carrying
  `comment "d2b managed: <ownership-id>"`. It never flushes a foreign table or
  removes a foreign rule. A foreign marker in a d2b ownership slot fails
  closed with `foreign-nft-rule-preserved`.
- One Host-global NetworkManager projection and one Host-global `/etc/hosts`
  projection aggregate all active Network contributions. Each broker apply
  replaces only the single `# d2b-managed begin` / `# d2b-managed end` region,
  preserves every byte outside it, and refuses missing, nested, duplicate, or
  foreign markers. Removing one Network recomputes the aggregate and cannot
  erase a sibling Network.

NetworkManager reload happens after its aggregate block is durable and before
new d2b links are created. The `/etc/hosts` aggregate is installed only after
addresses are admitted and resolved. systemd-networkd remains detection-only;
d2b never writes its configuration.

### Typed effects and ordered realization

No provider-authored command, nft script, route string, interface name,
ownership marker, or host path crosses the effect boundary. The canonical
semantic inputs are bounded typed `LinkIntent`, `AddressIntent`,
`RouteIntent`, `ForwardingIntent`, `NatIntent`, `FilterIntent`, and
`HostProjectionContribution` values. Core resolves them to private
bundle-backed broker operations; the net-agent resolves the guest portion
inside the net VM. Every mutation has typed readback and a generation fence.

Reconcile uses this order:

1. Commit the Resource revision, then atomically acquire Host-global CIDR,
   name, vsock, and external-NIC authority.
2. Publish and apply the aggregate NetworkManager projection.
3. Create bridges down with durable ownership markers; set the effective MTU,
   bridge flags, and IPv6 suppression; read them back; then bring the bridges
   up.
4. Assign the typed Host uplink address and create generation-fenced taps with
   the same effective MTU. A bridge or route is adoptable or deletable only
   when its kernel marker and broker-durable ownership record agree.
5. Create the net-VM Guest with the privately allocated vsock CID and attach
   its FDs through the LaunchTicket.
6. Start the owned net-agent Process and apply guest addresses first, routes
   second, forwarding third, filter/NAT fourth. The net-agent reports typed
   readback for each stage.
7. Start exactly one owned dnsmasq Process after guest address and route
   readiness. It reads the committed config Volume and must reach
   `dnsmasq-bound`.
8. Install the Host route only after guest routes and dnsmasq are ready, then
   apply the composed Host nftables and `/etc/hosts` projections.
9. Mark attachments and the Network Ready only after MTU, isolation, address,
   route, forwarding, NAT/filter, dnsmasq, and Host projection readback agree.

Delete reverses dependency order. It removes Host projections and routes before
addresses and links, releases each private attachment only after its VMM FD is
closed, and releases Host-global reservations last.

The effective MTU is `spec.mtu` or 1500. It is propagated to both bridges,
every d2b-created tap, both net-VM NICs, every attached workload Guest NIC, the
DHCP MTU option, and any d2b-created macvtap child. d2b never changes the
external parent NIC's MTU. If that parent cannot carry the requested MTU,
admission fails with `network-mtu-parent-too-small` and tells the operator to
lower `Network.spec.mtu` or raise the parent MTU.

### Durable link and route ownership

Bridge creation writes the trusted ownership ID to `IFLA_IFALIAS` before
link-up and fsyncs the corresponding broker-resolved ownership record before
reporting success. Route installation records an exact canonical route key and
the broker-selected d2b route protocol in the same durable ownership domain
before reporting success. Marker storage is declared through ADR 0034
`storage.json` and `sync.json` rows and is resolved by opaque ID; no
caller-supplied path is accepted.

Adoption, replacement, and deletion require agreement between the durable
record and live kernel readback. Missing, stale, duplicate, or foreign markers
fail closed and preserve the live object. Destination-only route ownership and
an interface-name-only bridge match are never sufficient authority.

## ResourceType threshold

Network satisfies all threshold criteria from
`ADR-046-primitive-resource-composition`:

- **independent identity**: a fabric exists before any workload Guest runs;
- **independent lifecycle**: bridges and the net VM outlive any single Guest;
- **independent controller/status**: the controller watches its own resource
  class through the reconciliation loop;
- **sharing**: several Guest resources attach to one Network concurrently;
- **Provider substitution**: `Provider/network-local` may be replaced by a
  future provider (e.g. WireGuard mesh, VLAN fabric) without changing the
  ResourceType.

## NetworkSpec

### Three-layer spec shape (D089)

D089 freezes Network spec as three layers. Layer 1 is the universal Resource
envelope and metadata. Layer 2 is the Network base spec at top-level `spec.*`,
including `spec.providerRef`; the CIDR, bridge, isolation, routing, DHCP/DNS,
attachment, mDNS, and net-VM fields documented here are base fields. Layer 3 is
the optional canonical selected-Provider extension
`spec.provider = { schemaId, schemaVersion, settings }`; it is the only
Provider-specific desired extension. It omits `providerRef` and
`observedProviderGeneration`: `spec.providerRef` is base, and spec is desired
rather than observed.

**D091 update policy.** The universal base spec carries `spec.updatePolicy` for
every Network: disruptive changes default to manual, while automatic
non-disruptive upgrades are permitted by policy. A `spec.provider` extension MAY
add provider-specific knobs, but MUST NOT bypass or weaken base
`spec.updatePolicy`.

**D090 expedited reconcile.** Authorized Network `Create`, `UpdateSpec`, and
`Delete` calls MAY set `waitForReconcile`. Under one mutation ticket,
`operationId`, and deadline, Core admission and the reserved-revision redb commit
run in parallel with controller preflight/plan, but the controller MUST NOT
perform external effects, finalizer release, or status mutation until Core
supplies `CommittedRevisionProof {resourceUid, generation, revision,
operationId}`; DB failure aborts with no effect. The API returns the committed
object plus one-pass projected layered status, `disposition`
(`Converged|Progressing|Blocked|UpgradeRequired|Failed`), `statusPersistence`
(`pending|committed`), and the last persisted status revision. The durable
commit is never rolled back on reconcile timeout or failure; effect idempotency
keys derive from `(UID,generation,revision,operationId)`, and the expedited pass
uses a bounded priority lane in the same per-resource single-flight.

Every Network Provider `ResourceApiBinding` MUST implement the exact Network
base spec schema version and fingerprint, accept the canonical minimal valid
base Spec, and pass base lifecycle/status/error/finalizer conformance. A
Provider MAY reject an optional base capability only through its signed standard
capability matrix and a typed provider-neutral `unsupported-capability` error;
it MUST NOT ignore, reinterpret, rename, duplicate, weaken, or require extension
data for base-required behavior. `spec.provider.settings` is strict
deny-unknown, bounded, schema-versioned and digested, validated against
`spec.providerRef` at Nix build and API admission, and fails with
`spec-provider-schema-invalid` or `spec-provider-shadow` when invalid or
shadowing/restating/overriding/renaming/duplicating a base field. Shared Network
semantics are promoted to the Network base spec and never live in
`spec.provider`; generic CLI/controllers operate on base spec plus base status.
For the same Provider, the `spec.provider` and `status.provider` schemas align.

```yaml
apiVersion: resources.d2bus.org/v3
type: Network
metadata:
  name: work-net
  zone: dev
  uid: <store-generated>
  generation: 1
  revision: <opaque>
  ownerRef: null
  finalizers: []
  deletionRequestedAt: null
  createdAt: 2026-07-22T00:00:00.000Z
  updatedAt: 2026-07-22T00:00:00.000Z
spec:
  providerRef: Provider/network-local

  # --- CIDR allocation ---
  lanCidr: "10.20.0.0/24"
  uplinkCidr: "192.0.2.0/30"

  # --- Layer-2/3 settings ---
  mtu: null           # null = 1500; applies to bridges, taps, and net-VM NICs
  mssClamp: false     # TCP MSS clamping in net-VM forward chain (tunneled uplinks)

  # --- Isolation policy ---
  isolation:
    allowEastWest: false
    # Effective east-west requires this field AND the false-by-default
    # d2b.site.allowUnsafeEastWest Host/site gate.

  # --- Egress policy ---
  routing:
    hostBlocklist:
      - "10.0.0.0/8"
      - "172.16.0.0/12"
      - "192.168.0.0/16"
      - "169.254.0.0/16"
    # The controller merges host network inventory from the Host resource and
    # all Host-global Network lanCidrs/uplinkCidrs into this list before emitting
    # firewall rules. Caller-supplied entries are additive; duplicates deduplicated.

  # --- DHCP/DNS ---
  dhcp:
    domain: null          # optional dnsmasq domain name for the LAN
    ignoreClientNames: true  # prevents workloads spoofing hostnames
    # dhcp-authoritative is always on. DHCP pool: lanCidr.251-254 (unreserved).
    # Static reservations derive from spec.attachments.

  dns:
    forwarders:
      - "198.51.100.53"
      - "203.0.113.53"
    cacheSize: 1000

  # --- External network attachment (optional) ---
  externalAttachment: null  # or ExternalAttachmentSpec; see below

  # --- mDNS (optional) ---
  mdns:
    enable: false
    # enable: true causes the Network controller to create owned Process
    # resources for the mDNS reflector and, when dnsmasqLocal is true, the
    # local DNS bridge. Both run inside the net-VM Guest via executionRef.
    # Foundation requires every ordinary process to be a Process resource;
    # mDNS is not an inline untracked systemd service.
    reflector: true            # create avahi reflector Process inside net VM
    dnsmasqLocal: false        # create local DNS bridge Process inside net VM
    dnsmasqLocalPort: 53530    # listen port for dnsmasqLocal Process
    publishWorkstation: false  # avahi advertises workstation presence

  # --- Net-VM Guest name override ---
  netVmNameOverride: null
  # Null → "net-<networkName>". Must match ResourceName regex.
  # When set, the controller creates Guest/<netVmNameOverride> instead.

  # --- Net-VM system artifact (REQUIRED) ---
  netVmSystemArtifactId: "<artifact-id>"
  # Required. Must reference a declared d2b.artifacts entry with type = "nixos-system".
  # Resolved and verified at Nix build time (Stage 2). Absent or wrong type is a
  # hard build error. The resolved artifact ID is stored verbatim in
  # Guest.spec.systemArtifactId. There is no implicit default; every Network must
  # name its net-VM nixos-system artifact explicitly.

  # --- Host/Guest attachment table ---
  attachments: []
  # List of AttachmentSpec; see below.
status:
  observedGeneration: 0
  phase: Pending
  conditions: []
  lastReconciledAt: null
  startedAt: null
  completedAt: null
  outcome: null
  resource: {}                       # Layer 2 ResourceType-common; {} until reconciled (D107)
  update:                            # universal currency object; present on every resource (D091)
    state: Unknown
    reasons: []
    observedGeneration: 0
    targetGeneration: 1
    disruption: None
    preserveState: true
    operationId: null
    lastAssessedAt: null
    owned: { count: 0, refs: [] }
    dependencies: { count: 0, refs: [] }
```

### AttachmentSpec (inline in NetworkSpec)

Each entry reserves one address and MAC on the LAN bridge for a Guest or Host:

```yaml
executionRef: Guest/corp-vm
index: 10
# index: 2..250; 1 is reserved for the net VM's LAN interface.
# Uniqueness within the Network is enforced at validateSpec time.
mac: null
# null = derive deterministically from (networkName, index) using mkMac.
# A fixed value must be a valid unicast MAC in colon notation.
# Indices are stable across reconcile cycles; changing an index
# changes both the DHCP reservation and the tap IfName.
```

Addresses:

| Role | Formula |
| --- | --- |
| Host uplink IP | `uplinkCidr.1` |
| Net VM uplink IP | `uplinkCidr.2` |
| Net VM LAN IP | `lanCidr.1` |
| Workload IP for attachment at index N | `lanCidr.N` |
| DHCP dynamic pool | `lanCidr.251`-`lanCidr.254` |

The `subnetIp` formula extracts the first three octets of the CIDR base and
appends the host ordinal, preserving the v3-baseline `subnetIp` helper in
`nixos-modules/lib.nix` line 399.

### ExternalAttachmentSpec (inline in NetworkSpec)

```yaml
externalAttachment:
  mode: macvtap             # only mode in the initial contract
  parentInterface: eno1     # host physical interface; IfName-validated
  macvtapMode: bridge       # bridge | private | vepa | passthru
  sharingPolicy: exclusive  # exclusive | multiplexed; multiplexed MUST be
                            # explicitly authored and is valid only with bridge
  mac: null                 # null = derive from (networkName, "home", 3)

  ipv4:
    method: dhcp            # dhcp | static
    address: null           # CIDR notation; required when method=static
    gateway: null           # required for static default route
    dns: []                 # static resolvers; used only when method=static

  egress:
    enable: false
    allowedCidrs: []        # CIDRs reachable through external0; MUST NOT
                            # overlap any Host-global Network CIDR or Host
                            # network inventory
    masquerade: true        # MASQUERADE outbound on external0

  portForwards: []          # list of PortForwardSpec; see below
```

`parentInterface` is a requested host inventory selector, not the authority
identity. Core resolves it against trusted Host network inventory and derives a
non-reversible `external-physical-nic/v1` identity. The resulting authority
index key is Host-global:
`(Host, external-physical-nic, opaqueKeyDigest)`. The digest is never supplied
by a caller or exposed in spec, status, audit, or telemetry.

The Host-global authority binds an isolation domain equal to the claimant's
Zone UID. `passthru`, `private`, and `vepa` always require exclusive
arbitration. `bridge` is also exclusive by default. It becomes multiplexed only
when every claimant explicitly authors `sharingPolicy: multiplexed`, every
claimant belongs to the same Zone (the same isolation domain), and the signed
Provider quota admits another holder. An absent policy defaults exclusive; a
mixed exclusive/multiplexed set, a non-bridge multiplexed claim, or quota excess
fails closed with `external-physical-nic-conflict` before macvtap creation or
VMM spawn. Because the index is Host-global, these checks span every Zone on the
Host: two claimants in different Zones that would multiplex one physical NIC in
`bridge` mode share a single L2 broadcast domain, so that combination is
categorically rejected fail closed with `external-physical-nic-cross-zone-l2`
regardless of `sharingPolicy`. Bridge multiplexing is admitted only within a
single Zone; work and personal Zones never share an L2 bridge (INV-NET-010).

### PortForwardSpec (inline in ExternalAttachmentSpec)

```yaml
- protocol: tcp             # tcp | udp
  listenPort: 2222          # port on net VM external0
  targetRef: null           # AttachmentSpec.executionRef resolved to
                            # its index→IP; mutually exclusive with targetIp
  targetIp: null            # explicit LAN IP; mutually exclusive with targetRef
  targetPort: 22
  sourceCidrs: []           # optional ingress source filter
```

Both `targetRef` and `targetIp` null is rejected at validateSpec time.
Both non-null is also rejected.

External IPv4 and forwarding validation is fail-closed:

- DHCP requires `address = null`, `gateway = null`, and `dns = []`.
- Static mode requires a canonical IPv4 CIDR, a usable host address, and a
  gateway in the same subnet. The address and gateway must differ and neither
  may be the network or broadcast address.
- Every static DNS entry is a validated IPv4 address. The list remains bounded
  by the canonical base schema.
- `targetRef` must resolve to exactly one Guest in this Network's
  `attachments` table. `targetIp` must equal exactly one derived attached-Guest
  address in `lanCidr`; an arbitrary LAN address is not accepted.
- `(protocol, listenPort)` is unique within the external attachment. Both
  ports are nonzero 16-bit values.
- Every `sourceCidrs` and `egress.allowedCidrs` entry is canonical, bounded,
  and disjoint from every Host-global d2b Network CIDR and the Host's observed
  non-d2b inventory.

Examples use RFC 5737 ranges for documentation-only external addresses.

## NetworkStatus

### Three-layer status shape (D088)

D088 freezes `Network` status as three layers. The universal `ResourceStatus`
base (Layer 1) lives at top-level `status` and owns `observedGeneration`,
`phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, and
bounded `outcome`. The `Network`-specific status fields documented in this
section constitute the ResourceType-common `status.resource` (Layer 2) object
and never restate the universal base. Optional implementation-only observation
belongs in `status.provider` (Layer 3) with exactly `providerRef`, qualified
immutable `schemaId`, semver `MAJOR.MINOR` `schemaVersion`, numeric
`observedProviderGeneration`, and strict, bounded, redacted,
unknown-field-denied `details`. Generic API, CLI, and controllers MUST consume
only the base-only projection (`status` base plus `status.resource`).
Controllers MUST write all present layers atomically in one status mutation with
one expected revision. D087/D088 mapping is: shared observations go to
`status.resource`; implementation-specific bounded non-secret observations go to
`status.provider.details`; secret, large, or private observations go to an
optional Volume. D088 bounds apply: total status <= 64 KiB, `status.resource` <=
32 KiB, `status.provider.details` <= 32 KiB, 32 conditions, 64-entry lists/maps,
and 4 KiB strings; violations use `status-oversize`,
`status-provider-schema-invalid`, or `status-provider-overlap`.

Mapping convention: within this spec a reference to `status.<field>` denotes the ResourceType-common `status.resource.<field>` unless `<field>` is a universal base field (`observedGeneration`, `phase`, `conditions`, `lastReconciledAt`, `startedAt`, `completedAt`, `outcome`).

**D091 update currency.** Every Network includes universal `status.update` with
`state` (`Current|UpdateAvailable|UpgradeRequired|Upgrading|Blocked|Unknown`),
`reasons`
(`CoreGenerationChanged|ProviderGenerationChanged|ArtifactChanged|ImageOrSystemGenerationChanged|SpecChanged|DependencyChanged|SecurityPolicyChanged`),
bounded non-secret observed/target generation and digest IDs, `disruption`
(`None|Reload|Restart|Recycle|Replace`), `preserveState`, optional
`operationId`, `lastAssessedAt`, and bounded/truncated `owned:{count,refs}` and
`dependencies:{count,refs}`. Network-specific fabric currency refinements live
in `status.resource` and never in `status.provider`; controllers set
`status.update` via `assess_update` on core/provider/artifact/spec/dependency/
security-policy triggers and MUST report `UpgradeRequired` for disruptive
changes rather than applying them in place. Disruptive fabric recycle affects
dependent attachments through the dependency-aware planner, which drains,
recycles, and restarts dependents instead of mutating them in place.

The existing `status.network` sub-object is carried within `status.resource` as
`status.resource.network` by the mapping convention. `Network` currently uses
`network-local`; the provider-neutral Network fields are frozen in
`status.resource` for `network-local` and any future implementation. Stable
resource references, bounded readiness, external authority state, and
attachment phase are the shared cross-resource dependency surface. Runtime
addresses, interface names, MACs, authority keys, attachment handles, and the
implementation-specific firewall digest are not common fields. The
network-local digest belongs only in its `status.provider.details`; shared
fields MUST NOT be duplicated there.

```yaml
status:
  observedGeneration: 1
  phase: Ready   # Pending | Ready | Succeeded | Degraded | Failed | Deleted | Unknown
  conditions:
    - type: FabricReady
      status: "True"
      reason: bridges-present
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:01.000Z
    - type: NetVmReady
      status: "True"
      reason: guest-ready
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:02.000Z
    - type: DhcpReady
      status: "True"
      reason: dnsmasq-bound
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:02.000Z
    - type: FirewallReady
      status: "True"
      reason: nft-applied
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:01.000Z
    - type: CidrConflict
      status: "False"
      reason: no-conflict
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:00.000Z
    - type: ExternalNicAuthorityReady
      status: "True"
      reason: external-physical-nic-claimed
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:01.000Z
  lastReconciledAt: 2026-07-22T00:00:02.000Z
  outcome: null

  # --- Network-specific typed fields ---
  network:
    netVmRef: Guest/net-work-net        # owned Guest ref
    lanBridge:
      phase: Ready
    uplinkBridge:
      phase: Ready
    externalAttachment: null            # or ExternalAttachmentStatus
    attachments: []                     # list of AttachmentStatus; see below
  provider:
    providerRef: Provider/network-local
    schemaId: network-local.d2bus.org/Network/status
    schemaVersion: 1.0.0
    observedProviderGeneration: 1
    details:
      firewallDigest: "<sha256 hex>"    # Network-owned rules only
```

### AttachmentStatus (inline in NetworkStatus)

```yaml
- executionRef: Guest/corp-vm
  phase: Ready   # Pending | Ready | Degraded | Unknown
```

### ExternalAttachmentStatus (inline in NetworkStatus)

```yaml
externalAttachment:
  phase: Ready
  authority:
    available: true
    holderCount: 1
    queueDepth: 0
    arbitration: exclusive
    updateCurrency: Current
```

### Condition types

| Type | Meaning |
| --- | --- |
| `FabricReady` | Host bridges and tap dispatch rules are present and correct |
| `NetVmReady` | Owned net-VM Guest is Ready per its own status |
| `DhcpReady` | dnsmasq is bound on the LAN bridge IP and DHCP reservations match spec |
| `FirewallReady` | nftables `inet d2b` table is applied and digest matches |
| `CidrConflict` | No CIDR overlap detected with Host-global Networks or the Host resource's network inventory |
| `ExternalNicAuthorityReady` | Core admitted or adopted the Host-global physical-NIC authority claim; absent external attachments report `not-required` |
| `ExternalAttachmentReady` | External macvtap interface is up and net VM external0 is configured |
| `ReconcileError` | Latest reconcile attempt produced a retryable or terminal error |
| `ConfigVolumeReady` | Owned config Volume is Ready and contains current configuration; guest-agent has applied it |
| `NetworkDraining` | Network deletion requested; owned resources and attachments are being removed |

Phase rules:

- `Ready`: `FabricReady`, `NetVmReady`, `DhcpReady`, `FirewallReady`, and
  `ConfigVolumeReady` are all True; no `CidrConflict`; when an external
  attachment is configured, `ExternalNicAuthorityReady` and
  `ExternalAttachmentReady` are also True.
- `Degraded`: fabric is present but at least one condition is impaired; workloads
  may be running with reduced capability.
- `Failed`: CIDR conflict, `external-physical-nic-conflict`, or another terminal
  reconcile error; fabric cannot converge without a spec or sharing-policy
  change.
- `Pending`: initial creation before first reconcile completes.
- `Unknown`: controller/host disconnect; last known state reported.
- `Succeeded`: reserved schema value; Network never steadily occupies this phase
  (network operation is continuous, not a one-shot task). Present in the common
  phase enum for schema compatibility.
- `Deleted`: schema value present in the common phase enum; appears only in the
  final single store transaction that atomically removes the resource row and index.
  No persisted resource row ever carries `phase = Deleted`; controllers wait for
  the Deleted watch event or resource absence, not for a phase transition to `Deleted`.

## Provider/network-local

### Package and crate boundary

`Provider/network-local` maps to one independently buildable crate
`packages/d2b-provider-network-local/`. It contains:

- one controller binary `d2b-provider-network-local-ctrl`;
- one signed net-VM NixOS guest-config template library;
- declared exported `Network` ResourceType schema and controller descriptor;
- no dependency on `d2bd`, broker internals, or another Provider's
  implementation.

#### Required crate layout

Every `packages/d2b-provider-<base>-<impl>/` crate must have all four of the
following paths present. **Absence of any path is a workspace/package policy
failure** (enforced by `xtask workspace-policy` and `make test-policy`):

| Path | Role |
| --- | --- |
| `src/` | Implementation binaries and libraries; colocated `#[cfg(test)]` unit tests within each source file |
| `tests/` | Hermetic Cargo integration tests: ResourceType schema round-trips, controller state-machine tests (deterministic clock), conformance suite, fault-injection tests; no containers or external processes |
| `integration/` | Heavier fixture scenarios: container-based, Host/Guest lifecycle, cross-process, and provider-system tests; invoked by the existing test orchestration (`make test-integration` / `make test-host-integration`) not directly by `cargo test` |
| `README.md` | Documents: Provider identity and `providerRef` value; config schema and all `spec` fields per ResourceType; controllers, services, workers, and binaries produced; Host/Guest placement and executionRef requirements; dependencies and RBAC grants; security invariants, state ownership, and telemetry surface; exact `cargo build`, `cargo test`, `cargo test --test '*'`, and `integration/` invocation commands; future standalone-repo migration path |

`src/`, `tests/`, and `integration/` must each contain at least one tracked
file; an empty directory does not satisfy the policy. The `README.md` must
cover all seven documented topics (identity through standalone-repo path).

The crate depends on:

- `d2b-contracts` for Network, IfName, and network-related DTOs;
- `d2b-controller-toolkit` for the async reconcile loop and ResourceClient;
- `d2b-host` for IfName derivation, nftables, bridge-port, and route-preflight
  modules (extracted from the current `d2b-host` crate; see work items);
- `d2b-provider-toolkit` for Provider registration and conformance.

The controller process runs as a Host Process resource under a system domain:

```yaml
type: Process
metadata:
  name: network-local-ctrl
  zone: dev
  ownerRef: Provider/network-local
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system
  domain: system
  processClass: controller
  template: controller-main
  sandbox: { ... }   # broker-compiled minimal profile; no net caps
  budget: { ... }
```

The controller has no ambient host capabilities; all host-kernel bridge/tap/
nftables/sysctl effects are mediated through the broker via typed operations.

### IfName derivation

Interface names are derived deterministically from
`(Network.metadata.name, role, optional Guest.metadata.name)` using FNV-1a
64-bit, base32-encoded (Crockford alphabet, no I/L/O/U), truncated to 8
characters, prefixed by:

| Role | Prefix | Example |
| --- | --- | --- |
| LAN bridge | `d2b-b<hash>` | `d2b-ba7mk3vp` |
| Uplink bridge | `d2b-b<hash>` | `d2b-b2qnxy4c` |
| Net-VM LAN tap | `d2b-t<hash>` | `d2b-t5f8rw9d` |
| Net-VM uplink tap | `d2b-t<hash>` | `d2b-tp3cjm6e` |
| Workload Guest tap | `d2b-t<hash>` | `d2b-tv8lzq1k` |
| External macvtap | `d2b-t<hash>` | `d2b-te4gnb0s` |

All derived names satisfy the IFNAMSIZ-1 (15-byte) Linux constraint by
construction. The derivation algorithm is identical to
`packages/d2b-host/src/ifname.rs:derive_ifname`, which replaces the legacy
`br-<env>-lan` / `br-<env>-up` naming from `nixos-modules/network.nix`.

Collision detection (`detect_collisions`) is re-run at every reconcile cycle.
A collision is a terminal condition; the controller reports `ReconcileError`
with `reason: ifname-collision` and halts reconciliation of the affected
Network. Operators must rename the Network or conflicting Guest.

The `IfNameMapping` table from `d2b-core/src/host.rs` (line 242) remains a
Core-private resolver input. It is not copied into Network status. Broker
adapters resolve user-visible resource identity to a kernel interface only
after authorization.

### Net-VM Guest lifecycle

The Network controller creates and owns one Guest resource for the net VM:

```yaml
type: Guest
metadata:
  name: net-work-net   # or netVmNameOverride value
  zone: dev
  ownerRef: Network/work-net
spec:
  providerRef: Provider/runtime-cloud-hypervisor
  defaultDomain: system
  allowedDomains: [system]
  budget: { ... }
  # Core resolves the Network owner relationship and supplies tap/macvtap FDs
  # directly to the selected runtime's ProviderSupervisor through an authorized
  # LaunchTicket. Runtime controllers, including runtime-qemu-media, receive
  # only opaque Network/Endpoint refs. No IfName, address, MAC,
  # parentInterface, fd, broker op, or authority key is copied here.
  systemArtifactId: <value-from-Network.spec.netVmSystemArtifactId>
  # Artifact ID referencing the net-VM nixos-system artifact (type=nixos-system)
  # in the artifact catalog. Set from the REQUIRED Network.spec.netVmSystemArtifactId;
  # absent or wrong-type artifact fails closed at Nix build time.
```

The controller reads `Network.spec.netVmSystemArtifactId` (a REQUIRED field validated
at build time) and stores it verbatim in `Guest.spec.systemArtifactId`. The nixos-system
artifact contains only the **generic net-VM OS** - the guest-agent binary and runtime,
kernel, base NixOS services, and systemd-networkd NIC bootstrap with the
`lib.mkForce` override - but NOT per-Network desired data (DHCP reservations,
nftables rules, attachment table, routing policy, or a dnsmasq service).
Per-Network configuration is delivered through a controller-created config
Volume. The net-agent applies the typed guest plan; a separate owned dnsmasq
Process reads its configuration.
See [Config Volume and guest-agent delivery](#config-volume-and-guest-agent-delivery).

Mutations to `Network.spec` that change only DHCP/DNS, firewall, or attachment
configuration update the config Volume, invoke the typed net-agent plan, and
restart the one dnsmasq Process when needed; a Guest
switch or restart is NOT required for config-only changes. Mutations that change NIC
topology (attachment index changes, external attachment add/remove) additionally
require a Guest spec update; `Provider/runtime-cloud-hypervisor` reconciles the
Guest lifecycle accordingly.

The net VM's NixOS config preserves the `lib.mkForce` override on the
`10-eth-dhcp` catch-all network definition. See [Security invariants](#security-invariants).

### Config Volume and guest-agent delivery

The Network controller creates and owns two additional child resources per Network
to deliver per-Network configuration to the net VM at runtime without requiring a
Guest switch on every spec change.

#### Config Volume

```yaml
type: Volume
metadata:
  name: net-<networkName>-config
  zone: <zone>
  ownerRef: Network/<networkName>
spec:
  providerRef: Provider/volume-local
  kind: ephemeral                    # tmpfs-backed; boot-scoped; no persistent backing
  source:
    executionRef: Host/<hostName>    # backing tmpfs allocated on this Host
    settings:
      kind: tmpfs                    # memory-backed; no hostPath; charged to Host memory budget
  quota:
    maxBytes: 4194304                # 4 MiB; tmpfs size= option; kernel-enforced
    maxInodes: 128                   # bounded nonzero; tmpfs nr_inodes= option
    enforcement: hard                # required for tmpfs (kernel enforces unconditionally)
  layout:
    - path: ""                       # Volume root directory
      type: directory
      ownerRef: User/net-local-controller
      groupRef: User/net-local-controller
      mode: "0750"
      accessAcl: []
      defaultAcl: []
      noFollow: true
      createPolicy: create-if-absent
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
    - path: "dnsmasq.conf"           # DHCP reservations, forwarders, domain config
      type: file
      ownerRef: User/net-local-controller
      groupRef: User/net-local-controller
      mode: "0640"                   # owner rw; group r (guest view is read-only)
      accessAcl: []
      defaultAcl: []
      noFollow: true
      createPolicy: create-if-absent
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
    - path: "network-plan.json"      # typed address/route/forward/filter/NAT plan
      type: file
      ownerRef: User/net-local-controller
      groupRef: User/net-local-controller
      mode: "0640"
      accessAcl: []
      defaultAcl: []
      noFollow: true
      createPolicy: create-if-absent
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
    - path: "attachments.json"       # typed attachment reservation table
      type: file
      ownerRef: User/net-local-controller
      groupRef: User/net-local-controller
      mode: "0640"
      accessAcl: []
      defaultAcl: []
      noFollow: true
      createPolicy: create-if-absent
      repairPolicy: exact-owner
      cleanupPolicy: owner-controlled
  views:
    guest-readonly:
      path: ""                       # root of the Volume subtree
      rights: [read, traverse]       # read file contents; traverse to enter directories
  attachments: []
  # attachments is empty on initial create; the Guest attachment is added after
  # the Guest reaches Ready (two-phase sequence - see Delivery lifecycle).
```

The Volume content is bounded: it carries only structured network configuration
rendered from `Network.spec`. No workload VM names, hostnames, or other per-workload
identifiers appear in paths or entries. `views.guest-readonly` grants read and
traverse rights exclusively to the declared Guest virtiofs attachment; the
`read, traverse` set is the minimum required for the agent to read config files.
`User/net-local-controller` is a proper Resource with full lifecycle.
`Provider/network-local`'s Nix package/module provisions the reserved
`net-local-controller` OS account with a private fixed UID/GID in Host
prerequisites and in the generic net-VM nixos-system artifact (same account and
UID/GID inside the Guest), ensuring virtiofs ACLs on config Volume layout entries
are enforced consistently on both sides. The network-local controller creates and
owns the `User/net-local-controller` Resource with `spec.osUsername:
net-local-controller` (`ownerRef: Provider/network-local`, `managedBy: controller`);
`Provider/system-core` verifies the account via NSS lookup and reconciles the User
Resource to Ready - it does not provision the OS account. Numeric UID/GID never
enter any ResourceSpec field, authz check, or audit record; `User.status` MAY carry
diagnostic `uid`/`gid` values discovered by NSS lookup, but those are informational
only and are never authorization inputs. The network-local
controller waits for `User/net-local-controller` to reach `Ready` before creating
any config Volume; this is a reconcile precondition, not a bootstrap side effect.
The tmpfs `quota.maxBytes = 4 MiB` is charged to the Host's memory budget at Volume
creation time.

The Volume is provisioned in two phases:

**Phase 1 - backing ready**: the controller creates the Volume with `source`,
`layout`, and `views` but an empty `attachments` list. The backing tmpfs on the
Host becomes Ready without any Guest attachment. The controller writes the initial
config content through the Volume write service before any Guest exists.

**Phase 2 - Guest attachment**: after the Guest reaches Ready, the controller
updates the Volume to add the attachment entry:
```yaml
attachments:
  - executionRef: Guest/<netVmName>
    transport: virtiofs
    view: guest-readonly
    access: read-only
    mountPath: "/run/d2b/net-config"
    settings:
      posixAcl: false
      xattr: false
      cache: auto             # auto | always | never
      inodeFileHandles: never # never | prefer | mandatory
      threadPoolSize: null    # null → vcpu count of the target Guest
      socketGroup: null       # null → broker-default (runner gid)
```
Only once the attachment reaches Ready may the guest-agent Process be created;
the agent's Volume mount depends on an active attachment.

#### Guest-agent Process

```yaml
type: Process
metadata:
  name: net-<networkName>-agent
  zone: <zone>
  ownerRef: Network/<networkName>
spec:
  providerRef: Provider/system-minijail
  executionRef: Guest/<netVmName>
  domain: system
  processClass: service             # serves typed ApplyPlan/ReadinessQuery only
  template: net-vm-agent
  sandbox:
    namespaceClasses: []            # empty: inherit all parent (Guest) namespaces;
                                    # the Process runs inside the net-VM Guest and
                                    # therefore inherits the Guest's network namespace,
                                    # not the host's; no new CLONE_NEWNET is created
    capabilityClasses: [network-admin, network-raw]
    # network-admin and network-raw apply typed guest network effects.
    # network-bind belongs only to the separate dnsmasq Process.
    # Both are granted within the inherited Guest network namespace only;
    # none confers any capability on the host network namespace.
  mounts:
    - volumeRef: Volume/net-<networkName>-config
      view: guest-readonly
      mountPath: "/run/d2b/net-config"
      access: read-only
      required: true
  budget: { }
```

The net-agent binary is included in the generic nixos-system artifact. After
the controller commits the Volume revision, it calls the typed
`NetworkAgentService.ApplyPlan(digest,generation)` method. The agent strictly
decodes `network-plan.json` and applies typed addresses, routes, forwarding,
filter, and NAT in that order with readback after each stage. It neither watches
the Volume nor starts, signals, or supervises dnsmasq. The separate dnsmasq
Process owns DHCP/DNS lifecycle and `dnsmasq-bound` readiness. The agent reports
only typed plan-stage readiness.

#### Delivery lifecycle

| Event | Controller action |
| --- | --- |
| Network created | Admit Host-global resources; create marked links; create Volume and Guest; attach Volume; create net-agent; apply typed guest plan; create exactly one dnsmasq Process; publish Host aggregates |
| Network spec changes (DHCP/DNS/firewall/attachment config only) | Update Volume content; agent applies the typed plan; restart the one dnsmasq Process when its config changes; no Guest switch required |
| Network spec changes (NIC topology: attachment index add/remove, external attachment) | Update Volume content AND update Guest spec; Guest reconcile handles NIC changes |
| Guest-agent Process fails | Controller observes Process status; re-creates Process on terminal failure; sets `NetVmReady=False/agent-restart` |
| Network deleted | Stop attached Guests; release private taps; delete owned Processes/Guest/Volume; remove Host contributions, routes, addresses, and marked bridges; release Host-global reservations; clear finalizer |

### Bridge and tap lifecycle (host-side)

**Bridge creation and deletion are dynamic broker operations.** `Provider/network-local`
creates and deletes host kernel bridge devices at reconcile time via new closed
broker operations (`CreateBridge` / `DeleteBridge`). A NixOS generation switch
is NOT required to create or remove a Network; the controller provisions all
fabric state at runtime.

Nix provisions schema validation, controller deployment, and static Host policy
only. Runtime NetworkManager entries are owned by the Host-global aggregate
dispatcher; Nix does not emit a per-Network unmanaged writer.

The broker's `CreateBridge` operation:

- creates the kernel bridge device with the derived IfName;
- sets MTU from `spec.mtu` (or 1500);
- disables STP and multicast snooping unconditionally;
- applies IPv6 suppression sysctls
  (`net.ipv6.conf.<ifname>.disable_ipv6 = 1`,
   `net.ipv6.conf.<ifname>.accept_ra = 0`,
   `net.ipv6.conf.<ifname>.autoconf = 0`)
  atomically before returning;
- writes and reads back `IFLA_IFALIAS` ownership, and fsyncs the
  broker-resolved durable ownership record before link-up.

The broker's `DeleteBridge` operation removes only the kernel bridge device,
after every persistent tap has been confirmed removed through
`DeletePersistentTap`. It never cascades deletion to an attached tap: a
remaining d2b-owned tap is retryable after tap cleanup, and a foreign
port/marker fails closed. It is idempotent when the bridge is already absent.

The network-local controller performs the following runtime effects through the
broker's typed effect interface:

| Broker op | Current source | Purpose |
| --- | --- | --- |
| `CreateBridge` / `DeleteBridge` | Present preview broker ops | Create/delete only durable-marked bridges with MTU and readback; never cascade tap deletion |
| `ReconcileNetworkAddress` | **New closed broker op** | Apply/remove a typed Host address and verify readback; accepts no command string |
| `ReconcileNetworkRoutes` | **New closed broker op** | Apply the Host-global typed route aggregate with durable protocol/record ownership |
| `ApplyNftablesProjection` | Present preview closed op, invoked only by Host-global dispatcher | Apply/remove one composed ownership projection; preserve sibling/device/foreign markers |
| `ApplyNmUnmanaged` | Existing closed op, invoked once per Host aggregate | Replace one marker-delimited aggregate and preserve foreign bytes |
| `ApplySysctl` | `d2b_contracts::broker_wire::ApplySysctlRequest`; `d2b-host/src/netlink.rs` | Per-bridge IPv6 suppression defense-in-depth (re-applied after networkd restart or sysctl drift) |
| `CreatePersistentTap` | Existing closed broker op | Create/adopt the persistent tap for an opaque attachment realization |
| `DeletePersistentTap` | **New closed broker op** paired with `CreatePersistentTap` | Delete exactly one opaque, generation-fenced, d2b-owned persistent tap; validated absence is success |
| `SetBridgePortFlags` | `d2b_contracts::broker_wire::SetBridgePortFlagsRequest`; `d2b-host/src/bridge_port.rs` | Isolated/neigh-suppress per-tap after tap creation |
| `UpdateHostsFile` | Existing closed op, invoked once per Host aggregate | Replace one marker-delimited aggregate and preserve foreign bytes |

IPv6 is suppressed per-bridge at `CreateBridge` time (atomically by the broker)
AND via `ApplySysctl` at each reconcile cycle (defense-in-depth, handling
`systemctl restart systemd-networkd` and any other sysctl drift). No
boot-time Nix sysctl entry is required for specific bridge IfNames because
bridges are created dynamically and do not exist at host activation.

**Persistent-tap creation** uses one operation chain. The network-local
controller declares an opaque attachment realization through
`NetworkEffectPort`. Its Core-owned adapter maps that semantic effect to
`CreatePersistentTap`, applies `SetBridgePortFlags`, and transfers the
already-authorized connected `OwnedFd` directly to ProviderSupervisor for the
selected Guest runtime's Process LaunchTicket. For QEMU, the qemu
Provider/controller receives only opaque Network/Endpoint refs; it receives no
broker operation or fd. The fd is never represented in a ResourceSpec/status,
serialized on d2b-bus, or delivered through a Provider controller
ComponentSession.

`CreateTapFd` is a distinct baseline operation-scoped, non-persistent
SCM_RIGHTS path. This ADR does not use it for Network attachment realizations;
the retained Network lifecycle and generation-fenced deletion require the
canonical `CreatePersistentTap`/`DeletePersistentTap` pair.

The Network adapter owns the connected `OwnedFd` with `FD_CLOEXEC` set until
ProviderSupervisor accepts it. ProviderSupervisor keeps its parent copy
CLOEXEC, makes only the declared child fd slot inheritable immediately before
exec, and closes its copy after successful spawn. The Guest VMM owns the child
copy until exit. Cancellation, LaunchTicket rejection, or spawn failure closes
every copy before the adapter invokes generation-fenced
`DeletePersistentTap`; the opaque realization remains retained until deletion
is confirmed. On attachment removal or Network finalization, network-local
likewise waits for the Guest/VMM fd owner to close before invoking
`DeletePersistentTap`.

`DeletePersistentTapRequest` contains only an opaque attachment ID,
`expectedNetworkGeneration`, and `expectedAttachmentGeneration`. It accepts no
IfName, path, or caller-authored marker. The broker resolves trusted private
realization state, validates both generations and the d2b ownership marker,
then deletes only that tap. Already-absent is idempotent success only when the
trusted record and marker state show no foreign replacement; a stale
generation or foreign marker fails closed without deletion. The controller
retains only the injected port's private associated capability and retries
retryable failures before consuming it on confirmed deletion.

### DHCP and DNS lifecycle (inside net VM)

Exactly one `Process/net-<networkName>-dnsmasq` runs inside the net VM. It is a
Network-owned worker supervised through the Process resource lifecycle, not a
child of the net-agent. The Network controller writes `dnsmasq.conf` to the
config Volume and restarts that Process after address and route readiness. The
generic nixos-system artifact declares no dnsmasq service.

Key dnsmasq invariants preserved from `nixos-modules/net.nix` lines 302-441,
now encoded in the controller-rendered `dnsmasq.conf` Volume entry:

- `bind-interfaces = true` binds only to `eth1` (LAN interface);
- `dhcp-ignore-names = true` prevents hostname spoofing;
- static DHCP host reservations are derived from `spec.attachments[]`;
- DHCP dynamic pool covers `lanCidr.251`-`lanCidr.254`;
- DNS forwarders are set from `spec.dns.forwarders`;
- the dnsmasq Process runs under the dedicated system user with hardened
  confinement (preserving the hardening from net.nix lines 363-441).

The `DhcpReady` condition is set by the network-local controller observing the
dnsmasq Process status and its `dnsmasq-bound` readiness predicate.

### Firewall and NAT lifecycle

**Host side** (`inet d2b` table, `d2b-host/src/nftables.rs`):

The network-local controller publishes one typed contribution to the
Host-global Core nftables dispatcher. Only that dispatcher calls the broker
`ApplyNftablesProjection` operation (see D-NETWORK-004). This op
mutates only the rules bearing that Network UID's ownership marker inside the
shared `inet d2b` table and byte-preserves every other marker; it never deletes
and recreates the whole table. The shipped `ApplyNftables` op discards
`ownership_id` and does a whole-table atomic replace, so mapping per-Network
firewall reconciles onto it would make independent Networks last-writer-wins and
would erase device-usbip rules; D-NETWORK-004 records why the projection-scoped
op is introduced instead. The `inet d2b` table:

- blocks all traffic on LAN bridges (host has no IP there);
- installs per-rule `comment "d2b managed: <ownership-id>"` markers
  (ownership ID is the Network resource UID);
- coexists with other firewall managers per the `FirewallCoexistencePolicy`
  (Coexist/Refuse/RequireUnmanaged matrix preserved from
  `packages/d2b-host/src/nftables.rs`).

Because the dispatcher composes the full contribution set and the broker op is
projection-scoped, independent Provider reconciles cannot erase one another.
The ordered OFD lock on the `inet d2b` table serializes mutations. The
generation fence does not serialize and has no compare-and-advance behavior; it
only rejects and requeues an intent whose `expected_generation_id` names a
superseded installed configuration generation. Same-generation mutations
converge idempotently under the lock. Network-local emits no TCP/3240 or other
USBIP allow rule. Its
`status.provider.details.firewallDigest` is the SHA-256 of the canonical
projection containing only rules owned by that Network UID. USBIP-owned chains,
rules, and ownership markers are excluded, so USBIP attach/detach cannot create
false Network drift. Network-local compares only this ownership-scoped digest
on each observe cycle.

**Net VM side** (typed plan delivered through the config Volume):

The Network controller writes a strict versioned `GuestNetworkPlan` to
`network-plan.json`. The net-agent validates its digest/generation and applies
typed addresses, routes, forwarding, filter, and NAT in that order with
readback after each stage. No Provider-authored command, route string, or nft
script is executed. The plan preserves these semantics from
`nixos-modules/net.nix`:

- `ip6 filter` table with policy drop on all chains (d2b is IPv4-only);
- `inet filter input` chain with stateful connection tracking;
- LAN DHCP/DNS accept (UDP/53, UDP/67, TCP/53 on `eth1`);
- ICMP echo rate-limited to 10/s burst 20;
- `inet filter forward` chain with per-env east-west, hostBlocklist drop,
  internet egress allow, external-network egress CIDRs;
- MSS clamp rule when `spec.mssClamp = true`;
- `inet nat postrouting` MASQUERADE on `eth0` (internet uplink);
- optional `MASQUERADE` on `external0` when egress.masquerade is true;
- optional DNAT prerouting rules from `spec.externalAttachment.portForwards`.

IPv6 is dropped on all chains in the net VM as well as suppressed at the
interface level via `disable_ipv6 = 1` sysctl.

### External network attachment lifecycle

When `spec.externalAttachment` is non-null, the network-local controller:

1. Core resolves the declared external-attachment intent from the owning
   Network, admits its Host-global physical-NIC authority, and records only an
   opaque claim/owner proof. The Network controller does not copy
   `parentInterface` or an authority key into the net-VM Guest spec and does not
   call a macvtap creation broker op. No `CreateMacvtap` broker op exists in the
   v3 baseline.
2. Core supplies the admitted attachment intent to
   `Provider/runtime-cloud-hypervisor` through the private owner/dependency
   resolver and an authorized LaunchTicket. The runtime calls `SpawnRunner` on
   the broker. The broker creates the macvtap FD internally (`live_create_macvtap_fd` in
   `packages/d2b-priv-broker/src/runtime.rs` line 5097) as part of VMM spawn
   dispatch, resolving the macvtap intent from `processes.json` fields
   (`ProcessMacvtapInterface` in `packages/d2b-core/src/processes.rs`).
3. The net-agent applies the validated typed external address and route effects
   for `external0`.
4. Port-forward DNAT and masquerade are typed NAT effects applied only after
   address, route, and forwarding readback.
5. Egress CIDRs enter typed filter and route effects. The d2b-created macvtap
   child receives the effective MTU; d2b never changes its physical parent.

Before step 1, Core resolves `parentInterface` against trusted Host inventory
and admits the Host-global `external-physical-nic/v1` authority claim. No
macvtap or VMM effect is permitted until `ExternalNicAuthorityReady=True`.
`passthru`, `private`, and `vepa` claims are exclusive. `bridge` is exclusive
unless every concurrent claimant explicitly requests compatible multiplexing and
all such claimants share one isolation domain (one Zone UID). A same-Zone
exclusive collision or mixed policy sets
`ExternalNicAuthorityReady=False/external-physical-nic-conflict`; a cross-Zone
`bridge` multiplex attempt sets
`ExternalNicAuthorityReady=False/external-physical-nic-cross-zone-l2`
(INV-NET-010). Either fails closed and performs no host effect.

`parentInterface`, `macvtapMode`, or sharing-policy changes require a disruptive
drain/recycle. Core drains the net VM and dependent attachments, releases the
old authority only after the old macvtap FD is closed, admits the replacement
claim, and then starts the replacement. Delete follows the same order and
releases the claim last. Restart adopts only an exact resource/process
`ownerProof`; ambiguity quarantines rather than creating a second attachment.
The `ExternalAttachmentReady` condition reflects the macvtap interface state
observed through the net VM's Guest readiness predicates, while
`ExternalNicAuthorityReady` reports authority admission/adoption.

When `spec.mdns.enable = true`, the Network controller creates owned `Process`
resources for the mDNS reflector (avahi) and, when `dnsmasqLocal = true`,
the local DNS bridge, each with `executionRef: Guest/<netVmName>`. Foundation
requires every ordinary process to be a Process or EphemeralProcess resource;
mDNS is not an inline untracked service. The Process spec parameters derive
entirely from the `spec.mdns.*` fields; no manual lifecycle management
is required.

### USBIP proxy boundary

The USBIP backend and proxy processes are **not** owned by the Network
controller. They are owned by `Provider/device-usbip`. Its controller may watch
only the referenced Network identity, phase, and generation. Core privately
resolves that Network UID to the relay attachment; no bridge IfName, host
uplink address, route table, or firewall body crosses the Provider boundary.

The device-usbip controller consumes `Network/work-net` through a
`networkRef` dependency, owns exactly one multiplexed relay `Endpoint` authority
per Network, declares its own backend/relay/Binding-proxy `Process` resources,
and invokes its typed `UsbipEffectPort`. The Core adapter alone resolves the
opaque per-Network/per-busid intent and dispatches the shared
`ApplyNftablesProjection` broker operation with the closed action enum
`Apply|Remove` (D123, D124); the shipped `UsbipBindFirewallRule`/`bind_firewall_rule`
op has no release path, so its `Remove` action is net-new privileged surface. That
path owns all USBIP TCP/3240
exposure, ownership markers, drift observation, and status. A Binding proxy
receives an authorized connected relay stream through Endpoint resolution and
its LaunchTicket; network-local therefore emits no generic TCP/3240 allow rule
on the host or in the net VM.

`Network.spec` has no `usbipCarveOut` or device-usbip extension field and must
not be mutated by device-usbip. Network-local's provider firewall digest and
the Network `FirewallReady` condition exclude USBIP-owned rules; USBIP drift is
reported only by the owning Service's strict `status.provider`.

## Host/Guest attachment mechanics

### How Guests attach to a Network

A Guest (workload VM) requests attachment to a Network by being listed in
`Network.spec.attachments`:

```yaml
# In Network/work-net spec:
attachments:
  - executionRef: Guest/corp-vm
    index: 10
  - executionRef: Guest/personal-vm
    index: 11
```

The attachment is owned by the Network, not the Guest. When a listed Guest
exists and Core validates it as launch-eligible, the network-local controller
declares the opaque attachment effect. The Core adapter performs
`CreatePersistentTap → SetBridgePortFlags` and routes the connected CLOEXEC
`OwnedFd` directly into ProviderSupervisor's LaunchTicket attachment; the
runtime controller never receives the fd or broker operation. The Guest's own
spec references the network for firewall/routing/sandbox purposes:

```yaml
# In Guest/corp-vm spec (inline ExecutionPolicy field):
networks:
  - networkRef: Network/work-net
    # The controller reconciles this ref and validates it matches an
    # attachment entry in the Network spec.
```

If a Guest is listed in `spec.attachments` but its `Guest` resource does not
exist, the attachment remains `Pending`; the Network is not blocked from
becoming `Ready` on its other conditions. Removal of an attachment entry waits for the Guest/VMM FD owner to close, then
triggers `DeletePersistentTap` with the retained opaque attachment ID and the
expected Network/attachment generations. The request has no IfName/path input;
validated absence succeeds, while a stale generation or foreign ownership
marker fails closed without deleting anything.

### Process network attachment

A Process resource running inside a Guest that is attached to a Network
inherits its network connectivity through the Guest's tap interface. The
Process spec declares network usage settings:

```yaml
# In Process spec:
network:
  networkRef: Network/work-net
  ingressPorts: []
  egressPolicy: inherit   # inherit | blocked | explicit
```

The Process Provider (system-minijail or system-systemd) enforces the declared
egress policy through the sandbox profile. No Process has direct access to the
host bridge or tap; all network reachability flows through the net VM.

### Host attachment

A Host may reference a Network to declare site policy. The network-local
controller validates that the selected Host resource grants the controller
permission to install host bridges, nftables rules, and sysctl entries.
The exact attachment field in the Host resource spec is defined by the Host
ResourceType spec; this Network spec does not invent or depend on a specific
`Host.spec.networks` field name.

Core admits CIDRs against the Host resource's observed network inventory and
every Network in every Zone on the Host. A controller may perform an
effect-free local preflight for diagnostics, but only the atomic Host-global
reservation authorizes effects. At Nix build time, evaluation rejects every
collision visible in the declared Host configuration; runtime admission closes
the gap for observed and concurrently created state.

## CIDR allocation and validation

### Constraints (all enforced by the network-local controller)

| Field | Constraint |
| --- | --- |
| `lanCidr` | Must be exactly `/24`; base address must end in `.0` |
| `uplinkCidr` | Must be exactly `/30` |
| `lanCidr` ↔ `uplinkCidr` | Must not overlap within the same Network |
| Any Network `lanCidr` or `uplinkCidr` ↔ any other Network CIDR | Must not overlap anywhere on the Host |
| Any Network CIDR ↔ Host resource network inventory | Must not overlap |
| `externalAttachment.egress.allowedCidrs` ↔ any Host-global Network CIDR or Host inventory CIDR | Must not overlap |
| Attachment `index` | 2-250 inclusive; unique within the Network |

CIDR overlap uses the same two-prefix IPv4 arithmetic as `cidrOverlaps` in
`nixos-modules/lib.nix` lines 429-462: two CIDRs overlap if and only if their
shorter prefix matches when applied to both network addresses. Containment
counts as overlap.

Validation and admission run at:

1. `validateSpec` call in the reconcile async interface - before any host-side
   effect. This checks one object's shape and returns a `ValidationResult` with
   stable code `network-cidr-conflict` and sets the `CidrConflict` condition.
2. Nix evaluation - rejects every collision visible in the complete declared
   Host configuration.
3. Core Host-global compare-and-reserve - atomically checks observed Host
   inventory and all Zones, then durably reserves the exact CIDRs before any
   effect. A concurrent create has exactly one winner.

The same CIDR rules apply to `externalAttachment.egress.allowedCidrs` entries.
A port-forward `sourceCidrs` entry must not overlap a Host-global Network CIDR
(prevents accidental cross-env routing).

### Env name / interface name constraints

The controller enforces at validateSpec time:

- `metadata.name` regex: `^[a-z][a-z0-9-]*$` (standard ResourceName);
- Effective LAN bridge name ≤ 15 bytes after IfName derivation (guaranteed by
  construction; verified via `detect_collisions`);
- `netVmNameOverride`, if set, must match `^[a-z][a-z0-9-]*$` and must not be
  `launcher` or start with `sys-`.

## Isolation, hierarchy, and budgets

### East-west isolation

East-west reachability requires the exact double opt-in
`Network.spec.isolation.allowEastWest &&
d2b.site.allowUnsafeEastWest`. Both inputs default false.

When `isolation.allowEastWest = false` (default), workload taps on the LAN
bridge are set to `Isolated = true` and the net-VM forward chain has no
`eth1→eth1 new accept` rule, preventing direct L2 communication between
workloads in the same Network.

When `isolation.allowEastWest = true` and the Host/site input is false, Core
refuses before mutation with `east-west-site-opt-in-required`. When both are
true, the controller:
1. Sets workload tap isolation bits to `Isolated = false` via broker
   `SetBridgePortFlags`.
2. Includes the east-west accept rule in the net VM's forward chain.

Bridge isolation is enforced at two layers:
- **Host kernel**: tap entries in the LAN bridge carry `Isolated = true` by
  default (all workload taps), preventing direct L2 frames between workloads.
  Only the net-VM tap is non-isolated (it can reach all workload taps).
- **Net-VM nftables**: the forward chain has no `eth1→eth1 new accept` rule
  unless both opt-ins are true.

Changing `allowEastWest` from true to false requires a full reconcile to
restore bridge isolation and apply the typed guest filter plan. It does not
rebuild the generic net-VM system artifact.

### hostBlocklist invariant

The effective `hostBlocklist` is:

```text
spec.routing.hostBlocklist
  ∪ { Host resource's observed network inventory (runtime fact; schema-neutral) }
  ∪ { lanCidr, uplinkCidr of every other active Network on the Host }
```

The controller computes this union at each reconcile cycle by querying the
Host resource's observed network inventory (a runtime-observable fact from the
Host resource, as defined by the Host ResourceType; at Nix build time the eval
may validate against declared host configuration input where available). Each
entry generates a `drop` rule in the net VM's forward chain (before the broad
`lan→internet accept`). This prevents workloads from routing to:
- host LAN ranges (from the Host resource's observed network inventory);
- other d2b networks on the Host;
- link-local and other RFC-reserved ranges in the default list.

The hostBlocklist cannot be entirely emptied; it is only additive relative
to the default RFC1918+link-local set.

### Network budget

The Network resource does not have its own compute/memory budget (it is not
itself a Process). Resource budgets for the net-VM Guest and the controller
Process are declared on those resources. The Network spec only declares
interface-level policy (MTU, isolation, egress) that informs the net VM's
resource envelope.

### Multiple Networks in one Zone

A Host may have several Zones and each Zone may have several Networks, all with
Host-globally distinct CIDRs, derived kernel names, and vsock CIDs. Networks
remain Zone-local Resource objects, but kernel namespace admission is
Host-global. There is no parent Network resource; Networks are peers.

## Async reconcile/observe/adopt/delete

The network-local controller implements the full reconciliation contract from
`ADR-046-resource-reconciliation`.

### Reconcile

1. Validate the one canonical base schema, external IPv4 and forward targets,
   attachment indices, and the two east-west inputs.
2. After `CommittedRevisionProof`, atomically acquire the Host-global CIDR,
   derived-name, vsock-CID, and external-NIC reservations. A failed reservation
   performs no effect.
3. Publish this Network's desired NetworkManager contribution. The Host-global
   aggregate owner applies the complete marker-delimited projection and reloads
   NetworkManager before link creation.
4. Through typed effect intents, create or adopt both marked bridges down,
   propagate MTU, apply bridge flags and IPv6 suppression, verify readback, and
   bring them up. Assign only the typed Host uplink address at this stage.
5. Create or adopt generation-fenced taps. Set the same effective MTU and the
   bridge isolation role chosen by the exact double opt-in, verify flags, and
   pass each connected CLOEXEC `OwnedFd` only through a LaunchTicket.
6. Create the owned bounded config Volume and write canonical typed guest-plan
   content plus dnsmasq configuration. Create the net-VM Guest using the
   required system artifact and the privately allocated Host-global vsock CID.
   Attach the read-only Volume only after both resources are Ready.
7. Create one owned net-agent Process. It applies and reads back guest
   addresses, routes, IPv4 forwarding, filter rules, and NAT in that order.
   Create exactly one separate owned dnsmasq Process only after address and
   route readiness. Create the optional mDNS Processes when requested.
8. After dnsmasq reports `dnsmasq-bound`, publish the typed Host-route,
   nftables, and `/etc/hosts` contributions. Their Host-global aggregate owners
   apply complete desired projections while preserving foreign bytes and
   ownership markers.
9. For each attached workload Guest, require LaunchTicket FD consumption, NIC
   MTU readback, DHCP or reserved-address reachability, and the requested
   isolation state. Removed attachments wait for VMM FD closure before the
   private handle is consumed by generation-fenced tap deletion.
10. Commit the child-resource batch and layered status only after every
    dependency/readback barrier required for that phase holds.

Long-running operations (nftables apply, sysctl, route) are dispatched in
background tasks through bounded blocking adapters. The reconcile handler does
not hold a redb transaction across these calls.

### Observe

The controller declares an observe interval for external drift detection:
`observeInterval: 60s`. On each observe cycle:

- Re-read the applied `inet d2b` table digest via broker and compare against
  `status.provider.details.firewallDigest`, considering only Network-UID-owned
  rules. If drift, set `FirewallReady=False` and queue a reconcile. Ignore
  every device-usbip ownership marker; that Provider owns USBIP drift/status.
- Check bridge IPv6 sysctl values against expected. If drift, queue reconcile.
- Check bridge isolation flags via broker readback. If drift, queue reconcile.
- Verify the durable Host-global CIDR, name, and vsock reservations still match
  the exact owner proof.
- Re-check the external-NIC authority owner proof and compatible holder policy.
  If missing, ambiguous, or conflicting, set
  `ExternalNicAuthorityReady=False` and do not recreate the macvtap.

Observation results are committed as status-only updates without incrementing
generation.

### Adopt

On controller restart (continuation event), the controller:

1. Lists all Network resources in every Zone assigned to the Host and reads the
   durable Host-global authority reservations.
2. For each Network, reads current bridge, route, address, isolation, nftables,
   NetworkManager, and hosts state through typed broker observations.
3. For an external attachment, asks Core to adopt the exact Host-global
   physical-NIC authority by resource/process owner proof. Ambiguity
   quarantines the attachment.
4. Adopts a bridge or route only when its live kernel marker, durable ownership
   record, exact desired key, and resource owner proof all agree. A mismatch
   blocks without deleting or replacing the live object.
5. Adopts Host-global aggregate projections only when their marker boundaries,
   contribution set, generation, and digest agree. The net-VM Guest and owned
   Process lifecycles are separately adopted by their owning controllers.
6. If owned state is absent, the normal reconcile loop recreates it after the
   reservation and ordering barriers.

Adoption does not modify any running state; it only updates the controller's
internal observed state. No bridge or tap is deleted during adoption.

### Delete

The finalizer `network.d2bus.org/fabric-cleanup` is owned by the network-local
controller. On deletion (strictly child-first order):

1. Controller receives `deletion-requested` trigger.
2. Sets `NetworkDraining` condition; updates attached workload Guest resources to
   request their own deletion (through their owner chain, not directly).
3. Waits for all attachment-phase statuses to become non-Ready (workload Guests
   are stopped by their own controllers).
4. Calls `DeletePersistentTap` for every retained attachment realization using
   its opaque attachment ID and expected Network/attachment generations. Each
   tap is removed only after its Guest/VMM FD owner has closed. Validated absence
   is success; transient failures retain the handle and retry; stale generations
   refresh and requeue; a foreign ownership marker blocks cleanup.
5. Requests deletion of the owned net-agent, the one owned dnsmasq Process, and
   any owned mDNS Process resources. Waits for their Deleted watch events
   (each Deleted step is a single store transaction: the REVISION event with
   `phase = Deleted` and row/index removal happen atomically; there is no persistent
   phase=Deleted row for the controller to observe).
6. Updates the Volume to remove the Guest attachment entry (sets `attachments: []`);
   waits for the attachment removal to be confirmed. This unbinds the read-only view
   from the net-VM before the Guest is stopped.
7. Deletes the owned net-VM `Guest/<netVmName>` resource; waits for the Deleted
   watch event. The net VM's macvtap FD (external attachment, if any) is released
   as part of the VMM teardown inside `Provider/runtime-cloud-hypervisor` - the
   broker destroys the macvtap interface when the SpawnRunner child exits.
8. Deletes the owned `Volume/net-<networkName>-config` resource; waits for the
   Deleted watch event. At this point the Guest attachment has already been removed
   (step 6) and the Volume backing is released cleanly.
9. Removes this Network's nftables, NetworkManager, and hosts contributions.
   Each Host-global owner applies the recomputed aggregate, preserving sibling
   and foreign bytes.
10. Removes the marked Host route, typed addresses, and empty marked bridges
    in reverse dependency order. A marker or durable-record mismatch blocks
    deletion; bridge deletion never cascades into an attached link.
11. Releases the Host-global external physical-NIC authority after the
    VMM/macvtap is gone. For a compatible same-Zone multiplex, authority
    ownership transfers atomically to the oldest remaining holder.
12. Releases Host-global CIDR, derived-name, and vsock-CID reservations.
13. Clears the finalizer.

Deletion is strictly child-first. If any owned child (Process, Guest, or Volume)
cannot be deleted (finalizer blocked by a dependency), the Network deletion is
blocked and the `NetworkDraining` condition describes the blocker.

## RBAC

### Roles

```yaml
type: Role
metadata:
  name: network-operator
  zone: dev
spec:
  rules:
    - resourceTypes: [Network]
      verbs: [get, list, watch, create, update-spec, delete]
      zones: [dev]
    - resourceTypes: [Zone]
      verbs: [get]        # read Zone for topology inspection
      zones: [dev]
```

```yaml
type: Role
metadata:
  name: network-reader
  zone: dev
spec:
  rules:
    - resourceTypes: [Network]
      verbs: [get, list, watch]
      zones: [dev]
```

```yaml
type: Role
metadata:
  name: network-local-controller
  zone: dev
spec:
  rules:
    - resourceTypes: [Network]
      verbs: [get, list, watch, update-status, update-finalizers]
      zones: [dev]
    - resourceTypes: [Guest]
      verbs: [get, list, watch, create, update-spec, delete]
      zones: [dev]
      # Scoped to Guests whose ownerRef resolves to a Network resource.
    - resourceTypes: [Volume]
      verbs: [get, list, watch, create, update-spec, delete]
      zones: [dev]
      # Scoped to Volumes whose ownerRef resolves to a Network resource.
    - resourceTypes: [Process]
      verbs: [get, list, watch, create, update-spec, delete]
      zones: [dev]
      # Scoped to Processes whose ownerRef resolves to a Network resource
      # (guest-agent, mDNS reflector, local DNS bridge).
    - resourceTypes: [Host]
      verbs: [get]         # read Host network inventory for hostBlocklist computation
      zones: [dev]
```

The `network-local-controller` role is bound to the authenticated process
subject `Provider/network-local` through:

```yaml
type: RoleBinding
metadata:
  name: network-local-ctrl-binding
  zone: dev
spec:
  roleRef: Role/network-local-controller
  subjects:
    - Provider/network-local
  externalPrincipalSelector: null
  scopeNarrowing: null
```

Guest controllers may need `get` on `Network` to resolve a `networkRef` in
Guest/Process specs. The relevant Guest Provider role includes:

```yaml
    - resourceTypes: [Network]
      verbs: [get]
      zones: [dev]
```

No Process or Guest has `update-spec` on Network; attachment changes always
require an explicit operator `UpdateSpec` on the Network resource.

### Status ownership

The `network-local` controller is the sole `update-status` owner for
`Network` resources. Status updates carry `observedGeneration` and expected
revision. The Guest-owned child resources are status-owned by their respective
Provider controllers.

## Audit, OTEL, and redaction

### Audit records

The resource API emits one audit record per mutation with the standard fields
from `ADR-046-resource-api-and-authorization`. Network-specific additions:

| Field | Included | Rationale |
| --- | --- | --- |
| ResourceType and resource name | Yes | operational identity |
| verb / subresource | Yes | standard |
| `network.lanCidr` | Yes | address allocation decision |
| `network.uplinkCidr` | Yes | address allocation decision |
| `network.isolation.allowEastWest` | Yes | security-relevant policy change |
| `network.attachments[].executionRef` | Yes | Guest identity is operational |
| Workload hostname, IP, MAC | **No** | redacted from API-level audit |
| nftables rule text | **No** | redacted from API-level audit |
| DHCP lease data | **No** | never written to audit |
| dnsmasq config contents | **No** | not audit material |
| `externalAttachment.portForwards[].targetIp` | **No** | workload-internal |
| Operator-declared `externalAttachment.parentInterface` | Yes, on spec mutation only | requested Host inventory selector; never the derived authority identity |
| Runtime IfName values | **No** | Core-private bridge/tap/macvtap identity |
| firewallDigest | Yes | drift evidence |
| External-NIC authority key/owner proof | **No** | Core-private authority material |
| Bridge/tap drift reason | Yes (stable code, no paths) | diagnostic |

Broker operations (`ApplyNftablesProjection`, `ApplyNmUnmanaged`, `ApplyRoute`,
`ApplySysctl`, `CreatePersistentTap`, `DeletePersistentTap`,
`SetBridgePortFlags`, `UpdateHostsFile`, `SeedDnsmasqLease`, etc.) emit their
own audit records with path-free outcome codes. `DeletePersistentTap` audit is
post-effect and contains only the exact op name, an opaque attachment digest,
the expected Network/attachment generations, outcome, error class, and
correlation ID. It contains no attachment-handle bytes, IfName, path, or
ownership-marker body. `ApplyNftablesProjection` audit is likewise post-effect
and carries only the op name, an opaque projection digest, the expected
projection generation, action, outcome, error class, and correlation ID; it
never carries rule text, an IfName, a path, or the ownership-marker body.

### OTEL spans and metrics

Network reconcile cycles emit one root span per Network resource per reconcile
attempt:

```
d2b.network.reconcile
  network.generation: <generation>
  reconcile.trigger: <reason-set>
  reconcile.attempt: <n>
  outcome: converged | pending | degraded | failed-retryable | failed-terminal
```

Child spans for broker operations:

```
d2b.network.bridge.create
d2b.network.bridge.delete
d2b.network.nftables.apply
d2b.network.volume.sync
d2b.network.guest.sync
d2b.network.agent.sync
d2b.network.mdns.sync
```

Metric labels use closed semantic cardinality and contain no Zone or Network
identity. No workload IP, MAC, hostname, nftables rule text, or DHCP lease data
appears in any span attribute, metric label, or log field. Zone identity
remains in the `d2b.zone` OTEL resource attribute. Network identity is likewise
available only as a bounded OTEL resource attribute and permitted audit field,
never as a span attribute or metric label.

Metrics:

| Metric | Labels |
| --- | --- |
| `d2b_network_reconcile_total` | `outcome` |
| `d2b_network_phase` | `phase` |
| `d2b_network_attachment_count` | (none) |
| `d2b_nftables_apply_total` | `outcome` |
| `d2b_nftables_drift_total` | (none) |
| `d2b_bridge_create_total` | `outcome` |
| `d2b_bridge_delete_total` | `outcome` |
| `d2b_network_volume_sync_total` | `outcome` |
| `d2b_network_agent_restart_total` | `outcome` |

## Security invariants

The following invariants are normative. Any change to the network-local
Provider or net-VM template that would violate them is a panel-blocking finding.

### INV-NET-001: lib.mkForce on 10-eth-dhcp

**Invariant**: the net VM's generated NixOS config MUST contain a
`lib.mkForce` override that replaces the `10-eth-dhcp` catch-all networkd
network definition with a non-matching bogus MAC address
(`00:00:00:00:00:00`).

**Rationale**: `nixos-modules/base.nix` declares `10-eth-dhcp` with
`matchConfig.Type = "ether"`, which DHCP-configures every Ethernet NIC.
The net VM has two NICs (uplink, LAN) that require static addressing. Without
the override, systemd-networkd would DHCP both NICs, breaking static addressing.

**Implementation**: the net-VM nixos-system artifact contains this
override unconditionally. It is not parameterized; it cannot be disabled by
any `netVmSystemArtifactId` substitution or operator field.

**Test**: `tests/unit/nix/cases/net-vm-network.nix` must assert that every
generated net-VM config contains
`config.systemd.network.networks."10-eth-dhcp".matchConfig.MACAddress == "00:00:00:00:00:00"`.

### INV-NET-002: IPv6 suppression at bridge creation and reconcile time

**Invariant**: every host bridge created by the network-local controller MUST
have `net.ipv6.conf.<ifname>.disable_ipv6 = 1`,
`net.ipv6.conf.<ifname>.accept_ra = 0`, and
`net.ipv6.conf.<ifname>.autoconf = 0` applied in two independent layers:

1. **At bridge creation**: the broker `CreateBridge` operation applies these
   sysctls atomically before returning, closing the race window between
   interface creation and the controller's next step.
2. **At each reconcile cycle**: the controller re-applies via broker
   `ApplySysctl` as a defense-in-depth step, handling `systemctl restart
   systemd-networkd`, manual sysctl resets, or any other drift path.

No boot-time Nix `boot.kernel.sysctl` entry per bridge IfName is required
because bridges are created dynamically and do not exist at host activation.

**Rationale**: dynamically-created kernel interfaces inherit `/proc/sys/net/
ipv6/conf/default/*` defaults, which may have IPv6 active. The broker's
`CreateBridge` baseline closes the race; the `ApplySysctl` reconcile step
closes any drift window introduced by subsequent host operations.

**Test**: `packages/d2b-priv-broker/tests/create_bridge_applies_ipv6_sysctl.rs`
asserts the broker applies all three sysctls before `CreateBridge` returns.
`packages/d2b-provider-network-local/tests/reconcile_applies_sysctl_defense_in_depth.rs`
asserts the controller re-applies via `ApplySysctl` independently.

### INV-NET-003: Bridge port isolation default

**Invariant**: workload Guest taps on the LAN bridge MUST have
`Isolated = true` (kernel bridge isolation flag) by default. The net VM's
LAN tap MUST have `Isolated = false`. Only when
`Network.spec.isolation.allowEastWest = true` **and**
`d2b.site.allowUnsafeEastWest = true` may workload taps be set to
`Isolated = false`.

**Rationale**: L2 isolation prevents direct workload-to-workload frames even if
the net VM's forwarding rules allow it. ADR 0012 requires both the per-Network
request and the independent Host/site acknowledgement.

**Test**: `packages/d2b-host/src/bridge_port.rs` bridge-port conformance tests;
the four-case production matrix in
`tests/host-integration/network-local-data-plane.nix`.

### INV-NET-004: hostBlocklist cannot be emptied

**Invariant**: the effective hostBlocklist must always contain at least the
default RFC1918+link-local set:
`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`, `169.254.0.0/16`.

**Rationale**: these prevent workloads from routing to host infrastructure
networks. The operator may add entries via `spec.routing.hostBlocklist`; the defaults cannot be removed.

**Implementation**: the controller enforces the default set during
`validateSpec`; any spec that specifies a `hostBlocklist` missing any of the
four defaults is rejected with `network-spec-invalid`.

### INV-NET-005: IPv4-only by construction

**Invariant**: d2b networking is IPv4-only. IPv6 is explicitly dropped on all
chains in the net VM nftables ruleset and suppressed on all bridges. The
Network spec must not expose IPv6 CIDR, address, or policy fields.

**Rationale**: no current v3 baseline feature requires IPv6. IPv6 dual-stack
is a separate future feature requiring its own reviewed ADR.

**Note**: when IPv6 dual-stack is added, INV-NET-002 and INV-NET-005 must both
be updated in a joint change.

### INV-NET-006: No workload identifiers in host nftables rules

**Invariant**: the `inet d2b` table installed on the host MUST NOT contain
workload VM names, user identifiers, or DHCP-assigned hostnames. Rules
reference only bridge IfNames, CIDR prefixes, IP addresses, and ports.

**Rationale**: the ownership-marker convention requires that foreign-rule
preservation be enforced fail-closed. Workload identity in rules would make
rules non-deterministic across configuration changes.

**Implementation**: nftables rule generation (`d2b-host/src/nftables.rs`)
uses only derived IfNames and configured CIDRs.

### INV-NET-007: ExternalAttachment CIDR isolation

**Invariant**: `externalAttachment.egress.allowedCidrs` entries MUST NOT
overlap with any Zone-local Network CIDR (lan or uplink) or the Host
resource's network inventory. A DNAT port-forward source CIDR must not
coincide with a LAN CIDR.

**Rationale**: an external CIDR overlapping a local CIDR would create an
ambiguous routing table; traffic to the CIDR would split between the external
and local paths unpredictably.

**Test**: `tests/unit/nix/cases/net-vm-network.nix` external network
section; CI assertions eval test.

### INV-NET-008: nixos-system artifact is generic; per-Network config via Volume only

**Invariant**: the nixos-system artifact referenced by `Network.spec.netVmSystemArtifactId`
MUST NOT contain per-Network desired state - DHCP reservations, nftables rules scoped
to Network spec fields, attachment tables, routing policy, or any data that varies
across Networks. The artifact contains only the generic net-VM OS: guest-agent binary
and runtime, kernel, base NixOS services, and systemd-networkd NIC bootstrap.
All per-Network configuration is delivered at runtime through the controller-owned
config Volume (`Volume/net-<networkName>-config`).

**Rationale**: encoding per-Network data in the nixos-system artifact would require
a new artifact build (and Guest switch) for every Network spec change, defeating the
live-update advantage of the config Volume delivery path. It would conflate a shared,
reusable generic system image with per-Network runtime state, making each Network's
nixos-system artifact unique and uncacheable.

**Implementation**: the nixos-system artifact build is a fixed-output derivation keyed
by generic OS inputs only, not by any Network spec data. The controller validates at
runtime that `netVmSystemArtifactId` resolves to a `type=nixos-system` artifact.

**Test**: `packages/d2b-provider-network-local/tests/net_vm_artifact_is_generic.rs`
asserts that two Networks with different CIDRs, attachment tables, and firewall policies
but the same `netVmSystemArtifactId` produce the same `Guest.spec.systemArtifactId`
and different config Volume content; the nixos-system artifact hash is unchanged by
config-only Network spec mutations.

### INV-NET-009: guest-agent capabilities are confined to the Guest network namespace

**Invariant**: the net-agent's `network-admin`/`network-raw` capabilities and
the dnsmasq Process's `network-bind`/`network-raw` capabilities are effective
only within the net-VM Guest's network namespace. Because each
`sandbox.namespaceClasses` is empty, the Process inherits the Guest's existing
network namespace (`CLONE_NEWNET` is NOT set). The Process Provider must not add
a host-adjacent namespace. `CAP_NET_ADMIN`, `CAP_NET_BIND_SERVICE`, and
`CAP_NET_RAW` MUST NOT appear
in the effective capability set of any process that shares the host network namespace.

**Rationale**: the net-VM Guest runs in its own isolated network namespace by
construction. These Processes have no path to the host network stack, so their
separate minimal capability sets pose no host-escalation risk.
A process with these capabilities in the host network namespace could manipulate
host routing, firewall rules, or bind privileged ports on host interfaces.

**Test**: `packages/d2b-provider-network-local/tests/host_capability_leakage.rs` -
negative leakage test: assert that after the guest-agent Process is running, no
process in the host network namespace (namespace identified by `/proc/1/ns/net`)
carries `CAP_NET_ADMIN`, `CAP_NET_BIND_SERVICE`, or `CAP_NET_RAW` in its effective
set as a result of the guest-agent launch. Verified by reading `/proc/<pid>/status`
`CapEff` for all processes sharing the host netns and asserting none of the three
bits are set on any process not already carrying them before the agent started.
Host-integration counterpart: `tests/host-integration/guest-agent-cap-confinement.nix` -
runNixOSTest asserts zero capability leakage to the host network namespace after
guest-agent start.

### INV-NET-010: external physical NIC bridge multiplexing never crosses a Zone L2 boundary

**Invariant**: the Host-global `external-physical-nic/v1` authority binds an
isolation domain equal to the claimant's Zone UID. A `bridge`-mode macvtap
multiplex of one physical NIC MUST admit only claimants that share a single
isolation domain (one Zone). Two `externalAttachment` claims from different
Zones that resolve to the same `(Host, external-physical-nic, opaqueKeyDigest)`
in `bridge` mode MUST be rejected fail closed with
`external-physical-nic-cross-zone-l2`, regardless of `sharingPolicy`; no macvtap
or VMM effect is performed for the rejected claim.

**Rationale**: macvtap endpoints on one physical NIC in `bridge` mode share a
single L2 broadcast domain. Admitting a cross-Zone multiplex would place two
Zones on one L2 segment, defeating Zone network isolation. This repository's
binding invariant is that work and personal realms never share a gateway guest
or an L2 bridge; the isolation-domain check makes that categorical at authority
admission time rather than relying on operator discipline. Same-Zone bridge
multiplexing remains permitted because those endpoints are already inside one
isolation domain.

**Test**: `packages/d2b-provider-network-local/tests/external_nic_cross_zone_l2.rs` -
two Networks in different Zones authoring `sharingPolicy: multiplexed`,
`macvtapMode: bridge` against one fake physical NIC are rejected with
`external-physical-nic-cross-zone-l2` and produce no host effect; a same-Zone
multiplexed pair against the same NIC is admitted; a cross-Zone pair that would
have collided in `passthru`/`private`/`vepa` still reports
`external-physical-nic-conflict`. Nix eval covers a declared cross-Zone bridge
multiplex rejected at build time.

### INV-NET-011: Host-global admission is atomic

**Invariant**: CIDR, derived kernel-name, vsock-CID, and external-NIC
reservations are acquired atomically in the Host-global Core authority index
before effects. Provider peer scans are diagnostic only and cannot authorize a
mutation.

**Test**: concurrent same-CIDR, same-derived-name, and same-vsock claimants
produce exactly one winner and zero effects from every loser, including across
two Zones.

### INV-NET-012: Host-global writers aggregate

**Invariant**: Network controllers publish contributions only. Exactly one
in-process Core owner per Host composes nftables, NetworkManager, and hosts
desired state. The broker preserves every foreign table, marker, and byte
outside the owned region and refuses an ambiguous or foreign marker.

**Test**: two Networks plus one device-usbip contribution reconcile in every
order; removal of each contributor preserves the other two and foreign bytes.

### INV-NET-013: link and route ownership is durable

**Invariant**: bridge and route adoption, replacement, and deletion require
matching live kernel markers, durable broker-resolved ownership records, exact
desired keys, and resource owner proof. Interface name or route destination
alone never proves ownership.

**Test**: daemon restart adopts matching state; missing, changed, duplicated,
and foreign markers block without mutation.

### INV-NET-014: dnsmasq has one Process owner

**Invariant**: each Network has exactly one
`Process/net-<networkName>-dnsmasq`, owned by the Network and supervised through
the Process resource lifecycle. The net-agent never forks, execs, or supervises
dnsmasq, and the generic net-VM artifact declares no competing dnsmasq service.

**Test**: create, config restart, daemon restart/adoption, and delete each
observe exactly one dnsmasq Process identity and no untracked child.

### INV-NET-015: attachment authority is private

**Invariant**: an attachment realization is retained only in Core-private
state. `AttachmentHandle` has no public concrete type, constructor, clone,
serializer, display, accessor, ResourceSpec/status field, audit field, bus
message, or Provider DTO. A Provider generic may only return the handle to the
same injected port by reference or consuming delete.

**Test**: compile-fail and API-surface seals reject construction, naming,
cloning, serialization, conversion, and extraction outside the Core adapter;
runtime tests prove FD handoff only through LaunchTicket.

## Azure/ACA scope boundary

Azure Container Apps (ACA) and Azure Virtual Machine (AVM) network integration
remains inside `Provider/runtime-azure-container-apps` and
`Provider/runtime-azure-virtual-machine` as declared by decision D045.

These Guest Providers manage:

- ACA virtual network injection and subnet delegation;
- AVM private virtual network attachment;
- Azure network security groups and load balancer rules;
- Azure DNS and private DNS zone integration.

The `Network` ResourceType and `Provider/network-local` are for local host
fabrics only. A future ADR may introduce `Provider/network-azure-vnet` or
similar when Azure-hosted Guests in a Zone require shared fabric management
that crosses Guest Provider boundaries.

Until then: processes that run inside an ACA or AVM Guest reference Azure
network policy through Guest-Provider-specific spec fields, not through a
`networkRef` to a local Network resource.

## Nix configuration contract

> **Generic contract**: the zone-wide resource authoring shape, bundle assembly
> pipeline, and generation lifecycle are specified in the forthcoming
> `ADR-046-nix-configuration-contract.md`. This section covers only the
> Network-specific Nix option surface, validation checks, examples, and
> Network finalizer behavior during generation-driven removal.

This section is normative for `Network` resources. It specifies the Nix option
surface, canonical JSON rendering, eval/build validation, Provider schema
verification, Zone resource bundle output, and example configurations.

### Option surface

Network resources are declared as:

```
d2b.zones.<zone>.resources.<name> = { type = "Network"; spec = { <exact ResourceSpec fields> }; };
```

The **attribute key is the resource name**; `metadata.name` is derived from it.
`metadata.zone` is derived from the Zone attribute key. `apiVersion` defaults to
`"resources.d2bus.org/v3"`. The `type` field is **required** (not inferred).

`status` is absent from the Nix form; it is read-only. All core management
metadata - `uid`, `generation`, `revision`, `managedBy`,
`configurationGeneration`, and all timestamps - are filled by core and must
never appear in the Nix authoring form.

An optional `metadata` attrset may appear containing only `ownerRef` for
explicit ownership declaration; all other metadata keys in the Nix form are
rejected at eval time:

```nix
work-net = {
  type     = "Network";
  metadata.ownerRef = "Provider/network-local";  # optional, presentation only
  spec     = { ... };
};
```

The `spec` sub-object fields are **identical** to the canonical ResourceSpec
JSON fields - same names, same nesting, same semantics. There is no separate Nix
vocabulary: no aliases, no re-nesting, no Nix-specific field names. At the
Version 3 baseline, `nixos-modules/resources-network.nix` is a hand-written
strict projection and is code canon. It is not a second schema authority.
Wave 6 must mechanically prove its accepted keys, defaults, bounds, canonical
minimal vector, and fingerprint equal
`core.d2bus.org_Network.schema.json`, then generate or retain the projection
only behind that parity guard. Provider extension options come only from the
signed Provider schema in the artifact catalog.

Resource names must be **unique across all resource types** within a Zone; the
`resources` attrset is keyed by name only, so a `Network` and a `Guest` with the
same name cannot coexist.

```nix
# In any NixOS module imported by the host configuration
{ config, lib, ... }:
{
  # Independent Host/site acknowledgement; false when omitted.
  d2b.site.allowUnsafeEastWest = false;

  d2b.zones.dev.resources = {

    work-net = {
      type = "Network";   # required; determines which spec schema applies
      spec = {
        # spec fields are the exact NetworkSpec JSON fields - no renaming
        providerRef = "Provider/network-local";  # required
        lanCidr     = "10.20.0.0/24";            # required; exactly /24; base ends .0
        uplinkCidr  = "192.0.2.0/30";            # required; exactly /30

        mtu      = null;        # null → 1500 (schema default)
        mssClamp = false;

        isolation.allowEastWest = false;
        # Effective east-west requires this AND allowUnsafeEastWest above.

        routing.hostBlocklist = [
          "10.0.0.0/8" "172.16.0.0/12" "192.168.0.0/16" "169.254.0.0/16"
        ];

        dhcp.domain            = null;
        dhcp.ignoreClientNames = true;

        dns.forwarders = [ "198.51.100.53" "203.0.113.53" ];
        dns.cacheSize  = 1000;

        externalAttachment = null;

        mdns.enable             = false;
        mdns.reflector          = true;
        mdns.dnsmasqLocal       = false;
        mdns.dnsmasqLocalPort   = 53530;
        mdns.publishWorkstation = false;

        netVmNameOverride = null;  # null → "net-work-net"

        # REQUIRED: must reference a d2b.artifacts entry with type = "nixos-system".
        # Absent or wrong-type fails the build.
        netVmSystemArtifactId = "net-vm-base";

        # attachments list mirrors AttachmentSpec JSON exactly
        attachments = [
          { executionRef = "Guest/corp-vm"; index = 10; }
          # mac = null; (schema default → deterministic derivation)
        ];
      };
    };

  };
}
```

#### Required spec fields

These constraints come from `core.d2bus.org_Network.schema.json`. The current
hand-written Nix projection enforces them early and must pass exact schema
parity; it is not an independent vocabulary.

| `spec` field | Constraint |
| --- | --- |
| `providerRef` | Required; must be a registered Provider in the artifact catalog |
| `lanCidr` | Required; IPv4 CIDR; exactly `/24`; base address ends `.0` |
| `uplinkCidr` | Required; IPv4 CIDR; exactly `/30` |
| `netVmSystemArtifactId` | Required; must reference a declared `d2b.artifacts` entry with `type = "nixos-system"`; absent or wrong type is a hard build error |

The `attachments` list may be empty on declaration but must be non-null.

#### Credential ref convention

Any spec field declared `secret: true` in the Provider schema MUST reference a
declared Credential resource rather than carrying an inline value:

```nix
someProviderField = { credentialRef = "Credential/work-vpn-psk"; };
```

The Nix resource compiler rejects an inline string value for a secret field with
a Nix eval error. `Credential/<name>` must appear in
`d2b.zones.<zone>.resources` or an imported Credentials module before the
reference is accepted.

### Canonical ResourceSpec JSON shape

The Nix resource compiler renders each declared Network to canonical JSON. This
is the **Nix-emitted input form**: `apiVersion`, `type`, `metadata.name`,
`metadata.zone`, optional `metadata.ownerRef`, and `spec`. Core adds `uid`,
`generation`, `revision`, timestamps, `managedBy`, and `configurationGeneration`
at activation; they are absent from the Nix-emitted form. The resource store
and controller validate incoming specs against the Network ResourceTypeSchema
derived from this shape.

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "Network",
  "metadata": {
    "name": "work-net",
    "zone": "dev"
  },
  "spec": {
    "providerRef": "Provider/network-local",
    "lanCidr": "10.20.0.0/24",
    "uplinkCidr": "192.0.2.0/30",
    "mtu": null,
    "mssClamp": false,
    "isolation": {
      "allowEastWest": false
    },
    "routing": {
      "hostBlocklist": [
        "10.0.0.0/8",
        "172.16.0.0/12",
        "192.168.0.0/16",
        "169.254.0.0/16"
      ]
    },
    "dhcp": {
      "domain": null,
      "ignoreClientNames": true
    },
    "dns": {
      "forwarders": ["198.51.100.53", "203.0.113.53"],
      "cacheSize": 1000
    },
    "externalAttachment": null,
    "mdns": {
      "enable": false,
      "reflector": true,
      "dnsmasqLocal": false,
      "dnsmasqLocalPort": 53530,
      "publishWorkstation": false
    },
    "netVmNameOverride": null,
    "netVmSystemArtifactId": "net-vm-base",
    "attachments": [
      {
        "executionRef": "Guest/corp-vm",
        "index": 10,
        "mac": null
      }
    ]
  }
}
```

Object keys within each `spec` sub-object are sorted lexicographically. Array
elements preserve declaration order. Null-valued optional `spec` fields are
included explicitly (not omitted) to enable stable content hashing.

### Eval/build validation pipeline

Validation is staged: early structural checks run at `nix eval` time;
Provider-schema validation and bundle assembly run at `nix build` time.

#### Stage 1 - eval-time (nix eval / nix flake check)

Eval-time checks are enforced by the strict Network resource module and its
schema-parity guard. Cross-resource checks use the complete declared Host
projection; runtime Core admission remains authoritative for observed and
concurrent Host-global state.

| Check | Error class |
| --- | --- |
| Attr key (resource name) matches `^[a-z][a-z0-9-]*$` | eval error |
| `type` field is present and names a declared ResourceType | eval error |
| Resource name is unique across all types in the Zone | eval error |
| `providerRef`, `lanCidr`, `uplinkCidr`, `netVmSystemArtifactId` present | eval error (generated required-field check from schema) |
| `lanCidr` is exactly `/24` with a `.0` base address | eval error (generated from schema `format: "cidr-v4-slash24"`) |
| `uplinkCidr` is exactly `/30` | eval error (generated from schema `format: "cidr-v4-slash30"`) |
| `lanCidr` and `uplinkCidr` do not overlap each other | eval error |
| All declared Network `lanCidr` and `uplinkCidr` values on the Host are pairwise non-overlapping and do not overlap declared Host network inventory; uses `lib.d2b.cidrOverlaps` | eval error |
| `isolation.allowEastWest = true` without `d2b.site.allowUnsafeEastWest = true` | eval error with `east-west-site-opt-in-required` remediation |
| Attachment `index` values are in `[2, 250]` | eval error |
| Attachment `index` values are unique within each Network | eval error |
| `attachments[].executionRef` matches `^(Guest\|Host)/[a-z][a-z0-9-]*$` | eval error |
| `netVmNameOverride`, if non-null, matches `^[a-z][a-z0-9-]*$` and is not `"launcher"` and does not start with `"sys-"` | eval error |
| `externalAttachment.portForwards[].targetRef` and `targetIp` are mutually exclusive; both null rejected | eval error |
| `externalAttachment` IPv4 and forwards satisfy the canonical shape, target membership, port uniqueness, and Host-global CIDR rules | eval error |
| Any `{ credentialRef = "..." }` value references a declared `Credential/<name>` resource | eval error |
| No inline value for a field declared `secret: true` in the Provider schema - enforced by the generated option's `type = lib.d2b.secretOrCredentialRef` type | eval error |

#### Stage 2 - build-time (nix build / nixos-rebuild)

1. **Provider schema resolution**: the resource compiler resolves `Provider/network-local`
   through its `Provider.spec.artifactId` entry in the artifact catalog (type=`provider`).
   The `core.d2bus.org_Network.schema.json` is embedded in that `type=provider`
   artifact. Its SHA-256
   digest is verified against the artifact catalog's recorded digest for that artifact.
   A missing artifact or digest mismatch is a hard build error.
2. **`netVmSystemArtifactId` resolution**: REQUIRED; the value must reference a
   declared `d2b.artifacts` entry with `type = "nixos-system"`. Absent field or
   wrong type is a hard build error; there is no implicit default. The build
   records the resolved artifact ID in the bundle for runtime use.
3. **`attachments[].mac` format**: if non-null, must be a unicast MAC in colon
   notation (`^([0-9a-f]{2}:){5}[0-9a-f]{2}$`, LSB of first octet = 0).
4. **Cross-Zone `executionRef`**: any `executionRef` pointing outside the
   current Zone is a build error in v3 initial implementation.
5. **Host-global admission**: Core atomically reserves every CIDR, derived
   kernel name, and net-VM vsock CID across all Zones assigned to the Host.
   External IPv4 and forwarding targets are revalidated against the committed
   attachment set immediately before reservation. A concurrent conflict loses
   admission and performs no effect.

#### Stage 3 - build output: Zone resource bundle

The build derivation produces a Zone resource bundle at
`/nix/store/<hash>-d2b-zone-<zone>-bundle/bundle.json`. The bundle is a
**fixed-output derivation**: identical Nix configuration always produces an
identical store path. Any change to any resource spec produces a different
store path and a different `contentHash`.

```json
{
  "schemaVersion": 3,
  "bundleVersion": 1,
  "zone": "dev",
  "contentHash": "sha256:e3b0c44298fc1c149afbf4c8996fb924...",
  "generatedAt": "1970-01-01T00:00:00.000Z",
  "resources": [
    {
      "apiVersion": "resources.d2bus.org/v3",
      "type": "Network",
      "metadata": {
        "name": "work-net",
        "zone": "dev"
      },
      "spec": { "..." : "..." }
    }
  ],
  "providerSchemaDigests": {
    "Provider/network-local": "sha256:<provider-schema-hash>"
  }
}
```

Bundle properties:

| Property | Value |
| --- | --- |
| **Canonical sort** | Resources sorted lexicographically by `(type, name)`. Sort is stable and deterministic. |
| **Content hash** | SHA-256 of the UTF-8 serialization of the sorted `resources` array (excluding `contentHash` and `generatedAt`). |
| **`generatedAt`** | Set to the Unix epoch in the Nix-built artifact (reproducible builds). The Zone runtime records actual activation time separately in its own generation record. |
| **`managedBy` / `configurationGeneration` in bundle** | Absent from the Nix-emitted bundle. Core sets `managedBy = "configuration"` and assigns `configurationGeneration` on every resource in the bundle when it applies it at activation. |
| **`providerSchemaDigests`** | Every `providerRef` used in the bundle has its Provider schema digest recorded from the `type=provider` artifact resolved via `Provider.spec.artifactId`. The runtime verifies these digests when applying the bundle. |
| **Activation path** | The NixOS activation script writes the bundle store path to `/etc/d2b/zones/<zone>/bundle-store-path`. The core controller reads this path on startup or after activation. Prior bundle copies are retained at `/var/lib/d2b/zones/<zone>/configuration/prior/<contentHash>.json`, never under `/etc`. |

### Examples

#### Minimal Network declaration

```nix
# In any NixOS module - host configuration or a dedicated network module
{
  d2b.zones.dev.resources = {
    work-net = {
      type = "Network";
      spec = {
        providerRef           = "Provider/network-local";
        lanCidr               = "10.20.0.0/24";
        uplinkCidr            = "192.0.2.0/30";
        netVmSystemArtifactId = "net-vm-base";
        attachments = [
          { executionRef = "Guest/corp-vm"; index = 10; }
        ];
      };
    };
  };
}
```

#### Two Networks with east-west opt-in

```nix
{
  d2b.site.allowUnsafeEastWest = true;

  d2b.zones.dev.resources = {
    work-net = {
      type = "Network";
      spec = {
        providerRef             = "Provider/network-local";
        lanCidr                 = "10.20.0.0/24";
        uplinkCidr              = "192.0.2.0/30";
        netVmSystemArtifactId   = "net-vm-base";
        isolation.allowEastWest = true;
        mssClamp                = true;
        attachments = [
          { executionRef = "Guest/corp-vm";  index = 10; }
          { executionRef = "Guest/dev-vm";   index = 11; }
        ];
      };
    };
    personal-net = {
      type = "Network";
      spec = {
        providerRef           = "Provider/network-local";
        lanCidr               = "10.30.0.0/24";
        uplinkCidr            = "198.51.100.0/30";
        netVmSystemArtifactId = "net-vm-base";
        attachments = [
          { executionRef = "Guest/personal-vm"; index = 10; }
        ];
      };
    };
  };
}
```

#### External network attachment with port-forward

```nix
{
  d2b.zones.dev.resources.work-net.spec.externalAttachment = {
    mode            = "macvtap";
    parentInterface = "eno1";
    macvtapMode     = "bridge";
    sharingPolicy   = "exclusive";
    ipv4 = {
      method  = "static";
      address = "203.0.113.10/24";
      gateway = "203.0.113.1";
      dns     = [ "198.51.100.53" ];
    };
    egress = {
      enable       = true;
      allowedCidrs = [ "198.51.100.0/24" ];
    };
    portForwards = [
      {
        protocol   = "tcp";
        listenPort = 2222;
        targetRef  = "Guest/corp-vm";   # attachment index 10, IP 10.20.0.10
        targetPort = 22;
        sourceCidrs = [ "192.0.2.0/24" ];
      }
    ];
  };
}
```

#### mDNS opt-in (creates owned Process resources)

```nix
{
  d2b.zones.dev.resources.work-net.spec.mdns = {
    enable      = true;
    reflector   = true;   # creates Process/net-work-net-mdns-reflector
    dnsmasqLocal = true;  # creates Process/net-work-net-mdns-dns-bridge
  };
}
```

When `enable = true`, the Network controller creates the listed `Process`
resources on the next reconcile cycle. They are owned by `Network/work-net` and
appear in `Network/work-net` status as child Process refs. Removing `enable`
from the Nix config or setting it to `false` does NOT directly delete the
Process resources; instead it updates the Network spec, and the controller's
reconcile path deletes the owned Processes through their finalizers.

## Production acceptance boundary

Network acceptance is a data-plane result, not a status assignment. The
runNixOSTest destination is
`tests/host-integration/network-local-data-plane.nix`, entered through the
evaluated Nix resource declaration, emitted bundle, daemon controller, Core
adapter, broker, net-VM Guest, Process resources, and at least one attached
workload Guest.

The positive case declares `Guest/acceptance-vm` in
`Network/acceptance-net.spec.attachments`, boots both Guests, and proves:

- the workload NIC has the derived IPv4 reservation and effective MTU;
- DHCP and DNS are served by the one owned dnsmasq Process;
- the workload reaches the permitted egress destination through the typed
  route, forwarding, and NAT plan;
- Host route, nftables, NetworkManager, and hosts readback match the aggregate
  desired projection while foreign bytes remain byte-identical; and
- deleting the Network removes only its marked links/routes/projections and
  releases its Host-global reservations.

The same production boundary runs all four east-west combinations from the
Version 3 matrix with two attached workload Guests. Only the true/true case
passes peer traffic. The true/false case must refuse before mutation with the
named remediation; the other false-effective cases remain Ready and isolated.
A fake effect port, empty attachment list, manually assigned Ready status, or
Network-owned net-VM Guest alone cannot satisfy this acceptance.

The current eval safety proof remains the root
`flake.checks.<system>.nix-unit` case
`tests/unit/nix/cases/net-vm-network.nix`. It must continue to assert that
`systemd.network.networks."10-eth-dhcp".matchConfig.Type` is not `ether` and
that `matchConfig.MACAddress` is exactly `00:00:00:00:00:00`, and it must retain
its MTU propagation cases. The retired standalone
`tests/net-vm-network-eval.sh` path is not current validation evidence.

## Generation lifecycle and cleanup contract

### Overview

Each NixOS activation produces or reads a Zone resource bundle with a unique
`contentHash`. Core maintains a monotonically increasing
`configurationGeneration` counter and increments it whenever a bundle with a new
`contentHash` is applied. Core diffs the incoming resource set against the
previous generation's resource set and classifies each resource:

| Classification | Action |
| --- | --- |
| Present in new generation only (added) | Schedule async `Create` |
| Present in both; spec changed | Schedule async `UpdateSpec` |
| Present in both; spec unchanged | No action |
| Present in prior generation only (removed) | Schedule async `Delete` via normal finalizer path |

New generation activation is **non-blocking**. Core marks the generation as
`active` as soon as the bundle validates. It does not wait for removed-resource
cleanup or newly-added resource reconciliation to complete.

### Configuration-managed, controller-owned, and api-created resources

Core classifies every resource by its `metadata.managedBy` field:

| Class | `metadata.managedBy` | Lifecycle authority |
| --- | --- | --- |
| Configuration-managed | `"configuration"` | Declared in a Nix bundle; absent from new bundle → async Delete |
| Controller-owned | `"controller"` | Created by a controller as an owned child; exact controller identity/UID tracked in separate internal metadata; deleted only through parent finalizer chain |
| API-created | `"api"` | Created via resource API without Nix origin; persists until explicit delete; no bundle-driven lifecycle |

**Core NEVER schedules bundle-driven Delete for resources where
`managedBy ≠ "configuration"`.** It only removes configuration-managed
resources absent from the new bundle.
Controller-owned children (the net-VM Guest, config Volume,
guest-agent Process, mDNS Process resources, etc.) are deleted exclusively
through their owner controller's finalizer path when their parent is deleted
or their parent spec no longer requires them.

**Name-collision per-item handling**: when a new bundle contains a resource whose
name already exists in the Zone store with `managedBy = "controller"` or
`managedBy = "api"`, core does NOT apply that item. The conflicting item is recorded
with `phase = Degraded, reason: name-conflict` identifying the existing resource's
`managedBy` value and UID. A `ResourceConflictSkipped` audit record is emitted for
that item. Non-conflicting items in the bundle proceed normally (Provider-state
contract: partial activation is preferable to whole-bundle rejection). Configuration
activation never seizes, overwrites, or replaces an existing controller-created or
API-created resource. The operator must delete the conflicting resource explicitly
via the resource API, after which the next bundle application will apply the item.

These invariants are enforced in core's generation-transition logic
and are tested by `INV-NET-LIFECYCLE-001` and `INV-NET-LIFECYCLE-002` in the
cleanup test suite (ADR046-network-008).

### Cleanup sequence for a removed Network resource

When `Network/work-net` is present in generation N but absent from generation N+1:

1. Core sets `metadata.deletionRequestedAt = <activationTime>` on
   `Network/work-net`. The finalizer `network.d2bus.org/fabric-cleanup` is already
   set from initial create.
2. The `NetworkDraining` condition is set by the network-local controller with
   `reason: configuration-generation-removed`.
3. The controller runs its normal Delete path (see [Delete](#delete)): requests
   deletion of owned children in child-first order (workload Guest/VMM FD close
   → generation-fenced `DeletePersistentTap` confirmation → guest-agent Process
   and mDNS Processes → net-VM Guest → config Volume), waits for each required
   confirmation/Deleted watch event, then calls remaining broker ops and clears
   the finalizer.
4. Core observes the finalizer cleared; the final store transaction atomically
   removes `Network/work-net` from the resource store and index and emits the
   `Deleted` REVISION event. A dedup-guarded `CleanupComplete` audit record follows
   the committed transaction. Controllers waiting for child deletion receive the
   watch event; there is no persistent phase=Deleted row in the store.

This entire sequence is fully asynchronous. Generation N+1 resources proceed
with their own `Create`/`UpdateSpec` independently of step 3.

### Zone and Network status during cleanup

The Zone self resource carries a `PendingCleanup` condition while any
prior-generation configuration-managed resource has not yet reached `Deleted`:

```yaml
# Zone/dev status excerpt
conditions:
  - type: PendingCleanup
    status: "True"
    reason: prior-generation-resources-pending
    message: "1 resource(s) from configurationGeneration 6 pending deletion"
    observedGeneration: 7
    lastTransitionAt: 2026-07-22T00:10:00.000Z
```

The Zone's aggregate `phase` is `Degraded` (not `Failed`) while `PendingCleanup`
is True and no other fatal condition exists.

The Network resource undergoing deletion reports:

```yaml
# Network/old-net status excerpt
phase: Degraded
deletionRequestedAt: "2026-07-22T00:10:00.000Z"
conditions:
  - type: NetworkDraining
    status: "True"
    reason: configuration-generation-removed
    message: "absent from Zone configurationGeneration 7; deletion in progress"
    observedGeneration: 3
    lastTransitionAt: 2026-07-22T00:10:00.000Z
  - type: ReconcileError
    status: "False"
    reason: none
```

The Network does NOT enter a `Terminating` phase. It remains `Degraded` with
`NetworkDraining = True` and `deletionRequestedAt` set throughout the drain
sequence. Final deletion is a single store transaction that atomically removes
the row and index and emits the `Deleted` REVISION event (with `phase = Deleted`
in the event payload); there is no `Terminating` phase between `Degraded` and
removal, and no persisted resource row ever carries `phase = Deleted`. The `Deleted`
phase value is a valid schema value that appears only in the final REVISION event;
controllers wait for the Deleted watch event or resource absence, not for a phase
transition. The dedup-guarded audit append follows the committed transaction.

### Owner controller behavior on parent spec change

When an existing `Network/<name>` spec changes between generations (an
`UpdateSpec` rather than a `Delete`), the network-local controller reconciles
the new spec through its normal reconcile path. Spec-driven child changes:

| Spec change | Controller action |
| --- | --- |
| New `attachments[]` entry | Updates config Volume; creates the private persistent-tap realization; verifies MTU and bridge role before LaunchTicket |
| Removed `attachments[]` entry | Updates config Volume; waits for Guest/VMM FD closure; consumes the private capability through generation-fenced deletion |
| `spec.dhcp.*` / `spec.dns.*` change | Updates `dnsmasq.conf`; performs one owned dnsmasq Process stop/terminal/start transition |
| `spec.routing.hostBlocklist` change | Updates typed `network-plan.json`; net-agent applies filter after address/route/forward readiness |
| `spec.isolation.allowEastWest` change | Re-runs exact double opt-in admission, updates typed filter plan, and reconciles bridge roles |
| `spec.externalAttachment.*` port-forward / egress change | Revalidates external IPv4/targets and updates typed route/filter/NAT plan |
| `spec.externalAttachment` add/remove | Updates config Volume AND updates Guest spec (NIC topology change); Guest switch/restart may occur |
| `spec.mdns.enable` false → true | Creates owned mDNS `Process` resources |
| `spec.mdns.enable` true → false | Deletes owned mDNS `Process` resources through their finalizers |
| Any `spec.lanCidr` / `spec.uplinkCidr` change | Disruptive Host-global re-admission followed by the complete ordered data-plane reconcile |

Core delivers only the parent `UpdateSpec`; it does NOT directly mutate
controller-owned children. Owner controllers reconcile their children as a
consequence of parent spec updates.

### Prior generation retention

Core retains prior generation bundle copies at
`/var/lib/d2b/zones/<zone>/configuration/prior/<contentHash>.json`.

- **Retention count**: configurable via `d2b.zones.<zone>.retainedGenerations`
  (a Nix/compiler-level Zone setting outside `Zone.spec`; default `3`, range `1..16`).
  No TTL. Oldest eligible generation is pruned when the count would be exceeded.
- **Eligibility for pruning**: a generation is eligible when all
  configuration-managed resources from it have either reached `Deleted` or are
  present unchanged in a newer generation with identical spec. A generation that
  is still cleaning up is never pruned regardless of count.
- **Rollback lock**: an operator may lock a specific `contentHash` via the Zone
  API to exclude it from count-based pruning. Rollback locks require an explicit
  `ReleaseRollbackLock` call before that slot is reclaimed. Rollback
  (re-applying a prior bundle) is not automatic; the operator re-declares the
  prior configuration in Nix and triggers a new activation.

Retained bundle files are copied into the mutable `/var/lib` state directory;
they are not Nix store paths and incur modest disk cost proportional to bundle size.

### Error handling during cleanup

If the network-local controller's Delete path encounters a retryable error
(e.g., broker `DeletePersistentTap` or `DeleteBridge` returns a temporary
failure):

- The controller sets `ReconcileError` with a stable coded reason on
  `Network/<name>`.
- Retries proceed with exponential backoff bounded by the
  `ADR-046-resource-reconciliation` retry policy.
- A persistent-tap transient failure retains the private capability and retries with
  the same fence. A generation mismatch first refreshes Network/attachment
  realization and then requeues; it never blindly retries a stale delete.
- The Zone `PendingCleanup` condition persists; the Zone phase remains
  `Degraded`.

If cleanup is permanently blocked (terminal error):

- The controller sets `ReconcileError` with `terminal: true`.
- The Zone `PendingCleanup` condition carries a `blockingResource` detail.
- Operator options: manually remove the blocking child via the resource API;
  or re-declare the Network in Nix (reverting the removal) to stop the cleanup
  and allow re-reconciliation.

A `DeletePersistentTap` foreign-marker/ownership conflict is terminal and
fail-closed. The controller neither deletes the interface nor clears the
finalizer; status and audit expose only the stable
`attachment-ownership-conflict` code.

### Audit records for generation transitions

Core emits one audit record per generation-driven lifecycle event. All records
use stable coded fields; no spec contents, CIDRs, attachment refs, or child
resource names appear in these records.

| Event | Fields emitted |
| --- | --- |
| `BundleActivated` | `zone`, `contentHash`, `configurationGeneration`, `resourceCount`, `providerSchemaDigests` map (digests from `type=provider` artifacts via `Provider.spec.artifactId`) |
| `ResourceDeletionScheduled` | `zone`, `resourceType`, `resourceName`, `removedConfigurationGeneration`, `activeConfigurationGeneration` |
| `ResourceCreationScheduled` | `zone`, `resourceType`, `resourceName`, `activeConfigurationGeneration` |
| `ResourceUpdateScheduled` | `zone`, `resourceType`, `resourceName`, `activeConfigurationGeneration`, `specChanged: true/false` |
| `CleanupComplete` | `zone`, `resourceType`, `resourceName`, `removedConfigurationGeneration`, `outcome: deleted\|superseded` |
| `CleanupBlocked` | `zone`, `resourceType`, `resourceName`, `removedConfigurationGeneration`, `reason` (stable code) |

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `nixos-modules/network.nix` (bridge/NAT/sysctl, 500+ lines), `nixos-modules/net.nix` (net-VM NixOS config, 450 lines), `nixos-modules/options-envs.nix` (`d2b.envs.<env>.*`), `nixos-modules/options-realms-network.nix` (`d2b.realms.<realm>.network.*` mode/cidrs), `nixos-modules/options-vms.nix` (`d2b.vms.<vm>.env` line 944, `d2b.vms.<vm>.index` line 962, `d2b.vms.<vm>.staticIp` deprecated line 974), `nixos-modules/options-site.nix` (`d2b.site.allowUnsafeEastWest` line 48, `d2b.hostLanCidrs` line 382), `nixos-modules/options-realms-workloads.nix` (`d2b.realms.<realm>.workloads.<workload>.networkIndex` line 326), `nixos-modules/host-json.nix` (emits `host.json` `environments[].nftables`, `environments[].ifNameMappings`, `environments[].usbipBusidLocks`), `nixos-modules/processes-json.nix` (emits `processes.json` `ProcessNetworkInterface`/`ProcessMacvtapInterface` per-VM runner), `nixos-modules/lib.nix` (`subnetIp` line 399, `subnetMask` line 408, `mkMac` ~line 60, `cidrOverlaps` lines 429-462), `nixos-modules/index.nix` (netMeta section), `packages/d2b-core/src/host.rs` (`NetEnv` lines 290-328, `VmRuntimeRow` lines 155-167 with `tap`/`bridge`/`net_vm`/`env` fields, `ExternalNetworkPolicy` lines 332-413, `NftablesModel` lines 520-549, `BridgePortFlags`, `TapRole`, `Ipv6SysctlEntry`, `IfNameMapping` lines 242-256), `packages/d2b-core/src/processes.rs` (`ProcessNetworkInterface` lines 98-113, `ProcessMacvtapInterface`), `packages/d2b-contracts/src/broker_wire.rs` (`ApplyNftables`, `ApplyNmUnmanaged`, `ApplyRoute`, `ApplySysctl`, `CreatePersistentTap`, `CreateTapFd`, `SetBridgePortFlags`, `UpdateHostsFile`, `SeedDnsmasqLease`), `packages/d2b-host/src/ifname.rs` (FNV-1a derivation), `packages/d2b-host/src/nftables.rs` (`NftBatch`, `hash_inet_d2b_table`, coexistence policy), `packages/d2b-host/src/bridge_port.rs`, `packages/d2b-host/src/routes.rs`, `packages/d2b-host/src/netlink.rs`, `packages/d2b-host-providers/src/lib.rs` (unwired ADR 0032 runtime/display/substrate provider adapters; no network Provider trait) |
| Evidence class | The v2 Nix network path and its broker operations are `implemented-and-reachable`. `packages/d2b-contracts/src/v3/network.rs`, `nixos-modules/resources-network.nix`, `packages/d2b-provider-network-local/`, and the newer bridge/projection broker primitives exist, but the Provider controller is not composed through a production Core adapter. Those v3 pieces are `generated-or-eval-contract`, `test-only-or-preview`, or `implemented-but-unwired` as their current call sites show. This Version 3 amendment must not be cited as production reachability. |
| Behavior retained | `lib.mkForce` 10-eth-dhcp neutralization; bridge isolation (`Isolated=true` default); ADR 0012's exact Network plus Host/site double opt-in; IPv6 suppression at boot and runtime; `cidrOverlaps` arithmetic; `hostBlocklist` defaults; IfName derivation and collision detection; dnsmasq DHCP reservations with `dhcp-ignore-names`; `bind-interfaces`; hardened confinement; nftables ownership markers and coexistence policy; net-VM IPv6 drop; MTU and MSS behavior; macvtap external attachment; DHCP/static IPv4; egress CIDRs, NAT, and validated port forwarding |
| Required delta | Production Core composition and adapter wiring; atomic Host-global CIDR/name/vsock admission; one canonical base/provider schema; private attachment capability; typed address/route/forwarding/filter/NAT effects and readback; durable bridge/route ownership; Host-global nftables, NetworkManager, and hosts aggregation; one Process-owned dnsmasq lifecycle; MTU propagation; external IPv4/forward validation; and production data-plane acceptance with attached Guests and the four-case double opt-in matrix |
| Reuse path | Extract `subnetIp`/`mkMac`/`cidrOverlaps` from `lib.nix`; copy IfName/derive/detect_collisions from `ifname.rs`; adapt `NetEnv`/`ExternalNetworkPolicy`/`NftablesModel`/`BridgePortFlags`/`TapRole`/`Ipv6SysctlEntry`/`IfNameMapping` from `host.rs`; extract nftables/bridge-port/routes/netlink modules from `d2b-host`; adapt `net.nix` and `network.nix` into sealed v3 template and controller. `VmRuntimeRow.tap`/`bridge`/`net_vm`/`env` fields (host.rs lines 155-167) become Network attachment status fields. `ProcessNetworkInterface`/`ProcessMacvtapInterface` (processes.rs) migrate to Guest spec under Provider/runtime-cloud-hypervisor (not NetworkSpec). |
| Replacement/deletion | `nixos-modules/network.nix`, `nixos-modules/net.nix`, `nixos-modules/options-envs.nix`, `nixos-modules/options-realms-network.nix`, `nixos-modules/index.nix` envMeta section removed only after `nixos-modules/resources-network.nix` and Provider/network-local controller pass parity tests; `d2b.envs.*` options removed only after the v3 cutover and consumer migration |
| Feasibility proof | Existing unit and eval tests prove primitives only. Production reachability and the acceptance matrix remain prospective Wave 6 work. |
| Future owner | `ADR046-network-*` work items below |

## Decisions

All decisions for this spec are resolved. No action is required from the
integrator before spec acceptance.

### D-NETWORK-001: mDNS reflector process identity - RESOLVED

**Resolution**: The mDNS reflector (avahi) and the local dnsmasq DNS bridge,
when enabled, are normal `Process` resources owned by the Network controller,
running inside the net-VM Guest via `executionRef: Guest/<netVmName>`. They
are not inline untracked NixOS services. Foundation requires every ordinary
process to be a `Process` or `EphemeralProcess` resource; this decision
applies that rule without exception.

**Rationale**: an inline systemd service would be invisible to the resource
control plane, could not be independently observed or restarted through the
reconcile loop, and would violate the foundation invariant that every managed
process has a typed resource identity. The Process resource overhead is minimal
and enables independent health monitoring via `NetVmReady` sub-conditions.

**Effect on spec**: `spec.mdns.enable = true` causes the Network controller to
create and own a `Process` resource per enabled mDNS component. All
`spec.mdns.*` fields remain as configuration inputs passed through to those
Process resources. No inline NixOS service for mDNS appears in the net-VM
template.

**Unblocks**: ADR046-network-003 (net-VM template omits mDNS inline services),
ADR046-network-005 (controller creates mDNS Process resources in reconcile step g).

### D-NETWORK-002: USBIP proxy process ownership - RESOLVED

**Resolution**: The USBIP backend and proxy Processes are owned by
`Provider/device-usbip`, not the Network controller. The device controller
watches only Network identity/readiness/generation. Its typed EffectPort
privately resolves the Network UID and dispatches the shared
`ApplyNftablesProjection` broker operation (D123, D124) with closed action enum
`Apply|Remove` for exact per-Network/per-busid TCP/3240 exposure.
`Remove` is generation-bound, ownership-scoped, foreign-marker fail-closed, and
idempotent after validated absence; because the shipped USBIP bind op exposes no
release path, that `Remove` is net-new privileged surface. Device-usbip owns one multiplexed relay `Endpoint` authority
per Network and owns its firewall drift/status. Network-local emits no USBIP
rule on either host or net VM, and its digest excludes device-usbip ownership
markers. `Network.spec` has no `usbipCarveOut` or device-usbip extension field;
the device-usbip controller does not mutate Network desired spec. Firewall
status/token and relay authority are released only after the broker confirms
the `Remove` effect.

**Rationale**: USBIP is a device access mechanism, not a network routing
mechanism. Placing its process lifecycle inside the Network controller would
couple infrastructure and device concerns, preventing independent delivery and
testing of the device-usbip Provider.

**Unblocks**: ADR046-network-007 (USBIP integration boundary work item).

### D-NETWORK-003: Runtime bridge creation - RESOLVED

**Resolution**: `Provider/network-local` dynamically creates and deletes host
kernel bridge devices at reconcile time via two new closed broker operations:
`CreateBridge` and `DeleteBridge`. A NixOS generation switch is NOT required
to create or remove a Network. Nix still provisions bootstrap/static
prerequisites and policy artifacts that do not depend on runtime bridge IfNames
(NetworkManager unmanaged prefix pattern, schema artifacts, controller binary).

**Rationale**: requiring a NixOS generation switch to create or delete a
Network resource would make the Network ResourceType a second-class citizen
in the control plane. Dynamic provisioning is required for the network-local
controller to satisfy the standard async reconcile contract (create on
`Pending → Ready`, delete on `DeletionRequested`). The alternative (static
activation path) would mean a `DeleteSpec` call only removes the resource
object while leaving kernel state alive until the next switch - violating the
finalizer contract.

**New broker ops required**: `CreateBridge` and `DeleteBridge` must be authored
and added to `packages/d2b-contracts/src/broker_wire.rs` and implemented in
`packages/d2b-priv-broker/src/runtime.rs` as `RealBrokerRequest` handlers. Both
ops require security review and broker policy coverage.

**Unblocks**: ADR046-network-004 (Nix emitter no longer generates
`systemd.network.netdevs`), ADR046-network-005 (controller creates/deletes
bridges at reconcile time).


### D-NETWORK-004: Per-ownership-projection host firewall mutation - RESOLVED

**Resolution**: Providers publish typed ownership contributions to one
Host-global in-process Core nftables dispatcher. The dispatcher validates and
sorts the complete contribution set for `inet d2b`, detects duplicate ownership
IDs and chain collisions, and serializes reconciliation. `Provider/network-local`
and `Provider/device-usbip` never dispatch broker firewall operations directly.

The dispatcher realizes each composed contribution through the closed
`ApplyNftablesProjection` broker operation. That operation mutates exactly one
validated ownership projection and byte-preserves every other ownership marker
in the table. This retains D123-D125's projection-scoped broker boundary while
adding the single Host-global composition owner required to prevent
cross-Provider ordering and stale-contribution races. The model does not map
onto the shipped whole-table `ApplyNftables` broker request.

The request carries only an opaque `bundle_nft_projection_ref` (resolved by the
broker to the validated projection - ownership marker plus rule set - from the
integrity-pinned private bundle), a closed `action` enum (`Apply|Remove`), an
`expected_generation_id` fence (the immutable installed configuration generation,
i.e. the bundle `generationId`/`contentHash`), and an optional `tracing_span_id`.
It carries no inline rule text, no IfName, and no caller-supplied ownership
marker. The broker compares `expected_generation_id` against the
currently-installed configuration generation (reloaded per request), then for
`Apply` atomically replaces only the rules bearing that projection's ownership
marker (`comment "d2b managed: <ownership-id>"`) inside `inet d2b`, and for
`Remove` deletes only that marker's rules. It never deletes and recreates the
whole `inet d2b` table. A validated already-absent projection is idempotent
success; a foreign marker where the resolved projection's marker is expected
fails closed with `foreign-nft-rule-preserved`; a request whose
`expected_generation_id` differs from the currently-installed generation mutates
nothing and requeues as `stale-projection-generation` after a fresh read. The op
returns a projection-scoped `FirewallDigest` (SHA-256 over only that marker's
rules) and appends a post-effect, path-free audit record (`op:
ApplyNftablesProjection`, opaque projection digest, expected generationId,
`action`, `outcome`, `error_class`, `correlation_id`; never rule text, IfName,
marker body, or projection bytes). The fence is not a live projection-generation
counter and there is no compare-and-advance (see D125): serialization is provided
by the ordered OFD lock on the `inet d2b` table (total acquisition order per ADR
0034), so concurrent applies to different projections commute and two concurrent
same-generation applies to the same projection converge on identical desired
state, eliminating the whole-table last-writer-wins behavior.

**Rationale**: the shipped `ApplyNftables` op (`packages/d2b-priv-broker/src/ops/nft.rs`)
explicitly discards `ownership_id` and renders a whole-table
`table ...; delete table ...; <full table>` replace. Mapping each dynamic
per-Network-UID `FirewallIntent` (including per-UID deletion) onto that op would
make independent Network reconciles last-writer-wins, let one Network's
projection erase another's, and erase device-usbip ownership markers -
directly violating the ownership-marker preservation
contract (a discovered foreign marker must produce `foreign-nft-rule-preserved`,
never a silent overwrite). Existing broker code is canon and is not respec'd to
invent per-projection semantics it does not implement. The Host-global
dispatcher therefore composes contributions but uses projection-scoped broker
effects. Provider ownership remains separate because every contribution keeps
its own typed owner and digest; only mutation scheduling and table-wide
validation are centralized. No new service or root-visible unit is introduced.

**Cross-provider invariant**: `inet d2b` is a shared table with multiple
ownership projections (one per Network UID, plus device-usbip per-Network/per-busid
markers). Every Provider submits contributions to the same dispatcher, and only
that dispatcher may invoke the projection-scoped, generation-fenced broker op.
Network-local and device-usbip contributions preserve one another's markers by
construction. D124's device-usbip apply/release path remains the same closed
projection operation rather than the shipped whole-table USBIP operation.

**New broker op required**: `ApplyNftablesProjection` and its `NftProjectionAction`
enum must be authored in `packages/d2b-contracts/src/broker_wire.rs` and
implemented as a `RealBrokerRequest` handler in
`packages/d2b-priv-broker/src/runtime.rs` / `ops/nft.rs`. It requires a
`docs/reference/privileges.md` op entry and the audit record shape above, plus
security review and broker policy coverage.

**Decision-register rows**: this projection contract is frozen by three landed
register rows in `ADR-046-decision-register.md`: D123 (network-local per-ownership-projection
host firewall mutation via the new closed `ApplyNftablesProjection` op replacing the
whole-table `ApplyNftables` mapping), D124 (device-usbip `apply_firewall`/`release_firewall`
map onto the same shared `ApplyNftablesProjection` op, and USBIP firewall release is
net-new privileged surface), and D125 (the `expected_generation_id` fence is the immutable
installed configuration generation, with no compare-and-advance).

**Unblocks**: ADR046-network-005 (controller publishes per-Network firewall
contributions at reconcile and finalize time), ADR046-network-007
(device-usbip publishes to the same Host-global dispatcher).

### Work-item authority correction

The `ADR046-network-*` rows below record merged primitive and schema history.
They do not claim the prospective production path. Version 4 of
`ADR-046-provider-network-local` and its `ADR046-nl-001` through
`ADR046-nl-020` rows own the Wave 6 correction and acceptance work. Any lower
row text that assigns Host/site policy to the Network schema, exposes a
concrete attachment handle, gives a Provider its own Host-global writer, or
claims production reachability is superseded by Version 3.


### ADR046-network-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-001` |
| Dependency/owner | W0 shared contract root; `d2b-contracts` |
| Current source | `packages/d2b-core/src/host.rs` lines 290-520 (`NetEnv`, `IfName`, `ExternalNetworkPolicy`, `NftablesModel`, `BridgePortFlags`, `TapRole`, `Ipv6SysctlEntry`, `IfNameMapping` lines 242-256; **also** `VmRuntimeRow` lines 155-167 with `tap`/`bridge`/`net_vm`/`env` fields - attachment status precursors); `packages/d2b-core/src/processes.rs` lines 98-141 (`ProcessNetworkInterface`, `ProcessNetworkInterfaceType`, `ProcessMacvtapInterface` - current VMM runner network interface DTOs; these are per-Guest VMM fields, not Network-level fields, and migrate to Guest spec under `Provider/runtime-cloud-hypervisor`); `packages/d2b-contracts/src/broker_wire.rs` (authoritative broker op list; network-relevant: `ApplyNftables`, `ApplyNmUnmanaged`, `ApplyRoute`, `ApplySysctl`, `SetBridgePortFlags`, `UpdateHostsFile`, `SeedDnsmasqLease`, `CreatePersistentTap`, `CreateTapFd`); `nixos-modules/lib.nix` lines 396-460 (`subnetIp`, `subnetMask`, `mkMac`, `cidrOverlaps`) |
| Reuse source | None from main; all from v3 baseline |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/network.rs` owns only canonical public Network base/status/value types. Version 3 removes the current preview concrete attachment handle from the public contract; `ADR046-nl-003` owns the Core-private associated capability and API seal. |
| Detailed design | Keep the strict canonical Network schema, IfName value/derivation, and CIDR arithmetic. Bind attachment deletion through a Core-private realization plus explicit generation fence, with no public concrete capability, IfName, or path. |
| Integration | Provider dossiers, Nix resource compiler, resource store/API bind these canonical types |
| Data migration | Full d2b 3.0 reset; no env→Network import |
| Validation | Golden JSON/CBOR vectors; CIDR overlap property tests; IfName collision and derivation determinism tests; default hostBlocklist enforcement; attachment index uniqueness; `User/net-local-controller` User resource lifecycle/readiness test: controller creates User Resource with `spec.osUsername = "net-local-controller"` (`ownerRef: Provider/network-local`); controller waits for User resource to reach `Ready` before proceeding; controller aborts with `ConfigVolumeReady=False/user-not-ready` if User resource is not Ready; verifies no numeric UID/GID appears in the Resource spec, authz check, or audit record; verifies that any diagnostic `uid`/`gid` in `User.status` is never used as an authorization input |
| Removal proof | Old `d2b_core::host::NetEnv` and related types removed only after v3 resource API consumers use `d2b_contracts::v3::network` types |
| Implementation state | Merged |
| Evidence | Canonical public Network and IfName types are present. The baseline also exposes a concrete cloneable attachment handle with public construction/accessors; Version 3 records that preview as non-authorizing and assigns its removal/private replacement to prospective `ADR046-nl-003`. |

### ADR046-network-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-002` |
| Dependency/owner | ADR046-network-001; d2b-host network modules owner |
| Current source | `packages/d2b-host/src/ifname.rs` (FNV-1a derivation, detect_collisions, DEFAULT_PREFIX, BRIDGE_TAG, TAP_TAG); `packages/d2b-host/src/bridge_port.rs` (BridgePortReadback, east-west policy, TapRole defaults); `packages/d2b-host/src/nftables.rs` (NftBatch, hash_inet_d2b_table, coexistence policy); `packages/d2b-host/src/routes.rs` (route/dnsmasq-bound/IPv6 preflight); `packages/d2b-host/src/netlink.rs` (IPv6 sysctl sequence) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-network-local/src/ifname.rs`, `bridge_port.rs`, `nftables.rs`, `routes.rs`, `netlink.rs` |
| Detailed design | Move IfName derivation to `d2b-contracts` (ADR046-network-001); keep bridge_port, nftables, routes, netlink in Provider crate. nftables: retain the Network-owned `inet d2b` chain layout, ownership markers, and coexistence matrix; emit no USBIP/TCP-3240 rule and compute drift over only the Network UID ownership projection. routes: adapt dnsmasq-bound check to use Network status instead of `HostJson.environments`. netlink: keep IPv6-off sequence; add defense-in-depth re-application path. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract (ifname, bridge_port, nftables) into shared network-local Provider library; adapt (routes, netlink) into controller observe loop. |
| Integration | Controller observe loop uses nftables digest drift, bridge_port readback, and IPv6 sysctl to drive `FirewallReady`, `FabricReady` conditions |
| Data migration | None (behavior preserved; host bridge names change from `br-<env>-*` to `d2b-b<hash>` after cutover) |
| Validation | Existing `bridge_port::tests::readback_matches_defaults`, `ops::tap::tests::set_bridge_port_flags_readback_drift_fails_closed`, `netlink::tests::ipv6_off_sequence_runs_in_order`, nftables coexistence matrix tests; all pinned in `tests/golden/pinned/host-prepare-network.txt` and `tests/golden/pinned/net-canaries.txt` |
| Removal proof | `packages/d2b-host/src/{ifname,bridge_port,nftables,routes,netlink}.rs` removed only after Provider conformance tests pass |
| Implementation state | Merged |
| Evidence | `packages/d2b-provider-network-local/src/{ifname,bridge_port,nftables,routes,netlink}.rs` and `tests/network_primitives.rs` are present. Tests cover deterministic names and collisions, bridge-port drift, projection apply and remove with sibling and foreign preservation, projection-scoped digests, route checks, and ordered IPv6 suppression. Caveat: the validation field's pin claim is stale: the adapted Provider tests are not named by the two cited pin files. A production `NetworkEffectPort` core adapter is absent and `integration/host_fabric.rs` is an explicitly declaration-only scenario, so the live broker boundary is not exercised here. |

### ADR046-network-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-003` |
| Dependency/owner | ADR046-network-001, ADR046-network-002; Provider/runtime-cloud-hypervisor dossier owner |
| Current source | `nixos-modules/net.nix` (full file, 450 lines); `nixos-modules/net-mdns.nix`; `nixos-modules/lib.nix` subnetIp/mkMac/cidrOverlaps |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-network-local/` - artifact catalog integration for net-VM nixos-system artifact resolution; `packages/d2b-provider-network-local/nix/` - default net-VM NixOS module (parameterized successor to net.nix), built and registered as a nixos-system artifact in `d2b.artifacts` |
| Detailed design | `Network.spec.netVmSystemArtifactId` is REQUIRED. It must reference a declared `d2b.artifacts` entry with `type = "nixos-system"`; verified at Nix build time (Stage 2 check, hard build error if absent or wrong type). No implicit default exists; Provider artifacts cannot silently provide a separately typed system artifact. The controller sets `Guest.spec.systemArtifactId` to the artifact ID value at reconcile time (the value is already validated by the build; the controller fails closed if absent at runtime). The net-VM nixos-system artifact is **generic** (INV-NET-008): it contains the guest-agent binary and runtime, kernel, base NixOS services, systemd-networkd NIC bootstrap, and the `net-local-controller` **OS account** provisioned by `Provider/network-local`'s Nix module (same private fixed UID/GID as on the Host, so that virtiofs view ACLs on config Volume layout entries are enforced consistently inside the Guest; `Provider/system-core` performs NSS lookup reconciliation, not OS account provisioning; no numeric UID/GID appears in any ResourceSpec field, authz check, or audit record; `User.status` MAY carry diagnostic `uid`/`gid` from NSS lookup but those are informational only and never authorization inputs). It does NOT encode per-Network desired data; per-Network config (dnsmasq, nftables, routing, attachments) is delivered via the controller-owned config Volume and applied by the guest-agent Process. The artifact preserves compile-time-fixed content: `lib.mkForce` on 10-eth-dhcp (INV-NET-001); two systemd-networkd interface units matched by MAC; IPv6 suppression sysctls on NIC interfaces; ip6 filter table drop-all policy. **mDNS reflector and local dnsmasq DNS bridge are separate owned Process resources** (D-NETWORK-001); they are not inline services in the artifact. |
| Integration | Network controller resolves artifact ID → sets `Guest.spec.systemArtifactId`. Controller separately creates `Volume/net-<networkName>-config` with per-Network config and `Process/net-<networkName>-agent` (guest-agent). `Provider/runtime-cloud-hypervisor` reads `systemArtifactId` to produce the net-VM bundle and mounts the Volume view into the Guest. |
| Data migration | Destructive v3 reset; existing net VMs are re-created under new IfNames |
| Validation | nix-unit: `tests/unit/nix/cases/net-vm-network.nix` (adapted to v3 resource API); INV-NET-001 assertion in new nix-unit case; no mDNS inline service appears in the generated artifact; no per-Network dnsmasq or nftables data in artifact (INV-NET-008); integration test: mDNS Process resources are created when `spec.mdns.enable = true`; Stage 2 build test: absent `netVmSystemArtifactId` fails with required-field build error; wrong artifact type fails with `artifact-type-mismatch` error; `packages/d2b-provider-network-local/tests/net_vm_artifact_is_generic.rs` - two Networks with different CIDRs produce same `systemArtifactId` and different config Volume content |
| Removal proof | `nixos-modules/net.nix` and `nixos-modules/net-mdns.nix` removed only after net-VM artifact parity tests pass |
| Implementation state | Merged |
| Evidence | `packages/d2b-provider-network-local/nix/{default,net-vm,artifacts}.nix`, artifact resolution in `src/artifact.rs`, `tests/net_vm_artifact_is_generic.rs`, and the adapted `tests/unit/nix/cases/net-vm-network.nix` are present. Tests prove two Networks share the system artifact while config Volume bytes differ, pin the typed Volume and agent shapes, preserve the host blocklist, retain the DHCP neutralizer and IPv6 posture, and exclude inline per-Network dnsmasq, nftables, and mDNS data. Caveat: no executable integration proves separate mDNS Process creation and deletion, and the production Stage 2 resource compiler and runtime artifact path remain unwired. Layer 2 scenarios do not run in the pull-request pipeline. |

### ADR046-network-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-004` |
| Dependency/owner | ADR046-network-001, ADR046-network-002, ADR046-network-003; Nix integrator |
| Current source | `nixos-modules/resources-network.nix` is the strict hand-written Network projection; `nixos-modules/network.nix`, `net.nix`, `host-json.nix`, and `processes-json.nix` remain the reachable v2 data-plane sources. `d2b.site.allowUnsafeEastWest` remains independent Host/site policy and composes with the per-Network request; it does not move into Network schema. |
| Reuse action | adapt |
| Destination | `nixos-modules/resources-network.nix`: Nix resource object emitter for Network ResourceType; `nixos-modules/index.nix`: network resource compilation section |
| Detailed design | Emit the exact NET Version 3 base shape and strict empty `network-local` extension; mechanically prove parity with the canonical schema/fingerprint. Retain both false defaults and require the exact Network/Host double opt-in. Validate declared Host-wide CIDRs, names, external IPv4/forwards, and artifact refs. Emit no runtime bridge, per-Network NetworkManager writer, or per-Network Host-global file writer; Core owns those aggregate runtime projections. |
| Integration | Nix resource objects serialize exactly the Rust NetworkSpec contract (ADR046-network-001). The provider install declares the schema digest. Zone runtime generation-transition logic (ADR046-network-008) reads the bundle at activation. |
| Data migration | Full v3 reset; `d2b.envs.*` declarations must be rewritten as Network resources |
| Validation | nix-unit CIDR overlap, assertion eval, and bridge-sysctl cases; `make test-flake` with updated examples; `make test-drift` for schema/emitter parity; `packages/d2b-contracts/tests/generation_bundle.rs` for bundle format and `contentHash` stability; nix-unit `tests/unit/nix/cases/generation-cleanup-absent-network.nix` for removed-resource scheduling (added by ADR046-network-008) |
| Removal proof | `nixos-modules/network.nix`, `nixos-modules/options-envs.nix`, and `nixos-modules/options-realms-network.nix` removed only after `resources-network.nix` and controller reach parity; `d2b.envs` consumer migration guide updated |
| Implementation state | Merged |
| Evidence | `nixos-modules/resources-network.nix` and the Network compilation in `nixos-modules/index.nix` are present. `tests/unit/nix/cases/net-vm-network.nix`, `generation-cleanup-absent-network.nix`, and `packages/d2b-contracts/tests/generation_bundle.rs` cover schema-shaped resource emission, CIDR and bridge assertions, canonical bundle fields, content hash stability, and absent-resource projection. Caveat: the named `make test-flake` example-evaluation evidence is not part of this update and was omitted from the closing evidence set. The Wave 4 assignment to import `nixos-modules/resources-volume.nix` remains unmet: `index.nix` still does not import it. Production bundle compilation and activation remain later runtime work. |

### ADR046-network-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-005` |
| Dependency/owner | ADR046-network-001 to 004; network-local controller owner; D-NETWORK-001, D-NETWORK-002, and D-NETWORK-003 resolved |
| Current source | Preview `CreateBridge`, `DeleteBridge`, `DeletePersistentTap`, and `ApplyNftablesProjection` broker operations and provider controller primitives exist. The production Core adapter, typed address/aggregate-route effects, durable bridge/route records, Host-global dispatchers, and data-plane acceptance do not. Reachable v2 modules remain canon. |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-network-local/src/controller.rs`: async NetworkReconciler; `packages/d2b-provider-network-local/src/plan.rs`: ReconcilePlan computation; `packages/d2b-provider-network-local/src/observe.rs`: drift-detection observe loop. Full crate layout required (see [Package and crate boundary](#package-and-crate-boundary)): `src/` (controller/plan/observe + colocated unit tests), `tests/` (hermetic conformance and state-machine tests), `integration/` (provider-system reconcile fixtures), `README.md` (Network ResourceType, controller binary, placement, RBAC, security invariants, build/test/integration commands). |
| Detailed design | Version 3 supersedes the historical controller order. Implement the exact ordered path in the authority amendment: Host-global admission; aggregate NM; durable marked links and typed addresses; private attachment and LaunchTicket handoff; typed guest plan; exactly one dnsmasq Process; aggregate routes/nft/hosts; readback-gated Ready. Finalization reverses dependency order and releases Host-global reservations last. |
| Integration | Controller process registers descriptor, watches `Network` resources via d2b-bus/ComponentSession/ResourceClient. Owned Guest and Process mutations trigger owner reconciliation. Device-usbip watches only Network identity/readiness/generation; its Core adapter privately resolves relay/firewall effects (D-NETWORK-002). |
| QEMU launch integration | For every authorized QEMU attachment, the controller supplies only the opaque realization to `NetworkEffectPort`. The Core adapter performs `CreatePersistentTap → SetBridgePortFlags`, then transfers the connected CLOEXEC `OwnedFd` directly to ProviderSupervisor's Process LaunchTicket attachment. The qemu Provider/controller receives only opaque Network/Endpoint refs and no broker op/fd; no fd is serialized through ResourceAPI, ComponentSession, or d2b-bus. Adapter/supervisor parent copies remain CLOEXEC and close after spawn; ticket rejection, cancellation, or spawn failure closes all copies before generation-fenced `DeletePersistentTap`, retaining the opaque realization until confirmation. |
| Data migration | None after full reset |
| Validation | Hermetic controller tests prove ordering and fail-closed transitions but cannot establish production reachability. Wave 6 acceptance must enter through evaluated Nix, the emitted bundle, daemon, production Core adapter, broker, net-VM, exactly one dnsmasq Process, and attached workload Guests; it covers Host-global collision races, private capability seals, typed effect ordering, full MTU propagation, aggregate foreign-state preservation, durable restart/adoption/delete, external IPv4/forwards, and all four east-west combinations. |
| QEMU launch validation | `qemu_tap_launch_order` proves create, flags, and ticket ordering; `qemu_tap_owned_fd_lifetime` proves CLOEXEC parent ownership, one intentional child slot, and closure on success/failure; `qemu_tap_failed_launch_cleanup` proves close-before-delete and private-capability retention until confirmation; `qemu_tap_no_bus_serialization` proves no fd, capability, or broker DTO enters Provider or bus payloads. |
| Removal proof | Daemon-orchestrated network/bridge lifecycle removed only after controller passes conformance and parity tests |
| Implementation state | Merged |
| Evidence | `packages/d2b-provider-network-local/src/{controller,plan,observe}.rs` and `tests/controller_state_machine.rs` are present. Hermetic tests cover CIDR and budget refusal, ordered bridge, sysctl, firewall, route, host and DHCP effects, Volume, Guest, attachment and agent barriers, User readiness, mDNS intent, stale-generation requeue, tap retry, and finalizer ordering. Live handlers for `CreateBridge`, `DeleteBridge`, `DeletePersistentTap`, and `ApplyNftablesProjection` are present in the broker with focused tests. Caveat: the controller uses fake ports because the production core `NetworkEffectPort` adapter is absent. No executable mDNS or container fabric lifecycle exists. The p95 gate is advisory, skips without `D2B_PERF_STABLE=1`, and cannot be met until the project has a pinned stable runner; the project currently has none. |

### ADR046-network-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-006` |
| Dependency/owner | ADR046-network-001, ADR046-network-005; test owner |
| Current source | `tests/unit/nix/cases/net-vm-network.nix`; `tests/golden/pinned/net-vm-bundle-gate.txt`; `tests/golden/pinned/net-canaries.txt`; `tests/golden/pinned/host-prepare-network.txt`; `tests/host-integration/bridge-isolation.nix`; `tests/integration/live/network-isolation.sh` |
| Reuse action | adapt |
| Destination | `tests/unit/nix/cases/net-vm-network.nix` (adapted to v3 resource API); updated golden pins; `tests/host-integration/bridge-isolation.nix` (adapted); `packages/d2b-priv-broker/tests/{bridge_lifecycle,persistent_tap_lifecycle}.rs` (new hermetic broker tests). Provider crate test directories: `packages/d2b-provider-network-local/tests/` - hermetic Cargo integration tests (conformance suite, controller state machine, CIDR validation vectors, IfName determinism, invariant tests INV-NET-001-007, reconcile/observe/finalize with deterministic clock, fault injection); `packages/d2b-provider-network-local/integration/` - container/Host/Guest lifecycle fixtures invoked by `make test-integration` (bridge isolation, east-west double opt-in, nftables drift detection, persistent-tap and macvtap lifecycle). Both directories required by package policy. |
| Detailed design | Rust integration tests: NetworkSpec CIDR validation golden vectors; AttachmentSpec index uniqueness; ExternalAttachmentSpec mutual-exclusion validation; IfName derivation determinism; CIDR overlap arithmetic; INV-NET-001 through INV-NET-010 invariant tests; reconcile/observe/finalize state machine (deterministic clock). Broker tests: `create_bridge_applies_ipv6_sysctl` (INV-NET-002 layer 1); `delete_bridge_is_idempotent`; `delete_bridge_never_cascades_attached_tap`; `create_bridge_parameters_match_spec` (MTU, STP disabled, multicast snooping disabled); `delete_persistent_tap_pairs_with_create`; `delete_persistent_tap_absent_is_idempotent_after_ownership_validation`; `delete_persistent_tap_rejects_stale_network_generation`; `delete_persistent_tap_rejects_stale_attachment_generation`; `delete_persistent_tap_foreign_marker_fails_closed`; `delete_persistent_tap_request_and_audit_have_no_ifname_or_path`; `apply_nftables_projection_apply_mutates_only_owned_marker`; `apply_nftables_projection_apply_preserves_sibling_network_and_usbip_markers`; `apply_nftables_projection_remove_deletes_only_owned_marker_never_whole_table`; `apply_nftables_projection_concurrent_apply_different_projections_commute`; `apply_nftables_projection_same_projection_generation_fence_rejects_stale`; `apply_nftables_projection_validated_absence_is_idempotent`; `apply_nftables_projection_foreign_marker_fails_closed`; `apply_nftables_projection_digest_is_projection_scoped`; `apply_nftables_projection_request_and_audit_have_no_rule_text_ifname_or_path`. Controller tests: `reconcile_applies_sysctl_defense_in_depth` (INV-NET-002 layer 2); `volume_created_before_guest`; `guest_not_created_until_volume_ready`; `agent_process_created_after_guest`; `removed_attachment_waits_for_vmm_then_delete_persistent_tap`; `finalizer_order_vmm_then_taps_then_agent_then_guest_then_volume_then_bridges`; `delete_persistent_tap_transient_retry_retains_handle`; `delete_persistent_tap_generation_mismatch_refreshes`; `delete_persistent_tap_foreign_marker_blocks_finalizer`; `config_only_spec_change_updates_volume_no_guest_restart` (INV-NET-008); `finalizer_calls_delete_bridge`; `mdns_process_created_on_enable`; `mdns_process_deleted_on_disable`; `host_capability_leakage` (INV-NET-009). nix-unit: INV-NET-001 lib.mkForce assertion; net-VM artifact has no inline mDNS service and no per-Network dnsmasq/nftables data (INV-NET-008); Network emitter CIDR constraint assertions; no `systemd.network.netdevs` bridge entries emitted. Host integration: bridge isolation with east-west opt-in; nftables drift detection; persistent-tap and macvtap create/delete lifecycle; config Volume update propagates to guest-agent without Guest restart; `tests/host-integration/guest-agent-cap-confinement.nix` (INV-NET-009 zero leakage to host netns). |
| QEMU TAP tests | Add `qemu_tap_launch_order`, `qemu_tap_owned_fd_lifetime`, `qemu_tap_failed_launch_cleanup`, and `qemu_tap_no_bus_serialization` to the network-local/provider-supervisor integration fixture. Assert the exact `CreatePersistentTap → SetBridgePortFlags → LaunchTicket` chain, direct `OwnedFd` handoff, CLOEXEC discipline, close-before-generation-fenced-delete on every failed launch, no operation-scoped `CreateTapFd`, and no fd/broker DTO at the qemu controller or bus boundary. |
| Integration | Pinned tests registered in `tests/golden/pinned/`; nix-unit cases in `tests/unit/nix/cases/`; host integration in `tests/host-integration/` |
| Data migration | None - full d2b 3.0 reset; no prior state to migrate |
| Validation | All listed tests must pass before `nixos-modules/network.nix` removal is eligible |
| Removal proof | Not applicable (this work item IS the test successor) |
| Implementation state | Merged |
| Evidence | `packages/d2b-provider-network-local/tests/` contains the contract and controller suites, `packages/d2b-priv-broker/tests/{bridge_lifecycle,persistent_tap_lifecycle}.rs` are present, and the adapted Nix and existing bridge-isolation destinations exist. Hermetic coverage exercises the four new live broker operations, projection preservation, stale fences, idempotence, marker refusal, bridge and tap lifecycle, controller faults, and invariant contracts. Caveat: `packages/d2b-provider-network-local/integration/host_fabric.rs` is a constant-list declaration and no `tests/integration/containers/` runner executes bridge, east-west, nftables, persistent-TAP, macvtap, or mDNS lifecycle. Host and container tiers are manual and absent from the pull-request pipeline. |

### ADR046-network-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-007` |
| Dependency/owner | ADR046-network-005; device-usbip Provider dossier; D-NETWORK-002 resolved |
| Current source | `nixos-modules/network.nix` lines 444-461 (USBIP host firewall); `packages/d2b-core/src/host.rs` lines 324-328 (usbip_backend_port, usbip_busid_locks in NetEnv); `packages/d2b-host/src/` usbip_argv.rs |
| Reuse action | adapt |
| Destination | `Provider/device-usbip` owns one relay Process/Endpoint authority per Network and calls the typed UsbipEffectPort for the shared closed `ApplyNftablesProjection` request with closed action enum `Apply|Remove`; Core owns the corresponding effect adapter (D124; the shipped `UsbipBindFirewallRule`/`bind_firewall_rule` op has no release path, so its `Remove` is net-new privileged surface). The controller watches only the `networkRef` resource's identity/readiness/generation; Core privately resolves Network UID to relay attachment and firewall intent. Network spec/status is not mutated with USBIP fields. Full crate layout required for `packages/d2b-provider-device-usbip/` (see [Package and crate boundary](#package-and-crate-boundary)): `src/` (controller and usbip runner + unit tests), `tests/` (hermetic conformance, dependency-watch state machine, `ApplyNftablesProjection` `Apply|Remove` round-trip), `integration/` (Host/Guest USBIP attach/detach lifecycle fixtures), `README.md` (Provider identity, provider-neutral USB Service/Binding types, USBIP Processes/Endpoints, Network least-privilege dependency contract, RBAC, security invariants, standalone-repo path). |
| Detailed design | The device-usbip controller is the sole semantic owner of every USBIP TCP/3240 rule and requests changes only through its injected UsbipEffectPort. The Core adapter resolves the opaque per-Network/per-busid intent and issues the shared `ApplyNftablesProjection` request with action `Apply` for apply and `Remove` for release (D124); the shipped `UsbipBindFirewallRule`/`bind_firewall_rule` op exposes only bind and no release, so this release path is net-new privileged surface delivered by `ApplyNftablesProjection { action: Remove }`. `Remove` is generation-bound, ownership-scoped, idempotent after validated absence, and foreign-marker fail-closed. The controller retains firewall token/status and the relay authority reference until the core adapter reports broker confirmation of `Remove`; its strict provider status owns firewall digest/drift. Network-local emits no generic host or net-VM TCP/3240 allow and ignores device-usbip ownership markers in Network drift. The device Provider owns exactly one multiplexed relay Endpoint authority per Network and supplies Binding proxies only authorized connected streams through LaunchTickets. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | device-usbip watches Network readiness → Core adapter resolves opaque Network attachment → `ApplyNftablesProjection { action: Apply, ... }` for apply or `{ action: Remove, ... }` for release + one relay Endpoint authority → Binding proxy LaunchTicket; release clears status/authority only after confirmed `Remove` |
| Data migration | Current network.nix USBIP carve-out replaced by the shared `ApplyNftablesProjection` broker op (D124) |
| Validation | device-usbip conformance tests cover the exact closed `Apply|Remove` enum (unknown actions rejected), shared `ApplyNftablesProjection` broker mapping for apply/release, net-new `Remove` release surface, expected Network/Service generation binding, exact per-Network/per-busid scoping, idempotent validated-absence `Remove`, one relay Endpoint authority, ownership-scoped drift/status, foreign-marker rejection, transient retry, and retention of status/token/authority until effect confirmation; network-local nftables tests assert no TCP/3240/USBIP rule on host or net VM and prove USBIP rule churn does not change Network `FirewallReady`; the pinned USBIP firewall golden moves to device-usbip ownership |
| Removal proof | Network.nix USBIP sections removed only after the `ApplyNftablesProjection` mechanism passes conformance |
| Implementation state | Merged |
| Evidence | `packages/d2b-provider-device-usbip/src/{controller,firewall}.rs` and its conformance, controller-state-machine, and redaction tests are present. They cover the closed Apply and Remove projection actions, generation fences, per-Network and per-device scoping, one relay lease, observation, validated-absence removal, foreign-marker and transient failures, and retention until confirmation; Network projection tests preserve USBIP marker bytes. Caveat: no implementation of `UsbipEffectPort` exists in `d2b-core-controller` or another Core runtime. All controller tests use fake ports, so no production adapter maps the intent to `ApplyNftablesProjection`, and no Host/Guest attach-detach integration is executable. |

### ADR046-network-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-008` |
| Dependency/owner | ADR046-network-004, ADR046-network-005; Zone runtime integrator |
| Current source | No current v3 source: generation lifecycle and `managedBy`/`configurationGeneration` classification do not exist on the pre-ADR45 v3 baseline. The v3 baseline uses NixOS activation scripts that atomically replace all host JSON artifacts; there is no per-resource generation tracking, no async cleanup queue, and no `managedBy` field. |
| Reuse action | create |
| Destination | `packages/d2b-core-controller/src/configuration.rs`: bundle application, diff, generation-transition logic (including per-item name-conflict handling), prior-bundle retention under `/var/lib/d2b/zones/<zone>/configuration/prior/`; `packages/d2b-core-controller/src/cleanup.rs`: removal scheduling and `PendingCleanup` condition tracking; `packages/d2b-contracts/src/generation_bundle.rs`: `ZoneBundle`/`BundleResource`/`BundleMetadata` **input** DTOs - MUST NOT include `managedBy` or `configurationGeneration` (both are persisted resource metadata set by core at activation, not bundle input fields); `ManagedBy` closed enum `{ Configuration, Controller, Api }` and `configurationGeneration: u64` live in `packages/d2b-core-controller/src/resource_store.rs` as persisted resource metadata fields; `nixos-modules/resources-network.nix` (emits bundle with `managedBy`/`configurationGeneration` absent; core sets both at activation per ADR046-network-004); `d2b.zones.<zone>.retainedGenerations` Nix/compiler-level Zone option (outside `Zone.spec`; default `3`, range `1..16`); `tests/unit/nix/cases/generation-cleanup-absent-network.nix`; `packages/d2b-contracts/tests/generation_bundle.rs`; `tests/host-integration/nix-generation-cleanup.nix` |
| Detailed design | **Core generation tracking** (`packages/d2b-core-controller/src/configuration.rs`): on each bundle application, core compares the incoming `contentHash` against the prior applied hash. If different, the sole durable writer ADR046-routing-013 assigns the next monotone `configurationGeneration` ordinal, durably stages the outgoing bundle, and atomically commits that ordinal with the new active pointer, prior pointer, and retention metadata in `generation.json`. Network-008 defers this commit to ADR046-routing-013; only after it returns does network-008 set `managedBy = "configuration"` and `configurationGeneration` to the committed ordinal on each resource in the bundle and perform its Network diff and removal scheduling. The `managedBy` and `configurationGeneration` fields are absent from the Nix-emitted bundle and are set exclusively by core after activation. **`managedBy` field and per-item name-conflict handling**: `ManagedBy` is a closed enum (`Configuration`, `Controller`, `Api`) persisted in resource metadata at `packages/d2b-core-controller/src/resource_store.rs`. It is NOT a field in `ZoneBundle`/`BundleResource` input DTOs; core sets it at activation. Controllers set `ManagedBy::Controller` when creating owned children (net-VM Guest, config Volume, guest-agent Process, mDNS Processes); exact controller identity/UID/generation are tracked in separate internal metadata, not embedded in the `managedBy` value. API-created resources carry `ManagedBy::Api` and persist until explicit delete with no bundle-driven lifecycle. Core's generation-transition logic only schedules bundle-driven Delete for `ManagedBy::Configuration` resources. **Per-item name-conflict handling**: when a bundle item's `(zone, name)` already exists with `managedBy ≠ "configuration"`, core skips that item and records it with `phase = Degraded, reason: name-conflict`; a `ResourceConflictSkipped` audit record is emitted for that item. All non-conflicting items in the bundle proceed normally (Provider-state contract). The existing resource is left completely untouched. The operator deletes the conflicting resource via the resource API; the next bundle application applies the item. **Removal scheduling**: on generation N+1 activation, core performs a set difference: `prev_configuration_managed - new_configuration_managed` = resources to delete. For each, it sets `metadata.deletionRequestedAt` in the resource store and emits a `ResourceDeletionScheduled` audit record. Normal finalizer-path Delete proceeds asynchronously. **`PendingCleanup` condition**: the Zone self resource carries a `PendingCleanup = True` condition while any `managedBy = Configuration` resource has `deletionRequestedAt` set and has not yet been atomically removed. Aggregate Zone `phase = Degraded` applies. The condition transitions to `False` and Zone phase returns to `Ready` when all scheduled deletions complete. **Prior generation bundle retention** (`cleanup.rs`): count-based (`d2b.zones.<zone>.retainedGenerations`, outside `Zone.spec`, default 3, range 1..16); no TTL. Core copies prior bundles to `/var/lib/d2b/zones/<zone>/configuration/prior/<contentHash>.json`. A generation is eligible for pruning when all configuration-managed resources from it have either been atomically removed or are present unchanged in a newer generation, AND the count would be exceeded. **`BundleActivated` audit record**: emitted at each generation transition with `contentHash`, `configurationGeneration`, `resourceCount`, and `providerSchemaDigests` map (digests from `type=provider` artifacts via `Provider.spec.artifactId`); no spec contents, CIDRs, or resource names appear in the record. Provider schema digests in the bundle are re-verified against installed Provider artifact digests at application time; a mismatch aborts application with a `BundleRejected` audit record. |
| Integration | ADR046-network-004 (emitter writes bundle format; core sets `managedBy`/`configurationGeneration` at activation) → ADR046-network-008 (runtime reads and applies). ADR046-network-005 (controller Delete path) is invoked by ADR046-network-008 removal scheduling for Network resources. Zone `PendingCleanup` condition and `Degraded` phase are read by CLI `d2b zone status`. |
| Data migration | None on v3 initial install (no prior generation state). Host upgrades from the pre-ADR45 v3 baseline perform a reset: ADR046-routing-013 assigns `configurationGeneration = 1` in the first durable activation commit and records no prior bundle. All declared resources are treated as new Creates. |
| Validation | **nix-unit**: `tests/unit/nix/cases/generation-cleanup-absent-network.nix` - verifies that a Network resource present in generation N and absent from generation N+1 receives `deletionRequestedAt` and appears in the `PendingCleanup` condition; verifies that a controller-owned `Guest` (`managedBy = "controller"`) does NOT receive a direct bundle-driven Delete; verifies that a re-declared (identical spec) Network is NOT scheduled for Delete; verifies `retainedGenerations` default is 3. **Rust contract tests**: Two separate test files - (1) `packages/d2b-contracts/tests/generation_bundle.rs`: tests the **input** bundle DTO only: `ZoneBundle`/`BundleResource`/`BundleMetadata` JSON round-trip, `contentHash` stability across serialization, `providerSchemaDigests` presence, `managedBy` and `configurationGeneration` fields ABSENT from `BundleResource` input struct (verified by both compile-time type check: the fields must not exist on the `BundleResource` type, and runtime JSON serialization: the serialized object must not contain those keys). (2) `packages/d2b-core-controller/tests/resource_metadata.rs`: `ManagedBy` closed enum round-trip with `"configuration"`/`"controller"`/`"api"` values tested separately here since `ManagedBy` is persisted resource metadata in `resource_store.rs`, not a field of the input bundle DTO. **Controller integration tests**: async Delete triggered through finalizers for Network; mDNS Process child deleted before Network finalizer clears; bridge `DeleteBridge` broker call made exactly once during finalizer; controller waits for Deleted watch event (not a persistent phase=Deleted row) before proceeding. **Host integration**: `tests/host-integration/nix-generation-cleanup.nix` - runNixOSTest scenario: apply generation 1 with Network resource, then apply generation 2 with that Network absent; assert Zone enters `Degraded/PendingCleanup`; assert Network `phase = Degraded` with `NetworkDraining = True` and `deletionRequestedAt` set and `reason: configuration-generation-removed`; assert cleanup completes (single store transaction: Deleted REVISION event + row/index removal; dedup-guarded audit append follows committed transaction) and Zone returns to `Ready`; assert no controller-owned children deleted directly by core; assert prior bundle copied to `/var/lib/d2b/zones/<zone>/configuration/prior/` and retained until cleanup complete; assert bundle pruned when `retainedGenerations` exceeded and generation eligible. **INV-NET-LIFECYCLE-001**: core never schedules bundle-driven Delete for `managedBy ≠ "configuration"` resources; verified by static analysis of core's generation-transition diff function, which is bounded at compile time to iterate only the `configuration_managed_resources` set. **INV-NET-LIFECYCLE-002**: per-item name-conflict - when a bundle item collides with `managedBy = "controller"` or `"api"`, that item is recorded as `Degraded/name-conflict`; the existing resource is left untouched; non-conflicting items continue to activate; tested by `packages/d2b-core-controller/tests/configuration_name_conflict.rs` (three cases: collision with a controller-owned child, an API-created resource, and a same-name configuration resource from a prior generation that completed deletion; each case asserts non-conflicting items still activate). |
| Removal proof | Not applicable (this is a new capability). The `PendingCleanup` condition and zone cleanup audit path have no prior equivalent to remove. |
| Implementation state | Merged |
| Evidence | The ruled destinations are present at `packages/d2b-contracts/src/generation_bundle.rs`, `packages/d2b-core-controller/src/configuration/{bundle_apply,generation_transition}.rs`, `cleanup.rs`, and `resource_store.rs`, plus the Nix emitter and generation eval case. Contract and core tests cover input-field closure, integrity and schema digests, post-commit generation binding, management ownership, per-item conflict, absent-resource scheduling, PendingCleanup projection, and count-based retention. Caveat: the production store/watch adapter and `tests/host-integration/nix-generation-cleanup.nix` are absent. No executable scenario proves mDNS child deletion, one live `DeleteBridge`, terminal Deleted-watch consumption, row/index removal, Zone status, audit append, or prior-bundle filesystem retention through a real activation. Durable `generation.json` commit remains delegated to later `ADR046-routing-013`. |

### ADR046-network-009

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-009` |
| Dependency/owner | D097 Host-global authority index; ADR046-network-001, ADR046-network-005; Provider/network-local and Core authority owners |
| Current source | Existing macvtap spawn path resolves `parentInterface` but has no cross-Zone authority admission or compatible-sharing contract |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/network.rs` external-attachment sharing schema/status; `packages/d2b-core-controller/src/authority.rs` Core-derived physical-NIC identity and Host-global claim; Provider/network-local descriptor/reconcile/finalizer |
| Detailed design | Resolve operator-declared `parentInterface` against trusted Host inventory and derive an opaque `external-physical-nic/v1` digest; index `(Host, external-physical-nic, opaqueKeyDigest)` before any macvtap/VMM effect. The authority binds an isolation domain equal to the claimant's Zone UID. `passthru`, `private`, and `vepa` are exclusive. `bridge` defaults exclusive and is multiplexed only under explicitly authored compatible policy shared by claimants in one isolation domain (one Zone); a cross-Zone `bridge` multiplex of the same physical NIC is categorically rejected fail closed with `external-physical-nic-cross-zone-l2` because it would place two Zones on one L2 broadcast domain (INV-NET-010; work and personal Zones never share an L2 bridge). Use typed `external-physical-nic-conflict` for same-Zone exclusive/mixed-policy/quota conflicts and `external-physical-nic-cross-zone-l2` for the cross-Zone L2 rejection; expose only bounded authority availability/holder-count/queue/arbitration/update-currency and conditions; keep digest, interface identity, and owner proof private. Parent/mode/policy update drains and releases the old claim before replacement; deletion closes macvtap/VMM ownership before releasing the claim; restart adopts exact owner proof and quarantines ambiguity. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt the existing private macvtap-FD spawn path; add authority admission before it. |
| Integration | Network validation and Core authority preflight gate runtime-cloud-hypervisor LaunchTicket/`SpawnRunner`; the finalizer and D091 update planner release in dependency order |
| Data migration | Full d2b 3.0 reset; no authority ledger import |
| Validation | Hermetic authority tests cover same-Zone and cross-Zone exclusive collisions, mixed-policy conflicts, non-bridge multiplex rejection, explicit compatible bridge multiplex admission within one Zone, categorical cross-Zone `bridge` multiplex rejection with `external-physical-nic-cross-zone-l2` and no host effect (INV-NET-010), Core-derived key equality for two selectors resolving to one fake NIC, caller-supplied digest rejection, no-effect conflict, owner-proof adoption/ambiguity, disruptive update, and release-after-close ordering. Nix eval covers schema, declared cross-Zone conflicts, and a declared cross-Zone bridge multiplex rejected at build time; host integration covers create/update/delete with a fake macvtap parent and status/condition transitions without raw identity exposure. |
| Removal proof | None - authority admission is new; existing direct macvtap spawn becomes unreachable without a claim. |
| Implementation state | Merged |
| Evidence | External sharing/status contracts are present in `packages/d2b-contracts/src/v3/network.rs`, and `packages/d2b-core-controller/src/authority.rs` implements trusted-inventory resolution, opaque Host-global keys, same-Zone compatible multiplexing, distinct cross-Zone refusal, exact-owner adoption, ambiguity quarantine, status projection, and close-before-release replacement. Inline and Provider tests cover those decisions and redaction. Caveat: no host-integration scenario drives a fake macvtap parent through create, disruptive update, delete, and status transitions, and no production VMM launch path consumes the authority lease before effect. The host tier is manual and absent from the pull-request pipeline. |
