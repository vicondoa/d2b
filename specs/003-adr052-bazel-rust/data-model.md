# Data Model: ADR 0052 Under ADR 0054

These are internal migration and evidence entities, not public application
data. Execution-manifest v1 remains authoritative for Rust gate evidence.

## Modelling rules

- Variants carry only fields valid for that variant.
- A constant invariant is prose, not a mutable field.
- Exact censuses are generated from authoritative inputs. Hand-written package
  or file counts are observations only.
- Repository-relative paths are normalized and sorted.
- An absence predicate is evaluated only after its root and complete nonempty
  census are established.
- Bazel Rust actions are no-network under the Nix-pinned patched Bazel Linux
  sandbox, whose child loads the fixed filter before exec of the full action
  command. Coverage includes IPv4, IPv6, netlink, packet, pathname/abstract
  Unix, socketpair, and io_uring paths in setup, compile/build, test, and
  descendants. Network namespaces are defense in depth and are not
  socket-creation proof. Mandatory socket-using tests remain exact
  non-advisory Cargo compatibility carriers under their existing surface IDs.
  Repository fetches stay outside governed actions, offline, and pinned.
- Underlying test verdict and evidence status are separate. Degraded evidence
  preserves the verdict and is rejected by surface completion and
  qualification.

## Product Workspace

| Field | Rule |
| --- | --- |
| `manifest` | Exactly `packages/Cargo.toml`. |
| `resolver` | Exactly `2`. |
| `members` | Generated Cargo metadata member set containing main, broker, and guest. |
| `lock` | Exactly `packages/Cargo.lock`. |
| `nested_workspaces` | Empty for broker and guest. |
| `nested_locks` | Empty for broker and guest. |

The member and lock package counts are derived. A lock entry is a resolution
member, not evidence that a selected package reaches it.

## Walker Workspace

| Field | Rule |
| --- | --- |
| `manifest` | `tests/tools/no-bash-ast-walker/Cargo.toml`. |
| `lock` | `tests/tools/no-bash-ast-walker/Cargo.lock`. |
| `product_path_dependencies` | Empty. |

The walker remains separate because it is gate plumbing outside the product
package tree.

## Generated Static-Guest Lock Input

| Field | Rule |
| --- | --- |
| `path` | `packages/Cargo.guest.lock`. |
| `producer` | Existing generated static-guest closure process. |
| `consumer` | Existing aggregate static-guest package and policy checks. |
| `cargo_workspace_authority` | Not present as a field because it is always false. |
| `hub_authority` | Not present as a field because it is always false. |

## Dependency Hub

Two variants exist:

| Variant | Manifest | Cargo lock | Bazel-side lock |
| --- | --- | --- | --- |
| `Product` | `packages/Cargo.toml` | `packages/Cargo.lock` | `bazel/cargo/product.lock` |
| `Walker` | walker manifest | walker lock | `bazel/cargo/walker.lock` |

The accepted identifier set is exactly `product`, `walker`.

`main`, `broker`, and `guest` are retired inputs, represented only by the
Retired Hub Refusal entity. They cannot be hub variants or aliases.

Every hub sets `lockfile`, `cargo_lockfile`, and
`skip_cargo_lockfile_overwrite = True`.

## Retired Hub Refusal

| Field | Rule |
| --- | --- |
| `retired_hub` | Exactly `main`, `broker`, or `guest`. |
| `diagnostic` | Exact ADR 0054 line for that hub. |
| `remediation_argv` | `["cargo", "xtask", "bazel-repin", "--hub", "product"]`. |
| `remediation_cwd` | Repository-relative `packages/`. |
| `executor` | Injected non-mutating executor. |

The refusal occurs before any Bazel child is started. A command containing
`cd packages` or a `packages/` argv prefix is invalid because cwd is already
`packages/`. No final-newline field exists.

## Cargo Build Context

The command shape is the variant. Package, lock, feature, exclusion, and
target-directory members exist only on variants that use them.

Variants:

| Variant | Selector | Lock and target rule | Topology |
| --- | --- | --- | --- |
| `GuestFormat` | `cargo fmt -p d2b-guest-shell-runner --check` | No `--locked`, feature selector, or target directory. | Package-only formatting. |
| `MainClippy` | locked workspace excluding broker and guest | Product lock; gate-owned main target. | Clippy all targets; includes contract crate. |
| `MainTests` | locked workspace excluding contract, broker, and guest | Product lock; gate-owned main target. | Process per case plus companions. |
| `BrokerDefault` | locked broker, no default features, empty features | Product lock; `packages/d2b-priv-broker/target`. | Process per test binary. |
| `BrokerLayer1` | locked broker, no default features, `layer1-bootstrap` | Product lock; `packages/d2b-priv-broker/target-layer1`. | Process per test binary. |
| `BrokerFake` | locked broker, no default features, `fake-backends` | Product lock; `packages/d2b-priv-broker/target-fakebackends`. | Process per test binary. |
| `GuestProduction` | locked guest, no default features, `real-libshpool` | Product lock; `packages/d2b-guest-shell-runner/target`. | Process per case plus companions. |

Broker contexts are serialized and use distinct target directories. Generic
main companion discovery uses the `MainTests` exclusions exactly.

## Configured First-Party Bazel Target

| Field | Rule |
| --- | --- |
| `label` | Native repository label, never a generated external first-party crate. |
| `cargo_context` | Exactly one Cargo Build Context. |
| `direct_first_party_deps` | Exact generated set. |
| `direct_product_deps` | Exact generated `@product` labels. |
| `cfgs` | Exact generated values. |
| `features` | Exact generated first-party feature set. |
| `closure_census` | Exact nonempty configured target and external identity set. |

`@product` may be a third-party package and feature superset. The configured
native target defines actual first-party edges and features.

## Selected Context Oracle

The oracle is a three-way join. Each source supplies only the columns it owns.

| Field | Source | Rule |
| --- | --- | --- |
| `metadata_command` | metadata | `cargo metadata --locked --offline --format-version 1 --filter-platform <target>` from `packages/`. |
| `identities` | metadata | Package identity, name, version, and `source` for every node. |
| `candidate_edges` | metadata | `resolve.nodes[].deps[].dep_kinds`, giving dependency kind and `cfg` predicate. |
| `checksums` | lock and pin | Registry `checksum` from `packages/Cargo.lock`; git `rev` and archive hash from the committed pin. Metadata carries no checksum field. |
| `tree_command` | tree | `cargo tree --locked --offline --manifest-path Cargo.toml -p <package> --target <target> --no-default-features --features <exact list> --edges <kinds> --charset ascii --prefix depth --no-dedupe --format '\|{p}\|{f}\|'`. |
| `root` | tree | Exactly one broker or guest package selection; metadata `resolve.root` is null for a workspace and is never used. |
| `target` | metadata and tree | Exact native GNU or musl target for the variant, identical in both. |
| `edge_kinds` | tree | Separate production (`normal,build`) and dev-inclusive (`normal,build,dev`) traversals; never post-filtered from one another. |
| `default_features` | tree | Explicitly disabled with `--no-default-features`; never inferred. |
| `features` | tree | Exact sorted requested set and the resolved `{f}` column. |
| `cross_check` | all three | Every traversal row's identity exists in metadata, its edge kind and `cfg` agree with `dep_kinds`, and every non-path identity has a lock-supplied checksum. |
| `parser_input` | tree | Pinned charset, depth prefix, no-dedupe, and repository-pinned delimited format; `--prefix depth` prints the depth with no separator, so the format begins with a delimiter. |
| `synthetic_input` | - | Absent: no synthetic manifest or splice exists. |

The feature canary is a required invalid variant: an unrelated workspace member
enables an otherwise-absent feature on a dependency shared with broker or
guest. The whole-workspace union may contain that feature, but it must not
appear in the `{f}` column of the broker or guest selected traversal. Generic
Cargo/Nix build/test and Clippy variants exclude broker and guest exactly;
dedicated variants retain exact selection.

Only `BrokerDefault`, `BrokerLayer1`, `BrokerFake`, and `GuestProduction`
carry this oracle. `GuestFormat` resolves no dependency graph, and
`MainClippy` and `MainTests` are multi-root workspace selections enforced by
their exact exclusion census rather than by a single-root oracle.

## Libshpool Dependency Contract

| Field | Rule |
| --- | --- |
| `dependency` | Normal `libshpool = "0.11.0"`. |
| `feature` | `real-libshpool = []`. |
| `code_activation` | Existing `cfg(feature = "real-libshpool")`. |
| `crate_spec_sites` | Exact empty set. |

## Package Policy Context

Common:

| Field | Rule |
| --- | --- |
| `nix_system` | `x86_64-linux` or `aarch64-linux`. |
| `cargo_target` | Exact matching GNU or musl target. |
| `root_package` | Broker or guest shell runner. |
| `default_features` | Disabled. |
| `features` | Empty for broker; `real-libshpool` for guest. |
| `root_lock` | `packages/Cargo.lock`. |

Variants:

| Variant | Target class |
| --- | --- |
| `BrokerProduction` | matching GNU target |
| `GuestRealLibshpool` | matching musl target |

There are exactly four system-and-target contexts.

## Package Policy Graph

Common:

| Field | Rule |
| --- | --- |
| `selected_root` | Exactly one package identity. |
| `system` | Matches parent Package Policy Context. |
| `target` | Matches parent context. |
| `nodes` | Exact nonempty sorted package set. |
| `edges` | Exact sorted set with authoritative `normal`, `build`, or `dev` kind and cfg. |
| `features` | Exact resolved feature set per node. |
| `filtered_lock` | Contains exactly reached external package identities. |
| `selected_sources` | One exact Selected Source Census. |

Variants:

- `ProductionGraph`: selected normal and build closure only.
- `PolicyGraph`: production graph plus every root dev edge and the complete
  transitive normal and build closure reached from each dev package.

Generated paths:

```text
packages/policy-inputs/<system>/<target>/<context>/production/closure.json
packages/policy-inputs/<system>/<target>/<context>/production/Cargo.lock
packages/policy-inputs/<system>/<target>/<context>/policy/metadata.json
packages/policy-inputs/<system>/<target>/<context>/policy/Cargo.lock
```

## Selected Source Census

| Field | Rule |
| --- | --- |
| `identities` | Exact sorted non-path `(name, version, source)` set from metadata. |
| `lock_identities` | Exact sorted non-path identity set from filtered lock. |
| `count` | Derived length of `identities`, positive. |
| `registry_sources` | URL plus Cargo checksum for every registry identity. |
| `git_sources` | URL, pinned rev, and committed archive checksum for every git identity. |
| `materialized_sources` | Exact readable set, no missing or extra item. |

Validity requires identity equality between metadata and filtered lock, exact
count equality after materialization, readability, and checksum verification
before policy execution.

## Package Policy Result

Variants:

| Variant | Inputs | Invariants |
| --- | --- | --- |
| `DependencyPolicy` | ProductionGraph | closure minimality and forbidden production classes |
| `PackageDeny` | PolicyGraph and sources | bans, licenses, sources, no `--exclude-dev` |
| `PackageAudit` | PolicyGraph filtered lock and pinned RustSec DB | `--no-fetch`; exact context ignore set |

Broker audit has no ignore. Guest audit has exactly
`RUSTSEC-2024-0384`.

## Guest License Exception

| Field | Rule |
| --- | --- |
| `package` | One of `bindgen`, `instant`, `inotify`, `inotify-sys`, `libloading`, `notify`. |
| `license` | Exact package-paired license from ADR 0054. |
| `scope` | Guest real-libshpool policy only. |

The set has exactly six entries:

```text
bindgen        BSD-3-Clause
instant        BSD-3-Clause
inotify        ISC
inotify-sys    ISC
libloading     ISC
notify         CC0-1.0
```

There is no global-license-allow field. A different package with one of these
licenses remains denied.

## Nix Artifact Context

Variants:

| Variant | Package target | Linkage |
| --- | --- | --- |
| `BrokerHost` | matching GNU | host dynamic artifact |
| `GuestStatic` | matching musl through `pkgsStatic` | static PIE |

Both consume root product source and lock and explicit package and binary
selectors. Guest additionally selects `real-libshpool`.

Each context retains its dedicated derivation, selected dependency policy,
binary-size evidence, and closure evidence. Both carry exactly:

```text
cargoLock.outputHashes."wl-proxy-0.1.2" =
  "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8="
```

Every `GuestStatic` result additionally binds:

| Field | Rule |
| --- | --- |
| `elf_type` | Exactly `ET_DYN`; `ET_EXEC` is rejected as non-PIE. |
| `elf_machine` | `EM_X86_64` on `x86_64-linux`; `EM_AARCH64` on `aarch64-linux`. |
| `program_interpreter` | Absent. |
| `needed_entries` | Empty. |

Non-PIE and wrong-machine artifacts are required invalid variants.

## Native Artifact Baseline

Exactly four rows exist: broker and guest for each native system. A row is
generated only from a realized derivation.

| Field | Rule |
| --- | --- |
| `binaryBytes` | Measured executable size. |
| `elfType`, `elfMachine` | Exact realized values. |
| `interpreter`, `needed` | Exact broker interpreter and sorted SONAMEs, or absent and empty for guest. |
| `closureCount`, `closureSha256` | Count and digest derived transiently from the exact sorted recursive Nix closure; no store path is persisted. |
| `selectedPolicyDigest` | Exact selected package-policy graph digest. |
| `measurementCommand`, `candidateCommit` | Immutable measurement provenance. |
| `sizeGrowthAuthorization` | The only allowance source: null for unchanged or smaller size, otherwise the closed approved object below carrying `priorBinaryBytes == binaryBytes`, `newBinaryBytes == actual realized bytes`, positive exact `deltaBytes`, normalized `rationalePath`, candidate/review digests, matching system/artifact, and `decision = "approved"`. |

The size predicate is `actual <= binaryBytes` when authorization is null and
`actual <= binaryBytes + sizeGrowthAuthorization.deltaBytes` otherwise. No
row-level allowance field exists. Authorization is valid only when
`priorBinaryBytes` equals the row baseline, `newBinaryBytes` equals the
realized artifact measurement, those values have the exact positive
`deltaBytes` difference, the rationale is repository-relative, both digests
bind the candidate and review, the system/artifact matches, and the decision is
approved in the same change. No prose byte ceiling exists. The positive
fixtures are unchanged size without authorization and exact authorized growth.
Missing, denied, stale, replayed, wrong-system/artifact, wrong-prior,
wrong-realized-new, absolute-rationale, arithmetic-mismatch, duplicate
allowance-source, and size-plus-one authorizations are invalid.

## Flake Check Wrapper

For each root flake system:

```text
broker-production-dependency-policy
guest-shell-runner-static-dependency-policy
broker-production-package-policy
guest-real-libshpool-package-policy
broker-host-artifact-contract
guest-static-elf
```

| Field | Rule |
| --- | --- |
| `system` | Exact native system. |
| `runner` | Matching native runner architecture. |
| `policy_input` | Exact Package Policy Context path. |
| `foreign_system_args` | Empty. |
| `builder_args` | Empty. |
| `realized` | Required. |

The authoritative wrapper set has exactly twelve entries: six checks per
system. Eight are package-policy wrappers and four are artifact-baseline
wrappers.

## Rust Surface

The existing fixed set remains:

```text
rust-api-surface
rust-main-format
rust-main-clippy
rust-main-workspace-tests
rust-no-bash-ast
rust-schema-reproducibility
rust-stub-no-socket
rust-assert-pinned
rust-broker-default
rust-broker-layer1
rust-broker-fakebackends
rust-guest-shell-runner
rust-deny-main
rust-deny-broker
rust-deny-guest
rust-audit-main
rust-audit-broker
rust-audit-guest
```

Each surface has a nonempty carrier set, one verdict owner, one of four slices,
and an exact generated census where applicable. Fixture-backed IDs are not
members.

## Carrier Target and Coverage Map

A carrier has:

- one Rust Surface;
- one native Bazel label;
- closed declared inputs and outputs;
- one configured first-party context where applicable;
- one topology where applicable;
- exact test or policy census;
- every hand-written fragment it consumes;
- declared runfiles and provider identities.

The mapping is total and unambiguous, not one-to-one. Label existence is proved
at analysis time. Graph completeness and query drift are proved outside Bazel
tests. No Bazel test invokes `bazel query`.

## Test Topology

Variants remain:

- `ProcessPerCase` for main and guest;
- `ProcessPerBinary` for each broker feature context;
- `Doctest`;
- `HarnessFree`.

The existing per-case result, verified executable handle, injected filesystem,
runfiles, clock, and index boundary models remain as decided by ADR 0052.
Changing the product workspace does not weaken them.

Every `ProcessPerBinary` broker variant carries exactly
`tags = ["exclusive"]`, cannot overlap any other test, and has a qualification
count of twenty consecutive executions for its own context.

## Verified Executable Handle

| Field | Rule |
| --- | --- |
| `anchor` | Close-on-exec runfiles or Cargo provider-root descriptor. |
| `relative` | Nonempty declared relative path with no absolute or `..` component. |
| `descriptor` | One `O_RDONLY|O_CLOEXEC` open using `RESOLVE_NO_MAGICLINKS` only, deliberately without `RESOLVE_BENEATH` or `RESOLVE_NO_SYMLINKS`. |
| `fallback` | Forced component walk: intermediate `O_DIRECTORY|O_NOFOLLOW|O_CLOEXEC`; declared leaf symlink permitted; leaf opened `O_RDONLY|O_CLOEXEC` without `O_NOFOLLOW`. |
| `identity` | Kind, executable mode, freshness, byte digest, and matching pre/post descriptor metadata. |
| `execution` | The sole safe Rust consuming API maps the verified open file description to a private fd; immutable static C `d2b-bazel-exec-supervisor` forks once, proves exec through a close-on-exec error pipe, remains alive, and supervises `execveat(private_fd, "", argv, envp, AT_EMPTY_PATH)`. |
| `enosys` | Named refusal requiring a kernel with `execveat`; no fallback. |
| `auxiliary_descriptors` | All close-on-exec and proven by child descriptor-table behavior. |
| `api_seal` | Private fields and minting trait; empty public inherent API and empty locally-authored explicit trait-impl allowlists; exact compiler-derived public, hidden, auto, and blanket API snapshots; no descriptor/path accessor or extraction, `Deref`, descriptor `Borrow`, `AsFd`, raw-fd trait, formatting, serialization, conversion, default, or duplication API. |
| `owner` | One dependency-leaf crate owns the type and the only public API that consumes it. |
| `execution_transfer` | The public API consumes the handle by value and uses the exact pinned reviewed safe `command-fds` mapping dependency to install the verified description at a fixed private fd while leaving declared stdin/stdout/stderr unchanged. |
| `helper_identity` | Exact immutable dedicated Nix store artifact, bound by C source, derivation dependency closure, output NAR, executable, protocol, and native-system hashes; runfiles, worktree, copied, symlinked, missing, and wrong outputs refuse. |
| `helper` | One tiny statically linked single-threaded C supervisor built outside the product Rust workspace. It creates the close-on-exec exec-error pipe, forks exactly once, and owns target supervision; no Rust unsafe exception exists. |
| `spawn_owner` | Safe Rust `std::process` and `command-fds` spawn and wait-own the supervisor. The C supervisor owns the only fork, signal normalization/forwarding, target wait, and reap. Rust never signals a numeric PID/PGID. The patched sandbox PID-1 monitor owns abnormal teardown. No Rust `pre_exec`, raw fork, or signal-handler path exists. |
| `invocation_policy` | Closed source/call-site census permits only the typed consumer to invoke the exact helper and rejects every other Rust, Bazel, Make, workflow, runfiles, or worktree invocation. |

There is no path accessor or public unchecked constructor. No
runfiles/worktree/copied helper path, direct helper invocation outside the
typed consumer, fd-0 executable transport, target path, `fexecve`,
`/proc/self/fd`, reopen, or post-`ENOSYS` fallback exists. Successful exec
preserves declared stdio and leaks no provider, private executable,
status-pipe, or auxiliary descriptor.

## Immutable Supervisor Launch

| Stage | Rust parent ownership, transition, and failure |
| --- | --- |
| `Verified` | Consumed `VerifiedExecutable` exclusively owns the provider `OwnedFd`; failure drops it once. |
| `HelperIdentity` | Exact immutable Nix path plus C source, derivation dependency, output NAR, executable, protocol, and native-system hashes validate before spawn; missing/wrong/rebound output closes the provider and creates no child. |
| `Mapped` | Parent owns the provider fd, protocol reader/writer ends, declared stdio, and mapping configuration. The pinned safe mapper uses fixed private fds outside 0/1/2; collision or preparation failure closes every private end and the provider without changing stdio. |
| `Spawned` | `std::process::Child` becomes the sole supervisor wait owner. Parent immediately closes its mapped provider and supervisor-side protocol copies; spawn failure creates no child and RAII closes every fd. |
| `Ready` | Parent's stateful framed decoder accepts exactly `D2BS`, version 1, type `READY`, and zero payload. Fragmented input is retained; coalesced later frames remain buffered. EOF, helper exit, timeout, malformed header, wrong length, overflow, duplicate, or out-of-order status is typed failure independent of process status. |
| `Executed` | The same decoder accepts version-1 zero-length `EXECUTED` only after `READY`. A fast target exit remains distinguishable because a coalesced terminal stays buffered; helper crash or EOF before this frame is never target status. |
| `Terminal` | Decoder accepts one version-1 `EXITED` or `SIGNALED` frame with its exact one-byte bounded payload, rejects any retained or later trailing frame or byte, drains to EOF, then waits for the supervisor and requires exact status equality. No status-stream one-byte overlong probe exists. |
| `Cleaned` | The successful path closes remaining protocol fds once and reaps the supervisor. A post-spawn failure closes owned fds and returns the Bazel action nonzero without signaling a numeric PID/PGID; the sandbox owns survivors. Injected close/read/wait failures preserve the first typed cause plus cleanup stage without raw OS text. |

| Stage | C supervisor ownership, transition, and failure |
| --- | --- |
| `Adopted` | Supervisor exclusively owns the mapped executable fd, declared stdio, and Rust status writer; wrong identity or absent descriptor emits a typed failure and closes all owned fds. |
| `Normalized` | The single thread first blocks the complete managed set. While blocked it installs default dispositions, ignored `SIGPIPE`, waitable default `SIGCHLD`, and fixed synchronous consumption, then establishes the final mask. Pending or normalization-time `SIGTERM` is consumed into the supervisor-owned pre-`READY` termination path. The child restores an empty mask and every catchable disposition to default; normalization failure emits a typed failure before fork. |
| `ExecPipe` | Supervisor creates exactly one `O_CLOEXEC|O_NONBLOCK` exec-error pipe and owns both ends; pipe failure emits a typed failure and forks no child. |
| `Forked` | Exactly one fork creates the target child. Supervisor closes the writer, owns the reader and child pid, emits `READY`, and must kill and reap on every later failure. |
| `ChildSetup` | Child establishes the target group, resets mask/dispositions, installs 0/1/2, sets the executable fd CLOEXEC, and closes supervisor-only fds. A stage failure writes one fixed exec-error record under the absolute deadline and `_exit`s. |
| `ExecResult` | Supervisor reads exact empty EOF as exec success or one complete fixed error record using exact `EINTR`/`EAGAIN`/short/partial/overlong loops under one original absolute deadline. The single-record exec-error reader alone uses one additional overlong byte. Status uses the separate framed stream and fixed bounded writer. Closed-reader `EPIPE`, held-open writer, timeout, or unknown data is typed failure; only empty exec-error EOF emits framed `EXECUTED`. |
| `Supervising` | After `EXECUTED`, supervisor remains alive, forwards only `SIGHUP`, `SIGINT`, `SIGTERM`, and `SIGQUIT` to the target group, and applies the complete fixed TERM/grace/unconditional-KILL policy on case expiry or external `SIGTERM`, including with no case deadline. |
| `Reaped` | Supervisor waits and reaps the direct target, emits framed `EXITED` or `SIGNALED`, closes the Rust status writer, and mirrors the exact target normal exit or terminating signal. A mirror, signal, wait, or reap failure is typed and cannot be reported as target status. |
| `Closed` | Every non-exec and post-exec path closes each owned fd once and reaps every created child. The first operation failure and any cleanup failure retain distinct fixed stages. |

| Stage | Patched sandbox ownership, transition, and failure |
| --- | --- |
| `NamespaceCreated` | Outer `linux-sandbox` owns one fresh `CLONE_NEWPID` monitor and synchronization pipes; failure reaps any created monitor and execs no action. |
| `MonitorReady` | Namespace PID 1 remains outside the action command tree, adopts every orphan, owns abnormal namespace kill/reap, and is wait-owned by outer `linux-sandbox`. |
| `ActionRunning` | The supervisor owns normal target TERM/grace/KILL/reap. Abnormal setup/action exit, including parent or supervisor crash, transitions once to `Aborting`. |
| `Aborting` | PID 1 namespace-kills every other member and makes nonblocking reap progress. One fixed 10,000 ms ceiling bounds userspace TERM/KILL/monitor escalation and the close-or-quarantine decision only. Kill, reap, and ceiling failures are distinct typed stages. |
| `PendingKernelCleanup` | If a consuming wait has not proved namespace members and PID 1 reaped at the userspace ceiling, outer `linux-sandbox` remains the wait owner and records `pending-kernel-cleanup`. Sandbox and outputs are quarantined; success and reuse are prohibited while nonblocking observation continues. |
| `Closed` | A consuming wait proved PID 1 reaped. Cleanup is `complete` or `complete-after-quarantine`; a quarantined action remains failed. No host PID, PID file, cgroup, or host process group is a fallback. |

The public API census is primary. Focused rustdoc `compile_fail` examples prove
downstream construction, descriptor access/extraction, trait coercion,
formatting/serialization, duplication/conversion, and mint-trait absence.
Cargo-shelling compile fixtures are not part of this entity.

Strict result, execution-manifest, JUnit-parent, and cleanup entities are a
different path variant and retain
`RESOLVE_BENEATH|RESOLVE_NO_SYMLINKS|RESOLVE_NO_MAGICLINKS`. Reintroducing
`RESOLVE_BENEATH` on a provider open is a required invalid mutation.

## PID-Namespace Containment Qualification Result

Qualification carries exactly one bounded result for each closed stage:

1. `crash-before-ready`;
2. `crash-after-ready`;
3. `crash-after-executed`;
4. `crash-during-grace`;
5. `direct-long-lived-descendant`;
6. `double-forked-long-lived-descendant`;
7. `beyond-ceiling-pending-cleanup`.

| Field | Rule |
| --- | --- |
| `stage` | One value from the seven-entry closed census above, occurring exactly once. |
| `supervisorRecoveryClass` | Closed `not-yet-ready`, `ready-not-executed`, `executed`, `grace-active`, `descendant-only`, or `pending-kernel-cleanup`; each stage has one permitted class. |
| `userspaceEscalationResult` | Closed `kill-monitor-complete` or `ceiling-entered-quarantine`. The fixed ceiling says nothing about kernel task exit. |
| `cleanupResult` | Closed `complete` or `complete-after-quarantine`; neither may be recorded until a consuming wait proves PID 1 reaped. |
| `quarantineResult` | Closed `not-entered` or `entered-and-released-after-consuming-reap`. The beyond-ceiling stage must use the latter and bind its prior pending observation. |
| `sandboxPatchSha256` | Exactly 64 lowercase hexadecimal characters matching the reviewed patch identity. |
| `sandboxMonitorIdentitySha256` | Exactly 64 lowercase hexadecimal characters over the canonical monitor source, executable, protocol-version, and patch-digest tuple; no free-form or opaque monitor identity exists. |
| `pendingObservationSha256` | `none` for ordinary stages; otherwise exactly 64 lowercase hexadecimal characters binding the typed pending transition, owned wait state, quarantine entry, no-success/no-reuse results, and absence of a reaped claim. |
| `resultSha256` | Exactly 64 lowercase hexadecimal characters over the closed fields and referenced plant verdict. |

The permitted tuples are exact:

| Stage | `supervisorRecoveryClass` | `userspaceEscalationResult` | `cleanupResult` | `quarantineResult` |
| --- | --- | --- | --- | --- |
| `crash-before-ready` | `not-yet-ready` | `kill-monitor-complete` | `complete` | `not-entered` |
| `crash-after-ready` | `ready-not-executed` | `kill-monitor-complete` | `complete` | `not-entered` |
| `crash-after-executed` | `executed` | `kill-monitor-complete` | `complete` | `not-entered` |
| `crash-during-grace` | `grace-active` | `kill-monitor-complete` | `complete` | `not-entered` |
| `direct-long-lived-descendant` | `descendant-only` | `kill-monitor-complete` | `complete` | `not-entered` |
| `double-forked-long-lived-descendant` | `descendant-only` | `kill-monitor-complete` | `complete` | `not-entered` |
| `beyond-ceiling-pending-cleanup` | `pending-kernel-cleanup` | `ceiling-entered-quarantine` | `complete-after-quarantine` | `entered-and-released-after-consuming-reap` |

Containment evidence contains no raw PID, process-group ID, descriptor, path,
process output, kernel text, command line, environment value, handle, or
opaque identity. The record carries only the closed fields above and digests.
The containment validator requires all seven results and passing results for
each mutation class: omitted stage, duplicate stage, unknown stage, wrong
supervisor recovery class, malformed digest, patch digest mismatch, monitor
digest mismatch, illegal cleanup/quarantine combination, false PID-1 reaped
claim, success after quarantine, resource reuse while quarantined, and every
forbidden raw or opaque field. Qualification cannot omit or summarize these
mutation results.

## Process Escalation

| Field | Rule |
| --- | --- |
| `group` | Dedicated child process group. |
| `grace` | Independently timed fixed interval, always elapsed in full. |
| `observations` | Repeated `waitid(EXITED|NOWAIT|NOHANG)` polls throughout grace. |
| `observation_effect` | Informational only; never consuming, blocking, shortening grace, or authorizing reap. |
| `escalation` | Unconditional group SIGKILL at grace expiry. |
| `reap` | Direct child only, after SIGKILL. |

Blocking-wait, early-reap, shortened-grace, and conditional-SIGKILL mutations
are required invalid variants.

## Module Lock Refresh

| Field | Rule |
| --- | --- |
| `command` | `cargo xtask bazel-module-refresh`, no arguments. |
| `child` | `bazel mod deps --lockfile_mode=update`. |
| `startup_options` | Absolute and byte-identical to other server-selecting commands. |
| `allowed_mutation` | Exactly `MODULE.bazel.lock`. |
| `idempotent` | Second run on current state changes nothing and exits zero. |
| `reachability` | Contributor shell only; absent from Make and workflows. |
| `remediation` | Exact repository line naming this command and only this command. |
| `refresh_order` | Always last. A product manifest change runs product lock, product hub, then this; a walker manifest or lock change runs walker lock, walker hub, then this; initial or combined setup runs product hub, walker hub, then this. |
| `byte_identity_proof` | The untouched hub's Cargo and Bazel inputs are proved byte-identical across the refresh. |

## Product Yanked Snapshot

| Field | Rule |
| --- | --- |
| `path` | `bazel/supply_chain/yanked-snapshot.json`. |
| `authority` | Exact sorted `(name, version)` key set from `packages/Cargo.lock` only. |
| `excluded_locks` | Walker lock and `packages/Cargo.guest.lock`. |
| `main_projection` | Full product snapshot. |
| `broker_projection` | Exact keys from the broker root-dev-inclusive package-policy graph. |
| `guest_projection` | Exact keys from the guest root-dev-inclusive package-policy graph. |
| `refresh` | Reviewed networked `cargo xtask bazel-yanked-refresh`; snapshot-only mutation. |
| `check` | Offline no-write `cargo xtask bazel-yanked-check`; no network client construction. |

## Supply Chain Equivalence Result

One result exists for each of `main`, `broker`, and `guest`.

| Field | Rule |
| --- | --- |
| `context` | Main full product, broker selected projection, or guest selected projection. |
| `cargo_exit_status` | Raw enforcing status from current `cargo deny check`. |
| `bazel_union_exit_status` | Cargo-compatible status for decomposed deny, audit, and yanked union. |
| `cargo_findings` | Sorted normalized finding keys. |
| `bazel_union_findings` | Sorted normalized union from all three decomposed carriers. |
| `equal` | Derived exact status and set equality. |

A finding key is
`(class, package, version, source, finding_id, detail)`. Operational errors
are separate invalid variants, not policy findings. A false `equal` blocks spec003w1,
qualification, and promotion.

## Action Network Evidence

| Field | Rule |
| --- | --- |
| `sandboxProvider` | Exact Nix-pinned Bazel 8.6.0 upstream source, Linux sandbox patch, fixed-policy, output NAR, executable, and capability-ABI hashes. |
| `actionKinds` | Exact stable/nightly Rustc, metadata, Clippy, rustdoc, doctest compile/run, rustfmt, unpretty, build-script, repository, setup, and test coverage from configured-target and `aquery` inventories. |
| `strategyInventory` | Every governed action uses the patched Linux `sandboxed` strategy; process, local, standalone, worker, remote, and every fallback are absent. |
| `loadPoint` | Sandbox child verifies and loads the fixed filter after sandbox construction and before exec of the full action command, covering compile/build commands, test setup, tests, and descendants. |
| `startupProbe` | The exact Nix output reports the fixed capability ABI and denies a planted syscall before any server or governed action starts. |
| `inheritedCapabilities` | Complete pre-filter descriptor census rejects sockets and every io_uring ring, including SQPOLL and registered/fixed-socket states. |
| `syscalls` | Closed denied socket-operation, `pidfd_getfd`, `socketcall` when present, and three-io_uring-entry-point set. |
| `plants` | Patch-removal, wrong-output, filter-load, strategy fallback, inherited socket/ring/SQPOLL/fixed-socket, setup-before-payload, compile/build, test, descendant, and exact eight IPv4, IPv6, netlink, packet, pathname Unix, abstract Unix, socketpair, and io_uring pre-action results. |
| `external_egress_plant` | A build/test action attempts host/external egress and is denied. |
| `live_index_plant` | The offline yanked validator receives a live-index source and refuses before resolution or socket use. |
| `repository_fetch_inventory` | Exact fetch sites outside governed actions, offline during gates, each pinned by lock checksum or git revision plus archive sha256. |
| `cargo_compatibility_carriers` | Exact generated test identities, Cargo selectors, existing surface IDs, same-commit verdicts, and non-advisory classification for mandatory socket users. |
| `containmentQualification` | Exact seven-stage bounded containment result set with closed recovery/escalation/cleanup/quarantine values, patch/monitor/pending/result digests, forbidden-field absence, and every validator mutation result. |
| `qualification_result` | Identity, startup, strategy, inherited-capability, setup-before-payload, all eight socket/io_uring, external-egress, and live-index plants fail at their own predicates; every sandbox-policy stage has its fixed redacted code/remedy; inventories are complete; and every compatibility carrier passes on the same head. |

There is no endpoint declaration field. A network namespace cannot enforce
one, and no such claim is part of qualification.

## Qualification Record

Each record remains a protected `v3` push produced by a merged pull request and
contains:

- one head commit shared by Cargo and Bazel runs;
- Cargo, Bazel, and fixture run references, each binding immutable run ID,
  positive attempt, head SHA, and terminal verdict;
- a passing same-commit fixture verdict;
- four Bazel slice verdicts;
- explicit `bazelRestoreCount`, `bazelSaveCount`, and
  `bazelPublicationCount`;
- `sliceDurationsSeconds` with four complete durations in every cold record;
- effective workflow permissions.

Those four camelCase names are the canonical spellings. All three counts are
mandatory in every record and zero during shadow. Every cold record also
requires `bazelRestoreCount` of zero and four `sliceDurationsSeconds`
entries. A missing field is a refusal, never an implied zero.

A pull-request shadow run is not a Qualification Record variant. It executes
zero cache actions and emits no qualification object. Only a protected-`v3`
push record carries the explicit zero counts and four durations.

`qualified` is derived by the Typed Qualification Validator below. Boolean and
summary fields in the record are informational mirrors; a mirror that disagrees
with the derived result is a refusal.

The validator receives complete paginated Cargo, Bazel, and fixture run
inventories. It rejects page gaps, missing attempts, duplicate/conflicting run
identities, or omitted intervening protected-`v3` pushes; normalizes a run ID
to its highest terminal attempt; derives same-head pairing and streak resets;
and selects the five newest qualifying cold records from that complete stream.

Qualification additionally binds:

- product and walker hub generation;
- all four Package Policy Contexts;
- exactly twelve native check wrappers, six per system;
- native x86_64 and aarch64 six-check realization sets;
- the selected-source and checksum refusal matrix;
- the narrow six-entry guest license policy.
- module-refresh mutation and remediation evidence;
- exact `wl-proxy` output-hash evidence for the broker and guest Nix
  derivations;
- broker `exclusive` tags, no-overlap mutation, and twenty-run result for each
  context;
- exact patched-Bazel source/patch/policy/output/executable/capability
  identities, startup probe, configured-target plus `aquery` stable/nightly
  action-kind inventory, strategy inventory, patch-removal/filter-load/
  setup-before-payload/inherited-capability/fallback results, eight pre-action
  socket/io_uring plants, external-egress and live-index plants, and exact
  Cargo compatibility census;
- all seven PID-namespace containment results, the exact sandbox patch and
  canonical monitor identity digests, closed supervisor recovery classes,
  cleanup/quarantine results, pending observation, and every
  containment-validator mutation result;
- product-only yanked authority and exact broker/guest projections;
- all three Supply Chain Equivalence Results;
- native arm `make test-rust-supply-chain` and stable-head renderer evidence.

## Cache Generation

| Field | Rule |
| --- | --- |
| `primary_key` | Unique per successful protected-`v3` run and cache kind; includes run ID. |
| `restore_prefix` | Omits run ID and commit SHA. |
| `kind` | `action` or `repository`, never output base. |
| `bazelRestoreCount` | Explicit nonnegative count; present in every record. |
| `bazelSaveCount` | Explicit nonnegative count; present in every record. |
| `bazelPublicationCount` | Explicit nonnegative count; present in every record. |
| `sliceDurationsSeconds` | Exactly four complete durations in every cold record. |
| `retention` | Keep the newest complete generation for each authorized prefix; delete only older authorized generations. |

The bound-input applicability table in
`contracts/cache-workflow-boundaries.md` is part of this entity. Every marked
action or repository dependency is mutation-tested, and `kind` is embedded in
the namespace so action and repository keys can never collapse.

## Post-Promotion Run Unit

A transient run unit is one distinct push-created `(runId, headSha)` pair. It
is the only streak-bearing entity; an attempt never is. The validator fetches
the complete stream on every run before deriving the streak.

| Field | Rule |
| --- | --- |
| `runId` | Required immutable workflow run ID. |
| `headSha` | Exact tested commit; `(runId, headSha)` is the unit identity. |
| `event` | Exactly `push`. |
| `branch` | Exactly `v3`. |
| `attempts` | Complete nested history `1..maxAttempt`; a missing attempt is invalid. |
| `conclusion` | Normalized to the conclusion of the highest terminal attempt. |
| `createdAt` | Immutable creation timestamp; primary ordering key. |
| `promotionAncestor` | Derived true only when promotion is an ancestor of `headSha`. |

### Run Attempt

| Field | Rule |
| --- | --- |
| `attempt` | Positive integer, unique inside its unit. |
| `conclusion` | Terminal or nonterminal conclusion of that attempt. |
| `runStartedAt` | Orders attempts inside a unit only; never orders units. |
| `completedAt` | Terminal timestamp not earlier than that attempt's start. |
| `headSha`, `event`, `branch`, provenance | Must equal the unit's values; any conflict is invalid. |

The source inventory carries page/cursor continuity and is complete before
derivation. Units sort by `(createdAt, runId)`. `runStartedAt` is never a
unit-ordering input, because a rerun updates it and would let an old rerun move
behind newer failures. Pagination gaps, missing attempts, conflicting attempt
provenance, missing or duplicate unit identities, a nonterminal highest
attempt, non-push/non-v3 records, and pre-promotion commits are invalid.

## Derived Promotion Streak

| Field | Rule |
| --- | --- |
| `ordered_units` | Complete validated Post-Promotion Run Unit set in `(createdAt, runId)` order. |
| `reset_positions` | Derived positions of every terminal non-success unit. |
| `current_successes` | Derived suffix length after the last reset, counting each unit once. |
| `retirement_eligible` | Derived true only when the final ten distinct ordered units are successes. |

No persisted `eligible`, count, or run-ID list is an input. A failure,
cancellation, timeout, or other terminal non-success between successes resets
the streak. A repeated successful attempt of an already-counted unit never
increments the streak, and a later rerun of an older unit never reorders it
behind newer failures.

## Bounded Post-Promotion Checkpoint

`post-promotion.json` persists no complete stream and no complete attempt
array.

| Field | Rule |
| --- | --- |
| `paginationState`, `pageCount`, `streamCount`, `streamSha256` | Closed `complete` state, counts, and digest of the complete transient stream; no raw cursor is persisted. |
| `promotionCommit` | Validated by the typed Promotion Record validator. |
| `lastTen` | At most the final ten normalized units, in immutable order. |
| `attemptCount`, `attemptHistorySha256` | Fixed-size summary per persisted unit. |
| `maxBytes`, `maxRecords` | Schema constants; overflow refuses before atomic replacement. |

The checkpoint is output only. Every refresh re-fetches the complete stream,
derives the verdict independently, replaces the bounded file, and proves the
bounded and complete-stream verdicts equal.

## Typed Qualification Validator

| Field | Rule |
| --- | --- |
| `module` | `packages/xtask/src/bazel_qualification.rs` with tests in `packages/xtask/tests/bazel_qualification.rs`, implemented no later than spec003w3. |
| `command` | `cargo xtask bazel-qualification-validate`, no arguments, fixed repository-relative record path, unreachable from Make and workflows. |
| `refreshCommand` | `cargo xtask bazel-evidence refresh-qualification`, no arguments, atomic fixed-record replacement, unreachable from Make and workflows. |
| `reference_kinds` | Workflow run (`runId`, positive `attempt`, and `headSha`), commit SHA, content path plus digest, generated path plus digest. |
| `derivation` | Every threshold is computed by counting or comparing referenced evidence; no stated number is trusted. |
| `outcomes` | Closed complete, typed degraded query/publication failure, or semantic refusal; query failure is never an empty inventory. |
| `refusals` | Inventory, omitted, forged or ill-formed, duplicate, inconsistent, wrong-candidate, and degraded-evidence classes. |
| `booleans` | Informational mirrors only; a mirror disagreeing with the derived result is a refusal. |
| `containment` | Derives the seven-stage census, stage-to-recovery-class mapping, patch/monitor digest equality, cleanup/quarantine legality, pending observation, no-success/no-reuse results, forbidden-field absence, and every required mutation result. |
| `callers` | Evidence curation, promotion validation, and contributor validation before any informational inspection. |

## No-Shell Spawn Inventory

| Field | Rule |
| --- | --- |
| `path` | `bazel/generated/no-shell-inventory.json`, generated, integrator-committed, drift-checked. |
| `governedSources` | Every repository-owned runner, cleanup, timeout, and process-control source derived from the first-party configured-target census. |
| `declaredInputs` | The exact declared inputs of the no-shell carrier. |
| `scanResults` | Exactly one successful scan record per governed source, including zero-site sources; raw record count and unique-source count each equal the governed-source count. |
| `spawnSites` | Every discovered spawn construct with its governed source, span, spawned program expression, and typed `shellInvocation` verdict; any true verdict refuses. |
| `nonempty` | `governedSources` and `declaredInputs` are nonempty. A source may validly record zero spawn sites. |
| `set_relationships` | Governed and declared source sets are equal; every spawn source is governed; scan-result sources equal governed sources; fresh and committed spawn-site keys are equal. |
| `plants` | Exactly `no-shell-inventory-empty`, `no-shell-inventory-missing-entry`, `no-shell-inventory-extra-entry`, `no-shell-inventory-unguarded-spawn`, `no-shell-inventory-missing-zero-site-record`, and `no-shell-inventory-planted-shell`. |

## Hybrid Disclosure Census

| Field | Rule |
| --- | --- |
| `source` | Exact sorted nonempty `cargoCompatibilityCarriers` entries from `tests/golden/bazel-rust-coverage.json`, each retaining surface ID, Cargo selector, test identity, and socket class; no surface-only projection. |
| `fixedDocuments` | Exactly `AGENTS.md`, `tests/AGENTS.md`, `docs/contributing/gates-and-lints.md`, `tests/README.md`, and `docs/reference/test-execution-manifest.md`. |
| `candidateFragments` | The promotion, alias-removal, and Cargo-retirement semantic fragments when present. |
| `comparison` | Every governed semantic block equals the complete carrier-identity source in both directions, with no duplicate or malformed entry; distinct cases sharing one surface remain distinct. |
| `enforcement` | Fixture-independent type-5 `policy_bazel_hybrid_docs.rs` under `make test-policy`, with isolated empty-census, missing, extra, malformed/duplicate block, malformed/duplicate identity, stale-attribution, and governed-document mismatch negatives. |

## Diagnostic Command Version

| Version | Valid repository state | Closed commands |
| --- | --- | --- |
| `bazel-diagnostic-v1` | Shadow through promoted aliases | `make test-bazel-rust`, `make test-bazel-rust-main`, `make test-bazel-rust-api`, `make test-bazel-rust-broker`, `make test-bazel-rust-aux` |
| `bazel-diagnostic-v2` | Alias removal and later | `make test-rust`, `make test-rust-slice-main`, `make test-rust-slice-api`, `make test-rust-slice-broker`, `make test-rust-slice-aux` |

Alias removal owns the only transition. It updates every production provider,
sandbox-policy, qualification-threshold, evidence/publication, cleanup, and
recovery renderer; both module-wiring roots; every byte-exact test; all
governed docs; the evidence record; and the semantic changelog in one change.
Version 1 survives only in the pre-change fixture with all shadow rules. A
state is invalid if any diagnostic, threshold, evidence variant, task-state
label, or document names a target absent from that state.

Execution recovery is a closed cross-product. Each Rust-parent, C-helper,
child-setup, and patched-sandbox cleanup stage maps to one stable public code,
one fixed repository-relative input, and one literal correction. The closed
slice plus the diagnostic version selects exactly one phase-valid command from
the table above. Missing, wrong-version, absent-in-phase, wrong-slice,
borrowed-remedy, free-form, numeric-PID/PGID, and unredacted variants are
invalid. T067 and T068 own only the runner parent/helper/child tests and
mapping. The patched sandbox and sequential T120 own sandbox mapping,
rendering, and live byte-exact tests.

## Evidence Sink Result

| Field | Rule |
| --- | --- |
| `testVerdict` | Underlying passed, failed, ignored, or interrupted result. |
| `evidenceStatus` | Closed tagged `Complete` or `Degraded` variant; never inferred from `testVerdict`. |
| `sinkKind` | Common field occurring exactly once: JUnit, `test.log`, execution evidence, qualification evidence, or exporter diagnostic. |
| `retentionClass` | Common field occurring exactly once and derived exactly from `sinkKind`. |
| `Complete` | Requires only `kind = "complete"` and `sinkPolicySha256`; rejects degradation fields and repeated sink/retention fields. |
| `Degraded` | Requires only `kind = "degraded"`, closed code, policy-row SHA-256, and closed retry command; rejects complete fields and repeated sink/retention fields. |
| `bytes`, `records` | At or below the committed measured sink-policy limits. |

Every forbidden planted value is absent from every sink. Qualification accepts
only the structurally valid complete variant but never rewrites the underlying
verdict. Execution-manifest v1 contains neither status variant.

## Promotion Record

| Field | Rule |
| --- | --- |
| `promotionCommit` | Actual protected-`v3` pull-request merge commit. |
| `sealedCandidateId`, `sealedContentId`, `sealedSnapshotSha256` | Exact identities from the `spec003w5` seal. |
| `pullRequestMergeCommit` | Immutable merge SHA from the merged PR record; equals `promotionCommit`. |
| `originV3Contains` | Derived ancestry result, never trusted. |
| `validation` | Typed re-derivation of seal/content/merge equality. |

A candidate head, older containing SHA, wrong seal, or unsealed merge is
invalid.

## Release Containment Result

| Field | Rule |
| --- | --- |
| `command` | No-argument `cargo xtask bazel-release-containment-validate`, unreachable from Make and workflows. |
| `tagReferenceSha256` | Digest of the validated semantic tag reference; the tag identifier remains transient and is never persisted or printed. |
| `containment` | Promotion commit is an ancestor of the peeled tag commit. |
| `origin` | Peeled local and origin tag commits agree. |
| `release` | Present, not draft, and not prerelease. |
| `outcome` | Closed `Complete`, typed query `Degraded`, or semantic `Refused`; query failure is never absence. |
| `diagnostic` | One fixed code and exact closed remedy; no tag/candidate/object/run/attempt identifier, raw output, OS text, path, cursor, descriptor, or handle. |

## Lifecycle

```text
planned
  -> product-foundation-ready
  -> coverage-complete
  -> safety-complete
  -> shadowing
  -> evidence-qualified
  -> promoted
  -> release-qualified -> aliases-removed
  -> green-run-qualified -> cargo-retired
```

`release-qualified` requires a containing published semantic release tag
matching `v<major>.<minor>.<patch>`. `green-run-qualified` requires ten
distinct ordered green post-promotion run units.

No lifecycle state is inherited from a parked Spec 003 foundation branch. The first transition
is reached only by merging and sealing the new `spec003w0` built from current
`v3`.

The two post-promotion eligibility clocks are independent. spec003w7
qualification and code preparation may run before spec003w6, but its shared
documentation/evidence task and merge depend on merged spec003w6. It then
rebases, revalidates, and obtains a new panel result. This encodes disjoint
ownership for every concurrently ready task.

## Relationships

```text
Product Workspace 1 -- 1 Product Hub
Walker Workspace 1 -- 1 Walker Hub
Product Workspace 1 -- many Cargo Build Contexts
Package-resolving Broker/Guest Cargo Build Context 1 -- 1 Selected Context Oracle
Cargo Build Context 1 -- many Configured First-Party Bazel Targets
Package Policy Context 1 -- 1 ProductionGraph
Package Policy Context 1 -- 1 PolicyGraph
Package Policy Graph 1 -- 1 Selected Source Census
Package Policy Context 1 -- many Package Policy Results
Nix Artifact Context 1 -- 1 Package Policy Context
Flake Check Wrapper 1 -- 1 exact system-and-target policy input
Coverage Map 1 -- 18 Rust Surfaces
Rust Surface 1 -- 1..n Carrier Targets
Qualification Evidence 1 -- 4 Package Policy Contexts
Qualification Evidence 1 -- 3 Supply Chain Equivalence Results
Qualification Evidence 1 -- 2 native architecture realization sets
Qualification Evidence 1 -- 1 Typed Qualification Validator verdict
Qualification Evidence 1 -- 1 No-Shell Spawn Inventory digest
Qualification Evidence 1 -- 7 PID-Namespace Containment Qualification Results
Qualification Evidence 1 -- 1 Promotion Record
Coverage Map 1 -- 1 Hybrid Disclosure Census
Hybrid Disclosure Census 1 -- many governed documents
Repository State 1 -- 1 Diagnostic Command Version
Promotion Record 1 -- many Post-Promotion Run Units
Post-Promotion Run Unit 1 -- 1..n Run Attempts
Post-Promotion Run Units many -- 1 Derived Promotion Streak
```
