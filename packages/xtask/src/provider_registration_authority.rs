//! The per-crate provider/service registrations and the generated
//! registration table.
//!
//! Every `packages/d2b-provider-*/registrations.json` declares the
//! effect-service ids one crate's family registers with the daemon's
//! composition root. The Provider identity that family registers under is
//! not stated here: it is the crate's own runtime identity, read from the
//! per-crate identity authority
//! (`packages/d2b-provider-*/provider-identity.json`), so a crate's directory
//! name is never the identity it registers. This module:
//!
//! - joins every registration row against the identity authority, refusing a
//!   crate that declares services but owns no runtime identity and one crate
//!   naming an identity another crate already owns;
//! - aggregates the joined rows into the `PROVIDER_REGISTRATIONS` table the
//!   daemon composition root composes (`include!`d from
//!   `packages/d2bd/src/resource_plane_v3.rs` straight out of the staged
//!   `generated/new-graph/` closure, so the declaration render and the
//!   compiled production bytes are one committed file), so a new family is
//!   registered without the daemon naming it - a lane that declares its
//!   family in the crate needs no daemon edit and no layout-ratchet row;
//! - runs the declaration-to-source parity gate: a declared service must be
//!   spelled in the crate's sources (a `ServiceDecl` const), every service
//!   the crate's sources spell or its descriptor registers must be declared,
//!   and a service declared by two crates fails naming both;
//! - owns the drift gate over the generated artifact: a hand edit fails
//!   and regeneration is idempotent.
//!
//! The generator is wired into the existing policy check
//! (`cargo xtask check-provider-crate-layout`): the check runs the parity
//! and drift gates after the crate-layout check, and `--fix` regenerates the
//! artifact. This keeps every gate in the suite the layout check already
//! runs, without editing `provider_crate_policy.rs`.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use crate::authority_common::{collect_rs_files, declaration_paths, verify_committed, Declaration};
use crate::provider_identity_authority::{ProviderIdentities, Surface};
#[cfg(test)]
use d2b_contracts_provider::v3::projection::PrivatePlanProjection;
use serde::Deserialize;

/// The directory-glob root the per-crate declarations live under.
const PACKAGES_DIR: &str = "packages";

/// The repository-relative generated artifact path: the staged closure copy
/// the daemon's composition root `include!`s, so the declaration render and
/// the compiled production bytes are the same file rather than two.
pub(crate) const GENERATED_ARTIFACT: &str =
    "generated/new-graph/provider_registrations.rs";

/// One provider crate's registration declaration.
///
/// The declaration is the crate's executable registration surface: the
/// effect-service ids its family registers. The Provider identity the row
/// registers under is the crate's runtime identity in the identity
/// authority, joined below; a service the crate does not spell in its own
/// sources fails the parity gate the same way.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistrationDeclaration {
    #[serde(rename = "crate")]
    crate_name: String,
    services: Vec<String>,
}

/// Run the authority's gates: parity, drift, and regeneration idempotence,
/// over the generated registration table.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub fn check(repo_root: &Path) -> Result<(), String> {
    let errors = parity_errors(repo_root)?;
    if !errors.is_empty() {
        return Err(format!(
            "provider-registration parity violations:\n- {}",
            errors.join("\n- ")
        ));
    }
    let rendered = render(repo_root)?;
    verify_committed(repo_root, GENERATED_ARTIFACT, &rendered, "provider-registration")?;
    let rendered_again = render(repo_root)?;
    if rendered_again != rendered {
        return Err("provider-registration regeneration is not idempotent".to_owned());
    }
    Ok(())
}

/// Regenerate the registration table from the declarations.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub fn regenerate(repo_root: &Path) -> Result<Vec<PathBuf>, String> {
    let errors = parity_errors(repo_root)?;
    if !errors.is_empty() {
        return Err(format!(
            "refusing to regenerate the provider registration table while the parity gate fails:\n- {}",
            errors.join("\n- ")
        ));
    }
    let rendered = render(repo_root)?;
    let artifact_path = repo_root.join(GENERATED_ARTIFACT);
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
            "cannot write the generated artifact {}: {error}",
            artifact_path.display()
        )
    })?;
    Ok(vec![artifact_path])
}

/// The provider registration rows a declaration projection produces.
///
/// The row is the composition root's view of one declaration: the identities a
/// daemon registers without naming the family. It is derived, so adding a
/// provider that uses existing primitives needs no handwritten registration.
#[cfg(test)]
pub(crate) fn render_declaration_registrations(plan: &PrivatePlanProjection) -> String {
    let mut out = String::new();
    out.push_str("// @generated\n");
    out.push_str("// Provenance: derived from the provider declaration (KTD1/U4). A\n");
    out.push_str("// generated artifact is an output, not a second source.\n");
    for row in plan.registrations() {
        out.push_str("ProviderRegistrationRow {\n");
        out.push_str(&format!("    artifact_id: {:?},\n", row.artifact_id()));
        out.push_str(&format!("    provider_ref: {:?},\n", row.provider_ref()));
        out.push_str(&format!(
            "    components: &[{}],\n",
            row.components()
                .iter()
                .map(|component| format!("{component:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push_str(&format!(
            "    resource_types: &[{}],\n",
            row.resource_types()
                .iter()
                .map(|resource_type| format!("{resource_type:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push_str(&format!(
            "    services: &[{}],\n",
            row.services()
                .iter()
                .map(|service| format!("{service:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push_str(&format!(
            "    methods: &[{}],\n",
            row.methods()
                .iter()
                .map(|method| format!("{method:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
        out.push_str("}\n");
    }
    out
}

/// The registration violations: a crate that declares services but owns no
/// runtime identity, a declared service the crate's sources do not spell, a
/// service the crate's sources spell or its descriptor registers that the
/// declaration omits, and a service two crates declare.
///
/// The Provider identity a row registers under is not decided here: it is the
/// crate's runtime identity in the identity authority, which already gates
/// the resource-name grammar, the deliberate cross-surface repetition, and
/// one identity naming one crate. What this gate adds is the join - a
/// registration row is a runtime registration, so its crate must own a
/// runtime identity - and the service facts, which only the registration
/// declaration and the crate's own sources speak.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn parity_errors(repo_root: &Path) -> Result<Vec<String>, String> {
    let declarations = load_declarations(repo_root)?;
    let identities = ProviderIdentities::load(repo_root)?;
    let mut errors = Vec::new();
    let mut services: BTreeMap<&str, &str> = BTreeMap::new();
    for (crate_name, declaration) in &declarations {
        if identities.identity(crate_name, Surface::Runtime).is_none() {
            errors.push(format!(
                "runtime-identity-missing: crate {crate_name} declares a provider registration but owns no runtime Provider identity; the identity a row registers under is the crate's own runtime identity in its `provider-identity.json`"
            ));
        }
        let src_dir = repo_root.join(PACKAGES_DIR).join(crate_name).join("src");
        let mut text = String::new();
        for path in collect_rs_files(&src_dir)? {
            text.push_str(
                &fs::read_to_string(&path)
                    .map_err(|error| format!("cannot read {}: {error}", path.display()))?,
            );
            text.push('\n');
        }
        let consts = service_decl_consts(&text);
        let spelled: BTreeSet<String> = consts.values().cloned().collect();
        let registered = descriptor_service_ids(&text, &consts);
        let declared: BTreeSet<String> = declaration.services.iter().cloned().collect();
        for service in &declaration.services {
            // The declared service must be spelled in the crate's own
            // sources: the `ServiceDecl` const is the crate's declaration
            // surface, so a declaration the crate cannot serve is a parity
            // violation naming both.
            if !spelled.contains(service) {
                errors.push(format!(
                    "declared-but-not-spelled: crate {crate_name} declares service {service} but its sources define no ServiceDecl with that id"
                ));
            }
            if !registered.contains(service) {
                errors.push(format!(
                    "declared-but-not-registered: crate {crate_name} declares service {service} but its descriptor registers no such service"
                ));
            }
            if let Some(first) = services.insert(service, crate_name) {
                errors.push(format!(
                    "service-duplicate: service {service} is declared by both {first} and {crate_name}"
                ));
            }
        }
        // Every service the crate's sources spell must be declared: a
        // `ServiceDecl` const the declaration omits would be a service the
        // registration table does not carry.
        for service in &spelled {
            if !declared.contains(service) {
                errors.push(format!(
                    "spelled-but-not-declared: crate {crate_name}'s sources define ServiceDecl {service} but its registration omits it"
                ));
            }
        }
        // Every service the crate's descriptor registers must be declared:
        // a served service the declaration omits would be a declared
        // service with no registered factory at the composition point.
        for service in &registered {
            if !declared.contains(service) {
                errors.push(format!(
                    "registered-but-not-declared: crate {crate_name}'s descriptor registers service {service} but its registration omits it"
                ));
            }
        }
    }
    Ok(errors)
}

/// Every `pub const NAME: ServiceDecl = ServiceDecl { id: "ID", ... };` in
/// one crate's sources, as an ident -> id map.
///
/// The scan is line-based so an unrelated `pub const` (a string constant, a
/// `const fn`) cannot consume a later `ServiceDecl` header: only a line that
/// opens a `ServiceDecl` block starts an entry, and the entry's id is read
/// from the block's own `id:` field before its closing `};`.
fn service_decl_consts(text: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    let lines: Vec<&str> = text.lines().collect();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index].trim();
        let Some(rest) = line.strip_prefix("pub const ") else {
            index += 1;
            continue;
        };
        let Some((ident, _)) = rest.split_once(": ServiceDecl = ServiceDecl {") else {
            index += 1;
            continue;
        };
        let mut id = None;
        for next in &lines[index + 1..] {
            let next = next.trim();
            if next == "};" {
                break;
            }
            if let Some(value) = next.strip_prefix("id: \"") {
                id = value.split('"').next().map(str::to_owned);
            }
        }
        if let Some(id) = id {
            map.insert(ident.trim().to_owned(), id);
        }
        index += 1;
    }
    map
}

/// The service ids a crate's descriptors register: every ident in an
/// `&[...]` list that resolves through the crate's `ServiceDecl` constants.
/// The descriptor tables reference their services either as a literal
/// `services: &[SERVICE]` field or as a `&[SERVICE]` argument to the
/// descriptor builder, so the list scan resolves both shapes; an ident that
/// is not a `ServiceDecl` constant (a method, a provider reference, a role)
/// resolves to nothing and stays out of the registered set.
fn descriptor_service_ids(text: &str, consts: &BTreeMap<String, String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let marker = "&[";
    let mut rest = text;
    while let Some(start) = rest.find(marker) {
        let after = &rest[start + marker.len()..];
        if let Some(list) = after.split(']').next() {
            for ident in list.split(',') {
                if let Some(id) = consts.get(ident.trim()) {
                    out.insert(id.clone());
                }
            }
        }
        rest = after;
    }
    out
}

/// Read every declaring crate's registration declaration into a
/// crate-keyed map, in crate-name order.
///
/// The crate set is [`declaration_paths`]', which refuses a provider crate
/// carrying no declaration rather than dropping it: a renamed
/// `registrations.json` would otherwise remove its family from the composed
/// registration table with the drift gate still green over the smaller input.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn load_declarations(repo_root: &Path) -> Result<BTreeMap<String, RegistrationDeclaration>, String> {
    let mut out = BTreeMap::new();
    for (name, declaration_path) in declaration_paths(repo_root, Declaration::Registrations)? {
        let text = fs::read_to_string(&declaration_path).map_err(|error| {
            format!("cannot read {}: {error}", declaration_path.display())
        })?;
        let declaration: RegistrationDeclaration = serde_json::from_str(&text).map_err(|error| {
            format!("cannot parse {}: {error}", declaration_path.display())
        })?;
        if declaration.crate_name != name {
            return Err(format!(
                "declaration-crate-mismatch: {} names crate {} but the directory is {name}",
                declaration_path.display(),
                declaration.crate_name
            ));
        }
        out.insert(name, declaration);
    }
    Ok(out)
}
/// Render the registration table from the declarations alone.
///
/// `gen-new-graph` renders its committed new-graph projection through this
/// entry point, so the staged bytes are the same render the `--fix` path
/// installs. It reads no crate source, so the parity gate stays a separate
/// cross-check the new-graph closure runs over the composition rather than a
/// condition of rendering it.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
pub(crate) fn render_declarations_only(repo_root: &Path) -> Result<String, String> {
    render(repo_root)
}

/// Render the generated registration table from the declarations joined
/// against the identity authority, in crate-name order.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn render(repo_root: &Path) -> Result<String, String> {
    let declarations = load_declarations(repo_root)?;
    let identities = ProviderIdentities::load(repo_root)?;
    // The parity gate owns the refusal a registration row with no runtime
    // identity; the render runs after it, so a row here always resolves.
    let rows = declarations
        .iter()
        .map(|(crate_name, declaration)| {
            let identity = identities
                .identity(crate_name, Surface::Runtime)
                .ok_or_else(|| {
                    format!("runtime-identity-missing: crate {crate_name} declares a provider registration but owns no runtime Provider identity")
                })?;
            Ok((identity, declaration))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut out = String::new();
    out.push_str("// @generated\n");
    out.push_str("// Provenance: emitted from the per-crate `registrations.json` service\n");
    out.push_str("// facts joined against the per-crate `provider-identity.json` runtime\n");
    out.push_str("// identities by `cargo xtask check-provider-crate-layout --fix`; the\n");
    out.push_str("// layout check's authority drift gate regenerates this file byte-for-byte,\n");
    out.push_str("// and refuses a hand edit.\n\n");
    out.push_str("/// One registered provider family row: the provider identity the daemon's\n");
    out.push_str("/// composition root composes and the effect-service ids the family declares.\n");
    out.push_str("pub(crate) struct ProviderRegistration {\n");
    out.push_str("    pub(crate) provider_ref: &'static str,\n");
    out.push_str("    pub(crate) services: &'static [&'static str],\n");
    out.push_str("}\n\n");
    out.push_str("/// The registered provider families, in declaration order.\n");
    out.push_str("pub(crate) const PROVIDER_REGISTRATIONS: &[ProviderRegistration] = &[\n");
    for (identity, declaration) in &rows {
        let services = declaration
            .services
            .iter()
            .map(|service| format!("\"{service}\""))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "    ProviderRegistration {{\n        provider_ref: \"{identity}\",\n        services: &[{services}],\n    }},\n"
        ));
    }
    out.push_str("];\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// The registration rows are derived, so a provider that uses existing
    /// primitives registers without a daemon edit or a layout-ratchet row.
    #[test]
    fn the_registration_rows_are_derived_from_the_declaration() {
        let plan = crate::resource_type_authority::declaration_fixture::plan(&["export", "close"]);
        let rendered = render_declaration_registrations(&plan);
        assert!(rendered.contains("artifact_id: \"provider-volume-virtiofs\""));
        assert!(
            rendered.contains("\"volume-virtiofs.d2bus.org/export\""),
            "the declared service is registered: {rendered}"
        );
        assert_eq!(rendered, render_declaration_registrations(&plan), "byte-stable");
    }

    /// Adding a method moves the registration row with the declaration.
    #[test]
    fn one_changed_method_moves_the_registration_row() {
        let before =
            render_declaration_registrations(&crate::resource_type_authority::declaration_fixture::plan(&["export"]));
        let after = render_declaration_registrations(
            &crate::resource_type_authority::declaration_fixture::plan(&["export", "close"]),
        );
        assert_ne!(before, after);
    }

    /// A throwaway fixture tree under the OS temp dir.
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
                "d2b-provider-registration-authority-{name}-{}-{nonce}",
                std::process::id()
            ));
            Self { root }
        }

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write(&self, relative: &str, content: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(&path, content).expect("write");
        }

        /// A registration file for one fixture crate with the given
        /// services. The Provider identity the row registers under is the
        /// crate's runtime identity, stated in its `provider-identity.json`.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_declaration(&self, crate_name: &str, services: &[&str]) {
            let services = services
                .iter()
                .map(|service| format!("      \"{service}\""))
                .collect::<Vec<_>>()
                .join(",\n");
            self.write(
                &format!("packages/{crate_name}/registrations.json"),
                &format!("{{\n  \"crate\": \"{crate_name}\",\n  \"services\": [\n{services}\n  ]\n}}\n"),
            );
        }

        /// The identity authority declaration for one fixture crate, owning
        /// `runtime` on the runtime surface and nothing on the other two.
        /// The evidence anchor names the crate's own driver source, which
        /// every fixture crate carries.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_runtime_identity(&self, crate_name: &str, runtime: &str) {
            // The evidence anchor has to name the identity, not merely some
            // symbol in a file the crate happens to hold, so the crate's own
            // identity module carries the name.
            self.write(
                &format!("packages/{crate_name}/src/identity.rs"),
                &format!("pub const PROVIDER_IDENTITY: &str = \"{runtime}\";\n"),
            );
            self.write(
                &format!("packages/{crate_name}/provider-identity.json"),
                &format!(
                    "{{\n  \"crate\": \"{crate_name}\",\n  \"family\": \"{runtime}\",\n  \"roles\": [\"runtime\"],\n  \"nonBinary\": false,\n  \"product\": {{\n    \"identity\": null,\n    \"reason\": \"no-identity-owned\"\n  }},\n  \"runtime\": {{\n    \"identity\": \"{runtime}\",\n    \"evidence\": [\n      {{\n        \"path\": \"packages/{crate_name}/src/identity.rs\",\n        \"symbol\": \"PROVIDER_IDENTITY\"\n      }}\n    ]\n  }},\n  \"session\": {{\n    \"identity\": null,\n    \"reason\": \"no-identity-owned\"\n  }},\n  \"blockers\": []\n}}\n"
                ),
            );
        }

        /// The fixture crate's sources spelling one `ServiceDecl` const per
        /// service and registering it in a descriptor's `services` list.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_sources(&self, crate_name: &str, services: &[&str]) {
            let consts = services
                .iter()
                .map(|service| {
                    let ident = service
                        .split('.')
                        .next()
                        .expect("service id prefix")
                        .to_ascii_uppercase()
                        .replace('-', "_");
                    format!(
                        "pub const {ident}_SERVICE: ServiceDecl = ServiceDecl {{\n    id: \"{service}\",\n    methods: &[],\n    attach_kinds: &[],\n    streams: &[],\n    endpoint_policy: None,\n}};\n"
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            let refs = services
                .iter()
                .map(|service| {
                    let ident = service
                        .split('.')
                        .next()
                        .expect("service id prefix")
                        .to_ascii_uppercase()
                        .replace('-', "_");
                    format!("        {ident}_SERVICE,")
                })
                .collect::<Vec<_>>()
                .join("\n");
            self.write(
                &format!("packages/{crate_name}/src/driver.rs"),
                &format!(
                    "use d2b_resource_types::DriverDescriptor;\n\
                     {consts}\n\
                     pub fn fixture_descriptor() -> DriverDescriptor {{\n\
                     \x20   DriverDescriptor {{\n\
                     \x20       resource_type: WellKnownType::FIXTURE,\n\
                     \x20       services: &[\n{refs}\n\x20       ],\n\
                     \x20   }}\n\
                     }}\n"
                ),
            );
        }
    }

    impl Drop for Fixture {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// A registration that omits a service its sources spell fails the
    /// parity check naming both the crate and the service.
    #[test]
    fn the_parity_check_fails_when_a_declaration_omits_a_spelled_service() {
        let fixture = Fixture::new("omitted-service");
        fixture.write_sources("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        fixture.write_runtime_identity("d2b-provider-fixture", "fixture");
        fixture.write_declaration("d2b-provider-fixture", &[]);
        let errors = parity_errors(&fixture.root).expect("parity loads");
        assert!(
            errors.iter().any(|error| {
                error.contains("spelled-but-not-declared")
                    && error.contains("d2b-provider-fixture")
                    && error.contains("fixture.d2bus.org/alpha")
            }),
            "expected the spelled-but-not-declared violation naming both crate and service: {errors:?}"
        );
    }

    /// A registration naming a service the crate's sources do not spell
    /// fails the parity check naming both.
    #[test]
    fn the_parity_check_fails_when_a_declaration_names_an_unspelled_service() {
        let fixture = Fixture::new("unspelled-service");
        fixture.write_sources("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        fixture.write_runtime_identity("d2b-provider-fixture", "fixture");
        fixture.write_declaration("d2b-provider-fixture", &["fixture.d2bus.org/ghost"]);
        let errors = parity_errors(&fixture.root).expect("parity loads");
        assert!(
            errors.iter().any(|error| {
                error.contains("declared-but-not-spelled")
                    && error.contains("d2b-provider-fixture")
                    && error.contains("fixture.d2bus.org/ghost")
            }),
            "expected the declared-but-not-spelled violation naming both: {errors:?}"
        );
    }

    /// A registration whose crate owns no runtime identity fails before any
    /// generation: a `registrations.json` states the services a family
    /// registers, and the identity it registers under is the crate's own
    /// runtime identity in the identity authority.
    #[test]
    fn the_parity_check_fails_when_a_registration_declares_no_runtime_identity() {
        let fixture = Fixture::new("no-runtime-identity");
        fixture.write_sources("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        fixture.write(
            "packages/d2b-provider-fixture/provider-identity.json",
            "{\n  \"crate\": \"d2b-provider-fixture\",\n  \"family\": \"fixture\",\n  \"roles\": [\"product\"],\n  \"nonBinary\": false,\n  \"product\": {\n    \"identity\": \"fixture\",\n    \"evidence\": [\n      {\n        \"path\": \"packages/d2b-provider-fixture/src/driver.rs\",\n        \"symbol\": \"DriverDescriptor\"\n      }\n    ]\n  },\n  \"runtime\": {\n    \"identity\": null,\n    \"reason\": \"composition-hosted\"\n  },\n  \"session\": {\n    \"identity\": null,\n    \"reason\": \"no-identity-owned\"\n  },\n  \"blockers\": []\n}\n",
        );
        fixture.write_declaration("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        let errors = parity_errors(&fixture.root).expect("parity loads");
        assert!(
            errors.iter().any(|error| {
                error.contains("runtime-identity-missing") && error.contains("d2b-provider-fixture")
            }),
            "expected the missing runtime identity violation naming the crate: {errors:?}"
        );
        assert!(
            render(&fixture.root).is_err(),
            "the render refuses the same tree the parity gate refused"
        );
    }

    /// A runtime identity the resource-name grammar refuses fails the
    /// identity authority before the registration is read. The identity is
    /// not held to the crate's directory name: a crate registers the
    /// identity it is, and the grammar is the whole of what a declared
    /// identity owes.
    #[test]
    fn the_parity_check_fails_when_the_runtime_identity_is_not_a_resource_name() {
        let fixture = Fixture::new("malformed-identity");
        fixture.write_sources("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        fixture.write_runtime_identity("d2b-provider-fixture", "not a name");
        fixture.write_declaration("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        let error = parity_errors(&fixture.root).expect_err("the authority refuses the identity");
        assert!(
            error.contains("not a name") && error.contains("d2b-provider-fixture"),
            "expected the identity violation naming both: {error}"
        );
    }

    /// A crate whose registered identity is not its directory's suffix
    /// passes: `d2b-provider-guest-qemu-media` registers
    /// `runtime-qemu-media`, and refusing that spelling is what made the
    /// identity inexpressible in the first place.
    #[test]
    fn the_parity_check_admits_an_identity_the_directory_name_does_not_spell() {
        let fixture = Fixture::new("renamed-identity");
        fixture.write_sources("d2b-provider-guest-qemu-media", &[]);
        fixture.write_runtime_identity("d2b-provider-guest-qemu-media", "runtime-qemu-media");
        fixture.write_declaration("d2b-provider-guest-qemu-media", &[]);
        assert_eq!(
            parity_errors(&fixture.root).expect("parity loads"),
            Vec::<String>::new(),
            "an identity the directory name does not spell is a declared identity, not a violation"
        );
        let rendered = render(&fixture.root).expect("the table resolves the identity");
        assert!(
            rendered.contains("provider_ref: \"runtime-qemu-media\""),
            "the row registers the crate's own runtime identity: {rendered}"
        );
    }

    /// A service declared by two crates fails the parity check naming both
    /// crates.
    #[test]
    fn the_parity_check_fails_when_two_crates_declare_one_service() {
        let fixture = Fixture::new("duplicate-service");
        for crate_name in ["d2b-provider-fixture", "d2b-provider-other"] {
            fixture.write_sources(crate_name, &["fixture.d2bus.org/alpha"]);
            fixture.write_runtime_identity(crate_name, crate_name.strip_prefix("d2b-provider-").expect("prefix"));
            fixture.write_declaration(crate_name, &["fixture.d2bus.org/alpha"]);
        }
        let errors = parity_errors(&fixture.root).expect("parity loads");
        assert!(
            errors.iter().any(|error| {
                error.contains("service-duplicate")
                    && error.contains("d2b-provider-fixture")
                    && error.contains("d2b-provider-other")
                    && error.contains("fixture.d2bus.org/alpha")
            }),
            "expected the duplicate-service violation naming both crates: {errors:?}"
        );
    }

    /// One runtime identity named by two crates fails the identity
    /// authority naming both crates, before the registration is read: an
    /// identity names exactly one owning crate.
    #[test]
    fn the_parity_check_fails_when_two_crates_declare_one_identity() {
        let fixture = Fixture::new("duplicate-identity");
        for crate_name in ["d2b-provider-fixture", "d2b-provider-other"] {
            fixture.write_sources(crate_name, &["fixture.d2bus.org/alpha"]);
            fixture.write_runtime_identity(crate_name, "fixture");
            fixture.write_declaration(crate_name, &["fixture.d2bus.org/alpha"]);
        }
        let error = parity_errors(&fixture.root).expect_err("the authority refuses the repetition");
        assert!(
            error.contains("identity-declared-twice")
                && error.contains("d2b-provider-fixture")
                && error.contains("d2b-provider-other"),
            "expected the duplicate-identity violation naming both crates: {error}"
        );
    }

    /// The parity and drift gates pass on a matching fixture, and the drift
    /// gate fails on a hand edit to the generated table.
    #[test]
    fn the_parity_and_drift_gates_pass_on_a_matching_fixture_and_fail_on_a_hand_edit() {
        let fixture = Fixture::new("happy");
        fixture.write_sources("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        fixture.write_runtime_identity("d2b-provider-fixture", "fixture");
        fixture.write_declaration("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        let rendered = render(&fixture.root).expect("render the registration table");
        assert!(rendered.contains("fixture.d2bus.org/alpha"), "{rendered}");
        fixture.write(GENERATED_ARTIFACT, &rendered);
        check(&fixture.root).expect("the parity and drift gates pass on a matching tree");

        let artifact_path = fixture.root.join(GENERATED_ARTIFACT);
        let edited = format!(
            "{}\n// hand edit naming a family the tree does not declare\n",
            fs::read_to_string(&artifact_path).expect("read artifact")
        );
        fs::write(&artifact_path, edited).expect("hand edit");
        let error = check(&fixture.root).expect_err("a hand edit must fail the drift gate");
        assert!(
            error.contains("drift"),
            "expected a drift violation naming the gate: {error}"
        );
    }

    /// Regeneration writes the committed artifact and a second run changes
    /// nothing.
    #[test]
    fn regeneration_writes_the_table_and_a_second_run_is_unchanged() {
        let fixture = Fixture::new("regenerate");
        fixture.write_sources("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        fixture.write_runtime_identity("d2b-provider-fixture", "fixture");
        fixture.write_declaration("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        let written = regenerate(&fixture.root).expect("regenerate the table");
        assert_eq!(written, vec![fixture.root.join(GENERATED_ARTIFACT)]);
        let first = fs::read_to_string(&written[0]).expect("read artifact");
        let again = regenerate(&fixture.root).expect("regenerate again");
        let second = fs::read_to_string(&again[0]).expect("read artifact");
        assert_eq!(first, second, "regeneration is idempotent");
        check(&fixture.root).expect("the gates pass after regeneration");
    }
}