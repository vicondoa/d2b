//! The per-crate session service catalog and the generated bus consumer.
//!
//! Every `packages/d2b-provider-*/service-catalog.json` declares the session
//! routing facts one crate publishes: the service packages its session
//! identity answers, and the fixed bootstrap resource UID when the catalog
//! row is the deployment's bootstrap row. The Provider identity the row
//! routes to is not stated there: it is the crate's session identity, read
//! from the per-crate identity authority
//! (`packages/d2b-provider-*/provider-identity.json`).
//!
//! This module joins the two, aggregates them into the catalog the
//! zone-plane session contract compiles (`include!`d by
//! `packages/d2b-contracts-zone-session/src/v3/mod.rs` straight out of the
//! staged `generated/new-graph/` closure), runs the declaration sanity gate,
//! and owns the drift and idempotence gates over the committed byte.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use crate::authority_common::{declaration_paths, Declaration};
use crate::provider_identity_authority::{ProviderIdentities, Role, Surface};

#[cfg(test)]
use d2b_contracts_provider::v3::projection::PrivatePlanProjection;
use serde::Deserialize;

/// The repository-relative generated artifact path: the staged closure copy
/// the zone-session contract `include!`s, so the declaration render and the
/// compiled production bytes are the same file rather than two.
pub(crate) const GENERATED_ARTIFACT: &str =
    "generated/new-graph/service_provider_catalog.rs";

/// One provider crate's service-catalog declaration.
///
/// The declaration is the crate's session routing surface: the closed
/// service packages its session identity answers, and the fixed bootstrap
/// resource UID the deployment's bootstrap row is committed under. The
/// identity those rows are routed to is the crate's session identity in the
/// identity authority, never a string composed here.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DeclarationFile {
    /// The fixed provider resource UID, when this catalog row is the
    /// bootstrap row. Exactly one row may carry one.
    #[serde(rename = "providerUid", default)]
    provider_uid: Option<String>,
    /// The service packages this provider serves on the session plane, in
    /// declaration order (the committed wire order the bus has consumed).
    #[serde(default)]
    services: Vec<String>,
}

/// The service catalog a declaration projection produces (KTD1/U4).
///
/// The rows are the declared services and the provider that answers each one,
/// derived from the declaration. The per-crate `service-catalog.json` files
/// this module still reads are the pre-declaration authoring form; the
/// unchanged production entry point keeps consuming them until the cutover,
/// and this renderer is what it will consume instead.
#[cfg(test)]
pub(crate) fn render_declaration_service_catalog(plan: &PrivatePlanProjection) -> String {
    let mut out = String::new();
    out.push_str("// @generated\n");
    out.push_str("// Provenance: derived from the provider declaration (KTD1/U4). A\n");
    out.push_str("// generated artifact is an output, not a second source.\n");
    out.push('\n');
    for row in plan.services() {
        out.push_str(&format!(
            "    {:?} => Some({:?}),\n",
            row.service_id(),
            row.provider_ref()
        ));
    }
    out
}

/// The parsed per-crate catalog inputs, crate name -> declaration.
type CatalogRegistry = BTreeMap<String, DeclarationFile>;

/// Render the service-to-provider catalog from the routing declarations
/// joined against the identity authority.
///
/// `gen-new-graph` renders its committed new-graph projection through this
/// entry point. The declaration sanity gate runs first, so a staged catalog
/// can never carry a service two crates claim or a routing row whose crate
/// owns no session identity, and nothing here reads a crate source.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub(crate) fn render_declarations_only(repo_root: &Path) -> Result<String, String> {
    let registry = load(repo_root)?;
    let identities = ProviderIdentities::load(repo_root)?;
    let errors = declaration_errors(&registry, &identities);
    if !errors.is_empty() {
        return Err(format!(
            "service-catalog declaration violations:\n- {}",
            errors.join("\n- ")
        ));
    }
    render(&registry, &identities)
}

/// Run the catalog's gates: declaration sanity, drift, and regeneration
/// idempotence. Wired into the layout check after the crate-layout check.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub fn check(repo_root: &Path) -> Result<(), String> {
    let registry = load(repo_root)?;
    let identities = ProviderIdentities::load(repo_root)?;
    let errors = declaration_errors(&registry, &identities);
    if !errors.is_empty() {
        return Err(format!(
            "service-catalog declaration violations:\n- {}",
            errors.join("\n- ")
        ));
    }
    let rendered = render(&registry, &identities)?;
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
    let rendered_again = render(&registry, &identities)?;
    if rendered_again != rendered {
        return Err("service-catalog regeneration is not idempotent".to_owned());
    }
    Ok(())
}

/// Regenerate the service-to-provider catalog artifact from the declarations.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub fn regenerate(repo_root: &Path) -> Result<Vec<PathBuf>, String> {
    let registry = load(repo_root)?;
    let identities = ProviderIdentities::load(repo_root)?;
    let errors = declaration_errors(&registry, &identities);
    if !errors.is_empty() {
        return Err(format!(
            "refusing to regenerate the service catalog while the declaration gate fails:\n- {}",
            errors.join("\n- ")
        ));
    }
    let rendered = render(&registry, &identities)?;
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
///
/// The crate set is [`declaration_paths`]', which refuses a provider crate
/// carrying no catalog rather than dropping it: a renamed
/// `service-catalog.json` would otherwise remove its crate's service packages
/// from the generated catalog with the drift gate still green over the
/// smaller input.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn load(repo_root: &Path) -> Result<CatalogRegistry, String> {
    let mut out = CatalogRegistry::new();
    for (crate_name, declaration_path) in declaration_paths(repo_root, Declaration::ServiceCatalog)?
    {
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
/// - a catalog row whose crate owns no session identity;
/// - a service package two crates declare;
/// - no catalog row carrying the fixed bootstrap resource UID, more than one
///   carrying it, or one whose crate owns no fixed-bootstrap identity.
///
/// The identity a routing row points at is not compared against the crate's
/// directory name, and it is not stated in the catalog at all: a crate is
/// named for the family it realizes and the identity it publishes is a
/// separate fact - `d2b-provider-guest-qemu-media` publishes
/// `runtime-qemu-media` - so a directory-name comparison published
/// references the product does not have and made the ones it does
/// inexpressible. What ties a row to an identity is the join below: the
/// routing row resolves through the crate's session identity, and a row that
/// resolves to nothing is a refusal rather than a row with a guessed
/// reference.
fn declaration_errors(
    registry: &CatalogRegistry,
    identities: &ProviderIdentities,
) -> Vec<String> {
    let mut errors = Vec::new();
    let mut services: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for (crate_name, file) in registry {
        if identities.identity(crate_name, Surface::Session).is_none() {
            errors.push(format!(
                "session-identity-missing: crate {crate_name} publishes a session service catalog but owns no session Provider identity; the identity a routing row points at is the crate's own session identity in its `provider-identity.json`"
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
    errors.extend(bootstrap_errors(registry, identities));
    errors
}

/// The bootstrap-row violations.
///
/// The deployment's fixed bootstrap Provider is one row: the catalog row
/// that states its fixed resource UID. That row's crate must own a
/// fixed-bootstrap identity, because a bootstrap registration the identity
/// authority does not classify as one is a row the deployment would start
/// outside the fixed-bootstrap set.
fn bootstrap_errors(
    registry: &CatalogRegistry,
    identities: &ProviderIdentities,
) -> Vec<String> {
    let mut errors = Vec::new();
    let carriers: Vec<&String> = registry
        .iter()
        .filter(|(_, file)| file.provider_uid.is_some())
        .map(|(crate_name, _)| crate_name)
        .collect();
    match carriers.as_slice() {
        [] => errors.push(
            "bootstrap-row-missing: no service catalog declares a fixed provider UID; the deployment's bootstrap Provider resource has no UID to commit under"
                .to_owned(),
        ),
        [crate_name] => {
            if !identities.has_role(crate_name, Role::FixedBootstrap) {
                errors.push(format!(
                    "unexpected-bootstrap-row: crate {crate_name} declares a fixed provider UID but its own identity declaration claims no fixed-bootstrap identity"
                ));
            }
        }
        [first, second, ..] => errors.push(format!(
            "bootstrap-row-declared-twice: crates {first} and {second} both declare a fixed provider UID; the deployment's bootstrap Provider resource has one UID"
        )),
    }
    errors
}

/// Emit the generated service-to-provider catalog artifact text.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn render(registry: &CatalogRegistry, identities: &ProviderIdentities) -> Result<String, String> {
    let mut out = String::new();
    out.push_str("// @generated\n");
    out.push_str("// Provenance:emitted from the per-crate `service-catalog.json` routing\n");
    out.push_str("// facts joined against the per-crate `provider-identity.json` session\n");
    out.push_str("// identities by `cargo xtask check-provider-crate-layout --fix`;the layout\n");
    out.push_str("// check's drift gate regenerates this file byte-for-byte,and refuses a hand\n");
    out.push_str("// edit.\n");
    out.push('\n');
    out.push_str("// Generated service-to-provider vocabulary for the zone-plane session\n");
    out.push_str("// contract. The bus is pinned provider-free,so it reads the provider refs\n");
    out.push_str("// here instead of depending on the owning crates' constants. The daemon's\n");
    out.push_str("// composition reads the same catalog rather than restating a hand table.\n");
    out.push('\n');
    let (bootstrap_crate, bootstrap) = registry
        .iter()
        .find(|(_, file)| file.provider_uid.is_some())
        .ok_or_else(|| {
            "service-catalog:no catalog row declares the fixed bootstrap provider UID".to_owned()
        })?;
    let bootstrap_identity = identities
        .identity(bootstrap_crate, Surface::Session)
        .ok_or_else(|| {
            format!("service-catalog:the bootstrap row's crate {bootstrap_crate} owns no session identity")
        })?;
    let bootstrap_ref = ProviderIdentities::provider_ref(bootstrap_identity);
    let bootstrap_uid = bootstrap.provider_uid.as_ref().ok_or_else(|| {
        "service-catalog:the bootstrap catalog row declares no providerUid".to_owned()
    })?;
    out.push_str(&format!(
        "/// The fixed bootstrap Provider reference (the {bootstrap_identity} Provider).\n"
    ));
    out.push_str(&format!("pub const BOOTSTRAP_PROVIDER_REF: &str = \"{bootstrap_ref}\";\n"));
    out.push('\n');
    out.push_str("/// The fixed bootstrap Provider resource UID.\n");
    out.push_str(&format!("pub const BOOTSTRAP_PROVIDER_UID: &str = \"{bootstrap_uid}\";\n"));
    out.push('\n');
    out.push_str("/// The provider reference one declared provider identity publishes.\n");
    out.push_str("pub fn provider_ref(identity: &str) -> Option<&'static str> {\n");
    out.push_str("    match identity {\n");
    for (_, identity) in identities.identities(Surface::Session) {
        let identity_ref = ProviderIdentities::provider_ref(identity);
        out.push_str(&format!("        \"{identity}\" => Some(\"{identity_ref}\"),\n"));
    }
    out.push_str("        _ => None,\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out.push('\n');
    out.push_str("/// The provider reference that serves one closed service package\n");
    out.push_str("/// on the zone-plane session contract, when a fixed provider serves it.\n");
    out.push_str("pub fn provider_ref_for_service(service: &str) -> Option<&'static str> {\n");
    out.push_str("    match service {\n");
    let mut service_rows: Vec<(&String, String)> = Vec::new();
    for (crate_name, file) in registry {
        let Some(identity) = identities.identity(crate_name, Surface::Session) else {
            continue;
        };
        for service in &file.services {
            service_rows.push((service, ProviderIdentities::provider_ref(identity)));
        }
    }
    service_rows.sort_by(|left, right| left.0.cmp(right.0));
    for (service, provider_ref) in &service_rows {
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

    /// The service catalog is the declared services and their providers, not
    /// the per-crate `service-catalog.json` files the production entry point
    /// reads.
    #[test]
    fn the_service_catalog_is_derived_from_the_declaration() {
        let plan = crate::resource_type_authority::declaration_fixture::plan(&["export", "close"]);
        let rendered = render_declaration_service_catalog(&plan);
        assert_eq!(
            rendered,
            render_declaration_service_catalog(&plan),
            "the catalog is byte-stable"
        );
        assert!(
            rendered.contains("\"volume-virtiofs.d2bus.org/export\" => Some(\"Provider/provider-volume-virtiofs\")"),
            "the declared service routes to its declaring provider: {rendered}"
        );
        assert!(
            !rendered.contains("d2b-provider-volume-local"),
            "no per-crate declaration file contributed a row: {rendered}"
        );
    }

    /// A typo'd declaration key is refused at the boundary instead of being
    /// silently ignored (the daemon's fixed-UID row would otherwise vanish).
    #[test]
    fn an_unknown_declaration_key_is_refused() {
        let error = serde_json::from_str::<DeclarationFile>(
            r#"{"providerUid":"fixed","providerUidTypo":"fixed"}"#,
        )
        .err()
        .expect("an unknown declaration key is refused");
        assert!(error.to_string().contains("providerUidTypo"), "{error}");
    }

    /// An identity field left behind in a routing declaration is refused
    /// rather than silently ignored: the identity a row routes to is the
    /// crate's session identity in the identity authority, so a declaration
    /// that states one would be a second authority for the same fact.
    #[test]
    fn a_routing_declaration_naming_an_identity_is_refused() {
        for declaration in [
            r#"{"provider":"system-core"}"#,
            r#"{"providerRef":"Provider/system-core"}"#,
            r#"{"provider":"system-core","providerRef":"Provider/system-core"}"#,
        ] {
            assert!(
                serde_json::from_str::<DeclarationFile>(declaration).is_err(),
                "a routing declaration must not carry an identity: {declaration}"
            );
        }
    }
}
