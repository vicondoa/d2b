//! The per-crate Provider identity authority (KTD1, U1).
//!
//! Every `packages/d2b-provider-*/provider-identity.json` is the one place a
//! provider crate's Provider identities are stated: the identity it owns on
//! each of the three identity surfaces (product, runtime, session), the closed
//! classification roles that ownership implies (fixed bootstrap, shared
//! driver, and the identity-less classes), the production source that names
//! each identity, the closed reason every empty surface states, and any
//! external blocker the crate is waiting on.
//!
//! The declaration is the identity authority the runtime registrations, the
//! session service catalogs, and the product packaging metadata resolve their
//! identities from. Those three keep their own schemas and their own facts -
//! a registration row states the services and effects a runtime identity
//! registers, a catalog row states the routing a session identity answers, and
//! the packaging row states the artifacts and metadata a product identity
//! ships - and this module is what they join against. It reads no crate
//! source, emits no artifact, and projects nothing on its own: the joins below
//! are the surface the generators consume once they cut over.
//!
//! What a declaration may not be is inferred. A crate's identity is not its
//! directory name (`d2b-provider-process-minijail` registers `system-minijail`
//! and `d2b-provider-guest-qemu-media` registers `runtime-qemu-media`), its
//! family is not derived from its directory, and neither is a role. Every
//! identity is stated here, spelled as the resource name the contracts admit
//! (the `Provider/<name>` reference form is derived, never authored), and
//! every non-null identity carries at least one named production source that
//! says so outside this repository's generated output and outside the Provider
//! matrix this declaration set replaces.
//!
//! Uniqueness is global and explicit: an identity names exactly one owning
//! crate across every surface, and the one admitted repetition - the same
//! crate naming one identity on two of its own surfaces - has to be stated on
//! both surfaces before it loads. Nothing here fails open: a missing, renamed,
//! malformed, duplicated, unexplained, unevidenced, or matrix-sourced
//! declaration is a refusal, not a row with defaults.
//!
//! The coverage gate requires every provider crate to carry the
//! declaration: [`Declaration::ProviderIdentity`] is a mandatory declaration
//! kind, so a provider-prefixed crate that arrives without a
//! `provider-identity.json` is refused by name rather than dropped from
//! every join below, and each crate's `BUILD.bazel` names the file so the
//! Bazel runfiles closure carries it into every drift action.
//!
//! [`Declaration::ProviderIdentity`]: crate::authority_common::Declaration::ProviderIdentity

// The generators join through the accessors below: the registration
// authority resolves every runtime row, the session catalog resolves every
// routing row, the packaging matrix resolves every product row, the crate
// layout policy classifies every provider-prefixed crate, and the closure
// cross-check resolves every declared Provider reference. No consumer reads
// an identity from a declaration file any more.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Component, Path},
};

use crate::authority_common::{admits_provider_identity, declaration_paths, Declaration};
use serde::Deserialize;

/// Longest family token one declaration may spell.
const MAX_FAMILY_BYTES: usize = 63;
/// Longest evidence symbol one anchor may spell.
const MAX_EVIDENCE_SYMBOL_BYTES: usize = 128;
/// Longest blocker summary one record may carry.
const MAX_BLOCKER_SUMMARY_BYTES: usize = 400;

/// The repository-relative source that holds the committed Provider matrix.
///
/// The matrix is the inventory this declaration set replaces, so it is not
/// evidence for any identity: an anchor naming it would restate the
/// declaration in the one file the declaration is cutting over from, which is
/// the two-authorities failure this authority exists to end.
const PROVIDER_MATRIX_SOURCE: &str = "packages/xtask/src/provider_crate_policy.rs";

/// The path component that marks a tree's generated output.
///
/// A generated file is an output of the declarations, so naming one as the
/// evidence for a declared identity would make the identity true because this
/// repository already asserted it.
const GENERATED_COMPONENT: &str = "generated";

/// The source extensions a production evidence anchor may name.
///
/// A Provider identity is named by the NixOS modules that admit or refuse a row
/// for it and by the Rust sources that gate its spec: those are the two
/// languages the product reads an identity in. Anything else - a document, a
/// lockfile, another declaration - is not evidence.
const EVIDENCE_SOURCE_EXTENSIONS: [&str; 2] = ["rs", "nix"];

/// The surface a Provider identity is owned on.
///
/// The three surfaces are independent slots in one declaration rather than one
/// list, because a crate routinely owns one identity on one surface and none
/// on another: the systemd Process provider ships a product-plane artifact and
/// registers a different runtime identity, and a crate that owns no session
/// routing says so instead of inheriting a neighbour's answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Surface {
    /// The identity the product packaging metadata and the Nix provider
    /// catalog ship.
    Product,
    /// The identity the daemon's composition root registers as a runtime
    /// Provider.
    Runtime,
    /// The identity the session-plane service catalog routes to.
    Session,
}

impl Surface {
    /// Every surface, in declaration order.
    pub(crate) const ALL: [Self; 3] = [Self::Product, Self::Runtime, Self::Session];

    /// The declaration key this surface is written under.
    pub(crate) const fn key(self) -> &'static str {
        match self {
            Self::Product => "product",
            Self::Runtime => "runtime",
            Self::Session => "session",
        }
    }

    /// The role that owns this surface: a crate that declares an identity on
    /// this surface carries the matching role, and a crate that carries the
    /// role owns an identity here.
    pub(crate) const fn owner_role(self) -> Role {
        match self {
            Self::Product => Role::Product,
            Self::Runtime => Role::Runtime,
            Self::Session => Role::Session,
        }
    }
}

/// The surface names itself as its declaration key, so a diagnostic reads
/// "the product surface" rather than spelling the enum variant.
impl std::fmt::Display for Surface {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.key())
    }
}

/// The closed classification vocabulary one crate's declaration carries.
///
/// The first three roles are the identity surfaces themselves; the rest say
/// what a crate is when it owns no identity of its own, and the last five
/// admit an all-null declaration only together. Nothing in this vocabulary is
/// derived from a crate's directory name, its dependency count, or the
/// presence of another declaration file beside it: the declaration states the
/// classification, and the loader refuses one whose roles and identities
/// disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Role {
    /// The crate owns its product-plane Provider identity.
    Product,
    /// The crate owns its runtime-plane Provider identity.
    Runtime,
    /// The crate owns its session-plane Provider identity.
    Session,
    /// The crate declares a driver another family's registration runs, so it
    /// owns the identity that driver serves and shares it outward.
    SharedDriver,
    /// The crate owns a fixed-bootstrap Provider identity: a packaged
    /// product identity the deployment registers at startup, outside ordinary
    /// Process projection and outside the ProviderSet runtime registrations.
    ///
    /// The identity lives on the product surface because that is where the
    /// artifact it names is shipped; its startup is a deployment fact about
    /// that artifact, not a runtime Provider registration. A fixed-bootstrap
    /// crate therefore owns no runtime identity, which is what keeps the
    /// generation cutover from minting a ProviderSet row for it.
    FixedBootstrap,
    /// The crate owns a ResourceType vocabulary and no Provider identity of
    /// its own.
    ResourceFamily,
    /// The crate publishes a service package and owns no Provider identity of
    /// its own.
    ServiceOnly,
    /// The crate is a support crate: it implements other crates' surfaces and
    /// owns none of its own.
    Support,
    /// The crate is test-only and owns no Provider identity.
    Test,
    /// The crate deliberately owns no Provider identity at all.
    NoIdentity,
}

impl Role {
    /// Every role, in declaration order: the three identity surfaces first,
    /// then the classification roles, then the identity-less classes.
    pub(crate) const ALL: [Self; 10] = [
        Self::Product,
        Self::Runtime,
        Self::Session,
        Self::SharedDriver,
        Self::FixedBootstrap,
        Self::ResourceFamily,
        Self::ServiceOnly,
        Self::Support,
        Self::Test,
        Self::NoIdentity,
    ];

    /// Whether this role states that the crate owns no Provider identity, in
    /// which case every surface slot is null.
    pub(crate) const fn is_identity_less(self) -> bool {
        matches!(
            self,
            Self::ResourceFamily | Self::ServiceOnly | Self::Support | Self::Test | Self::NoIdentity
        )
    }
}

/// The closed reasons one surface may state instead of an identity.
///
/// A null surface is a decision, so it carries its reason: a loader that
/// accepted an unexplained null would let a lost identity read as a crate that
/// owns none, and every consumer would silently stop shipping that row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum NoIdentityReason {
    /// No production source outside the crate graph names a Provider identity
    /// for this crate on this surface.
    NoIdentityOwned,
    /// The effect service this crate spells is hosted by the daemon's own
    /// composition rather than through a registration row, so a registration
    /// could carry neither half truthfully.
    CompositionHosted,
}

impl NoIdentityReason {
    /// Every reason, in declaration order.
    #[allow(dead_code, reason = "read by this module's own census tests; no generator consumes it")]
    pub(crate) const ALL: [Self; 2] = [Self::NoIdentityOwned, Self::CompositionHosted];

    /// The closed vocabulary as the `serde` error names it.
    pub(crate) const fn vocabulary() -> &'static str {
        "no-identity-owned | composition-hosted"
    }
}

/// One production source that names a declared identity.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EvidenceAnchor {
    /// Repository-relative path of the source.
    path: String,
    /// The stable symbol in that source which names the identity.
    symbol: String,
}

impl EvidenceAnchor {
    /// The repository-relative path this anchor names.
    fn path(&self) -> &str {
        &self.path
    }

    /// The symbol this anchor names.
    fn symbol(&self) -> &str {
        &self.symbol
    }
}

/// One surface slot: either the identity the crate owns here with the
/// production evidence that names it, or the closed reason it owns none.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SurfaceSlot {
    /// The identity this crate owns on this surface, spelled as the resource
    /// name the contracts admit.
    #[serde(default)]
    identity: Option<String>,
    /// Why the crate owns no identity on this surface, when it owns none.
    #[serde(default)]
    reason: Option<NoIdentityReason>,
    /// The surfaces of this same declaration that deliberately carry this
    /// identity too. A repeated identity is admitted only when both slots say
    /// so; nothing infers the repetition from the two values agreeing.
    #[serde(default)]
    shares_identity_with: Vec<Surface>,
    /// The production sources that name the identity.
    #[serde(default)]
    evidence: Vec<EvidenceAnchor>,
}

impl SurfaceSlot {
    /// The identity this crate owns here, when it owns one.
    fn identity(&self) -> Option<&str> {
        self.identity.as_deref()
    }

    /// The closed reason this crate owns none here, when it owns none.
    fn reason(&self) -> Option<NoIdentityReason> {
        self.reason
    }

    /// The surfaces this slot declares a deliberate identity repetition with.
    fn shares_identity_with(&self) -> &[Surface] {
        &self.shares_identity_with
    }

    /// The production sources that name the identity.
    fn evidence(&self) -> &[EvidenceAnchor] {
        &self.evidence
    }
}

/// One external issue a crate's declaration is waiting on.
///
/// A blocker records what is tracked outside the repository. It never excuses
/// a missing or empty declaration: the loader reads a blocker only from a
/// declaration that is present and complete, so an absent declaration fails
/// with no blocker in scope to soften it.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Blocker {
    /// The tracked issue, spelled `#<number>`.
    issue: String,
    /// What the crate is waiting on.
    summary: String,
}

impl Blocker {
    /// The tracked issue.
    fn issue(&self) -> &str {
        &self.issue
    }

    /// What the crate is waiting on.
    fn summary(&self) -> &str {
        &self.summary
    }
}

/// One provider crate's identity declaration.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct IdentityDeclaration {
    /// The declaring crate, which must be the directory the file sits in.
    #[serde(rename = "crate")]
    crate_name: String,
    /// The family this crate realizes, stated rather than derived from the
    /// directory name.
    family: String,
    /// The closed classification roles this crate carries.
    roles: Vec<Role>,
    /// The product-plane identity slot.
    product: SurfaceSlot,
    /// The runtime-plane identity slot.
    runtime: SurfaceSlot,
    /// The session-plane identity slot.
    session: SurfaceSlot,
    /// The external issues this crate's declaration is waiting on.
    #[serde(default)]
    blockers: Vec<Blocker>,
}

impl IdentityDeclaration {
    /// The declaring crate.
    fn crate_name(&self) -> &str {
        &self.crate_name
    }

    /// The family this crate realizes.
    fn family(&self) -> &str {
        &self.family
    }

    /// The classification roles this crate carries, in declaration order.
    fn roles(&self) -> &[Role] {
        &self.roles
    }

    /// The slot for one surface.
    fn slot(&self, surface: Surface) -> &SurfaceSlot {
        match surface {
            Surface::Product => &self.product,
            Surface::Runtime => &self.runtime,
            Surface::Session => &self.session,
        }
    }

    /// The external issues this declaration records.
    fn blockers(&self) -> &[Blocker] {
        &self.blockers
    }

    /// Whether this crate carries one classification role.
    fn has_role(&self, role: Role) -> bool {
        self.roles.contains(&role)
    }
}

/// Every provider crate's identity declaration, in crate-name order.
///
/// This is the join the generators read once they cut over: by crate for a
/// declaration's whole row, and by crate and surface for the one identity a
/// consumer needs. The crate order is the enumeration's own, so two loads of
/// the same tree produce the same sequence.
#[derive(Debug, Default)]
pub(crate) struct ProviderIdentities {
    declarations: BTreeMap<String, IdentityDeclaration>,
}

impl ProviderIdentities {
    /// Load and validate every provider crate's identity declaration.
    ///
    /// The crate set is [`declaration_paths`]', so a provider crate carrying
    /// no declaration is refused by name unless the absence ratchet records
    /// why it owns none, and a renamed declaration file fails the load instead
    /// of quietly dropping a crate's identities from every join below.
    #[allow(clippy::disallowed_methods, reason = "CLI-only path")]
    pub(crate) fn load(repo_root: &Path) -> Result<Self, String> {
        let mut declarations = BTreeMap::new();
        let mut errors = Vec::new();
        for (crate_name, path) in declaration_paths(repo_root, Declaration::ProviderIdentity)? {
            let text = fs::read_to_string(&path).map_err(|error| {
                format!("cannot read {}: {error}", path.display())
            })?;
            match serde_json::from_str::<IdentityDeclaration>(&text) {
                Ok(declaration) if declaration.crate_name() != crate_name => {
                    errors.push(format!(
                        "declaration-crate-mismatch: {} names crate {} but the directory is {crate_name}",
                        path.display(),
                        declaration.crate_name()
                    ));
                }
                Ok(declaration) => {
                    declarations.insert(crate_name, declaration);
                }
                Err(error) => errors.push(format!(
                    "malformed-provider-identity-declaration: cannot parse {}: {error}",
                    path.display()
                )),
            }
        }
        let sources = read_evidence_sources(repo_root, &declarations);
        errors.extend(declaration_errors(&declarations, &sources));
        if !errors.is_empty() {
            return Err(format!(
                "provider-identity declaration violations:\n- {}",
                errors.join("\n- ")
            ));
        }
        Ok(Self { declarations })
    }

    /// Every declaring crate with its declaration, in crate-name order.
    #[allow(dead_code, reason = "read by this module's own census tests; no generator consumes it")]
    pub(crate) fn declarations(&self) -> impl Iterator<Item = (&str, &IdentityDeclaration)> {
        self.declarations
            .iter()
            .map(|(crate_name, declaration)| (crate_name.as_str(), declaration))
    }

    /// One crate's declaration.
    pub(crate) fn declaration(&self, crate_name: &str) -> Option<&IdentityDeclaration> {
        self.declarations.get(crate_name)
    }

    /// The declaring crates, in crate-name order.
    pub(crate) fn crate_names(&self) -> impl Iterator<Item = &str> {
        self.declarations.keys().map(String::as_str)
    }

    /// The identity one crate owns on one surface.
    pub(crate) fn identity(&self, crate_name: &str, surface: Surface) -> Option<&str> {
        self.declaration(crate_name)
            .and_then(|declaration| declaration.slot(surface).identity())
    }

    /// Every `(crate, identity)` pair declared on one surface, in crate-name
    /// order.
    pub(crate) fn identities(&self, surface: Surface) -> impl Iterator<Item = (&str, &str)> {
        self.declarations
            .iter()
            .filter_map(move |(crate_name, declaration)| {
                declaration
                    .slot(surface)
                    .identity()
                    .map(|identity| (crate_name.as_str(), identity))
            })
    }

    /// Every `(crate, identity)` pair a fixed-bootstrap crate declares on the
    /// product surface, in crate-name order.
    ///
    /// The load gates the pair: a crate that claims fixed-bootstrap ownership
    /// without a product identity never reaches this join.
    pub(crate) fn fixed_bootstrap_identities(&self) -> Vec<(&str, &str)> {
        self.declarations
            .iter()
            .filter_map(|(crate_name, declaration)| {
                let identity = declaration.slot(Surface::Product).identity()?;
                declaration
                    .has_role(Role::FixedBootstrap)
                    .then_some((crate_name.as_str(), identity))
            })
            .collect()
    }

    /// The crates whose driver another family's registration runs, in
    /// crate-name order.
    pub(crate) fn shared_driver_crates(&self) -> Vec<&str> {
        self.declarations
            .iter()
            .filter(|(_, declaration)| declaration.has_role(Role::SharedDriver))
            .map(|(crate_name, _)| crate_name.as_str())
            .collect()
    }

    /// Every external issue the declarations record, in crate-name order.
    #[allow(dead_code, reason = "read by this module's own census tests; no generator consumes it")]
    pub(crate) fn blockers(&self) -> impl Iterator<Item = (&str, &Blocker)> {
        self.declarations
            .iter()
            .flat_map(|(crate_name, declaration)| {
                declaration
                    .blockers()
                    .iter()
                    .map(move |blocker| (crate_name.as_str(), blocker))
            })
    }

    /// The production sources that name one crate's identity on one surface.
    #[allow(dead_code, reason = "read by this module's own census tests; no generator consumes it")]
    pub(crate) fn evidence(&self, crate_name: &str, surface: Surface) -> &[EvidenceAnchor] {
        self.declaration(crate_name)
            .map(|declaration| declaration.slot(surface).evidence())
            .unwrap_or_default()
    }

    /// The `Provider/<name>` reference a declared identity is written as.
    pub(crate) fn provider_ref(identity: &str) -> String {
        format!("Provider/{identity}")
    }

    /// The family one crate realizes, stated by its own declaration.
    pub(crate) fn family(&self, crate_name: &str) -> Option<&str> {
        self.declaration(crate_name)
            .map(IdentityDeclaration::family)
    }

    /// Whether one crate carries one classification role.
    pub(crate) fn has_role(&self, crate_name: &str, role: Role) -> bool {
        self.declaration(crate_name)
            .is_some_and(|declaration| declaration.has_role(role))
    }

    /// Every identity the declarations own on any surface, in crate-name
    /// then surface order.
    ///
    /// This is the closed vocabulary a `Provider/<name>` reference resolves
    /// against: a reference the declarations do not own names no Provider the
    /// product has, whatever spelled it.
    pub(crate) fn all_identities(&self) -> impl Iterator<Item = &str> {
        self.declarations
            .values()
            .flat_map(|declaration| {
                Surface::ALL
                    .into_iter()
                    .filter_map(move |surface| declaration.slot(surface).identity())
            })
    }
}

/// Read every evidence source the declarations name, keyed by the declared
/// repository-relative path.
///
/// A path that is absent or unreadable is simply missing from the map: which
/// policy a path fails is the validator's call, not the reader's, so the
/// refusal names the rule rather than an `io` error.
#[allow(clippy::disallowed_methods, reason = "CLI-only path")]
fn read_evidence_sources(
    repo_root: &Path,
    declarations: &BTreeMap<String, IdentityDeclaration>,
) -> BTreeMap<String, String> {
    let mut sources = BTreeMap::new();
    for declaration in declarations.values() {
        for surface in Surface::ALL {
            for anchor in declaration.slot(surface).evidence() {
                // Only a repository-relative path is read at all: a
                // declaration that names an escaping or absolute path is
                // refused below, and the refusal must not depend on what
                // this loader found on the other side of the boundary.
                if !is_repo_relative(anchor.path()) || sources.contains_key(anchor.path()) {
                    continue;
                }
                let path = repo_root.join(anchor.path());
                if let Ok(text) = fs::read_to_string(&path) {
                    sources.insert(anchor.path().to_owned(), text);
                }
            }
        }
    }
    sources
}

/// The declaration violations across every crate, in crate-name order.
fn declaration_errors(
    declarations: &BTreeMap<String, IdentityDeclaration>,
    sources: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut errors = Vec::new();
    let mut owners: BTreeMap<&str, (&str, Surface)> = BTreeMap::new();
    for (crate_name, declaration) in declarations {
        errors.extend(crate_errors(crate_name, declaration, sources));
        for surface in Surface::ALL {
            let Some(identity) = declaration.slot(surface).identity() else {
                continue;
            };
            match owners.get(identity) {
                // A repetition inside one declaration is judged by the
                // per-crate pass above, which requires both surfaces to declare
                // it; a repetition across two crates is never admitted.
                Some((first, _)) if first == crate_name => {}
                Some((first, first_surface)) => errors.push(format!(
                    "identity-declared-twice: identity {identity} is declared by both {first} on its {} surface and {crate_name} on its {} surface; one Provider identity names one owning crate",
                    first_surface.key(),
                    surface.key()
                )),
                None => {
                    owners.insert(identity, (crate_name.as_str(), surface));
                }
            }
        }
    }
    errors
}

/// One crate's declaration violations, in a fixed order: the family, the
/// classification roles, then each surface's slot.
fn crate_errors(
    crate_name: &str,
    declaration: &IdentityDeclaration,
    sources: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut errors = Vec::new();
    if !admits_family(declaration.family()) {
        errors.push(format!(
            "malformed-family: crate {crate_name} declares family \"{}\"; a family is a lower-kebab token of at most {MAX_FAMILY_BYTES} bytes, and it is stated rather than read back out of the crate's directory name",
            declaration.family()
        ));
    }
    errors.extend(role_errors(crate_name, declaration));
    for surface in Surface::ALL {
        errors.extend(slot_errors(
            crate_name,
            surface,
            declaration.slot(surface),
            declaration,
            sources,
        ));
    }
    errors.extend(blocker_errors(crate_name, declaration));
    errors
}

/// The blocker violations: a blocker is a record, so it points at a tracked
/// issue and says what the crate is waiting on.
///
/// It is never an exemption. A blocker is only read out of a declaration that
/// is present and complete, so a crate whose declaration is missing or empty
/// fails with no blocker in scope to soften it - which is what keeps an
/// external issue from becoming a way to declare less.
fn blocker_errors(crate_name: &str, declaration: &IdentityDeclaration) -> Vec<String> {
    let mut errors = Vec::new();
    for blocker in declaration.blockers() {
        let issue = blocker.issue();
        if !issue.starts_with('#')
            || issue.len() < 2
            || !issue[1..].bytes().all(|byte| byte.is_ascii_digit())
        {
            errors.push(format!(
                "malformed-blocker-issue: crate {crate_name} names the blocker issue \"{issue}\"; a blocker is tracked by `#<number>`"
            ));
        }
        let summary = blocker.summary();
        if summary.is_empty()
            || summary.len() > MAX_BLOCKER_SUMMARY_BYTES
            || summary.chars().any(char::is_control)
        {
            errors.push(format!(
                "malformed-blocker-summary: crate {crate_name} states blocker {issue} with a summary of {} bytes; a blocker says what the crate is waiting on",
                summary.len()
            ));
        }
    }
    errors
}

/// The classification violations: a role set that is empty, repeats itself,
/// or disagrees with the identities the declaration states.
fn role_errors(crate_name: &str, declaration: &IdentityDeclaration) -> Vec<String> {
    let mut errors = Vec::new();
    let mut seen = BTreeSet::new();
    for role in declaration.roles() {
        if !seen.insert(*role) {
            errors.push(format!(
                "duplicate-role: crate {crate_name} names the {:?} role twice; the closed vocabulary is {}",
                role,
                Role::ALL
                    .iter()
                    .map(|role| format!("{role:?}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    let owns_any = Surface::ALL
        .iter()
        .any(|surface| declaration.slot(*surface).identity().is_some());
    let identity_less = declaration
        .roles()
        .iter()
        .copied()
        .find(|role| role.is_identity_less());
    if declaration.roles().is_empty() {
        errors.push(format!(
            "no-classification-role: crate {crate_name} declares no classification role; a crate states what it owns and, when it owns no identity, which identity-less class it belongs to"
        ));
    } else if owns_any && let Some(role) = identity_less {
        errors.push(format!(
            "contradictory-roles: crate {crate_name} declares a Provider identity and also the {role:?} role; the identity-less roles ({}) admit an all-null declaration only",
            Role::ALL
                .iter()
                .copied()
                .filter(|role| role.is_identity_less())
                .map(|role| format!("{role:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    } else if !owns_any && identity_less.is_none() {
        errors.push(format!(
            "no-classification-role: crate {crate_name} declares no Provider identity on any surface and names none of the identity-less roles ({}); a crate that owns no identity says which class it belongs to",
            Role::ALL
                .iter()
                .copied()
                .filter(|role| role.is_identity_less())
                .map(|role| format!("{role:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    for surface in Surface::ALL {
        let owns = declaration.slot(surface).identity().is_some();
        let claims = declaration.has_role(surface.owner_role());
        if claims && !owns {
            errors.push(format!(
                "role-without-identity: crate {crate_name} claims the {:?} role but declares no {} identity",
                surface.owner_role(),
                surface.key()
            ));
        } else if owns && !claims {
            errors.push(format!(
                "identity-without-role: crate {crate_name} declares a {} identity without the {:?} role that owns that surface",
                surface.key(),
                surface.owner_role()
            ));
        }
    }
    if declaration.has_role(Role::FixedBootstrap)
        && declaration.slot(Surface::Product).identity().is_none()
    {
        errors.push(format!(
            "fixed-bootstrap-without-product: crate {crate_name} claims fixed-bootstrap ownership but declares no product identity; a fixed-bootstrap Provider identity is a packaged product identity whose startup is a deployment fact, not a ProviderSet runtime registration"
        ));
    }
    if declaration.has_role(Role::SharedDriver) && !owns_any {
        errors.push(format!(
            "shared-driver-without-identity: crate {crate_name} claims shared-driver ownership but declares no Provider identity; the crate that declares a shared driver owns the identity that driver serves"
        ));
    }
    errors
}

/// One surface slot's violations, in a fixed order: the identity-or-reason
/// choice, the identity grammar, the deliberate repetition, then the evidence.
fn slot_errors(
    crate_name: &str,
    surface: Surface,
    slot: &SurfaceSlot,
    declaration: &IdentityDeclaration,
    sources: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut errors = Vec::new();
    match (slot.identity(), slot.reason()) {
        (None, None) => {
            errors.push(format!(
                "unexplained-null-identity: crate {crate_name} declares no {} identity and states no reason; a null surface carries one of {}",
                surface.key(),
                NoIdentityReason::vocabulary()
            ));
        }
        (Some(identity), Some(reason)) => errors.push(format!(
            "identity-with-null-reason: crate {crate_name} declares the {} identity {identity} and also the {reason:?} reason; a surface states one or the other",
            surface.key()
        )),
        (Some(identity), None) => {
            if !admits_provider_identity(identity) {
                errors.push(format!(
                    "malformed-provider-identity: crate {crate_name} declares the {} identity \"{identity}\"; a Provider identity is the resource name the contracts admit, spelled bare - the {} form is the reference the consumers derive",
                    surface.key(),
                    ProviderIdentities::provider_ref(identity)
                ));
            }
            if slot.evidence().is_empty() {
                errors.push(format!(
                    "missing-identity-evidence: crate {crate_name} declares the {} identity {identity} with no production evidence; an identity is named by a production source outside the crate graph, and this declaration is where that source is stated",
                    surface.key()
                ));
            }
            errors.extend(evidence_errors(crate_name, surface, identity, slot.evidence(), sources));
        }
        (None, Some(_)) => {
            if !slot.evidence().is_empty() {
                errors.push(format!(
                    "evidence-on-null-surface: crate {crate_name} states no {} identity yet anchors {} production source(s) on the surface",
                    surface.key(),
                    slot.evidence().len()
                ));
            }
        }
    }
    errors.extend(reuse_errors(crate_name, surface, slot, declaration));
    errors
}

/// The deliberate-repetition violations for one surface slot.
fn reuse_errors(
    crate_name: &str,
    surface: Surface,
    slot: &SurfaceSlot,
    declaration: &IdentityDeclaration,
) -> Vec<String> {
    let mut errors = Vec::new();
    for shared in slot.shares_identity_with() {
        if *shared == surface {
            errors.push(format!(
                "self-shared-identity: crate {crate_name} lists its own {} surface in sharesIdentityWith; a repetition is between two surfaces of one declaration",
                surface.key()
            ));
        } else if declaration.slot(*shared).identity().is_none() {
            errors.push(format!(
                "shared-identity-with-null-surface: crate {crate_name} shares its {} identity with the {} surface, which declares none",
                surface.key(),
                shared.key()
            ));
        } else if slot.identity().is_none() {
            errors.push(format!(
                "shared-identity-with-null-surface: crate {crate_name} declares no {} identity yet lists the {} surface in sharesIdentityWith",
                surface.key(),
                shared.key()
            ));
        } else if declaration.slot(*shared).identity() != slot.identity() {
            errors.push(format!(
                "identity-reuse-mismatch: crate {crate_name} shares its {} identity with the {} surface, which declares a different identity ({})",
                surface.key(),
                shared.key(),
                declaration.slot(*shared).identity().unwrap_or_default()
            ));
        }
    }
    if let Some(identity) = slot.identity() {
        let repeated: Vec<Surface> = Surface::ALL
            .iter()
            .copied()
            .filter(|other| *other != surface && declaration.slot(*other).identity() == Some(identity))
            .collect();
        if repeated.len() != slot.shares_identity_with().len()
            || repeated
                .iter()
                .any(|other| !slot.shares_identity_with().contains(other))
        {
            errors.push(format!(
                "identity-reuse-not-declared: crate {crate_name} declares the identity {identity} on the {} surface as well as this one without declaring the repetition from each of them; a repeated identity is admitted only when both slots name the other in sharesIdentityWith",
                repeated
                    .iter()
                    .map(|other| other.key())
                    .collect::<Vec<_>>()
                    .join(" and ")
            ));
        }
    }
    errors
}

/// The evidence violations for one declared identity.
///
/// The evidence is the whole of the identity's production standing, so each
/// rule is a refusal: the path is repository-relative, is a production source
/// rather than a generated output or the Provider matrix, exists, and spells
/// the symbol the declaration names.
fn evidence_errors(
    crate_name: &str,
    surface: Surface,
    identity: &str,
    evidence: &[EvidenceAnchor],
    sources: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut errors = Vec::new();
    let mut anchors = BTreeSet::new();
    for anchor in evidence {
        let path = anchor.path();
        if path.is_empty() {
            errors.push(format!(
                "evidence-not-repo-relative: crate {crate_name} anchors the {surface} identity {identity} on an empty path; evidence names a repository-relative source"
            ));
            continue;
        }
        if !is_repo_relative(path) {
            errors.push(format!(
                "evidence-not-repo-relative: crate {crate_name} anchors the {surface} identity {identity} on \"{path}\"; evidence names a repository-relative source, not an absolute or escaping path"
            ));
            continue;
        }
        if anchor.symbol().is_empty()
            || anchor.symbol().len() > MAX_EVIDENCE_SYMBOL_BYTES
            || anchor.symbol().chars().any(char::is_control)
        {
            errors.push(format!(
                "malformed-evidence-symbol: crate {crate_name} anchors the {surface} identity {identity} on the symbol \"{}\" in {path}; a symbol is a non-empty bounded token",
                anchor.symbol()
            ));
            continue;
        }
        if Path::new(path)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_none_or(|extension| !EVIDENCE_SOURCE_EXTENSIONS.contains(&extension))
        {
            errors.push(format!(
                "evidence-not-source: crate {crate_name} anchors the {surface} identity {identity} on {path}; production evidence is a {} source",
                EVIDENCE_SOURCE_EXTENSIONS.join(" or ")
            ));
            continue;
        }
        if has_generated_component(path) {
            errors.push(format!(
                "evidence-generated-output: crate {crate_name} anchors the {surface} identity {identity} on {path}; a generated file is an output of the declarations, so it cannot be the evidence for the identity it already asserts"
            ));
            continue;
        }
        if path == PROVIDER_MATRIX_SOURCE {
            errors.push(format!(
                "evidence-provider-matrix: crate {crate_name} anchors the {surface} identity {identity} on {PROVIDER_MATRIX_SOURCE}; the committed Provider matrix is the inventory this declaration set replaces, so it is not evidence for any identity"
            ));
            continue;
        }
        let Some(text) = sources.get(path) else {
            errors.push(format!(
                "evidence-path-missing: crate {crate_name} anchors the {surface} identity {identity} on {path}, which is not a readable file in the repository"
            ));
            continue;
        };
        if !spells(text, anchor.symbol()) {
            errors.push(format!(
                "evidence-symbol-missing: crate {crate_name} anchors the {surface} identity {identity} on the symbol \"{}\" in {path}, which spells no such symbol",
                anchor.symbol()
            ));
            continue;
        }
        if !anchors.insert((path.to_owned(), anchor.symbol().to_owned())) {
            errors.push(format!(
                "duplicate-identity-evidence: crate {crate_name} repeats the {surface} identity {identity} anchor {path}#{}",
                anchor.symbol()
            ));
        }
        if !spells(text, identity) {
            errors.push(format!(
                "evidence-identity-missing: crate {crate_name} anchors the {surface} identity {identity} on {path}, which never names that identity; an anchor points at a production source that says so, not at a file that happens to hold the symbol"
            ));
        }
    }
    errors
}

/// Whether one evidence path is repository-relative: every component an
/// ordinary name, so no absolute prefix, no `..`, and no `.`.
fn is_repo_relative(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

/// Whether one evidence path runs through the tree's generated output.
fn has_generated_component(path: &str) -> bool {
    Path::new(path)
        .components()
        .any(|component| component.as_os_str() == GENERATED_COMPONENT)
}

/// Whether one production source spells one symbol, matched on identifier
/// boundaries so a symbol cannot be "found" inside a longer identifier.
fn spells(text: &str, symbol: &str) -> bool {
    let is_identifier = |character: char| character.is_ascii_alphanumeric() || character == '_';
    text.match_indices(symbol).any(|(index, _)| {
        text[..index]
            .chars()
            .next_back()
            .is_none_or(|character| !is_identifier(character))
            && text[index + symbol.len()..]
                .chars()
                .next()
                .is_none_or(|character| !is_identifier(character))
    })
}

/// Whether one family token is the lower-kebab token a crate states for
/// itself: at most [`MAX_FAMILY_BYTES`] bytes of `[a-z0-9]` separated by
/// single hyphens and opened by a letter.
///
/// This is deliberately not the resource-name grammar: a family names the
/// crate group, and reading either one back out of the other's spelling is
/// what made a crate's real identity inexpressible.
fn admits_family(family: &str) -> bool {
    !family.is_empty()
        && family.len() <= MAX_FAMILY_BYTES
        && family.starts_with(|character: char| character.is_ascii_lowercase())
        && family
            .bytes()
            .enumerate()
            .all(|(index, byte)| match byte {
                b'-' => index > 0 && index + 1 < family.len(),
                byte => byte.is_ascii_lowercase() || byte.is_ascii_digit(),
            })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    // The census asserts against the tree the loader enumerated, so the
    // enumeration itself is read here rather than restated in the assertions.
    use crate::authority_common::provider_crates;

    /// The declaration path of one fixture crate, spelled once so the tests
    /// and the loader agree on where a declaration lives.
    fn declaration_path(crate_name: &str) -> PathBuf {
        PathBuf::from("packages")
            .join(crate_name)
            .join(Declaration::ProviderIdentity.file_name())
    }

    /// The live repository's declarations, read through the loader the normal
    /// gate runs: the assertions below state what the committed tree says,
    /// not what a fixture would accept.
    fn repository_identities() -> ProviderIdentities {
        ProviderIdentities::load(crate::repo_root().expect("the aggregate passes D2B_REPO_ROOT"))
            .expect("every provider crate declares its identities")
    }

    /// The NixOS source an accepted anchor names: outside the crate graph,
    /// spelling the symbol below.
    const NIX_SOURCE: &str = "nixos-modules/assertions.nix";
    /// The symbol the fixture's Nix source spells.
    const NIX_SYMBOL: &str = "providerRefAssertion";
    /// The Rust source an accepted anchor names.
    const RUST_SOURCE: &str = "packages/d2b-core/src/runtime.rs";
    /// The symbol the fixture's Rust source spells.
    const RUST_SYMBOL: &str = "SERVING_WORKER_PROVIDER_REF";

    /// Every Provider identity a fixture declaration anchors on one of the
    /// accepted sources above. Both sources spell all of them, so a fixture
    /// varies the placement rule rather than the source's willingness to name
    /// the identity it is asked to evidence.
    const FIXTURE_IDENTITIES: &[&str] = &[
        "alpha-product",
        "device-tpm",
        "endpoint",
        "process-systemd",
        "runtime-alpha",
        "runtime-endpoint",
        "session-alpha",
        "session-beta",
        "system-minijail",
        "system-systemd",
    ];

    /// The generated projection no identity may take its evidence from.
    const GENERATED_SOURCE: &str = "generated/new-graph/provider_registrations.rs";
    /// The symbol the fixture's generated projection really does spell, so the
    /// refusal is about where it lives rather than whether it exists.
    const GENERATED_SYMBOL: &str = "PROVIDER_REGISTRATIONS";

    /// A throwaway tree holding the fixture crates and the sources their
    /// declarations anchor evidence on.
    struct Fixture {
        root: PathBuf,
    }

    #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
    impl Fixture {
        /// A fixture root under the OS temp dir. The root is not created
        /// here: the first [`Fixture::write`] makes the path it needs.
        fn new(name: &str) -> Self {
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "d2b-provider-identity-authority-{name}-{}-{nonce}",
                std::process::id()
            ));
            Self { root }
        }

        /// Write one repository-relative file, creating its parents.
        fn write(&self, relative: &str, content: &str) {
            let path = self.root.join(relative);
            fs::create_dir_all(path.parent().expect("parent")).expect("mkdir");
            fs::write(&path, content).expect("write");
        }

        /// Read one repository-relative file back.
        fn read(&self, relative: &str) -> String {
            fs::read_to_string(self.root.join(relative)).expect("read the fixture file")
        }

        /// The two production sources the accepted anchors name. Each spells
        /// the symbols its anchors claim and every Provider identity the
        /// fixture declarations below anchor on it, because the loader
        /// refuses an anchor whose source holds the symbol without naming the
        /// identity.
        fn write_evidence_sources(&self) {
            self.write(
                NIX_SOURCE,
                &format!(
                    "{{ assertions = [\n  {NIX_SYMBOL} \"systemd\"\n{}];\n}}\n",
                    FIXTURE_IDENTITIES
                        .iter()
                        .map(|identity| format!("  {NIX_SYMBOL} \"Provider/{identity}\""))
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            );
            self.write(
                RUST_SOURCE,
                &format!(
                    "pub const {RUST_SYMBOL}: &str = \"Provider/process-systemd\";\n{}\n",
                    FIXTURE_IDENTITIES
                        .iter()
                        .enumerate()
                        .map(|(index, identity)| {
                            format!(
                                "pub const FIXTURE_PROVIDER_REF_{index}: &str = \"Provider/{identity}\";"
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                ),
            );
            self.write(
                GENERATED_SOURCE,
                &format!("pub const {GENERATED_SYMBOL}: &[&str] = &[];\n"),
            );
        }

        /// One crate's declaration file, written verbatim.
        fn write_declaration(&self, crate_name: &str, body: &str) {
            let relative = declaration_path(crate_name).to_string_lossy().into_owned();
            self.write(&relative, &format!("{body}\n"));
        }

        /// The evidence sources plus a fixture crate that owns a distinct
        /// product and runtime identity and no session identity: the shape
        /// every negative case varies from.
        fn with_identity_crate(&self) -> &Self {
            self.write_evidence_sources();
            self.write_declaration(
                "d2b-provider-fixture",
                &declaration(
                    "d2b-provider-fixture",
                    "process-systemd",
                    &["product", "runtime"],
                    &slot(
                        Some("system-systemd"),
                        None,
                        &[],
                        &[(NIX_SOURCE, NIX_SYMBOL)],
                    ),
                    &slot(
                        Some("process-systemd"),
                        None,
                        &[],
                        &[(RUST_SOURCE, RUST_SYMBOL)],
                    ),
                    &null_slot("no-identity-owned"),
                    "[]",
                ),
            );
            self
        }

        /// One fixture crate that owns exactly one surface's identity and
        /// states the other two as nulls: the shape a surface join needs, and
        /// the shape the cross-crate rules collide two crates over.
        fn write_crate_owning(
            &self,
            crate_name: &str,
            family: &str,
            surface: Surface,
            identity: &str,
            evidence: (&str, &str),
        ) {
            let owned = identity_slot(identity, evidence);
            let empty = null_slot("no-identity-owned");
            let (product, runtime, session) = match surface {
                Surface::Product => (owned, empty.clone(), empty),
                Surface::Runtime => (empty.clone(), owned, empty),
                Surface::Session => (empty.clone(), empty.clone(), owned),
            };
            self.write_declaration(
                crate_name,
                &declaration(
                    crate_name,
                    family,
                    &[surface.key()],
                    &product,
                    &runtime,
                    &session,
                    "[]",
                ),
            );
        }

        /// The refusal the load reports, or a panic naming what it said
        /// instead.
        fn expect_refusal(&self) -> String {
            ProviderIdentities::load(&self.root).expect_err("the load refuses the declaration")
        }

        /// Whether the load refuses with a violation naming `slug`.
        fn refuses(&self, slug: &str) -> bool {
            let error = self.expect_refusal();
            assert!(
                error.lines().any(|line| line.contains(slug)),
                "expected a {slug} violation:\n{error}"
            );
            true
        }
    }

    impl Drop for Fixture {
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    /// One surface slot: a null slot states its closed reason and a real one
    /// states its identity, the surfaces it deliberately repeats that
    /// identity on, and the production evidence naming it.
    fn slot(identity: Option<&str>, reason: Option<&str>, shares: &[&str], evidence: &[(&str, &str)]) -> String {
        let identity = match identity {
            Some(identity) => format!("\"{identity}\""),
            None => "null".to_owned(),
        };
        let mut fields = vec![format!("\"identity\": {identity}")];
        if let Some(reason) = reason {
            fields.push(format!("\"reason\": \"{reason}\""));
        }
        if !shares.is_empty() {
            fields.push(format!("\"sharesIdentityWith\": [{}]", quoted(shares)));
        }
        if !evidence.is_empty() {
            fields.push(format!(
                "\"evidence\": [{}]",
                evidence
                    .iter()
                    .map(|(path, symbol)| {
                        format!("{{\"path\": \"{path}\", \"symbol\": \"{symbol}\"}}")
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        format!("{{{}}}", fields.join(", "))
    }

    /// The null slot a crate that owns no identity on a surface writes.
    fn null_slot(reason: &str) -> String {
        slot(None, Some(reason), &[], &[])
    }

    /// The slot a crate that owns one identity on a surface writes, anchored
    /// on the given production source.
    fn identity_slot(identity: &str, evidence: (&str, &str)) -> String {
        slot(Some(identity), None, &[], &[evidence])
    }

    /// One declaration, spelled by key so a test states only what it varies.
    #[allow(clippy::too_many_arguments, reason = "one declaration has eight fields")]
    fn declaration(
        crate_name: &str,
        family: &str,
        roles: &[&str],
        product: &str,
        runtime: &str,
        session: &str,
        blockers: &str,
    ) -> String {
        format!(
            "{{\n  \"crate\": \"{crate_name}\",\n  \"family\": \"{family}\",\n  \"roles\": [{}],\n  \"product\": {product},\n  \"runtime\": {runtime},\n  \"session\": {session},\n  \"blockers\": {blockers}\n}}",
            quoted(roles)
        )
    }

    /// One blocker record.
    fn blocker(issue: &str, summary: &str) -> String {
        format!("{{\"issue\": \"{issue}\", \"summary\": \"{summary}\"}}")
    }

    fn quoted(values: &[&str]) -> String {
        values
            .iter()
            .map(|value| format!("\"{value}\""))
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// A crate's three surfaces are three independent slots, so a crate that
    /// ships a product artifact and registers a different runtime identity
    /// says both and is never asked to make them equal.
    #[test]
    fn one_crate_declares_a_distinct_product_and_runtime_identity() {
        let fixture = Fixture::new("distinct");
        fixture.with_identity_crate();
        let identities = ProviderIdentities::load(&fixture.root).expect("the declaration loads");
        assert_eq!(
            identities.identity("d2b-provider-fixture", Surface::Product),
            Some("system-systemd")
        );
        assert_eq!(
            identities.identity("d2b-provider-fixture", Surface::Runtime),
            Some("process-systemd")
        );
        assert_eq!(
            identities.identity("d2b-provider-fixture", Surface::Session),
            None,
            "the session surface states no identity"
        );
        let declaration = identities
            .declaration("d2b-provider-fixture")
            .expect("the crate declares");
        assert_eq!(declaration.family(), "process-systemd");
        assert_eq!(declaration.roles(), &[Role::Product, Role::Runtime]);
        assert_eq!(
            identities.evidence("d2b-provider-fixture", Surface::Product).len(),
1,
            "the product identity carries its production evidence"
        );
        assert_eq!(
            ProviderIdentities::provider_ref("system-systemd"),
            "Provider/system-systemd",
            "the reference form is derived from the identity, never authored"
        );
    }

    /// The joins are what the generators read: one row per crate, and one
    /// identity per surface, both in crate-name order.
    #[test]
    fn the_joins_report_every_declared_identity_in_crate_order() {
        let fixture = Fixture::new("joins");
        fixture.with_identity_crate();
        fixture.write_crate_owning(
            "d2b-provider-alpha",
            "alpha",
            Surface::Runtime,
            "runtime-alpha",
            (RUST_SOURCE, RUST_SYMBOL),
        );
        fixture.write_crate_owning(
            "d2b-provider-beta",
            "beta",
            Surface::Session,
            "session-beta",
            (NIX_SOURCE, NIX_SYMBOL),
        );
        let identities = ProviderIdentities::load(&fixture.root).expect("the declarations load");
        assert_eq!(
            identities.crate_names().collect::<Vec<_>>(),
            vec![
                "d2b-provider-alpha",
                "d2b-provider-beta",
                "d2b-provider-fixture"
            ],
            "the join is the enumeration's own crate order"
        );
        assert_eq!(
            identities
                .identities(Surface::Runtime)
                .collect::<Vec<_>>(),
            vec![
                ("d2b-provider-alpha", "runtime-alpha"),
                ("d2b-provider-fixture", "process-systemd")
            ],
            "one surface projects only the crates that own an identity on it"
        );
        assert_eq!(
            identities
                .identities(Surface::Product)
                .collect::<Vec<_>>(),
            vec![("d2b-provider-fixture", "system-systemd")]
        );
    }

    /// A crate that owns no identity at all still declares every surface and
    /// says which identity-less class it belongs to.
    #[test]
    fn an_identity_less_crate_declares_every_surface_as_null() {
        let fixture = Fixture::new("identity-less");
        fixture.write_evidence_sources();
        fixture.write_declaration(
            "d2b-provider-execution-policy",
            &declaration(
                "d2b-provider-execution-policy",
                "execution-policy",
                &["no-identity", "resource-family"],
                &null_slot("no-identity-owned"),
                &null_slot("no-identity-owned"),
                &null_slot("no-identity-owned"),
                &format!("[{}]", blocker("#629", "the policy vocabulary lane owns this row")),
            ),
        );
        let identities = ProviderIdentities::load(&fixture.root).expect("the declaration loads");
        let declaration = identities
            .declaration("d2b-provider-execution-policy")
            .expect("the crate declares");
        assert_eq!(declaration.roles(), &[Role::NoIdentity, Role::ResourceFamily]);
        for surface in Surface::ALL {
            assert_eq!(declaration.slot(surface).identity(), None);
            assert_eq!(
                declaration.slot(surface).reason(),
                Some(NoIdentityReason::NoIdentityOwned)
            );
        }
        assert_eq!(
            identities
                .blockers()
                .map(|(_, blocker)| (blocker.issue(), blocker.summary()))
                .collect::<Vec<_>>(),
            vec![("#629", "the policy vocabulary lane owns this row")],
            "an external blocker is recorded, not dropped"
        );
    }

    /// Fixed-bootstrap ownership is a product-surface fact: the crate that
    /// owns `system-minijail` declares it as the packaged identity it ships
    /// and registers no runtime Provider of its own.
    #[test]
    fn a_fixed_bootstrap_owner_declares_its_product_identity() {
        let fixture = Fixture::new("bootstrap");
        fixture.write_evidence_sources();
        fixture.write_declaration(
            "d2b-provider-process-minijail",
            &declaration(
                "d2b-provider-process-minijail",
                "system-minijail",
                &["product", "fixed-bootstrap"],
                &identity_slot("system-minijail", (NIX_SOURCE, NIX_SYMBOL)),
                &null_slot("no-identity-owned"),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let identities = ProviderIdentities::load(&fixture.root).expect("the declaration loads");
        assert_eq!(
            identities.fixed_bootstrap_identities(),
            vec![("d2b-provider-process-minijail", "system-minijail")],
            "the fixed-bootstrap join carries the owning crate and the product identity it ships"
        );
        assert_eq!(
            identities.identity("d2b-provider-process-minijail", Surface::Runtime),
            None,
            "a fixed-bootstrap Provider is a packaged artifact, never a ProviderSet runtime row"
        );
    }

    /// A crate may ship a product artifact and register no runtime Provider
    /// at all; the declaration says which half it owns rather than leaving
    /// the composition to guess.
    #[test]
    fn a_shared_driver_owner_declares_a_product_identity_and_no_runtime() {
        let fixture = Fixture::new("shared-driver");
        fixture.write_evidence_sources();
        fixture.write_declaration(
            "d2b-provider-device-tpm",
            &declaration(
                "d2b-provider-device-tpm",
                "device-tpm",
                &["product", "shared-driver"],
                &identity_slot("device-tpm", (NIX_SOURCE, NIX_SYMBOL)),
                &null_slot("composition-hosted"),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let identities = ProviderIdentities::load(&fixture.root).expect("the declaration loads");
        assert_eq!(
            identities.identity("d2b-provider-device-tpm", Surface::Product),
            Some("device-tpm")
        );
        assert_eq!(
            identities.identity("d2b-provider-device-tpm", Surface::Runtime),
            None
        );
        assert_eq!(
            identities.shared_driver_crates(),
            vec!["d2b-provider-device-tpm"]
        );
    }

    /// The coverage gate refuses a provider crate carrying no declaration by
    /// name, so a renamed file cannot quietly remove a crate's identities from
    /// every join.
    #[test]
    fn a_crate_with_no_declaration_is_refused_by_name() {
        let fixture = Fixture::new("missing");
        fixture.write("packages/d2b-provider-fixture/src/lib.rs", "pub fn driver() {}\n");
        let error = fixture.expect_refusal();
        assert!(
            error.contains("missing-declaration")
                && error.contains("d2b-provider-fixture")
                && error.contains(Declaration::ProviderIdentity.file_name()),
            "the refusal names the crate and the declaration it lost:\n{error}"
        );
    }

    /// The declaration binds itself to the directory it sits in, so a file
    /// copied from one crate into another cannot claim the neighbour's
    /// identities.
    #[test]
    fn a_declaration_naming_another_crate_is_refused() {
        let fixture = Fixture::new("crate-mismatch");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-fixture",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("declaration-crate-mismatch")
                && error.contains("d2b-provider-alpha")
                && error.contains("d2b-provider-fixture"),
            "the refusal names the file, the crate it claims, and the directory it sits in:\n{error}"
        );
    }

    /// A typo'd key is refused at the boundary rather than read as a default:
    /// a misspelled `evidence` would leave the identity with no production
    /// standing at all.
    #[test]
    fn an_unknown_declaration_key_is_refused() {
        let fixture = Fixture::new("unknown-key");
        fixture.with_identity_crate();
        let path = declaration_path("d2b-provider-fixture").to_string_lossy().into_owned();
        let text = fixture.read(&path);
        fixture.write(
            &path,
            &text.replace(
                "\"product\"",
                "\"produkt\"",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("malformed-provider-identity-declaration")
                && error.contains("produkt"),
            "the refusal names the key the grammar refused:\n{error}"
        );
    }

    /// An identity is the resource name the contracts admit, spelled bare:
    /// the `Provider/<name>` reference is what consumers derive from it.
    #[test]
    fn an_identity_the_resource_name_grammar_refuses_is_refused() {
        for identity in ["Provider/device-tpm", "System_Core", "device tpm"] {
            let fixture = Fixture::new("malformed-identity");
            fixture.with_identity_crate();
            fixture.write_declaration(
                "d2b-provider-alpha",
                &declaration(
                    "d2b-provider-alpha",
                    "alpha",
                    &["runtime"],
                    &null_slot("no-identity-owned"),
                    &identity_slot(identity, (RUST_SOURCE, RUST_SYMBOL)),
                    &null_slot("no-identity-owned"),
                    "[]",
                ),
            );
            assert!(
                fixture.refuses("malformed-provider-identity"),
                "{identity} is not a Provider identity"
            );
        }
    }

    /// The classification vocabulary is closed: a role no loader knows is a
    /// refusal, because a silently dropped role would leave the crate
    /// unclassified in every projection that reads this authority.
    #[test]
    fn an_unknown_role_is_refused() {
        let fixture = Fixture::new("unknown-role");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime", "bootstrap"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("malformed-provider-identity-declaration")
                && error.contains("bootstrap")
                && error.contains("fixed-bootstrap"),
            "the refusal names the unknown role and the closed vocabulary:\n{error}"
        );
    }

    /// The null reasons are closed too: a reason no loader knows would make
    /// the null a decision with no stated ground.
    #[test]
    fn an_unknown_null_reason_is_refused() {
        let fixture = Fixture::new("unknown-reason");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("not-implemented-yet"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("malformed-provider-identity-declaration")
                && error.contains("not-implemented-yet")
                && error.contains("no-identity-owned"),
            "the refusal names the unknown reason and the closed vocabulary:\n{error}"
        );
    }

    /// A null surface is a decision, so it carries its reason: an empty slot
    /// would let a lost identity read as a crate that owns none.
    #[test]
    fn an_unexplained_null_is_refused() {
        let fixture = Fixture::new("unexplained-null");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                "{}",
                "[]",
            ),
        );
        assert!(fixture.refuses("unexplained-null-identity"));
    }

    /// A surface states one thing: an identity with a null reason would make
    /// the same slot both owned and unowned.
    #[test]
    fn a_slot_may_not_state_both_an_identity_and_a_reason() {
        let fixture = Fixture::new("identity-and-reason");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &slot(
                    Some("runtime-alpha"),
                    Some("no-identity-owned"),
                    &[],
                    &[(RUST_SOURCE, RUST_SYMBOL)],
                ),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("identity-with-null-reason"));
    }

    /// An identity with no production evidence is a name nothing enforces.
    #[test]
    fn an_identity_without_evidence_is_refused() {
        let fixture = Fixture::new("no-evidence");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &slot(Some("runtime-alpha"), None, &[], &[]),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("missing-identity-evidence"));
    }

    /// A generated file is an output of the declarations: naming one as the
    /// evidence for an identity would make the identity true because this
    /// repository already asserted it.
    #[test]
    fn evidence_naming_generated_output_is_refused() {
        let fixture = Fixture::new("generated-evidence");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (GENERATED_SOURCE, GENERATED_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("evidence-generated-output") && error.contains(GENERATED_SOURCE),
            "the refusal names the generated file:\n{error}"
        );
    }

    /// The committed Provider matrix is the inventory this declaration set
    /// replaces, so it is not evidence for any identity: an anchor naming it
    /// would make the matrix and this declaration two authorities for one
    /// fact.
    #[test]
    fn evidence_naming_the_provider_matrix_is_refused() {
        let fixture = Fixture::new("matrix-evidence");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (PROVIDER_MATRIX_SOURCE, "PROVIDER_MATRIX")),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("evidence-provider-matrix") && error.contains(PROVIDER_MATRIX_SOURCE),
            "the refusal names the matrix:\n{error}"
        );
    }

    /// Evidence is a file in this repository: an anchor on a path nothing
    /// ships names no production source at all.
    #[test]
    fn evidence_naming_a_missing_source_is_refused() {
        let fixture = Fixture::new("missing-source");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", ("nixos-modules/absent.nix", "absentRef")),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("evidence-path-missing")
                && error.contains("nixos-modules/absent.nix"),
            "the refusal names the path nothing ships:\n{error}"
        );
    }

    /// A Provider identity is named by the NixOS modules and the Rust
    /// sources; a document is not a production source.
    #[test]
    fn evidence_naming_a_non_source_file_is_refused() {
        let fixture = Fixture::new("non-source-evidence");
        fixture.with_identity_crate();
        fixture.write("docs/adr/0049.md", "# ADR\n\nPROVIDER_REF\n");
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", ("docs/adr/0049.md", "PROVIDER_REF")),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("evidence-not-source"));
    }

    /// Evidence is repository-relative: an absolute or escaping path would
    /// make the standing of an identity depend on a machine rather than on
    /// this repository.
    #[test]
    fn evidence_escaping_the_repository_is_refused() {
        let fixture = Fixture::new("escaping-evidence");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", ("../outside/runtime.rs", "SOME_REF")),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("evidence-not-repo-relative") && error.contains("../outside/runtime.rs"),
            "the refusal names the escaping path:\n{error}"
        );
    }

    /// The anchor names a symbol, so the symbol has to be there: an identity
    /// anchored on a name its source does not spell is evidenced by nothing.
    #[test]
    fn evidence_naming_a_symbol_the_source_does_not_spell_is_refused() {
        let fixture = Fixture::new("absent-symbol");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (NIX_SOURCE, "SOME_OTHER_REF")),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("evidence-symbol-missing")
                && error.contains("SOME_OTHER_REF")
                && error.contains(NIX_SOURCE),
            "the refusal names the symbol the source does not spell:\n{error}"
        );
    }

    /// An anchor whose source spells the symbol but never the identity is the
    /// hollow case: a file can hold a well-known constant without saying
    /// which Provider this declaration is about.
    #[test]
    fn evidence_naming_a_source_that_never_names_the_identity_is_refused() {
        let fixture = Fixture::new("silent-source");
        fixture.with_identity_crate();
        fixture.write(
            RUST_SOURCE,
            &format!("pub const {RUST_SYMBOL}: &str = \"Provider/systemd\";\n"),
        );
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("evidence-identity-missing")
                && error.contains("runtime-alpha")
                && error.contains(RUST_SOURCE),
            "the refusal names the identity the source never mentions:\n{error}"
        );
    }

    /// The same anchor twice is noise that hides a missing second source.
    #[test]
    fn a_repeated_evidence_anchor_is_refused() {
        let fixture = Fixture::new("repeated-anchor");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &slot(
                    Some("runtime-alpha"),
                    None,
                    &[],
                    &[
                        (NIX_SOURCE, NIX_SYMBOL),
                        (NIX_SOURCE, NIX_SYMBOL),
                    ],
                ),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("duplicate-identity-evidence"));
    }

    /// A surface that states no identity cannot carry evidence either.
    #[test]
    fn evidence_on_a_null_surface_is_refused() {
        let fixture = Fixture::new("null-evidence");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &slot(
                    None,
                    Some("no-identity-owned"),
                    &[],
                    &[(NIX_SOURCE, NIX_SYMBOL)],
                ),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("evidence-on-null-surface"));
    }

    /// The family is stated, not read back out of the directory name.
    #[test]
    fn a_malformed_family_is_refused() {
        let fixture = Fixture::new("malformed-family");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "Alpha_Family",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("malformed-family"));
    }

    /// Every crate classifies itself: a declaration with no role leaves the
    /// crate unclassified in every projection that reads this authority.
    #[test]
    fn a_crate_with_no_classification_role_is_refused() {
        let fixture = Fixture::new("no-role");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &[],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("no-classification-role"));
    }

    /// A role named twice is a declaration whose intent cannot be read.
    #[test]
    fn a_duplicate_role_is_refused() {
        let fixture = Fixture::new("duplicate-role");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime", "runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("duplicate-role"));
    }

    /// A role and its surface identity are two statements of one fact, and
    /// the loader refuses a declaration where they disagree in either
    /// direction.
    #[test]
    fn a_role_without_its_surface_identity_is_refused() {
        let fixture = Fixture::new("role-without-identity");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["product", "runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("role-without-identity") && error.contains("product"),
            "the refusal names the role and the surface it claims:\n{error}"
        );
    }

    /// A declared identity with no owning role would be projected into every
    /// consumer's view of that surface without a classification behind it.
    #[test]
    fn a_surface_identity_without_its_role_is_refused() {
        let fixture = Fixture::new("identity-without-role");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &identity_slot("alpha-product", (NIX_SOURCE, NIX_SYMBOL)),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("identity-without-role") && error.contains("product"),
 "the refusal names the surface whose role is missing:\n{error}"
        );
    }

    /// The identity-less roles admit an all-null declaration only: a crate
    /// cannot own a Provider identity and simultaneously claim to own none.
    #[test]
    fn a_crate_may_not_both_own_an_identity_and_claim_an_identity_less_role() {
        let fixture = Fixture::new("contradictory-roles");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime", "service-only"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("contradictory-roles"));
    }

    /// Fixed-bootstrap ownership is a product-surface fact, and a shared
    /// driver is owned by the crate that spells it.
    #[test]
    fn bootstrap_and_shared_driver_ownership_need_an_identity_to_own() {
        let fixture = Fixture::new("ownership-without-identity");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["fixed-bootstrap"],
                &null_slot("no-identity-owned"),
                &null_slot("no-identity-owned"),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("fixed-bootstrap-without-product"));

        let fixture = Fixture::new("shared-driver-without-identity");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["shared-driver"],
                &null_slot("no-identity-owned"),
                &null_slot("no-identity-owned"),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("shared-driver-without-identity"));
    }

    /// A blocker is a tracked external issue: a free-text reference cannot be
    /// followed back to the work it stands for.
    #[test]
    fn a_blocker_must_name_a_tracked_issue_and_say_what_it_waits_on() {
        let fixture = Fixture::new("blocker");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                &format!("[{}]", blocker("issue 629", "the relay lane owns this row")),
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("malformed-blocker-issue") && error.contains("issue 629"),
            "the refusal names the reference that is not a tracked issue:\n{error}"
        );

        let fixture = Fixture::new("empty-blocker");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("runtime-alpha", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                &format!("[{}]", blocker("#629", "")),
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("malformed-blocker-summary"),
            "a blocker that says nothing it waits on is refused:\n{error}"
        );
    }

    /// An identity names exactly one owning crate: two crates claiming one
    /// on any surface would leave every `Provider/<name>` reference with two
    /// possible answers.
    #[test]
    fn two_crates_may_not_claim_one_identity_on_any_surface() {
        let fixture = Fixture::new("identity-twice");
        fixture.with_identity_crate();
        fixture.write_declaration(
            "d2b-provider-alpha",
            &declaration(
                "d2b-provider-alpha",
                "alpha",
                &["runtime"],
                &null_slot("no-identity-owned"),
                &identity_slot("system-systemd", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("identity-declared-twice")
                && error.contains("system-systemd")
                && error.contains("d2b-provider-alpha")
                && error.contains("d2b-provider-fixture"),
            "the refusal names the identity and both crates:\n{error}"
        );
    }

    /// The one admitted repetition is one crate naming one identity on two of
    /// its own surfaces, and it has to be stated from both sides.
    #[test]
    fn one_crate_may_share_one_identity_across_its_own_surfaces() {
        let fixture = Fixture::new("identity-reuse");
        fixture.write_evidence_sources();
        fixture.write_declaration(
            "d2b-provider-endpoint",
            &declaration(
                "d2b-provider-endpoint",
                "endpoint",
                &["product", "runtime"],
                &slot(
                    Some("endpoint"),
                    None,
                    &["runtime"],
                    &[(NIX_SOURCE, NIX_SYMBOL)],
                ),
                &slot(
                    Some("endpoint"),
                    None,
                    &["product"],
                    &[(RUST_SOURCE, RUST_SYMBOL)],
                ),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let identities = ProviderIdentities::load(&fixture.root).expect("the declaration loads");
        assert_eq!(identities.identity("d2b-provider-endpoint", Surface::Product), Some("endpoint"));
        assert_eq!(identities.identity("d2b-provider-endpoint", Surface::Runtime), Some("endpoint"));
    }

    /// Agreeing values are not a declaration: a repeated identity without
    /// the repetition stated is exactly the identity a second surface would
    /// publish without anyone having said so.
    #[test]
    fn a_repeated_identity_must_be_declared_from_both_surfaces() {
        let fixture = Fixture::new("undeclared-reuse");
        fixture.write_evidence_sources();
        fixture.write_declaration(
            "d2b-provider-endpoint",
            &declaration(
                "d2b-provider-endpoint",
                "endpoint",
                &["product", "runtime"],
                &identity_slot("endpoint", (NIX_SOURCE, NIX_SYMBOL)),
                &identity_slot("endpoint", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("identity-reuse-not-declared")
                && error.contains("sharesIdentityWith"),
            "the refusal names the repetition nobody declared:\n{error}"
        );
    }

    /// One-sided is not declared either: a list naming only the other
    /// surface leaves the reader guessing which of the two agreed.
    #[test]
    fn a_reuse_listing_must_name_the_other_surface_with_the_same_identity() {
        let fixture = Fixture::new("one-sided-reuse");
        fixture.write_evidence_sources();
        fixture.write_declaration(
            "d2b-provider-endpoint",
            &declaration(
                "d2b-provider-endpoint",
                "endpoint",
                &["product", "runtime"],
                &slot(
                    Some("endpoint"),
                    None,
                    &["runtime"],
                    &[(NIX_SOURCE, NIX_SYMBOL)],
                ),
                &identity_slot("endpoint", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("identity-reuse-not-declared"),
            "the runtime surface does not declare the repetition:\n{error}"
        );
    }

    /// A reuse list naming a surface that owns another identity, or the slot's
    /// own surface, is a claim about nothing.
    #[test]
    fn a_reuse_listing_must_name_a_surface_carrying_the_same_identity() {
        let fixture = Fixture::new("mismatched-reuse");
        fixture.write_evidence_sources();
        fixture.write_declaration(
            "d2b-provider-endpoint",
            &declaration(
                "d2b-provider-endpoint",
                "endpoint",
                &["product", "runtime"],
                &slot(
                    Some("endpoint"),
                    None,
                    &["runtime", "product"],
                    &[(NIX_SOURCE, NIX_SYMBOL)],
                ),
                &identity_slot("runtime-endpoint", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("self-shared-identity"),
            "the slot lists its own surface:\n{error}"
        );

        let fixture = Fixture::new("foreign-reuse");
        fixture.write_evidence_sources();
        fixture.write_declaration(
            "d2b-provider-endpoint",
            &declaration(
                "d2b-provider-endpoint",
                "endpoint",
                &["product", "runtime"],
                &slot(
                    Some("endpoint"),
                    None,
                    &["runtime"],
                    &[(NIX_SOURCE, NIX_SYMBOL)],
                ),
                &identity_slot("runtime-endpoint", (RUST_SOURCE, RUST_SYMBOL)),
                &null_slot("no-identity-owned"),
                "[]",
            ),
        );
        assert!(fixture.refuses("identity-reuse-mismatch"));
    }

    /// Two loads of one tree project the same sequence: the joins are what
    /// the generators read, and a generator's output must not depend on
    /// directory order.
    #[test]
    fn the_load_is_ordered_identically_across_runs() {
        let fixture = Fixture::new("deterministic");
        fixture.with_identity_crate();
        fixture.write_crate_owning(
            "d2b-provider-alpha",
            "alpha",
            Surface::Session,
            "session-alpha",
            (NIX_SOURCE, NIX_SYMBOL),
        );
        let project = |identities: &ProviderIdentities| {
            identities
                .declarations()
                .flat_map(|(crate_name, declaration)| {
                    Surface::ALL.into_iter().filter_map(move |surface| {
                        declaration
                            .slot(surface)
                            .identity()
                            .map(|identity| (crate_name.to_owned(), surface, identity.to_owned()))
                    })
                })
                .collect::<Vec<_>>()
        };
        let first = project(&ProviderIdentities::load(&fixture.root).expect("the first load"));
        let second = project(&ProviderIdentities::load(&fixture.root).expect("the second load"));
        assert_eq!(first, second, "identical inputs project identical rows");
        assert_eq!(
            first,
            vec![
                (
                    "d2b-provider-alpha".to_owned(),
                    Surface::Session,
                    "session-alpha".to_owned()
                ),
                (
                    "d2b-provider-fixture".to_owned(),
                    Surface::Product,
                    "system-systemd".to_owned()
                ),
                (
                    "d2b-provider-fixture".to_owned(),
                    Surface::Runtime,
                    "process-systemd".to_owned()
                ),
            ],
            "rows come out in crate-name order and then surface order"
        );
    }

    /// The census: every provider-prefixed crate the tree enumerates carries
    /// exactly one declaration, and the kind is mandatory, so nothing is
    /// read through an absence row any more.
    #[test]
    fn the_census_covers_every_provider_crate_exactly_once() {
        let repo_root = crate::repo_root().expect("the aggregate passes D2B_REPO_ROOT");
        let identities = repository_identities();
        let enumerated = provider_crates(repo_root).expect("the provider crates enumerate");
        assert_eq!(
            identities.crate_names().collect::<Vec<_>>(),
            enumerated
                .iter()
                .map(|(crate_name, _)| crate_name.as_str())
                .collect::<Vec<_>>(),
            "every provider-prefixed crate declares its identities, and nothing else does"
        );
        assert_eq!(
            declaration_paths(repo_root, Declaration::ProviderIdentity)
                .expect("the declaration kind is mandatory for every provider crate")
                .len(),
            enumerated.len(),
            "no crate carries the kind through an absence row any more"
        );
    }

    /// A provider-prefixed crate that arrives without a declaration fails the
    /// census by name: the crates that did declare are still read normally,
    /// and the refusal names only the crate that never carried the file.
    #[test]
    fn a_new_provider_prefixed_crate_without_a_declaration_fails_the_census() {
        let fixture = Fixture::new("census-missing");
        fixture.with_identity_crate();
        fixture.write(
            "packages/d2b-provider-newcomer/src/lib.rs",
            "pub fn driver() {}\n",
        );
        let error = fixture.expect_refusal();
        assert!(
            error.contains("missing-declaration")
                && error.contains("d2b-provider-newcomer")
                && error.contains(Declaration::ProviderIdentity.file_name()),
            "the refusal names the new crate and the declaration it never carried:\n{error}"
        );
        assert!(
            !error.contains("d2b-provider-fixture"),
            "the crate that did declare is read normally and is not dragged into the refusal:\n{error}"
        );
    }

    /// Every crate's declaration has to reach the Bazel runfiles closure: a
    /// manifest that leaves the file out makes the drift action build over an
    /// incomplete source tree and compare nothing.
    #[test]
    fn every_provider_crate_names_its_declaration_in_the_bazel_workspace_sources() {
        let repo_root = crate::repo_root().expect("the aggregate passes D2B_REPO_ROOT");
        let entry = format!("\"{}\",", Declaration::ProviderIdentity.file_name());
        for (crate_name, crate_dir) in
            provider_crates(repo_root).expect("the provider crates enumerate")
        {
            let manifest_path = crate_dir.join("BUILD.bazel");
            let manifest = fs::read_to_string(&manifest_path).unwrap_or_else(|error| {
                panic!(
                    "{} is a readable Bazel manifest: {error}",
                    manifest_path.display()
                )
            });
            assert!(
                manifest.lines().any(|line| line.trim() == entry),
                "{crate_name}/BUILD.bazel carries no {} entry in cargo_workspace_sources, so its runfiles closure drops the declaration and every drift action over it reads an incomplete source tree",
                Declaration::ProviderIdentity.file_name()
            );
        }
    }

    /// The census reads as a classification, not as a count of registrations:
    /// a crate that ships a product identity and registers no runtime Provider
    /// is a product-only crate, not a crate that owns no identity.
    #[test]
    fn the_census_classifies_every_crate_by_the_surfaces_it_owns() {
        let identities = repository_identities();
        let mut by_shape: BTreeMap<String, Vec<&str>> = BTreeMap::new();
        for (crate_name, declaration) in identities.declarations() {
            let owned = Surface::ALL
                .iter()
                .filter(|surface| declaration.slot(**surface).identity().is_some())
                .map(|surface| surface.key())
                .collect::<Vec<_>>();
            let shape = if owned.is_empty() {
                "none".to_owned()
            } else {
                owned.join("+")
            };
            by_shape.entry(shape).or_default().push(crate_name);
        }
        let expected = BTreeMap::from([
            (
                "none".to_owned(),
                vec![
                    "d2b-provider-emergency-policy",
                    "d2b-provider-execution-policy",
                    "d2b-provider-operation",
                    "d2b-provider-provider",
                    "d2b-provider-quota",
                    "d2b-provider-resource-export",
                    "d2b-provider-resource-import",
                    "d2b-provider-role",
                    "d2b-provider-role-binding",
                    "d2b-provider-seccomp-profile",
                    "d2b-provider-supervisor",
                    "d2b-provider-telemetry-binding",
                    "d2b-provider-telemetry-service",
                    "d2b-provider-test-controller",
                    "d2b-provider-toolkit",
                    "d2b-provider-zone",
                    "d2b-provider-zone-link",
                ],
            ),
            (
                "product".to_owned(),
                vec![
                    "d2b-provider-audio-pipewire",
                    "d2b-provider-device-tpm",
                    "d2b-provider-observability-otel",
                    "d2b-provider-process-minijail",
                ],
            ),
            (
                "product+runtime".to_owned(),
                vec![
                    "d2b-provider-activation-nixos",
                    "d2b-provider-credential-entra",
                    "d2b-provider-credential-managed-identity",
                    "d2b-provider-credential-secret-service",
                    "d2b-provider-device-gpu",
                    "d2b-provider-device-security-key",
                    "d2b-provider-device-usbip",
                    "d2b-provider-guest-azure-container-apps",
                    "d2b-provider-guest-azure-virtual-machine",
                    "d2b-provider-guest-cloud-hypervisor",
                    "d2b-provider-guest-qemu-media",
                    "d2b-provider-network-local",
                    "d2b-provider-process-systemd",
                    "d2b-provider-transport-azure-relay",
                    "d2b-provider-transport-vsock",
                    "d2b-provider-volume-local",
                    "d2b-provider-volume-virtiofs",
                ],
            ),
            (
                "product+session".to_owned(),
                vec![
                    "d2b-provider-clipboard-wayland",
                    "d2b-provider-display-wayland",
                    "d2b-provider-notification-desktop",
                    "d2b-provider-shell-terminal",
                    "d2b-provider-system-core",
                ],
            ),
            (
                "runtime".to_owned(),
                vec![
                    "d2b-provider-audio-binding",
                    "d2b-provider-audio-service",
                    "d2b-provider-credential",
                    "d2b-provider-device",
                    "d2b-provider-endpoint",
                    "d2b-provider-guest",
                    "d2b-provider-host",
                    "d2b-provider-process",
                    "d2b-provider-shell-pool",
                    "d2b-provider-shell-session",
                    "d2b-provider-user",
                    "d2b-provider-volume",
                    "d2b-provider-volume-binding",
                    "d2b-provider-wayland-policy",
                    "d2b-provider-wayland-session",
                ],
            ),
            ("session".to_owned(), vec!["d2b-provider-config-nixos"]),
        ]);
        assert_eq!(
            by_shape, expected,
            "every crate falls in exactly one classification, and registration absence is never read as identity absence"
        );
    }

    /// AE1: the systemd Process provider ships the product-plane
    /// `system-systemd` artifact and registers a different runtime identity;
    /// the two slots are independent and the declaration states both.
    #[test]
    fn the_systemd_process_crate_ships_one_identity_and_registers_another() {
        let identities = repository_identities();
        let crate_name = "d2b-provider-process-systemd";
        assert_eq!(
            identities.identity(crate_name, Surface::Product),
            Some("system-systemd")
        );
        assert_eq!(
            identities.identity(crate_name, Surface::Runtime),
            Some("process-systemd")
        );
        assert_eq!(
            identities.identity(crate_name, Surface::Session),
            None,
            "a crate that owns no session routing states that rather than inheriting a neighbour's answer"
        );
        assert_eq!(
            identities
                .declaration(crate_name)
                .expect("the crate declares")
                .roles(),
            &[Role::Product, Role::Runtime]
        );
    }

    /// AE2: `device-tpm` keeps its packaged product identity while the shared
    /// Device driver owns the executable reconciliation, so no runtime
    /// registration is synthesized for it.
    #[test]
    fn the_tpm_crate_keeps_its_product_identity_and_owns_the_shared_driver() {
        let identities = repository_identities();
        let crate_name = "d2b-provider-device-tpm";
        let declaration = identities
            .declaration(crate_name)
            .expect("the crate declares");
        assert_eq!(
            identities.identity(crate_name, Surface::Product),
            Some("device-tpm")
        );
        assert_eq!(identities.identity(crate_name, Surface::Runtime), None);
        assert_eq!(
            identities.shared_driver_crates(),
            vec![crate_name],
            "the crate that declares the shared driver owns the identity that driver serves"
        );
        assert_eq!(
            declaration.slot(Surface::Runtime).reason(),
            Some(NoIdentityReason::CompositionHosted),
            "the null runtime surface names the hosting the composition does, so a registration row could carry neither half"
        );
    }

    /// The fixed-bootstrap join is the whole of R5's ownership statement:
    /// exactly two crates, each reading its packaged product identity, and
    /// neither owning a runtime Provider that a cutover could mint a
    /// ProviderSet registration for.
    #[test]
    fn fixed_bootstrap_is_exactly_the_two_foundation_owned_products() {
        let identities = repository_identities();
        assert_eq!(
            identities.fixed_bootstrap_identities(),
            vec![
                ("d2b-provider-process-minijail", "system-minijail"),
                ("d2b-provider-system-core", "system-core"),
            ],
            "the fixed-bootstrap join reads the product surface, and foundation owns exactly these two"
        );
        for (crate_name, identity) in identities.fixed_bootstrap_identities() {
            assert_eq!(
                identities.identity(crate_name, Surface::Runtime),
                None,
                "{crate_name} is fixed bootstrap through its packaged {identity}, so it owns no runtime Provider"
            );
        }
    }

    /// AE3: `system-minijail` is a fixed-bootstrap product identity owned by
    /// the minijail crate, and no crate on any surface owns a
    /// `process-minijail` identity: the directory name is not an identity.
    #[test]
    fn the_minijail_crate_owns_the_fixed_bootstrap_system_minijail_identity() {
        let identities = repository_identities();
        let crate_name = "d2b-provider-process-minijail";
        let declaration = identities
            .declaration(crate_name)
            .expect("the crate declares");
        assert_eq!(declaration.roles(), &[Role::Product, Role::FixedBootstrap]);
        assert_eq!(
            identities.identity(crate_name, Surface::Product),
            Some("system-minijail")
        );
        assert_eq!(
            identities.identity(crate_name, Surface::Runtime),
            None,
            "the fixed-bootstrap identity is a packaged product, not a ProviderSet registration"
        );
        for surface in Surface::ALL {
            assert!(
                declaration.slot(surface).shares_identity_with().is_empty(),
                "the fixed-bootstrap identity is declared on one surface, so no {surface} slot repeats it"
            );
        }
        for surface in Surface::ALL {
            assert_ne!(
                identities.identity(crate_name, surface),
                Some("process-minijail"),
                "the crate's own directory name names no identity: the {surface} surface states none"
            );
        }
    }

    /// AE4: `endpoint` is a runtime resource family outside the packaged
    /// Provider matrix: it registers a Provider and ships no product.
    #[test]
    fn the_endpoint_family_is_a_runtime_identity_and_no_packaged_product() {
        let identities = repository_identities();
        let crate_name = "d2b-provider-endpoint";
        assert_eq!(
            identities.identity(crate_name, Surface::Runtime),
            Some("endpoint")
        );
        assert_eq!(identities.identity(crate_name, Surface::Product), None);
        assert_eq!(identities.identity(crate_name, Surface::Session), None);
        assert_eq!(
            identities
                .declaration(crate_name)
                .expect("the crate declares")
                .roles(),
            &[Role::Runtime]
        );
    }

    /// AE5: `execution-policy` owns a ResourceType vocabulary and no Provider
    /// identity at all, so the `Provider/execution-policy` reference
    /// `d2b-contracts-resource` publishes names no crate's identity.
    #[test]
    fn the_execution_policy_crate_declares_no_identity_on_any_surface() {
        let identities = repository_identities();
        let crate_name = "d2b-provider-execution-policy";
        let declaration = identities
            .declaration(crate_name)
            .expect("the crate declares");
        assert_eq!(
            declaration.roles(),
            &[Role::NoIdentity, Role::ResourceFamily]
        );
        for surface in Surface::ALL {
            assert_eq!(declaration.slot(surface).identity(), None);
            assert_eq!(
                declaration.slot(surface).reason(),
                Some(NoIdentityReason::NoIdentityOwned)
            );
            assert!(
                identities
                    .identities(surface)
                    .all(|(_, identity)| identity != "execution-policy"),
                "no surface projects a Provider/execution-policy identity"
            );
        }
    }

    /// AE12: a blocker records a tracked issue and claims nothing about the
    /// implementation behind it, so the surfaces that issue prevents stay
    /// null and the three deferred families are the only crates naming one.
    #[test]
    fn a_blocker_names_a_waiting_issue_and_claims_no_executable_readiness() {
        let identities = repository_identities();
        let mut by_crate: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for (crate_name, blocker) in identities.blockers() {
            by_crate
                .entry(crate_name)
                .or_default()
                .push(blocker.issue());
        }
        assert_eq!(
            by_crate,
            BTreeMap::from([
                ("d2b-provider-audio-pipewire", vec!["#629"]),
                ("d2b-provider-observability-otel", vec!["#630"]),
                ("d2b-provider-shell-pool", vec!["#631"]),
                ("d2b-provider-shell-session", vec!["#631"]),
                ("d2b-provider-shell-terminal", vec!["#631"]),
            ]),
            "#629, #630 and #631 stay explicit blocked surfaces on the audio, observability and shell families, and on no other crate"
        );
        for (crate_name, issues) in &by_crate {
            if issues.contains(&"#629") || issues.contains(&"#630") {
                let declaration = identities
                    .declaration(crate_name)
                    .expect("a blocked crate still declares its identities");
                for surface in [Surface::Runtime, Surface::Session] {
                    assert_eq!(
                        declaration.slot(surface).identity(),
                        None,
                        "{crate_name} is blocked on {issues:?}, so its {surface} surface states no identity"
                    );
                }
            }
        }
    }
}
