# Research: ADR 0052 Bazel Rust Gate

This document consolidates accepted ADR 0052 decisions and resolves the few
plan-level choices needed to execute them. Committed passing code remains the
baseline. No item is open.

## Decision 1: Keep the migration Rust-only and reversible

**Decision**: Bazel schedules only the eighteen Rust execution-manifest
surfaces behind `test-rust`. It does not migrate Nix evaluation, packaging,
images, fixtures, releases, static guest binaries, cross-compilation, remote
cache/execution, or another Layer-1 job.

**Rationale**: Rust scheduling is independently valuable and independently
revertible. Mixing it with Nix, linkage, image, or release evidence would make
coverage regressions harder to isolate.

**Alternatives**:

- Rust and Nix together: rejected as four coupled risk surfaces.
- Bazel only in CI: rejected because contributors could not reproduce it.
- Big-bang replacement: rejected because equivalence evidence requires both
  paths.

## Decision 2: Cargo and committed toolchains remain authoritative

**Decision**: The three Cargo workspaces, locks, policy files, Rust 1.97.0
pin, and `nightly-2026-02-16` API pin remain the only dependency, feature,
policy, and compiler authorities. `crate_universe` imports committed locks in
non-vendored mode with committed Bazel-side locks. `gen-bazel --check`,
toolchain equality, and repin drift fail closed.

Bazel is exactly 8.6.0 from pinned nixpkgs. `.bazelversion` is a checked
declaration, not a downloader. Bzlmod lock mode is `error`. The initial
`rules_rust`/`crate_universe` pin is 0.73.0, the ADR's accepted-date release.
W0 measures it with Bazel 8.6.0. If incompatible, W0 may pin only the highest
stable version proven compatible at that time and must record the measurement;
there is no floating or fallback resolution.

**Rationale**: Dependency changes must remain Cargo edits followed by
regeneration. A second authority would permit feature or compiler drift.

**Alternatives**:

- Bazel-authoritative dependency declarations: requires another ADR.
- Bazelisk download: rejected as unpinned tool substitution.
- Vendored `crate_universe` BUILD tree: rejected as a large committed tree
  without additional determinism.

## Decision 3: Generate first-party BUILD files in repository-owned xtask

**Decision**: `cargo xtask gen-bazel` reads `cargo metadata` for all three
workspaces and emits generated first-party BUILD files and the exact governed
Rust source manifest. `--check` regenerates in scratch and is wired into
existing `test-drift`. Hand-written fragments are allowed only when listed by
the coverage map.

**Rationale**: The repository already uses generator plus drift-check
ownership. It handles standalone locks, broker feature variants, harness-free
targets, doctests, and API census details without another trusted toolchain.

**Alternatives**:

- `gazelle_rust`: rejected because it adds Go and a third-party generator for
  cases that need repository-specific treatment.
- Hand-author every BUILD file: rejected because Cargo metadata drift would be
  review-only and fail open.
- Globs for governed sources: rejected because the scan and its declaration
  could be incomplete in the same way.

## Decision 4: Make the coverage map exact and one-to-one

**Decision**: `tests/golden/bazel-rust-coverage.json` maps exactly these IDs to
one carrier, one slice, one exact census, and one topology where applicable:

1. `rust-api-surface`
2. `rust-main-format`
3. `rust-main-clippy`
4. `rust-main-workspace-tests`
5. `rust-no-bash-ast`
6. `rust-schema-reproducibility`
7. `rust-stub-no-socket`
8. `rust-assert-pinned`
9. `rust-broker-default`
10. `rust-broker-layer1`
11. `rust-broker-fakebackends`
12. `rust-guest-shell-runner`
13. `rust-deny-main`
14. `rust-deny-broker`
15. `rust-deny-guest`
16. `rust-audit-main`
17. `rust-audit-broker`
18. `rust-audit-guest`

The fixed slices are `api`, `main`, `broker`, and `aux`. The guard rejects a
missing/duplicate ID, nonexistent carrier, unmapped Rust test, missing
topology/census, empty discovery, or unlisted hand-written fragment.
`rust-contract-tests` and `rust-cli-contract-tests` are explicitly excluded
and remain fixture-lane Cargo/Nix surfaces.

**Rationale**: Exact coverage catches a green result caused by omission.
Minimum counts cannot detect a missing file offset by a new one.

**Alternatives**:

- Target-count floor: rejected as fail-open.
- Infer coverage only from Bazel query: rejected because the graph cannot
  prove it represents the baseline contract.
- Include fixture surfaces: rejected because evaluated Nix fixtures are
  outside this ADR.

## Decision 5: Preserve the three existing test topologies

**Decision**: A repository-owned Rust runner enumerates libtest cases and
enforces:

- main workspace: one fresh process per test case;
- guest shell runner: one fresh process per test case;
- broker feature suites: one process per binary, bounded internal threads,
  all three targets tagged `exclusive`.

Ignored cases remain ignored and counted. Doctests and harness-free companions
remain separate derived targets and fail if discovery unexpectedly becomes
empty. The runner invokes no shell and uses `D2B_RUST_BUDGET` to prevent
scheduler and suite concurrency from multiplying.

**Rationale**: Plain `rules_rust` `rust_test` would weaken main and guest
isolation. Broker process-per-test is known to expose live sysfs behavior and
is deliberately not adopted.

**Alternatives**:

- Plain `rust_test`: rejected as a topology regression.
- Convert broker to per-case: rejected pending a dedicated isolation review.
- Custom Bazel local resources: rejected because measurement showed no
  serialization under Bazel 8.6.0.

## Decision 6: Split offline supply-chain policy by capability

**Decision**: For each lock, `cargo-deny` enforces `bans licenses sources`
against a declared cargo-vendor-shaped tree with `CARGO_NET_OFFLINE=1`.
`cargo-audit` alone enforces advisories against a committed RustSec snapshot
with `--no-fetch` and the current workspace-specific ignores. No Bazel action
may use the network. Promotion compares the union of findings with the current
Cargo outcome across all three locks.

Schema reproducibility gains `gen-schemas --out-dir`, performs two sequential
independent generations in one action, and checks the exact twenty-file,
nonempty, valid-JSON census before digest comparison. The no-bash carrier uses
the generated 641-file governed manifest, requires runfiles equality in both
directions, and requires parsed count to equal declared count.

**Rationale**: `crate_universe` repositories are not a Cargo registry, and
`cargo-deny` advisories fetch. The split preserves the aggregate policy while
making actions offline. Exact census prevents empty-success behavior already
present in the current schema leaf.

**Alternatives**:

- Let actions fetch: rejected as non-hermetic and unauthorized.
- Run all cargo-deny checks offline: rejected because advisories still require
  the configured database behavior.
- Two identical schema actions: rejected because the second could be an action
  cache replay rather than an independent invocation.

## Decision 7: Bind Bazel results to execution-manifest v1

**Decision**: The Make wrapper maps Bazel Build Event Protocol results onto the
existing v1 surface IDs, `completed_leaves`, `failed_surfaces`, run status, and
partial evidence semantics. The existing reference and schema remain
authoritative and are not redefined. Prior evidence is invalidated before
dispatch; failure and handled interruption publish partial evidence.

**Rationale**: Source discovery proves availability, not execution. Keeping
the schema stable allows Cargo and Bazel evidence to be compared directly.

**Alternatives**:

- A Bazel-specific manifest version: rejected because executor identity does
  not change the evidence contract.
- Logs only: rejected because logs do not provide atomic, comparable partial
  evidence.

## Decision 8: Use bounded local scratch with one measurement mode

**Decision**: Output user root, action cache, and repository cache persist only
under `.scratch/bazel/`. Action cache uses 8 GiB, 14 days, and zero idle GC
delay. Repository cache is capped at 2 GiB. Output-root marks are 20/40 GiB.
`make clean` owns reclamation under existing scratch controls.

The evidence-only
`cargo xtask bazel-evidence prepare-cold-local` helper prepares a fresh output
user root and empty action cache while retaining a populated repository cache.
It exists because the ADR defines a cold profile but no safe existing command
can produce that state without either preserving all scratch or deleting the
download cache. It is not a Make target or contributor compatibility contract,
and W5 removes it after W4 qualification.

Cleanup first shuts down the matching server, anchors `.scratch/` once, rejects
symlinks, magic links, escapes, tracked files, live ownership, and replacement
races, and deletes descriptor-relative with close-on-exec descriptors.
Refusal deletes nothing and gives its code-specific recovery.

**Rationale**: Persistent reuse is the local performance benefit, but an
unbounded or path-based cleanup can fill disk or delete unrelated data.

**Alternatives**:

- Shared worktree cache: rejected because worktrees own independent state.
- `realpath` then recursive remove: rejected for the check/use race.
- Raw manual removal for cold measurement: rejected because it bypasses the
  cleanup contract.

## Decision 9: Keep shadow CI credential-free and cache-free

**Decision**: One non-required workflow runs four parallel slices plus rollup.
It calls only approved Make targets, uses the mandated shell and credentialless
checkout, and runs on the amended ADR triggers/path filters. Shadow restores
and saves nothing. PR-reachable jobs have `contents: read`, no
`actions: write`, and no direct, indirect, post-step, or unknown cache writer.
The cold-CI set is the five most recent qualifying shadow runs for PRs merged
into protected `v3`, each bound to its PR, run ID, tested commit, and merge
commit. No default-branch schedule is used.

At promotion, action and download snapshots become separate 4/1 GiB entries.
Output base is never cached. Keys bind Bazel/module/config, both toolchains,
all locks and deny files, advisory pin, and generated BUILD digest. PRs restore
read-only; exactly one protected-`v3` job writes. Cache credentials exist only
inside cache actions and never in a `run:` or Bazel environment.

**Rationale**: The measured cache had only about 0.87 GiB headroom. Shadow
publication would evict caches used by the required Cargo path, and exposing a
cache token to Bazel exposes it to third-party build code.

**Alternatives**:

- Publish shadow cache: rejected due eviction risk.
- Cache output base: rejected as path/server/machine-specific.
- Remote cache or execution: rejected as out of scope and incompatible with
  the credential boundary.

## Decision 10: Use fixed performance and deadline semantics

**Decision**: A profile passes only when its median is at/below the ceiling and
no sample exceeds 1.2 times it. Local sets contain three consecutive runs; CI
uses the five most recent qualifying cold shadow runs for PRs merged into
`v3`. Warm is a successful run, one comment-only edit to
`d2b-core/src/lib.rs`, then a second run with the server live. Cold definitions
and complete CI job window are exactly those in the amended ADR.

Promoted CI reads `/proc/uptime` after checkout through one checked fixed-point
parser. Capture truncates, read rounds up, child duration rounds down. It
exports an absolute boot-relative integer-millisecond deadline of
`anchor + 780000`; checkout has a separate 2-minute bound. Missing deadline is
allowed locally and forbidden in promoted jobs. Expired is a normal budget
expiry. Malformed/overflowing values fail without echo.

The wrapper creates a dedicated child process group without a shell, sends
SIGTERM, waits the fixed grace in full, observes with
`EXITED|NOWAIT|NOHANG`, sends unconditional group SIGKILL, then reaps. It never
signals its own group or a server PID read from a file. Server shutdown uses
matching Bazel startup options and its own bound.

**Rationale**: A job timeout alone is unactionable. Conservative conversion
and group ownership make the ceiling a real upper bound without orphaning work
or signalling unrelated processes.

**Alternatives**:

- Float/raw uptime value: rejected for rounding and grammar ambiguity.
- Absolute deadline passed as duration: rejected as a silent many-hour bound.
- Reap or end grace on leader exit: rejected because descendants can survive.
- Relax a missed ceiling: not authorized. Only a larger runner or further
  disjoint slice split is allowed.

## Decision 11: Stage promotion, alias removal, and retirement separately

**Decision**: Cargo remains authoritative through shadow and W4 evidence.
Promotion requires exact coverage/census/topology, ten matching `v3`
verdicts, an eighteen-case isolated failure matrix, twenty broker repetitions,
all performance sets, supply-chain equivalence, and cache-policy evidence.

W4 commits one immutable qualification record. W5 writes a separate promotion
record after the ordered protected-`v3` maintenance/save run. Release
containment and the ten-green-run clock are recorded separately after
promotion: alias removal depends only on release containment, while Cargo
implementation retirement depends only on ten consecutive promoted green
runs. Neither retirement path waits for the other.

Cache cutover is ordered: pre-merge audit and protected-`v3` freeze establish
the candidate set; promotion stops old writes; protected-`v3` maintenance
deletes only authorized prefixes with complete pagination; usage plus planned
snapshot is checked at most 8 GiB, checked again immediately before save, then
one writer publishes. Maintenance verdict remains separate from Rust.

Promotion keeps required context `test-rust`, keeps eight old leaf names,
turns Bazel names into status-preserving aliases, retains Cargo fixture mode,
regenerates the required workflow, deletes shadow, and removes the
evidence-only cold-local preparation helper. Alias removal waits for one
containing release and is a separate change. Cargo implementation retirement
waits for ten consecutive green promoted runs and is another independent
change.

**Rationale**: Evidence collection cannot be combined with the irreversible
executor flip. Separate compatibility and implementation retirement preserve
rollback and distinguish naming failures from executor failures.

**Alternatives**:

- Promote while collecting evidence: rejected as circular.
- Delete aliases at promotion: rejected as an avoidable contributor break.
- Retire Cargo at promotion: rejected because rollback would require
  reconstruction.

## Decision 12: Use existing delivery and test surfaces

**Decision**: Behavioral guards live in the owning Rust crate,
source-shape cleanup guards extend `policy_docs.rs`, workflow/cache guards
extend `policy_ci.rs`, and generated drift extends `test-drift`. Each ships
with a positive and observable negative mutation. No host/manual tier is used.
Every wave has the full ten-role plan and diff panels, unanimous and bound to
one snapshot. This plan does not use pipelined dispatch.

**Rationale**: These are hermetic build-infrastructure properties. Existing
Layer-1 carriers are both lower and more enforcing than container, VM, live,
or hardware tiers. Strict serialization avoids invalidating work that consumes
new generated contracts or evidence.

**Alternatives**:

- New shell gate or Layer-1 job: rejected by test-layer policy and ADR 0017.
- Treat green tests as review: rejected because review and tests detect
different failure classes.
- Pipeline later waves after five reviews: constitutionally permitted but
  rejected here because the saved time does not outweigh contract/evidence
  rework.

## Decision 13: Amend branch authority before implementation

**Decision**: ADR 0052 is amended before W0. Protected `v3` becomes the sole
promotion, cache-maintenance, cache-publication, shadow-streak, and
post-promotion observation lineage. The cold-CI profile uses the five most
recent qualifying Bazel shadow runs for PRs merged into `v3`, rather than a
weekly run from the GitHub default branch.

**Rationale**: The repository's GitHub default branch is `main`, while binding
repository policy says `v3` is the clean-break integration lineage and never
merges to `main`. GitHub schedules execute from the default branch, so the
accepted ADR's default-branch schedule and writer language cannot implement
the selected `v3` migration without an explicit correction.

**Alternatives**:

- Change the GitHub default branch to `v3`: rejected as a repository-wide
  governance change outside this feature.
- Promote on `main`: rejected because the feature and release lineage is
  `v3`, which never merges to `main`.
- Add a `main` scheduler that dispatches `v3`: rejected in favor of using the
  most recent merged-PR gate evidence already produced by the protected
  lineage.
