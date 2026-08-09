"""Graph bindings for generated Bazel target inventories."""

def d2b_generated_target_manifest(name, product_targets, package_policy_targets):
    """Consume both generated target surfaces as one reachable graph node."""

    native.filegroup(
        name = name,
        srcs = [
            "//bazel/generated:package-policy-targets.bzl",
            "//bazel/generated:product-targets.bzl",
        ],
        tags = [
            "d2b-product-target-count-{}".format(len(product_targets)),
            "d2b-package-policy-target-count-{}".format(len(package_policy_targets)),
        ],
    )
