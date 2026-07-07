//! Host-side source boundary checks for realm relay credentials.

use std::collections::BTreeSet;
use std::process::Command;

use d2b_contract_tests::repo_root;

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|err| {
        panic!("failed to read {rel}: {err}");
    })
}

fn git_listed_files(roots: &[&str]) -> Vec<String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(repo_root())
        .args(["ls-files", "-z", "--"])
        .args(roots)
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

fn forbidden_needles() -> Vec<String> {
    vec![
        ["D2B", "_RELAY", "_"].concat(),
        ["Gateway", "Credential"].concat(),
        ["Relay", "Credential"].concat(),
        ["Azure", "Relay", "Transport", "Provider"].concat(),
        ["d2b", "-gateway", "-relay"].concat(),
    ]
}

#[test]
fn host_cli_and_host_crates_do_not_depend_on_realm_relay_providers() {
    let checked = ["packages/d2b/Cargo.toml", "packages/d2b-host/Cargo.toml"];
    let forbidden = [
        ["d2b", "-provider", "-relay"].concat(),
        ["d2b", "-gateway", "-runtime"].concat(),
    ];
    let mut violations = Vec::new();
    for rel in checked {
        let content = read(rel);
        for needle in &forbidden {
            if content.contains(needle) {
                violations.push(format!("{rel}: forbidden host realm-relay dependency"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "host-side realm relay dependency violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn host_cli_and_host_sources_do_not_construct_realm_relay() {
    let checked = ["packages/d2b/src/lib.rs", "packages/d2b-host/src"];
    let forbidden = forbidden_needles();
    let mut violations = Vec::new();
    for rel in checked {
        let paths = if rel.ends_with("/src") {
            collect_rs_files(rel)
        } else {
            vec![rel.to_owned()]
        };
        for path in paths {
            let content = read(&path);
            for needle in &forbidden {
                if content.contains(needle) {
                    violations.push(format!("{path}: forbidden host realm-relay boundary token"));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "host-side realm relay source boundary violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn host_daemon_broker_and_activation_do_not_store_realm_credentials() {
    let checked = git_listed_files(&[
        "packages/d2bd/src",
        "packages/d2b-priv-broker/src",
        "nixos-modules",
    ]);
    let allowlisted = BTreeSet::from([
        "packages/d2bd/src/realm_stubs.rs",
        "packages/d2bd/src/lib.rs",
        "nixos-modules/assertions.nix",
        "nixos-modules/gateway-vm.nix",
        "nixos-modules/options-gateway.nix",
    ]);
    let forbidden = [
        ["Remote", "Daemon", "Access", "Credential"].concat(),
        ["Relay", "Daemon", "Access", "Credential"].concat(),
        ["Browser", "Daemon", "Access", "Credential"].concat(),
        ["Redacted", "Daemon", "Access", "Credential"].concat(),
        ["Relay", "Credential"].concat(),
        ["Gateway", "Credential"].concat(),
        ["Provider", "Credential"].concat(),
        ["realm", "_audit"].concat(),
        ["realm", "Audit"].concat(),
        ["remote", "_node", "_registry"].concat(),
        ["Remote", "Node", "Registry"].concat(),
    ];
    let mut violations = Vec::new();
    for rel in checked {
        if allowlisted.contains(rel.as_str()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(repo_root().join(&rel)) else {
            continue;
        };
        for needle in &forbidden {
            if content.contains(needle) {
                violations.push(format!(
                    "{rel}: forbidden host realm credential/registry token"
                ));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "host daemon/broker/module realm-boundary violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn host_bundle_artifacts_do_not_materialize_realm_credentials_or_registries() {
    let checked = git_listed_files(&["nixos-modules"]);
    let mut violations = Vec::new();
    let forbidden = [
        ["relay", "Credential"].concat(),
        ["provider", "Credential"].concat(),
        ["realm", "Audit"].concat(),
        ["remote", "Node", "Registry"].concat(),
        ["remote", "Registry"].concat(),
    ];
    for rel in checked {
        if !(rel.ends_with("-json.nix")
            || rel.ends_with("manifest.nix")
            || rel.ends_with("bundle-artifacts.nix")
            || rel.ends_with("bundle.nix"))
        {
            continue;
        }
        let content = read(&rel);
        for needle in &forbidden {
            if content.contains(needle) {
                violations.push(format!("{rel}: forbidden host bundle realm artifact token"));
            }
        }
    }
    assert!(
        violations.is_empty(),
        "host-readable bundle artifact realm-boundary violations:\n{}",
        violations.join("\n")
    );
}

#[test]
fn host_relay_credentials_are_explicitly_refused_not_materialized() {
    let daemon = read("packages/d2bd/src/lib.rs");
    assert!(
        daemon.contains("allow_host_relay_credentials")
            && daemon.contains(
                "host-held gateway credentials and relay send-bearer minting are retired"
            ),
        "d2bd must retain the host-relay credential transition guard"
    );

    let host_daemon = read("nixos-modules/host-daemon.nix");
    assert!(
        host_daemon.contains(r#"forbiddenHostEnvPrefixes = [ "D2B_RELAY_" ]"#)
            && host_daemon
                .contains(r#"omitted = [ "payload" "headers" "token" "endpoint" "credential" ]"#),
        "host daemon module must emit only a deny/redaction policy for host relay credentials"
    );
}

fn collect_rs_files(rel_dir: &str) -> Vec<String> {
    let root = repo_root().join(rel_dir);
    let mut out = Vec::new();
    for entry in std::fs::read_dir(&root).unwrap_or_else(|err| {
        panic!("failed to read directory {rel_dir}: {err}");
    }) {
        let entry = entry.expect("read dir entry");
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(
                path.strip_prefix(repo_root())
                    .expect("repo-relative")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
    }
    out.sort();
    out
}
