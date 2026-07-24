# ADR 0046 Nix configuration and resource compilation

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-nix-configuration` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 2 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `nixos-modules/`, Nix resource compiler, generated bundle artifacts |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-primitive-resource-composition`, `ADR-046-provider-model-and-packaging`, `ADR-046-core-controllers` |
| Supersedes | Current `nixos-modules/options-realms*.nix`, `options-envs.nix`, `options-vms.nix`, `index.nix`, `bundle*.nix`, `*-json.nix`, and generated `/etc/d2b/*.json` |

## Source, reuse, and evidence policy

This spec uses `b5ddbed6` as its authoritative factual baseline.

**Old current names versus new target names.** The baseline uses `Realm`/
`RealmId`/`RealmPath`/`WorkloadId`/`NodeId` everywhere. These are the current
live symbols. The target names `Zone`, `Host`, `Guest`, `ResourceRef` do not
exist in the baseline. A current source search must use the old names.
Specifically:

- `RealmId` / `RealmPath` / `d2b.realms.*` → target `Zone` / `d2b.zones.*`
- `WorkloadId` / `d2b.realms.<r>.workloads.*` → target `Guest` (for VM/sandbox/
  cloud/remote execution), or target user-only `Host` (for unsafe-local workloads).
  The target physical/local-execution parent `Host` is a new ResourceType.
- `NodeId` / `NodeKind` / `WorkloadPlacement` → target `Host` or `Guest`
  depending on NodeKind value (see classification table below).
- `ProcessRole` / `VmProcessDag` / runner / systemd helper → target `Process`,
  `EphemeralProcess`, or controller/probe/bootstrap per the disposition table below.
- `StoragePathSpec` / `storage.json` / `store-view` / filesystem-view →
  target `Volume` where independent lifecycle; locks/internal handles remain
  implementation mechanisms.
- `d2b-realm-provider` traits/adapters → target Zone-local `Provider` resource
  plus controller/service/worker Process components. Reachability varies by
  adapter (see reachability section).
- `d2b-realm-router` / `d2b-realm-transport` → target `d2b-bus` and
  `ComponentSession` (new; may copy/adapt main commit `a1cc0b2d`). These crates
  are compile-checked in `d2bd` via `realm_stubs.rs` (explicitly
  `dead_code`-allowed, not called from the running daemon at baseline).
- `RealmControllersJson` / `realm-controllers.json` → target Zone self-resource
  in `zones/<z>/zone.json`.
- `RealmWorkloadsLauncherV2Json` / `realm-workloads-launcher-v2.json` →
  target Process resource annotations in `zones/<z>/processes.json`.
- `Capability` enum → see capability disposition table below (mapping resolved;
  verb set owned by resource-api/authz foundation spec).

No main-branch behavior is assumed live. Every reuse work item names an
exact baseline source file and a precise v3 destination.

## Purpose

This spec defines how operators declare d2b 3.0 resource objects in Nix, how
those declarations compile into Zone resource stores, and how generated bundle
artifacts are produced, staged, and rolled back. It covers:

- Zone self-resource declaration and naming;
- Provider package catalog and `Provider/<name>` install resources;
- `Host`, `Guest`, and shared `ExecutionPolicy` specs;
- `Process`, `EphemeralProcess`, `Volume`, `Network`, `Device`, `User`, and
  `Credential` specs;
- `Role` and `RoleBinding` resources and RBAC compilation;
- controller placement templates;
- ref validation at eval time;
- exact schemas and defaults for every folded field;
- the universal Nix authoring and validation contract: canonical ResourceSpec
  JSON shape, ResourceTypeSchema and Provider-schema validation, Credential ref
  enforcement, and bundle integrity;
- the required `src/`/`tests/`/`integration/`/`README.md` layout for every
  `packages/d2b-provider-<base>-<implementation>/` crate and the workspace
  policy gate that enforces it;
- active-configuration generation, staging, and rollback across Zones;
- the resource cleanup contract: configuration-owned vs controller-created
  classification, async absent-resource deletion, finalizer-safe owner-child
  cascade, prior-generation retention, status/audit/tests;
- normalized index and bundle artifacts;
- package closures into Hosts and Guests;
- prohibited fields (secrets, freeform paths);
- conflict detection and rejection;
- the artifact catalog (`d2b.artifacts`) separating derivation-valued inputs
  from ResourceSpecs, with `artifactId`/`systemArtifactId` as plain bounded
  identifiers validated at build time;
- exact mapping from current option paths and Rust symbols to future files; and
- complete work items.

## Architectural invariants

These invariants must be enforced at eval time (Nix assertion or type system),
not deferred to runtime:

1. Every `*Ref` field follows `<ResourceType>/<resource_name>`. No scheme,
   Zone prefix, query string, relative segment, or implicit type is accepted.
2. `resource_name` matches `^[a-z][a-z0-9-]*$` — the same LABEL_PATTERN as
   `RealmId`/`WorkloadId` in `d2b-realm-core/src/ids.rs`.
3. Every intra-Zone ref resolves to a declared resource of the stated type in
   the same Zone. Cross-Zone refs are rejected unless routed through an
   explicit `ZoneLink`.
4. Owner cycles fail at eval time.
5. No secret value, credential bytes, raw numeric UID/GID, freeform host path,
   ambient capability list, raw seccomp program, or arbitrary socket address
   appears in any Nix-emitted resource spec or generated artifact.
6. The Provider package catalog is offline and sorted by exact content digest.
   No version ranges, `latest`, PATH scan, or runtime marketplace.
7. Configuration generation is hermetic and deterministic. Running the same
   Nix derivation twice with the same inputs produces byte-identical output.
8. Staged configuration becomes active only after eval-time validation passes.
   No partial activation is possible.
9. Every generated artifact in `/etc/d2b/` is owned `root:d2bd` mode `0640`
   unless the type table below specifies otherwise.
10. No current behavior is removed until the equivalent resource/Provider
    successor is integrated and tested.

## Current-symbol classification tables

### NodeKind → Host or Guest

Current source: `packages/d2b-realm-core/src/node.rs`, `NodeKind` enum.

| Current `NodeKind` variant | Current meaning | v3 target ResourceType |
| --- | --- | --- |
| `FullHost` | A full d2b host (KVM, broker, vsock, device control) | `Host` resource |
| `Gateway` | A realm gateway guest VM | `Guest` with explicitly selected existing runtime Provider (e.g., `Provider/runtime-cloud-hypervisor`) |
| `ProviderManaged` | Limited-capability provider-managed node | `Guest` under the managing Provider |

`Host` is a new ResourceType with no current equivalent. Its Nix declaration is
the v3 successor to `d2b.realms.<r>` with `placement = "host-local"` plus the
per-realm path-partition entry in `allocator.json`.

`NodeKind::Gateway` maps to a `Guest` resource whose `providerRef` is explicitly
selected by the operator from the existing catalog (e.g., `Provider/runtime-cloud-hypervisor`
or `Provider/runtime-qemu-media`). There is no dedicated gateway runtime Provider
entry in the initial catalog. Gateway functionality is a declaration pattern (a
Guest whose role is to mediate realm/zone bridging), not a distinct ResourceType
or Provider kind.

### WorkloadProviderKind → Guest or Host ExecutionPolicy

Current source: `packages/d2b-realm-core/src/workload.rs`, `WorkloadProviderKind`
and `IsolationPosture` enums. These drive current `WorkloadExecutionPosture`.

| Current value | Current meaning | v3 target |
| --- | --- | --- |
| `LocalVm` | Locally supervised NixOS VM (Cloud Hypervisor) | `Guest` + `providerRef: Provider/runtime-cloud-hypervisor` |
| `QemuMedia` | Locally supervised QEMU external-media runner | `Guest` + `providerRef: Provider/runtime-qemu-media` |
| `ProviderManaged` | Runtime owned by a provider adapter | `Guest` + exact frozen `providerRef` selected in config (e.g., `Provider/runtime-azure-container-apps`, `Provider/runtime-azure-virtual-machine`) |
| `UnsafeLocal` | Host-user process, no isolation boundary | User-only `Host` under `Provider/system-core` with `providerSettings.isolationPosture: "none"` |

For current `WorkloadProviderKind::ProviderManaged` workloads, the `providerRef`
in the compiled `Guest` resource is the exact frozen catalog entry selected by
the operator in their Nix config. Current live ACA paths use
`WorkloadProviderKind::ProviderManaged` backed by the ACA adapter
(`d2b-realm-provider/src/provider.rs`); these map to
`Provider/runtime-azure-container-apps`. Current Azure VM paths map to
`Provider/runtime-azure-virtual-machine`. The operator must select the exact
catalog entry name; the compiler never infers `providerRef` from a current
`ProviderId` value.

### ProcessRole disposition

Current source: `packages/d2b-core/src/processes.rs`, `ProcessRole` enum.
Every `VmProcessDag` node carries one `ProcessRole`. The target
`Process`/`EphemeralProcess` classification is determined per variant.

Evaluation note: a ProcessRole variant that has no independent lifecycle, no
independent owner, and is purely an implementation mechanism (e.g., a transient
health probe) may become a non-resource controller action. The threshold from
`ADR-046-primitive-resource-composition` applies.

| Current `ProcessRole` | Current description | v3 classification | Target Provider |
| --- | --- | --- | --- |
| `HostReconcile` | Host reconciliation before VM-specific startup | Controller action (not a Process resource; owned by `Provider/system-core`) | `Provider/system-core` |
| `StoreVirtiofsPreflight` | Store and virtiofs preflight validation | `EphemeralProcess` | `Provider/volume-virtiofs` |
| `SwtpmPreStartFlush` | swtpm pre-start flush step | `EphemeralProcess` | `Provider/device-tpm` |
| `Swtpm` | swtpm sidecar (long-lived) | `Process` | `Provider/device-tpm` |
| `Virtiofsd` | virtiofsd sidecar (long-lived) | `Process` | `Provider/volume-virtiofs` |
| `Video` | Optional video sidecar | `Process` | `Provider/device-gpu` |
| `Gpu` | GPU/graphics sidecar | `Process` | `Provider/device-gpu` |
| `GpuRenderNode` | GPU render-node-only mode | `Process` | `Provider/device-gpu` |
| `Audio` | Audio sidecar | `Process` | `Provider/audio-pipewire` |
| `CloudHypervisorRunner` | Cloud Hypervisor VMM | `Process` (owned by Guest) | `Provider/runtime-cloud-hypervisor` |
| `QemuMediaRunner` | QEMU media runner | `Process` (owned by Guest) | `Provider/runtime-qemu-media` |
| `VsockRelay` | vsock relay sidecar | `Process` | `Provider/transport-vsock` |
| `OtelHostBridge` | Host-to-observability-VM OTLP bridge | `Process` | `Provider/observability-otel` |
| `GuestSshReadiness` | Legacy SSH readiness probe (compat window) | Retired at v3 cutover; no v3 Process equivalent; no compatibility period | — |
| `GuestControlHealth` | Authenticated guest-control Health probe | `EphemeralProcess` | `Provider/system-core` |
| `Usbip` (long-lived backend/proxy) | USBIP long-lived proxy/backend | `Process` | `Provider/device-usbip` |
| `Usbip` (per-busid attach/detach) | USBIP per-busid attach/detach helper | `EphemeralProcess` | `Provider/device-usbip` |
| `SecurityKeyFrontend` | Guest CTAPHID relay frontend | `Process` | `Provider/device-security-key` |
| `WaylandProxy` | Host-jailed Wayland proxy | `Process` | `Provider/display-wayland` |

`ProcessRole::VsockRelay` becomes a `Process` resource owned by
`Provider/transport-vsock`. It is not an implementation mechanism internal to
`Provider/runtime-cloud-hypervisor`; it has an independent lifecycle (shared
across Guests using vsock transport) and therefore satisfies the
primitive-resource-composition threshold for an independent Process resource.

`ProcessRole::GuestSshReadiness` is retired at the v3 clean cutover. The
baseline comment explicitly marks it as "Replaced by GuestControlHealth";
there is no v3 `EphemeralProcess` equivalent and no compatibility period.
Operators with SSH-dependent tooling must migrate to the authenticated
guest-control path (`GuestControlHealth` / `ProcessRole::GuestControlHealth`)
before adopting v3.

`ProcessRole::Usbip` covers two distinct resource kinds. The long-lived
USBIP backend/proxy (runs continuously) becomes a `Process` resource. Each
per-busid attach or detach operation (one-shot) becomes an `EphemeralProcess`
resource. Both are owned by `Provider/device-usbip`. The `Process` controller
starts `EphemeralProcess` instances for individual attach/detach operations;
they are not polymorphic on a single resource instance.

### Capability enum disposition

Current source: `packages/d2b-realm-core/src/capability.rs`, `Capability`
enum. Current capabilities are positive-assertion advertisement values.

The `Capability` enum must not be preserved as a Nix option or ResourceType
field merely because of the name. Each value must be individually classified.

| Current `Capability` value | Current purpose | v3 target |
| --- | --- | --- |
| `Lifecycle` | Workload create/start/stop/inspect | Implicit grant from `Role` binding with `Host`/`Guest` verbs |
| `Exec` | Command execution (admin-only) | Role verb `exec` on `Process` or `EphemeralProcess` |
| `Pty` | Interactive pseudo-terminal | Role verb on `Process`/`EphemeralProcess`; exact verb name per resource-api/authz foundation spec |
| `Logs` | Durable execution logs with cursors | Role verb on `EphemeralProcess`; exact verb name per resource-api/authz foundation spec |
| `FileCopy` | Bounded file copy | Not in initial verb set; reimplemented as Volume view copy op if needed |
| `PortForward` | One stream per connection | Not in initial verb set |
| `PersistentShell` | Named shell operations | Service capability of `Provider/shell-terminal` |
| `Vsock`, `Virtiofs` | Transport availability | Provider descriptor capability field (not a Role verb) |
| `WindowForwarding`, `DisplayStreaming`, `Clipboard` | Display/clipboard transport | Provider descriptor capability field for `Provider/display-wayland`/`Provider/clipboard-wayland` |
| `AudioPlayback`, `AudioCapture` | Audio | Provider descriptor capability field for `Provider/audio-pipewire` |
| `Hid` | Named HID device operations | Provider descriptor capability field for security-key Provider |
| `Usb` | Named USB device operations | Provider descriptor capability field for `Provider/device-usbip` |
| `GpuAccel` | Local GPU acceleration | Provider descriptor capability field for `Provider/device-gpu` |
| `Snapshots` | Snapshots | Typed Provider descriptor capability field; absent means fail closed; no Nix option |
| `Hotplug` | Device hotplug | Typed Provider descriptor capability field; absent means fail closed; no Nix option |
| `EphemeralSessions` | Provider-managed ephemeral sessions | Provider descriptor capability field; not a Role verb |
| `ProviderManagedIsolation` | Non-host-owned isolation boundary | Typed Provider descriptor capability field; absent means fail closed; no Nix option |
| `ConfiguredLaunch` | Execute a configured launcher item | Role verb on `EphemeralProcess` or `Process`; exact verb name per resource-api/authz foundation spec |

The verb set for `Role.rules[*].verbs` is owned by the resource-api and authz
foundation spec. The compiler validates that every declared verb is in the
closed set published by that spec; any verb not in that set is rejected at eval
time. Core-reserved verbs (verbs bound to internal controller identity and not
grantable to operator principals) cannot be declared in Nix RoleBindings; the
compiler rejects them with a structured eval error. Cross-reference:
`ADR-046-resource-api-and-authz`.

`Capability::Snapshots`, `Hotplug`, and `ProviderManagedIsolation` are retained
as typed Provider descriptor capability fields, declared in the Provider's
dossier/manifest. A Provider that does not declare one of these fields is
treated as not supporting that capability; dependent resources fail closed
(e.g., a Guest requesting a snapshot on a Provider without `Snapshots` in its
capability descriptor is a config-publication error). No Nix option forces or
defaults these fields; they are set exclusively by the Provider author in the
Provider dossier.

### Current reachability summary

This table records what is wired versus compile-only at baseline `b5ddbed6`.
Evidence class per `ADR-046-terminology-and-identities.md`.

| Component | Reachability at baseline | Evidence source |
| --- | --- | --- |
| `d2b-realm-core` identifiers, routing metadata, capability model | Compiled into `d2bd`; `RealmId`/`WorkloadId`/`RealmPath`/`RealmTarget`/`CapabilitySet` used in live access-resolver and identity-config paths | `packages/d2bd/src/realm_access_resolver.rs`, `lib.rs` |
| `d2b-realm-core` pure engines (`allocator_engine.rs`, `route_engine.rs`, `identity_store.rs`) | Compile-tested; pure in-memory — no live mutation | Inline module docs: "performs no netlink… live host mutation" / "never generates keys, signs data, writes files" |
| `d2b-realm-provider` `RuntimeProvider`/`WorkloadProvider`/`HostSubstrateProvider` traits | Implemented by live local-vm/ACA providers wired into `d2bd` | `packages/d2bd/src/lib.rs`: `WorkloadProvider` used; `realm_stubs.rs` documents that future gateway wiring is NOT called |
| `d2b-realm-router` `OperationRouter`, `DurableExecTable` | Imported by `d2bd` via `Cargo.toml` but used ONLY in `realm_stubs.rs` which is `dead_code`-allowed and explicitly "NOT called from the running daemon" | `packages/d2bd/src/realm_stubs.rs` header comment |
| `d2b-realm-transport` `LocalTcpTransport`, loopback | Compile-check and conformance tests only; no live socket opened | `packages/d2b-realm-transport/src/lib.rs` docs: "loopback connects… no real socket is opened" |
| `d2b-realm-codec-protobuf` | Compile-check only at baseline (no caller found in daemon) | `packages/d2b-realm-codec-protobuf/src/lib.rs` |
| `realm_access_resolver.rs` in `d2bd` | Live and wired: reads `/etc/d2b/realm-controllers.json`, resolves `RealmPath`/`WorkloadId` targets for auth | `packages/d2bd/src/realm_access_resolver.rs` |
| `d2b-realm-core` `RealmIdentityConfigJson`/`RealmIdentityConfigSummary` | Live: loaded by `d2bd` from `/etc/d2b/realm-identity.json` | `packages/d2bd/src/lib.rs`: `DEFAULT_REALM_IDENTITY_CONFIG_PATH` |
| ACA/Relay/gateway Providers | Live paths exist (implemented `WorkloadProvider`); not covered further in this spec | `d2b-realm-provider` implementors wired into `d2bd` |
| Per-VM process DAG (`VmProcessDag`, `ProcessNode`, `ProcessRole`) | Live: generated by Nix, read by broker/daemon at runtime | `packages/d2b-core/src/processes.rs`; `nixos-modules/processes-json.nix` |
| `StorageJson` / `SyncJson` / `AllocatorJson` | Live: generated by Nix, read by broker | `nixos-modules/storage-json.nix`, `sync-json.nix`, `allocator-json.nix` |
| `RealmWorkloadsLauncherV2Json` | Live: generated by Nix, read by launcher | `packages/d2b-core/src/realm_workloads_launcher.rs`; `nixos-modules/realm-workloads-launcher-v2-json.nix` |

## Zone declaration

Each Zone is declared under `d2b.zones.<name>`. The option path replaces
`d2b.realms.<r>` (current: `nixos-modules/options-realms.nix`). The Zone
`<name>` matches `^[a-z][a-z0-9-]*$` — the same LABEL_PATTERN as `RealmId`.

```nix
d2b.zones.dev = {
  label = "Development";   # optional; defaults to attribute name
};
```

Zone-level Nix options (`label`, `retainedGenerations`, `trustedPublishers`) are
compiler/configuration-service settings. They control build behavior and are
not emitted into the Zone ResourceSpec. Parent/child hierarchy is represented
exclusively by a ZoneLink resource in the parent Zone; there is no `parentRef`
in the Zone spec.

Every declared Zone compiles to exactly one `Zone/<name>` self-resource with
an empty spec:

```yaml
apiVersion: resources.d2bus.org/v3
type: Zone
metadata:
  name: dev
  zone: dev
  uid: <store-generated at first activation>
  generation: 1
  revision: <opaque>
  ownerRef: null
  finalizers: []
spec: {}
status:
  phase: Ready
```

`uid` is never specified in Nix. It is assigned by the Zone runtime at first
activation. A Nix redeclaration of the same name must not invent a new UID.

Zone UIDs are assigned by the Zone runtime store at first activation. Nix
binds a Zone exclusively by its declared name and config identity. A UID is
never pinned, predicted, or validated in Nix; bundle manifests bind Zone
_name_ only and UIDs are resolved at runtime. A Nix redeclaration of an
existing Zone name must not invent or specify a UID. The
`d2b-activation-helper` reads the runtime-assigned UID from the Zone store
after first activation; subsequent generations are linked to that store-held
UID automatically.

### ZoneLink

A `ZoneLink` resource connects a parent Zone to a child Zone. Because a
ZoneLink lives in the PARENT Zone's resource bundle, it is declared in the
parent zone's `resources` set. Parent/child relationships are represented
only by the parent-local ZoneLink; there is no `parentRef` in any Zone's spec
or cross-Zone ResourceRef. Current source for zone-hierarchy:
`d2b-realm-core/src/realm.rs` (`RealmPath` dotted hierarchy — e.g.,
`work.personal`); `d2b.realms.<r>.parent` option in
`nixos-modules/options-realms.nix`.

```nix
# Declared in the PARENT zone's resources:
d2b.zones.root.resources.link-dev = {
  type = "ZoneLink";
  spec = {
    childZoneName         = "dev";                      # plain Zone name, not a ResourceRef
    transportProviderRef  = "Provider/transport-unix";  # always explicit; no default
  };
};
```

An eval assertion verifies that for every `ZoneLink` resource in
`d2b.zones.<p>.resources` with `spec.childZoneName = "<z>"`, the Zone `<z>` is
declared in `d2b.zones`. There is no bidirectional `parentRef` requirement on
the child Zone; the ZoneLink in the parent is the sole declaration.

`ZoneLink.spec.transportProviderRef` is always explicit. There is no default
transport and no inference. Omitting `transportProviderRef` from a `ZoneLink`
spec is a structured eval error. The operator must select a declared Provider
(e.g., `Provider/transport-unix` for local-host ZoneLinks or
`Provider/transport-vsock` for cross-host links).

Compiled ZoneLink bundle entry:

```yaml
apiVersion: resources.d2bus.org/v3
type: ZoneLink
metadata:
  name: link-dev
  zone: root
  ownerRef: null
spec:
  childZoneName:        dev
  transportProviderRef: Provider/transport-unix
```

## Artifact catalog

Derivation-valued inputs — Provider package closures, NixOS system closures,
and any other Nix build output a resource spec must reference — are declared
in a separate named catalog. ResourceSpecs remain pure schema mirrors containing
only plain bounded string IDs; no embedded Nix derivation handles or
convenience wrappers appear inside any spec field.

### Declaration

```nix
d2b.artifacts.<id> = {
  package = <derivation>;                      # required Nix derivation
  type    = "provider"                         # closed type tag
           | "nixos-system"
           | "nixos-module-set"
           | "config-bundle";
};
```

`<id>` follows `^[a-z][a-z0-9-]*$`. Two entries sharing the same `<id>` is a
NixOS eval error (Nix attrset uniqueness). A spec field referencing an `<id>`
absent from `d2b.artifacts` fails the NixOS build. A spec field referencing
an `<id>` whose `type` does not match what the field expects (e.g., a
`"provider"` artifact in a `systemArtifactId` field that expects
`"nixos-system"`) fails the build with a structured type-mismatch error.

`Artifact` is not a ResourceType; artifact IDs are NOT ResourceRefs. Fields
that hold artifact IDs use plain unambiguous names (`artifactId`,
`systemArtifactId`) with no `*Ref` suffix.

Example declarations:

```nix
d2b.artifacts = {
  system-core            = { package = pkgs.d2b-provider-system-core;              type = "provider"; };
  system-systemd         = { package = pkgs.d2b-provider-system-systemd;            type = "provider"; };
  runtime-ch             = { package = pkgs.d2b-provider-runtime-cloud-hypervisor;  type = "provider"; };
  dev-vm-system          = { package = pkgs.nixosSystem { modules = [ ... ]; };     type = "nixos-system"; };
};
```

### Emitted artifact catalog

The NixOS build emits a private, integrity-pinned `artifact-catalog.json`
(installed `root:d2bd` 0640) alongside the Zone bundles. Store paths are
restricted from PUBLIC ResourceSpecs, status fields, audit records, and OTEL
telemetry. The private `artifact-catalog.json` must contain sufficient Nix
store location data — directly as `storePath` fields, or via a private
integrity-bound locator — for `d2b-activation-helper` to resolve and stage
each built artifact:

```json
{
  "schemaVersion": "v1",
  "entries": [
    {
      "artifactId":    "system-core",
      "type":          "provider",
      "storePath":     "/nix/store/aabbcc...-d2b-provider-system-core",
      "packageDigest": "sha256:aabbcc...",
      "closureDigest": "sha256:ddeeff...",
      "closureSize":   12345678
    },
    {
      "artifactId":    "dev-vm-system",
      "type":          "nixos-system",
      "storePath":     "/nix/store/112233...-nixos-system",
      "packageDigest": "sha256:112233...",
      "closureDigest": "sha256:445566...",
      "closureSize":   876543210
    }
  ]
}
```

`storePath` is a private field read only by `d2b-activation-helper` and the
Zone runtime for staging. It never appears in any public ResourceSpec, status
field, audit record, or OTEL telemetry export. `d2b-activation-helper` verifies
the artifact catalog digest against a `bundle.json`-sibling manifest entry
before staging. The catalog is content-addressed: same derivation inputs →
byte-identical output.

### Validation

| Rule | Layer |
| --- | --- |
| `<id>` matches `^[a-z][a-z0-9-]*$` | Eval |
| No duplicate `d2b.artifacts.<id>` keys | Eval (Nix attrset uniqueness) |
| Every `artifactId` / `systemArtifactId` in any spec exists in `d2b.artifacts` | Build |
| `type` of the artifact matches the expected type for the spec field | Build |
| Provider `artifactId` has `type = "provider"`; trust root validated | Build |
| `systemArtifactId` / `source.systemArtifactId` has `type = "nixos-system"` | Build |
| Store paths absent from all public ResourceSpecs, status fields, audit records, and OTEL telemetry | Build/Runtime |

## Provider package catalog

The catalog replaces the ad hoc per-crate package outputs in `flake.nix` and
`nixos-modules/host-daemon.nix`. Current provider construction is implicit
(direct Rust construction in `d2bd`). The v3 catalog is an offline sorted
exact-digest artifact.

### Catalog declaration

```nix
d2b.providerCatalog = {
  system-core = {
    artifactId = "system-core";   # must exist in d2b.artifacts with type = "provider"
    trust      = { publisherRef = "d2b-official"; };
  };
  system-systemd = {
    artifactId = "system-systemd";
    trust      = { publisherRef = "d2b-official"; };
  };
  runtime-cloud-hypervisor = {
    artifactId = "runtime-ch";    # artifact ID may differ from catalog entry name
    trust      = { publisherRef = "d2b-official"; };
  };
  # ... one entry per Provider in the frozen initial catalog
};
```

### Catalog artifact

Emits `/etc/d2b/provider-catalog.json` (sorted by `providerName`). The
`artifactId` field links each Provider catalog entry to the corresponding
artifact catalog entry. `packageDigest` and `settingsSchemaDigest` are
populated by the build from the artifact catalog and Provider package closure.
Store paths do not appear in this public catalog; staging data lives in the
private `artifact-catalog.json`:

```json
{
  "schemaVersion": "v1",
  "entries": [
    {
      "providerName":         "system-core",
      "artifactId":           "system-core",
      "packageDigest":        "sha256:aabbcc...",
      "executableDigest":     "sha256:...",
      "manifestDigest":       "sha256:...",
      "settingsSchemaDigest": "sha256:...",
      "publisherRef":         "d2b-official",
      "systems":              ["x86_64-linux"],
      "apiMajor":             3,
      "apiMinor":             0
    }
  ]
}
```

The trust root model uses a built-in `d2b-official` signing root embedded in
the Nix module (covers all initial catalog entries). Additional trusted
publisher roots are per-Zone Nix compiler settings, not Zone ResourceSpec
fields. They are declared at the Zone option level and consumed only during
the NixOS build:

```nix
d2b.zones.dev.trustedPublishers = {
  acme-corp = { signingKey = "<PEM-encoded public key>"; };
};
```

A Provider entry whose `publisherRef` is not the built-in `d2b-official` root
and is not registered in `d2b.zones.<z>.trustedPublishers` fails install
closed. An absent or unrecognized `publisherRef` is never a warning; it is a
hard failure at catalog resolution time.

### Provider install resource

Catalog presence does not install a Provider. Each Zone declares installed
Providers separately:

```nix
d2b.zones.dev.resources.system-core = {
  type = "Provider";
  spec = {
    artifactId = "system-core";   # plain bounded ID; must exist in d2b.artifacts with type="provider"
    config     = {};              # validated against Provider's signed settings schema; no raw host paths
  };
};
```

Compiles to a `Provider` resource in that Zone's generated bundle:

```yaml
apiVersion: resources.d2bus.org/v3
type: Provider
metadata:
  name: system-core
  zone: dev
  ownerRef: null
spec:
  artifactId: system-core
  config:     {}
```

`artifactId` is a plain bounded string (not a ResourceRef; `Artifact` is not a
ResourceType). The build resolves `artifactId` against `d2b.artifacts` (type
must be `"provider"`) and `d2b.providerCatalog` (trust validation); the catalog
is frozen with no `ProviderCatalogEntry` ResourceType. `packageDigest` and other
manifest-derived fields are private compiled catalog metadata and are not part of
the Provider ResourceSpec. `status` is read-only runtime state and is never
emitted in Nix-authored bundles.

`config` values are validated at build time against the Provider's signed settings
schema. `config` must not contain raw host paths, Nix store path strings, or any
opaque filesystem references — all derivation outputs and volume source policies
are expressed as artifact IDs or opaque source policy IDs, not literal paths.

Core `ProviderDeployment` reads the signed manifest/catalog selected by
`artifactId` and creates the Provider's static controller/service `Process`
graph. Provider controllers never bootstrap their own controller/service
Processes; they may only create authorized dynamic child resources at runtime.

### Provider crate layout

Every `packages/d2b-provider-<base>-<implementation>/` Rust crate in the
workspace must contain exactly the following four paths. Missing any path — or
having an empty `tests/` or `integration/` directory with no Rust source file —
fails the workspace policy check (`make test-policy`):

| Path | Required contents |
| --- | --- |
| `src/` | Crate implementation source and compiled binaries (one per process role declared in the Provider dossier). Colocated `#[cfg(test)]` unit tests are permitted here. No integration or provider-system tests. |
| `tests/` | Hermetic Cargo integration tests: ResourceType schema round-trips; controller/service/worker lifecycle; conformance via `d2b-provider-toolkit::conformance` (all declared provider-type axes); fault injection. No container, Host, Guest, or cross-process fixtures. |
| `integration/` | Heavier fixtures and scenarios requiring container launch, Host/Guest interaction, cross-process rendezvous, or provider-system state. Invoked by existing test orchestration (`make test-integration`, `make test-host-integration`). Must contain at least one `.rs` source file or fixture. |
| `README.md` | Provider identity and config schema; declared ResourceTypes and their spec/status fields; controllers, services, workers, and binaries with their process roles; placement constraints; dependencies and RBAC requirements; security posture, state lifecycle, and telemetry labels/cardinality; build, test, and integration commands; future standalone-repo usage notes. Minimum 200 bytes. |

The workspace policy test
(`packages/d2b-contract-tests/tests/provider-crate-layout.rs`) scans every
workspace member matching `packages/d2b-provider-*-*` and fails for any
missing or empty path. The gate runs as part of `make test-policy`.

Every work item in any spec that introduces a new
`packages/d2b-provider-<base>-<implementation>/` crate must:
- list all four required paths in its `Destination` field;
- include the layout policy gate (`make test-policy`) in its `Tests` field; and
- include a `README.md` stub commit in its first commit before any other
  implementation lands.

### Provider state Volumes (optional)

A `ProviderStateSet` is an optional, query-time logical grouping of the
*declared* `Volume` resources owned by a `Provider`. It is not a `ResourceType`,
is never stored as its own artifact or row, and is empty for a `Provider` that
declares no state Volume.

Bounded non-secret operational state belongs in the owning resource's `status`
subresource and the core Operation ledger by default (D087). A component
declares a state `Volume` only when a payload passes the storage-need test
(secret/sensitive private recovery data; large/binary/file content; private
data unsafe for status readers; or bounded-but-revision-unsuitable data with a
demonstrated recovery need). Core `ProviderDeployment` creates the static
controller/service `Process` resources and, before launching each component
`Process`, creates one private, framework-created state `Volume` for every
*declared* `stateNamespaces` entry in that component's signed state
declarations. A stateless component declares no namespace and receives no
`Volume`; there is no empty identity-only state `Volume`. Provider controllers
never bootstrap their own controller/service Processes or their own declared
per-component state Volumes.

Each declared state Volume uses the canonical full `Volume` schema, extended for
provider state with `stateSchema`, `persistenceClass`, `sensitivityClass`,
`quotaBytes`, `quota.maxBytes`, `quota.maxInodes`, `identityMarker`, and sealing
fields. Every declared state Volume has `kind = "state"` and
`persistenceClass = "persistent"`. `source.settings.sourcePolicyId` is always
set to the component's opaque private storage policy ID. `quotaBytes` must be
strictly greater than zero; `quota.maxBytes` and `quota.maxInodes` must also be
strictly greater than zero. Declaring `quotaBytes = 0` for a state namespace is
rejected at build and admission time. Layout principals use `User/<name>` refs
drawn from bounded, Nix-preprovisioned pools, and Nix must provision the
referenced `User` resources in advance.

Nix never authors runtime state Volumes. Nix does not declare them, does not
allocate their internal paths, and does not manage their runtime-only
identities; Core creates only the signed *declared* optional state Volumes at
runtime. Each component mounts only its own declared state view through its
`mounts`; there is no separate non-Volume compartment concept.

There is no bootstrap state-Volume exception. The fixed bootstrap components
(the first `volume-local` controller instance on each execution target, and
where present `system-core` and `system-minijail`) declare no state Volume and
reach Ready from resource `status`, the core Operation ledger, and independent
external observation (D086, superseded by D087). Because no component needs a
state Volume before a `volume-local` instance is Ready, there is no closed
bootstrap cycle, no per-execution-target local bootstrap storage mechanism, and
no hidden bootstrap store. A Guest still bootstraps its own Guest-local
`volume-local` instance from Guest-local primitives only, never a leaked
parent-Host dirfd or resource handle.



`Host` is a new ResourceType; there is no current equivalent. The closest
current concepts are `NodeKind::FullHost` (`d2b-realm-core/src/node.rs`),
`RealmControllerPlacement::HostLocal` (`d2b-realm-core/src/realm.rs`), and
the per-realm path-partition in `allocator-json.nix`. The option
`d2b.realms.<r>.placement = "host-local"` is the current declaration of a
host-resident realm.

```nix
d2b.zones.dev.resources.host-system = {
  type = "Host";
  spec = {
    providerRef    = "Provider/system-core";
    defaultDomain  = "system";
    allowedDomains = ["system" "user"];
    budget         = { cpu = {}; memory = {}; pids = {}; fds = {}; io = {}; storage = {}; network = {}; };
  };
};
```

Compiles to:

```yaml
apiVersion: resources.d2bus.org/v3
type: Host
metadata:
  name: host-system
  zone: dev
  ownerRef: null
spec:
  providerRef:     Provider/system-core
  defaultDomain:   system
  allowedDomains:  [system, user]
  defaultUserRef:  null
  budget:          { ... }
  networkAttachments: []
  deviceAttachments:  []
  volumeDefaults:     {}
```

### Unsafe-local Host

The current `d2b.realms.<r>.workloads.<w>.kind = "unsafe-local"` workload
(current type: `WorkloadProviderKind::UnsafeLocal`, current isolation:
`IsolationPosture::UnsafeLocal`, current files:
`nixos-modules/unsafe-local-workloads-json.nix`,
`nixos-modules/unsafe-local-helper.nix`,
`packages/d2b-core/src/unsafe_local_workloads.rs`) maps to a user-only `Host`
resource in v3. It is never a `Guest` and is not a v3 Provider.

The target shape is a `Host` resource reconciled by `Provider/system-core` with
`defaultDomain: user`, `allowedDomains: [user]`, and `defaultUserRef: User/<name>`.
Child processes are normal `Process` resources selecting any installed Process
Provider (`Provider/system-systemd`, `Provider/system-minijail`). No special
Provider or process type exists for unsafe-local execution.

The no-isolation posture is preserved explicitly across all surfaces:

- **Host status** — `Provider/system-core` sets a stable `NoIsolation` condition
  on `Host` status. The condition is present whenever
  `spec.providerSettings.isolationPosture` is `"none"`; it is never absent or
  cleared by a later upgrade.
- **CLI/UI** — `d2b host inspect` and any status display always render the
  no-isolation warning when the Host carries the `NoIsolation` condition.
  The warning text is not suppressible by operator flag.
- **Audit** — every `ProcessEffect` audit event under this Host carries a
  closed boolean field `no_isolation=true`. No isolation label appears in OTEL
  telemetry attributes.

```nix
d2b.zones.dev.resources.host-unsafe-local = {
  type = "Host";
  spec = {
    providerRef    = "Provider/system-core";
    defaultDomain  = "user";
    allowedDomains = ["user"];
    defaultUserRef = "User/alice";
    providerSettings = {
      isolationPosture = "none";   # declared in Host schema; required for user-only
                                   # system-core Hosts; cannot be set to any other value
    };
  };
};
```

Eval assertions:

- `providerSettings.isolationPosture` set to any value other than `"none"` for a
  user-only (`defaultDomain: user`, `allowedDomains: [user]`)
  `Provider/system-core` Host is rejected at eval time.
- A Process with `executionRef: Host/host-unsafe-local` and `domain: system`
  is rejected at eval time.
- No `Guest` ref is emitted for an unsafe-local declaration.

`providerSettings.isolationPosture: "none"` is declared in the `Host`
ResourceTypeSchema under `providerSettings` (visible to all controllers and
inspectable via the resource API). It is not a freeform field; the schema
restricts it to the bounded enum. `status.isolationPosture` reflects `none`
when `spec.providerSettings.isolationPosture` is `none`. The `NoIsolation`
status condition and the `no_isolation=true` `ProcessEffect` audit field are
required whenever this posture is in effect; OTEL telemetry never carries
an isolation label.

## Guest resource

`Guest` is the v3 successor to `d2b.vms.<vm>` (current: `nixos-modules/options-vms.nix`)
for NixOS VM and QEMU media workloads, and to `d2b.realms.<r>.workloads.<w>`
with `kind = "local-vm"` or `"qemu-media"` (current:
`nixos-modules/options-realms-workloads.nix`). The current `WorkloadId` maps
to `Guest/<name>` and the current `RealmTarget` address
`<WorkloadId>.<RealmPath>.d2b` maps to `Zone/<zone>/Guest/<name>`.

```nix
d2b.zones.dev.resources.dev-vm = {
  type = "Guest";
  spec = {
    providerRef        = "Provider/runtime-cloud-hypervisor";
    systemArtifactId   = "dev-vm-system";  # plain bounded ID; type="nixos-system" in d2b.artifacts
    defaultDomain      = "system";
    allowedDomains     = ["system"];
    budget             = { cpu = { cores = 4; }; memory = { bytes = 4294967296; }; };
    networkAttachments = [{ networkRef = "Network/dev-lan"; }];
    deviceAttachments  = [{ deviceRef = "Device/dev-tpm"; }];
    # Canonical selected-Provider extension envelope (D089): validated against the
    # Provider named by providerRef. No raw host paths; named closure-entry IDs only.
    provider = {
      schemaId      = "runtime-cloud-hypervisor.d2bus.org/Guest/spec";
      schemaVersion = "1.0";
      settings = {
        vsockCid      = 42;
        memoryBacking = "shared";
      };
    };
  };
};
```

The canonical `spec.provider = { schemaId, schemaVersion, settings }` envelope
(D089) replaces the former Host/Guest/Credential `providerSettings` and Device
`settings`; `spec.providerRef` and every other `spec.*` field is ResourceType
base. Mapping convention: a reference to `spec.providerSettings` (or Device's
former `spec.settings`) denotes `spec.provider.settings`. The `settings` object
is declared by the selected Provider's signed, versioned, deny-unknown JSON
Schema and validated against `spec.providerRef` at eval time and at API
admission; it may not shadow a base field, and any field shared across
implementations is promoted to the ResourceType base rather than carried here.
Values that reference Nix derivation outputs (e.g., a closure, a kernel module
path) are serialized as named closure-entry identifiers or content digests
validated against the package manifest. Raw Nix store path strings (e.g.,
`/nix/store/<hash>-foo`) are rejected at eval time and must never appear in
emitted resource spec JSON.

**Update policy and currency triggers (D091).** The base spec carries a
provider-neutral `spec.updatePolicy` (disruptive upgrades default to `manual`;
automatic application of non-disruptive changes MAY be permitted); a
`spec.provider` extension may add implementation knobs but cannot bypass it.
Selecting a new artifact/image ID, a new `systemArtifactId` NixOS generation, or
a new Provider package generation in Nix is a currency trigger: activation
records the observed/target generation/digest IDs, and the owning controller's
`assess_update` reports `status.update.state = UpgradeRequired` (with the
matching closed-enum reason) rather than applying a disruptive change in place.
The operator applies it with `d2b upgrade <ref> --apply` (optionally
`--recursive`).

**Endpoint resources (D092).** A stable endpoint is the `Endpoint` ResourceType,
never an inline field. Core creates a component's static `Endpoint` resources
from the signed manifest's Endpoint templates, and dynamic controllers create
their owned Endpoints; a static `Endpoint` may be Nix/API-authored only where
the ResourceType schema permits. An authored `Endpoint` spec carries only closed
`endpointClass`/`transport`/`locality`/`purpose` values and bounded fingerprints
— never a raw path/address/CID/port/fd/credential (those resolve privately
through the effect adapter at runtime). `ProcessSpec` carries no inline
`endpoints`; a Process's stable endpoints are owned `Endpoint` resources with
`producerRef`, and consumers reference `Endpoint/<name>`.

**Entra identity Guest composition (D093).** `credential-entra` login/token
acquisition is grounded in an Entrablau-enabled identity `Guest`, never a Host
login. The consumer declares the identity Guest (with a `systemArtifactId` whose
NixOS system composes `inputs.entrablau.nixosModules.default`), installs/binds
the login-service `Endpoint`, and declares the `Credential` with
`identityGuestRef`/`loginEndpointRef`/`consumerRef`:

```nix
# Identity Guest whose NixOS system composes the sibling Entrablau module.
# d2b core never imports entrablau; the consumer flake does.
d2b.zones.work.resources.entra-identity = {
  type = "Guest";
  spec = {
    providerRef      = "Provider/runtime-cloud-hypervisor";
    systemArtifactId = "entra-identity-system";   # its NixOS system imports inputs.entrablau.nixosModules.default
    defaultDomain    = "system";
    allowedDomains   = ["system"];
  };
};

# The login/token service Endpoint the Entrablau service produces inside the Guest.
d2b.zones.work.resources.entra-login = {
  type = "Endpoint";
  spec = {
    providerRef  = "Provider/credential-entra";
    producerRef  = "Guest/entra-identity";        # produced inside the identity Guest
    endpointClass = "service";
    transport    = "vsock";                        # closed class only; no CID/path
    purpose      = "entra-login-token";
    locality     = "guest-local";
  };
};

d2b.zones.work.resources.work-entra = {
  type = "Credential";
  spec = {
    providerRef      = "Provider/credential-entra";
    identityGuestRef = "Guest/entra-identity";
    loginEndpointRef = "Endpoint/entra-login";
    audience         = "api://work-graph";         # non-secret audience only
    consumerRef      = "Provider/runtime-azure-container-apps";
    allowedOperations = ["acquire-token"];
    rotation = { policy = "on-demand"; };
  };
};
```

Build/eval validation asserts: `identityGuestRef`, `loginEndpointRef`,
`consumerRef`, and `scope.executionRef` are all the same Zone; the `Endpoint`
`purpose`/schema and `providerRef` match the credential-entra login-service
contract; the Endpoint is Guest-placed (Host placement is rejected); the
Credential scope and consumer placement are consistent; and no store path or
token appears in any emitted spec.

### Package closures into Guests

Current source: `nixos-modules/closures-json.nix` — `pkgs.closureInfo` per VM,
outputs `/etc/d2b/closures/<vm>.json`. Current `VmProcessDag.nodes` contains
a `ProcessRole::Virtiofsd` node whose `share.source` sentinel
`"/nix/store"` (string literal) is the eval-time marker for the per-VM
hardlink-farm share path; see `nixos-modules/processes-json.nix` and
`nixos-modules/store.nix`.

In v3, every Guest that runs a closure-based OS pins its system artifact in
the artifact catalog, then references it by plain ID in the Guest spec:

```nix
# Step 1: declare the NixOS system derivation in the artifact catalog
d2b.artifacts.dev-vm-system = {
  package = pkgs.nixosSystem { modules = [ ... ]; };
  type    = "nixos-system";
};

# Step 2: reference it by ID in the Guest spec (mirrors the canonical schema)
d2b.zones.dev.resources.dev-vm = {
  type = "Guest";
  spec = {
    providerRef      = "Provider/runtime-cloud-hypervisor";
    systemArtifactId = "dev-vm-system";   # validated type="nixos-system" at build
    # ... other spec fields
  };
};
```

The compiler:

1. Resolves `systemArtifactId = "dev-vm-system"` from `d2b.artifacts`; validates
   `type = "nixos-system"`.
2. Computes `pkgs.closureInfo { rootPaths = [artifacts.dev-vm-system.package]; }`.
3. Emits a `Volume/<guest-name>-nix-store` resource with
   `source.kind = "nix-closure"` and `source.systemArtifactId = "dev-vm-system"`.
4. Emits a virtiofs attachment from that Volume to the Guest with
   `mountPath: /nix/store`.
5. Records closure digest, closure size, and private store path in the artifact
   catalog entry for `"dev-vm-system"`. The store path is a private field in
   `artifact-catalog.json`; it is never emitted in public ResourceSpecs, status
   fields, audit records, or OTEL telemetry.

The per-VM hardlink farm path is derived by `Provider/volume-virtiofs` at
runtime from the artifact catalog entry and the Zone's `stateDir`; it never
appears in any resource spec or status field. The current `share.source ==
"/nix/store"` sentinel in `nixos-modules/processes-json.nix` is the eval-time
equivalent; its exact migration mapping is covered in ADR046-nix-017.

## Process and EphemeralProcess

Current source: `packages/d2b-core/src/processes.rs` — `ProcessesJson`,
`VmProcessDag`, `ProcessNode`, `ProcessRole`. The per-VM DAG drives
`nixos-modules/processes-json.nix` which emits `/etc/d2b/processes.json`.

### Common spec

```nix
d2b.zones.dev.resources.wayland-proxy = {
  type = "Process";
  spec = {
    providerRef  = "Provider/system-systemd";   # replaces ProcessRole + minijail profile selection
    executionRef = "Host/host-system";          # replaces per-VM DAG node host/VM assignment
    domain       = "user";
    userRef      = "User/alice";
    processClass = "service";
    packageRef   = "Provider/display-wayland";  # replaces binaryPath in ProcessNode
    template     = "wayland-proxy-host";        # replaces ProcessRole for template dispatch
    configRef    = "Volume/wayland-proxy-config";
    mounts = [
      { volumeRef = "Volume/wayland-proxy-state";
        view      = "proxy";
        mountPath = "/state";
        access    = "read-write"; }
    ];
    budget  = { memory = { bytes = 134217728; }; };
    sandbox = {
      # Named profile only — replaces raw seccomp program and capability list.
      seccompProfile = "system-systemd-default";
      capabilities   = [];
      noNewPrivs     = true;
    };
    network   = { networkRef = null; ports = []; };
    devices   = [];
    endpoints = [{ name = "wayland-host-socket"; transport = "unix"; }];
    readiness = { kind = "unix-socket-accept"; };
    restart   = { policy = "on-failure"; maxRestarts = 5; backoffMs = 1000; };
  };
};
```

### Prohibited fields

The following are never accepted in any Process or EphemeralProcess Nix
declaration. They replace the free-form fields that current `ProcessNode`
carries (current: `unit`, `binary_path`, `argv`, numeric UID/GID in sandbox):

- Raw executable path (current `ProcessNode.binary_path` — now resolved by
  Provider from signed package manifest);
- Raw environment variable map (current `SpawnRunnerPlanOp` env fields);
- Numeric UID or GID (now `userRef: User/<name>`);
- Raw seccomp BPF program (now named profile ref);
- Ambient capability bitmask;
- Arbitrary socket path;
- Credential or secret bytes.

### EphemeralProcess

Current source: `ProcessRole::StoreVirtiofsPreflight`, `SwtpmPreStartFlush`,
`GuestControlHealth`, `GuestSshReadiness` (disposition table above).

```nix
d2b.zones.dev.resources.store-sync-dev-vm = {
  type = "EphemeralProcess";
  spec = {
    providerRef   = "Provider/system-minijail";
    executionRef  = "Host/host-system";
    domain        = "system";
    processClass  = "worker";
    packageRef    = "Provider/volume-virtiofs";
    template      = "store-sync";
    configRef     = "Volume/store-sync-config";
    successfulTtl = "1h";    # default; explicit for clarity
    failedTtl     = "24h";   # default
    startDeadline = "30s";
    mounts = [
      { volumeRef = "Volume/dev-vm-nix-store";
        view = "sync-source"; mountPath = "/source"; access = "read-only"; }
      { volumeRef = "Volume/dev-vm-store-farm";
        view = "sync-target"; mountPath = "/target"; access = "read-write"; }
    ];
  };
};
```

`startDeadline` and `runDeadline` use Go-style bounded duration strings.
Unbounded deadlines are rejected at eval time.

## Volume resource

Current source: `packages/d2b-core/src/storage.rs` — `StorageJson`,
`StorageRoot`, `StoragePathSpec` with `owner`/`group`/`mode`/`accessAcl`/
`defaultAcl`/`noFollow`/`createPolicy`/`repairPolicy`/`cleanupPolicy`.
The current `nixos-modules/storage-json.nix` emitter generates these rows
per VM using the `mkPath` helper. The v3 Volume layout/views replaces this
with a single ResourceType that carries the same fine-grained policy.

```nix
d2b.zones.dev.resources.wayland-proxy-state = {
  type = "Volume";
  spec = {
    providerRef = "Provider/volume-local";
    source = {
      executionRef = "Host/host-system";
      settings = {
        kind           = "local-path";
        sourcePolicyId = "wayland-proxy-state";  # opaque ID resolved through
                                                 # volume-local's private
                                                 # allowedHostPaths catalog
      };
    };
    layout = [
      { path          = "socket-dir";
        type          = "directory";
        ownerRef      = "User/alice";    # replaces PrincipalRef "uid"/"gid" in StoragePathSpec
        groupRef      = "User/alice";
        mode          = "0700";
        noFollow      = true;
        createPolicy  = "create-if-never-provisioned";
        repairPolicy  = "exact-owner";
        cleanupPolicy = "owner-controlled"; }
    ];
    views = {
      proxy = { path = "."; rights = ["read" "write" "create" "delete" "traverse"]; };
    };
    attachments = [];
  };
};
```

For `local-path` and `block-image` sources, `source.settings.sourcePolicyId`
is an opaque bounded ID that references an entry in volume-local's private
`config.allowedHostPaths` catalog. The raw host path is resolved only inside
the private effect-adapter/broker path and never appears in the public `Volume`
spec, status, or audit record.

`layout[*].ownerRef` and `layout[*].groupRef` accept only `User/<name>`
typed Zone refs. Numeric UID/GID strings (e.g., `"1000"`) are rejected at
eval time. There is no legacy numeric ref migration period: current
`PrincipalRef { kind: "uid", value: "..." }` entries are a clean-reset
migration; operators must declare corresponding `User` resources before
migrating storage layout declarations (tracked in ADR046-nix-009).

### Virtiofs Volume

Current source: `ProcessRole::Virtiofsd` in `VmProcessDag`; the current
`share.source` sentinel; `store.nix` for the per-VM hardlink farm.

```nix
d2b.zones.dev.resources.dev-vm-nix-store = {
  type = "Volume";
  spec = {
    providerRef = "Provider/volume-virtiofs";
    source = {
      kind             = "nix-closure";     # desired-state marker
      executionRef = "Host/host-system";
      systemArtifactId = "dev-vm-system";   # references artifact catalog; type="nixos-system"
    };
    views = {
      guest-ro = { path = "."; rights = ["read" "traverse"]; };
    };
    attachments = [
      { executionRef = "Guest/dev-vm";
        transport    = "virtiofs";
        mountPath    = "/nix/store";
        view         = "guest-ro";
        access       = "read-only"; }
    ];
  };
};
```

## Network resource

Current source: `nixos-modules/options-envs.nix` — `d2b.envs.<e>.*` with
`lanSubnet`, `uplinkSubnet`, `mtu`, `mssClamp`, `externalNetwork.*`,
`portForwards`. Current `index.nix` (`netMeta`) computes derived bridge names,
IP addresses, and DHCP ranges from these options. The `lib.mkForce`
neutralizer in `net.nix` is the live DHCP/NAT-prevention mechanism for the
net VM's uplink interface.

```nix
d2b.zones.dev.resources.dev-lan = {
  type = "Network";
  spec = {
    providerRef  = "Provider/network-local";
    lanSubnet    = "10.100.0.0/24";
    uplinkSubnet = "10.100.1.0/30";
    mtu          = 1500;
    mssClamp     = true;
    dhcp         = { enable = true; rangeStart = "10.100.0.200"; rangeEnd = "10.100.0.250"; };
    dns          = { enable = true; };
    nat          = { enable = true; };
    eastWest     = { allow = false; };
    externalNetwork = { enable = false; };
  };
};
```

CIDR ranges must not overlap with any other Network in the same Zone. The eval
assertion is the v3 successor to the current CIDR-overlap assertion in
`nixos-modules/assertions.nix`.

CIDR/ref/conflict validation is dual-layer: (1) a pure Nix eval assertion runs
at `nix flake check` for all statically known inputs — this is the primary
shift-left gate; (2) the configuration-publication controller repeats the same
validation before staging a new generation. Dynamic allocations that are not
known at eval time (e.g., DHCP ranges assigned by an external IPAM system)
are validated only at the runtime layer (2). Both layers are required; neither
may be omitted.

## Device resource

Current sources: `nixos-modules/components/tpm.nix` (swtpm; `ProcessRole::Swtpm`
and `SwtpmPreStartFlush`), `components/usbip.nix` (`ProcessRole::Usbip`),
`components/graphics.nix` (`ProcessRole::Gpu`, `GpuRenderNode`, `Video`).
Current per-VM toggle: `d2b.vms.<v>.tpm.enable`, `d2b.vms.<v>.usbip.*`,
`d2b.vms.<v>.graphics.*`.

```nix
d2b.zones.dev.resources.dev-tpm = {
  type = "Device";
  spec = {
    providerRef    = "Provider/device-tpm";
    stateVolumeRef = "Volume/dev-vm-tpm-state";
    # No raw device path. The Provider enumerates TPM hardware and swtpm state.
  };
};

d2b.zones.dev.resources.dev-usbip = {
  type = "Device";
  spec = {
    providerRef = "Provider/device-usbip";
    deviceRef   = null;   # opaque ref resolved by Provider at runtime
  };
};
```

Raw device node paths (`/dev/tpm0`, hidraw nodes) never appear in emitted
resource specs. The Provider is the sole authority for device enumeration.

## User resource

Current source: `nixos-modules/host-users.nix` — d2b system user/group
declarations; `d2b-realm-core/src/identity_store.rs` — `IdentityStore` owns
UID/session lifecycle metadata (pure in-memory). Current per-realm
`d2b.realms.<r>.allowedUsers`/`allowedGroups` options.

```nix
d2b.zones.dev.resources.alice = {
  type = "User";
  spec = {
    source = "nss";
    groups = [];
  };
};
```

Compiles to:

```yaml
apiVersion: resources.d2bus.org/v3
type: User
metadata:
  name: alice
  zone: dev
  ownerRef: null
spec:
  source:  nss
  groups:  []
status:
  phase:                    Unknown
  observedUid:              null
  observedGid:              null
  observedHome:             null
  sessionManagerAvailable:  false
```

`observedUid` and `observedGid` are status observations, never spec fields.
The current `PrincipalRef { kind: "uid", value: "..." }` form in storage
contracts must not appear in v3 User spec; it is a pre-migration form only.

## Credential resource

Current source: `packages/d2b-realm-core/src/identity_config.rs`
(`RealmIdentityConfigJson`, `IdentityConfigEntry`) — live in `d2bd` at
`/etc/d2b/realm-identity.json`. Current `d2b-realm-provider/src/credential.rs`
(`CredentialProvider` trait, `CredentialPlane`).

```nix
d2b.zones.dev.resources.work-entra = {
  type = "Credential";
  spec = {
    providerRef       = "Provider/credential-entra";
    scope             = {
      executionRef  = "Guest/work-vm";   # optional Host or Guest placement restriction
      domainFilter  = "user";            # optional: system | user
      userRef       = "User/alice";      # required when domainFilter=user
    };
    audience          = "azure-resource-manager";  # Provider-validated opaque audience token; no secrets
    consumerRef       = "Provider/display-wayland"; # optional; restricts acquiring Provider
    allowedOperations = [ "acquire-token" "refresh-token" "revoke-token" ];
    rotation          = {
      policy              = "proactive";
      proactiveWindowMs   = 300000;
      maxLeaseLifetimeMs  = 3600000;
    };
    expiry            = { hardDeadlineMs = 28800000; };
    revocation        = {
      onOwnerDelete         = "immediate";
      onProviderGeneration  = "immediate";
    };
    # providerSettings validated against Provider's signed schema; no secret bytes.
    providerSettings  = { tenantId = "..."; };  # optional; only when Provider schema declares it
  };
};
```

Credential spec fields mirror the `Credential` ResourceTypeSchema exactly:
`providerRef`, `scope.{executionRef,domainFilter,userRef}`, `audience`,
`consumerRef`, `allowedOperations`, `rotation`, `expiry`, `revocation`, and
optional `providerSettings` (permitted when the Provider schema declares it;
validated against signed Provider schema; no secret bytes). No invented fields
(`credentialType`, `ownerRef`, `domain`). The emitted spec never contains key
material, token bytes, or PEM. Current `identity_config.rs` values that carry
credential bytes are forbidden from the emitted spec; they remain inside the
Provider's external secret service.

## Role and RoleBinding

Current source: `packages/d2b-realm-core/src/access.rs` —
`RealmAccessBinding`, `RealmAccessClientBinding`, `RealmAccessClientContract`,
and `CapabilityPreflightStatus` model current access rules. Current live access
resolution is in `d2bd/src/realm_access_resolver.rs`.

```nix
d2b.zones.dev.resources.process-operator = {
  type = "Role";
  spec = {
    rules = [
      { resourceTypes = ["Process"];
        verbs         = ["get" "list" "watch" "update-status"]; }
      { resourceTypes = ["Host" "Guest"];
        verbs         = ["get" "list"]; }
    ];
  };
};

d2b.zones.dev.resources.process-operator-binding = {
  type = "RoleBinding";
  spec = {
    roleRef   = "Role/process-operator";
    subjects  = [
      "Provider/system-systemd"
      "Provider/system-minijail"
    ];
    expiresAt = null;
    narrowing  = null;
  };
};
```

Eval assertions:

- `roleRef` must resolve a declared `Role` in the same Zone.
- Every `subjects` entry must be a `<ResourceType>/<name>` canonical ref string.
  The initial closed subject ResourceType set is: `Zone`, `Provider`, `Host`,
  `Guest`, `Process`, `User`. `Group` is not a subject ResourceType; user group
  membership may narrow User admission at runtime but is not declared as a
  RoleBinding subject.
- A subject referencing a `Provider`, `Host`, `Guest`, `Process`, or `User`
  must resolve a declared resource of that type in the same Zone.
- Verbs must be from the closed set published by the resource-api and authz
  foundation spec; the compiler rejects any verb not in that set at eval time.

The initial closed subject ResourceType set (`Zone`, `Provider`, `Host`,
`Guest`, `Process`, `User`) is extended only by a future foundation spec
update. No `Group` ResourceType exists in the initial set; user group facts
(e.g., supplementary groups) may narrow User admission within the runtime
authz layer but are never RoleBinding subjects.

## Controller placement

Current source: `nixos-modules/options-realms.nix` — `d2b.realms.<r>.providers`
attrset with `kind`, `placement`, `capabilityRefs`, `configRef`,
`providerSpecificPlacement`. The `placementKinds` list
(`host-local`, `gateway-vm`, `cloud-full-host`, `provider-controller`,
`provider-agent`, `provider-specific`) in `options-realms.nix` is the current
list; these collapse into `executionRef: Host/<h>` or `Guest/<g>` per the
NodeKind table above.

In v3, placement bindings are not a generic sibling field of the Provider
ResourceSpec. The Provider ResourceSpec carries only `artifactId` and `config`.
Placement bindings are expressed in one of two ways:

1. **As validated `config` fields.** Where the Provider's signed deployment
   template schema defines placement-related config keys (e.g., which Host or
   Guest to use for each controller or worker process), the operator supplies
   those values inside the `config` object. Their structure, names, and
   validation rules are defined entirely by the Provider's schema — not the
   framework. The Nix build validates `config` against the Provider's signed
   settings schema.

2. **As controller-created Process resources.** At runtime, the Provider
   controller creates the required `Process` resources using the execution
   targets declared in the Zone. Nix does not declare these Process resources
   directly for Provider-internal processes; they are dynamic children managed
   by the Provider controller.

If an operator-supplied `config` key names a placement target (e.g., a Host or
Guest) that no longer matches a declared resource, config validation rejects it
at build time against the Provider's signed schema (which encodes which
`executionRef`-typed fields exist). Removed or renamed config keys produce a
structured build error; there is no warning-only path.

```nix
# Provider with placement expressed as config fields per its signed schema:
d2b.zones.dev.resources.display-wayland = {
  type = "Provider";
  spec = {
    artifactId = "display-wayland";
    config     = {
      # Placement-typed config keys defined by display-wayland's signed schema.
      # Names and types are determined by the Provider, not the framework.
      hostExecutionRef  = "Host/host-system";
      guestExecutionRef = "Guest/dev-vm";
    };
  };
};
```

## Ref validation

All ref validation runs at Nix eval time:

| Rule | Assertion |
| --- | --- |
| ResourceRef format | `<Type>/<name>` where name matches `^[a-z][a-z0-9-]*$` |
| Type exists in catalog | Type is in the closed standard catalog or an approved vendor type |
| Intra-Zone resolution | The named resource is declared in `d2b.zones.<z>.resources` |
| Cross-Zone ref rejection | Any ref containing a Zone component is rejected; no cross-Zone ResourceRef allowed |
| `providerRef` resolution | The named Provider is declared in `d2b.zones.<z>.resources` with `type = "Provider"` |
| `executionRef` resolution | The named Host or Guest is declared in `d2b.zones.<z>.resources` with `type = "Host"` or `"Guest"` |
| `userRef` resolution | The named User is declared in `d2b.zones.<z>.resources` with `type = "User"` |
| `ownerRef` resolution | The owning resource is declared in `d2b.zones.<z>.resources` and is not the resource itself |
| `ownerRef` acyclicity | No chain of `ownerRef`s forms a cycle |
| `roleRef` resolution | The named Role is declared in `d2b.zones.<z>.resources` with `type = "Role"` |
| `subjects` entries | Each entry is a canonical `<ResourceType>/<name>` ref string resolving to a declared resource of the stated type in the same Zone; type must be in the closed subject set |
| `transportProviderRef` resolution | The named Provider is declared in `d2b.zones.<z>.resources` with `type = "Provider"`; required on every ZoneLink; no default |
| `childZoneName` check | The named child Zone is declared in `d2b.zones`; plain Zone name, not a ResourceRef |
| `artifactId` / `systemArtifactId` format | Plain bounded string `^[a-z][a-z0-9-]*$`; not a `<Type>/<name>` ResourceRef; no `*Ref` suffix |
| `catalogEntryId` check | The named entry exists in `d2b.providerCatalog`; resolved to its `artifactId` |

Failed validation emits a structured eval error identifying the exact option
path and rejected value.

Vendor ResourceType names (e.g., `acme.io.Widget`) appearing in any Nix ref
field before the exporting Provider is installed reject at eval time and block
config publication. There is no deferred-warning or allow-with-warning path.
The compiler's closed ResourceType set is extended only when a Provider
declares the type in its installed dossier.

## Nix authoring and validation contract

### Universal resource spec shape

Every Zone resource is declared under the unified `d2b.zones.<zone>.resources`
attribute set using a `type`/`spec` envelope that mirrors the canonical
ResourceSpec JSON schema directly:

```nix
d2b.zones.<zone>.resources.<name> = {
  type = "<ResourceType>";     # string discriminator matching a known ResourceType
  spec = {
    # Exact ResourceType spec fields — identical names to the canonical JSON schema.
    # No field renaming; no parallel bespoke vocabulary.
    providerRef = "Provider/...";
    # ...
  };
};
```

`metadata.name` is derived from the attribute key (`<name>`).
`metadata.zone` is derived from the enclosing zone attribute key (`<zone>`).
`apiVersion` is defaulted to `"resources.d2bus.org/v3"` — never specified in Nix.
`status` is omitted from all emitted artifacts and is read-only; the Zone
runtime fills `uid`, `generation`, `revision`, `timestamps`, and management
metadata at first activation and on subsequent reconciles. All three status
layers (D088) — the universal `ResourceStatus` base, the ResourceType-common
`status.resource`, and any Provider-specific `status.provider` — are runtime
observation and are never authored in Nix. The `status.provider.details`
extension schema is signed into and registered with the **Provider package**
(resolved via `spec.artifactId`), not authored or emitted by the Zone Nix
configuration; Nix authors only `type` + `spec`.

The `managedBy` field (`configuration | controller | api`) is a core-set
management metadata field set exclusively by the core runtime. It is not
specified in Nix, not emitted by the Nix build, and not accepted in any
resource spec input. The Nix compiler rejects any resource whose `spec`
contains a `managedBy` key.

The `spec` field uses the **exact same field names and nesting** as the
ResourceTypeSchema JSON — there is no second bespoke Nix vocabulary. A spec
field called `providerRef` in the JSON schema is `providerRef` in Nix; a
nested struct called `budget.cpu.cores` maps to `budget.cpu.cores` in Nix.

Generated Nix option types, defaults, allowed values, and inline documentation
are derived from the same committed ResourceTypeSchema
(`docs/reference/schemas/v3/<ResourceType>.json`) and Provider schema
(`settingsSchemaDigest` in `provider-catalog.json`) — the module system and the
build validator use the same source of truth.

There are no Nix-only fields inside a resource declaration. The `type`/`spec`
envelope is the complete authoring shape; `type` is the ResourceType
discriminator and `spec` is emitted verbatim. To disable a resource, omit it
from `d2b.zones.<zone>.resources` or use the ResourceType's own desired-state
fields if that type defines them.

All spec fields are emitted verbatim into the canonical JSON envelope.
Derivation references and NixOS system closures belong in `d2b.artifacts`, not
inside any resource spec field.

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type":       "<ResourceType>",
  "metadata": {
    "name":       "<name>",
    "zone":       "<zone>",
    "ownerRef":   null,
    "finalizers": []
  },
  "spec": { /* exact spec fields, identical to Nix input */ }
}
```

`uid`, `generation`, `revision`, and `timestamp` fields are absent from
Nix-emitted artifacts. `ownerRef` defaults to `null`; it may be set in `spec`
only for resources that are explicitly owner-attributed at authoring time (not
for dynamically controller-created resources). `finalizers` defaults to `[]` in
emitted bundles and is managed exclusively at runtime.


### ResourceTypeSchema validation

Every ResourceType has a committed JSON Schema under
`docs/reference/schemas/v3/<ResourceType>.json` generated from canonical Rust
DTOs in `d2b-contracts` by `cargo xtask gen-schemas`. The Nix build derivation
validates every emitted `spec` against this schema before the derivation
succeeds. The drift gate `make test-drift` enforces `xtask gen-schemas` +
`git diff --exit-code` to prevent silent schema/code drift.

| Validation rule | Layer |
| --- | --- |
| Every `spec` field type-checks against committed JSON Schema | Build |
| All required fields present; no unknown top-level `spec` fields | Build |
| `spec` must not contain `managedBy` (core-set runtime field) | Eval |
| `resource_name` matches `^[a-z][a-z0-9-]*$` | Eval |
| All `*Ref` fields follow `<ResourceType>/<resource_name>` | Eval |
| All refs resolve within the same Zone | Eval |
| `ownerRef` acyclicity | Eval |
| `domain` ∈ `allowedDomains` of the target Host/Guest | Eval |
| CIDR ranges non-overlapping within Zone | Eval |
| Vendor ResourceType installed before use in any ref | Eval |
| `artifactId` / `systemArtifactId` exists in `d2b.artifacts` | Build |
| `artifactId` artifact has `type = "provider"` + trust validated | Build |
| `systemArtifactId` artifact has `type = "nixos-system"` | Build |
| `source.systemArtifactId` artifact has `type = "nixos-system"` | Build |
| Numeric/string bounds (e.g., vsockCid range) | Eval |
| `config` matches Provider's signed settings schema (Provider ResourceSpec) | Build |
| `config` must not contain raw host paths or store path strings | Build |
| `providerSettings` matches Provider's signed settings schema (where schema declares it) | Build |
| `providerSettings` must not contain raw host paths, store paths, or secret bytes | Build |
| Store paths absent from all public ResourceSpecs, status, audit, and OTEL telemetry | Build/Runtime |

A structured eval error identifies the exact NixOS option path and rejected
value for every rule violation.

### Provider-specific settings validation

The `Provider` ResourceSpec uses a `config` field validated against the
Provider's signed settings schema. Other semantic ResourceTypes (`Host`, `Guest`,
`Credential`, and others) may use a `providerSettings` field only when their
canonical ResourceTypeSchema declares it; if the schema does not declare
`providerSettings`, the field is absent and must not appear in Nix input. All
other spec fields are those declared in the ResourceTypeSchema exactly. Nix
mirrors each ResourceSpec 1:1; no blanket rule applies across types.

Validation is offline; no network access occurs during the build. The schema
fingerprint is recorded in `provider-catalog.json` under `settingsSchemaDigest`
(private compiled catalog metadata, not a ResourceSpec field); a schema mismatch
is a build error.

Rules:
- Additional fields not declared in the Provider schema are rejected
  (`additionalProperties: false`).
- `config` and any `providerSettings` value must not contain raw host paths or
  Nix store path strings; derivation outputs are expressed as artifact IDs or
  opaque source policy IDs. No provider-specific field in `providerSettings` may
  accept secret bytes.
- Numeric, string, and boolean bounds declared in the Provider schema are
  enforced at build time; out-of-bounds values fail the derivation.

### Credentials and secrets

No secret value (credential bytes, token, PSK material, key PEM, password,
certificate DER/PEM, bearer token, HMAC key) may appear in any Nix spec field,
any `providerSettings` value, or generated artifact. See "Prohibited fields
summary" below for the complete list. Provider-required secrets are always
declared as `Credential/<name>` refs. The `Credential` resource spec carries
configuration-only fields (`providerRef`, `scope`, `audience`, `consumerRef`,
`allowedOperations`, `rotation`, `expiry`, `revocation`, and optional
`providerSettings` where the Provider schema declares it) — never credential
bytes, tokens, or key material.

Actual secret bytes are injected at runtime via the broker's `StoreCredential`
op, which is never invoked from Nix. An eval assertion rejects any string field
matching known secret patterns (PEM `-----BEGIN` headers, JWT `eyJ...` prefix,
hex strings ≥ 32 bytes in secret-typed spec fields).

### Bundle integrity

The Zone resource bundle and the artifact catalog emitted by each Nix derivation
build are:

1. **Sorted**: every `*.json` file contains resources sorted by
   `metadata.name`. `bundle.json` file entries are sorted by filename.
   `artifact-catalog.json` entries are sorted by `artifactId`.
2. **Content-addressed**: `candidateId` is the sha256 of the concatenation of
   all per-file digests (Zone bundle files plus artifact catalog) in canonical
   sort order. `contentId` is the sha256 of canonical sorted resource content,
   stable across runtime-only metadata mutations (`uid`, `generation`,
   `revision`) applied after activation.
3. **Integrity-pinned**: `d2b-activation-helper` verifies every per-file digest
   against `bundle.json` before staging, including `artifact-catalog.json`;
   any mismatch fails activation closed.
4. **Hermetic**: same Nix inputs → byte-identical derivation output. No
   timestamps, randomness, or network access inside the derivation.
5. **Offline**: all required Provider JSON Schemas are in Provider package
   closures already in the Nix store at build time; no Provider dossier network
   fetch occurs during derivation evaluation or build.
6. **D070-compliant**: store paths are excluded from all public ResourceSpecs,
   status fields, audit records, and OTEL telemetry. The private
   `artifact-catalog.json` (root:d2bd 0640) contains `storePath` fields that
   are readable only by the Zone runtime and activation helper for staging.



### Generation compilation

Current source: `nixos-modules/bundle.nix` — monolithic bundle derivation,
`SHA256SUMS`, integrity chain. Current `d2b._bundle` attrset in
`bundle-artifacts.nix`. The v3 design replaces the monolithic bundle with
one content-addressed derivation per Zone.

Each Nix evaluation produces one immutable configuration generation: a closed
set of Zone resource bundles plus the provider catalog. Each bundle is a
content-addressed derivation:

```
/nix/store/<hash>-d2b-config-zone-dev/
  zone.json             — Zone self-resource
  providers.json        — Provider resources for this Zone
  hosts.json            — Host resources
  guests.json           — Guest resources
  processes.json        — Process and EphemeralProcess resources
  volumes.json          — Volume resources
  networks.json         — Network resources
  devices.json          — Device/User/Credential resources
  roles.json            — Role and RoleBinding resources
  index.json            — cross-resource index for this Zone
  bundle.json           — manifest of all files + content digests

/nix/store/<hash>-d2b-artifact-catalog/
  artifact-catalog.json — private integrity-pinned catalog; storePath per entry for staging
```

### Activation path

```
Nix eval
  → derivation build (hermetic, offline)
  → /nix/store/<hash>-d2b-config-zone-<name>/  (immutable)
  → system activation (d2b-activation-helper)
      1. verify bundle.json digest chain;
      2. validate resource refs, owners, RBAC cross-checks;
      3. stage new generation into Zone runtime (not yet active);
      4. atomically swap active pointer;
      5. trigger configuration-publication controller handler;
      6. record prior generation pointer for rollback window.
```

Steps 1–3 are fail-closed. Step 4 is atomic.

### Rollback

```bash
d2b zone rollback dev --generation <N>
```

Re-stages the prior generation's bundle and swaps the active pointer. The
configuration-publication controller reconciles affected resources.

Generation retention is a per-Zone Nix compiler setting outside the Zone
ResourceSpec. It is declared at the Zone option level:

```nix
d2b.zones.dev.retainedGenerations = 3;   # default 3; range 1..16
```

An eval assertion enforces `1 ≤ retainedGenerations ≤ 16`. The default is 3.
The minimum of 1 ensures at least one prior generation is always available for
rollback. The maximum of 16 is eval-enforced; values above 16 are rejected with
a structured error. `retainedGenerations` is consumed only during build and
runtime generation bookkeeping; it is never emitted into any Zone ResourceSpec
or bundle JSON.

### Cross-Zone generation ordering

When Zone A has a `ZoneLink` to Zone B, Zone A's bundle includes a `cursorRef`
pointing at the expected Zone B generation revision at compilation time. The
configuration-publication controller verifies Zone B revision before activating
Zone A.

Zone activations are independent. When Zone A activates and Zone B (referenced
via `ZoneLink`) has not yet reached the `cursorRef` revision recorded in
Zone A's bundle, Zone A activates independently and the `ZoneLink` resource
(and any Zone A resources that depend on Zone B state) enter `Degraded` status.
They reconcile asynchronously when Zone B becomes reachable and reaches the
expected revision. Zone A never claims a commit on behalf of Zone B. There is
no cross-Zone atomic activation and no option to block Zone A activation on
Zone B readiness.

### Resource cleanup contract

#### Configuration-owned vs controller-created resources

The configuration-publication controller classifies every resource in the Zone
runtime store using the core-set `managedBy` and `configurationGeneration`
fields set by the runtime at activation time:

- **Configuration-owned** (`managedBy=configuration`): resources whose
  `managedBy` field is `configuration`. The `configurationGeneration` field
  records the generation index at which the resource was last reconciled by
  config publication. The controller diffs `configurationGeneration` + name
  against the new generation's bundle to identify absent resources.
- **Controller-managed** (`managedBy=controller`) and **API-managed**
  (`managedBy=api`): resources set by runtime controllers or the resource API.
  These are **never** touched by the configuration-publication controller. No
  `ownerRef` inference, no label matching, and no "absent from emitted files"
  logic is used to determine cleanup eligibility for these resources.

The configuration-publication controller **only** enqueues for Delete the
resources that carry `managedBy=configuration` and whose `name`+`type` pair is
absent from the new generation's bundle. All other resources are untouched.

#### Absent-resource deletion

When a new configuration generation activates:

1. The configuration-publication controller reads the new generation's bundle
   `*.json` files to form the new configuration-owned name set.
2. Resources carrying `managedBy=configuration` whose name+type is absent from
   the new set are enqueued for asynchronous `Delete`.
3. **Activation does not block on cleanup.** Step 4 of the activation path
   (atomic pointer swap) completes before cleanup begins.
4. Each resource enqueued for Delete transitions to `status.phase: Pending`
   with a `PendingDeletion` condition (`reason: AbsentFromConfiguration`). The
   Zone's aggregate `status.phase` becomes `Degraded` until all pending deletes
   complete.
5. Deletion is finalizer-safe:
   - A resource with active finalizers receives a `DeletionBlocked` condition.
     The finalizer-holding controller must remove its finalizer before deletion
     proceeds; the cleanup controller waits and does not forcibly strip
     finalizers.
   - When a configuration-owned parent is enqueued for Delete, the parent's
     controller is responsible for observing the parent's `PendingDeletion`
     condition and reconciling owned children before clearing its finalizer.
     The cleanup controller cascades only to resources that also carry
     `managedBy=configuration`; controller-managed children of a deleted
     configuration-owned parent are handled by the parent's controller.
6. When all finalizers are clear and reconciliation is complete, a single store
   transaction appends the `Deleted` revision and removes the resource row and
   its indexes. Subsequent `Get` returns not-found. The `ResourceDelete` audit
   event is appended afterward with dedup/exactly-once recovery.

#### Prior generation retention and pruning

Prior generation bundles are retained according to `retainedGenerations`
(default 3, range 1..16). Pruning rules:

- A generation is eligible for pruning when it has been superseded by at least
  `retainedGenerations` newer activated generations.
- Pruning removes the generation's bundle pointer from the Zone runtime store.
  It does not forcibly interrupt in-flight deletions from that generation.
- On `d2b zone rollback dev --generation N`: configuration-owned resources
  absent from generation N that are undergoing cleanup are re-adopted into
  generation N's configuration-owned set. Resources that have already reached
  `Deleted` and been removed are re-created by the configuration-publication
  controller reconciler.

#### Status, errors, and audit

| Field | Values |
| --- | --- |
| `Zone.status.phase` | `Ready` — all configuration-owned resources reconciled; `Degraded` — deletion pending or ZoneLink lagging; `Pending` — new generation staged and pointer swapped, reconciliation in progress |
| `Resource.status.phase` | `Pending` — awaiting deletion completion; `Deleted` — a single store transaction appends the `Deleted` revision and removes the resource row and its indexes; subsequent `Get` returns not-found; `ResourceDelete` audit event is appended afterward with dedup/exactly-once recovery |
| `Resource.status.conditions[PendingDeletion]` | Present when resource is enqueued for deletion; `reason: AbsentFromConfiguration` |
| `Resource.status.conditions[DeletionBlocked]` | Present when a finalizer prevents deletion completion |
| `Resource.status.conditions[ReconcileError]` | Present on reconciliation failure |

Every absent-resource deletion initiated by a generation change emits a
structured audit event:

```json
{
  "kind":                  "ResourceDelete",
  "source":                "ConfigurationPublicationController",
  "zone":                  "<zone_name>",
  "resourceType":          "<ResourceType>",
  "resourceName":          "<resource_name>",
  "generationIndex":       <N>,
  "configurationGeneration": <prior_gen>,
  "reason":                "AbsentFromConfiguration"
}
```

#### Tests for removed-resource cleanup

| Test | Tier | Description |
| --- | --- | --- |
| Two-generation bundle diff | nix-unit | Generation 1 declares resource R; generation 2 omits R. Verify R absent from generation 2 bundle `*.json`; generation 1 bundle retains R. |
| Async cleanup activation | Integration | Activate generation 1 (R present, `managedBy=configuration`, phase Ready). Activate generation 2 (R absent). Verify R enters Pending/PendingDeletion; Zone phase Degraded. Complete cleanup. Verify a single store transaction appends the `Deleted` revision and removes the resource row and its indexes; subsequent `Get` returns not-found; `ResourceDelete` audit event is appended afterward with dedup/exactly-once recovery; Zone phase Ready. |
| Audit record | Integration | After async cleanup: verify structured `ResourceDelete` event with correct zone/type/name/generationIndex/configurationGeneration/reason fields. |
| Finalizer-blocked deletion | Integration | R holds active finalizer. Activate generation 2 (R absent). Verify R enters DeletionBlocked condition. Remove finalizer. Verify deletion completes: a single store transaction appends the `Deleted` revision and removes the resource row and its indexes; subsequent `Get` returns not-found; `ResourceDelete` audit event is appended afterward with dedup/exactly-once recovery; Zone phase returns Ready. |
| Controller-managed preservation | Integration | A resource carrying `managedBy=controller` exists. Activate generation 2. Verify the config-publication controller never touches that resource, regardless of ownerRef or bundle absence. |
| API-managed preservation | Integration | A resource carrying `managedBy=api` exists. Activate generation 2. Verify the config-publication controller never enqueues it for deletion. |
| Rollback after partial cleanup | Integration | Activate generation 2 (R absent, cleanup in progress). Before cleanup completes, roll back to generation 1. Verify R is re-adopted or re-created and returns to Ready. |
| Retention window enforcement | Integration | Activate generations 1–5 with `retainedGenerations=3`. Verify generation 1 is pruned and no longer available for rollback after generation 4 activates. |



Current source: `nixos-modules/index.nix` — `cfg._index.*` attribute tree
(`enabledEnvs`, `enabledVms`, `netMeta`, `declaredRealms`, `enabledRealms`,
`runtimeRows`). The `_index` is used by all other Nix emitters to avoid
repeated `filterAttrs` passes.

The v3 normalized index is a single cross-Zone artifact emitted to
`/etc/d2b/index.json`:

```json
{
  "schemaVersion": "v1",
  "zones": {
    "dev": {
      "hosts":     ["host-system"],
      "guests":    ["dev-vm"],
      "networks":  ["dev-lan"],
      "providers": ["system-core", "system-systemd", "runtime-cloud-hypervisor"]
    }
  },
  "executionIndex": {
    "Host/host-system": {
      "zone":        "dev",
      "providerRef": "Provider/system-core",
      "processes":   ["wayland-proxy"]
    },
    "Guest/dev-vm": {
      "zone":        "dev",
      "providerRef": "Provider/runtime-cloud-hypervisor",
      "processes":   []
    }
  },
  "networkIndex": {
    "Network/dev-lan": {
      "zone":          "dev",
      "lanSubnet":     "10.100.0.0/24",
      "attachedGuests": ["Guest/dev-vm"]
    }
  },
  "closureIndex": {
    "Guest/dev-vm": {
      "closureArtifact": "/etc/d2b/closures/dev-vm.json"
    }
  }
}
```

The index is derived; it is never edited directly. If it disagrees with the
resource bundle files the activation tool rejects the generation. The drift
gate enforces `xtask gen-index` + `git diff --exit-code`.

## Bundle artifacts

### `/etc/d2b/` layout

| File | Owner | Description |
| --- | --- | --- |
| `provider-catalog.json` | `root:d2bd 0640` | Offline Provider catalog |
| `index.json` | `root:d2bd 0640` | Cross-Zone normalized index |
| `zones/<name>/bundle.json` | `root:d2bd 0640` | Zone bundle manifest + digest chain |
| `zones/<name>/zone.json` | `root:d2bd 0640` | Zone self-resource |
| `zones/<name>/providers.json` | `root:d2bd 0640` | Provider resources |
| `zones/<name>/hosts.json` | `root:d2bd 0640` | Host resources |
| `zones/<name>/guests.json` | `root:d2bd 0640` | Guest resources |
| `zones/<name>/processes.json` | `root:d2bd 0640` | Process/EphemeralProcess resources |
| `zones/<name>/volumes.json` | `root:d2bd 0640` | Volume resources |
| `zones/<name>/networks.json` | `root:d2bd 0640` | Network resources |
| `zones/<name>/devices.json` | `root:d2bd 0640` | Device/User/Credential resources |
| `zones/<name>/roles.json` | `root:d2bd 0640` | Role/RoleBinding resources |
| `closures/<guest-name>.json` | `root:d2bd 0640` | Per-Guest closure map |
| `minijail-profiles/<id>.json` | `root:d2b-priv-broker 0640` | Minijail sandbox profiles |
| `privileges.json` | `root:d2bd 0640` | Broker op catalog (retained site-wide) |
| `realm-controllers.json` | `root:d2bd 0640` | Retained during migration; see ADR046-nix-008 |
| `realm-identity.json` | `root:d2bd 0640` | Retained during migration; see ADR046-nix-009 |

### Bundle manifest format

`zones/<name>/bundle.json`:

```json
{
  "schemaVersion":   "v1",
  "candidateId":     "<sha256 of concatenated file digests in canonical order>",
  "contentId":       "<sha256 of canonical resource content>",
  "generationIndex": 1,
  "files": [
    { "name": "zone.json",      "digest": "sha256:..." },
    { "name": "providers.json", "digest": "sha256:..." },
    { "name": "hosts.json",     "digest": "sha256:..." },
    { "name": "guests.json",    "digest": "sha256:..." },
    { "name": "processes.json", "digest": "sha256:..." },
    { "name": "volumes.json",   "digest": "sha256:..." },
    { "name": "networks.json",  "digest": "sha256:..." },
    { "name": "devices.json",   "digest": "sha256:..." },
    { "name": "roles.json",     "digest": "sha256:..." }
  ]
}
```

`candidateId`/`contentId` serve the same binding role as the current
`d2b._bundle` integrity chain (current source: `nixos-modules/bundle.nix`),
scoped per Zone.

## Conflict detection

The Nix compiler detects and rejects at eval time:

| Conflict | Rule |
| --- | --- |
| Duplicate Zone name | Two `d2b.zones.<x>` entries with the same `id` |
| ZoneLink cycle | A chain of `childZoneName` references in `ZoneLink` resources that loops |
| CIDR overlap | Two Networks in the same Zone with overlapping subnets |
| Guest name reserved prefix | A `Guest/<name>` starting with `sys-` |
| Owner cycle | Any `ownerRef` chain that loops |
| Type collision | Two `d2b.zones.<z>.resources` entries with the same attribute key (Nix prevents this by construction) or two entries with different keys but emitting the same `<Type>/<name>` pair (eval-checked for cross-type uniqueness) |
| Provider already installed | Two `d2b.zones.<z>.resources` entries of `type = "Provider"` with the same `catalogEntryId` |
| Catalog entry absent | A `catalogEntryId` not in `d2b.providerCatalog`, or an `artifactId`/`systemArtifactId` not in `d2b.artifacts` |
| Artifact type mismatch | An artifact ID used in a field that expects a different `type` (e.g., `"nixos-system"` where `"provider"` is required) |
| Duplicate artifact ID | Two `d2b.artifacts.<id>` entries with the same key |
| Missing ref target | Any `*Ref` field whose target is not declared |
| Role verb unknown | A verb not in the initial closed set |

## Current-code mapping

Every current v3 baseline Nix source path and Rust symbol is mapped below to
its v3 destination. Sources are at `b5ddbed6`. No source is removed until the
destination is integrated and tested.

### Nix modules

| Current source | Current purpose | Current live? | v3 destination | Work item |
| --- | --- | --- | --- | --- |
| `nixos-modules/options-realms.nix` | `d2b.realms.<r>.*` — `RealmId`/`RealmPath`/`RealmControllerPlacement`/`EntrypointMode` options | Yes | `nixos-modules/options-zones.nix` (Zone/Host/Guest/Provider options) | ADR046-nix-001 |
| `nixos-modules/options-realms-workloads.nix` | `d2b.realms.<r>.workloads.*` — `WorkloadId`/`WorkloadProviderKind`/`IsolationPosture` options | Yes | `nixos-modules/options-zones-resources.nix` (Guest/Host/Process) | ADR046-nix-001 |
| `nixos-modules/options-realms-network.nix` | `d2b.realms.<r>.network.*` | Yes | Network resource spec in `options-zones-resources.nix` | ADR046-nix-001 |
| `nixos-modules/options-envs.nix` | `d2b.envs.<e>.*` — env isolation substrate | Yes | Network resource spec; Env concept retired | ADR046-nix-002 |
| `nixos-modules/options-vms.nix` | `d2b.vms.<v>.*` — per-VM options | Yes | Guest resource spec; per-VM toggles become Device resources | ADR046-nix-002 |
| `nixos-modules/options-daemon.nix` | `d2b.site.*` daemon options | Yes | `options-site.nix` (retained + extended) | ADR046-nix-003 |
| `nixos-modules/index.nix` | `cfg._index.*` — `netMeta`/`enabledVms`/`realmRows` | Yes | `index.nix` (rewritten); emits `/etc/d2b/index.json` | ADR046-nix-004 |
| `nixos-modules/bundle-artifacts.nix` | `d2b._bundle.*` internal artifact table | Yes | `bundle-zones.nix` (per-Zone) + `bundle-artifacts.nix` (helpers retained) | ADR046-nix-005 |
| `nixos-modules/bundle.nix` | Bundle derivation + SHA256SUMS + integrity chain | Yes | Rewritten per-Zone; `bundle-zones.nix` | ADR046-nix-005 |
| `nixos-modules/processes-json.nix` | `VmProcessDag`/`ProcessRole`/`binaryPath`/argv | Yes | `resources-zones-processes.nix`; emits `zones/<z>/processes.json` | ADR046-nix-006 |
| `nixos-modules/storage-json.nix` | `StorageJson`/`StoragePathSpec`/ownership/ACL contract rows | Yes | `resources-zones-volumes.nix`; rows migrate to Volume resources | ADR046-nix-007 |
| `nixos-modules/sync-json.nix` | `SyncJson` OFD lock contract rows | Yes | Internal to `d2b-contracts`; removed from Nix artifacts | ADR046-nix-007 |
| `nixos-modules/allocator-json.nix` | `AllocatorJson` — realm/env bridge assignments, socket paths | Yes | Folded into Zone/Network/Host resources | ADR046-nix-008 |
| `nixos-modules/realm-controller-config-json.nix` | `RealmControllersJson`/`realm-controllers.json` | Yes (read by live d2bd `realm_access_resolver`) | Zone bootstrap bundle `zones/<z>/zone.json` | ADR046-nix-008 |
| `nixos-modules/realm-workloads-launcher-v2-json.nix` | `RealmWorkloadsLauncherV2Json`/`realm-workloads-launcher-v2.json` | Yes | Provider/display-wayland + Provider/shell-terminal Process configs in `zones/<z>/processes.json` | ADR046-nix-009 |
| `nixos-modules/realm-identity-config-json.nix` | `RealmIdentityConfigJson`/`realm-identity.json` | Yes (read by live d2bd) | Credential resource specs; identity config inside Provider resources | ADR046-nix-009 |
| `nixos-modules/unsafe-local-workloads-json.nix` | `unsafe-local-workloads.json` | Yes | User-only Host + Process resources | ADR046-nix-010 |
| `nixos-modules/privileges-json.nix` | `privileges.json` broker op catalog | Yes | Retained at `/etc/d2b/privileges.json` | ADR046-nix-011 |
| `nixos-modules/closures-json.nix` | `closures/<vm>.json` per-VM closure maps | Yes | Retained at `/etc/d2b/closures/<guest>.json`; emitter rewritten | ADR046-nix-012 |
| `nixos-modules/minijail-profiles.nix` | `minijail-profiles/<id>.json` | Yes | Retained at same path; emitter adapted to Zone Guest refs | ADR046-nix-012 |
| `nixos-modules/manifest.nix` | `manifest.json` `manifestVersion` contract | Yes | `zones/<z>/bundle.json`; `manifestVersion` → `schemaVersion` | ADR046-nix-013 |
| `nixos-modules/host-json.nix` | `host.json` host-side config | Yes | Folded into Host resource in `zones/<z>/hosts.json` | ADR046-nix-013 |
| `nixos-modules/assertions.nix` | Eval-time invariants (CIDR, platform, VM names) | Yes | Retained and extended | ADR046-nix-014 |
| `nixos-modules/host.nix`, `host-daemon.nix`, `host-activation.nix`, `host-users.nix` | Host NixOS modules, `d2bd`/`d2b-priv-broker` units, activation helper, users/groups | Yes | Retained and adapted to Zone bundle activation | ADR046-nix-015 |
| `nixos-modules/network.nix`, `net.nix` | Bridge/NAT/DHCP systemd-networkd units | Yes | Reconciled by `Provider/network-local`; retained until Provider successor | ADR046-nix-016 |
| `nixos-modules/store.nix` | Per-VM `/nix/store` hardlink farm; `share.source == "/nix/store"` sentinel | Yes | Reconciled by `Provider/volume-virtiofs`; retained until Provider successor | ADR046-nix-017 |
| `nixos-modules/components/` (graphics, tpm, usbip, audio) | Per-VM toggleable features | Yes | Each becomes a Provider install resource + Device/Guest spec | ADR046-nix-018 |

### Rust symbols

The following current Rust symbols need explicit v3 destination assignments
because their names carry old terminology. Each is a current-live path unless
marked compile-only.

| Current symbol | Current crate/file | Current live? | v3 destination | Work item |
| --- | --- | --- | --- | --- |
| `RealmId`, `RealmPath` | `d2b-realm-core/src/ids.rs`, `realm.rs` | Live in d2bd | `ZoneId` (`^[a-z][a-z0-9-]*$`), `ZonePath` in `d2b-contracts/src/v3/identity.rs` | ADR046-identities-001 |
| `WorkloadId` | `d2b-realm-core/src/ids.rs` | Live | `ResourceName` + `ResourceRef` for `Guest/<name>` or `Process/<name>` | ADR046-identities-001 |
| `NodeId`, `NodeSummary`, `NodeKind` | `d2b-realm-core/src/node.rs` | Live in metadata | `Host` / `Guest` ResourceType (see NodeKind table) | ADR046-identities-001 |
| `ProviderId` | `d2b-realm-core/src/ids.rs` | Live | `ResourceName` for `Provider/<name>` | ADR046-identities-001 |
| `RealmTarget` / `WorkloadTarget` (`<wid>.<realm>.d2b`) | `d2b-realm-core/src/target.rs`, `d2b-core/src/workload_identity.rs` | Live in resolver | `Zone/<z>` + `Guest/<name>` ResourceRef | ADR046-identities-001 |
| `RealmControllerPlacement` (`HostLocal`, `GatewayVm`, `CloudFullHost`, `ProviderController`, `ProviderAgent`) | `d2b-realm-core/src/realm.rs` | Metadata only | `Host.providerRef` + `Guest.providerRef` per NodeKind table | ADR046-nix-001 |
| `EntrypointMode` (`HostResident`, `GatewayBacked`) | `d2b-realm-core/src/realm.rs` | Metadata only | `Host` vs `Guest` ExecutionPolicy distinction | ADR046-nix-001 |
| `VmProcessDag`, `ProcessNode`, `ProcessRole` | `d2b-core/src/processes.rs` | Live (processes.json consumed by broker) | `Process`/`EphemeralProcess` per disposition table | ADR046-nix-006 |
| `ProcessesJson`, `ProcessRole.CloudHypervisorRunner` | `d2b-core/src/processes.rs`, `nixos-modules/processes-json.nix` | Live | `Process` under `Provider/runtime-cloud-hypervisor` owned by `Guest` | ADR046-nix-006 |
| `ProcessRole.Virtiofsd` + `share.source == "/nix/store"` sentinel | `d2b-core/src/processes.rs`, `nixos-modules/processes-json.nix` | Live | `Process` under `Provider/volume-virtiofs` + Volume nix-closure source | ADR046-nix-006 |
| `StorageJson`, `StoragePathSpec`, `PrincipalRef { kind: "uid"|"user" }` | `d2b-core/src/storage.rs`, `nixos-modules/storage-json.nix` | Live (storage.json consumed by broker) | `Volume` layout/views; `PrincipalRef` uid-kind → `User/<name>` ref | ADR046-nix-007 |
| `SyncJson`, OFD lock rows | `d2b-core/src/sync.rs`, `nixos-modules/sync-json.nix` | Live | Internal `d2b-contracts` implementation mechanism; removed from Nix artifacts | ADR046-nix-007 |
| `RealmControllersJson`, `RealmControllerMetadataSummary` | `d2b-core/src/realm_controller_config.rs`; read live by d2bd `realm_access_resolver` | Live | Zone self-resource + ZoneLink bootstrap; `realm-controllers.json` retained during migration | ADR046-nix-008 |
| `RealmWorkloadsLauncherV2Json`, `LauncherWorkloadSummary` | `d2b-core/src/realm_workloads_launcher.rs` | Live | Process resource annotations in `zones/<z>/processes.json` | ADR046-nix-009 |
| `RealmIdentityConfigJson` | `d2b-realm-core/src/identity_config.rs`; loaded live by d2bd | Live | Credential resource `scope`/`audience`/`allowedOperations`/`providerSettings` fields; `realm-identity.json` retained during migration | ADR046-nix-009 |
| `WorkloadProviderKind`, `IsolationPosture`, `WorkloadExecutionPosture` | `d2b-realm-core/src/workload.rs` | Live in launcher metadata | `LocalVm`/`QemuMedia`/`ProviderManaged` → `Guest.providerRef` per table; `UnsafeLocal` → user-only `Host` with `providerSettings.isolationPosture: "none"` (never `Guest`; not a v3 Provider) | ADR046-nix-001 |
| `Capability` enum, `CapabilitySet` | `d2b-realm-core/src/capability.rs` | Live in provider advertisement | Role verbs / Provider descriptor fields per Capability disposition table | ADR046-nix-001 |
| `RuntimeProvider`, `WorkloadProvider`, `HostSubstrateProvider` traits | `d2b-realm-provider/src/provider.rs` | Live (ACA/local-vm implement these) | Provider component descriptors + `ADR-046-provider-<name>.md` dossiers | ADR046-provider-001 |
| `OperationRouter`, `DurableExecTable`, `TargetResolver` | `d2b-realm-router/src/`; `d2bd/src/realm_stubs.rs` | COMPILE-ONLY at baseline (`dead_code`-allowed, not called) | `d2b-bus` routing; adapt `RealmSessionAuthority`/`CredentialCustody`/`RealmServiceLimits` from `main:packages/d2b-realm-router/src/service_v2.rs` | ADR046-bus-010 |
| `TransportProvider`, `loopback`, `LocalTcpTransport` | `d2b-realm-transport/src/lib.rs`, `local_tcp.rs` | COMPILE-ONLY / conformance tests only | `Provider/transport-unix` (Unix seqpacket); `Provider/transport-vsock`; transport primitives adapt `main:packages/d2b-session-unix/src/` | ADR046-nix-026 |

### Systemd unit mapping

No current unit is removed until the resource/Provider successor is integrated.

| Current unit | Current role | v3 treatment |
| --- | --- | --- |
| `d2bd.service` | PID1 local-root controller | Retained; becomes fixed Zone core controller launcher |
| `d2bd.socket` | Local-root public socket | Retained unchanged |
| `d2b-priv-broker.service` | Privileged broker | Retained unchanged |
| `d2b-priv-broker.socket` | Broker socket activation | Retained unchanged |
| Per-env bridge units (`br-<e>-lan`, `br-<e>-up`) | Network bridge, systemd-networkd | Owned by `Provider/network-local` Process resources after migration |
| `sys-<e>-net` VM unit | Auto-declared net VM | Replaced by `Network` + `Guest/sys-<e>-net` under `Provider/network-local` |
| Per-VM store-sync | Store hardlink farm sync | Replaced by `EphemeralProcess` under `Provider/volume-virtiofs` |
| Per-VM swtpm | TPM state (`ProcessRole::Swtpm`) | Replaced by `Process` under `Provider/device-tpm` |
| Per-VM GPU sidecar | Cloud Hypervisor VMM (`ProcessRole::CloudHypervisorRunner`) | Replaced by `Process` under `Provider/runtime-cloud-hypervisor` |
| `d2b@<workload>` | Legacy per-workload template | Not created in v3 |

## Prohibited fields summary

Never accepted in any Nix-authored resource spec or generated artifact:

- Credential/secret bytes, token values, PSK material, key PEM, passwords;
- Freeform host paths outside closure-backed derivation outputs;
- Raw numeric UID or GID in spec fields;
- Ambient capability bitmasks or named capability lists in Process spec;
- Raw seccomp BPF programs (only named Provider-owned profile refs);
- Arbitrary socket addresses or raw file descriptor numbers;
- Provider-internal `argv` or environment variable maps;
- `eval` or `builtins.exec` in Nix resource compiler expressions;
- `RealmTarget` format strings (`<wid>.<realm>.d2b`) in any spec field.

## Feasibility proof required

| Proof | Description |
| --- | --- |
| Zone self-resource round-trip | Nix → `zone.json` → Zone runtime → resource API GET returns matching spec |
| Provider install resource | Nix catalog + Provider resource → core controller lifecycle → Ready status |
| Host system/user Process | Process under Host/host-system with system-systemd; locally held pidfd |
| Guest with closure Volume | Guest + virtiofs Volume → per-VM store farm → guest `/nix/store` without direct host store export |
| Cross-Zone ZoneLink | Parent Zone declares unidirectional ZoneLink with `childZoneName`/`transportProviderRef`; activates with child cursor; child update propagates to parent; no `parentRef` in child spec |
| Configuration rollback | Two-generation rollback restores prior Zone resource state |
| Ref validation rejection | Malformed or missing ref fails eval with structured error |
| Conflict detection | CIDR overlap, owner cycle, and duplicate type/name rejection at eval time |
| ProcessRole parity | Every `ProcessRole` variant has a corresponding test case in the Process/EphemeralProcess resource schema |
| Unsafe-local Host | User-only `Host` with `providerSettings.isolationPosture: "none"` reconciled by `Provider/system-core`; child Processes use normal Process Providers; `NoIsolation` condition present in Host status; `no_isolation=true` in `ProcessEffect` audit events; OTEL never labels isolation; CLI/UI warning non-suppressible; no `Guest` emitted |
| ResourceTypeSchema validation | Every emitted `spec` validates against committed JSON Schema at build time; schema drift gate passes; unknown field in `providerSettings` (where schema declares it) or Provider `config` fails build |
| Credential ref enforcement | PEM header in spec field fails eval; `Credential/<name>` ref accepted; no secret bytes in any emitted artifact |
| Bundle integrity | Byte-identical rebuild from identical inputs; `candidateId`/`contentId` match computed values; file digest mismatch fails activation |
| Absent-resource async Delete | Generation 1 has resource R (`managedBy=configuration`); generation 2 omits R; R enters Pending/PendingDeletion; Zone Degraded; a single store transaction appends the `Deleted` revision and removes the resource row and its indexes; subsequent `Get` returns not-found; `ResourceDelete` audit event is appended afterward with dedup/exactly-once recovery; Zone Ready |
| Controller-managed preserved | Resource with `managedBy=controller` untouched by config-publication controller; never enqueued for deletion regardless of bundle absence |
| Finalizer-safe deletion | Resource with active finalizer enters DeletionBlocked condition; stays Pending; deletion completes after finalizer removed: a single store transaction appends the `Deleted` revision and removes the resource row and its indexes; subsequent `Get` returns not-found; `ResourceDelete` audit event is appended afterward with dedup/exactly-once recovery |
| Provider crate layout gate | Stub `d2b-provider-test-missing-integration/` missing `integration/` fails `make test-policy`; complete stub with all four paths passes |
| Artifact catalog resolution | Declare `d2b.artifacts.dev-vm-system = { package = …; type = "nixos-system"; }`; build succeeds; `artifact-catalog.json` contains matching entry with correct type/digests/storePath; `storePath` absent from all public ResourceSpecs, status, audit, OTEL; reference with wrong type fails build; absent artifact ID fails build |

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `nixos-modules/options-realms*.nix`; `options-vms.nix`; `options-envs.nix`; `index.nix`; `bundle*.nix`; `*-json.nix`; `d2b-realm-core` ids/realm/workload/capability/allocation; `d2b-core` processes/storage/workload-identity; live `realm_access_resolver`; live `realm-controllers.json`/`realm-identity.json` |
| Evidence class | Nix schemas and generated artifacts are live/reachable; `d2b-realm-router`/`d2b-realm-transport` are compile-only stubs at baseline; Zone/Host/Guest/ResourceRef/Provider resources are ADR-only |
| Behavior retained | Hermetic deterministic eval, offline derivations, integrity-pinned artifacts, no secrets in generated JSON, CIDR/platform/name assertions, fine-grained storage ownership/ACL/no-follow, minijail profiles, pidfd/adoption |
| Required delta | Zone/Resource option schema, per-Zone bundle layout, normalized index rewrite, configuration-publication controller hook, ZoneLink cursor, rollback activation path, Host ResourceType (new), ProcessRole disposition implementation |
| Reuse path | Extract pure schema validators, CIDR/name assertion logic, closure-info pattern, and index helpers per work item; `d2b-realm-core` ID validators adapted to `ZoneId`/`ResourceName`; main `a1cc0b2d` ComponentSession/Provider implementation adapted per ADR046-bus-* items |
| Replacement/deletion | No current options or artifact emitters are removed until the named destination is integrated and all tests pass; `realm-controllers.json` and `realm-identity.json` retained during migration |
| Feasibility proof | Items in the proof table above |
| Future owner | Work items below; Provider dossiers; `ADR-046-core-controllers.md` configuration-publication handler |

## Main-commit ComponentSession and Provider reuse inventory

Exact reuse sources are at `main:a1cc0b2da4a08ca3240a770a972fe4da6f912bef` (W9
commit "coordinate toolkit and sibling cutover"). None of these paths are
implemented on the v3 baseline `b5ddbed6`; they are reuse sources, not current
evidence. Every item specifies the exact main-commit file/symbol/tests, the
selected behavior, the v3 destination, and the ADR45 assumptions that must be
excluded or adapted before integration.

### Reuse summary

| Main package | Key files/symbols | v3 destination | Work item |
| --- | --- | --- | --- |
| `d2b-session` | `engine.rs`, `handshake.rs`, `lifecycle.rs`, `scheduler.rs`, `record.rs`, `fragmentation.rs`, `transport.rs`, `driver.rs`, `server.rs`, `metrics.rs` | `packages/d2b-bus/src/session/` | ADR046-nix-023 |
| `d2b-session` | `attachment.rs`, `streams.rs`, `cancellation.rs`, `deadline.rs`, `bootstrap.rs` | `packages/d2b-bus/src/session/` | ADR046-nix-024, ADR046-nix-025 |
| `d2b-contracts` | `v2_component_session.rs` — all wire constants + types | `packages/d2b-contracts/src/v3/component_session.rs` | ADR046-nix-027 |
| `d2b-contracts` | `generated_v2_services/` (24 service + 24 ttrpc files) | `packages/d2b-contracts/src/v3/services/` | ADR046-nix-028 |
| `d2b-session-unix` | `adapter.rs`, `socket.rs`, `descriptor.rs`, `credit.rs`, `pidfd.rs`, `systemd.rs`, `vsock.rs` | `packages/d2b-bus/src/transport/unix/` | ADR046-nix-026 |
| `d2b-provider` | `registry.rs`, `rpc.rs`, `instance.rs`, `context.rs` | `packages/d2b-provider/src/` (adapt in place) | ADR046-nix-029 |
| `d2b-provider-toolkit` | `adapter.rs`, `server.rs`, `conformance.rs`, `registration.rs` | `packages/d2b-provider-toolkit/src/` (adapt in place) | ADR046-nix-030 |
| `d2b-client` | `client.rs`, `session.rs`, `target.rs`, `service.rs`, `daemon_service.rs`, `guest_service.rs`, `host_socket.rs` | `packages/d2b-client/src/` (adapt in place) | ADR046-bus-009 |
| `d2b-realm-router` | `service_v2.rs` | `packages/d2b-bus/src/routing/zone_service.rs` | ADR046-bus-010 |
| `d2bd` | `provider_registry.rs` | `packages/d2bd/src/provider_registry.rs` (adapt in place) | ADR046-bus-011 |
| `d2bd` | `provider_effects.rs` | `packages/d2bd/src/provider_effects.rs` (adapt in place) | ADR046-bus-012 |

**ComponentSession wire contract — cross-reference only:** The choices for
`EndpointRole`, `ServicePackage`, `EndpointPurpose`, `PurposeClass`, and
`Locality` variant naming (including any rename of `RealmController`,
`RealmBroker`, `RealmV2`, `RealmPeer`, `RealmBootstrap` to Zone-prefixed
names), the exact Zone service name replacing `"d2b.realm.v2.RealmService"`,
the v3 `TargetInput` variant shape for Zone/Resource addressing, the v3
`PROVIDER_BUNDLE_VERSION` and `PROVIDER_BUNDLE_SCHEMA_VERSION`, and the v3
wire operation replacing `VmLifecycleRequest` — are all owned by the
ComponentSession/d2b-bus foundation spec and the provider-registry contract
work item. These are not defined here.

Invariants that apply regardless of naming decisions: wire tag values (numeric)
are stable and must not change regardless of string/identifier renames. Zone
service target is addressed as `ResourceRef/Zone`; Guest lifecycle operations
address resources as `ResourceRef` (`Guest/<name>`) with a Zone context
(`Zone/<z>`). Bundle/schema version constants are generated by the owning
contract work item (ADR046-bus-011/ADR046-bus-012). Cross-reference:
`ADR-046-componentsession-and-bus` and the d2b-contracts v3 work item.

## Implementation work items

### ADR046-nix-001

| Field | Value |
| --- | --- |
| Dependency/owner | W0; `d2b-contracts` identities (ADR046-identities-001, ADR046-identities-002) |
| Current source | `nixos-modules/options-realms.nix` (`RealmId`/`RealmPath`/`RealmControllerPlacement`/`EntrypointMode` labels); `options-realms-workloads.nix` (`WorkloadId`/`WorkloadProviderKind`/`IsolationPosture`/`WorkloadExecutionPosture`); `options-realms-network.nix`; `d2b-realm-core/src/realm.rs`, `workload.rs`, `capability.rs`, `ids.rs` (symbols to adapt) |
| Reuse action | adapt |
| Destination | `nixos-modules/options-zones.nix` (Zone-level options: `label`, `retainedGenerations`, `trustedPublishers` — compiler settings, not Zone spec fields); `nixos-modules/options-zones-resources.nix` (unified `resources` attrset) |
| Detailed design | `d2b.zones.<z>.resources.<name> = { type = "<ResourceType>"; spec = { ... }; }` — single attrset covering all ResourceTypes; `type` discriminates dispatch; `spec` fields mirror exact ResourceTypeSchema field names and nesting; Nix option types/defaults/docs generated from `docs/reference/schemas/v3/<ResourceType>.json`; no Nix-only fields inside resource declarations; `metadata.name` derives from attr key; `metadata.zone` derives from enclosing zone attr key; `apiVersion` defaulted; `uid`/`generation`/`revision`/`status`/`managedBy` never in Nix; `resource_name` regex `^[a-z][a-z0-9-]*$`; ref validation assertions; `WorkloadProviderKind` → Guest/Host mapping per disposition table above; `Capability` → Role verb mapping per resource-api/authz foundation spec; Zone self-resource spec is `{}`; `retainedGenerations`/`trustedPublishers` are Zone-level compiler settings not emitted in Zone spec |
| Integration | `nixos-modules/default.nix` imports new options files; old realms options coexist until ADR046-nix-002 |
| Data migration | Operator configs migrate `d2b.realms.*` → `d2b.zones.*`; `d2b.vms.*` → `d2b.zones.<z>.resources.*` with `type = "Guest"` |
| Validation | nix-unit vectors for each ResourceType; ref-validation rejection vectors; malformed ref error shape; ZoneLink `childZoneName` resolves declared Zone; missing `transportProviderRef` fails eval; `managedBy` in spec rejected at eval; Zone spec is `{}` (no `parentRef`, `retainedGenerations`, etc.) |
| Tests | `tests/unit/nix/cases/zones-options.nix`, `tests/unit/nix/cases/zones-ref-validation.nix`, `tests/unit/nix/cases/zones-zonelink.nix` |
| Drift pin | `make nix-unit-pin` after adding cases |
| Removal proof | `options-realms*.nix` removed after `options-zones*.nix` achieves parity and parity drift test passes |

### ADR046-nix-002

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-001; env/VM migration |
| Current source | `nixos-modules/options-envs.nix` (`lanSubnet`/`uplinkSubnet`/`mtu`/`mssClamp`/`externalNetwork.*`); `nixos-modules/options-vms.nix` (`d2b.vms.<v>.*`; `ProcessRole` toggle options for components) |
| Reuse action | adapt |
| Destination | `Network` resource fields in `nixos-modules/options-zones-resources.nix`; `Guest` resource fields |
| Detailed design | `d2b.envs.work.lanSubnet` → `d2b.zones.work.resources.work-lan = { type = "Network"; spec = { lanSubnet = "..."; ... }; }`; CIDR overlap assertion migrated; `sys-` reserved prefix and VM-name regex retained; `d2b.vms.<v>.tpm.enable` → `d2b.zones.<z>.resources.vm-tpm = { type = "Device"; spec = { providerRef = "Provider/device-tpm"; ... }; }` |
| Integration | `options-zones-resources.nix` emits Network/Guest resources consumed by the rewritten index/bundle and Provider/network-local/device Providers; assertions run during eval before bundle emission. |
| Data migration | Operator configs migrate env/VM component options into Zone Network/Guest/Device resources; full d2b 3.0 reset means no v2 runtime state/config import beyond rewritten Nix declarations. |
| Validation | nix-unit CIDR rejection; eval assertion for `sys-` prefix; VM-name regex |
| Tests | `tests/unit/nix/cases/zones-network.nix`, `tests/assertions-eval.sh` extended |
| Drift pin | `make nix-unit-pin` |
| Removal proof | `options-envs.nix`, `options-vms.nix` removed after migration parity test passes |

### ADR046-nix-003

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-001; site options |
| Current source | `nixos-modules/options-daemon.nix`, `options-site.nix`, `options-host.nix` |
| Reuse action | retain and extend |
| Destination | `nixos-modules/options-site.nix` (retained); per-Zone options in `options-zones.nix` |
| Detailed design | `d2b.zones.<z>.retainedGenerations` (default 3, range 1..16, compiler setting — not emitted in Zone spec); `d2b.site.stateDir` maps to Zone storage roots; `d2b.site.usePrebuiltHostTools` retained |
| Integration | `options-site.nix` retains site defaults consumed by `options-zones.nix` and `bundle-zones.nix`; activation-helper and Zone storage roots read `stateDir`/`usePrebuiltHostTools` settings. |
| Data migration | Operator site options stay in place; new per-Zone `retainedGenerations` defaults apply on first v3 activation; no v2 runtime state import. |
| Validation | nix-unit case verifies `retainedGenerations` default/range, `site.stateDir` mapping, and `usePrebuiltHostTools` retention. |
| Tests | `tests/unit/nix/cases/site-options.nix` |
| Drift pin | `make nix-unit-pin` |
| Removal proof | No removal; file extended only |

### ADR046-nix-004

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-001, ADR046-nix-002 |
| Current source | `nixos-modules/index.nix` — `cfg._index.*`: `enabledEnvs`, `enabledVms`, `netMeta` (derives bridge names/IPs from `lanSubnet`/`uplinkSubnet`), `declaredRealms`, `enabledRealms`, `workloadsInEnv`, `runtimeRows` |
| Reuse action | rewrite |
| Destination | `nixos-modules/index.nix` (rewritten); emits `/etc/d2b/index.json` |
| Detailed design | Cross-Zone normalized index: zone/host/guest/network/closure entries; executionIndex; networkIndex; closureIndex; sorted output; `cfg._index` attribute tree retained as internal helper during migration |
| Integration | `options-zones-resources.nix` feeds rewritten `index.nix`; `bundle-zones.nix`, resource emitters, and activation-helper consume `/etc/d2b/index.json`. |
| Data migration | Index is rebuilt from v3 Zone resources at activation; full d2b 3.0 reset; no v2 index state import. |
| Validation | nix-unit golden vectors for index shape; drift gate: `xtask gen-index` round-trip |
| Tests | `tests/unit/nix/cases/index-zones.nix`; `tests/unit/gates/drift-check.sh` extended |
| Drift pin | `make nix-unit-pin`; `make flake-matrix-pin` if flake checks change |
| Removal proof | `cfg._index.envMeta`, `cfg._index.realms.*` sub-trees removed after all callers migrate to Zone resource lookups |

### ADR046-nix-005

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-004 |
| Current source | `nixos-modules/bundle-artifacts.nix` (`artifactModule` submodule, `installFileName`, `mode 0640` ownership); `nixos-modules/bundle.nix` (bundle derivation, SHA256SUMS, `d2b._bundle` integrity chain) |
| Reuse action | extend and rewrite |
| Destination | `nixos-modules/bundle-zones.nix` (per-Zone bundle derivation); common helpers retained in `bundle-artifacts.nix` |
| Detailed design | Per-Zone `bundle.json` with `candidateId`/`contentId` binding; SHA256 digest chain; `generationIndex`; atomic activation pointer; `manifestVersion` → `schemaVersion` rename |
| Integration | `d2b-activation-helper` reads `bundle.json` per Zone; validates digest chain before staging |
| Data migration | Per-Zone bundles are generated from v3 resource declarations; full d2b 3.0 reset; no v2 monolithic bundle state import. |
| Validation | Artifact-shape contract tests in `packages/d2b-contract-tests/tests/`; determinism test (build twice, diff outputs) |
| Tests | `tests/unit/nix/cases/bundle-zones.nix`; `tests/unit/gates/drift-check.sh` for schema drift |
| Drift pin | `make test-drift` |
| Removal proof | Monolithic `bundle.json` and `d2b._bundle` artifact table retired after all Zone bundle tests pass |

### ADR046-nix-006

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-005; Process Provider work items (ADR046-primitives-002) |
| Current source | `nixos-modules/processes-json.nix` (`VmProcessDag`, `ProcessRole`, `binaryPath`, `argv`, `share.source == "/nix/store"` sentinel); `packages/d2b-core/src/processes.rs` (`ProcessRole` enum — all variants in disposition table above) |
| Reuse action | extract and adapt |
| Destination | `nixos-modules/resources-zones-processes.nix`; emits `zones/<z>/processes.json` |
| Detailed design | Process/EphemeralProcess resource serialization per disposition table; no free-form `binaryPath` or `argv`; template refs; mounts from `volumeRef`; sandbox from named profile; VsockRelay → `Process` under `Provider/transport-vsock`; GuestSshReadiness retired at v3 cutover; Usbip long-lived backend/proxy → `Process`, Usbip per-busid attach/detach → `EphemeralProcess`, all owned by `Provider/device-usbip` |
| Integration | `processes.json` replaces `cfg._bundle.processesJson`; Process Providers read the new format |
| Data migration | Processes are emitted from v3 Process/EphemeralProcess resources; full d2b 3.0 reset; no v2 process artifact import. |
| Validation | Process resource schema vectors; no-raw-path assertion; ProcessRole parity test (every variant has a test case) |
| Tests | `tests/unit/nix/cases/zones-processes.nix`; `packages/d2b-contract-tests/tests/processes-schema.rs` |
| Drift pin | `make test-drift` after schema changes |
| Removal proof | `processes-json.nix` and current `processes.json` schema removed after all Process Providers consume `zones/<z>/processes.json` |

### ADR046-nix-007

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-005; Volume Provider work items (ADR046-primitives-003) |
| Current source | `nixos-modules/storage-json.nix` (all `mkPath` calls, `PrincipalRef` `uid`/`gid`/`user` kinds, `StorageRoot`/`StoragePathSpec`/`repairPolicy`/`cleanupPolicy`); `packages/d2b-core/src/storage.rs` (`StorageJson`, `StoragePathSpec`); `nixos-modules/sync-json.nix` (`SyncJson` OFD lock rows); `packages/d2b-core/src/sync.rs` |
| Reuse action | extract storage policy → adapt; retire sync rows |
| Destination | `nixos-modules/resources-zones-volumes.nix`; emits `zones/<z>/volumes.json`; OFD lock rows move to `d2b-contracts` internals |
| Detailed design | Volume layout/views/ACL/no-follow/repair preserving current policy; `PrincipalRef { kind: "uid" }` → `User/<name>` typed ref only; OFD rows removed from Nix artifacts |
| Integration | Volume resources feed `zones/<z>/volumes.json`; the Volume Provider consumes it for path/view/ACL reconciliation; `d2b-contracts` internals own OFD lock rows. |
| Data migration | Volume resources are regenerated from v3 config; full d2b 3.0 reset; no v2 storage/sync state import. |
| Validation | Volume schema vectors; ACL/no-follow/view policy tests |
| Tests | `tests/unit/nix/cases/zones-volumes.nix`; `packages/d2b-contract-tests/tests/volumes-schema.rs` |
| Drift pin | `make test-drift` |
| Removal proof | `storage-json.nix`, `sync-json.nix`, and `/etc/d2b/storage.json`/`sync.json` removed after Volume controller parity |

### ADR046-nix-008

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-004; Zone/Network migration |
| Current source | `nixos-modules/allocator-json.nix` (realm/env bridge assignments, `allocatorStateDir`, socket paths, `providerPlacement`); `nixos-modules/realm-controller-config-json.nix` (emits `realm-controllers.json`); `packages/d2b-core/src/realm_controller_config.rs` (`RealmControllersJson`, `RealmControllerMetadataSummary`; read live by `d2bd/src/realm_access_resolver.rs` from `/etc/d2b/realm-controllers.json`) |
| Reuse action | adapt and retire |
| Destination | Zone self-resource in `zones/<z>/zone.json`; allocator concept retired from Nix; socket paths in Zone resource spec; `realm-controllers.json` RETAINED during migration (live d2bd reads it) |
| Detailed design | Zone runtime derives socket paths from Zone name; broker row → Network/Host resource; `realm-controllers.json` must remain published until `realm_access_resolver` is replaced by Zone resource API |
| Integration | Zone bootstrap config publishes socket paths consumed by the Zone runtime and `realm_access_resolver` during migration; the replacement Zone resource API takes over before legacy artifacts are removed. |
| Data migration | Zone bootstrap data is regenerated from v3 Zone resources; full d2b 3.0 reset; no allocator/realm-controller state import. |
| Validation | Zone resource round-trip; bootstrap socket path regression tests |
| Tests | `tests/unit/nix/cases/zones-bootstrap.nix`; `realm_access_resolver` contract test |
| Drift pin | `make nix-unit-pin` |
| Removal proof | `allocator-json.nix`, `realm-controller-config-json.nix`, and `/etc/d2b/allocator.json`/`realm-controllers.json` removed ONLY after `realm_access_resolver` is replaced by Zone bundle reader |

### ADR046-nix-009

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-006; display/credential Provider work items |
| Current source | `nixos-modules/realm-workloads-launcher-v2-json.nix` (`RealmWorkloadsLauncherV2Json`, `LauncherWorkloadSummary`; live); `nixos-modules/realm-identity-config-json.nix` (`RealmIdentityConfigJson`; live, read by d2bd from `/etc/d2b/realm-identity.json`); `packages/d2b-core/src/realm_workloads_launcher.rs`; `packages/d2b-realm-core/src/identity_config.rs` |
| Reuse action | adapt |
| Destination | Provider/display-wayland and Provider/shell-terminal Process configs in `zones/<z>/processes.json`; `Provider/credential-entra` Credential resource; `realm-identity.json` RETAINED during migration |
| Detailed design | Launcher metadata folded into Process resource annotations; identity config → Credential resource fields (`providerRef`, `scope`, `audience`, `allowedOperations`, `providerSettings` where Provider schema declares it; no secret bytes); `realm-identity.json` must remain until d2bd `RealmIdentityConfigJson` loading is replaced by Credential resource reader |
| Integration | Provider/display-wayland, Provider/shell-terminal, and Provider/credential-entra consume Process/Credential resources; `d2bd` continues reading `realm-identity.json` until the Credential reader lands. |
| Data migration | Launcher and identity config are re-emitted as v3 resources; full d2b 3.0 reset; no v2 launcher/identity state import. |
| Validation | Launcher metadata shape regression; no-secret assertion vectors |
| Tests | `tests/unit/nix/cases/zones-launcher-metadata.nix`; no-secret vectors |
| Drift pin | `make nix-unit-pin` |
| Removal proof | `realm-workloads-launcher-v2-json.nix`/`realm-identity-config-json.nix` and `/etc/d2b/realm-workloads-launcher-v2.json`/`realm-identity.json` removed ONLY after display/credential Providers read resource configs |

### ADR046-nix-010

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-001; unsafe-local migration |
| Current source | `nixos-modules/unsafe-local-workloads-json.nix` (`WorkloadProviderKind::UnsafeLocal`/`IsolationPosture::UnsafeLocal`; current `unsafe-local-workloads.json` artifact); `nixos-modules/unsafe-local-helper.nix` (user-domain process/helper definitions); `packages/d2b-core/src/unsafe_local_workloads.rs` |
| Reuse action | adapt |
| Destination | User-only `Host` resource in `zones/<z>/hosts.json` (`providerSettings.isolationPosture: "none"`, `defaultDomain: user`, `allowedDomains: [user]`, `defaultUserRef: User/<name>`); child `Process` resources in `zones/<z>/processes.json` using normal Process Providers; shell session supervisor → `Process` under `Provider/shell-terminal`; never a `Guest`; not a v3 Provider |
| Detailed design | `providerSettings.isolationPosture: "none"` is declared in the Host schema under `providerSettings`; enforced at eval time; user-only Host rejects system-domain Process refs; `NoIsolation` condition in Host status; `status.isolationPosture: none`; every `ProcessEffect` audit event under this Host carries `no_isolation=true`; OTEL telemetry never carries an isolation label; CLI/UI warning non-suppressible |
| Integration | The unsafe-local Nix emitter produces a user-only Host plus Process resources; Provider/system-core and shell-terminal consume them; CLI/UI and audit/telemetry surfaces read resulting status/events. |
| Data migration | unsafe-local declarations migrate to Host/Process resources in Nix; full d2b 3.0 reset; no helper runtime state import. |
| Validation | User-only Host rejection of system-domain Process refs; `providerSettings.isolationPosture != "none"` assertion rejection for user-only system-core Hosts; `NoIsolation` condition present in status; `no_isolation=true` in `ProcessEffect` audit event; OTEL attribute absent; no Guest emitted for unsafe-local declaration |
| Tests | `tests/unit/nix/cases/zones-unsafe-local.nix`; `tests/host-integration/unsafe-local-helper.nix` extended |
| Drift pin | `make nix-unit-pin` |
| Removal proof | `unsafe-local-workloads-json.nix` and unsafe-local-specific Nix code removed after user-only Host/Process resources pass all `tests/host-integration/unsafe-local-helper.nix` tests |

### ADR046-nix-011

| Field | Value |
| --- | --- |
| Dependency/owner | Broker privileges owner |
| Current source | `nixos-modules/privileges-json.nix` |
| Reuse action | retain |
| Destination | `nixos-modules/privileges-json.nix` (retained); `/etc/d2b/privileges.json` (retained, site-wide) |
| Detailed design | Broker op catalog is not Zone-scoped; no structural change required |
| Integration | `privileges-json.nix` remains a site-wide bundle artifact consumed by broker authorization and drift gates; Zone resource emission references it without Zone scoping. |
| Data migration | Full d2b 3.0 reset; no v2 state/config import |
| Validation | Existing `tests/unit/gates/drift-check.sh` |
| Removal proof | Not removed in this spec |

### ADR046-nix-012

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-005; ADR046-nix-022 (artifact catalog emitter); Provider/volume-virtiofs |
| Current source | `nixos-modules/closures-json.nix` (keyed by `d2b.vms.<name>`); `nixos-modules/minijail-profiles.nix` |
| Reuse action | adapt |
| Destination | `nixos-modules/closures-json.nix` (rewritten, keyed by artifact ID from `d2b.artifacts` with `type = "nixos-system"`); `nixos-modules/minijail-profiles.nix` (retained, adapted to reference Zone Guests) |
| Detailed design | Closure emitter iterates `d2b.artifacts` entries with `type = "nixos-system"`, computes `pkgs.closureInfo`, records `storePath`/digest/size in artifact catalog (private root:d2bd 0640 field; absent from all public ResourceSpecs/status/audit/OTEL); `Guest.spec.systemArtifactId` links Guest to artifact; `Volume.source.systemArtifactId` links Volume to artifact; minijail profile emitter structurally unchanged; old `d2b.vms.<name>` keying retired |
| Integration | Artifact catalog and Guest/Volume resources reference closure entries; activation-helper and Provider/volume-virtiofs consume private `storePath` metadata while public surfaces remain path-free. |
| Data migration | Closure entries are regenerated by artifact ID from `d2b.artifacts`; full d2b 3.0 reset; no v2 VM-keyed closure state import. |
| Validation | Closure map round-trip; per-VM store hardlink integrity; `storePath` present in private catalog; `storePath` absent from all emitted public ResourceSpecs and status/audit/OTEL surfaces |
| Tests | `tests/unit/nix/cases/closures-zones.nix`; `tests/unit/nix/cases/artifact-catalog-store-path-public-absent.nix` |
| Drift pin | `make nix-unit-pin` |
| Removal proof | Old `d2b.vms.*`-keyed closure entries removed after all Guests use `zones/<z>/guests.json` and artifact catalog |

### ADR046-nix-013

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-005; manifest contract |
| Current source | `nixos-modules/manifest.nix` (`manifestVersion` pinned contract); `nixos-modules/host-json.nix` |
| Reuse action | replace |
| Destination | Per-Zone `zones/<z>/bundle.json` (`schemaVersion`); Host resource in `zones/<z>/hosts.json` |
| Detailed design | `manifestVersion` → `schemaVersion`; `host.json` host config folded into Host resource spec; CHANGELOG entry for rename required |
| Integration | `bundle-zones.nix` writes `schemaVersion` into per-Zone `bundle.json`; activation-helper validates it and Host resource consumers replace `host.json` readers. |
| Data migration | Manifest/host artifacts are regenerated as Zone bundle/Host resources; full d2b 3.0 reset; no v2 manifest state import. |
| Validation | Schema drift gate; CHANGELOG enforcement |
| Tests | `tests/unit/gates/drift-check.sh` extended for `schemaVersion` |
| Drift pin | `make test-drift` |
| Removal proof | `manifest.nix` and `host.json` emitters removed after Zone bundle activation path passes |

### ADR046-nix-014

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-001, ADR046-nix-002 |
| Current source | `nixos-modules/assertions.nix` (CIDR overlap, `sys-` prefix, VM-name regex, platform gate) |
| Reuse action | extend in place |
| Destination | `nixos-modules/assertions.nix` |
| Detailed design | Migrate existing assertions to Zone/Resource terminology; add ref validation, owner cycles, CIDR overlap (Zones), provider resolution, RoleBinding verb set assertions |
| Integration | `assertions.nix` validates Zone/Resource declarations before index, bundle, and schema validation derivations run; failing configs block activation artifacts. |
| Data migration | None — docs/tooling only; no runtime state |
| Validation | Each new assertion has a failing-config test vector |
| Tests | `tests/assertions-eval.sh` extended; `tests/unit/nix/cases/assertions-zones.nix` |
| Drift pin | `make nix-unit-pin` |
| Removal proof | No removal; extended only |

### ADR046-nix-015

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-001; host activation |
| Current source | `nixos-modules/host.nix`, `host-daemon.nix` (fixed local-root endpoint set), `host-activation.nix`, `host-users.nix` |
| Reuse action | retain and adapt |
| Destination | Same files; updated to use Zone bundle activation path and Zone resource state dirs |
| Detailed design | `d2b-activation-helper` updated to validate/stage per-Zone bundles; `d2bd.service` updated to read Zone bundle; `d2b` group retained for `SO_PEERCRED` |
| Integration | Host activation units call `d2b-activation-helper` against per-Zone bundles, then `d2bd.service` reads the staged Zone bundle; `SO_PEERCRED` authorization remains via the `d2b` group. |
| Data migration | Host activation reads newly staged Zone bundles on first v3 switch; full d2b 3.0 reset; no v2 daemon runtime state import. |
| Validation | Host-integration test with Zone bundle activation and daemon readiness |
| Tests | `tests/host-integration/` extended for Zone activation |
| Removal proof | No removal; adapted in place |

### ADR046-nix-016

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-002; Provider/network-local dossier |
| Current source | `nixos-modules/network.nix`, `nixos-modules/net.nix` (including `lib.mkForce` DHCP neutralizer and per-env MTU/MSS/east-west wiring) |
| Reuse action | retain until Provider successor |
| Destination | Network reconciliation by `Provider/network-local` Process resources |
| Detailed design | Current bridge/NAT/DHCP/firewall Nix units retained; `Provider/network-local` controller emits equivalent configuration from Network resources; `lib.mkForce` neutralization preserved |
| Integration | Network resources feed Provider/network-local parity implementation while `net.nix`/`network.nix` remain the fallback; network eval tests compare both paths. |
| Data migration | Network resource specs are generated from v3 Nix declarations; full d2b 3.0 reset; no v2 bridge/NAT runtime state import. |
| Validation | `tests/net-vm-network-eval.sh` passes against Network resource spec |
| Tests | `tests/unit/nix/cases/zones-network-parity.nix` |
| Drift pin | `make nix-unit-pin` |
| Removal proof | `network.nix`/`net.nix` removed after `Provider/network-local` parity and `tests/net-vm-network-eval.sh` passes |

### ADR046-nix-017

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-012; Provider/volume-virtiofs |
| Current source | `nixos-modules/store.nix` (`ProcessRole::Virtiofsd` and `share.source == "/nix/store"` sentinel; per-VM hardlink farm) |
| Reuse action | retain until Provider successor |
| Destination | Per-VM store reconciliation by `Provider/volume-virtiofs` EphemeralProcess/Process resources |
| Detailed design | `store.nix` retained; `Provider/volume-virtiofs` controller creates equivalent EphemeralProcess; per-VM store path derived from Zone stateDir + Guest name via Provider, not raw path in spec |
| Integration | Guest/Volume resources drive Provider/volume-virtiofs, which stages hardlink farm processes consumed by Guest process resources; `store.nix` remains until parity. |
| Data migration | Full d2b 3.0 reset; no v2 store metadata import |
| Validation | Store hardlink integrity; no direct `/nix/store` export |
| Tests | Existing store integrity tests extended with Zone/Guest resource fixture |
| Removal proof | `store.nix` removed after `Provider/volume-virtiofs` manages farm lifecycle and existing store tests pass |

### ADR046-nix-018

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-002; Provider dossiers for graphics, tpm, usbip, audio |
| Current source | `nixos-modules/components/graphics.nix` (`ProcessRole::Gpu`, `GpuRenderNode`, `Video`); `components/tpm.nix` (`ProcessRole::Swtpm`, `SwtpmPreStartFlush`); `components/usbip.nix` (`ProcessRole::Usbip`); `components/audio/` (`ProcessRole::Audio`) |
| Reuse action | each component becomes a Provider install resource + Device/Guest spec field |
| Destination | `Provider/device-tpm`, `Provider/device-usbip`, `Provider/device-gpu`, `Provider/audio-pipewire` resource install declarations in `options-zones-resources.nix` |
| Detailed design | `d2b.vms.<v>.tpm.enable = true` → `d2b.zones.<z>.resources.vm-tpm = { type = "Device"; spec = { providerRef = "Provider/device-tpm"; ... }; }`; all component eval assertions migrated to `assertions.nix`; GuestSshReadiness retired at v3 cutover; Usbip long-lived backend/proxy → `Process`, per-busid attach/detach → `EphemeralProcess`, both owned by `Provider/device-usbip` |
| Integration | Device/Guest specs feed Provider/device-* and Provider/audio-pipewire install declarations; migrated assertions/tests consume Zone resource configs instead of component toggles. |
| Data migration | Component toggles migrate to v3 Device/Guest resource declarations; full d2b 3.0 reset; no v2 component runtime state import. |
| Validation | Existing component eval tests; `tests/usbip-gating-eval.sh`; `tests/video-contract-eval.sh` |
| Tests | `tests/unit/nix/cases/zones-devices.nix` |
| Drift pin | `make nix-unit-pin` |
| Removal proof | `components/` Nix units removed after Provider resource install achieves parity and all component eval tests pass against Zone resource configs |

### ADR046-nix-019

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-005; ADR046-nix-001; `d2b-contracts` schema generation (ADR046-nix-027) |
| Current source | `nixos-modules/bundle-artifacts.nix` (`artifactModule` submodule, mode/ownership); `nixos-modules/bundle.nix` (digest chain, SHA256SUMS); `packages/xtask/src/main.rs` (`gen-schemas`); no current per-ResourceType JSON Schema under `docs/reference/schemas/v3/` |
| Reuse action | extend xtask schema generation; new Nix eval/build validation hooks |
| Destination | `docs/reference/schemas/v3/<ResourceType>.json` for each ResourceType; `nixos-modules/resource-schema-validation.nix` (validates emitted spec against committed JSON Schema at build time); `nixos-modules/provider-settings-validation.nix` (validates `providerSettings` where declared in schema, and Provider `config`, against Provider-embedded schema at build time); `nixos-modules/assertions.nix` (Credential ref enforcement, secret-pattern rejection) |
| Detailed design | `cargo xtask gen-schemas` emits one JSON Schema per ResourceType under `docs/reference/schemas/v3/`; Nix derivation reads these schemas from `pkgs.d2b-resource-schemas` and validates every emitted `spec` JSON before producing the Zone bundle; Provider-settings validation reads `settingsSchemaDigest` from `provider-catalog.json` and resolves the schema from the Provider package closure; Credential ref enforcement: eval assertion rejects any `spec` string field matching `-----BEGIN`, `eyJ`, or a hex string ≥ 32 bytes in a secret-typed field; `managedBy` in any input spec rejected at eval (core-set runtime field, never in Nix input); bundle integrity: `candidateId`/`contentId` computed over canonical sorted output |
| Integration | Validation hooks wired into `bundle-zones.nix` derivation; `d2b-activation-helper` re-verifies digest chain at staging |
| Data migration | None — docs/tooling only; no runtime state |
| Validation | Schema round-trip: emit spec, validate against schema, verify byte-identical re-emit; `providerSettings` rejection test (unknown field, out-of-bounds value, raw store path, secret bytes) where schema declares the field; Provider `config` rejection test (unknown field); Credential ref enforcement: PEM-in-spec rejected; secret-pattern-in-spec rejected; valid `Credential/<name>` ref accepted; `managedBy` in spec input rejected at eval |
| Tests | `tests/unit/nix/cases/resource-schema-validation.nix`; `tests/unit/nix/cases/provider-settings-validation.nix`; `tests/unit/nix/cases/credential-ref-enforcement.nix`; `tests/unit/nix/cases/managed-by-rejection.nix`; `packages/d2b-contract-tests/tests/resource-schema-round-trip.rs` |
| Drift pin | `make test-drift` after any `gen-schemas` run; `make nix-unit-pin` after adding cases |
| Removal proof | Not removed; extended as new ResourceTypes are added |

### ADR046-nix-020

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-005; ADR046-nix-001; configuration-publication controller (ADR-046-core-controllers) |
| Current source | No current equivalent; current `bundle.nix` replaces all artifacts atomically with no per-resource cleanup tracking |
| Reuse action | new |
| Destination | Configuration-publication controller handler in `packages/d2bd/src/config_publication.rs`; `ConfigurationOwnedClassifier`; `AbsentResourceReaper`; `Zone` status conditions in `d2b-contracts/src/v3/zone_status.rs`; cleanup audit emitter in `d2b-state/src/audit_segments.rs` |
| Detailed design | `ConfigurationOwnedClassifier`: classify resources by core-set `managedBy` field only — `managedBy=configuration` resources are owned by config publication; `managedBy=controller` and `managedBy=api` resources are never touched. At activation, diff new-generation bundle name+type set against all resources with `managedBy=configuration` in the Zone store; resources absent from the new bundle are enqueued for Delete. Never infer ownership from `ownerRef`, labels, or absence from emitted files. `AbsentResourceReaper`: processes the Absent queue asynchronously; does not block pointer swap (step 4); sets `status.phase=Pending` + `PendingDeletion` condition (`reason: AbsentFromConfiguration`); waits for all finalizers to clear; commits a single store transaction that appends the `Deleted` revision and removes the resource row and its indexes; subsequent `Get` returns not-found; the `ResourceDelete` audit event is appended afterward with dedup/exactly-once recovery. Zone phase: `Pending` during pointer-swap-to-first-reconcile window; `Degraded` while any `managedBy=configuration` resource carries `PendingDeletion` or a ZoneLink lags; `Ready` when all reconciled. Generation pruning: prune when `generationIndex ≤ activeIndex - retainedGenerations` AND all enqueued resources from that generation have reached `Deleted`. Rollback: re-adopt `managedBy=configuration` resources in `Pending/PendingDeletion` back to the rollback target generation's owned set |
| Integration | `d2b-activation-helper` sets `managedBy=configuration` + `configurationGeneration` on every resource it activates; controller reads these fields to determine owned set — never `ownerRef` or bundle membership alone |
| Data migration | Full d2b 3.0 reset; existing resources gain `managedBy`/`configurationGeneration` only when first v3 activation creates them; no v2 state/config import. |
| Validation | Classification: `managedBy=controller` resource never enqueued (even if absent from bundle). `managedBy=api` resource never enqueued. `managedBy=configuration` resource absent from new bundle always enqueued. Finalizer safety: resource with active finalizer enters DeletionBlocked; not force-deleted; stays in `Pending`. Final deletion: a single store transaction appends the `Deleted` revision and removes the resource row and its indexes; subsequent `Get` returns not-found; `ResourceDelete` audit event is appended afterward with dedup/exactly-once recovery. Zone status: `Pending` during activation; `Degraded` while PendingDeletion outstanding; `Ready` when clean. Audit: `ResourceDelete` event includes `configurationGeneration` field. |
| Tests | `tests/unit/nix/cases/cleanup-two-generation.nix` (bundle diff); `tests/host-integration/cleanup-activation.nix` (async cleanup, Pending→Deleted, single store transaction appends `Deleted` revision + removes row/indexes, subsequent `Get` not-found, `ResourceDelete` audit event appended afterward with dedup/exactly-once recovery, Zone Pending→Degraded→Ready); `tests/host-integration/cleanup-finalizer.nix` (DeletionBlocked, stays Pending, cleared on finalizer removal, single store transaction appends `Deleted` revision + removes row/indexes, subsequent `Get` not-found); `tests/host-integration/cleanup-controller-managed.nix` (managedBy=controller preserved); `tests/host-integration/cleanup-api-managed.nix` (managedBy=api preserved); `tests/host-integration/cleanup-rollback.nix` (rollback re-adoption); `tests/host-integration/cleanup-retention-window.nix` (generation pruning) |
| Drift pin | `make nix-unit-pin`; `make test-drift` if status/audit schema changes |
| Removal proof | Not removed; extended as new ResourceTypes are added |

### ADR046-nix-021

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-001; `d2b-contracts` workspace policy tests |
| Current source | `packages/d2b-contract-tests/tests/` (existing workspace policy lints: `tests/workspace-member-sort.rs`, `tests/crate-naming.rs`); no current Provider crate layout gate exists |
| Reuse action | new |
| Destination | `packages/d2b-contract-tests/tests/provider-crate-layout.rs`; workspace scan in `packages/xtask/src/main.rs` (extend `check-workspace` or add `check-provider-layout` subcommand) |
| Detailed design | Parse the root `packages/Cargo.toml` workspace member list; for every member path matching `packages/d2b-provider-*-*`: assert (1) `src/` directory exists and contains at least one `.rs` file; (2) `tests/` directory exists and contains at least one `.rs` file; (3) `integration/` directory exists and contains at least one `.rs` or fixture file; (4) `README.md` exists and is ≥ 200 bytes. All four conditions required; any single failure fails the test with a structured message naming the crate and missing path. Test runs as `cargo test -p d2b-contract-tests provider_crate_layout`; wired into `make test-policy`. |
| Integration | Wired into `make test-policy` (same gate family as existing workspace policy tests); no new `Makefile` target needed unless `test-policy` does not yet exist |
| Data migration | None — docs/tooling only; no runtime state |
| Validation | Fixture: add a stub `packages/d2b-provider-test-missing-integration/` with `src/lib.rs` and `tests/smoke.rs` but no `integration/` and no `README.md`; assert test fails naming both missing paths. Add complete stub with all four paths; assert test passes. |
| Tests | `packages/d2b-contract-tests/tests/provider-crate-layout.rs` with fixture sub-directories under `packages/d2b-contract-tests/fixtures/`; included in `make test-policy` |
| Drift pin | `make test-policy`; re-run after any new `d2b-provider-*-*` crate is added to the workspace |
| Removal proof | Not removed; extended as new Provider crates are added to the workspace |

### ADR046-nix-022

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-005 (bundle derivation); `d2b-contracts` schema generation (ADR046-nix-027) |
| Current source | No current equivalent for a separate artifact catalog. Current `nixos-modules/bundle.nix` embeds package derivation references inline. Current `nixos-modules/closures-json.nix` uses `pkgs.closureInfo` keyed by `d2b.vms.<name>`. |
| Reuse action | new |
| Destination | `nixos-modules/artifact-catalog.nix` (new emitter); `nixos-modules/options-artifacts.nix` (new option: `d2b.artifacts.<id> = { package; type; }`); `/etc/d2b/artifact-catalog.json` (output artifact, `root:d2bd` 0640); `nixos-modules/bundle-zones.nix` (extend to include artifact catalog digest in `bundle.json`); `nixos-modules/options-zones-resources.nix` (replace `closureRef` / `nixosSystem` helpers with `systemArtifactId` validation) |
| Detailed design | `d2b.artifacts.<id>` attrset option: `id` matches `^[a-z][a-z0-9-]*$`; `type ∈ { "provider", "nixos-system", "nixos-module-set", "config-bundle" }`; no other fields. Emitter computes `pkgs.closureInfo` for each entry and writes `artifact-catalog.json` with sorted entries (by `artifactId`) containing `artifactId`, `type`, `storePath` (private, for activation-helper staging), `packageDigest`, `closureDigest`, `closureSize`. `storePath` is a private field of the root:d2bd 0640 file; it is never emitted in public ResourceSpecs, status fields, audit records, or OTEL telemetry. The `bundle.json` manifest includes the artifact catalog file entry and its SHA256 digest. `d2b-activation-helper` reads `storePath` from the catalog to resolve and stage each artifact; verifies catalog digest before staging. Build-time validation: `artifactId` / `systemArtifactId` / `source.systemArtifactId` fields in resource specs resolve against `d2b.artifacts`; type-mismatch fails with a structured error. `d2b.providerCatalog.<name>.package` option is removed; replaced by `d2b.providerCatalog.<name>.artifactId`. `Guest.spec.systemArtifactId` replaces the former `nixosSystem` Nix-only helper. `Volume.source.systemArtifactId` replaces `source.closureRef`. |
| Integration | `nixos-modules/closures-json.nix` rewritten (ADR046-nix-012) to key by artifact ID; `nixos-modules/bundle-zones.nix` includes artifact catalog in integrity chain |
| Data migration | Artifact catalog entries are generated from new `d2b.artifacts` declarations; full d2b 3.0 reset; no v2 `closureRef`/`nixosSystem` metadata import. |
| Validation | Artifact catalog round-trip: declare artifact, build, verify JSON entry present with correct type/digests/storePath; missing artifact ID fails build with structured error; wrong-type artifact fails build; `storePath` absent from all public ResourceSpecs and status/audit/OTEL surfaces |
| Tests | `tests/unit/nix/cases/artifact-catalog.nix` (declaration, resolution, storePath present in private catalog); `tests/unit/nix/cases/artifact-catalog-type-mismatch.nix` (build failure for wrong type); `tests/unit/nix/cases/artifact-catalog-missing-id.nix` (build failure for absent ID); `tests/unit/nix/cases/artifact-catalog-public-surfaces.nix` (storePath absent from all emitted ResourceSpecs); `packages/d2b-contract-tests/tests/artifact-catalog-schema.rs` |
| Drift pin | `make test-drift` (artifact catalog schema); `make nix-unit-pin` |
| Removal proof | Not removed; extended as new artifact types are added |


### ADR046-nix-023

| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046-componentsession-and-bus spec; ADR046-nix-027 (wire contract) must land first |
| Current source | Main commit source row below: `packages/d2b-session/` at `a1cc0b2d`, including session engine, handshake, lifecycle, scheduler, record protection, fragmentation, transport, driver, server, and metrics modules. |
| Reuse action | Reuse from main commit `a1cc0b2d`; adapt ADR45 EndpointPurpose/ServicePackage names listed in `ADR45 exclusions` while preserving wire tag values. |
| Destination | `packages/d2b-bus/src/session/` (new crate `d2b-bus`); `ComponentSessionDriver` becomes the central abstraction for all Zone bus sessions. |
| Detailed design | Complete session protocol: preface negotiation, Noise handshake profiles, record protection, replay cache, fair scheduler, fragmentation/reassembly, keepalive, ttrpc bridge, generation discovery, and `ComponentSessionDriver` application surface. |
| Integration | `d2b-bus` session runtime is used by Zone bus sessions for local-root controller to broker, controller to guest agent, and controller to provider agent; `serve_ttrpc_services` exposes service handlers. |
| Data migration | Full d2b 3.0 reset; no v2 session runtime state/config import |
| Validation | Port `component_session.rs` and `noise_vectors.rs`; run `make test-rust`; Noise KAT vectors must pass after copy. |
| Removal proof | ADR45 EndpointPurpose/ServicePackage realm names listed in `ADR45 exclusions` are adapted during copy; no prior v3 session owner to remove. |
| Main commit source | `packages/d2b-session/src/engine.rs` (`SessionEngine`, `SessionEvent`); `packages/d2b-session/src/handshake.rs` (`NoiseHandshake`, `HandshakeCredentials` Nn/Kk/IkPsk2 variants, `HandshakeRole`, `NegotiatedOffer`, `EstablishedHandshake`, `encode_offer`, `negotiate_offer`, `x25519_public_key`, `encode_generation_discovery_request`, `accept_generation_discovery_request`, `encode_generation_discovery_response`, `decode_generation_discovery_response`); `packages/d2b-session/src/lifecycle.rs` (`SessionLifecycle`, `SessionPhase`, `KeepaliveAction`); `packages/d2b-session/src/scheduler.rs` (`FairScheduler`, `QueueClass`, `OutboundFrame`); `packages/d2b-session/src/record.rs` (`RecordProtector`, `ProtectedRecord`; replay cache 1024 entries); `packages/d2b-session/src/fragmentation.rs` (`Fragmenter`, `Reassembler`, `Fragment`); `packages/d2b-session/src/transport.rs` (`OwnedTransport`, `TransportDescriptor`, `TransportPacket`, `TransportError`); `packages/d2b-session/src/driver.rs` (`ComponentSessionDriver` trait with 20 async methods, `SessionDriverHandle`); `packages/d2b-session/src/server.rs` (`serve_ttrpc_services`, `SessionServerError`); `packages/d2b-session/src/metrics.rs` (`MetricEvent`, `MetricsSink`, `NoopMetrics`) |
| Tests at main | `packages/d2b-session/tests/component_session.rs` — full Nn/Kk/IkPsk2 session lifecycle, fragmentation, attachments, named streams, cancellation, keepalive; `packages/d2b-session/tests/noise_vectors.rs` — KAT against `docs/reference/component-session-v2-vectors.json` |
| Selected behavior | Complete session protocol: preface negotiation, Noise handshake (all three profiles), record protection with 1024-entry replay cache, fair two-class scheduler, fragmentation/reassembly, keepalive (ping/timeout/close), ttrpc bridge (`serve_ttrpc_services`), generation discovery (pre-handshake version probe), `ComponentSessionDriver` as the sole application-layer control surface |
| v3 destination | `packages/d2b-bus/src/session/` (new crate `d2b-bus`); `ComponentSessionDriver` becomes the central abstraction for all Zone bus sessions (local-root controller ↔ broker, controller ↔ guest agent, controller ↔ provider agent) |
| ADR45 exclusions | `HandshakeOffer` fields `purpose: EndpointPurpose` and `service: ServicePackage` reference ADR45 enum values (`RealmPeer`, `RealmBootstrap`, `RealmV2`); adapt variant names per ADR-046-componentsession-and-bus owning spec; wire tag values are stable and must not change; `Locality::GuestLocal` remains valid for vsock guest sessions |
| Drift pin | `make test-rust` (session crate); Noise KAT vectors must pass after copy |

### ADR046-nix-024

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-023; ADR046-nix-027 |
| Current source | Main commit source row below: `packages/d2b-session/src/attachment.rs` and `packages/d2b-session/src/streams.rs` at `a1cc0b2d`. |
| Reuse action | Reuse from main commit `a1cc0b2d`; keep attachment/stream semantics and descriptor validation, with no ADR45 realm adaptation required for this item. |
| Destination | `packages/d2b-bus/src/session/` (same crate as ADR046-nix-023). |
| Detailed design | Attachment ownership validates descriptors only after authenticated decrypt; unconsumed payloads close on drop. NamedStreamMux preserves half-close, reset, per-stream limits, aggregate limits, and credit-based flow control. |
| Integration | `d2b-bus` session transport validates attachments after authenticated decrypt and exposes named streams to ttrpc/services; Unix/vsock transports provide payloads. |
| Data migration | Full d2b 3.0 reset; no v2 attachment or stream state/config import |
| Validation | Port `component_session.rs` attachment ownership, stream mux, and credit-accounting tests. |
| Removal proof | None — net-new; no prior owner to remove |
| Main commit source | `packages/d2b-session/src/attachment.rs` (`AttachmentPayload` trait with `validate_descriptor()`, `close()`, `as_any()`, `into_any()`; `OwnedAttachment` with `unbound()`/`bind_received()`/`bind_outbound()`/`validate_payload_descriptor()`/`into_payload()`; `AttachmentValidationError` enum: Kind/ObjectType/Access/CloseOnExec/Other); `packages/d2b-session/src/streams.rs` (`NamedStreamMux`, `StreamId`, `StreamEvent` enum: Data/RemoteClosed/Reset, `StreamPhase` enum: Open/HalfClosedLocal/HalfClosedRemote/Closed/Reset) |
| Tests at main | `packages/d2b-session/tests/component_session.rs` (attachment ownership, stream mux, credit accounting) |
| Selected behavior | Attachment lifecycle: `OwnedAttachment::unbound()` for transport-received before descriptor auth; `validate_descriptor()` called only after authenticated decryption; `into_payload()` transfers ownership without close; `Drop` closes remaining payload. `NamedStreamMux` with per-stream and aggregate queue byte limits; half-close semantics; credit-based flow control |
| v3 destination | `packages/d2b-bus/src/session/` (same crate as ADR046-nix-023); `AttachmentPayload` and `OwnedAttachment` are transport-neutral and require no ADR45 adaptation |
| ADR45 exclusions | `AttachmentDescriptor` from `v2_component_session` has `kind: AttachmentKind` and `object_type: KernelObjectType`; these values are wire-stable and carry no ADR45 realm naming; no exclusions for this item |

### ADR046-nix-025

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-023; ADR046-nix-027 |
| Current source | Main commit source row below: `packages/d2b-session/src/cancellation.rs`, `deadline.rs`, and `bootstrap.rs` at `a1cc0b2d`. |
| Reuse action | Reuse from main commit `a1cc0b2d`; adapt `CancelRequest`/`CancelResponse` module paths if v3 services move the common proto types. |
| Destination | `packages/d2b-bus/src/session/`. |
| Detailed design | Per-generation `RequestRegistry`, request-wide cancellation, `DeadlineBudget::admit_metadata()` as the inbound metadata gate, and single-use `BootstrapAdmission::consume()` replay protection. |
| Integration | `RequestRegistry`, `DeadlineBudget`, and `BootstrapAdmission` feed all `d2b-bus` inbound ttrpc calls and resource API cancellation paths. |
| Data migration | Full d2b 3.0 reset; no v2 cancellation/deadline/bootstrap state import |
| Validation | Port cancel-before-dispatch, cancel-after-dispatch, generation-mismatch cancel, and deadline admit tests; add v3 common-proto path adapter test. |
| Removal proof | ADR45 common proto type paths are adapted to the v3 service module; no prior v3 owner to remove. |
| Main commit source | `packages/d2b-session/src/cancellation.rs` (`Cancellation` with `cancel()`/`is_cancelled()`/`cancelled()` async notify; `RequestRegistry` with generation scoping, `register()`/`mark_dispatched()`/`cancel()`/`cancel_all()`/`cancel_generated()`/`complete()`/`remove()`/`signal()`/`active()`); `packages/d2b-session/src/deadline.rs` (`DeadlineBudget` with `admit_metadata()` validating clock skew ≤30 s, request lifetime ≤15 min, idempotency key, peer ttrpc timeout); `packages/d2b-session/src/bootstrap.rs` (`Secret32` zeroizing 32-byte key; `BootstrapPsk`; `AdmittedBootstrapPsk`; `BootstrapAdmission` single-use PSK with operation-ID + replay-nonce check) |
| Tests at main | `packages/d2b-session/tests/component_session.rs` (cancel-before-dispatch, cancel-after-dispatch, generation-mismatch cancel, deadline admit) |
| Selected behavior | `RequestRegistry` is per-generation; calling `cancel_all()` cancels every outstanding request; `DeadlineBudget::admit_metadata()` is the single gate for all inbound request metadata; `BootstrapAdmission::consume()` single-use PSK prevents replay |
| v3 destination | `packages/d2b-bus/src/session/` |
| ADR45 exclusions | `cancel_generated()` calls `common::CancelRequest`/`CancelResponse` from `v2_services`; the `common.rs` proto type paths may change when services are versioned to v3 (ADR046-nix-028); the underlying `CancelRequest`/`CancelAck`/`CancelResult` contract from `v2_component_session` is wire-stable and requires no change |

### ADR046-nix-026

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-023 |
| Current source | Main commit source row below: `packages/d2b-session-unix/src/{adapter,credit,descriptor,pidfd,socket,systemd,vsock}.rs` at `a1cc0b2d`. |
| Reuse action | Reuse from main commit `a1cc0b2d`; retain host-socket/native-vsock feature gates and replace only ADR45 socket-name strings with Zone bootstrap config. |
| Destination | `packages/d2b-bus/src/transport/unix/`. |
| Detailed design | Unix seqpacket/stream transports, pidfd identity verification, six-scope credit pool with host/process bounds, systemd-activated seqpacket listener support, and length-prefixed vsock framing. |
| Integration | `d2b-bus` listeners and Provider/guest sessions instantiate UnixSeqpacket/UnixStream/vsock transports; credit pool enforces attachment FD budgets before handing payloads to session runtime. |
| Data migration | Full d2b 3.0 reset; no v2 Unix transport state/config import |
| Validation | Port `unix_session.rs` seqpacket, stream, pidfd, FD attachment, and credit pool tests; add Zone bootstrap socket-name fixture. |
| Removal proof | ADR45 socket-name strings listed in exclusions are replaced by Zone bootstrap config; no prior v3 transport owner to remove. |
| Main commit source | `packages/d2b-session-unix/src/adapter.rs` (feature `host-socket`) — `UnixSeqpacketTransport`, `UnixStreamTransport`, `UnixAttachmentPayload`, `OwnedUnixAttachment`, `PeerIdentityPolicy` (Pathname/InheritedSocketpair), `DescriptorPolicyResolver`, `PathnamePeerVerifier`; `packages/d2b-session-unix/src/credit.rs` — `CreditBundle`, `CreditError`, `CreditPool`, `CreditScope`, `CreditScopeSet`, `ProcessCreditLimit`; `packages/d2b-session-unix/src/descriptor.rs` — `ReceivedPacket`, `AcceptedAttachment`, `DescriptorPolicy`, `FirstPacketCredentials`, `ObjectIdentity`, `PeerCredentials`, `PidfdIdentityPolicy`, `VerifiedPacket`; `packages/d2b-session-unix/src/pidfd.rs` — `DigestEvidenceCallback`, `PidfdEvidence`, `PidfdIdentityVerifier`, `PidfdInfoSource`, `ProcPidfdIdentityVerifier`, `ProcSelfFdInfoSource`, `parse_pidfd_fdinfo`; `packages/d2b-session-unix/src/socket.rs` — `AncillaryCapacity`, `OutboundPacket`, `PacketBurst`, `SendBurst`, `SentPacket`, `SeqpacketSocket`, `StreamRead`, `StreamSocket`, `prearmed_seqpacket_pair`; `packages/d2b-session-unix/src/systemd.rs` — `ActivatedSeqpacketListener`, `ActivatedSeqpacketListeners`, `SystemdActivationError`; `packages/d2b-session-unix/src/vsock.rs` (feature `native-vsock`) — `FramedVsockTransport`, `NativeVsockListener`, `NativeVsockTransport` |
| Tests at main | `packages/d2b-session-unix/tests/unix_session.rs` — seqpacket pair, Unix stream, pidfd identity, FD attachment validation, credit pool exhaustion and recovery |
| Selected behavior | Audited Unix seqpacket and stream transports implementing `OwnedTransport`; pidfd identity verification reads `/proc/<pid>/fdinfo/<pidfd>` for `st_dev`/`st_ino`; 6-scope credit pool with per-process and per-host limits (`MAX_PROCESS_ATTACHMENT_CREDITS=2048`, `MAX_HOST_ATTACHMENT_CREDITS=8192`); systemd `SD_LISTEN_FDS` seqpacket activation; vsock framing with length-prefixed records |
| v3 destination | `packages/d2b-bus/src/transport/unix/` — adapt as the Unix transport backend for `d2b-bus` sessions; keep the `host-socket`/`native-vsock` feature gates intact; the transport code itself has no ADR45 realm bindings |
| ADR45 exclusions | `ActivatedSeqpacketListeners` reads socket names from `SD_LISTEN_FDS`; socket names are bound to current `d2bd.socket` / `d2b-priv-broker.socket` unit names; v3 socket paths come from Zone bootstrap config — activation code is reusable, socket name strings are not. `PeerIdentityPolicy::accepted()` is transport-layer code and has no ADR45 binding |

### ADR046-nix-027

| Field | Value |
| --- | --- |
| Dependency/owner | ADR-046-componentsession-and-bus; naming and wire enumeration decisions per ADR-046-componentsession-and-bus owning spec before final adoption |
| Current source | Main commit source row below: `packages/d2b-contracts/src/v2_component_session.rs` at `a1cc0b2d` plus committed session/noise tests. |
| Reuse action | Reuse wire constants/types from main commit `a1cc0b2d`; rename ADR45 realm-oriented variants only as directed by the bus owning spec and preserve numeric tags. |
| Destination | `packages/d2b-contracts/src/v3/component_session.rs`. |
| Detailed design | Canonical wire values, bounded vectors, binary size constants, closed-enum tag codecs, and fail-closed validation for the ComponentSession protocol. |
| Integration | `d2b-contracts` v3 component_session module is consumed by `d2b-bus`, `d2b-session`, `d2b-session-unix`, `d2b-client`, and provider toolkit for shared wire constants/enums. |
| Data migration | Full d2b 3.0 reset; no v2 ComponentSession contract state import |
| Validation | Port component session and Noise vector tests; add wire-tag stability gates for renamed role/service/purpose variants. |
| Removal proof | ADR45 realm-named enum variants are renamed only if the owning spec requires while preserving wire tags; no prior v3 contract owner to remove. |
| Main commit source | `packages/d2b-contracts/src/v2_component_session.rs` (entire file, 2500+ lines at `a1cc0b2d`): wire constants — `PREFACE_MAGIC=*b"D2BCS2\r\n"`, `COMPONENT_SESSION_MAJOR=2`, `COMPONENT_SESSION_MINOR=0`, all `MAX_*` limits, `FRAGMENT_HEADER_LEN=24`, `RECORD_HEADER_LEN=24`; structs — `BoundedVec<T,MIN,MAX>`, `ComponentSessionPreface`, `HandshakeOffer`, `EndpointPolicy`, `EndpointPolicyIdentity`, `LimitProfile`, `TransportBinding`, `AttachmentPolicy`, `RequestEnvelope`, `AdmittedDeadline`, `RecordHeader`, `FragmentHeader`, `SendSequence`, `ReceiveSequence`, `KeepaliveRecord`, `CloseRecord`, `GuestSessionCredentialV1`, `GuestBootstrapCredentialV1`, `GuestBootstrapPsk`, `BootstrapPskBinding`, `BootstrapPskState`; enums — `EndpointPurpose`(19 variants), `PurposeClass`(3), `EndpointRole`(19), `ServicePackage`(15), `NoiseProfile`(3), `Locality`(4), `TransportClass`(5+), `AttachmentKind`, `KernelObjectType`, `CloseReason`, `ContractError`, `BinaryError`; closed-enum wire tag codec |
| Tests at main | `packages/d2b-session/tests/component_session.rs` (wire protocol round-trip); `packages/d2b-session/tests/noise_vectors.rs` (KAT from `docs/reference/component-session-v2-vectors.json`) |
| Selected behavior | Canonical wire values and fail-closed validation for the session protocol; `BoundedVec` serde+JsonSchema; all binary size constants are stable wire commitments and must not change without a `COMPONENT_SESSION_MAJOR` bump |
| v3 destination | `packages/d2b-contracts/src/v3/component_session.rs`; `COMPONENT_SESSION_MAJOR` stays 2 unless the wire handshake format changes; KAT vectors in `docs/reference/component-session-v2-vectors.json` must still pass after copy |
| ADR45 exclusions | `EndpointRole` variants `RealmController`(3), `RealmBroker`(5): may rename per ADR-046-componentsession-and-bus owning spec; wire tag values 3 and 5 are stable and must not change. `ServicePackage` variants `RealmV2`(2, `"d2b.realm.v2"`) and `DaemonV2`(1): `RealmV2` may rename per owning spec; wire tag 2 is stable. `EndpointPurpose` variants `RealmPeer`(3), `RealmBootstrap`(4): may rename per owning spec; wire tags 3 and 4 are stable. `PurposeClass` and `Locality` variant names: confirm per owning spec; no "realm" prefix in either enum so rename is unlikely but must be verified before adoption |

### ADR046-nix-028

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-027; Zone service naming per ADR-046-componentsession-and-bus owning spec |
| Current source | Main commit source row below: `packages/d2b-contracts/src/generated_v2_services/`, `v2_guest_services.rs`, and `v2_component_session.rs` service inventory at `a1cc0b2d`. |
| Reuse action | Reuse generated ttrpc service tables from main commit `a1cc0b2d`; adapt ADR45 Realm/Workload fields and service names to v3 Zone/Resource naming per owning bus spec. |
| Destination | `packages/d2b-contracts/src/v3/services/`. |
| Detailed design | Versioned ttrpc service modules preserve server dispatch tables, shared request/cancel/outcome/error types, and service fingerprint inventory. |
| Integration | Generated v3 services feed `d2b-bus` server/client dispatch, provider-toolkit conformance, and `SERVICE_INVENTORY` fingerprint validation. |
| Data migration | Full d2b 3.0 reset; no v2 generated service state import |
| Validation | Port provider-toolkit conformance and service fingerprint assertions; add v3 service-name gate for Zone service and ResourceName field adaptation. |
| Removal proof | ADR45 realm/user WorkloadId fields listed in exclusions are adapted to Zone/ResourceName; `generated_v2_services` remains only as reuse source. |
| Main commit source | `packages/d2b-contracts/src/generated_v2_services/` (48 files at `a1cc0b2d`): `activation.rs`+`_ttrpc.rs`, `broker.rs`+`_ttrpc.rs`, `clipboard.rs`+`_ttrpc.rs`, `clipboard_picker.rs`+`_ttrpc.rs`, `daemon.rs`+`_ttrpc.rs`, `guest.rs`+`_ttrpc.rs`, `notify.rs`+`_ttrpc.rs`, `provider_audio.rs`+`_ttrpc.rs`, `provider_credential.rs`+`_ttrpc.rs`, `provider_device.rs`+`_ttrpc.rs`, `provider_display.rs`+`_ttrpc.rs`, `provider_infrastructure.rs`+`_ttrpc.rs`, `provider_network.rs`+`_ttrpc.rs`, `provider_observability.rs`+`_ttrpc.rs`, `provider_runtime.rs`+`_ttrpc.rs`, `provider_storage.rs`+`_ttrpc.rs`, `provider_substrate.rs`+`_ttrpc.rs`, `provider_transport.rs`+`_ttrpc.rs`, `realm.rs`+`_ttrpc.rs`, `runtime_systemd_user.rs`+`_ttrpc.rs`, `security_key.rs`+`_ttrpc.rs`, `shell.rs`+`_ttrpc.rs`, `terminal.rs`, `tty.rs`+`_ttrpc.rs`, `user.rs`+`_ttrpc.rs`, `wayland.rs`+`_ttrpc.rs`, `common.rs`, `mod.rs`; also `packages/d2b-contracts/src/v2_guest_services.rs`, `v2_component_session.rs`'s `SERVICE_INVENTORY` + fingerprint functions |
| Tests at main | `packages/d2b-provider-toolkit/tests/conformance.rs` (all 11 provider-type axes); `packages/d2b-session/tests/component_session.rs` (service fingerprint assertions) |
| Selected behavior | Each `*_ttrpc.rs` defines the ttrpc `Arc<dyn XxxTtrpc>` server type and method dispatch table; `common.rs` defines shared `RequestMetadata`, `CancelRequest`/`CancelResponse`, `Outcome`, `ErrorKind`; `SERVICE_INVENTORY` indexes all services for schema-fingerprint verification |
| v3 destination | `packages/d2b-contracts/src/v3/services/` (versioned sub-path); service interfaces adopted as-is initially; breaking method changes require a new proto major version |
| ADR45 exclusions | `realm.rs` service `"d2b.realm.v2.RealmService"` and its `RealmId`-typed fields → rename per ADR-046-componentsession-and-bus owning spec; `user.rs` and `runtime_systemd_user.rs` reference `WorkloadId` in some method contexts → adapt to `ResourceName` on copy; the 11 provider service files (`provider_*.rs`) carry no realm naming and require no ADR45 adaptation |

### ADR046-nix-029

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-027; ADR046-nix-028; EndpointRole naming per ADR-046-componentsession-and-bus owning spec |
| Current source | Main commit source row below: `packages/d2b-provider/src/{registry,rpc,instance,context}.rs` at `a1cc0b2d`. |
| Reuse action | Reuse ProviderRegistry/RPC runtime from main commit `a1cc0b2d`; adapt `SessionIdentity` to Zone naming and update EndpointRole/ServicePackage names per owning bus spec. |
| Destination | `packages/d2b-provider/src/` (adapt in place). |
| Detailed design | ProviderRegistry snapshot admission/drain, bounded in-flight caps, typed RpcProviderProxy dispatch, and build-time RegistryLimits validation. |
| Integration | Zone controller publishes active ProviderRegistry snapshots; provider-toolkit RPC proxy admits sessions through `d2b-bus` and dispatches typed Provider operations. |
| Data migration | Full d2b 3.0 reset; no v2 ProviderRegistry runtime state import |
| Validation | Port `d2b-provider` `runtime.rs` tests; add v3 AdmissionOptions peer-role and SessionIdentity zone-name gates. |
| Removal proof | ADR45 role/service names are adapted per owning bus spec; no prior v3 ProviderRegistry owner to remove. |
| Main commit source | `packages/d2b-provider/src/registry.rs` (`ProviderRegistry`, `ProviderRegistryBuilder`, `RegistryLimits` defaults: total_in_flight=256, per_provider_in_flight=32, `AdmissionOptions`); `packages/d2b-provider/src/rpc.rs` (`AuthenticatedProviderRpc` trait, `RpcProviderProxy`, `RpcCall`, `RpcOperation` enum: Health/Capabilities/Method, `RpcPayload` enum: None/Operation/Plan/Adoption/LeaseRequest/Lease, `RpcResponse` enum: Health/Capabilities/Plan/Handle/Observation/ObservabilityQuery/Mutation/Lease, `SessionIdentity`, `ProviderClock`, `SystemProviderClock`); `packages/d2b-provider/src/instance.rs` (`ProviderInstance` enum, 11 variants: Runtime/Infrastructure/Transport/Substrate/Credential/Display/Network/Storage/Device/Audio/Observability); `packages/d2b-provider/src/context.rs` (`OwnedOperationContext`, `CancellationToken`) |
| Tests at main | `packages/d2b-provider/tests/runtime.rs` |
| Selected behavior | `ProviderRegistry` admits sessions against a versioned generational snapshot and drains before rotation; per-provider in-flight cap prevents one slow provider from consuming all capacity; `RpcProviderProxy` converts typed `RpcCall` into the correct per-ProviderType ttrpc service invocation through `AuthenticatedProviderRpc`; `RegistryLimits` validated at build time |
| v3 destination | `packages/d2b-provider/src/` (adapt in place); `ProviderRegistry` becomes the Zone controller's active Provider registry; `ProviderInstance` enum extended with new Provider types as dossiers are ratified |
| ADR45 exclusions | `AdmissionOptions.peer_role: EndpointRole` contains ADR45 role values — update variant names per ADR-046-componentsession-and-bus owning spec; `SessionIdentity` contains `provider_generation` and `service: ServicePackage` — the generation type is stable; `ServicePackage::ProviderV2` may rename per owning spec; `v2_identity::ProviderId`/`ProviderType` carry no realm naming and require no adaptation |

### ADR046-nix-030

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-029; ADR046-nix-028; EndpointRole and ServicePackage naming per ADR-046-componentsession-and-bus owning spec |
| Current source | Main commit source row below: `packages/d2b-provider-toolkit/src/{adapter,server,conformance,registration}.rs` at `a1cc0b2d`. |
| Reuse action | Reuse provider toolkit from main commit `a1cc0b2d`; adapt endpoint role/service variant names and v2_identity imports while preserving wire tags. |
| Destination | `packages/d2b-provider-toolkit/src/` (adapt in place). |
| Detailed design | ProviderAgentAdapter validates ComponentSession identity, GeneratedProviderServiceServer dispatches all ProviderType method families with bounded stores/in-flight state, and conformance kit gates every Provider implementation. |
| Integration | Provider processes embed `GeneratedProviderServiceServer`; `ProviderAgentAdapter` validates ComponentSession identity before invoking registry instances; conformance kit gates every Provider crate. |
| Data migration | Full d2b 3.0 reset; no v2 provider-toolkit runtime state import |
| Validation | Port `conformance.rs`; assert endpoint role/service wire tags remain 7/4 after any variant rename. |
| Removal proof | ADR45 `v2_identity` imports are adapted to ZoneId/ResourceName; no prior v3 toolkit owner to remove. |
| Main commit source | `packages/d2b-provider-toolkit/src/adapter.rs` (`ProviderAgentAdapter::new()` validates session identity — checks `peer_role == EndpointRole::ProviderAgent`, `service == ServicePackage::ProviderV2`, `binding.agent_generation == identity.provider_generation`; `invoke_session()` checks attachment index ordering); `packages/d2b-provider-toolkit/src/server.rs` (`GeneratedProviderServiceServer` with per-session object stores — `MAX_SESSION_PLANS=256`, `MAX_SESSION_HANDLES=1024`, `MAX_SESSION_LEASES=1024`, `MAX_AGENT_IN_FLIGHT=64`; atomic `accepting`/`in_flight`; `idle: Notify`; routes all 11 ProviderType ttrpc method families); `packages/d2b-provider-toolkit/src/conformance.rs` (`check_provider_conformance`, `check_descriptor_conformance`, `ConformanceError` enum); `packages/d2b-provider-toolkit/src/registration.rs` (`register_exact_instances`, `ToolkitError` enum) |
| Tests at main | `packages/d2b-provider-toolkit/tests/conformance.rs` — `every_axis_passes_identical_in_process_and_rpc_conformance` tests all 11 `ProviderType` variants both in-process and through the full RPC path |
| Selected behavior | `ProviderAgentAdapter` is the descriptor-bound validation gate between a ComponentSession and a provider instance; `GeneratedProviderServiceServer` is the agent-side ttrpc dispatch engine; conformance kit provides a reference test harness for every Provider implementation; `register_exact_instances` is the canonical pattern for building a test registry from static descriptors |
| v3 destination | `packages/d2b-provider-toolkit/src/` (adapt in place); conformance tests in `packages/d2b-provider-toolkit/tests/conformance.rs` must pass unchanged after the ADR45 exclusions are adapted |
| ADR45 exclusions | `ProviderAgentAdapter::new()` hard-checks `peer_role == EndpointRole::ProviderAgent` (tag 7) and `service == ServicePackage::ProviderV2` (tag 4) — update the Rust enum variant names if the owning spec renames them per ADR-046-componentsession-and-bus; wire tag values 7 and 4 must not change. `v2_identity::{RealmId, WorkloadId}` appear in test context imports — adapt `RealmId` → `ZoneId`, `WorkloadId` → `ResourceName` on copy |

### ADR046-bus-009

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-029; ADR046-nix-027; TargetInput v3 shape per ADR-046-componentsession-and-bus owning spec |
| Current source | Main commit source row below: `packages/d2b-client/src/{client,session,target,service,daemon_service,guest_service,host_socket}.rs` at `a1cc0b2d`. |
| Reuse action | Reuse async client from main commit `a1cc0b2d`; adapt TargetInput, ServiceOwner, service packages, and local socket identity to Zone/ResourceRef semantics. |
| Destination | `packages/d2b-client/src/` (adapt in place). |
| Detailed design | Transport-neutral typed async client, ComponentSessionConnector abstraction, precise SessionFailure classification, signed metadata envelopes, named-stream client abstraction, and host-socket connector. |
| Integration | CLI/controllers construct TargetInput/MetadataInput; `d2b-client` resolves Zone/Resource routes via `d2b-bus` and reuses ComponentSessionConnector for calls/streams. |
| Data migration | Full d2b 3.0 reset; no v2 client state/config import |
| Validation | Port `d2b-client` tests; add ResourceRef TargetInput and Zone bootstrap socket identity gates. |
| Removal proof | ADR45 TargetInput Realm/Workload forms are replaced by Zone/ResourceRef forms; no prior v3 client owner to remove. |
| Main commit source | `packages/d2b-client/src/client.rs` (`Client`, `ConnectedClient`, `MetadataInput`, `CallOptions`, `CancellationToken`, `Response`, `RetryPolicy`, `SystemClock`, `WallClock`); `packages/d2b-client/src/session.rs` (`ComponentSessionConnector` trait, `ConnectedSession` with `driver: SharedDriver` + `ttrpc_socket: Socket` + `limits: LimitProfile`, `SessionCall`, `SessionReply`, `SessionFailure` enum: BeforeDispatch/Retryable/Ambiguous/Disconnected/Deadline/Cancelled/Protocol, `NamedStream`, `SharedDriver`); `packages/d2b-client/src/target.rs` (`TargetInput` enum, `ServiceOwner`, `ResolvedTarget`, `RouteRecord`, `RouteTable`, `TransportKind`, `TransportSelection`); `packages/d2b-client/src/service.rs` (`GeneratedClient`, `MethodHandle`, `ServiceHandle`, `ServiceKind`); `packages/d2b-client/src/daemon_service.rs` (`DaemonClient`, `DaemonMethod`, `DaemonLifecycleRequest`, `DaemonTerminal`, `daemon_call_options`); `packages/d2b-client/src/guest_service.rs` (`GuestClient`, `GuestOperation`, `GuestCancelCall`, `GuestInspectCall`, `GuestRetainedLogCall`); `packages/d2b-client/src/host_socket.rs` (feature `host-socket`) — `HostSocketConnector`, `local_daemon_endpoint_identity` |
| Tests at main | `packages/d2b-client/tests/client.rs` |
| Selected behavior | Transport-neutral typed async client; `ComponentSessionConnector` abstracts connection setup; `SessionFailure` provides precise failure classification for retry policy; `MetadataInput` constructs signed request envelopes with clock-bounded lifetimes; `NamedStream` exposes named-stream channel as a client-side abstraction; `HostSocketConnector` is the reference Unix socket connection implementation |
| v3 destination | `packages/d2b-client/src/` (adapt in place); client becomes the primary CLI and controller access path for Zone-local and cross-Zone ComponentSession services |
| ADR45 exclusions | `TargetInput::Workload { realm: RealmId, workload: WorkloadId }` → v3 shape addresses resources as `ResourceRef` (e.g., `Zone/<z>`, `Guest/<name>`) per ADR-046-componentsession-and-bus owning spec; `TargetInput::Realm(RealmId)` → `TargetInput::Zone(ZoneId)`; `ServiceOwner::Workload { realm, workload }` → `ServiceOwner::Resource { zone: ZoneId, resource: ResourceName }`. `HostSocketConnector::local_daemon_endpoint_identity()` returns identity pinned to current `d2bd.socket` path — v3 socket path comes from Zone bootstrap config and must not be hard-coded. `DaemonClient`/`DaemonMethod` verb set per resource-api/authz foundation spec |

### ADR046-bus-010

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-023; ADR046-nix-027; naming and service name per ADR-046-componentsession-and-bus owning spec |
| Current source | Main commit source row below: `packages/d2b-realm-router/src/service_v2.rs` and tests at `a1cc0b2d`. |
| Reuse action | Reuse routing service behavior from main commit `a1cc0b2d`; rename RealmSessionAuthority/service identifiers to Zone terms per owning bus spec while preserving custody behavior. |
| Destination | `packages/d2b-bus/src/routing/zone_service.rs`. |
| Detailed design | ZoneSessionAuthority preserves no-host-credential custody, gateway/ZoneLink custody behavior, bounded concurrent dispatch, and graceful shutdown semantics. |
| Integration | `d2b-bus` Zone service handles route admission and shortcut lifecycle; ZoneSessionAuthority feeds session/audit/metric labels with stable custody state. |
| Data migration | Full d2b 3.0 reset; no v2 routing state import |
| Validation | Port `realm_service_v2.rs` and topology harness tests with Zone naming; add service-name wire gate. |
| Removal proof | ADR45 RealmService name and RealmId fields are adapted to Zone service; no prior v3 router owner to remove. |
| Main commit source | `packages/d2b-realm-router/src/service_v2.rs` (`RealmSessionAuthority` struct with `realm: RealmId`, `peer_role: EndpointRole`, `locality: Locality`, `purpose: PurposeClass`, `custody: CredentialCustody`; `CredentialCustody` enum: None/GatewayGuest; constructors `local_controller()`/`gateway_peer()`/`new()`; `REALM_SERVICE_NAME = "d2b.realm.v2.RealmService"`; constants `DEFAULT_MAX_REALM_BINDINGS=256`, `DEFAULT_MAX_SHORTCUTS=256`, `DEFAULT_MAX_MUTATION_RECORDS=1024`, `DEFAULT_AUDIT_CAPACITY=1024`; internal `MAX_DISPATCH_IN_FLIGHT=64`, `SHUTDOWN_TIMEOUT=5s`; `RealmServiceLimits`; dispatch using `Arc<Semaphore>(MAX_DISPATCH_IN_FLIGHT)` and `JoinSet<()>`); `packages/d2b-realm-router/tests/realm_service_v2.rs`; `packages/d2b-realm-router/tests/transport_topology_harness.rs` |
| Tests at main | `packages/d2b-realm-router/tests/realm_service_v2.rs` — routing service tests; `packages/d2b-realm-router/tests/transport_topology_harness.rs` — topology harness |
| Selected behavior | `RealmSessionAuthority` enforces that host-local sessions hold no realm credentials (`CredentialCustody::None`) while gateway sessions hold `GatewayGuest` custody — this is the runtime enforcement of ADR 0032 "relay identity is not local auth"; concurrent dispatch with `Semaphore(64)` bound; 5-second graceful shutdown via `JoinSet` |
| v3 destination | `packages/d2b-bus/src/routing/zone_service.rs`; `RealmSessionAuthority` renames to `ZoneSessionAuthority`; `CredentialCustody` is behavior-stable and requires no rename; `REALM_SERVICE_NAME` updates per ADR-046-componentsession-and-bus owning spec |
| ADR45 exclusions | `realm: RealmId` field in `RealmSessionAuthority` → `zone: ZoneId`; `REALM_SERVICE_NAME = "d2b.realm.v2.RealmService"` → v3 service name per ADR-046-componentsession-and-bus owning spec; `EndpointRole::LocalRootController`, `RealmController`, `RemotePeer` used in `new()` validation — may rename per owning spec; wire tags remain stable; `PurposeClass::Local`/`Enrolled`/`Bootstrap` — confirm per owning spec |

### ADR046-bus-011

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-nix-029; ADR046-nix-028; ADR046-nix-027; PROVIDER_BUNDLE_VERSION bump required on any bundle artifact format change |
| Current source | Main commit source row below: `packages/d2bd/src/provider_registry.rs` at `a1cc0b2d`. |
| Reuse action | Reuse fail-closed provider composition from main commit `a1cc0b2d`; adapt v2 identity/bundle artifact types to v3 provider registry contracts and bump bundle versions. |
| Destination | `packages/d2bd/src/provider_registry.rs` (adapt in place). |
| Detailed design | Named ProviderCompositionError variants, first-party factory instantiation, bundle resolver validation, version/schema constants, and monotone lifecycle operation IDs. |
| Integration | `provider_registry.rs` composes the v3 provider catalog into active ProviderRegistry snapshots consumed by `d2b-bus`/provider runtime and bundle validation. |
| Data migration | Full d2b 3.0 reset; provider bundle format bumps to v3 and no v2 provider registry state import. |
| Validation | Port provider_registry composition tests; add PROVIDER_BUNDLE_VERSION/SCHEMA_VERSION v3 bump gate and ZoneId/ResourceName binding test. |
| Removal proof | ADR45 `provider_registry_v2` bundle artifact types are replaced by `d2b_contracts::v3::provider_registry` after v3 bundle schema lands. |
| Main commit source | `packages/d2bd/src/provider_registry.rs` (`ProviderCompositionError` enum with 26 named variants including `AzureVmForbidden`, `LegacyRunnerForbidden`, `NondispatchableCapability`, `ProcessIdentityMismatch`, `LifecycleBudgetExceeded`; all first-party factory instantiations: `PipewireVhostUserAudioFactory`, `HostMediatedDeviceFactory`, `WaylandDisplayFactory`, `LocalRealmNetworkFactory`, `LocalObservabilityFactory`, `AzureContainerAppsRuntimeProviderFactory`, `LocalRuntimeProviderFactory` CH/QEMU/systemd-user, `LocalStorageFactory`, `HostSubstrateProviderFactory` Linux/NixOS, `AzureRelayProviderFactory`, `LocalTransportFactory`; constants `PROVIDER_BUNDLE_VERSION: u32 = 13`, `PROVIDER_BUNDLE_SCHEMA_VERSION: &str = "v2"`, `AZURE_VM_IMPLEMENTATION_ID: &str = "azure-vm"`, static `NEXT_LIFECYCLE_OPERATION_ID: AtomicU64`) |
| Tests at main | `packages/d2bd/src/` integration tests exercising composition (search `#[cfg(test)]` blocks in `provider_registry.rs`) |
| Selected behavior | Fail-closed composition: every error is named; `AzureVmForbidden` explicitly rejects non-production implementations; bundle loaded through `load_bundle_resolver()` and validated against `PROVIDER_BUNDLE_VERSION`; `NEXT_LIFECYCLE_OPERATION_ID` provides monotone IDs across restarts |
| v3 destination | `packages/d2bd/src/provider_registry.rs` (adapt in place); `PROVIDER_BUNDLE_VERSION` bumps when bundle artifact format changes; `PROVIDER_BUNDLE_SCHEMA_VERSION` updates from `"v2"` to `"v3"`; `ProviderCompositionError` variants retained with v3-specific variants added |
| ADR45 exclusions | Uses `d2b_contracts::v2_identity::{RealmId, WorkloadId, RealmPath as ProviderRealmPath}` in binding contexts → adapt to `ZoneId`/`ResourceName`; `d2b_contracts::provider_registry_v2` module types (`ProviderBindingV2ConsumerView`, `ProviderRegistryEntryV2`, `ProviderRegistryV2`) are ADR45 bundle artifact types → v3 replaces with `d2b_contracts::v3::provider_registry`; `PROVIDER_BUNDLE_VERSION = 13` is the ADR45 pinned version — a bump is required before v3 adoption; numeric value is determined when the v3 bundle format is finalized in this work item |

### ADR046-bus-012

| Field | Value |
| --- | --- |
| Dependency/owner | ADR046-bus-011; ADR046-nix-029; GuestLifecycleRequest ResourceRef addressing per ADR-046-componentsession-and-bus owning spec |
| Current source | Main commit source row below: `packages/d2bd/src/provider_effects.rs` at `a1cc0b2d`. |
| Reuse action | Reuse descriptor-bound effect dispatch from main commit `a1cc0b2d`; adapt lifecycle request wire type from VM name to Zone-scoped Guest ResourceRef. |
| Destination | `packages/d2bd/src/provider_effects.rs` (adapt in place). |
| Detailed design | DaemonEffectAdapters bind semantic effect ports; ProviderLifecycleDispatch bounds lifecycle mutations and routes broker start/stop through blocking adapters with idempotency-key deduplication. |
| Integration | `ProviderLifecycleDispatch` receives v3 Guest lifecycle operations from `d2b-bus`/core controller and routes broker start/stop through descriptor-bound effect ports. |
| Data migration | Full d2b 3.0 reset; no v2 lifecycle mutation state import |
| Validation | Port `provider_effects.rs` lifecycle dispatch tests; add ResourceRef Guest lifecycle request and BrokerCallerRole compatibility gates. |
| Removal proof | `VmLifecycleRequest`/`vm_name` path is replaced by a v3 ResourceRef Guest lifecycle op after broker/public wire contract versioning. |
| Main commit source | `packages/d2bd/src/provider_effects.rs` (`DaemonEffectAdapters` struct binding all semantic port traits; `ProviderLifecycleDispatch` with `MAX_TRACKED_LIFECYCLE_MUTATIONS=256` and `BTreeMap<String, ProviderLifecycleInvocation>`; all effect port imports: `AudioEffectPort`/`AudioQueryPort`, `DeviceEffectPort`/`DeviceQueryPort`, `DisplayEffectPort`, `NetworkEffectPort`, `BoundedExportSink`/`ObservabilityExportPort`/`ObservabilityQueryPort`, `RuntimeAdoptionControl`/`RuntimeConfiguredItemControl`/`RuntimeControlPort`/`RuntimeEnsureControl`/`RuntimeOperationControl`/`RuntimePlanDecision`, `StorageEffectPort`, `HostSubstratePort`, `LocalEndpointPort`; lifecycle dispatch functions `dispatch_broker_vm_start_on_blocking_adapter`, `dispatch_broker_vm_stop_on_blocking_adapter`; test helpers `reset_test_runtime_lifecycle_calls()`, `test_runtime_lifecycle_calls()`, `TEST_RUNTIME_START_CALLS`/`TEST_RUNTIME_STOP_CALLS` thread-locals) |
| Tests at main | `packages/d2bd/src/provider_effects.rs` `#[cfg(test)]` blocks; `packages/d2bd/src/provider_registry.rs` composition tests |
| Selected behavior | Each effect adapter is descriptor-bound at composition time in `provider_registry.rs`; `ProviderLifecycleDispatch` tracks in-flight lifecycle mutations with a bounded BTreeMap and idempotency-keyed deduplication; `dispatch_broker_vm_start/stop_on_blocking_adapter` routes to the broker via a blocking task adapter; test helpers provide per-test reset of lifecycle call counters |
| v3 destination | `packages/d2bd/src/provider_effects.rs` (adapt in place); effect port bindings retained; lifecycle dispatch updated from `VmLifecycleRequest` to a v3 Guest lifecycle op addressed by `ResourceRef` (`Guest/<name>`) with Zone context (`Zone/<z>`); exact wire type per ADR-046-componentsession-and-bus owning spec |
| ADR45 exclusions | `VmLifecycleRequest` from `d2b-contracts/src/public_wire.rs` uses `vm_name: String` (ADR45 daemon wire); v3 requires a `GuestLifecycleRequest` or equivalent with `ResourceRef` addressing; `BrokerCallerRole` from `broker_wire.rs` is an ADR45 broker identity type — keep in place and flag for broker wire update when broker contract is versioned; `DaemonAuditSinkStatus` references current audit shape — keep until v3 audit contract is defined |
