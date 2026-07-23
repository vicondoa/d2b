# ADR 0046 Provider dossier — Provider/network-local

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-provider-network-local` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 2 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-provider-network-local` crate, `d2b-host` IfName/nftables/bridge/routes modules |
| Depends on | `ADR-046-resources-network`, `ADR-046-resources-host-guest-process-user`, `ADR-046-resources-volume`, `ADR-046-provider-model-and-packaging`, `ADR-046-componentsession-and-bus`, `ADR-046-resource-reconciliation`, `ADR-046-nix-configuration`, `ADR-046-telemetry-audit-and-support`, `ADR-046-current-code-migration-map` |
| Supersedes | `nixos-modules/network.nix`, `nixos-modules/net.nix` |

---

## 1. Purpose and scope

This dossier is the exhaustive engineering specification for `Provider/network-local`.
It governs:

- the `Network` ResourceType: spec schema, status, IfName derivation, CIDR
  validation, attachment lifecycle, east-west isolation, DHCP/DNS, firewall,
  and mDNS;
- all child resources created per Network: one config Volume, one net-VM Guest,
  four Process resources (net-agent service, dnsmasq worker, mdns-reflector
  worker, mdns-dnsbridge worker), and one User resource;
- the `NetworkEffectPort` abstraction through which ALL host-kernel effects are
  driven — the provider crate has **no** broker dependency or socket;
- the controller's reconcile/observe/finalize loops, the ProviderStateSet, RBAC,
  d2b-bus, audit, OTEL, Nix configuration, and security invariants;
- migration from the v1 baseline, reuse of existing modules, work items, and
  the test structure required by policy.

The Provider baseline is pre-ADR 0045 (d2b 2.x) — no wave-N implementation crates
exist. Reuse is limited to `d2b-host` IfName/nftables/bridge/routes modules and
existing Nix module logic.

Sections reference `ADR-046-resources-network` (hereafter **NET**),
`ADR-046-resources-host-guest-process-user` (hereafter **PROC**), and
`ADR-046-resource-reconciliation` (hereafter **RECONCILE**) as normative sources.

---

## 2. Provider identity

| Field | Value |
| --- | --- |
| `providerRef` | `Provider/network-local` |
| Artifact ID | `provider-network-local` |
| Crate | `packages/d2b-provider-network-local/` |
| Controller binary | `d2b-provider-network-local-ctrl` |
| ResourceTypes implemented | `Network` |
| ResourceTypes consumed | `Host`, `Guest`, `Volume`, `Process`, `User`, `Zone` |
| Process Providers depended on | `Provider/system-minijail` |
| Data Providers depended on | `Provider/volume-local` |
| Broker dependency | **None** — all host-kernel effects via `NetworkEffectPort` |

**D089 spec extension contract:** `Provider/network-local` carries any
implementation-only Network desired configuration only in `spec.provider.settings`
under `network-local.d2b.io/Network/spec`; that schema is registered/signed in
the manifest, deny-unknown, bounded, versioned, and validated against
`spec.providerRef` at Nix build and API admission. Base Network fields stay at
`spec.*`; shared semantics are promoted to the Network base and never placed in
`spec.provider`. The Provider implements the exact base spec/status schema
version/fingerprint, accepts the canonical minimal valid base Spec, and rejects
an unsupported optional base capability only through its signed capability matrix
plus provider-neutral `unsupported-capability`. `spec.provider` aligns with
`status.provider` for `Provider/network-local`.

The provider crate does **not** depend on `d2bd`, `d2b-priv-broker`, any broker
socket or wire type, or any Provider's implementation crate. All host-kernel effects
are driven through the injected `NetworkEffectPort` async trait, which is declared in
`d2b-contracts` (the neutral provider contract crate); the core adapter (not the
provider crate) implements that trait, maps it to closed broker wire operations, and
emits the corresponding audit records.

---

## 3. Crate layout

```
packages/d2b-provider-network-local/
  README.md              # covers all 7 required topics (identity → standalone-repo path)
  src/
    main.rs              # controller binary entry point; dependency injection
    controller.rs        # reconcile/observe/finalize handlers
    validate.rs          # spec validation (CIDR, attachment, IfName constraints)
    config_volume.rs     # Volume resource creation and content rendering
    guest.rs             # net-VM Guest resource management
    process_specs.rs     # canonical Process resource specs (agent, dnsmasq, mDNS)
    user.rs              # User resource precondition check
    ifname.rs            # re-exports d2b_host::ifname::derive_ifname
    status.rs            # status and condition helpers
    audit.rs             # audit redaction helpers
    error.rs             # typed ReconcileError
    #[cfg(test)] units inside each source file
  tests/
    schema_roundtrip.rs  # NetworkSpec JSON serialize/deserialize
    ifname_derive.rs     # IfName derivation determinism
    cidr_overlap.rs      # CIDR validation matrix
    controller_state.rs  # reconcile state-machine with deterministic clock
    conformance.rs       # provider-toolkit conformance suite
    fault_injection.rs   # NetworkEffectPort (from d2b-contracts) error injection
  integration/
    host_fabric.rs       # bridge/tap/nftables lifecycle (container-based)
    guest_lifecycle.rs   # net-VM Guest create/delete
    agent_reload.rs      # agent service reload path
    mdns_reflector.rs    # mDNS reflector Process lifecycle
    delete_sequence.rs   # full delete ordering
```

`src/`, `tests/`, and `integration/` each contain at least one tracked file.
The root `README.md` covers all required topics.  The workspace policy test
(`make test-policy` / `xtask workspace-policy`) enforces these four paths.
A nested `integration/README.md` is optional and not required by policy.

Crate dependencies:

| Crate | Role |
| --- | --- |
| `d2b-contracts` | `NetworkSpec`, `NetworkStatus`, IfName, Network-related DTOs; **`NetworkEffectPort` trait**; opaque `FabricHandle`/`AttachmentHandle` types |
| `d2b-controller-toolkit` | async reconcile loop, `ResourceClient`, `ResourceMutationBatch` |
| `d2b-host` | `derive_ifname`, `nftables`, bridge-port, route-preflight, sysctl modules |
| `d2b-provider-toolkit` | Provider registration, conformance kit, fake-core/store/bus/effect |

No broker crate appears in `[dependencies]` or `[dev-dependencies]`.

---

## 4. Provider resource spec

The `Provider/network-local` resource is declared in Nix:

```yaml
# Generated from d2b.zones.<zone>.providers.network-local (Nix option)
apiVersion: resources.d2b.io/v3
type: Provider
metadata:
  name: network-local
  zone: dev
spec:
  artifactId: provider-network-local    # ^[a-z][a-z0-9-]*$; plain bounded ID
  config:
    controllerExecutionRef: Host/host-system
    # Root config validated against provider-network-local/Network.schema.json.
    # All config fields are Provider-specific; no raw broker parameters or
    # kernel interface names appear in config.
```

`artifactId` is a plain bounded ID matching `^[a-z][a-z0-9-]*$`.  It is **not** a
path, Nix store path, or ResourceRef.  The artifact catalog entry (§22) maps this ID
to the Nix derivation.

`config.controllerExecutionRef` is the `Host` ResourceRef on which the controller
Process runs.  The framework creates the controller Process resource from this config
field.

### 4.1 Controller Process resource

The framework creates the following Process resource when `Provider/network-local`
is installed:

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: network-local-ctrl
  zone: dev
  ownerRef: Provider/network-local
spec:
  providerRef: Provider/system-minijail
  executionRef: Host/host-system            # from Provider.spec.config.controllerExecutionRef
  domain: system
  processClass: controller
  template: controller-main
  sandbox:
    namespaceClasses: []                    # no additional namespace isolation; inherits host
    capabilityClasses: []                   # no ambient capabilities; effects via NetworkEffectPort
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
  budget:
    memory:
      request: "32Mi"
      limit: "128Mi"
    pids:
      limit: 64
    fds:
      limit: 512
  mounts: []                             # no Provider state Volume under D087
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "120s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "0s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
    class: ready-condition
  healthCheck:
    enabled: true
    interval: "30s"
    timeout: "5s"
    failureThreshold: 3
    class: provider-defined
  adoptionPolicy: adopt-on-restart
  drainTimeout: "30s"
```

The controller process has **no ambient host capabilities**.  All host-kernel bridge,
tap, nftables, sysctl, route, NM-unmanaged, and hosts-file effects are driven through
the injected `NetworkEffectPort` (§5).  No `kvm`, `net-admin`, or other
`capabilityClass` appears in this spec.

The required Host capabilities for the controller's execution environment are:
`pidfd`, `cgroup-v2`.  The `kvm` and `user-namespace` capabilities are **not**
required for the network controller; `kvm` belongs to
`Provider/runtime-cloud-hypervisor`.

---

## 5. NetworkEffectPort — broker abstraction layer

### 5.1 Purpose

The reconcile context (RECONCILE §Reconcile context) contains no database handle,
direct broker socket, reusable credential, or raw route table.  The network-local
controller drives all host-kernel mutations through an injected async
`NetworkEffectPort` trait object declared in `d2b-contracts`.  The core adapter
(in `d2b-core`, not the provider crate) implements this trait, maps opaque resource
UIDs and semantic intent structs to closed broker wire operations, and emits
broker-level audit records.

The provider crate sees the declared `NetworkSpec` in full — including `lanCidr`,
`uplinkCidr`, and other operator-declared IP policy fields — because those are the
desired spec inputs that drive the controller's reconcile logic.  What the provider
crate **never** sees through the `NetworkEffectPort` interface are runtime-observed
or kernel-derived values: kernel interface names, observed host addresses, DHCP MAC
assignments, tap FDs, or route-table text.  Opaque handle types (`FabricHandle`,
`AttachmentHandle`) carry internal identity material that is never exposed as a
printable string; they implement custom redacted `Debug` and are not `Clone` or
`Copy`.

### 5.2 Trait definition (Rust pseudocode)

```rust
/// Injected port for all host-kernel fabric and firewall effects.
/// Declared in d2b-contracts; implemented by the core adapter.
/// All methods are async and must not hold a redb transaction across any await.
/// Blocking kernel effects use explicit bounded adapters inside the core impl.
/// EffectError is a closed typed enum; no String-payload error variant exists.
#[async_trait]
pub trait NetworkEffectPort: Send + Sync {
    // ── Fabric (bridges) ─────────────────────────────────────────────────────
    /// Create or ensure a host kernel bridge fabric for a Network.
    /// The core adapter derives IfName internally from the networkName in the
    /// FabricIntent; the provider never receives the raw IfName.
    async fn create_fabric(
        &self,
        network_uid: &Uid,
        intent: &FabricIntent,
    ) -> Result<FabricHandle, EffectError>;

    /// Delete the host kernel bridge for a Network.  Idempotent on absence.
    async fn delete_fabric(
        &self,
        handle: &FabricHandle,
    ) -> Result<(), EffectError>;

    // ── Attachment taps ───────────────────────────────────────────────────────
    /// Declare a tap attachment intent for a specific Guest on a Network.
    /// Returns an opaque AttachmentHandle; the core adapter creates or adopts
    /// the tap and bridge-port configuration.  The tap IfName is never exposed.
    async fn declare_attachment_tap(
        &self,
        network_uid: &Uid,
        attachment_uid: &Uid,
        intent: &TapIntent,
    ) -> Result<AttachmentHandle, EffectError>;

    /// Revoke a previously declared tap attachment.  Removes the tap device.
    async fn revoke_attachment_tap(
        &self,
        handle: &AttachmentHandle,
    ) -> Result<(), EffectError>;

    /// Set the isolation flag on a tap's bridge port.
    async fn set_attachment_isolation(
        &self,
        handle: &AttachmentHandle,
        isolated: bool,
    ) -> Result<(), EffectError>;

    // ── Firewall ──────────────────────────────────────────────────────────────
    /// Apply or replace the inet-d2b table entries owned by this Network.
    /// Returns the SHA-256 digest of the applied ruleset (opaque; used for drift
    /// detection in status).  No rule text is stored in status or audit.
    async fn apply_host_firewall(
        &self,
        network_uid: &Uid,
        intent: &FirewallIntent,
    ) -> Result<FirewallDigest, EffectError>;

    /// Remove all inet-d2b rules owned by this Network (deletion path).
    async fn remove_host_firewall(
        &self,
        network_uid: &Uid,
    ) -> Result<(), EffectError>;

    // ── Routes ────────────────────────────────────────────────────────────────
    async fn apply_host_routes(
        &self,
        network_uid: &Uid,
        intent: &RouteIntent,
    ) -> Result<(), EffectError>;

    async fn remove_host_routes(
        &self,
        network_uid: &Uid,
    ) -> Result<(), EffectError>;

    // ── Sysctls ───────────────────────────────────────────────────────────────
    /// Re-apply per-bridge IPv6 suppression sysctls (defense-in-depth).
    async fn apply_host_sysctls(
        &self,
        network_uid: &Uid,
        intent: &SysctlIntent,
    ) -> Result<(), EffectError>;

    // ── NetworkManager ────────────────────────────────────────────────────────
    async fn apply_nm_unmanaged(
        &self,
        pattern: &NmUnmanagedPattern,
    ) -> Result<(), EffectError>;

    // ── /etc/hosts ────────────────────────────────────────────────────────────
    async fn update_hosts_file(
        &self,
        network_uid: &Uid,
        intent: &HostsIntent,
    ) -> Result<(), EffectError>;

    // ── DHCP pre-seed ─────────────────────────────────────────────────────────
    async fn seed_dhcp_reservations(
        &self,
        network_uid: &Uid,
        intent: &DhcpSeedIntent,
    ) -> Result<(), EffectError>;

    // ── Read-back (observe/drift detection) ───────────────────────────────────
    async fn read_firewall_digest(
        &self,
        network_uid: &Uid,
    ) -> Result<Option<FirewallDigest>, EffectError>;

    async fn read_sysctl_state(
        &self,
        network_uid: &Uid,
    ) -> Result<SysctlState, EffectError>;

    async fn read_attachment_isolation(
        &self,
        handle: &AttachmentHandle,
    ) -> Result<bool, EffectError>;
}
```

### 5.3 Opaque intent structs and handle types

Intent structs are declared in `d2b-contracts` and contain only semantic data.  They
never contain raw kernel interface names, IP strings derived at runtime, or MAC
address strings.  The core adapter resolves all opaque UIDs → kernel interface names
internally using the IfName derivation algorithm (§7).

Opaque handle types implement a custom redacted `Debug` that prints only a stable
type tag and no sensitive content.  They are not `Clone`, not `Copy`, and cannot be
serialized to JSON or transmitted over the resource API wire.

| Type | Semantic content | Constraints |
| --- | --- | --- |
| `FabricIntent` | `mtu`, `stp_disabled`, `multicast_snooping_disabled`, `ipv6_suppress` | All fields from declared spec |
| `TapIntent` | `attachment_index`, `neigh_suppress` | Index from declared spec |
| `AttachmentHandle` | opaque seal over internal `(network_uid, attachment_uid)` | Redacted Debug; not Clone/Copy/Serialize |
| `FabricHandle` | opaque seal over internal `network_uid` | Redacted Debug; not Clone/Copy/Serialize |
| `FirewallIntent` | `rules: Vec<FirewallRule>` (rules reference attachment handles, not IfNames) | No raw IfNames |
| `FirewallDigest` | opaque `[u8; 32]` SHA-256 | Stored in status for drift comparison only |
| `RouteIntent` | `destinations: Vec<IpNet>`, `via: Option<RouteViaHint>` | CIDRs from declared spec |
| `SysctlIntent` | `ipv6_suppress: bool` | — |
| `NmUnmanagedPattern` | `prefix_pattern: &'static str` (the `"d2b-*"` glob) | Compile-time constant; no runtime string |
| `HostsIntent` | `entries: Vec<HostEntry>` with resource names only | No raw IPs or MACs |
| `DhcpSeedIntent` | `reservations: Vec<DhcpReservation>` with opaque attachment refs | No raw MACs in provider surface |
| `EffectError` | closed typed enum; no String-payload variant | `#[non_exhaustive]` internally; stable codes to provider |

### 5.4 Broker op mapping (core adapter, not provider)

The core adapter maps NetworkEffectPort calls to the following broker wire operations.
This table is informational for the core adapter authors; it does not appear in the
provider crate.

| NetworkEffectPort method | Broker wire op | Migration source |
| --- | --- | --- |
| `create_fabric` | `CreateBridge` (new v3 op) | none (v3 new) |
| `delete_fabric` | `DeleteBridge` (new v3 op) | none (v3 new) |
| `declare_attachment_tap` | `CreatePersistentTap` + `SetBridgePortFlags` | `d2b-priv-broker/src/runtime.rs` tap ops |
| `revoke_attachment_tap` | `DeleteTap` | existing v3 baseline |
| `set_attachment_isolation` | `SetBridgePortFlags` | `d2b-host/src/bridge_port.rs` |
| `apply_host_firewall` | `ApplyNftables` | `d2b_contracts::broker_wire::ApplyNftablesRequest` |
| `remove_host_firewall` | `ApplyNftables` (empty set) | same |
| `apply_host_routes` | `ApplyRoute` | `d2b_contracts::broker_wire::ApplyRouteRequest` |
| `remove_host_routes` | `ApplyRoute` (empty) | same |
| `apply_host_sysctls` | `ApplySysctl` | `d2b_contracts::broker_wire::ApplySysctlRequest` |
| `apply_nm_unmanaged` | `ApplyNmUnmanaged` | `d2b_contracts::broker_wire::ApplyNmUnmanagedRequest` |
| `update_hosts_file` | `UpdateHostsFile` | `d2b_contracts::broker_wire::UpdateHostsFileRequest` |
| `seed_dhcp_reservations` | `SeedDnsmasqLease` | `d2b_contracts::broker_wire::SeedDnsmasqLeaseRequest` |
| `read_firewall_digest` | `ReadNftablesDigest` (new v3 op) | `d2b-host/src/nftables.rs:hash_inet_d2b_table` |
| `read_sysctl_state` | `ReadSysctlState` (new v3 op) | `d2b-host/src/netlink.rs` |
| `read_attachment_isolation` | `ReadBridgePortFlags` (new v3 op) | `d2b-host/src/bridge_port.rs` |

### 5.5 Runtime Provider attachment FD path

When `Provider/runtime-cloud-hypervisor` reconciles the net-VM Guest, it needs the
tap file descriptors to configure `--net fd=<fd>` arguments.  The runtime Provider
does **not** call the NetworkEffectPort directly.  The net-VM Guest has
`ownerRef: Network/work-net`; core uses the owner/dependency graph to find the
Network, reads the internally-stored `AttachmentHandle` records (never exposed in
any public spec or status field), and supplies the actual tap FDs to the runtime via
the LaunchTicket mechanism.

No attachment identity, tap FD, or kernel interface name flows through the Guest
spec, Guest status, or any other public resource surface.  The binding is purely
private to the core dependency resolver.

---

## 6. Network ResourceType spec

### 6.1 Full spec schema

```yaml
apiVersion: resources.d2b.io/v3
type: Network
metadata:
  name: work-net                       # ^[a-z][a-z0-9-]*$; max 63; Zone-local
  zone: dev
spec:
  # ── Identity ────────────────────────────────────────────────────────────────
  networkName: work-net                # defaults to metadata.name; used for IfName derivation
  netVmNameOverride: null              # optional; overrides auto-derived net-VM Guest name
  netVmSystemArtifactId: net-vm-base   # REQUIRED; ^[a-z][a-z0-9-]*$; type must be nixos-system
                                       # in d2b.artifacts catalog; checked at build time

  # ── CIDR ────────────────────────────────────────────────────────────────────
  lanCidr: "10.20.0.0/24"             # exactly /24; base ends in .0; RFC1918 recommended
  uplinkCidr: "192.0.2.0/30"          # exactly /30; host .1; net-VM .2

  # ── MTU and MSS ─────────────────────────────────────────────────────────────
  mtu: 1500                            # 576..9000; applied to both bridges
  mssClamp: false                      # true adds TCP MSS clamp rule in net-VM

  # ── Attachments (workload Guests) ───────────────────────────────────────────
  attachments:
    - executionRef: Guest/corp-vm
      index: 10                        # 2..250 inclusive; unique within Network
    - executionRef: Guest/personal-vm
      index: 11

  # ── External physical attachment ────────────────────────────────────────────
  externalAttachment: null             # null or ExternalAttachmentSpec
  # ExternalAttachmentSpec:
  #   hostInterface: eth0              # host physical NIC; macvtap created by runtime
  #   mode: bridge                     # bridge|private|vepa|passthrough
  #   mac: null                        # null → derived; static or null
  #   ipv4:
  #     mode: dhcp                     # dhcp|static
  #     address: null                  # static address/prefix
  #     gateway: null                  # static gateway
  #   egress:
  #     masquerade: false
  #     allowedCidrs: []               # egress CIDRs for forward chain
  #   portForwards: []
  #   # PortForwardSpec: {protocol, externalPort, sourceCidrs, targetIndex, targetPort}

  # ── Isolation ────────────────────────────────────────────────────────────────
  isolation:
    allowEastWest: false               # default false; workload taps set to Isolated=true

  # ── DNS ──────────────────────────────────────────────────────────────────────
  dns:
    forwarders: []                     # upstream DNS IPs passed to dnsmasq
    domain: null                       # optional local search domain
    searchDomains: []

  # ── Routing ──────────────────────────────────────────────────────────────────
  routing:
    hostBlocklist: []                  # additive; controller unions RFC1918+LL defaults

  # ── mDNS ─────────────────────────────────────────────────────────────────────
  mdns:
    enable: false                      # create mDNS reflector Process when true
    dnsmasqLocal: false                # create local DNS bridge Process when true
```

### 6.2 Field validation (validateSpec)

| Field | Constraint | Error code |
| --- | --- | --- |
| `networkName` | `^[a-z][a-z0-9-]*$` | `network-name-invalid` |
| `netVmSystemArtifactId` | Required; `^[a-z][a-z0-9-]*$`; artifact must be type `nixos-system` | `net-vm-artifact-missing`, `net-vm-artifact-type-mismatch` |
| `lanCidr` | Exactly `/24`; base ends in `.0` | `network-cidr-invalid` |
| `uplinkCidr` | Exactly `/30` | `network-cidr-invalid` |
| `lanCidr` ↔ `uplinkCidr` | No overlap | `network-cidr-conflict` |
| `lanCidr`, `uplinkCidr` ↔ peers | No overlap with any other Network in Zone | `network-cidr-conflict` |
| `attachments[].index` | 2..250; unique within Network | `attachment-index-invalid`, `attachment-index-duplicate` |
| `netVmNameOverride` | If set: `^[a-z][a-z0-9-]*$`; not `launcher`; not `sys-*` | `net-vm-name-reserved` |
| `externalAttachment.egress.allowedCidrs` | No overlap with any Network CIDR | `network-cidr-conflict` |
| IfName collision | All derived IfNames unique across Hosts | `ifname-collision` |

IfName collision is terminal: the controller sets `ReconcileError{reason: ifname-collision}`
and halts reconciliation until the operator adjusts `networkName`.

### 6.3 Network status schema

D088 status layering is normative: the controller populates the Network
ResourceType-common `status.resource` with network readiness, fabric readiness,
and attachment readiness in the same provider-neutral shape read by all generic
Network consumers. Local bridge/firewall/config observations, including bounded
firewall and config-volume digests, live only in `status.provider.details` with
`providerRef: Provider/network-local`, qualified `schemaId`
(`network-local.d2b.io/Network/status`), `schemaVersion`, and
`observedProviderGeneration`. Controller status writes include all present layers
atomically in one status mutation; shared fields are never duplicated into
`status.provider`, and the strict, ≤32 KiB, redacted extension schema is
registered and signed in the Provider manifest.

```yaml
status:
  observedGeneration: 1
  phase: Ready          # Pending|Ready|Degraded|Failed|Deleted|Unknown
  conditions: []
  lastReconciledAt: "2026-07-22T00:00:00Z"
  resource:
    # Per-workload attachment phases — opaque phase only; no raw IfName or IP
    attachments:
      - executionRef: Guest/corp-vm
        phase: Ready                          # Pending|Ready|Degraded|Absent
    fabricReady: true                         # bridges created and Ready
  provider:
    providerRef: Provider/network-local
    schemaId: network-local.d2b.io/Network/status
    schemaVersion: 1.0.0
    observedProviderGeneration: 1
    details:
      firewallDigest: "<hex-sha256>"          # SHA-256 of applied inet-d2b table; drift reference
      configVolumeRevisionDigest: "<hex>"     # digest of last committed config Volume content
```

**No** raw `ifName`, `bridgeName`, `tapIfName`, `hostUplinkIp`, `netVmUplinkIp`,
`netVmLanIp`, MAC address, attachment handle, FD reference, or kernel path appears
in any public status field, audit record, metric label, or OTEL span attribute.
Attachment handles and FDs are private to the core dependency resolver; they are
never stored in or read from the public resource store.

### 6.4 Conditions

| Condition type | Ready=True when | Reason codes |
| --- | --- | --- |
| `ControllerReady` | Controller Process is Ready | `controller-unavailable` |
| `FabricReady` | Both host bridges exist and sysctls applied | `bridge-create-error`, `sysctl-error`, `ifname-collision` |
| `FirewallReady` | Host nftables `inet d2b` rules applied; digest matches | `nftables-error`, `nftables-drift` |
| `NmUnmanagedReady` | `00-d2b-unmanaged.conf` written | `nm-unmanaged-error` |
| `HostRoutesReady` | Host route to LAN CIDR via uplink bridge applied | `route-error` |
| `ConfigVolumeReady` | Config Volume backing Ready; content written | `config-volume-error`, `volume-backing-error`, `attachment-not-ready` |
| `NetVmReady` | net-VM Guest in Ready phase | `net-vm-pending`, `net-vm-failed`, `net-vm-degraded`, `agent-restart` |
| `DhcpReady` | Guest-agent reports `dnsmasq-bound` readiness predicate | `agent-not-ready`, `dnsmasq-not-bound` |
| `FirewallReady` (guest) | Guest-agent reports `nft-applied` readiness predicate | `nft-not-applied` |
| `DnsReady` | Guest-agent reports `routes-applied` and dnsmasq DNS socket bound | `dns-not-ready` |
| `CidrConflict` | No CIDR overlap detected | `network-cidr-conflict` |
| `ExternalAttachmentReady` | macvtap interface in net VM Ready (if externalAttachment≠null) | `macvtap-not-ready` |
| `MdnsReady` | mDNS Process(es) in Ready phase (if mdns.enable) | `mdns-process-not-ready` |

---

## 7. IfName derivation

IfNames are **internal** to the core adapter.  They are derived deterministically
from `(networkName, role, optional guestName)` using the algorithm in
`packages/d2b-host/src/ifname.rs:derive_ifname`:

- FNV-1a 64-bit hash of the input tuple;
- Crockford base32 encoding (no I/L/O/U);
- truncated to 8 characters;
- prefixed as:

| Role | Prefix | Total max length |
| --- | --- | --- |
| LAN bridge | `d2b-b` | 14 chars ≤ IFNAMSIZ-1 |
| Uplink bridge | `d2b-b` | 14 chars |
| Net-VM LAN tap | `d2b-t` | 14 chars |
| Net-VM uplink tap | `d2b-t` | 14 chars |
| Workload Guest tap | `d2b-t` | 14 chars |
| External macvtap | `d2b-t` | 14 chars |

The 15-byte IFNAMSIZ-1 constraint is guaranteed by construction.  Collision
detection (`detect_collisions`) re-runs at every reconcile cycle.  A collision is
terminal (§6.2).

IfNames **never** appear in:
- `Network.spec` fields (any kind);
- `Network.status` fields (any kind);
- `Guest.spec.provider.settings`;
- audit records;
- OTEL span attributes or metric labels;
- any user-facing diagnostic beyond the bounded diagnostic API.

---

## 8. Net-VM Guest resource

The Network controller creates and owns exactly one net-VM Guest per Network.

```yaml
apiVersion: resources.d2b.io/v3
type: Guest
metadata:
  name: net-work-net                    # or spec.netVmNameOverride
  zone: dev
  ownerRef: Network/work-net            # owner relationship; core uses this to bind tap FDs
spec:
  providerRef: Provider/runtime-cloud-hypervisor
  defaultDomain: system
  allowedDomains: [system]
  budget:
    memory: { request: "256Mi", limit: "512Mi" }
    vcpus: 1
  systemArtifactId: net-vm-base         # from Network.spec.netVmSystemArtifactId
                                        # plain bounded ID ^[a-z][a-z0-9-]*$ — NOT a path
  # spec.provider.settings carries only runtime-cloud-hypervisor desired values.
  # Tap FDs are resolved privately by core from the Network→Guest owner relationship
  # and are supplied to the runtime via LaunchTicket.
  # No attachment identity, handle, IfName, IP, or MAC appears here.
  provider:
    schemaId: runtime-cloud-hypervisor.d2b.io/Guest/spec
    schemaVersion: 1.0.0
    settings:
      vsockCid: 1024                  # assigned from the Network's CIDR allocation
  # When spec.externalAttachment is non-null, the controller adds declared-spec
  # parameters (operator-specified, not kernel-observed) for the macvtap under
  # spec.provider.settings:
  # provider:
  #   schemaId: runtime-cloud-hypervisor.d2b.io/Guest/spec
  #   schemaVersion: 1.0.0
  #   settings:
  #     vsockCid: 1024
  #     externalHostInterface: eth0       # declared by operator in Network.spec.externalAttachment
  #     externalMode: bridge              # same
```

`systemArtifactId` is a plain bounded ID (`^[a-z][a-z0-9-]*$`); it is **not** a
Nix store path or a `nixos-system/...` path.  The artifact catalog entry
(`d2b.artifacts.net-vm-base`) maps this ID to the nixos-system derivation.

The nixos-system artifact contains the **generic** net-VM OS: guest-agent binary,
kernel, base NixOS services, and systemd-networkd NIC bootstrap with the
`lib.mkForce` override on `10-eth-dhcp` (INV-NET-001).  It does NOT encode
per-Network DHCP reservations, nftables rules, or routing policy.  All
per-Network desired state flows through the config Volume (§9).

Mutations changing only DHCP/DNS, firewall, or attachment configuration update the
config Volume and trigger a guest-agent `Reload()` call; a Guest switch or restart
is NOT required.  NIC topology changes (attachment index add/remove, external
attachment) additionally require a Guest spec update.

---

## 9. Config Volume resource

The Network controller creates one config Volume per Network.  `Provider/volume-local`
is the sole reconciler of all Volume resources; the network-local controller creates
Volume resource objects (with `ownerRef: Network/<name>`) but does not implement or
reconcile the Volume ResourceType.

```yaml
apiVersion: resources.d2b.io/v3
type: Volume
metadata:
  name: net-work-net-config             # net-<networkName>-config
  zone: dev
  ownerRef: Network/work-net
spec:
  providerRef: Provider/volume-local
  kind: ephemeral                       # tmpfs-backed; boot-scoped; no persistent backing
  source:
    executionRef: Host/host-system      # backing tmpfs on this Host
    settings:
      kind: tmpfs                       # memory-backed; charged to Host memory budget
  quota:
    maxBytes: 4194304                   # 4 MiB; tmpfs size= option; kernel-enforced
    maxInodes: 128                      # bounded; tmpfs nr_inodes= option
    enforcement: hard                   # required for tmpfs; kernel enforces
  layout:
    - path: ""                          # Volume root directory
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
    - path: "dnsmasq.conf"
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
    - path: "nftables.rules"
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
    - path: "routing.conf"
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
    - path: "attachments.json"
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
      path: ""                          # root of the Volume subtree
      rights: [read, traverse]          # minimum for agent to read config files
  attachments: []                       # initially empty; Guest attachment added in Phase 2
```

### 9.1 Volume content

Content is **bounded structured network configuration** rendered from `Network.spec`.
No workload VM names, host paths, raw IfNames, raw IP addresses of individual
workload VMs, or other per-workload identifiers appear in paths or file entries.
Content that appears in files:

| File | Content |
| --- | --- |
| `dnsmasq.conf` | DHCP reservations (MAC→index mapping, no external hostnames), forwarders, domain, static pools; `bind-interfaces=true`; `dhcp-ignore-names=true`; dnsmasq system user; hardened confinement settings from `nixos-modules/net.nix` lines 363–441 |
| `nftables.rules` | Complete `inet` filter/nat/ip6 rulesets for the net VM; all semantics from `nixos-modules/net.nix` lines 168–296; no raw tap IfNames (interface indices used via `eth0`/`eth1` NIC naming inside guest) |
| `routing.conf` | Static routes for external attachment egress CIDRs; no raw IfNames |
| `attachments.json` | Attachment index → MAC mapping; no Guest resource names or workload IPs |

No raw kernel interface name, host bridge name, IP address, or hostname appears in
any Volume file in a form that constitutes a network configuration secret.

### 9.2 Writes through Volume service

The controller writes all content through the typed Volume write service
(`Provider/volume-local`'s write API).  It does **not** directly manipulate the
underlying filesystem path.

### 9.3 Two-phase provisioning

**Phase 1 — backing ready**: create Volume with `attachments: []`.  Backing tmpfs
becomes Ready.  Controller writes initial config content via Volume service.

**Phase 2 — Guest attachment**: after net-VM Guest reaches Ready, update Volume to
add:
```yaml
attachments:
  - executionRef: Guest/net-work-net
    transport: virtiofs
    view: guest-readonly
    access: read-only
    mountPath: "/run/d2b/net-config"
    settings:
      posixAcl: false
      xattr: false
      cache: auto
      inodeFileHandles: never
      threadPoolSize: null
      socketGroup: null
```

Only after the attachment reaches Ready may the guest-agent Process be created.

---

## 10. User resource — net-local-controller

The `User/net-local-controller` resource is **declared in Nix** (§22) and
reconciled to Ready by `Provider/system-core` via NSS lookup.  The network-local
controller does **not** create this User resource dynamically; it waits for it to
be Ready as a reconcile precondition.

```yaml
apiVersion: resources.d2b.io/v3
type: User
metadata:
  name: net-local-controller            # ^[a-z][a-z0-9-]*$
  zone: dev
  ownerRef: Provider/network-local      # owner is the Provider; set in Nix
spec:
  osUsername: net-local-controller      # OS username for NSS getpwnam
  # spec contains only: osUsername, displayName (optional), groups (optional)
  # NO managedBy field — that is metadata.managedBy set by core, not spec
```

`spec.managedBy` does **not** exist in the User spec.  The `ownerRef` is in
`metadata`, not `spec`.  The Nix module pre-provisions the OS account (fixed UID/GID)
in Host prerequisites and in the generic net-VM nixos-system artifact so virtiofs
ACLs are consistent on both sides.  `Provider/system-core` reconciles the User
resource to Ready via NSS `getpwnam(net-local-controller)`.

Numeric UID/GID never enter any ResourceSpec field, authz check, or audit record.
`User.status` MAY carry diagnostic `uid`/`gid` values from NSS lookup; those are
informational only and are never authorization inputs.

The controller waits for `User/net-local-controller.status.phase == Ready` before
creating any config Volume.  This is a reconcile precondition enforced by checking
the `DependenciesReady` condition.

---

## 11. Process resources

The network-local controller creates four Process resources per Network.  All four
are owned by `Network/<networkName>` and run on `Guest/<netVmName>`.

### 11.1 Net-agent service (Process/net-\<networkName\>-agent)

The net-agent is a **`service`** (not a `worker`).  It serves an internal
ComponentSession method `NetworkAgentService` over a Noise-KK vsock.  It applies
nftables rules and ip routes inside the net VM on startup and on `Reload()` calls.
It does **not** supervise or spawn dnsmasq; dnsmasq is a separate Process (§11.2).

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: net-work-net-agent
  zone: dev
  ownerRef: Network/work-net
spec:
  providerRef: Provider/system-minijail
  executionRef: Guest/net-work-net      # runs INSIDE the net-VM Guest
  domain: system
  processClass: service                 # serves typed ComponentSession methods
  template: net-vm-agent
  sandbox:
    namespaceClasses: []                # empty: inherit all Guest namespaces (incl. netns)
    capabilityClasses: [network-admin, network-raw]
    # network-admin → CAP_NET_ADMIN: required for nft ruleset load and ip route
    # network-raw   → CAP_NET_RAW:   required for raw socket operations
    # Both are effective only within the inherited Guest network namespace; no
    # host capability is conferred (INV-NET-009).
    # network-bind is NOT required here; dnsmasq is a separate Process (§11.2).
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
  budget:
    memory: { request: "8Mi", limit: "32Mi" }
    pids: { limit: 16 }
    fds: { limit: 64 }
  mounts:
    - volumeRef: Volume/net-work-net-config
      view: guest-readonly
      mountPath: "/run/d2b/net-config"
      access: read-only
      required: true
  networkUsage:
    networkRef: Network/work-net
    ports: []
    allowEgress: true
  endpoints:
    - name: agent-service
      transport: vsock
      purpose: d2b.network.v3.agent/v1
      # ComponentSession service endpoint; accessible to the host controller only
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "2s"
    timeout: "30s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
    # Provider-defined: agent reports typed readiness predicates via its
    # ComponentSession service interface (see §11.1.1)
  healthCheck:
    enabled: true
    interval: "30s"
    timeout: "5s"
    failureThreshold: 3
    class: provider-defined
  adoptionPolicy: adopt-on-restart
  drainTimeout: "10s"
```

#### 11.1.1 NetworkAgentService ComponentSession interface

The agent serves one Noise-KK vsock ComponentSession service
`d2b.network.v3.agent/v1`:

```text
service NetworkAgentService {
  // Apply nftables rules and ip routes from the config Volume.
  // Called by the host controller after writing new config content.
  // config_digest: SHA-256 of the config Volume content at write time.
  // Returns: applied predicate set and any error codes.
  Reload(config_digest: ConfigDigest) -> ReloadResult

  // Return current readiness predicates.
  ReadinessQuery() -> AgentReadiness
}

message AgentReadiness {
  nft_applied: bool
  routes_applied: bool
}

message ReloadResult {
  applied_digest: ConfigDigest
  predicates: AgentReadiness
  errors: [AgentError]     # bounded typed error codes; no raw kernel output
}
```

The host controller calls `Reload()` after each successful config Volume write.
The agent does NOT watch the Volume directly or use any Volume watch interface.

The agent's sole responsibilities:
1. On startup: read `/run/d2b/net-config/nftables.rules` and apply via `nft -f`;
   read `/run/d2b/net-config/routing.conf` and apply via `ip route`;
   report `nft-applied` and `routes-applied` readiness predicates.
2. On `Reload(digest)`: atomically re-read and re-apply nftables and routes;
   return `ReloadResult`.

The agent does **not**: supervise dnsmasq; watch any Volume interface; fork or exec
any child process; expose any bus authority beyond the single vsock service endpoint;
or perform any Resource API calls.

### 11.2 Dnsmasq worker (Process/net-\<networkName\>-dnsmasq)

dnsmasq runs as a separate owned `worker` Process.  It reads its config from the
Volume mount at startup.  Workers have **no** bus authority, no dependency/resource
API, and no child-spawning.

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: net-work-net-dnsmasq
  zone: dev
  ownerRef: Network/work-net
spec:
  providerRef: Provider/system-minijail
  executionRef: Guest/net-work-net
  domain: system
  processClass: worker                  # no bus, no resource API, no child spawning
  template: net-vm-dnsmasq
  sandbox:
    namespaceClasses: []                # inherit Guest namespaces
    capabilityClasses: [network-bind, network-raw]
    # network-bind → CAP_NET_BIND_SERVICE: bind to port 53 (DNS) and port 67 (DHCP)
    # network-raw  → CAP_NET_RAW:          DHCP raw socket operations
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
  budget:
    memory: { request: "8Mi", limit: "32Mi" }
    pids: { limit: 8 }
    fds: { limit: 64 }
  mounts:
    - volumeRef: Volume/net-work-net-config
      view: guest-readonly
      mountPath: "/run/d2b/net-config"
      access: read-only
      required: true
  networkUsage:
    networkRef: Network/work-net
    ports:
      - port: 53
        protocol: udp
        purpose: dns
      - port: 53
        protocol: tcp
        purpose: dns
      - port: 67
        protocol: udp
        purpose: dhcp
    allowEgress: true
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "1s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "1s"
    timeout: "20s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
    # Provider-defined readiness: dnsmasq-bound socket detected by the Process Provider
  healthCheck:
    enabled: true
    interval: "30s"
    timeout: "5s"
    failureThreshold: 3
    class: provider-defined
  adoptionPolicy: adopt-on-restart
  drainTimeout: "5s"
```

**Config updates**: when the controller writes new config Volume content, it
forces a dnsmasq restart by setting `desiredLifecycle: stopped` followed by
`running` in a ResourceMutationBatch.  The controller waits for the dnsmasq
Process to reach Ready again before reporting `DhcpReady=True`.

dnsmasq invariants (preserved from `nixos-modules/net.nix` lines 302–441):
- `bind-interfaces=true` (binds only to `eth1`/LAN);
- `dhcp-ignore-names=true` (no hostname spoofing);
- static DHCP host reservations from `spec.attachments[]` (via config Volume);
- DHCP dynamic pool: `lanCidr.251`–`lanCidr.254`;
- DNS forwarders from `spec.dns.forwarders`;
- runs under the `net-local-controller` OS user with hardened minijail confinement.

### 11.3 mDNS reflector worker (Process/net-\<networkName\>-mdns-reflector)

Created only when `spec.mdns.enable = true`.

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: net-work-net-mdns-reflector
  zone: dev
  ownerRef: Network/work-net
spec:
  providerRef: Provider/system-minijail
  executionRef: Guest/net-work-net
  domain: system
  processClass: worker                  # no bus, no resource API
  template: net-vm-mdns-reflector
  sandbox:
    namespaceClasses: []
    capabilityClasses: [network-raw]    # CAP_NET_RAW for multicast socket
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
  budget:
    memory: { request: "4Mi", limit: "16Mi" }
    pids: { limit: 4 }
    fds: { limit: 32 }
  networkUsage:
    networkRef: Network/work-net
    ports:
      - port: 5353
        protocol: udp
        purpose: mdns
    allowEgress: true
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "1s"
    timeout: "15s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
  healthCheck:
    enabled: false
    interval: "30s"
    timeout: "5s"
    failureThreshold: 3
    class: provider-defined
  adoptionPolicy: adopt-on-restart
  drainTimeout: "5s"
```

### 11.4 mDNS local DNS bridge worker (Process/net-\<networkName\>-mdns-dnsbridge)

Created only when `spec.mdns.enable = true` and `spec.mdns.dnsmasqLocal = true`.

```yaml
apiVersion: resources.d2b.io/v3
type: Process
metadata:
  name: net-work-net-mdns-dnsbridge
  zone: dev
  ownerRef: Network/work-net
spec:
  providerRef: Provider/system-minijail
  executionRef: Guest/net-work-net
  domain: system
  processClass: worker
  template: net-vm-mdns-dnsbridge
  sandbox:
    namespaceClasses: []
    capabilityClasses: [network-bind]   # CAP_NET_BIND_SERVICE for DNS port
    seccompClass: strict
    noNewPrivileges: true
    startRoot: false
    readOnlyRoot: true
    environmentClass: minimal
  budget:
    memory: { request: "4Mi", limit: "16Mi" }
    pids: { limit: 4 }
    fds: { limit: 32 }
  networkUsage:
    networkRef: Network/work-net
    ports:
      - port: 5353
        protocol: udp
        purpose: mdns
    allowEgress: true
  desiredLifecycle: running
  restartPolicy:
    class: on-failure
    backoffBase: "2s"
    backoffMax: "60s"
    backoffMultiplier: 2.0
    maxRestarts: null
    resetAfter: "300s"
  readiness:
    initialDelay: "1s"
    timeout: "15s"
    failureThreshold: 3
    successThreshold: 1
    class: provider-defined
  healthCheck:
    enabled: false
    interval: "30s"
    timeout: "5s"
    failureThreshold: 3
    class: provider-defined
  adoptionPolicy: adopt-on-restart
  drainTimeout: "5s"
```

---

## 12. Host fabric lifecycle

### 12.1 Bridge creation and deletion

Bridge creation and deletion are **dynamic operations** driven through
`NetworkEffectPort.create_fabric()` and `NetworkEffectPort.delete_fabric()`.
A NixOS generation switch is NOT required to create or remove a Network.

`create_fabric` (via core adapter `CreateBridge` broker op):
- creates the kernel bridge device with the internally-derived IfName;
- sets MTU from `spec.mtu`;
- disables STP and multicast snooping unconditionally;
- applies IPv6 suppression sysctls atomically
  (`disable_ipv6=1`, `accept_ra=0`, `autoconf=0`)
  before returning (closes race between creation and subsequent sysctl step).

`delete_fabric` (via core adapter `DeleteBridge` broker op):
- removes the kernel bridge device and all tap ports still attached;
- idempotent (returns success if already absent).

### 12.2 IPv6 suppression (defense-in-depth)

IPv6 suppression is applied at two points:
1. Atomically inside `create_fabric` (per bridge, at creation time);
2. Via `apply_host_sysctls` at each reconcile cycle (handles `systemctl restart
   systemd-networkd` and any sysctl drift).

No Nix boot-time sysctl entry is required for specific bridge IfNames because
bridges are created dynamically.

### 12.3 Tap lifecycle

Tap creation for workload Guests is performed by **`Provider/runtime-cloud-hypervisor`**,
not by the network-local controller.  The network-local controller:
1. Calls `NetworkEffectPort.declare_attachment_tap()` to record the attachment intent
   and receive an `AttachmentHandle`;
2. Stores the handle in `Network.status.resource.attachments[]`;
3. Calls `NetworkEffectPort.set_attachment_isolation()` to apply isolated/
   neigh-suppress bridge port flags when the tap is created.

The runtime calls `revoke_attachment_tap()` during Guest deletion.

### 12.4 NM unmanaged and /etc/hosts

- `NetworkEffectPort.apply_nm_unmanaged()` writes `00-d2b-unmanaged.conf`
  with the `d2b-*` prefix pattern, covering all dynamically-created d2b bridges
  and taps regardless of specific IfNames.
- `NetworkEffectPort.update_hosts_file()` maintains VM→IP entries in the
  `d2b-managed` block of `/etc/hosts`.  **No hostname, IP, or MAC is stored in
  any public spec/status/audit field** — /etc/hosts entries are write-only
  from the resource API's perspective.

### 12.5 DHCP pre-seed

`NetworkEffectPort.seed_dhcp_reservations()` pre-seeds dnsmasq DHCP lease
reservations for known attachment MACs via `SeedDnsmasqLease` broker op.  Entries
use opaque attachment refs; the DHCP MAC-to-IP mappings are not stored in any
public resource field.

---

## 13. DHCP/DNS and firewall lifecycle

### 13.1 DHCP/DNS (inside net VM)

The dnsmasq worker Process (§11.2) runs inside the net VM.  The network-local
controller writes `dnsmasq.conf` to the config Volume; dnsmasq reads it at startup
from the mounted read-only Volume view at `/run/d2b/net-config/dnsmasq.conf`.

Config update flow:
1. Controller detects `Network.spec` change affecting DHCP/DNS.
2. Controller writes new `dnsmasq.conf` to the config Volume via Volume service.
3. Controller calls `NetworkAgentService.Reload(new_digest)` on the agent to apply
   updated nftables/routes.
4. Controller restarts the dnsmasq Process (sets `desiredLifecycle: stopped` then
   `running` in a ResourceMutationBatch).
5. dnsmasq Process Provider stops dnsmasq, waits for Process Ready, then starts
   dnsmasq with the new config.
6. Once dnsmasq Process returns to Ready, controller sets `DhcpReady=True`.

### 13.2 Host-side firewall (inet d2b table)

The controller calls `NetworkEffectPort.apply_host_firewall()` with a
`FirewallIntent` at each reconcile cycle.  The `inet d2b` table:
- blocks all traffic on LAN bridges (host has no IP there);
- allows TCP/3240 on uplink bridges (USBIP carve-out; raw IfName resolved internally
  by core adapter);
- installs per-rule `comment "d2b managed: <ownership-id>"` markers
  (ownership ID is the Network resource UID — opaque, not the IfName);
- coexists with other firewall managers per `FirewallCoexistencePolicy`
  (Coexist/Refuse/RequireUnmanaged matrix from `d2b-host/src/nftables.rs`).

The returned `FirewallDigest` is stored in `status.provider.details.firewallDigest` for
drift detection.  No rule text appears in status, audit, or telemetry.

### 13.3 Net-VM-side firewall (via config Volume)

The controller writes the net VM's nftables ruleset to the `nftables.rules` config
Volume entry.  The **net-agent service** reads and applies it via `nft -f` at
startup and on each `Reload()` call.  The ruleset preserves all semantics from
`nixos-modules/net.nix` lines 168–296 (see §16 security invariants for the full
chain).

### 13.4 Drift detection (observe)

On each observe cycle (`observeInterval: 60s`):
- `NetworkEffectPort.read_firewall_digest()` → compare against `status.provider.details.firewallDigest`;
  if drift, set `FirewallReady=False/nftables-drift` and queue reconcile.
- `NetworkEffectPort.read_sysctl_state()` → compare against expected IPv6 suppression;
  if drift, queue reconcile.
- `NetworkEffectPort.read_attachment_isolation()` per attachment → compare against
  expected isolation; if drift, queue reconcile.

Observation commits status-only updates without incrementing resource generation.

---

## 14. Attachment lifecycle

### 14.1 Workload Guest attachment

A workload Guest requests attachment by appearing in `Network.spec.attachments`.
The network-local controller:
1. Calls `NetworkEffectPort.declare_attachment_tap()` → receives `AttachmentHandle`.
2. Stores handle in `Network.status.resource.attachments[]` (opaque; no raw IfName).
3. Calls `NetworkEffectPort.set_attachment_isolation()` with `isolated: !spec.isolation.allowEastWest`.

The runtime-cloud-hypervisor Provider resolves the `AttachmentHandle` to an FD
via LaunchTicket when starting the workload Guest.  The runtime does not read
the `AttachmentHandle` directly; core supplies the FD implicitly.

### 14.2 East-west isolation

Default: `isolation.allowEastWest = false`:
- tap bridge port flags `Isolated=true`;
- no `eth1→eth1 new accept` rule in net-VM forward chain.

`allowEastWest = true`:
- `set_attachment_isolation(handle, isolated: false)` on all workload taps;
- adds east-west accept rule in net-VM forward chain.

### 14.3 External attachment (macvtap)

When `spec.externalAttachment` is non-null:
1. Controller copies operator-declared external attachment parameters from
   `Network.spec.externalAttachment` into the net-VM Guest spec's `spec.provider.settings`:
   `externalHostInterface` (operator-declared NIC name), `externalMode`, and
   optional static MAC/IP fields.  These are desired spec values specified by the
   operator, not dynamically derived kernel values.
2. `Provider/runtime-cloud-hypervisor` reads these typed parameters and calls the
   broker's `SpawnRunner` path.  The broker creates the macvtap FD internally
   (`live_create_macvtap_fd` in `d2b-priv-broker/src/runtime.rs`) as part of VMM
   spawn dispatch.
3. Port-forward DNAT rules are written to `nftables.rules` by the controller and
   applied by the net-agent inside the net VM.

The `ExternalAttachmentReady` condition reflects macvtap interface state via the
net VM's Guest readiness predicates.

---

## 15. USBIP proxy boundary

The USBIP backend and proxy processes are **not** owned by the network-local
controller.  They are owned by `Provider/device-usbip`.

The `UsbipBindFirewallRule` broker operation stays with `Provider/device-usbip`.
The network-local controller does **not** install USBIP firewall rules and does
**not** call `UsbipBindFirewallRule`.

The TCP/3240 carve-out in the host `inet d2b` table is generated by the
network-local controller as a general allow-rule on the uplink bridge (the rule
is parameterized by the Network's uplink bridge attachment handle, not by the
USBIP provider's configuration).  `Network.spec` has no `usbipCarveOut` field
and must not be mutated by the device-usbip provider.

`Provider/device-usbip` consumes `Network/work-net` via a `networkRef` dependency
in its Device resource spec.  It reads only the Network's `AttachmentHandle` for
the uplink (opaque) and uses it to request a firewall rule through its own broker
interaction.

---

## 16. Reconcile loop

The network-local controller implements the full reconciliation contract from
`ADR-046-resource-reconciliation`.

### 16.1 Async loop invariants

From RECONCILE §Async interface:
> No handler holds a redb transaction or blocking kernel/systemd/filesystem call
> across an await.  Blocking effects use explicit bounded adapters.

From RECONCILE §Async loop, step 7:
> Each resource has one running handler; independent resources run in parallel
> under semaphore/budget.

From RECONCILE §Reconcile context:
> It contains no database handle, direct broker socket, reusable credential, raw
> route table, or authority supplied by the resource payload.

All `NetworkEffectPort` calls are dispatched in background tasks through bounded
blocking adapters.  The reconcile handler releases any redb read transaction before
the first `await` on an effect call.  Each resource's reconcile/observe/finalize
handler runs independently; the watch receiver continues dispatching other ready
resources without waiting for any single handler.

### 16.2 Reconcile (Network resource)

```text
1. validateSpec
   └─ CIDR, attachment index, IfName collision, netVmSystemArtifactId format
   └─ fail → ReconcileError{reason}; set condition; return failed-retryable

2. Check User/net-local-controller status.phase == Ready
   └─ not Ready → set DependenciesReady=False; return pending

3. [background task] create_fabric + apply_host_sysctls + apply_host_firewall +
                     apply_host_routes + apply_nm_unmanaged
   └─ each dispatched independently; handler does not block between calls
   └─ error → set FabricReady=False or FirewallReady=False; return failed-retryable

4. Create or update Volume/net-<networkName>-config (Phase 1)
   └─ error → ConfigVolumeReady=False; return failed-retryable

5. Write config content to Volume via Volume service (all 4 files)
   └─ no raw IfName or workload hostname in content

6. Create or update Guest/<netVmName>
   └─ systemArtifactId = Network.spec.netVmSystemArtifactId (plain bounded ID)
   └─ spec.provider.settings.vsockCid from Network CIDR allocation
   └─ when externalAttachment non-null: add declared-spec external params only

7. Wait for Guest Ready (via DependenciesReady hint)
   └─ pending → set NetVmReady=False; return pending

8. Update Volume to add Guest attachment (Phase 2)
   └─ wait for attachment Ready
   └─ pending → ConfigVolumeReady=False/attachment-not-ready; return pending

9. Create Process/net-<networkName>-agent (service)
   Create Process/net-<networkName>-dnsmasq (worker)
   Create Process/net-<networkName>-mdns-reflector (worker; if mdns.enable)
   Create Process/net-<networkName>-mdns-dnsbridge (worker; if mdns.dnsmasqLocal)
   └─ each create is independent; handler does not block between creates

10. Call NetworkAgentService.Reload(config_digest) on agent service
    └─ wait for ReloadResult.predicates.{nft_applied, routes_applied}
    └─ update_hosts_file via NetworkEffectPort (no raw IfName/IP in audit)
    └─ seed_dhcp_reservations via NetworkEffectPort

11. Set_attachment_isolation for each workload tap via NetworkEffectPort

12. Commit ResourceMutationBatch with all child resource mutations + status

13. Evaluate conditions; report phase
    └─ all conditions True → phase: Ready
    └─ any terminal error → phase: Failed
    └─ partial → phase: Degraded
```

### 16.3 Finalizer (delete sequence, strictly child-first)

```text
network.d2b.io/fabric-cleanup finalizer

1. Set NetworkDraining condition

2. Set desiredLifecycle:stopped on all attached workload Guests (via ResourceMutation)

3. Wait for all attachment phases to become non-Ready (workload Guests stopped)

4. Delete mDNS Process resources (if any); wait for each Deleted watch event
   └─ Deleted event: single atomic store transaction (row+index removed); no
      persistent phase=Deleted row; audit record separate from deletion tx

5. Delete Process/net-<networkName>-agent; wait for Deleted event

6. Delete Process/net-<networkName>-dnsmasq; wait for Deleted event

7. Update Volume attachments to [] (remove Guest attachment); wait for removal

8. Delete Guest/<netVmName>; wait for Deleted event

9. Delete Volume/net-<networkName>-config; wait for Deleted event

10. [background tasks, independent]:
    remove_host_firewall(network_uid)
    remove_host_routes(network_uid)
    update_hosts_file(network_uid, empty)
    delete_fabric(lan_fabric_handle)
    delete_fabric(uplink_fabric_handle)
    apply_nm_unmanaged(empty pattern for this network)
    Each is idempotent; failure is retried before clearing finalizer

11. Clear finalizer
```

Each step is driven by `owned-resource-changed` hints rather than polling.
The handler does not block the watch receiver while waiting for child Deleted events.

### 16.4 Adopt (controller restart)

On controller restart (continuation event):
1. List all Network resources in Zone.
2. For each, read current host bridge state via `NetworkEffectPort.read_firewall_digest()`
   and `read_sysctl_state()`.
3. If the controller's internally-held fabric handles are consistent and digests
   match: mark adopted (no re-application).
4. If bridges absent: normal reconcile creates them.

Adoption never deletes or restarts running state.

---

## 17. ProviderStateSet

`ProviderStateSet(zone, "network-local")` is the **query-time grouping** of all
Volume resources in a Zone whose `metadata.ownerRef == "Provider/network-local"`.
It is not a ResourceType or a stored artifact; it is the logical set defined in
`ADR-046-provider-state`:

```text
ProviderStateSet(zone, "network-local") =
  { v : Volume | v.metadata.zone == zone
              && v.metadata.ownerRef == "Provider/network-local" }
```

The set itself is not a compartment type or a framework-managed object — it is a
query.  The Volumes in the set are ordinary Volume resources that happen to carry
`ownerRef: Provider/network-local`.

Under D087, `Provider/network-local` declares **no Provider state Volume**. Its
ProviderStateSet is therefore empty:

```text
ProviderStateSet(zone, "network-local") = {}
```

The controller's network operational state fails the storage-need test for a
durable Provider state Volume: bridge, route, nftables, DHCP, attachment, and
adoption observations are bounded, non-secret, and derivable from `Network.spec`,
`Network.status`, the core Operation ledger, broker operation results, and
external kernel/network observation after restart.

The controller Process therefore mounts no Provider state Volume, declares no
state namespace, has no dedicated state-layout `User/<name>` principal, and has
no identity marker, migration worker, Provider state reset path, or Provider
state destroy path. There is no bootstrap state-Volume mechanism; the previous
bootstrap exception (D086, superseded by D087) does not apply.

The per-Network config Volumes (§9) are preserved. They carry actual runtime
configuration content (`dnsmasq.conf`, `nftables.rules`, `routing.conf`,
`attachments.json`) on tmpfs with `ownerRef: Network/<networkName>`, so they are
runtime/config operational Volumes, not Provider state Volumes and not members of
the ProviderStateSet. Runtime network artifacts such as bridges, routes,
nftables rules, and mDNS/agent Processes are likewise unaffected and remain
broker/controller-managed operational state, not Provider state Volumes.

Status is observation only. It is revisioned, optimistic-status-writer
controlled, RBAC-readable, redacted, bounded to the global/provider-detail
limits, written only on material change, and re-verified against external
kernel/network reality after restart. It never contains secrets, authority
handles, private paths, argv/env, PIDs, unit names, raw command output, large
blobs, or unbounded collections; oversize status is rejected with
`status-oversize`.

The network-local controller does not add Volume to its exported `ResourceTypes
implemented`. `Provider/volume-local` remains the reconciler for per-Network
config Volume resources (§9); the controller creates those resource objects and
writes their bounded config content through the Volume service, but it does not
reconcile Volumes and does not create a Provider state Volume prerequisite.

---

## 18. RBAC

### 18.1 Roles

```yaml
# Operator roles
type: Role
metadata: { name: network-operator, zone: dev }
spec:
  rules:
    - resourceTypes: [Network]
      verbs: [get, list, watch, create, update-spec, delete]
      zones: [dev]
    - resourceTypes: [Zone]
      verbs: [get]
      zones: [dev]
---
type: Role
metadata: { name: network-reader, zone: dev }
spec:
  rules:
    - resourceTypes: [Network]
      verbs: [get, list, watch]
      zones: [dev]
---
# Controller role (bound to Provider/network-local)
type: Role
metadata: { name: network-local-controller, zone: dev }
spec:
  rules:
    - resourceTypes: [Network]
      verbs: [get, list, watch, update-status, update-finalizers]
      zones: [dev]
    - resourceTypes: [Guest]
      verbs: [get, list, watch, create, update-spec, delete]
      zones: [dev]
      # Scoped: only Guests with ownerRef resolving to a Network resource
    - resourceTypes: [Volume]
      verbs: [get, list, watch, create, update-spec, delete, write-content]
      zones: [dev]
      # Scoped: only Volumes with ownerRef resolving to a Network resource
    - resourceTypes: [Process]
      verbs: [get, list, watch, create, update-spec, delete]
      zones: [dev]
      # Scoped: only Processes with ownerRef resolving to a Network resource
    - resourceTypes: [User]
      verbs: [get, watch]               # read User/net-local-controller status
      zones: [dev]
    - resourceTypes: [Host]
      verbs: [get]
      zones: [dev]
    - resourceTypes: [Zone]
      verbs: [get]
      zones: [dev]
---
type: RoleBinding
metadata: { name: network-local-ctrl-binding, zone: dev }
spec:
  roleRef: Role/network-local-controller
  subjects:
    - Provider/network-local
```

The controller holds **no** broker role and **no** `network-admin` capability.
All host-kernel effects go through the injected `NetworkEffectPort`.

### 18.2 Resource/verb matrix

| Verb | Resource | Held by | Notes |
| --- | --- | --- | --- |
| `create` | `User/net-local-controller` | **Nix config publication** | NOT the network-local controller; declared in Nix |
| `update-status` | `User/net-local-controller` | `Provider/system-core` only | system-core reconciles via NSS |
| `get,watch` | `User/net-local-controller` | `Provider/network-local` | read-only precondition |
| `update-status` | `Network` | `Provider/network-local` | sole status owner |
| `create,update-spec` | `Guest` | `Provider/network-local` | scoped to ownerRef=Network |
| `create,update-spec` | `Volume` | `Provider/network-local` | creates per-Network config Volume resource objects only; volume-local reconciles; network-local does NOT implement Volume ResourceType |
| `write-content` | `Volume` | `Provider/network-local` | config content writes via Volume service |
| `create,update-spec` | `Process` | `Provider/network-local` | agent + dnsmasq + mDNS processes |

---

## 19. d2b-bus

The network-local controller authenticates to d2b-bus with a local Noise-NN
profile (no pre-shared key; Host/Zone scope).

### 19.1 Watch registrations

| ResourceType | Watch selector | Purpose |
| --- | --- | --- |
| `Network` | all in Zone | owns reconcile |
| `Guest` | `ownerRef: Network/*` | observe net-VM lifecycle |
| `Volume` | `ownerRef: Network/*` | observe config Volume lifecycle |
| `Process` | `ownerRef: Network/*` | observe agent/dnsmasq/mDNS lifecycle |
| `User` | `name: net-local-controller` | precondition check |
| `Host` | all in Zone | host network inventory for hostBlocklist |

No `Network` or `Host` resource is watch-subscribed by the agent or dnsmasq
processes.  Workers have no bus authority.  The agent service endpoint is the sole
bus-adjacent surface for guest-side interactions.

### 19.2 Service endpoint (agent)

The net-agent service process exposes `d2b.network.v3.agent/v1` on a Noise-KK
vsock endpoint.  Only the network-local controller is authorized to call this
service (bound via the Zone's internal ComponentSession RBAC).

---

## 20. Status, errors, and conditions

### 20.1 Error codes (stable; no raw kernel output)

| Code | Phase | Description |
| --- | --- | --- |
| `network-cidr-conflict` | Failed | CIDR overlap detected |
| `ifname-collision` | Failed | Derived IfName collision; terminal |
| `bridge-create-error` | Degraded | create_fabric failed |
| `sysctl-error` | Degraded | apply_host_sysctls failed |
| `nftables-error` | Degraded | apply_host_firewall failed |
| `nftables-drift` | Degraded | Firewall digest mismatch detected at observe |
| `nm-unmanaged-error` | Degraded | apply_nm_unmanaged failed |
| `route-error` | Degraded | apply_host_routes failed |
| `config-volume-error` | Degraded | Volume create failed |
| `volume-backing-error` | Degraded | Volume backing not Ready |
| `attachment-not-ready` | Degraded | Volume Guest attachment not Ready |
| `net-vm-pending` | Pending | Guest not yet Ready |
| `net-vm-failed` | Failed | Guest in Failed phase |
| `net-vm-degraded` | Degraded | Guest in Degraded phase |
| `agent-restart` | Degraded | Agent process restarted unexpectedly |
| `agent-reload-failed` | Degraded | NetworkAgentService.Reload() returned error |
| `dnsmasq-not-bound` | Degraded | dnsmasq process not Ready; DNS/DHCP unavailable |
| `nft-not-applied` | Degraded | Agent reports nft_applied=false |
| `macvtap-not-ready` | Degraded | External attachment macvtap not ready |
| `mdns-process-not-ready` | Degraded | mDNS reflector or bridge not Ready |
| `net-vm-artifact-missing` | Failed | netVmSystemArtifactId absent from artifact catalog |
| `net-vm-artifact-type-mismatch` | Failed | Artifact type is not nixos-system |
| `user-not-ready` | Pending | User/net-local-controller not Ready |

Error messages are bounded and contain no raw kernel output, ifNames, IPs, MACs,
cgroup paths, or internal resource paths.

### 20.2 Latency guidelines

| Operation | P50 target | P99 target |
| --- | --- | --- |
| Bridge creation (create_fabric) | < 200 ms | < 500 ms |
| Host nftables apply | < 100 ms | < 300 ms |
| Config Volume write (4 files) | < 50 ms | < 150 ms |
| Agent Reload() round-trip | < 500 ms | < 2 s |
| dnsmasq restart | < 1 s | < 3 s |
| Full Network provisioning | < 10 s | < 30 s |
| Full Network deletion | < 15 s | < 45 s |

---

## 21. Audit, OTEL, and redaction

### 21.1 Audit records

One audit record per Resource API mutation; additional records per `NetworkEffectPort`
call (emitted by the core adapter, not the provider crate).

Network-specific audit payload:

| Field | Included | Rationale |
| --- | --- | --- |
| ResourceType and resource name | Yes | operational identity |
| verb / subresource | Yes | standard |
| `network.lanCidr` | Yes | address allocation decision |
| `network.uplinkCidr` | Yes | address allocation decision |
| `network.isolation.allowEastWest` | Yes | security-relevant policy change |
| `network.attachments[].executionRef` | Yes | Guest identity is operational |
| `firewallDigest` | Yes | drift evidence (opaque hex; no rule text) |
| Bridge/tap drift reason | Yes (stable code; no raw IfName) | diagnostic |
| `network.attachments[].attachmentHandle` | Yes (opaque; no IfName) | fabric identity |
| Workload hostname, IP, MAC | **No** | redacted from API-level audit |
| nftables rule text | **No** | redacted |
| DHCP lease data | **No** | never written to audit |
| dnsmasq config contents | **No** | not audit material |
| Raw IfNames | **No** | internal to core adapter |
| raw kernel interface names | **No** | redacted |
| `externalAttachment.portForwards[].targetIp` | **No** | workload-internal |
| Error message body | **No** (error code only) | no kernel output |

Broker operations emit their own audit records (path-free outcome codes) from within
the core adapter.

### 21.2 OTEL spans and metrics

Root span per reconcile attempt:

```
d2b.network.reconcile
  network.name: <ResourceName>        # plain name; NOT hostname
  network.zone: <ZoneName>
  network.generation: <n>
  reconcile.trigger: <reason-set>
  reconcile.attempt: <n>
  outcome: converged|pending|degraded|failed-retryable|failed-terminal
```

Child spans (no raw IfName, IP, MAC, rule text, or lease data in any attribute):

```
d2b.network.effect.create_fabric
d2b.network.effect.delete_fabric
d2b.network.effect.apply_firewall
d2b.network.effect.apply_routes
d2b.network.effect.apply_sysctls
d2b.network.effect.update_hosts
d2b.network.effect.seed_dhcp
d2b.network.volume.sync
d2b.network.guest.sync
d2b.network.agent.reload
d2b.network.dnsmasq.restart
d2b.network.observe.drift_check
```

Labels use closed cardinality.  `network.name` is a ResourceName (bounded
`^[a-z][a-z0-9-]*$`), never a hostname or FQDN.

Metrics:

| Metric | Labels |
| --- | --- |
| `d2b_network_reconcile_total` | `zone`, `outcome` |
| `d2b_network_phase` | `zone`, `phase` |
| `d2b_network_attachment_count` | `zone`, `network` |
| `d2b_nftables_apply_total` | `zone`, `outcome` |
| `d2b_nftables_drift_total` | `zone` |
| `d2b_bridge_create_total` | `zone`, `outcome` |
| `d2b_bridge_delete_total` | `zone`, `outcome` |
| `d2b_network_volume_sync_total` | `zone`, `outcome` |
| `d2b_network_agent_reload_total` | `zone`, `outcome` |
| `d2b_network_agent_restart_total` | `zone`, `outcome` |
| `d2b_network_dnsmasq_restart_total` | `zone`, `outcome` |
| `d2b_network_observe_drift_total` | `zone`, `surface` |

---

## 22. Nix configuration

### 22.1 Artifact catalog entries

```nix
# In flake.nix / nixos-modules/bundle-artifacts.nix
d2b.artifacts.provider-network-local = {
  package = packages.${system}.d2b-provider-network-local;
  type    = "provider";
};

d2b.artifacts.net-vm-base = {
  package = pkgs.d2b-net-vm-nixos-system;
  type    = "nixos-system";
};
```

Artifact IDs match `^[a-z][a-z0-9-]*$`.  They are plain bounded IDs, not paths.
The resource spec/status/audit surface never exposes Nix store paths; only the
private artifact catalog retains the derivation reference.

### 22.2 Provider and User declaration

```nix
# In d2b.zones.dev.providers (or d2b.zones.dev.resources)
d2b.zones.dev.resources = {
  network-local = {
    type = "Provider";
    spec = {
      artifactId = "provider-network-local";
      config = {
        controllerExecutionRef = "Host/host-system";
      };
    };
  };

  # User resource declared here — NOT created dynamically by the controller
  net-local-controller = {
    type = "User";
    metadata.ownerRef = "Provider/network-local";
    spec = {
      osUsername = "net-local-controller";
      # displayName and groups are optional
    };
  };
};
```

The Nix module also provisions the OS account in the host NixOS system:
```nix
# nixos-modules/host-users.nix additions for network-local
users.users.net-local-controller = {
  uid         = <RESERVED_UID>;          # fixed private UID; never in ResourceSpec
  isSystemUser = true;
  group       = "net-local-controller";
  home        = "/var/empty";
  shell       = pkgs.shadow + "/bin/nologin";
};
users.groups.net-local-controller.gid = <RESERVED_GID>;
```

The same account (identical UID/GID) is baked into the generic `net-vm-base`
nixos-system artifact so virtiofs ACLs on config Volume layout entries are
consistent inside the net VM.

### 22.3 Network resource declaration

```nix
d2b.zones.dev.resources.work-net = {
  type = "Network";
  spec = {
    networkName         = "work-net";
    netVmSystemArtifactId = "net-vm-base";   # plain bounded ID
    lanCidr             = "10.20.0.0/24";
    uplinkCidr          = "192.0.2.0/30";
    attachments         = [
      { executionRef = "Guest/corp-vm";    index = 10; }
      { executionRef = "Guest/personal-vm"; index = 11; }
    ];
    isolation.allowEastWest = false;
    dns.forwarders      = [ "8.8.8.8" "8.8.4.4" ];
  };
};
```

### 22.4 Nix static prerequisites

Nix provisions the following **static** prerequisites (no runtime IfName
knowledge required):

| Artifact | Purpose |
| --- | --- |
| `networking.networkmanager.unmanaged` block for `d2b-*` prefix | Covers all dynamically-created d2b bridges/taps regardless of IfName |
| `net-local-controller` OS account (fixed UID/GID) | virtiofs ACL consistency on both Host and Guest |
| Schema validation artifacts | Checked at build time (`nix flake check`) |
| Controller binary deployment | Package in the host system closure |
| `net-vm-base` nixos-system derivation | Generic net-VM OS; no per-Network config encoded |

No per-Network or per-IfName Nix entries are required; all dynamic fabric state is
provisioned at runtime through `NetworkEffectPort`.

### 22.5 eval-time checks

| Check | What is verified |
| --- | --- |
| `netVmSystemArtifactId` present | Required field; fails if absent |
| `netVmSystemArtifactId` type is `nixos-system` | Artifact catalog type check |
| `lanCidr` / `uplinkCidr` format | Regex + prefix length at Nix eval time |
| CIDR overlaps between declared Networks | Cross-Network CIDR overlap check (where input is available at eval time) |
| `networkName` regex | `^[a-z][a-z0-9-]*$` |

Runtime checks in `validateSpec` cover the full set.

---

## 23. Security invariants

### INV-NET-001: lib.mkForce on 10-eth-dhcp

**Invariant**: the net VM's NixOS config MUST contain a `lib.mkForce` override
replacing the `10-eth-dhcp` catch-all networkd definition with a non-matching
bogus MAC (`00:00:00:00:00:00`).

**Rationale**: prevents the catch-all from being selected for any real NIC,
which would start DHCP on all interfaces.

**Validation**: `tests/net-vm-network-eval.sh` (Layer-1 eval gate).

### INV-NET-002: IPv6 suppression

**Invariant**: all host-side bridge interfaces created by the network-local
controller MUST have `net.ipv6.conf.<ifname>.disable_ipv6 = 1`,
`accept_ra = 0`, and `autoconf = 0` before the bridge becomes active.
Suppression is applied both atomically at `create_fabric` time and defensively
at each reconcile via `apply_host_sysctls`.

**Rationale**: d2b is IPv4-only; suppression prevents kernel autoconf and
inadvertent IPv6 router solicitation on tenant bridges.

### INV-NET-003: LAN bridge host isolation

**Invariant**: the host has no IP address on any LAN bridge; LAN bridge is not
a routable host interface.

**Rationale**: prevents the host from becoming a router to the tenant LAN.

### INV-NET-004: workload tap isolation (default)

**Invariant**: workload taps are created with `Isolated=true` on the LAN bridge
by default.  Only the net-VM tap is non-isolated.  East-west traffic between
workloads passes through the net VM and is subject to the `inet filter forward`
chain.

**Rationale**: workloads cannot communicate directly at L2 without traversing
the net-VM firewall.

### INV-NET-005: east-west default deny

**Invariant**: when `isolation.allowEastWest = false`, the net VM's forward
chain contains no `eth1→eth1 new accept` rule and workload taps carry
`Isolated=true`.

### INV-NET-006: CIDR non-overlap

**Invariant**: no two Networks in a Zone may have overlapping `lanCidr`,
`uplinkCidr`, or `externalAttachment.egress.allowedCidrs` entries.  Validated
at `validateSpec` time and re-checked at each reconcile cycle.

### INV-NET-007: hostBlocklist effectiveness

**Invariant**: the effective `hostBlocklist` in the net VM always includes the
default RFC1918+link-local set plus all other active Network CIDRs in the Zone
plus the Host resource's observed network inventory.  The hostBlocklist cannot
be entirely emptied; it is only additive.

**Rationale**: prevents workloads from routing to host LAN ranges or to other
tenant networks.

### INV-NET-008: Guest-network-admin isolation

**Invariant**: `CAP_NET_ADMIN`, `CAP_NET_RAW`, and `CAP_NET_BIND_SERVICE`
granted to the net-agent and dnsmasq Processes are effective only within the
inherited Guest VM network namespace (the `namespaceClasses: []` Process spec
field causes the Process Provider to inherit the Guest's netns).  No host
capability is conferred.

**Rationale**: the net VM's privileged processes cannot affect the host network
stack.

**Validation**: `tests/unit/nix/cases/process-sandbox-netns.nix` (Layer-1 eval
case).

### INV-NET-009: no raw IfName/IP/MAC on public surface

**Invariant**: no raw kernel interface name, host bridge name, tap interface
name, workload IP address, DHCP MAC address, or host uplink IP address appears
in any of:
- `Network.spec` fields;
- `Network.status` fields;
- `Guest.spec.provider.settings`;
- OTEL span attributes;
- metric label values;
- audit record payload fields.

Raw IfNames are internal to the core adapter and are never exposed through the
provider crate's API boundary.

**Rationale**: prevents information-disclosure about the host kernel interface
topology through the resource API surface.

---

## 24. Provider lifecycle (install / upgrade / remove)

### 24.1 Install

1. Nix activation deploys `d2b-provider-network-local-ctrl` binary to the host
   system closure.
2. Nix activation provisions `net-local-controller` OS account and group.
3. Nix config publication creates `User/net-local-controller` resource and
   `Provider/network-local` resource.
4. Framework creates controller Process resource (`Process/network-local-ctrl`);
   system-minijail starts the controller.
5. Controller registers `Network` watch plan on d2b-bus.
6. Controller reconciles any already-declared Network resources.

### 24.2 Upgrade

On controller binary upgrade:
- `adopt-on-restart` policy on the controller Process causes system-minijail to
  adopt the new controller process transparently.
- Net-VM Guest processes are NOT restarted unless the `net-vm-base` artifact
  generation changes.
- Per-Network config Volumes are updated if the new controller detects spec drift.
- The `NetworkEffectPort` contract version is checked; mismatched versions fail
  the controller launch.

### 24.3 Remove

1. Operator deletes all Network resources; waits for each to complete its
   finalizer sequence (§16.3).
2. Operator deletes `Provider/network-local` resource.
3. Framework deletes controller Process resource; system-minijail stops controller.
4. Framework removes `User/net-local-controller` resource (blocked if any
   per-Network config Volume layout still references `User/net-local-controller`
   — must clear Networks first).
5. Nix activation removes the account (separate operator step, outside the
   resource lifecycle).

---

## 25. Migration from v1 baseline

### 25.1 Reused modules

| Module | Location | Reuse scope |
| --- | --- | --- |
| IfName derivation | `packages/d2b-host/src/ifname.rs:derive_ifname` | Full reuse; algorithm unchanged |
| nftables apply/hash | `packages/d2b-host/src/nftables.rs` | Full reuse; wrapped by core adapter |
| Bridge-port flags | `packages/d2b-host/src/bridge_port.rs` | Full reuse; wrapped by core adapter |
| Route preflight | `packages/d2b-host/src/routes.rs` | Full reuse; wrapped by core adapter |
| sysctl apply | `packages/d2b-host/src/netlink.rs` | Full reuse; wrapped by core adapter |
| CIDR validation | `nixos-modules/lib.nix:cidrOverlaps` (lines 429–462) | Ported to `validate.rs` |
| dnsmasq invariants | `nixos-modules/net.nix` lines 302–441 | Encoded in `dnsmasq.conf` rendering |
| nftables rules | `nixos-modules/net.nix` lines 168–296 | Encoded in `nftables.rules` rendering |
| lib.mkForce override | `nixos-modules/base.nix`:`10-eth-dhcp` | Preserved in net-vm-base artifact |

### 25.2 Breaking changes from v1 baseline

| Change | v1 behavior | v3 behavior |
| --- | --- | --- |
| IfName exposure | `br-<env>-lan` / `br-<env>-up` in NixOS module | IfNames are internal; never in resource API |
| Bridge creation | Declared in NixOS activation (static, per-env) | Dynamic broker effects via NetworkEffectPort |
| dnsmasq management | systemd unit declared in Nix per env | Separate worker Process resource per Network |
| mDNS | avahi static Nix config | Separate worker Process resources per Network |
| DHCP config | Static Nix config per env | Config Volume written by controller at runtime |
| Firewall | Static Nix config per env | Dynamic NetworkEffectPort.apply_host_firewall |
| Net-VM artifact ID | implicit path in microvm.nix | Explicit `netVmSystemArtifactId` field |

### 25.3 Migration work items

| ID | Category | Description |
| --- | --- | --- |
| ADR046-network-001 | Core | Implement `NetworkEffectPort` core adapter in `d2b-core`; map to broker wire ops; emit audit records. Versioning: minor releases may add methods with default impls; major releases require Provider upgrade. The trait lives in `d2b-contracts`; the adapter in `d2b-core`. |
| ADR046-network-002 | Core | Add `CreateBridge`, `DeleteBridge`, `ReadNftablesDigest`, `ReadSysctlState`, `ReadBridgePortFlags` broker ops |
| ADR046-network-003 | Core | Implement `AttachmentHandle` and `FabricHandle` as opaque byte-array newtypes (32 bytes of HMAC-SHA-256 over internal identity material; key held by core). Each handle is single-use; revocation is implicit when the owning Network is deleted. These types are declared in `d2b-contracts`, not in the provider crate. |
| ADR046-network-004 | Core | Implement LaunchTicket FD resolution: when core builds the LaunchTicket for a Guest with `ownerRef: Network/<name>`, it walks the owner graph, locates the Network, reads its internally-held `AttachmentHandle` set, and includes the corresponding tap FDs in the ticket. No API surface for the provider or runtime is required beyond the existing LaunchTicket mechanism. |
| ADR046-network-005 | Provider | The `d2b-host` IfName/nftables/bridge/route modules are consumed directly by the core adapter (not by the provider crate). The provider crate re-exports only `d2b_host::ifname::derive_ifname` for validation purposes. No additional extraction work is required beyond confirming the `d2b-host` API surface is stable. |
| ADR046-network-006 | Provider | Implement `controller.rs` reconcile/observe/finalize handlers with `NetworkEffectPort` injection |
| ADR046-network-007 | Provider | Implement `NetworkAgentService` Noise-KK vsock ComponentSession (Reload + ReadinessQuery methods). Agent reconnect policy: if the controller cannot reach the agent vsock (Guest restart in progress), it retries with exponential backoff up to `drainTimeout` of the agent Process; after timeout it deletes and re-creates the agent Process resource. |
| ADR046-network-008 | Provider | Implement config Volume content rendering (dnsmasq.conf, nftables.rules, routing.conf, attachments.json) |
| ADR046-network-009 | Provider | Implement canonical Process spec builders for agent, dnsmasq, mdns-reflector, mdns-dnsbridge |
| ADR046-network-010 | net-vm artifact | Build generic `net-vm-base` nixos-system artifact with net-agent binary, agent-service endpoint, guest-agent binary, standard NIC bootstrap, lib.mkForce override; bake `net-local-controller` account with the UID/GID allocated from the host-users reservation table (documented in `nixos-modules/host-users.nix`) |
| ADR046-network-011 | Nix | Nix module for `Provider/network-local` resource declaration; `User/net-local-controller` declaration; OS account provisioning; artifact catalog entries |
| ADR046-network-012 | Nix | Build-time CIDR overlap check for declared Networks in flake check |
| ADR046-network-013 | Tests | Conformance suite: NetworkSpec round-trip, IfName derivation, CIDR validation matrix |
| ADR046-network-014 | Tests | Controller state-machine unit tests with fake `NetworkEffectPort` (from d2b-contracts mock) and deterministic clock |
| ADR046-network-015 | Tests | Integration tests: full Network lifecycle (create, config update, agent Reload, delete sequence) in container environment |
| ADR046-network-016 | Security | Verify INV-NET-008 (Guest-network-admin isolation): Process Provider correctly inherits Guest netns for agent/dnsmasq |
| ADR046-network-017 | Docs | `packages/d2b-provider-network-local/README.md` covering all 7 required topics |
| ADR046-network-018 | Broker | `UsbipBindFirewallRule` broker op stays with `Provider/device-usbip`; verify separation in integration tests |
| ADR046-network-019 | Provider | Confirm `controller-main` declares no stateNamespace and core ProviderDeployment creates no Provider state Volume or state mount; validate ProviderStateSet query returns empty for `Provider/network-local`; validate bounded operational state is written to revisioned/redacted status and the core Operation ledger with `status-oversize` conformance; confirm per-Network config Volumes remain `ownerRef: Network/<name>` runtime/config operational Volumes outside the ProviderStateSet and `Volume` is not in `ResourceTypes implemented` |

---

## 26. Tests

### 26.1 Workspace policy

The workspace policy (`make test-policy` / `xtask workspace-policy`) requires four
paths at the crate root:

| Required path | Satisfied by |
| --- | --- |
| `src/` | at least one tracked `.rs` source file |
| `tests/` | at least one tracked `.rs` test file |
| `integration/` | at least one tracked `.rs` integration scenario file |
| `README.md` | root README covering all 7 required topics |

A nested `integration/README.md` is **optional** and not required by policy.
The integration test invocation commands are documented in the root `README.md`.

### 26.2 Unit tests (`tests/`)

| Test file | Coverage |
| --- | --- |
| `schema_roundtrip.rs` | `NetworkSpec` and `NetworkStatus` JSON serialize/deserialize; all optional fields; all enum variants |
| `state_schema_roundtrip.rs` | Provider descriptor has no stateNamespace for `controller-main`; no Provider state Volume, state mount, identity marker, migration worker, or state-layout principal is emitted; ProviderStateSet query returns empty; bounded operational observations live in status/core Operation ledger and pass redaction/size-bound checks; per-Network config Volumes are excluded from ProviderStateSet (ownerRef mismatch) |
| `ifname_derive.rs` | IfName derivation determinism; collision detection; 15-byte constraint; all role prefixes |
| `cidr_overlap.rs` | CIDR overlap matrix: same Network, cross-Network, external CIDR; all boundaries; no-false-positive at adjacent CIDRs |
| `controller_state.rs` | Full reconcile state machine: Normal path; CIDR conflict; User not Ready; Volume error; Guest timeout; agent reload failure; finalizer sequence (all child ordering); adoption on restart; drift detection cycle |
| `conformance.rs` | Provider toolkit black-box conformance suite; descriptor validation; ResourceType schema fingerprint |
| `fault_injection.rs` | `NetworkEffectPort` returns each `EffectError` variant; each step fails independently; retry/requeue classification; reconcile context has no broker socket; provider crate has no broker import |

### 26.3 Integration tests (`integration/`)

| Test file | Coverage | Runner |
| --- | --- | --- |
| `host_fabric.rs` | Bridge create/delete; nftables apply; IPv6 suppression; NM unmanaged; drift detection; NetworkEffectPort real impl | `make test-integration` (container) |
| `guest_lifecycle.rs` | net-VM Guest create/delete; opaque attachment handle resolution; systemArtifactId binding | `make test-host-integration` |
| `agent_reload.rs` | Agent service Reload() call; nft-applied + routes-applied predicates; config digest match | `make test-host-integration` |
| `mdns_reflector.rs` | mDNS reflector Process lifecycle; create when mdns.enable; delete on Network delete | `make test-integration` (container) |
| `delete_sequence.rs` | Full finalizer ordering: Process Deleted events, Volume attachment removal, Guest Deleted, Volume Deleted, fabric cleanup | `make test-host-integration` |

### 26.4 Eval tests (Layer-1, `tests/unit/nix/cases/`)

| Case file | Coverage |
| --- | --- |
| `network-spec-eval.nix` | `d2b.zones.dev.resources.work-net` Nix option round-trip; `netVmSystemArtifactId` required field; artifact type check |
| `network-cidr-overlap-eval.nix` | Dual-Network CIDR overlap eval-time assertion |
| `process-sandbox-netns.nix` | Agent and dnsmasq Process sandbox: `namespaceClasses: []` → inherits Guest netns; no capabilityClass on host |
| `net-vm-artifact-id-eval.nix` | `net-vm-base` artifact ID format; `nixos-system` type; no path separator |
| `user-no-managed-by-eval.nix` | `User/net-local-controller` spec contains no `managedBy`; `ownerRef` is in metadata |
| `provider-state-volume-eval.nix` | ProviderStateSet query-time membership returns empty for `Provider/network-local`; per-Network config Volumes (ownerRef: `Network/<name>`) are excluded and remain runtime/config operational Volumes |

### 26.5 Drift gates (Layer-1)

| Gate | What is guarded |
| --- | --- |
| `make test-drift` | `xtask gen-schemas` → `git diff --exit-code` on `docs/reference/schemas/v2/*.json`; Network schema drift |
| `make test-policy` | `xtask workspace-policy` → all four paths (`src/`, `tests/`, `integration/`, `README.md`) present |

---

## 27. Removal checklist

When `Provider/network-local` is retired (superseded or removed):

- [ ] All `Network` resources in all Zones must be deleted and finalizers cleared.
- [ ] Verify no Provider state Volume exists for `Provider/network-local` before
  marking Provider Deleted.
- [ ] `User/net-local-controller` resources must be deleted (after Network deletion
  releases all per-Network config Volume layout references).
- [ ] `Provider/network-local` resource must be deleted (after all Networks cleared).
- [ ] `net-local-controller` OS account must be removed from host NixOS config.
- [ ] `net-vm-base` artifact catalog entry must be removed from `d2b.artifacts`.
- [ ] `provider-network-local` artifact catalog entry must be removed.
- [ ] `d2b-provider-network-local` crate must be removed from workspace members and
  the members list must remain alphanumerically sorted.
- [ ] Broker ops `CreateBridge` and `DeleteBridge` may be retired if no other
  Provider uses them; consult the broker op table in `docs/reference/privileges.md`.
- [ ] `NetworkEffectPort` trait declaration in `d2b-contracts` must be removed or
  marked `#[deprecated]` when no other Provider uses it; the core adapter
  implementation is removed alongside the Provider.
- [ ] All eval-time tests and drift gates referencing `network-local` or
  `net-vm-base` must be updated or removed.
- [ ] CHANGELOG.md entry required for the removal (as a `Removed` entry under the
  appropriate version section).
