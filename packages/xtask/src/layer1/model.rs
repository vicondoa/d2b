use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

use super::{LAYER1_MANIFEST_VERSION, Layer1Error, Result};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Layer1Manifest {
    pub version: u32,
    pub local: LocalConfig,
    pub ci: CiConfig,
    pub jobs: BTreeMap<String, JobSpec>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalConfig {
    pub default_jobs: usize,
    pub phases: Vec<LocalPhase>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalPhase {
    pub id: String,
    pub mode: PhaseMode,
    pub jobs: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PhaseMode {
    Serial,
    Parallel,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CiConfig {
    pub workflow_name: String,
    pub jobs: Vec<String>,
    pub rollup_job: String,
    pub rollup_needs: Vec<String>,
    #[serde(default)]
    pub allowed_skipped_rollup_jobs: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct JobSpec {
    pub display_name: String,
    pub make_target: Option<String>,
    #[serde(default)]
    pub local_env: BTreeMap<String, String>,
    pub ci_kind: Option<CiKind>,
    pub ci_job_id: Option<String>,
    #[serde(default)]
    pub needs: Vec<String>,
    pub timeout_minutes: Option<u64>,
    pub runs_on: Option<String>,
    pub max_parallel: Option<usize>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum CiKind {
    Tier0,
    SimpleNix,
    Changelog,
    Rust,
    FlakeDiscover,
    FlakeX86Shards,
    FlakeX86Outputs,
    FlakeX86Rollup,
    FlakeAarch64Smoke,
}

impl Layer1Manifest {
    pub fn validate(&self) -> Result<()> {
        if self.version != LAYER1_MANIFEST_VERSION {
            return Err(Layer1Error::new(format!(
                "unsupported Layer-1 manifest version {}; expected {LAYER1_MANIFEST_VERSION}",
                self.version
            )));
        }
        if self.jobs.is_empty() {
            return Err(Layer1Error::new("jobs must not be empty"));
        }
        validate_token(&self.ci.workflow_name, "ci.workflowName")?;
        self.validate_job_shapes()?;
        self.validate_dependencies()?;
        self.validate_local()?;
        self.validate_ci()?;
        Ok(())
    }

    fn validate_job_shapes(&self) -> Result<()> {
        for (job_id, job) in &self.jobs {
            validate_token(job_id, "job id")?;
            validate_single_line(&job.display_name, &format!("job {job_id} displayName"))?;
            if job.display_name.trim().is_empty() {
                return Err(Layer1Error::new(format!(
                    "job {job_id} displayName must not be empty"
                )));
            }
            if let Some(target) = &job.make_target {
                validate_make_target(target, &format!("job {job_id} makeTarget"))?;
            }
            if let Some(ci_job_id) = &job.ci_job_id {
                validate_github_job_id(ci_job_id, &format!("job {job_id} ciJobId"))?;
            }
            if job.ci_kind.is_some() {
                for dependency in &job.needs {
                    validate_github_job_id(dependency, &format!("CI job {job_id} needs entry"))?;
                }
            }
            for (name, value) in &job.local_env {
                validate_env_name(name, job_id)?;
                if value.contains('\0') {
                    return Err(Layer1Error::new(format!(
                        "job {job_id} localEnv value for {name} contains NUL"
                    )));
                }
            }
            ensure_unique(&job.needs, &format!("job {job_id} dependency"))?;
            if let Some(timeout) = job.timeout_minutes
                && timeout == 0
            {
                return Err(Layer1Error::new(format!(
                    "job {job_id} timeoutMinutes must be >= 1"
                )));
            }
            if let Some(max_parallel) = job.max_parallel
                && max_parallel == 0
            {
                return Err(Layer1Error::new(format!(
                    "job {job_id} maxParallel must be >= 1"
                )));
            }
        }
        Ok(())
    }

    fn validate_dependencies(&self) -> Result<()> {
        for (job_id, job) in &self.jobs {
            for dependency in &job.needs {
                if !self.jobs.contains_key(dependency) {
                    return Err(Layer1Error::new(format!(
                        "job {job_id} references unknown dependency {dependency}"
                    )));
                }
                if dependency == job_id {
                    return Err(Layer1Error::new(format!("job {job_id} depends on itself")));
                }
            }
        }
        detect_cycle(self.jobs.keys().map(String::as_str), |job_id| {
            self.jobs[job_id].needs.iter().map(String::as_str).collect()
        })
    }

    fn validate_local(&self) -> Result<()> {
        if self.local.default_jobs == 0 {
            return Err(Layer1Error::new("local.defaultJobs must be >= 1"));
        }
        let Some(preflight) = self.local.phases.first() else {
            return Err(Layer1Error::new("local.phases must not be empty"));
        };
        if preflight.id != "preflight" || preflight.mode != PhaseMode::Serial {
            return Err(Layer1Error::new(
                "the first local phase must be serial phase preflight",
            ));
        }

        let mut phase_ids = BTreeSet::new();
        let mut local_jobs = BTreeSet::new();
        let mut phase_by_job = BTreeMap::new();
        for (phase_index, phase) in self.local.phases.iter().enumerate() {
            validate_token(&phase.id, "local phase id")?;
            if !phase_ids.insert(phase.id.as_str()) {
                return Err(Layer1Error::new(format!(
                    "duplicate local phase id {}",
                    phase.id
                )));
            }
            if phase.jobs.is_empty() {
                return Err(Layer1Error::new(format!(
                    "local phase {} must contain at least one job",
                    phase.id
                )));
            }
            for (job_index, job_id) in phase.jobs.iter().enumerate() {
                let Some(job) = self.jobs.get(job_id) else {
                    return Err(Layer1Error::new(format!(
                        "local phase {} references unknown job {job_id}",
                        phase.id
                    )));
                };
                if job.make_target.is_none() {
                    return Err(Layer1Error::new(format!(
                        "local job {job_id} has no makeTarget"
                    )));
                }
                if !local_jobs.insert(job_id.as_str()) {
                    return Err(Layer1Error::new(format!(
                        "local job {job_id} appears in more than one phase"
                    )));
                }
                phase_by_job.insert(job_id.as_str(), (phase_index, job_index, phase.mode));
            }
        }

        for job_id in &local_jobs {
            let job = &self.jobs[*job_id];
            let (phase_index, job_index, mode) = phase_by_job[job_id];
            for dependency in &job.needs {
                let Some((dependency_phase, dependency_index, _)) =
                    phase_by_job.get(dependency.as_str()).copied()
                else {
                    return Err(Layer1Error::new(format!(
                        "local job {job_id} depends on non-local job {dependency}"
                    )));
                };
                if dependency_phase > phase_index
                    || (dependency_phase == phase_index
                        && mode == PhaseMode::Serial
                        && dependency_index >= job_index)
                {
                    return Err(Layer1Error::new(format!(
                        "local dependency {dependency} must run before {job_id}"
                    )));
                }
            }
        }
        Ok(())
    }

    fn validate_ci(&self) -> Result<()> {
        validate_github_job_id(&self.ci.rollup_job, "ci.rollupJob")?;
        if self.ci.rollup_job != "check" {
            return Err(Layer1Error::new(
                "ci.rollupJob must be the stable required context check",
            ));
        }
        if self.jobs.contains_key(&self.ci.rollup_job) {
            return Err(Layer1Error::new(
                "ci.rollupJob must not collide with a declared job",
            ));
        }
        ensure_unique(&self.ci.jobs, "ci.jobs entry")?;
        ensure_unique(&self.ci.rollup_needs, "ci.rollupNeeds entry")?;
        ensure_unique(
            &self.ci.allowed_skipped_rollup_jobs,
            "ci.allowedSkippedRollupJobs entry",
        )?;
        if self.ci.jobs.is_empty() || self.ci.rollup_needs.is_empty() {
            return Err(Layer1Error::new(
                "ci.jobs and ci.rollupNeeds must not be empty",
            ));
        }

        for job_id in &self.ci.jobs {
            validate_github_job_id(job_id, "ci.jobs entry")?;
        }
        for job_id in &self.ci.rollup_needs {
            validate_github_job_id(job_id, "ci.rollupNeeds entry")?;
        }
        for job_id in &self.ci.allowed_skipped_rollup_jobs {
            validate_github_job_id(job_id, "ci.allowedSkippedRollupJobs entry")?;
        }

        let ci_jobs = self
            .ci
            .jobs
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut rendered_job_ids = BTreeMap::new();
        for job_id in &self.ci.jobs {
            let Some(job) = self.jobs.get(job_id) else {
                return Err(Layer1Error::new(format!(
                    "ci.jobs references unknown job {job_id}"
                )));
            };
            let Some(kind) = job.ci_kind else {
                return Err(Layer1Error::new(format!("CI job {job_id} has no ciKind")));
            };
            let Some(ci_job_id) = job.ci_job_id.as_deref() else {
                return Err(Layer1Error::new(format!("CI job {job_id} has no ciJobId")));
            };
            validate_github_job_id(ci_job_id, &format!("CI job {job_id} ciJobId"))?;
            if let Some(existing) = rendered_job_ids.insert(ci_job_id, job_id.as_str()) {
                return Err(Layer1Error::new(format!(
                    "GitHub job id collision: jobs {existing} and {job_id} both render as {ci_job_id}"
                )));
            }
            if ci_job_id != job_id {
                return Err(Layer1Error::new(format!(
                    "CI job {job_id} ciJobId must equal its manifest job id"
                )));
            }
            if job.timeout_minutes.is_none() || job.runs_on.is_none() {
                return Err(Layer1Error::new(format!(
                    "CI job {job_id} requires timeoutMinutes and runsOn"
                )));
            }
            validate_single_line(
                job.runs_on.as_deref().unwrap_or_default(),
                &format!("CI job {job_id} runsOn"),
            )?;
            if job.runs_on.as_deref().is_none_or(str::is_empty) {
                return Err(Layer1Error::new(format!(
                    "CI job {job_id} runsOn must not be empty"
                )));
            }
            for dependency in &job.needs {
                validate_github_job_id(dependency, &format!("CI job {job_id} needs entry"))?;
                if !ci_jobs.contains(dependency.as_str()) {
                    return Err(Layer1Error::new(format!(
                        "CI job {job_id} depends on job {dependency} absent from ci.jobs"
                    )));
                }
            }
            validate_kind_fields(job_id, job, kind)?;
        }
        if let Some(existing) = rendered_job_ids.insert(self.ci.rollup_job.as_str(), "<rollup>") {
            return Err(Layer1Error::new(format!(
                "GitHub job id collision: job {existing} and ci.rollupJob both render as {}",
                self.ci.rollup_job
            )));
        }

        detect_cycle(ci_jobs.iter().copied(), |job_id| {
            self.jobs[job_id].needs.iter().map(String::as_str).collect()
        })?;
        self.validate_ci_kind_dependencies()?;

        let rollup_needs = self
            .ci
            .rollup_needs
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        for need in &rollup_needs {
            if !ci_jobs.contains(need) {
                return Err(Layer1Error::new(format!(
                    "ci.rollupNeeds references unknown CI job {need}"
                )));
            }
        }
        for skipped in &self.ci.allowed_skipped_rollup_jobs {
            if !rollup_needs.contains(skipped.as_str()) {
                return Err(Layer1Error::new(format!(
                    "allowed skipped rollup job {skipped} is absent from ci.rollupNeeds"
                )));
            }
        }

        let mut covered = BTreeSet::new();
        let mut pending = self.ci.rollup_needs.clone();
        while let Some(job_id) = pending.pop() {
            if covered.insert(job_id.clone()) {
                pending.extend(self.jobs[&job_id].needs.iter().cloned());
            }
        }
        let uncovered = self
            .ci
            .jobs
            .iter()
            .filter(|job_id| !covered.contains(*job_id))
            .cloned()
            .collect::<Vec<_>>();
        if !uncovered.is_empty() {
            return Err(Layer1Error::new(format!(
                "ci.rollupNeeds does not cover CI job(s): {}",
                uncovered.join(", ")
            )));
        }

        let skipped = self
            .ci
            .allowed_skipped_rollup_jobs
            .iter()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let mut guaranteed = BTreeSet::new();
        let mut pending = self
            .ci
            .rollup_needs
            .iter()
            .filter(|job_id| !skipped.contains(job_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        while let Some(job_id) = pending.pop() {
            if guaranteed.insert(job_id.clone()) {
                pending.extend(self.jobs[&job_id].needs.iter().cloned());
            }
        }
        let skippable_only = self
            .ci
            .jobs
            .iter()
            .filter(|job_id| !skipped.contains(job_id.as_str()) && !guaranteed.contains(*job_id))
            .cloned()
            .collect::<Vec<_>>();
        if !skippable_only.is_empty() {
            return Err(Layer1Error::new(format!(
                "ci.rollupNeeds must cover required CI job(s) through a non-skippable rollup root: {}",
                skippable_only.join(", ")
            )));
        }

        let local_or_ci = self
            .local
            .phases
            .iter()
            .flat_map(|phase| phase.jobs.iter())
            .chain(self.ci.jobs.iter())
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        let unused = self
            .jobs
            .keys()
            .filter(|job_id| !local_or_ci.contains(job_id.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        if !unused.is_empty() {
            return Err(Layer1Error::new(format!(
                "unused Layer-1 job(s): {}",
                unused.join(", ")
            )));
        }
        Ok(())
    }

    fn validate_ci_kind_dependencies(&self) -> Result<()> {
        for job_id in &self.ci.jobs {
            let job = &self.jobs[job_id];
            let dependency_kinds = job
                .needs
                .iter()
                .map(|dependency| {
                    self.jobs[dependency].ci_kind.ok_or_else(|| {
                        Layer1Error::new(format!(
                            "CI dependency {dependency} of {job_id} has no ciKind"
                        ))
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let expected = match job.ci_kind.expect("validated ciKind") {
                CiKind::FlakeX86Shards => Some(vec![CiKind::FlakeDiscover]),
                CiKind::FlakeX86Rollup => Some(vec![
                    CiKind::FlakeDiscover,
                    CiKind::FlakeX86Shards,
                    CiKind::FlakeX86Outputs,
                ]),
                _ => None,
            };
            if let Some(expected) = expected
                && dependency_kinds != expected
            {
                return Err(Layer1Error::new(format!(
                    "CI job {job_id} has invalid dependencies for {:?}: expected {:?}, found {:?}",
                    job.ci_kind.expect("validated ciKind"),
                    expected,
                    dependency_kinds
                )));
            }
        }
        Ok(())
    }
}

fn validate_kind_fields(job_id: &str, job: &JobSpec, kind: CiKind) -> Result<()> {
    let needs_make_target = matches!(kind, CiKind::Tier0 | CiKind::SimpleNix | CiKind::Rust);
    if needs_make_target && job.make_target.is_none() {
        return Err(Layer1Error::new(format!(
            "CI job {job_id} with ciKind {kind:?} requires makeTarget"
        )));
    }
    if kind == CiKind::FlakeX86Shards {
        if job.max_parallel.is_none() {
            return Err(Layer1Error::new(format!(
                "CI job {job_id} with ciKind flake-x86-shards requires maxParallel"
            )));
        }
    } else if job.max_parallel.is_some() {
        return Err(Layer1Error::new(format!(
            "CI job {job_id} may set maxParallel only for flake-x86-shards"
        )));
    }
    Ok(())
}

fn detect_cycle<'a, I, F>(nodes: I, dependencies: F) -> Result<()>
where
    I: IntoIterator<Item = &'a str>,
    F: Fn(&str) -> Vec<&'a str>,
{
    #[derive(Clone, Copy, Eq, PartialEq)]
    enum Visit {
        Visiting,
        Complete,
    }

    fn visit<'a, F>(
        node: &'a str,
        states: &mut BTreeMap<&'a str, Visit>,
        dependencies: &F,
    ) -> Result<()>
    where
        F: Fn(&str) -> Vec<&'a str>,
    {
        match states.get(node) {
            Some(Visit::Visiting) => {
                return Err(Layer1Error::new(format!(
                    "Layer-1 job dependency cycle includes {node}"
                )));
            }
            Some(Visit::Complete) => return Ok(()),
            None => {}
        }
        states.insert(node, Visit::Visiting);
        for dependency in dependencies(node) {
            visit(dependency, states, dependencies)?;
        }
        states.insert(node, Visit::Complete);
        Ok(())
    }

    let nodes = nodes.into_iter().collect::<Vec<_>>();
    let mut states = BTreeMap::new();
    for node in nodes {
        visit(node, &mut states, &dependencies)?;
    }
    Ok(())
}

fn ensure_unique(values: &[String], label: &str) -> Result<()> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !seen.insert(value.as_str()) {
            return Err(Layer1Error::new(format!("duplicate {label} {value}")));
        }
    }
    Ok(())
}

fn validate_token(value: &str, label: &str) -> Result<()> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(Layer1Error::new(format!("invalid {label} {value:?}")));
    }
    Ok(())
}

fn validate_make_target(value: &str, label: &str) -> Result<()> {
    let mut bytes = value.bytes();
    let valid_first = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || byte == b'_');
    if !valid_first
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(Layer1Error::new(format!(
            "invalid {label} {value:?}; expected [A-Za-z0-9_][A-Za-z0-9_.-]*"
        )));
    }
    Ok(())
}

fn validate_github_job_id(value: &str, label: &str) -> Result<()> {
    let mut bytes = value.bytes();
    let valid_first = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_');
    if !valid_first
        || !bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(Layer1Error::new(format!(
            "invalid GitHub job id for {label}: {value:?}"
        )));
    }
    Ok(())
}

fn validate_single_line(value: &str, label: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err(Layer1Error::new(format!(
            "{label} must be a single printable line"
        )));
    }
    Ok(())
}

fn validate_env_name(name: &str, job_id: &str) -> Result<()> {
    let mut bytes = name.bytes();
    let valid_first = bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_');
    if !valid_first || !bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_') {
        return Err(Layer1Error::new(format!(
            "job {job_id} has invalid localEnv name {name:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_json() -> serde_json::Value {
        serde_json::json!({
            "version": 1,
            "local": {
                "defaultJobs": 2,
                "phases": [
                    {"id": "preflight", "mode": "serial", "jobs": ["pre"]},
                    {"id": "parallel", "mode": "parallel", "jobs": ["one", "two"]},
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
            "jobs": {
                "pre": {
                    "displayName": "pre",
                    "makeTarget": "pre",
                    "ciKind": "tier0",
                    "ciJobId": "pre",
                    "timeoutMinutes": 1,
                    "runsOn": "ubuntu-latest"
                },
                "one": {
                    "displayName": "one",
                    "makeTarget": "one",
                    "ciKind": "simple-nix",
                    "ciJobId": "one",
                    "needs": ["pre"],
                    "timeoutMinutes": 1,
                    "runsOn": "ubuntu-latest"
                },
                "two": {
                    "displayName": "two",
                    "makeTarget": "two",
                    "ciKind": "simple-nix",
                    "ciJobId": "two",
                    "needs": ["one"],
                    "timeoutMinutes": 1,
                    "runsOn": "ubuntu-latest"
                },
                "after": {
                    "displayName": "after",
                    "makeTarget": "after",
                    "ciKind": "simple-nix",
                    "ciJobId": "after",
                    "needs": ["two"],
                    "timeoutMinutes": 1,
                    "runsOn": "ubuntu-latest"
                }
            }
        })
    }

    fn manifest() -> Layer1Manifest {
        serde_json::from_value(manifest_json()).expect("typed manifest")
    }

    #[test]
    fn accepts_typed_manifest() {
        manifest().validate().expect("valid manifest");
    }

    #[test]
    fn manifest_errors_fail_closed() {
        let mut unknown_job = manifest();
        unknown_job.local.phases[1].jobs[0] = "missing".to_owned();
        assert!(
            unknown_job
                .validate()
                .expect_err("unknown job")
                .to_string()
                .contains("unknown job")
        );

        let mut unknown_dependency = manifest();
        unknown_dependency.jobs.get_mut("one").expect("one").needs = vec!["missing".to_owned()];
        assert!(
            unknown_dependency
                .validate()
                .expect_err("unknown dependency")
                .to_string()
                .contains("unknown dependency")
        );

        let mut cycle = manifest();
        cycle.jobs.get_mut("one").expect("one").needs = vec!["two".to_owned()];
        assert!(
            cycle
                .validate()
                .expect_err("cycle")
                .to_string()
                .contains("cycle")
        );

        let mut invalid_rollup = manifest();
        invalid_rollup.ci.rollup_needs = vec!["one".to_owned()];
        assert!(
            invalid_rollup
                .validate()
                .expect_err("uncovered jobs")
                .to_string()
                .contains("does not cover")
        );
    }

    #[test]
    fn skippable_rollup_roots_cannot_be_the_only_required_coverage_path() {
        let mut all_skippable = manifest();
        all_skippable.ci.allowed_skipped_rollup_jobs = vec!["after".to_owned()];
        let error = all_skippable
            .validate()
            .expect_err("all required jobs are covered only by a skippable root");
        assert!(error.to_string().contains("non-skippable rollup root"));

        let mut transitive_gap = manifest();
        transitive_gap.ci.rollup_needs = vec!["after".to_owned(), "one".to_owned()];
        transitive_gap.ci.allowed_skipped_rollup_jobs = vec!["after".to_owned()];
        let error = transitive_gap
            .validate()
            .expect_err("two is covered only through the skippable after root");
        let message = error.to_string();
        assert!(message.contains("non-skippable rollup root"));
        assert!(message.contains("two"));
    }

    #[test]
    fn skippable_rollup_root_is_valid_when_required_dependencies_are_guaranteed() {
        let mut manifest = manifest();
        manifest.ci.rollup_needs = vec!["after".to_owned(), "two".to_owned()];
        manifest.ci.allowed_skipped_rollup_jobs = vec!["after".to_owned()];
        manifest
            .validate()
            .expect("two guarantees every dependency and after is explicitly skippable");
    }

    #[test]
    fn unsafe_make_targets_fail_closed_including_option_bypass() {
        for target in [
            "--version",
            "-s",
            ".DEFAULT",
            "name with space",
            "name\nnext",
            "name;echo",
            "$(command)",
            "NAME=value",
            "path/target",
        ] {
            let mut candidate = manifest();
            candidate.jobs.get_mut("pre").expect("pre").make_target = Some(target.to_owned());
            let error = candidate
                .validate()
                .expect_err("unsafe make target must be rejected");
            assert!(
                error.to_string().contains("makeTarget"),
                "unexpected error for {target:?}: {error}"
            );
        }
    }

    #[test]
    fn github_job_id_grammar_and_collisions_fail_closed() {
        for valid in ["job", "_job", "Job_9", "job-name"] {
            validate_github_job_id(valid, "test").expect("valid GitHub job id");
        }
        for invalid in ["1job", ".job", "-job", "job.name", "job/name", "job name"] {
            assert!(
                validate_github_job_id(invalid, "test").is_err(),
                "{invalid:?} must be rejected"
            );
        }

        let mut invalid_ci_id = manifest();
        invalid_ci_id.jobs.get_mut("one").expect("one").ci_job_id = Some("1one".to_owned());
        assert!(
            invalid_ci_id
                .validate()
                .expect_err("digit-first ciJobId")
                .to_string()
                .contains("invalid GitHub job id")
        );

        let mut invalid_need = manifest();
        invalid_need.jobs.get_mut("one").expect("one").needs = vec!["pre.job".to_owned()];
        assert!(
            invalid_need
                .validate()
                .expect_err("dotted needs entry")
                .to_string()
                .contains("invalid GitHub job id")
        );

        let mut invalid_rollup = manifest();
        invalid_rollup.ci.rollup_job = ".check".to_owned();
        assert!(
            invalid_rollup
                .validate()
                .expect_err("dotted rollup id")
                .to_string()
                .contains("invalid GitHub job id")
        );

        let mut collision = manifest();
        collision.jobs.get_mut("two").expect("two").ci_job_id = Some("one".to_owned());
        assert!(
            collision
                .validate()
                .expect_err("duplicate rendered id")
                .to_string()
                .contains("collision")
        );
    }

    #[test]
    fn serde_rejects_unknown_kind_and_field() {
        let mut unknown_kind = manifest_json();
        unknown_kind["jobs"]["one"]["ciKind"] = serde_json::json!("mystery");
        assert!(
            serde_json::from_value::<Layer1Manifest>(unknown_kind)
                .expect_err("unknown kind")
                .to_string()
                .contains("unknown variant")
        );

        let mut unknown_field = manifest_json();
        unknown_field["jobs"]["one"]["command"] = serde_json::json!("make one");
        assert!(
            serde_json::from_value::<Layer1Manifest>(unknown_field)
                .expect_err("unknown field")
                .to_string()
                .contains("unknown field")
        );
    }
}
