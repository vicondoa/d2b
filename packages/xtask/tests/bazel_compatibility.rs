#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const BAZEL_TEST_TOOL_PATH: &str = "/run/current-system/sw/bin:/bin:/usr/bin";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask lives under packages/xtask")
        .to_path_buf()
}

fn read_repo_file(relative: &str) -> String {
    std::fs::read_to_string(repo_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn bazel_binary() -> PathBuf {
    std::env::var_os("D2B_BAZEL_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("bazel"))
}

fn bazel_output_user_root() -> PathBuf {
    if let Some(path) = std::env::var_os("D2B_BAZEL_OUTPUT_USER_ROOT") {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        return PathBuf::from(path).join("d2b-bazel-compat-output");
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|path| path.join(".cache/d2b-bazel-compat-output"))
        .unwrap_or_else(|| panic!("D2B_BAZEL_OUTPUT_USER_ROOT or HOME is required"))
}

fn bazel_shell_executable() -> PathBuf {
    repo_root().join("tests/fixtures/bazel/compat/tools/bazel-shell")
}

fn run_bazel_output(arguments: &[&str]) -> std::process::Output {
    let mut command = Command::new(bazel_binary());
    command
        .current_dir(repo_root())
        .arg(format!(
            "--output_user_root={}",
            bazel_output_user_root().display()
        ))
        .arg(arguments.first().expect("Bazel command is non-empty"))
        .arg("--lockfile_mode=error")
        .arg("--repo_contents_cache=")
        .arg("--symlink_prefix=.scratch/bazel-")
        .arg(format!(
            "--shell_executable={}",
            bazel_shell_executable().display()
        ));
    if arguments.first() == Some(&"test") {
        command.arg(format!("--test_env=PATH={BAZEL_TEST_TOOL_PATH}"));
    }
    if arguments.first() == Some(&"run") {
        command.arg(format!(
            "--run_under=/run/current-system/sw/bin/env PATH={BAZEL_TEST_TOOL_PATH}"
        ));
    }
    command.args(&arguments[1..]);
    command
        .output()
        .unwrap_or_else(|error| panic!("run Bazel: {error}"))
}

fn run_bazel(arguments: &[&str]) {
    let output = run_bazel_output(arguments);
    assert!(
        output.status.success(),
        "Bazel command failed: bazel {}\nstdout:\n{}\nstderr:\n{}",
        arguments.join(" "),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_prebuilt_proto_action_graph() {
    let proto_actions = run_bazel_output(&[
        "aquery",
        "--noshow_progress",
        "--output=textproto",
        r#"mnemonic("GenProtoDescriptorSet", //tests/fixtures/bazel/compat/proto:compat_proto)"#,
    ]);
    assert!(
        proto_actions.status.success(),
        "Bazel must analyze the prebuilt protoc action graph:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&proto_actions.stdout),
        String::from_utf8_lossy(&proto_actions.stderr)
    );
    let proto_actions = String::from_utf8_lossy(&proto_actions.stdout);
    assert!(
        proto_actions.contains("mnemonic: \"GenProtoDescriptorSet\"")
            && proto_actions.contains("prebuilt_protoc")
            && proto_actions.contains("protoc"),
        "the compatibility graph must resolve a prebuilt protoc descriptor action"
    );
    assert!(
        !proto_actions.contains("CppCompile") && !proto_actions.contains("external/grpc"),
        "the prebuilt protoc probe must not compile external protobuf or gRPC C++"
    );
}

fn protobuf_policy_sources() -> [&'static str; 7] {
    [
        ".bazelrc",
        "MODULE.bazel",
        "BUILD.bazel",
        "tests/fixtures/bazel/compat/MODULE.bazel",
        "tests/fixtures/bazel/compat/BUILD.bazel",
        "tests/fixtures/bazel/compat/proto/BUILD.bazel",
        "tests/fixtures/bazel/compat/proto/compat.proto",
    ]
}

#[test]
fn protobuf_controls_are_pinned_and_reject_archived_or_direct_compilers() {
    // Controls follow Aspect's Bazel 9 protobuf guidance:
    // https://aspect.build/blog/bazel-9-protobuf
    assert_eq!(read_repo_file(".bazelversion").trim(), "9.2.0");

    let bazelrc = read_repo_file(".bazelrc");
    for required in [
        "common --@protobuf//bazel/toolchains:prefer_prebuilt_protoc",
        "common --per_file_copt=external/.*protobuf.*@--PROTOBUF_WAS_NOT_SUPPOSED_TO_BE_BUILT",
        "common --host_per_file_copt=external/.*protobuf.*@--PROTOBUF_WAS_NOT_SUPPOSED_TO_BE_BUILT",
        "common --per_file_copt=external/.*grpc.*@--GRPC_WAS_NOT_SUPPOSED_TO_BE_BUILT",
        "common --host_per_file_copt=external/.*grpc.*@--GRPC_WAS_NOT_SUPPOSED_TO_BE_BUILT",
    ] {
        assert!(
            bazelrc.contains(required),
            "protobuf policy is missing `{required}`"
        );
    }
    assert!(
        !bazelrc.contains("--incompatible_enable_proto_toolchain_resolution"),
        "Bazel 9 must not carry the earlier proto toolchain compatibility flag"
    );

    let root_module = read_repo_file("MODULE.bazel");
    assert!(
        root_module.contains("bazel_dep(name = \"protobuf\", version = \"33.4\")"),
        "the root module must resolve Bazel 9's protobuf 33.4 graph"
    );

    for relative in protobuf_policy_sources() {
        let source = read_repo_file(relative);
        for forbidden in ["toolchains_protoc", "rules_proto", "@protobuf//:protoc"] {
            assert!(
                !source.contains(forbidden),
                "{relative} must not add archived or direct protobuf compiler `{forbidden}`"
            );
        }
    }
}

#[test]
fn compatibility_fixture_declares_prebuilt_protoc_probe_without_cpp_targets() {
    let build = read_repo_file("tests/fixtures/bazel/compat/proto/BUILD.bazel");
    assert!(
        build.contains("proto_library("),
        "the compatibility fixture must exercise Bazel proto toolchain resolution"
    );
    assert!(
        build.contains("# keep"),
        "the proto compatibility target must remain an explicit checked-in exception"
    );

    let source = read_repo_file("tests/fixtures/bazel/compat/proto/compat.proto");
    assert!(
        source.contains("syntax = \"proto3\";"),
        "the compatibility proto must use a deterministic proto3 source"
    );
    assert!(
        !build.contains("cc_proto_library(") && !build.contains("grpc"),
        "the compatibility fixture must not introduce external protobuf or gRPC C++ targets"
    );
}

#[test]
fn exact_bazel_version_resolves_prebuilt_protoc_without_external_cpp() {
    let version = Command::new(bazel_binary())
        .arg("--version")
        .output()
        .expect("run Bazel --version");
    assert!(
        version.status.success(),
        "Bazel --version failed: {}",
        String::from_utf8_lossy(&version.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "bazel 9.2.0",
        "U1 must analyze the fixture with the exact pinned Bazel version"
    );

    assert_prebuilt_proto_action_graph();
}

#[test]
fn pins_exact_upstream_bazel_and_unmodified_bzlmod_tools() {
    assert_eq!(read_repo_file(".bazelversion").trim(), "9.2.0");

    let root_module = read_repo_file("MODULE.bazel");
    for required in [
        "module(",
        "bazel_dep(name = \"rules_rust\", version = \"0.67.0\")",
        "bazel_dep(name = \"rules_go\", version = \"0.59.0\")",
        "bazel_dep(name = \"gazelle\", version = \"0.47.0\")",
        "bazel_dep(name = \"gazelle_rust\", version = \"0.1.0\")",
        "bazel_dep(name = \"protobuf\", version = \"33.4\")",
    ] {
        assert!(
            root_module.contains(required),
            "root MODULE.bazel is missing `{required}`"
        );
    }
    assert!(
        !root_module.contains("WORKSPACE"),
        "the migration must not retain a WORKSPACE compatibility path"
    );
    assert!(
        !root_module.contains("local_path_override")
            && !root_module.contains("archive_override")
            && !root_module.contains("patch"),
        "root MODULE.bazel must use unmodified upstream modules without overrides or patches"
    );
    let lock = read_repo_file("MODULE.bazel.lock");
    assert!(
        lock.contains("\"lockFileVersion\"") && lock.contains("@@gazelle_rust+//Cargo.lock"),
        "Bzlmod must have a checked-in lock with stable external-repository inputs"
    );
    assert!(
        !lock.contains("FILE:@@//.scratch/"),
        "Bzlmod lock must not capture checkout-local output paths"
    );

    let bazelrc = read_repo_file(".bazelrc");
    for required in [
        "common --@protobuf//bazel/toolchains:prefer_prebuilt_protoc",
        "common --per_file_copt=external/.*protobuf.*@--PROTOBUF_WAS_NOT_SUPPOSED_TO_BE_BUILT",
        "common --host_per_file_copt=external/.*protobuf.*@--PROTOBUF_WAS_NOT_SUPPOSED_TO_BE_BUILT",
        "common --per_file_copt=external/.*grpc.*@--GRPC_WAS_NOT_SUPPOSED_TO_BE_BUILT",
        "common --host_per_file_copt=external/.*grpc.*@--GRPC_WAS_NOT_SUPPOSED_TO_BE_BUILT",
    ] {
        assert!(
            bazelrc.contains(required),
            "repository .bazelrc is missing `{required}`"
        );
    }

    let fixture_module = read_repo_file("tests/fixtures/bazel/compat/MODULE.bazel");
    for required in [
        "use_extension(\"@rules_rust//rust:extensions.bzl\", \"rust\")",
        "use_extension(\"@rules_rust//crate_universe:extension.bzl\", \"crate\")",
        "crate.from_cargo(",
        "use_repo(crate, \"crates\")",
        "register_toolchains(\"@rust_toolchains//:all\")",
        "bazel_dep(name = \"gazelle_rust\", version = \"0.1.0\")",
        "bazel_dep(name = \"protobuf\", version = \"33.4\")",
    ] {
        assert!(
            fixture_module.contains(required),
            "compatibility MODULE.bazel is missing `{required}`"
        );
    }
    assert!(
        !fixture_module.contains("local_path_override")
            && !fixture_module.contains("archive_override")
            && !fixture_module.contains("patch"),
        "compatibility fixture must use unmodified upstream modules without overrides or patches"
    );
}

#[test]
fn exact_bazel_version_analyzes_and_runs_the_upstream_compatibility_fixture() {
    let version = Command::new(bazel_binary())
        .arg("--version")
        .output()
        .expect("run Bazel --version");
    assert!(
        version.status.success(),
        "Bazel --version failed: {}",
        String::from_utf8_lossy(&version.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&version.stdout).trim(),
        "bazel 9.2.0",
        "U1 must run the exact pinned Bazel version"
    );

    run_bazel(&[
        "test",
        "--noshow_progress",
        "//tests/fixtures/bazel/compat:fixture_test",
    ]);
    run_bazel(&[
        "build",
        "--noshow_progress",
        "//tests/fixtures/bazel/compat/proto:compat_proto",
    ]);
    assert_prebuilt_proto_action_graph();
    run_bazel(&[
        "run",
        "--noshow_progress",
        "//tests/fixtures/bazel/compat:gazelle",
        "--",
        "-mode=diff",
        "-index=lazy",
        "tests/fixtures/bazel/compat",
    ]);

    let generated_build = read_repo_file("tests/fixtures/bazel/compat/BUILD.bazel");
    assert!(
        generated_build.contains("rust_library("),
        "Gazelle must own ordinary first-party Rust target generation"
    );
    assert!(
        generated_build.contains("explicit_exception"),
        "Gazelle must preserve the checked-in exceptional target"
    );
}

#[test]
fn compatibility_fixture_declares_the_third_party_and_exception_boundaries() {
    let build = read_repo_file("tests/fixtures/bazel/compat/BUILD.bazel");
    assert!(
        build.contains("@gazelle_rust//rust_language"),
        "ordinary targets must use the upstream gazelle_rust binary"
    );
    assert!(
        build.contains("rust_crates_prefix @crates//:")
            && build.contains("rust_cargo_lockfile Cargo.lock"),
        "Gazelle must resolve third-party crates through crate_universe"
    );

    let generated_build = read_repo_file("tests/fixtures/bazel/compat/BUILD.bazel");
    assert!(
        generated_build.contains("@crates//:itoa"),
        "the compatibility graph must contain a crate_universe dependency"
    );
    assert!(
        generated_build.contains("# keep"),
        "the explicit exception must carry a Gazelle keep marker"
    );
}

#[test]
fn fixture_shell_adapter_is_checked_in_and_does_not_use_host_path() {
    let shell = read_repo_file("tests/fixtures/bazel/compat/tools/bazel-shell");
    for tool in ["tools/bin/grep", "tools/bin/cat"] {
        assert!(
            read_repo_file(&format!("tests/fixtures/bazel/compat/{tool}"))
                .starts_with("#!/bin/bash"),
            "the fixture must provide the upstream authenticity command {tool}"
        );
    }
    assert!(
        shell.contains("exec /bin/bash \"$@\""),
        "the fixture shell adapter must delegate ordinary shell execution to Bash"
    );
    assert!(
        shell.contains("TOOL_DIR=") && shell.contains("/run/current-system/sw/bin:/bin:/usr/bin"),
        "the fixture shell adapter must use a fixed tool root"
    );
    assert!(
        !shell.contains("$PATH") && !shell.contains("env |") && !shell.contains("export -f"),
        "the fixture shell adapter must not import host PATH fragments"
    );
}

#[test]
fn compatibility_fixture_rejects_repository_owned_generators_and_overlays() {
    let root_module = read_repo_file("MODULE.bazel");
    for forbidden in [
        "local_path_override",
        "archive_override",
        "git_override",
        "single_version_override",
        "multiple_version_override",
        "patch",
        "overlay",
        "postprocessor",
        "custom_generator",
    ] {
        assert!(
            !root_module.contains(forbidden),
            "root MODULE.bazel must not introduce `{forbidden}`"
        );
    }

    let build = read_repo_file("tests/fixtures/bazel/compat/BUILD.bazel");
    assert!(
        build.contains("gazelle_binary(") && build.contains("gazelle("),
        "the fixture must use upstream Gazelle targets"
    );
    for forbidden in ["genrule(", "custom_gazelle", "postprocess"] {
        assert!(
            !build.contains(forbidden),
            "the fixture must not add a repository-owned generator `{forbidden}`"
        );
    }
}

#[test]
fn compatibility_metadata_is_valid_json() {
    for relative in [
        "tests/golden/bazel/check-coverage.json",
        "tests/golden/bazel/eligibility.json",
    ] {
        let bytes = std::fs::read(repo_root().join(relative))
            .unwrap_or_else(|error| panic!("read {relative}: {error}"));
        let value: Value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|error| panic!("parse {relative}: {error}"));
        assert!(
            value.get("schemaVersion").and_then(Value::as_u64) == Some(1),
            "{relative} must declare schemaVersion 1"
        );
    }
}
