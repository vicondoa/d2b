#![allow(dead_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    error::Error,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::hermeticity;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum GenBazelMode {
    Write,
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
const PREVIEW_ROOT: &str = ".scratch/bazel/generated-preview";

const RETIRED_HUB_MESSAGES: &[(&str, &str)] = &[
    (
        "main",
        "Hub 'main' is retired; after entering nix develop, run from packages/: cargo xtask bazel-repin --hub product",
    ),
    (
        "broker",
        "Hub 'broker' is retired; after entering nix develop, run from packages/: cargo xtask bazel-repin --hub product",
    ),
    (
        "guest",
        "Hub 'guest' is retired; after entering nix develop, run from packages/: cargo xtask bazel-repin --hub product",
    ),
];

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
cargo xtask gen-bazel
Review and commit the listed repository-relative generated paths.
Rerun cargo xtask gen-bazel --check, then rerun the failed command.",
        "D2B-BZLDRIFT-PACKAGE-POLICY" => "\
D2B-BZLDRIFT-PACKAGE-POLICY: package-policy output is stale.
From the repository root, run: nix develop
Then run: cd packages
cargo xtask gen-package-policy-inputs
Review and commit the generated changes under packages/policy-inputs/.
Rerun cargo xtask gen-package-policy-inputs --check, then rerun the failed command.",
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

struct ProcessExecutor;

impl BazelExecutor for ProcessExecutor {
    fn run(
        &mut self,
        root: &Path,
        startup_args: &[String],
        command_args: &[String],
        environment: &[(&str, &str)],
    ) -> Result<std::process::ExitStatus, Box<dyn Error>> {
        let executable = env::var_os("BAZEL").unwrap_or_else(|| "bazel".into());
        let mut command = Command::new(executable);
        command
            .current_dir(root)
            .args(startup_args)
            .args(command_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (name, value) in environment {
            command.env(name, value);
        }
        command
            .status()
            .map_err(|error| format!("could not start the Bazel child: {error}").into())
    }
}

pub(crate) fn parse_gen_bazel(args: &[String]) -> Result<GenBazelMode, Box<dyn Error>> {
    match args {
        [] => Ok(GenBazelMode::Write),
        [flag] if flag == "--check" => Ok(GenBazelMode::Check),
        _ => Err("usage: gen-bazel [--check]".into()),
    }
}

pub(crate) fn gen_bazel(args: &[String]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mode = parse_gen_bazel(args)?;
    let root = repo_root()?;
    let model = generate_model(&root)?;
    model.validate()?;
    let rendered = model.render();
    let paths = rendered.keys().map(PathBuf::from).collect::<Vec<_>>();

    match mode {
        GenBazelMode::Write => {
            // This wave is allowed to leave a reviewable preview only.  The
            // integrator owns generated BUILD files, inventories, pins, and
            // goldens, so a contributor command must not silently create one.
            let preview_root = root.join(PREVIEW_ROOT);
            for (relative, contents) in rendered {
                let path = preview_root.join(&relative);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(path, contents)?;
            }
            Ok(paths
                .into_iter()
                .map(|relative| PathBuf::from(PREVIEW_ROOT).join(relative))
                .collect())
        }
        GenBazelMode::Check => {
            for (relative, expected) in rendered {
                let path = root.join(&relative);
                if path.is_file() {
                    let actual = fs::read_to_string(&path).map_err(|_error| {
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
            }
            // A check is intentionally read-only.  It does not compare or
            // delete a stale output outside this scope and never creates a
            // generated repository artifact.
            Ok(paths
                .into_iter()
                .map(|relative| PathBuf::from(PREVIEW_ROOT).join(relative))
                .collect())
        }
    }
}

pub(crate) fn parse_repin(args: &[String]) -> Result<&str, Box<dyn Error>> {
    match args {
        [flag, hub] if flag == "--hub" && HUBS.iter().any(|(name, _, _)| name == hub) => {
            Ok(hub.as_str())
        }
        [flag, hub]
            if flag == "--hub" && RETIRED_HUB_MESSAGES.iter().any(|(name, _)| name == hub) =>
        {
            Err(RETIRED_HUB_MESSAGES
                .iter()
                .find(|(name, _)| name == hub)
                .map(|(_, message)| *message)
                .unwrap_or("retired Bazel hub")
                .into())
        }
        [flag, hub] if flag == "--hub" && !hub.is_empty() => {
            Err(format!("unknown Bazel dependency hub `{hub}`; expected product or walker").into())
        }
        _ => Err("usage: bazel-repin --hub <name>".into()),
    }
}

pub(crate) fn bazel_repin(args: &[String]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let hub = parse_repin(args)?;
    reject_ambient_repin("bazel-repin", Some(hub))?;
    let root = repo_root()?;
    let mut executor = ProcessExecutor;
    bazel_repin_with_executor(&root, hub, &mut executor)
}

pub fn bazel_repin_with_executor(
    root: &Path,
    hub: &str,
    executor: &mut dyn BazelExecutor,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    if !HUBS.iter().any(|(name, _, _)| *name == hub) {
        if let Some((_, message)) = RETIRED_HUB_MESSAGES.iter().find(|(name, _)| *name == hub) {
            return Err((*message).into());
        }
        return Err(
            format!("unknown Bazel dependency hub `{hub}`; expected product or walker").into(),
        );
    }
    reject_ambient_repin("bazel-repin", Some(hub))?;
    let before = mutation_snapshot(root)?;
    let options = startup_options(root);
    let command = options.repin_command_args(!root.join("MODULE.bazel.lock").is_file());
    let status = executor.run(
        root,
        &options.startup_args(),
        &command,
        &[("CARGO_BAZEL_REPIN", "1"), ("CARGO_BAZEL_REPIN_ONLY", hub)],
    )?;
    let after = mutation_snapshot(root)?;
    let lock = format!("bazel/cargo/{hub}.lock");
    let outside = changed_outside(&before, &after, Some(&lock));
    if !outside.is_empty() {
        return Err(unexpected_mutation_message(&outside).into());
    }
    if !status.success() {
        let code = if hub == "product" {
            "D2B-BZLDRIFT-PRODUCT-HUB"
        } else {
            "D2B-BZLDRIFT-WALKER-HUB"
        };
        return Err(adr0054_drift_message(code)
            .expect("hub diagnostic is closed")
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
    let mut executor = ProcessExecutor;
    bazel_module_refresh_with_executor(&root, &mut executor)
}

pub fn bazel_module_refresh_with_executor(
    root: &Path,
    executor: &mut dyn BazelExecutor,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    reject_ambient_repin("bazel-module-refresh", None)?;
    let before = mutation_snapshot(root)?;
    let options = startup_options(root);
    let status = executor.run(
        root,
        &options.startup_args(),
        &options.module_refresh_command_args(),
        &[],
    )?;
    let after = mutation_snapshot(root)?;
    let outside = changed_outside(&before, &after, Some("MODULE.bazel.lock"));
    if !outside.is_empty() {
        return Err(unexpected_mutation_message(&outside).into());
    }
    if !status.success() {
        return Err(adr0054_drift_message("D2B-BZLDRIFT-MODULE")
            .expect("module diagnostic is closed")
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
    symlink_prefix: PathBuf,
}

impl StartupOptions {
    fn startup_args(&self) -> Vec<String> {
        vec![
            format!("--output_user_root={}", self.output_user_root.display()),
            format!("--output_base={}", self.output_base.display()),
            format!("--symlink_prefix={}/", self.symlink_prefix.display()),
        ]
    }

    fn repin_command_args(&self, fresh_tree: bool) -> Vec<String> {
        let mut args = vec![
            "run".to_owned(),
            "@rules_rust//crate_universe:cargo-bazel".to_owned(),
            "--".to_owned(),
            "generate".to_owned(),
        ];
        if fresh_tree {
            args.insert(1, "--lockfile_mode=off".to_owned());
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
        symlink_prefix: scratch.join("symlinks"),
    }
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
        return fs::canonicalize(root)
            .map_err(|error| format!("cannot canonicalize D2B_BAZEL_WORKTREE: {error}").into());
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| "cannot locate repository root".into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct GeneratedBuild {
    path: String,
    content: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Census {
    executed: Vec<String>,
    out_of_census: Vec<(String, String)>,
}

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
    builds: Vec<GeneratedBuild>,
    governed_sources: Vec<String>,
    harness_free: Census,
    doctests: Census,
    bazelignore: Vec<String>,
    hermeticity: String,
}

impl GeneratedModel {
    fn validate(&self) -> Result<(), Box<dyn Error>> {
        if self.bazelignore.is_empty() || !self.bazelignore.iter().any(|entry| entry == ".scratch/")
        {
            return Err("generated .bazelignore must be nonempty and cover .scratch/".into());
        }
        let mut paths = BTreeSet::new();
        for build in &self.builds {
            if !is_generator_owned(&build.path) || !paths.insert(&build.path) {
                return Err(format!("generated BUILD ownership is invalid: {}", build.path).into());
            }
        }
        if self.governed_sources.is_empty() {
            return Err("generated governed Rust source inventory is empty".into());
        }
        if self.harness_free.executed.is_empty() {
            return Err("generated harness-free census is empty".into());
        }
        hermeticity::validate_action_network_inventory(
            &hermeticity::complete_action_network_inventory(),
        )
        .map_err(|error| format!("action-network inventory is invalid: {error}"))?;
        Ok(())
    }

    fn render(&self) -> BTreeMap<String, String> {
        let mut outputs = BTreeMap::new();
        let mut builds = self.builds.clone();
        builds.sort_by(|left, right| left.path.cmp(&right.path));
        for build in builds {
            outputs.insert(build.path, build.content);
        }

        let mut sources = self.governed_sources.clone();
        sources.sort();
        sources.dedup();
        let mut governed = String::from(
            "# Generated by cargo xtask gen-bazel. Do not edit.\n\nGOVERNED_RUST_SOURCES = [\n",
        );
        for source in sources {
            governed.push_str("    ");
            governed.push_str(&bazel_string(&source));
            governed.push_str(",\n");
        }
        governed.push_str("]\n");
        outputs.insert(
            "bazel/generated/governed-rust-sources.bzl".to_owned(),
            governed,
        );
        outputs.insert(
            "bazel/generated/harness-free-census.json".to_owned(),
            self.harness_free.json(),
        );
        outputs.insert(
            "bazel/generated/doctest-census.json".to_owned(),
            self.doctests.json(),
        );
        outputs.insert(
            hermeticity::GENERATED_ARTIFACT_PATH.to_owned(),
            self.hermeticity.clone(),
        );
        outputs.insert(
            "bazel/generated/action-network-inventory.json".to_owned(),
            serde_json::to_string_pretty(&hermeticity::complete_action_network_inventory())
                .expect("action-network inventory is serializable")
                + "\n",
        );

        let mut ignores = self.bazelignore.clone();
        ignores.sort();
        ignores.dedup();
        let mut bazelignore = String::new();
        for entry in ignores {
            bazelignore.push_str(entry.trim_end_matches('/'));
            bazelignore.push_str("/\n");
        }
        outputs.insert(".bazelignore".to_owned(), bazelignore);
        outputs
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ManifestInfo {
    relative: String,
    package_dir: String,
    package_name: String,
    lib_doctest: Option<bool>,
    tests: Vec<TargetInfo>,
    bins: Vec<TargetInfo>,
    benches: Vec<TargetInfo>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TargetInfo {
    name: String,
    path: String,
    harness: bool,
    required_features: bool,
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
}

fn generate_model(root: &Path) -> Result<GeneratedModel, Box<dyn Error>> {
    validate_generator_inputs(root)?;
    let hermeticity = hermeticity_artifact(root)?;
    let manifests = discover_manifests(root)?;
    if manifests.is_empty() {
        return Err("no Cargo package manifests were discovered".into());
    }
    let dependencies = dependency_graph(root)?;

    let mut builds = vec![render_workspace_build()];
    let mut harness_executed = Vec::new();
    let mut harness_out = Vec::new();
    let mut doctest_executed = Vec::new();
    let mut doctest_out = Vec::new();
    for manifest in &manifests {
        builds.push(render_build(manifest, root, &dependencies)?);
        for target in &manifest.tests {
            let entry = format!("{}#{}", manifest.relative, target.name);
            if target.harness && !target.required_features {
                continue;
            }
            if target.harness && target.required_features {
                continue;
            }
            if target.required_features {
                harness_out.push((
                    entry,
                    "required features are not enabled by the Cargo gate selector".to_owned(),
                ));
            } else {
                harness_executed.push(entry);
            }
        }
        for target in &manifest.benches {
            if !target.harness {
                harness_out.push((
                    format!("{}#{}", manifest.relative, target.name),
                    "bench targets are not selected by the harness-free test selector".to_owned(),
                ));
            }
        }
        match manifest.lib_doctest {
            Some(true) => doctest_executed.push(manifest.relative.clone()),
            Some(false) => doctest_out.push((
                manifest.relative.clone(),
                "the Cargo manifest disables doctests for this library".to_owned(),
            )),
            None => {}
        }
    }

    let governed_sources = governed_source_inventory(root)?;
    let bazelignore = bazelignore_entries(root, &manifests)?;
    Ok(GeneratedModel {
        builds,
        governed_sources,
        harness_free: Census::new(harness_executed, harness_out),
        doctests: Census::new(doctest_executed, doctest_out),
        bazelignore,
        hermeticity,
    })
}

fn dependency_graph(root: &Path) -> Result<BTreeMap<String, DependencyInfo>, Box<dyn Error>> {
    let mut graph = BTreeMap::new();
    for (hub, manifest, _) in HUBS {
        let output = Command::new("cargo")
            .current_dir(root)
            .args([
                "metadata",
                "--format-version",
                "1",
                "--locked",
                "--offline",
                "--manifest-path",
                manifest,
            ])
            .output()
            .map_err(|error| format!("could not run cargo metadata for {hub}: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "cargo metadata for {hub} failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )
            .into());
        }
        let document: Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| format!("cannot parse cargo metadata for {hub}: {error}"))?;
        let packages = document
            .get("packages")
            .and_then(Value::as_array)
            .ok_or_else(|| format!("cargo metadata for {hub} has no packages"))?;
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
                serde_json::from_str(&text).map_err(|error| {
                    format!(
                        "cannot parse Bazel-side lock for hermeticity inventory {name}: {error}"
                    )
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
        .map_err(|error| -> Box<dyn Error> { Box::new(error) })?;
    Ok(artifact.contents)
}

fn hermeticity_hub(name: &str) -> Result<hermeticity::Hub, Box<dyn Error>> {
    match name {
        "product" => Ok(hermeticity::Hub::Product),
        "walker" => Ok(hermeticity::Hub::Walker),
        _ => Err(format!("unknown hermeticity hub {name:?}").into()),
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
            .map_err(|error| format!("cannot read Cargo metadata root {manifest}: {error}"))?;
        let lock_text = fs::read_to_string(&lock_path)
            .map_err(|error| format!("cannot read Cargo lock {lock}: {error}"))?;
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
    let stable = fs::read_to_string(root.join("packages/rust-toolchain.toml"))?;
    let nightly = fs::read_to_string(root.join("packages/d2b-api-surface/rust-toolchain.toml"))?;
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
        let text = fs::read_to_string(root.join(relative))?;
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
            let text = fs::read_to_string(root.join(&relative))?;
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
    let mut tests = target_sections(&sections, "[[test]]");
    let package_dir = Path::new(relative)
        .parent()
        .ok_or("Cargo package manifest has no parent")?
        .to_string_lossy()
        .into_owned();
    let explicit_tests = tests
        .iter()
        .map(|target| target.path.clone())
        .collect::<BTreeSet<_>>();
    for target in implicit_test_targets(root, &package_dir)? {
        if !explicit_tests.contains(&target.path) {
            tests.push(target);
        }
    }
    Ok(ManifestInfo {
        relative: relative.to_owned(),
        package_dir,
        package_name,
        lib_doctest,
        tests,
        bins: target_sections(&sections, "[[bin]]"),
        benches: target_sections(&sections, "[[bench]]"),
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
    for entry in fs::read_dir(tests_dir)? {
        let entry = entry?;
        let path = entry.path();
        if !entry.file_type()?.is_file() || path.extension().is_none_or(|ext| ext != "rs") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
            continue;
        };
        let name = stem.to_owned();
        targets.push(TargetInfo {
            name,
            path: format!("tests/{}.rs", stem),
            harness: true,
            required_features: false,
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
                harness: value_bool(block, "harness").unwrap_or(true),
                required_features: block.contains("required-features"),
            })
        })
        .collect()
}

fn render_build(
    manifest: &ManifestInfo,
    root: &Path,
    dependencies: &BTreeMap<String, DependencyInfo>,
) -> Result<GeneratedBuild, Box<dyn Error>> {
    if manifest.relative == "packages/d2b-priv-broker/Cargo.toml" {
        return render_broker_build(manifest, root, dependencies);
    }
    if manifest.relative == "packages/d2b-guest-shell-runner/Cargo.toml" {
        return render_guest_build(manifest, root, dependencies);
    }

    let build_path = format!("{}/BUILD.bazel", manifest.package_dir);
    let mut source_files = Vec::new();
    let source_root = root.join(&manifest.package_dir);
    collect_rs_files(&source_root, &source_root, &mut source_files)?;
    source_files.sort();
    let package = dependencies
        .get(&manifest.relative)
        .ok_or_else(|| format!("missing dependency graph entry for {}", manifest.relative))?;
    let deps = dependency_labels(package, dependencies, &[]);
    let proc_macro_deps = proc_macro_dependency_labels(package, dependencies, &[], false);
    let mut content = String::from("# Generated by cargo xtask gen-bazel. Do not edit.\n");
    content.push_str("package(default_visibility = [\"//visibility:public\"])\n\n");
    content.push_str("load(\"@rules_rust//rust:defs.bzl\", \"rust_library\", \"rust_test\")\n\n");
    content.push_str("rust_library(\n");
    content.push_str(&format!(
        "    name = {},\n",
        bazel_string(&manifest.package_name)
    ));
    content.push_str(&format!(
        "    crate_name = {},\n",
        bazel_string(&rust_crate_name(&manifest.package_name))
    ));
    content.push_str("    edition = \"2024\",\n");
    content.push_str("    srcs = [\n");
    for source in source_files {
        content.push_str("        ");
        content.push_str(&bazel_string(&source));
        content.push_str(",\n");
    }

    content.push_str("    ],\n");
    append_deps(&mut content, &deps);
    append_proc_macro_deps(&mut content, &proc_macro_deps);
    content.push_str(")\n");
    for target in &manifest.bins {
        let target_name = if target.name == manifest.package_name {
            format!("{}-bin", target.name)
        } else {
            target.name.clone()
        };
        content.push_str("\nrust_test(\n");
        content.push_str(&format!("    name = {},\n", bazel_string(&target_name)));
        content.push_str("    edition = \"2024\",\n");
        content.push_str(&format!("    srcs = [{}],\n", bazel_string(&target.path)));
        append_deps(&mut content, &[format!(":{}", manifest.package_name)]);
        content.push_str(")\n");
    }
    for target in &manifest.tests {
        content.push_str("\nrust_test(\n");
        content.push_str(&format!("    name = {},\n", bazel_string(&target.name)));
        content.push_str("    edition = \"2024\",\n");
        content.push_str(&format!("    srcs = [{}],\n", bazel_string(&target.path)));
        append_deps(&mut content, &[format!(":{}", manifest.package_name)]);
        content.push_str(")\n");
    }
    if manifest.package_name == "d2b-core" {
        append_context_library(
            &mut content,
            &manifest.package_name,
            "d2b-core-test-support",
            "test-support",
            &source_files_for_package(root, &manifest.package_dir, true)?,
            &deps,
            &proc_macro_deps,
        );
    }
    if manifest.package_name == "d2b-host" {
        append_context_library(
            &mut content,
            &manifest.package_name,
            "d2b-host-fake-backends",
            "fake-backends",
            &source_files_for_package(root, &manifest.package_dir, true)?,
            &deps,
            &proc_macro_deps,
        );
    }
    Ok(GeneratedBuild {
        path: build_path,
        content,
    })
}

fn render_broker_build(
    manifest: &ManifestInfo,
    root: &Path,
    dependencies: &BTreeMap<String, DependencyInfo>,
) -> Result<GeneratedBuild, Box<dyn Error>> {
    let package = dependencies
        .get(&manifest.relative)
        .ok_or_else(|| format!("missing dependency graph entry for {}", manifest.relative))?;
    let sources = source_files_for_package(root, &manifest.package_dir, false)?;
    let normal_deps = dependency_labels(package, dependencies, &[]);
    let normal_proc_macro_deps = proc_macro_dependency_labels(package, dependencies, &[], false);
    let mut content = build_header();
    append_context_library(
        &mut content,
        &manifest.package_name,
        "d2b-priv-broker-lib",
        "",
        &sources,
        &normal_deps,
        &normal_proc_macro_deps,
    );
    append_binary(
        &mut content,
        "d2b-priv-broker",
        "d2b-priv-broker",
        "src/main.rs",
        &std::iter::once(":d2b-priv-broker-lib".to_owned())
            .chain(normal_deps.iter().cloned())
            .collect::<Vec<_>>(),
        &[],
    );

    let contexts = [
        ("", "d2b-priv-broker-default-lib", "tests", "tests"),
        (
            "layer1-bootstrap",
            "d2b-priv-broker-layer1-lib",
            "tests_layer1",
            "tests_layer1",
        ),
        (
            "fake-backends",
            "d2b-priv-broker-fakebackends-lib",
            "tests_fakebackends",
            "tests_fakebackends",
        ),
    ];
    let test_targets = manifest
        .tests
        .iter()
        .map(|target| target.name.clone())
        .collect::<Vec<_>>();
    for (feature, library, suite, _suite_alias) in contexts {
        let local_overrides = if feature == "fake-backends" {
            &[("d2b-host", "d2b-host-fake-backends")][..]
        } else {
            &[][..]
        };
        let context_deps = dependency_labels(package, dependencies, local_overrides);
        let context_proc_macro_deps =
            proc_macro_dependency_labels(package, dependencies, local_overrides, false);
        let test_overrides = local_overrides;
        let test_deps = test_dependency_labels(package, dependencies, test_overrides);
        let test_proc_macro_deps =
            proc_macro_dependency_labels(package, dependencies, test_overrides, false);
        append_context_library(
            &mut content,
            &manifest.package_name,
            library,
            feature,
            &sources,
            &context_deps,
            &context_proc_macro_deps,
        );
        for target in &manifest.tests {
            let target_name = if feature.is_empty() {
                format!("{}_default", target.name)
            } else {
                format!("{}_{}", target.name, feature.replace('-', "_"))
            };
            append_test(
                &mut content,
                &target_name,
                library,
                &target.path,
                feature,
                &test_deps,
                &test_proc_macro_deps,
                Some(("d2b-priv-broker", "CARGO_BIN_EXE_d2b-priv-broker")),
            );
        }
        content.push_str("\ntest_suite(\n");
        content.push_str(&format!("    name = {},\n", bazel_string(suite)));
        content.push_str("    tests = [\n");
        for target in &test_targets {
            content.push_str("        ");
            let target_name = if feature.is_empty() {
                format!("{}_default", target)
            } else {
                format!("{}_{}", target, feature.replace('-', "_"))
            };
            content.push_str(&bazel_string(&format!(":{}", target_name)));
            content.push_str(",\n");
        }
        content.push_str("    ],\n)\n");
    }

    Ok(GeneratedBuild {
        path: format!("{}/BUILD.bazel", manifest.package_dir),
        content,
    })
}

fn render_guest_build(
    manifest: &ManifestInfo,
    root: &Path,
    dependencies: &BTreeMap<String, DependencyInfo>,
) -> Result<GeneratedBuild, Box<dyn Error>> {
    let package = dependencies
        .get(&manifest.relative)
        .ok_or_else(|| format!("missing dependency graph entry for {}", manifest.relative))?;
    let sources = source_files_for_package(root, &manifest.package_dir, false)?;
    let default_deps = dependency_labels(package, dependencies, &[]);
    let default_proc_macro_deps = proc_macro_dependency_labels(package, dependencies, &[], false);
    let real_deps =
        dependency_labels_for_names(&package.normal, package, dependencies, &[], true, false);
    let real_proc_macro_deps = proc_macro_dependency_labels(package, dependencies, &[], true);
    let mut content = build_header();
    append_context_library(
        &mut content,
        &manifest.package_name,
        "d2b-guest-shell-runner-lib",
        "",
        &sources,
        &default_deps,
        &default_proc_macro_deps,
    );
    append_binary(
        &mut content,
        "d2b-guest-shell-runner",
        "d2b-guest-shell-runner",
        "src/main.rs",
        &std::iter::once(":d2b-guest-shell-runner-lib".to_owned())
            .chain(default_deps.iter().cloned())
            .collect::<Vec<_>>(),
        &[],
    );
    append_context_library(
        &mut content,
        &manifest.package_name,
        "d2b-guest-shell-runner-real-libshpool-lib",
        "real-libshpool",
        &sources,
        &real_deps,
        &real_proc_macro_deps,
    );
    append_binary(
        &mut content,
        "d2b-guest-shell-runner-real-libshpool",
        "d2b-guest-shell-runner",
        "src/main.rs",
        &std::iter::once(":d2b-guest-shell-runner-real-libshpool-lib".to_owned())
            .chain(real_deps.iter().cloned())
            .collect::<Vec<_>>(),
        &["real-libshpool".to_owned()],
    );
    for target in &manifest.tests {
        append_test(
            &mut content,
            &target.name,
            "d2b-guest-shell-runner-real-libshpool-lib",
            &target.path,
            "real-libshpool",
            &test_dependency_labels_with_optional(package, dependencies, &[], true),
            &real_proc_macro_deps,
            Some((
                "d2b-guest-shell-runner-real-libshpool",
                "CARGO_BIN_EXE_d2b-guest-shell-runner",
            )),
        );
    }
    content.push_str("\ntest_suite(\n");
    content.push_str("    name = \"tests\",\n");
    content.push_str("    tests = [\n");
    for target in &manifest.tests {
        content.push_str("        ");
        content.push_str(&bazel_string(&format!(":{}", target.name)));
        content.push_str(",\n");
    }
    content.push_str("    ],\n)\n");
    Ok(GeneratedBuild {
        path: format!("{}/BUILD.bazel", manifest.package_dir),
        content,
    })
}

fn build_header() -> String {
    String::from(
        "# Generated by cargo xtask gen-bazel. Do not edit.\n\
         package(default_visibility = [\"//visibility:public\"])\n\n\
         load(\"@rules_rust//rust:defs.bzl\", \"rust_binary\", \"rust_library\", \"rust_test\")\n",
    )
}

fn rust_crate_name(package_name: &str) -> String {
    package_name.replace('-', "_")
}

fn source_files_for_package(
    root: &Path,
    package_dir: &str,
    include_main: bool,
) -> Result<Vec<String>, Box<dyn Error>> {
    let package_root = root.join(package_dir);
    let source_root = package_root.join("src");
    let mut source_files = Vec::new();
    collect_rs_files(&source_root, &source_root, &mut source_files)?;
    source_files
        .retain(|source| (include_main || source != "main.rs") && !source.starts_with("bin/"));
    source_files.sort();
    Ok(source_files
        .into_iter()
        .map(|source| format!("src/{source}"))
        .collect())
}

fn dependency_labels(
    package: &DependencyInfo,
    dependencies: &BTreeMap<String, DependencyInfo>,
    local_overrides: &[(&str, &str)],
) -> Vec<String> {
    dependency_labels_for_names(
        &package.normal,
        package,
        dependencies,
        local_overrides,
        false,
        false,
    )
}

fn test_dependency_labels(
    package: &DependencyInfo,
    dependencies: &BTreeMap<String, DependencyInfo>,
    local_overrides: &[(&str, &str)],
) -> Vec<String> {
    test_dependency_labels_with_optional(package, dependencies, local_overrides, false)
}

fn test_dependency_labels_with_optional(
    package: &DependencyInfo,
    dependencies: &BTreeMap<String, DependencyInfo>,
    local_overrides: &[(&str, &str)],
    include_optional: bool,
) -> Vec<String> {
    let mut names = package.normal.clone();
    names.extend(package.dev.iter().cloned());
    names.sort();
    names.dedup();
    dependency_labels_for_names(
        &names,
        package,
        dependencies,
        local_overrides,
        include_optional,
        false,
    )
}

fn dependency_labels_for_names(
    names: &[String],
    package: &DependencyInfo,
    dependencies: &BTreeMap<String, DependencyInfo>,
    local_overrides: &[(&str, &str)],
    include_optional: bool,
    include_proc_macro: bool,
) -> Vec<String> {
    let mut labels = names
        .iter()
        .filter_map(|dependency| {
            if !include_optional && package.optional.contains(dependency) {
                return None;
            }
            if !include_proc_macro && package.proc_macro.contains(dependency) {
                return None;
            }
            if let Some((_, target)) = local_overrides.iter().find(|(name, _)| *name == dependency)
                && let Some(local) = dependencies
                    .values()
                    .find(|candidate| candidate.package_name == *dependency)
            {
                return Some(format!("//{}:{}", local.package_dir, target));
            }
            if let Some(local) = dependencies
                .values()
                .find(|candidate| candidate.package_name == *dependency)
            {
                return Some(format!("//{}:{}", local.package_dir, local.package_name));
            }
            Some(format!("@{}//:{}", package.hub, dependency))
        })
        .collect::<Vec<_>>();
    labels.sort();
    labels.dedup();
    labels
}

fn proc_macro_dependency_labels(
    package: &DependencyInfo,
    dependencies: &BTreeMap<String, DependencyInfo>,
    local_overrides: &[(&str, &str)],
    include_optional: bool,
) -> Vec<String> {
    let names = package.proc_macro.iter().cloned().collect::<Vec<_>>();
    dependency_labels_for_names(
        &names,
        package,
        dependencies,
        local_overrides,
        include_optional,
        true,
    )
}

fn append_deps(content: &mut String, deps: &[String]) {
    content.push_str("    deps = [\n");
    for dep in deps {
        content.push_str("        ");
        content.push_str(&bazel_string(dep));
        content.push_str(",\n");
    }
    content.push_str("    ],\n");
}

fn append_proc_macro_deps(content: &mut String, deps: &[String]) {
    if deps.is_empty() {
        return;
    }
    content.push_str("    proc_macro_deps = [\n");
    for dep in deps {
        content.push_str("        ");
        content.push_str(&bazel_string(dep));
        content.push_str(",\n");
    }
    content.push_str("    ],\n");
}

fn append_context_library(
    content: &mut String,
    package_name: &str,
    target_name: &str,
    feature: &str,
    sources: &[String],
    deps: &[String],
    proc_macro_deps: &[String],
) {
    content.push_str("\nrust_library(\n");
    content.push_str(&format!("    name = {},\n", bazel_string(target_name)));
    content.push_str(&format!(
        "    crate_name = {},\n",
        bazel_string(&rust_crate_name(package_name))
    ));
    content.push_str("    edition = \"2024\",\n");
    if !feature.is_empty() {
        content.push_str(&format!(
            "    crate_features = [{}],\n",
            bazel_string(feature)
        ));
    }
    content.push_str("    srcs = [\n");
    for source in sources {
        content.push_str("        ");
        content.push_str(&bazel_string(source));
        content.push_str(",\n");
    }
    content.push_str("    ],\n");
    append_deps(content, deps);
    append_proc_macro_deps(content, proc_macro_deps);
    content.push_str(")\n");
}

fn append_binary(
    content: &mut String,
    target_name: &str,
    package_name: &str,
    source: &str,
    deps: &[String],
    features: &[String],
) {
    content.push_str("\nrust_binary(\n");
    content.push_str(&format!("    name = {},\n", bazel_string(target_name)));
    content.push_str(&format!(
        "    crate_name = {},\n",
        bazel_string(&rust_crate_name(package_name))
    ));
    content.push_str("    edition = \"2024\",\n");
    if !features.is_empty() {
        content.push_str("    crate_features = [\n");
        for feature in features {
            content.push_str("        ");
            content.push_str(&bazel_string(feature));
            content.push_str(",\n");
        }
        content.push_str("    ],\n");
    }
    content.push_str(&format!("    srcs = [{}],\n", bazel_string(source)));
    append_deps(content, deps);
    content.push_str(")\n");
}

#[allow(clippy::too_many_arguments)]
fn append_test(
    content: &mut String,
    target_name: &str,
    library: &str,
    source: &str,
    feature: &str,
    deps: &[String],
    proc_macro_deps: &[String],
    binary_provider: Option<(&str, &str)>,
) {
    content.push_str("\nrust_test(\n");
    content.push_str(&format!("    name = {},\n", bazel_string(target_name)));
    content.push_str("    edition = \"2024\",\n");
    if !feature.is_empty() {
        content.push_str(&format!(
            "    crate_features = [{}],\n",
            bazel_string(feature)
        ));
    }
    if let Some((binary_provider, binary_env)) = binary_provider {
        content.push_str("    env_inherit = [\"PATH\"],\n");
        content.push_str(&format!(
            "    env = {{\"{binary_env}\": \"$(rootpath :{binary_provider})\"}},\n"
        ));
        content.push_str(&format!(
            "    rustc_env = {{\"{binary_env}\": \"$(rootpath :{binary_provider})\"}},\n"
        ));
    }
    content.push_str(&format!("    srcs = [{}],\n", bazel_string(source)));
    let mut all_deps = vec![format!(":{library}")];
    all_deps.extend(deps.iter().cloned());
    if let Some((binary_provider, _)) = binary_provider {
        all_deps.push(format!(":{binary_provider}"));
    }
    append_deps(content, &all_deps);
    append_proc_macro_deps(content, proc_macro_deps);
    content.push_str(")\n");
}

fn render_workspace_build() -> GeneratedBuild {
    GeneratedBuild {
        path: "packages/BUILD.bazel".to_owned(),
        content: String::from(
            "# Generated by cargo xtask gen-bazel. Do not edit.\n\
             package(default_visibility = [\"//visibility:public\"])\n\n\
            exports_files([\"Cargo.toml\", \"Cargo.lock\"])\n",
        ),
    }
}

fn collect_rs_files(
    root: &Path,
    current: &Path,
    output: &mut Vec<String>,
) -> Result<(), Box<dyn Error>> {
    if !current.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            collect_rs_files(root, &path, output)?;
        } else if file_type.is_file() && path.extension().is_some_and(|ext| ext == "rs") {
            output.push(
                path.strip_prefix(root)
                    .map_err(|_| "source path escaped Cargo package root")?
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
    Ok(())
}

fn governed_source_inventory(root: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let tracked = tracked_paths(root)?;
    let mut sources = tracked
        .into_iter()
        .filter(|path| {
            path.starts_with("packages/")
                && path.ends_with(".rs")
                && !Path::new(path).components().any(|component| {
                    matches!(
                        component,
                        std::path::Component::Normal(value)
                            if matches!(value.to_str(), Some("target" | "tests" | "fixtures" | ".git"))
                    )
                })
        })
        .collect::<Vec<_>>();
    sources.sort();
    sources.dedup();
    if sources.is_empty() {
        return Err("governed Rust source inventory is empty".into());
    }
    Ok(sources)
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
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
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
    path == ".bazelignore"
        || path.starts_with("bazel/generated/")
        || (path.ends_with("/BUILD.bazel")
            && (path.starts_with("packages/")
                || path == "tests/tools/no-bash-ast-walker/BUILD.bazel"))
}

fn tracked_paths(root: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["ls-files", "-z"])
        .output()
        .map_err(|error| format!("could not enumerate tracked files: {error}"))?;
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
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(name.to_str(), Some(".git" | ".scratch" | "target")) {
            continue;
        }
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            collect_files_without_git(root, &path, output)?;
        } else if entry.file_type()?.is_file() {
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

fn tracked_digests(root: &Path) -> Result<BTreeMap<String, [u8; 32]>, Box<dyn Error>> {
    let mut digests = BTreeMap::new();
    for relative in tracked_paths(root)? {
        match fs::read(root.join(&relative)) {
            Ok(bytes) => {
                digests.insert(relative, Sha256::digest(bytes).into());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(digests)
}

fn mutation_snapshot(root: &Path) -> Result<BTreeMap<String, [u8; 32]>, Box<dyn Error>> {
    let mut paths = Vec::new();
    collect_files_without_git(root, root, &mut paths)?;
    paths.sort();
    let mut digests = BTreeMap::new();
    for relative in paths {
        let bytes = fs::read(root.join(&relative))?;
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
    fn startup_options_are_absolute_and_worktree_derived() {
        let options = startup_options(Path::new("/worktree"));
        assert!(options.output_user_root.is_absolute());
        assert!(options.output_base.is_absolute());
        assert!(options.output_user_root.starts_with("/worktree"));
        assert!(options.output_base.starts_with("/worktree"));
        assert!(options.symlink_prefix.starts_with("/worktree"));
        assert_eq!(
            options.startup_args(),
            vec![
                "--output_user_root=/worktree/.scratch/bazel/output-user-root",
                "--output_base=/worktree/.scratch/bazel/output-base",
                "--symlink_prefix=/worktree/.scratch/bazel/symlinks/",
            ]
        );
        assert_eq!(
            options.repin_command_args(true),
            vec![
                "run".to_owned(),
                "--lockfile_mode=off".to_owned(),
                "@rules_rust//crate_universe:cargo-bazel".to_owned(),
                "--".to_owned(),
                "generate".to_owned(),
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
            builds: vec![
                GeneratedBuild {
                    path: "packages/z/BUILD.bazel".into(),
                    content: "z".into(),
                },
                GeneratedBuild {
                    path: "packages/a/BUILD.bazel".into(),
                    content: "a".into(),
                },
            ],
            governed_sources: vec![
                "packages/z/src/lib.rs".into(),
                "packages/a/src/lib.rs".into(),
            ],
            harness_free: Census::new(
                vec!["packages/z#test".into(), "packages/a#test".into()],
                vec![],
            ),
            doctests: Census::new(vec!["packages/a".into()], vec![]),
            bazelignore: vec!["target/".into(), ".scratch/".into()],
            hermeticity: "{}\n".into(),
        };
        let outputs = model.render();
        assert_eq!(
            outputs.keys().collect::<Vec<_>>(),
            vec![
                &".bazelignore".to_string(),
                &"bazel/generated/action-network-inventory.json".to_string(),
                &"bazel/generated/doctest-census.json".to_string(),
                &"bazel/generated/governed-rust-sources.bzl".to_string(),
                &"bazel/generated/harness-free-census.json".to_string(),
                &"bazel/generated/hermeticity-inventory.json".to_string(),
                &"packages/a/BUILD.bazel".to_string(),
                &"packages/z/BUILD.bazel".to_string(),
            ]
        );
        assert!(
            outputs["bazel/generated/governed-rust-sources.bzl"]
                .contains("\"packages/a/src/lib.rs\",\n    \"packages/z/src/lib.rs\"")
        );
        assert!(!outputs["bazel/generated/governed-rust-sources.bzl"].contains("generated_build"));
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
            builds: vec![],
            governed_sources: vec!["packages/sample/src/lib.rs".into()],
            harness_free: Census::new(
                vec!["packages/sample/Cargo.toml#run".into()],
                vec![
                    (
                        "packages/sample/Cargo.toml#fuzz".into(),
                        "required features are not enabled by the Cargo gate selector".into(),
                    ),
                    (
                        "packages/sample/Cargo.toml#bench".into(),
                        "bench targets are not selected by the harness-free test selector".into(),
                    ),
                ],
            ),
            doctests: Census::new(vec!["packages/sample/Cargo.toml".into()], vec![]),
            bazelignore: vec![".scratch/".into()],
            hermeticity: "{}\n".into(),
        };
        assert!(model.validate().is_ok());
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
            builds: vec![],
            governed_sources: vec!["packages/a/src/lib.rs".into()],
            harness_free: Census::new(vec!["a".into()], vec![]),
            doctests: Census::new(vec!["a".into()], vec![]),
            bazelignore: vec!["packages/target/".into()],
            hermeticity: "{}\n".into(),
        };
        assert!(model.validate().is_err());
        model.bazelignore.push(".scratch/".into());
        assert!(model.validate().is_ok());
    }
}
