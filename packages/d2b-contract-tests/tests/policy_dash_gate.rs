//! The non-ASCII dash ban and the tier0 gate that enforces it.
//!
//! `AGENTS.md` permits only the plain ASCII hyphen to spell a dash, and
//! `tests/tools/tier0-first-pass.sh` is the gate. A ban whose gate silently
//! stops matching is worse than no ban, so this lint drives the gate's scan
//! over a fixture tree and requires it to fail on every banned codepoint and
//! pass on a clean one. It also pins the structural properties the scan depends
//! on: the gate is wired into `make check-tier0`, it runs repository-wide in
//! its main body, and it matches on codepoints rather than on literal
//! characters that would make it flag its own source. Only the exact
//! agent-asset paths receive admission.
//!
//! Every banned character in this file is written as a `\u{...}` escape, never
//! as the character.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use d2b_contract_tests::{read_repo_file, repo_root};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const GATE: &str = "tests/tools/tier0-first-pass.sh";

/// The banned class, paired with the codepoint label the gate documents. Only
/// U+2011, U+2013, U+2014 and U+2212 were ever present in this repository; the
/// rest are banned pre-emptively so a future paste of any of them fails the
/// same way, and each still has to be proven rejected.
const BANNED: &[(char, &str)] = &[
    ('\u{2010}', "U+2010"),
    ('\u{2011}', "U+2011"),
    ('\u{2012}', "U+2012"),
    ('\u{2013}', "U+2013"),
    ('\u{2014}', "U+2014"),
    ('\u{2015}', "U+2015"),
    ('\u{2212}', "U+2212"),
    ('\u{FE58}', "U+FE58"),
    ('\u{FF0D}', "U+FF0D"),
];

const APPROVED_SKILLS: &[(&str, &str)] = &[
    (
        "third_party/agent-skills/ponytail/v4.9.0/skills",
        "ponytail",
    ),
    (
        "third_party/agent-skills/ponytail/v4.9.0/skills",
        "ponytail-audit",
    ),
    (
        "third_party/agent-skills/ponytail/v4.9.0/skills",
        "ponytail-debt",
    ),
    (
        "third_party/agent-skills/ponytail/v4.9.0/skills",
        "ponytail-gain",
    ),
    (
        "third_party/agent-skills/ponytail/v4.9.0/skills",
        "ponytail-help",
    ),
    (
        "third_party/agent-skills/ponytail/v4.9.0/skills",
        "ponytail-review",
    ),
    ("third_party/agent-skills/caveman/v2.0.0/skills", "caveman"),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
        "ce-babysit-pr",
    ),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
        "ce-brainstorm",
    ),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
        "ce-code-review",
    ),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
        "ce-commit-push-pr",
    ),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
        "ce-debug",
    ),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
        "ce-doc-review",
    ),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
        "ce-plan",
    ),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
        "ce-resolve-pr-feedback",
    ),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
        "ce-simplify-code",
    ),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
        "ce-work",
    ),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
        "ce-worktree",
    ),
];
const APPROVED_NOTICES: &[&str] = &[
    "third_party/agent-skills/ponytail/v4.9.0/LICENSE",
    "third_party/agent-skills/caveman/v2.0.0/LICENSE",
    "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/LICENSE",
];
const APPROVED_UPSTREAMS: &[(&str, &str, &str, &str, &str)] = &[
    (
        "third_party/agent-skills/ponytail/v4.9.0",
        "https://github.com/DietrichGebert/ponytail",
        "v4.9.0",
        "0a4dd63ad4541f4f655c4108a295916f3c1d8fda",
        "MIT",
    ),
    (
        "third_party/agent-skills/caveman/v2.0.0",
        "https://github.com/JuliusBrussee/caveman",
        "v2.0.0",
        "2c67abb9833689b48c7abba88afaa77c39a18657",
        "MIT",
    ),
    (
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4",
        "https://github.com/EveryInc/compound-engineering-plugin",
        "compound-engineering-v3.21.4",
        "0a2957852e2034d04eb01120fd7da6ed5307dc56",
        "MIT",
    ),
];

#[derive(Debug, Deserialize)]
struct UpstreamMetadata {
    files: BTreeMap<String, String>,
    imported_paths: Vec<String>,
    excluded_surfaces: Vec<String>,
    license: String,
    upstream_commit: String,
    upstream_repository: String,
    upstream_tag: String,
    vendor_date: String,
}

fn gate_path() -> PathBuf {
    repo_root().join(GATE)
}

fn scrubber_path() -> PathBuf {
    repo_root().join("tests/tools/scrub-shell-environment")
}

/// Run the gate's scan mode over `root` and return `(success, combined output)`.
fn scan(root: &Path) -> (bool, String) {
    scan_with_path(root, None)
}

fn scan_with_path(root: &Path, extra_path: Option<&Path>) -> (bool, String) {
    let mut command = Command::new(scrubber_path());
    if let Some(extra_path) = extra_path {
        let mut paths = vec![extra_path.to_path_buf()];
        paths.extend(std::env::split_paths(
            &std::env::var_os("PATH").expect("PATH must be set for policy tests"),
        ));
        command.env(
            "PATH",
            std::env::join_paths(paths).expect("join policy test PATH"),
        );
    }
    let output = command
        .args(["-c", "exec bash \"$@\"", "policy-dash-gate"])
        .arg(gate_path())
        .arg("--scan-dashes")
        .arg(root)
        .output()
        .expect("tier0 gate must be runnable");
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    (output.status.success(), combined)
}

fn fixture_tree(name: &str, body: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("dash-gate")
        .join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("nested")).expect("create fixture tree");
    fs::write(root.join("clean.md"), "A spaced hyphen - like this.\n").expect("write clean file");
    fs::write(root.join("nested/sample.md"), body).expect("write sample file");
    root
}

fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = fs::metadata(path)
        .expect("read executable metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).expect("make fixture executable");
}

fn git_fixture_tree(name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("dash-gate")
        .join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).expect("create git fixture tree");
    let status = Command::new("git")
        .arg("-C")
        .arg(&root)
        .args(["-c", "init.defaultBranch=main", "init", "--quiet"])
        .status()
        .expect("initialize git fixture");
    assert!(status.success(), "git fixture initialization must succeed");
    root
}

fn write_dash_file(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create dash fixture parent");
    }
    fs::write(path, "contains \u{2014}\n").expect("write dash fixture");
}

fn canonical_skill_path(root: &Path, skill_root: &str, skill: &str) -> PathBuf {
    root.join(skill_root).join(skill)
}

fn create_canonical_skills(root: &Path) {
    for (skill_root, skill) in APPROVED_SKILLS {
        let directory = canonical_skill_path(root, skill_root, skill);
        fs::create_dir_all(&directory).expect("create canonical skill directory");
        write_dash_file(&directory.join("SKILL.md"));
    }
}

fn create_approved_adapters(root: &Path) {
    fs::create_dir_all(root.join(".agents/skills")).expect("create agents adapter");
    fs::create_dir_all(root.join(".claude/skills")).expect("create claude adapter");

    for (skill_root, skill) in APPROVED_SKILLS {
        let canonical = format!("{skill_root}/{skill}");
        let agents_link = root.join(".agents/skills").join(skill);
        std::os::unix::fs::symlink(format!("../../{canonical}"), &agents_link)
            .expect("create agents skill symlink");

        let claude_directory = root.join(".claude/skills").join(skill);
        fs::create_dir_all(&claude_directory).expect("create claude fallback skill directory");
        std::os::unix::fs::symlink(
            format!("../../../{canonical}/SKILL.md"),
            claude_directory.join("SKILL.md"),
        )
        .expect("create claude component symlink");
    }
}

fn approved_version_roots() -> BTreeSet<&'static str> {
    APPROVED_SKILLS
        .iter()
        .map(|&(skill_root, _)| {
            skill_root
                .strip_suffix("/skills")
                .expect("approved skill root must end in /skills")
        })
        .collect()
}

fn collect_regular_files(root: &Path) -> BTreeSet<String> {
    fn visit(root: &Path, current: &Path, files: &mut BTreeSet<String>) {
        let entries = fs::read_dir(current)
            .unwrap_or_else(|error| panic!("cannot enumerate {}: {error}", current.display()));
        for entry in entries {
            let entry = entry.unwrap_or_else(|error| {
                panic!(
                    "cannot read directory entry in {}: {error}",
                    current.display()
                )
            });
            let path = entry.path();
            let file_type = fs::symlink_metadata(&path)
                .unwrap_or_else(|error| panic!("cannot stat {}: {error}", path.display()));
            if file_type.is_dir() {
                visit(root, &path, files);
            } else if file_type.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .unwrap_or_else(|error| {
                        panic!(
                            "cannot relativize {} against {}: {error}",
                            path.display(),
                            root.display()
                        )
                    })
                    .to_str()
                    .unwrap_or_else(|| panic!("non-UTF-8 path under {}", root.display()));
                if relative != "UPSTREAM.json" {
                    files.insert(relative.to_owned());
                }
            } else {
                panic!(
                    "vendored version roots may contain only directories and regular files: {}",
                    path.display()
                );
            }
        }
    }

    let mut files = BTreeSet::new();
    visit(root, root, &mut files);
    files
}

fn sha256_file(path: &Path) -> String {
    let bytes =
        fs::read(path).unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    format!("{:x}", Sha256::digest(bytes))
}

fn assert_canonical_skill_files(root: &Path) {
    for &(skill_root, skill) in APPROVED_SKILLS {
        let directory = canonical_skill_path(root, skill_root, skill);
        let directory_type = fs::symlink_metadata(&directory).unwrap_or_else(|error| {
            panic!(
                "missing canonical skill directory {}: {error}",
                directory.display()
            )
        });
        assert!(
            directory_type.is_dir(),
            "canonical skill path must be a directory: {}",
            directory.display()
        );

        let skill_file = directory.join("SKILL.md");
        let skill_file_type = fs::symlink_metadata(&skill_file).unwrap_or_else(|error| {
            panic!(
                "missing canonical skill file {}: {error}",
                skill_file.display()
            )
        });
        assert!(
            skill_file_type.is_file(),
            "canonical skill file must be a regular file: {}",
            skill_file.display()
        );
    }
}

fn assert_upstream_metadata(root: &Path, version_root: &str) {
    let (_, expected_repository, expected_tag, expected_commit, expected_license) =
        APPROVED_UPSTREAMS
            .iter()
            .find(|&&(candidate, _, _, _, _)| candidate == version_root)
            .unwrap_or_else(|| panic!("no pinned metadata expectation for {version_root}"));
    let metadata_path = root.join(version_root).join("UPSTREAM.json");
    let metadata_body = fs::read_to_string(&metadata_path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", metadata_path.display()));
    let metadata: UpstreamMetadata = serde_json::from_str(&metadata_body).unwrap_or_else(|error| {
        panic!(
            "invalid upstream metadata {}: {error}",
            metadata_path.display()
        )
    });

    assert_eq!(
        metadata.upstream_repository,
        *expected_repository,
        "{}: upstream_repository drifted from the pinned value",
        metadata_path.display()
    );
    assert_eq!(
        metadata.upstream_tag,
        *expected_tag,
        "{}: upstream_tag drifted from the pinned value",
        metadata_path.display()
    );
    assert_eq!(
        metadata.upstream_commit,
        *expected_commit,
        "{}: upstream_commit drifted from the pinned value",
        metadata_path.display()
    );
    assert_eq!(
        metadata.license,
        *expected_license,
        "{}: license drifted from the pinned value",
        metadata_path.display()
    );
    assert_eq!(
        metadata.vendor_date,
        "2026-08-14",
        "{}: vendor_date drifted from the pinned value",
        metadata_path.display()
    );
    let expected_skill_root = format!("{version_root}/skills");
    let expected_imports: BTreeSet<_> = std::iter::once("LICENSE".to_owned())
        .chain(
            APPROVED_SKILLS
                .iter()
                .filter(|&&(skill_root, _)| skill_root == expected_skill_root)
                .map(|&(_, skill)| format!("skills/{skill}")),
        )
        .collect();
    let actual_imports: BTreeSet<_> = metadata.imported_paths.iter().cloned().collect();
    assert_eq!(
        actual_imports.len(),
        metadata.imported_paths.len(),
        "{}: imported_paths must not contain duplicates",
        metadata_path.display()
    );
    assert_eq!(
        actual_imports,
        expected_imports,
        "{}: imported_paths must exactly match the approved subset",
        metadata_path.display()
    );
    assert!(
        !metadata.excluded_surfaces.is_empty(),
        "{}: excluded_surfaces must be present and non-empty",
        metadata_path.display()
    );
    assert!(
        metadata
            .excluded_surfaces
            .iter()
            .all(|surface| !surface.is_empty()),
        "{}: excluded_surfaces must not contain empty entries",
        metadata_path.display()
    );

    let version_path = root.join(version_root);
    let actual_files = collect_regular_files(&version_path);
    let listed_files: BTreeSet<_> = metadata.files.keys().cloned().collect();
    if let Some(missing) = actual_files.difference(&listed_files).next() {
        panic!(
            "{}: files map is missing regular file {}",
            metadata_path.display(),
            version_path.join(missing).display()
        );
    }
    if let Some(extra) = listed_files.difference(&actual_files).next() {
        panic!(
            "{}: files map contains extra file entry {}",
            metadata_path.display(),
            version_path.join(extra).display()
        );
    }

    for (relative, expected_hash) in &metadata.files {
        let path = version_path.join(relative);
        let actual_hash = sha256_file(&path);
        assert_eq!(
            actual_hash,
            expected_hash.to_ascii_lowercase(),
            "{}: hash mismatch for {}",
            metadata_path.display(),
            path.display()
        );
        assert!(
            expected_hash
                .chars()
                .all(|character| character.is_ascii_hexdigit() && !character.is_ascii_uppercase()),
            "{}: hash for {} must be lowercase hexadecimal",
            metadata_path.display(),
            path.display()
        );
    }
}

fn canonical_skill_for_name(root: &Path, skill: &str) -> PathBuf {
    APPROVED_SKILLS
        .iter()
        .find_map(|&(skill_root, candidate)| {
            (candidate == skill).then(|| canonical_skill_path(root, skill_root, skill))
        })
        .unwrap_or_else(|| panic!("no canonical skill path for adapter entry {skill}"))
}

fn assert_adapter_topology(root: &Path, adapter_root: &str) {
    let adapter_path = root.join(adapter_root);
    let adapter_type = fs::symlink_metadata(&adapter_path).unwrap_or_else(|error| {
        panic!(
            "missing adapter directory {}: {error}",
            adapter_path.display()
        )
    });
    assert!(
        adapter_type.is_dir(),
        "adapter root must be a directory: {}",
        adapter_path.display()
    );

    let expected_names: BTreeSet<_> = APPROVED_SKILLS
        .iter()
        .map(|&(_, skill)| skill.to_owned())
        .collect();
    let mut actual_names = BTreeSet::new();
    let entries = fs::read_dir(&adapter_path)
        .unwrap_or_else(|error| panic!("cannot enumerate {}: {error}", adapter_path.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|error| {
            panic!(
                "cannot read directory entry in {}: {error}",
                adapter_path.display()
            )
        });
        let entry_path = entry.path();
        let name = entry
            .file_name()
            .into_string()
            .unwrap_or_else(|_| panic!("non-UTF-8 adapter entry under {}", adapter_path.display()));
        assert!(
            expected_names.contains(&name),
            "unexpected adapter entry: {}",
            entry_path.display()
        );
        assert!(
            actual_names.insert(name.clone()),
            "duplicate adapter entry: {}",
            entry_path.display()
        );

        let entry_type = fs::symlink_metadata(&entry_path)
            .unwrap_or_else(|error| panic!("cannot stat {}: {error}", entry_path.display()));
        assert!(
            entry_type.file_type().is_symlink(),
            "adapter entry must be a symlink: {}",
            entry_path.display()
        );
        let target = fs::read_link(&entry_path).unwrap_or_else(|error| {
            panic!("cannot read symlink {}: {error}", entry_path.display())
        });
        assert!(
            !target.is_absolute(),
            "adapter symlink must be relative: {} -> {}",
            entry_path.display(),
            target.display()
        );
        let actual_target = fs::canonicalize(&entry_path)
            .unwrap_or_else(|error| panic!("cannot resolve {}: {error}", entry_path.display()));
        let expected_target = fs::canonicalize(canonical_skill_for_name(root, &name))
            .unwrap_or_else(|error| panic!("cannot resolve canonical skill {name}: {error}"));
        assert_eq!(
            actual_target,
            expected_target,
            "adapter symlink must resolve to the canonical skill directory: {}",
            entry_path.display()
        );
    }
    assert_eq!(
        actual_names,
        expected_names,
        "{}: adapter entries must be exactly the approved skill names",
        adapter_path.display()
    );
}

fn assert_claude_alias(root: &Path) {
    let alias = root.join("CLAUDE.md");
    let alias_type = fs::symlink_metadata(&alias).unwrap_or_else(|error| {
        panic!(
            "missing Claude instruction alias {}: {error}",
            alias.display()
        )
    });
    assert!(
        alias_type.file_type().is_symlink(),
        "CLAUDE.md must be a symlink: {}",
        alias.display()
    );
    let target = fs::read_link(&alias)
        .unwrap_or_else(|error| panic!("cannot read symlink {}: {error}", alias.display()));
    assert!(
        !target.is_absolute(),
        "CLAUDE.md symlink must be relative: {} -> {}",
        alias.display(),
        target.display()
    );
    assert_eq!(
        target,
        Path::new("AGENTS.md"),
        "CLAUDE.md must link exactly to AGENTS.md"
    );
    let actual_target = fs::canonicalize(&alias)
        .unwrap_or_else(|error| panic!("cannot resolve {}: {error}", alias.display()));
    let expected_target = fs::canonicalize(root.join("AGENTS.md")).unwrap_or_else(|error| {
        panic!(
            "cannot resolve {}: {error}",
            root.join("AGENTS.md").display()
        )
    });
    assert_eq!(
        actual_target, expected_target,
        "CLAUDE.md must resolve to AGENTS.md"
    );
}

fn fake_command_dir(name: &str, command: &str, body: &str) -> PathBuf {
    let directory = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("dash-gate")
        .join(name);
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("create fake command directory");
    let path = directory.join(command);
    fs::write(&path, body).expect("write fake command");
    make_executable(&path);
    directory
}

#[test]
fn pinned_skill_metadata_and_files_are_exact() {
    let root = repo_root();
    assert_canonical_skill_files(&root);

    let expected_roots: BTreeSet<_> = APPROVED_UPSTREAMS
        .iter()
        .map(|&(version_root, _, _, _, _)| version_root)
        .collect();
    assert_eq!(
        approved_version_roots(),
        expected_roots,
        "pinned metadata roots must cover the APPROVED_SKILLS version roots exactly"
    );
    for version_root in approved_version_roots() {
        assert_upstream_metadata(&root, version_root);
    }
}

#[test]
fn adapter_aliases_have_exact_canonical_topology() {
    let root = repo_root();
    assert_canonical_skill_files(&root);
    assert_adapter_topology(&root, ".agents/skills");
    assert_adapter_topology(&root, ".claude/skills");
    assert_claude_alias(&root);
}

#[test]
fn scan_fails_on_every_banned_codepoint_and_names_the_line() {
    for (dash, label) in BANNED {
        let body = format!("first line is clean\nsecond line has one {dash} here\n");
        let root = fixture_tree(&format!("offending-{label}"), &body);

        let (success, output) = scan(&root);
        assert!(
            !success,
            "the tier0 dash scan must fail closed on {label}; output:\n{output}"
        );
        assert!(
            output.contains("nested/sample.md:2"),
            "the scan must report the offending file:line for {label}; output:\n{output}"
        );
    }
}

#[test]
fn scan_passes_on_a_clean_tree() {
    let root = fixture_tree("clean", "no banned character on this line\n");

    let (success, output) = scan(&root);
    assert!(
        success,
        "the tier0 dash scan must pass a tree whose only dash is the ASCII hyphen; output:\n{output}"
    );
}

#[test]
fn scan_allows_only_exact_agent_assets_and_skips_grep_when_all_are_exempt() {
    let root = git_fixture_tree("approved-agent-assets");
    write_dash_file(&root.join("AGENTS.md"));
    write_dash_file(&root.join("tests/AGENTS.md"));
    write_dash_file(&root.join("labs/venus-vulkan-video/AGENTS.md"));
    write_dash_file(&root.join("CLAUDE.md"));
    for notice in APPROVED_NOTICES {
        write_dash_file(&root.join(notice));
    }
    create_canonical_skills(&root);
    create_approved_adapters(&root);

    let grep = fake_command_dir("all-exempt-grep", "grep", "#!/bin/sh\nexit 99\n");
    let (success, output) = scan_with_path(&root, Some(&grep));

    assert!(
        success,
        "all recognized instruction, canonical skill, and adapter paths must pass; output:\n{output}"
    );
    assert!(
        output.contains("grep skipped"),
        "an all-exempt tree must skip grep after proving enumeration was non-empty; output:\n{output}"
    );
}

#[test]
fn scan_rejects_lookalikes_unapproved_assets_and_ordinary_files() {
    let root = git_fixture_tree("unapproved-agent-assets");
    create_canonical_skills(&root);

    write_dash_file(&root.join("docs/AGENTS.md"));
    write_dash_file(&root.join("AGENTS.md.bak"));
    write_dash_file(&root.join("tests/AGENTS.md.bak"));
    write_dash_file(&root.join("CLAUDE.md.bak"));
    write_dash_file(&root.join("README.md"));
    write_dash_file(&root.join("docs/plan.md"));
    write_dash_file(&root.join("docs/plans/entry.md"));
    write_dash_file(&root.join("changelog.d/entry.md"));
    write_dash_file(&root.join("config.nix"));
    write_dash_file(&root.join("third_party/agent-skills/other/v4.9.0/skills/ponytail/SKILL.md"));
    write_dash_file(
        &root.join("third_party/agent-skills/ponytail/v4.9.1/skills/ponytail/SKILL.md"),
    );
    write_dash_file(
        &root.join("third_party/agent-skills/ponytail/v4.9.0/skills/ponytail-extra/SKILL.md"),
    );
    write_dash_file(
        &root.join("third_party/agent-skills/caveman/v2.0.0/skills/caveman-compress/SKILL.md"),
    );
    write_dash_file(&root.join("third_party/agent-skills/caveman/v2.0.1/LICENSE"));
    write_dash_file(&root.join("third_party/agent-skills/caveman/v2.0.0/LICENSING.md"));

    write_dash_file(&root.join(".agents/skills/not-approved"));
    write_dash_file(&root.join(".agents/skills/ponytail/SKILL.md"));
    write_dash_file(&root.join(".claude/skills/not-approved"));
    write_dash_file(&root.join(".claude/skills/ponytail/regular.md"));
    write_dash_file(&root.join("outside.md"));
    fs::create_dir_all(root.join(".claude/skills/ponytail")).expect("create fallback directory");
    std::os::unix::fs::symlink(
        "../../../outside.md",
        root.join(".claude/skills/ponytail/escape.md"),
    )
    .expect("create invalid claude component link");

    let (success, output) = scan(&root);

    assert!(
        !success,
        "lookalikes, ordinary files, and invalid adapter entries must fail; output:\n{output}"
    );
    for path in [
        "docs/AGENTS.md",
        "AGENTS.md.bak",
        "tests/AGENTS.md.bak",
        "CLAUDE.md.bak",
        "README.md",
        "docs/plan.md",
        "docs/plans/entry.md",
        "changelog.d/entry.md",
        "config.nix",
        "third_party/agent-skills/other/v4.9.0/skills/ponytail/SKILL.md",
        "third_party/agent-skills/ponytail/v4.9.1/skills/ponytail/SKILL.md",
        "third_party/agent-skills/ponytail/v4.9.0/skills/ponytail-extra/SKILL.md",
        "third_party/agent-skills/caveman/v2.0.0/skills/caveman-compress/SKILL.md",
        "third_party/agent-skills/caveman/v2.0.1/LICENSE",
        "third_party/agent-skills/caveman/v2.0.0/LICENSING.md",
        ".agents/skills/not-approved",
        ".agents/skills/ponytail/SKILL.md",
        ".claude/skills/not-approved",
        ".claude/skills/ponytail/regular.md",
        ".claude/skills/ponytail/escape.md",
    ] {
        assert!(
            output.contains(path),
            "the scan must report the denied path {path}; output:\n{output}"
        );
    }
    assert!(
        !output.contains("third_party/agent-skills/ponytail/v4.9.0/skills/ponytail/SKILL.md"),
        "the exact canonical skill path must be exempt; output:\n{output}"
    );
}

#[test]
fn scan_ignores_dash_bytes_inside_a_binary_file() {
    // A leading NUL marks the file binary to `grep -I`. The banned dash bytes
    // sit right after it, so if `grep -I` were dropped the scan would match
    // them and fail; the pass here is binary-skip, not a dead scan.
    let dash = '\u{2014}';
    let root = fixture_tree("binary", "clean text\n");
    let mut buf = [0u8; 4];
    let mut blob: Vec<u8> = vec![0x00];
    blob.extend_from_slice(dash.encode_utf8(&mut buf).as_bytes());
    blob.extend_from_slice(&[0x01, 0x02, 0xff]);
    blob.extend_from_slice(b"payload");
    fs::write(root.join("blob.bin"), &blob).expect("write binary fixture");

    let (success, output) = scan(&root);
    assert!(
        success,
        "grep -I must skip the binary blob even though it embeds {dash:?}; output:\n{output}"
    );

    // The identical codepoint in a text file (no NUL) must still fail, which
    // proves the pass above is binary-skip and not a scan that stopped matching.
    let text_root = fixture_tree("binary-text-control", &format!("has one {dash} here\n"));
    let (text_success, text_output) = scan(&text_root);
    assert!(
        !text_success,
        "the same codepoint in a text file must fail; output:\n{text_output}"
    );
    assert!(
        text_output.contains("nested/sample.md:1"),
        "the text control must report the offending file:line; output:\n{text_output}"
    );
}

#[test]
fn scan_fails_closed_when_the_file_enumerator_fails() {
    let root = git_fixture_tree("enumerator-error");
    write_dash_file(&root.join("ordinary.md"));
    let fake_git = fake_command_dir(
        "enumerator-error-git",
        "git",
        "#!/bin/sh\nif [ \"$3\" = \"rev-parse\" ]; then printf '%s\\n' \"$2\"; exit 0; fi\nexit 42\n",
    );

    let (success, output) = scan_with_path(&root, Some(&fake_git));

    assert!(
        !success,
        "enumerator failure must fail closed instead of scanning a partial tree; output:\n{output}"
    );
    assert!(
        output.contains("enumerator exited 42"),
        "enumerator failure must remain visible in the diagnostic; output:\n{output}"
    );
}

#[test]
fn scan_fails_closed_when_grep_returns_an_error() {
    let root = fixture_tree("grep-error", "ordinary text\n");
    let fake_grep = fake_command_dir("grep-error-command", "grep", "#!/bin/sh\nexit 2\n");

    let (success, output) = scan_with_path(&root, Some(&fake_grep));

    assert!(
        !success,
        "a grep error must fail closed instead of reporting a clean scan; output:\n{output}"
    );
    assert!(
        output.contains("grep exited 2"),
        "grep error status must remain visible in the diagnostic; output:\n{output}"
    );
}

#[test]
fn scan_fails_closed_on_an_unreadable_file() {
    use std::os::unix::fs::PermissionsExt;

    let root = fixture_tree("unreadable", "clean text\n");
    let secret = root.join("nested/secret.md");
    fs::write(&secret, "clean text\n").expect("write secret fixture");
    let mut perms = fs::metadata(&secret).expect("metadata").permissions();
    perms.set_mode(0o000);
    fs::set_permissions(&secret, perms).expect("chmod 000");

    // Running as root bypasses the mode bits, so the grep-error path cannot be
    // exercised; skip rather than assert a pass we cannot reach.
    if fs::read(&secret).is_ok() {
        let mut restore = fs::metadata(&secret).expect("metadata").permissions();
        restore.set_mode(0o644);
        let _ = fs::set_permissions(&secret, restore);
        eprintln!("skipping: cannot make a file unreadable to this uid (running as root?)");
        return;
    }

    let (success, output) = scan(&root);

    let mut restore = fs::metadata(&secret).expect("metadata").permissions();
    restore.set_mode(0o644);
    let _ = fs::set_permissions(&secret, restore);

    assert!(
        !success,
        "the scan must fail closed when grep cannot read a file, not report a pass having \
         scanned nothing; output:\n{output}"
    );
    assert!(
        output.contains("grep exited"),
        "the scan must name the grep error status; output:\n{output}"
    );
}

#[test]
fn the_repository_carries_no_non_ascii_dash() {
    let (success, output) = scan(&repo_root());
    assert!(
        success,
        "only the ASCII hyphen may spell a dash (AGENTS.md); output:\n{output}"
    );
}

#[test]
fn the_gate_matches_codepoints_and_declares_a_closed_asset_allowlist() {
    let gate = read_repo_file(GATE);
    for (dash, label) in BANNED {
        assert!(
            !gate.contains(*dash),
            "{GATE} must not carry a literal {label}; it would flag its own source"
        );
        let escape = format!(r"$'\u{}'", label.trim_start_matches("U+"));
        assert!(
            gate.contains(&escape),
            "{GATE} must match {label} by codepoint escape ({escape}) so the whole class is \
             rejected and the pattern survives editing"
        );
    }
    assert!(
        gate.contains("scan_dashes \"$ROOT\""),
        "{GATE} must run the repository-wide scan in its main body, not only in scan mode"
    );
    for required in [
        "DASH_EXEMPT_INSTRUCTION_PATHS",
        "DASH_APPROVED_SKILL_ROOTS",
        "DASH_EXEMPT_NOTICE_PATHS",
        "DASH_APPROVED_ADAPTER_ROOTS",
        "dash_canonical_skill_dir",
        "dash_symlink_matches",
        "dash_path_is_exempt",
        "grep skipped",
        "third_party/agent-skills/ponytail/v4.9.0/skills",
        "third_party/agent-skills/caveman/v2.0.0/skills",
        "third_party/agent-skills/compound-engineering/compound-engineering-v3.21.4/skills",
    ] {
        assert!(
            gate.contains(required),
            "{GATE} must declare the narrow dash allowlist element {required}"
        );
    }
    for retired_admission in [
        "CAVEMAN_DASH_ADMISSIONS",
        "validate_caveman_dash_admissions",
        "is_caveman_dash_admission",
    ] {
        assert!(
            !gate.contains(retired_admission),
            "{GATE} must not restore retired vendor bypass {retired_admission}"
        );
    }
}

#[test]
fn check_tier0_runs_the_gate() {
    let makefile = read_repo_file("Makefile");
    let wired = makefile
        .lines()
        .any(|line| line.trim_start().starts_with("bash ") && line.contains(GATE));
    assert!(
        wired,
        "the Makefile `check-tier0` target must run {GATE}; the dash ban has no other gate"
    );
}

#[test]
fn agents_md_states_the_prohibition() {
    let agents = read_repo_file("AGENTS.md");
    for (dash, label) in BANNED {
        assert!(
            agents.contains(label),
            "AGENTS.md must name {label} so the rule is greppable"
        );
        assert!(
            !agents.contains(*dash),
            "AGENTS.md must not use the {label} character it bans"
        );
    }
}
