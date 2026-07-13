use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::Deserialize;

use super::{DeliveryError, Result};

const MAX_COMMAND_OUTPUT_BYTES: usize = 4 * 1024 * 1024;
pub const GH_STACK_VERSION: &str = "0.0.7";
const GH_STACK_CANNOT_OPERATE: &str = "cannot operate: official gh-stack private preview is \
    unavailable or unverifiable; no fallback stack mutation is permitted";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GhStackPreviewRecord {
    id: u64,
    pull_requests: Vec<u64>,
}

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

pub fn check_gh_stack_private_preview<A: CommandOutputAdapter>(
    command: &A,
    repository: &str,
) -> Result<()> {
    validate_repository_slug(repository)?;

    let version = command
        .output("gh-stack", &["--version".to_owned()], None)
        .map_err(|_| DeliveryError::new(GH_STACK_CANNOT_OPERATE))?;
    if !version.success {
        return Err(DeliveryError::new(GH_STACK_CANNOT_OPERATE));
    }
    let version = String::from_utf8(version.stdout)
        .map_err(|_| DeliveryError::new(GH_STACK_CANNOT_OPERATE))?;
    let expected = format!("gh stack version {GH_STACK_VERSION}");
    if version.trim() != expected {
        return Err(DeliveryError::new(format!(
            "cannot operate: expected official gh-stack {GH_STACK_VERSION}; \
             no fallback stack mutation is permitted"
        )));
    }

    let path = format!("repos/{repository}/cli_internal/pulls/stacks");
    let response = command
        .output(
            "gh",
            &[
                "api".to_owned(),
                "--method".to_owned(),
                "GET".to_owned(),
                path,
            ],
            None,
        )
        .map_err(|_| DeliveryError::new(GH_STACK_CANNOT_OPERATE))?;
    if !response.success {
        return Err(DeliveryError::new(GH_STACK_CANNOT_OPERATE));
    }
    let stacks: Vec<GhStackPreviewRecord> = serde_json::from_slice(&response.stdout)
        .map_err(|_| DeliveryError::new(GH_STACK_CANNOT_OPERATE))?;
    if stacks
        .iter()
        .any(|stack| stack.id == 0 || stack.pull_requests.contains(&0))
    {
        return Err(DeliveryError::new(GH_STACK_CANNOT_OPERATE));
    }
    Ok(())
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

fn validate_repository_slug(repository: &str) -> Result<()> {
    let parts = repository.split('/').collect::<Vec<_>>();
    if parts.len() != 2
        || parts.iter().any(|part| {
            part.is_empty()
                || part.len() > 100
                || part.starts_with('.')
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
    {
        return Err(DeliveryError::new("invalid GitHub repository identity"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::VecDeque};

    use super::*;

    struct FakeCommand {
        outputs: RefCell<VecDeque<CommandOutput>>,
        calls: RefCell<Vec<(String, Vec<String>)>>,
    }

    impl FakeCommand {
        fn new(outputs: impl IntoIterator<Item = CommandOutput>) -> Self {
            Self {
                outputs: RefCell::new(outputs.into_iter().collect()),
                calls: RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandOutputAdapter for FakeCommand {
        fn output(
            &self,
            program: &str,
            args: &[String],
            _cwd: Option<&Path>,
        ) -> Result<CommandOutput> {
            self.calls
                .borrow_mut()
                .push((program.to_owned(), args.to_vec()));
            self.outputs
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| DeliveryError::new("missing fake command output"))
        }
    }

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

    #[test]
    fn gh_stack_private_preview_probe_is_read_only_and_version_pinned() {
        let command = FakeCommand::new([
            CommandOutput {
                success: true,
                stdout: b"gh stack version 0.0.7\n".to_vec(),
            },
            CommandOutput {
                success: true,
                stdout: b"[]\n".to_vec(),
            },
        ]);

        check_gh_stack_private_preview(&command, "example/d2b").expect("available");

        assert_eq!(
            *command.calls.borrow(),
            vec![
                ("gh-stack".to_owned(), vec!["--version".to_owned()]),
                (
                    "gh".to_owned(),
                    vec![
                        "api".to_owned(),
                        "--method".to_owned(),
                        "GET".to_owned(),
                        "repos/example/d2b/cli_internal/pulls/stacks".to_owned(),
                    ],
                ),
            ]
        );
    }

    #[test]
    fn gh_stack_private_preview_failure_has_no_mutating_fallback() {
        let command = FakeCommand::new([
            CommandOutput {
                success: true,
                stdout: b"gh stack version 0.0.7\n".to_vec(),
            },
            CommandOutput {
                success: false,
                stdout: Vec::new(),
            },
        ]);

        let error =
            check_gh_stack_private_preview(&command, "example/d2b").expect_err("unavailable");
        assert!(error.to_string().contains("cannot operate"));
        assert!(error.to_string().contains("no fallback stack mutation"));
        assert_eq!(command.calls.borrow().len(), 2);
    }

    #[test]
    fn gh_stack_version_or_malformed_preview_response_fails_closed() {
        let wrong_version = FakeCommand::new([CommandOutput {
            success: true,
            stdout: b"gh stack version 0.0.6\n".to_vec(),
        }]);
        let error = check_gh_stack_private_preview(&wrong_version, "example/d2b")
            .expect_err("wrong version");
        assert!(
            error
                .to_string()
                .contains("expected official gh-stack 0.0.7")
        );

        let malformed = FakeCommand::new([
            CommandOutput {
                success: true,
                stdout: b"gh stack version 0.0.7\n".to_vec(),
            },
            CommandOutput {
                success: true,
                stdout: br#"{"unexpected":true}"#.to_vec(),
            },
        ]);
        let error =
            check_gh_stack_private_preview(&malformed, "example/d2b").expect_err("malformed");
        assert!(error.to_string().contains("cannot operate"));

        for payload in [
            br#"[null]"#.as_slice(),
            br#"[{"unexpected":true}]"#.as_slice(),
            br#"[{"id":0,"pull_requests":[1]}]"#.as_slice(),
            br#"[{"id":1,"pull_requests":[0]}]"#.as_slice(),
        ] {
            let malformed_array = FakeCommand::new([
                CommandOutput {
                    success: true,
                    stdout: b"gh stack version 0.0.7\n".to_vec(),
                },
                CommandOutput {
                    success: true,
                    stdout: payload.to_vec(),
                },
            ]);
            check_gh_stack_private_preview(&malformed_array, "example/d2b")
                .expect_err("malformed stack record");
        }

        let valid = FakeCommand::new([
            CommandOutput {
                success: true,
                stdout: b"gh stack version 0.0.7\n".to_vec(),
            },
            CommandOutput {
                success: true,
                stdout: br#"[{"id":7,"pull_requests":[101,102]}]"#.to_vec(),
            },
        ]);
        check_gh_stack_private_preview(&valid, "example/d2b").expect("typed stack record");
    }
}
