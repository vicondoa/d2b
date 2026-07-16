//! Policy/source/doc cross-reference lints (the "H-group"), migrated from the
//! `tests/*-eval.sh` bash gates. Each test reads the real repo files (via the
//! `d2b_contract_tests::read_repo_file` helper) and asserts a structural or
//! documentation invariant. This crate runs only from
//! `tests/tools/rust-workspace-checks.sh` against the real checkout (it is excluded
//! from the hermetic Nix sandbox workspace build), so repo-file access is
//! sound.

use d2b_contract_tests::{read_repo_file, repo_path_exists, repo_root};
use regex::Regex;
use std::path::{Path, PathBuf};

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

fn is_nix_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_'
}

fn is_nix_ident_continue(byte: u8) -> bool {
    is_nix_ident_start(byte) || byte.is_ascii_digit() || matches!(byte, b'-' | b'\'')
}

fn skip_nix_trivia(source: &[u8], cursor: &mut usize) -> Result<(), String> {
    loop {
        while source.get(*cursor).is_some_and(u8::is_ascii_whitespace) {
            *cursor += 1;
        }
        if source.get(*cursor) == Some(&b'#') {
            while source.get(*cursor).is_some_and(|byte| *byte != b'\n') {
                *cursor += 1;
            }
            continue;
        }
        if source.get(*cursor..*cursor + 2) == Some(b"/*") {
            *cursor += 2;
            let mut depth = 1;
            while depth > 0 {
                match source.get(*cursor..*cursor + 2) {
                    Some(b"/*") => {
                        depth += 1;
                        *cursor += 2;
                    }
                    Some(b"*/") => {
                        depth -= 1;
                        *cursor += 2;
                    }
                    Some(_) => *cursor += 1,
                    None => return Err("unterminated block comment in Nix package list".into()),
                }
            }
            continue;
        }
        return Ok(());
    }
}

fn parse_nix_path_list(source: &str, list_open: usize) -> Result<(Vec<String>, usize), String> {
    let source = source.as_bytes();
    if source.get(list_open) != Some(&b'[') {
        return Err("Nix package list does not start with `[`".into());
    }

    let mut cursor = list_open + 1;
    let mut paths = Vec::new();
    loop {
        skip_nix_trivia(source, &mut cursor)?;
        let Some(&byte) = source.get(cursor) else {
            return Err("unterminated Nix package list".into());
        };
        if byte == b']' {
            return Ok((paths, cursor));
        }
        if !is_nix_ident_start(byte) {
            return Err(format!(
                "delivery runtime package list contains unsupported token `{}`",
                char::from(byte)
            ));
        }

        let mut path = String::new();
        loop {
            let segment_start = cursor;
            cursor += 1;
            while source
                .get(cursor)
                .is_some_and(|byte| is_nix_ident_continue(*byte))
            {
                cursor += 1;
            }
            if !path.is_empty() {
                path.push('.');
            }
            path.push_str(
                std::str::from_utf8(&source[segment_start..cursor])
                    .map_err(|_| "non-UTF-8 Nix identifier in delivery runtime package list")?,
            );

            skip_nix_trivia(source, &mut cursor)?;
            if source.get(cursor) != Some(&b'.') {
                break;
            }
            cursor += 1;
            skip_nix_trivia(source, &mut cursor)?;
            if !source
                .get(cursor)
                .is_some_and(|byte| is_nix_ident_start(*byte))
            {
                return Err(
                    "incomplete Nix attribute path in delivery runtime package list".into(),
                );
            }
        }
        paths.push(path);
    }
}

fn find_nix_indented_string_end(source: &str, content_start: usize) -> Result<usize, String> {
    source[content_start..]
        .find("''")
        .map(|offset| content_start + offset)
        .ok_or("unterminated Nix indented string".into())
}

fn skip_nix_quoted_string(source: &[u8], cursor: &mut usize) -> Result<(), String> {
    *cursor += 1;
    while let Some(&byte) = source.get(*cursor) {
        match byte {
            b'\\' => {
                *cursor += 1;
                if source.get(*cursor).is_none() {
                    return Err("unterminated escape in Nix quoted string".into());
                }
                *cursor += 1;
            }
            b'"' => {
                *cursor += 1;
                return Ok(());
            }
            _ => *cursor += 1,
        }
    }
    Err("unterminated Nix quoted string".into())
}

fn nix_indented_attribute<'a>(source: &'a str, name: &str) -> Result<&'a str, String> {
    let bytes = source.as_bytes();
    let mut cursor = 0;
    let mut value = None;
    while cursor < bytes.len() {
        skip_nix_trivia(bytes, &mut cursor)?;
        let Some(&byte) = bytes.get(cursor) else {
            break;
        };
        if bytes.get(cursor..cursor + 2) == Some(b"''") {
            let content_start = cursor + 2;
            cursor = find_nix_indented_string_end(source, content_start)? + 2;
            continue;
        }
        if byte == b'"' {
            skip_nix_quoted_string(bytes, &mut cursor)?;
            continue;
        }
        if !is_nix_ident_start(byte) {
            cursor += 1;
            continue;
        }

        let ident_start = cursor;
        cursor += 1;
        while bytes
            .get(cursor)
            .is_some_and(|byte| is_nix_ident_continue(*byte))
        {
            cursor += 1;
        }
        if &source[ident_start..cursor] != name {
            continue;
        }

        skip_nix_trivia(bytes, &mut cursor)?;
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        skip_nix_trivia(bytes, &mut cursor)?;
        if bytes.get(cursor..cursor + 2) != Some(b"''") {
            return Err(format!("{name} must be a Nix indented string"));
        }
        let content_start = cursor + 2;
        let content_end = find_nix_indented_string_end(source, content_start)?;
        if value.replace(&source[content_start..content_end]).is_some() {
            return Err(format!(
                "multiple active {name} attributes in delivery package"
            ));
        }
        cursor = content_end + 2;
    }

    value.ok_or_else(|| format!("delivery package is missing active {name}"))
}

fn shell_logical_commands(source: &str) -> Result<Vec<String>, String> {
    let bytes = source.as_bytes();
    let mut commands = Vec::new();
    let mut command = String::new();
    let mut cursor = 0;
    let mut quote = None;
    let mut nix_interpolation_depth = 0;

    while let Some(&byte) = bytes.get(cursor) {
        if bytes.get(cursor..cursor + 2) == Some(b"${") {
            nix_interpolation_depth += 1;
            command.push_str("${");
            cursor += 2;
            continue;
        }
        if nix_interpolation_depth > 0 {
            command.push(char::from(byte));
            cursor += 1;
            match byte {
                b'{' => nix_interpolation_depth += 1,
                b'}' => nix_interpolation_depth -= 1,
                _ => {}
            }
            continue;
        }
        if byte == b'\\' && bytes.get(cursor + 1) == Some(&b'\n') && quote != Some(b'\'') {
            command.push(' ');
            cursor += 2;
            continue;
        }
        if let Some(delimiter) = quote {
            command.push(char::from(byte));
            cursor += 1;
            if byte == b'\\' && delimiter == b'"' {
                let Some(&escaped) = bytes.get(cursor) else {
                    return Err("unterminated shell escape in postFixup".into());
                };
                command.push(char::from(escaped));
                cursor += 1;
            } else if byte == delimiter {
                quote = None;
            }
            continue;
        }
        if bytes.get(cursor..cursor + 2) == Some(b"<<") {
            return Err("shell heredocs are forbidden in delivery postFixup".into());
        }

        match byte {
            b'\'' | b'"' => {
                quote = Some(byte);
                command.push(char::from(byte));
                cursor += 1;
            }
            b'#' if command.is_empty()
                || command
                    .as_bytes()
                    .last()
                    .is_some_and(u8::is_ascii_whitespace) =>
            {
                while bytes.get(cursor).is_some_and(|byte| *byte != b'\n') {
                    cursor += 1;
                }
            }
            b'\n' | b';' => {
                if !command.trim().is_empty() {
                    commands.push(command.trim().to_owned());
                }
                command.clear();
                cursor += 1;
            }
            _ => {
                command.push(char::from(byte));
                cursor += 1;
            }
        }
    }
    if quote.is_some() {
        return Err("unterminated shell quote in postFixup".into());
    }
    if nix_interpolation_depth != 0 {
        return Err("unterminated Nix interpolation in postFixup".into());
    }
    if !command.trim().is_empty() {
        commands.push(command.trim().to_owned());
    }
    Ok(commands)
}

fn wrap_program_path_packages(post_fixup: &str) -> Result<Vec<String>, String> {
    let prefix = Regex::new(
        r#"^wrapProgram\s+(?:"(?:\\.|[^"\\])*"|'[^']*'|\S+)\s+--prefix\s+PATH\s+:\s+\$\{\s*pkgs\s*\.\s*lib\s*\.\s*makeBinPath\s*\["#,
    )
    .expect("valid wrapProgram PATH regex");
    let mut matches = Vec::new();
    for command in shell_logical_commands(post_fixup)? {
        let Some(runtime_path) = prefix.find(&command) else {
            continue;
        };
        let (packages, list_close) = parse_nix_path_list(&command, runtime_path.end() - 1)?;
        let bytes = command.as_bytes();
        let mut cursor = list_close + 1;
        skip_nix_trivia(bytes, &mut cursor)?;
        if bytes.get(cursor) != Some(&b'}') || !command[cursor + 1..].trim().is_empty() {
            return Err("wrapProgram PATH must end with the makeBinPath interpolation".into());
        }
        matches.push(packages);
    }
    match matches.as_slice() {
        [packages] => Ok(packages.clone()),
        [] => Err("delivery postFixup is missing active wrapProgram PATH prefix".into()),
        _ => Err("delivery postFixup has multiple wrapProgram PATH prefixes".into()),
    }
}

fn delivery_runtime_packages(flake: &str) -> Result<Vec<String>, String> {
    let branch_start = Regex::new(r#"spec\s*\.\s*buildKind\s*==\s*"deliveryWorkspace"\s*then"#)
        .expect("valid delivery branch regex")
        .find(flake)
        .ok_or("missing deliveryWorkspace package branch")?;
    let branch_tail = &flake[branch_start.end()..];
    let branch_end = Regex::new(
        r#"passthru\s*\.\s*rustToolchainVersion\s*=\s*deliveryTools\s*\.\s*rustStableVersion\s*;"#,
    )
    .expect("valid delivery branch end regex")
    .find(branch_tail)
    .ok_or("deliveryWorkspace package branch is missing its pinned toolchain passthru")?;
    let branch = &branch_tail[..branch_end.start()];
    wrap_program_path_packages(nix_indented_attribute(branch, "postFixup")?)
}

const REQUIRED_DELIVERY_RUNTIME_PACKAGES: [&str; 5] = [
    "pkgs.git",
    "pkgs.openssl",
    "pkgs.shellcheck",
    "deliveryTools.gh",
    "deliveryTools.gitTown",
];

fn required_delivery_runtime_packages(flake: &str) -> Result<Vec<String>, String> {
    let packages = delivery_runtime_packages(flake)?;
    for required in REQUIRED_DELIVERY_RUNTIME_PACKAGES {
        if !packages.iter().any(|package| package == required) {
            return Err(format!(
                "delivery wrapProgram PATH is missing {required}; found {packages:?}"
            ));
        }
    }
    if packages.len() != REQUIRED_DELIVERY_RUNTIME_PACKAGES.len() {
        return Err(format!(
            "delivery wrapProgram PATH must contain exactly the required tools; found {packages:?}"
        ));
    }
    Ok(packages)
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
    required_delivery_runtime_packages(&flake)
        .unwrap_or_else(|err| panic!("invalid delivery runtime package list: {err}"));
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
            && flake.contains(
                "passthru.rustToolchainVersion = deliveryTools.rustStableVersion;"
            )
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
      decoy = pkgs.lib.makeBinPath [
        pkgs.git pkgs.openssl pkgs.shellcheck
      ];
      value =
        if spec . buildKind == "deliveryWorkspace"
        then deliveryRustWorkspace (workspaceArgs // {
          # commentedRuntime = pkgs.lib.makeBinPath [
          #   pkgs.git pkgs.openssl pkgs.shellcheck
          #   deliveryTools.gh deliveryTools.gitTown
          # ];
          unusedRuntime = pkgs.lib.makeBinPath [
            pkgs.git pkgs.openssl pkgs.shellcheck
            deliveryTools.gh deliveryTools.gitTown
          ];
          stringDecoy = "postFixup pkgs.lib.makeBinPath [ pkgs.openssl ]";
          postFixup = ''
            # wrapProgram "$out/bin/decoy" --prefix PATH : ${pkgs.lib.makeBinPath [
            #   pkgs.git pkgs.openssl pkgs.shellcheck
            #   deliveryTools.gh deliveryTools.gitTown
            # ]}
            wrapProgram \
              "$out/bin/${spec.binary}" \
                --prefix   PATH : ${pkgs . lib . makeBinPath
                  [
                    pkgs . git
                    # Layout and comments are not package entries.
                    pkgs.openssl  pkgs . shellcheck
                    deliveryTools . gh
                    deliveryTools.gitTown
                  ]}
          '';
          passthru . rustToolchainVersion =
            deliveryTools . rustStableVersion ;
        })
        else null;
    "#
}

#[test]
fn delivery_runtime_package_parser_binds_active_wrap_program_path() {
    let flake = delivery_runtime_policy_fixture();

    assert_eq!(
        required_delivery_runtime_packages(flake).expect("formatting variant must parse"),
        [
            "pkgs.git",
            "pkgs.openssl",
            "pkgs.shellcheck",
            "deliveryTools.gh",
            "deliveryTools.gitTown",
        ]
    );

    let missing_active_tool = flake.replace(
        "                    pkgs.openssl  pkgs . shellcheck",
        "                    pkgs . shellcheck",
    );
    let error = required_delivery_runtime_packages(&missing_active_tool)
        .expect_err("commented and unused in-branch decoys must not mask a missing runtime tool");
    assert!(
        error.contains("missing pkgs.openssl"),
        "missing active tool must fail specifically: {error}"
    );

    let inactive_wrapper = flake.replacen(
        "            wrapProgram \\",
        "            echo wrapProgram \\",
        1,
    );
    let error = required_delivery_runtime_packages(&inactive_wrapper)
        .expect_err("commented, string, and unused decoys must not replace the active wrapper");
    assert!(
        error.contains("missing active wrapProgram PATH prefix"),
        "inactive wrapper must fail specifically: {error}"
    );
}

#[test]
fn delivery_runtime_package_parser_rejects_heredoc_decoys() {
    for opener in ["cat <<EOF", "cat <<'EOF'", "cat <<\"EOF\"", "cat <<-EOF"] {
        let heredoc_decoy = delivery_runtime_policy_fixture()
            .replacen(
                "            wrapProgram \\",
                &format!("            {opener}\n            wrapProgram \\"),
                1,
            )
            .replacen(
                "                  ]}\n",
                "                  ]}\n            EOF\n",
                1,
            );
        let error = required_delivery_runtime_packages(&heredoc_decoy)
            .expect_err("a wrapper-looking heredoc body must not satisfy runtime PATH policy");
        assert!(
            error.contains("heredocs are forbidden"),
            "{opener} must fail closed before its body is parsed: {error}"
        );
    }
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
