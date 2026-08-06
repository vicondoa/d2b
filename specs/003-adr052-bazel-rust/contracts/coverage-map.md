# Coverage Map Contract

`tests/golden/bazel-rust-coverage.json` binds the existing eighteen
execution-manifest IDs to Bazel carriers. It does not replace manifest v1.

## Exact ID set

```text
rust-api-surface
rust-main-format
rust-main-clippy
rust-main-workspace-tests
rust-no-bash-ast
rust-schema-reproducibility
rust-stub-no-socket
rust-assert-pinned
rust-broker-default
rust-broker-layer1
rust-broker-fakebackends
rust-guest-shell-runner
rust-deny-main
rust-deny-broker
rust-deny-guest
rust-audit-main
rust-audit-broker
rust-audit-guest
```

Fixture-backed IDs do not appear.

## Required row

Each row contains:

- one `surfaceId`;
- nonempty carriers with exactly one verdict owner;
- one of `main`, `api`, `broker`, `aux`;
- the current Cargo baseline using root product package selectors;
- exact generated census and out-of-census reasons;
- per-carrier topology;
- all carried Rust tests;
- every hand-written fragment;
- configured first-party target labels and direct dependency, cfg, and feature
  census;
- binary providers and declared runfiles-relative paths;
- locator migration dispositions;
- deliberate ADR 0052 differences;
- generated BUILD digest.
- `actionNetwork = "sandbox-local-declared"` only for carriers whose committed
  tests require declared loopback TCP or Unix sockets, otherwise
  `actionNetwork = "none"`; every row still records the declared-input source
  for every tool, advisory database, yanked record, and vendored crate;
- for each broker row, the literal target tag set `["exclusive"]`.

Rows and arrays are sorted. Required collections cannot be empty.

## Hub and native-target invariants

- Third-party product dependencies come only from `@product`.
- Walker dependencies come only from `@walker`.
- Every first-party product crate is a native Bazel target.
- Broker default, layer1, and fake contexts and guest real-libshpool each have
  an exact configured native target census.
- The product external package and feature union may exceed a configured
  context.
- Actual first-party dependencies and features are defined by configured
  native targets, not by the product hub union.
- No configured broker context reaches guest or an unrelated first-party
  sibling.
- No guest context reaches broker or an unrelated first-party sibling.

## Broker scheduling isolation

`rust-broker-default`, `rust-broker-layer1`, and
`rust-broker-fakebackends` each map to a Bazel suite carrying exactly
`tags = ["exclusive"]`. Bazel must schedule each after all nonexclusive tests,
so none may overlap another broker suite or any other test. A custom local
resource is not an equivalent mechanism.

The coverage guard rejects a missing or renamed tag. Its mutation removes
`exclusive` from one suite and must observe overlap with a planted ordinary
test. Qualification runs each broker context twenty consecutive times with
`--runs_per_test=20`, one context at a time, while an ordinary overlap probe is
present; every run must show the broker suite alone.

## Action network inventory

Every carrier row identifies whether an input was produced by a pinned
repository rule or is a committed/generated declared input. Actions may open
only declared sandbox-local loopback TCP and Unix sockets required by the
committed tests. Host or external egress, DNS, live package or advisory
indexes, and undeclared listeners are forbidden. The only fetch rows are
repository-rule rows pinned by a Cargo checksum or the `wl-proxy` revision
plus archive sha256.

The guard rejects an action-level URL, live-index input, downloader, external
destination, DNS resolver, undeclared listener, unpinned repository rule, or
missing declared input. The seeded matrix includes separate live-index and
external-egress plants, and both must fail their owning policy predicate rather
than a later carrier assertion. A blanket socket-syscall denial is itself a
failing mutation because it breaks canonical local-socket tests.

## Test-first non-main carriers

The generated carrier files are deliberately disjoint:

| Carrier file | Surface |
| --- | --- |
| `bazel/carriers/schema.bzl` | One action runs two sequential generations into distinct directories, proving two independent nonempty exact censuses before comparison; mismatch and empty-output plants. |
| `bazel/carriers/stub.bzl` | Stub-no-socket executable identity and runtime-state checks; missing executable, wrong identity, state creation, and forbidden undeclared-listener plants. |
| `bazel/carriers/inventory.bzl` | Pinned test inventory; empty, missing, and extra inventory plants. |
| `bazel/carriers/no_bash.bzl` | No-bash walker input and parsed-census wiring, separate from main. |

`bazel/carriers/main.bzl` is not a shared writer for these surfaces.

## Promoted public target mapping

Promotion introduces exactly four authoritative CI slice targets:

```text
test-rust-slice-main
test-rust-slice-api
test-rust-slice-broker
test-rust-slice-aux
```

Generated CI calls those names only. The eight existing public leaves retain
their current surface semantics and forward to these exact carrier subsets:

| Public leaf | Bazel subset after promotion |
| --- | --- |
| `test-rust-api-surface` | `//ci/rust:api_census`. |
| `test-rust-main` | `//ci/rust:fmt`, `//ci/rust:clippy`, `//ci/rust:main_tests`, `//ci/rust:main_doctests`, and `//ci/rust:main_harness_free`, plus the unchanged conditional Cargo/Nix fixture and CLI path. |
| `test-rust-broker` | `//ci/rust:broker_default`, `//ci/rust:broker_layer1`, and `//ci/rust:broker_fakebackends`. |
| `test-rust-guest-shell-runner` | `//ci/rust:guest_shell_runner`. |
| `test-rust-no-bash-ast` | `//ci/rust:no_bash_ast`. |
| `test-rust-schema` | `//ci/rust:schema_reproducibility`. |
| `test-rust-inventory` | `//ci/rust:stub_no_socket` and `//ci/rust:pinned_test_inventory`. |
| `test-rust-supply-chain` | `//ci/rust:deny_main`, `//ci/rust:deny_broker`, `//ci/rust:deny_guest`, `//ci/rust:audit_main`, `//ci/rust:audit_broker`, and `//ci/rust:audit_guest`; each deny carrier includes its yanked projection. |

## Guard placement

| Invariant | Enforcement |
| --- | --- |
| Mapped carrier label exists | Analysis-time `deps` or `data` edge |
| Carrier belongs to exactly one ID | Coverage test |
| No Rust test target is unclaimed | Make wrapper and `test-drift` over committed query result |
| Query result is current | `test-drift` |
| Exact census, topology, native target, cfg, feature, and fragment list | Coverage test |
| Hub and lock containment | Selected-context query checks |
| Generated BUILD and policy output current | `test-drift` |
| Broker suite keeps `tags = ["exclusive"]` and cannot overlap any test | Coverage test plus scheduling mutation |
| Only declared sandbox-local sockets are usable; external/live-index egress is denied; every fetch is a pinned repository rule | Hermeticity inventory, local-socket positives, external-egress/live-index plants, and `test-policy` |
| No-bash parsed-file census equals governed manifest and declared inputs | Walker unit tests plus coverage test |
| Generated `bazel/generated/no-shell-inventory.json` is nonempty; its three source projections agree in both directions; fresh-scan and committed spawn-site keys agree in both directions | Census-generator tests, coverage test, and `test-drift` |

No Bazel test invokes `bazel query` or starts a nested server.

## Required hand-written fragments

Exactly once:

- per-target nightly transition;
- `rustdoc_json` rule;
- pinned vendor repository rule;
- package-policy carriers and selected-source census checker;
- product and walker hub containment checker;
- aggregate, slice, carrier, and coverage guards.

There is no synthetic splice fragment and no `crate.spec` fragment.

## Fail-closed cases

The guard refuses missing, duplicate, or added IDs; empty carriers; multiply
claimed carriers; absent labels; unclaimed Rust tests; missing topology or
census; stale query or BUILD output; missing fragment; empty scan or companion
sets; mismatched configured native target dependencies, cfgs, or features;
wrong product or walker containment; cross-context edges; unrelated
first-party siblings; any first-party target represented as an external
generated crate; a broker tag removal or overlap; forbidden external egress or
a live-index input; a no-bash walk, read, or parse failure or mismatch among
the governed manifest, declared inputs, and parsed-file census; and an empty,
missing-entry, extra-entry, planted-shell, source-projection-mismatch, or
fresh-scan/committed-spawn-mismatch no-shell inventory.
