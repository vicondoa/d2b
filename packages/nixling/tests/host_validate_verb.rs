//! CLI-contract integration test for `nixling host validate`.
//!
//! Migrated from `tests/host-validate-verb-eval.sh`: these cases drive the
//! real CLI binary, redirect the validator/evidence directories into a
//! per-test scratch tree, and assert the same readiness-wave, evidence-write,
//! and exit-code contract as the retired shell gate.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;

struct Sandbox {
    _tmp: tempfile::TempDir,
    scripts_full: PathBuf,
    scripts_empty: PathBuf,
    evidence: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let tmp = target_tempdir();
        let scripts_full = tmp.path().join("scripts-full");
        let scripts_empty = tmp.path().join("scripts-empty");
        let evidence = tmp.path().join("evidence");
        fs::create_dir_all(&scripts_full).expect("mk scripts-full");
        fs::create_dir_all(&scripts_empty).expect("mk scripts-empty");
        fs::create_dir_all(&evidence).expect("mk evidence");

        let staged = stage_catalog_validators(&scripts_full);
        assert!(
            !staged.is_empty(),
            "validator fixture extraction must find WAVE_CATALOG script basenames"
        );

        Self {
            _tmp: tmp,
            scripts_full,
            scripts_empty,
            evidence,
        }
    }

    fn reset_evidence(&self) {
        if self.evidence.exists() {
            fs::remove_dir_all(&self.evidence).expect("clear evidence dir");
        }
        fs::create_dir_all(&self.evidence).expect("recreate evidence dir");
    }

    fn run(&self, scripts_dir: &Path, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_nixling"))
            .args(args)
            .env("NIXLING_VALIDATE_SCRIPTS_DIR", scripts_dir)
            .env("NIXLING_VALIDATE_EVIDENCE_DIR", &self.evidence)
            .output()
            .unwrap_or_else(|err| panic!("spawn nixling {}: {err}", args.join(" ")))
    }
}

fn target_tempdir() -> tempfile::TempDir {
    let base = std::env::var_os("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("tmp")
                .join("host-validate-verb")
        });
    fs::create_dir_all(&base).expect("mk cargo target temp base");
    tempfile::Builder::new()
        .prefix("host-validate-")
        .tempdir_in(base)
        .expect("tempdir in cargo target")
}

fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .expect("packages/nixling is two levels below the repository root")
        .to_path_buf()
}

fn wave_catalog_section() -> String {
    let path = repo_root().join("packages/nixling/src/host_validate.rs");
    let source =
        fs::read_to_string(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
    let start = source
        .find("pub const WAVE_CATALOG")
        .expect("WAVE_CATALOG declaration is present");
    let tail = &source[start..];
    let end = tail
        .find("];")
        .expect("WAVE_CATALOG declaration is terminated");
    tail[..end].to_owned()
}

fn catalog_waves() -> Vec<String> {
    wave_catalog_section()
        .lines()
        .filter_map(|line| {
            let rest = line.trim_start().strip_prefix("wave: \"")?;
            Some(
                rest.split('"')
                    .next()
                    .expect("wave string close")
                    .to_owned(),
            )
        })
        .collect()
}

fn options_readiness_waves() -> Vec<String> {
    let path = repo_root().join("nixos-modules/options-daemon.nix");
    fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read {}: {err}", path.display()))
        .lines()
        .filter_map(|line| {
            let line = line.trim_end();
            let rest = line.strip_prefix("    ")?;
            let name = rest.strip_suffix(" = {")?;
            if name
                .chars()
                .next()
                .is_some_and(|first| first.is_ascii_alphabetic())
                && name.chars().all(|c| c.is_ascii_alphanumeric())
            {
                Some(name.to_owned())
            } else {
                None
            }
        })
        .collect()
}

fn catalog_validator_scripts() -> Vec<String> {
    let mut scripts = BTreeSet::new();
    for line in wave_catalog_section().lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix('"') else {
            continue;
        };
        let name = rest.split('"').next().expect("validator string close");
        if name.ends_with(".sh")
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            scripts.insert(name.to_owned());
        }
    }
    scripts.into_iter().collect()
}

fn stage_catalog_validators(dir: &Path) -> Vec<String> {
    let scripts = catalog_validator_scripts();
    for name in &scripts {
        fs::write(dir.join(name), "").unwrap_or_else(|err| panic!("stage {name}: {err}"));
    }
    scripts
}

fn stdout_json(out: &Output) -> Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "stdout was not valid JSON: {err}\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn wave<'a>(envelope: &'a Value, name: &str) -> &'a Value {
    envelope["waves"]
        .as_array()
        .expect("waves[] is an array")
        .iter()
        .find(|entry| entry["wave"] == name)
        .unwrap_or_else(|| panic!("wave {name:?} missing from envelope:\n{envelope:#}"))
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("read evidence dir {}: {err}", dir.display()))
        .map(|entry| entry.expect("evidence dir entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

#[test]
fn catalog_waves_match_options_daemon_readiness_specs() {
    let catalog = catalog_waves();
    let options = options_readiness_waves();

    assert!(
        !catalog.is_empty(),
        "failed to extract WAVE_CATALOG entries from host_validate.rs"
    );
    assert!(
        !options.is_empty(),
        "failed to extract readinessWaveSpecs from options-daemon.nix"
    );
    assert_eq!(
        catalog, options,
        "WAVE_CATALOG order + names must match readinessWaveSpecs"
    );
}

#[test]
fn host_validate_without_apply_or_dry_run_exits_78_usage_envelope() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(&sandbox.scripts_full, &["host", "validate", "--json"]);

    assert_eq!(
        out.status.code(),
        Some(78),
        "missing mode should exit 78; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope = stdout_json(&out);
    assert_eq!(envelope["code"], "--apply-or-dry-run-required");
    assert_eq!(envelope["exitCode"], 78);
}

#[test]
fn host_validate_dry_run_reports_catalog_waves_and_writes_no_evidence() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(
        &sandbox.scripts_full,
        &["host", "validate", "--dry-run", "--json"],
    );

    assert_eq!(
        out.status.code(),
        Some(0),
        "dry-run should exit 0; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope = stdout_json(&out);
    assert_eq!(envelope["command"], "host validate");
    assert_eq!(envelope["mode"], "dry-run");
    assert_eq!(
        envelope["waves"].as_array().expect("waves[]").len(),
        catalog_waves().len(),
        "dry-run should report every catalog wave"
    );

    let p1 = wave(&envelope, "p1");
    assert_eq!(p1["status"], "ready");
    let validators = p1["validators"].as_array().expect("p1 validators[]");
    assert!(
        !validators.is_empty(),
        "p1 should report its per-validator presence map"
    );
    assert!(
        validators.iter().all(|validator| {
            validator["name"]
                .as_str()
                .is_some_and(|name| !name.is_empty())
                && validator["present"] == Value::Bool(true)
        }),
        "all staged p1 validators should be present; got:\n{p1:#}"
    );
    assert!(
        json_files(&sandbox.evidence).is_empty(),
        "dry-run must not write evidence files"
    );
}

#[test]
fn host_validate_apply_wave_p1_writes_canonical_evidence_only_for_p1() {
    let sandbox = Sandbox::new();
    sandbox.reset_evidence();
    let out = sandbox.run(
        &sandbox.scripts_full,
        &["host", "validate", "--apply", "--wave", "p1", "--json"],
    );

    assert_eq!(
        out.status.code(),
        Some(0),
        "--apply --wave p1 should exit 0; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope = stdout_json(&out);
    assert_eq!(envelope["mode"], "apply");
    assert_eq!(wave(&envelope, "p1")["status"], "attested");

    let evidence_path = sandbox.evidence.join("p1.json");
    assert!(
        evidence_path.is_file(),
        "p1 evidence file should exist at {}",
        evidence_path.display()
    );
    let evidence: Value = serde_json::from_slice(
        &fs::read(&evidence_path).unwrap_or_else(|err| panic!("read p1 evidence: {err}")),
    )
    .expect("p1 evidence is JSON");
    assert_eq!(evidence["wave"], "p1");
    assert!(
        evidence["timestamp"]
            .as_str()
            .is_some_and(|timestamp| timestamp.len() >= 5
                && timestamp[..4].chars().all(|c| c.is_ascii_digit())
                && timestamp.as_bytes()[4] == b'-'),
        "timestamp should start with an ISO-8601 year, got {}",
        evidence["timestamp"]
    );
    assert!(
        evidence["operatorSignature"]
            .as_str()
            .is_some_and(|sig| sig.starts_with("sha256:") && sig.len() > "sha256:".len()),
        "operatorSignature should carry the sha256: shape, got {}",
        evidence["operatorSignature"]
    );

    let files = json_files(&sandbox.evidence);
    assert_eq!(
        files,
        vec![evidence_path],
        "--wave p1 must constrain evidence writes to p1.json"
    );
}

#[test]
fn host_validate_apply_missing_validators_exits_78_and_writes_no_evidence() {
    let sandbox = Sandbox::new();
    sandbox.reset_evidence();
    let out = sandbox.run(
        &sandbox.scripts_empty,
        &["host", "validate", "--apply", "--wave", "p1", "--json"],
    );

    assert_eq!(
        out.status.code(),
        Some(78),
        "missing validators should exit 78; stdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope = stdout_json(&out);
    let p1 = wave(&envelope, "p1");
    assert_eq!(p1["status"], "missing");
    assert!(
        p1["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("evidence NOT written")),
        "missing-validator detail should explain that evidence was not written; got:\n{p1:#}"
    );
    assert!(
        !sandbox.evidence.join("p1.json").exists(),
        "missing validators must not write p1 evidence"
    );
}

#[test]
fn host_validate_apply_unknown_wave_exits_78_envelope() {
    let sandbox = Sandbox::new();
    let out = sandbox.run(
        &sandbox.scripts_full,
        &[
            "host",
            "validate",
            "--apply",
            "--wave",
            "bogus-wave",
            "--json",
        ],
    );

    assert_eq!(
        out.status.code(),
        Some(78),
        "unknown wave should exit 78; stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let envelope = stdout_json(&out);
    assert_eq!(envelope["code"], "unknown-wave");
    assert_eq!(envelope["exitCode"], 78);
    assert!(
        envelope["observedState"]
            .as_str()
            .is_some_and(|state| state.contains("--wave bogus-wave")),
        "unknown-wave envelope should name the rejected wave; got:\n{envelope:#}"
    );
}
