# Local Validation Target Contract

## Stable commands

The contributor-facing interface remains:

```bash
make test-rust
make test-nix-unit
make test-flake
```

All three commands:

- run from the repository root;
- return `0` only when every required surface passes;
- return nonzero when any required surface fails;
- preserve grouped, attributable diagnostics;
- report all independent failures observed in the invocation;
- print total elapsed time;
- preserve every item in the current enforcing coverage inventory;
- classify any new orchestration tests separately from the preserved baseline;
- when `D2B_EXECUTION_MANIFEST=<path>` is set, write a deterministic manifest
  of the execution leaves that completed their required commands.

## `make test-rust`

### Local behavior

- GNU Make schedules the Rust execution leaves with a bounded aggregate CPU
  budget.
- Output is synchronized by leaf.
- Independent leaves continue after another independent leaf fails.
- Operations sharing a Cargo target directory remain ordered.
- Broker feature passes remain serial.
- Doctests and discovered `harness = false` binaries remain required.

### CI behavior

CI may continue invoking:

```bash
make test-rust-api-surface
make test-rust-main
make test-rust-remaining
```

The stable `test-rust` CI context remains the rollup of those enforcing jobs.

### Configuration

- The public local override is `D2B_RUST_BUDGET=<positive-integer>`.
- The default aggregate budget is the smaller of available logical CPUs and a
  memory-derived cap using Linux `MemAvailable`, a 2 GiB host reserve, and a
  conservative 3 GiB allowance per heavy Rust job.
- GNU Make's jobserver schedules eligible workspace lanes. Each heavy lane
  receives an explicit Cargo `--jobs` and nextest thread quota, and every
  runnable DAG frontier MUST have a summed quota no greater than
  `D2B_RUST_BUDGET`.
- Invalid budget values return exit status `2`.

## `make test-nix-unit`

### Local behavior

- Discovers the native-system `nix-unit*` flake checks.
- Uses the fastest measured established runner or evaluator that satisfies
  this contract; the implementation is selected by the experiment matrix in
  `plan.md`, not prescribed here.
- Reports every failing case or shard observed in the invocation, including
  evaluation errors, without requiring a rerun to reveal the next failure.
- Fails if discovery returns an empty set.
- Preserves pin, duplicate-name, missing-file, and shard-coverage failures.

### CI behavior

`D2B_NIX_UNIT_CHECK=<name>` remains a CI selector for exactly one discovered
check. An unknown or unsafe name returns exit status `2`.

If the selected design retires `D2B_NIX_UNIT_JOBS`, setting it returns exit
status `2` with an actionable migration message. If the selected design still
has evaluator workers, the variable may remain as a compatibility alias for
the documented replacement during this change.

## `make test-flake`

### Local behavior

- Runs one native-system `nix flake check --no-build --keep-going`.
- Uses a `git+file://` source reference.
- Realizes only checks in the committed realized-check class.
- Does not build unrelated package or flake outputs.

### CI behavior

The existing selectors remain:

- `D2B_FLAKE_CHECK=<name>` for one check shard;
- `D2B_FLAKE_OUTPUTS=1` for the non-check output sweep.

CI may retain separate x86_64 and aarch64 jobs and its existing dynamic matrix.

Setting retired local-only `D2B_FLAKE_JOBS` or `D2B_FLAKE_LOCAL_SHARDS`
returns exit status `2` with a migration message. CI selectors remain valid.

### Performance comparison

- The hard 50% target compares the legacy local Layer-1 shard path with the
  optimized local Layer-1 path.
- The direct `make test-flake` path is measured separately and may regress by
  no more than 20%.

## Coverage compatibility

Optimization is invalid if any of the following disappears:

- a nextest test identifier;
- a Rust doctest;
- a discovered `harness = false` binary;
- a broker feature pass;
- a Nix unit case or integrity pin;
- a native flake check;
- the realized video command-surface check;
- a current supply-chain, schema, stub, or inventory assertion.
