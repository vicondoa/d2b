load("@rules_shell//shell:sh_binary.bzl", "sh_binary")
load("@rules_shell//shell:sh_test.bzl", "sh_test")

GENERATED_ARTIFACT_COMMANDS = [
    "gen-schemas",
    "gen-zone-storage-schema",
    "gen-cli-schemas",
    "gen-zone-schemas",
    "gen-zone-nix-options",
    "gen-resource-schemas",
    "gen-error-codes",
    "gen-provider-packaging",
    "gen-semantic-service-schemas",
    "gen-cli-shell-artifacts",
    "gen-resource-proto",
    "gen-resource-ttrpc",
    "gen-daemon-api",
    "gen-package-policy-inputs",
]

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

def generated_artifact_generator(name, data):
    sh_binary(
        name = name,
        srcs = ["//:tests/tools/generate-artifacts.sh"],
        args = ["$(rootpath :xtask)"] + GENERATED_ARTIFACT_COMMANDS,
        data = [
            ":xtask",
            "//:BUILD.bazel",
            "//:Cargo.toml",
            "//:Cargo.lock",
            "//:flake.nix",
            "//:.github/CODEOWNERS",
        ] + data,
        visibility = ["//visibility:public"],
    )
