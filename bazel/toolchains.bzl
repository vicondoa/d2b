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

def d2b_register_rust_toolchains(rust_extension):
    """Register stable and dated nightly without a global channel setting."""

    rust_extension.toolchain(
        edition = "2021",
        versions = d2b_rust_toolchain_versions(),
    )
