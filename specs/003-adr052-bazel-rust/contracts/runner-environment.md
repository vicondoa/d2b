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
- bounded sanitized failure text.

Forbidden content:

- environment values or argv;
- absolute, worktree, runfiles-root, store, or socket paths;
- process or user identifiers;
- opaque handles;
- unit names;
- shell names;
- terminal bytes;
- raw child output.

Raw stdout and stderr remain only in the ordinary per-target `test.log`.

Publication is enforcing. Publication failure makes an otherwise passing
carrier fail. If a test already failed, that test failure stays primary and
publication failure is additional.

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
  all are absent from JUnit and remain available only in `test.log`.

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
`bazel/generated/no-shell-inventory.json`. Each of its three sets is nonempty
by construction: an empty set is a refusal, never a vacuous pass. It records:

1. `governedSources` - every repository-owned runner, cleanup, timeout, and
   process-control source subject to this rule, derived from the first-party
   configured-target census, not from a hand-maintained list;
2. `declaredInputs` - the exact declared inputs of the no-shell carrier; and
3. `spawnSites` - every discovered process-spawn construct, each naming its
   governed source, span, spawned program expression, and a typed
   `shellInvocation` verdict; any true verdict refuses.

The three sets are compared bidirectionally by stable keys. The source-path
projections of `governedSources`, `declaredInputs`, and `spawnSites` are equal
in both directions. A fresh scan derives the exact keyed spawn-site set from
source path, span, and spawned-program expression; that set and the committed
`spawnSites` set are equal in both directions. Every governed source is scanned
and every governed source's scan result is recorded, so a walk, read, or parse
failure refuses instead of silently shrinking either comparison.

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
