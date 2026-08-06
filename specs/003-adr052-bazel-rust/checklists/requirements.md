# Specification Quality Checklist: ADR 0052 Under ADR 0054

**Purpose**: Validate the amended Track A artifact set before plan panel.

**Amended**: 2026-08-05

**Feature**: [spec.md](../spec.md)

## Authority and Scope

- [x] ADR 0052 and ADR 0054 are both named as binding authority.
- [x] Existing committed code is recorded as the pre-implementation canon.
- [x] Parked historical `spec003-w0-*` and `spec003-w0` branches are evidence
  only and are not treated as merged.
- [x] Track A classification is explicit and preserved.
- [x] Daemon, broker operation, VM, wire, and runtime behavior remain out of
  scope.
- [x] Narrow Nix derivation and dual-system policy changes required by ADR 0054
  are in scope without making Bazel a Nix builder.

## Workspace and Hub Model

- [x] One resolver-v2 product workspace covers main, broker, and guest.
- [x] `packages/Cargo.lock` is the only product Cargo lock authority.
- [x] The no-bash walker retains a separate workspace and lock.
- [x] `packages/Cargo.guest.lock` is static-guest closure input only.
- [x] Product and walker are the only accepted hubs.
- [x] Main, broker, and guest hub identifiers are retired rather than aliased.
- [x] Synthetic splice and forwarding lock assumptions are absent.
- [x] First-party product crates are native Bazel targets.
- [x] The product external repository is permitted to be a third-party union
  without becoming first-party edge authority.

## Cargo and Nix Selection

- [x] Broker default, layer1, and fake contexts use explicit root-workspace
  package and feature selection.
- [x] Guest production uses explicit package, default-feature, and
  real-libshpool selection.
- [x] Generic main Clippy and tests have the exact ADR 0054 exclusion sets.
- [x] Broker contexts retain serial execution and distinct target dirs.
- [x] All three Bazel broker suites require `tags = ["exclusive"]`, no overlap
  with any test, a tag-removal mutation, and twenty runs per context.
- [x] Broker and guest remain dedicated Nix derivations.
- [x] Root source and root lock selection is explicit for both derivations.
- [x] Both dedicated derivations retain the exact pinned
  `cargoLock.outputHashes."wl-proxy-0.1.2"` value and mutations.
- [x] Binary size, closure isolation, broker dynamic linkage, guest `ET_DYN`
  PIE, native `e_machine`, interpreter, and `NEEDED` checks remain enforcing,
  with broker and guest derivations realized on each native system, exactly
  four broker/guest-by-system baseline rows, exact broker
  interpreter/SONAME and recursive-closure count/digest baselines with no
  persisted store path, measured zero-delta size baselines, closed
  size-growth authorization positives/negatives, and
  linkage/closure/size/non-PIE/wrong-machine plants.
- [x] Libshpool is normal while code activation stays feature-gated.
- [x] Unsupported `crate.spec` use is forbidden.
- [x] The selected-context oracle is a three-way join: target-filtered locked
  offline root metadata supplies identities, sources, candidate edges, and
  `dep_kinds`; `packages/Cargo.lock` plus the committed git archive pin
  supplies every checksum; package-selected stable tree traversals supply the
  exact root, dependency-kind reach, and resolved features.
- [x] Tree parser input is pinned exactly with `--locked --offline`,
  `-p <package>`, `--target`, `--no-default-features`, explicit features,
  `--charset ascii`, `--prefix depth`, `--no-dedupe`, and a
  repository-pinned `--format` carrying package identity and feature columns.
- [x] Production and dev-inclusive edges are separate traversals, never one
  post-filtered into the other, and every traversal identity is cross-checked
  against metadata and the lock.
- [x] Metadata is recorded as supplying no checksums and a null workspace
  resolve root, and plain tree output is not assumed machine-readable.
- [x] The feature canary is an unrelated workspace member enabling an
  otherwise-absent feature on a dependency shared with broker or guest, and
  that feature stays absent from both selected traversals.
- [x] The spec003w0 Cargo gate reads the four native selected policy inputs
  with an exact source census, deny, and pinned `--no-fetch` audit, and no
  deleted nested lock path remains an input to the gate, the aggregate flake
  audit, or the guest static dependency policy.
- [x] The pinned test inventory lists with root-lock package selection and no
  nested-lock backup, restore function, scratch path, or `EXIT` trap, and the
  five stale comment files are owned by the same scope with tests first.

## Package Supply Chain

- [x] Broker GNU and guest musl contexts exist for x86_64 and aarch64.
- [x] Production and root-dev-inclusive policy graph shapes are specified.
- [x] Guest static dependency policy consumes only the production closure and
  production lock; deny and audit alone consume dev-inclusive policy inputs.
- [x] Exact root, nonempty census, edge-kind, cfg, feature, system, and target
  checks are specified.
- [x] Exact selected-source identity, count, readability, and checksum checks
  precede deny and audit.
- [x] Metadata and filtered-lock identity equality is required.
- [x] Package deny runs without `--exclude-dev`.
- [x] Package audit uses a pinned RustSec database and `--no-fetch`.
- [x] Broker and guest ignore sets are exact.
- [x] Aggregate root and generated guest closure checks remain independent on
  both the Cargo gate and the Nix side.
- [x] The six existing guest findings are named as a narrow implementation
  task.
- [x] Global guest license allowlist expansion is forbidden.
- [x] Different-package denial plants prove the exception remains narrow.
- [x] The yanked snapshot key set derives only from `packages/Cargo.lock`;
  main uses the full set and broker/guest use exact selected-graph
  projections.
- [x] The reviewed networked refresh and offline exact-key check remain
  separate, with live-index and key-set plants.
- [x] Every governed compile/build action and generated/custom test target
  names the repository seccomp wrapper as its executable, with no
  `--run_under` or pre-wrapper process; generated rule/toolchain and `aquery`
  inventories cover stable/nightly action kinds, inherited
  socket/ring/SQPOLL/fixed-socket preflight, full socket/io_uring denial, and
  no stage/unsandboxed fallback; Bazel setup before the Rust payload is outside
  the filter claim and namespaces are defense in depth only.
- [x] Mandatory socket-using tests remain on exact same-commit non-advisory
  Cargo compatibility carriers under their existing surface IDs.
- [x] All eight IPv4/IPv6/netlink/packet/pathname-Unix/abstract-Unix/
  socketpair/io_uring plants, external-egress/live-index results, provider
  inventory, and compatibility census enter qualification evidence.
- [x] Cargo current enforcing status and normalized findings are compared with
  the decomposed deny/audit/yanked union for main, broker, and guest.

## Dual Architecture

- [x] Exactly six native checks exist per system: four package checks, broker
  artifact contract, and guest ELF contract.
- [x] Native x86 and native arm realization is required.
- [x] `test-flake-aarch64` retains its ID and required rollup role.
- [x] `ubuntu-24.04-arm` and a 60-minute bound are specified.
- [x] The arm job also runs `make test-rust-supply-chain`, with renderer
  coverage and stable-head evidence.
- [x] Foreign-system, wrong-runner, and remote-builder refusals are distinct.
- [x] Aarch64 build evidence does not expand broker runtime support.
- [x] Native arm and x86 each realize exactly six checks, including the broker
  artifact contract, and `test-flake-aarch64` is non-advisory with a mutation.

## Repin and Mutation Safety

- [x] Exact retired-hub diagnostics match ADR 0054.
- [x] Exact product remediation argv and cwd are specified.
- [x] Tests use an injected non-mutating executor.
- [x] A duplicated packages path is rejected.
- [x] No test, workflow, or Make target runs a genuine repin.
- [x] Contributor repin and package generation remain shell-only operations
  after `nix develop` and `cd packages`.
- [x] Product and walker repins own only their matching Bazel-side locks.
- [x] Module refresh is test-first, no-argument, lock-only, idempotent, uses
  matching absolute startup options, has exact remediation, and is unreachable
  from Make and workflows.
- [x] Product-lock `cargo generate-lockfile --offline` is contributor-only and
  rejected from Make and workflows.
- [x] Lock-refresh authority is split by what changed: a product manifest
  change regenerates `packages/Cargo.lock`, then the product hub, then the
  module lock last, and proves the walker inputs byte-identical; a walker
  manifest or lock change regenerates the walker lock, then the walker hub,
  then the module lock last, and proves the product inputs byte-identical;
  initial or combined setup commits the product hub, then the walker hub, then
  the module lock last; only the two initial repins use command-local
  `--lockfile_mode=off`, create no module lock, and that mode refuses after
  bootstrap.
- [x] Every validation command sequence orders the walker hub before the module
  lock, and byte identity is proved by recorded hashes rather than a diff
  summary.

## Product Requirements

- [x] All six original user stories remain.
- [x] The eighteen execution-manifest IDs remain unchanged.
- [x] Exact coverage, topology, locator, per-case evidence, cache, deadline,
  performance, qualification, promotion, alias, and retirement requirements
  remain.
- [x] FR identifiers are sequential from FR-001 through FR-090.
- [x] Success criteria are sequential from SC-001 through SC-043.
- [x] Fixture-backed surfaces remain outside the eighteen-surface migration.
- [x] Public Make names and required context remain compatibility contracts.
- [x] Provider `RESOLVE_NO_MAGICLINKS`-only opens, deliberate absence of
  `RESOLVE_BENEATH`/`RESOLVE_NO_SYMLINKS`, permissive fallback leaf,
  strict result/cleanup flags, safe-by-value same-open-file-description
  `execveat(AT_EMPTY_PATH)`, `ENOSYS` refusal, broker-convention quarantined
  runner `sys.rs`, declared-stdio preservation, no helper/path/fd-0 transport,
  and behavioral CLOEXEC coverage are explicit.
- [x] Repeated non-consuming nonblocking grace observations, unconditional
  group kill, final reap, blocking-wait mutation, and early-reap mutation are
  explicit, with missing-process-group, wrapper-group, group-zero,
  group-minus-one, and PID-file-decoy plants preserving sibling/decoy life.
- [x] Exact cleanup/server recovery commands, per-code distinctions,
  redaction, and wrong-remedy mutations are explicit.
- [x] Qualification cache fields are canonically `bazelRestoreCount`,
  `bazelSaveCount`, `bazelPublicationCount`, and
  `sliceDurationsSeconds`; every record carries all three counts, every cold
  record carries four durations, and no snake_case spelling remains.
- [x] Run-unique keys, run/SHA-free restore prefixes, and newest-generation
  retention are explicit.
- [x] Manifest/JUnit prior invalidation, multi-carrier attribution, sorted
  atomic partial evidence, original-verdict preservation, ignored fidelity,
  full forbidden-value absence and bounds across JUnit, `test.log`, emitted
  evidence and exporter diagnostics, structurally closed tagged evidence with
  unchanged manifest v1, `junit-v1`, `test-log-v1`, `evidence-v1`, and
  `exporter-diagnostic-v1` age/count retention, no-shell enforcement, and
  combined budget mutations are explicit.
- [x] Every cache key input has an action/repository applicability row and a
  table-driven mutation of every applicable primary key and restore prefix.
- [x] Four authoritative promoted slice targets and exact mappings for all
  eight public leaves and five Bazel aliases are explicit.
- [x] Typed post-promotion run units derive eligibility from the complete
  paginated protected-`v3` run stream, where a unit is one distinct
  push-created (run ID, head SHA) pair, attempts `1..max` are that unit's
  complete nested history, the unit normalizes to its highest terminal attempt,
  and no further attempt increments the streak again.
- [x] Units are ordered by immutable creation order (`createdAt`, `runId`)
  and never by rerun start time, so an old rerun cannot move behind a newer
  failure; missing attempts and conflicting head or provenance are rejected;
  repeated-attempt and old-rerun-after-failure tests exist; and the final ten
  distinct ordered units must succeed.
- [x] A typed qualification validator in
  `packages/xtask/src/bazel_qualification.rs` with tests, implemented no
  later than spec003w3, derives every threshold from complete paginated,
  attempt-aware Cargo/Bazel/fixture inventories and immutable content
  references and rejects page gaps, missing attempts, omitted pushes, omitted,
  forged, duplicate, inconsistent, and wrong-candidate references; no trusted
  boolean can qualify a record.
- [x] The qualification task and the promotion validation both run the
  validator, and quickstart invokes it before any informational `jq`.
- [x] No-shell is bound to exact equal nonempty governed/declared sets, one
  successful scan result per governed source including zero-site sources, and
  an exact governed spawn-site subset, with raw and unique scan counts each
  equal governed-source count and exactly
  `no-shell-inventory-empty`, `no-shell-inventory-missing-entry`,
  `no-shell-inventory-extra-entry`,
  `no-shell-inventory-unguarded-spawn`,
  `no-shell-inventory-missing-zero-site-record`, and
  `no-shell-inventory-planted-shell`.
- [x] All six shadow Make targets enter `APPROVED_MAKE_TARGETS` in
  `packages/xtask/tests/policy_ci.rs` in the same wave, with positive and
  negative policy tests.
- [x] spec003w6 entry requires a containing published semantic release tag
  matching `v<major>.<minor>.<patch>`, proved by an anchored tag filter,
  ancestry, equal peeled local/origin tag commits, and a non-draft release,
  with both draft and prerelease false and a no-argument contributor-only
  validator with exact fixed-code command blocks.
- [x] VerifiedExecutable has compiler-derived closed public/hidden/inherent/
  explicit/auto/blanket API snapshots and focused rustdoc compile-fail
  construction, descriptor-access, trait, formatting/serialization,
  clone/copy/default, conversion, and minting seals, with no Cargo-shelling
  fixture.
- [x] Same-descriptor execution consumes `VerifiedExecutable` by value into
  one `unsafe_code = "deny"` runner `sys.rs` with narrow item allowances,
  parent-prepared async-signal-safe fork/dup/fcntl/error-pipe/execveat work,
  private CLOEXEC execution, preserved stdin/stdout/stderr, and API,
  same-open-file-description, stdin, close-on-exec, rebind-absence, and
  mutation coverage; no helper binary/runfile/path, direct invocation, fd-0
  transport, reopen, `/proc`, `fexecve`, path fallback, second unsafe file, or
  broad lint override remains.
- [x] Cache deletion uses a closed typed prefix enum and mixed pagination
  negatives preserve unauthorized entries.
- [x] The promotion record is typed and bound to the actual sealed merge before
  either post-promotion eligibility check.
- [x] Persisted post-promotion evidence is bounded while every decision derives
  from the complete transient protected stream; only complete state,
  page/stream counts, and digests persist, never a raw cursor.

## Plan and Task Quality

- [x] The wave graph is dependency-ordered and independently mergeable.
- [x] spec003w0 has a stable prep commit before parallel scopes.
- [x] spec003w0 prep creates and registers green runner and locator skeleton
  manifests/roots before their tests.
- [x] spec003w1, spec003w2, and spec003w5 prep own green crate-root and xtask contract seams;
  slices use test-local paths and the integrator wires completed modules only
  after the parallel frontier closes.
- [x] Every spec003w0 slice has disjoint file ownership and no slice edits a
  prep-owned file.
- [x] Shared generated output is integrator-owned.
- [x] Module/hub locks, Nix pins, BUILD files, and coverage/query goldens are
  integrator-generated only, and each refresh recipe follows the changed
  authority with the module lock committed last.
- [x] Every wave has commands and a mechanically checkable done condition.
- [x] Every task names an owned file set or exact evidence artifact.
- [x] Tests precede matching implementation tasks.
- [x] Native aarch64 required CI is a spec003w0 done condition.
- [x] spec003w0 owns Nix-unit pin regeneration, `make test-nix-unit`, and
  `tests/lib.sh` policy-binary wiring with run/exclusion regression coverage.
- [x] spec003w1 owns the no-bash walker implementation and fail-closed
  walk/read/parse census tests.
- [x] Schema, stub, inventory, and no-bash spec003w1 carriers are file-disjoint and
  carry their empty/mismatch/missing/extra/identity/state and no-shell
  relationship plants; all socket denial belongs only to the
  hermeticity/action-network carrier and never the stub carrier.
- [x] spec003w0 updates the release workflow and both existing fail-closed gate
  scripts rather than deleting either gate.
- [x] spec003w0 and spec003w5 include same-change binding-doc tasks without
  editing those docs in this amendment; spec003w0 includes all ten affected
  paths, corrects ADR 0054's stale proposed/four-hub summaries, and explicitly
  leaves ADR 0038 unchanged.
- [x] spec003w5 binding docs also own `tests/README.md` and
  `docs/reference/test-execution-manifest.md` because both describe the eight
  CI jobs.
- [x] `packages/xtask/tests/policy_ci.rs` has exactly one spec003w1 owner,
  its allowlist assertions are written test-first, and per-wave file ownership
  stays disjoint.
- [x] Every code-changing wave owns one semantic changelog fragment.
- [x] spec003w5 creates one atomic promotion candidate relative to its parent and
  rehearses reverting that exact commit, resolving the candidate from the
  verified current candidate HEAD and the recorded parent; `promotion-record.json`
  is read only after merge.
- [x] spec003w6 and spec003w7 eligibility clocks remain independent;
  spec003w7 qualification/code prep may run early, while its shared-file task
  and merge depend on merged spec003w6 and then rebase, revalidate, and
  re-panel.
- [x] Task IDs, inline dependencies, and the adjacency graph pass the
  read-only plan-structure validator, which first censuses every unchecked
  task-like checkbox, requires exact canonical headers, rejects dot/dot-dot,
  absolute, repeated-separator, malformed-quoting, duplicate/repeated metadata,
  post-graph tasks, and dynamic ownership, checks complete parsing, exact
  paths, acyclicity, and concurrent ownership conflicts, and ships one positive
  plus fifteen independent negative fixtures with fixed-code exact remedies
  and rerun.

## Documentation Hygiene

- [x] Every existing Spec 003 artifact is updated.
- [x] Every contract is internally aligned with ADR 0054.
- [x] Planning artifacts use qualified wave IDs: a scan of this artifact set
  finds exactly `spec003w0` through `spec003w7` plus `spec003w5fu1` process
  references, and only historical literal branch names remain otherwise.
- [x] Quickstart executable blocks use `set -euo pipefail`, check the complete
  absent path set, compare the exact lock inventory, and anchor lockfile grep.
- [x] Quickstart separates pull-request no-record/zero-cache-action inspection
  from protected-`v3` zero-count/four-duration inspection.
- [x] Mutating validation commands have clean-diff assertions before and after
  execution; quickstart does not use printed status as evidence.
- [x] The only non-Spec artifact modified by this amendment is the required
  semantic changelog fragment for the planning validator.
- [x] ASCII hyphens are used.
- [x] No unresolved clarification marker remains.
- [x] Provider, publication, qualification, release, artifact, and plan
  failures use fixed codes, repository-relative/digest-only identifiers, exact
  closed commands, and leak no `$!`, absolute/store path, raw cursor, or opaque
  handle.
- [x] Promotion and retirement docs and semantic changelog tasks disclose the
  exact permanently hybrid surfaces and retained Cargo socket cases until
  separately authorized.
- [x] An enforcing fixture-independent type-5 policy task derives the exact
  nonempty full compatibility-carrier census, retaining surface, selector,
  test identity, and socket class, and compares it bidirectionally with every
  governed hybrid doc and present semantic fragment, with missing and extra
  full-identity negatives.
- [x] Every seccomp stage, qualification query/refusal/publication failure,
  release query/refusal, and planning-validator failure has a closed fixed
  code, exact remedy/rerun, and leak-rejection coverage; query errors cannot
  become absence.
- [x] Size allowance derives only from its authorization object, sink kind and
  retention occur once, the authorization's prior/new bytes equal the baseline
  row and realized artifact, and complete/degraded evidence variants cannot
  contradict their common classification.
- [x] Required repository validation commands and artifact scans pass.
- [ ] Ten-role Track A plan panel returns unanimous signoff with empty
  recommendations.

## Readiness

The specification is ready to request the amended plan panel. Implementation
remains blocked until the final unchecked item passes.
