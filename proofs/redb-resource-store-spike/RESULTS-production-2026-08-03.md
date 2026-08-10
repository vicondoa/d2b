# Production redb backend and watch-dispatcher result

This artifact records a measurement of the production
`d2b-resource-store-redb` crate. The measured process opens the real
owned-descriptor redb backend, runs startup consistency validation, starts the
production writer and read pool, and drives the production watch coordinator.
No result from another crate is used as this evidence.

## Provenance

| Item | Value |
| --- | --- |
| Source SHA | `3f36ecb168df8922ae2047ad1045791c00fca2a6` |
| Measurement date | 2026-08-03 |
| Toolchain | `rustc 1.97.0` / `cargo 1.97.0` |
| Host shape | Linux 7.0.10 x86_64, ext4, 12 CPUs |
| Fixture | 10,000 valid production resource rows and 100 production watches |
| Cache bound | 4,194,304 bytes |
| Metric | GNU `time -v` whole-process `Maximum resident set size (kbytes)` |
| Baseline subtraction | None |

## Command

The hard fixture ran through the public heavy-gate semaphore:

```text
cargo run --quiet --manifest-path packages/Cargo.toml -p xtask -- heavy-gate -- cargo test --release --locked --manifest-path packages/Cargo.toml -p d2b-resource-store-redb --lib production_backend_hard_fixture_rss -- --ignored --nocapture --test-threads=1
```

The test parent created a fresh valid redb image for each run. GNU `time`
wrapped a child test process that opened that image through
`RedbResourceStore::open_owned`; the child then performed the production
backend and watch-dispatcher workload. The parent was not the measured
process, and the child measurement covered its complete lifetime without
subtracting a baseline.

## Raw RSS result

| Run | Raw maximum RSS |
| ---: | ---: |
| 1 | 18,584 KiB |
| 2 | 18,444 KiB |
| 3 | 18,776 KiB |
| **Median** | **18,584 KiB** |

| Threshold | Result | Headroom |
| --- | --- | ---: |
| Whole-process maximum RSS <= 24,576 KiB | **MEASURED-PASS** | 5,992 KiB |

Every raw run and the median are below the unchanged threshold.

## Production signal checks

The child asserted these signals while the fixture was live:

| Signal | Observed result |
| --- | ---: |
| Revision range seeks | 1 |
| Replay rows scanned | 1 |
| Replay rows decoded | 1 |
| Shared immutable batches | 3 |
| Fan-out references | 102 |
| Writer queue depth after work | 0 |
| Writer queue capacity | 256 |
| Read-pool worker threads | 4 |
| Maximum concurrent reads | 16 |
| Read permits restored after list | 16 |
| Peak registered watches | 100 |
| Peak queued watch budget after fan-out | 100 |
| Watch budget capacity | 1,024 |
| Admission backpressure rejections | 1 |
| Slow-watcher evictions | 1 |
| Replay work | 1 |

The 100 matching watchers received the same immutable batch, acknowledged it,
and released all queued budget. The slow watcher was evicted at its bounded
credit limit and its acknowledged cursor was retained for resume. Final watch
registration and budget gauges were both zero.

## Remaining production-watch obligation

This result closes the production RSS and backend-dispatch signal evidence
only. End-to-end named-stream integration through the future resource API and
bus watch surface remains outstanding, as do the broader disconnect, fan-in,
and compaction acceptance matrices outside this hard fixture. Those are not
claimed by this artifact.

## Current-tip rerun (2026-08-06)

The read-only validation lane reran the documented backend production command
at repository HEAD `da9295e7ff370b22cdd6c413e8d82b33936f285e`. This is a
current-tip rerun added to the dated 2026-08-03 artifact; it does not replace
the historical source SHA, measurements, or signal table above. The command
used the public heavy-gate slot:

```text
cargo run --quiet --manifest-path packages/Cargo.toml -p xtask -- heavy-gate -- cargo test --release --locked --manifest-path packages/Cargo.toml -p d2b-resource-store-redb --lib production_backend_hard_fixture_rss -- --ignored --nocapture --test-threads=1
```

| Item | Value |
| --- | --- |
| Source SHA | `da9295e7ff370b22cdd6c413e8d82b33936f285e` |
| Validation result | **PASS** |
| RSS runs | 18,784 / 18,788 / 19,160 KiB |
| Median RSS | 18,788 KiB |
| Threshold | 24,576 KiB |
| Median headroom | 5,788 KiB |

This rerun is the backend-only `--lib` fixture represented by this artifact;
it is distinct from the production watch-path rerun recorded in the sibling
artifact. The remaining-obligation paragraph above describes the dated
2026-08-03 run and does not claim coverage from that sibling rerun.
