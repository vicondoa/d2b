load("@rules_shell//shell:sh_test.bzl", "sh_test")

def generated_artifact_check(name, command, data):
    sh_test(
        name = name,
        srcs = ["//:tests/tools/generated-artifact-check.sh"],
        args = [command],
        data = [
            ":xtask",
            "//:BUILD.bazel",
            "//:Cargo.toml",
            "//:flake.nix",
            "@python3//:bin/python3",
        ] + data,
        env = {
            "D2B_PYTHON_RUNFILE": "$(rootpath @python3//:bin/python3)",
            "D2B_XTASK_RUNFILE": "$(rootpath :xtask)",
        },
        visibility = ["//visibility:public"],
    )
