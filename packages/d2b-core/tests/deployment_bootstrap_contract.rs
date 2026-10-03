//! The Nix-produced deployment document and the Rust verifier that reads it.
//!
//! `nixos-modules/deployment-bootstrap.nix` is the one constructor for this
//! document: the Host publishes the copy naming the system Zone, and every
//! Zone's Guest image carries the copy naming that Zone. The bytes read here
//! are that constructor's own `documentFor "work"` output, materialized by the
//! fixture aggregate - not a transcription of it - so what this decodes is
//! what a real Guest image closure carries at the path `d2bd guest` reads it
//! from.
//!
//! Three claims, each one a disagreement between the two halves that this
//! would catch before a deployment boots or refuses to:
//!
//! 1. The producer's bytes decode and verify under the deployment-root name
//!    both halves read. The schema tag, the camelCase field names, the
//!    canonical preimage with `graphDigest` removed, and the framed self-hash
//!    domain are one contract rather than two spellings of it.
//! 2. The document describes the Zone its publication is for and carries the
//!    fixed foundation vocabulary - the publisher `Role` and the Process
//!    provider's own self-binding - and nothing else, so the authority a
//!    Guest publishes is exactly what the publisher wrote.
//! 3. One edited field is refused by name. A document whose rows were changed
//!    after the digest was stamped fails closed rather than being partially
//!    understood.
//!
//! The Zone-agnostic half of the contract - that the daemon's guest boot path
//! accepts these same bytes for the Zone they name - is owned by
//! `packages/d2bd/src/composition.rs`, beside the verifier that runs it.

use std::path::Path;

use d2b_core::deployment_bootstrap::{
    BootstrapRefusal, DEPLOYMENT_BOOTSTRAP_FILE, DEPLOYMENT_BOOTSTRAP_SCHEMA,
    DeploymentBootstrap,
};

/// The Zone the rendered fixture publication names.
const ZONE: &str = "work";

/// The rendered per-Zone document, or a panic naming why it is absent.
///
/// The fixture aggregate always sets `D2B_FIXTURES` for this target (see the
/// `env` in `packages/d2b-core/BUILD.bazel`); there is no fallback rendering
/// here, because a fallback would be exactly the hand-copied document this
/// case exists to replace.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn rendered_document() -> Vec<u8> {
    let root = std::env::var_os("D2B_FIXTURES")
        .expect("D2B_FIXTURES names the rendered fixture directory this target declares");
    let path = Path::new(&root).join(format!("deployment-bootstrap-{ZONE}.json"));
    std::fs::read(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

/// The document's rows, as the exact resource references they were accepted
/// under.
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn row_references(rows: &[d2b_core::deployment_bootstrap::BootstrapAuthorityRow]) -> Vec<String> {
    rows.iter().map(|row| row.reference.clone()).collect()
}

/// The producer's per-Zone document verifies under the name both halves read.
#[test]
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn the_rendered_deployment_document_verifies_under_the_name_d2bd_reads() {
    let bytes = rendered_document();
    let document = DeploymentBootstrap::decode(&bytes, DEPLOYMENT_BOOTSTRAP_FILE)
        .unwrap_or_else(|error| panic!("the Nix producer's document was refused: {error}"));

    assert_eq!(document.schema_version, DEPLOYMENT_BOOTSTRAP_SCHEMA);
    assert_eq!(
        document.zone.as_str(),
        ZONE,
        "the publication names the Zone it is for, and `d2bd guest` reads no other"
    );
    assert_eq!(document.state_volume, "Volume/d2b-state");
    assert_eq!(document.store_incarnation.as_str(), "foundation-1");
    assert_eq!(
        document.graph_digest,
        document
            .seal()
            .expect("the document hashes over its own canonical bytes"),
        "the stamped digest is the one this verifier recomputes, not a field the producer filled in"
    );
}

/// The published authority vocabulary is the foundation's, whole and no more:
/// the publisher `Role` and the Process provider's own self-binding. A Guest
/// that published anything else would be publishing authority the producer
/// never wrote.
#[test]
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn the_rendered_document_carries_only_the_foundation_vocabulary() {
    let bytes = rendered_document();
    let document = DeploymentBootstrap::decode(&bytes, DEPLOYMENT_BOOTSTRAP_FILE)
        .unwrap_or_else(|error| panic!("the Nix producer's document was refused: {error}"));

    assert_eq!(row_references(&document.roles), vec!["Role/operation-publisher"]);
    assert_eq!(
        row_references(&document.role_bindings),
        vec!["RoleBinding/system-minijail-self-operation-publisher"],
        "the process provider is the one whose self-binding authorizes materialization, so it is \
         the one row the publication names"
    );

    let bindings = document
        .accepted_graph()
        .expect("the rendered rows are accepted canonical rows")
        .role_bindings()
        .map(|(reference, _)| reference.to_canonical_string())
        .collect::<Vec<_>>();
    assert_eq!(
        bindings,
        vec!["RoleBinding/system-minijail-self-operation-publisher".to_owned()],
        "this is the exact binding list the Guest publishes as its target authority"
    );
}

/// The implementation identities are published sorted and once each, so the
/// document's bytes do not depend on readdir order and a second projection of
/// the same declarations compares equal.
#[test]
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn the_rendered_implementation_identities_are_sorted_and_unique() {
    let bytes = rendered_document();
    let document = DeploymentBootstrap::decode(&bytes, DEPLOYMENT_BOOTSTRAP_FILE)
        .unwrap_or_else(|error| panic!("the Nix producer's document was refused: {error}"));

    assert!(
        !document.implementations.is_empty(),
        "a publication that declared no implementation would boot no provider at all"
    );
    let mut sorted = document.implementations.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(
        document.implementations, sorted,
        "the producer publishes each identity once, in sorted order"
    );
}

/// One edited field is refused by name.
///
/// The edit is the authority-bearing one: the document still parses, still
/// carries this release's schema tag, and still names a readable Zone. What
/// changed is a row the digest no longer covers, and the refusal says exactly
/// that rather than admitting a document it only partly understands.
#[test]
#[allow(clippy::disallowed_methods, reason = "synchronous path")]
fn an_edited_field_is_refused_by_name() {
    let bytes = rendered_document();
    let document = DeploymentBootstrap::decode(&bytes, DEPLOYMENT_BOOTSTRAP_FILE)
        .unwrap_or_else(|error| panic!("the Nix producer's document was refused: {error}"));

    let rendered = String::from_utf8(bytes).expect("the document renders as UTF-8");
    let edited = rendered.replace(
        "\"reference\":\"Role/operation-publisher\"",
        "\"reference\":\"Role/anything\"",
    );
    assert_ne!(
        edited, rendered,
        "the edit must land on the rendered document, not on a string it does not contain"
    );

    assert_eq!(
        DeploymentBootstrap::decode(edited.as_bytes(), DEPLOYMENT_BOOTSTRAP_FILE),
        Err(BootstrapRefusal::DigestMismatch {
            claimed: document.graph_digest,
        }),
        "an authority row renamed after the digest was stamped is refused by the digest it claims"
    );
}