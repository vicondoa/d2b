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
- A publication failure after an existing test failure or handled interruption
  preserves the original status and adds only a bounded publication error.
- Fixture IDs are emitted only by the unchanged fixture path.
- Executor, Cargo workspace, hub, architecture, and policy context are
  migration evidence and are not new v1 fields.
- Per-case results remain in the executor's per-target JUnit document, with
  exact passed, failed, and ignored outcomes. Publication is enforcing.
- One planted result contains every forbidden redaction value in its
  environment, argv, failure text, and raw output. JUnit contains none of
  them; `test.log` retains the raw diagnostic.
- Repository-owned manifest, runner, timeout, cleanup, and process-control
  paths use no shell.
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
