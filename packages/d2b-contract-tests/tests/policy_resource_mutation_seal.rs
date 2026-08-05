use std::{
    fs,
    path::{Path, PathBuf},
};

use d2b_contract_tests::{read_repo_file, repo_root};

const MUTATION_SEAL_SOURCE: &str = include_str!("../../d2b-resource-store/src/mutation_seal.rs");
const STORE_SOURCE: &str = include_str!("../../d2b-resource-store/src/lib.rs");
const STORE_MANIFEST: &str = include_str!("../../d2b-resource-store/Cargo.toml");
const REDB_MANIFEST: &str = include_str!("../../d2b-resource-store-redb/Cargo.toml");

const OLD_WRITE_SYMBOLS: &[&str] = &[
    "VerifiedMutation",
    "VerifiedMutationView",
    "VerifiedPreparedMutationView",
    "MutationPort",
    "type_name::<",
];

fn repository_files(root: &Path) -> Vec<PathBuf> {
    assert!(root.exists(), "policy scan root is missing");
    if root.is_file() {
        return vec![root.to_path_buf()];
    }
    assert!(root.is_dir(), "policy scan root is not a directory");
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        let entries = fs::read_dir(&directory).unwrap_or_else(|error| {
            panic!("policy scan cannot read repository directory: {error}")
        });
        for entry in entries {
            let entry = entry.unwrap_or_else(|error| {
                panic!("policy scan cannot read repository directory entry: {error}")
            });
            let path = entry.path();
            let file_type = entry.file_type().unwrap_or_else(|error| {
                panic!("policy scan cannot inspect repository entry: {error}")
            });
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                let excluded = path.file_name().is_some_and(|name| {
                    matches!(name.to_str(), Some("target" | ".scratch" | ".git"))
                });
                if !excluded {
                    pending.push(path);
                }
            } else if file_type.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

fn workspace_rust_sources() -> Vec<(String, String)> {
    let root = repo_root();
    let workspace = root.join("packages");
    let sources = repository_files(&workspace)
        .into_iter()
        .filter(|path| path.extension().is_some_and(|extension| extension == "rs"))
        .map(|path| {
            let relative = path
                .strip_prefix(&root)
                .expect("workspace source is below repository root")
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("policy scan cannot read {relative}: {error}"));
            (relative, source)
        })
        .collect::<Vec<_>>();
    assert!(
        !sources.is_empty(),
        "workspace source scan found no Rust files; refusing to report a passing no-op"
    );
    sources
}

fn is_test_attribute(line: &str) -> bool {
    let line = line.trim();
    line.starts_with("#[test]")
        || line.starts_with("#[tokio::test")
        || (line.starts_with("#[cfg(") && line.contains("test") && !line.contains("not(test)"))
}

fn quoted_string_end(source: &[u8], start: usize) -> usize {
    let mut index = start + 1;
    while index < source.len() {
        match source[index] {
            b'\\' => index += 2,
            b'"' => return index + 1,
            _ => index += 1,
        }
    }
    source.len()
}

fn raw_string_end(source: &[u8], start: usize) -> Option<usize> {
    if source.get(start).copied() != Some(b'r') {
        return None;
    }
    let mut hashes = 0;
    let mut index = start + 1;
    while source.get(index).copied() == Some(b'#') {
        hashes += 1;
        index += 1;
    }
    if source.get(index).copied() != Some(b'"') {
        return None;
    }
    let closing = format!("\"{}", "#".repeat(hashes));
    let closing = closing.as_bytes();
    source[index + 1..]
        .windows(closing.len())
        .position(|window| window == closing)
        .map(|offset| index + 1 + offset + closing.len())
}

fn char_literal_end(source: &[u8], start: usize) -> Option<usize> {
    let next = source.get(start + 1).copied()?;
    if next == b'\\' {
        return (start + 3 < source.len() && source[start + 3] == b'\'').then_some(start + 4);
    }
    (source.get(start + 2).copied() == Some(b'\'')).then_some(start + 3)
}

fn matching_brace_end(source: &str, open: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut index = open;
    let mut block_comment_depth = 0usize;
    while index < bytes.len() {
        if block_comment_depth > 0 {
            if bytes.get(index..index + 2) == Some(b"/*") {
                block_comment_depth += 1;
                index += 2;
            } else if bytes.get(index..index + 2) == Some(b"*/") {
                block_comment_depth -= 1;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if bytes.get(index..index + 2) == Some(b"//") {
            index = bytes[index..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |offset| index + offset + 1);
            continue;
        }
        if bytes.get(index..index + 2) == Some(b"/*") {
            block_comment_depth = 1;
            index += 2;
            continue;
        }
        if bytes[index] == b'"' {
            index = quoted_string_end(bytes, index);
            continue;
        }
        if let Some(end) = raw_string_end(bytes, index) {
            index = end;
            continue;
        }
        if bytes[index] == b'\''
            && let Some(end) = char_literal_end(bytes, index)
        {
            index = end;
            continue;
        }
        match bytes[index] {
            b'{' => depth += 1,
            b'}' => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index + 1);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn test_item_end(source: &str, start: usize) -> usize {
    let bytes = source.as_bytes();
    let mut index = start;
    let mut block_comment_depth = 0usize;
    while index < bytes.len() {
        if block_comment_depth > 0 {
            if bytes.get(index..index + 2) == Some(b"/*") {
                block_comment_depth += 1;
                index += 2;
            } else if bytes.get(index..index + 2) == Some(b"*/") {
                block_comment_depth -= 1;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if bytes.get(index..index + 2) == Some(b"//") {
            index = bytes[index..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(bytes.len(), |offset| index + offset + 1);
            continue;
        }
        if bytes.get(index..index + 2) == Some(b"/*") {
            block_comment_depth = 1;
            index += 2;
            continue;
        }
        if bytes[index] == b'"' {
            index = quoted_string_end(bytes, index);
            continue;
        }
        if let Some(end) = raw_string_end(bytes, index) {
            index = end;
            continue;
        }
        if bytes[index] == b'\''
            && let Some(end) = char_literal_end(bytes, index)
        {
            index = end;
            continue;
        }
        match bytes[index] {
            b'{' => {
                return matching_brace_end(source, index)
                    .unwrap_or_else(|| panic!("test-only item has an unclosed body"));
            }
            b';' => return index + 1,
            _ => index += 1,
        }
    }
    panic!("test-only attribute has no following Rust item");
}

fn strip_test_items(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut cursor = 0usize;
    let mut search = 0usize;
    while search < source.len() {
        let line_start = if search == 0 {
            0
        } else {
            source[..search]
                .rfind('\n')
                .map_or(0, |newline| newline + 1)
        };
        let line_end = source[search..]
            .find('\n')
            .map_or(source.len(), |offset| search + offset);
        if is_test_attribute(&source[line_start..line_end]) {
            let end = test_item_end(source, line_start);
            output.push_str(&source[cursor..line_start]);
            cursor = end;
            search = end;
        } else {
            search = line_end.saturating_add(1);
        }
    }
    output.push_str(&source[cursor..]);
    output
}

fn non_test_source(relative: &str, source: &str) -> String {
    if relative
        .split('/')
        .any(|component| matches!(component, "tests" | "benches"))
        || relative.ends_with("/tests.rs")
    {
        return String::new();
    }
    strip_test_items(source)
}

#[test]
fn cargo_bench_support_is_test_only() {
    let source = "mutation_seal_pair(store_identity());";
    assert!(
        non_test_source("packages/example/benches/support.rs", source).is_empty(),
        "Cargo bench support must not count as a production call site"
    );
    assert_eq!(
        non_test_source("packages/example/src/support.rs", source),
        source,
        "ordinary source must remain part of the production scan"
    );
}

fn without_solver_assertion(source: &str) -> String {
    let start_marker = "fn assert_mutation_seal_types_have_no_minting_traits";
    let end_marker = "const _: fn() = assert_mutation_seal_types_have_no_minting_traits;";
    let start = source
        .find(start_marker)
        .expect("mutation-seal trait-solver assertion is missing");
    let end = source[start..]
        .find(end_marker)
        .map(|offset| start + offset + end_marker.len())
        .expect("mutation-seal trait-solver assertion terminator is missing");
    let end = source[end..]
        .find('\n')
        .map_or(source.len(), |offset| end + offset);
    for forbidden in [
        "Display",
        "Serialize",
        "Deserialize",
        "JsonSchema",
        "PartialEq",
        "Eq",
        "as_str",
        "to_canonical_string",
    ] {
        assert!(
            !source[start..end].contains(forbidden),
            "mutation-seal trait-solver assertion contains forbidden token {forbidden:?}"
        );
    }
    format!("{}{}", &source[..start], &source[end..])
}

fn compact(source: &str) -> String {
    source
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn count_issuer_seal_calls(source: &str) -> usize {
    let compact = compact(source);
    let mut count = compact.matches("MutationSealIssuer::seal(").count();
    for (offset, _) in compact.match_indices(".seal(") {
        let name_end = offset;
        let name_start = compact[..name_end]
            .rfind(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .map_or(0, |index| index + 1);
        if compact[name_start..name_end].contains("issuer") {
            count += 1;
        }
    }
    count
}

#[test]
fn mutation_seal_mint_sites_are_single_owner() {
    let mut pair_definitions = 0usize;
    let mut pair_calls = 0usize;
    let mut issuer_seal_calls = 0usize;
    let mut store_slot_assignments = 0usize;

    for (relative, source) in workspace_rust_sources() {
        let source = non_test_source(&relative, &source);
        if source.is_empty() {
            continue;
        }
        let compact = compact(&source);
        pair_definitions += compact.matches("fnmutation_seal_pair(").count();
        pair_calls += compact.matches("mutation_seal_pair(").count();
        issuer_seal_calls += count_issuer_seal_calls(&source);
        store_slot_assignments += compact.matches("StoreSlot::new(").count();
    }

    assert_eq!(
        pair_definitions, 1,
        "mutation_seal_pair must have exactly one definition"
    );
    assert_eq!(
        pair_calls - pair_definitions,
        1,
        "mutation_seal_pair must have exactly one non-test call site"
    );
    assert_eq!(
        issuer_seal_calls, 1,
        "MutationSealIssuer::seal must have exactly one non-test call site"
    );
    assert!(
        store_slot_assignments <= 1,
        "StoreSlot::new must have at most one non-test call site, found {store_slot_assignments}"
    );
}

#[test]
fn mutation_seal_source_has_no_escape_hatches_or_rendering_traits() {
    assert!(
        !MUTATION_SEAL_SOURCE.contains("cfg(test)")
            && !MUTATION_SEAL_SOURCE.contains("cfg(not(test))"),
        "mutation_seal.rs must not contain a test-configuration escape hatch"
    );
    assert!(
        MUTATION_SEAL_SOURCE.contains("CapabilityMustNotImplementCloneCopyDefaultDebugOrFrom"),
        "mutation_seal.rs must carry the five-trait solver ambiguity assertion"
    );

    let source_without_solver = without_solver_assertion(MUTATION_SEAL_SOURCE);
    for forbidden in [
        "Debug",
        "Display",
        "Serialize",
        "Deserialize",
        "JsonSchema",
        "PartialEq",
        "Eq",
        "as_str",
        "to_canonical_string",
    ] {
        assert!(
            !source_without_solver.contains(forbidden),
            "mutation_seal.rs contains forbidden token {forbidden:?}"
        );
    }
}

#[test]
fn resource_store_seal_keeps_dependency_and_encoding_boundaries_closed() {
    for (name, source) in [
        ("d2b-resource-store/src/lib.rs", STORE_SOURCE),
        (
            "d2b-resource-store/src/mutation_seal.rs",
            MUTATION_SEAL_SOURCE,
        ),
    ] {
        for forbidden in ["RoleBinding", "Role::"] {
            assert!(
                !source.contains(forbidden),
                "{name} must not name RBAC evaluator symbol {forbidden:?}"
            );
        }
    }
    for (name, manifest) in [
        ("d2b-resource-store/Cargo.toml", STORE_MANIFEST),
        ("d2b-resource-store-redb/Cargo.toml", REDB_MANIFEST),
    ] {
        assert!(
            !manifest.contains("d2b-resource-api"),
            "{name} must not depend on d2b-resource-api"
        );
    }

    let root = repo_root();
    let source_roots = [
        root.join("packages/d2b-resource-store/src"),
        root.join("packages/d2b-resource-store-redb/src"),
    ];
    for source_root in source_roots {
        for path in repository_files(&source_root) {
            let relative = path
                .strip_prefix(&root)
                .expect("resource-store source is below repository root")
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("policy scan cannot read {relative}: {error}"));
            for forbidden in OLD_WRITE_SYMBOLS {
                assert!(
                    !source.contains(forbidden),
                    "{relative} retains old generic write symbol {forbidden:?}"
                );
            }
            assert!(
                !source.contains("d2b-resource-api"),
                "{relative} must not name d2b-resource-api"
            );
        }
    }

    let encoding_paths = [
        "packages/d2b-contracts",
        "packages/d2b-resource-store-redb/src/keys.rs",
        "packages/d2b-resource-store-redb/src/values.rs",
        "packages/d2b-resource-store-redb/src/schema.rs",
    ];
    for relative in encoding_paths {
        let path = root.join(relative);
        assert!(
            path.exists(),
            "encoding policy input is missing: {relative}"
        );
        for path in repository_files(&path) {
            let path_relative = path
                .strip_prefix(&root)
                .expect("encoding policy input is below repository root")
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(&path).unwrap_or_else(|error| {
                panic!("encoding policy cannot read {path_relative}: {error}")
            });
            assert!(
                !source.contains("StoreSlot"),
                "{path_relative} must not encode StoreSlot"
            );
        }
    }
}

#[test]
fn mutation_seal_policy_inputs_are_present() {
    for relative in [
        "packages/d2b-resource-store/src/mutation_seal.rs",
        "packages/d2b-resource-store-redb/src/keys.rs",
        "packages/d2b-resource-store-redb/src/values.rs",
        "packages/d2b-resource-store-redb/src/schema.rs",
    ] {
        assert!(
            repo_root().join(relative).is_file(),
            "policy input is missing: {relative}"
        );
    }
    let _ = read_repo_file("packages/d2b-resource-store/Cargo.toml");
}
