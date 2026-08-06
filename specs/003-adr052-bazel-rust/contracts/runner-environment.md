# Runner Environment and Per-Case Evidence Contract

ADR 0054 changes where product crates resolve, not the runner isolation
contract established by ADR 0052.

## Context selection

- Main and guest carriers run one fresh process per exact libtest case.
- Broker default, layer1, and fake contexts run one process per test binary,
  bounded internal threads, carry exactly `tags = ["exclusive"]`, and never
  overlap each other or any other test.
- Broker and guest targets are native first-party targets with explicit
  configured dependencies and features.
- The external `@product` union cannot select a test topology.
- Walker execution comes from the separate `@walker` hub.

## Child environment

- Derive from the Bazel test environment and forward only declared values.
- Give every case its own directory beneath `TEST_TMPDIR`.
- Resolve each test binary from declared runfiles.
- Use `D2B_RUST_BUDGET` as the only concurrency control.
- Validate the budget once as a positive integer with value-redacted errors,
  propagate the effective value to Bazel jobs, local test jobs, runner
  process-per-case concurrency, and broker libtest threads, and prove the
  combined live process count never exceeds it. Scheduler-only, suite-only,
  invalid-value, and multiplicative-limit mutations are rejected.

## Provider contract

The locator selects Cargo or Bazel mode once. A Bazel miss never falls back to
Cargo.

The shared filesystem boundary:

1. validates a nonempty declared runfiles-relative provider key with no
   absolute or `..` component;
2. opens one provider descriptor with `O_RDONLY|O_CLOEXEC`;
3. resolves with `RESOLVE_NO_MAGICLINKS` only and deliberately without
   `RESOLVE_BENEATH` or `RESOLVE_NO_SYMLINKS`, because a Bazel runfiles leaf
   symlink may escape the anchor;
4. on the forced component-walk fallback, opens each intermediate component
   with `O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC`, permits the declared leaf symlink,
   opens the leaf `O_RDONLY|O_CLOEXEC` without `O_NOFOLLOW`, and then applies
   every handle check;
5. refuses `ENOSYS` from `execveat` and names the kernel requirement;
6. checks regular-file kind, executable mode, freshness, and exact digest;
7. brackets the digest read with matching descriptor metadata;
8. returns an unforgeable verified handle; and
9. executes that same descriptor with `execveat` and `AT_EMPTY_PATH`.

No provider path is returned. No `Command` by path, `fexecve`, or
`/proc/self/fd` fallback is permitted.

`VerifiedExecutable` is an API seal, not only a runtime convention. Its fields
and minting trait remain private to the provider module. It exposes no
unchecked constructor, path conversion, path accessor, `Default`, `From`,
`Into`, `AsRef`, `Clone`, or `Copy` implementation. Compile-fail fixtures
outside the defining module must fail to construct it, recover a path, convert
an unverified descriptor or path into it, duplicate it, or implement the
sealed minting trait. A positive compile fixture proves only the provider can
mint a handle and the execution API consumes that handle.

The multithreaded runner prepares argv, envp, descriptor mappings, and the
fixed exec-error record completely in the parent. Between `fork` and
`execveat`, the child performs only async-signal-safe raw operations needed to
install already-open descriptors, close inherited descriptors, call
`execveat`, write one fixed-size error record to a raw close-on-exec pipe on
failure, and call `_exit`. It performs no allocation, formatting, logging,
locking, environment lookup, path lookup, trait dispatch, or Rust unwinding.
The parent reads the typed error record after child creation and maps it to the
redacted provider diagnostic. Compile and recording-backend mutations reject
parent preparation moved into the child, a non-close-on-exec error pipe, a
library logger or allocator after fork, a variable-length error write, and
return or panic instead of `_exit`.

All absent, non-regular, non-executable, stale, wrong-digest, rebound-path,
short-read, metadata-change, and exec errno cases are injected. The one
host-backed conformance test may exercise kernel `execveat` behavior against a
declared first-party probe. It executes the same verified descriptor twice,
asserts the provider descriptor is absent in each child, and asserts a planted
non-close-on-exec control descriptor is present.

Every auxiliary descriptor is close-on-exec: the runfiles anchor, provider,
freshness-input handles, per-case directory, stdio setup copies not intended
for the child, and exec-error pipe. A behavioral child enumerates its own
descriptor table. One mutation clears `O_CLOEXEC` at each auxiliary-descriptor
position in turn and must make that test fail. Source-marker checks are not
accepted as proof of descriptor inheritance.

Provider tests separately force the `openat2` and component-walk routes. The
fallback cases prove intermediate symlink refusal, declared leaf-symlink
acceptance followed by full identity verification, and identical same-handle
execution. Mutations that add `RESOLVE_BENEATH` or
`RESOLVE_NO_SYMLINKS`, place `O_NOFOLLOW` on the provider leaf, reopen for
digest or exec, use a path/`fexecve`/`/proc` fallback, or fall back after
`ENOSYS` must fail.

Every provider refusal is nonzero, redacted, and actionable:

| Reason | Stable input named | Exact recovery |
| --- | --- | --- |
| Runfiles entry missing in Bazel mode | Declared runfiles-relative provider key | Declare that key as `data` on the named repository-relative target, then rerun the owning `make test-bazel-rust-<slice>` target. |
| Provider is not a regular file | Declared runfiles-relative provider key | Correct the target's `data` declaration, then rerun the owning slice target. |
| Provider is not executable | Declared runfiles-relative provider key and mode | Rebuild the named target, then rerun the owning slice target. |
| Provider is older than its newest declared input | Declared runfiles-relative provider key | Rebuild the named target, then rerun the owning slice target. |
| Provider digest differs from the coverage map | Declared runfiles-relative provider key and coverage-map row | Rebuild the target, regenerate and review the coverage map, then rerun the owning slice target. |
| Handle metadata changed across digest read | Declared runfiles-relative provider key | Rerun the owning slice target; if it repeats, correct the writer that mutates the declared provider. |
| `execveat` returned `ENOSYS` | Stable kernel requirement | Run the owning slice on a supported kernel providing `execveat`; no path fallback is available. |
| Other typed exec errno | Declared runfiles-relative provider key and errno class | Rebuild the named target, then rerun the owning slice target. |

The declared runfiles-relative provider key is repository content and is
permitted in the refusal. The runfiles root, resolved absolute location,
descriptor number, argv, environment value, and child output remain forbidden.
Exact-message tests reject an omitted key, omitted action, omitted rerun,
borrowed remedy, or leaked local value.

Provider resolution is intentionally distinct from evidence and cleanup
resolution. Per-case directories, JUnit parents, execution-manifest parents,
and cleanup subtrees retain
`RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS`, with an
equivalent strict forced walk.

## Per-case result document

Write one JUnit document to `XML_OUTPUT_FILE` after all children are reaped.
Each enumerated case has one explicit `passed`, `failed`, or `ignored` entry.

Permitted content:

- stable case name;
- outcome;
- bounded duration;
- bounded sanitized failure text from a closed diagnostic-code table.

Forbidden content:

- environment values or argv;
- absolute, worktree, runfiles-root, store, or socket paths;
- process or user identifiers;
- opaque handles;
- unit names;
- shell names;
- terminal bytes;
- raw child output.

No sink receives raw child output. The runner sanitizes and bounds the stream
before writing JUnit, Bazel `test.log`, or emitted execution and qualification
evidence. `bazel/generated/evidence-sink-policy.json` is the committed
authority for each sink's maximum bytes, maximum records, closed permitted
fields, truncation code, and retention class. Initial limits are generated
from measured sanitized fixtures and committed with the measurements; a limit
or permitted-field change requires the measured old and new values, an
explicit allowed delta, and review in the same change. Truncation emits only
the stable `D2B-BZLEVIDENCE-TRUNCATED` code and never a prefix or suffix of
forbidden bytes.

The planted fixture places distinct forbidden values in environment, argv,
failure text, stdout, and stderr. It first proves every value reached the
pre-sanitization stream, then proves every value is absent from JUnit,
`test.log`, emitted manifest evidence, emitted qualification evidence, and
all exporter diagnostics. Each sink is also proved at or below its committed
byte and record limit.

Test outcome and evidence publication are separate typed results:

- `testVerdict` is the underlying `passed`, `failed`, `ignored`, or
  `interrupted` result and is never rewritten by an exporter;
- `evidenceStatus` is `complete` or `degraded`;
- a sanitizer, bound, write, rename, exporter, or workflow-publication failure
  preserves `testVerdict`, sets `evidenceStatus = "degraded"`, and records one
  bounded closed degradation code;
- surface completion and qualification reject degraded evidence, but report
  the evidence rejection separately rather than claiming the underlying test
  failed.

The exact redacted remediation table is:

| Code | Stable input named | Exact recovery |
| --- | --- | --- |
| `D2B-BZLEVIDENCE-SANITIZE` | Repository-relative carrier definition and sink kind | Correct the sanitizer or closed permitted-field table, then rerun the owning `make test-bazel-rust-<slice>` target. |
| `D2B-BZLEVIDENCE-LIMIT` | Repository-relative `bazel/generated/evidence-sink-policy.json` row and sink kind | Reduce the emitted diagnostic, or regenerate the row from measured sanitized fixtures and review the measured delta, then rerun the owning slice target. |
| `D2B-BZLEVIDENCE-PUBLISH` | Stable carrier label and sink kind | Correct the injected publication backend failure, then rerun the owning slice target and require `evidenceStatus = "complete"`. |
| `D2B-BZLEVIDENCE-NO-VERDICT` | Stable workflow name and protected branch | Rerun the named protected-`v3` workflow attempt at the same head; if that head cannot complete, merge a new `v3` commit and restart the qualification streak. |

Messages contain none of the forbidden planted values, no run ID, attempt ID,
absolute path, cache key, token, or raw exporter error. Exact-message tests
reject an omitted stable input, omitted command, borrowed remedy, leaked
value, or success-shaped `evidenceStatus`.

Runner tests explicitly cover:

- prior manifest invalidation before dispatch;
- attribution when one surface has several carriers;
- sorted atomic partial manifest v1 publication after success, failure, and
  handled interruption;
- preservation of the original nonzero test or interruption status when
  publication also fails;
- exact ignored-case fidelity in listing, JUnit, and surface census;
- a planted failed result whose environment, argv, paths, identifiers,
  handles, shell name, terminal bytes, and raw output contain every forbidden
  redaction value. The fixture first proves every value is present, then proves
  all are absent from JUnit, `test.log`, emitted evidence, and exporter
  diagnostics.

## Filesystem semantics

`TEST_TMPDIR` and the output parent are anchored close-on-exec descriptors.
Strict paths refuse symlink, magic-link, and `..` traversal on both the
`openat2` and forced component-walk routes. Temporary creation, write, sync,
rename, and cleanup are descriptor-relative.

Creation collisions, short writes, `EINTR`, `EAGAIN`, `ENOSPC`, terminal
write failures, link parents, existing case directories, and cleanup ownership
are injected and mutation-tested.

## No-shell scope

Repository-owned wrapper, runner, cleanup, timeout, and process-control code
invokes no shell. The `rules_rust` stable-channel generated doctest runner
remains the recorded ADR 0052 difference. ADR 0017's governed source set is
unchanged.

An enforcing source and behavioral test inventories repository-owned spawn
sites and rejects `sh`, `bash`, `-c`, shell-script wrappers, and indirect shell
helpers. The upstream generated doctest runner is the only recorded exception
and is not repository-owned.

That inventory is generated, committed, and drift-checked at
`bazel/generated/no-shell-inventory.json`. Its governed-source and
declared-input sets are nonempty. Its spawn-site set is exact and may contain
zero entries for an individual governed source. It records:

1. `governedSources` - every repository-owned runner, cleanup, timeout, and
   process-control source subject to this rule, derived from the first-party
   configured-target census, not from a hand-maintained list;
2. `declaredInputs` - the exact declared inputs of the no-shell carrier; and
3. `scanResults` - exactly one successful record for every governed source,
   including a zero-site record when the source has no spawn construct; and
4. `spawnSites` - every discovered process-spawn construct, each naming its
   governed source, span, spawned program expression, and a typed
   `shellInvocation` verdict; any true verdict refuses.

`governedSources` and `declaredInputs` are equal in both directions. Every
`spawnSites[].source` belongs to that set, but the spawn-site source projection
is not required to contain a source with zero sites. `scanResults` has exactly
one successful entry for every governed source and no ungoverned entry. A
fresh scan derives the exact keyed spawn-site set from source path, span, and
spawned-program expression; that set and the committed `spawnSites` set are
equal in both directions. A walk, open, read, or parse failure produces a
failed scan result and refuses rather than shrinking either comparison.

Four plants are mandatory and each must fail at its own diagnostic:

```text
no-shell-inventory-empty
no-shell-inventory-missing-entry
no-shell-inventory-extra-entry
no-shell-inventory-planted-shell
```

The integrator commits the generated inventory with the rest of
`bazel/generated/`; slices produce `.scratch/` previews only. Its digest and
plant results enter the qualification evidence set.
