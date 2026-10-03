//! The one verified deployment bootstrap document (U31, KTD7).
//!
//! One document describes one deployment. It is constructed once, in
//! `nixos-modules/deployment-bootstrap.nix`, and published twice: the Host
//! publishes the copy that names the system Zone, and every Zone's Guests
//! publish their own copy naming that Zone. The documents differ in exactly
//! one field - the Zone they name - and share the schema tag, the canonical
//! encoding, and the framed self-hash domain, so every reader verifies the
//! same bytes against the same profile rather than re-deriving a private
//! contract.
//!
//! # Pure
//!
//! Decoding, verification, and the accepted-graph projection are pure
//! functions of the document's bytes. Nothing here resolves a host path,
//! reads a store, or touches a filesystem, so the daemon, the Activation
//! family, and the one-shot ownership-bounded reset all reach the same
//! answer for the same document.
//!
//! # The root subject is the document, not a caller
//!
//! [`DeploymentBootstrap::accepted_graph`] roots the graph at
//! [`AuthoritySubjectKind::Bootstrap`]: the verified deployment document is
//! the deployment's own trust root. That is the same root every other
//! boundary builds, so an admission decided here cannot be satisfied by a
//! caller choosing a root that matches its own subject.

use std::collections::BTreeSet;

use d2b_contracts_resource::v3::{
    AuthoritySubject, AuthoritySubjectKind, CanonicalJsonObject, CanonicalJsonValue, ResourceRef,
    StoreIncarnation, ZoneId, framed_canonical_digest,
};

use crate::resource_authority::{AcceptedGraph, ProjectionRow};

/// The deployment-root-relative name of the verified deployment document.
///
/// The daemon and the broker share one deployment root, so one deployment
/// publishes one document and both halves of the trust root agree on where
/// it lives.
pub const DEPLOYMENT_BOOTSTRAP_FILE: &str = "deployment-bootstrap.json";

/// The document schema tag this release verifies.
///
/// A document carrying any other tag is an artifact of another contract
/// version and is refused; there is no compatibility parse and no default
/// for a missing or unknown tag (R43).
pub const DEPLOYMENT_BOOTSTRAP_SCHEMA: &str = "d2b-deployment-bootstrap/1";

/// The domain tag framing the document's own self-hash.
///
/// It is the same framed-digest profile the Nix bundle compiler and the
/// artifact catalog already cross with this package, so the document is
/// self-hashed by the same mechanism that verifies every other verified
/// artifact rather than by a second digest spelling.
pub const DEPLOYMENT_BOOTSTRAP_DIGEST_DOMAIN: &str = "d2b:v3:deployment-bootstrap";

/// The bounded read for the deployment document.
pub const MAX_DEPLOYMENT_BOOTSTRAP_BYTES: usize = 4 * 1024 * 1024;

/// Why a deployment document cannot be used.
///
/// Every variant is a refusal to use the document at all; none of them is a
/// permission to proceed with a partially understood one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapRefusal {
    /// The document is absent, unreadable, over the read bound, or not
    /// shaped like this release's contract.
    DocumentUnreadable {
        /// The deployment-root-relative path that was read.
        path: String,
    },
    /// The document's own schema tag is not this release's contract.
    SchemaUnsupported {
        /// The tag the document carried.
        observed: String,
    },
    /// The document's self-hash does not cover its own bytes.
    DigestMismatch {
        /// The digest the document claimed.
        claimed: String,
    },
    /// An authority row the document carries cannot be read as an exact
    /// resource reference, so which resource it grants is unknown.
    UnreadableRow {
        /// The row as the document spelled it.
        reference: String,
    },
    /// One resource reference is declared by two rows, so which bytes this
    /// deployment accepted for it would be a coin flip.
    DuplicateRow {
        /// The duplicated resource reference.
        reference: String,
    },
    /// A binding names a subject that is not an admitted authority class, so
    /// the grant it carries cannot be decided for anybody.
    UnclassifiedSubject {
        /// The binding reference that carries the subject.
        binding: String,
        /// The subject the binding named.
        subject: String,
    },
}

impl core::fmt::Display for BootstrapRefusal {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::DocumentUnreadable { path } => write!(
                formatter,
                "deployment document refused: {path} is absent, unreadable, or over the read bound"
            ),
            Self::SchemaUnsupported { observed } => write!(
                formatter,
                "deployment document refused: schema {observed} is not {DEPLOYMENT_BOOTSTRAP_SCHEMA}"
            ),
            Self::DigestMismatch { claimed } => write!(
                formatter,
                "deployment document refused: the document does not hash to its claimed digest \
                 {claimed}"
            ),
            Self::UnreadableRow { reference } => {
                write!(formatter, "deployment document refused: {reference} is not a readable row")
            }
            Self::DuplicateRow { reference } => write!(
                formatter,
                "deployment document refused: {reference} is declared twice"
            ),
            Self::UnclassifiedSubject { binding, subject } => write!(
                formatter,
                "deployment document refused: {binding} names {subject}, which is not an admitted \
                 authority class"
            ),
        }
    }
}

impl std::error::Error for BootstrapRefusal {}

/// One authority row of the verified deployment document.
///
/// The row travels as the canonical bytes the deployment accepted, so the
/// graph decides exactly what those bytes decide everywhere else.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BootstrapAuthorityRow {
    /// The exact resource reference the row was accepted for.
    pub reference: String,
    /// The row's canonical admitted bytes.
    pub admitted: serde_json::Value,
}

/// The verified deployment document, as published beside the deployment
/// root.
///
/// The document is self-hashed over its own canonical bytes with
/// `graphDigest` removed, so a document whose authority rows or declared
/// implementations were edited after verification fails closed before a
/// single provider is published or a single path is unlinked.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeploymentBootstrap {
    /// The document's own schema tag.
    pub schema_version: String,
    /// The Zone this publication describes.
    pub zone: ZoneId,
    /// The store incarnation the deployment is verified in.
    pub store_incarnation: StoreIncarnation,
    /// The deployment's own state Volume, published with the foundations.
    pub state_volume: String,
    /// The implementation identities this deployment publishes.
    pub implementations: Vec<String>,
    /// The accepted `Role` rows, as canonical bytes.
    pub roles: Vec<BootstrapAuthorityRow>,
    /// The accepted `RoleBinding` rows, as canonical bytes.
    pub role_bindings: Vec<BootstrapAuthorityRow>,
    /// The framed digest over the canonical bytes of this document without
    /// `graphDigest`.
    pub graph_digest: String,
}

impl DeploymentBootstrap {
    /// Decode and verify document bytes.
    ///
    /// The three refusals this can return before anything is acted on are
    /// deliberately distinct: an artifact of another contract version, a
    /// document whose bytes were edited after verification, and a document
    /// that is not shaped like this contract.
    pub fn decode(bytes: &[u8], path: &str) -> Result<Self, BootstrapRefusal> {
        if bytes.is_empty() || bytes.len() > MAX_DEPLOYMENT_BOOTSTRAP_BYTES {
            return Err(BootstrapRefusal::DocumentUnreadable {
                path: path.to_owned(),
            });
        }
        let document: Self =
            serde_json::from_slice(bytes).map_err(|_| BootstrapRefusal::DocumentUnreadable {
                path: path.to_owned(),
            })?;
        document.verify()?;
        Ok(document)
    }

    /// Verify the document's own schema tag and self-hash.
    ///
    /// The Zone is not checked here: the Host publication names the system
    /// Zone and a Guest publication names its own, and both are the same
    /// contract.
    pub fn verify(&self) -> Result<(), BootstrapRefusal> {
        if self.schema_version != DEPLOYMENT_BOOTSTRAP_SCHEMA {
            return Err(BootstrapRefusal::SchemaUnsupported {
                observed: self.schema_version.clone(),
            });
        }

        let bytes = self.canonical_bytes_without_digest()?;
        let observed = framed_canonical_digest(DEPLOYMENT_BOOTSTRAP_DIGEST_DOMAIN, &bytes);
        if observed != self.graph_digest {
            return Err(BootstrapRefusal::DigestMismatch {
                claimed: self.graph_digest.clone(),
            });
        }
        Ok(())
    }

    /// Recompute the self-hash a publisher stamps onto a document.
    ///
    /// It is the same profile [`Self::verify`] checks, so a document
    /// re-sealed through this function verifies: an editor that cannot
    /// produce a verifying self-hash has produced a document nothing
    /// accepts.
    pub fn seal(&self) -> Result<String, BootstrapRefusal> {
        Ok(framed_canonical_digest(
            DEPLOYMENT_BOOTSTRAP_DIGEST_DOMAIN,
            &self.canonical_bytes_without_digest()?,
        ))
    }

    /// The canonical bytes the self-hash covers: this document with its own
    /// `graphDigest` field removed.
    ///
    /// The publisher hashes the document it is about to write, which has no
    /// digest field yet, so verification removes the field rather than
    /// blanking it. Clearing it instead would hash a different byte string
    /// from the one the publisher hashed, and two readers of the same
    /// document would disagree about it.
    pub fn canonical_bytes_without_digest(&self) -> Result<Vec<u8>, BootstrapRefusal> {
        let rendered = serde_json::to_vec(self).map_err(|_| {
            BootstrapRefusal::DocumentUnreadable {
                path: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
            }
        })?;
        let mut document = CanonicalJsonValue::parse(&rendered).map_err(|_| {
            BootstrapRefusal::DocumentUnreadable {
                path: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
            }
        })?;
        let CanonicalJsonValue::Object(fields) = &mut document else {
            return Err(BootstrapRefusal::DocumentUnreadable {
                path: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
            });
        };
        if fields.remove("graphDigest").is_none() {
            return Err(BootstrapRefusal::DocumentUnreadable {
                path: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
            });
        }
        Ok(document.to_canonical_bytes())
    }

    /// The deployment's own state Volume, as the exact reference the
    /// foundations publish.
    pub fn state_volume_ref(&self) -> Result<ResourceRef, BootstrapRefusal> {
        ResourceRef::parse(self.state_volume.as_str()).map_err(|_| {
            BootstrapRefusal::UnreadableRow {
                reference: self.state_volume.clone(),
            }
        })
    }

    /// The accepted authority rows, decoded to their canonical objects.
    ///
    /// The returned objects own their bytes, so the projection rows built
    /// from them never point into a collection this call already dropped.
    fn decoded_rows(&self) -> Result<Vec<(ResourceRef, CanonicalJsonObject)>, BootstrapRefusal> {
        let mut decoded: Vec<(ResourceRef, CanonicalJsonObject)> = Vec::new();
        let mut seen = BTreeSet::new();
        for rows in [&self.roles, &self.role_bindings] {
            for row in rows {
                let reference =
                    ResourceRef::parse(row.reference.as_str()).map_err(|_| {
                        BootstrapRefusal::UnreadableRow {
                            reference: row.reference.clone(),
                        }
                    })?;
                if !seen.insert(reference.to_canonical_string()) {
                    return Err(BootstrapRefusal::DuplicateRow {
                        reference: row.reference.clone(),
                    });
                }
                let admitted =
                    serde_json::from_value::<CanonicalJsonObject>(row.admitted.clone()).map_err(
                        |_| BootstrapRefusal::UnreadableRow {
                            reference: row.reference.clone(),
                        },
                    )?;
                decoded.push((reference, admitted));
            }
        }
        Ok(decoded)
    }

    /// The prior accepted graph this deployment bootstraps from.
    ///
    /// Built through [`AcceptedGraph::from_canonical_rows`] so the graph
    /// decides exactly what the same rows decide at every other boundary,
    /// and rooted at the verified deployment document rather than at any
    /// grant the document itself introduces. A caller that then presents a
    /// subject is decided by the rows, not by the root it chose.
    pub fn accepted_graph(&self) -> Result<AcceptedGraph, BootstrapRefusal> {
        let decoded = self.decoded_rows()?;
        let rows = decoded
            .iter()
            .map(|(reference, admitted)| ProjectionRow::new(reference, admitted));
        AcceptedGraph::from_canonical_rows(
            self.zone.clone(),
            self.store_incarnation.clone(),
            AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap),
            rows,
        )
        .map_err(|_| BootstrapRefusal::UnreadableRow {
            reference: DEPLOYMENT_BOOTSTRAP_FILE.to_owned(),
        })
    }

    /// Every identity this deployment granted a grant to, in index order.
    ///
    /// These are the subjects the accepted `RoleBinding` rows name, resolved
    /// from the document's own committed bytes. They are the only identities
    /// an accepted graph can admit, so a caller that needs to know which one
    /// may act reads the deployment rather than assuming an identity.
    ///
    /// A binding that names a subject outside the admitted authority classes
    /// is a refusal rather than a silently skipped row: a grant nobody can
    /// be is not a narrower grant, it is an unreadable deployment.
    pub fn bound_subjects(&self) -> Result<Vec<AuthoritySubject>, BootstrapRefusal> {
        let graph = self.accepted_graph()?;
        let mut subjects: Vec<AuthoritySubject> = Vec::new();
        for (binding_ref, binding) in graph.role_bindings() {
            for subject in binding.subjects() {
                let Some(kind) = AuthoritySubjectKind::of_reference(subject) else {
                    return Err(BootstrapRefusal::UnclassifiedSubject {
                        binding: binding_ref.to_canonical_string(),
                        subject: subject.to_canonical_string(),
                    });
                };
                let resolved = AuthoritySubject::named(kind, subject.clone());
                if !subjects.contains(&resolved) {
                    subjects.push(resolved);
                }
            }
        }
        Ok(subjects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const IMPLEMENTATIONS: &[&str] = &["activation-nixos", "process"];

    fn publisher_role() -> serde_json::Value {
        json!({
            "operationRefs": [],
            "rules": [{
                "executionRefs": [],
                "resourceNames": [],
                "resourceTypes": ["Operation"],
                "sessionVerbs": [],
                "subresources": [],
                "verbs": ["create"],
                "zones": [],
            }],
        })
    }

    fn document() -> DeploymentBootstrap {
        let mut document = DeploymentBootstrap {
            schema_version: DEPLOYMENT_BOOTSTRAP_SCHEMA.to_owned(),
            zone: ZoneId::parse("system").unwrap(),
            store_incarnation: StoreIncarnation::parse("foundation-1").unwrap(),
            state_volume: "Volume/d2b-state".to_owned(),
            implementations: IMPLEMENTATIONS.iter().map(|name| (*name).to_owned()).collect(),
            roles: vec![BootstrapAuthorityRow {
                reference: "Role/operation-publisher".to_owned(),
                admitted: publisher_role(),
            }],
            role_bindings: vec![BootstrapAuthorityRow {
                reference: "RoleBinding/system-minijail-self-operation-publisher".to_owned(),
                admitted: json!({
                    "roleRef": "Role/operation-publisher",
                    "subjects": ["Provider/system-minijail"],
                }),
            }],
            graph_digest: String::new(),
        };
        document.graph_digest = document.seal().expect("seal the document");
        document
    }

    fn seal(document: &DeploymentBootstrap) -> DeploymentBootstrap {
        let mut sealed = document.clone();
        sealed.graph_digest = sealed.seal().expect("seal the document");
        sealed
    }

    #[test]
    fn the_document_round_trips_through_its_own_digest() {
        let document = document();
        let bytes = serde_json::to_vec(&document).unwrap();
        assert_eq!(
            DeploymentBootstrap::decode(&bytes, DEPLOYMENT_BOOTSTRAP_FILE).unwrap(),
            document
        );
    }

    #[test]
    fn another_contract_version_is_refused_even_when_the_digest_matches() {
        let mut document = document();
        document.schema_version = "d2b-deployment-bootstrap/0".to_owned();
        let document = seal(&document);
        assert_eq!(
            document.verify(),
            Err(BootstrapRefusal::SchemaUnsupported {
                observed: "d2b-deployment-bootstrap/0".to_owned(),
            })
        );
    }

    #[test]
    fn an_ownership_description_edited_after_verification_is_refused() {
        let mut document = document();
        document.state_volume = "Volume/somebody-elses".to_owned();
        assert_eq!(
            document.verify(),
            Err(BootstrapRefusal::DigestMismatch {
                claimed: document.graph_digest.clone(),
            })
        );
    }

    #[test]
    fn the_accepted_graph_is_rooted_at_the_document_and_grants_only_the_named_subject() {
        let document = document();
        let graph = document.accepted_graph().unwrap();
        assert_eq!(
            graph.root_subject(),
            &AuthoritySubject::unresourced(AuthoritySubjectKind::Bootstrap)
        );
        let subjects = document.bound_subjects().unwrap();
        assert_eq!(
            subjects,
            vec![AuthoritySubject::named(
                AuthoritySubjectKind::Provider,
                ResourceRef::parse("Provider/system-minijail").unwrap(),
            )]
        );
    }

    #[test]
    fn a_binding_naming_a_reference_outside_the_authority_classes_is_refused() {
        let mut document = document();
        document.role_bindings[0].admitted = json!({
            "roleRef": "Role/operation-publisher",
            "subjects": ["Zone/system"],
        });
        let document = seal(&document);
        assert_eq!(
            document.bound_subjects(),
            Err(BootstrapRefusal::UnclassifiedSubject {
                binding: "RoleBinding/system-minijail-self-operation-publisher".to_owned(),
                subject: "Zone/system".to_owned(),
            })
        );
    }

    #[test]
    fn a_row_declared_twice_is_refused_rather_than_resolved_by_order() {
        let mut document = document();
        let duplicate = document.roles[0].clone();
        document.role_bindings[0].admitted = json!({
            "roleRef": "Role/operation-publisher",
            "subjects": ["Provider/system-minijail"],
        });
        document.roles.push(duplicate);
        let document = seal(&document);
        assert_eq!(
            document.accepted_graph(),
            Err(BootstrapRefusal::DuplicateRow {
                reference: "Role/operation-publisher".to_owned(),
            })
        );
    }
}
