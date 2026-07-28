# redb resource store spike results

Measured on 2026-07-27 in
`/home/paydro/projects/d2b-w1-spike` with Rust 1.94.1 and redb 4.1.0.
Commands set `TMPDIR=$PWD/.scratch/tmp`, `RUSTUP_TOOLCHAIN=1.94.1`,
`RUSTC_WRAPPER=`, and `CARGO_BUILD_RUSTC_WRAPPER=`. The host reported
12 logical CPUs with `nproc`; CPU pinning was not applied.

## Threshold summary

| Threshold | Result | Measurement |
| --- | --- | --- |
| 10,000 resources, 5 runs, zero oracle divergence | MEASURED-PASS | 5/5 runs, 10,000 resources and 0 divergences per run |
| 100 watches, no misses, duplicates, or gaps | MEASURED-PASS | 21,866 ChangeBatch deliveries, 3,686 matching entries, 0 misses, 0 duplicates, 0 gaps |
| More than half of non-conflicting storm writes use a batch larger than 1 | MEASURED-PASS | 47/50, 94% |
| All 13 crash boundaries recover atomically or refuse to open | MEASURED-PASS | 13/13; boundaries 1-11 reopened at revision 1, boundaries 12-13 reopened at revision 2 |
| Median maximum RSS at or below 24 MiB | MEASURED-FAIL | 25,068 KiB (24.48 MiB), 492 KiB or about 2.0% over the 24,576 KiB threshold |
| Commit-to-handler p95 at or below 5,000 us in all profiles | MEASURED-PASS | 110.640 us / 109.250 us / 12.640 us |
| Commit-to-handler p99 reported; document any value above 20 ms | MEASURED-PASS | 125.597 us / 123.210 us / 36.546 us; none exceeded 20 ms |

No latency result requires an admission-fairness or post-commit dispatcher
change. The whole-process RSS result requires the physical schema and watch
serialization plan to change before `ADR046-store-004` starts.

## Functional scale metrics

Command:

```text
cargo test --release --manifest-path proofs/redb-resource-store-spike/Cargo.toml --test full_scale -- --ignored --test-threads=1 --nocapture
```

Raw output:

```text
Finished `release` profile [optimized] target(s) in 0.34s
Running tests/full_scale.rs (proofs/redb-resource-store-spike/target/release/deps/full_scale-b2360e35c86d37e9)

running 4 tests
test conflict_storm_groups_at_least_half_of_non_conflicting_writes ... writers=500 targets=50 successful_non_conflicting=50 conflicts=450 grouped_batch_gt_1=47 grouped_percent=94
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

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 100.11s
```

The correctness case compares all resources plus type, owner, producer, and
controller indexes and the contiguous revision log against a `BTreeMap` oracle
after every mutation, using point values and table cardinality to establish the
inductive transition and a complete scan at the end of each run. The watch case
registers 100 mixed-filter watchers while writes proceed; actor serialization
closes the replay/live gap and empty filtered ChangeBatches preserve Zone
revision continuity. The conflict test starts all 500 writers behind one
barrier, gets exactly one success for each of 50 resources, and computes the
batch percentage only over those 50 non-conflicting writes.

## Crash recovery

Command:

```text
for n in $(seq 1 13); do cargo run --quiet --manifest-path proofs/redb-resource-store-spike/Cargo.toml --bin crash-fixture -- --kill-at-txn "$n" || exit; done
```

Raw output:

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

Every invocation prepared a fresh database with one committed resource, spawned
a worker, delivered real `SIGKILL` at the selected write-algorithm boundary,
and reopened the file. Verification rejects an empty database, a missing
baseline resource, a partial index, or a noncontiguous revision log.

## RSS

GNU time was supplied through nix after the original `/usr/bin/time` attempt
failed. Each command was run three times from
`proofs/redb-resource-store-spike/`:

```text
nix shell --impure --expr '(import <nixpkgs> {}).time' --command bash -c 'TMPDIR=$PWD/.scratch/tmp time -v ./target/release/rss-fixture --resources 0 --watches 0'
nix shell --impure --expr '(import <nixpkgs> {}).time' --command bash -c 'TMPDIR=$PWD/.scratch/tmp time -v ./target/release/rss-fixture --resources 10000 --watches 0'
nix shell --impure --expr '(import <nixpkgs> {}).time' --command bash -c 'TMPDIR=$PWD/.scratch/tmp time -v ./target/release/rss-fixture --resources 10000 --watches 100'
```

Raw maximum-RSS lines from three fresh runs per configuration:

```text
--resources 0 --watches 0
Maximum resident set size (kbytes): 4092
Maximum resident set size (kbytes): 4048
Maximum resident set size (kbytes): 4028

--resources 10000 --watches 0
Maximum resident set size (kbytes): 18468
Maximum resident set size (kbytes): 18276
Maximum resident set size (kbytes): 18020

--resources 10000 --watches 100
Maximum resident set size (kbytes): 24924
Maximum resident set size (kbytes): 25264
Maximum resident set size (kbytes): 25068
```

The computed medians are 4,048 KiB, 18,276 KiB, and 25,068 KiB respectively.
The headline whole-process median is 25,068 KiB (24.48 MiB), so it is
MEASURED-FAIL: 492 KiB, about 2.0%, above the 24,576 KiB threshold. The wording
"store+actor alone" also permits a baseline-subtracted reading. Subtracting the
4,048 KiB empty-process median yields 21,020 KiB (20.53 MiB), which passes with
3,556 KiB (3.47 MiB) of margin. Both readings are recorded because the
threshold does not say whether Rust runtime/process overhead counts. The
parenthetical reserving the remainder of one 64 MiB aggregate budget for other
controller processes most naturally implies the conservative whole-process
reading: every deployed process has runtime overhead that must be budgeted.
The panel must settle this ambiguity; this spike does not silently choose the
baseline-subtracted pass.

The measured increments are 14,228 KiB for 10,000 resources, about 1.42 KiB
per resource, and 6,792 KiB for 100 watches, about 67.92 KiB per watch. Resource
encoding is therefore not the observed pressure. The watch registrar creates
one Tokio mpsc channel with logical capacity 1,024 per watcher. Tokio initially
allocates one message block rather than all 1,024 slots, so channel capacity
alone does not explain the full delta. Registration also calls
`revision_batches_after`, which scans and JSON-decodes all 10,000 revision-log
values before comparing `afterRevision`; the RSS fixture registers at the
current revision, so those 100 full scans deliver nothing but still raise redb
page and allocator high-water RSS. No revision-log slice remains retained: the
returned replay vectors are empty. The measured per-watch slope therefore
captures the dedicated channel state plus repeated full-log replay work and
retained allocator/page high-water, not a persistent 68 KiB replay buffer.

## Design implications for ADR046-store-004

Treat the whole-process miss as the gate result until the panel resolves the
wording. Before `ADR046-store-004`, revise the physical-schema/serialization
plan so replay seeks the big-endian revision key and streams only rows after
`afterRevision`, without decoding older complete envelopes. Live fan-out should
share one immutable decoded ChangeBatch rather than cloning its envelope per
watcher. The 1,024-entry watch-dispatch bound should be a global budget with
small per-watch cursor/filter state, not a fresh logical 1,024-entry channel
for every registration.

`ADR046-store-002` should make that global admission and memory bound explicit,
including typed backpressure or slow-watcher eviction when the shared budget
is exhausted. `ADR046-store-004` should implement revision-key range replay,
streaming decode, shared batch fan-out, and bounded watch registration, then
repeat the three RSS configurations before the production backend is accepted.

## Commit-to-handler latency

Command:

```text
cargo bench --manifest-path proofs/redb-resource-store-spike/Cargo.toml -- commit_to_handler
```

Raw measured percentile lines:

```text
commit_to_handler profile=none samples=1000 p50_us=74.514 p95_us=110.640 p99_us=125.597
commit_to_handler profile=10-writers-500-wps samples=1000 p50_us=75.370 p95_us=109.250 p99_us=123.210
commit_to_handler profile=100-writers-2000-wps samples=1000 p50_us=5.389 p95_us=12.640 p99_us=36.546
```

Criterion also exercised the real write, durable redb commit, actor dispatch,
consumer wake, and hint receive path during its 1,000-sample benchmark
configuration. Its wall-time estimates include the full write rather than only
the requested post-commit interval:

```text
commit_to_handler/none time: [2.8120 ms 2.8586 ms 2.9060 ms]
commit_to_handler/10-writers-500-wps time: [849.87 us 913.64 us 980.39 us]
commit_to_handler/100-writers-2000-wps time: [523.10 us 541.30 us 560.22 us]
```

The explicit p50/p95/p99 values use the complete 1,000 post-commit-to-receive
samples gathered for each profile. No p99 exceeded the 20 ms finding threshold.
Criterion 0.5.1 does not expose its internal percentile type publicly and does
not report p95/p99. The bench therefore applies Criterion's linear percentile
interpolation formula to the 1,000 raw samples while using Criterion's
`sample_size(1000)` harness for the real path. This is a harness-spec mismatch
to correct before promoting the disposable bench; it does not change the
measured store result.

## Fast gate validation

Commands:

```text
cargo clippy --manifest-path proofs/redb-resource-store-spike/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path proofs/redb-resource-store-spike/Cargo.toml
```

Raw summaries:

```text
Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.23s
```

```text
test result: ok. 3 passed; 0 failed; 0 ignored
test result: ok. 0 passed; 0 failed; 0 ignored
test result: ok. 0 passed; 0 failed; 0 ignored
test result: ok. 0 passed; 0 failed; 4 ignored
test result: ok. 0 passed; 0 failed; 0 ignored
```
