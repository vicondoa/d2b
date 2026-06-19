# `nixling host doctor --read-only` output

Schema: [`host-doctor.schema.json`](./host-doctor.schema.json)

`host doctor` is intentionally **read-only**. The JSON report layers a
structured `checks[]` array on top of the legacy fields
(`broker_ready`, `findings`, `summary`, `exitCode`) so existing parsers
keep working while operators get richer postmortem signal.

## Top-level fields

| Field          | Type               | Meaning                                                                                        | Stability                  |
| -------------- | ------------------ | ---------------------------------------------------------------------------------------------- | -------------------------- |
| `command`      | string             | Always `"host doctor"`.                                                                        | Stable wire contract.      |
| `mode`         | string             | Always `"read-only"`.                                                                          | Stable wire contract.      |
| `broker_ready` | boolean            | Backward-compat alias: `true` iff the `broker-ready` check passed.                             | Stable legacy carryover.   |
| `findings[]`   | array of strings   | Human-readable summary lines for every non-pass check; preserved for legacy log scrapers.      | Stable legacy carryover.   |
| `summary`      | object             | `{pass, warn, fail}` counts across all `checks[]` rows.                                        | Stable wire contract.      |
| `exitCode`     | integer            | `0` clean / `1` any warn / `2` any fail; usage errors still exit `78` with the typed envelope. | Stable wire contract.      |
| `checks[]`     | array of check rows | Structured per-check payload (see below).                                                       | Stable wire contract.      |

## Check rows

Every entry in `checks[]` has `name`, `status` (`pass`/`warn`/`fail`),
`detail` (human prose) and optionally `data` (structured payload).

| `name`                    | What it probes                                                                                            | `status` policy                                                                            | `data` keys                                                  |
| ------------------------- | --------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ | ------------------------------------------------------------ |
| `broker-ready`            | Connects to the broker `SOCK_SEQPACKET` socket (default `/run/nixling/broker.sock`).                       | `pass` on accept; `fail` on connect error (broker is the privileged dispatch path).         | `socket`                                                     |
| `daemon-ready`            | Connects to the public daemon socket (default `/run/nixling/public.sock`).                                 | `pass` on accept; `warn` on connect error (degraded but doctor is best-effort).             | `socket`                                                     |
| `metrics-endpoint`        | Loopback HTTP GET against the Prometheus scrape URL (default `http://127.0.0.1:9101/metrics`).             | `pass` on `200`; `warn` on non-200 / unreachable / non-loopback URL. **v1.2 status:** scrapable endpoint deferred to a later release (see [`daemon-metrics.md`](../daemon-metrics.md)); expected to report `warn` in v1.2. | `url`, `status` (optional)                                   |
| `signoz-ui-endpoint`      | When observability is enabled, reads `_observability.signozUrl` from `vms.json` and probes `/api/v1/health`. | `pass` on `200`; `warn` when manifest/URL/probe is unavailable. Observability remains optional. | `url`, `status` (optional)                                   |
| `otel-host-bridge-runner` | Looks for a `role: "otel-host-bridge"` entry in `pidfd-table.json`.                                        | `pass` when ≥1 entry; `warn` when absent (observability is optional).                       | `count`, `entries[]`                                         |
| `usbipd-runners`          | Counts `role: "usbip"` entries (one per env that owns USB).                                                | `pass` (zero or more); `data.count` + per-runner `vm`/`pid`/`startTimeTicks` snapshot.       | `count`, `entries[]`                                         |
| `kernel-module-matrix`    | Reads `kernel-module-report.json` written by the daemon on startup.                                       | `pass` clean; `warn` if optional missing or the report file is absent; `fail` if required missing. | `requiredMissing[]`, `optionalMissing[]`                     |
| `autostart-status`        | Reads `autostart-report.json` written by the daemon after its autostart pass.                             | `pass` all started; `warn` if any degraded; `fail` if any failed; `warn` if report absent. | `started`, `alreadyRunning`, `degraded`, `failed`, `degradedTotal` |
| `storage-lifecycle-report` | Reads `storage-lifecycle-report.json` written by daemon startup.                                         | `pass` clean; `warn` if report absent/unreadable/unparseable or legacy bundle contracts unavailable; `fail` if current-bundle storage/restart/sync contract checks are degraded. | `schemaVersion`, `storageContractPresent`, `syncContractPresent`, `pathCount`, `restartPolicyCount`, `lockCount`, `issueCount`, `issueKinds`, `issues[]`, `remediation` on non-pass |

The daemon persists these report files under
`$NIXLING_DAEMON_STATE_DIR` (default `/var/lib/nixling/daemon-state`)
during startup. Missing files are treated as "daemon hasn't run / has
been skipped" and surface as warnings rather than failures so the doctor
remains usable on fresh hosts.

## Environment overrides

| Variable                  | Default                                  | Purpose                                                      |
| ------------------------- | ---------------------------------------- | ------------------------------------------------------------ |
| `NIXLING_BROKER_SOCKET`   | `/run/nixling/broker.sock`               | Probe target for `broker-ready`.                             |
| `NIXLING_PUBLIC_SOCKET`   | `/run/nixling/public.sock`               | Probe target for `daemon-ready`.                             |
| `NIXLING_DAEMON_STATE_DIR` | `/var/lib/nixling/daemon-state`         | Directory the daemon writes pidfd/module/autostart/storage-lifecycle reports to. |
| `NIXLING_METRICS_URL`     | `http://127.0.0.1:9101/metrics`          | URL probed by `metrics-endpoint`.                            |

## Exit-code semantics

| Exit | Meaning                                                                                                             |
| ---- | ------------------------------------------------------------------------------------------------------------------- |
| `0`  | Every check passed.                                                                                                 |
| `1`  | At least one check is `warn`, none are `fail`.                                                                      |
| `2`  | At least one check is `fail`.                                                                                       |
| `78` | Usage error (e.g. `--read-only` omitted). Returned via the typed `--read-only-required` envelope; no `checks[]` emitted. |

## JSON example

```json
{
  "command": "host doctor",
  "mode": "read-only",
  "broker_ready": true,
  "findings": [
    "metrics-endpoint: unreachable: http://127.0.0.1:9101/metrics",
    "signoz-ui-endpoint: unreachable: http://10.40.0.10:8080/api/v1/health",
    "autostart-status: 1 VM(s) degraded"
  ],
  "summary": { "pass": 5, "warn": 3, "fail": 0 },
  "exitCode": 1,
  "checks": [
    { "name": "broker-ready",            "status": "pass", "detail": "broker socket accepted connection", "data": { "socket": "/run/nixling/broker.sock" } },
    { "name": "daemon-ready",            "status": "pass", "detail": "daemon public socket accepted connection", "data": { "socket": "/run/nixling/public.sock" } },
    { "name": "metrics-endpoint",        "status": "warn", "detail": "unreachable: http://127.0.0.1:9101/metrics", "data": { "url": "http://127.0.0.1:9101/metrics" } },
    { "name": "signoz-ui-endpoint",      "status": "warn", "detail": "SigNoz health endpoint at http://10.40.0.10:8080/api/v1/health unreachable: connect: timed out", "data": { "url": "http://10.40.0.10:8080/api/v1/health" } },
    { "name": "otel-host-bridge-runner", "status": "pass", "detail": "1 OtelHostBridge runner registered", "data": { "count": 1, "entries": [{ "vm": "obs-net", "pid": 1001, "startTimeTicks": 5 }] } },
    { "name": "usbipd-runners",          "status": "pass", "detail": "2 per-env usbipd runner(s) registered", "data": { "count": 2, "entries": [{ "vm": "corp-net", "pid": 1002, "startTimeTicks": 5 }, { "vm": "work-net", "pid": 1003, "startTimeTicks": 5 }] } },
    { "name": "kernel-module-matrix",    "status": "pass", "detail": "all required kernel modules present", "data": { "requiredMissing": [], "optionalMissing": [] } },
    { "name": "autostart-status",        "status": "warn", "detail": "1 VM(s) degraded", "data": { "started": 1, "alreadyRunning": 0, "degraded": 1, "failed": 0, "degradedTotal": 1 } },
    { "name": "storage-lifecycle-report", "status": "pass", "detail": "storage lifecycle startup contract check clean: paths=12 restartPolicies=4 locks=3", "data": { "schemaVersion": "v2", "storageContractPresent": true, "syncContractPresent": true, "pathCount": 12, "restartPolicyCount": 4, "lockCount": 3, "issueCount": 0, "issueKinds": "", "issues": [] } }
  ]
}
```
