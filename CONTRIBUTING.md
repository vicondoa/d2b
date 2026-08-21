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
4. Run focused checks for the components you change. Broader flake checks
   remain available when the changed surface requires them.

## Running quality gates

Run focused tests for each changed component. Use `make check` or another
broader Layer-1 target when the changed surface needs that coverage; none is a
prerequisite for opening a PR or starting review.
Container, host, live, hardware, and performance lanes are conditional on the
changed surface. See [tests/README.md](./tests/README.md) for the test layering
and public conditional integration targets.

`make check` invokes the single fixed Bazel graph. A developer host uses
BuildBuddy for eligible actions; GitHub Layer-1 runs the same graph locally
through `nix develop .#bazel` without a provider credential. Standalone Cargo
commands remain available for direct development.

<a id="rust-workspace-checks"></a>

### Rust workspace checks

The repository-root Cargo workspace is covered by CI's Layer-1 jobs. For a
local change, run the focused test and lint commands for the components you
changed; the aggregate gates remain available when the changed surface needs
broader coverage.

```bash
# Examples; select the commands that cover the changed components.
cargo --manifest-path Cargo.toml test -p <changed-crate>
cargo --manifest-path Cargo.toml clippy -p <changed-crate> --all-targets -- -D warnings

# Optional broader Layer-1 aggregate.
make check
```

The pinned toolchain in `rust-toolchain.toml` is honored by the repository-root
workspace. See
[ADR 0009](docs/adr/0009-rust-toolchain-msrv-and-supply-chain.md) for
toolchain, MSRV, and supply-chain policy.

The repository-root `.cargo/config.toml` governs the product workspace. Cargo
uses the conventional root `target/` directory by default; the broker's
serial feature streams may set explicit execution-only sibling target
directories, while independent fuzz and proof workspaces retain their own
configuration.

Cargo's internal locking makes concurrent worktree builds safe, but a
very old checkout may pay one slower rebuild while incremental state is
refreshed in the shared cache.

The persistent-shell feasibility helper is a standalone excluded workspace. Run
it explicitly when iterating on that crate:

```bash
cargo --manifest-path Cargo.toml fmt --check
cargo --manifest-path Cargo.toml clippy -p d2b-guest-shell-runner --all-targets --features real-libshpool -- -D warnings
cargo --manifest-path Cargo.toml test -p d2b-guest-shell-runner --features real-libshpool
cargo deny --manifest-path Cargo.toml check --config deny.toml
cargo xtask gen-package-policy-inputs --check
```

The fixed Bazel graph remains the broader Layer-1 gate when the changed
surface needs it. Focused labels are preferred while iterating.

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
cargo xtask gen-cli-schemas
cargo xtask gen-error-codes
cargo xtask gen-cli-shell-artifacts
cargo xtask gen-daemon-api
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
`docs/reference/{cgroup-delegation,inet-d2b-chains,privileges,support-matrix}.md`)
MUST run the host-prepare Layer-1 gate set when the change
touches those host-prepare surfaces:

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

Each applicable check is wired into the fixed Bazel graph. Running the
owner-local label standalone is recommended while iterating.

### When to run the L2 KVM tests

The Layer-2 (`tests/integration/live/d2b-store.sh`, `tests/integration/live/audio.sh`) tests
require a live host with d2b activated and are NOT part of the
PR gate. Run them locally when:

- You change a privileged broker handler whose effect is only
  observable on a real host (cgroup delegation, pidfd handoff,
  `ApplyNftables` apply, `ApplyNmUnmanaged` apply, `ModprobeIfAllowed`).
- You bump the L3 distro pin in
  `tests/golden/l3-matrix/w3-{ubuntu,fedora,arch}.txt`. The
  pinned image requires a fresh L2 run against the new image.
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
