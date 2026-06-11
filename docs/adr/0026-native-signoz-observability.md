# ADR 0026: Native SigNoz observability backend

- Status: Accepted
- Date: 2026-06-10
- Related: ADR 0006 (manifest bundle versioning), ADR 0015 (daemon-only
  clean break), ADR 0018 (microvm.nix removal), ADR 0023 (runner-role
  lifecycle matrix)

## Context

Nixling's current bundled observability path is a composed
Grafana/Prometheus/Loki/Tempo/Alloy stack in an auto-declared
observability VM. Host and workload telemetry flows through Alloy
pipelines and then fans out into Prometheus remote-write, Loki log
ingest, and Tempo trace ingest. That split made the initial dashboards
and retention policy straightforward, but it also introduced several
translation layers for data that is already OpenTelemetry-shaped by the
control plane.

The live design also left the host egress path partially wired. Host
telemetry expected a broker-spawned OTel host bridge and an Alloy-owned
egress socket, but the process graph did not emit the bridge runner and
the runtime ACLs did not consistently permit relay principals to connect
to the observability VM's Cloud Hypervisor vsock socket.

SigNoz is OpenTelemetry-native and stores logs, metrics, traces, and
metadata in ClickHouse through the SigNoz OTel Collector. Its native
Linux release artifacts can run under systemd without Docker, Podman,
Kubernetes, Helm, or compose.

## Decision

Move nixling's bundled observability backend to native SigNoz services in
the auto-declared observability VM.

### Stack shape

The observability VM is renamed from `sys-obs-stack` to `sys-obs`.
Nixling will run native systemd services inside that VM:

- ClickHouse for telemetry storage;
- a ClickHouse coordinator for the single-node SigNoz schema;
- SigNoz server and web UI;
- SigNoz OTel Collector;
- a schema-migration one-shot path ordered on real ClickHouse and
  coordinator readiness.

The implementation must not introduce Docker, Podman, OCI containers,
Kubernetes, Helm, or compose into the bundled observability path.

### Transport and identity

Nixling keeps ownership of the VM telemetry transport:

- host and workload edge collectors export OTLP to broker-supervised
  relay/bridge processes;
- the observability VM exposes only the intended per-source vsock ingress
  endpoints;
- each source gets a distinct obs-VM vsock port and distinct local
  collector receiver endpoint;
- the central collector stamps authoritative `vm.name`, `vm.env`, and
  `vm.role` from Nix/bundle metadata for that source-specific endpoint.

Workloads must not be able to forge another VM's telemetry identity by
setting OpenTelemetry resource attributes.

### Collector pipeline

Edge collectors stay thin: receive local telemetry, normalize resource
attributes, redact sensitive values, apply memory and batch controls,
and export OTLP over the existing Unix/vsock boundary.

The central SigNoz collector owns durable ingestion, ClickHouse
exporters, span metrics, any sampling, and self-observability. Span
metrics are computed from the unsampled trace stream; sampling may only
apply to the trace-storage branch. Edge collectors do not sample.

### Storage and exposure

ClickHouse and coordinator ports bind to loopback only. The SigNoz
collector's OTLP and internal telemetry endpoints bind to Unix sockets
or loopback only. The obs VM firewall opens only the SigNoz UI port by
default.

ClickHouse, coordinator, and SigNoz state live on dedicated persistent
virtio-blk volumes. They must not land on tmpfs, virtiofs, the Nix
store, or a writable store overlay.

Secrets for ClickHouse, SigNoz, and the collector are generated on the
host with root-only permissions and passed through systemd credential or
environment-file mechanisms. Secret literals must not be embedded in
world-readable Nix store config files.

### Manifest contract

The public `vms.json` `_observability` shape changes with this backend:
Grafana and Cloud Hypervisor exporter metadata are replaced by SigNoz UI
and collector-ingress metadata. The existing vsock transport fields stay
part of the contract.

This is a breaking manifest-shape change and therefore requires a
`manifestVersion` bump with a matching Rust DTO/schema update.

## Spec corrections

| Surface | Correction | Rationale |
| --- | --- | --- |
| `vms.json` / `manifestVersion` | The SigNoz migration intentionally bumps the manifest from v3 to v4 and replaces Grafana/CH-exporter observability metadata with SigNoz UI and collector-ingress metadata while preserving vsock transport fields. | Existing committed code is the baseline contract; the old reference shape describes the Grafana/Prometheus/Loki/Tempo backend and must change with the new SigNoz backend. |

## Consequences

- Operators get a SigNoz UI instead of Grafana.
- Historical Prometheus/Loki/Tempo telemetry and Grafana dashboards do
  not automatically migrate into SigNoz/ClickHouse.
- The old `sys-obs-stack` state directory is not deleted automatically.
  Operators must keep it for rollback or retire it intentionally after
  validating the new stack.
- Existing Grafana/Tempo/Loki/Prometheus-specific observability options
  need explicit migration shims or fail-loud remediation.
- The SigNoz package provenance and native service behavior become part
  of the observability review surface.

## Rejected alternatives

### Keep the Grafana/Prometheus/Loki/Tempo stack

Keeping the current backend would avoid the UI and data-continuity
change, but it preserves multiple translation layers and does not align
with nixling's OTel-native control-plane telemetry.

### Use the upstream container deployment

SigNoz documents Docker, Kubernetes, and other container-based install
paths. Nixling's bundled stack instead uses native NixOS services so the
microVM remains declarative and no container runtime becomes part of the
framework substrate.

### Trust client-supplied resource identity

Accepting workload-supplied `vm.name` / `vm.env` / `vm.role` attributes
would let one VM forge another VM's telemetry identity. The source
identity is therefore assigned at a trusted per-source ingress boundary.
