use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use quote::ToTokens;
use syn::{Attribute, Item, Visibility};

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
    let mut scanner = HiddenPublicScanner {
        crate_name,
        entries: BTreeSet::new(),
        visited: BTreeSet::new(),
    };
    let module_dir = source
        .parent()
        .expect("library target source has a parent directory");
    scanner.scan_file(source, &[], module_dir, false);
    scanner.entries
}

struct HiddenPublicScanner<'a> {
    crate_name: &'a str,
    entries: BTreeSet<String>,
    visited: BTreeSet<PathBuf>,
}

impl HiddenPublicScanner<'_> {
    fn scan_file(
        &mut self,
        source: &Path,
        module_path: &[String],
        module_dir: &Path,
        inherited_hidden: bool,
    ) {
        let source = fs::canonicalize(source).unwrap_or_else(|error| {
            panic!("canonicalize Rust source {}: {error}", source.display())
        });
        if !self.visited.insert(source.clone()) {
            return;
        }
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("read Rust source {}: {error}", source.display()));
        let file = syn::parse_file(&text)
            .unwrap_or_else(|error| panic!("parse Rust source {}: {error}", source.display()));
        self.scan_items(
            &file.items,
            module_path,
            module_dir,
            inherited_hidden || doc_hidden(&file.attrs),
        );
    }

    fn scan_items(
        &mut self,
        items: &[Item],
        module_path: &[String],
        module_dir: &Path,
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
                        self.scan_items(items, &child_path, &child_dir, hidden);
                    } else if let Some(source) = module_source(module, module_dir, &child_dir) {
                        self.scan_file(&source, &child_path, &child_dir, hidden);
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

fn module_source(module: &syn::ItemMod, module_dir: &Path, child_dir: &Path) -> Option<PathBuf> {
    if let Some(path) = module.attrs.iter().find_map(|attribute| {
        if !attribute.path().is_ident("path") {
            return None;
        }
        let syn::Meta::NameValue(value) = &attribute.meta else {
            return None;
        };
        let syn::Expr::Lit(value) = &value.value else {
            return None;
        };
        let syn::Lit::Str(path) = &value.lit else {
            return None;
        };
        Some(module_dir.join(path.value()))
    }) {
        return path.is_file().then_some(path);
    }
    let flat = module_dir.join(format!("{}.rs", module.ident));
    if flat.is_file() {
        return Some(flat);
    }
    let nested = child_dir.join("mod.rs");
    nested.is_file().then_some(nested)
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
