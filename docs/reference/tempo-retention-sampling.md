# Tempo retention + sampling policy

Status: canonical.
Audience: anyone changing the trace pipeline under
[`nixos-modules/components/observability/`](../../nixos-modules/components/observability/)
or sizing the obs VM's `/var/lib/tempo` disk budget.
Gate: [`tests/tempo-budget-eval.sh`](../../tests/tempo-budget-eval.sh).

## Why a trace budget

Tempo stores every received span on local disk inside the obs VM at
`/var/lib/tempo/blocks`. Block retention is configured globally on the
compactor and (per multitenancy) overridden per tenant. Without an
explicit per-tenant policy, every span — most of which are routine
VM-start / per-VM heartbeat noise — would sit on disk for the same
window as the rare critical events operators actually need at
forensic depth (SpawnRunner failures, BundleTampered, broker
authz denials, etc.). That trades cheap-but-bulky low-signal data
against the disk budget that protects the high-signal data.

This document pins the canonical two-tier retention + sampling
policy and the disk-budget math that backs it. The Nix-side
constants and the Alloy/Tempo wiring live in
[`nixos-modules/components/observability/stack.nix`](../../nixos-modules/components/observability/stack.nix).

## Policy

| Tier         | Tempo tenant       | Sampling                  | Retention                                | Use case                                                |
| ------------ | ------------------ | ------------------------- | ---------------------------------------- | ------------------------------------------------------- |
| **critical** | `nixling-critical` | 100 % (always_sample)     | **30 days** (`retention.tracesCritical`) | Framework-critical span trees: SpawnRunner failures, BundleTampered, broker authz denials, audit-log writer failures, lifecycle DAG aborts. |
| **default**  | `nixling-default`  | 10 % (probabilistic, traceID-deterministic) | **7 days** (`retention.traces`)         | Everything else: routine VM-start trees, per-VM heartbeat, virtiofsd / swtpm lifecycle, store-sync.       |

Span selection is by **OTel span attribute** — pinned by:

- `sampling.criticalAttribute = "kind"` (key)
- `sampling.criticalValue = "critical"` (value)

The ratios and tenant ids are also pinned as first-class Nix
options on `nixling.observability.sampling`:

- `sampling.criticalRatio = 1.0` — every critical span is kept.
- `sampling.defaultRatio = 0.1` — 10 % of non-critical traces kept.
- `sampling.criticalTenant = "nixling-critical"` — `X-Scope-OrgID`
  for the critical pipeline.
- `sampling.defaultTenant = "nixling-default"` — `X-Scope-OrgID`
  for the default pipeline.

A span with `kind="critical"` (set by the emitter at instrumentation
time) is routed to the critical tenant; all other spans land in the
default tenant.

### Critical-span instrumentation contract

Emitters MUST set the attribute on the **root** of the trace they
want pinned. The tail-sampling processor decides per trace (not per
span), so any non-critical descendant is still retained at 30 d if
the trace's sampling decision was `keep`. Conversely, a stray
`kind="critical"` on a leaf span without root-level intent is still
honoured but may produce orphan-looking traces in the critical
tenant — emitters are expected to keep the attribute consistent
across the trace.

Names of critical events (non-exhaustive, indicative):

- `SpawnRunner` — when it returns an error envelope.
- `BundleTampered` — bundle-digest verify failures inside the
  daemon.
- `BrokerAuthzDenied` — broker `denied-refused` or
  `denied-unknown` `authz_result` paths.
- `AuditWriteFailed` — broker / daemon audit-log writer
  open/append failures (the operator's last line of defence).
- `HostPrepareFailed` — `host prepare --apply` returning an
  error path.

The full list is owned by the daemon / broker; this doc records
the *category*, not the per-call ABI.

## Implementation pipeline

Inside `sys-obs-stack`:

```
otelcol.receiver.otlp.ingress.traces
  -> otelcol.processor.tail_sampling.tempo_budget
       policy "critical_keep_all":      attr kind=critical -> always_sample
       policy "default_probabilistic":  probabilistic 10 %
  -> otelcol.connector.routing.tempo_tenant
       attribute kind=critical -> traces_critical
       else                    -> traces_default
  -> otelcol.exporter.otlp.traces_critical
       headers: X-Scope-OrgID = nixling-critical
  -> otelcol.exporter.otlp.traces_default
       headers: X-Scope-OrgID = nixling-default

Tempo:
  multitenancy_enabled            = true
  compactor.block_retention       = 30d   (global ceiling = tracesCritical)
  overrides.defaults.block_retention            = 7d   (default tenant)
  overrides.per_tenant_override_config:
    nixling-critical.block_retention            = 30d
```

The global compactor ceiling is set to the **longer** retention so
the per-tenant override for the critical tenant is actually
honoured — Tempo refuses to keep blocks longer than the global
compactor budget.

Grafana provisions two Tempo datasources:

- `Tempo` (uid `tempo`) — default tenant, the one dashboards link to.
- `Tempo (Critical)` (uid `tempo-critical`) — critical tenant, for
  forensic queries beyond the 7-day default window.

## Per-VM expected trace volume

The numbers below are the framework's own measured baseline on a
single-host deployment of the v0.2.0 obs stack (16-core Ryzen,
NVMe). Treat them as **planning numbers**, not contractual.

| Source                                         | Spans / hr / VM (pre-sampling) | Avg span size | Hourly bytes (pre-sampling) |
| ---------------------------------------------- | ------------------------------:| -------------:| ---------------------------:|
| VM-start tree (`nixling vm start --apply`)     | ~120 spans × 1 boot/hr = 120   | ~1.0 KiB      | ~120 KiB                    |
| Per-VM heartbeat / state-poll                  | ~60                            | ~0.7 KiB      | ~42 KiB                     |
| Virtiofsd / swtpm sidecar lifecycle            | ~20                            | ~0.9 KiB      | ~18 KiB                     |
| Store-sync                                     | ~10                            | ~1.2 KiB      | ~12 KiB                     |
| Broker host-prepare dispatch                   | ~30                            | ~1.5 KiB      | ~45 KiB                     |
| **Total / hr / VM (pre-sampling)**             | **~240**                       |               | **~237 KiB**                |

Applying the 10 % default-tier sampling:

- Default-tier bytes / hr / VM = ~24 KiB (~570 KiB / day / VM).
- Critical-tier bytes / hr / VM = the few-per-day critical events
  only, conservatively budget at 100 KiB / day / VM.

## Cost model — `/var/lib/tempo` disk budget

Per-VM, 7-day default tenant retention:

- 570 KiB / day × 7 d ≈ **4 MiB / VM**.

Per-VM, 30-day critical tenant retention:

- 100 KiB / day × 30 d ≈ **3 MiB / VM**.

**Total per-VM, both tenants, both retention windows: ≈ 7 MiB.**

For a 50-VM deployment (the framework's notional upper bound for a
single-host operator), the steady-state Tempo disk footprint is:

- 50 VMs × 7 MiB ≈ **350 MiB**.

Add Tempo's own WAL + in-flight block overhead (~200 MiB worst case)
and a 2× headroom factor: **~1 GiB** is the operator-sizing budget
for `/var/lib/tempo` on the obs VM.

The default obs-VM volume is sized far above this (the same volume
also carries Prometheus's metric retention and Loki's log chunks); a
single-host operator does NOT need to provision extra disk to fit the
trace policy.

### Sensitivity

If an operator raises `sampling.defaultRatio` to 1.0 (sampling
disabled), the per-VM default-tier daily bytes grow ~10×: ~5.7 MiB /
day / VM, ~40 MiB / VM / 7 d, ~2 GiB across 50 VMs. Still fits
inside the default volume but materially shifts the budget — flag
it as a deliberate operator choice in the host config.

If `retention.traces` is extended from 7 d to 30 d (matching the
critical tier), default-tier bytes grow ~4.3×: ~17 MiB / VM × 50 VMs
≈ 850 MiB. Also fits; the rationale for the 7-d default is *signal
freshness* (operators rarely need >1 week of 10 %-sampled noise),
not disk constraints.

## Changing the policy

Any change to the canonical constants — retention windows, sampling
ratios, tenant names, the `kind` attribute key, or the literal
critical value — MUST update **all four** locations together in a
single commit:

1. `nixos-modules/components/observability/stack.nix` (the
   `retention.*` and `sampling.*` option defaults).
2. `nixos-modules/options-observability.nix` (the host-side mirror).
3. This document (the policy table and any affected numbers in
   "Per-VM expected trace volume" / "Cost model").
4. CHANGELOG.md (under `## Unreleased`).

[`tests/tempo-budget-eval.sh`](../../tests/tempo-budget-eval.sh)
enforces drift across (1) and (3); skipping it is not optional.
