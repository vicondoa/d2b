### Added

- `cargo xtask spec-registry` regenerates `docs/specs/ADR-046-spec-set.json`
  and `docs/specs/ADR-046-work-items.json` from the specification Markdown,
  deriving every content digest, status, dependency edge, and work-item field
  from the source of truth instead of a hand-maintained copy.
- `cargo xtask implementation-graph` regenerates
  `docs/specs/ADR-046-implementation-graph.json` and its Markdown rendering
  from the two manifests plus the recorded wave topology, including
  topological ranks, parallel groups, and the critical path.
- `cargo xtask test-runtime-ledger` records and enforces absolute hermetic
  execution budgets for individual tests and crates, plus a source lint for
  non-hermetic placement and wall-clock reads. It holds no baseline and makes
  no historical-regression claim; a real multi-crate shard inventory and a
  cross-machine regression baseline are the deferred follow-up
  `runtime-ledger-full-census-and-real-shards`.
- `make test-runtime-ledger` warm-builds the census crates pinned in
  `tests/runtime-ledger-census.json`, records their execution-only per-test
  and per-crate timings into a portable ledger, and fails closed on any
  per-test or per-crate budget violation.
- A fail-closed contract test independently re-derives the Gate 0 bijection
  between the specification Markdown, both manifests, and the implementation
  graph, and asserts the generated artifacts are byte-portable.
- Both the generator and the contract test pin the closed corpus: the member
  and work-item counts, the per-spelling work-item heading census, the
  reuse-action domain, and the derived graph shape. A parser or source
  regression that narrows the corpus or moves the schedule now fails instead
  of quietly rewriting the manifests.

### Changed

- The generated-artifact drift gate now regenerates and diffs the three
  ADR 0046 manifests, so a specification edit that is not accompanied by a
  regeneration fails the gate.
- Regenerated the ADR 0046 manifests and implementation graph against the
  current specification tree: every member now records the accepted status
  and its current content digest, and the panel binding text follows the
  amended provider policy.
