# ADR 0046 resources: Network

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-resources-network` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 2 |
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
- per-attachment isolation policy (east-west, hostBlocklist, egress CIDRs);
- optional external network attachment (macvtap) with port-forward and egress.

`Provider/network-local` is the only initial Network Provider. Azure Container
Apps and Azure Virtual Machine networking remain inside their respective Guest
Providers (`Provider/runtime-azure-container-apps` and
`Provider/runtime-azure-virtual-machine`) until those networking surfaces
require independent sharing across Guests. See [Azure/ACA scope boundary](#azureaca-scope-boundary).

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
  createdAt: 2026-07-22T00:00:00Z
  updatedAt: 2026-07-22T00:00:00Z
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
    # allowEastWest = true is the explicit per-Network opt-in for east-west.
    # No Zone-level gate is required.

  # --- Egress policy ---
  routing:
    hostBlocklist:
      - "10.0.0.0/8"
      - "172.16.0.0/12"
      - "192.168.0.0/16"
      - "169.254.0.0/16"
    # The controller merges host network inventory from the Host resource and
    # all peer-Network lanCidrs/uplinkCidrs into this list before emitting
    # firewall rules. Caller-supplied entries are additive; duplicates deduplicated.

  # --- DHCP/DNS ---
  dhcp:
    domain: null          # optional dnsmasq domain name for the LAN
    ignoreClientNames: true  # prevents workloads spoofing hostnames
    # dhcp-authoritative is always on. DHCP pool: lanCidr.251–254 (unreserved).
    # Static reservations derive from spec.attachments.

  dns:
    forwarders:
      - "1.1.1.1"
      - "8.8.8.8"
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
status: {}
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
| DHCP dynamic pool | `lanCidr.251`–`lanCidr.254` |

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
                            # overlap peer Network cidrs or the Host resource's
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

`passthru`, `private`, and `vepa` always require exclusive arbitration.
`bridge` is also exclusive by default. It becomes multiplexed only when every
claimant explicitly authors `sharingPolicy: multiplexed` and the signed
Provider quota admits another holder. An absent policy defaults exclusive; a
mixed exclusive/multiplexed set, a non-bridge multiplexed claim, or quota excess
fails closed with
`external-physical-nic-conflict` before macvtap creation or VMM spawn. Because
the index is Host-global, this rule applies across Zones on the same Host.

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
      lastTransitionAt: 2026-07-22T00:00:01Z
    - type: NetVmReady
      status: "True"
      reason: guest-ready
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:02Z
    - type: DhcpReady
      status: "True"
      reason: dnsmasq-bound
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:02Z
    - type: FirewallReady
      status: "True"
      reason: nft-applied
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:01Z
    - type: CidrConflict
      status: "False"
      reason: no-conflict
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:00Z
    - type: ExternalNicAuthorityReady
      status: "True"
      reason: external-physical-nic-claimed
      observedGeneration: 1
      lastTransitionAt: 2026-07-22T00:00:01Z
  lastReconciledAt: 2026-07-22T00:00:02Z
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
| `CidrConflict` | No CIDR overlap detected with peer Networks or the Host resource's network inventory |
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

Interface names are derived deterministically from `(networkName, role, optional guestName)` using FNV-1a 64-bit, base32-encoded (Crockford alphabet, no I/L/O/U), truncated to 8 characters, prefixed by:

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
Network. Operators must adjust the `networkName` to resolve collisions.

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
  # to Provider/runtime-cloud-hypervisor through an authorized LaunchTicket.
  # No IfName, address, MAC, parentInterface, or authority key is copied here.
  systemArtifactId: <value-from-Network.spec.netVmSystemArtifactId>
  # Artifact ID referencing the net-VM nixos-system artifact (type=nixos-system)
  # in the artifact catalog. Set from the REQUIRED Network.spec.netVmSystemArtifactId;
  # absent or wrong-type artifact fails closed at Nix build time.
```

The controller reads `Network.spec.netVmSystemArtifactId` (a REQUIRED field validated
at build time) and stores it verbatim in `Guest.spec.systemArtifactId`. The nixos-system
artifact contains only the **generic net-VM OS** — the guest-agent binary and runtime,
kernel, base NixOS services, and systemd-networkd NIC bootstrap with the
`lib.mkForce` override — but NOT per-Network desired data (DHCP reservations,
nftables rules, attachment table, routing policy). Per-Network configuration is
delivered at runtime through a controller-created config Volume; the guest-agent
reads the Volume view and applies dnsmasq/nftables/routing policy inside the net VM.
See [Config Volume and guest-agent delivery](#config-volume-and-guest-agent-delivery).

Mutations to `Network.spec` that change only DHCP/DNS, firewall, or attachment
configuration update the config Volume and trigger a guest-agent reload; a Guest
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
    - path: "nftables.rules"         # inet filter/nat chains rendered from spec
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
    - path: "routing.conf"           # external attachment static routes
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
    - path: "attachments.json"       # attachment table: MAC→IP, index→IfName
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
  # the Guest reaches Ready (two-phase sequence — see Delivery lifecycle).
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
Resource to Ready — it does not provision the OS account. Numeric UID/GID never
enter any ResourceSpec field, authz check, or audit record; `User.status` MAY carry
diagnostic `uid`/`gid` values discovered by NSS lookup, but those are informational
only and are never authorization inputs. The network-local
controller waits for `User/net-local-controller` to reach `Ready` before creating
any config Volume; this is a reconcile precondition, not a bootstrap side effect.
The tmpfs `quota.maxBytes = 4 MiB` is charged to the Host's memory budget at Volume
creation time.

The Volume is provisioned in two phases:

**Phase 1 — backing ready**: the controller creates the Volume with `source`,
`layout`, and `views` but an empty `attachments` list. The backing tmpfs on the
Host becomes Ready without any Guest attachment. The controller writes the initial
config content through the Volume write service before any Guest exists.

**Phase 2 — Guest attachment**: after the Guest reaches Ready, the controller
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
  processClass: worker              # narrow worker; no reconcile loop or d2b-bus authority
  template: net-vm-agent
  sandbox:
    namespaceClasses: []            # empty: inherit all parent (Guest) namespaces;
                                    # the Process runs inside the net-VM Guest and
                                    # therefore inherits the Guest's network namespace,
                                    # not the host's; no new CLONE_NEWNET is created
    capabilityClasses: [network-admin, network-bind, network-raw]
    # network-admin  → CAP_NET_ADMIN: required for nft ruleset load and ip route
    # network-bind   → CAP_NET_BIND_SERVICE: required for dnsmasq binding to port 53 (DNS)
    #                  and port 67 (DHCP server)
    # network-raw    → CAP_NET_RAW: required for dnsmasq's DHCP raw socket operations
    # All three are granted within the inherited Guest network namespace only;
    # none confers any capability on the host network namespace.
  mounts:
    - volumeRef: Volume/net-<networkName>-config
      view: guest-readonly
      mountPath: "/run/d2b/net-config"
      access: read-only
      required: true
  budget: { }
```

The guest-agent binary is included in the nixos-system artifact (generic). On
startup it reads the config Volume view mounted at `/run/d2b/net-config`, enters its
reconciliation loop, and applies the current config: loading the nftables ruleset
(`nft -f /run/d2b/net-config/nftables.rules`), starting and configuring dnsmasq
from `/run/d2b/net-config/dnsmasq.conf`, and applying routing policy from
`/run/d2b/net-config/routing.conf`. `capabilityClasses: [network-admin, network-bind, network-raw]`
grants `CAP_NET_ADMIN` (for `nft` and `ip route`), `CAP_NET_BIND_SERVICE` (for
dnsmasq binding to port 53/DNS and port 67/DHCP), and `CAP_NET_RAW` (for dnsmasq's
DHCP raw socket). Because `namespaceClasses` is empty the Process inherits the
Guest VM's network namespace; all three capabilities are effective only within that
isolated Guest namespace and have no effect on the host network stack (INV-NET-009). On each Volume update notification (via the Volume service's
watch interface), the agent re-reads changed entries and applies the diff atomically
(SIGHUP dnsmasq for config changes; atomic `nft replace` for firewall changes).
The agent reports readiness predicates (`dnsmasq-bound`, `firewall-applied`) that
the Network controller uses to set `DhcpReady` and `FirewallReady` conditions.

#### Delivery lifecycle

| Event | Controller action |
| --- | --- |
| Network created | Create Volume (backing + layout + views; empty attachments); backing Ready → write config content → create Guest → Guest Ready → update Volume with Guest attachment → attachment Ready → create guest-agent Process |
| Network spec changes (DHCP/DNS/firewall/attachment config only) | Update Volume content; agent receives watch notification and applies diff; no Guest switch required |
| Network spec changes (NIC topology: attachment index add/remove, external attachment) | Update Volume content AND update Guest spec; Guest reconcile handles NIC changes |
| Guest-agent Process fails | Controller observes Process status; re-creates Process on terminal failure; sets `NetVmReady=False/agent-restart` |
| Network deleted | Delete guest-agent Process → remove Guest attachment from Volume → delete Guest → wait Deleted → delete Volume → wait Deleted → broker ops (nftables/bridges) → clear finalizer |

### Bridge and tap lifecycle (host-side)

**Bridge creation and deletion are dynamic broker operations.** `Provider/network-local`
creates and deletes host kernel bridge devices at reconcile time via new closed
broker operations (`CreateBridge` / `DeleteBridge`). A NixOS generation switch
is NOT required to create or remove a Network; the controller provisions all
fabric state at runtime.

Nix still provisions bootstrap/static prerequisites and policy artifacts that do
not require knowing runtime bridge IfNames:

- `networking.networkmanager.unmanaged` pattern block for the `d2b-*` prefix
  (emitted via `00-d2b-unmanaged.conf`; covers all dynamically-created d2b
  bridges and taps regardless of specific IfNames);
- schema validation and controller binary deployment;
- static host security policy artifacts.

The broker's `CreateBridge` operation:

- creates the kernel bridge device with the derived IfName;
- sets MTU from `spec.mtu` (or 1500);
- disables STP and multicast snooping unconditionally;
- applies IPv6 suppression sysctls
  (`net.ipv6.conf.<ifname>.disable_ipv6 = 1`,
   `net.ipv6.conf.<ifname>.accept_ra = 0`,
   `net.ipv6.conf.<ifname>.autoconf = 0`)
  atomically before returning, closing any race between interface
  creation and the controller's subsequent `ApplySysctl` defense-in-depth step.

The broker's `DeleteBridge` operation removes only the kernel bridge device,
after every persistent tap has been confirmed removed through
`DeletePersistentTap`. It never cascades deletion to an attached tap: a
remaining d2b-owned tap is retryable after tap cleanup, and a foreign
port/marker fails closed. It is idempotent when the bridge is already absent.

The network-local controller performs the following runtime effects through the
broker's typed effect interface:

| Broker op | Current source | Purpose |
| --- | --- | --- |
| `CreateBridge` | **New broker op** (v3; no v3-baseline equivalent) | Create host kernel bridge with derived IfName, MTU, STP/multicast-snooping disabled, IPv6 suppression sysctls applied atomically |
| `DeleteBridge` | **New broker op** (v3; no v3-baseline equivalent) | Remove an empty host kernel bridge device; never cascade tap deletion; idempotent on absence |
| `ApplyNftables` | `d2b_contracts::broker_wire::ApplyNftablesRequest`; `d2b-host/src/nftables.rs` | Install/replace the host-side `inet d2b` table |
| `ApplyNmUnmanaged` | `d2b_contracts::broker_wire::ApplyNmUnmanagedRequest` | Write `00-d2b-unmanaged.conf` bridge/tap pattern block |
| `ApplyRoute` | `d2b_contracts::broker_wire::ApplyRouteRequest`; `d2b-host/src/routes.rs` | Static host route to LAN CIDR via uplink bridge |
| `ApplySysctl` | `d2b_contracts::broker_wire::ApplySysctlRequest`; `d2b-host/src/netlink.rs` | Per-bridge IPv6 suppression defense-in-depth (re-applied after networkd restart or sysctl drift) |
| `CreatePersistentTap` | Existing closed broker op | Create/adopt the persistent tap for an opaque attachment realization |
| `DeletePersistentTap` | **New closed broker op** paired with `CreatePersistentTap` | Delete exactly one opaque, generation-fenced, d2b-owned persistent tap; validated absence is success |
| `SetBridgePortFlags` | `d2b_contracts::broker_wire::SetBridgePortFlagsRequest`; `d2b-host/src/bridge_port.rs` | Isolated/neigh-suppress per-tap after tap creation |
| `UpdateHostsFile` | `d2b_contracts::broker_wire::UpdateHostsFileRequest` | VM→IP entries in the `d2b-managed` /etc/hosts block |
| `SeedDnsmasqLease` | `d2b_contracts::broker_wire::SeedDnsmasqLeaseRequest` | Pre-seed DHCP reservations for known attachment MACs |

IPv6 is suppressed per-bridge at `CreateBridge` time (atomically by the broker)
AND via `ApplySysctl` at each reconcile cycle (defense-in-depth, handling
`systemctl restart systemd-networkd` and any other sysctl drift). No
boot-time Nix sysctl entry is required for specific bridge IfNames because
bridges are created dynamically and do not exist at host activation.

**Persistent-tap creation** is declared by the network-local controller through
its semantic EffectPort and maps to `CreatePersistentTap`; Core supplies the
resulting FD privately to `Provider/runtime-cloud-hypervisor` through the
LaunchTicket path. `CreateTapFd` remains the runtime's non-persistent FD path.
On attachment removal or Network finalization, network-local waits for the
Guest/VMM FD owner to close and invokes the paired `DeletePersistentTap`
through its EffectPort.

`DeletePersistentTapRequest` contains only an opaque attachment ID,
`expectedNetworkGeneration`, and `expectedAttachmentGeneration`. It accepts no
IfName, path, or caller-authored marker. The broker resolves trusted private
realization state, validates both generations and the d2b ownership marker,
then deletes only that tap. Already-absent is idempotent success only when the
trusted record and marker state show no foreign replacement; a stale
generation or foreign marker fails closed without deletion. The controller
retains the opaque attachment handle and retries retryable failures before
advancing or clearing a finalizer.

### DHCP and DNS lifecycle (inside net VM)

dnsmasq runs inside the net VM supervised by the guest-agent Process
(`Process/net-<networkName>-agent`). The Network controller writes `dnsmasq.conf`
to the config Volume; the guest-agent reads it from the mounted read-only Volume
view and manages dnsmasq as a supervised child process. The nixos-system artifact
does not encode per-Network dnsmasq configuration; all DHCP/DNS desired state flows
through the config Volume.

Key dnsmasq invariants preserved from `nixos-modules/net.nix` lines 302–441,
now encoded in the controller-rendered `dnsmasq.conf` Volume entry:

- `bind-interfaces = true` binds only to `eth1` (LAN interface);
- `dhcp-ignore-names = true` prevents hostname spoofing;
- static DHCP host reservations are derived from `spec.attachments[]`;
- DHCP dynamic pool covers `lanCidr.251`–`lanCidr.254`;
- DNS forwarders are set from `spec.dns.forwarders`;
- the guest-agent runs dnsmasq under the `dnsmasq` system user with hardened
  confinement (preserving the hardening from net.nix lines 363–441).

The `DhcpReady` condition is set by the network-local controller observing the
guest-agent Process status. The guest-agent reports the `dnsmasq-bound` readiness
predicate when dnsmasq has bound its socket. The controller does NOT manage dnsmasq
directly; the guest-agent owns its lifecycle inside the net VM.

### Firewall and NAT lifecycle

**Host side** (`inet d2b` table, `d2b-host/src/nftables.rs`):

The network-local controller applies nftables rules through the broker
`ApplyNftables` operation. The `inet d2b` table:

- blocks all traffic on LAN bridges (host has no IP there);
- installs per-rule `comment "d2b managed: <ownership-id>"` markers
  (ownership ID is the Network resource UID);
- coexists with other firewall managers per the `FirewallCoexistencePolicy`
  (Coexist/Refuse/RequireUnmanaged matrix preserved from
  `packages/d2b-host/src/nftables.rs`).

Network-local emits no TCP/3240 or other USBIP allow rule. Its
`status.provider.details.firewallDigest` is the SHA-256 of the canonical
projection containing only rules owned by that Network UID. USBIP-owned chains,
rules, and ownership markers are excluded, so USBIP attach/detach cannot create
false Network drift. Network-local compares only this ownership-scoped digest
on each observe cycle.

**Net VM side** (nftables rules delivered through the config Volume):

The Network controller writes the net VM's nftables ruleset to the `nftables.rules`
entry in the config Volume; the guest-agent reads this entry from the mounted
read-only Volume view and applies it atomically using `nft -f`. The nixos-system
artifact does not encode per-Network nftables rules; all firewall desired state flows
through the config Volume. The ruleset preserves all semantics from
`nixos-modules/net.nix` lines 168–296:

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
3. The net VM configures `external0` using systemd-networkd (DHCP or static,
   preserving the logic from `nixos-modules/net.nix` lines 106–146).
4. Port-forward DNAT rules are written to the `nftables.rules` config Volume
   entry by the Network controller and applied by the guest-agent inside the net VM.
5. Egress CIDRs are included in the forward chain and postrouting chain.

Before step 1, Core resolves `parentInterface` against trusted Host inventory
and admits the Host-global `external-physical-nic/v1` authority claim. No
macvtap or VMM effect is permitted until `ExternalNicAuthorityReady=True`.
`passthru`, `private`, and `vepa` claims are exclusive. `bridge` is exclusive
unless every concurrent claimant explicitly requests compatible multiplexing.
A same- or cross-Zone conflict sets
`ExternalNicAuthorityReady=False/external-physical-nic-conflict` and performs no
host effect.

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
opaque per-Network/per-busid intent and dispatches the closed
`UsbipBindFirewallRule` broker operation with the closed action enum
`Ensure|Remove`. That path owns all USBIP TCP/3240
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

The attachment is owned by the Network, not the Guest. The network-local
controller creates the tap interface and bridge-port configuration when a
listed Guest exists and is in a Ready-or-better phase. The Guest's own spec
references the network for firewall/routing/sandbox purposes:

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

When checking for host LAN CIDR collisions, the controller reads the Host
resource's observed network inventory (a runtime fact from the Host resource's
status or runtime-observable fields, as defined by the Host ResourceType).
At Nix build time the eval may validate against declared host configuration
input where available.

## CIDR allocation and validation

### Constraints (all enforced by the network-local controller)

| Field | Constraint |
| --- | --- |
| `lanCidr` | Must be exactly `/24`; base address must end in `.0` |
| `uplinkCidr` | Must be exactly `/30` |
| `lanCidr` ↔ `uplinkCidr` | Must not overlap within the same Network |
| Any Network `lanCidr` ↔ any other Network's `lanCidr` or `uplinkCidr` | Must not overlap within the Zone |
| Any Network CIDR ↔ Host resource network inventory | Must not overlap |
| `externalAttachment.egress.allowedCidrs` ↔ any Network CIDR in Zone | Must not overlap |
| Attachment `index` | 2–250 inclusive; unique within the Network |

CIDR overlap uses the same two-prefix IPv4 arithmetic as `cidrOverlaps` in
`nixos-modules/lib.nix` lines 429–462: two CIDRs overlap if and only if their
shorter prefix matches when applied to both network addresses. Containment
counts as overlap.

Validation runs at:

1. `validateSpec` call in the reconcile async interface — before any host-side
   effect. A failing validateSpec returns a `ValidationResult` with
   stable code `network-cidr-conflict` and sets the `CidrConflict` condition.
2. Each reconcile cycle — the controller re-checks against current peer Network
   specs because concurrent creates can pass individual validation and still
   conflict.

The same CIDR rules apply to `externalAttachment.egress.allowedCidrs` entries.
A port-forward `sourceCidrs` entry must not coincide with a peer Network CIDR
(prevents accidental cross-env routing).

### Env name / interface name constraints

The controller enforces at validateSpec time:

- `networkName` regex: `^[a-z][a-z0-9-]*$` (standard ResourceName);
- Effective LAN bridge name ≤ 15 bytes after IfName derivation (guaranteed by
  construction; verified via `detect_collisions`);
- `netVmNameOverride`, if set, must match `^[a-z][a-z0-9-]*$` and must not be
  `launcher` or start with `sys-`.

## Isolation, hierarchy, and budgets

### East-west isolation

`isolation.allowEastWest` is the explicit per-Network opt-in for intra-network
east-west traffic. No Zone-level policy gate is required.

When `isolation.allowEastWest = false` (default), workload taps on the LAN
bridge are set to `Isolated = true` and the net-VM forward chain has no
`eth1→eth1 new accept` rule, preventing direct L2 communication between
workloads in the same Network.

When `isolation.allowEastWest = true`, the controller:
1. Sets workload tap isolation bits to `Isolated = false` via broker
   `SetBridgePortFlags`.
2. Includes the east-west accept rule in the net VM's forward chain.

Bridge isolation is enforced at two layers:
- **Host kernel**: tap entries in the LAN bridge carry `Isolated = true` by
  default (all workload taps), preventing direct L2 frames between workloads.
  Only the net-VM tap is non-isolated (it can reach all workload taps).
- **Net-VM nftables**: the forward chain has no `eth1→eth1 new accept` rule
  unless `allowEastWest = true`.

Changing `allowEastWest` from true to false requires a full reconcile to
re-apply bridge isolation flags and regenerate the net-VM nixos-system artifact.

### hostBlocklist invariant

The effective `hostBlocklist` is:

```text
spec.routing.hostBlocklist
  ∪ { Host resource's observed network inventory (runtime fact; schema-neutral) }
  ∪ { lanCidr, uplinkCidr of every other active Network in the Zone }
```

The controller computes this union at each reconcile cycle by querying the
Host resource's observed network inventory (a runtime-observable fact from the
Host resource, as defined by the Host ResourceType; at Nix build time the eval
may validate against declared host configuration input where available). Each
entry generates a `drop` rule in the net VM's forward chain (before the broad
`lan→internet accept`). This prevents workloads from routing to:
- host LAN ranges (from the Host resource's observed network inventory);
- other d2b networks in the Zone;
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

A Zone may have several Networks, each with distinct, non-overlapping CIDRs
and independent net-VM Guests. There is no parent Network resource; Networks
are peers. The Zone self resource is the shared policy anchor for Zone-wide
policy; Network-specific isolation is per-Network only.

## Async reconcile/observe/adopt/delete

The network-local controller implements the full reconciliation contract from
`ADR-046-resource-reconciliation`.

### Reconcile

1. Call `validateSpec`: verify CIDRs, attachment indices, interface name
   constraints, CIDR overlaps against peer Networks, and external-NIC mode/
   sharing constraints. Core resolves `parentInterface` from trusted Host
   inventory and preflights the Host-global authority index. Return a conflict
   condition if validation or authority admission fails; do not proceed to host
   effects.
2. Call `plan`: compute desired bridge, tap, nftables, sysctl, and Guest states.
   Diff against current status; produce a `ReconcilePlan`.
3. Call `reconcile`:
   a. Create LAN and uplink bridge devices via broker `CreateBridge` if not
      present. `CreateBridge` specifies the derived IfName, MTU, STP disabled,
      multicast-snooping disabled, and applies IPv6 suppression sysctls
      atomically. Returns success immediately if the bridge already exists
      with matching parameters. A `CreateBridge` failure sets
      `FabricReady=False` with reason `bridge-create-error` and aborts.
   b. Ensure IPv6 suppression sysctls are applied to all bridges via broker
      `ApplySysctl` (defense-in-depth after possible networkd restart or drift).
   c. Apply host nftables `inet d2b` table via broker `ApplyNftables`.
   d. Ensure NetworkManager unmanaged config covers all bridge/tap patterns.
   e. Create the owned `Volume/net-<networkName>-config` resource with
      `providerRef: Provider/volume-local`, `kind: ephemeral`, `source.executionRef:
      Host/<hostName>`, `source.settings.kind: tmpfs`, `quota.maxBytes: 4194304`,
      `quota.maxInodes: 128`, `quota.enforcement: hard`, the `layout` entries (root
      directory + four files each with `type: file`, `ownerRef: User/net-local-controller`,
      `mode: "0640"`, and conservative policies), and `views.guest-readonly.rights:
      [read, traverse]`. Set `attachments: []` (no Guest attachment yet). The tmpfs
      quota is charged to the Host memory budget at Volume creation time.
      `User/net-local-controller` must be a Ready User resource in the same Zone.
      It is owned by `Provider/network-local` controller (`spec.osUsername:
      net-local-controller`, `ownerRef: Provider/network-local`, `managedBy:
      controller`); `Provider/system-core` verifies the account via NSS lookup and
      reconciles it to Ready — it does not provision the OS account. The
      `net-local-controller` OS account is provisioned by `Provider/network-local`'s
      Nix package/module in Host prerequisites and the net-VM artifact. No numeric
      UID/GID never enter ResourceSpec fields, authz checks, or audit records;
      `User.status` MAY carry diagnostic `uid`/`gid` discovered by NSS lookup, but
      those are informational only and are never authorization inputs.
      See work item ADR046-network-001. If Volume creation returns a terminal error, set
      `ConfigVolumeReady=False/config-volume-error` and abort.
   f. Write the bounded canonical Network config content to the Volume through the
      Volume write service (dnsmasq.conf, nftables.rules, routing.conf,
      attachments.json; no workload names or raw host paths). Wait for the Volume
      backing to reach `Ready`. If `Degraded` or `Failed`, set
      `ConfigVolumeReady=False/volume-backing-error` and requeue.
   g. Set `Guest.spec.systemArtifactId` from the REQUIRED
      `Network.spec.netVmSystemArtifactId` (already validated at build time;
      controller fails closed if absent at runtime). Create or update the owned
      `Guest/<netVmName>` resource with network interface parameters. The Volume
      attachment is NOT added to the Guest spec yet.
   h. Wait for the Guest to reach `Ready`. Then update the Volume to add the Guest
      attachment:
      `attachments: [{executionRef: Guest/<netVmName>, transport: virtiofs,
      view: guest-readonly, access: read-only, mountPath: "/run/d2b/net-config",
      settings: {posixAcl: false, xattr: false, cache: auto,
      inodeFileHandles: never, threadPoolSize: null, socketGroup: null}}]`.
      Wait for the Volume attachment to reach `Ready`. If `Degraded`, set
      `ConfigVolumeReady=False/attachment-not-ready` and requeue.
   i. Create or update the owned guest-agent `Process/net-<networkName>-agent` with
      `executionRef: Guest/<netVmName>`, `processClass: worker`, `sandbox.namespaceClasses: []`
      (inherits Guest network namespace), `sandbox.capabilityClasses: [network-admin, network-bind, network-raw]`
      (`CAP_NET_ADMIN`/`CAP_NET_BIND_SERVICE`/`CAP_NET_RAW` effective in Guest network namespace only; no host capability; INV-NET-009),
      and mount `[{volumeRef: Volume/net-<networkName>-config, view: guest-readonly,
      mountPath: "/run/d2b/net-config", access: read-only, required: true}]`.
      Create or update tap IfName records for each attachment entry that has a Ready
      Guest. If `spec.mdns.enable = true`, create or update the owned mDNS reflector
      `Process` resource (and local DNS bridge `Process` if `dnsmasqLocal = true`)
      with `executionRef: Guest/<netVmName>`.
   j. Set bridge port flags (Isolated, neigh-suppress) for each tap via broker.
   k. For each attachment removed from the current spec, wait for its Guest/VMM
      FD ownership to close, then invoke `DeletePersistentTap` through the
      EffectPort with the retained opaque attachment ID and current expected
      Network/attachment generations. Retain the handle across retryable
      failures; refresh/requeue on generation mismatch; fail closed on an
      ownership-marker conflict.
   l. Commit a `ResourceMutationBatch` with the Volume, Guest, and Process updates
      and status.
4. Report conditions and phase.
5. On any child (Volume, Guest, or Process) mutation, receive
   `owned-resource-changed` hint and re-evaluate `ConfigVolumeReady`,
   `NetVmReady`, `DhcpReady`, and `FirewallReady` from the Volume/Guest/Process
   status.

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
- Re-check CIDR overlaps against current peer Network specs.
- Re-check the external-NIC authority owner proof and compatible holder policy.
  If missing, ambiguous, or conflicting, set
  `ExternalNicAuthorityReady=False` and do not recreate the macvtap.

Observation results are committed as status-only updates without incrementing
generation.

### Adopt

On controller restart (continuation event), the controller:

1. Lists all Network resources in the Zone.
2. For each Network, reads current host bridge state through broker ops
   (bridge present/not, isolation flags, nftables digest).
3. For an external attachment, asks Core to adopt the exact Host-global
   physical-NIC authority by resource/process owner proof. Ambiguity
   quarantines the attachment.
4. If bridges are present and the Network-owned nftables digest matches, marks the
   attachment as adopted without re-applying. The net-VM Guest lifecycle
   is separately adopted by `Provider/runtime-cloud-hypervisor`.
4. If bridges are absent (not yet created, or lost after host restart),
   the normal reconcile loop creates them via `CreateBridge` on the next
   reconcile cycle. No special adoption path is needed.

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
5. Requests deletion of the owned guest-agent `Process/net-<networkName>-agent`
   and any owned mDNS `Process` resources. Waits for their Deleted watch events
   (each Deleted step is a single store transaction: the REVISION event with
   `phase = Deleted` and row/index removal happen atomically; there is no persistent
   phase=Deleted row for the controller to observe).
6. Updates the Volume to remove the Guest attachment entry (sets `attachments: []`);
   waits for the attachment removal to be confirmed. This unbinds the read-only view
   from the net-VM before the Guest is stopped.
7. Deletes the owned net-VM `Guest/<netVmName>` resource; waits for the Deleted
   watch event. The net VM's macvtap FD (external attachment, if any) is released
   as part of the VMM teardown inside `Provider/runtime-cloud-hypervisor` — the
   broker destroys the macvtap interface when the SpawnRunner child exits.
8. Deletes the owned `Volume/net-<networkName>-config` resource; waits for the
   Deleted watch event. At this point the Guest attachment has already been removed
   (step 6) and the Volume backing is released cleanly.
9. Removes `inet d2b` rules scoped to this Network's ownership-id via broker
   `ApplyNftables` (empty rule set for this UID).
10. Clears NetworkManager unmanaged config for the removed patterns via broker
   `ApplyNmUnmanaged`.
11. Clears /etc/hosts entries for this Network's VMs via broker `UpdateHostsFile`.
12. Deletes the LAN bridge and uplink bridge via broker `DeleteBridge` for each.
   `DeleteBridge` is idempotent and succeeds if the bridge is already absent.
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

Broker operations (`ApplyNftables`, `ApplyNmUnmanaged`, `ApplyRoute`,
`ApplySysctl`, `CreatePersistentTap`, `DeletePersistentTap`,
`SetBridgePortFlags`, `UpdateHostsFile`, `SeedDnsmasqLease`, etc.) emit their
own audit records with path-free outcome codes. `DeletePersistentTap` audit is
post-effect and contains only the exact op name, an opaque attachment digest,
the expected Network/attachment generations, outcome, error class, and
correlation ID. It contains no attachment-handle bytes, IfName, path, or
ownership-marker body.

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
`Network.spec.isolation.allowEastWest = true` may workload taps be set to
`Isolated = false`.

**Rationale**: L2 isolation prevents direct workload-to-workload frames even if
the net VM's forwarding rules allow it. The per-Network opt-in is the sole
requirement; no Zone-level policy gate is needed.

**Test**: `packages/d2b-host/src/bridge_port.rs` bridge-port conformance tests;
`tests/host-integration/bridge-isolation.nix` host integration test.

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
MUST NOT contain per-Network desired state — DHCP reservations, nftables rules scoped
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

**Invariant**: the capabilities granted by `sandbox.capabilityClasses:
[network-admin, network-bind, network-raw]` to the guest-agent Process are effective
only within the net-VM Guest's network namespace. Because `sandbox.namespaceClasses`
is empty the Process inherits the Guest's existing network namespace (`CLONE_NEWNET`
is NOT set); the Process Provider (system-minijail) must NOT add `CLONE_NEWNET` or
any other namespace class that would create a new, potentially host-adjacent network
context. `CAP_NET_ADMIN`, `CAP_NET_BIND_SERVICE`, and `CAP_NET_RAW` MUST NOT appear
in the effective capability set of any process that shares the host network namespace.

**Rationale**: the net-VM Guest runs in its own isolated network namespace by
construction. A guest-agent Process that inherits this namespace has no path to
the host network stack, so the three capabilities pose no host-escalation risk.
A process with these capabilities in the host network namespace could manipulate
host routing, firewall rules, or bind privileged ports on host interfaces.

**Test**: `packages/d2b-provider-network-local/tests/host_capability_leakage.rs`
— negative leakage test: assert that after the guest-agent Process is running, no
process in the host network namespace (namespace identified by `/proc/1/ns/net`)
carries `CAP_NET_ADMIN`, `CAP_NET_BIND_SERVICE`, or `CAP_NET_RAW` in its effective
set as a result of the guest-agent launch. Verified by reading `/proc/<pid>/status`
`CapEff` for all processes sharing the host netns and asserting none of the three
bits are set on any process not already carrying them before the agent started.
Host-integration counterpart: `tests/host-integration/guest-agent-cap-confinement.nix`
— runNixOSTest asserts zero capability leakage to the host network namespace after
guest-agent start.

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
metadata — `uid`, `generation`, `revision`, `managedBy`,
`configurationGeneration`, and all timestamps — are filled by core and must
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
JSON fields — same names, same nesting, same semantics. There is no separate Nix
vocabulary: no aliases, no re-nesting, no Nix-specific field names. The Nix
option types, defaults, and constraints for `spec.*` fields are generated from
the same `Network.schema.json` (ResourceTypeSchema) used for JSON validation;
they are not hand-written. Provider-specific extension fields in `spec` are
generated from `Provider/network-local`'s signed schema in the artifact catalog.

Resource names must be **unique across all resource types** within a Zone; the
`resources` attrset is keyed by name only, so a `Network` and a `Guest` with the
same name cannot coexist.

```nix
# In any NixOS module imported by the host configuration
{ config, lib, ... }:
{
  d2b.zones.dev.resources = {

    work-net = {
      type = "Network";   # required; determines which spec schema applies
      spec = {
        # spec fields are the exact NetworkSpec JSON fields — no renaming
        providerRef = "Provider/network-local";  # required
        lanCidr     = "10.20.0.0/24";            # required; exactly /24; base ends .0
        uplinkCidr  = "192.0.2.0/30";            # required; exactly /30

        mtu      = null;        # null → 1500 (schema default)
        mssClamp = false;

        isolation.allowEastWest = false;
        # allowEastWest = true is the sole opt-in; no Zone-level gate.

        routing.hostBlocklist = [
          "10.0.0.0/8" "172.16.0.0/12" "192.168.0.0/16" "169.254.0.0/16"
        ];

        dhcp.domain            = null;
        dhcp.ignoreClientNames = true;

        dns.forwarders = [ "1.1.1.1" "8.8.8.8" ];
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
        netVmSystemArtifactId = "nixos-system/net-vm-base-abc123";

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

These constraints are enforced by the generated Nix option types, not
hand-written custom checks. The source of truth is `Network.schema.json`.

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
      "forwarders": ["1.1.1.1", "8.8.8.8"],
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
    "netVmSystemArtifactId": "nixos-system/net-vm-base-abc123",
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

#### Stage 1 — eval-time (nix eval / nix flake check)

Eval-time checks are enforced by the **generated Nix option types** for
`d2b.zones.<zone>.resources.<name>`. The types and their constraints are derived
from `Network.schema.json` and the Provider schema; they are not hand-written in
the module system. Cross-resource checks (CIDR overlap, unique names) are
enforced by the `d2b.zones.<zone>` module aggregation logic.

| Check | Error class |
| --- | --- |
| Attr key (resource name) matches `^[a-z][a-z0-9-]*$` | eval error |
| `type` field is present and names a declared ResourceType | eval error |
| Resource name is unique across all types in the Zone | eval error |
| `providerRef`, `lanCidr`, `uplinkCidr`, `netVmSystemArtifactId` present | eval error (generated required-field check from schema) |
| `lanCidr` is exactly `/24` with a `.0` base address | eval error (generated from schema `format: "cidr-v4-slash24"`) |
| `uplinkCidr` is exactly `/30` | eval error (generated from schema `format: "cidr-v4-slash30"`) |
| `lanCidr` and `uplinkCidr` do not overlap each other | eval error |
| All Network `lanCidr` and `uplinkCidr` values in the Zone are pairwise non-overlapping and do not overlap the declared host network inventory (when available as Nix configuration input); uses `lib.d2b.cidrOverlaps` (exact algorithm of `nixos-modules/lib.nix` lines 429–462) | eval error |
| Attachment `index` values are in `[2, 250]` | eval error |
| Attachment `index` values are unique within each Network | eval error |
| `attachments[].executionRef` matches `^(Guest\|Host)/[a-z][a-z0-9-]*$` | eval error |
| `netVmNameOverride`, if non-null, matches `^[a-z][a-z0-9-]*$` and is not `"launcher"` and does not start with `"sys-"` | eval error |
| `externalAttachment.portForwards[].targetRef` and `targetIp` are mutually exclusive; both null rejected | eval error |
| `externalAttachment.egress.allowedCidrs` do not overlap any Zone Network CIDR | eval error |
| Any `{ credentialRef = "..." }` value references a declared `Credential/<name>` resource | eval error |
| No inline value for a field declared `secret: true` in the Provider schema — enforced by the generated option's `type = lib.d2b.secretOrCredentialRef` type | eval error |

#### Stage 2 — build-time (nix build / nixos-rebuild)

1. **Provider schema resolution**: the resource compiler resolves `Provider/network-local`
   through its `Provider.spec.artifactId` entry in the artifact catalog (type=`provider`).
   The `Network.schema.json` is embedded in that `type=provider` artifact. Its SHA-256
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
5. **`portForwards[].sourceCidrs` overlap**: each entry must not coincide with
   any Zone Network LAN CIDR; checked against the full resolved Zone resource set.

#### Stage 3 — build output: Zone resource bundle

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
  "generatedAt": "1970-01-01T00:00:00Z",
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

#### Minimal single-env network declaration

```nix
# In any NixOS module — host configuration or a dedicated network module
{
  d2b.zones.dev.resources = {
    work-net = {
      type = "Network";
      spec = {
        providerRef           = "Provider/network-local";
        lanCidr               = "10.20.0.0/24";
        uplinkCidr            = "192.0.2.0/30";
        netVmSystemArtifactId = "nixos-system/net-vm-base-abc123";
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
  d2b.zones.dev.resources = {
    work-net = {
      type = "Network";
      spec = {
        providerRef             = "Provider/network-local";
        lanCidr                 = "10.20.0.0/24";
        uplinkCidr              = "192.0.2.0/30";
        netVmSystemArtifactId   = "nixos-system/net-vm-base-abc123";
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
        netVmSystemArtifactId = "nixos-system/net-vm-base-abc123";
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
    ipv4.method     = "dhcp";
    egress = {
      enable       = true;
      allowedCidrs = [ "192.168.1.0/24" ];
    };
    portForwards = [
      {
        protocol   = "tcp";
        listenPort = 2222;
        targetRef  = "Guest/corp-vm";   # → attachment index 10 → IP 10.20.0.10
        targetPort = 22;
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
    lastTransitionAt: 2026-07-22T00:10:00Z
```

The Zone's aggregate `phase` is `Degraded` (not `Failed`) while `PendingCleanup`
is True and no other fatal condition exists.

The Network resource undergoing deletion reports:

```yaml
# Network/old-net status excerpt
phase: Degraded
deletionRequestedAt: "2026-07-22T00:10:00Z"
conditions:
  - type: NetworkDraining
    status: "True"
    reason: configuration-generation-removed
    message: "absent from Zone configurationGeneration 7; deletion in progress"
    observedGeneration: 3
    lastTransitionAt: 2026-07-22T00:10:00Z
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
| New `attachments[]` entry | Updates config Volume (attachments.json, dnsmasq.conf); creates persistent tap; stores opaque attachment realization; sets bridge port flags |
| Removed `attachments[]` entry | Updates config Volume; waits for Guest/VMM FD closure; calls generation-fenced `DeletePersistentTap`; retains handle until confirmed |
| `spec.dhcp.*` / `spec.dns.*` change | Updates config Volume (dnsmasq.conf); guest-agent applies SIGHUP reload; no Guest restart required |
| `spec.routing.hostBlocklist` change | Updates config Volume (nftables.rules); guest-agent applies atomic `nft replace`; no Guest restart required |
| `spec.isolation.allowEastWest` change | Updates config Volume (nftables.rules); guest-agent applies atomic `nft replace`; also updates bridge port flags via broker |
| `spec.externalAttachment.*` port-forward / egress change | Updates config Volume (nftables.rules); no Guest restart required |
| `spec.externalAttachment` add/remove | Updates config Volume AND updates Guest spec (NIC topology change); Guest switch/restart may occur |
| `spec.mdns.enable` false → true | Creates owned mDNS `Process` resources |
| `spec.mdns.enable` true → false | Deletes owned mDNS `Process` resources through their finalizers |
| Any `spec.lanCidr` / `spec.uplinkCidr` change | Updates config Volume; re-reconciles full bridge/nftables/net-VM chain |

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
- A persistent-tap transient failure retains the opaque handle and retries with
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
| Current anchor | `nixos-modules/network.nix` (bridge/NAT/sysctl, 500+ lines), `nixos-modules/net.nix` (net-VM NixOS config, 450 lines), `nixos-modules/options-envs.nix` (`d2b.envs.<env>.*`), `nixos-modules/options-realms-network.nix` (`d2b.realms.<realm>.network.*` mode/cidrs), `nixos-modules/options-vms.nix` (`d2b.vms.<vm>.env` line 944, `d2b.vms.<vm>.index` line 962, `d2b.vms.<vm>.staticIp` deprecated line 974), `nixos-modules/options-site.nix` (`d2b.site.allowUnsafeEastWest` line 48, `d2b.hostLanCidrs` line 382), `nixos-modules/options-realms-workloads.nix` (`d2b.realms.<realm>.workloads.<workload>.networkIndex` line 326), `nixos-modules/host-json.nix` (emits `host.json` `environments[].nftables`, `environments[].ifNameMappings`, `environments[].usbipBusidLocks`), `nixos-modules/processes-json.nix` (emits `processes.json` `ProcessNetworkInterface`/`ProcessMacvtapInterface` per-VM runner), `nixos-modules/lib.nix` (`subnetIp` line 399, `subnetMask` line 408, `mkMac` ~line 60, `cidrOverlaps` lines 429–462), `nixos-modules/index.nix` (netMeta section), `packages/d2b-core/src/host.rs` (`NetEnv` lines 290–328, `VmRuntimeRow` lines 155–167 with `tap`/`bridge`/`net_vm`/`env` fields, `ExternalNetworkPolicy` lines 332–413, `NftablesModel` lines 520–549, `BridgePortFlags`, `TapRole`, `Ipv6SysctlEntry`, `IfNameMapping` lines 242–256), `packages/d2b-core/src/processes.rs` (`ProcessNetworkInterface` lines 98–113, `ProcessMacvtapInterface`), `packages/d2b-contracts/src/broker_wire.rs` (`ApplyNftables`, `ApplyNmUnmanaged`, `ApplyRoute`, `ApplySysctl`, `CreatePersistentTap`, `CreateTapFd`, `SetBridgePortFlags`, `UpdateHostsFile`, `SeedDnsmasqLease`), `packages/d2b-host/src/ifname.rs` (FNV-1a derivation), `packages/d2b-host/src/nftables.rs` (`NftBatch`, `hash_inet_d2b_table`, coexistence policy), `packages/d2b-host/src/bridge_port.rs`, `packages/d2b-host/src/routes.rs`, `packages/d2b-host/src/netlink.rs`, `packages/d2b-host-providers/src/lib.rs` (unwired ADR 0032 runtime/display/substrate provider adapters; no network Provider trait) |
| Evidence class | All current network Nix modules, host.rs/processes.rs DTOs, and broker ops are `implemented-and-reachable`. `d2b.realms.<realm>.network` with `mode="declared"` is `implemented-and-reachable` as the v2-native transitional surface (superseded by Network ResourceType at v3 reset). `packages/d2b-host-providers/src/lib.rs` provider trait surface is `implemented-but-unwired` (ADR 0032 realm trait adapters). The v3 Network ResourceType, `Provider/network-local`, and controller are `ADR-only`. `ProcessNetworkInterface`/`ProcessMacvtapInterface` are `implemented-and-reachable` in the current daemon but map to Guest spec network fields under `Provider/runtime-cloud-hypervisor`, not to NetworkSpec. |
| Behavior retained | `lib.mkForce` 10-eth-dhcp neutralization; bridge isolation (`Isolated=true` default); IPv6 suppression at boot and runtime; `cidrOverlaps` arithmetic; `hostBlocklist` default set; IfName FNV-1a derivation and collision detection; per-Network east-west opt-in (`isolation.allowEastWest`); dnsmasq DHCP static reservations with `dhcp-ignore-names`; `bind-interfaces`; hardened systemd confinement; nftables `inet d2b` table with ownership markers; firewall coexistence policy matrix; net-VM nftables drop IPv6 on all chains; MSS clamp; macvtap external attachment (via SpawnRunner/runtime-ch); DHCP/static IPv4 on `external0`; egress CIDRs and MASQUERADE; port-forward DNAT; `ConfigureWithoutCarrier` on uplink bridge (emitter-owned) |
| Required delta | Network ResourceType schema; spec/status API; `network-local` Provider and controller crate; async reconcile loop; owned net-VM Guest lifecycle; owned config Volume lifecycle (`Volume/net-<networkName>-config`) and guest-agent Process lifecycle (`Process/net-<networkName>-agent`) for runtime config delivery; owned mDNS Process lifecycle (D-NETWORK-001); CIDR overlap validation at reconcile time; RBAC for network resources (Network, Guest, Volume, Process, Host); OTEL spans/metrics; Nix resource emitter (`resources-network.nix`, bootstrap/static prerequisites only — no `systemd.network.netdevs`); removal of `d2b.envs.*` and `d2b.realms.<realm>.network` surfaces; new canonical `DeletePersistentTap` paired with `CreatePersistentTap`, plus new broker ops `CreateBridge` and `DeleteBridge`, in `broker_wire.rs` and `runtime.rs` (D-NETWORK-003) |
| Reuse path | Extract `subnetIp`/`mkMac`/`cidrOverlaps` from `lib.nix`; copy IfName/derive/detect_collisions from `ifname.rs`; adapt `NetEnv`/`ExternalNetworkPolicy`/`NftablesModel`/`BridgePortFlags`/`TapRole`/`Ipv6SysctlEntry`/`IfNameMapping` from `host.rs`; extract nftables/bridge-port/routes/netlink modules from `d2b-host`; adapt `net.nix` and `network.nix` into sealed v3 template and controller. `VmRuntimeRow.tap`/`bridge`/`net_vm`/`env` fields (host.rs lines 155–167) become Network attachment status fields. `ProcessNetworkInterface`/`ProcessMacvtapInterface` (processes.rs) migrate to Guest spec under Provider/runtime-cloud-hypervisor (not NetworkSpec). |
| Replacement/deletion | `nixos-modules/network.nix`, `nixos-modules/net.nix`, `nixos-modules/options-envs.nix`, `nixos-modules/options-realms-network.nix`, `nixos-modules/index.nix` envMeta section removed only after `nixos-modules/resources-network.nix` and Provider/network-local controller pass parity tests; `d2b.envs.*` options removed only after the v3 cutover and consumer migration |
| Feasibility proof | All network invariants have passing golden/integration tests at v3 baseline; IfName derivation has property tests; bridge-port isolation has integration tests; nftables apply has coexistence matrix unit tests; no new proof required before spec acceptance |
| Future owner | `ADR046-network-*` work items below |

## Decisions

All decisions for this spec are resolved. No action is required from the
integrator before spec acceptance.

### D-NETWORK-001: mDNS reflector process identity — RESOLVED

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

### D-NETWORK-002: USBIP proxy process ownership — RESOLVED

**Resolution**: The USBIP backend and proxy Processes are owned by
`Provider/device-usbip`, not the Network controller. The device controller
watches only Network identity/readiness/generation. Its typed EffectPort
privately resolves the Network UID and dispatches the closed
`UsbipBindFirewallRule` broker operation with closed action enum
`Ensure|Remove` for exact per-Network/per-busid TCP/3240 exposure.
`Remove` is generation-bound, ownership-scoped, foreign-marker fail-closed, and
idempotent after validated absence. Device-usbip owns one multiplexed relay `Endpoint` authority
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

### D-NETWORK-003: Runtime bridge creation — RESOLVED

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
object while leaving kernel state alive until the next switch — violating the
finalizer contract.

**New broker ops required**: `CreateBridge` and `DeleteBridge` must be authored
and added to `packages/d2b-contracts/src/broker_wire.rs` and implemented in
`packages/d2b-priv-broker/src/runtime.rs` as `RealBrokerRequest` handlers. Both
ops require security review and broker policy coverage.

**Unblocks**: ADR046-network-004 (Nix emitter no longer generates
`systemd.network.netdevs`), ADR046-network-005 (controller creates/deletes
bridges at reconcile time).


### ADR046-network-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-001` |
| Dependency/owner | W0 shared contract root; `d2b-contracts` |
| Current source | `packages/d2b-core/src/host.rs` lines 290–520 (`NetEnv`, `IfName`, `ExternalNetworkPolicy`, `NftablesModel`, `BridgePortFlags`, `TapRole`, `Ipv6SysctlEntry`, `IfNameMapping` lines 242–256; **also** `VmRuntimeRow` lines 155–167 with `tap`/`bridge`/`net_vm`/`env` fields — attachment status precursors); `packages/d2b-core/src/processes.rs` lines 98–141 (`ProcessNetworkInterface`, `ProcessNetworkInterfaceType`, `ProcessMacvtapInterface` — current VMM runner network interface DTOs; these are per-Guest VMM fields, not Network-level fields, and migrate to Guest spec under `Provider/runtime-cloud-hypervisor`); `packages/d2b-contracts/src/broker_wire.rs` (authoritative broker op list; network-relevant: `ApplyNftables`, `ApplyNmUnmanaged`, `ApplyRoute`, `ApplySysctl`, `SetBridgePortFlags`, `UpdateHostsFile`, `SeedDnsmasqLease`, `CreatePersistentTap`, `CreateTapFd`); `nixos-modules/lib.nix` lines 396–460 (`subnetIp`, `subnetMask`, `mkMac`, `cidrOverlaps`) |
| Reuse source | None from main; all from v3 baseline |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/network.rs`: NetworkSpec, NetworkStatus, AttachmentSpec, AttachmentStatus, ExternalAttachmentSpec, ExternalAttachmentStatus, PortForwardSpec, NetworkConditionType, opaque AttachmentHandle, and AttachmentGenerationFence; `packages/d2b-contracts/src/v3/ifname.rs`: IfName newtype, derivation, collision detection (extracted from `d2b-host/src/ifname.rs`). Also defines `User/net-local-controller` as a proper Resource with explicit lifecycle: `Provider/network-local`'s Nix package/module provisions the reserved `net-local-controller` OS account with a private fixed UID/GID in Host prerequisites and in the generic net-VM nixos-system artifact (same account, same UID/GID inside the Guest); the network-local controller creates and owns the User Resource (`spec.osUsername: net-local-controller`, `ownerRef: Provider/network-local`, `managedBy: controller`); `Provider/system-core` verifies the account via NSS lookup and reconciles the User Resource to Ready — it does not provision the OS account. No numeric UID/GID enters any ResourceSpec field, authz check, or audit record; `User.status` MAY carry diagnostic `uid`/`gid` values discovered by NSS lookup, but those are informational only and are never authorization inputs. The network-local controller waits for `User/net-local-controller` to reach `Ready` before creating any config Volume (reconcile precondition, not a bootstrap side effect). |
| Detailed design | Strict ResourceEnvelope with Network-specific spec/status. IfName newtype: IFNAMSIZ-1 validated, FNV-1a 64-bit derivation, base32 Crockford, 8-char suffix, bridge/tap role prefixes, detect_collisions over IfNameMapping slice. cidrOverlaps: pure Rust IPv4 arithmetic, same algorithm as lib.nix. NetworkSpec validators: /24 lanCidr with .0 base, /30 uplinkCidr, unique attachment indices 2–250, default hostBlocklist enforcement. The opaque attachment realization binds Network UID/generation and attachment UID/generation so deletion can supply a non-printable ID plus explicit expected generation fence without an IfName/path. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | Provider dossiers, Nix resource compiler, resource store/API bind these canonical types |
| Data migration | Full d2b 3.0 reset; no env→Network import |
| Validation | Golden JSON/CBOR vectors; CIDR overlap property tests; IfName collision and derivation determinism tests; default hostBlocklist enforcement; attachment index uniqueness; `User/net-local-controller` User resource lifecycle/readiness test: controller creates User Resource with `spec.osUsername = "net-local-controller"` (`ownerRef: Provider/network-local`); controller waits for User resource to reach `Ready` before proceeding; controller aborts with `ConfigVolumeReady=False/user-not-ready` if User resource is not Ready; verifies no numeric UID/GID appears in the Resource spec, authz check, or audit record; verifies that any diagnostic `uid`/`gid` in `User.status` is never used as an authorization input |
| Removal proof | Old `d2b_core::host::NetEnv` and related types removed only after v3 resource API consumers use `d2b_contracts::v3::network` types |

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

### ADR046-network-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-003` |
| Dependency/owner | ADR046-network-001, ADR046-network-002; Provider/runtime-cloud-hypervisor dossier owner |
| Current source | `nixos-modules/net.nix` (full file, 450 lines); `nixos-modules/net-mdns.nix`; `nixos-modules/lib.nix` subnetIp/mkMac/cidrOverlaps |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-network-local/` — artifact catalog integration for net-VM nixos-system artifact resolution; `packages/d2b-provider-network-local/nix/` — default net-VM NixOS module (parameterized successor to net.nix), built and registered as a nixos-system artifact in `d2b.artifacts` |
| Detailed design | `Network.spec.netVmSystemArtifactId` is REQUIRED. It must reference a declared `d2b.artifacts` entry with `type = "nixos-system"`; verified at Nix build time (Stage 2 check, hard build error if absent or wrong type). No implicit default exists; Provider artifacts cannot silently provide a separately typed system artifact. The controller sets `Guest.spec.systemArtifactId` to the artifact ID value at reconcile time (the value is already validated by the build; the controller fails closed if absent at runtime). The net-VM nixos-system artifact is **generic** (INV-NET-008): it contains the guest-agent binary and runtime, kernel, base NixOS services, systemd-networkd NIC bootstrap, and the `net-local-controller` **OS account** provisioned by `Provider/network-local`'s Nix module (same private fixed UID/GID as on the Host, so that virtiofs view ACLs on config Volume layout entries are enforced consistently inside the Guest; `Provider/system-core` performs NSS lookup reconciliation, not OS account provisioning; no numeric UID/GID appears in any ResourceSpec field, authz check, or audit record; `User.status` MAY carry diagnostic `uid`/`gid` from NSS lookup but those are informational only and never authorization inputs). It does NOT encode per-Network desired data; per-Network config (dnsmasq, nftables, routing, attachments) is delivered via the controller-owned config Volume and applied by the guest-agent Process. The artifact preserves compile-time-fixed content: `lib.mkForce` on 10-eth-dhcp (INV-NET-001); two systemd-networkd interface units matched by MAC; IPv6 suppression sysctls on NIC interfaces; ip6 filter table drop-all policy. **mDNS reflector and local dnsmasq DNS bridge are separate owned Process resources** (D-NETWORK-001); they are not inline services in the artifact. |
| Integration | Network controller resolves artifact ID → sets `Guest.spec.systemArtifactId`. Controller separately creates `Volume/net-<networkName>-config` with per-Network config and `Process/net-<networkName>-agent` (guest-agent). `Provider/runtime-cloud-hypervisor` reads `systemArtifactId` to produce the net-VM bundle and mounts the Volume view into the Guest. |
| Data migration | Destructive v3 reset; existing net VMs are re-created under new IfNames |
| Validation | nix-unit: `tests/unit/nix/cases/net-vm-network.nix` (adapted to v3 resource API); INV-NET-001 assertion in new nix-unit case; no mDNS inline service appears in the generated artifact; no per-Network dnsmasq or nftables data in artifact (INV-NET-008); integration test: mDNS Process resources are created when `spec.mdns.enable = true`; Stage 2 build test: absent `netVmSystemArtifactId` fails with required-field build error; wrong artifact type fails with `artifact-type-mismatch` error; `packages/d2b-provider-network-local/tests/net_vm_artifact_is_generic.rs` — two Networks with different CIDRs produce same `systemArtifactId` and different config Volume content |
| Removal proof | `nixos-modules/net.nix` and `nixos-modules/net-mdns.nix` removed only after net-VM artifact parity tests pass |

### ADR046-network-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-004` |
| Dependency/owner | ADR046-network-001, ADR046-network-002, ADR046-network-003; Nix integrator |
| Current source | `nixos-modules/network.nix` (full file, 500+ lines; bridge/netdev/sysctl/NM-unmanaged/route sections); `nixos-modules/host-json.nix` (emits `host.json` `environments[].nftables`, `environments[].ifNameMappings`, `environments[].usbipBusidLocks` — superseded by Network status API); `nixos-modules/processes-json.nix` (emits `processes.json` `ProcessNetworkInterface`/`ProcessMacvtapInterface` per runner — superseded by Guest spec network fields); `nixos-modules/index.nix` envMeta/netMeta sections; `nixos-modules/options-envs.nix`; `nixos-modules/options-realms-network.nix`; `nixos-modules/options-vms.nix` (`d2b.vms.<vm>.env` line 944, `d2b.vms.<vm>.index` line 962 — current attachment semantics source; maps to `Network.spec.attachments[].executionRef` + `index`); `nixos-modules/options-site.nix` (`d2b.site.allowUnsafeEastWest` line 48 — maps to per-Network `Network.spec.isolation.allowEastWest`; Zone.spec is empty in v3; `d2b.hostLanCidrs` line 382 — maps to Host resource network inventory at runtime; not a Zone.spec field) |
| Reuse action | adapt |
| Destination | `nixos-modules/resources-network.nix`: Nix resource object emitter for Network ResourceType; `nixos-modules/index.nix`: network resource compilation section |
| Detailed design | The emitter replaces `d2b.envs.<env>` with `d2b.zones.<zone>.resources.<name> = { type = "Network"; spec = { ... }; }` (attr key = resource name; `type` explicit field; `spec` fields identical to the canonical ResourceSpec JSON — no bespoke Nix vocabulary). It validates CIDR shape, attachment index uniqueness, external attachment constraints, and CIDR overlap (reusing `cidrOverlaps` from `lib.nix`). **Bridges are NOT emitted as `systemd.network.netdevs` entries** (D-NETWORK-003 resolved; bridges are created dynamically by the broker `CreateBridge` op at reconcile time). The Nix emitter provisions only bootstrap/static prerequisites that do not require runtime bridge IfNames: `networking.networkmanager.unmanaged` pattern for the `d2b-*` prefix (covers all dynamically-created bridges and taps regardless of specific IfNames; emitted to `00-d2b-unmanaged.conf`); schema validation and controller binary deployment artifacts. Current `d2b.vms.<vm>.env` + `d2b.vms.<vm>.index` attachment semantics (`options-vms.nix` lines 944/962) become `Network.spec.attachments[].executionRef` + `index`. Current `d2b.site.allowUnsafeEastWest` (`options-site.nix` line 48) moves to the per-Network `isolation.allowEastWest` field; Zone.spec is empty in v3. Current `d2b.hostLanCidrs` (`options-site.nix` line 382) becomes the Host resource's network inventory, queried at runtime; at Nix build time the eval may validate CIDRs against declared host configuration input. The emitter does not emit `boot.kernel.sysctl` entries per bridge IfName (bridges do not exist at activation time; IPv6 suppression is applied by `CreateBridge` and `ApplySysctl` per INV-NET-002). **Nix option types** for `spec.*` fields are generated from `Network.schema.json`; they are not hand-written. **Bundle generation**: the emitter collects all declared `Network` resource objects, sorts them lexicographically by `(type, name)`, serializes each as canonical JSON **omitting `managedBy` and `configurationGeneration`** (core sets these at activation), and assembles the Zone resource bundle at `$out/bundle.json` (see [Nix configuration contract — Stage 3](#stage-3--build-output-zone-resource-bundle)). The emitter records a `providerSchemaDigest` entry for `Provider/network-local` in the bundle resolved from the artifact catalog. The bundle's `contentHash` is a SHA-256 of the sorted canonical resource array; the derivation is fixed-output so that identical configuration always produces the same store path. The `managedBy` field is NOT set by the emitter; core sets `managedBy = "configuration"` and assigns the `configurationGeneration` counter when applying the bundle. The core controller retains prior bundle copies under `/var/lib/d2b/zones/<zone>/configuration/prior/` per [Generation lifecycle and cleanup contract](#generation-lifecycle-and-cleanup-contract). |
| Integration | Nix resource objects serialize exactly the Rust NetworkSpec contract (ADR046-network-001). The provider install declares the schema digest. Zone runtime generation-transition logic (ADR046-network-008) reads the bundle at activation. |
| Data migration | Full v3 reset; `d2b.envs.*` declarations must be rewritten as Network resources |
| Validation | nix-unit CIDR overlap, assertion eval, and bridge-sysctl cases; `make test-flake` with updated examples; `make test-drift` for schema/emitter parity; `packages/d2b-contracts/tests/generation_bundle.rs` for bundle format and `contentHash` stability; nix-unit `tests/unit/nix/cases/generation-cleanup-absent-network.nix` for removed-resource scheduling (added by ADR046-network-008) |
| Removal proof | `nixos-modules/network.nix`, `nixos-modules/options-envs.nix`, and `nixos-modules/options-realms-network.nix` removed only after `resources-network.nix` and controller reach parity; `d2b.envs` consumer migration guide updated |

### ADR046-network-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-005` |
| Dependency/owner | ADR046-network-001–004; network-local controller owner; D-NETWORK-001, D-NETWORK-002, and D-NETWORK-003 resolved |
| Current source | `nixos-modules/network.nix` (tap/sysctl sections); `packages/d2b-host/src/{bridge_port,nftables,netlink,routes}.rs`; broker ops in `packages/d2b-contracts/src/broker_wire.rs`: **real runtime ops** `ApplyNftables`, `ApplyNmUnmanaged`, `ApplyRoute`, `ApplySysctl`, `CreatePersistentTap`, `SetBridgePortFlags`, `UpdateHostsFile`, `SeedDnsmasqLease` (all `implemented-and-reachable`); **new ops to author**: canonical `DeletePersistentTap` paired with `CreatePersistentTap`, plus `CreateBridge` and `DeleteBridge` (do not exist in v3 baseline; must be added to `broker_wire.rs` and implemented as `RealBrokerRequest` handlers in `packages/d2b-priv-broker/src/runtime.rs`); **NOT current ops**: no unsuffixed tap-deletion alias is valid, and `CreateMacvtap` does not exist — macvtap is created inside broker's `SpawnRunner` dispatch (`packages/d2b-priv-broker/src/runtime.rs` line 5097 `live_create_macvtap_fd`) |
| Reuse action | adapt |
| Destination | `packages/d2b-provider-network-local/src/controller.rs`: async NetworkReconciler; `packages/d2b-provider-network-local/src/plan.rs`: ReconcilePlan computation; `packages/d2b-provider-network-local/src/observe.rs`: drift-detection observe loop. Full crate layout required (see [Package and crate boundary](#package-and-crate-boundary)): `src/` (controller/plan/observe + colocated unit tests), `tests/` (hermetic conformance and state-machine tests), `integration/` (provider-system reconcile fixtures), `README.md` (Network ResourceType, controller binary, placement, RBAC, security invariants, build/test/integration commands). |
| Detailed design | Implements full async reconcile interface from `ADR-046-resource-reconciliation`. `plan()` computes desired vs. actual bridge-presence, sysctl, host-side nftables, hosts-file, NM-unmanaged, config Volume content, Guest, guest-agent Process, and mDNS-Process states. `reconcile()` dispatches in order: `CreateBridge` for each bridge not present (broker applies IPv6 sysctls atomically; `CreateBridge` failure sets `FabricReady=False/bridge-create-error` and aborts) → `ApplySysctl` (defense-in-depth IPv6) → `ApplyNftables` (host-side `inet d2b` table) → `ApplyNmUnmanaged` → `ApplyRoute` → `UpdateHostsFile` → `SeedDnsmasqLease` for new reservations → **Volume upsert** (two-phase): Phase 1 — create `Volume/net-<networkName>-config` with `kind: ephemeral`, `source.executionRef: Host/<hostName>`, `source.settings.kind: tmpfs`, `quota: {maxBytes: 4194304, maxInodes: 128, enforcement: hard}` (tmpfs quota charged to Host memory budget), `layout` entries (root directory with `type: directory` and four config files each with `type: file`, `ownerRef: User/net-local-controller`, `groupRef: User/net-local-controller`, `mode: "0640"`, `accessAcl: []`, `defaultAcl: []`, `noFollow: true`, conservative create/repair/cleanup policies), `views: {guest-readonly: {path: "", rights: [read, traverse]}}`, `attachments: []` (no Guest attachment); abort on terminal error with `ConfigVolumeReady=False/config-volume-error`. Wait for Volume backing to reach `Ready`; requeue on `Degraded`/`Failed` with `ConfigVolumeReady=False/volume-not-ready`. Write rendered config content through Volume write service (no raw host paths). Phase 2 — create Guest upsert with `systemArtifactId` from REQUIRED `Network.spec.netVmSystemArtifactId`. Wait for Guest `Ready`. Then update Volume with Guest attachment: `attachments: [{executionRef: Guest/<netVmName>, transport: virtiofs, view: guest-readonly, access: read-only, mountPath: "/run/d2b/net-config", settings: {posixAcl: false, xattr: false, cache: auto, inodeFileHandles: never, threadPoolSize: null, socketGroup: null}}]`; wait for attachment `Ready`; requeue on `Degraded` with `ConfigVolumeReady=False/attachment-not-ready`. Create or update guest-agent Process `Process/net-<networkName>-agent` with `processClass: worker`, `sandbox: {namespaceClasses: [], capabilityClasses: [network-admin, network-bind, network-raw]}` (inherits Guest network namespace; `network-admin`→`CAP_NET_ADMIN`, `network-bind`→`CAP_NET_BIND_SERVICE`, `network-raw`→`CAP_NET_RAW`, all effective in Guest network namespace only; INV-NET-009), `mounts: [{volumeRef: Volume/net-<networkName>-config, view: guest-readonly, mountPath: "/run/d2b/net-config", access: read-only, required: true}]`. mDNS Process upsert when `spec.mdns.enable = true` (D-NETWORK-001) → `SetBridgePortFlags` per tap. Removed attachments first wait for Guest/VMM FD ownership to close, then issue `DeletePersistentTap` with the retained opaque attachment ID and current expected Network/attachment generations; the handle remains retained until confirmed effect or validated absence. Stale generation refreshes/requeues, transient kernel error retries, and foreign marker fails closed. Each broker op returns typed audit evidence. `observe()` re-reads `firewallDigest` (host-side), bridge isolation flags, IPv6 sysctls, and guest-agent Process status (`dnsmasq-bound`, `firewall-applied` predicates); queues reconcile on drift. Metrics use only the fixed semantic labels in §OTEL spans and metrics; Zone/Network identity remains in OTEL resource attributes and permitted audit fields and never enters metric labels or span attributes. **Finalizer** (strictly child-first): `NetworkDraining` → stop workload Guests and await VMM FD closure → generation-fenced `DeletePersistentTap` for each retained attachment, awaiting confirmation → delete guest-agent Process and mDNS Processes; wait for each Deleted watch event → update Volume to remove Guest attachment (`attachments: []`); wait for attachment removal confirmed → delete `Guest/<netVmName>`; wait for Deleted watch event → delete `Volume/net-<networkName>-config`; wait for Deleted watch event → `ApplyNftables` (empty) → `ApplyNmUnmanaged` (empty) → `UpdateHostsFile` (empty) → `DeleteBridge` for each bridge (idempotent) → clear finalizer. No USBIP rules installed by Network; device-usbip issues the existing `UsbipBindFirewallRule` request with action `Ensure` for apply or `Remove` for release (D-NETWORK-002). |
| Integration | Controller process registers descriptor, watches `Network` resources via d2b-bus/ComponentSession/ResourceClient. Owned Guest and Process mutations trigger owner reconciliation. Device-usbip watches only Network identity/readiness/generation; its Core adapter privately resolves relay/firewall effects (D-NETWORK-002). |
| Data migration | None after full reset |
| Validation | `ADR046-reconcile-001` toolkit conformance; latency gates (p95 ≤5 ms hint-to-handler); Network-specific: CIDR conflict blocks reconcile, `CreateBridge` failure sets `FabricReady=False`, Volume creation failure sets `ConfigVolumeReady=False/config-volume-error`, `User/net-local-controller` not Ready aborts with `ConfigVolumeReady=False/user-not-ready`, Volume schema round-trip (kind=ephemeral, source.settings.kind=tmpfs, quota.enforcement=hard, layout type=file entries, views.guest-readonly.rights=[read,traverse]), tmpfs quota charged to Host memory budget (test Volume creation fails when Host memory budget exceeded), Guest not created before Volume backing `Ready`, Guest attachment not added before Guest `Ready`, guest-agent Process created after attachment `Ready` (`processClass: worker`, `namespaceClasses: []`, `capabilityClasses: [network-admin, network-bind, network-raw]`, `access: read-only`, `required: true`), host-capability leakage test: no host-netns process gains `CAP_NET_ADMIN`/`CAP_NET_BIND_SERVICE`/`CAP_NET_RAW` as result of guest-agent launch (INV-NET-009; `tests/host-integration/guest-agent-cap-confinement.nix`), removed attachment and finalizer call `DeletePersistentTap` only after Guest/VMM FD closure with opaque ID/current generations, validated absence succeeds, transient failure retains handle/retries, stale generation refreshes, foreign marker blocks without deletion, request/audit contain no IfName/path, `DeleteBridge` called only after tap confirmations, Volume attachment removed before net-VM Guest deletion in finalizer (test order: workload FD closure → persistent taps deleted → agent Deleted → Volume attachment removed → net-VM Guest Deleted → Volume Deleted → bridges), east-west invariant (INV-NET-003), hostBlocklist enforcement (INV-NET-004), macvtap attachment status (delegated to runtime-ch), mDNS Process created/deleted with `spec.mdns.enable` toggle, broker INV-NET-002 tests, config-only spec change updates Volume content and triggers agent reload without Guest restart (INV-NET-008); golden tests updated for v3 IfNames; structural metric descriptor test asserts exact absence of `vm`, `zone`, `zone_id`, `zone_uid`, `network`, and every resource-name-derived label and verifies a Network-name canary is absent from emitted label values |
| Removal proof | Daemon-orchestrated network/bridge lifecycle removed only after controller passes conformance and parity tests |

### ADR046-network-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-006` |
| Dependency/owner | ADR046-network-001, ADR046-network-005; test owner |
| Current source | `tests/unit/nix/cases/net-vm-network.nix`; `tests/golden/pinned/net-vm-bundle-gate.txt`; `tests/golden/pinned/net-canaries.txt`; `tests/golden/pinned/host-prepare-network.txt`; `tests/host-integration/bridge-isolation.nix`; `tests/integration/live/network-isolation.sh` |
| Reuse action | adapt |
| Destination | `tests/unit/nix/cases/net-vm-network.nix` (adapted to v3 resource API); updated golden pins; `tests/host-integration/bridge-isolation.nix` (adapted); `packages/d2b-priv-broker/tests/{bridge_lifecycle,persistent_tap_lifecycle}.rs` (new hermetic broker tests). Provider crate test directories: `packages/d2b-provider-network-local/tests/` — hermetic Cargo integration tests (conformance suite, controller state machine, CIDR validation vectors, IfName determinism, invariant tests INV-NET-001–007, reconcile/observe/finalize with deterministic clock, fault injection); `packages/d2b-provider-network-local/integration/` — container/Host/Guest lifecycle fixtures invoked by `make test-integration` (bridge isolation, east-west double opt-in, nftables drift detection, persistent-tap and macvtap lifecycle). Both directories required by package policy. |
| Detailed design | Rust integration tests: NetworkSpec CIDR validation golden vectors; AttachmentSpec index uniqueness; ExternalAttachmentSpec mutual-exclusion validation; IfName derivation determinism; CIDR overlap arithmetic; INV-NET-001 through INV-NET-009 invariant tests; reconcile/observe/finalize state machine (deterministic clock). Broker tests: `create_bridge_applies_ipv6_sysctl` (INV-NET-002 layer 1); `delete_bridge_is_idempotent`; `delete_bridge_never_cascades_attached_tap`; `create_bridge_parameters_match_spec` (MTU, STP disabled, multicast snooping disabled); `delete_persistent_tap_pairs_with_create`; `delete_persistent_tap_absent_is_idempotent_after_ownership_validation`; `delete_persistent_tap_rejects_stale_network_generation`; `delete_persistent_tap_rejects_stale_attachment_generation`; `delete_persistent_tap_foreign_marker_fails_closed`; `delete_persistent_tap_request_and_audit_have_no_ifname_or_path`. Controller tests: `reconcile_applies_sysctl_defense_in_depth` (INV-NET-002 layer 2); `volume_created_before_guest`; `guest_not_created_until_volume_ready`; `agent_process_created_after_guest`; `removed_attachment_waits_for_vmm_then_delete_persistent_tap`; `finalizer_order_vmm_then_taps_then_agent_then_guest_then_volume_then_bridges`; `delete_persistent_tap_transient_retry_retains_handle`; `delete_persistent_tap_generation_mismatch_refreshes`; `delete_persistent_tap_foreign_marker_blocks_finalizer`; `config_only_spec_change_updates_volume_no_guest_restart` (INV-NET-008); `finalizer_calls_delete_bridge`; `mdns_process_created_on_enable`; `mdns_process_deleted_on_disable`; `host_capability_leakage` (INV-NET-009). nix-unit: INV-NET-001 lib.mkForce assertion; net-VM artifact has no inline mDNS service and no per-Network dnsmasq/nftables data (INV-NET-008); Network emitter CIDR constraint assertions; no `systemd.network.netdevs` bridge entries emitted. Host integration: bridge isolation with east-west opt-in; nftables drift detection; persistent-tap and macvtap create/delete lifecycle; config Volume update propagates to guest-agent without Guest restart; `tests/host-integration/guest-agent-cap-confinement.nix` (INV-NET-009 zero leakage to host netns). |
| Integration | Pinned tests registered in `tests/golden/pinned/`; nix-unit cases in `tests/unit/nix/cases/`; host integration in `tests/host-integration/` |
| Data migration | None — full d2b 3.0 reset; no prior state to migrate |
| Validation | All listed tests must pass before `nixos-modules/network.nix` removal is eligible |
| Removal proof | Not applicable (this work item IS the test successor) |

### ADR046-network-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-007` |
| Dependency/owner | ADR046-network-005; device-usbip Provider dossier; D-NETWORK-002 resolved |
| Current source | `nixos-modules/network.nix` lines 444–461 (USBIP host firewall); `packages/d2b-core/src/host.rs` lines 324–328 (usbip_backend_port, usbip_busid_locks in NetEnv); `packages/d2b-host/src/` usbip_argv.rs |
| Reuse action | adapt |
| Destination | `Provider/device-usbip` owns one relay Process/Endpoint authority per Network plus the typed EffectPort adapter for the existing closed `UsbipBindFirewallRule` request with closed action enum `Ensure|Remove`. The controller watches only the `networkRef` resource's identity/readiness/generation; Core privately resolves Network UID to relay attachment and firewall intent. Network spec/status is not mutated with USBIP fields. Full crate layout required for `packages/d2b-provider-device-usbip/` (see [Package and crate boundary](#package-and-crate-boundary)): `src/` (controller and usbip runner + unit tests), `tests/` (hermetic conformance, dependency-watch state machine, `UsbipBindFirewallRule` `Ensure|Remove` round-trip), `integration/` (Host/Guest USBIP attach/detach lifecycle fixtures), `README.md` (Provider identity, provider-neutral USB Service/Binding types, USBIP Processes/Endpoints, Network least-privilege dependency contract, RBAC, security invariants, standalone-repo path). |
| Detailed design | Device-usbip's typed EffectPort is the sole semantic owner of every USBIP TCP/3240 rule. Its Core adapter resolves the opaque per-Network/per-busid intent and issues the same `UsbipBindFirewallRule` request with action `Ensure` for apply and `Remove` for release; no separate release op exists. `Remove` is generation-bound, ownership-scoped, idempotent after validated absence, and foreign-marker fail-closed. The controller retains firewall token/status and the relay authority reference until the broker confirms `Remove`; its strict provider status owns firewall digest/drift. Network-local emits no generic host or net-VM TCP/3240 allow and ignores device-usbip ownership markers in Network drift. The device Provider owns exactly one multiplexed relay Endpoint authority per Network and supplies Binding proxies only authorized connected streams through LaunchTickets. Primary reuse disposition: `adapt`. Preserved source-plan detail: extract and adapt. |
| Integration | device-usbip watches Network readiness → Core adapter resolves opaque Network attachment → `UsbipBindFirewallRule { action: Ensure, ... }` for apply or `{ action: Remove, ... }` for release + one relay Endpoint authority → Binding proxy LaunchTicket; release clears status/authority only after confirmed `Remove` |
| Data migration | Current network.nix USBIP carve-out replaced by UsbipBindFirewallRule broker op |
| Validation | device-usbip conformance tests cover the exact closed `Ensure|Remove` enum (unknown actions rejected), same-request broker mapping for apply/release, expected Network/Service generation binding, exact per-Network/per-busid scoping, idempotent validated-absence `Remove`, one relay Endpoint authority, ownership-scoped drift/status, foreign-marker rejection, transient retry, and retention of status/token/authority until effect confirmation; network-local nftables tests assert no TCP/3240/USBIP rule on host or net VM and prove USBIP rule churn does not change Network `FirewallReady`; the pinned USBIP firewall golden moves to device-usbip ownership |
| Removal proof | Network.nix USBIP sections removed only after UsbipBindFirewallRule mechanism passes conformance |

### ADR046-network-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-008` |
| Dependency/owner | ADR046-network-004, ADR046-network-005; Zone runtime integrator |
| Current source | No current v3 source: generation lifecycle and `managedBy`/`configurationGeneration` classification do not exist on the pre-ADR45 v3 baseline. The v3 baseline uses NixOS activation scripts that atomically replace all host JSON artifacts; there is no per-resource generation tracking, no async cleanup queue, and no `managedBy` field. |
| Reuse action | create |
| Destination | `packages/d2b-core-controller/src/configuration.rs`: bundle application, diff, generation-transition logic (including per-item name-conflict handling), prior-bundle retention under `/var/lib/d2b/zones/<zone>/configuration/prior/`; `packages/d2b-core-controller/src/cleanup.rs`: removal scheduling and `PendingCleanup` condition tracking; `packages/d2b-contracts/src/generation_bundle.rs`: `ZoneBundle`/`BundleResource`/`BundleMetadata` **input** DTOs — MUST NOT include `managedBy` or `configurationGeneration` (both are persisted resource metadata set by core at activation, not bundle input fields); `ManagedBy` closed enum `{ Configuration, Controller, Api }` and `configurationGeneration: u64` live in `packages/d2b-core-controller/src/resource_store.rs` as persisted resource metadata fields; `nixos-modules/resources-network.nix` (emits bundle with `managedBy`/`configurationGeneration` absent; core sets both at activation per ADR046-network-004); `d2b.zones.<zone>.retainedGenerations` Nix/compiler-level Zone option (outside `Zone.spec`; default `3`, range `1..16`); `tests/unit/nix/cases/generation-cleanup-absent-network.nix`; `packages/d2b-contracts/tests/generation_bundle.rs`; `tests/host-integration/nix-generation-cleanup.nix` |
| Detailed design | **Core generation tracking** (`packages/d2b-core-controller/src/configuration.rs`): core maintains a monotone `configurationGeneration` counter per Zone in its durable state. On each bundle application it compares the incoming `contentHash` against the prior applied hash. If different, it increments `configurationGeneration`, sets `managedBy = "configuration"` and the new counter value on each resource in the bundle, and performs the resource diff (create/update/delete scheduling). The `managedBy` and `configurationGeneration` fields are absent from the Nix-emitted bundle and are set exclusively by core at activation time. **`managedBy` field and per-item name-conflict handling**: `ManagedBy` is a closed enum (`Configuration`, `Controller`, `Api`) persisted in resource metadata at `packages/d2b-core-controller/src/resource_store.rs`. It is NOT a field in `ZoneBundle`/`BundleResource` input DTOs; core sets it at activation. Controllers set `ManagedBy::Controller` when creating owned children (net-VM Guest, config Volume, guest-agent Process, mDNS Processes); exact controller identity/UID/generation are tracked in separate internal metadata, not embedded in the `managedBy` value. API-created resources carry `ManagedBy::Api` and persist until explicit delete with no bundle-driven lifecycle. Core's generation-transition logic only schedules bundle-driven Delete for `ManagedBy::Configuration` resources. **Per-item name-conflict handling**: when a bundle item's `(zone, name)` already exists with `managedBy ≠ "configuration"`, core skips that item and records it with `phase = Degraded, reason: name-conflict`; a `ResourceConflictSkipped` audit record is emitted for that item. All non-conflicting items in the bundle proceed normally (Provider-state contract). The existing resource is left completely untouched. The operator deletes the conflicting resource via the resource API; the next bundle application applies the item. **Removal scheduling**: on generation N+1 activation, core performs a set difference: `prev_configuration_managed - new_configuration_managed` = resources to delete. For each, it sets `metadata.deletionRequestedAt` in the resource store and emits a `ResourceDeletionScheduled` audit record. Normal finalizer-path Delete proceeds asynchronously. **`PendingCleanup` condition**: the Zone self resource carries a `PendingCleanup = True` condition while any `managedBy = Configuration` resource has `deletionRequestedAt` set and has not yet been atomically removed. Aggregate Zone `phase = Degraded` applies. The condition transitions to `False` and Zone phase returns to `Ready` when all scheduled deletions complete. **Prior generation bundle retention** (`cleanup.rs`): count-based (`d2b.zones.<zone>.retainedGenerations`, outside `Zone.spec`, default 3, range 1..16); no TTL. Core copies prior bundles to `/var/lib/d2b/zones/<zone>/configuration/prior/<contentHash>.json`. A generation is eligible for pruning when all configuration-managed resources from it have either been atomically removed or are present unchanged in a newer generation, AND the count would be exceeded. **`BundleActivated` audit record**: emitted at each generation transition with `contentHash`, `configurationGeneration`, `resourceCount`, and `providerSchemaDigests` map (digests from `type=provider` artifacts via `Provider.spec.artifactId`); no spec contents, CIDRs, or resource names appear in the record. Provider schema digests in the bundle are re-verified against installed Provider artifact digests at application time; a mismatch aborts application with a `BundleRejected` audit record. |
| Integration | ADR046-network-004 (emitter writes bundle format; core sets `managedBy`/`configurationGeneration` at activation) → ADR046-network-008 (runtime reads and applies). ADR046-network-005 (controller Delete path) is invoked by ADR046-network-008 removal scheduling for Network resources. Zone `PendingCleanup` condition and `Degraded` phase are read by CLI `d2b zone status`. |
| Data migration | None on v3 initial install (no prior generation state). Host upgrades from the pre-ADR45 v3 baseline perform a reset: core starts with `configurationGeneration = 1` and no prior bundle. All declared resources are treated as new Creates. |
| Validation | **nix-unit**: `tests/unit/nix/cases/generation-cleanup-absent-network.nix` — verifies that a Network resource present in generation N and absent from generation N+1 receives `deletionRequestedAt` and appears in the `PendingCleanup` condition; verifies that a controller-owned `Guest` (`managedBy = "controller"`) does NOT receive a direct bundle-driven Delete; verifies that a re-declared (identical spec) Network is NOT scheduled for Delete; verifies `retainedGenerations` default is 3. **Rust contract tests**: Two separate test files — (1) `packages/d2b-contracts/tests/generation_bundle.rs`: tests the **input** bundle DTO only: `ZoneBundle`/`BundleResource`/`BundleMetadata` JSON round-trip, `contentHash` stability across serialization, `providerSchemaDigests` presence, `managedBy` and `configurationGeneration` fields ABSENT from `BundleResource` input struct (verified by both compile-time type check: the fields must not exist on the `BundleResource` type, and runtime JSON serialization: the serialized object must not contain those keys). (2) `packages/d2b-core-controller/tests/resource_metadata.rs`: `ManagedBy` closed enum round-trip with `"configuration"`/`"controller"`/`"api"` values tested separately here since `ManagedBy` is persisted resource metadata in `resource_store.rs`, not a field of the input bundle DTO. **Controller integration tests**: async Delete triggered through finalizers for Network; mDNS Process child deleted before Network finalizer clears; bridge `DeleteBridge` broker call made exactly once during finalizer; controller waits for Deleted watch event (not a persistent phase=Deleted row) before proceeding. **Host integration**: `tests/host-integration/nix-generation-cleanup.nix` — runNixOSTest scenario: apply generation 1 with Network resource, then apply generation 2 with that Network absent; assert Zone enters `Degraded/PendingCleanup`; assert Network `phase = Degraded` with `NetworkDraining = True` and `deletionRequestedAt` set and `reason: configuration-generation-removed`; assert cleanup completes (single store transaction: Deleted REVISION event + row/index removal; dedup-guarded audit append follows committed transaction) and Zone returns to `Ready`; assert no controller-owned children deleted directly by core; assert prior bundle copied to `/var/lib/d2b/zones/<zone>/configuration/prior/` and retained until cleanup complete; assert bundle pruned when `retainedGenerations` exceeded and generation eligible. **INV-NET-LIFECYCLE-001**: core never schedules bundle-driven Delete for `managedBy ≠ "configuration"` resources; verified by static analysis of core's generation-transition diff function, which is bounded at compile time to iterate only the `configuration_managed_resources` set. **INV-NET-LIFECYCLE-002**: per-item name-conflict — when a bundle item collides with `managedBy = "controller"` or `"api"`, that item is recorded as `Degraded/name-conflict`; the existing resource is left untouched; non-conflicting items continue to activate; tested by `packages/d2b-core-controller/tests/configuration_name_conflict.rs` (three cases: collision with a controller-owned child, an API-created resource, and a same-name configuration resource from a prior generation that completed deletion; each case asserts non-conflicting items still activate). |
| Removal proof | Not applicable (this is a new capability). The `PendingCleanup` condition and zone cleanup audit path have no prior equivalent to remove. |

### ADR046-network-009

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-network-009` |
| Dependency/owner | D097 Host-global authority index; ADR046-network-001, ADR046-network-005; Provider/network-local and Core authority owners |
| Current source | Existing macvtap spawn path resolves `parentInterface` but has no cross-Zone authority admission or compatible-sharing contract |
| Reuse action | adapt |
| Destination | `packages/d2b-contracts/src/v3/network.rs` external-attachment sharing schema/status; `packages/d2b-core-controller/src/authority.rs` Core-derived physical-NIC identity and Host-global claim; Provider/network-local descriptor/reconcile/finalizer |
| Detailed design | Resolve operator-declared `parentInterface` against trusted Host inventory and derive an opaque `external-physical-nic/v1` digest; index `(Host, external-physical-nic, opaqueKeyDigest)` before any macvtap/VMM effect. `passthru`, `private`, and `vepa` are exclusive. `bridge` defaults exclusive and is multiplexed only under explicitly authored compatible policy. Use typed `external-physical-nic-conflict`; expose only bounded authority availability/holder-count/queue/arbitration/update-currency and conditions; keep digest, interface identity, and owner proof private. Parent/mode/policy update drains and releases the old claim before replacement; deletion closes macvtap/VMM ownership before releasing the claim; restart adopts exact owner proof and quarantines ambiguity. Primary reuse disposition: `adapt`. Preserved source-plan detail: adapt the existing private macvtap-FD spawn path; add authority admission before it. |
| Integration | Network validation and Core authority preflight gate runtime-cloud-hypervisor LaunchTicket/`SpawnRunner`; the finalizer and D091 update planner release in dependency order |
| Data migration | Full d2b 3.0 reset; no authority ledger import |
| Validation | Hermetic authority tests cover same-Zone and cross-Zone exclusive collisions, mixed-policy conflicts, non-bridge multiplex rejection, explicit compatible bridge multiplex admission, Core-derived key equality for two selectors resolving to one fake NIC, caller-supplied digest rejection, no-effect conflict, owner-proof adoption/ambiguity, disruptive update, and release-after-close ordering. Nix eval covers schema and declared cross-Zone conflicts; host integration covers create/update/delete with a fake macvtap parent and status/condition transitions without raw identity exposure. |
| Removal proof | None — authority admission is new; existing direct macvtap spawn becomes unreachable without a claim. |
