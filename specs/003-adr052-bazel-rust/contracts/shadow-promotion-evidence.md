# Shadow and Promotion Evidence Contract

Evidence summaries under `specs/003-adr052-bazel-rust/evidence/` contain
immutable references and computed outcomes only. They contain no logs,
transcripts, credentials, or attestation payloads.

## Qualification record

One record is a `push` on `refs/heads/v3` produced by a merged pull request and
contains:

- one head commit for Cargo and Bazel workflow runs;
- both run IDs and verdicts;
- a passing same-commit fixture-contract verdict;
- four Bazel slice verdicts and, for a cold record, `sliceDurationsSeconds`
  with exactly four complete durations;
- manifest references;
- explicit `bazelRestoreCount`, `bazelSaveCount`, and
  `bazelPublicationCount`;
- a closed seven-entry PID-namespace containment result set, canonical sandbox
  patch and monitor identity digests, and the complete containment-validator
  mutation result set;
- one closed exec-event qualification result binding both native
  startup/conformance proofs, protocol/seccomp identities, platform,
  minimum-kernel and Yama gates, the exact four request/pid/address/data ptrace
  tuples, exact four-request allowance, unchanged no-network result,
  event/detach positives, every call-position and event mutation, distinct
  pre-helper Nix/toolchain/sandbox code and wrong-remedy result, and every
  post-spawn helper recovery-code result;
- effective permissions.

The canonical cache field spellings are exactly `bazelRestoreCount`,
`bazelSaveCount`, `bazelPublicationCount`, and `sliceDurationsSeconds`.
No other spelling appears in any artifact or record.

Pull-request, `main`-push, scheduled, and dispatched runs are diagnostic only.
Pull-request runs emit no qualification record and contain zero cache actions;
they therefore have no synthetic zero-count qualification object.

Every record carries all three counts. Shadow qualification requires each to be
zero. Every cold record additionally carries `sliceDurationsSeconds` with four
complete durations and `bazelRestoreCount` of zero. A missing count or
duration makes the record non-qualifying; it is never read as zero.

Differing verdicts reset the streak. For every protected-`v3` push, a missing
Cargo, Bazel, fixture, exporter, or publication verdict produces a bounded
record with the available underlying `testVerdict` values,
the structurally valid degraded `evidenceStatus` variant, and one closed
degradation code. This includes
a push where neither workflow reaches a verdict. Degraded records never extend
the streak and reset it. Pull-request, `main`, scheduled, and dispatched runs
remain non-records.

## Typed qualification validator

`packages/xtask/src/bazel_qualification.rs`, with tests in
`packages/xtask/tests/bazel_qualification.rs`, is the only authority that
decides whether a record qualifies. It is implemented no later than spec003w3.

Every threshold is derived from complete paginated Cargo, Bazel, and fixture
workflow-run inventories plus the record's immutable content references.
A workflow reference is (`runId`, positive `attempt`, `headSha`); content
references are a
commit reference (full SHA), a content reference (path plus digest), or a
generated-artifact reference (generated path plus digest). The validator:

- refuses pagination gaps, missing attempts, duplicate or conflicting run
  identities, and a record that omits an intervening protected-`v3` push;
- normalizes each run ID to its highest terminal attempt, pairs Cargo, Bazel,
  and fixture verdicts by head SHA, and derives mismatch resets rather than
  trusting a curated streak;
- derives the five newest qualifying cold records from the complete ordered
  stream rather than trusting a curated five-record subset;
- resolves each threshold by counting or comparing the referenced evidence, so
  a threshold such as ten matching records, twenty broker executions, eighteen
  isolated failures, five cold measurements, or four slice durations is a
  property of the references, never of a stated number;
- refuses an omitted reference for any threshold it must derive;
- refuses a forged or ill-formed reference: a run ID, SHA, digest, or path that
  does not parse in its declared shape, or a digest that does not match the
  referenced content;
- refuses duplicate references, whether repeated inside one threshold or reused
  across thresholds that must be independent;
- refuses inconsistent references, where two references that must agree
  disagree on head SHA, commit, digest, or count;
- refuses a wrong-candidate reference, where evidence that must bind the
  candidate commit binds a different commit.
- requires exactly one result for each of `crash-before-ready`,
  `crash-after-ready`, `crash-after-executed`, `crash-during-grace`,
  `direct-long-lived-descendant`, `double-forked-long-lived-descendant`, and
  `beyond-ceiling-pending-cleanup`; derives each closed
  `supervisorRecoveryClass`, `userspaceEscalationResult`, `cleanupResult`, and
  `quarantineResult`; and verifies exact lowercase SHA-256 patch, canonical
  monitor identity, pending-observation, and result digests;
- refuses any containment value carrying a raw PID, process-group ID,
  descriptor, path, process output, kernel text, command line, environment,
  handle, or opaque identity;
- requires passing omitted/duplicate/unknown-stage, wrong-recovery-class,
  malformed-digest, patch/monitor-mismatch, illegal-cleanup/quarantine,
  false-reaped, success-after-quarantine, quarantined-reuse, and
  forbidden-field mutation results. No summary count substitutes for those
  results.

`qualification.json` cannot qualify through a trusted boolean. Boolean and
count fields such as `qualified`, `eligible`, or any summary total are
informational mirrors of the derived result; a mirror that disagrees with the
derived result is a refusal, not a warning. The validator's own status output
is the verdict.

The command is `cargo xtask bazel-qualification-validate`. It takes no
arguments, reads the fixed repository-relative record path, is unreachable from
Make and every workflow, and is listed in the contributor-only command set of
`make-target-compatibility.md`. Evidence curation runs it before sealing the
record, promotion validation runs it against the sealed record at the promotion
candidate, and contributor validation runs it before any informational
inspection of the record.

`cargo xtask bazel-evidence refresh-qualification` is the paired
no-argument contributor-only correction command. It fetches the complete
protected stream through typed backends, rebuilds the fixed
repository-relative record from immutable references, and atomically replaces
only that record. It is unreachable from Make and workflows. A query failure
does not replace the prior record.

Every query degradation or validator refusal has one fixed code and exact
rendered command block:

| Refusal class | Code | Exact command block |
| --- | --- | --- |
| Cargo, Bazel, or fixture inventory query failed | `D2B-BZLQUAL-QUERY` | `git fetch origin v3`; `(cd packages && cargo xtask bazel-evidence refresh-qualification)`; `(cd packages && cargo xtask bazel-qualification-validate)`. |
| Pagination gap, missing attempt, or omitted protected push | `D2B-BZLQUAL-INVENTORY` | `git fetch origin v3`; `(cd packages && cargo xtask bazel-evidence refresh-qualification)`; `(cd packages && cargo xtask bazel-qualification-validate)`. |
| Omitted, malformed, or ill-formed reference | `D2B-BZLQUAL-REFERENCE` | `(cd packages && cargo xtask bazel-evidence refresh-qualification)`; `(cd packages && cargo xtask bazel-qualification-validate)`. |
| Duplicate or inconsistent reference | `D2B-BZLQUAL-CONSISTENCY` | `(cd packages && cargo xtask bazel-evidence refresh-qualification)`; `(cd packages && cargo xtask bazel-qualification-validate)`. |
| Wrong candidate or stale head | `D2B-BZLQUAL-CANDIDATE` | `git fetch origin v3`; `(cd packages && cargo xtask bazel-evidence refresh-qualification)`; `(cd packages && cargo xtask bazel-qualification-validate)`. |
| A derived qualification threshold is unsatisfied | `D2B-BZLQUAL-THRESHOLD` | Run the literal command selected by the closed threshold-class table below; `(cd packages && cargo xtask bazel-evidence refresh-qualification)`; `(cd packages && cargo xtask bazel-qualification-validate)`. |
| Degraded evidence | `D2B-BZLQUAL-DEGRADED` | Run the exact closed slice retry command carried by the degraded variant; `(cd packages && cargo xtask bazel-evidence refresh-qualification)`; `(cd packages && cargo xtask bazel-qualification-validate)`. |
| Atomic record replacement failed | `D2B-BZLQUAL-PUBLISH` | Correct write access to `specs/003-adr052-bazel-rust/evidence/qualification.json`; `(cd packages && cargo xtask bazel-evidence refresh-qualification)`; `(cd packages && cargo xtask bazel-qualification-validate)`. |

The threshold class is a closed enum, not record text:

| Threshold class | Exact correction |
| --- | --- |
| Main carrier, runner, manifest, or sink | `make test-bazel-rust-main` |
| API census | `make test-bazel-rust-api` |
| Broker topology or repetition | `make test-bazel-rust-broker` |
| Guest, schema, inventory, no-shell, or auxiliary carrier | `make test-bazel-rust-aux` |
| Package policy, yanked state, or compatibility census | `make test-rust-supply-chain` |
| Native realization or artifact contract | `make test-flake` |
| Workflow or source policy | `make test-policy` |
| Fixture companion | `D2B_ENABLE_FIXTURE_BUILD=1 make test-fixture-contracts` |
| Record-count or performance window | Complete a new protected `v3` merge. |

The renderer accepts no other command. Query errors are typed degraded
outcomes and are never interpreted as empty inventories. Messages include only
the fixed code, repository-relative record or policy row, SHA-256 references,
and the exact command block. They never include `$!`, an absolute, socket, or
Nix store path, raw pagination cursor, workflow run or attempt identifier,
candidate identifier, tag identifier, descriptor, OS error text, or raw
API/exporter/tool output. Table-driven tests cover every row and reject a
missing step, borrowed or free-form remedy, leaked value, or query error
reported as absence.

## Workspace and hub evidence

Qualification proves:

- one product resolver-v2 workspace and root lock;
- no broker or guest nested workspace or lock;
- a separate walker workspace and lock;
- `Cargo.guest.lock` absent from hub authority;
- only product and walker hubs;
- retired main, broker, and guest identifiers refusing with exact argv and cwd
  tests;
- first-party product crates represented by native targets;
- no `crate.spec`;
- product and walker repin outputs current.
- module refresh proving exact `MODULE.bazel.lock`-only mutation, second-run
  idempotence, absolute startup-option identity, exact drift remediation, and
  no Make or workflow reachability;
- the broker and guest Nix derivations retaining the exact
  `cargoLock.outputHashes."wl-proxy-0.1.2"` value.

## Package policy evidence

For each of broker GNU and guest musl on x86_64 and aarch64, qualification
contains:

- exact production and policy graph digests;
- exact selected root and nonempty closure;
- system, target, edge-kind, cfg, and feature checks;
- exact selected-source identity set and count;
- metadata and filtered-lock identity equality;
- source readability and checksum results;
- deny results over root-dev-inclusive metadata;
- pinned RustSec `--no-fetch` audit result;
- broker empty ignore and guest one-ignore assertion;
- closure leakage and forbidden dependency results.

The guest record proves exactly six package-scoped license exceptions and
proves a different package with the same licenses remains denied.

The yanked record proves one committed snapshot whose exact key set derives
only from `packages/Cargo.lock`, excluding the walker and
`Cargo.guest.lock`. It proves `rust-deny-main` checked the full product set and
the broker and guest carriers checked exact selected-policy-graph projections
against that same snapshot. It includes separate reviewed
`bazel-yanked-refresh` network observation and offline
`bazel-yanked-check` verdict references.

The supply-chain equivalence record contains, for main, broker, and guest, the
current Cargo raw enforcing exit status, the decomposed Bazel deny/audit/yanked
status, both sorted normalized finding sets, and an equality result. Main uses
the full product; broker and guest use exact selected policy projections. Any
status or finding difference makes the record unqualified and blocks both spec003w1
and promotion.

## Native architecture evidence

For each native runner, qualification contains realization references for
exactly these six checks:

```text
broker-production-dependency-policy
guest-shell-runner-static-dependency-policy
broker-production-package-policy
guest-real-libshpool-package-policy
broker-host-artifact-contract
guest-static-elf
```

It also proves:

- matching system and GNU or musl target;
- matching runner architecture;
- no foreign-system argument;
- no `--builders`;
- no remote builder;
- both generated system inventories current.
- four broker/guest-by-system artifact realizations;
- exact broker interpreter and `DT_NEEDED` SONAME set;
- broker and guest binary sizes checked against committed measured baselines
  and separately reviewed allowed deltas;
- exact selected Nix closure inventories and forbidden-sibling absence;
- `make test-rust-supply-chain` passed on the same native arm stable head as
  the six aarch64 realizations;
- the workflow renderer test covers that arm command and the PR head did not
  change between rendered-workflow validation and native evidence.
- every guest static ELF is `ET_DYN` for the native system's expected
  `e_machine`, with no `PT_INTERP` and no `DT_NEEDED`; non-PIE and
  wrong-machine plants are present and refused.

## Complete qualification

`qualification.json` is qualified only with:

1. exact eighteen-surface coverage;
2. ten consecutive matching qualification records;
3. eighteen isolated surface failures;
4. exact test, companion, scan, schema, and API censuses;
5. main and guest per-case topology and broker per-binary topology proof,
   including literal `tags = ["exclusive"]`, no overlap with any other test,
   and a passing tag-removal mutation;
6. twenty consecutive executions per broker context with
   `--runs_per_test=20` and exclusivity in force;
7. warm local, cold local, and cold CI performance sets;
8. complete package policy evidence above;
9. the x86 and arm six-check realization sets above;
10. `bazelRestoreCount`, `bazelSaveCount`, and `bazelPublicationCount` of
    zero in every shadow record, with five cold records each carrying four
    complete `sliceDurationsSeconds` entries;
11. complete locator and per-case evidence guards;
12. all workflow, cache, deadline, cleanup, repin, and seeded policy
    refusals.
13. all eight IPv4, IPv6, netlink, packet, pathname Unix, abstract Unix,
    socketpair, and io_uring pre-action plants denied; inherited socket,
    ordinary-ring, SQPOLL-ring, and fixed-socket-ring plants refused before
    load; exact patched-Bazel identity and startup capability, patch-removal,
    filter-load, setup-before-payload, compile/build, test, descendant, and
    strategy-fallback plants refused; external-egress and live-index refused;
    every configured-target, `aquery`, and strategy inventory complete; and
    every fetch outside governed actions, offline, and pinned;
14. exact Cargo/decomposed-Bazel supply-chain equivalence for all three
    contexts;
15. manifest/JUnit/bounded-test.log/emitted-evidence/exporter redaction,
    ignored-case, original-verdict, typed-degraded-status, no-shell evidence,
    and combined-budget mutations;
16. the committed `bazel/generated/no-shell-inventory.json` reference and
    digest, equal nonempty governed/declared sets, governed spawn sources,
    raw and unique scan-record counts each equal to the governed-source count,
    complete per-source scan records including zero-site sources,
    fresh-scan/committed spawn-site-key equality, and exactly
    `no-shell-inventory-empty`, `no-shell-inventory-missing-entry`,
    `no-shell-inventory-extra-entry`,
    `no-shell-inventory-unguarded-spawn`,
    `no-shell-inventory-missing-zero-site-record`, and
    `no-shell-inventory-planted-shell`;
17. a successful `cargo xtask bazel-qualification-validate` verdict derived
    from the references above.
18. manifest evidence that `test-flake-aarch64`, all four promoted Rust slice
    jobs, and the `test-rust` rollup are enforcing and not advisory, plus
    advisory-classification mutations for each class;
19. exact same-commit Cargo compatibility-carrier verdicts for every mandatory
    socket-using test, with every Bazel action retaining `actionNetwork =
    "none"`;
20. complete evidence-sink sanitization and bound results, with no forbidden
    planted value in JUnit, `test.log`, emitted evidence, or exporter
    diagnostics, `junit-v1`, `test-log-v1`, `evidence-v1`, and
    `exporter-diagnostic-v1` age/count retention enforced, and no degraded
    evidence in the qualified set;
21. exactly four artifact-baseline row digests, all four artifact realization
    results, and every nonzero size-growth authorization digest and positive
    and negative authorization fixture;
22. exact Bazel 8.6.0 source, Linux sandbox patch, fixed-policy, output NAR,
    executable, and capability-ABI hashes; startup capability result;
    configured-target plus `aquery` stable/nightly action-kind and sandbox
    strategy inventories; patch-removal, wrong-output, filter-load, and
    setup-before-payload results; inherited socket/ring/SQPOLL/fixed-socket
    preflight; every closed stage diagnostic; all eight pre-action plants; and
    no process/local/standalone/worker/remote fallback; fresh PID-namespace
    monitor identity; crash-before-`READY`, crash-after-`READY`,
    crash-after-`EXECUTED`, crash-during-grace, direct and double-forked
    long-lived-descendant results; fixed userspace containment ceiling; one
    beyond-ceiling `pending-kernel-cleanup` plant with owned quarantine,
    no-success/no-reuse, no false-reaped claim, and consuming reap by the
    original live monitor; byte-exact pending/runbook-link/release results and
    a resolved
    `docs/contributing/critical-subsystems.md#bazel-pending-kernel-cleanup-quarantine`
    locator; and namespace/teardown-patch/ceiling/quarantine/reboot/
    retry-before-release/replacement-waiter/manual-release/fallback mutation
    results.
23. exact immutable static C `d2b-bazel-exec-supervisor` derivation,
    dependency-closure, output NAR/executable, source, and protocol hashes;
    one-site Rust invocation policy; private-fd identity; descriptor absence;
    CLOEXEC and stdin results; single-record exec-error plus stateful framed
    `READY`/`EXECUTED`/terminal transport; fragmented/coalesced and
    malformed/duplicate/order results; held-open-writer, closed-reader `EPIPE`,
    exact partial-I/O, fast-same-status, waitable `SIGCHLD`, safe serialized
    spawning-thread mask capture/block/exact restoration after both spawn
    outcomes, capture/block/poison/restoration failures, overlapping-launch
    one-guard and restore-before-unlock mutations, inherited managed `SIG_IGN`
    refusal, handoff-window/normalization-time `SIGTERM`, child/supervisor
    setpgid and initial-stop races, typed
    `ESRCH`/`EPERM`/early-child-exit cleanup, exact four-argument
    `TRACEME`/initial-stop/`SETOPTIONS`/`CONT`/event/`DETACH` order,
    request/pid/address/data position tests and omission/exchange/wrong-pid/
    options-in-address/nonzero-signal mutations, pending signal before
    group/trace confirmation,
    pre-`READY` termination ownership, deterministic post-`READY` pre-exec
    signals, one setup request, pre-exec death/fault/OOM-like kill, empty EOF
    without event, missing/wrong event, detach failure, fast first-instruction
    exit, helper group kill/reap, distinct pre-helper Nix/toolchain/sandbox
    system/kernel/Yama/probe/policy codes, fixed inputs, byte-exact remedies,
    phase-valid reruns and wrong-remedy results, distinct helper
    stop/options/continue/event/detach codes, exact four-request ptrace seccomp
    allowance with unchanged no-network,
    no pre-exec forwarding/grace/`EXECUTED`/target terminal/audit, and post-exec
    forwarding, no-deadline external-TERM escalation, target-ignore-TERM, absence of
    numeric Rust signaling; and every Rust-parent and C-supervisor
    ownership/closure/cleanup/wait/reap result; plus every parent/helper/child
    runner recovery-code byte-exact result and every patched-sandbox-owned
    sandbox recovery-code byte-exact result for the phase-valid diagnostic
    command version.
24. exactly seven bounded PID-namespace containment result rows with closed
    supervisor recovery, userspace escalation, cleanup, and quarantine
    values; matching sandbox patch and canonical monitor identity digests;
    the pending-observation digest where required; no raw PID, descriptor,
    path, process output, or opaque identity; and every required containment
    validator mutation result.

Candidate-specific evidence binds one integrated commit. A content change
invalidates affected evidence. The qualified record merges before promotion
and is immutable afterward.

## Promotion and retirement

Promotion references the qualification digest. The promotion record captures
the promotion SHA, cache maintenance and save, first promoted verdict, and
rollback rehearsal. `promotion-record.json` is created after the promotion
merge; the pre-merge rollback rehearsal therefore resolves its candidate from
the verified current atomic candidate HEAD and the recorded spec003w5 parent,
and no pre-merge step reads the promotion record.

After merge, `cargo xtask bazel-promotion-record-validate` reads the fixed
promotion record and the sealed `spec003w5` delivery record. It requires the
recorded promotion SHA to equal the actual protected-`v3` merge commit reported
by the merged pull request, requires that commit to be reachable from
`origin/v3`, re-derives the sealed content and candidate identities from the
merge, and requires them to equal the seal. A pre-merge candidate SHA, an older
containing SHA, an unsealed merge, a seal for another candidate, and a wrong
pull-request merge SHA each fail. The command has no override and runs before
`spec003w5fu1` seals, before alias-removal entry, and before Cargo-retirement
entry.

Promotion documentation and its semantic changelog fragment list the exact
surface IDs whose mandatory socket-using tests remain Cargo compatibility
carriers. They call those surfaces permanently hybrid under this
specification, state that spec003w7 retains the cases and their public
executor, and name separate authorization as the only retirement path.

Post-promotion run-unit inventory keeps independent release-containment and
green-run clocks. Alias removal depends only on containment in a published
semantic release tag matching `v<major>.<minor>.<patch>`. Cargo implementation
retirement eligibility depends only on ten distinct ordered green promoted
`v3` run units. Its qualification and code preparation may run first, but its
shared documentation/evidence task and merge depend on merged alias removal,
then rebase, rerun complete validation, and obtain a new ten-seat panel result.
Neither removes a public Rust Make name.

## Typed post-promotion run units

The validator paginates the authoritative workflow-run API to completion into
a transient protected stream and inventories every promoted protected-`v3`
`test-rust` run unit. Eligibility is derived from that complete transient
stream on every run. `post-promotion.json` is not the input authority and does
not persist the complete stream.

The persisted record contains only a fixed-shape checkpoint
(`paginationState = "complete"`, `pageCount`, `streamCount`,
complete-stream SHA-256, promotion SHA, and validation time), the final ten
normalized run units needed for the eligibility decision, and for each
persisted unit an attempt-history count and SHA-256 rather than the attempt
array. It persists no raw page token or cursor. The schema sets a fixed
maximum record count and byte size and rejects unknown fields or overflow
before atomic rename. Refreshing evidence replaces the prior bounded record;
it never appends.
Tests feed a complete transient stream larger than the persistence limit,
derive the same verdict as the unbounded in-memory oracle, and prove the saved
file remains within both limits.

A run unit is one distinct push-created `(runId, headSha)` pair. An attempt is
never a unit and never a streak position.

Each transient unit contains:

- immutable `runId` and `headSha`, whose pair is the unit identity;
- `event`, exactly `push`;
- `branch`, exactly `v3`;
- `attempts`, the complete nested history `1..maxAttempt` with each
  attempt's `conclusion`, `runStartedAt`, and `completedAt`;
- `conclusion`, normalized to the conclusion of the highest terminal attempt;
- `createdAt`, the immutable creation timestamp of the unit; and
- `promotionAncestor`, derived by verifying the promotion commit is an
  ancestor of `headSha`.

Ordering is ascending `(createdAt, runId)`. `runStartedAt` is never an
ordering input: a rerun updates it, and ordering by it would let an old rerun
move behind newer failures and silently repair a broken streak. Attempt
timestamps order attempts inside a unit only.

Pagination page and cursor continuity are validation inputs, not persisted
eligibility claims. Refusals are: missing pages; a unit missing any attempt in
`1..maxAttempt`; attempts of one unit carrying conflicting `headSha`,
`event`, `branch`, or promotion provenance; repeated or missing unit
identities; non-v3 or non-push runs; pre-promotion ancestry; and a highest
attempt with a nonterminal conclusion. Any terminal non-success conclusion,
including failure, cancellation, timeout, or startup failure, resets the
streak.

The validator computes the reset positions and current consecutive-success
streak from the complete ordered unit stream, counting each unit exactly once.
It never reads or trusts `eligible`, `consecutive_green_count`, or
`green_run_ids` fields. Retirement requires the derived final ten distinct
ordered units to be successes with no intervening failure or cancellation.

Two fixtures are mandatory:

- a repeated-attempt fixture, where one unit has several successful attempts
  and contributes exactly one streak position; and
- an old-rerun-after-failure fixture, where a unit created before a later
  failing unit is rerun successfully after that failure and still orders before
  it, leaving the streak reset in place.
