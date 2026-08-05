# Shadow and Promotion Evidence Contract

Evidence summaries are internal migration artifacts under
`specs/003-adr052-bazel-rust/evidence/`. They contain immutable references and
computed outcomes, not full logs, panel transcripts, credentials, or
attestation payloads.

## Qualification record

A qualification record is a `push` event on `refs/heads/v3` produced by a
merged pull request. Each record identifies:

- the head commit, which is identical for both workflow runs because the
  required Cargo workflow also triggers on `push` for `v3`;
- the Bazel shadow workflow run ID and the required Cargo workflow run ID;
- both rollup verdicts, where the Cargo verdict is
  `D2B_SKIP_FIXTURE_BUILD=1 make test-rust`;
- the same-commit `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`
  verdict, which must be passing;
- the four Bazel slice verdicts and, for a cold-sample record, the four
  complete slice job durations;
- manifest references, cache restore and write counts, and effective workflow
  permissions.

Pairing is on the head commit under the push event, never on a pull-request
number: `refs/pull/N/merge` is recomputed against a moving base, so two
workflows triggered by the same pull request can legitimately test different
trees. Pull-request, `main`-push, scheduled, and dispatched runs are
diagnostic. They never enter a streak or a measurement set.

The fixture lane is not compared between executors, because its two surfaces
stay explicitly outside this migration. It is a required companion verdict so a
qualification record cannot hide a contract-layer regression.

### Streak arithmetic

- Matching verdicts with a passing fixture verdict extend the streak.
- Differing verdicts reset the streak to zero.
- A Bazel run that reaches no verdict while its paired Cargo run reaches one is
  a mismatch and resets the streak, because otherwise cancelling a run about to
  go red would launder the streak.
- A push where neither side reaches a verdict, which is what a superseding push
  produces, is not a record: it neither extends nor resets.

### Cold-sample qualification

A record supplies a cold continuous-integration measurement only when no Bazel
cache of any kind was restored and all four slice jobs ran to completion with a
recorded duration. Its scalar record duration is the maximum of those four
slice durations, matching the workflow critical path. During the shadow stage
every run is cold by construction,
because nothing is published or restored, so the qualifier excludes runs that
produced no measurement rather than selecting among warm and cold runs.

## Qualification record set

`qualification.json` is `qualified` only with:

1. both halves of the coverage guard passing for exactly eighteen rows, the
   analysis-time half and the out-of-test completeness half;
2. ten consecutive matching qualification records, each with one shared head
   commit and a passing fixture-contract verdict;
3. eighteen seeded failure records, each failing only its owning surface;
4. exact generator-derived test, ignored, doctest, harness-free, API, schema,
   scanner, and pinned-test censuses, with every out-of-census entry recorded
   with its reason;
5. main and guest per-case topology, broker exclusive per-binary topology, and
   per-case result publication proofs;
6. twenty consecutive passes for each broker feature suite;
7. three valid warm-local, three cold-local, and the five most recent
   qualifying cold measurements, with the cold continuous-integration set
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
run IDs. A candidate content change invalidates affected candidate-specific
evidence. `qualification.json` is reviewed and merged before promotion work
begins and is immutable after qualification.

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
