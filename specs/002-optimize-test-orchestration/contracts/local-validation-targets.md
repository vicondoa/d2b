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
- classify any new orchestration tests separately from the preserved baseline.

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

- The default aggregate budget is derived from available logical CPUs.
- An explicit positive local budget may be supplied for diagnosis or
  constrained hosts.
- Invalid budget values return exit status `2`.

## `make test-nix-unit`

### Local behavior

- Discovers the native-system `nix-unit*` flake checks.
- Passes the complete discovered set to one native Nix invocation.
- Uses native keep-going behavior.
- Fails if discovery returns an empty set.
- Preserves pin, duplicate-name, missing-file, and shard-coverage failures.

### CI behavior

`D2B_NIX_UNIT_CHECK=<name>` remains a CI selector for exactly one discovered
check. An unknown or unsafe name returns exit status `2`.

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
