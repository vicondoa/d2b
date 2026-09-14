//! `deadcode-check` - repeatable workspace dead-code/visibility/unused-dependency gate.
//!
//! Runs three independent scans, each in-process style with `std::process::Command`:
//!
//! 1. `cargo hawk check` - dead / overbroad public API (`pub` → `pub(crate)`).
//! 2. `cargo shear` - unused `Cargo.toml` dependencies (edition-2024 aware).
//! 3. `cargo check --workspace --all-targets` with `RUSTFLAGS="-A unused -D dead_code"`
//!    - isolates the rustc `dead_code` lint to a hard error.
//!
//! Every finding must be fixed by restructuring code (delete / reduce visibility /
//! real use / per-entry shear config), never by `#![allow]` or a tool-wide switch.

use std::path::Path;
use std::process::{Command, ExitCode};

/// Run the full dead-code gate from the workspace root.
pub fn run() -> ExitCode {
    let Ok(root) = super::repo_root() else {
        eprintln!("dead-code pass: cannot locate repo root");
        return ExitCode::FAILURE;
    };
    let mut failed = false;

    if !run_cargo_hawk(root) {
        failed = true;
    }
    if !run_cargo_shear(root) {
        failed = true;
    }
    if !run_rustc_dead_code(root) {
        failed = true;
    }

    if failed {
        eprintln!("dead-code pass: reported findings (fix by restructuring; never suppress)");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// `cargo hawk check` from the repo root. The tool is required, not optional.
fn run_cargo_hawk(root: &Path) -> bool {
    if which("cargo-hawk").is_none() {
        eprintln!("dead-code pass: cargo-hawk reported findings");
        eprintln!("  install: cargo install cargo-hawk");
        return false;
    }
    run_command(
        "cargo-hawk",
        &["check"],
        root,
        "cargo hawk check",
    )
}

/// `cargo shear` from the repo root. The tool is required, not optional.
fn run_cargo_shear(root: &Path) -> bool {
    if which("cargo-shear").is_none() {
        eprintln!("dead-code pass: cargo-shear reported findings");
        eprintln!("  install: cargo install cargo-shear");
        return false;
    }
    run_command("cargo-shear", &[], root, "cargo shear")
}

/// rustc `dead_code` pass: workspace all-targets check with dead_code as a hard
/// error and `unused` explicitly allowed, so this invocation reports only
/// dead-code findings (unused deps are cargo-shear's job).
fn run_rustc_dead_code(root: &Path) -> bool {
    let mut command = Command::new("cargo");
    command
        .arg("check")
        .arg("--workspace")
        .arg("--all-targets")
        .current_dir(root)
        .env("RUSTFLAGS", "-A unused -D dead_code");
    run_command_handle(command, "rustc dead_code")
}

fn run_command(program: &str, args: &[&str], root: &Path, label: &str) -> bool {
    let mut command = Command::new(program);
    command.args(args).current_dir(root);
    run_command_handle(command, label)
}

fn run_command_handle(mut command: Command, label: &str) -> bool {
    let output = match command.output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("dead-code pass: could not run {label}: {error}");
            return false;
        }
    };
    if output.status.success() {
        true
    } else {
        eprintln!("{}", String::from_utf8_lossy(&output.stdout));
        eprintln!("{}", String::from_utf8_lossy(&output.stderr));
        eprintln!("dead-code pass: {label} reported findings");
        false
    }
}

/// Minimum PATH lookup for the required binaries.
fn which(program: &str) -> Option<std::path::PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}
