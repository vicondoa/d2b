use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

const AUTHORITY_REPOSITORY: &str = "vicondoa/d2b";
const WAVE_BRANCH: &str = "adr0045-w6-edge";

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root")
        .to_path_buf()
}

fn sanitized_command(program: &str, root: &Path) -> Command {
    let mut command = Command::new(program);
    command.current_dir(root);
    for (key, _) in env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    command
        .env_remove("GH_HOST")
        .env_remove("GH_REPO")
        .env("GIT_NO_REPLACE_OBJECTS", "1");
    command
}

fn command_output(mut command: Command, args: &[&str], label: &str) -> String {
    let output = command.args(args).output().expect(label);
    assert!(
        output.status.success(),
        "{label}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("command output is UTF-8")
        .trim()
        .to_owned()
}

fn git_output(root: &Path, args: &[&str]) -> String {
    let mut command = sanitized_command("git", root);
    command.args(["--no-optional-locks", "-c", "diff.ignoreSubmodules=none"]);
    command_output(command, args, "git command failed")
}

fn optional_git_output(root: &Path, args: &[&str]) -> Option<String> {
    let mut command = sanitized_command("git", root);
    let output = command
        .args(["--no-optional-locks", "-c", "diff.ignoreSubmodules=none"])
        .args(args)
        .output()
        .expect("execute optional git query");
    if !output.status.success() {
        assert_eq!(output.status.code(), Some(1), "optional git query failed");
        assert!(
            output.stderr.is_empty(),
            "optional git query emitted an error"
        );
        return None;
    }
    Some({
        String::from_utf8(output.stdout)
            .expect("git output is UTF-8")
            .trim()
            .to_owned()
    })
}

fn reject_graph_metadata(root: &Path) {
    assert!(
        git_output(
            root,
            &["for-each-ref", "--format=%(refname)", "refs/replace"]
        )
        .is_empty(),
        "repository contains forbidden replacement refs"
    );
    assert_eq!(
        git_output(root, &["rev-parse", "--is-shallow-repository"]),
        "false",
        "shallow history cannot establish a wave segment"
    );
    let common_dir = PathBuf::from(git_output(
        root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ));
    for path in [common_dir.join("info/grafts"), common_dir.join("shallow")] {
        assert!(
            !path.exists(),
            "repository contains forbidden graph-rewrite metadata"
        );
    }
}

fn manifest_segment(root: &Path) -> (String, u64) {
    let manifest = fs::read_to_string(root.join("delivery/manifests/w6.json"))
        .expect("read W6 delivery authority");
    let mut pending_branch = None;
    let mut nodes = Vec::new();
    for line in manifest.lines().map(str::trim) {
        if let Some(branch) = line
            .strip_prefix(r#""branch": ""#)
            .and_then(|value| value.strip_suffix("\","))
        {
            pending_branch = Some(branch.to_owned());
        } else if let Some(number) = line
            .strip_prefix(r#""pr_number": "#)
            .and_then(|value| value.strip_suffix(','))
        {
            nodes.push((
                pending_branch.take().expect("manifest branch before PR"),
                number.parse::<u64>().expect("manifest PR number"),
            ));
        }
    }
    let index = nodes
        .iter()
        .position(|(branch, _)| branch == WAVE_BRANCH)
        .expect("manifest W6 node");
    assert!(index > 0, "W6 node has no segment base");
    (nodes[index - 1].0.clone(), nodes[index].1)
}

fn is_commit_oid(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

#[test]
#[ignore = "explicit live edge ownership preflight"]
fn verify_historical_edge_segment_authority() {
    let root = repository_root();
    reject_graph_metadata(&root);
    let (manifest_parent, manifest_pr) = manifest_segment(&root);

    let parent_key = format!("git-town-branch.{WAVE_BRANCH}.parent");
    if let Some(configured_parent) = optional_git_output(&root, &["config", "--get", &parent_key]) {
        assert_eq!(configured_parent, manifest_parent);
    }

    let gh = sanitized_command("gh", &root);
    let pr_arg = manifest_pr.to_string();
    let status = command_output(
        gh,
        &[
            "pr",
            "view",
            &pr_arg,
            "--repo",
            AUTHORITY_REPOSITORY,
            "--json",
            "state,baseRefName,baseRefOid,headRefName,headRefOid,isCrossRepository",
            "--jq",
            "[.state,.baseRefName,.baseRefOid,.headRefName,.headRefOid,.isCrossRepository] | @tsv",
        ],
        "read W6 pull request authority",
    );
    let fields = status.split('\t').collect::<Vec<_>>();
    assert_eq!(fields.len(), 6, "invalid GitHub PR authority row");
    assert!(matches!(fields[0], "OPEN" | "MERGED"));
    assert_eq!(fields[1], manifest_parent);
    assert_eq!(fields[3], WAVE_BRANCH);
    assert_eq!(fields[5], "false");
    let base_oid = fields[2];
    let head_oid = fields[4];
    assert!(is_commit_oid(base_oid));
    assert!(is_commit_oid(head_oid));

    for oid in [base_oid, head_oid] {
        let object = format!("{oid}^{{commit}}");
        assert_eq!(git_output(&root, &["cat-file", "-t", &object]), "commit");
    }
    let mut command = sanitized_command("git", &root);
    let ancestry = command
        .args(["--no-optional-locks", "merge-base", "--is-ancestor"])
        .arg(base_oid)
        .arg(head_oid)
        .status()
        .expect("verify W6 segment ancestry");
    assert!(ancestry.success(), "W6 segment base is not an ancestor");

    let changed = git_output(
        &root,
        &[
            "diff",
            "--name-only",
            "--no-renames",
            "--ignore-submodules=none",
            base_oid,
            head_oid,
            "--",
        ],
    );
    assert!(!changed.is_empty(), "historical W6 segment is empty");
}
