# Bazel and package-policy workflows

This page is the procedure detail for the workspace and native policy
surfaces. The always-loaded router is [`../../AGENTS.md`](../../AGENTS.md);
the committed authority for the context and native-check matrix is
[`../../tests/golden/native-policy-check-manifest.json`](../../tests/golden/native-policy-check-manifest.json).

## Workspace authorities

The product Cargo workspace is `packages/Cargo.toml` with the sole product
lock `packages/Cargo.lock`. The broker and guest shell runner are members of
that workspace. `packages/Cargo.guest.lock` is generated static-guest closure
input, not a workspace or Bazel hub. The no-bash AST walker is a separate
workspace under `tests/tools/no-bash-ast-walker/` with its own lock.

Bazel has exactly two `crate.from_cargo` hubs:

| Hub | Manifest and Cargo lock | Bazel-side lock |
| --- | --- | --- |
| `product` | `packages/Cargo.toml`, `packages/Cargo.lock` | `bazel/cargo/product.lock` |
| `walker` | `tests/tools/no-bash-ast-walker/Cargo.toml`, its `Cargo.lock` | `bazel/cargo/walker.lock` |

The Bazel-side locks are rendered graph inputs, not alternate Cargo
authorities. They use `skip_cargo_lockfile_overwrite = True`.

## Regeneration order

Run generators from `packages/` inside the pinned development shell:

```text
cargo xtask bazel-repin --hub product
cargo xtask bazel-repin --hub walker
cargo xtask bazel-module-refresh
cargo xtask gen-bazel
cargo xtask gen-package-policy-inputs
```

Repin one hub at a time. A product manifest or lock change refreshes the root
Cargo lock, the product hub, and then `MODULE.bazel.lock`; a walker change
refreshes the walker lock, walker hub, and then the module lock. Prove the
untouched authority is byte-identical and run each command again as a clean
no-op before committing generated output.

`gen-bazel` and `gen-package-policy-inputs` write complete previews below
`.scratch/`. Review the preview before promoting it with the explicit
`--install` mode:

```text
cargo xtask gen-bazel --install
cargo xtask gen-package-policy-inputs --install
```

Only the exact generator-owned tracked outputs may be promoted. Use the
matching `--check` command after installation. Do not hand-edit generated
Bazel files, policy locks, or module locks.

The xtask CLI keeps its existing one-line stdout completion contract,
`<command> generated <count> file(s)`. Returned path lists are an internal
generator result and are not additional stdout records. After installation,
use `git status --short --untracked-files=all` for the changed-path census.
For `gen-bazel`, compare that census with
`bazel/generated/output-manifest.json`; package-policy output is confined
below `packages/policy-inputs/`.

## Native policy and check matrix

The manifest names four policy contexts, each on both native systems:

- broker GNU production: `d2b-priv-broker`, default features disabled;
- guest musl production: `d2b-guest-shell-runner`, `real-libshpool`, default
  features disabled.

It also names the exact six enforcing native checks:

```text
broker-production-dependency-policy
guest-shell-runner-static-dependency-policy
broker-production-package-policy
guest-real-libshpool-package-policy
broker-host-artifact-contract
guest-static-elf
```

The flake, xtask, shell gates, CI workflow generator, and Bazel target
inventory consume or exactly verify this manifest. `video-binary-contract` is
a separate realized check and is intentionally not part of the six
architecture-specific checks.

The native arm workflow realizes the six checks on the native
`ubuntu-24.04-arm` runner, binds one stable pull-request head, and then runs
`make test-rust-supply-chain` on that same head. It must not pass a foreign
`--system`, configure a remote builder, or classify the job as advisory.

The supply-chain target resolves the flake's
`packages.<system>.rustsec-advisory-db` output, pinned to RustSec advisory-db
commit `831c50f4a4304068f125e603add6a8839f08b3eb` with Nix hash
`sha256-wXKYURZz76ZC5lbuDA1oVQA/MxSB3pSJ1raF1HG0oIc=`, and passes that path to
`cargo audit --no-fetch`. Do not run an audit against an ambient database.

## Validation

The focused checks are:

```bash
make test-rust-supply-chain
make test-drift
make test-flake
```

For a complete Layer-1 run use `make check`. The Layer-1 manifest remains the
authority for enforcement classification; an advisory pass is not evidence.
