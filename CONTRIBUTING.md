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
3. Run `make <target>` directly from the checkout. The Makefile detects the
   d2b shell contract and enters the pinned `.#bazel` shell once when needed;
   no global Bazel installation is required.
4. For an interactive contributor session, use `nix develop`. This is the
   complete shell with the pinned Bazel and Rust toolchains. The focused
   `nix develop .#bazel` shell is intended for short Bazel/Make commands.
   Optional direnv integration may enter the shell automatically, but is not
   required.

## Running quality gates

Run focused tests for each changed component. Use `make check` or another
broader Layer-1 target when the changed surface needs that coverage; none is a
prerequisite for opening a PR or starting review.
Container, host, live, hardware, and performance lanes are conditional on the
changed surface. See [tests/README.md](./tests/README.md) for the test layering
and public conditional integration targets.

`make check` invokes the single fixed Bazel graph. A developer host uses
BuildBuddy for eligible actions; GitHub Layer-1 runs the same graph locally
with `D2B_BAZEL_PROFILE=local` and `D2B_BAZEL_UNTRUSTED=1`. CI only needs Nix
installed; Make selects the pinned shell and preserves those profile and trust
variables. Cargo manifests and `Cargo.lock` remain rules_rs metadata
authority, but Cargo is not a contributor or CI gate.

<a id="rust-workspace-checks"></a>

### Rust and Nix owner checks

Use the owner-local Bazel label for the crate or Nix surface you changed, then
use the matching Make alias when a broader lane is useful:

```bash
# Make aliases are the stable public interface.
make test-rust
make test-nix-unit
make test-policy
make check

# Direct labels use the focused pinned shell explicitly.
nix develop --no-write-lock-file .#bazel -c bazel test //packages/<crate>:<owner-test>
nix develop --no-write-lock-file .#bazel -c bazel test //bazel/checks/nix:nix-unit-<surface>
```

The complete crate surface, including doctests, harness-free binaries,
fixtures, feature variants, and policy checks, is declared by Bazel BUILD
targets. The pinned Rust toolchain, Cargo manifests, and lockfiles remain
rules_rs inputs; standalone crate Cargo commands may still work for local
debugging but are not documented or required validation.

#### Schema and shell-artifact drift gates

Generated CLI/API reference artifacts must be regenerated locally
before committing whenever you touch the corresponding Rust types,
`clap` surface, or prose companion docs.

**xtask subcommands** (run them through the focused shell):

- `nix develop --no-write-lock-file .#bazel -c bazel run //packages/xtask:xtask -- gen-cli-schemas`
- `nix develop --no-write-lock-file .#bazel -c bazel run //packages/xtask:xtask -- gen-error-codes`
- `nix develop --no-write-lock-file .#bazel -c bazel run //packages/xtask:xtask -- gen-cli-shell-artifacts`
- `nix develop --no-write-lock-file .#bazel -c bazel run //packages/xtask:xtask -- gen-daemon-api`

**Drift gates**

- `bash tests/cli-json-drift.sh`
- `bash tests/error-codes-drift.sh`
- `bash tests/manpage-completion-drift.sh`
- `bash tests/daemon-api-drift.sh`
- `bash tests/cli-contract-coverage.sh`

A typical regeneration loop is:

```bash
nix develop --no-write-lock-file .#bazel -c bazel run //packages/xtask:xtask -- gen-cli-schemas
nix develop --no-write-lock-file .#bazel -c bazel run //packages/xtask:xtask -- gen-error-codes
nix develop --no-write-lock-file .#bazel -c bazel run //packages/xtask:xtask -- gen-cli-shell-artifacts
nix develop --no-write-lock-file .#bazel -c bazel run //packages/xtask:xtask -- gen-daemon-api
make test-drift
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
