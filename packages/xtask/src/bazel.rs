use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs,
    io::Write,
    os::fd::AsFd,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::hermeticity;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GenBazelMode {
    Write,
    Install,
    Check,
}

const HUBS: &[(&str, &str, &str)] = &[
    ("product", "packages/Cargo.toml", "packages/Cargo.lock"),
    (
        "walker",
        "tests/tools/no-bash-ast-walker/Cargo.toml",
        "tests/tools/no-bash-ast-walker/Cargo.lock",
    ),
];

const STABLE_TOOLCHAIN: &str = "1.97.0";
const NIGHTLY_TOOLCHAIN: &str = "nightly-2026-02-16";
const GENERATOR_METADATA_TARGET: &str = "x86_64-unknown-linux-gnu";
const PREVIEW_ROOT: &str = ".scratch/bazel/generated-preview";
const OUTPUT_MANIFEST_PATH: &str = "bazel/generated/output-manifest.json";
const NATIVE_POLICY_CHECK_MANIFEST: &str =
    include_str!("../../../tests/golden/native-policy-check-manifest.json");
const BAZEL_EXECUTABLE_DIAGNOSTIC: &str = "\
D2B-BZL-EXECUTABLE: the repository-pinned Bazel executable is unavailable.
From the repository root, run: nix develop
Then run from packages/: cargo xtask gen-bazel --check
If the command still fails, verify the pinned Bazel tool in the development shell.";
const MAX_SELECTOR_DIAGNOSTIC_BYTES: usize = 64;

const APPROVED_OUTPUT_PATHS: &[&str] = &[
    ".bazelignore",
    "bazel/generated/BUILD.bazel",
    "bazel/generated/action-network-policy.json",
    "bazel/generated/configured-targets.json",
    "bazel/generated/evidence-sink-policy.json",
    "bazel/generated/no-shell-inventory.json",
    OUTPUT_MANIFEST_PATH,
    "bazel/generated/package-policy-targets.bzl",
    "bazel/generated/product-targets.bzl",
    "bazel/generated/source-census.json",
];

const APPROVED_GENERATED_EXPORTS: &[&str] = &[
    "action-network-policy.json",
    "configured-targets.json",
    "evidence-sink-policy.json",
    "no-shell-inventory.json",
    "output-manifest.json",
    "package-policy-targets.bzl",
    "product-targets.bzl",
    "source-census.json",
];

const RETIRED_HUB_MESSAGES: &[(&str, &str)] = &[
    ("main", RETIRED_HUB_MESSAGE),
    ("broker", RETIRED_HUB_MESSAGE),
    ("guest", RETIRED_HUB_MESSAGE),
];

const RETIRED_HUB_MESSAGE: &str = "\
D2B-BZL-RETIRED-HUB: the requested Bazel dependency hub is retired.
From the repository root, run: nix develop
Then run from packages/: cargo xtask bazel-repin --hub product.";

fn bounded_redacted_selector(value: &str) -> String {
    if value.is_empty() {
        return "<empty>".to_owned();
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return "<redacted>".to_owned();
    }
    if value.len() <= MAX_SELECTOR_DIAGNOSTIC_BYTES {
        return value.to_owned();
    }
    format!("{}...[truncated]", &value[..MAX_SELECTOR_DIAGNOSTIC_BYTES])
}

fn hub_selection_error(hub: &str, retired: bool) -> String {
    let supplied = bounded_redacted_selector(hub);
    if retired {
        format!("{RETIRED_HUB_MESSAGE}\nSupplied hub: {supplied}")
    } else {
        format!(
            "D2B-BZL-INVALID-HUB: supplied hub {supplied}; select one of the repository's product or walker hubs."
        )
    }
}

#[cfg(test)]
#[allow(dead_code)]
pub fn retired_hub_remediation(hub: &str) -> Option<(&'static str, Vec<String>, &'static str)> {
    RETIRED_HUB_MESSAGES
        .iter()
        .find(|(name, _)| *name == hub)
        .map(|(_, message)| {
            (
                *message,
                vec![
                    "cargo".to_owned(),
                    "xtask".to_owned(),
                    "bazel-repin".to_owned(),
                    "--hub".to_owned(),
                    "product".to_owned(),
                ],
                "packages/",
            )
        })
}

#[cfg(test)]
#[allow(dead_code)]
pub fn validate_retired_hub_remediation(argv: &[String], cwd: &str) -> Result<(), Box<dyn Error>> {
    if cwd != "packages/"
        || argv
            .first()
            .is_some_and(|argument| argument == "cd" || argument.starts_with("packages/"))
        || argv
            != [
                "cargo".to_owned(),
                "xtask".to_owned(),
                "bazel-repin".to_owned(),
                "--hub".to_owned(),
                "product".to_owned(),
            ]
    {
        return Err(
            "retired hub remediation must use the repository-relative packages/ cwd".into(),
        );
    }
    Ok(())
}

pub trait BazelExecutor {
    fn run(
        &mut self,
        root: &Path,
        startup_args: &[String],
        command_args: &[String],
        environment: &[(&str, &str)],
    ) -> Result<std::process::ExitStatus, Box<dyn Error>>;

    fn diagnostic(&self) -> Option<&str> {
        None
    }
}

pub fn adr0054_drift_message(code: &str) -> Option<&'static str> {
    Some(match code {
        "D2B-CARGODRIFT-PRODUCT" => "\
D2B-CARGODRIFT-PRODUCT: packages/Cargo.lock is stale.
From the repository root, run: nix develop
Then run: cd packages
cargo generate-lockfile --offline
Review and commit packages/Cargo.lock.
Rerun cargo generate-lockfile --offline; run cargo xtask bazel-repin --hub product and review and commit bazel/cargo/product.lock; run cargo xtask bazel-module-refresh and review and commit MODULE.bazel.lock; then rerun the failed command.",
        "D2B-CARGODRIFT-WALKER" => "\
D2B-CARGODRIFT-WALKER: walker Cargo.lock is stale.
From the repository root, run: nix develop
Then run: cd packages
cargo generate-lockfile --offline --manifest-path ../tests/tools/no-bash-ast-walker/Cargo.toml
Review and commit tests/tools/no-bash-ast-walker/Cargo.lock.
Rerun the walker cargo generate-lockfile command; run cargo xtask bazel-repin --hub walker and review and commit bazel/cargo/walker.lock; run cargo xtask bazel-module-refresh and review and commit MODULE.bazel.lock; then rerun the failed command.",
        "D2B-BZLDRIFT-PRODUCT-HUB" => "\
D2B-BZLDRIFT-PRODUCT-HUB: bazel/cargo/product.lock is stale.
From the repository root, run: nix develop
Then run: cd packages
cargo xtask bazel-repin --hub product
Review and commit bazel/cargo/product.lock.
Rerun cargo xtask bazel-repin --hub product, then rerun the failed command.",
        "D2B-BZLDRIFT-WALKER-HUB" => "\
D2B-BZLDRIFT-WALKER-HUB: bazel/cargo/walker.lock is stale.
From the repository root, run: nix develop
Then run: cd packages
cargo xtask bazel-repin --hub walker
Review and commit bazel/cargo/walker.lock.
Rerun cargo xtask bazel-repin --hub walker, then rerun the failed command.",
        "D2B-BZLDRIFT-MODULE" => "\
D2B-BZLDRIFT-MODULE: MODULE.bazel.lock is stale.
From the repository root, run: nix develop
Then run: cd packages
cargo xtask bazel-module-refresh
Review and commit MODULE.bazel.lock.
Rerun cargo xtask bazel-module-refresh, then rerun the failed command.",
        "D2B-BZLDRIFT-GENERATOR" => "\
D2B-BZLDRIFT-GENERATOR: generated Bazel output is stale.
From the repository root, run: nix develop
Then run: cd packages
Review the scratch preview, then run cargo xtask gen-bazel --install.
Run git status --short --untracked-files=all and review and commit only the generator-owned paths listed in bazel/generated/output-manifest.json.
Rerun cargo xtask gen-bazel --check, then rerun the failed command.",
        "D2B-BZLDRIFT-PACKAGE-POLICY" => "\
D2B-BZLDRIFT-PACKAGE-POLICY: package-policy output is stale.
From the repository root, run: nix develop
Then run: cd packages
Review the scratch preview, then run cargo xtask gen-package-policy-inputs --install.
Run git status --short --untracked-files=all and review and commit only changes below packages/policy-inputs/.
Rerun cargo xtask gen-package-policy-inputs --check, then rerun the failed command.",
        "D2B-BZL-METADATA" => "\
D2B-BZL-METADATA: pinned offline Cargo metadata could not be generated.
From the repository root, run: nix develop
Then run: cd packages
cargo xtask gen-bazel
Review the scratch preview and rerun the failed command.",
        "D2B-BZLDRIFT-YANKED" => "\
D2B-BZLDRIFT-YANKED: yanked snapshot is stale.
From the repository root, run: nix develop
Then run: cd packages
cargo xtask bazel-yanked-refresh
Review and commit bazel/supply_chain/yanked-snapshot.json.
Rerun cargo xtask bazel-yanked-check, then rerun the failed command.",
        "D2B-BZL-AMBIENT-REPIN" => "\
D2B-BZL-AMBIENT-REPIN: a repin control is present.
From the repository root, run: nix develop
Then run: cd packages
unset CARGO_BAZEL_REPIN REPIN CARGO_BAZEL_REPIN_ONLY
Review the requested contributor command and its selected hub; no file is changed by this refusal.
Rerun the exact refused command from the closed contributor-command set.",
        "D2B-BZL-UNEXPECTED-MUTATION" => "\
D2B-BZL-UNEXPECTED-MUTATION: a mutation changed an unapproved tracked path.
From the repository root, run: nix develop
Then run: cd packages
git status --short --untracked-files=all
Review every listed repository-relative path; commit the intended generated change or remove the unintended change.
Rerun the exact refused command from the closed contributor-command set.",
        _ => return None,
    })
}

struct ProcessExecutor {
    diagnostic: Option<String>,
}

const EXPLICIT_BAZEL_ENVIRONMENT: &[&str] = &[
    "PATH",
    "HOME",
    "USER",
    "LOGNAME",
    "TMPDIR",
    "TMP",
    "TEMP",
    "LANG",
    "LC_ALL",
    "TZ",
    "JAVA_HOME",
];

impl BazelExecutor for ProcessExecutor {
    fn run(
        &mut self,
        root: &Path,
        startup_args: &[String],
        command_args: &[String],
        environment: &[(&str, &str)],
    ) -> Result<std::process::ExitStatus, Box<dyn Error>> {
        let executable = bazel_executable()?;
        let mut command = Command::new(executable);
        command
            .current_dir(root)
            .args(startup_args)
            .args(command_args)
            .env_clear()
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for name in EXPLICIT_BAZEL_ENVIRONMENT {
            if let Some(value) = env::var_os(name) {
                command.env(name, value);
            }
        }
        for (name, value) in environment {
            command.env(name, value);
        }
        let output = command
            .output()
            .map_err(|_| -> Box<dyn Error> { BAZEL_EXECUTABLE_DIAGNOSTIC.into() })?;
        self.diagnostic = Some(bounded_child_diagnostic(&output.stderr));
        Ok(output.status)
    }

    fn diagnostic(&self) -> Option<&str> {
        self.diagnostic.as_deref()
    }
}

const MAX_CHILD_DIAGNOSTIC_BYTES: usize = 768;

pub(crate) fn bounded_child_diagnostic(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut rendered = String::new();
    for token in text.split_whitespace() {
        let token = if token.contains('/') || token.contains('\\') {
            "<path>"
        } else {
            token
        };
        if !rendered.is_empty() {
            rendered.push(' ');
        }
        rendered.push_str(token);
        if rendered.len() >= MAX_CHILD_DIAGNOSTIC_BYTES {
            let mut boundary = MAX_CHILD_DIAGNOSTIC_BYTES;
            while !rendered.is_char_boundary(boundary) {
                boundary -= 1;
            }
            rendered.truncate(boundary);
            rendered.push_str("...[truncated]");
            break;
        }
    }
    if rendered.is_empty() {
        "<no child diagnostic>".to_owned()
    } else {
        rendered
    }
}

fn command_label(command_args: &[String]) -> String {
    if command_args.is_empty() {
        "bazel <unknown>".to_owned()
    } else {
        format!("bazel {}", command_args.join(" "))
    }
}

fn command_failure_message(
    hub: &str,
    command_args: &[String],
    status: &str,
    diagnostic: Option<&str>,
) -> String {
    let hub = bounded_redacted_selector(hub);
    let diagnostic =
        bounded_child_diagnostic(diagnostic.unwrap_or("<no child diagnostic>").as_bytes());
    format!(
        "D2B-BZL-COMMAND: hub={hub} command={} status={status} diagnostic={}",
        command_label(command_args),
        diagnostic
    )
}

pub(crate) fn parse_gen_bazel(args: &[String]) -> Result<GenBazelMode, Box<dyn Error>> {
    match args {
        [] => Ok(GenBazelMode::Write),
        [flag] if flag == "--install" => Ok(GenBazelMode::Install),
        [flag] if flag == "--check" => Ok(GenBazelMode::Check),
        _ => Err("usage: gen-bazel [--check|--install]".into()),
    }
}

pub(crate) fn gen_bazel(args: &[String]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mode = parse_gen_bazel(args)?;
    let root = repo_root()?;
    let model = generate_model(&root)?;
    model.validate()?;
    let rendered = model.render()?;
    validate_rendered_outputs(&rendered)?;
    let paths = rendered.keys().map(PathBuf::from).collect::<Vec<_>>();

    match mode {
        GenBazelMode::Write => write_bazel_preview(&root, &rendered),
        GenBazelMode::Install => {
            install_bazel_outputs(&root, &rendered)?;
            Ok(paths)
        }
        GenBazelMode::Check => {
            validate_committed_output_census(&root, &rendered)?;
            for (relative, expected) in rendered {
                let path = root.join(&relative);
                let actual = fs::read_to_string(&path).map_err(|_| {
                    format!(
                        "{}\nStale path: {relative}",
                        adr0054_drift_message("D2B-BZLDRIFT-GENERATOR")
                            .expect("generator diagnostic is closed")
                    )
                })?;
                if actual != expected {
                    return Err(format!(
                        "{}\nStale path: {relative}",
                        adr0054_drift_message("D2B-BZLDRIFT-GENERATOR")
                            .expect("generator diagnostic is closed")
                    )
                    .into());
                }
            }
            Ok(paths)
        }
    }
}

fn write_bazel_preview(
    root: &Path,
    rendered: &BTreeMap<String, String>,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let preview_root = root.join(PREVIEW_ROOT);
    if let Ok(metadata) = fs::symlink_metadata(&preview_root) {
        if !metadata.file_type().is_dir() {
            return Err("D2B-BZLDRIFT-GENERATOR: Bazel preview root is not a directory.".into());
        }
        remove_anchored_directory(&preview_root)?;
    }
    let _ = ensure_bazel_directory(&preview_root)?;
    for (relative, contents) in rendered {
        let path = preview_root.join(relative);
        atomic_write_file(&path, contents)?;
    }
    validate_committed_output_census(&preview_root, rendered)?;
    Ok(rendered
        .keys()
        .map(|relative| PathBuf::from(PREVIEW_ROOT).join(relative))
        .collect())
}

fn install_bazel_outputs(
    root: &Path,
    rendered: &BTreeMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    let generated_root = root.join("bazel/generated");
    let generated_fd = ensure_bazel_directory(&generated_root)?;
    if let Ok(metadata) = fs::symlink_metadata(&generated_root) {
        if !metadata.file_type().is_dir() {
            return Err(
                "D2B-BZLDRIFT-GENERATOR: bazel/generated is not a regular directory.".into(),
            );
        }
        for entry in fs::read_dir(&generated_root)
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not inspect the Bazel output directory.")?
        {
            let entry = entry.map_err(
                |_| "D2B-BZLDRIFT-GENERATOR: could not inspect the Bazel output directory.",
            )?;
            if !entry
                .file_type()
                .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not inspect a Bazel output entry.")?
                .is_file()
            {
                return Err("D2B-BZLDRIFT-GENERATOR: a Bazel output is not a regular file.".into());
            }
        }
    }

    let bazelignore = root.join(".bazelignore");
    if let Ok(metadata) = fs::symlink_metadata(&bazelignore)
        && !metadata.file_type().is_file()
    {
        return Err("D2B-BZLDRIFT-GENERATOR: .bazelignore is not a regular file.".into());
    }

    for (relative, contents) in rendered {
        atomic_write_file(&root.join(relative), contents)?;
    }

    let expected = rendered.keys().cloned().collect::<BTreeSet<_>>();
    for entry in fs::read_dir(&generated_root)
        .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not inspect the Bazel output directory.")?
    {
        let entry = entry
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not inspect the Bazel output directory.")?;
        let relative = format!(
            "bazel/generated/{}",
            entry
                .file_name()
                .to_str()
                .ok_or("D2B-BZLDRIFT-GENERATOR: Bazel output path is not UTF-8.")?
        );
        if !expected.contains(&relative) {
            let name = entry
                .file_name()
                .to_str()
                .ok_or("D2B-BZLDRIFT-GENERATOR: Bazel output path is not UTF-8.")?
                .to_owned();
            let stat = rustix::fs::statat(
                generated_fd.as_fd(),
                name.as_str(),
                rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
            )
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not inspect a stale Bazel output.")?;
            if rustix::fs::FileType::from_raw_mode(stat.st_mode) == rustix::fs::FileType::Symlink {
                return Err(
                    "D2B-BZLDRIFT-GENERATOR: refusing a symlinked stale Bazel output.".into(),
                );
            }
            rustix::fs::unlinkat(
                generated_fd.as_fd(),
                name.as_str(),
                rustix::fs::AtFlags::empty(),
            )
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not remove a stale Bazel output.")?;
        }
    }
    rustix::fs::fsync(generated_fd.as_fd())
        .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not sync Bazel outputs.")?;
    validate_committed_output_census(root, rendered)
}

fn atomic_write_file(path: &Path, contents: &str) -> Result<(), Box<dyn Error>> {
    let parent = path
        .parent()
        .ok_or("D2B-BZLDRIFT-GENERATOR: generated output has no parent directory.")?;
    let parent_fd = ensure_bazel_directory(parent)?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("D2B-BZLDRIFT-GENERATOR: generated output path is not UTF-8.")?;
    if let Ok(stat) = rustix::fs::statat(
        parent_fd.as_fd(),
        file_name,
        rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
    ) && rustix::fs::FileType::from_raw_mode(stat.st_mode) == rustix::fs::FileType::Symlink
    {
        return Err("D2B-BZLDRIFT-GENERATOR: generated output is a symlink.".into());
    }
    for attempt in 0..100_u32 {
        let temporary = format!(".{file_name}.d2b-install-{}-{attempt}", std::process::id());
        let descriptor = match rustix::fs::openat(
            parent_fd.as_fd(),
            temporary.as_str(),
            rustix::fs::OFlags::WRONLY
                | rustix::fs::OFlags::CREATE
                | rustix::fs::OFlags::EXCL
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::from_bits_truncate(0o600),
        ) {
            Ok(descriptor) => descriptor,
            Err(rustix::io::Errno::EXIST) => continue,
            Err(_) => {
                return Err("D2B-BZLDRIFT-GENERATOR: could not create an atomic output.".into());
            }
        };
        let mut file = fs::File::from(descriptor);
        if file.write_all(contents.as_bytes()).is_err() || file.sync_all().is_err() {
            let _ = rustix::fs::unlinkat(
                parent_fd.as_fd(),
                temporary.as_str(),
                rustix::fs::AtFlags::empty(),
            );
            return Err("D2B-BZLDRIFT-GENERATOR: could not write an atomic output.".into());
        }
        drop(file);
        if rustix::fs::renameat(
            parent_fd.as_fd(),
            temporary.as_str(),
            parent_fd.as_fd(),
            file_name,
        )
        .is_err()
        {
            let _ = rustix::fs::unlinkat(
                parent_fd.as_fd(),
                temporary.as_str(),
                rustix::fs::AtFlags::empty(),
            );
            return Err("D2B-BZLDRIFT-GENERATOR: could not install an output.".into());
        }
        rustix::fs::fsync(parent_fd.as_fd())
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not sync an output directory.")?;
        return Ok(());
    }
    Err("D2B-BZLDRIFT-GENERATOR: could not reserve an atomic output.".into())
}

pub(crate) fn ensure_bazel_directory(path: &Path) -> Result<std::fs::File, Box<dyn Error>> {
    let mut current = std::fs::File::from(
        rustix::fs::open(
            "/",
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        )
        .map_err(|_| "D2B-BZLDRIFT-GENERATOR: filesystem root is not anchored.")?,
    );
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        env::current_dir()
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: current directory is unavailable.")?
            .join(path)
    };
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            if matches!(
                component,
                std::path::Component::RootDir | std::path::Component::Prefix(_)
            ) {
                continue;
            }
            return Err("D2B-BZLDRIFT-GENERATOR: output path contains traversal.".into());
        };
        let name = name
            .to_str()
            .ok_or("D2B-BZLDRIFT-GENERATOR: output path is not UTF-8.")?;
        let child = match rustix::fs::openat(
            current.as_fd(),
            name,
            rustix::fs::OFlags::RDONLY
                | rustix::fs::OFlags::DIRECTORY
                | rustix::fs::OFlags::NOFOLLOW
                | rustix::fs::OFlags::CLOEXEC,
            rustix::fs::Mode::empty(),
        ) {
            Ok(child) => child,
            Err(rustix::io::Errno::NOENT) => {
                rustix::fs::mkdirat(
                    current.as_fd(),
                    name,
                    rustix::fs::Mode::from_bits_truncate(0o755),
                )
                .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not create an output directory.")?;
                rustix::fs::fsync(current.as_fd())
                    .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not sync an output directory.")?;
                rustix::fs::openat(
                    current.as_fd(),
                    name,
                    rustix::fs::OFlags::RDONLY
                        | rustix::fs::OFlags::DIRECTORY
                        | rustix::fs::OFlags::NOFOLLOW
                        | rustix::fs::OFlags::CLOEXEC,
                    rustix::fs::Mode::empty(),
                )
                .map_err(|_| "D2B-BZLDRIFT-GENERATOR: created output directory changed.")?
            }
            Err(_) => {
                return Err("D2B-BZLDRIFT-GENERATOR: output path is not a safe directory.".into());
            }
        };
        let named =
            rustix::fs::statat(current.as_fd(), name, rustix::fs::AtFlags::SYMLINK_NOFOLLOW)
                .map_err(|_| "D2B-BZLDRIFT-GENERATOR: output directory identity is unavailable.")?;
        let pinned = rustix::fs::fstat(&child)
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: output directory cannot be statted.")?;
        if named.st_dev != pinned.st_dev || named.st_ino != pinned.st_ino {
            return Err("D2B-BZLDRIFT-GENERATOR: output directory changed identity.".into());
        }
        current = std::fs::File::from(child);
    }
    Ok(current)
}

fn reject_symlink_components(path: &Path) -> Result<(), Box<dyn Error>> {
    let mut current = if path.is_absolute() {
        PathBuf::from("/")
    } else {
        env::current_dir()
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: current directory is unavailable.")?
    };
    for component in path.components() {
        let std::path::Component::Normal(name) = component else {
            continue;
        };
        current.push(name);
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("D2B-BZLDRIFT-GENERATOR: output path contains a symlink.".into());
            }
            Ok(metadata) if !metadata.file_type().is_dir() => {
                return Err(
                    "D2B-BZLDRIFT-GENERATOR: output path component is not a directory.".into(),
                );
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => {
                return Err("D2B-BZLDRIFT-GENERATOR: output path cannot be inspected.".into());
            }
        }
    }
    Ok(())
}

fn verify_directory_identity(
    path: &Path,
    descriptor: impl std::os::fd::AsFd,
) -> Result<(), Box<dyn Error>> {
    let named = fs::symlink_metadata(path)
        .map_err(|_| "D2B-BZLDRIFT-GENERATOR: output parent changed identity.")?;
    let pinned = rustix::fs::fstat(descriptor.as_fd())
        .map_err(|_| "D2B-BZLDRIFT-GENERATOR: output parent cannot be statted.")?;
    if named.dev() != pinned.st_dev || named.ino() != pinned.st_ino {
        return Err("D2B-BZLDRIFT-GENERATOR: output parent changed identity.".into());
    }
    Ok(())
}

pub(crate) fn remove_anchored_directory(path: &Path) -> Result<(), Box<dyn Error>> {
    let parent = path
        .parent()
        .ok_or("D2B-BZLDRIFT-GENERATOR: anchored directory has no parent.")?;
    reject_symlink_components(parent)?;
    let parent_fd = rustix::fs::openat2(
        rustix::fs::CWD,
        parent,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
        rustix::fs::ResolveFlags::NO_SYMLINKS | rustix::fs::ResolveFlags::NO_MAGICLINKS,
    )
    .map_err(|_| "D2B-BZLDRIFT-GENERATOR: directory parent is not anchored.")?;
    verify_directory_identity(parent, parent_fd.as_fd())?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("D2B-BZLDRIFT-GENERATOR: directory name is not UTF-8.")?;
    let target = rustix::fs::openat(
        parent_fd.as_fd(),
        name,
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|_| "D2B-BZLDRIFT-GENERATOR: directory target is not safely anchored.")?;
    remove_anchored_children(target.as_fd())?;
    rustix::fs::unlinkat(parent_fd.as_fd(), name, rustix::fs::AtFlags::REMOVEDIR)
        .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not remove anchored directory.")?;
    rustix::fs::fsync(parent_fd.as_fd())
        .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not sync removed directory.")?;
    Ok(())
}

fn remove_anchored_children(parent: impl std::os::fd::AsFd) -> Result<(), Box<dyn Error>> {
    let directory = rustix::fs::Dir::read_from(parent.as_fd())
        .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not enumerate anchored directory.")?;
    let names = directory
        .map(|entry| {
            let entry =
                entry.map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not read directory entry.")?;
            entry
                .file_name()
                .to_str()
                .map(str::to_owned)
                .map_err(|_| "D2B-BZLDRIFT-GENERATOR: directory entry is not UTF-8.")
        })
        .filter(|entry| !matches!(entry, Ok(name) if name == "." || name == ".."))
        .collect::<Result<Vec<_>, _>>()?;
    for name in names {
        let stat = rustix::fs::statat(
            parent.as_fd(),
            name.as_str(),
            rustix::fs::AtFlags::SYMLINK_NOFOLLOW,
        )
        .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not inspect directory entry.")?;
        let file_type = rustix::fs::FileType::from_raw_mode(stat.st_mode);
        if file_type == rustix::fs::FileType::Symlink {
            return Err("D2B-BZLDRIFT-GENERATOR: refusing a symlinked directory entry.".into());
        }
        if file_type == rustix::fs::FileType::Directory {
            let child = rustix::fs::openat(
                parent.as_fd(),
                name.as_str(),
                rustix::fs::OFlags::RDONLY
                    | rustix::fs::OFlags::DIRECTORY
                    | rustix::fs::OFlags::NOFOLLOW
                    | rustix::fs::OFlags::CLOEXEC,
                rustix::fs::Mode::empty(),
            )
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: child directory is not anchored.")?;
            remove_anchored_children(child.as_fd())?;
            rustix::fs::unlinkat(
                parent.as_fd(),
                name.as_str(),
                rustix::fs::AtFlags::REMOVEDIR,
            )
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not remove child directory.")?;
        } else {
            rustix::fs::unlinkat(parent.as_fd(), name.as_str(), rustix::fs::AtFlags::empty())
                .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not remove child output.")?;
        }
    }
    Ok(())
}

pub(crate) fn parse_repin(args: &[String]) -> Result<&str, Box<dyn Error>> {
    match args {
        [flag, hub] if flag == "--hub" && HUBS.iter().any(|(name, _, _)| name == hub) => {
            Ok(hub.as_str())
        }
        [flag, hub]
            if flag == "--hub" && RETIRED_HUB_MESSAGES.iter().any(|(name, _)| name == hub) =>
        {
            Err(hub_selection_error(hub, true).into())
        }
        [flag, hub] if flag == "--hub" => Err(hub_selection_error(hub, false).into()),
        _ => Err("usage: bazel-repin --hub <name>".into()),
    }
}

pub fn bazel_repin_with_executor(
    root: &Path,
    hub: &str,
    executor: &mut dyn BazelExecutor,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    if !HUBS.iter().any(|(name, _, _)| *name == hub) {
        if RETIRED_HUB_MESSAGES.iter().any(|(name, _)| *name == hub) {
            return Err(hub_selection_error(hub, true).into());
        }
        return Err(hub_selection_error(hub, false).into());
    }
    reject_ambient_repin("bazel-repin", Some(hub))?;
    let before = mutation_snapshot(root)?;
    let options = startup_options(root);
    let command = options.repin_command_args(!root.join("MODULE.bazel.lock").is_file());
    let status = executor
        .run(
            root,
            &options.startup_args(),
            &command,
            &[("CARGO_BAZEL_REPIN", "1"), ("CARGO_BAZEL_REPIN_ONLY", hub)],
        )
        .map_err(|error| {
            command_failure_message(hub, &command, "not-started", Some(&error.to_string()))
        })?;
    let after = mutation_snapshot(root)?;
    let lock = format!("bazel/cargo/{hub}.lock");
    let outside = changed_outside(&before, &after, Some(&lock));
    if !outside.is_empty() {
        return Err(unexpected_mutation_message(&outside).into());
    }
    if !status.success() {
        return Err(command_failure_message(
            hub,
            &command,
            &status_text(&status),
            executor.diagnostic(),
        )
        .into());
    }
    Ok(if before.get(&lock) != after.get(&lock) {
        vec![PathBuf::from(lock)]
    } else {
        Vec::new()
    })
}

pub(crate) fn bazel_module_refresh(args: &[String]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    if !args.is_empty() {
        return Err("usage: bazel-module-refresh".into());
    }
    reject_ambient_repin("bazel-module-refresh", None)?;
    let root = repo_root()?;
    let mut executor = ProcessExecutor { diagnostic: None };
    bazel_module_refresh_with_executor(&root, &mut executor)
}

pub fn bazel_module_refresh_with_executor(
    root: &Path,
    executor: &mut dyn BazelExecutor,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    reject_ambient_repin("bazel-module-refresh", None)?;
    let before = mutation_snapshot(root)?;
    let options = startup_options(root);
    let command = options.module_refresh_command_args();
    let status = executor
        .run(root, &options.startup_args(), &command, &[])
        .map_err(|error| {
            command_failure_message("module", &command, "not-started", Some(&error.to_string()))
        })?;
    let after = mutation_snapshot(root)?;
    let outside = changed_outside(&before, &after, Some("MODULE.bazel.lock"));
    if !outside.is_empty() {
        return Err(unexpected_mutation_message(&outside).into());
    }
    if !status.success() {
        return Err(command_failure_message(
            "module",
            &command,
            &status_text(&status),
            executor.diagnostic(),
        )
        .into());
    }
    Ok(
        if before.get("MODULE.bazel.lock") != after.get("MODULE.bazel.lock") {
            vec![PathBuf::from("MODULE.bazel.lock")]
        } else {
            Vec::new()
        },
    )
}

fn reject_ambient_repin(command: &str, hub: Option<&str>) -> Result<(), Box<dyn Error>> {
    let present = ["CARGO_BAZEL_REPIN", "REPIN", "CARGO_BAZEL_REPIN_ONLY"]
        .iter()
        .any(|name| env::var_os(name).is_some());
    if !present {
        return Ok(());
    }
    let _ = (command, hub);
    Err(adr0054_drift_message("D2B-BZL-AMBIENT-REPIN")
        .expect("ambient repin diagnostic is closed")
        .into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StartupOptions {
    output_user_root: PathBuf,
    output_base: PathBuf,
}

impl StartupOptions {
    fn startup_args(&self) -> Vec<String> {
        vec![
            format!("--output_user_root={}", self.output_user_root.display()),
            format!("--output_base={}", self.output_base.display()),
        ]
    }

    fn repin_command_args(&self, fresh_tree: bool) -> Vec<String> {
        // Bazel 8.6's `sync` command is WORKSPACE-only; `mod deps` is the
        // Bzlmod extension evaluation that rules_rust uses for this path.
        let mut args = vec!["mod".to_owned(), "deps".to_owned()];
        if fresh_tree {
            args.push("--lockfile_mode=off".to_owned());
        }
        args
    }

    fn module_refresh_command_args(&self) -> Vec<String> {
        vec![
            "mod".to_owned(),
            "deps".to_owned(),
            "--lockfile_mode=update".to_owned(),
        ]
    }
}

fn startup_options(root: &Path) -> StartupOptions {
    let root = absolute_root(root);
    let scratch = root.join(".scratch/bazel");
    StartupOptions {
        output_user_root: scratch.join("output-user-root"),
        output_base: scratch.join("output-base"),
    }
}

pub(crate) fn bazel_executable() -> Result<PathBuf, Box<dyn Error>> {
    let executable = env::var_os("BAZEL").unwrap_or_else(|| "bazel".into());
    let executable = PathBuf::from(executable);
    if executable.is_absolute() {
        return Ok(executable);
    }
    let path = env::var_os("PATH").ok_or(BAZEL_EXECUTABLE_DIAGNOSTIC)?;
    env::split_paths(&path)
        .map(|directory| directory.join(&executable))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| BAZEL_EXECUTABLE_DIAGNOSTIC.into())
}

pub(crate) const fn bazel_executable_diagnostic() -> &'static str {
    BAZEL_EXECUTABLE_DIAGNOSTIC
}

fn absolute_root(root: &Path) -> PathBuf {
    if root.is_absolute() {
        root.to_path_buf()
    } else {
        env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("."))
            .join(root)
    }
}

fn status_text(status: &std::process::ExitStatus) -> String {
    status
        .code()
        .map(|code| code.to_string())
        .unwrap_or_else(|| "signal".to_owned())
}

fn repo_root() -> Result<PathBuf, Box<dyn Error>> {
    if let Some(root) = env::var_os("D2B_BAZEL_WORKTREE") {
        return fs::canonicalize(root).map_err(|_| {
            "D2B-BZL-WORKTREE: D2B_BAZEL_WORKTREE is not a repository directory.".into()
        });
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot locate repository root".into())
}

#[cfg(test)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct Census {
    executed: Vec<String>,
    out_of_census: Vec<(String, String)>,
}

#[cfg(test)]
impl Census {
    fn new(mut executed: Vec<String>, mut out_of_census: Vec<(String, String)>) -> Self {
        executed.sort();
        executed.dedup();
        out_of_census.sort();
        out_of_census.dedup();
        Self {
            executed,
            out_of_census,
        }
    }

    fn json(&self) -> String {
        serde_json::to_string_pretty(&json!({
            "executed": self.executed,
            "outOfCensus": self.out_of_census
                .iter()
                .map(|(entry, reason)| json!({"entry": entry, "reason": reason}))
                .collect::<Vec<_>>(),
        }))
        .expect("census JSON is serializable")
            + "\n"
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GeneratedModel {
    bazelignore: Vec<String>,
    action_network_policy: String,
    configured_targets: String,
    evidence_sink_policy: String,
    no_shell_inventory: String,
    package_policy_targets: String,
    product_targets: String,
    source_census: String,
}

impl GeneratedModel {
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        if self.bazelignore.is_empty() || !self.bazelignore.iter().any(|entry| entry == ".scratch/")
        {
            return Err("generated .bazelignore must be nonempty and cover .scratch/".into());
        }
        hermeticity::validate_action_network_inventory(
            &hermeticity::complete_action_network_inventory(),
        )
        .map_err(|_| "action-network inventory is invalid")?;
        for (name, contents) in [
            ("action-network-policy.json", &self.action_network_policy),
            ("configured-targets.json", &self.configured_targets),
            ("evidence-sink-policy.json", &self.evidence_sink_policy),
            ("no-shell-inventory.json", &self.no_shell_inventory),
            ("source-census.json", &self.source_census),
        ] {
            let value: Value = serde_json::from_str(contents)
                .map_err(|_| format!("generated {name} is not valid JSON"))?;
            if !value.is_object() {
                return Err(format!("generated {name} must be a JSON object").into());
            }
        }
        if self.package_policy_targets.trim().is_empty() || self.product_targets.trim().is_empty() {
            return Err("generated target definitions must be nonempty".into());
        }
        Ok(())
    }

    fn render(&self) -> Result<BTreeMap<String, String>, Box<dyn Error>> {
        let mut outputs = BTreeMap::new();
        insert_output(
            &mut outputs,
            ".bazelignore",
            render_bazelignore(&self.bazelignore),
        )?;
        insert_output(
            &mut outputs,
            "bazel/generated/BUILD.bazel",
            generated_build_file(),
        )?;
        insert_output(
            &mut outputs,
            "bazel/generated/action-network-policy.json",
            self.action_network_policy.clone(),
        )?;
        insert_output(
            &mut outputs,
            "bazel/generated/configured-targets.json",
            self.configured_targets.clone(),
        )?;
        insert_output(
            &mut outputs,
            "bazel/generated/evidence-sink-policy.json",
            self.evidence_sink_policy.clone(),
        )?;
        insert_output(
            &mut outputs,
            "bazel/generated/no-shell-inventory.json",
            self.no_shell_inventory.clone(),
        )?;
        insert_output(
            &mut outputs,
            "bazel/generated/package-policy-targets.bzl",
            self.package_policy_targets.clone(),
        )?;
        insert_output(
            &mut outputs,
            "bazel/generated/product-targets.bzl",
            self.product_targets.clone(),
        )?;
        insert_output(
            &mut outputs,
            "bazel/generated/source-census.json",
            self.source_census.clone(),
        )?;
        let manifest = output_manifest(&outputs);
        insert_output(&mut outputs, OUTPUT_MANIFEST_PATH, manifest)?;
        Ok(outputs)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManifestInfo {
    relative: String,
    package_dir: String,
    package_name: String,
    lib_doctest: Option<bool>,
    lib: Option<TargetInfo>,
    tests: Vec<TargetInfo>,
    bins: Vec<TargetInfo>,
    benches: Vec<TargetInfo>,
    examples: Vec<TargetInfo>,
    default_features: Vec<String>,
    feature_dependencies: BTreeMap<String, Vec<String>>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetInfo {
    name: String,
    path: String,
    kind: String,
    harness: Option<bool>,
    doctest: Option<bool>,
    required_features: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DependencyInfo {
    package_name: String,
    package_dir: String,
    hub: String,
    normal: Vec<String>,
    dev: Vec<String>,
    optional: BTreeSet<String>,
    proc_macro: BTreeSet<String>,
    target_conditions: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetRow {
    label: String,
    package: String,
    manifest: String,
    kind: String,
    cargo_context: String,
    system: Option<String>,
    cargo_target: Option<String>,
    policy_input: Option<String>,
    source_files: Vec<String>,
    direct_first_party_deps: Vec<String>,
    direct_product_deps: Vec<String>,
    cfgs: Vec<String>,
    features: Vec<String>,
    target_name: String,
    target_kind: String,
    target_path: String,
    harness: Option<bool>,
    doctest: Option<bool>,
    required_features: Vec<String>,
    default_features: bool,
    dependency_conditions: BTreeMap<String, String>,
    closure_configured_targets: Vec<String>,
    closure_external_identities: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SpawnSite {
    source: String,
    start_line: usize,
    start_column: usize,
    end_line: usize,
    end_column: usize,
    program_expression: String,
    shell_invocation: bool,
}

type PolicyTargetContext = (
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static str,
    &'static [&'static str],
);

const POLICY_TARGET_CONTEXTS: &[PolicyTargetContext] = &[
    (
        "broker-production",
        "d2b-priv-broker",
        "x86_64-linux",
        "x86_64-unknown-linux-gnu",
        "packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-gnu/broker-production",
        &[],
    ),
    (
        "guest-real-libshpool",
        "d2b-guest-shell-runner",
        "x86_64-linux",
        "x86_64-unknown-linux-musl",
        "packages/policy-inputs/x86_64-linux/x86_64-unknown-linux-musl/guest-real-libshpool",
        &["real-libshpool"],
    ),
    (
        "broker-production",
        "d2b-priv-broker",
        "aarch64-linux",
        "aarch64-unknown-linux-gnu",
        "packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-gnu/broker-production",
        &[],
    ),
    (
        "guest-real-libshpool",
        "d2b-guest-shell-runner",
        "aarch64-linux",
        "aarch64-unknown-linux-musl",
        "packages/policy-inputs/aarch64-linux/aarch64-unknown-linux-musl/guest-real-libshpool",
        &["real-libshpool"],
    ),
];

fn validate_native_policy_check_manifest() -> Result<(), Box<dyn Error>> {
    let manifest: Value = serde_json::from_str(NATIVE_POLICY_CHECK_MANIFEST)
        .map_err(|_| "native policy/check manifest is not valid JSON")?;
    if manifest.get("schemaVersion") != Some(&json!(1)) {
        return Err("native policy/check manifest schema version is unsupported".into());
    }
    let contexts = manifest
        .get("contexts")
        .and_then(Value::as_array)
        .ok_or("native policy/check manifest contexts are missing")?;
    if contexts.len() != POLICY_TARGET_CONTEXTS.len() {
        return Err("native policy/check manifest context census is not exact".into());
    }
    for (record, (name, package, system, target, policy_input, features)) in
        contexts.iter().zip(POLICY_TARGET_CONTEXTS)
    {
        let id = format!("{system}/{target}/{name}");
        let expected_features = features
            .iter()
            .map(|feature| Value::String((*feature).to_owned()))
            .collect::<Vec<_>>();
        if record.get("id").and_then(Value::as_str) != Some(id.as_str())
            || record.get("system").and_then(Value::as_str) != Some(*system)
            || record.get("target").and_then(Value::as_str) != Some(*target)
            || record.get("context").and_then(Value::as_str) != Some(*name)
            || record.get("package").and_then(Value::as_str) != Some(*package)
            || record.get("policyInput").and_then(Value::as_str) != Some(*policy_input)
            || record.get("features") != Some(&Value::Array(expected_features))
            || record.get("defaultFeatures") != Some(&Value::Bool(false))
            || record.get("productionEdgeKinds").and_then(Value::as_str) != Some("normal,build")
            || record.get("policyEdgeKinds").and_then(Value::as_str) != Some("normal,build,dev")
        {
            return Err("native policy/check manifest context differs from xtask".into());
        }
    }
    let checks = manifest
        .get("nativeChecks")
        .and_then(Value::as_array)
        .ok_or("native policy/check manifest checks are missing")?;
    let expected = [
        "broker-production-dependency-policy",
        "guest-shell-runner-static-dependency-policy",
        "broker-production-package-policy",
        "guest-real-libshpool-package-policy",
        "broker-host-artifact-contract",
        "guest-static-elf",
    ];
    let actual = checks
        .iter()
        .map(|check| check.as_str().ok_or("native check name is not a string"))
        .collect::<Result<Vec<_>, _>>()?;
    if actual != expected {
        return Err("native policy/check manifest check census differs from xtask".into());
    }
    Ok(())
}

fn ordered_object(fields: impl IntoIterator<Item = (String, Value)>) -> Value {
    let fields = fields.into_iter().collect::<BTreeMap<_, _>>();
    Value::Object(fields.into_iter().collect())
}

fn json_array(values: impl IntoIterator<Item = Value>) -> Value {
    Value::Array(values.into_iter().collect())
}

fn json_string_array(values: &[String]) -> Value {
    json_array(values.iter().cloned().map(Value::String))
}

fn pretty_json(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("generated JSON is serializable") + "\n"
}

fn insert_output(
    outputs: &mut BTreeMap<String, String>,
    relative: &str,
    contents: String,
) -> Result<(), Box<dyn Error>> {
    if !is_generator_owned(relative) {
        return Err(format!("generator attempted to emit an unowned path: {relative}").into());
    }
    if outputs.insert(relative.to_owned(), contents).is_some() {
        return Err(format!("generator emitted a duplicate path: {relative}").into());
    }
    Ok(())
}

fn render_bazelignore(entries: &[String]) -> String {
    let mut entries = entries.to_vec();
    entries.sort();
    entries.dedup();
    entries
        .into_iter()
        .map(|entry| format!("{}/\n", entry.trim_end_matches('/')))
        .collect()
}

fn generated_build_file() -> String {
    let mut content = String::from(
        "# Generated by cargo xtask gen-bazel. Do not edit.\n\
         # First-party target definitions live in the consolidated .bzl files.\n\n\
         package(default_visibility = [\"//visibility:public\"])\n\n\
         exports_files([\n",
    );
    for file in APPROVED_GENERATED_EXPORTS {
        content.push_str("    ");
        content.push_str(&bazel_string(file));
        content.push_str(",\n");
    }
    content.push_str("])\n");
    content
}

fn output_manifest(outputs: &BTreeMap<String, String>) -> String {
    let output_paths = APPROVED_OUTPUT_PATHS
        .iter()
        .map(|path| Value::String((*path).to_owned()))
        .collect::<Vec<_>>();
    let output_digests = outputs
        .iter()
        .map(|(path, contents)| {
            ordered_object([
                ("path".to_owned(), Value::String(path.clone())),
                (
                    "sha256".to_owned(),
                    Value::String(sha256_hex(contents.as_bytes())),
                ),
            ])
        })
        .collect::<Vec<_>>();
    pretty_json(&ordered_object([
        ("schemaVersion".to_owned(), json!(1)),
        ("manifestPath".to_owned(), json!(OUTPUT_MANIFEST_PATH)),
        ("selfDigest".to_owned(), Value::Null),
        ("outputCount".to_owned(), json!(APPROVED_OUTPUT_PATHS.len())),
        ("outputPaths".to_owned(), Value::Array(output_paths)),
        ("outputs".to_owned(), Value::Array(output_digests)),
    ]))
}

fn validate_rendered_outputs(outputs: &BTreeMap<String, String>) -> Result<(), Box<dyn Error>> {
    let expected = APPROVED_OUTPUT_PATHS
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<BTreeSet<_>>();
    let actual = outputs.keys().cloned().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err("generated output census differs from the closed ownership contract".into());
    }
    if outputs.values().any(String::is_empty) {
        return Err("generated output census contains an empty output".into());
    }
    if outputs.keys().any(|path| {
        path.contains("action-network-inventory")
            || path.contains("hermeticity-inventory")
            || path.contains("governed-rust-sources")
            || path.contains("harness-free-census")
            || path.contains("doctest-census")
            || path.ends_with("/BUILD.bazel") && path.starts_with("packages/")
    }) {
        return Err("generated output census contains an obsolete output".into());
    }
    validate_json_schema(
        "action-network-policy.json",
        &outputs["bazel/generated/action-network-policy.json"],
        &[
            "action_network",
            "aquery_actions",
            "bazel_archive_sha256",
            "bazel_source_sha256",
            "capability_abi",
            "configured_targets",
            "coverage_targets",
            "declared_inputs",
            "denied_syscalls",
            "derivation_sha256_method",
            "executable_sha256",
            "fallback_strategies",
            "fixed_policy",
            "hermeticity",
            "inherited_descriptor_plants",
            "load_point",
            "native_outputs",
            "output_nar_sha256",
            "patch_sha256",
            "policy_sha256",
            "ptrace_requests",
            "repository_fetches_outside_actions",
            "sandbox_provider",
            "schema_version",
            "socket_plants",
            "strategy_inventory",
        ],
    )?;
    validate_json_schema(
        "configured-targets.json",
        &outputs["bazel/generated/configured-targets.json"],
        &[
            "actionKinds",
            "coverageTargets",
            "hubs",
            "nativeChecks",
            "nativePolicyCheckManifest",
            "nativePolicyContextIds",
            "schemaVersion",
            "targets",
        ],
    )?;
    validate_json_schema(
        "evidence-sink-policy.json",
        &outputs["bazel/generated/evidence-sink-policy.json"],
        &["measured", "rows", "schemaVersion", "status"],
    )?;
    validate_json_schema(
        "no-shell-inventory.json",
        &outputs["bazel/generated/no-shell-inventory.json"],
        &[
            "declaredInputs",
            "governedSources",
            "scanResults",
            "schemaVersion",
            "spawnSites",
        ],
    )?;
    validate_json_schema(
        "source-census.json",
        &outputs["bazel/generated/source-census.json"],
        &["schemaVersion", "sources"],
    )?;
    validate_json_schema(
        "output-manifest.json",
        &outputs[OUTPUT_MANIFEST_PATH],
        &[
            "manifestPath",
            "outputCount",
            "outputPaths",
            "outputs",
            "schemaVersion",
            "selfDigest",
        ],
    )?;
    validate_output_manifest(outputs)?;
    Ok(())
}

fn validate_json_schema(
    name: &str,
    contents: &str,
    expected_keys: &[&str],
) -> Result<(), Box<dyn Error>> {
    let value: Value =
        serde_json::from_str(contents).map_err(|_| format!("{name} is not valid JSON"))?;
    let object = value
        .as_object()
        .ok_or_else(|| format!("{name} must be a JSON object"))?;
    let actual = object.keys().map(String::as_str).collect::<BTreeSet<_>>();
    let expected = expected_keys.iter().copied().collect::<BTreeSet<_>>();
    if actual != expected {
        return Err(format!("{name} schema differs from the closed generator contract").into());
    }
    Ok(())
}

fn validate_output_manifest(outputs: &BTreeMap<String, String>) -> Result<(), Box<dyn Error>> {
    let value: Value = serde_json::from_str(&outputs[OUTPUT_MANIFEST_PATH])
        .map_err(|_| "output manifest is not valid JSON")?;
    let object = value
        .as_object()
        .ok_or("output manifest is not an object")?;
    if object.get("schemaVersion") != Some(&json!(1))
        || object.get("manifestPath").and_then(Value::as_str) != Some(OUTPUT_MANIFEST_PATH)
        || object.get("outputCount").and_then(Value::as_u64)
            != Some(APPROVED_OUTPUT_PATHS.len() as u64)
        || !object.get("selfDigest").is_some_and(Value::is_null)
    {
        return Err("output manifest header is invalid".into());
    }
    let paths = object
        .get("outputPaths")
        .and_then(Value::as_array)
        .ok_or("output manifest paths are missing")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| "output manifest contains a non-string path".into())
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let expected_paths = APPROVED_OUTPUT_PATHS
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<Vec<_>>();
    if paths != expected_paths {
        return Err("output manifest path census is not exact and sorted".into());
    }
    let entries = object
        .get("outputs")
        .and_then(Value::as_array)
        .ok_or("output manifest digest entries are missing")?;
    if entries.len() != APPROVED_OUTPUT_PATHS.len() - 1 {
        return Err("output manifest digest census has the wrong size".into());
    }
    let mut seen = BTreeSet::new();
    for entry in entries {
        let entry = entry
            .as_object()
            .ok_or("output manifest digest entry is not an object")?;
        let path = entry
            .get("path")
            .and_then(Value::as_str)
            .ok_or("output manifest digest entry has no path")?;
        let digest = entry
            .get("sha256")
            .and_then(Value::as_str)
            .ok_or("output manifest digest entry has no sha256")?;
        let expected_digest = outputs
            .get(path)
            .map(|contents| sha256_hex(contents.as_bytes()))
            .ok_or_else(|| format!("output manifest names an unknown path: {path}"))?;
        if path == OUTPUT_MANIFEST_PATH
            || !seen.insert(path.to_owned())
            || digest != expected_digest
        {
            return Err(format!("output manifest digest is invalid for {path}").into());
        }
    }
    let expected_digest_paths = expected_paths
        .into_iter()
        .filter(|path| path != OUTPUT_MANIFEST_PATH)
        .collect::<BTreeSet<_>>();
    if seen != expected_digest_paths {
        return Err("output manifest digest paths are not exact".into());
    }
    Ok(())
}

fn validate_committed_output_census(
    root: &Path,
    expected: &BTreeMap<String, String>,
) -> Result<(), Box<dyn Error>> {
    if expected.is_empty() {
        return Err("D2B-BZLDRIFT-GENERATOR: expected generated output census is empty.".into());
    }
    let mut actual = BTreeSet::new();
    let bazelignore = root.join(".bazelignore");
    match fs::symlink_metadata(&bazelignore) {
        Ok(metadata) if metadata.file_type().is_file() => {
            actual.insert(".bazelignore".to_owned());
        }
        Ok(_) => {
            return Err("D2B-BZLDRIFT-GENERATOR: .bazelignore is not a regular file.".into());
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err("D2B-BZLDRIFT-GENERATOR: could not inspect .bazelignore.".into());
        }
    }
    let generated = root.join("bazel/generated");
    let generated_metadata = match fs::symlink_metadata(&generated) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err("D2B-BZLDRIFT-GENERATOR: bazel/generated output root is absent.".into());
        }
        Err(_) => {
            return Err("D2B-BZLDRIFT-GENERATOR: could not inspect bazel/generated.".into());
        }
    };
    if !generated_metadata.file_type().is_dir() {
        return Err(
            "D2B-BZLDRIFT-GENERATOR: bazel/generated output root is not a directory.".into(),
        );
    }
    for entry in fs::read_dir(&generated)
        .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not inspect bazel/generated.")?
    {
        let entry =
            entry.map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not inspect a generated output.")?;
        let file_type = entry
            .file_type()
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: could not inspect a generated output.")?;
        if !file_type.is_file() {
            return Err("D2B-BZLDRIFT-GENERATOR: generated output is not a regular file.".into());
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| "D2B-BZLDRIFT-GENERATOR: generated output path is not UTF-8.")?;
        actual.insert(format!("bazel/generated/{name}"));
    }
    let expected_paths = expected.keys().cloned().collect::<BTreeSet<_>>();
    let missing = expected_paths
        .difference(&actual)
        .cloned()
        .collect::<Vec<_>>();
    let extras = actual
        .difference(&expected_paths)
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() || !extras.is_empty() {
        let mut message = adr0054_drift_message("D2B-BZLDRIFT-GENERATOR")
            .expect("generator diagnostic is closed")
            .to_owned();
        if !missing.is_empty() {
            message.push_str("\nMissing paths: ");
            message.push_str(&missing.join(", "));
        }
        if !extras.is_empty() {
            message.push_str("\nExtra paths: ");
            message.push_str(&extras.join(", "));
        }
        return Err(message.into());
    }
    Ok(())
}

fn target_row_value(row: &TargetRow) -> Value {
    ordered_object([
        ("label".to_owned(), Value::String(row.label.clone())),
        ("native".to_owned(), json!(true)),
        ("package".to_owned(), Value::String(row.package.clone())),
        ("manifest".to_owned(), Value::String(row.manifest.clone())),
        ("kind".to_owned(), Value::String(row.kind.clone())),
        (
            "cargoContext".to_owned(),
            Value::String(row.cargo_context.clone()),
        ),
        (
            "system".to_owned(),
            row.system.clone().map_or(Value::Null, Value::String),
        ),
        (
            "cargoTarget".to_owned(),
            row.cargo_target.clone().map_or(Value::Null, Value::String),
        ),
        (
            "policyInput".to_owned(),
            row.policy_input.clone().map_or(Value::Null, Value::String),
        ),
        (
            "sourceFiles".to_owned(),
            json_string_array(&row.source_files),
        ),
        (
            "directFirstPartyDeps".to_owned(),
            json_string_array(&row.direct_first_party_deps),
        ),
        (
            "directProductDeps".to_owned(),
            json_string_array(&row.direct_product_deps),
        ),
        ("cfgs".to_owned(), json_string_array(&row.cfgs)),
        ("features".to_owned(), json_string_array(&row.features)),
        (
            "target".to_owned(),
            ordered_object([
                ("name".to_owned(), Value::String(row.target_name.clone())),
                ("kind".to_owned(), Value::String(row.target_kind.clone())),
                ("path".to_owned(), Value::String(row.target_path.clone())),
                (
                    "harness".to_owned(),
                    row.harness.map_or(Value::Null, Value::Bool),
                ),
                (
                    "doctest".to_owned(),
                    row.doctest.map_or(Value::Null, Value::Bool),
                ),
                (
                    "requiredFeatures".to_owned(),
                    json_string_array(&row.required_features),
                ),
                ("defaultFeatures".to_owned(), json!(row.default_features)),
                (
                    "dependencyConditions".to_owned(),
                    ordered_object(
                        row.dependency_conditions.iter().map(|(name, condition)| {
                            (name.clone(), Value::String(condition.clone()))
                        }),
                    ),
                ),
            ]),
        ),
        (
            "closureCensus".to_owned(),
            ordered_object([
                (
                    "configuredTargets".to_owned(),
                    json_string_array(&row.closure_configured_targets),
                ),
                (
                    "externalIdentities".to_owned(),
                    json_string_array(&row.closure_external_identities),
                ),
            ]),
        ),
    ])
}

fn configured_targets_json(rows: &[TargetRow]) -> Result<String, Box<dyn Error>> {
    validate_native_policy_check_manifest()?;
    let mut rows = rows.to_vec();
    rows.sort_by(|left, right| left.label.cmp(&right.label));
    if rows.windows(2).any(|pair| pair[0].label == pair[1].label) {
        return Err("configured-target census contains a duplicate label".into());
    }
    let mut action_kinds = hermeticity::GOVERNED_ACTION_KINDS
        .iter()
        .map(|kind| (*kind).to_owned())
        .collect::<Vec<_>>();
    action_kinds.sort();
    let mut coverage_targets = hermeticity::CONFIGURED_COVERAGE_TARGETS
        .iter()
        .map(|target| (*target).to_owned())
        .collect::<Vec<_>>();
    coverage_targets.sort();
    let value =
        ordered_object([
            ("schemaVersion".to_owned(), json!(1)),
            (
                "hubs".to_owned(),
                json_array(
                    ["product", "walker"]
                        .into_iter()
                        .map(|hub| Value::String(hub.to_owned())),
                ),
            ),
            (
                "targets".to_owned(),
                json_array(rows.iter().map(target_row_value)),
            ),
            (
                "coverageTargets".to_owned(),
                json_string_array(&coverage_targets),
            ),
            ("actionKinds".to_owned(), json_string_array(&action_kinds)),
            (
                "nativePolicyCheckManifest".to_owned(),
                Value::String("tests/golden/native-policy-check-manifest.json".to_owned()),
            ),
            (
                "nativePolicyContextIds".to_owned(),
                json_array(POLICY_TARGET_CONTEXTS.iter().map(
                    |(context, _, system, target, _, _)| {
                        Value::String(format!("{system}/{target}/{context}"))
                    },
                )),
            ),
            (
                "nativeChecks".to_owned(),
                json_array(
                    [
                        "broker-production-dependency-policy",
                        "guest-shell-runner-static-dependency-policy",
                        "broker-production-package-policy",
                        "guest-real-libshpool-package-policy",
                        "broker-host-artifact-contract",
                        "guest-static-elf",
                    ]
                    .into_iter()
                    .map(|check| Value::String(check.to_owned())),
                ),
            ),
        ]);
    Ok(pretty_json(&value))
}

fn product_targets_bzl(rows: &[TargetRow]) -> Result<String, Box<dyn Error>> {
    let mut product_rows = rows
        .iter()
        .filter(|row| row.system.is_none())
        .map(target_row_value)
        .collect::<Vec<_>>();
    product_rows.sort_by_key(|row| {
        row.get("label")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    });
    if product_rows.is_empty() {
        return Err("product target definitions are empty".into());
    }
    Ok(format!(
        "# Generated by cargo xtask gen-bazel. Do not edit.\n\
         # Native first-party targets; third-party dependencies remain in @product.\n\n\
         PRODUCT_TARGETS = {}\n",
        starlark_value(&Value::Array(product_rows), 0)
    ))
}

fn package_policy_targets_bzl(rows: &[TargetRow]) -> Result<String, Box<dyn Error>> {
    let mut policy_rows = rows
        .iter()
        .filter(|row| row.system.is_some())
        .map(target_row_value)
        .collect::<Vec<_>>();
    policy_rows.sort_by_key(|row| {
        row.get("label")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    });
    if policy_rows.len() != POLICY_TARGET_CONTEXTS.len() {
        return Err(format!(
            "package-policy target census must contain exactly {} contexts",
            POLICY_TARGET_CONTEXTS.len()
        )
        .into());
    }
    Ok(format!(
        "# Generated by cargo xtask gen-bazel. Do not edit.\n\
         # Native package-policy targets are selected from the root product lock.\n\n\
         PACKAGE_POLICY_TARGETS = {}\n",
        starlark_value(&Value::Array(policy_rows), 0)
    ))
}

fn starlark_value(value: &Value, indent: usize) -> String {
    match value {
        Value::Null => "None".to_owned(),
        Value::Bool(value) => value
            .to_string()
            .replace("true", "True")
            .replace("false", "False"),
        Value::Number(value) => value.to_string(),
        Value::String(value) => bazel_string(value),
        Value::Array(values) => {
            if values.is_empty() {
                return "[]".to_owned();
            }
            let mut output = String::from("[\n");
            for value in values {
                output.push_str(&" ".repeat(indent + 4));
                output.push_str(&starlark_value(value, indent + 4));
                output.push_str(",\n");
            }
            output.push_str(&" ".repeat(indent));
            output.push(']');
            output
        }
        Value::Object(values) => {
            if values.is_empty() {
                return "{}".to_owned();
            }
            let mut output = String::from("{\n");
            for (key, value) in values {
                output.push_str(&" ".repeat(indent + 4));
                output.push_str(&bazel_string(key));
                output.push_str(": ");
                output.push_str(&starlark_value(value, indent + 4));
                output.push_str(",\n");
            }
            output.push_str(&" ".repeat(indent));
            output.push('}');
            output
        }
    }
}

fn active_features(manifest: &ManifestInfo, requested: &[String]) -> BTreeSet<String> {
    let mut active = requested.iter().cloned().collect::<BTreeSet<_>>();
    let mut pending = active.iter().cloned().collect::<Vec<_>>();
    while let Some(feature) = pending.pop() {
        let Some(values) = manifest.feature_dependencies.get(&feature) else {
            continue;
        };
        for value in values {
            active.insert(value.clone());
            let name = value
                .strip_prefix("dep:")
                .unwrap_or(value)
                .split_once('/')
                .map_or(value.as_str(), |(name, _)| name);
            if manifest.feature_dependencies.contains_key(name) && active.insert(name.to_owned()) {
                pending.push(name.to_owned());
            }
        }
    }
    active
}

fn effective_target_features(base: &[String], required: &[String]) -> Vec<String> {
    let mut effective = base.to_vec();
    for feature in required {
        if !effective.contains(feature) {
            effective.push(feature.clone());
        }
    }
    effective
}

fn effective_dependency_names(
    manifest: &ManifestInfo,
    package: &DependencyInfo,
    requested_features: &[String],
    include_dev: bool,
) -> Vec<String> {
    let active = active_features(manifest, requested_features);
    let mut names = package.normal.clone();
    if include_dev {
        names.extend(package.dev.iter().cloned());
    }
    names.sort();
    names.dedup();
    names
        .into_iter()
        .filter(|name| {
            !package.optional.contains(name)
                || active.contains(name)
                || active.contains(&format!("dep:{name}"))
        })
        .collect()
}

fn direct_dependency_sets(
    manifest: &ManifestInfo,
    package: &DependencyInfo,
    dependencies: &BTreeMap<String, DependencyInfo>,
    requested_features: &[String],
    include_dev: bool,
) -> (Vec<String>, Vec<String>, BTreeMap<String, String>) {
    let mut first_party = Vec::new();
    let mut product = Vec::new();
    let mut conditions = BTreeMap::new();
    for name in effective_dependency_names(manifest, package, requested_features, include_dev) {
        if let Some(local) = dependencies
            .values()
            .find(|candidate| candidate.package_name == name && candidate.hub == "product")
        {
            first_party.push(format!("//{}:{}", local.package_dir, local.package_name));
        } else if package.hub == "product" {
            product.push(format!("@product//:{}", name));
        } else {
            product.push(format!("@walker//:{}", name));
        }
        if let Some(condition) = package.target_conditions.get(&name) {
            conditions.insert(name, condition.clone());
        }
    }
    first_party.sort();
    first_party.dedup();
    product.sort();
    product.dedup();
    (first_party, product, conditions)
}

fn target_cfgs(system: &str, cargo_target: &str) -> Vec<String> {
    let arch = system.strip_suffix("-linux").unwrap_or(system);
    let environment = cargo_target
        .rsplit_once('-')
        .map(|(_, environment)| environment)
        .unwrap_or("gnu");
    let mut cfgs = vec![
        format!("target_arch=\"{arch}\""),
        "target_os=\"linux\"".to_owned(),
        format!("target_env=\"{environment}\""),
    ];
    cfgs.sort();
    cfgs
}

fn configured_target_rows(
    _root: &Path,
    manifests: &[ManifestInfo],
    dependencies: &BTreeMap<String, DependencyInfo>,
) -> Result<Vec<TargetRow>, Box<dyn Error>> {
    let mut rows = Vec::new();
    for manifest in manifests {
        let package = dependencies
            .get(&manifest.relative)
            .ok_or_else(|| format!("missing dependency graph entry for {}", manifest.relative))?;
        let package_sources = package_source_paths(_root, &manifest.package_dir)?;
        let mut targets = Vec::new();
        if let Some(target) = &manifest.lib {
            targets.push(target.clone());
        }
        targets.extend(manifest.bins.iter().cloned());
        targets.extend(manifest.tests.iter().cloned());
        targets.extend(manifest.benches.iter().cloned());
        targets.extend(manifest.examples.iter().cloned());
        if targets.is_empty() {
            return Err(format!(
                "Cargo package has no discoverable targets: {}",
                manifest.package_name
            )
            .into());
        }
        for target in targets {
            let requested_features =
                effective_target_features(&manifest.default_features, &target.required_features);
            let include_dev = matches!(target.kind.as_str(), "test" | "bench");
            let (direct_first_party_deps, direct_product_deps, dependency_conditions) =
                direct_dependency_sets(
                    manifest,
                    package,
                    dependencies,
                    &requested_features,
                    include_dev,
                );
            let label = target_label(manifest, &target);
            rows.push(TargetRow {
                label: label.clone(),
                package: manifest.package_name.clone(),
                manifest: manifest.relative.clone(),
                kind: bazel_target_kind(&target.kind).to_owned(),
                cargo_context: "main-default".to_owned(),
                system: None,
                cargo_target: None,
                policy_input: None,
                source_files: package_sources.clone(),
                direct_first_party_deps,
                direct_product_deps: direct_product_deps.clone(),
                cfgs: Vec::new(),
                features: requested_features,
                target_name: target.name,
                target_kind: target.kind,
                target_path: target.path,
                harness: target.harness,
                doctest: target.doctest,
                required_features: target.required_features,
                default_features: true,
                dependency_conditions,
                closure_configured_targets: vec![label],
                closure_external_identities: direct_product_deps,
            });
        }
    }

    for (context, package_name, system, cargo_target, policy_input, features) in
        POLICY_TARGET_CONTEXTS
    {
        let manifest = manifests
            .iter()
            .find(|manifest| manifest.package_name == *package_name)
            .ok_or_else(|| format!("package-policy package is missing: {package_name}"))?;
        let package = dependencies
            .get(&manifest.relative)
            .ok_or_else(|| format!("missing dependency graph entry for {}", manifest.relative))?;
        let selected_target = manifest
            .bins
            .iter()
            .find(|target| target.name == *package_name)
            .or_else(|| manifest.bins.first())
            .or(manifest.lib.as_ref())
            .ok_or_else(|| format!("package-policy target is missing: {package_name}"))?;
        let requested_features = features
            .iter()
            .map(|feature| (*feature).to_owned())
            .collect::<Vec<_>>();
        let requested_features =
            effective_target_features(&requested_features, &selected_target.required_features);
        let required_features = requested_features.clone();
        let (direct_first_party_deps, direct_product_deps, dependency_conditions) =
            direct_dependency_sets(manifest, package, dependencies, &requested_features, true);
        let system_suffix = system.strip_suffix("-linux").unwrap_or(system);
        let label = format!(
            "//{}:{}-{}-{}",
            manifest.package_dir, manifest.package_name, context, system_suffix
        );
        rows.push(TargetRow {
            label: label.clone(),
            package: manifest.package_name.clone(),
            manifest: manifest.relative.clone(),
            kind: bazel_target_kind(&selected_target.kind).to_owned(),
            cargo_context: format!("{context}-{system_suffix}"),
            system: Some((*system).to_owned()),
            cargo_target: Some((*cargo_target).to_owned()),
            policy_input: Some((*policy_input).to_owned()),
            source_files: package_source_paths(_root, &manifest.package_dir)?,
            direct_first_party_deps,
            direct_product_deps: direct_product_deps.clone(),
            cfgs: target_cfgs(system, cargo_target),
            features: requested_features,
            target_name: selected_target.name.clone(),
            target_kind: selected_target.kind.clone(),
            target_path: selected_target.path.clone(),
            harness: selected_target.harness,
            doctest: selected_target.doctest,
            required_features,
            default_features: false,
            dependency_conditions,
            closure_configured_targets: vec![label],
            closure_external_identities: direct_product_deps,
        });
    }
    normalize_first_party_labels(&mut rows, manifests);
    complete_target_closures(&mut rows);
    rows.sort_by(|left, right| left.label.cmp(&right.label));
    if rows.windows(2).any(|pair| pair[0].label == pair[1].label) {
        return Err("configured target definitions contain duplicate labels".into());
    }
    Ok(rows)
}

fn bazel_target_kind(kind: &str) -> &'static str {
    match kind {
        "lib" => "rust_library",
        "bin" | "example" => "rust_binary",
        "test" | "bench" => "rust_test",
        _ => "rust_binary",
    }
}

fn target_label(manifest: &ManifestInfo, target: &TargetInfo) -> String {
    let name = if target.kind == "bin" && target.name == manifest.package_name {
        format!("{}-bin", target.name)
    } else {
        target.name.clone()
    };
    format!("//{}:{}", manifest.package_dir, name)
}

fn normalize_first_party_labels(rows: &mut [TargetRow], manifests: &[ManifestInfo]) {
    let primary = manifests
        .iter()
        .filter_map(|manifest| {
            let target = manifest
                .lib
                .as_ref()
                .or_else(|| manifest.bins.first())
                .map(|target| target_label(manifest, target))?;
            Some((manifest.package_name.clone(), target))
        })
        .collect::<BTreeMap<_, _>>();
    for row in rows {
        for dependency in &mut row.direct_first_party_deps {
            let Some(package_name) = dependency.rsplit_once(':').map(|(_, name)| name) else {
                continue;
            };
            if let Some(label) = primary.get(package_name) {
                *dependency = label.clone();
            }
        }
        row.direct_first_party_deps.sort();
        row.direct_first_party_deps.dedup();
    }
}

fn complete_target_closures(rows: &mut [TargetRow]) {
    for _ in 0..rows.len().saturating_add(1) {
        let previous = rows
            .iter()
            .map(|row| {
                (
                    row.label.clone(),
                    (
                        row.closure_configured_targets.clone(),
                        row.closure_external_identities.clone(),
                    ),
                )
            })
            .collect::<BTreeMap<_, _>>();
        let mut changed = false;
        for row in rows.iter_mut() {
            let mut configured = BTreeSet::from([row.label.clone()]);
            let mut external = row
                .closure_external_identities
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>();
            for dependency in &row.direct_first_party_deps {
                if let Some((child_configured, child_external)) = previous.get(dependency) {
                    configured.extend(child_configured.iter().cloned());
                    external.extend(child_external.iter().cloned());
                }
            }
            let configured = configured.into_iter().collect::<Vec<_>>();
            let external = external.into_iter().collect::<Vec<_>>();
            changed |= row.closure_configured_targets != configured
                || row.closure_external_identities != external;
            row.closure_configured_targets = configured;
            row.closure_external_identities = external;
        }
        if !changed {
            break;
        }
    }
}

fn package_source_paths(root: &Path, package_dir: &str) -> Result<Vec<String>, Box<dyn Error>> {
    let prefix = format!("{package_dir}/src/");
    let mut paths = tracked_paths(root)?
        .into_iter()
        .filter(|path| path.starts_with(&prefix) && path.ends_with(".rs"))
        .collect::<Vec<_>>();
    paths.sort();
    paths.dedup();
    if paths.is_empty() {
        return Err(
            format!("first-party package has no tracked Rust sources: {package_dir}").into(),
        );
    }
    Ok(paths)
}

fn first_party_source_paths(
    root: &Path,
    manifests: &[ManifestInfo],
) -> Result<Vec<String>, Box<dyn Error>> {
    let mut paths = BTreeSet::new();
    for manifest in manifests {
        paths.extend(package_source_paths(root, &manifest.package_dir)?);
    }
    if paths.is_empty() {
        return Err("source census is empty".into());
    }
    Ok(paths.into_iter().collect())
}

fn source_census_json(
    root: &Path,
    manifests: &[ManifestInfo],
    rows: &[TargetRow],
    source_paths: &[String],
) -> Result<String, Box<dyn Error>> {
    let mut sources = Vec::new();
    for path in source_paths {
        let manifest = manifests
            .iter()
            .find(|manifest| path.starts_with(&format!("{}/", manifest.package_dir)))
            .ok_or_else(|| format!("source is outside first-party manifests: {path}"))?;
        let mut target_labels = rows
            .iter()
            .filter(|row| row.source_files.iter().any(|source| source == path))
            .map(|row| row.label.clone())
            .collect::<Vec<_>>();
        target_labels.sort();
        target_labels.dedup();
        if target_labels.is_empty() {
            return Err(format!("source is not bound to a configured target: {path}").into());
        }
        sources.push(ordered_object([
            ("path".to_owned(), Value::String(path.clone())),
            (
                "manifest".to_owned(),
                Value::String(manifest.relative.clone()),
            ),
            (
                "package".to_owned(),
                Value::String(manifest.package_name.clone()),
            ),
            (
                "sha256".to_owned(),
                Value::String(sha256_hex(
                    &fs::read(root.join(path))
                        .map_err(|_| format!("source census entry is unreadable: {path}"))?,
                )),
            ),
            ("targetLabels".to_owned(), json_string_array(&target_labels)),
        ]));
    }
    Ok(pretty_json(&ordered_object([
        ("schemaVersion".to_owned(), json!(1)),
        ("sources".to_owned(), Value::Array(sources)),
    ])))
}

fn no_shell_inventory_json(root: &Path, source_paths: &[String]) -> Result<String, Box<dyn Error>> {
    if source_paths.is_empty() {
        return Err("no-shell-inventory-empty".into());
    }
    let mut all_sites = Vec::new();
    let mut scan_results = Vec::new();
    for source in source_paths {
        let contents = fs::read_to_string(root.join(source))
            .map_err(|_| format!("no-shell-inventory-missing-entry: {source}"))?;
        let sites = scan_spawn_sites(source, &contents)?;
        if sites.iter().any(|site| site.shell_invocation) {
            return Err("no-shell-inventory-planted-shell".into());
        }
        let keys = sites.iter().map(spawn_site_key).collect::<Vec<_>>();
        scan_results.push(ordered_object([
            ("source".to_owned(), Value::String(source.clone())),
            ("status".to_owned(), json!("scanned")),
            ("spawnSiteCount".to_owned(), json!(sites.len())),
            ("spawnSiteKeys".to_owned(), json_string_array(&keys)),
        ]));
        all_sites.extend(sites);
    }
    all_sites.sort_by_key(spawn_site_key);
    if all_sites
        .windows(2)
        .any(|pair| spawn_site_key(&pair[0]) == spawn_site_key(&pair[1]))
    {
        return Err("no-shell-inventory-extra-entry".into());
    }
    let governed = source_paths
        .iter()
        .map(|path| ordered_object([("path".to_owned(), Value::String(path.clone()))]))
        .collect::<Vec<_>>();
    let spawn_sites = all_sites
        .iter()
        .map(|site| {
            ordered_object([
                ("source".to_owned(), Value::String(site.source.clone())),
                (
                    "span".to_owned(),
                    ordered_object([
                        ("startLine".to_owned(), json!(site.start_line)),
                        ("startColumn".to_owned(), json!(site.start_column)),
                        ("endLine".to_owned(), json!(site.end_line)),
                        ("endColumn".to_owned(), json!(site.end_column)),
                    ]),
                ),
                (
                    "programExpression".to_owned(),
                    Value::String(site.program_expression.clone()),
                ),
                ("shellInvocation".to_owned(), json!(site.shell_invocation)),
            ])
        })
        .collect::<Vec<_>>();
    Ok(pretty_json(&ordered_object([
        ("schemaVersion".to_owned(), json!(1)),
        ("governedSources".to_owned(), Value::Array(governed.clone())),
        ("declaredInputs".to_owned(), Value::Array(governed)),
        ("scanResults".to_owned(), Value::Array(scan_results)),
        ("spawnSites".to_owned(), Value::Array(spawn_sites)),
    ])))
}

fn scan_spawn_sites(source: &str, contents: &str) -> Result<Vec<SpawnSite>, Box<dyn Error>> {
    let masked = mask_rust_source(contents);
    let marker = "Command::new(";
    let mut sites = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = masked[cursor..].find(marker) {
        let marker_start = cursor + relative;
        let alias_start = marker_start
            .checked_sub(5)
            .filter(|start| &masked[*start..marker_start] == "Tokio");
        if alias_start.is_none()
            && marker_start > 0
            && is_identifier_byte(masked.as_bytes()[marker_start - 1])
        {
            cursor = marker_start + marker.len();
            continue;
        }
        let open = marker_start + marker.len() - 1;
        let close = matching_paren(&masked, open)
            .ok_or_else(|| format!("no-shell-inventory-missing-entry: {source}"))?;
        let expression = contents[open + 1..close].trim().to_owned();
        if expression.is_empty() {
            return Err(format!("no-shell-inventory-unguarded-spawn: {source}").into());
        }
        let call_start = alias_start.unwrap_or(marker_start);
        let (start_line, start_column) = line_column(contents, call_start);
        let (end_line, end_column) = line_column(contents, close + 1);
        sites.push(SpawnSite {
            source: source.to_owned(),
            start_line,
            start_column,
            end_line,
            end_column,
            shell_invocation: shell_program_expression(&expression),
            program_expression: expression.split_whitespace().collect::<Vec<_>>().join(" "),
        });
        cursor = close + 1;
    }
    sites.sort_by_key(spawn_site_key);
    Ok(sites)
}

fn is_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

fn mask_rust_source(contents: &str) -> String {
    let bytes = contents.as_bytes();
    let mut masked = bytes.to_vec();
    let mut index = 0;
    let mut block_depth = 0usize;
    while index < bytes.len() {
        if block_depth > 0 {
            if bytes[index..].starts_with(b"/*") {
                masked[index] = b' ';
                if index + 1 < masked.len() {
                    masked[index + 1] = b' ';
                }
                block_depth += 1;
                index += 2;
            } else if bytes[index..].starts_with(b"*/") {
                masked[index] = b' ';
                if index + 1 < masked.len() {
                    masked[index + 1] = b' ';
                }
                block_depth -= 1;
                index += 2;
            } else {
                if masked[index] != b'\n' {
                    masked[index] = b' ';
                }
                index += 1;
            }
            continue;
        }
        if bytes[index..].starts_with(b"//") {
            masked[index] = b' ';
            if index + 1 < masked.len() {
                masked[index + 1] = b' ';
            }
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                masked[index] = b' ';
                index += 1;
            }
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            masked[index] = b' ';
            if index + 1 < masked.len() {
                masked[index + 1] = b' ';
            }
            block_depth = 1;
            index += 2;
            continue;
        }
        if bytes[index] == b'r' {
            let mut quote = index + 1;
            while quote < bytes.len() && bytes[quote] == b'#' {
                quote += 1;
            }
            if quote < bytes.len() && bytes[quote] == b'"' {
                let hashes = quote - index - 1;
                for byte in &mut masked[index..=quote] {
                    if *byte != b'\n' {
                        *byte = b' ';
                    }
                }
                index = quote + 1;
                while index < bytes.len() {
                    if bytes[index] == b'"' && bytes[index + 1..].starts_with(&vec![b'#'; hashes]) {
                        masked[index] = b' ';
                        for offset in 0..hashes {
                            masked[index + 1 + offset] = b' ';
                        }
                        index += hashes + 1;
                        break;
                    }
                    if masked[index] != b'\n' {
                        masked[index] = b' ';
                    }
                    index += 1;
                }
                continue;
            }
        }
        if bytes[index] == b'"' {
            masked[index] = b' ';
            index += 1;
            while index < bytes.len() {
                let escaped = index > 0 && bytes[index - 1] == b'\\';
                if bytes[index] == b'"' && !escaped {
                    masked[index] = b' ';
                    index += 1;
                    break;
                }
                if masked[index] != b'\n' {
                    masked[index] = b' ';
                }
                index += 1;
            }
            continue;
        }
        let is_char_literal = bytes[index] == b'\''
            && (bytes.get(index + 1) == Some(&b'\\') || bytes.get(index + 2) == Some(&b'\''));
        if is_char_literal {
            masked[index] = b' ';
            index += 1;
            while index < bytes.len() {
                let escaped = index > 0 && bytes[index - 1] == b'\\';
                if bytes[index] == b'\'' && !escaped {
                    masked[index] = b' ';
                    index += 1;
                    break;
                }
                if masked[index] != b'\n' {
                    masked[index] = b' ';
                }
                index += 1;
            }
            continue;
        }
        index += 1;
    }
    String::from_utf8(masked).expect("source was valid UTF-8")
}

fn matching_paren(masked: &str, open: usize) -> Option<usize> {
    let bytes = masked.as_bytes();
    let mut depth: usize = 0;
    for (index, byte) in bytes.iter().enumerate().skip(open) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn line_column(contents: &str, offset: usize) -> (usize, usize) {
    let prefix = &contents[..offset.min(contents.len())];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix.len() + 1, |(_, line)| line.len() + 1);
    (line, column)
}

fn shell_program_expression(expression: &str) -> bool {
    let normalized = expression.trim().trim_matches('"').to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "sh" | "bash"
            | "/bin/sh"
            | "/bin/bash"
            | "/usr/bin/sh"
            | "/usr/bin/bash"
            | "/usr/bin/env sh"
            | "/usr/bin/env bash"
    ) || normalized.contains("sh -c")
        || normalized.contains("bash -c")
}

fn spawn_site_key(site: &SpawnSite) -> String {
    format!(
        "{}:{}:{}:{}",
        site.source, site.start_line, site.start_column, site.program_expression
    )
}

fn evidence_sink_policy_json() -> String {
    let rows = [
        (
            "evidence-v1",
            "execution-evidence",
            30,
            262_144,
            32,
            "workflow-and-head-digest",
        ),
        (
            "exporter-diagnostic-v1",
            "exporter-diagnostic",
            7,
            32_768,
            64,
            "workflow-and-head-digest",
        ),
        ("junit-v1", "junit", 14, 65_536, 128, "slice-output-root"),
        (
            "test-log-v1",
            "test-log",
            14,
            65_536,
            128,
            "slice-output-root",
        ),
    ];
    let rows = rows
        .iter()
        .map(
            |(retention_class, sink_kind, max_age_days, max_bytes, max_records, scope)| {
                ordered_object([
                    (
                        "retentionClass".to_owned(),
                        Value::String((*retention_class).to_owned()),
                    ),
                    (
                        "sinkKind".to_owned(),
                        Value::String((*sink_kind).to_owned()),
                    ),
                    ("maxAgeDays".to_owned(), json!(max_age_days)),
                    ("maxBytes".to_owned(), json!(max_bytes)),
                    ("maxRecords".to_owned(), json!(max_records)),
                    ("scope".to_owned(), Value::String((*scope).to_owned())),
                    (
                        "permittedFields".to_owned(),
                        json_array(
                            [
                                "sinkKind",
                                "retentionClass",
                                "testVerdict",
                                "evidenceStatus",
                            ]
                            .into_iter()
                            .map(|field| Value::String(field.to_owned())),
                        ),
                    ),
                    (
                        "truncationCode".to_owned(),
                        json!("D2B-BZLEVIDENCE-TRUNCATED"),
                    ),
                ])
            },
        )
        .collect::<Vec<_>>();
    pretty_json(&ordered_object([
        ("schemaVersion".to_owned(), json!(1)),
        ("status".to_owned(), json!("unmeasured")),
        ("measured".to_owned(), json!(false)),
        ("rows".to_owned(), Value::Array(rows)),
    ]))
}

fn action_network_policy_json(hermeticity: &str) -> Result<String, Box<dyn Error>> {
    let inventory = hermeticity::complete_action_network_inventory();
    hermeticity::validate_action_network_inventory(&inventory)
        .map_err(|_| "action-network inventory is invalid")?;
    let hermeticity: Value =
        serde_json::from_str(hermeticity).map_err(|_| "hermeticity inventory is not valid JSON")?;
    let mut policy = serde_json::to_value(inventory)
        .map_err(|_| "action-network inventory could not be serialized")?;
    let object = policy
        .as_object_mut()
        .ok_or("action-network inventory did not serialize as an object")?;
    object.insert("schema_version".to_owned(), json!(1));
    object.insert("hermeticity".to_owned(), hermeticity);
    Ok(pretty_json(&policy))
}

fn generate_model(root: &Path) -> Result<GeneratedModel, Box<dyn Error>> {
    validate_native_policy_check_manifest()?;
    validate_generator_inputs(root)?;
    let manifests = discover_manifests(root)?;
    if manifests.is_empty() {
        return Err("no Cargo package manifests were discovered".into());
    }
    let dependencies = dependency_graph(root)?;
    let product_manifests = manifests
        .iter()
        .filter(|manifest| manifest.relative.starts_with("packages/"))
        .cloned()
        .collect::<Vec<_>>();
    if product_manifests.is_empty() {
        return Err("no first-party product manifests were discovered".into());
    }
    let target_rows = configured_target_rows(root, &product_manifests, &dependencies)?;
    if target_rows.is_empty() {
        return Err("configured first-party target census is empty".into());
    }
    let source_paths = first_party_source_paths(root, &product_manifests)?;
    let source_census = source_census_json(root, &product_manifests, &target_rows, &source_paths)?;
    let no_shell_inventory = no_shell_inventory_json(root, &source_paths)?;
    let hermeticity = hermeticity_artifact(root)?;
    let action_network_policy = action_network_policy_json(&hermeticity)?;
    let configured_targets = configured_targets_json(&target_rows)?;
    let product_targets = product_targets_bzl(&target_rows)?;
    let package_policy_targets = package_policy_targets_bzl(&target_rows)?;
    let evidence_sink_policy = evidence_sink_policy_json();
    let bazelignore = bazelignore_entries(root, &manifests)?;
    Ok(GeneratedModel {
        bazelignore,
        action_network_policy,
        configured_targets,
        evidence_sink_policy,
        no_shell_inventory,
        package_policy_targets,
        product_targets,
        source_census,
    })
}

fn dependency_graph(root: &Path) -> Result<BTreeMap<String, DependencyInfo>, Box<dyn Error>> {
    let mut graph = BTreeMap::new();
    for (hub, manifest, _) in HUBS {
        let mut command = Command::new("cargo");
        command.current_dir(root).args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--offline",
        ]);
        if *hub == "walker" {
            command.arg("--no-deps");
        } else {
            command.args(["--filter-platform", GENERATOR_METADATA_TARGET]);
        }
        let output = command
            .args(["--manifest-path", manifest])
            .output()
            .map_err(|_| {
                adr0054_drift_message("D2B-BZL-METADATA")
                    .expect("metadata diagnostic is closed")
                    .to_owned()
            })?;
        if !output.status.success() {
            return Err(adr0054_drift_message("D2B-BZL-METADATA")
                .expect("metadata diagnostic is closed")
                .into());
        }
        let document: Value = serde_json::from_slice(&output.stdout).map_err(|_| {
            adr0054_drift_message("D2B-BZL-METADATA")
                .expect("metadata diagnostic is closed")
                .to_owned()
        })?;
        let packages = document
            .get("packages")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                adr0054_drift_message("D2B-BZL-METADATA")
                    .expect("metadata diagnostic is closed")
                    .to_owned()
            })?;
        let proc_macro_packages = packages
            .iter()
            .filter(|package| {
                package.get("source").and_then(Value::as_str).is_some()
                    && package
                        .get("targets")
                        .and_then(Value::as_array)
                        .is_some_and(|targets| {
                            targets.iter().any(|target| {
                                target
                                    .get("kind")
                                    .and_then(Value::as_array)
                                    .is_some_and(|kinds| {
                                        kinds.iter().any(|kind| kind.as_str() == Some("proc-macro"))
                                    })
                            })
                        })
            })
            .filter_map(|package| package.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        for package in packages {
            let Some(manifest_path) = package.get("manifest_path").and_then(Value::as_str) else {
                continue;
            };
            let manifest_path = Path::new(manifest_path);
            let Ok(relative) = manifest_path.strip_prefix(root) else {
                continue;
            };
            let relative = relative.to_string_lossy().replace('\\', "/");
            let Some(package_name) = package.get("name").and_then(Value::as_str) else {
                continue;
            };
            let mut normal = BTreeSet::new();
            let mut dev = BTreeSet::new();
            let mut optional = BTreeSet::new();
            let mut target_conditions = BTreeMap::new();
            if let Some(dependencies) = package.get("dependencies").and_then(Value::as_array) {
                for dependency in dependencies {
                    let Some(name) = dependency.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    if dependency.get("kind").and_then(Value::as_str) == Some("dev") {
                        dev.insert(name.to_owned());
                    } else {
                        normal.insert(name.to_owned());
                    }
                    if dependency
                        .get("optional")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                    {
                        optional.insert(name.to_owned());
                    }
                    if let Some(target) = dependency.get("target").and_then(Value::as_str) {
                        target_conditions.insert(name.to_owned(), target.to_owned());
                    }
                }
            }
            let package_dir = Path::new(&relative)
                .parent()
                .ok_or_else(|| format!("Cargo manifest has no parent: {relative}"))?
                .to_string_lossy()
                .replace('\\', "/");
            let proc_macro = normal
                .iter()
                .chain(dev.iter())
                .filter(|name| proc_macro_packages.contains(*name))
                .cloned()
                .collect::<BTreeSet<_>>();
            graph.insert(
                relative,
                DependencyInfo {
                    package_name: package_name.to_owned(),
                    package_dir,
                    hub: (*hub).to_owned(),
                    normal: normal.into_iter().collect(),
                    dev: dev.into_iter().collect(),
                    optional,
                    proc_macro,
                    target_conditions,
                },
            );
        }
    }
    if graph.is_empty() {
        return Err("cargo metadata produced an empty first-party dependency graph".into());
    }
    Ok(graph)
}

fn hermeticity_artifact(root: &Path) -> Result<String, Box<dyn Error>> {
    let hubs = HUBS
        .iter()
        .map(|(name, _, cargo_lock)| {
            let side_lock = root.join(format!("bazel/cargo/{name}.lock"));
            let text = fs::read_to_string(&side_lock).unwrap_or_default();
            let document = if text.trim().is_empty() {
                Value::Object(serde_json::Map::new())
            } else {
                serde_json::from_str(&text).map_err(|_| {
                    format!("Bazel-side lock for hermeticity inventory {name} is not valid JSON")
                })?
            };
            let crates = document
                .get("crates")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let packages = crates
                .values()
                .map(|record| {
                    let package_name = record
                        .get("name")
                        .and_then(Value::as_str)
                        .ok_or_else(|| "Bazel crate record has no name".to_owned())?;
                    let version =
                        record
                            .get("version")
                            .and_then(Value::as_str)
                            .ok_or_else(|| {
                                format!("Bazel crate record {package_name} has no version")
                            })?;
                    let source = record
                        .get("repository")
                        .filter(|repository| !repository.is_null())
                        .map(|_| {
                            record
                                .get("package_url")
                                .and_then(Value::as_str)
                                .unwrap_or("registry")
                                .to_owned()
                        });
                    let build_script_target = record
                        .get("targets")
                        .and_then(Value::as_array)
                        .is_some_and(|targets| {
                            targets.iter().any(|target| {
                                target
                                    .as_object()
                                    .is_some_and(|target| target.contains_key("BuildScript"))
                            })
                        });
                    let required_annotations = (build_script_target && source.is_some())
                        .then(|| lock_annotations(record.get("build_script_attrs")));
                    Ok(hermeticity::PackageInput {
                        name: package_name.to_owned(),
                        version: version.to_owned(),
                        source,
                        build_script_target,
                        required_annotations,
                    })
                })
                .collect::<Result<Vec<_>, String>>()?;
            Ok(hermeticity::HubInput {
                hub: hermeticity_hub(name)?,
                lock_attrs: hermeticity::HubLockAttrs {
                    lockfile: format!("bazel/cargo/{name}.lock"),
                    cargo_lockfile: (*cargo_lock).to_owned(),
                    skip_cargo_lockfile_overwrite: true,
                },
                packages,
            })
        })
        .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
    let input = hermeticity::InventoryInput {
        hubs,
        observed_action_environment: hermeticity::pinned_action_env_allowlist()
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
    };
    let artifact = hermeticity::generated_artifact(&input)
        .map_err(|_| "hermeticity inventory generation failed")?;
    Ok(artifact.contents)
}

fn hermeticity_hub(name: &str) -> Result<hermeticity::Hub, Box<dyn Error>> {
    match name {
        "product" => Ok(hermeticity::Hub::Product),
        "walker" => Ok(hermeticity::Hub::Walker),
        _ => Err("D2B-BZL-HUB: generator hub selection is not closed.".into()),
    }
}

fn lock_annotations(attrs: Option<&Value>) -> hermeticity::RequiredAnnotations {
    let mut annotations = hermeticity::RequiredAnnotations::default();
    let Some(attrs) = attrs else {
        return annotations;
    };
    for (field, destination) in [
        ("data_glob", &mut annotations.build_script_data),
        ("tools", &mut annotations.build_script_tools),
        ("toolchains", &mut annotations.build_script_toolchains),
    ] {
        if let Some(values) = attrs.get(field).and_then(Value::as_array) {
            for value in values.iter().filter_map(Value::as_str) {
                destination.insert(value.to_owned());
            }
        }
    }
    if let Some(values) = attrs.get("env").and_then(Value::as_object) {
        for (name, value) in values {
            annotations
                .build_script_env
                .insert(name.clone(), value.as_str().unwrap_or_default().to_owned());
        }
    }
    annotations.build_script_use_cc_toolchain = attrs
        .get("use_cc_toolchain")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    annotations.build_script_use_default_shell_env = attrs
        .get("use_default_shell_env")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    annotations
}

fn validate_generator_inputs(root: &Path) -> Result<(), Box<dyn Error>> {
    let require_hub_locks = root.join("MODULE.bazel.lock").is_file();
    for (name, manifest, lock) in HUBS {
        let manifest_path = root.join(manifest);
        let lock_path = root.join(lock);
        let manifest_text = fs::read_to_string(&manifest_path)
            .map_err(|_| format!("cannot read Cargo metadata root {manifest}"))?;
        let lock_text =
            fs::read_to_string(&lock_path).map_err(|_| format!("cannot read Cargo lock {lock}"))?;
        let package_names = package_names(root, manifest, &manifest_text);
        let lock_packages = lock_packages(&lock_text);
        if lock_packages.is_empty() {
            return Err(format!("Cargo lock {lock} has no package records").into());
        }
        let missing = package_names
            .into_iter()
            .filter(|name| !lock_packages.iter().any(|(lock_name, _)| lock_name == name))
            .collect::<Vec<_>>();
        if !missing.is_empty() {
            let code = if *name == "product" {
                "D2B-CARGODRIFT-PRODUCT"
            } else {
                "D2B-CARGODRIFT-WALKER"
            };
            return Err(adr0054_drift_message(code)
                .expect("Cargo drift diagnostic is closed")
                .to_owned()
                .into());
        }
        let side_lock = root.join(format!("bazel/cargo/{name}.lock", name = name));
        let Ok(side_text) = fs::read_to_string(&side_lock) else {
            if require_hub_locks {
                let code = if *name == "product" {
                    "D2B-BZLDRIFT-PRODUCT-HUB"
                } else {
                    "D2B-BZLDRIFT-WALKER-HUB"
                };
                return Err(adr0054_drift_message(code)
                    .expect("hub drift diagnostic is closed")
                    .into());
            }
            continue;
        };
        if side_text.trim().is_empty() {
            let code = if *name == "product" {
                "D2B-BZLDRIFT-PRODUCT-HUB"
            } else {
                "D2B-BZLDRIFT-WALKER-HUB"
            };
            return Err(adr0054_drift_message(code)
                .expect("hub drift diagnostic is closed")
                .into());
        }
        if let Some(recorded) = recorded_lock_digest(&side_text) {
            let actual = sha256_hex(lock_text.as_bytes());
            if recorded != actual {
                let code = if *name == "product" {
                    "D2B-BZLDRIFT-PRODUCT-HUB"
                } else {
                    "D2B-BZLDRIFT-WALKER-HUB"
                };
                return Err(adr0054_drift_message(code)
                    .expect("hub drift diagnostic is closed")
                    .into());
            }
        }
    }

    validate_toolchains(root)
}

fn validate_toolchains(root: &Path) -> Result<(), Box<dyn Error>> {
    let stable = fs::read_to_string(root.join("packages/rust-toolchain.toml"))
        .map_err(|_| "stable Rust toolchain file is unreadable")?;
    let nightly = fs::read_to_string(root.join("packages/d2b-api-surface/rust-toolchain.toml"))
        .map_err(|_| "nightly Rust toolchain file is unreadable")?;
    let stable_channel = channel(&stable).ok_or("stable Rust toolchain channel is missing")?;
    let nightly_channel = channel(&nightly).ok_or("nightly Rust toolchain channel is missing")?;
    if stable_channel != STABLE_TOOLCHAIN {
        return Err(format!(
            "stable Rust toolchain mismatch: expected {STABLE_TOOLCHAIN}, found {stable_channel}"
        )
        .into());
    }
    if nightly_channel != NIGHTLY_TOOLCHAIN {
        return Err(format!(
            "nightly Rust toolchain mismatch: expected {NIGHTLY_TOOLCHAIN}, found {nightly_channel}"
        )
        .into());
    }
    Ok(())
}

fn channel(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("channel = "))
        .and_then(|value| value.trim().strip_prefix('"'))
        .and_then(|value| value.strip_suffix('"'))
        .map(str::to_owned)
}

fn discover_manifests(root: &Path) -> Result<Vec<ManifestInfo>, Box<dyn Error>> {
    let mut paths = BTreeSet::new();
    for (_, manifest, _) in HUBS {
        let relative = Path::new(manifest);
        let text = fs::read_to_string(root.join(relative))
            .map_err(|_| format!("Cargo manifest is unreadable: {manifest}"))?;
        if package_name(&text).is_some() {
            paths.insert(relative.to_string_lossy().into_owned());
            continue;
        }
        for member in workspace_members(&text) {
            let package = relative
                .parent()
                .ok_or("workspace Cargo manifest has no parent")?
                .join(member)
                .join("Cargo.toml");
            let package = normalize_relative(&package)?;
            paths.insert(package);
        }
    }
    paths
        .into_iter()
        .map(|relative| {
            let text = fs::read_to_string(root.join(&relative))
                .map_err(|_| format!("Cargo package manifest is unreadable: {relative}"))?;
            parse_manifest(&relative, &text, root)
        })
        .collect()
}

fn parse_manifest(relative: &str, text: &str, root: &Path) -> Result<ManifestInfo, Box<dyn Error>> {
    let package_name =
        package_name(text).ok_or_else(|| format!("Cargo package name is missing in {relative}"))?;
    let sections = toml_sections(text);
    let lib_doctest = sections
        .iter()
        .find(|(name, _)| name == "[lib]")
        .map(|(_, block)| value_bool(block, "doctest").unwrap_or(true));
    let package_dir = Path::new(relative)
        .parent()
        .ok_or("Cargo package manifest has no parent")?
        .to_string_lossy()
        .into_owned();
    let lib = if root.join(&package_dir).join("src/lib.rs").is_file() {
        Some(TargetInfo {
            name: package_name.clone(),
            path: "src/lib.rs".to_owned(),
            kind: "lib".to_owned(),
            harness: None,
            doctest: lib_doctest,
            required_features: Vec::new(),
        })
    } else {
        None
    };
    let mut tests = target_sections(&sections, "[[test]]");
    let explicit_tests = tests
        .iter()
        .map(|target| target.path.clone())
        .collect::<BTreeSet<_>>();
    for target in implicit_test_targets(root, &package_dir)? {
        if !explicit_tests.contains(&target.path) {
            tests.push(target);
        }
    }
    let mut bins = target_sections(&sections, "[[bin]]");
    let explicit_bin_paths = bins
        .iter()
        .map(|target| target.path.clone())
        .collect::<BTreeSet<_>>();
    if root.join(&package_dir).join("src/main.rs").is_file()
        && !explicit_bin_paths.contains("src/main.rs")
    {
        bins.push(TargetInfo {
            name: package_name.clone(),
            path: "src/main.rs".to_owned(),
            kind: "bin".to_owned(),
            harness: None,
            doctest: Some(false),
            required_features: Vec::new(),
        });
    }
    for target in implicit_bin_targets(root, &package_dir)? {
        if !explicit_bin_paths.contains(&target.path) {
            bins.push(target);
        }
    }
    let benches = {
        let mut targets = target_sections(&sections, "[[bench]]");
        let explicit = targets
            .iter()
            .map(|target| target.path.clone())
            .collect::<BTreeSet<_>>();
        for target in implicit_bench_targets(root, &package_dir)? {
            if !explicit.contains(&target.path) {
                targets.push(target);
            }
        }
        targets
    };
    let examples = implicit_example_targets(root, &package_dir)?;
    let feature_dependencies = sections
        .iter()
        .find(|(name, _)| name == "[features]")
        .map(|(_, block)| feature_map(block))
        .unwrap_or_default();
    let default_features = feature_dependencies
        .get("default")
        .cloned()
        .unwrap_or_default();
    Ok(ManifestInfo {
        relative: relative.to_owned(),
        package_dir,
        package_name,
        lib_doctest,
        lib,
        tests,
        bins,
        benches,
        examples,
        default_features,
        feature_dependencies,
    })
}

fn implicit_test_targets(
    root: &Path,
    package_dir: &str,
) -> Result<Vec<TargetInfo>, Box<dyn Error>> {
    let tests_dir = root.join(package_dir).join("tests");
    if !tests_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut targets = Vec::new();
    for entry in fs::read_dir(tests_dir).map_err(|_| "Cargo test target directory is unreadable")? {
        let entry = entry.map_err(|_| "Cargo test target entry is unreadable")?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|_| "Cargo test target type is unavailable")?
            .is_file()
            || path.extension().is_none_or(|ext| ext != "rs")
        {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let name = stem.to_owned();
        targets.push(TargetInfo {
            name,
            path: format!("tests/{}.rs", stem),
            kind: "test".to_owned(),
            harness: Some(true),
            doctest: Some(false),
            required_features: Vec::new(),
        });
    }
    targets.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(targets)
}

fn target_sections(sections: &[(String, String)], target_kind: &str) -> Vec<TargetInfo> {
    sections
        .iter()
        .filter(|(name, _)| name == target_kind)
        .filter_map(|(_, block)| {
            let name = value_string(block, "name")?;
            let default_path = match target_kind {
                "[[bin]]" => format!("src/bin/{name}.rs"),
                "[[bench]]" => format!("benches/{name}.rs"),
                _ => format!("tests/{name}.rs"),
            };
            let path = value_string(block, "path").unwrap_or(default_path);
            Some(TargetInfo {
                name,
                path,
                kind: target_kind
                    .trim_matches(|character| character == '[' || character == ']')
                    .trim_start_matches("[]")
                    .to_owned(),
                harness: Some(value_bool(block, "harness").unwrap_or(true)),
                doctest: None,
                required_features: value_strings(block, "required-features"),
            })
        })
        .collect()
}

fn implicit_bin_targets(root: &Path, package_dir: &str) -> Result<Vec<TargetInfo>, Box<dyn Error>> {
    implicit_file_targets(root, package_dir, "src/bin", "bin")
}

fn implicit_bench_targets(
    root: &Path,
    package_dir: &str,
) -> Result<Vec<TargetInfo>, Box<dyn Error>> {
    implicit_file_targets(root, package_dir, "benches", "bench")
}

fn implicit_example_targets(
    root: &Path,
    package_dir: &str,
) -> Result<Vec<TargetInfo>, Box<dyn Error>> {
    implicit_file_targets(root, package_dir, "examples", "example")
}

fn implicit_file_targets(
    root: &Path,
    package_dir: &str,
    relative_dir: &str,
    kind: &str,
) -> Result<Vec<TargetInfo>, Box<dyn Error>> {
    let directory = root.join(package_dir).join(relative_dir);
    if !directory.is_dir() {
        return Ok(Vec::new());
    }
    let mut targets = Vec::new();
    for entry in
        fs::read_dir(directory).map_err(|_| "implicit Cargo target directory is unreadable")?
    {
        let entry = entry.map_err(|_| "implicit Cargo target entry is unreadable")?;
        let path = entry.path();
        if !entry
            .file_type()
            .map_err(|_| "implicit Cargo target type is unavailable")?
            .is_file()
            || path.extension().is_none_or(|ext| ext != "rs")
        {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        targets.push(TargetInfo {
            name: stem.to_owned(),
            path: format!("{relative_dir}/{stem}.rs"),
            kind: kind.to_owned(),
            harness: Some(true),
            doctest: Some(false),
            required_features: Vec::new(),
        });
    }
    targets.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(targets)
}

fn feature_map(block: &str) -> BTreeMap<String, Vec<String>> {
    block
        .lines()
        .filter_map(|line| {
            let (name, value) = line.trim().split_once('=')?;
            Some((name.trim().to_owned(), quoted_values(value)))
        })
        .collect()
}

fn value_strings(block: &str, key: &str) -> Vec<String> {
    block
        .lines()
        .find_map(|line| {
            let (name, value) = line.trim().split_once('=')?;
            (name.trim() == key).then(|| quoted_values(value))
        })
        .unwrap_or_default()
}

fn bazelignore_entries(
    root: &Path,
    manifests: &[ManifestInfo],
) -> Result<Vec<String>, Box<dyn Error>> {
    let mut entries = BTreeSet::from([".scratch/".to_owned()]);
    entries.insert("packages/target/".to_owned());
    entries.insert("packages/d2b-priv-broker/target-layer1/".to_owned());
    entries.insert("packages/d2b-priv-broker/target-fakebackends/".to_owned());
    entries.insert("proofs/target/".to_owned());
    entries.insert("labs/target/".to_owned());
    for manifest in manifests {
        entries.insert(format!("{}/target/", manifest.package_dir));
    }
    let mut cargo_manifests = Vec::new();
    collect_named_files(root, "Cargo.toml", &mut cargo_manifests)?;
    for manifest in cargo_manifests {
        let relative = manifest
            .strip_prefix(root)
            .map_err(|_| "Cargo manifest escaped repository root")?;
        let relative = relative
            .parent()
            .ok_or("Cargo manifest has no parent")?
            .to_string_lossy()
            .replace('\\', "/");
        if !relative.is_empty() {
            entries.insert(format!("{relative}/target/"));
        }
    }
    Ok(entries.into_iter().collect())
}

fn collect_named_files(
    root: &Path,
    name: &str,
    output: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    if !root.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|_| "Cargo manifest census directory is unreadable")? {
        let entry = entry.map_err(|_| "Cargo manifest census entry is unreadable")?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_| "Cargo manifest census entry type is unavailable")?;
        if file_type.is_dir()
            && !matches!(
                entry.file_name().to_str(),
                Some(".git" | ".scratch" | "target")
            )
        {
            collect_named_files(&path, name, output)?;
        } else if file_type.is_file() && entry.file_name() == name {
            output.push(path);
        }
    }
    Ok(())
}

fn package_names(root: &Path, manifest_relative: &str, manifest: &str) -> Vec<String> {
    if let Some(name) = package_name(manifest) {
        return vec![name];
    }
    workspace_members(manifest)
        .into_iter()
        .filter_map(|member| {
            let path = root
                .join(Path::new(manifest_relative).parent()?)
                .join(member)
                .join("Cargo.toml");
            fs::read_to_string(path)
                .ok()
                .and_then(|text| package_name(&text))
        })
        .collect()
}

fn package_name(text: &str) -> Option<String> {
    toml_sections(text)
        .into_iter()
        .find(|(name, _)| name == "[package]")
        .and_then(|(_, block)| value_string(&block, "name"))
}

fn workspace_members(text: &str) -> Vec<String> {
    let Some(start) = text.find("members") else {
        return Vec::new();
    };
    let remainder = &text[start..];
    let Some(open) = remainder.find('[') else {
        return Vec::new();
    };
    let remainder = &remainder[open..];
    let Some(close) = remainder.find(']') else {
        return Vec::new();
    };
    quoted_values(&remainder[..=close])
}

fn toml_sections(text: &str) -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current = String::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            if let Some(name) = current_name.take() {
                sections.push((name, current.clone()));
            }
            current.clear();
            current_name = Some(trimmed.to_owned());
        } else if current_name.is_some() {
            current.push_str(line);
            current.push('\n');
        }
    }
    if let Some(name) = current_name {
        sections.push((name, current));
    }
    sections
}

fn value_string(block: &str, key: &str) -> Option<String> {
    block.lines().find_map(|line| {
        let line = line.trim();
        let (name, value) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }
        let value = value.trim();
        let value = value.strip_prefix('"')?;
        let end = value.find('"')?;
        Some(value[..end].to_owned())
    })
}

fn value_bool(block: &str, key: &str) -> Option<bool> {
    block.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        if name.trim() != key {
            return None;
        }
        match value.trim() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    })
}

fn quoted_values(text: &str) -> Vec<String> {
    let mut values = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('"') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('"') else {
            break;
        };
        values.push(rest[..end].to_owned());
        rest = &rest[end + 1..];
    }
    values
}

fn lock_packages(text: &str) -> Vec<(String, String)> {
    text.split("[[package]]")
        .skip(1)
        .filter_map(|block| {
            let name = value_string(block, "name")?;
            let version = value_string(block, "version")?;
            Some((name, version))
        })
        .collect()
}

fn recorded_lock_digest(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        if !line.contains("cargo_lock_sha256") && !line.contains("cargo-lock-sha256") {
            return None;
        }
        let value = line
            .split(|character: char| !character.is_ascii_hexdigit())
            .find(|part| part.len() == 64)?;
        Some(value.to_ascii_lowercase())
    })
}

fn normalize_relative(path: &Path) -> Result<String, Box<dyn Error>> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            std::path::Component::Normal(value) => components.push(value.to_string_lossy()),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if components.pop().is_none() {
                    return Err("workspace member escapes repository root".into());
                }
            }
            _ => return Err("workspace member is not a relative path".into()),
        }
    }
    Ok(components.join("/"))
}

fn bazel_string(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn is_generator_owned(path: &str) -> bool {
    APPROVED_OUTPUT_PATHS.contains(&path)
}

fn tracked_paths(root: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|_| "could not start the tracked-file census")?;
    if !output.status.success() {
        let mut files = Vec::new();
        collect_files_without_git(root, root, &mut files)?;
        files.sort();
        return Ok(files);
    }
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| {
            String::from_utf8(entry.to_vec()).map_err(|_| "tracked path is not valid UTF-8".into())
        })
        .collect()
}

fn collect_files_without_git(
    root: &Path,
    current: &Path,
    output: &mut Vec<String>,
) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(current).map_err(|_| "fallback file census is unreadable")? {
        let entry = entry.map_err(|_| "fallback file census entry is unreadable")?;
        let name = entry.file_name();
        if matches!(name.to_str(), Some(".git" | ".scratch" | "target")) {
            continue;
        }
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|_| "fallback file census entry type is unavailable")?;
        if file_type.is_dir() {
            collect_files_without_git(root, &path, output)?;
        } else if file_type.is_file() {
            output.push(
                path.strip_prefix(root)
                    .map_err(|_| "fallback tracked path escaped root")?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

fn mutation_snapshot(root: &Path) -> Result<BTreeMap<String, [u8; 32]>, Box<dyn Error>> {
    let mut paths = Vec::new();
    collect_files_without_git(root, root, &mut paths)?;
    paths.sort();
    let mut digests = BTreeMap::new();
    for relative in paths {
        let bytes =
            fs::read(root.join(&relative)).map_err(|_| "mutation snapshot entry is unreadable")?;
        digests.insert(relative, Sha256::digest(bytes).into());
    }
    Ok(digests)
}

fn changed_outside(
    before: &BTreeMap<String, [u8; 32]>,
    after: &BTreeMap<String, [u8; 32]>,
    allowed: Option<&str>,
) -> Vec<String> {
    let paths = before
        .keys()
        .chain(after.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    paths
        .into_iter()
        .filter(|path| Some(path.as_str()) != allowed)
        .filter(|path| before.get(path) != after.get(path))
        .collect()
}

fn unexpected_mutation_message(paths: &[String]) -> String {
    let mut message = adr0054_drift_message("D2B-BZL-UNEXPECTED-MUTATION")
        .expect("unexpected mutation diagnostic is closed")
        .to_owned();
    if !paths.is_empty() {
        message.push_str("\nChanged paths: ");
        message.push_str(&paths.join(", "));
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn repin_accepts_only_the_closed_hub_set() {
        for hub in ["product", "walker"] {
            assert_eq!(parse_repin(&["--hub".into(), hub.into()]).unwrap(), hub);
        }
        for hub in ["", "all", "workspace", "main", "broker", "guest"] {
            assert!(
                parse_repin(&["--hub".into(), hub.into()]).is_err(),
                "unexpectedly accepted hub {hub:?}"
            );
        }
    }

    #[test]
    fn rejected_hubs_are_bounded_and_redacted_in_diagnostics() {
        let retired = parse_repin(&["--hub".into(), "main".into()])
            .expect_err("retired hub")
            .to_string();
        assert!(retired.contains("D2B-BZL-RETIRED-HUB"));
        assert!(retired.contains("Supplied hub: main"));

        let invalid = parse_repin(&["--hub".into(), "workspace".into()])
            .expect_err("invalid hub")
            .to_string();
        assert!(invalid.contains("supplied hub workspace"));

        let sensitive = parse_repin(&["--hub".into(), "/home/operator/private".into()])
            .expect_err("sensitive hub")
            .to_string();
        assert!(sensitive.contains("supplied hub <redacted>"));
        assert!(!sensitive.contains("/home/operator"));

        let long = "a".repeat(MAX_SELECTOR_DIAGNOSTIC_BYTES + 40);
        let bounded = parse_repin(&["--hub".into(), long.clone()])
            .expect_err("long hub")
            .to_string();
        assert!(bounded.contains("...[truncated]"));
        assert!(!bounded.contains(&long));
    }

    #[test]
    fn startup_options_are_absolute_and_worktree_derived() {
        let options = startup_options(Path::new("/worktree"));
        assert!(options.output_user_root.is_absolute());
        assert!(options.output_base.is_absolute());
        assert!(options.output_user_root.starts_with("/worktree"));
        assert!(options.output_base.starts_with("/worktree"));
        assert_eq!(
            options.startup_args(),
            vec![
                "--output_user_root=/worktree/.scratch/bazel/output-user-root",
                "--output_base=/worktree/.scratch/bazel/output-base",
            ]
        );
        assert_eq!(
            options.repin_command_args(true),
            vec![
                "mod".to_owned(),
                "deps".to_owned(),
                "--lockfile_mode=off".to_owned(),
            ]
        );
        assert_eq!(
            options.module_refresh_command_args(),
            vec![
                "mod".to_owned(),
                "deps".to_owned(),
                "--lockfile_mode=update".to_owned()
            ]
        );
    }

    #[test]
    fn generator_models_have_stable_order_and_exact_ownership() {
        let model = GeneratedModel {
            bazelignore: vec!["target/".into(), ".scratch/".into()],
            action_network_policy: "{}\n".into(),
            configured_targets: "{}\n".into(),
            evidence_sink_policy: "{}\n".into(),
            no_shell_inventory: "{}\n".into(),
            package_policy_targets: "PACKAGE_POLICY_TARGETS = []\n".into(),
            product_targets: "PRODUCT_TARGETS = []\n".into(),
            source_census: "{}\n".into(),
        };
        let outputs = model.render().expect("rendered model");
        assert_eq!(outputs.len(), APPROVED_OUTPUT_PATHS.len());
        assert_eq!(
            outputs.keys().cloned().collect::<Vec<_>>(),
            APPROVED_OUTPUT_PATHS
                .iter()
                .map(|path| (*path).to_owned())
                .collect::<Vec<_>>()
        );
        assert!(outputs["bazel/generated/BUILD.bazel"].contains("exports_files"));
        assert!(
            !outputs
                .keys()
                .any(|path| path.contains("action-network-inventory"))
        );
    }

    #[test]
    fn derived_change_check_allows_only_the_named_hub_lock() {
        let mut before = BTreeMap::new();
        before.insert("bazel/cargo/product.lock".to_owned(), [1; 32]);
        before.insert("Cargo.lock".to_owned(), [2; 32]);
        let mut after = before.clone();
        after.insert("bazel/cargo/product.lock".to_owned(), [3; 32]);
        assert_eq!(
            changed_outside(&before, &after, Some("bazel/cargo/product.lock")),
            Vec::<String>::new()
        );
        after.insert("Cargo.lock".to_owned(), [4; 32]);
        assert_eq!(
            changed_outside(&before, &after, Some("bazel/cargo/product.lock")),
            vec!["Cargo.lock".to_owned()]
        );
    }

    #[test]
    fn census_records_excluded_entries_without_hand_written_counts() {
        let census = Census::new(
            vec!["packages/d2b-core/Cargo.toml#d2b-core-smoke".into()],
            vec![(
                "packages/d2b-core/Cargo.toml#d2b-core-fuzz-manifest".into(),
                "required features are not enabled by the Cargo gate selector".into(),
            )],
        );
        let json = census.json();
        assert!(json.contains("\"executed\""));
        assert!(json.contains("\"outOfCensus\""));
        assert!(json.contains("d2b-core-fuzz-manifest"));
        assert!(!json.contains("\"count\""));
    }

    #[test]
    fn generator_input_set_is_the_product_and_walker_hubs() {
        assert_eq!(HUBS.len(), 2);
        assert_eq!(
            HUBS.iter()
                .map(|(_, manifest, lock)| (*manifest, *lock))
                .collect::<Vec<_>>(),
            vec![
                ("packages/Cargo.toml", "packages/Cargo.lock"),
                (
                    "tests/tools/no-bash-ast-walker/Cargo.toml",
                    "tests/tools/no-bash-ast-walker/Cargo.lock"
                ),
            ]
        );
    }

    #[test]
    fn metadata_parser_derives_harness_and_doctest_census_entries() {
        let manifest = r#"
[package]
name = "sample"

[lib]
doctest = true

[[test]]
name = "run"
path = "tests/run.rs"
harness = false

[[test]]
name = "fuzz"
path = "fuzz.rs"
harness = false
required-features = ["fuzz"]

[[bench]]
name = "bench"
harness = false
"#;
        let parsed =
            parse_manifest("packages/sample/Cargo.toml", manifest, Path::new(".")).unwrap();
        assert_eq!(parsed.lib_doctest, Some(true));
        assert_eq!(parsed.tests.len(), 2);
        assert_eq!(parsed.benches.len(), 1);
        let model = GeneratedModel {
            bazelignore: vec![".scratch/".into()],
            action_network_policy: "{}\n".into(),
            configured_targets: "{}\n".into(),
            evidence_sink_policy: "{}\n".into(),
            no_shell_inventory: "{}\n".into(),
            package_policy_targets: "PACKAGE_POLICY_TARGETS = []\n".into(),
            product_targets: "PRODUCT_TARGETS = []\n".into(),
            source_census: "{}\n".into(),
        };
        assert!(model.validate().is_ok());
    }

    #[test]
    fn target_required_features_select_only_that_targets_optional_dependencies() {
        let manifest = ManifestInfo {
            relative: "packages/sample/Cargo.toml".to_owned(),
            package_dir: "packages/sample".to_owned(),
            package_name: "sample".to_owned(),
            lib_doctest: Some(true),
            lib: None,
            tests: Vec::new(),
            bins: Vec::new(),
            benches: Vec::new(),
            examples: Vec::new(),
            default_features: Vec::new(),
            feature_dependencies: BTreeMap::from([(
                "fuzz".to_owned(),
                vec!["dep:bolero".to_owned()],
            )]),
        };
        let package = DependencyInfo {
            package_name: "sample".to_owned(),
            package_dir: "packages/sample".to_owned(),
            hub: "product".to_owned(),
            normal: vec!["bolero".to_owned()],
            dev: Vec::new(),
            optional: BTreeSet::from(["bolero".to_owned()]),
            proc_macro: BTreeSet::new(),
            target_conditions: BTreeMap::new(),
        };
        let dependencies = BTreeMap::new();
        let library_features = effective_target_features(&manifest.default_features, &[]);
        let (_, library_external, _) =
            direct_dependency_sets(&manifest, &package, &dependencies, &library_features, false);
        assert!(library_external.is_empty());

        let fuzz_features =
            effective_target_features(&manifest.default_features, &["fuzz".to_owned()]);
        let (_, fuzz_external, _) =
            direct_dependency_sets(&manifest, &package, &dependencies, &fuzz_features, false);
        assert_eq!(fuzz_features, vec!["fuzz"]);
        assert_eq!(fuzz_external, vec!["@product//:bolero"]);
    }

    #[test]
    fn parser_and_schema_errors_do_not_emit_raw_diagnostics() {
        let malformed = validate_json_schema(
            "configured-targets.json",
            r#"{"private":"/home/operator/private""#,
            &["schemaVersion"],
        )
        .expect_err("malformed JSON")
        .to_string();
        assert_eq!(malformed, "configured-targets.json is not valid JSON");
        assert!(!malformed.contains("/home/operator"));
        assert!(!malformed.contains("column"));

        let wrong_schema = validate_json_schema(
            "configured-targets.json",
            r#"{"private":"/home/operator/private"}"#,
            &["schemaVersion"],
        )
        .expect_err("wrong schema")
        .to_string();
        assert_eq!(
            wrong_schema,
            "configured-targets.json schema differs from the closed generator contract"
        );
        assert!(!wrong_schema.contains("private"));
        assert!(!wrong_schema.contains('{'));
    }

    #[test]
    fn stale_side_lock_digest_is_detectable_without_rewriting_inputs() {
        let expected = sha256_hex(b"cargo-lock");
        assert_eq!(
            recorded_lock_digest(&format!("# cargo_lock_sha256: {expected}\n")),
            Some(expected)
        );
        assert_ne!(
            recorded_lock_digest(
                "# cargo_lock_sha256: 0000000000000000000000000000000000000000000000000000000000000000\n"
            ),
            Some(sha256_hex(b"cargo-lock"))
        );
        assert_eq!(channel("channel = \"1.97.0\"\n"), Some("1.97.0".to_owned()));
        assert_eq!(
            channel("channel = \"nightly-2026-02-16\"\n"),
            Some("nightly-2026-02-16".to_owned())
        );
    }

    #[test]
    fn empty_or_incomplete_bazelignore_models_are_rejected() {
        let mut model = GeneratedModel {
            bazelignore: vec!["packages/target/".into()],
            action_network_policy: "{}\n".into(),
            configured_targets: "{}\n".into(),
            evidence_sink_policy: "{}\n".into(),
            no_shell_inventory: "{}\n".into(),
            package_policy_targets: "PACKAGE_POLICY_TARGETS = []\n".into(),
            product_targets: "PRODUCT_TARGETS = []\n".into(),
            source_census: "{}\n".into(),
        };
        assert!(model.validate().is_err());
        model.bazelignore.push(".scratch/".into());
        assert!(model.validate().is_ok());
    }

    #[test]
    fn committed_generator_census_rejects_missing_extra_nonregular_and_absent_roots() {
        let root = std::env::temp_dir().join(format!("d2b-bazel-census-{}", std::process::id()));
        let generated = root.join("bazel/generated");
        fs::create_dir_all(&generated).expect("generated root");
        fs::write(root.join(".bazelignore"), ".scratch/\n").expect("bazelignore");
        fs::write(generated.join("one.json"), "{}\n").expect("generated output");
        let expected = BTreeMap::from([
            (".bazelignore".to_owned(), ".scratch/\n".to_owned()),
            ("bazel/generated/one.json".to_owned(), "{}\n".to_owned()),
        ]);
        validate_committed_output_census(&root, &expected).expect("exact census");

        fs::remove_file(generated.join("one.json")).expect("remove expected output");
        assert!(
            validate_committed_output_census(&root, &expected).is_err(),
            "missing output must fail closed"
        );
        fs::write(generated.join("one.json"), "{}\n").expect("restore output");

        fs::write(generated.join("stale.json"), "{}\n").expect("extra output");
        let extra = validate_committed_output_census(&root, &expected)
            .expect_err("extra output must fail closed");
        assert!(extra.to_string().contains("stale.json"));
        fs::remove_file(generated.join("stale.json")).expect("remove extra output");

        fs::remove_file(generated.join("one.json")).expect("remove output for nonregular test");
        fs::create_dir(generated.join("one.json")).expect("replace output with directory");
        assert!(
            validate_committed_output_census(&root, &expected).is_err(),
            "nonregular output must fail closed"
        );
        fs::remove_dir_all(&root).expect("remove census root");
        assert!(
            validate_committed_output_census(&root, &expected).is_err(),
            "absent output root must fail closed"
        );
    }

    #[test]
    fn child_command_diagnostics_are_bounded_redacted_and_hub_specific() {
        let diagnostic = bounded_child_diagnostic(
            b"fatal: path=/home/operator/private /nix/store/secret\nsecond-line",
        );
        assert!(!diagnostic.contains("/home/operator"));
        assert!(!diagnostic.contains("/nix/store"));
        assert!(diagnostic.contains("<path>"));
        assert!(
            command_failure_message(
                "product",
                &["mod".into(), "deps".into()],
                "36",
                Some(&diagnostic)
            )
            .contains("hub=product command=bazel mod deps status=36")
        );
        let unicode = "x".repeat(MAX_CHILD_DIAGNOSTIC_BYTES - 1) + "\u{00e9}";
        let bounded = bounded_child_diagnostic(unicode.as_bytes());
        assert!(bounded.ends_with("...[truncated]"));
    }

    #[cfg(unix)]
    #[test]
    fn atomic_output_refuses_symlinked_parent_and_anchored_cleanup() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("d2b-bazel-anchored-output-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let real = root.join("real");
        fs::create_dir_all(&real).expect("real output");
        let link = root.join("link");
        symlink(&real, &link).expect("output symlink");
        assert!(atomic_write_file(&link.join("output"), "unsafe").is_err());
        assert!(remove_anchored_directory(&link).is_err());
        fs::remove_dir_all(&root).expect("cleanup");
    }
}
