"""Pinned cargo-bazel repository rule.

rules_rust 0.73.0 has an empty release URL table in its source archive and
otherwise creates a source-bootstrap repository.  The root module overrides
that repository with this checksum-verified binary download.
"""

def _cargo_bazel_repository_impl(repository_ctx):
    repository_ctx.download(
        url = repository_ctx.attr.url,
        output = "cargo-bazel",
        sha256 = repository_ctx.attr.sha256,
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
        "sha256": attr.string(mandatory = True),
        "url": attr.string(mandatory = True),
    },
)
