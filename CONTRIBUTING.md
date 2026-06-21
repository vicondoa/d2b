# Contributing to nixling

For repo-specific operational policy, see [AGENTS.md](./AGENTS.md).

## Filing issues

- Use [GitHub Issues](https://github.com/vicondoa/nixling/issues) for bugs, docs fixes, and feature requests.
- Include a minimal reproduction, expected vs actual behavior, and any relevant logs.
- Include the nixling version: `nixling --version` on an installed host, or the repo tag / commit you tested.
- **Do not** file security vulnerabilities publicly; follow [SECURITY.md](./SECURITY.md).

## Setting up a dev environment

1. Clone the repo and enter it:
   ```bash
   git clone https://github.com/vicondoa/nixling.git
   cd nixling
   ```
2. Install Nix with flakes enabled (`experimental-features = nix-command flakes`).
3. No separate `nix develop` shell is needed.
4. Validate the framework with:
   ```bash
   nix flake check --no-build --all-systems
   ```

## Running quality gates

- `bash tests/static.sh` is the top-level Layer-1 gate.
- It runs parse checks, smoke evals, assertion tests, manifest schema validation, and per-example flake checks.
- See [tests/README.md](./tests/README.md) for the full test layering and optional Layer-2 integration tests.

<a id="rust-workspace-checks"></a>

### Rust workspace checks

The `packages/` Cargo workspace is gated by `tests/static.sh` and by
`nix flake check --no-build --all-systems`. To run the gate locally:

```bash
cargo --manifest-path packages/Cargo.toml fmt --check
cargo --manifest-path packages/Cargo.toml clippy --workspace --all-targets -- -D warnings
cargo --manifest-path packages/Cargo.toml test --workspace
cargo --manifest-path packages/Cargo.toml deny check
cargo --manifest-path packages/Cargo.toml audit
nix build .#checks.x86_64-linux.rust-build \
          .#checks.x86_64-linux.rust-tests \
          .#checks.x86_64-linux.rust-clippy \
          .#checks.x86_64-linux.rust-deny \
          .#checks.x86_64-linux.rust-audit
for c in rust-build rust-tests rust-clippy rust-deny rust-audit; do
  nix eval --raw ".#checks.aarch64-linux.${c}.drvPath" >/dev/null || exit 1
done
```

The pinned toolchain in `packages/rust-toolchain.toml` is honored only
when cargo is invoked with `--manifest-path packages/Cargo.toml` or from
inside `packages/`. See
[ADR 0009](docs/adr/0009-rust-toolchain-msrv-and-supply-chain.md) for
toolchain, MSRV, and supply-chain policy.

All nixling worktrees on paydro's host share Cargo build artifacts via
repo-local `.cargo/config.toml` files:

- `packages/.cargo/config.toml` → `/home/paydro/.cache/nixling-cargo-target/workspace`
- `packages/nixling-priv-broker/.cargo/config.toml` → `/home/paydro/.cache/nixling-cargo-target/broker`
- `packages/nixling-guest-shell-runner/.cargo/config.toml` → the helper workspace target dir
- `packages/nixling-core/fuzz/.cargo/config.toml` → `/home/paydro/.cache/nixling-cargo-target/fuzz`

Cargo's internal locking makes concurrent worktree builds safe, but a
very old checkout may pay one slower rebuild while incremental state is
refreshed in the shared cache.

The persistent-shell feasibility helper is a standalone excluded workspace. Run
it explicitly when iterating on that crate:

```bash
cargo --manifest-path packages/nixling-guest-shell-runner/Cargo.toml fmt --check
cargo --manifest-path packages/nixling-guest-shell-runner/Cargo.toml clippy --workspace --all-targets --features real-libshpool -- -D warnings
cargo --manifest-path packages/nixling-guest-shell-runner/Cargo.toml test --workspace --features real-libshpool
cargo deny --manifest-path packages/nixling-guest-shell-runner/Cargo.toml check --config packages/nixling-guest-shell-runner/deny.toml
cargo audit --file packages/nixling-guest-shell-runner/Cargo.lock --ignore RUSTSEC-2024-0384
```

`bash tests/static.sh` also has a fast path for Rust-heavy gates:

- it resolves one shared Rust toolchain shell at the top of the run and
  reuses that PATH in child scripts instead of spawning a fresh `nix shell`
  per gate;
- independent Rust, schema, and example gates run behind a small semaphore
  controlled by `NL_STATIC_JOBS` (default `4`);
- `bash tests/tools/static-timing.sh` writes a per-gate wall-clock report to
  `$ROOT/.static-timing.log`;
- to profile one gate in isolation, run `time bash tests/<gate>.sh`.

#### Schema and shell-artifact drift gates

Generated CLI/API reference artifacts must be regenerated locally
before committing whenever you touch the corresponding Rust types,
`clap` surface, or prose companion docs.

**xtask subcommands**

- `cargo xtask gen-cli-schemas`
- `cargo xtask gen-error-codes`
- `cargo xtask gen-cli-shell-artifacts`
- `cargo xtask gen-daemon-api`

**Drift gates**

- `bash tests/cli-json-drift.sh`
- `bash tests/error-codes-drift.sh`
- `bash tests/manpage-completion-drift.sh`
- `bash tests/daemon-api-drift.sh`
- `bash tests/cli-contract-coverage.sh`

A typical regeneration loop is:

```bash
cd packages
cargo xtask gen-cli-schemas
cargo xtask gen-error-codes
cargo xtask gen-cli-shell-artifacts
cargo xtask gen-daemon-api
cd ..
bash tests/cli-json-drift.sh
bash tests/error-codes-drift.sh
bash tests/manpage-completion-drift.sh
bash tests/daemon-api-drift.sh
bash tests/cli-contract-coverage.sh
```

## Submitting a pull request

- Use short imperative commit subjects with an area prefix, for example `net: fix ...` or `cli: add ...`.
- Keep one logical change per commit.
- Draft PRs are welcome.
- Reference resolved issues with `Closes #N`.

## Code is canon

When docs disagree with committed, passing code, the code wins. Update the docs to match reality and see [AGENTS.md](./AGENTS.md#existing-code-is-canon) for the full policy.

## Host-prepare gates

Contributors touching anything in `packages/nixling-host/`,
`packages/nixling-priv-broker/src/ops/`, or the host-prepare
docs (`docs/how-to/host-prepare.md`,
`docs/how-to/host-prepare.d/*.md`,
`docs/reference/{cgroup-delegation,inet-nixling-chains,privileges,support-matrix}.md`,
ADRs 0011–0014) MUST run the host-prepare Layer-1 gate set before
submitting:

```bash
# From the repo root:
bash tests/cgroup-delegation-oracle.sh
bash tests/pidfd-handoff.sh
bash tests/host-prepare-network.sh
bash tests/ipv6-off-readback.sh
bash tests/ifname-collision.sh
bash tests/path-safety-violation-fs.sh
bash tests/nft-coexistence.sh
bash tests/nft-foreign-rule-preservation.sh
bash tests/usbip-firewall-skeleton.sh
bash tests/kernel-module-matrix.sh
bash tests/device-node-matrix.sh
bash tests/ioctl-negative.sh
bash tests/runner-shape-preflight.sh
bash tests/minijail-version-check.sh
bash tests/multi-env-daemon-backed.sh
```

Each of these is also wired into `tests/static.sh` per the
integrator-owned wiring rule (scope agents add the standalone test
under `tests/`, the integrator registers it). Running them
standalone is recommended while iterating because the parallel-gate
pool in `static.sh` adds ≈ 4-10 minutes of wall-clock per gate.

### When to run the L2 KVM tests

The Layer-2 (`tests/integration/live/nixling-store.sh`, `tests/integration/live/audio.sh`) tests
require a live host with nixling activated and are NOT part of the
PR gate. Run them locally when:

- You change a privileged broker handler whose effect is only
  observable on a real host (cgroup delegation, pidfd handoff,
  `ApplyNftables` apply, `ApplyNmUnmanaged` apply, `ModprobeIfAllowed`).
- You bump the L3 distro pin in
  `tests/golden/l3-matrix/w3-{ubuntu,fedora,arch}.txt`. The
  panel-gated pin requires a fresh L2 run against the new image.
- You touch the runner-shape preflight or the minijail version
  check.

### Distro matrix expectations

PRs that touch host-prepare are reviewed against the Tier 0
(NixOS) and Tier 1 (Ubuntu 24.04 LTS) rows of
[`docs/reference/support-matrix.md`](./docs/reference/support-matrix.md).
Tier 1-later (Fedora/Arch) and Tier 2 (other Linux) issues are
filed and triaged but do not block merge unless the contributor
explicitly targets those tiers.

## License

nixling is licensed under [Apache-2.0](./LICENSE). By contributing, you agree to license your contributions under the same terms.
