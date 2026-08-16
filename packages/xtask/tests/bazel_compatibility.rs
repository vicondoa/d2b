#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;

const EXPECTED_BAZEL_VERSION: &str = "bazel 9.2.0";

fn repo_root() -> PathBuf {
    let mut candidates = Vec::new();
    for variable in ["D2B_REPO_ROOT", "TEST_SRCDIR", "RUNFILES_DIR"] {
        if let Some(base) = std::env::var_os(variable).map(PathBuf::from) {
            candidates.push(base.clone());
            if let Some(workspace) = std::env::var_os("TEST_WORKSPACE") {
                candidates.push(base.join(workspace));
            }
            candidates.push(base.join("_main"));
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("xtask lives under packages/xtask")
            .to_path_buf(),
    );
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir);
    }
    for candidate in candidates {
        let mut path = candidate;
        if path.is_file() {
            path.pop();
        }
        loop {
            if path.join("Cargo.toml").is_file()
                && path.join("BUILD.bazel").is_file()
                && path.join("flake.nix").is_file()
            {
                return path;
            }
            if !path.pop() {
                break;
            }
        }
    }
    panic!("repository root with Cargo.toml, BUILD.bazel, and flake.nix is not discoverable")
}

fn read_repo_file(relative: &str) -> String {
    std::fs::read_to_string(repo_root().join(relative))
        .unwrap_or_else(|error| panic!("read {relative}: {error}"))
}

fn bazel_binary() -> PathBuf {
    static BAZEL: OnceLock<PathBuf> = OnceLock::new();
    BAZEL.get_or_init(resolve_bazel_binary).clone()
}

fn resolve_bazel_binary() -> PathBuf {
    if let Some(path) = std::env::var_os("D2B_BAZEL_BIN").map(PathBuf::from) {
        assert_exact_bazel(&path, "D2B_BAZEL_BIN");
        return path;
    }

    let ambient = PathBuf::from("bazel");
    if bazel_version(&ambient).as_deref() == Some(EXPECTED_BAZEL_VERSION) {
        return ambient;
    }

    if let Some(path) = nix_bazel_provider() {
        assert_exact_bazel(&path, "the repository Bazel provider");
        return path;
    }

    panic!(
        "Bazel {EXPECTED_BAZEL_VERSION} is required, but the ambient Bazel is unavailable or \
         wrong; run `nix develop .#bazel -c cargo test --manifest-path Cargo.toml \
         -p xtask --test bazel_compatibility` or set D2B_BAZEL_BIN to an exact provider"
    );
}

fn bazel_version(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn assert_exact_bazel(path: &Path, source: &str) {
    let version = bazel_version(path).unwrap_or_else(|| {
        panic!(
            "Bazel provider from {source} could not execute: {}",
            path.display()
        )
    });
    assert_eq!(
        version, EXPECTED_BAZEL_VERSION,
        "Bazel provider from {source} must be exactly {EXPECTED_BAZEL_VERSION}"
    );
}

fn nix_bazel_provider() -> Option<PathBuf> {
    let system = match std::env::consts::ARCH {
        "x86_64" => "x86_64-linux",
        "aarch64" => "aarch64-linux",
        _ => return None,
    };
    let attribute = format!(".#packages.{system}.bazel-9_2_0");
    let output = Command::new("nix")
        .args([
            "build",
            "--no-link",
            "--no-write-lock-file",
            "--print-out-paths",
            &attribute,
        ])
        .current_dir(repo_root())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let output_stdout = String::from_utf8_lossy(&output.stdout);
    let store_path = output_stdout
        .lines()
        .rfind(|line| !line.trim().is_empty())?;
    let bazel = PathBuf::from(store_path.trim()).join("bin/bazel");
    bazel.is_file().then_some(bazel)
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

fn bazel_test_tool_path() -> String {
    static TOOL_PATH: OnceLock<String> = OnceLock::new();
    TOOL_PATH.get_or_init(resolve_bazel_test_tool_path).clone()
}

fn resolve_bazel_test_tool_path() -> String {
    if let Ok(path) = std::env::var("D2B_BAZEL_TEST_PATH") {
        return path;
    }

    let output = Command::new("nix")
        .args([
            "develop",
            "--no-write-lock-file",
            ".#bazel",
            "-c",
            "bash",
            "-c",
            r#"printf '%s\n' "$D2B_BAZEL_TEST_PATH""#,
        ])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|error| panic!("resolve the Bazel compatibility tool path: {error}"));
    assert!(
        output.status.success(),
        "resolve the Bazel compatibility tool path from `nix develop .#bazel`: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .rfind(|line| line.starts_with("/nix/store/"))
        .unwrap_or_else(|| {
            panic!(
                "`nix develop .#bazel` did not emit D2B_BAZEL_TEST_PATH:\n{}",
                String::from_utf8_lossy(&output.stdout)
            )
        })
        .to_owned()
}

fn run_bazel_output(arguments: &[&str]) -> std::process::Output {
    let mut command = Command::new(bazel_binary());
    let tool_path = bazel_test_tool_path();
    command
        .current_dir(repo_root())
        .arg(format!(
            "--output_user_root={}",
            bazel_output_user_root().display()
        ))
        .arg(arguments.first().expect("Bazel command is non-empty"))
        .arg("--lockfile_mode=error")
        .arg("--repo_contents_cache=")
        .arg("--symlink_prefix=bazel-");
    if arguments.first() == Some(&"test") {
        command.arg(format!("--test_env=PATH={tool_path}"));
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
fn nix_surface_pins_the_official_bazel_9_2_0_provider_for_supported_systems() {
    let package = read_repo_file("pkgs/bazel-9.2.0/default.nix");
    for required in [
        "version = \"9.2.0\"",
        "x86_64-linux",
        "aarch64-linux",
        "https://github.com/bazelbuild/bazel/releases/download/9.2.0/bazel-9.2.0-linux-x86_64",
        "https://github.com/bazelbuild/bazel/releases/download/9.2.0/bazel-9.2.0-linux-arm64",
        "7668a95db1250f12c40407251e4e203b4ec8bf39bc495d2f485b2d8c99048694",
        "049dd21f40ad979db11c3ee68c96a42ce75f1185e69ac61ab20de1501427a410",
        "dontUnpack = true",
        "dontStrip = true",
        "dontPatchELF = true",
        "fetchurl",
    ] {
        assert!(
            package.contains(required),
            "the Bazel provider is missing `{required}`"
        );
    }
    for forbidden in [
        "toolchains_protoc",
        "rules_proto",
        "applyPatches",
        "overrideAttrs",
        "patches =",
    ] {
        assert!(
            !package.contains(forbidden),
            "the Bazel provider must not use `{forbidden}`"
        );
    }

    let flake = read_repo_file("flake.nix");
    for required in [
        "bazel920For = system:",
        "import ./pkgs/bazel-9.2.0",
        "bazel-9_2_0 = bazel920",
        "bazelActionShell = pkgs.buildFHSEnv",
        "executableName = \"bash\"",
        "targetPkgs =",
        "bash",
        "coreutils",
        "gnugrep",
        "runScript = \"${pkgs.bash}/bin/bash\"",
        "packages = [ bazel920 pkgs.rustup ]",
        "bazel = pkgs.mkShellNoCC",
        "export D2B_BAZEL_BIN=\"${bazel920}/bin/bazel\"",
        "export BAZEL_SH=\"${bazelActionShell}/bin/bash\"",
        "export D2B_BAZEL_TEST_PATH=",
        "bazel-9_2_0-provider-smoke",
    ] {
        assert!(
            flake.contains(required),
            "flake.nix is missing Bazel provider wiring `{required}`"
        );
    }
    assert!(
        !flake.contains("/run/current-system/sw/bin")
            && !flake.contains("--test_env=PATH=$PATH")
            && !flake.contains("/nix/store/"),
        "Bazel compatibility wiring must not capture host PATH or machine paths"
    );
    assert!(
        !flake.contains("applyPatches")
            && !flake.contains("patches =")
            && !flake.contains("overrideAttrs")
            && !flake.contains("local_path_override"),
        "the Bazel action shell must wrap the unmodified upstream provider"
    );
    for forbidden in [
        "bazelFhs",
        "executableName = \"bazel\"",
        "runScript = \"${bazel920}/bin/bazel\"",
    ] {
        assert!(
            !flake.contains(forbidden),
            "the Bazel server must not be wrapped by `{forbidden}`"
        );
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
        EXPECTED_BAZEL_VERSION,
        "U1 must analyze the fixture with the exact pinned Bazel version"
    );

    assert_prebuilt_proto_action_graph();
}

#[test]
fn bazel_9_2_advertises_stable_credential_helper_for_remote_surfaces() {
    let output = Command::new(bazel_binary())
        .args(["help", "build"])
        .output()
        .expect("run Bazel credential-helper help");
    assert!(
        output.status.success(),
        "Bazel credential-helper help failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let help = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        help.contains("--credential_helper"),
        "Bazel 9.2 must expose the stable credential-helper option"
    );
    for surface in ["--remote_cache", "--remote_executor", "--bes_backend"] {
        assert!(
            help.contains(surface),
            "Bazel 9.2 build options must expose the {surface} remote surface"
        );
    }
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
        "versions = [\"1.97.0\"]",
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
        EXPECTED_BAZEL_VERSION,
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
    run_bazel(&[
        "run",
        "--noshow_progress",
        "//:gazelle",
        "--",
        "-mode=diff",
        "-index=lazy",
        "packages",
        "tests/fixtures/bazel/gazelle",
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
        generated_build.contains("@crates//:serde_json"),
        "the compatibility graph must contain a crate_universe dependency"
    );
    assert!(
        generated_build.contains("# keep"),
        "the explicit exception must carry a Gazelle keep marker"
    );
}

#[test]
fn compatibility_uses_standard_bazel_sh_without_repository_shell_adapter() {
    let flake = read_repo_file("flake.nix");
    assert!(
        flake.contains("export BAZEL_SH=\"${bazelActionShell}/bin/bash\""),
        "the Bazel shell must be supplied through standard BAZEL_SH"
    );
    assert!(
        !read_repo_file("tests/fixtures/bazel/compat/BUILD.bazel").contains("bazel-shell"),
        "the compatibility fixture must not own a repository shell adapter"
    );
    assert!(
        !repo_root()
            .join("tests/fixtures/bazel/compat/tools/bazel-shell")
            .exists(),
        "the compatibility fixture shell adapter must not remain checked in"
    );
}

#[test]
fn raw_bazel_reuses_server_and_executes_the_declared_action_shell() {
    let shell = std::env::var_os("BAZEL_SH")
        .map(PathBuf::from)
        .unwrap_or_else(|| panic!("BAZEL_SH must be set by `nix develop .#bazel`"));
    assert!(
        shell.ends_with("bin/bash"),
        "BAZEL_SH must point at the standard FHS bash executable: {}",
        shell.display()
    );

    let first = run_bazel_output(&["info", "--noshow_progress", "server_pid"]);
    assert!(
        first.status.success(),
        "Bazel server_pid probe failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&first.stdout),
        String::from_utf8_lossy(&first.stderr)
    );
    let first_pid = String::from_utf8_lossy(&first.stdout).trim().to_owned();
    assert!(
        first_pid
            .chars()
            .all(|character| character.is_ascii_digit()),
        "Bazel server_pid must be numeric, got `{first_pid}`"
    );

    run_bazel(&[
        "build",
        "--noshow_progress",
        "//tests/fixtures/gas-city/buildbuddy:round_trip_payload",
    ]);

    let action = run_bazel_output(&[
        "aquery",
        "--noshow_progress",
        "--output=textproto",
        "--include_commandline",
        r#"mnemonic("Genrule", //tests/fixtures/gas-city/buildbuddy:round_trip_payload)"#,
    ]);
    assert!(
        action.status.success(),
        "Bazel shell action graph probe failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&action.stdout),
        String::from_utf8_lossy(&action.stderr)
    );
    let action_graph = String::from_utf8_lossy(&action.stdout);
    assert!(
        action_graph.contains(&format!("arguments: \"{}\"", shell.display())),
        "the Genrule action must invoke the declared BAZEL_SH executable"
    );

    let second = run_bazel_output(&["info", "--noshow_progress", "server_pid"]);
    assert!(
        second.status.success(),
        "second Bazel server_pid probe failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&second.stdout),
        String::from_utf8_lossy(&second.stderr)
    );
    let second_pid = String::from_utf8_lossy(&second.stdout).trim().to_owned();
    assert_eq!(
        first_pid, second_pid,
        "Bazel must reuse one server across the shell-action probe"
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
