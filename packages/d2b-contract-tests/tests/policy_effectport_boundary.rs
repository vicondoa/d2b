//! EffectPort and worker-capability boundary policy.
//!
//! Provider controllers decide semantic work through typed ports.  Broker
//! adapters and attachment/socket seams are the only places where the
//! corresponding low-level representation may appear.  Workers are narrower:
//! they inherit authority through their LaunchTicket and may not regain a
//! broker, bus, credential, process-spawn, or host-locator capability.

use std::{
    fs,
    path::{Path, PathBuf},
};

use d2b_contract_tests::repo_root;

const NON_PROVIDER_PREFIXED: &[&str] = &[
    "d2b-provider",
    "d2b-provider-supervisor",
    "d2b-provider-toolkit",
];

const BROKER_MARKERS: &[&str] = &[
    "d2b_priv_broker",
    "d2b-priv-broker",
    "broker_wire::",
    "BrokerRequest",
    "BrokerResponse",
];

const RAW_HOST_MARKERS: &[&str] = &[
    "std::process::Command",
    "Command::new(",
    "systemctl",
    "std::fs::",
    "std::os::unix::net",
    "rustix::net",
    "nix::sys::socket",
    "zbus::",
    "tokio_vsock",
];

const WORKER_CAPABILITY_MARKERS: &[&str] = &[
    "ResourceClient",
    "d2b-bus",
    "d2b_bus",
    "Credential",
    "std::process::Command",
    "Command::new(",
    "tokio::process",
    "std::process::Stdio",
    "broker_wire::",
    "BrokerRequest",
    "BrokerResponse",
    "UnixStream",
    "UnixListener",
    "UnixDatagram",
    "systemctl",
    "nix::",
    "rustix::net",
    "std::fs::",
    "std::path::Path",
    "PathBuf",
];

fn provider_source_files() -> Vec<(String, PathBuf)> {
    let packages = repo_root().join("packages");
    let mut files = Vec::new();
    let entries = fs::read_dir(&packages).expect("read packages directory");
    for entry in entries {
        let entry = entry.expect("read packages entry");
        let file_type = entry.file_type().expect("inspect packages entry");
        let name = entry.file_name().to_string_lossy().into_owned();
        if !file_type.is_dir()
            || !name.starts_with("d2b-provider-")
            || NON_PROVIDER_PREFIXED.contains(&name.as_str())
        {
            continue;
        }
        collect_rust_files(&entry.path().join("src"), &mut files);
    }
    files
        .into_iter()
        .map(|path| {
            let relative = path
                .strip_prefix(repo_root())
                .expect("Provider source is below repository root")
                .to_string_lossy()
                .replace('\\', "/");
            (relative, path)
        })
        .collect()
}

fn collect_rust_files(root: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

fn code_line(line: &str) -> &str {
    line.split("//").next().unwrap_or_default()
}

fn typed_effect_seam(path: &str) -> bool {
    [
        "/src/broker.rs",
        "/src/effect_port.rs",
        "/src/effects.rs",
        "/src/fd.rs",
        "/src/emitter_socket.rs",
        "/src/identity.rs",
        "/src/portal.rs",
    ]
    .iter()
    .any(|suffix| path.ends_with(suffix))
}

fn boundary_violations(path: &str, source: &str) -> Vec<String> {
    let seam = typed_effect_seam(path);
    let mut violations = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let code = code_line(line);
        for marker in BROKER_MARKERS {
            if code.contains(marker) && !seam {
                violations.push(format!(
                    "{path}:{}: direct broker marker `{marker}`",
                    index + 1
                ));
            }
        }
        for marker in RAW_HOST_MARKERS {
            if code.contains(marker) && !seam {
                violations.push(format!("{path}:{}: raw host marker `{marker}`", index + 1));
            }
        }
    }
    violations
}

fn worker_violations(path: &str, source: &str) -> Vec<String> {
    let mut violations = Vec::new();
    for (index, line) in source.lines().enumerate() {
        let code = code_line(line);
        for marker in WORKER_CAPABILITY_MARKERS {
            if code.contains(marker) {
                violations.push(format!(
                    "{path}:{}: worker capability marker `{marker}`",
                    index + 1
                ));
            }
        }
    }
    violations
}

#[test]
fn provider_sources_use_only_typed_effect_seams_for_raw_interfaces() {
    let mut violations = Vec::new();
    for (relative, path) in provider_source_files() {
        let source = fs::read_to_string(&path).expect("read Provider source");
        violations.extend(boundary_violations(&relative, &source));
    }
    assert!(
        violations.is_empty(),
        "Provider source crossed an EffectPort boundary:\n{}",
        violations.join("\n")
    );
}

#[test]
fn provider_workers_inherit_authority_instead_of_reacquiring_it() {
    let mut violations = Vec::new();
    for (relative, path) in provider_source_files() {
        let is_worker = Path::new(&relative)
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.contains("worker"));
        if !is_worker {
            continue;
        }
        let source = fs::read_to_string(&path).expect("read Provider worker");
        violations.extend(worker_violations(&relative, &source));
    }
    assert!(
        violations.is_empty(),
        "Provider worker crossed its inherited-capability boundary:\n{}",
        violations.join("\n")
    );
}

#[test]
fn direct_broker_imports_are_rejected_outside_a_typed_seam() {
    let violations = boundary_violations(
        "packages/d2b-provider-example/src/controller.rs",
        "use d2b_contracts::broker_wire::BrokerRequest;",
    );
    assert_eq!(violations.len(), 2);
    assert!(
        violations
            .iter()
            .any(|violation| violation.contains("BrokerRequest"))
    );
}

#[test]
fn worker_capability_imports_are_rejected_by_the_same_checker() {
    let violations = worker_violations(
        "packages/d2b-provider-example/src/worker.rs",
        "use d2b_resource_api::ResourceClient;\nuse std::process::Command;",
    );
    assert_eq!(violations.len(), 2);
}
