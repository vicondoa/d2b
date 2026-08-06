# Shadow and Promotion Evidence Contract

Evidence summaries under `specs/003-adr052-bazel-rust/evidence/` contain
immutable references and computed outcomes only. They contain no logs,
transcripts, credentials, or attestation payloads.

## Qualification record

One record is a `push` on `refs/heads/v3` produced by a merged pull request and
contains:

- one head commit for Cargo and Bazel workflow runs;
- both run IDs and verdicts;
- a passing same-commit fixture-contract verdict;
- four Bazel slice verdicts and, for a cold record, `sliceDurationsSeconds`
  with exactly four complete durations;
- manifest references;
- explicit `bazelRestoreCount`, `bazelSaveCount`, and
  `bazelPublicationCount`;
- effective permissions.

The canonical cache field spellings are exactly `bazelRestoreCount`,
`bazelSaveCount`, `bazelPublicationCount`, and `sliceDurationsSeconds`.
No other spelling appears in any artifact or record.

Pull-request, `main`-push, scheduled, and dispatched runs are diagnostic only.
Pull-request runs emit no qualification record and contain zero cache actions;
they therefore have no synthetic zero-count qualification object.

Every record carries all three counts. Shadow qualification requires each to be
zero. Every cold record additionally carries `sliceDurationsSeconds` with four
complete durations and `bazelRestoreCount` of zero. A missing count or
duration makes the record non-qualifying; it is never read as zero.

Differing verdicts reset the streak. A Bazel run with no verdict beside a Cargo
verdict is a mismatch. A push where neither reaches a verdict is no record.

## Typed qualification validator

`packages/xtask/src/bazel_qualification.rs`, with tests in
`packages/xtask/tests/bazel_qualification.rs`, is the only authority that
decides whether a record qualifies. It is implemented no later than spec003w3.

Every threshold is derived from complete paginated Cargo, Bazel, and fixture
workflow-run inventories plus the record's immutable content references.
A workflow reference is (`runId`, positive `attempt`, `headSha`); content
references are a
commit reference (full SHA), a content reference (path plus digest), or a
generated-artifact reference (generated path plus digest). The validator:

- refuses pagination gaps, missing attempts, duplicate or conflicting run
  identities, and a record that omits an intervening protected-`v3` push;
- normalizes each run ID to its highest terminal attempt, pairs Cargo, Bazel,
  and fixture verdicts by head SHA, and derives mismatch resets rather than
  trusting a curated streak;
- derives the five newest qualifying cold records from the complete ordered
  stream rather than trusting a curated five-record subset;
- resolves each threshold by counting or comparing the referenced evidence, so
  a threshold such as ten matching records, twenty broker executions, eighteen
  isolated failures, five cold measurements, or four slice durations is a
  property of the references, never of a stated number;
- refuses an omitted reference for any threshold it must derive;
- refuses a forged or ill-formed reference: a run ID, SHA, digest, or path that
  does not parse in its declared shape, or a digest that does not match the
  referenced content;
- refuses duplicate references, whether repeated inside one threshold or reused
  across thresholds that must be independent;
- refuses inconsistent references, where two references that must agree
  disagree on head SHA, commit, digest, or count;
- refuses a wrong-candidate reference, where evidence that must bind the
  candidate commit binds a different commit.

`qualification.json` cannot qualify through a trusted boolean. Boolean and
count fields such as `qualified`, `eligible`, or any summary total are
informational mirrors of the derived result; a mirror that disagrees with the
derived result is a refusal, not a warning. The validator's own status output
is the verdict.

The command is `cargo xtask bazel-qualification-validate`. It takes no
arguments, reads the fixed repository-relative record path, is unreachable from
Make and every workflow, and is listed in the contributor-only command set of
`make-target-compatibility.md`. Evidence curation runs it before sealing the
record, promotion validation runs it against the sealed record at the promotion
candidate, and contributor validation runs it before any informational
inspection of the record.

## Workspace and hub evidence

Qualification proves:

- one product resolver-v2 workspace and root lock;
- no broker or guest nested workspace or lock;
- a separate walker workspace and lock;
- `Cargo.guest.lock` absent from hub authority;
- only product and walker hubs;
- retired main, broker, and guest identifiers refusing with exact argv and cwd
  tests;
- first-party product crates represented by native targets;
- no `crate.spec`;
- product and walker repin outputs current.
- module refresh proving exact `MODULE.bazel.lock`-only mutation, second-run
  idempotence, absolute startup-option identity, exact drift remediation, and
  no Make or workflow reachability;
- both dedicated Nix derivations retaining the exact
  `cargoLock.outputHashes."wl-proxy-0.1.2"` value.

## Package policy evidence

For each of broker GNU and guest musl on x86_64 and aarch64, qualification
contains:

- exact production and policy graph digests;
- exact selected root and nonempty closure;
- system, target, edge-kind, cfg, and feature checks;
- exact selected-source identity set and count;
- metadata and filtered-lock identity equality;
- source readability and checksum results;
- deny results over root-dev-inclusive metadata;
- pinned RustSec `--no-fetch` audit result;
- broker empty ignore and guest one-ignore assertion;
- closure leakage and forbidden dependency results.

The guest record proves exactly six package-scoped license exceptions and
proves a different package with the same licenses remains denied.

The yanked record proves one committed snapshot whose exact key set derives
only from `packages/Cargo.lock`, excluding the walker and
`Cargo.guest.lock`. It proves `rust-deny-main` checked the full product set and
the broker and guest carriers checked exact selected-policy-graph projections
against that same snapshot. It includes separate reviewed
`bazel-yanked-refresh` network observation and offline
`bazel-yanked-check` verdict references.

The supply-chain equivalence record contains, for main, broker, and guest, the
current Cargo raw enforcing exit status, the decomposed Bazel deny/audit/yanked
status, both sorted normalized finding sets, and an equality result. Main uses
the full product; broker and guest use exact selected policy projections. Any
status or finding difference makes the record unqualified and blocks both spec003w1
and promotion.

## Native architecture evidence

For each native runner, qualification contains realization references for:

```text
broker-production-dependency-policy
guest-shell-runner-static-dependency-policy
broker-production-package-policy
guest-real-libshpool-package-policy
guest-static-elf
```

It also proves:

- matching system and GNU or musl target;
- matching runner architecture;
- no foreign-system argument;
- no `--builders`;
- no remote builder;
- both generated system inventories current.
- `make test-rust-supply-chain` passed on the same native arm stable head as
  the five aarch64 realizations;
- the workflow renderer test covers that arm command and the PR head did not
  change between rendered-workflow validation and native evidence.
- every guest static ELF is `ET_DYN` for the native system's expected
  `e_machine`, with no `PT_INTERP` and no `DT_NEEDED`; non-PIE and
  wrong-machine plants are present and refused.

## Complete qualification

`qualification.json` is qualified only with:

1. exact eighteen-surface coverage;
2. ten consecutive matching qualification records;
3. eighteen isolated surface failures;
4. exact test, companion, scan, schema, and API censuses;
5. main and guest per-case topology and broker per-binary topology proof,
   including literal `tags = ["exclusive"]`, no overlap with any other test,
   and a passing tag-removal mutation;
6. twenty consecutive executions per broker context with
   `--runs_per_test=20` and exclusivity in force;
7. warm local, cold local, and cold CI performance sets;
8. complete package policy evidence above;
9. both native architecture realization sets above;
10. `bazelRestoreCount`, `bazelSaveCount`, and `bazelPublicationCount` of
    zero in every shadow record, with five cold records each carrying four
    complete `sliceDurationsSeconds` entries;
11. complete locator and per-case evidence guards;
12. all workflow, cache, deadline, cleanup, repin, and seeded policy
    refusals.
13. canonical declared loopback TCP and Unix-socket tests passing, plus
    external-egress and live-index plants proving host/external egress was
    denied and every permitted fetch was a pinned repository rule;
14. exact Cargo/decomposed-Bazel supply-chain equivalence for all three
    contexts;
15. manifest/JUnit/redaction/ignored-case/original-status/no-shell evidence and
    combined-budget mutations;
16. the committed `bazel/generated/no-shell-inventory.json` reference and
    digest, its nonempty result, its bidirectional three-source-projection and
    fresh-scan/committed spawn-site-key equality results, and the empty,
    missing-entry, extra-entry, and planted-shell plant results;
17. a successful `cargo xtask bazel-qualification-validate` verdict derived
    from the references above.

Candidate-specific evidence binds one integrated commit. A content change
invalidates affected evidence. The qualified record merges before promotion
and is immutable afterward.

## Promotion and retirement

Promotion references the qualification digest. The promotion record captures
the promotion SHA, cache maintenance and save, first promoted verdict, and
rollback rehearsal. `promotion-record.json` is created after the promotion
merge; the pre-merge rollback rehearsal therefore resolves its candidate from
the verified current atomic candidate HEAD and the recorded spec003w5 parent,
and no pre-merge step reads the promotion record.

Post-promotion run-unit inventory keeps independent release-containment and
green-run clocks. Alias removal depends only on containment in a published semantic
release tag matching `v<major>.<minor>.<patch>`. Cargo implementation
retirement depends only on ten distinct ordered green promoted `v3` run
units. Either child may land first. If both edit a shared file, the child
that lands second rebases onto the merged first child, reruns its complete
validation on the new stable head, and obtains a new ten-seat panel result.
Neither removes a public Rust Make name.

## Typed post-promotion run units

The run-unit source paginates the authoritative workflow-run API to
completion and inventories every promoted protected-`v3` `test-rust` run
unit. A run unit is one distinct push-created `(runId, headSha)` pair. An
attempt is never a unit and never a streak position.

Each unit contains:

- immutable `runId` and `headSha`, whose pair is the unit identity;
- `event`, exactly `push`;
- `branch`, exactly `v3`;
- `attempts`, the complete nested history `1..maxAttempt` with each
  attempt's `conclusion`, `runStartedAt`, and `completedAt`;
- `conclusion`, normalized to the conclusion of the highest terminal attempt;
- `createdAt`, the immutable creation timestamp of the unit; and
- `promotionAncestor`, derived by verifying the promotion commit is an
  ancestor of `headSha`.

Ordering is ascending `(createdAt, runId)`. `runStartedAt` is never an
ordering input: a rerun updates it, and ordering by it would let an old rerun
move behind newer failures and silently repair a broken streak. Attempt
timestamps order attempts inside a unit only.

Pagination page and cursor continuity are validation inputs, not persisted
eligibility claims. Refusals are: missing pages; a unit missing any attempt in
`1..maxAttempt`; attempts of one unit carrying conflicting `headSha`,
`event`, `branch`, or promotion provenance; repeated or missing unit
identities; non-v3 or non-push runs; pre-promotion ancestry; and a highest
attempt with a nonterminal conclusion. Any terminal non-success conclusion,
including failure, cancellation, timeout, or startup failure, resets the
streak.

The validator computes the reset positions and current consecutive-success
streak from the complete ordered unit stream, counting each unit exactly once.
It never reads or trusts `eligible`, `consecutive_green_count`, or
`green_run_ids` fields. Retirement requires the derived final ten distinct
ordered units to be successes with no intervening failure or cancellation.

Two fixtures are mandatory:

- a repeated-attempt fixture, where one unit has several successful attempts
  and contributes exactly one streak position; and
- an old-rerun-after-failure fixture, where a unit created before a later
  failing unit is rerun successfully after that failure and still orders before
  it, leaving the streak reset in place.
