//! The per-crate provider/service registrations and the generated
//! registration table.
//!
//! Every `packages/d2b-provider-*/registrations.json` declares the provider
//! identity its owning crate registers with the daemon's composition root
//! and the effect-service ids the family declares. This module:
//!
//! - aggregates those declarations into the `PROVIDER_REGISTRATIONS` table
//!   the daemon composition root composes (`include!`d from
//!   `packages/d2bd/src/resource_plane_v3.rs`), so a new family is
//!   registered without the daemon naming it - a lane that declares its
//!   family in the crate needs no daemon edit and no layout-ratchet row;
//! - runs the declaration-to-source parity gate: a declared provider must
//!   be the crate's own family, a declared service must be spelled in the
//!   crate's sources (a `ServiceDecl` const), every service the crate's
//!   sources spell or its descriptor registers must be declared, and a
//!   service or provider declared by two crates fails naming both;
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

use serde::Deserialize;

/// The directory-glob root the per-crate declarations live under.
const PACKAGES_DIR: &str = "packages";
const PROVIDER_PREFIX: &str = "d2b-provider-";
const DECLARATION_FILE: &str = "registrations.json";

/// The repository-relative generated artifact path (relative to the source
/// file that `include!`s it, so `include!("generated/...")` resolves it).
pub(crate) const GENERATED_ARTIFACT: &str =
    "packages/d2bd/src/generated/provider_registrations.rs";

/// One provider crate's registration declaration.
///
/// The declaration is the crate's registration surface: the provider
/// identity the daemon's composition root composes and the effect-service
/// ids the family declares. The provider is the crate's own family by
/// construction (the parity gate refuses anything else), and a service the
/// crate does not spell in its own sources fails the same way.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RegistrationDeclaration {
    #[serde(rename = "crate")]
    crate_name: String,
    provider: String,
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
    verify_committed(repo_root, &rendered)?;
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

/// Fail when the committed copy of the generated artifact differs from the
/// declarations' render.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn verify_committed(repo_root: &Path, rendered: &str) -> Result<(), String> {
    let artifact_path = repo_root.join(GENERATED_ARTIFACT);
    let on_disk = fs::read_to_string(&artifact_path).map_err(|_| {
        format!(
            "provider-registration artifact is missing at {}; run `cargo xtask check-provider-crate-layout --fix`",
            artifact_path.display()
        )
    })?;
    if on_disk != rendered {
        return Err(format!(
            "provider-registration drift: the committed generated artifact {} differs from the declarations' output; a hand edit or a stale generation must be repaired by `cargo xtask check-provider-crate-layout --fix`",
            artifact_path.display()
        ));
    }
    Ok(())
}

/// The declaration-to-source parity violations: a declared provider that is
/// not the crate's own family, a declared service the crate's sources do not
/// spell, a service the crate's sources spell or its descriptor registers
/// that the declaration omits, and a service or provider declared by two
/// crates.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn parity_errors(repo_root: &Path) -> Result<Vec<String>, String> {
    let declarations = load_declarations(repo_root)?;
    let mut errors = Vec::new();
    let mut providers: BTreeMap<&str, &str> = BTreeMap::new();
    let mut services: BTreeMap<&str, &str> = BTreeMap::new();
    for (crate_name, declaration) in &declarations {
        // The declared provider is the crate's own family by construction:
        // the crate name spells the family it owns, so a declaration naming
        // another family is a parity violation naming both.
        let family = crate_name
            .strip_prefix(PROVIDER_PREFIX)
            .unwrap_or(crate_name);
        if declaration.provider != family {
            errors.push(format!(
                "provider-mismatch: crate {crate_name} declares provider {}; a crate declares only its own family",
                declaration.provider
            ));
        }
        if let Some(first) = providers.insert(&declaration.provider, crate_name) {
            errors.push(format!(
                "provider-duplicate: provider {} is declared by both {first} and {crate_name}",
                declaration.provider
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

/// Recursively collect every `.rs` file under one source tree.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn collect_rs_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Err(format!(
            "missing-src: the declaring crate has no src tree at {}",
            dir.display()
        ));
    }
    let entries = fs::read_dir(dir)
        .map_err(|error| format!("cannot read {}: {error}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read a source entry: {error}"))?;
        let path = entry.path();
        if path.is_dir() {
            out.extend(collect_rs_files(&path)?);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
    Ok(out)
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
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn load_declarations(repo_root: &Path) -> Result<BTreeMap<String, RegistrationDeclaration>, String> {
    let packages = repo_root.join(PACKAGES_DIR);
    let entries = fs::read_dir(&packages)
        .map_err(|error| format!("cannot read {}: {error}", packages.display()))?;
    let mut out = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("cannot read a package entry: {error}"))?;
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !name.starts_with(PROVIDER_PREFIX) {
            continue;
        }
        let declaration_path = path.join(DECLARATION_FILE);
        if !declaration_path.is_file() {
            continue;
        }
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
        out.insert(name.to_owned(), declaration);
    }
    Ok(out)
}

/// Render the generated registration table from the declarations, in
/// crate-name order.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn render(repo_root: &Path) -> Result<String, String> {
    let declarations = load_declarations(repo_root)?;
    let mut out = String::new();
    out.push_str("// @generated\n");
    out.push_str("// Provenance: emitted from the per-crate `registrations.json` declarations\n");
    out.push_str("// by `cargo xtask check-provider-crate-layout --fix`; the layout check's\n");
    out.push_str("// authority drift gate regenerates this file byte-for-byte, and refuses a\n");
    out.push_str("// hand edit.\n\n");
    out.push_str("/// One registered provider family row: the provider identity the daemon's\n");
    out.push_str("/// composition root composes and the effect-service ids the family declares.\n");
    out.push_str("pub(crate) struct ProviderRegistration {\n");
    out.push_str("    pub(crate) provider_ref: &'static str,\n");
    out.push_str("    pub(crate) services: &'static [&'static str],\n");
    out.push_str("}\n\n");
    out.push_str("/// The registered provider families, in declaration order.\n");
    out.push_str("pub(crate) const PROVIDER_REGISTRATIONS: &[ProviderRegistration] = &[\n");
    for declaration in declarations.values() {
        let services = declaration
            .services
            .iter()
            .map(|service| format!("\"{service}\""))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!(
            "    ProviderRegistration {{\n        provider_ref: \"{}\",\n        services: &[{services}],\n    }},\n",
            declaration.provider
        ));
    }
    out.push_str("];\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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

        /// A declaration file for one fixture crate with the given services.
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn write_declaration(&self, crate_name: &str, services: &[&str]) {
            let services = services
                .iter()
                .map(|service| format!("      \"{service}\""))
                .collect::<Vec<_>>()
                .join(",\n");
            self.write(
                &format!("packages/{crate_name}/registrations.json"),
                &format!(
                    "{{\n  \"crate\": \"{crate_name}\",\n  \"provider\": \"{family}\",\n  \"services\": [\n{services}\n  ]\n}}\n",
                    family = crate_name.strip_prefix("d2b-provider-").expect("provider prefix")
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

    /// A declaration that omits a service its sources spell fails the
    /// parity check naming both the crate and the service.
    #[test]
    fn the_parity_check_fails_when_a_declaration_omits_a_spelled_service() {
        let fixture = Fixture::new("omitted-service");
        fixture.write_sources("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
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

    /// A declaration naming a service the crate's sources do not spell fails
    /// the parity check naming both.
    #[test]
    fn the_parity_check_fails_when_a_declaration_names_an_unspelled_service() {
        let fixture = Fixture::new("unspelled-service");
        fixture.write_sources("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
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

    /// A declaration whose provider is not the crate's own family fails the
    /// parity check.
    #[test]
    fn the_parity_check_fails_when_the_provider_is_not_the_crates_family() {
        let fixture = Fixture::new("foreign-provider");
        fixture.write_sources("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        fixture.write_declaration("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        let path = fixture.root.join("packages/d2b-provider-fixture/registrations.json");
        let text = fs::read_to_string(&path).expect("declaration");
        fs::write(
            &path,
            text.replace("\"provider\": \"fixture\"", "\"provider\": \"foreign\""),
        )
        .expect("mutate");
        let errors = parity_errors(&fixture.root).expect("parity loads");
        assert!(
            errors.iter().any(|error| {
                error.contains("provider-mismatch")
                    && error.contains("d2b-provider-fixture")
                    && error.contains("foreign")
            }),
            "expected the provider violation naming both: {errors:?}"
        );
    }

    /// A service declared by two crates fails the parity check naming both
    /// crates.
    #[test]
    fn the_parity_check_fails_when_two_crates_declare_one_service() {
        let fixture = Fixture::new("duplicate-service");
        for crate_name in ["d2b-provider-fixture", "d2b-provider-other"] {
            fixture.write_sources(crate_name, &["fixture.d2bus.org/alpha"]);
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

    /// A provider declared by two crates fails the parity check naming both
    /// crates.
    #[test]
    fn the_parity_check_fails_when_two_crates_declare_one_provider() {
        let fixture = Fixture::new("duplicate-provider");
        fixture.write_sources("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        fixture.write_declaration("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
        fixture.write_sources("d2b-provider-other", &[]);
        fixture.write_declaration("d2b-provider-other", &[]);
        let path = fixture.root.join("packages/d2b-provider-other/registrations.json");
        let text = fs::read_to_string(&path).expect("declaration");
        fs::write(
            &path,
            text.replace("\"provider\": \"other\"", "\"provider\": \"fixture\""),
        )
        .expect("mutate");
        let errors = parity_errors(&fixture.root).expect("parity loads");
        assert!(
            errors.iter().any(|error| {
                error.contains("provider-duplicate")
                    && error.contains("d2b-provider-fixture")
                    && error.contains("d2b-provider-other")
                    && error.contains("fixture")
            }),
            "expected the duplicate-provider violation naming both crates: {errors:?}"
        );
    }

    /// The parity and drift gates pass on a matching fixture, and the drift
    /// gate fails on a hand edit to the generated table.
    #[test]
    fn the_parity_and_drift_gates_pass_on_a_matching_fixture_and_fail_on_a_hand_edit() {
        let fixture = Fixture::new("happy");
        fixture.write_sources("d2b-provider-fixture", &["fixture.d2bus.org/alpha"]);
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