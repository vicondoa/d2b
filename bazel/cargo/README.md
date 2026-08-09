# Cargo Universe hubs

The authoritative workspace, hub, policy-context, and regeneration procedure
lives in
[`../../docs/contributing/bazel-and-policy.md`](../../docs/contributing/bazel-and-policy.md).
This local README remains a short pointer for contributors working beside the
two Bazel-side locks.

The only hubs are `product` and `walker`; `Cargo.guest.lock` is not a hub.
Run the explicit `cargo xtask bazel-repin --hub <product|walker>` command from
`packages/`, then refresh `MODULE.bazel.lock` after the selected hub. Review
scratch previews before using `--install`, and use the matching `--check`
command afterward.

The `cargo-bazel` executable is fetched from the pinned rules_rust release
asset by `cargo_bazel.bzl` using its URL and SHA-256. The source-bootstrap
repository in rules_rust is overridden and is not a permitted fallback.
