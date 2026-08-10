//! The Provider crate layout, identity, and dependency-direction policy.
//!
//! `ADR-046-resources-zone-control` section 4.8 makes the Provider crate shape
//! normative: every Provider implementation lives in its own workspace member
//! named `d2b-provider-<base>-<implementation>` and MUST carry `src/`,
//! `tests/`, `integration/`, and a `README.md` holding the nine headings that
//! section 4.8.3 enumerates. Section 4.8.4 requires every `integration/*.rs`
//! file to declare exactly one orchestration target in its first 20 lines.
//! `ADR-046-provider-model-and-packaging` adds the two rules that make a
//! Provider a unit rather than a bag of code: one crate is exactly one Provider
//! identity, and a Provider depends only on the public neutral contract, the
//! toolkits, and the SDK.
//!
//! The check is filesystem-only. It compiles nothing, so it stays hermetic and
//! runs in the enforcing `make test-policy` lane rather than the fixture lane.
//!
//! Every rule is proven twice: once against the real workspace, and once
//! against a synthetic crate tree that violates it, so a rule whose matcher
//! silently stops matching fails here rather than passing vacuously.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU32, Ordering},
};

use d2b_contract_tests::{read_repo_file, repo_root};

/// The four paths section 4.8.1 requires.
const REQUIRED_PATHS: &[&str] = &["src", "tests", "integration", "README.md"];

/// The nine README headings section 4.8.3 enumerates, in its order.
const REQUIRED_SECTIONS: &[&str] = &[
    "Provider identity",
    "Config schema",
    "Exported resource types",
    "Controllers / services / workers / binaries",
    "Placement and dependencies",
    "RBAC requirements",
    "Security posture",
    "State and telemetry",
    "Build and test",
];

/// The two orchestration targets section 4.8.4 admits.
const INTEGRATION_TARGETS: &[&str] = &["container", "host-integration"];

/// Lines of an `integration/*.rs` file the target declaration must appear in.
const DECLARATION_WINDOW: usize = 20;

/// Workspace crates a Provider crate may depend on.
///
/// This is an allowlist rather than a denylist on purpose: a denylist admits
/// every crate nobody thought to name, and the direction rule is exactly the
/// kind of invariant a new workspace member quietly breaks. The admitted set is
/// the public neutral contract, the shared conformance kit, the toolkits, and
/// the Provider SDK.
const ALLOWED_WORKSPACE_DEPS: &[&str] = &[
    "d2b-contracts",
    "d2b-controller-toolkit",
    "d2b-core",
    "d2b-process-conformance",
    "d2b-provider",
    "d2b-provider-toolkit",
];

/// Workspace crates whose appearance in a Provider crate names a specific
/// inversion, so the failure can say which rule broke rather than only that the
/// allowlist rejected the name.
const NAMED_INVERSIONS: &[(&str, &str)] = &[
    ("d2bd", "the daemon"),
    ("d2b-priv-broker", "the privileged broker"),
    ("d2b-resource-store", "the Zone store"),
    ("d2b-resource-store-redb", "the Zone store backend"),
    ("d2b-resource-api", "the Zone store API"),
    ("d2b-bus", "the Zone message bus"),
    ("d2b-zone-routing", "the Zone routing plane"),
    ("d2b-host", "the host lifecycle primitives"),
];

/// Crates exempt from the naming rule and therefore from this whole policy,
/// each with the reason recorded in the wave's implementation-debt register.
///
/// Both are pre-ADR-046 crates carrying a single segment after
/// `d2b-provider-`, so neither matches `<base>-<implementation>` at all, and
/// both are dispositioned REPLACE by the migration map. Reshaping a crate
/// scheduled for deletion would be work thrown away, so the exemption is
/// recorded rather than the crates renamed.
const EXEMPT_CRATES: &[(&str, &str)] = &[
    (
        "d2b-provider-aca",
        "pre-ADR-046 crate, single-segment name, dispositioned REPLACE by \
         Provider/runtime-azure-container-apps; exemption retires with its removal",
    ),
    (
        "d2b-provider-relay",
        "pre-ADR-046 crate, single-segment name, dispositioned REPLACE by \
         Provider/transport-azure-relay; exemption retires with its removal",
    ),
];

/// Workspace members under `packages/` that carry the `d2b-provider-` prefix
/// but are not Provider implementations.
///
/// `d2b-provider` is the Provider SDK, `d2b-provider-toolkit` is the shared
/// toolkit, and `d2b-provider-supervisor` is the host-side supervisor a
/// Provider process runs under. None names a `<base>-<implementation>` pair,
/// and none is a Provider: the supervisor launches Providers rather than
/// implementing one, so holding it to the Provider layout would assert a
/// dossier and an identity it has no business declaring.
const NON_PROVIDER_PREFIXED: &[&str] = &[
    "d2b-provider",
    "d2b-provider-supervisor",
    "d2b-provider-toolkit",
];

// ---------------------------------------------------------------------------
// The checker, expressed over a crate directory so it can be driven by both the
// real workspace and a synthetic violating tree.
// ---------------------------------------------------------------------------

/// One policy violation, rendered as the message the gate prints.
type Violation = String;

/// Split `d2b-provider-<base>-<implementation>` into its two segments.
///
/// Returns `None` when the name carries fewer than two segments after the
/// prefix, which is exactly the shape the two exempt crates have.
fn split_provider_name(crate_name: &str) -> Option<(&str, &str)> {
    let rest = crate_name.strip_prefix("d2b-provider-")?;
    let (base, implementation) = rest.split_once('-')?;
    if base.is_empty() || implementation.is_empty() {
        return None;
    }
    Some((base, implementation))
}

/// The Provider identity a crate name denotes: `<base>-<implementation>`.
fn provider_identity(crate_name: &str) -> Option<String> {
    split_provider_name(crate_name).map(|(base, implementation)| format!("{base}-{implementation}"))
}

/// Whether this workspace member is in the naming rule's scope.
fn is_in_scope(crate_name: &str) -> bool {
    if NON_PROVIDER_PREFIXED.contains(&crate_name) {
        return false;
    }
    if EXEMPT_CRATES.iter().any(|(name, _)| *name == crate_name) {
        return false;
    }
    provider_identity(crate_name).is_some()
}

/// Section 4.8.1: all four required paths exist.
fn check_required_paths(crate_name: &str, dir: &Path) -> Vec<Violation> {
    REQUIRED_PATHS
        .iter()
        .filter(|path| !dir.join(path).exists())
        .map(|path| format!("{crate_name}: missing required path '{path}'"))
        .collect()
}

/// Normalize a Markdown heading line to its comparable text, or `None` when the
/// line is not a heading.
fn heading_text(line: &str) -> Option<String> {
    let trimmed = line.trim();
    let stripped = trimmed.strip_prefix('#')?;
    Some(stripped.trim_start_matches('#').trim().to_lowercase())
}

/// Section 4.8.3: all nine headings are present, matched case-insensitively.
fn check_readme_sections(crate_name: &str, dir: &Path) -> Vec<Violation> {
    let readme = dir.join("README.md");
    let Ok(text) = fs::read_to_string(&readme) else {
        // Absence is reported by `check_required_paths`; do not double-report.
        return Vec::new();
    };
    let present: BTreeSet<String> = text.lines().filter_map(heading_text).collect();
    REQUIRED_SECTIONS
        .iter()
        .filter(|section| !present.contains(&section.to_lowercase()))
        .map(|section| format!("{crate_name}/README.md: missing required section '{section}'"))
        .collect()
}

/// Section 4.8.4: each `integration/*.rs` file declares exactly one valid
/// orchestration target within its first 20 lines.
fn check_integration_targets(crate_name: &str, dir: &Path) -> Vec<Violation> {
    let integration = dir.join("integration");
    let Ok(entries) = fs::read_dir(&integration) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .collect();
    files.sort();

    let mut violations = Vec::new();
    for file in files {
        let name = file
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("<unnamed>")
            .to_owned();
        let label = format!("{crate_name}/integration/{name}");
        let Ok(text) = fs::read_to_string(&file) else {
            violations.push(format!("{label}: unreadable"));
            continue;
        };
        let declared: Vec<&str> = text
            .lines()
            .take(DECLARATION_WINDOW)
            .filter_map(|line| line.trim().strip_prefix("//! integration-target:"))
            .map(|value| value.split("//").next().unwrap_or(value).trim())
            .collect();
        match declared.as_slice() {
            [] => violations.push(format!(
                "{label}: no 'integration-target:' declaration in the first \
                 {DECLARATION_WINDOW} lines"
            )),
            [one] => {
                if !INTEGRATION_TARGETS.contains(one) {
                    violations.push(format!(
                        "{label}: invalid integration-target '{one}'; expected one of {}",
                        INTEGRATION_TARGETS.join(", ")
                    ));
                }
            }
            many => violations.push(format!(
                "{label}: {} 'integration-target:' declarations; exactly one is required",
                many.len()
            )),
        }
    }
    violations
}

/// Workspace-local dependency names declared by a crate's `Cargo.toml`.
///
/// A workspace-local dependency is one declared with a `path = "../<name>"`
/// entry. Registry dependencies carry no such key and are outside the direction
/// rule, which is about the shape of the internal graph.
fn workspace_dependencies(manifest: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') || !trimmed.contains("path = \"../") {
            continue;
        }
        let Some((name, _)) = trimmed.split_once('=') else {
            continue;
        };
        let name = name.trim().trim_matches('"');
        if !name.is_empty() {
            found.insert(name.to_owned());
        }
    }
    found
}

/// The dependency-direction rule: a Provider crate depends only on the public
/// neutral contract, the toolkits, and the SDK.
fn check_dependency_direction(crate_name: &str, dir: &Path) -> Vec<Violation> {
    let Ok(manifest) = fs::read_to_string(dir.join("Cargo.toml")) else {
        return vec![format!("{crate_name}: Cargo.toml is unreadable")];
    };
    let mut violations = Vec::new();
    for dependency in workspace_dependencies(&manifest) {
        if ALLOWED_WORKSPACE_DEPS.contains(&dependency.as_str()) {
            continue;
        }
        if let Some((_, what)) = NAMED_INVERSIONS
            .iter()
            .find(|(name, _)| *name == dependency)
        {
            violations.push(format!(
                "{crate_name}: depends on '{dependency}' ({what}); a Provider crate \
                 depends only on the public neutral contract, the toolkits, and the SDK"
            ));
            continue;
        }
        if is_in_scope(&dependency) {
            violations.push(format!(
                "{crate_name}: depends on sibling Provider crate '{dependency}'; a Provider \
                 never reaches into another Provider's internals"
            ));
            continue;
        }
        violations.push(format!(
            "{crate_name}: depends on workspace crate '{dependency}', which is not an \
             admitted Provider dependency; admitted crates are {}",
            ALLOWED_WORKSPACE_DEPS.join(", ")
        ));
    }
    violations
}

/// The one-crate-one-identity rule, README half.
///
/// The crate name already denotes exactly one `<base>-<implementation>` pair,
/// so the half a filesystem check can add is that the README does not declare a
/// second identity, or a different one.
fn check_single_identity(crate_name: &str, dir: &Path) -> Vec<Violation> {
    let Some(identity) = provider_identity(crate_name) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(dir.join("README.md")) else {
        return Vec::new();
    };
    let declared: Vec<String> = text
        .lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with('|') {
                return None;
            }
            let cells: Vec<&str> = trimmed
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();
            let [label, value, ..] = cells.as_slice() else {
                return None;
            };
            if !label.eq_ignore_ascii_case("Provider name") {
                return None;
            }
            Some(value.trim_matches('`').to_owned())
        })
        .collect();
    match declared.as_slice() {
        [] => Vec::new(),
        [one] if *one == identity => Vec::new(),
        [one] => vec![format!(
            "{crate_name}/README.md: declares Provider name '{one}' but the crate name \
             denotes identity '{identity}'; one crate is exactly one Provider"
        )],
        many => vec![format!(
            "{crate_name}/README.md: declares {} Provider names; one crate is exactly \
             one Provider",
            many.len()
        )],
    }
}

// ---------------------------------------------------------------------------
// Dossier parity, expressed over a dossier directory so it can be driven by
// both the real `docs/specs/providers/` tree and a synthetic violating one.
// ---------------------------------------------------------------------------

/// Where the Provider dossiers live, relative to the repository root.
const DOSSIER_DIR: &str = "docs/specs/providers";

/// The dossier file name a Provider identity denotes.
///
/// `ADR-046-provider-model-and-packaging` says every Provider "has one
/// `ADR-046-provider-<provider-name>.md` dossier", and the frozen catalog
/// repeats it per row. The Provider name is the `<base>-<implementation>` pair
/// the crate name already denotes, so the crate name fixes the file name.
fn dossier_file_name(identity: &str) -> String {
    format!("ADR-046-provider-{identity}.md")
}

/// The values of every `| <label> | <value> |` row in a Markdown table.
fn table_row_values(text: &str, label: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if !trimmed.starts_with('|') {
                return None;
            }
            let cells: Vec<&str> = trimmed
                .trim_matches('|')
                .split('|')
                .map(str::trim)
                .collect();
            let [row_label, value, ..] = cells.as_slice() else {
                return None;
            };
            if !row_label.eq_ignore_ascii_case(label) {
                return None;
            }
            Some(value.trim_matches('`').to_owned())
        })
        .collect()
}

/// Dossier parity: every in-scope Provider crate has a dossier, and the two
/// agree on the identity.
///
/// The two directions are deliberately not symmetric. A dossier with no crate
/// is legitimate: the dossier set is the frozen initial Provider catalog, and
/// its crates land over several waves, so the specification's obligation runs
/// from the crate to its dossier and not back. A crate with no dossier is the
/// failure, because a Provider that ships without its normative specification
/// is exactly the drift this rule exists to catch.
fn check_dossier_parity(crate_name: &str, dossiers: &Path) -> Vec<Violation> {
    let Some(identity) = provider_identity(crate_name) else {
        // Out of scope: no identity, so no dossier is named.
        return Vec::new();
    };
    let file_name = dossier_file_name(&identity);
    let path = dossiers.join(&file_name);
    let Ok(text) = fs::read_to_string(&path) else {
        return vec![format!(
            "{crate_name}: no Provider dossier at {DOSSIER_DIR}/{file_name}; every Provider \
             crate has exactly one dossier naming its identity '{identity}'"
        )];
    };
    let expected = format!("ADR-046-provider-{identity}");
    let declared = table_row_values(&text, "Spec ID");
    match declared.as_slice() {
        [] => vec![format!(
            "{DOSSIER_DIR}/{file_name}: no 'Spec ID' row; the dossier for '{crate_name}' must \
             declare Spec ID '{expected}'"
        )],
        [one] if *one == expected => Vec::new(),
        [one] => vec![format!(
            "{DOSSIER_DIR}/{file_name}: declares Spec ID '{one}' but crate '{crate_name}' \
             denotes identity '{identity}', so its dossier declares '{expected}'"
        )],
        many => vec![format!(
            "{DOSSIER_DIR}/{file_name}: declares {} 'Spec ID' rows; a dossier declares exactly \
             one identity",
            many.len()
        )],
    }
}

/// Every dossier file name in a dossier directory, as its declared identity.
///
/// `README.md` is the directory's own index, not a dossier.
fn dossier_identities(dossiers: &Path) -> BTreeSet<String> {
    let Ok(entries) = fs::read_dir(dossiers) else {
        return BTreeSet::new();
    };
    entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let stem = name.strip_suffix(".md")?;
            stem.strip_prefix("ADR-046-provider-").map(str::to_owned)
        })
        .collect()
}

/// Every rule, applied to one in-scope crate directory.
fn check_crate(crate_name: &str, dir: &Path) -> Vec<Violation> {
    let mut violations = check_required_paths(crate_name, dir);
    violations.extend(check_readme_sections(crate_name, dir));
    violations.extend(check_integration_targets(crate_name, dir));
    violations.extend(check_dependency_direction(crate_name, dir));
    violations.extend(check_single_identity(crate_name, dir));
    violations
}

// ---------------------------------------------------------------------------
// Workspace discovery
// ---------------------------------------------------------------------------

/// The `[workspace] members` list from `packages/Cargo.toml`.
fn workspace_members() -> Vec<String> {
    let manifest = read_repo_file("packages/Cargo.toml");
    let start = manifest
        .find("members = [")
        .expect("packages/Cargo.toml declares [workspace] members");
    let rest = &manifest[start..];
    let end = rest.find(']').expect("the members list terminates");
    rest[..end]
        .lines()
        .skip(1)
        .filter_map(|line| {
            let trimmed = line.trim().trim_end_matches(',');
            trimmed
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .map(str::to_owned)
        })
        .collect()
}

/// In-scope Provider crates, as `(name, directory)`.
fn provider_crates() -> Vec<(String, PathBuf)> {
    let packages = repo_root().join("packages");
    workspace_members()
        .into_iter()
        .filter(|name| is_in_scope(name))
        .map(|name| {
            let dir = packages.join(&name);
            (name, dir)
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Synthetic-tree scaffolding for the negative cases
// ---------------------------------------------------------------------------

static SCRATCH_COUNTER: AtomicU32 = AtomicU32::new(0);

/// A synthetic Provider crate directory, removed on drop.
struct Fixture {
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl Fixture {
    /// A conformant synthetic crate: all four paths, all nine headings, one
    /// valid integration target, one admitted dependency, one identity.
    fn conformant(label: &str) -> Self {
        let serial = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "d2b-provider-crates-policy-{}-{serial}-{label}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).expect("fixture src");
        fs::create_dir_all(dir.join("tests")).expect("fixture tests");
        fs::create_dir_all(dir.join("integration")).expect("fixture integration");
        fs::write(dir.join("src/lib.rs"), "").expect("fixture lib.rs");
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"d2b-provider-fixture-example\"\n\n\
             [dependencies]\n\
             d2b-contracts = { path = \"../d2b-contracts\" }\n\
             serde = { workspace = true }\n",
        )
        .expect("fixture Cargo.toml");
        fs::write(
            dir.join("integration/scenario.rs"),
            "//! integration-target: container\n",
        )
        .expect("fixture integration scenario");
        let fixture = Self { dir };
        fixture.write_readme(REQUIRED_SECTIONS, Some("fixture-example"));
        fixture
    }

    /// Rewrite the README with the given headings and optional identity row.
    fn write_readme(&self, sections: &[&str], identity: Option<&str>) {
        let mut text = String::from("# fixture\n\n");
        for section in sections {
            text.push_str(&format!("## {section}\n\n"));
            if *section == "Provider identity"
                && let Some(name) = identity
            {
                text.push_str("| Field | Value |\n| --- | --- |\n");
                text.push_str(&format!("| Provider name | `{name}` |\n\n"));
            }
        }
        fs::write(self.dir.join("README.md"), text).expect("fixture README.md");
    }

    fn path(&self) -> &Path {
        &self.dir
    }

    fn check(&self) -> Vec<Violation> {
        check_crate("d2b-provider-fixture-example", &self.dir)
    }
}

/// A synthetic dossier directory, removed on drop.
struct DossierFixture {
    dir: PathBuf,
}

impl Drop for DossierFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

impl DossierFixture {
    /// An empty synthetic dossier directory carrying only the index README.
    fn empty(label: &str) -> Self {
        let serial = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "d2b-provider-dossiers-policy-{}-{serial}-{label}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("fixture dossier directory");
        fs::write(dir.join("README.md"), "# index\n").expect("fixture dossier README");
        Self { dir }
    }

    /// Write a dossier file for `identity` declaring `spec_id`, or no `Spec ID`
    /// row at all when `spec_id` is `None`.
    fn write(&self, identity: &str, spec_id: Option<&str>) {
        let mut text =
            format!("# ADR 0046 Provider/{identity} dossier\n\n| Field | Value |\n| --- | --- |\n");
        if let Some(spec_id) = spec_id {
            text.push_str(&format!("| Spec ID | `{spec_id}` |\n"));
        }
        text.push_str("| Status | Accepted |\n");
        fs::write(self.dir.join(dossier_file_name(identity)), text).expect("fixture dossier");
    }

    fn path(&self) -> &Path {
        &self.dir
    }
}

// ---------------------------------------------------------------------------
// The workspace assertions
// ---------------------------------------------------------------------------
fn assert_clean(violations: Vec<Violation>) {
    assert!(
        violations.is_empty(),
        "Provider crate policy violations:\n  {}",
        violations.join("\n  ")
    );
}

/// The scope is not empty. A policy that silently matched nothing would pass
/// every other assertion vacuously, which is the failure mode this whole file
/// exists to prevent.
#[test]
fn the_provider_crate_scope_is_non_empty() {
    let crates = provider_crates();
    assert!(
        crates.len() >= 2,
        "expected at least two in-scope Provider crates, found {crates:?}"
    );
    for (name, dir) in &crates {
        assert!(dir.is_dir(), "{name}: {} is not a directory", dir.display());
    }
}

/// `provider-crate-layout-src-required` and its three siblings, on the real
/// workspace.
#[test]
fn every_provider_crate_has_the_four_required_paths() {
    let mut violations = Vec::new();
    for (name, dir) in provider_crates() {
        violations.extend(check_required_paths(&name, &dir));
    }
    assert_clean(violations);
}

/// `provider-readme-sections-all-present`, on the real workspace.
#[test]
fn every_provider_readme_has_the_nine_required_sections() {
    let mut violations = Vec::new();
    for (name, dir) in provider_crates() {
        violations.extend(check_readme_sections(&name, &dir));
    }
    assert_clean(violations);
}

/// `provider-integration-target-declared` and its two siblings, on the real
/// workspace.
#[test]
fn every_integration_file_declares_one_valid_target() {
    let mut violations = Vec::new();
    for (name, dir) in provider_crates() {
        violations.extend(check_integration_targets(&name, &dir));
    }
    assert_clean(violations);
}

/// The dependency-direction rule, on the real workspace.
#[test]
fn every_provider_crate_respects_the_dependency_direction() {
    let mut violations = Vec::new();
    for (name, dir) in provider_crates() {
        violations.extend(check_dependency_direction(&name, &dir));
    }
    assert_clean(violations);
}

/// One crate is exactly one Provider identity, on the real workspace: the
/// derived identities are distinct, and no README declares a second or
/// different one.
#[test]
fn one_crate_is_exactly_one_provider_identity() {
    let mut by_identity: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut violations = Vec::new();
    for (name, dir) in provider_crates() {
        let identity = provider_identity(&name).expect("in-scope crate has an identity");
        by_identity.entry(identity).or_default().push(name.clone());
        violations.extend(check_single_identity(&name, &dir));
    }
    for (identity, crates) in &by_identity {
        assert_eq!(
            crates.len(),
            1,
            "Provider identity '{identity}' is claimed by {crates:?}; one crate is exactly \
             one Provider"
        );
    }
    assert_clean(violations);
}

/// `provider-crate-naming-convention`: an implementation-before-base name is
/// still two segments and so is admitted by the grammar, but the workspace
/// member list is alphabetically sorted by base, and the identity a name
/// denotes is read base-first. This pins the parse so a rename cannot silently
/// re-point an identity.
#[test]
fn the_naming_convention_reads_base_before_implementation() {
    assert_eq!(
        split_provider_name("d2b-provider-volume-virtiofs"),
        Some(("volume", "virtiofs"))
    );
    assert_eq!(
        provider_identity("d2b-provider-volume-virtiofs").as_deref(),
        Some("volume-virtiofs")
    );
    // The inverted spelling denotes a different identity, so a crate carrying
    // it cannot satisfy the README identity row of the Provider it meant to be.
    assert_eq!(
        provider_identity("d2b-provider-virtiofs-volume").as_deref(),
        Some("virtiofs-volume")
    );
}

/// `provider-crate-layout-non-provider-exempt`: the check runs on no crate
/// outside the naming rule.
#[test]
fn non_provider_crates_are_exempt() {
    for name in ["d2b-core", "d2b-contracts", "d2bd", "d2b-priv-broker"] {
        assert!(!is_in_scope(name), "{name} must be out of scope");
    }
    for name in NON_PROVIDER_PREFIXED {
        assert!(
            !is_in_scope(name),
            "{name} is the SDK or the toolkit, not a Provider"
        );
    }
}

/// Every `d2b-provider-*` workspace name belongs to exactly one visible
/// classification: a non-Provider helper, one of the two recorded legacy
/// exemptions, a conforming Provider identity, or a malformed name that must
/// be rejected. Keeping the partition explicit prevents a new prefixed crate
/// from becoming silently unclassified.
#[test]
fn every_provider_prefixed_workspace_name_has_one_classification() {
    for name in workspace_members()
        .into_iter()
        .filter(|name| name.starts_with("d2b-provider"))
    {
        let non_provider = NON_PROVIDER_PREFIXED.contains(&name.as_str());
        let legacy = EXEMPT_CRATES.iter().any(|(exempt, _)| *exempt == name);
        let provider = provider_identity(&name).is_some();
        let malformed = name.starts_with("d2b-provider-")
            && !non_provider
            && !legacy
            && split_provider_name(&name).is_none();
        let classifications = [non_provider, legacy, provider, malformed]
            .into_iter()
            .filter(|classified| *classified)
            .count();
        assert_eq!(
            classifications, 1,
            "{name} must have exactly one Provider-name classification"
        );
    }
}

/// The two recorded exemptions, and only those two.
///
/// Each is asserted to still exist and to still fail the naming rule. An
/// exemption for a crate that no longer exists, or that has since been renamed
/// into conformance, is stale and must be retired rather than left standing.
#[test]
fn the_two_recorded_exemptions_are_exactly_the_naming_mismatches() {
    let packages = repo_root().join("packages");
    for (name, reason) in EXEMPT_CRATES {
        assert!(
            packages.join(name).is_dir(),
            "exempt crate {name} no longer exists; retire the exemption ({reason})"
        );
        assert!(
            split_provider_name(name).is_none(),
            "exempt crate {name} now matches <base>-<implementation>; retire the exemption"
        );
    }
    let unexpected: Vec<String> = workspace_members()
        .into_iter()
        .filter(|name| name.starts_with("d2b-provider"))
        .filter(|name| !is_in_scope(name))
        .filter(|name| !NON_PROVIDER_PREFIXED.contains(&name.as_str()))
        .filter(|name| EXEMPT_CRATES.iter().all(|(exempt, _)| exempt != name))
        .collect();
    assert!(
        unexpected.is_empty(),
        "these d2b-provider-* crates are neither in scope nor recorded as exempt: {unexpected:?}"
    );
}

// ---------------------------------------------------------------------------
// The negative cases: each rule proven to reject
// ---------------------------------------------------------------------------

#[test]
fn a_conformant_synthetic_crate_produces_no_violation() {
    let fixture = Fixture::conformant("clean");
    assert_eq!(fixture.check(), Vec::<Violation>::new());
}

#[test]
fn a_missing_required_path_is_rejected() {
    for missing in REQUIRED_PATHS {
        let fixture = Fixture::conformant("missing");
        let target = fixture.path().join(missing);
        if target.is_dir() {
            fs::remove_dir_all(&target).expect("remove fixture directory");
        } else {
            fs::remove_file(&target).expect("remove fixture file");
        }
        let violations = fixture.check();
        assert!(
            violations
                .iter()
                .any(|v| v.contains(&format!("missing required path '{missing}'"))),
            "removing {missing} must be rejected; got {violations:?}"
        );
    }
}

#[test]
fn a_readme_missing_one_of_nine_sections_names_that_section() {
    for omitted in REQUIRED_SECTIONS {
        let fixture = Fixture::conformant("section");
        let kept: Vec<&str> = REQUIRED_SECTIONS
            .iter()
            .copied()
            .filter(|section| section != omitted)
            .collect();
        fixture.write_readme(&kept, Some("fixture-example"));
        let violations = fixture.check();
        assert_eq!(
            violations,
            vec![format!(
                "d2b-provider-fixture-example/README.md: missing required section '{omitted}'"
            )],
            "omitting '{omitted}' must be rejected by name"
        );
    }
}

#[test]
fn an_integration_file_without_a_declaration_is_rejected() {
    let fixture = Fixture::conformant("nodecl");
    fs::write(
        fixture.path().join("integration/scenario.rs"),
        "// nothing declared here\nfn main() {}\n",
    )
    .expect("write scenario");
    let violations = fixture.check();
    assert!(
        violations
            .iter()
            .any(|v| v.contains("no 'integration-target:' declaration")),
        "an undeclared integration file must be rejected; got {violations:?}"
    );
}

#[test]
fn a_declaration_past_the_window_is_rejected() {
    let fixture = Fixture::conformant("window");
    let mut text = "//\n".repeat(DECLARATION_WINDOW);
    text.push_str("//! integration-target: container\n");
    fs::write(fixture.path().join("integration/scenario.rs"), text).expect("write scenario");
    let violations = fixture.check();
    assert!(
        violations
            .iter()
            .any(|v| v.contains("no 'integration-target:' declaration")),
        "a declaration past line {DECLARATION_WINDOW} must be rejected; got {violations:?}"
    );
}

#[test]
fn two_declarations_are_rejected() {
    let fixture = Fixture::conformant("dup");
    fs::write(
        fixture.path().join("integration/scenario.rs"),
        "//! integration-target: container\n//! integration-target: host-integration\n",
    )
    .expect("write scenario");
    let violations = fixture.check();
    assert!(
        violations
            .iter()
            .any(|v| v.contains("2 'integration-target:' declarations")),
        "two declarations must be rejected; got {violations:?}"
    );
}

#[test]
fn an_invalid_target_value_is_rejected_and_both_valid_values_are_accepted() {
    for valid in INTEGRATION_TARGETS {
        let fixture = Fixture::conformant("valid");
        fs::write(
            fixture.path().join("integration/scenario.rs"),
            format!("//! integration-target: {valid}\n"),
        )
        .expect("write scenario");
        assert_eq!(
            fixture.check(),
            Vec::<Violation>::new(),
            "'{valid}' is an admitted orchestration target"
        );
    }
    let fixture = Fixture::conformant("invalid");
    fs::write(
        fixture.path().join("integration/scenario.rs"),
        "//! integration-target: live\n",
    )
    .expect("write scenario");
    let violations = fixture.check();
    assert!(
        violations
            .iter()
            .any(|v| v.contains("invalid integration-target 'live'")),
        "an unknown target must be rejected; got {violations:?}"
    );
}

#[test]
fn a_dependency_on_the_daemon_broker_or_store_is_rejected() {
    for (dependency, what) in NAMED_INVERSIONS {
        let fixture = Fixture::conformant("dep");
        fs::write(
            fixture.path().join("Cargo.toml"),
            format!(
                "[package]\nname = \"d2b-provider-fixture-example\"\n\n\
                 [dependencies]\n\
                 d2b-contracts = {{ path = \"../d2b-contracts\" }}\n\
                 {dependency} = {{ path = \"../{dependency}\" }}\n"
            ),
        )
        .expect("write Cargo.toml");
        let violations = fixture.check();
        assert!(
            violations
                .iter()
                .any(|v| v.contains(&format!("depends on '{dependency}' ({what})"))),
            "a dependency on {dependency} must be rejected; got {violations:?}"
        );
    }
}

#[test]
fn a_dependency_on_a_sibling_provider_is_rejected() {
    let fixture = Fixture::conformant("sibling");
    fs::write(
        fixture.path().join("Cargo.toml"),
        "[package]\nname = \"d2b-provider-fixture-example\"\n\n\
         [dependencies]\n\
         d2b-provider-volume-local = { path = \"../d2b-provider-volume-local\" }\n",
    )
    .expect("write Cargo.toml");
    let violations = fixture.check();
    assert!(
        violations
            .iter()
            .any(|v| v.contains("depends on sibling Provider crate 'd2b-provider-volume-local'")),
        "a sibling Provider dependency must be rejected; got {violations:?}"
    );
}

#[test]
fn an_unlisted_workspace_dependency_is_rejected_and_every_admitted_one_is_accepted() {
    let fixture = Fixture::conformant("unlisted");
    fs::write(
        fixture.path().join("Cargo.toml"),
        "[package]\nname = \"d2b-provider-fixture-example\"\n\n\
         [dependencies]\n\
         d2b-guestd = { path = \"../d2b-guestd\" }\n",
    )
    .expect("write Cargo.toml");
    let violations = fixture.check();
    assert!(
        violations
            .iter()
            .any(|v| v.contains("depends on workspace crate 'd2b-guestd'")),
        "an unlisted workspace dependency must be rejected; got {violations:?}"
    );

    let admitted = Fixture::conformant("admitted");
    let mut manifest =
        String::from("[package]\nname = \"d2b-provider-fixture-example\"\n\n[dependencies]\n");
    for dependency in ALLOWED_WORKSPACE_DEPS {
        manifest.push_str(&format!(
            "{dependency} = {{ path = \"../{dependency}\" }}\n"
        ));
    }
    fs::write(admitted.path().join("Cargo.toml"), manifest).expect("write Cargo.toml");
    assert_eq!(
        admitted.check(),
        Vec::<Violation>::new(),
        "every admitted dependency must be accepted"
    );
}

#[test]
fn a_second_or_mismatched_identity_row_is_rejected() {
    let mismatched = Fixture::conformant("mismatch");
    mismatched.write_readme(REQUIRED_SECTIONS, Some("something-else"));
    let violations = mismatched.check();
    assert!(
        violations
            .iter()
            .any(|v| v.contains("declares Provider name 'something-else'")),
        "an identity that disagrees with the crate name must be rejected; got {violations:?}"
    );

    let doubled = Fixture::conformant("doubled");
    fs::write(
        doubled.path().join("README.md"),
        format!(
            "{}\n| Provider name | `fixture-example` |\n| Provider name | `fixture-other` |\n",
            REQUIRED_SECTIONS
                .iter()
                .map(|section| format!("## {section}\n"))
                .collect::<String>()
        ),
    )
    .expect("write README.md");
    let violations = doubled.check();
    assert!(
        violations
            .iter()
            .any(|v| v.contains("declares 2 Provider names")),
        "two identity rows must be rejected; got {violations:?}"
    );
}

/// The workspace-member parse actually finds the members, and finds the
/// Provider crates among them. A parser that returned an empty list would make
/// every workspace assertion above vacuous.
#[test]
fn the_workspace_member_parse_finds_the_provider_crates() {
    let members = workspace_members();
    assert!(
        members.len() > 20,
        "expected the full workspace member list, got {} entries",
        members.len()
    );
    for expected in [
        "d2b-provider-system-core",
        "d2b-provider-volume-local",
        "d2b-provider-volume-virtiofs",
    ] {
        assert!(
            members.iter().any(|name| name == expected),
            "{expected} must appear in the workspace member list"
        );
    }
}

/// The dependency parse reads a real manifest rather than matching nothing.
#[test]
fn the_dependency_parse_reads_a_real_manifest() {
    let manifest = read_repo_file("packages/d2b-provider-volume-local/Cargo.toml");
    let deps = workspace_dependencies(&manifest);
    assert!(
        deps.contains("d2b-contracts"),
        "expected d2b-contracts among the parsed workspace dependencies, got {deps:?}"
    );
}

// ---------------------------------------------------------------------------
// Dossier parity
// ---------------------------------------------------------------------------

/// The dossier directory the parity check reads is real and populated. A
/// directory that read as empty would make the crate-side assertion below fail
/// loudly rather than vacuously, but it would make the asymmetry assertion
/// vacuous, so it is pinned once here.
#[test]
fn the_dossier_directory_holds_the_frozen_provider_catalog() {
    let dossiers = repo_root().join(DOSSIER_DIR);
    let identities = dossier_identities(&dossiers);
    assert!(
        identities.len() > 20,
        "expected the frozen Provider catalog under {DOSSIER_DIR}, found {identities:?}"
    );
    for expected in ["system-core", "volume-local", "volume-virtiofs"] {
        assert!(
            identities.contains(expected),
            "{expected} must have a dossier under {DOSSIER_DIR}"
        );
    }
}

/// `ADR-046-provider-model-and-packaging` crate/package boundary: every
/// Provider crate has one `ADR-046-provider-<provider-name>.md` dossier, and
/// the dossier's declared Spec ID is the identity the crate name denotes.
#[test]
fn every_provider_crate_has_a_dossier_declaring_the_same_identity() {
    let dossiers = repo_root().join(DOSSIER_DIR);
    let mut violations = Vec::new();
    for (name, _) in provider_crates() {
        violations.extend(check_dossier_parity(&name, &dossiers));
    }
    assert_clean(violations);
}

/// The asymmetry, on the real tree: the catalog holds more dossiers than the
/// workspace holds crates, and that is not a violation. The Providers whose
/// crates land in later waves already have their normative dossiers.
#[test]
fn a_dossier_without_a_crate_is_not_a_violation() {
    let dossiers = repo_root().join(DOSSIER_DIR);
    let with_dossiers = dossier_identities(&dossiers);
    let with_crates: BTreeSet<String> = provider_crates()
        .into_iter()
        .filter_map(|(name, _)| provider_identity(&name))
        .collect();
    let unimplemented: BTreeSet<&String> = with_dossiers.difference(&with_crates).collect();
    assert!(
        !unimplemented.is_empty(),
        "expected at least one dossier whose crate is not yet implemented; without one this \
         assertion proves nothing"
    );
    // The checker is crate-driven, so those dossiers are reported by nobody.
    let mut violations = Vec::new();
    for (name, _) in provider_crates() {
        violations.extend(check_dossier_parity(&name, &dossiers));
    }
    assert_clean(violations);

    // And a synthetic directory holding only unimplemented dossiers is clean
    // for a crate that has its own, which pins the direction rather than
    // relying on the real tree's current contents.
    let fixture = DossierFixture::empty("asymmetric");
    fixture.write("fixture-example", Some("ADR-046-provider-fixture-example"));
    fixture.write("fixture-unbuilt", Some("ADR-046-provider-fixture-unbuilt"));
    assert_eq!(
        check_dossier_parity("d2b-provider-fixture-example", fixture.path()),
        Vec::<Violation>::new(),
        "a dossier with no crate must not be reported"
    );
}

#[test]
fn a_crate_without_a_dossier_is_rejected() {
    let fixture = DossierFixture::empty("missing");
    fixture.write("fixture-other", Some("ADR-046-provider-fixture-other"));
    let violations = check_dossier_parity("d2b-provider-fixture-example", fixture.path());
    assert!(
        violations
            .iter()
            .any(|v| v.contains("no Provider dossier at")),
        "a Provider crate without a dossier must be rejected; got {violations:?}"
    );
}

#[test]
fn a_dossier_declaring_a_different_identity_is_rejected() {
    let fixture = DossierFixture::empty("mismatch");
    fixture.write("fixture-example", Some("ADR-046-provider-fixture-other"));
    let violations = check_dossier_parity("d2b-provider-fixture-example", fixture.path());
    assert!(
        violations
            .iter()
            .any(|v| v.contains("declares Spec ID 'ADR-046-provider-fixture-other'")),
        "a dossier whose Spec ID disagrees with the crate must be rejected; got {violations:?}"
    );
}

#[test]
fn a_dossier_without_a_spec_id_row_is_rejected() {
    let fixture = DossierFixture::empty("nospecid");
    fixture.write("fixture-example", None);
    let violations = check_dossier_parity("d2b-provider-fixture-example", fixture.path());
    assert!(
        violations.iter().any(|v| v.contains("no 'Spec ID' row")),
        "a dossier with no declared identity must be rejected; got {violations:?}"
    );
}

#[test]
fn a_dossier_declaring_two_identities_is_rejected() {
    let fixture = DossierFixture::empty("twospecids");
    fs::write(
        fixture.path().join(dossier_file_name("fixture-example")),
        "| Field | Value |\n| --- | --- |\n\
         | Spec ID | `ADR-046-provider-fixture-example` |\n\
         | Spec ID | `ADR-046-provider-fixture-other` |\n",
    )
    .expect("write dossier");
    let violations = check_dossier_parity("d2b-provider-fixture-example", fixture.path());
    assert!(
        violations
            .iter()
            .any(|v| v.contains("declares 2 'Spec ID' rows")),
        "two Spec ID rows must be rejected; got {violations:?}"
    );
}

/// A conformant synthetic pairing produces no violation, so the rejections
/// above are the rule firing rather than the fixture being malformed.
#[test]
fn a_matching_crate_and_dossier_produce_no_violation() {
    let fixture = DossierFixture::empty("clean");
    fixture.write("fixture-example", Some("ADR-046-provider-fixture-example"));
    assert_eq!(
        check_dossier_parity("d2b-provider-fixture-example", fixture.path()),
        Vec::<Violation>::new()
    );
}

/// The exempt and non-Provider crates name no identity, so they name no
/// dossier. The exemption list stays exactly the naming mismatches.
#[test]
fn out_of_scope_crates_are_owed_no_dossier() {
    let empty = DossierFixture::empty("outofscope");
    for (name, _) in EXEMPT_CRATES {
        assert_eq!(
            check_dossier_parity(name, empty.path()),
            Vec::<Violation>::new(),
            "{name} is exempt from the naming rule and so owes no dossier"
        );
    }
    for name in NON_PROVIDER_PREFIXED {
        assert_eq!(
            check_dossier_parity(name, empty.path()),
            Vec::<Violation>::new(),
            "{name} is the SDK or the toolkit, not a Provider"
        );
    }
}

/// The dossier table parse reads a real dossier rather than matching nothing.
#[test]
fn the_dossier_spec_id_parse_reads_a_real_dossier() {
    let text = read_repo_file("docs/specs/providers/ADR-046-provider-volume-local.md");
    assert_eq!(
        table_row_values(&text, "Spec ID"),
        vec!["ADR-046-provider-volume-local".to_owned()]
    );
}
