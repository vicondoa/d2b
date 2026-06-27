# Daemon audit-check

Operator reference for the in-daemon replacement for the retired
`d2b-audit-check.{service,timer}` host singleton + timer that
previously validated broker audit log shape on a 24h cadence.

Source of truth: `packages/d2bd/src/audit_check.rs`.

This folds the host singleton into the unprivileged daemon's health
surface.

## What it replaces

| Legacy host singleton                          | Replacement                                             |
| ---------------------------------------------- | ------------------------------------------------------- |
| `d2b-audit-check.service` (oneshot)        | `GET /health/audit-check` on the daemon's HTTP surface  |
| `d2b-audit-check.timer` (`OnCalendar=daily`) | Supervisor event-loop sweep every 5 minutes (default) |

Both legacy units were declared in `nixos-modules/host-audit.nix`
with scheduled-removal markers in their `description` so operators saw
the deprecation in `systemctl status` output. v1.0 removes them
outright.

## What the check asserts

For every JSONL line in scope (everything in the audit directory
whose name matches `broker-YYYY-MM-DD.jsonl`, optionally filtered by
`since` to records emitted after a prior successful check):

1. The line parses as a JSON **object**. Arrays, scalars, or
   non-JSON garbage land as `parse-error` defects.
2. Every required header field is present with the expected JSON
   type. The required set mirrors the broker's
   `OwnedOpAuditRecord` shape:

   | Field                 | Type   |
   | --------------------- | ------ |
   | `ts_ms`               | number |
   | `broker_version`      | string |
   | `bundle_version`      | string |
   | `bundle_hash`         | string |
   | `operation`           | string |
   | `public_operation_id` | string |
   | `peer_uid`            | number |
   | `peer_gid`            | number |
   | `authz_result`        | string |
   | `subject_id`          | string |
   | `scope_id`            | string |
   | `decision`            | string |

3. `decision` is one of `allowed`, `denied-refused`,
   `denied-unknown`, `errored`.
4. `authz_result` is one of `launcher`, `admin`, `deny`.
5. **Orphan rule.** A `decision = "errored"` record MUST carry a
   non-null `error_kind`. A populated `error_kind` MUST NOT appear
   alongside `decision = "allowed"`. (Denied decisions may carry an
   `error_kind` — that's the broker's authz rejection reason and is
   not an orphan.)

The check is read-only and hermetic. It never invokes the broker,
never touches the cgroup tree, and never opens a socket.

## When it runs

* **Per request.** `GET /health/audit-check` runs one sweep
  synchronously and returns the report as JSON.
* **Periodic.** The supervisor event loop runs the same sweep every
  `DEFAULT_SWEEP_INTERVAL_SECS` (5 minutes). Operators can override
  this in a later phase if hosts with very large audit volume need a
  lower cadence; the constant lives in
  `d2bd::audit_check::DEFAULT_SWEEP_INTERVAL_SECS`.

## HTTP contract

```
GET /health/audit-check HTTP/1.1
```

Response: `200 OK`, `Content-Type: application/json`, body is the
`AuditCheckReport`:

```json
{
  "lines_scanned": 1247,
  "lines_ok": 1247,
  "defects": []
}
```

When defects are present the response is still `200 OK` — the sweep
ran to completion, so the request itself succeeded; the report's
`defects` array tells the operator which lines tripped which
assertion. `d2b host doctor` consumes the same JSON and
surfaces non-empty `defects` as a host-doctor finding.

`5xx` is reserved for sweep failure (e.g., audit directory exists
but is unreadable). Body shape:

```json
{ "error": "permission denied (os error 13)" }
```

Other methods return `405 Method Not Allowed`; other paths return
`404 Not Found`.

### Defect payload shape

```json
{
  "line_index": 42,
  "source_file": "broker-2024-01-01.jsonl",
  "problem": { "kind": "missing-field", "field": "bundle_hash" }
}
```

`problem.kind` is one of:

| `kind`                  | Extra fields                                  |
| ----------------------- | --------------------------------------------- |
| `parse-error`           | `message`                                     |
| `missing-field`         | `field`                                       |
| `wrong-field-type`      | `field`, `expected`, `actual`                 |
| `unknown-decision`      | `value`                                       |
| `unknown-authz-result`  | `value`                                       |
| `orphan-record`         | `decision`, `error_kind` (nullable)           |

## Migration notes for operators

* **No more `systemctl start d2b-audit-check.service`.** Use
  `curl --unix-socket … http://localhost/health/audit-check` (or
  `d2b host doctor`, which polls the daemon for you).
* **No more daily timer wait.** The 5-minute sweep catches malformed
  records within minutes instead of within a day.
* **No `d2b audit --strict` reuse.** The retired oneshot invoked
  `d2b audit --strict` to validate broker audit log shape. The
  shape check is now narrower and faster: it does not re-run the
  full security audit, only the broker audit-log invariants. Use
  `d2b audit` directly if you want the broader scan (it's
  daemon-mediated via `ExportBrokerAudit`).

See also: `docs/reference/daemon-api.md` §"Audit",
`docs/reference/daemon-metrics.md`,
`docs/reference/kernel-module-check.md`.

> **Local scope of this check.** The audit check described in this
> document covers only the local broker audit log
> (`/var/lib/d2b/audit/broker-<utc-date>.jsonl`). Any future
> gateway or realm audit (realm access events, provider operation
> records) is separate and resides inside the gateway guest VM.
> Relay or realm identity never enters the local broker audit or
> auth path: `peer_uid` and `authz_result` in the records above
> reflect only the local `SO_PEERCRED`-derived classification.
