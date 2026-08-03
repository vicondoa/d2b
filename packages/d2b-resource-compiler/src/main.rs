//! Deterministic build-time entry point for the Zone resource compiler.
//!
//! The CLI deliberately accepts one declared JSON input and one declared
//! output. It does not discover artifacts, walk a package search path, or
//! invoke a second validation implementation. Provider layout and signature
//! checks are delegated to the library's anchored Linux implementation; this
//! file owns only the resource-bundle envelope and the input/output contract.

use std::{
    collections::{BTreeMap, BTreeSet},
    env, fs,
    path::{Path, PathBuf},
    process::ExitCode,
};

use d2b_contracts::v3::{
    ArtifactDigest, ArtifactId, CanonicalJsonValue, ProviderManifest, canonical_json_bytes,
    framed_canonical_digest,
};
use d2b_resource_compiler::{
    ArtifactCatalogEntry, CatalogDigests, Diagnostic, StaticPublisherKeys, compile_linux_artifact,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

const RESOURCE_BUNDLE_DOMAIN_TAG: &str = "d2b:v3:resource-bundle";
const MAX_DIAGNOSTIC_BYTES: usize = d2b_resource_compiler::MAX_DIAGNOSTIC_BYTES;
const MAX_RESOURCES: usize = 4096;
const MAX_RESOURCE_BYTES: usize = 512 * 1024;
const MAX_SCHEMA_BYTES: usize = 8 * 1024 * 1024;

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
        if strict_secrets && contains_secret_shape(resource) {
            return Err(CliError::new(
                "resource-compiler-inline-secret",
                format!(
                    "resource {} contains inline secret-shaped material",
                    safe_token(name)
                ),
            ));
        }
        if let Some(schema) = schema_cache.schema(resource_type)? {
            let spec = object
                .get("spec")
                .cloned()
                .unwrap_or(Value::Object(Map::new()));
            let spec_schema = schema
                .get("properties")
                .and_then(Value::as_object)
                .and_then(|properties| properties.get("spec"))
                .ok_or_else(|| {
                    CliError::new(
                        "resource-compiler-schema-invalid",
                        format!("schema for ResourceType {resource_type} has no spec schema"),
                    )
                })?;
            validate_schema_from_root(
                &schema,
                spec_schema,
                &spec,
                &format!("{resource_type}/{name}.spec"),
            )?;
        }
    }
    Ok(())
}

struct SchemaCache {
    root: Option<PathBuf>,
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
        })
    }

    fn schema(&self, resource_type: &str) -> Result<Option<Value>, CliError> {
        let Some(root) = &self.root else {
            return Ok(None);
        };
        if !valid_schema_name(resource_type) {
            return Err(CliError::new(
                "resource-compiler-resource-type-invalid",
                "resource type is not a supported schema name",
            ));
        }
        let path = root.join(format!("{resource_type}.schema.json"));
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
        serde_json::from_slice(&bytes).map(Some).map_err(|_| {
            CliError::new(
                "resource-compiler-schema-invalid",
                "declared ResourceType schema is not valid JSON",
            )
        })
    }
}

fn validate_schema(schema: Value, value: &Value, path: &str) -> Result<(), CliError> {
    let root = schema.clone();
    validate_schema_from_root(&root, &schema, value, path)
}

fn validate_schema_from_root(
    root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
) -> Result<(), CliError> {
    validate_schema_root(root, schema, value, path)
}

fn validate_schema_root(
    root: &Value,
    schema: &Value,
    value: &Value,
    path: &str,
) -> Result<(), CliError> {
    if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
        let Some(name) = reference.strip_prefix("#/definitions/") else {
            return Err(schema_error(
                path,
                "contains an unsupported schema reference",
            ));
        };
        let definition = root
            .get("definitions")
            .and_then(Value::as_object)
            .and_then(|definitions| definitions.get(name))
            .ok_or_else(|| schema_error(path, "references a missing schema definition"))?;
        return validate_schema_root(root, definition, value, path);
    }
    if let Some(expected) = schema.get("type") {
        let expected = expected
            .as_str()
            .map(|value| vec![value.to_owned()])
            .or_else(|| {
                expected.as_array().map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
            })
            .unwrap_or_default();
        if !expected.iter().any(|kind| value_matches(kind, value)) {
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
            && !simple_pattern_match(pattern, value)
        {
            return Err(schema_error(path, "does not match the schema pattern"));
        }
        if let Some(minimum) = schema.get("minLength").and_then(Value::as_u64)
            && value.len() < minimum as usize
        {
            return Err(schema_error(path, "is shorter than the schema minimum"));
        }
        if let Some(maximum) = schema.get("maxLength").and_then(Value::as_u64)
            && value.len() > maximum as usize
        {
            return Err(schema_error(path, "is longer than the schema maximum"));
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
                validate_schema_root(root, item_schema, item, &format!("{path}.{index}"))?;
            }
        }
    }
    if let Some(value) = value.as_object() {
        let properties = schema
            .get("properties")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        if schema.get("additionalProperties") == Some(&Value::Bool(false))
            && value.keys().any(|key| !properties.contains_key(key))
        {
            return Err(schema_error(path, "contains an undeclared field"));
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
                validate_schema_root(root, property_schema, value, &format!("{path}.{key}"))?;
            }
        }
    }
    for (branch_name, expected_count) in [("anyOf", 1_usize), ("oneOf", 1_usize)] {
        if let Some(branches) = schema.get(branch_name).and_then(Value::as_array) {
            let passed = branches
                .iter()
                .filter(|branch| validate_schema_root(root, branch, value, path).is_ok())
                .count();
            if passed != expected_count {
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

fn simple_pattern_match(pattern: &str, value: &str) -> bool {
    // The committed schemas use the bounded name/ref patterns. Keeping this
    // parser deliberately small avoids adding a second regex dependency to
    // the compiler's deterministic build surface.
    if pattern == "^[a-z][a-z0-9-]{0,62}$" {
        return valid_name(value);
    }
    if pattern == "^sha256:[0-9a-f]{64}$" {
        return ArtifactDigest::parse(value.to_owned()).is_ok();
    }
    value.contains(pattern.trim_matches('^').trim_matches('$'))
}

fn schema_error(path: &str, reason: &str) -> CliError {
    CliError::new(
        "resource-compiler-schema-invalid",
        format!("{} {}", safe_token(path), reason),
    )
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
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| forbidden_key(key) || contains_secret_shape(value)),
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

fn valid_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes[0].is_ascii_lowercase()
        && bytes[1..]
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-')
}

fn valid_schema_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    valid_name(value)
        || value.contains(".d2bus.org.")
        || (!bytes.is_empty()
            && bytes.len() <= 63
            && bytes[0].is_ascii_uppercase()
            && bytes[1..].iter().all(|byte| byte.is_ascii_alphanumeric()))
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
