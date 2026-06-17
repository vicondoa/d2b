# ADR 0033: Host OTel collector parity and hostname identity

- Status: Accepted
- Date: 2026-06-17
- Related: ADR 0026 (native SigNoz observability backend)

## Context

The native SigNoz observability backend (ADR 0026) runs two edge
collectors that both export OTLP to broker-supervised relay/bridge
processes and into the `sys-obs` VM's per-source vsock ingress:

- Each opted-in workload VM runs a **guest** collector
  (`nixos-modules/components/observability/guest.nix`) with an `otlp`
  ingest receiver (for in-guest apps), an optional `journald` receiver
  (`scrapeJournal`, default on), optional `hostmetrics`
  (`scrapeNodeMetrics`, default on), and `traces` / `logs` / `metrics`
  pipelines.
- The **host** runs an edge collector
  (`nixos-modules/components/observability/host.nix`) that ships only
  `hostmetrics` and a StoreSync-audit `filelog`. It has no `journald`
  receiver, no `otlp` ingest receiver, and no `traces` or general `logs`
  pipeline.

Two gaps follow from this asymmetry:

1. **Host telemetry is under-collected.** Host-local services and the
   host's own systemd journal are not represented in SigNoz, and there
   is no host OTLP endpoint for host-side instrumentation to push
   traces/logs/metrics. Operators see workload VMs in far more detail
   than the host that runs them.

2. **Host identity is a bare literal.** The trusted per-source ingress
   boundary (ADR 0026, "Trust client-supplied resource identity")
   assigns the `host` source `vm.name = "host"`, `vm.env = "host"`,
   `vm.role = "host"`, and upserts `host.name = "host"`
   (`stack.nix` source map, emitted by `observability-vm.nix`
   `obsIngressSources`). The physical machine name only appears as
   `deployment.environment` (`cfg.hostName`). On a multi-host operator
   estate, or simply for clarity, host-origin telemetry cannot be
   distinguished by the actual hostname in `vm.name` / `host.name`.

The identity is intentionally stamped at the ingress boundary, not
trusted from the edge collector, per ADR 0026. Any identity change must
preserve that property.

## Decision

Bring the host edge collector to parity with the guest collector and give
host-origin telemetry the host's real name, while keeping identity
assignment at the trusted ingress boundary.

### Identity

- Introduce `nixling.observability.host.identityName`
  (default `config.networking.hostName`). It is the value stamped as
  `vm.name` and `host.name` for the `host` source.
- The identity is threaded into the `host` ingress source **only in
  `observability-vm.nix` `obsIngressSources`**, which evaluates in the
  host config context and therefore can read the host option. The
  central collector upserts `vm.name` and `host.name` from
  `source.vmName` at the trusted boundary, so this changes both
  attributes.
- The `stack.nix` `defaultIngressSources.host.vmName` fallback **stays
  the literal `"host"`**. It is only reached when `cfg.ingress.sources ==
  {}` (a standalone `stack.nix` import with no host-provided sources);
  the bundled path always sets `ingress.sources` via `observability-vm.nix`
  and thus always overrides it. `stack.nix` must **not** derive the
  fallback from `config.networking.hostName`, because there it is the
  obs-VM's own hostname, not the physical host's. This avoids a second,
  divergent host-identity contract; the fallback is documented as the
  standalone default.
- `vm.role` stays `"host"` so all host-origin telemetry remains
  selectable as a class regardless of machine name. `vm.env` /
  `service.namespace` stay `"host"`. `deployment.environment` stays
  `cfg.hostName` (unchanged).
- The **ingress source key stays `host`**: only the source *value*
  changes, so the ingress unit (`nixling-otel-vsock-in-host`), the vsock
  port (`14317`), and the firewall posture are unchanged.
- The host edge collector's own `resource` / `resource/store_sync_audit`
  labels are also updated to `identityName` for consistency, but remain
  advisory — the ingress boundary re-stamps identity. No identity is
  trusted from the edge.

### Collection parity

Mirror `guest.nix` in `host.nix`, gated so the new surface is opt-in.
Critically, the host's existing `resource` processor upserts
`service.name = "nixling-host-otel-collector"`; reusing it for ingested
app/journal telemetry would clobber the source's own `service.name`. So
the processors are restructured to mirror `guest.nix`:

- an **identity-only** `resource` processor (`vm.name` / `vm.env` /
  `vm.role`, **no `service.name`**) for the otlp + journald + hostmetrics
  pipelines;
- a `resource/self` processor (adds `service.name =
  nixling-host-otel-collector`) used **only** for the collector's own
  self-`prometheus` metrics pipeline;
- the existing `resource/store_sync_audit` processor is retained
  (with its `vm.name` updated to `identityName`).

New gated surface:

- `nixling.observability.host.otlpIngest.enable` (default `false`):
  add an `otlp` ingest receiver bound to a **Unix domain socket only**,
  in a **dedicated ingest subdirectory**
  (`/run/nixling/otel/ingest/host-otlp.sock`), a `traces` pipeline, and
  `otlp` as a receiver on the `metrics` and `logs` pipelines. No loopback
  gRPC TCP listener is added (see Rejected alternatives). Host-side
  instrumentation pushes OTLP over this socket through the same
  host->`sys-obs` bridge as host metrics. Socket access model:
  - The ingest socket lives in its own directory
    (`/run/nixling/otel/ingest/`), separate from `host-egress.sock`,
    because Linux checks `unlink`/`rename` authority on the **parent
    directory**: granting the collector write on the egress directory to
    `bind(2)` would also let it delete or replace `host-egress.sock`. The
    collector gets write only on `/run/nixling/otel/ingest/`;
    `host-egress.sock` stays in a directory the collector cannot write.
  - The unit runs `ProtectSystem = strict`, so `/run` is read-only in its
    namespace; `ReadWritePaths = [ "/run/nixling/otel/ingest" ]` is added
    (only when `otlpIngest.enable`) so `bind(2)` of the socket succeeds.
    ACLs alone do not make the bind work.
  - `ExecStartPre` unlinks any stale `host-otlp.sock` before start, so a
    crash/`SIGKILL` + `Restart=on-failure` does not wedge on a leftover
    pathname-socket inode.
  - The socket defaults to collector-owned `0600` (root + collector
    only). `nixling.observability.host.otlpIngest.clientGroup`
    (str or null, default `null`) optionally group-owns the socket
    `0660` (+ matching ACL) so members of that group can emit, with
    execute-only (`--x`) traversal granted on the parent dirs; this is
    the explicit, opt-in path to broaden access. The change never widens
    permissions on `host-egress.sock` or the shared `/run/nixling/otel`
    directory; only the dedicated `ingest/` subdirectory and the socket
    itself are touched.
- `nixling.observability.host.scrapeJournal` (default `false`):
  add a `journald` receiver (`start_at = "end"`, a `file_storage` read
  cursor, and the same PRIORITY->severity mapping as the guest), and a
  `logs` pipeline. The collector identity gains the `systemd-journal`
  supplementary group and `journalctl` in `PATH`. The `file_storage`
  cursor directory lives under the host unit's own `StateDirectory`
  (`/var/lib/nixling-host-otel-collector/journald`), **not** the guest's
  `/var/lib/otel/journald` path (which would be unwritable under
  `ProtectSystem = strict`).

The existing `hostmetrics`, self-`prometheus`, and StoreSync-audit
`filelog` pipelines are preserved.

### Defaults and trust posture

- Both new receivers default **off**. The host journal is at least as
  sensitive as a guest journal (it can contain auth failures, sudo
  command lines, and service-logged secrets). The same trust boundary as
  the guest path applies — telemetry leaves only over
  `/run/nixling/otel/host-egress.sock` -> broker `OtelHostBridge` ->
  `sys-obs` vsock, never the workload/host LAN — but the conservative
  framework default is opt-in, matching `ch.exporter.includeTopologyLabels`
  and the SigNoz anonymous-viewer posture.
- **`identityName` defaults to the hostname and is *not* gated by the
  receiver flags**, so an observability-enabled host's central-stamped
  `vm.name` / `host.name` change from the literal `"host"` to the
  hostname on upgrade even with both receivers off. This is an
  **intentional, documented label change** (the whole motivation of this
  ADR): the old constant is uninformative and the hostname is a bounded,
  more useful identity. It is called out in Consequences, the CHANGELOG,
  and the how-to migration note. A consumer that wants the old behavior
  can set `host.identityName = "host"`.
- Attribute hygiene: the host journal and host OTLP streams are
  **non-redacting**, exactly like the existing guest journal path (only a
  `severity_parser` runs; unlike the StoreSync-audit export, which is
  pre-redacted by a broker allow-list). Bulk redaction / secret-key and
  `/nix/store`-path scrubbing is **out of scope** here and tracked as
  cross-cutting future work that must apply to the guest journal too;
  conflating it into this host-parity change would create an asymmetric,
  host-only redaction contract. The mitigation for this ADR is
  default-off + a prominent sensitivity warning + the existing vsock-only
  trust boundary.
- Retention of these host-sensitive logs is governed by
  **SigNoz/ClickHouse TTL inside `sys-obs`**, not by
  `nixling.observability.retention.*` (which currently only warns). The
  docs state this explicitly.
- `identityName` is host-config-controlled and assigned at the trusted
  ingress boundary, so it does not weaken ADR 0026's anti-forgery
  property. The host OTLP ingest socket is a *local* boundary: a
  permitted local writer can emit host-scoped telemetry, but the central
  upsert still prevents it from forging another source's
  `vm.name`/`vm.env`/`vm.role`.

### Testing, docs, and landing

- **Testability:** `host.nix` currently keeps its collector config in a
  private `pkgs.writeText` binding, so eval tests cannot inspect it. The
  pre-serialization attrset is exposed via an internal, `visible = false`
  option (e.g. `nixling.observability._internal.hostCollectorConfig`) so
  `tests/unit/nix/eval-cases/observability.nix` can assert receivers /
  pipelines / extensions.
- **Eval cases** cover: defaults-off (no `journald`/`otlp` receiver, no
  `traces`/general `logs` pipeline, no journal group/cursor), each flag
  on, both flags on (no conditional-merge clobber), identity resolution +
  override, and socket-path non-collision
  (`host-otlp.sock` != `host-egress.sock` / guest sockets; no SigNoz
  4317/4318 or vsock 14317 reuse).
- **Contract tests (Rust):**
  `packages/nixling-contract-tests/tests/policy_state.rs::store_sync_export`
  regex-asserts the StoreSync `vm.name` *value* is `"host"`, so it is
  updated to follow `identityName` while `vm.env` / `vm.role` stay
  `"host"`. The resource-attribute **key**-allowlist gate
  `packages/nixling-contract-tests/tests/policy_observability.rs::loki_native_otel_resource_attributes`
  (legacy name — the framework uses native SigNoz/ClickHouse, not Loki;
  the retired `tests/loki-label-cardinality-eval.sh` shell gate is gone)
  checks the *set of keys*, not values, and forbids retired
  Loki/Tempo/Grafana surfaces. This change adds no new resource-attribute
  keys (all of `vm.name`/`vm.env`/`vm.role`/`host.name`/`service.name`/
  `source` are already allow-listed) and no retired surfaces, so that gate
  needs no edit — only re-verification that it stays green.
- **Docs:** `docs/reference/components-observability.md` gains the three
  host option rows, a "Socket and port contract" entry for
  `host-otlp.sock`, and a Secrets/sensitivity note for host journal/OTLP;
  `docs/how-to/enable-observability.md` documents opting in and the
  identity migration; `CHANGELOG.md` records the label change and new
  options; `AGENTS.md` notes the load-bearing behavior (ingress-boundary
  identity stamping, host source key/port invariants, default-off host
  journal/OTLP). This ADR is added to `docs/adr/README.md` (the index
  coverage guard).
- **Status convention:** this ADR is `Proposed` during panel review and
  flipped to `Accepted` in the commit that lands the implementation.

## Consequences

- Host-origin telemetry can be filtered by the real hostname in
  `vm.name` / `host.name`; `vm.role = "host"` still selects the class.
  `vm.name` for the host source changes from a constant to one bounded
  value per machine (cardinality stays low).
- With `scrapeJournal` enabled, host journal logs land in SigNoz tagged
  with the host identity; operators must treat the obs VM as holding
  host-sensitive log data (already true for guest journals).
- With `otlpIngest.enable`, the host exposes a local OTLP boundary for
  host instrumentation; nothing is opened on any LAN.
- New option surface (`nixling.observability.host.*`) becomes part of the
  observability review and documentation surface.
- Consumers that do not set the new receiver flags get **no new
  collection surface** (no host journal, no host OTLP ingest, no extra
  pipelines). However, an observability-enabled host's central-stamped
  `vm.name` / `host.name` **do change** from the literal `"host"` to the
  hostname by default, because `identityName` is not gated by the
  receiver flags. This is the intentional identity migration; a consumer
  that needs the old labels sets `host.identityName = "host"`.

## Rejected alternatives

### Always-on host journal/OTLP (match the guest defaults verbatim)

The guest collector defaults `scrapeJournal` / `scrapeNodeMetrics` on.
Mirroring that for the host would forward host journal contents by
default on every upgrade of an observability-enabled host. Given the
elevated sensitivity of the host journal, the framework default stays
opt-in; consumers that want full parity set the flags explicitly.

### Stamp the hostname only at the edge collector

Setting `vm.name` to the hostname in `host.nix` alone would be
overridden by the trusted ingress boundary's upsert and would invite
trusting edge-supplied identity — the exact pattern ADR 0026 rejects.
Identity is therefore changed at the ingress source.

### A second, consumer-side host collector in `/etc/nixos`

A parallel collector outside the framework cannot reach `sys-obs`
cleanly: the host->obs path is the framework's broker-spawned vsock
bridge, and the obs VM exposes OTLP only on loopback inside the guest
(only the SigNoz UI port is firewalled open). Extending the existing
framework collector reuses the established trusted transport instead of
duplicating it.

### A loopback gRPC TCP OTLP ingest endpoint

An earlier draft exposed the host OTLP ingest receiver as a loopback
gRPC TCP listener (in addition to, or instead of, a Unix socket). It is
rejected in favor of **UDS-only** ingest. A TCP listener — even on
`127.0.0.1` — needs an explicit non-colliding port (the obs surface
already uses `4317`/`4318`/`8888` inside `sys-obs`, `9101` for the host
CH-exporter, and `12345` for the collector's self-metrics) and is
reachable by any local process without filesystem-permission scoping. A
Unix-domain socket under `/run/nixling/otel/` gets ownership/mode/ACL
access control for free (default `0600`, opt-in `clientGroup`), adds no
routable surface, and matches the guest collector's socket-based local
ingress. A loopback TCP endpoint can be reintroduced later behind its
own explicit option if a concrete host workload needs it.
