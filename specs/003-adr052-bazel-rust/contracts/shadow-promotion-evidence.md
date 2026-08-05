# Shadow and Promotion Evidence Contract

Evidence summaries are internal migration artifacts under
`specs/003-adr052-bazel-rust/evidence/`. They contain immutable references and
computed outcomes, not full logs, panel transcripts, credentials, or
attestation payloads.

## Qualification record and canonical predicate

ADR 0052 section 9's `Q(row)` table is the single canonical qualification
predicate. This contract projects it; it does not define a second predicate.
The complete lineage starts at the merged W3 shadow-workflow commit and ends at
the recorded protected-`v3` tip. It retains every chronological first-parent
`push` on `refs/heads/v3` whose head is a pull request's merge commit into
`v3`. Pull-request, `main`-push, scheduled, and dispatched runs are diagnostic
and never enter that lineage.

For every eligible push, the repository-owned authoritative resolver records
the GitHub workflow ID, immutable run ID, run attempt, immutable job ID, head
SHA, event, branch, conclusion, and normalized Actions/Pulls API evidence
reference and digest for:

- the Cargo `D2B_SKIP_FIXTURE_BUILD=1 make test-rust` rollup;
- the Bazel rollup;
- the same-commit `make test-policy` job, which owns `policy_docs` and the
  other fixture-independent policy binaries selected by `tests/lib.sh`;
- the same-commit
  `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` job for the
  fixture-dependent contract and CLI surfaces; and
- all four attributed Bazel slice jobs.

Every resolved job must belong to the recorded workflow run and attempt at the
eligible head SHA. A job ID cannot be reused by another row or carrier slot.
Policy and fixture are not compared between executors, but both must pass on
that same commit. Fixture evidence is never cited as policy-doc execution.
`Q(row)` is true only when Cargo, Bazel, policy, fixture, and all four slices
resolve and pass with zero shadow cache restore or write.

The resolver enumerates the complete eligible lineage before reading authored
records. A genuine eligible push with a missing, failed, timed-out, skipped, or
cancelled carrier remains a false-`Q` row and resets the streak, even when
every carrier was cancelled. A forged ID, wrong-commit ID, stale reused job,
wrong attempt or workflow, ineligible inserted event, omitted or duplicate
eligible push, order change, or omitted reset disqualifies the evidence set
until corrected. The stored streak is the maximal true-`Q` suffix derived by
replaying the complete lineage; it is never an authored counter.

### Cold-sample qualification

A record supplies a cold continuous-integration measurement only when
`C(row) = Q(row) && cache_restored == 0 && four complete resolved slice
durations`. The same authoritative slice job IDs that satisfy `Q` supply the
Actions-API durations; a duration copied from another run or attempt is a
wrong-commit/stale-ID failure. The scalar duration is their maximum, matching
the workflow critical path. Therefore both same-commit policy and fixture jobs
must pass for every cold sample. During shadow every conforming run is cold by
construction because nothing is published or restored.

## Qualification record set

`qualification.json` is `qualified` only with:

1. both halves of the coverage guard passing for exactly eighteen rows, the
   analysis-time half and the out-of-test completeness half;
2. a complete resolver-confirmed eligible lineage whose maximal suffix
   contains at least ten true-`Q` rows, with passing same-commit Cargo, Bazel,
   `make test-policy`, fixture-contract, and four-slice jobs in every row;
3. eighteen seeded failure records, each failing only its owning surface;
4. exact generator-derived test, ignored, doctest, harness-free, API, schema,
   scanner, and pinned-test censuses, with every out-of-census entry recorded
   with its reason;
5. main and guest per-case topology, broker exclusive per-binary topology, and
   per-case result publication proofs;
6. twenty consecutive passes for each broker feature suite;
7. three valid warm-local, three cold-local, and the five most recent `C` rows
   from that same complete lineage, with the cold continuous-integration set
   referencing the W3 feasibility measurement that made the ceiling binding;
8. identical enforcing supply-chain outcomes for all three locks, and the
   yanked carrier landed under the existing `rust-deny-*` identifiers with its
   committed lock-bounded snapshot passing the offline key-set drift check for
   all three locks, whatever the comparison found;
9. zero shadow cache restore, save, or publication;
10. the complete locator migration record set, every file migrated or recorded
    as needing no migration, with the injected stale-provider negative failing
    in Bazel mode as required and the injected post-open path-rebind negative
    proving the verified descriptor's bytes are the ones that ran, both
    supplied through the `FileSystem` and
    `RunfilesView` fakes rather than by an executable written to a live path,
    plus the host-backed `execveat` conformance result, which is the one
    provider property a fake cannot establish;
11. positive and all required negative workflow, cache, deadline, cleanup,
    per-case-redaction, and result-filesystem controls passing.

Candidate-specific coverage, seeded-failure, topology, locator,
local-performance, and supply-chain evidence binds one integrated candidate
commit. Historical qualification records each retain their own head commit and
immutable workflow run/job IDs plus authoritative resolver evidence. A
candidate content change invalidates affected candidate-specific evidence.
`qualification.json` is reviewed and merged before promotion work begins and
is immutable after qualification.

## Predicate consistency and negative fixtures

The normative-site inventory is closed:

1. ADR 0052 sections 9, 11, 12, Consequences, the laundered-streak failure,
   invariant 21, and References;
2. this contract plus `cache-workflow-boundaries.md` and
   `execution-manifest-binding.md`, and the qualification command boundary in
   `make-target-compatibility.md`;
3. Spec 003 `spec.md`, `research.md`, `data-model.md`, `plan.md`, `tasks.md`,
   and `quickstart.md`; and
4. the executable predicate and resolver in
   `packages/xtask/src/bazel_qualification.rs`, their tests in
   `packages/xtask/tests/bazel_qualification.rs`, and the workflow carrier in
   `packages/xtask/tests/policy_ci.rs`; the normative-site inventory lint is
   in `packages/d2b-contract-tests/tests/policy_docs.rs`.

The consistency lint enumerates every item rather than globbing and fails on a
missing, extra, or duplicate normative/executable site. All executable
qualification, cold-sample, streak, and promotion decisions call the one
`qualification_decision` implementation in `bazel_qualification.rs`; JSON or
workflow adapters may not restate it.

Committed fixtures under
`packages/xtask/tests/fixtures/qualification/` cover one passing complete
lineage and independent negatives for missing policy, missing fixture, failed,
cancelled, wrong commit, stale reused job, forged ID, ineligible event,
omitted eligible push, and omitted reset. The failed fixture is parameterized
over Cargo, Bazel, policy, fixture, and each slice carrier. The cancelled
fixture includes a fully cancelled eligible push so no implementation can
revive the former "no record" loophole. Every fixture first passes schema and
resolver-input parsing, then fails only its named canonical decision. These
tests run in the existing enforcing Layer-1 Rust carrier
`make test-rust-main`; the closed normative-site lint runs in the existing
enforcing `make test-policy` carrier. The fixture lane is still only the
same-commit runtime companion and does not own these policy tests.

## Promotion and retirement

Promotion references the qualification digest without altering the qualified
record. After the ordered protected-`v3` maintenance and save run,
`promotion-record.json` records the promotion SHA, the qualification digest,
the cache deletion, synchronous trim, headroom, and save results, the first
promoted verdict, and the rollback rehearsal reference.

`post-promotion.json` records release containment and promoted green run IDs.
Alias removal requires only a release containing the promotion commit. Cargo
implementation retirement independently requires only ten consecutive green
promoted `v3` runs, and removes implementations only: every public
`make test-rust` and `make test-rust-<leaf>` name survives and continues to
invoke the authoritative Bazel carriers. Neither child waits for the other, and
neither condition may be inferred from elapsed time.
