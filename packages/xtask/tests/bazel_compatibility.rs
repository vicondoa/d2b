#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use serde_json::Value;

const EXPECTED_BAZEL_VERSION: &str = "bazel 9.2.0";

fn repo_root() -> PathBuf {
    let mut candidates = Vec::new();
    if let Some(root) = std::env::var_os("D2B_REPO_ROOT") {
        candidates.push(PathBuf::from(root));
    }
    for variable in ["TEST_SRCDIR", "RUNFILES_DIR"] {
        if let Some(base) = std::env::var_os(variable).map(PathBuf::from) {
            candidates.push(base.clone());
            if let Some(workspace) = std::env::var_os("TEST_WORKSPACE") {
                candidates.push(base.join(workspace));
            }
            candidates.push(base.join("_main"));
        }
    }
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
                return std::fs::canonicalize(&path).unwrap_or(path);
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
    repo_root()
        .parent()
        .map(|path| path.join(".d2b-bazel-compat-output"))
        .unwrap_or_else(|| panic!("repository parent is required for nested Bazel output"))
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
    static BAZEL_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = BAZEL_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .expect("serialize nested Bazel compatibility commands");
    let mut command = Command::new(bazel_binary());
    let tool_path = bazel_test_tool_path();
    let output_root = bazel_output_user_root();
    std::fs::create_dir_all(&output_root).expect("create Bazel compatibility output root");
    command
        .current_dir(repo_root())
        .env_remove("TEST_TMPDIR")
        .env_remove("TEST_SRCDIR")
        .env_remove("RUNFILES_DIR")
        .env_remove("TEST_WORKSPACE")
        .arg(format!("--output_user_root={}", output_root.display()))
        .arg(arguments.first().expect("Bazel command is non-empty"))
        .arg("--lockfile_mode=error")
        .arg("--repo_contents_cache=")
        .arg(format!("--action_env=PATH={tool_path}"))
        .arg(format!(
            "--shell_executable={}",
            std::env::var("BAZEL_SH").expect("BAZEL_SH is set")
        ))
        .arg(format!(
            "--symlink_prefix={}",
            output_root.join("symlinks/bazel-").display()
        ));
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
        root_module.contains("bazel_dep(name = \"protobuf\", version = \"34.0.bcr.1\")"),
        "the root module must resolve the Bazel 9 protobuf graph"
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
        "packages = [ bazel920 pkgs.rustup pkgs.git ]",
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
fn pins_rules_rs_foundation_and_locked_cargo_authority() {
    assert_eq!(read_repo_file(".bazelversion").trim(), "9.2.0");

    let root_module = read_repo_file("MODULE.bazel");
    for required in [
        "module(",
        "bazel_dep(name = \"rules_rs\", version = \"0.0.105\")",
        "bazel_dep(name = \"llvm\", version = \"0.8.9\")",
        "bazel_dep(name = \"rules_nixpkgs_core\", version = \"0.13.0\")",
        "bazel_dep(name = \"aspect_rules_lint\", version = \"2.7.2\")",
        "bazel_dep(name = \"bazel_skylib\", version = \"1.9.2\")",
        "bazel_dep(name = \"bazel_lib\", version = \"3.7.1\")",
        "bazel_dep(name = \"rules_go\", version = \"0.59.0\")",
        "bazel_dep(name = \"protobuf\", version = \"34.0.bcr.1\")",
        "bazel_dep(name = \"rules_shell\", version = \"0.6.1\")",
        "use_extension(\"@rules_rs//rs:rules_rust.bzl\", \"rules_rust\")",
        "use_repo(rules_rust, \"rules_rust\")",
        "toolchains.toolchain(",
        "version = \"1.97.0\"",
        "\"@default_rust_toolchains//...\"",
        "\"@llvm//toolchain:all\"",
        "use_extension(\"@rules_rs//rs:extensions.bzl\", \"crate\")",
        "cargo_lock = \"//:Cargo.lock\"",
        "cargo_toml = \"//:Cargo.toml\"",
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
        root_module.contains("archive_override(")
            && root_module.contains("module_name = \"rules_rs\"")
            && root_module.contains(
                "https://github.com/hermeticbuild/rules_rs/releases/download/v0.0.105/rules_rs-v0.0.105.tar.gz",
            )
            && root_module.contains(
                "sha256-KE4xcf3WAoxT+UjzUNtEUDEuYcnk/2VBd3ytms8f5gE=",
            ),
        "root MODULE.bazel must use the pinned official rules_rs release archive"
    );
    for forbidden in [
        "bazel_dep(name = \"rules_rust\"",
        "crate_universe",
        "bazel_dep(name = \"gazelle\"",
        "bazel_dep(name = \"gazelle_rust\"",
    ] {
        assert!(
            !root_module.contains(forbidden),
            "root MODULE.bazel must not retain `{forbidden}`"
        );
    }
    let lock = read_repo_file("MODULE.bazel.lock");
    assert!(
        lock.contains("\"lockFileVersion\"")
            && lock.contains("@@rules_rs+//rs:extensions.bzl%crate")
            && lock.contains("@@rules_rs+//rs/toolchains:module_extension.bzl%toolchains"),
        "Bzlmod must have a checked-in lock for the rules_rs extensions"
    );
    assert!(
        !lock.contains("bazel/checks/rust/Cargo.lock")
            && !lock.contains("bazel_only_crates")
            && !lock.contains("FILE:@@//.scratch/"),
        "Bzlmod lock must not capture a second Cargo authority or checkout-local output paths"
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
        "bazel_dep(name = \"rules_rs\", version = \"0.0.105\")",
        "use_extension(\"@rules_rs//rs:rules_rust.bzl\", \"rules_rust\")",
        "use_repo(rules_rust, \"rules_rust\")",
        "use_extension(\"@rules_rs//rs/toolchains:module_extension.bzl\", \"toolchains\")",
        "version = \"1.97.0\"",
        "crate.from_cargo(",
        "cargo_lock = \"//:Cargo.lock\"",
        "cargo_toml = \"//:Cargo.toml\"",
        "use_repo(crate, \"crates\")",
        "register_toolchains(",
        "\"@default_rust_toolchains//...\"",
        "\"@llvm//toolchain:all\"",
        "bazel_dep(name = \"llvm\", version = \"0.8.9\")",
        "bazel_dep(name = \"protobuf\", version = \"34.0.bcr.1\")",
    ] {
        assert!(
            fixture_module.contains(required),
            "compatibility MODULE.bazel is missing `{required}`"
        );
    }
    assert!(
        fixture_module.contains("archive_override(")
            && !fixture_module.contains("crate_universe")
            && !fixture_module.contains("gazelle")
            && !fixture_module.contains("rules_go"),
        "compatibility fixture must use the rules_rs facade without Gazelle or crate_universe"
    );
}

#[test]
fn exact_bazel_version_analyzes_and_runs_the_rules_rs_compatibility_fixture() {
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

    let generated_build = read_repo_file("tests/fixtures/bazel/compat/BUILD.bazel");
    assert!(
        generated_build.contains("rust_library("),
        "the compatibility graph must declare an ordinary first-party Rust target"
    );
    assert!(
        generated_build.contains("explicit_exception"),
        "the compatibility graph must preserve the checked-in exceptional target"
    );
    assert!(
        generated_build.contains("@crates//:serde_json") && !generated_build.contains("gazelle"),
        "the compatibility graph must use explicit rules_rs targets and no Gazelle"
    );
}

#[test]
fn compatibility_fixture_declares_the_third_party_and_exception_boundaries() {
    let build = read_repo_file("tests/fixtures/bazel/compat/BUILD.bazel");
    assert!(
        build.contains("load(\"@rules_rust//rust:defs.bzl\""),
        "ordinary targets must use the rules_rs-managed rules_rust facade"
    );
    assert!(
        build.contains("@crates//:serde_json"),
        "explicit targets must resolve third-party crates through rules_rs"
    );
    assert!(
        build.contains("# keep"),
        "the explicit exception must remain checked in"
    );
    assert!(
        !build.contains("gazelle") && !build.contains("crate_universe"),
        "the compatibility graph must not retain Gazelle or crate_universe directives"
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
        build.contains("rust_library(") && build.contains("rust_test("),
        "the fixture must use explicit native Rust targets"
    );
    for forbidden in [
        "gazelle",
        "crate_universe",
        "genrule(",
        "custom_generator",
        "postprocess",
    ] {
        assert!(
            !build.contains(forbidden),
            "the fixture must not retain a generator `{forbidden}`"
        );
    }
}

#[test]
fn compatibility_metadata_is_valid_json() {
    for relative in [
        "tests/golden/bazel/check-coverage.json",
        "tests/golden/bazel/cache-policy.json",
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
