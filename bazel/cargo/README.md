# Cargo Universe hubs

The two repositories declared by the root `MODULE.bazel` are independent
`crate.from_cargo` hubs:

| Hub | Cargo manifest and lock | Bazel-side lock |
| --- | --- | --- |
| `product` | `packages/Cargo.toml`, `packages/Cargo.lock` | `product.lock` |
| `walker` | `tests/tools/no-bash-ast-walker/Cargo.toml`, `Cargo.lock` | `walker.lock` |

Each hub supplies its committed Bazel-side lock, authoritative Cargo lock,
and `skip_cargo_lockfile_overwrite = True`. Cargo manifests and Cargo locks
remain the dependency authority; the Bazel-side locks only pin the rendered
crate graph.

The product hub contains the broker and guest runner as first-party packages.
`packages/Cargo.guest.lock` is deliberately not a hub. It is a generator and
cache-key input for the Nix guest source path, and no Bazel gate surface builds
against it.

The supported regeneration command is one explicit hub at a time:

```text
cargo xtask bazel-repin --hub <product|walker>
```

The module graph has its separate refresh command:

```text
cargo xtask bazel-module-refresh
```

The generator commands run from `packages/`. Bazel and package-policy write
commands create complete scratch previews under `.scratch/` and return the
paths that were written. To promote a preview, use the explicit install mode;
it atomically rewrites the exact owned tracked outputs and removes stale
sidecars before returning their repository-relative paths:

```text
cargo xtask gen-bazel --install
cargo xtask gen-package-policy-inputs --install
```

The default schema command writes the authoritative JSON schemas under
`docs/reference/schemas/v2/`. The schema reproducibility gate hashes that
committed directory before and after generation, so missing, extra, stale, or
absent schema roots cannot pass by being recreated during the check.

The `cargo-bazel` executable is fetched from the pinned rules_rust release
asset by `cargo_bazel.bzl` using its URL and SHA-256. The source-bootstrap
repository in rules_rust is overridden and is not a permitted fallback.
