#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

const CONTRACTS_CRATE: &str = "d2b-contracts";
const FOCUSED_POLICY_PACKAGES: &[&str] = &["d2b-priv-broker", "d2b-guest-shell-runner"];
const V2_FOUNDATION_CRATES: &[&str] = &[
    "d2b-client",
    "d2b-provider",
    "d2b-provider-toolkit",
    "d2b-session",
    "d2b-session-unix",
    "d2b-state",
];
const IMPLEMENTATION_CRATES: &[&str] = &[
    "d2b-provider-aca",
    "d2b-provider-host",
    "d2b-provider-relay",
    "d2b-realm-codec-protobuf",
    "d2b-session-unix",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("xtask lives under packages/xtask")
        .to_path_buf()
}

fn read_repo_file(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).expect("read repo file")
}

fn git_tracked_files() -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["ls-files", "-z"])
        .output()
        .expect("run git ls-files");
    assert!(
        output.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|entry| !entry.is_empty())
        .map(|entry| String::from_utf8(entry.to_vec()).expect("tracked paths are UTF-8"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[test]
fn workspace_names_contract_crate_by_role() {
    let workspace = read_repo_file("packages/Cargo.toml");
    assert!(
        workspace.contains(&format!("\"{CONTRACTS_CRATE}\"")),
        "main workspace must include the contract/DTO crate by role"
    );
    assert!(
        !workspace.contains(&format!("\"{}{}\"", "d2b", "-ipc")),
        "main workspace must not reintroduce the old transport-shaped contract crate name"
    );

    let manifest = read_repo_file("packages/d2b-contracts/Cargo.toml");
    assert!(
        manifest.contains(&format!("name = \"{CONTRACTS_CRATE}\"")),
        "contract crate manifest must use the role-based package name"
    );
}

#[test]
fn implementation_crates_are_base_first_and_workspace_members_are_sorted() {
    let workspace = read_repo_file("packages/Cargo.toml");
    for package in IMPLEMENTATION_CRATES {
        assert!(
            workspace.contains(&format!("\"{package}\"")),
            "workspace must contain base-first implementation crate {package}"
        );
    }

    for forbidden in ["d2b-host-providers", "d2b-unix-session"] {
        assert!(
            !workspace.contains(forbidden),
            "workspace must not contain implementation-before-base crate {forbidden}"
        );
    }

    let members = workspace
        .split_once("members = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(members, _)| members)
        .expect("workspace members array");
    let actual = members
        .lines()
        .filter_map(|line| line.trim().strip_prefix('"'))
        .filter_map(|line| line.strip_suffix("\","))
        .collect::<Vec<_>>();
    let mut sorted = actual.clone();
    sorted.sort_unstable();
    assert_eq!(
        actual, sorted,
        "workspace members must remain alphanumerically sorted"
    );
}

#[test]
fn standalone_proofs_isolate_mixed_toolchain_targets() {
    let script = read_repo_file("tests/test-proofs.sh");
    assert!(
        script.contains("CARGO_TARGET_DIR/d2b-proofs/$RUSTUP_TOOLCHAIN")
            && script.contains("proof_target_args=(--target-dir"),
        "standalone proof crates must not share target metadata across rustc versions"
    );
}

#[test]
fn stale_ipc_crate_name_is_absent_from_current_sources() {
    let old_hyphen = format!("{}{}", "d2b", "-ipc");
    let old_underscore = format!("{}{}", "d2b", "_ipc");
    let self_path = "packages/xtask/tests/policy_workspace.rs";
    let mut violations = Vec::new();
    for rel in git_tracked_files() {
        if rel == self_path {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(repo_root().join(&rel)) else {
            continue;
        };
        if content.contains(&old_hyphen) || content.contains(&old_underscore) {
            violations.push(rel);
        }
    }
    assert!(
        violations.is_empty(),
        "stale contract crate references remain:\n{}",
        violations.join("\n")
    );
}

#[test]
fn focused_packages_share_workspace_lock_and_keep_supply_chain_policy() {
    let root = repo_root();
    let main_workspace = read_repo_file("packages/Cargo.toml");
    let flake = read_repo_file("flake.nix");
    assert!(
        flake.contains("packages/Cargo.lock"),
        "flake supply-chain gates must use the canonical workspace lock"
    );
    for package in FOCUSED_POLICY_PACKAGES {
        assert!(
            main_workspace.contains(&format!("\"{package}\"")),
            "main workspace members must include {package}"
        );
        for required in ["Cargo.toml", "deny.toml"] {
            let path = root.join("packages").join(package).join(required);
            assert!(path.exists(), "{} must exist", path.display());
        }

        assert!(
            !root
                .join("packages")
                .join(package)
                .join("Cargo.lock")
                .exists(),
            "{package} must not carry a nested lockfile"
        );
        assert!(
            flake.contains(&format!("packages/{package}/deny.toml")),
            "flake supply-chain gates must cover {package}"
        );
    }
}

#[test]
fn v2_foundation_crates_are_default_empty_and_not_publishable() {
    let root = repo_root();
    let workspace = read_repo_file("packages/Cargo.toml");
    for package in V2_FOUNDATION_CRATES {
        assert!(
            workspace.contains(&format!("\"{package}\""))
                && workspace.contains(&format!(
                    "{package} = {{ path = \"{package}\", version = \"2.0.0\", default-features = false }}"
                )),
            "workspace must own {package} with default features disabled"
        );
        let manifest = read_repo_file(&format!("packages/{package}/Cargo.toml"));
        for required in [
            "version.workspace = true",
            "rust-version.workspace = true",
            "publish = false",
            "[features]\ndefault = []",
            "[lints]\nworkspace = true",
        ] {
            assert!(
                manifest.contains(required),
                "{package} manifest is missing {required:?}"
            );
        }

        assert!(
            !root
                .join("packages")
                .join(package)
                .join("Cargo.lock")
                .exists(),
            "{package} must use the workspace lockfile"
        );
        for dependency in ["d2b-contracts", "d2b-provider", "d2b-session", "ttrpc"] {
            if manifest.contains(&format!("{dependency} =")) {
                assert!(
                    manifest.lines().any(|line| {
                        line.starts_with(&format!("{dependency} ="))
                            && line.contains("default-features = false")
                    }),
                    "{package} must disable default features for {dependency}"
                );
            }
        }
    }
}

#[test]
fn v2_foundation_delivery_fingerprints_cover_every_tracked_file() {
    let manifest: serde_json::Value =
        serde_json::from_str(&read_repo_file("delivery/manifest.json")).expect("delivery manifest");
    let actual = manifest["contract_fingerprints"]
        .as_array()
        .expect("contract fingerprints")
        .iter()
        .filter(|row| {
            row["name"]
                .as_str()
                .is_some_and(|name| name.starts_with("w3-"))
        })
        .map(|row| row["path"].as_str().expect("fingerprint path").to_owned())
        .collect::<BTreeSet<_>>();

    let mut expected = git_tracked_files()
        .into_iter()
        .filter(|path| {
            V2_FOUNDATION_CRATES
                .iter()
                .any(|package| path.starts_with(&format!("packages/{package}/")))
        })
        .collect::<BTreeSet<_>>();
    expected.insert("docs/reference/v2-foundation-crates.md".to_owned());
    expected.insert("packages/xtask/tests/policy_workspace.rs".to_owned());

    assert_eq!(actual, expected);
}

#[test]
fn v2_foundation_io_surfaces_are_async_first() {
    let client = read_repo_file("packages/d2b-client/src/client.rs");
    let connector = read_repo_file("packages/d2b-client/src/session.rs");
    let session_driver = read_repo_file("packages/d2b-session/src/driver.rs");
    let provider_rpc = read_repo_file("packages/d2b-provider/src/rpc.rs");
    for required in [
        "pub async fn connect",
        "pub async fn invoke",
        "pub async fn invoke_with_attachments",
        "pub async fn named_stream",
    ] {
        assert!(client.contains(required), "client is missing {required}");
    }
    assert!(
        connector.contains("#[async_trait]")
            && connector.contains("pub trait ComponentSessionConnector")
            && connector.contains("async fn connect"),
        "client connector must be async"
    );
    assert!(
        session_driver.contains("#[async_trait]")
            && session_driver.contains("pub trait ComponentSessionDriver")
            && session_driver.contains("async fn start_ttrpc")
            && session_driver.contains("async fn receive_ttrpc")
            && session_driver.contains("async fn complete_ttrpc")
            && !session_driver.contains("async fn invoke"),
        "canonical session driver must be async"
    );
    assert!(
        provider_rpc.contains("#[async_trait]")
            && provider_rpc.contains("pub trait AuthenticatedProviderRpc")
            && provider_rpc.contains("async fn invoke"),
        "provider RPC must be async"
    );

    let state_manifest = read_repo_file("packages/d2b-state/Cargo.toml");
    let state_async = read_repo_file("packages/d2b-state/src/tokio_api.rs");
    assert!(
        state_manifest.contains("tokio = [\"host-fs\", \"dep:tokio\"]")
            && state_manifest
                .contains("tokio = { workspace = true, features = [\"rt\"], optional = true }"),
        "state Tokio adapters must remain explicit and optional"
    );
    assert!(
        state_async.contains("tokio::task::spawn_blocking")
            && !state_async.contains("std::fs::")
            && !state_async.contains("thread::sleep"),
        "sync kernel state APIs must be isolated behind spawn_blocking"
    );
    assert!(
        read_repo_file("packages/d2b-state/src/lib.rs").contains(
            "#[cfg(all(feature = \"host-fs\", not(target_os = \"linux\")))]\ncompile_error!(\"the host-fs feature requires Linux\");"
        ),
        "state host filesystem/OFD-lock support must fail explicitly off Linux"
    );
}
