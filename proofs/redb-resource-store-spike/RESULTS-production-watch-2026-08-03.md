# Production redb watch path RSS result

This artifact records the production hard fixture after the measured child was
extended through redb, Resource API framing and pumping, the bounded d2b-bus
named stream, and the controller-toolkit queue consumer. The historical
backend-only result remains unchanged.

## Provenance

| Item | Value |
| --- | --- |
| Source SHA | `728d2124e278f8e2fcd0acd5145de51717069d8e` |
| Measurement date | 2026-08-03 |
| Toolchain | `rustc 1.97.0` / `cargo 1.97.0` |
| Host shape | Linux 7.0.10 x86_64, ext4, 12 CPUs |
| Fixture | 10,000 valid production resource rows and 100 production watches |
| Bus path | authenticated fixed Zone route, Resource API watch frames, bounded named streams, controller queue fan-in |
| Cache bound | 4,194,304 bytes |
| Metric | GNU `time -v` whole-process `Maximum resident set size (kbytes)` |
| Baseline subtraction | None |

## Command

The hard fixture ran through the public heavy-gate semaphore:

```text
cargo run --quiet --manifest-path packages/Cargo.toml -p xtask -- heavy-gate -- cargo test --release --manifest-path packages/Cargo.toml -p d2b-resource-store-redb --test production_watch_rss production_backend_hard_fixture_rss -- --ignored --nocapture --test-threads=1
```

The parent created a fresh provisioned redb image for each run. GNU `time`
wrapped the child integration-test process that opened the image through
`RedbResourceStore::open_owned`, opened Resource API watches, pumped them over
the bounded named stream, and delivered each frame to the controller queue.
The parent was not the measured process. Each child measurement covered its
complete lifetime without subtracting a baseline.

## Raw RSS result

| Run | Raw maximum RSS |
| ---: | ---: |
| 1 | 20,228 KiB |
| 2 | 20,248 KiB |
| 3 | 20,096 KiB |
| **Median** | **20,228 KiB** |

| Threshold | Result | Headroom |
| --- | --- | ---: |
| Whole-process maximum RSS <= 24,576 KiB | **MEASURED-PASS** | 4,348 KiB |

## Production signal checks

The measured child asserted the existing backend and watch signals:

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
| Watch registrations at completion | 0 |
| Watch budget used at completion | 0 |
| Watch budget capacity | 1,024 |
| Admission backpressure rejections | 1 |
| Slow-watcher evictions | 1 |
| Replay work | 1 |

The 100 production API watches delivered frames through the authenticated
named-stream route. The controller queue consumed the 100 deliveries with
same-resource fan-in coalescing, and transport grants released the watch
budget. The zero-credit admission and slow-watcher signal assertions also
passed.
