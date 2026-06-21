//! Host-side source boundary checks for realm relay credentials.

use nixling_contract_tests::repo_root;

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|err| {
        panic!("failed to read {rel}: {err}");
    })
}

fn forbidden_needles() -> Vec<String> {
    vec![
        ["NIXLING", "_RELAY", "_"].concat(),
        ["Gateway", "Credential"].concat(),
        ["Relay", "Credential"].concat(),
        ["Azure", "Relay", "Transport", "Provider"].concat(),
        ["nixling", "-gateway", "-relay"].concat(),
    ]
}

#[test]
fn host_cli_and_host_crates_do_not_depend_on_realm_relay_providers() {
    let checked = [
        "packages/nixling/Cargo.toml",
        "packages/nixling-host/Cargo.toml",
    ];
    let forbidden = [
        ["nixling", "-provider", "-relay"].concat(),
        ["nixling", "-gateway", "-runtime"].concat(),
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
    let checked = ["packages/nixling/src/lib.rs", "packages/nixling-host/src"];
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
