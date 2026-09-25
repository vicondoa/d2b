# Flows

The four routes a user of this set actually walks, in the form the map
summarizes: the situation, the ordered skills, and the handoff signal that
moves from one step to the next. The choice between two skills that both seem
right is settled at the end of this file.

One skill deliberately joins no flow: `/rust-macros` is reached from a
decision — "should this be a macro?" — not from a stage of work, and
forcing it into a flow would misrepresent when it fires.

## Starting fresh in a Rust repo

**Situation.** A new Rust repo — or an existing one the set has not yet been
pointed at — and the question is what to do first and in what order.

**The route.**

1. `/setup-rust-skills` — configure the lint and format settings and record
   the project posture: edition, MSRV, async runtime, `no_std`, and the unsafe
   policy. **Handoff:** the recorded posture exists in the repo and a re-run
   of the skill reports nothing to change; from here every other skill in the
   set verifies at the level the repo set. An optional next step, once the
   lints exist, is `/setup-rust-ci`, which makes CI run what the skills run
   locally.
2. The craft skills, as the code demands them — `/type-driven-design` while
   the model is still soft, `/rust-errors` at the first fallible boundary,
   `/rust-api-design` before the first published version, and the rest by the
   questions they raise. **Alongside, not after:** `/rust-testing` writes the
   test for each behaviour change as it lands, so the suite never trails the
   code, and `/rust-docs` writes the doc comment for each public item as its
   signature settles, so the contract never trails the code either.
   **Handoff:** the code compiles clean at the repo lint level, and the
   questions a review would route to the craft skills are already answered.
3. `/rust-code-review` — before merge. **Handoff:** the report leads with a
   verdict and nothing blocking is open.

## Reviewing someone else's code

**Situation.** A diff, branch, or pull request that someone else wrote, and
the question is whether it is ready to merge.

**The route.**

1. `/rust-code-review` — first, and for most reviews the only entry point: it
   runs the machine before either pass reads a line, runs the standards and
   spec passes separately, and dispatches every smell to the craft skill that
   owns the standard. **Handoff:** the report is out, leads with a verdict,
   and every finding that needs depth already names the skill that owns it.
2. A craft skill directly — only when the review already named it and the
   depth is wanted: the finding points at `/ownership-not-clone` and the
   question is the clone decision, not the one-line fix. **Handoff:** the
   question the finding raised is settled by the rules of the named skill.

## Porting from another language

**Situation.** A Rust implementation is replacing an existing one — C, C++,
Python, TypeScript, Go, or Java — either outright or behind a boundary that
stays, and the question is how to sequence the work so the port does not
drift from the behaviour of its source.

**The route.**

1. `/port-to-rust` — the end state, the parity contract, the seam, and the
   phase sequence; for a Python source, `/port-from-python` alongside it,
   and for a TypeScript or JavaScript source, `/port-from-typescript`,
   and for a Go source, `/port-from-go`, and for a Java source,
   `/port-from-java`, and for a C++ source, `/port-from-cpp`, and
   for a C source, `/port-from-c`, for the construct mapping and the
   boundary.
   **Handoff:** a written contract and a named seam exist.
2. `/rust-testing` — the differential harness against the existing
   implementation: the same inputs through both, the outputs compared, the
   differences recorded. **Handoff:** the harness fails when the ported code
   diverges, so a drift is a report rather than a production incident.
3. The craft skills on the Rust side, as the ported code raises their
   questions — the decision table in `SKILL.md` settles which one a given
   question belongs to — then `/rust-code-review` before merge, over the
   ported diff. **Handoff:** the ported code reads like Rust rather than
   like a translation, at the level the repo configured, and the report
   leads with a verdict and nothing blocking is open.
4. `/rust-ffi` — at the end, for the case where the port leaves a
   permanent boundary rather than replacing the source outright: the seam
   the other language keeps calling across is designed, not discovered —
   the layer only translates, no panic crosses, and every pointer states
   its ownership. The end-state decision in `/port-to-rust` is what tells
   you whether it applies. **Handoff:** the other side loads the seam and
   exercises it, create to destroy, under the platform leak checker, so
   the boundary is checked on both sides rather than one.

## Making working code production-ready

**Situation.** The code works — the feature is in and the tests pass — and
the question is what stands between it and production: a measurement says
it is too slow, the work needs to happen in parallel, a failure in
production has to be diagnosable, a type crosses a wire format, and the
crate is about to be published.

**The route.**

1. `/rust-performance` — when a measurement says the code is too slow: the
   profile is the entry ticket, the change is the one the measurement named,
   and the codegen flags stay the last five percent. **Handoff:** a re-run of
   the benchmark shows the number moved where the profile said it would.
2. `/rust-concurrency` — when the work needs to happen in parallel: the shape
   of the workload picks the model — data parallelism, scoped threads,
   channels, shared state last — and the weakest correct atomic ordering
   wins. **Handoff:** the shape has picked a model, and any ordering in use
   is the weakest one whose argument fits in two sentences.
3. `/rust-observability` — so a failure in production is diagnosable: events
   with named fields, spans for the unit of work, the error chain logged once
   at the boundary that handles it, and no secret in a field. **Handoff:** a
   failure is a query over named fields, not a regex over a formatted string.
4. `/rust-serde` — where the crate has a wire format: the shape a type
   takes on the wire is settled before the code is depended on by anything
   the crate does not control, so a re-shape afterwards is a bug that
   surfaces in a system the crate does not run. **Handoff:** every type
   crossing the wire round-trips a real payload from the other side, not
   just its own output.
5. `/rust-docs` — before the crate is published: the doc comment states the
   contract a caller must know, the canonical sections carry the failure
   modes, and the doctests run under `cargo test`. **Handoff:** `cargo doc`
   is clean and the doctests pass, so the documented contract is the one the
   code keeps.

## When two skills both seem right

The tie-breaker is which one names the decision as its defining constraint.
That is on each `docs/` page under `## What it does`: read the two defining
constraints, and the skill whose defining constraint matches the decision in
front of you is the one to run. The "not to be confused with" column in the
decision table is the same test, pre-computed for the pairs that actually get
confused.
