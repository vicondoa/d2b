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
| The amendment described offline tools without preserving the binding action no-network rule. | ADR 0052 prohibits every socket across the whole governed action, including Bazel test setup before the Rust payload. Linux network namespaces do not deny socket creation. | Nix-pin a Bazel 8.6.0 Linux sandbox patch that loads the fixed filter before exec of the action command, bind exact source/patch/policy/output hashes, require the patched sandbox with no strategy fallback, and cover setup, compile/build, test, descendant, inherited-capability, eight pre-action socket/io_uring, external-egress, and live-index plants; keep mandatory socket tests on same-commit non-advisory Cargo compatibility carriers until separately authorized. |
| Moving both Nix packages to the root lock made the git output hash look implicit. | ADR 0054 says both dedicated derivations retain the pinned git output hash. | Assert the exact `wl-proxy-0.1.2` key and value in both derivations. |
| Module lock drift named no complete repository path. | Measured Bazel remediation lacks worktree startup options. | Test then implement no-argument, lock-only, idempotent `bazel-module-refresh` with exact repository remediation. |
| Current amended recovery prose shortened command sequences. | ADR 0052 fixes exact per-code commands and forbidden cross-code remedies. | Restore literal commands and redaction/wrong-remedy mutations. |
| The post-promotion children were both concurrently ready while owning the same binding docs and evidence file. | Eligibility clocks are independent, but concurrently ready scopes must be file-disjoint. | Permit spec003w7 qualification and code preparation in parallel, then make its shared documentation/evidence task and merge depend on merged spec003w6, followed by revalidation and a new panel. |
| The amended cache record retained only a zero-write summary. | ADR 0052 requires explicit restore, save, publication, duration, key, prefix, and retention semantics. | Restore fields and their fixtures in spec003w3 through spec003w5. |
| Provider and expiry summaries omitted load-bearing kernel detail. | Same-descriptor `execveat`, close-on-exec behavior, and non-consuming grace observations are binding. | Restore exact flags, fallback semantics, mutations, and host conformance. |
| Provider behavior tests alone proved the verified-handle seal. | Runtime behavior cannot prove an API is absent, and execution must preserve the same verified open file description without exposing it. | Use the compiler-derived closed API/trait census plus focused rustdoc compile-fail examples; co-locate the handle and sole consuming API in one dependency-leaf crate; pass it through pinned safe command-fd mapping to an exact immutable statically built C execution supervisor. |
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
| Network namespaces were described as action-wide no-socket enforcement, then an action wrapper was claimed to cover only payload descendants. | A namespace does not deny socket creation, socketpair, or io_uring networking, and a payload wrapper cannot cover Bazel's `test-setup.sh`. | Patch the Nix-pinned Bazel 8.6.0 Linux sandbox so its child preflights inherited authority and loads the fixed filter before exec of the complete action command. Bind configured action and strategy inventories, setup-before-payload and patch-removal plants, and no process/local/standalone/worker/remote fallback. |
| The runner planned a repository-authored post-fork raw-syscall child under a workspace that forbids unsafe code, then treated a runner-local unsafe quarantine as pre-authorized. | ADR 0009 permits no new first-party Rust unsafe exception without another ADR, and safe Rust helper wrappers cannot make post-fork supervision unambiguous. | Keep every new crate at `unsafe_code = "forbid"`. Put `VerifiedExecutable` and its sole consuming API in one dependency-leaf crate; use pinned safe command-fd mapping; invoke one exact immutable statically linked single-threaded C supervisor built as dedicated Nix tooling outside the Rust workspace; bind its complete protocol and identity; close every other invocation by policy. |
| The exec helper treated close-on-exec EOF as success after replacing itself with the target, so a fast target exit and a helper failure could carry the same status with no surviving authority to distinguish them. | Exec success needs an observer that remains alive after the target image starts and owns signal forwarding, wait, and reap. | Make the static C helper a supervisor: it creates the child exec-error pipe, forks once, emits `READY` and `EXECUTED`, remains alive, forwards the fixed termination-signal set, reaps, and mirrors exact target status. EOF or crash before `EXECUTED` is always typed helper failure. |
| The surviving C supervisor was treated as the final target owner, so supervisor crash could orphan a target group and long-lived descendants. | The exact patched Bazel Linux sandbox already creates a fresh PID namespace with a distinct PID-1 monitor, but uninterruptible kernel cleanup cannot be bounded honestly. | Patch and pin namespace PID 1 as abnormal-teardown owner. Bound only userspace TERM/KILL/monitor escalation and the close-or-quarantine decision to 10,000 ms. At that ceiling, a task not observably reaped enters owned `pending-kernel-cleanup`; success and reuse remain prohibited until consuming reap. Real plants cover every crash stage, both descendant shapes, and beyond-ceiling quarantine; namespace, patch, ceiling, quarantine, false-reap, reuse, and fallback mutations fail. |
| Supervisor signal prose left inherited `SIGPIPE`, non-waitable `SIGCHLD`, external `SIGTERM` without a case deadline, and a normalization window before signal ownership. | A closed status reader must be typed transport failure, child status must remain waitable, and pending termination must not kill the helper during normalization. | Block managed signals first; while blocked install dispositions and synchronous consumption; only then establish the final mask. Ignore `SIGPIPE`, restore waitable `SIGCHLD`, define supervisor ownership before `READY`, and make external `SIGTERM` run the fixed escalation. Add pending-at-entry, normalization-time, closed-reader, inherited-SIGCHLD, blocked/ignored-SIGTERM, and target-ignore-TERM fixtures. |
| Supervisor transport described bounded I/O but applied single-record overlong probing to the multi-record status stream. | Exec-error is one record, while status can legally coalesce several records in one pipe read. | Keep EOF/one-record-plus-one-byte overlong handling only on exec-error. Give status a fixed header/version/type/length and bounded stateful decoder that retains fragmented or coalesced `READY`, `EXECUTED`, and terminal frames; reject malformed, duplicate, and out-of-order frames without a one-byte status probe. |
| HELPER/CHILD and parent/sandbox cleanup stages had typed names but no complete operator recovery mapping. | Every refusal needs a stable code, resolvable fixed input, literal correction, and a rerun target that exists in that phase; a renderer must outlive the stage it reports. | T067/T068 own only parent/helper/child mapping and tests. The patched sandbox, owned by sequential T120, owns `SANDBOX_*` mapping/rendering and live exact tests. Both harnesses resolve every governed repository-relative path and Markdown anchor and cover all slices and both command versions. |
| Ad hoc Cargo-shelling compile fixtures were the VerifiedExecutable API proof. | `tests/AGENTS.md` makes the compiler-derived API census primary and reserves rustdoc compile-fail for downstream type properties. | Make VerifiedExecutable a capability root with empty public-inherent and locally-authored explicit-trait allowlists, pin compiler auto/blanket impls, and use focused rustdoc compile-fail examples only. |
| The plan validator truncated aggregate ownership prose after `and every`, and its first census saw only unordered unquoted checkboxes. | An ordered, indented, blockquoted, omitted, or zero-task plan can evade canonical parsing; setup exceptions and dynamic values can also leak paths or plan content. | Census every Markdown unchecked task-list form before parsing; reject all noncanonical forms and zero tasks; compare parsed IDs with an independent exact census in `tasks.md`; isolate every branch including actual task omitted from census and malformed/unbalanced markers; byte-match complete stderr; authorize only bounded numeric plus closed `none`/`overflow` locators; and initially guard temp-dir, path, open3, and subprocess setup with the fixed self-test-contract diagnostic. The later seam-classification correction below supersedes only that initial setup-diagnostic choice. |
| Crash containment was qualification prose without a closed result shape. | Qualification can pass only when every crash stage, monitor identity, cleanup outcome, quarantine transition, and validator mutation is independently bound without persisting process data. | Add exactly seven bounded containment results with closed supervisor recovery/escalation/cleanup/quarantine enums and SHA-256 patch, canonical-monitor, pending-observation, and result digests. Prohibit raw PIDs, descriptors, paths, process output, and opaque identities; require every result and mutation. |
| Native inventory prose alternated between five checks, two artifact rows, and six checks. | There are exactly six native checks per system and exactly four artifact baseline rows. | Normalize every inventory, task, evidence threshold, quickstart check, and checklist item to those cardinalities. |
| Size growth relied on prose review without a typed authorization. | A changed baseline must bind its exact candidate and review. | Add closed positive/negative size-growth authorization fixtures and require all four row digests plus every nonzero authorization digest in qualification. |
| Artifact baselines persisted exact Nix closure paths and post-promotion checkpoints persisted a cursor. | Exact store paths and pagination tokens are transient validation data. | Persist only closed states, counts, and SHA-256 digests; make fixed-code diagnostics repository-relative and digest-only. |
| Sink policy named a retention class but defined no closed limits or expiry. | JUnit, `test.log`, unsealed evidence, and exporter diagnostics need enforced age and count limits. | Add `junit-v1`, `test-log-v1`, `evidence-v1`, and `exporter-diagnostic-v1` with injected age/count/expiry tests before publication. |
| No-shell prose sometimes listed four plants and checked only a unique scan projection. | The exact six plants and both raw and unique scan counts are binding. | List all six everywhere and require both counts to equal governed-source count. |
| Socket plants leaked into stub-carrier acceptance language. | Socket denial belongs only to hermeticity/action-network. | Remove forbidden-listener/socket plants from the stub carrier and enforce carrier ownership. |
| Evidence status was a string with optional fields. | Complete and degraded states must be structurally closed without changing manifest v1. | Put `sinkKind` and `retentionClass` once in the common record and use a tagged sidecar union with disjoint fields, closed codes/commands, and schema-valid unchanged manifest-v1 output. |
| Provider, publication, qualification, and release refusal prose used generic rerun placeholders. | Operator refusal UX must render exact commands without leaking identifiers. | Add closed reason-by-slice and qualification/release command tables with fixed-code, repository-relative, digest-only diagnostics. |
| Promotion docs named aliases but not retained Cargo coverage. | Socket-using compatibility cases survive Cargo retirement. | Require promotion and retirement docs and semantic changelog fragments to list exact hybrid surfaces and state separate authorization is required for retirement. |
| Hybrid disclosure depended on prose review. | A retained Cargo case can disappear from one binding doc while execution remains hybrid, and a surface-only set collapses distinct cases. | Add an enforcing type-5 policy lint that derives the exact nonempty full carrier census from the coverage map, retaining surface, selector, test identity, and socket class, and compares it bidirectionally with every governed hybrid doc and present semantic fragment. Isolate empty census, missing, extra, malformed/duplicate block, malformed/duplicate identity, stale attribution, and governed-document mismatch fixtures. |
| Recovery diagnostics kept naming shadow aliases after the aliases were removed. | A fixed remedy, threshold, evidence variant, state label, or doc that invokes a nonexistent target is not actionable and violates the fail-closed contract. | Version the closed diagnostic command enum. Use shadow targets only in a pre-change fixture where they exist, then make T108 atomically own every production renderer, both module roots, qualification table, evidence/publication path, exact-message test, governed doc, evidence field, and semantic fragment while switching to enduring promoted aggregate/slice targets. |
| Release queries suppressed `git` and `gh` errors while trying the next tag. | An unavailable query backend is degraded evidence, not proof that a release object is absent. | Use closed typed complete/degraded/refused outcomes, distinct query codes, no persisted or printed candidate/tag identifier, no raw output, and exact remedies. |
| Artifact rows carried a size allowance beside the authorization, and evidence variants repeated sink classification. | Duplicated authorities admit contradictory records, and unrelated old/new arithmetic can authorize a real artifact. | Derive positive size delta only from `sizeGrowthAuthorization`, require its prior/new bytes to equal the baseline row and realized artifact, keep sink kind and retention once in the common evidence record, and reject duplicate authority fields. |
| Blocking managed signals only inside the helper left an exec-time handoff window, and resetting inherited `SIG_IGN` would silently override caller intent. | A spawned image inherits the spawning thread's mask while ignored dispositions survive exec. | Under a process-wide guard, the safe Rust typed consumer uses reviewed safe `nix::sys::signal::SigSet` calls to capture and block the full managed set before spawn and restore the exact mask after every spawn result. The helper's first setup operation inspects managed dispositions while blocked and fails before fork with a typed recovery code on any `SIG_IGN`; it never resets and continues. Deterministic ignored-disposition and handoff-window `SIGTERM` tests add no Rust unsafe. |
| The child alone created its process group, so `READY` or an early termination request could race group creation. | Forwarding is safe only after the supervisor has proved the target group exists and the child is still live. | Child and supervisor both call `setpgid`; a close-on-exec confirmation barrier keeps managed signals blocked until the supervisor confirms the exact group. `READY` and managed-signal consumption/forwarding wait for confirmation. Deterministic parent-first, child-first, early-signal, `ESRCH`, `EPERM`, other-error, mismatch, and early-child-exit tests require typed cleanup. |
| Pending-kernel-cleanup recovery suggested reboot and had no governed operator path. | Reboot destroys the original wait owner and cannot prove a consuming reap. | Keep the original live patched-sandbox monitor as sole wait owner through its consuming reap. T120 creates and owns `docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine`, links the byte-exact pending diagnostic, and specifies inspection, drain-without-terminate, wait, release confirmation, then rerun. Reboot, retry-before-release, replacement wait ownership, and manual release are forbidden and mutation-tested. |
| Validator setup exceptions collapsed into one generic diagnostic and tests bypassed the public CLI entrypoint. | An operator needs to repair validator setup rather than rewrite `tasks.md`, and buffered sentinel/error data must never leak. | Initially give temp-dir, path, open3, and subprocess setup distinct fixed classes and validator-specific remedies, and exercise `run_cli_entrypoint --self-test` with sentinel output plus die and warn. The later actual-seam correction below supersedes only the injected generic-wrapper design and expands the closed boundary set. |
| Group confirmation and `READY` were treated as authority to forward a managed signal while the exec result was still pending. | A signal can kill the child before exec and produce empty close-on-exec EOF, which is not execution evidence once a pre-exec termination request exists. | Between confirmation and `EXECUTED`, coalesce managed signals into one closed pre-exec setup termination, forward nothing, run no grace, prioritize that request over empty EOF, kill/reap the confirmed group through the helper, emit `HELPER_PRE_EXEC_TERMINATION`, and publish neither `EXECUTED`, target terminal status, nor a target-executed audit event. Add deterministic post-`READY` barrier tests and false-execution mutations; the patched sandbox remains the containment backstop. |
| The signal-handoff contract named one process-wide guard but its tests covered only normal restoration. | Per-launch guards, guard poison, capture/block failure, and unlock-before-restoration could pass while violating process-wide signal inheritance. | Inject capture, block, poisoned-guard, and restoration failures after both spawn outcomes. Hold two launches at deterministic barriers and mutation-test one shared guard plus restoration attempt before unlock, using only reviewed safe APIs and no new unsafe. |
| Validator setup self-tests supplied the class they expected to a generic wrapper. | A wrong production classifier could remain green, and `self-test-contract` was being used for repairable operation failures. | Put fixed classification at the temp-dir, path-resolution, make-path, copy, mkdir, open3, and subprocess capture/wait boundaries. Inject failure and warning at each actual seam through the public CLI, byte-match status/stdout/stderr and redaction, give each seam its own setup remedy, and reserve `self-test-contract` for invalid validator self-test behavior. |
| Empty close-on-exec EOF remained an ambiguous execution proof. | Child death before exec closes the same writer, so no signal-priority rule can turn EOF into a kernel exec fact. | This correction supersedes the confirmation-pipe and EOF-success rows above. Use the natural parent-child `PTRACE_TRACEME` initial stop as the sole release barrier, install `PTRACE_O_TRACEEXEC`, emit `READY`, release with zero-signal `PTRACE_CONT`, require exact kernel `PTRACE_EVENT_EXEC`, detach with signal zero, and emit `EXECUTED` only after successful detach. Empty EOF is failure-channel closure only. Bind Linux/Yama/platform gates, the static four-request plus enforceable constant-argument ptrace seccomp allowance, supervisor-owned dynamic child identity, unchanged action no-network, host evidence, qualification, recovery, negative tests, and mutations. |
| Validator setup tests covered only thrown exceptions and warnings. | False, undefined, malformed, or truthy results without the required side effect could bypass the tested classifier. | Exercise every temp-dir, path-resolution, make-path, copy, mkdir, open3, and subprocess seam through the public CLI with exception, warning, false, undefined, malformed, and successful-with-missing-side-effect returns; require the same seam-specific fixed diagnostic for every variant. |
| Failed validator subprocess capture discarded descriptor-close and wait results. | A child or descriptor could leak while the public CLI reported only the primary setup failure. | Check every close, perform a bounded consuming wait, inject close/wait/retry/exhaustion results, preserve the primary typed setup failure, append only fixed `D2B-SPEC003-PLAN-CLEANUP` on cleanup failure, and render no raw warning, error, or path. The later owned-capture correction below fixes the exact identities and eight-attempt bound. |
| C ptrace examples omitted variadic libc arguments or passed integer literals in pointer-valued positions. | An omitted or incorrectly promoted pointer argument lets libc consume a type-incorrect value; for `CONT` or `DETACH` that can inject an unintended signal. | Spell the only four C calls exactly as `ptrace(PTRACE_TRACEME, 0, (void *)0, (void *)0)`, `ptrace(PTRACE_SETOPTIONS, child, (void *)0, (void *)(uintptr_t)PTRACE_O_TRACEEXEC)`, `ptrace(PTRACE_CONT, child, (void *)0, (void *)0)`, and `ptrace(PTRACE_DETACH, child, (void *)0, (void *)0)`. Bind request and pid values plus pointer positions and types with exact tests and omission, integer-in-pointer-position, exchange, wrong-pid, nonchild, options-in-address, and nonzero-signal mutations. |
| Unsupported system, old kernel, Yama refusal, startup-probe failure, and ptrace seccomp drift shared one helper policy code. | Those predicates fail before helper start and require different repairs; a helper code misstates the owner and can send an operator to change the wrong layer. | Give Nix evaluation, toolchain startup, and patched sandbox distinct fixed codes and causing inputs. Keep helper codes only for initial stop, options, continuation, exec event, and detach after spawn. Byte-test each exact code, correction, and phase-valid rerun; reject borrowed remedies; bind every result into qualification. |
| Validator failed-subprocess cleanup trusted a returned pid tuple and accepted `ECHILD` as reap completion. | A malformed resource-bearing result can substitute another pid, leave the actual child unreaped, skip later descriptor closes after one failure, or hide a retry-bound change. | Return one owned capture object retaining the actual child and three independently snapshotted raw birth descriptor identities. Attempt each descriptor exactly once even after failures, consume-reap only that child with at most eight wait attempts, and accept `ECHILD` only after the object already recorded a consuming reap. Tests use a literal eight independent of the production bound, forbid a ninth wait, inject every descriptor position and rebound mismatch, wrong supplied pid, resource-bearing malformed result, prefix progress, `ECHILD`, retry success, and exhaustion, and prove the actual child was reaped while preserving the primary typed failure plus bounded cleanup code. |
| Static seccomp prose matched `SETOPTIONS`, `CONT`, and `DETACH` to a future child pid. | Classic seccomp sees numeric syscall arguments but cannot derive the supervisor's future fork result or a parent-child relation. | Match only the four request values and enforceable constant arguments. Leave the dynamic pid unmatched for the three parent-issued requests; enforce identity through the supervisor-owned fork result, confirmed process group/direct-parent relation, traced initial stop, wait ownership, and exact event. Native host-conformance and wrong-pid/nonchild mutations must refuse without claiming static child-pid filtering. |
| Child setup prose allowed final signal restoration before `PTRACE_TRACEME`. | A pending signal could be delivered before trace ownership and bypass the initial-stop protocol. | Require one order everywhere: complete stdio/CLOEXEC/descriptor closure setup, call `PTRACE_TRACEME`, restore final child signal mask and dispositions, then raise `SIGSTOP`. Bind the order with source and mutation tests. |
| Validator birth-identity and cleanup-progress checks were self-referential. | Capturing identities inside the object under test can hide a rebound descriptor, while testing only zero progress misses double-close or skipped-tail regressions. | Snapshot all three raw birth identities independently, inject mismatch at each position, and prove refusal closes only owned handles and reaps the actual child. Add position-0 and positions-0-1 prefix-progress cases for successful and failed attempts; assert no double-close, each remaining close exactly once, and retain the literal-eight/no-ninth wait proof. |

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
15. Every governed Bazel Rust action uses the exact Nix-pinned Bazel 8.6.0
    output whose Linux sandbox patch, source, policy, output NAR, executable,
    and capability hashes are committed. The sandbox child preflights inherited
    sockets and every io_uring ring, loads the fixed filter, and only then
    execs the action command, so compile/build commands, test setup, tests, and
    descendants inherit it. Configured-target, `aquery`, and strategy
    inventories forbid process/local/standalone/worker/remote fallback.
    Mandatory socket-using tests remain exact same-commit non-advisory Cargo
    compatibility carriers; repository fetches remain outside governed
    actions, offline, and pinned.
16. Both dedicated Nix derivations carry the exact committed `wl-proxy` output
    hash.
17. Provider execution opens with `O_RDONLY|O_CLOEXEC` and
    `RESOLVE_NO_MAGICLINKS` only, uses no `RESOLVE_BENEATH` or
    `RESOLVE_NO_SYMLINKS`, executes the same verified open file description
    through a private descriptor mapped to the exact immutable Nix-built
    static C `d2b-bazel-exec-supervisor`. Under the one process-wide
    serialization guard, the
    safe Rust consumer blocks the spawning thread's complete managed set before
    spawn and attempts restoration of its exact prior mask after every spawn
    result before unlock. The
    single-threaded supervisor first refuses any inherited managed `SIG_IGN`,
    installs synchronous consumption while the inherited set stays blocked,
    forks once, performs both sides of a confirmed setpgid handshake before
    `READY`, then emits framed `READY`. Until `EXECUTED`, it coalesces any
    managed signal into one typed setup termination, forwards nothing, runs no
    grace, and suppresses false execution, target status, and audit
    publication even on empty exec-pipe EOF. It then emits `EXECUTED` only for
    proven exec, remains alive, forwards the fixed post-exec signal allowlist, reaps, and
    mirrors exact target status through the stateful framed stream. The patched sandbox's fresh
    PID-namespace monitor owns abnormal teardown with one fixed userspace
    ceiling and typed pending-kernel-cleanup quarantine, including supervisor
    crash; Rust never signals a numeric PID or PGID. The child
    installs stdio, sets the executable fd CLOEXEC, and calls
    `execveat(AT_EMPTY_PATH)` with no path fallback. Forced walk applies
    `O_NOFOLLOW` only to intermediates. Strict result and cleanup paths keep
    all three resolve flags. Expiry observes without consuming until
    unconditional group kill and direct-child reap.
18. Every mutating validation command leaves the committed candidate clean.
19. Native guest ELF evidence requires expected `e_machine`, `ET_DYN`, no
    `PT_INTERP`, and no `DT_NEEDED`.
20. Generated locks, BUILD files, and coverage/query goldens are
    integrator-owned. The three Nix-unit presence pins have two ordered owners:
    T120 generates the initial toolchain pins before T008, and integrator T020
    may regenerate them after later Nix cases.
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
28. `VerifiedExecutable` has compiler-derived closed public/trait allowlists
    and is co-located with its sole consuming API in one dependency-leaf crate.
    That safe Rust API preserves stdio, uses pinned safe command-fd mapping,
    and is the one permitted Rust invocation of the exact immutable Nix-built
    static C supervisor. Every first-party Rust crate remains
    `unsafe_code = "forbid"`. The supervisor source, derivation dependencies,
    protocol, output, signal, transport, owner/closure, and exact-status
    behavior are pinned, and every other invocation is an enforcing policy
    failure.
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
36. Every sandbox-policy stage, qualification failure, release query, and planning
    validator failure has a closed fixed code, exact repository-relative
    remedy, exact rerun, and leak-rejection coverage.
37. The exact nonempty Cargo compatibility census is enforced against every
    governed hybrid document and semantic migration fragment by a type-5
    policy lint with empty-census, missing, extra, malformed/duplicate block,
    malformed/duplicate identity, stale-attribution, and governed-document
    mismatch negatives.
38. Diagnostic command version 1 names existing shadow targets. Alias removal
    atomically changes every diagnostic and byte-exact test to version 2's
    enduring promoted aggregate or slice target and records that transition in
    its semantic changelog.

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
  d2b-bazel-exec/
  d2b-bazel-runner/
  d2b-test-locator/
  d2b-contract-tests/tests/{policy_bazel_hybrid_docs.rs,policy_bazel_toolchain.rs}
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
  golden/bazel-toolchain.json
  golden/bazel-exec-supervisor.json
  golden/flake-check-matrix/
  golden/pinned/{kernel-canaries,usbip-firewall-skeleton,host-prepare-network,broker-socket-acl,broker-export-audit}.txt
  layer1-jobs.json
  test-rust.sh
  tools/assert-pinned-tests.sh
  tools/d2b-bazel-exec-supervisor/{supervisor.c,sandbox-crash-plant.c}
  tools/no-bash-ast-walker/src/main.rs
  tools/flake-check-classes.sh
  unit/nix/pinned/{common,x86_64-linux,aarch64-linux}.txt
  unit/nix/cases/bazel-toolchain.nix
.github/workflows/pr-bazel-rust.yml
.github/workflows/pr-l1-static-fast.yml
nixos-modules/host-broker.nix
pkgs/bazel-8.6.0-seccomp/{default.nix,linux-sandbox-seccomp.patch,seccomp-policy.json}
pkgs/d2b-bazel-exec-supervisor/default.nix
flake.nix
AGENTS.md
tests/AGENTS.md
docs/contributing/gates-and-lints.md
docs/reference/test-execution-manifest.md
specs/003-adr052-bazel-rust/tools/validate-plan-structure.pl
specs/003-adr052-bazel-rust/tools/validator-fixtures/
```

The generated output set above is closed and exact. Generated outputs are
integrator-owned except the three Nix-unit presence pins explicitly owned by
sequential T120 before T008 and later integrator T020. A parallel slice may
generate a scratch preview but never commits any other shared generated
output. No task may replace this list with a manifest expansion or other
dynamic ownership expression.

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

- Before each implementation wave, run a Track A plan panel using the
  authoritative lifecycle selection. Apply every trigger and the applicable
  floor from `.github/skills/d2b-panel-round/selection-table.json`, dispatch
  exactly the ordered roster and per-seat profiles recorded by the selection,
  and require unanimous signoff with empty recommendations from every selected
  seat.
- For fix verification, rerun selection over the full current candidate and
  every fix delta, union each result into the lifecycle roster, never remove a
  selected seat, and dispatch exactly the roster and profiles in the resulting
  selection.
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
  generated workflows after merging scope commits, except that sequential
  T120 owns the initial three Nix-unit presence pins before T008; later T020
  may regenerate only those same pins.
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
  Before canonical parsing, the read-only command censuses every Markdown
  unchecked task-list form, including unordered, both ordered forms, indented,
  and blockquoted variants, and rejects every noncanonical form. It rejects
  zero tasks and binds the parsed main-plan IDs to the independent exact census
  stored in `tasks.md`. It then checks task-ID uniqueness, dependency existence
  and textual order, exact adjacency-list equality, acyclicity, literal exact
  file ownership, complete parsing, and conflicts between incomparable tasks.
  It rejects dot/dot-dot components, absolute paths, repeated separators,
  malformed quoting/backticks, unresolved expressions, duplicate paths or
  dependencies, and repeated metadata fields. One positive and forty-seven
  isolated negative fixtures preserve the prior forty-four and add actual
  task omitted from census plus malformed and unbalanced census-marker
  coverage. They cover whole-task omission, empty input,
  unordered/ordered/indented/blockquoted forms, and every remaining branch.
  Every negative compares complete stderr byte-for-byte with an independent
  literal through the injectable entrypoint, including exact exit status.
  Temp-dir, path-resolution, make-path, copy, mkdir, open3, and subprocess
  capture/wait exceptions, warnings, false, undefined, malformed, and
  successful-with-missing-side-effect results are injected at their actual
  operation seams and execute through `run_cli_entrypoint --self-test` after
  the runner writes sentinel stdout/stderr. No test supplies an expected
  reason to a generic setup wrapper. Each seam produces status 1, empty
  stdout, and only its fixed setup-class stderr with its own validator-specific
  remedy, not a task rewrite. Failed-subprocess capture returns one owned
  process object retaining the actual child identity and three independently
  snapshotted raw descriptor birth identities. A mismatch at each position
  refuses while closing only the owned handles and consume-reaping the actual
  child. Position-0 and positions-0-1 prefix-progress cases cover successful
  and failed prior attempts, forbid double-close, and close every remaining
  descriptor exactly once. Cleanup otherwise attempts each descriptor exactly
  once even when an earlier close fails, then consume-reaps only the owned
  child in at most eight wait attempts; `ECHILD` is success only after that
  object already recorded a consuming reap. Tests use an independent literal
  `8`, assert no ninth wait, inject each descriptor position, wrong supplied
  pid, resource-bearing malformed result, wait `ECHILD`, retry success, and
  retry exhaustion, and prove the actual child was reaped. Every result
  preserves the primary typed
  failure and appends only fixed `D2B-SPEC003-PLAN-CLEANUP` on cleanup failure;
  raw warning/error/path and sentinel content are discarded.
  `self-test-contract` is byte-tested only for invalid validator self-test
  behavior.
  Unreadable-source status 1 and
  unsupported-argument status 2 execute actual subprocesses with empty stdout
  and byte-exact stderr. Adjacency tests
  independently scan the fixture to assert the reported physical row. Census,
  section, and mismatch diagnostics retain actual offsets and ordinals. A
  diagnostic contains only its fixed code, fixed repository-relative source,
  a bounded 1-based numeric locator or the closed `none`/`overflow` sentinel,
  fixed class reason, fixed remedy, and exact self-test-plus-plan rerun.
  Oversized record and line inputs assert both sentinels. It emits no
  task/dependency
  ID, owned path, content, count, operator value, raw OS text, or absolute path.
  Every code has one exact remedy and rerun command. A nonzero result blocks
  dispatch. It remains a planning tool under this directory and is not added
  to a repository gate.

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
packages/d2b-bazel-support/tests/provider_handle.rs
packages/d2b-bazel-support/tests/startup.rs
packages/d2b-bazel-exec/Cargo.toml
packages/d2b-bazel-exec/src/lib.rs
packages/d2b-bazel-exec/src/provider.rs
packages/d2b-bazel-exec/src/execute.rs
packages/d2b-bazel-exec/tests/verified_executable_api.rs
packages/d2b-bazel-exec/tests/execute.rs
packages/d2b-bazel-exec/tests/supervisor_protocol.rs
packages/d2b-bazel-runner/Cargo.toml
packages/d2b-bazel-runner/src/lib.rs
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
- creates one dependency-leaf execution crate that owns
  `VerifiedExecutable` and the only public API that consumes it. The API uses
  the exact pinned reviewed safe `command-fds` dependency to map the verified
  descriptor to the fixed private fd while preserving declared stdio. The
  safe Rust consumer has the only helper invocation site. Under the one process-wide
  serialization guard, its spawning thread uses the reviewed safe
  pinned `nix` 0.29 `signal` feature's `nix::sys::signal::SigSet` API to
  capture its exact mask, block the complete managed set before spawn, and
  attempt restoration of the captured mask after successful or failed spawn
  before unlock.
  No new signal FFI dependency is added. It models the fixed single-record
  exec-error failure protocol, exact initial trace stop,
  `PTRACE_O_TRACEEXEC`, kernel exec-event and zero-signal detach transitions,
  and fixed-header stateful framed `READY`, `EXECUTED`, failure, and terminal
  status protocol;
- adds no Rust helper crate. Injected prep tests cover the Rust-parent
  stage-error and owner/closure table, one-site invocation policy, private-fd
  mapping, capture/block/guard-poison/restoration failures after both spawn
  outcomes, overlapping-launch serialization and restore-before-unlock
  mutations, protocol discrimination, held-open/partial transport, group and
  initial-trace-stop races, deterministic post-`READY` pre-exec signal
  queuing, pre-exec death/fault/empty-EOF/wrong-event/detach failures, no
  false execution/audit, and a fast target status equal to a planted helper
  crash after the event and detach. The dedicated static C
  supervisor source and real-output conformance land in the sequential
  toolchain scope. No runner `sys.rs`, Rust raw fork, `pre_exec`, Rust signal
  handler, signal-disposition mutation, runfiles/worktree helper path, target
  reopen, or unsafe exception exists;
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

### Sequential toolchain foundation

Before the Bazel generator opens, `spec003w0-toolchain` exclusively owns
`flake.nix`, `pkgs/bazel-8.6.0-seccomp/{default.nix,linux-sandbox-seccomp.patch,seccomp-policy.json}`,
`tests/tools/d2b-bazel-exec-supervisor/{supervisor.c,sandbox-crash-plant.c}`,
`pkgs/d2b-bazel-exec-supervisor/default.nix`,
`tests/golden/{bazel-toolchain.json,bazel-exec-supervisor.json}`,
`tests/unit/nix/cases/bazel-toolchain.nix`,
`tests/unit/nix/pinned/{common,x86_64-linux,aarch64-linux}.txt`, and
`docs/contributing/critical-subsystems.md`, and
`packages/d2b-contract-tests/tests/policy_bazel_toolchain.rs`. It lands the
green Nix packages, exact identity pins, startup capability, patch-removal,
filter-load, fresh PID-namespace monitor, fixed abnormal-teardown ceiling,
patched-sandbox-owned `SANDBOX_*` mapping/rendering and live exact tests,
real crash-stage/long-lived-descendant integration, and a beyond-ceiling
`pending-kernel-cleanup` plant with owned no-success/no-reuse quarantine and
eventual consuming reap by the original live monitor. It creates the governed
`docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine`
runbook section with exact inspect, drain-without-terminate, wait,
release-confirmation, then rerun steps and no reboot, retry-before-release,
replacement waiter, or manual release,
static-supervisor source/dependency/output/protocol, and
framed status, single-record exec-error, inherited-signal verification,
ignored-disposition refusal, Rust-to-helper handoff-window `SIGTERM`,
parent/child setpgid confirmation, exact initial ptrace stop/options/release,
all four libc arguments for the exact
`ptrace(PTRACE_TRACEME, 0, (void *)0, (void *)0)`/
`ptrace(PTRACE_SETOPTIONS, child, (void *)0, (void *)(uintptr_t)PTRACE_O_TRACEEXEC)`/
`ptrace(PTRACE_CONT, child, (void *)0, (void *)0)`/
`ptrace(PTRACE_DETACH, child, (void *)0, (void *)0)` calls, kernel
exec-event, zero-signal detach, exact argument-position/type mutations,
early-signal/group-race cleanup,
post-`READY` pre-exec signal queuing,
pre-exec-death/fault/empty-EOF/wrong-event/detach refusal, helper group
kill/reap, fast first-instruction exit, separately owned and coded unsupported
system/kernel/Yama/startup-probe gates before helper start, patched-sandbox
ptrace-policy drift, static four-request plus enforceable constant-argument
ptrace seccomp allowance, supervisor-owned dynamic child identity and native
wrong-pid/nonchild refusal with unchanged action no-network, no
false `EXECUTED`/terminal/audit publication, and no-first-party-Rust-unsafe
tests.
Cargo tests retain mocks; the real
containment proof runs only through the patched Bazel Linux sandbox. It
implements sandbox mapping/rendering only in
`pkgs/bazel-8.6.0-seccomp/linux-sandbox-seccomp.patch`; live byte assertions
and the beyond-ceiling plant live only in
`tests/tools/d2b-bazel-exec-supervisor/sandbox-crash-plant.c`,
`tests/unit/nix/cases/bazel-toolchain.nix`, and
`packages/d2b-contract-tests/tests/policy_bazel_toolchain.rs`; the two golden
JSON files bind their identities. The policy test resolves the contributing
runbook file and anchor exactly once and byte-matches the pending diagnostic,
runbook link, and consuming-reap release. It
regenerates all three Nix-unit presence pins, proves a second regeneration is
a clean no-op, and runs `make test-nix-unit` before the generator may open.
The later Nix-policy scope is a dependency descendant and may extend
`flake.nix`; it must preserve these identity pins byte-for-byte. T020 may
regenerate the same three pins after its later Nix cases land. No concurrent
scope overlaps this ownership.

### Parallel slice ownership

| Slice | Owned files |
| --- | --- |
| `spec003w0-cargo-gates` | `tests/test-rust.sh`, `tests/tools/assert-pinned-tests.sh`, `tests/golden/pinned/kernel-canaries.txt`, `tests/golden/pinned/usbip-firewall-skeleton.txt`, `tests/golden/pinned/host-prepare-network.txt`, `tests/golden/pinned/broker-socket-acl.txt`, `tests/golden/pinned/broker-export-audit.txt` |
| `spec003w0-bazel-generator` | `.bazelversion`, `.bazelrc`, `MODULE.bazel`, `BUILD.bazel`, `bazel/BUILD.bazel`, `bazel/defs.bzl`, `bazel/toolchains.bzl`, `bazel/rules/sandboxed_action.bzl`, `bazel/cargo/README.md`, `bazel/cargo/BUILD.bazel`, `bazel/cargo/cargo_bazel.bzl`, `packages/xtask/src/bazel.rs`, `packages/xtask/src/package_policy.rs`, `packages/xtask/src/bazel_yanked.rs`, `packages/xtask/src/schema.rs`, `packages/xtask/src/hermeticity.rs`, `packages/xtask/tests/bazel_foundation.rs`, `packages/xtask/tests/bazel_module_refresh.rs`, `packages/xtask/tests/package_policy_refusals.rs`, `packages/xtask/tests/bazel_action_network.rs` |
| `spec003w0-runner-foundation` | `packages/d2b-bazel-runner/src/exec_handle.rs`, `packages/d2b-bazel-runner/src/bin/d2b-exec-probe.rs`, `packages/d2b-bazel-runner/tests/exec_handle.rs` |
| `spec003w0-locator-foundation` | `packages/d2b-test-locator/src/mode.rs`, `packages/d2b-test-locator/tests/mode_selection.rs` |
| `spec003w0-nix-policy` | `nixos-modules/host-broker.nix`, `flake.nix`, `tests/unit/nix/cases/bazel-package-policy.nix`, `packages/d2b-contract-tests/tests/policy_bazel_nix.rs`, `packages/d2b-contract-tests/tests/policy_bazel_supply_chain.rs`, `packages/d2b-guest-shell-runner/deny.toml` |
| `spec003w0-policy-ci` | `tests/lib.sh`, `packages/xtask/tests/policy_ci.rs`, `packages/d2b-contract-tests/tests/policy_docs.rs`, `tests/unit/meta/w0-dep-direction.sh`, `tests/unit/meta/ci-runner-regression.py`, `tests/unit/gates/flake-check-matrix-sync.sh`, `tests/unit/gates/ci-rust-cache-sync.sh`, `tests/layer1-jobs.json`, `tests/tools/layer1-jobs.py`, `tests/ci/layer1-workflow.template.yml`, `tests/tools/flake-check-classes.sh`, `tests/tools/gen-flake-check-matrix-pin.sh`, `.github/workflows/release-host-binaries.yml` |
| `spec003w0-binding-docs` | `AGENTS.md`, `tests/AGENTS.md`, `CONTRIBUTING.md`, `docs/contributing/gates-and-lints.md`, `docs/contributing/workflow.md`, `docs/contributing/critical-subsystems.md`, `docs/adr/0052-bazel-rust-build-and-test.md`, `docs/adr/README.md`, `changelog.d/adr0054-broker-hub.md`, `packages/d2b-contract-tests/tests/policy_modules.rs` |

Only the sequential toolchain scope opens from the first prep tip. After its
exact Nix outputs and pins are green, the Bazel generator opens. After it
integrates, the integrator wires xtask, generates the product and walker hub
locks with that patched Bazel, and refreshes the module lock. The remaining independent spec003w0
scopes open from that green generator-checkpoint tip, so no Cargo process
observes routing or lock mutation in flight.
`spec003w0-nix-policy` begins only after the generator's policy schema is
integrated. `spec003w0-policy-ci` begins only after all three new
fixture-independent policy binaries exist; T022 then adds each exactly once
to the fail-closed `test-policy` inventory and tests missing, extra, and
duplicate membership. `spec003w0-binding-docs` begins
after the integrated command and gate shapes are stable. Its later comparable
ownership of `docs/contributing/critical-subsystems.md` preserves the exact
T120-created pending-cleanup runbook section and edits only the workspace and
gate guidance elsewhere in that file.
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
- one dependency-leaf crate owns both `VerifiedExecutable` and its only
  consuming public API; that safe Rust API consumes the handle by value, maps
  it with the exact reviewed `command-fds` pin, preserves stdin/stdout/stderr,
  and is the one Rust site that invokes the exact immutable Nix store path of
  `d2b-bazel-exec-supervisor`. Under the one process-wide guard, the spawning thread
  uses reviewed safe `nix::sys::signal::SigSet` calls to capture its exact
  mask, block the complete managed set before spawn, and attempt restoration
  of the captured mask after successful or failed spawn before unlock.
  Capture/block/poison/restoration cases and a deterministically overlapping
  pair prove one guard and restore-before-unlock;
- the C source, Nix expression, derivation dependency closure, static ELF,
  protocol, output NAR, native system, and executable digests match
  `tests/golden/bazel-exec-supervisor.json`; the helper is a dedicated static
  build/test-tooling derivation and no Rust workspace crate or unsafe exception
  implements it. It stays single-threaded and inherits managed signals blocked.
  Its first setup operation inspects inherited managed dispositions; any
  `SIG_IGN` is a typed fail-before-fork refusal and is never
  reset-and-continued. Only then does it install normalized dispositions and
  synchronous consumption. It creates one close-on-exec nonblocking child
  exec-error pipe and forks once; no confirmation pipe exists. Child and
  supervisor both call `setpgid`. The child completes stdio, CLOEXEC,
  descriptor setup, exact four-argument
  `ptrace(PTRACE_TRACEME, 0, (void *)0, (void *)0)`, final child signal
  restoration, and then the initial `SIGSTOP`, in that order.
  The supervisor proves the exact live group and initial trace state, installs
  options with
  `ptrace(PTRACE_SETOPTIONS, child, (void *)0, (void *)(uintptr_t)PTRACE_O_TRACEEXEC)`,
  completes `READY`, and releases with
  `ptrace(PTRACE_CONT, child, (void *)0, (void *)0)`. The child
  immediately performs
  same-open-file-description `execveat(AT_EMPTY_PATH)` with no reopen or
  fallback. The exec-error pipe accepts one record or EOF and alone uses the
  additional overlong byte. The status pipe emits fixed-header version-1
  `READY`, `EXECUTED`, and terminal frames; its 27-byte stateful decoder
  retains fragmented and coalesced frames and rejects malformed, duplicate,
  out-of-order, partial-EOF, trailing, and overflow input without a one-byte
  status probe. All I/O retains exact bounded
  `EINTR`/`EAGAIN`/short/partial/closed-reader/held-writer handling under the
  original deadline. Empty exec-error EOF is closure only. Before `EXECUTED`,
  the helper coalesces managed signals into one typed setup termination,
  forwards nothing, runs no grace, and kills/reaps the confirmed group. It
  accepts execution only on exact `PTRACE_EVENT_EXEC` followed by successful
  `ptrace(PTRACE_DETACH, child, (void *)0, (void *)0)`; pre-exec
  death/fault/empty EOF/wrong event or
  detach failure emits no false `EXECUTED`, target terminal, or
  target-executed audit event. After `EXECUTED`, it remains alive, forwards the
  fixed termination signals, reaps, and mirrors exact target status. After its initial
  inherited-disposition and mask verification, it installs dispositions and
  synchronous consumption while blocked and establishes the final mask. It
  ignores `SIGPIPE`, restores waitable default `SIGCHLD`, owns pending,
  handoff-window, normalization-time, or pre-trace-confirmation `SIGTERM`
  before `READY`, and escalates external `SIGTERM` through the complete fixed
  grace even with no case deadline. Missing/wrong output, rebind,
  private-fd identity, descriptor absence, CLOEXEC, stdin, helper crash/EOF
  before `EXECUTED`, fast same-status target exit, inherited
  ignored/`SA_NOCLDWAIT` SIGCHLD, capture/block/guard-poison/restoration
  failures after both spawn outcomes, one-guard overlapping-launch and
  restore-before-unlock mutations, blocked SIGTERM,
  managed-`SIG_IGN` refusal, handoff-window SIGTERM, parent-first/child-first
  setpgid races, typed `ESRCH`/`EPERM`/other-error/early-exit cleanup,
  initial-stop/options/continue failures, pre-confirmation signal ownership,
  deterministic post-`READY` pre-exec signals, pre-exec
  `SIGKILL`/`SIGSYS`/fault/exit/OOM-like kill, empty EOF without event,
  missing/wrong event, detach failure, fast first-instruction exit, no false
  execution/audit, exact request/pid values and pointer-position/type call
  tests and argument mutations, distinct pre-helper Nix/toolchain/sandbox
  system/kernel/Yama/probe/policy codes, distinct post-spawn helper
  stop/options/continue/event/detach codes, exact remedies and phase-valid
  reruns, exact static ptrace request/constant-argument filter, dynamic child
  identity, wrong-pid/nonchild host refusal, and unchanged no-network plants,
  target-ignore-TERM, signal/status
  mismatch, and every Rust-parent and C-supervisor
  ownership/closure/cleanup/wait/reap failure test passes; every
  invocation outside the one typed Rust consumer fails the closed policy;
- the actual Bazel executable matches `tests/golden/bazel-toolchain.json` and
  its exact Bazel 8.6.0 source, Linux sandbox patch, fixed policy, output NAR,
  executable, and capability hashes. The startup probe passes; patch-removal,
  wrong-output, and filter-load plants fail before a governed action.
  Unsupported system, old kernel, Yama refusal, real startup-probe failure,
  and ptrace seccomp-policy drift each fail before helper spawn with its own
  Nix/toolchain/sandbox code, fixed causing input, exact repair, phase-valid
  rerun, byte-exact expectation, and wrong-remedy mutation;
  configured-target, `aquery`, and strategy inventories cover every stable/
  nightly compile, build, setup, and test action and reject process, local,
  standalone, worker, remote, or other fallback. Inherited socket,
  ordinary-ring, SQPOLL-ring, and fixed-socket-ring plants refuse before load;
  setup-before-payload and all eight pre-action socket/io_uring plants return
  the policy errno. Its fresh PID-namespace monitor owns abnormal teardown.
  The fixed 10,000 ms ceiling bounds userspace escalation and the
  close-or-quarantine decision only. A not-yet-reaped PID 1 enters owned
  `pending-kernel-cleanup`; sandbox and outputs cannot succeed or be reused,
  and the original live outer `linux-sandbox` monitor remains the sole wait
  owner through consuming reap and publishes the only release. Its fixed
  pending diagnostic links to the governed
  `docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine`
  runbook, which requires inspect, drain-without-terminate, wait, release
  confirmation, then rerun and prohibits reboot, retry-before-release,
  replacement wait ownership, and manual release.
  Real sandbox plants crash the helper before `READY`, after `READY`, after
  `EXECUTED`, during grace, with direct and double-forked descendants, and in
  a beyond-ceiling pending cleanup. PID-namespace, teardown-patch, ceiling,
  quarantine, false-reap, reboot-remedy, retry-before-release,
  replacement-waiter, manual-release, no-success/no-reuse, and
  strategy-fallback mutations fail. The patched sandbox owns every
  `SANDBOX_*` renderer and live byte-exact test; exact stage diagnostics,
  pending/runbook/release bytes, and full repository-relative locator
  resolution pass leak-rejection tests;
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
- the sequential toolchain task regenerated all three Nix-unit presence pin
  files and passed `make test-nix-unit` before the Bazel generator opened;
  after later Nix cases, T020 may regenerate the same pins, then
  `make nix-unit-pin` is a no-op and `make test-nix-unit` passes;
- all three new fixture-independent policy binaries appear exactly once in the
  fail-closed `test-policy` inventory in `tests/lib.sh`, run under
  `make test-policy`, and are excluded from fixture contracts; missing, extra,
  and duplicate membership regressions pass;
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
- the selected-roster integrated-diff panel has unanimous signoff with empty
  recommendations from every seat in its lifecycle selection;
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
- exact patched-Bazel startup identity plus generated configured-target,
  `aquery`, and strategy inventories prove every governed stable/nightly
  compile, build, test-setup, and test action enters the patched Linux sandbox;
  no process/local/standalone/worker/remote or stage fallback exists;
  inherited socket/ring/SQPOLL/fixed-socket plants refuse before load,
  setup-before-payload and all eight pre-action socket/io_uring plants return
  the policy errno, every stage diagnostic passes exact-message/leak tests,
  patch-removal, filter-load, external-egress, and live-index plants fail,
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

The exact `spec003w2-recovery` ownership is parent/helper/child mapping and
tests only. It contains no Nix-ptrace, toolchain-ptrace, or `SANDBOX_*`
mapping, renderer, or byte-exact case; those pre-helper rows were already
landed in the sequential spec003w0 toolchain files that exist before action
setup and remain through quarantine and consuming reap.
Its closed rows include `PARENT_SIGNAL_HANDOFF`,
`HELPER_SIGNAL_INHERITED_IGNORED`, `HELPER_SIGNAL_HANDOFF`,
`HELPER_GROUP_ESRCH`, `HELPER_GROUP_EPERM`, `HELPER_GROUP_ERROR`, and
`HELPER_GROUP_EARLY_EXIT`, plus distinct helper initial-stop, options,
continue, event, detach, and `HELPER_PRE_EXEC_TERMINATION` rows. Both recovery
harnesses resolve every governed
fixed file and Markdown anchor from the repository root; the T120 sandbox
harness additionally resolves the contributing runbook and byte-matches
pending, link, and consuming-reap release records.

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
- exact patched-Bazel source/patch/policy/output/executable/capability hashes,
  startup probe, configured-target plus `aquery` stable/nightly action-kind
  coverage, sandbox strategy inventory, patch-removal and filter-load results,
  inherited socket/ring/SQPOLL/fixed-socket plants, setup-before-payload and
  all eight pre-action socket/io_uring plants, every fixed-code stage
  diagnostic, all four exact libc ptrace request/pid values and pointer
  positions/types plus every argument mutation, distinct pre-helper
  Nix/toolchain/sandbox code/remedy/
  wrong-remedy result, distinct post-spawn helper stop/options/continue/event/
  detach result, external-egress and live-index results, and the pinned
  offline repository-fetch inventory outside governed actions;
- exact same-commit non-advisory Cargo compatibility-carrier passes for every
  mandatory socket-using test;
- exactly seven bounded PID-namespace containment results covering each crash
and descendant stage plus beyond-ceiling pending cleanup; closed supervisor
recovery, userspace escalation, cleanup, and quarantine values; matching
sandbox patch and canonical monitor identity digests; pending-observation
and result digests; no raw PID, descriptor, path, process output, or opaque
identity; and every required containment-validator mutation result;
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
contains all seven containment results and every named validator mutation,
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
  same-surface cases remain distinct; empty-census, missing, extra,
  malformed/duplicate block, malformed/duplicate identity, stale-attribution,
  and governed-document mismatch fixtures fail; `make test-policy` runs the
  lint, and fixture contracts exclude it;
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
`packages/d2b-bazel-exec/src/provider.rs`,
`packages/d2b-bazel-exec/src/execute.rs`,
`packages/d2b-bazel-exec/tests/execute.rs`,
`packages/d2b-bazel-runner/src/lib.rs`,
`packages/d2b-bazel-runner/src/coverage.rs`,
`packages/d2b-bazel-runner/src/diagnostic.rs`,
`packages/d2b-bazel-runner/src/junit.rs`,
`packages/d2b-bazel-runner/src/manifest.rs`,
`packages/d2b-bazel-runner/src/recovery.rs`,
`packages/d2b-bazel-runner/tests/diagnostic.rs`,
`packages/d2b-bazel-runner/tests/provider_execution.rs`,
`packages/d2b-bazel-runner/tests/recovery.rs`,
`packages/d2b-bazel-runner/tests/result_publication.rs`,
`packages/xtask/src/main.rs`,
`packages/xtask/src/bazel_evidence.rs`,
`packages/xtask/src/bazel_qualification.rs`,
`packages/xtask/src/hermeticity.rs`,
`packages/xtask/tests/bazel_action_network.rs`,
`packages/xtask/tests/bazel_evidence.rs`,
`packages/xtask/tests/bazel_qualification.rs`,
`packages/xtask/tests/policy_ci.rs`,
`AGENTS.md`, `tests/AGENTS.md`, `tests/README.md`,
`docs/contributing/gates-and-lints.md`,
`docs/reference/test-execution-manifest.md`,
`changelog.d/adr052-bazel-alias-removal.md`, and the alias fields in
`specs/003-adr052-bazel-rust/evidence/post-promotion.json`. This exact,
non-dynamic ownership covers every production provider, sandbox-policy,
qualification-threshold, evidence/publication, cleanup, and recovery renderer;
both module-wiring roots; every exact-message test; every governed semantic
doc; and the semantic changelog. T108 changes them atomically. T109 owns no
file and audits the closed census. The slice does not remove Cargo
implementation files.

### Mechanical done condition

The interface test is updated and observed failing before alias removal, then
passes after the atomic T108 change. Only Bazel-specific aliases are removed;
`make bazel-shutdown`,
`make test-rust`, and all eight public leaf names remain; no workflow names a
removed alias. In the same atomic change, diagnostic command version 1 is
retired and version 2 makes every provider, sandbox-policy, qualification
threshold/table, evidence/publication, cleanup, and recovery renderer, both
module roots, every exact-message fixture, all five governed docs, the
post-promotion alias fields, and the semantic fragment name only
`make test-rust` or the enduring
`make test-rust-slice-{main,api,broker,aux}` target. Version 1 remains only as
the pre-change fixture whose shadow rules all exist. A closed policy test
proves no renderer or state label names a removed or nonexistent target. The
type-5 hybrid disclosure census matches every governed doc and the
alias-removal fragment; validation and fixture contracts pass; panel, seal
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
unanimous panel verdict from every seat in the widened lifecycle selection
before merge.

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
| An unpatched Bazel runs `test-setup.sh` before a payload wrapper, or inherited socket/ring authority bypasses ordinary syscall denial. | Exact Nix Bazel source/patch/policy/output identity, startup capability probe, sandbox-child preflight and filter load before action-command exec, setup-before-payload/patch-removal plants, configured action and strategy inventories, and no process/local/standalone/worker/remote fallback. |
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
| The immutable helper is rebound, fd 0 stops being stdin, a mapped descriptor is wrong, concurrent launches use separate signal guards, restoration follows unlock, inherited `SIG_IGN` is silently overridden, group creation races `READY`, empty EOF or a wrong stop false-confirms exec, detach fails after the event, or a fast target exit is mistaken for a helper crash with the same status. | The dependency-leaf safe Rust consumer uses the exact static C Nix output, reviewed safe command-fd mapping, and one serialized safe `SigSet` mask handoff at its invocation site. Capture/block/poison/restoration and overlap mutations prove restore-before-unlock. The child completes descriptor setup, enters `PTRACE_TRACEME`, restores final signal state, and raises the initial stop in that order; the supervisor confirms group/direct-parent/wait/tracing state, installs `PTRACE_O_TRACEEXEC`, emits `READY`, releases with zero signal, accepts only exact kernel `PTRACE_EVENT_EXEC`, and detaches with zero signal before `EXECUTED`. Pre-exec signals/death/fault/EOF/wrong-event/detach failure publish no execution. Platform/kernel/Yama, static request/constant-argument seccomp, dynamic child identity, wrong-pid/nonchild refusal, unchanged no-network, framed-status, identity, stdio, CLOEXEC, transport, recovery, cleanup, wait, and reap plants cover every stage without Rust unsafe. |
| The supervisor crashes after forking and leaves a target or daemonized descendant alive, Rust cleanup signals a reused numeric identity, or recovery destroys the only wait owner before reap. | The patched Bazel sandbox creates one fresh PID namespace whose original live monitor survives the action tree and remains sole wait owner. Its fixed ceiling bounds userspace escalation and the close-or-quarantine decision, while pending kernel cleanup remains quarantined, failed, and non-reusable until that monitor publishes consuming-reap release. The governed runbook drains without terminating it and forbids reboot, early retry, replacement waiter, and manual release. Real crash-stage, descendant, beyond-ceiling, byte-exact diagnostic/link/release, and recovery mutations prove the boundary. |
| Inherited `SIGPIPE`, non-waitable `SIGCHLD`, a pending managed signal, an ignored managed disposition, or a stalled short-I/O loop defeats supervision. | Safe Rust block-before-spawn and restore-before-unlock, helper first-operation ignored-disposition refusal, typed closed-reader `EPIPE`, default waitable `SIGCHLD`, confirmed group before `READY`, no forwarding or grace before `EXECUTED`, deterministic pre-exec setup termination, no-deadline external-TERM escalation, single-record exec-error, and stateful framed-status tests cover every boundary. |
| A cache API page interleaves a foreign prefix and maintenance adopts it. | Closed typed prefix enum, mixed-page fixtures, preservation checks, and zero delete calls on every authorization refusal. |
| Tests pass while forbidden values persist in `test.log` or exporter output. | Pre-sink streaming sanitization, committed measured bounds, planted-value absence across every sink, and typed degraded evidence rejected by qualification. |
| Old or excessive diagnostics accumulate after passing sanitizer bounds. | Four closed age/count retention classes, descriptor-relative expiry before publication, and injected boundary/failure tests. |
| A binary growth allowance is copied to another artifact or accepted without review. | Exactly four baseline rows and a candidate/review-digest-bound closed authorization with positive and replay/stale/wrong-row/size-plus-one negatives. |
| A diagnostic or checkpoint leaks a store path or API cursor. | Transient full validation, persisted closed states/counts/digests only, and fixed-code repository-relative digest-only failure tests. |
| A stale but valid promotion SHA unlocks retirement. | Typed record validation against the actual protected-`v3` PR merge and exact `spec003w5` seal before both eligibility paths. |
| One hybrid document silently omits or corrupts a retained Cargo identity. | Enforcing exact full-identity bidirectional comparison from the nonempty compatibility census to every governed doc and semantic migration fragment, with isolated empty, missing, extra, malformed/duplicate block, malformed/duplicate identity, stale-attribution, and governed-document mismatch plants. |
| Alias removal leaves a diagnostic pointing at a removed shadow target. | Alias removal owns one atomic diagnostic-version transition, every exact-message fixture, and the semantic changelog; version 1 names existing shadows and version 2 names enduring promoted targets. |
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
12. the read-only plan-structure validator passes with the parsed IDs equal to
    the independent exact `tasks.md` census, nonzero tasks, unique IDs,
    existing and earlier dependencies, exact adjacency, an acyclic graph,
    literal exact ownership, a pre-parse census rejecting every unordered,
    ordered, indented, or blockquoted noncanonical task form, and no conflict
    among incomparable scopes; its positive and all forty-seven isolated
    negative fixtures preserve the prior forty-four and add
    actual-task-omitted-from-census and malformed/unbalanced-marker coverage. The
    byte-exact closed diagnostic contract passes. Census,
    section, adjacency, and mismatch locators name actual physical positions;
    non-record and overflow locators are closed; oversized record/line inputs
    and real unreadable-source and unsupported-argument subprocesses pass.
    Temp-dir, path-resolution, make-path, copy, mkdir, open3, and subprocess
    capture/wait exceptions, warnings, false, undefined, malformed, and
    successful-with-missing-side-effect results are injected at their actual
    seams and pass through `run_cli_entrypoint --self-test` after sentinel
    output. Each exact case returns status 1 with empty stdout and only its
    seam-specific fixed setup diagnostic and remedy; no test passes an
    expected reason into a generic wrapper. Failed-subprocess capture returns
    an owned object with the actual child and three independently snapshotted
    raw birth descriptor identities. Per-position rebound mismatches refuse
    while cleanup closes only the owned descriptors and consume-reaps the
    actual child. Position-0 and positions-0-1 prefix-progress cases cover
    successful and failed prior attempts, forbid double-close, and close every
    remaining descriptor exactly once. Cleanup otherwise attempts every
    descriptor exactly once despite prior failure, then consume-reaps only
    that child in at most eight wait attempts.
    `ECHILD` is not success without a previously recorded consuming reap.
    Tests use literal `8` independently of the production bound, assert no
    ninth wait, inject failure at each descriptor position, wrong supplied
    pid, resource-bearing malformed result, `ECHILD`, retry success, and retry
    exhaustion, and prove the actual child reaped. Outcomes preserve the
    primary setup failure and append only fixed
    `D2B-SPEC003-PLAN-CLEANUP` on cleanup failure. No raw warning/error/path,
    sentinel, or task-rewrite remedy appears. `self-test-contract` appears
    only in the exact invalid validator-contract case.
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
    compatibility census and every governed doc/fragment matches every full
    identity in both directions; empty, missing, extra, malformed/duplicate
    block, malformed/duplicate identity, stale-attribution, and
    governed-document mismatch negatives fail.
18. the dependency-leaf type and safe consuming API, exact immutable static C
    supervisor identity, safe command-fd mapping, one-site invocation policy,
    same-open-file-description, stdio, CLOEXEC, single-record exec-error and
    stateful framed `READY`/`EXECUTED`/terminal transport, fragmented/coalesced
    input, malformed/duplicate/order negatives, held-open writer, partial I/O,
    fast-same-status discrimination, closed-reader `EPIPE`, waitable default
    `SIGCHLD`, safe serialized spawning-thread mask block/exact restoration,
    capture/block/poison/restoration failures after both spawn outcomes, one
    shared guard under overlapping launch, restore-before-unlock mutations,
    helper first-operation managed-`SIG_IGN` refusal, handoff-window and
    normalization-time `SIGTERM`, parent/child setpgid and initial-stop races,
    typed `ESRCH`/`EPERM`/early-child-exit cleanup, descriptor setup/
    `ptrace(PTRACE_TRACEME, 0, (void *)0, (void *)0)`/final signal
    restoration/`SIGSTOP` order, then
    `ptrace(PTRACE_SETOPTIONS, child, (void *)0, (void *)(uintptr_t)PTRACE_O_TRACEEXEC)`/
    `ptrace(PTRACE_CONT, child, (void *)0, (void *)0)`/event/
    `ptrace(PTRACE_DETACH, child, (void *)0, (void *)0)` order and
    argument-position/type mutations, pending signal before group and trace
    confirmation, pre-`READY` ownership, deterministic post-`READY` pre-exec
    signals, one queued setup termination, pre-exec
    `SIGKILL`/`SIGSYS`/fault/exit/OOM-like kill, empty EOF without event,
    missing/wrong event, detach failure, fast first-instruction exit, helper
    kill/reap, distinct pre-helper Nix/toolchain/sandbox codes for unsupported
    system, minimum kernel, Yama, startup probe, and ptrace seccomp-policy
    drift, distinct post-spawn helper initial-stop/options/continue/event/
    detach codes, exact remedies and phase-valid reruns, static four-request
    plus constant-argument ptrace seccomp allowance, dynamic child identity
    and wrong-pid/nonchild host refusal, unchanged action no-network, no pre-exec
    forwarding/grace, no false
    `EXECUTED`/target terminal/audit event, no-deadline external-TERM escalation,
    target-status
    mirroring, ownership, cleanup, wait, and reap coverage all pass. The real
    patched-sandbox integration passes crash-before-`READY`,
    crash-after-`READY`, crash-after-`EXECUTED`, crash-during-grace, and
    direct/double-forked long-lived-descendant plants plus a beyond-ceiling
    `pending-kernel-cleanup` quarantine plant whose original live monitor alone
    publishes consuming-reap release. Its governed runbook path resolves and
    its pending/link/release bytes match. PID-namespace, teardown-patch,
    ceiling, quarantine, false-reap, reboot-remedy, retry-before-release,
    replacement-waiter, manual-release, no-success/no-reuse, and fallback
    mutations fail; Cargo tests claim only mock coverage. No Rust
    helper crate, runner `sys.rs`, target path/reopen/fallback, fd-0 transport,
    ambiguous Rust numeric signal, or first-party Rust unsafe exception
    remains.
19. the exact Nix-patched Bazel 8.6.0 identity and startup capability pass;
    its sandbox child loads the fixed filter before compile/build/test-setup/
    test action-command exec; configured action and strategy inventories,
    patch-removal, filter-load, inherited ring/SQPOLL/fixed-socket, and every
    fixed-code sandbox-policy stage diagnostic pass.
20. release query errors remain typed degraded outcomes, qualification and
    planning-validator failures have exact closed remedies, and no tested
    diagnostic leaks runtime paths, descriptors, OS text, raw output, or
    dynamic identifiers.
21. diagnostics name shadow targets only while those targets exist; the
    alias-removal change atomically moves every diagnostic and byte-exact test
    to enduring promoted targets and records the transition semantically.
22. qualification contains every closed PID-namespace containment result,
    canonical patch/monitor identity digest, cleanup/quarantine outcome, and
    validator mutation result, and contains no raw PID, descriptor, path,
    process output, or opaque identity.
