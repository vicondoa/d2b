use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    io::Read,
    os::{fd::AsFd, unix::process::CommandExt},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, TryRecvError},
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{
    DeliveryError, Result,
    model::{
        CheckPublisher, CheckPublisherKind, GhStackGraph, GitObjectFormat, PullRequestState,
        validate_git_ref, validate_hash, validate_repository_id,
    },
};

pub const DEFAULT_COMMAND_OUTPUT_BYTES: usize = 1024 * 1024;
pub const MAX_GIT_BLOB_BYTES: usize = 16 * 1024 * 1024;
const MAX_COMMAND_TIMEOUT: Duration = Duration::from_secs(24 * 60 * 60);
const DEFAULT_COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
pub const GH_STACK_VERSION: &str = "0.0.7";
const GH_STACK_CANNOT_OPERATE: &str = "cannot operate: official gh-stack private preview is \
    unavailable or unverifiable; no fallback stack mutation is permitted";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CommandLimits {
    pub stdout_bytes: usize,
    pub stderr_bytes: usize,
    pub timeout: Duration,
}

impl Default for CommandLimits {
    fn default() -> Self {
        Self {
            stdout_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
            stderr_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
            timeout: DEFAULT_COMMAND_TIMEOUT,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GhStackPreviewRecord {
    id: u64,
    pull_requests: Vec<u64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

pub trait CommandOutputAdapter {
    fn output_with_limits(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        limits: CommandLimits,
    ) -> Result<CommandOutput>;

    fn output_with_environment(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        environment: &BTreeMap<OsString, OsString>,
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        let _ = environment;
        self.output_with_limits(program, args, cwd, limits)
    }

    fn output(&self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<CommandOutput> {
        self.output_with_limits(program, args, cwd, CommandLimits::default())
    }
}

impl<A: CommandOutputAdapter + ?Sized> CommandOutputAdapter for &A {
    fn output_with_limits(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        (**self).output_with_limits(program, args, cwd, limits)
    }

    fn output_with_environment(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        environment: &BTreeMap<OsString, OsString>,
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        (**self).output_with_environment(program, args, cwd, environment, limits)
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessCommandOutput;

impl CommandOutputAdapter for ProcessCommandOutput {
    fn output_with_limits(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        self.output_with_environment(program, args, cwd, &BTreeMap::new(), limits)
    }

    fn output_with_environment(
        &self,
        program: &str,
        args: &[String],
        cwd: Option<&Path>,
        environment: &BTreeMap<OsString, OsString>,
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        if limits.stdout_bytes == 0
            || limits.stderr_bytes == 0
            || limits.stdout_bytes > MAX_GIT_BLOB_BYTES
            || limits.stderr_bytes > MAX_GIT_BLOB_BYTES
            || limits.timeout.is_zero()
            || limits.timeout > MAX_COMMAND_TIMEOUT
        {
            return Err(DeliveryError::new(
                "command limits are zero or exceed hard process bounds",
            ));
        }
        let mut command = Command::new(program);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        command.envs(environment);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let started = Instant::now();
        let mut child = command
            .spawn()
            .map_err(|error| DeliveryError::new(format!("could not execute {program}: {error}")))?;
        let process_group = i32::try_from(child.id())
            .ok()
            .and_then(rustix::process::Pid::from_raw);
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| DeliveryError::new("child stdout pipe was not available"))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| DeliveryError::new("child stderr pipe was not available"))?;
        let stdout_overflow = Arc::new(AtomicBool::new(false));
        let stderr_overflow = Arc::new(AtomicBool::new(false));
        let cancel_readers = Arc::new(AtomicBool::new(false));
        let (stdout_reader, stdout_rx) = spawn_capped_reader(
            stdout,
            limits.stdout_bytes,
            &stdout_overflow,
            &cancel_readers,
        );
        let (stderr_reader, stderr_rx) = spawn_capped_reader(
            stderr,
            limits.stderr_bytes,
            &stderr_overflow,
            &cancel_readers,
        );

        let mut timed_out = false;
        let mut terminated = false;
        let mut status = None;
        let mut stdout_result = None;
        let mut stderr_result = None;
        loop {
            receive_reader(&stdout_rx, &mut stdout_result)?;
            receive_reader(&stderr_rx, &mut stderr_result)?;
            if status.is_none() {
                status = child.try_wait().map_err(|error| {
                    DeliveryError::new(format!("could not poll {program}: {error}"))
                })?;
            }
            let deadline_reached = started.elapsed() >= limits.timeout;
            let overflowed =
                stdout_overflow.load(Ordering::Acquire) || stderr_overflow.load(Ordering::Acquire);
            if (deadline_reached || overflowed) && !terminated {
                timed_out = deadline_reached;
                terminated = true;
                cancel_readers.store(true, Ordering::Release);
                if let Some(process_group) = process_group {
                    let _ = rustix::process::kill_process_group(
                        process_group,
                        rustix::process::Signal::Kill,
                    );
                }
                let _ = child.kill();
            }
            if status.is_some() && stdout_result.is_some() && stderr_result.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(5));
        }
        let status = match status {
            Some(status) => status,
            None => child.wait().map_err(|error| {
                DeliveryError::new(format!("could not reap {program}: {error}"))
            })?,
        };

        let stdout = stdout_result.expect("reader completion was checked")?;
        let stderr = stderr_result.expect("reader completion was checked")?;
        stdout_reader
            .join()
            .map_err(|_| DeliveryError::new("stdout reader panicked"))?;
        stderr_reader
            .join()
            .map_err(|_| DeliveryError::new("stderr reader panicked"))?;
        if timed_out {
            return Err(DeliveryError::new(format!(
                "{program} exceeded its command timeout"
            )));
        }
        if stdout_overflow.load(Ordering::Acquire) {
            return Err(DeliveryError::new(format!(
                "{program} stdout exceeds {} bytes",
                limits.stdout_bytes
            )));
        }
        if stderr_overflow.load(Ordering::Acquire) {
            return Err(DeliveryError::new(format!(
                "{program} stderr exceeds {} bytes",
                limits.stderr_bytes
            )));
        }
        Ok(CommandOutput {
            success: status.success(),
            exit_code: status.code(),
            stdout,
            stderr,
        })
    }
}

fn spawn_capped_reader<R: Read + AsFd + Send + 'static>(
    mut reader: R,
    limit: usize,
    overflow: &Arc<AtomicBool>,
    cancel: &Arc<AtomicBool>,
) -> (thread::JoinHandle<()>, Receiver<Result<Vec<u8>>>) {
    let overflow = Arc::clone(overflow);
    let cancel = Arc::clone(cancel);
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let result = (|| {
            let flags = rustix::fs::fcntl_getfl(&reader).map_err(|error| {
                DeliveryError::new(format!("cannot inspect child output pipe: {error}"))
            })?;
            rustix::fs::fcntl_setfl(&reader, flags | rustix::fs::OFlags::NONBLOCK).map_err(
                |error| {
                    DeliveryError::new(format!(
                        "cannot make child output pipe nonblocking: {error}"
                    ))
                },
            )?;
            let mut output = Vec::with_capacity(limit.min(64 * 1024));
            let mut buffer = [0_u8; 8192];
            loop {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                let read = match reader.read(&mut buffer) {
                    Ok(read) => read,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(2));
                        continue;
                    }
                    Err(error) => {
                        return Err(DeliveryError::new(format!(
                            "cannot read child output: {error}"
                        )));
                    }
                };
                if read == 0 {
                    break;
                }
                let remaining = limit.saturating_sub(output.len());
                output.extend_from_slice(&buffer[..read.min(remaining)]);
                if read > remaining {
                    overflow.store(true, Ordering::Release);
                    break;
                }
            }
            Ok(output)
        })();
        let _ = tx.send(result);
    });
    (handle, rx)
}

fn receive_reader(
    receiver: &Receiver<Result<Vec<u8>>>,
    slot: &mut Option<Result<Vec<u8>>>,
) -> Result<()> {
    if slot.is_some() {
        return Ok(());
    }
    match receiver.try_recv() {
        Ok(result) => {
            *slot = Some(result);
            Ok(())
        }
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(DeliveryError::new(
            "child output reader disconnected before completion",
        )),
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedBlob {
    pub oid: String,
    pub mode: String,
    pub bytes: Vec<u8>,
}

pub trait RepositoryProbe {
    fn canonical_root(&self, root: &Path) -> Result<PathBuf>;
    fn repository_identity(&self, root: &Path) -> Result<String>;
    fn git_common_dir(&self, root: &Path) -> Result<PathBuf>;
    fn object_format(&self, root: &Path) -> Result<GitObjectFormat>;
    fn resolve_commit(&self, root: &Path, revision: &str) -> Result<String>;
    fn tree_for_commit(&self, root: &Path, commit_oid: &str) -> Result<String>;
    fn is_dirty(&self, root: &Path) -> Result<bool>;
    fn is_ancestor(&self, root: &Path, ancestor: &str, descendant: &str) -> Result<bool>;
    fn tracked_blob(&self, root: &Path, commit_oid: &str, path: &Path) -> Result<TrackedBlob>;
    fn canonical_diff(
        &self,
        root: &Path,
        base_oid: &str,
        head_oid: &str,
        paths: &[PathBuf],
    ) -> Result<Vec<u8>>;
    fn prospective_merge_tree(&self, root: &Path, base_oid: &str, head_oid: &str)
    -> Result<String>;
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
    fn git_output(
        &self,
        root: &Path,
        arguments: &[String],
        limits: CommandLimits,
    ) -> Result<CommandOutput> {
        let root_string = path_string(root)?;
        let mut args = vec!["-C".to_owned(), root_string];
        args.extend_from_slice(arguments);
        self.command.output_with_limits("git", &args, None, limits)
    }

    fn git_stdout(&self, root: &Path, arguments: &[String]) -> Result<String> {
        let output = self.git_output(root, arguments, CommandLimits::default())?;
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
        reject_symlink_components(root)?;
        let canonical = fs::canonicalize(root).map_err(|error| {
            DeliveryError::new(format!(
                "cannot canonicalize repository root {}: {error}",
                root.display()
            ))
        })?;
        let reported = self.git_stdout(
            &canonical,
            &["rev-parse".to_owned(), "--show-toplevel".to_owned()],
        )?;
        reject_symlink_components(Path::new(&reported))?;
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

    fn repository_identity(&self, root: &Path) -> Result<String> {
        let remote = self.git_stdout(
            root,
            &[
                "config".to_owned(),
                "--get".to_owned(),
                "remote.origin.url".to_owned(),
            ],
        )?;
        parse_repository_identity(&remote)
    }

    fn git_common_dir(&self, root: &Path) -> Result<PathBuf> {
        let value = self.git_stdout(
            root,
            &["rev-parse".to_owned(), "--git-common-dir".to_owned()],
        )?;
        let path = PathBuf::from(value);
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        reject_symlink_components(&path)?;
        fs::canonicalize(&path).map_err(|error| {
            DeliveryError::new(format!(
                "cannot canonicalize Git common directory {}: {error}",
                path.display()
            ))
        })
    }

    fn object_format(&self, root: &Path) -> Result<GitObjectFormat> {
        match self
            .git_stdout(
                root,
                &[
                    "rev-parse".to_owned(),
                    "--show-object-format=storage".to_owned(),
                ],
            )?
            .as_str()
        {
            "sha1" => Ok(GitObjectFormat::Sha1),
            "sha256" => Ok(GitObjectFormat::Sha256),
            value => Err(DeliveryError::new(format!(
                "unsupported Git object format {value}"
            ))),
        }
    }

    fn resolve_commit(&self, root: &Path, revision: &str) -> Result<String> {
        validate_git_ref(revision, "Git revision")?;
        let oid = self.git_stdout(
            root,
            &[
                "rev-parse".to_owned(),
                "--verify".to_owned(),
                "--end-of-options".to_owned(),
                format!("{revision}^{{commit}}"),
            ],
        )?;
        validate_hash(&oid, "resolved commit")?;
        Ok(oid)
    }

    fn tree_for_commit(&self, root: &Path, commit_oid: &str) -> Result<String> {
        validate_hash(commit_oid, "commit OID")?;
        let tree = self.git_stdout(
            root,
            &[
                "rev-parse".to_owned(),
                "--verify".to_owned(),
                "--end-of-options".to_owned(),
                format!("{commit_oid}^{{tree}}"),
            ],
        )?;
        validate_hash(&tree, "resolved tree")?;
        Ok(tree)
    }

    fn is_dirty(&self, root: &Path) -> Result<bool> {
        Ok(!self
            .git_stdout(
                root,
                &[
                    "status".to_owned(),
                    "--porcelain=v1".to_owned(),
                    "--untracked-files=normal".to_owned(),
                ],
            )?
            .is_empty())
    }

    fn is_ancestor(&self, root: &Path, ancestor: &str, descendant: &str) -> Result<bool> {
        validate_hash(ancestor, "ancestor commit")?;
        validate_hash(descendant, "descendant commit")?;
        let output = self.git_output(
            root,
            &[
                "merge-base".to_owned(),
                "--is-ancestor".to_owned(),
                ancestor.to_owned(),
                descendant.to_owned(),
            ],
            CommandLimits::default(),
        )?;
        match output.exit_code {
            Some(0) => Ok(true),
            Some(1) => Ok(false),
            _ => Err(DeliveryError::new("git merge-base --is-ancestor failed")),
        }
    }

    fn tracked_blob(&self, root: &Path, commit_oid: &str, path: &Path) -> Result<TrackedBlob> {
        validate_hash(commit_oid, "blob commit")?;
        super::model::validate_repo_relative_path(path)?;
        let path_string = path_string(path)?;
        let listing = self.git_output(
            root,
            &[
                "ls-tree".to_owned(),
                "-z".to_owned(),
                commit_oid.to_owned(),
                "--".to_owned(),
                format!(":(literal){path_string}"),
            ],
            CommandLimits::default(),
        )?;
        if !listing.success {
            return Err(DeliveryError::new(format!(
                "cannot inspect tracked blob {path_string}"
            )));
        }
        let entries = listing
            .stdout
            .split(|byte| *byte == 0)
            .filter(|entry| !entry.is_empty())
            .collect::<Vec<_>>();
        if entries.len() != 1 {
            return Err(DeliveryError::new(format!(
                "tracked fingerprint path must resolve to exactly one Git blob: {path_string}"
            )));
        }
        let entry = std::str::from_utf8(entries[0])
            .map_err(|_| DeliveryError::new("git ls-tree output was not UTF-8"))?;
        let (metadata, listed_path) = entry
            .split_once('\t')
            .ok_or_else(|| DeliveryError::new("malformed git ls-tree output"))?;
        if listed_path != path_string {
            return Err(DeliveryError::new(
                "git ls-tree returned a different fingerprint path",
            ));
        }
        let fields = metadata.split_ascii_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 || !matches!(fields[0], "100644" | "100755") || fields[1] != "blob" {
            return Err(DeliveryError::new(format!(
                "fingerprint path is not a tracked regular Git blob: {path_string}"
            )));
        }
        validate_hash(fields[2], "tracked blob OID")?;
        let payload = self.git_output(
            root,
            &[
                "cat-file".to_owned(),
                "blob".to_owned(),
                fields[2].to_owned(),
            ],
            CommandLimits {
                stdout_bytes: MAX_GIT_BLOB_BYTES,
                stderr_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
                timeout: DEFAULT_COMMAND_TIMEOUT,
            },
        )?;
        if !payload.success {
            return Err(DeliveryError::new(format!(
                "cannot read tracked Git blob for {path_string}"
            )));
        }
        Ok(TrackedBlob {
            oid: fields[2].to_owned(),
            mode: fields[0].to_owned(),
            bytes: payload.stdout,
        })
    }

    fn canonical_diff(
        &self,
        root: &Path,
        base_oid: &str,
        head_oid: &str,
        paths: &[PathBuf],
    ) -> Result<Vec<u8>> {
        validate_hash(base_oid, "diff base OID")?;
        validate_hash(head_oid, "diff head OID")?;
        let mut arguments = vec![
            "diff".to_owned(),
            "--binary".to_owned(),
            "--no-ext-diff".to_owned(),
            "--no-renames".to_owned(),
            "--full-index".to_owned(),
            "--src-prefix=a/".to_owned(),
            "--dst-prefix=b/".to_owned(),
            base_oid.to_owned(),
            head_oid.to_owned(),
        ];
        if !paths.is_empty() {
            arguments.push("--".to_owned());
            let mut normalized = paths.to_vec();
            normalized.sort();
            normalized.dedup();
            for path in normalized {
                super::model::validate_repo_relative_path(&path)?;
                arguments.push(format!(":(literal){}", path_string(&path)?));
            }
        }
        let output = self.git_output(
            root,
            &arguments,
            CommandLimits {
                stdout_bytes: MAX_GIT_BLOB_BYTES,
                stderr_bytes: DEFAULT_COMMAND_OUTPUT_BYTES,
                timeout: DEFAULT_COMMAND_TIMEOUT,
            },
        )?;
        if !output.success {
            return Err(DeliveryError::new(
                "cannot compute canonical base-to-head Git diff",
            ));
        }
        Ok(output.stdout)
    }

    fn prospective_merge_tree(
        &self,
        root: &Path,
        base_oid: &str,
        head_oid: &str,
    ) -> Result<String> {
        validate_hash(base_oid, "merge base OID")?;
        validate_hash(head_oid, "merge head OID")?;
        let output = self.git_output(
            root,
            &[
                "merge-tree".to_owned(),
                "--write-tree".to_owned(),
                base_oid.to_owned(),
                head_oid.to_owned(),
            ],
            CommandLimits::default(),
        )?;
        if !output.success {
            return Err(DeliveryError::new(
                "prospective merge tree has conflicts or could not be computed",
            ));
        }
        let stdout = String::from_utf8(output.stdout)
            .map_err(|_| DeliveryError::new("git merge-tree output was not UTF-8"))?;
        let tree = stdout
            .lines()
            .next()
            .ok_or_else(|| DeliveryError::new("git merge-tree returned no tree OID"))?
            .trim()
            .to_owned();
        validate_hash(&tree, "prospective merge tree")?;
        Ok(tree)
    }
}

pub trait StackGraphSource {
    fn graph(&self, repository: &str, checkout_root: &Path) -> Result<GhStackGraph>;
}

#[derive(Debug)]
pub struct GhStackSource<'a, A> {
    command: &'a A,
}

impl<'a, A> GhStackSource<'a, A> {
    pub fn new(command: &'a A) -> Self {
        Self { command }
    }
}

impl<A: CommandOutputAdapter> StackGraphSource for GhStackSource<'_, A> {
    fn graph(&self, repository: &str, checkout_root: &Path) -> Result<GhStackGraph> {
        validate_repository_id(repository)?;
        let output = self.command.output(
            "gh",
            &["stack".to_owned(), "view".to_owned(), "--json".to_owned()],
            Some(checkout_root),
        )?;
        if !output.success {
            return Err(DeliveryError::new(format!(
                "gh stack view --json failed for {repository}"
            )));
        }
        let graph: GhStackGraph = serde_json::from_slice(&output.stdout)
            .map_err(|error| DeliveryError::new(format!("invalid gh-stack JSON: {error}")))?;
        graph.validate()?;
        Ok(graph)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObservedCheckState {
    Successful,
    Failed,
    Pending,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedCheck {
    pub name: String,
    pub publisher: CheckPublisher,
    pub check_run_id: Option<u64>,
    pub workflow_run_id: Option<u64>,
    pub status: String,
    pub conclusion: String,
    pub state: ObservedCheckState,
    pub commit_oid: String,
    pub started_at_unix_seconds: u64,
    pub completed_at_unix_seconds: Option<u64>,
    pub workflow_created_at_unix_seconds: Option<u64>,
    pub workflow_updated_at_unix_seconds: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PullRequestStatus {
    pub repository: String,
    pub number: u64,
    pub state: PullRequestState,
    pub merge_state: String,
    pub base_ref: String,
    pub base_oid: String,
    pub head_repository: String,
    pub head_ref: String,
    pub head_oid: String,
    pub merge_commit_oid: Option<String>,
    pub merge_commit_tree_oid: Option<String>,
    pub is_in_merge_queue: bool,
    pub is_merge_queue_enabled: bool,
    pub merge_queue_entry: Option<ObservedMergeQueueEntry>,
    pub checks: Vec<ObservedCheck>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservedMergeQueueEntry {
    pub id: String,
    pub state: String,
    pub base_oid: String,
    pub head_oid: String,
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

const PR_STATUS_QUERY: &str = r#"query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){
    nameWithOwner
    pullRequest(number:$number){
      number state mergeStateStatus baseRefName baseRefOid headRefName headRefOid
      isInMergeQueue isMergeQueueEnabled
      mergeQueueEntry{id state baseCommit{oid} headCommit{oid}}
      headRepository{nameWithOwner}
      mergeCommit{oid tree{oid}}
      commits(last:1){
        nodes{commit{oid statusCheckRollup{contexts(first:100){
          nodes{
            __typename
            ... on CheckRun{
              databaseId name status conclusion startedAt completedAt
              checkSuite{
                app{slug databaseId}
                commit{oid}
                workflowRun{
                  databaseId createdAt updatedAt
                  workflow{name databaseId}
                }
              }
            }
            ... on StatusContext{context state createdAt creator{login} commit{oid}}
          }
          pageInfo{hasNextPage}
        }}}}
      }
    }
  }
}"#;

impl<A: CommandOutputAdapter> PullRequestStatusSource for GhStatusSource<'_, A> {
    fn status(&self, repository: &str, pr: u64) -> Result<PullRequestStatus> {
        let (owner, name) = github_owner_name(repository)?;
        let args = vec![
            "api".to_owned(),
            "graphql".to_owned(),
            "-f".to_owned(),
            format!("query={PR_STATUS_QUERY}"),
            "-F".to_owned(),
            format!("owner={owner}"),
            "-F".to_owned(),
            format!("name={name}"),
            "-F".to_owned(),
            format!("number={pr}"),
        ];
        let output = self.command.output("gh", &args, None)?;
        if !output.success {
            return Err(DeliveryError::new(format!(
                "GitHub status query failed for {repository}#{pr}"
            )));
        }
        parse_gh_status(repository, pr, &output.stdout)
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlEnvelope {
    data: GraphQlData,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlData {
    repository: Option<GraphQlRepository>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlRepository {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
    #[serde(rename = "pullRequest")]
    pull_request: Option<GraphQlPullRequest>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlPullRequest {
    number: u64,
    state: String,
    #[serde(rename = "mergeStateStatus")]
    merge_state_status: String,
    #[serde(rename = "baseRefName")]
    base_ref_name: String,
    #[serde(rename = "baseRefOid")]
    base_ref_oid: String,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    #[serde(rename = "headRefOid")]
    head_ref_oid: String,
    #[serde(rename = "headRepository")]
    head_repository: GraphQlName,
    #[serde(rename = "mergeCommit")]
    merge_commit: Option<GraphQlMergeCommit>,
    #[serde(rename = "isInMergeQueue")]
    is_in_merge_queue: bool,
    #[serde(rename = "isMergeQueueEnabled")]
    is_merge_queue_enabled: bool,
    #[serde(rename = "mergeQueueEntry")]
    merge_queue_entry: Option<GraphQlMergeQueueEntry>,
    commits: GraphQlCommits,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlMergeCommit {
    oid: String,
    tree: GraphQlOid,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlMergeQueueEntry {
    id: String,
    state: String,
    #[serde(rename = "baseCommit")]
    base_commit: Option<GraphQlOid>,
    #[serde(rename = "headCommit")]
    head_commit: Option<GraphQlOid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlOid {
    oid: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlName {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlCommits {
    nodes: Vec<GraphQlCommitNode>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlCommitNode {
    commit: GraphQlCommit,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlCommit {
    oid: String,
    #[serde(rename = "statusCheckRollup")]
    status_check_rollup: Option<GraphQlRollup>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlRollup {
    contexts: GraphQlContexts,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlContexts {
    nodes: Vec<serde_json::Value>,
    #[serde(rename = "pageInfo")]
    page_info: GraphQlPageInfo,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct GraphQlPageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
}

pub(crate) fn parse_gh_status(
    repository: &str,
    expected_pr: u64,
    bytes: &[u8],
) -> Result<PullRequestStatus> {
    validate_repository_id(repository)?;
    let parsed: GraphQlEnvelope = serde_json::from_slice(bytes)
        .map_err(|error| DeliveryError::new(format!("invalid GitHub status JSON: {error}")))?;
    let repo = parsed
        .data
        .repository
        .ok_or_else(|| DeliveryError::new("GitHub repository was not found"))?;
    if !repository_matches(repository, &repo.name_with_owner) {
        return Err(DeliveryError::new(
            "GitHub response repository identity does not match",
        ));
    }
    let pr = repo
        .pull_request
        .ok_or_else(|| DeliveryError::new("GitHub pull request was not found"))?;
    if pr.number != expected_pr {
        return Err(DeliveryError::new(
            "GitHub response PR number does not match",
        ));
    }
    let state = match pr.state.as_str() {
        "OPEN" => PullRequestState::Open,
        "MERGED" => PullRequestState::Merged,
        "CLOSED" => PullRequestState::Closed,
        other => {
            return Err(DeliveryError::new(format!(
                "unknown GitHub PR state {other}"
            )));
        }
    };
    let (merge_commit_oid, merge_commit_tree_oid) = match (state, &pr.merge_commit) {
        (PullRequestState::Merged, Some(commit)) => {
            validate_hash(&commit.oid, "GitHub merge commit OID")?;
            validate_hash(&commit.tree.oid, "GitHub merge commit tree OID")?;
            (Some(commit.oid.clone()), Some(commit.tree.oid.clone()))
        }
        (PullRequestState::Merged, None) => {
            return Err(DeliveryError::new(
                "merged GitHub PR has no exact merge commit authority",
            ));
        }
        (_, None) => (None, None),
        (_, Some(_)) => {
            return Err(DeliveryError::new(
                "unmerged GitHub PR unexpectedly has merge commit authority",
            ));
        }
    };
    if pr.commits.nodes.len() != 1 {
        return Err(DeliveryError::new(
            "GitHub status response must contain exactly one latest commit",
        ));
    }
    let commit = &pr.commits.nodes[0].commit;
    if commit.oid != pr.head_ref_oid {
        return Err(DeliveryError::new(
            "GitHub check rollup is not associated with the PR head commit",
        ));
    }
    let mut checks = Vec::new();
    if let Some(rollup) = &commit.status_check_rollup {
        if rollup.contexts.page_info.has_next_page {
            return Err(DeliveryError::new(
                "GitHub check rollup exceeds the supported 100 contexts",
            ));
        }
        for value in &rollup.contexts.nodes {
            checks.push(parse_check(value, &commit.oid)?);
        }
    }
    checks.sort();
    let mut names = BTreeSet::new();
    for check in &checks {
        if !names.insert(check.name.as_str()) {
            return Err(DeliveryError::new(format!(
                "duplicate same-name GitHub check publisher for {}",
                check.name
            )));
        }
    }
    validate_hash(&pr.base_ref_oid, "GitHub base OID")?;
    validate_hash(&pr.head_ref_oid, "GitHub head OID")?;
    validate_git_ref(&pr.base_ref_name, "GitHub base ref")?;
    validate_git_ref(&pr.head_ref_name, "GitHub head ref")?;
    let merge_queue_entry = match pr.merge_queue_entry {
        Some(entry) => {
            if !pr.is_in_merge_queue {
                return Err(DeliveryError::new(
                    "GitHub returned a merge-queue entry for a PR outside the queue",
                ));
            }
            let base_oid = entry
                .base_commit
                .ok_or_else(|| DeliveryError::new("merge-queue entry has no base commit"))?
                .oid;
            let head_oid = entry
                .head_commit
                .ok_or_else(|| DeliveryError::new("merge-queue entry has no head commit"))?
                .oid;
            validate_bounded_github_value(&entry.id, "merge-queue entry ID")?;
            validate_bounded_github_value(&entry.state, "merge-queue entry state")?;
            validate_hash(&base_oid, "merge-queue base OID")?;
            validate_hash(&head_oid, "merge-queue head OID")?;
            Some(ObservedMergeQueueEntry {
                id: entry.id,
                state: entry.state,
                base_oid,
                head_oid,
            })
        }
        None if pr.is_in_merge_queue => {
            return Err(DeliveryError::new(
                "GitHub reports a queued PR without exact merge-queue authority",
            ));
        }
        None => None,
    };
    Ok(PullRequestStatus {
        repository: repository.to_owned(),
        number: pr.number,
        state,
        merge_state: pr.merge_state_status,
        base_ref: pr.base_ref_name,
        base_oid: pr.base_ref_oid,
        head_repository: logical_github_id(&pr.head_repository.name_with_owner),
        head_ref: pr.head_ref_name,
        head_oid: pr.head_ref_oid,
        merge_commit_oid,
        merge_commit_tree_oid,
        is_in_merge_queue: pr.is_in_merge_queue,
        is_merge_queue_enabled: pr.is_merge_queue_enabled,
        merge_queue_entry,
        checks,
    })
}

fn validate_bounded_github_value(value: &str, label: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > super::model::MAX_STRING_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(DeliveryError::new(format!("invalid GitHub {label}")));
    }
    Ok(())
}

fn parse_check(value: &serde_json::Value, outer_commit: &str) -> Result<ObservedCheck> {
    let object = value
        .as_object()
        .ok_or_else(|| DeliveryError::new("GitHub check entry is not an object"))?;
    let kind = required_string(object, "__typename")?;
    let check = match kind {
        "CheckRun" => {
            let suite = object
                .get("checkSuite")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| DeliveryError::new("GitHub check run has no checkSuite"))?;
            let commit_oid = nested_string(suite, "commit", "oid")?;
            if commit_oid != outer_commit {
                return Err(DeliveryError::new(
                    "GitHub check suite is associated with a different commit",
                ));
            }
            let name = required_string(object, "name")?.to_owned();
            let check_run_id = required_u64(object, "databaseId")?;
            let status = required_string(object, "status")?.to_owned();
            let conclusion = object
                .get("conclusion")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("NONE")
                .to_owned();
            let started_at_unix_seconds =
                parse_github_timestamp(required_string(object, "startedAt")?)?;
            let completed_at_unix_seconds = optional_timestamp(object, "completedAt")?;
            let (workflow, workflow_id, workflow_run_id, workflow_created, workflow_updated) =
                optional_workflow_run(suite)?;
            let app_slug = nested_string(suite, "app", "slug")?.to_owned();
            let app_id = nested_u64(suite, "app", "databaseId")?;
            let state = check_run_state(&status, &conclusion)?;
            if state == ObservedCheckState::Successful && completed_at_unix_seconds.is_none() {
                return Err(DeliveryError::new(
                    "successful GitHub check run has no completion timestamp",
                ));
            }
            ObservedCheck {
                name,
                publisher: CheckPublisher {
                    kind: CheckPublisherKind::CheckRun,
                    app_slug,
                    app_id,
                    workflow,
                    workflow_id,
                },
                check_run_id: Some(check_run_id),
                workflow_run_id,
                status,
                conclusion,
                state,
                commit_oid: commit_oid.to_owned(),
                started_at_unix_seconds,
                completed_at_unix_seconds,
                workflow_created_at_unix_seconds: workflow_created,
                workflow_updated_at_unix_seconds: workflow_updated,
            }
        }
        "StatusContext" => {
            let commit_oid = nested_string(object, "commit", "oid")?;
            if commit_oid != outer_commit {
                return Err(DeliveryError::new(
                    "GitHub status context is associated with a different commit",
                ));
            }
            let name = required_string(object, "context")?.to_owned();
            let status = required_string(object, "state")?.to_owned();
            let app_slug = nested_string(object, "creator", "login")?.to_owned();
            let created_at = parse_github_timestamp(required_string(object, "createdAt")?)?;
            let state = match status.as_str() {
                "SUCCESS" => ObservedCheckState::Successful,
                "PENDING" | "EXPECTED" => ObservedCheckState::Pending,
                "ERROR" | "FAILURE" => ObservedCheckState::Failed,
                other => {
                    return Err(DeliveryError::new(format!(
                        "unknown GitHub status context state {other}"
                    )));
                }
            };
            ObservedCheck {
                name,
                publisher: CheckPublisher {
                    kind: CheckPublisherKind::StatusContext,
                    app_slug,
                    app_id: 0,
                    workflow: "status-context".to_owned(),
                    workflow_id: 0,
                },
                check_run_id: None,
                workflow_run_id: None,
                status: status.clone(),
                conclusion: status,
                state,
                commit_oid: commit_oid.to_owned(),
                started_at_unix_seconds: created_at,
                completed_at_unix_seconds: Some(created_at),
                workflow_created_at_unix_seconds: None,
                workflow_updated_at_unix_seconds: None,
            }
        }
        other => {
            return Err(DeliveryError::new(format!(
                "unknown GitHub check publisher type {other}"
            )));
        }
    };
    Ok(check)
}

fn check_run_state(status: &str, conclusion: &str) -> Result<ObservedCheckState> {
    match status {
        "QUEUED" | "IN_PROGRESS" | "WAITING" | "PENDING" | "REQUESTED" => {
            Ok(ObservedCheckState::Pending)
        }
        "COMPLETED" => match conclusion {
            "SUCCESS" => Ok(ObservedCheckState::Successful),
            "ACTION_REQUIRED" | "CANCELLED" | "FAILURE" | "NEUTRAL" | "SKIPPED" | "STALE"
            | "TIMED_OUT" | "STARTUP_FAILURE" => Ok(ObservedCheckState::Failed),
            "NONE" | "" => Ok(ObservedCheckState::Pending),
            other => Err(DeliveryError::new(format!(
                "unknown GitHub check conclusion {other}"
            ))),
        },
        other => Err(DeliveryError::new(format!(
            "unknown GitHub check status {other}"
        ))),
    }
}

fn required_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<&'a str> {
    object
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| DeliveryError::new(format!("GitHub check has no {field}")))
}

fn nested_string<'a>(
    object: &'a serde_json::Map<String, serde_json::Value>,
    parent: &str,
    field: &str,
) -> Result<&'a str> {
    object
        .get(parent)
        .and_then(serde_json::Value::as_object)
        .and_then(|nested| nested.get(field))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| {
            DeliveryError::new(format!(
                "GitHub check has no {parent}.{field} publisher identity"
            ))
        })
}

fn nested_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    parent: &str,
    field: &str,
) -> Result<u64> {
    object
        .get(parent)
        .and_then(serde_json::Value::as_object)
        .and_then(|nested| nested.get(field))
        .and_then(serde_json::Value::as_u64)
        .filter(|value| *value != 0)
        .ok_or_else(|| {
            DeliveryError::new(format!(
                "GitHub check has no {parent}.{field} publisher identity"
            ))
        })
}

fn required_u64(object: &serde_json::Map<String, serde_json::Value>, field: &str) -> Result<u64> {
    object
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .filter(|value| *value != 0)
        .ok_or_else(|| DeliveryError::new(format!("GitHub check has no {field}")))
}

type WorkflowRunIdentity = (String, u64, Option<u64>, Option<u64>, Option<u64>);

fn optional_workflow_run(
    check_suite: &serde_json::Map<String, serde_json::Value>,
) -> Result<WorkflowRunIdentity> {
    match check_suite.get("workflowRun") {
        None | Some(serde_json::Value::Null) => Ok(("none".to_owned(), 0, None, None, None)),
        Some(serde_json::Value::Object(run)) => {
            let run_id = required_u64(run, "databaseId")?;
            let created = parse_github_timestamp(required_string(run, "createdAt")?)?;
            let updated = parse_github_timestamp(required_string(run, "updatedAt")?)?;
            if updated < created {
                return Err(DeliveryError::new(
                    "GitHub workflow run timestamps are out of order",
                ));
            }
            let workflow = run
                .get("workflow")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| DeliveryError::new("GitHub workflow run has no workflow"))?;
            let name = workflow
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| DeliveryError::new("GitHub check workflow has no name"))?;
            let id = workflow
                .get("databaseId")
                .and_then(serde_json::Value::as_u64)
                .filter(|id| *id != 0)
                .ok_or_else(|| DeliveryError::new("GitHub check workflow has no databaseId"))?;
            Ok((
                name.to_owned(),
                id,
                Some(run_id),
                Some(created),
                Some(updated),
            ))
        }
        Some(_) => Err(DeliveryError::new(
            "GitHub check workflowRun is not an object or null",
        )),
    }
}

fn optional_timestamp(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &str,
) -> Result<Option<u64>> {
    match object.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(value)) => parse_github_timestamp(value).map(Some),
        Some(_) => Err(DeliveryError::new(format!(
            "GitHub check {field} is not a timestamp or null"
        ))),
    }
}

pub(crate) fn parse_github_timestamp(value: &str) -> Result<u64> {
    let bytes = value.as_bytes();
    if bytes.len() != 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes[10] != b'T'
        || bytes[13] != b':'
        || bytes[16] != b':'
        || bytes[19] != b'Z'
    {
        return Err(DeliveryError::new(
            "GitHub timestamp is not canonical UTC RFC3339",
        ));
    }
    let year = decimal(bytes, 0, 4)? as i64;
    let month = decimal(bytes, 5, 2)? as i64;
    let day = decimal(bytes, 8, 2)? as i64;
    let hour = decimal(bytes, 11, 2)? as i64;
    let minute = decimal(bytes, 14, 2)? as i64;
    let second = decimal(bytes, 17, 2)? as i64;
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return Err(DeliveryError::new("GitHub timestamp is out of range"));
    }
    let days = days_from_civil(year, month, day);
    if days < 0 {
        return Err(DeliveryError::new(
            "GitHub timestamp predates the Unix epoch",
        ));
    }
    u64::try_from(days * 86_400 + hour * 3_600 + minute * 60 + second)
        .map_err(|_| DeliveryError::new("GitHub timestamp exceeds supported range"))
}

fn decimal(bytes: &[u8], offset: usize, length: usize) -> Result<u32> {
    bytes[offset..offset + length]
        .iter()
        .try_fold(0_u32, |value, byte| {
            byte.is_ascii_digit()
                .then(|| value * 10 + u32::from(byte - b'0'))
                .ok_or_else(|| DeliveryError::new("GitHub timestamp contains a non-digit"))
        })
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

pub trait PullRequestMerger {
    fn merge_with_expected_base_and_head(
        &self,
        repository: &str,
        pr: u64,
        expected_base: &str,
        expected_head: &str,
    ) -> Result<()>;
}

#[derive(Debug)]
pub struct GhMergeSource<'a, A> {
    command: &'a A,
}

impl<'a, A> GhMergeSource<'a, A> {
    pub fn new(command: &'a A) -> Self {
        Self { command }
    }
}

impl<A: CommandOutputAdapter> PullRequestMerger for GhMergeSource<'_, A> {
    fn merge_with_expected_base_and_head(
        &self,
        repository: &str,
        pr: u64,
        expected_base: &str,
        expected_head: &str,
    ) -> Result<()> {
        validate_repository_id(repository)?;
        if pr == 0 {
            return Err(DeliveryError::new("merge PR number must be non-zero"));
        }
        validate_hash(expected_base, "expected merge base")?;
        validate_hash(expected_head, "expected merge head")?;
        let _ = &self.command;
        Err(DeliveryError::new(format!(
            "refusing GitHub merge for {repository}#{pr}: gh exposes expected-head protection but no exact base+head compare-and-swap or verified merge-group authority"
        )))
    }
}

fn github_owner_name(repository: &str) -> Result<(&str, &str)> {
    validate_repository_id(repository)?;
    let parts = repository.split('/').collect::<Vec<_>>();
    if parts.len() == 3 && parts[0] == "github.com" {
        return Ok((parts[1], parts[2]));
    }
    Err(DeliveryError::new(format!(
        "GitHub repository identity must be github.com/owner/name: {repository}"
    )))
}

fn logical_github_id(name_with_owner: &str) -> String {
    format!("github.com/{name_with_owner}")
}

fn repository_matches(logical: &str, name_with_owner: &str) -> bool {
    logical == logical_github_id(name_with_owner)
}

fn parse_repository_identity(remote: &str) -> Result<String> {
    let path = if let Some(path) = remote.strip_prefix("https://github.com/") {
        path
    } else if let Some(path) = remote.strip_prefix("http://github.com/") {
        path
    } else if let Some(path) = remote.strip_prefix("git@github.com:") {
        path
    } else if let Some(path) = remote.strip_prefix("ssh://git@github.com/") {
        path
    } else {
        return Err(DeliveryError::new(
            "origin remote is not an unambiguous github.com repository identity",
        ));
    };
    let path = path.trim_end_matches('/').trim_end_matches(".git");
    if path.split('/').count() != 2 {
        return Err(DeliveryError::new(
            "origin remote does not contain exact owner/repository identity",
        ));
    }
    let identity = format!("github.com/{path}");
    validate_repository_id(&identity)?;
    Ok(identity)
}

fn path_string(path: &Path) -> Result<String> {
    path.to_str()
        .map(str::to_owned)
        .ok_or_else(|| DeliveryError::new("path is not valid UTF-8"))
}

pub fn reject_symlink_components(path: &Path) -> Result<()> {
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let mut current = PathBuf::new();
    for component in absolute.components() {
        match component {
            Component::RootDir | Component::Prefix(_) => current.push(component.as_os_str()),
            Component::CurDir => {}
            Component::ParentDir => {
                return Err(DeliveryError::new(format!(
                    "path contains parent traversal: {}",
                    path.display()
                )));
            }
            Component::Normal(name) => {
                current.push(name);
                match fs::symlink_metadata(&current) {
                    Ok(metadata) if metadata.file_type().is_symlink() => {
                        return Err(DeliveryError::new(format!(
                            "path contains a symlink component: {}",
                            current.display()
                        )));
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                    Err(error) => {
                        return Err(DeliveryError::new(format!(
                            "cannot inspect path component {}: {error}",
                            current.display()
                        )));
                    }
                }
            }
        }
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

    type CommandCall = (String, Vec<String>, Option<PathBuf>);

    struct FakeCommand {
        calls: RefCell<Vec<CommandCall>>,
        outputs: RefCell<VecDeque<CommandOutput>>,
    }

    impl FakeCommand {
        fn new(outputs: impl IntoIterator<Item = CommandOutput>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                outputs: RefCell::new(outputs.into_iter().collect()),
            }
        }
    }

    impl CommandOutputAdapter for FakeCommand {
        fn output_with_limits(
            &self,
            program: &str,
            args: &[String],
            cwd: Option<&Path>,
            _limits: CommandLimits,
        ) -> Result<CommandOutput> {
            self.calls.borrow_mut().push((
                program.to_owned(),
                args.to_vec(),
                cwd.map(Path::to_path_buf),
            ));
            self.outputs
                .borrow_mut()
                .pop_front()
                .ok_or_else(|| DeliveryError::new("missing fake command output"))
        }
    }

    fn successful_output(stdout: Vec<u8>) -> CommandOutput {
        CommandOutput {
            success: true,
            exit_code: Some(0),
            stdout,
            stderr: vec![],
        }
    }

    fn graphql(checks: serde_json::Value) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "data": {
                "repository": {
                    "nameWithOwner": "example/d2b",
                    "pullRequest": {
                        "number": 42,
                        "state": "OPEN",
                        "mergeStateStatus": "CLEAN",
                        "baseRefName": "main",
                        "baseRefOid": "a".repeat(40),
                        "headRefName": "feature",
                        "headRefOid": "b".repeat(40),
                        "headRepository": {"nameWithOwner": "example/d2b"},
                        "mergeCommit": null,
                        "isInMergeQueue": false,
                        "isMergeQueueEnabled": false,
                        "mergeQueueEntry": null,
                        "commits": {"nodes": [{
                            "commit": {
                                "oid": "b".repeat(40),
                                "statusCheckRollup": {
                                    "contexts": {
                                        "nodes": checks,
                                        "pageInfo": {"hasNextPage": false}
                                    }
                                }
                            }
                        }]}
                    }
                }
            }
        }))
        .expect("JSON")
    }

    fn check(name: &str, app: &str, status: &str, conclusion: &str) -> serde_json::Value {
        serde_json::json!({
            "__typename": "CheckRun",
            "databaseId": 9876,
            "name": name,
            "status": status,
            "conclusion": conclusion,
            "startedAt": "2026-07-13T10:00:00Z",
            "completedAt": "2026-07-13T10:01:00Z",
            "checkSuite": {
                "app": {"slug": app, "databaseId": 15368},
                "commit": {"oid": "b".repeat(40)},
                "workflowRun": {
                    "databaseId": 6543,
                    "createdAt": "2026-07-13T09:59:00Z",
                    "updatedAt": "2026-07-13T10:01:00Z",
                    "workflow": {"name": "Layer 1", "databaseId": 321}
                }
            }
        })
    }

    #[test]
    fn parses_exact_check_publisher_and_commit_association() {
        let status = parse_gh_status(
            "github.com/example/d2b",
            42,
            &graphql(serde_json::json!([check(
                "check",
                "github-actions",
                "COMPLETED",
                "SUCCESS"
            )])),
        )
        .expect("status");
        assert_eq!(status.head_oid, "b".repeat(40));
        assert_eq!(status.checks[0].publisher.app_slug, "github-actions");
        assert_eq!(status.checks[0].state, ObservedCheckState::Successful);
        assert_eq!(status.checks[0].check_run_id, Some(9876));
        assert_eq!(status.checks[0].workflow_run_id, Some(6543));
        assert_eq!(
            status.checks[0].completed_at_unix_seconds,
            Some(1_783_936_860)
        );
    }

    #[test]
    fn duplicate_same_name_publishers_fail_closed() {
        let error = parse_gh_status(
            "github.com/example/d2b",
            42,
            &graphql(serde_json::json!([
                check("check", "github-actions", "COMPLETED", "SUCCESS"),
                check("check", "other-app", "COMPLETED", "SUCCESS")
            ])),
        )
        .expect_err("duplicates");
        assert!(error.to_string().contains("duplicate same-name"));
    }

    #[test]
    fn process_runner_caps_stdout_stderr_and_timeout() {
        let runner = ProcessCommandOutput;
        let stdout = runner
            .output_with_limits(
                "sh",
                &["-c".to_owned(), "printf 12345".to_owned()],
                None,
                CommandLimits {
                    stdout_bytes: 4,
                    stderr_bytes: 4,
                    timeout: Duration::from_secs(1),
                },
            )
            .expect_err("stdout overflow");
        assert!(stdout.to_string().contains("stdout exceeds"));

        let stderr = runner
            .output_with_limits(
                "sh",
                &["-c".to_owned(), "printf 12345 >&2".to_owned()],
                None,
                CommandLimits {
                    stdout_bytes: 4,
                    stderr_bytes: 4,
                    timeout: Duration::from_secs(1),
                },
            )
            .expect_err("stderr overflow");
        assert!(stderr.to_string().contains("stderr exceeds"));

        let started = Instant::now();
        let timeout = runner
            .output_with_limits(
                "sh",
                &["-c".to_owned(), "sleep 5".to_owned()],
                None,
                CommandLimits {
                    stdout_bytes: 4,
                    stderr_bytes: 4,
                    timeout: Duration::from_millis(20),
                },
            )
            .expect_err("timeout");
        assert!(timeout.to_string().contains("timeout"));
        assert!(started.elapsed() < Duration::from_secs(1));

        let started = Instant::now();
        let descendant = runner
            .output_with_limits(
                "sh",
                &[
                    "-c".to_owned(),
                    "(sleep 30) & printf parent-complete".to_owned(),
                ],
                None,
                CommandLimits {
                    stdout_bytes: 64,
                    stderr_bytes: 64,
                    timeout: Duration::from_millis(50),
                },
            )
            .expect_err("descendant retained output pipe");
        assert!(descendant.to_string().contains("timeout"));
        assert!(started.elapsed() < Duration::from_secs(2));

        let started = Instant::now();
        let escaped_pipe = runner
            .output_with_limits(
                "sh",
                &[
                    "-c".to_owned(),
                    "setsid sh -c 'exec sleep 0.25' & printf parent-complete".to_owned(),
                ],
                None,
                CommandLimits {
                    stdout_bytes: 64,
                    stderr_bytes: 64,
                    timeout: Duration::from_millis(20),
                },
            )
            .expect_err("detached descendant retained output pipe");
        assert!(escaped_pipe.to_string().contains("timeout"));
        assert!(started.elapsed() < Duration::from_millis(200));
    }

    #[test]
    fn object_format_validation_is_exact() {
        assert!(
            super::super::model::validate_hash_for_format(
                &"a".repeat(40),
                GitObjectFormat::Sha1,
                "oid"
            )
            .is_ok()
        );
        assert!(
            super::super::model::validate_hash_for_format(
                &"a".repeat(40),
                GitObjectFormat::Sha256,
                "oid"
            )
            .is_err()
        );
    }

    #[test]
    fn gh_stack_adapter_uses_official_machine_readable_surface() {
        let graph = serde_json::json!({
            "trunk": "main",
            "prefix": "",
            "currentBranch": "feature",
            "branches": [{
                "name": "feature",
                "head": "b".repeat(40),
                "base": "a".repeat(40),
                "isCurrent": true,
                "isMerged": false,
                "isQueued": false,
                "needsRebase": false,
                "pr": {"number": 42, "url": "", "state": "OPEN"}
            }]
        });
        let command = FakeCommand::new(vec![successful_output(
            serde_json::to_vec(&graph).expect("graph JSON"),
        )]);
        GhStackSource::new(&command)
            .graph("github.com/example/d2b", Path::new("/checkout"))
            .expect("graph");
        let calls = command.calls.borrow();
        assert_eq!(calls[0].0, "gh");
        assert_eq!(calls[0].1, ["stack", "view", "--json"]);
        assert_eq!(calls[0].2.as_deref(), Some(Path::new("/checkout")));
    }

    #[test]
    fn github_status_query_uses_real_check_suite_schema() {
        let command = FakeCommand::new(vec![successful_output(graphql(serde_json::json!([
            check("check", "github-actions", "COMPLETED", "SUCCESS")
        ])))]);
        GhStatusSource::new(&command)
            .status("github.com/example/d2b", 42)
            .expect("status");
        let calls = command.calls.borrow();
        let query = calls[0].1.join(" ");
        for field in [
            "baseRefOid",
            "headRefOid",
            "status",
            "conclusion",
            "workflow",
            "workflowRun",
            "checkSuite",
            "databaseId",
            "startedAt",
            "completedAt",
            "createdAt",
            "updatedAt",
            "isInMergeQueue",
            "isMergeQueueEnabled",
            "mergeQueueEntry",
            "mergeCommit",
            "baseCommit",
            "headCommit",
            "tree",
            "app",
            "commit",
        ] {
            assert!(query.contains(field), "query omitted {field}");
        }
        assert!(!query.contains("CheckRun{name status conclusion workflow"));
        assert!(!query.contains("conclusion workflow{name"));
        assert!(!query.contains("databaseId} app{"));
    }

    #[test]
    fn merge_queue_authority_is_complete_or_rejected() {
        let mut queued: serde_json::Value =
            serde_json::from_slice(&graphql(serde_json::json!([check(
                "check",
                "github-actions",
                "COMPLETED",
                "SUCCESS"
            )])))
            .expect("fixture JSON");
        queued
            .pointer_mut("/data/repository/pullRequest")
            .and_then(serde_json::Value::as_object_mut)
            .expect("pull request")
            .insert("isInMergeQueue".to_owned(), serde_json::json!(true));
        let missing = serde_json::to_vec(&queued).expect("queued JSON");
        let error = parse_gh_status("github.com/example/d2b", 42, &missing)
            .expect_err("missing queue authority");
        assert!(
            error
                .to_string()
                .contains("without exact merge-queue authority")
        );

        queued
            .pointer_mut("/data/repository/pullRequest")
            .and_then(serde_json::Value::as_object_mut)
            .expect("pull request")
            .insert(
                "mergeQueueEntry".to_owned(),
                serde_json::json!({
                    "id": "MQE_example",
                    "state": "AWAITING_CHECKS",
                    "baseCommit": {"oid": "a".repeat(40)},
                    "headCommit": {"oid": "c".repeat(40)}
                }),
            );
        let complete = parse_gh_status(
            "github.com/example/d2b",
            42,
            &serde_json::to_vec(&queued).expect("queued JSON"),
        )
        .expect("complete queue authority");
        assert!(complete.is_in_merge_queue);
        assert_eq!(
            complete.merge_queue_entry.expect("queue entry").base_oid,
            "a".repeat(40)
        );
    }

    #[test]
    fn merged_pr_fixture_carries_exact_merge_commit_and_tree() {
        let mut merged: serde_json::Value =
            serde_json::from_slice(&graphql(serde_json::json!([check(
                "check",
                "github-actions",
                "COMPLETED",
                "SUCCESS"
            )])))
            .expect("fixture JSON");
        let pull_request = merged
            .pointer_mut("/data/repository/pullRequest")
            .and_then(serde_json::Value::as_object_mut)
            .expect("pull request");
        pull_request.insert("state".to_owned(), serde_json::json!("MERGED"));
        pull_request.insert(
            "mergeCommit".to_owned(),
            serde_json::json!({
                "oid": "c".repeat(40),
                "tree": {"oid": "d".repeat(40)}
            }),
        );
        let status = parse_gh_status(
            "github.com/example/d2b",
            42,
            &serde_json::to_vec(&merged).expect("merged JSON"),
        )
        .expect("merged status");
        assert_eq!(status.state, PullRequestState::Merged);
        assert_eq!(status.merge_commit_oid, Some("c".repeat(40)));
        assert_eq!(status.merge_commit_tree_oid, Some("d".repeat(40)));
    }

    #[test]
    fn gh_merge_adapter_refuses_head_only_authority() {
        let command = FakeCommand::new(vec![]);
        let error = GhMergeSource::new(&command)
            .merge_with_expected_base_and_head(
                "github.com/example/d2b",
                42,
                &"a".repeat(40),
                &"b".repeat(40),
            )
            .expect_err("no base compare-and-swap");
        assert!(error.to_string().contains("base+head compare-and-swap"));
        assert!(command.calls.borrow().is_empty());
    }

    #[test]
    fn gh_stack_private_preview_probe_is_read_only_and_version_pinned() {
        let command = FakeCommand::new([
            successful_output(b"gh stack version 0.0.7\n".to_vec()),
            successful_output(b"[]\n".to_vec()),
        ]);

        check_gh_stack_private_preview(&command, "example/d2b").expect("available");

        assert_eq!(
            *command.calls.borrow(),
            vec![
                ("gh-stack".to_owned(), vec!["--version".to_owned()], None),
                (
                    "gh".to_owned(),
                    vec![
                        "api".to_owned(),
                        "--method".to_owned(),
                        "GET".to_owned(),
                        "repos/example/d2b/cli_internal/pulls/stacks".to_owned(),
                    ],
                    None,
                ),
            ]
        );
    }

    #[test]
    fn gh_stack_private_preview_failure_has_no_mutating_fallback() {
        let command = FakeCommand::new([
            successful_output(b"gh stack version 0.0.7\n".to_vec()),
            CommandOutput {
                success: false,
                exit_code: Some(1),
                stdout: Vec::new(),
                stderr: Vec::new(),
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
        let wrong_version =
            FakeCommand::new([successful_output(b"gh stack version 0.0.6\n".to_vec())]);
        let error = check_gh_stack_private_preview(&wrong_version, "example/d2b")
            .expect_err("wrong version");
        assert!(
            error
                .to_string()
                .contains("expected official gh-stack 0.0.7")
        );

        let malformed = FakeCommand::new([
            successful_output(b"gh stack version 0.0.7\n".to_vec()),
            successful_output(br#"{"unexpected":true}"#.to_vec()),
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
                successful_output(b"gh stack version 0.0.7\n".to_vec()),
                successful_output(payload.to_vec()),
            ]);
            check_gh_stack_private_preview(&malformed_array, "example/d2b")
                .expect_err("malformed stack record");
        }

        let valid = FakeCommand::new([
            successful_output(b"gh stack version 0.0.7\n".to_vec()),
            successful_output(br#"[{"id":7,"pull_requests":[101,102]}]"#.to_vec()),
        ]);
        check_gh_stack_private_preview(&valid, "example/d2b").expect("typed stack record");
    }
}
