# Execution Manifest Binding

`docs/reference/test-execution-manifest.md` and schema v1 remain authoritative.
ADR 0054 changes dependency resolution and package-policy inputs, not manifest
fields or IDs.

Requirements:

- Build Event Protocol results map to the exact eighteen IDs.
- A surface completes only after every mapped carrier and companion succeeds.
- A surface with several carriers emits one surface verdict owned by its
  coverage-map verdict carrier only after every carrier succeeds. Tests plant
  one success plus one failure and reject early completion or attribution to
  the wrong carrier.
- Prior evidence is invalidated before dispatch.
- Success, failure, and handled interruption publish sorted atomic manifest v1
  evidence. Partial evidence contains only completed leaves and the exact
  failed/interrupted surfaces available at the boundary.
- The underlying `testVerdict` and typed `evidenceStatus` are separate.
  `evidenceStatus` is the closed tagged complete/degraded union in
  `runner-environment.md`; common `sinkKind` and `retentionClass` fields occur
  exactly once outside the union and must match, while both variants require
  only their own structural fields and reject unknown, repeated, or
  opposite-variant fields. A sanitizer, bound, retention, exporter, or
  publication failure preserves the original passed, failed, or interrupted
  test verdict and emits the degraded variant with one closed bounded code.
  Surface completion and qualification reject degraded evidence without
  relabelling the underlying test as failed.
- Execution-manifest v1 is unchanged. The tagged evidence status is an
  executor/publication sidecar and never becomes a v1 field. Success, failure,
  interruption, and degraded-publication tests schema-check the emitted v1
  document against the existing schema.
- Fixture IDs are emitted only by the unchanged fixture path.
- Executor, Cargo workspace, hub, architecture, and policy context are
  migration evidence and are not new v1 fields.
- Per-case results remain in the executor's per-target JUnit document, with
  exact passed, failed, and ignored outcomes. Complete publication is required
  for surface completion.
- One planted result contains every forbidden redaction value in its
  environment, argv, failure text, stdout, and stderr. JUnit, `test.log`,
  execution-manifest output, qualification output, and exporter diagnostics
  contain none of them. Every sink stays within the byte and record bounds in
  `bazel/generated/evidence-sink-policy.json`.
- JUnit, `test.log`, unsealed evidence, and exporter diagnostics use
  `junit-v1`, `test-log-v1`, `evidence-v1`, and
  `exporter-diagnostic-v1`, respectively. Injected-clock expiry and
  count-overflow tests prove enforcement before publication.
- Repository-owned manifest, runner, timeout, cleanup, and process-control
  paths use no shell.
- No-shell evidence includes raw and unique scan-record counts equal to the
  governed-source count and exactly `no-shell-inventory-empty`,
  `no-shell-inventory-missing-entry`, `no-shell-inventory-extra-entry`,
  `no-shell-inventory-unguarded-spawn`,
  `no-shell-inventory-missing-zero-site-record`, and
  `no-shell-inventory-planted-shell`.
- `D2B_RUST_BUDGET` is validated once and propagated as one combined Bazel and
  suite-concurrency bound.

Cargo and Bazel evidence is comparable only at one commit, with the same
fixture mode and a schema-valid manifest. Source inventory, hub containment,
package-policy results, and Nix realization supplement execution evidence;
none substitutes for it.

Qualification-only `bazelRestoreCount`, `bazelSaveCount`,
`bazelPublicationCount`, `sliceDurationsSeconds`, action-network, and
stable-head fields remain outside manifest v1. They are required in the
qualification record and MUST NOT be added to this schema.
