"""Pinned cargo-bazel repository rule.

rules_rust 0.73.0 has an empty release URL table in its source archive and
otherwise creates a source-bootstrap repository. The root module overrides
that repository with a checksum-verified native release binary.
"""

def _cargo_bazel_repository_impl(repository_ctx):
    arch = repository_ctx.os.arch
    if arch == "amd64" or arch == "x86_64":
        url = repository_ctx.attr.x86_64_url
        sha256 = repository_ctx.attr.x86_64_sha256
    elif arch == "aarch64" or arch == "arm64":
        url = repository_ctx.attr.aarch64_url
        sha256 = repository_ctx.attr.aarch64_sha256
    else:
        fail("unsupported native cargo-bazel architecture: {}".format(arch))
    repository_ctx.download(
        url = url,
        output = "cargo-bazel",
        sha256 = sha256,
        executable = True,
    )
    repository_ctx.file(
        "BUILD.bazel",
        """package(default_visibility = ["//visibility:public"])

exports_files(["cargo-bazel"])
""",
    )

cargo_bazel_repository = repository_rule(
    implementation = _cargo_bazel_repository_impl,
    attrs = {
        "aarch64_sha256": attr.string(mandatory = True),
        "aarch64_url": attr.string(mandatory = True),
        "x86_64_sha256": attr.string(mandatory = True),
        "x86_64_url": attr.string(mandatory = True),
    },
)
