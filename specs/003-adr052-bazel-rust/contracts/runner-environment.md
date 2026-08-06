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
and minting trait remain private to the provider module. Its public inherent
API allowlist is empty: callers receive it from the provider and can only pass
it by value to the execution function. The defining crate exposes no
descriptor extraction or access, unchecked constructor, path conversion or
accessor, `Deref`, `Borrow<OwnedFd>`, `AsFd`, `AsRawFd`, `IntoRawFd`,
`Default`, `From`, `Into`, `AsRef`, `Clone`, `Copy`, `Debug`, `Display`,
`Serialize`, or `Deserialize`.

The compiler-derived API census under `packages/d2b-api-surface/` is the
authority. `VerifiedExecutable` is a capability root. Its public item snapshot
must contain only the opaque type and the by-value provider/execution
signatures, its explicit locally-authored trait-implementation allowlist is
empty, and its compiler-emitted auto/blanket implementation set is pinned
exactly for the selected toolchain. Any added public field, method, associated
item, re-export, explicit trait implementation, or changed auto/blanket set is
an API-surface failure. Focused rustdoc `compile_fail` examples prove the
downstream type-system properties that the census alone cannot: callers cannot
construct the type, access or extract its descriptor, coerce it through
`Deref`/`Borrow`/`AsFd`, clone it, serialize or format it, convert an
unverified path or descriptor into it, or implement the sealed minting trait.
There are no Cargo-shelling compile fixtures.

The multithreaded runner never runs repository code in a post-fork child. It
consumes `VerifiedExecutable` into `Stdio` for fd 0 of the separately declared
`d2b-bazel-execveat` helper using safe `std::process::Command`; it registers no
`pre_exec` closure. The existing standard-library spawn implementation is the
only child-creation boundary; the repository adds no callback to it. A Linux
host conformance trace permits only the standard library's descriptor
dup/close, signal-mask/reset, and exec operations between child creation and
the helper image, and rejects allocation, logging, path lookup, or any
repository symbol in that interval. The consumed descriptor may be duplicated to fd 0 by the
standard library, but remains the same verified open file description and is
never reopened. The helper itself is a normal workspace binary, inherits
`unsafe_code = "forbid"`, is an exact-digest declared runfile, accepts no path,
and performs no fork. In its fresh single-threaded process it prepares C
strings and invokes the safe `nix::unistd::execveat` wrapper on fd 0 with an
empty path and `AT_EMPTY_PATH`. `ENOSYS` and every other exec error produce a
fixed typed status; there is no `fexecve`, `/proc/self/fd`, `Command`-by-target
path, or path fallback.

This process boundary removes the unsafe after-fork window instead of
hand-writing one. Tests prove the execution API consumes the handle, the
helper receives the verified open file description on fd 0, provider and
auxiliary descriptors are absent after target exec, argv and environment
contain no provider path, the target executes twice from the same verified
open file description, and a path-rebind mutation cannot affect it. Separate
policy tests reject `pre_exec`, `fork`, a locally-authored unsafe block, a
workspace-lint override, direct target `Command`, `/proc/self/fd`, `fexecve`,
or any reopen. The only unsafe/FFI boundary involved is the existing standard
library and pinned `nix` implementation; no product crate receives an unsafe
lint exemption.

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
| Runfiles entry missing in Bazel mode | Declared runfiles-relative provider key | `make test-bazel-rust-main`, `make test-bazel-rust-api`, `make test-bazel-rust-broker`, or `make test-bazel-rust-aux`, selected only by the closed coverage-map slice enum after declaring the key as `data`. |
| Provider is not a regular file | Declared runfiles-relative provider key | The same exact closed slice command after correcting the named target's `data` declaration. |
| Provider is not executable | Declared runfiles-relative provider key and mode | The same exact closed slice command after rebuilding the named target. |
| Provider is older than its newest declared input | Declared runfiles-relative provider key | The same exact closed slice command after rebuilding the named target. |
| Provider digest differs from the coverage map | Declared runfiles-relative provider key and coverage-map row | `(cd packages && cargo xtask gen-bazel --check)`, then the exact closed slice command after regenerating and reviewing the coverage map. |
| Handle metadata changed across digest read | Declared runfiles-relative provider key | The exact closed slice command; if it repeats, correct the writer named by the repository-relative coverage row and rerun that same command. |
| `execveat` returned `ENOSYS` | Stable kernel requirement | The exact closed slice command on a supported kernel providing `execveat`; no path fallback is available. |
| Other typed exec errno | Declared runfiles-relative provider key and errno class | The exact closed slice command after rebuilding the named target. |

The renderer accepts only the four literal commands shown in the table; there
is no free-form command string. The declared runfiles-relative provider key is
repository content and is permitted in the refusal. The runfiles root,
resolved absolute location, descriptor number, argv, environment value, and
child output remain forbidden. Exact-message tests cover every reason in every
slice and reject an omitted key, omitted action, omitted rerun, borrowed
remedy, nonliteral command, or leaked local value.

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
fields, truncation code, and retention class. Retention classes are closed:

| Class | Sink | Maximum age | Maximum count and scope |
| --- | --- | ---: | --- |
| `junit-v1` | JUnit | 14 days | 128 files per slice output root |
| `test-log-v1` | `test.log` | 14 days | 128 files per slice output root |
| `evidence-v1` | unsealed execution and qualification evidence | 30 days | 32 files per workflow and head digest |
| `exporter-diagnostic-v1` | exporter diagnostics | 7 days | 64 records per workflow and head digest |

Sealed, schema-bounded source records under this specification are state
documents, not raw sink payloads; they remain one atomically replaced record
per declared path. Every other persisted sink must name exactly one class.
Before publication, descriptor-relative expiry removes owned entries older
than the class age and then retains only the newest permitted count. Failure
to classify, inspect, or expire refuses publication. CI upload configuration
uses the same literal age. Injected-clock tests cover just-inside, exact-bound,
and expired ages; count-minus-one, exact-count, and count-plus-one inventories;
newest retention; unowned/link refusal; and expiry failure with no
publication. Initial limits are generated
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

Test outcome and evidence publication are separate typed results.
`testVerdict` is the underlying `passed`, `failed`, `ignored`, or
`interrupted` result and is never rewritten by an exporter. `evidenceStatus`
is a closed tagged union:

- `{"kind":"complete","sinkPolicySha256":"<sha256>","retentionClass":"<closed>"}`;
- `{"kind":"degraded","code":"<closed-code>","sinkKind":"<closed>",
  "policyRowSha256":"<sha256>","retryCommand":"<closed-command>"}`.

The complete variant rejects degradation-only fields. The degraded variant
requires every field above and rejects complete-only fields, unknown fields,
unknown codes, and free-form commands. A sanitizer, bound, retention, write,
rename, exporter, or workflow-publication failure preserves `testVerdict` and
produces the structurally valid degraded variant. Surface completion and
qualification reject degraded evidence but report the evidence refusal
separately rather than claiming the underlying test failed. Execution-manifest
v1 remains byte- and schema-compatible: the tagged status is a sidecar
publication result and is never added to manifest v1.

The exact redacted remediation table is:

| Code | Stable input named | Exact recovery |
| --- | --- | --- |
| `D2B-BZLEVIDENCE-SANITIZE` | Repository-relative carrier definition and sink kind | Correct the sanitizer or closed permitted-field table, then run the exact closed slice command selected from the provider table. |
| `D2B-BZLEVIDENCE-LIMIT` | Repository-relative `bazel/generated/evidence-sink-policy.json` row and sink kind | Reduce the emitted diagnostic, or run `(cd packages && cargo xtask gen-bazel --check)` after reviewing measured policy changes, then run the exact closed slice command. |
| `D2B-BZLEVIDENCE-RETENTION` | Repository-relative sink-policy row and retention class | Correct the owned retention inventory, then run the exact closed slice command. |
| `D2B-BZLEVIDENCE-PUBLISH` | Stable carrier label and sink kind | Correct the publication backend, run the exact closed slice command, and require the complete tagged variant. |
| `D2B-BZLEVIDENCE-NO-VERDICT` | Stable workflow name and protected branch | `git fetch origin v3`, then `(cd packages && cargo xtask bazel-qualification-validate)`; if the fixed record remains incomplete, merge a new protected `v3` commit and rerun the same validator. |

Messages contain none of the forbidden planted values, no `$!`, run ID,
attempt ID, absolute path, Nix store path, cache key, token, opaque handle, or
raw exporter error. Artifact and validator failures render a fixed code,
repository-relative policy row, and SHA-256 only. Exact-message tests cover
every code/slice combination and reject an omitted stable input, omitted
command, borrowed remedy, leaked value, free-form command, or malformed or
success-shaped status variant.

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

Six plants are mandatory and each must fail at its own diagnostic:

```text
no-shell-inventory-empty
no-shell-inventory-missing-entry
no-shell-inventory-extra-entry
no-shell-inventory-unguarded-spawn
no-shell-inventory-missing-zero-site-record
no-shell-inventory-planted-shell
```

Both the raw `scanResults` record count and the unique scan-source count must
equal the governed-source count. A duplicate record is a refusal even when the
unique projection still matches.

The integrator commits the generated inventory with the rest of
`bazel/generated/`; slices produce `.scratch/` previews only. Its digest and
plant results enter the qualification evidence set.
