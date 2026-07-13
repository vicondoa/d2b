use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use super::{
    Layer1Error, Result,
    model::{JobSpec, Layer1Manifest, LocalPhase, PhaseMode},
};

static NEXT_LOG_DIR: AtomicU64 = AtomicU64::new(1);
const FAILURE_TAIL_LINES: usize = 200;
const SUMMARY_TAIL_LINES: usize = 40;
const STEP_SUMMARY_MAX_BYTES: usize = 16 * 1024;
const STEP_SUMMARY_TRUNCATED: &str = "\n\n_Additional output omitted._\n";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Succeeded,
    Failed(JobResult),
    Blocked(Vec<String>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JobResult {
    ExitCode(i32),
    Signal(i32),
    RunnerError,
}

impl JobResult {
    fn status(self) -> JobStatus {
        match self {
            Self::ExitCode(0) => JobStatus::Succeeded,
            failure => JobStatus::Failed(failure),
        }
    }

    fn summary(self) -> String {
        match self {
            Self::ExitCode(code) => format!("exit {code}"),
            Self::Signal(signal) => render_signal(signal),
            Self::RunnerError => "runner error".to_owned(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobOutcome {
    pub job_id: String,
    pub make_target: String,
    pub status: JobStatus,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ExecutionReport {
    pub outcomes: Vec<JobOutcome>,
    pub failures: Vec<JobOutcome>,
}

impl ExecutionReport {
    pub fn failure_summary(&self) -> String {
        let failures = self
            .failures
            .iter()
            .map(|outcome| match &outcome.status {
                JobStatus::Failed(result) => {
                    format!("{} ({})", outcome.job_id, result.summary())
                }
                JobStatus::Blocked(dependencies) => format!(
                    "{} (blocked by {})",
                    outcome.job_id,
                    dependencies.join(", ")
                ),
                JobStatus::Succeeded => outcome.job_id.clone(),
            })
            .collect::<Vec<_>>();
        format!("Layer-1 job failure(s): {}", failures.join("; "))
    }
}

pub trait LocalJobRunner: Sync {
    fn run(&self, job_id: &str, job: &JobSpec) -> JobResult;
}

#[derive(Clone, Debug)]
struct SummaryEntry {
    job_id: String,
    lines: Vec<String>,
}

#[derive(Clone, Debug)]
struct WorkspaceRedactor {
    roots: Vec<String>,
}

impl WorkspaceRedactor {
    fn new(root: &Path, aliases: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut paths = Vec::new();
        paths.push(root.to_path_buf());
        paths.extend(aliases);

        let mut roots = Vec::new();
        for path in paths {
            if let Some(absolute) = absolute_path(&path) {
                push_root_variants(&mut roots, &absolute);
            }
            if let Ok(canonical) = fs::canonicalize(&path) {
                push_root_variants(&mut roots, &canonical);
            }
        }
        roots.sort_by_key(|root| std::cmp::Reverse(root.len()));
        roots.dedup();
        Self { roots }
    }

    fn redact(&self, text: &str) -> String {
        self.roots.iter().fold(text.to_owned(), |redacted, root| {
            replace_path_root(&redacted, root)
        })
    }
}

#[derive(Clone, Debug)]
pub struct ProcessJobRunner {
    root: PathBuf,
    redactor: WorkspaceRedactor,
    step_summary: Option<PathBuf>,
    summary_entries: Arc<Mutex<Vec<SummaryEntry>>>,
}

impl ProcessJobRunner {
    pub fn new(root: PathBuf) -> Self {
        let workspace = env::var_os("GITHUB_WORKSPACE").map(PathBuf::from);
        let step_summary = env::var_os("GITHUB_STEP_SUMMARY").map(PathBuf::from);
        Self::with_step_summary(root, workspace, step_summary)
    }

    fn with_step_summary(
        root: PathBuf,
        workspace: Option<PathBuf>,
        step_summary: Option<PathBuf>,
    ) -> Self {
        let redactor = WorkspaceRedactor::new(&root, workspace);
        Self {
            root,
            redactor,
            step_summary,
            summary_entries: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn run_process(&self, job_id: &str, job: &JobSpec) -> Result<JobResult> {
        let target = job
            .make_target
            .as_deref()
            .ok_or_else(|| Layer1Error::new(format!("local job {job_id} has no makeTarget")))?;
        let (log_dir, log_path) = self.create_log(job_id)?;
        println!("==> {target} ({})", job.display_name);
        flush_stdout();

        let log = File::create(&log_path).map_err(|error| {
            Layer1Error::new(format!(
                "cannot create job log {}: {error}",
                log_path.display()
            ))
        })?;
        let stderr_log = log.try_clone().map_err(|error| {
            Layer1Error::new(format!(
                "cannot clone job log {}: {error}",
                log_path.display()
            ))
        })?;
        let status = self
            .make_command(target, job)
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr_log))
            .status()
            .map_err(|error| {
                Layer1Error::new(format!("could not execute make -- {target}: {error}"))
            })?;

        if status.success() {
            println!("ok: {target}");
            flush_stdout();
            self.record_summary(job_id, Vec::new());
            if env::var("D2B_CHECK_KEEP_LOGS").as_deref() != Ok("1") {
                let _ = fs::remove_file(&log_path);
                let _ = fs::remove_dir(&log_dir);
            }
            return Ok(JobResult::ExitCode(0));
        }

        let result = job_result(status);
        let status_summary = result.summary();
        let redacted_log_path = self.redactor.redact(&log_path.display().to_string());
        eprintln!("FAIL: {target} ({status_summary}); tail of {redacted_log_path}:");
        match tail_lines(&log_path, FAILURE_TAIL_LINES) {
            Ok(lines) => {
                let redacted = lines
                    .into_iter()
                    .map(|line| self.redactor.redact(&line))
                    .collect::<Vec<_>>();
                for line in &redacted {
                    eprintln!("{line}");
                }
                self.record_summary(
                    job_id,
                    redacted
                        .into_iter()
                        .rev()
                        .take(SUMMARY_TAIL_LINES)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect(),
                );
            }
            Err(error) => {
                let message = format!(
                    "could not read {redacted_log_path}: {}",
                    self.redactor.redact(&error.to_string())
                );
                eprintln!("{message}");
                self.record_summary(job_id, vec![message]);
            }
        }
        Ok(result)
    }

    fn record_summary(&self, job_id: &str, lines: Vec<String>) {
        self.summary_entries
            .lock()
            .expect("Layer-1 summary entries lock")
            .push(SummaryEntry {
                job_id: job_id.to_owned(),
                lines,
            });
    }

    pub fn append_step_summary(&self, report: &ExecutionReport) -> Result<()> {
        let Some(path) = &self.step_summary else {
            return Ok(());
        };
        let entries = self
            .summary_entries
            .lock()
            .expect("Layer-1 summary entries lock")
            .clone();
        let summary = render_step_summary(report, &entries, &self.redactor);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .mode(0o600)
            .open(path)
            .map_err(|error| {
                Layer1Error::new(format!("cannot append GitHub step summary: {error}"))
            })?;
        file.write_all(summary.as_bytes()).map_err(|error| {
            Layer1Error::new(format!("cannot append GitHub step summary: {error}"))
        })
    }

    fn create_log(&self, job_id: &str) -> Result<(PathBuf, PathBuf)> {
        let configured = env::var_os("D2B_LAYER1_LOG_DIR").map(PathBuf::from);
        let base = configured.map_or_else(
            || self.root.join("packages/target/layer1-logs"),
            |path| {
                if path.is_absolute() {
                    path
                } else {
                    self.root.join(path)
                }
            },
        );
        fs::create_dir_all(&base).map_err(|error| {
            Layer1Error::new(format!(
                "cannot create Layer-1 log root {}: {error}",
                base.display()
            ))
        })?;
        for _ in 0..1024 {
            let sequence = NEXT_LOG_DIR.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("{job_id}.{}.{sequence}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok((path.clone(), path.join("output.log"))),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(Layer1Error::new(format!(
                        "cannot create Layer-1 job log directory {}: {error}",
                        path.display()
                    )));
                }
            }
        }
        Err(Layer1Error::new(format!(
            "could not reserve a unique log directory for {job_id}"
        )))
    }

    fn make_command(&self, target: &str, job: &JobSpec) -> Command {
        let mut command = Command::new("make");
        command
            .args(["--", target])
            .current_dir(&self.root)
            .envs(&job.local_env);
        command
    }
}

fn absolute_path(path: &Path) -> Option<PathBuf> {
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        env::current_dir().ok().map(|current| current.join(path))
    }
}

fn push_root_variants(roots: &mut Vec<String>, path: &Path) {
    let root = path.to_string_lossy();
    let root = root.trim_end_matches('/');
    if root.is_empty() {
        return;
    }
    roots.push(root.to_owned());
    roots.push(root.replace('/', r"\/"));

    let shell_escaped = root
        .chars()
        .flat_map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '/' | '_' | '-' | '.') {
                [None, Some(character)]
            } else {
                [Some('\\'), Some(character)]
            }
        })
        .flatten()
        .collect::<String>();
    roots.push(shell_escaped.replace('/', r"\/"));
    roots.push(shell_escaped);
}

fn replace_path_root(text: &str, root: &str) -> String {
    let mut rendered = String::with_capacity(text.len());
    let mut remainder = text;
    while let Some(index) = remainder.find(root) {
        let (prefix, candidate) = remainder.split_at(index);
        rendered.push_str(prefix);
        let suffix = &candidate[root.len()..];
        let boundary = suffix.chars().next().is_none_or(
            |character| !matches!(character, 'A'..='Z' | 'a'..='z' | '0'..='9' | '_' | '-' | '.'),
        );
        if boundary {
            rendered.push('.');
        } else {
            rendered.push_str(root);
        }
        remainder = suffix;
    }
    rendered.push_str(remainder);
    rendered
}

fn job_result(status: ExitStatus) -> JobResult {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;

        if let Some(signal) = status.signal() {
            return JobResult::Signal(signal);
        }
    }

    status
        .code()
        .map_or(JobResult::RunnerError, JobResult::ExitCode)
}

fn render_signal(signal: i32) -> String {
    let name = known_signal_name(signal);
    name.map_or_else(
        || format!("signal {signal}"),
        |name| format!("signal {signal} ({name})"),
    )
}

fn known_signal_name(signal: i32) -> Option<&'static str> {
    #[cfg(unix)]
    {
        use signal_hook::consts::{SIGKILL, SIGTERM};

        match signal {
            SIGKILL => Some("SIGKILL"),
            SIGTERM => Some("SIGTERM"),
            _ => None,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = signal;
        None
    }
}

fn render_step_summary(
    report: &ExecutionReport,
    entries: &[SummaryEntry],
    redactor: &WorkspaceRedactor,
) -> String {
    let mut rendered = String::from("## Layer-1 outcome\n\n");
    if report.failures.is_empty() {
        rendered.push_str(&format!(
            "✅ All {} Layer-1 jobs passed.\n",
            report.outcomes.len()
        ));
    } else {
        rendered.push_str(&format!(
            "❌ {} of {} Layer-1 jobs did not pass.\n",
            report.failures.len(),
            report.outcomes.len()
        ));
        for outcome in &report.failures {
            let status = match &outcome.status {
                JobStatus::Failed(result) => result.summary(),
                JobStatus::Blocked(dependencies) => {
                    format!("blocked by {}", dependencies.join(", "))
                }
                JobStatus::Succeeded => "passed".to_owned(),
            };
            rendered.push_str(&format!("- `{}` ({status})\n", outcome.job_id));
        }

        let entries = entries
            .iter()
            .map(|entry| (entry.job_id.as_str(), entry))
            .collect::<BTreeMap<_, _>>();
        for outcome in &report.failures {
            let JobStatus::Failed(result) = outcome.status else {
                continue;
            };
            let Some(entry) = entries.get(outcome.job_id.as_str()) else {
                continue;
            };
            rendered.push_str(&format!(
                "\n<details><summary>Redacted tail for <code>{}</code> ({})</summary>\n\n",
                outcome.job_id,
                result.summary()
            ));
            if entry.lines.is_empty() {
                rendered.push_str("    No log tail was captured.\n");
            } else {
                for line in &entry.lines {
                    rendered.push_str("    ");
                    rendered.push_str(&redactor.redact(line));
                    rendered.push('\n');
                }
            }
            rendered.push_str("\n</details>\n");
        }
    }

    truncate_step_summary(redactor.redact(&rendered))
}

fn truncate_step_summary(mut summary: String) -> String {
    if summary.len() <= STEP_SUMMARY_MAX_BYTES {
        return summary;
    }
    let mut keep = STEP_SUMMARY_MAX_BYTES - STEP_SUMMARY_TRUNCATED.len();
    while !summary.is_char_boundary(keep) {
        keep -= 1;
    }
    summary.truncate(keep);
    summary.push_str(STEP_SUMMARY_TRUNCATED);
    summary
}

impl LocalJobRunner for ProcessJobRunner {
    fn run(&self, job_id: &str, job: &JobSpec) -> JobResult {
        match self.run_process(job_id, job) {
            Ok(result) => result,
            Err(error) => {
                let message = self.redactor.redact(&error.to_string());
                eprintln!("FAIL: Layer-1 job {job_id}: {message}");
                self.record_summary(job_id, vec![message]);
                JobResult::RunnerError
            }
        }
    }
}

pub fn resolve_max_jobs(
    cli_value: Option<&str>,
    environment_value: Option<&str>,
    default: usize,
) -> Result<usize> {
    let (source, value) = if let Some(value) = cli_value {
        ("--jobs", value)
    } else if let Some(value) = environment_value {
        ("D2B_CHECK_JOBS", value)
    } else {
        return if default == 0 {
            Err(Layer1Error::new("local.defaultJobs must be >= 1"))
        } else {
            Ok(default)
        };
    };
    let parsed = value
        .parse::<usize>()
        .map_err(|_| Layer1Error::new(format!("{source} must be an integer >= 1")))?;
    if parsed == 0 {
        return Err(Layer1Error::new(format!("{source} must be >= 1")));
    }
    Ok(parsed)
}

pub fn render_local_plan(
    manifest: &Layer1Manifest,
    skip_preflight: bool,
    max_jobs: usize,
) -> Result<String> {
    manifest.validate()?;
    if max_jobs == 0 {
        return Err(Layer1Error::new("maximum jobs must be >= 1"));
    }
    let mut rendered = format!("Layer-1 local plan (max jobs: {max_jobs})\n");
    for phase in selected_phases(manifest, skip_preflight) {
        let mode = match phase.mode {
            PhaseMode::Serial => "serial",
            PhaseMode::Parallel => "parallel",
        };
        rendered.push_str(&format!("{} ({mode})\n", phase.id));
        for job_id in &phase.jobs {
            let job = &manifest.jobs[job_id];
            let target = job
                .make_target
                .as_deref()
                .ok_or_else(|| Layer1Error::new(format!("local job {job_id} has no makeTarget")))?;
            rendered.push_str(&format!("  {job_id}: make -- {target}\n"));
        }
    }
    Ok(rendered)
}

pub fn execute_local<R: LocalJobRunner>(
    manifest: &Layer1Manifest,
    skip_preflight: bool,
    max_jobs: usize,
    runner: &R,
) -> Result<ExecutionReport> {
    manifest.validate()?;
    if max_jobs == 0 {
        return Err(Layer1Error::new("maximum jobs must be >= 1"));
    }

    let mut outcomes = Vec::new();
    let mut succeeded = BTreeSet::new();
    if skip_preflight {
        for phase in manifest
            .local
            .phases
            .iter()
            .filter(|phase| phase.id == "preflight")
        {
            succeeded.extend(phase.jobs.iter().cloned());
        }
    }

    for phase in selected_phases(manifest, skip_preflight) {
        let mode = match phase.mode {
            PhaseMode::Serial => "serial",
            PhaseMode::Parallel => "parallel",
        };
        println!("==> Layer-1 phase: {} ({mode})", phase.id);
        flush_stdout();
        let failures_before = outcomes
            .iter()
            .filter(|outcome: &&JobOutcome| outcome.status != JobStatus::Succeeded)
            .count();
        match phase.mode {
            PhaseMode::Serial => {
                for job_id in &phase.jobs {
                    let blocked = failed_or_missing_dependencies(
                        &manifest.jobs[job_id],
                        &succeeded,
                        &outcomes,
                    );
                    if !blocked.is_empty() {
                        outcomes.push(blocked_outcome(manifest, job_id, blocked));
                        return Ok(report(outcomes));
                    }
                    let outcome = run_one(manifest, job_id, runner);
                    if outcome.status == JobStatus::Succeeded {
                        succeeded.insert(job_id.clone());
                    } else {
                        outcomes.push(outcome);
                        return Ok(report(outcomes));
                    }
                    outcomes.push(outcome);
                }
            }
            PhaseMode::Parallel => run_parallel_phase(
                manifest,
                phase,
                max_jobs,
                runner,
                &mut succeeded,
                &mut outcomes,
            )?,
        }
        let failures_after = outcomes
            .iter()
            .filter(|outcome| outcome.status != JobStatus::Succeeded)
            .count();
        if failures_after > failures_before {
            return Ok(report(outcomes));
        }
    }
    Ok(report(outcomes))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SchedulerEvent {
    BeforeSchedule,
    Waiting,
}

fn run_parallel_phase<R: LocalJobRunner>(
    manifest: &Layer1Manifest,
    phase: &LocalPhase,
    max_jobs: usize,
    runner: &R,
    succeeded: &mut BTreeSet<String>,
    outcomes: &mut Vec<JobOutcome>,
) -> Result<()> {
    run_parallel_phase_observed(
        manifest,
        phase,
        max_jobs,
        runner,
        succeeded,
        outcomes,
        &|_, _| {},
    )
}

fn run_parallel_phase_observed<R, O>(
    manifest: &Layer1Manifest,
    phase: &LocalPhase,
    max_jobs: usize,
    runner: &R,
    succeeded: &mut BTreeSet<String>,
    outcomes: &mut Vec<JobOutcome>,
    observer: &O,
) -> Result<()>
where
    R: LocalJobRunner,
    O: Fn(SchedulerEvent, &str) + Sync,
{
    #[derive(Default)]
    struct SchedulerState {
        running: usize,
        started: BTreeSet<String>,
        results: BTreeMap<String, JobStatus>,
    }

    let previous_successes = succeeded.clone();
    let state = Mutex::new(SchedulerState::default());
    let changed = Condvar::new();
    let batch = std::thread::scope(|scope| {
        let handles = phase
            .jobs
            .iter()
            .enumerate()
            .map(|(job_index, job_id)| {
                let previous_successes = &previous_successes;
                let state = &state;
                let changed = &changed;
                scope.spawn(move || {
                    observer(SchedulerEvent::BeforeSchedule, job_id);
                    let job = &manifest.jobs[job_id];
                    let target = job
                        .make_target
                        .clone()
                        .unwrap_or_else(|| "<missing>".to_owned());
                    loop {
                        let mut scheduler = state.lock().expect("Layer-1 scheduler lock");
                        let mut blocked = Vec::new();
                        let mut waiting = false;
                        for dependency in &job.needs {
                            if previous_successes.contains(dependency) {
                                continue;
                            }
                            match scheduler.results.get(dependency) {
                                Some(JobStatus::Succeeded) => {}
                                Some(JobStatus::Failed(_) | JobStatus::Blocked(_)) => {
                                    blocked.push(dependency.clone());
                                }
                                None => waiting = true,
                            }
                        }
                        if !blocked.is_empty() {
                            blocked.sort();
                            blocked.dedup();
                            let status = JobStatus::Blocked(blocked);
                            scheduler.results.insert(job_id.clone(), status.clone());
                            changed.notify_all();
                            return JobOutcome {
                                job_id: job_id.clone(),
                                make_target: target,
                                status,
                            };
                        }
                        let earlier_ready = phase.jobs[..job_index].iter().any(|candidate_id| {
                            if scheduler.started.contains(candidate_id)
                                || scheduler.results.contains_key(candidate_id)
                            {
                                return false;
                            }
                            manifest.jobs[candidate_id].needs.iter().all(|dependency| {
                                previous_successes.contains(dependency)
                                    || matches!(
                                        scheduler.results.get(dependency),
                                        Some(JobStatus::Succeeded)
                                    )
                            })
                        });
                        if waiting || earlier_ready || scheduler.running >= max_jobs {
                            observer(SchedulerEvent::Waiting, job_id);
                            drop(
                                changed
                                    .wait(scheduler)
                                    .expect("Layer-1 scheduler wait lock"),
                            );
                            continue;
                        }
                        scheduler.running += 1;
                        scheduler.started.insert(job_id.clone());
                        changed.notify_all();
                        drop(scheduler);

                        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            runner.run(job_id, job)
                        }))
                        .unwrap_or_else(|_| {
                            eprintln!("FAIL: Layer-1 job worker panicked for {job_id}");
                            JobResult::RunnerError
                        });
                        let status = result.status();
                        let mut scheduler = state.lock().expect("Layer-1 scheduler lock");
                        scheduler.running -= 1;
                        scheduler.results.insert(job_id.clone(), status.clone());
                        changed.notify_all();
                        return JobOutcome {
                            job_id: job_id.clone(),
                            make_target: target,
                            status,
                        };
                    }
                })
            })
            .collect::<Vec<_>>();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .map_err(|_| Layer1Error::new("Layer-1 scheduler worker panicked"))
            })
            .collect::<Result<Vec<_>>>()
    })?;
    for outcome in batch {
        if outcome.status == JobStatus::Succeeded {
            succeeded.insert(outcome.job_id.clone());
        }
        outcomes.push(outcome);
    }
    Ok(())
}

fn run_one<R: LocalJobRunner>(manifest: &Layer1Manifest, job_id: &str, runner: &R) -> JobOutcome {
    let job = &manifest.jobs[job_id];
    let target = job
        .make_target
        .clone()
        .unwrap_or_else(|| "<missing>".to_owned());
    let result = runner.run(job_id, job);
    JobOutcome {
        job_id: job_id.to_owned(),
        make_target: target,
        status: result.status(),
    }
}

fn blocked_outcome(
    manifest: &Layer1Manifest,
    job_id: &str,
    mut dependencies: Vec<String>,
) -> JobOutcome {
    dependencies.sort();
    dependencies.dedup();
    JobOutcome {
        job_id: job_id.to_owned(),
        make_target: manifest.jobs[job_id]
            .make_target
            .clone()
            .unwrap_or_else(|| "<missing>".to_owned()),
        status: JobStatus::Blocked(dependencies),
    }
}

fn failed_or_missing_dependencies(
    job: &JobSpec,
    succeeded: &BTreeSet<String>,
    outcomes: &[JobOutcome],
) -> Vec<String> {
    let failed = outcomes
        .iter()
        .filter(|outcome| outcome.status != JobStatus::Succeeded)
        .map(|outcome| outcome.job_id.as_str())
        .collect::<BTreeSet<_>>();
    job.needs
        .iter()
        .filter(|dependency| {
            failed.contains(dependency.as_str()) || !succeeded.contains(*dependency)
        })
        .cloned()
        .collect()
}

fn report(outcomes: Vec<JobOutcome>) -> ExecutionReport {
    let failures = outcomes
        .iter()
        .filter(|outcome| outcome.status != JobStatus::Succeeded)
        .cloned()
        .collect();
    ExecutionReport { outcomes, failures }
}

fn selected_phases(
    manifest: &Layer1Manifest,
    skip_preflight: bool,
) -> impl Iterator<Item = &LocalPhase> {
    manifest
        .local
        .phases
        .iter()
        .filter(move |phase| !skip_preflight || phase.id != "preflight")
}

fn tail_lines(path: &Path, count: usize) -> std::io::Result<Vec<String>> {
    let mut file = File::open(path)?;
    let length = file.seek(SeekFrom::End(0))?;
    let read_length = length.min(1024 * 1024);
    file.seek(SeekFrom::End(-(read_length as i64)))?;
    let mut bytes = Vec::with_capacity(read_length as usize);
    file.read_to_end(&mut bytes)?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text
        .lines()
        .rev()
        .take(count)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .map(str::to_owned)
        .collect())
}

fn flush_stdout() {
    let _ = std::io::stdout().flush();
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{Condvar, Mutex},
        time::{Duration, Instant},
    };

    use super::*;

    #[derive(Default)]
    struct RecordingRunner {
        events: Mutex<Vec<String>>,
        statuses: BTreeMap<String, JobResult>,
    }

    impl LocalJobRunner for RecordingRunner {
        fn run(&self, job_id: &str, _job: &JobSpec) -> JobResult {
            self.events
                .lock()
                .expect("events")
                .push(format!("start:{job_id}"));
            let status = self
                .statuses
                .get(job_id)
                .copied()
                .unwrap_or(JobResult::ExitCode(0));
            self.events
                .lock()
                .expect("events")
                .push(format!("end:{job_id}"));
            status
        }
    }

    #[derive(Default)]
    struct BlockingState {
        active: usize,
        max_active: usize,
        release: bool,
    }

    #[derive(Default)]
    struct BlockingRunner {
        state: Mutex<BlockingState>,
        changed: Condvar,
    }

    impl BlockingRunner {
        fn wait_for_active(&self, expected: usize) -> bool {
            let deadline = Instant::now() + Duration::from_secs(2);
            let mut state = self.state.lock().expect("blocking runner state");
            while state.max_active < expected {
                let now = Instant::now();
                if now >= deadline {
                    break;
                }
                let (next, timeout) = self
                    .changed
                    .wait_timeout(state, deadline.saturating_duration_since(now))
                    .expect("blocking runner wait");
                state = next;
                if timeout.timed_out() {
                    break;
                }
            }
            state.max_active >= expected
        }

        fn release(&self) {
            let mut state = self.state.lock().expect("blocking runner state");
            state.release = true;
            self.changed.notify_all();
        }
    }

    impl LocalJobRunner for BlockingRunner {
        fn run(&self, _job_id: &str, _job: &JobSpec) -> JobResult {
            let mut state = self.state.lock().expect("blocking runner state");
            state.active += 1;
            state.max_active = state.max_active.max(state.active);
            self.changed.notify_all();
            while !state.release {
                state = self.changed.wait(state).expect("blocking runner wait");
            }
            state.active -= 1;
            JobResult::ExitCode(0)
        }
    }

    fn manifest(parallel_jobs: serde_json::Value, jobs: serde_json::Value) -> Layer1Manifest {
        serde_json::from_value(serde_json::json!({
            "version": 1,
            "local": {
                "defaultJobs": 2,
                "phases": [
                    {"id": "preflight", "mode": "serial", "jobs": ["pre"]},
                    {"id": "parallel", "mode": "parallel", "jobs": parallel_jobs},
                    {"id": "after", "mode": "serial", "jobs": ["after"]}
                ]
            },
            "ci": {
                "workflowName": "layer1",
                "jobs": ["pre", "one", "two", "after"],
                "rollupJob": "check",
                "rollupNeeds": ["after"],
                "allowedSkippedRollupJobs": []
            },
            "jobs": jobs
        }))
        .expect("manifest")
    }

    fn job(name: &str, needs: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "displayName": name,
            "makeTarget": name,
            "ciKind": if name == "pre" { "tier0" } else { "simple-nix" },
            "ciJobId": name,
            "needs": needs,
            "timeoutMinutes": 1,
            "runsOn": "ubuntu-latest"
        })
    }

    #[test]
    fn parallel_dependencies_preserve_phase_order() {
        let manifest = manifest(
            serde_json::json!(["one", "two"]),
            serde_json::json!({
                "pre": job("pre", &[]),
                "one": job("one", &["pre"]),
                "two": job("two", &["one"]),
                "after": job("after", &["two"])
            }),
        );
        let runner = RecordingRunner::default();
        let report = execute_local(&manifest, false, 2, &runner).expect("execute");
        assert!(report.failures.is_empty());
        assert_eq!(
            runner.events.into_inner().expect("events"),
            vec![
                "start:pre",
                "end:pre",
                "start:one",
                "end:one",
                "start:two",
                "end:two",
                "start:after",
                "end:after"
            ]
        );
    }

    #[test]
    fn parallel_scheduler_fills_max_jobs_before_first_completion() {
        let manifest = manifest(
            serde_json::json!(["one", "two"]),
            serde_json::json!({
                "pre": job("pre", &[]),
                "one": job("one", &["pre"]),
                "two": job("two", &["pre"]),
                "after": job("after", &["one", "two"])
            }),
        );
        let runner = BlockingRunner::default();
        let schedule_release = (Mutex::new(false), Condvar::new());
        let observer = |event: SchedulerEvent, job_id: &str| match (event, job_id) {
            (SchedulerEvent::BeforeSchedule, "one") => {
                let (lock, changed) = &schedule_release;
                let mut released = lock.lock().expect("schedule release");
                while !*released {
                    released = changed.wait(released).expect("schedule release wait");
                }
            }
            (SchedulerEvent::Waiting, "two") => {
                let (lock, changed) = &schedule_release;
                *lock.lock().expect("schedule release") = true;
                changed.notify_all();
            }
            _ => {}
        };

        let (reached_limit, result) = std::thread::scope(|scope| {
            let handle = scope.spawn(|| {
                let mut succeeded = BTreeSet::from(["pre".to_owned()]);
                let mut outcomes = Vec::new();
                run_parallel_phase_observed(
                    &manifest,
                    &manifest.local.phases[1],
                    2,
                    &runner,
                    &mut succeeded,
                    &mut outcomes,
                    &observer,
                )
                .map(|()| outcomes)
            });
            let reached_limit = runner.wait_for_active(2);
            runner.release();
            (
                reached_limit,
                handle.join().expect("parallel scheduler thread"),
            )
        });

        let outcomes = result.expect("parallel phase");
        assert!(
            reached_limit,
            "scheduler waited for a completion instead of filling max_jobs"
        );
        assert!(
            outcomes
                .iter()
                .all(|outcome| outcome.status == JobStatus::Succeeded)
        );
    }

    #[test]
    fn parallel_failures_are_aggregated_before_stopping() {
        let mut manifest = manifest(
            serde_json::json!(["one", "two"]),
            serde_json::json!({
                "pre": job("pre", &[]),
                "one": job("one", &["pre"]),
                "two": job("two", &["pre"]),
                "after": job("after", &["one", "two"])
            }),
        );
        manifest.ci.rollup_needs = vec!["after".to_owned()];
        let runner = RecordingRunner {
            events: Mutex::new(Vec::new()),
            statuses: BTreeMap::from([
                ("one".to_owned(), JobResult::ExitCode(3)),
                ("two".to_owned(), JobResult::ExitCode(7)),
            ]),
        };
        let report = execute_local(&manifest, false, 2, &runner).expect("execute");
        assert_eq!(
            report
                .failures
                .iter()
                .map(|outcome| outcome.job_id.as_str())
                .collect::<Vec<_>>(),
            vec!["one", "two"]
        );
        let events = runner.events.into_inner().expect("events");
        assert!(events.contains(&"start:one".to_owned()));
        assert!(events.contains(&"start:two".to_owned()));
        assert!(!events.contains(&"start:after".to_owned()));
        assert!(report.failure_summary().contains("one (exit 3)"));
        assert!(report.failure_summary().contains("two (exit 7)"));
    }

    #[cfg(unix)]
    #[test]
    fn process_results_preserve_exit_codes_and_termination_signals() {
        use signal_hook::consts::{SIGKILL, SIGTERM};

        let exit = Command::new("sh")
            .args(["-c", "exit 23"])
            .status()
            .expect("normal exit");
        let sigkill = Command::new("sh")
            .args(["-c", "kill -KILL $$"])
            .status()
            .expect("SIGKILL exit");
        let sigterm = Command::new("sh")
            .args(["-c", "kill -TERM $$"])
            .status()
            .expect("SIGTERM exit");

        assert_eq!(job_result(exit), JobResult::ExitCode(23));
        assert_eq!(job_result(sigkill), JobResult::Signal(SIGKILL));
        assert_eq!(job_result(sigterm), JobResult::Signal(SIGTERM));

        let report = report(vec![
            JobOutcome {
                job_id: "killed".to_owned(),
                make_target: "killed".to_owned(),
                status: JobStatus::Failed(JobResult::Signal(SIGKILL)),
            },
            JobOutcome {
                job_id: "terminated".to_owned(),
                make_target: "terminated".to_owned(),
                status: JobStatus::Failed(JobResult::Signal(SIGTERM)),
            },
        ]);
        let human = report.failure_summary();
        assert!(human.contains(&format!("killed (signal {SIGKILL} (SIGKILL))")));
        assert!(human.contains(&format!("terminated (signal {SIGTERM} (SIGTERM))")));

        let summary = render_step_summary(
            &report,
            &[],
            &WorkspaceRedactor::new(Path::new("/checkout/d2b"), []),
        );
        assert!(summary.contains(&format!("`killed` (signal {SIGKILL} (SIGKILL))")));
        assert!(summary.contains(&format!("`terminated` (signal {SIGTERM} (SIGTERM))")));
        assert!(!summary.contains("core"));
    }

    #[test]
    fn make_argv_terminates_options_before_the_validated_target() {
        let runner = ProcessJobRunner::new(PathBuf::from("repository"));
        let mut spec: JobSpec = serde_json::from_value(job("one", &[])).expect("job");
        spec.make_target = Some("--version".to_owned());
        let command = runner.make_command("--version", &spec);
        let arguments = command
            .get_args()
            .map(|argument| argument.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(arguments, ["--", "--version"]);
    }

    #[test]
    fn job_limit_precedence_is_cli_then_environment_then_manifest() {
        assert_eq!(resolve_max_jobs(Some("2"), Some("3"), 4).unwrap(), 2);
        assert_eq!(resolve_max_jobs(None, Some("3"), 4).unwrap(), 3);
        assert_eq!(resolve_max_jobs(None, None, 4).unwrap(), 4);
        assert!(resolve_max_jobs(Some("0"), None, 4).is_err());
        assert!(resolve_max_jobs(None, Some("many"), 4).is_err());
    }

    #[test]
    fn workspace_redaction_preserves_relative_context_for_nested_and_escaped_paths() {
        let root = PathBuf::from("/home/runner/work/d2b/d2b checkout");
        let alias = PathBuf::from("/actions/workspace/d2b");
        let redactor = WorkspaceRedactor::new(&root, [alias.clone()]);
        let text = format!(
            "nested: {}/packages/xtask/src/layer1/runner.rs\n\
             shell: /home/runner/work/d2b/d2b\\ checkout/packages/a\\ b.rs\n\
             combined: \\/home\\/runner\\/work\\/d2b\\/d2b\\ checkout\\/packages\\/nested.rs\n\
             escaped: \\/actions\\/workspace\\/d2b\\/packages\\/xtask\\/Cargo.toml\n\
             unrelated: /usr/lib/libc.so\n\
             prefix: {}-archive",
            root.display(),
            alias.display()
        );
        let redacted = redactor.redact(&text);

        assert!(!redacted.contains(&root.display().to_string()));
        assert!(!redacted.contains(r"/home/runner/work/d2b/d2b\ checkout"));
        assert!(!redacted.contains(r"\/home\/runner\/work\/d2b\/d2b\ checkout"));
        assert!(!redacted.contains(r"\/actions\/workspace\/d2b"));
        assert!(redacted.contains("nested: ./packages/xtask/src/layer1/runner.rs"));
        assert!(redacted.contains("shell: ./packages/a\\ b.rs"));
        assert!(redacted.contains(r"combined: .\/packages\/nested.rs"));
        assert!(redacted.contains(r"escaped: .\/packages\/xtask\/Cargo.toml"));
        assert!(redacted.contains("unrelated: /usr/lib/libc.so"));
        assert!(redacted.contains("prefix: /actions/workspace/d2b-archive"));
    }

    #[test]
    fn step_summary_reports_success_without_raw_logs() {
        let report = ExecutionReport {
            outcomes: vec![JobOutcome {
                job_id: "test-rust".to_owned(),
                make_target: "test-rust".to_owned(),
                status: JobStatus::Succeeded,
            }],
            failures: Vec::new(),
        };
        let redactor = WorkspaceRedactor::new(Path::new("/checkout/d2b"), []);
        let summary = render_step_summary(&report, &[], &redactor);

        assert_eq!(
            summary,
            "## Layer-1 outcome\n\n✅ All 1 Layer-1 jobs passed.\n"
        );
    }

    #[test]
    fn step_summary_redacts_failed_job_tail_and_reports_blocked_jobs() {
        let report = ExecutionReport {
            outcomes: vec![
                JobOutcome {
                    job_id: "test-rust".to_owned(),
                    make_target: "test-rust".to_owned(),
                    status: JobStatus::Failed(JobResult::ExitCode(17)),
                },
                JobOutcome {
                    job_id: "test-policy".to_owned(),
                    make_target: "test-policy".to_owned(),
                    status: JobStatus::Blocked(vec!["test-rust".to_owned()]),
                },
            ],
            failures: vec![
                JobOutcome {
                    job_id: "test-rust".to_owned(),
                    make_target: "test-rust".to_owned(),
                    status: JobStatus::Failed(JobResult::ExitCode(17)),
                },
                JobOutcome {
                    job_id: "test-policy".to_owned(),
                    make_target: "test-policy".to_owned(),
                    status: JobStatus::Blocked(vec!["test-rust".to_owned()]),
                },
            ],
        };
        let redactor = WorkspaceRedactor::new(
            Path::new("/home/runner/work/d2b/d2b"),
            [PathBuf::from("/actions/checkout")],
        );
        let entries = vec![SummaryEntry {
            job_id: "test-rust".to_owned(),
            lines: vec![
                "at /home/runner/work/d2b/d2b/packages/xtask/src/main.rs:1".to_owned(),
                "also /actions/checkout/packages/xtask/src/lib.rs".to_owned(),
            ],
        }];
        let summary = render_step_summary(&report, &entries, &redactor);

        assert!(summary.contains("❌ 2 of 2 Layer-1 jobs did not pass."));
        assert!(summary.contains("`test-rust` (exit 17)"));
        assert!(summary.contains("`test-policy` (blocked by test-rust)"));
        assert!(summary.contains("at ./packages/xtask/src/main.rs:1"));
        assert!(summary.contains("also ./packages/xtask/src/lib.rs"));
        assert!(!summary.contains("/home/runner/work/d2b/d2b"));
        assert!(!summary.contains("/actions/checkout"));
    }

    #[test]
    fn step_summary_is_byte_bounded_after_path_redaction() {
        let report = ExecutionReport {
            outcomes: vec![JobOutcome {
                job_id: "test-rust".to_owned(),
                make_target: "test-rust".to_owned(),
                status: JobStatus::Failed(JobResult::ExitCode(1)),
            }],
            failures: vec![JobOutcome {
                job_id: "test-rust".to_owned(),
                make_target: "test-rust".to_owned(),
                status: JobStatus::Failed(JobResult::ExitCode(1)),
            }],
        };
        let redactor = WorkspaceRedactor::new(Path::new("/secret/checkout"), []);
        let entries = vec![SummaryEntry {
            job_id: "test-rust".to_owned(),
            lines: vec![format!(
                "/secret/checkout/packages/{}",
                "x".repeat(STEP_SUMMARY_MAX_BYTES * 2)
            )],
        }];
        let summary = render_step_summary(&report, &entries, &redactor);

        assert_eq!(summary.len(), STEP_SUMMARY_MAX_BYTES);
        assert!(summary.ends_with(STEP_SUMMARY_TRUNCATED));
        assert!(!summary.contains("/secret/checkout"));
    }

    #[test]
    fn github_step_summary_is_appended_and_absence_is_a_noop() {
        let sequence = NEXT_LOG_DIR.fetch_add(1, Ordering::Relaxed);
        let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/xtask-layer1-summary-tests")
            .join(format!("{}.{}", std::process::id(), sequence));
        fs::create_dir_all(&directory).expect("summary test directory");
        let path = directory.join("summary.md");
        fs::write(&path, "existing\n").expect("existing summary");
        let report = ExecutionReport {
            outcomes: vec![JobOutcome {
                job_id: "test-rust".to_owned(),
                make_target: "test-rust".to_owned(),
                status: JobStatus::Succeeded,
            }],
            failures: Vec::new(),
        };

        let runner =
            ProcessJobRunner::with_step_summary(directory.clone(), None, Some(path.clone()));
        runner
            .append_step_summary(&report)
            .expect("append step summary");
        let contents = fs::read_to_string(&path).expect("read step summary");
        assert!(contents.starts_with("existing\n"));
        assert!(contents.ends_with("✅ All 1 Layer-1 jobs passed.\n"));

        let without_summary = ProcessJobRunner::with_step_summary(directory.clone(), None, None);
        without_summary
            .append_step_summary(&report)
            .expect("missing summary is a no-op");

        fs::remove_dir_all(directory).expect("remove summary test directory");
    }
}
