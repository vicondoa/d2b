use std::{
    collections::BTreeSet,
    fs,
    io::Read,
    os::unix::process::CommandExt,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
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

    fn output(&self, program: &str, args: &[String], cwd: Option<&Path>) -> Result<CommandOutput> {
        self.output_with_limits(program, args, cwd, CommandLimits::default())
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
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
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
        let stdout_reader = spawn_capped_reader(stdout, limits.stdout_bytes, &stdout_overflow);
        let stderr_reader = spawn_capped_reader(stderr, limits.stderr_bytes, &stderr_overflow);

        let started = Instant::now();
        let mut timed_out = false;
        let status = loop {
            if stdout_overflow.load(Ordering::Acquire)
                || stderr_overflow.load(Ordering::Acquire)
                || started.elapsed() >= limits.timeout
            {
                timed_out = started.elapsed() >= limits.timeout;
                if let Some(process_group) = process_group {
                    let _ = rustix::process::kill_process_group(
                        process_group,
                        rustix::process::Signal::Kill,
                    );
                }
                let _ = child.kill();
                break child.wait().map_err(|error| {
                    DeliveryError::new(format!("could not reap {program}: {error}"))
                })?;
            }
            if let Some(status) = child
                .try_wait()
                .map_err(|error| DeliveryError::new(format!("could not poll {program}: {error}")))?
            {
                break status;
            }
            thread::sleep(Duration::from_millis(5));
        };

        let stdout = stdout_reader
            .join()
            .map_err(|_| DeliveryError::new("stdout reader panicked"))??;
        let stderr = stderr_reader
            .join()
            .map_err(|_| DeliveryError::new("stderr reader panicked"))??;
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

fn spawn_capped_reader<R: Read + Send + 'static>(
    mut reader: R,
    limit: usize,
    overflow: &Arc<AtomicBool>,
) -> thread::JoinHandle<Result<Vec<u8>>> {
    let overflow = Arc::clone(overflow);
    thread::spawn(move || {
        let mut output = Vec::with_capacity(limit.min(64 * 1024));
        let mut buffer = [0_u8; 8192];
        loop {
            let read = reader.read(&mut buffer).map_err(|error| {
                DeliveryError::new(format!("cannot read child output: {error}"))
            })?;
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
    })
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
    pub status: String,
    pub conclusion: String,
    pub state: ObservedCheckState,
    pub commit_oid: String,
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

const PR_STATUS_QUERY: &str = r#"query($owner:String!,$name:String!,$number:Int!){
  repository(owner:$owner,name:$name){
    nameWithOwner
    pullRequest(number:$number){
      number state mergeStateStatus baseRefName baseRefOid headRefName headRefOid
      headRepository{nameWithOwner}
      commits(last:1){
        nodes{commit{oid statusCheckRollup{contexts(first:100){
          nodes{
            __typename
            ... on CheckRun{name status conclusion workflow{name databaseId} app{slug databaseId} commit{oid}}
            ... on StatusContext{context state creator{login} commit{oid}}
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
    commits: GraphQlCommits,
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
        checks,
    })
}

fn parse_check(value: &serde_json::Value, outer_commit: &str) -> Result<ObservedCheck> {
    let object = value
        .as_object()
        .ok_or_else(|| DeliveryError::new("GitHub check entry is not an object"))?;
    let kind = required_string(object, "__typename")?;
    let commit_oid = object
        .get("commit")
        .and_then(serde_json::Value::as_object)
        .and_then(|commit| commit.get("oid"))
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| DeliveryError::new("GitHub check has no commit association"))?;
    if commit_oid != outer_commit {
        return Err(DeliveryError::new(
            "GitHub check is associated with a different commit",
        ));
    }
    let check = match kind {
        "CheckRun" => {
            let name = required_string(object, "name")?.to_owned();
            let status = required_string(object, "status")?.to_owned();
            let conclusion = object
                .get("conclusion")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("NONE")
                .to_owned();
            let (workflow, workflow_id) = optional_workflow(object)?;
            let app_slug = nested_string(object, "app", "slug")?.to_owned();
            let app_id = nested_u64(object, "app", "databaseId")?;
            let state = check_run_state(&status, &conclusion)?;
            ObservedCheck {
                name,
                publisher: CheckPublisher {
                    kind: CheckPublisherKind::CheckRun,
                    app_slug,
                    app_id,
                    workflow,
                    workflow_id,
                },
                status,
                conclusion,
                state,
                commit_oid: commit_oid.to_owned(),
            }
        }
        "StatusContext" => {
            let name = required_string(object, "context")?.to_owned();
            let status = required_string(object, "state")?.to_owned();
            let app_slug = nested_string(object, "creator", "login")?.to_owned();
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
                status: status.clone(),
                conclusion: status,
                state,
                commit_oid: commit_oid.to_owned(),
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

fn optional_workflow(object: &serde_json::Map<String, serde_json::Value>) -> Result<(String, u64)> {
    match object.get("workflow") {
        None | Some(serde_json::Value::Null) => Ok(("none".to_owned(), 0)),
        Some(serde_json::Value::Object(workflow)) => {
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
            Ok((name.to_owned(), id))
        }
        Some(_) => Err(DeliveryError::new(
            "GitHub check workflow is not an object or null",
        )),
    }
}

pub trait PullRequestMerger {
    fn merge_with_expected_head(
        &self,
        repository: &str,
        pr: u64,
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
    fn merge_with_expected_head(
        &self,
        repository: &str,
        pr: u64,
        expected_head: &str,
    ) -> Result<()> {
        validate_repository_id(repository)?;
        validate_hash(expected_head, "expected merge head")?;
        let output = self.command.output(
            "gh",
            &[
                "pr".to_owned(),
                "merge".to_owned(),
                pr.to_string(),
                "--repo".to_owned(),
                github_repo_arg(repository)?,
                "--merge".to_owned(),
                "--match-head-commit".to_owned(),
                expected_head.to_owned(),
            ],
            None,
        )?;
        if !output.success {
            return Err(DeliveryError::new(format!(
                "atomic GitHub merge failed for {repository}#{pr}"
            )));
        }
        Ok(())
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

fn github_repo_arg(repository: &str) -> Result<String> {
    let (owner, name) = github_owner_name(repository)?;
    Ok(format!("{owner}/{name}"))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};

    type CommandCall = (String, Vec<String>, Option<PathBuf>);

    struct FakeCommand {
        calls: RefCell<Vec<CommandCall>>,
        outputs: RefCell<VecDeque<CommandOutput>>,
    }

    impl FakeCommand {
        fn new(outputs: Vec<CommandOutput>) -> Self {
            Self {
                calls: RefCell::new(Vec::new()),
                outputs: RefCell::new(outputs.into()),
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
            "name": name,
            "status": status,
            "conclusion": conclusion,
            "workflow": {"name": "Layer 1", "databaseId": 321},
            "app": {"slug": app, "databaseId": 15368},
            "commit": {"oid": "b".repeat(40)}
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
    fn github_status_and_merge_adapters_bind_exact_oids_and_expected_head() {
        let command = FakeCommand::new(vec![
            successful_output(graphql(serde_json::json!([check(
                "check",
                "github-actions",
                "COMPLETED",
                "SUCCESS"
            )]))),
            successful_output(vec![]),
        ]);
        GhStatusSource::new(&command)
            .status("github.com/example/d2b", 42)
            .expect("status");
        GhMergeSource::new(&command)
            .merge_with_expected_head("github.com/example/d2b", 42, &"b".repeat(40))
            .expect("merge");
        let calls = command.calls.borrow();
        let query = calls[0].1.join(" ");
        for field in [
            "baseRefOid",
            "headRefOid",
            "status",
            "conclusion",
            "workflow",
            "app",
            "commit",
        ] {
            assert!(query.contains(field), "query omitted {field}");
        }
        assert!(
            calls[1]
                .1
                .windows(2)
                .any(|pair| { pair[0] == "--match-head-commit" && pair[1] == "b".repeat(40) })
        );
    }
}
