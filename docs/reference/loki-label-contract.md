# Loki label contract for nixling logs

Status: canonical (P3 — `ph3-p3-loki-label-contract`).
Audience: anyone changing the Alloy configs under
[`nixos-modules/components/observability/`](../../nixos-modules/components/observability/)
or the daemon-side OtelHostBridge that forwards into the obs VM.
Gate: [`tests/loki-label-cardinality-eval.sh`](../../tests/loki-label-cardinality-eval.sh).

## Why a label contract

Loki indexes streams by their label set. Every distinct combination of
label values produces a new stream, and stream count grows
**multiplicatively** with per-label cardinality. Unbounded labels
(unit names, paths, request ids, trace ids, hostnames-of-the-week)
silently blow up index size, fragment compaction, and degrade query
latency long before any single label looks "too big" in isolation.

The contract below pins the small, stable set of labels nixling emits
on the journald → Alloy → otelcol.receiver.loki path, and forbids
everything else. Anything operators actually need to slice on at query
time (unit name, systemd identifier, audit rule key, trace id) lives
in the **log line content / structured fields**, not as a label.

## Allowed labels

| Label      | Meaning                                                          | Cardinality budget | Source of values                                           |
| ---------- | ---------------------------------------------------------------- | ------------------ | ---------------------------------------------------------- |
| `vm`       | The VM the log originated in. `"host"` for host-side units.      | ≤ 20               | `nixling.vms.<name>` keys + literal `"host"`.              |
| `env`      | The nixling env (network namespace) the VM belongs to.           | ≤ 5                | `nixling.envs.<name>` keys + literals `"host"`, `"obs"`, and `nixling.observability.env`. |
| `role`     | What the emitter does. Stable enum, see below.                   | ≤ 10               | Static literal in the Alloy config.                        |
| `severity` | Log severity (RFC5424 keyword, lowercased).                      | ≤ 5                | Optional pipeline stage on the log line; not required.     |
| `source`   | Where the line came from on the emitter.                         | ≤ 5                | Static literal in the Alloy config.                        |

### `role` values (closed set)

| Value      | When used                                                                       |
| ---------- | ------------------------------------------------------------------------------- |
| `workload` | Per-VM guest Alloy emitting journald (and audit) for the workload payload.      |
| `host`     | Host-side singletons (`nixling-otel-host-bridge`, `nixling-ch-exporter`, `usbipd-nixling`, per-VM sidecar units like `microvm@<vm>.service`, `swtpm@<vm>.service`). |
| `usbipd`   | Per-env USBIP backend / proxy units on the host (`nixling-sys-<env>-usbipd-*`). |
| `router`   | Auto-declared per-env net VMs (`sys-<env>-net`). Reserved for the future when the net VM gets its own observability guest. |
| `obs`      | Observability stack VM (`sys-obs-stack`). Reserved for stack-internal Alloy.    |

New role values require a CHANGELOG entry and a budget review. The
`role` budget (≤ 10) leaves five slots of headroom.

### `source` values (closed set)

| Value     | When used                                                       |
| --------- | --------------------------------------------------------------- |
| `journal` | Default: log line came from a `loki.source.journal` stage.      |
| `audit`   | Audit subsystem stream (`audisp-syslog`).                       |

### `severity` values (closed set, optional)

`debug`, `info`, `warn`, `error`, `critical`. Currently not emitted by
the framework's Alloy configs; reserved here so that a future pipeline
stage that extracts `PRIORITY` from journald can promote it without
re-negotiating the contract.

## Forbidden labels (non-exhaustive)

The static gate rejects any label key outside the allowlist. Common
mistakes:

| Forbidden label  | Why                                                                                                                                                                              | Where it goes instead                                                |
| ---------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------- |
| `unit`           | systemd unit names are unbounded (per-VM templates, per-env templates, per-busid USBIP units, ad-hoc transient units). Querying by unit is a log-content filter, not a label.    | journald `_SYSTEMD_UNIT` is preserved in the log line as a field.    |
| `host`           | Hostname is a per-machine axis that interacts multiplicatively with `vm`. Within a single-host deployment, `vm="host"` carries the same information.                              | `vm="host"` (host-side emitters) or `vm=<vmname>` (guest emitters).  |
| `job`            | Alloy `loki.source.*` "job" labels duplicate `role`/`source` and add no query value.                                                                                              | `role` + `source`.                                                   |
| `instance`       | Prometheus convention, not Loki. Adds per-process cardinality that Loki cannot meaningfully compact.                                                                              | Prometheus metrics; kept out of Loki.                                |
| `service_name`   | OpenTelemetry resource attribute that should remain a resource attribute on traces/metrics, not a Loki label.                                                                     | OTel resource attribute (preserved end-to-end through otelcol).      |
| `trace_id` / `span_id` | Unbounded by construction (one new value per request). Promoting either to a Loki label is the canonical "blow up the index" mistake.                                       | Parsed as a structured **field** on the log line via a pipeline stage; correlated with Tempo by the `traceID` derived field in Grafana datasource provisioning. |
| Path-like labels (anything containing `/`, or an absolute path) | Filesystem paths are operator-controlled and unbounded; promoting them to labels also leaks deployment topology into the index. | Path stays in the log content.                                       |

## Hard rules enforced by the gate

[`tests/loki-label-cardinality-eval.sh`](../../tests/loki-label-cardinality-eval.sh)
inspects every `loki.source.*` stanza emitted by

- [`nixos-modules/components/observability/host.nix`](../../nixos-modules/components/observability/host.nix),
- [`nixos-modules/components/observability/stack.nix`](../../nixos-modules/components/observability/stack.nix),
- [`nixos-modules/components/observability/guest.nix`](../../nixos-modules/components/observability/guest.nix),

and asserts:

1. Every label key appears in the allowlist `{vm, env, role, severity, source}`.
2. No label value is path-like — no value contains `/`, and no literal
   value begins with `/`.
3. The literal-value count per closed-enum label respects its budget:
   `role ≤ 10`, `severity ≤ 5`, `source ≤ 5`.
4. The dynamic-value labels (`vm`, `env`) only use values from
   `${quote …}` interpolations of `vmName` / `envName` / `cfg.env` /
   `hostName` — never bare strings outside the documented literals
   (`"host"`, `"obs"`).

The `vm ≤ 20` and `env ≤ 5` budgets are **operator-enforced** at site
config time (a host with 21 workload VMs blows the budget regardless
of what nixling does). The gate cannot statically know how many VMs a
consumer will declare, so it instead pins the contract by:

- ensuring `vm` values only ever come from the `nixling.vms.*` attrset
  or the literal `"host"`,
- ensuring `env` values only ever come from the `nixling.envs.*`
  attrset or the literals `"host"` / `"obs"` / `cfg.env`,

so a consumer that wants more than 20 VMs or 5 envs sees a deliberate
budget breach in their Grafana cardinality dashboard, not a silent
contract drift inside nixling itself.

## Relationship to OtelHostBridge

The host-side `nixling-otel-host-bridge` forwards OTLP logs/metrics/
traces from host Alloy into the obs VM's vsock OTLP receiver. It does
not rewrite labels — whatever the upstream `loki.source.*` stanza
produced flows through unchanged. The contract therefore applies
**at the emitter** (every `loki.source.*` stanza in the three files
above), not at the bridge.

If a future component adds a new `loki.source.*` stanza in a different
file, it MUST be added to the gate's file allowlist in
[`tests/loki-label-cardinality-eval.sh`](../../tests/loki-label-cardinality-eval.sh)
in the same commit, and the new labels MUST conform to this contract.

## See also

- [`docs/reference/components-observability.md`](./components-observability.md)
  — option surface for the observability stack.
- `ph3-p3-tracing-contract` (P3 sibling) — trace span attribute hygiene.
- `ph3-p3-prometheus-otlp-shape` (P3 sibling) — metric label cardinality budget.
