# Daemon metrics (Prometheus scrape)

**Diataxis category:** reference.

> Canonical metric inventory exposed by `nixlingd`.
> Implementation: [`packages/nixlingd/src/metrics.rs`](../../packages/nixlingd/src/metrics.rs).
> Static gate: [`tests/daemon-metrics-eval.sh`](../../tests/daemon-metrics-eval.sh).

> **v1.2 status — scrapable endpoint deferred.** The in-process
> registry described below is wired and exercised by the daemon
> (`broker-fallback` and friends record correctly), but the actual
> scrapable HTTP `/metrics` listener is **deferred to a later release** —
> see [`TODO.md`](../../TODO.md) "scrapable /metrics endpoint for
> nixlingd". An attempt to multiplex HTTP through the public
> `SOCK_SEQPACKET` socket was reverted because Prometheus scrapers
> require `SOCK_STREAM`. A later release will land a dedicated
> `SOCK_STREAM` metrics socket (loopback) per the same
> trust model as the broker. Until then `metrics-endpoint` in
> `nixling host doctor` warns by design, and the URL/port shape
> below documents the *intended* contract — not a currently
> reachable endpoint.

## Endpoint shape

`nixlingd` exposes a **Prometheus text-format scrape endpoint**
(content-type `text/plain; version=0.0.4`) on the daemon's public
socket. The request line is `GET /metrics HTTP/1.1`. The response
body is the registry rendered in
[exposition format v0.0.4](https://prometheus.io/docs/instrumenting/exposition_formats/#text-based-format).

Why scrape and not OTLP push:

- The daemon is long-lived and already owns a listening socket; an
  additional scrape path is zero new sockets and zero new
  capabilities.
- A scrape collector decides cardinality + retention policy
  out-of-band, so the daemon doesn't need a remote-write client
  buffer, retry loop, or backoff state.
- The observability env's local Alloy already runs a
  `prometheus.scrape` component (see
  [`docs/reference/components-observability.md`](./components-observability.md))
  so wiring the scrape side is a config-only change.

OTLP push is intentionally *out of scope* for the daemon process
itself. Operators who need OTLP metrics shipping run the in-stack
Alloy as a `prometheus.scrape` → `otelcol.exporter.otlp` pipeline.

## Metric inventory

Every metric below ships with the `nixling_daemon_` name prefix so
collector relabeling can scope-match the daemon without enumerating
each metric individually. Label cardinality is bounded by the
declared schema; see "Cardinality bounds" below.

### `nixling_daemon_vm_state`

- **Type:** gauge
- **Labels:** `vm`, `state`
- **State values:** `running`, `stopped`, `degraded`
- **Meaning:** Per-VM lifecycle state. Exactly one series per `(vm,
  state)` tuple is set to `1`; the other tuples for the same `vm`
  are `0`. Operators graph `sum by (state) (...)` for an at-a-glance
  fleet view.

### `nixling_daemon_vm_start_duration_seconds`

- **Type:** histogram
- **Labels:** `vm`, `outcome`
- **Outcome values:** `success`, `failure`
- **Buckets (seconds):** `0.5, 1, 2, 5, 10, 20, 30, 60, 120, 300`
- **Meaning:** Wall-clock duration of `nixling vm start <vm>` as
  observed by the daemon's supervisor DAG, from the moment the
  start intent is accepted to the moment the runner is either
  ready or declared failed.

### `nixling_daemon_host_prep_step_duration_seconds`

- **Type:** histogram
- **Labels:** `step`
- **Step values:** one of the host-prepare DAG step IDs documented
  in [`docs/reference/host-prep-dag.md`](./host-prep-dag.md)
  (e.g. `nft`, `route`, `sysctl`, `hosts`, `nm-unmanaged`,
  `usbip-firewall`, `cgroup-delegate`).
- **Buckets (seconds):** `0.01, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 10`
- **Meaning:** Per-step duration of a single host-prepare reconcile
  pass. The label space is closed: only documented step IDs are
  emitted.

### `nixling_daemon_broker_request_total`

- **Type:** counter
- **Labels:** `op`, `outcome`
- **Op values:** every `broker_wire` request name documented in
  [`docs/reference/daemon-api.md`](./daemon-api.md#broker-operations)
  (e.g. `ApplyNftables`, `ApplyRoute`, `ApplySysctl`,
  `UpdateHostsFile`, `OpenPidfd`, `SpawnRunner`, `RunActivation`,
  `RunGc`, `RunHostInstall`, `RunHostKeyTrust`,
  `RunKeysRotate`, `RunMigrate`, `RunRotateKnownHost`,
  `UsbipBind`, `UsbipUnbind`, `UsbipProxyReconcile`,
  `ValidateBundle`, `ExportBrokerAudit`).
- **Outcome values:** `ok`, `denied`, `error`
- **Meaning:** Cumulative count of broker requests issued by the
  daemon, partitioned by the wire op name and the broker's typed
  disposition. `denied` corresponds to the broker's
  `denied-refused` / `denied-unknown` disposition; `error`
  corresponds to `errored`.

### `nixling_daemon_broker_request_duration_seconds`

- **Type:** histogram
- **Labels:** `op`
- **Op values:** same set as `nixling_daemon_broker_request_total`.
- **Buckets (seconds):** `0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5`
- **Meaning:** Round-trip latency of a single broker request
  (send → receive → typed-decode) as measured by the daemon.

### `nixling_daemon_ownership_drift_total`

- **Type:** counter
- **Labels:** `vm`
- **Meaning:** Number of times the daemon's ownership preflight
  detected drift on a per-VM state path (uid/gid/mode mismatch on
  files under `${stateDir}/vms/<vm>/`). A non-zero counter is
  always a remediation signal.

### `nixling_daemon_ssh_host_key_drift_total`

- **Type:** counter
- **Labels:** `vm`
- **Meaning:** Number of times the daemon's SSH host-key preflight
  observed a mismatch between the framework-managed
  `${keysDir}/<vm>_ed25519.pub` and the guest's running host key.
  Increment paths are documented in
  [`docs/reference/ssh-host-key-preflight.md`](./ssh-host-key-preflight.md).

### `nixling_daemon_pidfd_table_size`

- **Type:** gauge
- **Labels:** *(none)*
- **Meaning:** Current number of live pidfd entries the supervisor
  holds for child runners (cloud-hypervisor processes and per-VM
  sidecars). Tracks the supervisor pidfd table documented in the
  Control-plane row of [`AGENTS.md`](../../AGENTS.md).

### `nixling_daemon_uptime_seconds`

- **Type:** gauge
- **Labels:** *(none)*
- **Meaning:** Wall-clock seconds since the daemon process started.
  Resets to zero on every restart; pair with
  `changes(nixling_daemon_uptime_seconds[5m]) > 0` for a restart
  alert.

## Cardinality bounds

| Label | Source | Bound |
| --- | --- | --- |
| `vm` | declared `nixling.vms.<vm>` + auto-declared `sys-*` VMs | one series per declared VM |
| `state` | closed enum | 3 |
| `outcome` (vm start) | closed enum | 2 |
| `step` | closed enum (host-prep DAG step IDs) | bounded by [`host-prep-dag.md`](./host-prep-dag.md) |
| `op` | closed enum (broker wire op names) | bounded by [`daemon-api.md`](./daemon-api.md) |
| `outcome` (broker) | closed enum | 3 |

No label carries free-form text (no error messages, no store paths,
no command output). The
[observability panel's cardinality + PII rules](../../AGENTS.md#default-observability-panel)
apply.

## Scrape configuration example

```alloy
prometheus.scrape "nixlingd" {
  targets = [{
    __address__ = "/run/nixling/public.sock",
    __scheme__  = "http+unix",
    __metrics_path__ = "/metrics",
  }]
  forward_to = [prometheus.remote_write.obs.receiver]
  scrape_interval = "30s"
}
```

The 30-second scrape interval is the recommended default; faster
scrapes (5–10 s) are appropriate during incident investigation but
inflate Prometheus storage proportionally.
