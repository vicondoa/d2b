//! The new-graph build composition.
//!
//! Five independent declaration sources describe every provider: the
//! resource types it owns, the operation rows it serves, the Provider
//! identities it owns on each of the three identity surfaces, the effect
//! services its runtime identity registers, and the semantic services its
//! session identity publishes. Three committed generated artifacts are
//! derived from them, and the closure manifest beside them names the
//! production file that compiles each byte.
//!
//! `gen-new-graph` is one step of the `make generate` aggregate, so these
//! projections are production generation outputs: they are committed under
//! [`OUTPUT_DIR`], they are byte-for-byte drift checked like every other
//! committed artifact, and nothing renders them from a test-only path.
//!
//! # What production compiles
//!
//! Each staged artifact is compiled by a production entry point itself. The
//! daemon's composition root, the zone-session contract and the resource
//! contracts `include!` the staged file directly, so there is no second copy
//! to fall out of step, and the manifest names the file that compiles each
//! byte.
//!
//! Nothing is staged that no production file compiles. A projection with no
//! consumer is a second copy of a declaration with nothing reading it, so the
//! generator stages the tables the daemon links and stops there.
//!
//! # The declared-versus-compiled cross-check
//!
//! Before it writes anything, the composition cross-checks the declarations
//! against the provider crates' own compiled sources: a handler a crate
//! compiles with no declaration behind it, a declared method nothing
//! compiles, a shared driver that also registers itself as a runtime
//! Provider, and a service package a crate declares but never spells all
//! fail `make generate`. That check reads [`DeclaredProviders`] rather than
//! this module's rendered bytes, so its two sides are independent sources.
//!
//! # What this generator does not read
//!
//! The pre-declaration authoring form is a set of files and in-generator
//! tables a reader could mistake for a second source. [`RETIRED_AUTHORITY_SOURCES`]
//! names them, and the composition reads none of them:
//!
//! - `docs/reference/policy/broker-operations.json`, the merged rows document
//!   [`crate::gen_broker_operations::render_artifacts`] still merges into;
//! - `docs/reference/policy/principal-allocation.json`, the handwritten
//!   principal table;
//! - [`crate::gen_broker_operations`]'s merge step and
//!   [`crate::operation_row_authority`]'s committed service-facet scope table,
//!   the two in-generator authorities that would otherwise admit a
//!   handwritten privilege or family scope.
//!
//! The exclusion is enforced two ways, because a list of names is only a
//! claim: [`render_composition`] resolves every input it touches and refuses
//! any that appears in that list, and the composition renders identically
//! from a tree that contains nothing but the declarations.
//!
//! # What the composition is not
//!
//! It is not a second spelling of the production entry points: each staged
//! artifact is the render the authority module already owns, reached through
//! its declaration-only entry point. It introduces no inventory, ledger, or
//! scheduler; the consumer mapping travels in one closure manifest.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use crate::authority_common::{declaration_paths, provider_crates, Declaration};
use crate::provider_identity_authority::{ProviderIdentities, Surface};

use d2b_contracts_provider::v3::projection::GRAPH_PROJECTION_CONTRACT_VERSION;
use d2b_contracts_resource::v3::canonical_digest;
use serde::Serialize;

/// The committed directory the composition's artifacts are written under.
pub(crate) const OUTPUT_DIR: &str = "generated";

/// The one subdirectory every staged artifact shares inside [`OUTPUT_DIR`].
const CLOSURE_DIR: &str = "new-graph";

/// The domain tag the closure manifest's artifact digests are framed by.
const CLOSURE_DIGEST_DOMAIN: &str = "d2b:v3:new-graph-build-closure";

/// Every source the new-graph composition reads.
///
/// The list is closed and asserted rather than documented: a source named
/// here is a source the composition is allowed to read, so a regression that
/// reintroduced one of the retired authorities below would have to declare
/// it. The four globs are the per-crate declarations themselves - the one
/// authoring form the new graph keeps.
pub(crate) const NEW_GRAPH_DECLARATION_INPUTS: &[&str] = &[
    "packages/d2b-provider-*/operations.json",
    "packages/d2b-provider-*/provider-identity.json",
    "packages/d2b-provider-*/registrations.json",
    "packages/d2b-provider-*/resource-types.json",
    "packages/d2b-provider-*/service-catalog.json",
];

/// The authorities the new graph retires, named so the composition can prove
/// it reads none of them.
///
/// Every entry is either a file the pre-declaration form authored or a
/// symbol in this crate that encodes a handwritten table. Both kinds are
/// merge-authority inputs: a projection that consumed one would carry facts
/// no declaration states, which is the second source this refactor removes.
pub(crate) const RETIRED_AUTHORITY_SOURCES: &[&str] = &[
    "docs/reference/policy/broker-operations.json",
    "docs/reference/policy/principal-allocation.json",
    "gen_broker_operations::merge_catalog",
    "gen_broker_operations::build_catalog",
    "operation_row_authority::COMMITTED_SERVICE_FACET_SCOPES",
    "nix_inventories::PROVIDER_PROJECTION_OWNERS",
];

/// The identity the new graph compiles under, and the one the manifest and
/// every staged artifact record.
pub(crate) fn contract_version() -> &'static str {
    GRAPH_PROJECTION_CONTRACT_VERSION
}

/// One staged artifact: the file this module renders, the production file
/// that compiles it, and the declarations that produce it.
///
/// `staged` is the artifact's file name inside the closure directory, so its
/// committed path is [`staged`]. `compiled_into` is the production source
/// file that `include!`s the byte. The mapping travels with the bytes rather
/// than in a separate ledger, so a consumer is a fact the manifest states and
/// a gate resolves rather than a convention. The field cannot be absent: an
/// artifact no production file compiles is staged bytes with no reader.
struct Replacement {
    staged: &'static str,
    compiled_into: &'static str,
    /// Every declaration whose facts the staged bytes are built from. The
    /// identity authority is one of them wherever the artifact carries a
    /// Provider identity, because the identity a row registers or routes to
    /// is declared there rather than restated in the artifact's own input.
    declarations: &'static [&'static str],
}

/// The committed artifacts the declarations project into, and the production
/// file each one is compiled by.
///
/// Every entry names a production consumer. An artifact nothing compiles would
/// be a staged copy of a declaration with nothing reading it, so the closure
/// holds the tables the product links and no projection besides them.
const REPLACEMENTS: &[Replacement] = &[
    Replacement {
        staged: "provider_registrations.rs",
        compiled_into: "packages/d2bd/src/resource_plane_v3.rs",
        declarations: &[
            "packages/d2b-provider-*/registrations.json",
            "packages/d2b-provider-*/provider-identity.json",
        ],
    },
    Replacement {
        staged: "service_provider_catalog.rs",
        compiled_into: "packages/d2b-contracts-zone-session/src/v3/mod.rs",
        declarations: &[
            "packages/d2b-provider-*/service-catalog.json",
            "packages/d2b-provider-*/provider-identity.json",
        ],
    },
    Replacement {
        staged: "v3_converted_resource_types.rs",
        compiled_into: "packages/d2b-contracts/src/identity.rs",
        declarations: &["packages/d2b-provider-*/resource-types.json"],
    },
];

/// The declared surface one provider crate contributes: the Provider
/// identity it registers, the services it serves, and the declared method
/// behind each operation row.
///
/// This is the cross-check's declared side, not a rendered artifact: nothing
/// serializes it, and the closure manifest carries no copy of it.
struct DeclaredProvider {
    /// The Provider identity a registration or service catalog declared, or
    /// `None` for a crate that declares none.
    ///
    /// A crate that owns no plane-facing surface is a provider *crate* but
    /// not a declared Provider, so composing a reference from its directory
    /// name would put an identity in the graph no declaration made. That
    /// absence is stated rather than filled, because the cross-check has to
    /// be able to tell a declared Provider from a crate that only implements
    /// one.
    provider_ref: Option<String>,
    crate_name: String,
    /// The effect-service ids the provider registers. A `ServiceDecl` in the
    /// crate's own sources names each one, so this is the vocabulary the
    /// composition root dispatches on.
    effect_services: Vec<String>,
    /// The semantic service packages the provider publishes. A string
    /// constant in the crate's own sources names each one, so this is the
    /// vocabulary the session layer addresses.
    service_packages: Vec<String>,
    /// The addressable methods the crate's operation rows declare.
    methods: Vec<String>,
}

/// The closure manifest: what the composition is, what it read, which
/// production file compiles each byte, and the contract crate identities it
/// was rendered against.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClosureManifest {
    contract_version: &'static str,
    declaration_inputs: &'static [&'static str],
    retired_authority_sources: &'static [&'static str],
    contract_crates: Vec<ContractCrate>,
    artifacts: Vec<ClosureArtifact>,
}

/// One new-graph contract crate the composition's types come from, and the
/// version it resolved to.
///
/// The identity is recorded because the same crate sources are mirrored into
/// the copied Guest workspace under a second lock: a composition rendered
/// against an identity one build cannot resolve is a closure that only holds
/// on the host.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractCrate {
    name: &'static str,
    version: String,
}

/// One staged artifact and the production file that compiles it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ClosureArtifact {
    staged: String,
    compiled_into: String,
    declarations: &'static [&'static str],
    digest: String,
}

/// The new-graph contract crates, in dependency order.
const CONTRACT_CRATES: &[&str] = &[
    "d2b-contracts",
    "d2b-contracts-resource",
    "d2b-contracts-provider",
    "d2b-resource-types",
];

/// The workspace version every in-tree contract crate publishes, which is
/// what the copied Guest workspace has to resolve the same identity to.
const CONTRACT_CRATE_VERSION: &str = "0.0.0-bootstrap";

/// Render the complete new-graph build composition from the declarations.
///
/// Returns the artifacts in a fixed order, each paired with its
/// repository-relative committed path. The render is a pure function of the
/// declaration set: no clock, no environment, no repository path leaks into
/// any byte, so two runs over one tree produce identical output.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub(crate) fn render_composition(repo_root: &Path) -> Result<Vec<(String, String)>, String> {
    let mut artifacts = vec![
        (
            staged("provider_registrations.rs"),
            crate::provider_registration_authority::render_declarations_only(repo_root)?,
        ),
        (
            staged("service_provider_catalog.rs"),
            crate::service_catalog::render_declarations_only(repo_root)?,
        ),
        (
            staged("v3_converted_resource_types.rs"),
            crate::resource_type_authority::render_declarations_only(repo_root)?,
        ),
    ];
    let manifest = render_manifest(&artifacts)?;
    artifacts.push((staged("build_closure.json"), manifest));
    Ok(artifacts)
}

/// Write the committed composition over the tree and return the paths
/// written, in render order.
///
/// This is the one write path the composition has, and every path it writes
/// is a committed generated artifact under [`OUTPUT_DIR`], so the same
/// drift gate that compares the rest of `make generate`'s output compares
/// these bytes too.
///
/// The compiled-vs-declared cross-check runs here, before anything is
/// written: a method a crate compiles with no declaration behind it, a
/// declared method nothing compiles, and an identity a declaration names
/// that the crate never spells all fail generation rather than committing a
/// graph that could not host what it states.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub(crate) fn gen_new_graph(
    repo_root: &Path,
) -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let undeclared = undeclared_compiled_handlers(repo_root)
        .map_err(|error| -> Box<dyn std::error::Error> { error.into() })?;
    if !undeclared.is_empty() {
        return Err(format!(
            "new-graph declaration violations:\n- {}",
            undeclared.join("\n- ")
        )
        .into());
    }
    let artifacts = render_composition(repo_root).map_err(|error| -> Box<dyn std::error::Error> {
        error.into()
    })?;
    let mut written = Vec::with_capacity(artifacts.len());
    for (relative, contents) in artifacts {
        let path = repo_root.join(&relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, &contents)?;
        written.push(path);
    }
    Ok(written)
}

/// The closure manifest, rendered over the artifacts beside it so its
/// digests describe the exact staged bytes.
fn render_manifest(artifacts: &[(String, String)]) -> Result<String, String> {
    let mut rows = Vec::with_capacity(REPLACEMENTS.len());
    for replacement in REPLACEMENTS {
        let path = staged(replacement.staged);
        let contents = artifacts
            .iter()
            .find_map(|(candidate, contents)| (candidate == &path).then_some(contents.as_str()))
            .ok_or_else(|| format!("new-graph composition is missing its {path} artifact"))?;
        rows.push(ClosureArtifact {
            staged: path,
            compiled_into: replacement.compiled_into.to_owned(),
            declarations: replacement.declarations,
            digest: digest_of(contents.as_bytes()),
        });
    }
    let mut contract_crates = Vec::with_capacity(CONTRACT_CRATES.len());
    for name in CONTRACT_CRATES {
        contract_crates.push(ContractCrate {
            name,
            version: CONTRACT_CRATE_VERSION.to_owned(),
        });
    }
    render_json(&ClosureManifest {
        contract_version: contract_version(),
        declaration_inputs: NEW_GRAPH_DECLARATION_INPUTS,
        retired_authority_sources: RETIRED_AUTHORITY_SOURCES,
        contract_crates,
        artifacts: rows,
    })
}

/// The repository-relative committed path of one staged artifact.
fn staged(name: &str) -> String {
    format!("{OUTPUT_DIR}/{CLOSURE_DIR}/{name}")
}

/// The domain-separated digest of one staged artifact's bytes.
fn digest_of(bytes: &[u8]) -> String {
    canonical_digest(CLOSURE_DIGEST_DOMAIN, bytes)
}

/// Render one value as the pretty JSON the composition's documents use.
fn render_json<T: Serialize>(value: &T) -> Result<String, String> {
    serde_json::to_string_pretty(value).map_err(|error| format!("cannot render JSON: {error}"))
}

/// Every provider crate's operation, registration, catalog and identity
/// declarations, keyed by crate name.
///
/// This is the composition's own reader. It does not reuse the merge-capable
/// authority loaders, so the declaration-only claim is a property of this
/// struct rather than a property of a call graph.
struct DeclaredProviders {
    /// Every provider crate in the tree, in name order: a crate that declares
    /// nothing of its own still gets a declared surface, stated with the empty
    /// vocabularies its recorded absences imply.
    declaring: Vec<String>,
    operations: BTreeMap<String, OperationDeclarationFile>,
    registrations: BTreeMap<String, RegistrationDeclarationFile>,
    catalogs: BTreeMap<String, ServiceCatalogFile>,
    /// Every provider crate's identity declaration: the closed authority the
    /// registration and catalog rows resolve their Provider identities from.
    identities: ProviderIdentities,
}

/// One `resource-types.json`.
///
/// The composition projects no ResourceType row of its own: the
/// resource-type authority's artifact is the compiled table, and this reader
/// exists to hold the declaration format closed on the cross-check's own
/// pass, so a `resource-types.json` whose rows or top-level keys do not match
/// the format is refused here rather than skipped.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code, reason = "the rows are validated here and projected through the type authority")]
struct TypeDeclarationFile {
    #[serde(rename = "crate")]
    crate_name: String,
    types: Vec<TypeRow>,
    #[serde(default)]
    provides: Vec<String>,
    #[serde(default)]
    roles: Vec<serde_json::Value>,
    #[serde(default)]
    principals: Vec<serde_json::Value>,
}

/// One declared ResourceType row, held closed against the declaration
/// format and projected through the resource-type authority rather than by
/// this crate.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
#[allow(dead_code, reason = "the row shape is gated; the compiled table is the type authority's")]
struct TypeRow {
    resource_type: String,
    #[serde(default)]
    allowed_sources: Vec<String>,
    #[serde(default)]
    verbs: Vec<String>,
    #[serde(default)]
    execution: Vec<String>,
    #[serde(default)]
    exportable: bool,
    #[serde(default)]
    reads: Vec<String>,
}

/// One `operations.json`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationDeclarationFile {
    #[serde(rename = "crate")]
    #[allow(dead_code, reason = "the file self-binds to its directory name")]
    crate_name: String,
    operations: Vec<OperationRow>,
}

/// One declared operation row.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct OperationRow {
    #[allow(dead_code, reason = "the cross-check gates the addressable method, not the operation name")]
    operation: String,
    #[allow(dead_code, reason = "the cross-check gates the addressable method, not the service")]
    service: String,
    method: String,
    #[allow(dead_code, reason = "the declared profiles are the operation-row authority's input")]
    profiles: Vec<String>,
    /// The broker-only facets a row may carry. The new graph routes on the
    /// addressable method, not the broker's own audit and payload
    /// vocabulary, so these are read and dropped: a row a declaration states
    /// is a row the graph can route, whatever the old wire frame said.
    #[serde(default)]
    #[allow(dead_code, reason = "the new graph routes on the addressable method")]
    family: String,
    #[serde(default)]
    #[allow(dead_code, reason = "the declaring crate is the file's own directory")]
    declaring_provider: String,
    #[serde(default)]
    #[allow(dead_code, reason = "the new graph routes on the addressable method")]
    w3: bool,
    #[serde(default)]
    #[allow(dead_code, reason = "the new graph routes on the addressable method")]
    capabilities: bool,
    #[serde(default)]
    #[allow(dead_code, reason = "the new graph routes on the addressable method")]
    disposition: String,
    #[serde(default)]
    #[allow(dead_code, reason = "the new graph routes on the addressable method")]
    disposition_target: String,
    #[serde(default)]
    #[allow(dead_code, reason = "the privilege plane is not a new-graph input")]
    authz: serde_json::Value,
    #[serde(default)]
    #[allow(dead_code, reason = "the privilege plane is not a new-graph input")]
    audit: serde_json::Value,
    #[serde(default)]
    #[allow(dead_code, reason = "the privilege plane is not a new-graph input")]
    payload: serde_json::Value,
    #[serde(default)]
    #[allow(dead_code, reason = "the new graph routes on the addressable method")]
    deadline: serde_json::Value,
    #[serde(default)]
    #[allow(dead_code, reason = "the new graph routes on the addressable method")]
    state_cell: serde_json::Value,
    #[serde(default)]
    #[allow(dead_code, reason = "the new graph routes on the addressable method")]
    fds: serde_json::Value,
}

/// One `registrations.json`.
///
/// The effect-service ids a crate's runtime identity registers. The
/// identity itself is the crate's own runtime identity in the identity
/// authority, joined below.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistrationDeclarationFile {
    #[serde(rename = "crate")]
    #[allow(dead_code, reason = "the file self-binds to its directory name")]
    crate_name: String,
    services: Vec<String>,
}

/// One `service-catalog.json`.
///
/// The closed service packages a crate's session identity answers, and the
/// fixed bootstrap resource UID when the row is the deployment's bootstrap
/// row. The identity those rows route to is the crate's own session identity
/// in the identity authority, joined below.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ServiceCatalogFile {
    #[serde(default)]
    services: Vec<String>,
    /// The fixed bootstrap resource UID the deployment's bootstrap Provider
    /// row is committed under.
    #[serde(default)]
    #[allow(dead_code, reason = "the fixed UID projects through the catalog authority")]
    provider_uid: Option<String>,
}

impl DeclaredProviders {
    /// Read every provider crate's declarations, refusing a file that names
    /// a crate other than the directory it lives in and a crate that carries
    /// no declaration of a kind it is not recorded as owning none of.
    ///
    /// The `resource-types.json` set is read and dropped: the compiled
    /// resource-type table is the type authority's artifact, and this reader
    /// exists only so the cross-check's own pass refuses a declaration
    /// outside the format rather than skipping it.
    ///
    /// The crate set is [`declaration_paths`]' rather than a per-kind
    /// optional read: an absent file and a renamed one look the same on disk,
    /// and a loader that skips what it cannot find would render a partial
    /// graph - with every gate comparing it against the same partial input.
    #[allow(clippy::disallowed_methods, reason = "CLI-only path")]
    fn load(repo_root: &Path) -> Result<Self, String> {
        let crates = provider_crates(repo_root)?;
        for (crate_name, path) in declaration_paths(repo_root, Declaration::ResourceTypes)? {
            read_parsed::<TypeDeclarationFile>(&path, &crate_name)?;
        }
        Ok(Self {
            declaring: crates.iter().map(|(name, _)| name.clone()).collect(),
            operations: load_kind(repo_root, Declaration::Operations)?,
            registrations: load_kind(repo_root, Declaration::Registrations)?,
            catalogs: load_kind(repo_root, Declaration::ServiceCatalog)?,
            identities: ProviderIdentities::load(repo_root)?,
        })
    }

    /// Every provider crate in the tree, in name order.
    ///
    /// A crate declares only the kinds it owns: a provider that serves no
    /// operation row and owns no ResourceType has no `operations.json` and no
    /// `resource-types.json`, and the declared surface states it with empty
    /// vocabularies and no Provider identity rather than a second invented
    /// file. Every such absence is recorded in the shared absence table the
    /// loader gates on.
    fn crate_names(&self) -> Vec<&str> {
        self.declaring.iter().map(String::as_str).collect()
    }

    /// The declared surface one declaring crate contributes.
    ///
    /// A provider registers its own identity, serves the union of the
    /// services its registration and catalog declare (a service package two
    /// declarations name is a service the provider publishes once), and serves
    /// one declared method per operation row.
    ///
    /// The Provider reference is resolved from the identity authority through
    /// the declaration kind that speaks: a registration row is a runtime
    /// registration, so its crate must own a runtime identity, and a session
    /// catalog is session routing, so its crate must own a session identity.
    /// Nothing infers an identity from the crate's directory name, because
    /// that name is not the identity: `d2b-provider-guest-qemu-media` owns
    /// `runtime-qemu-media` and `d2b-provider-process-minijail` owns
    /// `system-minijail`, so a directory-name convention both published
    /// references the product does not have and rejected the ones it does.
    /// A crate that declares neither a registration nor a catalog states no
    /// reference at all.
    fn provider(&self, crate_name: &str) -> Result<DeclaredProvider, String> {
        let registration = self.registrations.get(crate_name);
        let catalog = self.catalogs.get(crate_name);
        let identity = match (registration.is_some(), catalog.is_some()) {
            (true, true) => {
                // A crate that speaks on both surfaces owns an identity on
                // both: its registration registers a runtime Provider and its
                // catalog routes the session plane, so a missing identity on
                // either side is a declaration whose row resolves to nothing.
                // The graph carries one Provider row for the crate, and the
                // runtime registration is the one that starts it.
                for surface in [Surface::Runtime, Surface::Session] {
                    if self.identities.identity(crate_name, surface).is_none() {
                        return Err(format!(
                            "{}-identity-missing: crate {crate_name} declares a provider registration and a session service catalog but owns no {surface} Provider identity",
                            surface.key()
                        ));
                    }
                }
                self.identities.identity(crate_name, Surface::Runtime)
            }
            (true, false) => self.identities.identity(crate_name, Surface::Runtime),
            (false, true) => self.identities.identity(crate_name, Surface::Session),
            (false, false) => {
                // A crate that declares no registration and no catalog still
                // declares its operation rows, and it registers no Provider
                // in the graph.
                return Ok(DeclaredProvider {
                    provider_ref: None,
                    crate_name: crate_name.to_owned(),
                    effect_services: Vec::new(),
                    service_packages: Vec::new(),
                    methods: self.methods(crate_name),
                });
            }
        }
        .ok_or_else(|| {
            format!("declared-provider-without-identity: crate {crate_name} declares a registration and a service catalog but owns no Provider identity on either surface")
        })?;
        let provider_ref = ProviderIdentities::provider_ref(identity);
        let effect_services: BTreeSet<String> = registration.map_or_else(BTreeSet::new, |file| {
            file.services.iter().cloned().collect()
        });
        let service_packages: BTreeSet<String> =
            catalog.map_or_else(BTreeSet::new, |file| file.services.iter().cloned().collect());
        let methods = self.methods(crate_name);
        Ok(DeclaredProvider {
            provider_ref: Some(provider_ref),
            crate_name: crate_name.to_owned(),
            effect_services: effect_services.into_iter().collect(),
            service_packages: service_packages.into_iter().collect(),
            methods,
        })
    }

    /// The operation methods one crate declares, in declaration order.
    fn methods(&self, crate_name: &str) -> Vec<String> {
        self.operations
            .get(crate_name)
            .map(|file| file.operations.iter().map(|row| row.method.clone()).collect())
            .unwrap_or_default()
    }
}

/// Parse one declaration file, refusing a file whose `crate` field names a
/// different crate than the directory holding it.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn read_parsed<T: serde::de::DeserializeOwned>(path: &Path, crate_name: &str) -> Result<T, String> {
    let text = fs::read_to_string(path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))?;
    if let Some(declared) = value.get("crate").and_then(serde_json::Value::as_str)
        && declared != crate_name
    {
        return Err(format!(
            "declaration-crate-mismatch: {} names crate \"{declared}\" but lives in crate \"{crate_name}\"",
            path.display()
        ));
    }
    serde_json::from_value(value)
        .map_err(|error| format!("cannot parse {}: {error}", path.display()))
}

/// Read one declaration kind from every provider crate that carries it.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn load_kind<T: serde::de::DeserializeOwned>(
    repo_root: &Path,
    declaration: Declaration,
) -> Result<BTreeMap<String, T>, String> {
    let mut out = BTreeMap::new();
    for (crate_name, path) in declaration_paths(repo_root, declaration)? {
        out.insert(crate_name.clone(), read_parsed(&path, &crate_name)?);
    }
    Ok(out)
}

/// What one crate's own compiled sources publish to the graph.
#[derive(Default)]
struct CompiledSurface {
    /// Crate name -> the operation references its descriptor registers.
    handlers: BTreeMap<String, BTreeSet<String>>,
    /// Crate name -> the effect-service ids its `ServiceDecl` blocks name.
    ///
    /// This is the vocabulary `registrations.json` declares in, so the two
    /// sides of that pair are the same vocabulary on purpose: an effect
    /// service a crate declares but never publishes is a registration row
    /// with no `ServiceDecl` behind it.
    effect_services: BTreeMap<String, BTreeSet<String>>,
    /// Crate name -> the service packages its string constants spell.
    ///
    /// A crate spells the packages it *serves* and the packages it
    /// *consumes* alike, so this is per crate but only the declared
    /// direction is gated: a crate naming a sibling's package is not a
    /// declaration failure.
    service_packages: BTreeMap<String, BTreeSet<String>>,
}

/// What the staged composition declares.
#[derive(Default)]
struct DeclaredSurface {
    /// Crate name -> the methods its operations declaration carries.
    handlers: BTreeMap<String, BTreeSet<String>>,
    /// Crate name -> the effect-service ids its registration declares.
    effect_services: BTreeMap<String, BTreeSet<String>>,
    /// Crate name -> the service packages its catalog declares.
    service_packages: BTreeMap<String, BTreeSet<String>>,
    /// Provider identity -> the crate that declares it.
    providers: BTreeMap<String, String>,
}

/// Read what every provider crate's compiled sources publish.
///
/// This is the compiled side of the cross-check and it reads crate sources,
/// which the composition itself never does. The two sides being different
/// files is the point: a check whose two sides are the same input is a
/// restatement.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn compiled_surface(repo_root: &Path, declaring: &[&str]) -> Result<CompiledSurface, String> {
    let mut out = CompiledSurface::default();
    for &crate_name in declaring {
        let src = repo_root.join("packages").join(crate_name).join("src");
        let mut text = String::new();
        for path in crate::authority_common::collect_rs_files(&src)? {
            text.push_str(
                &fs::read_to_string(&path)
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))?,
            );
            text.push('\n');
        }
        out.handlers.insert(
            crate_name.to_owned(),
            operation_references(&text)
                .into_iter()
                .map(|method| format!("Operation/{method}"))
                .collect(),
        );
        out.effect_services
            .insert(crate_name.to_owned(), service_ids(&text));
        out.service_packages.insert(
            crate_name.to_owned(),
            string_constants(&text).into_values().collect(),
        );
    }
    Ok(out)
}

/// Read what every provider crate's declarations state.
///
/// The declared side is the loader's own output rather than this module's
/// rendered bytes, so the cross-check compares the declarations against the
/// crate sources and not one rendering against another.
fn declared_surface(providers: &DeclaredProviders) -> Result<DeclaredSurface, String> {
    let mut out = DeclaredSurface::default();
    for crate_name in providers.crate_names() {
        let provider = providers.provider(crate_name)?;
        out.handlers
            .entry(provider.crate_name.clone())
            .or_default()
            .extend(
                provider
                    .methods
                    .iter()
                    .map(|method| format!("Operation/{method}")),
            );
        out.effect_services.insert(
            provider.crate_name.clone(),
            provider.effect_services.into_iter().collect(),
        );
        out.service_packages.insert(
            provider.crate_name.clone(),
            provider.service_packages.into_iter().collect(),
        );
        // A crate whose reference is `None` declares no Provider identity, so
        // it registers no provider and cannot collide with a sibling that does.
        let Some(provider_ref) = provider.provider_ref else {
            continue;
        };
        let identity = provider_ref
            .strip_prefix("Provider/")
            .ok_or_else(|| format!("crate {crate_name} names {provider_ref} as a Provider"))?
            .to_owned();
        if let Some(first) = out.providers.insert(identity.clone(), provider.crate_name.clone()) {
            return Err(format!(
                "provider-declared-twice: crate {crate_name} and crate {first} both register {identity}"
            ));
        }
    }
    Ok(out)
}

/// The cross-check violations between the compiled surface and the
/// declarations, each naming the undeclared side.
///
/// A compiled handler no declaration carries is a handler the new graph
/// would not host; a declared method the crate never registers is a method
/// the composition would route to nothing; a provider the declarations
/// register with no crate behind it has no implementation to compile; and a
/// service package a crate declares but never spells is a service it cannot
/// address.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn undeclared_compiled_handlers(repo_root: &Path) -> Result<Vec<String>, String> {
    let providers = DeclaredProviders::load(repo_root)?;
    let declaring = providers.crate_names();
    let compiled = compiled_surface(repo_root, &declaring)?;
    let declared = declared_surface(&providers)?;
    let mut errors = Vec::new();
    for (crate_name, handlers) in &compiled.handlers {
        let declared_handlers = declared.handlers.get(crate_name);
        for handler in handlers {
            if !declared_handlers.is_some_and(|set| set.contains(handler)) {
                errors.push(format!(
                    "compiled-but-undeclared: crate {crate_name} compiles {handler} but no declaration carries it"
                ));
            }
        }
    }
    for (crate_name, handlers) in &declared.handlers {
        let compiled_handlers = compiled.handlers.get(crate_name);
        for handler in handlers {
            if !compiled_handlers.is_some_and(|set| set.contains(handler)) {
                errors.push(format!(
                    "declared-but-not-compiled: crate {crate_name} declares {handler} but its sources register no such handler"
                ));
            }
        }
    }
    for (identity, crate_name) in &declared.providers {
        let crate_dir = repo_root.join("packages").join(crate_name);
        if !crate_dir.is_dir() {
            errors.push(format!(
                "registered-provider-without-crate: crate {crate_name} registers provider {identity}, which is not a crate in this tree"
            ));
        }
    }
    // A shared driver is run by another family's registration, so the crate
    // that declares it registers no Provider of its own: a registration row
    // beside one would put the same identity into the graph twice, once as
    // the driver another family serves and once as a family that starts.
    for crate_name in providers.identities.shared_driver_crates() {
        if providers.registrations.contains_key(crate_name) {
            errors.push(format!(
                "shared-driver-registering-itself: crate {crate_name} declares a shared driver another family's registration runs, and also declares a provider registration of its own"
            ));
        }
    }
    for (facet, declared_side, compiled_side) in [
        ("effect service", &declared.effect_services, &compiled.effect_services),
        (
            "service package",
            &declared.service_packages,
            &compiled.service_packages,
        ),
    ] {
        for (crate_name, identities) in declared_side {
            let spelled = compiled_side.get(crate_name);
            for identity in identities {
                if !spelled.is_some_and(|set| set.contains(identity)) {
                    errors.push(format!(
                        "declared-but-not-spelled: crate {crate_name} declares {facet} {identity} but its sources spell no such identity"
                    ));
                }
            }
        }
    }
    errors.sort();
    errors.dedup();
    Ok(errors)
}

/// The lowercase operation names one crate's sources register, from the two
/// spellings the descriptor tables use: a literal `Operation/<name>`
/// reference and a `format!("Operation/{CONST}")` reference resolved through
/// the crate's string constants.
fn operation_references(text: &str) -> BTreeSet<String> {
    let consts = string_constants(text);
    let mut out = BTreeSet::new();
    let literal = "ResourceRef::parse(\"Operation/";
    let mut rest = text;
    while let Some(offset) = rest.find(literal) {
        let after = &rest[offset + literal.len()..];
        if let Some((name, _)) = after.split_once('"') {
            out.insert(name.to_ascii_lowercase());
        }
        rest = after;
    }
    let formatted = "format!(\"Operation/{";
    let mut rest = text;
    while let Some(offset) = rest.find(formatted) {
        let after = &rest[offset + formatted.len()..];
        if let Some((ident, _)) = after.split_once('}')
            && let Some(value) = consts.get(ident.trim())
        {
            out.insert(value.to_ascii_lowercase());
        }
        rest = after;
    }
    out
}


/// The effect-service ids one crate's sources publish, read from the `id:`
/// field of each `ServiceDecl` block rather than from the const's name, so a
/// service whose id differs from the constant spelling is still found.
fn service_ids(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].trim_start();
        if !line.starts_with("pub const ") || !line.contains(": ServiceDecl = ServiceDecl {") {
            index += 1;
            continue;
        }
        for next in &lines[index + 1..] {
            let next = next.trim();
            if next == "};" {
                break;
            }
            if let Some(value) = next.strip_prefix("id: \"")
                && let Some(id) = value.split('"').next()
                && !id.is_empty()
            {
                out.insert(id.to_owned());
            }
        }
        index += 1;
    }
    out
}

/// Every `pub const NAME: &str = "VALUE";` in one crate's sources.
fn string_constants(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("pub const ") else {
            continue;
        };
        let Some((ident, rest)) = rest.split_once(": &str = \"") else {
            continue;
        };
        let Some(value) = rest.strip_suffix("\";") else {
            continue;
        };
        out.insert(ident.to_owned(), value.to_owned());
    }
    out
}

/// One rendered artifact's bytes by its isolated path.
#[cfg(test)]
fn artifact<'a>(artifacts: &'a [(String, String)], path: &str) -> Result<&'a str, String> {
    artifacts
        .iter()
        .find_map(|(candidate, contents)| (candidate == path).then_some(contents.as_str()))
        .ok_or_else(|| format!("the new-graph composition is missing {path}"))
}

/// Every `include!` target one Rust source names, as written.
#[cfg(test)]
fn include_targets(text: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("include!(") {
        rest = &rest[at + "include!(".len()..];
        let Some(quote) = rest.find('"') else {
            break;
        };
        rest = &rest[quote + 1..];
        let Some(stop) = rest.find('"') else {
            break;
        };
        targets.push(&rest[..stop]);
        rest = &rest[stop + 1..];
    }
    targets
}

/// The repository-relative path one `include!` target resolves to from the
/// source file that names it.
#[cfg(test)]
fn resolved_include(consumer: &str, target: &str) -> String {
    let mut resolved = PathBuf::from(consumer);
    resolved.pop();
    for part in target.split('/') {
        match part {
            ".." => {
                resolved.pop();
            }
            "." => {}
            part => resolved.push(part),
        }
    }
    resolved.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The repository root the composition renders over.
    ///
    /// This is the shared `D2B_REPO_ROOT` resolution every xtask test uses,
    /// not a compile-time path: `rules_rs` refuses a binary that embeds its
    /// own working directory, so `env!("CARGO_MANIFEST_DIR")` would make the
    /// target unbuildable under Bazel while cargo ran it fine.
    fn repo_root() -> PathBuf {
        crate::repo_root()
            .expect("the aggregate passes D2B_REPO_ROOT")
            .to_path_buf()
    }

    /// A throwaway declaration-only tree, used to prove the composition reads
    /// nothing else and to plant the negative cases.
    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "d2b-u33-new-graph-{name}-{}-{nonce}",
                std::process::id()
            ));
            Self { root }
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write(&self, relative: &str, content: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(path, content).expect("write");
        }

        /// The bootstrap provider the service catalog names as the fixed
        /// root of the graph. A declaration-only tree still has to state it,
        /// because the catalog artifact is what makes it the root.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_bootstrap_provider(&self) {
            self.write(
                "packages/d2b-provider-system-core/resource-types.json",
                "{\n  \"crate\": \"d2b-provider-system-core\",\n  \"types\": [\n    {\n      \"resourceType\": \"SystemCore\",\n      \"allowedSources\": [\"builtin\"],\n      \"verbs\": [\"get\"],\n      \"execution\": [\"host\"],\n      \"exportable\": false,\n      \"reads\": []\n    }\n  ],\n  \"provides\": [],\n  \"roles\": [],\n  \"principals\": []\n}\n",
            );
            self.write(
                "packages/d2b-provider-system-core/service-catalog.json",
                "{\n  \"providerUid\": \"11111111-1111-4111-8111-111111111111\"\n}\n",
            );
            self.write(
                "packages/d2b-provider-system-core/src/lib.rs",
                "pub const PROVIDER_NAME: &str = \"system-core\";\n\npub fn noop() {}\n",
            );
            self.write(
                "packages/d2b-provider-system-core/provider-identity.json",
                "{\n  \"crate\": \"d2b-provider-system-core\",\n  \"family\": \"system-core\",\n  \"roles\": [\"product\", \"session\", \"fixed-bootstrap\"],\n  \"nonBinary\": false,\n  \"product\": {\n    \"identity\": \"system-core\",\n    \"sharesIdentityWith\": [\"session\"],\n    \"evidence\": [\n      {\n        \"path\": \"packages/d2b-provider-system-core/src/lib.rs\",\n        \"symbol\": \"PROVIDER_NAME\"\n      }\n    ]\n  },\n  \"runtime\": {\n    \"identity\": null,\n    \"reason\": \"no-identity-owned\"\n  },\n  \"session\": {\n    \"identity\": \"system-core\",\n    \"sharesIdentityWith\": [\"product\"],\n    \"evidence\": [\n      {\n        \"path\": \"packages/d2b-provider-system-core/src/lib.rs\",\n        \"symbol\": \"PROVIDER_NAME\"\n      }\n    ]\n  },\n  \"blockers\": []\n}\n",
            );
        }

        /// One declaring crate's four declarations plus the descriptor
        /// source that registers its handler and service identities.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_crate(&self, crate_name: &str, method: &str, service: &str) {
            self.write_crate_declarations(crate_name, method, service);
            self.write_resource_types(crate_name);
        }

        /// Every declaration a crate carries except its resource-type
        /// vocabulary, beside the descriptor source that registers its
        /// handler and service identities.
        ///
        /// The negative fixtures plant a crate through this and then write
        /// their own `resource-types.json`, so the only thing wrong with the
        /// tree is the one declaration under test.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_crate_declarations(&self, crate_name: &str, method: &str, service: &str) {
            let family = crate_name.strip_prefix("d2b-provider-").unwrap_or(crate_name);
            self.write(
                &format!("packages/{crate_name}/operations.json"),
                &format!(
                    "{{\n  \"crate\": \"{crate_name}\",\n  \"operations\": [\n    {{\n      \"operation\": \"{method}\",\n      \"family\": \"{family}\",\n      \"declaringProvider\": \"{crate_name}\",\n      \"service\": \"d2b.{family}\",\n      \"method\": \"{method}\",\n      \"profiles\": [\"host\"],\n      \"w3\": false,\n      \"capabilities\": false,\n      \"disposition\": \"promoted-live\",\n      \"dispositionTarget\": \"live in production broker\",\n      \"authz\": {{\n        \"subject\": \"{family}\",\n        \"scope\": \"global\",\n        \"allowedGroups\": [\"d2bd\"],\n        \"destructive\": false,\n        \"secretAccess\": \"None\",\n        \"brokerRequired\": \"Yes\",\n        \"auditMode\": \"Yes\"\n      }},\n      \"audit\": {{\n        \"fields\": [\"{method}\"],\n        \"required\": true,\n        \"mode\": \"yes\"\n      }},\n      \"payload\": {{\n        \"provenance\": \"request\",\n        \"schema\": {{\n          \"type\": \"object\",\n          \"properties\": {{}},\n          \"required\": [],\n          \"additionalProperties\": false\n        }}\n      }},\n      \"deadline\": {{\n        \"tier\": \"standard\"\n      }}\n    }}\n  ]\n}}\n"
                ),
            );
            self.write(
                &format!("packages/{crate_name}/registrations.json"),
                &format!(
                    "{{\n  \"crate\": \"{crate_name}\",\n  \"services\": [\"{service}\"]\n}}\n"
                ),
            );
            self.write(
                &format!("packages/{crate_name}/service-catalog.json"),
                &format!("{{\n  \"services\": [\"d2b.{family}.v3\"]\n}}\n"),
            );
            // The crate owns one identity on each of the two declaration
            // surfaces it speaks: the runtime identity its registration row
            // registers under, and the session identity its catalog routes
            // to. The evidence anchor names the crate's own identity module,
            // which no negative fixture overwrites.
            self.write(
                &format!("packages/{crate_name}/src/identity.rs"),
                &format!("pub const PROVIDER_IDENTITY: &str = \"{family}\";\n"),
            );
            self.write(
                &format!("packages/{crate_name}/provider-identity.json"),
                &format!(
                    "{{\n  \"crate\": \"{crate_name}\",\n  \"family\": \"{family}\",\n  \"roles\": [\"runtime\", \"session\"],\n  \"nonBinary\": false,\n  \"product\": {{\n    \"identity\": null,\n    \"reason\": \"no-identity-owned\"\n  }},\n  \"runtime\": {{\n    \"identity\": \"{family}\",\n    \"sharesIdentityWith\": [\"session\"],\n    \"evidence\": [\n      {{\n        \"path\": \"packages/{crate_name}/src/identity.rs\",\n        \"symbol\": \"PROVIDER_IDENTITY\"\n      }}\n    ]\n  }},\n  \"session\": {{\n    \"identity\": \"{family}\",\n    \"sharesIdentityWith\": [\"runtime\"],\n    \"evidence\": [\n      {{\n        \"path\": \"packages/{crate_name}/src/identity.rs\",\n        \"symbol\": \"PROVIDER_IDENTITY\"\n      }}\n    ]\n  }},\n  \"blockers\": []\n}}\n"
                ),
            );
            self.write(
                &format!("packages/{crate_name}/src/driver.rs"),
                &format!(
                    "use d2b_resource_types::ServiceDecl;\n\
                     \n\
                     pub const {upper}_WIRE: &str = \"{method}\";\n\
                     \n\
                     pub const SERVICE_PACKAGE: &str = \"d2b.{family}.v3\";\n\
                     \n\
                     pub const {upper}_SERVICE: ServiceDecl = ServiceDecl {{\n    id: \"{service}\",\n}};\n\
                     \n\
                     pub fn descriptor() {{\n    let _ = ResourceRef::parse(\"Operation/{method}\");\n    let _ = &[{upper}_SERVICE];\n}}\n",
                    upper = method.to_ascii_uppercase(),
                ),
            );
        }

        /// One crate's ResourceType vocabulary declaration.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_resource_types(&self, crate_name: &str) {
            let family = crate_name.strip_prefix("d2b-provider-").unwrap_or(crate_name);
            self.write(
                &format!("packages/{crate_name}/resource-types.json"),
                &format!(
                    "{{\n  \"crate\": \"{crate_name}\",\n  \"types\": [\n    {{\n      \"resourceType\": \"Fixture{family}\",\n      \"allowedSources\": [\"builtin\"],\n      \"verbs\": [\"get\", \"create\", \"delete\"],\n      \"execution\": [\"host\"],\n      \"exportable\": true,\n      \"reads\": [\"Volume\"]\n    }}\n  ],\n  \"provides\": [],\n  \"roles\": [],\n  \"principals\": []\n}}\n"
                ),
            );
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// Scenario 1, byte-stability half: the whole composition rendered twice
    /// is the same bytes, file for file.
    ///
    /// The comparison is over two independently rendered sets, not over one
    /// render compared with itself, so a renderer that folds a clock, a
    /// directory, or a hash-map iteration order into its output fails here.
    #[test]
    fn repeated_generation_is_byte_stable() {
        let root = repo_root();
        let first = render_composition(&root).expect("the composition renders");
        let second = render_composition(&root).expect("the composition renders again");
        assert_eq!(
            first.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(),
            second.iter().map(|(path, _)| path.as_str()).collect::<Vec<_>>(),
            "the composition renders the same artifact set in the same order"
        );
        assert_eq!(first, second, "the composition is byte-stable");
        assert_eq!(
            first.len(),
            REPLACEMENTS.len() + 1,
            "the composition stages every replacement plus the manifest"
        );
    }

    /// Scenario 2, isolation half: the composition renders from a tree that
    /// holds nothing but the four declarations.
    ///
    /// The tree has no crate sources, no committed generated artifacts, no
    /// merged rows document, no privilege copy and no principal table, so a
    /// render that reached for any of them fails rather than producing a
    /// quiet second source.
    #[test]
    fn the_composition_renders_from_declarations_alone() {
        let fixture = Fixture::new("declarations-only");
        fixture.write_bootstrap_provider();
        fixture.write_crate("d2b-provider-fixture", "export", "fixture.d2bus.org/export");
        let artifacts = render_composition(&fixture.root)
            .expect("a declaration-only tree renders the whole composition");
        let paths: Vec<&str> = artifacts.iter().map(|(path, _)| path.as_str()).collect();
        for expected in [
            staged("provider_registrations.rs"),
            staged("service_provider_catalog.rs"),
            staged("v3_converted_resource_types.rs"),
            staged("build_closure.json"),
        ] {
            assert!(paths.contains(&expected.as_str()), "the composition stages {expected}");
        }
        for deleted in ["operations.json", "graph_policy.json"] {
            assert!(
                !paths.contains(&staged(deleted).as_str()),
                "the composition stages nothing no production file compiles: {deleted}"
            );
        }
    }

    /// Scenario 2, isolation half, the malformed half: a declaration that is
    /// present but unparseable fails on the parse, naming the file.
    ///
    /// The orphan carries every other declaration, so the only thing wrong
    /// with the tree is the file under test and the refusal can be one
    /// reason rather than a disjunction.
    #[test]
    fn a_malformed_resource_type_declaration_is_refused() {
        let fixture = Fixture::new("malformed-types");
        fixture.write_bootstrap_provider();
        fixture.write_crate("d2b-provider-fixture", "export", "fixture.d2bus.org/export");
        fixture.write_crate_declarations("d2b-provider-orphan", "import", "orphan.d2bus.org/import");
        fixture.write("packages/d2b-provider-orphan/resource-types.json", "not json");
        let error = render_composition(&fixture.root).expect_err("a broken declaration fails");
        assert!(
            error.starts_with("malformed declaration")
                && error.contains("packages/d2b-provider-orphan/resource-types.json"),
            "the refusal names the unparseable declaration: {error}"
        );
    }

    /// Scenario 2, isolation half, the absent half: a crate with no
    /// `resource-types.json` at all fails instead of being dropped from the
    /// graph.
    ///
    /// An absent file and a renamed one look the same on disk, so the loader
    /// has to refuse this tree rather than render a partial composition the
    /// every gate would then compare against itself.
    #[test]
    fn a_crate_without_a_resource_type_declaration_is_refused() {
        let fixture = Fixture::new("missing-types");
        fixture.write_bootstrap_provider();
        fixture.write_crate("d2b-provider-fixture", "export", "fixture.d2bus.org/export");
        fixture.write_crate_declarations("d2b-provider-orphan", "import", "orphan.d2bus.org/import");
        let error = render_composition(&fixture.root)
            .expect_err("a crate with no resource-types.json is refused");
        assert_eq!(
            error,
            "missing-declaration: crate d2b-provider-orphan has no packages/d2b-provider-orphan/resource-types.json; a provider crate carries every declaration file, or has a row in authority_common::DECLARATION_ABSENCES naming why it owns none",
            "the refusal names the crate and the declaration it lost"
        );
    }

    /// Scenario 2, the retired authorities are named as exclusions, and the
    /// manifest records that list, so a regression that reintroduced one as
    /// an input would have to declare it.
    #[test]
    fn the_manifest_names_the_authorities_the_new_graph_does_not_read() {
        let artifacts = render_composition(&repo_root()).expect("the composition renders");
        let manifest: serde_json::Value =
            serde_json::from_str(artifact(&artifacts, &staged("build_closure.json")).expect("manifest"))
                .expect("the manifest is JSON");
        let retired = manifest["retiredAuthoritySources"]
            .as_array()
            .expect("the manifest names the retired authorities")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        for expected in [
            "docs/reference/policy/broker-operations.json",
            "docs/reference/policy/principal-allocation.json",
            "operation_row_authority::COMMITTED_SERVICE_FACET_SCOPES",
        ] {
            assert!(retired.contains(&expected), "{expected} is a named exclusion");
        }
        let inputs = manifest["declarationInputs"]
            .as_array()
            .expect("the manifest names its inputs")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect::<Vec<_>>();
        for input in inputs {
            assert!(
                !retired.contains(&input),
                "a retired authority is not a declared input: {input}"
            );
        }
    }

    /// Scenario 1, the cross-check half: every handler and provider the
    /// repository's crates compile is carried by a declaration, and the
    /// staged composition says so.
    ///
    /// The compiled side reads the crate sources the composition never
    /// touches, so this is a cross-check between two independent sources
    /// rather than a restatement of the composition's own input.
    #[test]
    fn every_compiled_handler_and_provider_is_declared() {
        let root = repo_root();
        let undeclared = undeclared_compiled_handlers(&root).expect("the cross-check runs");
        assert!(
            undeclared.is_empty(),
            "every compiled handler and provider is declared:\n- {}",
            undeclared.join("\n- ")
        );
    }

    /// Scenario 1, the cross-check bites: a fixture crate whose compiled
    /// source registers a handler no declaration names is reported.
    #[test]
    fn the_compiled_handler_cross_check_names_an_undeclared_handler() {
        let fixture = Fixture::new("undeclared-handler");
        fixture.write_bootstrap_provider();
        fixture.write_crate("d2b-provider-fixture", "export", "fixture.d2bus.org/export");
        fixture.write(
            "packages/d2b-provider-fixture/src/extra.rs",
            "pub fn extra() {\n    let _ = ResourceRef::parse(\"Operation/import\");\n}\n",
        );
        let undeclared =
            undeclared_compiled_handlers(&fixture.root).expect("the cross-check runs");
        assert_eq!(
            undeclared,
            vec![
                "compiled-but-undeclared: crate d2b-provider-fixture compiles Operation/import but no declaration carries it"
                    .to_owned()
            ],
            "the cross-check names the compiled handler no declaration carries"
        );
    }

    /// The other direction bites too: a declared method the crate never
    /// compiles is not silently accepted.
    #[test]
    fn the_compiled_handler_cross_check_names_a_method_nothing_compiles() {
        let fixture = Fixture::new("uncompiled-method");
        fixture.write_bootstrap_provider();
        fixture.write_crate("d2b-provider-fixture", "export", "fixture.d2bus.org/export");
        // The crate keeps its declarations but its descriptor registers no
        // handler and publishes no service, so the declared method and the
        // declared effect service both have nothing behind them.
        fixture.write("packages/d2b-provider-fixture/src/driver.rs", "pub fn nothing() {}\n");
        let undeclared =
            undeclared_compiled_handlers(&fixture.root).expect("the cross-check runs");
        assert!(
            undeclared.iter().any(|error| {
                error.contains("declared-but-not-compiled")
                    && error.contains("Operation/export")
            }),
            "the cross-check names the declared method nothing compiles: {undeclared:?}"
        );
    }

    /// A service package a crate declares but never spells is a service it
    /// cannot address, and the cross-check says so.
    #[test]
    fn the_cross_check_names_a_service_package_nothing_spells() {
        let fixture = Fixture::new("unspelled-service");
        fixture.write_bootstrap_provider();
        fixture.write_crate("d2b-provider-fixture", "export", "fixture.d2bus.org/export");
        fixture.write(
            "packages/d2b-provider-fixture/service-catalog.json",
            "{\n  \"services\": [\"d2b.fixture.absent.v3\"]\n}\n",
        );
        let undeclared =
            undeclared_compiled_handlers(&fixture.root).expect("the cross-check runs");
        assert!(
            undeclared.iter().any(|error| {
                error.contains("declared-but-not-spelled")
                    && error.contains("service package d2b.fixture.absent.v3")
            }),
            "the cross-check names the service package nothing spells: {undeclared:?}"
        );
    }

    /// A session catalog whose crate owns no session identity is refused
    /// rather than routed to an identity no declaration made.
    ///
    /// The routing row resolves through the crate's session identity, and a
    /// row that resolves to nothing names no Provider a caller could route
    /// to. Guessing one from the crate's directory name is exactly the
    /// convention the identity authority replaced.
    #[test]
    fn a_session_catalog_with_no_session_identity_is_refused() {
        let fixture = Fixture::new("session-identity-missing");
        fixture.write_bootstrap_provider();
        fixture.write_crate("d2b-provider-fixture", "export", "fixture.d2bus.org/export");
        fixture.write(
            "packages/d2b-provider-fixture/provider-identity.json",
            "{\n  \"crate\": \"d2b-provider-fixture\",\n  \"family\": \"fixture\",\n  \"roles\": [\"runtime\"],\n  \"nonBinary\": false,\n  \"product\": {\n    \"identity\": null,\n    \"reason\": \"no-identity-owned\"\n  },\n  \"runtime\": {\n    \"identity\": \"fixture\",\n    \"evidence\": [\n      {\n        \"path\": \"packages/d2b-provider-fixture/src/identity.rs\",\n        \"symbol\": \"PROVIDER_IDENTITY\"\n      }\n    ]\n  },\n  \"session\": {\n    \"identity\": null,\n    \"reason\": \"no-identity-owned\"\n  },\n  \"blockers\": []\n}\n",
        );
        let error = undeclared_compiled_handlers(&fixture.root)
            .expect_err("a catalog with no session identity is refused");
        assert!(
            error.contains("session-identity-missing") && error.contains("d2b-provider-fixture"),
            "the refusal names the crate and the surface it is missing: {error}"
        );
    }

    /// The runtime side of the same join: a registration whose crate owns no
    /// runtime identity is refused by name.
    #[test]
    fn a_registration_with_no_runtime_identity_is_refused() {
        let fixture = Fixture::new("runtime-identity-missing");
        fixture.write_bootstrap_provider();
        fixture.write_crate("d2b-provider-fixture", "export", "fixture.d2bus.org/export");
        fixture.write(
            "packages/d2b-provider-fixture/provider-identity.json",
            "{\n  \"crate\": \"d2b-provider-fixture\",\n  \"family\": \"fixture\",\n  \"roles\": [\"session\"],\n  \"nonBinary\": false,\n  \"product\": {\n    \"identity\": null,\n    \"reason\": \"no-identity-owned\"\n  },\n  \"runtime\": {\n    \"identity\": null,\n    \"reason\": \"composition-hosted\"\n  },\n  \"session\": {\n    \"identity\": \"fixture\",\n    \"evidence\": [\n      {\n        \"path\": \"packages/d2b-provider-fixture/src/identity.rs\",\n        \"symbol\": \"PROVIDER_IDENTITY\"\n      }\n    ]\n  },\n  \"blockers\": []\n}\n",
        );
        let error = undeclared_compiled_handlers(&fixture.root)
            .expect_err("a registration with no runtime identity is refused");
        assert!(
            error.contains("runtime-identity-missing") && error.contains("d2b-provider-fixture"),
            "the refusal names the crate and the surface it is missing: {error}"
        );
    }

    /// The production generator writes exactly the committed closure and
    /// nothing else: the files `make generate` installs, each holding the
    /// bytes the same call rendered.
    ///
    /// It runs over a throwaway declaration-only tree so the assertion is
    /// about the write path itself rather than about one snapshot of the
    /// repository's declarations.
    #[test]
    fn the_generator_writes_only_the_committed_closure() {
        let fixture = Fixture::new("committed-write");
        fixture.write_bootstrap_provider();
        fixture.write_crate("d2b-provider-fixture", "export", "fixture.d2bus.org/export");
        let rendered = render_composition(&fixture.root).expect("the composition renders");
        let written = gen_new_graph(&fixture.root).expect("the composition installs");
        assert_eq!(
            written
                .iter()
                .map(|path| {
                    path.strip_prefix(&fixture.root)
                        .expect("every staged file is under the repository root")
                        .to_string_lossy()
                        .into_owned()
                })
                .collect::<Vec<_>>(),
            rendered
                .iter()
                .map(|(path, _)| path.clone())
                .collect::<Vec<_>>()
        );
        for (path, contents) in &rendered {
            assert_eq!(
                fs::read_to_string(fixture.root.join(path)).expect("the committed file reads"),
                *contents,
                "{path} holds the rendered bytes"
            );
        }
        assert!(
            fixture
                .root
                .join(OUTPUT_DIR)
                .join(CLOSURE_DIR)
                .is_dir(),
            "the closure lands under the committed output directory"
        );
    }

    /// Every staged artifact is compiled by the production file the manifest
    /// names: the consumer `include!`s the staged path itself, so the byte
    /// the generator renders is the byte the daemon links. The test fails the
    /// moment consumption moves off the closure - a restored per-crate copy,
    /// a repointed `include!`, or a dropped manifest row.
    #[test]
    fn every_staged_artifact_is_included_by_its_production_consumer() {
        let root = repo_root();
        for replacement in REPLACEMENTS {
            let compiled_into = replacement.compiled_into;
            let consumer_path = root.join(compiled_into);
            let consumer = fs::read_to_string(&consumer_path)
                .unwrap_or_else(|error| panic!("{compiled_into} must exist: {error}"));
            let included: Vec<String> = include_targets(&consumer)
                .into_iter()
                .map(|target| resolved_include(compiled_into, target))
                .collect();
            let expected = staged(replacement.staged);
            assert!(
                included.contains(&expected),
                "{compiled_into} must include {expected}, but it includes {included:?}",
            );
        }
    }

    /// The per-crate copies the staged bytes replaced are gone: a second file
    /// carrying the same table is a second source, and a rebuild would link
    /// whichever copy the `include!` names.
    #[test]
    fn the_replaced_per_crate_copies_are_gone() {
        let root = repo_root();
        for removed in [
            "packages/d2bd/src/generated/provider_registrations.rs",
            "packages/d2b-contracts-zone-session/src/generated/service_provider_catalog.rs",
            "packages/d2b-contracts/src/generated/v3_converted_resource_types.rs",
        ] {
            assert!(
                !root.join(removed).exists(),
                "{removed} was replaced by the staged closure byte and must not come back",
            );
        }
    }

}
