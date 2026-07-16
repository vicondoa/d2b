//! Policy/source/doc cross-reference lints (the "H-group"), migrated from the
//! `tests/*-eval.sh` bash gates. Each test reads the real repo files (via the
//! `d2b_contract_tests::read_repo_file` helper) and asserts a structural or
//! documentation invariant. This crate runs only from
//! `tests/tools/rust-workspace-checks.sh` against the real checkout (it is excluded
//! from the hermetic Nix sandbox workspace build), so repo-file access is
//! sound.

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use regex::Regex;
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};
use tree_sitter::{Node, Parser};

/// Assert `haystack` contains a line matching `pattern` (multi-line, `^`/`$`
/// anchor lines), with a descriptive failure message.
fn assert_line_matches(haystack: &str, pattern: &str, ctx: &str) {
    let re = Regex::new(pattern).expect("valid regex");
    assert!(re.is_match(haystack), "{ctx}: no line matched /{pattern}/");
}

fn collect_markdown_files(dir: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = std::fs::read_dir(dir)
        .unwrap_or_else(|err| panic!("failed to read {}: {err}", dir.display()))
        .map(|entry| entry.expect("valid directory entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_markdown_files(&path, files);
        } else if path.extension().is_some_and(|extension| extension == "md") {
            files.push(path);
        }
    }
}

fn node_text<'source>(node: Node<'_>, source: &'source str) -> Result<&'source str, String> {
    node.utf8_text(source.as_bytes())
        .map_err(|_| "bash AST node is not valid UTF-8".into())
}

fn named_children<'tree>(node: Node<'tree>) -> Vec<Node<'tree>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}

fn contains_shell_redirection(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "file_redirect" | "heredoc_redirect" | "herestring_redirect" | "redirected_statement"
    ) || named_children(node)
        .into_iter()
        .any(contains_shell_redirection)
}

fn exact_word(node: Node<'_>, expected: &str, source: &str) -> Result<(), String> {
    if node.kind() != "word"
        || node.named_child_count() != 0
        || node_text(node, source)? != expected
    {
        return Err(format!(
            "delivery wrapper expected word {expected:?}, found {} {:?}",
            node.kind(),
            node_text(node, source)?
        ));
    }
    Ok(())
}

fn exact_xtask_target(node: Node<'_>, source: &str) -> Result<(), String> {
    if node.kind() != "string" {
        return Err(format!(
            "delivery wrapper target must be one double-quoted string node, found {}",
            node.kind()
        ));
    }
    let children = named_children(node);
    if children.len() != 2
        || children[0].kind() != "simple_expansion"
        || children[1].kind() != "string_content"
        || node_text(children[0], source)? != "$out"
        || node_text(children[1], source)? != "/bin/xtask"
    {
        return Err(format!(
            "delivery wrapper target must be a simple $out expansion followed by literal /bin/xtask, found {:?}",
            node_text(node, source)?
        ));
    }
    let expansion_children = named_children(children[0]);
    if expansion_children.len() != 1
        || expansion_children[0].kind() != "variable_name"
        || node_text(expansion_children[0], source)? != "out"
    {
        return Err("delivery wrapper target expansion must name only $out".into());
    }
    Ok(())
}

fn exact_wrapper_command(post_fixup: &str, canonical_path: &str) -> Result<(), String> {
    let mut parser = Parser::new();
    parser
        .set_language(&tree_sitter_bash::LANGUAGE.into())
        .map_err(|err| format!("initialize bash AST parser: {err}"))?;
    let tree = parser
        .parse(post_fixup, None)
        .ok_or("bash AST parser returned no syntax tree")?;
    let root = tree.root_node();
    if root.has_error() {
        return Err("delivery postFixup is not valid bash syntax".into());
    }
    let statements = named_children(root)
        .into_iter()
        .filter(|node| node.kind() != "comment")
        .collect::<Vec<_>>();
    if statements.iter().copied().any(contains_shell_redirection) {
        return Err("delivery postFixup redirections and heredocs are forbidden".into());
    }
    if statements.len() != 1 {
        return Err(format!(
            "delivery postFixup must contain exactly one top-level command node; found {}",
            statements.len()
        ));
    }
    let command = statements[0];
    if command.kind() != "command" {
        return Err(format!(
            "delivery postFixup must contain one unconditional command, not {}",
            command.kind()
        ));
    }

    let name = command
        .child_by_field_name("name")
        .ok_or("delivery wrapper command is missing its name")?;
    let name_children = named_children(name);
    if name.kind() != "command_name"
        || name_children.len() != 1
        || exact_word(name_children[0], "wrapProgram", post_fixup).is_err()
    {
        return Err("delivery wrapper command name must be the plain word wrapProgram".into());
    }
    let mut cursor = command.walk();
    let arguments = command
        .children_by_field_name("argument", &mut cursor)
        .collect::<Vec<_>>();
    if arguments.len() != 5 || command.named_child_count() != 6 {
        return Err(format!(
            "delivery wrapper must have exactly five arguments and no redirects or assignments; found {} arguments",
            arguments.len()
        ));
    }
    exact_xtask_target(arguments[0], post_fixup)?;
    exact_word(arguments[1], "--prefix", post_fixup)?;
    exact_word(arguments[2], "PATH", post_fixup)?;
    exact_word(arguments[3], ":", post_fixup)?;
    exact_word(arguments[4], canonical_path, post_fixup)?;
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluatedDeliveryRuntimeTool {
    name: String,
    bin_path: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct EvaluatedDeliveryRuntime {
    post_fixup: String,
    delivery_runtime_tools: Vec<EvaluatedDeliveryRuntimeTool>,
}

const REQUIRED_DELIVERY_RUNTIME_TOOLS: [&str; 5] =
    ["git", "openssl", "shellcheck", "gh", "git-town"];

fn required_delivery_runtime_paths(
    post_fixup: &str,
    tools: &[EvaluatedDeliveryRuntimeTool],
) -> Result<Vec<String>, String> {
    let names = tools
        .iter()
        .map(|tool| tool.name.as_str())
        .collect::<Vec<_>>();
    if names != REQUIRED_DELIVERY_RUNTIME_TOOLS {
        return Err(format!(
            "evaluated delivery runtime tools differ from the required set: {names:?}"
        ));
    }
    let expected_paths = tools
        .iter()
        .map(|tool| tool.bin_path.clone())
        .collect::<Vec<_>>();
    if expected_paths
        .iter()
        .any(|path| !path.starts_with("/nix/store/") || !path.ends_with("/bin"))
    {
        return Err(format!(
            "evaluated delivery runtime tool paths must be Nix store bin paths: {expected_paths:?}"
        ));
    }
    let expected_path = expected_paths.join(":");
    exact_wrapper_command(post_fixup, &expected_path)?;
    Ok(expected_paths)
}

fn evaluated_delivery_runtime_contracts() -> BTreeMap<String, EvaluatedDeliveryRuntime> {
    let root = repo_root();
    let flake_ref = format!("git+file://{}", root.display());
    let flake_ref = serde_json::to_string(&flake_ref).expect("serialize flake reference");
    let expression = format!(
        r#"
          let
            flake = builtins.getFlake {flake_ref};
          in
          builtins.listToAttrs (map (system:
            let package = flake.packages.${{system}}.d2b-delivery;
            in {{
              name = system;
              value = {{
                inherit (package) postFixup deliveryRuntimeTools;
              }};
            }}
          ) flake.lib.supportedSystems)
        "#
    );
    let output = Command::new("nix")
        .args([
            "eval",
            "--impure",
            "--quiet",
            "--no-warn-dirty",
            "--no-write-lock-file",
            "--json",
            "--expr",
            &expression,
        ])
        .current_dir(root)
        .output()
        .expect("run nix eval for delivery runtime contracts");
    assert!(
        output.status.success(),
        "all-system delivery runtime eval failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse all-system delivery runtime eval")
}

// Migrated from tests/daemon-experimental-warning-eval.sh.
#[test]
fn daemon_experimental_option_documents_default_with_migration() {
    let opts = read_repo_file("nixos-modules/options-daemon.nix");
    assert!(
        opts.contains("consumers should leave it at its default"),
        "options-daemon.nix must document daemonExperimental as a leave-at-default option"
    );
    let guide = read_repo_file("docs/how-to/migrate-d2b-v1-0-to-v1-1.md");
    assert!(
        guide.contains("Remove `d2b.daemonExperimental.enable`"),
        "the v1.0->v1.1 migration guide must instruct removing d2b.daemonExperimental.enable"
    );
}

// Migrated from tests/v1.1-kernel-floor-eval.sh.
#[test]
fn v1_1_kernel_floor_declared_in_adr_and_migration_guide() {
    let adr = read_repo_file("docs/adr/0008-supported-platforms-and-rejected-targets.md");
    let adr_floor = Regex::new(r"(>=\s*6\.9|≥\s*6\.9|6\.9\+|kernel-floor uplift)").unwrap();
    assert!(
        adr_floor.is_match(&adr),
        "ADR 0008 must declare the v1.1 >=6.9 kernel floor"
    );
    let guide = read_repo_file("docs/how-to/migrate-d2b-v1-0-to-v1-1.md");
    let guide_floor = Regex::new(r"kernel\s*≥?\s*6\.9|kernel\s*>=\s*6\.9").unwrap();
    assert!(
        guide_floor.is_match(&guide),
        "the v1.0->v1.1 migration guide must mention the v1.1 kernel-floor prerequisite"
    );
}

// Migrated from tests/adr-0015-presence-eval.sh.
#[test]
fn adr_0015_superseded_with_historical_invariants() {
    for f in [
        "docs/adr/0015-daemon-only-clean-break.md",
        "docs/adr/0045-provider-and-transport-framework.md",
        "AGENTS.md",
        "docs/adr/README.md",
    ] {
        assert!(repo_path_exists(f), "missing {f}");
    }
    let adr = read_repo_file("docs/adr/0015-daemon-only-clean-break.md");
    assert_line_matches(&adr, r"(?m)^# 0015\. ", "ADR 0015 title");
    assert_line_matches(
        &adr,
        r"(?m)^- Status: Superseded by \[ADR 0045\]\(0045-provider-and-transport-framework\.md\)$",
        "ADR 0015 canonical superseded Status header",
    );
    assert_line_matches(&adr, r"(?m)^- Wave: P6$", "ADR 0015 Wave header");
    assert_line_matches(
        &adr,
        r"(?m)^- Date: [0-9]{4}-[0-9]{2}-[0-9]{2}$",
        "ADR 0015 ISO Date header",
    );
    for section in [
        r"(?m)^## Context$",
        r"(?m)^## Decision$",
        r"(?m)^## Consequences$",
    ] {
        assert_line_matches(&adr, section, "ADR 0015 required section");
    }

    let adr_0045 = read_repo_file("docs/adr/0045-provider-and-transport-framework.md");
    assert_line_matches(&adr_0045, r"(?m)^# ADR 0045: ", "ADR 0045 title");
    assert_line_matches(
        &adr_0045,
        r"(?m)^- Status: Accepted$",
        "ADR 0045 accepted Status header",
    );
    assert!(
        adr_0045.contains("[ADR 0015](0015-daemon-only-clean-break.md)"),
        "accepted ADR 0045 must name ADR 0015 in its supersession list"
    );

    let agents = read_repo_file("AGENTS.md");
    assert!(
        agents.contains("0015-daemon-only-clean-break.md"),
        "AGENTS.md must cross-reference 0015-daemon-only-clean-break.md"
    );
    let adr_index = read_repo_file("docs/adr/README.md");
    assert!(
        adr_index.lines().any(|line| {
            line.contains("0015-daemon-only-clean-break.md")
                && line.contains("| Superseded |")
                && line.contains("ADR 0045")
        }),
        "ADR index must mark ADR 0015 as superseded by ADR 0045"
    );
}

#[test]
fn adr_0045_accepted_with_realm_and_delivery_contracts() {
    for file in [
        "docs/adr/0045-provider-and-transport-framework.md",
        "AGENTS.md",
        "tests/AGENTS.md",
        "tests/README.md",
        "docs/how-to/adding-a-test.md",
        "docs/adr/README.md",
    ] {
        assert!(repo_path_exists(file), "missing {file}");
    }

    let adr = read_repo_file("docs/adr/0045-provider-and-transport-framework.md");
    assert_line_matches(&adr, r"(?m)^# ADR 0045: ", "ADR 0045 title");
    assert_line_matches(&adr, r"(?m)^- Status: Accepted$", "ADR 0045 Status header");
    assert_line_matches(
        &adr,
        r"(?m)^- Date: [0-9]{4}-[0-9]{2}-[0-9]{2}$",
        "ADR 0045 ISO Date header",
    );
    for section in [
        r"(?m)^## Context$",
        r"(?m)^## Decision summary$",
        r"(?m)^## Realm process and authority model$",
        r"(?m)^## Normative precedence$",
    ] {
        assert_line_matches(&adr, section, "ADR 0045 required section");
    }
    for required in [
        "parent-spawns each child controller",
        "and broker as separate pidfd-supervised processes",
        "Child processes are not PID1 units.",
        "Delivery uses Git Town ordinary PR stacks, Rust `xtask`, immutable tree snapshots",
        "validation and panel lanes",
    ] {
        assert!(
            adr.contains(required),
            "ADR 0045 is missing required contract text: {required}"
        );
    }

    let agents = read_repo_file("AGENTS.md");
    for required in [
        "## Realm-local control-plane end state",
        "parent-spawned",
        "pidfd-supervised",
        "run concurrently against that",
        "they never run tests, builds, evals",
    ] {
        assert!(
            agents.contains(required),
            "AGENTS.md is missing accepted ADR 0045 policy: {required}"
        );
    }

    let test_agents = read_repo_file("tests/AGENTS.md");
    assert!(
        test_agents.contains("the ten-role panel") && test_agents.contains("proceed\nconcurrently"),
        "tests/AGENTS.md must require concurrent final lanes"
    );
    assert!(
        test_agents.contains("Reviewers") && test_agents.contains("never execute tests"),
        "tests/AGENTS.md must keep reviewers out of validator execution"
    );

    let test_readme = read_repo_file("tests/README.md");
    assert!(
        test_readme.contains("Open or update the ordinary PR and Git Town parent graph")
            && test_readme.contains("panel concurrently against that snapshot")
            && test_readme.contains("Do not paste raw"),
        "tests/README.md must keep PR-before-final-lanes and external-summary-only evidence"
    );
    let adding_test = read_repo_file("docs/how-to/adding-a-test.md");
    assert!(
        adding_test.contains("## Open the PR before final gates")
            && adding_test.contains("final validator lane after the PR opens")
            && adding_test.contains("Never paste raw"),
        "adding-a-test guide must keep PR-before-final-lanes and external-summary-only evidence"
    );

    let adr_index = read_repo_file("docs/adr/README.md");
    assert!(
        adr_index.lines().any(|line| {
            line.contains("0045-provider-and-transport-framework.md")
                && line.contains("| Accepted |")
        }),
        "ADR index must list ADR 0045 as Accepted"
    );
}

#[test]
fn delivery_tool_sources_and_toolchains_are_exactly_pinned() {
    let tools = read_repo_file("pkgs/delivery-tools.nix");
    for pin in [
        r#"ghVersion = "2.92.0";"#,
        r#"gitTownVersion = "23.0.1";"#,
        r#"cargoUdepsVersion = "0.1.61";"#,
        r#"cargoUdepsNightlyDate = "2025-12-01";"#,
        r#"cargoSemverChecksVersion = "0.47.0";"#,
        r#"rustStableVersion = "1.94.1";"#,
        r#"owner = "cli";"#,
        r#"repo = "cli";"#,
        r#"hash = "sha256-/7EiX4ZZPhSNgY/D5OVOako/c0ujHq05GMj3UB11bqQ=";"#,
        r#"vendorHash = "sha256-pBLRCIRjN3VoXbTFSq+R9/N3uAUCEjvPtk8LKKKS51s=";"#,
        r#"owner = "git-town";"#,
        r#"repo = "git-town";"#,
        r#"hash = "sha256-kAAzfb0rg10k9PnUKYEqdSWYWi0JR6jiKDHUv/RSUSs=";"#,
        "vendorHash = null;",
        r#"hash = "sha256-yT/EJWGGhQapbU1o1Gus1Vk5cAhso5ALTBecB3BH46g=";"#,
        r#"cargoHash = "sha256-DGfAsBucFRFJkjmJkpTpNfQO79jaNa5NezXKf7hYYeM=";"#,
        r#"hash = "sha256-1D6WFsiMOl/bJr0J+mmvLlgnRSKN6rPhDSnDsdLTC9E=";"#,
        r#"cargoHash = "sha256-YbtYIHj899eJSrp5n5jODgTkL9L26EnruzECwBrBF00=";"#,
    ] {
        assert!(
            tools.contains(pin),
            "delivery tooling is missing exact pin {pin}"
        );
    }
    assert!(
        !tools.contains("fakeHash") && !tools.contains("fakeSha256"),
        "delivery tooling must not contain placeholder hashes"
    );
    assert!(
        !tools.contains("curl") && !tools.contains("rustup"),
        "delivery tools must not download toolchains or binaries at runtime"
    );
    assert!(
        tools.matches("pkgs.buildGoModule").count() == 2
            && !tools.contains("pkgs.git-town")
            && !tools.contains("pkgs.gh"),
        "Git Town and GitHub CLI must be repository-owned source builds, not nixpkgs aliases"
    );
    let flake = read_repo_file("flake.nix");
    assert!(
        flake.contains("gh --version | grep -F 'gh version 2.92.0'")
            && flake.contains("git-town --version | grep -Fx 'Git Town 23.0.1'")
            && flake.contains("gh = deliveryTools.gh;")
            && flake.contains("git-town = deliveryTools.gitTown;"),
        "delivery packages and checks must expose all runtime verification tools"
    );

    let flake = read_repo_file("flake.nix");
    assert!(
        flake.contains(r#"inputs.nixpkgs.follows = "nixpkgs";"#)
            && flake.contains("rust-overlay.overlays.default"),
        "the locked rust-overlay input must follow nixpkgs and remain scoped to delivery tooling"
    );
    assert!(
        flake.contains("devShells = forAllSystems")
            && flake.contains("shell = pkgs.mkShell {")
            && flake.contains("pkgs.stdenv.cc")
            && flake.contains("pkgs.pkg-config")
            && flake.contains("pkgs.openssl")
            && flake.contains("pkgs.cmake")
            && flake.contains("pkgs.sccache")
            && flake.contains("cargo-udeps-nightly = deliveryTools.cargoUdepsNightly;")
            && flake.contains("cargo-semver-checks = deliveryTools.cargoSemverChecks;"),
        "supported systems must expose the pinned delivery tools and native-capable shell"
    );
    assert!(
        tools.contains("stableRustPlatform = pkgs.makeRustPlatform")
            && tools.contains("cargo = stableRust;")
            && tools.contains("rustc = stableRust;")
            && tools.contains("pkgs.lib.makeBinPath [ nightlyRust pkgs.sccache ]")
            && tools.contains("--set CARGO ${nightlyRust}/bin/cargo")
            && tools.contains("--set RUSTC ${nightlyRust}/bin/rustc"),
        "cargo-udeps must contain nightly and sccache without replacing ordinary stable cargo"
    );
    assert!(
        flake.contains(
            "deliveryRustWorkspace =\n          rustWorkspaceWith deliveryTools.stableRustPlatform;"
        ) && flake.contains(r#"output = "d2b-delivery";"#)
            && flake.contains(r#"buildKind = "deliveryWorkspace";"#)
            && flake.contains("deliveryRustWorkspace (workspaceArgs // {")
            && flake.contains("deliveryRuntimeToolSpecs = [")
            && flake.contains("pkgs.lib.makeBinPath deliveryRuntimePackages")
            && flake.contains("rustToolchainVersion = deliveryTools.rustStableVersion;")
            && flake.contains("inherit deliveryRuntimeTools;")
            && flake.contains(
                r#"outputHashes."wl-proxy-0.1.2" = "sha256-1yO1zgzSyzQ2DnDMpVxcnI5BsTNvXfzIUS+RNlPj4A8=";"#
            ),
        "d2b-delivery must use the pinned stable Rust platform and locked Cargo sources"
    );
    assert!(
        flake.contains("export CARGO_NET_OFFLINE=true")
            && flake.contains("cargo metadata \\")
            && flake.contains("find_package(OpenSSL REQUIRED)")
            && flake.contains("native-smoke/build/d2b-delivery-native-smoke"),
        "delivery tooling check must smoke metadata and native compilation without network"
    );
    assert!(
        flake.contains("overlays.default = _final: _prev: { };"),
        "developer tooling must not expand the public overlay"
    );
    let layer1_workflow = read_repo_file(".github/workflows/pr-l1-static-fast.yml");
    assert!(
        layer1_workflow
            .contains("mozilla-actions/sccache-action@9e7fa8a12102821edf02ca5dbea1acd0f89a2696")
            && layer1_workflow.contains("D2B_CI_SCCACHE: \"1\"")
            && layer1_workflow.contains(".sccache")
            && !layer1_workflow.contains("RUSTC_WRAPPER=\"\"")
            && !layer1_workflow.contains("CARGO_BUILD_RUSTC_WRAPPER=\"\"")
            && !layer1_workflow.contains("RUSTC_WRAPPER: \"\"")
            && !layer1_workflow.contains("CARGO_BUILD_RUSTC_WRAPPER: \"\""),
        "generated Layer-1 CI must install and retain sccache without wrapper-clearing overrides"
    );
    let release_workflow = read_repo_file(".github/workflows/release-host-binaries.yml");
    assert!(
        release_workflow
            .contains("mozilla-actions/sccache-action@9e7fa8a12102821edf02ca5dbea1acd0f89a2696")
            && release_workflow.contains("D2B_CI_SCCACHE: \"1\"")
            && release_workflow.contains("SCCACHE_DIR: ${{ github.workspace }}/.sccache")
            && release_workflow.contains("            .sccache")
            && !release_workflow.contains("packages/.d2b-gate-targets"),
        "release CI must install and persist sccache without restoring gate target directories"
    );
    let lock = read_repo_file("flake.lock");
    for pin in [
        r#""rev": "64c08a7ca051951c8eae34e3e3cb1e202fe36786""#,
        r#""narHash": "sha256-tpyBcxPpcQb8ukyNF7DoCwfSY3VPsxHoYwj00Cayv5o=""#,
        r#""rev": "e013376c32a8fcf07ddb6ec71739552bc118b7bd""#,
        r#""narHash": "sha256-DsSIQSRMrLOz40LrGZ03sp2RlJ9sz3wKpd8XPTOzXnw=""#,
    ] {
        assert!(
            lock.contains(pin),
            "rust-overlay lock is missing exact pin {pin}"
        );
    }

    let delivery_command = read_repo_file("packages/xtask/src/delivery/command.rs");
    assert!(
        delivery_command.contains(r#""git-town""#)
            && delivery_command.contains(r#""config".to_owned()"#)
            && delivery_command.contains(r#""get-parent".to_owned()"#)
            && delivery_command.contains(r#""api".to_owned()"#)
            && delivery_command.contains(r#""graphql".to_owned()"#)
            && !delivery_command.contains("cli_internal")
            && !delivery_command.contains("pulls/stacks"),
        "xtask must derive topology from Git Town and ordinary GitHub PR authority"
    );
    let delivery_cli = read_repo_file("packages/xtask/src/delivery/mod.rs");
    for stale in ["gh stack", "gh-stack", "private preview", "cli_internal"] {
        assert!(
            !delivery_cli.to_ascii_lowercase().contains(stale),
            "delivery machine-readable help contains stale stack integration {stale}"
        );
    }
    for mutation in [
        r#""POST".to_owned()"#,
        r#""PUT".to_owned()"#,
        r#""PATCH".to_owned()"#,
        r#""DELETE".to_owned()"#,
    ] {
        assert!(
            !delivery_command.contains(mutation),
            "xtask must not implement a GitHub stack mutation with {mutation}"
        );
    }
}

fn delivery_runtime_policy_fixture() -> &'static str {
    r#"
      # wrapProgram "$out/bin/decoy" --prefix PATH : /nix/store/decoy/bin
      # Benign comment input-redirection text must remain inert: < << <<<.
      wrapProgram \
        "$out/bin/xtask" \
          --prefix   PATH : /nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-git/bin:/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-openssl/bin:/nix/store/cccccccccccccccccccccccccccccccc-shellcheck/bin:/nix/store/dddddddddddddddddddddddddddddddd-gh/bin:/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-git-town/bin
    "#
}

fn delivery_runtime_policy_tools() -> Vec<EvaluatedDeliveryRuntimeTool> {
    [
        ("git", "/nix/store/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa-git/bin"),
        (
            "openssl",
            "/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-openssl/bin",
        ),
        (
            "shellcheck",
            "/nix/store/cccccccccccccccccccccccccccccccc-shellcheck/bin",
        ),
        ("gh", "/nix/store/dddddddddddddddddddddddddddddddd-gh/bin"),
        (
            "git-town",
            "/nix/store/eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee-git-town/bin",
        ),
    ]
    .into_iter()
    .map(|(name, bin_path)| EvaluatedDeliveryRuntimeTool {
        name: name.to_owned(),
        bin_path: bin_path.to_owned(),
    })
    .collect()
}

#[test]
fn delivery_runtime_wrapper_eval_matches_canonical_tools_on_all_systems() {
    let contracts = evaluated_delivery_runtime_contracts();
    assert_eq!(
        contracts.keys().map(String::as_str).collect::<Vec<_>>(),
        ["aarch64-linux", "x86_64-linux"],
        "delivery runtime policy must evaluate every supported flake system"
    );
    for (system, contract) in contracts {
        required_delivery_runtime_paths(&contract.post_fixup, &contract.delivery_runtime_tools)
            .unwrap_or_else(|err| panic!("{system} delivery runtime policy failed: {err}"));
    }
}

#[test]
fn delivery_runtime_shell_ast_binds_active_wrap_program_path() {
    let post_fixup = delivery_runtime_policy_fixture();
    let tools = delivery_runtime_policy_tools();

    assert_eq!(
        required_delivery_runtime_paths(post_fixup, &tools)
            .expect("evaluated formatting variant must parse"),
        tools
            .iter()
            .map(|tool| tool.bin_path.clone())
            .collect::<Vec<_>>()
    );
    let one_line = format!(
        "wrapProgram \"$out/bin/xtask\" --prefix PATH : {}",
        tools
            .iter()
            .map(|tool| tool.bin_path.as_str())
            .collect::<Vec<_>>()
            .join(":")
    );
    required_delivery_runtime_paths(&one_line, &tools)
        .expect("one-line and quoted-target formatting must parse");
    let fully_split = format!(
        "wrapProgram \\\n\"$out/bin/xtask\" \\\n--prefix \\\nPATH \\\n: \\\n{}",
        tools
            .iter()
            .map(|tool| tool.bin_path.as_str())
            .collect::<Vec<_>>()
            .join(":")
    );
    required_delivery_runtime_paths(&fully_split, &tools)
        .expect("backslash-separated argument formatting must parse");

    let missing_active_tool = post_fixup.replace(
        ":/nix/store/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb-openssl/bin",
        "",
    );
    let error = required_delivery_runtime_paths(&missing_active_tool, &tools)
        .expect_err("commented shell decoys must not mask a missing runtime tool");
    assert!(
        error.contains("expected word"),
        "missing active tool must fail specifically: {error}"
    );

    let inactive_wrapper =
        post_fixup.replacen("      wrapProgram \\", "      echo wrapProgram \\", 1);
    let error = required_delivery_runtime_paths(&inactive_wrapper, &tools)
        .expect_err("commented, quoted, and unused decoys must not replace the active wrapper");
    assert!(
        error.contains("command name"),
        "inactive wrapper must fail specifically: {error}"
    );
}

#[test]
fn delivery_runtime_shell_ast_rejects_inactive_or_wrong_wrappers() {
    let post_fixup = delivery_runtime_policy_fixture();
    let tools = delivery_runtime_policy_tools();
    let canonical_path = tools
        .iter()
        .map(|tool| tool.bin_path.as_str())
        .collect::<Vec<_>>()
        .join(":");
    let cases = [
        (
            "false branch",
            format!("if false; then\n{post_fixup}\nfi\n"),
        ),
        (
            "uncalled function",
            format!("wrapper() {{\n{post_fixup}\n}}\n"),
        ),
        (
            "wrong target",
            post_fixup.replace("$out/bin/xtask", "$out/bin/not-xtask"),
        ),
        (
            "literal target",
            post_fixup.replace("\"$out/bin/xtask\"", "'$out/bin/xtask'"),
        ),
        (
            "split quoted dollar",
            post_fixup.replace("\"$out/bin/xtask\"", "\"$\"out/bin/xtask"),
        ),
        (
            "concatenated target",
            post_fixup.replace("\"$out/bin/xtask\"", "\"$out\"/bin/xtask"),
        ),
        (
            "escaped target expansion",
            post_fixup.replace("$out/bin/xtask", "\\$out/bin/xtask"),
        ),
        (
            "backslash in double quote",
            post_fixup.replace("/bin/xtask\"", "/bin/xta\\sk\""),
        ),
        (
            "command substitution target",
            post_fixup.replace("\"$out/bin/xtask\"", "\"$(printf '$out/bin/xtask')\""),
        ),
        (
            "array",
            format!("args=(wrapProgram \"$out/bin/xtask\" --prefix PATH : {canonical_path})\n"),
        ),
        ("extra wrapper", format!("{post_fixup}\n{post_fixup}")),
    ];
    for (name, script) in cases {
        let error = required_delivery_runtime_paths(&script, &tools)
            .expect_err("only one unconditional exact xtask wrapper may pass");
        assert!(
            error.contains("top-level command")
                || error.contains("unconditional command")
                || error.contains("command name")
                || error.contains("target"),
            "{name} must fail the whole-script grammar: {error}"
        );
    }
}

#[test]
fn delivery_runtime_shell_ast_rejects_heredoc_decoys() {
    for opener in ["cat <<EOF", "cat <<'EOF'", "cat <<\"EOF\"", "cat <<-EOF"] {
        let heredoc_decoy = format!("{opener}\n{}\nEOF\n", delivery_runtime_policy_fixture());
        let error =
            required_delivery_runtime_paths(&heredoc_decoy, &delivery_runtime_policy_tools())
                .expect_err("a wrapper-looking heredoc body must not satisfy runtime PATH policy");
        assert!(
            error.contains("heredocs are forbidden"),
            "{opener} must fail closed before its body is parsed: {error}"
        );
    }
}

#[test]
fn delivery_runtime_shell_ast_rejects_redirection_decoys() {
    for redirection in [
        "cat <input",
        "cat < <EOF",
        "cat <\\\n<EOF",
        "cat <<<value",
        "cat 3<input",
        "cat >output",
        "cat 2>output",
    ] {
        let redirection_decoy = format!("{redirection}\n{}", delivery_runtime_policy_fixture());
        let error =
            required_delivery_runtime_paths(&redirection_decoy, &delivery_runtime_policy_tools())
                .expect_err("delivery postFixup redirection must fail closed");
        assert!(
            error.contains("redirection") || error.contains("valid bash syntax"),
            "{redirection:?} must fail before wrapper matching: {error}"
        );
    }

    let quoted_extra = format!("echo '<'\n{}", delivery_runtime_policy_fixture());
    let error = required_delivery_runtime_paths(&quoted_extra, &delivery_runtime_policy_tools())
        .expect_err("a quoted less-than is inert but still an extra command");
    assert!(
        error.contains("exactly one top-level") && !error.contains("redirection"),
        "quoted less-than must remain inert while the extra command fails: {error}"
    );
}

#[test]
fn non_generated_pr_workflows_cover_stacked_bases_safely() {
    for path in [
        ".github/workflows/pr-eval-shell-tests.yml",
        ".github/workflows/eval-with-entra-id.yml",
    ] {
        let workflow = read_repo_file(path);
        assert!(
            workflow.contains("  pull_request: {}"),
            "{path} must run for pull requests targeting feature branches"
        );
        assert!(
            !workflow.contains("pull_request_target"),
            "{path} must not execute untrusted code through pull_request_target"
        );
        assert!(
            workflow.contains("permissions:\n  contents: read"),
            "{path} must retain read-only workflow permissions"
        );
        assert!(
            workflow.contains("ref: ${{ github.event.pull_request.head.sha || github.sha }}")
                && workflow.contains("persist-credentials: false"),
            "{path} must explicitly check out the exact candidate SHA without credentials"
        );
        assert!(
            workflow.contains("actual_sha=$(git rev-parse HEAD)")
                && workflow.contains(r#"test "$actual_sha" = "$EXPECTED_SHA""#)
                && workflow.contains("CHECKED_TREE: ${{ steps.candidate.outputs.sha }}")
                && workflow.contains("GITHUB_STEP_SUMMARY"),
            "{path} must bind its summary to the verified actual checkout"
        );
    }

    let reference = read_repo_file("docs/reference/delivery-tooling.md");
    let how_to = read_repo_file("docs/how-to/manage-stacked-wave-prs.md");
    assert!(
        reference
            .contains("Git Town is the only stack topology, propose, and synchronization mutator")
            && reference.contains("ordinary pull-request API")
            && reference.contains("Use `$XDG_STATE_HOME/d2b/delivery`")
            && reference.contains("Git metadata is never delivery state")
            && reference.contains("must never be added\nto the reviewed tree")
            && reference.contains(r#"--state-dir "$XDG_STATE_HOME/d2b/delivery""#)
            && reference.contains("cargo xtask delivery wave validation-import")
            && reference.contains("cargo xtask delivery wave verify")
            && reference.contains("cargo xtask delivery wave eligibility")
            && reference.contains(
                "[Manage stacked wave pull requests with Git Town](../how-to/manage-stacked-wave-prs.md)"
            )
            && reference.matches(r#"--payload "$PAYLOAD""#).count() == 2
            && reference.contains("D2B_FLAKE_CHECK=delivery-tooling make test-flake"),
        "delivery reference must describe the contract, align both invocation forms, and link the procedure"
    );
    assert!(
        !reference.contains("git town set-parent")
            && !reference.contains("git town propose --stack")
            && how_to.contains("git town set-parent \"$parent\" --non-interactive")
            && how_to.contains("git town sync --stack --non-interactive --no-auto-resolve")
            && how_to.contains(
                "git town propose --stack --non-interactive --no-browser --no-auto-resolve"
            )
            && how_to.contains("git town config get-parent \"$branch\"")
            && how_to.contains("git status --porcelain=v1 --untracked-files=all")
            && how_to.contains("git show-ref --verify --quiet \"refs/heads/$1\"")
            && how_to.contains("Retarget a dependent pull request"),
        "Git Town's fail-closed setup, propose, update, and retarget procedure belongs in the how-to"
    );
    for stale in [
        "evidence import",
        "evidence verify",
        "merge eligibility",
        "--request",
        "--evidence",
        "--node",
    ] {
        assert!(
            !reference.contains(stale),
            "delivery reference contains stale CLI surface {stale}"
        );
    }

    let root = repo_root();
    let mut live_docs = vec![
        root.join("AGENTS.md"),
        root.join("README.md"),
        root.join("tests/AGENTS.md"),
        root.join("tests/README.md"),
    ];
    collect_markdown_files(&root.join("docs"), &mut live_docs);
    for path in live_docs {
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| panic!("failed to read {}: {err}", path.display()));
        let lower = content.to_ascii_lowercase();
        for stale in [
            "gh stack",
            "gh-stack",
            "private preview",
            "private-preview",
            "cli_internal",
            "pulls/stacks",
        ] {
            assert!(
                !lower.contains(stale),
                "{} contains stale private stack integration claim {stale}",
                path.display()
            );
        }
        let eval_shell = read_repo_file(".github/workflows/pr-eval-shell-tests.yml");
        assert!(
            eval_shell.contains(
                "mozilla-actions/sccache-action@9e7fa8a12102821edf02ca5dbea1acd0f89a2696"
            ) && eval_shell.contains("D2B_CI_SCCACHE: \"1\"")
                && eval_shell.contains("SCCACHE_DIR: ${{ github.workspace }}/.sccache"),
            "the Rust eval workflow must preserve the pinned sccache wrapper and cache directory"
        );
    }
}

#[test]
fn generated_layer1_workflow_checks_out_every_exact_candidate_head() {
    let workflow = read_repo_file(".github/workflows/pr-l1-static-fast.yml");
    let checkout_count = workflow.matches("uses: actions/checkout@").count();
    let exact_ref_count = workflow
        .matches("ref: ${{ github.event.pull_request.head.sha || github.sha }}")
        .count();

    assert!(
        checkout_count > 0,
        "Layer-1 workflow must check out its tree"
    );
    assert_eq!(
        checkout_count, exact_ref_count,
        "every generated Layer-1 checkout must select the exact candidate head"
    );
}
