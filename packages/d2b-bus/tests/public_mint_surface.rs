use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use quote::ToTokens;
use syn::{Attribute, Item, Meta, Visibility, ext::IdentExt, visit::Visit};

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
    let updating = std::env::var_os("D2B_UPDATE_BUS_PUBLIC_API").is_some();
    // Regenerating the approved snapshots must never read a cached render.
    // The assert path below is an exact-set comparison and so fails closed on a
    // render that came back short, but the write path has no such check: it
    // would bake whatever the cache produced into a narrower allowlist, after
    // which every later run passes against the reduced inventory. Rendering
    // cold costs one slow run on a command that is already deliberate.
    if updating && std::env::var_os("D2B_BUS_PUBLIC_API_FRESH").is_none() {
        panic!(
            "refusing to rewrite the approved API snapshots from a possibly cached render; \
             re-run with D2B_BUS_PUBLIC_API_FRESH=1 D2B_UPDATE_BUS_PUBLIC_API=1 so the \
             workspace is rendered from scratch"
        );
    }
    let scratch = Scratch::cache(
        repository_root
            .join(".scratch")
            .join(format!("bus-public-api-{}", toolchain_cache_key())),
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
    if updating {
        write_snapshot(&crate_root.join("tests/approved-public-api.txt"), &actual);
        write_snapshot(
            &crate_root.join("tests/approved-capability-api.txt"),
            &capability_surface,
        );
        write_snapshot(
            &crate_root.join("tests/approved-hidden-public-api.txt"),
            &hidden_public.entries,
        );
        write_snapshot(
            &crate_root.join("tests/approved-capability-trait-impls.txt"),
            &capability_trait_impls.entries,
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

#[test]
fn workspace_capability_source_globs_are_classified() {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = crate_root.parent().unwrap().join("Cargo.toml");
    for package in workspace_render_packages(&manifest).into_values() {
        source_capability_inventory_with_externals(
            &package.crate_name,
            &package.source,
            &package.external_crates,
        );
    }
}

fn assert_mutation_fixture(workspace_docs: &[DocumentedCrate]) {
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let fixture = crate_root.join("tests/ui/public-api-mutations");
    // Ephemeral, unlike the workspace render above. The fixture is a small
    // separate workspace, so a persistent build directory saves little, and
    // caching it was observed to change what rustdoc emitted for the fixture
    // crate depending on the harness driving the test. Keeping this tree
    // per-process preserves the original semantics exactly where the payoff
    // does not justify the risk.
    let scratch = Scratch::ephemeral(
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
        .entries
        .iter()
        .find(|symbol| symbol.contains("hidden_rogue_admission"))
        .unwrap_or_else(|| {
            panic!("hidden rogue ComponentSessionAdmission factory escaped classification")
        });
    let hidden_rogue_name = hidden_rogue
        .split_once('\t')
        .map_or(hidden_rogue.as_str(), |(name, _)| name);
    let hidden_const_generic = hidden_public
        .entries
        .iter()
        .find(|symbol| symbol.contains("hidden_const_generic_path"))
        .expect("hidden const-generic mutation was inventoried");
    assert!(
        hidden_const_generic.contains("/home/alice/private/secret.rs"),
        "hidden API snapshot did not retain its rendered signature: {hidden_const_generic}"
    );
    let error = hidden_public_inventory_error(&hidden_public, &BTreeSet::new())
        .expect("hidden rogue ComponentSessionAdmission factory passed the inventory");
    assert!(
        error.contains(hidden_rogue_name)
            && error.contains("hidden public function")
            && error.contains("d2b_bus_public_api_mutations (lib.rs)"),
        "hidden public inventory did not identify the rogue admission factory with fixed \
         metadata: {error}"
    );
    assert!(
        !error.contains("/home/alice/private/secret.rs"),
        "hidden public inventory diagnostic echoed a const-generic path literal: {error}"
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
        ("trait-impl-cfg-unrenamed-import.rs", "Default"),
        ("trait-impl-cfg-attr-derive.rs", "Clone"),
        ("trait-impl-nested-cfg-attr-derive.rs", "Copy"),
        ("trait-impl-cfg-attr-gated.rs", "Default"),
        ("trait-impl-renamed-glob-target.rs", "From<LocalInput>"),
        (
            "trait-impl-group-renamed-glob-target.rs",
            "From<LocalInput>",
        ),
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
        ("trait-impl-raw-renamed-module.rs", "module alias", "cap"),
        (
            "trait-impl-plain-module.rs",
            "cannot resolve module alias aliases in impl self type aliases::Admission",
            "aliases",
        ),
        (
            "trait-impl-plain-self-module.rs",
            "cannot resolve module alias aliases in impl self type aliases::Admission",
            "aliases",
        ),
        (
            "trait-impl-alias-before-capability.rs",
            "cannot resolve module alias cap in impl self type cap::Admission",
            "cap",
        ),
        (
            "trait-impl-chained-renamed-module.rs",
            "cannot resolve module alias second in impl self type second::Admission",
            "second",
        ),
        (
            "trait-impl-chained-reexport-module.rs",
            "cannot resolve module alias second in impl self type second::Admission",
            "second",
        ),
        (
            "trait-impl-glob-module.rs",
            "cannot resolve module alias aliases in impl self type aliases::Admission",
            "aliases",
        ),
        (
            "trait-impl-group-glob-module.rs",
            "cannot resolve module alias aliases in impl self type aliases::Admission",
            "aliases",
        ),
        (
            "trait-impl-glob-nested-reexport.rs",
            "cannot resolve module alias wrapper in impl self type wrapper::nested::Admission",
            "wrapper",
        ),
        (
            "trait-impl-glob-unresolved-target.rs",
            "cannot resolve module alias aliases",
            "aliases",
        ),
        (
            "trait-impl-glob-unknown-destination.rs",
            "external or unresolved glob import",
            "aliases",
        ),
        (
            "trait-impl-glob-unresolved-two-hop.rs",
            "cannot resolve module alias aliases",
            "aliases",
        ),
        (
            "trait-impl-block-glob.rs",
            "cannot resolve block-local glob module alias aliases",
            "aliases",
        ),
        (
            "trait-impl-block-group-glob.rs",
            "cannot resolve block-local glob module alias aliases",
            "aliases",
        ),
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
    for name in [
        "trait-impl-noncapability-direct-renamed-module.rs",
        "trait-impl-noncapability-self-renamed-module.rs",
        "trait-impl-noncapability-plain-module.rs",
        "trait-impl-noncapability-plain-self-module.rs",
        "trait-impl-noncapability-chained-reexport-module.rs",
        "trait-impl-glob-cycle-shadowed.rs",
        "trait-impl-noncapability-block-glob.rs",
        "trait-impl-noncapability-renamed-glob-target.rs",
    ] {
        let source = fs::read_to_string(fixture.join(name))
            .unwrap_or_else(|error| panic!("read {name} compile-pass fixture: {error}"));
        let inventory = source_capability_inventory_from_text("d2b_bus", &source, name);
        assert!(
            inventory.trait_impls.is_empty(),
            "{name} incorrectly classified an ordinary impl as capability-relevant: {:?}",
            inventory.trait_impls
        );
    }
    assert_source_diagnostics_redact_attacker_content(&fixture);
    assert_tool_output_redaction(&fixture);
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
    let mut mutated = InventorySnapshot {
        entries: approved.clone(),
        diagnostics: BTreeMap::new(),
    };
    mutated.entries.extend(inventory.trait_impls);
    mutated.diagnostics.extend(inventory.trait_impl_diagnostics);
    let error = capability_trait_impl_inventory_error(&mutated, approved)
        .unwrap_or_else(|| panic!("{expected_trait} capability trait implementation passed"));
    assert!(
        error.contains("implementation d2b_bus::ComponentSessionAdmission")
            && error.contains(source_name),
        "trait-implementation inventory did not identify the rogue {expected_trait} \
         implementation with fixed metadata: {error}"
    );
}

fn assert_trait_impl_source_scan_fails_closed(
    source: &str,
    source_name: &str,
    expected_diagnostic: &str,
    expected_alias: &str,
) -> String {
    let failure = match std::panic::catch_unwind(|| {
        source_capability_inventory_from_text("d2b_bus", source, source_name)
    }) {
        Ok(_) => panic!("{source_name} unresolvable capability alias passed the source inventory"),
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
    diagnostic.to_owned()
}

fn assert_source_diagnostics_redact_attacker_content(fixture: &Path) {
    let const_generic_name = "trait-impl-const-generic-redaction.rs";
    let const_generic = fs::read_to_string(fixture.join(const_generic_name))
        .expect("read const-generic redaction fixture");
    let inventory =
        source_capability_inventory_from_text("d2b_bus", &const_generic, const_generic_name);
    let rendered = inventory
        .trait_impls
        .iter()
        .find(|entry| entry.contains("ComponentSessionAdmission"))
        .expect("const-generic capability impl was inventoried");
    assert!(
        rendered.contains("/home/alice/private/secret.rs"),
        "trait snapshot did not retain the rendered signature needed by the allowlist: {rendered}"
    );
    let actual = InventorySnapshot {
        entries: inventory.trait_impls,
        diagnostics: inventory.trait_impl_diagnostics,
    };
    let diagnostic = capability_trait_impl_inventory_error(&actual, &BTreeSet::new())
        .expect("const-generic capability impl passed the inventory");
    assert!(
        diagnostic.contains("explicit trait implementation")
            && diagnostic.contains("d2b_bus::ComponentSessionAdmission")
            && diagnostic.contains(const_generic_name)
            && !diagnostic.contains("/home/alice/private/secret.rs"),
        "trait inventory diagnostic did not separate fixed metadata from the rendered snapshot: \
         {diagnostic}"
    );

    let unsupported_alias_name = "trait-impl-unsupported-alias-redaction.rs";
    let unsupported_alias = fs::read_to_string(fixture.join(unsupported_alias_name))
        .expect("read unsupported alias redaction fixture");
    let alias_diagnostic = assert_trait_impl_source_scan_fails_closed(
        &unsupported_alias,
        unsupported_alias_name,
        "unsupported array type target for capability alias",
        "AdmissionAlias",
    );
    assert!(
        !alias_diagnostic.contains("/home/alice/private/alias.rs")
            && !alias_diagnostic.contains("PRIVATE_ALIAS_PATH")
            && !alias_diagnostic.contains("ComponentSessionAdmission;"),
        "unsupported-alias diagnostic echoed attacker-authored source: {alias_diagnostic}"
    );

    let unsupported_self_name = "trait-impl-unsupported-self-type-redaction.rs";
    let unsupported_self = fs::read_to_string(fixture.join(unsupported_self_name))
        .expect("read unsupported self-type redaction fixture");
    let failure = match std::panic::catch_unwind(|| {
        source_capability_inventory_from_text("d2b_bus", &unsupported_self, unsupported_self_name)
    }) {
        Ok(_) => panic!("unsupported capability self type passed the source inventory"),
        Err(failure) => failure,
    };
    let self_diagnostic = panic_message(&failure);
    assert!(
        self_diagnostic.contains("unsupported array type syntax")
            && self_diagnostic.contains(unsupported_self_name)
            && !self_diagnostic.contains("/home/alice/private/self-type.rs")
            && !self_diagnostic.contains("PRIVATE_SELF_TYPE_PATH")
            && !self_diagnostic.contains("ComponentSessionAdmission;"),
        "unsupported-self-type diagnostic was not fixed-label and redacted: {self_diagnostic}"
    );

    let parse_name = "parse-error-redaction.rs";
    let parse_source =
        fs::read_to_string(fixture.join(parse_name)).expect("read parse-error redaction fixture");
    let failure = match std::panic::catch_unwind(|| {
        source_capability_inventory_from_text("d2b_bus", &parse_source, parse_name)
    }) {
        Ok(_) => panic!("invalid Rust mutation source parsed successfully"),
        Err(failure) => failure,
    };
    let parse_diagnostic = panic_message(&failure);
    assert!(
        parse_diagnostic.contains(parse_name)
            && !parse_diagnostic.contains("/home/alice/private/parse.rs")
            && !parse_diagnostic.contains("PRIVATE_PARSE_PATH")
            && !parse_diagnostic.contains(parse_source.trim()),
        "parse diagnostic echoed attacker-authored source: {parse_diagnostic}"
    );
}

fn assert_tool_output_redaction(fixture: &Path) {
    use std::os::unix::process::ExitStatusExt;

    let attacker_output = fs::read(fixture.join("tool-output-redaction.txt"))
        .expect("read tool-output redaction fixture");
    let output = Output {
        status: std::process::ExitStatus::from_raw(23 << 8),
        stdout: Vec::new(),
        stderr: attacker_output.clone(),
    };
    let diagnostic = tool_failure("render rustdoc", "d2b_bus", &output);
    let attacker_output = String::from_utf8(attacker_output).expect("fixture is UTF-8");
    assert!(
        diagnostic.contains("render rustdoc")
            && diagnostic.contains("d2b_bus")
            && diagnostic.contains("23")
            && !diagnostic.contains(attacker_output.trim())
            && !diagnostic.contains("/home/alice/private"),
        "tool failure diagnostic exposed attacker-authored output or an absolute path: \
         {diagnostic}"
    );
}

fn panic_message(failure: &Box<dyn std::any::Any + Send>) -> &str {
    failure
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| failure.downcast_ref::<&str>().copied())
        .unwrap_or("<non-string panic>")
}

fn assert_module_source_mutations_fail_closed(crate_root: &Path) {
    let repository_root = crate_root.parent().unwrap().parent().unwrap();
    let scratch = Scratch::ephemeral(repository_root.join(".scratch").join(format!(
        "bus-source-module-mutations-{}",
        std::process::id()
    )));

    let cfg_attr = scratch.path().join("cfg-attr-path");
    fs::create_dir_all(&cfg_attr).expect("create cfg_attr module mutation");
    fs::write(
        cfg_attr.join("lib.rs"),
        "#[cfg_attr(all(), path = \"/home/alice/private/rogue.rs\")]\nmod router;\n",
    )
    .expect("write cfg_attr module mutation root");
    fs::write(cfg_attr.join("router.rs"), "struct Harmless;\n")
        .expect("write default module source");
    fs::write(
        cfg_attr.join("rogue.rs"),
        "struct ComponentSessionAdmission;\n",
    )
    .expect("write compiler-selected module source");
    let conditional_path_diagnostic = assert_source_file_scan_fails_closed(
        &cfg_attr.join("lib.rs"),
        &["conditional path attribute", "router", "d2b_bus (lib.rs)"],
    );
    assert!(
        !conditional_path_diagnostic.contains("/home/alice/private/rogue.rs")
            && !conditional_path_diagnostic.contains("path ="),
        "conditional-path diagnostic echoed untrusted attribute tokens: \
         {conditional_path_diagnostic}"
    );

    let unrecognised_attribute_diagnostic = assert_source_file_scan_fails_closed(
        &crate_root.join("tests/ui/module-cfg-attr-unrecognised/lib.rs"),
        &[
            "cfg_attr",
            "unrecognised conditional module attribute",
            "unrecognised_module_cfg_attr",
            "d2b_bus (lib.rs)",
            "allowlist",
        ],
    );
    assert!(
        !unrecognised_attribute_diagnostic.contains("rewrite_module")
            && !unrecognised_attribute_diagnostic.contains("security_tool")
            && !unrecognised_attribute_diagnostic.contains("ZoneRegistrar")
            && !unrecognised_attribute_diagnostic.contains("ComponentSessionAdmission")
            && !unrecognised_attribute_diagnostic.contains("/home/alice/private/attribute.rs")
            && !unrecognised_attribute_diagnostic.contains("path ="),
        "unrecognised-attribute diagnostic echoed untrusted attribute tokens: \
         {unrecognised_attribute_diagnostic}"
    );

    let direct_attribute_diagnostic = assert_source_file_scan_fails_closed(
        &crate_root.join("tests/ui/module-direct-attr-unrecognised/lib.rs"),
        &[
            "unsupported direct module attribute",
            "unrecognised_direct_module_attribute",
            "d2b_bus (lib.rs)",
            "exact proven-inert path and shape",
        ],
    );
    assert!(
        !direct_attribute_diagnostic.contains("inject_hidden_capability_impl")
            && !direct_attribute_diagnostic.contains("security_tool")
            && !direct_attribute_diagnostic.contains("ZoneRegistrar")
            && !direct_attribute_diagnostic.contains("ComponentSessionAdmission")
            && !direct_attribute_diagnostic.contains("/home/alice/private/direct-attribute.rs"),
        "direct-attribute diagnostic echoed untrusted attribute tokens: \
         {direct_attribute_diagnostic}"
    );

    let missing_module = scratch.path().join("missing-module");
    fs::create_dir_all(&missing_module).expect("create missing module fixture");
    fs::write(missing_module.join("lib.rs"), "mod absent;\n").expect("write missing module root");
    assert_source_file_scan_fails_closed(
        &missing_module.join("lib.rs"),
        &[
            "cannot resolve Rust module d2b_bus::absent",
            "d2b_bus (lib.rs)",
            "partial source scan",
        ],
    );

    let missing_path = scratch.path().join("missing-path");
    fs::create_dir_all(&missing_path).expect("create missing path fixture");
    fs::write(
        missing_path.join("lib.rs"),
        "#[path = \"/home/alice/private/missing.rs\"]\nmod absent;\n",
    )
    .expect("write missing path root");
    let missing_path_diagnostic = assert_source_file_scan_fails_closed(
        &missing_path.join("lib.rs"),
        &[
            "cannot resolve Rust module d2b_bus::absent",
            "d2b_bus (lib.rs)",
            "partial source scan",
        ],
    );
    assert!(
        !missing_path_diagnostic.contains("/home/alice/private/missing.rs"),
        "missing-path diagnostic echoed an untrusted path literal: {missing_path_diagnostic}"
    );

    let block_local_selected = scratch.path().join("block-local-selected");
    fs::create_dir_all(&block_local_selected).expect("create block-local selected fixture");
    fs::write(
        block_local_selected.join("lib.rs"),
        r#"
fn declare_local_module() {
    #[path = "selected.rs"]
    mod local;
}
"#,
    )
    .expect("write block-local selected root");
    fs::write(
        block_local_selected.join("selected.rs"),
        r#"
pub struct ComponentSessionAdmission;

impl From<()> for ComponentSessionAdmission {
    fn from(_: ()) -> Self {
        Self
    }
}
"#,
    )
    .expect("write block-local selected module");
    let block_local_inventory =
        source_capability_inventory("d2b_bus", &block_local_selected.join("lib.rs"));
    assert!(
        block_local_inventory
            .trait_impls
            .iter()
            .any(|implementation| {
                implementation.contains("ComponentSessionAdmission")
                    && implementation.contains("From<()>")
            }),
        "block-local external module did not contribute its capability impl: {:?}",
        block_local_inventory.trait_impls
    );

    let block_local_attribute = scratch.path().join("block-local-attribute");
    fs::create_dir_all(&block_local_attribute).expect("create block-local attribute fixture");
    fs::write(
        block_local_attribute.join("lib.rs"),
        r#"
fn declare_local_module() {
    #[security_tool::rewrite_module]
    mod local {}
}
"#,
    )
    .expect("write block-local attribute root");
    let block_attribute_diagnostic = assert_source_file_scan_fails_closed(
        &block_local_attribute.join("lib.rs"),
        &[
            "unsupported direct module attribute",
            "local",
            "d2b_bus (lib.rs)",
        ],
    );
    assert!(
        !block_attribute_diagnostic.contains("security_tool")
            && !block_attribute_diagnostic.contains("rewrite_module"),
        "block-local attribute diagnostic echoed untrusted tokens: {block_attribute_diagnostic}"
    );

    let block_local_missing = scratch.path().join("block-local-missing");
    fs::create_dir_all(&block_local_missing).expect("create block-local missing fixture");
    fs::write(
        block_local_missing.join("lib.rs"),
        "fn declare_local_module() { mod absent; }\n",
    )
    .expect("write block-local missing root");
    assert_source_file_scan_fails_closed(
        &block_local_missing.join("lib.rs"),
        &[
            "cannot resolve Rust module d2b_bus::absent",
            "d2b_bus (lib.rs)",
            "partial source scan",
        ],
    );

    let inert_cfg_attr = scratch.path().join("inert-cfg-attr");
    fs::create_dir_all(&inert_cfg_attr).expect("create inert cfg_attr module fixture");
    fs::write(
        inert_cfg_attr.join("lib.rs"),
        r#"#[cfg(all())]
#[rustfmt::skip]
#[cfg_attr(rustfmt, rustfmt::skip)]
#[cfg_attr(all(), doc = "ordinary module")]
#[cfg_attr(all(), cfg_attr(all(), allow(dead_code)))]
mod ordinary;
"#,
    )
    .expect("write inert cfg_attr module root");
    fs::write(
        inert_cfg_attr.join("ordinary.rs"),
        r#"
struct Request;

impl Default for Request {
    fn default() -> Self {
        Self
    }
}
"#,
    )
    .expect("write inert cfg_attr module source");
    let inert_inventory = source_capability_inventory("d2b_bus", &inert_cfg_attr.join("lib.rs"));
    assert!(
        inert_inventory.trait_impls.is_empty(),
        "inert module cfg_attr incorrectly made an ordinary impl capability-relevant: {:?}",
        inert_inventory.trait_impls
    );

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

    let path_child = scratch.path().join("path-child");
    fs::create_dir_all(path_child.join("chosen/outer"))
        .expect("create path-loaded child decoy directory");
    fs::write(
        path_child.join("lib.rs"),
        "#[path = \"chosen/outer.rs\"]\nmod outer;\n",
    )
    .expect("write path-loaded child root");
    fs::write(path_child.join("chosen/outer.rs"), "mod child;\n")
        .expect("write path-loaded parent source");
    fs::write(
        path_child.join("chosen/child.rs"),
        r#"
pub struct ComponentSessionAdmission;

impl From<()> for ComponentSessionAdmission {
    fn from(_value: ()) -> Self {
        Self
    }
}

#[doc(hidden)]
pub fn compiler_selected_child() {}
"#,
    )
    .expect("write compiler-selected path-loaded child");
    fs::write(
        path_child.join("chosen/outer/child.rs"),
        "#[doc(hidden)]\npub fn path_child_decoy() {}\n",
    )
    .expect("write path-loaded child decoy");
    assert_selected_module_sources(
        &path_child.join("lib.rs"),
        "compiler_selected_child",
        &["path_child_decoy"],
        true,
    );

    let raw_identifiers = scratch.path().join("raw-identifiers");
    fs::create_dir_all(raw_identifiers.join("inline"))
        .expect("create raw inline selected directory");
    fs::create_dir_all(raw_identifiers.join("r#inline"))
        .expect("create raw inline decoy directory");
    fs::write(
        raw_identifiers.join("lib.rs"),
        "mod r#router;\nmod r#inline { mod r#nested; }\n",
    )
    .expect("write raw identifier root");
    fs::write(
        raw_identifiers.join("router.rs"),
        r#"
pub struct r#ComponentSessionAdmission;

impl From<()> for r#ComponentSessionAdmission {
    fn from(_value: ()) -> Self {
        Self
    }
}

#[doc(hidden)]
pub fn raw_external_selected() {}
"#,
    )
    .expect("write raw external selected source");
    fs::write(
        raw_identifiers.join("r#router.rs"),
        "#[doc(hidden)]\npub fn raw_external_decoy() {}\n",
    )
    .expect("write raw external decoy");
    fs::write(
        raw_identifiers.join("inline/nested.rs"),
        "#[doc(hidden)]\npub fn raw_inline_selected() {}\n",
    )
    .expect("write raw inline selected source");
    fs::write(
        raw_identifiers.join("r#inline/r#nested.rs"),
        "#[doc(hidden)]\npub fn raw_inline_decoy() {}\n",
    )
    .expect("write raw inline decoy");
    let raw_capabilities = source_capability_inventory("d2b_bus", &raw_identifiers.join("lib.rs"));
    assert!(
        raw_capabilities.trait_impls.iter().any(|implementation| {
            implementation.contains("ComponentSessionAdmission")
                && implementation.contains("From<()>")
        }),
        "raw capability path escaped the trait inventory: {:?}",
        raw_capabilities.trait_impls
    );
    let raw_hidden = hidden_public_api("d2b_bus", &raw_identifiers.join("lib.rs"));
    for selected in ["raw_external_selected", "raw_inline_selected"] {
        assert!(
            raw_hidden.iter().any(|symbol| symbol.contains(selected)),
            "raw identifier scan missed {selected}: {raw_hidden:?}"
        );
    }
    for decoy in ["raw_external_decoy", "raw_inline_decoy"] {
        assert!(
            raw_hidden.iter().all(|symbol| !symbol.contains(decoy)),
            "raw identifier scan selected {decoy}: {raw_hidden:?}"
        );
    }

    #[cfg(unix)]
    assert_symlink_module_sources(&scratch);

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

fn assert_selected_module_sources(
    source: &Path,
    selected: &str,
    decoys: &[&str],
    expect_from_unit: bool,
) {
    let capability_inventory = source_capability_inventory("d2b_bus", source);
    if expect_from_unit {
        assert!(
            capability_inventory
                .trait_impls
                .iter()
                .any(|implementation| {
                    implementation.contains("ComponentSessionAdmission")
                        && implementation.contains("From<()>")
                }),
            "selected module source did not contribute its capability impl: {:?}",
            capability_inventory.trait_impls
        );
    }
    let hidden_inventory = hidden_public_api("d2b_bus", source);
    assert!(
        hidden_inventory
            .iter()
            .any(|symbol| symbol.contains(selected)),
        "module scan missed compiler-selected source {selected}: {hidden_inventory:?}"
    );
    for decoy in decoys {
        assert!(
            hidden_inventory
                .iter()
                .all(|symbol| !symbol.contains(decoy)),
            "module scan selected decoy {decoy}: {hidden_inventory:?}"
        );
    }
}

#[cfg(unix)]
fn assert_symlink_module_sources(scratch: &Scratch) {
    use std::os::unix::fs::symlink;

    let symlink_file = scratch.path().join("symlink-file");
    fs::create_dir_all(symlink_file.join("targets/actual"))
        .expect("create symlink file target decoy directory");
    fs::create_dir_all(symlink_file.join("linked"))
        .expect("create symlink file lexical module directory");
    fs::write(symlink_file.join("lib.rs"), "mod linked;\n").expect("write symlink file root");
    fs::write(symlink_file.join("targets/actual.rs"), "mod child;\n")
        .expect("write symlink file target");
    symlink("targets/actual.rs", symlink_file.join("linked.rs"))
        .expect("create symlinked Rust source file");
    fs::write(
        symlink_file.join("linked/child.rs"),
        r#"
pub struct ComponentSessionAdmission;

impl From<()> for ComponentSessionAdmission {
    fn from(_value: ()) -> Self {
        Self
    }
}

#[doc(hidden)]
pub fn symlink_file_selected() {}
"#,
    )
    .expect("write symlink file compiler-selected child");
    fs::write(
        symlink_file.join("targets/actual/child.rs"),
        "#[doc(hidden)]\npub fn symlink_file_decoy() {}\n",
    )
    .expect("write symlink file target-relative decoy");
    assert_selected_module_sources(
        &symlink_file.join("lib.rs"),
        "symlink_file_selected",
        &["symlink_file_decoy"],
        true,
    );

    let symlink_directory = scratch.path().join("symlink-directory");
    fs::create_dir_all(symlink_directory.join("target-dir"))
        .expect("create symlink directory target");
    fs::write(symlink_directory.join("lib.rs"), "mod linked;\n")
        .expect("write symlink directory root");
    fs::write(symlink_directory.join("target-dir/mod.rs"), "mod child;\n")
        .expect("write symlink directory module root");
    fs::write(
        symlink_directory.join("target-dir/child.rs"),
        "#[doc(hidden)]\npub fn symlink_directory_selected() {}\n",
    )
    .expect("write symlink directory selected child");
    symlink("target-dir", symlink_directory.join("linked"))
        .expect("create symlinked Rust module directory");
    assert_selected_module_sources(
        &symlink_directory.join("lib.rs"),
        "symlink_directory_selected",
        &[],
        false,
    );
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
    // Restore the page afterwards. The render tree is now a cache that outlives
    // the process, so deleting a page and leaving it deleted would poison every
    // later run: Cargo's doc fingerprint stays fresh, rustdoc is not re-run, and
    // the missing page resurfaces as "this is a doc-build problem" against a
    // crate nobody touched. Reading it back first also keeps the simulation
    // honest if the assertion below panics.
    let page = documented.root.join(&advertised.href);
    let preserved = fs::read(&page).expect("read the advertised item page before removing it");
    fs::remove_file(&page)
        .expect("remove one advertised item to simulate a partial rustdoc render");

    let outcome = validate_documented_crate(&documented.crate_name, &documented.root);
    fs::write(&page, &preserved).expect("restore the advertised item page");
    let error = outcome.expect_err("partial rustdoc render passed completeness validation");
    let symbol = format!("{}::{}", documented.crate_name, advertised.name);
    assert!(
        error.contains(&symbol) && error.contains("doc-build problem"),
        "partial-render failure did not name the missing advertised item: {error}"
    );
}

/// A directory-safe token identifying the toolchain that will drive the nested
/// `cargo doc` invocations.
///
/// The scratch trees cache compiled artifacts and rendered HTML, and neither is
/// portable across compiler versions. The gate provisions its own pinned
/// toolchain through rustup while a developer shell commonly has a different
/// one, so the same repository can be exercised by two rustc versions that
/// disagree about what a cached render should contain. Keying the cache path on
/// the toolchain gives each its own tree instead of letting them corrupt one.
fn toolchain_cache_key() -> String {
    let version = Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .unwrap_or_else(|| "unknown".to_owned());
    let token: String = version
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    token.trim_matches('-').to_owned()
}

/// A repository-local scratch tree that caches rustdoc's *compilation* work
/// across runs while keeping its *rendered output* per-run.
///
/// Rendering the workspace costs one `cargo doc` per package, and the tree used
/// to be per-process and deleted on drop, so every one of those invocations
/// recompiled the whole dependency graph from cold on every run. That is the
/// dominant cost of this test.
///
/// The split matters, and the two halves are not equally safe to reuse:
///
/// * `build/` (`CARGO_BUILD_BUILD_DIR`) holds compiled dependency artifacts and
///   is reused between runs. Cargo owns staleness through its own fingerprints,
///   so a surviving build directory cannot make a changed crate look unchanged.
///
/// * `renders/` holds rustdoc's HTML and mostly persists too, but
///   [`render_workspace_docs`] discards the render of any package whose lib and
///   bin targets collide on a single `doc/<crate>` directory. Cargo re-runs only
///   the target it considers dirty, and that target overwrites the shared
///   directory with its own pages alone; in the lib case the pages lost are
///   exactly the private items `--document-private-items` exists to surface, so
///   a cached render there can silently shrink the inventory this guard compares
///   against. Discarding only the colliding packages (8 of 35 in this
///   workspace) keeps the guard fail-closed while letting the rest stay cached,
///   which is where most of the speedup comes from.
///
/// Concurrency is delegated to Cargo's target-directory lock, which serializes
/// concurrent `cargo doc` invocations against the same directory. The only
/// non-Cargo mutations are directory creation and dependency symlink planting,
/// and both are idempotent (see `plant_dependency_doc_link`).
///
/// Set `D2B_BUS_PUBLIC_API_FRESH=1` to discard the compilation cache too and
/// render fully cold.
struct Scratch {
    path: PathBuf,
    kind: ScratchKind,
}

/// Whether a [`Scratch`] tree outlives the process that created it.
enum ScratchKind {
    /// Survives between runs and is addressed by a stable, process-independent
    /// path. Used for the workspace rustdoc render, where reusing compiled
    /// dependencies across runs is the entire win.
    ///
    /// A stable path is also what makes this leak-proof: an interrupted run
    /// leaves a directory the next run adopts, rather than stranding a
    /// multi-gigabyte per-process tree that nothing will ever collect.
    Cache,
    /// Deleted on drop, and addressed by a per-process path. Used for the small
    /// synthetic source trees the fail-closed mutation fixtures plant, which
    /// must never be reused because each case writes a differently-shaped
    /// module layout, and for the mutation fixture's own render.
    Ephemeral,
}

impl Scratch {
    fn cache(path: PathBuf) -> Self {
        if std::env::var_os("D2B_BUS_PUBLIC_API_FRESH").is_some() && path.exists() {
            fs::remove_dir_all(&path).expect("discard repository-local rustdoc cache");
        }
        fs::create_dir_all(&path).expect("create repository-local scratch");
        Self {
            path,
            kind: ScratchKind::Cache,
        }
    }

    fn ephemeral(path: PathBuf) -> Self {
        if path.exists() {
            fs::remove_dir_all(&path).expect("remove stale repository-local scratch");
        }
        fs::create_dir_all(&path).expect("create repository-local scratch");
        Self {
            path,
            kind: ScratchKind::Ephemeral,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if matches!(self.kind, ScratchKind::Ephemeral) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Point `<doc_root>/<crate_name>` at an already-rendered dependency doc tree.
///
/// Rustdoc resolves cross-crate trait and Deref regions through sibling
/// directories under its own output root, so each package render needs its
/// dependencies' rendered trees visible there. The cache in [`Scratch`]
/// survives between runs, so this has to converge rather than assume a clean
/// tree: a link left by an earlier run may be correct, may point at a path that
/// has since moved, or may dangle entirely. `Path::exists` follows symlinks and
/// so reports `false` for a dangling one, which would make a naive
/// create-if-absent call fail with `EEXIST`.
///
/// Replacing only what does not already match keeps the common warm path free
/// of filesystem churn while still repairing a stale or dangling link.
fn plant_dependency_doc_link(
    doc_root: &Path,
    crate_name: &str,
    target: &Path,
) -> std::io::Result<()> {
    let link = doc_root.join(crate_name);
    match fs::read_link(&link) {
        // Already pointing where we want it.
        Ok(existing) if existing == target => return Ok(()),
        // A symlink to somewhere else, or a dangling one: replace it.
        Ok(_) => fs::remove_file(&link)?,
        // Not a symlink. A real directory here is a rendered crate Cargo owns,
        // which is exactly what we would have linked to, so leave it alone.
        Err(_) if link.exists() => return Ok(()),
        Err(_) => {}
    }
    std::os::unix::fs::symlink(target, &link)
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
    hidden_public_diagnostics: BTreeMap<String, InventoryDiagnostic>,
    capability_declarations: BTreeMap<String, BTreeSet<String>>,
    capability_trait_impls: BTreeSet<String>,
    capability_trait_impl_diagnostics: BTreeMap<String, InventoryDiagnostic>,
}

#[derive(Debug, Clone)]
struct InventoryDiagnostic {
    syntax_kind: &'static str,
    identity: String,
    source: String,
}

impl InventoryDiagnostic {
    fn render(&self) -> String {
        format!("{} {} at {}", self.syntax_kind, self.identity, self.source)
    }
}

#[derive(Default)]
struct InventorySnapshot {
    entries: BTreeSet<String>,
    diagnostics: BTreeMap<String, InventoryDiagnostic>,
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
    external_crates: BTreeSet<String>,
    workspace_dependencies: BTreeSet<String>,
    /// Whether this package has a binary target that rustdoc renders into the
    /// same `doc/<crate>` directory as its library target.
    ///
    /// Such a package cannot reuse a cached render. Cargo re-runs only the
    /// target it considers dirty, and that target then overwrites the shared
    /// directory with its own pages alone - dropping, in the lib case, exactly
    /// the private items `--document-private-items` exists to surface. Only
    /// these packages need their render discarded before each run; the rest
    /// reuse theirs, which is where the speedup comes from.
    doc_name_collision: bool,
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
    let packages = workspace_render_packages(manifest);
    let temp = scratch.join("tmp");
    fs::create_dir_all(&temp).expect("create rustdoc temporary directory");
    // Compiled artifacts persist across runs. Renders persist too, except for
    // the packages whose lib and bin collide on one output directory - those
    // are discarded per run. See RenderPackage::doc_name_collision and the
    // `Scratch` docs.
    let build = scratch.join("build");
    let renders = scratch.join("renders");
    let mut docs = Vec::new();
    for (package_name, package) in dependency_order(packages) {
        let crate_name = package.crate_name;
        let hidden_public = hidden_public_api(&crate_name, &package.source);
        let source_capabilities = source_capability_inventory_with_externals(
            &crate_name,
            &package.source,
            &package.external_crates,
        );
        let target = renders.join(&crate_name);
        if package.doc_name_collision && target.exists() {
            fs::remove_dir_all(&target).unwrap_or_else(|_| {
                panic!("discard colliding rustdoc render for package {package_name}")
            });
        }
        let doc_root = target.join("doc");
        fs::create_dir_all(&doc_root).unwrap_or_else(|_| {
            panic!("create isolated rustdoc output for package {package_name}")
        });
        for documented in external_docs.iter().chain(docs.iter()) {
            plant_dependency_doc_link(&doc_root, &documented.crate_name, &documented.root)
                .unwrap_or_else(|error| {
                    panic!(
                        "link rustdoc dependency {} while rendering package {package_name}: {error}",
                        documented.crate_name
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
            .unwrap_or_else(|_| panic!("start rustdoc for package {package_name}"));
        if !output.status.success() {
            panic!("{}", tool_failure("render rustdoc", &package_name, &output));
        }

        let root = target.join("doc").join(&crate_name);
        let advertised =
            validate_documented_crate(&crate_name, &root).unwrap_or_else(|error| panic!("{error}"));
        docs.push(DocumentedCrate {
            crate_name,
            root,
            advertised,
            hidden_public: hidden_public.entries,
            hidden_public_diagnostics: hidden_public.diagnostics,
            capability_declarations: source_capabilities.declarations,
            capability_trait_impls: source_capabilities.trait_impls,
            capability_trait_impl_diagnostics: source_capabilities.trait_impl_diagnostics,
        });
    }
    docs.sort_by(|left, right| left.crate_name.cmp(&right.crate_name));
    docs
}

fn workspace_render_packages(manifest: &Path) -> BTreeMap<String, RenderPackage> {
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
        .unwrap_or_else(|_| panic!("start Cargo metadata for workspace"));
    if !metadata.status.success() {
        panic!(
            "{}",
            tool_failure("run Cargo metadata", "workspace", &metadata)
        );
    }
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
            // rustdoc derives a target's output directory from its crate name,
            // so a binary whose name normalises to the library's name renders
            // into the same directory. See RenderPackage::doc_name_collision.
            let doc_name_collision = package["targets"]
                .as_array()
                .expect("package targets")
                .iter()
                .filter(|target| {
                    target["kind"]
                        .as_array()
                        .expect("target kind")
                        .iter()
                        .any(|kind| kind == "bin")
                })
                .any(|target| {
                    target["name"]
                        .as_str()
                        .expect("binary target name")
                        .replace('-', "_")
                        == crate_name.replace('-', "_")
                });
            let mut dependency_features = BTreeSet::new();
            let mut external_crates = BTreeSet::new();
            let mut workspace_dependencies = BTreeSet::new();
            for dependency in package["dependencies"]
                .as_array()
                .expect("package dependencies")
                .iter()
                .filter(|dependency| dependency["kind"].is_null())
            {
                let dependency_name = dependency["name"].as_str().expect("dependency name");
                let command_name = dependency["rename"].as_str().unwrap_or(dependency_name);
                external_crates.insert(command_name.replace('-', "_"));
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
                    external_crates,
                    workspace_dependencies,
                    doc_name_collision,
                },
            );
        }
    }
    assert!(!packages.is_empty(), "workspace has no library crates");
    packages
}

fn tool_failure(operation: &str, identity: &str, output: &Output) -> String {
    format!(
        "{operation} failed for {identity} with status {}",
        output.status
    )
}

fn workspace_hidden_public_api(docs: &[DocumentedCrate]) -> InventorySnapshot {
    let mut inventory = InventorySnapshot::default();
    for documented in docs {
        inventory
            .entries
            .extend(documented.hidden_public.iter().cloned());
        inventory
            .diagnostics
            .extend(documented.hidden_public_diagnostics.clone());
    }
    inventory
}

fn workspace_capability_trait_impls(docs: &[DocumentedCrate]) -> InventorySnapshot {
    let mut declarations = BTreeMap::<String, BTreeSet<String>>::new();
    let mut inventory = InventorySnapshot::default();
    for documented in docs {
        for (identity, locations) in &documented.capability_declarations {
            declarations
                .entry(identity.clone())
                .or_default()
                .extend(locations.iter().cloned());
        }
        inventory
            .entries
            .extend(documented.capability_trait_impls.iter().cloned());
        inventory
            .diagnostics
            .extend(documented.capability_trait_impl_diagnostics.clone());
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
    inventory
}

fn assert_capability_trait_impl_inventory(actual: &InventorySnapshot, approved: &BTreeSet<String>) {
    if let Some(error) = capability_trait_impl_inventory_error(actual, approved) {
        panic!("{error}");
    }
}

fn capability_trait_impl_inventory_error(
    actual: &InventorySnapshot,
    approved: &BTreeSet<String>,
) -> Option<String> {
    let unapproved = actual
        .entries
        .difference(approved)
        .take(40)
        .map(|entry| {
            actual.diagnostics.get(entry).map_or_else(
                || "unapproved trait implementation at unknown source".to_owned(),
                InventoryDiagnostic::render,
            )
        })
        .collect::<Vec<_>>();
    let missing = approved
        .iter()
        .filter(|entry| !actual.entries.contains(*entry))
        .count();
    if !unapproved.is_empty() {
        return Some(format!(
            "a trait implementation on a capability type is outside the \
             explicitly approved inventory; unapproved {} (first 40: \
             {unapproved:?}). Review whether this trait can mint, clone, or \
             otherwise widen the capability before updating \
             approved-capability-trait-impls.txt.",
            actual.entries.difference(approved).count()
        ));
    }
    if missing == 0 {
        None
    } else {
        Some(format!(
            "{missing} approved capability trait implementations are absent; review whether \
             each implementation was intentionally removed before updating \
             approved-capability-trait-impls.txt"
        ))
    }
}

fn assert_hidden_public_inventory(actual: &InventorySnapshot, approved: &BTreeSet<String>) {
    if let Some(error) = hidden_public_inventory_error(actual, approved) {
        panic!("{error}");
    }
}

fn hidden_public_inventory_error(
    actual: &InventorySnapshot,
    approved: &BTreeSet<String>,
) -> Option<String> {
    let unapproved = actual
        .entries
        .difference(approved)
        .take(40)
        .map(|entry| {
            actual.diagnostics.get(entry).map_or_else(
                || "unapproved hidden public item at unknown source".to_owned(),
                InventoryDiagnostic::render,
            )
        })
        .collect::<Vec<_>>();
    let missing = approved
        .iter()
        .filter(|entry| !actual.entries.contains(*entry))
        .count();
    if !unapproved.is_empty() {
        return Some(format!(
            "a public doc(hidden) signature is outside the reviewed hidden API; \
                 unapproved {} (first 40: {unapproved:?}). This inventory is required \
                 because the pinned stable rustdoc does not render hidden items.",
            actual.entries.difference(approved).count()
        ));
    }
    if missing == 0 {
        None
    } else {
        Some(format!(
            "{missing} reviewed public doc(hidden) signatures are absent; review whether the \
             API was intentionally removed"
        ))
    }
}

fn hidden_public_api(crate_name: &str, source: &Path) -> HiddenPublicInventory {
    let source_root = source
        .parent()
        .expect("library target source has a parent directory")
        .to_path_buf();
    let mut scanner = HiddenPublicScanner {
        crate_name,
        source_root,
        entries: BTreeSet::new(),
        diagnostics: BTreeMap::new(),
        visited: BTreeMap::new(),
    };
    scanner.scan_file(source, &[], SourceFileKind::CrateRoot, false);
    HiddenPublicInventory {
        entries: scanner.entries,
        diagnostics: scanner.diagnostics,
    }
}

#[derive(Debug)]
struct HiddenPublicInventory {
    entries: BTreeSet<String>,
    diagnostics: BTreeMap<String, InventoryDiagnostic>,
}

impl HiddenPublicInventory {
    fn iter(&self) -> impl Iterator<Item = &String> {
        self.entries.iter()
    }
}

// This syntax-level inventory supplies best-effort breadth beyond the
// compiler-checked negative bounds on the enumerated minting traits. Glob
// resolution is bounded to parsed bindings and declared local modules; an impl
// reached through an unresolved, ambiguous, or otherwise unmodelled glob fails
// closed. This is not a replacement for rustc name or module resolution, and
// macro or include expansion can escape it. The primary boundary remains
// construction through private types, private fields, sealed traits, and
// consumed capabilities.
#[derive(Default)]
struct SourceCapabilityInventory {
    declarations: BTreeMap<String, BTreeSet<String>>,
    trait_impls: BTreeSet<String>,
    trait_impl_diagnostics: BTreeMap<String, InventoryDiagnostic>,
}

fn source_capability_inventory(crate_name: &str, source: &Path) -> SourceCapabilityInventory {
    source_capability_inventory_with_externals(crate_name, source, &BTreeSet::new())
}

fn source_capability_inventory_with_externals(
    crate_name: &str,
    source: &Path,
    external_crates: &BTreeSet<String>,
) -> SourceCapabilityInventory {
    let source_root = source
        .parent()
        .expect("library target source has a parent directory")
        .to_path_buf();
    let mut scanner = CapabilitySourceScanner {
        crate_name,
        source_root,
        external_crates,
        facts: SourceCapabilityFacts::default(),
        visited: BTreeMap::new(),
    };
    scanner.scan_file(source, &[], SourceFileKind::CrateRoot);
    scanner.finish()
}

fn source_capability_inventory_from_text(
    crate_name: &str,
    text: &str,
    source_name: &str,
) -> SourceCapabilityInventory {
    let file = syn::parse_file(text)
        .unwrap_or_else(|error| panic!("parse Rust mutation source {source_name}: {error}"));
    let mut facts = SourceCapabilityFacts::default();
    CapabilitySourceCollector {
        source: source_name.to_owned(),
        module_path: Vec::new(),
        lexical_scope: Vec::new(),
        facts: &mut facts,
    }
    .visit_file(&file);
    finish_source_capability_inventory(crate_name, facts, &BTreeSet::new())
}

struct CapabilitySourceScanner<'a> {
    crate_name: &'a str,
    source_root: PathBuf,
    external_crates: &'a BTreeSet<String>,
    facts: SourceCapabilityFacts,
    visited: BTreeMap<PathBuf, Vec<String>>,
}

#[derive(Clone, Copy)]
enum SourceFileKind {
    CrateRoot,
    OrdinaryModule,
    PathLoadedModule,
}

struct ResolvedModuleSource {
    path: PathBuf,
    kind: SourceFileKind,
}

impl CapabilitySourceScanner<'_> {
    fn scan_file(&mut self, source: &Path, module_path: &[String], kind: SourceFileKind) {
        let logical_source =
            source_location(self.crate_name, &self.source_root, source, module_path);
        let canonical_source = fs::canonicalize(source)
            .unwrap_or_else(|error| panic!("canonicalize Rust source {logical_source}: {error}"));
        if let Some(previous) = self.visited.get(&canonical_source) {
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
        self.visited.insert(canonical_source, module_path.to_vec());
        let text = fs::read_to_string(source)
            .unwrap_or_else(|error| panic!("read Rust source {logical_source}: {error}"));
        let file = syn::parse_file(&text)
            .unwrap_or_else(|error| panic!("parse Rust source {logical_source}: {error}"));
        CapabilitySourceCollector {
            source: logical_source.clone(),
            module_path: module_path.to_vec(),
            lexical_scope: Vec::new(),
            facts: &mut self.facts,
        }
        .visit_file(&file);
        let module_dir = source_module_dir(source, kind);
        let path_base = source
            .parent()
            .expect("lexical Rust source has a parent directory");
        let mut module_scanner = CapabilityModuleScanner {
            scanner: self,
            module_path: module_path.to_vec(),
            module_dir,
            path_base: path_base.to_path_buf(),
            logical_source,
        };
        for item in &file.items {
            module_scanner.visit_item(item);
        }
    }

    fn finish(self) -> SourceCapabilityInventory {
        finish_source_capability_inventory(self.crate_name, self.facts, self.external_crates)
    }
}

struct CapabilityModuleScanner<'scanner, 'crate_name> {
    scanner: &'scanner mut CapabilitySourceScanner<'crate_name>,
    module_path: Vec<String>,
    module_dir: PathBuf,
    path_base: PathBuf,
    logical_source: String,
}

impl<'ast> Visit<'ast> for CapabilityModuleScanner<'_, '_> {
    fn visit_item_mod(&mut self, module: &'ast syn::ItemMod) {
        let module_name = ident_name(&module.ident);
        let child_dir = self.module_dir.join(&module_name);
        let mut child_path = self.module_path.clone();
        child_path.push(module_name);
        if let Some((_, items)) = &module.content {
            let inline_dir = module_path_override(module, &self.path_base, &self.logical_source)
                .unwrap_or(child_dir);
            let mut child_scanner = CapabilityModuleScanner {
                scanner: &mut *self.scanner,
                module_path: child_path,
                module_dir: inline_dir.clone(),
                path_base: inline_dir,
                logical_source: self.logical_source.clone(),
            };
            for item in items {
                child_scanner.visit_item(item);
            }
        } else {
            let source = module_source(
                module,
                &self.module_dir,
                &self.path_base,
                &child_dir,
                &self.logical_source,
            )
            .unwrap_or_else(|| {
                panic!(
                    "cannot resolve Rust module {}::{} in {}; capability trait inventory \
                     refuses a partial source scan",
                    self.scanner.crate_name,
                    child_path.join("::"),
                    self.logical_source
                )
            });
            self.scanner
                .scan_file(&source.path, &child_path, source.kind);
        }
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
    Unsupported {
        syntax_kind: &'static str,
        identifiers: BTreeSet<String>,
    },
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
    visibility_scope: Vec<String>,
    source: String,
}

#[derive(Clone)]
struct SourceModuleAlias {
    binding: SourceBinding,
    target: SourcePath,
    declared_target: Option<Vec<String>>,
    visibility_scope: Vec<String>,
    lexical_scope: Vec<usize>,
}

#[derive(Clone)]
struct SourceGlob {
    module_path: Vec<String>,
    target: SourcePath,
    conditional: bool,
    visibility_scope: Vec<String>,
    lexical_scope: Vec<usize>,
    source: String,
}

#[derive(Clone)]
struct SourceDeclaration {
    identity: syn::Ident,
    kind: &'static str,
    attributes: Vec<Attribute>,
    module_path: Vec<String>,
    visibility_scope: Vec<String>,
    source: String,
}

#[derive(Clone)]
struct SourceImpl {
    implementation: syn::ItemImpl,
    module_path: Vec<String>,
    lexical_scope: Vec<usize>,
    source: String,
}

#[derive(Default)]
struct SourceCapabilityFacts {
    aliases: Vec<SourceAlias>,
    declarations: Vec<SourceDeclaration>,
    globs: Vec<SourceGlob>,
    implementations: Vec<SourceImpl>,
    module_paths: BTreeSet<Vec<String>>,
    module_aliases: Vec<SourceModuleAlias>,
    next_lexical_scope: usize,
}

struct CapabilitySourceCollector<'a> {
    source: String,
    module_path: Vec<String>,
    lexical_scope: Vec<usize>,
    facts: &'a mut SourceCapabilityFacts,
}

impl CapabilitySourceCollector<'_> {
    fn record_declaration(
        &mut self,
        identity: &syn::Ident,
        kind: &'static str,
        attributes: &[Attribute],
        visibility: &Visibility,
    ) {
        self.facts.declarations.push(SourceDeclaration {
            identity: identity.clone(),
            kind,
            attributes: attributes.to_vec(),
            module_path: self.module_path.clone(),
            visibility_scope: visibility_scope(visibility, &self.module_path),
            source: self.source.clone(),
        });
    }

    fn record_impl(&mut self, implementation: &syn::ItemImpl) {
        self.facts.implementations.push(SourceImpl {
            implementation: implementation.clone(),
            module_path: self.module_path.clone(),
            lexical_scope: self.lexical_scope.clone(),
            source: self.source.clone(),
        });
    }

    fn record_type_alias(&mut self, alias: &syn::ItemType) {
        let target = match alias.ty.as_ref() {
            syn::Type::Path(path) if path.qself.is_none() => {
                SourceAliasTarget::Path(source_path(&path.path))
            }
            other => SourceAliasTarget::Unsupported {
                syntax_kind: source_type_syntax_kind(other),
                identifiers: source_type_identifiers(other),
            },
        };
        self.facts.aliases.push(SourceAlias {
            binding: SourceBinding {
                module_path: self.module_path.clone(),
                name: ident_name(&alias.ident),
            },
            target,
            generic: !alias.generics.params.is_empty() || alias.generics.where_clause.is_some(),
            conditional: conditional_attributes(&alias.attrs),
            fail_if_conditional: true,
            fail_if_unresolved: true,
            lexical_scope: !self.lexical_scope.is_empty(),
            visibility_scope: visibility_scope(&alias.vis, &self.module_path),
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
                lexical_scope: self.lexical_scope.clone(),
                visibility_scope: visibility_scope(&item.vis, &self.module_path),
            },
            &self.source,
            self.facts,
        );
    }
}

impl<'ast> Visit<'ast> for CapabilitySourceCollector<'_> {
    fn visit_item_struct(&mut self, item: &'ast syn::ItemStruct) {
        self.record_declaration(&item.ident, "struct", &item.attrs, &item.vis);
        syn::visit::visit_item_struct(self, item);
    }

    fn visit_item_enum(&mut self, item: &'ast syn::ItemEnum) {
        self.record_declaration(&item.ident, "enum", &item.attrs, &item.vis);
        syn::visit::visit_item_enum(self, item);
    }

    fn visit_item_union(&mut self, item: &'ast syn::ItemUnion) {
        self.record_declaration(&item.ident, "union", &item.attrs, &item.vis);
        syn::visit::visit_item_union(self, item);
    }

    fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
        self.record_declaration(&item.ident, "trait", &item.attrs, &item.vis);
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
        let module_name = ident_name(&item.ident);
        let mut declared_path = self.module_path.clone();
        declared_path.push(module_name.clone());
        self.facts.module_paths.insert(declared_path.clone());
        let mut target = vec!["crate".to_owned()];
        target.extend(self.module_path.iter().cloned());
        target.push(module_name.clone());
        self.facts.module_aliases.push(SourceModuleAlias {
            binding: SourceBinding {
                module_path: self.module_path.clone(),
                name: module_name,
            },
            target: SourcePath {
                leading_colon: false,
                segments: target,
            },
            declared_target: Some(declared_path),
            visibility_scope: visibility_scope(&item.vis, &self.module_path),
            lexical_scope: self.lexical_scope.clone(),
        });
        let Some((_, items)) = &item.content else {
            return;
        };
        self.module_path.push(ident_name(&item.ident));
        for item in items {
            self.visit_item(item);
        }
        self.module_path.pop();
    }

    fn visit_block(&mut self, block: &'ast syn::Block) {
        let scope = self.facts.next_lexical_scope;
        self.facts.next_lexical_scope += 1;
        self.lexical_scope.push(scope);
        syn::visit::visit_block(self, block);
        self.lexical_scope.pop();
    }
}

fn source_path(path: &syn::Path) -> SourcePath {
    SourcePath {
        leading_colon: path.leading_colon.is_some(),
        segments: path
            .segments
            .iter()
            .map(|segment| ident_name(&segment.ident))
            .collect(),
    }
}

fn visibility_scope(visibility: &Visibility, module_path: &[String]) -> Vec<String> {
    match visibility {
        Visibility::Public(_) => Vec::new(),
        Visibility::Inherited => module_path.to_vec(),
        Visibility::Restricted(restricted) => {
            resolve_module_path(&source_path(&restricted.path), module_path, "crate")
                .unwrap_or_else(|| module_path.to_vec())
        }
    }
}

#[derive(Clone)]
struct SourceUseContext {
    conditional: bool,
    lexical_scope: Vec<usize>,
    visibility_scope: Vec<String>,
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
            prefix.push(ident_name(&path.ident));
            collect_use_bindings(
                &path.tree,
                leading_colon,
                prefix,
                module_path,
                context.clone(),
                source,
                facts,
            );
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let mut segments = prefix.clone();
            let imported_name = ident_name(&name.ident);
            let binding_name = if imported_name == "self" {
                let Some(binding_name) = segments.last().cloned() else {
                    return;
                };
                binding_name
            } else {
                segments.push(imported_name.clone());
                imported_name.clone()
            };
            facts.module_aliases.push(SourceModuleAlias {
                binding: SourceBinding {
                    module_path: module_path.to_vec(),
                    name: binding_name,
                },
                target: SourcePath {
                    leading_colon,
                    segments: segments.clone(),
                },
                declared_target: None,
                visibility_scope: context.visibility_scope.clone(),
                lexical_scope: context.lexical_scope.clone(),
            });
            if imported_name == "self" {
                return;
            }
            facts.aliases.push(SourceAlias {
                binding: SourceBinding {
                    module_path: module_path.to_vec(),
                    name: imported_name,
                },
                target: SourceAliasTarget::Path(SourcePath {
                    leading_colon,
                    segments,
                }),
                generic: false,
                conditional: context.conditional,
                fail_if_conditional: false,
                fail_if_unresolved: false,
                lexical_scope: !context.lexical_scope.is_empty(),
                visibility_scope: context.visibility_scope.clone(),
                source: source.to_owned(),
            });
        }
        syn::UseTree::Rename(rename) => {
            let binding = SourceBinding {
                module_path: module_path.to_vec(),
                name: ident_name(&rename.rename),
            };
            let mut segments = prefix.clone();
            let renamed_ident = ident_name(&rename.ident);
            if renamed_ident != "self" {
                segments.push(renamed_ident);
            }
            let target = SourcePath {
                leading_colon,
                segments,
            };
            facts.module_aliases.push(SourceModuleAlias {
                binding: binding.clone(),
                target: target.clone(),
                declared_target: None,
                visibility_scope: context.visibility_scope.clone(),
                lexical_scope: context.lexical_scope.clone(),
            });
            if ident_name(&rename.ident) == "self" {
                return;
            }
            facts.aliases.push(SourceAlias {
                binding,
                target: SourceAliasTarget::Path(target),
                generic: false,
                conditional: context.conditional,
                fail_if_conditional: true,
                fail_if_unresolved: true,
                lexical_scope: !context.lexical_scope.is_empty(),
                visibility_scope: context.visibility_scope.clone(),
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
            visibility_scope: context.visibility_scope,
            lexical_scope: context.lexical_scope,
            source: source.to_owned(),
        }),
        syn::UseTree::Group(group) => {
            for item in &group.items {
                collect_use_bindings(
                    item,
                    leading_colon,
                    prefix,
                    module_path,
                    context.clone(),
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
    external_crates: &BTreeSet<String>,
) -> SourceCapabilityInventory {
    let mut inventory = SourceCapabilityInventory::default();
    let mut resolved = BTreeMap::<SourceBinding, String>::new();
    let mut capability_visibility = BTreeMap::<SourceBinding, BTreeSet<Vec<String>>>::new();
    for declaration in &facts.declarations {
        let identity = ident_name(&declaration.identity);
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
        capability_visibility
            .entry(binding)
            .or_default()
            .insert(declaration.visibility_scope.clone());
        record_capability_derives(crate_name, declaration, &mut inventory);
    }

    let resolved_module_aliases = resolve_module_aliases_to_fixed_point(
        &facts.module_aliases,
        &facts.globs,
        &facts.module_paths,
        external_crates,
        crate_name,
    );
    let alias_bindings = facts
        .aliases
        .iter()
        .map(|alias| alias.binding.clone())
        .collect::<BTreeSet<_>>();
    let explicit_names = facts
        .declarations
        .iter()
        .map(|declaration| SourceBinding {
            module_path: declaration.module_path.clone(),
            name: ident_name(&declaration.identity),
        })
        .chain(facts.aliases.iter().map(|alias| alias.binding.clone()))
        .chain(
            facts
                .module_aliases
                .iter()
                .filter(|alias| alias.lexical_scope.is_empty())
                .map(|alias| alias.binding.clone()),
        )
        .collect::<BTreeSet<_>>();
    let capability_binding_universe = explicit_names.len()
        + facts
            .globs
            .iter()
            .filter(|glob| glob.lexical_scope.is_empty())
            .count()
            * explicit_names.len()
        + 1;
    let capability_visibility_universe = facts
        .aliases
        .iter()
        .map(|alias| alias.visibility_scope.clone())
        .chain(facts.globs.iter().map(|glob| glob.visibility_scope.clone()))
        .collect::<BTreeSet<_>>()
        .len()
        .max(1);
    let capability_binding_budget =
        capability_binding_universe * (capability_visibility_universe + 1);
    for iteration in 0..=capability_binding_budget {
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
            changed |= capability_visibility
                .entry(alias.binding.clone())
                .or_default()
                .insert(alias.visibility_scope.clone());
        }
        for glob in facts
            .globs
            .iter()
            .filter(|glob| glob.lexical_scope.is_empty())
        {
            let target = resolve_module_alias_target(
                &glob.target,
                &glob.module_path,
                crate_name,
                &resolved_module_aliases.modules,
                &resolved_module_aliases.binding_universe,
                &resolved_module_aliases.known_modules,
                &resolved_module_aliases.external_crates,
                &resolved_module_aliases.tainted_bindings,
            );
            if target.unresolved
                || target.tainted
                || target.modules.len() != 1
                || target
                    .modules
                    .iter()
                    .any(|module| resolved_module_aliases.tainted_modules.contains(module))
            {
                continue;
            }
            let mut imported = Vec::new();
            for target_module in &target.modules {
                for (binding, identity) in &resolved {
                    if &binding.module_path != target_module {
                        continue;
                    }
                    let visible = capability_visibility.get(binding).is_some_and(|scopes| {
                        scopes
                            .iter()
                            .any(|scope| glob.module_path.starts_with(scope))
                    });
                    if visible {
                        imported.push((binding.name.clone(), identity.clone()));
                    }
                }
            }
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
                if explicit_names.contains(&binding) {
                    continue;
                }
                changed |=
                    insert_resolved_binding(&mut resolved, &binding, &identity, &glob.source);
                changed |= capability_visibility
                    .entry(binding)
                    .or_default()
                    .insert(glob.visibility_scope.clone());
            }
        }
        if !changed {
            break;
        }
        assert!(
            iteration < capability_binding_budget,
            "capability glob propagation exceeded its finite binding budget"
        );
    }
    let mut module_alias_bindings = facts
        .module_aliases
        .iter()
        .filter(|alias| {
            if !alias.lexical_scope.is_empty() {
                return true;
            }
            if resolved_module_aliases
                .external_bindings
                .contains(&alias.binding)
            {
                return false;
            }
            resolved_module_aliases
                .modules
                .get(&alias.binding)
                .is_none_or(|target_modules| {
                    resolved_module_aliases
                        .tainted_bindings
                        .contains(&alias.binding)
                        || target_modules.len() != 1
                        || target_modules.iter().any(|target_module| {
                            resolved
                                .keys()
                                .any(|binding| binding.module_path.starts_with(target_module))
                                || resolved_module_aliases
                                    .tainted_modules
                                    .iter()
                                    .any(|module| module.starts_with(target_module))
                        })
                })
        })
        .map(|alias| alias.binding.clone())
        .collect::<BTreeSet<_>>();
    module_alias_bindings.extend(resolved_module_aliases.modules.iter().filter_map(
        |(binding, target_modules)| {
            (!resolved_module_aliases.external_bindings.contains(binding)
                && (resolved_module_aliases.tainted_bindings.contains(binding)
                    || target_modules.len() != 1
                    || target_modules.iter().any(|target_module| {
                        resolved
                            .keys()
                            .any(|capability| capability.module_path.starts_with(target_module))
                            || resolved_module_aliases
                                .tainted_modules
                                .iter()
                                .any(|module| module.starts_with(target_module))
                    })))
            .then_some(binding.clone())
        },
    ));
    module_alias_bindings.extend(resolved_module_aliases.tainted_bindings.iter().cloned());
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
        let SourceAliasTarget::Unsupported {
            syntax_kind,
            identifiers,
        } = &alias.target
        else {
            continue;
        };
        let mentions_capability = CAPABILITY_TYPE_IDENTITIES
            .iter()
            .any(|identity| identifiers.contains(*identity))
            || resolved
                .keys()
                .any(|binding| identifiers.contains(&binding.name));
        if mentions_capability {
            panic!(
                "unsupported {syntax_kind} target for capability alias {} in {}; capability \
                 trait inventory fails closed without rendering attacker-authored syntax",
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
            &capability_visibility,
            &resolved_module_aliases,
            &facts.module_aliases,
            &facts.globs,
            &implementation.lexical_scope,
            resolved_module_aliases
                .tainted_modules
                .contains(&implementation.module_path),
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
        let entry = format!(
            "{crate_name}::{identity}\t{qualifier}impl{generic_parameters} \
             {polarity}{} for {}{where_clause}",
            compact_tokens(trait_path),
            compact_tokens(&implementation.implementation.self_ty),
        );
        inventory.trait_impls.insert(entry.clone());
        inventory.trait_impl_diagnostics.insert(
            entry,
            InventoryDiagnostic {
                syntax_kind: "explicit trait implementation",
                identity: format!("{crate_name}::{identity}"),
                source: implementation.source.clone(),
            },
        );
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
                || matches!(alias.target, SourceAliasTarget::Unsupported { .. })
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
                SourceAliasTarget::Unsupported { .. } => unreachable!(),
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

struct ResolvedModuleAliases {
    modules: BTreeMap<SourceBinding, BTreeSet<Vec<String>>>,
    visibility_scopes: BTreeMap<SourceBinding, BTreeSet<Vec<String>>>,
    external_bindings: BTreeSet<SourceBinding>,
    tainted_bindings: BTreeSet<SourceBinding>,
    tainted_modules: BTreeSet<Vec<String>>,
    binding_universe: BTreeSet<SourceBinding>,
    known_modules: BTreeSet<Vec<String>>,
    external_crates: BTreeSet<String>,
}

fn resolve_module_aliases_to_fixed_point(
    aliases: &[SourceModuleAlias],
    globs: &[SourceGlob],
    declared_modules: &BTreeSet<Vec<String>>,
    external_crates: &BTreeSet<String>,
    crate_name: &str,
) -> ResolvedModuleAliases {
    let aliases = aliases
        .iter()
        .filter(|alias| alias.lexical_scope.is_empty())
        .collect::<Vec<_>>();
    let globs = globs
        .iter()
        .filter(|glob| glob.lexical_scope.is_empty())
        .collect::<Vec<_>>();
    let declared_alias_bindings = aliases
        .iter()
        .map(|alias| alias.binding.clone())
        .collect::<BTreeSet<_>>();
    let alias_names = declared_alias_bindings
        .iter()
        .map(|binding| binding.name.clone())
        .collect::<BTreeSet<_>>();
    let mut binding_universe = declared_alias_bindings.clone();
    for glob in &globs {
        for name in &alias_names {
            binding_universe.insert(SourceBinding {
                module_path: glob.module_path.clone(),
                name: name.clone(),
            });
        }
    }
    let mut known_modules = declared_modules.clone();
    known_modules.insert(Vec::new());
    let mut resolved = BTreeMap::<SourceBinding, BTreeSet<Vec<String>>>::new();
    let mut visibility_scopes = BTreeMap::<SourceBinding, BTreeSet<Vec<String>>>::new();
    let visibility_universe = aliases
        .iter()
        .map(|alias| alias.visibility_scope.clone())
        .chain(globs.iter().map(|glob| glob.visibility_scope.clone()))
        .collect::<BTreeSet<_>>();
    let target_budget = binding_universe.len() * known_modules.len().max(1);
    let visibility_budget = binding_universe.len() * visibility_universe.len().max(1);
    let convergence_budget = target_budget + visibility_budget + 1;
    for iteration in 0..=convergence_budget {
        let mut next = resolved.clone();
        let mut next_visibility_scopes = visibility_scopes.clone();
        for alias in &aliases {
            let target = alias.declared_target.as_ref().map_or_else(
                || {
                    resolve_module_alias_target(
                        &alias.target,
                        &alias.binding.module_path,
                        crate_name,
                        &resolved,
                        &binding_universe,
                        &known_modules,
                        external_crates,
                        &BTreeSet::new(),
                    )
                },
                |target| ModuleAliasTarget {
                    modules: known_modules
                        .contains(target)
                        .then(|| target.clone())
                        .into_iter()
                        .collect(),
                    unresolved: !known_modules.contains(target),
                    tainted: false,
                    external: false,
                },
            );
            next.entry(alias.binding.clone())
                .or_default()
                .extend(target.modules);
            next_visibility_scopes
                .entry(alias.binding.clone())
                .or_default()
                .insert(alias.visibility_scope.clone());
        }
        for glob in &globs {
            let target = resolve_module_alias_target(
                &glob.target,
                &glob.module_path,
                crate_name,
                &resolved,
                &binding_universe,
                &known_modules,
                external_crates,
                &BTreeSet::new(),
            );
            for target_module in target.modules {
                for (source_binding, target_modules) in &resolved {
                    if source_binding.module_path != target_module {
                        continue;
                    }
                    let visible = visibility_scopes.get(source_binding).is_some_and(|scopes| {
                        scopes
                            .iter()
                            .any(|scope| glob.module_path.starts_with(scope))
                    });
                    if !visible {
                        continue;
                    }
                    let imported = SourceBinding {
                        module_path: glob.module_path.clone(),
                        name: source_binding.name.clone(),
                    };
                    if declared_alias_bindings.contains(&imported) {
                        continue;
                    }
                    next.entry(imported.clone())
                        .or_default()
                        .extend(target_modules.iter().cloned());
                    next_visibility_scopes
                        .entry(imported)
                        .or_default()
                        .insert(glob.visibility_scope.clone());
                }
            }
        }
        if next == resolved && next_visibility_scopes == visibility_scopes {
            break;
        }
        assert!(
            iteration < convergence_budget,
            "module alias resolution exceeded its finite binding and module target budget"
        );
        resolved = next;
        visibility_scopes = next_visibility_scopes;
    }

    let taint_budget = binding_universe.len() + known_modules.len() + 1;
    let mut external_bindings = BTreeSet::new();
    for alias in &aliases {
        if alias.declared_target.is_some() {
            continue;
        }
        let target = resolve_module_alias_target(
            &alias.target,
            &alias.binding.module_path,
            crate_name,
            &resolved,
            &binding_universe,
            &known_modules,
            external_crates,
            &BTreeSet::new(),
        );
        if target.external {
            external_bindings.insert(alias.binding.clone());
        }
    }
    let mut tainted_bindings = BTreeSet::new();
    let mut tainted_modules = BTreeSet::new();
    for iteration in 0..=taint_budget {
        let mut next_bindings = tainted_bindings.clone();
        let mut next_modules = tainted_modules.clone();
        for alias in &aliases {
            let target = alias.declared_target.as_ref().map_or_else(
                || {
                    resolve_module_alias_target(
                        &alias.target,
                        &alias.binding.module_path,
                        crate_name,
                        &resolved,
                        &binding_universe,
                        &known_modules,
                        external_crates,
                        &tainted_bindings,
                    )
                },
                |target| ModuleAliasTarget {
                    modules: known_modules
                        .contains(target)
                        .then(|| target.clone())
                        .into_iter()
                        .collect(),
                    unresolved: !known_modules.contains(target),
                    tainted: false,
                    external: false,
                },
            );
            if !target.external
                && (target.unresolved || target.tainted || target.modules.len() != 1)
            {
                next_bindings.insert(alias.binding.clone());
            }
        }
        for glob in &globs {
            let target = resolve_module_alias_target(
                &glob.target,
                &glob.module_path,
                crate_name,
                &resolved,
                &binding_universe,
                &known_modules,
                external_crates,
                &tainted_bindings,
            );
            let target_tainted = target
                .modules
                .iter()
                .any(|module| tainted_modules.contains(module));
            if !target.external
                && (target.unresolved
                    || target.tainted
                    || target.modules.len() != 1
                    || target_tainted)
            {
                next_modules.insert(glob.module_path.clone());
            }
            for target_module in &target.modules {
                for source_binding in &binding_universe {
                    if &source_binding.module_path != target_module {
                        continue;
                    }
                    let visible = visibility_scopes.get(source_binding).is_some_and(|scopes| {
                        scopes
                            .iter()
                            .any(|scope| glob.module_path.starts_with(scope))
                    });
                    if !visible {
                        continue;
                    }
                    let imported = SourceBinding {
                        module_path: glob.module_path.clone(),
                        name: source_binding.name.clone(),
                    };
                    if declared_alias_bindings.contains(&imported) {
                        continue;
                    }
                    if tainted_bindings.contains(source_binding)
                        || tainted_modules.contains(target_module)
                    {
                        next_bindings.insert(imported);
                    }
                }
            }
        }
        if next_bindings == tainted_bindings && next_modules == tainted_modules {
            break;
        }
        assert!(
            iteration < taint_budget,
            "module alias taint propagation exceeded its finite binding and module budget"
        );
        tainted_bindings = next_bindings;
        tainted_modules = next_modules;
    }

    ResolvedModuleAliases {
        modules: resolved,
        visibility_scopes,
        external_bindings,
        tainted_bindings,
        tainted_modules,
        binding_universe,
        known_modules,
        external_crates: external_crates.clone(),
    }
}

struct ModuleAliasTarget {
    modules: BTreeSet<Vec<String>>,
    unresolved: bool,
    tainted: bool,
    external: bool,
}

#[allow(clippy::too_many_arguments)]
fn resolve_module_alias_target(
    path: &SourcePath,
    module_path: &[String],
    crate_name: &str,
    resolved_aliases: &BTreeMap<SourceBinding, BTreeSet<Vec<String>>>,
    alias_bindings: &BTreeSet<SourceBinding>,
    known_modules: &BTreeSet<Vec<String>>,
    external_crates: &BTreeSet<String>,
    tainted_bindings: &BTreeSet<SourceBinding>,
) -> ModuleAliasTarget {
    for end in (1..=path.segments.len()).rev() {
        let prefix = SourcePath {
            leading_colon: path.leading_colon,
            segments: path.segments[..end].to_vec(),
        };
        let candidates = binding_candidates(&prefix, module_path, crate_name)
            .into_iter()
            .filter(|binding| alias_bindings.contains(binding))
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            continue;
        }
        let suffix = &path.segments[end..];
        let mut modules = BTreeSet::new();
        let mut unresolved = candidates.len() != 1;
        let mut tainted = false;
        for candidate in candidates {
            tainted |= tainted_bindings.contains(&candidate);
            let Some(targets) = resolved_aliases.get(&candidate) else {
                unresolved = true;
                continue;
            };
            for target in targets {
                let mut expanded = target.clone();
                expanded.extend_from_slice(suffix);
                if known_modules.contains(&expanded) {
                    modules.insert(expanded);
                } else {
                    unresolved = true;
                }
            }
        }
        return ModuleAliasTarget {
            modules,
            unresolved,
            tainted,
            external: false,
        };
    }
    let external = path.segments.first().is_some_and(|root| {
        !matches!(root.as_str(), "crate" | "self" | "super") && external_crates.contains(root)
    });
    if external {
        return ModuleAliasTarget {
            modules: BTreeSet::new(),
            unresolved: false,
            tainted: false,
            external: true,
        };
    }
    let direct = resolve_module_path(path, module_path, crate_name);
    let unresolved = direct
        .as_ref()
        .is_none_or(|target| !known_modules.contains(target));
    ModuleAliasTarget {
        modules: direct
            .into_iter()
            .filter(|target| known_modules.contains(target))
            .collect(),
        unresolved,
        tainted: false,
        external: false,
    }
}

#[derive(Clone, Copy)]
struct SourceAliasBindings<'a> {
    type_aliases: &'a BTreeSet<SourceBinding>,
    module_aliases: &'a BTreeSet<SourceBinding>,
}

#[allow(clippy::too_many_arguments)]
fn resolve_impl_self_type(
    ty: &syn::Type,
    module_path: &[String],
    crate_name: &str,
    resolved: &BTreeMap<SourceBinding, String>,
    aliases: SourceAliasBindings<'_>,
    capability_visibility: &BTreeMap<SourceBinding, BTreeSet<Vec<String>>>,
    module_aliases: &ResolvedModuleAliases,
    source_module_aliases: &[SourceModuleAlias],
    globs: &[SourceGlob],
    lexical_scope: &[usize],
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
            capability_visibility,
            module_aliases,
            source_module_aliases,
            globs,
            lexical_scope,
            unresolved_glob,
            source,
        ),
        syn::Type::Paren(paren) => resolve_impl_self_type(
            &paren.elem,
            module_path,
            crate_name,
            resolved,
            aliases,
            capability_visibility,
            module_aliases,
            source_module_aliases,
            globs,
            lexical_scope,
            unresolved_glob,
            source,
        ),
        syn::Type::Path(path) if path.qself.is_none() => {
            let path = source_path(&path.path);
            if let Some(identity) = resolve_lexical_glob_self_type(
                &path,
                module_path,
                crate_name,
                resolved,
                capability_visibility,
                module_aliases,
                source_module_aliases,
                globs,
                lexical_scope,
                source,
            ) {
                return Some(identity);
            }
            if let Some(alias) =
                module_alias_in_path(&path, module_path, crate_name, aliases.module_aliases)
            {
                panic!(
                    "cannot resolve module alias {} in impl self type {} in {source}; \
                     capability trait inventory fails closed instead of modelling \
                     module-prefix imports",
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
            let identifiers = source_type_identifiers(other);
            let mentions_capability = CAPABILITY_TYPE_IDENTITIES
                .iter()
                .any(|identity| identifiers.contains(*identity))
                || resolved
                    .keys()
                    .any(|binding| identifiers.contains(&binding.name));
            if mentions_capability {
                panic!(
                    "cannot classify possible capability impl self type with unsupported {} \
                     syntax in {source}; capability trait inventory fails closed without \
                     rendering attacker-authored syntax",
                    source_type_syntax_kind(other)
                );
            }
            None
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn resolve_lexical_glob_self_type(
    path: &SourcePath,
    module_path: &[String],
    crate_name: &str,
    resolved: &BTreeMap<SourceBinding, String>,
    capability_visibility: &BTreeMap<SourceBinding, BTreeSet<Vec<String>>>,
    module_aliases: &ResolvedModuleAliases,
    source_module_aliases: &[SourceModuleAlias],
    globs: &[SourceGlob],
    lexical_scope: &[usize],
    source: &str,
) -> Option<String> {
    let visible_globs = globs
        .iter()
        .filter(|glob| {
            !glob.lexical_scope.is_empty()
                && glob.module_path == module_path
                && lexical_scope.starts_with(&glob.lexical_scope)
        })
        .collect::<Vec<_>>();
    if visible_globs.is_empty() {
        return None;
    }

    let mut identities = BTreeSet::new();
    let mut module_targets = BTreeSet::new();
    let imported_name = path.segments.first();
    for glob in visible_globs {
        let target = resolve_lexical_module_alias_target(
            &glob.target,
            &glob.module_path,
            crate_name,
            lexical_scope,
            source_module_aliases,
            &module_aliases.known_modules,
        )
        .unwrap_or_else(|| {
            resolve_module_alias_target(
                &glob.target,
                &glob.module_path,
                crate_name,
                &module_aliases.modules,
                &module_aliases.binding_universe,
                &module_aliases.known_modules,
                &module_aliases.external_crates,
                &module_aliases.tainted_bindings,
            )
        });
        if target.unresolved
            || target.tainted
            || target.modules.len() != 1
            || target
                .modules
                .iter()
                .any(|module| module_aliases.tainted_modules.contains(module))
        {
            panic!(
                "cannot classify impl self type {} in {source} because a block-local glob \
                 target is unresolved or ambiguous; capability trait inventory fails closed",
                display_source_path(path)
            );
        }
        for target_module in &target.modules {
            if path.segments.len() == 1 {
                for (binding, identity) in resolved {
                    if &binding.module_path != target_module || Some(&binding.name) != imported_name
                    {
                        continue;
                    }
                    let visible = capability_visibility.get(binding).is_some_and(|scopes| {
                        scopes.iter().any(|scope| module_path.starts_with(scope))
                    });
                    if visible {
                        identities.insert(identity.clone());
                    }
                }
                continue;
            }
            let Some(imported_name) = imported_name else {
                continue;
            };
            let source_binding = SourceBinding {
                module_path: target_module.clone(),
                name: imported_name.clone(),
            };
            let lexical_children = source_module_aliases
                .iter()
                .filter(|alias| {
                    !alias.lexical_scope.is_empty()
                        && lexical_scope.starts_with(&alias.lexical_scope)
                        && alias.binding == source_binding
                })
                .collect::<Vec<_>>();
            assert!(
                lexical_children.len() <= 1,
                "block-local glob module alias {} is ambiguous in impl self type {} in \
                 {source}; capability trait inventory fails closed",
                imported_name,
                display_source_path(path)
            );
            let lexical_child = lexical_children.first().copied();
            let target_modules = lexical_child
                .and_then(|alias| alias.declared_target.clone())
                .map(|child| [child].into_iter().collect())
                .or_else(|| module_aliases.modules.get(&source_binding).cloned());
            let Some(target_modules) = target_modules else {
                if lexical_child.is_some() {
                    panic!(
                        "cannot resolve block-local glob module alias {} in impl self type {} in \
                         {source}; capability trait inventory fails closed",
                        imported_name,
                        display_source_path(path)
                    );
                }
                continue;
            };
            let visible = lexical_child.is_some()
                || module_aliases
                    .visibility_scopes
                    .get(&source_binding)
                    .is_some_and(|scopes| {
                        scopes.iter().any(|scope| module_path.starts_with(scope))
                    });
            if !visible {
                continue;
            }
            if module_aliases.tainted_bindings.contains(&source_binding)
                || target_modules.len() != 1
                || target_modules.iter().any(|target_module| {
                    resolved
                        .keys()
                        .any(|capability| capability.module_path.starts_with(target_module))
                        || module_aliases
                            .tainted_modules
                            .iter()
                            .any(|module| module.starts_with(target_module))
                })
            {
                panic!(
                    "cannot resolve block-local glob module alias {} in impl self type {} in \
                     {source}; capability trait inventory fails closed",
                    imported_name,
                    display_source_path(path)
                );
            }
            module_targets.extend(target_modules.iter().cloned());
        }
    }
    assert!(
        identities.len() <= 1,
        "block-local glob imports make impl self type {} ambiguous in {source}; capability \
         trait inventory fails closed",
        display_source_path(path)
    );
    if let Some(identity) = identities.into_iter().next() {
        return Some(identity);
    }
    assert!(
        module_targets.len() <= 1,
        "block-local glob imports make module alias {} ambiguous in impl self type {} in \
         {source}; capability trait inventory fails closed",
        imported_name.map_or("<unknown>", String::as_str),
        display_source_path(path)
    );
    None
}

fn resolve_lexical_module_alias_target(
    path: &SourcePath,
    module_path: &[String],
    crate_name: &str,
    lexical_scope: &[usize],
    aliases: &[SourceModuleAlias],
    known_modules: &BTreeSet<Vec<String>>,
) -> Option<ModuleAliasTarget> {
    for end in (1..=path.segments.len()).rev() {
        let prefix = SourcePath {
            leading_colon: path.leading_colon,
            segments: path.segments[..end].to_vec(),
        };
        let candidates = binding_candidates(&prefix, module_path, crate_name)
            .into_iter()
            .flat_map(|binding| {
                aliases.iter().filter(move |alias| {
                    !alias.lexical_scope.is_empty()
                        && lexical_scope.starts_with(&alias.lexical_scope)
                        && alias.binding == binding
                })
            })
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            continue;
        }
        if candidates.len() != 1 {
            return Some(ModuleAliasTarget {
                modules: BTreeSet::new(),
                unresolved: true,
                tainted: true,
                external: false,
            });
        }
        let suffix = &path.segments[end..];
        let Some(target) = candidates[0].declared_target.as_ref() else {
            return Some(ModuleAliasTarget {
                modules: BTreeSet::new(),
                unresolved: true,
                tainted: true,
                external: false,
            });
        };
        let mut expanded = target.clone();
        expanded.extend_from_slice(suffix);
        let known = known_modules.contains(&expanded);
        return Some(ModuleAliasTarget {
            modules: known.then_some(expanded).into_iter().collect(),
            unresolved: !known,
            tainted: false,
            external: false,
        });
    }
    None
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
    inventory: &mut SourceCapabilityInventory,
) {
    let identity = ident_name(&declaration.identity);
    for attribute in &declaration.attributes {
        if attribute.path().is_ident("derive") {
            record_derive_meta(
                crate_name,
                &identity,
                &attribute.meta,
                &declaration.source,
                inventory,
            );
        } else if attribute.path().is_ident("cfg_attr") {
            record_cfg_attr_derives(
                crate_name,
                &identity,
                &attribute.meta,
                &declaration.source,
                inventory,
            );
        }
    }
}

fn record_derive_meta(
    crate_name: &str,
    identity: &str,
    meta: &Meta,
    source: &str,
    inventory: &mut SourceCapabilityInventory,
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
        let entry = format!(
            "{crate_name}::{identity}\tderive {} for {identity}",
            compact_tokens(&trait_path)
        );
        inventory.trait_impls.insert(entry.clone());
        inventory.trait_impl_diagnostics.insert(
            entry,
            InventoryDiagnostic {
                syntax_kind: "derive implementation",
                identity: format!("{crate_name}::{identity}"),
                source: source.to_owned(),
            },
        );
    }
}

fn record_cfg_attr_derives(
    crate_name: &str,
    identity: &str,
    meta: &Meta,
    source: &str,
    inventory: &mut SourceCapabilityInventory,
) {
    let nested = parse_cfg_attr(meta, crate_name, identity, source);
    for attribute in nested {
        if attribute.path().is_ident("derive") {
            record_derive_meta(crate_name, identity, &attribute, source, inventory);
        } else if attribute.path().is_ident("cfg_attr") {
            record_cfg_attr_derives(crate_name, identity, &attribute, source, inventory);
        } else if !safe_inert_cfg_attr(&attribute) {
            panic!(
                "unrecognised conditional attribute on capability declaration \
                 {crate_name}::{identity} in {source}; capability trait inventory fails closed"
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
                "unrecognised conditional attribute on capability impl \
                 {crate_name}::{identity} in {source}; capability trait inventory fails closed"
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

#[derive(Default)]
struct SourceTypeIdentifierCollector {
    identifiers: BTreeSet<String>,
}

impl<'ast> Visit<'ast> for SourceTypeIdentifierCollector {
    fn visit_ident(&mut self, ident: &'ast syn::Ident) {
        self.identifiers.insert(ident_name(ident));
    }
}

fn source_type_identifiers(ty: &syn::Type) -> BTreeSet<String> {
    let mut collector = SourceTypeIdentifierCollector::default();
    collector.visit_type(ty);
    collector.identifiers
}

fn source_type_syntax_kind(ty: &syn::Type) -> &'static str {
    match ty {
        syn::Type::Array(_) => "array type",
        syn::Type::BareFn(_) => "bare-function type",
        syn::Type::Group(_) => "grouped type",
        syn::Type::ImplTrait(_) => "impl-trait type",
        syn::Type::Infer(_) => "inferred type",
        syn::Type::Macro(_) => "macro type",
        syn::Type::Never(_) => "never type",
        syn::Type::Paren(_) => "parenthesized type",
        syn::Type::Path(_) => "qualified-path type",
        syn::Type::Ptr(_) => "raw-pointer type",
        syn::Type::Reference(_) => "reference type",
        syn::Type::Slice(_) => "slice type",
        syn::Type::TraitObject(_) => "trait-object type",
        syn::Type::Tuple(_) => "tuple type",
        syn::Type::Verbatim(_) => "verbatim type",
        _ => "unknown type",
    }
}

struct HiddenPublicScanner<'a> {
    crate_name: &'a str,
    source_root: PathBuf,
    entries: BTreeSet<String>,
    diagnostics: BTreeMap<String, InventoryDiagnostic>,
    visited: BTreeMap<PathBuf, Vec<String>>,
}

impl HiddenPublicScanner<'_> {
    fn scan_file(
        &mut self,
        source: &Path,
        module_path: &[String],
        kind: SourceFileKind,
        inherited_hidden: bool,
    ) {
        let logical_source =
            source_location(self.crate_name, &self.source_root, source, module_path);
        let canonical_source = fs::canonicalize(source)
            .unwrap_or_else(|error| panic!("canonicalize Rust source {logical_source}: {error}"));
        if let Some(previous) = self.visited.get(&canonical_source) {
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
        self.visited.insert(canonical_source, module_path.to_vec());
        let text = fs::read_to_string(source)
            .unwrap_or_else(|error| panic!("read Rust source {logical_source}: {error}"));
        let file = syn::parse_file(&text)
            .unwrap_or_else(|error| panic!("parse Rust source {logical_source}: {error}"));
        let module_dir = source_module_dir(source, kind);
        let path_base = source
            .parent()
            .expect("lexical Rust source has a parent directory");
        self.scan_items(
            &file.items,
            module_path,
            &module_dir,
            path_base,
            inherited_hidden || doc_hidden(&file.attrs),
            &logical_source,
        );
    }

    fn scan_items(
        &mut self,
        items: &[Item],
        module_path: &[String],
        module_dir: &Path,
        path_base: &Path,
        inherited_hidden: bool,
        logical_source: &str,
    ) {
        for item in items {
            match item {
                Item::Fn(function)
                    if matches!(function.vis, Visibility::Public(_))
                        && (inherited_hidden || doc_hidden(&function.attrs)) =>
                {
                    self.record(
                        module_path,
                        ident_name(&function.sig.ident),
                        &function.sig,
                        "hidden public function",
                        logical_source,
                    );
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
                            let name = format!("{owner}::method:{}", ident_name(&method.sig.ident));
                            self.record(
                                module_path,
                                name,
                                &method.sig,
                                "hidden public method",
                                logical_source,
                            );
                        }
                    }
                }
                Item::Trait(trait_item) => {
                    let hidden = inherited_hidden || doc_hidden(&trait_item.attrs);
                    for member in &trait_item.items {
                        if let syn::TraitItem::Fn(method) = member
                            && (hidden || doc_hidden(&method.attrs))
                        {
                            let name = format!(
                                "{}::tymethod:{}",
                                ident_name(&trait_item.ident),
                                ident_name(&method.sig.ident)
                            );
                            self.record(
                                module_path,
                                name,
                                &method.sig,
                                "hidden trait method",
                                logical_source,
                            );
                        }
                    }
                }
                Item::Mod(module) => {
                    let hidden = inherited_hidden || doc_hidden(&module.attrs);
                    let module_name = ident_name(&module.ident);
                    let mut child_path = module_path.to_vec();
                    child_path.push(module_name.clone());
                    if hidden && matches!(module.vis, Visibility::Public(_)) {
                        let signature = format!("pub mod {}", module.ident);
                        self.record(
                            module_path,
                            module_name.clone(),
                            &signature,
                            "hidden public module",
                            logical_source,
                        );
                    }
                    let child_dir = module_dir.join(module_name);
                    if let Some((_, items)) = &module.content {
                        let inline_dir = module_path_override(module, path_base, logical_source)
                            .unwrap_or_else(|| child_dir.clone());
                        self.scan_items(
                            items,
                            &child_path,
                            &inline_dir,
                            &inline_dir,
                            hidden,
                            logical_source,
                        );
                    } else if let Some(source) =
                        module_source(module, module_dir, path_base, &child_dir, logical_source)
                    {
                        self.scan_file(&source.path, &child_path, source.kind, hidden);
                    }
                }
                _ => {}
            }
        }
    }

    fn record(
        &mut self,
        module_path: &[String],
        name: String,
        signature: &impl ToTokens,
        syntax_kind: &'static str,
        source: &str,
    ) {
        let mut symbol = self.crate_name.to_owned();
        for module in module_path {
            symbol.push_str("::");
            symbol.push_str(module);
        }
        symbol.push_str("::");
        symbol.push_str(&name);
        let entry = format!("{symbol}\t{}", signature.to_token_stream());
        self.entries.insert(entry.clone());
        self.diagnostics.insert(
            entry,
            InventoryDiagnostic {
                syntax_kind,
                identity: symbol,
                source: source.to_owned(),
            },
        );
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
        return ident_name(&segment.ident);
    }
    "<unsupported-self-type>".to_owned()
}

fn ident_name(ident: &syn::Ident) -> String {
    ident.unraw().to_string()
}

fn source_module_dir(source: &Path, kind: SourceFileKind) -> PathBuf {
    let parent = source
        .parent()
        .expect("lexical Rust source has a parent directory");
    if matches!(
        kind,
        SourceFileKind::CrateRoot | SourceFileKind::PathLoadedModule
    ) || source.file_name().is_some_and(|name| name == "mod.rs")
    {
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
    logical_source: &str,
) -> Option<ResolvedModuleSource> {
    if let Some(path) = module_path_override(module, path_base, logical_source) {
        return path.is_file().then_some(ResolvedModuleSource {
            path,
            kind: SourceFileKind::PathLoadedModule,
        });
    }
    let flat = module_dir.join(format!("{}.rs", ident_name(&module.ident)));
    if flat.is_file() {
        return Some(ResolvedModuleSource {
            path: flat,
            kind: SourceFileKind::OrdinaryModule,
        });
    }
    let nested = child_dir.join("mod.rs");
    nested.is_file().then_some(ResolvedModuleSource {
        path: nested,
        kind: SourceFileKind::OrdinaryModule,
    })
}

fn module_path_override(
    module: &syn::ItemMod,
    path_base: &Path,
    logical_source: &str,
) -> Option<PathBuf> {
    let mut path_attribute = None;
    for attribute in &module.attrs {
        if attribute.path().is_ident("path") {
            assert!(
                path_attribute.replace(attribute).is_none(),
                "multiple path attributes on Rust module {} in {logical_source}; source \
                 inventories fail closed",
                module.ident,
            );
        } else if attribute.path().is_ident("cfg_attr") {
            validate_module_cfg_attr(&attribute.meta, &module.ident, logical_source);
        } else if !safe_inert_module_attribute(&attribute.meta) {
            panic!(
                "unsupported direct module attribute on Rust module {} in {logical_source}; \
                 source inventories fail closed. Remove the attribute or add its exact \
                 proven-inert path and shape to the module attribute allowlist with a regression \
                 fixture",
                module.ident
            );
        }
    }
    let attribute = path_attribute?;
    let path = match &attribute.meta {
        syn::Meta::NameValue(value) => {
            let syn::Expr::Lit(value) = &value.value else {
                panic!(
                    "path attribute on Rust module {} in {logical_source} is not a string literal; \
                     source inventories fail closed",
                    module.ident
                );
            };
            let syn::Lit::Str(path) = &value.lit else {
                panic!(
                    "path attribute on Rust module {} in {logical_source} is not a string literal; \
                     source inventories fail closed",
                    module.ident
                );
            };
            path
        }
        _ => panic!(
            "path attribute on Rust module {} in {logical_source} is not name-value syntax; \
             source inventories fail closed",
            module.ident
        ),
    };
    Some(path_base.join(path.value()))
}

fn validate_module_cfg_attr(meta: &Meta, module: &syn::Ident, logical_source: &str) {
    let Meta::List(list) = meta else {
        panic!(
            "cfg_attr on Rust module {module} in {logical_source} is not list syntax; source \
             inventories cannot determine whether it changes the compiler-selected module file"
        );
    };
    let values = list
        .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        .unwrap_or_else(|error| {
            panic!(
                "cannot parse cfg_attr on Rust module {module} in {logical_source}: {error}; \
                 source inventories cannot determine whether it changes the compiler-selected \
                 module file"
            )
        })
        .into_iter()
        .collect::<Vec<_>>();
    assert!(
        values.len() >= 2,
        "cfg_attr on Rust module {module} in {logical_source} must contain a condition and at \
         least one attribute; source inventories cannot determine the compiler-selected module \
         file"
    );
    for attribute in values.into_iter().skip(1) {
        if attribute.path().is_ident("cfg_attr") {
            validate_module_cfg_attr(&attribute, module, logical_source);
        } else if attribute.path().is_ident("path") {
            panic!(
                "conditional path attribute on Rust module {module} in {logical_source}; source \
                 inventories fail closed because the compiler-selected module file is ambiguous"
            );
        } else if !safe_inert_module_cfg_attr(&attribute) {
            panic!(
                "cfg_attr on Rust module {module} in {logical_source} contains an unrecognised \
                 conditional module attribute; source inventories fail closed because only the \
                 explicit inert attribute allowlist is permitted. Remove the attribute or add its \
                 exact proven-inert path and shape with a regression fixture"
            );
        }
    }
}

fn safe_inert_module_cfg_attr(meta: &Meta) -> bool {
    safe_inert_module_attribute(meta)
}

fn safe_inert_module_attribute(meta: &Meta) -> bool {
    match meta {
        Meta::List(list) if list.path.is_ident("cfg") => true,
        Meta::List(list)
            if list.path.is_ident("allow")
                || list.path.is_ident("warn")
                || list.path.is_ident("deny")
                || list.path.is_ident("forbid")
                || list.path.is_ident("expect") =>
        {
            true
        }
        Meta::List(list) if list.path.is_ident("doc") || list.path.is_ident("deprecated") => true,
        Meta::NameValue(value)
            if value.path.is_ident("doc") || value.path.is_ident("deprecated") =>
        {
            true
        }
        Meta::Path(path) if path.is_ident("deprecated") => true,
        Meta::Path(path) if exact_meta_path(path, &["rustfmt", "skip"]) => true,
        _ => false,
    }
}

fn exact_meta_path(path: &syn::Path, expected: &[&str]) -> bool {
    path.leading_colon.is_none()
        && path.segments.len() == expected.len()
        && path
            .segments
            .iter()
            .zip(expected)
            .all(|(segment, expected)| {
                ident_name(&segment.ident) == *expected
                    && matches!(segment.arguments, syn::PathArguments::None)
            })
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
    let all = fs::read_to_string(&all_path).map_err(|_| {
        format!(
            "rustdoc output for {crate_name} is incomplete: cannot read the crate-relative \
             all-items index. This is a doc-build problem."
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
                "rustdoc output for {crate_name} has a malformed all-items link at entry {}. \
                 This is a doc-build problem.",
                index + 1
            )
        })?;
        let (name, _) = rest.split_once("</a>").ok_or_else(|| {
            format!(
                "rustdoc output for {crate_name} has malformed all-items label for \
                 entry {}. This is a doc-build problem.",
                index + 1
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
                 at entry {}. This is a doc-build problem.",
                index + 1
            ));
        }
        let symbol = format!("{crate_name}::{name}");
        let path = root.join(href);
        let html = fs::read_to_string(&path).map_err(|_| {
            format!(
                "rustdoc output is incomplete: advertised item {symbol} has no \
                 readable crate-relative page. This is a doc-build problem."
            )
        })?;
        if item_declaration(&html).is_none() {
            return Err(format!(
                "rustdoc output is incomplete: advertised item {symbol} could not \
                 be parsed from its crate-relative page. This is a doc-build problem."
            ));
        }
        advertised.push(AdvertisedItem {
            href: PathBuf::from(href),
            name: name.to_owned(),
        });
    }
    if advertised.is_empty() {
        return Err(format!(
            "rustdoc output for {crate_name} advertises no items in its crate-relative \
             all-items index. This is a doc-build problem."
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
    let mut identity =
        fs::canonicalize(path).unwrap_or_else(|_| panic!("canonicalize rustdoc type identity"));
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
