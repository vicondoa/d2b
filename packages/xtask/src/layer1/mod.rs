use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
};

pub mod checks;
pub mod model;
pub mod runner;
pub mod workflow;

use model::Layer1Manifest;

pub const LAYER1_MANIFEST_VERSION: u32 = 1;

pub type Result<T> = std::result::Result<T, Layer1Error>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Layer1Error {
    message: String,
}

impl Layer1Error {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for Layer1Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for Layer1Error {}

impl From<std::io::Error> for Layer1Error {
    fn from(error: std::io::Error) -> Self {
        Self::new(format!("I/O error: {error}"))
    }
}

impl From<serde_json::Error> for Layer1Error {
    fn from(error: serde_json::Error) -> Self {
        Self::new(format!("JSON error: {error}"))
    }
}

#[derive(Clone, Debug)]
struct Layer1Paths {
    root: PathBuf,
    manifest: PathBuf,
    template: PathBuf,
    workflow: PathBuf,
}

impl Layer1Paths {
    fn repository_defaults() -> Result<Self> {
        let root = repository_root()?;
        Ok(Self {
            manifest: root.join("tests/layer1-jobs.json"),
            template: root.join("tests/ci/layer1-workflow.template.yml"),
            workflow: root.join(".github/workflows/pr-l1-static-fast.yml"),
            root,
        })
    }
}

pub fn repository_root() -> Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .ok_or_else(|| Layer1Error::new("cannot locate repository root"))
}

pub fn load_manifest(path: &Path) -> Result<Layer1Manifest> {
    let bytes = fs::read(path).map_err(|error| {
        Layer1Error::new(format!(
            "cannot read Layer-1 manifest {}: {error}",
            path.display()
        ))
    })?;
    let manifest: Layer1Manifest = serde_json::from_slice(&bytes).map_err(|error| {
        Layer1Error::new(format!(
            "invalid Layer-1 manifest {}: {error}",
            path.display()
        ))
    })?;
    manifest.validate()?;
    Ok(manifest)
}

pub fn run_cli(args: &[String]) -> std::process::ExitCode {
    match run_cli_inner(args) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("layer1 failed: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run_cli_inner(args: &[String]) -> Result<()> {
    let mut paths = Layer1Paths::repository_defaults()?;
    match args {
        [action, rest @ ..] if action == "validate" => {
            let options = CliOptions::parse(rest, &["--manifest"], &[])?;
            paths.manifest = options.path_or("--manifest", paths.manifest);
            load_manifest(&paths.manifest)?;
            println!("Layer-1 manifest is valid");
            Ok(())
        }
        [action, rest @ ..] if action == "run-local" => {
            let options = CliOptions::parse(
                rest,
                &["--manifest", "--jobs"],
                &["--skip-preflight", "--dry-run"],
            )?;
            paths.manifest = options.path_or("--manifest", paths.manifest);
            let manifest = load_manifest(&paths.manifest)?;
            let jobs = runner::resolve_max_jobs(
                options.value("--jobs"),
                std::env::var("D2B_CHECK_JOBS").ok().as_deref(),
                manifest.local.default_jobs,
            )?;
            let skip_preflight = options.has_flag("--skip-preflight");
            let dry_run = options.has_flag("--dry-run");

            if dry_run {
                print!(
                    "{}",
                    runner::render_local_plan(&manifest, skip_preflight, jobs)?
                );
                return Ok(());
            }

            let process_runner = runner::ProcessJobRunner::new(paths.root);
            let report = runner::execute_local(&manifest, skip_preflight, jobs, &process_runner)?;
            process_runner.append_step_summary(&report)?;
            if report.failures.is_empty() {
                println!("Layer-1 manifest runner OK");
                Ok(())
            } else {
                Err(Layer1Error::new(report.failure_summary()))
            }
        }
        [area, action, rest @ ..] if area == "workflow" => {
            let options =
                CliOptions::parse(rest, &["--manifest", "--template", "--workflow"], &[])?;
            paths.manifest = options.path_or("--manifest", paths.manifest);
            paths.template = options.path_or("--template", paths.template);
            paths.workflow = options.path_or("--workflow", paths.workflow);
            let manifest = load_manifest(&paths.manifest)?;
            let template = fs::read_to_string(&paths.template).map_err(|error| {
                Layer1Error::new(format!(
                    "cannot read workflow template {}: {error}",
                    paths.template.display()
                ))
            })?;
            let rendered = workflow::render_workflow(&manifest, &template)?;
            match action.as_str() {
                "render" => {
                    print!("{rendered}");
                    Ok(())
                }
                "write" => {
                    fs::write(&paths.workflow, rendered).map_err(|error| {
                        Layer1Error::new(format!(
                            "cannot write workflow {}: {error}",
                            paths.workflow.display()
                        ))
                    })?;
                    println!("wrote {}", paths.workflow.display());
                    Ok(())
                }
                "check" => {
                    workflow::check_workflow_file(&paths.workflow, &rendered)?;
                    println!("layer1 workflow: generated artifact is up to date");
                    Ok(())
                }
                _ => Err(usage()),
            }
        }
        [area, action, rest @ ..] if area == "checks" && action == "list" => {
            let options = CliOptions::parse(rest, &["--system"], &[])?;
            let system = options.value("--system");
            let json = checks::discover_check_json(&paths.root, system)?;
            println!("{json}");
            Ok(())
        }
        _ => Err(usage()),
    }
}

fn usage() -> Layer1Error {
    Layer1Error::new(
        "usage: cargo xtask layer1 <validate [--manifest PATH]|run-local \
         [--manifest PATH] [--jobs N] [--skip-preflight] [--dry-run]|workflow \
         <render|write|check> [--manifest PATH] [--template PATH] \
         [--workflow PATH]|checks list [--system SYSTEM]>",
    )
}

#[derive(Debug)]
struct CliOptions {
    values: BTreeMap<String, String>,
    flags: BTreeSet<String>,
}

impl CliOptions {
    fn parse(args: &[String], value_options: &[&str], flag_options: &[&str]) -> Result<Self> {
        let allowed_values = value_options.iter().copied().collect::<BTreeSet<_>>();
        let allowed_flags = flag_options.iter().copied().collect::<BTreeSet<_>>();
        let mut values = BTreeMap::new();
        let mut flags = BTreeSet::new();
        let mut index = 0;
        while index < args.len() {
            let option = args[index].as_str();
            if allowed_flags.contains(option) {
                if !flags.insert(option.to_owned()) {
                    return Err(Layer1Error::new(format!("duplicate option {option}")));
                }
                index += 1;
                continue;
            }
            if allowed_values.contains(option) {
                let Some(value) = args.get(index + 1) else {
                    return Err(Layer1Error::new(format!(
                        "option {option} is missing its value"
                    )));
                };
                if values.insert(option.to_owned(), value.clone()).is_some() {
                    return Err(Layer1Error::new(format!("duplicate option {option}")));
                }
                index += 2;
                continue;
            }
            return Err(Layer1Error::new(format!("unknown option {option}")));
        }
        Ok(Self { values, flags })
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(String::as_str)
    }

    fn path_or(&self, name: &str, default: PathBuf) -> PathBuf {
        self.values.get(name).map_or(default, PathBuf::from)
    }

    fn has_flag(&self, name: &str) -> bool {
        self.flags.contains(name)
    }
}
