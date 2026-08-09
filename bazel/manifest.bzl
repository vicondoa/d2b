"""Graph binding for targets that produce live Bazel evidence."""

def d2b_evidence_manifest(name, targets):
    """Make the real evidence targets reachable without metadata-only tags."""

    native.filegroup(
        name = name,
        srcs = targets,
    )
