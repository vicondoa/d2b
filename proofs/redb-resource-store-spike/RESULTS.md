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
| More than half of non-conflicting storm writes use a batch larger than 1 | MEASURED-PASS | 49/50, 98% |
| All 13 crash boundaries recover atomically or refuse to open | MEASURED-PASS | 13/13; boundaries 1-11 reopened at revision 1, boundaries 12-13 reopened at revision 2 |
| Median maximum RSS at or below 24 MiB | NOT-MEASURED | `/usr/bin/time` was absent; the fixture itself completed with 10,000 resources and 100 watches |
| Commit-to-handler p95 at or below 5,000 us in all profiles | MEASURED-PASS | 114.536 us / 14.767 us / 114.733 us |
| Commit-to-handler p99 reported; document any value above 20 ms | MEASURED-PASS | 134.615 us / 30.841 us / 141.162 us; none exceeded 20 ms |

No measured result requires an admission-fairness or post-commit dispatcher
change. RSS remains unresolved and must be measured on a host providing GNU
`/usr/bin/time` before the physical schema plan can be treated as satisfying
the 24 MiB store budget.

## Functional scale metrics

Command:

```text
cargo test --release --manifest-path proofs/redb-resource-store-spike/Cargo.toml --test full_scale -- --ignored --test-threads=1 --nocapture
```

Raw output:

```text
Compiling redb-resource-store-spike v0.1.0 (/home/paydro/projects/d2b-w1-spike/proofs/redb-resource-store-spike)
Finished `release` profile [optimized] target(s) in 4.55s
Running tests/full_scale.rs (proofs/redb-resource-store-spike/target/release/deps/full_scale-b2360e35c86d37e9)

running 4 tests
test conflict_storm_groups_at_least_half_of_non_conflicting_writes ... writers=500 targets=50 successful_non_conflicting=50 conflicts=450 grouped_batch_gt_1=49 grouped_percent=98
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

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 288.54s
```

The correctness case compares all resources plus type, owner, producer, and
controller indexes and the contiguous revision log against a `BTreeMap` oracle
after each group of at most 16 independently validated mutations. The watch
case registers 100 mixed-filter watchers while writes proceed; actor
serialization closes the replay/live gap and empty filtered ChangeBatches
preserve Zone revision continuity. The conflict test starts all 500 writers
behind one barrier, gets exactly one success for each of 50 resources, and
computes the batch percentage only over those 50 non-conflicting writes.

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

Required command, attempted three times:

```text
/usr/bin/time -v cargo run --manifest-path proofs/redb-resource-store-spike/Cargo.toml --bin rss-fixture -- --resources 10000 --watches 100
```

Raw output for each attempt:

```text
/bin/bash: line 1: /usr/bin/time: No such file or directory
```

The required maximum-RSS metric is therefore NOT-MEASURED. It was not replaced
with an estimate or a different measurement source. The fixture was run
without GNU time to establish that it reaches the requested steady state:

```text
resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
```

## Commit-to-handler latency

Command:

```text
cargo bench --manifest-path proofs/redb-resource-store-spike/Cargo.toml -- commit_to_handler
```

Raw measured percentile lines:

```text
commit_to_handler profile=none samples=1000 p50_us=32.237 p95_us=114.536 p99_us=134.615
commit_to_handler profile=10-writers-500-wps samples=1000 p50_us=5.444 p95_us=14.767 p99_us=30.841
commit_to_handler profile=100-writers-2000-wps samples=1000 p50_us=82.758 p95_us=114.733 p99_us=141.162
```

Criterion also exercised the real write, durable redb commit, actor dispatch,
consumer wake, and hint receive path during its 1,000-sample benchmark
configuration. Its wall-time estimates include the full write rather than only
the requested post-commit interval:

```text
commit_to_handler/none time: [7.6741 ms 8.6486 ms 9.6937 ms]
commit_to_handler/10-writers-500-wps time: [2.7519 ms 2.8462 ms 2.9426 ms]
commit_to_handler/100-writers-2000-wps time: [5.7079 ms 6.2047 ms 6.7333 ms]
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
Finished `dev` profile [unoptimized + debuginfo] target(s) in 6.06s
```

```text
test result: ok. 3 passed; 0 failed; 0 ignored
test result: ok. 0 passed; 0 failed; 0 ignored
test result: ok. 0 passed; 0 failed; 0 ignored
test result: ok. 0 passed; 0 failed; 4 ignored
test result: ok. 0 passed; 0 failed; 0 ignored
```
