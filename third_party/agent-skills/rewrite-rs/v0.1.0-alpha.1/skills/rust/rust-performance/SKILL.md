---
name: rust-performance
description: Profile before optimizing, cut allocation out of hot paths, and treat LTO, codegen-units, PGO and target-cpu as the last five percent. Use when Rust code is too slow, when a benchmark regresses, when reviewing code that allocates or formats inside a loop, when release-profile or codegen flags come up, or when the user asks how to make Rust faster.
---

# Rust Performance

No change lands here that no measurement named: the profile is the entry ticket,
and the codegen flags are the last five percent, not the first move. Async
throughput is `/async-rust`; `unsafe` for speed is `/unsafe-rust`.

## Measure, then change

No optimisation lands without a measurement that named it. Skipping it costs
twice: the change that made nothing faster, and the change that made the code
worse in exchange for nothing. A profile finds the one function that holds the
time — almost never the one that looked expensive while reading.

## Benchmarks that mean something

`criterion` for a statistical comparison against a stored baseline; `divan` when
a lighter harness is enough. Benchmark the operation, not the setup around it.

Without it, the profiler shows addresses instead of function names:

```toml
[profile.bench]
debug = 1
```

A benchmark run on a laptop under thermal throttling produces a number, just not
a comparable one — the machine has to be quiet between the runs being compared.

## Allocation is usually the answer

The highest-yield category, and the one a profile names most often:

- `Vec::with_capacity` when the size is known — the default grows and copies.
- Clear-and-reuse over allocate-per-iteration: buffer outside, `clear()` inside.
- `Box<[T]>` and `Box<str>` for an owned sequence that will never grow.
- `mem::take` to move a value out of a `&mut` without cloning.
- No `format!` in a hot loop — every call allocates a fresh `String`.
- `Cow` for the borrow-or-own case — copy only when the value changed.

One complete example, joining with a known total size:

```rust
fn joined(parts: &[String]) -> String {
    let total = parts.iter().map(|p| p.len() + 1).sum();
    let mut out = String::with_capacity(total);
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}
```

The reuse patterns and the buffer-passing shapes are in `ALLOCATION.md`.

## Pick the collection for the access pattern

`Vec` beats `HashMap` for small `n` because of cache behaviour, not
asymptotics. `BTreeMap` when iteration order matters. A sorted `Vec` plus
`binary_search` when built once and read many times. The crossover is a thing
to measure, not a constant to memorise.

## Hashing

The default hasher is DoS-resistant and correspondingly slow. Swapping it is
safe and worthwhile only when the keys are not attacker-controlled — internal
IDs, enum discriminants, interned strings. With attacker-controlled keys it is
a denial-of-service surface, and no lookup speed buys that back. Say the risk
plainly rather than presenting the swap as a free win.

## Iterators and bounds checks

An iterator chain gives the optimiser the length invariant an index loop hides,
which is how bounds checks disappear: two `[i]` reads in one loop become
`iter().zip()`, and the optimiser can prove the accesses in range. This is the
one place where the idiomatic form is also the fast form — no tradeoff to
argue about.

## The last five percent

LTO, `codegen-units = 1`, PGO, `target-cpu`. Per ADR 0007: application
decisions, never library defaults, each with the portability cost stated
alongside the gain — a knob is one line and a profile is an afternoon:

- `target-cpu=native` in committed config produces a binary that faults on an
  older machine.
- PGO needs a representative workload, or it optimises for the wrong path.
- LTO and `codegen-units = 1` buy a percentage, not a factor, at the cost of
  build time.

`panic = "abort"` is not a performance default: it is an application-level
choice, a library cannot assume unwinding, and it changes behaviour under
`#[should_panic]`.

## Deferrals

Async throughput, executor stalls, and `spawn_blocking` are `/async-rust` — the
runtime question, not the throughput one. Reaching for `unsafe` to skip a check
is `/unsafe-rust`, and the answer there is that the block needs a soundness
argument, not a benchmark. Whether the clone the profile found should exist at
all is `/ownership-not-clone`.

## Verification

```bash
cargo bench                  # against the recorded baseline where one exists
cargo clippy --all-targets   # at the level the repo configures; add -- -D warnings only if it configures none
cargo test --release
```

A benchmark with no stored baseline proves nothing about a change — record the
before, then measure the after, and compare against the noise the benchmark
reports. Clippy keeps the lint level the target repo configured; `-D warnings`
is the fallback for a repo that configures no lints at all.
