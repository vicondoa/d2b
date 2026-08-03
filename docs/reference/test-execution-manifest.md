# Test execution manifest

The test execution manifest is an opt-in record of work that an aggregate
validation target actually completed. It is evidence of execution, not a
replacement for source discovery or the enforcing test gate. The binding
schema is
[`schemas/test-execution-manifest-v1.json`](./schemas/test-execution-manifest-v1.json).

The first emitter is the Rust target:

```bash
D2B_EXECUTION_MANIFEST=.scratch/test-rust-executed.json make test-rust
```

The manifest is written only when `D2B_EXECUTION_MANIFEST` is set. The public
Rust target removes the prior requested evidence before dispatching its
recursive Make DAG. A normal, failed, or handled-interruption run attempts to
publish a complete current record atomically. If final publication or cleanup
fails, the emitter returns a manifest error after scheduler success and
preserves the scheduler's nonzero status otherwise. An uncatchable termination
may leave no new record, but it cannot leave the prior successful record in
place.

Each completed Rust sub-surface publishes its own deterministic fragment. A
fragment publication error after a successful sub-surface fails that leaf with
a static retry diagnostic; when the test surface already failed, recording its
failed fragment is best effort and the original test status is preserved.

## Schema version

The binding schema version is **1**. The top-level `version` field is the
integer `1`, and the prose and JSON schema must agree on that value. The
schema rejects unknown top-level fields so a producer and consumer cannot
silently drift.

## Fields

| Field | Meaning |
| --- | --- |
| `version` | Binding schema version, currently `1`. |
| `target` | Stable public target that produced the record: `test-rust`, `test-nix-unit`, or `test-flake`. |
| `commit` | Git commit used by the aggregate invocation. |
| `run_status` | `passed`, `failed`, or `interrupted`. |
| `completed_leaves` | Sorted leaf identifiers whose required command completed successfully. |
| `failed_surfaces` | Sorted leaf or scheduler identifiers that failed before finalization. |
| `installables` | Sorted Nix installables submitted by the target. Rust currently emits an empty list. |
| `realized_checks` | Sorted flake checks actually realized by the target. Rust currently emits an empty list. |
| `source_inventory_digest` | SHA-256 digest of the matching source inventory, or an empty value when the target has no digest. |
| `external_contention` | One closed contention code: `not-measured`, `none`, `nix-daemon-shared`, or `host-busy`. |

Arrays are unique and sorted. The resulting JSON is canonicalized with stable
keys and a final newline, so equal executions produce equal evidence.

## Secure lifecycle

The emitter anchors the manifest parent before opening any evidence file.
Linux `openat2` resolution uses `RESOLVE_NO_SYMLINKS` and
`RESOLVE_NO_MAGICLINKS`; the portable fallback walks each component through
`openat` with `O_NOFOLLOW` and rejects symlink, magic-link, and non-directory
components. Evidence descriptors use `O_CLOEXEC`.

The persistent `<manifest>.lock` is a separate current-user-owned regular file
with mode `0600`. It is opened relative to the anchored parent with
`O_CLOEXEC` and `O_NOFOLLOW`, then locked with nonblocking
`F_OFD_SETLK`. Contention emits the fixed telemetry code
`manifest-lock-contended`. The diagnostic is intentionally path-free:

> execution-manifest lock is active; wait for the active run to finish and
> retry.

Wait for the active invocation to finish, then retry the command. Do not
delete the lock file.

Run fragments live in a current-user-owned mode `0700` directory adjacent to
the requested manifest. The directory is checked to be on the same filesystem
as its parent. A leaf writes a private mode `0600` temporary fragment,
flushes it, and atomically renames it to its final fragment name. The
finalizer reads only complete renamed fragments, writes the complete manifest
to a temporary file, and atomically renames that file into the anchored
parent.

Stale cleanup is anchored and fd-relative. Each candidate is opened with
`O_CLOEXEC` and `O_NOFOLLOW`, checked with `fstat` for type, current effective
uid, and mode, and removed with `unlinkat`. Invalid or foreign entries are
skipped or cause a fail-closed cleanup error; cleanup never performs a
stat-then-path-unlink sequence or a path-based recursive removal. The
persistent lock file is never removed as part of replacement.

## Shutdown

The scheduler runs in a dedicated process group. A handled `INT` or `TERM` is
forwarded to that group. The production grace period is fixed at 10 seconds.
After the grace period, surviving processes receive `SIGKILL` and are reaped.
Finalization is idempotent and preserves the scheduler's original exit status.
Handled interruption publishes `run_status` `interrupted` and whatever
completed leaf evidence was available at the boundary. A failed scheduler
publishes `run_status` `failed`; only a zero-status scheduler with no failed
surface publishes `passed`. A finalization error is itself a nonzero manifest
failure when the scheduler had returned zero; an earlier scheduler failure or
interruption remains the returned status.

The implementation keeps clock, process-control, and path-resolution
boundaries injectable internally for hermetic lifecycle tests. There is no
public shutdown-grace environment variable or timing knob.

## Coverage use

Static discovery remains required. Compare `completed_leaves` with the
baseline execution manifest and compare the source inventory separately. A
failed or interrupted record is useful diagnostic evidence but cannot satisfy
coverage acceptance. A passing Rust aggregate preserves this baseline leaf set
exactly when its conditional fixture/CLI surfaces are enabled:

- `rust-api-surface`
- `rust-main-format`
- `rust-main-clippy`
- `rust-main-workspace-tests`
- `rust-contract-tests`
- `rust-cli-contract-tests`
- `rust-no-bash-ast`
- `rust-broker-default`
- `rust-broker-layer1`
- `rust-broker-fakebackends`
- `rust-guest-shell-runner`
- `rust-schema-reproducibility`
- `rust-deny-main`
- `rust-deny-broker`
- `rust-deny-guest`
- `rust-audit-main`
- `rust-audit-broker`
- `rust-audit-guest`
- `rust-stub-no-socket`
- `rust-assert-pinned`

The default `make test-rust` and focused `make test-rust-main` include the
fixture-dependent contract and CLI surfaces once when Nix is available.
`D2B_SKIP_FIXTURE_BUILD=1` omits those two surfaces so the local or CI
Layer-1 graph can run the separate enforcing
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` lane without
duplicating them. Fixture and CLI work uses an isolated stable target below
`.scratch/rust-test-cache`, so it can overlap the main workspace without
sharing mutable Cargo state.

That isolated layout is warm-local only. Cold local runs restore shared
workspace targets and retain the split API census cache across `make clean`.
They overlap a bounded prebuild frontier, then run fixture, inventory and
schema as a full-budget chain. CI alone uses the shared API census target
and runs API, main, broker, guest, no-bash, schema, inventory and supply chain
as separate full-budget Make jobs before the stable `test-rust` join.
