# Coverage Map Contract

`tests/golden/bazel-rust-coverage.json` is an internal committed artifact. It
does not version or replace execution-manifest v1.

## Required row

Each row contains:

- `surfaceId`: one baseline execution-manifest ID;
- `carrier`: one existing Bazel label owning the verdict;
- `companions`: additional labels needed for the same surface;
- `slice`: `main`, `api`, `broker`, or `aux`;
- `cargoBaseline`: current leaf/mode reference;
- `census`: exact manifest reference, expected entries/count, and derivation;
- `topology`: topology reference or explicit not-applicable reason;
- `testTargets`: all transitively carried Rust tests;
- `handwrittenFragments`: all non-generated BUILD fragments;
- `binaryProviders`: expected provider label and binary identity;
- `deliberateDifferences`: applicable ADR section 13 entries;
- `generatedBuildDigest`: digest binding the row to generated graph state.

Arrays and rows are sorted deterministically. Paths are normalized
repository-relative paths. Empty required collections are invalid.

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

`rust-contract-tests` and `rust-cli-contract-tests` must not appear.

## Fail-closed invariants

The guard rejects:

- an ID missing, duplicated, added, or mapped to multiple carriers;
- a carrier or companion label absent from Bazel query;
- a Rust test target not transitively claimed exactly once;
- a suite without exact nonempty census or required topology;
- a hand-written fragment not listed exactly once;
- a scan input set unequal to its committed manifest in either direction;
- parsed no-bash files unequal to declared inputs;
- empty/missing harness-free or doctest discovery;
- a schema generation differing from the exact twenty-file nonempty valid-JSON
  census before content comparison;
- a binary provider that is absent, non-executable, stale, or wrong identity;
- generated BUILD drift or Cargo/Bazel lock drift.

The guard itself is carried by the Bazel aggregate and generator drift by
existing `test-drift`. A passing guard is necessary but not sufficient for
promotion; execution evidence is also required.
