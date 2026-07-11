# Daemon metrics (Prometheus scrape)

**Diataxis category:** reference.

> Canonical metric inventory exposed by `d2bd`.
> Implementation: [`packages/d2bd/src/metrics.rs`](../../packages/d2bd/src/metrics.rs).
> Static gate: [`tests/daemon-metrics-eval.sh`](../../tests/daemon-metrics-eval.sh).

> **v1.2 status — scrapable endpoint deferred.** The in-process
> registry described below is wired and exercised by the daemon
> (`broker-fallback` and friends record correctly), but the actual
> scrapable HTTP `/metrics` listener is **deferred to a later release** —
> see [`TODO.md`](../../TODO.md) "scrapable /metrics endpoint for
> d2bd". An attempt to multiplex HTTP through the public
> `SOCK_SEQPACKET` socket was reverted because Prometheus scrapers
> require `SOCK_STREAM`. A later release will land a dedicated
> `SOCK_STREAM` metrics socket (loopback) per the same
> trust model as the broker. Until then `metrics-endpoint` in
> `d2b host doctor` warns by design, and the URL/port shape
> below documents the *intended* contract — not a currently
> reachable endpoint.

## Endpoint shape

`d2bd` exposes a **Prometheus text-format scrape endpoint**
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
- The observability pipeline's OTel Collector runs a scrape receiver
  for this endpoint (see
  [`docs/reference/components-observability.md`](./components-observability.md))
  so wiring the scrape side is a config-only change.

OTLP push is intentionally *out of scope* for the daemon process
itself. Operators who need OTLP metrics shipping run an OTel Collector
pipeline that scrapes this endpoint and exports OTLP downstream.

## Metric inventory

Every metric below ships with the `d2b_daemon_` name prefix so
collector relabeling can scope-match the daemon without enumerating
each metric individually. Label cardinality is bounded by the
declared schema; see "Cardinality bounds" below.

### `d2b_daemon_vm_state`

- **Type:** gauge
- **Labels:** `vm`, `state`
- **State values:** `running`, `stopped`, `degraded`
- **Meaning:** Per-VM lifecycle state. Exactly one series per `(vm,
  state)` tuple is set to `1`; the other tuples for the same `vm`
  are `0`. Operators graph `sum by (state) (...)` for an at-a-glance
  fleet view.

### `d2b_daemon_vm_start_duration_seconds`

- **Type:** histogram
- **Labels:** `vm`, `outcome`
- **Outcome values:** `success`, `failure`
- **Buckets (seconds):** `0.5, 1, 2, 5, 10, 20, 30, 60, 120, 300`
- **Meaning:** Wall-clock duration of `d2b vm start <vm>` as
  observed by the daemon's supervisor DAG, from the moment the
  start intent is accepted to the moment the runner is either
  ready or declared failed.

### `d2b_daemon_host_prep_step_duration_seconds`

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

### `d2b_daemon_broker_request_total`

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

### `d2b_daemon_broker_request_duration_seconds`

- **Type:** histogram
- **Labels:** `op`
- **Op values:** same set as `d2b_daemon_broker_request_total`.
- **Buckets (seconds):** `0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5`
- **Meaning:** Round-trip latency of a single broker request
  (send → receive → typed-decode) as measured by the daemon.

### `d2b_daemon_vm_shutdown_total`

- **Type:** counter
- **Labels:** `vm`, `vmm`, `outcome`
- **VMM values:** `cloud_hypervisor`, `qemu_media`, `unknown`
- **Outcome values:** bounded daemon enum such as `clean_guest_shutdown`,
  `clean_vmm_cleanup`, `api_unavailable`, `timeout_exceeded`,
  `force_requested`, `disabled`, and `forced_cleanup`.
- **Meaning:** Cumulative count of VM stop attempts by provider graceful
  shutdown outcome. Labels never include human summaries or provider error
  text.

### `d2b_daemon_vm_shutdown_duration_seconds`

- **Type:** histogram
- **Labels:** `vm`, `vmm`, `outcome`
- **Buckets (seconds):** `0.5, 1, 2, 5, 10, 30, 60, 90, 120, 300, 600`
- **Meaning:** Elapsed provider graceful-shutdown wait time. Explicit
  force and config-disabled paths record near-zero observations with their
  bounded outcomes.

### `d2b_daemon_activation_phase_duration_seconds`

- **Type:** histogram
- **Labels:** `phase`, `mode`, `status`
- **Phase values:** `prepare`, `marker-write`, `guest`, `commit`,
  `metadata-only`
- **Mode values:** `switch`, `boot`, `test`, `rollback`
- **Status values:** bounded daemon outcomes such as `success`, `failure`,
  `indeterminate`, `broker-error`, `protocol-error`, and
  `dispatch-error`
- **Buckets (seconds):** `0.01, 0.05, 0.1, 0.25, 0.5, 1, 2.5, 5, 30,
  120, 600`
- **Meaning:** Wall-clock duration for the daemon-owned VM activation
  orchestration phases. Labels never include activation ids, store paths,
  switch script paths, guest output, or error text.

### `d2b_daemon_vm_degraded`

- **Type:** gauge
- **Labels:** `vm`, `reason`
- **Reason values:** currently `activation_pending`
- **Meaning:** Per-VM degraded-state indicator for bounded daemon reasons
  that should be visible to operators even when lifecycle state remains
  `Running` or `Stopped`. Activation sets this gauge while a host pending
  marker is unresolved and clears it after a successful commit or
  definitive guest activation failure.

### `d2b_daemon_ownership_drift_total`

- **Type:** counter
- **Labels:** `vm`
- **Meaning:** Number of times the daemon's ownership preflight
  detected drift on a per-VM state path (uid/gid/mode mismatch on
  files under `${stateDir}/vms/<vm>/`). A non-zero counter is
  always a remediation signal.

### `d2b_daemon_ssh_host_key_drift_total`

- **Type:** counter
- **Labels:** `vm`
- **Meaning:** Number of times the daemon's SSH host-key preflight
  observed a mismatch between the framework-managed
  `${keysDir}/<vm>_ed25519.pub` and the guest's running host key.
  Increment paths are documented in
  [`docs/reference/ssh-host-key-preflight.md`](./ssh-host-key-preflight.md).

### `d2b_daemon_pidfd_table_size`

- **Type:** gauge
- **Labels:** *(none)*
- **Meaning:** Current number of live pidfd entries the supervisor
  holds for child runners (cloud-hypervisor processes and per-VM
  sidecars). Tracks the supervisor pidfd table documented in the
  Control-plane row of [`AGENTS.md`](../../AGENTS.md).

### `d2b_daemon_uptime_seconds`

- **Type:** gauge
- **Labels:** *(none)*
- **Meaning:** Wall-clock seconds since the daemon process started.
  Resets to zero on every restart; pair with
  `changes(d2b_daemon_uptime_seconds[5m]) > 0` for a restart
  alert.

### `d2b_daemon_guest_control_exec_total`

- **Type:** counter
- **Labels:** `subsystem`, `outcome`, `error_kind`
- **Meaning:** Cumulative count of guest-control exec session/op outcomes by
  subsystem, closed outcome, and bounded error bucket.

### `d2b_daemon_guest_control_shell_total`

- **Type:** counter
- **Labels:** `subsystem`, `outcome`, `error_kind`
- **Meaning:** Cumulative count of guest-control persistent-shell management and
  attached-owner outcomes. Shell names, session ids, terminal session handles, attach ids,
  terminal stream ids, provider/resource ids, provider endpoints,
  provider credentials, process environments, working directories, helper
  diagnostics, and terminal bytes are never metric labels.

  ### `d2b_daemon_workload_availability`

  - **Type:** gauge
  - **Labels:** `provider`, `component`, `state`
  - **Meaning:** Current provider prerequisite posture for the closed components
    `helper`, `scope`, `proxy`, `launcher`, and `shell`. Provider and state are
    closed enums (`not-applicable` is used where a provider has no component);
    workload ids and runtime details are not labels.

  ### `d2b_daemon_workload_lifecycle_total`

  - **Type:** counter
  - **Labels:** `provider`, `operation`, `outcome`
  - **Meaning:** Configured workload lifecycle outcomes. Values are bounded
    provider/operation/outcome enums and never include argv, environment, cwd,
    paths, process ids, unit names, or helper diagnostics.

## Cardinality bounds

| Label | Source | Bound |
| --- | --- | --- |
| `vm` | declared `d2b.vms.<vm>` + auto-declared `sys-*` VMs | one series per declared VM |
| `state` | closed enum | 3 |
| `outcome` (vm start) | closed enum | 2 |
| `step` | closed enum (host-prep DAG step IDs) | bounded by [`host-prep-dag.md`](./host-prep-dag.md) |
| `op` | closed enum (broker wire op names) | bounded by [`daemon-api.md`](./daemon-api.md) |
| `outcome` (broker) | closed enum | 3 |
| `provider` (workload) | closed runtime-provider enum | 4 |
| `component` (workload) | closed prerequisite enum | 5 |
| `operation` (workload) | closed lifecycle-operation enum | 2 |
| `state` (workload) | closed availability enum plus `not-applicable` | 9 |
| `outcome` (workload) | closed lifecycle-result enum | 3 |
| `vmm` | closed VM shutdown runtime enum | 3 |
| `outcome` (VM shutdown) | closed daemon enum | bounded by daemon code |
| `phase` | closed activation orchestration enum | 5 |
| `mode` | closed activation mode enum | 4 |
| `status` | closed activation phase outcome enum | bounded by daemon code |
| `reason` | closed degraded reason enum | bounded by daemon code |
| `subsystem` | closed guest-control subsystem enum | bounded by daemon code |
| `outcome` (guest-control) | closed enum | bounded by daemon code |
| `error_kind` | normalized daemon error bucket | bounded by daemon code |

No label carries free-form text (no error messages, no store paths,
no activation ids, no command output, no shell session names, no
terminal handles, no terminal stream ids, and no provider resource ids). The
[observability panel's cardinality + PII rules](../../AGENTS.md#default-observability-panel)
apply.

## Scrape configuration example

```yaml
receivers:
  prometheus:
    config:
      scrape_configs:
        - job_name: d2bd
          scrape_interval: 30s
          metrics_path: /metrics
          static_configs:
            - targets: ["127.0.0.1:9101"]
```

The 30-second scrape interval is the recommended default; faster
scrapes (5–10 s) are appropriate during incident investigation but
inflate backend storage proportionally.
