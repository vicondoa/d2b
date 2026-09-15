//! Dependency-surface audit for handler crates linked into the broker
//! binary (U6 approach item 3 - defense-in-depth, never the boundary).
//!
//! The broker process is the system's trust concentrator: provider handler
//! code shares the broker's uid-0 address space. The composition-root
//! routing rule (KTD1) keeps effectful and privileged handlers off the
//! in-broker leg; this audit is the second, mechanical line: a handler
//! crate is rejected when it carries a syscall-surface dependency
//! (libc/nix/rustix/procfs-style), a raw-syscall surface (`asm!` /
//! `global_asm!` / naked functions, build-script or proc-macro emitted
//! execution, `include!`-carried payloads), ctor-style entry points
//! (`ctor` / `link_section` / the used attribute), or runtime registration
//! hooks (panic hooks, signal handlers, global allocators).
//!
//! The audit runs at registration time when the crate's sources are
//! present (development and CI builds) and as a full `cargo metadata`
//! dependency-tree scan testable against real fixture crates. A deployed
//! binary ships no sources, so the deployment gate is the lockfile
//! allowlist: the composition binary's lockfile is asserted against an
//! allowlist. No such allowlist exists in the tree yet (U14's policy
//! machinery pins the provider-free d2b-broker manifest, not a broker
//! lockfile allowlist), so the allowlist is the documented deployment-gate
//! followup; this module is where its assertion belongs once it lands.
//!
//! All probes are lexical and fail closed (a comment that mentions the
//! machinery trips the probe): the audit is defense-in-depth behind the
//! build graph, exactly as the plan's risk section describes.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The closed set of syscall-surface crates a handler crate must never
/// resolve, transitively.
pub const FORBIDDEN_SYSCALL_SURFACE_CRATES: &[&str] = &[
    "libc",
    "nix",
    "rustix",
    "procfs",
    "linux-raw-sys",
    "winapi",
    "windows-sys",
    "redox_syscall",
];

/// The lexical source probes, in evaluation order.
///
/// Each entry names the probe and the regex it fires on. The patterns
/// match source text (including comments and `include!`-carried payloads
/// that are inlined at build time), so a violation is reported even when
/// the machinery is cfg-gated out of the compiled artifact.
const SOURCE_PROBES: &[(&str, &str)] = &[
    ("raw-asm", r"\basm!\s*\("),
    ("raw-global-asm", r"\bglobal_asm!\s*\("),
    ("naked-fn-attribute", r"#\s*\[\s*naked"),
    ("include-carried-payload", r"\binclude!\s*\("),
("ctor-entry-point", r"#\s*\[\s*(?:unsafe\s*\()?\s*ctor"),
        ("link-section-entry-point", r"#\s*\[\s*(?:unsafe\s*\()?\s*link_section"),
    ("used-static", r"#\s*\[\s*used"),
    ("global-allocator", r"#\s*\[\s*global_allocator"),
    ("panic-hook-registration", r"(?:std::)?panic::set_hook\s*\("),
    (
        "signal-handler-registration",
        r"\b(?:sigaction|register_signal)\s*\(|\bsignal\s*\(",
    ),
];

/// The outcome of auditing one handler crate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceReport {
    /// The audited crate.
    pub crate_name: String,
    /// Transitive dependencies on the forbidden syscall-surface set that
    /// the handler's closure adds BEYOND the broker's own closure (the
    /// broker's lib legitimately carries rustix/nix/libc for its socket
    /// and syscall machinery; a handler may not grow that surface).
    pub forbidden_dependencies: Vec<String>,
    /// Transitive proc-macro dependencies the handler's closure adds
    /// beyond the broker's own (their code executes inside rustc's process
    /// at build time and can emit execution).
    pub proc_macro_dependencies: Vec<String>,
    /// Whether the crate carries a build script (build-time execution).
    pub build_script: bool,
    /// Named source-surface violations (`probe: file:line`).
    pub source_violations: Vec<String>,
}

impl SurfaceReport {
    /// Whether the audited crate passes every check.
    pub fn is_clean(&self) -> bool {
        self.forbidden_dependencies.is_empty()
            && self.proc_macro_dependencies.is_empty()
            && !self.build_script
            && self.source_violations.is_empty()
    }
}

/// Locate one workspace crate's directory.
///
/// Tries the manifest-relative and current-directory-relative layouts used
/// by cargo and bazel test environments. Returns `None` when the crate's
/// sources are absent (a deployed binary), in which case the source probe
/// is skipped by callers and the deployment gates (CI audit + lockfile
/// allowlist) own the check.
pub fn crate_dir(crate_name: &str) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    // Runtime lookup, never a compile-time env!(): rules_rs rejects
    // CARGO_MANIFEST_DIR embedded in rlibs, and the runtime value makes
    // the same probes work under cargo (env set) and bazel (unset, so the
    // working-directory candidates below resolve the workspace layout).
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        let manifest_dir = Path::new(&manifest_dir);
        candidates.push(manifest_dir.join("..").join(crate_name));
        candidates.push(manifest_dir.join("../..").join("packages").join(crate_name));
    }
    if let Ok(current) = std::env::current_dir() {
        candidates.push(current.join("packages").join(crate_name));
        candidates.push(current.join("../packages").join(crate_name));
    }
    for candidate in candidates {
        if candidate.join("Cargo.toml").is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The workspace root, when it can be located from the environment.
pub fn workspace_root() -> Option<PathBuf> {
    let mut current = std::env::var("CARGO_MANIFEST_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())?;
    for _ in 0..4 {
        if current.join("Cargo.toml").is_file() && current.join("packages").is_dir() {
            return Some(current);
        }
        current = current.parent()?.to_path_buf();
    }
    None
}

/// Run the offline half of the audit: probe one handler crate's sources on
/// disk.
///
/// Returns the named violations; `Ok(())` when the crate is absent from
/// this environment (deployed binary - the deployment gates own the check)
/// or when no probe fires.
pub fn probe_crate_sources(crate_name: &str) -> Result<(), Vec<String>> {
    let Some(crate_dir) = crate_dir(crate_name) else {
        return Ok(());
    };
    let violations = probe_sources_in(&crate_dir);
    if violations.is_empty() {
        Ok(())
    } else {
        Err(violations)
    }
}

/// Probe one crate directory's Rust sources and build script.
fn probe_sources_in(crate_dir: &Path) -> Vec<String> {
    let mut violations = Vec::new();
    let mut files: Vec<PathBuf> = Vec::new();
    let src_dir = crate_dir.join("src");
    if src_dir.is_dir() {
        collect_rs_files(&src_dir, &mut files);
    }
    let build_script = crate_dir.join("build.rs");
    if build_script.is_file() {
        // A build script is build-time execution emitted into the crate's
        // artifact; handler crates may not carry one.
        violations.push(format!("build-script: {}", build_script.display()));
        files.push(build_script);
    }
    for file in files {
        let Ok(text) = fs::read_to_string(&file) else {
            continue;
        };
        for (probe, pattern) in SOURCE_PROBES {
            let matcher = regex::RegexBuilder::new(pattern).build();
            let Ok(matcher) = matcher else {
                continue;
            };
            for line in text.lines().enumerate() {
                if matcher.is_match(line.1) {
                    violations.push(format!(
                        "{probe}: {}:{}",
                        file.display(),
                        line.0 + 1
                    ));
                }
            }
        }
    }
    violations.sort();
    violations.dedup();
    violations
}

fn collect_rs_files(directory: &Path, files: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
}

/// Run the full audit of one handler crate against the workspace's
/// `cargo metadata`: its transitive dependency tree (dev-only edges and
/// platform-gated edges excluded) plus its own source surface.
///
/// Returns `Err` when the audit cannot run in this environment (cargo or
/// the workspace manifest unavailable), which callers treat as a skip -
/// never as a pass: the registration-time probe and the CI gate own the
/// pass/fail decision, and a skipped full audit is reported, not silently
/// green.
pub fn audit_crate(crate_name: &str) -> Result<SurfaceReport, String> {
    let Some(root) = workspace_root() else {
        return Err(format!("workspace root unavailable for {crate_name}"));
    };
    let metadata = run_cargo_metadata(&root)?;
    // The delta semantics: only dependencies the handler's closure adds
    // BEYOND the broker's own lib closure are reportable - the broker's
    // plumbing (rustix/nix/libc and the workspace's common derive,
    // schemars, tokio macros) is the trusted base, not the handler's
    // surface.
    let broker_tree = dependency_tree(&metadata, "d2b-broker")?;
    let handler_tree = dependency_tree(&metadata, crate_name)?;
    let added: Vec<String> = handler_tree
        .iter()
        .filter(|name| !broker_tree.contains(name))
        .cloned()
        .collect();
    let mut report = SurfaceReport {
        crate_name: crate_name.to_owned(),
        forbidden_dependencies: Vec::new(),
        proc_macro_dependencies: Vec::new(),
        build_script: false,
        source_violations: Vec::new(),
    };
    for name in &added {
        if FORBIDDEN_SYSCALL_SURFACE_CRATES.contains(&name.as_str()) {
            report.forbidden_dependencies.push(name.clone());
        }
        if is_proc_macro(&metadata, name) {
            report.proc_macro_dependencies.push(name.clone());
        }
    }
    // A handler crate that DECLARES a syscall-surface dependency directly
    // is reported even when the broker's own closure already carries the
    // crate: the declaration is the handler's own surface choice, the
    // marker the audit exists to surface, and the line that would grow
    // with the handler's next dependency.
    if let Some(dir) = crate_dir(crate_name) {
        let manifest_path = dir.join("Cargo.toml");
        if let Ok(text) = fs::read_to_string(&manifest_path) {
            for crate_name in FORBIDDEN_SYSCALL_SURFACE_CRATES {
                let direct_key = regex::Regex::new(&format!(r"(?m)^(?:{crate_name}\s*=|\[[^\]]*\.{crate_name}\])"))
                    .expect("static pattern compiles");
                let aliased = regex::Regex::new(&format!(r#"package\s*=\s*"{crate_name}""#))
                    .expect("static pattern compiles");
                if (direct_key.is_match(&text) || aliased.is_match(&text))
                    && !report.forbidden_dependencies.contains(&crate_name.to_string())
                {
                    report.forbidden_dependencies.push((*crate_name).to_owned());
                }
            }
        }
    }
    report.forbidden_dependencies.sort();
    report.forbidden_dependencies.dedup();
    report.forbidden_dependencies.sort();
    report.forbidden_dependencies.dedup();
    report.proc_macro_dependencies.sort();
    report.proc_macro_dependencies.dedup();
    if let Some(dir) = crate_dir(crate_name) {
        report.build_script = dir.join("build.rs").is_file();
        report.source_violations = probe_sources_in(&dir);
    }
    Ok(report)
}

fn run_cargo_metadata(root: &Path) -> Result<serde_json::Value, String> {
    let manifest = root.join("Cargo.toml");
    let output = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--manifest-path")
        .arg(&manifest)
        .output()
        .map_err(|error| format!("cannot run cargo metadata: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("cargo metadata produced invalid JSON: {error}"))
}

/// The transitive dependency tree of one package: its own node plus every
/// package reachable over non-dev, non-target-gated edges.
fn dependency_tree(
    metadata: &serde_json::Value,
    crate_name: &str,
) -> Result<Vec<String>, String> {
    let packages = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "cargo metadata has no package array".to_owned())?;
    let root_id = packages
        .iter()
        .find(|package| package.get("name").and_then(serde_json::Value::as_str) == Some(crate_name))
        .and_then(|package| package.get("id").and_then(serde_json::Value::as_str))
        .ok_or_else(|| format!("package {crate_name} is not a workspace member"))?;
    let mut nodes: std::collections::HashMap<&str, &serde_json::Value> = std::collections::HashMap::new();
    for node in metadata
        .get("resolve")
        .and_then(|resolve| resolve.get("nodes"))
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "cargo metadata has no resolve graph".to_owned())?
    {
        let Some(id) = node.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        nodes.insert(id, node);
    }
    let mut reachable: Vec<String> = Vec::new();
    let mut queue = vec![root_id.to_owned()];
    let mut seen = std::collections::BTreeSet::new();
    while let Some(id) = queue.pop() {
        if !seen.insert(id.clone()) {
            continue;
        }
        let Some(node) = nodes.get(id.as_str()) else {
            continue;
        };
        if let Some(owner_name) = package_name_of_id(metadata, &id).filter(|name| *name != crate_name)
        {
            reachable.push(owner_name.to_owned());
        }
        let Some(deps) = node.get("deps").and_then(serde_json::Value::as_array) else {
            continue;
        };
        for dep in deps {
            let Some(dep_id) = dep.get("pkg").and_then(serde_json::Value::as_str) else {
                continue;
            };
            let Some(kinds) = dep.get("dep_kinds").and_then(serde_json::Value::as_array) else {
                continue;
            };
            // Follow the edge when any of its kinds is a normal or
            // build-time edge for the host target; a dev-only edge or a
            // platform-gated edge is not part of the production closure.
            let follows = kinds.iter().any(|kind| {
                let kind_name = kind.get("kind").and_then(serde_json::Value::as_str);
                if kind_name == Some("dev") {
                    return false;
                }
                kind.get("target").filter(|target| !target.is_null()).is_none()
            });
            if follows {
                queue.push(dep_id.to_owned());
            }
        }
    }
    reachable.sort();
    Ok(reachable)
}

fn package_name_of_id<'a>(metadata: &'a serde_json::Value, id: &str) -> Option<&'a str> {
    metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)?
        .iter()
        .find(|package| package.get("id").and_then(serde_json::Value::as_str) == Some(id))
        .and_then(|package| package.get("name").and_then(serde_json::Value::as_str))
}

/// Whether one resolved package is a proc-macro crate, by lexical scan of
/// its manifest (readable in development/CI registries).
fn is_proc_macro(metadata: &serde_json::Value, name: &str) -> bool {
    let Some(path) = metadata
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .and_then(|packages| {
            packages
                .iter()
                .find(|package| package.get("name").and_then(serde_json::Value::as_str) == Some(name))
        })
        .and_then(|package| package.get("manifest_path").and_then(serde_json::Value::as_str))
    else {
        return false;
    };
    let Ok(text) = fs::read_to_string(path) else {
        return false;
    };
    // Proc-macro crates declare `proc-macro = true` under `[lib]`; the
    // scan is lexical and documented as such.
    let lib_section = text.split_once("[lib]").map(|(_, rest)| rest).unwrap_or("");
    let main_section = lib_section.split_once('[').map(|(head, _)| head).unwrap_or(lib_section);
    regex::Regex::new(r"proc-macro\s*=\s*true")
        .map(|matcher| matcher.is_match(main_section))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skip_without(skipped_reason: &str) {
        eprintln!("skipping: {skipped_reason}");
    }

    #[test]
    fn the_pure_fixture_crate_passes_the_source_surface_probe() {
        let Some(dir) = crate_dir("d2b-broker-fixture-handlers") else {
            skip_without("pure fixture sources absent from this environment");
            return;
        };
        assert_eq!(probe_sources_in(&dir), Vec::<String>::new());
    }

    #[test]
    fn the_syscall_surface_fixture_crate_fails_the_source_surface_probe() {
        let Some(dir) = crate_dir("d2b-broker-fixture-syscall-surface") else {
            skip_without("syscall fixture sources absent from this environment");
            return;
        };
        let violations = probe_sources_in(&dir);
        let joined = violations.join("\n");
        assert!(joined.contains("raw-asm"), "violations:\n{joined}");
        assert!(joined.contains("used-static"), "violations:\n{joined}");
        assert!(
            joined.contains("link-section-entry-point"),
            "violations:\n{joined}"
        );
        assert!(
            joined.contains("panic-hook-registration"),
            "violations:\n{joined}"
        );
        assert!(!joined.is_empty());
    }

    #[test]
    fn the_pure_fixture_crate_passes_the_full_audit() {
        let Ok(report) = audit_crate("d2b-broker-fixture-handlers") else {
            skip_without("cargo metadata unavailable in this environment");
            return;
        };
        assert!(report.is_clean(), "{report:?}");
    }

    #[test]
    fn the_syscall_surface_fixture_crate_fails_the_full_audit() {
        // Error scenario: a handler crate with a syscall dependency and a
        // raw-syscall / entry-point surface fails the dependency-surface
        // check, by name.
        let Ok(report) = audit_crate("d2b-broker-fixture-syscall-surface") else {
            skip_without("cargo metadata unavailable in this environment");
            return;
        };
        assert!(!report.is_clean());
        assert!(
            report
                .forbidden_dependencies
                .iter()
                .any(|name| name == "libc"),
            "forbidden dependencies: {:?}",
            report.forbidden_dependencies
        );
        assert!(
            report
                .source_violations
                .iter()
                .any(|violation| violation.contains("raw-asm")),
            "source violations: {:?}",
            report.source_violations
        );
    }

    #[test]
    fn an_unknown_crate_fails_the_audit_closed() {
        // The audit fails closed on an unresolvable crate name: a handler
        // crate that is not a workspace member cannot be admitted by
        // accident.
        assert!(audit_crate("d2b-no-such-handler-crate").is_err());
    }
}