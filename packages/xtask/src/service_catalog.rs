//! The per-crate service-to-provider catalog and the generated bus consumer.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;

/// The directory-glob root the per-crate declarations live under.
const PACKAGES_DIR: &str = "packages";
const PROVIDER_PREFIX: &str = "d2b-provider-";
const DECLARATION_FILE: &str = "service-catalog.json";

/// The repository-relative generated artifact path.
pub(crate) const GENERATED_ARTIFACT: &str =
    "packages/d2b-contracts-zone-session/src/generated/service_provider_catalog.rs";

/// One provider crate's service-catalog declaration.

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclarationFile {
    /// The provider identity this crate publishes.
    #[serde(rename = "provider")]
    provider: String,
    /// The canonical `Provider/...` reference of that identity.
    #[serde(rename = "providerRef")]
    provider_ref: String,
    /// The fixed provider resource UID, when one exists (only the bootstrap
    /// system-core provider carries one).
    #[serde(rename = "providerUid", default)]
    provider_uid: Option<String>,
    /// The service packages this provider serves on the session plane, in
    /// declaration order (the committed wire order the bus has consumed).
    #[serde(default)]
    services: Vec<String>,
}

/// The parsed per-crate catalog inputs, crate name -> declaration.
type CatalogRegistry = BTreeMap<String, DeclarationFile>;

/// Run the catalog's gates: declaration sanity, drift, and regeneration
/// idempotence. Wired into the layout check after the crate-layout check.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub fn check(repo_root: &Path) -> Result<(), String> {
    let registry = load(repo_root)?;
    let errors = declaration_errors(&registry);
    if !errors.is_empty() {
        return Err(format!(
            "service-catalog declaration violations:\n- {}",
            errors.join("\n- ")
        ));
    }
    let rendered = render(&registry)?;
    let artifact_path = generated_artifact_path(repo_root);
    let on_disk = fs::read_to_string(&artifact_path).map_err(|_| {
        format!(
            "service-catalog artifact is missing at {}; run `cargo xtask check-provider-crate-layout --fix`",
            artifact_path.display()
        )
    })?;
    if on_disk != rendered {
        return Err(format!(
            "service-catalog drift: the committed generated artifact {} differs from the declarations' output; a hand edit or a stale generation must be repaired by `cargo xtask check-provider-crate-layout --fix`",
            artifact_path.display()
        ));
    }
    let rendered_again = render(&registry)?;
    if rendered_again != rendered {
        return Err("service-catalog regeneration is not idempotent".to_owned());
    }
    Ok(())
}

/// Regenerate the service-to-provider catalog artifact from the declarations.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub fn regenerate(repo_root: &Path) -> Result<Vec<PathBuf>, String> {
    let registry = load(repo_root)?;
    let errors = declaration_errors(&registry);
    if !errors.is_empty() {
        return Err(format!(
            "refusing to regenerate the service catalog while the declaration gate fails:\n- {}",
            errors.join("\n- ")
        ));
    }
    let rendered = render(&registry)?;
    let artifact_path = generated_artifact_path(repo_root);
    if let Some(parent) = artifact_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "cannot create the generated-artifact directory {}: {error}",
                parent.display()
            )
        })?;
    }
    fs::write(&artifact_path, &rendered).map_err(|error| {
        format!(
            "cannot write the generated service catalog {}: {error}",
            artifact_path.display()
        )
    })?;
    Ok(vec![artifact_path])
}

/// Read every provider crate's declaration file into a crate-keyed map.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn load(repo_root: &Path) -> Result<CatalogRegistry, String> {
    let packages_dir = repo_root.join(PACKAGES_DIR);
    let mut out = CatalogRegistry::new();
    let entries = fs::read_dir(&packages_dir).map_err(|error| {
        format!("cannot read {}: {error}", packages_dir.display())
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read a packages entry: {error}"))?;
        let crate_name = entry.file_name().to_string_lossy().into_owned();
        if !crate_name.starts_with(PROVIDER_PREFIX) {

            continue;
        }
        let declaration_path = entry.path().join(DECLARATION_FILE);
        if !declaration_path.is_file() {
            continue;
        }
        let text = fs::read_to_string(&declaration_path).map_err(|error| {
            format!("cannot read the declaration {}: {error}", declaration_path.display())
        })?;
        let file: DeclarationFile = serde_json::from_str(&text).map_err(|error| {
            format!(
                "malformed declaration {}: {error}",
                declaration_path.display()
            )
        })?;
        out.insert(crate_name, file);
    }
    Ok(out)
}

/// The declaration sanity violations:
///
/// - a provider identity that does not match its owning crate's name (the
///   suffix after `d2b-provider-` must be the declared provider identity);
/// - a provider ref that does not name the declared provider identity;
/// - a service package declared by two crates;
/// - a non-bootstrap provider declaring a fixed UID (the only fixed UID
///   belongs to system-core).
fn declaration_errors(registry: &CatalogRegistry) -> Vec<String> {
    let mut errors = Vec::new();
    let mut services: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (crate_name, file) in registry {
        let expected_provider = crate_name
            .strip_prefix(PROVIDER_PREFIX)
            .unwrap_or(crate_name.as_str());
        if file.provider != expected_provider {



            errors.push(format!(
                "provider-mismatch: {} declares provider \"{}\" but its crate name implies \"{expected_provider}\"",
                crate_name, file.provider
            ));
        }
        let expected_ref = format!("Provider/{}", file.provider);
        if file.provider_ref != expected_ref {
            errors.push(format!(
                "provider-ref-mismatch: {} declares provider ref \"{}\" but its provider identity implies \"{expected_ref}\"",
                crate_name, file.provider_ref
            ));
        }
        if file.provider_uid.is_some() && file.provider != "system-core" {
            errors.push(format!(
                "unexpected-provider-uid: {} declares a fixed provider UID; only the bootstrap system-core provider may carry one",
                crate_name
            ));
        }
        for service in &file.services {
            services.entry(service.as_str()).or_default().push(crate_name.as_str());
        }
    }
    for (service, crates) in &services {
        if crates.len() > 1 {
            errors.push(format!(
                "service-declared-twice: {service} is declared by both {}and {}",
                crates[0], crates[1]
            ));
        }
    }
    errors
}

/// Emit the generated service-to-provider catalog artifact text.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn render(registry: &CatalogRegistry) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("// @generated\n");
    out.push_str("// Provenance:emitted from the per-crate `service-catalog.json` declarations\n");
    out.push_str("// by `cargo xtask check-provider-crate-layout --fix`;the layout check's\n");
    out.push_str("// drift gate regenerates this file byte-for-byte,and refuses a hand edit.\n");
    out.push('\n');
    out.push_str("// Generated service-to-provider vocabulary for the zone-plane session\n");
    out.push_str("// contract. The bus is pinned provider-free,so it reads the provider refs\n");
    out.push_str("// here instead of depending on the owning crates' constants. The daemon's\n");
    out.push_str("// composition reads the same catalog rather than restating a hand table.\n");
    out.push('\n');
    let bootstrap = registry
        .get("d2b-provider-system-core")
        .ok_or_else(|| "service-catalog:the bootstrap system-core provider has no declaration".to_owned())?;
    let bootstrap_ref = &bootstrap.provider_ref;
    let bootstrap_uid = bootstrap
        .provider_uid

        .as_ref()
        .ok_or_else(|| "service-catalog:the bootstrap system-core provider declares no providerUid".to_owned())?;
    out.push_str("/// The fixed bootstrap Provider reference (the system-core Provider).\n");
    out.push_str(&format!("pub const BOOTSTRAP_PROVIDER_REF: &str = \"{bootstrap_ref}\";\n"));
    out.push('\n');
    out.push_str("/// The fixed bootstrap Provider resource UID.\n");
    out.push_str(&format!("pub const BOOTSTRAP_PROVIDER_UID: &str = \"{bootstrap_uid}\";\n"));
    out.push('\n');
    out.push_str("/// The provider reference one declared provider identity publishes.\n");
    out.push_str("pub fn provider_ref(identity: &str) -> Option<&'static str> {\n");
    out.push_str("    match identity {\n");
    let mut identities = registry.keys().collect::<Vec<_>>();
    identities.sort();
    for crate_name in identities {
        let file = &registry[crate_name];
        out.push_str(&format!(
            "        \"{}\" => Some(\"{}\"),\n",
            file.provider, file.provider_ref
        ));
    }
    out.push_str("        _ => None,\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out.push('\n');
    out.push_str("/// The provider reference that serves one closed service package\n");
    out.push_str("/// on the zone-plane session contract, when a fixed provider serves it.\n");
    out.push_str("pub fn provider_ref_for_service(service: &str) -> Option<&'static str> {\n");
    out.push_str("    match service {\n");
    let mut service_rows: Vec<(&String, &String)> = Vec::new();
    for file in registry.values() {
        for service in &file.services {
            service_rows.push((service, &file.provider_ref));
        }
    }
    service_rows.sort_by(|left, right| left.0.cmp(right.0));
    for (service, provider_ref) in service_rows {
        out.push_str(&format!("        \"{service}\" => Some(\"{provider_ref}\"),\n"));
    }
    out.push_str("        _ => None,\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    Ok(out)
}

fn generated_artifact_path(repo_root: &Path) -> PathBuf {

    repo_root.join(GENERATED_ARTIFACT)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A typo'd declaration key is refused at the boundary instead of being
    /// silently ignored (the daemon's fixed-UID row would otherwise vanish).
    #[test]
    fn an_unknown_declaration_key_is_refused() {
        let error = serde_json::from_str::<DeclarationFile>(
            r#"{"provider":"system-core","providerRef":"Provider/system-core","providerUid":"fixed","providerUidTypo":"fixed"}"#,
        )
        .err()
        .expect("an unknown declaration key is refused");
        assert!(error.to_string().contains("providerUidTypo"), "{error}");
    }
}
