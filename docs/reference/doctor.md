# `nixling host doctor` — probe reference

**Diataxis category:** reference.

> Read-only diagnostic probes run by `nixling host doctor --read-only`.
> Implementation: [`packages/nixling/src/doctor.rs`](../../packages/nixling/src/doctor.rs).

Each probe is a passive, read-only check. Exit codes: `0` = all pass,
`1` = at least one warn (no fail), `2` = at least one fail.

---

## Existing probes (v1.1 baseline)

### `broker-ready`

| Field | Value |
|-------|-------|
| Invariant | Broker socket is reachable |
| Source | `connect(AF_UNIX, SOCK_SEQPACKET)` to `NIXLING_BROKER_SOCKET` |
| Pass | Socket connect succeeds |
| Fail | Socket connect fails |

### `daemon-ready`

| Field | Value |
|-------|-------|
| Invariant | Public daemon socket is reachable |
| Source | `connect(AF_UNIX, SOCK_SEQPACKET)` to `NIXLING_PUBLIC_SOCKET` |
| Pass | Socket connect succeeds |
| Warn | Socket connect fails (non-fatal for read-only doctor) |

### `metrics-endpoint`

| Field | Value |
|-------|-------|
| Invariant | Prometheus scrape endpoint returns HTTP 200 |
| Source | `GET /metrics` to `NIXLING_METRICS_URL` (default `http://127.0.0.1:9101/metrics`) |
| Pass | HTTP 200 |
| Warn | Non-200 response or connection failure (scrape endpoint is optional) |

> **v1.2 status.** The scrapable endpoint is **deferred to a later
> release** (see [`daemon-metrics.md`](./daemon-metrics.md) status
> banner and [`TODO.md`](../../TODO.md)). Until that release lands, this check is
> expected to report `warn` — the in-process metric counters are
> wired and correct, but no HTTP listener serves `/metrics` yet.
> Operators can ignore `warn` on this row in v1.2.

### `otel-host-bridge-runner`

| Field | Value |
|-------|-------|
| Invariant | An OtelHostBridge runner is registered in `pidfd-table.json` |
| Source | `<daemon-state-dir>/pidfd-table.json`, role field contains `otel-host-bridge` |
| Pass | ≥ 1 matching entry |
| Warn | Table missing or no matching entry |

### `usbipd-runners`

| Field | Value |
|-------|-------|
| Invariant | Per-env usbipd runners are counted (informational) |
| Source | `<daemon-state-dir>/pidfd-table.json`, role field contains `usbip` |
| Pass | Always (count surfaced as data; zero is acceptable) |
| Warn | Table missing or unreadable |

### `kernel-module-matrix`

| Field | Value |
|-------|-------|
| Invariant | All required kernel modules are present |
| Source | `<daemon-state-dir>/kernel-module-report.json` |
| Pass | `missing_required` is empty |
| Warn | File missing, or optional modules absent |
| Fail | Any required module in `missing_required` |

### `autostart-status`

| Field | Value |
|-------|-------|
| Invariant | No autostart VM is in failed/degraded state |
| Source | `<daemon-state-dir>/autostart-report.json` |
| Pass | No outcomes with kind `failed` or `degraded` |
| Warn | Any outcome with kind `degraded` |
| Fail | Any outcome with kind `failed` |

---

## v1.2 invariant probes

These four probes were added in v1.2 to close visibility gaps in the
runtime health surface.

### `seccomp-bpf-loaded` — D4 visibility

| Field | Value |
|-------|-------|
| Invariant | Every registered runner is running under a seccomp BPF filter (mode 2) |
| Closes | D4 — seccomp BPF compilation from `ioctl_policy.rs` |
| Source data | `/proc/<pid>/status` field `Seccomp:` for each PID in `<daemon-state-dir>/pidfd-table.json` |
| Pass | All live registered runners report `Seccomp: 2` |
| Warn | `pidfd-table.json` is missing, or empty, or all PIDs have exited (nothing to check) |
| Fail | Any live runner reports `Seccomp: 0` (disabled) or `Seccomp: 1` (strict mode, not BPF filter) |

**Seccomp mode values** (`/proc/<pid>/status Seccomp:`):
- `0` — seccomp disabled
- `1` — strict mode (`SECCOMP_MODE_STRICT`)
- `2` — BPF filter mode (`SECCOMP_MODE_FILTER`) — required

**Probe-substitution note**: no substitution required; `/proc/<pid>/status`
is universally available on Linux ≥ 3.8 (the minimum kernel for the
`seccomp(2)` syscall). Stale pidfd-table entries whose `/proc` files are
gone (process already exited) are silently skipped.

---

### `pre-ns-posture` — D5 visibility

| Field | Value |
|-------|-------|
| Invariant | Every D5-scoped runner is inside a broker-pre-established user namespace |
| Closes | D5 — broker-pre-NS extension to swtpm/gpu/audio roles |
| Source data | `/proc/<pid>/status` field `NStgid:` for each D5-scoped PID in `pidfd-table.json` |
| Pass | All live D5-scoped runners have ≥ 2 tab-separated values on the `NStgid:` line (nested user NS) |
| Warn | `pidfd-table.json` missing, or no D5-scoped runners registered, or all PIDs exited |
| Fail | Any live D5-scoped runner has exactly 1 `NStgid` value (process is in the initial user NS) |

**D5-scoped roles for v1.2**: `swtpm` only. gpu (render-node-only) and
audio are conditionally scoped per the single-entry user-NS architectural
constraint described in ADR 0021 §"Future work"; they are **not**
mandatory in this probe for v1.2.

**`NStgid:` semantics**: the kernel populates one value per user namespace
nesting level, innermost last. A process spawned via
`clone3(CLONE_NEWUSER)` will show two values: the TID in the parent NS
and the TID in the new NS (usually `1`). A single value means the process
is in the initial user NS.

**Probe-substitution note**: no substitution required. `NStgid:` has
been present in `/proc/<pid>/status` since Linux 4.1.

---

### `broker-reap-health` — D7 visibility

| Field | Value |
|-------|-------|
| Invariant | No registered runner is in zombie (`Z`) or dead (`X`) process state |
| Closes | D7 — broker pidfd-reap (`waitid(P_PIDFD)` + `ChildReaped` IPC) visibility |
| Source data | `/proc/<pid>/stat` field 3 (state character) for each PID in `pidfd-table.json` |
| Pass | No registered runner in state `Z` or `X` |
| Warn | `pidfd-table.json` missing or unreadable |
| Fail | Any registered runner is in state `Z` (zombie) or `X` (dead, not yet reaped) |

**Process state `Z`** (`defunct`) indicates the child exited but its
parent has not called `waitid()` / `waitpid()`. If the broker's
`waitid(P_PIDFD)` reap loop is functioning correctly, registered runners
will never remain in state `Z` for more than one SIGCHLD delivery
interval.

**Broker replay-buffer depth**: the D7 `ChildReaped` replay-buffer depth
(in-memory ring of up to 256 events, used to handle nixlingd
disconnect/reconnect) is **not yet observable** via a stable CLI command
(`nixling-priv-broker --report-state` is not implemented in v1.2). The
`data.bufferDepth` field in the JSON output is always `null` for v1.2.
When D7 fully lands the IPC mechanism, this field will carry the actual
depth and the probe will add Warn (buffer ≥ 200 of 256) and Fail (buffer
overflow flag set) thresholds.

**Probe-substitution note**: for the v1.2 scope, the zombie-count probe
is the primary signal. If `/proc/<pid>/stat` is unavailable (e.g. running
inside a restricted container), the probe degrades to Warn with a
descriptive message rather than Fail. Stale entries whose `/proc` files
are gone (already reaped) are silently skipped.

---

### `bridge-ipv6-sysctl` — D8 visibility

| Field | Value |
|-------|-------|
| Invariant | Every declared nixling bridge has `net.ipv6.conf.<bridge>.disable_ipv6 = 1` |
| Closes | D8 — bridge IPv6 sysctl boot-time application and persistence guard |
| Source data | `sysctl -n net.ipv6.conf.<bridge>.disable_ipv6` for each bridge discovered from `<daemon-state-dir>/envs.json` (or `/sys/class/net/` fallback) |
| Pass | All discovered bridges return `1` |
| Warn | No bridges discovered (no envs running), or sysctl query errors for some bridges |
| Fail | Any bridge returns `0` (IPv6 is active on that bridge) |

**Bridge discovery order**:
1. Read `<daemon-state-dir>/envs.json`; extract `lanBridge` and
   `uplinkBridge` fields from each env entry. If the file parses
   successfully (even with an empty `envs` list), the sysfs fallback
   is suppressed.
2. If `envs.json` is absent or unparseable: scan `/sys/class/net/` for
   interfaces matching the nixling naming pattern `br-<env>-lan` or
   `br-<env>-up`.

**Why IPv6 must be disabled on nixling bridges**: nixling bridges carry
L2 frames between the host tap and the per-env `net VM`'s uplink. IPv6
link-local autoconfiguration (`fe80::/10`) on the bridge would allow the
host kernel to respond to NDP solicitations destined for VM traffic,
breaking the network isolation model. The sysctl `disable_ipv6 = 1` must
survive `systemctl restart systemd-networkd` — the live-smoke gate (D1
`--full` mode) asserts this.

**Probe-substitution note**: `sysctl(8)` is required on the PATH (present
on all NixOS hosts). If `sysctl` is not found, the probe returns Warn
with a descriptive error rather than Fail.

---

## Environment overrides (test / staging)

| Variable | Default | Purpose |
|----------|---------|---------|
| `NIXLING_BROKER_SOCKET` | `/run/nixling/priv.sock` | Override broker socket path |
| `NIXLING_PUBLIC_SOCKET` | `/run/nixling/public.sock` | Override public socket path |
| `NIXLING_DAEMON_STATE_DIR` | `/var/lib/nixling/daemon-state` | Override daemon state directory |
| `NIXLING_METRICS_URL` | `http://127.0.0.1:9101/metrics` | Override Prometheus scrape URL |
