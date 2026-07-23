# `d2b.observability.*` / `d2b.vms.<vm>.observability.*`

Reference for the bundled native SigNoz observability component.

For constellation-wide inspection and `d2b op inspect`, see
[`constellation-observability.md`](./constellation-observability.md).

Option sources:

- [`nixos-modules/options-observability.nix`](../../nixos-modules/options-observability.nix)
- [`nixos-modules/options-vms.nix`](../../nixos-modules/options-vms.nix)

Implementation modules:

- [`nixos-modules/observability-vm.nix`](../../nixos-modules/observability-vm.nix)
- [`nixos-modules/components/observability/{host,guest,stack}.nix`](../../nixos-modules/components/observability/)

## Overview

Set `d2b.observability.enable = true` to auto-declare the `obs` env
and the `sys-obs` observability VM. Set
`d2b.vms.<vm>.observability.enable = true` on each workload VM that
should send telemetry.

The bundled backend is native SigNoz:

- ClickHouse stores telemetry.
- ClickHouse Keeper coordinates the single-node ClickHouse cluster used
  by SigNoz's replicated schema.
- SigNoz serves the UI and API.
- SigNoz OTel Collector ingests OTLP and writes logs, metrics, traces,
  and metadata to ClickHouse.

No Docker, Podman, Kubernetes, Helm, or compose deployment is emitted.
The collector runs from d2b's static generated config; SigNoz OpAMP
manager mode is intentionally not enabled so it cannot rewrite the
source-specific receivers.

Each opted-in workload VM runs a guest OpenTelemetry collector that
forwards OTLP (metrics, traces, logs) over the guest→host vsock relay.
The guest collector also follows the VM's systemd journal through the
contrib `journald` receiver (`scrapeJournal`, default on) so guest
service logs land in SigNoz tagged with the VM's `vm.name` / `vm.env`
resource attributes. The journal `PRIORITY` field is mapped to a
readable OTel severity (`INFO`/`WARN`/`ERROR`/…), and a `file_storage`
cursor lets a collector restart resume where it left off rather than
dropping entries written during downtime.

The central SigNoz collector stamps these resource attributes on every
ingested source: `vm.name` (the source's d2b name — the host or the
VM), `host.name` (the same per-source name, i.e. the hostname telemetry
is collected from: `ddbus` for the host, `work-aad` for a VM), `vm.env` /
`service.namespace` (the env), `vm.role` (`host` or `workload`), and
`deployment.environment` — the physical host for host telemetry
(`<hostName>`, e.g. `ddbus`) and `<hostName>-<env>` for workload VMs
(e.g. `ddbus-work`, `ddbus-personal`).

> The systemd journal can contain sensitive data (auth failures,
> command lines, service-logged secrets). Guest journal logs are
> forwarded only over the in-guest → vsock → `sys-obs` path (never the
> workload env LAN) into the operator's own observability VM. Set
> `d2b.vms.<vm>.observability.scrapeJournal = false` to disable
> guest log collection for a VM.
>
> The **host** journal is at least as sensitive and is forwarded the same
> way (host edge collector → `host-egress.sock` → vsock → `sys-obs`, never
> a LAN). Host journal/OTLP collection is **default-off**; opt in with
> `d2b.observability.host.scrapeJournal` /
> `d2b.observability.host.otlpIngest.enable`. Like the guest journal,
> host logs are forwarded non-redacted (only a severity parser runs), so
> only enable them when `sys-obs` is a trusted operator sink.

## Data path

```text
workload VM
  d2b-otel-collector.service
    -> /run/d2b/otel/otlp-egress.sock
    -> d2b-otel-vsock-out.service
    -> host CH-vsock relay
    -> sys-obs d2b-otel-vsock-in-<vm>.service
    -> signoz-otel-collector.service
    -> ClickHouse

host
  d2b-host-otel-collector.service
    receivers: hostmetrics, StoreSync filelog,
               journald (opt-in: host.scrapeJournal),
               otlp UDS (opt-in: host.otlpIngest.enable,
                         /run/d2b/otel/ingest/host-otlp.sock)
    -> /run/d2b/otel/host-egress.sock
    -> broker-spawned OtelHostBridge
    -> sys-obs d2b-otel-vsock-in-host.service
    -> signoz-otel-collector.service
    -> ClickHouse
```

Telemetry uses Unix sockets and vsock. It does not traverse workload env
LAN routing.

## Host-level options

| Option | Type | Default | Meaning |
| --- | --- | --- | --- |
| `d2b.observability.enable` | bool | `false` | Enable the bundled observability stack. |
| `d2b.observability.env` | str | `"obs"` | Auto-declared observability env name. |
| `d2b.observability.vmName` | str | `"sys-obs"` | Auto-declared observability VM name. |
| `d2b.observability.hostName` | str | host `networking.hostName` | Physical host name stamped as the `deployment.environment` resource attribute on all ingested telemetry. |
| `d2b.observability.index` | int | `10` | LAN index for `sys-obs`. |
| `d2b.observability.lanSubnet` | str | `"10.40.0.0/24"` | Observability LAN CIDR. |
| `d2b.observability.uplinkSubnet` | str | `"203.0.113.0/30"` | Host↔obs point-to-point CIDR. |
| `d2b.observability.signoz.listenAddress` | str | derived obs IP | SigNoz UI bind address. |
| `d2b.observability.signoz.listenPort` | port | `8080` | SigNoz UI port. |
| `d2b.observability.signoz.otlpGrpcPort` | port | `4317` | SigNoz collector loopback OTLP gRPC port. |
| `d2b.observability.signoz.otlpHttpPort` | port | `4318` | SigNoz collector loopback OTLP HTTP port. |
| `d2b.observability.signoz.adminEmail` | str | `"admin@d2b.local"` | Root SigNoz admin email. |
| `d2b.observability.signoz.jwtSecretFile` | path or string or null | `null` | Optional host path for the SigNoz JWT/tokenizer secret. |
| `d2b.observability.signoz.rootPasswordFile` | path or string or null | `null` | Optional host path for the SigNoz root password. |
| `d2b.observability.signoz.clickhousePasswordFile` | path or string or null | `null` | Optional host path for the ClickHouse password used by SigNoz services. |
| `d2b.observability.transport.relayPackage` | package | `pkgs.socat` | Socat-compatible relay package for vsock bridges. |
| `d2b.observability.host.identityName` | str | host `networking.hostName` | Identity stamped as `vm.name` / `host.name` for host-origin telemetry, at the trusted ingress boundary. `vm.role` stays `"host"`. Set to `"host"` to keep the pre-0.2.x literal label. |
| `d2b.observability.host.scrapeJournal` | bool | `false` | Tail the **host** systemd journal (journald receiver) and forward it to SigNoz as logs. Default off — see the host-journal sensitivity note in Secrets. |
| `d2b.observability.host.otlpIngest.enable` | bool | `false` | Expose a host-local OTLP ingest endpoint (Unix socket only) so host-side instrumentation can push traces/logs/metrics through the host→`sys-obs` bridge. |
| `d2b.observability.host.otlpIngest.clientGroup` | str or null | `null` | Group granted write access to the host OTLP ingest socket. `null` ⇒ `0600` (collector + root only); set ⇒ `0660` group-owned, members may emit. |

Legacy Grafana/Tempo/Loki/Prometheus-specific options are retired or
kept only as migration shims. Do not use them for new configurations.
`retention.*` and `sampling.*` currently warn when changed and do not
configure SigNoz/ClickHouse TTL.

### Host collector parity and identity (ADR 0033)

The host edge collector (`d2b-host-otel-collector.service`) always
ships hostmetrics and the StoreSync audit log. The `host.*` options bring
it to parity with the per-VM guest collector:

- `host.scrapeJournal` adds a host `journald` receiver (severity-mapped,
  with a restart-resuming `file_storage` cursor), and
- `host.otlpIngest.enable` adds a host-local `otlp` receiver plus a
  `traces` pipeline and `otlp` on the `metrics`/`logs` pipelines.

Host-origin telemetry identity is assigned at the **trusted ingress
boundary** (never trusted from the edge), per ADR 0026. `host.identityName`
(default the hostname) is stamped as `vm.name` and `host.name`;
`vm.role` stays `"host"`.

> **Identity migration:** `host.identityName` defaults to the hostname and
> is **not** gated by the receiver flags. On upgrade, an
> observability-enabled host's `vm.name` / `host.name` change from the
> literal `"host"` to the hostname even with both receivers off. Set
> `d2b.observability.host.identityName = "host"` to keep the old
> labels. The receivers themselves stay default-off, so no new collection
> surface appears unless you opt in.

## Per-VM options

| Option | Type | Default | Meaning |
| --- | --- | --- | --- |
| `d2b.vms.<vm>.observability.enable` | bool | `false` | Enable telemetry collection for this VM. |
| `d2b.vms.<vm>.observability.scrapeJournal` | bool | `true` | Tail the guest systemd journal (journald receiver) and forward it to SigNoz as logs. |
| `d2b.vms.<vm>.observability.scrapeNodeMetrics` | bool | `true` | Enable guest hostmetrics collection. |

## Runtime services

Host:

- `d2b-host-otel-collector.service`
- broker-spawned `RunnerRole::OtelHostBridge` with process role
  `otel-host-bridge`

Workload VM:

- `d2b-otel-collector.service`
- `d2b-otel-vsock-out.service`

`sys-obs`:

- `clickhouse.service`
- `clickhouse-keeper.service`
- `signoz-schema-migrate-sync.service`
- `signoz-schema-migrate-async.service`
- `signoz.service`
- `signoz-otel-collector.service`
- `d2b-otel-vsock-in-host.service`
- `d2b-otel-vsock-in-<vm>.service` for each observed workload

## Host StoreSync observability export

The privileged broker emits a StoreSync-only telemetry export at:

```text
${d2b.site.stateDir}/observability/store-sync/store-sync-<utc-date>.jsonl
```

The host OTel collector follows `store-sync-*.jsonl` with a `filelog`
receiver and forwards those records through the same host→`sys-obs`
OTLP bridge as host metrics. This export is a positive allow-list
projection, not the broker audit log. Host-confidential fields
(`caller_principal`, retained generation lists, host/store paths,
`db.dump`, marker payloads) are redacted by construction in the broker.
The target VM/env stay in JSON content (`target_vm` / `target_env`) and
are not promoted to resource attributes.

The collector identity gets focused read/traverse ACLs on the StoreSync
export directory only. It is not added to the `d2bd` group and gets
no access to the unified broker audit log, privileged daemon socket, or
other broker state. Static gates:

- [`packages/d2b-contract-tests/tests/policy_state.rs`](../../packages/d2b-contract-tests/tests/policy_state.rs) (`store_sync_export`)
- [`packages/d2b-contract-tests/tests/policy_observability.rs`](../../packages/d2b-contract-tests/tests/policy_observability.rs) (`loki_native_otel_resource_attributes` — the SigNoz resource-attribute key-allowlist gate; legacy name, the framework uses native SigNoz/ClickHouse, not Loki)

## USB audit HMAC keys and observability

USB hardware serial HMAC keys are intentionally not distributed to non-root
observability components. The privileged broker owns
`${d2b.site.stateDir}/secrets/usb-audit-serial-hmac/current.key` and the
optional `previous.key`; the host OTel collector is not granted ACLs on that
directory and receives no systemd credentials for those files. The broker
reloads the keyring on each `UsbipBind`, so a key rotation does not require
restarting the collector or exposing a secure IPC key-read path.

Rotation observability is data-only: broker audit/log records contain key IDs,
active-key count, the 30-day grace-window length, and the closed correlation
version, plus HMAC values inside the privileged audit record. Raw key material,
raw serials, bus IDs, sysfs paths, and dynamic metric labels are not emitted.

## Socket and port contract

| Resource | Value |
| --- | --- |
| Obs VM vsock CID | `1000` |
| Workload observability CID | `100 + envIndex * 100 + vm.index` |
| Host obs ingress vsock port | `14317` |
| Workload obs ingress vsock ports | `14318+`, one per observed VM |
| Workload collector loopback gRPC receivers | `14318+`, matching the workload vsock port to avoid SigNoz internal control-plane ports |
| Host collector egress | `/run/d2b/otel/host-egress.sock` |
| Host OTLP ingest (opt-in) | `/run/d2b/otel/ingest/host-otlp.sock` (Unix socket only; dedicated subdir, isolated from the egress socket) |
| Guest local OTLP | `/run/d2b/otel/otlp.sock` with compatibility symlink `/run/d2b/otlp.sock` |
| Guest relay handoff | `/run/d2b/otel/otlp-egress.sock` |
| SigNoz UI | `signoz.listenAddress:signoz.listenPort` |
| SigNoz OTLP gRPC | loopback `signoz.otlpGrpcPort` inside `sys-obs` |
| SigNoz OTLP HTTP | loopback `signoz.otlpHttpPort` inside `sys-obs` |

Only the SigNoz UI port is opened through the obs VM firewall by default.
ClickHouse, ClickHouse Keeper, OTLP, health, pprof, and zpages listeners
stay loopback or Unix-socket scoped.

## Secrets

D2b generates SigNoz and ClickHouse credentials on the host under:

```text
/var/lib/d2b/observability/
```

The host directory is root-owned `0700`; files are root-owned `0444` so
guest-side systemd can read them through the read-only virtiofs secret
share at `/run/d2b-obs-secrets`. Secrets are consumed through
systemd credentials or environment files, not embedded as literals in the
Nix store.

Host journal and host OTLP telemetry (opt-in, see Host-level options) are
**not redacted** — they may carry secret-bearing log lines or span
attributes. Their retention is governed by SigNoz/ClickHouse TTL inside
`sys-obs`, not by `d2b.observability.retention.*` (which currently
only warns).

## Default resources

`sys-obs` defaults are sized for a single-node SigNoz store:

| Resource | Default |
| --- | --- |
| vCPU | `4` |
| RAM | `8192` MiB |
| ClickHouse volume | `32768` MiB |
| ClickHouse Keeper volume | `2048` MiB |
| SigNoz volume | `4096` MiB |
| SigNoz collector volume | `2048` MiB |

## Migration notes

The old auto-declared VM name was `sys-obs-stack`; the new name is
`sys-obs`. If a host used the old default, upgrading creates a new VM
state directory. The old state is preserved for rollback and must be
retired intentionally.

Historical Prometheus/Loki/Tempo data and Grafana dashboard/alert state
do not automatically migrate to SigNoz.
