# Error envelope guidance

**Diataxis category:** reference.

nixling now has four closely related error/reporting surfaces:

1. the public daemon/operator envelope;
2. the private broker envelope;
3. CLI-local host refusal envelopes;
4. mutating-verb dispatch outcomes.

This guide keeps those shapes aligned so operators get actionable failures and
callers get stable machine-readable contracts.

## Shared rules

Every new error or refusal should follow these rules first:

- keep one stable machine key (`kind`, `code`, or `operation`) that scripts can
  match without scraping prose;
- include a remediation string that tells the operator what to do next;
- point at a durable anchor in [`./error-codes.md`](./error-codes.md)
  whenever the surface exposes a `docsAnchor` / `docs_anchor` field;
- keep public-surface wording free of raw secret material, private key paths,
  and unnecessary filesystem detail;
- use camelCase on JSON field names and kebab-case for stable symbolic kinds.

## Public daemon/operator envelope

`nixling_core::error::Error` serializes as the public operator envelope:

| Field | Meaning |
| --- | --- |
| `kind` | Stable symbolic error kind (for example `broker-unimplemented`). |
| `code` | Stable numeric code used in docs and tests. |
| `message` | Human-readable summary. |
| `remediation` | Concrete next action. |
| `docsAnchor` | Anchor into `docs/reference/error-codes.md`. |
| `owningCommand` | Which CLI/API surface owns the error. |

Use this shape whenever an error crosses the public daemon boundary or is meant
to appear in the documented daemon API.

## CLI-local host refusal envelope

Some failures happen before the CLI can or should call the daemon. Those use the
local `HostErrorEnvelope` shape:

| Field | Meaning |
| --- | --- |
| `kind` | Short symbolic refusal kind. |
| `code` | Stable string code (for example `--apply-or-dry-run-required`). |
| `exitCode` | Final CLI exit status. |
| `whatWasChecked` | Which local precondition the CLI evaluated. |
| `observedState` | The concrete state that caused the refusal. |
| `remediation` | What the operator should do next. |
| `docsAnchor` | Anchor into `docs/reference/error-codes.md`. |

Use this for flag-validation errors, Tier-0 host-surface refusals, and other
pre-daemon checks. Do **not** reuse the broker envelope for purely local CLI
refusals.

## Private broker envelope

Broker failures on `priv.sock` use `BrokerErrorResponse`:

| Field | Meaning |
| --- | --- |
| `kind` | Namespaced broker error kind (for example `Broker.BundleIntentMissing`). |
| `operation` | The broker subsystem or verb involved. |
| `targetWave` | Optional rollout / deferral marker when the broker is intentionally staged. |
| `message` | Internal but still operator-readable failure summary. |
| `action` | Next step for the daemon/operator. |

Use this shape for bundle-resolution failures, live-handler failures, and other
privileged-runtime errors that the daemon may need to re-render for operators.
Because it stays on the private wire, it can be more specific than public
operator prose — but it still must not leak secrets.

## Mutating dispatch surfaces

Mutating public verbs have a second, non-error reporting surface:
`MutatingVerbResponse`.

That envelope is for dispatch **outcomes**, not arbitrary failures:

| Outcome | Meaning |
| --- | --- |
| `dry-run-planned` | The daemon accepted the verb and returned a plan only. |
| `applied` | The daemon accepted and applied the mutation. |
| `broker-error` | The daemon reached the broker, and the broker refused or failed the request. |
| `not-yet-implemented` | The public surface is intentionally staged. |
| `invalid-request` | The verb payload was structurally wrong. |

Guidance:

- prefer `MutatingVerbResponse` when the verb itself succeeded as a dry-run or
  apply decision;
- keep `summary` concise and `remediation` actionable;
- if the broker returns a typed error, preserve it on the `broker-error` path
  instead of flattening it into a fake `applied` or generic string;
- when available, include `brokerErrorKind` (or the legacy
  `brokerKind` / `errorKind` hint) so the CLI can re-render the same
  redacted, user-actionable remediation table without exposing the raw
  private broker payload;
- native-only mode (`NIXLING_NATIVE_ONLY=1`) should surface the real native
  refusal, not silently fall back to bash.

## Mapping to `error-codes.md`

`docs/reference/error-codes.md` is the public index for documented error kinds.
When you add or rename a public-facing error/refusal:

1. add or update the row in `error-codes.md`;
2. make `docsAnchor` / `docs_anchor` match that row exactly;
3. update any CLI/API prose that names the old code;
4. keep tests and JSON examples in lockstep.

If an error is intentionally private to the broker, document it in daemon/broker
reference prose instead of inventing a fake public `docsAnchor`.

## Authoring checklist

Before shipping a new failure shape, check all of these:

- Is the machine key stable?
- Does the operator get a concrete remediation?
- Does the public surface avoid leaking secret-bearing paths or raw command
  output?
- Does the docs anchor exist?
- Is the same failure represented consistently across daemon JSON, CLI human
  output, and broker logs?

## See also

- [`./error-codes.md`](./error-codes.md)
- [`./daemon-api.md`](./daemon-api.md)
- [`./cli-contract.md`](./cli-contract.md)
- [`./privileges.md`](./privileges.md)
