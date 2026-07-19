# Daemon audit-check

Operator reference for the in-daemon replacement for the retired
`d2b-audit-check.{service,timer}` host singleton + timer that
previously validated broker audit log shape on a 24h cadence.

Source of truth: `packages/d2bd/src/audit_check.rs`.

The checker remains an in-process validation primitive. It is not an HTTP
surface on the daemon public socket.

## What it replaces

| Legacy host singleton                          | Replacement                                             |
| ---------------------------------------------- | ------------------------------------------------------- |
| `d2b-audit-check.service` (oneshot) | In-process checker used by daemon health evaluation |
| `d2b-audit-check.timer` (`OnCalendar=daily`) | Supervisor-owned periodic evaluation |

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

## Result contract

One completed sweep produces an `AuditCheckReport`:

```json
{
  "lines_scanned": 1247,
  "lines_ok": 1247,
  "defects": []
}
```

When defects are present, the sweep still completed; the `defects` array tells
the operator which lines tripped which assertion. An unreadable audit directory
is a sweep failure rather than a report with synthetic defects.

`/run/d2b/public.sock` accepts only authenticated `d2b.daemon.v2`
ComponentSessions. It never accepts HTTP. The generated admin-only
`ExportAudit` RPC routes to the broker adapter and returns exported bytes on an
authenticated named stream; it does not weaken the checker or expose raw host
paths.

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
  `d2b host doctor`; do not send HTTP to the daemon socket.
* **No `d2b audit --strict` reuse.** The retired oneshot invoked
  `d2b audit --strict` to validate broker audit log shape. The
  shape check is now narrower and faster: it does not re-run the
  full security audit, only the broker audit-log invariants. Use
  `d2b audit` directly if you want the broader scan (it's
  daemon-mediated through the generated `ExportAudit` operation).

See also: `docs/reference/daemon-api.md` §"Audit",
`docs/reference/daemon-metrics.md`,
`docs/reference/kernel-module-check.md`.

> **Local scope of this check.** The audit check described in this
> document covers only the local broker audit log
> (`/var/lib/d2b/audit/broker-<utc-date>.jsonl`). Realm-controller and
> provider-agent audit (realm access events and provider operation records)
> is separate and belongs to the component that owns those decisions.
> Relay or realm identity never enters the local broker audit or
> auth path: `peer_uid` and `authz_result` in the records above
> reflect only the local `SO_PEERCRED`-derived classification.
