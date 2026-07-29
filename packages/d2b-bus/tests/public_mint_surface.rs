use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use quote::ToTokens;
use syn::{Attribute, Item, Meta, Visibility, visit::Visit};

const APPROVED_CAPABILITY_MINT_POINTS: &[(&str, &str)] = &[
    (
        "d2b_bus",
        "router::ZoneRegistrar::method:component_session_acceptor",
    ),
    (
        "d2b_session_unix",
        "VerifiedUnixPeer::method:verify_seqpacket",
    ),
    ("d2b_session_unix", "VerifiedUnixPeer::method:verify_stream"),
];

const CAPABILITY_TYPE_IDENTITIES: &[&str] = &[
    "ComponentSessionAdmission",
    "AuthenticatedComponentSession",
    "SessionAcceptor",
    "SessionRegistrationCapability",
    "VerifiedUnixPeer",
];

const CLAIM_TYPE_IDENTITIES: &[&str] = &["ResourceRef", "ResourceUid"];

#[test]
fn public_api_has_only_the_approved_capability_mint_surface() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let scratch = Scratch::new(
        repository_root
            .join(".scratch")
            .join(format!("bus-public-api-{}", std::process::id())),
    );
    let temp = scratch.path().join("tmp");
    fs::create_dir_all(&temp).expect("create repository-local rustdoc scratch");

    let workspace_docs = render_workspace_docs(
        &crate_root.parent().unwrap().join("Cargo.toml"),
        scratch.path(),
        &[],
    );

    let approved = approved_entries(include_str!("approved-public-api.txt"));
    let snapshot_crates = approved
        .iter()
        .filter_map(|symbol| symbol.split_once("::").map(|(crate_name, _)| crate_name))
        .collect::<BTreeSet<_>>();
    let snapshot_docs = workspace_docs
        .iter()
        .filter(|documented| snapshot_crates.contains(documented.crate_name.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let rendered = snapshot_docs
        .iter()
        .map(|documented| documented.crate_name.as_str())
        .collect::<BTreeSet<_>>();
    let missing = snapshot_crates
        .iter()
        .filter(|crate_name| !rendered.contains(**crate_name))
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "rustdoc output is incomplete: {missing:?} appear in approved-public-api.txt \
         but were not rendered, so the comparison below would report their whole \
         API as removed. This is a doc-build problem, not an API change."
    );
    let actual = snapshot_public_api(&snapshot_docs, &approved);
    let (_, capability_surface) = workspace_public_api(&workspace_docs, &BTreeSet::new(), true);
    let hidden_public = workspace_hidden_public_api(&workspace_docs);
    let capability_trait_impls = workspace_capability_trait_impls(&workspace_docs);
    if std::env::var_os("D2B_UPDATE_BUS_PUBLIC_API").is_some() {
        write_snapshot(&crate_root.join("tests/approved-public-api.txt"), &actual);
        write_snapshot(
            &crate_root.join("tests/approved-capability-api.txt"),
            &capability_surface,
        );
        write_snapshot(
            &crate_root.join("tests/approved-hidden-public-api.txt"),
            &hidden_public,
        );
        write_snapshot(
            &crate_root.join("tests/approved-capability-trait-impls.txt"),
            &capability_trait_impls,
        );
        return;
    }
    assert_snapshot(
        &actual,
        &approved,
        "d2b-bus public API changed; review capability minting before updating \
         approved-public-api.txt with the pinned toolchain",
    );
    for (crate_name, mint) in APPROVED_CAPABILITY_MINT_POINTS {
        let mint = format!("{crate_name}::{mint}");
        assert!(
            actual.contains(&mint),
            "approved capability mint point {mint:?} is absent from the actual public API"
        );
    }
    let approved_capabilities = approved_entries(include_str!("approved-capability-api.txt"));
    assert_capability_inventory(&capability_surface, &approved_capabilities);
    let approved_hidden = approved_entries(include_str!("approved-hidden-public-api.txt"));
    assert_hidden_public_inventory(&hidden_public, &approved_hidden);
    let approved_trait_impls =
        approved_entries(include_str!("approved-capability-trait-impls.txt"));
    assert_capability_trait_impl_inventory(&capability_trait_impls, &approved_trait_impls);

    let router = fs::read_to_string(crate_root.join("src/router.rs")).expect("read router source");
    assert_eq!(
        source_occurrences(&router, "\n            ComponentSessionAdmission {"),
        1,
        "ComponentSessionAdmission must be constructed only by the approved registrar mint point"
    );
    assert_eq!(
        source_occurrences(&router, "SessionAcceptor::from_verified_adapter("),
        1,
        "SessionAcceptor construction widened beyond the approved registrar mint point"
    );
    assert_mutation_fixture(&workspace_docs);
    assert_partial_render_fails_closed(&workspace_docs);
}

#[test]
fn capability_trait_source_mutations_fail_closed() {
    let approved = approved_entries(include_str!("approved-capability-trait-impls.txt"));
    assert_trait_impl_mutations(&approved);
}

fn assert_mutation_fixture(workspace_docs: &[DocumentedCrate]) {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let fixture = crate_root.join("tests/ui/public-api-mutations");
    let scratch = Scratch::new(
        repository_root
            .join(".scratch")
            .join(format!("bus-public-api-mutations-{}", std::process::id())),
    );
    let temp = scratch.path().join("tmp");
    fs::create_dir_all(&temp).expect("create repository-local mutation scratch");
    let docs = render_workspace_docs(&fixture.join("Cargo.toml"), scratch.path(), workspace_docs);
    let mut all_docs = workspace_docs.to_vec();
    all_docs.extend(docs.iter().cloned());
    let doc_root = docs
        .iter()
        .find(|docs| docs.crate_name == "d2b_bus_public_api_mutations")
        .expect("mutation fixture documentation was rendered")
        .root
        .clone();
    let rogue_html =
        fs::read_to_string(doc_root.join("struct.Rogue.html")).expect("read Deref mutation page");
    assert!(
        rogue_html.contains("id=\"deref-methods-"),
        "mutation fixture did not execute the Deref-region parser branch"
    );
    for wrapper in ["PrincipalClaim", "SerialClaim"] {
        let item = documented_items(&docs)
            .into_iter()
            .find(|item| item.symbol.ends_with(&format!("opaque_claims::{wrapper}")))
            .unwrap_or_else(|| panic!("opaque wrapper {wrapper} was not documented"));
        let constructor = item
            .html
            .split("<section id=\"method.from_raw\"")
            .nth(1)
            .and_then(code_header)
            .unwrap_or_else(|| panic!("opaque wrapper {wrapper} has no raw-string constructor"));
        assert!(
            constructor.contains(">str</a>")
                && !constructor.contains("ResourceRef")
                && !constructor.contains("ResourceUid"),
            "opaque wrapper {wrapper} publicly exposes its private claim type: {constructor}"
        );
    }
    let (_, capabilities) = workspace_public_api(&all_docs, &BTreeSet::new(), true);
    let rogue_admission = capabilities
        .iter()
        .find(|symbol| symbol.ends_with("::rogue_admission"))
        .unwrap_or_else(|| {
            panic!("rogue public ComponentSessionAdmission factory escaped classification")
        });
    let mut mutation_approved = capabilities.clone();
    mutation_approved.remove(rogue_admission);
    let error = capability_inventory_error(&capabilities, &mutation_approved)
        .expect("rogue public ComponentSessionAdmission factory passed the inventory");
    assert!(
        error.contains(rogue_admission),
        "capability inventory did not name the rogue admission factory: {error}"
    );
    let hidden_public = workspace_hidden_public_api(&docs);
    let hidden_rogue = hidden_public
        .iter()
        .find(|symbol| symbol.contains("hidden_rogue_admission"))
        .unwrap_or_else(|| {
            panic!("hidden rogue ComponentSessionAdmission factory escaped classification")
        });
    let hidden_rogue_name = hidden_rogue
        .split_once('\t')
        .map_or(hidden_rogue.as_str(), |(name, _)| name);
    let error = hidden_public_inventory_error(&hidden_public, &BTreeSet::new())
        .expect("hidden rogue ComponentSessionAdmission factory passed the inventory");
    assert!(
        error.contains(hidden_rogue_name),
        "hidden public inventory did not name the rogue admission factory: {error}"
    );
    assert!(
        capabilities
            .iter()
            .any(|symbol| symbol.ends_with("Rogue::method:construct")),
        "constructing public trait implementation escaped the capability inventory"
    );
    assert!(
        capabilities
            .iter()
            .any(|symbol| symbol.ends_with("Rogue::method:capability")),
        "public capability accessor escaped the capability inventory"
    );
    assert!(
        capabilities
            .iter()
            .any(|symbol| symbol.ends_with("::RogueSubjectClaims")),
        "renamed opaque subject claims from another crate escaped the capability inventory"
    );
    assert!(
        capabilities
            .iter()
            .any(|symbol| symbol.ends_with("opaque_claims::PrincipalClaim"))
            && capabilities
                .iter()
                .any(|symbol| symbol.ends_with("opaque_claims::SerialClaim")),
        "opaque claim wrappers were not classified from their private field types"
    );
    assert!(
        capabilities
            .iter()
            .any(|symbol| symbol.ends_with("RogueSubjectClaims::method:inject")),
        "a public subject-claim injection method escaped the capability inventory"
    );
}

fn assert_trait_impl_mutations(approved: &BTreeSet<String>) {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let router = fs::read_to_string(crate_root.join("src/router.rs"))
        .expect("read router source for trait-implementation mutations");
    assert_trait_impl_mutation_fails(
        approved,
        &router,
        r#"
#[doc(hidden)]
impl Default for ComponentSessionAdmission {
    fn default() -> Self {
        Self {
            identity: Arc::new(ComponentSessionAdmissionIdentity),
        }
    }
}
"#,
        "Default",
    );
    assert_trait_impl_mutation_fails(
        approved,
        &router,
        r#"
impl From<Arc<ComponentSessionAdmissionIdentity>> for ComponentSessionAdmission {
    fn from(identity: Arc<ComponentSessionAdmissionIdentity>) -> Self {
        Self { identity }
    }
}
"#,
        "From<Arc<ComponentSessionAdmissionIdentity>>",
    );
    let fixture = crate_root.join("tests/ui/public-api-mutations");
    for (name, expected_trait) in [
        ("trait-impl-type-alias.rs", "Default"),
        ("trait-impl-renamed-import.rs", "Default"),
        ("trait-impl-cfg-attr-derive.rs", "Clone"),
        ("trait-impl-nested-cfg-attr-derive.rs", "Copy"),
        ("trait-impl-cfg-attr-gated.rs", "Default"),
    ] {
        let source = fs::read_to_string(fixture.join(name))
            .unwrap_or_else(|error| panic!("read {name} mutation fixture: {error}"));
        assert_trait_impl_source_fails(approved, &source, name, expected_trait);
    }
    for (name, expected_diagnostic, expected_alias) in [
        ("trait-impl-generic-alias.rs", "generic", "AdmissionAlias"),
        ("trait-impl-cfg-alias.rs", "cfg-gated", "AdmissionAlias"),
        (
            "trait-impl-unsupported-alias.rs",
            "unsupported",
            "AdmissionAlias",
        ),
        ("trait-impl-renamed-module.rs", "module alias", "cap"),
        ("trait-impl-direct-renamed-module.rs", "module alias", "cap"),
        (
            "trait-impl-lexical-alias.rs",
            "lexically scoped",
            "AdmissionAlias",
        ),
    ] {
        let source = fs::read_to_string(fixture.join(name))
            .unwrap_or_else(|error| panic!("read {name} mutation fixture: {error}"));
        assert_trait_impl_source_scan_fails_closed(
            &source,
            name,
            expected_diagnostic,
            expected_alias,
        );
    }
    assert_module_source_mutations_fail_closed(&crate_root);
}

fn assert_trait_impl_mutation_fails(
    approved: &BTreeSet<String>,
    source: &str,
    mutation: &str,
    expected_trait: &str,
) {
    let mut mutated_source = source.to_owned();
    mutated_source.push_str(mutation);
    assert_trait_impl_source_fails(
        approved,
        &mutated_source,
        "inline router mutation",
        expected_trait,
    );
}

fn assert_trait_impl_source_fails(
    approved: &BTreeSet<String>,
    source: &str,
    source_name: &str,
    expected_trait: &str,
) {
    let inventory = source_capability_inventory_from_text("d2b_bus", source, source_name);
    let mut mutated = approved.clone();
    mutated.extend(inventory.trait_impls);
    let error = capability_trait_impl_inventory_error(&mutated, approved)
        .unwrap_or_else(|| panic!("{expected_trait} capability trait implementation passed"));
    assert!(
        error.contains("ComponentSessionAdmission") && error.contains(expected_trait),
        "trait-implementation inventory did not name the rogue {expected_trait} \
         implementation: {error}"
    );
}

fn assert_trait_impl_source_scan_fails_closed(
    source: &str,
    source_name: &str,
    expected_diagnostic: &str,
    expected_alias: &str,
) {
    let failure = match std::panic::catch_unwind(|| {
        source_capability_inventory_from_text("d2b_bus", source, source_name)
    }) {
        Ok(_) => panic!("unresolvable capability alias passed the source inventory"),
        Err(failure) => failure,
    };
    let diagnostic = failure
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| failure.downcast_ref::<&str>().copied())
        .unwrap_or("<non-string panic>");
    assert!(
        diagnostic.contains(expected_diagnostic)
            && diagnostic.contains(expected_alias)
            && diagnostic.contains(source_name),
        "fail-closed alias diagnostic did not identify {source_name} as \
         {expected_diagnostic} through {expected_alias}: {diagnostic}"
    );
}

fn assert_module_source_mutations_fail_closed(crate_root: &Path) {
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let scratch = Scratch::new(repository_root.join(".scratch").join(format!(
        "bus-source-module-mutations-{}",
        std::process::id()
    )));

    let cfg_attr = scratch.path().join("cfg-attr-path");
    fs::create_dir_all(&cfg_attr).expect("create cfg_attr module mutation");
    fs::write(
        cfg_attr.join("lib.rs"),
        "#[cfg_attr(all(), path = \"rogue.rs\")]\nmod router;\n",
    )
    .expect("write cfg_attr module mutation root");
    fs::write(cfg_attr.join("router.rs"), "struct Harmless;\n")
        .expect("write default module source");
    fs::write(
        cfg_attr.join("rogue.rs"),
        "struct ComponentSessionAdmission;\n",
    )
    .expect("write compiler-selected module source");
    assert_source_file_scan_fails_closed(&cfg_attr.join("lib.rs"), &["cfg_attr", "router"]);

    let direct_path = scratch.path().join("direct-path");
    fs::create_dir_all(direct_path.join("router")).expect("create direct path mutation");
    fs::write(direct_path.join("lib.rs"), "mod router;\n")
        .expect("write direct path mutation root");
    fs::write(
        direct_path.join("router.rs"),
        r#"#[path = "rogue.rs"]
mod selected;

#[path = "inline-selected"]
mod inline {
    #[path = "nested.rs"]
    mod nested;
}
"#,
    )
    .expect("write direct path module declaration");
    fs::write(
        direct_path.join("rogue.rs"),
        r#"
pub struct ComponentSessionAdmission;

#[doc(hidden)]
impl From<()> for ComponentSessionAdmission {
    fn from(_value: ()) -> Self {
        Self
    }
}

#[doc(hidden)]
pub fn compiler_selected_source() {}
"#,
    )
    .expect("write compiler-selected direct path source");
    fs::write(
        direct_path.join("router/rogue.rs"),
        "#[doc(hidden)]\npub fn decoy_source() {}\n",
    )
    .expect("write direct path decoy source");
    fs::create_dir_all(direct_path.join("inline-selected"))
        .expect("create compiler-selected inline module directory");
    fs::write(
        direct_path.join("inline-selected/nested.rs"),
        "#[doc(hidden)]\npub fn inline_compiler_selected_source() {}\n",
    )
    .expect("write compiler-selected inline path source");
    fs::create_dir_all(direct_path.join("router/inline"))
        .expect("create inline path decoy directory");
    fs::write(
        direct_path.join("router/inline/nested.rs"),
        "#[doc(hidden)]\npub fn inline_decoy_source() {}\n",
    )
    .expect("write inline path decoy source");
    let capability_inventory = source_capability_inventory("d2b_bus", &direct_path.join("lib.rs"));
    assert!(
        capability_inventory
            .trait_impls
            .iter()
            .any(|implementation| {
                implementation.contains("ComponentSessionAdmission")
                    && implementation.contains("From<()>")
            }),
        "direct #[path] scan did not inspect the compiler-selected source: {:?}",
        capability_inventory.trait_impls
    );
    let hidden_inventory = hidden_public_api("d2b_bus", &direct_path.join("lib.rs"));
    assert!(
        hidden_inventory
            .iter()
            .any(|symbol| symbol.contains("::selected::compiler_selected_source\t"))
            && hidden_inventory.iter().any(|symbol| {
                symbol.contains("::inline::nested::inline_compiler_selected_source\t")
            })
            && hidden_inventory
                .iter()
                .all(|symbol| !symbol.contains("decoy_source")),
        "direct #[path] hidden-public scan selected the decoy instead of the compiler source: \
         {hidden_inventory:?}"
    );

    let duplicate = scratch.path().join("duplicate-logical-path");
    fs::create_dir_all(&duplicate).expect("create duplicate module-path mutation");
    fs::write(
        duplicate.join("lib.rs"),
        "#[path = \"shared.rs\"]\nmod first;\n#[path = \"shared.rs\"]\nmod second;\n",
    )
    .expect("write duplicate module-path mutation root");
    fs::write(
        duplicate.join("shared.rs"),
        "#[doc(hidden)]\npub struct ComponentSessionAdmission;\n",
    )
    .expect("write shared module source");
    let capability_diagnostic = assert_source_file_scan_fails_closed(
        &duplicate.join("lib.rs"),
        &["d2b_bus::first", "d2b_bus::second", "shared.rs"],
    );
    let hidden_diagnostic = assert_hidden_source_file_scan_fails_closed(
        &duplicate.join("lib.rs"),
        &["d2b_bus::first", "d2b_bus::second", "shared.rs"],
    );
    let canonical_duplicate = fs::canonicalize(&duplicate)
        .expect("canonicalize duplicate module mutation")
        .display()
        .to_string();
    for diagnostic in [capability_diagnostic, hidden_diagnostic] {
        assert!(
            !diagnostic.contains(&canonical_duplicate),
            "module-source diagnostic leaked its canonical workspace path: {diagnostic}"
        );
    }
}

fn assert_source_file_scan_fails_closed(source: &Path, expected: &[&str]) -> String {
    let failure = match std::panic::catch_unwind(|| source_capability_inventory("d2b_bus", source))
    {
        Ok(_) => panic!("ambiguous module source passed the capability inventory"),
        Err(failure) => failure,
    };
    let diagnostic = failure
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| failure.downcast_ref::<&str>().copied())
        .unwrap_or("<non-string panic>");
    for expected in expected {
        assert!(
            diagnostic.contains(expected),
            "module-source diagnostic did not contain {expected:?}: {diagnostic}"
        );
    }
    diagnostic.to_owned()
}

fn assert_hidden_source_file_scan_fails_closed(source: &Path, expected: &[&str]) -> String {
    let failure = match std::panic::catch_unwind(|| hidden_public_api("d2b_bus", source)) {
        Ok(_) => panic!("ambiguous module source passed the hidden-public inventory"),
        Err(failure) => failure,
    };
    let diagnostic = failure
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| failure.downcast_ref::<&str>().copied())
        .unwrap_or("<non-string panic>");
    for expected in expected {
        assert!(
            diagnostic.contains(expected),
            "hidden module-source diagnostic did not contain {expected:?}: {diagnostic}"
        );
    }
    diagnostic.to_owned()
}

fn assert_partial_render_fails_closed(docs: &[DocumentedCrate]) {
    let documented = docs
        .iter()
        .find(|documented| !documented.advertised.is_empty())
        .expect("workspace rustdoc has an advertised item");
    let advertised = documented
        .advertised
        .first()
        .expect("selected rustdoc crate has an advertised item");
    fs::remove_file(documented.root.join(&advertised.href))
        .expect("remove one advertised item to simulate a partial rustdoc render");

    let error = validate_documented_crate(&documented.crate_name, &documented.root)
        .expect_err("partial rustdoc render passed completeness validation");
    let symbol = format!("{}::{}", documented.crate_name, advertised.name);
    assert!(
        error.contains(&symbol) && error.contains("doc-build problem"),
        "partial-render failure did not name the missing advertised item: {error}"
    );
}

struct Scratch(PathBuf);

impl Scratch {
    fn new(path: PathBuf) -> Self {
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale repository-local scratch");
        }
        fs::create_dir_all(&path).expect("create repository-local scratch");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn source_occurrences(source: &str, needle: &str) -> usize {
    source.match_indices(needle).count()
}

fn write_snapshot(path: &Path, entries: &BTreeSet<String>) {
    let mut rendered = entries.iter().cloned().collect::<Vec<_>>().join("\n");
    rendered.push('\n');
    fs::write(path, rendered).expect("write reviewed public API snapshot");
}

fn assert_snapshot(actual: &BTreeSet<String>, approved: &BTreeSet<String>, message: &str) {
    if actual == approved {
        return;
    }
    let added = actual.difference(approved).take(40).collect::<Vec<_>>();
    let removed = approved.difference(actual).take(40).collect::<Vec<_>>();
    panic!(
        "{message}; added {} (first 40: {added:?}), removed {} (first 40: {removed:?})",
        actual.difference(approved).count(),
        approved.difference(actual).count()
    );
}

fn assert_capability_inventory(actual: &BTreeSet<String>, approved: &BTreeSet<String>) {
    if let Some(error) = capability_inventory_error(actual, approved) {
        panic!("{error}");
    }
}

fn capability_inventory_error(
    actual: &BTreeSet<String>,
    approved: &BTreeSet<String>,
) -> Option<String> {
    let unapproved = actual.difference(approved).take(40).collect::<Vec<_>>();
    let missing = approved
        .iter()
        .filter(|entry| !actual.contains(*entry))
        .take(40)
        .collect::<Vec<_>>();
    if !unapproved.is_empty() {
        return Some(format!(
            "a public signature now exposes a capability or claim type outside the \
             explicitly approved capability API; unapproved {} (first 40: \
             {unapproved:?}). Review whether this widens the capability mint \
             surface before adding it to approved-capability-api.txt.",
            actual.difference(approved).count()
        ));
    }
    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "{} required approved capability entries are absent from the complete \
             rustdoc inventory (first 40: {missing:?}); review whether the API was \
             intentionally removed before updating approved-capability-api.txt",
            approved
                .iter()
                .filter(|entry| !actual.contains(*entry))
                .count()
        ))
    }
}

fn approved_entries(snapshot: &str) -> BTreeSet<String> {
    snapshot
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[derive(Debug, Clone)]
struct DocumentedCrate {
    crate_name: String,
    root: PathBuf,
    advertised: Vec<AdvertisedItem>,
    hidden_public: BTreeSet<String>,
    capability_declarations: BTreeMap<String, BTreeSet<String>>,
    capability_trait_impls: BTreeSet<String>,
}

#[derive(Debug, Clone)]
struct AdvertisedItem {
    href: PathBuf,
    name: String,
}

#[derive(Debug)]
struct RenderPackage {
    crate_name: String,
    source: PathBuf,
    dependency_features: BTreeSet<String>,
    workspace_dependencies: BTreeSet<String>,
}

#[derive(Debug)]
struct DocumentedItem {
    crate_name: String,
    symbol: String,
    path: PathBuf,
    html: String,
    rendered_html: String,
}

fn render_workspace_docs(
    manifest: &Path,
    scratch: &Path,
    external_docs: &[DocumentedCrate],
) -> Vec<DocumentedCrate> {
    let metadata = Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--quiet",
            "--locked",
            "--format-version",
            "1",
            "--no-deps",
            "--manifest-path",
        ])
        .arg(manifest)
        .output()
        .expect("discover workspace library crates");
    assert!(
        metadata.status.success(),
        "cargo metadata failed:\n{}",
        String::from_utf8_lossy(&metadata.stderr)
    );
    let metadata: serde_json::Value =
        serde_json::from_slice(&metadata.stdout).expect("parse cargo metadata");
    let workspace_members = metadata["workspace_members"]
        .as_array()
        .expect("workspace_members array")
        .iter()
        .map(|member| member.as_str().expect("workspace member id"))
        .collect::<BTreeSet<_>>();
    let workspace_packages = metadata["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .filter(|package| workspace_members.contains(package["id"].as_str().expect("package id")))
        .collect::<Vec<_>>();
    let workspace_package_names = workspace_packages
        .iter()
        .map(|package| package["name"].as_str().expect("workspace package name"))
        .collect::<BTreeSet<_>>();
    let mut unified_dependency_features = BTreeMap::<&str, BTreeSet<&str>>::new();
    for package in &workspace_packages {
        for dependency in package["dependencies"]
            .as_array()
            .expect("package dependencies")
            .iter()
            .filter(|dependency| dependency["kind"].is_null())
        {
            let dependency_name = dependency["name"].as_str().expect("dependency name");
            unified_dependency_features
                .entry(dependency_name)
                .or_default()
                .extend(
                    dependency["features"]
                        .as_array()
                        .expect("dependency features")
                        .iter()
                        .map(|feature| feature.as_str().expect("dependency feature")),
                );
        }
    }
    let mut packages = BTreeMap::new();
    for package in workspace_packages {
        let library = package["targets"]
            .as_array()
            .expect("package targets")
            .iter()
            .find(|target| {
                target["kind"]
                    .as_array()
                    .expect("target kind")
                    .iter()
                    .any(|kind| kind == "lib" || kind == "rlib")
            });
        if let Some(library) = library {
            let crate_name = library["name"].as_str().expect("library target name");
            let mut dependency_features = BTreeSet::new();
            let mut workspace_dependencies = BTreeSet::new();
            for dependency in package["dependencies"]
                .as_array()
                .expect("package dependencies")
                .iter()
                .filter(|dependency| dependency["kind"].is_null())
            {
                let dependency_name = dependency["name"].as_str().expect("dependency name");
                let command_name = dependency["rename"].as_str().unwrap_or(dependency_name);
                if let Some(features) = unified_dependency_features.get(dependency_name) {
                    dependency_features.extend(
                        features
                            .iter()
                            .map(|feature| format!("{command_name}/{feature}")),
                    );
                }
                if workspace_package_names.contains(dependency_name) {
                    workspace_dependencies.insert(dependency_name.to_owned());
                }
            }
            packages.insert(
                package["name"]
                    .as_str()
                    .expect("workspace package name")
                    .to_owned(),
                RenderPackage {
                    crate_name: crate_name.to_owned(),
                    source: PathBuf::from(
                        library["src_path"]
                            .as_str()
                            .expect("library target source path"),
                    ),
                    dependency_features,
                    workspace_dependencies,
                },
            );
        }
    }
    assert!(!packages.is_empty(), "workspace has no library crates");

    let temp = scratch.join("tmp");
    fs::create_dir_all(&temp).expect("create rustdoc temporary directory");
    let build = scratch.join("build");
    let mut docs = Vec::new();
    for (package_name, package) in dependency_order(packages) {
        let crate_name = package.crate_name;
        let hidden_public = hidden_public_api(&crate_name, &package.source);
        let source_capabilities = source_capability_inventory(&crate_name, &package.source);
        let target = scratch.join("renders").join(&crate_name);
        let doc_root = target.join("doc");
        fs::create_dir_all(&doc_root).unwrap_or_else(|error| {
            panic!("create isolated rustdoc output for package {package_name}: {error}")
        });
        for documented in external_docs.iter().chain(docs.iter()) {
            let link = doc_root.join(&documented.crate_name);
            if link.exists() {
                continue;
            }
            std::os::unix::fs::symlink(&documented.root, &link).unwrap_or_else(|error| {
                panic!(
                    "link dependency rustdoc root {} -> {}: {error}",
                    link.display(),
                    documented.root.display()
                )
            });
        }
        let mut command = Command::new(env!("CARGO"));
        command
            .args([
                "doc",
                "--quiet",
                "--locked",
                "--no-deps",
                "--document-private-items",
                "--manifest-path",
            ])
            .arg(manifest)
            .arg("-p")
            .arg(&package_name);
        if !package.dependency_features.is_empty() {
            command.arg("--features").arg(
                package
                    .dependency_features
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(","),
            );
        }
        let output = command
            .arg("--target-dir")
            .arg(&target)
            .env("CARGO_BUILD_BUILD_DIR", &build)
            .env("TMPDIR", &temp)
            .output()
            .unwrap_or_else(|error| {
                panic!("render public API for package {package_name}: {error}")
            });
        assert!(
            output.status.success(),
            "rustdoc failed for package {package_name}:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );

        let root = target.join("doc").join(&crate_name);
        let advertised =
            validate_documented_crate(&crate_name, &root).unwrap_or_else(|error| panic!("{error}"));
        docs.push(DocumentedCrate {
            crate_name,
            root,
            advertised,
            hidden_public,
            capability_declarations: source_capabilities.declarations,
            capability_trait_impls: source_capabilities.trait_impls,
        });
    }
    docs.sort_by(|left, right| left.crate_name.cmp(&right.crate_name));
    docs
}

fn workspace_hidden_public_api(docs: &[DocumentedCrate]) -> BTreeSet<String> {
    docs.iter()
        .flat_map(|documented| documented.hidden_public.iter().cloned())
        .collect()
}

fn workspace_capability_trait_impls(docs: &[DocumentedCrate]) -> BTreeSet<String> {
    let mut declarations = BTreeMap::<String, BTreeSet<String>>::new();
    let mut trait_impls = BTreeSet::new();
    for documented in docs {
        for (identity, locations) in &documented.capability_declarations {
            declarations
                .entry(identity.clone())
                .or_default()
                .extend(locations.iter().cloned());
        }
        trait_impls.extend(documented.capability_trait_impls.iter().cloned());
    }
    for identity in CAPABILITY_TYPE_IDENTITIES {
        let locations = declarations.get(*identity).cloned().unwrap_or_default();
        assert_eq!(
            locations.len(),
            1,
            "capability source identity {identity:?} must have exactly one \
             declaration across workspace library sources, found {locations:?}"
        );
    }
    let unexpected = declarations
        .keys()
        .filter(|identity| !CAPABILITY_TYPE_IDENTITIES.contains(&identity.as_str()))
        .collect::<Vec<_>>();
    assert!(
        unexpected.is_empty(),
        "unexpected capability source identities: {unexpected:?}"
    );
    trait_impls
}

fn assert_capability_trait_impl_inventory(actual: &BTreeSet<String>, approved: &BTreeSet<String>) {
    if let Some(error) = capability_trait_impl_inventory_error(actual, approved) {
        panic!("{error}");
    }
}

fn capability_trait_impl_inventory_error(
    actual: &BTreeSet<String>,
    approved: &BTreeSet<String>,
) -> Option<String> {
    let unapproved = actual.difference(approved).take(40).collect::<Vec<_>>();
    let missing = approved
        .iter()
        .filter(|entry| !actual.contains(*entry))
        .take(40)
        .collect::<Vec<_>>();
    if !unapproved.is_empty() {
        return Some(format!(
            "a trait implementation on a capability type is outside the \
             explicitly approved inventory; unapproved {} (first 40: \
             {unapproved:?}). Review whether this trait can mint, clone, or \
             otherwise widen the capability before updating \
             approved-capability-trait-impls.txt.",
            actual.difference(approved).count()
        ));
    }
    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "{} approved capability trait implementations are absent (first 40: \
             {missing:?}); review whether each implementation was intentionally \
             removed before updating approved-capability-trait-impls.txt",
            approved
                .iter()
                .filter(|entry| !actual.contains(*entry))
                .count()
        ))
    }
}

fn assert_hidden_public_inventory(actual: &BTreeSet<String>, approved: &BTreeSet<String>) {
    if let Some(error) = hidden_public_inventory_error(actual, approved) {
        panic!("{error}");
    }
}

fn hidden_public_inventory_error(
    actual: &BTreeSet<String>,
    approved: &BTreeSet<String>,
) -> Option<String> {
    let unapproved = actual.difference(approved).take(40).collect::<Vec<_>>();
    let missing = approved
        .iter()
        .filter(|entry| !actual.contains(*entry))
        .take(40)
        .collect::<Vec<_>>();
    if !unapproved.is_empty() {
        return Some(format!(
            "a public doc(hidden) signature is outside the reviewed hidden API; \
                 unapproved {} (first 40: {unapproved:?}). This inventory is required \
                 because the pinned stable rustdoc does not render hidden items.",
            actual.difference(approved).count()
        ));
    }
    if missing.is_empty() {
        None
    } else {
        Some(format!(
            "{} reviewed public doc(hidden) signatures are absent (first 40: \
                 {missing:?}); review whether the API was intentionally removed",
            approved
                .iter()
                .filter(|entry| !actual.contains(*entry))
                .count()
        ))
    }
}

fn hidden_public_api(crate_name: &str, source: &Path) -> BTreeSet<String> {
    let source_root = source
        .parent()
        .expect("library target source has a parent directory")
        .to_path_buf();
    let mut scanner = HiddenPublicScanner {
        crate_name,
        source_root,
        entries: BTreeSet::new(),
        visited: BTreeMap::new(),
    };
    scanner.scan_file(source, &[], true, false);
    scanner.entries
}

// This syntax-level inventory supplies breadth beyond the compiler-checked
// negative bounds on the enumerated minting traits. It resolves the source
// forms it models and fails closed on aliases, module paths, or logical source
// paths it cannot classify with confidence. Macro expansion and include
// expansion remain compiler responsibilities and can escape this breadth scan.
// The primary boundary remains construction through private types, private
// fields, sealed traits, and consumed capabilities.
#[derive(Default)]
struct SourceCapabilityInventory {
    declarations: BTreeMap<String, BTreeSet<String>>,
    trait_impls: BTreeSet<String>,
}

fn source_capability_inventory(crate_name: &str, source: &Path) -> SourceCapabilityInventory {
    let source_root = source
        .parent()
        .expect("library target source has a parent directory")
        .to_path_buf();
    let mut scanner = CapabilitySourceScanner {
        crate_name,
        source_root,
        facts: SourceCapabilityFacts::default(),
        visited: BTreeMap::new(),
    };
    scanner.scan_file(source, &[], true);
    scanner.finish()
}

fn source_capability_inventory_from_text(
    crate_name: &str,
    text: &str,
    source: &str,
) -> SourceCapabilityInventory {
    let file = syn::parse_file(text)
        .unwrap_or_else(|error| panic!("parse Rust mutation source {source}: {error}"));
    let mut facts = SourceCapabilityFacts::default();
    CapabilitySourceCollector {
        source: source.to_owned(),
        module_path: Vec::new(),
        lexical_depth: 0,
        facts: &mut facts,
    }
    .visit_file(&file);
    finish_source_capability_inventory(crate_name, facts)
}

struct CapabilitySourceScanner<'a> {
    crate_name: &'a str,
    source_root: PathBuf,
    facts: SourceCapabilityFacts,
    visited: BTreeMap<PathBuf, Vec<String>>,
}

impl CapabilitySourceScanner<'_> {
    fn scan_file(&mut self, source: &Path, module_path: &[String], crate_root: bool) {
        let logical_source =
            source_location(self.crate_name, &self.source_root, source, module_path);
        let source = fs::canonicalize(source)
            .unwrap_or_else(|error| panic!("canonicalize Rust source {logical_source}: {error}"));
        if let Some(previous) = self.visited.get(&source) {
            assert_eq!(
                previous,
                module_path,
                "Rust source {logical_source} is reachable as both {} and {}; capability trait \
                 inventory refuses an ambiguous logical module path",
                display_module(self.crate_name, previous),
                display_module(self.crate_name, module_path)
            );
            return;
        }
        self.visited.insert(source.clone(), module_path.to_vec());
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("read Rust source {logical_source}: {error}"));
        let file = syn::parse_file(&text)
            .unwrap_or_else(|error| panic!("parse Rust source {logical_source}: {error}"));
        CapabilitySourceCollector {
            source: logical_source,
            module_path: module_path.to_vec(),
            lexical_depth: 0,
            facts: &mut self.facts,
        }
        .visit_file(&file);
        let module_dir = source_module_dir(&source, crate_root);
        let path_base = source
            .parent()
            .expect("canonical Rust source has a parent directory");
        self.scan_external_modules(&file.items, module_path, &module_dir, path_base);
    }

    fn scan_external_modules(
        &mut self,
        items: &[Item],
        module_path: &[String],
        module_dir: &Path,
        path_base: &Path,
    ) {
        for item in items {
            let Item::Mod(module) = item else {
                continue;
            };
            let child_dir = module_dir.join(module.ident.to_string());
            let mut child_path = module_path.to_vec();
            child_path.push(module.ident.to_string());
            if let Some((_, items)) = &module.content {
                let inline_dir =
                    module_path_override(module, path_base).unwrap_or_else(|| child_dir.clone());
                self.scan_external_modules(items, &child_path, &inline_dir, &inline_dir);
            } else {
                let source = module_source(module, module_dir, path_base, &child_dir)
                    .unwrap_or_else(|| {
                        panic!(
                            "cannot resolve Rust module {}::{}; capability trait inventory \
                             refuses a partial source scan",
                            self.crate_name,
                            child_path.join("::")
                        )
                    });
                self.scan_file(&source, &child_path, false);
            }
        }
    }

    fn finish(self) -> SourceCapabilityInventory {
        finish_source_capability_inventory(self.crate_name, self.facts)
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct SourceBinding {
    module_path: Vec<String>,
    name: String,
}

#[derive(Clone)]
struct SourcePath {
    leading_colon: bool,
    segments: Vec<String>,
}

#[derive(Clone)]
enum SourceAliasTarget {
    Path(SourcePath),
    Unsupported(String),
}

#[derive(Clone)]
struct SourceAlias {
    binding: SourceBinding,
    target: SourceAliasTarget,
    generic: bool,
    conditional: bool,
    fail_if_conditional: bool,
    fail_if_unresolved: bool,
    lexical_scope: bool,
    source: String,
}

#[derive(Clone)]
struct SourceModuleAlias {
    binding: SourceBinding,
    target: SourcePath,
    conservative: bool,
}

#[derive(Clone)]
struct SourceGlob {
    module_path: Vec<String>,
    target: SourcePath,
    conditional: bool,
    source: String,
}

#[derive(Clone)]
struct SourceDeclaration {
    identity: syn::Ident,
    kind: &'static str,
    attributes: Vec<Attribute>,
    module_path: Vec<String>,
    source: String,
}

#[derive(Clone)]
struct SourceImpl {
    implementation: syn::ItemImpl,
    module_path: Vec<String>,
    source: String,
}

#[derive(Default)]
struct SourceCapabilityFacts {
    aliases: Vec<SourceAlias>,
    declarations: Vec<SourceDeclaration>,
    globs: Vec<SourceGlob>,
    implementations: Vec<SourceImpl>,
    module_aliases: Vec<SourceModuleAlias>,
}

struct CapabilitySourceCollector<'a> {
    source: String,
    module_path: Vec<String>,
    lexical_depth: usize,
    facts: &'a mut SourceCapabilityFacts,
}

impl CapabilitySourceCollector<'_> {
    fn record_declaration(
        &mut self,
        identity: &syn::Ident,
        kind: &'static str,
        attributes: &[Attribute],
    ) {
        self.facts.declarations.push(SourceDeclaration {
            identity: identity.clone(),
            kind,
            attributes: attributes.to_vec(),
            module_path: self.module_path.clone(),
            source: self.source.clone(),
        });
    }

    fn record_impl(&mut self, implementation: &syn::ItemImpl) {
        self.facts.implementations.push(SourceImpl {
            implementation: implementation.clone(),
            module_path: self.module_path.clone(),
            source: self.source.clone(),
        });
    }

    fn record_type_alias(&mut self, alias: &syn::ItemType) {
        let target = match alias.ty.as_ref() {
            syn::Type::Path(path) if path.qself.is_none() => {
                SourceAliasTarget::Path(source_path(&path.path))
            }
            other => SourceAliasTarget::Unsupported(compact_tokens(other)),
        };
        self.facts.aliases.push(SourceAlias {
            binding: SourceBinding {
                module_path: self.module_path.clone(),
                name: alias.ident.to_string(),
            },
            target,
            generic: !alias.generics.params.is_empty() || alias.generics.where_clause.is_some(),
            conditional: conditional_attributes(&alias.attrs),
            fail_if_conditional: true,
            fail_if_unresolved: true,
            lexical_scope: self.lexical_depth > 0,
            source: self.source.clone(),
        });
    }

    fn record_use(&mut self, item: &syn::ItemUse) {
        let mut prefix = Vec::new();
        collect_use_bindings(
            &item.tree,
            item.leading_colon.is_some(),
            &mut prefix,
            &self.module_path,
            SourceUseContext {
                conditional: conditional_attributes(&item.attrs),
                lexical_scope: self.lexical_depth > 0,
            },
            &self.source,
            self.facts,
        );
    }
}

impl<'ast> Visit<'ast> for CapabilitySourceCollector<'_> {
    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.record_declaration(&item.ident, "struct", &item.attrs);
        syn::visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.record_declaration(&item.ident, "enum", &item.attrs);
        syn::visit::visit_item_enum(self, item);
    }

    fn visit_item_union(&mut self, item: &'ast syn::ItemUnion) {
        self.record_declaration(&item.ident, "union", &item.attrs);
        syn::visit::visit_item_union(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.record_declaration(&item.ident, "trait", &item.attrs);
        syn::visit::visit_item_trait(self, item);
    }

    fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
        self.record_impl(item);
        syn::visit::visit_item_impl(self, item);
    }

    fn visit_item_type(&mut self, item: &'ast syn::ItemType) {
        self.record_type_alias(item);
        syn::visit::visit_item_type(self, item);
    }

    fn visit_item_use(&mut self, item: &'ast syn::ItemUse) {
        self.record_use(item);
        syn::visit::visit_item_use(self, item);
    }

    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let Some((_, items)) = &item.content else {
            return;
        };
        self.module_path.push(item.ident.to_string());
        for item in items {
            self.visit_item(item);
        }
        self.module_path.pop();
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        self.lexical_depth += 1;
        syn::visit::visit_block(self, block);
        self.lexical_depth -= 1;
    }
}

fn source_path(path: &syn::Path) -> SourcePath {
    SourcePath {
        leading_colon: path.leading_colon.is_some(),
        segments: path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect(),
    }
}

#[derive(Clone, Copy)]
struct SourceUseContext {
    conditional: bool,
    lexical_scope: bool,
}

fn collect_use_bindings(
    tree: &syn::UseTree,
    leading_colon: bool,
    prefix: &mut Vec<String>,
    module_path: &[String],
    context: SourceUseContext,
    source: &str,
    facts: &mut SourceCapabilityFacts,
) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            collect_use_bindings(
                &path.tree,
                leading_colon,
                prefix,
                module_path,
                context,
                source,
                facts,
            );
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            if name.ident == "self" {
                return;
            }
            let mut segments = prefix.clone();
            segments.push(name.ident.to_string());
            facts.aliases.push(SourceAlias {
                binding: SourceBinding {
                    module_path: module_path.to_vec(),
                    name: name.ident.to_string(),
                },
                target: SourceAliasTarget::Path(SourcePath {
                    leading_colon,
                    segments,
                }),
                generic: false,
                conditional: context.conditional,
                fail_if_conditional: false,
                fail_if_unresolved: false,
                lexical_scope: context.lexical_scope,
                source: source.to_owned(),
            });
        }
        syn::UseTree::Rename(rename) => {
            let binding = SourceBinding {
                module_path: module_path.to_vec(),
                name: rename.rename.to_string(),
            };
            let mut segments = prefix.clone();
            if rename.ident != "self" {
                segments.push(rename.ident.to_string());
            }
            let target = SourcePath {
                leading_colon,
                segments,
            };
            facts.module_aliases.push(SourceModuleAlias {
                binding: binding.clone(),
                target: target.clone(),
                conservative: rename.ident == "self",
            });
            if rename.ident == "self" {
                return;
            }
            facts.aliases.push(SourceAlias {
                binding,
                target: SourceAliasTarget::Path(target),
                generic: false,
                conditional: context.conditional,
                fail_if_conditional: true,
                fail_if_unresolved: true,
                lexical_scope: context.lexical_scope,
                source: source.to_owned(),
            });
        }
        syn::UseTree::Glob(_) => facts.globs.push(SourceGlob {
            module_path: module_path.to_vec(),
            target: SourcePath {
                leading_colon,
                segments: prefix.clone(),
            },
            conditional: context.conditional,
            source: source.to_owned(),
        }),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_bindings(
                    item,
                    leading_colon,
                    prefix,
                    module_path,
                    context,
                    source,
                    facts,
                );
            }
        }
    }
}

fn conditional_attributes(attributes: &[Attribute]) -> bool {
    attributes
        .iter()
        .any(|attribute| attribute.path().is_ident("cfg") || attribute.path().is_ident("cfg_attr"))
}

fn finish_source_capability_inventory(
    crate_name: &str,
    facts: SourceCapabilityFacts,
) -> SourceCapabilityInventory {
    let mut inventory = SourceCapabilityInventory::default();
    let mut resolved = BTreeMap::<SourceBinding, String>::new();
    for declaration in &facts.declarations {
        let identity = declaration.identity.to_string();
        if !CAPABILITY_TYPE_IDENTITIES.contains(&identity.as_str()) {
            continue;
        }
        inventory
            .declarations
            .entry(identity.clone())
            .or_default()
            .insert(format!(
                "{crate_name}::{identity} ({}; {})",
                declaration.kind, declaration.source
            ));
        let binding = SourceBinding {
            module_path: declaration.module_path.clone(),
            name: identity.clone(),
        };
        if let Some(previous) = resolved.insert(binding.clone(), identity.clone()) {
            panic!(
                "capability source binding {} resolved to both {previous} and {identity}",
                display_binding(&binding)
            );
        }
        record_capability_derives(crate_name, declaration, &mut inventory.trait_impls);
    }

    let alias_bindings = facts
        .aliases
        .iter()
        .map(|alias| alias.binding.clone())
        .collect::<BTreeSet<_>>();
    let mut unresolved_globs = BTreeSet::new();
    loop {
        let mut changed = false;
        for alias in &facts.aliases {
            let Some(identity) = resolve_alias_target(
                &alias.target,
                &alias.binding.module_path,
                crate_name,
                &resolved,
                &alias_bindings,
            ) else {
                continue;
            };
            if alias.lexical_scope {
                panic!(
                    "lexically scoped capability alias {} in {} cannot be resolved \
                     without modelling Rust block scopes; capability trait inventory \
                     fails closed",
                    display_binding(&alias.binding),
                    alias.source
                );
            }
            if alias.generic || (alias.conditional && alias.fail_if_conditional) {
                let reason = match (
                    alias.generic,
                    alias.conditional && alias.fail_if_conditional,
                ) {
                    (true, true) => "generic and cfg-gated",
                    (true, false) => "generic",
                    (false, true) => "cfg-gated",
                    (false, false) => unreachable!(),
                };
                panic!(
                    "{reason} capability alias {} in {} cannot be resolved with \
                     confidence; capability trait inventory fails closed",
                    display_binding(&alias.binding),
                    alias.source
                );
            }
            changed |=
                insert_resolved_binding(&mut resolved, &alias.binding, &identity, &alias.source);
        }
        for glob in &facts.globs {
            let Some(target_module) =
                resolve_module_path(&glob.target, &glob.module_path, crate_name)
            else {
                unresolved_globs.insert(glob.module_path.clone());
                continue;
            };
            let imported = resolved
                .iter()
                .filter(|(binding, _)| binding.module_path == target_module)
                .map(|(binding, identity)| (binding.name.clone(), identity.clone()))
                .collect::<Vec<_>>();
            if imported.is_empty() {
                continue;
            }
            if glob.conditional {
                panic!(
                    "cfg-gated glob import in {} can expose a capability alias; \
                     capability trait inventory fails closed",
                    glob.source
                );
            }
            for (name, identity) in imported {
                let binding = SourceBinding {
                    module_path: glob.module_path.clone(),
                    name,
                };
                changed |=
                    insert_resolved_binding(&mut resolved, &binding, &identity, &glob.source);
            }
        }
        if !changed {
            break;
        }
    }
    let module_alias_bindings = facts
        .module_aliases
        .iter()
        .filter(|alias| {
            alias.conservative
                || resolve_module_path(&alias.target, &alias.binding.module_path, crate_name)
                    .is_some_and(|target_module| {
                        resolved
                            .keys()
                            .any(|binding| binding.module_path == target_module)
                    })
        })
        .map(|alias| alias.binding.clone())
        .collect::<BTreeSet<_>>();
    let noncapability_aliases =
        resolve_noncapability_aliases(&facts.aliases, crate_name, &resolved, &alias_bindings);
    let fail_closed_alias_bindings = facts
        .aliases
        .iter()
        .filter(|alias| {
            alias.fail_if_unresolved
                && !resolved.contains_key(&alias.binding)
                && !noncapability_aliases.contains(&alias.binding)
        })
        .map(|alias| alias.binding.clone())
        .collect::<BTreeSet<_>>();
    for alias in &facts.aliases {
        if resolved.contains_key(&alias.binding) || noncapability_aliases.contains(&alias.binding) {
            continue;
        }
        let SourceAliasTarget::Unsupported(tokens) = &alias.target else {
            continue;
        };
        let mentions_capability = CAPABILITY_TYPE_IDENTITIES
            .iter()
            .any(|identity| tokens.contains(identity))
            || resolved
                .keys()
                .any(|binding| tokens.contains(&binding.name));
        if mentions_capability {
            panic!(
                "unsupported alias {} in {} may resolve to a capability through \
                 {tokens}; capability trait inventory fails closed",
                display_binding(&alias.binding),
                alias.source
            );
        }
    }

    for implementation in &facts.implementations {
        let Some((polarity, trait_path, _)) = implementation.implementation.trait_.as_ref() else {
            continue;
        };
        let identity = resolve_impl_self_type(
            &implementation.implementation.self_ty,
            &implementation.module_path,
            crate_name,
            &resolved,
            SourceAliasBindings {
                type_aliases: &fail_closed_alias_bindings,
                module_aliases: &module_alias_bindings,
            },
            unresolved_globs.contains(&implementation.module_path),
            &implementation.source,
        );
        let Some(identity) = identity else {
            continue;
        };
        validate_capability_impl_cfg_attrs(
            crate_name,
            &identity,
            &implementation.implementation.attrs,
            &implementation.source,
        );
        let generic_parameters = if implementation.implementation.generics.params.is_empty() {
            String::new()
        } else {
            format!(
                "<{}>",
                compact_tokens(&implementation.implementation.generics.params)
            )
        };
        let where_clause = implementation
            .implementation
            .generics
            .where_clause
            .as_ref()
            .map_or_else(String::new, |clause| format!(" {}", compact_tokens(clause)));
        let qualifier = match (
            implementation.implementation.defaultness.is_some(),
            implementation.implementation.unsafety.is_some(),
        ) {
            (true, true) => "default unsafe ",
            (true, false) => "default ",
            (false, true) => "unsafe ",
            (false, false) => "",
        };
        let polarity = polarity.as_ref().map_or("", |_| "!");
        inventory.trait_impls.insert(format!(
            "{crate_name}::{identity}\t{qualifier}impl{generic_parameters} \
             {polarity}{} for {}{where_clause}",
            compact_tokens(trait_path),
            compact_tokens(&implementation.implementation.self_ty),
        ));
    }
    inventory
}

fn resolve_noncapability_aliases(
    aliases: &[SourceAlias],
    crate_name: &str,
    capabilities: &BTreeMap<SourceBinding, String>,
    alias_bindings: &BTreeSet<SourceBinding>,
) -> BTreeSet<SourceBinding> {
    let mut noncapabilities = BTreeSet::new();
    loop {
        let mut changed = false;
        for alias in aliases {
            if capabilities.contains_key(&alias.binding) || noncapabilities.contains(&alias.binding)
            {
                continue;
            }
            if alias.generic
                || (alias.conditional && alias.fail_if_conditional)
                || matches!(alias.target, SourceAliasTarget::Unsupported(_))
            {
                continue;
            }
            let is_noncapability = match &alias.target {
                SourceAliasTarget::Path(path) => {
                    let candidates =
                        binding_candidates(path, &alias.binding.module_path, crate_name);
                    !path.segments.last().is_some_and(|identity| {
                        CAPABILITY_TYPE_IDENTITIES.contains(&identity.as_str())
                    }) && candidates.iter().all(|candidate| {
                        !alias_bindings.contains(candidate) || noncapabilities.contains(candidate)
                    })
                }
                SourceAliasTarget::Unsupported(_) => unreachable!(),
            };
            if is_noncapability {
                changed |= noncapabilities.insert(alias.binding.clone());
            }
        }
        if !changed {
            return noncapabilities;
        }
    }
}

fn insert_resolved_binding(
    resolved: &mut BTreeMap<SourceBinding, String>,
    binding: &SourceBinding,
    identity: &str,
    source: &str,
) -> bool {
    if let Some(previous) = resolved.get(binding) {
        assert_eq!(
            previous,
            identity,
            "capability alias {} in {source} is ambiguous: it resolves to both \
             {previous} and {identity}",
            display_binding(binding)
        );
        return false;
    }
    resolved.insert(binding.clone(), identity.to_owned());
    true
}

fn resolve_alias_target(
    target: &SourceAliasTarget,
    module_path: &[String],
    crate_name: &str,
    resolved: &BTreeMap<SourceBinding, String>,
    alias_bindings: &BTreeSet<SourceBinding>,
) -> Option<String> {
    let SourceAliasTarget::Path(path) = target else {
        return None;
    };
    resolve_source_path(path, module_path, crate_name, resolved, alias_bindings).identity
}

struct SourcePathResolution {
    identity: Option<String>,
    unresolved_alias: bool,
}

fn resolve_source_path(
    path: &SourcePath,
    module_path: &[String],
    crate_name: &str,
    resolved: &BTreeMap<SourceBinding, String>,
    alias_bindings: &BTreeSet<SourceBinding>,
) -> SourcePathResolution {
    if let Some(identity) = path
        .segments
        .last()
        .filter(|identity| CAPABILITY_TYPE_IDENTITIES.contains(&identity.as_str()))
    {
        return SourcePathResolution {
            identity: Some(identity.clone()),
            unresolved_alias: false,
        };
    }
    let candidates = binding_candidates(path, module_path, crate_name);
    let identities = candidates
        .iter()
        .filter_map(|candidate| resolved.get(candidate).cloned())
        .collect::<BTreeSet<_>>();
    assert!(
        identities.len() <= 1,
        "capability alias path {} is ambiguous in module {}",
        display_source_path(path),
        display_module(crate_name, module_path)
    );
    SourcePathResolution {
        identity: identities.into_iter().next(),
        unresolved_alias: candidates
            .iter()
            .any(|candidate| alias_bindings.contains(candidate)),
    }
}

fn binding_candidates(
    path: &SourcePath,
    module_path: &[String],
    crate_name: &str,
) -> Vec<SourceBinding> {
    let Some(name) = path.segments.last().cloned() else {
        return Vec::new();
    };
    let prefix = &path.segments[..path.segments.len() - 1];
    let normalized_crate_name = crate_name.replace('-', "_");
    let mut candidates = BTreeSet::new();
    if path.leading_colon {
        if prefix
            .first()
            .is_some_and(|segment| segment == &normalized_crate_name)
        {
            candidates.insert(SourceBinding {
                module_path: prefix[1..].to_vec(),
                name,
            });
        }
        return candidates.into_iter().collect();
    }
    match prefix.first().map(String::as_str) {
        Some("crate") => {
            candidates.insert(SourceBinding {
                module_path: prefix[1..].to_vec(),
                name,
            });
        }
        Some("self") => {
            let mut target = module_path.to_vec();
            target.extend_from_slice(&prefix[1..]);
            candidates.insert(SourceBinding {
                module_path: target,
                name,
            });
        }
        Some("super") => {
            let mut target = module_path.to_vec();
            let mut index = 0;
            while prefix.get(index).is_some_and(|segment| segment == "super") {
                assert!(
                    target.pop().is_some(),
                    "source path {} escapes the crate root from module {}",
                    display_source_path(path),
                    display_module(crate_name, module_path)
                );
                index += 1;
            }
            target.extend_from_slice(&prefix[index..]);
            candidates.insert(SourceBinding {
                module_path: target,
                name,
            });
        }
        Some(first) if first == normalized_crate_name => {
            candidates.insert(SourceBinding {
                module_path: prefix[1..].to_vec(),
                name,
            });
        }
        Some(_) => {
            let mut relative = module_path.to_vec();
            relative.extend_from_slice(prefix);
            candidates.insert(SourceBinding {
                module_path: relative,
                name: name.clone(),
            });
            candidates.insert(SourceBinding {
                module_path: prefix.to_vec(),
                name,
            });
        }
        None => {
            candidates.insert(SourceBinding {
                module_path: module_path.to_vec(),
                name,
            });
        }
    }
    candidates.into_iter().collect()
}

fn resolve_module_path(
    path: &SourcePath,
    module_path: &[String],
    crate_name: &str,
) -> Option<Vec<String>> {
    let normalized_crate_name = crate_name.replace('-', "_");
    if path.leading_colon {
        return path
            .segments
            .first()
            .is_some_and(|segment| segment == &normalized_crate_name)
            .then(|| path.segments[1..].to_vec());
    }
    match path.segments.first().map(String::as_str) {
        Some("crate") => Some(path.segments[1..].to_vec()),
        Some("self") => {
            let mut target = module_path.to_vec();
            target.extend_from_slice(&path.segments[1..]);
            Some(target)
        }
        Some("super") => {
            let mut target = module_path.to_vec();
            let mut index = 0;
            while path
                .segments
                .get(index)
                .is_some_and(|segment| segment == "super")
            {
                target.pop()?;
                index += 1;
            }
            target.extend_from_slice(&path.segments[index..]);
            Some(target)
        }
        Some(first) if first == normalized_crate_name => Some(path.segments[1..].to_vec()),
        Some(_) => {
            let mut target = module_path.to_vec();
            target.extend_from_slice(&path.segments);
            Some(target)
        }
        None => None,
    }
}

#[derive(Clone, Copy)]
struct SourceAliasBindings<'a> {
    type_aliases: &'a BTreeSet<SourceBinding>,
    module_aliases: &'a BTreeSet<SourceBinding>,
}

fn resolve_impl_self_type(
    ty: &syn::Type,
    module_path: &[String],
    crate_name: &str,
    resolved: &BTreeMap<SourceBinding, String>,
    aliases: SourceAliasBindings<'_>,
    unresolved_glob: bool,
    source: &str,
) -> Option<String> {
    match ty {
        syn::Type::Group(group) => resolve_impl_self_type(
            &group.elem,
            module_path,
            crate_name,
            resolved,
            aliases,
            unresolved_glob,
            source,
        ),
        syn::Type::Paren(paren) => resolve_impl_self_type(
            &paren.elem,
            module_path,
            crate_name,
            resolved,
            aliases,
            unresolved_glob,
            source,
        ),
        syn::Type::Path(path) if path.qself.is_none() => {
            let path = source_path(&path.path);
            if let Some(alias) =
                module_alias_in_path(&path, module_path, crate_name, aliases.module_aliases)
            {
                panic!(
                    "cannot resolve module alias {} in impl self type {} in {source}; \
                     capability trait inventory fails closed instead of modelling \
                     renamed module prefixes",
                    display_binding(&alias),
                    display_source_path(&path)
                );
            }
            let resolution = resolve_source_path(
                &path,
                module_path,
                crate_name,
                resolved,
                aliases.type_aliases,
            );
            if let Some(identity) = resolution.identity {
                return Some(identity);
            }
            if resolution.unresolved_alias {
                panic!(
                    "cannot resolve possible capability alias {} used as an impl self type \
                     in {source}; generic, cfg-gated, cyclic, or unsupported aliases fail closed",
                    display_source_path(&path)
                );
            }
            if unresolved_glob {
                panic!(
                    "cannot classify impl self type {} in {source} because an external or \
                     unresolved glob import may bind a capability alias; capability trait \
                     inventory fails closed",
                    display_source_path(&path)
                );
            }
            None
        }
        other => {
            let tokens = compact_tokens(other);
            let mentions_capability = CAPABILITY_TYPE_IDENTITIES
                .iter()
                .any(|identity| tokens.contains(identity));
            if mentions_capability {
                panic!(
                    "cannot classify possible capability impl self type {tokens} in {source}; \
                     unsupported self-type syntax fails closed"
                );
            }
            None
        }
    }
}

fn module_alias_in_path(
    path: &SourcePath,
    module_path: &[String],
    crate_name: &str,
    module_alias_bindings: &BTreeSet<SourceBinding>,
) -> Option<SourceBinding> {
    if path.segments.len() < 2 {
        return None;
    }
    for end in 1..path.segments.len() {
        let prefix = SourcePath {
            leading_colon: path.leading_colon,
            segments: path.segments[..end].to_vec(),
        };
        if let Some(alias) = binding_candidates(&prefix, module_path, crate_name)
            .into_iter()
            .find(|binding| module_alias_bindings.contains(binding))
        {
            return Some(alias);
        }
    }
    None
}

fn record_capability_derives(
    crate_name: &str,
    declaration: &SourceDeclaration,
    trait_impls: &mut BTreeSet<String>,
) {
    let identity = declaration.identity.to_string();
    for attribute in &declaration.attributes {
        if attribute.path().is_ident("derive") {
            record_derive_meta(
                crate_name,
                &identity,
                &attribute.meta,
                &declaration.source,
                trait_impls,
            );
        } else if attribute.path().is_ident("cfg_attr") {
            record_cfg_attr_derives(
                crate_name,
                &identity,
                &attribute.meta,
                &declaration.source,
                trait_impls,
            );
        }
    }
}

fn record_derive_meta(
    crate_name: &str,
    identity: &str,
    meta: &Meta,
    source: &str,
    trait_impls: &mut BTreeSet<String>,
) {
    let Meta::List(list) = meta else {
        panic!("derive on capability {crate_name}::{identity} in {source} is not a list");
    };
    let traits = list
        .parse_args_with(syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated)
        .unwrap_or_else(|error| {
            panic!(
                "parse derive list for capability {crate_name}::{identity} in \
                 {source}: {error}"
            )
        });
    for trait_path in traits {
        trait_impls.insert(format!(
            "{crate_name}::{identity}\tderive {} for {identity}",
            compact_tokens(&trait_path)
        ));
    }
}

fn record_cfg_attr_derives(
    crate_name: &str,
    identity: &str,
    meta: &Meta,
    source: &str,
    trait_impls: &mut BTreeSet<String>,
) {
    let nested = parse_cfg_attr(meta, crate_name, identity, source);
    for attribute in nested {
        if attribute.path().is_ident("derive") {
            record_derive_meta(crate_name, identity, &attribute, source, trait_impls);
        } else if attribute.path().is_ident("cfg_attr") {
            record_cfg_attr_derives(crate_name, identity, &attribute, source, trait_impls);
        } else if !safe_inert_cfg_attr(&attribute) {
            panic!(
                "unrecognised cfg_attr attribute {} on capability declaration \
                 {crate_name}::{identity} in {source}; capability trait inventory \
                 fails closed",
                compact_tokens(&attribute)
            );
        }
    }
}

fn validate_capability_impl_cfg_attrs(
    crate_name: &str,
    identity: &str,
    attributes: &[Attribute],
    source: &str,
) {
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident("cfg_attr"))
    {
        validate_impl_cfg_attr(crate_name, identity, &attribute.meta, source);
    }
}

fn validate_impl_cfg_attr(crate_name: &str, identity: &str, meta: &Meta, source: &str) {
    for attribute in parse_cfg_attr(meta, crate_name, identity, source) {
        if attribute.path().is_ident("cfg_attr") {
            validate_impl_cfg_attr(crate_name, identity, &attribute, source);
        } else if !safe_inert_cfg_attr(&attribute) {
            panic!(
                "unrecognised cfg_attr attribute {} on capability impl \
                 {crate_name}::{identity} in {source}; capability trait inventory \
                 fails closed",
                compact_tokens(&attribute)
            );
        }
    }
}

fn parse_cfg_attr(meta: &Meta, crate_name: &str, identity: &str, source: &str) -> Vec<Meta> {
    let Meta::List(list) = meta else {
        panic!("cfg_attr on capability {crate_name}::{identity} in {source} is not a list");
    };
    let values = list
        .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        .unwrap_or_else(|error| {
            panic!(
                "parse cfg_attr for capability {crate_name}::{identity} in \
                 {source}: {error}"
            )
        })
        .into_iter()
        .collect::<Vec<_>>();
    assert!(
        values.len() >= 2,
        "cfg_attr on capability {crate_name}::{identity} in {source} must contain \
         a condition and at least one attribute"
    );
    values.into_iter().skip(1).collect()
}

fn safe_inert_cfg_attr(meta: &Meta) -> bool {
    let path = meta.path();
    path.is_ident("cfg")
        || path.is_ident("doc")
        || path.is_ident("allow")
        || path.is_ident("warn")
        || path.is_ident("deny")
        || path.is_ident("forbid")
        || path.is_ident("expect")
        || path.is_ident("deprecated")
        || path.is_ident("must_use")
}

fn display_binding(binding: &SourceBinding) -> String {
    let mut path = binding.module_path.join("::");
    if !path.is_empty() {
        path.push_str("::");
    }
    path.push_str(&binding.name);
    path
}

fn display_source_path(path: &SourcePath) -> String {
    let mut rendered = if path.leading_colon {
        "::".to_owned()
    } else {
        String::new()
    };
    rendered.push_str(&path.segments.join("::"));
    rendered
}

fn display_module(crate_name: &str, module_path: &[String]) -> String {
    if module_path.is_empty() {
        crate_name.to_owned()
    } else {
        format!("{crate_name}::{}", module_path.join("::"))
    }
}

fn source_location(
    crate_name: &str,
    source_root: &Path,
    source: &Path,
    module_path: &[String],
) -> String {
    let module = display_module(crate_name, module_path);
    let relative = source
        .strip_prefix(source_root)
        .ok()
        .filter(|path| !path.as_os_str().is_empty())
        .or_else(|| source.file_name().map(Path::new))
        .map_or_else(|| "<unknown>".to_owned(), |path| path.display().to_string());
    format!("{module} ({relative})")
}

fn compact_tokens(tokens: &impl ToTokens) -> String {
    tokens
        .to_token_stream()
        .to_string()
        .split_whitespace()
        .collect()
}

struct HiddenPublicScanner<'a> {
    crate_name: &'a str,
    source_root: PathBuf,
    entries: BTreeSet<String>,
    visited: BTreeMap<PathBuf, Vec<String>>,
}

impl HiddenPublicScanner<'_> {
    fn scan_file(
        &mut self,
        source: &Path,
        module_path: &[String],
        crate_root: bool,
        inherited_hidden: bool,
    ) {
        let logical_source =
            source_location(self.crate_name, &self.source_root, source, module_path);
        let source = fs::canonicalize(source)
            .unwrap_or_else(|error| panic!("canonicalize Rust source {logical_source}: {error}"));
        if let Some(previous) = self.visited.get(&source) {
            assert_eq!(
                previous,
                module_path,
                "Rust source {logical_source} is reachable as both {} and {}; hidden public \
                 inventory refuses an ambiguous logical module path",
                display_module(self.crate_name, previous),
                display_module(self.crate_name, module_path)
            );
            return;
        }
        self.visited.insert(source.clone(), module_path.to_vec());
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("read Rust source {logical_source}: {error}"));
        let file = syn::parse_file(&text)
            .unwrap_or_else(|error| panic!("parse Rust source {logical_source}: {error}"));
        let module_dir = source_module_dir(&source, crate_root);
        let path_base = source
            .parent()
            .expect("canonical Rust source has a parent directory");
        self.scan_items(
            &file.items,
            module_path,
            &module_dir,
            path_base,
            inherited_hidden || doc_hidden(&file.attrs),
        );
    }

    fn scan_items(
        &mut self,
        items: &[Item],
        module_path: &[String],
        module_dir: &Path,
        path_base: &Path,
        inherited_hidden: bool,
    ) {
        for item in items {
            match item {
                Item::Fn(function)
                    if matches!(function.vis, Visibility::Public(_))
                        && (inherited_hidden || doc_hidden(&function.attrs)) =>
                {
                    self.record(module_path, function.sig.ident.to_string(), &function.sig);
                }
                Item::Fn(_) => {}
                Item::Impl(implementation) => {
                    let hidden = inherited_hidden || doc_hidden(&implementation.attrs);
                    let owner = type_name(&implementation.self_ty);
                    for member in &implementation.items {
                        if let syn::ImplItem::Fn(method) = member
                            && (hidden || doc_hidden(&method.attrs))
                            && matches!(method.vis, Visibility::Public(_))
                        {
                            let name = format!("{owner}::method:{}", method.sig.ident);
                            self.record(module_path, name, &method.sig);
                        }
                    }
                }
                Item::Trait(trait_item) => {
                    let hidden = inherited_hidden || doc_hidden(&trait_item.attrs);
                    for member in &trait_item.items {
                        if let syn::TraitItem::Fn(method) = member
                            && (hidden || doc_hidden(&method.attrs))
                        {
                            let name =
                                format!("{}::tymethod:{}", trait_item.ident, method.sig.ident);
                            self.record(module_path, name, &method.sig);
                        }
                    }
                }
                Item::Mod(module) => {
                    let hidden = inherited_hidden || doc_hidden(&module.attrs);
                    let mut child_path = module_path.to_vec();
                    child_path.push(module.ident.to_string());
                    if hidden && matches!(module.vis, Visibility::Public(_)) {
                        let signature = format!("pub mod {}", module.ident);
                        self.record(module_path, module.ident.to_string(), &signature);
                    }
                    let child_dir = module_dir.join(module.ident.to_string());
                    if let Some((_, items)) = &module.content {
                        let inline_dir = module_path_override(module, path_base)
                            .unwrap_or_else(|| child_dir.clone());
                        self.scan_items(items, &child_path, &inline_dir, &inline_dir, hidden);
                    } else if let Some(source) =
                        module_source(module, module_dir, path_base, &child_dir)
                    {
                        self.scan_file(&source, &child_path, false, hidden);
                    }
                }
                _ => {}
            }
        }
    }

    fn record(&mut self, module_path: &[String], name: String, signature: &impl ToTokens) {
        let mut symbol = self.crate_name.to_owned();
        for module in module_path {
            symbol.push_str("::");
            symbol.push_str(module);
        }
        symbol.push_str("::");
        symbol.push_str(&name);
        self.entries
            .insert(format!("{symbol}\t{}", signature.to_token_stream()));
    }
}

fn doc_hidden(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        if !attribute.path().is_ident("doc") {
            return false;
        }
        let mut hidden = false;
        let _ = attribute.parse_nested_meta(|meta| {
            hidden |= meta.path.is_ident("hidden");
            Ok(())
        });
        hidden
    })
}

fn type_name(ty: &syn::Type) -> String {
    if let syn::Type::Path(path) = ty
        && let Some(segment) = path.path.segments.last()
    {
        return segment.ident.to_string();
    }
    ty.to_token_stream().to_string()
}

fn source_module_dir(source: &Path, crate_root: bool) -> PathBuf {
    let parent = source
        .parent()
        .expect("canonical Rust source has a parent directory");
    if crate_root || source.file_name().is_some_and(|name| name == "mod.rs") {
        return parent.to_path_buf();
    }
    let stem = source
        .file_stem()
        .expect("non-root Rust module source has a file stem");
    parent.join(stem)
}

fn module_source(
    module: &syn::ItemMod,
    module_dir: &Path,
    path_base: &Path,
    child_dir: &Path,
) -> Option<PathBuf> {
    if let Some(path) = module_path_override(module, path_base) {
        return path.is_file().then_some(path);
    }
    let flat = module_dir.join(format!("{}.rs", module.ident));
    if flat.is_file() {
        return Some(flat);
    }
    let nested = child_dir.join("mod.rs");
    nested.is_file().then_some(nested)
}

fn module_path_override(module: &syn::ItemMod, path_base: &Path) -> Option<PathBuf> {
    if module
        .attrs
        .iter()
        .any(|attribute| attribute.path().is_ident("cfg_attr"))
    {
        panic!(
            "cfg_attr on Rust module {} may select a different source; \
             source inventories fail closed instead of guessing which file the \
             compiler reads",
            module.ident
        );
    }
    let mut attributes = module
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("path"));
    let attribute = attributes.next()?;
    assert!(
        attributes.next().is_none(),
        "multiple path attributes on Rust module {}; source inventories fail closed",
        module.ident
    );
    let path = match &attribute.meta {
        syn::Meta::NameValue(value) => {
            let syn::Expr::Lit(value) = &value.value else {
                panic!(
                    "path attribute on Rust module {} is not a string literal; \
                     source inventories fail closed",
                    module.ident
                );
            };
            let syn::Lit::Str(path) = &value.lit else {
                panic!(
                    "path attribute on Rust module {} is not a string literal; \
                     source inventories fail closed",
                    module.ident
                );
            };
            path
        }
        _ => panic!(
            "path attribute on Rust module {} is not name-value syntax; \
             source inventories fail closed",
            module.ident
        ),
    };
    Some(path_base.join(path.value()))
}
fn dependency_order(mut packages: BTreeMap<String, RenderPackage>) -> Vec<(String, RenderPackage)> {
    let mut ordered = Vec::with_capacity(packages.len());
    let mut rendered = BTreeSet::new();
    while !packages.is_empty() {
        let ready = packages
            .iter()
            .filter(|(_, package)| package.workspace_dependencies.is_subset(&rendered))
            .map(|(name, _)| name.clone())
            .collect::<Vec<_>>();
        assert!(
            !ready.is_empty(),
            "workspace library dependency graph is cyclic or incomplete: {:?}",
            packages.keys().collect::<Vec<_>>()
        );
        for name in ready {
            let package = packages
                .remove(&name)
                .expect("ready rustdoc package remains pending");
            rendered.insert(name.clone());
            ordered.push((name, package));
        }
    }
    ordered
}

fn validate_documented_crate(crate_name: &str, root: &Path) -> Result<Vec<AdvertisedItem>, String> {
    let all_path = root.join("all.html");
    let all = fs::read_to_string(&all_path).map_err(|error| {
        format!(
            "rustdoc output for {crate_name} is incomplete: cannot read {}: \
             {error}. This is a doc-build problem.",
            all_path.display()
        )
    })?;
    let marker = "<li><a href=\"";
    let mut advertised = Vec::new();
    for (index, entry) in all.split(marker).skip(1).enumerate() {
        let (href, rest) = entry.split_once('"').ok_or_else(|| {
            format!(
                "rustdoc output for {crate_name} has malformed all-items entry {}. \
                 This is a doc-build problem.",
                index + 1
            )
        })?;
        let (_, rest) = rest.split_once('>').ok_or_else(|| {
            format!(
                "rustdoc output for {crate_name} has malformed all-items link {href:?}. \
                 This is a doc-build problem."
            )
        })?;
        let (name, _) = rest.split_once("</a>").ok_or_else(|| {
            format!(
                "rustdoc output for {crate_name} has malformed all-items label for \
                 {href:?}. This is a doc-build problem."
            )
        })?;
        if href.starts_with('#') {
            continue;
        }
        if href.starts_with('/')
            || href.split('/').any(|component| component == "..")
            || !href.ends_with(".html")
        {
            return Err(format!(
                "rustdoc output for {crate_name} advertises invalid item path \
                 {href:?}. This is a doc-build problem."
            ));
        }
        let symbol = format!("{crate_name}::{name}");
        let path = root.join(href);
        let html = fs::read_to_string(&path).map_err(|error| {
            format!(
                "rustdoc output is incomplete: advertised item {symbol} has no \
                 readable page at {}: {error}. This is a doc-build problem.",
                path.display()
            )
        })?;
        if item_declaration(&html).is_none() {
            return Err(format!(
                "rustdoc output is incomplete: advertised item {symbol} could not \
                 be parsed from {}. This is a doc-build problem.",
                path.display()
            ));
        }
        advertised.push(AdvertisedItem {
            href: PathBuf::from(href),
            name: name.to_owned(),
        });
    }
    if advertised.is_empty() {
        return Err(format!(
            "rustdoc output for {crate_name} advertises no items in {}. This is \
             a doc-build problem.",
            all_path.display()
        ));
    }
    Ok(advertised)
}

fn workspace_public_api(
    docs: &[DocumentedCrate],
    snapshot_crates: &BTreeSet<&str>,
    require_all_roots: bool,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let items = documented_items(docs);
    let (capability_roots, capability_types) =
        capability_type_identities(&items, require_all_roots);
    let mut public = BTreeSet::new();
    let mut capability_surface = BTreeSet::new();
    for item in &items {
        let snapshot = snapshot_crates.contains(item.crate_name.as_str());
        if snapshot {
            public.insert(item.symbol.clone());
        }
        if capability_types.contains(&type_identity(&item.path))
            || item_declaration(&item.html).is_some_and(|signature| {
                signature_links_to(signature, &item.path, &capability_types)
            })
        {
            capability_surface.insert(item.symbol.clone());
        }
        collect_members(
            item,
            snapshot,
            &capability_roots,
            &capability_types,
            &mut public,
            &mut capability_surface,
        );
    }
    (public, capability_surface)
}

fn snapshot_public_api(docs: &[DocumentedCrate], approved: &BTreeSet<String>) -> BTreeSet<String> {
    let mut public = BTreeSet::new();
    for item in documented_items(docs) {
        public.insert(item.symbol.clone());
        let html = without_deref_methods(&item.rendered_html);
        for section in html.split("<section id=\"").skip(1) {
            let Some((id, rest)) = section.split_once('"') else {
                continue;
            };
            let Some((kind, member)) = id.split_once('.') else {
                continue;
            };
            if !matches!(
                kind,
                "method" | "tymethod" | "structfield" | "associatedtype" | "associatedconstant"
            ) {
                continue;
            }
            let Some((class_prefix, body)) = rest.split_once('>') else {
                continue;
            };
            let body = body.split_once("</section>").map_or(body, |(body, _)| body);
            let trait_implementation = class_prefix.contains("trait-impl");
            let symbol = format!("{}::{kind}:{member}", item.symbol);
            if trait_implementation {
                if approved.contains(&symbol) {
                    public.insert(symbol);
                }
                continue;
            }
            if kind != "method" || body.contains("<h4 class=\"code-header\">pub ") {
                public.insert(symbol);
            }
        }
    }
    public
}

fn documented_items(docs: &[DocumentedCrate]) -> Vec<DocumentedItem> {
    let mut items = Vec::new();
    for docs in docs {
        let restricted_modules = restricted_module_directories(&docs.root);
        for advertised in &docs.advertised {
            let rendered_path = docs.root.join(&advertised.href);
            let rendered_path =
                fs::canonicalize(&rendered_path).expect("canonicalize rendered rustdoc item page");
            if restricted_modules
                .iter()
                .any(|module| rendered_path.starts_with(module))
            {
                continue;
            }
            let path = canonical_path(&rendered_path);
            let rendered_html =
                fs::read_to_string(&rendered_path).expect("read rendered rustdoc item page");
            let html = if path == rendered_path {
                rendered_html.clone()
            } else {
                fs::read_to_string(&path).expect("read canonical rustdoc item page")
            };
            if !item_declaration(&html).is_some_and(is_public_declaration) {
                continue;
            }
            items.push(DocumentedItem {
                crate_name: docs.crate_name.clone(),
                symbol: format!("{}::{}", docs.crate_name, advertised.name),
                path,
                html,
                rendered_html,
            });
        }
    }
    items
}

fn restricted_module_directories(root: &Path) -> BTreeSet<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut restricted = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).expect("walk rustdoc module directories") {
            let path = entry.expect("read rustdoc module entry").path();
            if path.is_dir() {
                pending.push(path);
            }
        }
        let index = directory.join("index.html");
        if !index.is_file() {
            continue;
        }
        let html = fs::read_to_string(index).expect("read rustdoc module index");
        for item in html.split("<dt>").skip(1) {
            let item = item.split_once("</dt>").map_or(item, |(item, _)| item);
            if !item.contains("title=\"Restricted Visibility\"") {
                continue;
            }
            let Some(href) = item
                .split_once("<a class=\"mod\" href=\"")
                .and_then(|(_, rest)| rest.split_once('"').map(|(href, _)| href))
            else {
                continue;
            };
            let module_index = directory.join(href);
            if module_index.is_file() {
                restricted.insert(
                    fs::canonicalize(
                        module_index
                            .parent()
                            .expect("restricted rustdoc module parent"),
                    )
                    .expect("canonicalize restricted rustdoc module"),
                );
            }
        }
    }
    restricted
}

fn capability_type_identities(
    items: &[DocumentedItem],
    require_all_roots: bool,
) -> (BTreeSet<PathBuf>, BTreeSet<PathBuf>) {
    let mut by_type_name = BTreeMap::<&str, BTreeSet<PathBuf>>::new();
    for item in items.iter().filter(|item| is_type_page(&item.path)) {
        let type_name = item.symbol.rsplit("::").next().expect("rustdoc type name");
        by_type_name
            .entry(type_name)
            .or_default()
            .insert(type_identity(&item.path));
    }
    let capability_roots = CAPABILITY_TYPE_IDENTITIES
        .iter()
        .chain(CLAIM_TYPE_IDENTITIES)
        .filter_map(|identity| {
            let matches = by_type_name.get(identity).cloned().unwrap_or_default();
            assert!(
                matches.len() <= 1,
                "capability type identity {identity:?} is ambiguous: {matches:?}"
            );
            if require_all_roots {
                assert!(
                    matches.len() == 1,
                    "capability type identity {identity:?} is undocumented"
                );
            }
            matches.into_iter().next()
        })
        .collect::<BTreeSet<_>>();
    let mut capability_types = capability_roots.clone();

    loop {
        let mut added = false;
        for item in items {
            if capability_types.contains(&type_identity(&item.path)) || !is_type_page(&item.path) {
                continue;
            }
            if public_signature_links_to(&item.html, &item.path, &capability_types) {
                added |= capability_types.insert(type_identity(&item.path));
            }
        }
        if !added {
            return (capability_roots, capability_types);
        }
    }
}

fn collect_members(
    item: &DocumentedItem,
    snapshot: bool,
    capability_roots: &BTreeSet<PathBuf>,
    capability_types: &BTreeSet<PathBuf>,
    public: &mut BTreeSet<String>,
    capability_surface: &mut BTreeSet<String>,
) {
    let item_identity = type_identity(&item.path);
    let item_is_capability = capability_types.contains(&item_identity);
    let html = without_deref_methods(&item.html);
    for section in html.split("<section id=\"").skip(1) {
        let Some((id, rest)) = section.split_once('"') else {
            continue;
        };
        let Some((kind, member)) = id.split_once('.') else {
            continue;
        };
        if !matches!(
            kind,
            "method" | "tymethod" | "structfield" | "associatedtype" | "associatedconstant"
        ) {
            continue;
        }
        let Some((class_prefix, body)) = rest.split_once('>') else {
            continue;
        };
        let body = body.split_once("</section>").map_or(body, |(body, _)| body);
        let trait_implementation = class_prefix.contains("trait-impl");
        let signature_is_capability = code_header(body)
            .is_some_and(|signature| signature_links_to(signature, &item.path, capability_types));
        let capability = signature_is_capability || (item_is_capability && !trait_implementation);
        let symbol = format!("{}::{kind}:{member}", item.symbol);
        if trait_implementation {
            if capability {
                capability_surface.insert(symbol.clone());
            }
            if snapshot && capability_roots.contains(&item_identity) {
                public.insert(symbol);
            }
            continue;
        }
        if kind == "method" && !body.contains("<h4 class=\"code-header\">pub ") {
            continue;
        }
        if capability {
            capability_surface.insert(symbol.clone());
        }
        if snapshot {
            public.insert(symbol);
        }
    }
}

fn without_deref_methods(html: &str) -> std::borrow::Cow<'_, str> {
    // Preserve trait implementations emitted after rustdoc's inherited Deref
    // methods; only the inherited compiler-version-dependent region is omitted.
    let Some((before, after)) = html.split_once("id=\"deref-methods-") else {
        return std::borrow::Cow::Borrowed(html);
    };
    // Resume at whichever section rustdoc emits after the deref block. The
    // anchor set differs between compiler versions, and if none matches we keep
    // the page whole rather than dropping the remainder: an unexcised inherited
    // method surfaces as an addition, which is loud and correct, whereas
    // silently discarding every trait implementation after this point hides
    // exactly the capability accessors this guard exists to find. That is what
    // made CI report 625 symbols removed on the pinned toolchain.
    let Some(rest) = [
        "id=\"trait-implementations",
        "id=\"synthetic-implementations",
        "id=\"blanket-implementations",
    ]
    .iter()
    .find_map(|anchor| after.split_once(anchor).map(|(_, rest)| rest)) else {
        return std::borrow::Cow::Borrowed(html);
    };
    std::borrow::Cow::Owned(format!("{before}{rest}"))
}

fn public_signature_links_to(html: &str, item_path: &Path, identities: &BTreeSet<PathBuf>) -> bool {
    if item_declaration(html)
        .is_some_and(|signature| signature_links_to(signature, item_path, identities))
    {
        return true;
    }
    let html = without_deref_methods(html);
    for section in html.split("<section id=\"").skip(1) {
        let Some((id, rest)) = section.split_once('"') else {
            continue;
        };
        let Some((kind, _)) = id.split_once('.') else {
            continue;
        };
        if !matches!(
            kind,
            "method" | "tymethod" | "structfield" | "associatedtype" | "associatedconstant"
        ) {
            continue;
        }
        let body = rest
            .split_once('>')
            .map(|(_, body)| body)
            .unwrap_or_default();
        if code_header(body)
            .is_some_and(|signature| signature_links_to(signature, item_path, identities))
        {
            return true;
        }
    }
    false
}

fn signature_links_to(signature: &str, item_path: &Path, identities: &BTreeSet<PathBuf>) -> bool {
    for link in signature.split("href=\"").skip(1) {
        let Some((href, _)) = link.split_once('"') else {
            continue;
        };
        let href = href.split_once('#').map_or(href, |(path, _)| path);
        if href.is_empty() || href.contains("://") {
            continue;
        }
        let linked = item_path.parent().expect("rustdoc item parent").join(href);
        if linked.is_file() && identities.contains(&type_identity(&linked)) {
            return true;
        }
    }
    false
}

fn is_type_page(path: &Path) -> bool {
    let file = path
        .file_name()
        .and_then(|file| file.to_str())
        .unwrap_or_default();
    ["struct.", "enum.", "trait.", "type.", "union."]
        .iter()
        .any(|prefix| file.starts_with(prefix))
}

fn canonical_path(path: &Path) -> PathBuf {
    let mut identity = fs::canonicalize(path).unwrap_or_else(|error| {
        panic!(
            "canonicalize rustdoc type identity {}: {error}",
            path.display()
        )
    });
    for _ in 0..4 {
        let html = fs::read_to_string(&identity).expect("read rustdoc identity page");
        let Some(url) = html
            .split_once("<meta http-equiv=\"refresh\" content=\"0;URL=")
            .and_then(|(_, rest)| rest.split_once('"').map(|(url, _)| url))
        else {
            return identity;
        };
        identity = fs::canonicalize(
            identity
                .parent()
                .expect("rustdoc redirect parent")
                .join(url),
        )
        .expect("resolve rustdoc type identity redirect");
    }

    panic!("rustdoc type identity redirect depth exceeded")
}

fn type_identity(path: &Path) -> PathBuf {
    let page = canonical_path(path);
    let html = fs::read_to_string(&page).expect("read rustdoc type identity page");
    let Some(source_href) = html
        .split_once("<a class=\"src\" href=\"")
        .and_then(|(_, rest)| rest.split_once('"').map(|(href, _)| href))
    else {
        return page;
    };
    let (source_path, fragment) = source_href
        .split_once('#')
        .map_or((source_href, ""), |(source, fragment)| (source, fragment));
    let source = normalize_path(
        page.parent()
            .expect("rustdoc type page parent")
            .join(source_path),
    );
    PathBuf::from(format!("{}#{fragment}", source.display()))
}

fn normalize_path(path: PathBuf) -> PathBuf {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            std::path::Component::CurDir => {}
            component => normalized.push(component.as_os_str()),
        }
    }
    normalized
}

fn item_declaration(html: &str) -> Option<&str> {
    html.split_once("<pre class=\"rust item-decl\"><code>")?
        .1
        .split_once("</code></pre>")
        .map(|(declaration, _)| declaration)
}

fn is_public_declaration(declaration: &str) -> bool {
    declaration.starts_with("pub ")
}

fn code_header(section: &str) -> Option<&str> {
    section
        .split_once("<h4 class=\"code-header\">")?
        .1
        .split_once("</h4>")
        .map(|(header, _)| header)
}
