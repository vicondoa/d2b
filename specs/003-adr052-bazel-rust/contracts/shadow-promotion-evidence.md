# Shadow and Promotion Evidence Contract

Evidence summaries are internal migration artifacts under
`specs/003-adr052-bazel-rust/evidence/`. They contain immutable references and
computed outcomes, not full logs, panel transcripts, credentials, or
attestation payloads.

## Shadow record

Each record identifies one PR targeting protected `v3`, its tested commit,
resulting merge commit, Cargo/Bazel run IDs, their rollup verdicts, four Bazel
slice verdicts, manifest references, complete-job duration, cache writes, and
effective workflow permissions. A canceled, skipped, unmerged,
mismatched-commit, or incomplete run is not comparable and breaks a
consecutive streak.

## Qualification record

`qualification.json` is `qualified` only with:

1. coverage guard passing for exactly eighteen rows;
2. ten consecutive matching protected-`v3` Cargo/Bazel verdicts;
3. eighteen seeded failure records, each failing only its owning surface;
4. exact test, ignored, doctest, harness-free, API, schema, scanner, and
   pinned-test censuses;
5. main/guest per-case and broker exclusive per-binary topology proofs;
6. twenty consecutive passes for each broker feature suite;
7. three valid warm-local, three cold-local, and the five most recent
   qualifying cold-CI measurements from PRs merged into `v3`;
8. identical enforcing supply-chain outcomes for all three locks;
9. zero shadow cache restore/save/publication;
10. positive and all required negative workflow/cache/deadline/cleanup
    controls passing.

Candidate-specific coverage, seeded-failure, topology, local-performance, and
supply-chain evidence binds one integrated candidate commit. Historical shadow and merged-PR cold-CI samples each retain their tested
commit, `v3` merge commit, and run ID. A candidate content change invalidates
affected candidate-specific evidence. `qualification.json` is reviewed and
merged before promotion work begins and is immutable after qualification.

## Promotion and retirement

Promotion references the qualification digest without altering the qualified
record. After the ordered protected-`v3` maintenance/save run,
`promotion-record.json` records the promotion SHA, qualification digest,
cache deletion/headroom/save results, first promoted verdict, and rollback
rehearsal.

`post-promotion.json` records release containment and promoted green run IDs.
Alias removal requires only a release containing the promotion commit. Cargo
implementation retirement independently requires only ten consecutive green
promoted `v3` runs. Neither waits for the other, and neither
condition may be inferred from elapsed time.
