//! Read-side helpers over a stored `CredentialBinding` row.
//!
//! Every consumer of a binding row - the plane deciding whether a delivery
//! gates a start, an operator asking what one row currently holds, and this
//! crate's own tests - reads its delivery state through the pure readers
//! here. Neither touches a store, and both fail closed on a row they cannot
//! read: an unfenced, stale, or unparseable projection is never read as a
//! live delivery.
//!
//! The row's desired spec is read through the contract layer's own typed spec
//! ([`CredentialBindingSpec`]), never through a shape spelled here. This
//! module deliberately holds no second copy of a binding facet: a reader that
//! parsed its own field set could drift from the contract the graph publishes,
//! which is exactly the failure this refactor exists to prevent.

use d2b_contracts_resource::v3::StoredResource;
use d2b_contracts_resource::v3::credential_binding::CredentialBindingSpec;

use crate::driver::CredentialBindingStatusResource;

/// The reserved envelope fields a stored spec carries beside the typed ones.
const RESERVED_SPEC_FIELDS: [&str; 3] = ["providerRef", "updatePolicy", "provider"];

/// Parse a stored CredentialBinding row to the contract's typed spec.
///
/// The reserved envelope fields the store keeps beside the base layer are
/// stripped first; everything else must be exactly the row contract's field
/// set. `None` means the stored bytes are not a canonical binding row, and a
/// reader must not guess a relationship out of bytes it cannot parse.
pub fn parsed_binding_spec(row: &StoredResource) -> Option<CredentialBindingSpec> {
    let mut spec = serde_json::from_slice::<serde_json::Value>(&row.canonical_json)
        .ok()?
        .get("spec")?
        .clone();
    {
        let object = spec.as_object_mut()?;
        for field in RESERVED_SPEC_FIELDS {
            object.remove(field);
        }
    }
    serde_json::from_value::<CredentialBindingSpec>(spec).ok()
}

/// The fenced delivery projection one stored row carries.
///
/// `None` means the row carries no readable projection: the layer is absent,
/// it is not this projection's field set, or a value in it is not canonical.
/// A partial layer is never returned as a projection.
pub fn stored_binding_status(row: &StoredResource) -> Option<CredentialBindingStatusResource> {
    let value = serde_json::from_slice::<serde_json::Value>(&row.canonical_json).ok()?;
    let projection = value.pointer("/status/resource")?;
    CredentialBindingStatusResource::parse(projection)
}

/// Whether one stored CredentialBinding carries a current fenced delivery.
///
/// Current means the row reports a live delivery observed under its own
/// identity: the fence's uid and generation must match the stored row's, and
/// the fence revision must not be ahead of the stored revision. A row whose
/// projection was authored under a replaced identity, a superseded spec, or a
/// revision the store never held is not current, whatever it says about
/// `delivered`.
pub fn binding_readiness_current(row: &StoredResource) -> bool {
    stored_binding_status(row).is_some_and(|status| {
        status.readiness_is_current(&row.uid, row.generation, row.revision)
    })
}

#[cfg(test)]
mod tests {
    use d2b_contracts_resource::v3::{
        BindingLifecycleState, CanonicalJsonValue, ResourceGeneration, ResourceRef, ResourceUid,
        StateDigest, ZoneId, ZoneRevision, resource_status::StatusCode,
    };

    use super::*;
    use crate::driver::CredentialBindingReadinessFence;

    /// A stored row whose base spec is the row contract, carrying the reserved
    /// envelope fields the store keeps beside it.
    #[test]
    fn the_typed_spec_reads_back_out_of_committed_bytes() {
        let row = stored_row(
            1,
            1,
            delivered_projection(1, 1),
        );
        let mut value = serde_json::from_slice::<serde_json::Value>(&row.canonical_json)
            .expect("canonical resource");
        value["spec"] = serde_json::json!({
            "providerRef": "Provider/credential-binding",
            "credentialRef": "Credential/data",
            "executionRef": "Guest/work-vm",
            "operations": ["acquire-token"],
            "lifetimeMs": 60_000,
            "slot": "operator",
            "source": {
                "admittedRights": ["consume"],
                "arbitration": "shared",
                "realizedFacets": ["credential-delivery"]
            }
        });
        let mut row = row;
        row.canonical_json = CanonicalJsonValue::parse(
            &serde_json::to_vec(&value).expect("resource serialization"),
        )
        .expect("canonical resource")
        .to_canonical_bytes();

        let spec = parsed_binding_spec(&row).expect("the row is a canonical binding row");
        assert_eq!(spec.credential_ref().to_canonical_string(), "Credential/data");
        assert_eq!(spec.execution_ref().to_canonical_string(), "Guest/work-vm");
        assert_eq!(spec.slot().as_str(), "operator");
        assert_eq!(spec.lifetime_ms(), 60_000);
        assert_eq!(spec.operations().len(), 1);

        // A row that is not the canonical shape is not guessed at.
        let mut broken = row.clone();
        broken.canonical_json = b"not a resource".to_vec();
        assert!(parsed_binding_spec(&broken).is_none());
        let mut widened = row.clone();
        let mut value = serde_json::from_slice::<serde_json::Value>(&widened.canonical_json)
            .expect("canonical resource");
        value["spec"]["leaseHandle"] = serde_json::json!("material");
        widened.canonical_json = CanonicalJsonValue::parse(
            &serde_json::to_vec(&value).expect("resource serialization"),
        )
        .expect("canonical resource")
        .to_canonical_bytes();
        assert!(
            parsed_binding_spec(&widened).is_none(),
            "a row carrying an extra field is not this contract's row"
        );
    }

    fn uid() -> ResourceUid {
        ResourceUid::parse("123e4567-e89b-42d3-a456-426614174000").expect("fixture uid")
    }

    fn stored_row(generation: u64, revision: u64, projection: serde_json::Value) -> StoredResource {
        let resource_ref = ResourceRef::parse("CredentialBinding/delivery").expect("row reference");
        let zone = ZoneId::parse("work").expect("zone");
        let value = serde_json::json!({
            "apiVersion": "resources.d2bus.org/v3",
            "type": resource_ref.resource_type().as_str(),
            "metadata": {
                "name": resource_ref.name().as_str(),
                "zone": zone.as_str(),
                "ownerRef": "Credential/data",
                "labels": {},
                "annotations": {},
                "finalizers": [],
                "managedBy": "controller",
                "deletionRequestedAt": null,
                "createdAt": "2026-08-19T00:00:00.000Z",
                "updatedAt": "2026-08-19T00:00:00.000Z",
                "generation": generation,
                "revision": revision,
                "uid": uid().as_str(),
            },
            "spec": {},
            "status": {
                "observedGeneration": 0,
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
                    "observedGeneration": 0,
                    "operationId": null,
                    "owned": {"count": 0, "refs": []},
                    "preserveState": true,
                    "reasons": [],
                    "state": "Unknown",
                    "targetGeneration": generation
                },
                "resource": projection
            }
        });
        StoredResource {
            resource_ref,
            zone,
            uid: uid(),
            owner_uid: None,
            owner_generation: None,
            generation: ResourceGeneration::new(generation).expect("nonzero generation"),
            revision: ZoneRevision::new(revision),
            canonical_json: CanonicalJsonValue::parse(
                &serde_json::to_vec(&value).expect("resource serialization"),
            )
            .expect("canonical resource")
            .to_canonical_bytes(),
            payload_digest: StateDigest::parse(format!("sha256:{}", "0".repeat(64)))
                .expect("a zero digest is a valid state digest"),
        }
    }

    fn delivered_projection(generation: u64, revision: u64) -> serde_json::Value {
        CredentialBindingStatusResource::render(
            BindingLifecycleState::Active,
            true,
            CredentialBindingReadinessFence::new(
                uid(),
                ResourceGeneration::new(generation).expect("nonzero generation"),
                ZoneRevision::new(revision),
            ),
            1,
            1_760_000_060_000,
            None,
        )
    }

    /// A projection authored under the row's own identity and generation is
    /// current, and it round-trips through the reader with its field set
    /// intact.
    #[test]
    fn a_current_projection_is_read_back_intact() {
        let row = stored_row(2, 7, delivered_projection(2, 7));
        let status = stored_binding_status(&row).expect("the projection is this contract's");
        assert_eq!(status.state(), "active");
        assert!(status.delivered());
        assert_eq!(status.sequence(), 1);
        assert_eq!(status.expires_unix_ms(), 1_760_000_060_000);
        assert!(status.reason().is_none());
        assert_eq!(status.fence().generation().get(), 2);
        assert!(binding_readiness_current(&row));
    }

    /// A fence the row's own identity, generation, or stored revision does not
    /// support never reads as a live delivery, whatever it says.
    #[test]
    fn a_stale_or_foreign_fence_is_never_current() {
        // A superseded spec: the projection was observed at generation 1 and
        // the row has moved on.
        let superseded = stored_row(2, 7, delivered_projection(1, 7));
        assert!(!binding_readiness_current(&superseded));

        // A reassigned row: same name, new identity.
        let mut reassigned = stored_row(2, 7, delivered_projection(2, 7));
        reassigned.uid =
            ResourceUid::parse("223e4567-e89b-42d3-a456-426614174001").expect("fixture uid");
        assert!(!binding_readiness_current(&reassigned));

        // A fence ahead of the stored revision: evidence claiming a commit
        // the store never held is forged or corrupt, never current.
        let ahead = stored_row(2, 7, delivered_projection(2, 8));
        assert!(!binding_readiness_current(&ahead));
    }

    /// A row that reports no live delivery is not ready, however well fenced
    /// the projection is.
    #[test]
    fn a_not_delivered_projection_is_never_current() {
        let projection = CredentialBindingStatusResource::render(
            BindingLifecycleState::Revoking,
            false,
            CredentialBindingReadinessFence::new(
                uid(),
                ResourceGeneration::new(1).expect("nonzero generation"),
                ZoneRevision::new(3),
            ),
            1,
            1_760_000_060_000,
            Some(
                StatusCode::parse("credential-delivery-not-established")
                    .expect("the reason is a stable code"),
            ),
        );
        let row = stored_row(1, 3, projection);
        let status = stored_binding_status(&row).expect("the projection is this contract's");
        assert_eq!(status.state(), "revoking");
        assert!(!status.delivered());
        assert_eq!(
            status.reason().map(StatusCode::as_str),
            Some("credential-delivery-not-established")
        );
        assert!(!binding_readiness_current(&row));
    }

    /// A layer that is not this projection fails closed rather than being read
    /// as a partial report.
    #[test]
    fn an_unreadable_layer_is_never_read_as_a_delivery() {
        // An empty layer.
        let empty = stored_row(1, 1, serde_json::json!({}));
        assert!(stored_binding_status(&empty).is_none());
        assert!(!binding_readiness_current(&empty));

        // An extra field this contract does not define.
        let mut widened = delivered_projection(1, 1);
        widened["leaseHandle"] = serde_json::json!("material");
        let widened = stored_row(1, 1, widened);
        assert!(stored_binding_status(&widened).is_none());
        assert!(!binding_readiness_current(&widened));

        // A state outside the closed lifecycle vocabulary.
        let mut invented = delivered_projection(1, 1);
        invented["state"] = serde_json::json!("delivering-secretly");
        let invented = stored_row(1, 1, invented);
        assert!(stored_binding_status(&invented).is_none());

        // A field of the wrong shape.
        let mut mistyped = delivered_projection(1, 1);
        mistyped["delivered"] = serde_json::json!("yes");
        let mistyped = stored_row(1, 1, mistyped);
        assert!(stored_binding_status(&mistyped).is_none());

        // A fence that is not a canonical uid.
        let mut forged = delivered_projection(1, 1);
        forged["fence"]["uid"] = serde_json::json!("not-a-uid");
        let forged = stored_row(1, 1, forged);
        assert!(stored_binding_status(&forged).is_none());

        // A row whose stored bytes are not JSON at all.
        let mut broken = stored_row(1, 1, delivered_projection(1, 1));
        broken.canonical_json = b"not a resource".to_vec();
        assert!(stored_binding_status(&broken).is_none());
        assert!(!binding_readiness_current(&broken));
    }
}
