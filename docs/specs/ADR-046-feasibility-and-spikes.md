# ADR 0046 feasibility and spikes

| Field | Value |
| --- | --- |
| Spec ID | `ADR-046-feasibility-and-spikes` |
| Parent | ADR 0046 |
| Status | Proposed |
| Version | 1 |
| Baseline | `b5ddbed67867d9244bf33390868101bd9b053e49` |
| Normative | Yes |
| Owners | ADR 0046 integrator; validation/delivery; spike/proof authors |
| Depends on | `ADR-046-decision-register`, `ADR-046-terminology-and-identities`, `ADR-046-resource-store-redb`, `ADR-046-resource-reconciliation`, `ADR-046-componentsession-and-bus`, `ADR-046-components-processes-and-sandbox`, `ADR-046-provider-model-and-packaging`, `ADR-046-resources-credential`, `ADR-046-resources-volume`, `ADR-046-resources-host-guest-process-user`, `ADR-046-provider-state`, `ADR-046-zone-routing`, `ADR-046-nix-configuration`, `ADR-046-cli-and-operations`, `ADR-046-current-code-migration-map` |
| Supersedes | None |

## Purpose

ADR 0046 is documentation-only (D024): the parent and every `ADR-046-*` spec
describe a Proposed design, not a running system. This spec is the single
place that says, for every hard numeric target and every cross-process
contract in that design, exactly which claims already have executable proof,
which claims require a new disposable proof before any implementation work
item may begin, and which claims can only be closed by the production
implementation's own conformance/bench suite once that work item exists.

It does three things and nothing else:

1. Separates **completed evidence** (work already executed, with numbers) from
   **mandatory disposable spikes** (work item authors must run these, or an
   equivalent superseding proof, before starting the named production work
   items) from **implementation validation** (the permanent test/bench
   coverage that eventually replaces the disposable spike inside the real
   crate).
2. Defines every mandatory spike as an exact, reproducible artifact: fixed
   inputs, an exact command/harness, fixed metrics, a numeric pass/fail
   threshold, an expected resource budget, a stated failure interpretation,
   the exact decisions/work items the result binds, and a cleanup rule.
3. Fixes the evidence-classification vocabulary and anti-claim rules that
   every other ADR 0046 spec's "Feasibility proof" row must honor, so a green
   spike can never be cited as if it were a green production test.

This spec creates no crate, dependency, Nix module, service, controller, or
Provider. It defines proof artifacts that live outside the production tree
(`proofs/<spike-slug>/`, matching the existing `proofs/w0-ch-connect-proof`
and `proofs/chunked-stdio-conformance` convention validated by
`make test-proofs` / `tests/test-proofs.sh`) until the owning work item below
promotes the proven behavior into a permanent in-tree test. No such crate is
created by this spec; authoring them is future implementation work, exactly
like every other `ADR046-*` work item in the set.

## Evidence classification and anti-claim rules

Every claim in this spec and in every spec it depends on carries exactly one
of the six classes defined in `docs/specs/README.md`:

| Class | Meaning here |
| --- | --- |
| `implemented-and-reachable` | A live call path exists in a production binary today, with committed tests exercising it. |
| `implemented-but-unwired` | Code exists (in v3 or in main) and its own tests pass, but no v3 production caller invokes it yet. |
| `generated-or-eval-contract` | Produced by Nix/xtask codegen; validated by an eval/build/drift gate, not by a running process. |
| `test-only-or-preview` | Exercised only by test code or an exploratory branch; no production caller. |
| `ADR-only` | Described by an `ADR-046-*` spec; no code exists anywhere at any commit. |
| `unknown-requires-spike` | The claim is plausible but has no executable evidence at any commit; a spike is mandatory before any owning work item starts. |

### Anti-claim rules (binding on this spec and every spec that cites it)

1. **A spike changes only this spec's own spike-status field.** A passing
   spike proves the named hypothesis under the named harness. It never
   changes the evidence class recorded for the corresponding v3 production
   artifact in any other spec's "Current-code fit" table. That table's
   `ADR-only` / `unknown-requires-spike` entries move to
   `implemented-and-reachable` only when the exact named work item lands in
   `packages/` with its own committed tests passing in this repository's CI.
2. **Main reuse evidence never upgrades v3 reachability.** Per
   `docs/specs/README.md` and decision D041, a passing main-commit test suite
   (see §"Already-proven main evidence" below) proves the *borrowed
   behavior's* correctness at the cited main commit. It is
   `implemented-but-unwired` evidence for the *copy/adapt* work item, not
   `implemented-and-reachable` evidence for v3. No claim in this document
   states or implies that v3 already has a running ComponentSession, bus,
   resource store, or Provider process.
3. **A failing spike changes the Proposed design, never the target.**
   Mirroring `ADR-046-resource-store-redb`'s performance-contract rule,
   generalized to every spike in this document: if a spike's measured value
   misses its threshold, the affected spec's Proposed design changes (schema,
   algorithm, batching, concurrency limits, or component boundary). No spike
   result may be used to relax a durability, authorization, audit, redaction,
   or security invariant defined by any `ADR-046-*` spec in order to pass.
4. **Rejected-alternative evidence is retained narratively, not
   re-executed.** The completed kcp spike (§"Completed evidence") is retained
   as historical numeric evidence supporting decision D003. No work item in
   this set depends on kcp; nothing in the set requires re-running or
   re-verifying that measurement, and doing so would validate a rejected
   alternative rather than a required capability.
5. **A spike never substitutes for the panel or for implementation
   validation.** Per D024 this task's initial output is documentation only,
   and per the parent ADR's review process a ten-role panel runs only against
   an integrated implementation candidate. Nothing in this document runs, or
   claims to have run, that panel. Every spike below is additionally
   scheduled to be **subsumed and then deleted**: once the named production
   work item exists with its own in-tree conformance/bench test achieving
   equal or stricter coverage, the disposable `proofs/<slug>/` crate is
   deleted and `tests/test-proofs.sh` drops its entry. Until that day, the
   spike's continued presence under `proofs/` is not evidence that the
   production behavior exists.
6. **Every spike is independently reproducible.** Each spike below pins its
   Rust/redb/snow/tokio dependency versions (or points at the existing
   `packages/rust-toolchain.toml` pin), states its exact command, and avoids
   wall-clock nondeterminism in pass/fail logic (latency spikes use
   monotonic clocks and a fixed percentile estimator over a fixed, stated
   sample count; no spike's threshold depends on unpinned background load).
7. **No unresolved entries.** Every spike below has a concrete status. Status
   `not yet executed` is a factual statement about sequencing (D024: this PR
   is documentation only; execution is separate future implementation work),
   not an unresolved decision, missing threshold, or placeholder. Every
   numeric threshold, budget, and command in this document is final.

## Completed evidence

These items already have executable results. They are evidence, not spikes:
nothing further needs to run before the affected decisions can be treated as
settled for planning purposes. Re-running them is not a prerequisite for any
work item below.

### E1 — kcp runtime-substrate rejection (decision D003)

| Field | Value |
| --- | --- |
| Claim | A kcp-based resource plane (typed spec/status, revisions, watches, optimistic conflicts, owner/finalizer behavior, hierarchy, controller clients) is unsuitable for recursive Host/Guest/Zone embedding at d2b's scale. |
| Measured result | Approximately 490 MiB RSS and a 176 MiB executable for a minimal kcp-based control plane instance. |
| Source of the number | Parent ADR (`docs/adr/0046-d2b-3-provider-control-plane.md`, "A kcp feasibility spike proved ... measured approximately 490 MiB RSS and a 176 MiB executable") and `ADR-046-decision-register` entry D003 ("The spike proved desired object semantics but measured about 490 MiB RSS and a 176 MiB executable."). |
| Original artifact location | Not present in this repository at any inspected commit (`b5ddbed6`, `a1cc0b2d`, or main `HEAD`); the measurement is retained only as narrative evidence in the two citations above. |
| Semantics proven useful and retained | Typed spec/status, revisions, watches, optimistic conflicts, owner/finalizer behavior, resource hierarchy, and controller-client ergonomics — all carried into the native `ADR-046-resource-object-model` / `ADR-046-resource-store-redb` / `ADR-046-resource-reconciliation` design without the Kubernetes-API/etcd/workqueue machinery. |
| Consequence | D003: kcp is rejected as the v3 runtime substrate. D004-D006: one embedded redb database per Zone, one writer with concurrent MVCC readers, embedded in the Zone runtime process, replaces it. |
| Reproducibility disposition | Not reproduced by this spec (anti-claim rule 4). Any future dispute about the number must re-derive it against a stated kcp version/build profile as a new, separately reviewed spike; this spec does not schedule that work because no ADR 0046 work item depends on it. |

### E2 — redb selection rationale (decisions D004, D005, D006)

redb is selected as the per-Zone embedded store on **design-level**, not
**workload-level**, grounds:

- **Embedded, pure Rust, no separate server process** — avoids the
  kcp-class footprint measured in E1 (no separate apiserver/etcd binary, no
  gRPC/HTTP control-plane surface to run per Zone).
- **ACID transactions with one writer and concurrent MVCC readers** — matches
  D005 (single-writer optimistic-conflict model) without an external lock
  manager.
- **Crash-safe B-tree storage with an `FileBackend::new(File)`-shaped API** —
  matches `ADR-046-resource-store-redb`'s "Ownership and process boundary"
  requirement that the store never resolves a caller-controlled path; the
  storage owner passes one already-open `File`.
- **No wire protocol of its own** — every client access is mediated by
  `d2b-bus`/ComponentSession per D011; redb is never reachable except through
  the one embedding Zone runtime process (D006).

This rationale is a completed **design** decision (D004-D006 are Resolved,
not `decision-required`). It is explicitly **not** a completed **workload**
proof: no committed evidence in this repository demonstrates that a redb
database built against the exact `ADR-046-resource-store-redb` schema (§
"Physical tables") meets the aggregate ≤64 MiB idle-RSS, ≤500 ms readiness,
≤2 ms p95 read, ≤10 ms p95 write, ≤5 ms p95 commit-to-handler, or ≤20 ms p95
ready-Process-to-launch-attempt targets in that spec's "Performance contract"
table. Those targets are `unknown-requires-spike` and are covered by
SPIKE-01 and SPIKE-02 below. Pinned version for every redb-touching spike in
this document:

```toml
redb = { version = "=4.1.0", default-features = false }
# crates.io checksum (sha256, as published):
# 8e925444704b5f17d32bf42f5b6e2df050bceebc3dcd6e71cc73dafe8092e839
# upstream: https://github.com/cberner/redb ; rust_version 1.89 (satisfied by
# the pinned packages/rust-toolchain.toml channel 1.94.1); edition 2024.
```

### E3 — main a1cc0b2d ComponentSession/Noise/Unix/vsock transport re-verification

`ADR-046-componentsession-and-bus` cites main commit `a1cc0b2d` (`ADR-0045
W9: coordinate toolkit and sibling cutover (#314)`) as the exact reuse source
for `d2b-session` and `d2b-session-unix`. That commit's own test suites were
re-executed during the authoring of this spec, in a disposable detached
worktree outside any tracked branch, to confirm the cited behavior is
presently green and not merely narratively true:

```bash
# Reproduction (main a1cc0b2d only; never run against this v3 branch, which
# does not contain these crates):
git worktree add --detach /path/scratch a1cc0b2d
cd /path/scratch/packages
cargo test -p d2b-session-unix -p d2b-session --locked
cargo test -p d2b-session-unix --test unix_session --features host-socket --locked
cargo test -p d2b-session-unix --lib --features host-socket,native-vsock --locked
git worktree remove /path/scratch --force   # cleanup; no artifact retained
```

| Suite | File | Feature flags | Result |
| --- | --- | --- | --- |
| `d2b-session` unit tests | `packages/d2b-session/src/{driver,server}.rs` inline `#[cfg(test)]` | default | 2 passed |
| `d2b-session` integration | `packages/d2b-session/tests/component_session.rs` | default | 27 passed |
| `d2b-session` Noise vectors | `packages/d2b-session/tests/noise_vectors.rs` | default | 1 passed (`every_canonical_w2_vector_verifies_exactly_with_snow_0_10`) |
| `d2b-session-unix` integration | `packages/d2b-session-unix/tests/unix_session.rs` | `host-socket` | 25 passed |
| `d2b-session-unix` unit tests (incl. vsock) | inline `#[cfg(test)]` in `adapter.rs`, `socket.rs`, `systemd.rs`, `vsock.rs` | `host-socket,native-vsock` | 15 passed |
| **Total** | | | **70 passed, 0 failed** |

Toolchain used: `cargo 1.95.0` / `rustc 1.95.0` (nix-profile), against the
repository's pinned channel `1.94.1` (`packages/rust-toolchain.toml`) — a
newer patch/minor toolchain than pinned, disclosed here for transparency.
Anyone re-running this reproduction with the exact pinned `1.94.1` toolchain
is expected to reproduce the same pass count; a mismatch is itself a finding
to be filed against `ADR046-session-001`/`ADR046-session-002`, not silently
reconciled.

**Evidence class implication:** this makes the *cited main behavior itself*
(Noise NN/KK/IKpsk2 handshake, directional record protection, replay/sequence
rejection, fair scheduling, named-stream credit, Unix seqpacket/stream
transport, pidfd/SO_PEERCRED identity evidence, in-memory vsock framing) a
demonstrated, currently-green implementation shape — this is what the
`Reuse action: copy and adapt` in `ADR046-session-001`/`ADR046-session-002`
inherits. Per anti-claim rule 2, it remains `implemented-but-unwired` for v3:
no ComponentSession exists in the `b5ddbed6` v3 baseline, and copying these
70 tests into `packages/d2b-session*` at their new v3 destination — then
adding the subject/RBAC/revocation/resource-watch/latency integration tests
those work items require — is exactly the validation column of
`ADR046-session-001` and `ADR046-session-002` in `ADR-046-componentsession-and-bus`,
not a step this spec performs.

### E4 — current v3 reachable-evidence inventory

These v3 (`b5ddbed6`) files exist at the exact paths cited by their owning
specs and were confirmed present during authoring of this spec. They are
`implemented-and-reachable` or `production-reachable` (per
`ADR-046-current-code-migration-map` §0 notation) evidence for the
*current-role* behavior being migrated, not for any ADR 0046 resource,
controller, or Provider (all of which remain `ADR-only` until their own work
item lands):

| Path | Cited by | Confirmed |
| --- | --- | --- |
| `packages/d2b-core/src/storage.rs` (440 lines) | `ADR046-store-001` | present, contains `#[cfg(test)]` coverage |
| `packages/d2b-core/src/sync.rs` (232 lines) | `ADR046-store-001` | present, contains `#[cfg(test)]` coverage |
| `packages/d2bd/src/supervisor/dag.rs` | `ADR046-reconcile-001`, `ADR046-reconcile-003` | present |
| `packages/d2bd/src/supervisor/pidfd.rs` | `ADR046-reconcile-003` | present |
| `packages/d2b-priv-broker/src/ops/spawn_runner.rs` | `ADR046-process-001` | present |
| `packages/d2b-priv-broker/tests/pidfd_real_spawner.rs` | `ADR046-process-001` | present |
| `packages/d2b-realm-router/src/{session,secure_session,mux_session,session_lifecycle,target_resolver,execution,remote_node,display_transport}.rs` | `ADR-046-componentsession-and-bus`, `ADR-046-zone-routing` | present |

Per `ADR-046-resource-reconciliation`'s own current-code-fit row, the DAG/
pidfd/route logic in these files is tested for its **current** role-specific
behavior; the **generic** async controller loop, store watches, owner
triggers, and cross-resource concurrency described by ADR 0046 do not exist
in any of them and remain `ADR-only`.

## Evidence classification matrix

| Subsystem / claim | Evidence class today | Spike required before its work item starts |
| --- | --- | --- |
| kcp unsuitable at recursive Host/Guest/Zone scale | completed evidence (E1) | No — see anti-claim rule 4 |
| redb is the right embedding shape (single-writer/MVCC/pure-Rust/fd-backed) | design rationale, `ADR-only` for workload fit (E2) | Yes — SPIKE-01 |
| redb meets the 10k-resource functional/index/revision/watch/group-commit/crash-recovery/RSS targets | `unknown-requires-spike` | Yes — SPIKE-01 |
| p95 durable commit → controller handler start ≤5 ms | `unknown-requires-spike` | Yes — SPIKE-02 |
| p95 ready-Process commit → launch-attempt start ≤20 ms under concurrent dispatch | `unknown-requires-spike` | Yes — SPIKE-03 |
| Async EffectPort adapters never block the executor | `unknown-requires-spike` | Yes — SPIKE-04 |
| Noise NN/KK/IKpsk2 handshake, records, fair scheduling, Unix/vsock transport (main shape) | `implemented-but-unwired` for v3, proven at main `a1cc0b2d` (E3) | Only the v3 integration delta — SPIKE-06/07/08 |
| Independent Provider crate → multiple binaries → manifest/config-schema/component registration | `ADR-only` | Yes — SPIKE-05 |
| d2b-bus exact-addressed routing with per-recipient distinct Noise sessions | `ADR-only` | Yes — SPIKE-06 |
| Unix/vsock/Azure-Relay transports carry only opaque Noise record bytes | `ADR-only` for v3 wiring; per-transport performance gates already specified in each dossier | Yes — SPIKE-07 |
| Credential Provider → consumer Provider raw token/signature delivery over dedicated Noise_KK session | `ADR-only` | Yes — SPIKE-08 |
| Optional declared state-Volume creation order, guest-local vs. host-backed-guest, virtiofs `Export` child | `ADR-only` | Yes — SPIKE-09 |
| Volume ACL/`sourcePolicyId`/quota/marker/adoption policies | `ADR-only` | Yes — SPIKE-10 |
| systemd/minijail Process Providers: pidfd acquire/verify/reap/adopt/quarantine conformance | `ADR-only` | Yes — SPIKE-11 |
| Nix direct ResourceSpec authoring → codegen → build validation → removed-resource cleanup | `generated-or-eval-contract` pattern proven by existing `nixos-modules/assertions.nix` + `xtask gen-schemas` + drift gate machinery; ADR 0046 schema set is `ADR-only` | Yes — SPIKE-12 |
| CLI dynamic Provider-projection discovery with bounded deadline/size | `ADR-only` | Yes — SPIKE-13 |
| Clean v3 reset/cutover (no v2 alias dispatch, fresh Zone bootstrap) | `ADR-only` | Yes — SPIKE-14 |
| Representative local/cloud/interaction Provider end-to-end composition | `ADR-only` | Yes — SPIKE-15 |
| Three-layer status shape: schema parity across implementations, base-only projection, extension versioning/unknown-field | `ADR-only` | Yes — SPIKE-16 |
| Current v3 storage/DAG/pidfd/spawn_runner/router files exist and pass their own tests | `implemented-and-reachable`/`production-reachable` for current role (E4) | No — already evidenced |

## Mandatory disposable spike catalog

Every spike below lives at `proofs/<slug>/`, is a standalone
`[workspace]`-rooted crate (matching `proofs/w0-ch-connect-proof/Cargo.toml`
and `proofs/chunked-stdio-conformance/Cargo.toml`), is **not** a member of
`packages/Cargo.toml`, and is run with
`cargo test --manifest-path proofs/<slug>/Cargo.toml` (plus
`cargo bench --manifest-path proofs/<slug>/Cargo.toml` where the metric is a
latency/throughput percentile) using the pinned `packages/rust-toolchain.toml`
channel. None of these crates exist yet; authoring one is the first step of
its listed work item in "Implementation work items" below, not a step
performed by this documentation-only spec.

### SPIKE-01 — redb functional scale, indexes, revisions, watch, group commit, crash recovery, RSS

| Field | Value |
| --- | --- |
| Hypothesis | An embedded redb 4.1.0 database, built against the `ADR-046-resource-store-redb` physical schema (`store_meta`, `api_schemas`, `resources`, `type_index`, `owner_index`, `controller_index`, `revision_log`, `operations`, `zone_link_cursors`), sustains 10,000 resources with 100 live watches, correct revision/index maintenance, bounded group commit, and forced-crash recovery, inside the aggregate ≤64 MiB idle-RSS budget shared with the fixed system-core/system-minijail controllers. |
| Minimal disposable artifact | `proofs/redb-resource-store-spike/` — a standalone crate implementing exactly the eight tables above over `redb::Database` with a `FileBackend::new(File)`-backed open, an async bounded fair write queue feeding one blocking store-actor thread (via `tokio::task::spawn_blocking`), and a minimal watch registrar replaying `revision_log`. No d2b-bus, no ComponentSession, no broker — a fake in-process caller drives the API directly. |
| Inputs | (a) empty store; (b) 10,000 pre-seeded resources across 6 synthetic ResourceTypes with realistic key/value sizes (JSON spec/status ≤4 KiB each, matching `ADR-046-resource-object-model` bounded-message expectations); (c) 100 concurrently registered watches with mixed ResourceType filters; (d) an expected-revision conflict storm of 500 concurrent writers targeting 50 shared resources; (e) an owner-trigger fan-in tree 4 levels deep, 8 children per level; (f) `SIGKILL` injected at each of the 13 commit-transaction boundaries listed in the write-transaction algorithm. |
| Command/harness | `cargo test --manifest-path proofs/redb-resource-store-spike/Cargo.toml -- --test-threads=1` for functional/index/revision/watch/conflict/owner-trigger/compaction cases; `cargo run --manifest-path proofs/redb-resource-store-spike/Cargo.toml --bin crash-fixture -- --kill-at-txn <n>` (13 invocations, one per boundary, each in a fresh subprocess) for crash recovery; `/usr/bin/time -v cargo run --manifest-path proofs/redb-resource-store-spike/Cargo.toml --bin rss-fixture -- --resources 10000 --watches 100` for RSS, read three times and take the median "Maximum resident set size (kbytes)". |
| Metrics | (1) resource/index/revision correctness — exact match of 10k resources against a parallel `BTreeMap` oracle after every mutation; (2) watch replay/live no-gap — every one of 100 watchers receives every committed ChangeBatch entry after its `afterRevision`, verified by a monotonic per-watcher received-revision set with no gap; (3) group-commit batch size distribution under the conflict storm; (4) crash-recovery: process re-open succeeds or fails closed (never silently creates an empty replacement) at all 13 boundaries; (5) median RSS in KiB. |
| Pass/fail threshold | (1) zero divergence from oracle across 10k resources / 5 repeated runs; (2) zero missed/duplicated ChangeBatch deliveries across 100 watchers; (3) non-conflicting writes in the storm achieve group commit (batch size > 1) at least 50% of the time; (4) 13/13 boundaries either recover to the last fully-committed state or refuse to open (never a silent empty store); (5) median RSS for the store+actor alone ≤ 24 MiB (leaving ≥40 MiB of the 64 MiB aggregate budget in `ADR-046-resource-store-redb` for the fixed system-core/system-minijail controllers measured separately in SPIKE-11/SPIKE-15). |
| Expected resource budget | Single-threaded build+run ≤5 minutes on a 4-vCPU/8 GiB CI runner; peak build RSS ≤1 GiB; on-disk database file ≤200 MiB for the 10k-resource fixture. |
| Failure interpretation | RSS miss → the schema/serialization plan in `ADR-046-resource-store-redb` §"Physical tables" changes (e.g., narrower key encoding, smaller in-memory index shape) before `ADR046-store-001` starts; a correctness miss on watch/crash-recovery → the async storage adapter or write-transaction algorithm in that same spec is revised, never the tolerance; group-commit batch-size miss → the "bounded group commit" admission window is retuned, but per anti-claim rule 3 not by weakening per-mutation validation. |
| Affected decisions/work items | D003, D004, D005, D006, D008, D053; `ADR046-store-001`, `ADR046-store-002`, `ADR046-store-003`. |
| Cleanup | `proofs/redb-resource-store-spike/` is deleted, and its entry removed from `tests/test-proofs.sh`, once `packages/d2b-resource-store-redb` (the real `ADR046-store-001` destination) has an in-tree benchmark reproducing metrics (1)-(5) at equal or stricter thresholds. |
| Status | Specified — not yet executed (D024: documentation-only task; execution is separate future implementation work). |

### SPIKE-02 — durable commit → controller handler start, p95 ≤5 ms

| Field | Value |
| --- | --- |
| Hypothesis | The post-commit dispatcher inside the Zone runtime (redb write-transaction commit → in-memory index swap → matching watch/reconcile-hint push) delivers a hint to a waiting async consumer with p95 latency ≤5 ms, independent of concurrent unrelated commit traffic. |
| Minimal disposable artifact | `proofs/redb-commit-handler-latency-spike/` — extends SPIKE-01's store-actor with a minimal in-process "hint bus" (a bounded `tokio::sync::mpsc` per registered consumer, no real d2b-bus/ComponentSession) and a synthetic "controller" task that records `Instant::now()` on hint receipt. |
| Inputs | 1,000 sequential single-resource writes each immediately followed (same async task) by measuring elapsed time to the matching consumer's hint receipt; repeated under three concurrency profiles: (a) no background writers; (b) 10 background writer tasks issuing unrelated resource writes at a combined 500 writes/s; (c) 100 background writer tasks at a combined 2,000 writes/s. |
| Command/harness | `cargo bench --manifest-path proofs/redb-commit-handler-latency-spike/Cargo.toml -- commit_to_handler` using `criterion` with `--sample-size 1000`; percentile computed by criterion's built-in estimator over the full 1,000-sample run per profile. |
| Metrics | p50/p95/p99 elapsed time in microseconds from `write_transaction.commit()` return to consumer task waking and reading the hint, for each of the 3 concurrency profiles. |
| Pass/fail threshold | p95 ≤5,000 µs (5 ms) in all 3 profiles; p99 ≤10,000 µs recorded and reported (not gating, but any p99 >20 ms is a documented finding attached to the result). |
| Expected resource budget | ≤3 minutes wall time per profile; single CI runner core pinned via `taskset`/`cset` where available, otherwise best-effort with reported CPU count and load. |
| Failure interpretation | A miss under profile (a) is a dispatcher-design failure — the post-commit swap/push path in `ADR-046-resource-store-redb` §"Async storage adapter" changes. A miss only under profiles (b)/(c) is an admission-fairness failure — the "per-principal/controller fair admission" rule in the same section is retuned (e.g., smaller max group-commit batch, dedicated hint-delivery task priority) before `ADR046-store-002` starts. |
| Affected decisions/work items | D030; `ADR046-store-001`, `ADR046-store-002`, `ADR046-reconcile-002`. |
| Cleanup | Deleted once `packages/d2b-controller-toolkit/benches/reaction.rs` (the real `ADR046-reconcile-003` destination named in `ADR-046-resource-reconciliation`) reproduces the same 3-profile p95/p99 gate against the real store. |
| Status | Specified — not yet executed. |

### SPIKE-03 — ready Process commit → launch-attempt start, p95 ≤20 ms, concurrent reading and dispatch

| Field | Value |
| --- | --- |
| Hypothesis | When a Process resource is durably committed with all dependencies Ready, an independent async controller task reaches "launch-attempt start" with p95 ≤20 ms, and — critically — the watch receiver dispatches the *next* independent ready Process without waiting for any in-flight launch, readiness wait, or long-running effect to complete. |
| Minimal disposable artifact | `proofs/process-fastlaunch-spike/` — builds on SPIKE-02's hint bus, replaces the synthetic consumer with a minimal async "Process controller" loop matching `ADR-046-resource-reconciliation` §"Async loop" (steps 1-14: register, list, watch, per-resource single-flight dispatch, parallel independent resources under a semaphore) and a fake `ProcessLaunchEffectPort` whose `spawn()` sleeps a configurable 0-500 ms to model a slow real launch without touching any real process/broker/systemd/minijail code. |
| Inputs | 1, 10, and 100 concurrently-ready Process resources committed in the same synthetic Zone within a 50 ms window; fake effect-port launch latency fixed at 200 ms per resource (chosen to exceed the 20 ms gate by 10x, so a passing "commit-to-launch-attempt-start" measurement cannot be an artifact of the launch itself finishing fast). |
| Command/harness | `cargo bench --manifest-path proofs/process-fastlaunch-spike/Cargo.toml -- launch_attempt_start` with `criterion`, one benchmark group per concurrency level (1/10/100), `--sample-size 200` per group. |
| Metrics | (1) p50/p95/p99 elapsed time from commit to launch-attempt-start (fake effect-port `spawn()` call entry) per concurrency level; (2) "next-dispatch" latency: elapsed time from resource *N*'s commit to resource *N+1*'s handler-start when both are ready simultaneously, for *N* in 1..100; (3) total wall time for all 100 resources to reach launch-attempt-start. |
| Pass/fail threshold | (1) p95 ≤20,000 µs (20 ms) at all three concurrency levels; (2) no resource's next-dispatch latency exceeds 20 ms regardless of how many earlier resources are still inside their (200 ms) fake launch sleep — this is the concurrency-independence assertion; (3) total wall time for 100 resources ≤ (20 ms dispatch budget + configured semaphore width × 200 ms), proving effects run in parallel under budget rather than serially. |
| Expected resource budget | ≤5 minutes wall time for the 100-resource benchmark group; process peak RSS ≤128 MiB (in-memory synthetic Zone only, no real redb file). |
| Failure interpretation | A miss in metric (1) revises the controller dispatch loop or hint-bus design in `ADR-046-resource-reconciliation` §"Process fast path". A miss in metric (2) — one resource's launch blocking another's dispatch — is treated as a severity-blocking finding against the "independent resources run in parallel under semaphore/budget" invariant in the same section; the fix is structural (remove the blocking await), never a threshold relaxation. |
| Affected decisions/work items | D030; `ADR-046-resource-reconciliation` (Process fast path, async loop); `ADR-046-resources-host-guest-process-user` (Fast path contract); `ADR046-reconcile-001`, `ADR046-reconcile-003`, `ADR046-process-001`. |
| Cleanup | Deleted once `packages/d2b-controller-toolkit/benches/reaction.rs` and the Process-Provider integration tests named by `ADR046-reconcile-003` reproduce the same 1/10/100-concurrency p95/independence gates against real Process Provider controllers. |
| Status | Specified — not yet executed. |

### SPIKE-04 — async EffectPort adapters never block the executor

| Field | Value |
| --- | --- |
| Hypothesis | `ProcessLaunchEffectPort`, `VolumeLayoutEffectPort`/`VolumeSourceEffectPort`, `NetworkEffectPort`, and `DeviceEffectPort` calls are fully async from the calling controller's perspective; every blocking kernel/filesystem/broker-socket call inside their implementation runs on a bounded blocking-adapter (`tokio::task::spawn_blocking` or an equivalent dedicated thread pool), and no controller task holds a redb transaction or a blocking call across an `.await`. |
| Minimal disposable artifact | `proofs/effectport-async-spike/` — implements the four EffectPort trait signatures from `ADR-046-components-processes-and-sandbox` and `ADR-046-resources-volume` against fake backends that perform a real blocking syscall analogue (a `std::fs::File::sync_all()` on a temp file standing in for a broker filesystem op, and a `std::thread::sleep` standing in for a blocking `clone3`/`pidfd_open` call), instrumented with a tokio `LocalSet`-free single-threaded current-thread runtime so any accidental blocking call inside an `.await` stalls the whole runtime and is trivially detected. |
| Inputs | 200 concurrent EffectPort calls (50 per port) issued against a single-threaded `tokio` runtime (`#[tokio::main(flavor = "current_thread")]`) alongside a fixed 10 ms-interval heartbeat task; each fake backend's blocking primitive is deliberately slow (50 ms) to make any accidental synchronous execution on the async worker visibly stall the heartbeat. |
| Command/harness | `cargo test --manifest-path proofs/effectport-async-spike/Cargo.toml -- --test-threads=1 effectport_never_blocks`; the test asserts on the heartbeat task's own recorded tick-to-tick jitter, not on the EffectPort latency itself. |
| Metrics | Heartbeat tick-to-tick jitter (max observed gap between consecutive 10 ms heartbeat ticks) while 200 EffectPort calls with 50 ms blocking backends are in flight on the same current-thread runtime. |
| Pass/fail threshold | Max heartbeat gap ≤15 ms (50% tolerance over the 10 ms nominal interval) throughout the entire 200-call run. A gap ≥50 ms (matching the fake backend's blocking duration) is conclusive proof a blocking call executed directly on the async worker and is an automatic fail. |
| Expected resource budget | ≤1 minute wall time; single OS thread for the async runtime plus the bounded blocking-adapter pool (sized to 16 threads, matching a conservative real broker/effect-adapter pool budget). |
| Failure interpretation | Any observed stall traces directly to the offending EffectPort implementation call site; the fix is moving that call behind `spawn_blocking` (or the broker-side blocking adapter it dispatches to) — this is a structural fix, not a threshold change, per anti-claim rule 3 (no relaxing "no handler holds ... a blocking kernel/filesystem call across an await" from `ADR-046-resource-reconciliation`). |
| Affected decisions/work items | D077; `ADR-046-components-processes-and-sandbox` (ProviderSupervisor and EffectPort); `ADR-046-resource-reconciliation` (Async interface); `ADR046-process-001`, `ADR046-volume-001` (see `ADR-046-resources-volume` implementation work items). |
| Cleanup | Deleted once `packages/d2b-provider-supervisor` (the real `ADR046-process-001` destination) and the volume-domain effect adapter each carry an in-tree blocking-adapter regression test with an equal or stricter jitter gate. |
| Status | Specified — not yet executed. |

### SPIKE-05 — independent Provider crate → multiple binaries → manifest/config-schema/component registration

| Field | Value |
| --- | --- |
| Hypothesis | A single Provider crate can declare one Provider identity, build two independently sandboxed binaries (a controller and a service, per `ADR-046-provider-model-and-packaging` §"Provider components"), publish a signed manifest carrying component descriptors + a root JSON Schema, and have core `ProviderDeployment` parse that manifest and create the exact declared static Process graph — without the crate importing `d2bd`, broker, or Zone-store internals. |
| Minimal disposable artifact | `proofs/provider-packaging-spike/` — a two-binary Cargo crate (`src/bin/controller.rs`, `src/bin/service.rs`) plus a hand-authored `manifest.json` matching the field list in `ADR-046-provider-model-and-packaging` §"Provider resource" and §"Package catalog" (package/executable/manifest/component digests, exported ResourceTypes, controller/service component descriptors, root config JSON Schema); a fake `ProviderDeployment` reads this manifest and asserts it can enumerate exactly the declared components/binaries/digests with no additional discovery. |
| Inputs | One manifest declaring 1 controller + 1 service + 1 worker template, a root config schema with 3 required fields (2 typed, 1 with a `sourcePolicyId`-shaped opaque string), and a deliberately mismatched second manifest (wrong digest) to test the negative path. |
| Command/harness | `cargo test --manifest-path proofs/provider-packaging-spike/Cargo.toml -- --test-threads=1`; a workspace-policy check script (`proofs/provider-packaging-spike/check_layout.sh`) asserting the crate has its own `src/`, `tests/`, `integration/`, and `README.md` per D059, mirrored against `cargo metadata --manifest-path proofs/provider-packaging-spike/Cargo.toml --no-deps` to confirm zero dependency edges on any `d2bd`/broker/Zone-store crate name. |
| Metrics | (1) exact component/binary/digest enumeration match against the manifest; (2) config validation rejects unknown top-level fields (`additionalProperties: false`) and out-of-bounds values; (3) digest-mismatch manifest is rejected before any component is considered; (4) `cargo metadata` dependency graph contains zero edges to any forbidden crate name. |
| Pass/fail threshold | All four metrics must hold exactly (binary pass/fail, no partial credit) across 20 repeated manifest-load cycles with randomized field ordering in the JSON to rule out order-dependent parsing bugs. |
| Expected resource budget | ≤2 minutes wall time; no filesystem beyond the crate's own `target/`. |
| Failure interpretation | A component/digest enumeration mismatch or forbidden dependency edge blocks `ADR046-provider-001`/`ADR046-provider-002` (see `ADR-046-provider-model-and-packaging` implementation work items) from starting until the manifest schema or crate-layout policy check is corrected. |
| Affected decisions/work items | D012, D057, D059, D075, D078; `ADR046-provider-001`, `ADR046-provider-002`, `ADR046-provider-003`. |
| Cleanup | Deleted once the real Provider-toolkit crate (destination of `ADR046-provider-001`) ships its own manifest-parsing/component-enumeration/workspace-policy conformance tests with equal or stricter coverage. |
| Status | Specified — not yet executed. |

### SPIKE-06 — d2b-bus exact-addressed routing with per-recipient distinct Noise protection

| Field | Value |
| --- | --- |
| Hypothesis | `d2b-bus` resolves the exact `(Zone, service package, method/stream, target ResourceRef or Provider, schema fingerprint, generation)` route key to exactly one destination process, and two distinct recipient components enrolled with distinct static Noise keys each get their own independent `Noise_KK` session — no session key, record sequence, or transcript is shared or reusable across recipients. |
| Minimal disposable artifact | `proofs/bus-routing-noise-spike/` — reuses main a1cc0b2d's `d2b-session`/`d2b-session-unix` crates (copied verbatim into the spike's own `Cargo.toml` path dependency from a pinned local checkout of `a1cc0b2d`, per the reuse policy in `ADR-046-componentsession-and-bus`) plus a minimal in-process router implementing only the route-key resolution and RBAC-attribute check (a fake static-allow-list RBAC, not the real Role/RoleBinding engine). |
| Inputs | 3 synthetic recipient components (`recipient-a`, `recipient-b`, `recipient-c`), each with its own enrolled `Noise_KK` static keypair; 500 routed calls fanned out round-robin across the 3 recipients; one deliberate cross-wiring attempt where the router is fed `recipient-a`'s route key but `recipient-b`'s transport handle, which must fail closed. |
| Command/harness | `cargo test --manifest-path proofs/bus-routing-noise-spike/Cargo.toml -- --test-threads=1 bus_routing_and_per_recipient_noise`. |
| Metrics | (1) every one of the 500 calls is delivered to exactly the recipient named by its route key, verified by a per-recipient received-call counter; (2) each recipient's session transcript hash and record sequence counter are independent (no shared state, verified by asserting the three sessions' internal sequence counters never reference each other's session object); (3) the deliberate cross-wiring case is rejected before any record is exchanged. |
| Pass/fail threshold | 500/500 correct routing, 0 cross-recipient session-state leakage, 1/1 cross-wiring case rejected with a stable typed error (not a panic, not a silent no-op). |
| Expected resource budget | ≤2 minutes wall time; ≤64 MiB RSS (three lightweight Noise sessions plus the fake router). |
| Failure interpretation | A misrouted call or shared-session-state finding blocks `ADR046-bus-001` from starting until the route-key resolution or per-session isolation in `ADR-046-componentsession-and-bus` §"d2b-bus" is corrected; per anti-claim rule 3, the fix is never "widen the route key to fail open." |
| Affected decisions/work items | D011, D039, D040, D054; `ADR046-session-001`, `ADR046-session-002`, `ADR046-bus-001`. |
| Cleanup | Deleted once `packages/d2b-bus/src/router.rs` (the real `ADR046-bus-001` destination) carries an in-tree message-isolation/route-authorization/no-direct-store-path conformance test with equal or stricter coverage, per that work item's own Validation column. |
| Status | Specified — not yet executed. |

### SPIKE-07 — Unix/vsock/Azure-Relay transports carry only opaque Noise record bytes

| Field | Value |
| --- | --- |
| Hypothesis | All three initial Transport Providers (`transport-unix` seqpacket/stream, `transport-vsock` framed vsock, `transport-azure-relay` WebSocket-carried) expose the same `OpenTransport`/`CloseTransport`/`ObserveTransport` shape and carry only 2-byte length-prefixed opaque Noise record bytes end-to-end; none of the three can decrypt, interpret, or leak a credential/path/PID through its carriage layer. |
| Minimal disposable artifact | `proofs/transport-opaque-streams-spike/` — three minimal transport backends: (a) a real Unix seqpacket socketpair (reusing main a1cc0b2d's `d2b-session-unix::adapter`); (b) main a1cc0b2d's in-memory vsock adapter (`d2b-session-unix::vsock` test-only `InMemoryVsockAdapter`, already exercised in E3); (c) a fake WebSocket-shaped byte pipe (an in-process `tokio::io::duplex` standing in for the real Azure Relay WebSocket, since no live Azure Relay resource is provisioned by a disposable spike) — each wrapped by the same generic `TransportHandle` trait object and driven by one shared conformance test suite. |
| Inputs | A 64 KiB pseudorandom Noise-record-shaped payload (2-byte length prefix + ciphertext) sent across each of the three transports; a byte-level payload inspector sitting "in the middle" of each transport that must observe only opaque length-prefixed bytes (never a decrypted plaintext, never an `SCM_RIGHTS` control message on the vsock/relay legs). |
| Command/harness | `cargo test --manifest-path proofs/transport-opaque-streams-spike/Cargo.toml -- --test-threads=1 opaque_byte_stream_conformance` — one parametrized test instantiated three times, once per transport backend. |
| Metrics | (1) byte-exact delivery of the 64 KiB payload on each transport; (2) the middle-observer never decodes a valid Noise record (proving it never sees plaintext — it can only see the same ciphertext bytes the endpoints exchange); (3) `SCM_RIGHTS`/attachment attempts are accepted only on the Unix seqpacket backend and rejected (`attachment-not-permitted-over-zone-link`) on the vsock and relay-shaped backends; (4) each transport's own dossier-defined performance gate: Unix `OpenTransport` p95 ≤2 ms (seqpacket) / ≤1 ms (stream); vsock `OpenTransport` overhead ≤2 ms p99 (excluding connect) and bridge throughput ≥512 MiB/s on loopback; relay-shaped backend backpressure propagates (send blocks) when the fake WebSocket leg's buffer is deliberately capped at 4 KiB. |
| Pass/fail threshold | (1)-(3) must hold exactly (binary); (4) must meet the exact numeric gates already committed in `docs/specs/providers/ADR-046-provider-transport-unix.md` §"Performance targets" and `ADR-046-provider-transport-vsock.md` §"Performance gates" (reproduced here, not redefined): Unix seqpacket open p95 ≤2 ms, Unix stream open p95 ≤1 ms, vsock open overhead p99 ≤2 ms, vsock bridge throughput ≥512 MiB/s. |
| Expected resource budget | ≤3 minutes wall time; ≤256 KiB working set per active transport (matching the vsock dossier's own bridge-task memory gate), measured via the same `/usr/bin/time -v` methodology as SPIKE-01. |
| Failure interpretation | A plaintext leak or accepted attachment on the vsock/relay-shaped backend is a severity-blocking security finding against `ADR-046-zone-routing` §"No FD, credential, or host path forwarding" and blocks every `ADR046-transport-*-00x` work item until fixed; a latency/throughput miss revises the transport's own implementation (buffer sizing, syscall batching), never the already-committed dossier target. |
| Affected decisions/work items | D081; `ADR046-transport-unix-001..011`, `ADR046-transport-vsock` work items, `ADR046-transport-relay-001..007`. |
| Cleanup | Deleted once each real transport Provider crate (`d2b-provider-transport-unix`, `-vsock`, `-azure-relay`) reproduces its own share of these metrics in its `tests/`/`integration/` per its dossier's "Required tests" section, including a real (non-fake) Azure Relay integration test gated behind the existing live-credential opt-in convention used elsewhere in this repository for cloud-backed tests. |
| Status | Specified — not yet executed. |

### SPIKE-08 — Credential Provider → consumer Provider raw delivery over dedicated Noise_KK

| Field | Value |
| --- | --- |
| Hypothesis | A Credential Provider can deliver a bounded raw token (or `SignChallenge` signature) to one authorized consumer Provider/component over a dedicated end-to-end `Noise_KK` ComponentSession, with the delivery binding contract fields from `ADR-046-resources-credential` §"Binding contract" enforced, `d2b-bus` forwarding only opaque records without decrypting them, and the plaintext zeroized immediately after extraction with no logging/audit/metric surface ever seeing the byte value. |
| Minimal disposable artifact | `proofs/credential-kk-e2e-spike/` — reuses main a1cc0b2d's `d2b-session` Noise_KK handshake/record machinery (proven green in E3) plus a fake Credential Provider (`acquire-token`) and fake consumer Provider, with a fake `d2b-bus` relay in the middle that can only forward opaque records (it is deliberately given no decryption capability at all, not merely "policy forbidding" it, so any successful decryption at the relay is a code-level bug, not a policy violation). |
| Inputs | A 256-byte synthetic "token" value; the 11 binding-contract fields from `ADR-046-resources-credential` §"Binding contract" (`credentialRef`, `credentialUID`, `credentialGeneration`, `consumerProviderRef`, `consumerComponentGeneration`, `audience`, `operationClass`, `expiryUnixMs`, `deadlineUnixMs`, `routeDigest`, `schemaVersion`, `maxTokenBytes`, `transcriptDigest`); one deliberate NN-profile bootstrap attempt (must be rejected per §"Security requirements", item 1); one deliberate oversize (larger than `maxTokenBytes`) delivery attempt (must be rejected per item 3); one deliberate replay of a prior sequence number (must be rejected per item 2). |
| Command/harness | `cargo test --manifest-path proofs/credential-kk-e2e-spike/Cargo.toml -- --test-threads=1 credential_kk_delivery`. |
| Metrics | (1) successful delivery: consumer receives the exact 256-byte token, binding fields verified by both endpoints before accepting records; (2) relay-observability: the fake relay's log of forwarded bytes never contains the plaintext token (checked by a substring search over everything the relay ever touches); (3) NN-bootstrap-for-delivery rejection; (4) oversize-payload rejection with channel close+zeroize; (5) sequence-replay rejection; (6) post-ACK channel closes and a canary zeroizing wrapper confirms the buffer is zeroed (test-only introspection into the zeroizing type, not available in production code paths). |
| Pass/fail threshold | All six metrics binary pass; zero occurrences of the plaintext token anywhere the relay can observe, across 100 repeated deliveries with freshly generated random tokens each run. |
| Expected resource budget | ≤2 minutes wall time; ≤32 MiB RSS. |
| Failure interpretation | Any plaintext-at-relay occurrence is an automatic, severity-blocking security finding against `ADR-046-resources-credential` §"Credential-delivery endpoint contract" and blocks every `ADR046-credential-00x` work item; a binding-field check bypass blocks the same work items pending a fix to the offer/prologue construction, never a relaxed binding contract. |
| Affected decisions/work items | D055, D056, D068; `ADR046-credential-001` through `ADR046-credential-008`. |
| Cleanup | Deleted once the real `d2b-provider-credential-secret-service` (or any of the three frozen Credential Provider crates) carries its own end-to-end KK delivery conformance test with equal or stricter coverage, per `ADR-046-resources-credential` §"Runtime tests". |
| Status | Specified — not yet executed. |

### SPIKE-09 — Optional state-Volume creation order, guest-local vs. host-backed-guest, virtiofs Export child

| Field | Value |
| --- | --- |
| Hypothesis | Core `ProviderDeployment` creates a component's *declared* optional state Volume (a stateless component declares none and gets none — its bounded non-secret state lives in resource `status`/the core Operation ledger per D087) before launching that component's Process, requires no bootstrap state-Volume mechanism (the first `volume-local` instance per execution target declares no state Volume and reaches Ready from its own status without crossing a Host/Guest boundary), and correctly creates exactly one `virtiofs.d2b.io.Export` child (owned by the source Volume, not by the Provider) per attachment for `placementMode: host-backed-guest`, while `placementMode: guest-local` creates zero Export children and the Host volume-local controller never touches its bytes/dirfd/path. |
| Minimal disposable artifact | `proofs/provider-state-export-spike/` — an in-process fake resource store (a plain `HashMap`-backed oracle, not real redb) modeling just enough of `ADR-046-provider-state` §"State placement under Host/Guest/user execution" to construct the ordering/ownership graph and assert it; two fake execution targets ("Host/h1", "Guest/g1") each with their own fake `volume-local` instance that reaches Ready from status alone. |
| Inputs | (a) a stateless worker component descriptor (no declared `stateNamespaces`); (b) a stateful controller component descriptor with `placementMode: guest-local` targeting `Guest/g1`; (c) a stateful controller component descriptor with `placementMode: host-backed-guest` and 2 declared `attachments[]` targeting `Guest/g1`; (d) an attempted `placementMode: host-backed-guest` for a component descriptor schema-flagged as carrying "gateway credentials" (must be rejected `guest-local-required`). |
| Command/harness | `cargo test --manifest-path proofs/provider-state-export-spike/Cargo.toml -- --test-threads=1 provider_state_set_and_export`. |
| Metrics | (1) creation order: the stateless worker (a) has zero state Volumes; each *declared* state Volume exists and is Ready before its component's Process resource is created, in cases (b)/(c); (2) no bootstrap state Volume: the first `volume-local` instance on each target reaches Ready without any state Volume, and no cross-target bootstrap reference exists (asserted by tagging every provisioning call with its execution-target ID and checking zero cross-target references and zero bootstrap-storage calls); (3) Export-child count: exactly 0 for case (b), exactly 2 for case (c), each owned (`ownerRef`) by the source Volume, not by `Provider/<name>`; (4) case (d) is rejected with `guest-local-required` before any Volume is created. |
| Pass/fail threshold | All four metrics binary pass across all 4 input cases; zero cross-target references and zero bootstrap-storage calls (metric 2) is a hard zero-tolerance gate, not a threshold. |
| Expected resource budget | ≤2 minutes wall time; ≤32 MiB RSS (pure in-memory graph construction, no real filesystem/virtiofs). |
| Failure interpretation | A creation-order violation (Process created before its declared state Volume is Ready) blocks every `ADR-046-provider-state` work item and the `ADR046-provider-00x` work items that depend on it; any bootstrap state-Volume mechanism or cross-target reference is a severity-blocking finding against the "No bootstrap state Volume" invariant in `ADR-046-components-processes-and-sandbox` and must be fixed structurally, never suppressed. |
| Affected decisions/work items | D076, D078, D086, D087; provider-state and volume-virtiofs implementation work items named in `ADR-046-provider-state` and `ADR-046-resources-volume`. |
| Cleanup | Deleted once core `ProviderDeployment`'s real implementation (destination named by the `ADR-046-provider-state` work items) carries an equivalent in-tree ordering/ownership/Export-count conformance test. |
| Status | Specified — not yet executed. |

### SPIKE-10 — Volume ACL, `sourcePolicyId`, quota, and lifecycle-marker policies

| Field | Value |
| --- | --- |
| Hypothesis | A Volume-local implementation can enforce `AclGrant` principals typed strictly as `User/<name>` (no numeric UID/GID form accepted), resolve `source.settings.sourcePolicyId` to a real path only inside the private effect-adapter boundary (never surfacing the path in spec/status/audit), enforce `hard` quota by failing Volume creation immediately when the backing filesystem cannot guarantee the limit, and drive `CreatePolicy`/`RepairPolicy`/`CleanupPolicy`/`AdoptionPolicy`/`RestartPolicy` through their full state tables without silently reinterpreting any value. |
| Minimal disposable artifact | `proofs/volume-policy-spike/` — a fake `volume-local` controller operating against a real temporary directory (via `tempfile::TempDir`, cleaned up automatically) standing in for one allowlisted root, with a fake `VolumeSourceEffectPort`/`VolumeLayoutEffectPort` implementing exactly the layout-entry/ACL/quota logic described in `ADR-046-resources-volume`, never a broker call. |
| Inputs | (a) 6 layout entries covering every `CreatePolicy` value; (b) an ACL grant with a numeric-UID-shaped string (must be rejected at schema validation, not at runtime); (c) a `tmpfs`-kind Volume with `quota.maxBytes = 8 MiB`/`maxInodes = 1024` and an attempt to write 16 MiB (must fail at or before the 8 MiB boundary via the kernel `size=`/`nr_inodes=` mount options); (d) a `block-image`-kind Volume with `enforcement: hard` on a backing filesystem/loop device deliberately unable to guarantee the quota (a sparse file smaller than the requested quota) — must fail closed to `Failed` status with zero layout operations attempted; (e) one `sourcePolicyId` resolution, asserting the resolved absolute path never appears in the returned "public" Volume status/spec echo. |
| Command/harness | `cargo test --manifest-path proofs/volume-policy-spike/Cargo.toml -- --test-threads=1 volume_acl_quota_policy`. |
| Metrics | (1) every `CreatePolicy` value produces exactly its documented `StorageLifecycle`-analog behavior (matching the table in `ADR-046-resources-volume` §"CreatePolicy"); (2) numeric-UID ACL is rejected before any filesystem mutation; (3) tmpfs write beyond quota fails at the kernel boundary (`ENOSPC`/`EDQUOT`-class error), not silently truncated; (4) hard-enforcement Volume on an unenforceable backend reaches `Failed` with zero layout side effects (verified by asserting the target directory remains exactly as it was before the attempt); (5) resolved host path is absent from every field of the returned status/spec/audit-record structs (checked via a recursive string-search helper over the serialized DTOs). |
| Pass/fail threshold | All five metrics binary pass across all listed inputs; metric (5) is zero-tolerance (a single path leak anywhere in a public DTO is an automatic fail). |
| Expected resource budget | ≤2 minutes wall time; ≤64 MiB RSS; ≤32 MiB scratch disk (bounded by the 8 MiB tmpfs fixture plus the undersized sparse-file fixture). |
| Failure interpretation | A path leak is a severity-blocking security finding against D082 and blocks `ADR046-volume-001` through `ADR046-volume-006`; a quota-enforcement miss blocks the same work items pending a fix to the pre-creation filesystem-capability probe, never a relaxed `hard` semantics. |
| Affected decisions/work items | D032, D044, D062, D082, D083, D084; `ADR046-volume-001` through `ADR046-volume-006`. |
| Cleanup | Deleted once `d2b-provider-volume-local`'s own `tests/`/`integration/` suite (per its dossier) reproduces these five metrics with equal or stricter coverage against the real broker-mediated layout effect adapter. |
| Status | Specified — not yet executed. |

### SPIKE-11 — systemd/minijail Process Provider pidfd acquire/verify/reap/adopt/quarantine conformance

| Field | Value |
| --- | --- |
| Hypothesis | Both Process Provider implementations satisfy the identical pidfd contract in `ADR-046-resources-host-guest-process-user` §"Pidfd rules": a verified pidfd is acquired only after stable-identity verification, is never persisted/serialized/bus-exposed, is closed and reopened with full re-verification after every ProviderSupervisor restart, and ambiguous post-restart identity always quarantines rather than silently adopts or kills. |
| Minimal disposable artifact | `proofs/process-provider-conformance-spike/` — one shared conformance test suite (a single Rust `trait ProcessProviderHarness` with `launch`/`restart_supervisor`/`simulate_identity_drift` methods) instantiated twice: once against a real `clone3(CLONE_PIDFD)`-based minijail-shaped launcher (reusing the existing `packages/d2b-priv-broker/src/ops/spawn_runner.rs` real-spawn shape as a reference, invoked directly and unprivileged via a plain `fork`+`exec` substitute — no real broker/minijail sandbox compilation, since this spike proves the pidfd/identity state machine, not sandbox compilation), and once against a real transient systemd user-scope launcher (InvocationID+cgroup+MainPID+start-time) using `systemd-run --user --scope`. |
| Inputs | (a) 20 successful launches per implementation, each followed by a supervisor "restart" (drop and reacquire the pidfd) and re-verification; (b) 5 deliberate identity-drift cases per implementation (kill the original process and immediately start an unrelated process reusing the same PID before re-verification runs) which must quarantine, never silently adopt; (c) 1 case per implementation where the process exits cleanly before restart — must not attempt an invalid reap. |
| Command/harness | `cargo test --manifest-path proofs/process-provider-conformance-spike/Cargo.toml --features systemd-user -- --test-threads=1 process_provider_conformance` (the `systemd-user` feature gates the systemd half behind `systemd-run --user` availability, matching this repository's existing host-integration-only gating pattern for systemd-dependent tests; the minijail-shaped half has no such gate). |
| Metrics | (1) 20/20 successful launches per implementation acquire a pidfd only after all identity checks pass; (2) 20/20 restarts close and reopen the pidfd with full re-verification (never reusing the pre-restart pidfd); (3) 5/5 identity-drift cases per implementation quarantine (`adoptionState: quarantined`), never silently adopt the unrelated process nor blindly `SIGKILL` an unrelated PID; (4) 1/1 clean-exit case per implementation reports terminal status without an invalid reap attempt (no `ESRCH`/error swallowed silently). |
| Pass/fail threshold | All four metrics at 100% across both implementations (40 launches, 40 restarts, 10 drift cases, 2 clean-exit cases total); metric (3) is zero-tolerance — any single false adoption of an unrelated process is an automatic fail. |
| Expected resource budget | ≤5 minutes wall time (includes real process spawn/kill cycles); requires a Linux host with `clone3`/`pidfd_open` support and, for the systemd half, a running user `systemd --user` instance (skip with a clear diagnostic, not a silent pass, when unavailable — matching this repository's existing host-integration skip convention). |
| Failure interpretation | Any false adoption blocks `ADR046-process-002` (the systemd/minijail Provider work item in `ADR-046-components-processes-and-sandbox`) until the identity-verification tuple (executable hash + template generation + cgroup/scope placement + provider-specific attributes) is corrected; per anti-claim rule 3, the fix is never "trust the PID alone," which the spec already forbids. |
| Affected decisions/work items | D022, D051; `ADR046-process-001`, `ADR046-process-002`. |
| Cleanup | Deleted once `packages/d2b-provider-system-systemd` and `packages/d2b-provider-system-minijail` each carry this exact shared conformance suite in their own `tests/` per D059's required crate layout. |
| Status | Specified — not yet executed. |

### SPIKE-12 — Nix direct ResourceSpec authoring → codegen → build validation → removed-resource cleanup

| Field | Value |
| --- | --- |
| Hypothesis | A Nix expression authoring `d2b.zones.<zone>.resources.<name> = { type = "..."; spec = { ... }; }` directly mirrors the canonical ResourceTypeSchema (no second Nix vocabulary), a `cargo xtask gen-schemas`-shaped generator produces the same committed JSON Schema the Nix build validates against, an eval-time assertion rejects bound/ref/domain/cycle violations before any build, a build-time derivation rejects schema-violating or store-path-carrying `spec`/`config` values, and removing a configuration-managed resource from the next Nix generation triggers asynchronous finalizer-safe deletion (visible Degraded/pending-cleanup status) rather than an immediate destructive sweep. |
| Minimal disposable artifact | `proofs/nix-authoring-spike/` — a minimal flake (`flake.nix` + `resources.nix`) declaring 2 synthetic ResourceTypes (one with a `Ref` field, one with a nested object and a bound numeric field) with hand-written committed JSON Schemas under `proofs/nix-authoring-spike/schemas/`, a small standalone Rust `xtask`-shaped binary (`proofs/nix-authoring-spike/src/bin/gen-schemas.rs`) that emits the same schema from a hand-written DTO to prove the codegen/drift-gate shape (`git diff --exit-code`-style comparison) works end to end, and a Nix derivation that validates a rendered `resources.json` against the committed schema using the same offline/hermetic/no-network approach as `ADR-046-nix-configuration` §"Bundle integrity". |
| Inputs | (a) a valid two-resource configuration; (b) a configuration with a dangling `Ref` (must fail eval); (c) a configuration whose rendered `spec` violates the committed schema bound (must fail build); (d) a configuration whose `spec` contains a Nix-store-path-shaped string (must fail build per D070); (e) a second Nix generation that omits one of the two resources present in generation (a), with that resource's `managedBy: configuration`. |
| Command/harness | `nix build --no-link ./proofs/nix-authoring-spike#checks.$(nix eval --impure --raw --expr builtins.currentSystem).resources-valid` for case (a); `nix eval --impure ./proofs/nix-authoring-spike#checks... 2>&1` (expect eval failure) for case (b); `nix build ./proofs/nix-authoring-spike#checks....resources-schema-violation 2>&1` (expect build failure) for cases (c)/(d); `cargo run --manifest-path proofs/nix-authoring-spike/Cargo.toml --bin gen-schemas -- --check` (expect nonzero exit on intentional drift, matching `make test-drift`'s `xtask gen-schemas` + `git diff --exit-code` pattern) for the codegen check; a small harness script simulating generation (e) and asserting the omitted resource's status becomes `Degraded`/pending-cleanup rather than disappearing synchronously. |
| Metrics | (1) case (a) builds successfully and the rendered JSON is byte-identical on a second hermetic build (D-070-style reproducibility); (2) case (b) fails at eval, not build; (3) cases (c)/(d) fail at build with a structured error naming the exact offending Nix option path; (4) the codegen check exits nonzero exactly when the hand-written schema and generator-emitted schema diverge, and zero when they match; (5) the omitted resource in generation (e) is not present in the new generation's active resource set, is still observable (Degraded/pending-cleanup) until its simulated finalizer completes, and the resource present in both generations is untouched. |
| Pass/fail threshold | All five metrics binary pass; metric (1)'s reproducibility check must match byte-for-byte across 3 repeated hermetic builds. |
| Expected resource budget | ≤5 minutes wall time (Nix evaluation/build dominates); no network access during any `nix build` invocation (`--offline` or an equivalent sandboxed evaluation, matching this repository's existing hermetic-build convention). |
| Failure interpretation | A non-reproducible build or a schema-violating value that still builds blocks every `ADR-046-nix-configuration` work item and every ResourceType/Provider spec's own "Nix authoring and configuration cleanup" section; a synchronous (non-finalizer-safe) deletion on generation change is a severity-blocking finding against D057 and must be fixed structurally. |
| Affected decisions/work items | D057, D058, D069, D070; the Nix-authoring implementation work items in `ADR-046-nix-configuration` and in every ResourceType/Provider spec's own Nix section. |
| Cleanup | Deleted once the real `nixos-modules/resources.nix` + `packages/xtask` `gen-schemas` implementation (per `ADR-046-nix-configuration`'s own work items) reproduces these five metrics as part of `make test-drift`/`make test-flake`. |
| Status | Specified — not yet executed. |

### SPIKE-13 — CLI dynamic Provider-projection discovery with bounded deadline and size

| Field | Value |
| --- | --- |
| Hypothesis | The CLI discovers a Provider's `cliProjection` lazily via `InspectSchema` only when that Provider's subcommand is invoked or `--help` for `d2b provider` is requested, respects the exact bounds in `ADR-046-cli-and-operations` §"Dynamic descriptors — safety bounds" (64 KiB total, 32-byte name limits, 32 sub-verbs, 2 s per-Provider / 10 s total deadline), rejects a Provider-projected name colliding with a built-in verb at install/bind time (not at CLI runtime), and imposes zero added startup latency on any non-Provider command. |
| Minimal disposable artifact | `proofs/cli-discovery-spike/` — a fake `InspectSchema` server (a plain async TCP/Unix-socket stub, not real ComponentSession) returning configurable projection payloads, plus a minimal CLI-shaped harness implementing exactly the discovery/caching/rendering logic described in that section (lazy fetch, per-invocation-only cache, deadline enforcement, byte-escaping of completion strings). |
| Inputs | (a) a well-formed 8 KiB projection; (b) a 128 KiB projection (must be skipped for exceeding the 64 KiB bound, with the documented single stderr line, not a crash); (c) a projection whose top-level subcommand name is `list` (must be rejected at bind time with the built-in-collision rule); (d) a Provider whose `InspectSchema` never responds (must time out at exactly the 2 s per-Provider deadline and continue without blocking the CLI); (e) 6 Providers simultaneously slow, each taking 3 s, to verify the 10 s total-fetch deadline caps overall wait; (f) a projection containing a newline and an HTML-special character in a completion string, to verify escaping. |
| Command/harness | `cargo test --manifest-path proofs/cli-discovery-spike/Cargo.toml -- --test-threads=1 cli_dynamic_discovery`. |
| Metrics | (1) case (a) renders correctly; (2) case (b) is skipped with the exact documented single-line warning format; (3) case (c) is rejected before any command dispatch, with a stable error, not a runtime shadowing; (4) case (d)'s per-Provider wait is capped at 2 s (measured); (5) case (e)'s total wait is capped at 10 s, not 6×3=18 s; (6) case (f)'s rendered completion string contains no raw newline and no unescaped HTML/shell-special character; (7) a non-Provider command's measured startup time is statistically indistinguishable (within measurement noise, ≤5 ms difference) between a CLI build with zero installed Providers and one with 20 installed (slow) Providers, proving zero added startup cost. |
| Pass/fail threshold | All seven metrics binary pass except (7), which is a numeric gate: ≤5 ms measured difference in non-Provider command startup latency (median of 50 runs) between the 0-Provider and 20-Provider fixtures. |
| Expected resource budget | ≤3 minutes wall time (dominated by the deliberate 2 s/3 s timeout cases). |
| Failure interpretation | A collision that reaches runtime shadowing, or a startup-latency regression on non-Provider commands, blocks `ADR046-cli-001` through `ADR046-cli-011` (the exact set depends on which sub-item owns discovery/projection rendering) until the discovery/caching/bind-time-rejection logic is corrected; per anti-claim rule 3, the fix is never "widen the deadline" to make a slow-Provider case pass. |
| Affected decisions/work items | D064; the CLI implementation work items `ADR046-cli-001` through `ADR046-cli-011` in `ADR-046-cli-and-operations`. |
| Cleanup | Deleted once the real `d2b` CLI binary (destination of the owning `ADR046-cli-*` work item) carries an equivalent in-tree discovery/bound/collision/latency conformance test. |
| Status | Specified — not yet executed. |

### SPIKE-14 — clean v3 reset and cutover (no v2 alias dispatch, fresh Zone bootstrap)

| Field | Value |
| --- | --- |
| Hypothesis | A v3 CLI binary built from this spec's authoring baseline contains zero executable dispatch paths for any of the removed v2 commands listed in `ADR-046-cli-and-operations` §"v2 command surface removed at 3.0 clean break" (only an optional `d2b migrate-check` diagnostic that explains replacements without dispatching), and a fresh Zone can bootstrap from an empty state directory through Nix generation activation to a Ready `Zone/<name>` self-resource with no v2/Realm state import of any kind. |
| Minimal disposable artifact | `proofs/clean-cutover-spike/` — a static-analysis check (`cargo metadata` + a symbol-table scan of the built CLI binary, e.g., via `nm`/`strings`-shaped inspection or, more robustly, a source-level `grep`-based assertion against the CLI crate's command-table source) confirming none of the 27 removed-command strings from that table dispatch to a handler function, plus a minimal "fresh Zone bootstrap" harness driving the fixed bootstrap sequence in `ADR-046-components-processes-and-sandbox` §"Bootstrap boundary" against fakes for the Zone runtime, broker, and fixed controllers (no real redb/broker — this spike proves *sequencing and state-freshness*, not the redb/process spikes already covered by SPIKE-01/SPIKE-03/SPIKE-11). |
| Inputs | (a) the 27-row removed-command table from `ADR-046-cli-and-operations`; (b) an empty state directory (no `/var/lib/d2b` content of any kind, no Realm artifacts); (c) one deliberately injected legacy Realm-shaped file dropped into the fresh state directory before bootstrap, which must be ignored (never imported, never migrated) by the v3 bootstrap sequence. |
| Command/harness | A source-scan test: `cargo test --manifest-path proofs/clean-cutover-spike/Cargo.toml -- --test-threads=1 no_v2_alias_dispatch` asserting, for every one of the 27 removed-command strings, that the CLI crate's command-table source (a copy of the relevant `packages/d2b/src/lib.rs`-shaped command enum used by this spike's fixture, not the real crate) contains no executable arm; a second test, `fresh_zone_bootstrap_ignores_legacy_state`, drives the fake bootstrap sequence against inputs (b) and (c). |
| Metrics | (1) 27/27 removed commands have zero executable dispatch arms (only, optionally, a `migrate-check` explanatory branch); (2) fresh bootstrap from an empty directory reaches a Ready `Zone/<name>` self-resource through the fixed sequence (Zone runtime → broker → core-controller → minimum Host/Guest supervisor → user supervisor → system-minijail) with no step skipped or reordered; (3) the injected legacy Realm-shaped file is never read, referenced, or copied by any step of the bootstrap sequence (verified by a file-access-tracing fake filesystem that fails the test on any open() of that path). |
| Pass/fail threshold | All three metrics binary pass; metric (3) is zero-tolerance — a single stat()/open() of the legacy file is an automatic fail, since ADR 0046 defines the cutover as destructive with the pre-ADR45 v3 tree as ancestry, not main, and with no v2 data-import path of any kind. |
| Expected resource budget | ≤2 minutes wall time; ≤32 MiB RSS. |
| Failure interpretation | Any surviving v2 dispatch arm or any legacy-file access blocks the entire CLI/cutover implementation wave; per D001/D041, the fix is deletion of the offending path, never a compatibility shim. |
| Affected decisions/work items | D001, D002, D041, D064; the v2-removal work items implied by `ADR-046-cli-and-operations` §"v2 command surface removed at 3.0 clean break" and the bootstrap work items in `ADR-046-components-processes-and-sandbox`. |
| Cleanup | Deleted once the real `packages/d2b` CLI crate and the real Zone-runtime bootstrap sequence are built; their own workspace-policy/lint gates (extending the existing `deny.toml`/policy-test convention already used for other closed-set invariants in this repository) enforce the same zero-v2-dispatch and no-legacy-import guarantees permanently, superseding this spike. |
| Status | Specified — not yet executed. |

### SPIKE-15 — representative local / cloud / interaction Provider end-to-end composition

| Field | Value |
| --- | --- |
| Hypothesis | Three representative Provider compositions each reconcile end-to-end through Zone bootstrap → Provider install → Process launch → Ready status inside the shared aggregate resource-plane budget, without any composition requiring a code path outside the ones already defined by the other 14 spikes: (a) **local** — `Guest/dev-vm` on `Provider/runtime-cloud-hypervisor` with a `Provider/volume-local`-backed state Volume and Process Providers `Provider/system-minijail` launching the VMM process; (b) **cloud** — a `Guest` on `Provider/runtime-azure-container-apps` (or `runtime-azure-virtual-machine`) acquiring its identity through `Provider/credential-managed-identity` over the SPIKE-08 KK delivery path; (c) **interaction** — a `Provider/shell-terminal` (or `display-wayland`) Process under a user-domain `Host`, mounting an operator-declared `Volume` via the SPIKE-10 ACL/quota path (shell-terminal/display-wayland declare no Provider state Volume of their own; their bounded non-secret operational state lives in resource `status`/the core Operation ledger per D087). |
| Minimal disposable artifact | `proofs/e2e-composition-spike/` — a single fake-but-integrated harness wiring together the fakes already built for SPIKE-01 (store), SPIKE-02/03 (reconcile/fast-path), SPIKE-04 (EffectPort), SPIKE-05 (Provider packaging), SPIKE-06/07 (bus/transport), SPIKE-08 (credential KK), SPIKE-09/10 (state/volume), and SPIKE-11 (process conformance) into one process, driving all three compositions through the same fixed bootstrap sequence as SPIKE-14, so this spike is explicitly an **integration** of the other 14 fakes rather than a sixteenth independent fake stack. |
| Inputs | Three Nix-authored (per SPIKE-12's harness) ResourceSpec sets, one per composition, each declaring exactly the resources named in the Hypothesis row; a shared Zone budget ceiling matching `ADR-046-resource-store-redb`'s aggregate ≤64 MiB idle-RSS target, measured with all three compositions' fixed/mandatory processes running simultaneously (system-core, system-minijail, and each composition's own controller/service processes — excluding the large Guest runtime processes themselves, which are out of the resource-plane budget by definition in that spec). |
| Command/harness | `cargo test --manifest-path proofs/e2e-composition-spike/Cargo.toml -- --test-threads=1 end_to_end_composition_{local,cloud,interaction}` (three named tests, one per composition) plus `/usr/bin/time -v` RSS measurement (methodology identical to SPIKE-01) taken once with all three compositions' fixed/mandatory control-plane processes running concurrently. |
| Metrics | (1) each composition reaches `phase: Ready` on its top-level resource (the `Guest` for (a)/(b), the `Process` for (c)) within a bounded wall-clock budget of 5 s from Nix-generation-activation-equivalent trigger to Ready, using only the fakes already validated by the other 14 spikes (no new unvalidated code path); (2) the cloud composition's credential acquisition reuses exactly the SPIKE-08 KK delivery path with no plaintext leak, re-asserted in this integrated context; (3) the interaction composition's mounted `Volume` passes the exact SPIKE-10 ACL/`sourcePolicyId`/quota checks, re-asserted in this integrated context; (4) combined idle RSS of the fixed/mandatory control-plane processes across all three compositions running simultaneously stays within the aggregate budget scaled for 3 Zones sharing one host (≤3× the single-Zone ≤64 MiB target, i.e., ≤192 MiB, since each Zone embeds its own store/core-controller/system-minijail per D006/D007). |
| Pass/fail threshold | Metrics (1)-(3) binary pass for all three compositions; metric (4) ≤192 MiB combined median RSS over 3 repeated runs. |
| Expected resource budget | ≤10 minutes wall time (dominated by the 3×5 s Ready-latency budget plus fixture setup); ≤256 MiB total build RSS. |
| Failure interpretation | A composition that only reaches Ready by adding a code path not already covered by SPIKE-01 through SPIKE-14 is itself a finding: it means the feasibility catalog is incomplete for that composition, and the missing capability must be added to this catalog (via a spec revision) before the corresponding production work item is scheduled, per anti-claim rule 7 ("no unresolved entries") applied prospectively. A metric-4 RSS miss revises the per-Zone footprint budget or the number of Zones assumed to co-reside on one host, never the individual Zone target already fixed by `ADR-046-resource-store-redb`. |
| Affected decisions/work items | D006, D007, D008, D043, D044, D047, D048, D076; every Guest/Volume/Credential/interaction Provider dossier's own implementation work items, and the core bootstrap work items in `ADR-046-components-processes-and-sandbox`. |
| Cleanup | Deleted once the real integration test suites named by the individual Provider dossiers (`integration/` per D059) collectively reproduce compositions (a), (b), and (c) against real (non-fake) Zone/store/bus/broker code, at which point this spike's role — proving the fakes compose without a missing capability — is fully subsumed. |
| Status | Specified — not yet executed. |

### SPIKE-16 — three-layer status shape: schema parity, base-only projection, extension versioning

| Field | Value |
| --- | --- |
| Hypothesis | The frozen three-layer status shape (D088) — universal `ResourceStatus` base + ResourceType-common `status.resource` + optional Provider-specific `status.provider` — supports (a) identical `status.resource` shape across multiple implementations of one ResourceType; (b) a generic base-only consumer (universal base + `status.resource`) that reads/watches successfully while ignoring an absent, unknown, or version-mismatched `status.provider`; (c) strict per-layer bounds and signed/registered `status.provider.details` validation with unknown-field denial; and (d) an atomic single-mutation write of all present layers, with no shared field duplicated across provider extensions. |
| Minimal disposable artifact | `proofs/status-shape-spike/` — an in-process fake resource store (a `HashMap` oracle, not real redb) with a registered extension-schema table; two fake `Guest` implementations (`runtime-cloud-hypervisor`, `runtime-azure-container-apps`) each writing the same `status.resource` and their own `status.provider.details`; a fake base-only consumer reading only universal + `status.resource`. |
| Inputs | (a) two Guest implementations writing identical `status.resource` runtime-readiness/capability fields plus distinct `status.provider.details`; (b) a `status.provider` with an unregistered `schemaId`; (c) a `status.provider.details` carrying an unknown field; (d) a `status.provider` restating a `status.resource` field; (e) an oversize `details` (>32 KiB); (f) a base-only consumer against each of (a)-(e). |
| Command/harness | `cargo test --manifest-path proofs/status-shape-spike/Cargo.toml -- --test-threads=1 status_three_layer_shape`. |
| Metrics | (1) both Guest implementations produce a byte-identical `status.resource` shape for equivalent observed state; (2) base-only consumer reads/watches all cases successfully and never parses `details`; (3) unregistered schema → `status-provider-schema-invalid`; unknown field → rejected; overlap with `status.resource`/base → `status-provider-overlap`; oversize → `status-oversize`; (4) all present layers commit in exactly one status mutation with one expected revision; a forced partial-layer write is rejected. |
| Pass/fail threshold | All four metrics binary pass; metric (2) base-only compatibility across an unknown/newer `status.provider` is a hard zero-tolerance gate (a base-only consumer must never fail because a Provider extension changed). |
| Expected resource budget | ≤2 minutes wall time; ≤32 MiB RSS (pure in-memory shape/validation checks). |
| Failure interpretation | A base-only consumer that fails on an unknown/newer `status.provider`, or two implementations that diverge on `status.resource`, is a severity-blocking finding against D088: the shared field is either mis-placed in a provider extension (must be promoted to `status.resource`) or the base-only projection is not truly provider-neutral. Fixed structurally, never suppressed. |
| Affected decisions/work items | D027, D028, D037, D088; resource object/API/store/reconcile work items and every Provider dossier's status-schema work item. |
| Cleanup | Deleted once the real resource-contract crate (`packages/d2b-contracts`) and the provider conformance kit reach equal schema-parity, base-only-projection, and extension-version test coverage. |
| Status | Specified — not yet executed. |

## Implementation validation — how a spike is retired

Every spike above ends with a "Cleanup" row naming the exact future crate
whose own in-tree tests must reach equal or stricter coverage before the
disposable `proofs/<slug>/` crate is deleted. This is the same disposition
already used for `proofs/w0-ch-connect-proof` and
`proofs/chunked-stdio-conformance` (both remain until their own owning
implementation absorbs their coverage) and it generalizes here as a fixed
three-stage lifecycle:

1. **Spike stage.** The disposable crate under `proofs/` exists solely to
   falsify or corroborate one hypothesis with a fixed harness. It is added to
   `tests/test-proofs.sh`'s crate list (mirroring the two existing entries)
   only when it is authored — this spec does not add it, since this spec
   adds no code.
2. **Work-item stage.** The named `ADR046-*` work item (in the resource,
   controller, or Provider spec that owns the destination) copies the
   *proven shape* — not the disposable crate itself — into its real
   destination crate, and extends its own `tests/`/`integration/` suite to
   reach or exceed the spike's metrics. This is the "Validation" column of
   that work item, already required by `docs/specs/README.md`; this spec
   does not duplicate or relax it.
3. **Retirement stage.** Once the real crate's CI-gated test/bench suite
   reproduces the spike's pass/fail thresholds (or stricter), the spike
   author (or the owning work item's author) deletes the `proofs/<slug>/`
   crate and its `tests/test-proofs.sh` entry in the same change that lands
   the real coverage, so there is never a window where a deleted disposable
   proof is the only evidence for a shipped behavior.

No spike in this catalog is retired by this spec; retirement is always
performed by the owning production work item, per anti-claim rule 5.

## Current-code fit

| Item | Treatment |
| --- | --- |
| Current anchor | `docs/adr/0046-d2b-3-provider-control-plane.md` (kcp measurement, decision D003); `ADR-046-decision-register` D003-D086; `packages/d2b-core/src/{storage,sync}.rs`; `packages/d2bd/src/supervisor/{dag,pidfd}.rs`; `packages/d2b-priv-broker/src/ops/spawn_runner.rs` and `tests/pidfd_real_spawner.rs`; `packages/d2b-realm-router/src/*`; main `a1cc0b2d` `packages/d2b-session*` (re-executed in E3: 70/70 tests passing) |
| Evidence class | Mixed by claim; see §"Evidence classification matrix" above for the exact class of every subsystem this spec touches — no single class applies to the whole document |
| Behavior retained | The kcp measurement and its rejection rationale (E1); the redb design rationale (E2); the exact main reuse inventory and its currently-green test count (E3); the current v3 reachable-file inventory for storage/DAG/pidfd/router (E4) |
| Required delta | Every item in the "Evidence classification matrix" marked `unknown-requires-spike` needs its named spike executed (with a passing or revised-and-repassing result) before its owning `ADR046-*` work item may start, per anti-claim rules 1, 3, and 5 |
| Reuse path | SPIKE-06/07/08 explicitly reuse main `a1cc0b2d`'s `d2b-session`/`d2b-session-unix` crates as path dependencies from a pinned local checkout, per the same reuse policy as `ADR-046-componentsession-and-bus` |
| Replacement/deletion | No spike crate is ever a replacement for production code; each is deleted per its own Cleanup row once its owning production work item supplies equal-or-stricter in-tree coverage |
| Feasibility proof | This spec *is* the feasibility-proof registry; it has no further "proof of itself" beyond the completed evidence in E1-E4 and the reproducibility discipline in anti-claim rule 6 |
| Future owner | `ADR046-feasibility-001` through `ADR046-feasibility-010` below |

## Implementation work items

Each item below authors one or more of the 15 spikes above as a disposable
`proofs/<slug>/` crate, runs it, and records its result. None of these items
touch `packages/`, `nixos-modules/`, or any other production path; their sole
production-adjacent output is the pass/fail evidence that unblocks the
downstream `ADR046-*` work items named in each spike's "Affected
decisions/work items" row.

### ADR046-feasibility-001

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-feasibility-001` |
| Dependency/owner | W0 shared contract root; store/reconciliation integrator |
| Current source | `packages/d2b-core/src/{storage,sync}.rs` (atomic/idempotency reference shape, E4); no redb usage exists anywhere in this repository at any inspected commit |
| Reuse source | None (redb is a new external dependency; no main or v3 code implements it) |
| Reuse action | `adapt` (the atomic-write/idempotency discipline in `storage.rs`/`sync.rs` is adapted into the spike's write-transaction algorithm; redb itself is used unmodified) |
| Destination | `proofs/redb-resource-store-spike/` |
| Detailed design | Implements SPIKE-01 and SPIKE-02: the eight-table schema, fair write queue, blocking store-actor, watch registrar, and hint bus described in those two spike entries |
| Integration | None (standalone; no d2b-bus/ComponentSession/broker dependency) |
| Data migration | None (disposable fixture data only) |
| Validation | SPIKE-01 metrics (1)-(5) and SPIKE-02 metrics (1) across all 3 concurrency profiles, per those entries' exact pass/fail thresholds |
| Removal proof | Per SPIKE-01/SPIKE-02 Cleanup rows: deleted once `packages/d2b-resource-store-redb` and `packages/d2b-controller-toolkit/benches/reaction.rs` reproduce equal-or-stricter coverage |

### ADR046-feasibility-002

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-feasibility-002` |
| Dependency/owner | `ADR046-feasibility-001`; reconciliation/process integrator |
| Current source | `packages/d2bd/src/supervisor/{dag,pidfd}.rs` (current DAG/pidfd reference shape, E4) |
| Reuse source | None (the generic async controller loop is ADR-only per `ADR-046-resource-reconciliation`'s own current-code-fit row) |
| Reuse action | `adapt` (current DAG ordering/readiness concepts are adapted into the spike's per-resource single-flight/parallel-semaphore loop) |
| Destination | `proofs/process-fastlaunch-spike/` |
| Detailed design | Implements SPIKE-03: the fake Process controller loop, fake `ProcessLaunchEffectPort`, and the 1/10/100-concurrency commit-to-launch-attempt and next-dispatch-independence benchmarks |
| Integration | Consumes `ADR046-feasibility-001`'s hint-bus shape as its watch-receiver input |
| Data migration | None |
| Validation | SPIKE-03 metrics (1)-(3) and thresholds |
| Removal proof | Deleted once `packages/d2b-controller-toolkit/benches/reaction.rs` and the Process Provider integration tests named by `ADR046-reconcile-003` reproduce equal-or-stricter coverage |

### ADR046-feasibility-003

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-feasibility-003` |
| Dependency/owner | Independent of `-001`/`-002`; EffectPort/ProviderSupervisor integrator |
| Current source | `packages/d2b-priv-broker/src/ops/spawn_runner.rs` and `tests/pidfd_real_spawner.rs` (current blocking-call reference shape, E4) |
| Reuse source | None |
| Reuse action | `adapt` |
| Destination | `proofs/effectport-async-spike/` |
| Detailed design | Implements SPIKE-04: the four fake EffectPort traits, the deliberately slow blocking-primitive backends, and the current-thread-runtime heartbeat-jitter detector |
| Integration | None (standalone) |
| Data migration | None |
| Validation | SPIKE-04 heartbeat-jitter metric and threshold |
| Removal proof | Deleted once `packages/d2b-provider-supervisor` and the volume-domain effect adapter each carry an equal-or-stricter in-tree blocking-adapter regression test |

### ADR046-feasibility-004

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-feasibility-004` |
| Dependency/owner | Independent of `-001`/`-002`/`-003`; Provider packaging/toolkit integrator |
| Current source | None in v3 at `b5ddbed6` (no generic Provider registry exists; per parent ADR context, this is explicitly listed as missing) |
| Reuse source | None |
| Reuse action | `adapt` (the crate-layout policy check reuses the same `src/`/`tests/`/`integration/`/`README.md` structure already enforced elsewhere in this repository's workspace policy tests) |
| Destination | `proofs/provider-packaging-spike/` |
| Detailed design | Implements SPIKE-05: the two-binary crate, hand-authored manifest, fake `ProviderDeployment`, and the `cargo metadata` dependency-edge check |
| Integration | None (standalone) |
| Data migration | None |
| Validation | SPIKE-05 metrics (1)-(4) across 20 repeated randomized-order manifest loads |
| Removal proof | Deleted once the real Provider-toolkit crate (`ADR046-provider-001` destination) ships equal-or-stricter manifest-parsing/enumeration/workspace-policy coverage |

### ADR046-feasibility-005

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-feasibility-005` |
| Dependency/owner | Independent of `-001` through `-004`; bus/session/transport/credential integrator |
| Current source | `packages/d2b-realm-router/src/{session,secure_session,mux_session}.rs` (current routing reference shape, E4); main `a1cc0b2d` `packages/d2b-session/**`, `packages/d2b-session-unix/**` (re-verified green in E3) |
| Reuse source | main `a1cc0b2d`: `packages/d2b-session/src/{handshake,bootstrap,record,engine,scheduler,streams,lifecycle,transport}.rs`, `packages/d2b-session-unix/src/{adapter,vsock,pidfd,socket,systemd,credit,descriptor}.rs`, and the exact test files listed in E3's table |
| Reuse action | `copy-unchanged` for the Noise/record/transport machinery (path-dependency on a pinned local checkout of `a1cc0b2d`); `adapt` for the fake router/relay/credential-delivery wrapper code that SPIKE-06/07/08 add on top |
| Destination | `proofs/bus-routing-noise-spike/`, `proofs/transport-opaque-streams-spike/`, `proofs/credential-kk-e2e-spike/` |
| Detailed design | Implements SPIKE-06 (exact-addressed routing + per-recipient Noise isolation), SPIKE-07 (Unix/vsock/relay-shaped opaque byte-stream conformance across 3 backends), and SPIKE-08 (Credential Provider → consumer Provider KK delivery with the 13-field binding contract) |
| Integration | SPIKE-07's Unix backend and SPIKE-08's session machinery both depend on the same pinned `a1cc0b2d` path-dependency established for SPIKE-06 |
| Data migration | None |
| Validation | SPIKE-06 metrics (1)-(3), SPIKE-07 metrics (1)-(4) against the exact numeric gates already committed in the transport-unix/vsock dossiers, SPIKE-08 metrics (1)-(6) |
| Removal proof | Deleted per each spike's own Cleanup row: `packages/d2b-bus/src/router.rs` for SPIKE-06; the three real transport Provider crates for SPIKE-07; the real Credential Provider crates for SPIKE-08 |

### ADR046-feasibility-006

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-feasibility-006` |
| Dependency/owner | Independent of `-001` through `-005`; Provider-state/Volume integrator |
| Current source | None in v3 at `b5ddbed6` (ProviderStateSet and the generalized Volume ResourceType are both ADR-only) |
| Reuse source | None |
| Reuse action | `adapt` |
| Destination | `proofs/provider-state-export-spike/`, `proofs/volume-policy-spike/` |
| Detailed design | Implements SPIKE-09 (optional declared state-Volume creation order, guest-local/host-backed-guest placement, virtiofs Export child ownership) and SPIKE-10 (Volume ACL/`sourcePolicyId`/quota/lifecycle-marker policy conformance) |
| Integration | None between the two spikes beyond sharing the same fake resource-store oracle shape |
| Data migration | None |
| Validation | SPIKE-09 metrics (1)-(4); SPIKE-10 metrics (1)-(5), zero-tolerance on path leakage |
| Removal proof | Deleted per each spike's Cleanup row: the real `ADR-046-provider-state` work-item destination for SPIKE-09; `d2b-provider-volume-local`'s own `tests/`/`integration/` suite for SPIKE-10 |

### ADR046-feasibility-007

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-feasibility-007` |
| Dependency/owner | Independent of `-001` through `-006`; Process Provider integrator |
| Current source | `packages/d2b-priv-broker/src/ops/spawn_runner.rs` (real-spawn reference shape, E4); current unsafe-local helper runtime/systemd invocation shape |
| Reuse source | None |
| Reuse action | `adapt` |
| Destination | `proofs/process-provider-conformance-spike/` |
| Detailed design | Implements SPIKE-11: the shared `ProcessProviderHarness` trait, the minijail-shaped `clone3(CLONE_PIDFD)` launcher, and the systemd transient-user-scope launcher, plus the identity-drift/quarantine and clean-exit cases |
| Integration | None (standalone; requires a Linux host with `clone3`/`pidfd_open`, and optionally a running `systemd --user` instance behind the `systemd-user` feature) |
| Data migration | None |
| Validation | SPIKE-11 metrics (1)-(4), zero-tolerance on false adoption |
| Removal proof | Deleted once `packages/d2b-provider-system-systemd` and `packages/d2b-provider-system-minijail` each carry this exact shared conformance suite in their own `tests/` |

### ADR046-feasibility-008

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-feasibility-008` |
| Dependency/owner | Independent of `-001` through `-007`; Nix/xtask integrator |
| Current source | `nixos-modules/assertions.nix` pattern, `packages/xtask` `gen-schemas` pattern, `make test-drift` gate (existing generated-or-eval-contract precedent) |
| Reuse source | None |
| Reuse action | `adapt` |
| Destination | `proofs/nix-authoring-spike/` |
| Detailed design | Implements SPIKE-12: the minimal flake, the two synthetic ResourceTypes, the hand-written committed schemas, the standalone `gen-schemas`-shaped drift check, and the two-generation removed-resource cleanup simulation |
| Integration | None (standalone flake; no dependency on the main `flake.nix`) |
| Data migration | None |
| Validation | SPIKE-12 metrics (1)-(5), byte-for-byte reproducibility across 3 hermetic builds |
| Removal proof | Deleted once the real `nixos-modules/resources.nix` and `packages/xtask` `gen-schemas` implementation reproduce these metrics as part of `make test-drift`/`make test-flake` |

### ADR046-feasibility-009

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-feasibility-009` |
| Dependency/owner | Independent of `-001` through `-008`; CLI integrator |
| Current source | `packages/d2b/src/lib.rs` `cmd_audio`/`cmd_clipboard_arm` and the current command-table shape (current CLI reference shape) |
| Reuse source | None |
| Reuse action | `adapt` |
| Destination | `proofs/cli-discovery-spike/`, `proofs/clean-cutover-spike/` |
| Detailed design | Implements SPIKE-13 (dynamic Provider-projection discovery, bounds, latency isolation) and SPIKE-14 (zero v2 dispatch, fresh Zone bootstrap ignoring legacy state) |
| Integration | None between the two spikes beyond sharing the same fixture command-table shape |
| Data migration | None |
| Validation | SPIKE-13 metrics (1)-(7); SPIKE-14 metrics (1)-(3), zero-tolerance on legacy-file access |
| Removal proof | Deleted per each spike's Cleanup row: the real `d2b` CLI crate's own discovery conformance test for SPIKE-13; the real CLI crate's workspace-policy/lint gate plus the real bootstrap sequence for SPIKE-14 |

### ADR046-feasibility-010

| Field | Value |
| --- | --- |
| Work item ID | `ADR046-feasibility-010` |
| Dependency/owner | `ADR046-feasibility-001` through `ADR046-feasibility-009` (integrates their fakes; must run last) |
| Current source | None (this is a pure integration of the other nine work items' fixtures) |
| Reuse source | None beyond what `-001` through `-009` already reuse |
| Reuse action | `adapt` |
| Destination | `proofs/e2e-composition-spike/` |
| Detailed design | Implements SPIKE-15: the three representative compositions (local/cloud-hypervisor, cloud/azure, interaction/shell-terminal-or-wayland), wired from the fakes built by `-001` through `-009`, plus the combined 3-Zone aggregate RSS measurement |
| Integration | Depends on and imports the fake shapes from `proofs/redb-resource-store-spike/`, `proofs/process-fastlaunch-spike/`, `proofs/effectport-async-spike/`, `proofs/provider-packaging-spike/`, `proofs/bus-routing-noise-spike/`, `proofs/transport-opaque-streams-spike/`, `proofs/credential-kk-e2e-spike/`, `proofs/provider-state-export-spike/`, `proofs/volume-policy-spike/`, and `proofs/process-provider-conformance-spike/` |
| Data migration | None |
| Validation | SPIKE-15 metrics (1)-(4) across all three compositions |
| Removal proof | Deleted once the real integration test suites named by the individual Provider dossiers (`integration/` per D059) collectively reproduce all three compositions against real, non-fake Zone/store/bus/broker code |
