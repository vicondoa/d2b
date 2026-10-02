//! The numeric principal of one committed Zone consumer, derived by the broker.
//!
//! A privileged effect the broker performs on a consumer's behalf - posting an
//! ACL entry on the exact endpoint that consumer reaches, chowning a directory
//! to it, opening a host path under its identity - needs a number, not a name.
//! The number is not on the wire and not on the row: a `Process` spec commits
//! its `executionRef`, its domain, its `userRef`, and its template, and the
//! numeric uid those resolve to is the id the row's host account holds - the
//! account the host layer provisions per Zone, read back through the account
//! database rather than derived a second time. A row whose host has no such
//! account resolves to no principal at all, so this module is where that
//! refusal is performed, and nothing here reads a number a caller supplied.
//!
//! The broker reaches this the same way it reaches every other fact: the
//! verified Zone resource bundle reloaded from `ServerConfig.bundle_path` on the
//! request being served. That bundle's per-Zone template bindings are the
//! committed executable bindings for the Zone's rows, they are hash-pinned
//! alongside the rows themselves, and `d2bd` ingests those same bytes into the
//! Zone's desired store - so the row this resolves from is the same row the
//! store commits, not a second identity that could drift from it.
//!
//! A caller that also carries a claim gets it checked, never trusted:
//! [`repin_consumer_principal`] resolves the principal independently and
//! refuses a claim that does not reproduce it, which is the same fence
//! [`super::device_worker::repin_launch_scope`] and
//! `security_key_authority_binding` apply to a Device scope and a security-key
//! selector. Nothing downstream reads the claim's copy either way.

use d2b_contracts_resource::v3::{ResourceRef, ResourceUid};
use d2b_core::bundle_resolver::{BundleResolver, ConsumerPrincipal};

/// The closed reason a committed consumer's principal could not be resolved.
///
/// Each variant names the committed fact that was missing or that the caller's
/// claim disagreed with, so a refusal is filed against the field the verified
/// bundle answered for rather than collapsing into an opaque effect failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumerPrincipalError {
    /// No verified bundle on this host carries the Zone self-resource uid the
    /// effect was scoped to.
    ZoneUnresolved,
    /// The reference is not a row type that runs as a principal at all.
    NotAConsumer { consumer: String },
    /// The Zone's verified bundle declares no committed row - and therefore no
    /// executable binding - for this consumer reference.
    RowUnresolved { consumer: String },
    /// The consumer's claim does not reproduce what the verified bundle
    /// derived, so the claim is refused rather than repaired.
    ClaimMismatch { claimed: String, derived: String },
}

impl std::fmt::Display for ConsumerPrincipalError {
    /// The closed, path-free slug a refusal and its audit record carry. The
    /// references themselves stay in the structured fields: this string is what
    /// a broker envelope surfaces.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ZoneUnresolved => "consumer-principal-zone-unresolved",
            Self::NotAConsumer { .. } => "consumer-principal-not-a-consumer-row",
            Self::RowUnresolved { .. } => "consumer-principal-row-unresolved",
            Self::ClaimMismatch { .. } => "consumer-principal-claim-mismatch",
        })
    }
}

/// The `(uid, gid)` pair a refusal reports for a claimed and a derived
/// principal.
///
/// A number, not a path or a reference: the point of the refusal record is to
/// say which two principals disagreed, and both of them are numbers the broker
/// already holds.
pub(crate) fn principal_label(principal: &ConsumerPrincipal) -> String {
    format!("{}:{}", principal.uid, principal.gid)
}

/// Resolve the numeric principal one committed consumer reference runs as.
///
/// `zone_uid` and `consumer_ref` are the effect's committed scope: the Zone
/// self-resource identity and the exact `Process` / `EphemeralProcess` row the
/// effect acts on behalf of. Both are resolved against the verified Zone
/// resource bundle and against nothing else, so a Zone uid this host has no
/// verified bundle for, a reference that is not a consumer row, and a row the
/// Zone does not declare all refuse rather than borrow another Zone's or
/// another row's number.
pub(crate) fn resolve_consumer_principal(
    resolver: &BundleResolver,
    zone_uid: &ResourceUid,
    consumer_ref: &ResourceRef,
) -> Result<ConsumerPrincipal, ConsumerPrincipalError> {
    let zone = resolver
        .zone_name_for_uid(zone_uid)
        .ok_or(ConsumerPrincipalError::ZoneUnresolved)?;
    resolver
        .consumer_principal(zone, consumer_ref)
        .ok_or_else(|| {
            if matches!(
                consumer_ref.resource_type().as_str(),
                "Process" | "EphemeralProcess"
            ) {
                ConsumerPrincipalError::RowUnresolved {
                    consumer: consumer_ref.to_canonical_string(),
                }
            } else {
                ConsumerPrincipalError::NotAConsumer {
                    consumer: consumer_ref.to_canonical_string(),
                }
            }
        })
}

/// Re-pin a claimed consumer principal against what the broker derived.
///
/// The claim is a check, never a source: the returned principal is always the
/// derived one, so nothing downstream can act on the claim's copy even when it
/// agrees. A claim that does not reproduce the derivation is refused with both
/// principals named, which is what makes this the same fence the security-key
/// authority binding and the Device-worker launch scope apply - the caller
/// proves it resolved the same committed row, and a caller that did not is
/// refused instead of obeyed.
pub(crate) fn repin_consumer_principal(
    resolver: &BundleResolver,
    zone_uid: &ResourceUid,
    consumer_ref: &ResourceRef,
    claimed: &ConsumerPrincipal,
) -> Result<ConsumerPrincipal, ConsumerPrincipalError> {
    let derived = resolve_consumer_principal(resolver, zone_uid, consumer_ref)?;
    if claimed != &derived {
        return Err(ConsumerPrincipalError::ClaimMismatch {
            claimed: principal_label(claimed),
            derived: principal_label(&derived),
        });
    }
    Ok(derived)
}

#[cfg(test)]
mod tests {
    use super::*;
    use d2b_contracts_resource::v3::resource_schema::{
        CanonicalJsonValue, canonical_json_bytes, framed_canonical_digest,
    };
    use d2b_core::bundle::{Bundle, BundleGeneration};
    use d2b_core::bundle_resolver::DEVICE_TPM_PROVIDER_REF;
    use d2b_core::manifest_v04::ManifestV04;
    use d2b_core::processes::ProcessesJson;
    use std::collections::BTreeMap;

    /// The Zone self-resource uid the `work` fixture bundle is bound to.
    const WORK_ZONE_UID: &str = "123e4567-e89b-42d3-a456-426614174000";
    /// The Zone self-resource uid the `personal` fixture bundle is bound to.
    const PERSONAL_ZONE_UID: &str = "123e4567-e89b-42d3-a456-4266141740ff";

    /// One authored `Process` row owned by the Provider that runs it, carrying
    /// the `spec` fields the row commits and the launch lookup reads: the
    /// template it declares, the execution target it names, and the controller
    /// classification that makes it a Provider-owned controller row.
    fn consumer_row(
        zone: &str,
        name: &str,
        owner_ref: &str,
        host: &str,
        template: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Process",
            "metadata": {
                "name": name,
                "zone": zone,
                "ownerRef": owner_ref,
            },
            "spec": {
                "domain": "system",
                "executionRef": format!("Host/{host}"),
                "processClass": "controller",
                "providerRef": "Provider/system-minijail",
                "template": template,
            },
        })
    }

    /// The canonical content hash one fixture resource array hashes to, computed
    /// the way `ResourceBundle` computes it, so the fixture bundle verifies.
    fn fixture_content_hash(resources: &[serde_json::Value]) -> String {
        let array = serde_json::Value::Array(resources.to_vec());
        let canonical = CanonicalJsonValue::parse(
            &serde_json::to_vec(&array).expect("fixture resources serialize"),
        )
        .expect("fixture resources are canonical JSON");
        framed_canonical_digest(
            "d2b:v3:resource-bundle",
            &canonical_json_bytes(&canonical).expect("fixture resources encode"),
        )
    }

    /// One Zone resource bundle declaring a single template-bound `Process`
    /// consumer row, in the shape the compiler emits: the owning `Provider`
    /// row and the consumer row under `resources`, and the executable binding
    /// that gives the row a principal under `processTemplates`.
    fn zone_bundle(zone: &str, zone_uid: &str, row_name: &str) -> Vec<u8> {
        let host = format!("{zone}-host");
        let provider = serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "Provider",
            "metadata": { "name": "device-tpm", "zone": zone },
            "spec": { "artifactId": "device-tpm" },
        });
        let consumer = consumer_row(
            zone,
            row_name,
            DEVICE_TPM_PROVIDER_REF,
            &host,
            "consumer-worker",
        );
        // `ResourceBundle::verify` requires rows sorted by `(type, name)`.
        let resources = vec![consumer, provider];
        let binding = serde_json::json!({
            "processRef": format!("Process/{row_name}"),
            "ownerRef": DEVICE_TPM_PROVIDER_REF,
            "executionRef": format!("Host/{host}"),
            "template": "consumer-worker",
            "artifactId": "device-tpm",
            "binaryRef": "swtpm",
            "artifactDigest": format!("sha256:{}", "a".repeat(64)),
            "binaryPath": "/nix/store/device-tpm/bin/swtpm",
        });
        serde_json::to_vec(&serde_json::json!({
            "schemaVersion": 3,
            "bundleVersion": 1,
            "zone": zone,
            "zoneUid": zone_uid,
            "contentHash": fixture_content_hash(&resources),
            "artifactCatalogDigest": format!("sha256:{}", "c".repeat(64)),
            "schemaFingerprints": {},
            "providerSchemaDigests": {},
            "resources": resources,
            "processTemplates": [binding],
            "generatedAt": "1970-01-01T00:00:00.000Z",
        }))
        .expect("fixture zone resource bundle serializes")
    }

    /// A resolver carrying the given Zones' committed bundles, assembled the way
    /// the loader assembles them from the verified artifact bytes.
    fn resolver(zones: &[(&str, &str)]) -> BundleResolver {
        BundleResolver::from_artifacts_with_zone_resource_bundles(
            Bundle {
                bundle_version: 11,
                schema_version: "v2".to_owned(),
                storage_path: None,
                realm_workloads_launcher_v2_path: None,
                generation: BundleGeneration {
                    generator: "test".to_owned(),
                    source_revision: None,
                    generated_at: None,
                },
                bundle_hash: None,
                artifact_hashes: None,
            },
            serde_json::from_str(include_str!(
                "../../../../tests/fixtures/deny-unknown/host-valid.json"
            ))
            .expect("host fixture parses"),
            ProcessesJson {
                schema_version: "v2".to_owned(),
                vms: Vec::new(),
            },
            ManifestV04::from_slice(
                include_str!("../../../../tests/golden/manifest_v04/baseline-vms.json").as_bytes(),
            )
            .expect("manifest fixture parses"),
            BTreeMap::from_iter(
                zones
                    .iter()
                    .map(|(zone, uid)| (zone.to_string(), zone_bundle(zone, uid, "shell"))),
            ),
        )
    }

    /// The `work` fixture: one Zone, one declared `Process/shell` consumer row.
    fn work_resolver() -> BundleResolver {
        resolver(&[("work", WORK_ZONE_UID)])
    }

    fn uid(value: &str) -> ResourceUid {
        ResourceUid::parse(value).expect("fixture zone uid")
    }

    fn process(name: &str) -> ResourceRef {
        ResourceRef::parse(&format!("Process/{name}")).expect("fixture row ref")
    }

    /// A committed consumer reference resolves to the principal its row's host
    /// account holds, and to nothing at all where the host has provisioned no
    /// account for that row.
    ///
    /// The equality this used to assert - the resolved principal against the
    /// uid/gid the launch path applies - needs a row that runs as a real
    /// account, and this fixture host provisions none, so what is left is the
    /// half the contract turns on: a committed row with no host account
    /// produces no principal, so no effect can name its uid and no ACL entry
    /// is ever written for it. The broker answers its own closed refusal
    /// rather than inventing the number.
    #[test]
    fn a_committed_consumer_row_with_no_host_account_resolves_to_nothing() {
        let resolver = work_resolver();
        let row = process("shell");

        assert_eq!(
            resolve_consumer_principal(&resolver, &uid(WORK_ZONE_UID), &row),
            Err(ConsumerPrincipalError::RowUnresolved {
                consumer: "Process/shell".to_owned(),
            }),
            "a committed row whose host has provisioned no account resolves to \
             no principal, so the grant path writes no ACL entry for it"
        );
        assert_eq!(
            resolver.consumer_principal("work", &row),
            None,
            "and the resolver holds no entry for that row to be read from"
        );
    }

    /// The broker resolves the principal itself, so a caller that carries a
    /// claim cannot select a uid by asserting one: what comes back is the
    /// row's own resolution or its refusal, never the claim's copy.
    #[test]
    fn a_caller_supplied_claim_is_never_answered_with_its_own_numbers() {
        let resolver = work_resolver();
        let zone_uid = uid(WORK_ZONE_UID);
        let row = process("shell");
        let claimed = ConsumerPrincipal {
            uid: 50_001,
            gid: 50_001,
        };

        assert_eq!(
            repin_consumer_principal(&resolver, &zone_uid, &row, &claimed),
            Err(ConsumerPrincipalError::RowUnresolved {
                consumer: "Process/shell".to_owned(),
            }),
            "a claim over a row with no host account is answered with the row's \
             own refusal, never with the uid the caller asserted"
        );
        assert_eq!(
            claimed,
            ConsumerPrincipal {
                uid: 50_001,
                gid: 50_001
            },
            "and the claim never becomes the effect's principal"
        );
    }

    /// A row this Zone does not declare, a reference that is not a consumer row,
    /// and a Zone uid this host has no verified bundle for each refuse - none of
    /// them borrows another row's or another Zone's principal.
    #[test]
    fn an_uncommitted_consumer_resolves_to_nothing() {
        let resolver = work_resolver();
        let zone_uid = uid(WORK_ZONE_UID);

        assert_eq!(
            resolve_consumer_principal(&resolver, &zone_uid, &process("undeclared")),
            Err(ConsumerPrincipalError::RowUnresolved {
                consumer: "Process/undeclared".to_owned(),
            }),
            "a row the Zone does not declare resolves to no principal"
        );
        assert_eq!(
            resolve_consumer_principal(
                &resolver,
                &zone_uid,
                &ResourceRef::parse("Volume/shell").expect("row ref"),
            ),
            Err(ConsumerPrincipalError::NotAConsumer {
                consumer: "Volume/shell".to_owned(),
            }),
            "a row type that runs as no principal is refused by type"
        );
        assert_eq!(
            resolve_consumer_principal(
                &resolver,
                &uid("123e4567-e89b-42d3-a456-4266141740aa"),
                &process("shell"),
            ),
            Err(ConsumerPrincipalError::ZoneUnresolved),
            "a Zone uid with no verified bundle resolves to no principal"
        );
    }

    /// One row reference declared in two Zones is resolved per Zone, and where
    /// the host has provisioned no account for either, neither Zone answers for
    /// the row name the other declares. Each Zone refuses on its own committed
    /// facts rather than borrowing the sibling's.
    #[test]
    fn one_row_reference_in_two_zones_is_resolved_per_zone() {
        let resolver = resolver(&[("work", WORK_ZONE_UID), ("personal", PERSONAL_ZONE_UID)]);
        let row = process("shell");

        for (zone, zone_uid) in [("work", WORK_ZONE_UID), ("personal", PERSONAL_ZONE_UID)] {
            assert_eq!(
                resolve_consumer_principal(&resolver, &uid(zone_uid), &row),
                Err(ConsumerPrincipalError::RowUnresolved {
                    consumer: "Process/shell".to_owned(),
                }),
                "{zone}: the row this Zone declares resolves on this Zone's own \
                 account lookup, and never on the sibling Zone's"
            );
        }
    }

    /// The broker's own per-request bundle load: the artifacts written to a
    /// scratch directory, read back through the no-follow / ownership /
    /// artifact-hash checks, and the Zone-keyed tables re-derived from the
    /// verified bytes. This is the same
    /// [`BundleResolver::load_with_policy`] `load_kernel_resolver` runs before
    /// every bundle-dependent effect, so the refusal below is the one the
    /// broker's own verified bytes produce, not one an in-memory assembly
    /// invented.
    #[test]
    fn the_brokers_own_bundle_load_resolves_no_principal_without_a_host_account() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt as _;

        use d2b_core::bundle_resolver::BundleVerifyPolicy;
        use sha2::Digest as _;

        let dir = tempfile::TempDir::new().expect("scratch bundle root");
        let root = dir.path();

        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        let write_artifact = |name: &str, bytes: &[u8]| -> String {
            let path = root.join(name);
            if let Some(parent) = path.parent() {
                #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
                fs::create_dir_all(parent).expect("create artifact dir");
            }
            #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
            fs::write(&path, bytes).expect("write artifact");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).expect("chmod artifact");
            format!(
                "sha256:{}",
                sha2::Sha256::digest(bytes)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>()
            )
        };

        let zone_key = "zones/work/resource-bundle.json";
        let zone_hash = write_artifact(
            zone_key,
            &zone_bundle("work", WORK_ZONE_UID, "shell"),
        );

        // The sealed cross-Zone index the v3 loader requires beside a Zone
        // resource bundle: one Zone, no parent, and the framed canonical digest
        // the loader re-derives.
        let parent_map = BTreeMap::from([("work".to_owned(), None::<String>)]);
        let index_hash = write_artifact(
            "index.json",
            &serde_json::to_vec(&serde_json::json!({
                "schemaVersion": "v1",
                "zones": { "work": {} },
                "topology": {
                    "sealed": true,
                    "parentMap": { "work": null },
                    "parentMapDigest": framed_canonical_digest(
                        "d2b:v3:parent-topology",
                        &serde_json::to_vec(&parent_map).expect("parent map bytes"),
                    ),
                    "generationByZone": { "work": "sha256:zone" },
                },
            }))
            .expect("index bytes"),
        );

        // The index's self-hash covers the document with `artifactHashes`
        // nulled and no `bundleHash`, exactly as the loader re-derives it.
        let mut bundle = serde_json::json!({
            "artifactHashes": serde_json::Value::Null,
            "bundleVersion": 1,
            "schemaVersion": "v3",
            "zones": [{ "zone": "work", "path": zone_key }],
            "generation": {
                "generator": "nixos-modules/bundle.nix",
                "sourceRevision": null,
                "generatedAt": null,
            },
        });
        let bundle_hash = format!(
            "sha256:{}",
            sha2::Sha256::digest(
                serde_json::to_vec(&bundle).expect("bundle hash preimage")
            )
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
        );
        bundle["bundleHash"] = serde_json::Value::String(bundle_hash);
        bundle["artifactHashes"] = serde_json::json!({
            zone_key: zone_hash,
            "index.json": index_hash,
        });
        let bundle_path = root.join("bundle.json");
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fs::write(
            &bundle_path,
            serde_json::to_vec(&bundle).expect("bundle bytes"),
        )
        .expect("write bundle.json");
        #[allow(clippy::disallowed_methods, reason = "cfg(test) helper")]
        fs::set_permissions(&bundle_path, fs::Permissions::from_mode(0o640))
            .expect("chmod bundle.json");

        let resolver = BundleResolver::load_with_policy(
            &bundle_path,
            &BundleVerifyPolicy {
                required_uid: rustix::process::getuid().as_raw(),
                required_gid: Some(rustix::process::getgid().as_raw()),
                required_mode: 0o640,
            },
        )
        .expect("the broker's own bundle load succeeds");

        assert_eq!(
            resolver.zone_name_for_uid(&uid(WORK_ZONE_UID)),
            Some("work"),
            "the verified bundle binds the Zone uid the effect carries"
        );
        assert_eq!(
            resolve_consumer_principal(&resolver, &uid(WORK_ZONE_UID), &process("shell")),
            Err(ConsumerPrincipalError::RowUnresolved {
                consumer: "Process/shell".to_owned(),
            }),
            "the broker's own load resolves no principal for a row whose host \
             has provisioned no account, so no ACL entry is written from it"
        );
        assert_eq!(
            work_resolver().consumer_principal("work", &process("shell")),
            None,
            "and the in-memory assembly resolves the same consumer to nothing"
        );
    }

}
