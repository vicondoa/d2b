# Coverage Map Contract

`tests/golden/bazel-rust-coverage.json` is an internal committed artifact. It
does not version or replace execution-manifest v1.

## Required row

Each row contains:

- `surfaceId`: one baseline execution-manifest ID;
- `carriers`: the nonempty set of carrier entries for this surface, each naming
  an existing Bazel label, whether it owns the verdict, and either its topology
  or an explicit not-applicable reason. Exactly one entry owns the verdict.
  Topology is per carrier, not per surface, because
  `rust-main-workspace-tests` carries a process-per-case suite, a doctest
  carrier, and a harness-free carrier under one identifier;
- `slice`: `main`, `api`, `broker`, or `aux`;
- `cargoBaseline`: current leaf/mode reference;
- `census`: the generator-derived census artifact, its expected entries and
  count, and the derivation that produced it;
- `outOfCensus`: every manifest entry the executed selector excludes, each with
  its reason;
- `testTargets`: all transitively carried Rust tests;
- `handwrittenFragments`: all non-generated BUILD fragments;
- `binaryProviders`: expected provider label, runfiles path, and the byte
  digest the located descriptor must match before that same descriptor is
  executed;
- `locatorFiles`: the migrated first-party files this surface's tests use, each
  `migrated` or `no-migration-needed` with a reason;
- `deliberateDifferences`: applicable ADR section 13 entries;
- `generatedBuildDigest`: digest binding the row to generated graph state.

Arrays and rows are sorted deterministically. Paths are normalized
repository-relative paths. Empty required collections are invalid, and a
hand-written count is invalid wherever the generator can derive one.

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

## Cardinality

The mapping is **total and unambiguous**, not one-to-one: every ID has a
nonempty carrier set and every carrier belongs to exactly one ID.
`rust-main-workspace-tests` already needs three carriers. The guard enforces
both directions of totality.

## Required hand-written fragments

These are not generated and must each appear exactly once in
`handwrittenFragments`:

- the per-target nightly channel transition rule over the API census subgraph;
- the `rustdoc_json` rule that renders the census and emits the toolchain
  version the action actually used;
- the vendor repository rule that materializes the offline dependency tree;
- the yanked-state carrier fragment that consumes the committed lock-bounded
  snapshot, runs the repository-owned offline `bazel-yanked-check` validator
  over it and the three committed locks as declared inputs, and reports under
  `rust-deny-main`, `rust-deny-broker`, and
  `rust-deny-guest`, which exists unconditionally and adds no nineteenth ID;
- the aggregate, slice, carrier, and guard fragments under `bazel/` and
  `ci/rust/`.

## Where each invariant is proved

A Bazel test action has no server, no source tree, and no sanctioned way to
reach one. A condition phrased as a nested `bazel query` inside the test cannot
execute and would leave the guard green while proving less than it claims. The
split is therefore load-bearing:

| Invariant | Proved at |
| --- | --- |
| Every mapped carrier label exists | Analysis time, through real `deps`/`data` edges from the guard target |
| Every carrier belongs to exactly one ID | Bazel test |
| No Rust test target is unclaimed | Make wrapper and `test-drift`, over `tests/golden/bazel-rust-query.json` |
| Query result is not stale | Make wrapper and `test-drift` |
| Exact census, topology, hand-written-fragment listing | Bazel test |
| Generated BUILD and lock drift | `test-drift` |

No Bazel test invokes `bazel query`, and no test action runs a nested Bazel
server. The committed query result is a declared input to the out-of-test
check and is drift-checked by the same mechanism that guards every other
generated output, so no new gate, Layer-1 job, or Make target is created.

## Fail-closed invariants

The guard rejects:

- an ID missing, duplicated, or added;
- an ID with an empty carrier set;
- a carrier claimed by more than one ID;
- a carrier or companion label that does not exist, which fails analysis
  naming the label before any test runs;
- a Rust test target not transitively claimed exactly once;
- a suite without an exact nonempty census or a required topology;
- a hand-written fragment not listed exactly once, including the four
  fragments named above;
- a scan input set unequal to its committed manifest in either direction;
- parsed no-bash files unequal to declared inputs;
- empty or missing harness-free or doctest discovery, and a harness-free
  census that does not match the selector the Cargo gate uses;
- an out-of-census manifest entry with no recorded reason;
- a schema generation differing from the generated nonempty valid-JSON census
  before content comparison;
- a binary provider that is absent, non-regular, non-executable, stale, or of
  the wrong identity, or whose path is rebound to a different file after the
  single anchored open, where each of those states is supplied through the
  injected `FileSystem` and `RunfilesView` boundaries rather than arranged on
  disk;
- a locator file that is neither migrated nor recorded as needing no
  migration;
- an emitted census toolchain version that differs from the committed pin;
- a `.bazelrc` line or wrapper argument that sets the toolchain channel flag;
- generated BUILD drift, `.bazelignore` drift, or Cargo/Bazel lock drift.

A passing guard is necessary but not sufficient for promotion; execution
evidence is also required.
