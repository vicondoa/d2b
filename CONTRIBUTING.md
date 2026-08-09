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

- `make check` is the PR-equivalent Layer-1 gate.
- `make test-unit` is the post-preflight Layer-1 development umbrella.
- `make check-static` retains the legacy/full-static `tests/static.sh` gate.
- `make check` runs the manifest's parse checks, smoke evals, assertion tests,
  manifest schema validation, and per-example flake checks.
- See [tests/README.md](./tests/README.md) for the full test layering and optional Layer-2 integration tests.

<a id="rust-workspace-checks"></a>

### Rust workspace checks

The product Cargo workspace is rooted at `packages/Cargo.toml` and uses
`packages/Cargo.lock` as its only authoritative product lock. It includes the
broker and guest shell runner. The no-bash AST walker is the separate gate-tool
workspace under `tests/tools/no-bash-ast-walker/`; it is not another product
workspace. `packages/Cargo.guest.lock` is a generated static-guest closure
input, not a product workspace or hub authority.

Use the Make targets for complete gates:

```bash
make test-rust
make test-rust-supply-chain
make test-policy
```

For a focused package-selection pass, enter the pinned development shell from
the repository root, then run the commands from `packages/`:

```bash
nix develop
cd packages
cargo fmt --all --check
cargo clippy --locked --workspace --all-targets \
  --exclude d2b-priv-broker --exclude d2b-guest-shell-runner -- -D warnings
cargo nextest run --locked --workspace \
  --exclude d2b-priv-broker --exclude d2b-guest-shell-runner
cargo test --locked -p d2b-priv-broker --no-default-features -- --test-threads 1
cargo test --locked -p d2b-priv-broker --no-default-features \
  --features layer1-bootstrap -- --test-threads 1
cargo test --locked -p d2b-priv-broker --no-default-features \
  --features fake-backends -- --test-threads 1
cargo fmt -p d2b-guest-shell-runner --check
cargo clippy --locked -p d2b-guest-shell-runner --no-default-features \
  --features real-libshpool --all-targets -- -D warnings
cargo nextest run --locked -p d2b-guest-shell-runner \
  --no-default-features --features real-libshpool
cargo deny check --config deny.toml bans licenses sources
cd ..
```

The focused commands select broker and guest packages from the unified product
workspace. The supply-chain target additionally checks these exact generated
policy contexts:

- `packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-gnu/broker-production`
- `packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-gnu/broker-production`
- `packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool`
- `packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool`

Do not run `cargo audit` against an ambient advisory database. The supported
`make test-rust-supply-chain` target resolves the flake's
`packages.<system>.rustsec-advisory-db` output, pinned to RustSec
`advisory-db` commit `831c50f4a4304068f125e603add6a8839f08b3eb` with Nix hash
`sha256-wXKYURZz76ZC5lbuDA1oVQA/MxSB3pSJ1raF1HG0oIc=`, and passes it to
`cargo audit` with `--no-fetch`.

The pinned toolchain in `packages/rust-toolchain.toml` is honored when Cargo
runs from inside `packages/`. See
[ADR 0009](docs/adr/0009-rust-toolchain-msrv-and-supply-chain.md) for
toolchain, MSRV, and supply-chain policy.

Each worktree keeps Cargo outputs local while sccache deduplicates compiled
outputs. The product workspace uses `packages/target`; the dedicated broker
and guest gate streams use stable package-selected target directories, and the
walker uses `tests/tools/no-bash-ast-walker/target`:

Cargo's internal locking makes concurrent worktree builds safe, while stable
target names keep sccache keys reusable and prevent feature-pass contention.

The persistent-shell feasibility helper is a product-workspace member. Run its
selected root-workspace stream explicitly when iterating on that crate:

```bash
cargo fmt --manifest-path packages/Cargo.toml -p d2b-guest-shell-runner --check
cargo clippy --locked --manifest-path packages/Cargo.toml \
  --package d2b-guest-shell-runner --no-default-features \
  --features real-libshpool --all-targets -- -D warnings
cargo test --locked --manifest-path packages/Cargo.toml \
  --package d2b-guest-shell-runner --no-default-features --features real-libshpool
cargo deny --metadata-path packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool/policy/metadata.json \
  check --config packages/d2b-guest-shell-runner/deny.toml
make test-rust-supply-chain
```

The selected broker and guest policy inputs are generated for GNU and musl
targets on both `x86_64-linux` and `aarch64-linux`. Package policy checks read
only their exact system-and-target paths; the root lock and generated
`Cargo.guest.lock` checks remain independent.

The release workflow builds every product binary from `packages/Cargo.toml`
with `--locked`, explicit package/bin/default-feature selectors, and copies
from `packages/target/release`. Its cache has one `packages -> target`
workspace mapping plus explicit broker and guest gate target directories.
The native `test-flake-aarch64` job realizes exactly these six checks on
`aarch64-linux`, then runs `make test-rust-supply-chain` on the same stable
head:

```text
broker-production-dependency-policy
guest-shell-runner-static-dependency-policy
broker-production-package-policy
guest-real-libshpool-package-policy
broker-host-artifact-contract
guest-static-elf
```

It does not use a foreign system, `--builders`, a remote builder, or an
advisory classification.

`bash tests/static.sh` also has a fast path for Rust-heavy gates:

- it resolves one shared Rust toolchain shell at the top of the run and
  reuses that PATH in child scripts instead of spawning a fresh `nix shell`
  per gate;
- independent Rust, schema, and example gates run behind a small semaphore
  controlled by `D2B_STATIC_JOBS` (default `4`);
- `bash tests/tools/static-timing.sh` writes a per-gate wall-clock report to
  `$ROOT/.static-timing.log`;
- to profile one gate in isolation, run `time bash tests/<gate>.sh`.

#### Schema and shell-artifact drift gates

Generated CLI/API reference artifacts must be regenerated locally
before committing whenever you touch the corresponding Rust types,
`clap` surface, or prose companion docs.

**xtask subcommands**

From the repository root, enter `nix develop`, then run `cd packages` before
using these contributor-only generators:

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

Contributors touching anything in `packages/d2b-host/`,
`packages/d2b-priv-broker/src/ops/`, or the host-prepare
docs (`docs/how-to/host-prepare.md`,
`docs/how-to/host-prepare.d/*.md`,
`docs/reference/{cgroup-delegation,inet-d2b-chains,privileges,support-matrix}.md`,
ADRs 0011-0014) MUST run the host-prepare Layer-1 gate set before
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

The Layer-2 (`tests/integration/live/d2b-store.sh`, `tests/integration/live/audio.sh`) tests
require a live host with d2b activated and are NOT part of the
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

d2b is licensed under [Apache-2.0](./LICENSE). By contributing, you agree to license your contributions under the same terms.
