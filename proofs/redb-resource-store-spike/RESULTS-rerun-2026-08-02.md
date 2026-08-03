# redb resource store SPIKE-01 RSS rerun result of record

This artifact is the authoritative result of the gated whole-process RSS
rerun performed on 2026-08-02. It supersedes only the RSS conclusion in
`RESULTS.md`; it does not overwrite that historical record, and it does not
make the production backend accepted.

`RESULTS-corrections.md` remains a non-authoritative prototype. Its figures
were not used as the rerun result.

## Authority and source

| Item | Value |
| --- | --- |
| Measurement source tree | `2e9ef32` |
| Proof source changes during rerun | None |
| Public gate | `cargo run --quiet --manifest-path packages/Cargo.toml -p xtask -- heavy-gate -- ...` |
| Gate result | Slot acquired, command completed successfully |
| Fixture used for the hard verdict | `rss-fixture --resources 10000 --watches 100` |
| Metric | GNU `time -v` whole-process maximum resident set size |
| Hard threshold | `24,576 KiB` |
| Baseline subtraction | None |
| Sample method | Three independent runs; median of the three hard-fixture values |
| Relationship to `RESULTS.md` | Supersedes its RSS conclusion; preserves its history |
| Relationship to `RESULTS-corrections.md` | Independent gated rerun; prototype remains non-authoritative |
| Gate 0 | Not closed by this artifact |

## Reproducible command

The command below was run from the repository root through the public heavy
gate. The fixture shapes and `time -v` accounting are unchanged from
`RESULTS.md`. The historical result command explicitly cleared compiler
wrapper variables; the current repository policy forbids that clearing, so
those exports are intentionally absent here. This does not change the
fixture, binary profile, RSS field, threshold, or baseline method.

```text
cargo run --quiet --manifest-path packages/Cargo.toml -p xtask -- heavy-gate -- bash -lc 'mkdir -p proofs/redb-resource-store-spike/.scratch/tmp; export TMPDIR=$PWD/proofs/redb-resource-store-spike/.scratch/tmp; printf "rustc="; rustc --version; printf "cargo="; cargo --version; printf "kernel="; uname -srmo; printf "filesystem="; findmnt -n -o FSTYPE,OPTIONS -T proofs/redb-resource-store-spike; printf "cpus="; nproc; cargo build --release --locked --manifest-path proofs/redb-resource-store-spike/Cargo.toml --bin rss-fixture; printf "pre_measure_loadavg="; cat /proc/loadavg; printf "pre_measure_pressure="; cat /proc/pressure/memory; printf "pre_measure_swap="; awk "/^(pswpin|pswpout) / {print}" /proc/vmstat; for args in "--resources 0 --watches 0" "--resources 10000 --watches 0" "--resources 10000 --watches 100"; do echo "$args"; for run in 1 2 3; do echo "run=$run"; nix shell --impure --expr "(import <nixpkgs> {}).time" --command bash -c "TMPDIR=\$PWD/proofs/redb-resource-store-spike/.scratch/tmp time -v proofs/redb-resource-store-spike/target/release/rss-fixture $args"; done; done'
```

The command acquired heavy-gate slot 0 of 2. Before the measured child runs,
no `rustc`, `cargo`, `nix build`, or `nix eval` process belonging to a
worktree was observed. The host was quiet with respect to worktree builds and
evaluation; memory pressure and swap-in/out were quiescent.

## Environment

Only reproducibility metadata permitted by repository policy is recorded.

```text
rustc=rustc 1.97.0 (2d8144b78 2026-07-07)
cargo=cargo 1.97.0 (c980f4866 2026-06-30)
kernel=Linux 7.0.10 x86_64 GNU/Linux
filesystem=ext4 rw,relatime
cpus=12
```

The pre-measurement load averages were `5.26`, `5.08`, and `6.31` for
one, five, and fifteen minutes. `free -m` before gate acquisition reported
64,014 MiB total memory, 21,367 MiB used, 14,906 MiB free, 42,646 MiB
available, and 70,416 MiB total swap with 16,748 MiB used. Immediately before
the RSS sweep, `/proc/pressure/memory` reported `some avg10=0.00 avg60=0.00
avg300=0.00` and `full avg10=0.00 avg60=0.00 avg300=0.00`; `/proc/vmstat`
reported `pswpin 0` and `pswpout 0`.

## Raw maximum RSS summary

These are the raw `Maximum resident set size (kbytes)` values emitted by
GNU `time -v`, in run order.

| Fixture | Run 1 | Run 2 | Run 3 | Median | Range |
| --- | ---: | ---: | ---: | ---: | ---: |
| 0 resources, 0 watches | 3,844 KiB | 3,908 KiB | 3,832 KiB | 3,844 KiB | 3,832-3,908 KiB |
| 10,000 resources, 0 watches | 18,356 KiB | 18,520 KiB | 18,552 KiB | 18,520 KiB | 18,356-18,552 KiB |
| 10,000 resources, 100 watches | 18,428 KiB | 18,396 KiB | 18,552 KiB | 18,428 KiB | 18,396-18,552 KiB |

The hard-fixture raw lines were:

```text
Maximum resident set size (kbytes): 18428
Maximum resident set size (kbytes): 18396
Maximum resident set size (kbytes): 18552
```

## Threshold verdict

| Threshold | Result | Evidence |
| --- | --- | --- |
| Whole-process maximum RSS for 10,000 resources and 100 watches at or below 24,576 KiB | MEASURED-PASS | Median `18,428 KiB`, `6,148 KiB` below the threshold; every individual run was below the threshold |

No baseline was subtracted. The empty-process and zero-watch rows are context
only and do not contribute to the pass calculation.

This result resolves the disposable proof's RSS failure under the corrected
prototype, but it does not close the production backend, watch dispatcher, or
reaction benchmark gates. Those still require their production evidence and
the authority workflow described by the SPIKE-01 amendment draft.
