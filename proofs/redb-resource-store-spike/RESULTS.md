# redb resource store spike results

This is the canonical 2026-07-27 run. All commands below start at the
repository root and use the checked-in `Cargo.lock`. The measurements used
Rust 1.95.0, redb 4.1.0, Linux 7.0.10, and ext4. CPU pinning was not applied.

Copy-paste provenance command:

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

The canonical commands set the repository-local temporary directory and
disable compiler wrappers:

```text
mkdir -p proofs/redb-resource-store-spike/.scratch/tmp
export TMPDIR=$PWD/proofs/redb-resource-store-spike/.scratch/tmp
export RUSTC_WRAPPER=
export CARGO_BUILD_RUSTC_WRAPPER=
```

## Final threshold summary

| Threshold | Result | Final measurement |
| --- | --- | --- |
| 10,000 resources, 5 runs, zero oracle divergence | MEASURED-PASS | 5/5 runs, 10,000 resources and 0 divergences per run |
| 100 watches, no misses, duplicates, or gaps | MEASURED-PASS | 21,866 exact ChangeBatch comparisons, 3,686 exact matching entries, 0 misses, 0 duplicates, 0 gaps |
| More than half of non-conflicting storm writes use a batch larger than 1 | MEASURED-PASS | 48/50, 96% |
| All 13 crash boundaries recover atomically or refuse to open | MEASURED-PASS | 13/13 exact raw checkpoints; boundaries 1-11 matched the old checkpoint and 12-13 matched the new checkpoint |
| Median whole-process maximum RSS at or below 24 MiB | MEASURED-FAIL | 25,216 KiB (24.625 MiB), 640 KiB or about 2.6% above 24,576 KiB |
| Commit-to-handler p95 at or below 5,000 us in all profiles | MEASURED-PASS | 115.043 us / 116.195 us / 128.902 us |
| Commit-to-handler p99 reported; document any value above 20 ms | MEASURED-PASS | 134.834 us / 140.928 us / 1,009.871 us; none exceeded 20 ms |

Oracle hardening did not turn a previously passing threshold into a failure.
The whole-process RSS threshold remains the one measured failure. The latency
methodology correction removed the earlier unexplained inverted profile.

## Functional scale and watch oracle

Command:

```text
cargo test --release --manifest-path proofs/redb-resource-store-spike/Cargo.toml --test full_scale -- --ignored --test-threads=1 --nocapture
```

Raw output:

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

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 253.06s
```

The correctness case compares every resource and the complete type, owner,
producer, and controller indexes against an independent `BTreeMap` oracle.
The index-removal unit case adds three transitions for one Endpoint:

1. create it with initial owner, producer, and controller bindings;
2. change all three bindings and assert every old raw index key is absent and
   every new raw key is present;
3. remove owner and producer, change the controller again, and assert the
   replaced keys are absent.

The complete resource/index oracle is checked after every transition and again
after closing and reopening the database. A no-op
`remove_previous_indexes` fails these assertions.

The watch test no longer accepts a filtered batch merely because all entries
that happen to exist have the right type. For each watcher and each revision,
it independently constructs the exact expected `ChangeBatch`, including the
resource bytes, operation id, event, ordinal, and whether entries must be empty
or non-empty. Every filter must observe at least one matching replay revision
(at or before revision 24) and one matching live revision (after revision 24).
A one-second receive timeout turns removed delivery into a failure instead of
a hang. The fast `watch_oracle_rejects_removed_matching_delivery` test removes
the required revision-1 Process entry and proves that the oracle rejects it.

## Crash recovery

SIGKILL command:

```text
for n in $(seq 1 13); do
  cargo run --quiet --manifest-path proofs/redb-resource-store-spike/Cargo.toml \
    --bin crash-fixture -- --kill-at-txn "$n" || exit
done
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

Before the worker is spawned, the parent builds immutable old and new raw
checkpoints. Each checkpoint contains exact key/value bytes for resources,
type, owner, producer, and controller indexes, operations, revision batches,
and store metadata. After reopen, `LastCommittedState` and
`NewCommittedState` are returned only after one complete checkpoint compares
equal. A decoded but wrong resource, operation, ChangeBatch, index, or metadata
value matches neither checkpoint.

The 13 logical boundaries do not put SIGKILL inside redb's `commit()`.
Boundaries 1-11 prove rollback when the process dies before entering commit;
boundaries 12-13 prove exact reopen after `commit()`, including `sync_data`,
has returned. They do not test an interrupted data write, header publication,
or `fdatasync`, and they are not power-loss evidence.

The delegating `StorageBackend` harness adds deterministic errors at the first
data write, after a partial data write, at the offset-zero header publication,
after a partial header write, and in `sync_data`. Write faults return `EIO`;
partial faults first persist half of the requested bytes and then return
`EIO`. Command:

```text
cargo test --manifest-path proofs/redb-resource-store-spike/Cargo.toml \
  disk::tests::commit_faults_recover_only_complete_raw_checkpoints_or_refuse_open \
  -- --nocapture
```

Raw output:

```text
fault=BeforeData recovery=LastCommittedState result=PASS
fault=PartialData recovery=LastCommittedState result=PASS
fault=BeforeHeader recovery=LastCommittedState result=PASS
fault=PartialHeader recovery=LastCommittedState result=PASS
fault=DuringSync recovery=NewCommittedState result=PASS
test disk::tests::commit_faults_recover_only_complete_raw_checkpoints_or_refuse_open ... ok
```

These injected faults validate redb's `StorageBackend` error paths against the
same exact checkpoints. They still run above the live Linux page cache on ext4.
They do not model device-cache loss, controller reordering, torn sectors,
filesystem bugs, or a machine losing power while `fdatasync` is in flight.
The SIGKILL process also leaves the kernel and device caches alive. Therefore
this spike makes no power-loss-safety claim. Its result assumes the Linux
`pwrite`/`fdatasync` contract exposed by redb's `FileBackend`, a correctly
functioning ext4 filesystem, and storage that honors completed flushes.

## RSS

The release binary is built explicitly so a fresh checkout does not depend on
an already-existing `target/release/rss-fixture`:

```text
cargo build --release --locked --manifest-path proofs/redb-resource-store-spike/Cargo.toml --bin rss-fixture
for args in \
  '--resources 0 --watches 0' \
  '--resources 10000 --watches 0' \
  '--resources 10000 --watches 100'
do
  echo "$args"
  for run in 1 2 3; do
    echo "run=$run"
    nix shell --impure --expr '(import <nixpkgs> {}).time' \
      --command bash -c \
      "TMPDIR=\$PWD/proofs/redb-resource-store-spike/.scratch/tmp time -v proofs/redb-resource-store-spike/target/release/rss-fixture $args"
  done
done
```

Raw fixture and maximum-RSS lines:

```text
--resources 0 --watches 0
run=1 resources=0 watches=0 revision=0 file_bytes=1056768 result=READY
Maximum resident set size (kbytes): 4024
run=2 resources=0 watches=0 revision=0 file_bytes=1056768 result=READY
Maximum resident set size (kbytes): 3880
run=3 resources=0 watches=0 revision=0 file_bytes=1056768 result=READY
Maximum resident set size (kbytes): 3924

--resources 10000 --watches 0
run=1 resources=10000 watches=0 revision=10000 file_bytes=52314112 result=READY
Maximum resident set size (kbytes): 18632
run=2 resources=10000 watches=0 revision=10000 file_bytes=52314112 result=READY
Maximum resident set size (kbytes): 18480
run=3 resources=10000 watches=0 revision=10000 file_bytes=52314112 result=READY
Maximum resident set size (kbytes): 18468

--resources 10000 --watches 100
run=1 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
Maximum resident set size (kbytes): 25180
run=2 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
Maximum resident set size (kbytes): 25312
run=3 resources=10000 watches=100 revision=10000 file_bytes=52314112 result=READY
Maximum resident set size (kbytes): 25216
```

The medians are 3,924 KiB, 18,480 KiB, and 25,216 KiB. The threshold is
whole-process RSS, so subtracting the empty-process baseline is not a valid
pass calculation. The final 25,216 KiB result is 640 KiB (about 2.6%) above
the 24,576 KiB gate and is MEASURED-FAIL.

The measured median increments are 14,556 KiB for 10,000 resources, about
1.46 KiB per resource, and 6,736 KiB for 100 watches, about 67.36 KiB per
watch. Registration scans and decodes the complete revision log even when
registering at the current revision, and each watch has its own 1,024-entry
channel. The result supports range-seek replay, streaming decode, shared
immutable ChangeBatch fan-out, and a global bounded watch-admission budget.
Those changes remain prerequisites before the production backend can satisfy
the unchanged RSS gate.

## Commit-to-handler latency

Command:

```text
cargo bench --manifest-path proofs/redb-resource-store-spike/Cargo.toml \
  --bench commit_to_handler -- --noplot
```

Canonical percentile and load-accounting lines:

```text
commit_to_handler profile=none samples=1000 p50_us=21.664 p95_us=115.043 p99_us=134.834 measurement_s=4.704 background_commits=0 active_writers=0 min_writer_commits=0 achieved_wps=0.0 configured_wps=0
commit_to_handler profile=10-writers-500-wps samples=1000 p50_us=20.979 p95_us=116.195 p99_us=140.928 measurement_s=7.042 background_commits=3484 active_writers=10 min_writer_commits=347 achieved_wps=494.7 configured_wps=500
commit_to_handler profile=100-writers-2000-wps samples=1000 p50_us=36.623 p95_us=128.902 p99_us=1009.871 measurement_s=19.573 background_commits=35079 active_writers=100 min_writer_commits=350 achieved_wps=1792.2 configured_wps=2000
```

The background writers create their resources, meet at a start barrier, and
then begin staggered periodic writes. Per-writer committed-write counters are
sampled only across the 1,000 latency samples. Every writer must commit at
least once, and the achieved aggregate rate must be within 20% of the
configured rate. The observed 494.7 and 1,792.2 writes/s are respectively
1.1% and 10.4% below their targets, so both contention profiles meet that
declared tolerance. The minimum per-writer counts prove all 10 and all 100
writers were active.

The earlier unloaded-versus-loaded p95 inversion was a measurement artifact.
The sampling future waited for its write receipt before it started polling the
hint receiver. Under high load, other Tokio tasks kept runtime workers awake
and the already-queued hint was consumed immediately; under no load, parking
and waking the same runtime made that delay much larger. The profiles therefore
measured different consumer scheduling states.

The corrected harness has a dedicated OS handler thread continuously blocked
on the hint receiver. That thread timestamps the hint before handing the
sample back asynchronously, so caller rescheduling is outside the latency
value. Every profile uses the same 1,000-sample distribution. The corrected
p95 increases monotonically from 115.043 to 116.195 to 128.902 us. This is the
canonical latency evidence; the superseded values are not used by any table.

Criterion's whole-operation estimates are separate from the explicit
post-commit percentiles:

```text
commit_to_handler/none time: [4.4732 ms 4.5958 ms 4.7201 ms]
commit_to_handler/10-writers-500-wps time: [5.2332 ms 5.6469 ms 6.1253 ms]
commit_to_handler/100-writers-2000-wps time: [14.472 ms 15.308 ms 16.166 ms]
```

They include the write and durable commit as well as dispatch and increase
with load, as expected. No post-commit p99 exceeded the 20 ms reporting
threshold.

## Fast validation

Commands:

```text
cd proofs/redb-resource-store-spike
cargo clippy --all-targets -- -D warnings
cargo test
```

The full-scale fixtures remain ignored by default. The explicit command above
is the reproducible path for the scale evidence.
