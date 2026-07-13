use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Condvar, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use super::{
    Layer1Error, Result,
    model::{JobSpec, Layer1Manifest, LocalPhase, PhaseMode},
};

static NEXT_LOG_DIR: AtomicU64 = AtomicU64::new(1);
const FAILURE_TAIL_LINES: usize = 200;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum JobStatus {
    Succeeded,
    Failed(i32),
    Blocked(Vec<String>),
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
                JobStatus::Failed(code) => {
                    format!("{} (exit {code})", outcome.job_id)
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
    fn run(&self, job_id: &str, job: &JobSpec) -> i32;
}

#[derive(Clone, Debug)]
pub struct ProcessJobRunner {
    root: PathBuf,
}

impl ProcessJobRunner {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn run_process(&self, job_id: &str, job: &JobSpec) -> Result<i32> {
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
        let status = Command::new("make")
            .arg(target)
            .current_dir(&self.root)
            .envs(&job.local_env)
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(stderr_log))
            .status()
            .map_err(|error| {
                Layer1Error::new(format!("could not execute make {target}: {error}"))
            })?;

        if status.success() {
            println!("ok: {target}");
            flush_stdout();
            if env::var("D2B_CHECK_KEEP_LOGS").as_deref() != Ok("1") {
                let _ = fs::remove_file(&log_path);
                let _ = fs::remove_dir(&log_dir);
            }
            return Ok(0);
        }

        let code = status.code().unwrap_or(1);
        eprintln!(
            "FAIL: {target} (exit {code}); tail of {}:",
            log_path.display()
        );
        match tail_lines(&log_path, FAILURE_TAIL_LINES) {
            Ok(lines) => {
                for line in lines {
                    eprintln!("{line}");
                }
            }
            Err(error) => eprintln!("could not read {}: {error}", log_path.display()),
        }
        Ok(code)
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
}

impl LocalJobRunner for ProcessJobRunner {
    fn run(&self, job_id: &str, job: &JobSpec) -> i32 {
        match self.run_process(job_id, job) {
            Ok(code) => code,
            Err(error) => {
                eprintln!("FAIL: Layer-1 job {job_id}: {error}");
                1
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
            rendered.push_str(&format!("  {job_id}: make {target}\n"));
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

fn run_parallel_phase<R: LocalJobRunner>(
    manifest: &Layer1Manifest,
    phase: &LocalPhase,
    max_jobs: usize,
    runner: &R,
    succeeded: &mut BTreeSet<String>,
    outcomes: &mut Vec<JobOutcome>,
) -> Result<()> {
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
                            drop(
                                changed
                                    .wait(scheduler)
                                    .expect("Layer-1 scheduler wait lock"),
                            );
                            continue;
                        }
                        scheduler.running += 1;
                        scheduler.started.insert(job_id.clone());
                        drop(scheduler);

                        let code = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                            runner.run(job_id, job)
                        }))
                        .unwrap_or_else(|_| {
                            eprintln!("FAIL: Layer-1 job worker panicked for {job_id}");
                            1
                        });
                        let status = if code == 0 {
                            JobStatus::Succeeded
                        } else {
                            JobStatus::Failed(code)
                        };
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
    let code = runner.run(job_id, job);
    JobOutcome {
        job_id: job_id.to_owned(),
        make_target: target,
        status: if code == 0 {
            JobStatus::Succeeded
        } else {
            JobStatus::Failed(code)
        },
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
    use std::{collections::BTreeMap, sync::Mutex};

    use super::*;

    #[derive(Default)]
    struct RecordingRunner {
        events: Mutex<Vec<String>>,
        statuses: BTreeMap<String, i32>,
    }

    impl LocalJobRunner for RecordingRunner {
        fn run(&self, job_id: &str, _job: &JobSpec) -> i32 {
            self.events
                .lock()
                .expect("events")
                .push(format!("start:{job_id}"));
            let status = self.statuses.get(job_id).copied().unwrap_or(0);
            self.events
                .lock()
                .expect("events")
                .push(format!("end:{job_id}"));
            status
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
            statuses: BTreeMap::from([("one".to_owned(), 3), ("two".to_owned(), 7)]),
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

    #[test]
    fn job_limit_precedence_is_cli_then_environment_then_manifest() {
        assert_eq!(resolve_max_jobs(Some("2"), Some("3"), 4).unwrap(), 2);
        assert_eq!(resolve_max_jobs(None, Some("3"), 4).unwrap(), 3);
        assert_eq!(resolve_max_jobs(None, None, 4).unwrap(), 4);
        assert!(resolve_max_jobs(Some("0"), None, 4).is_err());
        assert!(resolve_max_jobs(None, Some("many"), 4).is_err());
    }
}
