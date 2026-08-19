#![forbid(unsafe_code)]

use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

fn repo_root() -> PathBuf {
    PathBuf::from(std::env::var_os("D2B_REPO_ROOT").expect("D2B_REPO_ROOT is set"))
}

fn run(program: &OsStr, args: &[&OsStr], root: &Path) {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .env("CARGO_TARGET_DIR", root.join(".scratch/cargo-lint-target"))
        .output()
        .unwrap_or_else(|error| panic!("run {}: {error}", program.to_string_lossy()));
    assert!(
        output.status.success(),
        "{} failed with {}\nstdout:\n{}\nstderr:\n{}",
        program.to_string_lossy(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
}

fn files_with_extension(root: &Path, extension: &str, recursive: bool) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory)
            .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
        {
            let entry = entry.expect("read directory entry");
            let path = entry.path();
            if path.is_dir() {
                if recursive {
                    pending.push(path);
                }
            } else if path.extension() == Some(OsStr::new(extension)) {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn workspace_lint() {
    let root = repo_root();
    let cargo = fs::canonicalize(std::env::var_os("CARGO").expect("CARGO is set"))
        .expect("resolve Cargo executable");

    run(
        cargo.as_os_str(),
        &[
            OsStr::new("fmt"),
            OsStr::new("--all"),
            OsStr::new("--check"),
        ],
        &root,
    );
    run(
        cargo.as_os_str(),
        &[
            OsStr::new("clippy"),
            OsStr::new("--locked"),
            OsStr::new("--workspace"),
            OsStr::new("--all-targets"),
            OsStr::new("--"),
            OsStr::new("-D"),
            OsStr::new("warnings"),
        ],
        &root,
    );

    let mut nix_files = files_with_extension(&root.join("nixos-modules"), "nix", true);
    nix_files.extend(files_with_extension(&root.join("tests"), "nix", true));
    nix_files.push(root.join("flake.nix"));
    for path in nix_files {
        run(
            OsStr::new("nix-instantiate"),
            &[OsStr::new("--parse"), path.as_os_str()],
            &root,
        );
    }

    let mut shell_files = Vec::new();
    for directory in ["tests", "scripts", "harness/ubuntu"] {
        let path = root.join(directory);
        if path.is_dir() {
            shell_files.extend(files_with_extension(&path, "sh", false));
        }
    }
    assert!(!shell_files.is_empty(), "no shell scripts found");
    let mut shellcheck_args = vec![OsStr::new("--severity=warning"), OsStr::new("-x")];
    shellcheck_args.extend(shell_files.iter().map(|path| path.as_os_str()));
    if Command::new("shellcheck").arg("--version").output().is_ok() {
        run(OsStr::new("shellcheck"), &shellcheck_args, &root);
    } else {
        let mut nix_args = vec![
            OsStr::new("shell"),
            OsStr::new("--quiet"),
            OsStr::new("--inputs-from"),
            root.as_os_str(),
            OsStr::new("nixpkgs#shellcheck"),
            OsStr::new("--command"),
            OsStr::new("shellcheck"),
        ];
        nix_args.extend(shellcheck_args);
        run(OsStr::new("nix"), &nix_args, &root);
    }
}
