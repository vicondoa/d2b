//! Compiler-derived API-surface policy over paired, already-built rustdoc JSON censuses.
//!
//! This crate deliberately does not invoke Cargo or rustdoc. The caller supplies separate
//! public and private-plus-hidden rustdoc JSON files for every workspace library, together
//! with exact censuses describing both sets. All externally rendered failures use closed
//! operation and error labels; parser, path, source, and tool diagnostics are never forwarded.

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    fmt, fs,
    path::{Component, Path, PathBuf},
};

use rustdoc_types::{
    AssocItemConstraint, AssocItemConstraintKind, Crate as RustdocCrate, DynTrait, Function,
    FunctionPointer, FunctionSignature, GenericArg, GenericArgs, GenericBound, GenericParamDef,
    GenericParamDefKind, Generics, Id, Item, ItemEnum, ItemKind, ItemSummary, Path as RustdocPath,
    PolyTrait, StructKind, Term, Type, VariantKind, Visibility, WherePredicate,
};
use serde::{Deserialize, Serialize};

/// The only rustdoc-producing toolchain accepted by this policy.
pub const PINNED_NIGHTLY: &str = "nightly-2026-02-16";
/// The only accepted rustdoc JSON schema version.
pub const RUSTDOC_FORMAT_VERSION: u32 = 57;
const _: () = assert!(rustdoc_types::FORMAT_VERSION == RUSTDOC_FORMAT_VERSION);
/// The metadata and root-spec schema version.
pub const POLICY_SCHEMA_VERSION: u32 = 1;

/// Closed operation labels suitable for logs and command-line diagnostics.
pub mod operation {
    pub const ARGUMENT_PARSE: &str = "argument-parse";
    pub const METADATA_LOAD: &str = "metadata-load";
    pub const ROOT_SPEC_LOAD: &str = "root-spec-load";
    pub const JSON_DIRECTORY_LOAD: &str = "json-directory-load";
    pub const RUSTDOC_VALIDATE: &str = "rustdoc-validate";
    pub const IDENTITY_RESOLVE: &str = "identity-resolve";
    pub const POLICY_ANALYZE: &str = "policy-analyze";
    pub const PUBLIC_API_RENDER: &str = "public-api-render";
    pub const SNAPSHOT_CHECK: &str = "snapshot-check";
    pub const SNAPSHOT_WRITE: &str = "snapshot-write";
}

/// Closed error labels suitable for logs and command-line diagnostics.
pub mod error_label {
    pub const INVALID_ARGUMENTS: &str = "invalid-arguments";
    pub const INPUT_UNREADABLE: &str = "input-unreadable";
    pub const INPUT_NOT_REGULAR_FILE: &str = "input-not-regular-file";
    pub const INPUT_NOT_DIRECTORY: &str = "input-not-directory";
    pub const INVALID_JSON: &str = "invalid-json";
    pub const SCHEMA_MISMATCH: &str = "schema-mismatch";
    pub const FILE_SET_MISMATCH: &str = "file-set-mismatch";
    pub const TOOLCHAIN_MISMATCH: &str = "toolchain-mismatch";
    pub const FORMAT_MISMATCH: &str = "format-mismatch";
    pub const PRIVATE_ITEMS_REQUIRED: &str = "private-items-required";
    pub const HIDDEN_ITEMS_REQUIRED: &str = "hidden-items-required";
    pub const CENSUS_MISMATCH: &str = "census-mismatch";
    pub const ROOT_MISMATCH: &str = "root-mismatch";
    pub const DUPLICATE_IDENTITY: &str = "duplicate-identity";
    pub const UNRESOLVED_IDENTITY: &str = "unresolved-identity";
    pub const IDENTITY_MISMATCH: &str = "identity-mismatch";
    pub const STRUCTURE_MISMATCH: &str = "structure-mismatch";
    pub const PUBLIC_API_INCOMPLETE: &str = "public-api-incomplete";
    pub const SNAPSHOT_MISMATCH: &str = "snapshot-mismatch";
    pub const SNAPSHOT_INVALID: &str = "snapshot-invalid";
}

/// A deliberately redacted policy failure.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PolicyError {
    operation: &'static str,
    label: &'static str,
}

impl PolicyError {
    #[must_use]
    pub const fn new(operation: &'static str, label: &'static str) -> Self {
        Self { operation, label }
    }

    #[must_use]
    pub const fn operation(self) -> &'static str {
        self.operation
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        self.label
    }
}

impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.operation, self.label)
    }
}

impl std::error::Error for PolicyError {}

pub type Result<T> = std::result::Result<T, PolicyError>;

/// Exact census metadata for all workspace library rustdoc blobs.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceMetadata {
    pub schema_version: u32,
    pub nightly: String,
    pub rustdoc_format_version: u32,
    /// Exact target triple shared by every public and private blob.
    pub target_triple: String,
    pub crates: Vec<CrateMetadata>,
}

/// Exact expected counts and paired file identity for one workspace library.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrateMetadata {
    /// Rust crate name, not Cargo package name (for example `d2b_bus`).
    pub crate_name: String,
    /// A single plain filename below the supplied public JSON directory.
    pub public_json_file: String,
    /// Exact census for the public-only rustdoc blob.
    pub public_census: CrateCensus,
    /// A single plain filename below the supplied private JSON directory.
    pub private_json_file: String,
    /// Exact census for the private-plus-hidden rustdoc blob.
    pub private_census: CrateCensus,
    /// Exact number of compiler-emitted `#[doc(hidden)]` attributes in the private
    /// blob. Zero is meaningful and valid. A nonzero pin distinguishes a complete
    /// private build from one that omitted `--document-hidden-items`.
    pub private_hidden_items: usize,
}

/// Closed census checked before any policy result is produced.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrateCensus {
    pub index_items: usize,
    pub path_items: usize,
    pub external_crates: usize,
}

/// Policy roots and the crate whose canonical public API is snapshotted.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootSpec {
    pub schema_version: u32,
    pub public_api_crate: String,
    pub capability_roots: Vec<Identity>,
    pub claim_roots: Vec<Identity>,
}

/// Stable rustdoc item kind used in composite identities.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityKind {
    Module,
    ExternCrate,
    Use,
    Struct,
    StructField,
    Union,
    Enum,
    Variant,
    Function,
    TypeAlias,
    Constant,
    Trait,
    TraitAlias,
    Impl,
    Static,
    ExternType,
    Macro,
    ProcAttribute,
    ProcDerive,
    AssocConst,
    AssocType,
    Primitive,
    Keyword,
    Attribute,
}

impl From<ItemKind> for IdentityKind {
    fn from(kind: ItemKind) -> Self {
        match kind {
            ItemKind::Module => Self::Module,
            ItemKind::ExternCrate => Self::ExternCrate,
            ItemKind::Use => Self::Use,
            ItemKind::Struct => Self::Struct,
            ItemKind::StructField => Self::StructField,
            ItemKind::Union => Self::Union,
            ItemKind::Enum => Self::Enum,
            ItemKind::Variant => Self::Variant,
            ItemKind::Function => Self::Function,
            ItemKind::TypeAlias => Self::TypeAlias,
            ItemKind::Constant => Self::Constant,
            ItemKind::Trait => Self::Trait,
            ItemKind::TraitAlias => Self::TraitAlias,
            ItemKind::Impl => Self::Impl,
            ItemKind::Static => Self::Static,
            ItemKind::ExternType => Self::ExternType,
            ItemKind::Macro => Self::Macro,
            ItemKind::ProcAttribute => Self::ProcAttribute,
            ItemKind::ProcDerive => Self::ProcDerive,
            ItemKind::AssocConst => Self::AssocConst,
            ItemKind::AssocType => Self::AssocType,
            ItemKind::Primitive => Self::Primitive,
            ItemKind::Keyword => Self::Keyword,
            ItemKind::Attribute => Self::Attribute,
        }
    }
}

/// Fail-closed identity: origin crate, exact definition path, and exact item kind.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    pub origin_crate: String,
    pub path: Vec<String>,
    pub kind: IdentityKind,
}

impl Identity {
    fn snapshot_key(&self) -> String {
        format!(
            "{}::{}\t{}",
            self.origin_crate,
            self.path.join("::"),
            kind_label(self.kind)
        )
    }
}

/// All deterministic snapshots produced by analysis.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Snapshots {
    pub public_api: Vec<String>,
    pub capability_api: Vec<String>,
    pub hidden_public_api: Vec<String>,
    pub capability_trait_impls: Vec<String>,
}

impl Snapshots {
    /// Render one snapshot as sorted newline-terminated records.
    pub fn render(lines: &[String]) -> Result<String> {
        if lines
            .iter()
            .any(|line| line.is_empty() || line.contains(['\n', '\r']))
        {
            return Err(PolicyError::new(
                operation::POLICY_ANALYZE,
                error_label::STRUCTURE_MISMATCH,
            ));
        }
        let mut rendered = lines.join("\n");
        if !rendered.is_empty() {
            rendered.push('\n');
        }
        Ok(rendered)
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StrictRustdocCrate {
    root: Id,
    crate_version: Option<String>,
    includes_private: bool,
    index: HashMap<Id, Item>,
    paths: HashMap<Id, ItemSummary>,
    external_crates: HashMap<u32, rustdoc_types::ExternalCrate>,
    target: rustdoc_types::Target,
    format_version: u32,
}

impl From<StrictRustdocCrate> for RustdocCrate {
    fn from(value: StrictRustdocCrate) -> Self {
        Self {
            root: value.root,
            crate_version: value.crate_version,
            includes_private: value.includes_private,
            index: value.index,
            paths: value.paths,
            external_crates: value.external_crates,
            target: value.target,
            format_version: value.format_version,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BlobKind {
    Public,
    Private,
}

#[derive(Debug)]
struct LoadedCrate {
    metadata: CrateMetadata,
    public_json_path: PathBuf,
    public_krate: RustdocCrate,
    public_local_crate_id: u32,
    public_identities: BTreeMap<Identity, Id>,
    public_identity_by_id: BTreeMap<Id, Identity>,
    public_owner_by_id: BTreeMap<Id, Id>,
    private_krate: RustdocCrate,
    private_local_crate_id: u32,
    private_identities: BTreeMap<Identity, Id>,
    private_identity_by_id: BTreeMap<Id, Identity>,
    private_owner_by_id: BTreeMap<Id, Id>,
}

#[derive(Debug)]
struct ValidatedBlob {
    krate: RustdocCrate,
    local_crate_id: u32,
    identities: BTreeMap<Identity, Id>,
    identity_by_id: BTreeMap<Id, Identity>,
    owner_by_id: BTreeMap<Id, Id>,
}

/// Strictly loaded paired workspace rustdoc censuses.
#[derive(Debug)]
pub struct Workspace {
    crates: BTreeMap<String, LoadedCrate>,
    all_private_identities: BTreeMap<Identity, (String, Id)>,
}

/// Load and strictly validate metadata and every paired public/private JSON blob.
pub fn load_workspace(
    public_json_dir: &Path,
    private_json_dir: &Path,
    metadata_path: &Path,
) -> Result<Workspace> {
    let metadata = read_json::<WorkspaceMetadata>(metadata_path, operation::METADATA_LOAD)?;
    validate_metadata(&metadata)?;
    load_workspace_with_metadata(public_json_dir, private_json_dir, metadata)
}

/// Load a root specification with exact schema validation.
pub fn load_root_spec(path: &Path) -> Result<RootSpec> {
    let roots = read_json::<RootSpec>(path, operation::ROOT_SPEC_LOAD)?;
    if roots.schema_version != POLICY_SCHEMA_VERSION
        || roots.public_api_crate.is_empty()
        || roots.capability_roots.is_empty()
    {
        return Err(PolicyError::new(
            operation::ROOT_SPEC_LOAD,
            error_label::SCHEMA_MISMATCH,
        ));
    }
    validate_identity_specs(
        roots.capability_roots.iter().chain(&roots.claim_roots),
        operation::ROOT_SPEC_LOAD,
    )?;
    Ok(roots)
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path, operation: &'static str) -> Result<T> {
    let bytes = read_regular_file(path, operation)?;
    serde_json::from_slice(&bytes)
        .map_err(|_| PolicyError::new(operation, error_label::INVALID_JSON))
}

fn read_rustdoc_json(path: &Path) -> Result<StrictRustdocCrate> {
    let bytes = read_regular_file(path, operation::RUSTDOC_VALIDATE)?;
    let strict = deserialize_unbounded::<StrictRustdocCrate>(&bytes)?;
    let supplied = deserialize_unbounded::<serde_json::Value>(&bytes)?;
    let canonical = serde_json::to_value(&strict)
        .map_err(|_| PolicyError::new(operation::RUSTDOC_VALIDATE, error_label::SCHEMA_MISMATCH))?;
    if supplied != canonical {
        return Err(PolicyError::new(
            operation::RUSTDOC_VALIDATE,
            error_label::SCHEMA_MISMATCH,
        ));
    }
    Ok(strict)
}

fn deserialize_unbounded<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    deserializer.disable_recursion_limit();
    let parsed = T::deserialize(&mut deserializer)
        .map_err(|_| PolicyError::new(operation::RUSTDOC_VALIDATE, error_label::INVALID_JSON))?;
    deserializer
        .end()
        .map_err(|_| PolicyError::new(operation::RUSTDOC_VALIDATE, error_label::INVALID_JSON))?;
    Ok(parsed)
}

fn read_regular_file(path: &Path, operation: &'static str) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| PolicyError::new(operation, error_label::INPUT_UNREADABLE))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(PolicyError::new(
            operation,
            error_label::INPUT_NOT_REGULAR_FILE,
        ));
    }
    fs::read(path).map_err(|_| PolicyError::new(operation, error_label::INPUT_UNREADABLE))
}

fn validate_metadata(metadata: &WorkspaceMetadata) -> Result<()> {
    if metadata.schema_version != POLICY_SCHEMA_VERSION {
        return Err(PolicyError::new(
            operation::METADATA_LOAD,
            error_label::SCHEMA_MISMATCH,
        ));
    }
    if metadata.nightly != PINNED_NIGHTLY {
        return Err(PolicyError::new(
            operation::METADATA_LOAD,
            error_label::TOOLCHAIN_MISMATCH,
        ));
    }
    if metadata.rustdoc_format_version != RUSTDOC_FORMAT_VERSION
        || rustdoc_types::FORMAT_VERSION != RUSTDOC_FORMAT_VERSION
    {
        return Err(PolicyError::new(
            operation::METADATA_LOAD,
            error_label::FORMAT_MISMATCH,
        ));
    }
    if metadata.target_triple.is_empty() {
        return Err(PolicyError::new(
            operation::METADATA_LOAD,
            error_label::CENSUS_MISMATCH,
        ));
    }
    if metadata.crates.is_empty() {
        return Err(PolicyError::new(
            operation::METADATA_LOAD,
            error_label::CENSUS_MISMATCH,
        ));
    }
    let mut crate_names = BTreeSet::new();
    let mut public_filenames = BTreeSet::new();
    let mut private_filenames = BTreeSet::new();
    for entry in &metadata.crates {
        if entry.crate_name.is_empty()
            || !plain_json_filename(&entry.public_json_file)
            || !plain_json_filename(&entry.private_json_file)
            || !crate_names.insert(entry.crate_name.clone())
            || !public_filenames.insert(entry.public_json_file.clone())
            || !private_filenames.insert(entry.private_json_file.clone())
        {
            return Err(PolicyError::new(
                operation::METADATA_LOAD,
                error_label::CENSUS_MISMATCH,
            ));
        }
    }
    Ok(())
}

fn plain_json_filename(filename: &str) -> bool {
    let path = Path::new(filename);
    path.extension()
        .is_some_and(|extension| extension == "json")
        && path.components().count() == 1
        && matches!(path.components().next(), Some(Component::Normal(_)))
}

fn validate_identity_specs<'a>(
    identities: impl Iterator<Item = &'a Identity>,
    operation: &'static str,
) -> Result<()> {
    let mut seen = BTreeSet::new();
    for identity in identities {
        if identity.origin_crate.is_empty()
            || identity.path.is_empty()
            || identity.path.iter().any(String::is_empty)
            || identity.path.first() != Some(&identity.origin_crate)
            || !seen.insert(identity.clone())
        {
            return Err(PolicyError::new(operation, error_label::ROOT_MISMATCH));
        }
    }
    Ok(())
}

fn load_workspace_with_metadata(
    public_json_dir: &Path,
    private_json_dir: &Path,
    metadata: WorkspaceMetadata,
) -> Result<Workspace> {
    validate_json_directory(
        public_json_dir,
        metadata
            .crates
            .iter()
            .map(|entry| entry.public_json_file.as_str()),
    )?;
    validate_json_directory(
        private_json_dir,
        metadata
            .crates
            .iter()
            .map(|entry| entry.private_json_file.as_str()),
    )?;

    let mut crates = BTreeMap::new();
    for crate_metadata in metadata.crates {
        let public_json_path = public_json_dir.join(&crate_metadata.public_json_file);
        let private_json_path = private_json_dir.join(&crate_metadata.private_json_file);
        let public = validate_crate_blob(
            &crate_metadata,
            read_rustdoc_json(&public_json_path)?.into(),
            BlobKind::Public,
            &metadata.target_triple,
        )?;
        let private = validate_crate_blob(
            &crate_metadata,
            read_rustdoc_json(&private_json_path)?.into(),
            BlobKind::Private,
            &metadata.target_triple,
        )?;
        // Public and private rustdoc modes may expose different compiler-only
        // helper paths. Public items are mapped to private identities lazily by
        // exact composite identity during policy analysis.

        let loaded = LoadedCrate {
            metadata: crate_metadata,
            public_json_path,
            public_krate: public.krate,
            public_local_crate_id: public.local_crate_id,
            public_identities: public.identities,
            public_identity_by_id: public.identity_by_id,
            public_owner_by_id: public.owner_by_id,
            private_krate: private.krate,
            private_local_crate_id: private.local_crate_id,
            private_identities: private.identities,
            private_identity_by_id: private.identity_by_id,
            private_owner_by_id: private.owner_by_id,
        };
        if crates
            .insert(loaded.metadata.crate_name.clone(), loaded)
            .is_some()
        {
            return Err(PolicyError::new(
                operation::RUSTDOC_VALIDATE,
                error_label::CENSUS_MISMATCH,
            ));
        }
    }

    let _ = collect_workspace_identities(&crates, BlobKind::Public)?;
    let all_private_identities = collect_workspace_identities(&crates, BlobKind::Private)?;
    let workspace = Workspace {
        crates,
        all_private_identities,
    };
    Ok(workspace)
}

fn validate_json_directory<'a>(
    json_dir: &Path,
    expected_files: impl Iterator<Item = &'a str>,
) -> Result<()> {
    let directory_metadata = fs::symlink_metadata(json_dir).map_err(|_| {
        PolicyError::new(
            operation::JSON_DIRECTORY_LOAD,
            error_label::INPUT_UNREADABLE,
        )
    })?;
    if !directory_metadata.file_type().is_dir() || directory_metadata.file_type().is_symlink() {
        return Err(PolicyError::new(
            operation::JSON_DIRECTORY_LOAD,
            error_label::INPUT_NOT_DIRECTORY,
        ));
    }

    let expected_files = expected_files.map(str::to_owned).collect::<BTreeSet<_>>();
    let mut actual_files = BTreeSet::new();
    let entries = fs::read_dir(json_dir).map_err(|_| {
        PolicyError::new(
            operation::JSON_DIRECTORY_LOAD,
            error_label::INPUT_UNREADABLE,
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|_| {
            PolicyError::new(
                operation::JSON_DIRECTORY_LOAD,
                error_label::INPUT_UNREADABLE,
            )
        })?;
        let name = entry.file_name().into_string().map_err(|_| {
            PolicyError::new(
                operation::JSON_DIRECTORY_LOAD,
                error_label::FILE_SET_MISMATCH,
            )
        })?;
        let file_type = entry.file_type().map_err(|_| {
            PolicyError::new(
                operation::JSON_DIRECTORY_LOAD,
                error_label::INPUT_UNREADABLE,
            )
        })?;
        if !file_type.is_file() || file_type.is_symlink() || !plain_json_filename(&name) {
            return Err(PolicyError::new(
                operation::JSON_DIRECTORY_LOAD,
                error_label::FILE_SET_MISMATCH,
            ));
        }
        actual_files.insert(name);
    }
    if actual_files != expected_files {
        return Err(PolicyError::new(
            operation::JSON_DIRECTORY_LOAD,
            error_label::FILE_SET_MISMATCH,
        ));
    }
    Ok(())
}

fn collect_workspace_identities(
    crates: &BTreeMap<String, LoadedCrate>,
    kind: BlobKind,
) -> Result<BTreeMap<Identity, (String, Id)>> {
    let mut all_identities = BTreeMap::new();
    for (crate_name, loaded) in crates {
        let identities = match kind {
            BlobKind::Public => &loaded.public_identities,
            BlobKind::Private => &loaded.private_identities,
        };
        for (identity, id) in identities {
            if all_identities
                .insert(identity.clone(), (crate_name.clone(), *id))
                .is_some()
            {
                return Err(PolicyError::new(
                    operation::IDENTITY_RESOLVE,
                    error_label::DUPLICATE_IDENTITY,
                ));
            }
        }
    }
    Ok(all_identities)
}

fn validate_crate_blob(
    metadata: &CrateMetadata,
    krate: RustdocCrate,
    kind: BlobKind,
    target_triple: &str,
) -> Result<ValidatedBlob> {
    if krate.format_version != RUSTDOC_FORMAT_VERSION {
        return Err(PolicyError::new(
            operation::RUSTDOC_VALIDATE,
            error_label::FORMAT_MISMATCH,
        ));
    }
    if krate.target.triple != target_triple {
        return Err(PolicyError::new(
            operation::RUSTDOC_VALIDATE,
            error_label::IDENTITY_MISMATCH,
        ));
    }
    match kind {
        BlobKind::Public if krate.includes_private => {
            return Err(PolicyError::new(
                operation::RUSTDOC_VALIDATE,
                error_label::PRIVATE_ITEMS_REQUIRED,
            ));
        }
        BlobKind::Private if !krate.includes_private => {
            return Err(PolicyError::new(
                operation::RUSTDOC_VALIDATE,
                error_label::PRIVATE_ITEMS_REQUIRED,
            ));
        }
        BlobKind::Public | BlobKind::Private => {}
    }
    if kind == BlobKind::Private {
        // The private contract requires both `--document-private-items` (proved by
        // `includes_private`) and `--document-hidden-items` (proved by this exact
        // census whenever the crate has hidden items). A legitimate zero remains valid.
        let hidden_items = krate
            .index
            .values()
            .flat_map(|item| &item.attrs)
            .filter(|attribute| is_doc_hidden(attribute))
            .count();
        if hidden_items != metadata.private_hidden_items {
            return Err(PolicyError::new(
                operation::RUSTDOC_VALIDATE,
                error_label::HIDDEN_ITEMS_REQUIRED,
            ));
        }
    }
    let actual_census = CrateCensus {
        index_items: krate.index.len(),
        path_items: krate.paths.len(),
        external_crates: krate.external_crates.len(),
    };
    let expected_census = match kind {
        BlobKind::Public => metadata.public_census,
        BlobKind::Private => metadata.private_census,
    };
    if actual_census != expected_census {
        return Err(PolicyError::new(
            operation::RUSTDOC_VALIDATE,
            error_label::CENSUS_MISMATCH,
        ));
    }
    let root_item = krate.index.get(&krate.root).ok_or_else(|| {
        PolicyError::new(operation::RUSTDOC_VALIDATE, error_label::STRUCTURE_MISMATCH)
    })?;
    if root_item.id != krate.root
        || !matches!(&root_item.inner, ItemEnum::Module(module) if module.is_crate)
    {
        return Err(PolicyError::new(
            operation::RUSTDOC_VALIDATE,
            error_label::STRUCTURE_MISMATCH,
        ));
    }
    let root_summary = krate.paths.get(&krate.root).ok_or_else(|| {
        PolicyError::new(
            operation::RUSTDOC_VALIDATE,
            error_label::UNRESOLVED_IDENTITY,
        )
    })?;
    if root_summary.kind != ItemKind::Module
        || root_summary.path.as_slice() != [metadata.crate_name.as_str()]
        || root_summary.crate_id != root_item.crate_id
    {
        return Err(PolicyError::new(
            operation::RUSTDOC_VALIDATE,
            error_label::IDENTITY_MISMATCH,
        ));
    }
    for (id, item) in &krate.index {
        if *id != item.id {
            return Err(PolicyError::new(
                operation::RUSTDOC_VALIDATE,
                error_label::STRUCTURE_MISMATCH,
            ));
        }
    }

    let local_crate_id = root_item.crate_id;
    let mut identities = BTreeMap::new();
    let mut identity_by_id = BTreeMap::new();
    for (id, summary) in &krate.paths {
        let identity =
            resolve_summary_identity(&metadata.crate_name, local_crate_id, &krate, summary)?;
        if summary.crate_id == local_crate_id {
            let item = krate.index.get(id).ok_or_else(|| {
                PolicyError::new(
                    operation::IDENTITY_RESOLVE,
                    error_label::UNRESOLVED_IDENTITY,
                )
            })?;
            if IdentityKind::from(item.inner.item_kind()) != identity.kind {
                return Err(PolicyError::new(
                    operation::IDENTITY_RESOLVE,
                    error_label::IDENTITY_MISMATCH,
                ));
            }
            if identities.insert(identity.clone(), *id).is_some()
                || identity_by_id.insert(*id, identity).is_some()
            {
                return Err(PolicyError::new(
                    operation::IDENTITY_RESOLVE,
                    error_label::DUPLICATE_IDENTITY,
                ));
            }
        }
    }

    let owner_by_id = build_owner_map(&krate)?;

    Ok(ValidatedBlob {
        krate,
        local_crate_id,
        identities,
        identity_by_id,
        owner_by_id,
    })
}

fn resolve_summary_identity(
    local_name: &str,
    local_crate_id: u32,
    krate: &RustdocCrate,
    summary: &ItemSummary,
) -> Result<Identity> {
    let origin_crate = if summary.crate_id == local_crate_id {
        local_name.to_owned()
    } else {
        krate
            .external_crates
            .get(&summary.crate_id)
            .map(|external| external.name.clone())
            .ok_or_else(|| {
                PolicyError::new(
                    operation::IDENTITY_RESOLVE,
                    error_label::UNRESOLVED_IDENTITY,
                )
            })?
    };
    if summary.path.is_empty() || summary.path.iter().any(String::is_empty) {
        return Err(PolicyError::new(
            operation::IDENTITY_RESOLVE,
            error_label::IDENTITY_MISMATCH,
        ));
    }
    // Rustdoc can assign the standard-library crate_id to variants of a local
    // `Result` alias while retaining the local definition path. Identity is
    // therefore anchored by the exact path's crate component; crate_id remains
    // a resolvability check, not the identity name.
    let path_origin = summary.path.first().cloned().ok_or_else(|| {
        PolicyError::new(operation::IDENTITY_RESOLVE, error_label::IDENTITY_MISMATCH)
    })?;
    let origin_crate = if path_origin == local_name || path_origin == origin_crate {
        path_origin
    } else {
        return Err(PolicyError::new(
            operation::IDENTITY_RESOLVE,
            error_label::IDENTITY_MISMATCH,
        ));
    };
    Ok(Identity {
        origin_crate,
        path: summary.path.clone(),
        kind: summary.kind.into(),
    })
}

fn build_owner_map(krate: &RustdocCrate) -> Result<BTreeMap<Id, Id>> {
    let mut owners = BTreeMap::new();
    for (owner_id, item) in &krate.index {
        for child in structural_children(&item.inner) {
            let Some(child_item) = krate.index.get(&child) else {
                if matches!(item.inner, ItemEnum::Module(_)) && krate.paths.contains_key(&child) {
                    continue;
                }
                return Err(PolicyError::new(
                    operation::RUSTDOC_VALIDATE,
                    error_label::STRUCTURE_MISMATCH,
                ));
            };
            // Rustdoc attaches some external trait items to local impl blocks.
            // They are reference edges, not locally owned children, and must
            // never participate in the local owner map.
            if child_item.crate_id != item.crate_id {
                if matches!(item.inner, ItemEnum::Impl(_))
                    || (matches!(item.inner, ItemEnum::Module(_))
                        && krate.paths.contains_key(&child))
                {
                    continue;
                }
                return Err(PolicyError::new(
                    operation::RUSTDOC_VALIDATE,
                    error_label::STRUCTURE_MISMATCH,
                ));
            }
            // Rustdoc lists one impl under every participating type (for
            // example both `From<T>`'s T and Self). An impl therefore has no
            // unique structural owner; its `for_` type is handled explicitly
            // by the impl inventory.
            if matches!(child_item.inner, ItemEnum::Impl(_)) {
                continue;
            }
            if let Some(previous) = owners.insert(child, *owner_id)
                && previous != *owner_id
            {
                return Err(PolicyError::new(
                    operation::RUSTDOC_VALIDATE,
                    error_label::STRUCTURE_MISMATCH,
                ));
            }
        }
    }
    Ok(owners)
}

fn structural_children(inner: &ItemEnum) -> Vec<Id> {
    match inner {
        ItemEnum::Module(module) => module.items.clone(),
        ItemEnum::ExternCrate { .. } | ItemEnum::Use(_) => Vec::new(),
        ItemEnum::Union(union_) => union_.fields.iter().chain(&union_.impls).copied().collect(),
        ItemEnum::Struct(struct_) => {
            let mut children = match &struct_.kind {
                StructKind::Unit => Vec::new(),
                StructKind::Tuple(fields) => fields.iter().flatten().copied().collect(),
                StructKind::Plain { fields, .. } => fields.clone(),
            };
            children.extend(&struct_.impls);
            children
        }
        ItemEnum::StructField(_) => Vec::new(),
        ItemEnum::Enum(enum_) => enum_.variants.iter().chain(&enum_.impls).copied().collect(),
        ItemEnum::Variant(variant) => match &variant.kind {
            VariantKind::Plain => Vec::new(),
            VariantKind::Tuple(fields) => fields.iter().flatten().copied().collect(),
            VariantKind::Struct { fields, .. } => fields.clone(),
        },
        ItemEnum::Function(_) => Vec::new(),
        ItemEnum::Trait(trait_) => trait_.items.clone(),
        ItemEnum::TraitAlias(_) => Vec::new(),
        ItemEnum::Impl(impl_) => impl_.items.clone(),
        ItemEnum::TypeAlias(_)
        | ItemEnum::Constant { .. }
        | ItemEnum::Static(_)
        | ItemEnum::ExternType
        | ItemEnum::Macro(_)
        | ItemEnum::ProcMacro(_)
        | ItemEnum::Primitive(_)
        | ItemEnum::AssocConst { .. }
        | ItemEnum::AssocType { .. } => Vec::new(),
    }
}

fn resolve_id(
    crate_name: &str,
    local_crate_id: u32,
    krate: &RustdocCrate,
    id: Id,
) -> Result<Identity> {
    let summary = krate.paths.get(&id).ok_or_else(|| {
        PolicyError::new(
            operation::IDENTITY_RESOLVE,
            error_label::UNRESOLVED_IDENTITY,
        )
    })?;
    resolve_summary_identity(crate_name, local_crate_id, krate, summary)
}

impl Workspace {
    fn crate_by_name(&self, name: &str) -> Result<&LoadedCrate> {
        self.crates
            .get(name)
            .ok_or_else(|| PolicyError::new(operation::POLICY_ANALYZE, error_label::ROOT_MISMATCH))
    }

    fn resolve_private_in(&self, loaded: &LoadedCrate, id: Id) -> Result<Identity> {
        resolve_id(
            &loaded.metadata.crate_name,
            loaded.private_local_crate_id,
            &loaded.private_krate,
            id,
        )
    }

    fn private_item_for_identity(&self, identity: &Identity) -> Result<(&LoadedCrate, &Item)> {
        let (crate_name, id) = self.all_private_identities.get(identity).ok_or_else(|| {
            PolicyError::new(operation::POLICY_ANALYZE, error_label::ROOT_MISMATCH)
        })?;
        let loaded = self.crate_by_name(crate_name)?;
        let item = loaded.private_krate.index.get(id).ok_or_else(|| {
            PolicyError::new(operation::POLICY_ANALYZE, error_label::UNRESOLVED_IDENTITY)
        })?;
        Ok((loaded, item))
    }
}

/// Analyze the complete census and produce all four deterministic snapshots.
pub fn analyze(workspace: &Workspace, roots: &RootSpec) -> Result<Snapshots> {
    if roots.schema_version != POLICY_SCHEMA_VERSION {
        return Err(PolicyError::new(
            operation::POLICY_ANALYZE,
            error_label::SCHEMA_MISMATCH,
        ));
    }
    validate_identity_specs(
        roots.capability_roots.iter().chain(&roots.claim_roots),
        operation::POLICY_ANALYZE,
    )?;
    workspace.crate_by_name(&roots.public_api_crate)?;

    let root_set = roots
        .capability_roots
        .iter()
        .chain(&roots.claim_roots)
        .cloned()
        .collect::<BTreeSet<_>>();
    for root in &root_set {
        let (_, item) = workspace.private_item_for_identity(root)?;
        if IdentityKind::from(item.inner.item_kind()) != root.kind {
            return Err(PolicyError::new(
                operation::POLICY_ANALYZE,
                error_label::ROOT_MISMATCH,
            ));
        }
    }

    let direct_references = definition_reference_graph(workspace)?;
    let capability_types = capability_fixed_point(&root_set, &direct_references);
    let workspace_public_items = canonical_workspace_public_api(workspace)?;
    let public_items = workspace_public_items
        .iter()
        .filter(|entry| entry.crate_name == roots.public_api_crate)
        .collect::<Vec<_>>();
    if public_items.is_empty() {
        return Err(PolicyError::new(
            operation::PUBLIC_API_RENDER,
            error_label::PUBLIC_API_INCOMPLETE,
        ));
    }

    let capability_api =
        capability_public_api(workspace, &workspace_public_items, &capability_types)?;
    let hidden_public_api = hidden_public_inventory(workspace)?;
    let capability_trait_impls =
        trait_impl_inventory(workspace, &workspace_public_items, &capability_types)?;

    Ok(Snapshots {
        public_api: public_items
            .iter()
            .map(|entry| entry.line.clone())
            .collect(),
        capability_api,
        hidden_public_api,
        capability_trait_impls,
    })
}

fn definition_reference_graph(
    workspace: &Workspace,
) -> Result<BTreeMap<Identity, BTreeSet<Identity>>> {
    let mut graph = BTreeMap::new();
    for loaded in workspace.crates.values() {
        for (identity, id) in &loaded.private_identities {
            if !is_type_identity_kind(identity.kind) {
                continue;
            }
            let mut pending = VecDeque::from([*id]);
            let mut visited = BTreeSet::new();
            let mut references = BTreeSet::new();
            while let Some(current) = pending.pop_front() {
                if !visited.insert(current) {
                    continue;
                }
                let item = loaded.private_krate.index.get(&current).ok_or_else(|| {
                    PolicyError::new(operation::POLICY_ANALYZE, error_label::STRUCTURE_MISMATCH)
                })?;
                let mut ids = BTreeSet::new();
                visit_item_signatures(&item.inner, &mut ids);
                for reference in ids {
                    match workspace.resolve_private_in(loaded, reference) {
                        Ok(identity) => {
                            references.insert(identity);
                        }
                        Err(_error) if !loaded.private_krate.index.contains_key(&reference) => {}
                        Err(error) => return Err(error),
                    }
                }
                for child in definition_children(&item.inner) {
                    pending.push_back(child);
                }
            }
            graph.insert(identity.clone(), references);
        }
    }
    Ok(graph)
}

fn definition_children(inner: &ItemEnum) -> Vec<Id> {
    match inner {
        ItemEnum::Union(union_) => union_.fields.clone(),
        ItemEnum::Struct(struct_) => match &struct_.kind {
            StructKind::Unit => Vec::new(),
            StructKind::Tuple(fields) => fields.iter().flatten().copied().collect(),
            StructKind::Plain { fields, .. } => fields.clone(),
        },
        ItemEnum::Enum(enum_) => enum_.variants.clone(),
        ItemEnum::Variant(variant) => match &variant.kind {
            VariantKind::Plain => Vec::new(),
            VariantKind::Tuple(fields) => fields.iter().flatten().copied().collect(),
            VariantKind::Struct { fields, .. } => fields.clone(),
        },
        ItemEnum::Trait(trait_) => trait_.items.clone(),
        ItemEnum::Module(_)
        | ItemEnum::ExternCrate { .. }
        | ItemEnum::Use(_)
        | ItemEnum::StructField(_)
        | ItemEnum::Function(_)
        | ItemEnum::TraitAlias(_)
        | ItemEnum::Impl(_)
        | ItemEnum::TypeAlias(_)
        | ItemEnum::Constant { .. }
        | ItemEnum::Static(_)
        | ItemEnum::ExternType
        | ItemEnum::Macro(_)
        | ItemEnum::ProcMacro(_)
        | ItemEnum::Primitive(_)
        | ItemEnum::AssocConst { .. }
        | ItemEnum::AssocType { .. } => Vec::new(),
    }
}

fn capability_fixed_point(
    roots: &BTreeSet<Identity>,
    graph: &BTreeMap<Identity, BTreeSet<Identity>>,
) -> BTreeSet<Identity> {
    let mut capabilities = roots.clone();
    loop {
        let additions = graph
            .iter()
            .filter(|(identity, references)| {
                !capabilities.contains(*identity)
                    && references
                        .iter()
                        .any(|reference| capabilities.contains(reference))
            })
            .map(|(identity, _)| identity.clone())
            .collect::<Vec<_>>();
        if additions.is_empty() {
            return capabilities;
        }
        capabilities.extend(additions);
    }
}

#[derive(Debug)]
struct CanonicalPublicItem {
    crate_name: String,
    /// ID in the public blob only. It must never index the private blob.
    public_id: Id,
    /// Parent ID in the public blob only.
    public_parent_id: Option<Id>,
    /// Composite identity mapped independently into the private closure when rustdoc
    /// publishes one for this item. Items such as impl blocks use a dedicated composite.
    private_identity: Option<Identity>,
    line: String,
}

fn canonical_workspace_public_api(workspace: &Workspace) -> Result<Vec<CanonicalPublicItem>> {
    let mut workspace_items = Vec::new();
    for (crate_name, loaded) in &workspace.crates {
        let api = public_api::Builder::from_rustdoc_json(&loaded.public_json_path)
            .sorted(true)
            .build()
            .map_err(|_| {
                PolicyError::new(operation::PUBLIC_API_RENDER, error_label::INVALID_JSON)
            })?;
        // Missing IDs are expected for references into dependencies because
        // each crate is emitted as a standalone blob. Every workspace-local
        // identity is validated and mapped independently across the census.
        let mut items = api
            .items()
            .map(|item| {
                let public_id = item.id();
                Ok(CanonicalPublicItem {
                    crate_name: crate_name.clone(),
                    public_id,
                    public_parent_id: item.parent_id(),
                    private_identity: public_item_private_identity(
                        workspace,
                        loaded,
                        public_id,
                        item.parent_id(),
                    )?,
                    line: item.to_string(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        if items
            .iter()
            .any(|entry| entry.line.is_empty() || entry.line.contains(['\n', '\r']))
        {
            return Err(PolicyError::new(
                operation::PUBLIC_API_RENDER,
                error_label::STRUCTURE_MISMATCH,
            ));
        }
        workspace_items.append(&mut items);
    }
    workspace_items.sort_by(|left, right| {
        left.crate_name
            .cmp(&right.crate_name)
            .then_with(|| left.line.cmp(&right.line))
            .then_with(|| left.private_identity.cmp(&right.private_identity))
            .then_with(|| left.public_id.cmp(&right.public_id))
            .then_with(|| left.public_parent_id.cmp(&right.public_parent_id))
    });
    workspace_items
        .dedup_by(|left, right| left.crate_name == right.crate_name && left.line == right.line);
    Ok(workspace_items)
}

fn public_item_private_identity(
    _workspace: &Workspace,
    loaded: &LoadedCrate,
    public_id: Id,
    public_parent_id: Option<Id>,
) -> Result<Option<Identity>> {
    if let Some(identity) = loaded.public_identity_by_id.get(&public_id) {
        if !loaded.private_identities.contains_key(identity) {
            return Err(PolicyError::new(
                operation::IDENTITY_RESOLVE,
                error_label::IDENTITY_MISMATCH,
            ));
        }
        return Ok(Some(identity.clone()));
    }
    let public_item = loaded.public_krate.index.get(&public_id).ok_or_else(|| {
        PolicyError::new(
            operation::POLICY_ANALYZE,
            error_label::PUBLIC_API_INCOMPLETE,
        )
    })?;
    if !matches!(public_item.inner, ItemEnum::Impl(_)) {
        return Ok(None);
    }
    explicit_impl_identity(
        &loaded.metadata.crate_name,
        loaded.public_local_crate_id,
        &loaded.public_krate,
        public_item,
        public_parent_id,
    )
    .map(Some)
}

fn capability_public_api(
    workspace: &Workspace,
    public_items: &[CanonicalPublicItem],
    capability_types: &BTreeSet<Identity>,
) -> Result<Vec<String>> {
    let mut lines = Vec::new();
    for public in public_items {
        let loaded = workspace.crate_by_name(&public.crate_name)?;
        let item = loaded
            .public_krate
            .index
            .get(&public.public_id)
            .ok_or_else(|| {
                PolicyError::new(
                    operation::POLICY_ANALYZE,
                    error_label::PUBLIC_API_INCOMPLETE,
                )
            })?;
        let mut public_references = BTreeSet::new();
        visit_item_signatures(&item.inner, &mut public_references);
        let mut signature_capability = false;
        for id in public_references {
            match resolve_id(
                &loaded.metadata.crate_name,
                loaded.public_local_crate_id,
                &loaded.public_krate,
                id,
            ) {
                Ok(identity) => signature_capability |= capability_types.contains(&identity),
                Err(_error) if !loaded.public_krate.index.contains_key(&id) => {}
                Err(error) => return Err(error),
            }
        }
        let own_capability = public_owner_identity(loaded, public)
            .is_some_and(|identity| capability_types.contains(&identity));
        if signature_capability || own_capability {
            lines.push(format!("{}\t{}", public.crate_name, public.line));
        }
    }
    lines.sort();
    lines.dedup();
    Ok(lines)
}

fn public_owner_identity(loaded: &LoadedCrate, public: &CanonicalPublicItem) -> Option<Identity> {
    let mut current = Some(public.public_id);
    let mut visited = BTreeSet::new();
    while let Some(public_id) = current {
        if !visited.insert(public_id) {
            return None;
        }
        if let Some(identity) = loaded.public_identity_by_id.get(&public_id)
            && is_type_identity_kind(identity.kind)
        {
            return loaded
                .private_identities
                .contains_key(identity)
                .then(|| identity.clone());
        }
        current = loaded
            .public_owner_by_id
            .get(&public_id)
            .copied()
            .or_else(|| {
                (public_id == public.public_id)
                    .then_some(public.public_parent_id)
                    .flatten()
            });
    }
    None
}

fn hidden_public_inventory(workspace: &Workspace) -> Result<Vec<String>> {
    let mut entries = BTreeSet::new();
    for loaded in workspace.crates.values() {
        for item in loaded.private_krate.index.values() {
            if !item.attrs.iter().any(is_doc_hidden) {
                continue;
            }
            let public_hidden = matches!(item.visibility, Visibility::Public)
                || loaded
                    .private_owner_by_id
                    .get(&item.id)
                    .and_then(|owner| loaded.private_krate.index.get(owner))
                    .is_some_and(|owner| {
                        matches!(owner.inner, ItemEnum::Trait(_))
                            && matches!(owner.visibility, Visibility::Public)
                    });
            if !public_hidden {
                continue;
            }
            let identity =
                private_identity_for_item_or_owner(loaded, item.id).unwrap_or_else(|_| Identity {
                    origin_crate: loaded.metadata.crate_name.clone(),
                    path: vec![
                        loaded.metadata.crate_name.clone(),
                        item.name.clone().unwrap_or_else(|| "<unnamed>".to_owned()),
                    ],
                    kind: item.inner.item_kind().into(),
                });
            let row = format!(
                "{}\t{}",
                identity.snapshot_key(),
                kind_label(item.inner.item_kind().into())
            );
            entries.insert(row);
        }
    }
    Ok(entries.into_iter().collect())
}

fn is_doc_hidden(attribute: &rustdoc_types::Attribute) -> bool {
    matches!(attribute, rustdoc_types::Attribute::Other(value) if value == "#[doc(hidden)]")
}

fn private_identity_for_item_or_owner(loaded: &LoadedCrate, id: Id) -> Result<Identity> {
    if let Some(identity) = loaded.private_identity_by_id.get(&id) {
        return Ok(identity.clone());
    }
    let mut current = loaded.private_owner_by_id.get(&id).copied();
    let mut visited = BTreeSet::new();
    while let Some(owner) = current {
        if !visited.insert(owner) {
            break;
        }
        if let Some(identity) = loaded.private_identity_by_id.get(&owner) {
            let item = loaded.private_krate.index.get(&id).ok_or_else(|| {
                PolicyError::new(operation::POLICY_ANALYZE, error_label::UNRESOLVED_IDENTITY)
            })?;
            let mut path = identity.path.clone();
            path.push(item.name.clone().unwrap_or_else(|| "<unnamed>".to_owned()));
            return Ok(Identity {
                origin_crate: identity.origin_crate.clone(),
                path,
                kind: item.inner.item_kind().into(),
            });
        }
        current = loaded.private_owner_by_id.get(&owner).copied();
    }
    Err(PolicyError::new(
        operation::POLICY_ANALYZE,
        error_label::UNRESOLVED_IDENTITY,
    ))
}

fn trait_impl_inventory(
    workspace: &Workspace,
    public_items: &[CanonicalPublicItem],
    capability_types: &BTreeSet<Identity>,
) -> Result<Vec<String>> {
    // Public lines are joined to private impls only through composite identity.
    // Numeric IDs are scoped to one blob and are never compared across blobs.
    let public_impl_lines = public_items
        .iter()
        .filter_map(|item| {
            item.private_identity
                .as_ref()
                .filter(|identity| identity.kind == IdentityKind::Impl)
                .map(|identity| (identity.clone(), item.line.as_str()))
        })
        .collect::<BTreeMap<_, _>>();
    let mut entries = BTreeSet::new();
    for loaded in workspace.crates.values() {
        for item in loaded.private_krate.index.values() {
            let ItemEnum::Impl(impl_) = &item.inner else {
                continue;
            };
            if impl_.is_synthetic || impl_.blanket_impl.is_some() {
                continue;
            }
            let Some(trait_path) = &impl_.trait_ else {
                continue;
            };
            let Some(self_id) = direct_resolved_path_id(&impl_.for_) else {
                continue;
            };
            let self_identity = match workspace.resolve_private_in(loaded, self_id) {
                Ok(identity) => identity,
                Err(_error) if !loaded.private_krate.index.contains_key(&self_id) => continue,
                Err(error) => return Err(error),
            };
            if !capability_types.contains(&self_identity) {
                continue;
            }
            let trait_identity = match workspace.resolve_private_in(loaded, trait_path.id) {
                Ok(identity) => identity,
                Err(_error) if !loaded.private_krate.index.contains_key(&trait_path.id) => continue,
                Err(error) => return Err(error),
            };
            let impl_identity = explicit_impl_identity(
                &loaded.metadata.crate_name,
                loaded.private_local_crate_id,
                &loaded.private_krate,
                item,
                loaded.private_owner_by_id.get(&item.id).copied(),
            )?;
            let public_impl_identity = private_impl_public_identity(loaded, item, impl_identity)?;
            let canonical_line = public_impl_lines
                .get(&public_impl_identity)
                .copied()
                .map(str::to_owned)
                .unwrap_or_else(|| canonical_explicit_impl(&self_identity, &trait_identity, impl_));
            let entry = format!(
                "{}\t{}\t{}",
                self_identity.snapshot_key(),
                trait_identity.snapshot_key(),
                canonical_line
            );
            if !entries.insert(entry) {
                return Err(PolicyError::new(
                    operation::POLICY_ANALYZE,
                    error_label::DUPLICATE_IDENTITY,
                ));
            }
        }
    }
    Ok(entries.into_iter().collect())
}

fn private_impl_public_identity(
    loaded: &LoadedCrate,
    private_item: &Item,
    private_identity: Identity,
) -> Result<Identity> {
    let ItemEnum::Impl(private_impl) = &private_item.inner else {
        return Err(PolicyError::new(
            operation::IDENTITY_RESOLVE,
            error_label::IDENTITY_MISMATCH,
        ));
    };
    let Some(private_trait) = &private_impl.trait_ else {
        return Ok(private_identity);
    };
    let private_self = direct_resolved_path_id(&private_impl.for_)
        .map(|id| {
            resolve_id(
                &loaded.metadata.crate_name,
                loaded.private_local_crate_id,
                &loaded.private_krate,
                id,
            )
        })
        .transpose()?;
    let private_trait = resolve_id(
        &loaded.metadata.crate_name,
        loaded.private_local_crate_id,
        &loaded.private_krate,
        private_trait.id,
    )?;
    for public_item in loaded.public_krate.index.values() {
        let ItemEnum::Impl(public_impl) = &public_item.inner else {
            continue;
        };
        let Some(public_trait) = &public_impl.trait_ else {
            continue;
        };
        let public_self = direct_resolved_path_id(&public_impl.for_)
            .map(|id| {
                resolve_id(
                    &loaded.metadata.crate_name,
                    loaded.public_local_crate_id,
                    &loaded.public_krate,
                    id,
                )
            })
            .transpose()?;
        let public_trait = resolve_id(
            &loaded.metadata.crate_name,
            loaded.public_local_crate_id,
            &loaded.public_krate,
            public_trait.id,
        )?;
        if public_self == private_self
            && public_trait == private_trait
            && public_impl.is_negative == private_impl.is_negative
            && public_impl.generics == private_impl.generics
        {
            return explicit_impl_identity(
                &loaded.metadata.crate_name,
                loaded.public_local_crate_id,
                &loaded.public_krate,
                public_item,
                loaded.public_owner_by_id.get(&public_item.id).copied(),
            );
        }
    }
    Ok(private_identity)
}

fn explicit_impl_identity(
    crate_name: &str,
    local_crate_id: u32,
    krate: &RustdocCrate,
    item: &Item,
    owner_id: Option<Id>,
) -> Result<Identity> {
    let ItemEnum::Impl(impl_) = &item.inner else {
        return Err(PolicyError::new(
            operation::IDENTITY_RESOLVE,
            error_label::IDENTITY_MISMATCH,
        ));
    };
    let owner = owner_id
        .map(|id| resolve_id(crate_name, local_crate_id, krate, id))
        .transpose()?;
    let self_identity = direct_resolved_path_id(&impl_.for_)
        .map(|id| resolve_id(crate_name, local_crate_id, krate, id))
        .transpose()?;
    let trait_identity = impl_
        .trait_
        .as_ref()
        .map(|path| resolve_id(crate_name, local_crate_id, krate, path.id))
        .transpose()?;
    let anchor = owner.or(self_identity).ok_or_else(|| {
        PolicyError::new(
            operation::IDENTITY_RESOLVE,
            error_label::UNRESOLVED_IDENTITY,
        )
    })?;
    let mut path = anchor.path;
    path.push(format!(
        "<impl:{}:{}:{}:{}>",
        trait_identity
            .as_ref()
            .map_or("inherent".to_owned(), |identity| identity.snapshot_key()),
        impl_.is_negative,
        impl_.generics.params.len(),
        impl_.generics.where_predicates.len()
    ));
    Ok(Identity {
        origin_crate: anchor.origin_crate,
        path,
        kind: IdentityKind::Impl,
    })
}

fn canonical_explicit_impl(
    self_identity: &Identity,
    trait_identity: &Identity,
    impl_: &rustdoc_types::Impl,
) -> String {
    format!(
        "private explicit impl {} for {} negative={} generic-params={} where-predicates={}",
        trait_identity.path.join("::"),
        self_identity.path.join("::"),
        impl_.is_negative,
        impl_.generics.params.len(),
        impl_.generics.where_predicates.len()
    )
}

fn direct_resolved_path_id(type_: &Type) -> Option<Id> {
    match type_ {
        Type::ResolvedPath(path) => Some(path.id),
        Type::DynTrait(_)
        | Type::Generic(_)
        | Type::Primitive(_)
        | Type::FunctionPointer(_)
        | Type::Tuple(_)
        | Type::Slice(_)
        | Type::Array { .. }
        | Type::Pat { .. }
        | Type::ImplTrait(_)
        | Type::Infer
        | Type::RawPointer { .. }
        | Type::BorrowedRef { .. }
        | Type::QualifiedPath { .. } => None,
    }
}

fn is_type_identity_kind(kind: IdentityKind) -> bool {
    matches!(
        kind,
        IdentityKind::Struct
            | IdentityKind::Union
            | IdentityKind::Enum
            | IdentityKind::Trait
            | IdentityKind::TraitAlias
            | IdentityKind::TypeAlias
            | IdentityKind::ExternType
            | IdentityKind::Primitive
    )
}

fn kind_label(kind: IdentityKind) -> &'static str {
    match kind {
        IdentityKind::Module => "module",
        IdentityKind::ExternCrate => "extern_crate",
        IdentityKind::Use => "use",
        IdentityKind::Struct => "struct",
        IdentityKind::StructField => "struct_field",
        IdentityKind::Union => "union",
        IdentityKind::Enum => "enum",
        IdentityKind::Variant => "variant",
        IdentityKind::Function => "function",
        IdentityKind::TypeAlias => "type_alias",
        IdentityKind::Constant => "constant",
        IdentityKind::Trait => "trait",
        IdentityKind::TraitAlias => "trait_alias",
        IdentityKind::Impl => "impl",
        IdentityKind::Static => "static",
        IdentityKind::ExternType => "extern_type",
        IdentityKind::Macro => "macro",
        IdentityKind::ProcAttribute => "proc_attribute",
        IdentityKind::ProcDerive => "proc_derive",
        IdentityKind::AssocConst => "assoc_const",
        IdentityKind::AssocType => "assoc_type",
        IdentityKind::Primitive => "primitive",
        IdentityKind::Keyword => "keyword",
        IdentityKind::Attribute => "attribute",
    }
}

/// Exhaustive visitor for every signature-bearing `ItemEnum` variant.
fn visit_item_signatures(inner: &ItemEnum, references: &mut BTreeSet<Id>) {
    match inner {
        ItemEnum::Module(_) | ItemEnum::ExternCrate { .. } => {}
        ItemEnum::Use(use_) => {
            if let Some(id) = use_.id {
                references.insert(id);
            }
        }
        ItemEnum::Union(union_) => visit_generics(&union_.generics, references),
        ItemEnum::Struct(struct_) => visit_generics(&struct_.generics, references),
        ItemEnum::StructField(type_) => visit_type(type_, references),
        ItemEnum::Enum(enum_) => visit_generics(&enum_.generics, references),
        ItemEnum::Variant(_) => {}
        ItemEnum::Function(function) => visit_function(function, references),
        ItemEnum::Trait(trait_) => {
            visit_generics(&trait_.generics, references);
            visit_bounds(&trait_.bounds, references);
        }
        ItemEnum::TraitAlias(alias) => {
            visit_generics(&alias.generics, references);
            visit_bounds(&alias.params, references);
        }
        ItemEnum::Impl(impl_) => {
            visit_generics(&impl_.generics, references);
            if let Some(trait_) = &impl_.trait_ {
                visit_path(trait_, references);
            }
            visit_type(&impl_.for_, references);
            if let Some(blanket) = &impl_.blanket_impl {
                visit_type(blanket, references);
            }
        }
        ItemEnum::TypeAlias(alias) => {
            visit_type(&alias.type_, references);
            visit_generics(&alias.generics, references);
        }
        ItemEnum::Constant { type_, .. } => visit_type(type_, references),
        ItemEnum::Static(static_) => visit_type(&static_.type_, references),
        ItemEnum::ExternType | ItemEnum::Macro(_) | ItemEnum::ProcMacro(_) => {}
        ItemEnum::Primitive(_) => {}
        ItemEnum::AssocConst { type_, .. } => visit_type(type_, references),
        ItemEnum::AssocType {
            generics,
            bounds,
            type_,
        } => {
            visit_generics(generics, references);
            visit_bounds(bounds, references);
            if let Some(type_) = type_ {
                visit_type(type_, references);
            }
        }
    }
}

fn visit_function(function: &Function, references: &mut BTreeSet<Id>) {
    visit_function_signature(&function.sig, references);
    visit_generics(&function.generics, references);
}

fn visit_function_pointer(pointer: &FunctionPointer, references: &mut BTreeSet<Id>) {
    visit_function_signature(&pointer.sig, references);
    for parameter in &pointer.generic_params {
        visit_generic_param(parameter, references);
    }
}

fn visit_function_signature(signature: &FunctionSignature, references: &mut BTreeSet<Id>) {
    for (_, type_) in &signature.inputs {
        visit_type(type_, references);
    }
    if let Some(output) = &signature.output {
        visit_type(output, references);
    }
}

/// Exhaustive visitor for rustdoc `Type`.
fn visit_type(type_: &Type, references: &mut BTreeSet<Id>) {
    match type_ {
        Type::ResolvedPath(path) => visit_path(path, references),
        Type::DynTrait(dyn_trait) => visit_dyn_trait(dyn_trait, references),
        Type::Generic(_) | Type::Primitive(_) => {}
        Type::FunctionPointer(pointer) => visit_function_pointer(pointer, references),
        Type::Tuple(types) => {
            for type_ in types {
                visit_type(type_, references);
            }
        }
        Type::Slice(type_)
        | Type::Array { type_, .. }
        | Type::Pat { type_, .. }
        | Type::RawPointer { type_, .. }
        | Type::BorrowedRef { type_, .. } => visit_type(type_, references),
        Type::ImplTrait(bounds) => visit_bounds(bounds, references),
        Type::Infer => {}
        Type::QualifiedPath {
            args,
            self_type,
            trait_,
            ..
        } => {
            if let Some(args) = args {
                visit_generic_args(args, references);
            }
            visit_type(self_type, references);
            if let Some(trait_) = trait_ {
                visit_path(trait_, references);
            }
        }
    }
}

fn visit_dyn_trait(dyn_trait: &DynTrait, references: &mut BTreeSet<Id>) {
    for trait_ in &dyn_trait.traits {
        visit_poly_trait(trait_, references);
    }
}

fn visit_poly_trait(trait_: &PolyTrait, references: &mut BTreeSet<Id>) {
    visit_path(&trait_.trait_, references);
    for parameter in &trait_.generic_params {
        visit_generic_param(parameter, references);
    }
}

fn visit_path(path: &RustdocPath, references: &mut BTreeSet<Id>) {
    references.insert(path.id);
    if let Some(args) = &path.args {
        visit_generic_args(args, references);
    }
}

/// Exhaustive visitor for rustdoc `GenericArgs` and `GenericArg`.
fn visit_generic_args(args: &GenericArgs, references: &mut BTreeSet<Id>) {
    match args {
        GenericArgs::AngleBracketed { args, constraints } => {
            for argument in args {
                match argument {
                    GenericArg::Lifetime(_) | GenericArg::Const(_) | GenericArg::Infer => {}
                    GenericArg::Type(type_) => visit_type(type_, references),
                }
            }
            for constraint in constraints {
                visit_assoc_constraint(constraint, references);
            }
        }
        GenericArgs::Parenthesized { inputs, output } => {
            for input in inputs {
                visit_type(input, references);
            }
            if let Some(output) = output {
                visit_type(output, references);
            }
        }
        GenericArgs::ReturnTypeNotation => {}
    }
}

fn visit_assoc_constraint(constraint: &AssocItemConstraint, references: &mut BTreeSet<Id>) {
    if let Some(args) = &constraint.args {
        visit_generic_args(args, references);
    }
    match &constraint.binding {
        AssocItemConstraintKind::Equality(term) => visit_term(term, references),
        AssocItemConstraintKind::Constraint(bounds) => visit_bounds(bounds, references),
    }
}

fn visit_term(term: &Term, references: &mut BTreeSet<Id>) {
    match term {
        Term::Type(type_) => visit_type(type_, references),
        Term::Constant(_) => {}
    }
}

fn visit_generics(generics: &Generics, references: &mut BTreeSet<Id>) {
    for parameter in &generics.params {
        visit_generic_param(parameter, references);
    }
    for predicate in &generics.where_predicates {
        visit_where_predicate(predicate, references);
    }
}

fn visit_generic_param(parameter: &GenericParamDef, references: &mut BTreeSet<Id>) {
    match &parameter.kind {
        GenericParamDefKind::Lifetime { .. } => {}
        GenericParamDefKind::Type {
            bounds, default, ..
        } => {
            visit_bounds(bounds, references);
            if let Some(default) = default {
                visit_type(default, references);
            }
        }
        GenericParamDefKind::Const { type_, .. } => visit_type(type_, references),
    }
}

/// Exhaustive visitor for rustdoc `WherePredicate`.
fn visit_where_predicate(predicate: &WherePredicate, references: &mut BTreeSet<Id>) {
    match predicate {
        WherePredicate::BoundPredicate {
            type_,
            bounds,
            generic_params,
        } => {
            visit_type(type_, references);
            visit_bounds(bounds, references);
            for parameter in generic_params {
                visit_generic_param(parameter, references);
            }
        }
        WherePredicate::LifetimePredicate { .. } => {}
        WherePredicate::EqPredicate { lhs, rhs } => {
            visit_type(lhs, references);
            visit_term(rhs, references);
        }
    }
}

fn visit_bounds(bounds: &[GenericBound], references: &mut BTreeSet<Id>) {
    for bound in bounds {
        visit_generic_bound(bound, references);
    }
}

/// Exhaustive visitor for rustdoc `GenericBound`.
fn visit_generic_bound(bound: &GenericBound, references: &mut BTreeSet<Id>) {
    match bound {
        GenericBound::TraitBound {
            trait_,
            generic_params,
            modifier: _,
        } => {
            visit_path(trait_, references);
            for parameter in generic_params {
                visit_generic_param(parameter, references);
            }
        }
        GenericBound::Outlives(_) | GenericBound::Use(_) => {}
    }
}

/// Compare a snapshot without disclosing its contents or a raw diff.
pub fn check_snapshot(path: &Path, lines: &[String]) -> Result<()> {
    let expected = Snapshots::render(lines)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| PolicyError::new(operation::SNAPSHOT_CHECK, error_label::INPUT_UNREADABLE))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(PolicyError::new(
            operation::SNAPSHOT_CHECK,
            error_label::INPUT_NOT_REGULAR_FILE,
        ));
    }
    let actual = fs::read(path)
        .map_err(|_| PolicyError::new(operation::SNAPSHOT_CHECK, error_label::INPUT_UNREADABLE))?;
    if actual != expected.as_bytes() {
        return Err(PolicyError::new(
            operation::SNAPSHOT_CHECK,
            error_label::SNAPSHOT_MISMATCH,
        ));
    }
    Ok(())
}

/// Atomically replace a snapshot without following a destination symlink.
pub fn write_snapshot(path: &Path, lines: &[String]) -> Result<()> {
    let rendered = Snapshots::render(lines)?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.file_type().is_file())
    {
        return Err(PolicyError::new(
            operation::SNAPSHOT_WRITE,
            error_label::INPUT_NOT_REGULAR_FILE,
        ));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|_| PolicyError::new(operation::SNAPSHOT_WRITE, error_label::INPUT_UNREADABLE))?;
    if !parent_metadata.file_type().is_dir() || parent_metadata.file_type().is_symlink() {
        return Err(PolicyError::new(
            operation::SNAPSHOT_WRITE,
            error_label::INPUT_NOT_DIRECTORY,
        ));
    }
    let filename = path.file_name().ok_or_else(|| {
        PolicyError::new(operation::SNAPSHOT_WRITE, error_label::SNAPSHOT_INVALID)
    })?;
    let temporary = parent.join(format!(
        ".{}.d2b-api-surface-{}-tmp",
        filename.to_string_lossy(),
        std::process::id()
    ));
    if fs::symlink_metadata(&temporary).is_ok() {
        return Err(PolicyError::new(
            operation::SNAPSHOT_WRITE,
            error_label::SNAPSHOT_INVALID,
        ));
    }
    fs::write(&temporary, rendered)
        .map_err(|_| PolicyError::new(operation::SNAPSHOT_WRITE, error_label::INPUT_UNREADABLE))?;
    fs::rename(&temporary, path).map_err(|_| {
        let _ = fs::remove_file(&temporary);
        PolicyError::new(operation::SNAPSHOT_WRITE, error_label::INPUT_UNREADABLE)
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_error_does_not_render_sensitive_input() {
        let error = PolicyError::new(operation::RUSTDOC_VALIDATE, error_label::INVALID_JSON);
        assert_eq!(error.to_string(), "rustdoc-validate: invalid-json");
        assert!(!error.to_string().contains("/home/"));
    }

    #[test]
    fn metadata_rejects_wrong_toolchain_and_duplicate_files() {
        let mut metadata = WorkspaceMetadata {
            schema_version: POLICY_SCHEMA_VERSION,
            nightly: "nightly-attacker-value".to_owned(),
            rustdoc_format_version: RUSTDOC_FORMAT_VERSION,
            target_triple: "x86_64-unknown-linux-gnu".to_owned(),
            crates: vec![crate_metadata("a", "a.json")],
        };
        assert_eq!(
            validate_metadata(&metadata).unwrap_err().label(),
            error_label::TOOLCHAIN_MISMATCH
        );
        metadata.nightly = PINNED_NIGHTLY.to_owned();
        metadata.crates.push(crate_metadata("b", "a.json"));
        assert_eq!(
            validate_metadata(&metadata).unwrap_err().label(),
            error_label::CENSUS_MISMATCH
        );
    }

    #[test]
    fn metadata_rejects_traversal_and_non_json_names() {
        assert!(!plain_json_filename("../a.json"));
        assert!(!plain_json_filename("nested/a.json"));
        assert!(!plain_json_filename("a.txt"));
        assert!(plain_json_filename("d2b_bus.json"));
    }

    #[test]
    fn fixed_point_reaches_private_wrapper_chain() {
        let root = identity("demo", &["demo", "Cap"], IdentityKind::Struct);
        let private = identity("demo", &["demo", "Private"], IdentityKind::Struct);
        let public = identity("demo", &["demo", "Public"], IdentityKind::Struct);
        let mut graph = BTreeMap::new();
        graph.insert(private.clone(), BTreeSet::from([root.clone()]));
        graph.insert(public.clone(), BTreeSet::from([private.clone()]));
        let closure = capability_fixed_point(&BTreeSet::from([root.clone()]), &graph);
        assert_eq!(closure, BTreeSet::from([root, private, public]));
    }

    #[test]
    fn visitor_reaches_nested_generic_constraint_and_function_pointer() {
        let root = Id(9);
        let trait_id = Id(10);
        let type_ = Type::ResolvedPath(RustdocPath {
            path: "Outer".to_owned(),
            id: Id(1),
            args: Some(Box::new(GenericArgs::AngleBracketed {
                args: vec![GenericArg::Type(Type::FunctionPointer(Box::new(
                    FunctionPointer {
                        sig: FunctionSignature {
                            inputs: vec![(
                                "value".to_owned(),
                                Type::ResolvedPath(RustdocPath {
                                    path: "Cap".to_owned(),
                                    id: root,
                                    args: None,
                                }),
                            )],
                            output: None,
                            is_c_variadic: false,
                        },
                        generic_params: Vec::new(),
                        header: rustdoc_types::FunctionHeader {
                            is_const: false,
                            is_unsafe: false,
                            is_async: false,
                            abi: rustdoc_types::Abi::Rust,
                        },
                    },
                )))],
                constraints: vec![AssocItemConstraint {
                    name: "Item".to_owned(),
                    args: None,
                    binding: AssocItemConstraintKind::Constraint(vec![GenericBound::TraitBound {
                        trait_: RustdocPath {
                            path: "Trait".to_owned(),
                            id: trait_id,
                            args: None,
                        },
                        generic_params: Vec::new(),
                        modifier: rustdoc_types::TraitBoundModifier::None,
                    }]),
                }],
            })),
        });
        let mut references = BTreeSet::new();
        visit_type(&type_, &mut references);
        assert_eq!(references, BTreeSet::from([Id(1), root, trait_id]));
    }

    #[test]
    fn snapshot_render_is_sorted_by_caller_and_newline_terminated() {
        let lines = vec!["a".to_owned(), "b".to_owned()];
        assert_eq!(Snapshots::render(&lines).unwrap(), "a\nb\n");
        assert_eq!(Snapshots::render(&[]).unwrap(), "");
        assert_eq!(
            Snapshots::render(&["bad\nline".to_owned()])
                .unwrap_err()
                .label(),
            error_label::STRUCTURE_MISMATCH
        );
    }

    #[test]
    fn identity_validation_is_exact_and_composite() {
        let valid = identity("demo", &["demo", "Cap"], IdentityKind::Struct);
        let different_kind = identity("demo", &["demo", "Cap"], IdentityKind::Enum);
        assert_ne!(valid, different_kind);
        validate_identity_specs(
            [&valid, &different_kind].into_iter(),
            operation::POLICY_ANALYZE,
        )
        .unwrap();
        let invalid = identity("demo", &["other", "Cap"], IdentityKind::Struct);
        assert_eq!(
            validate_identity_specs([&invalid].into_iter(), operation::POLICY_ANALYZE)
                .unwrap_err()
                .label(),
            error_label::ROOT_MISMATCH
        );
    }

    #[test]
    fn rustdoc_types_format_version_matches_policy() {
        assert_eq!(rustdoc_types::FORMAT_VERSION, 57);
        assert_eq!(rustdoc_types::FORMAT_VERSION, RUSTDOC_FORMAT_VERSION);
    }

    #[test]
    fn strict_rustdoc_json_rejects_unknown_and_malformed_fields() {
        let directory = Scratch::new("strict-json");
        let (public_dir, private_dir, metadata_path) = pair_paths(&directory);
        let mut metadata = metadata_for("demo", "demo.json", census(1, 1, 0));
        write_json(&metadata_path, &metadata);
        write_pair(
            &public_dir,
            &private_dir,
            "demo.json",
            minimal_crate("demo"),
        );

        let mut value = serde_json::to_value(public_crate("demo")).unwrap();
        value.as_object_mut().unwrap().insert(
            "attacker_unknown".to_owned(),
            serde_json::Value::String("/home/alice/private".to_owned()),
        );
        write_json(&public_dir.join("demo.json"), &value);
        let error = load_workspace(&public_dir, &private_dir, &metadata_path).unwrap_err();
        assert_eq!(error.label(), error_label::INVALID_JSON);
        assert!(!error.to_string().contains("alice"));

        value.as_object_mut().unwrap().remove("attacker_unknown");
        value.as_object_mut().unwrap().insert(
            "format_version".to_owned(),
            serde_json::Value::String("57".to_owned()),
        );
        write_json(&public_dir.join("demo.json"), &value);
        assert_eq!(
            load_workspace(&public_dir, &private_dir, &metadata_path)
                .unwrap_err()
                .label(),
            error_label::INVALID_JSON
        );

        write_json(&public_dir.join("demo.json"), &public_crate("demo"));
        let mut malformed_private = serde_json::to_value(minimal_crate("demo")).unwrap();
        malformed_private.as_object_mut().unwrap().insert(
            "attacker_unknown".to_owned(),
            serde_json::Value::String("/home/alice/private".to_owned()),
        );
        write_json(&private_dir.join("demo.json"), &malformed_private);
        let error = load_workspace(&public_dir, &private_dir, &metadata_path).unwrap_err();
        assert_eq!(error.label(), error_label::INVALID_JSON);
        assert!(!error.to_string().contains("alice"));

        write_json(&private_dir.join("demo.json"), &minimal_crate("demo"));
        metadata.crates[0].public_census.index_items = 2;
        write_json(&metadata_path, &metadata);
        write_json(&public_dir.join("demo.json"), &public_crate("demo"));
        assert_eq!(
            load_workspace(&public_dir, &private_dir, &metadata_path)
                .unwrap_err()
                .label(),
            error_label::CENSUS_MISMATCH
        );
    }

    #[test]
    fn paired_directory_census_rejects_missing_and_extra_files() {
        let directory = Scratch::new("directory-census");
        let (public_dir, private_dir, metadata_path) = pair_paths(&directory);
        let metadata = metadata_for("demo", "demo.json", census(1, 1, 0));
        write_json(&metadata_path, &metadata);
        write_json(&public_dir.join("demo.json"), &public_crate("demo"));
        assert_eq!(
            load_workspace(&public_dir, &private_dir, &metadata_path)
                .unwrap_err()
                .label(),
            error_label::FILE_SET_MISMATCH
        );
        write_json(&private_dir.join("demo.json"), &minimal_crate("demo"));
        fs::write(private_dir.join("extra.json"), b"{}").unwrap();
        assert_eq!(
            load_workspace(&public_dir, &private_dir, &metadata_path)
                .unwrap_err()
                .label(),
            error_label::FILE_SET_MISMATCH
        );
    }

    #[test]
    fn rustdoc_validation_rejects_format_privacy_target_and_identity_mutations() {
        let directory = Scratch::new("rustdoc-mutations");
        let (public_dir, private_dir, metadata_path) = pair_paths(&directory);
        let metadata = metadata_for("demo", "demo.json", census(1, 1, 0));
        write_json(&metadata_path, &metadata);

        let mut public = public_crate("demo");
        let private = minimal_crate("demo");
        write_pair(&public_dir, &private_dir, "demo.json", private.clone());
        public.format_version = 56;
        write_json(&public_dir.join("demo.json"), &public);
        assert_eq!(
            load_workspace(&public_dir, &private_dir, &metadata_path)
                .unwrap_err()
                .label(),
            error_label::FORMAT_MISMATCH
        );

        public.format_version = RUSTDOC_FORMAT_VERSION;
        public.includes_private = true;
        write_json(&public_dir.join("demo.json"), &public);
        assert_eq!(
            load_workspace(&public_dir, &private_dir, &metadata_path)
                .unwrap_err()
                .label(),
            error_label::PRIVATE_ITEMS_REQUIRED
        );

        public.includes_private = false;
        public.target.triple = "attacker-target".to_owned();
        write_json(&public_dir.join("demo.json"), &public);
        assert_eq!(
            load_workspace(&public_dir, &private_dir, &metadata_path)
                .unwrap_err()
                .label(),
            error_label::IDENTITY_MISMATCH
        );

        public.target.triple = TEST_TARGET.to_owned();
        write_json(&public_dir.join("demo.json"), &public);
        let mut private_without_flag = private.clone();
        private_without_flag.includes_private = false;
        write_json(&private_dir.join("demo.json"), &private_without_flag);
        assert_eq!(
            load_workspace(&public_dir, &private_dir, &metadata_path)
                .unwrap_err()
                .label(),
            error_label::PRIVATE_ITEMS_REQUIRED
        );

        let mut wrong_identity = private;
        wrong_identity
            .paths
            .get_mut(&wrong_identity.root)
            .unwrap()
            .path = vec!["wrong".to_owned()];
        write_json(&private_dir.join("demo.json"), &wrong_identity);
        assert_eq!(
            load_workspace(&public_dir, &private_dir, &metadata_path)
                .unwrap_err()
                .label(),
            error_label::IDENTITY_MISMATCH
        );
    }

    #[test]
    fn private_hidden_census_detects_omitted_flag_and_accepts_legitimate_zero() {
        let directory = Scratch::new("hidden-census");
        let (public_dir, private_dir, metadata_path) = pair_paths(&directory);
        let mut metadata = metadata_for("demo", "demo.json", census(1, 1, 0));
        metadata.crates[0].private_hidden_items = 1;
        write_json(&metadata_path, &metadata);
        let mut complete_private = minimal_crate("demo");
        complete_private
            .index
            .get_mut(&complete_private.root)
            .unwrap()
            .attrs
            .push(rustdoc_types::Attribute::Other("#[doc(hidden)]".to_owned()));
        write_pair(&public_dir, &private_dir, "demo.json", complete_private);
        load_workspace(&public_dir, &private_dir, &metadata_path).unwrap();

        // Simulate the same crate built without `--document-hidden-items`.
        write_json(&private_dir.join("demo.json"), &minimal_crate("demo"));
        assert_eq!(
            load_workspace(&public_dir, &private_dir, &metadata_path)
                .unwrap_err()
                .label(),
            error_label::HIDDEN_ITEMS_REQUIRED
        );

        metadata.crates[0].private_hidden_items = 0;
        write_json(&metadata_path, &metadata);
        load_workspace(&public_dir, &private_dir, &metadata_path).unwrap();
    }

    #[test]
    fn public_ids_map_to_private_by_composite_identity_not_numeric_id() {
        let directory = Scratch::new("cross-blob-ids");
        let (public_dir, private_dir, metadata_path) = pair_paths(&directory);
        let mut public = public_crate("demo");
        let old_root = public.root;
        let new_root = Id(41);
        let mut root_item = public.index.remove(&old_root).unwrap();
        root_item.id = new_root;
        let root_summary = public.paths.remove(&old_root).unwrap();
        public.root = new_root;
        public.index.insert(new_root, root_item);
        public.paths.insert(new_root, root_summary);
        write_json(&public_dir.join("demo.json"), &public);
        write_json(&private_dir.join("demo.json"), &minimal_crate("demo"));
        write_json(
            &metadata_path,
            &metadata_for("demo", "demo.json", census(1, 1, 0)),
        );

        let workspace = load_workspace(&public_dir, &private_dir, &metadata_path).unwrap();
        let loaded = workspace.crate_by_name("demo").unwrap();
        assert_eq!(
            public_item_private_identity(&workspace, loaded, new_root, None)
                .unwrap()
                .unwrap(),
            identity("demo", &["demo"], IdentityKind::Module)
        );
        assert!(!loaded.private_krate.index.contains_key(&new_root));
    }

    #[test]
    fn snapshot_check_reports_only_closed_mismatch_label() {
        let directory = Scratch::new("snapshot-check");
        let path = directory.path().join("snapshot.txt");
        fs::write(&path, "/home/alice/secret\n").unwrap();
        let error = check_snapshot(&path, &["expected".to_owned()]).unwrap_err();
        assert_eq!(error.label(), error_label::SNAPSHOT_MISMATCH);
        assert!(!error.to_string().contains("alice"));
    }

    const TEST_TARGET: &str = "x86_64-unknown-linux-gnu";

    fn metadata_for(crate_name: &str, json_file: &str, counts: CrateCensus) -> WorkspaceMetadata {
        WorkspaceMetadata {
            schema_version: POLICY_SCHEMA_VERSION,
            nightly: PINNED_NIGHTLY.to_owned(),
            rustdoc_format_version: RUSTDOC_FORMAT_VERSION,
            target_triple: TEST_TARGET.to_owned(),
            crates: vec![CrateMetadata {
                crate_name: crate_name.to_owned(),
                public_json_file: json_file.to_owned(),
                public_census: counts,
                private_json_file: json_file.to_owned(),
                private_census: counts,
                private_hidden_items: 0,
            }],
        }
    }

    const fn census(index_items: usize, path_items: usize, external_crates: usize) -> CrateCensus {
        CrateCensus {
            index_items,
            path_items,
            external_crates,
        }
    }

    fn public_crate(crate_name: &str) -> StrictRustdocCrate {
        let mut krate = minimal_crate(crate_name);
        krate.includes_private = false;
        krate
    }

    fn minimal_crate(crate_name: &str) -> StrictRustdocCrate {
        let root = Id(0);
        StrictRustdocCrate {
            root,
            crate_version: None,
            includes_private: true,
            index: HashMap::from([(
                root,
                Item {
                    id: root,
                    crate_id: 0,
                    name: Some(crate_name.to_owned()),
                    span: None,
                    visibility: Visibility::Public,
                    docs: None,
                    links: HashMap::new(),
                    attrs: Vec::new(),
                    deprecation: None,
                    inner: ItemEnum::Module(rustdoc_types::Module {
                        is_crate: true,
                        items: Vec::new(),
                        is_stripped: false,
                    }),
                },
            )]),
            paths: HashMap::from([(
                root,
                ItemSummary {
                    crate_id: 0,
                    path: vec![crate_name.to_owned()],
                    kind: ItemKind::Module,
                },
            )]),
            external_crates: HashMap::new(),
            target: rustdoc_types::Target {
                triple: TEST_TARGET.to_owned(),
                target_features: Vec::new(),
            },
            format_version: RUSTDOC_FORMAT_VERSION,
        }
    }

    fn pair_paths(directory: &Scratch) -> (PathBuf, PathBuf, PathBuf) {
        let public_dir = directory.path().join("public");
        let private_dir = directory.path().join("private");
        fs::create_dir(&public_dir).unwrap();
        fs::create_dir(&private_dir).unwrap();
        (
            public_dir,
            private_dir,
            directory.path().join("metadata.json"),
        )
    }

    fn write_pair(
        public_dir: &Path,
        private_dir: &Path,
        json_file: &str,
        private: StrictRustdocCrate,
    ) {
        let mut public = minimal_crate(&private.paths[&private.root].path[0]);
        public.includes_private = false;
        write_json(&public_dir.join(json_file), &public);
        write_json(&private_dir.join(json_file), &private);
    }

    fn write_json(path: &Path, value: &impl Serialize) {
        fs::write(path, serde_json::to_vec(value).unwrap()).unwrap();
    }

    struct Scratch(PathBuf);

    impl Scratch {
        fn new(label: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("d2b-api-surface-{label}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir(&path).unwrap();
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

    fn crate_metadata(crate_name: &str, json_file: &str) -> CrateMetadata {
        CrateMetadata {
            crate_name: crate_name.to_owned(),
            public_json_file: json_file.to_owned(),
            public_census: census(1, 1, 0),
            private_json_file: json_file.to_owned(),
            private_census: census(1, 1, 0),
            private_hidden_items: 0,
        }
    }

    fn identity(origin: &str, path: &[&str], kind: IdentityKind) -> Identity {
        Identity {
            origin_crate: origin.to_owned(),
            path: path.iter().map(|part| (*part).to_owned()).collect(),
            kind,
        }
    }
}
