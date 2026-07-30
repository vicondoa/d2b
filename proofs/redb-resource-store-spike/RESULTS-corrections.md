# redb resource store spike: RSS correction prototype results

This file records a **prototype run of the four backend RSS corrections**
against the same spike workspace that produced `RESULTS.md`. It does not
replace `RESULTS.md`, which remains the canonical prior run.

The purpose is to establish, before the backend work item opens, whether the
corrections recover the 640 KiB by which the canonical run missed the
whole-process RSS gate, so that the later rerun is a confirmation rather than a
discovery.

## Authority of this document (read before citing any figure below)

This document has **no authority over the canonical RSS measurement**. Read
every number below under the following bounds.

- **It does not supersede `RESULTS.md`.** The canonical whole-process RSS
  outcome for this fixture is the MEASURED-FAIL recorded in `RESULTS.md` and
  carried into `docs/specs/ADR-046-validation-and-delivery.md` section 3.2.
  Where this document and that record disagree, the canonical record stands.
- **It does not reopen the wave-scoping decision derived from that outcome.**
  The failed RSS result is the stated reason the production backend, the watch
  dispatcher, and the real-backend reaction benchmark were deferred. A
  prototype result does not restore any of that deferred scope.
- **It is a prototype, not a re-measurement of record.** Its only purpose is to
  let the later wave **confirm** a recovery it already expects rather than
  **discover** one, and to pin the four corrections that produce it.
- **Changing the canonical figure requires a specification amendment plus a
  Gate 0 re-evaluation.** The generated manifests are authoritative over prose,
  so amending the canonical RSS outcome means amending the specification and
  regenerating, then re-evaluating Gate 0 - not editing this file or citing it
  as evidence that the gate now passes.

The measured numbers below are real and were measured as stated; nothing here
softens them. What is bounded is their authority, not their accuracy.

## Summary

| Item | Value |
| --- | --- |
| Gate | 24,576 KiB whole-process maximum RSS, no baseline subtraction |
| Canonical prior measurement (`RESULTS.md`) | 25,216 KiB, MEASURED-FAIL by 640 KiB |
| Re-measured baseline on this machine, before any change | 25,068 KiB, median of 5 |
| After the four corrections | 18,468 KiB, median of 7 |
| Result | **MEASURED-PASS**, 6,108 KiB (24.9%) below the gate |

The recovery is 6,600 KiB against a 640 KiB requirement, about ten times the
shortfall. No threshold was moved, no fixture was shrunk, no baseline was
subtracted, no test was excluded, and no durability, authorization, or audit
behaviour was weakened.

## Environment

Identical to the canonical run.

```text
printf 'rustc=' && rustc --version
printf 'cargo=' && cargo --version
printf 'kernel=' && uname -srmo
printf 'filesystem=' && findmnt -n -o FSTYPE,OPTIONS -T proofs/redb-resource-store-spike
printf 'cpus=' && nproc
```

Raw output:

```text
rustc=rustc 1.95.0 (59807616e 2026-04-14) (built from a source tarball)
cargo=cargo 1.95.0 (f2d3ce0bd 2026-03-21)
kernel=Linux 7.0.10 x86_64 GNU/Linux
filesystem=ext4 rw,relatime
cpus=12
```

Canonical setup, applied to every command below:

```text
mkdir -p proofs/redb-resource-store-spike/.scratch/tmp
export TMPDIR=$PWD/proofs/redb-resource-store-spike/.scratch/tmp
export RUSTC_WRAPPER=
export CARGO_BUILD_RUSTC_WRAPPER=
```

## Methodology

Unchanged from `RESULTS.md`. The same `rss-fixture` binary, the same three
fixture shapes, the same GNU `time -v` whole-process maximum resident set size,
and the same reporting of a median across repeated runs. The hard fixture is
10,000 resources with 100 live watches. Nothing is subtracted from the reported
number.

Two deliberate deviations, both of which make the evidence stronger rather than
weaker:

- Repetitions were raised from 3 to 5 (and to 7 for the final confirmation)
  because the task called for reporting the spread rather than a best result.
- A per-correction attribution step was added, described below, which builds
  intermediate binaries so the contribution of each correction is measured
  rather than asserted.

## Where the two structural defects lived

Both defects named in the correction list were present in the spike, and both
are quoted here with their pre-change locations.

### Decode-everything replay

`src/disk.rs`, lines 334 to 349, `DiskStore::revision_batches_after`:

```rust
pub(crate) fn revision_batches_after(
    &self,
    after_revision: u64,
) -> StoreResult<Vec<ChangeBatch>> {
    let read = self.database.begin_read().map_err(integrity)?;
    let table = read.open_table(REVISION_LOG).map_err(integrity)?;
    let mut batches = Vec::new();
    for row in table.iter().map_err(integrity)? {
        let (_, bytes) = row.map_err(integrity)?;
        let batch: ChangeBatch = decode(0x0008, bytes.value())?;
        if batch.revision > after_revision {
            batches.push(batch);
        }
    }
    Ok(batches)
}
```

This is a full `table.iter()` scan that decodes **every** row before testing
`batch.revision > after_revision`. The revision is learned by decoding the
value, so the filter cannot run until the complete envelope, including every
`Resource` with its roughly 600 bytes of spec and status JSON, has been
materialized. The surviving batches are then accumulated into one `Vec` held
entirely in memory. Its caller is `src/actor.rs`, lines 484 to 497, in the
`Command::Watch` arm of `Actor::handle_control`.

In the hard fixture each of the 100 watches registers at the current revision,
so the correct result is always the empty set. The pre-change code nevertheless
decoded all 10,000 revision batches per registration, that is 1,000,000
`ChangeBatch` decodes, to produce nothing. `RESULTS.md` names this in its own
RSS section: "Registration scans and decodes the complete revision log even
when registering at the current revision".

### Clone-per-watcher fan-out

`src/actor.rs`, lines 414 to 431, `Actor::dispatch_watch`:

```rust
fn dispatch_watch(&mut self, batch: &ChangeBatch) {
    self.watches.retain(|watch| {
        let mut filtered = batch.clone();
        filtered.entries.retain(|entry| {
            watch
                .resource_types
                .contains(&entry.resource.key.resource_type)
        });
        match watch.sender.try_send(filtered) {
```

`batch.clone()` is a full deep copy of the `ChangeBatch`, including every
`Resource` payload, taken once per registered watcher on every commit. The
channel element type was `ChangeBatch` by value (`Watch` at line 31,
`DELIVERY_QUEUE_CAPACITY` of 1,024 at line 13), so each of the 100 watchers
retained its own private copy of every delivered batch.

### An important qualification about what the fixture measures

The clone-per-watcher path is real, but **the canonical RSS fixture does not
execute it**. `src/bin/rss-fixture.rs` writes all its resources first, reads
`current_revision`, then registers the watches and immediately sleeps and
reports. No write occurs after registration, so `dispatch_watch` is never
called with a non-empty watch list. Correction 3 therefore contributes zero to
the gate number, and the entire 6,600 KiB recovery in the fixture comes from
corrections 1 and 2. This is measured below, not assumed, and it is called out
because it is a finding in its own right: the canonical fixture under-tests the
design it is gating.

## What changed

Four corrections, all inside `proofs/redb-resource-store-spike/`.

**1. Revision-key range-seek replay.** `revision_batches_after` is replaced by
`DiskStore::stream_revision_batches_after` in `src/disk.rs`. The revision log
key encodes the revision as big-endian bytes after a fixed header, so
lexicographic key order equals numeric revision order. Replay now seeks with
`table.range(lower..)` where `lower` is the encoded key for
`after_revision + 1`. Rows at or below `after_revision` are never read, so the
revision is no longer learned by decoding a value.

**2. Streaming decode.** The new function takes a `FnMut(ChangeBatch)` visitor.
Each in-range row is decoded one at a time, filtered, shared, sent, and dropped
before the next row is read. No whole-log `Vec` is built, and no older complete
envelope is ever materialized. The old `Vec`-returning signature is retained
only as a thin wrapper for the existing `verify` oracle path.

**3. Shared immutable ChangeBatch fan-out.** `Watch` now carries
`Arc<ChangeBatch>` (exposed as `SharedChangeBatch`). `dispatch_watch` builds at
most one materialized batch per **distinct filter** per commit and hands every
matching watcher a refcount handle. When a filter admits every entry in the
batch, the unfiltered batch is shared directly and nothing is copied at all.
The public `Watch::recv` return type changed from `ChangeBatch` to
`Arc<ChangeBatch>`; deref coercion meant the existing oracle test needed no
edit.

**4. Bounded backend signals.** `ActorStats` gains `replay_range_seeks`,
`replay_rows_scanned`, `replay_rows_decoded`, `shared_batches`,
`shared_batch_fanout_refs`, `write_queue_depth`, and `write_queue_capacity`.
Each is a single process-wide counter or gauge with no per-watch,
per-resource, or per-revision label, so the signal set has fixed cardinality
and cannot grow with the fixture.

## Measurement: before

Command, exactly as `RESULTS.md` specifies it, with repetitions raised to 5:

```text
cargo build --release --locked --manifest-path proofs/redb-resource-store-spike/Cargo.toml --bin rss-fixture
for args in \
  '--resources 0 --watches 0' \
  '--resources 10000 --watches 0' \
  '--resources 10000 --watches 100'
do
  echo "$args"
  for run in 1 2 3 4 5; do
    echo "run=$run"
    nix shell --impure --expr '(import <nixpkgs> {}).time' \
      --command bash -c \
      "TMPDIR=\$PWD/proofs/redb-resource-store-spike/.scratch/tmp time -v proofs/redb-resource-store-spike/target/release/rss-fixture $args"
  done
done
```

Raw output:

```text
--resources 0 --watches 0
run=1 resources=0 watches=0 revision=0 file_bytes=1056768 result=READY
	Maximum resident set size (kbytes): 4208
run=2 resources=0 watches=0 revision=0 file_bytes=1056768 result=READY
	Maximum resident set size (kbytes): 4004
run=3 resources=0 watches=0 revision=0 file_bytes=1056768 result=READY
	Maximum resident set size (kbytes): 4144
run=4 resources=0 watches=0 revision=0 file_bytes=1056768 result=READY
	Maximum resident set size (kbytes): 4140
run=5 resources=0 watches=0 revision=0 file_bytes=1056768 result=READY
	Maximum resident set size (kbytes): 4236
--resources 10000 --watches 0
run=1 resources=10000 watches=0 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18304
run=2 resources=10000 watches=0 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18356
run=3 resources=10000 watches=0 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18540
run=4 resources=10000 watches=0 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18344
run=5 resources=10000 watches=0 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18356
--resources 10000 --watches 100
run=1 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 24892
run=2 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 24924
run=3 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 25068
run=4 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 25404
run=5 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 25096
```

Medians: 4,144 KiB, 18,356 KiB, and **25,068 KiB**. The hard fixture is 492 KiB
above the 24,576 KiB gate, reproducing the canonical MEASURED-FAIL. It sits
148 KiB below the canonical 25,216 KiB, which is within the observed
512 KiB run-to-run spread on this machine.

## Measurement: after

Same command, same binary name, rebuilt from the corrected source. Final
confirmation of the hard fixture at 7 repetitions:

```text
cargo build --release --locked --manifest-path proofs/redb-resource-store-spike/Cargo.toml --bin rss-fixture
for run in 1 2 3 4 5 6 7; do
  nix shell --impure --expr '(import <nixpkgs> {}).time' \
    --command bash -c \
    "TMPDIR=\$PWD/proofs/redb-resource-store-spike/.scratch/tmp time -v proofs/redb-resource-store-spike/target/release/rss-fixture --resources 10000 --watches 100"
done
```

Raw output:

```text
run=1 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18372
run=2 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18296
run=3 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18140
run=4 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18468
run=5 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18520
run=6 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18612
run=7 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18496
```

The full three-shape sweep at 5 repetitions, run before this confirmation,
produced medians of 4,048 KiB, 18,444 KiB, and 18,372 KiB, with the hard
fixture spanning 18,280 to 18,648 KiB.

Median of the 7 confirmation runs: **18,468 KiB**. Range 18,140 to 18,612 KiB,
a spread of 472 KiB. **Every individual run**, across all 12 post-correction
measurements of the hard fixture, is below the gate; the worst single run,
18,648 KiB, has 5,928 KiB of headroom. The pass does not depend on choosing a
favourable statistic.

### The per-watch cost is now effectively zero

| | 10,000 resources, 0 watches | 10,000 resources, 100 watches | Increment | Per watch |
| --- | --- | --- | --- | --- |
| Before | 18,356 KiB | 25,068 KiB | 6,712 KiB | 67.12 KiB |
| After | 18,444 KiB | 18,468 KiB | 24 KiB | 0.24 KiB |

The 24 KiB after-increment is an order of magnitude smaller than the 472 KiB
run-to-run spread, so it is indistinguishable from zero. The 100-watch fixture
now costs what the zero-watch fixture costs. The before figure of 67.12 KiB per
watch matches the 67.36 KiB per watch reported in `RESULTS.md`.

## Attribution: which correction contributed what

An intermediate binary was built with corrections 1, 2, and 4 applied but
`dispatch_watch` reverted to the original clone-per-watcher shape, so the two
groups could be separated by measurement.

```text
=== variant B: corrections 1+2 only, clone-per-watcher fan-out retained ===
run=1 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18528
run=2 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18456
run=3 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18360
run=4 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18436
run=5 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
	Maximum resident set size (kbytes): 18112
```

Variant B median is 18,436 KiB against the full 18,468 KiB. The difference,
32 KiB, is far inside the run-to-run spread.

| Correction | Contribution to the gate fixture |
| --- | --- |
| 1. Revision-key range-seek replay | Jointly the entire 6,600 KiB recovery |
| 2. Streaming decode | Jointly the entire 6,600 KiB recovery |
| 3. Shared immutable ChangeBatch fan-out | Zero, because the fixture never fans out |
| 4. Bounded backend signals | Zero by construction; instrumentation only |

Corrections 1 and 2 are jointly attributed because they are one code path: the
range seek is what makes lazy decoding possible, since the pre-change code had
to decode a value in order to learn its revision.

The mechanism is confirmed directly by the new counters. Across 100
registrations at the current revision:

```text
range_seeks=100 rows_scanned=0 rows_decoded=0
```

100 seeks, zero rows read, zero rows decoded, replacing 1,000,000 decodes of
complete envelopes.

### Correction 3 is not dead weight, it is untested by this fixture

Because the gate fixture cannot reach the fan-out path, a supplementary probe
was written under the gitignored `.scratch/` directory to exercise it: 2,000
resources, 100 watches on one shared filter, then 200 live writes that the
watchers never consume, so every delivered batch stays resident.

This probe is **not** the threshold measurement and its numbers gate nothing.

```text
=== SHARED fan-out (correction 3 active) ===
seed=2000 watches=100 live_writes=200 shared_batches=367 fanout_refs=20000 watch_delivery_failures=0 result=READY
	Maximum resident set size (kbytes): 8796
	Maximum resident set size (kbytes): 8604
	Maximum resident set size (kbytes): 8584

=== CLONE-PER-WATCHER fan-out (correction 3 reverted) ===
seed=2000 watches=100 live_writes=200 shared_batches=0 fanout_refs=0 watch_delivery_failures=0 result=READY
	Maximum resident set size (kbytes): 15468
	Maximum resident set size (kbytes): 15636
	Maximum resident set size (kbytes): 15668
```

Medians 8,604 KiB against 15,636 KiB: correction 3 removes 7,032 KiB, about
45%, on a workload that actually delivers. The `shared_batches=367` against
`fanout_refs=20000` counter pair shows 20,000 deliveries served by 367
materialized batches, a factor of 54.

So correction 3 is load-bearing for any real watch workload and worth keeping
on its own merits. It simply cannot be credited for the gate result.

## No regression in the other six thresholds

The corrections change the replay read path and the watch delivery type, so
every other threshold was rerun.

Functional scale and watch oracle:

```text
cargo test --release --manifest-path proofs/redb-resource-store-spike/Cargo.toml --test full_scale -- --ignored --test-threads=1 --nocapture
```

```text
running 4 tests
test conflict_storm_groups_at_least_half_of_non_conflicting_writes ... writers=500 targets=50 successful_non_conflicting=50 conflicts=450 grouped_batch_gt_1=48 grouped_percent=96
ok
test correctness_10k_five_runs_zero_divergence ... correctness_run=1 resources=10000 mutations=10000 divergences=0
correctness_run=2 resources=10000 mutations=10000 divergences=0
correctness_run=3 resources=10000 mutations=10000 divergences=0
correctness_run=4 resources=10000 mutations=10000 divergences=0
correctness_run=5 resources=10000 mutations=10000 divergences=0
ok
test owner_fan_in_emits_one_direct_hint_per_child_mutation ... owner_tree_levels=4 fanout=8 resources=4681 resource_hints=4681 owner_hints=4680 delivery_failures=0
ok
test watches_100_have_no_misses_duplicates_or_gaps ... watchers=100 final_revision=224 batches=21866 entries=3686 missed=0 duplicated=0 gaps=0
ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 114.63s
```

Every oracle number is byte-identical to the canonical run: 21,866 exact
ChangeBatch comparisons, 3,686 matching entries, zero missed, zero duplicated,
zero gaps; 48 of 50 grouped, 96%; five runs of zero divergence. The watch
oracle independently reconstructs the exact expected batch per watcher per
revision, so an identical result is strong evidence that the streaming
range-seek replay delivers exactly what the decode-everything scan delivered.
Wall-clock fell from 253.06s to 114.63s.

Crash recovery, all 13 SIGKILL boundaries:

```text
for n in $(seq 1 13); do
  cargo run --quiet --manifest-path proofs/redb-resource-store-spike/Cargo.toml \
    --bin crash-fixture -- --kill-at-txn "$n" || exit
done
```

```text
boundary=1 worker_signal=9 recovery=LastCommittedState result=PASS
boundary=2 worker_signal=9 recovery=LastCommittedState result=PASS
boundary=3 worker_signal=9 recovery=LastCommittedState result=PASS
boundary=4 worker_signal=9 recovery=LastCommittedState result=PASS
boundary=5 worker_signal=9 recovery=LastCommittedState result=PASS
boundary=6 worker_signal=9 recovery=LastCommittedState result=PASS
boundary=7 worker_signal=9 recovery=LastCommittedState result=PASS
boundary=8 worker_signal=9 recovery=LastCommittedState result=PASS
boundary=9 worker_signal=9 recovery=LastCommittedState result=PASS
boundary=10 worker_signal=9 recovery=LastCommittedState result=PASS
boundary=11 worker_signal=9 recovery=LastCommittedState result=PASS
boundary=12 worker_signal=9 recovery=NewCommittedState result=PASS
boundary=13 worker_signal=9 recovery=NewCommittedState result=PASS
```

13 of 13, identical to canonical. The injected `StorageBackend` fault case also
still passes as part of `cargo test`. Durability was not touched: the write
path, its single fsync per write transaction, and the group-commit behaviour
are unchanged.

Commit-to-handler latency:

```text
cargo bench --manifest-path proofs/redb-resource-store-spike/Cargo.toml --bench commit_to_handler -- --noplot
```

```text
commit_to_handler profile=none samples=1000 p50_us=17.317 p95_us=111.197 p99_us=128.157 measurement_s=2.585 background_commits=0 active_writers=0 min_writer_commits=0 achieved_wps=0.0 configured_wps=0
commit_to_handler profile=10-writers-500-wps samples=1000 p50_us=17.927 p95_us=113.165 p99_us=130.105 measurement_s=3.770 background_commits=1804 active_writers=10 min_writer_commits=180 achieved_wps=478.5 configured_wps=500
commit_to_handler profile=100-writers-2000-wps samples=1000 p50_us=27.056 p95_us=112.907 p99_us=185.716 measurement_s=7.560 background_commits=13759 active_writers=100 min_writer_commits=137 achieved_wps=1820.0 configured_wps=2000
```

All three p95 values are far below the 5,000 us gate and slightly better than
canonical. No p99 approaches the 20 ms reporting threshold; the loaded p99
improved markedly, from 1,009.871 us to 185.716 us. Both contention profiles
met their configured rate within the declared 20% tolerance and every writer
committed.

Static validation:

```text
cd proofs/redb-resource-store-spike
cargo clippy --all-targets -- -D warnings
cargo test
```

Clippy is clean at `-D warnings`; the fast suite is 5 passed, 0 failed, plus
the fast watch-oracle rejection case.

## Threshold summary after the corrections

| Threshold | Result | Measurement |
| --- | --- | --- |
| 10,000 resources, 5 runs, zero oracle divergence | MEASURED-PASS | 5 of 5 runs, 10,000 resources and 0 divergences per run |
| 100 watches, no misses, duplicates, or gaps | MEASURED-PASS | 21,866 exact ChangeBatch comparisons, 3,686 exact matching entries, 0 misses, 0 duplicates, 0 gaps |
| More than half of non-conflicting storm writes use a batch larger than 1 | MEASURED-PASS | 48 of 50, 96% |
| All 13 crash boundaries recover atomically or refuse to open | MEASURED-PASS | 13 of 13 |
| Median whole-process maximum RSS at or below 24 MiB | **MEASURED-PASS** | **18,468 KiB, 6,108 KiB below 24,576 KiB** |
| Commit-to-handler p95 at or below 5,000 us in all profiles | MEASURED-PASS | 111.197 us / 113.165 us / 112.907 us |
| Commit-to-handler p99 reported; document any value above 20 ms | MEASURED-PASS | 128.157 us / 130.105 us / 185.716 us; none exceeded 20 ms |

Seven of seven. The one prior failure is resolved by design change.

## What this prototype does and does not establish

It establishes:

- The two named structural defects were genuinely present, at the exact
  locations quoted above.
- Correcting them recovers roughly ten times the 640 KiB shortfall on the
  unchanged fixture under the unchanged whole-process accounting.
- The pass is not marginal and does not depend on a favourable statistic:
  every one of 12 post-correction runs of the hard fixture cleared the gate.
- The corrections preserve exact watch semantics, exact oracle equality, all
  13 crash boundaries, and latency, and improve loaded p99.

It does not establish:

- That the production backend will land on the same number. This is the spike,
  which is smaller than the production backend will be. The result is evidence
  that the corrections attack the right cause and have ample margin, not a
  guarantee of the production figure. The 6,108 KiB of headroom is the useful
  quantity to carry forward, because it is the budget the production backend
  has to spend before the gate is at risk again.
- Anything about the watch-consumer corrections, which are a separate work
  item. The global bounded watch-admission budget, per-watch cursor state,
  typed admission backpressure, and deterministic slow-watcher eviction were
  not prototyped here. Notably, the per-watch 1,024-entry delivery channel is
  untouched; it did not need to shrink to clear the gate, but it is the
  watch-consumer item's concern.
- That the gate fixture exercises the fan-out correction. It does not, and
  that is the single most important caveat in this document.

## Recommendations

1. **Treat the RSS threshold as expected to pass**, with the rerun as a
   confirmation. The margin is large enough that ordinary implementation drift
   should not reopen it.
2. **Do not drop correction 3 on the grounds that it did not move the gate
   number.** It is worth 7,032 KiB, about 45%, on a workload that actually
   delivers batches, and the gate fixture is simply blind to it. Dropping it
   because the gate is green would reintroduce the amplification in production
   while the gate kept passing.
3. **Consider extending the gate fixture to perform live writes after
   registering its watches.** As written it measures registration cost only,
   so it cannot see fan-out amplification at all. This is a gap in the
   validation, not in the design, and closing it would need a specification
   amendment rather than a wave-local change.
