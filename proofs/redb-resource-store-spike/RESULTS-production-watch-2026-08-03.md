# Production redb watch path RSS result

This artifact records the production hard fixture after the measured child was
extended through redb, Resource API framing and pumping, the bounded d2b-bus
named stream, and the controller-toolkit queue consumer. The historical
backend-only result remains unchanged.

## Provenance

| Item | Value |
| --- | --- |
| Source SHA | `1413cbbd3b6f7215864cd27b55867235b871bdfb` |
| Measurement date | 2026-08-03 |
| Toolchain | `rustc 1.97.0` / `cargo 1.97.0` |
| Host shape | Linux 7.0.10 x86_64, ext4, 12 CPUs |
| Fixture | 10,000 valid production resource rows and 100 production watches |
| Bus path | authenticated fixed Zone route, Resource API watch frames, bounded named streams, controller queue fan-in |
| Cache bound | 4,194,304 bytes |
| Metric | GNU `time -v` whole-process `Maximum resident set size (kbytes)` |
| Baseline subtraction | None |
| Harness owner | `packages/d2b-bus/tests/production_watch_rss.rs` |

### Harness ownership

`ef4f5455` moved this fixture from the redb crate's integration-test target to
the bus crate's, so the redb crate no longer depends on `d2b-resource-api` and
the sealed mutation policy holds again. The readings below come from the
bus-owned target after that move and cover the complete redb, Resource API,
named-stream, and controller-queue path.

The sibling backend-only artifact,
[`RESULTS-production-2026-08-03.md`](./RESULTS-production-2026-08-03.md), is
**unaffected**. It measures the `--lib` target
`d2b-resource-store-redb/src/tests.rs`, which `ef4f5455` did not move. The two
artifacts run same-named tests in different targets, so a global rename of the
owner would have been wrong.

## Command

The hard fixture ran through the public heavy-gate semaphore at the Source SHA
above:

```text
cargo run --quiet --manifest-path packages/Cargo.toml -p xtask -- heavy-gate -- cargo test --release --manifest-path packages/Cargo.toml -p d2b-bus --features production-rss-fixture --test production_watch_rss production_backend_hard_fixture_rss -- --ignored --nocapture --test-threads=1
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
| 1 | 20,188 KiB |
| 2 | 20,308 KiB |
| 3 | 20,456 KiB |
| **Median** | **20,308 KiB** |

| Threshold | Result | Headroom |
| --- | --- | ---: |
| Whole-process maximum RSS <= 24,576 KiB | **MEASURED-PASS** | 4,268 KiB |

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
