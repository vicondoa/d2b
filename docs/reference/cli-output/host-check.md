# `nixling host check` output

Schema: [`host-check.schema.json`](./host-check.schema.json)

Host check is intentionally **read-only**. The JSON report is shaped
for both humans and automation: a short summary, a machine-readable list
of individual checks, and a runner-parity appendix keyed by VM.

## Fields

| Field | Type | Meaning | Stability |
| --- | --- | --- | --- |
| `summary.pass` | integer | Count of passing checks. | Stable wire contract. |
| `summary.warn` | integer | Count of warning checks. | Stable wire contract. |
| `summary.fail` | integer | Count of failing checks. | Stable wire contract. |
| `checks[].id` | string | Stable check identifier. | Stable wire contract. |
| `checks[].severity` | string enum | `pass`, `warn`, or `fail`. | Stable wire contract. |
| `checks[].required` | boolean | Whether the check contributes to the failure exit code. | Stable wire contract. |
| `checks[].message` | string | Human-readable result summary. | Stable field; text may refine between minors. |
| `checks[].remediation` | string or `null` | Suggested next step. Present and `null` when no remediation is needed. | Stable wire contract. |
| `runnerParity[].vm` | string | VM name. | Stable wire contract. |
| `runnerParity[].declaredRunner` | string | Declared runner store path from `closures.json`. | Stable wire contract. |
| `runnerParity[].runnerParityPath` | string | Path used as the parity oracle. | Stable wire contract. |
| `runnerParity[].runnerParityOk` | boolean | Whether the declared runner matches the parity oracle. | Stable wire contract. |

## Ordering and null handling

- `checks[]` is emitted in the same logical order as the human report.
- `runnerParity[]` is ordered by VM name.
- `remediation` is present and may be `null`; no other documented fields
  are nullable.

## Stability promise

The field names above and the `severity` enum are the stable contract.
Check messages and remediation strings may be clarified over time, but
new semantics belong in new `id` values rather than by changing the
meaning of an existing one.

## Human example

```text
$ nixling host check
PASS
- kernel-version: running kernel 6.8.0 satisfies >= 6.6
- cgroup-v2: /sys/fs/cgroup/cgroup.controllers is present

WARN
- firewalld-coexistence: firewalld is active; coexistence is reported but host rules are not mutated
```

## JSON example

```json
{
  "summary": {
    "pass": 3,
    "warn": 1,
    "fail": 0
  },
  "checks": [
    {
      "id": "kernel-version",
      "severity": "pass",
      "required": true,
      "message": "Kernel 6.8.0 satisfies >= 6.6",
      "remediation": null
    },
    {
      "id": "firewalld-coexistence",
      "severity": "warn",
      "required": false,
      "message": "firewalld is active; keep the host ruleset unchanged",
      "remediation": "Use host prepare for automated firewall reconcile."
    }
  ],
  "runnerParity": [
    {
      "vm": "corp-vm",
      "declaredRunner": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-microvm-cloud-hypervisor-corp-vm",
      "runnerParityPath": "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-microvm-cloud-hypervisor-corp-vm",
      "runnerParityOk": true
    }
  ]
}
```
