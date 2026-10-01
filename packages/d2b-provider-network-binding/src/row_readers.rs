//! Read-side helpers over a stored `NetworkBinding` row.
//!
//! Every consumer of a membership row - the plane's admission check reading
//! what a binding declares, and a consumer deciding whether its own membership
//! is realized - reads it through the two pure readers here: the fenced
//! readiness projection the row's actor publishes, and the typed spec parse.
//! Neither touches a store, and both fail closed on a row they cannot read.

use d2b_contracts_resource::v3::StoredResource;
use d2b_contracts_resource::v3::network_binding::NetworkBindingSpec;

use crate::driver::NetworkBindingStatusResource;

/// Whether one stored `NetworkBinding` carries a current fenced readiness
/// projection.
///
/// An unparseable projection, an unfenced one, and one authored under another
/// identity or spec generation all fail closed.
pub fn binding_readiness_current(binding: &StoredResource) -> bool {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&binding.canonical_json) else {
        return false;
    };
    let Some(resource) = value
        .pointer("/status/resource")
        .cloned()
        .map(serde_json::from_value::<NetworkBindingStatusResource>)
        .transpose()
        .ok()
        .flatten()
    else {
        return false;
    };
    resource.readiness_is_current(&binding.uid, binding.generation, binding.revision)
}

/// The Network generation the stored row reports its membership realized
/// under, when its projection names one.
///
/// A row whose projection reports no generation holds no observed membership,
/// so this reads `None` rather than a zero generation that would compare equal
/// to nothing.
pub fn parsed_fabric_generation(binding: &StoredResource) -> Option<u64> {
    let value = serde_json::from_slice::<serde_json::Value>(&binding.canonical_json).ok()?;
    let resource: NetworkBindingStatusResource =
        serde_json::from_value(value.pointer("/status/resource")?.clone()).ok()?;
    resource.fabric_generation.map(|generation| generation.get())
}

/// Parse a stored binding envelope to its typed spec, stripping the reserved
/// envelope fields the store holds alongside the three typed ones.
///
/// Returns `None` for genuinely broken resources, and for a row whose base is
/// not a canonical `NetworkBinding` request at all.
pub fn parsed_binding_spec(binding: &StoredResource) -> Option<NetworkBindingSpec> {
    let mut spec = serde_json::from_slice::<serde_json::Value>(&binding.canonical_json)
        .ok()?
        .get("spec")?
        .clone();
    {
        let object = spec.as_object_mut()?;
        for field in ["providerRef", "updatePolicy", "provider"] {
            object.remove(field);
        }
    }
    serde_json::from_value::<NetworkBindingSpec>(spec).ok()
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::{
        CanonicalJsonValue, ResourceRef, ResourceUid, ZoneId, ZoneRevision,
    };

    use super::*;

    const ROW_UID: &str = "123e4567-e89b-42d3-a456-426614174000";

    fn binding_ref() -> ResourceRef {
        ResourceRef::parse("NetworkBinding/lan-guest-a").expect("resource reference")
    }

    fn canonical(value: &serde_json::Value) -> Vec<u8> {
        CanonicalJsonValue::parse(&serde_json::to_vec(value).expect("serialization"))
            .expect("canonical resource")
            .to_canonical_bytes()
    }

    /// A stored row carrying the canonical base spec this crate parses.
    fn stored_row(generation: u64, status_resource: serde_json::Value) -> StoredResource {
        let zone = ZoneId::parse("dev").expect("zone");
        let value = serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": "NetworkBinding",
            "metadata": {
                "name": "lan-guest-a",
                "zone": zone.as_str(),
                "ownerRef": null,
                "labels": {},
                "annotations": {},
                "finalizers": [],
                "managedBy": "controller",
                "deletionRequestedAt": null,
                "createdAt": "2026-08-19T00:00:00.000Z",
                "updatedAt": "2026-08-19T00:00:00.000Z",
                "generation": generation,
                "revision": generation,
                "uid": ROW_UID
            },
            "spec": {
                "providerRef": "Provider/network-local",
                "networkRef": "Network/lan",
                "executionRef": "Guest/guest-a",
                "presentation": {"presentation": "shared-fabric"},
                "source": {
                    "admittedRights": ["consume"],
                    "arbitration": "shared",
                    "realizedFacets": ["shared-fabric"]
                }
            },
            "status": {
                "observedGeneration": generation,
                "phase": "Ready",
                "conditions": [],
                "lastReconciledAt": null,
                "startedAt": null,
                "completedAt": null,
                "outcome": null,
                "update": {
                    "dependencies": {"count": 0, "refs": []},
                    "disruption": "None",
                    "lastAssessedAt": null,
                    "observedGeneration": generation,
                    "operationId": null,
                    "owned": {"count": 0, "refs": []},
                    "preserveState": true,
                    "reasons": [],
                    "state": "Unknown",
                    "targetGeneration": generation
                },
                "resource": status_resource
            }
        });
        StoredResource {
            resource_ref: binding_ref(),
            zone,
            uid: ResourceUid::parse(ROW_UID).expect("uid"),
            owner_uid: None,
            owner_generation: None,
            generation: d2b_contracts_resource::v3::ResourceGeneration::new(generation)
                .expect("generation"),
            revision: ZoneRevision::new(generation),
            canonical_json: canonical(&value),
            payload_digest: d2b_contracts_resource::v3::StateDigest::parse(format!(
                "sha256:{}",
                "0".repeat(64)
            ))
            .expect("a zero digest is a valid state digest"),
        }
    }

    fn ready_projection(uid: &str, generation: u64, revision: u64) -> serde_json::Value {
        serde_json::json!({
            "ready": true,
            "fence": {"uid": uid, "generation": generation, "revision": revision},
            "fabricGeneration": 4
        })
    }

    #[test]
    fn a_fenced_ready_projection_is_current_and_a_foreign_one_is_not() {
        let row = stored_row(2, ready_projection(ROW_UID, 2, 2));
        assert!(binding_readiness_current(&row));
        assert_eq!(parsed_fabric_generation(&row), Some(4));

        // A projection authored under another spec generation never reports the
        // row ready.
        let stale = stored_row(3, ready_projection(ROW_UID, 2, 2));
        assert!(!binding_readiness_current(&stale));
        // Nor does one authored under another identity.
        let foreign = stored_row(
            2,
            ready_projection("11111111-1111-4111-8111-111111111111", 2, 2),
        );
        assert!(!binding_readiness_current(&foreign));
        // Nor does one whose fence sits ahead of the stored revision: the
        // revision is a currency bound, not a free field.
        let ahead = stored_row(2, ready_projection(ROW_UID, 2, 9));
        assert!(!binding_readiness_current(&ahead));
    }

    #[test]
    fn an_unrealized_or_unreadable_projection_never_reports_ready() {
        let pending = stored_row(
            2,
            serde_json::json!({
                "ready": false,
                "fence": {"uid": ROW_UID, "generation": 2, "revision": 2},
                "reason": "network-binding-membership-not-realized"
            }),
        );
        assert!(!binding_readiness_current(&pending));
        assert_eq!(parsed_fabric_generation(&pending), None);

        let empty = stored_row(2, serde_json::json!({}));
        assert!(!binding_readiness_current(&empty));
    }

    #[test]
    fn the_typed_spec_reads_the_committed_row_and_rejects_a_foreign_shape() {
        let row = stored_row(2, ready_projection(ROW_UID, 2, 2));
        let spec = parsed_binding_spec(&row).expect("the canonical row parses");
        assert_eq!(
            spec.network_ref().to_canonical_string(),
            "Network/lan",
            "the reader strips the reserved envelope fields and keeps the typed ones"
        );
        assert_eq!(
            spec.execution_ref().to_canonical_string(),
            "Guest/guest-a"
        );

        // A row whose base is not a canonical NetworkBinding is not guessed at.
        let mut foreign = serde_json::from_slice::<serde_json::Value>(&row.canonical_json)
            .expect("stored row");
        foreign["spec"] = serde_json::json!({"networkRef": "Network/lan"});
        let foreign = StoredResource {
            canonical_json: canonical(&foreign),
            ..row
        };
        assert!(parsed_binding_spec(&foreign).is_none());
    }
}
