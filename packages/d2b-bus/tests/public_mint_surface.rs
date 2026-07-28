use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

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
        None,
    );

    let approved = approved_entries(include_str!("approved-public-api.txt"));
    let snapshot_crates = approved
        .iter()
        .filter_map(|symbol| symbol.split_once("::").map(|(crate_name, _)| crate_name))
        .collect::<BTreeSet<_>>();
    let snapshot_docs = render_workspace_docs(
        &crate_root.parent().unwrap().join("Cargo.toml"),
        &scratch.path().join("snapshot"),
        Some(&snapshot_crates),
    );
    // Fail loudly when rustdoc did not render a crate the snapshot expects.
    // Without this, an incomplete or racing doc build shows up as that crate's
    // entire API having been "removed", which reads like a capability change
    // and sends the reader looking for a defect that is not there.
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
    if std::env::var_os("D2B_UPDATE_BUS_PUBLIC_API").is_some() {
        write_snapshot(&crate_root.join("tests/approved-public-api.txt"), &actual);
        write_snapshot(
            &crate_root.join("tests/approved-capability-api.txt"),
            &capability_surface,
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
    assert_snapshot(
        &capability_surface,
        &approved_capabilities,
        "a public signature now exposes a capability or claim type outside the \
         explicitly approved capability API",
    );

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
}

#[test]
fn mutation_fixture_detects_trait_constructor_and_capability_accessor() {
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
    render_fixture_type_roots(
        &crate_root.parent().unwrap().join("Cargo.toml"),
        scratch.path(),
    );
    let docs = render_workspace_docs(&fixture.join("Cargo.toml"), scratch.path(), None);
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
    let (_, capabilities) = workspace_public_api(&docs, &BTreeSet::new(), false);
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
        "opaque claim wrappers were not classified by linked type identity"
    );
    assert!(
        capabilities
            .iter()
            .any(|symbol| symbol.ends_with("RogueSubjectClaims::method:inject")),
        "a public subject-claim injection method escaped the capability inventory"
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

fn approved_entries(snapshot: &str) -> BTreeSet<String> {
    snapshot
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(str::to_owned)
        .collect()
}

#[derive(Debug)]
struct DocumentedCrate {
    crate_name: String,
    root: PathBuf,
}

#[derive(Debug)]
struct DocumentedItem {
    crate_name: String,
    symbol: String,
    path: PathBuf,
    html: String,
}

fn render_workspace_docs(
    manifest: &Path,
    scratch: &Path,
    selected_crates: Option<&BTreeSet<&str>>,
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
    let mut packages = BTreeSet::new();
    for package in metadata["packages"].as_array().expect("packages array") {
        if !workspace_members.contains(package["id"].as_str().expect("package id")) {
            continue;
        }
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
            if selected_crates.is_some_and(|selected| !selected.contains(crate_name)) {
                continue;
            }
            packages.insert(
                package["name"]
                    .as_str()
                    .expect("workspace package name")
                    .to_owned(),
            );
        }
    }
    assert!(!packages.is_empty(), "workspace has no library crates");

    let target = scratch.join("target");
    let temp = scratch.join("tmp");
    fs::create_dir_all(&temp).expect("create rustdoc temporary directory");
    let mut command = Command::new(env!("CARGO"));
    command.args(["doc", "--quiet", "--locked", "--no-deps"]);
    command.arg("--manifest-path").arg(manifest);
    for package in packages {
        command.arg("-p").arg(package);
    }
    let output = command
        .arg("--target-dir")
        .arg(&target)
        .env("TMPDIR", &temp)
        .output()
        .expect("render workspace public APIs");
    assert!(
        output.status.success(),
        "rustdoc failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );

    let doc_root = target.join("doc");
    let mut docs = fs::read_dir(&doc_root)
        .expect("read rustdoc output")
        .filter_map(|entry| {
            let root = entry.ok()?.path();
            if !root.join("all.html").is_file() {
                return None;
            }
            let crate_name = root.file_name()?.to_str()?.to_owned();
            Some(DocumentedCrate { crate_name, root })
        })
        .collect::<Vec<_>>();
    docs.sort_by(|left, right| left.crate_name.cmp(&right.crate_name));
    assert!(!docs.is_empty(), "rustdoc rendered no library crates");
    docs
}

fn render_fixture_type_roots(manifest: &Path, scratch: &Path) {
    let temp = scratch.join("tmp");
    fs::create_dir_all(&temp).expect("create fixture rustdoc temporary directory");
    let output = Command::new(env!("CARGO"))
        .args(["doc", "--quiet", "--locked", "--no-deps", "--manifest-path"])
        .arg(manifest)
        .args(["-p", "d2b-contracts", "-p", "d2b-session", "--target-dir"])
        .arg(scratch.join("target"))
        .env("TMPDIR", temp)
        .output()
        .expect("render mutation fixture type roots");
    assert!(
        output.status.success(),
        "fixture type rustdoc failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
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
        if capability_types.contains(&canonical_path(&item.path))
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
        let all =
            fs::read_to_string(docs.root.join("all.html")).expect("read rustdoc all-items page");
        for entry in all.split("<li><a href=\"").skip(1) {
            let Some((href, rest)) = entry.split_once('"') else {
                continue;
            };
            if href.starts_with('#') || !href.ends_with(".html") {
                continue;
            }
            let Some((_, text)) = rest.split_once('>') else {
                continue;
            };
            let Some((name, _)) = text.split_once("</a>") else {
                continue;
            };
            let path = docs.root.join(href);
            if !path.is_file() {
                continue;
            }
            items.push(DocumentedItem {
                crate_name: docs.crate_name.clone(),
                symbol: format!("{}::{name}", docs.crate_name),
                html: fs::read_to_string(&path).expect("read rustdoc item page"),
                path,
            });
        }
    }
    items
}

fn capability_type_identities(
    items: &[DocumentedItem],
    require_all_roots: bool,
) -> (BTreeSet<PathBuf>, BTreeSet<PathBuf>) {
    let mut by_type_name = BTreeMap::<&str, Vec<PathBuf>>::new();
    for item in items.iter().filter(|item| is_type_page(&item.path)) {
        let type_name = item.symbol.rsplit("::").next().expect("rustdoc type name");
        by_type_name
            .entry(type_name)
            .or_default()
            .push(canonical_path(&item.path));
    }
    let capability_roots = CAPABILITY_TYPE_IDENTITIES
        .iter()
        .chain(CLAIM_TYPE_IDENTITIES)
        .filter_map(|identity| {
            let matches = by_type_name.get(identity).cloned().unwrap_or_default();
            assert!(
                matches.len() <= 1,
                "capability type identity {identity:?} is ambiguous"
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
            if capability_types.contains(&canonical_path(&item.path)) || !is_type_page(&item.path) {
                continue;
            }
            if public_signature_links_to(&item.html, &item.path, &capability_types) {
                added |= capability_types.insert(canonical_path(&item.path));
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
    let item_is_capability = capability_types.contains(&canonical_path(&item.path));
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
            if snapshot && capability_roots.contains(&canonical_path(&item.path)) {
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
    let rest = after
        .split_once("id=\"trait-implementations")
        .map_or("", |(_, rest)| rest);
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
        if linked.is_file() && identities.contains(&canonical_path(&linked)) {
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

fn item_declaration(html: &str) -> Option<&str> {
    html.split_once("<pre class=\"rust item-decl\"><code>")?
        .1
        .split_once("</code></pre>")
        .map(|(declaration, _)| declaration)
}

fn code_header(section: &str) -> Option<&str> {
    section
        .split_once("<h4 class=\"code-header\">")?
        .1
        .split_once("</h4>")
        .map(|(header, _)| header)
}
