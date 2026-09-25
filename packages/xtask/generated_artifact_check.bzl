load("@rules_shell//shell:sh_binary.bzl", "sh_binary")
load("@rules_shell//shell:sh_test.bzl", "sh_test")

GENERATED_ARTIFACT_COMMANDS = [
    "gen-schemas",
    "gen-zone-storage-schema",
    "gen-cli-schemas",
    "gen-zone-schemas",
    "gen-zone-nix-options",
    "gen-layer-catalogs",
    "gen-error-codes",
    "gen-provider-packaging",
    "gen-nix-inventories",
    "gen-semantic-service-schemas",
    "gen-cli-shell-artifacts",
    "gen-resource-proto",
    "gen-resource-ttrpc",
    "gen-daemon-api",
    "gen-broker-operations",
    "gen-package-policy-inputs",
]

def generated_artifact_check(name, command, data, env_inherit = [], tags = []):
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
        # A generator that shells out (for example to `cargo`) needs the
        # invoking environment: the sandbox supplies neither the host PATH nor
        # the tool homes the caller has warmed.
        env_inherit = env_inherit,
        tags = tags,
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
