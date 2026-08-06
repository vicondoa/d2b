# Implementation Plan: ADR 0052 Under ADR 0054

**Track**: A

**Branch**: `spec003-adr0054-amend`

**Date**: 2026-08-05

**Spec**: [spec.md](./spec.md)

## Summary

Restart Spec 003 from merged `v3` after ADR 0054. First merge broker and guest
into one resolver-v2 product Cargo workspace and root lock, establish one
product and one walker Bazel hub, generate package-scoped broker GNU and guest
musl policy inputs for both root flake systems, and retain targeted Cargo and
dedicated Nix build contexts. Then continue ADR 0052's exact-coverage shadow,
safety, qualification, promotion, alias-removal, and Cargo-retirement
lifecycle.

No parked historical `spec003-w0-*` or `spec003-w0` commit is assumed merged.
Its source shape and the unified Bazel spike are read-only evidence. Every
implementation file is recreated or
adapted from current `v3`, committed in the new wave, validated, reviewed, and
sealed normally.

## Technical Context

**Product Cargo authority**:
`packages/{Cargo.toml,Cargo.lock}`, resolver version 2, including broker and
guest.

**Tool Cargo authority**:
`tests/tools/no-bash-ast-walker/{Cargo.toml,Cargo.lock}`.

**Generated static-guest input**:
`packages/Cargo.guest.lock`, not a workspace or hub.

**Bazel dependency hubs**:
`product` and `walker`.

**Configured product contexts**:
generic main, broker default, broker `layer1-bootstrap`, broker
`fake-backends`, guest `real-libshpool`.

**Package policy contexts**:

```text
x86_64-linux/x86_64-unknown-linux-gnu/broker-production
x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool
aarch64-linux/aarch64-unknown-linux-gnu/broker-production
aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool
```

**Rust tools**:
stable 1.97.0 and the existing pinned nightly API toolchain.

**Build tools**:
Bazel 8.6.0, reviewed `rules_rust` pin, pinned `cargo-bazel`.

**Execution surface**:
the existing eighteen manifest IDs and two fixture companion IDs.

**Platforms**:
root flake systems `x86_64-linux` and `aarch64-linux`. Native x86 and arm
realization is required. ADR 0008 runtime support is unchanged.

**Validation carriers**:
existing Rust, policy, drift, flake, fixture-contract, workflow-policy, and
Layer-1 surfaces. No new top-level shell gate or Layer-1 job.

## Constitution Check

| Principle | Result | Basis |
| --- | --- | --- |
| Daemon-only control plane | PASS | No service, unit, broker operation, or runtime path changes. |
| Broker-mediated privilege | PASS | Build and policy work is unprivileged. |
| Isolation | PASS | Selected package closures, dedicated Nix derivations, native first-party targets, and existing test topologies remain explicit. |
| Contract compatibility | PASS | Manifest v1, required context, Make names, and fixture split stay stable. |
| Test-layer discipline | PASS | Existing carriers own every new guard and architecture realization. |
| Panel-gated work | PASS | Track A plan and integrated-diff panels gate every wave. |
| Traceable artifacts | PASS | Qualified wave IDs remain in planning only; shipped artifacts use semantic text. |

## Spec Corrections

| Prior artifact claim | Canon or ADR 0054 authority retained | Plan treatment |
| --- | --- | --- |
| Product dependencies came from main, broker, and guest workspaces and locks. | One resolver-v2 product workspace and root lock. | Merge broker and guest in `spec003w0`; delete nested workspace tables and locks. |
| Four Bazel hubs existed. | Exactly product and walker hubs. | Retire main, broker, and guest identifiers with exact diagnostics. |
| The walker could be folded into main dependency resolution. | Walker is separate gate plumbing with its own lock. | Keep walker hub and Bazel-side lock byte-independent from product repin. |
| `Cargo.guest.lock` was a hub or workspace input. | It is generated static-guest closure input only. | Keep aggregate guest checks and cache binding; exclude it from hub authority. |
| Optional libshpool plus `crate.spec` modeled production. | Normal libshpool dependency, code feature gate retained, no `crate.spec`. | Change guest manifest in spec003w0 prep and add a no-`crate.spec` guard. |
| Whole-workspace broker and guest locks proved closure isolation. | Package-selected Cargo/Nix contexts and package-scoped closure policy prove isolation. | Generate system-and-target production and policy graphs from root lock. |
| Package policy could scan the shared lock union. | Exact selected roots, graphs, sources, and checksums are authoritative. | Require exact source census and filtered-lock equality before policy tools. |
| Guest license findings could be solved by broad license allow. | Six package/license pairs need a narrow reviewed update. | Add package-scoped exceptions and different-package denial plants. |
| Aarch64 flake evidence was eval-only on x86. | ADR 0054 requires native aarch64 package and ELF realization, and the broker artifact needs its own linkage/closure carrier. | Keep job ID, move to `ubuntu-24.04-arm`, 60 minutes, and realize six checks including `broker-host-artifact-contract`. |
| Parked historical foundation implementation was the base. | Branch head contains ADR 0054 only and no Bazel implementation. | Start spec003w0 from current merged `v3`; parked branches are evidence only. |
| A synthetic splice workspace was needed for selected policy or Bazel generation. | Locked root metadata plus configured native targets suffice. | No splice generator, splice lock, or synthetic manifest task exists. |
| Main Clippy included broker and guest after merge. | ADR 0054 requires generic main Clippy to exclude broker and guest while retaining contract Clippy. | Add explicit exclusions and dedicated package Clippy/test lanes. |
| Current standalone paths in `tests/test-rust.sh`, Nix, and CI remain valid after merge. | Committed code is the pre-merge baseline, not the target state. | Update them in spec003w0 while preserving selectors, target isolation, and job IDs. |
| The amendment summarized broker serialization without its Bazel mechanism. | ADR 0052 requires all three suites to carry `tags = ["exclusive"]`, overlap no test, and pass twenty executions per context. | Restore tag, mutation, scheduler, and qualification requirements in spec003w1 and spec003w4. |
| The amendment described offline tools without preserving the binding action no-network rule. | ADR 0052 prohibits every socket in a governed Rust payload. Linux network namespaces do not deny socket creation. | Bind the seccomp wrapper as each compile/build/test executable, use the full socket/io_uring set, inherited-capability preflight, eight in-action socket plants, external-egress/live-index plants, and no fallback; keep exact mandatory socket-using tests on same-commit non-advisory Cargo compatibility carriers until separately authorized. |
| Moving both Nix packages to the root lock made the git output hash look implicit. | ADR 0054 says both dedicated derivations retain the pinned git output hash. | Assert the exact `wl-proxy-0.1.2` key and value in both derivations. |
| Module lock drift named no complete repository path. | Measured Bazel remediation lacks worktree startup options. | Test then implement no-argument, lock-only, idempotent `bazel-module-refresh` with exact repository remediation. |
| Current amended recovery prose shortened command sequences. | ADR 0052 fixes exact per-code commands and forbidden cross-code remedies. | Restore literal commands and redaction/wrong-remedy mutations. |
| The post-promotion children were both concurrently ready while owning the same binding docs and evidence file. | Eligibility clocks are independent, but concurrently ready scopes must be file-disjoint. | Permit spec003w7 qualification and code preparation in parallel, then make its shared documentation/evidence task and merge depend on merged spec003w6, followed by revalidation and a new panel. |
| The amended cache record retained only a zero-write summary. | ADR 0052 requires explicit restore, save, publication, duration, key, prefix, and retention semantics. | Restore fields and their fixtures in spec003w3 through spec003w5. |
| Provider and expiry summaries omitted load-bearing kernel detail. | Same-descriptor `execveat`, close-on-exec behavior, and non-consuming grace observations are binding. | Restore exact flags, fallback semantics, mutations, and host conformance. |
| Provider behavior tests alone proved the verified-handle seal. | Runtime behavior cannot prove an API is absent, and execution must preserve the same verified open file description without exposing it. | Use the compiler-derived closed API/trait census plus focused rustdoc compile-fail examples, and make the safe runner layer consume the handle into the quarantined `sys.rs` fork/execveat boundary. |
| Provider opens reused strict result-path resolution. | Bazel runfiles leaf symlinks may leave the anchor. Provider opens therefore use `O_RDONLY|O_CLOEXEC` with `RESOLVE_NO_MAGICLINKS` only; forced walks use `O_NOFOLLOW` only on intermediates. Strict result and cleanup paths retain all three strict resolve flags. | Remove `RESOLVE_BENEATH` and `RESOLVE_NO_SYMLINKS` from provider opens, retain them on result/cleanup paths, and add a mutation rejecting provider `RESOLVE_BENEATH`. |
| Guest static ELF evidence accepted `ET_EXEC` and did not bind machine identity. | ADR 0054 keeps static PIE, and the native system decides the expected `e_machine`. | Require `ET_DYN`, expected native `e_machine`, no `PT_INTERP`, and no `DT_NEEDED`; add non-PIE and wrong-machine plants. |
| spec003w0 foundation tasks proposed failing future-behavior tests behind inert seams. | A merged wave must be green. | Implement spec003w0 behavior tested there; defer spec003w1/spec003w2 tests with their implementations. |
| Yanked authority still reflected the superseded three-lock model. | ADR 0054 creates one product lock plus selected policy projections. | Key one snapshot only from `packages/Cargo.lock`; project broker and guest from selected graphs. |
| Selected closures could be reconstructed from metadata alone or a synthetic splice. | Exact package context needs a three-way join; measured `cargo metadata` carries no `checksum` field and null workspace `resolve.root`, and plain `cargo tree` output is not machine-readable. | Join target-filtered locked offline root metadata (identities, sources, candidate edges, `cfg`), `packages/Cargo.lock` plus the committed git archive pin (checksums), and package-selected stable `cargo tree` traversals under pinned `--charset ascii --prefix depth --no-dedupe` and a repository-pinned delimited `--format` (root, dependency-kind reach, resolved features); forbid synthetic manifests and require a shared-dependency feature canary. |
| The initial amendment left the release workflow and two existing drift gates outside spec003w0. | The workspace merge changes their root-manifest, output, and cache assumptions. | spec003w0 updates `release-host-binaries.yml`, `flake-check-matrix-sync.sh`, and `ci-rust-cache-sync.sh`; neither gate is deleted. |
| Only three contributor docs were scheduled for the spec003w0 workspace correction. | `CONTRIBUTING.md`, workflow guidance, critical-subsystem wording, and `policy_modules.rs` also encode the sibling-workspace shape. | Add all four to the future spec003w0 binding-doc scope. Do not edit dated ADR 0038; record that ADR 0054 governs the newer shape. |
| ADR 0052 and its index/changelog summary still call accepted ADR 0054 proposed and retain the retired four-hub inventory. | ADR 0054 is accepted settled context. | Correct the ADR 0052 amendment-status paragraph, ADR index summary, and ADR 0054 changelog fragment with the spec003w0 documentation correction; leave ADR 0038 historical text unchanged. |
| Slice ownership included module locks, hub locks, Nix pins, generated BUILD files, and coverage/query goldens. | Those artifacts are generator results shared by all slices. | Integrator alone commits them. Slices write scratch previews. Lock refresh follows the changed authority and always refreshes `MODULE.bazel.lock` last, then clean no-op checks. |
| One lock-refresh order was described for every change, and two validation blocks refreshed the module lock before the walker hub. | The product and walker hubs are independent authorities and the module lock consumes both. | Split into three authorities: product manifest change (`packages/Cargo.lock`, product hub, module lock last, walker inputs proved byte-identical); walker manifest or lock change (walker Cargo lock, walker hub, module lock last, product inputs proved byte-identical); initial or combined setup (product hub, walker hub, module lock last). Reorder every validation block accordingly. |
| The spec003w0 Cargo gate could keep auditing the nested broker and guest locks. | Those locks are deleted by the same wave, so the gate has no nested authority left. | Move package deny/audit onto the native-system broker-GNU and guest-musl selected policy inputs with exact source census and pinned `--no-fetch` audit, keep the aggregate root and `Cargo.guest.lock` checks independent, and replace `tests/tools/assert-pinned-tests.sh` nested-lock backup/restore with root-lock package selection. |
| Six shadow Make targets could be introduced without touching the workflow allowlist. | `APPROVED_MAKE_TARGETS` in `packages/xtask/tests/policy_ci.rs` is the only allowlist, and an unlisted target silently escapes the ci-uses-make guard. | The same wave that introduces the shadow targets adds all six to the allowlist with positive and negative tests owned by one exact spec003w1 slice. |
| Qualification could be read from boolean verdict fields. | A boolean is self-asserted; only immutable references are evidence. | Add a typed validator that derives every threshold from immutable evidence references and refuses omitted, forged, duplicate, inconsistent, and wrong-candidate references. |
| No-shell was asserted over an implied file set. | An implied set cannot detect an unscanned new spawn site. | Bind no-shell to a generated, drift-checked, nonempty inventory compared bidirectionally against governed sources and declared inputs, with empty, missing, extra, and planted-shell negatives. |
| spec003w6 entry accepted any containing tag. | Repository tags include two-component names such as `v1.0`, `v1.1`, and `v1.2`, so containment alone is not a release. | Require a containing published tag matching `v<major>.<minor>.<patch>` exactly, resolving on origin with a non-draft release. |
| The pre-merge rollback rehearsal read `promotion-record.json`. | That record is written only after merge. | Rehearse from the verified current atomic candidate HEAD and the recorded spec003w5 parent; keep promotion-record reads post-merge. |
| spec003w5 binding docs covered only the three contributor docs. | `tests/README.md` and `docs/reference/test-execution-manifest.md` also describe the eight CI jobs. | Add both to the spec003w5 binding-doc scope. |
| Qualification cache fields were written in mixed snake_case and ad hoc names. | One canonical camelCase field set must appear in every artifact. | Fix `bazelRestoreCount`, `bazelSaveCount`, `bazelPublicationCount`, and `sliceDurationsSeconds` as the only spellings. |
| Post-promotion streak positions were counted per observation, allowing attempts to advance the streak. | An attempt is a rerun of one unit, and rerun start time is mutable. | Count distinct push-created (run ID, head SHA) units, normalize to the highest terminal attempt, and order by immutable `createdAt` then run ID. |
| Promotion reused old leaf names as CI slice targets. | Promotion needs four authoritative CI slice targets while preserving all eight public leaves. | Introduce `test-rust-slice-{main,api,broker,aux}` for CI and map every old leaf to an exact Bazel subset. Compatibility aliases forward only to the aggregate or matching slice target. |
| Post-promotion eligibility trusted count and ID fields in the evidence file. | Eligibility must be derived from the complete protected-`v3` run stream. | Inventory typed run units with pagination, provenance, terminality, ordering, and promotion-ancestry checks; derive resets and the current streak. |
| Raw child output remained in `test.log`, and publication failure rewrote a passing test as failed. | Every persistent or emitted sink must be sanitized and bounded; exporter state is not the test verdict. | Sanitize and bound JUnit, `test.log`, execution evidence, qualification evidence, and exporter diagnostics. Preserve `testVerdict`, emit typed degraded evidence, and make surface completion and qualification reject degradation separately. |
| Cache deletion authority was an undefined authorized-prefix predicate. | Destructive maintenance requires a closed committed authority. | Use a typed prefix enum, preserve unknown entries, and test mixed authorized/unauthorized pagination with zero delete calls on refusal. |
| No-shell required every governed source to contain a spawn site. | A governed source can validly contain zero sites. | Make governed and declared sets equal, make spawn sources a subset, require one scan result per governed source including zero-site records, and compare fresh and committed spawn keys exactly. |
| Artifact requirements named size and closure checks without realizations, baselines, or exact broker linkage. | ADR 0054 preserves dedicated derivations and their security checks. | Realize four broker/guest-by-system artifacts, commit exactly four measured zero-delta baseline rows with transient closure validation, mutation-test them, and bind them into qualification. |
| A promotion record could name any containing commit. | Post-promotion eligibility must bind the actual sealed merge. | Add a typed validator tying the record to the protected-`v3` PR merge and the exact `spec003w5` seal before either child enters. |
| Network namespaces were described as action-wide no-socket enforcement. | A namespace does not deny socket creation, socketpair, or io_uring networking, and Bazel setup precedes the Rust payload. | Make the wrapper the executable field of every governed compile/build action and the generated/custom test executable itself; bind it through generated rules/toolchains and `aquery`, reject pre-wrapper and `--run_under` shapes, preflight sockets and every ring including SQPOLL/fixed-socket state, deny the full syscall set, and claim only wrapper payload descendants. |
| The runner planned a repository-authored post-fork raw-syscall child under a workspace that forbids unsafe code, then replaced it with a path-launched helper using fd 0. | Existing broker canon keeps a runner crate at `unsafe_code = "deny"` and quarantines required low-level calls in one reviewed `sys.rs`; fd 0 is target stdin and a helper path can be rebound. | Remove the helper binary and runfile entirely. The safe layer consumes `VerifiedExecutable` by value; parent-prepared state enters one item-allowlisted `sys.rs` boundary for fork, dup/fcntl, error-pipe, and same-open-file-description `execveat`, preserving declared stdio and leaking no provider fd. |
| Ad hoc Cargo-shelling compile fixtures were the VerifiedExecutable API proof. | `tests/AGENTS.md` makes the compiler-derived API census primary and reserves rustdoc compile-fail for downstream type properties. | Make VerifiedExecutable a capability root with empty public-inherent and locally-authored explicit-trait allowlists, pin compiler auto/blanket impls, and use focused rustdoc compile-fail examples only. |
| The plan validator truncated aggregate ownership prose after `and every`. | A malformed checkbox or aliased path can hide a concurrent write. | Census every unchecked task-like checkbox before parsing; require the exact `- [ ] TNNN` header; reject dot/dot-dot components, absolute paths, repeated separators, malformed quoting, unresolved expressions, and duplicates; add independent header, path, parser, dependency, adjacency, cycle, conflict, and dynamic fixtures with fixed-code exact remedies. |
| Native inventory prose alternated between five checks, two artifact rows, and six checks. | There are exactly six native checks per system and exactly four artifact baseline rows. | Normalize every inventory, task, evidence threshold, quickstart check, and checklist item to those cardinalities. |
| Size growth relied on prose review without a typed authorization. | A changed baseline must bind its exact candidate and review. | Add closed positive/negative size-growth authorization fixtures and require all four row digests plus every nonzero authorization digest in qualification. |
| Artifact baselines persisted exact Nix closure paths and post-promotion checkpoints persisted a cursor. | Exact store paths and pagination tokens are transient validation data. | Persist only closed states, counts, and SHA-256 digests; make fixed-code diagnostics repository-relative and digest-only. |
| Sink policy named a retention class but defined no closed limits or expiry. | JUnit, `test.log`, unsealed evidence, and exporter diagnostics need enforced age and count limits. | Add `junit-v1`, `test-log-v1`, `evidence-v1`, and `exporter-diagnostic-v1` with injected age/count/expiry tests before publication. |
| No-shell prose sometimes listed four plants and checked only a unique scan projection. | The exact six plants and both raw and unique scan counts are binding. | List all six everywhere and require both counts to equal governed-source count. |
| Socket plants leaked into stub-carrier acceptance language. | Socket denial belongs only to hermeticity/action-network. | Remove forbidden-listener/socket plants from the stub carrier and enforce carrier ownership. |
| Evidence status was a string with optional fields. | Complete and degraded states must be structurally closed without changing manifest v1. | Put `sinkKind` and `retentionClass` once in the common record and use a tagged sidecar union with disjoint fields, closed codes/commands, and schema-valid unchanged manifest-v1 output. |
| Provider, publication, qualification, and release refusal prose used generic rerun placeholders. | Operator refusal UX must render exact commands without leaking identifiers. | Add closed reason-by-slice and qualification/release command tables with fixed-code, repository-relative, digest-only diagnostics. |
| Promotion docs named aliases but not retained Cargo coverage. | Socket-using compatibility cases survive Cargo retirement. | Require promotion and retirement docs and semantic changelog fragments to list exact hybrid surfaces and state separate authorization is required for retirement. |
| Hybrid disclosure depended on prose review. | A retained Cargo case can disappear from one binding doc while execution remains hybrid, and a surface-only set collapses distinct cases. | Add an enforcing type-5 policy lint that derives the exact nonempty full carrier census from the coverage map, retaining surface, selector, test identity, and socket class, and compares it bidirectionally with every governed hybrid doc and present semantic fragment, with independent missing and extra full-identity fixtures. |
| Release queries suppressed `git` and `gh` errors while trying the next tag. | An unavailable query backend is degraded evidence, not proof that a release object is absent. | Use closed typed complete/degraded/refused outcomes, distinct query codes, no persisted or printed candidate/tag identifier, no raw output, and exact remedies. |
| Artifact rows carried a size allowance beside the authorization, and evidence variants repeated sink classification. | Duplicated authorities admit contradictory records, and unrelated old/new arithmetic can authorize a real artifact. | Derive positive size delta only from `sizeGrowthAuthorization`, require its prior/new bytes to equal the baseline row and realized artifact, keep sink kind and retention once in the common evidence record, and reject duplicate authority fields. |

## Shared Design Invariants

1. Product dependency authority is one root manifest and lock.
2. Walker dependency authority remains separate.
3. `Cargo.guest.lock` is never a Cargo or hub authority.
4. Broker and guest contexts always name package, default-feature state,
   features, target, and target directory.
5. Nix keeps dedicated broker and guest derivations.
6. Product first-party crates are native Bazel targets.
7. The product external repository may be a third-party superset.
8. Every absence or leakage predicate follows a root, nonempty closure, and
   exact census assertion.
9. Selected-source identities and checksums are exact and verified before deny
   or audit.
10. Package audit is pinned and no-fetch.
11. Native x86 and arm lanes realize the same six system-specific checks
    without foreign-system or remote-builder arguments.
12. Contributor mutations are unavailable from Make and workflows.
13. All existing ADR 0052 coverage, runner, cache, deadline, and promotion
    invariants remain.
14. The three broker suites are exclusive against every other test and each
    qualifies with twenty consecutive executions.
15. Every governed Bazel Rust compile/build action names the seccomp wrapper
    as its executable, and every generated/custom test names the wrapper as
    the test executable with the real binary as a declared input. Generated
    rule/toolchain plus `aquery` inventories cover stable/nightly action kinds,
    reject `--run_under` and pre-wrapper processes, and preflight inherited
    sockets and all io_uring rings including SQPOLL/fixed-socket state before
    full syscall denial with no fallback. Bazel setup before the payload is
    outside the filter claim. Mandatory socket-using tests remain exact
    same-commit non-advisory Cargo compatibility carriers; only pinned
    repository rules fetch.
16. Both dedicated Nix derivations carry the exact committed `wl-proxy` output
    hash.
17. Provider execution opens with `O_RDONLY|O_CLOEXEC` and
    `RESOLVE_NO_MAGICLINKS` only, uses no `RESOLVE_BENEATH` or
    `RESOLVE_NO_SYMLINKS`, executes the same verified open file description
    through a private CLOEXEC descriptor with `execveat(AT_EMPTY_PATH)`, and
    has no path fallback. Forced walk applies
    `O_NOFOLLOW` only to intermediates. Strict result and cleanup paths keep
    all three resolve flags. Expiry observes without consuming until
    unconditional group kill and direct-child reap.
18. Every mutating validation command leaves the committed candidate clean.
19. Native guest ELF evidence requires expected `e_machine`, `ET_DYN`, no
    `PT_INTERP`, and no `DT_NEEDED`.
20. Generated locks, pins, BUILD files, and coverage/query goldens are
    integrator-owned.
21. Cargo/decomposed-Bazel supply-chain exit status and normalized findings
    must match for main, broker, and guest.
22. Lock refresh follows the authority that changed, always commits
    `MODULE.bazel.lock` last, and proves the untouched hub's inputs
    byte-identical.
23. The selected-context oracle is a three-way join: metadata supplies
    identities, sources, candidate edges, and `cfg`; the product lock and
    committed git pin supply checksums; pinned package-selected `cargo tree`
    traversals supply root, dependency-kind reach, and resolved features.
24. Every qualification threshold is derived from immutable evidence
    references; no boolean field can qualify a record.
25. No-shell is bound to a generated, drift-checked, nonempty inventory whose
    governed and declared source sets agree, whose scan records cover every
    governed source including zero-site sources, whose raw and unique record
    counts each equal governed-source count, and whose spawn sites are an exact
    governed subset.
26. A shadow Make target is approved in `APPROVED_MAKE_TARGETS` by the same
    wave that introduces it.
27. A post-promotion streak position is one distinct push-created
    (run ID, head SHA) unit ordered by immutable creation order; an attempt
    never adds a position.
28. `VerifiedExecutable` has compiler-derived closed public/trait allowlists,
    and the safe execution API consumes it into one `unsafe_code = "deny"`
    runner `sys.rs` boundary that preserves declared stdio and performs
    same-open-file-description `execveat` with no helper or path fallback.
29. Every evidence sink is sanitized and bounded before writing. Exporter
    degradation preserves the underlying test verdict and blocks qualification
    separately.
30. Cache deletion authority is a closed typed prefix set; caller data cannot
    widen it.
31. Four broker/guest-by-system Nix artifacts realize with exact linkage,
    closure count/digest, and measured size-baseline evidence.
32. The promotion record must validate against the actual sealed
    protected-`v3` merge before either post-promotion child enters.
33. Native artifact authority is exactly four rows with no persisted store
    path and with closed authorization for every nonzero size delta.
34. Evidence status is a closed sidecar union, manifest v1 is unchanged, and
    `junit-v1`, `test-log-v1`, `evidence-v1`, and
    `exporter-diagnostic-v1` enforce age and count before publication.
35. Size allowance exists only in a bound positive-delta authorization;
    evidence sink kind and retention occur once; complete/degraded records
    cannot contradict their common classification.
36. Every seccomp stage, qualification failure, release query, and planning
    validator failure has a closed fixed code, exact repository-relative
    remedy, exact rerun, and leak-rejection coverage.
37. The exact nonempty Cargo compatibility census is enforced against every
    governed hybrid document and semantic migration fragment by a type-5
    policy lint with missing and extra negatives.

## Expected Implementation Locations

```text
.bazelversion
.bazelrc
.bazelignore
MODULE.bazel
MODULE.bazel.lock
BUILD.bazel
bazel/
  cargo/{product.lock,walker.lock}
  carriers/
  generated/{BUILD.bazel,action-network-policy.json,configured-targets.json,evidence-sink-policy.json,no-shell-inventory.json,output-manifest.json,package-policy-targets.bzl,product-targets.bzl,source-census.json}
  patches/rules_rust-seccomp-wrapper.patch
  rules/
  supply_chain/yanked-snapshot.json
  vendor/
ci/rust/BUILD.bazel
packages/
  Cargo.toml
  Cargo.lock
  Cargo.guest.lock
  policy-inputs/
  d2b-bazel-support/
  d2b-bazel-runner/
    src/sys.rs
  d2b-bazel-seccomp-exec/
  d2b-test-locator/
  d2b-contract-tests/tests/policy_bazel_hybrid_docs.rs
  d2b-priv-broker/Cargo.toml
  d2b-guest-shell-runner/{Cargo.toml,deny.toml}
  xtask/src/{bazel,bazel_evidence,bazel_qualification,bazel_yanked,hermeticity,package_policy,release_containment,schema}.rs
  xtask/tests/policy_ci.rs
bazel/generated/no-shell-inventory.json
tests/
  README.md
  lib.sh
  golden/bazel-rust-coverage.json
  golden/bazel-rust-artifact-baselines.json
  golden/bazel-rust-query.json
  golden/flake-check-matrix/
  golden/pinned/{kernel-canaries,usbip-firewall-skeleton,host-prepare-network,broker-socket-acl,broker-export-audit}.txt
  layer1-jobs.json
  test-rust.sh
  tools/assert-pinned-tests.sh
  tools/no-bash-ast-walker/src/main.rs
  tools/flake-check-classes.sh
  unit/nix/pinned/{common,x86_64-linux,aarch64-linux}.txt
.github/workflows/pr-bazel-rust.yml
.github/workflows/pr-l1-static-fast.yml
nixos-modules/host-broker.nix
flake.nix
AGENTS.md
tests/AGENTS.md
docs/contributing/gates-and-lints.md
docs/reference/test-execution-manifest.md
specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
specs/003-adr052-bazel-rust/tools/validator-fixtures/
```

The generated output set above is closed and exact. Generated outputs are
integrator-owned. A parallel slice may generate a scratch preview but never
commits a shared generated output. No task may replace this list with a
manifest expansion or other dynamic ownership expression.

## Wave Graph

```text
ADR 0054 and amended Spec 003 plan panel
  -> spec003w0 product foundation
  -> spec003w1 complete Bazel carriers
  -> spec003w2 operational safety
  -> spec003w3 shadow CI
  -> spec003w4 immutable qualification
  -> spec003w5 promotion and promotion record
       -> spec003w6 alias removal, release-containment gate
       -> spec003w7 Cargo implementation retirement, ten-green-run gate
```

Every wave is independently reviewable, mergeable, and sealed before a child
wave begins. After promotion, release-containment and green-run qualification
may proceed independently. spec003w7 code preparation may proceed before
spec003w6, but its shared documentation/evidence task and merge wait for merged
spec003w6, then rebase, revalidate, and re-panel. No concurrently ready scopes
own the same file.

## Global Delivery Rules

- Run a unanimous ten-role Track A plan panel before each implementation wave.
- Reviewers inspect supplied validation and do not rerun gates.
- Land an integrator prep commit before parallel scopes where shared contracts
  are needed.
- Prep contracts are complete and green before dispatch. No parallel scope
  edits a prep-owned file. After all dependent scopes merge, the integrator
  may edit a prep-owned crate root or command router only to wire the completed
  scope-owned modules. A missing contract or dependency requires a second prep
  commit and all affected scopes restart from it.
- Each scope writes only its ownership list.
- Each scope commits before integration validation.
- The integrator alone updates shared manifests, generated outputs, and
  generated workflows after merging scope commits.
- Any content change invalidates the integrated-diff panel.
- Required CI and native architecture jobs must pass on the stable PR head
  before seal and merge.
- Every generator, repin, refresh, and pin-regeneration validation is enclosed
  by tracked, staged, and untracked clean-diff assertions.
- Future binding docs and changelog fragments use semantic language and contain
  no delivery process markers.
- Run `nix-collect-garbage` after every wave merge.
- Before every plan panel, run
  `perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
  --self-test`, then
  `perl specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl`.
  The read-only command checks task-ID uniqueness, dependency existence and
  textual order, exact adjacency-list equality, acyclicity, literal exact file
  ownership, complete checkbox census and task parsing, and conflicts between
  tasks that are incomparable in the dependency graph and therefore
  concurrently ready. It requires the exact `- [ ] TNNN` header and rejects
  dot/dot-dot components, absolute paths, repeated separators, malformed
  quoting/backticks, unresolved expressions, duplicate paths or dependencies,
  and repeated metadata fields. Its positive and fifteen independent negative
  fixtures must pass. Every failure prints a
  fixed code, repository-relative source, exact remedy, and the exact
  self-test-plus-plan rerun; it emits no raw OS text or absolute path. A
  nonzero result blocks dispatch. It remains a planning tool under this
  directory and is not added to a repository gate.

## spec003w0 - Product Workspace and Reversible Foundation

### Deliverable

One product workspace and lock, separate walker workspace and lock, product and
walker hubs, generator-owned native first-party graph, package-scoped policy
inputs and checks for both systems, root-lock Nix derivations, exact source
census and checksum policy, retired-hub refusals, and the reusable support,
runner, locator, schema, and hermeticity foundation. Cargo remains the
authoritative Rust executor.

### Entry condition

The branch base is a descendant of ADR 0054 commit `a7093601`, and these paths
are absent from the base:

```text
.bazelversion
.bazelrc
.bazelignore
MODULE.bazel
MODULE.bazel.lock
bazel/
BUILD.bazel
```

The entry check also proves no parked historical `spec003-w0-*` or
`spec003-w0` commit is an ancestor requirement.

### Integrator prep

The prep commit lands all shared contracts before slices open:

```text
packages/Cargo.toml
packages/Cargo.lock
packages/d2b-priv-broker/Cargo.toml
packages/d2b-priv-broker/Cargo.lock              # delete
packages/d2b-guest-shell-runner/Cargo.toml
packages/d2b-guest-shell-runner/Cargo.lock       # delete
packages/d2b-contract-tests/Cargo.toml
packages/xtask/tests/policy_workspace.rs
packages/d2b-bazel-support/Cargo.toml
packages/d2b-bazel-support/src/lib.rs
packages/d2b-bazel-support/src/fsops.rs
packages/d2b-bazel-support/src/runfiles.rs
packages/d2b-bazel-support/src/startup.rs
packages/d2b-bazel-support/src/verified_executable.rs
packages/d2b-bazel-support/tests/provider_handle.rs
packages/d2b-bazel-support/tests/verified_executable_api.rs
packages/d2b-bazel-support/tests/startup.rs
packages/d2b-bazel-seccomp-exec/Cargo.toml
packages/d2b-bazel-seccomp-exec/src/main.rs
packages/d2b-bazel-seccomp-exec/src/filter.rs
packages/d2b-bazel-seccomp-exec/tests/filter.rs
packages/d2b-bazel-runner/Cargo.toml
packages/d2b-bazel-runner/src/lib.rs
packages/d2b-bazel-runner/src/sys.rs
packages/d2b-test-locator/Cargo.toml
packages/d2b-test-locator/src/lib.rs
packages/xtask/Cargo.toml
packages/xtask/src/main.rs
```

The prep commit:

- merges broker and guest into the root workspace;
- removes nested workspace and profile tables;
- makes libshpool normal and `real-libshpool` empty;
- regenerates the root lock offline;
- creates and registers the complete neutral support crate, including the
  filesystem, runfiles, startup interfaces, fakes, and provider behavior that
  runner, locator, xtask, and policy slices read;
- adds `VerifiedExecutable` as a compiler-derived capability root with an
  empty inherent API and empty locally-authored explicit-trait allowlist, exact
  auto/blanket snapshot, and focused rustdoc compile-fail examples; no
  Cargo-shelling compile fixture is created;
- creates the `unsafe_code = "forbid"` seccomp wrapper and the runner's one
  `unsafe_code = "deny"` quarantined `sys.rs` boundary, matching the reviewed
  broker convention. The wrapper uses the pinned safe libseccomp API, while
  its external Rust/C FFI dependencies receive a separate package-policy and
  source-hash review. The public safe runner layer consumes
  `VerifiedExecutable`; parent-prepared state enters only item-level
  fork/dup/fcntl/error-pipe/execveat functions, preserves declared stdio, and
  accepts no helper or target path;
- creates and registers green runner and locator crate skeletons before any
  runner or locator test, with their complete future dependency sets and
  stable crate-root contract seams. Scope tests load their not-yet-wired
  implementation files through test-local paths, and the integrator wires the
  completed modules only after parallel work ends;
- owns the complete spec003w0 xtask dependency set and a green command root without declaring
  not-yet-present generator modules. The integrator wires those modules after
  the generator scope merges;
- regenerates and commits `packages/Cargo.lock` after the manifest changes;
- implements every behavior its tests assert. Coverage, topology, manifest,
  deadline, process, cleanup, and recovery tests remain deferred to
  spec003w1 or spec003w2 with their implementations rather than landing red
  behind inert seams.

No parallel slice edits a prep-owned file. If a shared seam is incomplete, an
additional prep commit lands before scopes resume.

### Parallel slice ownership

| Slice | Owned files |
| --- | --- |
| `spec003w0-cargo-gates` | `tests/test-rust.sh`, `tests/tools/assert-pinned-tests.sh`, `tests/golden/pinned/kernel-canaries.txt`, `tests/golden/pinned/usbip-firewall-skeleton.txt`, `tests/golden/pinned/host-prepare-network.txt`, `tests/golden/pinned/broker-socket-acl.txt`, `tests/golden/pinned/broker-export-audit.txt` |
| `spec003w0-bazel-generator` | `.bazelversion`, `.bazelrc`, `MODULE.bazel`, `BUILD.bazel`, `bazel/BUILD.bazel`, `bazel/defs.bzl`, `bazel/toolchains.bzl`, `bazel/rules/seccomp_action.bzl`, `bazel/patches/rules_rust-seccomp-wrapper.patch`, `bazel/cargo/README.md`, `bazel/cargo/BUILD.bazel`, `bazel/cargo/cargo_bazel.bzl`, `packages/xtask/src/bazel.rs`, `packages/xtask/src/package_policy.rs`, `packages/xtask/src/bazel_yanked.rs`, `packages/xtask/src/schema.rs`, `packages/xtask/src/hermeticity.rs`, `packages/xtask/tests/bazel_foundation.rs`, `packages/xtask/tests/bazel_module_refresh.rs`, `packages/xtask/tests/package_policy_refusals.rs`, `packages/xtask/tests/bazel_action_network.rs` |
| `spec003w0-runner-foundation` | `packages/d2b-bazel-runner/src/exec_handle.rs`, `packages/d2b-bazel-runner/src/bin/d2b-exec-probe.rs`, `packages/d2b-bazel-runner/tests/exec_handle.rs` |
| `spec003w0-locator-foundation` | `packages/d2b-test-locator/src/mode.rs`, `packages/d2b-test-locator/tests/mode_selection.rs` |
| `spec003w0-nix-policy` | `nixos-modules/host-broker.nix`, `flake.nix`, `tests/unit/nix/cases/bazel-package-policy.nix`, `packages/d2b-contract-tests/tests/policy_bazel_nix.rs`, `packages/d2b-contract-tests/tests/policy_bazel_supply_chain.rs`, `packages/d2b-guest-shell-runner/deny.toml` |
| `spec003w0-policy-ci` | `tests/lib.sh`, `packages/xtask/tests/policy_ci.rs`, `packages/d2b-contract-tests/tests/policy_docs.rs`, `tests/unit/meta/w0-dep-direction.sh`, `tests/unit/meta/ci-runner-regression.py`, `tests/unit/gates/flake-check-matrix-sync.sh`, `tests/unit/gates/ci-rust-cache-sync.sh`, `tests/layer1-jobs.json`, `tests/tools/layer1-jobs.py`, `tests/ci/layer1-workflow.template.yml`, `tests/tools/flake-check-classes.sh`, `tests/tools/gen-flake-check-matrix-pin.sh`, `.github/workflows/release-host-binaries.yml` |
| `spec003w0-binding-docs` | `AGENTS.md`, `tests/AGENTS.md`, `CONTRIBUTING.md`, `docs/contributing/gates-and-lints.md`, `docs/contributing/workflow.md`, `docs/contributing/critical-subsystems.md`, `docs/adr/0052-bazel-rust-build-and-test.md`, `docs/adr/README.md`, `changelog.d/adr0054-broker-hub.md`, `packages/d2b-contract-tests/tests/policy_modules.rs` |

Only the Bazel generator slice opens from the first prep tip. After it
integrates, the integrator wires xtask, generates the product and walker hub
locks, and refreshes the module lock. The remaining independent spec003w0
scopes open from that green generator-checkpoint tip, so no Cargo process
observes routing or lock mutation in flight.
`spec003w0-nix-policy` begins only after the generator's policy schema is
integrated. `spec003w0-policy-ci` begins only after the new fixture-independent policy
binaries exist. `spec003w0-binding-docs` begins
after the integrated command and gate shapes are stable.
The release-workflow substep of `spec003w0-policy-ci` begins after the root
workspace and gate target directories are stable. These are real dependencies,
not parallel work. The binding-doc scope records that ADR 0054 governs the
newer workspace shape and does not edit dated ADR 0038.

### Integrator-owned reconciliation

After slice commits merge, only the integrator updates:

```text
Makefile
packages/Cargo.toml
packages/Cargo.lock
.bazelignore
MODULE.bazel.lock
bazel/generated/BUILD.bazel
bazel/generated/action-network-policy.json
bazel/generated/configured-targets.json
bazel/generated/evidence-sink-policy.json
bazel/generated/no-shell-inventory.json
bazel/generated/output-manifest.json
bazel/generated/package-policy-targets.bzl
bazel/generated/product-targets.bzl
bazel/generated/source-census.json
bazel/cargo/product.lock
bazel/cargo/walker.lock
the sixteen exact package-policy files enumerated in tasks T026
tests/unit/nix/pinned/common.txt
tests/unit/nix/pinned/x86_64-linux.txt
tests/unit/nix/pinned/aarch64-linux.txt
tests/golden/api-surface/roots.json
tests/golden/api-surface/capability-api.txt
tests/golden/api-surface/capability-trait-impls.txt
tests/golden/api-surface/hidden-public-api.txt
tests/golden/api-surface/public-api.txt
tests/golden/bazel-rust-coverage.json
tests/golden/bazel-rust-artifact-baselines.json
tests/golden/bazel-rust-query.json
tests/golden/flake-check-matrix/x86_64-linux.txt
tests/golden/flake-check-matrix/aarch64-linux.txt
.github/workflows/pr-l1-static-fast.yml
changelog.d/adr052-bazel-foundation.md
```

Initial hub and module lock generation occurs here only after the tested
generator is integrated and routed. Lock refresh follows the authority that
changed, and `MODULE.bazel.lock` is always committed last:

- initial or combined setup commits `bazel/cargo/product.lock`, then
  `bazel/cargo/walker.lock`, then `MODULE.bazel.lock`; while the module lock is
  absent, the two initial repins alone use command-local
  `--lockfile_mode=off`, which creates no module lock and is refused after
  bootstrap;
- a later product manifest change regenerates and commits
  `packages/Cargo.lock`, then product repin, then module refresh, and proves
  the walker Cargo lock and `bazel/cargo/walker.lock` byte-identical;
- a later walker manifest or lock change regenerates and commits the walker
  Cargo lock, then walker repin, then module refresh, and proves
  `packages/Cargo.lock` and `bazel/cargo/product.lock` byte-identical.

Byte-identity is proved by comparing recorded hashes of the untouched files
before and after the refresh. Only after those commits does clean no-op
validation run.
Slices may place previews under `.scratch/`; no slice commits a generated
lock, pin, BUILD file, coverage golden, or query golden.

### Validation

On the committed integrated candidate:

```bash
set -euo pipefail
assert_clean() {
  git diff --exit-code -- "$@"
  git diff --cached --exit-code -- "$@"
  test -z "$(git status --porcelain --untracked-files=all -- "$@")"
}
assert_clean packages/Cargo.lock
(cd packages && cargo generate-lockfile --offline)
assert_clean packages/Cargo.lock
(cd packages && cargo metadata --locked --offline --format-version 1)
assert_clean .bazelignore bazel packages MODULE.bazel.lock
(cd packages && cargo xtask gen-bazel --check)
assert_clean .bazelignore bazel packages MODULE.bazel.lock
assert_clean packages/policy-inputs
(cd packages && cargo xtask gen-package-policy-inputs --check)
assert_clean packages/policy-inputs
assert_clean bazel/cargo/product.lock
(cd packages && cargo xtask bazel-repin --hub product)
assert_clean bazel/cargo/product.lock
assert_clean bazel/cargo/walker.lock
(cd packages && cargo xtask bazel-repin --hub walker)
assert_clean bazel/cargo/walker.lock
assert_clean MODULE.bazel.lock
(cd packages && cargo xtask bazel-module-refresh)
assert_clean MODULE.bazel.lock
assert_clean tests/unit/nix/pinned
make nix-unit-pin
assert_clean tests/unit/nix/pinned
make check-tier0
make test-lint
make test-rust-main
make test-rust-broker
make test-rust-guest-shell-runner
make test-rust-schema
make test-rust-inventory
make test-rust-supply-chain
make test-rust
make test-policy
make test-drift
make test-flake
make test-nix-unit
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Run the ADR 0054 x86 Nix block natively. On the stable PR head, require the
generated native `test-flake-aarch64` job and the x86 realized job to pass.
The arm job must realize all six checks and run
`make test-rust-supply-chain`; renderer coverage and both results must bind the
same unchanged stable head.

### Mechanical done condition

All must be true:

- the scoped authority inventory over `packages/Cargo.lock`,
  `packages/Cargo.guest.lock`, both retired nested lock paths, and the walker
  lock reports root product, generated guest, and walker locks only; unrelated
  lab, proof, and compile-fixture locks remain outside this assertion;
- Cargo metadata reports broker and guest as root workspace members;
- broker and guest manifests have no `[workspace]` or workspace profiles;
- guest manifest has normal libshpool and empty real-libshpool;
- no `crate.spec` exists;
- MODULE declares only product and walker hubs;
- product repin changes only product lock when stale and nothing when current;
- walker repin changes only walker lock and is current;
- module refresh changes only `MODULE.bazel.lock` when stale, is idempotent,
  uses matching absolute startup options, emits the exact remediation, and is
  unreachable from Make and workflows;
- retired hubs pass exact injected argv/cwd refusal tests;
- the compiler-derived API census proves the closed `VerifiedExecutable`
  public/hidden/inherent/trait surface, and focused rustdoc compile-fail
  examples prove downstream construction, descriptor access, coercion,
  formatting/serialization, duplication/conversion, and minting are absent;
  no Cargo-shelling compile fixture exists;
- the safe runner execution API consumes the verified handle by value and the
  only low-level implementation is its `unsafe_code = "deny"` quarantined
  `src/sys.rs`, with item-level allowances matching the reviewed broker
  convention; parent-prepared async-signal-safe child work preserves declared
  stdin/stdout/stderr and executes the original verified open file description
  through a private CLOEXEC descriptor; tests reject a helper binary/runfile/
  path, direct helper invocation, fd-0 transport, reopen, `/proc`, `fexecve`,
  path fallback, leaked provider/private fd, replaced stdin, a second unsafe
  file, and broad lint overrides;
- the Nix-built `d2b-bazel-seccomp-exec` digest and pinned libseccomp Rust/C
  inputs are reviewed; generated rule/toolchain and `aquery` inventories prove
  it is the executable of every stable/nightly compile/build action and every
  generated/custom test target, with no `--run_under` or pre-wrapper process;
  inherited socket, ordinary-ring, SQPOLL-ring, and fixed-socket ring plants
  refuse before load; no stage or strategy falls back; all eight
  socket/io_uring plants return the policy errno; and exact stage diagnostics
  pass leak-rejection tests;
- `gen-bazel --check` and `gen-package-policy-inputs --check` pass;
- the selected-context oracle joins locked offline target-filtered root
  metadata (identities, sources, candidate edges, `cfg`),
  `packages/Cargo.lock` plus the committed git archive pin (checksums), and
  package-selected stable `cargo tree` traversals pinned to
  `--locked --offline -p <package> --target <target> --no-default-features`
  with explicit `--features`, `--charset ascii`, `--prefix depth`,
  `--no-dedupe`, and the repository-pinned delimited `--format`, with
  production and dev-inclusive edges traversed separately and every traversal
  identity cross-checked against metadata and the lock; it uses no synthetic
  manifest or splice;
- a feature canary on an unrelated workspace member enabling an
  otherwise-absent feature of a dependency shared with broker or guest cannot
  enter either selected output;
- module refresh is committed after both hub locks in every refresh order, and
  the untouched hub's Cargo and Bazel inputs are proved byte-identical;
- the Cargo gate's package deny, audit, and source census read the four native
  selected policy inputs for broker GNU and guest musl, the package audit is
  pinned with `--no-fetch`, and the aggregate root-lock and
  `Cargo.guest.lock` checks remain independent and enforcing;
- `tests/tools/assert-pinned-tests.sh` selects packages from the one root
  lock, backs up and restores no lock file, and leaves the candidate clean;
  the five affected `tests/golden/pinned/*.txt` comment files describe the
  root-lock shape;
- generic Cargo and Nix build/test and Clippy contexts exclude broker and guest
  exactly, while each dedicated context retains its selectors;
- the exact four policy contexts and twelve native check wrappers exist, six
  per system;
- both dedicated Nix derivations retain the exact committed
  `wl-proxy-0.1.2` output hash and all three pin mutations fail;
- broker and guest derivations realize on each native system; exactly four
  artifact baseline rows record broker interpreter/`DT_NEEDED`, guest static
  linkage, executable sizes, closure counts/digests without store paths,
  selected-policy digests, null initial size authorization, and no row-level
  allowance field. Unchanged and
  authorized-growth positives pass; missing/denied/stale/replayed/wrong-row/
  arithmetic/absolute-rationale/size-plus-one, linkage, closure,
  sibling-leakage, static-broker, and dynamic-guest mutations fail;
- every guest static artifact is `ET_DYN`, reports the expected native
  `e_machine`, has no `PT_INTERP` and no `DT_NEEDED`, and the non-PIE and
  wrong-machine plants fail;
- the release workflow uses the root manifest with `--locked`, explicit
  package/bin/default-feature selectors and `packages/target/release`, and its
  cache mapping is `packages -> target` plus the explicit gate directories;
- both existing fail-closed gate scripts are updated for that shape and remain
  present;
- `make nix-unit-pin` is a no-op and `make test-nix-unit` passes;
- new fixture-independent policy binaries appear exactly once in
  `tests/lib.sh`, run under `make test-policy`, and are excluded from fixture
  contracts by the regression test;
- the six guest license exceptions are exact and different-package plants fail;
- the six-check x86 inventory and six-check arm inventory pass and the arm
  stable head also passes
  `make test-rust-supply-chain`;
- `test-flake-aarch64` is enforcing, not advisory, and the advisory mutation
  fails the renderer/manifest guard;
- all ten spec003w0 binding-doc, ADR-status, and policy paths describe the unified workspace
  without process markers, with ADR 0054 governing the newer shape and ADR
  0038 unchanged;
- every mutating check above leaves the candidate clean;
- Cargo remains the required `test-rust` executor;
- the ten-role integrated-diff panel signs off;
- the PR is sealed as `spec003w0` and merged.

## spec003w1 - Complete Bazel Coverage Carriers

### Deliverable

All eighteen Bazel carriers, native first-party configured contexts, exact
coverage map, runner and locator implementation, offline supply-chain
carriers, nightly API census, schema, no-bash, inventory, and execution-manifest
adapter. Cargo remains authoritative.

### Integrator prep

Prep fixes shared interfaces in:

```text
packages/d2b-bazel-runner/Cargo.toml
packages/d2b-bazel-runner/src/lib.rs
packages/d2b-bazel-runner/src/contracts.rs
packages/d2b-test-locator/Cargo.toml
packages/d2b-test-locator/src/lib.rs
packages/d2b-test-locator/src/contracts.rs
packages/xtask/Cargo.toml
packages/xtask/src/main.rs
packages/Cargo.lock
bazel/cargo/product.lock
MODULE.bazel.lock
bazel/generated/locator-migration-files.json
```

Those files contain complete green interfaces, complete future dependencies,
crate-root and xtask contract seams, and the exact sorted migration file
inventory. They do not declare not-yet-present implementation modules. Scope
tests load their scope-owned implementation modules through test-local paths;
after the parallel frontier closes, the integrator wires completed modules
into the prep-owned roots. No spec003w1 slice edits a prep-owned root,
contract, inventory, or lock. If a product manifest changes, prep regenerates
`packages/Cargo.lock`, then product-repins, then module-refreshes, committing
each generated result in that order before no-op checks, and proves both walker
inputs byte-identical. If a walker manifest or lock changes, prep regenerates
the walker Cargo lock, then walker-repins, then module-refreshes in that order,
and proves `packages/Cargo.lock` and `bazel/cargo/product.lock`
byte-identical. `MODULE.bazel.lock` is always committed last. The
integrator owns Make, `ci/rust/BUILD.bazel`, and generated reconciliation after
slice integration.

### Slice ownership

| Slice | Owned files |
| --- | --- |
| `spec003w1-main` | `bazel/carriers/main.bzl`, `packages/d2b-bazel-runner/tests/main_topology.rs` |
| `spec003w1-api` | `bazel/rules/channel_transition.bzl`, `bazel/rules/rustdoc_json.bzl`, `bazel/rules/tests/channel_transition.rs`, `bazel/rules/tests/rustdoc_json.rs` |
| `spec003w1-broker` | `bazel/carriers/broker.bzl`, `packages/d2b-bazel-runner/tests/broker_topology.rs`, `packages/d2b-bazel-runner/tests/broker_exclusive.rs` |
| `spec003w1-guest` | `bazel/carriers/guest.bzl`, `packages/d2b-bazel-runner/tests/guest_topology.rs` |
| `spec003w1-supply-chain` | `bazel/vendor/repositories.bzl`, `bazel/supply_chain/BUILD.bazel`, `bazel/supply_chain/defs.bzl`, `packages/xtask/src/bazel_yanked.rs`, `packages/xtask/tests/bazel_yanked.rs`, `packages/xtask/tests/bazel_action_network.rs` |
| `spec003w1-runner` | `packages/d2b-bazel-runner/src/coverage.rs`, `packages/d2b-bazel-runner/src/topology.rs`, `packages/d2b-bazel-runner/src/runner_env.rs`, `packages/d2b-bazel-runner/src/junit.rs`, `packages/d2b-bazel-runner/src/manifest.rs`, `packages/d2b-bazel-runner/tests/coverage.rs`, `packages/d2b-bazel-runner/tests/result_publication.rs`, `packages/d2b-bazel-runner/tests/provider_execution.rs` |
| `spec003w1-locator` | `packages/d2b-test-locator/src/locator.rs`, `packages/d2b-test-locator/tests/locator.rs`; the prep-owned disposition inventory records existing Cargo-only call sites as retained and requires no dynamic source-file ownership |
| `spec003w1-no-bash` | `bazel/carriers/no_bash.bzl`, `tests/tools/no-bash-ast-walker/src/main.rs`, including its inline unit tests |
| `spec003w1-census-generator` | `bazel/carriers/schema.bzl`, `bazel/carriers/inventory.bzl`, `bazel/carriers/stub.bzl`, `packages/d2b-bazel-runner/tests/schema_inventory.rs`, `packages/xtask/src/bazel.rs`, `packages/xtask/src/schema.rs`, `packages/xtask/tests/bazel_generation.rs` |
| `spec003w1-coverage` | `bazel/carriers/coverage.bzl`, `packages/d2b-bazel-runner/tests/coverage_map.rs`, `packages/xtask/tests/policy_ci.rs` |
| Integrator | `Makefile`, `ci/rust/BUILD.bazel`, `packages/d2b-bazel-runner/src/lib.rs`, `packages/d2b-test-locator/src/lib.rs`, `packages/xtask/src/main.rs`, the exact nine generated paths listed under Expected Implementation Locations, `tests/golden/bazel-rust-coverage.json`, `tests/golden/bazel-rust-query.json`, `bazel/supply_chain/yanked-snapshot.json`, `changelog.d/adr052-bazel-carriers.md` |

The locator disposition inventory is generated and committed in prep. It
records existing Cargo-only call sites as retained and the two exact
scope-owned locator files above as the only changed paths; it is not a dynamic
ownership expansion. The no-bash tests in `main.rs` are added before its
implementation change and are sequential because they edit the same file.

The integrator alone regenerates BUILD files, `.bazelignore`, module and hub
locks, `bazel/supply_chain/yanked-snapshot.json`, all files beneath
`bazel/generated/` including `bazel/generated/no-shell-inventory.json`, and
both coverage/query goldens.
Slices emit scratch previews only.

`spec003w1-coverage` owns `packages/xtask/tests/policy_ci.rs` alone. It adds
all six shadow Make targets (`test-bazel-rust`, `test-bazel-rust-main`,
`test-bazel-rust-api`, `test-bazel-rust-broker`, `test-bazel-rust-aux`,
and `bazel-shutdown`) to `APPROVED_MAKE_TARGETS` in the same wave that
introduces them, with a positive test that every approved shadow name resolves
to a rule in a supplied Makefile fixture and that a workflow step calling it
is accepted, and
negative tests that an unapproved `test-bazel-rust-<name>` call and an
approved shadow name with no Makefile rule are both rejected. The integrated
repository consistency form runs after the integrator adds the six entry
points and must be green on the candidate. The integrator reconciles `Makefile` and
`ci/rust/BUILD.bazel`, then wires completed modules into the prep-owned runner,
locator, and xtask roots; prep already owns Cargo manifests and their ordered
lock refresh.

### Validation

```bash
set -euo pipefail
make test-bazel-rust
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
D2B_EXECUTION_MANIFEST=.scratch/spec003w1-manifest.json make test-bazel-rust
make test-rust
make test-policy
make test-drift
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Also compare Cargo's enforcing exit status and normalized finding set with the
decomposed Bazel deny, audit, and yanked union for full-product main and exact
selected broker and guest projections. Any difference fails validation.

### Mechanical done condition

- coverage map has exactly eighteen IDs and no orphan carrier;
- every first-party crate is native and every configured context census is
  exact and nonempty;
- product and walker containment checks pass;
- main, guest, and broker topology tests pass;
- all broker suites carry `tags = ["exclusive"]`, overlap no test, and the
  tag-removal mutation fails;
- generated rule/toolchain and `aquery` inventories prove every governed
  compile/build action and generated/custom test target names the seccomp
  wrapper as its executable, stable/nightly action-kind coverage is exact,
  Bazel setup before the Rust payload is not claimed filtered, no
  `--run_under`, pre-wrapper process, unsandboxed, or stage fallback exists,
  inherited socket/ring/SQPOLL/fixed-socket plants refuse before load, all
  eight socket/io_uring plants return the policy errno, every stage diagnostic
  passes exact-message/leak tests, external-egress and live-index plants fail,
  every repository-rule fetch is pinned, and every mandatory socket-using case
  appears in the exact
  same-commit non-advisory Cargo compatibility census;
- package carriers use the four spec003w0 policy contexts, the one product-lock
  yanked snapshot with exact main/broker/guest semantics, and pinned no-fetch
  audit;
- no-bash walk/read/parse errors refuse and parsed-file census equals the
  governed manifest and declared input set;
- `bazel/generated/no-shell-inventory.json` is committed, nonempty in each of
  its governed and declared sets, and drift-checked; those two source sets are
  equal; every spawn-site source is governed; every governed source has one
  successful scan record including zero-site sources; raw and unique
  scan-record counts each equal governed-source count; a fresh scan's exact
  keyed spawn-site set equals the committed `spawnSites` set; and the empty,
  missing-entry, extra-entry, ungoverned-spawn, missing-zero-site-record, and
  planted-shell plants each fail as exactly
  `no-shell-inventory-empty`, `no-shell-inventory-missing-entry`,
  `no-shell-inventory-extra-entry`,
  `no-shell-inventory-unguarded-spawn`,
  `no-shell-inventory-missing-zero-site-record`, and
  `no-shell-inventory-planted-shell`;
- all six shadow Make targets appear in `APPROVED_MAKE_TARGETS`, each
  resolves to a real Makefile rule, an unapproved `test-bazel-rust-<name>`
  call fails, and an approved shadow name with no Makefile rule fails;
- schema performs two independent nonempty exact-census generations and its
  mismatch and empty-output plants fail;
- stub-no-socket rejects missing executable, wrong binary identity, and
  runtime state, and owns no socket-denial plant;
- pinned inventory rejects empty, missing, and extra inventories;
- runner tests cover prior-evidence invalidation, multi-carrier attribution,
  sorted atomic manifest v1 evidence for success, failure, and handled
  interruption, original-verdict preservation, ignored-case fidelity, complete
  forbidden-value redaction across JUnit, bounded `test.log`, emitted evidence,
  and exporter diagnostics, one canonical common sink-kind/retention pair,
  non-contradictory typed complete/degraded evidence, required complete
  publication, and no shell;
- `bazel/generated/evidence-sink-policy.json` carries measured byte and record
  bounds and the four closed age/count retention classes; every sink, limit,
  age, count, and expiry mutation passes;
- `D2B_RUST_BUDGET` validation and propagation bound Bazel jobs and suite
  concurrency as one combined limit; invalid and multiplicative mutations
  fail;
- Cargo and decomposed Bazel supply-chain status and normalized finding unions
  match for main, broker, and guest;
- all seeded spec003w1 carrier negatives fail;
- Cargo and Bazel censuses match;
- Cargo remains authoritative;
- `changelog.d/adr052-bazel-carriers.md` is a semantic fragment owned and
  generated by the integrator;
- panel, seal `spec003w1`, and merge complete.

## spec003w2 - Operational Safety

### Deliverable

Bounded local state, one startup-option construction, synchronous trim, safe
cleanup, deadline and process-group control, exact recovery messages, and the
temporary cold-local evidence helper.

### Integrator prep and ownership

Prep lands complete green interfaces in
`packages/d2b-bazel-runner/Cargo.toml`,
`packages/d2b-bazel-runner/src/lib.rs`,
`packages/d2b-bazel-runner/src/clock.rs`,
`packages/d2b-bazel-runner/src/process_backend.rs`,
`packages/xtask/Cargo.toml`,
`packages/xtask/src/main.rs`,
`packages/d2b-bazel-support/src/startup.rs`, plus prep-only
`packages/d2b-bazel-runner/tests/process_backend_contract.rs` and
`packages/d2b-bazel-support/tests/startup_contract.rs`. Prep owns every spec003w2
crate-root and contract seam but does not declare not-yet-present
implementation modules. Scope tests load those modules through test-local
paths; the integrator wires them after the parallel frontier closes. No slice
edits a prep-owned root, contract, or lock. If a product manifest changes, prep
regenerates `packages/Cargo.lock`, then product-repins, then
module-refreshes, commits those generated outputs in order, and proves the
three commands are clean no-ops and both walker inputs byte-identical before
dispatch. If a walker manifest or lock changes, prep regenerates the walker
Cargo lock, then walker-repins, then module-refreshes, and proves
`packages/Cargo.lock` and `bazel/cargo/product.lock` byte-identical.
`MODULE.bazel.lock` is always committed last.

| Slice | Owned files |
| --- | --- |
| `spec003w2-process` | `packages/d2b-bazel-runner/src/deadline.rs`, `packages/d2b-bazel-runner/src/process.rs`, `packages/d2b-bazel-runner/tests/deadline.rs`, `packages/d2b-bazel-runner/tests/process.rs` |
| `spec003w2-cleanup` | `packages/d2b-bazel-runner/src/cleanup.rs`, `packages/d2b-bazel-runner/tests/cleanup.rs`, the cleanup-only test module in `packages/d2b-contract-tests/tests/policy_docs.rs` |
| `spec003w2-local-wrapper` | `Makefile`, `.bazelrc`, `packages/d2b-bazel-support/tests/startup.rs` |
| `spec003w2-recovery` | `packages/d2b-bazel-runner/src/recovery.rs`, `packages/d2b-bazel-runner/tests/recovery.rs` |
| `spec003w2-evidence` | `packages/xtask/src/bazel_evidence.rs`, `packages/xtask/tests/bazel_evidence.rs` |
| Integrator | `packages/d2b-bazel-runner/src/lib.rs`, `packages/xtask/src/main.rs`, `packages/Cargo.lock`, `bazel/cargo/product.lock`, `MODULE.bazel.lock`, generated BUILD files listed by the committed generation manifest, `changelog.d/adr052-bazel-safety.md` |

Any shared support trait change belongs in prep. No slice extends it.

### Validation

```bash
set -euo pipefail
make test-rust-main
make test-policy
make check-tier0
make test-drift
make test-bazel-rust
D2B_CLEAN_DRY_RUN=1 make clean
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Run every cleanup, descriptor, race, deadline, process, recovery, startup,
trim, cache-limit, and no-shell mutation.

### Mechanical done condition

- all local state is under `.scratch/bazel/`;
- one startup option set selects every server command;
- synchronous trim completes before measurement;
- every unsafe cleanup plant deletes nothing;
- expiry repeatedly observes with `EXITED|NOWAIT|NOHANG` throughout the
  independently timed full grace, treats observations as informational,
  unconditionally kills the group, then reaps the direct child;
- blocking-wait, early-reap, shortened-grace, and conditional-kill mutations
  fail;
- provider and cleanup auxiliary descriptor inheritance mutations fail;
- every recovery code emits its exact ADR 0052 command sequence and only its
  own redacted remedy; wrong-remedy and unsafe-external-action mutations fail;
- provider, sanitizer, sink-limit, exporter, publication, and qualification
  degradation rows name their stable repository-relative input, corrective
  action, and rerun command, with exact-message and redaction mutations;
- the ADR-0054 drift table covers product and walker hub locks, module lock,
  generator output, package-policy output, yanked snapshot, ambient repin
  controls, and unexpected tracked mutation with exact `nix develop`, then
  `cd packages`, command, review/commit, and rerun steps;
- no new gate exists;
- panel, seal `spec003w2`, and merge complete.

## spec003w3 - Cache-Free Shadow CI

### Deliverable

Non-required four-slice shadow workflow, workflow-policy fixtures,
qualification record capture, the typed qualification validator, and cold-CI
feasibility measurement.

### Ownership

| Slice | Owned files |
| --- | --- |
| `spec003w3-shadow-workflow` | `.github/workflows/pr-bazel-rust.yml`, `packages/xtask/src/bazel_qualification.rs`, `packages/xtask/tests/bazel_qualification.rs` |
| `spec003w3-workflow-policy` | `packages/xtask/tests/policy_ci.rs`, `packages/xtask/tests/fixtures/ci/cache-save-pr.yml`, `packages/xtask/tests/fixtures/ci/cache-post-step-pr.yml`, `packages/xtask/tests/fixtures/ci/unknown-cache-writer-pr.yml`, `packages/xtask/tests/fixtures/ci/actions-write-job-pr.yml`, `packages/xtask/tests/fixtures/ci/actions-write-workflow-pr.yml`, `packages/xtask/tests/fixtures/ci/shadow-valid.yml`, `packages/xtask/tests/fixtures/ci/qualification-wrong-event.yml`, `packages/xtask/tests/fixtures/ci/qualification-missing-count.yml` |
| Integrator | `packages/xtask/src/main.rs`, workflow allowlist, shared trigger and path-filter reconciliation, `changelog.d/adr052-bazel-shadow.md` |

### Validation

```bash
set -euo pipefail
make test-bazel-rust-main
make test-bazel-rust-api
make test-bazel-rust-broker
make test-bazel-rust-aux
make test-rust-main
make test-policy
make test-lint
make check-tier0
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Inspect a draft PR run for four attributed slices, zero cache actions,
read-only permissions, approved Make targets, and no qualification record.
Run only the injected fixture suite for the qualification validator in this
wave:

```bash
(cd packages && cargo test -p xtask --test bazel_qualification)
```

The no-argument fixed-path command is not run in spec003w3 because
`evidence/qualification.json` does not exist until spec003w4. The fixture suite
includes complete, degraded no-verdict, page-gap, missing-attempt,
omitted-push, and wrong-reference streams. The real
`cargo xtask bazel-qualification-validate` command runs only after spec003w4
initializes and completes the fixed record.

### Mechanical done condition

- workflow is not required;
- a pull-request run emits no qualification record and contains zero cache
  actions;
- pull-request permissions are read-only;
- policy fixtures prove every writer and permission refusal;
- protected `v3` pushes produce complete qualification records with explicit
  `bazelRestoreCount` of zero, explicit `bazelSaveCount` and
  `bazelPublicationCount`, and four complete `sliceDurationsSeconds`
  entries;
- every protected-`v3` push with either or both workflows lacking a verdict
  produces a bounded typed degraded record that resets the streak; no-verdict
  pushes are never silently omitted;
- `packages/xtask/src/bazel_qualification.rs` exists with tests, derives
  every threshold from immutable evidence references, and refuses omitted,
  forged, duplicate, inconsistent, and wrong-candidate references;
- a record whose boolean mirror disagrees with the derived verdict is refused,
  and no record qualifies through a boolean field;
- the fixture validator passes and the no-argument command is unreachable from
  Make and workflows but is not invoked before spec003w4 creates its fixed
  input;
- cold feasibility measurement exists and either supports the ceiling or names
  one authorized remedy;
- panel, seal `spec003w3`, and merge complete.

## spec003w4 - Immutable Qualification

### Deliverable

Only `specs/003-adr052-bazel-rust/evidence/qualification.json`.

### Ownership

One curator owns that file. Every log, manifest, and raw measurement stays
under `.scratch/`.

### Evidence

- ten consecutive matching qualification records;
- eighteen isolated surface failure records;
- exact Cargo and Bazel censuses;
- five topology proofs and per-case publication;
- twenty consecutive executions for each broker context with
  `tags = ["exclusive"]`, no overlap, and the tag-removal mutation;
- complete locator proof;
- three warm, three cold-local, and five cold-CI measurements;
- all four package policy contexts with exact source and checksum results;
- Cargo versus decomposed Bazel deny, audit, and yanked enforcing exit-status
  and normalized-finding equality for main, broker, and guest;
- one product-lock yanked snapshot, full main evaluation, exact broker/guest
  projections, reviewed refresh, and offline check;
- the x86 and arm six-check realization sets, including four dedicated
  artifact realizations, exact broker linkage, selected closure
  counts/digests, exactly four measured baseline rows, and size-authorization
  fixtures;
- native arm supply-chain and renderer results from the same stable head;
- all safety and workflow plants;
- explicit `bazelRestoreCount` of zero, `bazelSaveCount`, and
  `bazelPublicationCount` in every shadow record, with four
  `sliceDurationsSeconds` entries in each cold record;
- the typed qualification validator's derived verdict for the record, plus its
  omitted, forged, duplicate, inconsistent, and wrong-candidate reference
  refusals;
- module-refresh lock-only/idempotence/remediation evidence and both exact Nix
  output-hash results;
- generated rule/toolchain plus `aquery` seccomp executable-field and
  stable/nightly action-kind coverage, wrapper-as-test-executable and explicit
  pre-payload Bazel setup scope, no-`--run_under`/pre-wrapper/
  unsandboxed/stage-fallback results, inherited socket/ring/SQPOLL/
  fixed-socket plants, all eight socket/io_uring plants, every fixed-code stage
  diagnostic, external-egress and live-index results, and the pinned
  repository-rule fetch inventory;
- exact same-commit non-advisory Cargo compatibility-carrier passes for every
  mandatory socket-using test;
- expected native `e_machine`, `ET_DYN`, no-interpreter and no-`DT_NEEDED`
  evidence plus non-PIE and wrong-machine plant results;
- manifest/JUnit/bounded-sanitized-test.log/emitted-evidence/exporter
  redaction, ignored-case, original-verdict, typed-degraded-evidence, no-shell,
  and combined Bazel-plus-suite budget mutations;
- the committed `bazel/generated/no-shell-inventory.json` digest with its
  equal nonempty governed/declared sets, governed spawn sources, complete
  per-source scan records including zero-site sources, raw and unique
  scan-record counts each equal governed-source count,
  fresh-scan/committed spawn-site-key equality, and all six relationship and
  planted-shell results.
- enforcement records and advisory-classification mutations for
  `test-flake-aarch64`, all four Rust slices, and `test-rust`.

### Mechanical done condition

`cargo xtask bazel-qualification-validate` derives every threshold from the
record's immutable evidence references and returns success. The record contains
no pending or incomparable item, binds one candidate commit where required,
contains no raw logs or attestations, passes both Rust aggregates and fixture
companion validation, and is panel-signed, sealed as `spec003w4`, merged, and
immutable.

## spec003w5 - Promotion and Promotion Record

### Deliverable

Switch eighteen surfaces to Bazel, preserve fixture behavior and required
context, replace eight CI leaves with four slices, remove shadow workflow,
perform ordered cache cutover, and record promotion in a follow-up.

### Integrator prep and ownership

Prep owns complete green interfaces in
`packages/xtask/src/bazel_cache_contract.rs` and
`packages/xtask/src/promotion_contract.rs`, plus
`packages/xtask/Cargo.toml`, `packages/xtask/src/main.rs`,
`packages/Cargo.lock`, `bazel/cargo/product.lock`, and
`MODULE.bazel.lock`. Prep owns the cache and promotion contracts and a green
xtask root that does not declare not-yet-present implementation modules.
Cache-slice tests load their modules through test-local paths; after the
parallel frontier closes, the integrator wires them into xtask. No promotion
slice edits the prep-owned contracts, routing files, manifests, or locks. If a
product manifest changes, prep regenerates the product Cargo lock,
product-repins, and module-refreshes in that order, then proves all three are
clean no-ops and both walker inputs byte-identical. If a walker manifest or
lock changes, prep regenerates the walker Cargo lock, walker-repins, and
module-refreshes in that order, and proves `packages/Cargo.lock` and
`bazel/cargo/product.lock` byte-identical. `MODULE.bazel.lock` is always
committed last.

| Slice | Owned files |
| --- | --- |
| `spec003w5-promotion-make` | `Makefile`, `tests/test-rust.sh` |
| `spec003w5-promotion-manifest` | `tests/unit/meta/ci-runner-regression.py`, `tests/layer1-jobs.json`, `tests/tools/layer1-jobs.py`, `tests/ci/layer1-workflow.template.yml` |
| `spec003w5-cache` | `packages/xtask/src/bazel_cache.rs`, `packages/xtask/src/post_promotion_observations.rs`, `packages/xtask/src/promotion_record.rs`, `packages/xtask/src/release_containment.rs`, `packages/xtask/tests/bazel_cache.rs`, `packages/xtask/tests/post_promotion_observations.rs`, `packages/xtask/tests/promotion_record.rs`, `packages/xtask/tests/release_containment.rs`, `packages/xtask/tests/policy_ci.rs`, `packages/xtask/tests/fixtures/ci/promoted-cache-valid.yml`, `packages/xtask/tests/fixtures/ci/promoted-cache-prefix-run-id.yml`, `packages/xtask/tests/fixtures/ci/promoted-cache-prefix-sha.yml`, `packages/xtask/tests/fixtures/ci/promoted-cache-delete-newest.yml` |
| `spec003w5-interface-tests` | `packages/d2b-bazel-runner/tests/make_interface.rs` |
| `spec003w5-hybrid-policy` | `packages/d2b-contract-tests/tests/policy_bazel_hybrid_docs.rs`, `tests/lib.sh` |
| `spec003w5-binding-docs` | `AGENTS.md`, `tests/AGENTS.md`, `docs/contributing/gates-and-lints.md`, `tests/README.md`, `docs/reference/test-execution-manifest.md` |
| Integrator | `packages/xtask/src/main.rs`, `.github/workflows/pr-l1-static-fast.yml`, deletion of `.github/workflows/pr-bazel-rust.yml`, `specs/003-adr052-bazel-rust/evidence/promotion-record.json`, `specs/003-adr052-bazel-rust/evidence/post-promotion.json`, `changelog.d/adr052-bazel-promotion.md` |

### Validation

```bash
set -euo pipefail
make layer1-workflow
make test-drift
make check
D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts
```

Also run `cargo xtask bazel-qualification-validate` against the sealed
spec003w4 record, then validate qualification digest, alias status, promoted
deadline policy, cache pagination, synchronous trim, two headroom checks, one
writer, and a one-commit rollback rehearsal. The pre-merge rehearsal identifies
the candidate from the verified current atomic candidate HEAD and the recorded
spec003w5 parent; `promotion-record.json` does not exist yet and is not an
input.

### Mechanical done condition

- required context remains `test-rust`;
- eighteen surfaces use Bazel and fixtures use existing path;
- current Cargo and decomposed Bazel deny/audit/yanked status and normalized
  finding equality still holds for main, broker, and guest at the promotion
  candidate;
- generated CI calls exactly `test-rust-slice-main`,
  `test-rust-slice-api`, `test-rust-slice-broker`, and
  `test-rust-slice-aux`;
- all eight old public leaf names retain exact forwarding subsets, including
  conditional fixture behavior in `test-rust-main`;
- Bazel compatibility aliases print their exact stderr replacement lines,
  forward to the aggregate or matching authoritative slice, and preserve
  status;
- shadow workflow is absent;
- retired Cargo cache writes stop;
- one writer publishes separate bounded caches after ordered maintenance;
- cache maintenance derives deletion authority only from the closed typed
  prefix enum; mixed authorized/unauthorized pagination preserves every
  unauthorized entry, and any page gap, caller-supplied prefix, unknown prefix,
  or ambiguous match refuses before the first delete call;
- each primary key is unique for one successful protected-`v3` run, restore
  prefixes omit run ID and commit SHA, and maintenance retains the newest
  complete generation;
- binding docs, including `tests/README.md` and
  `docs/reference/test-execution-manifest.md`, describe four Bazel slices
  behind `test-rust` instead of eight Cargo leaves, list the exact permanently
  hybrid surface IDs and retained socket-using Cargo cases, state separate
  authorization is required for retirement, and contain no process markers;
- the enforcing fixture-independent type-5 policy derives the exact nonempty
  full compatibility-carrier census from the coverage map, retaining surface,
  selector, test identity, and socket class, and compares it bidirectionally
  with all five fixed hybrid docs and the present semantic fragment; distinct
  same-surface cases remain distinct, independent missing and extra
  full-identity fixtures fail, `make test-policy` runs the lint, and fixture
  contracts exclude it;
- `test-flake-aarch64`, all four generated Rust slices, and the `test-rust`
  rollup are non-advisory, and each advisory-classification mutation fails;
- `cargo xtask bazel-qualification-validate` succeeds against the sealed
  spec003w4 record at the promotion candidate;
- all spec003w5 scope results are integrated or squashed into one atomic promotion
  candidate relative to the recorded spec003w5 parent, and the complete path diff is
  asserted before panel;
- pre-merge rollback rehearsal resolves the candidate from the verified
  current atomic candidate HEAD and the recorded spec003w5 parent, reverts that
  exact atomic commit, and restores Cargo authority; `promotion-record.json`
  is read only after merge;
- promotion PR is panel-signed, sealed as `spec003w5`, and merged;
- the post-merge typed promotion-record validator proves the recorded SHA is
  the actual protected-`v3` PR merge and re-derives the exact sealed candidate,
  content, and snapshot identities; old-SHA, candidate-SHA, wrong-seal, and
  unsealed-merge mutations fail;
- `post-promotion.json` is a bounded atomically replaced complete-state,
  page/stream-count, digest checkpoint and final-ten suffix derived from the
  complete transient protected-`v3` stream, persists no raw cursor, and is
  never an append-only full attempt history;
- follow-up promotion record is panel-signed, sealed as `spec003w5fu1`, and
  merged.

## spec003w6 - Compatibility Alias Removal

### Entry

A containing published semantic release tag exists. Entry runs exactly:

```bash
set -euo pipefail
(cd packages && cargo xtask bazel-promotion-record-validate)
(cd packages && cargo xtask bazel-release-containment-validate)
```

The second command implements this derivation:

1. Read and validate the fixed promotion record.
2. Enumerate local tag references matching
   `^v[0-9]+\.[0-9]+\.[0-9]+$`.
3. Prove promotion ancestry for each transient candidate.
4. Query and compare the peeled origin tag object.
5. Query release metadata and require present, non-draft, and non-prerelease.
6. Persist only a successful tag-reference digest, never the candidate name.

Any containing tag that does not match `^v[0-9]+\.[0-9]+\.[0-9]+$` is not a
release tag; the repository already carries two-component tags such as
`v1.0`, `v1.1`, and `v1.2` that must not satisfy entry. An unpushed tag, a
divergent same-named local and remote tag, a draft release, or a prerelease
also fails entry. The typed promotion-record validator must pass first.
Every `git` and `gh` operation returns a typed result: query errors produce
their own degraded code and exact identifier-free remedy and are never
suppressed, skipped, or treated as absence. The complete closed query/refusal
table in `contracts/make-target-compatibility.md` is binding.
Green-run count is irrelevant.

### Ownership

One `spec003w6-alias-removal` slice owns `Makefile`,
`packages/d2b-bazel-runner/tests/make_interface.rs`,
`packages/xtask/tests/policy_ci.rs`, `AGENTS.md`, `tests/AGENTS.md`,
`tests/README.md`, `docs/contributing/gates-and-lints.md`,
`docs/reference/test-execution-manifest.md`,
`changelog.d/adr052-bazel-alias-removal.md`, and the alias fields in
`specs/003-adr052-bazel-rust/evidence/post-promotion.json`. It does not edit
Cargo implementation files.

### Mechanical done condition

The interface test is updated and observed failing before alias removal, then
passes after the Make edit. Only Bazel-specific aliases are removed;
`make bazel-shutdown`,
`make test-rust`, and all eight public leaf names remain; no workflow names a
removed alias; the type-5 hybrid disclosure census matches every governed doc
and the alias-removal fragment; validation and fixture contracts pass; panel, seal
`spec003w6`, and merge complete.

## spec003w7 - Cargo Implementation Retirement

### Entry

`post-promotion.json` contains ten distinct ordered green promoted
protected-`v3` `test-rust` run units derived from the complete paginated run
inventory, where a unit is a distinct push-created (run ID, head SHA) pair and
never an attempt.
The typed promotion-record validator passes before the run inventory is read.
Release containment is irrelevant.

### Ownership

One `spec003w7-cargo-retirement` slice owns `tests/test-rust.sh`,
`packages/xtask/src/post_promotion_observations.rs`,
`packages/xtask/tests/post_promotion_observations.rs`,
`packages/xtask/tests/policy_workspace.rs`, `AGENTS.md`, `tests/AGENTS.md`,
`docs/contributing/gates-and-lints.md`,
`changelog.d/adr052-cargo-retirement.md`, and the typed run-unit inventory
and derived validator result in
`specs/003-adr052-bazel-rust/evidence/post-promotion.json`.

spec003w7 qualification and code preparation may proceed before spec003w6.
Its shared binding-doc and `post-promotion.json` task waits for merged
spec003w6, rebases onto it, reruns the entire validation, and receives a new
ten-seat panel verdict before merge.

### Mechanical done condition

Only Cargo implementations for the eighteen surfaces and unreachable
Cargo-only plumbing are removed. Every public Rust Make name, all Bazel
carriers, fixture mode, fixture IDs, and exact socket-using Cargo compatibility
cases remain. Retirement docs and the semantic changelog list the permanently
hybrid surface IDs and the separate authorization requirement, and the type-5
policy proves each governed doc and the retirement fragment exactly matches
the nonempty source census. `make check`,
Rust, policy, drift, and fixture validation pass; panel, seal `spec003w7`, and
merge complete.

The eligibility check inventories every promoted protected-`v3` `test-rust`
run unit. A unit is one distinct push-created (run ID, head SHA) pair carrying
its complete `1..max` attempt history, push event, `v3` branch, terminal
conclusion, immutable creation ordering metadata, and verified promotion
ancestry. Attempts are nested history of one unit, never streak positions: the
unit's conclusion normalizes to its highest terminal attempt, and no further
attempt of the same unit may increment the streak. Units are ordered by
immutable creation order, `createdAt` then run ID; rerun start time is never
an ordering input, so an old rerun cannot move behind newer failures. The check
rejects incomplete pagination, missing attempts within a unit, attempts with
conflicting head SHA or promotion provenance, missing or duplicate unit
identities, non-push or non-v3 runs, pre-promotion commits, and nonterminal
conclusions. It derives resets and the current streak from the ordered units
and ignores any self-asserted eligible, count, or run-ID summary field.
Retirement requires the derived final ten distinct ordered units to be
successes with no intervening failed or cancelled result. Repeated-attempt and
old-rerun-after-failure fixtures prove both rules.

## Specific Risks and Guards

| Failure made possible | Guard |
| --- | --- |
| Shared lock is mistaken for broker or guest reach. | Package-selected builds, exact production closure, and unrelated-sibling leakage plants. |
| Product hub union becomes first-party authority. | Native configured targets and direct first-party edge census. |
| Empty selected closure passes a no-leakage predicate. | Root, nonempty closure, and exact census precede every predicate. |
| Missing source makes license scan report fewer findings. | Exact selected-source count, readability, and checksum gate before deny. |
| Guest license fix broadens policy for unrelated packages. | Six package-scoped exceptions plus different-package denial plants. |
| `Cargo.guest.lock` silently becomes a third hub. | Exact hub inventory and cache-key classification guard. |
| Repin test mutates product lock. | Injected non-mutating executor with exact argv and cwd. |
| Retired hub starts Bazel before refusing. | Executor call count remains zero for main, broker, and guest refusals. |
| Aarch64 check runs on x86 with a foreign system. | Native runner mapping plus separate foreign-system, wrong-runner, and remote-builder plants. |
| Parked historical foundation code is treated as merged history. | spec003w0 entry inventory and base ancestry check. |
| Walker repin changes during product merge. | Byte-identity check on walker manifest, lock, and Bazel-side lock. |
| Dedicated binary isolation disappears in a broad Nix build. | Separate derivations, explicit package flags, ELF, size, and closure checks. |
| A broker suite loses `exclusive` and races another test's signal/reap state. | Literal-tag guard, overlap plant, and twenty runs per context. |
| A Bazel setup shim runs before the wrapper, or inherited socket/ring authority bypasses ordinary syscall denial. | Generated rule/toolchain plus `aquery` executable-field inventory, wrapper-as-test-executable binding, explicit exclusion of pre-payload Bazel setup from the claim, socket/ring/SQPOLL/fixed-socket preflight, full syscall set, binding/removal/pre-wrapper plants, and no stage/strategy fallback. |
| A mandatory socket test is moved into Bazel and weakens ADR 0052, or is omitted to keep actions offline. | The exact same-commit non-advisory Cargo compatibility census preserves the cases, and promotion/retirement docs and changelog list the permanently hybrid surfaces and separate authorization requirement. |
| Root-lock Nix migration drops the git output hash from one derivation. | Exact key/value assertion and one-sided-pin mutation. |
| A contributor follows Bazel's bare module remediation and starts a second server outside worktree scratch. | Repository-owned lock-only module refresh with absolute startup options and exact remediation. |
| A provider is verified by descriptor but executed through a rebound path, or a runfiles leaf symlink is rejected by strict result-path flags. | One `O_RDONLY|O_CLOEXEC` handle with only `RESOLVE_NO_MAGICLINKS`, private-CLOEXEC same-open-file-description `execveat`, leaf-symlink and rebound tests, a mutation rejecting provider `RESOLVE_BENEATH`, and no fallback. |
| A blocking observation or early reap defeats the independent grace timer. | Recording backend requires repeated `EXITED|NOWAIT|NOHANG`, full grace, unconditional kill, then reap. |
| Cache restore prefixes become unique and never restore, or maintenance deletes the newest entry. | Run/SHA-free prefix fixtures and newest-generation retention test. |
| A shadow Make target is introduced without an allowlist entry, so a shadow workflow escapes the ci-uses-make guard. | All six shadow names enter `APPROVED_MAKE_TARGETS` in the same wave, with an unapproved-call negative and an approved-name-without-Makefile-rule negative. |
| A record qualifies because a boolean verdict field says so while its evidence reference is missing or points at another candidate. | Typed validator derives every threshold from immutable references and refuses omitted, forged, duplicate, inconsistent, and wrong-candidate references; a disagreeing boolean mirror is a refusal. |
| A new runner source spawns a shell and is never scanned because the governed set was implied. | Generated nonempty no-shell inventory compared bidirectionally against governed sources and declared inputs, with empty, missing, extra, and planted-shell plants. |
| The module lock is refreshed before the walker hub, so it pins a stale walker input that the next repin invalidates. | Split refresh authorities commit `MODULE.bazel.lock` last in every order and prove the untouched hub's inputs byte-identical. |
| The pinned inventory backs up and restores a lock file, so a gate mutates the candidate it is validating. | Root-lock package selection with no backup or restore plus tracked, staged, and untracked clean-diff assertions. |
| A two-component or unpushed tag containing the promotion commit is accepted as a release. | Anchored `^v[0-9]+\.[0-9]+\.[0-9]+$` match plus origin resolution and non-draft release checks. |
| A rerun of an old run inflates the streak or reorders behind newer failures. | Streak positions are distinct push-created (run ID, head SHA) units ordered by `createdAt` then run ID, with repeated-attempt and old-rerun-after-failure fixtures. |
| The pre-merge rollback rehearsal reads a promotion record that does not exist yet and silently rehearses nothing. | Rehearsal resolves the candidate from verified candidate HEAD and the recorded spec003w5 parent; promotion-record reads are post-merge only. |
| A verified executable becomes forgeable or descriptor-revealing through a harmless-looking trait, formatter, serializer, constructor, or accessor. | Compiler-derived closed public/hidden/inherent/explicit/auto/blanket API snapshots plus focused rustdoc compile-fail examples. |
| A path helper is rebound, fd 0 stops being stdin, or a multithreaded fork child allocates before exec. | No helper exists. The safe layer consumes the capability; the runner's single broker-convention `sys.rs` boundary receives parent-prepared pointers and collision-free CLOEXEC fds, performs only async-signal-safe child operations, preserves declared stdio, and execs the same open file description. |
| A cache API page interleaves a foreign prefix and maintenance adopts it. | Closed typed prefix enum, mixed-page fixtures, preservation checks, and zero delete calls on every authorization refusal. |
| Tests pass while forbidden values persist in `test.log` or exporter output. | Pre-sink streaming sanitization, committed measured bounds, planted-value absence across every sink, and typed degraded evidence rejected by qualification. |
| Old or excessive diagnostics accumulate after passing sanitizer bounds. | Four closed age/count retention classes, descriptor-relative expiry before publication, and injected boundary/failure tests. |
| A binary growth allowance is copied to another artifact or accepted without review. | Exactly four baseline rows and a candidate/review-digest-bound closed authorization with positive and replay/stale/wrong-row/size-plus-one negatives. |
| A diagnostic or checkpoint leaks a store path or API cursor. | Transient full validation, persisted closed states/counts/digests only, and fixed-code repository-relative digest-only failure tests. |
| A stale but valid promotion SHA unlocks retirement. | Typed record validation against the actual protected-`v3` PR merge and exact `spec003w5` seal before both eligibility paths. |
| One hybrid document silently omits a retained Cargo surface. | Enforcing type-5 bidirectional comparison from the nonempty compatibility census to every governed doc and semantic migration fragment, with independent missing and extra plants. |
| A `git` or `gh` failure looks like no release. | Closed typed degraded query outcomes distinct from semantic refusals, exact identifier-free remedies, and raw-output/query-as-absence negatives. |
| A size or sink record carries two contradictory authorities, or unrelated size arithmetic authorizes growth. | Size delta derives only from an authorization whose prior/new bytes equal the baseline and realized artifact; sink kind and retention occur once in the common evidence record; wrong-measurement, duplicate-source, and cross-variant fixtures refuse. |

## Final Cross-Artifact Verification

After the desired waves:

1. every FR-001 through FR-090 maps to at least one completed task;
2. every SC-001 through SC-043 maps to mechanical evidence;
3. no obsolete product workspace, hub, nested-lock, synthetic-splice, or
   optional-libshpool assumption remains except as a clearly labelled rejected
   or retired case;
4. no implementation file cites a parked historical `spec003-w0-*` or
   `spec003-w0` commit as an ancestor;
5. every completed wave is merged and sealed;
6. planning evidence contains no credentials, logs, transcripts, or
   attestation payloads.
7. every scope-owned path is disjoint within its wave, every prep-owned file is
   untouched by parallel scopes, and the task dependency graph matches the
   task list exactly;
8. every mutating validation command leaves the committed candidate clean;
9. every planning artifact names waves as `spec003w0` through `spec003w7`
   plus `spec003w5fu1`; only historical literal branch names such as
   `spec003-w0` remain otherwise;
10. the canonical qualification cache field spellings `bazelRestoreCount`,
    `bazelSaveCount`, `bazelPublicationCount`, and `sliceDurationsSeconds`
    are the only ones used in spec, plan, data model, quickstart, contracts,
    and tasks;
11. every lock-refresh sequence in every artifact commits `MODULE.bazel.lock`
    last and states which hub's inputs are proved byte-identical.
12. the read-only plan-structure validator passes with unique IDs, existing and
    earlier dependencies, exact adjacency, an acyclic graph, literal exact
    ownership, complete checkbox census and parsing, exact canonical headers,
    and no conflict among incomparable concurrently ready scopes; its positive
    and all fifteen negative fixtures and fixed-code diagnostic contract pass.
13. process references use exactly `spec003w0` through `spec003w7` plus
    `spec003w5fu1`; no other qualified Spec 003 wave is accepted.
14. every authoritative native inventory is six checks per system, artifact
    authority is exactly four rows, and no persisted artifact contains a Nix
    store path or raw pagination cursor.
15. all six no-shell plants appear everywhere, and both raw and unique
    scan-record counts equal governed-source count.
16. promotion and retirement documentation and semantic changelog fragments
    list the exact permanently hybrid surfaces and retained Cargo socket cases.
17. the enforcing type-5 hybrid-disclosure policy derives a nonempty exact
    compatibility census and every governed doc/fragment matches it in both
    directions; missing and extra negatives fail.
18. no helper/path/fd-0 execution design remains outside explicit rejected
    mutations; the runner uses only the safe-by-value API and one
    broker-convention quarantined `sys.rs`, with same-open-file-description,
    stdio, CLOEXEC, rebind-absence, and mutation coverage.
19. compile/build process executable fields and configured test target
    executables bind the seccomp wrapper without `--run_under`, while
    Bazel-owned test setup stays explicitly outside the claim; inherited
    ring/SQPOLL/fixed-socket plants and every fixed-code seccomp stage
    diagnostic pass.
20. release query errors remain typed degraded outcomes, qualification and
    planning-validator failures have exact closed remedies, and no tested
    diagnostic leaks runtime paths, descriptors, OS text, raw output, or
    dynamic identifiers.
