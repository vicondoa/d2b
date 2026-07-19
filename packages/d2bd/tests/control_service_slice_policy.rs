use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Component as PathComponent, Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Deserialize;

const POLICY_JSON: &str = include_str!("fixtures/control-service-slices.json");
const BASELINE_ROOT: &str = "b2b50e67cfab4fb8601ebb1a63946e84eccba5c1";
const DECLARATIVE_DOCUMENTS: [&str; 2] = [
    "docs/reference/local-root-allocator.md",
    "docs/reference/realm-identity-lifecycle.md",
];
const GUEST_TOKEN_PIN: &str = "tests/golden/pinned/guest-control-token-materializer.txt";
const REQUIRED_OWNERS: [(&str, &str); 26] = [
    (
        "docs/reference/allocator-service-api.md",
        "allocator-child-broker",
    ),
    ("docs/reference/realm-service-identity.md", "realm-service"),
    ("packages/d2b-client/Cargo.toml", "client-cli"),
    ("packages/d2b-daemon-access/Cargo.toml", "client-cli"),
    ("packages/d2b-exec-runner/Cargo.toml", "guest-service"),
    ("packages/d2b-gateway-runtime/Cargo.toml", "provider-agent"),
    (
        "packages/d2b-gateway-runtime/src/bin/d2b-gateway-enroll.rs",
        "provider-agent",
    ),
    (
        "packages/d2b-gateway-runtime/src/bin/d2b-gateway-relay.rs",
        "provider-agent",
    ),
    (
        "packages/d2b-gateway-runtime/src/bin/d2b-provider-agent.rs",
        "provider-agent",
    ),
    ("packages/d2b-gateway/Cargo.toml", "realm-service"),
    ("packages/d2b-guestd/Cargo.toml", "guest-service"),
    ("packages/d2b-host/Cargo.toml", "allocator-child-broker"),
    (
        "packages/d2b-host/tests/guest_control_token_materializer.rs",
        "guest-service",
    ),
    (
        "packages/d2b-host/tests/guest_vsock_ttrpc_compile.rs",
        "guest-service",
    ),
    ("packages/d2b-priv-broker/Cargo.toml", "broker-service"),
    (
        "packages/d2b-priv-broker/tests/common/mod.rs",
        "broker-service",
    ),
    (
        "packages/d2b-realm-codec-protobuf/Cargo.toml",
        "realm-service",
    ),
    ("packages/d2b-realm-provider/Cargo.toml", "provider-agent"),
    ("packages/d2b-realm-router/Cargo.toml", "realm-service"),
    ("packages/d2b-realm-transport/Cargo.toml", "realm-service"),
    ("packages/d2b/Cargo.toml", "client-cli"),
    ("packages/d2b/tests/common/mod.rs", "client-cli"),
    ("packages/d2bd/Cargo.toml", "daemon-service"),
    ("packages/d2bd/tests/common/mod.rs", "daemon-service"),
    (
        "packages/d2bd/tests/daemon_version_file.rs",
        "daemon-service",
    ),
    (GUEST_TOKEN_PIN, "guest-service"),
];
const W5_PREP_FINGERPRINTS: [(&str, &str); 35] = [
    (
        "w5-client-daemon-service",
        "packages/d2b-client/src/daemon_service.rs",
    ),
    (
        "w5-prep-allocator-service-reference",
        "docs/reference/allocator-service-api.md",
    ),
    ("w5-prep-cli-lib", "packages/d2b/src/lib.rs"),
    ("w5-prep-cli-service", "packages/d2b/src/service_v2.rs"),
    ("w5-prep-client-lib", "packages/d2b-client/src/lib.rs"),
    (
        "w5-prep-daemon-access-component-session",
        "packages/d2b-daemon-access/src/component_session.rs",
    ),
    (
        "w5-prep-daemon-access-lib",
        "packages/d2b-daemon-access/src/lib.rs",
    ),
    (
        "w5-prep-daemon-composition-allocator",
        "packages/d2bd/src/control_services/allocator.rs",
    ),
    (
        "w5-prep-daemon-composition-broker",
        "packages/d2bd/src/control_services/broker.rs",
    ),
    (
        "w5-prep-daemon-composition-daemon",
        "packages/d2bd/src/control_services/daemon.rs",
    ),
    (
        "w5-prep-daemon-composition-guest",
        "packages/d2bd/src/control_services/guest.rs",
    ),
    (
        "w5-prep-daemon-composition-provider",
        "packages/d2bd/src/control_services/provider.rs",
    ),
    (
        "w5-prep-daemon-composition-realm",
        "packages/d2bd/src/control_services/realm.rs",
    ),
    (
        "w5-prep-daemon-composition-root",
        "packages/d2bd/src/control_services/mod.rs",
    ),
    ("w5-prep-daemon-lib", "packages/d2bd/src/lib.rs"),
    (
        "w5-prep-daemon-realm-child-supervisor",
        "packages/d2bd/src/realm_child_supervisor.rs",
    ),
    (
        "w5-prep-daemon-slice-policy",
        "packages/d2bd/tests/control_service_slice_policy.rs",
    ),
    (
        "w5-prep-daemon-slice-policy-fixture",
        "packages/d2bd/tests/fixtures/control-service-slices.json",
    ),
    ("w5-prep-guest-lib", "packages/d2b-guestd/src/lib.rs"),
    (
        "w5-prep-guest-service",
        "packages/d2b-guestd/src/service_v2.rs",
    ),
    (
        "w5-prep-guest-token-materializer-pin",
        "tests/golden/pinned/guest-control-token-materializer.txt",
    ),
    (
        "w5-prep-guest-token-materializer-test",
        "packages/d2b-host/tests/guest_control_token_materializer.rs",
    ),
    (
        "w5-prep-guest-vsock-ttrpc-test",
        "packages/d2b-host/tests/guest_vsock_ttrpc_compile.rs",
    ),
    ("w5-prep-host-lib", "packages/d2b-host/src/lib.rs"),
    (
        "w5-prep-host-realm-children",
        "packages/d2b-host/src/realm_children.rs",
    ),
    (
        "w5-prep-priv-broker-allocator-service",
        "packages/d2b-priv-broker/src/allocator_service.rs",
    ),
    (
        "w5-prep-priv-broker-lib",
        "packages/d2b-priv-broker/src/lib.rs",
    ),
    (
        "w5-prep-priv-broker-service",
        "packages/d2b-priv-broker/src/service_v2.rs",
    ),
    (
        "w5-prep-provider-agent-bin",
        "packages/d2b-gateway-runtime/src/bin/d2b-provider-agent.rs",
    ),
    (
        "w5-prep-provider-agent-cargo",
        "packages/d2b-gateway-runtime/Cargo.toml",
    ),
    (
        "w5-prep-provider-agent-composition-lib",
        "packages/d2b-gateway-runtime/src/lib.rs",
    ),
    (
        "w5-prep-provider-agent-service",
        "packages/d2b-gateway-runtime/src/provider_agent.rs",
    ),
    (
        "w5-prep-realm-router-lib",
        "packages/d2b-realm-router/src/lib.rs",
    ),
    (
        "w5-prep-realm-router-service",
        "packages/d2b-realm-router/src/service_v2.rs",
    ),
    (
        "w5-prep-realm-service-identity-reference",
        "docs/reference/realm-service-identity.md",
    ),
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SlicePolicy {
    schema_version: u32,
    baseline_root: String,
    common_prompt: String,
    components: Vec<Slice>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Slice {
    id: String,
    implementation_dependencies: Vec<String>,
    integration_dependencies: Vec<String>,
    implementation_prompt: String,
    owned_files: Vec<String>,
    #[serde(default)]
    retired_read_only_files: Vec<String>,
    baseline_call_graph: Vec<String>,
    retirement_surfaces: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct SharedPolicy {
    waves: Vec<WavePolicy>,
    protected_paths: Vec<String>,
    protected_prefixes: Vec<String>,
    frozen_prefixes: Vec<String>,
    documentation_paths: Vec<String>,
    documentation_prefixes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct WavePolicy {
    wave: String,
    manifest_path: String,
    allowed_prefixes: Vec<String>,
    foreign_prefixes: Vec<String>,
    additional_protected_paths: Vec<String>,
    allowed_protected_paths: Vec<String>,
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root")
        .to_path_buf()
}

fn load_policy() -> SlicePolicy {
    serde_json::from_str(POLICY_JSON).expect("valid control-service slice policy")
}

fn load_shared_policy(root: &Path) -> SharedPolicy {
    let bytes = fs::read(root.join("delivery/shared-contracts.json"))
        .expect("read shared ownership policy");
    serde_json::from_slice(&bytes).expect("valid shared ownership policy")
}

fn assert_sorted(values: &[String], label: &str) {
    assert!(
        values.windows(2).all(|pair| pair[0] < pair[1]),
        "{label} must be strictly sorted"
    );
}

fn assert_relative_file(path: &str) {
    let candidate = Path::new(path);
    assert!(!candidate.as_os_str().is_empty(), "owned path is empty");
    assert!(!candidate.is_absolute(), "owned path must be relative");
    assert!(
        candidate
            .components()
            .all(|component| matches!(component, PathComponent::Normal(_))),
        "owned path must be normalized"
    );
    assert!(!path.ends_with('/'), "owned path must name an exact file");
}

fn path_has_prefix(path: &str, prefixes: &[String]) -> bool {
    prefixes.iter().any(|prefix| path.starts_with(prefix))
}

fn rust_files_below(root: &Path, relative: &str) -> Vec<String> {
    fn visit(root: &Path, directory: &Path, files: &mut Vec<String>) {
        let mut entries = fs::read_dir(directory)
            .expect("read test directory")
            .map(|entry| entry.expect("read test entry"))
            .collect::<Vec<_>>();
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            let file_type = entry.file_type().expect("read test entry type");
            if file_type.is_dir() {
                visit(root, &path, files);
            } else if file_type.is_file() && path.extension().is_some_and(|value| value == "rs") {
                files.push(
                    path.strip_prefix(root)
                        .expect("test path below repository")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }

    let mut files = Vec::new();
    visit(root, &root.join(relative), &mut files);
    files
}

fn validate_dependency_graph(
    slices: &BTreeMap<&str, &Slice>,
    selector: impl Fn(&Slice) -> &[String] + Copy,
) {
    fn visit<'a>(
        id: &'a str,
        slices: &BTreeMap<&'a str, &'a Slice>,
        selector: impl Fn(&Slice) -> &[String] + Copy,
        active: &mut BTreeSet<&'a str>,
        complete: &mut BTreeSet<&'a str>,
    ) {
        if complete.contains(id) {
            return;
        }
        assert!(
            active.insert(id),
            "component dependency graph contains a cycle"
        );
        for dependency in selector(slices[id]) {
            assert!(
                slices.contains_key(dependency.as_str()),
                "component names an unknown dependency"
            );
            visit(dependency, slices, selector, active, complete);
        }
        active.remove(id);
        complete.insert(id);
    }

    let mut active = BTreeSet::new();
    let mut complete = BTreeSet::new();
    for id in slices.keys().copied() {
        visit(id, slices, selector, &mut active, &mut complete);
    }
}

#[test]
fn component_file_ownership_is_disjoint_and_within_shared_policy() {
    let root = repository_root();
    let policy = load_policy();
    assert_eq!(policy.schema_version, 3);
    assert_eq!(policy.baseline_root, BASELINE_ROOT);
    assert!(
        policy.common_prompt.contains("Cargo.lock")
            && policy.common_prompt.contains("owned_files")
            && policy.common_prompt.contains("ownership check"),
        "common implementation prompt omits a required boundary"
    );

    let shared = load_shared_policy(&root);
    let wave = shared
        .waves
        .iter()
        .find(|wave| wave.wave == "w5")
        .expect("runtime-service ownership policy");
    assert_eq!(wave.manifest_path, "delivery/manifests/w5.json");

    let component_ids = policy
        .components
        .iter()
        .map(|component| component.id.as_str())
        .collect::<Vec<_>>();
    assert!(
        component_ids.windows(2).all(|pair| pair[0] < pair[1]),
        "component ids must be strictly sorted"
    );
    assert_eq!(
        component_ids,
        [
            "allocator-child-broker",
            "broker-service",
            "client-cli",
            "daemon-service",
            "guest-service",
            "provider-agent",
            "realm-service"
        ]
    );

    let slices = policy
        .components
        .iter()
        .map(|component| (component.id.as_str(), component))
        .collect::<BTreeMap<_, _>>();
    validate_dependency_graph(&slices, |component| &component.implementation_dependencies);
    validate_dependency_graph(&slices, |component| &component.integration_dependencies);
    let allocator = slices["allocator-child-broker"];
    assert!(
        allocator
            .baseline_call_graph
            .iter()
            .any(|entry| entry.contains("live_handlers") && entry.contains("sys::pidfd_sys")),
        "allocator inventory omits the live spawn and pidfd call graph"
    );
    assert!(
        allocator
            .baseline_call_graph
            .iter()
            .all(|entry| !entry.starts_with("d2b-priv-broker::ops::spawn_runner ->")),
        "allocator inventory names the planning facade as the live call graph"
    );

    let mut owners = BTreeMap::new();
    for component in &policy.components {
        assert_sorted(
            &component.implementation_dependencies,
            "implementation dependencies",
        );
        assert_sorted(
            &component.integration_dependencies,
            "integration dependencies",
        );
        assert_sorted(&component.owned_files, "owned files");
        assert_sorted(
            &component.retired_read_only_files,
            "retired read-only files",
        );
        assert!(
            !component.implementation_prompt.trim().is_empty(),
            "component implementation prompt is empty"
        );
        assert!(!component.baseline_call_graph.is_empty());
        assert!(!component.retirement_surfaces.is_empty());

        for path in &component.owned_files {
            assert_relative_file(path);
            assert!(
                owners
                    .insert(path.as_str(), component.id.as_str())
                    .is_none(),
                "an exact file has multiple component owners"
            );

            let documentation_allowed = shared.documentation_paths.contains(path)
                || path_has_prefix(path, &shared.documentation_prefixes);
            assert!(
                path_has_prefix(path, &wave.allowed_prefixes)
                    || documentation_allowed
                    || path == GUEST_TOKEN_PIN,
                "component owns {path} outside runtime-service or documentation authority"
            );

            let protected = (shared.protected_paths.contains(path)
                || wave.additional_protected_paths.contains(path)
                || path_has_prefix(path, &shared.protected_prefixes))
                && !wave.allowed_protected_paths.contains(path);
            assert!(!protected, "component owns shared protected file {path}");
            assert!(
                !path_has_prefix(path, &shared.frozen_prefixes),
                "component owns frozen implementation file {path}"
            );
            assert!(
                !path_has_prefix(path, &wave.foreign_prefixes),
                "component owns another authority's implementation file {path}"
            );
        }
    }

    let guest = slices["guest-service"];
    assert!(
        guest
            .baseline_call_graph
            .iter()
            .any(|entry| entry.contains("guest_control_token_materializer"))
            && guest
                .baseline_call_graph
                .iter()
                .any(|entry| entry.contains("guest_vsock_ttrpc_compile")),
        "guest inventory omits a retired token or direct-vsock path"
    );
    assert_eq!(
        guest.retired_read_only_files,
        [
            "nixos-modules/guest-control-host.nix",
            "nixos-modules/guest-control-token-materialize.py"
        ]
    );
    for component in &policy.components {
        for path in &component.retired_read_only_files {
            assert_relative_file(path);
            assert!(
                root.join(path).is_file(),
                "retired read-only file is absent"
            );
            assert!(
                !owners.contains_key(path.as_str()),
                "retired read-only file also has a component owner"
            );
            assert!(
                shared.protected_paths.contains(path)
                    || path_has_prefix(path, &shared.protected_prefixes)
                    || path_has_prefix(path, &shared.frozen_prefixes)
                    || path_has_prefix(path, &wave.foreign_prefixes),
                "retired read-only file is not protected by shared authority"
            );
        }
    }

    for (path, expected_owner) in REQUIRED_OWNERS {
        assert_eq!(
            owners.get(path).copied(),
            Some(expected_owner),
            "required implementation file has the wrong component owner"
        );
        assert!(
            root.join(path).is_file(),
            "required implementation file is absent"
        );
    }
    for path in DECLARATIVE_DOCUMENTS {
        assert!(
            !owners.contains_key(path),
            "declarative documentation must not be owned by a runtime component"
        );
    }

    for path in rust_files_below(&root, "packages/d2b/tests") {
        assert_eq!(
            owners.get(path.as_str()).copied(),
            Some("client-cli"),
            "CLI integration test is not owned by the client component"
        );
    }
    for path in rust_files_below(&root, "packages/d2bd/tests") {
        if path == "packages/d2bd/tests/control_service_slice_policy.rs" {
            continue;
        }
        let expected = match path.as_str() {
            "packages/d2bd/tests/provider_agent_service_v2.rs" => "provider-agent",
            "packages/d2bd/tests/realm_child_supervisor_v2.rs" => "allocator-child-broker",
            "packages/d2bd/tests/realm_service_v2.rs" => "realm-service",
            _ => "daemon-service",
        };
        assert_eq!(
            owners.get(path.as_str()).copied(),
            Some(expected),
            "daemon integration test is not owned by its service component"
        );
    }
    for path in rust_files_below(&root, "packages/d2b-priv-broker/tests") {
        let expected = if matches!(
            path.as_str(),
            "packages/d2b-priv-broker/tests/allocator_service_v2.rs"
                | "packages/d2b-priv-broker/tests/pidfd_handoff_scm_rights.rs"
                | "packages/d2b-priv-broker/tests/pidfd_real_spawner.rs"
        ) {
            "allocator-child-broker"
        } else {
            "broker-service"
        };
        assert_eq!(
            owners.get(path.as_str()).copied(),
            Some(expected),
            "broker integration test is not owned by its service component"
        );
    }
    for path in rust_files_below(&root, "packages/d2b-gateway-runtime/src/bin") {
        assert_eq!(
            owners.get(path.as_str()).copied(),
            Some("provider-agent"),
            "gateway executable is not owned by the provider-agent component"
        );
    }

    let manifest: serde_json::Value = serde_json::from_slice(
        &fs::read(root.join("delivery/manifests/w5.json")).expect("read delivery manifest"),
    )
    .expect("valid delivery manifest");
    assert!(
        W5_PREP_FINGERPRINTS
            .windows(2)
            .all(|pair| pair[0].0 < pair[1].0),
        "expected preparation fingerprints must be sorted"
    );
    let actual_w5_fingerprints = manifest["contract_fingerprints"]
        .as_array()
        .expect("contract fingerprint array")
        .iter()
        .filter_map(|fingerprint| {
            let name = fingerprint["name"].as_str()?;
            name.starts_with("w5-").then(|| {
                (
                    name,
                    fingerprint["path"]
                        .as_str()
                        .expect("fingerprint path string"),
                )
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        actual_w5_fingerprints, W5_PREP_FINGERPRINTS,
        "delivery manifest does not fingerprint the complete preparation set"
    );
    for (name, path) in [
        ("delivery-authority", "delivery/manifests/w5.json"),
        ("shared-changelog", "CHANGELOG.md"),
    ] {
        assert_eq!(
            manifest["contract_fingerprints"]
                .as_array()
                .expect("contract fingerprint array")
                .iter()
                .filter(|fingerprint| fingerprint["name"] == name && fingerprint["path"] == path)
                .count(),
            1,
            "shared preparation file is not fingerprinted exactly once"
        );
    }

    let gateway_manifest = fs::read_to_string(root.join("packages/d2b-gateway-runtime/Cargo.toml"))
        .expect("read gateway runtime manifest");
    assert!(
        gateway_manifest.contains("[[bin]]")
            && gateway_manifest.contains("name = \"d2b-provider-agent\"")
            && gateway_manifest.contains("path = \"src/bin/d2b-provider-agent.rs\""),
        "gateway runtime manifest omits the reserved provider-agent entrypoint"
    );
}

fn git(root: &Path, args: &[&str]) -> String {
    git_output(root, args, None)
}

fn git_with_external_metadata(
    root: &Path,
    args: &[&str],
    graft_file: &Path,
    shallow_file: &Path,
) -> String {
    git_output(root, args, Some((graft_file, shallow_file)))
}

fn git_output(root: &Path, args: &[&str], external_metadata: Option<(&Path, &Path)>) -> String {
    let mut command = Command::new("git");
    if let Some((graft_file, shallow_file)) = external_metadata {
        command
            .env("GIT_GRAFT_FILE", graft_file)
            .env("GIT_SHALLOW_FILE", shallow_file);
    }
    let output = command
        .current_dir(root)
        .env("GIT_GRAFT_FILE", "/dev/null")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .env("GIT_SHALLOW_FILE", "/dev/null")
        .arg("--no-replace-objects")
        .args(["-c", "diff.ignoreSubmodules=none"])
        .args(args)
        .output()
        .expect("execute git");
    assert!(
        output.status.success(),
        "git command failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output is UTF-8")
        .trim()
        .to_owned()
}

fn git_common_dir(root: &Path) -> PathBuf {
    let common = PathBuf::from(git(root, &["rev-parse", "--git-common-dir"]));
    fs::canonicalize(if common.is_absolute() {
        common
    } else {
        root.join(common)
    })
    .expect("canonical Git common directory")
}

fn history_rewrite_error(root: &Path) -> Option<String> {
    if !git(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/replace"],
    )
    .is_empty()
    {
        return Some("repository contains forbidden refs/replace metadata".to_owned());
    }
    let common = git_common_dir(root);
    for (relative, label) in [("info/grafts", "graft"), ("shallow", "shallow")] {
        match fs::symlink_metadata(common.join(relative)) {
            Ok(_) => {
                return Some(format!(
                    "repository contains forbidden Git {label} metadata"
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Some(format!("cannot inspect Git {label} metadata: {error}"));
            }
        }
    }
    None
}

fn assert_no_history_rewrites(root: &Path) {
    if let Some(error) = history_rewrite_error(root) {
        panic!("{error}");
    }
}

fn assert_clean(root: &Path) {
    assert!(
        git(
            root,
            &[
                "status",
                "--porcelain=v1",
                "--untracked-files=all",
                "--ignore-submodules=none"
            ]
        )
        .is_empty(),
        "slice policy requires a clean worktree"
    );
}

struct GitScratch {
    root: PathBuf,
}

impl GitScratch {
    fn new(label: &str) -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/control-service-slice-policy")
            .join(format!(
                "{label}-{}-{nonce}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
        fs::create_dir_all(&root).expect("create policy Git scratch");
        setup_git(&root, &["init", "--quiet"]);
        setup_git(&root, &["config", "user.name", "d2b-test"]);
        setup_git(&root, &["config", "user.email", "d2b-test@example.invalid"]);
        Self { root }
    }

    fn commit(&self, content: &str) -> String {
        fs::write(self.root.join("tracked"), content).expect("write tracked test file");
        setup_git(&self.root, &["add", "tracked"]);
        setup_git(&self.root, &["commit", "--quiet", "-m", content]);
        setup_git_output(&self.root, &["rev-parse", "HEAD"])
    }
}

impl Drop for GitScratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn setup_git(root: &Path, args: &[&str]) {
    let _ = git(root, args);
}

fn setup_git_output(root: &Path, args: &[&str]) -> String {
    git(root, args)
}

#[test]
fn history_rewrite_metadata_is_rejected_before_component_diff() {
    let replace = GitScratch::new("replace");
    let replaced = replace.commit("base");
    let replacement = replace.commit("replacement");
    setup_git(
        &replace.root,
        &[
            "update-ref",
            &format!("refs/replace/{replaced}"),
            &replacement,
        ],
    );
    assert!(
        history_rewrite_error(&replace.root).is_some_and(|error| error.contains("refs/replace"))
    );

    let graft = GitScratch::new("graft");
    let graft_parent = graft.commit("base");
    let graft_head = graft.commit("head");
    let grafts = graft.root.join(".git/info/grafts");
    fs::create_dir_all(grafts.parent().expect("grafts parent")).expect("create grafts parent");
    fs::write(grafts, format!("{graft_head} {graft_parent}\n")).expect("write graft metadata");
    assert!(history_rewrite_error(&graft.root).is_some_and(|error| error.contains("graft")));

    let shallow = GitScratch::new("shallow");
    let shallow_head = shallow.commit("head");
    fs::write(
        shallow.root.join(".git/shallow"),
        format!("{shallow_head}\n"),
    )
    .expect("write shallow metadata");
    assert!(history_rewrite_error(&shallow.root).is_some_and(|error| error.contains("shallow")));
}

#[test]
fn external_graft_and_shallow_environment_cannot_change_ancestry() {
    let repository = GitScratch::new("external-metadata");
    let base = repository.commit("base");
    let head = repository.commit("head");
    let expected = format!("{head} {base}");
    let absent = repository.root.join("absent-metadata");

    let graft = repository.root.join("caller-grafts");
    fs::write(&graft, format!("{head}\n")).expect("write caller graft metadata");
    assert_eq!(
        git_with_external_metadata(
            &repository.root,
            &["rev-list", "--parents", "-n", "1", &head],
            &graft,
            &absent,
        ),
        expected
    );

    let shallow = repository.root.join("caller-shallow");
    fs::write(&shallow, format!("{head}\n")).expect("write caller shallow metadata");
    assert_eq!(
        git_with_external_metadata(
            &repository.root,
            &["rev-list", "--parents", "-n", "1", &head],
            &absent,
            &shallow,
        ),
        expected
    );
}

#[test]
fn candidate_diff_is_limited_to_its_exact_component_files() {
    let component_id = env::var("D2B_CONTROL_SERVICE_SLICE").ok();
    let candidate_root = env::var_os("D2B_CONTROL_SERVICE_CANDIDATE_ROOT");
    if component_id.is_none() && candidate_root.is_none() {
        return;
    }
    let component_id = component_id.expect("both slice policy variables are required");
    let candidate_root =
        PathBuf::from(candidate_root.expect("both slice policy variables are required"));
    let trusted_root = repository_root();

    assert_no_history_rewrites(&trusted_root);
    assert_no_history_rewrites(&candidate_root);
    assert_clean(&trusted_root);
    assert_clean(&candidate_root);

    let trusted_common = git_common_dir(&trusted_root);
    let candidate_common = git_common_dir(&candidate_root);
    assert_eq!(
        trusted_common, candidate_common,
        "candidate must be a worktree of the trusted repository"
    );

    let trusted_head = git(&trusted_root, &["rev-parse", "HEAD"]);
    let candidate_head = git(&candidate_root, &["rev-parse", "HEAD"]);
    assert_ne!(
        trusted_head, candidate_head,
        "candidate must contain a committed component change"
    );
    let base = git(
        &candidate_root,
        &["merge-base", &trusted_head, &candidate_head],
    );
    for authority in [
        "delivery/manifests/w5.json",
        "delivery/shared-contracts.json",
        "packages/d2bd/src/control_services/mod.rs",
        "packages/d2bd/tests/control_service_slice_policy.rs",
        "packages/d2bd/tests/fixtures/control-service-slices.json",
    ] {
        let trusted_object = format!("{trusted_head}:{authority}");
        let base_object = format!("{base}:{authority}");
        assert_eq!(
            git(&trusted_root, &["rev-parse", &trusted_object]),
            git(&candidate_root, &["rev-parse", &base_object]),
            "candidate does not descend from the trusted preparation authority"
        );
    }

    let policy = load_policy();
    let component = policy
        .components
        .iter()
        .find(|component| component.id == component_id)
        .expect("known control-service component");
    let owned = component
        .owned_files
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let range = format!("{base}..{candidate_head}");
    let changed = git(
        &candidate_root,
        &[
            "diff",
            "--name-only",
            "--no-renames",
            "--ignore-submodules=none",
            &range,
            "--",
        ],
    );
    assert!(
        !changed.is_empty(),
        "candidate has no committed file changes"
    );
    for path in changed.lines() {
        assert!(
            owned.contains(path),
            "candidate changed a file outside its exact component ownership"
        );
    }
}
