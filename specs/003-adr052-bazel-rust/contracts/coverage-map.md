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
- `actionNetwork = "none"` for every Bazel action, plus the declared-input
  source for every tool, advisory database, yanked record, and vendored crate;
- `cargoCompatibilityCarriers`, an exact sorted census of mandatory
  socket-using Rust tests that cannot run as Bazel actions under ADR 0052.
  Each entry binds its existing surface ID, Cargo selector, test identity,
  socket class, and same-commit verdict owner. A row with no such carrier uses
  an empty array;
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

ADR 0052's action no-network rule remains absolute: no Bazel Rust action may
open an IPv4, IPv6, netlink, packet, or Unix socket, including loopback and
sandbox-local listeners. Network namespaces prevent host and external egress;
they do not enforce per-endpoint declarations, and this contract makes no such
claim. Structural inventory and sandbox strategy checks reject an action-level
URL, live-index input, downloader, network-enabling tag, local-strategy
fallback, or missing declared offline input. Behavioral plants attempt one
loopback TCP socket and one Unix socket from a Bazel action and must both fail.
The existing external-egress and live-index plants remain additional
failures. The only fetch rows are repository rules pinned by a Cargo checksum
or the `wl-proxy` revision plus archive sha256.

Committed mandatory tests that use sockets remain on the existing
non-Bazel Cargo compatibility path until a separately authorized design
changes the invariant. Their exact case census is generated from the Cargo
listing and committed in the coverage map. The same protected commit must
produce both the Bazel carrier verdict and every compatibility-carrier verdict
for their shared surface. Missing, skipped, advisory, stale-head, or
misattributed compatibility evidence fails surface completion and promotion.
Promotion reports these surfaces as hybrid and Cargo retirement retains the
compatibility executor and its public target. No endpoint declaration or
network namespace is cited as enforcement.

## Test-first non-main carriers

The generated carrier files are deliberately disjoint:

| Carrier file | Surface |
| --- | --- |
| `bazel/carriers/schema.bzl` | One action runs two sequential generations into distinct directories, proving two independent nonempty exact censuses before comparison; mismatch and empty-output plants. |
| `bazel/carriers/stub.bzl` | Stub-no-socket executable identity and runtime-state checks; missing executable, wrong identity, and state-creation plants. |
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
| Every Bazel action is no-network; mandatory socket tests remain exact same-commit Cargo compatibility carriers; every fetch is a pinned repository rule | Linux-sandbox and strategy inventory, loopback-TCP and Unix-socket denial plants, external-egress/live-index plants, compatibility census, and `test-policy` |
| No-bash parsed-file census equals governed manifest and declared inputs | Walker unit tests plus coverage test |
| Generated `bazel/generated/no-shell-inventory.json` has equal nonempty governed and declared sets, one scan record per governed source including zero-site records, only governed spawn sites, and exact fresh-scan/committed spawn-site keys | Census-generator tests, coverage test, and `test-drift` |

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
generated crate; a broker tag removal or overlap; any Bazel socket use,
forbidden external egress, a live-index input, or missing, stale, advisory, or
wrong-head Cargo compatibility evidence; a no-bash walk, read, or parse
failure or mismatch among
the governed manifest, declared inputs, and parsed-file census; and an empty,
missing-entry, extra-entry, planted-shell, governed/declared mismatch,
unguarded-spawn-site, missing-zero-site-scan-record, or
fresh-scan/committed-spawn-mismatch no-shell inventory.
