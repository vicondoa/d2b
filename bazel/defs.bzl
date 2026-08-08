"""Repository-owned Bazel constants and small, hermetic helper macros."""

D2B_STABLE_RUST_VERSION = "1.97.0"

# rules_rust 0.73.0 consumes a dated channel in slash form.  The repository
# pin is the nightly-2026-02-16 toolchain; the spelling is normalized by the
# pinned extension at the registration boundary.
D2B_NIGHTLY_RUST_PIN = "nightly-2026-02-16"
D2B_NIGHTLY_RUST_VERSION = "nightly/2026-02-16"
D2B_BAZEL_VERSION = "8.6.0"
D2B_BAZEL_SANDBOX_OUTPUT = "pkgs/bazel-8.6.0-seccomp"
D2B_BAZEL_CAPABILITY_ABI = "d2b-bazel-seccomp-abi-v1"
D2B_BAZEL_ACTION_NETWORK = "none"

def d2b_rust_toolchain_versions():
    """Return the two repository Rust toolchain pins in extension syntax."""

    return [
        D2B_STABLE_RUST_VERSION,
        D2B_NIGHTLY_RUST_VERSION,
    ]

def d2b_broker_test_tags():
    """Keep each broker feature context in Bazel's exclusive test lane."""

    return ["exclusive"]
