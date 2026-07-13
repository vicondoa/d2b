use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::Deserialize;

use super::{DeliveryError, Result};

const MAX_COMMAND_OUTPUT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub success: bool,
    pub stdout: Vec<u8>,
}

pub trait CommandOutputAdapter {
    fn output(&self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<CommandOutput>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessCommandOutput;

impl CommandOutputAdapter for ProcessCommandOutput {
    fn output(&self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<CommandOutput> {
        let mut command = Command::new(program);
        command.args(args);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let output = command
            .output()
            .map_err(|error| DeliveryError::new(format!("could not execute {program}: {error}")))?;
        if output.stdout.len() > MAX_COMMAND_OUTPUT_BYTES {
            return Err(DeliveryError::new(format!(
                "{program} output exceeds {MAX_COMMAND_OUTPUT_BYTES} bytes"
            )));
        }
        Ok(CommandOutput {
            success: output.status.success(),
            stdout: output.stdout,
        })
    }
}

pub trait RepositoryProbe {
    fn canonical_root(&self, root: &Path) -> Result<PathBuf>;
    fn git_common_dir(&self, root: &Path) -> Result<PathBuf>;
    fn resolve_commit(&self, root: &Path, revision: &str) -> Result<String>;
    fn resolve_tree(&self, root: &Path, revision: &str) -> Result<String>;
    fn is_dirty(&self, root: &Path) -> Result<bool>;
}

#[derive(Clone, Debug)]
pub struct GitProbe<A> {
    command: A,
}

impl<A> GitProbe<A> {
    pub fn new(command: A) -> Self {
        Self { command }
    }
}

impl<A: CommandOutputAdapter> GitProbe<A> {
    fn git_stdout(&self, root: &Path, arguments: &[&str]) -> Result<String> {
        let root_string = path_string(root)?;
        let mut args = vec!["-C".to_owned(), root_string];
        args.extend(arguments.iter().map(|argument| (*argument).to_owned()));
        let output = self.command.output("git", &args, None)?;
        if !output.success {
            return Err(DeliveryError::new(format!(
                "git {} failed",
                arguments.join(" ")
            )));
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| DeliveryError::new("git output was not UTF-8"))?;
        Ok(stdout.trim().to_owned())
    }
}

impl<A: CommandOutputAdapter> RepositoryProbe for GitProbe<A> {
    fn canonical_root(&self, root: &Path) -> Result<PathBuf> {
        let canonical = fs::canonicalize(root).map_err(|error| {
            DeliveryError::new(format!(
                "cannot canonicalize repository root {}: {error}",
                root.display()
            ))
        })?;
        let reported = self.git_stdout(&canonical, &["rev-parse", "--show-toplevel"])?;
        let reported = fs::canonicalize(reported).map_err(|error| {
            DeliveryError::new(format!("cannot canonicalize Git root: {error}"))
        })?;
        if reported != canonical {
            return Err(DeliveryError::new(format!(
                "{} is not the repository root",
                root.display()
            )));
        }
        Ok(canonical)
    }

    fn git_common_dir(&self, root: &Path) -> Result<PathBuf> {
        let value = self.git_stdout(root, &["rev-parse", "--git-common-dir"])?;
        let path = PathBuf::from(value);
        let absolute = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        fs::canonicalize(&absolute).map_err(|error| {
            DeliveryError::new(format!(
                "cannot canonicalize Git common directory {}: {error}",
                absolute.display()
            ))
        })
    }

    fn resolve_commit(&self, root: &Path, revision: &str) -> Result<String> {
        validate_revision(revision)?;
        self.git_stdout(
            root,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{revision}^{{commit}}"),
            ],
        )
    }

    fn resolve_tree(&self, root: &Path, revision: &str) -> Result<String> {
        validate_revision(revision)?;
        self.git_stdout(
            root,
            &[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{revision}^{{tree}}"),
            ],
        )
    }

    fn is_dirty(&self, root: &Path) -> Result<bool> {
        Ok(!self
            .git_stdout(
                root,
                &["status", "--porcelain=v1", "--untracked-files=normal"],
            )?
            .is_empty())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PullRequestState {
    Open,
    Merged,
    Closed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservedCheckState {
    Successful,
    Failed,
    Pending,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservedCheck {
    pub name: String,
    pub state: ObservedCheckState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PullRequestStatus {
    pub state: PullRequestState,
    pub merge_state: String,
    pub checks: Vec<ObservedCheck>,
}

pub trait PullRequestStatusSource {
    fn status(&self, repository: &str, pr: u64) -> Result<PullRequestStatus>;
}

#[derive(Debug)]
pub struct GhStatusSource<'a, A> {
    command: &'a A,
}

impl<'a, A> GhStatusSource<'a, A> {
    pub fn new(command: &'a A) -> Self {
        Self { command }
    }
}

impl<A: CommandOutputAdapter> PullRequestStatusSource for GhStatusSource<'_, A> {
    fn status(&self, repository: &str, pr: u64) -> Result<PullRequestStatus> {
        let args = vec![
            "pr".to_owned(),
            "view".to_owned(),
            pr.to_string(),
            "--repo".to_owned(),
            repository.to_owned(),
            "--json".to_owned(),
            "state,mergeStateStatus,statusCheckRollup".to_owned(),
        ];
        let output = self.command.output("gh", &args, None)?;
        if !output.success {
            return Err(DeliveryError::new(format!(
                "GitHub status query failed for {repository}#{pr}"
            )));
        }
        parse_gh_status(&output.stdout)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GhStatus {
    state: String,
    #[serde(rename = "mergeStateStatus")]
    merge_state_status: String,
    #[serde(rename = "statusCheckRollup")]
    status_check_rollup: Vec<serde_json::Value>,
}

fn parse_gh_status(bytes: &[u8]) -> Result<PullRequestStatus> {
    let parsed: GhStatus = serde_json::from_slice(bytes)
        .map_err(|error| DeliveryError::new(format!("invalid gh status JSON: {error}")))?;
    let state = match parsed.state.as_str() {
        "OPEN" => PullRequestState::Open,
        "MERGED" => PullRequestState::Merged,
        "CLOSED" => PullRequestState::Closed,
        other => {
            return Err(DeliveryError::new(format!(
                "unknown GitHub PR state {other}"
            )));
        }
    };
    let checks = parsed
        .status_check_rollup
        .iter()
        .map(parse_check)
        .collect::<Result<Vec<_>>>()?;
    Ok(PullRequestStatus {
        state,
        merge_state: parsed.merge_state_status,
        checks,
    })
}

fn parse_check(value: &serde_json::Value) -> Result<ObservedCheck> {
    let object = value
        .as_object()
        .ok_or_else(|| DeliveryError::new("GitHub check entry is not an object"))?;
    let name = object
        .get("name")
        .or_else(|| object.get("context"))
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .ok_or_else(|| DeliveryError::new("GitHub check entry has no name"))?
        .to_owned();

    let state = if let Some(state) = object.get("state").and_then(serde_json::Value::as_str) {
        match state {
            "SUCCESS" => ObservedCheckState::Successful,
            "PENDING" | "EXPECTED" => ObservedCheckState::Pending,
            _ => ObservedCheckState::Failed,
        }
    } else {
        let status = object
            .get("status")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("UNKNOWN");
        if status != "COMPLETED" {
            ObservedCheckState::Pending
        } else {
            match object.get("conclusion").and_then(serde_json::Value::as_str) {
                Some("SUCCESS") => ObservedCheckState::Successful,
                Some(_) => ObservedCheckState::Failed,
                None => ObservedCheckState::Pending,
            }
        }
    };
    Ok(ObservedCheck { name, state })
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DeliveryError::new("repository path is not valid UTF-8"))
}

fn validate_revision(revision: &str) -> Result<()> {
    if revision.trim().is_empty()
        || revision.len() > 1024
        || revision.starts_with('-')
        || revision.chars().any(char::is_control)
    {
        return Err(DeliveryError::new("invalid Git revision"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_check_runs_and_status_contexts() {
        let status = parse_gh_status(
            br#"{
                "state":"OPEN",
                "mergeStateStatus":"CLEAN",
                "statusCheckRollup":[
                    {"name":"unit","status":"COMPLETED","conclusion":"SUCCESS"},
                    {"context":"lint","state":"PENDING"}
                ]
            }"#,
        )
        .expect("valid status");
        assert_eq!(status.state, PullRequestState::Open);
        assert_eq!(
            status.checks,
            vec![
                ObservedCheck {
                    name: "unit".to_owned(),
                    state: ObservedCheckState::Successful,
                },
                ObservedCheck {
                    name: "lint".to_owned(),
                    state: ObservedCheckState::Pending,
                }
            ]
        );
    }

    #[test]
    fn malformed_gh_output_fails_closed() {
        let error = parse_gh_status(br#"{"state":"OPEN"}"#).expect_err("missing fields");
        assert!(error.to_string().contains("invalid gh status JSON"));
    }

    #[test]
    fn rejects_option_shaped_git_revision() {
        let error = validate_revision("--help").expect_err("option-shaped revision");
        assert!(error.to_string().contains("invalid Git revision"));
    }
}
