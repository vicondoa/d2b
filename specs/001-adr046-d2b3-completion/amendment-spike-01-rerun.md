# Amendment request: authoritative SPIKE-01 RSS rerun

| Field | Value |
| --- | --- |
| Scope | Replace the failed SPIKE-01 RSS evidence with a separately recorded gated rerun, without rewriting either prior result artifact |
| Raised under | W5 audit F4 and FR-056 |
| Affected member spec | `ADR-046-validation-and-delivery`, section 3.2, plus the linked feasibility and store evidence rows |
| Affected policy | `packages/d2b-contract-tests/tests/policy_adr046_spec_literals.rs` |
| Status | **Applied.** The member amendment, regenerated manifests, and policy-literal update landed together; the spec panel is still owed |
| Gate 0 | Re-evaluated per FR-056 in [`gate0-reevaluation-spike-01-rss-rerun.md`](./gate0-reevaluation-spike-01-rss-rerun.md); mechanical half discharged, human-review half invalidates the outstanding `ADR046-W5` panel request |

## 1. Authority and supersession

The existing `proofs/redb-resource-store-spike/RESULTS.md` remains the
historical failed record. Its median whole-process maximum RSS result is
25,216 KiB against the unchanged 24,576 KiB threshold.

`proofs/redb-resource-store-spike/RESULTS-corrections.md` remains a
non-authoritative corrections prototype. It is not a rerun of record and must
not be cited as evidence that the gate passes.

The new artifact
`proofs/redb-resource-store-spike/RESULTS-rerun-2026-08-02.md` supersedes the
RSS conclusion in `RESULTS.md` while preserving `RESULTS.md` and
`RESULTS-corrections.md` byte-for-byte. It records a median of `18,428 KiB`
for the hard fixture, with `6,148 KiB` of headroom below `24,576 KiB`. It is
not a claim that the production backend is accepted.

## 2. Completed measurement contract

The hard fixture is unchanged:

```text
rss-fixture --resources 10000 --watches 100
```

The metric is GNU `time -v`'s complete child-process line:

```text
Maximum resident set size (kbytes)
```

The reported value is the whole-process maximum RSS. No empty-process,
runtime, allocator, or other baseline is subtracted. The hard threshold is
exactly `24,576 KiB`.

The repository's established fixture sweep is the command recorded in
`RESULTS.md`: build `rss-fixture` in release mode, run the empty, 10,000
resource/zero-watch, and 10,000 resource/100-watch shapes three times each,
and take the median of the three hard-fixture values. The rerun used the same
fixture, release binary, `TMPDIR` shape, `time -v` field, and sample method.
The hard-fixture median was `18,428 KiB`.

The measurement command was the child of the public heavy gate, using the
repository-supported arbitrary-command form:

```text
cargo run --quiet --manifest-path packages/Cargo.toml -p xtask -- \
  heavy-gate -- bash -lc '<the established RESULTS.md RSS command>'
```

The command recorded only privacy-permitted reproducibility metadata:
Rust and Cargo versions, kernel/architecture, filesystem type and options,
CPU count, load average, free memory, memory-pressure values, and swap state.
It recorded the quiet-machine precondition beside the result. It did not
record hostnames, user names, process IDs, credentials, or host-specific paths.

The current repository policy says not to clear `RUSTC_WRAPPER` or
`CARGO_BUILD_RUSTC_WRAPPER`; the historical command in `RESULTS.md` predates
that policy. The rerun retained the fixture and accounting method while
following the current wrapper policy. This command drift is called out in the
result artifact rather than hidden.

## 3. Exact member-spec amendment after a passing gated rerun

The separate amendment must make these semantic replacements. For this rerun,
`<R>` is `18,428 KiB` and `<H>` is `6,148 KiB`. No rounded MiB value may
replace either literal.

1. In `docs/specs/ADR-046-validation-and-delivery.md` section 3.2, change the
   `ADR046-W1` row from:

   > the failed RSS result defers the production backend, watch dispatcher,
   > and real-backend reaction benchmark

   to:

   > the corrected SPIKE-01 RSS rerun is recorded in
   > `proofs/redb-resource-store-spike/RESULTS-rerun-2026-08-02.md` at
   > `18,428 KiB`, below the unchanged 24,576 KiB whole-process gate; the
   > production backend, watch dispatcher, and real-backend reaction benchmark
   > remain W5 implementation work and still require their own production
   > validation

2. In the `ADR046-feasibility-001` row in the same section, replace the
   historical failure sentence with:

   > The original `RESULTS.md` failure is superseded for this RSS row by the
   > gated rerun at `18,428 KiB`, `6,148 KiB` below 24,576 KiB, with no baseline
   > subtraction. The rerun is spike evidence only and does not make the
   > production backend reachable or accepted.

3. In the same section, keep the W5 assignments and production acceptance
   dependencies. A passing disposable proof does not move
   `ADR046-store-004`, `ADR046-store-002`, `ADR046-store-005`, or
   `ADR046-reconcile-003` to `Merged`, and it does not satisfy Gate 0 by
   itself.

4. Update the corresponding failure-versus-pass evidence prose in
   `ADR-046-feasibility-and-spikes`, `ADR-046-resource-store-redb`, and D128
   in `ADR-046-decision-register`. Preserve the statement that production
   acceptance requires the production backend's own conformance, security,
   durability, watch-budget, backup/migration, and reaction evidence.

5. Regenerate `ADR-046-spec-set.json`, `ADR-046-work-items.json`, and
   `ADR-046-implementation-graph.json` with the repository generators. Do not
   hand-edit generated manifests. The resulting digest change is a mechanical
   Gate 0 consequence, not Gate 0 acceptance.

6. Keep the historical failure in `CHANGELOG.md` as history unless the
   release-note owner approves a separate correction entry. Do not rewrite
   history merely to make a global literal count pass.

## 4. Exact policy-literal amendment after acceptance

`policy_adr046_spec_literals` currently treats `RESULTS.md` as the sole
canonical source for all seven spike rows and pins the failed RSS literals.
The policy amendment must preserve that historical source for the six
unchanged rows and bind only the RSS measurement to the new result artifact.

The policy change must:

1. Add a result-source field to the measurement specification, or an
   equivalent closed selector, so the six non-RSS measurements still read
   `proofs/redb-resource-store-spike/RESULTS.md` while `median RSS` reads
   `RESULTS-rerun-2026-08-02.md`.
2. Change the RSS expected outcome from `MEASURED-FAIL` to
   `MEASURED-PASS`.
3. Replace the RSS fingerprint and its inventory patterns for
   `25,216 KiB`, `24.625 MiB`, `640 KiB`, and `2.6% above 24,576 KiB` with
   the exact `18,428 KiB` and `6,148 KiB below 24,576 KiB` wording emitted by
   the new result artifact. The policy must not accept a rounded or
   baseline-subtracted variant.
4. Replace every registered failure summary in the feasibility, store,
   decision-register, validation, and generated work-item documents with the
   accepted rerun summary. Keep any explicitly retained historical Changelog
   copy registered at its actual count; do not add a suppression marker.
5. Change the mutation fixture for the RSS row from the old `RESULTS.md`
   failure literal to the new result artifact and ensure that mutating either
   the result row or a registered derived copy fails closed.
6. Continue to exclude `RESULTS-corrections.md` from authority. A prototype
   result must not satisfy the canonical measurement parser.

The accepted member amendment, regenerated manifests, policy-literal update,
Gate 0 mechanical check, architect decision, and required panel acceptance are
separate dependencies. This draft does not claim any of them.
