# Feature Specification: Implement ADR 0052 Under ADR 0054

**Delivery track**: Track A

**Feature branch**: `spec003-adr0054-amend`

**Created**: 2026-08-02

**Amended**: 2026-08-05

**Status**: Draft amendment awaiting a new plan panel

**Authority**: [ADR 0052](../../docs/adr/0052-bazel-rust-build-and-test.md),
as amended by
[ADR 0054](../../docs/adr/0054-single-product-cargo-workspace.md)

## Context

Spec 003 migrates the eighteen Rust execution-manifest surfaces behind
`test-rust` from Cargo scheduling to Bazel scheduling through a reversible
shadow period. The product goal is unchanged: remove duplicated compilation
and scheduling while preserving exact coverage, test topology, supply-chain
policy, failure attribution, performance ceilings, cache boundaries, and the
public Make interface.

ADR 0054 changes the dependency model beneath that goal. Product Rust now has
one resolver-v2 Cargo workspace rooted at `packages/Cargo.toml`, one
authoritative `packages/Cargo.lock`, and one `crate_universe` hub named
`product`. The workspace includes the existing main members,
`d2b-priv-broker`, and `d2b-guest-shell-runner`. The no-bash AST walker remains
a separate workspace with its own Cargo lock and one hub named `walker`.
`packages/Cargo.guest.lock` remains a generated static-guest closure input. It
is not a Cargo workspace lock for this migration and is not a
`crate_universe` authority.

The single product lock is a dependency-resolution union, not a build-closure
claim. Broker and guest isolation remains enforced by package, feature, and
target selected Cargo commands; dedicated Nix derivations; package-scoped
selected-closure policy; static ELF checks; and native first-party Bazel
targets with explicit dependencies. The product external repository may be a
third-party package and feature superset. It does not define actual
first-party edges.

At committed base `a7093601`, no Spec 003 implementation is merged. The root
workspace still excludes broker and guest, both remain nested workspaces with
their own locks, and no Bazel workspace file exists. Parked historical
branches `spec003-w0-*` and `spec003-w0`, plus the unified Bazel spike, are
evidence about a validated shape,
not ancestors to merge or assumptions about the base. Implementation restarts
from the merged `v3` lineage after this amended artifact set passes a new plan
panel.

This remains a Track A feature because it changes the required Rust gate,
dependency and policy authority, workflow structure, cache behavior, and
promotion path. It does not change daemon behavior, broker operations, guest
runtime behavior, public wire schemas, or the daemon-only control-plane
invariant.

## Scope

In scope:

- the eighteen Rust execution-manifest surfaces behind `test-rust`;
- one product Cargo workspace and lock plus the separate walker workspace and
  lock;
- package-selected broker and guest Cargo lanes;
- dedicated broker and static guest Nix derivations using the root product
  lock;
- one `product` and one `walker` `crate_universe` hub;
- native first-party Bazel targets for product crates and configured broker and
  guest contexts;
- package-scoped broker GNU and guest musl policy inputs for
  `x86_64-linux` and `aarch64-linux`;
- exact selected-source identity and checksum enforcement;
- pinned RustSec `--no-fetch` package audits;
- native realization of package policy and guest static ELF checks on both
  architectures;
- the shadow, qualification, promotion, compatibility, and retirement
  lifecycle decided by ADR 0052.

Out of scope:

- Bazel building Nix outputs or replacing Nix derivations;
- VM images, NixOS module behavior, fixture materialization, or release
  artifact redesign;
- remote execution or a remote Bazel cache;
- a new Layer-1 job, required context, linter, formatter, or hook;
- merging the no-bash walker into the product workspace;
- treating `packages/Cargo.guest.lock` as a Cargo or Bazel dependency
  authority;
- weakening dependency, license, advisory, static ELF, binary-size, or closure
  isolation policy.

## Clarifications

### Workspace and dependency authority

- The product workspace uses resolver version 2 and contains main, broker, and
  guest packages.
- `packages/Cargo.lock` is the only authoritative product Cargo lock.
- The walker keeps
  `tests/tools/no-bash-ast-walker/{Cargo.toml,Cargo.lock}`.
- The nested broker and guest `[workspace]` and `[profile.*]` tables and locks
  are removed. No forwarding lock or synthetic splice workspace remains.
- Lock refresh follows the authority that changed. A product manifest change
  regenerates `packages/Cargo.lock`, then the product hub lock, then
  `MODULE.bazel.lock` last, and leaves the walker Cargo lock and
  `bazel/cargo/walker.lock` byte-identical. A walker manifest or lock change
  regenerates the walker Cargo lock, then `bazel/cargo/walker.lock`, then
  `MODULE.bazel.lock` last, and leaves `packages/Cargo.lock` and
  `bazel/cargo/product.lock` byte-identical. Initial or combined setup
  generates the product hub lock, then the walker hub lock, then
  `MODULE.bazel.lock` last. The module lock is always refreshed last.
- `libshpool` is a normal `libshpool = "0.11.0"` dependency. The
  `real-libshpool` feature remains, with an empty dependency activation list,
  and continues to gate code through `cfg(feature = "real-libshpool")`.
- Generated Bazel metadata must not use `crate.spec` for `libshpool`.

### Selected build contexts

- Broker default, `layer1-bootstrap`, and `fake-backends` remain three serial
  `cargo test` contexts with default features disabled and distinct target
  directories.
- Their Bazel suites carry exactly `tags = ["exclusive"]`, run after all
  ordinary tests, and cannot overlap each other or any other test. Removing
  the tag is a required scheduling mutation. Qualification runs each context
  twenty consecutive times with `--runs_per_test=20`.
- Guest production uses package
  `d2b-guest-shell-runner`, default features disabled, and feature
  `real-libshpool`.
- Guest formatting is exactly package-scoped
  `cargo fmt -p d2b-guest-shell-runner --check`; it takes no `--locked`,
  default-feature, or feature selector.
- Generic main Clippy and tests exclude broker and guest. Generic tests also
  retain the existing `d2b-contract-tests` exclusion and companion fixture
  lane. Generic main Clippy continues to compile `d2b-contract-tests`.
- Generic Nix build/test and Clippy contexts exclude broker and guest exactly.
  Dedicated broker and guest contexts retain their exact package,
  default-feature, feature, target, and dependency-kind selection.
- Nix builds name the package, binary, default-feature state, and guest feature
  explicitly. They remain dedicated derivations so binary size, dynamic
  linkage, static PIE posture, and closure isolation stay independently
  reviewable.
- Both dedicated Nix derivations retain
  `cargoLock.outputHashes."wl-proxy-0.1.2" =
  "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8="`.

### Hubs and contributor mutation

- The only accepted hub identifiers are `product` and `walker`.
- `main`, `broker`, and `guest` are retired, not aliases.
- Each retired identifier fails before Bazel starts and emits the exact ADR
  0054 diagnostic for that identifier.
- Tests use an injected non-mutating executor. They assert argv
  `cargo xtask bazel-repin --hub product` and cwd `packages/`; they never run a
  genuine repin.
- A duplicated `cd packages` or `packages/` path prefix is rejected because it
  would resolve to `packages/packages`.
- Contributor repin and policy generation are entered from repository root
  with `nix develop`, followed by `cd packages` and the documented
  `cargo xtask` command. They remain unreachable from workflows and Make.
- `cargo xtask bazel-module-refresh` is the only module-lock mutation. It
  takes no arguments, uses the same absolute server-selecting startup options,
  changes only `MODULE.bazel.lock`, is idempotent, and is the exact
  remediation for module drift.
- `cargo generate-lockfile --offline` is the only product-lock regeneration
  command. It is contributor-only and is unreachable from Make and workflows.
- Final validation surrounds every generate or repin command with clean
  worktree assertions and fails if the committed candidate changes.

### Action network and yanked authority

- Bazel Rust actions remain no-network under ADR 0052, including loopback TCP
  and Unix sockets. Linux network namespaces enforce only the action-wide
  boundary and are not claimed to enforce per-endpoint declarations.
- Only pinned repository rules may fetch: registry sources use the root lock
  URL and checksum, and `wl-proxy` uses its pinned revision and archive
  sha256.
- Mandatory committed socket-using tests remain on exact non-Bazel Cargo
  compatibility carriers under their existing surface IDs until a separate
  authorized design exists. Their generated case census, same-commit verdict,
  and non-advisory classification are qualification inputs and survive Cargo
  implementation retirement.
- Plants deny loopback TCP, Unix sockets, external egress, and live-index
  access from Bazel actions. Promotion identifies affected surfaces as hybrid
  and never claims those compatibility cases ran under Bazel.
- One committed yanked snapshot has the exact key set derived only from
  `packages/Cargo.lock`; the walker lock and `Cargo.guest.lock` are excluded.
- `rust-deny-main` evaluates the full product snapshot. Broker and guest deny
  carriers evaluate exact projections of their selected root-dev-inclusive
  package-policy graphs against the same snapshot.
- `bazel-yanked-refresh` is the reviewed networked contributor mutation;
  `bazel-yanked-check` is the offline exact-key validator. Neither is
  reachable from Make or workflows.

### Package-scoped policy

- Four selected production contexts exist: broker GNU and guest musl for each
  of the root flake's two systems.
- Each context has a production graph and a root-dev-inclusive policy graph.
  Both are generated from locked, offline root metadata.
- Before policy evaluation, each checker proves one selected root, a nonempty
  complete graph, exact edge kinds, exact system and target, the exact sorted
  selected-source identity set, the exact source count, readability, and every
  registry checksum or pinned git revision and archive checksum.
- Metadata and filtered locks must contain equal
  `(name, version, source)` identity sets before `cargo-deny` or `cargo-audit`
  starts.
- Package audits use the pinned RustSec database and `--no-fetch`. Broker has
  no ignore. Guest has exactly `RUSTSEC-2024-0384`.
- The six current guest real-libshpool license findings are an implementation
  task, not a waiver: BSD-3-Clause for `bindgen` and `instant`, ISC for
  `inotify`, `inotify-sys`, and `libloading`, and CC0-1.0 for `notify`.
  The guest policy update is package-scoped to those six package and license
  pairs. Adding a license to the global allowlist does not satisfy this
  requirement.

### Architecture realization

- Broker package contexts use matching GNU targets:
  `x86_64-unknown-linux-gnu` and `aarch64-unknown-linux-gnu`.
- Guest static contexts use matching musl targets:
  `x86_64-unknown-linux-musl` and `aarch64-unknown-linux-musl`.
- The four package checks, `broker-host-artifact-contract`, and
  `guest-static-elf` realize natively on both architectures. No lane passes a
  foreign `--system`, configures
  `--builders`, or depends on a remote builder.
- The existing `test-flake-aarch64` job ID and rollup context remain. Its
  implementation changes from x86-hosted eval-only smoke to native
  `ubuntu-24.04-arm` realization with a 60-minute bound.
- That native job also runs `make test-rust-supply-chain`. Renderer tests bind
  the command, and realization plus supply-chain evidence must come from one
  unchanged stable PR head.
- Aarch64 broker artifacts prove evaluation and build support only. They do
  not expand ADR 0008 runtime support.

## User Scenarios and Testing

### User Story 1 - Run a complete Bazel Rust gate beside Cargo - P1

As a contributor, I can run a Bazel-backed aggregate and four attributed
slices covering the same eighteen Rust surfaces while the Cargo path remains
authoritative during shadow.

Acceptance:

1. A passing aggregate publishes all eighteen existing v1 surface IDs.
2. A carrier failure names its surface and leaves schema-valid partial
   evidence.
3. Fixture-backed contract surfaces remain on the Cargo and Nix fixture lane.
4. The root Cargo workspace and lock are used with explicit package and feature
   selectors for main, broker, and guest.
5. Product and walker are the only dependency hubs.

### User Story 2 - Preserve exact coverage and test isolation - P1

As a maintainer, I can prove Bazel executes the complete Rust test and policy
surface with the current process-isolation properties.

Acceptance:

1. Every baseline ID has a nonempty carrier set and every carrier belongs to
   exactly one ID.
2. Repository scans and generated-output checks refuse empty, incomplete,
   unreadable, unparsable, or stale censuses.
3. Main and guest tests run one fresh process per case with ignored-case
   accounting and structured per-case evidence.
4. Broker feature contexts retain one process per test binary, bounded threads,
   target isolation, exact `exclusive` tags, and non-overlap with every test.
5. Doctest and harness-free companions remain independently discovered and
   refuse an unexpectedly empty set.
6. First-party binaries and fixtures are declared inputs, resolved once, and
   never silently taken from another executor.
7. Mandatory socket-using cases keep exact same-commit Cargo compatibility
   coverage, while every Bazel action remains no-network.

### User Story 3 - Keep supply-chain policy enforcing - P1

As a security or release maintainer, I can prove the selected broker and guest
closures, sources, licenses, and advisories for each supported flake system.

Acceptance:

1. Broker GNU and guest musl production and policy graphs are generated from
   the root lock for both systems.
2. The exact selected-source census and every checksum or pinned git identity
   is verified before policy tools run.
3. Package deny checks run over root-dev-inclusive metadata without
   `--exclude-dev`.
4. Package audits use the pinned RustSec database with `--no-fetch`.
5. Broker has no advisory ignore and guest has only
   `RUSTSEC-2024-0384`.
6. The six existing guest license findings require a narrow package-scoped
   update and no global license-policy expansion.
7. Aggregate root-lock and `Cargo.guest.lock` checks remain enforcing and may
   block independently.
8. Missing root, empty closure, wrong system, wrong target, wrong edge kind,
   source identity, checksum, policy, and cross-context plants each fail at the
   predicate they mutate.

### User Story 4 - Get faster, bounded local feedback - P2

As a contributor, I receive bounded concurrency, persistent incremental reuse,
predictable disk use, safe cleanup, and actionable failure reporting.

Acceptance:

1. Warm and cold local profiles retain the ADR 0052 ceilings and measurement
   rules.
2. `D2B_RUST_BUDGET` remains the only resource control.
3. Local Bazel state stays under `.scratch/bazel/` with soft and hard limits.
4. Unsafe or live cleanup layouts refuse before deletion.
5. Deadline, filesystem, provider, and result-publication negatives use
   injected boundaries rather than ambient host failures.
6. Provider execution uses one verified descriptor and no fallback; expiry
   observes without consuming throughout the full grace, kills the group, and
   only then reaps.
7. Each cleanup or server code emits its exact ADR 0052 recovery commands and
   no other code's remedy.
8. JUnit, `test.log`, execution evidence, qualification evidence, and exporter
   diagnostics are sanitized and bounded before publication. Degraded evidence
   preserves the underlying test verdict and is rejected separately.

### User Story 5 - Compare safely in continuous integration - P2

As a maintainer, I can collect trustworthy shadow and dual-architecture
evidence without evicting required caches, exposing credentials, or granting
pull requests write capability.

Acceptance:

1. Shadow CI is non-required and publishes no Bazel cache.
2. Pull-request-reachable jobs have read-only permissions and no cache writer.
3. Qualification records pair Cargo and Bazel verdicts on the same protected
   `v3` push commit, include the fixture companion, and carry explicit
   `bazelRestoreCount`, `bazelSaveCount`, and `bazelPublicationCount`
   plus four complete `sliceDurationsSeconds` entries.
4. Native x86_64 and aarch64 lanes each realize their own six policy and artifact
   checks without foreign-system or remote-builder arguments; the arm lane
   also runs the native package supply-chain target on the same stable head.
5. Cache maintenance remains a separate verdict and credentials never enter
   Bazel or repository code.

### User Story 6 - Promote and retire without breaking contracts - P3

As a maintainer, I can promote only after complete evidence exists, keep the
required context and public Make names stable, and retire the Cargo
implementation in separately reversible steps.

Acceptance:

1. Incomplete coverage, failure plants, topology, package policy,
   architecture realization, or performance evidence blocks promotion.
2. Promotion keeps the required context `test-rust` and the fixture lane.
3. Bazel-specific aliases forward with status preservation until a published
   semantic release tag contains promotion.
4. Cargo implementation retirement waits for ten distinct ordered green
   promoted `v3` run units and removes no public target name.

## Functional Requirements

- **FR-001**: The feature MUST remain limited to the Rust gate and the narrow
  Cargo, Nix derivation, package-policy, and dual-system realization changes
  ADR 0054 requires.
- **FR-002**: `packages/Cargo.toml` MUST be one resolver-v2 product workspace
  containing main, broker, and guest, and `packages/Cargo.lock` MUST be its only
  authoritative product lock. The no-bash walker MUST retain its separate
  manifest and lock. `packages/Cargo.guest.lock` MUST be treated only as a
  generated static-guest closure input.
- **FR-003**: Guest format MUST be exactly package-only
  `cargo fmt -p d2b-guest-shell-runner --check`, without `--locked` or feature
  selectors. Every dependency-resolving broker and guest command MUST use
  `--locked`, exact package/default-feature/feature selectors, and the
  gate-owned target directory. Generic main Clippy and tests MUST use the exact
  distinct ADR 0054 exclusion sets.
- **FR-004**: Bazel, Bzlmod, `rules_rust`, `cargo-bazel`, Rust toolchains,
  module locks, and Bazel-side hub locks MUST remain pinned and MUST have only
  repository-owned regeneration paths.
- **FR-005**: Shadow MUST add the Bazel aggregate, four slices, and shutdown
  entry point while keeping Cargo authoritative.
- **FR-006**: Workflows MUST call approved Make targets and MUST NOT invoke
  Bazel or contributor mutation commands directly.
- **FR-007**: The aggregate MUST represent exactly the eighteen baseline Rust
  surface IDs and no fixture-backed conditional ID.
- **FR-008**: The coverage map MUST associate every surface with a nonempty
  carrier set, one slice, an exact derived census, and a topology where
  applicable.
- **FR-009**: Coverage validation MUST fail on missing or duplicate IDs,
  carriers, test targets, censuses, topologies, queries, or hand-written
  fragments. No Bazel test may start a nested Bazel server.
- **FR-010**: Every logical check MUST retain an independently attributable
  verdict.
- **FR-011**: Execution-manifest v1 identifiers, completion, failure, and
  interruption semantics MUST remain unchanged.
- **FR-012**: Main and guest test suites MUST retain one fresh process per
  case, exact census, and faithful ignored-case reporting.
- **FR-013**: Broker default, layer1, and fake contexts MUST retain one process
  per test binary, bounded threads, distinct target directories, literal
  `tags = ["exclusive"]`, and non-overlap with every other test.
- **FR-014**: Doctest and harness-free companions MUST be derived from the
  current selectors, independently executed, and nonempty where required.
- **FR-015**: Every repository scan and generated-output comparison MUST prove
  an exact nonempty census before evaluating an absence or equality predicate.
- **FR-016**: Schema reproducibility MUST compare two independent generations
  against the generator-returned exact census.
- **FR-017**: The no-bash scan MUST prove equality among the governed manifest,
  declared inputs, and parsed files while remaining in the separate walker
  workspace. Walk, read, or parse failure MUST refuse rather than skip.
- **FR-018**: First-party tests MUST resolve declared binaries and fixtures
  through the executor-specific arm selected once, verify one opened provider
  handle opened `O_RDONLY|O_CLOEXEC` with
  `RESOLVE_NO_MAGICLINKS` and deliberately no `RESOLVE_BENEATH` or
  `RESOLVE_NO_SYMLINKS`, and execute that same handle using
  `execveat(..., AT_EMPTY_PATH)` without a path, `fexecve`, `/proc`, or
  `ENOSYS` fallback. The forced walk MUST use `O_NOFOLLOW` on intermediate
  components but not on the provider leaf. Strict result, cleanup, and
  evidence paths MUST retain
  `RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS`. Every auxiliary
  descriptor MUST be close-on-exec and behaviorally tested.
  `VerifiedExecutable` MUST have private fields and sealed construction with no
  public unchecked constructor, path accessor or conversion, `Default`,
  `From`, `Into`, `AsRef`, `Clone`, or `Copy`, enforced by compile-fail API
  fixtures. Argv, envp, descriptor mappings, and the fixed exec-error record
  MUST be prepared before fork; the child MUST perform only async-signal-safe
  raw descriptor operations, `execveat`, a fixed-size error-pipe write, and
  `_exit`.
- **FR-019**: `crate_universe` MUST declare exactly `product` from
  `packages/{Cargo.toml,Cargo.lock}` and `walker` from the no-bash walker
  manifest and lock. Product first-party crates MUST be native Bazel targets.
- **FR-020**: Broker and guest configured native targets MUST declare their
  own direct first-party and `@product` dependencies, cfgs, and features.
  Exact third-party feature parity with the product external union MUST NOT be
  required.
- **FR-021**: `libshpool` MUST be a normal dependency while code activation
  remains behind `real-libshpool`; generated or hand-written `crate.spec` use
  for it MUST be rejected.
- **FR-022**: The only accepted repin hubs MUST be `product` and `walker`.
  Retired `main`, `broker`, and `guest` inputs MUST fail before Bazel starts
  with their exact ADR 0054 diagnostics.
- **FR-023**: Repin refusal tests MUST use an injected non-mutating executor,
  exact product argv, and cwd `packages/`, and MUST reject a duplicated
  packages prefix.
- **FR-024**: Contributor repin, module refresh, yanked refresh/check, policy
  generation, product-lock `cargo generate-lockfile --offline`, and evidence
  mutation commands MUST remain unreachable from workflows and Make.
- **FR-025**: Package policy inputs MUST be generated for broker GNU and guest
  musl on x86_64-linux and aarch64-linux from locked, offline root metadata.
- **FR-026**: Every production and policy graph MUST bind selected root,
  system, target, package identity, version, source, checksum, edge kind, cfg,
  and resolved features.
- **FR-027**: Before deny or audit, the checker MUST prove the exact nonempty
  selected-source set, count, readability, identity, checksums, and equality
  between metadata and filtered-lock identities.
- **FR-028**: Package deny MUST evaluate the root-dev-inclusive policy graph
  without `--exclude-dev`; package audit MUST use the pinned RustSec database
  with `--no-fetch`.
- **FR-029**: Broker package audit MUST have no ignore. Guest package audit
  MUST have exactly `RUSTSEC-2024-0384`. Existing aggregate ignores MUST remain
  unchanged.
- **FR-030**: Guest real-libshpool license policy MUST be updated only for the
  six named package and license pairs. A global license allowlist expansion is
  forbidden.
- **FR-031**: Dedicated broker and guest Nix derivations MUST consume the root
  source and root lock with explicit package, binary, default-feature, and
  guest feature selectors, and both MUST retain the exact pinned
  `cargoLock.outputHashes."wl-proxy-0.1.2"` value.
- **FR-032**: Broker dynamic-host and guest static-PIE, interpreter,
  `NEEDED`, native `e_machine`, binary-size, and closure-isolation checks MUST
  remain independently enforcing. Guest ELF checks MUST require `ET_DYN` and
  reject a non-PIE or wrong-machine artifact. Both dedicated derivations MUST
  realize natively. The broker interpreter and `DT_NEEDED` SONAME set and both
  recursive Nix closures MUST match committed exact baselines. Size thresholds
  MUST come from realized measured baselines plus separately committed,
  reviewed measured deltas; no prose byte ceiling is accepted.
- **FR-033**: The four package checks and `guest-static-elf` MUST exist and
  realize for both root flake systems, each reading only its exact
  system-and-target policy input.
- **FR-034**: `test-flake-aarch64` MUST retain its job ID and required rollup
  role while moving to native `ubuntu-24.04-arm` realization with no foreign
  system or remote builder, and MUST run `make test-rust-supply-chain` on that
  arm runner.
- **FR-035**: `make test-rust-supply-chain`, `make test-drift`, and
  `make test-flake` MUST carry recurring source, policy, mapping, refusal,
  inventory, pin, and realization enforcement without adding a Layer-1 job.
- **FR-036**: Local concurrency MUST use `D2B_RUST_BUDGET` and remain bounded
  across scheduler and suite concurrency.
- **FR-037**: Local Bazel state MUST remain beneath ignored worktree scratch,
  have size and age bounds, and use synchronous trimming before measurement.
- **FR-038**: Cleanup MUST be descriptor-relative, refuse unsafe or live
  ownership before deletion, reach no tracked or external content, and emit
  the exact ADR 0052 command sequence for its own code.
- **FR-039**: Refusal and result messages MUST omit local identifiers, secrets,
  raw paths, handles, and cross-condition remedies while naming exact
  repository-relative input, corrective action, and rerun command. Provider,
  sanitizer, sink-limit, exporter, publication, and qualification-degradation
  classes MUST each have an exact tested row.
- **FR-040**: The shadow workflow MUST remain non-required, keep the required
  graph unchanged, and publish no cache.
- **FR-041**: Pull-request jobs MUST remain read-only, request no
  `actions: write`, and reach no direct, indirect, post-step, or unknown cache
  writer.
- **FR-042**: Cache credentials MUST remain unavailable to Bazel, build
  scripts, proc macros, and repository run steps.
- **FR-043**: Promotion caching MUST keep action and download caches separate,
  exclude output bases, and bind every Cargo, hub, policy, toolchain, generated
  graph, target-map, and action-environment input. Primary keys MUST be unique
  per successful protected-`v3` run; restore prefixes MUST omit run ID and
  commit SHA; retention MUST preserve the newest complete generation.
- **FR-044**: Cache maintenance MUST run only on protected `v3`, paginate
  completely, derive authorization only from a closed committed typed prefix
  enum, reject caller-supplied, unknown, or ambiguous prefixes, preserve
  unauthorized entries, delete only authorized generations, verify headroom
  twice, and remain outside the Rust verdict. Mixed authorized and unauthorized
  entries across pagination boundaries MUST be mutation-tested.
- **FR-045**: Warm local, cold local, and cold CI profiles MUST retain ADR
  0052's 10, 15, and 15 minute ceilings and 1.2 maximum multiplier.
- **FR-046**: A promoted job MUST enforce the ADR 0052 in-band deadline and
  process-group cleanup, with the outer timeout only as a backstop.
- **FR-047**: A missed ceiling MUST authorize only a larger runner or a further
  disjoint slice split, never weaker coverage or a relaxed ceiling.
- **FR-048**: Promotion MUST require exact coverage, the isolated
  eighteen-surface failure matrix, topology, selected package policy, native
  dual-architecture realization, performance, cache, and ten-record
  equivalence evidence, including the three explicit camelCase cache counts,
  broker twenty-run exclusivity, all four Bazel no-network plants, exact
  same-commit Cargo compatibility-carrier coverage, complete sink
  sanitization/bounds, non-advisory arm/four-slice/rollup classification, both
  realized artifact contracts, and arm supply-chain stable-head evidence.
- **FR-049**: Promotion MUST preserve required context `test-rust`, public
  `test-rust-*` names, and the fixture lane.
- **FR-050**: Bazel-specific aliases MUST forward with status preservation and
  MUST NOT be called by workflows after promotion.
- **FR-051**: Alias removal MUST wait for a release containing promotion and
  MUST be a separate change. The release MUST be neither draft nor prerelease.
- **FR-052**: Cargo implementation retirement MUST wait for ten consecutive
  green promoted `v3` runs and MUST remove no public Make name or fixture
  behavior.
- **FR-053**: Every guard MUST land with a positive case and a planted negative
  or mutation in an existing Rust, policy, drift, or workflow-policy carrier.
- **FR-054**: Migrated test suites MUST publish enforcing, redacted per-case
  result documents. JUnit, `test.log`, emitted evidence, and exporter
  diagnostics MUST be sanitized and bounded before writing and MUST contain no
  forbidden planted value. `testVerdict` MUST remain the underlying operation
  result; publication failure MUST emit separately typed degraded evidence
  that surface completion and qualification reject.
- **FR-055**: No new top-level shell gate, Layer-1 job, required context,
  linter, formatter, hook, remote cache, or remote execution surface may be
  added.
- **FR-056**: Qualification MUST run each broker context twenty consecutive
  times with `--runs_per_test=20`, exclusivity in force, and a passing
  tag-removal/overlap mutation.
- **FR-057**: Bazel Rust actions MUST use no sockets, including loopback TCP
  and Unix sockets. Only pinned
  repository-rule fetches are permitted. Loopback, Unix-socket,
  forbidden-external-egress, and live-index plants MUST fail and enter
  qualification. Mandatory socket-using Rust tests MUST remain on an exact
  non-advisory Cargo compatibility path under their existing surface IDs until
  separately authorized; qualification MUST bind their same-commit census and
  verdict and MUST NOT claim namespace isolation enforces endpoint
  declarations.
- **FR-058**: `cargo xtask bazel-module-refresh` MUST be test-first,
  no-argument, `MODULE.bazel.lock`-only, idempotent, and use the same absolute
  server-selecting startup options. Module drift MUST name its exact
  repository remediation and no Make or workflow path may reach it.
- **FR-059**: Recovery messages MUST preserve ADR 0052's per-code command
  sequences and external-content prohibitions. Redaction, missing-remedy,
  borrowed-remedy, wrong-external-target, replacement-directory,
  recursive-removal, and manual-signal mutations MUST fail.
- **FR-060**: Release containment and green-run qualification remain
  independent inputs. `spec003w7` qualification and code preparation MAY run
  before `spec003w6`, but its shared documentation/evidence task and merge MUST
  depend on merged `spec003w6`, then rebase, revalidate, and receive a new
  panel result. This keeps concurrently ready file ownership disjoint.
- **FR-061**: Every qualification record MUST carry the explicit camelCase
  fields `bazelRestoreCount`, `bazelSaveCount`, and
  `bazelPublicationCount`. Every cold record MUST additionally carry
  `sliceDurationsSeconds` with exactly four complete durations and
  `bazelRestoreCount` of zero. A missing field is a refusal, never an implied
  zero.
- **FR-062**: Deadline expiry MUST repeatedly perform non-consuming,
  nonblocking `waitid(EXITED|NOWAIT|NOHANG)` observations throughout an
  independently timed full grace, treat them as informational only, send
  unconditional group SIGKILL, then reap the direct child. Blocking-wait and
  early-reap mutations MUST fail.
- **FR-063**: The spec003w0 implementation MUST own Nix-unit pin regeneration,
  run `make test-nix-unit`, add new fixture-independent policy binaries to the
  shared `tests/lib.sh` list, prove `make test-policy` runs them, and prove the
  fixture-contract lane excludes them.
- **FR-064**: Final validation MUST run clean-diff assertions before and after
  every generate, repin, refresh, or product-lock regeneration command and
  fail if the committed candidate mutates.
- **FR-065**: The spec003w0 implementation change MUST update `AGENTS.md`,
  `tests/AGENTS.md`, and `docs/contributing/gates-and-lints.md` for the unified
  product workspace. The spec003w5 promotion change MUST update the same files
  plus `tests/README.md` and
  `docs/reference/test-execution-manifest.md` from eight Rust leaves to four
  Bazel slices, because those two reference documents also describe the eight
  CI jobs. These shipped docs MUST contain no process markers.
- **FR-066**: The spec003w1 no-bash scope MUST own
  `tests/tools/no-bash-ast-walker/src/main.rs` and its tests, fail closed on
  walk/read/parse errors, and prove parsed-file census equality with both the
  governed manifest and declared inputs.
- **FR-067**: The post-merge yanked authority MUST be one committed snapshot
  keyed exactly from `packages/Cargo.lock`. Main MUST evaluate the full
  snapshot; broker and guest MUST evaluate exact selected-policy-graph
  projections. Walker and `Cargo.guest.lock` MUST remain excluded.
- **FR-068**: The spec003w0 runner and locator foundation MUST leave no intentionally
  failing test behind an inert implementation. It MUST either implement the
  behavior required for its spec003w0 tests or defer both test and behavior to the
  owning later wave.
- **FR-069**: spec003w0 prep MUST create and register green runner and locator crate
  manifests and roots, with complete future dependencies and stable module
  contract seams, before any runner or locator test starts. Each later prep
  MUST own the relevant crate roots and xtask dependency and contract seams
  without declaring not-yet-present implementation modules. Scope tests MUST
  load their implementation through test-local paths, and only the integrator
  may wire completed modules after the parallel frontier closes.
- **FR-070**: Lock refresh MUST follow the authority that changed. A product
  manifest change MUST regenerate `packages/Cargo.lock`, then repin and commit
  `bazel/cargo/product.lock`, then refresh and commit `MODULE.bazel.lock`
  last, and MUST prove the walker Cargo lock and `bazel/cargo/walker.lock`
  byte-identical. A walker manifest or lock change MUST regenerate the walker
  Cargo lock, then repin and commit `bazel/cargo/walker.lock`, then refresh
  and commit `MODULE.bazel.lock` last, and MUST prove `packages/Cargo.lock`
  and `bazel/cargo/product.lock` byte-identical. Initial or combined setup
  MUST commit the product hub lock, then the walker hub lock, then
  `MODULE.bazel.lock` last. The two initial repins MUST use command-local
  `--lockfile_mode=off` only while the module lock is absent; neither may
  create the module lock, and that mode MUST refuse after bootstrap. Every
  order MUST end in clean no-op validation.
  Hub locks, module locks, Nix pins, generated BUILD files, generated
  inventories, and coverage/query goldens are integrator-generated only.
- **FR-071**: Exact selected package contexts MUST be proved by a three-way
  join. Target-filtered locked offline root Cargo metadata supplies package
  identities, sources, candidate edges, and their `cfg` predicates;
  `packages/Cargo.lock` plus the committed git archive pin supplies registry
  and git checksums; package-selected stable `cargo tree` traversals supply
  the exact root, dependency-kind reach, and resolved features. Metadata
  supplies no checksums, and plain `cargo tree` output MUST NOT be assumed
  machine-readable: every traversal MUST pin `--locked --offline -p <package>
  --target <target> --no-default-features` with explicit `--features`,
  `--charset ascii`, `--prefix depth`, `--no-dedupe`, and a
  repository-pinned `--format` carrying package identity and feature columns
  behind a leading delimiter. Production and dev-inclusive edges MUST be
  traversed separately wherever dependency kind matters, and every traversal
  identity MUST be cross-checked against metadata and the lock. The oracle MUST
  NOT use a synthetic manifest or splice.
- **FR-072**: A feature canary MUST prove feature union cannot leak into broker
  or guest graphs. The canary MUST be an unrelated workspace member that
  enables an otherwise-absent feature on a dependency shared with broker or
  guest, and that feature MUST remain absent from the selected broker and guest
  output. Generic Cargo and Nix build/test and Clippy contexts MUST exclude
  broker and guest exactly, while dedicated contexts retain exact selection.
- **FR-073**: Schema, stub-no-socket, pinned inventory, and no-bash carriers
  MUST be distinct, file-disjoint spec003w1 carriers. Schema MUST run two independent
  nonempty exact-census generations and reject mismatch and empty output.
  Stub and inventory carriers MUST reject every planted missing, wrong,
  empty, extra, runtime-state, and forbidden-listener case assigned to them.
- **FR-074**: The runner MUST explicitly test prior-evidence invalidation,
  multi-carrier attribution, sorted atomic partial manifest v1 evidence for
  success, failure, and handled interruption, original-status preservation,
  ignored-case fidelity, and a planted result containing every forbidden
  redaction value. Repository-owned runner paths MUST invoke no shell, bound to
  the generated inventory of FR-087.
- **FR-075**: `D2B_RUST_BUDGET` MUST be validated once, propagated to Bazel
  scheduling and suite concurrency, and bounded as one combined limit.
  Missing local, invalid, scheduler-only, suite-only, and multiplicative
  combined-limit mutations MUST be covered.
- **FR-076**: Cache tests MUST table-drive every key input named in
  `cache-workflow-boundaries.md` and prove that each applicable action or
  repository primary key and restore prefix changes while the two cache
  namespaces remain distinct.
- **FR-077**: Promotion MUST compare Cargo's current enforcing exit status and
  normalized finding set with the union of decomposed Bazel deny, audit, and
  yanked results for main, broker, and guest. Main uses the full product;
  broker and guest use exact selected projections. Any difference blocks the
  wave and promotion.
- **FR-078**: spec003w0 MUST update
  `.github/workflows/release-host-binaries.yml` for the root manifest, locked
  explicit package/bin/default-feature selectors, root release copy path,
  collapsed workspace cache mapping, and explicit gate target directories.
  It MUST update, not delete,
  `tests/unit/gates/flake-check-matrix-sync.sh` and
  `tests/unit/gates/ci-rust-cache-sync.sh`.
- **FR-079**: The future spec003w0 binding-doc scope MUST update `CONTRIBUTING.md`,
  `docs/contributing/workflow.md`,
  `docs/contributing/critical-subsystems.md`, and
  `packages/d2b-contract-tests/tests/policy_modules.rs` in addition to the
  three existing binding-doc paths. It MUST also correct
  `docs/adr/0052-bazel-rust-build-and-test.md`, `docs/adr/README.md`, and
  `changelog.d/adr0054-broker-hub.md` to call ADR 0054 accepted and describe
  the two-hub model. ADR 0038 MUST remain unchanged; ADR 0054 governs the
  newer workspace shape.
- **FR-080**: Failure contracts MUST define exact nonzero, redacted,
  repository-relative remediation for stale product and walker hub locks,
  module lock drift, generator drift, package-policy drift, yanked snapshot
  drift, ambient repin controls, and unexpected tracked mutation. Each remedy
  MUST include the `nix develop`, `cd packages`, exact command,
  review/commit, and rerun sequence. Exact retired-hub diagnostics remain
  unchanged.
- **FR-081**: Promotion MUST introduce authoritative
  `test-rust-slice-main`, `test-rust-slice-api`,
  `test-rust-slice-broker`, and `test-rust-slice-aux` targets. Generated CI
  calls only those targets. The eight existing public leaves retain their
  exact semantics and map to exact Bazel subsets, including
  `test-rust-main` conditional fixture behavior.
- **FR-082**: Compatibility aliases MUST map `test-bazel-rust` to
  `test-rust` and each `test-bazel-rust-<slice>` to the matching
  `test-rust-slice-<slice>`, print the exact contract line on stderr, and
  preserve status. Promotion docs and changelog announce every replacement,
  and the spec003w6 interface test updates before alias removal.
- **FR-083**: Post-promotion evidence MUST inventory every promoted protected
  `v3` `test-rust` run unit, where a unit is a distinct push-created
  (run ID, head SHA) pair and never an attempt. Attempts `1..max` MUST form
  the complete nested history of exactly one unit; a missing attempt, or
  attempts with conflicting head SHA or promotion provenance, MUST fail. A
  unit's conclusion normalizes to its highest terminal attempt, and no further
  attempt of the same unit may add a streak position. Units MUST be ordered by
  immutable creation order (`createdAt`, then run ID) and MUST NOT be ordered
  by rerun start time, so an old rerun cannot move behind newer failures. Each
  unit records immutable run ID, head SHA, event, branch, complete attempt
  history, terminal conclusion, deterministic ordering metadata, and verified
  promotion ancestry. Pagination gaps, missing or duplicate identities,
  non-v3, non-push, pre-promotion, and nonterminal units MUST fail.
  Eligibility, count, and run IDs MUST be derived, never trusted from
  self-asserted fields. Derivation MUST consume the complete transient stream
  on every run, while `post-promotion.json` persists only a bounded fixed-shape
  stream checkpoint and final ten normalized units with attempt-history
  digests. Persisted bytes and records MUST be schema-bounded and atomically
  replaced, never appended.
- **FR-084**: Retirement MUST require the derived last ten distinct ordered run
  units to be successes with no intervening failure or cancellation.
  Promotion MUST integrate all spec003w5 scope results into one atomic candidate
  relative to the spec003w5 parent, assert its complete path diff, and revert that
  exact commit in rehearsal. Pre-merge rehearsal MUST identify the candidate
  from the verified current atomic candidate HEAD and the recorded spec003w5
  parent; `promotion-record.json` is created only after merge and MUST NOT be
  a pre-merge input. After merge, a typed validator MUST bind
  `promotion-record.json` to the actual protected-`v3` pull-request merge
  commit and the exact sealed `spec003w5` candidate, content, and snapshot
  identities before the follow-up seals or either post-promotion child enters.
  Every code-changing wave MUST own one semantic changelog fragment.

- **FR-085**: The same change that introduces each shadow Make target MUST add
  that target to `APPROVED_MAKE_TARGETS`. All six shadow targets (the
  aggregate, the four slices, and shutdown) MUST be approved in spec003w1,
  with a positive test proving each approved name resolves to a real Makefile
  rule and that a workflow step calling it is accepted, and a negative test
  proving both an unapproved `test-bazel-rust-<name>` call and an approved
  name with no Makefile rule are rejected.
- **FR-086**: A typed qualification validator, implemented no later than
  spec003w3 in `packages/xtask/src/bazel_qualification.rs` with tests and
  exposed as the contributor-only `cargo xtask bazel-qualification-validate`,
  MUST derive every qualification
  threshold from complete paginated, attempt-aware Cargo, Bazel, and fixture
  run inventories plus immutable content references. Every workflow reference
  MUST bind `(runId, positive attempt, headSha)`. It MUST reject page gaps,
  missing attempts, omitted intervening protected-`v3` pushes, omitted,
  forged, duplicate, inconsistent, and wrong-candidate references.
  It MUST normalize each run ID to its highest terminal attempt, derive
  same-head pairing and mismatch resets, and select the five newest qualifying
  cold records from the complete stream.
  `qualification.json` MUST NOT qualify through trusted booleans; boolean
  fields are informational mirrors only, and a mirror that disagrees with the
  derived result is a refusal. Evidence curation and promotion validation MUST
  run the validator, and contributor validation MUST run it before any
  informational inspection.
- **FR-087**: The no-shell property MUST be bound to
  `bazel/generated/no-shell-inventory.json`, an exact, generated,
  drift-checked source, scan-result, and spawn-site inventory. Governed sources
  and declared carrier inputs MUST be equal and nonempty. Every spawn-site
  source MUST be governed, but a governed source MAY have zero spawn sites and
  MUST then carry a successful zero-site scan record. A fresh scan's exact
  spawn-site keys MUST equal the committed `spawnSites` keys in both
  directions. Empty governed input, missing/extra entry, ungoverned spawn,
  missing zero-site scan result, and planted-shell inventories MUST fail. The
  integrator commits the generated inventory; slices preview it only.
- **FR-088**: The spec003w0 Cargo gate MUST take its package supply-chain
  inputs from the native-system selected policy inputs for broker GNU and
  guest musl, with an exact source census, deny, and pinned `--no-fetch`
  audit, while the aggregate root-lock and `Cargo.guest.lock` checks stay
  independent. Guest static dependency policy MUST consume only the selected
  production closure and production lock; policy metadata and lock are
  reserved for deny and audit. The pinned test inventory MUST select packages from the one root
  lock and MUST NOT back up, restore, or otherwise mutate any lock file. Tests
  for both MUST precede implementation.
- **FR-089**: spec003w6 entry MUST require a containing published semantic
  release tag matching `v<major>.<minor>.<patch>` exactly, not any containing
  tag. Entry MUST prove the tag matches the anchored pattern, contains the
  promotion commit, resolves to the same peeled commit locally and on the
  origin remote, and carries a published release that is neither draft nor
  prerelease.
- **FR-090**: A read-only Spec 003 plan-structure validator under this
  specification directory MUST check task-ID uniqueness, dependency
  existence, dependency order, adjacency equality, acyclicity, and overlapping
  ownership between incomparable concurrently ready scopes. It MUST run
  without production-code changes and be required before every plan panel.

## Key Entities

- **Product Workspace**: The resolver-v2 workspace rooted at
  `packages/Cargo.toml` with the authoritative `packages/Cargo.lock`.
- **Walker Workspace**: The separate no-bash AST tool manifest and lock.
- **Dependency Hub**: Exactly `product` or `walker`.
- **Configured First-Party Target**: A native Bazel target whose direct
  first-party dependencies, features, and cfg values describe one selected
  Cargo context.
- **Package Policy Input**: A generated system-and-target-specific production
  or policy graph plus filtered lock.
- **Selected Context Oracle**: A three-way join of locked offline
  target-filtered root Cargo metadata (identities, sources, candidate edges,
  `cfg`), `packages/Cargo.lock` plus the committed git archive pin
  (checksums), and package-selected stable Cargo tree traversals under pinned
  flags (root, dependency-kind reach, resolved features); no synthetic
  manifest or splice.
- **Selected Source Census**: The exact sorted non-path source identities and
  checksums derived from one policy graph and its filtered lock.
- **Product Yanked Snapshot**: One committed product-lock-bounded key set used
  whole by main and by exact selected-graph projections for broker and guest.
- **Verified Executable Handle**: One `O_RDONLY|O_CLOEXEC` provider descriptor
  opened with `RESOLVE_NO_MAGICLINKS` only, whose bytes are verified and
  executed with `execveat(AT_EMPTY_PATH)`.
- **Cache Generation**: One action or repository cache entry with a
  run-unique primary key, run/SHA-free restore prefix, and newest-generation
  retention, counted in a record as `bazelRestoreCount`, `bazelSaveCount`,
  and `bazelPublicationCount`.
- **No-Shell Spawn Inventory**: One generated, drift-checked, nonempty record
  whose governed and declared source sets agree, whose scan results cover
  every governed source including zero-site sources, and whose discovered
  spawn sites are an exact governed subset.
- **Rust Surface**: One of the existing eighteen execution-manifest IDs.
- **Carrier Target**: One independently reported Bazel target assigned to one
  Rust surface.
- **Qualification Record**: A protected `v3` push record pairing Cargo,
  Bazel, and fixture verdicts on one commit, whose qualified status is derived
  from immutable evidence references, never from a boolean field.
- **Typed Qualification Validator**: The contributor-reachable checker that
  resolves every qualification threshold from immutable evidence references and
  refuses omitted, forged, duplicate, inconsistent, or wrong-candidate ones.
- **Promotion Evidence Set**: The immutable coverage, failure, topology,
  package-policy, architecture, performance, cache, and equivalence evidence
  required before promotion.
- **Supply Chain Equivalence Result**: Exact current-Cargo versus decomposed
  Bazel status and normalized-finding equality for one context.
- **Post-Promotion Run Unit**: One API-derived immutable push-created
  `(run ID, head SHA)` pair carrying its complete `1..max` attempt history,
  provenance, normalized terminal conclusion, immutable creation ordering, and
  verified promotion ancestry. A unit contributes exactly one streak position;
  an attempt never does.
- **Bounded Post-Promotion Checkpoint**: A fixed-shape digest and final-ten
  suffix derived from the complete transient run stream; never an eligibility
  input or an append-only history.
- **Evidence Sink Result**: An underlying test verdict plus complete or
  degraded evidence status, bounded under the committed sink policy.

## Success Criteria

- **SC-001**: Exactly eighteen surface IDs have nonempty, total, unambiguous
  carrier coverage.
- **SC-002**: Ten consecutive protected-`v3` qualification records carry
  matching Cargo and Bazel verdicts and a passing same-commit fixture verdict.
- **SC-003**: Eighteen isolated plants fail exactly their owning surfaces.
- **SC-004**: Cargo and Bazel report equal test, ignored, doctest,
  harness-free, API, schema, scan, and pinned-inventory censuses.
- **SC-005**: Each broker context passes twenty consecutive executions under
  the required serialized topology.
- **SC-006**: Three warm local samples have median at most 10 minutes and
  maximum at most 12.
- **SC-007**: Three cold local samples have median at most 15 minutes and
  maximum at most 18.
- **SC-008**: The five most recent qualifying cold CI records have median at
  most 15 minutes and no record above 18.
- **SC-009**: Shadow creates zero shared Bazel cache entries and pull requests
  have zero write-capable cache paths.
- **SC-010**: Promotion publishes separate bounded action and download caches
  from one writer after synchronous trimming and two at-most-8-GiB headroom
  checks.
- **SC-011**: All four broker/guest system-and-target contexts pass exact
  closure, source census, checksum, deny, pinned no-fetch audit, and leakage
  checks.
- **SC-012**: All eight per-system package check wrappers plus both
  architecture-specific `guest-static-elf` checks exist in the pins and
  realize on their native runners.
- **SC-013**: All required planted guards, including retired-hub argv/cwd,
  source census, license policy, wrong-system, wrong-target, wrong-runner,
  foreign-system, remote-builder, and stale-output cases, fail as specified.
- **SC-014**: Promotion and retirement change zero required context names and
  remove zero public Rust Make entry points.
- **SC-015**: Before Cargo retirement, reverting the promotion commit restores
  Cargo authority without reconstructing deleted behavior.
- **SC-016**: All three broker suites retain `tags = ["exclusive"]`, fail the
  tag-removal mutation, and pass twenty consecutive executions per context
  without overlapping any test.
- **SC-017**: Loopback TCP, Unix-socket, external-egress, and live-index Bazel
  plants fail; every mandatory socket-using test passes through its exact
  same-commit non-advisory Cargo compatibility carrier; and the fetch inventory
  contains only pinned repository rules.
- **SC-018**: Both dedicated Nix derivations contain the exact
  `wl-proxy-0.1.2` hash, and missing, wrong, or one-sided pins fail.
- **SC-019**: Module refresh changes only `MODULE.bazel.lock` when stale,
  changes nothing on its second run, carries matching absolute startup
  options, and has zero Make/workflow reachability.
- **SC-020**: Provider route, descriptor inheritance, `ENOSYS`, blocking-wait,
  early-reap, shortened-grace, and conditional-SIGKILL mutations all fail.
- **SC-021**: Every qualifying record carries `bazelRestoreCount`,
  `bazelSaveCount`, and `bazelPublicationCount`; every selected cold record
  has `bazelRestoreCount` of zero and four `sliceDurationsSeconds` entries;
  cache
  retention keeps the newest generation and restore prefixes contain neither
  run ID nor commit SHA.
- **SC-022**: Native `test-flake-aarch64` passes six realizations and
  `make test-rust-supply-chain` on one renderer-covered stable head.
- **SC-023**: Every mutating validation command leaves the committed candidate
  clean under tracked, staged, and untracked-path assertions.
- **SC-024**: Main, broker, and guest yanked checks use the one product-lock
  snapshot with exact full-set or projection semantics; walker and
  `Cargo.guest.lock` never enter its key authority.
- **SC-025**: Every Bazel action remains no-network and no namespace or policy
  check claims per-endpoint enforcement. The compatibility census is complete,
  and missing, skipped, advisory, wrong-head, or misattributed compatibility
  evidence fails.
- **SC-026**: Provider tests accept escaping runfiles leaf symlinks without
  `RESOLVE_BENEATH`, reject reintroducing it, and preserve strict flags on
  result and cleanup paths.
- **SC-027**: Native guest artifacts are `ET_DYN` for the expected x86_64 or
  aarch64 `e_machine` and have no interpreter or `DT_NEEDED`; non-PIE and
  wrong-machine plants fail.
- **SC-028**: Manifest, JUnit, bounded sanitized `test.log`, emitted-evidence,
  exporter, no-shell, redaction, ignored-case, original-verdict,
  typed-degraded-status, and combined-budget mutations all fail their exact
  guards.
- **SC-029**: Every bound cache input changes each applicable primary key and
  restore prefix, and action and repository namespaces never collapse.
- **SC-030**: Cargo and decomposed Bazel supply-chain exit statuses and
  normalized finding unions are equal for all three contexts.
- **SC-031**: The release workflow, both retained fail-closed gate scripts, the
  three-way-join context oracle under its pinned traversal flags, the
  shared-dependency feature canary, and generic/dedicated context selectors
  pass.
- **SC-032**: Post-promotion run units are complete and derived; the last ten
  distinct ordered run units are successes with no intervening reset before
  retirement.
- **SC-033**: All six shadow Make targets are approved in spec003w1 and each
  resolves to a real Makefile rule; an unapproved shadow target call and an
  approved name without a Makefile rule both fail.
- **SC-034**: The typed qualification validator derives every threshold from
  complete paginated, attempt-aware run inventories and immutable content
  references; page-gap, missing-attempt, omitted-push, omitted, forged,
  duplicate, inconsistent, and wrong-candidate plants each fail, and no
  trusted boolean can qualify a record.
- **SC-035**: The generated no-shell governed and declared source sets are
  nonempty and equal; every spawn source is governed; every governed source
  has exactly one successful scan record including zero-site sources; and the
  fresh-scan/committed spawn-site keys are equal. Empty, missing-entry,
  extra-entry, ungoverned-spawn, missing-zero-site-record, and planted-shell
  cases each fail.
- **SC-036**: The pinned inventory lists under the one root lock with no lock
  backup or restore, the four native selected policy inputs drive the package
  census, deny, and pinned `--no-fetch` audit, guest static dependency policy
  reads only its production graph/lock, and the aggregate root and
  `Cargo.guest.lock` checks remain independent.
- **SC-037**: spec003w6 entry accepts only a containing published
  `v<major>.<minor>.<patch>` tag whose peeled local and origin commits match;
  a two-component tag, an unpushed tag, a divergent local/remote tag, and a
  draft or prerelease each fail entry.
- **SC-038**: The last ten distinct ordered run units succeed, while a
  repeated-attempt plant and an old-rerun-after-failure plant each fail to
  extend the streak.
- **SC-039**: Both dedicated Nix derivations realize on each native system;
  exact broker linkage, guest static linkage, selected closures, and measured
  size baselines pass, while linkage, closure, size-plus-one, static-broker,
  dynamic-guest, non-PIE, and wrong-machine mutations fail and qualification
  references the results.
- **SC-040**: The read-only plan-structure validator reports all task IDs
  unique, every dependency present and earlier, adjacency equal, the graph
  acyclic, and no ownership conflict among incomparable scopes.
- **SC-041**: `promotion-record.json` validates against the actual sealed
  `spec003w5` protected-`v3` merge; old-SHA, candidate-SHA, wrong-seal, and
  unsealed-merge plants fail before either post-promotion eligibility check.
- **SC-042**: Qualification rejects advisory classification for
  `test-flake-aarch64`, each of the four Rust slices, and `test-rust`; each
  advisory mutation fails.
- **SC-043**: Post-promotion eligibility is identical when derived from the
  complete transient stream and from its in-memory oracle, while persisted
  `post-promotion.json` remains within its fixed record and byte bounds.

## Assumptions and Dependencies

- ADR 0054 is merged and is the binding amendment to ADR 0052.
- The current committed tree at `a7093601` is the implementation base. Parked
  historical `spec003-w0-*` and `spec003-w0` branches and spike commits are
  read-only evidence.
- Existing passing code wins when it conflicts with older Spec 003 prose. The
  complete corrections are recorded in `plan.md`.
- The root flake system set remains exactly `x86_64-linux` and
  `aarch64-linux`.
- The existing eighteen surface IDs and two fixture-backed companion surfaces
  remain unchanged.
- Implementation may begin only after this amended artifact set receives
  unanimous Track A plan-panel signoff.
