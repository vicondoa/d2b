//! Deterministic build-time entry point for the Zone resource compiler.
//!
//! The CLI deliberately accepts one declared JSON input and one declared
//! output. It does not discover artifacts, walk a package search path, or
//! invoke a second validation implementation. Provider layout and signature
//! checks are delegated to the library's anchored Linux implementation; this
//! file owns only the resource-bundle envelope and the input/output contract.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use d2b_contracts_provider::v3::{
    ArtifactDigest,
    ProviderManifest,
    semantic_services::catalog,
};
use d2b_contracts_resource::v3::{
    ArtifactId, CanonicalJsonValue, NIXOS_GENERATION_RESOURCE_TYPE, canonical_json_bytes,
    framed_canonical_digest, identity::STANDARD_RESOURCE_TYPES, is_canonical_digest,
    resource::RESOURCE_API_VERSION, ResourceUid,
};
use d2b_resource_compiler::{
    ArtifactCatalogEntry, CatalogDigests, Diagnostic, StaticPublisherKeys, compile_linux_artifact,
};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const RESOURCE_BUNDLE_DOMAIN_TAG: &str = "d2b:v3:resource-bundle";
const MAX_DIAGNOSTIC_BYTES: usize = d2b_resource_compiler::MAX_DIAGNOSTIC_BYTES;
const MAX_RESOURCES: usize = 4096;
const MAX_RESOURCE_BYTES: usize = 512 * 1024;
const MAX_SCHEMA_BYTES: usize = 8 * 1024 * 1024;
const ADDITIONAL_RESOURCE_TYPES: &[&str] = &[
    NIXOS_GENERATION_RESOURCE_TYPE,
    "display-wayland.d2bus.org.WaylandPolicy",
    "display-wayland.d2bus.org.WaylandSession",
];

fn resource_schema_filename(resource_type: &str) -> String {
    if STANDARD_RESOURCE_TYPES.contains(&resource_type) {
        format!("core.d2bus.org_{resource_type}.schema.json")
    } else if let Some((_namespace, type_segment)) = qualified_resource_type_parts(resource_type) {
        format!(
            "{}_{}.schema.json",
            resource_type
                .rsplit_once('.')
                .map(|(namespace, _)| namespace)
                .unwrap_or(resource_type),
            type_segment
        )
    } else {
        format!("{resource_type}.schema.json")
    }
}

fn qualified_resource_type_parts(resource_type: &str) -> Option<(&str, &str)> {
    let (namespace, type_segment) = resource_type.rsplit_once(".d2bus.org.")?;
    if namespace.is_empty()
        || namespace.len() > 63
        || type_segment.is_empty()
        || type_segment.len() > 63
        || !namespace.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
        })
        || !type_segment
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_uppercase())
        || !type_segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
    {
        return None;
    }
    Some((namespace, type_segment))
}

fn valid_resource_type(resource_type: &str) -> bool {
    STANDARD_RESOURCE_TYPES.contains(&resource_type)
        || ADDITIONAL_RESOURCE_TYPES.contains(&resource_type)
        || (qualified_resource_type_parts(resource_type).is_some()
            && catalog().iter().any(|pair| {
                pair.service().resource_type().as_str() == resource_type
                    || pair.binding().resource_type().as_str() == resource_type
            }))
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CliError {
    code: &'static str,
    exit_code: u8,
    message: String,
}

impl CliError {
    fn new(code: &'static str, message: impl AsRef<str>) -> Self {
        Self {
            code,
            exit_code: 1,
            message: bound_ascii(message.as_ref()),
        }
    }

    fn with_exit_code(code: &'static str, exit_code: u8, message: impl AsRef<str>) -> Self {
        Self {
            code,
            exit_code,
            message: bound_ascii(message.as_ref()),
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for CliError {}

impl From<Diagnostic> for CliError {
    fn from(error: Diagnostic) -> Self {
        Self::with_exit_code(error.code(), error.exit_code(), error.message())
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CompileInput {
    zone: String,
    #[serde(default)]
    zone_uid: Option<String>,
    resources: Vec<Value>,
    #[serde(default)]
    provider_schema_digests: BTreeMap<String, String>,
    #[serde(default)]
    providers: Vec<ProviderInput>,
    #[serde(default)]
    artifact_catalog_path: Option<PathBuf>,
    #[serde(default)]
    expected_artifact_catalog_digest: Option<String>,
    #[serde(default)]
    schema_root: Option<PathBuf>,
    #[serde(default)]
    expected_content_hash: Option<String>,
    #[serde(default)]
    strict_secrets: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ProviderInput {
    artifact_id: String,
    #[serde(rename = "type")]
    artifact_type: String,
    store_path: PathBuf,
    publisher: String,
    #[serde(default = "default_signature_id")]
    signature_id: String,
    package_digest: String,
    executable_digest: String,
    manifest_digest: String,
    config_schema_digest: String,
    signing_key: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BundleOutput<'a> {
    artifact_catalog_digest: &'a str,
    bundle_version: u32,
    content_hash: &'a str,
    generated_at: &'static str,
    provider_schema_digests: &'a BTreeMap<String, String>,
    resources: &'a [Value],
    schema_version: u32,
    zone: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    zone_uid: Option<&'a str>,
}

#[derive(Debug)]
struct CompiledProvider {
    input: ProviderInput,
    manifest: ProviderManifest,
    config_schema_digest: String,
    config_schema: Value,
}

fn default_signature_id() -> String {
    "default".to_owned()
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(error.exit_code)
        }
    }
}

fn run() -> Result<(), CliError> {
    let (input_path, output_path, strict_override) = parse_args()?;
    let input_bytes = fs::read(&input_path).map_err(|_| {
        CliError::new(
            "resource-compiler-input-unreadable",
            "declared input could not be read",
        )
    })?;
    if input_bytes.len() > MAX_RESOURCE_BYTES.saturating_mul(4) {
        return Err(CliError::new(
            "resource-compiler-input-too-large",
            "declared compiler input exceeds the bounded input size",
        ));
    }
    let input: CompileInput = serde_json::from_slice(&input_bytes).map_err(|_| {
        CliError::new(
            "resource-compiler-input-invalid",
            "declared compiler input is not a supported JSON object",
        )
    })?;
    compile(input, strict_override, &output_path)
}

fn parse_args() -> Result<(PathBuf, PathBuf, Option<bool>), CliError> {
    let mut args = env::args_os();
    let program = args.next().unwrap_or_default();
    let Some(command) = args.next() else {
        return Err(usage(&program));
    };
    if command != "compile" {
        return Err(usage(&program));
    }

    let mut input = None;
    let mut output = None;
    let mut strict_override = None;
    while let Some(argument) = args.next() {
        match argument.to_str() {
            Some("--input") => {
                input = Some(args.next().ok_or_else(|| usage(&program))?);
            }
            Some("--output") => {
                output = Some(args.next().ok_or_else(|| usage(&program))?);
            }
            Some("--strict-secrets") => strict_override = Some(true),
            Some("--allow-inline-secrets") => strict_override = Some(false),
            _ => return Err(usage(&program)),
        }
    }
    let input = input.ok_or_else(|| usage(&program))?;
    let output = output.ok_or_else(|| usage(&program))?;
    Ok((PathBuf::from(input), PathBuf::from(output), strict_override))
}

fn usage(program: &std::ffi::OsStr) -> CliError {
    let _ = program;
    CliError::new(
        "resource-compiler-usage",
        "usage: d2b-resource-compiler compile --input <declared-input.json> --output <bundle.json> [--strict-secrets]",
    )
}

fn compile(
    input: CompileInput,
    strict_override: Option<bool>,
    output_path: &Path,
) -> Result<(), CliError> {
    if input.zone.is_empty() || input.zone.len() > 63 || !valid_name(&input.zone) {
        return Err(CliError::new(
            "resource-compiler-zone-invalid",
            "declared Zone name is invalid",
        ));
    }
    if input.resources.len() > MAX_RESOURCES {
        return Err(CliError::new(
            "resource-compiler-resource-bound",
            "declared resource count exceeds the compiler bound",
        ));
    }
    let strict_secrets = strict_override.unwrap_or(input.strict_secrets);
    if let Some(zone_uid) = input.zone_uid.as_deref() {
        ResourceUid::parse(zone_uid.to_owned()).map_err(|_| {
            CliError::new("resource-compiler-zone-uid-invalid", "declared Zone UID is invalid")
        })?;
    }

    let artifact_catalog_digest = verify_artifact_catalog(&input)?;
    let compiled_providers = compile_providers(&input)?;
    check_provider_resource_admission(&input, &compiled_providers)?;
    check_resource_type_collisions(&compiled_providers)?;
    validate_resources(&input, strict_secrets)?;

    let resources_bytes = canonical_value_bytes(&Value::Array(input.resources.clone()))?;
    let content_hash = framed_canonical_digest(RESOURCE_BUNDLE_DOMAIN_TAG, &resources_bytes);
    if let Some(expected) = input.expected_content_hash.as_deref()
        && expected != content_hash
    {
        return Err(CliError::new(
            "resource-compiler-content-hash-mismatch",
            format!(
                "resource bundle contentHash differs between declared ({}) and compiler ({})",
                safe_token(expected),
                safe_token(&content_hash)
            ),
        ));
    }

    let output = BundleOutput {
        artifact_catalog_digest: &artifact_catalog_digest,
        bundle_version: 1,
        content_hash: &content_hash,
        generated_at: "1970-01-01T00:00:00.000Z",
        provider_schema_digests: &input.provider_schema_digests,
        resources: &input.resources,
        schema_version: 3,
        zone: &input.zone,
        zone_uid: input.zone_uid.as_deref(),
    };
    let output_bytes = canonical_json_bytes(&output).map_err(|_| {
        CliError::new(
            "resource-compiler-output-invalid",
            "compiled resource bundle could not be rendered canonically",
        )
    })?;
    fs::write(output_path, [output_bytes.as_slice(), b"\n"].concat()).map_err(|_| {
        CliError::new(
            "resource-compiler-output-unwritable",
            "declared compiler output could not be written",
        )
    })
}

fn verify_artifact_catalog(input: &CompileInput) -> Result<String, CliError> {
    let Some(path) = input.artifact_catalog_path.as_deref() else {
        return input
            .expected_artifact_catalog_digest
            .clone()
            .ok_or_else(|| {
                CliError::new(
                    "resource-compiler-catalog-missing",
                    "artifact catalog digest was not declared",
                )
            });
    };
    let bytes = fs::read(path).map_err(|_| {
        CliError::new(
            "resource-compiler-catalog-unreadable",
            "declared artifact catalog could not be read",
        )
    })?;
    let document: Value = serde_json::from_slice(&bytes).map_err(|_| {
        CliError::new(
            "resource-compiler-catalog-invalid",
            "declared artifact catalog is not valid JSON",
        )
    })?;
    let actual = document
        .get("catalogDigest")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            CliError::new(
                "resource-compiler-catalog-invalid",
                "declared artifact catalog has no catalogDigest",
            )
        })?;
    if let Some(expected) = input.expected_artifact_catalog_digest.as_deref()
        && expected != actual
    {
        return Err(CliError::new(
            "resource-compiler-catalog-digest-mismatch",
            format!(
                "artifact catalog digest differs between declared ({}) and realised ({})",
                safe_token(expected),
                safe_token(actual)
            ),
        ));
    }
    if let Some(entries) = document.get("entries").and_then(Value::as_array) {
        for provider in &input.providers {
            let Some(entry) = entries.iter().find(|entry| {
                entry.get("artifactId").and_then(Value::as_str)
                    == Some(provider.artifact_id.as_str())
            }) else {
                return Err(CliError::new(
                    "provider-artifact-id-not-found",
                    format!(
                        "Provider artifact {} is absent from the realised artifact catalog",
                        safe_token(&provider.artifact_id)
                    ),
                ));
            };
            let artifact_type = entry.get("type").and_then(Value::as_str).unwrap_or("");
            if artifact_type != provider.artifact_type {
                return Err(CliError::new(
                    "provider-artifact-type-invalid",
                    format!(
                        "Provider artifact {} type differs between declared ({}) and realised ({})",
                        safe_token(&provider.artifact_id),
                        safe_token(&provider.artifact_type),
                        safe_token(artifact_type)
                    ),
                ));
            }
            let realised_package_digest = entry
                .get("packageDigest")
                .and_then(Value::as_str)
                .unwrap_or("");
            if realised_package_digest != provider.package_digest {
                return Err(CliError::new(
                    "provider-digest-mismatch",
                    format!(
                        "provider artifact {} digest package differs between catalog ({}) and \
                         compiler ({})",
                        safe_token(&provider.artifact_id),
                        safe_token(realised_package_digest),
                        safe_token(&provider.package_digest)
                    ),
                ));
            }
        }
    }
    Ok(actual.to_owned())
}

fn compile_providers(input: &CompileInput) -> Result<Vec<CompiledProvider>, CliError> {
    let mut providers = Vec::new();
    for provider in &input.providers {
        let artifact_id = ArtifactId::parse(provider.artifact_id.clone()).map_err(|_| {
            CliError::new(
                "provider-artifact-id-invalid",
                "declared Provider artifact ID is invalid",
            )
        })?;
        if provider.artifact_type != "provider" {
            return Err(CliError::new(
                "provider-artifact-type-invalid",
                format!(
                    "provider artifact {} declares type {} instead of provider",
                    safe_token(&provider.artifact_id),
                    safe_token(&provider.artifact_type)
                ),
            ));
        }
        let digests = CatalogDigests::new(
            parse_digest(&provider.package_digest)?,
            parse_digest(&provider.executable_digest)?,
            parse_digest(&provider.manifest_digest)?,
            parse_digest(&provider.config_schema_digest)?,
        );
        let entry = ArtifactCatalogEntry::new(
            artifact_id,
            provider.store_path.clone(),
            provider.publisher.clone(),
            provider.signature_id.clone(),
            digests,
        );
        let mut keys = StaticPublisherKeys::default();
        let signing_key = read_public_key(&provider.signing_key)?;
        keys.insert_key(
            provider.publisher.clone(),
            provider.signature_id.clone(),
            signing_key,
        );
        let compiled = compile_linux_artifact(&entry, &keys).map_err(CliError::from)?;
        providers.push(CompiledProvider {
            input: clone_provider_input(provider),
            config_schema_digest: compiled.config_schema_digest().as_str().to_owned(),
            config_schema: serde_json::from_slice(compiled.config_schema_bytes()).map_err(
                |_| {
                    CliError::new(
                        "provider-config-schema-invalid",
                        "verified Provider config schema could not be decoded",
                    )
                },
            )?,
            manifest: compiled.manifest().clone(),
        });
    }
    Ok(providers)
}

fn clone_provider_input(input: &ProviderInput) -> ProviderInput {
    ProviderInput {
        artifact_id: input.artifact_id.clone(),
        artifact_type: input.artifact_type.clone(),
        store_path: input.store_path.clone(),
        publisher: input.publisher.clone(),
        signature_id: input.signature_id.clone(),
        package_digest: input.package_digest.clone(),
        executable_digest: input.executable_digest.clone(),
        manifest_digest: input.manifest_digest.clone(),
        config_schema_digest: input.config_schema_digest.clone(),
        signing_key: input.signing_key.clone(),
    }
}

fn read_public_key(value: &str) -> Result<Vec<u8>, CliError> {
    if value.starts_with("-----BEGIN ") {
        return Ok(value.as_bytes().to_vec());
    }
    fs::read(value).map_err(|_| {
        CliError::new(
            "provider-signature-key-unreadable",
            "declared public publisher key could not be read",
        )
    })
}

fn check_provider_resource_admission(
    input: &CompileInput,
    providers: &[CompiledProvider],
) -> Result<(), CliError> {
    let by_id: BTreeMap<_, _> = providers
        .iter()
        .map(|provider| (provider.input.artifact_id.as_str(), provider))
        .collect();
    for resource in &input.resources {
        if resource.get("type").and_then(Value::as_str) != Some("Provider") {
            continue;
        }
        let spec = resource
            .get("spec")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                CliError::new(
                    "provider-spec-invalid",
                    "Provider resource has no object spec",
                )
            })?;
        let artifact_id = spec
            .get("artifactId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CliError::new(
                    "provider-artifact-id-missing",
                    "Provider resource has no artifactId",
                )
            })?;
        let Some(provider) = by_id.get(artifact_id) else {
            return Err(CliError::new(
                "provider-artifact-id-not-found",
                format!(
                    "Provider resource artifact ID {} is not present in the declared provider catalog",
                    safe_token(artifact_id)
                ),
            ));
        };
        if provider.input.artifact_type != "provider" {
            return Err(CliError::new(
                "provider-artifact-type-invalid",
                format!(
                    "Provider resource artifact {} does not select a provider artifact",
                    safe_token(artifact_id)
                ),
            ));
        }
        let provider_name = resource
            .get("metadata")
            .and_then(|metadata| metadata.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("<provider>");
        let config = spec
            .get("config")
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        validate_schema(
            provider.config_schema.clone(),
            &config,
            &format!("Provider/{provider_name}.spec.config"),
        )?;
        let schema_key = format!("Provider/{provider_name}");
        let expected = input
            .provider_schema_digests
            .get(&schema_key)
            .ok_or_else(|| {
                CliError::new(
                    "provider-schema-digest-missing",
                    format!(
                        "Provider {} has no declared schema digest",
                        safe_token(provider_name)
                    ),
                )
            })?;
        if expected != &provider.config_schema_digest {
            return Err(CliError::new(
                "provider-schema-digest-mismatch",
                format!(
                    "Provider {} schema digest differs between declared ({}) and compiler ({})",
                    safe_token(provider_name),
                    safe_token(expected),
                    safe_token(&provider.config_schema_digest)
                ),
            ));
        }
    }
    Ok(())
}

fn check_resource_type_collisions(providers: &[CompiledProvider]) -> Result<(), CliError> {
    let mut owners = BTreeMap::<String, String>::new();
    for provider in providers {
        for resource_type in provider_resource_types(&provider.manifest) {
            if let Some(previous) =
                owners.insert(resource_type.clone(), provider.input.artifact_id.clone())
                && previous != provider.input.artifact_id
            {
                return Err(CliError::new(
                    "provider-resourcetype-collision",
                    format!(
                        "Provider artifacts {} and {} export the same ResourceType {}",
                        safe_token(&previous),
                        safe_token(&provider.input.artifact_id),
                        safe_token(&resource_type)
                    ),
                ));
            }
        }
    }
    Ok(())
}

fn provider_resource_types(manifest: &ProviderManifest) -> BTreeSet<String> {
    let mut types = BTreeSet::new();
    for component in manifest.components() {
        types.extend(
            component
                .exported_resource_types()
                .iter()
                .map(|resource_type| resource_type.as_str().to_owned()),
        );
    }
    types.extend(
        manifest
            .api_bindings()
            .iter()
            .map(|binding| binding.resource_type().as_str().to_owned()),
    );
    types
}

fn validate_resources(input: &CompileInput, strict_secrets: bool) -> Result<(), CliError> {
    let mut previous_key: Option<(String, String)> = None;
    let schema_cache = SchemaCache::new(input.schema_root.as_deref())?;
    let mut identities = BTreeSet::new();
    for (index, resource) in input.resources.iter().enumerate() {
        if serde_json::to_vec(resource).map_or(usize::MAX, |bytes| bytes.len()) > MAX_RESOURCE_BYTES
        {
            return Err(CliError::new(
                "resource-compiler-resource-too-large",
                format!("resource {} exceeds the bounded resource size", index),
            ));
        }
        let object = resource.as_object().ok_or_else(|| {
            CliError::new(
                "resource-compiler-resource-invalid",
                format!("resource {} is not an object", index),
            )
        })?;
        let api_version = object
            .get("apiVersion")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CliError::new(
                    "resource-compiler-version-mismatch",
                    "resource apiVersion is missing",
                )
            })?;
        if api_version != RESOURCE_API_VERSION {
            return Err(CliError::new(
                "resource-compiler-version-mismatch",
                "resource apiVersion does not match resources.d2bus.org/v3",
            ));
        }
        let resource_type = object.get("type").and_then(Value::as_str).ok_or_else(|| {
            CliError::new(
                "resource-compiler-resource-type-invalid",
                "resource type is missing",
            )
        })?;
        let metadata = object
            .get("metadata")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                CliError::new(
                    "resource-compiler-resource-metadata-invalid",
                    "resource metadata is missing",
                )
            })?;
        let name = metadata
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CliError::new(
                    "resource-compiler-resource-name-invalid",
                    "resource name is missing",
                )
            })?;
        let zone = metadata
            .get("zone")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                CliError::new(
                    "resource-compiler-resource-zone-invalid",
                    "resource Zone is missing",
                )
            })?;
        if zone != input.zone {
            return Err(CliError::new(
                "resource-compiler-resource-zone-mismatch",
                format!(
                    "resource {} is assigned to Zone {} instead of the declared Zone {}",
                    safe_token(name),
                    safe_token(zone),
                    safe_token(&input.zone)
                ),
            ));
        }
        let key = (resource_type.to_owned(), name.to_owned());
        if previous_key
            .as_ref()
            .is_some_and(|previous| previous > &key)
        {
            return Err(CliError::new(
                "resource-compiler-resource-order-invalid",
                "resource array is not sorted by (type,name)",
            ));
        }
        previous_key = Some(key);
        if !identities.insert((resource_type.to_owned(), name.to_owned())) {
            return Err(CliError::new(
                "resource-compiler-resource-duplicate",
                "resource array contains a duplicate (type,name) identity",
            ));
        }
        if strict_secrets && contains_secret_shape(resource) {
            return Err(CliError::new(
                "resource-compiler-inline-secret",
                format!(
                    "resource {} contains inline secret-shaped material",
                    safe_token(name)
                ),
            ));
        }
    }
    identities.insert(("Zone".to_owned(), input.zone.clone()));
    for resource in &input.resources {
        let object = resource
            .as_object()
            .expect("resource object was checked in the first pass");
        let resource_type = object
            .get("type")
            .and_then(Value::as_str)
            .expect("resource type was checked in the first pass");
        let name = object
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("name"))
            .and_then(Value::as_str)
            .expect("resource name was checked in the first pass");
        let schema = schema_cache.schema(resource_type)?.ok_or_else(|| {
            CliError::new(
                "resource-compiler-schema-root-missing",
                "strict ResourceType compilation requires a committed schema root",
            )
        })?;
        validate_schema_from_root(
            &schema,
            &schema,
            resource,
            &format!("{resource_type}/{name}"),
        )?;
    }
    for (index, resource) in input.resources.iter().enumerate() {
        let object = resource
            .as_object()
            .expect("resource object was checked in the first pass");
        let resource_type = object
            .get("type")
            .and_then(Value::as_str)
            .expect("resource type was checked in the first pass");
        let name = object
            .get("metadata")
            .and_then(Value::as_object)
            .and_then(|metadata| metadata.get("name"))
            .and_then(Value::as_str)
            .expect("resource name was checked in the first pass");
        let schema = schema_cache
            .schema(resource_type)?
            .expect("schema was loaded in the validation pass");
        validate_resource_references(
            resource,
            &schema,
            &identities,
            &format!("{resource_type}/{name}"),
            index,
        )?;
    }
    Ok(())
}

fn validate_resource_references(
    resource: &Value,
    schema: &Value,
    identities: &BTreeSet<(String, String)>,
    path: &str,
    index: usize,
) -> Result<(), CliError> {
    const MAX_REFERENCE_DEPTH: usize = 64;
    const MAX_REFERENCE_STEPS: usize = 100_000;

    #[allow(clippy::too_many_arguments)]
    fn visit(
        root: &Value,
        value: &Value,
        schema: &Value,
        path: &str,
        identities: &BTreeSet<(String, String)>,
        depth: usize,
        steps: &mut usize,
        active_refs: &mut BTreeSet<String>,
    ) -> Result<(), CliError> {
        *steps = steps.saturating_add(1);
        if *steps > MAX_REFERENCE_STEPS || depth > MAX_REFERENCE_DEPTH {
            return Err(schema_integrity_error(
                "resource reference traversal exceeds its bounded budget",
            ));
        }
        if value.is_null() {
            return Ok(());
        }

        if schema_is_resource_ref(root, schema) {
            validate_resource_ref_value(value, schema, identities, path)?;
        }

        if let Some(branches) = schema.get("allOf").and_then(Value::as_array) {
            for branch in branches {
                visit(
                    root,
                    value,
                    branch,
                    path,
                    identities,
                    depth + 1,
                    steps,
                    active_refs,
                )?;
            }
        }
        for branch_name in ["anyOf", "oneOf"] {
            if let Some(branches) = schema.get(branch_name).and_then(Value::as_array) {
                for branch in branches {
                    if schema_shape_matches(branch, value) {
                        visit(
                            root,
                            value,
                            branch,
                            path,
                            identities,
                            depth + 1,
                            steps,
                            active_refs,
                        )?;
                    }
                }
            }
        }

        if let Some(reference) = schema.get("$ref").and_then(Value::as_str)
            && let Some(definition) = resolve_schema_ref(root, root, reference)
        {
            if !active_refs.insert(reference.to_owned()) {
                return Err(schema_integrity_error(
                    "resource reference schema contains a cycle",
                ));
            }
            let result = visit(
                root,
                value,
                definition,
                path,
                identities,
                depth + 1,
                steps,
                active_refs,
            );
            active_refs.remove(reference);
            result?;
        }

        match value {
            Value::Object(object) => {
                let properties = schema
                    .get("properties")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                for (key, value) in object {
                    let child_schema = properties.get(key).or_else(|| {
                        schema
                            .get("additionalProperties")
                            .filter(|candidate| candidate.is_object())
                    });
                    if let Some(child_schema) = child_schema {
                        visit(
                            root,
                            value,
                            child_schema,
                            &format!("{path}.{key}"),
                            identities,
                            depth + 1,
                            steps,
                            active_refs,
                        )?;
                    }
                }
            }
            Value::Array(values) => {
                if let Some(item_schema) = schema.get("items") {
                    for (index, value) in values.iter().enumerate() {
                        visit(
                            root,
                            value,
                            item_schema,
                            &format!("{path}.{index}"),
                            identities,
                            depth + 1,
                            steps,
                            active_refs,
                        )?;
                    }
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
        }
        Ok(())
    }

    let mut steps = 0;
    let mut active_refs = BTreeSet::new();
    visit(
        schema,
        resource,
        schema,
        &format!("{path}[{index}]"),
        identities,
        0,
        &mut steps,
        &mut active_refs,
    )
}

fn resolve_schema_ref<'a>(
    root: &'a Value,
    _current: &'a Value,
    reference: &str,
) -> Option<&'a Value> {
    let name = reference.strip_prefix("#/definitions/")?;
    root.get("definitions")?.as_object()?.get(name)
}

fn schema_is_resource_ref(root: &Value, schema: &Value) -> bool {
    if schema.get("x-d2b-reference-kind").and_then(Value::as_str) == Some("ResourceRef") {
        return true;
    }
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| resolve_schema_ref(root, root, reference))
        .is_some_and(|definition| {
            definition
                .get("x-d2b-reference-kind")
                .and_then(Value::as_str)
                == Some("ResourceRef")
                || schema
                    .get("$ref")
                    .and_then(Value::as_str)
                    .is_some_and(|reference| reference.ends_with("/ResourceRef"))
        })
}

fn validate_resource_ref_value(
    value: &Value,
    schema: &Value,
    identities: &BTreeSet<(String, String)>,
    path: &str,
) -> Result<(), CliError> {
    let Some(reference) = value.as_str() else {
        return Err(CliError::new(
            "resource-compiler-reference-invalid",
            format!("{path} must be a ResourceRef string"),
        ));
    };
    let Some((resource_type, resource_name)) = reference.split_once('/') else {
        return Err(CliError::new(
            "resource-compiler-reference-invalid",
            format!("{path} must contain one ResourceRef type/name pair"),
        ));
    };
    if resource_type.is_empty()
        || resource_name.is_empty()
        || resource_name.contains('/')
        || !valid_resource_type(resource_type)
        || !valid_name(resource_name)
    {
        return Err(CliError::new(
            "resource-compiler-reference-invalid",
            format!("{path} contains an invalid ResourceRef identity"),
        ));
    }
    let allowed_types = schema
        .get("x-d2b-allowed-ref-types")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>())
        .unwrap_or_default();
    if !allowed_types.is_empty() && !allowed_types.contains(&resource_type) {
        return Err(CliError::new(
            "resource-compiler-reference-invalid",
            format!("{path} uses a ResourceRef type outside the schema allowance"),
        ));
    }
    let reference_scope = schema
        .get("x-d2b-reference-scope")
        .and_then(Value::as_str)
        .unwrap_or("same-zone");
    if !matches!(reference_scope, "same-zone" | "external") {
        return Err(schema_integrity_error(
            "ResourceRef schema has an unsupported locality declaration",
        ));
    }
    if reference_scope == "same-zone"
        && !identities.contains(&(resource_type.to_owned(), resource_name.to_owned()))
        && !is_bootstrap_external_reference(resource_type, resource_name)
    {
        return Err(CliError::new(
            "resource-compiler-reference-invalid",
            format!("{path} references a resource that is not declared in this Zone"),
        ));
    }
    Ok(())
}

fn is_bootstrap_external_reference(resource_type: &str, resource_name: &str) -> bool {
    resource_type == "Provider" && resource_name == "system-core"
}

fn schema_shape_matches(schema: &Value, value: &Value) -> bool {
    if let Some(constant) = schema.get("const") {
        return value == constant;
    }
    let Some(expected) = schema.get("type") else {
        return true;
    };
    let matches = |kind: &str| match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => false,
    };
    expected.as_str().is_some_and(matches)
        || expected
            .as_array()
            .is_some_and(|values| values.iter().filter_map(Value::as_str).any(matches))
}

struct SchemaCache {
    root: Option<PathBuf>,
    schemas: RefCell<BTreeMap<String, Value>>,
}

fn validate_schema_document(schema: &Value) -> Result<(), CliError> {
    let Some(object) = schema.as_object() else {
        return Err(schema_integrity_error("schema root is not an object"));
    };
    if object.get("$schema").and_then(Value::as_str)
        != Some("https://json-schema.org/draft/2020-12/schema")
    {
        return Err(schema_integrity_error(
            "schema must declare JSON Schema draft 2020-12",
        ));
    }
    if let Some(resource_type) = object.get("x-d2b-resource-type") {
        let resource_type = resource_type.as_str().ok_or_else(|| {
            schema_integrity_error("x-d2b-resource-type must be a ResourceType string")
        })?;
        if !valid_resource_type(resource_type)
            || object
                .get("title")
                .and_then(Value::as_str)
                .is_some_and(|title| title != resource_type && !title.contains('/'))
        {
            return Err(schema_integrity_error(
                "schema resource identity is invalid or disagrees with its title",
            ));
        }
    }
    let mut budget = SchemaDocumentBudget {
        nodes: 0,
        regexes: 0,
    };
    validate_schema_node_with_budget(schema, schema, "$", 0, &mut budget)?;
    if let Some(definitions) = schema.get("definitions").and_then(Value::as_object) {
        for (name, definition) in definitions {
            validate_schema_node_with_budget(
                definition,
                schema,
                &format!("$.definitions.{name}"),
                1,
                &mut budget,
            )?;
        }
    }
    Ok(())
}

struct SchemaDocumentBudget {
    nodes: usize,
    regexes: usize,
}

fn validate_schema_node_with_budget(
    schema: &Value,
    root: &Value,
    path: &str,
    depth: usize,
    budget: &mut SchemaDocumentBudget,
) -> Result<(), CliError> {
    budget.nodes = budget.nodes.saturating_add(1);
    if budget.nodes > 50_000 || depth > 128 {
        return Err(schema_integrity_error(
            "schema document exceeds structural node or depth budget",
        ));
    }
    let Some(object) = schema.as_object() else {
        return Err(schema_integrity_error(&format!(
            "{path} is not a schema object"
        )));
    };
    const KEYWORDS: &[&str] = &[
        "$id",
        "$ref",
        "$schema",
        "additionalProperties",
        "allOf",
        "anyOf",
        "const",
        "default",
        "definitions",
        "description",
        "enum",
        "format",
        "items",
        "maxItems",
        "maxLength",
        "maxProperties",
        "maximum",
        "minItems",
        "minLength",
        "minProperties",
        "minimum",
        "oneOf",
        "pattern",
        "properties",
        "required",
        "title",
        "type",
    ];
    for keyword in object.keys() {
        if !KEYWORDS.contains(&keyword.as_str())
            && !matches!(
                keyword.as_str(),
                "x-d2b-reference-kind"
                    | "x-d2b-reference-scope"
                    | "x-d2b-allowed-ref-types"
                    | "x-d2b-resource-type"
                    | "x-d2b-schema-version"
                    | "x-d2b-schema-fingerprint"
                    | "x-d2b-allowed-backing-ref-types"
                    | "x-d2b-allowed-binding-target-ref-types"
                    | "x-d2b-binding-resource-type"
                    | "x-d2b-exportability"
                    | "x-d2b-factory-fingerprint"
                    | "x-d2b-projection-protocol-version"
                    | "x-d2b-projection-schema-fingerprint"
            )
        {
            return Err(schema_integrity_error(&format!(
                "{path} contains unsupported keyword {}",
                safe_token(keyword)
            )));
        }
    }
    if let Some(reference) = object.get("$ref") {
        let reference = reference
            .as_str()
            .ok_or_else(|| schema_integrity_error(&format!("{path}.$ref must be a string")))?;
        if !reference.starts_with("#/definitions/")
            || resolve_schema_ref(root, root, reference).is_none()
        {
            return Err(schema_integrity_error(&format!(
                "{path} contains an unsupported or missing schema reference"
            )));
        }
    }
    if let Some(pattern) = object.get("pattern") {
        let pattern = pattern
            .as_str()
            .ok_or_else(|| schema_integrity_error(&format!("{path}.pattern must be a string")))?;
        budget.regexes = budget.regexes.saturating_add(1);
        if budget.regexes > 4_096 {
            return Err(schema_integrity_error(
                "schema document exceeds regex budget",
            ));
        }
        Regex::new(pattern).map_err(|_| schema_integrity_error("schema pattern is malformed"))?;
    }
    if let Some(kind) = object.get("type") {
        let valid = kind.as_str().is_some_and(valid_schema_type)
            || kind.as_array().is_some_and(|values| {
                !values.is_empty()
                    && values
                        .iter()
                        .all(|value| value.as_str().is_some_and(valid_schema_type))
                    && values
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == values.len()
            });
        if !valid {
            return Err(schema_integrity_error(&format!("{path}.type is invalid")));
        }
    }
    for keyword in ["minimum", "maximum"] {
        if let Some(value) = object.get(keyword)
            && (!value.is_number() || value.as_f64().is_none())
        {
            return Err(schema_integrity_error(&format!(
                "{path}.{keyword} must be numeric"
            )));
        }
    }
    for keyword in [
        "minLength",
        "maxLength",
        "minItems",
        "maxItems",
        "minProperties",
        "maxProperties",
    ] {
        if let Some(value) = object.get(keyword)
            && value.as_u64().is_none()
        {
            return Err(schema_integrity_error(&format!(
                "{path}.{keyword} must be a non-negative integer"
            )));
        }
    }
    if let Some(format) = object.get("format") {
        let format = format
            .as_str()
            .ok_or_else(|| schema_integrity_error(&format!("{path}.format must be a string")))?;
        if !matches!(format, "int32" | "uint8" | "uint16" | "uint32" | "uint64") {
            return Err(schema_integrity_error(&format!(
                "{path}.format is unsupported"
            )));
        }
    }
    if let Some(required) = object.get("required") {
        let required = required
            .as_array()
            .ok_or_else(|| schema_integrity_error(&format!("{path}.required must be an array")))?;
        let names = required
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| schema_integrity_error(&format!("{path}.required is invalid")))
            })
            .collect::<Result<Vec<_>, _>>()?;
        if names.iter().collect::<BTreeSet<_>>().len() != names.len() {
            return Err(schema_integrity_error(&format!(
                "{path}.required contains duplicate names"
            )));
        }
    }
    if let Some(additional) = object.get("additionalProperties")
        && !additional.is_boolean()
        && !additional.is_object()
    {
        return Err(schema_integrity_error(&format!(
            "{path}.additionalProperties must be a boolean or schema object"
        )));
    }
    if let Some(items) = object.get("items")
        && !items.is_boolean()
        && !items.is_object()
    {
        return Err(schema_integrity_error(&format!(
            "{path}.items must be a boolean or schema object"
        )));
    }
    if let Some(properties) = object.get("properties") {
        let properties = properties.as_object().ok_or_else(|| {
            schema_integrity_error(&format!("{path}.properties must be an object"))
        })?;
        for (name, property) in properties {
            if !property.is_object() {
                return Err(schema_integrity_error(&format!(
                    "{path}.properties.{name} is not a schema object"
                )));
            }
        }
    }
    if let Some(definitions) = object.get("definitions") {
        let definitions = definitions.as_object().ok_or_else(|| {
            schema_integrity_error(&format!("{path}.definitions must be an object"))
        })?;
        for (name, definition) in definitions {
            if !definition.is_object() {
                return Err(schema_integrity_error(&format!(
                    "{path}.definitions.{name} is not a schema object"
                )));
            }
        }
    }
    if let Some(enum_values) = object.get("enum") {
        let values = enum_values
            .as_array()
            .ok_or_else(|| schema_integrity_error(&format!("{path}.enum must be an array")))?;
        if values.is_empty() {
            return Err(schema_integrity_error(&format!(
                "{path}.enum must not be empty"
            )));
        }
    }
    for keyword in ["allOf", "anyOf", "oneOf"] {
        if let Some(branches) = object.get(keyword) {
            let branches = branches.as_array().ok_or_else(|| {
                schema_integrity_error(&format!("{path}.{keyword} must be an array"))
            })?;
            if branches.is_empty() {
                return Err(schema_integrity_error(&format!(
                    "{path}.{keyword} must not be empty"
                )));
            }
            if branches.iter().any(|branch| !branch.is_object()) {
                return Err(schema_integrity_error(&format!(
                    "{path}.{keyword} contains a non-schema branch"
                )));
            }
        }
    }
    validate_x_d2b_metadata(object, root, path)?;
    if let (Some(minimum), Some(maximum)) = (
        object.get("minimum").and_then(Value::as_number),
        object.get("maximum").and_then(Value::as_number),
    ) && compare_schema_numbers(minimum, maximum) == Some(std::cmp::Ordering::Greater)
    {
        return Err(schema_integrity_error(&format!(
            "{path} has minimum greater than maximum"
        )));
    }
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, property) in properties {
            validate_schema_node_with_budget(
                property,
                root,
                &format!("{path}.properties.{name}"),
                depth + 1,
                budget,
            )?;
        }
    }
    if let Some(definitions) = object.get("definitions").and_then(Value::as_object) {
        for (name, definition) in definitions {
            validate_schema_node_with_budget(
                definition,
                root,
                &format!("{path}.definitions.{name}"),
                depth + 1,
                budget,
            )?;
        }
    }
    if let Some(Value::Object(_)) = object.get("additionalProperties") {
        let additional = object.get("additionalProperties").expect("checked above");
        validate_schema_node_with_budget(
            additional,
            root,
            &format!("{path}.additionalProperties"),
            depth + 1,
            budget,
        )?;
    }
    if let Some(Value::Object(_)) = object.get("items") {
        let items = object.get("items").expect("checked above");
        validate_schema_node_with_budget(items, root, &format!("{path}.items"), depth + 1, budget)?;
    }
    for keyword in ["allOf", "anyOf", "oneOf"] {
        if let Some(branches) = object.get(keyword).and_then(Value::as_array) {
            for (index, branch) in branches.iter().enumerate() {
                validate_schema_node_with_budget(
                    branch,
                    root,
                    &format!("{path}.{keyword}.{index}"),
                    depth + 1,
                    budget,
                )?;
            }
        }
    }
    Ok(())
}

fn valid_schema_type(value: &str) -> bool {
    matches!(
        value,
        "array" | "boolean" | "integer" | "null" | "number" | "object" | "string"
    )
}

fn validate_x_d2b_metadata(
    object: &Map<String, Value>,
    _root: &Value,
    path: &str,
) -> Result<(), CliError> {
    if let Some(kind) = object.get("x-d2b-reference-kind")
        && kind.as_str() != Some("ResourceRef")
    {
        return Err(schema_integrity_error(&format!(
            "{path}.x-d2b-reference-kind must be ResourceRef"
        )));
    }
    if let Some(scope) = object.get("x-d2b-reference-scope")
        && !matches!(scope.as_str(), Some("same-zone" | "external"))
    {
        return Err(schema_integrity_error(&format!(
            "{path}.x-d2b-reference-scope is invalid"
        )));
    }
    for key in ["x-d2b-allowed-ref-types", "x-d2b-allowed-backing-ref-types"] {
        if let Some(value) = object.get(key) {
            let values = value
                .as_array()
                .ok_or_else(|| schema_integrity_error(&format!("{path}.{key} must be an array")))?;
            for item in values {
                let item = item.as_str().ok_or_else(|| {
                    schema_integrity_error(&format!("{path}.{key} contains a non-string"))
                })?;
                if !valid_resource_type(item) {
                    return Err(schema_integrity_error(&format!(
                        "{path}.{key} contains an invalid ResourceType"
                    )));
                }
            }
        }
    }
    if let Some(value) = object.get("x-d2b-allowed-binding-target-ref-types") {
        let values = value.as_array().ok_or_else(|| {
            schema_integrity_error(&format!(
                "{path}.x-d2b-allowed-binding-target-ref-types must be an array"
            ))
        })?;
        for item in values {
            let item = item.as_str().ok_or_else(|| {
                schema_integrity_error(&format!(
                    "{path}.x-d2b-allowed-binding-target-ref-types contains a non-string"
                ))
            })?;
            if !valid_name(item) {
                return Err(schema_integrity_error(&format!(
                    "{path}.x-d2b-allowed-binding-target-ref-types contains an invalid target"
                )));
            }
        }
    }
    for key in ["x-d2b-resource-type", "x-d2b-binding-resource-type"] {
        if let Some(value) = object.get(key) {
            let value = value.as_str().ok_or_else(|| {
                schema_integrity_error(&format!("{path}.{key} must be a ResourceType string"))
            })?;
            if !valid_resource_type(value) {
                return Err(schema_integrity_error(&format!(
                    "{path}.{key} is not a valid ResourceType"
                )));
            }
        }
    }
    for key in [
        "x-d2b-schema-version",
        "x-d2b-exportability",
        "x-d2b-projection-protocol-version",
    ] {
        if let Some(value) = object.get(key)
            && value.as_str().is_none()
        {
            return Err(schema_integrity_error(&format!(
                "{path}.{key} must be a string"
            )));
        }
    }
    for key in [
        "x-d2b-schema-fingerprint",
        "x-d2b-factory-fingerprint",
        "x-d2b-projection-schema-fingerprint",
    ] {
        if let Some(value) = object.get(key) {
            let value = value.as_str().ok_or_else(|| {
                schema_integrity_error(&format!("{path}.{key} must be a digest string"))
            })?;
            if !valid_digest(value) {
                return Err(schema_integrity_error(&format!(
                    "{path}.{key} is not a sha256 digest"
                )));
            }
        }
    }
    Ok(())
}

fn valid_digest(value: &str) -> bool {
    is_canonical_digest(value)
}

impl SchemaCache {
    fn new(root: Option<&Path>) -> Result<Self, CliError> {
        if let Some(root) = root {
            let metadata = fs::metadata(root).map_err(|_| {
                CliError::new(
                    "resource-compiler-schema-root-unreadable",
                    "declared schema root could not be read",
                )
            })?;
            if !metadata.is_dir() {
                return Err(CliError::new(
                    "resource-compiler-schema-root-invalid",
                    "declared schema root is not a directory",
                ));
            }
        }
        Ok(Self {
            root: root.map(Path::to_owned),
            schemas: RefCell::new(BTreeMap::new()),
        })
    }

    fn schema(&self, resource_type: &str) -> Result<Option<Value>, CliError> {
        if let Some(schema) = self.schemas.borrow().get(resource_type) {
            return Ok(Some(schema.clone()));
        }
        let Some(root) = &self.root else {
            return Ok(None);
        };
        if !valid_resource_type(resource_type) {
            return Err(CliError::new(
                "resource-compiler-resource-type-invalid",
                "resource type is not a supported schema name",
            ));
        }

        let path = root.join(resource_schema_filename(resource_type));
        if !path.exists() {
            return Err(CliError::new(
                "resource-compiler-schema-missing",
                format!(
                    "schema for ResourceType {} is missing",
                    safe_token(resource_type)
                ),
            ));
        }
        let bytes = fs::read(path).map_err(|_| {
            CliError::new(
                "resource-compiler-schema-unreadable",
                "declared ResourceType schema could not be read",
            )
        })?;
        if bytes.len() > MAX_SCHEMA_BYTES {
            return Err(CliError::new(
                "resource-compiler-schema-too-large",
                "declared ResourceType schema exceeds the bounded schema size",
            ));
        }
        let schema: Value = serde_json::from_slice(&bytes).map_err(|_| {
            CliError::new(
                "resource-compiler-schema-integrity-failure",
                "declared ResourceType schema is not valid JSON",
            )
        })?;
        validate_schema_document(&schema)?;
        if schema.get("x-d2b-resource-type").and_then(Value::as_str) != Some(resource_type) {
            return Err(schema_integrity_error(
                "loaded ResourceType schema identity does not match the requested ResourceType",
            ));
        }
        self.schemas
            .borrow_mut()
            .insert(resource_type.to_owned(), schema.clone());
        Ok(Some(schema))
    }
}

fn validate_schema(schema: Value, value: &Value, path: &str) -> Result<(), CliError> {
    validate_schema_document(&schema)?;
    let root = schema.clone();
    validate_schema_from_root(&root, &schema, value, path)
}

fn validate_schema_from_root(
    root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
) -> Result<(), CliError> {
    let mut budget = SchemaValueBudget {
        steps: 0,
        active_refs: BTreeSet::new(),
    };
    validate_schema_root(root, schema, value, path, 0, &mut budget)
}

struct SchemaValueBudget {
    steps: usize,
    active_refs: BTreeSet<String>,
}

fn validate_schema_root(
    root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
    depth: usize,
    budget: &mut SchemaValueBudget,
) -> Result<(), CliError> {
    budget.steps = budget.steps.saturating_add(1);
    if budget.steps > 100_000 || depth > 64 {
        return Err(schema_integrity_error(
            "schema validation exceeds its bounded budget",
        ));
    }
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let definition = resolve_schema_ref(root, root, reference)
            .ok_or_else(|| schema_error(path, "references a missing schema definition"))?;
        if !budget.active_refs.insert(reference.to_owned()) {
            return Err(schema_integrity_error("schema reference cycle detected"));
        }
        let result = validate_schema_root(root, definition, value, path, depth + 1, budget);
        budget.active_refs.remove(reference);
        return result;
    }
    if let Some(expected) = schema.get("type") {
        let matches = expected
            .as_str()
            .is_some_and(|kind| value_matches(kind, value))
            || expected.as_array().is_some_and(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .any(|kind| value_matches(kind, value))
            });
        if !matches {
            return Err(schema_error(path, "has the wrong JSON type"));
        }
    }
    if let Some(constant) = schema.get("const")
        && value != constant
    {
        return Err(schema_error(path, "does not equal the schema constant"));
    }
    if let Some(values) = schema.get("enum").and_then(Value::as_array)
        && !values.iter().any(|candidate| candidate == value)
    {
        return Err(schema_error(path, "is outside the schema enum"));
    }
    if let Some(value) = value.as_str() {
        if let Some(pattern) = schema.get("pattern").and_then(Value::as_str)
            && !Regex::new(pattern)
                .map_err(|_| schema_integrity_error("schema pattern is malformed"))?
                .is_match(value)
        {
            return Err(schema_error(path, "does not match the schema pattern"));
        }
        if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64)
            && value.chars().count() < minimum as usize
        {
            return Err(schema_error(path, "is shorter than the schema minimum"));
        }
        if let Some(maximum) = schema.get("maxLength").and_then(Value::as_u64)
            && value.chars().count() > maximum as usize
        {
            return Err(schema_error(path, "is longer than the schema maximum"));
        }
    }
    if let Some(number) = value.as_f64() {
        if let Some(minimum) = schema.get("minimum").and_then(Value::as_f64)
            && number < minimum
        {
            return Err(schema_error(path, "is below the schema minimum"));
        }
        if let Some(maximum) = schema.get("maximum").and_then(Value::as_f64)
            && number > maximum
        {
            return Err(schema_error(path, "is above the schema maximum"));
        }
        if let Some(format) = schema.get("format").and_then(Value::as_str) {
            let valid = match format {
                "int32" => {
                    number.fract() == 0.0 && (-2_147_483_648.0..=2_147_483_647.0).contains(&number)
                }
                "uint8" => number.fract() == 0.0 && (0.0..=255.0).contains(&number),
                "uint16" => number.fract() == 0.0 && (0.0..=65_535.0).contains(&number),
                "uint32" => number.fract() == 0.0 && (0.0..=4_294_967_295.0).contains(&number),
                "uint64" => number.fract() == 0.0 && number >= 0.0,
                _ => true,
            };
            if !valid {
                return Err(schema_error(
                    path,
                    "does not satisfy the schema numeric format",
                ));
            }
        }
        if let Some(number) = value.as_number() {
            for (keyword, is_lower_bound) in [("minimum", true), ("maximum", false)] {
                let Some(bound) = schema.get(keyword).and_then(Value::as_number) else {
                    continue;
                };
                if let Some(ordering) = compare_integer_numbers(number, bound) {
                    let invalid = if is_lower_bound {
                        ordering == std::cmp::Ordering::Less
                    } else {
                        ordering == std::cmp::Ordering::Greater
                    };
                    if invalid {
                        return Err(schema_error(
                            path,
                            if is_lower_bound {
                                "is below the schema minimum"
                            } else {
                                "is above the schema maximum"
                            },
                        ));
                    }
                }
            }
        }
    }
    if let Some(value) = value.as_array() {
        if let Some(minimum) = schema.get("minItems").and_then(Value::as_u64)
            && value.len() < minimum as usize
        {
            return Err(schema_error(
                path,
                "has fewer items than the schema minimum",
            ));
        }
        if let Some(maximum) = schema.get("maxItems").and_then(Value::as_u64)
            && value.len() > maximum as usize
        {
            return Err(schema_error(path, "has more items than the schema maximum"));
        }
        if let Some(item_schema) = schema.get("items") {
            for (index, item) in value.iter().enumerate() {
                if let Some(item_schema) = item_schema.as_object() {
                    validate_schema_root(
                        root,
                        &Value::Object(item_schema.clone()),
                        item,
                        &format!("{path}.{index}"),
                        depth + 1,
                        budget,
                    )?;
                } else if item_schema == &Value::Bool(false) {
                    return Err(schema_error(
                        path,
                        "contains an item rejected by the schema",
                    ));
                }
            }
        }
    }
    if let Some(value) = value.as_object() {
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if let Some(minimum) = schema.get("minProperties").and_then(Value::as_u64)
            && value.len() < minimum as usize
        {
            return Err(schema_error(
                path,
                "has fewer properties than the schema minimum",
            ));
        }
        if let Some(maximum) = schema.get("maxProperties").and_then(Value::as_u64)
            && value.len() > maximum as usize
        {
            return Err(schema_error(
                path,
                "has more properties than the schema maximum",
            ));
        }
        if let Some(required) = schema.get("required").and_then(Value::as_array)
            && required
                .iter()
                .filter_map(Value::as_str)
                .any(|key| !value.contains_key(key))
        {
            return Err(schema_error(path, "is missing a required field"));
        }
        for (key, value) in value {
            if let Some(property_schema) = properties.get(key) {
                validate_schema_root(
                    root,
                    property_schema,
                    value,
                    &format!("{path}.{key}"),
                    depth + 1,
                    budget,
                )?;
            } else if let Some(additional) = schema.get("additionalProperties") {
                match additional {
                    Value::Bool(false) => {
                        return Err(schema_error(path, "contains an undeclared field"));
                    }
                    Value::Object(additional_schema) => {
                        validate_schema_root(
                            root,
                            &Value::Object(additional_schema.clone()),
                            value,
                            &format!("{path}.{key}"),
                            depth + 1,
                            budget,
                        )?;
                    }
                    _ => {}
                }
            }
        }
    }
    if let Some(branches) = schema.get("allOf").and_then(Value::as_array) {
        for branch in branches {
            validate_schema_root(root, branch, value, path, depth + 1, budget)?;
        }
    }
    for branch_name in ["anyOf", "oneOf"] {
        if let Some(branches) = schema.get(branch_name).and_then(Value::as_array) {
            let passed = branches
                .iter()
                .filter(|branch| {
                    validate_schema_root(root, branch, value, path, depth + 1, budget).is_ok()
                })
                .count();
            let valid = if branch_name == "oneOf" {
                passed == 1
            } else {
                passed >= 1
            };
            if !valid {
                return Err(schema_error(path, "does not satisfy the schema branch"));
            }
        }
    }
    Ok(())
}

fn value_matches(kind: &str, value: &Value) -> bool {
    match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    }
}

fn compare_integer_numbers(
    left: &serde_json::Number,
    right: &serde_json::Number,
) -> Option<std::cmp::Ordering> {
    match (left.as_i64(), left.as_u64(), right.as_i64(), right.as_u64()) {
        (Some(left), _, Some(right), _) => Some(left.cmp(&right)),
        (_, Some(left), _, Some(right)) => Some(left.cmp(&right)),
        (Some(left), _, _, Some(_right)) if left < 0 => Some(std::cmp::Ordering::Less),
        (Some(left), _, _, Some(right)) => Some((left as u64).cmp(&right)),
        (_, Some(_left), Some(right), _) if right < 0 => Some(std::cmp::Ordering::Greater),
        (_, Some(left), Some(right), _) => Some(left.cmp(&(right as u64))),
        _ => None,
    }
}

fn compare_schema_numbers(
    left: &serde_json::Number,
    right: &serde_json::Number,
) -> Option<std::cmp::Ordering> {
    compare_integer_numbers(left, right).or_else(|| left.as_f64()?.partial_cmp(&right.as_f64()?))
}

fn schema_error(path: &str, reason: &str) -> CliError {
    CliError::new(
        "resource-compiler-schema-invalid",
        format!("{} {}", safe_token(path), reason),
    )
}

fn schema_integrity_error(reason: &str) -> CliError {
    CliError::new("resource-compiler-schema-integrity-failure", reason)
}

fn canonical_value_bytes(value: &Value) -> Result<Vec<u8>, CliError> {
    let parsed = CanonicalJsonValue::parse(&serde_json::to_vec(value).map_err(|_| {
        CliError::new(
            "resource-compiler-json-invalid",
            "JSON value could not be rendered",
        )
    })?)
    .map_err(|_| {
        CliError::new(
            "resource-compiler-json-noncanonical",
            "JSON value is outside d2b-cjson/v1",
        )
    })?;
    canonical_json_bytes(&parsed).map_err(|_| {
        CliError::new(
            "resource-compiler-json-noncanonical",
            "JSON value is outside d2b-cjson/v1",
        )
    })
}

fn parse_digest(value: &str) -> Result<ArtifactDigest, CliError> {
    ArtifactDigest::parse(value.to_owned()).map_err(|_| {
        CliError::new(
            "provider-digest-invalid",
            "declared Provider digest is not sha256:<64 lowercase hex>",
        )
    })
}

fn contains_secret_shape(value: &Value) -> bool {
    if value
        .get("type")
        .and_then(Value::as_str)
        .is_some_and(|resource_type| resource_type == "Volume")
    {
        return contains_volume_secret_shape(value);
    }
    contains_secret_shape_at(value, false)
}

fn contains_volume_secret_shape(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return contains_secret_shape_at(value, false);
    };
    object.iter().any(|(key, value)| {
        if forbidden_key(key) && key != "path" {
            return true;
        }
        if key == "spec" {
            return contains_volume_spec_secret_shape(value);
        }
        contains_secret_shape_at(value, false)
    })
}

fn contains_volume_spec_secret_shape(value: &Value) -> bool {
    let Some(object) = value.as_object() else {
        return contains_secret_shape_at(value, false);
    };
    object.iter().any(|(key, value)| {
        if forbidden_key(key) && key != "attachments" && key != "layout" && key != "views" {
            return true;
        }
        match key.as_str() {
            "attachments" => value.as_array().is_none_or(|attachments| {
                attachments
                    .iter()
                    .any(|attachment| contains_secret_shape_at(attachment, true))
            }),
            "layout" => value.as_array().is_none_or(|entries| {
                entries
                    .iter()
                    .any(|entry| contains_secret_shape_at(entry, true))
            }),
            "views" => value.as_object().is_none_or(|views| {
                views
                    .values()
                    .any(|view| contains_secret_shape_at(view, true))
            }),
            _ => contains_secret_shape_at(value, false),
        }
    })
}

fn contains_secret_shape_at(value: &Value, allow_path: bool) -> bool {
    match value {
        Value::String(value) => {
            let lower = value.to_ascii_lowercase();
            lower.contains("-----begin")
                || lower.contains("/nix/store/")
                || lower.starts_with("eyj")
                || lower.contains("privatekey")
                || lower.contains("access_token")
                || lower.contains("password")
                || lower.contains("secret")
        }
        Value::Array(values) => values.iter().any(contains_secret_shape),
        Value::Object(values) => values.iter().any(|(key, value)| {
            (forbidden_key(key) && !(allow_path && (key == "path" || key == "mountPath")))
                || contains_secret_shape_at(value, false)
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn forbidden_key(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "secret",
        "password",
        "token",
        "privatekey",
        "argv",
        "commandline",
        "socket",
        "path",
        "pid",
        "uid",
        "env",
        "exe",
    ]
    .iter()
    .any(|needle| lower == *needle || lower.ends_with(needle))
}

#[allow(clippy::items_after_test_module)]
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn schema_root() -> std::path::PathBuf {
        std::env::var_os("D2B_RESOURCE_SCHEMA_ROOT")
            .map(std::path::PathBuf::from)
            .map(|path| {
                if path.extension().is_some() {
                    path.parent().unwrap_or(&path).to_path_buf()
                } else {
                    path
                }
            })
            .unwrap_or_else(|| {
                std::env::current_dir()
                    .expect("current directory")
                    .ancestors()
                    .map(|root| root.join("docs/reference/schemas/v3"))
                    .find(|path| path.is_dir())
                    .expect("resource schemas are discoverable")
            })
    }

    fn draft_schema(mut body: Value) -> Value {
        body.as_object_mut().unwrap().insert(
            "$schema".to_owned(),
            Value::String("https://json-schema.org/draft/2020-12/schema".to_owned()),
        );
        body
    }

    #[test]
    fn malformed_committed_pattern_is_schema_integrity_failure() {
        let error = validate_schema(
            draft_schema(json!({"type": "string", "pattern": "["})),
            &json!("value"),
            "value",
        )
        .unwrap_err();
        assert_eq!(error.code, "resource-compiler-schema-integrity-failure");
    }

    #[test]
    fn all_of_numeric_bounds_and_schema_valued_additional_properties_are_enforced() {
        let schema = draft_schema(json!({
            "allOf": [
                {"type": "object", "minProperties": 1},
                {"type": "object", "properties": {"count": {
                    "type": "integer", "minimum": 1, "maximum": 3
                }}}
            ],
            "type": "object",
            "properties": {"count": {
                "type": "integer", "minimum": 1, "maximum": 3
            }},
            "additionalProperties": {"type": "string"}
        }));
        validate_schema(schema.clone(), &json!({"count": 2, "label": "ok"}), "value").unwrap();
        assert!(validate_schema(schema.clone(), &json!({}), "value").is_err());
        assert!(validate_schema(schema, &json!({"count": 4}), "value").is_err());
    }

    #[test]
    fn every_committed_v3_schema_passes_integrity_validation() {
        let root = schema_root();
        let mut count = 0;
        for entry in fs::read_dir(root).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let schema: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            validate_schema_document(&schema)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            count += 1;
        }
        assert!(count >= 40);
    }

    #[test]
    fn qualified_resource_refs_are_local_when_schema_metadata_says_same_zone() {
        let schema = json!({
            "type": "string",
            "pattern": "^audio\\.d2bus\\.org\\.AudioService/[a-z][a-z0-9-]{0,62}$",
            "x-d2b-reference-kind": "ResourceRef",
            "x-d2b-reference-scope": "same-zone",
            "x-d2b-allowed-ref-types": ["audio.d2bus.org.AudioService"]
        });
        let mut identities = BTreeSet::new();
        identities.insert((
            "audio.d2bus.org.AudioService".to_owned(),
            "audio".to_owned(),
        ));
        validate_resource_ref_value(
            &json!("audio.d2bus.org.AudioService/audio"),
            &schema,
            &identities,
            "binding.serviceRef",
        )
        .unwrap();
        assert!(
            validate_resource_ref_value(
                &json!("audio.d2bus.org.AudioService/missing"),
                &schema,
                &identities,
                "binding.serviceRef",
            )
            .is_err()
        );
        assert!(
            validate_resource_ref_value(
                &json!("Provider/audio"),
                &schema,
                &identities,
                "binding.serviceRef",
            )
            .is_err()
        );
    }

    #[test]
    fn qualified_resource_types_use_generated_envelope_filenames() {
        assert_eq!(
            resource_schema_filename("audio.d2bus.org.AudioService"),
            "audio.d2bus.org_AudioService.schema.json"
        );
    }

    #[test]
    fn malformed_schema_keyword_shapes_and_ref_cycles_fail_integrity_closed() {
        for schema in [
            draft_schema(json!({"allOf": {"type": "string"}})),
            draft_schema(json!({"required": "name"})),
            draft_schema(json!({"$ref": 7})),
            draft_schema(json!({"definitions": [], "type": "object"})),
            draft_schema(json!({"enum": "value"})),
            draft_schema(json!({"x-d2b-reference-kind": "not-a-ref"})),
        ] {
            let error = validate_schema(schema, &json!({}), "value").unwrap_err();
            assert_eq!(error.code, "resource-compiler-schema-integrity-failure");
        }
        let schema = draft_schema(json!({
            "type": "object",
            "definitions": {
                "Loop": {"$ref": "#/definitions/Loop"}
            },
            "properties": {"value": {"$ref": "#/definitions/Loop"}}
        }));
        let error = validate_schema(schema, &json!({"value": {}}), "value").unwrap_err();
        assert_eq!(error.code, "resource-compiler-schema-integrity-failure");
    }

    #[test]
    fn integer_bounds_keep_exact_json_integer_semantics() {
        let schema = draft_schema(json!({
            "type": "integer",
            "maximum": 9007199254740993u64
        }));
        validate_schema(schema.clone(), &json!(9007199254740993u64), "value").unwrap();
        assert!(validate_schema(schema, &json!(9007199254740994u64), "value").is_err());
    }

    #[test]
    fn populated_ref_forms_compile_against_the_closed_identity_set() {
        let mut identities = BTreeSet::new();
        for (resource_type, name) in [
            ("Provider", "audio"),
            ("Guest", "workstation"),
            ("User", "alice"),
            ("Endpoint", "audio"),
            ("Role", "operator"),
            ("ZoneLink", "uplink"),
            ("audio.d2bus.org.AudioService", "host-audio"),
        ] {
            identities.insert((resource_type.to_owned(), name.to_owned()));
        }
        for (reference, allowed) in [
            ("Provider/audio", vec!["Provider"]),
            ("Guest/workstation", vec!["Guest"]),
            ("User/alice", vec!["User"]),
            ("Endpoint/audio", vec!["Endpoint"]),
            ("Role/operator", vec!["Role"]),
            ("ZoneLink/uplink", vec!["ZoneLink"]),
            (
                "audio.d2bus.org.AudioService/host-audio",
                vec!["audio.d2bus.org.AudioService"],
            ),
        ] {
            let schema = generic_ref_test_schema(&allowed);
            validate_resource_ref_value(&json!(reference), &schema, &identities, "fixture.ref")
                .unwrap();
        }
    }

    #[test]
    fn schema_document_depth_budget_runs_before_value_validation() {
        let mut nested = json!({});
        for _ in 0..130 {
            nested = json!({"properties": {"nested": nested}});
        }
        let error = validate_schema_document(&draft_schema(nested)).unwrap_err();
        assert_eq!(error.code, "resource-compiler-schema-integrity-failure");
    }

    fn generic_ref_test_schema(allowed: &[&str]) -> Value {
        let pattern = if allowed.is_empty() {
            "^(?:[A-Z][A-Za-z0-9]{0,62}|[a-z][a-z0-9-]{0,62}\\.d2bus\\.org\\.[A-Z][A-Za-z0-9]{0,62})/[a-z][a-z0-9-]{0,62}$".to_owned()
        } else {
            format!(
                "^(?:{})/[a-z][a-z0-9-]{{0,62}}$",
                allowed
                    .iter()
                    .map(|value| value.replace('.', "\\."))
                    .collect::<Vec<_>>()
                    .join("|")
            )
        };
        json!({
            "type": "string",
            "pattern": pattern,
            "x-d2b-reference-kind": "ResourceRef",
            "x-d2b-reference-scope": "same-zone",
            "x-d2b-allowed-ref-types": allowed
        })
    }

    #[test]
    fn populated_core_and_qualified_resources_validate_together() {
        let resources = vec![
            json!({
                "apiVersion": RESOURCE_API_VERSION,
                "type": "Provider",
                "metadata": {"name": "audio", "zone": "local-root"},
                "spec": {"artifactId": "provider-audio", "config": {}}
            }),
            json!({
                "apiVersion": RESOURCE_API_VERSION,
                "type": "audio.d2bus.org.AudioBinding",
                "metadata": {"name": "binding", "zone": "local-root"},
                "spec": {
                    "providerRef": "Provider/audio",
                    "serviceRef": "audio.d2bus.org.AudioService/service",
                    "grants": {}
                }
            }),
            json!({
                "apiVersion": RESOURCE_API_VERSION,
                "type": "audio.d2bus.org.AudioService",
                "metadata": {"name": "service", "zone": "local-root"},
                "spec": {
                    "providerRef": "Provider/audio",
                    "serviceRole": "authority",
                    "implementationEndpointRefs": [],
                    "operations": []
                }
            }),
        ];
        let input = CompileInput {
            zone: "local-root".to_owned(),
            zone_uid: None,
            resources,
            provider_schema_digests: BTreeMap::new(),
            providers: Vec::new(),
            artifact_catalog_path: None,
            expected_artifact_catalog_digest: None,
            schema_root: Some(schema_root()),
            expected_content_hash: None,
            strict_secrets: false,
        };
        validate_resources(&input, false).unwrap();
    }

    #[test]
    fn display_wayland_resources_validate_against_committed_schemas() {
        let resources = vec![
            json!({
                "apiVersion": RESOURCE_API_VERSION,
                "type": "Guest",
                "metadata": {"name": "acceptance-guest", "zone": "local-root"},
                "spec": {
                    "allowedDomains": ["system"],
                    "budget": {},
                    "defaultDomain": "system",
                    "deviceAttachments": [],
                    "networkAttachments": [],
                    "volumeAttachmentDefaults": []
                }
            }),
            json!({
                "apiVersion": RESOURCE_API_VERSION,
                "type": "Host",
                "metadata": {"name": "host-system", "zone": "local-root"},
                "spec": {
                    "providerRef": "Provider/system-core",
                    "allowedDomains": ["system"],
                    "budget": {},
                    "defaultDomain": "system",
                    "deviceAttachments": [],
                    "networkAttachments": [],
                    "volumeAttachmentDefaults": []
                }
            }),
            json!({
                "apiVersion": RESOURCE_API_VERSION,
                "type": "User",
                "metadata": {"name": "alice", "zone": "local-root"},
                "spec": {
                    "displayName": "Alice",
                    "groups": [],
                    "osUsername": "alice"
                }
            }),
            json!({
                "apiVersion": RESOURCE_API_VERSION,
                "type": "display-wayland.d2bus.org.WaylandPolicy",
                "metadata": {"name": "display-wayland-policy", "zone": "local-root"},
                "spec": {
                    "allowGlobals": [],
                    "denyGlobals": [],
                    "maxVersions": {},
                    "dmabufAllow": [],
                    "dmabufDeny": [],
                    "defaults": {
                        "acceleratedRendering": "allow",
                        "clipboardBoundary": "virtualize",
                        "highRisk": "deny",
                        "appDefaults": "allow",
                        "offDefaults": "deny",
                        "unclassified": "deny"
                    }
                }
            }),
            json!({
                "apiVersion": RESOURCE_API_VERSION,
                "type": "display-wayland.d2bus.org.WaylandSession",
                "metadata": {"name": "acceptance-wayland-session", "zone": "local-root"},
                "spec": {
                    "guestRef": "Guest/acceptance-guest",
                    "hostRef": "Host/host-system",
                    "userRef": "User/alice",
                    "policyRef": "display-wayland.d2bus.org.WaylandPolicy/display-wayland-policy",
                    "identity": {
                        "label": "acceptance-guest",
                        "activeColor": "#7fc8ff",
                        "inactiveColor": "#45475a",
                        "urgentColor": "#f38ba8",
                        "borderEnabled": true,
                        "borderWidth": 9,
                        "labelEnabled": true,
                        "labelText": null,
                        "labelPosition": "top-left"
                    },
                    "crossDomainTrusted": true,
                    "virglVideo": false,
                    "filter": {
                        "debugLogging": false,
                        "denyGlobals": [],
                        "allowGlobals": [],
                        "maxVersions": {},
                        "dmabufAllow": [],
                        "dmabufDeny": []
                    }
                }
            }),
        ];
        let input = CompileInput {
            zone: "local-root".to_owned(),
            zone_uid: None,
            resources,
            provider_schema_digests: BTreeMap::new(),
            providers: Vec::new(),
            artifact_catalog_path: None,
            expected_artifact_catalog_digest: None,
            schema_root: Some(schema_root()),
            expected_content_hash: None,
            strict_secrets: false,
        };
        validate_resources(&input, false).unwrap();
    }
}

fn valid_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn safe_token(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        if (character.is_ascii_graphic() && character != '/' && character != '\\')
            || character == ' '
        {
            output.push(character);
        } else {
            output.push('?');
        }
    }
    bound_ascii(&output)
}

fn bound_ascii(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        let character = if character.is_control() {
            ' '
        } else {
            character
        };
        if !character.is_ascii() || output.len() + 1 > MAX_DIAGNOSTIC_BYTES {
            break;
        }
        output.push(character);
    }
    if output.len() < value.len() && output.len() + 3 <= MAX_DIAGNOSTIC_BYTES {
        output.push_str("...");
    }
    output
}
