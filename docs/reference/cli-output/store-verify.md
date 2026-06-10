# `nixling store verify --json`

**Diataxis category:** reference.

`nixling store verify <vm> --json` emits one JSON object on stdout. The
CLI is a thin daemon client: it never reads `store-view/live` or
`store-view/state` directly.

Schema: [`store-verify.schema.json`](./store-verify.schema.json).

## Shape

| Field | Type | Semantics |
| --- | --- | --- |
| `vm` | string | VM name being verified. |
| `status` | enum | `ok`, `drift`, `unknown`, `repaired`, `failed`, or `not_found`. |
| `checked` | integer | Number of manifest top-level store basenames checked. |
| `drifted` | integer | Number of top-level basenames or readiness markers that drifted. |
| `repaired` | integer | Number of paths repaired. W6 reports `0`; real repair is deferred. |
| `unknown_reason` | string or null | Present for `unknown`: `marker_or_manifest_missing`, `marker_or_manifest_unreadable`, `older_host_generation`, or `generation_identity_unavailable`. |
| `audit_ref` | string or null | Audit reference for the latest verify/repair attempt when available. |
| `remediation` | string or null | Operator remediation for non-`ok` statuses. |

## Exit codes

| Exit | Status | Meaning |
| --- | --- | --- |
| `0` | `ok`, `repaired` | Live pool is clean, or repair completed successfully. |
| `4` | `drift`, `unknown` | Drift was found, or integrity could not be established. |
| `70` | `not_found` | VM is not declared or the caller is not authorized to know it exists. |
| `78` | `failed` | Broker/system error; inspect logs and retry. |

## Example

```json
{
  "vm": "work-aad",
  "status": "unknown",
  "checked": 0,
  "drifted": 0,
  "repaired": 0,
  "unknown_reason": "generation_identity_unavailable",
  "audit_ref": null,
  "remediation": "restore state/current or activate a new generation, then rerun verify"
}
```
