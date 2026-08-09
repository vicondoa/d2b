"""Rust toolchain registration for the repository root module."""

load(
    ":defs.bzl",
    "D2B_BAZEL_CAPABILITY_ABI",
    "D2B_BAZEL_SANDBOX_OUTPUT",
    "D2B_BAZEL_VERSION",
    "d2b_rust_toolchain_versions",
)

D2B_CONFIGURED_BAZEL = struct(
    version = D2B_BAZEL_VERSION,
    output = D2B_BAZEL_SANDBOX_OUTPUT,
    capability_abi = D2B_BAZEL_CAPABILITY_ABI,
    strategy = "sandboxed",
    action_network = "none",
)

def d2b_toolchain_metadata_tags():
    """Expose the pinned toolchain identity to reachable graph nodes."""

    return [
        "d2b-bazel-version-{}".format(D2B_CONFIGURED_BAZEL.version),
        "d2b-bazel-capability-{}".format(D2B_CONFIGURED_BAZEL.capability_abi),
        "d2b-bazel-strategy-{}".format(D2B_CONFIGURED_BAZEL.strategy),
        "d2b-bazel-network-{}".format(D2B_CONFIGURED_BAZEL.action_network),
    ]

def d2b_register_rust_toolchains(rust_extension):
    """Register stable and dated nightly without a global channel setting."""

    rust_extension.toolchain(
        edition = "2021",
        versions = d2b_rust_toolchain_versions(),
    )
