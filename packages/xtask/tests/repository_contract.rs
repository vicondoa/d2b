#![forbid(unsafe_code)]

use std::{env, fs, path::PathBuf};

fn repo_file(relative: &str) -> String {
    let mut candidates = Vec::new();
    if let Some(base) = env::var_os("TEST_SRCDIR").map(PathBuf::from) {
        if let Some(workspace) = env::var_os("TEST_WORKSPACE") {
            candidates.push(base.join(workspace).join(relative));
        }
        candidates.push(base.join("_main").join(relative));
    }
    if let Some(root) = env::var_os("D2B_REPO_ROOT").map(PathBuf::from) {
        candidates.push(root.join(relative));
    }
    if let Ok(current_dir) = env::current_dir() {
        candidates.push(current_dir.join(relative));
    }

    candidates
        .into_iter()
        .find_map(|path| fs::read_to_string(path).ok())
        .unwrap_or_else(|| panic!("repository file is not discoverable: {relative}"))
}

#[test]
fn pull_request_template_requires_exact_make_check_evidence() {
    let template = repo_file(".github/PULL_REQUEST_TEMPLATE.md");

    assert!(template.contains("## Summary"));
    assert!(template.contains("## Validation evidence"));
    assert!(template.contains("## Notes"));
    assert!(template.contains(
        "- [ ] **Exact `make check`:** result=`passed`; \
evidence=`<truthful workflow URL or concise local run summary>`."
    ));
    assert!(!template.contains("- [x] **Exact `make check`:**"));
}

#[test]
fn generation_uses_one_local_bazel_entrypoint() {
    let makefile = repo_file("Makefile");
    assert!(makefile.contains("D2B_MAKE_UTILITY_TARGETS := changelog-fold generate"));
    assert!(makefile.contains(
        "generate:\n\t$(BAZEL_BIN) run --config=local //packages/xtask:generate"
    ));

    let build = repo_file("packages/xtask/BUILD.bazel");
    assert!(build.contains("generated_artifact_generator("));
    assert!(build.contains("name = \"generate\""));

    let starlark = repo_file("packages/xtask/generated_artifact_check.bzl");
    assert!(starlark.contains("def generated_artifact_generator("));
    let inventory = starlark
        .split_once("GENERATED_ARTIFACT_COMMANDS = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(commands, _)| commands)
        .expect("generator command inventory is declared");

    let main = repo_file("packages/xtask/src/main.rs");
    let generator_commands = main
        .lines()
        .filter_map(|line| {
            let line = line.trim_start();
            if !line.starts_with("[command") {
                return None;
            }
            let command = line.split("if command == \"").nth(1)?;
            let command = command.split('"').next()?;
            command.starts_with("gen-").then_some(command)
        })
        .collect::<Vec<_>>();
    assert!(
        !generator_commands.is_empty(),
        "xtask generator dispatch inventory is empty"
    );
    for command in generator_commands {
        assert!(
            inventory.contains(&format!("\"{command}\",")),
            "GENERATED_ARTIFACT_COMMANDS is missing {command}"
        );
    }

    for command in build.lines().filter_map(|line| {
        line.trim()
            .strip_prefix("command = \"")
            .and_then(|value| value.strip_suffix("\","))
    }) {
        assert!(
            starlark.contains(&format!("\"{command}\"")),
            "generator inventory is missing {command}"
        );
    }
}
