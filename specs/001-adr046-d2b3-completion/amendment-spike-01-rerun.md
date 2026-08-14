# Historical SPIKE-01 RSS rerun

This artifact preserves the measurement history and its technical limits for
memory-footprint reasoning.

The existing `proofs/redb-resource-store-spike/RESULTS.md` remains the
historical failed record. Its median whole-process maximum RSS result is
25,216 KiB against the unchanged 24,576 KiB threshold.

`proofs/redb-resource-store-spike/RESULTS-corrections.md` remains a
non-authoritative corrections prototype. It is not a rerun of record and must
not be cited as evidence that the gate passes.

The artifact
`proofs/redb-resource-store-spike/RESULTS-rerun-2026-08-02.md` records a
median of `18,428 KiB` for the hard fixture, with `6,148 KiB` of headroom below
`24,576 KiB`. This is spike evidence only; it does not establish production
backend acceptance.

## Measurement contract

The hard fixture is unchanged:

```text
rss-fixture --resources 10000 --watches 100
```

The metric is GNU `time -v`'s complete child-process line:

```text
Maximum resident set size (kbytes)
```

The reported value is the whole-process maximum RSS. No empty-process,
runtime, allocator, or other baseline is subtracted. The hard threshold is
exactly `24,576 KiB`.

The established fixture sweep builds `rss-fixture` in release mode and runs
the empty, 10,000-resource/zero-watch, and 10,000-resource/100-watch shapes
three times each, taking the median of the three hard-fixture values. The
rerun used the same fixture, release binary, `TMPDIR` shape, `time -v` field,
and sample method. The hard-fixture median was `18,428 KiB`.

The measurement used the repository-supported arbitrary-command form:

```text
cargo run --quiet --manifest-path packages/Cargo.toml -p xtask -- \
  heavy-gate -- bash -lc '<the established RESULTS.md RSS command>'
```

Only privacy-permitted reproducibility metadata was recorded: Rust and Cargo
versions, kernel/architecture, filesystem type and options, CPU count, load
average, free memory, memory-pressure values, and swap state. Hostnames, user
names, process IDs, credentials, and host-specific paths were excluded.

The repository policy says not to clear `RUSTC_WRAPPER` or
`CARGO_BUILD_RUSTC_WRAPPER`; the historical command in `RESULTS.md` predates
that policy. The rerun retained the fixture and accounting method while
following the current wrapper policy. This command drift is called out in the
result artifact rather than hidden.

Production acceptance still requires the production backend's own conformance,
security, durability, watch-budget, backup/migration, and reaction evidence.
A disposable spike result does not replace those product checks.
