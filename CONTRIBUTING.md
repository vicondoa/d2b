# Contributing to d2b

For repo-specific operational policy, see [AGENTS.md](./AGENTS.md).

## Filing issues

- Use [GitHub Issues](https://github.com/vicondoa/d2b/issues) for bugs, docs fixes, and feature requests.
- Include a minimal reproduction, expected vs actual behavior, and any relevant logs.
- Include the d2b version: `d2b --version` on an installed host, or the repo tag / commit you tested.
- **Do not** file security vulnerabilities publicly; follow [SECURITY.md](./SECURITY.md).

## Setting up a dev environment

1. Clone the repo and enter it:
   ```bash
   git clone https://github.com/vicondoa/d2b.git
   cd d2b
   ```
2. Install Nix with flakes enabled (`experimental-features = nix-command flakes`).
3. No separate `nix develop` shell is needed.
4. Validate the framework with:
   ```bash
   nix flake check --no-build --all-systems
   ```

## Running quality gates

- `make check` is the PR-equivalent Layer-1 entry point.
- The checked Layer-1 manifest and Rust `xtask` own validation, parallel local
  execution, check discovery, and generated workflow rendering. Use the
  existing `make` entry points. The direct equivalents, run from `packages/`,
  are `cargo xtask layer1 validate`, `cargo xtask layer1 run-local`, and
  `cargo xtask layer1 workflow <write|check>`; do not create an ad-hoc
  orchestrator.
- Run the smallest relevant focused preflight before opening or updating a PR.
  Final CI, local/host validation, and review run concurrently on the immutable
  PR tree.
- See [tests/README.md](./tests/README.md) for the full test layering and optional Layer-2 integration tests.

<a id="rust-workspace-checks"></a>

### Rust workspace checks

The `packages/` Cargo workspace is gated by the manifest-owned
`make test-rust` job and by `nix flake check --no-build --all-systems`.
For a focused local run:

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

All d2b worktrees on paydro's host share Cargo build artifacts via
repo-local `.cargo/config.toml` files:

- `packages/.cargo/config.toml` → `/home/paydro/.cache/d2b-cargo-target/workspace`
- `packages/d2b-priv-broker/.cargo/config.toml` → `/home/paydro/.cache/d2b-cargo-target/broker`
- `packages/d2b-guest-shell-runner/.cargo/config.toml` → the helper workspace target dir
- `packages/d2b-core/fuzz/.cargo/config.toml` → `/home/paydro/.cache/d2b-cargo-target/fuzz`

Cargo's internal locking makes concurrent worktree builds safe, but a
very old checkout may pay one slower rebuild while incremental state is
refreshed in the shared cache.

The persistent-shell feasibility helper is a standalone excluded workspace. Run
it explicitly when iterating on that crate:

```bash
cargo --manifest-path packages/d2b-guest-shell-runner/Cargo.toml fmt --check
cargo --manifest-path packages/d2b-guest-shell-runner/Cargo.toml clippy --workspace --all-targets --features real-libshpool -- -D warnings
cargo --manifest-path packages/d2b-guest-shell-runner/Cargo.toml test --workspace --features real-libshpool
cargo deny --manifest-path packages/d2b-guest-shell-runner/Cargo.toml check --config packages/d2b-guest-shell-runner/deny.toml
cargo audit --file packages/d2b-guest-shell-runner/Cargo.lock --ignore RUSTSEC-2024-0384
```

Use the owning crate test or `make test-rust` while iterating. The legacy
monolithic `make check-static` entry point remains available, but it is not
where new coverage or orchestration is added.

#### Schema and shell-artifact drift gates

Generated CLI/API reference artifacts must be regenerated locally
before committing whenever you touch the corresponding Rust types,
`clap` surface, or prose companion docs.

**xtask subcommands**

- `cargo xtask gen-cli-schemas`
- `cargo xtask gen-error-codes`
- `cargo xtask gen-cli-shell-artifacts`
- `cargo xtask gen-daemon-api`

A typical regeneration loop is:

```bash
cd packages
cargo xtask gen-cli-schemas
cargo xtask gen-error-codes
cargo xtask gen-cli-shell-artifacts
cargo xtask gen-daemon-api
cd ..
make test-drift
```

## Submitting a pull request

- Use short imperative commit subjects with an area prefix, for example `net: fix ...` or `cli: add ...`.
- Keep one logical change per commit.
- For dependent work, use Git Town to own parent topology, propose ordinary
  GitHub PRs, synchronize, restack, and retarget the graph. Use
  `git town propose --stack --non-interactive --no-browser`.
- Commit the candidate, run focused preflight, open or update the PR/stack, and
  then create the canonical `xtask` snapshot of that exact open state before
  final long validation or panel review.
- GitHub CI, final local/host validators, and the full end-of-wave panel may be
  pending when the PR opens and run concurrently. Every required lane and the
  tree-bound seal must pass before merge.
- Any content change invalidates validator and panel results. History-only
  reuse requires canonical `xtask` proof of byte-identical content and rerun CI.
- Keep evidence and panel output external. The PR body contains dependency,
  base/head/tree, `candidate_id`/`content_id`, and check-status summaries only,
  with no raw output, AI, assistant, tool, model, run, or provider metadata.
- Run `cd packages && cargo xtask delivery wave help` for the canonical,
  machine-readable delivery command and option index. The `delivery` namespace
  is mandatory.
- ADR-scale branches merge through GitHub only. After merge, restore the primary
  clone to `main` and fast-forward it; never locally merge the branch into
  `main` beforehand.
- Draft PRs are welcome once focused preflight has passed.
- Reference resolved issues with `Closes #N`.

## Code is canon

When docs disagree with committed, passing code, the code wins. Update the docs to match reality and see [AGENTS.md](./AGENTS.md#existing-code-is-canon) for the full policy.

## Host-prepare gates

Contributors touching anything in `packages/d2b-host/`,
`packages/d2b-priv-broker/src/ops/`, or the host-prepare docs
(`docs/how-to/host-prepare.md`, `docs/how-to/host-prepare.d/*.md`,
`docs/reference/{cgroup-delegation,inet-d2b-chains,privileges,support-matrix}.md`,
ADRs 0011–0014) must cover the change through the closed taxonomy in
[`tests/AGENTS.md`](./tests/AGENTS.md). Use focused owning-crate or nix-unit
tests while iterating, then include the applicable manifest jobs:

```bash
make test-rust
make test-flake
make test-policy
```

Do not add a standalone `tests/*.sh` gate or wire new work into
`tests/static.sh`. If the Layer-1 graph itself changes, edit
`tests/layer1-jobs.json`, then run:

```bash
cd packages
cargo xtask layer1 validate
cargo xtask layer1 workflow write
cargo xtask layer1 workflow check
```

### When to run the L2 KVM tests

The Layer-2 (`tests/integration/live/d2b-store.sh`,
`tests/integration/live/audio.sh`) tests require a live host with d2b activated
and do not run in GitHub CI. Run applicable tests in the final validator lane
after the PR opens when:

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

d2b is licensed under [Apache-2.0](./LICENSE). By contributing, you agree to license your contributions under the same terms.
