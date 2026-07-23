# ADR 0046 telemetry, audit, and support

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-telemetry-audit-and-support` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 2 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | `d2b-telemetry`, `d2b-audit`, `d2b-provider-observability-otel`, `d2b zone` CLI |
| Depends on | `ADR-046-terminology-and-identities`, `ADR-046-resource-object-model`, `ADR-046-resource-store-redb`, `ADR-046-componentsession-and-bus`, `ADR-046-core-controllers`, `ADR-046-components-processes-and-sandbox`, `ADR-046-provider-model-and-packaging` |
| Supersedes | Current `d2bd` hand-rolled Prometheus registry; current daemon/broker/gateway JSONL audit paths |

## Terminology mapping: baseline names → v3 targets

The pre-ADR45 v3 baseline uses the following names. This spec cites baseline
code with its actual names and explicitly states the v3 target name at each
design boundary. All claims about current behavior derive from the exact
baseline symbols named below.

| Baseline name / symbol | v3 ADR 0046 target | Evidence class |
| --- | --- | --- |
| `RealmPath` (`d2b-realm-core/src/realm.rs`) | Zone name (string, name of `Zone/<name>` self resource) | implemented-and-reachable |
| `RealmId` (`d2b-realm-core/src/ids.rs`) | Component of Zone name | implemented-and-reachable |
| `WorkloadId` (`d2b-realm-core/src/ids.rs`) | Opaque resource UID (for `Process`, `Guest`, or null) | implemented-and-reachable |
| `NodeId` (`d2b-realm-core/src/ids.rs`) | Not retained as a standalone audit field; resolved through `Host/<name>` or `Guest/<name>` resource references | implemented-and-reachable |
| `PrincipalId` (`d2b-realm-core/src/ids.rs`) | `subject_digest: sha256:<hex>` in v3 audit records | implemented-and-reachable |
| `AuditEnvelope.realm: RealmPath` | `zone: <zone_name>` in v3 audit record | implemented-and-reachable |
| `AuditEnvelope.node: NodeId` | Not a standalone field; execution context resolved from resource | implemented-and-reachable |
| `AuditEnvelope.workload: WorkloadId` | Opaque resource UID (for Process under the operation, or null) | implemented-and-reachable |
| `AuditEnvelope.principal: PrincipalId` | `subject_digest: sha256:<hex>` | implemented-and-reachable |
| `AuditStreamKind::Daemon` | Zone-local audit stream (Zone runtime process) | implemented-and-reachable |
| `AuditStreamKind::Gateway` | ZoneLink-boundary audit stream (gateway-backed realm → ZoneLink) | implemented-and-reachable |
| `AuditStreamKind::RemoteNode` | RemoteZone audit stream (cross-Zone link) | implemented-and-reachable |
| `VmProcessDag` / `ProcessNode` / `ProcessRole` (`d2b-core/src/processes.rs`) | `Process` or `EphemeralProcess` resource; or fixed bootstrap; per role-disposition table in ADR-046-components-processes-and-sandbox | implemented-and-reachable |
| `ProcessRole::OtelHostBridge` | `Process` resource under `observability-otel` Provider | implemented-and-reachable |
| `RunnerRole::OtelHostBridge` (`d2b-contracts/src/broker_wire.rs`) | `Process` resource under `observability-otel` Provider; broker `SpawnRunner` becomes Provider supervisor ticket | implemented-and-reachable |
| `RunnerRole::CloudHypervisor`, `QemuMedia`, `Virtiofsd`, `Swtpm`, etc. | `Process` or `EphemeralProcess` under each VM/Device Provider; see ADR-046-components-processes-and-sandbox | implemented-and-reachable |
| `WorkloadIdentity` / `WorkloadTarget` / `RealmTarget` (`d2b-core/src/workload_identity.rs`) | Zone self-resource reference `Zone/<zone_name>` | implemented-and-reachable |
| `d2b.realms` Nix option (`nixos-modules/options-realms.nix`) | `d2b.zones` Nix option (ADR-only target) | generated-or-eval-contract |
| `realm-controllers.json` bundle artifact | Zone runtime config (new generated artifact; existing file is retired) | generated-or-eval-contract |
| `d2b_daemon_vm_*` metrics with `vm` label (`packages/d2bd/src/metrics.rs`) | `vm` label (VM name) removed from v3 metric labels; VM-identity carried only in OTEL resource attributes and trace context | implemented-and-reachable |
| `vm.name`, `vm.env`, `vm.role` OTEL resource attributes (`nixos-modules/components/observability/{host,stack,guest}.nix`) | Preserved in v3 (advisory from edge collector; re-stamped at ingress boundary). Extended with `d2b.zone`, `d2b.provider`, `d2b.component` (ADR-only additions) | implemented-and-reachable |
| `d2b.observability.vmName` / `identityName` Nix options | `d2b.zones.<name>.observability.*` Nix options (ADR-only target) | generated-or-eval-contract |
| `config_source = "realm-controllers"` tracing field (`d2b-priv-broker/src/runtime.rs`) | `config_source = "zone-config"` in v3 startup tracing | implemented-and-reachable |
| `d2b-clipd/src/audit.rs::AuditEvent.source_realm`, `.destination_realm` | `source_zone`, `destination_zone` (cross-Zone clipboard audit) | implemented-and-reachable |
| `kind = "unsafe-local"` workload (`nixos-modules/options-realms-workloads.nix:221,233`) | `Host/<name>` resource — user-only, **no isolation boundary**; reconciled by `Provider/system-core` with `defaultDomain=user`, `allowedDomains=[user]`, `defaultUserRef=User/<name>`; child processes use normal Process Providers; **not** a v3 Provider | implemented-and-reachable |
| `UnsafeLocalWorkloadsJson` / `UnsafeLocalWorkload` / `UnsafeLocalLauncherItem` (`packages/d2b-core/src/unsafe_local_workloads.rs`) | `Host` resource spec serialized in the private bundle; `UnsafeLocalWorkload.identity.runtime_kind = "unsafe-local"` / `provider_id = "unsafe-local"` → `Provider/system-core` catalog entry | implemented-and-reachable |
| `HelperRegistry` / `HelperConnection` / `dispatch_launch` (`packages/d2bd/src/unsafe_local_helper.rs`) | user-domain process supervision; `HelperRegistry::allowed_uids` → `defaultUserRef=User/<name>` constraint; v3 replaces with normal Process Provider supervisor ticket | implemented-and-reachable |
| `DaemonToUnsafeLocalHelper` / `UnsafeLocalHelperToDaemon` / `HelperLaunchRequest` / `HelperShellRequest` (`packages/d2b-contracts/src/unsafe_local_wire.rs`) | internal launch/shell protocol between `d2bd` and the helper binary; retired in v3 when launch moves to Process Provider supervisor ticket | implemented-and-reachable |
| `d2b-unsafe-local-helper` binary (`packages/d2b-unsafe-local-helper/src/{main,protocol,runtime,systemd}.rs`) | fixed user-domain supervisor process; v3 equivalent is a user-domain `Process` under `Provider/system-core` | implemented-and-reachable |
| `nixos-modules/unsafe-local-workloads-json.nix` (`runtimeKind = "unsafe-local"`, `providerId = "unsafe-local"`) | Nix emitter for the private bundle artifact; v3 target is the `Provider/system-core` `Host` resource spec | generated-or-eval-contract |
| `nixos-modules/unsafe-local-helper.nix` (service unit for `d2b-unsafe-local-helper`) | fixed user-supervisor Nix unit; retired after Process Provider supervisor ticket migration | generated-or-eval-contract |

## SDK placement: resolved

Zone/core processes (Zone runtime, core-controller, mandatory Providers) use
**lightweight bounded emitters** — `tracing` + a bounded in-process ring — to
push telemetry frames over a private local Unix datagram socket. They carry no
`opentelemetry_sdk` or `opentelemetry-otlp` dependency. This matches the current
v3 baseline (`d2bd` uses only `tracing` crate; no OTEL SDK present at
`packages/d2bd/src/lib.rs` or `packages/d2b-priv-broker/src/runtime.rs`).

`Provider/observability-otel` is an **ordinary optional Process** that runs the
full OTEL SDK with an OTLP/gRPC exporter. It owns the per-Zone OTLP receiver
socket, drains the emitter ring, and forwards to the SigNoz backend. Because it
is an optional non-bootstrap Process:

- It does **not** count toward the mandatory ≤64 MiB core aggregate defined in
  ADR-046-resource-store-redb. That budget is unchanged.
- Zone runtime and core-controller startup proceed before and without it.
- If the `observability-otel` Provider is absent, unready, or crashes: the
  emitter ring fills, oldest frames are dropped, `d2b_telemetry_drop_total`
  increments, and Zone/controller health transitions to `Degraded` (not
  `Failed`). Authoritative audit is unaffected.

Current-code evidence: `d2bd` uses `tracing` crate exclusively for structured
logging/tracing (`packages/d2bd/src/lib.rs` lines 720+,
`packages/d2b-priv-broker/src/runtime.rs` lines 34–35). No `opentelemetry_sdk`
crate exists in the v3 baseline. Hand-rolled Prometheus registry in
`packages/d2bd/src/metrics.rs` (no OTEL SDK). This resolved design requires no
ADR-046-resource-store-redb budget revision.

## Separation invariant

Telemetry (OTEL metrics/traces/logs) and authoritative audit are two distinct
subsystems with no shared writer path.

- OTEL telemetry is performance and health observability. It is best-effort,
  buffered, and lossy under back-pressure. No OTEL field carries event payload,
  authorization decision text, resource spec/status bytes, argv, secrets, paths,
  or subject names. OTEL data is exported through the `observability-otel`
  Provider and is never an authz input.
- Authoritative audit is a durable tamper-evident record of security-relevant
  decisions. Audit records must be committed before the operation they describe
  completes. Audit is never a telemetry stream and never enters an OTEL
  pipeline.
- OTEL spans and audit records share an opaque `operation_id` / `correlation_id`
  for cross-system joining. Neither direction carries the other's payload.

Both subsystems must fail safely and independently. OTEL unavailability never
blocks mutations, reconciliation, or process launch. Audit unavailability for
privileged records fails the operation closed; see durability class policy
below.

## OTEL resource attributes

### Current baseline attribute set (implemented-and-reachable)

`packages/d2b-contract-tests/tests/policy_observability.rs::loki_native_otel_resource_attributes`
enforces a closed allowlist:

```
deployment.environment, host.name, service.name, service.namespace,
source, vm.env, vm.name, vm.role
```

Required keys: `service.name`, `vm.env`, `vm.name`, `vm.role`.

These are stamped advisorily by each process/collector. The SigNoz OTel
Collector re-stamps them authoritatively at the trusted ingress boundary
(ADR 0026/0033 contract, preserved in v3).

The current `vm.name` carries the VM name (from `d2b.vms.<vm>` in
`d2b_daemon_vm_*` metrics and the Nix `identityName`/`vmName` Nix options).
In v3, a VM (current `d2b.vms.<vm>`) whose execution is VM-backed becomes a
`Guest/<name>` resource. VM names remain as advisory `vm.name` values because
this is an OTEL resource attribute, not a metric label.

### v3 target attribute additions (ADR-only)

The v3 `d2b-telemetry` crate extends the allowlist with these additional keys:

| Attribute | Source | Values |
| --- | --- | --- |
| `d2b.zone` | Zone name string (matches `Zone/<name>` self resource name) | Advisory; re-stamped at ingress |
| `d2b.provider` | Provider name (from closed Provider name catalog) | Provider processes only |
| `d2b.component` | Component ID (from signed component descriptor) | Controller/service/worker only |
| `service.version` | `CARGO_PKG_VERSION` | All processes |

The existing `vm.name`/`vm.env`/`vm.role`/`host.name`/`service.name`
keys are **preserved unchanged** in the v3 allowlist. No key is removed.
No key outside the allowlist may be stamped by any v3 process; the
`policy_observability.rs::loki_native_otel_resource_attributes` test is
adapted to include the new keys.

`d2b.zone` (Zone name) is allowed in resource attributes but **not** in
metric label values; see cardinality rules below.

## Host resource (unsafe-local) posture requirements

### Baseline identity

The current `kind = "unsafe-local"` workload in `nixos-modules/options-realms-workloads.nix` is the
**only** current workload kind that runs as the authenticated user with no isolation boundary.
Its Nix description (line 233–235) reads:

> `unsafe-local` — Host-user process runtime with no isolation boundary. Requires explicit realm policy opt-in.

The Nix module explicitly records that `stateDir` and `runDir` are null for this kind (lines
264–275): there is no host VM state path and no `/run/d2b/vms/<id>` runtime directory, because
user scopes are owned by the authenticated user's systemd manager.

In v3, this maps to a `Host/<name>` resource. It is:

- **Not** a `Guest` (no VM/sandbox execution boundary).
- **Not** a v3 Provider. `Provider/system-core` is the reconciler, not the execution substrate.
- Reconciled by `Provider/system-core` with `defaultDomain=user`, `allowedDomains=[user]`,
  `defaultUserRef=User/<name>`.
- Child processes launched via normal Process Providers (`provider=system-core-user`,
  `domain=user`).
- Explicitly no-isolation: user processes share the host UID, filesystem, and environment.

The "no isolation boundary" is a **first-class semantic property** of the
user-only `Host` resource (the unsafe-local successor with
`defaultDomain=user`, `allowedDomains=[user]`), not an implementation gap or
transitional state. It does not apply to other `Host` ResourceType variants
that may carry different execution policies. v3 must make this explicit and
persistent in four surfaces.

### Host resource status

The `Provider/system-core` reconciler sets a non-negotiable `isolationPosture`
status field on every user-only `Host` resource at reconciliation time.
Operators cannot suppress or override it:

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "type": "Host",
  "status": {
    "phase": "Ready",
    "isolationPosture": "none",
    "isolationPostureMessage": "This host resource runs processes as the authenticated user with no isolation boundary. All child processes share the host user environment."
  }
}
```

`isolationPosture: "none"` is set on user-only Host resources
(`defaultDomain=user`, `allowedDomains=[user]`). It is not set on Host
resources with other execution policies. Any resource status snapshot in a
support bundle must include this field for user-only Hosts; it is not redacted.

### CLI/UI

`d2b zone list` and `d2b zone inspect` must render a visible warning line for
any Zone containing a user-only `Host` resource
(`isolationPosture: "none"`). Example rendering:

```
  Host/laptop-shell   Ready   ⚠ no isolation boundary (user domain)
```

The warning annotation must not be omittable via a flag or environment
variable. The warning text must not carry the resource name, user name, UID,
or executable path. Host resources without `isolationPosture: "none"` do not
emit this annotation.

`d2b zone doctor` includes the following named check for each user-only `Host`
resource:

| Check name | Pass condition |
| --- | --- |
| `isolation-posture-declared` | User-only `Host` resource status has `isolationPosture: "none"` set by the reconciler |

### Audit: ProcessEffect for Host processes

Every process launch and stop under a `Host` resource emits a `ProcessEffect` record.
The record carries `domain=user` and `no_isolation=true`:

```json
{
  "record_class": "process-effect",
  "process_effect_fields": {
    "event":                 "launch|stop|adopt|quarantine",
    "provider":              "system-core-user",
    "domain":                "user",
    "no_isolation":          true,
    "execution_ref_digest":  "sha256:<hex>",
    "process_uid":           "<opaque uid>",
    "outcome":               "ok|error",
    "exit_class":            "exited|signaled|killed|null"
  }
}
```

`no_isolation: true` is set whenever `domain=user` and the parent `Host`
resource has `isolationPosture: "none"` (i.e., user-only Hosts). It is a
**required field** for these records; audit consumers must not need to join
the resource status to determine the isolation posture of a record.

**Current gap**: The existing `DaemonEvent` enum in `packages/d2bd/src/daemon_audit.rs` has
no dedicated `unsafe-local` launch or stop audit event. The `HelperRegistry::dispatch_launch`
path (`packages/d2bd/src/unsafe_local_helper.rs`) does not emit a `DaemonEvent`. This gap
is a required correction in v3: every user-only Host process launch and stop
must emit a `ProcessEffect` record with `no_isolation: true`.

### OTEL telemetry for Host processes

Metric labels for Host processes:

- `provider=system-core-user` (closed-set value; the controller handler name
  `system_core_user` in the handler closed set maps to this label value)
- `domain=user`

`no_isolation=true` is **not** a metric label. It is carried only in resource status and
audit records. It must not appear as a span attribute, log field, or metric label value.

OTEL resource attributes for Host processes:

- `service.name`: fixed name for the system-core Provider (e.g., `d2b-provider-system-core`)
- `d2b.zone`: Zone name (resource attribute; not a metric label)
- `d2b.provider`: `system-core` (closed-set provider name)
- `d2b.component`: controller/service component ID

No `vm.name`, no user name, no UID, no argv, no path appears in resource attributes or
span attributes for Host processes.

## Metrics

### Cardinality rules

All metric label values must come from a **closed set enumerated in this spec**.
The following are unconditionally forbidden as metric label values:

- VM names, Zone names, Provider names, resource names (`metadata.name` values)
  — these appear only in OTEL resource attributes
- Zone/Provider/Process UIDs
- Host/Guest/User/Volume/Network/Device names
- Filesystem paths, socket paths, executable paths
- argv or environment values
- Status detail messages or outcome text beyond stable error codes
- Subject names or principal identifiers
- PID, pidfd, or cgroup path values
- Operation IDs or correlation IDs (allowed in trace span attributes, not metric labels)
- Endpoint addresses, port numbers, or IP addresses

Note: the current `d2b_daemon_vm_*` metrics in `packages/d2bd/src/metrics.rs`
use `vm` labels with VM name values (e.g. labels `["vm", "state"]`,
`["vm", "outcome"]`, `["vm", "vmm", "outcome"]`, `["vm", "reason"]`).
These existing metrics are **not adopted into v3**. They are retained in d2bd
only until that daemon is superseded; v3 metrics carry no resource-name labels.

### Standard instruments

#### Zone runtime and resource store

Target crate: `d2b-resource-store-redb` (ADR-only). Current analog: none;
hand-rolled Prometheus registry in `packages/d2bd/src/metrics.rs` covers
daemon-level VM lifecycle, not a generic resource store.

| Metric | Type | Labels | Buckets (s) |
| --- | --- | --- | --- |
| `d2b_store_write_duration_seconds` | histogram | `kind={single,group}`, `outcome={ok,conflict,error}` | 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.5, 1.0 |
| `d2b_store_read_duration_seconds` | histogram | `op={get,list,scan}` | 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1 |
| `d2b_store_group_commit_size` | histogram | (none) | 1, 2, 4, 8, 16, 32, 64 |
| `d2b_store_conflict_total` | counter | `resource_type` | — |
| `d2b_store_watch_active` | gauge | (none) | — |
| `d2b_store_revision` | gauge | (none) | — |
| `d2b_store_compaction_duration_seconds` | histogram | `outcome={ok,error}` | 0.01, 0.05, 0.1, 0.5, 1.0, 5.0 |
| `d2b_store_backup_duration_seconds` | histogram | `outcome={ok,error}` | 0.1, 0.5, 1.0, 5.0, 10.0, 30.0 |
| `d2b_store_queue_depth` | gauge | `queue={write,read}` | — |

`resource_type` values come from the bound closed catalog short-name set.
Unknown or vendor-qualified types use the literal string `vendor`.

#### Resource API

Target crate: `d2b-resource-api` (ADR-only). No current analog.

| Metric | Type | Labels | Buckets (s) |
| --- | --- | --- | --- |
| `d2b_api_request_total` | counter | `verb={get,list,watch,create,update,patch,status,delete,finalize}`, `resource_type`, `outcome={ok,conflict,invalid,denied,not_found,quota,error}` | — |
| `d2b_api_request_duration_seconds` | histogram | `verb`, `resource_type` | 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.5 |
| `d2b_api_watch_active` | gauge | (none) | — |
| `d2b_api_admission_rejected_total` | counter | `reason={auth,quota,conflict,invalid,schema}` | — |

#### d2b-bus

Target crate: `d2b-bus` (ADR-only). Current analog: operation routing in
`d2b-realm-router/src/route_engine.rs` and `mux_session.rs`
(implemented-but-unwired as a generic bus).

| Metric | Type | Labels | Buckets (s) |
| --- | --- | --- | --- |
| `d2b_bus_route_total` | counter | `service`, `direction={local,host,guest,zone_link}`, `outcome={ok,denied,not_found,error}` | — |
| `d2b_bus_route_duration_seconds` | histogram | `service`, `direction` | 0.0001, 0.0005, 0.001, 0.005, 0.01, 0.05, 0.1 |
| `d2b_bus_session_active` | gauge | `transport={unix,vsock,zone_link}` | — |

`service` values are names from the closed bound service package catalog.
`direction=zone_link` replaces the current `AuditStreamKind::Gateway` /
`RemoteNode` distinction: gateway-backed realms (current `EntrypointMode::GatewayBacked`
in `d2b-realm-core/src/realm.rs`) become `ZoneLink`-connected Zones.

#### ComponentSession

Target crate: `d2b-session` (copied/adapted from main `a1cc0b2d` per
ADR-046-componentsession-and-bus). Current v3 analog:
`d2b-realm-router/src/secure_session.rs` and
`d2b-realm-router/src/mux_session.rs` (implemented-but-unwired for
generic ComponentSession).

| Metric | Type | Labels | Buckets (s) |
| --- | --- | --- | --- |
| `d2b_session_connect_total` | counter | `profile={NN,KK,IKpsk2}`, `purpose_class={local,enrolled,bootstrap}`, `outcome={ok,auth,transcript,policy,timeout,error}` | — |
| `d2b_session_reconnect_total` | counter | `outcome={ok,error,abandoned}` | — |
| `d2b_session_record_total` | counter | `direction={send,recv}`, `kind={control,ttrpc,stream,attachment}` | — |
| `d2b_session_active` | gauge | `transport={unix,vsock,zone_link}` | — |

#### Core controller

Target crate: `d2b-core-controller` (ADR-only). Current analog: `d2bd`
provides daemon-level VM lifecycle metrics through the hand-rolled registry
in `packages/d2bd/src/metrics.rs` (implemented-and-reachable), but carries
no per-handler controller metrics.

Key current metrics that inform bucket design:

- `d2b_daemon_broker_request_duration_seconds`: `BROKER_REQUEST_BUCKETS_SECONDS =
  [0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0]`
- `d2b_daemon_activation_phase_duration_seconds`: `ACTIVATION_PHASE_BUCKETS_SECONDS =
  [0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 30.0, 120.0, 600.0]`

| Metric | Type | Labels | Buckets (s) |
| --- | --- | --- | --- |
| `d2b_controller_reconcile_total` | counter | `handler`, `outcome={ok,requeue,conflict,error}` | — |
| `d2b_controller_reconcile_duration_seconds` | histogram | `handler`, `outcome` | 0.0005, 0.001, 0.005, 0.01, 0.025, 0.05, 0.1, 0.5, 2.0 |
| `d2b_controller_queue_depth` | gauge | `handler` | — |
| `d2b_controller_hint_to_handler_seconds` | histogram | `handler` | 0.001, 0.002, 0.005, 0.010, 0.015, 0.020, 0.030, 0.050 |
| `d2b_controller_watch_revision_lag` | gauge | `handler` | — |

`handler` values are the closed set defined in ADR-046-core-controllers:
`configuration`, `api_catalog`, `authz`, `provider`, `controller_registration`,
`ownership`, `watch_maintenance`, `ephemeral_cleanup`, `zone_link`, `budget`,
`store_lifecycle`, `system_core_host`, `system_core_user`.

`d2b_controller_hint_to_handler_seconds` measures the interval from durable
store commit to the first instruction of the matching controller handler. The
p95 hard target is ≤5 ms per ADR 0046.

#### Process Providers

Target crates: `d2b-provider-system-minijail` and `d2b-provider-system-systemd`
(ADR-only). Current analog: `d2bd` hand-rolled metrics for VM lifecycle:

- `d2b_daemon_vm_start_duration_seconds`: `VM_START_BUCKETS_SECONDS =
  [0.5, 1.0, 2.0, 5.0, 10.0, 20.0, 30.0, 60.0, 120.0, 300.0]`
  labels `["vm", "outcome"]` — current `vm` label carries a VM name
  (current `d2b.vms.<vm>` → target `Guest/<name>` or `Host/<name>`)
- `d2b_daemon_vm_shutdown_duration_seconds`: `VM_SHUTDOWN_BUCKETS_SECONDS`,
  labels `["vm", "vmm", "outcome"]` — `vmm` is the current `RunnerRole`
  (`CloudHypervisor`, `QemuMedia`), renamed to `provider` in v3
- `d2b_daemon_vm_degraded` labels `["vm", "reason"]`
- `d2b_daemon_pidfd_table_size` — adapts to `d2b_process_pidfd_active`

v3 replaces all `vm`-name labels with closed-set `provider` and `domain` labels:

| Metric | Type | Labels | Buckets (s) |
| --- | --- | --- | --- |
| `d2b_process_launch_total` | counter | `provider={minijail,systemd}`, `domain={system,user}`, `outcome={ok,error,quota}` | — |
| `d2b_process_launch_duration_seconds` | histogram | `provider`, `domain` | 0.001, 0.005, 0.010, 0.015, 0.020, 0.030, 0.050, 0.1, 0.5, 2.0 |
| `d2b_process_active` | gauge | `provider`, `domain` | — |
| `d2b_process_restart_total` | counter | `provider`, `class={exited,signaled,killed}` | — |
| `d2b_process_adoption_total` | counter | `provider`, `outcome={ok,quarantine,error}` | — |
| `d2b_process_pidfd_active` | gauge | (none) | — |

`d2b_process_launch_duration_seconds` measures from the instant the
`Process` resource commits to `Ready` to the instant the first OS spawn
call (clone3 or systemd unit start) is issued. The p95 hard target is
≤20 ms per ADR 0046.

Current `d2b_daemon_vm_shutdown_duration_seconds` maps to a new
`d2b_process_stop_duration_seconds` histogram with labels
`provider`, `stop_class={graceful,forced}`, `outcome`.

#### Provider (all Provider processes)

Target crates: individual Provider crates (ADR-only).

| Metric | Type | Labels | Buckets (s) |
| --- | --- | --- | --- |
| `d2b_provider_reconcile_total` | counter | `resource_type`, `outcome={ok,requeue,conflict,error}` | — |
| `d2b_provider_reconcile_duration_seconds` | histogram | `resource_type` | 0.001, 0.005, 0.01, 0.05, 0.1, 0.5, 2.0 |
| `d2b_provider_component_phase` | gauge | `component_type={controller,service,worker}`, `phase={pending,ready,degraded,failed,unknown}` | — |

#### Telemetry subsystem self-metrics

| Metric | Type | Labels | Buckets |
| --- | --- | --- | --- |
| `d2b_telemetry_drop_total` | counter | `signal={metric,trace,log}`, `reason={buffer_full,export_error}` | — |
| `d2b_telemetry_export_total` | counter | `signal`, `outcome={ok,error}` | — |
| `d2b_audit_write_total` | counter | `record_class`, `outcome={ok,rate_limited,error}` | — |
| `d2b_audit_drop_total` | counter | `record_class={privileged,unprivileged}` | — |

## Traces

### Trace context

`packages/d2b-realm-core/src/trace_context.rs` implements `TraceContext`
(implemented-and-reachable):

- opaque bounded printable-ASCII `trace_id` and `span_id` fields;
- maximum field length: 64 bytes (`MAX_TRACE_FIELD_LEN`);
- validated constructor `TraceContext::new` — returns `None` on invalid
  tokens;
- redacted in all Debug output;
- serialized by `d2b-realm-codec-protobuf/src/lib.rs` (`encode_trace_context`,
  `decode_trace_context`) over the current constellation protobuf codec.

v3 target: extract `TraceContext` unchanged to `d2b-telemetry`. Adapt the
protobuf codec to the v3 resource API contract framing. The current
`traceparent` field in `d2b-realm-core` (referenced from
`packages/d2bd/src/usbip_reconcile_state.rs` and `typed_error.rs`) is
preserved and extended to carry the OTEL W3C `traceparent` format.

`TraceContext` is carried in every d2b-bus route request and every
ComponentSession operation. The existing `AuditEnvelope.trace:
Option<TraceContext>` field (implemented-and-reachable in
`d2b-realm-core/src/audit.rs`) is preserved in v3 audit records.

### Span catalog

Every span carries standard `SpanKind`, the `d2b.zone` resource attribute,
an `outcome` attribute set at span end, and no path/name/argv/credential/
PID attribute. `operation_id` and `correlation_id` are allowed as span
attributes (they are opaque digests).

| Span name | Kind | Attributes | Notes |
| --- | --- | --- | --- |
| `d2b.store.write` | Internal | `kind`, `group_size`, `revision`, `outcome` | Per write transaction |
| `d2b.store.read` | Internal | `op`, `resource_type`, `outcome` | Per read transaction |
| `d2b.store.compaction` | Internal | `segments_removed`, `outcome` | |
| `d2b.api.request` | Server | `verb`, `resource_type`, `operation_id`, `outcome` | |
| `d2b.api.watch.event` | Internal | `resource_type`, `event_kind`, `outcome` | |
| `d2b.bus.route` | Client | `service`, `method`, `direction`, `outcome` | |
| `d2b.session.handshake` | Server | `profile`, `purpose_class`, `outcome` | |
| `d2b.session.reconnect` | Internal | `generation`, `outcome` | |
| `d2b.controller.reconcile` | Internal | `handler`, `resource_type`, `generation`, `outcome` | |
| `d2b.controller.hint` | Consumer | `handler`, `resource_type`, `hint_kind` | Commit → handler start |
| `d2b.process.launch` | Internal | `provider`, `domain`, `outcome` | Commit-to-Ready → launch attempt |
| `d2b.process.stop` | Internal | `provider`, `domain`, `stop_class`, `outcome` | |
| `d2b.provider.reconcile` | Internal | `resource_type`, `outcome` | Per Provider instance |
| `d2b.provider.install` | Internal | `provider_name`, `outcome` | |

### Trace context propagation

Resource API requests entering via d2b-bus carry an incoming W3C `traceparent`
header. The request creates a child span, stores the resulting `TraceContext`
in the operation record (adapts existing `AuditEnvelope.trace` field in
`d2b-realm-core/src/audit.rs`), and propagates it to:

- the store write transaction span;
- the post-commit controller hint;
- the controller reconcile span;
- any downstream Process launch span.

A complete request-to-launch trace spans:
`API request → store write → controller hint → controller reconcile → process launch`.

Cross-Zone operations propagate trace context through `ZoneLink` cursor
operations (adapts existing `d2b-realm-core/src/routing.rs` route propagation).

## Logs

### Structured OTEL log records

Each component emits structured OTEL log records for lifecycle transitions.
Records use `tracing` macros (already present in d2bd:
`packages/d2bd/src/lib.rs` and `packages/d2b-priv-broker/src/runtime.rs`).
The existing policy `startup_tracing_avoids_host_path_fields` in
`packages/d2b-contract-tests/tests/policy_observability.rs` is extended to
all v3 component startup paths.

Current startup tracing fields (implemented-and-reachable):

```rust
// packages/d2b-priv-broker/src/runtime.rs lines 689–717
config_source = "realm-controllers",  // → v3: "zone-config"
config_present = true,
```

v3 target: these fields are preserved with `config_source = "zone-config"` to
match the renamed artifact.

Forbidden log body content (extends current policies):

- Raw provider error strings
- Resource names, paths, PIDs, argv, or environment values
- Credential bytes or digests in non-audit context
- Terminal bytes or Wayland buffer content
- `RealmPath` or `WorkloadId` string values in log fields (old names not
  leaked into v3 log bodies)

Required structured log events per component are identical to those specified
in the prior version of this spec (Zone startup, configuration publication,
controller start/stop, Provider install/ready/failed, Process launch/stop,
Session handshake failures, audit segment rotation, telemetry buffer events).

### journald stdout/stderr ingestion

The current `scrapeJournal` option in `nixos-modules/components/observability/host.nix`
(implemented-and-reachable) collects journald entries for the host. The v3
`observability-otel` Provider adapts this to follow per-Zone cgroup entries:

- cgroup filter: `z-<zone-id>/*` (Zone runtime processes, from
  ADR-046-components-processes-and-sandbox naming)
- execution filter: `s-<execution-id>/*` (Process/EphemeralProcess cgroup
  leaves)
- disabled by default in the Provider spec; requires explicit operator consent

The collector applies a redaction filter before forwarding: drops `MESSAGE`
bodies matching credential/secret/path patterns, drops `_CMDLINE`, `_EXE`,
`INVOCATION_ID` fields. Retains `_SYSTEMD_CGROUP`, `PRIORITY`, `SYSLOG_IDENTIFIER`,
and structured `KEY=VALUE` pairs from the declared allow-set.

## Private OTEL endpoints

### Emitter architecture (resolved)

Zone runtime, core-controller, and all other core processes use a
**lightweight bounded emitter** from `d2b-telemetry`. The emitter:

- uses `tracing` + `tracing-subscriber` (already present in `d2bd`/broker)
  for structured log/span capture;
- serializes metric increments and span events into compact frames;
- writes frames over a private Unix datagram socket to the `observability-otel`
  Provider process;
- holds a bounded in-process ring (default 4 MiB metrics, 4 MiB traces, 2 MiB
  logs per process — configurable via the observability-otel Provider spec);
- drops oldest frames on ring-full, incrementing `d2b_telemetry_drop_total`.

No `opentelemetry_sdk` or `opentelemetry-otlp` dependency is added to any
Zone/core crate.

`Provider/observability-otel` runs the full OTEL SDK with an OTLP/gRPC exporter
as an **ordinary optional Process**. It:

- is not bootstrap;
- does not count toward the mandatory ≤64 MiB core aggregate (ADR-046-resource-store-redb unchanged);
- drains frames from the per-Zone datagram socket and forwards via OTLP to the SigNoz backend.

If the `observability-otel` Provider is absent, unready, or restarts: emitter
ring fills, frames are dropped with `d2b_telemetry_drop_total` increments, Zone
and core-controller health transitions to `Degraded` (not `Failed`). Zone/controller
startup and all authoritative audit are completely unaffected.

### Current architecture (implemented-and-reachable)

The current v3 baseline provides:

- `nixos-modules/components/observability/host.nix`: host-side OTel Collector
  writing to `${otelRuntimeDir}/host-egress.sock` (Unix UDS), collecting
  host metrics and tailing `storeSyncExportDir/*.jsonl` audit export.
- `nixos-modules/components/observability/stack.nix`: SigNoz obs VM with
  ClickHouse + ClickHouse Keeper + SigNoz + SigNoz OTel Collector.
- `nixos-modules/components/observability/guest.nix`: per-VM guest OTel
  Collector with `vm.name`/`vm.env`/`vm.role` resource attributes.
- `packages/d2b-host/src/otel_host_bridge_argv.rs`: argv generator for the
  `RunnerRole::OtelHostBridge` socat-based vsock OTLP forwarder
  (broker-spawned, `ProcessRole::OtelHostBridge` in
  `packages/d2b-core/src/processes.rs`).
- `packages/d2bd/src/otel_host_bridge_readiness.rs`: readiness gate for the
  `RunnerRole::OtelHostBridge` runner.

In v3 `ProcessRole::OtelHostBridge` / `RunnerRole::OtelHostBridge` maps to a
`Process` resource under the `observability-otel` Provider. The socat-based
vsock forwarding path (`otel_host_bridge_argv.rs`) is replaced by a native
OTLP/gRPC-over-vsock transport owned by the Provider.

### v3 per-Zone datagram receiver (ADR-only)

The `observability-otel` Provider owns the per-Zone telemetry socket.

On Provider installation, the observability-otel controller creates:

```
$ZONE_STATE/telemetry/emitter.sock
```

This Unix datagram socket receives compact telemetry frames from core process
emitters. Owner: the `observability-otel` collector process UID. Mode: `0660`.
Group: a generated `d2b-<zone>-otel-writers` group containing all component
UIDs for that Zone. ACL pattern follows `nixos-modules/components/observability/
host.nix`'s `otelRuntimeDir` ACL setup (`setfacl -m u:<uid>:--x` on the parent,
`d:u:<uid>:rw` default ACL).

A second socket `$ZONE_STATE/telemetry/otlp.sock` receives OTLP/gRPC from any
process that embeds the full SDK (e.g., Provider processes that opt in).



### Forwarding to obs Zone

Adapts the current vsock forwarding approach from `otel_host_bridge_argv.rs`
and the current `nixos-modules/components/observability/stack.nix` pipeline to
use native OTLP/gRPC over vsock. The SigNoz stack is unchanged; only the
forwarding transport changes. The current Nix ingress source model
(`ingressSources` in `stack.nix`) is preserved with a per-Zone entry replacing
the per-VM entry.

## Nix configuration and resource bundle

### Nix resource authoring shape

All Zone resources are authored through one uniform option:

```nix
d2b.zones.<zone>.resources.<name> = {
  type = "...";
  spec = { ...exact ResourceType spec fields... };
};
```

`metadata.name` is derived from the `<name>` attribute key; `metadata.zone`
from the `<zone>` attribute key; `apiVersion` defaults to `"resources.d2bus.org/v3"`. Nix
authors may also provide `metadata.ownerRef` and presentation-only
`metadata.labels`. They do **not** author `status`, `uid`, `revision`,
`finalizers`, `metadata.managedBy`, or `metadata.configurationGeneration` —
these fields are managed exclusively by the core runtime and publication
handler.

`spec` contains exactly the canonical ResourceType spec fields. No second Nix
vocabulary renames or re-nests them. The `resources` module is schema-aware:
for a known `type`, `spec.*` option types, defaults, and documentation are
generated from the committed `ResourceTypeSchema` JSON for that type.

**Zone self resource** (`<name>` = `<zone>`; `spec.telemetry` and `spec.audit`
are Zone ResourceTypeSchema fields owned by this spec):

```nix
d2b.zones.work.resources.work = {
  type = "Zone";
  # Optional: metadata.ownerRef and presentation metadata.labels may be set.
  # metadata.managedBy and metadata.configurationGeneration are NOT authored here.
  spec = {
    telemetry.emitter.ringCapacityBytes = 4194304;  # default 2097152
    audit = {
      retentionDays   = 30;        # range 1..3650
      maxSegmentBytes = 67108864;  # range 1 MiB..1 GiB
    };
  };
};
```

**Provider resource** — `spec` fields are defined by the Provider
ResourceTypeSchema in **ADR-046-provider-model-and-packaging**; `spec.config`
sub-fields are generated from the signed Provider schema for the installed
package (selected via `spec.artifactId`). This spec governs the
`observability-otel` Provider's placement as an optional non-bootstrap Process;
it does not redefine the Provider spec schema.
Secrets must be `Credential/<name>` refs; secret values are never inlined:

```nix
d2b.zones.work.resources.observability-otel = {
  type    = "Provider";
  # spec fields: see ADR-046-provider-model-and-packaging for exact schema.
  # config.* options generated from the Provider's signed resourceTypeSchema.
  spec = {
    # ... Provider spec fields per ADR-046-provider-model-and-packaging
    # artifactId selects the catalog entry / package.
    # config.signozBackend.credentialRef = "Credential/signoz-api-key";
  };
};
```

**Credential resource** — `spec` fields are defined by the Credential
ResourceTypeSchema in the owning primitive spec
(**ADR-046-primitive-resource-composition**); this spec does not redefine that
schema. Credential secret values are injected at runtime via the credential
store and never written to the bundle or Nix store:

```nix
d2b.zones.work.resources.signoz-api-key = {
  type = "Credential";
  # spec fields: see ADR-046-primitive-resource-composition for exact schema.
  spec = {
    # ... Credential spec fields per ADR-046-primitive-resource-composition
  };
};
```

### Canonical ResourceSpec JSON shapes

The `nix build` derivation for a Zone emits one sorted integrity-pinned bundle.
All keys are sorted canonically (alphabetically at every level) in the emitted
file.

The bundle contains **input** records: only the fields Nix authors and the
derivation produce. Core-set fields (`uid`, `revision`, `generation`,
`finalizers`, `managedBy`, `configurationGeneration`) are absent from the bundle
file; the configuration service populates them when persisting resources at
activation.

**Zone self resource — Nix-rendered bundle input** (telemetry/audit fields
owned by this spec):

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": {
    "name": "work",
    "zone": "work"
  },
  "spec": {
    "audit": {
      "maxSegmentBytes": 67108864,
      "retentionDays": 30
    },
    "telemetry": {
      "emitter": { "ringCapacityBytes": 4194304 }
    }
  },
  "type": "Zone"
}
```

**Zone self resource — persisted record after activation** (core-set fields
shown; `status` is a separate read-only sub-document):

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": {
    "configurationGeneration": 1,
    "finalizers": [],
    "generation": 1,
    "managedBy": "configuration",
    "name": "work",
    "ownerRef": null,
    "revision": 1,
    "uid": "01960000-0000-7000-8000-000000000000",
    "zone": "work"
  },
  "spec": {
    "audit": {
      "maxSegmentBytes": 67108864,
      "retentionDays": 30
    },
    "telemetry": {
      "emitter": { "ringCapacityBytes": 4194304 }
    }
  },
  "type": "Zone"
}
```

**Provider/observability-otel resource — Nix-rendered bundle input** — `spec`
fields defined in **ADR-046-provider-model-and-packaging**; secret values never
appear in the bundle:

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": {
    "name": "observability-otel",
    "zone": "work"
  },
  "spec": {
    "...": "see ADR-046-provider-model-and-packaging for exact Provider spec schema"
  },
  "type": "Provider"
}
```

**Credential/signoz-api-key resource — Nix-rendered bundle input** — `spec`
fields defined in **ADR-046-primitive-resource-composition**; secret values
injected at runtime via the credential store and never written to the bundle or
Nix store:

```json
{
  "apiVersion": "resources.d2bus.org/v3",
  "metadata": {
    "name": "signoz-api-key",
    "zone": "work"
  },
  "spec": {
    "...": "see ADR-046-primitive-resource-composition for exact Credential spec schema"
  },
  "type": "Credential"
}
```

For Provider and Credential resources the persisted record follows the same
pattern as the Zone example above: core sets `uid`, `revision`, `generation`,
`finalizers`, `managedBy = "configuration"`, and `configurationGeneration` at
activation.

### NixOS eval and build validation

**Eval-time (NixOS module assertions)**:

1. `type` must be a registered ResourceType known to the `resources` module;
   unknown `type` values fail at eval with an unknown-type message.
2. `spec.*` option types, defaults, and docs are generated from the committed
   `ResourceTypeSchema` for the declared `type`. For Provider resources,
   `spec.config.*` options are generated from the signed Provider schema
   for the package identified by `spec.artifactId`. Unknown `spec` fields are
   rejected at eval time by the generated option set.
3. `credentialRef` values must match `^Credential/[a-z][a-z0-9-]*$` and the
   referenced resource must be declared as a `type = "Credential"` entry in
   `d2b.zones.<zone>.resources`.
4. Numeric bounds, enum values, and required fields are enforced by the
   generated option types — the same constraints embedded in the
   `ResourceTypeSchema`. No hand-written bespoke assertions duplicate these
   constraints.
5. Resource `<name>` attribute keys are validated against `^[a-z][a-z0-9-]*$`.
6. Two `resources.<name>` entries with the same `type` and `<name>` within one
   Zone fail with a duplicate-name assertion.
7. Any bare host path, PID, UID number, argv string, or secret token in a
   `spec` leaf triggers an assertion failure. Secrets must be Credential refs.

**Build-time (`resources-bundle` derivation)**:

`nixos-modules/resources-bundle.nix` (ADR-only target):

1. Serializes each resource to canonical sorted JSON.
2. Validates each resource against its `ResourceTypeSchema`: core schemas for
   Zone/Credential/primitive types; the signed schema embedded in the Provider
   package for Provider resources.
3. Validates Provider `spec.config` against the exact signed Provider schema
   for the package identified by `spec.artifactId`. Schema mismatch or unknown
   config fields fail the build.
4. Asserts no resource `spec` field contains a bare secret, host path, argv
   token, or UID string (pattern-checked against the forbidden-field set —
   same set used by `policy_observability.rs::startup_tracing_avoids_host_path_fields`
   in current baseline `packages/d2b-contract-tests/tests/`).
5. Computes a SHA-256 digest for each resource spec (deterministic canonical
   bytes).
6. Sorts resources by `(type, name)` and computes the generation digest as
   SHA-256 of the newline-separated sorted digest list.
7. Emits one `zone-resources-<zone>.json` bundle in the Nix store; its store
   path is the integrity pin (content-addressed).

**Runtime activation (core-controller)**:

1. Reads the bundle from the Nix store path embedded in the system closure
   (same `git+file://` integrity guarantee used by existing `d2b_flake_ref` in
   `tests/lib.sh`).
2. Re-validates Provider package identity (per ADR-046-provider-model-and-packaging
   package identity contract) against the installed Provider package.
3. Resolves each `credentialRef` to a live credential store entry; fails closed
   if any ref is unresolved.
4. Checks name/domain/owner/bounds conflicts with existing resources in the Zone
   store.
5. Validates the Provider schema matches the installed package schema.
6. Any validation failure → reject the new generation; retain the prior
   generation; emit a `generation-rejected` audit record with a closed-enum
   `reason` field.

No validation step falls back to a partial activation.

## Authoritative audit

### Current implementation (implemented-and-reachable)

The v3 baseline has three separate audit implementations:

1. **Daemon JSONL audit** (`packages/d2bd/src/daemon_audit.rs`): hash-chain
   JSONL with `prev_hash`/`record_hash` (SHA-256), daily date-segmented files
   `daemon-events-YYYY-MM-DD.jsonl`, structured `DaemonEvent` variants
   covering VM start/stop/degraded, DetachedExec, Shell, OtelHostBridge.

2. **Broker audit** (`packages/d2b-priv-broker/src/audit.rs`): rate-limited
   O_APPEND JSONL; `AuditWriteClass::{Privileged,Unprivileged}`;
   `AuditDropSummary` counting `privileged_rate_limited` /
   `unprivileged_rate_limited`; `DEFAULT_AUDIT_WRITES_PER_SECOND = 4096`;
   `OpAuditRecord` variants per broker op
   (`packages/d2b-priv-broker/src/ops/audit_op.rs`).

3. **Gateway/realm-core audit** (`packages/d2b-gateway/src/audit.rs`,
   `packages/d2b-gateway-runtime/src/audit_jsonl.rs`): `GatewayAuditKind`
   events + `AuditEnvelope` (from `d2b-realm-core/src/audit.rs`) with fields
   `realm: RealmPath`, `node: NodeId`, `workload: Option<WorkloadId>`,
   `principal: PrincipalId`, `scope: AuthorizationScope`, `decision:
   AuthzDecision`, `trace: Option<TraceContext>`.

4. **Realm-core chain types** (`packages/d2b-realm-core/src/audit.rs`):
   `AuditHash`, `AuditChainLink`, `AuditChainRecord` (with `realm: RealmPath`,
   `node: NodeId` fields), `AuditStreamKind::{Gateway, RemoteNode, Daemon}`,
   `AuditSinkHealth`, `AuditRetentionFloorStatus`.

### v3 target audit record types

The `d2b-audit` crate (ADR-only new crate) provides unified hash-chain JSONL
adapted from the implementations above. Every record carries:

```json
{
  "ts_ms":          1234567890123,
  "schema_version": 1,
  "zone":           "<zone_name>",
  "record_class":   "<class>",
  "operation_id":   "<opaque>",
  "correlation_id": "<opaque>",
  "trace_id":       "<opaque or null>",
  "source":         "<component>",
  "prev_hash":      "sha256:<hex>",
  "record_hash":    "sha256:<hex>",
  "<class>_fields": { ... }
}
```

`zone` is the Zone name (baseline: `RealmPath` → target: Zone name).
`operation_id` and `correlation_id` are the same opaque-digest types from
`d2b-realm-core/src/ids.rs` (`OperationId`, `CorrelationId`), retained
unchanged. `trace_id` is the `TraceContext.trace_id` field from
`d2b-realm-core/src/trace_context.rs`, adapted to the v3 contract.

#### ResourceMutation

```json
{
  "record_class": "resource-mutation",
  "resource_mutation_fields": {
    "verb":              "create|update|patch|status|delete|finalize",
    "resource_type":     "<closed catalog type>",
    "resource_uid":      "<opaque uid>",
    "generation":        12,
    "expected_revision": 7,
    "resulting_revision":8,
    "subject_digest":    "sha256:<hex>",
    "policy_revision":   3,
    "outcome":           "ok|conflict|denied|invalid|error",
    "error_code":        "<stable code or null>"
  }
}
```

`subject_digest` is SHA-256 of the normalized canonical subject string from
the v3 `AuthenticatedSubjectContext` (ADR-046-componentsession-and-bus); this
replaces the current `PrincipalId` in `AuditEnvelope`. No resource name, spec
bytes, or status bytes appear in this record.

#### ResourceUpgrade (D091) and expedited reconcile (D090)

An `assess_update`/`plan_upgrade`/`execute_upgrade` operation emits a
`resource-upgrade` record; an authorized expedited (`waitForReconcile`) mutation
is recorded via the existing `resource-mutation` record extended with an
`expedited: true` flag and its `operation_id`. Neither carries spec/status bytes,
secrets, or raw artifact paths — only bounded closed-enum currency/disruption
values and opaque generation/digest IDs:

```json
{
  "record_class": "resource-upgrade",
  "resource_upgrade_fields": {
    "verb":                 "assess|plan|execute",
    "resource_type":        "<closed catalog type>",
    "resource_uid":         "<opaque uid>",
    "update_state":         "Current|UpdateAvailable|UpgradeRequired|Upgrading|Blocked|Unknown",
    "disruption":           "None|Reload|Restart|Recycle|Replace",
    "preserve_state":       true,
    "reasons":              ["<closed-enum currency reason>"],
    "observed_generation":  11,
    "target_generation":    12,
    "affected_owned_count": 3,
    "operation_id":         "<opaque>",
    "outcome":              "ok|blocked|conflict|denied|error",
    "error_code":           "<stable code or null>"
  }
}
```

The core Operation ledger owns upgrade idempotency/progress; this record is the
authoritative security history of the upgrade decision/execution, not a second
ledger. OTEL metrics for currency/upgrade and expedited reconcile use only
bounded closed-enum labels (`update_state`, `disruption`, `disposition`,
`outcome`) plus the existing `zone`/`provider`/`component` resource attributes —
never resource names, generation digests as labels, or per-operation IDs as
labels (cardinality rules below).

#### RBACChange

```json
{
  "record_class": "rbac-change",
  "rbac_change_fields": {
    "verb":           "create|update|delete",
    "resource_type":  "Role|RoleBinding",
    "resource_uid":   "<opaque uid>",
    "generation":     4,
    "subject_digest": "sha256:<hex>",
    "policy_revision":3,
    "outcome":        "ok|denied|error"
  }
}
```

#### SessionConnect

```json
{
  "record_class": "session-connect",
  "session_connect_fields": {
    "event":              "connect|reconnect|close",
    "profile":            "NN|KK|IKpsk2",
    "purpose_class":      "local|enrolled|bootstrap",
    "transport_class":    "unix|vsock|zone_link",
    "subject_digest":     "sha256:<hex>",
    "authz_decision":     "allowed|denied",
    "authz_revision":     7,
    "session_gen_digest": "sha256:<hex>",
    "outcome":            "ok|auth|policy|timeout|error",
    "error_code":         "<stable code or null>"
  }
}
```

Adapts current `GatewayAuditEvent` (from `d2b-gateway/src/audit.rs`) and
`AuditEnvelope.scope = AuthorizationScope::capability(Capability::WindowForwarding)`
pattern to cover all ComponentSession connections. `transport_class=zone_link`
covers what the current `AuditStreamKind::Gateway` and `AuditStreamKind::RemoteNode`
streams recorded.

#### RouteAdmission

```json
{
  "record_class": "route-admission",
  "route_admission_fields": {
    "service":        "<closed catalog service name>",
    "method":         "<method name>",
    "direction":      "local|host|guest|zone_link",
    "subject_digest": "sha256:<hex>",
    "authz_decision": "allowed|denied",
    "authz_revision": 7,
    "outcome":        "ok|denied|error"
  }
}
```

#### BrokerEffect

```json
{
  "record_class": "broker-effect",
  "broker_effect_fields": {
    "op_class":                  "<stable broker op class>",
    "subject_digest":            "sha256:<hex>",
    "execution_context_digest":  "sha256:<hex>",
    "resource_context_digest":   "sha256:<hex>",
    "outcome":                   "ok|denied|error",
    "error_code":                "<stable code or null>"
  }
}
```

Adapts existing `OpAuditRecord` from
`packages/d2b-priv-broker/src/ops/audit_op.rs`. No raw paths, device
identifiers, or broker operation arguments. Current `SwtpmDirAudit` fields
(`base_dir_hash`, `result`, `mode`, `owner_uid`, `marker_result`) are
preserved by encoding them into `resource_context_digest` plus a
`swtpm_dir_fields` sub-object that carries the closed-set enums without paths.

#### ProcessEffect

```json
{
  "record_class": "process-effect",
  "process_effect_fields": {
    "event":                 "launch|stop|adopt|quarantine",
    "provider":              "minijail|systemd|system-core-user",
    "domain":                "system|user",
    "no_isolation":          false,
    "execution_ref_digest":  "sha256:<hex>",
    "process_uid":           "<opaque uid>",
    "outcome":               "ok|error",
    "exit_class":            "exited|signaled|killed|null"
  }
}
```

`no_isolation: true` is set when `domain=user` and the parent resource is a
user-only `Host` (i.e., `isolationPosture: "none"`). It is `false` for all
other process effects. The field is always present.
`provider=system-core-user` is the closed-set label for user-only Host child
processes.

Adapts current `DaemonEvent::{VmStartRunnerExit, RunnerExitKind, VmShutdown}` variants
from `packages/d2bd/src/daemon_audit.rs`. Current `RunnerExitKind::{Exited,
Signaled, Killed}` maps directly to `exit_class`. No PID, pidfd, argv,
environment, or unit name; current `VmShutdownProvider::{CloudHypervisor,
QemuMedia, Unknown}` maps to the closed `provider` enum.

**Current gap** (unsafe-local): the existing `HelperRegistry::dispatch_launch` path
in `packages/d2bd/src/unsafe_local_helper.rs` does not emit a `DaemonEvent` for
unsafe-local launches or stops. In v3 every user-only Host process launch and
stop must emit a `ProcessEffect` record with `no_isolation: true`, `domain=user`,
`provider=system-core-user`.

#### StateReset

```json
{
  "record_class": "state-reset",
  "state_reset_fields": {
    "scope":        "zone|provider|host|guest",
    "trigger":      "operator|upgrade|corruption|emergency",
    "generation":   5,
    "prior_digest": "sha256:<hex>",
    "outcome":      "ok|error"
  }
}
```

### AuditStreamKind v3 mapping

| Current (`AuditStreamKind`) | v3 target | Notes |
| --- | --- | --- |
| `Daemon` | `Zone` (Zone-local) | Zone runtime owns this stream |
| `Gateway` | `ZoneLink` | Gateway-backed realms become ZoneLink-connected Zones |
| `RemoteNode` | `RemoteZone` | Cross-Zone audit stream over remote Zone link |

The `AuditChainRecord` type (current: `realm: RealmPath`, `node: NodeId`) is
re-versioned in v3 with `zone: String` replacing `realm` and `node` dropped.

### Durability classes

| Class | Records | Durability | Failure policy |
| --- | --- | --- | --- |
| Privileged | ResourceMutation (RBAC verbs), RBACChange, SessionConnect (auth-failure/denial), StateReset | Durable before operation completes | Fail operation with `audit-unavailable` |
| Standard | ResourceMutation (non-RBAC), RouteAdmission, ProcessEffect | Durable within bounded window | Log warning; metric increment; continue |
| Best-effort | BrokerEffect (informational), telemetry-self records | Async append | Drop under rate-limit; no operation impact |

Privileged records are never rate-limited. Unprivileged records follow the
existing `DEFAULT_AUDIT_WRITES_PER_SECOND = 4096` rate limit from
`packages/d2b-priv-broker/src/audit.rs` (`AuditWriteClass::Unprivileged` path).

### Segmentation and retention

Segments rotate at 64 MiB or UTC midnight (whichever first). Segment names:
`audit-<YYYYMMDDHHMMSSNNNNNN>.jsonl` (adapts the 20-digit format from ADR
0045 v3 audit journal). Files are immutable after rotation. Default retention:
30 days (adapts `DEFAULT_GATEWAY_AUDIT_RETENTION_DAYS = 14` from
`packages/d2b-gateway-runtime/src/audit_jsonl.rs`; extended for the more
authoritative Zone audit).

### Export

`d2b zone audit export [--zone <name>] [--after <segment>] [--before <segment>]`

- Requires `audit-export` verb on the Zone resource (admin-only).
- Adapts `ExportBrokerAuditOk` response contract from
  `packages/d2b-priv-broker/tests/broker_export_audit.rs`.
- Hash chain breaks reported inline in output stream.
- No plaintext resource names, paths, argv, or credential bytes.

## Doctor and support bundles

### `d2b zone doctor [--zone <name>] [--json]`

Read-only aggregate health report. Adapts `doctor::render_summary` from
`packages/d2bd/src/lib.rs` (referenced in
`packages/d2b/tests/host_doctor_contract.rs`). Extends `d2b host doctor
--read-only` behavior to the Zone resource API.

Current doctor probes (implemented-and-reachable): `broker_ready`, per-check
`status`/`data`, `summary`, `exitCode`, plus probes redirected via
`D2B_BROKER_SOCKET`, `D2B_PUBLIC_SOCKET`, `D2B_DAEMON_STATE_DIR`,
`D2B_METRICS_URL`, `D2B_MANIFEST_PATH`. The `audit_check.rs` pattern
(`packages/d2bd/src/audit_check.rs`) provides the `defects` array for audit
chain validation.

v3 JSON envelope:

```json
{
  "zone":           "<name>",
  "zone_phase":     "Pending|Ready|Degraded|Failed|Unknown",
  "store_health":   { "phase": "…", "revision": 12345, "compaction_floor": 100, "watch_active": 3 },
  "controllers":    [{ "handler": "…", "phase": "…", "queue_depth": 0, "last_reconciled_at": "…" }],
  "providers":      [{ "provider": "…", "phase": "…", "component_phases": {} }],
  "process_counts": { "active": 4, "failed": 0 },
  "audit":          { "phase": "ok|rate_limited|unavailable", "segments": 3, "drop_privileged": 0, "drop_total": 2 },
  "telemetry":      { "phase": "ok|buffering|unavailable", "drop_total": 0 },
  "checks":         [{ "name": "…", "status": "ok|warn|error", "detail": "…" }],
  "summary":        { "ok": 12, "warn": 1, "error": 0 }
}
```

Named check set: `store-revision-monotonic`, `controller-all-ready`,
`mandatory-providers-ready`, `audit-sink-healthy`, `otel-sink-reachable`,
`schema-catalog-consistent`, `watch-quota-headroom`, `audit-hash-chain-clean`,
`isolation-posture-declared`.

Forbidden: resource names, paths, argv, credential bytes, PID values, audit
record content, raw error messages beyond stable codes.

### `d2b zone support-bundle [--zone <name>]`

Bounded redacted diagnostic snapshot for operator/support use. Requires
`support-bundle` verb (admin-only). Output is NDJSON.

Contents:

1. Doctor output.
2. Bounded resource status snapshots: last 32 per ResourceType, 512 total;
   only `metadata` (UID, zone, generation, revision, timestamps, phase) and
   top-level `status` fields (phase, conditions, observedGeneration, outcome
   code). No `spec` content; no resource names beyond opaque UID.
3. Controller checkpoint / queue-depth snapshots.
4. Schema catalog inventory: ResourceType names + versions only.
5. Audit segment inventory: filenames (date-derived), sizes, record counts,
   drop totals per class.
6. OTEL collector metrics summary (if observability-otel Provider is available).
7. Bounded structured log ring: last 2000 structured log records already
   emitted and redacted by the Zone runtime and core-controller.

## Error, failure, and outage behavior

### OTEL collector unavailable

When the per-Zone emitter socket (`emitter.sock`) is absent or the
`observability-otel` Provider process is not running:

1. Emitter ring fills; oldest frames dropped in FIFO order.
2. `d2b_telemetry_drop_total` increments per dropped frame.
3. All mutations, reconciliation, and process launch continue normally.
4. Doctor: `telemetry: { "phase": "buffering" }`.
5. Zone/core-controller health: `Degraded` (not `Failed`). Zone startup is not
   blocked.
6. `observability-otel` Provider: `Degraded` phase with `telemetry-export-unavailable`.
7. On socket recovery: emitter resumes draining ring in FIFO order.

### Audit sink unavailable

When the audit segment file cannot be opened or written:

1. Privileged records: fail the operation with stable error code `audit-unavailable`.
2. Standard records: log warning via `tracing::warn!`, increment
   `d2b_audit_drop_total{record_class="standard"}`, continue.
3. Best-effort records: increment counter, drop.
4. Doctor: `audit: { "phase": "unavailable" }`.
5. Rate-limited records: privileged records are never rate-limited (adapts
   existing `AuditWriteClass::Privileged` invariant from
   `packages/d2b-priv-broker/src/audit.rs`).

On audit file descriptor loss (rotation/restart): Zone runtime re-opens the
segment file with `O_APPEND | O_CREAT` before the next write, mirroring the
existing `packages/d2bd/src/daemon_audit.rs` file-open pattern.

### Zone store quarantined

During store quarantine (per ADR-046-resource-store-redb):

1. Emitter ring fills; frames dropped (observability-otel Provider stopped with store).
2. Committed audit segments are preserved.
3. Doctor available via read-only quarantine snapshot.
4. Support bundle available from quarantine snapshot.
5. Doctor: `zone_phase: "Failed"`, condition `store-quarantined`.

### Controller handler degraded/failed

A single handler degrading does not prevent other handlers, OTEL export,
audit writes, or doctor/support-bundle reads. The affected handler retries
with bounded backoff; a `StateReset` audit record is emitted if owned
resources are cleared during recovery.

### Process Provider failed

Marks affected `Process` resources `Degraded`/`Failed`. Emits
`ProcessEffect{event:"quarantine"}`. OTEL metrics reflect phase change via
`d2b_provider_component_phase`. Other Zones/Providers continue.

### Support bundle during Zone failure

A bundle request during `zone_phase: "Failed"` returns available data from
the quarantine snapshot, adds `bundle_completeness: "partial"` at the top
level, and exits with code 1.

## Configuration-owned resource cleanup contract

### Configuration management metadata

The core distinguishes configuration-authored resources from controller-created
and API-created resources using `metadata.managedBy` and
`metadata.configurationGeneration`. These fields are set by the core at
activation or create time; Nix authors and the bundle derivation never produce
them.

- `metadata.managedBy = "configuration"` — resource was last written by the
  Nix configuration publication service; it owns the spec and is the only
  value on which the cleanup contract acts.
- `metadata.configurationGeneration = <N>` — the monotonically increasing
  generation count at which this resource was last written by configuration
  publication.
- `metadata.managedBy = "controller"` — resource was created by a controller
  (e.g., a `Process` child created by the `observability-otel` Provider
  lifecycle controller).
- `metadata.managedBy = "api"` — resource was created or last written through
  the Zone API (e.g., by an operator tool or integration).

The configuration publication service issues Delete only on resources where
`managedBy = "configuration"`. Resources with `managedBy = "controller"` or
`managedBy = "api"` are never touched by configuration cleanup, regardless of
whether they are absent from the new bundle.

**Never use presentation labels as cleanup authority.** Labels may be set by
Nix authors for presentation/organization purposes but must not be used to
determine which resources to delete or retain during generation activation.

### New generation activation algorithm

The configuration publication handler (per ADR-046-core-controllers
§Configuration publication) follows this algorithm on new bundle arrival:

1. **Validate** the new bundle (all eval, build, and runtime checks from the
   previous section). Reject with a `generation-rejected` audit record on any
   failure; retain the prior active generation.
2. **Stage** new and updated resources in inactive store transactions. Resources
   are not yet visible to controllers.
3. **Atomic swap**: install the new bundle generation digest and switch resource
   watchers to the staged versions in one store transaction. Returns immediately;
   does not block on cleanup of removed resources.
4. **Trigger** affected Providers/controllers/resources via the reconciliation
   hint system (ADR-046-resource-reconciliation).
5. **Schedule Delete** for each resource with `managedBy=configuration` present
   in the prior generation but absent from the new bundle. Each scheduled Delete
   sets `metadata.deletionRequestedAt` on the resource and adds a
   `deletion-pending` condition (`status: "True"`, `reason: "ConfigRemoved"`).
   No phase change is applied to the resource being deleted.
6. **Set condition** `pending-cleanup` (`status: "True"`) on the Zone resource
   status while any such Delete has not completed. Zone phase moves to
   `Degraded` (not `Failed`).
7. **Retain prior generations** count-based: default 3 prior generations
   retained (range 1..16). Pruning of a prior generation is permitted only after
   all cleanup from that generation is complete and the count limit is exceeded.
   No time-based TTL or rollback window.

### Cleanup ordering and safety

**Deletion ordering**: core drives deletions in reverse-dependency (child-first)
order via the owner-child index. When a resource's `metadata.deletionRequestedAt`
is set, each configuration-managed resource that has owned children receives a
`deletion-requested` hint to its controller. The controller works through its
children and clears its own finalizer when done. After all finalizers clear,
core performs one atomic store transaction that writes the `Deleted`
revision/change event and removes the resource row and all index entries. The
audit subsystem then appends the deletion audit record from that committed
revision with dedup/exactly-once recovery. No controller needs to implement its
own deletion ordering.

**The cleanup contract never**:
- Deletes resources with `managedBy = "controller"` or `managedBy = "api"`
  solely because they are absent from the new bundle. Owner controllers
  reconcile and delete their own children; API-managed resources are owned by
  their creator.
- Deletes audit segment files, OTEL emitter ring state, or any non-resource
  filesystem artifact. Audit data is governed exclusively by `retentionDays`,
  `maxSegmentBytes`, and durability-class rules.
- Touches resources in other Zones.
- Applies broad `chmod`, `chown`, `setfacl`, or path sweeps — consistent with
  the ADR 0034 no-broad-sweep invariant.

**observability-otel Provider cleanup sequence**:

When `Provider/observability-otel` is removed from Nix config:

1. Config-publication sets `metadata.deletionRequestedAt` on
   `Provider/observability-otel` and adds a `deletion-pending` condition
   (`status: "True"`, `reason: "ConfigRemoved"`). Zone phase moves to
   `Degraded`; Zone condition `pending-cleanup` is added.
2. The Provider lifecycle controller receives the `deletion-requested` hint;
   adds `deleting-children` condition (`status: "True"`) to its own status.
3. Controller deletes owned `Process` (collector) and `Volume` (emitter socket
   directory) children in dependency order.
4. After all children are deleted, the controller clears its own finalizer.
5. Core performs final deletion as one atomic transaction: emits
   `ResourceMutation{event="deleted", trigger="config-cleanup"}` audit record
   and removes the `Provider/observability-otel` row and all index entries.
6. Zone status: condition `telemetry-export-unavailable` is set; emitter ring
   fills; `d2b_telemetry_drop_total` increments; Zone phase remains `Degraded`.
7. `pending-cleanup` condition is cleared when step 5 completes.

The emitter socket is never removed while core emitter processes are still
writing to it: the `Volume` child is deleted only after the collector `Process`
has stopped and its finalizer clears.

### Cleanup status, audit, and error surface

**Zone resource status during cleanup** (Zone itself carries `pending-cleanup`):

```json
{
  "status": {
    "observedGeneration": 3,
    "phase": "Degraded",
    "conditions": [
      {
        "lastTransitionTime": "...",
        "message": "1 configuration-managed resource pending deletion: Provider/observability-otel",
        "reason": "ConfigRemovedResources",
        "status": "True",
        "type": "pending-cleanup"
      },
      {
        "lastTransitionTime": "...",
        "message": "telemetry export unavailable; emitter ring filling",
        "reason": "ObservabilityProviderDeleted",
        "status": "True",
        "type": "telemetry-export-unavailable"
      }
    ]
  }
}
```

**Resource being deleted** (`Provider/observability-otel` persisted record during
deletion; no phase change — `deletion-pending` and `deleting-children` are
typed conditions):

```json
{
  "metadata": {
    "deletionRequestedAt": "2026-07-22T21:00:00Z",
    "managedBy": "configuration",
    "name": "observability-otel",
    "zone": "work"
  },
  "status": {
    "conditions": [
      {
        "lastTransitionTime": "...",
        "reason": "ConfigRemoved",
        "status": "True",
        "type": "deletion-pending"
      },
      {
        "lastTransitionTime": "...",
        "reason": "AwaitingChildDeletion",
        "status": "True",
        "type": "deleting-children"
      }
    ]
  },
  "type": "Provider"
}
```

**Cleanup stall detection**: if a scheduled Delete has not completed within
30 minutes, condition `cleanup-stalled` is added with the resource name and
stall duration. Zone phase remains `Degraded`. A `HealthCondition` audit record
(best-effort durability class) is emitted with a closed-enum stall reason
(`finalizer-holder-gone`, `controller-unresponsive`, `child-deletion-failed`).

**Audit records for cleanup events**:

| Event | Record class | Durability |
| --- | --- | --- |
| New generation accepted and staged | `ResourceMutation{event="generation-staged"}` | standard |
| Generation atomically activated | `ResourceMutation{event="generation-activated", bundle_digest="sha256:..."}` | standard |
| Generation rejected (validation failure) | `ResourceMutation{event="generation-rejected", reason="<enum>"}` | standard |
| Config-removed resource Delete scheduled | `ResourceMutation{event="delete-scheduled", trigger="config-cleanup"}` | standard |
| Config-removed resource Delete completed | `ResourceMutation{event="deleted", trigger="config-cleanup"}` | standard |
| Cleanup stall detected | `HealthCondition{type="cleanup-stalled", reason="<enum>"}` | best-effort |

No resource name, path, argv, or secret value appears in any cleanup audit
field. The `reason` on `generation-rejected` uses only closed-enum values:
`schema-validation-failed`, `package-identity-mismatch`, `credential-unresolved`,
`conflict-detected`, `bounds-violation`.

### Prior generation retention

Prior generations are retained count-based (default 3, range 1..16). There is
no time-based TTL or rollback window. Activating an older generation counts as
a new activation: resources absent from the activated bundle but present in the
replaced generation receive Delete; resources present in both are reconciled.

## Tests

### Metric inventory policy tests

Adapt `packages/d2b-contract-tests/tests/policy_metrics.rs`:

- Assert every metric declared in this spec is present in the closed
  `METRIC_INVENTORY` table in its owning crate.
- Assert no metric label value from the forbidden list (`vm`, name-shaped
  strings) appears in any `MetricDescriptor` label set.
- Assert `d2b_controller_hint_to_handler_seconds` bucket list includes a
  5 ms bucket (0.005).
- Assert `d2b_process_launch_duration_seconds` bucket list includes a 20 ms
  bucket (0.020).
- Assert the old `d2b_daemon_vm_state` metric's `vm` label is not present
  in any v3 metric descriptor.

### Cardinality / redaction static lint

New test `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs`:

- Port and extend `startup_tracing_avoids_host_path_fields` from
  `packages/d2b-contract-tests/tests/policy_observability.rs` to all v3
  component startup paths (Zone runtime, core-controller, all Provider
  processes).
- Scan all `tracing::` and `span!`/`event!` call sites for forbidden field
  names: `path`, `socket`, `argv`, `env`, `pid`, `exe`, `realm`, `workload_id`
  (old baseline names must not leak).
- Assert `TraceContext` fields only accessed via the validated constructor
  `TraceContext::new`.
- Assert no `format!("{}", ...)` of a `PathBuf` appears as a span attribute.
- Assert the OTEL resource attribute initialization uses only the allowlist
  from this spec (extends `loki_native_otel_resource_attributes` test from
  `packages/d2b-contract-tests/tests/policy_observability.rs`).
- Assert `config_source = "realm-controllers"` string literal does not appear
  in any v3 component source; only `config_source = "zone-config"` is allowed.

### Audit record shape tests

New `packages/d2b-audit/tests/`:

- `audit_record_hash_chain`: adapted from existing hash-chain test pattern
  in `packages/d2b/tests/audit_contract.rs`; verifies `prev_hash` /
  `record_hash` chain and truncation detection.
- `audit_record_schema`: deserialize every record class from fixture JSON;
  verify `realm`, `node`, `workload_id` field names are absent (old baseline
  names must not appear in v3 schema); verify `zone` is present.
- `audit_segment_rotation`: adapted from rotation logic tests in
  `packages/d2b-gateway-runtime/src/audit_jsonl.rs`.
- `audit_rate_limit_privileged_never_dropped`: adapted from
  `AuditWriteClass::Privileged` invariant in
  `packages/d2b-priv-broker/src/audit.rs`.
- `audit_unavailable_blocks_privileged`: verifies `audit-unavailable` error.

### Doctor contract tests

New `packages/d2b/tests/zone_doctor_contract.rs`:

- Adapts test scaffold from `packages/d2b/tests/host_doctor_contract.rs`
  (env-redirect sandbox: `D2B_PUBLIC_SOCKET`, `D2B_DAEMON_STATE_DIR`, etc.).
- Asserts JSON envelope fields from this spec; asserts `zone_phase` field.
- Asserts old fields `broker_ready` (from current doctor) are renamed or absent.
- Asserts no resource names, paths, argv, PIDs in any check detail.
- Exit 0 on all-ready fixture; non-zero on degraded; exit code 1 on quarantine.
- Doctor works when OTEL socket absent (`telemetry.phase = "unavailable"`).
- Doctor works when Zone store is in quarantine mode.
- `isolation-posture-declared` check passes when a user-only `Host` resource
  status carries `isolationPosture: "none"`; fails when the field is absent.
- Zone with no user-only `Host` resource: `isolation-posture-declared` check
  is omitted from the check list (not reported as an error).

### Host posture tests

New `packages/d2b-provider-system-core/tests/host_posture_contract.rs`:

- `host_status_carries_no_isolation`: reconcile a user-only `Host` resource
  fixture (`defaultDomain=user`, `allowedDomains=[user]`); assert
  `status.isolationPosture = "none"` is set by the reconciler; assert no
  operator-supplied `isolationPosture` value is accepted; assert a Host
  resource with a different execution policy does not have `isolationPosture`
  set.
- `host_process_effect_carries_no_isolation_flag`: drive a user-only Host
  process launch through the audit sink fixture; assert the emitted
  `ProcessEffect` record has `no_isolation: true`, `domain=user`,
  `provider=system-core-user`; assert `no_isolation` is absent (or `false`)
  in ProcessEffect records for non-user-only-Host processes.
- `host_cli_renders_isolation_warning`: drive `d2b zone inspect` output for a
  Zone containing a user-only `Host` resource; assert the warning annotation
  (`⚠ no isolation boundary`) is present in the output; assert the warning is
  not suppressed by any flag; assert a Zone with only non-user-only Host
  resources does not emit this annotation.
- `host_otel_no_no_isolation_label`: assert that no span attribute, log field, or
  metric label carries the key `no_isolation`; `no_isolation` is audit-and-status only.
- `unsafe_local_launch_emits_audit_record`: drive the unsafe-local launch path
  (adapting `packages/d2bd/src/unsafe_local_helper.rs` `dispatch_launch` call site)
  and assert a `ProcessEffect{no_isolation:true}` record is emitted; this test
  documents the current gap in baseline `DaemonEvent` coverage.

### Support bundle tests

New `packages/d2b/tests/zone_support_bundle_contract.rs`:

- All content sections from this spec are present.
- Resource status snapshots contain no `spec` bytes and no `metadata.name`.
- `bundle_completeness: "complete"` on healthy Zone; `"partial"` on quarantined.

### Performance histogram tests

In the performance benchmark fixture (per ADR-046-resource-store-redb):

- `hint_to_handler_latency`: drive 1000 committed resources; assert p95
  `d2b_controller_hint_to_handler_seconds` ≤5 ms.
- `commit_to_launch_latency`: drive 100 concurrent ready Process resources;
  assert p95 `d2b_process_launch_duration_seconds` ≤20 ms.

### OTEL endpoint tests

New `packages/d2b-provider-observability-otel/tests/`:

- `emitter_socket_receive`: lightweight emitter writes frames to the per-Zone
  datagram socket; Provider drains and forwards; assert spans arrive at the
  mock OTLP sink with correct `d2b.zone` resource attribute and no forbidden
  labels.
- `emitter_ring_drains_on_socket_available`: emitter writes frames before socket
  exists; socket appears; assert buffered frames arrive in FIFO order.
- `emitter_ring_drop_on_overflow`: fill ring past capacity; assert
  `d2b_telemetry_drop_total` increments; assert Zone/controller health is
  `Degraded` (not `Failed`); assert Zone startup is not blocked.
- `no_vm_label_in_metrics`: assert no emitted frame carries a label named
  `vm` with a resource-name value; assert old `d2b_daemon_vm_state` metric
  shape is absent from all v3 collectors.
- `zone_startup_proceeds_without_provider`: drive Zone bootstrap with
  observability-otel Provider absent; assert Zone reaches `Ready`; assert
  `d2b_telemetry_drop_total` > 0; assert no audit records are affected.

### Export audit tests

New `packages/d2b-audit/tests/export_audit.rs` (adapts
`packages/d2b-priv-broker/tests/broker_export_audit.rs` pattern):

- Admin-only `audit-export` verb is enforced.
- NDJSON output only.
- Hash chain breaks reported inline.
- No `realm`, `node`, `workload_id` field names in exported records.
- No resource name, path, or argv appears in exported records.

### Nix configuration and resource bundle tests

New `tests/unit/nix/cases/resources-bundle-telemetry.nix` (nix-unit eval
cases; auto-discovered; adapts the existing eval-case pattern from
`tests/unit/nix/eval-cases/`):

- `eval_rejects_unknown_type`: set `d2b.zones.work.resources.foo.type = "Unknown"`;
  assert NixOS eval fails with an unknown-ResourceType message.
- `eval_rejects_invalid_emitter_ring_size`: set
  `d2b.zones.work.resources.work.spec.telemetry.emitter.ringCapacityBytes = 0`;
  assert NixOS eval fails with a schema-range violation.
- `eval_rejects_unknown_provider_config`: set an unknown key in
  `d2b.zones.work.resources.observability-otel.spec.config`; assert eval
  fails with an unknown-field message (schema-generated option set rejects it).
- `eval_rejects_inline_secret_in_config`: set a `spec.config` leaf to a raw
  secret string matching the forbidden-field pattern; assert eval fails.
- `eval_rejects_unresolved_credential_ref`: set `spec.config.signozBackend.credentialRef =
  "Credential/nonexistent"` without a matching `resources.nonexistent` Credential
  entry; assert eval fails with a missing-credential assertion.
- `eval_rejects_duplicate_resource_name`: declare two `resources` entries of
  type `Provider` with the same `<name>` attr key; assert eval fails.

New `packages/d2b-provider-observability-otel/tests/bundle_contract.rs`:

- `bundle_is_sorted_canonically`: render a two-resource bundle; assert JSON
  keys at every level are in ascending alphabetical order; assert resources
  are sorted by `(type, name)`.
- `bundle_digest_is_deterministic`: render the same config twice from
  independent evaluation; assert the generation digest embedded in the
  `metadata.configurationGeneration` token round-trips identically.
- `bundle_contains_no_secret_values`: set a `credentialRef`; assert the
  rendered bundle JSON contains no key named `secretValue`, `password`,
  `token`, `key`, or any value matching a secret pattern heuristic.
- `bundle_schema_validates_against_provider_schema`: assert the rendered
  `Provider/observability-otel` spec fields validate against the JSON Schema
  produced by the declared Provider package's `resourceTypeSchema` output.

### Configuration-owned cleanup contract tests

New `packages/d2b-core-controller/tests/config_cleanup.rs`:

- `managedby_configuration_set_on_activated_resources`: activate a bundle;
  assert every resource in the Zone store carries
  `metadata.managedBy = "configuration"` and a non-zero
  `metadata.configurationGeneration`.
- `controller_created_resources_have_managedby_controller`: Provider lifecycle
  controller creates a child `Process`; assert the child carries
  `metadata.managedBy = "controller"` and no `managedBy = "configuration"`.
- `absent_resource_receives_delete_on_new_generation`: activate a bundle with
  `Provider/observability-otel`; activate a new bundle without it; assert
  `Provider/observability-otel` has `metadata.deletionRequestedAt` set and
  carries a `deletion-pending` condition (`status: "True"`,
  `reason: "ConfigRemoved"`); assert
  `ResourceMutation{event="delete-scheduled", trigger="config-cleanup"}` audit
  record is written.
- `cleanup_does_not_touch_controller_children`: config-owned `Provider`
  deleted; assert controller-owned `Process` child is deleted by the Provider
  lifecycle controller (finalizer clear), not by config-publication; assert
  config-publication does not issue a direct Delete on the `managedBy=controller`
  `Process`.
- `deletion_sets_deletionrequestedat_not_phase`: schedule Delete on a resource;
  assert `metadata.deletionRequestedAt` is set to a non-zero timestamp; assert
  no `"phase": "Deleting"` appears in the resource record.
- `final_deletion_is_atomic`: mock a resource with all finalizers cleared;
  assert that the `Deleted` audit record emission and the row/index removal
  occur in one store transaction; assert no intermediate state where the row
  is removed but the audit record is absent, or vice versa.
- `pending_cleanup_condition_set_on_zone`: scheduled Delete pending; assert
  Zone `status.conditions` contains `type="pending-cleanup"`, `status="True"`.
- `zone_is_degraded_not_failed_during_cleanup`: scheduled Delete pending;
  assert Zone `status.phase = "Degraded"` (not `"Failed"`).
- `pending_cleanup_cleared_after_deletion_completes`: all scheduled Deletes
  complete; assert `pending-cleanup` condition removed; assert Zone phase
  transitions to `Ready` (absent other conditions).
- `prior_generation_retained_count_based`: new generation activated; assert
  up to 3 prior generation bundles are retained in the Zone store; assert
  generation 4 causes the oldest prior bundle to be pruned (after all its
  cleanup is complete).
- `rollback_schedules_delete_for_new_generation_resources`: activate G1,
  activate G2 (adds Provider), rollback to G1; assert Provider receives
  Delete; assert `pending-cleanup` is set on the Zone.
- `audit_segments_preserved_on_provider_delete`: delete
  `Provider/observability-otel`; assert audit segment files in
  `$ZONE_STATE/audit/` are unchanged; assert no segment is deleted as part
  of config cleanup.
- `cleanup_stall_condition_set`: inject a finalizer holder that never clears;
  wait >30 min (simulated); assert condition `cleanup-stalled` appears with
  a closed-enum `reason` field.
- `generation_rejected_emits_audit_record`: submit a bundle with a
  schema-validation failure; assert no resources are changed in the Zone store;
  assert a `ResourceMutation{event="generation-rejected", reason=
  "schema-validation-failed"}` audit record is written.



| Item | Treatment |
| --- | --- |
| Current anchor | (1) `packages/d2bd/src/metrics.rs`: 16-metric hand-rolled Prometheus registry; `vm` name labels; `VM_START_BUCKETS_SECONDS`, `BROKER_REQUEST_BUCKETS_SECONDS`, `ACTIVATION_PHASE_BUCKETS_SECONDS`. (2) `packages/d2bd/src/daemon_audit.rs`: hash-chain JSONL; `DaemonEvent` variants; `VmStartRunnerExitReason`, `RunnerExitKind`, `VmShutdownProvider` enums. **No** `DaemonEvent` for unsafe-local launches — this is a gap documented in the ProcessEffect section. (3) `packages/d2b-priv-broker/src/audit.rs`: `AuditWriteClass`, `AuditDropSummary`, rate-limit, O_APPEND, rotation. (4) `packages/d2b-realm-core/src/audit.rs`: `AuditHash`, `AuditChainLink`, `AuditChainRecord{realm: RealmPath, node: NodeId}`, `AuditStreamKind::{Gateway,RemoteNode,Daemon}`, `AuditEnvelope{realm, node, workload, principal}`, `AuditSinkHealth`. (5) `packages/d2b-realm-core/src/trace_context.rs`: `TraceContext{trace_id, span_id}`. (6) `packages/d2b-realm-core/src/ids.rs`: `RealmId`, `WorkloadId`, `NodeId`, `PrincipalId`, `OperationId`, `CorrelationId`. (7) `packages/d2b-gateway/src/audit.rs`: `GatewayAuditEvent`, `GatewayAuditKind`. (8) `packages/d2b-gateway-runtime/src/audit_jsonl.rs`: `JsonlGatewayAudit`, `DEFAULT_GATEWAY_AUDIT_RETENTION_DAYS`. (9) `packages/d2b-priv-broker/src/ops/audit_op.rs`: `OpAuditRecord`, `SwtpmDirAudit`. (10) `packages/d2b-host/src/otel_host_bridge_argv.rs`, `packages/d2bd/src/otel_host_bridge_readiness.rs`. (11) `nixos-modules/components/observability/{host,stack,guest}.nix`: `scrapeJournal`, `identityName`/`vmName`, `vm.name`/`vm.env`/`vm.role` OTEL resource attributes, SigNoz stack. (12) `packages/d2b-contract-tests/tests/{policy_observability,policy_metrics,minijail_relay_otel}.rs`. (13) `packages/d2b/tests/{audit_contract,host_doctor_contract}.rs`. (14) `packages/d2b-priv-broker/tests/broker_export_audit.rs`. (15) **unsafe-local sources**: `packages/d2b-core/src/unsafe_local_workloads.rs` (`UnsafeLocalWorkloadsJson`, `UnsafeLocalWorkload`, `UnsafeLocalLauncherItem`, `UnsafeLocalExecItem`, `UnsafeLocalShellItem`, `UnsafeLocalShellPolicy`, `UNSAFE_LOCAL_WORKLOADS_SCHEMA_VERSION`, `MAX_UNSAFE_LOCAL_WORKLOADS`); `packages/d2b-contracts/src/unsafe_local_wire.rs` (`HelperHello`, `HelperLaunchRequest`, `HelperShellRequest`, `HelperFailureCode`, `HelperScopeKind`, `DaemonToUnsafeLocalHelper`, `UnsafeLocalHelperToDaemon`); `packages/d2bd/src/unsafe_local_helper.rs` (`HelperRegistry`, `HelperConnection`, `dispatch_launch`, `allowed_uids`, `bind_helper_socket`); `packages/d2b-unsafe-local-helper/src/{main,protocol,runtime,systemd}.rs` (`HelperClient`, `ScopeRuntime`, `run_scope_supervisor`, `SystemdUserScopeManager`); `nixos-modules/options-realms-workloads.nix` (lines 221, 233–235, 264–275: `kind = "unsafe-local"`, null `stateDir`/`runDir`); `nixos-modules/unsafe-local-workloads-json.nix`; `nixos-modules/unsafe-local-helper.nix`. |
| Evidence class | (1) Hand-rolled metrics: implemented-and-reachable (no OTEL SDK). (2-4) Audit JSONL/hash/rate-limit: implemented-and-reachable. (5-6) TraceContext/IDs: implemented-and-reachable. (7-9) Gateway/broker/op audit: implemented-and-reachable. (10) OtelHostBridge runner: implemented-and-reachable. (11) Nix OTEL pipeline: implemented-and-reachable for the v1 daemon; the v3 `observability-otel` Provider is ADR-only. (12-14) Tests: implemented-and-reachable. (15) unsafe-local: implemented-and-reachable; **gap**: no `DaemonEvent` for unsafe-local launch/stop in current daemon. |
| Behavior retained | SHA-256 hash chain `prev_hash`/`record_hash`; O_APPEND JSONL segment files; privileged-never-dropped audit rate-limit invariant; `TraceContext` opaque bounded fields; SigNoz backend + OTEL Collector pipeline shape; `vm.name`/`vm.env`/`vm.role` OTEL resource attributes (advisory); journald scrape option; startup-tracing-avoids-host-path policy; `loki_native_otel_resource_attributes` closed allowlist; `broker_export_audit` admin-only / path-free / NDJSON contract; `UnsafeLocalWorkload` private-bundle-only argv/shell policy; `HelperRegistry::allowed_uids` per-UID isolation |
| Required delta | Lightweight `BoundedEmitter` crate (no OTEL SDK in core); v3 metrics with no `vm`-name labels; traces with `d2b.zone`/`d2b.provider` resource attributes; v3 resource/RBAC/session/route/state-reset audit records; `zone` field replacing `realm: RealmPath` in all audit records; per-Zone emitter socket; `observability-otel` Provider (full OTEL SDK in its own Process only); `d2b zone doctor`/`support-bundle` CLI; performance histogram benchmarks; **user-only Host resource `isolationPosture: "none"` status field**; **ProcessEffect `no_isolation: true` for all user-only Host (unsafe-local successor) process launches and stops** (gap fill); **CLI/UI isolation warning for user-only Host only**; `isolation-posture-declared` doctor check |
| Reuse path | Extract `TraceContext` + `AuditHash` + `AuditChainLink` to `d2b-telemetry` unchanged; copy hash-chain append/rate-limit/rotation logic from `daemon_audit.rs` and broker `audit.rs`; adapt `OpAuditRecord` BrokerEffect fields; copy Nix OTEL pipeline shape from `{host,stack,guest}.nix`; adapt `policy_observability`/`policy_metrics` tests; adapt `broker_export_audit` test scaffold; adapt `host_doctor_contract.rs` test harness; adapt `UnsafeLocalWorkload` private-bundle contract for Host resource spec |
| Replacement/deletion | `d2bd/src/metrics.rs` hand-rolled registry removed only after OTEL SDK metrics reach parity in all covered paths; `otel_host_bridge_argv.rs` socat runner retired after `observability-otel` Provider delivers native OTLP/vsock; daemon/broker/gateway `*_audit.rs` JSONL writers retired per-component after `d2b-audit` sink achieves parity; `AuditStreamKind` renamed `Daemon→Zone`, `Gateway→ZoneLink`, `RemoteNode→RemoteZone` in `d2b-telemetry`; `AuditChainRecord{realm, node}` re-versioned to `{zone}` in `d2b-audit`; `d2b-unsafe-local-helper` binary and `DaemonToUnsafeLocalHelper` wire protocol retired after Process Provider supervisor ticket migration for Host resources |
| Feasibility proof | `TraceContext` proven. Hash-chain JSONL proven in broker/daemon. Rate-limit/rotation proven in broker. `BoundedEmitter` datagram-socket pattern follows existing `otelRuntimeDir` ACL + socket design. OTEL SDK + Unix socket exporter proven in the v3 Nix OTEL pipeline (Provider-only). SigNoz stack operational. `broker_export_audit` path-free NDJSON proven. `host_doctor_contract` env-redirect scaffold proven |
| Future owner | Work items below |

## Implementation work items

### ADR046-telem-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-telem-001` |
| Dependency/owner | W0/W1a; telemetry crate owner |
| Current source | `packages/d2b-realm-core/src/trace_context.rs` (`TraceContext`, `MAX_TRACE_FIELD_LEN`); `packages/d2b-realm-core/src/audit.rs` (`AuditHash`, `AuditHashError`, `AuditChainLink`, `AuditChainRecord`); `packages/d2b-realm-core/src/ids.rs` (`OperationId`, `CorrelationId`); `packages/d2b-realm-codec-protobuf/src/lib.rs` (`encode_trace_context`, `decode_trace_context`); `packages/d2b-contract-tests/tests/policy_observability.rs::startup_tracing_avoids_host_path_fields` |
| Reuse action | extract unchanged (`TraceContext`, `AuditHash`, `AuditChainLink`); adapt (`OperationId`/`CorrelationId` for v3 record contract); add bounded emitter |
| Destination | `packages/d2b-telemetry/src/{trace_context.rs,audit_hash.rs,emitter.rs,meter_registry.rs,redaction_guard.rs}` |
| Detailed design | `d2b-telemetry` provides: (1) `TraceContext` / `AuditHash` / `AuditChainLink` extracted unchanged; (2) `BoundedEmitter`: `tracing`-subscriber layer that serializes span/metric events into compact frames and writes them over a private Unix datagram socket to the `observability-otel` Provider — no `opentelemetry_sdk` dependency; (3) `RedactionGuard` span wrapper that asserts the v3 resource attribute allowlist at span creation. No OTEL SDK in this crate. |
| Integration | Every v3 core process initializes a `BoundedEmitter` pointing at `$ZONE_STATE/telemetry/emitter.sock`; v3 audit records use `AuditHash`/`AuditChainLink` from this crate |
| Data migration | None |
| Validation | Unit test for `RedactionGuard` attribute gate; unit test for `BoundedEmitter` ring-full drop and FIFO drain; `policy_telemetry_redaction.rs::startup_tracing_avoids_host_path_fields` port; assert `config_source = "realm-controllers"` absent; assert no `opentelemetry_sdk` dependency in `d2b-telemetry` Cargo.toml |
| Removal proof | Not applicable |

### ADR046-telem-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-telem-002` |
| Dependency/owner | ADR046-telem-001 + ADR046-store-001; store owner |
| Current source | `packages/d2bd/src/metrics.rs` (`MetricDescriptor`, `MetricKind`, `VM_START_BUCKETS_SECONDS`, `BROKER_REQUEST_BUCKETS_SECONDS`, `ACTIVATION_PHASE_BUCKETS_SECONDS` — for bucket pattern reference only; the `vm` labels are not reused) |
| Reuse action | adapt bucket boundary constants (rename; remove `vm` labels); replace hand-rolled `Registry` with `d2b-telemetry` `BoundedEmitter` meter API |
| Destination | `packages/d2b-resource-store-redb/src/metrics.rs`, `packages/d2b-resource-store-redb/src/tracing.rs` |
| Detailed design | Instrument the store actor, write/read/group-commit paths with the metric inventory from this spec via `d2b-telemetry` `BoundedEmitter`. Emit `d2b.store.*` spans. The p95 `d2b_store_write_duration_seconds` hard target (≤10 ms) feeds the benchmark fixture. No `vm` label; `resource_type` label only from closed catalog. No OTEL SDK in the store crate. |
| Integration | Store actor calls `d2b-telemetry` meter/tracer via `BoundedEmitter`; spans linked to API request spans via `TraceContext` |
| Data migration | None |
| Validation | p95 write ≤10 ms benchmark fixture; metric inventory policy test asserting no `vm` label; assert old `d2b_daemon_vm_state` shape absent |
| Removal proof | Hand-rolled registry in `d2bd/src/metrics.rs` retained until daemon-level ADR 0046 cutover |

### ADR046-telem-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-telem-003` |
| Dependency/owner | ADR046-telem-001 + ADR046-session-001 + ADR046-bus-001; session/bus owner |
| Current source | `packages/d2b-realm-core/src/ids.rs` (`OperationId`, `CorrelationId`); `packages/d2b-realm-codec-protobuf/src/lib.rs` (`encode_trace_context`, `decode_trace_context` for v3 codec adaptation); `packages/d2b-realm-router/src/mux_session.rs`, `route_engine.rs` (current implemented-but-unwired generic routing) |
| Reuse action | adapt `TraceContext` protobuf codec for v3 resource API framing; adapt routing metrics patterns |
| Destination | `packages/d2b-resource-api/src/metrics.rs`, `packages/d2b-session/src/metrics.rs`, `packages/d2b-bus/src/metrics.rs` |
| Detailed design | Instrument resource API verb dispatch, watch delivery, bus route resolution, and session handshake/reconnect per the metric/span catalog in this spec. Propagate `TraceContext` from incoming bus request to store write transaction span as child context. |
| Integration | ResourceClient → bus → API → store span chain via `TraceContext` |
| Data migration | None |
| Validation | API request metric inventory test; session profile/outcome label cardinality gate; bus direction label gate; assert no `realm` field in span attributes |
| Removal proof | Not applicable |

### ADR046-telem-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-telem-004` |
| Dependency/owner | ADR046-telem-001 + ADR046-core-001; core-controller owner |
| Current source | `packages/d2bd/src/metrics.rs` (`BROKER_REQUEST_BUCKETS_SECONDS` for reference; `d2b_daemon_broker_request_duration_seconds` labels `["op"]` as a cardinality-safe example); `packages/d2b-realm-core/src/allocator_engine.rs` (field `trace: Option<TraceContext>` at line 873 — existing trace context wiring pattern) |
| Reuse action | adapt bucket patterns; adapt trace-context-in-reconcile pattern from `allocator_engine.rs` |
| Destination | `packages/d2b-core-controller/src/metrics.rs`, `packages/d2b-core-controller/src/tracing.rs` |
| Detailed design | Emit `d2b.controller.hint` span at the instant the post-commit dispatcher fires; emit `d2b.controller.reconcile` child span at handler entry. Interval = p95 ≤5 ms target. `handler` label from closed set; no resource name labels. |
| Integration | Post-commit dispatcher creates hint span; handler creates child reconcile span via `TraceContext` |
| Data migration | None |
| Validation | `hint_to_handler_latency` benchmark with p95 ≤5 ms assertion; closed `handler` label set gate |
| Removal proof | Not applicable |

### ADR046-telem-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-telem-005` |
| Dependency/owner | ADR046-telem-001 + ADR046-process-001; Process Provider owner |
| Current source | `packages/d2bd/src/metrics.rs` (`d2b_daemon_vm_start_duration_seconds` `VM_START_BUCKETS_SECONDS`; `d2b_daemon_vm_shutdown_duration_seconds` `VM_SHUTDOWN_BUCKETS_SECONDS`; `d2b_daemon_vm_shutdown_total` labels `["vm", "vmm", "outcome"]` — `vmm` = current `RunnerRole` → v3 `provider`); `packages/d2bd/src/supervisor/pidfd.rs` (pidfd adoption/launch call sites); `packages/d2b-contracts/src/broker_wire.rs::RunnerRole` (`CloudHypervisor`, `QemuMedia`, `OtelHostBridge` etc. → v3 `provider` label values) |
| Reuse action | adapt launch histogram bucket constants; rename `vm` label to no label (process identity in resource attributes); rename `vmm`/`RunnerRole` → `provider` closed enum |
| Destination | `packages/d2b-provider-supervisor/src/metrics.rs`, `packages/d2b-provider-supervisor/src/tracing.rs` |
| Detailed design | `d2b_process_launch_duration_seconds`: start = instant Process controller receives commit-to-Ready hint; end = first OS spawn call (clone3 or systemd unit start). This implements p95 ≤20 ms. `provider` label replaces `vmm`/`RunnerRole` with the closed set `{minijail,systemd}`. No `vm` name label. A separate `d2b_process_ready_duration_seconds` histogram covers launch-attempt → readiness signal (not a hard target). |
| Integration | Process Provider controller start handler → supervisor ticket delivery → first spawn call |
| Data migration | None |
| Validation | `commit_to_launch_latency` benchmark with p95 ≤20 ms assertion; assert no `vm` label in process metrics; `vmm→provider` label rename gate |
| Removal proof | `d2b_daemon_vm_start_duration_seconds` (with `vm` label) retained in `d2bd/src/metrics.rs` until daemon-level ADR 0046 cutover |

### ADR046-telem-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-telem-006` |
| Dependency/owner | ADR046-process-001 + ADR046-provider-001; `observability-otel` Provider owner |
| Current source | `nixos-modules/components/observability/host.nix` (`otelRuntimeDir = "/run/d2b/otel"`, `hostEgressSocket`, ACL `setfacl` pattern, `scrapeJournal`, `hostCfg.identityName`); `nixos-modules/components/observability/stack.nix` (SigNoz stack, `ingressSources`, per-source `vmName`, `receiverGrpcPort`/`receiverHttpPort`, loopback binding, `cfg.signoz.listenPort`); `nixos-modules/components/observability/guest.nix` (`vm.name`/`vm.env`/`vm.role` identity stamping, guest collector); `packages/d2b-host/src/otel_host_bridge_argv.rs` (`OtelHostBridgeArgvInputs`; vsock forwarding); `packages/d2bd/src/otel_host_bridge_readiness.rs` (readiness gate pattern: `OtelHostBridgeReadiness::{Ready,Pending,Failed}`); `packages/d2b-core/src/processes.rs::ProcessRole::OtelHostBridge`; `packages/d2b-contracts/src/broker_wire.rs::RunnerRole::OtelHostBridge`; `packages/d2b-contract-tests/tests/{policy_observability.rs,minijail_relay_otel.rs}` |
| Reuse action | adapt Nix pipeline shape (replace per-VM `vmName` with per-Zone naming); adapt `OtelHostBridgeArgvInputs` vsock forwarding to native OTLP/gRPC-over-vsock; adapt readiness gate pattern (`OtelHostBridgeReadiness::Ready` → Provider phase `Ready`); adapt `ingressSources` per-VM → per-Zone |
| Destination | `packages/d2b-provider-observability-otel/src/`, `nixos-modules/components/observability/` (adapted files) |
| Detailed design | `Provider/observability-otel` is an **ordinary optional non-bootstrap Process** (not counted toward the ≤64 MiB mandatory core aggregate). It owns: (1) per-Zone datagram receiver socket at `$ZONE_STATE/telemetry/emitter.sock` (drains frames from core emitters) and OTLP/gRPC Unix socket at `$ZONE_STATE/telemetry/otlp.sock`; (2) the full OTEL SDK with OTLP exporter — only this process links `opentelemetry_sdk`; (3) OTel Collector pipeline per Zone and per Host; (4) vsock OTLP forwarding to obs Zone (replaces socat-based `OtelHostBridgeArgvInputs`); (5) SigNoz stack Nix adapted from `stack.nix` with per-Zone `ingressSources` replacing per-VM `vmName`; (6) journald scrape (optional, disabled by default); (7) self-metrics endpoint. Zone/controller startup does not wait for this Provider. If absent or unready, Zone health is `Degraded` (not `Failed`). Readiness: socket exists and first drain cycle completes successfully. `d2b.observability.host.identityName` option preserved; `vmName` in `ingressSources` populated from Zone name. |
| Integration | Core process `BoundedEmitter` → `emitter.sock` → observability-otel collector → `otlp.sock` → vsock → obs Zone SigNoz; Zone startup independent of Provider readiness |
| Data migration | Existing SigNoz data not migrated; v3 starts fresh |
| Validation | `emitter_socket_receive`, `emitter_ring_drains_on_socket_available`, `emitter_ring_drop_on_overflow`, `no_vm_label_in_metrics`, `zone_startup_proceeds_without_provider` tests; adapted `policy_observability.rs` tests (retain `loki_native_otel_resource_attributes` and SigNoz-only backend assertions); adapted `minijail_relay_otel.rs` shape test for Provider-managed runner |
| Removal proof | `otel_host_bridge_argv.rs` socat runner and `otel_host_bridge_readiness.rs` retired after `observability-otel` Provider delivers native OTLP/vsock and passes conformance; `ProcessRole::OtelHostBridge` and `RunnerRole::OtelHostBridge` retired from `d2b-core/src/processes.rs` and `d2b-contracts/src/broker_wire.rs` after Provider migration |

### ADR046-audit-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audit-001` |
| Dependency/owner | W0/W1a; audit crate owner |
| Current source | `packages/d2b-realm-core/src/audit.rs` (`AuditHash::parse`, `AuditChainLink::new`/`verify`, `AuditChainRecord{stream: AuditStreamKind, realm: RealmPath, node: NodeId}`, `AuditStreamKind::{Gateway,RemoteNode,Daemon}`, `AuditSinkHealth`, `AuditRetentionFloorStatus`); `packages/d2bd/src/daemon_audit.rs` (hash-chain append algorithm, `prev_hash`/`record_hash` SHA-256 pattern, daily segment files `daemon-events-YYYY-MM-DD.jsonl`, `DaemonEvent` additive contract); `packages/d2b-priv-broker/src/audit.rs` (`AuditWriteClass::{Privileged,Unprivileged}`, `AuditDropSummary`, `DEFAULT_AUDIT_WRITES_PER_SECOND = 4096`, O_APPEND CLOEXEC file open, `AuditDropWarningState`); `packages/d2b-gateway-runtime/src/audit_jsonl.rs` (`JsonlGatewayAudit`, `DEFAULT_GATEWAY_AUDIT_RETENTION_DAYS = 14`, `prune_old` rotation algorithm); `packages/d2b-priv-broker/src/ops/audit_op.rs` (`OpAuditRecord`, `SwtpmDirAudit`, `SwtpmDirResult`, `SwtpmMarkerResult`); `packages/d2b-realm-core/src/ids.rs` (`OperationId`, `CorrelationId`, `PrincipalId` — `PrincipalId` becomes `subject_digest`); `packages/d2b/tests/audit_contract.rs`; `packages/d2b-priv-broker/tests/broker_export_audit.rs` |
| Reuse action | extract unchanged: `AuditHash`, `AuditChainLink` from `d2b-realm-core/src/audit.rs`; copy hash-chain append algorithm from `daemon_audit.rs`; copy `AuditWriteClass`/rate-limit/rotation/prune from broker `audit.rs`; adapt `JsonlGatewayAudit` segment writer; adapt `OpAuditRecord` to `BrokerEffect` record class |
| Destination | `packages/d2b-audit/src/{hash_chain.rs,segment.rs,rate_limit.rs,record_types.rs,sink.rs,export.rs}` |
| Detailed design | `d2b-audit` provides: typed record structs per class; canonical serialization with `zone` replacing `realm: RealmPath`; SHA-256 hash chain (extracted from `daemon_audit.rs`); segment writer (O_APPEND CLOEXEC, 64 MiB / UTC-midnight rotation); 30-day compaction (adapts `prune_old` from `JsonlGatewayAudit`); `AuditWriteClass::{Privileged,Standard,BestEffort}` (extends current `{Privileged,Unprivileged}`); rate-limit with privileged-never-dropped invariant; export iterator with inline hash-break reporting. `AuditStreamKind` re-versioned: `Daemon→Zone`, `Gateway→ZoneLink`, `RemoteNode→RemoteZone`. `AuditChainRecord` re-versioned: `{zone: String}` replaces `{realm: RealmPath, node: NodeId}`. |
| Integration | Zone runtime, core-controller, Process Providers, broker effect bridge → `d2b-audit` sink; `d2b zone audit export` → export iterator |
| Data migration | v3 bootstrap; existing daemon/broker JSONL files not migrated |
| Validation | `audit_record_hash_chain`, `audit_record_schema` (no `realm`/`node` fields), `audit_segment_rotation`, `audit_rate_limit_privileged_never_dropped`, `audit_unavailable_blocks_privileged` |
| Removal proof | `daemon_audit.rs`, broker `audit.rs`, `JsonlGatewayAudit` retired per-component after `d2b-audit` sink achieves parity |

### ADR046-audit-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audit-002` |
| Dependency/owner | ADR046-audit-001 + ADR046-store-001; store/authz owner |
| Current source | `packages/d2b-priv-broker/src/ops/audit_op.rs` (`OpAuditRecord` structural pattern — operation, peer_uid, decision, result fields); `packages/d2b-realm-core/src/audit.rs::AuditEnvelope{principal: PrincipalId, scope: AuthorizationScope, decision: AuthzDecision}` (principal → v3 `subject_digest`) |
| Reuse action | adapt `OpAuditRecord` structural pattern for `ResourceMutation` / `RBACChange` record classes; adapt `PrincipalId` → `subject_digest` derivation |
| Destination | `packages/d2b-resource-store-redb/src/audit.rs`, `packages/d2b-core-controller/src/authz_audit.rs` |
| Detailed design | `ResourceMutation` records emitted by the store actor inside the write transaction before commit returns. The audit sink must durably fsync the audit record before returning the commit success (privileged durability class). `RBACChange` emitted by the authz handler in the same write transaction. `subject_digest` = SHA-256 of normalized canonical subject string from v3 `AuthenticatedSubjectContext` (ADR-046-componentsession-and-bus). |
| Integration | Store write transaction → `d2b-audit` sink → fsync → commit result |
| Data migration | None |
| Validation | Integration test: 100 mutations → verify hash-chained audit records with `zone` field, no `realm` field |
| Removal proof | Not applicable |

### ADR046-audit-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audit-003` |
| Dependency/owner | ADR046-audit-001 + ADR046-session-001 + ADR046-bus-001; session/bus owner |
| Current source | `packages/d2b-gateway/src/audit.rs` (`GatewayAuditEvent`, `GatewayAuditKind::{DisplaySessionOpenAdmitted,DisplaySessionOpenDenied,DisplaySessionRunning,DisplaySessionClosed}`, `GatewayAudit` trait, `NoopGatewayAudit`); `packages/d2b-realm-core/src/audit.rs::AuditEnvelope{realm, principal, scope, decision, trace}` (fields adapted to v3 `SessionConnect` record class) |
| Reuse action | adapt `GatewayAudit` trait pattern for `SessionConnect` and `RouteAdmission` record classes; `NoopGatewayAudit` pattern reused for test sinks |
| Destination | `packages/d2b-session/src/audit.rs`, `packages/d2b-bus/src/audit.rs` |
| Detailed design | `SessionConnect` records emitted at handshake completion. `GatewayAuditKind::DisplaySessionOpenAdmitted/Denied` → `event="connect"`, `authz_decision="allowed/denied"`. `GatewayAuditKind::DisplaySessionRunning` → informational `ProcessEffect`. `GatewayAuditKind::DisplaySessionClosed` → `event="close"`. `transport_class=zone_link` covers what the current `AuditStreamKind::Gateway` stream recorded for gateway-backed realm sessions. `RouteAdmission` records emitted at bus route resolution for denied routes. |
| Integration | Session engine and bus router → `d2b-audit` sink |
| Data migration | None |
| Validation | Session connect/close/auth-failure audit tests; `GatewayAuditKind` → `SessionConnect` mapping test |
| Removal proof | `NoopGatewayAudit` and gateway JSONL sink retired after gateway is on v3 resource API |

### ADR046-audit-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-audit-004` |
| Dependency/owner | ADR046-audit-001; CLI owner |
| Current source | `packages/d2b/tests/audit_contract.rs` (`d2b audit --strict` returns 78; `auditResponse` relay; `authz-audit-requires-admin` denial; daemon-down exit 1 without bash fallback); `packages/d2b-priv-broker/tests/broker_export_audit.rs` (`export_audit_requires_admin_and_exports_op_audit_records`: admin-only, path-free, NDJSON `ExportBrokerAuditOk` shape, `peer_uid` field, `ApplyNftables` operation name in records) |
| Reuse action | adapt audit CLI contract test (daemon-down/exit behavior); adapt broker export test to new `zone` field and v3 record schema |
| Destination | `packages/d2b/src/zone_audit.rs` (new `d2b zone audit export` subcommand); `packages/d2b/tests/zone_audit_contract.rs` |
| Detailed design | `d2b zone audit export` opens segments read-only (shared flock), streams NDJSON to stdout, validates hash chain inline, reports breaks as inline error records, enforces `audit-export` verb via resource API (admin-only, same `SO_PEERCRED`/Role check as current `ExportBrokerAuditOk`). Assert no `realm`, `node`, `workload_id` fields in exported records. |
| Integration | `d2b` CLI → resource API `audit-export` verb → `d2b-audit` export iterator → stdout |
| Data migration | None |
| Validation | `export_audit.rs`: admin-only, hash break inline, no old field names (`realm`/`node`/`workload_id`), no path/argv in output, exit 0 on clean chain |
| Removal proof | `d2b audit` legacy command retained until `d2b zone audit export` covers all record classes |

### ADR046-doctor-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-doctor-001` |
| Dependency/owner | ADR046-core-001 + ADR046-audit-001; CLI/doctor owner |
| Current source | `packages/d2b/tests/host_doctor_contract.rs` (env-redirect sandbox: `D2B_BROKER_SOCKET`, `D2B_PUBLIC_SOCKET`, `D2B_DAEMON_STATE_DIR`, `D2B_METRICS_URL`, `D2B_MANIFEST_PATH`; `doctor::render_summary` JSON envelope fields: `command`, `mode`, `broker_ready`, per-check `status`+`data`, `summary`, `exitCode`); `packages/d2bd/src/audit_check.rs` (`defects` array audit-chain validation pattern); `packages/d2bd/src/lib.rs` (doctor read-only path: `host doctor --read-only` reads from `D2B_DAEMON_STATE_DIR`, pidfd_table file, kernel-module check file, metrics URL) |
| Reuse action | adapt env-redirect sandbox test harness; adapt `defects` array pattern for `audit-hash-chain-clean` check; adapt `broker_ready` → `zone_phase` field |
| Destination | `packages/d2b/src/zone_doctor.rs`, `packages/d2b/tests/zone_doctor_contract.rs` |
| Detailed design | `d2b zone doctor [--zone <name>] [--json]` reads resource status from Zone API (read-only verb), OTEL self-metrics from `observability-otel` Provider endpoint (optional), and audit segment inventory from `d2b-audit` segment reader. Named check set from this spec. Exit 0 on all-ready; 1 on any warn/error. Env-redirect sandbox for all test fixtures. Current `MANIFEST_JSON` fixture pattern adapted: `"_observability": {"enabled": false}` test ensures OTEL probe short-circuits cleanly. |
| Integration | `d2b` CLI → Zone resource API status reads + audit segment reader |
| Data migration | None |
| Validation | `zone_doctor_contract.rs`: all-ready/degraded/quarantine/otel-absent/audit-absent fixtures; no resource names/paths/argv/PIDs; `zone_phase` field present; no legacy `broker_ready` field |
| Removal proof | `d2b host doctor` retained until `d2b zone doctor` covers all check parity |

### ADR046-doctor-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-doctor-002` |
| Dependency/owner | ADR046-doctor-001; CLI/doctor owner |
| Current source | `packages/d2b/tests/host_doctor_contract.rs` (env-redirect sandbox scaffold — no current `support-bundle` equivalent exists) |
| Reuse action | reuse env-redirect sandbox scaffold |
| Destination | `packages/d2b/src/zone_support_bundle.rs`, `packages/d2b/tests/zone_support_bundle_contract.rs` |
| Detailed design | `d2b zone support-bundle [--zone <name>]` requires `support-bundle` verb. Reads bounded resource status snapshots (32 per type, 512 total; metadata + status only; no spec bytes; no `metadata.name`), controller queue depths, schema catalog (names+versions only), audit segment inventory, OTEL collector metrics summary, bounded structured log ring (2000 entries). NDJSON output. On quarantine: `bundle_completeness: "partial"`, exit 1. |
| Integration | `d2b` CLI → Zone resource API list (status subresource only) + controller introspection + audit segment reader + OTEL self-metrics |
| Data migration | None |
| Validation | `zone_support_bundle_contract.rs`: complete/partial bundles; no spec/name/path/argv; field completeness |
| Removal proof | Not applicable |

### ADR046-telem-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-telem-007` |
| Dependency/owner | ADR046-telem-006; Nix/observability owner |
| Current source | `nixos-modules/components/observability/host.nix` (`scrapeJournal = hostCfg.scrapeJournal`, `journaldStorageDir = "/var/lib/d2b-host-otel-collector/journald"`, cgroup-path filtering pattern for host units) |
| Reuse action | adapt journald receiver config for per-Zone cgroup filter (`z-<zone-id>/*`, `s-<execution-id>/*`) |
| Destination | `packages/d2b-provider-observability-otel/src/nix/journald.nix` (new Nix fragment) |
| Detailed design | `d2b.zones.<name>.observability.journald.enable = false` (default). When enabled: journald receiver follows `z-<zone-id>/*` and `s-<execution-id>/*` cgroup filters. Collector applies redaction: drops `MESSAGE` credential/path patterns, `_CMDLINE`, `_EXE`, `INVOCATION_ID`. Current `scrapeJournal` host option is preserved unchanged. |
| Integration | `observability-otel` Provider Nix config → OTel Collector journald receiver → redaction filter → obs Zone |
| Data migration | None |
| Validation | Nix eval test: filter expression set when enabled; test that `_CMDLINE` and `INVOCATION_ID` appear in drop list |
| Removal proof | Not applicable |

### ADR046-telem-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-telem-008` |
| Dependency/owner | ADR046-telem-001; policy/contract-tests owner |
| Current source | `packages/d2b-contract-tests/tests/policy_observability.rs` (`loki_native_otel_resource_attributes` allowlist: `["deployment.environment","host.name","service.name","service.namespace","source","vm.env","vm.name","vm.role"]`; `tempo_stack_signoz_backend_and_collector` SigNoz-only assertion; `startup_tracing_avoids_host_path_fields` forbidden fields); `packages/d2b-contract-tests/tests/policy_metrics.rs` (`EXPECTED_METRICS` table parity with `docs/reference/daemon-metrics.md`) |
| Reuse action | adapt and extend; keep existing tests; add new policy gates |
| Destination | `packages/d2b-contract-tests/tests/policy_telemetry_redaction.rs` (new); updated `policy_observability.rs`; updated `policy_metrics.rs` |
| Detailed design | (1) Extend `loki_native_otel_resource_attributes` allowlist to include `d2b.zone`, `d2b.provider`, `d2b.component`, `service.version`. (2) Add redaction lint: scan all v3 instrumentation call sites for `realm`, `workload_id`, `node_id`, `vm` (as label key), `path`, `socket`, `argv`, `pid`, `exe`. (3) Add metric label gate: assert no v3 `MetricDescriptor` carries a `vm` label. (4) Add bucket boundary gates for 5 ms and 20 ms. (5) Retain: `startup_tracing_avoids_host_path_fields`; SigNoz-only backend assertion; `tempo_guest_collector_shape`; `config_source = "realm-controllers"` absence gate. |
| Integration | Contract-tests run in workspace check and `make test-drift` |
| Data migration | None |
| Validation | These tests are their own validation artifact |
| Removal proof | Not applicable |

### ADR046-host-posture-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-host-posture-001` |
| Dependency/owner | ADR046-audit-001 + ADR046-core-001; `Provider/system-core` owner |
| Current source | `packages/d2b-core/src/unsafe_local_workloads.rs` (`UnsafeLocalWorkloadsJson`, `UnsafeLocalWorkload`, `UnsafeLocalLauncherItem`, `UNSAFE_LOCAL_WORKLOADS_SCHEMA_VERSION = "v2"`, `MAX_UNSAFE_LOCAL_WORKLOADS = 256`); `packages/d2b-contracts/src/unsafe_local_wire.rs` (`HelperHello.uid: u32`, `HelperLaunchRequest`, `HelperShellRequest`, `HelperScopeKind::{Exec,Shell}`, `DaemonToUnsafeLocalHelper`, `UnsafeLocalHelperToDaemon`); `packages/d2bd/src/unsafe_local_helper.rs` (`HelperRegistry::new(daemon_uid, allowed_uids)`, `dispatch_launch`, `bind_helper_socket`); `packages/d2b-unsafe-local-helper/src/{main,protocol,runtime,systemd}.rs` (`HelperClient`, `ScopeRuntime`, `run_scope_supervisor`, `SystemdUserScopeManager`); `nixos-modules/options-realms-workloads.nix` (lines 221, 233–235 `kind = "unsafe-local"` description; lines 264–275 null `stateDir`/`runDir`); `nixos-modules/unsafe-local-workloads-json.nix` (`runtimeKind = "unsafe-local"`, `providerId = "unsafe-local"`); `nixos-modules/unsafe-local-helper.nix` (service unit) |
| Reuse action | adapt `UnsafeLocalWorkload` private-bundle contract for the `Host` resource spec payload; adapt `HelperRegistry::allowed_uids` constraint as `defaultUserRef=User/<name>` validation; adapt Nix `unsafe-local-workloads-json.nix` emitter for the new Host resource shape; gap-fill: add `ProcessEffect{no_isolation:true}` at `dispatch_launch` / stop call sites |
| Destination | `packages/d2b-provider-system-core/src/{host_reconciler.rs,host_status.rs,host_process_audit.rs}`; adapted `nixos-modules/unsafe-local-workloads-json.nix`; `packages/d2b-provider-system-core/tests/host_posture_contract.rs` |
| Detailed design | `Provider/system-core` reconciler: (1) On user-only `Host` resource creation (`defaultDomain=user`, `allowedDomains=[user]`), set `status.isolationPosture = "none"` and `status.isolationPostureMessage = "..."` unconditionally; reject any operator-supplied value for these fields. Host resources with other execution policies do not receive `isolationPosture`. (2) On every user-only Host process launch: emit `ProcessEffect{event:"launch", provider:"system-core-user", domain:"user", no_isolation:true, ...}` audit record. (3) On every user-only Host process stop: emit `ProcessEffect{event:"stop", ...}`. (4) `d2b zone list`/`inspect` CLI renders `⚠ no isolation boundary (user domain)` annotation only for `Host` resources with `isolationPosture: "none"`; annotation is not suppressible. (5) `isolation-posture-declared` doctor check: passes when user-only `Host` resource status has `isolationPosture: "none"`; omitted when Zone has no user-only `Host` resources. (6) `no_isolation=true` is emitted in `ProcessEffect` records only; it does not appear in any OTEL span attribute, log field, or metric label. |
| Integration | `Provider/system-core` reconciler → `d2b-audit` sink; `d2b zone doctor` → resource status check; `d2b zone list`/`inspect` → CLI output renderer |
| Data migration | None |
| Validation | `host_posture_contract.rs` tests from the Host posture tests section of this spec; `d2b-contract-tests/tests/policy_telemetry_redaction.rs` asserts `no_isolation` key absent from all span/metric/log surfaces |
| Removal proof | `d2b-unsafe-local-helper` binary and `DaemonToUnsafeLocalHelper`/`UnsafeLocalHelperToDaemon` wire types retired after `Provider/system-core` Process Provider supervisor ticket migration; `nixos-modules/unsafe-local-helper.nix` Nix unit retired after migration; `nixos-modules/unsafe-local-workloads-json.nix` adapted (not deleted) to emit Host resource spec format |

## Main-commit reuse work items

The ADR45 W5–W9 implementation in main commit `a1cc0b2da4a08ca3240a770a972fe4da6f912bef`
provides production-grade implementations of ComponentSession v2, d2b-bus routing,
async clients, Provider proxies, attachment/stream/cancellation subsystems, and their
conformance tests. These are **reuse sources** for v3, not pre-ADR-0045 v3 baseline
evidence. Each item below records: exact main commit file/symbol, selected behavior,
exact v3 destination/integration, and the ADR45-specific assumptions that must be
excluded or adapted.

### ADR046-reuse-001 — ComponentSession v2 runtime

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reuse-001` |
| Dependency/owner | ADR046-telem-001 (d2b-telemetry must exist first for MetricsSink injection); session owner |
| Main commit source | `packages/d2b-session/` at `a1cc0b2d`. Key files: `src/lib.rs` (full public surface), `src/handshake.rs` (`NoiseHandshake`, `HandshakeCredentials`, `HandshakeRole`, `NegotiatedOffer`, generation-discovery encode/decode, transcript-hash), `src/engine.rs` (`SessionEngine`, `SessionEvent`), `src/driver.rs` (`ComponentSessionDriver`, `SessionDriverHandle`), `src/cancellation.rs` (`Cancellation`, `RequestRegistry`, `CancelRequest`/`CancelAck`/`CancelResult`), `src/streams.rs` (`NamedStreamMux`, `StreamId`, `StreamPhase`, `StreamEvent`), `src/attachment.rs` (`OwnedAttachment`, `AttachmentPayload`, `AttachmentValidationError`), `src/metrics.rs` (`MetricEvent`, `MetricsSink`, `NoopMetrics`), `src/record.rs` (`ProtectedRecord`, `RecordProtector`), `src/scheduler.rs` (`FairScheduler`, `OutboundFrame`, `QueueClass`), `src/server.rs` (`serve_ttrpc_services`), `src/lifecycle.rs` (`SessionLifecycle`, `SessionPhase`, `KeepaliveAction`), `src/deadline.rs` (`DeadlineBudget`). Tests: `tests/component_session.rs` (noise vector tests, record-protection round-trip, schema-mismatch rejection, LimitProfile fixture), `tests/noise_vectors.rs` (committed KAT vectors). Contract source: `packages/d2b-contracts/src/v2_component_session.rs` (all `closed_enum!` tables, `LimitProfile`, `EndpointPolicy`, `EndpointPolicyIdentity`, `MetricLabels`, all 50+ constants including `MAX_ACTIVE_NAMED_STREAMS=128`, `MAX_SESSION_ATTACHMENTS=256`, `LOCAL_HANDSHAKE_DEADLINE_MS=5000`, `MAX_RECONNECT_ATTEMPTS=10`). |
| Selected behavior | Full Noise_NN/KK/IKpsk2 handshake engine; `RecordProtector` encrypt/decrypt; `FairScheduler` ttrpc/stream credit; `NamedStreamMux` open/close/reset/credit; `Cancellation`/`RequestRegistry` per-generation cancel; `OwnedAttachment` typed payload validation; `DeadlineBudget` deadline propagation; `MetricsSink` injection point; `serve_ttrpc_services` server loop; generation-discovery handshake. |
| Excluded ADR45 assumptions | `EndpointPurpose::{DaemonLocal, DaemonRemote, RealmPeer}` are ADR45 daemon topology names — v3 replaces with `Purpose::{ZoneLocal, ZoneLink, ProviderAgent}` (or adapts to closed-set). `PurposeClass::{Local, Enrolled, Bootstrap}` are retained verbatim. `ServicePackage::{DaemonV2, RealmV2, GuestV2}` are ADR45 service names — v3 replaces with the v3 bus service package catalog. `RealmId`/`WorkloadId` in `RealmSessionAuthority` (d2b-realm-router) are ADR45 realm concepts — v3 uses Zone name strings and resource UIDs. |
| v3 Destination | `packages/d2b-session/` copied verbatim (zero ADR45 topology assumptions in the core crates). Wire constants in `v2_component_session.rs` are adopted without change. `EndpointPurpose` enum values renamed in v3 contract extension; existing values kept for backward wire compatibility during transition. |
| Integration | `d2b-bus` route handler calls `serve_ttrpc_services`; `d2b-session-unix` provides `OwnedTransport` impl; `d2b-telemetry` `MetricsSink` impl feeds `d2b_session_*` metrics inventory from this spec. |
| Validation | Adopt `tests/component_session.rs` and `tests/noise_vectors.rs` unchanged; extend with v3 `EndpointPurpose` enum gate test; add `d2b-contract-tests/tests/component_session_v2_vectors.rs` (existing at `a1cc0b2d`) as-is. |

### ADR046-reuse-002 — Unix transport substrate

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reuse-002` |
| Dependency/owner | ADR046-reuse-001; Unix transport owner |
| Main commit source | `packages/d2b-session-unix/` at `a1cc0b2d`. Key files: `src/adapter.rs` (`UnixSeqpacketTransport`, `UnixStreamTransport`, `PeerIdentityPolicy`, `DescriptorPolicyResolver`, `OwnedUnixAttachment`, `UnixAttachmentPayload`, `PathnamePeerVerifier`), `src/credit.rs` (`CreditPool`, `CreditScope`, `CreditScopeSet`, `ProcessCreditLimit`, `CreditBundle`, `CreditError` — rollback-on-failure, emergency-headroom constants), `src/descriptor.rs` (`PeerCredentials`, `FirstPacketCredentials`, `ObjectIdentity`, `PidfdIdentityPolicy`, `DescriptorPolicy`, `AcceptedAttachment`, `ReceivedPacket`), `src/pidfd.rs` (`PidfdEvidence`, `PidfdIdentityVerifier`, `verify_pidfd`), `src/socket.rs` (`SeqpacketSocket`, `StreamSocket`), `src/vsock.rs` (vsock transport). Tests: `tests/unix_session.rs` (`ancillary_capacity_is_derived_from_closed_hard_bounds`, `process_limit_preserves_emergency_headroom`, `failed_multiscope_reservation_rolls_back_every_prior_scope`, `staged_credit_reservations_release_once_at_each_scope`, `inherited_passcred_is_verified_but_never_repaired`, `first_packet_has_exact_directional_credentials`, `seqpacket_transfer_is_atomic_cloexec_and_object_exact`, `duplicate_kernel_objects_are_rejected_and_cleaned_up`, `owned_transport_adapters_transfer_packets_and_owned_files_end_to_end`, `stream_transport_reassembles_partial_and_coalesced_records`, `stream_transport_distinguishes_clean_and_partial_eof`). |
| Selected behavior | SO_PASSCRED first-packet credential verification; pidfd identity verification; multi-scope credit reservation with rollback; emergency headroom reservation; seqpacket atomic cloexec transfer; stream transport reassembly; vsock transport; `DescriptorPolicy` per-fd type enforcement. |
| Excluded ADR45 assumptions | No ADR45-specific topology assumptions in this crate. `PeerIdentityPolicy` uses Unix `SO_PEERCRED` which is host-local — vsock peers use vsock CID policy instead; no change needed. |
| v3 Destination | `packages/d2b-session-unix/` copied verbatim. Feature flags `host-socket` / `native-vsock` retained unchanged. |
| Integration | `d2b-bus` Zone-local listeners use `UnixSeqpacketTransport`; Provider agent connections use vsock transport from this crate; `CreditPool`/`CreditScopeSet` enforce per-Zone attachment FD budgets. |
| Validation | Adopt all `unix_session.rs` tests unchanged. |

### ADR046-reuse-003 — Async client and retry layer

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reuse-003` |
| Dependency/owner | ADR046-reuse-001; client owner |
| Main commit source | `packages/d2b-client/` at `a1cc0b2d`. Key files: `src/client.rs` (`Client<R,C,W>`, `ConnectedClient`, `CallOptions`, `RetryPolicy`, `CancellationToken`, `MetadataInput`, `Response`, `WallClock`, `SystemClock`), `src/service.rs` (`ServiceHandle`, `MethodHandle`, `GeneratedClient`, `ServiceKind`), `src/session.rs` (session reconnect and credential binding), `src/target.rs` (target/resolver abstraction), `src/daemon_service.rs` (`DaemonClient`, `DaemonMethod`, `DaemonLifecycleRequest`, `DaemonTerminal`, `daemon_call_options`), `src/guest_service.rs` (`GuestClient`, `GuestOperation`, `GuestInspectCall`, `GuestCancelCall`, `GuestRetainedLogCall`), `src/host_socket.rs` (`HostSocketConnector`, `local_daemon_endpoint_identity`), `src/error.rs` (`ClientError`, `RemoteErrorKind`, `RetryClass`). Tests: `client.rs` (`typed_routes_select_exact_transport_without_fallback`, `daemon_typed_list_preserves_projection_and_truncation`, `daemon_typed_errors_and_generation_mismatch_are_actionable`, `guest_exec_management_preserves_typed_state_and_cancel_correlation`, `guest_retained_log_open_binds_range_resource_and_selection`, `terminal_uses_server_stream_and_validates_bidirectional_lifecycle`, `daemon_guest_proxy_reuses_the_authenticated_session`, `absent_daemon_guest_proxy_fails_closed_without_reconnecting`). |
| Selected behavior | `MetadataInput` W3C trace-id propagation (`with_trace([u8; 16])`), correlation, idempotency-key; `RetryPolicy` max-attempts; `CancellationToken`; `Client` generics over resolver/connector/clock; typed `DaemonClient`/`GuestClient` service proxies; stream-based terminal; `local_daemon_endpoint_identity` host-socket verifier. |
| Excluded ADR45 assumptions | `DaemonClient`/`GuestClient` wrap ADR45 `DaemonV2`/`GuestV2` service packages. v3 replaces with v3 service package names. `local_daemon_endpoint_identity` uses `RealmPath::parse("local-root")` — v3 uses the Zone name for the local-root Zone. `DaemonMethod`/`GuestMethod` enums are ADR45 method sets — v3 replaces with v3 bus service method sets. |
| v3 Destination | `packages/d2b-client/` copied; `DaemonClient`/`GuestClient` adapted to v3 service packages; `local_daemon_endpoint_identity` adapted to use Zone name; `MetadataInput::with_trace` drives `TraceContext` propagation into d2b-bus route requests. |
| Integration | Every controller/service that makes outbound calls uses `Client`; `MetadataInput::with_trace` feeds `d2b_api_request_duration_seconds` trace-id into `d2b.bus.route` span. |
| Validation | Adopt typed-route, proxy-reuse, and cancel tests unchanged. Add v3 service-package name gate test. |

### ADR046-reuse-004 — Provider registry, RPC proxy, and conformance toolkit

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reuse-004` |
| Dependency/owner | ADR046-reuse-001 + ADR046-reuse-003; Provider owner |
| Main commit source | `packages/d2b-provider/` at `a1cc0b2d`. Files: `src/registry.rs` (`ProviderRegistry`, `ProviderRegistryBuilder`, `RegistryLimits`, `AdmissionOptions`), `src/rpc.rs` (`RpcCall`, `RpcPayload`, `RpcResponse`, `RpcOperation`, `AuthenticatedProviderRpc`, `RpcProviderProxy`, `SessionIdentity`), `src/instance.rs` (`ProviderInstance`, `ProviderFactory`, `CancellationToken`), `src/context.rs` (`ProviderCallContext`, `OwnedOperationContext`), `src/error.rs` (`ProviderRuntimeError`, `RegistryBuildError`, `FactoryError`). Tests: `runtime.rs` (`closed_error_context_is_actionable_without_identity_leaks`, `health_uses_no_input_inspection_methods_for_every_axis`, `all_provider_traits_are_object_safe`). `packages/d2b-provider-toolkit/` at `a1cc0b2d`. Files: `src/server.rs` (`GeneratedProviderServiceServer`, session admission, ttrpc method dispatch), `src/adapter.rs` (`invoke_session` → `AuthenticatedProviderRpc` bridge), `src/conformance.rs` (`check_descriptor_conformance`, `check_provider_conformance`), `src/fixture.rs` (conformance fixture), `src/redaction.rs` (`Redacted<T>`, `Secret<T>`), `src/registration.rs`, `src/values.rs`. Tests: `tests/conformance.rs` (`every_axis_passes_identical_in_process_and_rpc_conformance`, `conformance_uses_the_exact_real_descriptor_placement_and_target`, `provider_values_preserve_all_descriptor_and_operation_bindings`, `exact_registration_supports_all_axes_and_shared_factories`, `adapter_rejects_authenticated_identity_mismatch`, `rpc_proxy_fails_closed_on_cancellation_and_method_mismatch`, `rpc_proxy_preserves_plan_handle_and_adoption_bindings`, `rpc_proxy_rejects_mismatched_observability_query_results`, `redaction_wrappers_do_not_expose_canaries`). |
| Selected behavior | `ProviderRegistryBuilder` factory registration with `RegistryLimits` bound enforcement; `RpcProviderProxy` `AuthenticatedProviderRpc` bridge with cancellation + method mismatch fail-closed; `GeneratedProviderServiceServer` session admission + ttrpc dispatch; `Redacted<T>`/`Secret<T>` Debug suppression; `check_descriptor_conformance`/`check_provider_conformance` conformance gate. |
| Excluded ADR45 assumptions | `ProviderRegistry` is wired to ADR45 `d2bd` daemon via `DaemonEffectAdapters` (`provider_effects.rs` in d2bd). In v3, Providers are independent processes registering with the Zone runtime via the resource API, not daemon-embedded. The `SessionIdentity` in `rpc.rs` carries `realm: RealmId` — v3 replaces with Zone name string. `ProviderRealmPath`/`WorkloadId` imports in `d2bd/src/provider_registry.rs` are ADR45 daemon coupling — excluded. |
| v3 Destination | `packages/d2b-provider/` and `packages/d2b-provider-toolkit/` copied; `SessionIdentity` adapted to use Zone name; `ProviderRegistry` wired to v3 bus service via `d2b-bus` route handler instead of daemon embedding; `GeneratedProviderServiceServer` session admission adapted to v3 `EndpointPurpose` values. |
| Integration | Each v3 Provider process embeds `ProviderRegistry` + `GeneratedProviderServiceServer`; `check_provider_conformance` runs in Provider install-time conformance check (feeds `d2b_provider_reconcile_total{outcome="error"}` on failure). |
| Validation | Adopt all `conformance.rs` and `runtime.rs` tests unchanged. Add v3 `SessionIdentity` zone-name gate. Add conformance-failure → `d2b_provider_reconcile_total` metric integration test. |

### ADR046-reuse-005 — Provider agent process and gateway-runtime audit bridge

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reuse-005` |
| Dependency/owner | ADR046-reuse-004 + ADR046-audit-003; Provider agent / observability-otel owner |
| Main commit source | `packages/d2b-gateway-runtime/src/provider_agent.rs` at `a1cc0b2d`. Types: `ProviderAgentProcess` (`from_registry`, `from_registry_with`, `provider_type`, `service_names`, `audit_snapshot`), `ProviderAgentError` (closed-set: `SessionClosed`, `ProtocolViolation`, `InvalidAuditCapacity`, `RegistryNotAccepting`, `UnregisteredAdapter`, `RegistrationRejected`), `ProviderAgentAuditOutcome`, `ProviderAgentAuditEvent`. Tests: `tests/provider_agent_v2.rs` (full `ComponentSessionDriver` mock implementing all 20 async driver methods: `start_ttrpc`/`complete_ttrpc`/`cancel`/`send_ttrpc`/`receive_ttrpc`/`register_inbound_call`/`complete_inbound_call`/`remove_inbound_call`/`send_attachments`/`receive_attachments`/`open_named_stream`/`send_named_stream`/`receive_named_stream`/`grant_named_stream_credit`/`close_named_stream`/`reset_named_stream`/`drive_keepalive`/`receive_control`/`close`). |
| Selected behavior | `ProviderAgentProcess::from_registry` constructs a session-bound provider agent from a `ProviderRegistry`; bounded audit ring (`audit_snapshot`); `ProviderAgentError` closed-set for session-level fail paths; full `ComponentSessionDriver` mock trait for hermetic tests. |
| Excluded ADR45 assumptions | `ProviderAgentProcess` in main is wired to the ADR45 gateway-runtime which uses `GatewayAuditKind` / `AuditEnvelope` (realm-scoped). In v3, the provider-agent audit bridge uses `d2b-audit` `SessionConnect`/`RouteAdmission` record classes with `zone` field. Gateway-runtime `AuditEnvelope.realm: RealmPath` is excluded. `bin/d2b-provider-agent.rs` launch path is ADR45-specific; v3 provider agent is launched via `Provider/system-core` or the owning Provider supervisor. |
| v3 Destination | `packages/d2b-provider-observability-otel/src/agent.rs` (adapted); the `ComponentSessionDriver` mock from `tests/provider_agent_v2.rs` becomes the reusable test fixture for all v3 Provider session tests. |
| Integration | `observability-otel` Provider embeds a `ProviderAgentProcess`; session connect/disconnect emits `SessionConnect` audit records via `d2b-audit`; `ProviderAgentAuditEvent` ring feeds `d2b_provider_reconcile_total` metric on session error. |
| Validation | Adopt `provider_agent_v2.rs` mock harness unchanged as shared v3 Provider session fixture. Add v3 audit-bridge test: provider-agent session → `SessionConnect{transport_class="zone_link"}` record emitted. |

### ADR046-reuse-006 — Realm service v2 routing and remote-node routing state

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reuse-006` |
| Dependency/owner | ADR046-reuse-001 + ADR046-bus-001; bus routing owner |
| Main commit source | `packages/d2b-realm-router/` at `a1cc0b2d`. Key files: `src/service_v2.rs` (`RealmServiceServer`, `RealmSessionAuthority`, `RealmAuditEvent`, `RealmServiceProcess`, `RealmServiceLimits`, `CredentialCustody`, `REALM_SERVICE_NAME = "d2b.realm.v2.RealmService"`, `DEFAULT_MAX_REALM_BINDINGS = 256`, `DEFAULT_MAX_SHORTCUTS = 256`, `DEFAULT_MAX_MUTATION_RECORDS = 1024`, `DEFAULT_AUDIT_CAPACITY = 1024`), `src/remote_node.rs` (`RemoteNodeAvailability`, `RemoteNodeErrorKind` with stable `code()` method, `RemoteNodeError`), `src/session_lifecycle.rs`, `src/target_resolver.rs`, `src/execution.rs`. Tests: `tests/realm_service_v2.rs` (`authority_keeps_remote_credentials_in_gateway_guests`, `authenticated_bootstrap_enrollment_route_and_shortcut_lifecycle`). |
| Selected behavior | `RealmServiceServer` ttrpc handler table and per-session authority; `RealmSessionAuthority::local_controller`/`gateway_peer`/`new` for local/ZoneLink/remote purposes; `CredentialCustody::{Host,Gateway}` — host-local sessions retain only public pins; `RemoteNodeErrorKind::code()` stable low-cardinality code method (safe for audit/error labels); bounds `DEFAULT_MAX_REALM_BINDINGS`/`DEFAULT_MAX_SHORTCUTS`. |
| Excluded ADR45 assumptions | `RealmSessionAuthority` carries `realm: RealmId` — in v3, the routing authority carries a Zone name string and resource UID. `REALM_SERVICE_NAME = "d2b.realm.v2.RealmService"` is the ADR45 wire name — v3 replaces with the v3 bus service package name from the closed catalog. `RealmId::parse(request.stream_id)` in route dispatch — v3 uses opaque resource UID. `PurposeClass::Enrolled` maps to gateway-backed enrollment — v3 ZoneLink replaces "gateway". |
| v3 Destination | `packages/d2b-bus/src/routing.rs` (adapted from `service_v2.rs`); `RemoteNodeErrorKind` → v3 `BusErrorKind` with same `code()` stable-label pattern; bounds adopted verbatim. |
| Integration | `d2b-bus` route handler adapts `RealmServiceServer` dispatch table; `RemoteNodeErrorKind::code()` values feed `d2b_bus_route_total{outcome}` metric labels; `CredentialCustody::Host` maps to `purpose_class=local` in `d2b_session_connect_total`. |
| Validation | Adopt `authority_keeps_remote_credentials_in_gateway_guests` test renamed to `authority_keeps_remote_credentials_in_zone_link_sessions`; adapt `RealmId` → Zone name; adopt `authenticated_bootstrap_enrollment_route_and_shortcut_lifecycle` renamed with zone terminology. |

### ADR046-reuse-007 — d2bd service routing, provider effects, and daemon session tests

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reuse-007` |
| Dependency/owner | ADR046-reuse-004 + ADR046-reuse-006; core-controller routing owner |
| Main commit source | `packages/d2bd/` at `a1cc0b2d`. Key files: `src/provider_registry.rs` (full `ProviderCompositionError` mapping, factory composition, `DaemonEffectAdapters` wiring), `src/provider_effects.rs` (`ProviderLifecycleDispatch`, `DaemonEffectAdapterError`, effect adapter structs per domain: `DeviceEffectAdapter`, `AudioEffectAdapter`, etc.), `src/control_services/provider.rs` (`owns`: `service.package == "d2b.provider.v2"` route gate), `src/control_services/daemon.rs` (daemon service route gate), `src/realm_child_supervisor.rs` (realm supervisor with pidfd adoption), `src/realm_stubs.rs`. Tests: `tests/daemon_service_v2.rs` (`every_generated_daemon_method_has_one_typed_adapter`, `local_daemon_policy_is_fixed_and_has_no_negotiation_or_fd_surface`, `public_daemon_handshake_rejects_daemon_or_guest_proxy_schema_mismatch`, `daemon_uses_shared_bootstrap_and_enrolled_guest_credential_bindings`, `shared_guest_session_credential_rejects_zero_authority`, `daemon_guest_paths_do_not_call_broker_signing_or_define_a_private_codec`), `tests/realm_child_supervisor_v2.rs`, `tests/realm_service_v2.rs`. |
| Selected behavior | `service.package == "d2b.provider.v2"` route gate pattern (closed-set package matching without reflection); `ProviderLifecycleDispatch` effect-adapter composition; `DaemonEffectAdapterError` closed-set; `local_daemon_policy_is_fixed` test invariant (no negotiation surface, no fd surface on local policy); bootstrap/enrolled credential binding shape. |
| Excluded ADR45 assumptions | `DaemonEffectAdapters` is daemon-embedded effect composition — v3 effect adapters are per-Provider-process, not daemon-embedded. `control_services/provider.rs::owns` routes to the ADR45 `d2bd` daemon service; v3 routes through `d2b-bus`. `RealmPath as ProviderRealmPath`/`WorkloadId` imports are ADR45 realm concepts — excluded from v3 bus routing. `realm_child_supervisor.rs` uses `RealmId`/`WorkloadId` to supervise child realm processes — v3 replaces with Zone resource UID supervision. `realm_stubs.rs` stubs out ADR45 realm creation — excluded. |
| v3 Destination | `packages/d2b-bus/src/service_router.rs` (adapts route-gate pattern); `packages/d2b-core-controller/src/provider_effects.rs` (adapts `ProviderLifecycleDispatch`); route-gate policy tests adapted from `daemon_service_v2.rs` invariants. |
| Integration | Bus service router uses `service.package` closed-set matching from route-gate pattern; `ProviderLifecycleDispatch` feeds `d2b_provider_component_phase` metric. |
| Validation | Port `local_daemon_policy_is_fixed_and_has_no_negotiation_or_fd_surface` invariant to v3 bus local policy test; port `every_generated_daemon_method_has_one_typed_adapter` to v3 bus method adapter completeness test. |

### ADR046-reuse-008 — ComponentSession v2 vector tests and contract conformance

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reuse-008` |
| Dependency/owner | ADR046-reuse-001; contract-tests owner |
| Main commit source | `packages/d2b-contract-tests/tests/component_session_v2_vectors.rs` at `a1cc0b2d` (`committed_noise_vectors_verify_with_pinned_snow`, `declared_noise_public_key_corruption_is_rejected`, `bootstrap_fixture_mutations_execute_typed_admission_state`, `transcript_and_psk_mutations_are_rejected`). `packages/d2b-contracts/tests/component_session_v2.rs` at `a1cc0b2d`. `packages/d2b-session/tests/noise_vectors.rs` (pinned KAT vectors). |
| Selected behavior | Pinned Noise KAT vectors against the exact `snow` version; transcript+PSK mutation rejection; bootstrap fixture typed-admission state machine; public-key corruption detection. These tests are the ground-truth for session wire security. |
| Excluded ADR45 assumptions | None — these tests have no topology dependency. They test only the cryptographic layer. |
| v3 Destination | `packages/d2b-contract-tests/tests/component_session_v2_vectors.rs` copied verbatim. `tests/noise_vectors.rs` copied verbatim. Neither file requires modification. |
| Integration | These tests run in `make test-rust` / `cargo test -p d2b-contract-tests` and `cargo test -p d2b-session`. They are gating for any Noise library update. |
| Validation | These tests are self-validating. Add one gate: assert `COMPONENT_SESSION_MAJOR = 2` and `COMPONENT_SESSION_MINOR = 0` constants are unchanged in v3 contract. |

### ADR046-reuse-009 — Session MetricsSink → d2b-telemetry bridge

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-reuse-009` |
| Dependency/owner | ADR046-reuse-001 + ADR046-telem-001 + ADR046-telem-003; telemetry/session owner |
| Main commit source | `packages/d2b-session/src/metrics.rs` at `a1cc0b2d` (`MetricEvent` enum: `ActiveSessions`, `Handshake`, `ConnectAttempt`, `ReconnectAttempt`, `Close`, `ControlCreditExhaustion`, `QueueDepth`, `QueueCapacity`, `SchedulingDelay`, `RejectedRecord`; `MetricsSink` trait; `NoopMetrics`). `packages/d2b-contracts/src/v2_component_session.rs::MetricLabels` (`transport: TransportClass`, `purpose: EndpointPurpose`, `channel_class: ChannelClass`, `noise: NoiseProfile`, `locality: Locality`). |
| Selected behavior | `MetricsSink` injection interface: `record(event: MetricEvent, labels: MetricLabels, value: u64)`. Every `MetricEvent` variant maps to a v3 metric instrument from the `d2b_session_*` inventory in this spec. `MetricLabels` fields provide the closed-set label values for those instruments. `NoopMetrics` used in hermetic tests. |
| Excluded ADR45 assumptions | `EndpointPurpose` variants in `MetricLabels` carry ADR45 purpose names (`DaemonLocal`, `DaemonRemote`, `RealmPeer`) — these become v3 purpose names in the OTEL label. `MetricLabels.channel_class` and `MetricLabels.locality` are currently unexported in main but map directly to `session_active{transport}` and `session_connect_total{purpose_class}` labels. |
| v3 Destination | `packages/d2b-telemetry/src/session_metrics_sink.rs`: implements `MetricsSink` backed by the OTEL `d2b_session_*` instruments. `MetricEvent::ActiveSessions` → `d2b_session_active` gauge; `MetricEvent::Handshake` → `d2b_session_connect_total` counter; `MetricEvent::ReconnectAttempt` → `d2b_session_reconnect_total`; `MetricEvent::ControlCreditExhaustion` → `d2b_telemetry_drop_total{signal="session"}`. `MetricLabels.noise` → `profile` label using `NoiseProfile::as_str()`. |
| Integration | `serve_ttrpc_services` receives a `Box<dyn MetricsSink>` from `d2b-telemetry`; all session endpoints call through this bridge. |
| Validation | New test `packages/d2b-telemetry/tests/session_sink_bridge.rs`: drive `MetricEvent` variants through the sink; assert OTEL counter/gauge values; assert `MetricLabels` closed-set values map only to allowed label strings (no `DaemonLocal` string in v3 metric output). |

### ADR046-nix-001 — Nix resource authoring shape, schema-driven options, and bundle emission

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-nix-001` |
| Dependency/owner | W0/W1a; Nix integrator (adapts `nixos-modules/options-observability.nix` and `options-realms.nix`) |
| Current source | `nixos-modules/options-observability.nix` (`d2b.observability.host.identityName`, `scrapeJournal`, `otlpIngest.enable`, `signoz.jwtSecretFile`, `signoz.rootPasswordFile`, `signoz.clickhousePasswordFile`, `retention.*`, `sampling.*` — all predecessor options); `nixos-modules/options-realms.nix` (submodule/option-type/assertion pattern); `nixos-modules/components/observability/{host,stack,guest}.nix` (Nix pipeline shape, ACL pattern, `identityName`/`vmName`); `nixos-modules/manifest.nix` (resource-bundle emission pattern); `packages/d2b-contract-tests/tests/policy_observability.rs::loki_native_otel_resource_attributes` (closed allowlist enforcement — adapt as bundle schema policy test) |
| Reuse action | Implement uniform `d2b.zones.<zone>.resources.<name> = { type; spec; }` option with schema-driven `spec.*` generated option types; adapt option submodule pattern from `options-realms.nix`; adapt pipeline shape from `{host,stack,guest}.nix`; emit canonical sorted ResourceSpec JSON from `resources-bundle.nix` |
| Destination | `nixos-modules/resources.nix` (uniform `d2b.zones.<zone>.resources` schema-aware option; `spec.*` option types generated from `ResourceTypeSchema` for each `type`); `nixos-modules/resources-bundle.nix` (ADR-only: sorted integrity-pinned bundle derivation) |
| Detailed design | (1) Implement `d2b.zones.<zone>.resources = lib.mkOption { type = lib.types.attrsOf (schemaAwareResourceSubmodule); }` where the submodule, given `config.type`, loads the registered `ResourceTypeSchema` and generates `spec.*` option types from it. For `type = "Provider"`, `spec.config.*` options are generated from the signed Provider schema for the package identified by `spec.artifactId` (see ADR-046-provider-model-and-packaging). No second bespoke vocabulary; `spec` fields mirror the canonical JSON fields exactly. (2) `resources-bundle.nix` derivation: serialize each resource to canonical sorted JSON (keys alphabetically sorted at every level); sort resources by `(type, name)`; compute generation digest; emit `zone-resources-<zone>.json` as Nix store output. Publication handler sets `metadata.managedBy = "configuration"` and `metadata.configurationGeneration` on activation — these fields are NOT authored in Nix. (3) `status`, UID, generation, revision, and timestamps are absent from Nix authoring; core fills them. |
| Integration | `d2b-core-controller` reads the Nix store path from the activated system closure; secrets never appear in the bundle |
| Data migration | Current `d2b.observability.*` options are retained with compat warnings (same pattern as current `retention.*`/`sampling.*` compat options); the v3 `d2b.zones.<zone>.resources.*` option is the authoritative surface |
| Validation | `eval_rejects_unknown_type`, `eval_rejects_invalid_emitter_ring_size`, `eval_rejects_unknown_provider_settings`, `eval_rejects_inline_secret_in_settings`, `eval_rejects_unresolved_credential_ref`, `eval_rejects_duplicate_resource_name` nix-unit cases; `bundle_is_sorted_canonically`, `bundle_digest_is_deterministic`, `bundle_contains_no_secret_values`, `bundle_schema_validates_against_provider_schema` contract tests |
| Removal proof | `nixos-modules/options-observability.nix` predecessor options retained with compat warnings until `d2b.observability.enable` migration is complete |

### ADR046-nix-002 — Build-time and runtime ResourceTypeSchema validation

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-nix-002` |
| Dependency/owner | ADR046-nix-001 + ADR046-telem-006 + ADR046-store-001; schema/validation owner |
| Current source | `packages/d2b-contract-tests/tests/policy_observability.rs::startup_tracing_avoids_host_path_fields` (forbidden-field pattern enforcement — adapt as bundle forbidden-field gate); `packages/d2b-contract-tests/tests/policy_metrics.rs` (metric inventory policy test pattern); `packages/d2b-priv-broker/src/runtime.rs` (current runtime schema load/verify pattern); `packages/d2b-contracts/src/provider_registry_v2.rs::ProviderBindingV2` (non-exhaustive signed schema contract) |
| Reuse action | Adapt `startup_tracing_avoids_host_path_fields` forbidden-field pattern for bundle schema gate; adapt `ProviderBindingV2` non-exhaustive contract for Provider-specific settings schema fingerprint |
| Destination | `nixos-modules/resources-bundle.nix` (build-time validation step 4 in the `resources-bundle` derivation); `packages/d2b-core-controller/src/configuration.rs` (runtime activation checks) |
| Detailed design | Build-time: (1) For each `Provider` resource, fetch the `resourceTypeSchema` output from the package; validate `settings` JSON against the JSON Schema; fail the build on schema mismatch or unknown fields. (2) Assert no resource spec field contains a bare secret/path/argv (forbidden-field pattern from `startup_tracing_avoids_host_path_fields`). Runtime: (3) Core-controller re-validates Provider package identity (per ADR-046-provider-model-and-packaging) against the installed package; resolves Credential refs; checks conflict/bounds; rejects with closed-enum `generation-rejected` reason on any failure; no partial activation. (4) Provider schema mismatch between the bundle's schema and the installed Provider's live schema → reject, emit `generation-rejected{reason="package-identity-mismatch"}`. |
| Integration | Nix `resources-bundle.nix` derivation gate + core-controller `configuration.rs` activation path |
| Data migration | None |
| Validation | `bundle_schema_validates_against_provider_schema` bundle contract test; `generation_rejected_emits_audit_record` cleanup contract test with a `schema-validation-failed` reason; add a nix-unit case `eval_rejects_unknown_fields_against_signed_schema` that runs the bundle derivation with a schema mismatch and asserts build failure |
| Removal proof | Not applicable; this is new tooling |

### ADR046-nix-003 — Configuration-owned resource cleanup contract implementation

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-nix-003` |
| Dependency/owner | ADR046-nix-001 + ADR046-nix-002 + ADR046-audit-001 + ADR046-store-001; core-controller owner |
| Current source | `packages/d2bd/src/daemon_audit.rs` (hash-chain `ResourceMutation`-like append pattern — adapt for cleanup audit records); `packages/d2b-priv-broker/src/audit.rs` (`AuditWriteClass::{Standard,Unprivileged}` — cleanup audit records use `Standard` durability); `packages/d2b-realm-core/src/audit.rs::AuditChainLink::new` (hash-chain append for cleanup audit records); `nixos-modules/manifest.nix` (prior-generation retention pattern in the current bundle contract) |
| Reuse action | Adapt hash-chain append from `daemon_audit.rs` for `ResourceMutation{trigger="config-cleanup"}` records; adapt prior-generation retention window from `manifest.nix` pattern |
| Destination | `packages/d2b-core-controller/src/{configuration.rs, ownership.rs}` |
| Detailed design | (1) On new generation activation, every stored `managedBy=configuration` resource absent from the new configured set receives `deletionRequestedAt` plus `deletion-pending`; controller/API-managed resources are untouched. (2) Activation returns after durable intent queueing and does not wait for cleanup. (3) The ownership handler drives child-before-parent finalizers. (4) When finalizers clear, one atomic store transaction writes the `Deleted` revision/change event and removes the row and indexes. After commit, the audit subsystem appends `ResourceMutation{event="deleted", trigger="config-cleanup"}` from that revision using a dedup/exactly-once recovery key; audit append is not part of the store transaction. (5) Stall detection sets `cleanup-stalled` without force-removing finalizers. (6) Prior generations use count retention, default 3 and range 1..16, with no TTL. (7) Core sets `managedBy`/`configurationGeneration` in persisted resources; input bundles omit both. |
| Integration | `d2b-core-controller::configuration.rs` (generation activation); `d2b-core-controller::ownership.rs` (cleanup ordering and atomic final deletion); `d2b-audit` sink (cleanup audit records) |
| Data migration | None — the `managedBy`/`configurationGeneration`/`deletionRequestedAt` fields are new; existing resources gain them on first v3 activation |
| Validation | All tests in "Configuration-owned cleanup contract tests" subsection; additionally: `managedby_configuration_set_on_activated_resources`, `controller_created_resources_have_managedby_controller`, `absent_resource_receives_delete_on_new_generation`, `deletion_sets_deletionrequestedat_not_phase`, `final_deletion_is_atomic`, `cleanup_does_not_touch_controller_children`, `pending_cleanup_condition_set_on_zone`, `zone_is_degraded_not_failed_during_cleanup`, `pending_cleanup_cleared_after_deletion_completes`, `prior_generation_retained_count_based`, `rollback_schedules_delete_for_new_generation_resources`, `audit_segments_preserved_on_provider_delete`, `cleanup_stall_condition_set`, `generation_rejected_emits_audit_record` |
| Removal proof | Not applicable; this is new behavior |
