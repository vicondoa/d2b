# Research: ADR 0052 Bazel Rust Gate

This document consolidates the amended ADR 0052 decisions and resolves the
plan-level choices needed to execute them. ADR 0052 was amended on 2026-08-03
after an upstream review measured five mechanics that the first draft could not
implement and two supporting statements that were wrong about the substrate.
That amended record is merged and is settled authority here. Committed passing
code remains the baseline. No item is open.

Every version-sensitive claim below is sourced in the "Upstream evidence"
section at the end. Where the review measured something against the pinned
versions, the measurement wins over documentation.

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
- Importing the flake's vendored-Cargo output into Bazel: rejected as
  Bazel-to-Nix packaging, which ADR 0052 section 1 does not decide.

## Decision 2: Cargo and committed toolchains remain authoritative

**Decision**: The Cargo workspaces, their locks, the policy files, the Rust
1.97.0 pin, and the `nightly-2026-02-16` API pin remain the only dependency,
feature, policy, and compiler authorities. `crate_universe` imports committed
locks in non-vendored mode with a committed Bazel-side lock per hub.
`gen-bazel --check`, toolchain equality, and lock drift fail closed.

**Four hubs, not three.** The Rust gate builds four independently resolved
Cargo workspaces, so it declares four `crate.from_cargo` hubs:

| Hub | Cargo lock | What it carries |
| --- | --- | --- |
| main | `packages/Cargo.lock` | The 56-member main workspace |
| broker | `packages/d2b-priv-broker/Cargo.lock` | Three broker feature passes |
| guest | `packages/d2b-guest-shell-runner/Cargo.lock` | Guest shell runner |
| walker | `tests/tools/no-bash-ast-walker/Cargo.lock` | The no-bash AST scanner |

The walker is a standalone workspace with its own manifest and lock. Folding
its `syn` and `walkdir` requirements into the main hub would silently
re-resolve them and destroy the `--locked` equivalence the migration exists to
preserve, so it gets its own hub and its own Bazel-side lock.

`packages/Cargo.guest.lock` is a fifth committed lock. It is a generator input
and a cache-key input because the flake's guest/static path consumes it, but no
Rust gate surface builds against it, so it is deliberately **not** a
`crate_universe` hub.

**Two lock mechanisms, kept separate.** These are different systems and
conflating them loses one of them:

- `MODULE.bazel.lock` pins the module graph, including the transitive registry
  modules `rules_rust` brings in. `common --lockfile_mode=error` is what makes
  a drifted module resolution a failure instead of a silent rewrite, and
  `common --check_direct_dependencies=error` is what keeps a declared direct
  version that the graph absorbs from passing as a warning. This is the
  mechanism that pins the transitive modules; `MODULE.bazel` alone does
  not. Its one regeneration path is `cargo xtask bazel-module-refresh`.
- Each hub's committed Bazel-side lock is the `--locked` equivalent for
  crates. `crate_universe` re-runs its generator query and fails with a named
  remediation when the committed lock is stale. That check exists **only when
  `lockfile = ...` is set on the `crate.from_cargo` tag**; omitting the
  attribute makes repin unconditional and silently removes the guard. All four
  hubs therefore set it, together with `cargo_lockfile` and
  `skip_cargo_lockfile_overwrite = True`.

**No repin escape hatch, and one supported regeneration path.**
`CARGO_BAZEL_REPIN`, `REPIN`, and
`CARGO_BAZEL_REPIN_ONLY` are never set by the Make wrapper or by continuous
integration, and a policy assertion proves it. A repin control in the gate
environment converts the fail-closed lock check into an automatic rewrite.

A prohibition with no supported alternative is a prohibition that gets routed
around: the contributor who hits the stale-lock refusal at the end of a long
change has one obvious next move, and it is `CARGO_BAZEL_REPIN=1 make ...`.
Regeneration is therefore a repository-owned command,
`cargo xtask bazel-repin --hub <main|broker|guest|walker>`, and it is the only
site in the repository where those three names may appear as a
process-environment assignment. Its contract:

- it refuses any hub name outside the closed four-hub set, and it refuses to
  run at all when the ambient environment already carries any of the three
  controls, so it cannot be used to launder a setting a contributor exported;
- it sets `CARGO_BAZEL_REPIN` and `CARGO_BAZEL_REPIN_ONLY=<hub>` only on the
  `Command` it builds for the one Bazel child it spawns, never through a
  process-global mutation;
- it reuses the wrapper's absolute startup options and output user root, so it
  cannot start a second server against the same tree;
- it snapshots the digests of every committed derived artifact first and fails
  unless the named hub's Bazel-side lock is the only tracked file that changed
  afterwards;
- it is not a Make target, and the workflow guard refuses any workflow that
  invokes it.

Three measured facts at `rules_rust` 0.73.0 make that contract enforceable
rather than aspirational. First, `determine_repin` in
`crate_universe/private/generate_utils.bzl` treats `CARGO_BAZEL_REPIN_ONLY` as
a comma-delimited allowlist compared by exact repository name, so single-hub
scoping is a substrate property, not a convention this plan invents. Second,
any value other than `false`, `no`, `0`, or `off` is truthy, so a guard that
looks for `=1` and stops is wrong; the guard rejects the name, not a value.
Third, `skip_cargo_lockfile_overwrite` defaults to `False` in
`crate_universe/extensions.bzl`, which means a repin writes the plain
`Cargo.lock` back by default. That directly contradicts the authority rule this
decision exists to protect, so every hub sets it to `True`, and the command's
post-check catches it independently if a future default changes. For the same
family of reasons every hub also sets both `lockfile` and `cargo_lockfile`: the
extension reports itself reproducible only when both are present, and
`--lockfile_mode=error` only constrains a reproducible extension.

One thing is deliberately not settled here. The extension docstring at 0.73.0
recommends `CARGO_BAZEL_REPIN=1 bazel sync --only=<hub>`, which is a
WORKSPACE-era control, while the bzlmod code default for `regen_command` is
`bazel mod show_repo`, which does not repin anything. The two disagree, so W0
records the invocation observed to actually repin exactly one hub on Bazel
8.6.0 with this module graph, and the command asserts the outcome rather than
trusting the docstring. The invariant that must hold is "exactly one hub lock
changed", and that is checked, not assumed. W0 records that invocation in its
wave notes as a command shape with `<worktree>` placeholders, not as the real
absolute path of the worktree it was measured in; a wave note is quoted forward
into later waves and into review, so a real home directory path in one has
escaped exactly the way an echoed environment value would have.

**The module lock has its own regeneration command, because the diagnostic
that names one names the wrong one.** The four hub locks and
`MODULE.bazel.lock` are different mechanisms, so the module lock needs its own
repository-owned command rather than a mode flip or a hand edit:
`cargo xtask bazel-module-refresh`. Three things measured directly against
Bazel 8.6.0 on a scratch module graph forced its shape.

First, `--lockfile_mode=error` does not fail merely because `MODULE.bazel`
changed. It fails when the resolution needs a registry file whose checksum the
lock does not carry, and it exits 48 with:

```text
ERROR: Error computing the main repository mapping: Missing checksum for
registry file https://bcr.bazel.build/modules/<name>/<version>/MODULE.bazel
not permitted with --lockfile_mode=error. Please run `bazel mod deps
--lockfile_mode=update` to update your lockfile.
```

The lock is not rewritten by the failing run, which is the fail-closed property
the pin depends on.

Second, the invocation that diagnostic names is correct as far as it goes and
dangerous as written. `bazel mod deps --lockfile_mode=update` does update only
`MODULE.bazel.lock`, creates and removes no other file, and changes nothing on
a second identical run. But the line a contributor copies out of the terminal
carries no startup options, so it runs against the default output user root
under the home directory: a second server on the worktree and a second output
base outside `.scratch/`, defeating the workspace-boundary and bounded-scratch
rules in one paste. The repository therefore ships its own command, which
issues that invocation with the same absolute output user root and output base
as everything else, verifies afterwards that `MODULE.bazel.lock` was the only
tracked file that changed, and exits zero having changed nothing when the tree
is already current. `bazel mod` rejects `--symlink_prefix`, so what this child
shares with the build commands is the startup-option set that selects the
server, not every command option.

Third, and separately from the lock, a direct dependency whose declared version
loses to a higher one elsewhere in the graph produces a `WARNING` and exit zero
under `--lockfile_mode=error` alone. That is a check that degrades rather than
denies, so `.bazelrc` also carries `common --check_direct_dependencies=error`.
Without it the module lock faithfully records a resolution nobody declared.

W0 re-measures the update invocation against this repository's real module
graph before the remediation message ships, for the same reason it measures the
repin invocation: the measurement above was taken on a two-module scratch graph
with no `rules_rust` and no `crate_universe` extension.

**The generator binary is a pinned tool too.** `crate_universe` executes a
`cargo-bazel` binary. The registry release form of `rules_rust` carries an
explicit URL and sha256 for it and downloads it with a checksum; a plain git
checkout has empty URL tables and falls back to building the generator from
source through a non-reproducible bootstrap that needs a host `cargo`. The
module therefore consumes the registry release form, the URL and sha256 are
part of the FR-004 no-unpinned-fallback statement and part of the cache key,
its download is charged to the cold profile, and a structural guard refuses the
source-bootstrap fallback.

**Version compatibility is already published, so W0 measures something else.**
`rules_rust` 0.73.0 is on the Bazel Central Registry and its presubmit matrix
tests Bazel 7.x, 8.x, and 9.x, and it declares no restrictive
`bazel_compatibility`. Treating basic compatibility as unknown was wrong. W0
keeps a measurement, scoped to *this repository's* graph: the four hubs, the
three feature variants, the standalone workspaces, and the hand-written
fragments. If that measurement fails, W0 may pin only the highest release
proven compatible at that time and must record the measurement; there is no
floating or fallback resolution.

Bazel is exactly 8.6.0 from pinned nixpkgs. `.bazelversion` is a checked
declaration, not a downloader. Bazelisk is not on the gate path and is not
required in the dev shell; the dev shell instead gains `bazel_8` and
`bazel-buildtools`, and no Bazel target lands before it does.

**Rationale**: Dependency changes must remain Cargo edits followed by
regeneration. A second authority would permit feature or compiler drift, and a
lock check that can be disabled by an environment variable is not a lock check.
A regeneration path that is narrow, reviewed, and self-verifying is not a
weakening of that rule; it is what keeps the rule from being bypassed.

**Alternatives**:

- Bazel-authoritative dependency declarations: requires another ADR.
- Bazelisk download: rejected as unpinned tool substitution.
- Vendored `crate_universe` BUILD tree: rejected as a large committed tree
  without additional determinism, and it produces a Bazel-shaped tree rather
  than the vendor layout the dependency policy tools need.
- One hub for all four workspaces: rejected because it re-resolves the
  standalone locks.
- Omitting `lockfile` and relying on `--lockfile_mode=error`: rejected because
  that flag governs the module lock only and omitting `lockfile` makes repin
  unconditional.
- A documented `CARGO_BAZEL_REPIN=1 make ...` escape hatch: rejected because
  it repins every hub, is indistinguishable in a shell history from an
  accidental export, and at the 0.73.0 default also rewrites the authoritative
  `Cargo.lock`.
- A `make bazel-repin` target: rejected because every Make target is reachable
  from a workflow and the approved-target policy would then have to carve out
  an exception; an `xtask` subcommand is reachable only from a shell a
  contributor typed into.
- Naming Bazel's own `bazel mod deps --lockfile_mode=update` line as the
  module-lock remediation: rejected on the measurement above. That invocation
  is right about the mode and silent about every startup option this
  repository requires, so following it puts a second server and a second output
  base on the worktree.
- Flipping `--lockfile_mode` to `update` in `.bazelrc` so drift self-heals:
  rejected because it converts the only pin on the transitive module graph into
  an automatic rewrite, which is the same failure the repin prohibition exists
  to prevent, one level up.
- Hand-editing `MODULE.bazel.lock`: rejected because the lock carries registry
  file checksums and extension results that no human can compute by inspection.
- Folding the module-lock update into `cargo xtask bazel-repin`: rejected
  because it gives one command two authorities and makes "exactly one hub lock
  changed" unprovable; the two locks are separate mechanisms and each gets one
  command.
- Leaving regeneration undocumented: rejected as the failure mode the repin
  sub-decision exists to prevent.

## Decision 3: Generate first-party BUILD files in repository-owned xtask

**Decision**: `cargo xtask gen-bazel` reads `cargo metadata` for all four hub
workspaces and emits the generated first-party BUILD files, the exact governed
Rust source manifest, the derived censuses, and the workspace-boundary
exclusion list. `--check` regenerates in scratch and is wired into existing
`test-drift`. Hand-written fragments are allowed only when listed by the
coverage map.

Generator outputs that the migration adds:

- generated first-party `BUILD.bazel` files for all four hubs;
- the governed Rust source manifest the no-bash carrier declares;
- the **executed** harness-free census and the doctest census, each derived
  from the same selector the current Cargo gate uses, with every excluded
  manifest entry recorded together with its exclusion reason;
- the emitted schema census, taken from the manifest the schema writer already
  returns rather than from a hand-maintained number;
- `.bazelignore`, covering `.scratch/` and every Cargo output directory any
  workspace or tool in the worktree creates;
- the enumerated third-party build-script inventory and the action-environment
  allowlist from Decision 16.

**Rationale**: The repository already uses generator plus drift-check
ownership. Every census the gate asserts must be derived from the same source
the gate executes, or the census becomes a second, drifting authority.

**Alternatives**:

- `gazelle_rust`: rejected because it adds Go and a third-party generator for
  cases that need repository-specific treatment.
- Hand-author every BUILD file: rejected because Cargo metadata drift would be
  review-only and fail open.
- Globs for governed sources: rejected because the scan and its declaration
  could be incomplete in the same way.
- Literal counts in planning prose as the census: rejected, because a literal
  pinned at a manifest count fails closed forever the moment the executed set
  differs from it, and a literal pinned at today's executed count silently
  becomes wrong.

## Decision 4: Make the coverage map total and unambiguous

**Decision**: `tests/golden/bazel-rust-coverage.json` maps exactly these IDs,
each to a nonempty carrier set, one slice, one derived census, and one topology
where applicable:

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

The mapping is **total and unambiguous, not one-to-one**: every identifier has
a nonempty carrier set and every carrier belongs to exactly one identifier.
`rust-main-workspace-tests` already needs three carriers, so cardinality one
was never the property under enforcement. The fixed slices are `api`, `main`,
`broker`, and `aux`. `rust-contract-tests` and `rust-cli-contract-tests` are
explicitly excluded and remain fixture-lane Cargo/Nix surfaces.

**The guard is split by where each condition can actually be proven.** A Bazel
test action has no server, no source tree, and no sanctioned way to reach one,
so a condition phrased as a nested `bazel query` inside the test cannot execute
and would leave the guard green while proving less than it claims:

| Condition | Proven where |
| --- | --- |
| Mapped carrier label exists | Analysis time, through real `deps`/`data` edges on the guard target |
| Carrier claimed by exactly one identifier | Bazel test, over committed artifacts |
| No unmapped Rust test target in the graph | Make wrapper and `test-drift`, over a committed drift-checked or declared query result |
| Query drift | Make wrapper and `test-drift` |
| Census, topology, hand-written-fragment listing | Bazel test |

No Bazel test invokes `bazel query`, and no test action runs a nested Bazel
server. `test-drift` already carries query-derived staleness for every other
generated output, so no new gate, Layer-1 job, or Make target is created.

**Hand-written fragments the map must list.** The migration knowingly carries
four fragments upstream does not provide, plus the aggregate and carrier
fragments: the per-target nightly channel transition (Decision 14), the
`rustdoc_json` rule (Decision 14), the vendor repository rule (Decision 6), and
the yanked-state carrier fragment (Decision 6). Each tracks upstream internals
or a committed snapshot and is a review surface at every version bump.

**Rationale**: Exact coverage catches a green result caused by omission.
Minimum counts cannot detect a missing file offset by a new one. A guard that
cannot run half of its own invariants is worse than a red gate.

**Alternatives**:

- Target-count floor: rejected as fail-open.
- Infer coverage only from Bazel query: rejected because the graph cannot
  prove it represents the baseline contract.
- Include fixture surfaces: rejected because evaluated Nix fixtures are
  outside this ADR.
- Let the guard shell out to `bazel query`: rejected because it would put both
  a shell and a second Bazel server inside a test action.

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

**The runner's environment contract is part of the topology, not an
afterthought.** One Bazel test action per carrier means the build event stream
carries one result per target, so per-case attribution has to come from the
structured result the runner writes. The runner therefore derives each child
environment from the Bazel test environment, gives each case its own directory
beneath `TEST_TMPDIR`, resolves the test binary through runfiles, forwards only
the declared test environment, and writes one JUnit document to
`XML_OUTPUT_FILE` with one case element per enumerated case and explicit
passed, failed, and ignored outcomes. That document is redacted and bounded;
raw child output stays in the ordinary `test.log` artifact. Publication is
enforcing evidence, so an otherwise passing carrier fails when the document
cannot be written, while an existing test failure stays the primary diagnosis.
`contracts/runner-environment.md` is the full contract.

**Why publication failure is a carrier failure, not a warning.** The objection
to a fail-closed publication rule is that it converts an observability defect
into a test failure. That objection assumes the structured result is telemetry.
Here it is not. The amended ADR and `contracts/execution-manifest-binding.md`
make the JUnit and build-event stream the *only* mechanism that attributes a
result to a surface and a case: one Bazel test action per carrier means the
event stream carries exactly one verdict per target, and everything finer comes
from the document the runner writes. A carrier that returns success with no
document has not produced a weaker signal, it has produced no evidence that the
eighteen-surface manifest can consume, and the manifest finalization contract
cannot then mark that surface complete. Degrading to a warning would let a run
report `passed` for a surface whose result nothing observed, which is the exact
class of empty success this migration exists to eliminate, and it would do so
silently in the one direction reviewers do not look. The cost is bounded and
named: the failure is reported as a runner error distinct from a test failure,
it never displaces an existing test failure as the primary diagnosis, and its
remedy is a per-code recovery message rather than a rerun.

Two properties keep that rule from becoming a flake source, and both are
mechanical rather than aspirational. Publication happens after every child is
reaped, through the injected filesystem boundary, into a same-directory
close-on-exec temporary that is synced and installed with `renameat`; there is
no window in which a slow or contended filesystem produces a partial document
that the carrier accepts. And every terminal error in that path is a mapped
errno with a planted mutation behind it, so "publication failed" is a specific,
reproducible condition rather than an unexplained nonzero exit.

**Two scheduling consequences are recorded because they change measurement
shape.** First, exclusive tests are executed one at a time **after the entire
parallel build and test phase completes**, so in the single local invocation
the three broker suites are strictly last and nothing overlaps them; that
matters for warm and cold profile shape and for what partial evidence exists at
deadline expiry. Second, `exclusive` does **not** disable local test-result
caching, and `--test_output=streamed` silently makes every test exclusive, so
it is forbidden during any measured run.

**Why custom local resources are inert.** The ADR measured that custom
`resources:` tags did not serialize anything. The durable cause is narrower and
stronger than "the mechanism is not a mechanism": Bazel's test resource
computation returns its local-test-jobs-based resources unconditionally when
`--local_test_jobs` is in effect and discards every tag-derived resource, and
ADR 0052 section 8 mandates `--local_test_jobs` derived from `D2B_RUST_BUDGET`.
Custom resources are therefore permanently inert in the only configuration this
migration authorizes.

**Repetition evidence does not need a cache flag.** `--runs_per_test` is
exempt from `--cache_test_results=auto`, so the twenty-consecutive-execution
broker evidence genuinely executes twenty times without disabling result
caching.

**Rationale**: Plain `rules_rust` `rust_test` would weaken main and guest
isolation. Broker process-per-test is known to expose live sysfs behavior and
is deliberately not adopted. Per-case attribution that exists only on stdout is
not evidence a reviewer or the Actions UI can use.

**Alternatives**:

- Plain `rust_test`: rejected as a topology regression.
- Convert broker to per-case: rejected pending a dedicated isolation review.
- Custom Bazel local resources: rejected because they are discarded whenever
  `--local_test_jobs` is set, which this configuration always sets.
- `--nocache_test_results` for the repetition evidence: unnecessary, because
  `--runs_per_test` already bypasses result caching.
- Streaming test output during measurement: rejected because it silently
  serializes the entire run.

## Decision 6: Split offline supply-chain policy by capability

**Decision**: For each of the three workspace locks, `cargo-deny` enforces
`bans licenses sources` against a declared cargo-vendor-shaped tree with
`CARGO_NET_OFFLINE=1`. `cargo-audit` alone enforces advisories against a
committed RustSec snapshot with `--no-fetch` and the current
workspace-specific ignores. No Bazel action uses the network. Promotion
compares the union of findings with the current Cargo outcome across all three
locks.

**The vendor tree comes from a repository-owned repository rule, not from the
Bazel repository cache.** The repository cache is an internal
content-addressed store with no label and no enumeration interface, and
`crate_universe`'s generated spoke repositories expose per-crate rules rather
than `.crate` archives or a whole-tree filegroup, so the earlier "read it out
of the cache" phrasing is not a mechanism. The rule re-declares the downloads
instead:

- for every registry package in a lock, `ctx.download` with the crate's
  registry URL and the checksum the lock already records, which is served from
  `--repository_cache` when the bytes are already present, then extraction and
  a `.cargo-checksum.json` of the shape `{"files":{},"package":"<sha256>"}`
  that the committed flake path already produces;
- for the one pinned git source, `wl-proxy`, a fetch at repository-rule time by
  pinned rev **and** a committed archive sha256 cross-checked against the
  existing `outputHashes` pin in `flake.nix`, a checksum file carrying
  `"package": null`, and the matching source-replacement entry.

**Repository fetch is permitted; action network is not.** The no-network rule
is about actions and is absolute: no action in the Rust gate opens a socket,
and the vendored tree, the advisory database, and every tool reach an action as
declared inputs. A repository rule may fetch, and only under a pin.

**Classification is total and refuses rather than skips.** Every lock entry is
a first-party path dependency, a default-index registry package with a
checksum, or that one git source. A mirror or alternate index, or a
checksum-less non-git entry, is a named refusal. Before `cargo-deny` runs, the
action asserts the materialized package count equals the lock's, because a
vendor tree quietly short a crate makes `licenses` harvest fewer license files,
report fewer findings, and exit zero.

**The yanked-state carrier is unconditional, because a conditional capability
is not a capability.** Today's leaf runs `cargo deny check` with no subcommand
list, so `advisories` runs there in addition to `cargo audit`, which is what
makes the promotion comparison meaningful. The decomposed pair can lose
yanked-crate detection, which needs a registry index that neither the vendored
tree nor the RustSec snapshot provides.

An earlier draft built the yanked carrier only if the recorded comparison
showed a difference. That is backwards. The comparison is one observation of
one lock set at one instant, and "no crate in these three locks is yanked
today" says nothing about whether the gate can detect the next one. A
capability gated on a finding is missing exactly when the first finding
arrives, and by then promotion has retired the Cargo executor that used to
carry the outcome. So the committed, lock-bounded index snapshot and its three
carriers land in the shadow stage either way, reporting under the existing
`rust-deny-main`, `rust-deny-broker`, and `rust-deny-guest` identifiers rather
than as a nineteenth surface. An all-clear snapshot is the expected normal case
and is still committed; it is the baseline the next diff line is read against.
The comparison keeps its full promotion-blocking force; what it no longer
decides is whether the detection exists.

**The snapshot gets one updater and one validator, and they are different
commands.** `cargo xtask bazel-yanked-refresh` is the explicit reviewed
networked update outside the gate: it reaches the index, rewrites
`bazel/supply_chain/yanked-snapshot.json` with one entry per `(name, version)`
in the three committed locks plus the index revision it observed, and writes
nothing else. `cargo xtask bazel-yanked-check` is the offline exact key-set
validator: it reads the committed snapshot and the three committed locks,
proves `(name, version)` equality in both directions, opens no socket, and
writes nothing.

Splitting them is what makes the recovery text end somewhere useful. A remedy
that stops at `bazel-yanked-refresh` tells a contributor to regenerate and
commit, then leaves them to discover in continuous integration whether what
they committed actually satisfies the check. The validator is one binary with
one message, the three Bazel carriers run it as a declared-input action instead
of reimplementing the comparison per carrier, and a contributor runs the same
command in a shell and reads the same bytes before pushing. The drift recovery
therefore reads: refresh, review and commit the snapshot, then run the check.

**The refresh reaches the index through an injected boundary.** The refresh is
the only command in this migration that opens a socket, and its interesting
failures are all shapes of answer the index can return: a key the three locks
declare that the index does not know, a key the index knows that no lock
declares, an answer with no revision, a transport failure part-way through, a
malformed payload. None of those can be arranged locally, and reaching the live
index to produce them would put a network dependency inside the test suite this
migration requires to have none.

So `packages/xtask/src/bazel_yanked.rs` declares `trait YankedIndex` with
exactly the surface the refresh needs, the observed index revision and the
yanked state of one `(name, version)` key, and carries `IndexClient` as its
single networked implementation. The refresh is written against the trait and
receives the implementation from the command-line routing seam; its unit tests
inject a fake that returns canned responses for every case above. The offline
`bazel-yanked-check` names neither the trait nor the client, so it is offline by
construction and not by discipline, and a structural guard asserts that.

This is the same rule already applied to the filesystem and to time, extended
to the one remaining ambient dependency in the design. What the fake cannot
prove is that `IndexClient` talks to the real index correctly, and that is
deliberately measured somewhere else: the reviewed contributor-run refresh
produces a snapshot whose diff and observed revision the committing wave
records. One measured observation outside the gate is the right price for a
gate that opens no socket.

Schema reproducibility gains `gen-schemas --out-dir`, performs two sequential
independent generations in one action, and checks the exact generated census
for nonempty valid JSON before digest comparison. The no-bash carrier uses the
generated governed manifest, requires runfiles equality in both directions, and
requires parsed count to equal declared count.

**Rationale**: `crate_universe` repositories are not a Cargo registry, and
`cargo-deny advisories` fetches. The split preserves the aggregate policy while
making actions offline. Exact census prevents the empty-success behavior
already present in the current schema leaf.

**Alternatives**:

- Let actions fetch: rejected as non-hermetic and unauthorized.
- Read `.crate` archives out of the repository cache: rejected on the
  substrate; the cache has no enumeration interface.
- Run all `cargo-deny` checks offline: rejected because advisories still
  require the configured database behavior.
- Two identical schema actions: rejected because the second could be an action
  cache replay rather than an independent invocation.
- Accept a yanked-state difference as a deliberate difference: rejected,
  because promotion criterion 7 requires no differing enforcing outcome and
  ADR 0009 authorizes no supply-chain waiver.
- Build the yanked carrier only when the comparison finds a difference:
  rejected because it makes the capability absent precisely when it is first
  needed, and because it puts the decision to build a security check inside a
  promotion window where the cheapest answer is not to.
- Pin a full crates.io index snapshot: rejected because the state needed is
  bounded by three committed locks, so the artifact is bounded by them too.
- One command that refreshes and then validates: rejected because it makes the
  networked path the easy path. The check must be runnable, and must be run,
  without touching the network or the committed snapshot.
- Reimplementing the key-set comparison inside each of the three carriers:
  rejected because three implementations produce three messages that drift, and
  a contributor would have no way to reproduce any of them locally.
- Construct the index client inline inside `bazel-yanked-refresh`: rejected
  because it makes every refusal path in the refresh unreachable from a test.
  A command whose error handling can only be exercised by the index
  misbehaving is a command whose error handling is unproven.
- Assert refresh behavior against the live index in a test: rejected because it
  puts a network dependency and a moving third party inside a suite whose whole
  claim is that it opens no socket, and because it fails for reasons unrelated
  to the change under review.
- Put `YankedIndex` in a separate crate: rejected as premature. One trait with
  two methods, one networked implementation, and one fake all live beside the
  only command that uses them; a crate boundary buys nothing and costs a
  dependency edge the direction gate would have to be argued about.

## Decision 7: Bind Bazel results to execution-manifest v1

**Decision**: The Make wrapper maps Bazel Build Event Protocol results onto the
existing v1 surface IDs, `completed_leaves`, `failed_surfaces`, run status, and
partial evidence semantics. The existing reference and schema remain
authoritative and are not redefined. Prior evidence is invalidated before
dispatch; failure and handled interruption publish partial evidence.

The per-case JUnit documents from Decision 5 sit **below** this binding: they
give per-case attribution inside a carrier, while the manifest continues to
carry exactly the eighteen surface identifiers. No manifest field is added,
renamed, or reinterpreted.

**Rationale**: Source discovery proves availability, not execution. Keeping
the schema stable allows Cargo and Bazel evidence to be compared directly.

**Alternatives**:

- A Bazel-specific manifest version: rejected because executor identity does
  not change the evidence contract.
- Logs only: rejected because logs do not provide atomic, comparable partial
  evidence.
- Adding per-case identifiers to manifest v1: rejected; per-case evidence
  belongs in the JUnit document the executor already designates.

## Decision 8: Use bounded local scratch with an explicit workspace boundary

**Decision**: Output user root, action cache, and repository cache persist only
under `.scratch/bazel/`. Action cache uses 8 GiB and 14 days. Repository cache
is capped at 2 GiB. Output-root marks are 20/40 GiB. `make clean` owns
reclamation under existing scratch controls.

**Startup options come from the wrapper, as absolute paths.** `%workspace%` is
resolved only for rc import paths and for a small set of Java-side options, and
`--output_user_root` and `--output_base` are startup options parsed by the
client, so a `startup --output_user_root=%workspace%/.scratch/bazel` line
creates a literal `%workspace%` directory. Therefore:

- `.bazelrc` carries only `common`, `build`, `test`, and `build:<config>`
  lines. `common --lockfile_mode=error` is valid there, because `common`
  applies to every command that supports an option and ignores it elsewhere.
  `common --check_direct_dependencies=error` sits beside it, because that check
  warns by default and a warning is not a pin.
- The Make and Rust wrapper supplies every startup option as an absolute path
  derived from the worktree, and supplies **byte-identical** startup options to
  `build`, `test`, `query`, `info`, `shutdown`, and `clean`. That is what makes
  "shut down with the same startup options" mechanically enforceable instead of
  aspirational: mismatched startup options start a second server and leave the
  live one owning the tree.

**The workspace boundary is declared, not assumed.** With the output user root
under `.scratch/bazel/`, the output base, every external repository, and the
convenience symlinks live inside the source tree, and Bazel does not
automatically exclude a real directory under the workspace from package loading
or from `glob()`. The worktree also carries many Cargo output directories.
Therefore `.bazelignore` is a generated, drift-checked artifact covering
`.scratch/` and every Cargo output directory any workspace or tool creates, and
the wrapper passes an absolute `--symlink_prefix` pointing beneath `.scratch/`
so no convenience link lands at the repository root.

**Trimming is synchronous and on demand.** Bazel's built-in disk cache garbage
collection runs asynchronously in the server while it idles, so a job that
proceeds directly to a size measurement, or that shuts the server down first as
the cleanup contract requires, can observe an untrimmed cache and then
correctly refuse to publish, permanently. That is exactly the deadlock ADR 0052
section 10 exists to prevent. The migration therefore invokes the upstream
on-demand collector `//src/tools/diskcache:gc`, present at Bazel 8.6.0, or a
pinned repository-owned equivalent, as a named step before measurement and
save. Idle-delay-based collection and the size refusal remain as secondary
mechanism and backstop. Because the size design depends on
`--experimental_*` flags and on an upstream tool label, **any Bazel version
bump reopens the disk-cache garbage-collection design review** rather than
being an ordinary version bump.

The evidence-only `cargo xtask bazel-evidence prepare-cold-local` helper
prepares a fresh output user root and empty action cache while retaining a
populated repository cache. It exists because the ADR defines a cold profile
but no safe existing command can produce that state without either preserving
all scratch or deleting the download cache. It is not a Make target or
contributor compatibility contract, and W5 removes it after W4 qualification.

Cleanup first shuts down the matching server with the same startup options,
anchors `.scratch/` once, rejects symlinks, magic links, escapes, tracked
files, live ownership, and replacement races, and deletes descriptor-relative
with close-on-exec descriptors. Refusal deletes nothing and gives its
code-specific recovery.

**Cleanup and result publication share one injectable filesystem boundary.**
`packages/d2b-bazel-support/src/fsops.rs` owns `openat2`, the forced
component-walk fallback, `open`, `write`, `fsync`, `renameat`, `unlinkat`,
directory enumeration by name, the anchored provider open, the metadata and
byte reads the provider checks need, and the `execveat` of a verified handle,
and the JUnit writer, cleanup, the topology provider checks, the locator, and
the wave-note policy lint all call through it. Two reasons, and neither is
stylistic. The first is that these
subsystems enforce the *same* properties on the same syscalls: anchored
close-on-exec
descriptors, refusal of symlink and magic-link parents, refusal of an anchored
`..` escape, and unlinking only what the runner created. Implementing them
twice means the planted mutations only prove one copy is correct, and a future
change fixes one caller. The second is that every negative in this design is an
errno at an exact call ordering, and the only reliable way to produce
`ENOSPC`, a short write, an `EINTR` retry, an `EEXIST` collision, a
replacement race, or a provider path rebound between the open and the exec on a
shared reference host is to inject it. A test that needs a genuinely full disk
is a test that will be marked ignored within a quarter.

**The resolve policy is a parameter, because the call sites genuinely differ.**
Paths the runner creates and files the repository commits are opened with
`RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS`; providers are
opened with `RESOLVE_NO_MAGICLINKS` alone. That is not a relaxation for
convenience. Measured against a runfiles-shaped symlink whose target lies
outside the anchor, the strict policy fails `EXDEV` or `ELOOP`, so applying it
to providers would refuse every real Bazel provider; and measured against a
plain committed directory, the strict policy refuses a symlink with `ELOOP`, a
`..` escape with `EXDEV`, and a mode `0000` entry with `EACCES` while a
subdirectory opens and reads `EISDIR`, which is exactly the four-way
distinguishability the wave-note lint needs. The fake supplies both routes and
each caller's choice is asserted, so a site cannot silently inherit the other's
policy.

**The same parameter decides the leaf on the forced component-walk route.**
An earlier draft of the runner-environment contract described that route as
`O_NOFOLLOW` on every component *except the final one*, unconditionally. That
is a silent downgrade of every strict caller on one of its two routes: the
wave-note lint, cleanup, and the two output-path opens would all follow a
symlink planted at the final name instead of refusing it, and the four-way
errno distinguishability the lint depends on would collapse. Measured on the
reference host against the same runfiles-shaped leaf symlink, the leaf flag is
exactly the lever that reproduces the policy difference: `openat` on the leaf
without `O_NOFOLLOW` opens and returns the same `st_ino` that `openat2` with
`RESOLVE_NO_MAGICLINKS` returns, while `openat` on the leaf with `O_NOFOLLOW`
fails `ELOOP`. Intermediate components stay `O_NOFOLLOW` under both policies;
the errno there is `ENOTDIR` rather than `ELOOP`, because `O_DIRECTORY` reaches
the refusal first, and a test that asserts one errno for both positions asserts
something the kernel does not do.

**What the walk route cannot reproduce is recorded, not approximated.**
`RESOLVE_NO_MAGICLINKS` has no `openat` flag. Measured: a leaf symlink whose
body names `/proc/<pid>/fd/<n>` is refused `ELOOP` by `openat2` under
`RESOLVE_NO_MAGICLINKS`, opens successfully on the walk route's permissive
leaf, and yields a handle carrying the target's own inode and the target's own
`fstatfs` filesystem type, so it is indistinguishable from a handle opened
through a leaf symlink that names the target directly. `fstatfs` was measured
specifically to check whether it could stand in: it reports `0x9fa0` for a
procfs regular file and `0x6e736673` for `nsfs`, but it reports the ordinary
filesystem for the laundered case, which is the case that matters. A partial
check that looks like a magic-link refusal and is not one is worse than the
recorded difference, because it gets cited as one, so none is added. Two things
bound the residual. Handle identity is untouched on that route, and handle
identity is what protects a provider: kind, mode, freshness, the digest
compared against the coverage map, and the bracketing `fstat` all still run on
the resulting descriptor. And the kernel floor closes the production case
outright: ADR 0008 pins supported hosts at `6.6` with the v1.1 uplift raising
it to `6.9`, `openat2` landed in `5.6`, and the repository already relies on
that floor unconditionally in
`packages/d2b-host/src/bin/d2b-activation-helper.rs`.
No supported host takes the walk route; it exists so the walk's ordering and
errno mapping are provable through the fake.

**Directory order is not an order.** The wave-note lint sorts enumerated entry
names by unsigned byte order before it opens anything, and both the returned
entry sequence and any one-based position label derive from that sorted
sequence. Measured on the reference host, the same seven note names enumerate
as `w2 w0 w1 w11 w3 w10 w9` on ext4 and as `w3 w11 w1 w0 w2 w10 w9` on tmpfs.
A position label taken from raw enumeration therefore names a different entry
in CI than it does locally, and a contributor handed such a refusal cannot
reproduce it. Byte order rather than a locale collation, because it is total
over raw directory-entry bytes and identical on every machine.

**The wave-note lint reads its corpus through the same boundary.** It does not
call `std::fs::read_dir`, does not call `std::fs::read_to_string`, and never
concatenates a `DirEntry` onto a parent path. A guard whose own reads follow a
symlink out of the directory it is policing, or which resolves that directory a
second time between enumeration and read, is the check-then-use shape the rest
of this design refuses, performed by the code that exists to catch leaks.
`packages/d2b-contract-tests` reaches the boundary as a dev-dependency only,
which keeps the support crate's non-dev consumer list at the runner, the
locator, and `xtask`.

**The boundary lives in a neutral crate, not in the runner.**
`packages/d2b-bazel-support/` is an internal build-tooling crate that declares
no first-party dependency at all. Three crates read it as a non-dev dependency,
the runner, the locator, and, from W2, `xtask`, and one more,
`packages/d2b-contract-tests`, reads it as a dev-dependency for the wave-note
lint. Putting the trait in the runner instead would
have forced `xtask -> d2b-bazel-runner` as soon as the repin and module-refresh
commands needed the one shared startup-option construction, and that edge runs
the wrong way: `xtask` is the generator, and the runner's own Bazel targets are
part of what `xtask` generates. The neutral crate is also what keeps the
locator honest, because the locator must not depend on the runner to get a
filesystem seam. `RunfilesView` sits beside `FileSystem` in the same crate for
the same reason: both the locator's Bazel arm and the runner's child-binary
resolution look up declared runfiles, and a fake runfiles view is the only way
to plant a missing entry without editing a real runfiles tree.

The boundary is a W0-frozen module path, not a W2 refactor, and W0 lands it
whole, so the W1 runner and locator scopes and the W2 cleanup, result
publication, and deadline scopes all open against one stable trait surface with
one fake, and the slices stay file-disjoint. The W0 `generator` scope, which
owns the wave-note lint, opens against the same surface from the same prep tip.

**Time does not move with it.** `Clock` and `UptimeSource` stay in
`packages/d2b-bazel-runner/src/clock.rs`, because only the deadline and process
paths read them: the locator has no clock dependency, since provider staleness
is a comparison of two timestamps the filesystem boundary already returns from
the provider's own descriptor, and
`xtask` has none either. A boundary shared by one crate belongs in that crate;
moving it into the neutral crate would buy nothing and would widen a surface
three crates then have to agree on.

**Rationale**: Persistent reuse is the local performance benefit, but an
unbounded or path-based cleanup can fill disk or delete unrelated data, and a
trim whose completion cannot be observed converts a safety check into a
permanent refusal.

**Alternatives**:

- `%workspace%` in a `startup` line: rejected because it is not expanded there
  and silently creates a literal directory.
- Relying on idle garbage collection before measurement: rejected because it is
  asynchronous and a shutdown-first cleanup order can never observe it.
- Shared worktree cache: rejected because worktrees own independent state.
- `realpath` then recursive remove: rejected for the check/use race.
- Raw manual removal for cold measurement: rejected because it bypasses the
  cleanup contract.
- Relying on Bazel to ignore in-workspace output directories: rejected;
  measured behavior is that it traverses them.

## Decision 9: Keep shadow CI credential-free and cache-free

**Decision**: One non-required workflow runs four parallel slices plus a
rollup. It calls only approved Make targets, uses the mandated shell and
credentialless checkout, and runs on the amended ADR's triggers and path
filters. Shadow restores and saves nothing. PR-reachable jobs have
`contents: read`, no `actions: write`, and no direct, indirect, post-step, or
unknown cache writer.

**Evidence comes only from qualification records.** A qualification record is
a `push` event on `refs/heads/v3` produced by a merged pull request. The
required Cargo workflow triggers on `push` for `[main, v3]`, so both runs are
identified by the same head commit under the same event, which is what makes
"both paths tested the same commit" mechanically true. Each record carries the
head commit, both run identifiers, both rollup verdicts, the same-commit
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` companion verdict,
and, for a cold-sample record, the four slice durations.

**Pull-request runs stay diagnostic and stay path-filtered.**
`refs/pull/N/merge` is recomputed against a moving base, so two workflows
triggered by the same pull request can legitimately test different trees; and a
sample drawn only from pull requests that touched Bazel-owning paths is
precisely the sample in which a Cargo-versus-Bazel divergence cannot appear.
Dropping the path filter was considered and rejected: with no cache published
during the shadow stage, every such run is a full cold Rust build that buys
nothing.

**Streak arithmetic is fail-closed and machine-evaluable.** A record whose two
verdicts differ resets the streak. A shadow run that reaches no verdict while
its paired Cargo run reaches one counts as a mismatch and also resets, because
otherwise cancelling a run about to go red would launder the streak. A push
where neither side reached a verdict is not a record and neither extends nor
resets.

At promotion, action and download snapshots become separate 4/1 GiB entries.
The output base is never cached. Keys bind, at minimum: `.bazelversion`,
`MODULE.bazel`, `MODULE.bazel.lock`, `.bazelrc`, both `rust-toolchain.toml`
files, all four hub Cargo locks, `packages/Cargo.guest.lock`, all four per-hub
`crate_universe` Bazel-side locks, the `cargo-bazel` generator sha256, all deny
configurations, the advisory-database pin, the committed yanked snapshot,
`.bazelignore`, the symlink-prefix and startup-option configuration, the
build-script annotation digest and action-environment allowlist, and the
generated BUILD tree digest. PRs restore read-only; exactly one protected-`v3`
job writes. Cache credentials exist only inside cache actions and never in a
`run:` or Bazel environment.

**Rationale**: The measured cache had about 0.87 GiB headroom. Shadow
publication would evict caches the required Cargo path uses, and exposing a
cache token to Bazel exposes it to third-party build code. A key that does not
bind an input that changes behavior produces a subtly stale cache, which is
worse than no cache.

**Alternatives**:

- Publish shadow cache: rejected due to eviction risk.
- Build the streak from pull-request runs: rejected twice over, on cold cost
  and on the moving merge base.
- Keep every clock on the literal default branch: rejected on measurement;
  `origin/HEAD` is `main` and `v3` never merges to `main`.
- Cache the output base: rejected as path, server, and machine specific.
- Remote cache or execution: rejected as out of scope and incompatible with
  the credential boundary.

## Decision 10: Use fixed performance and deadline semantics

**Decision**: A profile passes only when its median is at or below the ceiling
and no sample exceeds 1.2 times it. Local sets contain three consecutive runs;
the continuous-integration set is the five most recent qualifying cold
qualification records, where qualifying means no Bazel cache was restored and
all four slice jobs completed with a recorded duration. During the shadow stage
every run is cold by construction, so the qualifier excludes runs that produced
no measurement rather than selecting among warm and cold runs. Scheduled,
dispatched, and `main`-push runs are liveness probes and never enter the set.

**A feasibility gate precedes the binding cold-CI ceiling.** The 15-minute
median and 18-minute maximum are asserted against a 4 vCPU runner that pays a
full cold build of a 56-member workspace, clippy over all targets, the whole
test set, Nix installation, Bazel acquisition, a four-hub `crate_universe`
fetch, the `cargo-bazel` download, and a second configuration for the census
subgraph, with no step carved out. W3 therefore records a cold-CI feasibility
measurement before the ceiling becomes binding, and the only pre-authorized
answers to a shortfall are a larger runner class or a further disjoint slice
split. A missed ceiling never becomes pressure to weaken coverage.

**Measurement invalidation.** A cleanup, hard refusal, server restart, wrong
edit, cache-state change, heavy-lane overlap, mismatched environment, or any
use of `--test_output=streamed` invalidates a sample. Invalid samples are
retained with their reason and replaced.

Promoted CI reads `/proc/uptime` after checkout through one checked
fixed-point parser. Capture truncates, read rounds up, child duration rounds
down. It exports an absolute boot-relative integer-millisecond deadline of
`anchor + 780000`; checkout has a separate 2-minute bound. A missing deadline
is allowed locally and forbidden in promoted jobs; the promoted-job assertion
is an implementation deliverable rather than an existing check. Expired is a
normal budget expiry. Malformed or overflowing values fail without echo.

**Time enters through one injected boundary.**
`packages/d2b-bazel-runner/src/clock.rs` declares an `UptimeSource` that yields
the raw `/proc/uptime` field and a `Clock` that yields the current
boot-relative instant; the deadline parser, the remaining-budget subtraction,
and the child-duration rounding take both by injection and never read the host
clock or the procfs path directly. That is what makes the grammar and rounding
table testable at all. The interesting cases here are a rejected exponent, a
second separator, a non-ASCII digit, an overflowing value, a capture that must
truncate while the paired read must round up, and a remaining budget of exactly
zero. Every one of them is a specific input, and none of them can be produced
by waiting on a real clock; a test that sleeps to reach a boundary is a test
that is flaky on a loaded host and green on an idle one. Expiry-path tests
drive the fake clock past the deadline deterministically, so the SIGTERM,
full-grace, SIGKILL, and reap ordering is asserted without a timing race.

The wrapper creates a dedicated child process group without a shell, sends
SIGTERM, waits the fixed grace in full, observes with `EXITED|NOWAIT|NOHANG`,
sends unconditional group SIGKILL, then reaps. It never signals its own group
or a server PID read from a file. Server shutdown uses matching Bazel startup
options and its own bound.

**Rationale**: A job timeout alone is unactionable. Conservative conversion and
group ownership make the ceiling a real upper bound without orphaning work or
signalling unrelated processes. A ceiling asserted with no supporting
measurement is a guess that becomes coverage pressure at the worst moment. A
deadline implementation that reads the clock directly cannot be tested at its
boundaries, and an untestable boundary is where the off-by-one lives.

**Alternatives**:

- Float or raw uptime value: rejected for rounding and grammar ambiguity.
- Absolute deadline passed as duration: rejected as a silent many-hour bound.
- Reap or end grace on leader exit: rejected because descendants can survive.
- Relax a missed ceiling: not authorized. Only a larger runner or a further
  disjoint slice split is allowed.
- Read `/proc/uptime` and the system clock directly from the deadline module:
  rejected because every boundary case then requires either a sleep or a
  privileged clock change, and both produce tests that get disabled.

## Decision 11: Stage promotion, alias removal, and retirement separately

**Decision**: Cargo remains authoritative through the shadow stage and W4
evidence. Promotion requires exact coverage, census, and topology, ten matching
qualification records, an eighteen-case isolated failure matrix, twenty broker
repetitions, all performance sets, supply-chain equivalence including the
yanked outcome, and cache-policy evidence.

W4 commits one immutable qualification record set. W5 writes a separate
promotion record after the ordered protected-`v3` maintenance and save run.
Release containment and the ten-green-run clock are recorded separately after
promotion: alias removal depends only on release containment, while Cargo
implementation retirement depends only on ten consecutive promoted green runs.
Neither retirement path waits for the other.

Cache cutover is ordered: pre-merge audit and protected-`v3` freeze establish
the candidate set; promotion stops old writes; protected-`v3` maintenance
deletes only authorized prefixes with complete pagination; an explicit
synchronous trim runs; usage plus planned snapshot is checked at most 8 GiB,
checked again immediately before save; then one writer publishes. The
maintenance verdict remains separate from the Rust verdict in both directions.

**Retirement removes implementations, never names.** Cargo implementation
retirement deletes only the eighteen surfaces' Cargo leaf modes from
`tests/test-rust.sh` and unreachable Cargo-specific plumbing. The public
`make test-rust` target and all eight `make test-rust-<leaf>` names remain and
forward to the authoritative Bazel carriers; deleting them, or leaving
`test-rust` with only the fixture leaf, is forbidden. `fixture-contracts`
stays.

**Rationale**: Evidence collection cannot be combined with the irreversible
executor flip. Separate compatibility and implementation retirement preserve
rollback and distinguish naming failures from executor failures.

**Alternatives**:

- Promote while collecting evidence: rejected as circular.
- Delete aliases at promotion: rejected as an avoidable contributor break.
- Retire Cargo at promotion: rejected because rollback would require
  reconstruction.
- Retire the public leaf names with their Cargo implementations: rejected as a
  contributor and documentation break unrelated to the executor change.

## Decision 12: Use existing delivery and test surfaces

**Decision**: Behavioral guards live in the owning Rust crate, source-shape
cleanup guards extend `policy_docs.rs`, workflow and cache guards extend
`policy_ci.rs`, crate dependency direction extends
`tests/unit/meta/w0-dep-direction.sh`, and generated drift extends
`test-drift`. Each ships with a
positive case and an observable negative mutation. No host or manual tier is
used. Every wave has the full ten-role plan and diff panels, unanimous and
bound to one snapshot. This plan does not use pipelined dispatch.

**The dependency-direction guard extends the resolver-backed gate that already
exists.** `tests/unit/meta/w0-dep-direction.sh` is wired into both
`tests/test-policy.sh` and `tests/static.sh`, resolves dependencies with
`cargo metadata --no-deps` rather than by reading manifest text, and already
fails closed when the resolver cannot run. Those three properties are the whole
reason to extend it rather than to write a Rust manifest scan: a `package =`
rename, a workspace-inherited dependency, and a target-specific dependency all
resolve correctly there and would all be invisible to a text scan. Extending it
adds no gate, no Layer-1 job, and no required context, so FR-053 is satisfied
by the same reasoning that puts the wave-note lint in an existing Rust carrier.
Its planted negatives are an `xtask -> d2b-bazel-runner` edge and a first-party
edge out of the support crate, each added, observed refused, and reverted
during W0 integrated validation, where all three build-tooling crates are
workspace members and the gate's required-crate assertion can be satisfied.

**The `software` seat is filled by the Bazel and `rules_rust` expert for this
delivery run.** The findings that forced the ADR amendment were all
substrate-level: channel scope, rustdoc-JSON absence, compile-time environment
expansion, repository-cache enumeration, disk-cache collection timing, and rc
file expansion rules. A generalist software seat produced a plan that read
correctly and could not be built. That seat assignment is a delivery-run
requirement, not a preference, and it applies to every plan panel and every
integrated-diff panel in every wave, including the post-promotion W6 and W7
children.

**Choosing the carrier is not free, because one Rust crate is invisible to the
workspace leaves.** `tests/test-rust.sh` sets
`workspace_test_excludes=(--exclude d2b-contract-tests)`, so a guard placed in
`packages/d2b-contract-tests/tests/` runs only under
`D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts`, never under
`make test-rust-main`. The reverse is also true and is the sharper edge: that
crate's `policy_broker_schema.rs` walks every `.rs` file under `packages/`, so
a wave that adds a runner test file has already changed one of its inputs even
though the diff looks unrelated to fixtures. The plan's fixture-dependent
validation rule derives the input set from the crate with a `grep` rather than
from memory, and every code-changing wave in this migration runs the
fixture-contract target as a result.

**Rationale**: These are hermetic build-infrastructure properties. Existing
Layer-1 carriers are both lower and more enforcing than container, VM, live, or
hardware tiers. Strict serialization avoids invalidating work that consumes new
generated contracts or evidence.

**Alternatives**:

- New shell gate or Layer-1 job: rejected by test-layer policy and by the
  amended no-shell scoping.
- Treat green tests as review: rejected because review and tests detect
  different failure classes.
- Pipeline later waves after five reviews: constitutionally permitted but
  rejected here because the saved time does not outweigh contract and evidence
  rework.

## Decision 13: Branch authority is settled by the merged amendment

**Decision**: The ADR 0052 amendment is merged. Protected `v3` is the sole
promotion, cache-maintenance, cache-publication, equivalence-streak, and
post-promotion observation lineage, and the cold continuous-integration profile
draws from qualifying cold qualification records on `v3`. No task in this
feature amends the ADR; W0 instead verifies that the amended commit is present
in the base it builds on and refuses to proceed otherwise.

**Rationale**: The repository's GitHub default branch is `main`, while binding
repository policy makes `v3` the clean-break integration lineage that never
merges to `main`. GitHub schedules execute from the default branch, so a
default-branch schedule and default-branch writer language could not implement
the selected `v3` migration. That correction landed in the ADR, so the plan
consumes it rather than proposing it.

**Alternatives**:

- Change the GitHub default branch to `v3`: rejected as a repository-wide
  governance change outside this feature.
- Promote on `main`: rejected because the feature and release lineage is `v3`.
- Add a `main` scheduler that dispatches `v3`: rejected in favor of the
  qualification-record stream the protected lineage already produces.

## Decision 14: Reach nightly by a per-target transition and render JSON with a repository-owned rule

**Decision**: The API census subgraph sits behind a repository-owned Starlark
rule carrying an outgoing `cfg = transition(...)` that sets
`@rules_rust//rust/toolchain/channel` to `"nightly"` over that subgraph only,
copying the shape of the in-tree private transition upstream already uses for
its own nightly-only rule. Inside it, a repository-owned `rustdoc_json` rule
invokes the resolved nightly rustdoc from the registered toolchain with the
JSON output format, declares one JSON output per crate, and declares **the
toolchain version string the action actually used** as an additional output. A
diff test compares the rendered JSON against `tests/golden/api-surface`; a
guard compares the emitted version to `packages/d2b-api-surface/rust-toolchain.toml`.

**One Bazel invocation remains.** The transition buys subgraph isolation inside
the single invocation, so ADR 0052 section 8's single-invocation decision is
preserved.

**Why not the obvious things.** The channel is a global build setting with
universal scope and upstream ships no public per-target transition, so
`--@rules_rust//rust/toolchain/channel=nightly` on the command line flips the
entire invocation: every first-party crate would compile on nightly while the
gate stayed green, silently violating pin equality against
`packages/rust-toolchain.toml`. No `.bazelrc` line and no wrapper argument sets
that flag, and a guard fails closed on one. `rules_rust` 0.73.0 exports
`rust_doc` (HTML) and `rust_doc_test` and nothing else in that family, so there
is no rustdoc-JSON rule to use; the current script additionally installs a
toolchain through `rustup` at run time, which no action can do.

**Cost, recorded rather than elided.** A transition creates a second
configuration, so the census subgraph's dependencies analyze and build once per
configuration. That cost is charged to the `api` slice's cold and warm
profiles.

**Alternatives**:

- Global channel flag: rejected; universal scope, silent pin violation.
- Two Bazel invocations, one stable and one nightly: rejected; it contradicts
  the single-invocation decision, pays a second analysis phase and server
  interaction, and its cost would have to be charged to the profiles anyway.
- Waiting for an upstream rustdoc-JSON rule: rejected; none exists at the pin.
  If one lands, replacing the fragment is an ordinary change.
- Asserting the pin file instead of the emitted version: rejected; that proves
  what was requested, not what executed.

## Decision 15: Locate binaries and fixtures through a dual-mode locator

**Decision**: First-party tests stop resolving binaries through compile-time
`env!("CARGO_BIN_EXE_*")` and stop resolving repository paths by walking out of
`CARGO_MANIFEST_DIR`, and move to a repository-owned locator with two arms:

- **Under Bazel**, a run-time lookup through `@rules_rust//tools/runfiles`
  against a declared runfiles-relative path, with the binary a `data`
  dependency of the
  test target so a missing binary is an analysis failure. No test resolves
  anything by an absolute execution-root path under either executor.
- **Under Cargo**, the existing environment, unchanged. Cargo defines
  `CARGO_BIN_EXE_<name>` only for the integration tests of the crate that
  declares the binary, so the Cargo arm **must expand in the calling test
  crate**: it is a macro, not a function in a shared library crate. A shared
  function would capture the locator crate's own environment and resolve to
  nothing, or to the wrong crate, while compiling cleanly.

**Mode is selected once and the arms never chain.** The locator reads the
runfiles environment exactly once; if it indicates a Bazel test, a missing
runfiles entry is a hard failure naming the declared runfiles-relative path,
which is repository content rather than a local value, and it
never falls back to the Cargo arm. Chaining is the failure that matters:
`packages/target/` holds real, executable, out-of-date binaries for the whole
shadow stage, so a fallback would find one and the test would go green against
the wrong binary.

**One open, one descriptor, one execution.** The locator does not return a
path. It returns a verified descriptor, and the caller executes that
descriptor. Both arms produce an anchor plus one declared relative component,
the boundary performs exactly one
`openat2(anchor, relative, O_RDONLY|O_CLOEXEC, RESOLVE_NO_MAGICLINKS)`, and
every check runs against that open descriptor: `fstat` for regular-file kind,
executable mode and modification time; `pread` from offset zero for the byte
digest the coverage map records; and a second `fstat` that must agree with the
first on `st_dev`, `st_ino`, `st_size`, `st_mtim`, and `st_ctim`. Execution is
`execveat(fd, "", argv, envp, AT_EMPTY_PATH)` on the same descriptor. There is
no `std::process::Command` by path, no `fexecve`, and no `/proc/self/fd/<n>`
fallback; an `ENOSYS` from `execveat` is a refusal, not a reason to reopen.

The earlier draft stated the provider and then spawned by path, which is two
resolutions of one name with a window between them. That is not theoretical
here: `packages/target/` is the directory the shadow stage keeps full of stale
binaries, and a concurrent Cargo build replaces entries in it by rename while
the gate runs. Measured on this host rather than reasoned about:

- after the provider path is replaced by a different executable, executing a
  retained descriptor still runs the original verified bytes, while a freshly
  path-opened descriptor runs the replacement, which is the check-then-spawn
  defect observed end to end;
- `execveat` with `AT_EMPTY_PATH` succeeds on an `O_RDONLY|O_CLOEXEC`
  descriptor, and that descriptor is absent from the child's descriptor table,
  while a control descriptor opened without `O_CLOEXEC` is present;
- the same descriptor execs repeatedly, so process-per-case is preserved with
  one open and one digest per provider per carrier invocation;
- `pread` on an `O_PATH` descriptor returns `EBADF` although `execveat` on it
  succeeds, so `O_PATH` cannot carry identity and is rejected;
- `RESOLVE_BENEATH` fails `EXDEV` and `RESOLVE_NO_SYMLINKS` fails `ELOOP` on a
  runfiles-shaped symlink whose target lies outside the anchor, so the provider
  open uses `RESOLVE_NO_MAGICLINKS` alone and relies on handle identity rather
  than on link refusal, while `RESOLVE_NO_MAGICLINKS` still refuses a
  `/proc/<pid>/fd/<n>` path with `ELOOP`;
- on the forced component-walk route the same leaf symlink opens without
  `O_NOFOLLOW`, returning the same `st_ino` the `RESOLVE_NO_MAGICLINKS` open
  returns, and fails `ELOOP` with `O_NOFOLLOW`, so the leaf flag is exactly the
  lever that reproduces the resolve policy on that route, while an intermediate
  directory symlink opened `O_DIRECTORY|O_NOFOLLOW` fails `ENOTDIR` rather than
  `ELOOP`;
- a leaf symlink whose body names `/proc/<pid>/fd/<n>` opens on the walk
  route's permissive leaf and yields the target's own inode and the target's
  own `fstatfs` filesystem type, so no descriptor-side check distinguishes it
  from a leaf symlink naming the target directly; `fstatfs` reports `0x9fa0`
  for a procfs regular file and `0x6e736673` for `nsfs` but reports the
  ordinary filesystem for exactly the case that matters, which is why no
  partial stand-in for `RESOLVE_NO_MAGICLINKS` is added and the difference is
  recorded instead;
- writing into an already-open regular file changes the bytes a later `pread`
  returns and moves `st_mtim` with `st_ino` unchanged, which is what the second
  `fstat` catches;
- `execveat` returns `EACCES` for a mode `0644` regular file and for a
  directory descriptor, so the kernel stays the authoritative permission
  decision and the mode check is the better-named early refusal;
- a `#!` script executed from a close-on-exec descriptor fails `ENOENT`,
  because the interpreter reopens the descriptor by its `/proc` path. First
  party providers are compiled binaries, so this is recorded as a measured
  limitation with a named refusal rather than worked around.

**Every provider negative is injected, and nothing is planted on disk.** The
absent, non-regular, non-executable, stale, and wrong-identity providers, and
the path rebound to a different inode after the open, are states of the
`FileSystem` fake in `packages/d2b-bazel-support/`, and the removed runfiles
entry is a state of the `RunfilesView` fake beside it. The fake models inodes
rather than paths, so a rebind after the open is representable and its effect
is observable. The guard that proves the guard is therefore a fake filesystem
that reports an out-of-date, wrong-digest executable at the Cargo path while
the fake runfiles view reports no entry, run in Bazel mode, requiring failure.
An earlier draft wrote a real stale executable into the Cargo path. That is the
one arrangement this design must not use: `packages/target/` is exactly the
directory whose real, out-of-date binaries are the hazard being guarded
against, so a test that plants one there has manufactured the hazard on the
shared reference host, and an interrupted run leaves it behind for whatever runs
next. The same reasoning that keeps `ENOSPC` and `EINTR` off the real disk keeps
a stale provider off the real path.

One test is deliberately not injected, because the list above is a list of
kernel behaviors and a fake cannot prove a kernel.
`packages/d2b-bazel-runner/tests/exec_handle.rs` drives the host-backed
implementation against `packages/d2b-bazel-runner/src/bin/d2b-exec-probe.rs`, a
first-party probe binary that prints its own descriptor table with the device
and inode each descriptor names. It arranges nothing on the host; it executes a
declared input the graph already builds.

**All fixture reads become declared data**, resolved through the same locator.
A check that needs the repository *inventory* rather than a file consumes a
generated drift-checked manifest as a declared input instead.

**The migration is enumerated, not sampled.** Measured in this worktree: 25
files under `packages/` locate binaries through `env!("CARGO_BIN_EXE_...")`;
50 files reference `CARGO_MANIFEST_DIR`, of which 20 are test files, 11 of
those through a `repo_root()` helper. Every one is migrated, or recorded in the
coverage map as needing no migration together with the reason. Both arms stay
green on the Cargo path for the whole shadow stage. This is the largest
first-party code change the migration requires.

**Alternatives**:

- Set `CARGO_BIN_EXE_*` through `rust_test.env`: rejected; that reaches only
  the run-time environment, which the compiler never sees.
- Bake the path into `rustc_env`: rejected; the value freezes into a cached
  artifact that then travels into a different execution root, which is the
  wrong-binary failure with extra steps.
- Add a `build.rs` to each affected test crate: rejected; a build script cannot
  see the runfiles tree that exists at test time, and it would add a
  first-party build-script surface this workspace measurably does not have.
- Put the Cargo arm in a shared library crate: rejected; it captures the wrong
  environment and compiles cleanly while resolving nothing.
- Return a path from the locator and let the caller run
  `Command::new(path)`: rejected; the check and the spawn then resolve the same
  name twice and the second resolution wins, which was measured to run a
  replacement binary while the verification had passed on the original.
- Keep the path but re-stat immediately before the spawn: rejected; it narrows
  the window without closing it, and a narrowed race is the class of guard that
  passes review and fails in production.
- Verify with one boundary and execute through a second injectable boundary:
  rejected; a composition can satisfy both fakes while still executing by path,
  so verification and execution stay operations on one trait.
- Hold the provider open with `O_PATH` and exec that: rejected; measured,
  `pread` on an `O_PATH` descriptor returns `EBADF`, so identity would need a
  second open and the gap returns.
- Use `fexecve` from libc: rejected; glibc falls back to
  `/proc/self/fd/<n>` when `execveat` is unavailable, which is a reopen by
  path, and the fallback is silent.

## Decision 16: Measure third-party build scripts and pin the action environment

**Decision**: W0 enumerates, per hub, every third-party crate for which
`crate_universe` generates a build-script target, records the annotations each
requires, and pins an explicit minimal action-environment allowlist. The
enumeration, the annotation set, and the allowlist are generator outputs and
cache-key inputs.

**Why this is a measurement task and not a risk row.** The workspace has no
first-party build scripts and no first-party proc-macro crates, which the ADR
verified. But the hub locks resolve several hundred third-party packages each,
and build scripts among them run as Bazel actions in a restricted environment.
Host-probing scripts diverge from Cargo behavior and can fail under sandboxing.
The only levers are the per-crate annotations and the build-script generation
switch, so the set of crates needing each lever has to be known before W1
carriers depend on them.

**Any action-environment change invalidates the whole action cache** and must
therefore be included in the cache-key and budget review rather than added
casually.

**Alternatives**:

- Discover build-script breakage during W1: rejected; it would surface as
  unattributable carrier failures after the graph already exists.
- A permissive action environment: rejected; it makes actions host-dependent
  and defeats cache reuse across runners.

## Upstream evidence

These claims were verified upstream against the pinned versions during the
review that forced the ADR amendment. They are recorded here so a future
reader does not re-derive them from documentation that disagrees.

| Claim | Source |
| --- | --- |
| Bazel 8.6.0 exists and Bazel 9 is current, so 8 is a prior LTS line | Bazel release metadata |
| `exclusive` serializes tests and runs them after all other build and test activity | `bazelbuild/bazel` `SkyframeBuilder.java` at `refs/tags/8.6.0` |
| `exclusive` implies no remote execution for those tests | `bazelbuild/bazel` `TestTargetProperties.java` at `refs/tags/8.6.0` |
| Tag-derived custom resources are discarded whenever `--local_test_jobs` is set | `TestTargetProperties.getLocalResourceUsage` at `refs/tags/8.6.0` |
| `--runs_per_test` is exempt from `--cache_test_results=auto` | Bazel 8.6.0 user manual |
| `common` in an rc file ignores inapplicable options; `test` inherits from `build` | Bazel bazelrc documentation |
| Disk-cache garbage collection runs asynchronously while the server idles, with a default 5-minute idle delay | Bazel 8.6.0 caching documentation |
| An on-demand disk-cache collector `//src/tools/diskcache:gc` exists at 8.6.0 | `bazelbuild/bazel` `src/tools/diskcache/BUILD` at `refs/tags/8.6.0` |
| `%workspace%` is resolved only for rc import paths and a specific Java option set, not for startup options | `bazelbuild/bazel` client rc handling and option classes at `refs/tags/8.6.0` |
| `rules_rust` 0.73.0 is registry-published and CI-tested against Bazel 7.x, 8.x, and 9.x, with no restrictive `bazel_compatibility` | Bazel Central Registry metadata and presubmit for 0.73.0 |
| `crate.from_cargo` accepts `cargo_lockfile`, `lockfile`, and `manifests`, and duplicate hub names are rejected | `rules_rust` `crate_universe/extensions.bzl` at `refs/tags/0.73.0` |
| Repin is fail-closed on a stale committed lock, and repin is unconditional when `lockfile` is omitted | `rules_rust` `crate_universe/private/generate_utils.bzl` and `extensions.bzl` at `refs/tags/0.73.0` |
| `CARGO_BAZEL_REPIN_ONLY` is a comma-delimited allowlist matched by exact hub repository name in `determine_repin`, and any repin value outside `false`, `no`, `0`, `off` is truthy | `rules_rust` `crate_universe/private/generate_utils.bzl` and `common_utils.bzl` at `refs/tags/0.73.0` |
| `skip_cargo_lockfile_overwrite` defaults to `False`, so a repin writes the plain `Cargo.lock` back unless the hub opts out | `rules_rust` `crate_universe/extensions.bzl` at `refs/tags/0.73.0` |
| The extension reports itself reproducible only when both `lockfile` and `cargo_lockfile` are set, which is what makes `--lockfile_mode=error` bind it | `rules_rust` `crate_universe/extensions.bzl` at `refs/tags/0.73.0` |
| The 0.73.0 docstring recommends `bazel sync --only=<hub>` while the bzlmod `regen_command` default is `bazel mod show_repo`, so neither can be trusted as the repin invocation without measurement | `rules_rust` `crate_universe/extensions.bzl` at `refs/tags/0.73.0` |
| `--lockfile_mode=error` never rewrites `MODULE.bazel.lock` and fails instead | Bazel 8.6.0 external-dependency lockfile documentation, confirmed by measurement in this worktree |
| `--lockfile_mode=error` fails on a missing registry file checksum, not on a changed `MODULE.bazel`, and exits 48 naming `bazel mod deps --lockfile_mode=update` | Measured at Bazel 8.6.0 on a scratch module graph in this worktree |
| `bazel mod deps --lockfile_mode=update` with absolute startup options changes only `MODULE.bazel.lock`, adds and removes no file, and changes nothing on a second identical run | Measured at Bazel 8.6.0 on a scratch module graph in this worktree |
| `bazel mod` rejects `--symlink_prefix` as an unrecognized option, so command-option identity cannot span `mod` and the build commands; startup-option identity can | Measured at Bazel 8.6.0 in this worktree |
| A direct `bazel_dep` version the graph absorbs produces only a warning and exit zero under `--lockfile_mode=error`, so `--check_direct_dependencies=error` is required to fail closed | Measured at Bazel 8.6.0 on a scratch module graph in this worktree |
| `cargo-bazel` is downloaded by URL and sha256 in the registry release form, and built from source through a non-reproducible bootstrap otherwise | `rules_rust` `crate_universe/private/urls.bzl` and `internal_extensions.bzl` at `refs/tags/0.73.0` |
| Stable and nightly toolchains register together, one version per channel | `rules_rust` `rust/extensions.bzl` at `refs/tags/0.73.0` |
| The toolchain channel is a universal-scope global build setting with no public per-target transition | `rules_rust` `rust/toolchain/channel/BUILD.bazel` and `rust/private/unpretty.bzl` at `refs/tags/0.73.0` |
| `rust_doc_test` emits a generated shell runner on a non-nightly channel | `rules_rust` `rust/private/rustdoc_test.bzl` at `refs/tags/0.73.0` |
| No rustdoc-JSON rule exists in `rules_rust` 0.73.0 | `rules_rust` `rust/defs.bzl` at `refs/tags/0.73.0` |
| A runfiles library exists at the pin | `rules_rust` `tools/runfiles` at `refs/tags/0.73.0` |
| The Bazel repository cache has no enumeration interface and is reached only by a checksum-declared download | Bazel 8.6.0 caching documentation |

Repository facts re-measured in this worktree rather than taken from prose: 25
files under `packages/` reference `CARGO_BIN_EXE_`; 50 reference
`CARGO_MANIFEST_DIR`, 20 of them test files; the no-bash walker has its own
`Cargo.toml` and `Cargo.lock` under `tests/tools/no-bash-ast-walker/`; five
`[[test]] harness = false` entries exist in `packages/d2b-core/Cargo.toml`, of
which four carry `required-features = ["fuzz"]` and `fuzz` is not a default
feature, plus one `[[bench]] harness = false` in
`packages/d2b-zone-routing/Cargo.toml` that the gate's discovery filters out;
`tests/test-rust.sh` sets `workspace_test_excludes=(--exclude
d2b-contract-tests)` and `packages/d2b-contract-tests/tests/policy_broker_schema.rs`
walks every `.rs` file under `packages/`, so that crate is invisible to every
workspace leaf and its inputs include the whole first-party Rust tree;
`flake.nix` exposes no Bazel tooling today; and the repository has no root
`BUILD.bazel`, `MODULE.bazel`, `.bazelrc`, `.bazelignore`, or `.bazelversion`,
so the migration is greenfield.
